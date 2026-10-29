//! OS input handling (`src/nvim/os/input.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_CURSORHOLD, EVENT_CURSORHOLDI, PROF_YES, apply_autocmds, trigger_cursorhold,
};
use crate::eval::vars::{VV_USERACTIVE, set_vim_var_nr};
use crate::event::r#loop::{loop_poll_events, main_loop_events, main_loop_ptr, uv_guess_handle};
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_empty, multiqueue_process_events, multiqueue_put_event,
};
use crate::globals::*;
use crate::input::{before_blocking, typebuf_changed};
use crate::insexpand::{
    ins_compl_active, ins_compl_autocomplete_elapsed, ins_compl_autocomplete_pending,
};
use crate::keycodes::{
    FSK_KEYCODE, K_SPECIAL, KE_COMPLETE_DELAY, KE_EVENT, KE_FILLER, KS_EXTRA, KS_MODIFIER,
    KS_SPECIAL, get_mouse_button, trans_special_slice,
};
use crate::log::{LOGLVL_WRN, logmsg};
use crate::main_init::{getout, input_unanswerable};
use crate::option_vars::*;
use crate::os::time::{os_hrtime, os_realtime};
use crate::profile::{prof_input_end, prof_input_start};
use crate::state::{MODE_INSERT, MODE_MAX_LENGTH, get_mode, get_real_state};
use std::ptr;

use crate::eval::typval::Varnumber;

const READ_BUFFER_SIZE: usize = 0xfff;
const MAX_KEY_CODE_LEN: usize = 6;
const INPUT_BUFFER_SIZE: usize = (READ_BUFFER_SIZE * 4) + MAX_KEY_CODE_LEN;

const KE_LEFTMOUSE: i32 = 44;
const KE_RIGHTRELEASE: i32 = 52;
const KE_MOUSEDOWN: i32 = 75;
const KE_MOUSERIGHT: i32 = 78;
const KE_X1MOUSE: i32 = 89;
const KE_X2RELEASE: i32 = 94;
const KE_MOUSEMOVE: i32 = 100;

const MOD_MASK_CTRL: u8 = 0x04;
const MOD_MASK_2CLICK: u8 = 0x20;
const MOD_MASK_3CLICK: u8 = 0x40;
const MOD_MASK_4CLICK: u8 = 0x60;

const CTRL_C_CONST_CONST_CONST: u8 = 3;

const BREAKCHECK_SKIP: i32 = 1000;

struct OsInputState {
    input_buffer: [u8; INPUT_BUFFER_SIZE],
    input_read_pos: usize,
    input_write_pos: usize,
    blocking: bool,
    cursorhold_time: i32,
    cursorhold_tb_change_cnt: i32,
    breakcheck_count: i32,
    orig_num_clicks: i32,
    orig_mouse_code: i32,
    orig_mouse_grid: i32,
    orig_mouse_col: i32,
    orig_mouse_row: i32,
    orig_mouse_time: u64,
    push_event_key_idx: usize,
}

impl OsInputState {
    const fn new() -> Self {
        Self {
            input_buffer: [0; INPUT_BUFFER_SIZE],
            input_read_pos: 0,
            input_write_pos: 0,
            blocking: false,
            cursorhold_time: 0,
            cursorhold_tb_change_cnt: 0,
            breakcheck_count: 0,
            orig_num_clicks: 0,
            orig_mouse_code: 0,
            orig_mouse_grid: 0,
            orig_mouse_col: 0,
            orig_mouse_row: 0,
            orig_mouse_time: 0,
            push_event_key_idx: 0,
        }
    }
}

static OSI_STATE: GlobalState<OsInputState> = GlobalState::new(OsInputState::new());

#[inline]
fn osi_st() -> &'static mut OsInputState {
    unsafe { &mut *OSI_STATE.as_ptr() }
}

fn parse_mouse_coords(s: &[u8]) -> Option<(i32, i32, usize)> {
    if s.first().copied() != Some(b'<') {
        return None;
    }
    let parse_int = |mut idx: usize| -> Option<(i32, usize)> {
        while matches!(
            s.get(idx).copied(),
            Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
        ) {
            idx += 1;
        }
        let neg = if s.get(idx).copied() == Some(b'-') {
            idx += 1;
            true
        } else {
            if s.get(idx).copied() == Some(b'+') {
                idx += 1;
            }
            false
        };
        if !s.get(idx).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        let mut acc: i64 = 0;
        while let Some(d) = s.get(idx).copied().filter(u8::is_ascii_digit) {
            acc = acc.saturating_mul(10).saturating_add(i64::from(d - b'0'));
            idx += 1;
        }
        let val = if neg { -acc } else { acc } as i32;
        Some((val, idx))
    };
    let (col, after_col) = parse_int(1)?;
    if s.get(after_col).copied() != Some(b',') {
        return None;
    }
    let (row, after_row) = parse_int(after_col + 1)?;
    if s.get(after_row).copied() != Some(b'>') {
        return None;
    }
    Some((col, row, after_row + 1))
}

use crate::event::stream::UV_TTY;

pub fn cursorhold_event() {
    let event = if (g_st().state_global & MODE_INSERT) != 0 {
        EVENT_CURSORHOLDI
    } else {
        EVENT_CURSORHOLD
    };
    apply_autocmds(
        event,
        ptr::null_mut(),
        ptr::null_mut(),
        false,
        g_st().curbuf,
    );
    g_st().did_cursorhold = true;
}

fn create_cursorhold_event(events_enabled: bool) {
    let events = main_loop_events();
    debug_assert!(!events_enabled || multiqueue_empty(unsafe { &*events }));
    multiqueue_put_event(unsafe { &mut *events }, Event::CursorHold);
}

fn reset_cursorhold_wait(tb_change_cnt: i32) {
    osi_st().cursorhold_time = 0;
    osi_st().cursorhold_tb_change_cnt = tb_change_cnt;
}

/// Reads OS input into `buf`, and consumes pending events while waiting (if `ms != 0`).
pub fn input_get(
    buf: *mut u8,
    maxlen: i32,
    ms: i32,
    tb_change_cnt: i32,
    events: *mut MultiQueue,
) -> i32 {
    debug_assert!((maxlen > 0 && !buf.is_null()) || maxlen == 0);
    // This check is needed so that feeding typeahead from RPC can prevent CursorHold.
    if tb_change_cnt != osi_st().cursorhold_tb_change_cnt {
        reset_cursorhold_wait(tb_change_cnt);
    }

    if maxlen != 0 && input_available() != 0 {
        reset_cursorhold_wait(tb_change_cnt);
        let to_read = (maxlen as usize).min(input_available());
        let rpos = osi_st().input_read_pos;
        unsafe { ptr::copy_nonoverlapping(osi_st().input_buffer[rpos..].as_ptr(), buf, to_read) };
        osi_st().input_read_pos = rpos + to_read;
        debug_assert!(to_read <= i32::MAX as usize);
        return to_read as i32;
    }

    // No risk of a UI flood, so disable CTRL-C "interrupt" behavior if it's mapped.
    if ((g_st().mapped_ctrl_c | curbuf_mut().b_mapped_ctrl_c) & get_real_state()) != 0 {
        g_st().ctrl_c_interrupts = false;
    }

    if ms >= 0 {
        if !inbuf_poll(ms, events) {
            return 0;
        }
    } else {
        let wait_start = os_hrtime();
        osi_st().cursorhold_time = osi_st().cursorhold_time.min(opt_vars().p_ut as i32);
        let delay_pending = ins_compl_autocomplete_pending()
            && opt_vars().p_acl > 0
            && g_st().typebuf.tb_len == 0
            && !ins_compl_active()
            && (get_real_state() & MODE_INSERT) != 0;
        let mut wait_time = opt_vars().p_ut - i64::from(osi_st().cursorhold_time);
        if delay_pending {
            let delay_left = opt_vars().p_acl - ins_compl_autocomplete_elapsed();
            wait_time = delay_left.max(0).min(wait_time);
        }
        if !inbuf_poll(wait_time as i32, events) {
            if g_st().silent_mode {
                // Ran out of input: exit silent/batch-mode (-es/-Es).
                getout(0);
            }
            if delay_pending
                && ins_compl_autocomplete_elapsed() >= opt_vars().p_acl
                && (buf.is_null() || maxlen >= 3)
                && !typebuf_changed(tb_change_cnt)
            {
                if buf.is_null() {
                    let ibuf: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_COMPLETE_DELAY];
                    input_enqueue_raw(&ibuf);
                    return 0;
                }
                unsafe {
                    *buf = K_SPECIAL;
                }
                unsafe {
                    *buf.wrapping_add(1) = KS_EXTRA;
                }
                unsafe {
                    *buf.wrapping_add(2) = KE_COMPLETE_DELAY;
                }
                return 3;
            }
            reset_cursorhold_wait(tb_change_cnt);
            if trigger_cursorhold() && !typebuf_changed(tb_change_cnt) {
                create_cursorhold_event(events == main_loop_events());
            } else {
                before_blocking();
                inbuf_poll(-1, events);
            }
        } else {
            osi_st().cursorhold_time += ((os_hrtime() - wait_start) / 1_000_000) as i32;
        }
    }

    g_st().ctrl_c_interrupts = true;

    // If input was put directly in typeahead buffer bail out here.
    if typebuf_changed(tb_change_cnt) {
        return 0;
    }

    if maxlen != 0 && input_available() != 0 {
        reset_cursorhold_wait(tb_change_cnt);
        let to_read = (maxlen as usize).min(input_available());
        let rpos = osi_st().input_read_pos;
        unsafe { ptr::copy_nonoverlapping(osi_st().input_buffer[rpos..].as_ptr(), buf, to_read) };
        osi_st().input_read_pos = rpos + to_read;
        debug_assert!(to_read <= i32::MAX as usize);
        return to_read as i32;
    }

    // If there are events, return the keys directly
    if maxlen != 0 && pending_events(unsafe { events.as_ref() }) {
        return push_event_key(buf, maxlen);
    }

    0
}

/// Check if a character is available for reading
pub fn os_char_avail() -> bool {
    inbuf_poll(0, ptr::null_mut())
}

/// Poll for fast events. `got_int` will be set to `true` if CTRL-C was typed.
pub fn os_breakcheck() {
    if g_st().got_int {
        return;
    }

    loop_poll_events(&mut *main_loop_ptr(), 0);
}

/// Check for CTRL-C pressed, but only once in a while.
#[inline]
pub fn line_breakcheck() {
    osi_st().breakcheck_count += 1;
    if osi_st().breakcheck_count >= BREAKCHECK_SKIP {
        osi_st().breakcheck_count = 0;
        os_breakcheck();
    }
}

/// Like line_breakcheck() but check 10 times less often.
#[inline]
pub fn fast_breakcheck() {
    osi_st().breakcheck_count += 1;
    if osi_st().breakcheck_count >= BREAKCHECK_SKIP * 10 {
        osi_st().breakcheck_count = 0;
        os_breakcheck();
    }
}

/// Like line_breakcheck() but check 100 times less often.
#[inline]
pub fn veryfast_breakcheck() {
    osi_st().breakcheck_count += 1;
    if osi_st().breakcheck_count >= BREAKCHECK_SKIP * 100 {
        osi_st().breakcheck_count = 0;
        os_breakcheck();
    }
}

/// Test whether a file descriptor refers to a terminal.
pub fn os_isatty(fd: i32) -> bool {
    unsafe { uv_guess_handle(fd) == UV_TTY }
}

pub fn input_available() -> usize {
    osi_st().input_write_pos.saturating_sub(osi_st().input_read_pos)
}

fn input_space() -> usize {
    INPUT_BUFFER_SIZE.saturating_sub(osi_st().input_write_pos)
}

pub fn input_enqueue_raw(data: &[u8]) {
    let st = osi_st();
    if st.input_read_pos > 0 {
        let available = st.input_write_pos.saturating_sub(st.input_read_pos);
        st.input_buffer
            .copy_within(st.input_read_pos..st.input_write_pos, 0);
        st.input_read_pos = 0;
        st.input_write_pos = available;
    }

    let to_write = data.len().min(INPUT_BUFFER_SIZE.saturating_sub(st.input_write_pos));
    let wpos = st.input_write_pos;
    st.input_buffer[wpos..wpos + to_write].copy_from_slice(&data[..to_write]);
    st.input_write_pos = wpos + to_write;
}

pub fn input_enqueue(chan_id: u64, keys: NvimString) -> usize {
    g_st().current_ui = chan_id;

    if osi_st().input_read_pos == osi_st().input_write_pos {
        osi_st().input_read_pos = 0;
        osi_st().input_write_pos = 0;
    }

    let key_bytes = keys.as_bytes();
    if !key_bytes.is_empty() {
        set_vim_var_nr(VV_USERACTIVE, os_realtime() as Varnumber);
    }

    let mut pos: usize = 0;
    while input_space() >= 19 && pos < key_bytes.len() {
        let mut buf = [0u8; 19];
        if let Some((consumed, dlen)) =
            trans_special_slice(&key_bytes[pos..], &mut buf, FSK_KEYCODE, true, None)
        {
            pos += consumed;
            let new_size = handle_mouse_event(key_bytes, &mut pos, &mut buf, dlen as u32);
            if new_size > 0 {
                input_enqueue_raw(&buf[..new_size as usize]);
            }
            continue;
        }

        if key_bytes[pos] == b'<' {
            let old_pos = pos;
            // Invalid or incomplete key sequence, skip until the next '>' or *end.
            loop {
                pos += 1;
                if pos >= key_bytes.len() || key_bytes[pos] == b'>' {
                    break;
                }
            }
            if pos >= key_bytes.len() || key_bytes[pos] != b'>' {
                // Incomplete key sequence, return without consuming.
                pos = old_pos;
                break;
            }
            pos += 1;
            continue;
        }

        // copy the character, escaping K_SPECIAL
        if key_bytes[pos] == K_SPECIAL {
            input_enqueue_raw(&[K_SPECIAL, KS_SPECIAL, KE_FILLER]);
        } else {
            input_enqueue_raw(&key_bytes[pos..pos + 1]);
        }
        pos += 1;
    }

    process_ctrl_c();
    pos
}

fn check_multiclick(code: i32, grid: i32, row: i32, col: i32, skip_event: &mut bool) -> u8 {
    if (KE_MOUSEDOWN..=KE_MOUSERIGHT).contains(&code) {
        return 0;
    }

    let no_move = osi_st().orig_mouse_grid == grid
        && osi_st().orig_mouse_col == col
        && osi_st().orig_mouse_row == row;
    let mut is_click = false;
    let mut is_drag = false;
    let _ = get_mouse_button(code, &mut is_click, &mut is_drag);

    if is_drag && no_move {
        *skip_event = true;
        return 0;
    }

    if is_click {
        // For click events the number of clicks is updated.
        let mouse_time = os_hrtime(); // time of current mouse click (ns)
        let timediff = mouse_time.wrapping_sub(osi_st().orig_mouse_time);
        let mouset = (opt_vars().p_mouset as u64).wrapping_mul(1_000_000);
        if code == osi_st().orig_mouse_code
            && no_move
            && timediff < mouset
            && osi_st().orig_num_clicks != 4
        {
            osi_st().orig_num_clicks += 1;
        } else {
            osi_st().orig_num_clicks = 1;
        }
        osi_st().orig_mouse_code = code;
        osi_st().orig_mouse_time = mouse_time;
    } else if !no_move {
        // For drag and release events the number of clicks is kept.
        // However, if the mouse has moved, the next click should reset the click count.
        osi_st().orig_mouse_code = code;
    }

    osi_st().orig_mouse_grid = grid;
    osi_st().orig_mouse_col = col;
    osi_st().orig_mouse_row = row;

    let mut modifiers: u8 = 0;
    if code != KE_MOUSEMOVE {
        if osi_st().orig_num_clicks == 2 {
            modifiers |= MOD_MASK_2CLICK;
        } else if osi_st().orig_num_clicks == 3 {
            modifiers |= MOD_MASK_3CLICK;
        } else if osi_st().orig_num_clicks == 4 {
            modifiers |= MOD_MASK_4CLICK;
        }
    }
    modifiers
}

/// Mouse event handling code (extract row/col if available and detect multiple clicks)
fn handle_mouse_event(
    key_bytes: &[u8],
    pos: &mut usize,
    buf: &mut [u8; 19],
    mut bufsize: u32,
) -> u32 {
    let mut mouse_code: i32 = 0;
    let mut key_type: i32 = 0;

    if bufsize == 3 {
        mouse_code = i32::from(buf[2]);
        key_type = i32::from(buf[1]);
    } else if bufsize == 6 {
        // prefixed with K_SPECIAL KS_MODIFIER mod
        mouse_code = i32::from(buf[5]);
        key_type = i32::from(buf[4]);
    }

    if key_type != i32::from(KS_EXTRA)
        || !((KE_LEFTMOUSE..=KE_RIGHTRELEASE).contains(&mouse_code)
            || (KE_X1MOUSE..=KE_X2RELEASE).contains(&mouse_code)
            || (KE_MOUSEDOWN..=KE_MOUSERIGHT).contains(&mouse_code)
            || mouse_code == KE_MOUSEMOVE)
    {
        return bufsize;
    }

    if let Some((mut col, mut row, advance)) = parse_mouse_coords(&key_bytes[*pos..]) {
        if col >= 0 && row >= 0 {
            if col >= g_st().columns_global {
                col = g_st().columns_global - 1;
            }
            if row >= g_st().rows_global {
                row = g_st().rows_global - 1;
            }
            g_st().mouse_grid = 0;
            g_st().mouse_row = row;
            g_st().mouse_col = col;
        }
        *pos += advance;
    }

    let mut skip_event = false;
    let modifiers = check_multiclick(
        mouse_code,
        g_st().mouse_grid,
        g_st().mouse_row,
        g_st().mouse_col,
        &mut skip_event,
    );
    if skip_event {
        return 0;
    }

    if modifiers != 0 {
        if buf[1] != KS_MODIFIER {
            // no modifiers in the buffer yet, shift the bytes 3 positions
            buf.copy_within(0..3, 3);
            // add the modifier sequence
            buf[0] = K_SPECIAL;
            buf[1] = KS_MODIFIER;
            buf[2] = modifiers;
            bufsize += 3;
        } else {
            buf[2] |= modifiers;
        }
    }

    bufsize
}

pub fn input_enqueue_mouse(code: i32, mut modifier: u8, grid: i32, row: i32, col: i32) {
    let mut skip_event = false;
    modifier |= check_multiclick(code, grid, row, col, &mut skip_event);
    if skip_event {
        return;
    }
    let mut buf = [0u8; 7];
    let mut idx = 0usize;
    if modifier != 0 {
        buf[0] = K_SPECIAL;
        buf[1] = KS_MODIFIER;
        buf[2] = modifier;
        idx = 3;
    }
    buf[idx] = K_SPECIAL;
    buf[idx + 1] = KS_EXTRA;
    buf[idx + 2] = code as u8;

    g_st().mouse_grid = grid;
    g_st().mouse_row = row;
    g_st().mouse_col = col;

    input_enqueue_raw(&buf[..idx + 3]);
}

/// @return true if the main loop is blocked and waiting for input.
pub fn input_blocking() -> bool {
    osi_st().blocking
}

/// Checks for (but does not read) available input, and consumes `main_loop.events` while waiting.
fn inbuf_poll(ms: i32, events: *mut MultiQueue) -> bool {
    if os_input_ready(unsafe { events.as_ref() }) {
        return true;
    }

    if ms == 0 {
        loop_poll_events(&mut *main_loop_ptr(), 0);
        return os_input_ready(unsafe { events.as_ref() });
    }

    if g_st().do_profiling == PROF_YES {
        prof_input_start();
    }

    if events != main_loop_events() {
        // The pending input provoked a osi_st().blocking wait. Do special events now. #6247
        osi_st().blocking = true;
        multiqueue_process_events(unsafe { &mut *g_st().ch_before_blocking_events });
    }

    // LOOP_PROCESS_EVENTS_UNTIL(&main_loop, NULL, ms, os_input_ready(events) || input_unanswerable());
    let mut remaining = i64::from(ms);
    let mut before = if remaining > 0 { os_hrtime() } else { 0 };
    while !(os_input_ready(unsafe { events.as_ref() }) || input_unanswerable()) {
        loop_poll_events(&mut *main_loop_ptr(), remaining);
        if remaining == 0 {
            break;
        } else if remaining > 0 {
            let now = os_hrtime();
            remaining -= ((now - before) / 1_000_000) as i64;
            before = now;
            if remaining <= 0 {
                break;
            }
        }
    }

    osi_st().blocking = false;

    if g_st().do_profiling == PROF_YES {
        prof_input_end();
    }

    if input_unanswerable() && !os_input_ready(unsafe { events.as_ref() }) {
        // Dismiss the prompt (CTRL-C) instead of waiting forever.
        static DID_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !DID_LOG.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let mut mode = [0u8; MODE_MAX_LENGTH];
            get_mode(&mut mode);
            let mode_cstr = std::ffi::CStr::from_bytes_until_nul(&mode).unwrap_or(c"");
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"inbuf_poll",
                line!() as i32,
                true,
                c"Interrupted prompt (mode=%s): user input is unlikely",
                mode_cstr.as_ptr(),
            );
        }
        input_enqueue_raw(b"\x03");
        // Also interrupt the code that prompted. Skips remaining exit autocmds.
        g_st().got_int = true;
    }

    os_input_ready(unsafe { events.as_ref() })
}

fn process_ctrl_c() {
    if !g_st().ctrl_c_interrupts {
        return;
    }

    let st = osi_st();
    let rpos = st.input_read_pos;
    let wpos = st.input_write_pos;
    let available = wpos.saturating_sub(rpos);
    let mut i = available as isize - 1;
    while i >= 0 {
        let idx = rpos + (i as usize);
        let c = st.input_buffer[idx];
        if c == CTRL_C_CONST_CONST_CONST
            || (c == b'C'
                && i >= 3
                && st.input_buffer[idx - 3] == K_SPECIAL
                && st.input_buffer[idx - 2] == KS_MODIFIER
                && st.input_buffer[idx - 1] == MOD_MASK_CTRL)
        {
            st.input_buffer[idx] = CTRL_C_CONST_CONST_CONST;
            g_st().got_int = true;
            break;
        }
        i -= 1;
    }

    if g_st().got_int && i > 0 {
        // Remove all unprocessed input (typeahead) before the CTRL-C.
        st.input_read_pos = rpos + (i as usize);
    }
}

/// Pushes bytes from the "event" key sequence (KE_EVENT) partially between calls to input_get when
/// `maxlen < 3`.
fn push_event_key(buf: *mut u8, maxlen: i32) -> i32 {
    const KEY: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_EVENT];
    {
        let mut buf_idx: i32 = 0;

        loop {
            unsafe {
                *buf.wrapping_add(buf_idx as usize) = KEY[osi_st().push_event_key_idx];
            }
            buf_idx += 1;
            osi_st().push_event_key_idx = (osi_st().push_event_key_idx + 1) % 3;
            if osi_st().push_event_key_idx == 0 || buf_idx >= maxlen {
                break;
            }
        }

        buf_idx
    }
}

/// Check if there's pending input already in typebuf or `events`
pub fn os_input_ready(events: Option<&MultiQueue>) -> bool {
    g_st().typebuf_was_filled || input_available() != 0 || pending_events(events)
}

fn pending_events(events: Option<&MultiQueue>) -> bool {
    events.is_some() && !multiqueue_empty(events.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_enqueue_raw_and_push_event_key_work() {
        osi_st().input_read_pos = 0;
        osi_st().input_write_pos = 0;
        assert_eq!(input_available(), 0);
        input_enqueue_raw(b"abc");
        assert_eq!(input_available(), 3);
        osi_st().input_read_pos = 0;
        osi_st().input_write_pos = 0;

        osi_st().push_event_key_idx = 0;
        let mut buf = [0u8; 3];
        assert_eq!(push_event_key(buf.as_mut_ptr(), 2), 2);
        assert_eq!(buf[0], K_SPECIAL);
        assert_eq!(buf[1], KS_EXTRA);
        assert_eq!(push_event_key(buf.as_mut_ptr().wrapping_add(2), 2), 1);
        assert_eq!(buf[2], KE_EVENT);

        assert_eq!(parse_mouse_coords(b"<12,34>rest"), Some((12, 34, 7)));
        assert_eq!(parse_mouse_coords(b"<-1,0>"), Some((-1, 0, 6)));
        assert_eq!(parse_mouse_coords(b"<12,34"), None);
    }
}
