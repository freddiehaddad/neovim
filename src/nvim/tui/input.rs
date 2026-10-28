//! Terminal input stream and key event handling (`src/nvim/tui/input.c`).

use crate::api::private::helpers::{boolean_obj, integer_obj, string_obj};
use crate::api::private::validate::{Array, NvimString};
use crate::event::r#loop::{
    Loop, loop_schedule_fast, uv_close, uv_timer_init, uv_timer_start, uv_timer_stop, uv_timer_t,
};
use crate::event::multiqueue::Event;
use crate::event::rstream::{
    RStream, rstream_available, rstream_consume, rstream_init_fd, rstream_may_close, rstream_start,
    rstream_stop,
};
use crate::event::stream::StreamCbData;
use crate::globals::*;
use crate::main_init::os_exit;
use crate::msgpack_rpc::channel::rpc_send_event;
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
    TERMKEY_TYPE_UNKNOWN_CSI, TermKey, TermKeyKey, TermKeyResult, TerminfoEntry,
};
use crate::tui::termkey::termkey::{
    TERMKEY_CANON_DELBS, TERMKEY_FLAG_KEEPC0, TERMKEY_FLAG_NOSTART, TERMKEY_FLAG_UTF8,
    TERMKEY_FORMAT_ALTISMETA, TERMKEY_FORMAT_WRAPBRACKET, TermKeyFormat,
    termkey_get_buffer_remaining, termkey_get_buffer_size, termkey_get_canonflags, termkey_getkey,
    termkey_getkey_force, termkey_interpret_string, termkey_new_abstract, termkey_push_bytes,
    termkey_set_buffer_size, termkey_set_canonflags, termkey_start, termkey_strfkey,
};
use crate::tui::tui::{
    TUIData, tui_enable_extended_underline, tui_handle_primary_device_attr, tui_handle_term_mode,
    tui_query_bg_color, tui_set_size,
};
use std::ffi::{c_char, c_int, c_uint};
use std::ptr;

use crate::channel::STDIN_FILENO;
use crate::option::OptInt;
pub type KeyEncoding = c_int;

const INPUT_BUFFER_SIZE: usize = 256;
const KEY_BUFFER_SIZE: usize = 0x1000;

const TERMKEY_FORMAT_VIM: TermKeyFormat = TERMKEY_FORMAT_ALTISMETA | TERMKEY_FORMAT_WRAPBRACKET;

pub(crate) const K_KEY_ENCODING_LEGACY: KeyEncoding = 0;
pub(crate) const K_KEY_ENCODING_KITTY: KeyEncoding = 1;
pub(crate) const K_KEY_ENCODING_XTERM: KeyEncoding = 2;

const KEYMOD_SUPER: c_int = 1 << 3;
const KEYMOD_META: c_int = 1 << 5;
const KEYMOD_RECOGNIZED: c_int =
    TERMKEY_KEYMOD_SHIFT | TERMKEY_KEYMOD_ALT | TERMKEY_KEYMOD_CTRL | KEYMOD_SUPER | KEYMOD_META;

const START_PASTE: &[u8; 6] = b"\x1b[200~";
const END_PASTE: &[u8; 6] = b"\x1b[201~";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PrimaryDeviceAttrAction {
    SetKeyEncoding,
    Stop,
    #[cfg(unix)]
    Suspend,
}

pub struct TermInput {
    pub in_fd: c_int,
    pub paste: i8,
    pub ttimeout: bool,
    pub primary_device_attr: Option<PrimaryDeviceAttrAction>,
    pub key_encoding: KeyEncoding,
    pub ttimeoutlen: OptInt,
    pub tk: Option<TermKey>,
    pub timer_handle: [u64; 32],
    pub bg_query_timer: [u64; 32],
    pub loop_: *mut Loop,
    pub read_stream: RStream,
    pub tui_data: *mut TUIData,
    pub key_buffer: [c_char; KEY_BUFFER_SIZE],
    pub key_buffer_len: usize,
}

impl TermInput {
    pub const fn new() -> Self {
        Self {
            in_fd: 0,
            paste: 0,
            ttimeout: false,
            primary_device_attr: None,
            key_encoding: K_KEY_ENCODING_LEGACY,
            ttimeoutlen: 0,
            tk: None,
            timer_handle: [0; 32],
            bg_query_timer: [0; 32],
            loop_: ptr::null_mut(),
            read_stream: RStream::new(),
            tui_data: ptr::null_mut(),
            key_buffer: [0; KEY_BUFFER_SIZE],
            key_buffer_len: 0,
        }
    }

    #[inline]
    fn tk(&self) -> &TermKey {
        self.tk.as_ref().unwrap()
    }

    #[inline]
    fn tk_mut(&mut self) -> &mut TermKey {
        self.tk.as_mut().unwrap()
    }
}

struct TuiInputModuleState {
    last_pressed_button: c_int,
}

impl TuiInputModuleState {
    const fn new() -> Self {
        Self {
            last_pressed_button: 0,
        }
    }
}

static TUI_INP_STATE: crate::globals::GlobalState<TuiInputModuleState> =
    crate::globals::GlobalState::new(TuiInputModuleState::new());

#[inline]
fn tui_inp_st() -> &'static mut TuiInputModuleState {
    unsafe { &mut *TUI_INP_STATE.as_ptr() }
}

use crate::event::r#loop::uv_timer_get_due_in;

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

pub fn tinput_init(input: &mut TermInput, loop_: *mut Loop, ti: &TerminfoEntry) {
    unsafe {
        assert!(input.loop_.is_null());
        input.loop_ = loop_;

        input.paste = 0;
        input.in_fd = STDIN_FILENO;
        {
            input.ttimeout = opt_vars().p_ttimeout != 0;
            input.ttimeoutlen = opt_vars().p_ttm;
        }

        rstream_init_fd(loop_, &mut input.read_stream, input.in_fd);

        input.tk = termkey_new_abstract(
            Some(ti),
            TERMKEY_FLAG_UTF8 | TERMKEY_FLAG_NOSTART | TERMKEY_FLAG_KEEPC0,
        );
        termkey_set_buffer_size(input.tk_mut(), INPUT_BUFFER_SIZE);
        let tk_ptr: *mut TermKey = input.tk_mut();
        termkey_start(&mut *tk_ptr, Some(ti), Some(&mut *input));

        let curflags = termkey_get_canonflags(input.tk_mut());
        termkey_set_canonflags(input.tk_mut(), curflags | TERMKEY_CANON_DELBS);

        {
            let th: *mut uv_timer_t = (&raw mut input.timer_handle).cast();
            uv_timer_init(loop_.cast(), th);
            *th.cast::<*mut TermInput>() = input;

            let bg_th: *mut uv_timer_t = (&raw mut input.bg_query_timer).cast();
            uv_timer_init(loop_.cast(), bg_th);
            *bg_th.cast::<*mut TermInput>() = input;
        }
    }
}

pub fn tinput_destroy(input: &mut TermInput) {
    unsafe {
        {
            uv_close((&raw mut input.timer_handle).cast(), None);
            uv_close((&raw mut input.bg_query_timer).cast(), None);
        }
        rstream_may_close(&mut input.read_stream);
        input.tk = None;
        input.loop_ = ptr::null_mut();
    }
}

pub fn tinput_start(input: &mut TermInput) {
    let input_ptr = &raw mut *input;
    rstream_start(&mut input.read_stream, StreamCbData::Input(input_ptr));
}

pub fn tinput_stop(input: &mut TermInput) {
    unsafe {
        rstream_stop(&mut input.read_stream);
        {
            uv_timer_stop((&raw mut input.timer_handle).cast());
            uv_timer_stop((&raw mut input.bg_query_timer).cast());
        }
    }
}

pub fn tinput_done_event() {
    os_exit(1);
}

/// Send all pending input in key buffer to Nvim server.
fn tinput_flush(input: &mut TermInput) {
    let keys = NvimString {
        data: input.key_buffer.as_mut_ptr(),
        size: input.key_buffer_len,
    };
    if input.paste != 0 {
        let mut items = [
            string_obj(keys),
            boolean_obj(true),
            integer_obj(i64::from(input.paste)),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        rpc_send_event(g_st().ui_client_channel_id, c"nvim_paste", args);
        if input.paste == 1 {
            input.paste = 2;
        }
    } else if input.key_buffer_len > 0 {
        let mut items = [string_obj(keys)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        rpc_send_event(g_st().ui_client_channel_id, c"nvim_input", args);
    }
    input.key_buffer_len = 0;
}

fn tinput_enqueue(input: &mut TermInput, buf: impl crate::strings::IntoCCharPtr, size: usize) {
    let buf = buf.into_c_char_ptr();
    unsafe {
        if input.key_buffer_len > KEY_BUFFER_SIZE.saturating_sub(size) {
            tinput_flush(input);
        }
        let to_copy = size.min(KEY_BUFFER_SIZE - input.key_buffer_len);
        ptr::copy_nonoverlapping(
            buf,
            input.key_buffer.as_mut_ptr().add(input.key_buffer_len),
            to_copy,
        );
        input.key_buffer_len += to_copy;
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

fn handle_kitty_key_protocol(input: &mut TermInput, key: &TermKeyKey) {
    if let Some(name) = kitty_key_name(key.code.codepoint as c_int) {
        let mut buf = String::with_capacity(32);
        buf.push('<');
        push_termkey_modifiers(key.modifiers, &mut buf);
        push_more_modifiers(key.modifiers, &mut buf);
        buf.push_str(name);
        buf.push('>');
        tinput_enqueue(input, buf.as_ptr().cast::<c_char>(), buf.len());
    }
}

fn forward_simple_utf8(input: &mut TermInput, key: &TermKeyKey) {
    unsafe {
        let cp = key.code.codepoint as c_int;
        if (0xE000..=0xF8FF).contains(&cp) && kitty_key_name(cp).is_some() {
            handle_kitty_key_protocol(input, key);
            return;
        }

        let mut buf = [0u8; 64];
        let mut len: usize = 0;
        let mut ptr = key.utf8.as_ptr();
        while *ptr != 0 {
            if crate::memory::cstr_first_byte(ptr) == b'<' {
                buf[len..len + 4].copy_from_slice(b"<lt>");
                len += 4;
            } else {
                buf[len] = crate::memory::cstr_first_byte(ptr);
                len += 1;
            }
            assert!(len < buf.len());
            ptr = ptr.add(1);
        }

        tinput_enqueue(input, buf.as_ptr().cast::<c_char>(), len);
    }
}

fn forward_modified_utf8(input: &mut TermInput, key: &mut TermKeyKey) {
    unsafe {
        let mut buf = [0 as c_char; 64];
        let mut len: usize;

        if key.r#type == TERMKEY_TYPE_KEYSYM && key.code.sym == TERMKEY_SYM_SUSPEND {
            let s = b"<C-Z>\0";
            ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), buf.as_mut_ptr(), s.len());
            len = 5;
        } else if key.r#type != TERMKEY_TYPE_UNICODE {
            len = termkey_strfkey(
                input.tk(),
                buf.as_mut_ptr(),
                buf.len(),
                key,
                TERMKEY_FORMAT_VIM,
            );
        } else {
            assert!(key.modifiers != 0);
            let cp = key.code.codepoint as c_int;
            if (0xE000..=0xF8FF).contains(&cp) && kitty_key_name(cp).is_some() {
                handle_kitty_key_protocol(input, key);
                return;
            }
            len = termkey_strfkey(
                input.tk(),
                buf.as_mut_ptr(),
                buf.len(),
                key,
                TERMKEY_FORMAT_VIM,
            );
            if (key.modifiers & TERMKEY_KEYMOD_CTRL) != 0
                && (key.modifiers & TERMKEY_KEYMOD_SHIFT) == 0
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
        push_more_modifiers(key.modifiers, &mut more_str);
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

fn forward_mouse_event(input: &mut TermInput, key: &TermKeyKey) {
    {
        let mut button: c_int = 0;
        let mut row: c_int = 0;
        let mut col: c_int = 0;
        let mut ev: TermKeyMouseEvent = TERMKEY_MOUSE_UNKNOWN;

        termkey_interpret_mouse(input.tk(), key, &mut ev, &mut button, &mut row, &mut col);

        if (ev == TERMKEY_MOUSE_RELEASE || ev == TERMKEY_MOUSE_DRAG) && button == 0 {
            button = tui_inp_st().last_pressed_button;
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
        push_termkey_modifiers(key.modifiers, &mut buf);

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
                    tui_inp_st().last_pressed_button = button;
                }
            },
            TERMKEY_MOUSE_DRAG => {
                buf.push_str("Drag");
            }
            TERMKEY_MOUSE_RELEASE => {
                buf.push_str(if button != 0 { "Release" } else { "MouseMove" });
                tui_inp_st().last_pressed_button = 0;
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
fn tk_getkey(tk: &mut TermKey, key: &mut TermKeyKey, force: bool) -> TermKeyResult {
    if force {
        termkey_getkey_force(tk, key)
    } else {
        termkey_getkey(tk, key)
    }
}

fn tk_getkeys(input: &mut TermInput, force: bool) {
    unsafe {
        let mut key = TermKeyKey::new();
        let mut result: TermKeyResult;

        loop {
            result = tk_getkey(input.tk_mut(), &mut key, force);
            if result != TERMKEY_RES_KEY {
                break;
            }

            if key.event != TERMKEY_EVENT_PRESS && key.event != TERMKEY_EVENT_REPEAT {
                continue;
            }

            if key.r#type == TERMKEY_TYPE_UNICODE && (key.modifiers & KEYMOD_RECOGNIZED) == 0 {
                forward_simple_utf8(input, &key);
            } else if key.r#type == TERMKEY_TYPE_UNICODE
                || key.r#type == TERMKEY_TYPE_FUNCTION
                || key.r#type == TERMKEY_TYPE_KEYSYM
            {
                forward_modified_utf8(input, &mut key);
            } else if key.r#type == TERMKEY_TYPE_MOUSE {
                forward_mouse_event(input, &key);
            } else if key.r#type == TERMKEY_TYPE_MODEREPORT {
                handle_modereport(input, &key);
            } else if key.r#type == TERMKEY_TYPE_UNKNOWN_CSI {
                handle_unknown_csi(input, &key);
            } else if key.r#type == TERMKEY_TYPE_OSC
                || key.r#type == TERMKEY_TYPE_DCS
                || key.r#type == TERMKEY_TYPE_APC
            {
                handle_term_response(input, &key);
            }
        }

        if result != TERMKEY_RES_AGAIN {
            return;
        }

        if input.ttimeout && input.ttimeoutlen >= 0 {
            {
                let th: *mut uv_timer_t = (&raw mut input.timer_handle).cast();
                uv_timer_stop(th);
                uv_timer_start(th, Some(tinput_timer_cb), input.ttimeoutlen as u64, 0);
            }
        } else {
            tk_getkeys(input, true);
        }
    }
}

unsafe extern "C" fn tinput_timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let input = &mut **handle.cast::<*mut TermInput>();
        let rs = &raw mut input.read_stream;
        let size = rstream_available(&mut *rs);
        if size != 0 {
            let consumed = handle_raw_buffer(input, true, (*rs).read_ptr(), size);
            rstream_consume(&mut *rs, consumed);
        }
        tk_getkeys(input, true);
        tinput_flush(input);
    }
}

unsafe extern "C" fn bg_query_timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let input = &mut **handle.cast::<*mut TermInput>();
        tui_query_bg_color(&mut *input.tui_data);
    }
}

fn handle_focus_event(
    _input: &mut TermInput,
    ptr: impl crate::strings::IntoCCharPtr,
    size: usize,
) -> usize {
    let ptr = ptr.into_c_char_ptr();
    {
        if size >= 3 {
            let bytes = crate::memory::cstr_slice(ptr, 3);
            if bytes == b"\x1b[I" || bytes == b"\x1b[O" {
                let focus_gained = bytes[2] == b'I';
                let mut items = [boolean_obj(focus_gained)];
                let args = Array {
                    items: items.as_mut_ptr(),
                    size: 1,
                    capacity: 1,
                };
                rpc_send_event(g_st().ui_client_channel_id, c"nvim_ui_set_focus", args);
                return 3;
            }
        }
        0
    }
}

fn handle_bracketed_paste(
    input: &mut TermInput,
    ptr: impl crate::strings::IntoCCharPtr,
    size: usize,
    incomplete: &mut bool,
) -> usize {
    let ptr = ptr.into_c_char_ptr();
    {
        let bytes = crate::memory::cstr_slice(ptr, size);
        if size >= 6 && (&bytes[..6] == START_PASTE || &bytes[..6] == END_PASTE) {
            let enable = bytes[4] == b'0';
            if input.paste != 0 && enable {
                return 0;
            }

            if (input.paste != 0) == enable {
                return 6;
            }

            if enable {
                tinput_flush(input);
                input.paste = 1;
            } else if input.paste != 0 {
                input.paste = if input.paste == 2 { 3 } else { -1 };
                tinput_flush(input);
                input.paste = 0;
            }
            return 6;
        } else if size < 6 && (bytes == &START_PASTE[..size] || bytes == &END_PASTE[..size]) {
            *incomplete = true;
            return 0;
        }
        0
    }
}

fn send_termresponse(response: &[u8]) {
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
    rpc_send_event(g_st().ui_client_channel_id, c"nvim_ui_term_event", args);
}

fn handle_term_response(input: &mut TermInput, key: &TermKeyKey) {
    unsafe {
        let mut str_ptr: *const c_char = ptr::null();
        if termkey_interpret_string(input.tk(), key, &raw mut str_ptr) == TERMKEY_RES_KEY {
            assert!(!str_ptr.is_null());
            let s_bytes = crate::memory::cstr_as_bytes(str_ptr);

            if key.r#type == TERMKEY_TYPE_DCS
                && (s_bytes.starts_with(b"1$r4:3m") || s_bytes.starts_with(b"1$r0;4:3m"))
            {
                tui_enable_extended_underline(&mut *input.tui_data);
            }

            let prefix: &[u8] = match key.r#type {
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

fn handle_primary_device_attr(input: &mut TermInput, params: &[TermKeyCsiParam], nparams: usize) {
    unsafe {
        if let Some(action) = input.primary_device_attr.take() {
            tui_handle_primary_device_attr(&mut *input.tui_data, action);
        }

        if nparams == 0 {
            return;
        }

        let mut response = String::from("\x1b[?");
        for (i, &param) in params.iter().enumerate().take(nparams) {
            let mut arg: c_int = 0;
            if termkey_interpret_csi_param(param, &mut arg, None, None) != TERMKEY_RES_KEY {
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

fn handle_modereport(input: &mut TermInput, key: &TermKeyKey) {
    unsafe {
        let mut initial: c_int = 0;
        let mut mode: c_int = 0;
        let mut value: c_int = 0;
        if termkey_interpret_modereport(input.tk(), key, &mut initial, &mut mode, &mut value)
            == TERMKEY_RES_KEY
        {
            tui_handle_term_mode(&mut *input.tui_data, mode, value);
        }
    }
}

fn handle_unknown_csi(input: &mut TermInput, key: &TermKeyKey) {
    unsafe {
        let mut params = [TermKeyCsiParam::new(); 16];
        let mut nparams: usize = 16;
        let mut cmd: c_uint = 0;
        if termkey_interpret_csi(input.tk(), key, &mut params, &mut nparams, &mut cmd)
            != TERMKEY_RES_KEY
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
                        if termkey_interpret_csi_param(*param, &mut arg, None, None)
                            != TERMKEY_RES_KEY
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
                    input.key_encoding = K_KEY_ENCODING_KITTY;
                }
            }
            b'c' => {
                if initial == b'?' {
                    handle_primary_device_attr(input, &params, nparams);
                }
            }
            b't' => {
                if nparams == 5 {
                    let mut args = [0 as c_int; 3];
                    for i in 0..args.len() {
                        if termkey_interpret_csi_param(params[i], &mut args[i], None, None)
                            != TERMKEY_RES_KEY
                        {
                            return;
                        }
                    }
                    if args[0] == 48 {
                        let height_chars = args[1];
                        let width_chars = args[2];
                        tui_set_size(&mut *input.tui_data, width_chars, height_chars);
                    }
                }
            }
            b'n' => {
                if nparams == 1 {
                    let mut arg: c_int = 0;
                    if termkey_interpret_csi_param(params[0], &mut arg, None, None)
                        != TERMKEY_RES_KEY
                    {
                        return;
                    }
                    let mut bytes = format!("\x1b[{arg}n").into_bytes();
                    bytes.push(0);
                    send_termresponse(&bytes[..bytes.len() - 1]);
                } else if nparams == 2 {
                    let mut args = [0 as c_int; 2];
                    for i in 0..args.len() {
                        if termkey_interpret_csi_param(params[i], &mut args[i], None, None)
                            != TERMKEY_RES_KEY
                        {
                            return;
                        }
                    }
                    if args[0] == 997 {
                        {
                            let bg_th: *mut uv_timer_t = (&raw mut input.bg_query_timer).cast();
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

fn handle_raw_buffer(
    input: &mut TermInput,
    force: bool,
    data: impl crate::strings::IntoCCharPtr,
    mut size: usize,
) -> usize {
    let data = data.into_c_char_ptr();
    unsafe {
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
                let consumed = handle_bracketed_paste(input, ptr, size, &mut incomplete);
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
                if (crate::memory::cstr_byte_at(ptr, i)) == 0x1b && count > 1 {
                    count -= 1;
                    break;
                }
            }

            if input.paste != 0 {
                tinput_enqueue(input, ptr, count);
                ptr = ptr.add(count);
                size -= count;
                if size == 0 {
                    break;
                }
                continue;
            }

            let to_use = count.min(size);
            let remaining = termkey_get_buffer_remaining(input.tk());
            if to_use > remaining {
                let delta = to_use - remaining;
                let bufsize = termkey_get_buffer_size(input.tk());
                if termkey_set_buffer_size(input.tk_mut(), (bufsize + delta).max(bufsize * 2)) == 0
                {
                    std::process::abort();
                }
            }

            let consumed = termkey_push_bytes(input.tk_mut(), ptr, to_use);
            assert!(consumed <= to_use);
            ptr = ptr.add(consumed);
            size -= consumed;

            tk_getkeys(input, false);

            if size == 0 {
                break;
            }
        }

        let tk_size = termkey_get_buffer_size(input.tk());
        let tk_remaining = termkey_get_buffer_remaining(input.tk());
        let tk_count = tk_size - tk_remaining;
        if tk_count < INPUT_BUFFER_SIZE
            && tk_size > INPUT_BUFFER_SIZE
            && termkey_set_buffer_size(input.tk_mut(), INPUT_BUFFER_SIZE) == 0
        {
            std::process::abort();
        }

        (ptr as usize) - (data as usize)
    }
}

pub(crate) fn tinput_read_cb(
    _stream: &mut RStream,
    buf: impl crate::strings::IntoCCharPtr,
    count_: usize,
    input: &mut TermInput,
    eof: bool,
) -> usize {
    let buf = buf.into_c_char_ptr();
    unsafe {
        let consumed = handle_raw_buffer(input, false, buf, count_);
        tinput_flush(input);

        if eof {
            loop_schedule_fast(&mut g_st().main_loop, Event::TInputDone);
            return consumed;
        }

        if consumed < count_ {
            let ms = if input.ttimeout {
                input.ttimeoutlen.max(0)
            } else {
                0
            };
            {
                let th: *mut uv_timer_t = (&raw mut input.timer_handle).cast();
                uv_timer_stop(th);
                uv_timer_start(th, Some(tinput_timer_cb), u64::from(ms as u32), 0);
            }
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
