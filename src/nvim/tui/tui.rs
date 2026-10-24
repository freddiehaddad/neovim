//! Terminal UI functions invoked by the UI client process (`src/nvim/tui/tui.c`).

use crate::api::private::helpers::{
    api_clear_error, api_free_object, arena_array, array_obj, boolean_obj, cstr_as_obj, dict_obj,
    static_cstr_as_obj, static_cstr_as_string, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType,
};
use crate::buffer_updates::ARENA_EMPTY;
use crate::charset::getdigits_int;
use crate::cursor_shape::{CursorShape, SHAPE_IDX_COUNT, SHAPE_IDX_N, cursorentry_T, shape_table};
use crate::event::r#loop::uv_sleep;
#[cfg(unix)]
use crate::event::r#loop::uv_tty_set_mode;
use crate::event::r#loop::{
    Loop, UV_RUN_DEFAULT, loop_poll_events, loop_purge, loop_queues, loop_size, uv_chdir, uv_close,
    uv_handle_t, uv_is_closing, uv_loop_close, uv_loop_init, uv_loop_t, uv_pipe_init, uv_pipe_open,
    uv_run, uv_strerror, uv_timer_init, uv_timer_start, uv_timer_t, uv_tty_init, uv_write,
};
#[cfg(windows)]
use crate::event::r#loop::{uv_handle_get_type, uv_handle_size};
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
use crate::event::signal::{
    SignalWatcher, signal_watcher_close, signal_watcher_init, signal_watcher_start,
    signal_watcher_stop,
};
use crate::event::stream::stream_set_blocking;
#[cfg(windows)]
use crate::event::stream::{ENABLE_VIRTUAL_TERMINAL_INPUT, UV_STREAM, UV_TTY};
use crate::event::wstream::uv_buf_t;
use crate::file_search::NULL_STRING;
use crate::fileio::{FILE, fclose, fopen, fwrite};
use crate::globals::{DFLT_COLS, DFLT_ROWS, *};
use crate::grid::{
    sattr_T, schar_T, schar_cache_clear_if_full, schar_from_ascii, schar_get, schar_get_ascii,
};
use crate::highlight::{HLATTRS_INIT, HlAttrs};
use crate::log::{LOGLVL_DBG, LOGLVL_ERR, LOGLVL_WRN, g_min_log_level, logmsg};
use crate::lua::executor::{kRetObject, nlua_exec};
use crate::mbyte::{utf_ambiguous_width, utf_char2cells, utf_ptr2char};
use crate::memory::{
    Arena, arena_finish, arena_mem_free, arena_memdupz, arena_strdup, strequal, xfree,
};
#[cfg(windows)]
use crate::memory::xstrdup;
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::option::OptInt;
#[cfg(windows)]
use crate::os::env::os_setenv;
use crate::os::env::{os_env_exists, os_getenv, os_getenv_buf};
#[cfg(not(windows))]
use crate::os::fs::UV_EINTR;
use crate::os::input::os_isatty;
use crate::os::time::os_hrtime;
use crate::tui::input::{
    TermInput, kKeyEncodingKitty, kKeyEncodingLegacy, kKeyEncodingXterm, tinput_destroy,
    tinput_init, tinput_start, tinput_stop,
};
use crate::tui::termkey::driver_ti::{TerminfoEntry, kTermCount, kTerminfoFuncKeyMax};
use crate::tui::ugrid::{
    UCell, UGrid, ugrid_clear, ugrid_clear_chunk, ugrid_free, ugrid_goto, ugrid_init, ugrid_resize,
    ugrid_scroll,
};
use crate::ui_client::ui_client_set_size;
#[cfg(unix)]
use crate::ui_client::{ui_client_attach, ui_client_detach};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::{GetConsoleMode, os_tty_guess_term};
use crate::tui::terminfo::*;

use crate::msgpack_rpc::packer::Integer;
pub type TermMode = c_int;
pub type TermModeState = c_int;
pub type ModeShape = c_int;
pub type LineFlags = c_int;
pub type TerminfoDef = usize;

pub const kTermModeLeftAndRightMargins: TermMode = 69;
pub const kTermModeMouseButtonEvent: TermMode = 1002;
pub const kTermModeMouseAnyEvent: TermMode = 1003;
pub const kTermModeMouseSGRExt: TermMode = 1006;
pub const kTermModeBracketedPaste: TermMode = 2004;
pub const kTermModeSynchronizedOutput: TermMode = 2026;
pub const kTermModeGraphemeClusters: TermMode = 2027;
pub const kTermModeThemeUpdates: TermMode = 2031;
pub const kTermModeResizeEvents: TermMode = 2048;

pub const kTermModeNotRecognized: TermModeState = 0;
pub const kTermModeSet: TermModeState = 1;
pub const kTermModeReset: TermModeState = 2;
pub const kTermModePermanentlySet: TermModeState = 3;
pub const kTermModePermanentlyReset: TermModeState = 4;

const EXIT_TIMEOUT_MS: i64 = 1000;
const OUTBUF_SIZE: usize = 0xffff;
const TOO_MANY_EVENTS: usize = 1_000_000;
const TERMINFO_SEQ_LIMIT: usize = 128;
const DEL: c_char = 0x7f;

#[cfg(unix)]
use crate::event::r#loop::UV_TTY_MODE_NORMAL;
#[cfg(unix)]
const UV_TTY_MODE_IO: c_int = 2;
#[cfg(target_os = "linux")]
const SIGSTOP: c_int = 19;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGSTOP: c_int = 17;

const LINUXSET0C: &[u8] = b"\x1b[?0c";
const LINUXSET1C: &[u8] = b"\x1b[?1c";

const XTERM_SETAF_256_COLON: *const c_char =
    c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38:5:%p1%d%;m".as_ptr();
const XTERM_SETAB_256_COLON: *const c_char =
    c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48:5:%p1%d%;m".as_ptr();
const XTERM_SETAF_256: *const c_char =
    c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m".as_ptr();
const XTERM_SETAB_256: *const c_char =
    c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m".as_ptr();
const XTERM_SETAF_16: *const c_char =
    c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e39%;m".as_ptr();
const XTERM_SETAB_16: *const c_char =
    c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e39%;m".as_ptr();

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub top: c_int,
    pub bot: c_int,
    pub left: c_int,
    pub right: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TerminfoExt {
    pub enable_focus_reporting: *const c_char,
    pub disable_focus_reporting: *const c_char,
    pub reset_scroll_region: *const c_char,
    pub enter_altfont_mode: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FlushBufFinish {
    kFlushBufPartial = 0,
    kFlushBufFinal = 1,
}

#[repr(C)]
pub struct TUIData {
    pub loop_: *mut Loop,
    pub buf: [c_char; OUTBUF_SIZE],
    pub buf_to_flush: *mut c_char,
    pub bufpos: usize,
    pub input: TermInput,
    pub write_loop: [u64; 256],
    pub ti: TerminfoEntry,
    pub term: *mut c_char,
    pub output_handle: [u64; 80],
    pub out_isatty: bool,
    pub winch_handle: SignalWatcher,
    pub startup_delay_timer: [u64; 32],
    pub grid: UGrid,
    pub invalid_regions: Vec<Rect>,
    pub row: c_int,
    pub col: c_int,
    pub out_fd: c_int,
    pub pending_resize_events: c_int,
    pub terminfo_found_in_db: bool,
    pub can_change_scroll_region: bool,
    pub has_left_and_right_margin_mode: bool,
    pub has_sync_mode: bool,
    pub can_set_lr_margin: bool,
    pub can_scroll: bool,
    pub can_erase_chars: bool,
    pub immediate_wrap_after_last_column: bool,
    pub bce: bool,
    pub mouse_enabled: bool,
    pub mouse_move_enabled: bool,
    pub mouse_enabled_save: bool,
    pub title_enabled: bool,
    pub sync_output: bool,
    pub sync_output_active: bool,
    pub busy: bool,
    pub is_invisible: bool,
    pub want_invisible: bool,
    pub set_cursor_color_as_str: bool,
    pub cursor_has_color: bool,
    pub is_starting: bool,
    pub resize_events_enabled: bool,
    pub modes_grapheme_clusters: bool,
    pub modes_theme_updates: bool,
    pub modes_resize_events: bool,
    pub screenshot: *mut FILE,
    pub cursor_shapes: [cursorentry_T; SHAPE_IDX_COUNT],
    pub clear_attrs: HlAttrs,
    pub attrs: Vec<HlAttrs>,
    pub print_attr_id: c_int,
    pub default_attr: bool,
    pub set_default_colors: bool,
    pub can_clear_attr: bool,
    pub showing_mode: ModeShape,
    pub verbose: Integer,
    pub terminfo_ext: TerminfoExt,
    pub can_set_title: bool,
    pub can_set_underline_color: bool,
    pub can_resize_screen: bool,
    pub stopped: bool,
    pub width: c_int,
    pub height: c_int,
    pub rgb: bool,
    pub screen_or_tmux: bool,
    pub url: c_int,
    pub ti_arena: Arena,
}

static mut cursor_style_enabled: bool = false;
static mut stty_erase_buf: [c_char; 2] = [0, 0];
static mut stty_erase_ptr: *const c_char = ptr::null();

static mut urls: Vec<std::ffi::CString> = Vec::new();

#[cfg(unix)]
use crate::fileio::{getpgid, getpgrp, getppid, kill};

use crate::event::r#loop::{uv_tty_get_winsize, uv_tty_reset_mode};

use crate::api::ui::{MAX_SCHAR_SIZE, kLineFlagWrap};
use crate::channel::STDOUT_FILENO;
use crate::charset::NUL;
use crate::drawline::{HL_OVERLINE, HL_STRIKETHROUGH, HL_UNDERLINE_MASK};
use crate::eval::funcs::{
    HL_BLINK, HL_BOLD, HL_CONCEALED, HL_DIM, HL_INVERSE, HL_ITALIC, HL_STANDOUT, HL_UNDERCURL,
    HL_UNDERDASHED, HL_UNDERDOTTED, HL_UNDERDOUBLE, HL_UNDERLINE,
};
use crate::highlight::{HL_ALTFONT, HL_BG_INDEXED, HL_FG_INDEXED};
#[cfg(unix)]
use crate::os::pty_proc_unix as unix_termios;
use crate::os::signal::SIGWINCH;

#[inline]
unsafe fn tui_input(tui: *mut TUIData) -> *mut TermInput {
    unsafe { &raw mut (*tui).input }
}

#[inline]
unsafe fn tui_write_loop(tui: *mut TUIData) -> *mut uv_loop_t {
    unsafe { (*tui).write_loop.as_mut_ptr().cast::<uv_loop_t>() }
}

#[inline]
unsafe fn tui_output_handle(tui: *mut TUIData) -> *mut uv_handle_t {
    unsafe { (*tui).output_handle.as_mut_ptr().cast::<uv_handle_t>() }
}

#[inline]
unsafe fn tui_winch_handle(tui: *mut TUIData) -> *mut SignalWatcher {
    unsafe { &raw mut (*tui).winch_handle }
}

#[inline]
unsafe fn tui_startup_delay_timer(tui: *mut TUIData) -> *mut uv_timer_t {
    unsafe { (*tui).startup_delay_timer.as_mut_ptr().cast::<uv_timer_t>() }
}

#[inline]
unsafe fn cstr_contains(haystack: *const c_char, needle: &[u8]) -> bool {
    if haystack.is_null() {
        return false;
    }
    let bytes = unsafe { CStr::from_ptr(haystack).to_bytes() };
    bytes.windows(needle.len()).any(|w| w == needle)
}

pub unsafe fn tui_start(
    tui_p: *mut *mut TUIData,
    width: *mut c_int,
    height: *mut c_int,
    term: *mut *mut c_char,
    rgb: *mut bool,
) {
    unsafe {
        let mut tui_uninit = Box::new(std::mem::MaybeUninit::<TUIData>::zeroed());
        let tui = tui_uninit.as_mut_ptr();
        ptr::write(&raw mut (*tui).invalid_regions, Vec::new());
        ptr::write(&raw mut (*tui).attrs, vec![HLATTRS_INIT]);
        ptr::write(&raw mut (*tui).grid.cells, Vec::new());
        let _ = Box::into_raw(tui_uninit);
        (*tui).is_starting = true;
        (*tui).screenshot = ptr::null_mut();
        (*tui).stopped = false;
        (*tui).loop_ = (&raw mut main_loop).cast::<Loop>();
        (*tui).url = -1;

        signal_watcher_init((*tui).loop_, tui_winch_handle(tui), tui.cast());
        signal_watcher_start(tui_winch_handle(tui), Some(sigwinch_cb), SIGWINCH);

        (*tui).input.tk_ti_hook_fn = Some(tui_tk_ti_getstr);
        ugrid_init(&mut (*tui).grid);
        tui_terminal_start(tui);

        let timer = tui_startup_delay_timer(tui);
        uv_timer_init((*tui).loop_.cast(), timer);
        *timer.cast::<*mut TUIData>() = tui;
        uv_timer_start(timer, Some(after_startup_cb), 100, 0);

        *tui_p = tui;
        loop_poll_events((&raw mut main_loop).cast::<Loop>(), 1);
        *width = (*tui).width;
        *height = (*tui).height;
        *term = (*tui).term;
        *rgb = (*tui).rgb;
    }
}

unsafe fn tui_request_term_mode(tui: *mut TUIData, mode: TermMode) {
    unsafe {
        let s = format!("\x1b[?{mode}$p");
        assert!(!s.is_empty() && s.len() < 12);
        out(tui, s.as_ptr().cast::<c_char>(), s.len());
    }
}

unsafe fn tui_set_term_mode(tui: *mut TUIData, mode: TermMode, set: bool) {
    unsafe {
        let ch = if set { 'h' } else { 'l' };
        let s = format!("\x1b[?{mode}{ch}");
        assert!(!s.is_empty() && s.len() < 12);
        out(tui, s.as_ptr().cast::<c_char>(), s.len());
    }
}

pub unsafe fn tui_handle_term_mode(tui: *mut TUIData, mode: TermMode, state: TermModeState) {
    unsafe {
        let mut is_set = false;
        match state {
            kTermModeNotRecognized | kTermModePermanentlyReset => {
                if !nvim_testing && g_min_log_level <= LOGLVL_WRN {
                    logmsg!(
                        LOGLVL_WRN,
                        ptr::null(),
                        c"tui_handle_term_mode".as_ptr(),
                        236,
                        true,
                        c"TUI: terminal mode %d unavailable, state %d".as_ptr(),
                        mode,
                        state,
                    );
                }
            }
            kTermModePermanentlySet | kTermModeSet | kTermModeReset => {
                if state == kTermModePermanentlySet || state == kTermModeSet {
                    is_set = true;
                }
                if !nvim_testing && g_min_log_level <= LOGLVL_WRN {
                    logmsg!(
                        LOGLVL_WRN,
                        ptr::null(),
                        c"tui_handle_term_mode".as_ptr(),
                        249,
                        true,
                        c"TUI: terminal mode %d detected, state %d".as_ptr(),
                        mode,
                        state,
                    );
                }

                match mode {
                    kTermModeSynchronizedOutput => {
                        (*tui).has_sync_mode = true;
                    }
                    kTermModeGraphemeClusters => {
                        if !is_set {
                            tui_set_term_mode(tui, mode, true);
                            (*tui).modes_grapheme_clusters = true;
                        }
                    }
                    kTermModeThemeUpdates => {
                        if !is_set {
                            tui_set_term_mode(tui, mode, true);
                            (*tui).modes_theme_updates = true;
                        }
                    }
                    kTermModeResizeEvents => {
                        if !is_set {
                            tui_set_term_mode(tui, mode, true);
                            (*tui).modes_resize_events = true;
                        }
                        (*tui).resize_events_enabled = true;
                    }
                    kTermModeLeftAndRightMargins => {
                        (*tui).has_left_and_right_margin_mode = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

unsafe fn tui_query_extended_underline(tui: *mut TUIData) {
    unsafe {
        let seq = b"\x1b[0m\x1b[4:3m\x1bP$qm\x1b\\";
        out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
        (*tui).print_attr_id = -1;
    }
}

pub unsafe fn tui_enable_extended_underline(tui: *mut TUIData) {
    unsafe {
        terminfo_set_if_empty(tui, kTerm_set_underline_style, c"\x1b[4:%p1%dm".as_ptr());
        (*tui).can_set_underline_color = true;
    }
}

unsafe fn tui_set_key_encoding_cb(tui: *mut TUIData) {
    unsafe {
        tui_set_key_encoding(tui);
    }
}

unsafe fn tui_query_kitty_keyboard(tui: *mut TUIData) {
    unsafe {
        (*tui).input.callbacks.primary_device_attr = Some(tui_set_key_encoding_cb);
        let seq = b"\x1b[?u\x1b[c";
        out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
    }
}

pub unsafe fn tui_set_key_encoding(tui: *mut TUIData) {
    unsafe {
        match (*tui).input.key_encoding {
            kKeyEncodingKitty => {
                let seq = b"\x1b[>3u";
                out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            }
            kKeyEncodingXterm => {
                let seq = b"\x1b[>4;2m";
                out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            }
            kKeyEncodingLegacy => {}
            _ => {}
        }
    }
}

unsafe fn tui_reset_key_encoding(tui: *mut TUIData) {
    unsafe {
        match (*tui).input.key_encoding {
            kKeyEncodingKitty => {
                let seq = b"\x1b[<u";
                out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            }
            kKeyEncodingXterm => {
                let seq = b"\x1b[>4;0m";
                out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            }
            kKeyEncodingLegacy => {}
            _ => {}
        }
    }
}

unsafe fn tui_query_bg_color_noflush(tui: *mut TUIData) {
    unsafe {
        let seq = b"\x1b]11;?\x07\x1b[5n";
        out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
    }
}

pub unsafe fn tui_query_bg_color(tui: *mut TUIData) {
    unsafe {
        tui_query_bg_color_noflush(tui);
        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
    }
}

#[derive(Clone, Copy)]
enum TermdefFieldTarget {
    BoolBackColorErase,
    BoolTc,
    BoolRGB,
    BoolSu,
    IntMaxColors,
    IntLines,
    IntColumns,
    Def(usize),
    KeyStr(usize),
    KeyArr(usize),
    FKey(usize),
}

impl TermdefFieldTarget {
    const fn expected_type(self) -> ObjectType {
        match self {
            Self::BoolBackColorErase | Self::BoolTc | Self::BoolRGB | Self::BoolSu => {
                ObjectType::kObjectTypeBoolean
            }
            Self::IntMaxColors | Self::IntLines | Self::IntColumns => {
                ObjectType::kObjectTypeInteger
            }
            Self::Def(_) | Self::KeyStr(_) | Self::FKey(_) => ObjectType::kObjectTypeString,
            Self::KeyArr(_) => ObjectType::kObjectTypeArray,
        }
    }
}

fn lookup_termdef_field(name: &[u8]) -> Option<TermdefFieldTarget> {
    use TermdefFieldTarget::*;
    match name {
        b"back_color_erase" => Some(BoolBackColorErase),
        b"Tc" => Some(BoolTc),
        b"RGB" => Some(BoolRGB),
        b"Su" => Some(BoolSu),
        b"max_colors" => Some(IntMaxColors),
        b"lines" => Some(IntLines),
        b"columns" => Some(IntColumns),
        b"carriage_return" => Some(Def(kTerm_carriage_return)),
        b"change_scroll_region" => Some(Def(kTerm_change_scroll_region)),
        b"clear_screen" => Some(Def(kTerm_clear_screen)),
        b"clr_eol" => Some(Def(kTerm_clr_eol)),
        b"clr_eos" => Some(Def(kTerm_clr_eos)),
        b"cursor_address" => Some(Def(kTerm_cursor_address)),
        b"cursor_down" => Some(Def(kTerm_cursor_down)),
        b"cursor_invisible" => Some(Def(kTerm_cursor_invisible)),
        b"cursor_left" => Some(Def(kTerm_cursor_left)),
        b"cursor_home" => Some(Def(kTerm_cursor_home)),
        b"cursor_normal" => Some(Def(kTerm_cursor_normal)),
        b"cursor_up" => Some(Def(kTerm_cursor_up)),
        b"cursor_right" => Some(Def(kTerm_cursor_right)),
        b"delete_line" => Some(Def(kTerm_delete_line)),
        b"enter_blink_mode" => Some(Def(kTerm_enter_blink_mode)),
        b"enter_bold_mode" => Some(Def(kTerm_enter_bold_mode)),
        b"enter_ca_mode" => Some(Def(kTerm_enter_ca_mode)),
        b"enter_dim_mode" => Some(Def(kTerm_enter_dim_mode)),
        b"enter_italics_mode" => Some(Def(kTerm_enter_italics_mode)),
        b"enter_reverse_mode" => Some(Def(kTerm_enter_reverse_mode)),
        b"enter_secure_mode" => Some(Def(kTerm_enter_secure_mode)),
        b"enter_standout_mode" => Some(Def(kTerm_enter_standout_mode)),
        b"enter_underline_mode" => Some(Def(kTerm_enter_underline_mode)),
        b"erase_chars" => Some(Def(kTerm_erase_chars)),
        b"exit_attribute_mode" => Some(Def(kTerm_exit_attribute_mode)),
        b"exit_ca_mode" => Some(Def(kTerm_exit_ca_mode)),
        b"from_status_line" => Some(Def(kTerm_from_status_line)),
        b"insert_line" => Some(Def(kTerm_insert_line)),
        b"keypad_local" => Some(Def(kTerm_keypad_local)),
        b"keypad_xmit" => Some(Def(kTerm_keypad_xmit)),
        b"parm_delete_line" => Some(Def(kTerm_parm_delete_line)),
        b"parm_down_cursor" => Some(Def(kTerm_parm_down_cursor)),
        b"parm_insert_line" => Some(Def(kTerm_parm_insert_line)),
        b"parm_left_cursor" => Some(Def(kTerm_parm_left_cursor)),
        b"parm_right_cursor" => Some(Def(kTerm_parm_right_cursor)),
        b"parm_up_cursor" => Some(Def(kTerm_parm_up_cursor)),
        b"set_a_background" => Some(Def(kTerm_set_a_background)),
        b"set_a_foreground" => Some(Def(kTerm_set_a_foreground)),
        b"set_attributes" => Some(Def(kTerm_set_attributes)),
        b"set_lr_margin" => Some(Def(kTerm_set_lr_margin)),
        b"to_status_line" => Some(Def(kTerm_to_status_line)),
        b"reset_cursor_style" => Some(Def(kTerm_reset_cursor_style)),
        b"set_cursor_style" => Some(Def(kTerm_set_cursor_style)),
        b"enter_strikethrough_mode" => Some(Def(kTerm_enter_strikethrough_mode)),
        b"set_rgb_foreground" => Some(Def(kTerm_set_rgb_foreground)),
        b"set_rgb_background" => Some(Def(kTerm_set_rgb_background)),
        b"set_cursor_color" => Some(Def(kTerm_set_cursor_color)),
        b"reset_cursor_color" => Some(Def(kTerm_reset_cursor_color)),
        b"set_underline_style" => Some(Def(kTerm_set_underline_style)),
        b"key_backspace" => Some(KeyStr(kTermKey_backspace)),
        b"key_beg" => Some(KeyArr(kTermKey_beg)),
        b"key_btab" => Some(KeyStr(kTermKey_btab)),
        b"key_clear" => Some(KeyStr(kTermKey_clear)),
        b"key_dc" => Some(KeyArr(kTermKey_dc)),
        b"key_end" => Some(KeyArr(kTermKey_end)),
        b"key_find" => Some(KeyArr(kTermKey_find)),
        b"key_home" => Some(KeyArr(kTermKey_home)),
        b"key_ic" => Some(KeyArr(kTermKey_ic)),
        b"key_npage" => Some(KeyStr(kTermKey_npage)),
        b"key_ppage" => Some(KeyStr(kTermKey_ppage)),
        b"key_select" => Some(KeyStr(kTermKey_select)),
        b"key_suspend" => Some(KeyArr(kTermKey_suspend)),
        b"key_undo" => Some(KeyArr(kTermKey_undo)),
        b"key_left" => Some(KeyArr(kTermKey_left)),
        b"key_right" => Some(KeyArr(kTermKey_right)),
        _ => {
            if let Some(rest) = name.strip_prefix(b"key_f")
                && let Ok(s) = std::str::from_utf8(rest)
                && let Ok(n) = s.parse::<usize>()
                && (1..=kTerminfoFuncKeyMax).contains(&n)
            {
                return Some(FKey(n - 1));
            }
            None
        }
    }
}

unsafe fn push_termdef_err(err_arena: &mut Arena, chunks: &mut Array, msg: &str) {
    unsafe {
        let dup = arena_memdupz(err_arena, msg.as_ptr().cast::<c_char>(), msg.len());
        let str_val = NvimString {
            data: dup,
            size: msg.len(),
        };
        let mut err = arena_array(err_arena, 2);
        *err.items.add(err.size) = string_obj(str_val);
        err.size += 1;
        *err.items.add(err.size) = static_cstr_as_obj(c"ErrorMsg");
        err.size += 1;
        chunks.push(array_obj(err));
    }
}

unsafe fn apply_termdefs(tui: *mut TUIData) {
    unsafe {
        if !os_env_exists(c"NVIM_TERMDEFS".as_ptr(), false) {
            return;
        }

        let mut lua_err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let empty_args = Array {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let rv = nlua_exec(
            static_cstr_as_string(c"return require('vim.tty')._get_termdefs()"),
            ptr::null(),
            empty_args,
            kRetObject,
            ptr::null_mut(),
            &raw mut lua_err,
        );
        if rv.obj_type != ObjectType::kObjectTypeDict {
            api_free_object(rv);
            api_clear_error(&raw mut lua_err);
            return;
        }

        let mut err_arena = ARENA_EMPTY;
        let mut chunks = Array::new();
        let mut err_count = 0usize;

        let dict = rv.data.dict;
        for i in 0..dict.size {
            let kv = *dict.items.add(i);
            let key_bytes = if kv.key.data.is_null() {
                &[][..]
            } else {
                CStr::from_ptr(kv.key.data).to_bytes()
            };
            let key_str = String::from_utf8_lossy(key_bytes);

            let Some(target) = lookup_termdef_field(key_bytes) else {
                let msg = format!("skipping invalid field in $NVIM_TERMDEFS: '{key_str}'\n");
                push_termdef_err(&mut err_arena, &mut chunks, &msg);
                err_count += 1;
                continue;
            };
            if kv.value.obj_type != target.expected_type() {
                let msg = format!("skipping invalid field in $NVIM_TERMDEFS: '{key_str}'\n");
                push_termdef_err(&mut err_arena, &mut chunks, &msg);
                err_count += 1;
                continue;
            }

            let val = kv.value;
            match target {
                TermdefFieldTarget::BoolBackColorErase => {
                    (*tui).ti.back_color_erase = val.data.boolean;
                }
                TermdefFieldTarget::BoolTc => {
                    (*tui).ti.Tc = val.data.boolean;
                }
                TermdefFieldTarget::BoolRGB => {
                    (*tui).ti.RGB = val.data.boolean;
                }
                TermdefFieldTarget::BoolSu => {
                    (*tui).ti.Su = val.data.boolean;
                }
                TermdefFieldTarget::IntMaxColors => {
                    (*tui).ti.max_colors = val.data.integer as c_int;
                }
                TermdefFieldTarget::IntLines => {
                    (*tui).ti.lines = val.data.integer as c_int;
                }
                TermdefFieldTarget::IntColumns => {
                    (*tui).ti.columns = val.data.integer as c_int;
                }
                TermdefFieldTarget::Def(idx) => {
                    (*tui).ti.defs[idx] =
                        arena_strdup(&raw mut (*tui).ti_arena, val.data.string.data);
                }
                TermdefFieldTarget::KeyStr(idx) => {
                    (*tui).ti.keys[idx][0] =
                        arena_strdup(&raw mut (*tui).ti_arena, val.data.string.data);
                }
                TermdefFieldTarget::KeyArr(idx) => {
                    let arr = val.data.array;
                    if arr.size > 2 {
                        let msg = format!(
                            "skipping invalid field definition in $NVIM_TERMDEFS for {key_str}: array should have at most two entries for unshifted and shifted key variants.\n"
                        );
                        push_termdef_err(&mut err_arena, &mut chunks, &msg);
                        err_count += 1;
                        continue;
                    }
                    (*tui).ti.keys[idx][0] = ptr::null();
                    (*tui).ti.keys[idx][1] = ptr::null();
                    for key_map_idx in 0..arr.size {
                        let key_mapping = *arr.items.add(key_map_idx);
                        if key_mapping.obj_type != ObjectType::kObjectTypeString {
                            let msg = format!(
                                "skipping invalid field definition in $NVIM_TERMDEFS for {key_str}[{key_map_idx}]: expected type 'string'\n"
                            );
                            push_termdef_err(&mut err_arena, &mut chunks, &msg);
                            err_count += 1;
                            continue;
                        }
                        (*tui).ti.keys[idx][key_map_idx] =
                            arena_strdup(&raw mut (*tui).ti_arena, key_mapping.data.string.data);
                    }
                }
                TermdefFieldTarget::FKey(idx) => {
                    (*tui).ti.f_keys[idx] =
                        arena_strdup(&raw mut (*tui).ti_arena, val.data.string.data);
                }
            }
        }

        if err_count > 0 {
            let mut opts_items = [KeyValuePair {
                key: static_cstr_as_string(c"err"),
                value: boolean_obj(true),
            }];
            let opts = Dict {
                items: opts_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            let mut args_items = [array_obj(chunks), boolean_obj(true), dict_obj(opts)];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 3,
                capacity: 3,
            };
            rpc_send_event(ui_client_channel_id, c"nvim_echo".as_ptr(), args);
        }

        arena_mem_free(arena_finish(&raw mut err_arena));
        chunks.destroy();
        api_free_object(rv);
        api_clear_error(&raw mut lua_err);
    }
}

unsafe fn terminfo_start(tui: *mut TUIData) {
    unsafe {
        (*tui).bufpos = 0;
        (*tui).default_attr = false;
        (*tui).can_clear_attr = false;
        (*tui).is_invisible = true;
        (*tui).want_invisible = false;
        (*tui).busy = false;
        (*tui).set_cursor_color_as_str = false;
        (*tui).cursor_has_color = false;
        (*tui).resize_events_enabled = false;
        (*tui).modes_grapheme_clusters = false;
        (*tui).modes_resize_events = false;
        (*tui).modes_theme_updates = false;
        (*tui).showing_mode = SHAPE_IDX_N as ModeShape;
        (*tui).terminfo_ext.enable_focus_reporting = ptr::null();
        (*tui).terminfo_ext.disable_focus_reporting = ptr::null();

        (*tui).out_fd = STDOUT_FILENO;
        (*tui).out_isatty = os_isatty((*tui).out_fd);
        (*tui).input.tui_data = tui.cast();

        (*tui).ti_arena = ARENA_EMPTY;
        assert!((*tui).term.is_null());

        #[cfg(windows)]
        let mut term = os_getenv(c"TERM".as_ptr());
        #[cfg(not(windows))]
        let term = os_getenv(c"TERM".as_ptr());
        #[cfg(windows)]
        {
            let mut guessed_term: *const c_char = ptr::null();
            os_tty_guess_term(&raw mut guessed_term, (*tui).out_fd);
            if term.is_null() && !guessed_term.is_null() {
                term = xstrdup(guessed_term);
                os_setenv(c"TERM".as_ptr(), guessed_term, 1);
            }
        }

        (*tui).terminfo_found_in_db = false;
        if !term.is_null()
            && !os_env_exists(c"NVIM_TERMDEFS".as_ptr(), false)
            && terminfo_from_database(&raw mut (*tui).ti, term, &raw mut (*tui).ti_arena)
        {
            (*tui).term = arena_strdup(&raw mut (*tui).ti_arena, term);
            (*tui).terminfo_found_in_db = true;
        }

        if !(*tui).terminfo_found_in_db {
            let new_ti = terminfo_from_builtin(term, &raw mut (*tui).term);
            (*tui).ti = *new_ti;
        }

        let colorterm = os_getenv(c"COLORTERM".as_ptr());
        let termprg = os_getenv(c"TERM_PROGRAM".as_ptr());
        let vte_version_env = os_getenv(c"VTE_VERSION".as_ptr());
        let konsolev_env = os_getenv(c"KONSOLE_VERSION".as_ptr());
        let term_program_version_env = os_getenv(c"TERM_PROGRAM_VERSION".as_ptr());

        let mut vte_version_end = vte_version_env;
        let vtev = if !vte_version_env.is_null() {
            getdigits_int(&raw mut vte_version_end, false, 0)
        } else {
            0
        };
        let iterm_env = cstr_contains(termprg, b"iTerm.app");
        let nsterm = cstr_contains(termprg, b"Apple_Terminal")
            || terminfo_is_term_family(term, c"nsterm".as_ptr());
        let konsole = terminfo_is_term_family(term, c"konsole".as_ptr())
            || os_env_exists(c"KONSOLE_PROFILE_NAME".as_ptr(), true)
            || os_env_exists(c"KONSOLE_DBUS_SESSION".as_ptr(), true);
        let mut konsolev_end = konsolev_env;
        let konsolev = if !konsolev_env.is_null() {
            getdigits_int(&raw mut konsolev_end, false, 0)
        } else if konsole {
            1
        } else {
            0
        };
        let wezterm = strequal(termprg, c"WezTerm".as_ptr());
        let weztermv: *const c_char = if wezterm {
            term_program_version_env
        } else {
            ptr::null()
        };
        let screen = terminfo_is_term_family(term, c"screen".as_ptr());
        let tmux = terminfo_is_term_family(term, c"tmux".as_ptr())
            || os_env_exists(c"TMUX".as_ptr(), true);
        (*tui).screen_or_tmux = screen || tmux;

        (*tui).rgb = term_has_truecolor(tui, colorterm);

        patch_terminfo_bugs(tui, term, colorterm, vtev, konsolev, iterm_env, nsterm);
        augment_terminfo(tui, term, vtev, konsolev, weztermv, iterm_env, nsterm);
        apply_termdefs(tui);

        let has_set_underline_style = !(*tui).ti.defs[kTerm_set_underline_style].is_null();
        (*tui).can_change_scroll_region = !(*tui).ti.defs[kTerm_change_scroll_region].is_null();
        (*tui).can_set_lr_margin = !(*tui).ti.defs[kTerm_set_lr_margin].is_null();
        (*tui).can_scroll = !(*tui).ti.defs[kTerm_delete_line].is_null()
            && !(*tui).ti.defs[kTerm_parm_delete_line].is_null()
            && !(*tui).ti.defs[kTerm_insert_line].is_null()
            && !(*tui).ti.defs[kTerm_parm_insert_line].is_null();
        (*tui).can_erase_chars = !(*tui).ti.defs[kTerm_erase_chars].is_null();
        (*tui).immediate_wrap_after_last_column = terminfo_is_term_family(term, c"conemu".as_ptr())
            || terminfo_is_term_family(term, c"cygwin".as_ptr())
            || terminfo_is_term_family(term, c"win32con".as_ptr())
            || terminfo_is_term_family(term, c"interix".as_ptr());
        (*tui).bce = (*tui).ti.back_color_erase;
        t_colors = (*tui).ti.max_colors;

        terminfo_out(tui, kTerm_enter_ca_mode);
        terminfo_out(tui, kTerm_keypad_xmit);
        terminfo_out(tui, kTerm_clear_screen);

        tui_set_term_mode(tui, kTermModeBracketedPaste, true);

        (*tui).has_left_and_right_margin_mode = false;
        (*tui).has_sync_mode = false;

        if !nsterm {
            tui_request_term_mode(tui, kTermModeLeftAndRightMargins);
            tui_request_term_mode(tui, kTermModeSynchronizedOutput);
            tui_request_term_mode(tui, kTermModeGraphemeClusters);
            tui_request_term_mode(tui, kTermModeThemeUpdates);
            tui_request_term_mode(tui, kTermModeResizeEvents);
        }

        if !has_set_underline_style && !(screen || tmux || nsterm) {
            tui_query_extended_underline(tui);
        }

        tui_query_kitty_keyboard(tui);
        tui_query_bg_color_noflush(tui);

        let write_loop = tui_write_loop(tui);
        let output_handle = tui_output_handle(tui);
        debug_assert!(crate::event::r#loop::uv_loop_size() <= size_of::<[u64; 256]>());
        debug_assert!(
            crate::event::r#loop::uv_handle_size(crate::event::stream::UV_NAMED_PIPE).max(
                crate::event::r#loop::uv_handle_size(crate::event::stream::UV_TTY)
            ) <= size_of::<[u64; 80]>()
        );
        uv_loop_init(write_loop);
        if (*tui).out_isatty {
            let ret = uv_tty_init(write_loop, output_handle.cast(), (*tui).out_fd, 0);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"terminfo_start".as_ptr(),
                    628,
                    true,
                    c"uv_tty_init failed: %s".as_ptr(),
                    uv_strerror(ret),
                );
            }
            #[cfg(not(windows))]
            {
                let mut retry_count = 10;
                let mut ret;
                loop {
                    ret = uv_tty_set_mode(output_handle.cast(), UV_TTY_MODE_IO);
                    if ret == UV_EINTR && retry_count > 0 {
                        retry_count -= 1;
                    } else {
                        break;
                    }
                }
                if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"terminfo_start".as_ptr(),
                        638,
                        true,
                        c"uv_tty_set_mode failed: %s".as_ptr(),
                        uv_strerror(ret),
                    );
                }
            }
        } else {
            let ret = uv_pipe_init(write_loop, output_handle.cast(), 0);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"terminfo_start".as_ptr(),
                    644,
                    true,
                    c"uv_pipe_init failed: %s".as_ptr(),
                    uv_strerror(ret),
                );
            }
            let ret = uv_pipe_open(output_handle.cast(), (*tui).out_fd);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"terminfo_start".as_ptr(),
                    648,
                    true,
                    c"uv_pipe_open failed: %s".as_ptr(),
                    uv_strerror(ret),
                );
            }
        }
        flush_buf(tui, FlushBufFinish::kFlushBufFinal);

        xfree(term.cast());
        xfree(colorterm.cast());
        xfree(termprg.cast());
        xfree(vte_version_env.cast());
        xfree(konsolev_env.cast());
        xfree(term_program_version_env.cast());
    }
}

unsafe fn terminfo_disable(tui: *mut TUIData) {
    unsafe {
        if (*tui).modes_theme_updates {
            tui_set_term_mode(tui, kTermModeThemeUpdates, false);
        }

        tui_mode_change(tui, NULL_STRING, SHAPE_IDX_N as Integer);
        tui_mouse_off(tui);
        terminfo_out(tui, kTerm_exit_attribute_mode);
        terminfo_out(tui, kTerm_cursor_normal);
        terminfo_out(tui, kTerm_reset_cursor_style);
        terminfo_out(tui, kTerm_keypad_local);

        tui_reset_key_encoding(tui);

        if (*tui).modes_resize_events {
            tui_set_term_mode(tui, kTermModeResizeEvents, false);
        }

        if (*tui).modes_grapheme_clusters {
            tui_set_term_mode(tui, kTermModeGraphemeClusters, false);
        }

        tui_set_title(tui, NULL_STRING);
        if (*tui).cursor_has_color {
            terminfo_out(tui, kTerm_reset_cursor_color);
        }
        tui_set_term_mode(tui, kTermModeBracketedPaste, false);
        out_len(tui, (*tui).terminfo_ext.disable_focus_reporting);

        let da1 = b"\x1b[c";
        out(tui, da1.as_ptr().cast::<c_char>(), da1.len());

        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
    }
}

unsafe fn terminfo_stop(tui: *mut TUIData) {
    unsafe {
        if ui_client_exit_status == 0 && ui_client_error_exit > 0 {
            ui_client_exit_status = ui_client_error_exit;
        }

        if ui_client_exit_status == ui_client_error_exit.max(0) {
            cursor_goto(tui, (*tui).height - 1, 0);
            terminfo_out(tui, kTerm_exit_ca_mode);
        }

        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
        uv_tty_reset_mode();
        uv_close(tui_output_handle(tui), None);
        uv_run(tui_write_loop(tui), UV_RUN_DEFAULT);
        if uv_loop_close(tui_write_loop(tui)) != 0 {
            std::process::abort();
        }
        arena_mem_free(arena_finish(&raw mut (*tui).ti_arena));
        ptr::write_bytes(&raw mut (*tui).ti, 0, 1);
        (*tui).term = ptr::null_mut();
    }
}

unsafe fn tui_terminal_start(tui: *mut TUIData) {
    unsafe {
        (*tui).print_attr_id = -1;
        terminfo_start(tui);
        let input = tui_input(tui);
        if (*input).loop_.is_null() {
            tinput_init(
                input,
                (&raw mut main_loop).cast::<Loop>(),
                &raw mut (*tui).ti,
            );
        }
        tui_guess_size(tui);
        tinput_start(input);
    }
}

unsafe extern "C" fn after_startup_cb(handle: *mut uv_timer_t) {
    unsafe {
        let tui = *handle.cast::<*mut TUIData>();
        tui_terminal_after_startup(tui);
    }
}

unsafe fn tui_terminal_after_startup(tui: *mut TUIData) {
    unsafe {
        out_len(tui, (*tui).terminfo_ext.enable_focus_reporting);
        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
    }
}

pub unsafe fn tui_stop(tui: *mut TUIData) {
    unsafe {
        if uv_is_closing(tui_output_handle(tui)) != 0 {
            if g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"tui_stop".as_ptr(),
                    760,
                    true,
                    c"TUI already stopped (race?)".as_ptr(),
                );
            }
            (*tui).stopped = true;
            return;
        }

        let input = tui_input(tui);
        (*input).callbacks.primary_device_attr = Some(tui_stop_cb);
        terminfo_disable(tui);

        let wait_start = os_hrtime();
        let events = (*loop_queues((*tui).loop_)).events;
        let rs = &raw mut (*input).read_stream;
        let mut remaining: i64 = EXIT_TIMEOUT_MS;
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !(ptr::read_volatile(&raw const (*tui).stopped)
            || ptr::read_volatile(&raw const (*rs).did_eof))
        {
            if !events.is_null() && !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events((*tui).loop_, remaining);
            }
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
        if !(*tui).stopped
            && ((os_hrtime() - wait_start) / 1_000_000) as i64 >= EXIT_TIMEOUT_MS
            && g_min_log_level <= LOGLVL_WRN
        {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"tui_stop".as_ptr(),
                773,
                true,
                c"TUI: timed out waiting for DA1 response".as_ptr(),
            );
        }
        (*tui).stopped = true;

        tui_terminal_stop(tui);
        stream_set_blocking((*input).in_fd, true);
        tinput_destroy(input);
        signal_watcher_stop(tui_winch_handle(tui));
        signal_watcher_close(tui_winch_handle(tui), None);
        uv_close(tui_startup_delay_timer(tui).cast(), None);
    }
}

unsafe fn tui_stop_cb(tui: *mut TUIData) {
    unsafe {
        (*tui).stopped = true;
    }
}

unsafe fn tui_terminal_stop(tui: *mut TUIData) {
    unsafe {
        tinput_stop(tui_input(tui));
        terminfo_stop(tui);
    }
}

pub unsafe fn tui_is_stopped(tui: *mut TUIData) -> bool {
    unsafe { (*tui).stopped }
}

pub unsafe fn tui_free_all_mem(tui: *mut TUIData) {
    unsafe {
        ugrid_free(&mut (*tui).grid);

        let urls_ptr = &raw mut urls;
        *urls_ptr = Vec::new();

        drop(Box::from_raw(tui));
    }
}

unsafe fn sigwinch_cb(_watcher: *mut SignalWatcher, _signum: c_int, cbdata: *mut c_void) {
    unsafe {
        let tui = cbdata.cast::<TUIData>();
        if tui_is_stopped(tui) || (*tui).resize_events_enabled {
            return;
        }
        tui_guess_size(tui);
    }
}

unsafe fn attrs_differ(tui: *mut TUIData, id1: c_int, id2: c_int, rgb: bool) -> bool {
    unsafe {
        if id1 == id2 {
            return false;
        } else if id1 < 0 || id2 < 0 {
            return true;
        }
        let attrs = &(*tui).attrs;
        let a1 = attrs[id1 as usize];
        let a2 = attrs[id2 as usize];

        if a1.url != a2.url {
            return true;
        }

        if rgb {
            a1.rgb_fg_color != a2.rgb_fg_color
                || a1.rgb_bg_color != a2.rgb_bg_color
                || a1.rgb_ae_attr != a2.rgb_ae_attr
                || a1.rgb_sp_color != a2.rgb_sp_color
        } else {
            a1.cterm_fg_color != a2.cterm_fg_color
                || a1.cterm_bg_color != a2.cterm_bg_color
                || a1.cterm_ae_attr != a2.cterm_ae_attr
                || ((a1.cterm_ae_attr & HL_UNDERLINE_MASK) != 0
                    && a1.rgb_sp_color != a2.rgb_sp_color)
        }
    }
}

unsafe fn update_attrs(tui: *mut TUIData, attr_id: c_int) {
    unsafe {
        if !attrs_differ(tui, attr_id, (*tui).print_attr_id, (*tui).rgb) {
            (*tui).print_attr_id = attr_id;
            return;
        }
        (*tui).print_attr_id = attr_id;
        let attrs = (*tui).attrs.as_slice()[attr_id as usize];
        let attr = if (*tui).rgb {
            attrs.rgb_ae_attr
        } else {
            attrs.cterm_ae_attr
        };

        let bold = (attr & HL_BOLD) != 0;
        let italic = (attr & HL_ITALIC) != 0;
        let reverse = (attr & HL_INVERSE) != 0;
        let standout = (attr & HL_STANDOUT) != 0;
        let strikethrough = (attr & HL_STRIKETHROUGH) != 0;
        let altfont = (attr & HL_ALTFONT) != 0;
        let dim = (attr & HL_DIM) != 0;
        let blink = (attr & HL_BLINK) != 0;
        let conceal = (attr & HL_CONCEALED) != 0;
        let overline = (attr & HL_OVERLINE) != 0;

        let underline: bool;
        let undercurl: bool;
        let underdouble: bool;
        let underdotted: bool;
        let underdashed: bool;
        if !(*tui).ti.defs[kTerm_set_underline_style].is_null() {
            let ul = attr & HL_UNDERLINE_MASK;
            underline = ul == HL_UNDERLINE;
            undercurl = ul == HL_UNDERCURL;
            underdouble = ul == HL_UNDERDOUBLE;
            underdashed = ul == HL_UNDERDASHED;
            underdotted = ul == HL_UNDERDOTTED;
        } else {
            underline = (attr & HL_UNDERLINE_MASK) != 0;
            undercurl = false;
            underdouble = false;
            underdotted = false;
            underdashed = false;
        }

        let has_any_underline = undercurl || underline || underdouble || underdotted || underdashed;

        if !(*tui).ti.defs[kTerm_set_attributes].is_null() {
            if bold || dim || blink || reverse || underline || standout {
                let mut params = [TPVAR {
                    num: 0,
                    string: ptr::null_mut(),
                }; 9];
                params[0].num = c_long::from(standout);
                params[1].num = c_long::from(underline);
                params[2].num = c_long::from(reverse);
                params[3].num = c_long::from(blink);
                params[4].num = c_long::from(dim);
                params[5].num = c_long::from(bold);
                params[6].num = 0;
                params[7].num = 0;
                params[8].num = 0;
                terminfo_print(tui, kTerm_set_attributes, params.as_mut_ptr());
            } else if !(*tui).default_attr {
                terminfo_out(tui, kTerm_exit_attribute_mode);
            }
        } else {
            if !(*tui).default_attr {
                terminfo_out(tui, kTerm_exit_attribute_mode);
            }
            if bold {
                terminfo_out(tui, kTerm_enter_bold_mode);
            }
            if underline {
                terminfo_out(tui, kTerm_enter_underline_mode);
            }
            if standout {
                terminfo_out(tui, kTerm_enter_standout_mode);
            }
            if reverse {
                terminfo_out(tui, kTerm_enter_reverse_mode);
            }
            if dim {
                terminfo_out(tui, kTerm_enter_dim_mode);
            }
            if blink {
                terminfo_out(tui, kTerm_enter_blink_mode);
            }
        }
        if italic {
            terminfo_out(tui, kTerm_enter_italics_mode);
        }
        if altfont {
            out_len(tui, (*tui).terminfo_ext.enter_altfont_mode);
        }
        if strikethrough {
            terminfo_out(tui, kTerm_enter_strikethrough_mode);
        }
        if conceal {
            terminfo_out(tui, kTerm_enter_secure_mode);
        }
        if overline {
            let seq = b"\x1b[53m";
            out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
        }
        if !(*tui).ti.defs[kTerm_set_underline_style].is_null() {
            if undercurl {
                terminfo_print_num1(tui, kTerm_set_underline_style, 3);
            }
            if underdouble {
                terminfo_print_num1(tui, kTerm_set_underline_style, 2);
            }
            if underdotted {
                terminfo_print_num1(tui, kTerm_set_underline_style, 4);
            }
            if underdashed {
                terminfo_print_num1(tui, kTerm_set_underline_style, 5);
            }
        }

        if has_any_underline && (*tui).can_set_underline_color {
            let color = attrs.rgb_sp_color;
            if color != -1 {
                let r = (color >> 16) & 0xff;
                let g = (color >> 8) & 0xff;
                let b = color & 0xff;
                let s = format!("\x1b[58:2::{r}:{g}:{b}m");
                out_printf_str(tui, 128, &s);
            }
        }

        let fg: c_int;
        if (*tui).rgb && (attr & HL_FG_INDEXED) == 0 {
            fg = if attrs.rgb_fg_color != -1 {
                attrs.rgb_fg_color
            } else {
                (*tui).clear_attrs.rgb_fg_color
            };
            if fg != -1 {
                terminfo_print_num(
                    tui,
                    kTerm_set_rgb_foreground,
                    (fg >> 16) & 0xff,
                    (fg >> 8) & 0xff,
                    fg & 0xff,
                );
            }
        } else {
            fg = if attrs.cterm_fg_color != 0 {
                c_int::from(attrs.cterm_fg_color) - 1
            } else {
                c_int::from((*tui).clear_attrs.cterm_fg_color) - 1
            };
            if fg != -1 {
                terminfo_print_num1(tui, kTerm_set_a_foreground, fg);
            }
        }

        let bg: c_int;
        if (*tui).rgb && (attr & HL_BG_INDEXED) == 0 {
            bg = if attrs.rgb_bg_color != -1 {
                attrs.rgb_bg_color
            } else {
                (*tui).clear_attrs.rgb_bg_color
            };
            if bg != -1 {
                terminfo_print_num(
                    tui,
                    kTerm_set_rgb_background,
                    (bg >> 16) & 0xff,
                    (bg >> 8) & 0xff,
                    bg & 0xff,
                );
            }
        } else {
            bg = if attrs.cterm_bg_color != 0 {
                c_int::from(attrs.cterm_bg_color) - 1
            } else {
                c_int::from((*tui).clear_attrs.cterm_bg_color) - 1
            };
            if bg != -1 {
                terminfo_print_num1(tui, kTerm_set_a_background, bg);
            }
        }

        if (*tui).url != attrs.url {
            if attrs.url >= 0 {
                let urls_ptr = &raw const urls;
                let url_str = (*urls_ptr).as_slice()[attrs.url as usize].to_string_lossy();
                let id: u64 = 0xE1EA_0000u64 + u64::from(attrs.url as u32);
                let formatted = format!("\x1b]8;id={id};{url_str}\x1b\\");
                out(tui, formatted.as_ptr().cast::<c_char>(), formatted.len());
            } else {
                let seq = b"\x1b]8;;\x1b\\";
                out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            }
            (*tui).url = attrs.url;
        }

        (*tui).default_attr = fg == -1
            && bg == -1
            && !bold
            && !dim
            && !blink
            && !conceal
            && !overline
            && !italic
            && !has_any_underline
            && !reverse
            && !standout
            && !strikethrough;

        (*tui).can_clear_attr = !reverse
            && !standout
            && !dim
            && !blink
            && !conceal
            && !overline
            && !has_any_underline
            && !strikethrough
            && ((*tui).bce || bg == -1);
    }
}

unsafe fn final_column_wrap(tui: *mut TUIData) {
    unsafe {
        let grid = &mut (*tui).grid;
        if grid.row != -1 && grid.col == (*tui).width {
            grid.col = 0;
            if grid.row < (*tui).height.min(grid.height - 1) {
                grid.row += 1;
            }
        }
    }
}

unsafe fn print_cell(tui: *mut TUIData, buf: *const c_char, attr: sattr_T) {
    unsafe {
        if !(*tui).immediate_wrap_after_last_column {
            final_column_wrap(tui);
        }
        update_attrs(tui, attr);
        let len = CStr::from_ptr(buf).to_bytes().len();
        out(tui, buf, len);
        (*tui).grid.col += 1;
        if (*tui).immediate_wrap_after_last_column {
            final_column_wrap(tui);
        }
    }
}

unsafe fn cheap_to_print(tui: *mut TUIData, row: c_int, col: c_int, mut next: c_int) -> bool {
    unsafe {
        let cells = &(*tui).grid.cells;
        let row_cells = &cells[row as usize];
        let mut idx = col as usize;
        while next != 0 {
            next -= 1;
            let cell = row_cells[idx];
            if attrs_differ(tui, cell.attr, (*tui).print_attr_id, (*tui).rgb) && (*tui).default_attr
            {
                return false;
            }
            if schar_get_ascii(cell.data) == 0 {
                return false;
            }
            idx += 1;
        }
        true
    }
}

unsafe fn cursor_goto(tui: *mut TUIData, row: c_int, col: c_int) {
    unsafe {
        if row == (*tui).grid.row && col == (*tui).grid.col {
            return;
        }

        if (*tui).url >= 0 {
            let seq = b"\x1b]8;;\x1b\\";
            out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            (*tui).url = -1;
            (*tui).print_attr_id = -1;
        }

        if row == 0 && col == 0 {
            terminfo_out(tui, kTerm_cursor_home);
            ugrid_goto(&mut (*tui).grid, row, col);
            return;
        }
        if (*tui).grid.row >= 0 {
            let do_cr = if col == 0 {
                col != (*tui).grid.col
            } else if row != (*tui).grid.row {
                false
            } else if col == 1 {
                2 < (*tui).grid.col && cheap_to_print(tui, (*tui).grid.row, 0, col)
            } else if col == 2 {
                5 < (*tui).grid.col && cheap_to_print(tui, (*tui).grid.row, 0, col)
            } else {
                false
            };
            if do_cr {
                terminfo_out(tui, kTerm_carriage_return);
                let cur_row = (*tui).grid.row;
                ugrid_goto(&mut (*tui).grid, cur_row, 0);
            }
            if row == (*tui).grid.row {
                if col < (*tui).grid.col
                    && ((*tui).immediate_wrap_after_last_column || (*tui).grid.col < (*tui).width)
                {
                    let n = (*tui).grid.col - col;
                    if n <= 4 {
                        for _ in 0..n {
                            terminfo_out(tui, kTerm_cursor_left);
                        }
                    } else {
                        terminfo_print_num1(tui, kTerm_parm_left_cursor, n);
                    }
                    ugrid_goto(&mut (*tui).grid, row, col);
                    return;
                } else if col > (*tui).grid.col {
                    let n = col - (*tui).grid.col;
                    if n <= 2 {
                        for _ in 0..n {
                            terminfo_out(tui, kTerm_cursor_right);
                        }
                    } else {
                        terminfo_print_num1(tui, kTerm_parm_right_cursor, n);
                    }
                    ugrid_goto(&mut (*tui).grid, row, col);
                    return;
                }
            }
            if col == (*tui).grid.col {
                if row > (*tui).grid.row {
                    let n = row - (*tui).grid.row;
                    if n <= 4 {
                        for _ in 0..n {
                            terminfo_out(tui, kTerm_cursor_down);
                        }
                    } else {
                        terminfo_print_num1(tui, kTerm_parm_down_cursor, n);
                    }
                    ugrid_goto(&mut (*tui).grid, row, col);
                    return;
                } else if row < (*tui).grid.row {
                    let n = (*tui).grid.row - row;
                    if n <= 2 {
                        for _ in 0..n {
                            terminfo_out(tui, kTerm_cursor_up);
                        }
                    } else {
                        terminfo_print_num1(tui, kTerm_parm_up_cursor, n);
                    }
                    ugrid_goto(&mut (*tui).grid, row, col);
                    return;
                }
            }
        }

        terminfo_print_num2(tui, kTerm_cursor_address, row, col);
        ugrid_goto(&mut (*tui).grid, row, col);
    }
}

unsafe fn print_spaces(tui: *mut TUIData, width: c_int) {
    unsafe {
        let mut left = width as usize;
        loop {
            let buf_fit = left.min((*tui).buf.len() - (*tui).bufpos);
            ptr::write_bytes((*tui).buf.as_mut_ptr().add((*tui).bufpos), b' ', buf_fit);
            (*tui).bufpos += buf_fit;
            left -= buf_fit;
            if left == 0 {
                break;
            }
            flush_buf(tui, FlushBufFinish::kFlushBufPartial);
        }

        (*tui).grid.col += width;
        if (*tui).immediate_wrap_after_last_column {
            final_column_wrap(tui);
        }
    }
}

unsafe fn print_cell_at_pos(
    tui: *mut TUIData,
    row: c_int,
    col: c_int,
    cell: UCell,
    is_doublewidth: bool,
) {
    unsafe {
        if (*tui).grid.row == -1 && cell.data == 0 {
            return;
        }

        cursor_goto(tui, row, col);

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(buf.as_mut_ptr(), cell.data);
        let c = utf_ptr2char(buf.as_ptr());
        let mut is_ambiwidth = utf_ambiguous_width(buf.as_ptr());
        if is_doublewidth && (is_ambiwidth || utf_char2cells(c) == 1) {
            is_ambiwidth = true;
            update_attrs(tui, cell.attr);
            print_spaces(tui, 2);
            cursor_goto(tui, row, col);
        }

        print_cell(tui, buf.as_ptr(), cell.attr);

        if is_ambiwidth {
            (*tui).grid.row = -1;
        }
    }
}

unsafe fn clear_region(
    tui: *mut TUIData,
    top: c_int,
    bot: c_int,
    left: c_int,
    right: c_int,
    attr_id: c_int,
) {
    unsafe {
        if (*tui).set_default_colors {
            update_attrs(tui, attr_id);
        } else {
            terminfo_out(tui, kTerm_exit_attribute_mode);
        }

        if (*tui).can_clear_attr && left == 0 && right == (*tui).width && bot == (*tui).height {
            if top == 0 {
                terminfo_out(tui, kTerm_clear_screen);
                ugrid_goto(&mut (*tui).grid, top, left);
            } else {
                cursor_goto(tui, top, 0);
                terminfo_out(tui, kTerm_clr_eos);
            }
        } else {
            let width = right - left;
            for row in top..bot {
                cursor_goto(tui, row, left);
                if (*tui).can_clear_attr && right == (*tui).width {
                    terminfo_out(tui, kTerm_clr_eol);
                } else if (*tui).can_erase_chars && (*tui).can_clear_attr {
                    terminfo_print_num1(tui, kTerm_erase_chars, width);
                } else {
                    print_spaces(tui, width);
                }
            }
        }
    }
}

unsafe fn set_scroll_region(tui: *mut TUIData, top: c_int, bot: c_int, left: c_int, right: c_int) {
    unsafe {
        terminfo_print_num2(tui, kTerm_change_scroll_region, top, bot);
        if left != 0 || right != (*tui).width - 1 {
            tui_set_term_mode(tui, kTermModeLeftAndRightMargins, true);
            terminfo_print_num2(tui, kTerm_set_lr_margin, left, right);
        }
        (*tui).grid.row = -1;
    }
}

unsafe fn reset_scroll_region(tui: *mut TUIData, fullwidth: bool) {
    unsafe {
        if !(*tui).terminfo_ext.reset_scroll_region.is_null() {
            out_len(tui, (*tui).terminfo_ext.reset_scroll_region);
        } else {
            terminfo_print_num2(tui, kTerm_change_scroll_region, 0, (*tui).height - 1);
        }
        if !fullwidth {
            terminfo_print_num2(tui, kTerm_set_lr_margin, 0, (*tui).width - 1);
            tui_set_term_mode(tui, kTermModeLeftAndRightMargins, false);
        }
        (*tui).grid.row = -1;
    }
}

pub unsafe fn tui_grid_resize(tui: *mut TUIData, _g: Integer, width: Integer, height: Integer) {
    unsafe {
        ugrid_resize(&mut (*tui).grid, width as c_int, height as c_int);

        let grid_height = (*tui).grid.height;
        let grid_width = (*tui).grid.width;
        for r in &mut (*tui).invalid_regions {
            r.bot = r.bot.min(grid_height);
            r.right = r.right.min(grid_width);
        }

        if (*tui).pending_resize_events == 0 && !(*tui).is_starting {
            let s = format!("\x1b[8;{};{}t", height as c_int, width as c_int);
            out_printf_str(tui, 64, &s);
        } else {
            (*tui).pending_resize_events = if (*tui).pending_resize_events > 0 {
                (*tui).pending_resize_events - 1
            } else {
                0
            };
            (*tui).grid.row = -1;
        }
    }
}

pub unsafe fn tui_grid_clear(tui: *mut TUIData, _g: Integer) {
    unsafe {
        ugrid_clear(&mut (*tui).grid);
        schar_cache_clear_if_full();
        (*tui).invalid_regions.clear();
        clear_region(tui, 0, (*tui).height, 0, (*tui).width, 0);
    }
}

pub unsafe fn tui_grid_cursor_goto(tui: *mut TUIData, _grid: Integer, row: Integer, col: Integer) {
    unsafe {
        (*tui).row = row as c_int;
        (*tui).col = col as c_int;
    }
}

unsafe fn tui_cursor_decode_shape(shape_str: *const c_char) -> CursorShape {
    unsafe {
        if strequal(shape_str, c"block".as_ptr()) {
            CursorShape::SHAPE_BLOCK
        } else if strequal(shape_str, c"vertical".as_ptr()) {
            CursorShape::SHAPE_VER
        } else if strequal(shape_str, c"horizontal".as_ptr()) {
            CursorShape::SHAPE_HOR
        } else {
            if g_min_log_level <= LOGLVL_WRN {
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"tui_cursor_decode_shape".as_ptr(),
                    1268,
                    true,
                    c"Unknown shape value '%s'".as_ptr(),
                    shape_str,
                );
            }
            CursorShape::SHAPE_BLOCK
        }
    }
}

unsafe fn tui_cursor_reset_style(tui: *mut TUIData) {
    unsafe {
        terminfo_out(tui, kTerm_reset_cursor_style);
    }
}

unsafe fn decode_cursor_entry(args: Dict) -> cursorentry_T {
    unsafe {
        let mut r = shape_table[0];
        for i in 0..args.size {
            let item = *args.items.add(i);
            let key = item.key.data;
            let value = item.value;
            if strequal(key, c"cursor_shape".as_ptr()) {
                r.shape = tui_cursor_decode_shape(value.data.string.data);
            } else if strequal(key, c"blinkon".as_ptr()) {
                r.blinkon = value.data.integer as c_int;
            } else if strequal(key, c"blinkoff".as_ptr()) {
                r.blinkoff = value.data.integer as c_int;
            } else if strequal(key, c"attr_id".as_ptr()) {
                r.id = value.data.integer as c_int;
            }
        }
        r
    }
}

pub unsafe fn tui_mode_info_set(tui: *mut TUIData, guicursor_enabled: bool, args: Array) {
    unsafe {
        cursor_style_enabled = guicursor_enabled;
        if !guicursor_enabled {
            tui_cursor_reset_style(tui);
            return;
        }

        assert!(args.size != 0);

        for i in 0..args.size {
            let item = *args.items.add(i);
            assert!(item.obj_type == ObjectType::kObjectTypeDict);
            let r = decode_cursor_entry(item.data.dict);
            (*tui).cursor_shapes[i] = r;
        }

        tui_set_mode(tui, (*tui).showing_mode);
    }
}

pub unsafe fn tui_update_menu(_tui: *mut TUIData) {}

pub unsafe fn tui_busy_start(tui: *mut TUIData) {
    unsafe {
        (*tui).busy = true;
    }
}

pub unsafe fn tui_busy_stop(tui: *mut TUIData) {
    unsafe {
        (*tui).busy = false;
    }
}

pub unsafe fn tui_mouse_on(tui: *mut TUIData) {
    unsafe {
        if !(*tui).mouse_enabled {
            tui_set_term_mode(tui, kTermModeMouseButtonEvent, true);
            tui_set_term_mode(tui, kTermModeMouseSGRExt, true);
            if (*tui).mouse_move_enabled {
                tui_set_term_mode(tui, kTermModeMouseAnyEvent, true);
            }
            (*tui).mouse_enabled = true;
        }
    }
}

pub unsafe fn tui_mouse_off(tui: *mut TUIData) {
    unsafe {
        if (*tui).mouse_enabled {
            if (*tui).mouse_move_enabled {
                tui_set_term_mode(tui, kTermModeMouseAnyEvent, false);
            }
            tui_set_term_mode(tui, kTermModeMouseButtonEvent, false);
            tui_set_term_mode(tui, kTermModeMouseSGRExt, false);
            (*tui).mouse_enabled = false;
        }
    }
}

unsafe fn tui_set_mode(tui: *mut TUIData, mode: ModeShape) {
    unsafe {
        if !cursor_style_enabled {
            tui_cursor_reset_style(tui);
            return;
        }
        let c = (*tui).cursor_shapes[mode as usize];

        if c.id != 0 && (c.id as usize) < (*tui).attrs.len() && (*tui).rgb {
            let aep = (*tui).attrs.as_slice()[c.id as usize];
            (*tui).want_invisible = aep.hl_blend == 100;
            if !(*tui).want_invisible && (aep.rgb_ae_attr & HL_INVERSE) != 0 {
                terminfo_out(tui, kTerm_reset_cursor_color);
            } else if !(*tui).want_invisible && aep.rgb_bg_color >= 0 {
                let mut params = [TPVAR {
                    num: 0,
                    string: ptr::null_mut(),
                }; 9];
                let mut hexbuf = [0u8; 8];
                if (*tui).set_cursor_color_as_str {
                    let s = format!("#{:06x}", aep.rgb_bg_color as u32 & 0x00ff_ffff);
                    hexbuf[..7].copy_from_slice(s.as_bytes());
                    hexbuf[7] = 0;
                    params[0].string = hexbuf.as_mut_ptr().cast::<c_char>();
                } else {
                    params[0].num = c_long::from(aep.rgb_bg_color);
                }
                terminfo_print(tui, kTerm_set_cursor_color, params.as_mut_ptr());
                (*tui).cursor_has_color = true;
            }
        } else if c.id == 0 && ((*tui).want_invisible || (*tui).cursor_has_color) {
            (*tui).want_invisible = false;
            (*tui).cursor_has_color = false;
            terminfo_out(tui, kTerm_reset_cursor_color);
        }

        let shape = match c.shape {
            CursorShape::SHAPE_BLOCK => 1,
            CursorShape::SHAPE_HOR => 3,
            CursorShape::SHAPE_VER => 5,
        };
        let steady = c_int::from(c.blinkon == 0 || c.blinkoff == 0);
        terminfo_print_num1(tui, kTerm_set_cursor_style, shape + steady);
    }
}

pub unsafe fn tui_mode_change(tui: *mut TUIData, _mode: NvimString, mode_idx: Integer) {
    unsafe {
        #[cfg(unix)]
        if (*tui).out_isatty && (*tui).is_starting && !stdin_isatty {
            let ret = uv_tty_set_mode(tui_output_handle(tui).cast(), UV_TTY_MODE_NORMAL);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"tui_mode_change".as_ptr(),
                    1413,
                    true,
                    c"uv_tty_set_mode failed: %s".as_ptr(),
                    uv_strerror(ret),
                );
            }
            let ret = uv_tty_set_mode(tui_output_handle(tui).cast(), UV_TTY_MODE_IO);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"tui_mode_change".as_ptr(),
                    1417,
                    true,
                    c"uv_tty_set_mode failed: %s".as_ptr(),
                    uv_strerror(ret),
                );
            }
        }
        tui_set_mode(tui, mode_idx as ModeShape);
        if (*tui).is_starting && (*tui).verbose >= 3 {
            show_verbose_terminfo(tui);
        }
        (*tui).is_starting = false;
        (*tui).showing_mode = mode_idx as ModeShape;
    }
}

pub unsafe fn tui_grid_scroll(
    tui: *mut TUIData,
    _g: Integer,
    mut startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    endcol: Integer,
    rows: Integer,
    _cols: Integer,
) {
    unsafe {
        let top = startrow as c_int;
        let bot = (endrow - 1) as c_int;
        let left = startcol as c_int;
        let right = (endcol - 1) as c_int;

        let fullwidth = left == 0 && right == (*tui).width - 1;
        let full_screen_scroll = fullwidth && top == 0 && bot == (*tui).height - 1;

        ugrid_scroll(&mut (*tui).grid, top, bot, left, right, rows as c_int);

        let has_lr_margins = (*tui).has_left_and_right_margin_mode && (*tui).can_set_lr_margin;
        let can_scroll = (*tui).can_scroll
            && (full_screen_scroll
                || ((*tui).can_change_scroll_region
                    && ((left == 0 && right == (*tui).width - 1) || has_lr_margins)));

        if can_scroll {
            if !full_screen_scroll {
                set_scroll_region(tui, top, bot, left, right);
            }
            cursor_goto(tui, top, left);
            update_attrs(tui, 0);

            if rows > 0 {
                if rows == 1 {
                    terminfo_out(tui, kTerm_delete_line);
                } else {
                    terminfo_print_num1(tui, kTerm_parm_delete_line, rows as c_int);
                }
            } else if rows == -1 {
                terminfo_out(tui, kTerm_insert_line);
            } else {
                terminfo_print_num1(tui, kTerm_parm_insert_line, -(rows as c_int));
            }

            if !full_screen_scroll {
                reset_scroll_region(tui, fullwidth);
            }
        } else {
            if rows > 0 {
                endrow -= rows;
            } else {
                startrow -= rows;
            }
            invalidate(
                tui,
                startrow as c_int,
                endrow as c_int,
                startcol as c_int,
                endcol as c_int,
            );
        }
    }
}

pub unsafe fn tui_add_url(_tui: *mut TUIData, url: *const c_char) -> i32 {
    unsafe {
        if url.is_null() {
            return -1;
        }

        let url_cstr = CStr::from_ptr(url);
        let urls_ptr = &raw mut urls;
        if let Some(pos) = (*urls_ptr).iter().position(|u| u.as_c_str() == url_cstr) {
            return pos as i32;
        }
        let idx = (*urls_ptr).len() as i32;
        (*urls_ptr).push(url_cstr.to_owned());
        idx
    }
}

pub unsafe fn tui_hl_attr_define(
    tui: *mut TUIData,
    id: Integer,
    mut attrs: HlAttrs,
    cterm_attrs: HlAttrs,
    _info: Array,
) {
    unsafe {
        attrs.cterm_ae_attr = cterm_attrs.cterm_ae_attr;
        attrs.cterm_fg_color = cterm_attrs.cterm_fg_color;
        attrs.cterm_bg_color = cterm_attrs.cterm_bg_color;

        let idx = id as usize;
        let tui_attrs = &mut (*tui).attrs;
        if tui_attrs.len() <= idx {
            tui_attrs.resize(idx + 1, HLATTRS_INIT);
        }
        tui_attrs[idx] = attrs;
    }
}

pub unsafe fn tui_bell(tui: *mut TUIData) {
    unsafe {
        let seq = b"\x07";
        out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
    }
}

pub unsafe fn tui_visual_bell(tui: *mut TUIData) {
    unsafe {
        if (*tui).screen_or_tmux {
            let seq = b"\x1bg";
            out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
        } else {
            let on = b"\x1b[?5h";
            out(tui, on.as_ptr().cast::<c_char>(), on.len());

            flush_buf(tui, FlushBufFinish::kFlushBufFinal);
            uv_sleep(100);

            let off = b"\x1b[?5l";
            out(tui, off.as_ptr().cast::<c_char>(), off.len());
        }
        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
    }
}

pub unsafe fn tui_default_colors_set(
    tui: *mut TUIData,
    rgb_fg: Integer,
    rgb_bg: Integer,
    rgb_sp: Integer,
    cterm_fg: Integer,
    cterm_bg: Integer,
) {
    unsafe {
        (*tui).clear_attrs.rgb_fg_color = rgb_fg as i32;
        (*tui).clear_attrs.rgb_bg_color = rgb_bg as i32;
        (*tui).clear_attrs.rgb_sp_color = rgb_sp as i32;
        (*tui).clear_attrs.cterm_fg_color = cterm_fg as i16;
        (*tui).clear_attrs.cterm_bg_color = cterm_bg as i16;

        (*tui).print_attr_id = -1;
        (*tui).set_default_colors = true;
        invalidate(tui, 0, (*tui).grid.height, 0, (*tui).grid.width);
    }
}

pub unsafe fn tui_ui_send(tui: *mut TUIData, content: NvimString) {
    unsafe {
        if !(*tui).invalid_regions.is_empty() || (*tui).bufpos > 0 {
            out(tui, content.data, content.size);
            return;
        }
        let mut req = [0u64; 64];
        let buf = uv_buf_t::new(content.data, content.size);
        let ret = uv_write(
            req.as_mut_ptr().cast(),
            tui_output_handle(tui).cast(),
            &raw const buf,
            1,
            None,
        );
        if ret != 0 && g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"tui_ui_send".as_ptr(),
                1569,
                true,
                c"uv_write failed: %s".as_ptr(),
                uv_strerror(ret),
            );
        }
        uv_run(tui_write_loop(tui), UV_RUN_DEFAULT);
    }
}

pub unsafe fn tui_flush(tui: *mut TUIData) {
    unsafe {
        let nrevents = loop_size((*tui).loop_);
        if nrevents > TOO_MANY_EVENTS {
            if g_min_log_level <= LOGLVL_WRN {
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"tui_flush".as_ptr(),
                    1583,
                    true,
                    c"TUI event-queue flooded (thread_events=%zu); purging".as_ptr(),
                    nrevents,
                );
            }
            loop_purge((*tui).loop_);
            tui_busy_stop(tui);
        }

        while let Some(r) = (*tui).invalid_regions.pop() {
            assert!(r.bot <= (*tui).grid.height && r.right <= (*tui).grid.width);

            for row in r.top..r.bot {
                let clear_attr = {
                    let cells = &(*tui).grid.cells;
                    cells[row as usize][(r.right - 1) as usize].attr
                };
                let mut clear_col = r.right;
                while clear_col > 0 {
                    let cell = {
                        let cells = &(*tui).grid.cells;
                        cells[row as usize][(clear_col - 1) as usize]
                    };
                    if !(cell.data == schar_from_ascii(b' ') && cell.attr == clear_attr) {
                        break;
                    }
                    clear_col -= 1;
                }

                for curcol in r.left..clear_col {
                    let (cell, is_double) = {
                        let cells = &(*tui).grid.cells;
                        let row_cells = &cells[row as usize];
                        (
                            row_cells[curcol as usize],
                            curcol < clear_col - 1 && row_cells[(curcol + 1) as usize].data == 0,
                        )
                    };
                    print_cell_at_pos(tui, row, curcol, cell, is_double);
                }
                if clear_col < r.right {
                    clear_region(tui, row, row + 1, clear_col, r.right, clear_attr);
                }
            }
        }

        cursor_goto(tui, (*tui).row, (*tui).col);
        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
    }
}

unsafe fn show_verbose_terminfo(tui: *mut TUIData) {
    unsafe {
        let mut title_items = [
            cstr_as_obj(c"\n\n--- Terminal info --- {{{\n".as_ptr()),
            cstr_as_obj(c"Title".as_ptr()),
        ];
        let title = Array {
            items: title_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        let str_val = terminfo_info_msg(
            &raw const (*tui).ti,
            (*tui).term,
            (*tui).terminfo_found_in_db,
        );
        let mut info_items = [string_obj(str_val)];
        let info = Array {
            items: info_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        let mut end_fold_items = [
            cstr_as_obj(c"}}}\n".as_ptr()),
            cstr_as_obj(c"Title".as_ptr()),
        ];
        let end_fold = Array {
            items: end_fold_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        let mut chunks_items = [array_obj(title), array_obj(info), array_obj(end_fold)];
        let chunks = Array {
            items: chunks_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        let mut opts_items = [KeyValuePair {
            key: static_cstr_as_string(c"verbose"),
            value: boolean_obj(true),
        }];
        let opts = Dict {
            items: opts_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        let mut args_items = [array_obj(chunks), boolean_obj(true), dict_obj(opts)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        rpc_send_event(ui_client_channel_id, c"nvim_echo".as_ptr(), args);
        xfree(str_val.data.cast());
    }
}

pub unsafe fn tui_suspend(tui: *mut TUIData) {
    #[cfg(unix)]
    unsafe {
        ui_client_detach();
        (*tui).mouse_enabled_save = (*tui).mouse_enabled;
        (*tui).input.callbacks.primary_device_attr = Some(tui_suspend_cb);
        terminfo_disable(tui);
    }
    #[cfg(not(unix))]
    let _ = tui;
}

#[cfg(unix)]
unsafe fn tui_suspend_cb(tui: *mut TUIData) {
    unsafe {
        let input = tui_input(tui);
        tui_terminal_stop(tui);
        stream_set_blocking((*input).in_fd, true);

        let ppid = getppid();
        if getpgid(ppid) == getpgrp() {
            kill(ppid, SIGSTOP);
        }
        kill(0, SIGSTOP);

        tui_terminal_start(tui);
        tui_terminal_after_startup(tui);
        if (*tui).mouse_enabled_save {
            tui_mouse_on(tui);
        }
        stream_set_blocking((*input).in_fd, false);
        ui_client_attach((*tui).width, (*tui).height, (*tui).term, (*tui).rgb);
    }
}

pub unsafe fn tui_set_title(tui: *mut TUIData, title: NvimString) {
    unsafe {
        if !(*tui).can_set_title {
            return;
        }

        let too_long = title.size > 4096;
        if too_long && g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"tui_set_title".as_ptr(),
                1682,
                true,
                c"set_title: title string too long!".as_ptr(),
            );
        }
        if title.size > 0 && !too_long {
            if !(*tui).title_enabled {
                let seq = b"\x1b[22;0t";
                out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
                (*tui).title_enabled = true;
            }

            if ((*tui).buf.len() - (*tui).bufpos) < title.size + 2 * TERMINFO_SEQ_LIMIT {
                flush_buf(tui, FlushBufFinish::kFlushBufFinal);
            }
            terminfo_out(tui, kTerm_to_status_line);
            out(tui, title.data, title.size);
            terminfo_out(tui, kTerm_from_status_line);
        } else if (*tui).title_enabled {
            let seq = b"\x1b[23;0t";
            out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
            (*tui).title_enabled = false;
        }
    }
}

pub unsafe fn tui_set_icon(_tui: *mut TUIData, _icon: NvimString) {}

pub unsafe fn tui_screenshot(tui: *mut TUIData, path: NvimString) {
    unsafe {
        let f = fopen(path.data, c"w".as_ptr());
        if f.is_null() {
            return;
        }

        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
        (*tui).grid.row = 0;
        (*tui).grid.col = 0;

        (*tui).screenshot = f;
        let header = format!("{},{}\n", (*tui).grid.height, (*tui).grid.width);
        fwrite(header.as_ptr().cast(), header.len(), 1, f);
        terminfo_out(tui, kTerm_clear_screen);
        for i in 0..(*tui).grid.height {
            cursor_goto(tui, i, 0);
            for j in 0..(*tui).grid.width {
                let cell = {
                    let cells = &(*tui).grid.cells;
                    cells[i as usize][j as usize]
                };
                let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
                schar_get(buf.as_mut_ptr(), cell.data);
                print_cell(tui, buf.as_ptr(), cell.attr);
            }
        }
        flush_buf(tui, FlushBufFinish::kFlushBufFinal);
        (*tui).screenshot = ptr::null_mut();

        fclose(f);
    }
}

pub unsafe fn tui_option_set(tui: *mut TUIData, name: NvimString, value: Object) {
    unsafe {
        if strequal(name.data, c"mousemoveevent".as_ptr()) {
            if (*tui).mouse_move_enabled != value.data.boolean {
                if (*tui).mouse_enabled {
                    tui_mouse_off(tui);
                    (*tui).mouse_move_enabled = value.data.boolean;
                    tui_mouse_on(tui);
                } else {
                    (*tui).mouse_move_enabled = value.data.boolean;
                }
            }
        } else if strequal(name.data, c"termguicolors".as_ptr()) {
            (*tui).rgb = value.data.boolean;
            (*tui).print_attr_id = -1;
            invalidate(tui, 0, (*tui).grid.height, 0, (*tui).grid.width);

            if ui_client_channel_id != 0 {
                let mut args_items = [
                    cstr_as_obj(c"rgb".as_ptr()),
                    boolean_obj(value.data.boolean),
                ];
                let args = Array {
                    items: args_items.as_mut_ptr(),
                    size: 2,
                    capacity: 2,
                };
                rpc_send_event(ui_client_channel_id, c"nvim_ui_set_option".as_ptr(), args);
            }
        } else if strequal(name.data, c"ttimeout".as_ptr()) {
            (*tui).input.ttimeout = value.data.boolean;
        } else if strequal(name.data, c"ttimeoutlen".as_ptr()) {
            (*tui).input.ttimeoutlen = value.data.integer as OptInt;
        } else if strequal(name.data, c"verbose".as_ptr()) {
            (*tui).verbose = value.data.integer;
        } else if strequal(name.data, c"termsync".as_ptr()) {
            (*tui).sync_output = value.data.boolean;
        }
    }
}

pub unsafe fn tui_chdir(_tui: *mut TUIData, path: NvimString) {
    unsafe {
        let err = uv_chdir(path.data);
        if err != 0 && g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"tui_chdir".as_ptr(),
                1777,
                true,
                c"Failed to chdir to %s: %s".as_ptr(),
                path.data,
                uv_strerror(err),
            );
        }
    }
}

pub unsafe fn tui_raw_line(
    tui: *mut TUIData,
    _g: Integer,
    linerow: Integer,
    startcol: Integer,
    endcol: Integer,
    clearcol: Integer,
    clearattr: Integer,
    flags: LineFlags,
    chunk: *const schar_T,
    attrs: *const sattr_T,
) {
    unsafe {
        {
            let attr_count = (*tui).attrs.len();
            let cells = &mut (*tui).grid.cells;
            let row_cells = &mut cells[linerow as usize];
            for c in startcol..endcol {
                let idx = (c - startcol) as usize;
                let cell = &mut row_cells[c as usize];
                cell.data = *chunk.add(idx);
                assert!((*attrs.add(idx) as usize) < attr_count);
                cell.attr = *attrs.add(idx);
            }
        }
        for curcol in (startcol as c_int)..(endcol as c_int) {
            let (cell, is_double) = {
                let cells = &(*tui).grid.cells;
                let row_cells = &cells[linerow as usize];
                (
                    row_cells[curcol as usize],
                    curcol < (endcol as c_int) - 1 && row_cells[(curcol + 1) as usize].data == 0,
                )
            };
            print_cell_at_pos(tui, linerow as c_int, curcol, cell, is_double);
        }

        if clearcol > endcol {
            ugrid_clear_chunk(
                &mut (*tui).grid,
                linerow as c_int,
                endcol as c_int,
                clearcol as c_int,
                clearattr as sattr_T,
            );
            clear_region(
                tui,
                linerow as c_int,
                (linerow as c_int) + 1,
                endcol as c_int,
                clearcol as c_int,
                clearattr as c_int,
            );
        }

        if (flags & kLineFlagWrap) != 0
            && (*tui).width == (*tui).grid.width
            && (linerow as c_int) + 1 < (*tui).grid.height
        {
            if endcol as c_int != (*tui).grid.width {
                let last_col = ((*tui).grid.width - 1) as usize;
                let (target_col, cell, size) = {
                    let cells = &(*tui).grid.cells;
                    let row_cells = &cells[linerow as usize];
                    let size = if row_cells[last_col].data == 0 { 2 } else { 1 };
                    let target_col = (*tui).grid.width - size;
                    (target_col, row_cells[target_col as usize], size)
                };
                print_cell_at_pos(tui, linerow as c_int, target_col, cell, size == 2);
            }

            final_column_wrap(tui);
        }
    }
}

unsafe fn invalidate(tui: *mut TUIData, top: c_int, bot: c_int, left: c_int, right: c_int) {
    unsafe {
        if let Some(intersects) = (*tui)
            .invalid_regions
            .iter_mut()
            .find(|r| !(top > r.bot || bot < r.top) && !(left > r.right || right < r.left))
        {
            intersects.top = top.min(intersects.top);
            intersects.bot = bot.max(intersects.bot);
            intersects.left = left.min(intersects.left);
            intersects.right = right.max(intersects.right);
        } else {
            (*tui).invalid_regions.push(Rect {
                top,
                bot,
                left,
                right,
            });
        }
    }
}

pub unsafe fn tui_set_size(tui: *mut TUIData, width: c_int, height: c_int) {
    unsafe {
        (*tui).pending_resize_events += 1;
        (*tui).width = width;
        (*tui).height = height;
        ui_client_set_size(width, height);
    }
}

unsafe fn parse_sscanf_int(val: *const c_char, out: &mut c_int) -> bool {
    if val.is_null() {
        return false;
    }
    let bytes = unsafe { CStr::from_ptr(val).to_bytes() };
    let mut idx = 0;
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx += 1;
    }
    if idx >= bytes.len() {
        return false;
    }
    let start = idx;
    if bytes[idx] == b'+' || bytes[idx] == b'-' {
        idx += 1;
    }
    let digit_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == digit_start {
        return false;
    }
    if let Ok(s) = std::str::from_utf8(&bytes[start..idx])
        && let Ok(v) = s.parse::<c_int>()
    {
        *out = v;
        return true;
    }
    false
}

pub unsafe fn tui_guess_size(tui: *mut TUIData) {
    unsafe {
        let mut width: c_int = 0;
        let mut height: c_int = 0;

        let mut got_size = false;
        if (*tui).out_isatty
            && uv_tty_get_winsize(
                tui_output_handle(tui).cast(),
                &raw mut width,
                &raw mut height,
            ) == 0
        {
            got_size = true;
        }

        if !got_size {
            let mut lines_buf = [0 as c_char; 32];
            let mut cols_buf = [0 as c_char; 32];
            let lines = os_getenv_buf(c"LINES".as_ptr(), lines_buf.as_mut_ptr(), lines_buf.len());
            let columns =
                os_getenv_buf(c"COLUMNS".as_ptr(), cols_buf.as_mut_ptr(), cols_buf.len());
            if parse_sscanf_int(lines, &mut height) && parse_sscanf_int(columns, &mut width) {
                got_size = true;
            }
        }

        if !got_size {
            height = (*tui).ti.lines;
            width = (*tui).ti.columns;
        }

        if width <= 0 || height <= 0 {
            width = DFLT_COLS;
            height = DFLT_ROWS;
        }

        tui_set_size(tui, width, height);
    }
}

unsafe fn out(tui: *mut TUIData, str_: *const c_char, len: usize) {
    unsafe {
        let available = (*tui).buf.len() - (*tui).bufpos;
        if len > available {
            flush_buf(tui, FlushBufFinish::kFlushBufPartial);
            if len > (*tui).buf.len() {
                (*tui).buf_to_flush = str_.cast_mut();
                (*tui).bufpos = len;
                flush_buf(tui, FlushBufFinish::kFlushBufPartial);
                return;
            }
        }

        ptr::copy_nonoverlapping(str_, (*tui).buf.as_mut_ptr().add((*tui).bufpos), len);
        (*tui).bufpos += len;
    }
}

unsafe fn out_len(tui: *mut TUIData, str_: *const c_char) {
    unsafe {
        if !str_.is_null() {
            out(tui, str_, CStr::from_ptr(str_).to_bytes().len());
        }
    }
}

unsafe fn out_printf_str(tui: *mut TUIData, limit: usize, formatted: &str) {
    unsafe {
        assert!(limit <= (*tui).buf.len());
        let available = (*tui).buf.len() - (*tui).bufpos;
        if available < limit {
            flush_buf(tui, FlushBufFinish::kFlushBufPartial);
        }
        let bytes = formatted.as_bytes();
        let copy_len = bytes.len().min(limit.saturating_sub(1));
        ptr::copy_nonoverlapping(
            bytes.as_ptr().cast::<c_char>(),
            (*tui).buf.as_mut_ptr().add((*tui).bufpos),
            copy_len,
        );
        if limit > 0 {
            *(*tui).buf.as_mut_ptr().add((*tui).bufpos + copy_len) = 0;
        }
        (*tui).bufpos += bytes.len();
    }
}

unsafe fn terminfo_out(tui: *mut TUIData, what: TerminfoDef) {
    unsafe {
        let mut null_params = [TPVAR {
            num: 0,
            string: ptr::null_mut(),
        }; 9];
        terminfo_print(tui, what, null_params.as_mut_ptr());
    }
}

unsafe fn terminfo_print_num1(tui: *mut TUIData, what: TerminfoDef, num: c_int) {
    unsafe {
        terminfo_print_num(tui, what, num, 0, 0);
    }
}

unsafe fn terminfo_print_num2(tui: *mut TUIData, what: TerminfoDef, num1: c_int, num2: c_int) {
    unsafe {
        terminfo_print_num(tui, what, num1, num2, 0);
    }
}

unsafe fn terminfo_print_num(
    tui: *mut TUIData,
    what: TerminfoDef,
    num1: c_int,
    num2: c_int,
    num3: c_int,
) {
    unsafe {
        let mut params = [TPVAR {
            num: 0,
            string: ptr::null_mut(),
        }; 9];
        params[0].num = c_long::from(num1);
        params[1].num = c_long::from(num2);
        params[2].num = c_long::from(num3);
        terminfo_print(tui, what, params.as_mut_ptr());
    }
}

unsafe fn terminfo_print(tui: *mut TUIData, what: TerminfoDef, params: *mut TPVAR) {
    unsafe {
        if what >= kTermCount {
            std::process::abort();
        }

        let str_ = (*tui).ti.defs[what];
        if str_.is_null() || *str_ == NUL {
            return;
        }

        let buf_start = (*tui).buf.as_mut_ptr();
        let buf_end = buf_start.add((*tui).buf.len());
        if (*tui).buf.len() - (*tui).bufpos > TERMINFO_SEQ_LIMIT {
            let mut copy_params = [TPVAR {
                num: 0,
                string: ptr::null_mut(),
            }; 9];
            ptr::copy_nonoverlapping(params, copy_params.as_mut_ptr(), 9);
            let len = terminfo_fmt(
                buf_start.add((*tui).bufpos),
                buf_end,
                str_,
                copy_params.as_mut_ptr(),
            );
            if len > 0 {
                (*tui).bufpos += len;
                return;
            }
        }

        flush_buf(tui, FlushBufFinish::kFlushBufPartial);
        let len = terminfo_fmt(buf_start.add((*tui).bufpos), buf_end, str_, params);
        if len > 0 {
            (*tui).bufpos += len;
        }
    }
}

unsafe fn terminfo_set_if_empty(tui: *mut TUIData, str_: TerminfoDef, val: *const c_char) {
    unsafe {
        if (*tui).ti.defs[str_].is_null() {
            (*tui).ti.defs[str_] = val;
        }
    }
}

unsafe fn terminfo_set_str(tui: *mut TUIData, str_: TerminfoDef, val: *const c_char) {
    unsafe {
        (*tui).ti.defs[str_] = val;
    }
}

unsafe fn term_has_truecolor(tui: *mut TUIData, colorterm: *const c_char) -> bool {
    unsafe {
        if strequal(colorterm, c"truecolor".as_ptr()) || strequal(colorterm, c"24bit".as_ptr()) {
            return true;
        }

        if (*tui).ti.Tc || (*tui).ti.RGB {
            return true;
        }

        !(*tui).ti.defs[kTerm_set_rgb_foreground].is_null()
            && !(*tui).ti.defs[kTerm_set_rgb_background].is_null()
    }
}

unsafe fn patch_terminfo_bugs(
    tui: *mut TUIData,
    term: *const c_char,
    colorterm: *const c_char,
    vte_version: c_int,
    konsolev: c_int,
    iterm_env: bool,
    nsterm: bool,
) {
    unsafe {
        let xterm_version = os_getenv(c"XTERM_VERSION".as_ptr());
        let xterm = terminfo_is_term_family(term, c"xterm".as_ptr()) || nsterm;
        let hterm = terminfo_is_term_family(term, c"hterm".as_ptr());
        let kitty = terminfo_is_term_family(term, c"xterm-kitty".as_ptr());
        let linuxvt = terminfo_is_term_family(term, c"linux".as_ptr());
        let bsdvt = terminfo_is_bsd_console(term);
        let rxvt = terminfo_is_term_family(term, c"rxvt".as_ptr());
        let teraterm = terminfo_is_term_family(term, c"teraterm".as_ptr());
        let putty = terminfo_is_term_family(term, c"putty".as_ptr());
        let screen = terminfo_is_term_family(term, c"screen".as_ptr());
        let tmux = terminfo_is_term_family(term, c"tmux".as_ptr())
            || os_env_exists(c"TMUX".as_ptr(), true);
        let st = terminfo_is_term_family(term, c"st".as_ptr());
        let gnome = terminfo_is_term_family(term, c"gnome".as_ptr())
            || terminfo_is_term_family(term, c"vte".as_ptr());
        let iterm = terminfo_is_term_family(term, c"iterm".as_ptr())
            || terminfo_is_term_family(term, c"iterm2".as_ptr())
            || terminfo_is_term_family(term, c"iTerm.app".as_ptr())
            || terminfo_is_term_family(term, c"iTerm2.app".as_ptr());
        let alacritty = terminfo_is_term_family(term, c"alacritty".as_ptr());
        let foot = terminfo_is_term_family(term, c"foot".as_ptr());
        let iterm_pretending_xterm = xterm && iterm_env;
        let gnome_pretending_xterm = xterm && cstr_contains(colorterm, b"gnome-terminal");
        let mate_pretending_xterm = xterm && cstr_contains(colorterm, b"mate-terminal");
        let true_xterm = xterm && !xterm_version.is_null() && !bsdvt;
        let cygwin = terminfo_is_term_family(term, c"cygwin".as_ptr());
        let ghostty = terminfo_is_term_family(term, c"xterm-ghostty".as_ptr());

        let mut fix_normal = (*tui).ti.defs[kTerm_cursor_normal];
        if !fix_normal.is_null() {
            let bytes = CStr::from_ptr(fix_normal).to_bytes();
            if bytes.starts_with(b"\x1b[?12l") {
                fix_normal = fix_normal.add(b"\x1b[?12l".len());
                terminfo_set_str(tui, kTerm_cursor_normal, fix_normal);
            }
            let bytes = CStr::from_ptr(fix_normal).to_bytes();
            if linuxvt && bytes.ends_with(LINUXSET0C) {
                let new_normal = arena_memdupz(
                    &raw mut (*tui).ti_arena,
                    fix_normal,
                    bytes.len() - LINUXSET0C.len(),
                );
                terminfo_set_str(tui, kTerm_cursor_normal, new_normal);
            }
        }
        let fix_invisible = (*tui).ti.defs[kTerm_cursor_invisible];
        if !fix_invisible.is_null() {
            let bytes = CStr::from_ptr(fix_invisible).to_bytes();
            if linuxvt && bytes.ends_with(LINUXSET1C) {
                let new_invisible = arena_memdupz(
                    &raw mut (*tui).ti_arena,
                    fix_invisible,
                    bytes.len() - LINUXSET1C.len(),
                );
                terminfo_set_str(tui, kTerm_cursor_invisible, new_invisible);
            }
        }

        if tmux || screen || kitty {
            (*tui).ti.back_color_erase = false;
        }

        if xterm || hterm {
            if !hterm {
                terminfo_set_if_empty(tui, kTerm_to_status_line, c"\x1b]0;".as_ptr());
                terminfo_set_if_empty(tui, kTerm_from_status_line, c"\x07".as_ptr());
            }
            terminfo_set_if_empty(tui, kTerm_enter_italics_mode, c"\x1b[3m".as_ptr());
            terminfo_set_if_empty(tui, kTerm_set_lr_margin, c"\x1b[%i%p1%d;%p2%ds".as_ptr());
            #[cfg(windows)]
            {
                terminfo_set_str(tui, kTerm_cursor_down, c"\x1b[B".as_ptr());
            }
        } else if rxvt {
            terminfo_set_if_empty(tui, kTerm_enter_italics_mode, c"\x1b[3m".as_ptr());
            terminfo_set_if_empty(tui, kTerm_to_status_line, c"\x1b]2".as_ptr());
            terminfo_set_if_empty(tui, kTerm_from_status_line, c"\x07".as_ptr());
            terminfo_set_str(tui, kTerm_enter_ca_mode, c"\x1b[?1049h".as_ptr());
            terminfo_set_str(tui, kTerm_exit_ca_mode, c"\x1b[?1049l".as_ptr());
            terminfo_set_str(tui, kTerm_reset_cursor_style, c"\x1b[2 q".as_ptr());
            terminfo_set_str(tui, kTerm_set_cursor_style, c"\x1b[%p1%d q".as_ptr());
        } else if screen {
            terminfo_set_if_empty(tui, kTerm_to_status_line, c"\x1b_".as_ptr());
            terminfo_set_if_empty(tui, kTerm_from_status_line, c"\x1b\\".as_ptr());
        } else if tmux {
            terminfo_set_if_empty(tui, kTerm_to_status_line, c"\x1b_".as_ptr());
            terminfo_set_if_empty(tui, kTerm_from_status_line, c"\x1b\\".as_ptr());
            terminfo_set_if_empty(tui, kTerm_enter_italics_mode, c"\x1b[3m".as_ptr());
        } else if terminfo_is_term_family(term, c"interix".as_ptr()) {
            terminfo_set_if_empty(tui, kTerm_carriage_return, c"\x0d".as_ptr());
        } else if linuxvt {
            terminfo_set_if_empty(tui, kTerm_parm_up_cursor, c"\x1b[%p1%dA".as_ptr());
            terminfo_set_if_empty(tui, kTerm_parm_down_cursor, c"\x1b[%p1%dB".as_ptr());
            terminfo_set_if_empty(tui, kTerm_parm_right_cursor, c"\x1b[%p1%dC".as_ptr());
            terminfo_set_if_empty(tui, kTerm_parm_left_cursor, c"\x1b[%p1%dD".as_ptr());
        } else if putty {
            // No bugs in the vanilla terminfo for our purposes.
        } else if iterm {
            terminfo_set_str(tui, kTerm_enter_ca_mode, c"\x1b[?1049h".as_ptr());
            terminfo_set_str(tui, kTerm_exit_ca_mode, c"\x1b[?1049l".as_ptr());
            terminfo_set_if_empty(tui, kTerm_enter_italics_mode, c"\x1b[3m".as_ptr());
        } else if st {
            // No bugs in the vanilla terminfo for our purposes.
        }

        if (*tui).ti.max_colors < 256 {
            if true_xterm || iterm || iterm_pretending_xterm {
                (*tui).ti.max_colors = 256;
                terminfo_set_str(tui, kTerm_set_a_foreground, XTERM_SETAF_256_COLON);
                terminfo_set_str(tui, kTerm_set_a_background, XTERM_SETAB_256_COLON);
            } else if konsolev != 0
                || xterm
                || gnome
                || rxvt
                || st
                || putty
                || linuxvt
                || mate_pretending_xterm
                || gnome_pretending_xterm
                || tmux
                || cstr_contains(colorterm, b"256")
                || cstr_contains(term, b"256")
            {
                (*tui).ti.max_colors = 256;
                terminfo_set_str(tui, kTerm_set_a_foreground, XTERM_SETAF_256);
                terminfo_set_str(tui, kTerm_set_a_background, XTERM_SETAB_256);
            }
        }
        if (*tui).ti.max_colors < 16 && !colorterm.is_null() {
            (*tui).ti.max_colors = 16;
            terminfo_set_if_empty(tui, kTerm_set_a_foreground, XTERM_SETAF_16);
            terminfo_set_if_empty(tui, kTerm_set_a_background, XTERM_SETAB_16);
        }

        if st || (vte_version != 0 && vte_version < 3900) || konsolev != 0 {
            (*tui).ti.defs[kTerm_reset_cursor_style] = ptr::null();
        }

        if !bsdvt
            && (putty
                || hterm
                || vte_version != 0
                || konsolev != 0
                || tmux
                || screen
                || st
                || iterm
                || iterm_pretending_xterm
                || teraterm
                || alacritty
                || cygwin
                || foot
                || kitty
                || ghostty
                || (linuxvt && (!xterm_version.is_null() || !colorterm.is_null())))
        {
            terminfo_set_str(tui, kTerm_set_cursor_style, c"\x1b[%p1%d q".as_ptr());
            terminfo_set_str(tui, kTerm_reset_cursor_style, c"\x1b[0 q".as_ptr());
        } else if linuxvt {
            terminfo_set_str(
                tui,
                kTerm_set_cursor_style,
                c"\x1b[?%?%p1%{2}%<%t%{8}%e%p1%{2}%=%t%{112}%e%p1%{3}%=%t%{4}%e%p1%{4}%=%t%{4}%e%p1%{5}%=%t%{2}%e%p1%{6}%=%t%{2}%e%{0}%;%dc".as_ptr(),
            );
            terminfo_set_str(tui, kTerm_reset_cursor_style, c"\x1b[?c".as_ptr());
        } else if !bsdvt && xterm {
            terminfo_set_if_empty(tui, kTerm_set_cursor_style, c"\x1b[%p1%d q".as_ptr());
            terminfo_set_if_empty(tui, kTerm_reset_cursor_style, c"\x1b[2 q".as_ptr());
        }

        xfree(xterm_version.cast());
    }
}

unsafe fn augment_terminfo(
    tui: *mut TUIData,
    term: *const c_char,
    vte_version: c_int,
    konsolev: c_int,
    weztermv: *const c_char,
    iterm_env: bool,
    nsterm: bool,
) {
    unsafe {
        let xterm_version = os_getenv(c"XTERM_VERSION".as_ptr());
        let xterm = terminfo_is_term_family(term, c"xterm".as_ptr()) || nsterm;
        let hterm = terminfo_is_term_family(term, c"hterm".as_ptr());
        let bsdvt = terminfo_is_bsd_console(term);
        let dtterm = terminfo_is_term_family(term, c"dtterm".as_ptr());
        let rxvt = terminfo_is_term_family(term, c"rxvt".as_ptr());
        let teraterm = terminfo_is_term_family(term, c"teraterm".as_ptr());
        let putty = terminfo_is_term_family(term, c"putty".as_ptr());
        let screen = terminfo_is_term_family(term, c"screen".as_ptr());
        let tmux = terminfo_is_term_family(term, c"tmux".as_ptr())
            || os_env_exists(c"TMUX".as_ptr(), true);
        let st = terminfo_is_term_family(term, c"st".as_ptr());
        let iterm = terminfo_is_term_family(term, c"iterm".as_ptr())
            || terminfo_is_term_family(term, c"iterm2".as_ptr())
            || terminfo_is_term_family(term, c"iTerm.app".as_ptr())
            || terminfo_is_term_family(term, c"iTerm2.app".as_ptr());
        let alacritty = terminfo_is_term_family(term, c"alacritty".as_ptr());
        let kitty = terminfo_is_term_family(term, c"xterm-kitty".as_ptr());
        let iterm_pretending_xterm = xterm && iterm_env;
        let true_xterm = xterm && !xterm_version.is_null() && !bsdvt;

        if dtterm || xterm || konsolev != 0 || teraterm || rxvt {
            (*tui).can_resize_screen = true;
        }

        if putty || xterm || hterm || rxvt {
            (*tui).terminfo_ext.reset_scroll_region = c"\x1b[r".as_ptr();
        }

        (*tui).terminfo_ext.enter_altfont_mode = c"\x1b[11m".as_ptr();

        let has_colon_rgb =
            !tmux && !screen && vte_version == 0 && (iterm || iterm_pretending_xterm || true_xterm);

        if (*tui).ti.defs[kTerm_set_rgb_foreground].is_null() {
            (*tui).ti.defs[kTerm_set_rgb_foreground] = if has_colon_rgb {
                c"\x1b[38:2:%p1%d:%p2%d:%p3%dm".as_ptr()
            } else {
                c"\x1b[38;2;%p1%d;%p2%d;%p3%dm".as_ptr()
            };
        }
        if (*tui).ti.defs[kTerm_set_rgb_background].is_null() {
            (*tui).ti.defs[kTerm_set_rgb_background] = if has_colon_rgb {
                c"\x1b[48:2:%p1%d:%p2%d:%p3%dm".as_ptr()
            } else {
                c"\x1b[48;2;%p1%d;%p2%d;%p3%dm".as_ptr()
            };
        }

        if (*tui).ti.defs[kTerm_set_cursor_color].is_null() {
            if iterm || iterm_pretending_xterm {
                (*tui).ti.defs[kTerm_set_cursor_color] = if tmux {
                    c"\x1bPtmux;\x1b\x1b]Pl%p1%06x\x1b\\\x1b\\".as_ptr()
                } else {
                    c"\x1b]Pl%p1%06x\x1b\\".as_ptr()
                };
            } else if (xterm || hterm || rxvt || tmux || alacritty || st)
                && (vte_version == 0 || vte_version >= 3900)
            {
                (*tui).ti.defs[kTerm_set_cursor_color] = c"\x1b]12;%p1%s\x07".as_ptr();
            }
        }
        if !(*tui).ti.defs[kTerm_set_cursor_color].is_null() {
            (*tui).set_cursor_color_as_str =
                cstr_contains((*tui).ti.defs[kTerm_set_cursor_color], b"%s");
            terminfo_set_if_empty(tui, kTerm_reset_cursor_color, c"\x1b]112\x07".as_ptr());
        }

        if !(*tui).ti.defs[kTerm_to_status_line].is_null()
            && !(*tui).ti.defs[kTerm_from_status_line].is_null()
        {
            (*tui).can_set_title = true;
        }

        (*tui).terminfo_ext.enable_focus_reporting = if rxvt {
            c"\x1b[?1004h\x1b]777;focus;on\x07".as_ptr()
        } else {
            c"\x1b[?1004h".as_ptr()
        };
        (*tui).terminfo_ext.disable_focus_reporting = if rxvt {
            c"\x1b[?1004l\x1b]777;focus;off\x07".as_ptr()
        } else {
            c"\x1b[?1004l".as_ptr()
        };

        if (*tui).ti.defs[kTerm_set_underline_style].is_null() {
            let wezterm_ok = !weztermv.is_null()
                && CStr::from_ptr(weztermv).to_bytes() > &b"20210203-095643"[..];
            if vte_version >= 5102 || konsolev >= 221170 || (*tui).ti.Su || wezterm_ok {
                tui_enable_extended_underline(tui);
            }
        } else {
            tui_enable_extended_underline(tui);
        }

        if kitty || (vte_version != 0 && vte_version < 5400) {
            (*tui).input.key_encoding = kKeyEncodingLegacy;
        } else {
            (*tui).input.key_encoding = kKeyEncodingXterm;
        }

        xfree(xterm_version.cast());
    }
}

#[inline]
unsafe fn should_invisible(tui: *mut TUIData) -> bool {
    unsafe { (*tui).busy || (*tui).want_invisible }
}

unsafe fn flush_buf(tui: *mut TUIData, finish: FlushBufFinish) {
    unsafe {
        let should_be_invisible = should_invisible(tui);

        if (*tui).bufpos == 0
            && (*tui).is_invisible == should_be_invisible
            && !(finish == FlushBufFinish::kFlushBufFinal && (*tui).sync_output_active)
        {
            return;
        }

        let mut req = [0u64; 64];
        let mut pre = [0 as c_char; 32];
        let mut post = [0 as c_char; 32];
        let mut pre_len = 0usize;
        let mut post_len = 0usize;
        let mut null_params = [TPVAR {
            num: 0,
            string: ptr::null_mut(),
        }; 9];

        if !(*tui).sync_output_active && (*tui).sync_output && (*tui).has_sync_mode {
            let sync_start = b"\x1b[?2026h\0";
            ptr::copy_nonoverlapping(
                sync_start.as_ptr().cast::<c_char>(),
                pre.as_mut_ptr(),
                sync_start.len(),
            );
            pre_len += sync_start.len() - 1;
            (*tui).sync_output_active = true;
        } else if !(*tui).sync_output_active && !(*tui).is_invisible {
            (*tui).is_invisible = true;
            let str_ = (*tui).ti.defs[kTerm_cursor_invisible];
            if !str_.is_null() {
                pre_len += terminfo_fmt(
                    pre.as_mut_ptr().add(pre_len),
                    pre.as_mut_ptr().add(pre.len()),
                    str_,
                    null_params.as_mut_ptr(),
                );
            }
        }

        if finish == FlushBufFinish::kFlushBufFinal && (*tui).sync_output_active {
            let sync_end = b"\x1b[?2026l\0";
            ptr::copy_nonoverlapping(
                sync_end.as_ptr().cast::<c_char>(),
                post.as_mut_ptr(),
                sync_end.len(),
            );
            post_len += sync_end.len() - 1;
            (*tui).sync_output_active = false;
        }

        if !(*tui).sync_output_active {
            let mut str_: *const c_char = ptr::null();
            if (*tui).is_invisible && !should_be_invisible {
                str_ = (*tui).ti.defs[kTerm_cursor_normal];
                (*tui).is_invisible = false;
            } else if !(*tui).is_invisible && should_be_invisible {
                str_ = (*tui).ti.defs[kTerm_cursor_invisible];
                (*tui).is_invisible = true;
            }
            if !str_.is_null() {
                post_len += terminfo_fmt(
                    post.as_mut_ptr().add(post_len),
                    post.as_mut_ptr().add(post.len()),
                    str_,
                    null_params.as_mut_ptr(),
                );
            }
        }

        let mid_base = if !(*tui).buf_to_flush.is_null() {
            (*tui).buf_to_flush
        } else {
            (*tui).buf.as_mut_ptr()
        };
        let bufs = [
            uv_buf_t::new(pre.as_mut_ptr(), pre_len),
            uv_buf_t::new(mid_base, (*tui).bufpos),
            uv_buf_t::new(post.as_mut_ptr(), post_len),
        ];

        if !(*tui).screenshot.is_null() {
            for b in &bufs {
                fwrite(b.base.cast(), b.len as _, 1, (*tui).screenshot);
            }
        } else {
            let mut nbufs = bufs.len();
            while nbufs > 0 && bufs[nbufs - 1].len == 0 {
                nbufs -= 1;
            }
            if nbufs > 0 {
                let ret = uv_write(
                    req.as_mut_ptr().cast(),
                    tui_output_handle(tui).cast(),
                    bufs.as_ptr(),
                    nbufs as c_uint,
                    None,
                );
                if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"flush_buf".as_ptr(),
                        2660,
                        true,
                        c"uv_write failed: %s".as_ptr(),
                        uv_strerror(ret),
                    );
                }
                uv_run(tui_write_loop(tui), UV_RUN_DEFAULT);
            }
        }
        (*tui).buf_to_flush = ptr::null_mut();
        (*tui).bufpos = 0;
    }
}

unsafe fn tui_get_stty_erase(input: *mut TermInput) -> *const c_char {
    unsafe {
        #[cfg(unix)]
        {
            let mut t: unix_termios::termios = std::mem::zeroed();
            if unix_termios::tcgetattr((*input).in_fd, &raw mut t) != -1 {
                stty_erase_buf[0] = t.c_cc[unix_termios::VERASE] as c_char;
                stty_erase_buf[1] = NUL;
                if g_min_log_level <= LOGLVL_DBG {
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"tui_get_stty_erase".as_ptr(),
                        2684,
                        true,
                        c"stty/termios:erase=%s".as_ptr(),
                        (&raw const stty_erase_buf).cast::<c_char>(),
                    );
                }
            }
        }
        #[cfg(windows)]
        {
            let uv_ptr: *mut c_void = (&raw mut (*input).read_stream.s.uv).cast();
            let mut dw_mode: u32 = 0;
            if uv_handle_get_type(uv_ptr.cast()) == UV_TTY {
                let tty_handle = *uv_ptr
                    .cast::<u8>()
                    .add(uv_handle_size(UV_STREAM))
                    .cast::<*mut c_void>();
                if GetConsoleMode(tty_handle, &raw mut dw_mode) != 0
                    && (dw_mode & ENABLE_VIRTUAL_TERMINAL_INPUT) != 0
                {
                    stty_erase_buf[0] = 0x7f;
                    stty_erase_buf[1] = NUL;
                }
            }
        }
        (&raw const stty_erase_buf).cast::<c_char>()
    }
}

unsafe fn tui_tk_ti_getstr(
    name: *const c_char,
    value: *const c_char,
    input: *mut TermInput,
) -> *const c_char {
    unsafe {
        if stty_erase_ptr.is_null() {
            stty_erase_ptr = tui_get_stty_erase(input);
        }

        if strequal(name, c"key_backspace".as_ptr()) {
            if g_min_log_level <= LOGLVL_DBG {
                logmsg!(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"tui_tk_ti_getstr".as_ptr(),
                    2709,
                    true,
                    c"libtermkey:kbs=%s".as_ptr(),
                    value,
                );
            }
            if *stty_erase_ptr != 0 {
                return stty_erase_ptr;
            }
        } else if strequal(name, c"key_dc".as_ptr()) {
            if g_min_log_level <= LOGLVL_DBG {
                logmsg!(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"tui_tk_ti_getstr".as_ptr(),
                    2714,
                    true,
                    c"libtermkey:kdch1=%s".as_ptr(),
                    value,
                );
            }
            if !value.is_null()
                && value != usize::MAX as *const c_char
                && strequal(stty_erase_ptr, value)
            {
                return if *stty_erase_ptr == DEL {
                    c"\x08".as_ptr()
                } else {
                    c"\x7f".as_ptr()
                };
            }
        } else if strequal(name, c"key_mouse".as_ptr()) {
            if g_min_log_level <= LOGLVL_DBG {
                logmsg!(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"tui_tk_ti_getstr".as_ptr(),
                    2720,
                    true,
                    c"libtermkey:kmous=%s".as_ptr(),
                    value,
                );
            }
            return ptr::null();
        }

        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn termdef_lookup_and_sscanf_int_work() {
        assert!(matches!(
            lookup_termdef_field(b"back_color_erase"),
            Some(TermdefFieldTarget::BoolBackColorErase)
        ));
        assert!(matches!(
            lookup_termdef_field(b"max_colors"),
            Some(TermdefFieldTarget::IntMaxColors)
        ));
        assert!(matches!(
            lookup_termdef_field(b"enter_ca_mode"),
            Some(TermdefFieldTarget::Def(kTerm_enter_ca_mode))
        ));
        assert!(matches!(
            lookup_termdef_field(b"key_home"),
            Some(TermdefFieldTarget::KeyArr(kTermKey_home))
        ));
        assert!(matches!(
            lookup_termdef_field(b"key_f63"),
            Some(TermdefFieldTarget::FKey(62))
        ));
        assert!(lookup_termdef_field(b"key_f64").is_none());

        let mut v = 0;
        unsafe {
            assert!(parse_sscanf_int(c"  120cols".as_ptr(), &mut v));
            assert_eq!(v, 120);
            assert!(!parse_sscanf_int(c"abc".as_ptr(), &mut v));
        }
    }
}
