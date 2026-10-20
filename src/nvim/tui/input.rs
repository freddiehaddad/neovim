//! Terminal input stream and key event handling (`src/nvim/tui/input.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::api::private::helpers::{boolean_obj, integer_obj, string_obj};
use crate::api::private::validate::Array;
#[cfg(not(test))]
use crate::event::r#loop::{
    loop_schedule_fast, uv_close, uv_handle_size, uv_timer_init, uv_timer_start, uv_timer_stop,
};
#[cfg(not(test))]
use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event};
use crate::event::rstream::{
    RStream, rstream_init_fd, rstream_may_close, rstream_size, rstream_start, rstream_stop,
};
#[cfg(not(test))]
use crate::event::rstream::{rstream_available, rstream_consume, rstream_fields};
use crate::event::stream::Loop;
#[cfg(not(test))]
use crate::event::time::uv_timer_t;
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::main_init::os_exit;
use crate::map_glyph_cache::NvimString;
#[cfg(not(test))]
use crate::msgpack_rpc::channel::rpc_send_event;
#[cfg(not(test))]
use crate::option_vars::*;
use crate::tui::termkey::driver_csi::{
    TERMKEY_MOUSE_DRAG, TERMKEY_MOUSE_PRESS, TERMKEY_MOUSE_RELEASE, TERMKEY_MOUSE_UNKNOWN,
    TermKeyCsiParam, TermKeyMouseEvent, termkey_interpret_csi, termkey_interpret_csi_param,
    termkey_interpret_modereport, termkey_interpret_mouse,
};
use crate::tui::termkey::driver_ti::{
    TERMKEY_EVENT_PRESS, TERMKEY_EVENT_REPEAT, TERMKEY_KEYMOD_ALT, TERMKEY_KEYMOD_CTRL,
    TERMKEY_KEYMOD_SHIFT, TERMKEY_RES_AGAIN, TERMKEY_RES_KEY, TERMKEY_SYM_SUSPEND,
    TERMKEY_TYPE_APC, TERMKEY_TYPE_DCS, TERMKEY_TYPE_FUNCTION, TERMKEY_TYPE_KEYSYM,
    TERMKEY_TYPE_MODEREPORT, TERMKEY_TYPE_MOUSE, TERMKEY_TYPE_OSC, TERMKEY_TYPE_UNICODE,
    TERMKEY_TYPE_UNKNOWN_CSI, TermKey, TermKeyKey, TermKeyResult, TermKeyTerminfoGetstrHook,
    TerminfoEntry,
};
use crate::tui::termkey::termkey::{
    TERMKEY_CANON_DELBS, TERMKEY_FLAG_KEEPC0, TERMKEY_FLAG_NOSTART, TERMKEY_FLAG_UTF8,
    TERMKEY_FORMAT_ALTISMETA, TERMKEY_FORMAT_WRAPBRACKET, TermKeyFormat, termkey_destroy,
    termkey_get_buffer_remaining, termkey_get_buffer_size, termkey_get_canonflags, termkey_getkey,
    termkey_getkey_force, termkey_hook_terminfo_getstr, termkey_interpret_string,
    termkey_new_abstract, termkey_push_bytes, termkey_set_buffer_size, termkey_set_canonflags,
    termkey_start, termkey_strfkey,
};
#[cfg(not(test))]
use crate::tui::tui::{
    tui_enable_extended_underline, tui_handle_term_mode, tui_query_bg_color, tui_set_size,
};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

pub use crate::option::OptInt;
pub use crate::tui::tui::TUIData;
pub use crate::tui::tui::TermMode;
pub use crate::tui::tui::TermModeState;
pub type TermInput = c_void;
pub type KeyEncoding = c_int;

#[cfg(not(test))]
const UV_TIMER: c_int = 13;
const STDIN_FILENO: c_int = 0;
const INPUT_BUFFER_SIZE: usize = 256;
const KEY_BUFFER_SIZE: usize = 0x1000;

const TERMKEY_FORMAT_VIM: TermKeyFormat = TERMKEY_FORMAT_ALTISMETA | TERMKEY_FORMAT_WRAPBRACKET;

pub(crate) const kKeyEncodingLegacy: KeyEncoding = 0;
pub(crate) const kKeyEncodingKitty: KeyEncoding = 1;
pub(crate) const kKeyEncodingXterm: KeyEncoding = 2;

const KEYMOD_SUPER: c_int = 1 << 3;
const KEYMOD_META: c_int = 1 << 5;
const KEYMOD_RECOGNIZED: c_int =
    TERMKEY_KEYMOD_SHIFT | TERMKEY_KEYMOD_ALT | TERMKEY_KEYMOD_CTRL | KEYMOD_SUPER | KEYMOD_META;

const START_PASTE: &[u8; 6] = b"\x1b[200~";
const END_PASTE: &[u8; 6] = b"\x1b[201~";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermInputCallbacks {
    pub primary_device_attr: Option<unsafe extern "C" fn(tui: *mut TUIData)>,
}

#[repr(C)]
pub(crate) struct TermInputPrefix {
    pub(crate) in_fd: c_int,
    pub(crate) paste: i8,
    pub(crate) ttimeout: bool,
    pub(crate) callbacks: TermInputCallbacks,
    pub(crate) key_encoding: KeyEncoding,
    pub(crate) ttimeoutlen: OptInt,
    pub(crate) tk: *mut TermKey,
    pub(crate) tk_ti_hook_fn: TermKeyTerminfoGetstrHook,
}

#[repr(C)]
pub(crate) struct TermInputSuffix {
    pub(crate) tui_data: *mut TUIData,
    pub(crate) key_buffer: [c_char; KEY_BUFFER_SIZE],
    pub(crate) key_buffer_len: usize,
}

static mut LAST_PRESSED_BUTTON: c_int = 0;

#[cfg(not(test))]
unsafe extern "C" {

    fn uv_timer_get_due_in(handle: *const uv_timer_t) -> u64;

}

#[inline]
pub(crate) unsafe fn input_prefix(input: *mut TermInput) -> *mut TermInputPrefix {
    input.cast::<TermInputPrefix>()
}

#[cfg(not(test))]
#[inline]
unsafe fn input_timer_handle(input: *mut TermInput) -> *mut uv_timer_t {
    unsafe {
        input
            .cast::<u8>()
            .add(size_of::<TermInputPrefix>())
            .cast::<uv_timer_t>()
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn input_bg_query_timer(input: *mut TermInput) -> *mut uv_timer_t {
    unsafe {
        #[cfg(not(test))]
        let timer_sz = uv_handle_size(UV_TIMER);
        #[cfg(test)]
        let timer_sz = 160;
        input
            .cast::<u8>()
            .add(size_of::<TermInputPrefix>() + timer_sz)
            .cast::<uv_timer_t>()
    }
}

#[inline]
pub(crate) unsafe fn input_loop_ptr(input: *mut TermInput) -> *mut *mut Loop {
    unsafe {
        #[cfg(not(test))]
        let timer_sz = uv_handle_size(UV_TIMER);
        #[cfg(test)]
        let timer_sz = 160;
        input
            .cast::<u8>()
            .add(size_of::<TermInputPrefix>() + 2 * timer_sz)
            .cast::<*mut Loop>()
    }
}

#[inline]
pub(crate) unsafe fn input_read_stream(input: *mut TermInput) -> *mut RStream {
    unsafe {
        #[cfg(not(test))]
        let timer_sz = uv_handle_size(UV_TIMER);
        #[cfg(test)]
        let timer_sz = 160;
        input
            .cast::<u8>()
            .add(size_of::<TermInputPrefix>() + 2 * timer_sz + size_of::<*mut Loop>())
            .cast::<RStream>()
    }
}

#[inline]
pub(crate) unsafe fn input_suffix(input: *mut TermInput) -> *mut TermInputSuffix {
    unsafe {
        #[cfg(not(test))]
        let timer_sz = uv_handle_size(UV_TIMER);
        #[cfg(test)]
        let timer_sz = 160;
        input
            .cast::<u8>()
            .add(
                size_of::<TermInputPrefix>()
                    + 2 * timer_sz
                    + size_of::<*mut Loop>()
                    + rstream_size(),
            )
            .cast::<TermInputSuffix>()
    }
}

#[inline]
const fn kitty_key_name(key: c_int) -> Option<&'static str> {
    match key {
        57344 => Some("Esc"),
        57345 => Some("CR"),
        57346 => Some("Tab"),
        57347 => Some("BS"),
        57348 => Some("Insert"),
        57349 => Some("Del"),
        57350 => Some("Left"),
        57351 => Some("Right"),
        57352 => Some("Up"),
        57353 => Some("Down"),
        57354 => Some("PageUp"),
        57355 => Some("PageDown"),
        57356 => Some("Home"),
        57357 => Some("End"),
        57364 => Some("F1"),
        57365 => Some("F2"),
        57366 => Some("F3"),
        57367 => Some("F4"),
        57368 => Some("F5"),
        57369 => Some("F6"),
        57370 => Some("F7"),
        57371 => Some("F8"),
        57372 => Some("F9"),
        57373 => Some("F10"),
        57374 => Some("F11"),
        57375 => Some("F12"),
        57376 => Some("F13"),
        57377 => Some("F14"),
        57378 => Some("F15"),
        57379 => Some("F16"),
        57380 => Some("F17"),
        57381 => Some("F18"),
        57382 => Some("F19"),
        57383 => Some("F20"),
        57384 => Some("F21"),
        57385 => Some("F22"),
        57386 => Some("F23"),
        57387 => Some("F24"),
        57388 => Some("F25"),
        57389 => Some("F26"),
        57390 => Some("F27"),
        57391 => Some("F28"),
        57392 => Some("F29"),
        57393 => Some("F30"),
        57394 => Some("F31"),
        57395 => Some("F32"),
        57396 => Some("F33"),
        57397 => Some("F34"),
        57398 => Some("F35"),
        57399 => Some("k0"),
        57400 => Some("k1"),
        57401 => Some("k2"),
        57402 => Some("k3"),
        57403 => Some("k4"),
        57404 => Some("k5"),
        57405 => Some("k6"),
        57406 => Some("k7"),
        57407 => Some("k8"),
        57408 => Some("k9"),
        57409 => Some("kPoint"),
        57410 => Some("kDivide"),
        57411 => Some("kMultiply"),
        57412 => Some("kMinus"),
        57413 => Some("kPlus"),
        57414 => Some("kEnter"),
        57415 => Some("kEqual"),
        57417 => Some("kLeft"),
        57418 => Some("kRight"),
        57419 => Some("kUp"),
        57420 => Some("kDown"),
        57421 => Some("kPageUp"),
        57422 => Some("kPageDown"),
        57423 => Some("kHome"),
        57424 => Some("kEnd"),
        57425 => Some("kInsert"),
        57426 => Some("kDel"),
        57427 => Some("kOrigin"),
        _ => None,
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tinput_init(
    input: *mut TermInput,
    loop_: *mut Loop,
    ti: *mut TerminfoEntry,
) {
    unsafe {
        let loop_ptr = input_loop_ptr(input);
        assert!((*loop_ptr).is_null());
        *loop_ptr = loop_;

        let p = input_prefix(input);
        (*p).paste = 0;
        (*p).in_fd = STDIN_FILENO;
        #[cfg(not(test))]
        {
            (*p).ttimeout = p_ttimeout != 0;
            (*p).ttimeoutlen = p_ttm;
        }

        rstream_init_fd(loop_, input_read_stream(input), (*p).in_fd);

        (*p).tk = termkey_new_abstract(
            ti,
            TERMKEY_FLAG_UTF8 | TERMKEY_FLAG_NOSTART | TERMKEY_FLAG_KEEPC0,
        );
        termkey_set_buffer_size((*p).tk, INPUT_BUFFER_SIZE);
        termkey_hook_terminfo_getstr((*p).tk, (*p).tk_ti_hook_fn, input.cast());
        termkey_start((*p).tk);

        let curflags = termkey_get_canonflags((*p).tk);
        termkey_set_canonflags((*p).tk, curflags | TERMKEY_CANON_DELBS);

        #[cfg(not(test))]
        {
            let th = input_timer_handle(input);
            uv_timer_init(loop_.cast(), th);
            *th.cast::<*mut c_void>() = input.cast();

            let bg_th = input_bg_query_timer(input);
            uv_timer_init(loop_.cast(), bg_th);
            *bg_th.cast::<*mut c_void>() = input.cast();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tinput_destroy(input: *mut TermInput) {
    unsafe {
        #[cfg(not(test))]
        {
            uv_close(input_timer_handle(input).cast(), None);
            uv_close(input_bg_query_timer(input).cast(), None);
        }
        rstream_may_close(input_read_stream(input));
        termkey_destroy((*input_prefix(input)).tk);
        *input_loop_ptr(input) = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tinput_start(input: *mut TermInput) {
    unsafe {
        rstream_start(input_read_stream(input), Some(tinput_read_cb), input.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tinput_stop(input: *mut TermInput) {
    unsafe {
        rstream_stop(input_read_stream(input));
        #[cfg(not(test))]
        {
            uv_timer_stop(input_timer_handle(input));
            uv_timer_stop(input_bg_query_timer(input));
        }
    }
}

#[cfg(not(test))]
unsafe extern "C" fn tinput_done_event(_argv: *mut *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        os_exit(1);
    }
}

/// Send all pending input in key buffer to Nvim server.
unsafe fn tinput_flush(input: *mut TermInput) {
    unsafe {
        let p = input_prefix(input);
        let s = input_suffix(input);
        let keys = NvimString {
            data: (*s).key_buffer.as_mut_ptr(),
            size: (*s).key_buffer_len,
        };
        if (*p).paste != 0 {
            let mut items = [
                string_obj(keys),
                boolean_obj(true),
                integer_obj(i64::from((*p).paste)),
            ];
            let args = Array {
                items: items.as_mut_ptr(),
                size: 3,
                capacity: 3,
            };
            #[cfg(not(test))]
            rpc_send_event(ui_client_channel_id, c"nvim_paste".as_ptr(), args);
            #[cfg(test)]
            let _ = args;
            if (*p).paste == 1 {
                (*p).paste = 2;
            }
        } else if (*s).key_buffer_len > 0 {
            let mut items = [string_obj(keys)];
            let args = Array {
                items: items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            #[cfg(not(test))]
            rpc_send_event(ui_client_channel_id, c"nvim_input".as_ptr(), args);
            #[cfg(test)]
            let _ = args;
        }
        (*s).key_buffer_len = 0;
    }
}

unsafe fn tinput_enqueue(input: *mut TermInput, buf: *const c_char, size: usize) {
    unsafe {
        let s = input_suffix(input);
        if (*s).key_buffer_len > KEY_BUFFER_SIZE.saturating_sub(size) {
            tinput_flush(input);
        }
        let to_copy = size.min(KEY_BUFFER_SIZE - (*s).key_buffer_len);
        ptr::copy_nonoverlapping(
            buf,
            (*s).key_buffer.as_mut_ptr().add((*s).key_buffer_len),
            to_copy,
        );
        (*s).key_buffer_len += to_copy;
    }
}

#[inline]
fn push_termkey_modifiers(modifiers: c_int, out: &mut String) {
    if (modifiers & TERMKEY_KEYMOD_SHIFT) != 0 {
        out.push_str("S-");
    }
    if (modifiers & TERMKEY_KEYMOD_ALT) != 0 {
        out.push_str("A-");
    }
    if (modifiers & TERMKEY_KEYMOD_CTRL) != 0 {
        out.push_str("C-");
    }
}

#[inline]
fn push_more_modifiers(modifiers: c_int, out: &mut String) {
    if (modifiers & KEYMOD_SUPER) != 0 {
        out.push_str("D-");
    }
    if (modifiers & KEYMOD_META) != 0 {
        out.push_str("T-");
    }
}

unsafe fn handle_kitty_key_protocol(input: *mut TermInput, key: *const TermKeyKey) {
    unsafe {
        if let Some(name) = kitty_key_name((*key).code.codepoint as c_int) {
            let mut buf = String::with_capacity(32);
            buf.push('<');
            push_termkey_modifiers((*key).modifiers, &mut buf);
            push_more_modifiers((*key).modifiers, &mut buf);
            buf.push_str(name);
            buf.push('>');
            tinput_enqueue(input, buf.as_ptr().cast::<c_char>(), buf.len());
        }
    }
}

unsafe fn forward_simple_utf8(input: *mut TermInput, key: *const TermKeyKey) {
    unsafe {
        let cp = (*key).code.codepoint as c_int;
        if (0xE000..=0xF8FF).contains(&cp) && kitty_key_name(cp).is_some() {
            handle_kitty_key_protocol(input, key);
            return;
        }

        let mut buf = [0u8; 64];
        let mut len: usize = 0;
        let mut ptr = (*key).utf8.as_ptr();
        while *ptr != 0 {
            if *ptr == b'<' as c_char {
                buf[len..len + 4].copy_from_slice(b"<lt>");
                len += 4;
            } else {
                buf[len] = *ptr as u8;
                len += 1;
            }
            assert!(len < buf.len());
            ptr = ptr.add(1);
        }

        tinput_enqueue(input, buf.as_ptr().cast::<c_char>(), len);
    }
}

unsafe fn forward_modified_utf8(input: *mut TermInput, key: *mut TermKeyKey) {
    unsafe {
        let p = input_prefix(input);
        let mut buf = [0 as c_char; 64];
        let mut len: usize;

        if (*key).r#type == TERMKEY_TYPE_KEYSYM && (*key).code.sym == TERMKEY_SYM_SUSPEND {
            let s = b"<C-Z>\0";
            ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), buf.as_mut_ptr(), s.len());
            len = 5;
        } else if (*key).r#type != TERMKEY_TYPE_UNICODE {
            len = termkey_strfkey(
                (*p).tk,
                buf.as_mut_ptr(),
                buf.len(),
                key,
                TERMKEY_FORMAT_VIM,
            );
        } else {
            assert!((*key).modifiers != 0);
            let cp = (*key).code.codepoint as c_int;
            if (0xE000..=0xF8FF).contains(&cp) && kitty_key_name(cp).is_some() {
                handle_kitty_key_protocol(input, key);
                return;
            }
            len = termkey_strfkey(
                (*p).tk,
                buf.as_mut_ptr(),
                buf.len(),
                key,
                TERMKEY_FORMAT_VIM,
            );
            if ((*key).modifiers & TERMKEY_KEYMOD_CTRL) != 0
                && ((*key).modifiers & TERMKEY_KEYMOD_SHIFT) == 0
                && (b'A' as c_int..=b'Z' as c_int).contains(&cp)
            {
                assert!(len + 2 < buf.len());
                ptr::copy(buf.as_ptr().add(1), buf.as_mut_ptr().add(3), len - 1);
                buf[1] = b'S' as c_char;
                buf[2] = b'-' as c_char;
                len += 2;
            }
        }

        let mut more_str = String::new();
        push_more_modifiers((*key).modifiers, &mut more_str);
        let more_len = more_str.len();
        if more_len > 0 {
            assert!(len + more_len < buf.len());
            ptr::copy(
                buf.as_ptr().add(1),
                buf.as_mut_ptr().add(1 + more_len),
                len - 1,
            );
            ptr::copy_nonoverlapping(
                more_str.as_ptr().cast::<c_char>(),
                buf.as_mut_ptr().add(1),
                more_len,
            );
            len += more_len;
        }

        assert!(len < buf.len());
        tinput_enqueue(input, buf.as_ptr(), len);
    }
}

unsafe fn forward_mouse_event(input: *mut TermInput, key: *const TermKeyKey) {
    unsafe {
        let p = input_prefix(input);
        let mut button: c_int = 0;
        let mut row: c_int = 0;
        let mut col: c_int = 0;
        let mut ev: TermKeyMouseEvent = TERMKEY_MOUSE_UNKNOWN;

        termkey_interpret_mouse(
            (*p).tk,
            key,
            &raw mut ev,
            &raw mut button,
            &raw mut row,
            &raw mut col,
        );

        if (ev == TERMKEY_MOUSE_RELEASE || ev == TERMKEY_MOUSE_DRAG) && button == 0 {
            button = LAST_PRESSED_BUTTON;
        }

        if (button == 0 && ev != TERMKEY_MOUSE_RELEASE)
            || (ev != TERMKEY_MOUSE_PRESS
                && ev != TERMKEY_MOUSE_DRAG
                && ev != TERMKEY_MOUSE_RELEASE)
        {
            return;
        }

        row -= 1;
        col -= 1;

        let mut buf = String::with_capacity(64);
        buf.push('<');
        push_termkey_modifiers((*key).modifiers, &mut buf);

        match button {
            1 => buf.push_str("Left"),
            2 => buf.push_str("Middle"),
            3 => buf.push_str("Right"),
            8 => buf.push_str("X1"),
            9 => buf.push_str("X2"),
            _ => {}
        }

        match ev {
            TERMKEY_MOUSE_PRESS => match button {
                4 => buf.push_str("ScrollWheelUp"),
                5 => buf.push_str("ScrollWheelDown"),
                6 => buf.push_str("ScrollWheelLeft"),
                7 => buf.push_str("ScrollWheelRight"),
                _ => {
                    buf.push_str("Mouse");
                    LAST_PRESSED_BUTTON = button;
                }
            },
            TERMKEY_MOUSE_DRAG => {
                buf.push_str("Drag");
            }
            TERMKEY_MOUSE_RELEASE => {
                buf.push_str(if button != 0 { "Release" } else { "MouseMove" });
                LAST_PRESSED_BUTTON = 0;
            }
            _ => std::process::abort(),
        }

        use std::fmt::Write as _;
        let _ = write!(buf, "><{col},{row}>");
        assert!(buf.len() < 64);
        tinput_enqueue(input, buf.as_ptr().cast::<c_char>(), buf.len());
    }
}

#[inline]
unsafe fn tk_getkey(tk: *mut TermKey, key: *mut TermKeyKey, force: bool) -> TermKeyResult {
    unsafe {
        if force {
            termkey_getkey_force(tk, key)
        } else {
            termkey_getkey(tk, key)
        }
    }
}

unsafe fn tk_getkeys(input: *mut TermInput, force: bool) {
    unsafe {
        let p = input_prefix(input);
        let mut key: TermKeyKey = std::mem::zeroed();
        let mut result: TermKeyResult;

        loop {
            result = tk_getkey((*p).tk, &raw mut key, force);
            if result != TERMKEY_RES_KEY {
                break;
            }

            if key.event != TERMKEY_EVENT_PRESS && key.event != TERMKEY_EVENT_REPEAT {
                continue;
            }

            if key.r#type == TERMKEY_TYPE_UNICODE && (key.modifiers & KEYMOD_RECOGNIZED) == 0 {
                forward_simple_utf8(input, &raw const key);
            } else if key.r#type == TERMKEY_TYPE_UNICODE
                || key.r#type == TERMKEY_TYPE_FUNCTION
                || key.r#type == TERMKEY_TYPE_KEYSYM
            {
                forward_modified_utf8(input, &raw mut key);
            } else if key.r#type == TERMKEY_TYPE_MOUSE {
                forward_mouse_event(input, &raw const key);
            } else if key.r#type == TERMKEY_TYPE_MODEREPORT {
                handle_modereport(input, &raw const key);
            } else if key.r#type == TERMKEY_TYPE_UNKNOWN_CSI {
                handle_unknown_csi(input, &raw const key);
            } else if key.r#type == TERMKEY_TYPE_OSC
                || key.r#type == TERMKEY_TYPE_DCS
                || key.r#type == TERMKEY_TYPE_APC
            {
                handle_term_response(input, &raw const key);
            }
        }

        if result != TERMKEY_RES_AGAIN {
            return;
        }

        if (*p).ttimeout && (*p).ttimeoutlen >= 0 {
            #[cfg(not(test))]
            {
                let th = input_timer_handle(input);
                uv_timer_stop(th);
                uv_timer_start(th, Some(tinput_timer_cb), (*p).ttimeoutlen as u64, 0);
            }
        } else {
            tk_getkeys(input, true);
        }
    }
}

#[cfg(not(test))]
unsafe extern "C" fn tinput_timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let input = *handle.cast::<*mut TermInput>();
        let rs = input_read_stream(input);
        let size = rstream_available(rs);
        if size != 0 {
            let r = rstream_fields(rs);
            let consumed = handle_raw_buffer(input, true, (*r).read_pos, size);
            rstream_consume(rs, consumed);
        }
        tk_getkeys(input, true);
        tinput_flush(input);
    }
}

#[cfg(not(test))]
unsafe extern "C" fn bg_query_timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let input = *handle.cast::<*mut TermInput>();
        #[cfg(not(test))]
        tui_query_bg_color((*input_suffix(input)).tui_data);
        #[cfg(test)]
        let _ = input;
    }
}

unsafe fn handle_focus_event(_input: *mut TermInput, ptr: *const c_char, size: usize) -> usize {
    unsafe {
        if size >= 3 {
            let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), 3);
            if bytes == b"\x1b[I" || bytes == b"\x1b[O" {
                let focus_gained = bytes[2] == b'I';
                let mut items = [boolean_obj(focus_gained)];
                let args = Array {
                    items: items.as_mut_ptr(),
                    size: 1,
                    capacity: 1,
                };
                #[cfg(not(test))]
                rpc_send_event(ui_client_channel_id, c"nvim_ui_set_focus".as_ptr(), args);
                #[cfg(test)]
                let _ = args;
                return 3;
            }
        }
        0
    }
}

unsafe fn handle_bracketed_paste(
    input: *mut TermInput,
    ptr: *const c_char,
    size: usize,
    incomplete: *mut bool,
) -> usize {
    unsafe {
        let p = input_prefix(input);
        let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), size);
        if size >= 6 && (&bytes[..6] == START_PASTE || &bytes[..6] == END_PASTE) {
            let enable = bytes[4] == b'0';
            if (*p).paste != 0 && enable {
                return 0;
            }

            if ((*p).paste != 0) == enable {
                return 6;
            }

            if enable {
                tinput_flush(input);
                (*p).paste = 1;
            } else if (*p).paste != 0 {
                (*p).paste = if (*p).paste == 2 { 3 } else { -1 };
                tinput_flush(input);
                (*p).paste = 0;
            }
            return 6;
        } else if size < 6 && (bytes == &START_PASTE[..size] || bytes == &END_PASTE[..size]) {
            *incomplete = true;
            return 0;
        }
        0
    }
}

unsafe fn send_termresponse(response: &[u8]) {
    let mut items = [
        string_obj(NvimString {
            data: c"termresponse".as_ptr().cast_mut(),
            size: 12,
        }),
        string_obj(NvimString {
            data: response.as_ptr().cast::<c_char>().cast_mut(),
            size: response.len(),
        }),
    ];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    #[cfg(not(test))]
    unsafe {
        rpc_send_event(ui_client_channel_id, c"nvim_ui_term_event".as_ptr(), args);
    }
    #[cfg(test)]
    let _ = args;
}

unsafe fn handle_term_response(input: *mut TermInput, key: *const TermKeyKey) {
    unsafe {
        let p = input_prefix(input);
        let mut str_ptr: *const c_char = ptr::null();
        if termkey_interpret_string((*p).tk, key, &raw mut str_ptr) == TERMKEY_RES_KEY {
            assert!(!str_ptr.is_null());
            let s_bytes = CStr::from_ptr(str_ptr).to_bytes();

            if (*key).r#type == TERMKEY_TYPE_DCS
                && (s_bytes.starts_with(b"1$r4:3m") || s_bytes.starts_with(b"1$r0;4:3m"))
            {
                #[cfg(not(test))]
                tui_enable_extended_underline((*input_suffix(input)).tui_data);
            }

            let prefix: &[u8] = match (*key).r#type {
                TERMKEY_TYPE_OSC => b"\x1b]",
                TERMKEY_TYPE_DCS => b"\x1bP",
                TERMKEY_TYPE_APC => b"\x1b_",
                _ => std::process::abort(),
            };

            let mut response = Vec::with_capacity(prefix.len() + s_bytes.len() + 1);
            response.extend_from_slice(prefix);
            response.extend_from_slice(s_bytes);
            response.push(0);

            send_termresponse(&response[..response.len() - 1]);
        }
    }
}

unsafe fn handle_primary_device_attr(
    input: *mut TermInput,
    params: *mut TermKeyCsiParam,
    nparams: usize,
) {
    unsafe {
        let p = input_prefix(input);
        if let Some(cb_save) = (*p).callbacks.primary_device_attr {
            (*p).callbacks.primary_device_attr = None;
            cb_save((*input_suffix(input)).tui_data);
        }

        if nparams == 0 {
            return;
        }

        let mut response = String::from("\x1b[?");
        for i in 0..nparams {
            let mut arg: c_int = 0;
            if termkey_interpret_csi_param(
                *params.add(i),
                &raw mut arg,
                ptr::null_mut(),
                ptr::null_mut(),
            ) != TERMKEY_RES_KEY
            {
                return;
            }
            use std::fmt::Write as _;
            let _ = write!(response, "{arg}");
            if i < nparams - 1 {
                response.push(';');
            }
        }
        response.push('c');
        let mut bytes = response.into_bytes();
        bytes.push(0);
        send_termresponse(&bytes[..bytes.len() - 1]);
    }
}

unsafe fn handle_modereport(input: *mut TermInput, key: *const TermKeyKey) {
    unsafe {
        let p = input_prefix(input);
        let mut initial: c_int = 0;
        let mut mode: c_int = 0;
        let mut value: c_int = 0;
        if termkey_interpret_modereport(
            (*p).tk,
            key,
            &raw mut initial,
            &raw mut mode,
            &raw mut value,
        ) == TERMKEY_RES_KEY
        {
            #[cfg(not(test))]
            tui_handle_term_mode((*input_suffix(input)).tui_data, mode, value);
        }
    }
}

unsafe fn handle_unknown_csi(input: *mut TermInput, key: *const TermKeyKey) {
    unsafe {
        let p = input_prefix(input);
        let mut params = [TermKeyCsiParam {
            param: ptr::null(),
            length: 0,
        }; 16];
        let mut nparams: usize = 16;
        let mut cmd: c_uint = 0;
        if termkey_interpret_csi(
            (*p).tk,
            key,
            params.as_mut_ptr(),
            &raw mut nparams,
            &raw mut cmd,
        ) != TERMKEY_RES_KEY
        {
            return;
        }

        let intermediate = ((cmd >> 16) & 0xff) as u8;
        let initial = ((cmd >> 8) & 0xff) as u8;
        let command = (cmd & 0xff) as u8;

        match command {
            b'q' => {
                if initial == b'>' && intermediate == b' ' {
                    let mut response = String::from("\x1b[>");
                    for (i, param) in params.iter().enumerate().take(nparams) {
                        let mut arg: c_int = 0;
                        if termkey_interpret_csi_param(
                            *param,
                            &raw mut arg,
                            ptr::null_mut(),
                            ptr::null_mut(),
                        ) != TERMKEY_RES_KEY
                        {
                            return;
                        }
                        use std::fmt::Write as _;
                        let _ = write!(response, "{arg}");
                        if i < nparams - 1 {
                            response.push(';');
                        }
                    }
                    response.push_str(" q");
                    let mut bytes = response.into_bytes();
                    bytes.push(0);
                    send_termresponse(&bytes[..bytes.len() - 1]);
                }
            }
            b'u' => {
                if initial == b'?' {
                    (*p).key_encoding = kKeyEncodingKitty;
                }
            }
            b'c' => {
                if initial == b'?' {
                    handle_primary_device_attr(input, params.as_mut_ptr(), nparams);
                }
            }
            b't' => {
                if nparams == 5 {
                    let mut args = [0 as c_int; 3];
                    for i in 0..args.len() {
                        if termkey_interpret_csi_param(
                            params[i],
                            &raw mut args[i],
                            ptr::null_mut(),
                            ptr::null_mut(),
                        ) != TERMKEY_RES_KEY
                        {
                            return;
                        }
                    }
                    if args[0] == 48 {
                        let height_chars = args[1];
                        let width_chars = args[2];
                        #[cfg(not(test))]
                        tui_set_size((*input_suffix(input)).tui_data, width_chars, height_chars);
                        #[cfg(test)]
                        let _ = (height_chars, width_chars);
                    }
                }
            }
            b'n' => {
                if nparams == 1 {
                    let mut arg: c_int = 0;
                    if termkey_interpret_csi_param(
                        params[0],
                        &raw mut arg,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    ) != TERMKEY_RES_KEY
                    {
                        return;
                    }
                    let mut bytes = format!("\x1b[{arg}n").into_bytes();
                    bytes.push(0);
                    send_termresponse(&bytes[..bytes.len() - 1]);
                } else if nparams == 2 {
                    let mut args = [0 as c_int; 2];
                    for i in 0..args.len() {
                        if termkey_interpret_csi_param(
                            params[i],
                            &raw mut args[i],
                            ptr::null_mut(),
                            ptr::null_mut(),
                        ) != TERMKEY_RES_KEY
                        {
                            return;
                        }
                    }
                    if args[0] == 997 {
                        #[cfg(not(test))]
                        {
                            let bg_th = input_bg_query_timer(input);
                            if uv_timer_get_due_in(bg_th) > 0 {
                                return;
                            }
                            uv_timer_start(bg_th, Some(bg_query_timer_cb), 100, 0);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

unsafe fn handle_raw_buffer(
    input: *mut TermInput,
    force: bool,
    data: *const c_char,
    mut size: usize,
) -> usize {
    unsafe {
        let p = input_prefix(input);
        let mut ptr = data;

        loop {
            if !force {
                let consumed = handle_focus_event(input, ptr, size);
                if consumed != 0 {
                    ptr = ptr.add(consumed);
                    size -= consumed;
                    if size == 0 {
                        break;
                    }
                    continue;
                }

                let mut incomplete = false;
                let consumed = handle_bracketed_paste(input, ptr, size, &raw mut incomplete);
                if incomplete {
                    assert_eq!(consumed, 0);
                    return (ptr as usize) - (data as usize);
                } else if consumed != 0 {
                    ptr = ptr.add(consumed);
                    size -= consumed;
                    if size == 0 {
                        break;
                    }
                    continue;
                }
            }

            let mut count: usize = 0;
            for i in 0..size {
                count = i + 1;
                if (*ptr.add(i) as u8) == 0x1b && count > 1 {
                    count -= 1;
                    break;
                }
            }

            if (*p).paste != 0 {
                tinput_enqueue(input, ptr, count);
                ptr = ptr.add(count);
                size -= count;
                if size == 0 {
                    break;
                }
                continue;
            }

            let to_use = count.min(size);
            let remaining = termkey_get_buffer_remaining((*p).tk);
            if to_use > remaining {
                let delta = to_use - remaining;
                let bufsize = termkey_get_buffer_size((*p).tk);
                if termkey_set_buffer_size((*p).tk, (bufsize + delta).max(bufsize * 2)) == 0 {
                    std::process::abort();
                }
            }

            let consumed = termkey_push_bytes((*p).tk, ptr, to_use);
            assert!(consumed <= to_use);
            ptr = ptr.add(consumed);
            size -= consumed;

            tk_getkeys(input, false);

            if size == 0 {
                break;
            }
        }

        let tk_size = termkey_get_buffer_size((*p).tk);
        let tk_remaining = termkey_get_buffer_remaining((*p).tk);
        let tk_count = tk_size - tk_remaining;
        if tk_count < INPUT_BUFFER_SIZE
            && tk_size > INPUT_BUFFER_SIZE
            && termkey_set_buffer_size((*p).tk, INPUT_BUFFER_SIZE) == 0
        {
            std::process::abort();
        }

        (ptr as usize) - (data as usize)
    }
}

unsafe extern "C" fn tinput_read_cb(
    _stream: *mut RStream,
    buf: *const c_char,
    count_: usize,
    data: *mut c_void,
    eof: bool,
) -> usize {
    unsafe {
        let input = data.cast::<TermInput>();
        let p = input_prefix(input);

        let consumed = handle_raw_buffer(input, false, buf, count_);
        tinput_flush(input);

        if eof {
            #[cfg(not(test))]
            loop_schedule_fast(
                (&raw mut main_loop).cast(),
                Event {
                    handler: Some(tinput_done_event),
                    argv: [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
                },
            );
            return consumed;
        }

        if consumed < count_ {
            let ms = if (*p).ttimeout {
                (*p).ttimeoutlen.max(0)
            } else {
                0
            };
            #[cfg(not(test))]
            {
                let th = input_timer_handle(input);
                uv_timer_stop(th);
                uv_timer_start(th, Some(tinput_timer_cb), u64::from(ms as u32), 0);
            }
            #[cfg(test)]
            let _ = ms;
        }

        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_key_name_and_modifier_helpers_work() {
        assert_eq!(kitty_key_name(57344), Some("Esc"));
        assert_eq!(kitty_key_name(57364), Some("F1"));
        assert_eq!(kitty_key_name(57398), Some("F35"));
        assert_eq!(kitty_key_name(57427), Some("kOrigin"));
        assert_eq!(kitty_key_name(57358), None);

        let mut mods = String::new();
        push_termkey_modifiers(TERMKEY_KEYMOD_SHIFT | TERMKEY_KEYMOD_CTRL, &mut mods);
        push_more_modifiers(KEYMOD_SUPER | KEYMOD_META, &mut mods);
        assert_eq!(mods, "S-C-D-T-");
    }
}
