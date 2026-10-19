//! OS input handling (`src/nvim/os/input.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::globals::*;
use crate::option_vars::*;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::eval::deprecated::buf_T;
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_empty, multiqueue_process_events,
    multiqueue_put_event,
};
use crate::keycodes::trans_special;
use crate::map_glyph_cache::NvimString;
use crate::os::time::{os_hrtime, os_realtime};
use crate::profile::{prof_input_end, prof_input_start};
use crate::state::get_real_state;

pub type OptInt = i64;
pub type varnumber_T = i64;
pub type Loop = c_void;

const READ_BUFFER_SIZE: usize = 0xfff;
const MAX_KEY_CODE_LEN: usize = 6;
const INPUT_BUFFER_SIZE: usize = (READ_BUFFER_SIZE * 4) + MAX_KEY_CODE_LEN;

const MODE_INSERT: c_int = 0x10;
const EVENT_CURSORHOLD: c_int = 38;
const EVENT_CURSORHOLDI: c_int = 39;

const K_SPECIAL: u8 = 0x80;
const KS_SPECIAL: u8 = 254;
const KS_EXTRA: u8 = 253;
const KS_MODIFIER: u8 = 252;
const KE_FILLER: u8 = b'X';

const KE_LEFTMOUSE: c_int = 44;
const KE_RIGHTRELEASE: c_int = 52;
const KE_MOUSEDOWN: c_int = 75;
const KE_MOUSERIGHT: c_int = 78;
const KE_X1MOUSE: c_int = 89;
const KE_X2RELEASE: c_int = 94;
const KE_MOUSEMOVE: c_int = 100;
const KE_EVENT: u8 = 102;
const KE_COMPLETE_DELAY: u8 = 110;

const MOD_MASK_CTRL: u8 = 0x04;
const MOD_MASK_2CLICK: u8 = 0x20;
const MOD_MASK_3CLICK: u8 = 0x40;
const MOD_MASK_4CLICK: u8 = 0x60;

const FSK_KEYCODE: c_int = 0x01;
const Ctrl_C: u8 = 3;
const EOF: c_int = -1;

const VV_USERACTIVE: c_int = 106;
const PROF_YES: c_int = 1;
const UV_TTY: c_int = 14;
const BREAKCHECK_SKIP: c_int = 1000;

pub use crate::mapping::typebuf_T;

static mut input_buffer: [c_char; INPUT_BUFFER_SIZE] = [0; INPUT_BUFFER_SIZE];
static mut input_read_pos: *mut c_char = ptr::null_mut();
static mut input_write_pos: *mut c_char = ptr::null_mut();

static mut blocking: bool = false;
static mut cursorhold_time: c_int = 0;
static mut cursorhold_tb_change_cnt: c_int = 0;
static mut breakcheck_count: c_int = 0;

static mut orig_num_clicks: c_int = 0;
static mut orig_mouse_code: c_int = 0;
static mut orig_mouse_grid: c_int = 0;
static mut orig_mouse_col: c_int = 0;
static mut orig_mouse_row: c_int = 0;
static mut orig_mouse_time: u64 = 0;

static mut push_event_key_idx: usize = 0;

#[inline]
unsafe fn input_buf_ptr() -> *mut c_char {
    (&raw mut input_buffer).cast::<c_char>()
}

#[inline]
unsafe fn ensure_input_ptrs_init() {
    unsafe {
        if input_read_pos.is_null() {
            input_read_pos = input_buf_ptr();
            input_write_pos = input_buf_ptr();
        }
    }
}

unsafe extern "C" {
    static mut main_loop: [u8; 0];

    fn uv_loop_size() -> usize;
    fn uv_guess_handle(file: c_int) -> c_int;
    fn loop_poll_events(loop_: *mut Loop, ms: i64) -> bool;

    fn apply_autocmds(
        event: c_int,
        fname: *mut c_char,
        fname_io: *mut c_char,
        force: bool,
        buf: *mut buf_T,
    ) -> bool;
    fn trigger_cursorhold() -> bool;

    fn ins_compl_autocomplete_pending() -> bool;
    fn ins_compl_active() -> bool;
    fn ins_compl_autocomplete_elapsed() -> i64;

    fn typebuf_changed(tb_change_cnt: c_int) -> bool;
    fn before_blocking();
    fn getout(r: c_int) -> !;
    fn set_vim_var_nr(idx: c_int, val: varnumber_T);
    fn get_mouse_button(code: c_int, is_click: *mut bool, is_drag: *mut bool) -> c_int;

    fn sscanf(s: *const c_char, format: *const c_char, ...) -> c_int;
}

#[inline]
unsafe fn main_loop_ptr() -> *mut Loop {
    (&raw mut main_loop).cast()
}

#[inline]
unsafe fn main_loop_events() -> *mut MultiQueue {
    unsafe {
        *(&raw mut main_loop)
            .cast::<u8>()
            .add(uv_loop_size())
            .cast::<*mut MultiQueue>()
    }
}

unsafe extern "C" fn cursorhold_event(_argv: *mut *mut c_void) {
    unsafe {
        let event = if (State & MODE_INSERT) != 0 {
            EVENT_CURSORHOLDI
        } else {
            EVENT_CURSORHOLD
        };
        apply_autocmds(event, ptr::null_mut(), ptr::null_mut(), false, curbuf);
        did_cursorhold = true;
    }
}

unsafe fn create_cursorhold_event(events_enabled: bool) {
    unsafe {
        let events = main_loop_events();
        debug_assert!(!events_enabled || multiqueue_empty(events));
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        multiqueue_put_event(
            events,
            Event {
                handler: Some(cursorhold_event),
                argv,
            },
        );
        let _ = &mut argv;
    }
}

unsafe fn reset_cursorhold_wait(tb_change_cnt: c_int) {
    unsafe {
        cursorhold_time = 0;
        cursorhold_tb_change_cnt = tb_change_cnt;
    }
}

/// Reads OS input into `buf`, and consumes pending events while waiting (if `ms != 0`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn input_get(
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
        if tb_change_cnt != cursorhold_tb_change_cnt {
            reset_cursorhold_wait(tb_change_cnt);
        }

        if maxlen != 0 && input_available() != 0 {
            reset_cursorhold_wait(tb_change_cnt);
            let to_read = (maxlen as usize).min(input_available());
            ptr::copy_nonoverlapping(input_read_pos.cast::<u8>(), buf, to_read);
            input_read_pos = input_read_pos.add(to_read);
            debug_assert!(to_read <= c_int::MAX as usize);
            return to_read as c_int;
        }

        // No risk of a UI flood, so disable CTRL-C "interrupt" behavior if it's mapped.
        if ((mapped_ctrl_c | (*curbuf).b_mapped_ctrl_c) & get_real_state()) != 0 {
            ctrl_c_interrupts = false;
        }

        if ms >= 0 {
            if !inbuf_poll(ms, events) {
                return 0;
            }
        } else {
            let wait_start = os_hrtime();
            cursorhold_time = cursorhold_time.min(p_ut as c_int);
            let delay_pending = ins_compl_autocomplete_pending()
                && p_acl > 0
                && typebuf.tb_len == 0
                && !ins_compl_active()
                && (get_real_state() & MODE_INSERT) != 0;
            let mut wait_time = p_ut - i64::from(cursorhold_time);
            if delay_pending {
                let delay_left = p_acl - ins_compl_autocomplete_elapsed();
                wait_time = delay_left.max(0).min(wait_time);
            }
            if !inbuf_poll(wait_time as c_int, events) {
                if silent_mode {
                    // Ran out of input: exit silent/batch-mode (-es/-Es).
                    getout(0);
                }
                if delay_pending
                    && ins_compl_autocomplete_elapsed() >= p_acl
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
                cursorhold_time += ((os_hrtime() - wait_start) / 1_000_000) as c_int;
            }
        }

        ctrl_c_interrupts = true;

        // If input was put directly in typeahead buffer bail out here.
        if typebuf_changed(tb_change_cnt) {
            return 0;
        }

        if maxlen != 0 && input_available() != 0 {
            reset_cursorhold_wait(tb_change_cnt);
            let to_read = (maxlen as usize).min(input_available());
            ptr::copy_nonoverlapping(input_read_pos.cast::<u8>(), buf, to_read);
            input_read_pos = input_read_pos.add(to_read);
            debug_assert!(to_read <= c_int::MAX as usize);
            return to_read as c_int;
        }

        // If there are events, return the keys directly
        if maxlen != 0 && pending_events(events) {
            return push_event_key(buf, maxlen);
        }

        0
    }
}

/// Check if a character is available for reading
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_char_avail() -> bool {
    unsafe { inbuf_poll(0, ptr::null_mut()) }
}

/// Poll for fast events. `got_int` will be set to `true` if CTRL-C was typed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_breakcheck() {
    unsafe {
        if got_int {
            return;
        }

        loop_poll_events(main_loop_ptr(), 0);
    }
}

/// Check for CTRL-C pressed, but only once in a while.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn line_breakcheck() {
    unsafe {
        breakcheck_count += 1;
        if breakcheck_count >= BREAKCHECK_SKIP {
            breakcheck_count = 0;
            os_breakcheck();
        }
    }
}

/// Like line_breakcheck() but check 10 times less often.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fast_breakcheck() {
    unsafe {
        breakcheck_count += 1;
        if breakcheck_count >= BREAKCHECK_SKIP * 10 {
            breakcheck_count = 0;
            os_breakcheck();
        }
    }
}

/// Like line_breakcheck() but check 100 times less often.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn veryfast_breakcheck() {
    unsafe {
        breakcheck_count += 1;
        if breakcheck_count >= BREAKCHECK_SKIP * 100 {
            breakcheck_count = 0;
            os_breakcheck();
        }
    }
}

/// Test whether a file descriptor refers to a terminal.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_isatty(fd: c_int) -> bool {
    unsafe { uv_guess_handle(fd) == UV_TTY }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn input_available() -> usize {
    unsafe {
        ensure_input_ptrs_init();
        (input_write_pos as usize).wrapping_sub(input_read_pos as usize)
    }
}

unsafe fn input_space() -> usize {
    unsafe {
        ensure_input_ptrs_init();
        let buf_end = input_buf_ptr().add(INPUT_BUFFER_SIZE);
        (buf_end as usize).wrapping_sub(input_write_pos as usize)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn input_enqueue_raw(data: *const c_char, size: usize) {
    unsafe {
        ensure_input_ptrs_init();
        let buf_start = input_buf_ptr();
        if (input_read_pos as usize) > (buf_start as usize) {
            let available = input_available();
            ptr::copy(input_read_pos, buf_start, available);
            input_read_pos = buf_start;
            input_write_pos = buf_start.add(available);
        }

        let to_write = size.min(input_space());
        ptr::copy_nonoverlapping(data, input_write_pos, to_write);
        input_write_pos = input_write_pos.add(to_write);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn input_enqueue(chan_id: u64, keys: NvimString) -> usize {
    unsafe {
        ensure_input_ptrs_init();
        current_ui = chan_id;

        if input_read_pos == input_write_pos {
            input_read_pos = input_buf_ptr();
            input_write_pos = input_buf_ptr();
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
                ptr::null_mut(),
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

unsafe fn check_multiclick(
    code: c_int,
    grid: c_int,
    row: c_int,
    col: c_int,
    skip_event: *mut bool,
) -> u8 {
    unsafe {
        if (KE_MOUSEDOWN..=KE_MOUSERIGHT).contains(&code) {
            return 0;
        }

        let no_move = orig_mouse_grid == grid && orig_mouse_col == col && orig_mouse_row == row;
        let mut is_click = false;
        let mut is_drag = false;
        let _ = get_mouse_button(code, &raw mut is_click, &raw mut is_drag);

        if is_drag && no_move {
            *skip_event = true;
            return 0;
        }

        if is_click {
            // For click events the number of clicks is updated.
            let mouse_time = os_hrtime(); // time of current mouse click (ns)
            let timediff = mouse_time.wrapping_sub(orig_mouse_time);
            let mouset = (p_mouset as u64).wrapping_mul(1_000_000);
            if code == orig_mouse_code && no_move && timediff < mouset && orig_num_clicks != 4 {
                orig_num_clicks += 1;
            } else {
                orig_num_clicks = 1;
            }
            orig_mouse_code = code;
            orig_mouse_time = mouse_time;
        } else if !no_move {
            // For drag and release events the number of clicks is kept.
            // However, if the mouse has moved, the next click should reset the click count.
            orig_mouse_code = code;
        }

        orig_mouse_grid = grid;
        orig_mouse_col = col;
        orig_mouse_row = row;

        let mut modifiers: u8 = 0;
        if code != KE_MOUSEMOVE {
            if orig_num_clicks == 2 {
                modifiers |= MOD_MASK_2CLICK;
            } else if orig_num_clicks == 3 {
                modifiers |= MOD_MASK_3CLICK;
            } else if orig_num_clicks == 4 {
                modifiers |= MOD_MASK_4CLICK;
            }
        }
        modifiers
    }
}

/// Mouse event handling code (extract row/col if available and detect multiple clicks)
unsafe fn handle_mouse_event(
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

        let mut col: c_int = 0;
        let mut row: c_int = 0;
        let mut advance: c_int = 0;
        if sscanf(
            *ptr_p,
            c"<%d,%d>%n".as_ptr(),
            &raw mut col,
            &raw mut row,
            &raw mut advance,
        ) != EOF
            && advance != 0
        {
            if col >= 0 && row >= 0 {
                if col >= Columns {
                    col = Columns - 1;
                }
                if row >= Rows {
                    row = Rows - 1;
                }
                mouse_grid = 0;
                mouse_row = row;
                mouse_col = col;
            }
            *ptr_p = (*ptr_p).offset(advance as isize);
        }

        let mut skip_event = false;
        let modifiers = check_multiclick(
            mouse_code,
            mouse_grid,
            mouse_row,
            mouse_col,
            &raw mut skip_event,
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn input_enqueue_mouse(
    code: c_int,
    mut modifier: u8,
    grid: c_int,
    row: c_int,
    col: c_int,
) {
    unsafe {
        let mut skip_event = false;
        modifier |= check_multiclick(code, grid, row, col, &raw mut skip_event);
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

        mouse_grid = grid;
        mouse_row = row;
        mouse_col = col;

        let written = 3 + (p as usize).wrapping_sub(buf.as_ptr() as usize);
        input_enqueue_raw(buf.as_ptr().cast(), written);
    }
}

/// @return true if the main loop is blocked and waiting for input.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn input_blocking() -> bool {
    unsafe { blocking }
}

/// Checks for (but does not read) available input, and consumes `main_loop.events` while waiting.
unsafe fn inbuf_poll(ms: c_int, events: *mut MultiQueue) -> bool {
    unsafe {
        if os_input_ready(events) {
            return true;
        }

        if do_profiling == PROF_YES && ms != 0 {
            prof_input_start();
        }

        if (ms == -1 || ms > 0) && events != main_loop_events() {
            // The pending input provoked a blocking wait. Do special events now. #6247
            blocking = true;
            multiqueue_process_events(ch_before_blocking_events);
        }

        // LOOP_PROCESS_EVENTS_UNTIL(&main_loop, NULL, ms, os_input_ready(events));
        let mut remaining = i64::from(ms);
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !os_input_ready(events) {
            loop_poll_events(main_loop_ptr(), remaining);
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

        blocking = false;

        if do_profiling == PROF_YES && ms != 0 {
            prof_input_end();
        }

        os_input_ready(events)
    }
}

unsafe fn process_ctrl_c() {
    unsafe {
        if !ctrl_c_interrupts {
            return;
        }

        let available = input_available();
        let mut i = available as isize - 1;
        while i >= 0 {
            let c = *input_read_pos.offset(i) as u8;
            if c == Ctrl_C
                || (c == b'C'
                    && i >= 3
                    && *input_read_pos.offset(i - 3) as u8 == K_SPECIAL
                    && *input_read_pos.offset(i - 2) as u8 == KS_MODIFIER
                    && *input_read_pos.offset(i - 1) as u8 == MOD_MASK_CTRL)
            {
                *input_read_pos.offset(i) = Ctrl_C as c_char;
                got_int = true;
                break;
            }
            i -= 1;
        }

        if got_int && i > 0 {
            // Remove all unprocessed input (typeahead) before the CTRL-C.
            input_read_pos = input_read_pos.offset(i);
        }
    }
}

/// Pushes bytes from the "event" key sequence (KE_EVENT) partially between calls to input_get when
/// `maxlen < 3`.
unsafe fn push_event_key(buf: *mut u8, maxlen: c_int) -> c_int {
    const KEY: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_EVENT];
    unsafe {
        let mut buf_idx: c_int = 0;

        loop {
            *buf.add(buf_idx as usize) = KEY[push_event_key_idx];
            buf_idx += 1;
            push_event_key_idx = (push_event_key_idx + 1) % 3;
            if push_event_key_idx == 0 || buf_idx >= maxlen {
                break;
            }
        }

        buf_idx
    }
}

/// Check if there's pending input already in typebuf or `events`
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_input_ready(events: *mut MultiQueue) -> bool {
    unsafe { typebuf_was_filled || input_available() != 0 || pending_events(events) }
}

unsafe fn pending_events(events: *mut MultiQueue) -> bool {
    unsafe { !events.is_null() && !multiqueue_empty(events) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_enqueue_raw_and_push_event_key_work() {
        unsafe {
            input_read_pos = input_buf_ptr();
            input_write_pos = input_buf_ptr();
            assert_eq!(input_available(), 0);
            input_enqueue_raw(c"abc".as_ptr(), 3);
            assert_eq!(input_available(), 3);
            input_read_pos = input_buf_ptr();
            input_write_pos = input_buf_ptr();

            push_event_key_idx = 0;
            let mut buf = [0u8; 3];
            assert_eq!(push_event_key(buf.as_mut_ptr(), 2), 2);
            assert_eq!(buf[0], K_SPECIAL);
            assert_eq!(buf[1], KS_EXTRA);
            assert_eq!(push_event_key(buf.as_mut_ptr().add(2), 2), 1);
            assert_eq!(buf[2], KE_EVENT);
        }
    }
}
