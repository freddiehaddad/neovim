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
    KS_SPECIAL, get_mouse_button, trans_special,
};
use crate::log::{LOGLVL_WRN, logmsg};
use crate::main_init::{getout, input_unanswerable};
use crate::option_vars::*;
use crate::os::time::{os_hrtime, os_realtime};
use crate::profile::{prof_input_end, prof_input_start};
use crate::state::{MODE_INSERT, MODE_MAX_LENGTH, get_mode, get_real_state};
use std::ffi::{c_char, c_int, c_uint};
use std::ptr;

use crate::eval::typval::varnumber_T;

const READ_BUFFER_SIZE: usize = 0xfff;
const MAX_KEY_CODE_LEN: usize = 6;
const INPUT_BUFFER_SIZE: usize = (READ_BUFFER_SIZE * 4) + MAX_KEY_CODE_LEN;

const KE_LEFTMOUSE: c_int = 44;
const KE_RIGHTRELEASE: c_int = 52;
const KE_MOUSEDOWN: c_int = 75;
const KE_MOUSERIGHT: c_int = 78;
const KE_X1MOUSE: c_int = 89;
const KE_X2RELEASE: c_int = 94;
const KE_MOUSEMOVE: c_int = 100;

const MOD_MASK_CTRL: u8 = 0x04;
const MOD_MASK_2CLICK: u8 = 0x20;
const MOD_MASK_3CLICK: u8 = 0x40;
const MOD_MASK_4CLICK: u8 = 0x60;

const Ctrl_C: u8 = 3;

const BREAKCHECK_SKIP: c_int = 1000;

struct OsInputState {
    input_buffer: [c_char; INPUT_BUFFER_SIZE],
    input_read_pos: *mut c_char,
    input_write_pos: *mut c_char,
    blocking: bool,
    cursorhold_time: c_int,
    cursorhold_tb_change_cnt: c_int,
    breakcheck_count: c_int,
    orig_num_clicks: c_int,
    orig_mouse_code: c_int,
    orig_mouse_grid: c_int,
    orig_mouse_col: c_int,
    orig_mouse_row: c_int,
    orig_mouse_time: u64,
    push_event_key_idx: usize,
}

impl OsInputState {
    const fn new() -> Self {
        Self {
            input_buffer: [0; INPUT_BUFFER_SIZE],
            input_read_pos: ptr::null_mut(),
            input_write_pos: ptr::null_mut(),
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

#[inline]
fn input_buf_ptr() -> *mut c_char {
    (&raw mut osi_st().input_buffer).cast::<c_char>()
}

#[inline]
fn ensure_input_ptrs_init() {
    if osi_st().input_read_pos.is_null() {
            osi_st().input_read_pos = input_buf_ptr();
            osi_st().input_write_pos = input_buf_ptr();
        }
}

fn parse_mouse_coords(s: impl crate::strings::IntoCCharPtr) -> Option<(c_int, c_int, usize)> {
    let s = s.into_c_char_ptr();
    unsafe {
        if s.is_null() || *s as u8 != b'<' {
            return None;
        }
        let mut p = s.add(1);
        let parse_int = |mut cur: *const c_char| -> Option<(c_int, *const c_char)> {
            while matches!(*cur as u8, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                cur = cur.add(1);
            }
            let neg = if *cur as u8 == b'-' {
                cur = cur.add(1);
                true
            } else {
                if *cur as u8 == b'+' {
                    cur = cur.add(1);
                }
                false
            };
            if !(*cur as u8).is_ascii_digit() {
                return None;
            }
            let mut acc: i64 = 0;
            while (*cur as u8).is_ascii_digit() {
                acc = acc
                    .saturating_mul(10)
                    .saturating_add(i64::from((*cur as u8) - b'0'));
                cur = cur.add(1);
            }
            let val = if neg { -acc } else { acc } as c_int;
            Some((val, cur))
        };
        let (col, after_col) = parse_int(p)?;
        p = after_col;
        if *p as u8 != b',' {
            return None;
        }
        p = p.add(1);
        let (row, after_row) = parse_int(p)?;
        p = after_row;
        if *p as u8 != b'>' {
            return None;
        }
        p = p.add(1);
        Some((col, row, p.offset_from(s) as usize))
    }
}

use crate::event::stream::UV_TTY;

pub fn cursorhold_event() {
    let event = if (g_st().State & MODE_INSERT) != 0 {
            EVENT_CURSORHOLDI
        } else {
            EVENT_CURSORHOLD
        };
        apply_autocmds(event, ptr::null_mut(), ptr::null_mut(), false, g_st().curbuf);
        g_st().did_cursorhold = true;
}

fn create_cursorhold_event(events_enabled: bool) {
    let events = main_loop_events();
        debug_assert!(!events_enabled || multiqueue_empty(unsafe { &*events }));
        multiqueue_put_event(unsafe { &mut *events }, Event::CursorHold);
}

fn reset_cursorhold_wait(tb_change_cnt: c_int) {
    osi_st().cursorhold_time = 0;
        osi_st().cursorhold_tb_change_cnt = tb_change_cnt;
}

/// Reads OS input into `buf`, and consumes pending events while waiting (if `ms != 0`).
pub fn input_get(
    buf: *mut u8,
    maxlen: c_int,
    ms: c_int,
    tb_change_cnt: c_int,
    events: *mut MultiQueue,
) -> c_int {
    unsafe {
        ensure_input_ptrs_init();
        debug_assert!((maxlen > 0 && !buf.is_null()) || maxlen == 0);
        // This check is needed so that feeding typeahead from RPC can prevent CursorHold.
        if tb_change_cnt != osi_st().cursorhold_tb_change_cnt {
            reset_cursorhold_wait(tb_change_cnt);
        }

        if maxlen != 0 && input_available() != 0 {
            reset_cursorhold_wait(tb_change_cnt);
            let to_read = (maxlen as usize).min(input_available());
            ptr::copy_nonoverlapping(osi_st().input_read_pos.cast::<u8>(), buf, to_read);
            osi_st().input_read_pos = osi_st().input_read_pos.add(to_read);
            debug_assert!(to_read <= c_int::MAX as usize);
            return to_read as c_int;
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
            osi_st().cursorhold_time = osi_st().cursorhold_time.min(opt_vars().p_ut as c_int);
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
            if !inbuf_poll(wait_time as c_int, events) {
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
                        input_enqueue_raw(ibuf.as_ptr().cast(), 3);
                        return 0;
                    }
                    *buf = K_SPECIAL;
                    *buf.add(1) = KS_EXTRA;
                    *buf.add(2) = KE_COMPLETE_DELAY;
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
                osi_st().cursorhold_time += ((os_hrtime() - wait_start) / 1_000_000) as c_int;
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
            ptr::copy_nonoverlapping(osi_st().input_read_pos.cast::<u8>(), buf, to_read);
            osi_st().input_read_pos = osi_st().input_read_pos.add(to_read);
            debug_assert!(to_read <= c_int::MAX as usize);
            return to_read as c_int;
        }

        // If there are events, return the keys directly
        if maxlen != 0 && pending_events(events.as_ref()) {
            return push_event_key(buf, maxlen);
        }

        0
    }
}

/// Check if a character is available for reading
pub fn os_char_avail() -> bool {
    inbuf_poll(0, ptr::null_mut())
}

/// Poll for fast events. `got_int` will be set to `true` if CTRL-C was typed.
pub fn os_breakcheck() {
    unsafe {
        if g_st().got_int {
            return;
        }

        loop_poll_events(&mut *main_loop_ptr(), 0);
    }
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
pub fn os_isatty(fd: c_int) -> bool {
    unsafe { uv_guess_handle(fd) == UV_TTY }
}

pub fn input_available() -> usize {
    ensure_input_ptrs_init();
        (osi_st().input_write_pos as usize).wrapping_sub(osi_st().input_read_pos as usize)
}

fn input_space() -> usize {
    unsafe {
        ensure_input_ptrs_init();
        let buf_end = input_buf_ptr().add(INPUT_BUFFER_SIZE);
        (buf_end as usize).wrapping_sub(osi_st().input_write_pos as usize)
    }
}

pub fn input_enqueue_raw(data: impl crate::strings::IntoCCharPtr, size: usize) {
    let data = data.into_c_char_ptr();
    unsafe {
        ensure_input_ptrs_init();
        let buf_start = input_buf_ptr();
        if (osi_st().input_read_pos as usize) > (buf_start as usize) {
            let available = input_available();
            ptr::copy(osi_st().input_read_pos, buf_start, available);
            osi_st().input_read_pos = buf_start;
            osi_st().input_write_pos = buf_start.add(available);
        }

        let to_write = size.min(input_space());
        ptr::copy_nonoverlapping(data, osi_st().input_write_pos, to_write);
        osi_st().input_write_pos = osi_st().input_write_pos.add(to_write);
    }
}

pub fn input_enqueue(chan_id: u64, keys: NvimString) -> usize {
    unsafe {
        ensure_input_ptrs_init();
        g_st().current_ui = chan_id;

        if osi_st().input_read_pos == osi_st().input_write_pos {
            osi_st().input_read_pos = input_buf_ptr();
            osi_st().input_write_pos = input_buf_ptr();
        }

        if keys.size > 0 {
            set_vim_var_nr(VV_USERACTIVE, os_realtime() as varnumber_T);
        }

        let mut ptr_cur: *const c_char = keys.data.cast_const();
        let end = ptr_cur.add(keys.size);

        while input_space() >= 19 && (ptr_cur as usize) < (end as usize) {
            let mut buf = [0u8; 19];
            let mut new_size = trans_special(
                &raw mut ptr_cur,
                (end as usize).wrapping_sub(ptr_cur as usize),
                buf.as_mut_ptr().cast(),
                FSK_KEYCODE,
                true,
                None,
            );

            if new_size > 0 {
                new_size = handle_mouse_event(&raw mut ptr_cur, buf.as_mut_ptr(), new_size);
                if new_size > 0 {
                    input_enqueue_raw(buf.as_ptr().cast(), new_size as usize);
                }
                continue;
            }

            if *ptr_cur == b'<' as c_char {
                let old_ptr = ptr_cur;
                // Invalid or incomplete key sequence, skip until the next '>' or *end.
                loop {
                    ptr_cur = ptr_cur.add(1);
                    if (ptr_cur as usize) >= (end as usize) || *ptr_cur == b'>' as c_char {
                        break;
                    }
                }
                if (ptr_cur as usize) >= (end as usize) || *ptr_cur != b'>' as c_char {
                    // Incomplete key sequence, return without consuming.
                    ptr_cur = old_ptr;
                    break;
                }
                ptr_cur = ptr_cur.add(1);
                continue;
            }

            // copy the character, escaping K_SPECIAL
            if *ptr_cur as u8 == K_SPECIAL {
                let k_spec = K_SPECIAL as c_char;
                let ks_spec = KS_SPECIAL as c_char;
                let ke_fill = KE_FILLER as c_char;
                input_enqueue_raw(&raw const k_spec, 1);
                input_enqueue_raw(&raw const ks_spec, 1);
                input_enqueue_raw(&raw const ke_fill, 1);
            } else {
                input_enqueue_raw(ptr_cur, 1);
            }
            ptr_cur = ptr_cur.add(1);
        }

        let rv = (ptr_cur as usize).wrapping_sub(keys.data as usize);
        process_ctrl_c();
        rv
    }
}

fn check_multiclick(
    code: c_int,
    grid: c_int,
    row: c_int,
    col: c_int,
    skip_event: &mut bool,
) -> u8 {
    {
        if (KE_MOUSEDOWN..=KE_MOUSERIGHT).contains(&code) {
            return 0;
        }

        let no_move = osi_st().orig_mouse_grid == grid && osi_st().orig_mouse_col == col && osi_st().orig_mouse_row == row;
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
            if code == osi_st().orig_mouse_code && no_move && timediff < mouset && osi_st().orig_num_clicks != 4 {
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
}

/// Mouse event handling code (extract row/col if available and detect multiple clicks)
fn handle_mouse_event(
    ptr_p: *mut *const c_char,
    buf: *mut u8,
    mut bufsize: c_uint,
) -> c_uint {
    unsafe {
        let mut mouse_code: c_int = 0;
        let mut key_type: c_int = 0;

        if bufsize == 3 {
            mouse_code = c_int::from(*buf.add(2));
            key_type = c_int::from(*buf.add(1));
        } else if bufsize == 6 {
            // prefixed with K_SPECIAL KS_MODIFIER mod
            mouse_code = c_int::from(*buf.add(5));
            key_type = c_int::from(*buf.add(4));
        }

        if key_type != c_int::from(KS_EXTRA)
            || !((KE_LEFTMOUSE..=KE_RIGHTRELEASE).contains(&mouse_code)
                || (KE_X1MOUSE..=KE_X2RELEASE).contains(&mouse_code)
                || (KE_MOUSEDOWN..=KE_MOUSERIGHT).contains(&mouse_code)
                || mouse_code == KE_MOUSEMOVE)
        {
            return bufsize;
        }

        if let Some((mut col, mut row, advance)) = parse_mouse_coords(*ptr_p) {
            if col >= 0 && row >= 0 {
                if col >= g_st().Columns {
                    col = g_st().Columns - 1;
                }
                if row >= g_st().Rows {
                    row = g_st().Rows - 1;
                }
                g_st().mouse_grid = 0;
                g_st().mouse_row = row;
                g_st().mouse_col = col;
            }
            *ptr_p = (*ptr_p).add(advance);
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
            if *buf.add(1) != KS_MODIFIER {
                // no modifiers in the buffer yet, shift the bytes 3 positions
                ptr::copy(buf, buf.add(3), 3);
                // add the modifier sequence
                *buf = K_SPECIAL;
                *buf.add(1) = KS_MODIFIER;
                *buf.add(2) = modifiers;
                bufsize += 3;
            } else {
                *buf.add(2) |= modifiers;
            }
        }

        bufsize
    }
}

pub fn input_enqueue_mouse(
    code: c_int,
    mut modifier: u8,
    grid: c_int,
    row: c_int,
    col: c_int,
) {
    unsafe {
        let mut skip_event = false;
        modifier |= check_multiclick(code, grid, row, col, &mut skip_event);
        if skip_event {
            return;
        }
        let mut buf = [0u8; 7];
        let mut p = buf.as_mut_ptr();
        if modifier != 0 {
            *p = K_SPECIAL;
            *p.add(1) = KS_MODIFIER;
            *p.add(2) = modifier;
            p = p.add(3);
        }
        *p = K_SPECIAL;
        *p.add(1) = KS_EXTRA;
        *p.add(2) = code as u8;

        g_st().mouse_grid = grid;
        g_st().mouse_row = row;
        g_st().mouse_col = col;

        let written = 3 + (p as usize).wrapping_sub(buf.as_ptr() as usize);
        input_enqueue_raw(buf.as_ptr().cast(), written);
    }
}

/// @return true if the main loop is blocked and waiting for input.
pub fn input_blocking() -> bool {
    osi_st().blocking
}

/// Checks for (but does not read) available input, and consumes `main_loop.events` while waiting.
fn inbuf_poll(ms: c_int, events: *mut MultiQueue) -> bool {
    unsafe {
        if os_input_ready(events.as_ref()) {
            return true;
        }

        if ms == 0 {
            loop_poll_events(&mut *main_loop_ptr(), 0);
            return os_input_ready(events.as_ref());
        }

        if g_st().do_profiling == PROF_YES {
            prof_input_start();
        }

        if events != main_loop_events() {
            // The pending input provoked a osi_st().blocking wait. Do special events now. #6247
            osi_st().blocking = true;
            multiqueue_process_events( &mut *g_st().ch_before_blocking_events);
        }

        // LOOP_PROCESS_EVENTS_UNTIL(&main_loop, NULL, ms, os_input_ready(events) || input_unanswerable());
        let mut remaining = i64::from(ms);
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !(os_input_ready(events.as_ref()) || input_unanswerable()) {
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

        if input_unanswerable() && !os_input_ready(events.as_ref()) {
            // Dismiss the prompt (CTRL-C) instead of waiting forever.
            static DID_LOG: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !DID_LOG.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let mut mode = [0 as c_char; MODE_MAX_LENGTH];
                get_mode(mode.as_mut_ptr());
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"inbuf_poll",
                    line!() as c_int,
                    true,
                    c"Interrupted prompt (mode=%s): user input is unlikely",
                    mode.as_ptr(),
                );
            }
            input_enqueue_raw(c"\x03", 1);
            // Also interrupt the code that prompted. Skips remaining exit autocmds.
            g_st().got_int = true;
        }

        os_input_ready(events.as_ref())
    }
}

fn process_ctrl_c() {
    unsafe {
        if !g_st().ctrl_c_interrupts {
            return;
        }

        let available = input_available();
        let mut i = available as isize - 1;
        while i >= 0 {
            let c = *osi_st().input_read_pos.offset(i) as u8;
            if c == Ctrl_C
                || (c == b'C'
                    && i >= 3
                    && *osi_st().input_read_pos.offset(i - 3) as u8 == K_SPECIAL
                    && *osi_st().input_read_pos.offset(i - 2) as u8 == KS_MODIFIER
                    && *osi_st().input_read_pos.offset(i - 1) as u8 == MOD_MASK_CTRL)
            {
                *osi_st().input_read_pos.offset(i) = Ctrl_C as c_char;
                g_st().got_int = true;
                break;
            }
            i -= 1;
        }

        if g_st().got_int && i > 0 {
            // Remove all unprocessed input (typeahead) before the CTRL-C.
            osi_st().input_read_pos = osi_st().input_read_pos.offset(i);
        }
    }
}

/// Pushes bytes from the "event" key sequence (KE_EVENT) partially between calls to input_get when
/// `maxlen < 3`.
fn push_event_key(buf: *mut u8, maxlen: c_int) -> c_int {
    const KEY: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_EVENT];
    unsafe {
        let mut buf_idx: c_int = 0;

        loop {
            *buf.add(buf_idx as usize) = KEY[osi_st().push_event_key_idx];
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
    events.is_some() && !multiqueue_empty( events.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_enqueue_raw_and_push_event_key_work() {
        unsafe {
            osi_st().input_read_pos = input_buf_ptr();
            osi_st().input_write_pos = input_buf_ptr();
            assert_eq!(input_available(), 0);
            input_enqueue_raw(c"abc", 3);
            assert_eq!(input_available(), 3);
            osi_st().input_read_pos = input_buf_ptr();
            osi_st().input_write_pos = input_buf_ptr();

            osi_st().push_event_key_idx = 0;
            let mut buf = [0u8; 3];
            assert_eq!(push_event_key(buf.as_mut_ptr(), 2), 2);
            assert_eq!(buf[0], K_SPECIAL);
            assert_eq!(buf[1], KS_EXTRA);
            assert_eq!(push_event_key(buf.as_mut_ptr().add(2), 2), 1);
            assert_eq!(buf[2], KE_EVENT);

            assert_eq!(
                parse_mouse_coords(c"<12,34>rest"),
                Some((12, 34, 7))
            );
            assert_eq!(parse_mouse_coords(c"<-1,0>"), Some((-1, 0, 6)));
            assert_eq!(parse_mouse_coords(c"<12,34"), None);
        }
    }
}
