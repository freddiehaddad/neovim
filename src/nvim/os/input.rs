#![forbid(unsafe_code)]
//! OS input handling (`src/nvim/os/input.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_CURSORHOLD, EVENT_CURSORHOLDI, PROF_YES, apply_autocmds, trigger_cursorhold,
};
use crate::eval::vars::{VV_USERACTIVE, set_vim_var_nr};
use crate::event::r#loop::{loop_poll_events, main_loop_events, main_loop_ptr};
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
    input_buffer: std::cell::RefCell<[u8; INPUT_BUFFER_SIZE]>,
    input_read_pos: std::cell::Cell<usize>,
    input_write_pos: std::cell::Cell<usize>,
    blocking: std::cell::Cell<bool>,
    cursorhold_time: std::cell::Cell<u64>,
    cursorhold_tb_change_cnt: std::cell::Cell<i32>,
    breakcheck_count: std::cell::Cell<i32>,
    orig_num_clicks: std::cell::Cell<i32>,
    orig_mouse_code: std::cell::Cell<i32>,
    orig_mouse_grid: std::cell::Cell<i32>,
    orig_mouse_col: std::cell::Cell<i32>,
    orig_mouse_row: std::cell::Cell<i32>,
    orig_mouse_time: std::cell::Cell<u64>,
    push_event_key_idx: std::cell::Cell<usize>,
}

impl OsInputState {
    const fn new() -> Self {
        Self {
            input_buffer: std::cell::RefCell::new([0; INPUT_BUFFER_SIZE]),
            input_read_pos: std::cell::Cell::new(0),
            input_write_pos: std::cell::Cell::new(0),
            blocking: std::cell::Cell::new(false),
            cursorhold_time: std::cell::Cell::new(0),
            cursorhold_tb_change_cnt: std::cell::Cell::new(0),
            breakcheck_count: std::cell::Cell::new(0),
            orig_num_clicks: std::cell::Cell::new(0),
            orig_mouse_code: std::cell::Cell::new(0),
            orig_mouse_grid: std::cell::Cell::new(0),
            orig_mouse_col: std::cell::Cell::new(0),
            orig_mouse_row: std::cell::Cell::new(0),
            orig_mouse_time: std::cell::Cell::new(0),
            push_event_key_idx: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static OSI_STATE: OsInputState = const { OsInputState::new() };
}

#[inline]
fn osi_st<R>(f: impl FnOnce(&OsInputState) -> R) -> R {
    OSI_STATE.with(f)
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
    let event = if (g_st().state_global.get() & MODE_INSERT) != 0 {
        EVENT_CURSORHOLDI
    } else {
        EVENT_CURSORHOLD
    };
    apply_autocmds(event, None, None, false, curbuf_opt_mut().as_deref());
    g_st().did_cursorhold.set(true);
}

fn create_cursorhold_event(events_enabled: bool) {
    let events = main_loop_events();
    debug_assert!(!events_enabled || multiqueue_empty(events));
    multiqueue_put_event(events, Event::CursorHold);
}

fn reset_cursorhold_wait(tb_change_cnt: i32) {
    osi_st(|st| {
        st.cursorhold_time.set(0);
        st.cursorhold_tb_change_cnt.set(tb_change_cnt);
    });
}

/// Reads OS input into `out_buf`, and consumes pending events while waiting (if `ms != 0`).
pub fn input_get(
    mut out_buf: Option<&mut [u8]>,
    ms: i32,
    tb_change_cnt: i32,
    events: Option<MultiQueue>,
) -> i32 {
    if out_buf.as_ref().is_some_and(|s| s.is_empty()) {
        out_buf = None;
    }
    let maxlen = out_buf.as_ref().map_or(0, |s| s.len() as i32);
    // This check is needed so that feeding typeahead from RPC can prevent CursorHold.
    if tb_change_cnt != osi_st(|st| st.cursorhold_tb_change_cnt.get()) {
        reset_cursorhold_wait(tb_change_cnt);
    }

    if let Some(ref mut out_slice) = out_buf
        && input_available() != 0
    {
        reset_cursorhold_wait(tb_change_cnt);
        let to_read = out_slice.len().min(input_available());
        osi_st(|st| {
            let rpos = st.input_read_pos.get();
            out_slice[..to_read].copy_from_slice(&st.input_buffer.borrow()[rpos..rpos + to_read]);
            st.input_read_pos.set(rpos + to_read);
        });
        debug_assert!(to_read <= i32::MAX as usize);
        return to_read as i32;
    }

    // No risk of a UI flood, so disable CTRL-C "interrupt" behavior if it's mapped.
    if ((g_st().mapped_ctrl_c.get() | curbuf_mut().b_mapped_ctrl_c.get()) & get_real_state()) != 0 {
        g_st().ctrl_c_interrupts.set(false);
    }

    if ms >= 0 {
        if !inbuf_poll(ms, events) {
            return 0;
        }
    } else {
        let wait_start = os_hrtime();
        let ch_time = osi_st(|st| {
            let t = st
                .cursorhold_time
                .get()
                .min((opt_vars().p_ut.get() as u64) * 1_000_000);
            st.cursorhold_time.set(t);
            t
        });
        let delay_pending = ins_compl_autocomplete_pending()
            && opt_vars().p_acl.get() > 0
            && g_st().typebuf.tb_len.get() == 0
            && !ins_compl_active()
            && (get_real_state() & MODE_INSERT) != 0;
        let mut wait_time = opt_vars().p_ut.get() - ((ch_time / 1_000_000) as i64);
        if delay_pending {
            let delay_left = opt_vars().p_acl.get() - ins_compl_autocomplete_elapsed();
            wait_time = delay_left.max(0).min(wait_time);
        }
        if !inbuf_poll(wait_time as i32, events) {
            if g_st().silent_mode.get() {
                // Ran out of input: exit silent/batch-mode (-es/-Es).
                getout(0);
            }
            if delay_pending
                && ins_compl_autocomplete_elapsed() >= opt_vars().p_acl.get()
                && (out_buf.is_none() || maxlen >= 3)
                && !typebuf_changed(tb_change_cnt)
            {
                let Some(ref mut out_slice) = out_buf else {
                    let ibuf: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_COMPLETE_DELAY];
                    input_enqueue_raw(&ibuf);
                    return 0;
                };
                out_slice[..3].copy_from_slice(&[K_SPECIAL, KS_EXTRA, KE_COMPLETE_DELAY]);
                return 3;
            }
            reset_cursorhold_wait(tb_change_cnt);
            if trigger_cursorhold() && !typebuf_changed(tb_change_cnt) {
                create_cursorhold_event(events == Some(main_loop_events()));
            } else {
                before_blocking();
                inbuf_poll(-1, events);
            }
        } else {
            let elapsed = os_hrtime() - wait_start;
            osi_st(|st| st.cursorhold_time.set(st.cursorhold_time.get() + elapsed));
        }
    }

    g_st().ctrl_c_interrupts.set(true);

    // If input was put directly in typeahead buffer bail out here.
    if typebuf_changed(tb_change_cnt) {
        return 0;
    }

    if let Some(ref mut out_slice) = out_buf
        && input_available() != 0
    {
        reset_cursorhold_wait(tb_change_cnt);
        let to_read = out_slice.len().min(input_available());
        osi_st(|st| {
            let rpos = st.input_read_pos.get();
            out_slice[..to_read].copy_from_slice(&st.input_buffer.borrow()[rpos..rpos + to_read]);
            st.input_read_pos.set(rpos + to_read);
        });
        debug_assert!(to_read <= i32::MAX as usize);
        return to_read as i32;
    }

    // If there are events, return the keys directly
    if let Some(ref mut out_slice) = out_buf
        && pending_events(events)
    {
        return push_event_key(out_slice);
    }

    0
}

/// Check if a character is available for reading
pub fn os_char_avail() -> bool {
    inbuf_poll(0, None)
}

/// Poll for fast events. `got_int` will be set to `true` if CTRL-C was typed.
pub fn os_breakcheck() {
    if g_st().got_int.get() {
        return;
    }

    loop_poll_events(&mut *main_loop_ptr(), 0);
}

/// Check for CTRL-C pressed, but only once in a while.
#[inline]
pub fn line_breakcheck() {
    let do_check = osi_st(|st| {
        let c = st.breakcheck_count.get() + 1;
        if c >= BREAKCHECK_SKIP {
            st.breakcheck_count.set(0);
            true
        } else {
            st.breakcheck_count.set(c);
            false
        }
    });
    if do_check {
        os_breakcheck();
    }
}

/// Like line_breakcheck() but check 10 times less often.
#[inline]
pub fn fast_breakcheck() {
    let do_check = osi_st(|st| {
        let c = st.breakcheck_count.get() + 1;
        if c >= BREAKCHECK_SKIP * 10 {
            st.breakcheck_count.set(0);
            true
        } else {
            st.breakcheck_count.set(c);
            false
        }
    });
    if do_check {
        os_breakcheck();
    }
}

/// Like line_breakcheck() but check 100 times less often.
#[inline]
pub fn veryfast_breakcheck() {
    let do_check = osi_st(|st| {
        let c = st.breakcheck_count.get() + 1;
        if c >= BREAKCHECK_SKIP * 100 {
            st.breakcheck_count.set(0);
            true
        } else {
            st.breakcheck_count.set(c);
            false
        }
    });
    if do_check {
        os_breakcheck();
    }
}

/// Test whether a file descriptor refers to a terminal.
pub fn os_isatty(fd: i32) -> bool {
    crate::os::fs::os_guess_handle(fd) == UV_TTY
}

pub fn input_available() -> usize {
    osi_st(|st| {
        st.input_write_pos
            .get()
            .saturating_sub(st.input_read_pos.get())
    })
}

fn input_space() -> usize {
    INPUT_BUFFER_SIZE.saturating_sub(osi_st(|st| st.input_write_pos.get()))
}

pub fn input_enqueue_raw(data: &[u8]) {
    osi_st(|st| {
        let mut buf = st.input_buffer.borrow_mut();
        let rpos = st.input_read_pos.get();
        let mut wpos = st.input_write_pos.get();
        if rpos > 0 {
            let available = wpos.saturating_sub(rpos);
            buf.copy_within(rpos..wpos, 0);
            st.input_read_pos.set(0);
            wpos = available;
            st.input_write_pos.set(wpos);
        }

        let to_write = data.len().min(INPUT_BUFFER_SIZE.saturating_sub(wpos));
        buf[wpos..wpos + to_write].copy_from_slice(&data[..to_write]);
        st.input_write_pos.set(wpos + to_write);
    });
}

pub fn input_enqueue(chan_id: u64, keys: NvimString) -> usize {
    g_st().current_ui.set(chan_id);

    osi_st(|st| {
        if st.input_read_pos.get() == st.input_write_pos.get() {
            st.input_read_pos.set(0);
            st.input_write_pos.set(0);
        }
    });

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

    let no_move = osi_st(|st| {
        st.orig_mouse_grid.get() == grid
            && st.orig_mouse_col.get() == col
            && st.orig_mouse_row.get() == row
    });
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
        let mouset = (opt_vars().p_mouset.get() as u64).wrapping_mul(1_000_000);
        osi_st(|st| {
            let timediff = mouse_time.wrapping_sub(st.orig_mouse_time.get());
            let clicks = st.orig_num_clicks.get();
            if code == st.orig_mouse_code.get() && no_move && timediff < mouset && clicks != 4 {
                st.orig_num_clicks.set(clicks + 1);
            } else {
                st.orig_num_clicks.set(1);
            }
            st.orig_mouse_code.set(code);
            st.orig_mouse_time.set(mouse_time);
        });
    } else if !no_move {
        // For drag and release events the number of clicks is kept.
        // However, if the mouse has moved, the next click should reset the click count.
        osi_st(|st| st.orig_mouse_code.set(code));
    }

    let num_clicks = osi_st(|st| {
        st.orig_mouse_grid.set(grid);
        st.orig_mouse_col.set(col);
        st.orig_mouse_row.set(row);
        st.orig_num_clicks.get()
    });

    let mut modifiers: u8 = 0;
    if code != KE_MOUSEMOVE {
        if num_clicks == 2 {
            modifiers |= MOD_MASK_2CLICK;
        } else if num_clicks == 3 {
            modifiers |= MOD_MASK_3CLICK;
        } else if num_clicks == 4 {
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
            if col >= g_st().columns_global.get() {
                col = g_st().columns_global.get() - 1;
            }
            if row >= g_st().rows_global.get() {
                row = g_st().rows_global.get() - 1;
            }
            g_st().mouse_grid.set(0);
            g_st().mouse_row.set(row);
            g_st().mouse_col.set(col);
        }
        *pos += advance;
    }

    let mut skip_event = false;
    let modifiers = check_multiclick(
        mouse_code,
        g_st().mouse_grid.get(),
        g_st().mouse_row.get(),
        g_st().mouse_col.get(),
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

    g_st().mouse_grid.set(grid);
    g_st().mouse_row.set(row);
    g_st().mouse_col.set(col);

    input_enqueue_raw(&buf[..idx + 3]);
}

/// @return true if the main loop is blocked and waiting for input.
pub fn input_blocking() -> bool {
    osi_st(|st| st.blocking.get())
}

/// Checks for (but does not read) available input, and consumes `main_loop.events` while waiting.
fn inbuf_poll(ms: i32, events_opt: Option<MultiQueue>) -> bool {
    if os_input_ready(events_opt) {
        return true;
    }

    if ms == 0 {
        loop_poll_events(&mut *main_loop_ptr(), 0);
        return os_input_ready(events_opt);
    }

    if g_st().do_profiling.get() == PROF_YES {
        prof_input_start();
    }

    if events_opt != Some(main_loop_events()) {
        // The pending input provoked a osi_st().blocking wait. Do special events now. #6247
        osi_st(|st| st.blocking.set(true));
        multiqueue_process_events(g_st().ch_before_blocking_events.get());
    }

    // LOOP_PROCESS_EVENTS_UNTIL(&main_loop, NULL, ms, os_input_ready(events) || input_unanswerable());
    let timeout_ms = i64::from(ms);
    let start = if timeout_ms >= 0 { os_hrtime() } else { 0 };
    let mut remaining = timeout_ms;
    let mut expired = false;
    while !(os_input_ready(events_opt) || input_unanswerable()) && !expired {
        loop_poll_events(&mut *main_loop_ptr(), remaining);
        if timeout_ms >= 0 {
            remaining = timeout_ms - ((os_hrtime() - start) / 1_000_000) as i64;
            expired = remaining <= 0;
        }
    }

    osi_st(|st| st.blocking.set(false));

    if g_st().do_profiling.get() == PROF_YES {
        prof_input_end();
    }

    if input_unanswerable() && !os_input_ready(events_opt) {
        // Dismiss the prompt (CTRL-C) instead of waiting forever.
        static DID_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !DID_LOG.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let mut mode = [0u8; MODE_MAX_LENGTH];
            get_mode(&mut mode);
            let mode_cstr = std::ffi::CStr::from_bytes_until_nul(&mode).unwrap_or(c"");
            logmsg!(
                LOGLVL_WRN,
                None,
                Some(c"inbuf_poll"),
                line!() as i32,
                true,
                c"Interrupted prompt (mode=%s): user input is unlikely",
                mode_cstr,
            );
        }
        input_enqueue_raw(b"\x03");
        // Also interrupt the code that prompted. Skips remaining exit autocmds.
        g_st().got_int.set(true);
    }

    os_input_ready(events_opt)
}

fn process_ctrl_c() {
    if !g_st().ctrl_c_interrupts.get() {
        return;
    }

    osi_st(|st| {
        let mut buf = st.input_buffer.borrow_mut();
        let rpos = st.input_read_pos.get();
        let wpos = st.input_write_pos.get();
        let available = wpos.saturating_sub(rpos);
        let mut i = available as isize - 1;
        while i >= 0 {
            let idx = rpos + (i as usize);
            let c = buf[idx];
            if c == CTRL_C_CONST_CONST_CONST
                || (c == b'C'
                    && i >= 3
                    && buf[idx - 3] == K_SPECIAL
                    && buf[idx - 2] == KS_MODIFIER
                    && buf[idx - 1] == MOD_MASK_CTRL)
            {
                buf[idx] = CTRL_C_CONST_CONST_CONST;
                g_st().got_int.set(true);
                break;
            }
            i -= 1;
        }

        if g_st().got_int.get() && i > 0 {
            // Remove all unprocessed input (typeahead) before the CTRL-C.
            st.input_read_pos.set(rpos + (i as usize));
        }
    });
}

/// Pushes bytes from the "event" key sequence (KE_EVENT) partially between calls to input_get when
/// `buf.len() < 3`.
fn push_event_key(buf: &mut [u8]) -> i32 {
    const KEY: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_EVENT];
    let mut buf_idx: usize = 0;

    osi_st(|st| {
        while buf_idx < buf.len() {
            let key_idx = st.push_event_key_idx.get();
            buf[buf_idx] = KEY[key_idx];
            buf_idx += 1;
            let next_idx = (key_idx + 1) % 3;
            st.push_event_key_idx.set(next_idx);
            if next_idx == 0 {
                break;
            }
        }
    });

    buf_idx as i32
}

/// Check if there's pending input already in typebuf or `events`
pub fn os_input_ready(events: Option<MultiQueue>) -> bool {
    g_st().typebuf_was_filled.get() || input_available() != 0 || pending_events(events)
}

fn pending_events(events: Option<MultiQueue>) -> bool {
    events.is_some_and(|ev| !multiqueue_empty(ev))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_enqueue_raw_and_push_event_key_work() {
        osi_st(|st| {
            st.input_read_pos.set(0);
            st.input_write_pos.set(0);
        });
        assert_eq!(input_available(), 0);
        input_enqueue_raw(b"abc");
        assert_eq!(input_available(), 3);
        osi_st(|st| {
            st.input_read_pos.set(0);
            st.input_write_pos.set(0);
            st.push_event_key_idx.set(0);
        });
        let mut buf = [0u8; 3];
        assert_eq!(push_event_key(&mut buf[..2]), 2);
        assert_eq!(buf[0], K_SPECIAL);
        assert_eq!(buf[1], KS_EXTRA);
        assert_eq!(push_event_key(&mut buf[2..]), 1);
        assert_eq!(buf[2], KE_EVENT);

        assert_eq!(parse_mouse_coords(b"<12,34>rest"), Some((12, 34, 7)));
        assert_eq!(parse_mouse_coords(b"<-1,0>"), Some((-1, 0, 6)));
        assert_eq!(parse_mouse_coords(b"<12,34"), None);
    }
}
