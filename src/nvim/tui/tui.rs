//! Terminal UI functions invoked by the UI client process (`src/nvim/tui/tui.c`).

use crate::api::private::helpers::{
    api_clear_error, api_free_object, arena_array, array_obj, boolean_obj, cstr_as_obj, dict_obj,
    static_cstr_as_obj, static_cstr_as_string, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, KeyValuePair, NvimString, Object, ObjectType,
};
use crate::buffer_updates::ARENA_EMPTY;
use crate::charset::getdigits_int;
use crate::cursor_shape::{
    CursorShape, Cursorentry, SHAPE_IDX_COUNT, SHAPE_IDX_N, shape_table_mut,
};
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
use crate::fileio::{fopen, fwrite};
use crate::globals::{DFLT_COLS, DFLT_ROWS, *};
use crate::grid::{Sattr, Schar, schar_cache_clear_if_full, schar_from_ascii, schar_get_ascii};
use crate::highlight::{HLATTRS_INIT, HlAttrs};
use crate::log::{LOGLVL_DBG, LOGLVL_ERR, LOGLVL_WRN, log_st, logmsg};
use crate::lua::executor::{K_RET_OBJECT, nlua_exec};
use crate::mbyte::{utf_ambiguous_width, utf_char2cells, utf_ptr2char};
use crate::memory::{Arena, arena_finish, arena_mem_free, arena_memdupz};
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::option::OptInt;
#[cfg(windows)]
use crate::os::env::os_setenv;
use crate::os::env::{os_env_exists, os_getenv_cstr};
#[cfg(not(windows))]
use crate::os::fs::UV_EINTR;
use crate::os::input::os_isatty;
use crate::os::time::os_hrtime;
use crate::tui::input::{
    K_KEY_ENCODING_KITTY, K_KEY_ENCODING_LEGACY, K_KEY_ENCODING_XTERM, PrimaryDeviceAttrAction,
    TermInput, tinput_destroy, tinput_init, tinput_start, tinput_stop,
};
use crate::tui::termkey::driver_ti::{K_TERM_COUNT, K_TERMINFO_FUNC_KEY_MAX, TerminfoEntry};
use crate::tui::ugrid::{
    UCell, UGrid, ugrid_clear, ugrid_clear_chunk, ugrid_goto, ugrid_resize, ugrid_scroll,
};
use crate::ui_client::ui_client_set_size;
#[cfg(unix)]
use crate::ui_client::{ui_client_attach, ui_client_detach};
use std::ffi::{CStr, c_char};
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::{GetConsoleMode, HANDLE, os_tty_guess_term};
use crate::tui::terminfo::*;

use crate::msgpack_rpc::packer::Integer;
pub type TermMode = i32;
pub type TermModeState = i32;
pub type ModeShape = i32;
pub type LineFlags = i32;
pub type TerminfoDef = usize;

pub const K_TERM_MODE_LEFT_AND_RIGHT_MARGINS: TermMode = 69;
pub const K_TERM_MODE_MOUSE_BUTTON_EVENT: TermMode = 1002;
pub const K_TERM_MODE_MOUSE_ANY_EVENT: TermMode = 1003;
pub const K_TERM_MODE_MOUSE_SGREXT: TermMode = 1006;
pub const K_TERM_MODE_BRACKETED_PASTE: TermMode = 2004;
pub const K_TERM_MODE_SYNCHRONIZED_OUTPUT: TermMode = 2026;
pub const K_TERM_MODE_GRAPHEME_CLUSTERS: TermMode = 2027;
pub const K_TERM_MODE_THEME_UPDATES: TermMode = 2031;
pub const K_TERM_MODE_RESIZE_EVENTS: TermMode = 2048;

pub const K_TERM_MODE_NOT_RECOGNIZED: TermModeState = 0;
pub const K_TERM_MODE_SET: TermModeState = 1;
pub const K_TERM_MODE_RESET: TermModeState = 2;
pub const K_TERM_MODE_PERMANENTLY_SET: TermModeState = 3;
pub const K_TERM_MODE_PERMANENTLY_RESET: TermModeState = 4;

const EXIT_TIMEOUT_MS: i64 = 1000;
const OUTBUF_SIZE: usize = 0xffff;
const TOO_MANY_EVENTS: usize = 1_000_000;
const TERMINFO_SEQ_LIMIT: usize = 128;
const DEL: c_char = 0x7f;

#[cfg(unix)]
use crate::event::r#loop::UV_TTY_MODE_NORMAL;
#[cfg(unix)]
const UV_TTY_MODE_IO: i32 = 2;
#[cfg(target_os = "linux")]
const SIGSTOP: i32 = 19;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGSTOP: i32 = 17;

const LINUXSET0C: &[u8] = b"\x1b[?0c";
const LINUXSET1C: &[u8] = b"\x1b[?1c";

const XTERM_SETAF_256_COLON: &CStr =
    c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38:5:%p1%d%;m";
const XTERM_SETAB_256_COLON: &CStr =
    c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48:5:%p1%d%;m";
const XTERM_SETAF_256: &CStr = c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m";
const XTERM_SETAB_256: &CStr =
    c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m";
const XTERM_SETAF_16: &CStr = c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e39%;m";
const XTERM_SETAB_16: &CStr = c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e39%;m";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub top: i32,
    pub bot: i32,
    pub left: i32,
    pub right: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TerminfoExt {
    pub enable_focus_reporting: Option<&'static [u8]>,
    pub disable_focus_reporting: Option<&'static [u8]>,
    pub reset_scroll_region: Option<&'static [u8]>,
    pub enter_altfont_mode: Option<&'static [u8]>,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FlushBufFinish {
    FlushBufPartial = 0,
    FlushBufFinal = 1,
}

#[repr(C)]
pub struct TUIData {
    pub loop_: *mut Loop,
    pub buf: [u8; OUTBUF_SIZE],
    pub bufpos: usize,
    pub input: TermInput,
    pub write_loop: [u64; 256],
    pub ti: TerminfoEntry,
    pub term: Option<std::ffi::CString>,
    pub output_handle: [u64; 80],
    pub out_isatty: bool,
    pub winch_handle: SignalWatcher,
    pub startup_delay_timer: [u64; 32],
    pub grid: UGrid,
    pub invalid_regions: Vec<Rect>,
    pub row: i32,
    pub col: i32,
    pub out_fd: i32,
    pub pending_resize_events: i32,
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
    pub screenshot: Option<crate::os::fs::CFile>,
    pub cursor_shapes: [Cursorentry; SHAPE_IDX_COUNT],
    pub clear_attrs: HlAttrs,
    pub attrs: Vec<HlAttrs>,
    pub print_attr_id: i32,
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
    pub width: i32,
    pub height: i32,
    pub rgb: bool,
    pub screen_or_tmux: bool,
    pub url: i32,
}

impl TUIData {
    pub fn new() -> Self {
        Self {
            loop_: (&raw mut g_st().main_loop).cast::<Loop>(),
            buf: [0; OUTBUF_SIZE],
            bufpos: 0,
            input: TermInput::new(),
            write_loop: [0; 256],
            ti: TerminfoEntry::new(),
            term: None,
            output_handle: [0; 80],
            out_isatty: false,
            winch_handle: SignalWatcher::new(),
            startup_delay_timer: [0; 32],
            grid: UGrid::default(),
            invalid_regions: Vec::new(),
            row: 0,
            col: 0,
            out_fd: 0,
            pending_resize_events: 0,
            terminfo_found_in_db: false,
            can_change_scroll_region: false,
            has_left_and_right_margin_mode: false,
            has_sync_mode: false,
            can_set_lr_margin: false,
            can_scroll: false,
            can_erase_chars: false,
            immediate_wrap_after_last_column: false,
            bce: false,
            mouse_enabled: false,
            mouse_move_enabled: false,
            mouse_enabled_save: false,
            title_enabled: false,
            sync_output: false,
            sync_output_active: false,
            busy: false,
            is_invisible: false,
            want_invisible: false,
            set_cursor_color_as_str: false,
            cursor_has_color: false,
            is_starting: true,
            resize_events_enabled: false,
            modes_grapheme_clusters: false,
            modes_theme_updates: false,
            modes_resize_events: false,
            screenshot: None,
            cursor_shapes: [Cursorentry {
                full_name: c"",
                shape: CursorShape::ShapeBlock,
                mshape: 0,
                percentage: 0,
                blinkwait: 0,
                blinkon: 0,
                blinkoff: 0,
                id: 0,
                id_lm: 0,
                name: c"",
                used_for: 0,
            }; SHAPE_IDX_COUNT],
            clear_attrs: HLATTRS_INIT,
            attrs: vec![HLATTRS_INIT],
            print_attr_id: 0,
            default_attr: false,
            set_default_colors: false,
            can_clear_attr: false,
            showing_mode: 0,
            verbose: 0,
            terminfo_ext: TerminfoExt::default(),
            can_set_title: false,
            can_set_underline_color: false,
            can_resize_screen: false,
            stopped: false,
            width: 0,
            height: 0,
            rgb: false,
            screen_or_tmux: false,
            url: -1,
        }
    }
}

struct TuiModuleState {
    cursor_style_enabled: bool,
    stty_erase_buf: [c_char; 2],
    stty_erase_ptr: *const c_char,
    urls: Vec<std::ffi::CString>,
}

impl TuiModuleState {
    const fn new() -> Self {
        Self {
            cursor_style_enabled: false,
            stty_erase_buf: [0, 0],
            stty_erase_ptr: ptr::null(),
            urls: Vec::new(),
        }
    }
}

static TUI_MOD_STATE: GlobalState<TuiModuleState> = GlobalState::new(TuiModuleState::new());

#[inline]
fn tui_st() -> &'static mut TuiModuleState {
    unsafe { &mut *TUI_MOD_STATE.as_ptr() }
}

#[cfg(unix)]
use crate::fileio::{getpgid, getpgrp, getppid, kill};

use crate::event::r#loop::{uv_tty_get_winsize, uv_tty_reset_mode};

use crate::api::ui::{K_LINE_FLAG_WRAP, MAX_SCHAR_SIZE};
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
fn tui_input(tui: &mut TUIData) -> &'static mut TermInput {
    let ptr: *mut TermInput = &raw mut tui.input;
    unsafe { &mut *ptr }
}

#[inline]
fn tui_write_loop(tui: &mut TUIData) -> *mut uv_loop_t {
    tui.write_loop.as_mut_ptr().cast::<uv_loop_t>()
}

#[inline]
fn tui_output_handle(tui: &mut TUIData) -> *mut uv_handle_t {
    tui.output_handle.as_mut_ptr().cast::<uv_handle_t>()
}

#[inline]
fn tui_winch_handle(tui: &mut TUIData) -> &'static mut SignalWatcher {
    let ptr: *mut SignalWatcher = &raw mut tui.winch_handle;
    unsafe { &mut *ptr }
}

#[inline]
fn tui_startup_delay_timer(tui: &mut TUIData) -> *mut uv_timer_t {
    tui.startup_delay_timer.as_mut_ptr().cast::<uv_timer_t>()
}

#[inline]
fn slice_contains(haystack: Option<&[u8]>, needle: &[u8]) -> bool {
    haystack.is_some_and(|bytes| bytes.windows(needle.len()).any(|w| w == needle))
}

pub fn tui_start(
    tui_out: *mut Option<Box<TUIData>>,
    width: &mut i32,
    height: &mut i32,
    term: *mut Option<std::ffi::CString>,
    rgb: &mut bool,
) {
    unsafe {
        *tui_out = Some(Box::new(TUIData::new()));
        let tui: &mut TUIData = (*tui_out).as_mut().unwrap();

        signal_watcher_init(&mut *tui.loop_, tui_winch_handle(tui), &raw mut *tui);
        signal_watcher_start(&mut *tui_winch_handle(tui), SIGWINCH);

        tui.grid.cells.clear();
        tui_terminal_start(tui);

        let timer = tui_startup_delay_timer(tui);
        uv_timer_init(tui.loop_.cast(), timer);
        *timer.cast::<*mut TUIData>() = &raw mut *tui;
        uv_timer_start(timer, Some(after_startup_cb), 100, 0);

        loop_poll_events(&mut g_st().main_loop, 1);
        *width = tui.width;
        *height = tui.height;
        *term = tui.term.clone();
        *rgb = tui.rgb;
    }
}

fn tui_request_term_mode(tui: &mut TUIData, mode: TermMode) {
    let s = format!("\x1b[?{mode}$p");
    assert!(!s.is_empty() && s.len() < 12);
    out(tui, s.as_bytes());
}

fn tui_set_term_mode(tui: &mut TUIData, mode: TermMode, set: bool) {
    let ch = if set { 'h' } else { 'l' };
    let s = format!("\x1b[?{mode}{ch}");
    assert!(!s.is_empty() && s.len() < 12);
    out(tui, s.as_bytes());
}

pub fn tui_handle_term_mode(tui: &mut TUIData, mode: TermMode, state: TermModeState) {
    let mut is_set = false;
    match state {
        K_TERM_MODE_NOT_RECOGNIZED | K_TERM_MODE_PERMANENTLY_RESET => {
            if !g_st().nvim_testing && log_st().g_min_log_level <= LOGLVL_WRN {
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"tui_handle_term_mode",
                    236,
                    true,
                    c"TUI: terminal mode %d unavailable, state %d",
                    mode,
                    state,
                );
            }
        }
        K_TERM_MODE_PERMANENTLY_SET | K_TERM_MODE_SET | K_TERM_MODE_RESET => {
            if state == K_TERM_MODE_PERMANENTLY_SET || state == K_TERM_MODE_SET {
                is_set = true;
            }
            if !g_st().nvim_testing && log_st().g_min_log_level <= LOGLVL_WRN {
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"tui_handle_term_mode",
                    249,
                    true,
                    c"TUI: terminal mode %d detected, state %d",
                    mode,
                    state,
                );
            }

            match mode {
                K_TERM_MODE_SYNCHRONIZED_OUTPUT => {
                    tui.has_sync_mode = true;
                }
                K_TERM_MODE_GRAPHEME_CLUSTERS => {
                    if !is_set {
                        tui_set_term_mode(tui, mode, true);
                        tui.modes_grapheme_clusters = true;
                    }
                }
                K_TERM_MODE_THEME_UPDATES => {
                    if !is_set {
                        tui_set_term_mode(tui, mode, true);
                        tui.modes_theme_updates = true;
                    }
                }
                K_TERM_MODE_RESIZE_EVENTS => {
                    if !is_set {
                        tui_set_term_mode(tui, mode, true);
                        tui.modes_resize_events = true;
                    }
                    tui.resize_events_enabled = true;
                }
                K_TERM_MODE_LEFT_AND_RIGHT_MARGINS => {
                    tui.has_left_and_right_margin_mode = true;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn tui_query_extended_underline(tui: &mut TUIData) {
    out(tui, b"\x1b[0m\x1b[4:3m\x1bP$qm\x1b\\");
    tui.print_attr_id = -1;
}

pub fn tui_enable_extended_underline(tui: &mut TUIData) {
    terminfo_set_if_empty(tui, K_TERM_SET_UNDERLINE_STYLE, c"\x1b[4:%p1%dm");
    tui.can_set_underline_color = true;
}

pub fn tui_handle_primary_device_attr(tui: &mut TUIData, action: PrimaryDeviceAttrAction) {
    match action {
        PrimaryDeviceAttrAction::SetKeyEncoding => tui_set_key_encoding(tui),
        PrimaryDeviceAttrAction::Stop => tui_stop_cb(tui),
        #[cfg(unix)]
        PrimaryDeviceAttrAction::Suspend => tui_suspend_cb(tui),
    }
}

fn tui_query_kitty_keyboard(tui: &mut TUIData) {
    tui.input.primary_device_attr = Some(PrimaryDeviceAttrAction::SetKeyEncoding);
    out(tui, b"\x1b[?u\x1b[c");
}

pub fn tui_set_key_encoding(tui: &mut TUIData) {
    match tui.input.key_encoding {
        K_KEY_ENCODING_KITTY => {
            out(tui, b"\x1b[>3u");
        }
        K_KEY_ENCODING_XTERM => {
            out(tui, b"\x1b[>4;2m");
        }
        K_KEY_ENCODING_LEGACY => {}
        _ => {}
    }
}

fn tui_reset_key_encoding(tui: &mut TUIData) {
    match tui.input.key_encoding {
        K_KEY_ENCODING_KITTY => {
            out(tui, b"\x1b[<u");
        }
        K_KEY_ENCODING_XTERM => {
            out(tui, b"\x1b[>4;0m");
        }
        K_KEY_ENCODING_LEGACY => {}
        _ => {}
    }
}

fn tui_query_bg_color_noflush(tui: &mut TUIData) {
    out(tui, b"\x1b]11;?\x07\x1b[5n");
}

pub fn tui_query_bg_color(tui: &mut TUIData) {
    tui_query_bg_color_noflush(tui);
    flush_buf(tui, FlushBufFinish::FlushBufFinal);
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
                ObjectType::ObjectTypeBoolean
            }
            Self::IntMaxColors | Self::IntLines | Self::IntColumns => ObjectType::ObjectTypeInteger,
            Self::Def(_) | Self::KeyStr(_) | Self::FKey(_) => ObjectType::ObjectTypeString,
            Self::KeyArr(_) => ObjectType::ObjectTypeArray,
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
        b"carriage_return" => Some(Def(K_TERM_CARRIAGE_RETURN)),
        b"change_scroll_region" => Some(Def(K_TERM_CHANGE_SCROLL_REGION)),
        b"clear_screen" => Some(Def(K_TERM_CLEAR_SCREEN)),
        b"clr_eol" => Some(Def(K_TERM_CLR_EOL)),
        b"clr_eos" => Some(Def(K_TERM_CLR_EOS)),
        b"cursor_address" => Some(Def(K_TERM_CURSOR_ADDRESS)),
        b"cursor_down" => Some(Def(K_TERM_CURSOR_DOWN)),
        b"cursor_invisible" => Some(Def(K_TERM_CURSOR_INVISIBLE)),
        b"cursor_left" => Some(Def(K_TERM_CURSOR_LEFT)),
        b"cursor_home" => Some(Def(K_TERM_CURSOR_HOME)),
        b"cursor_normal" => Some(Def(K_TERM_CURSOR_NORMAL)),
        b"cursor_up" => Some(Def(K_TERM_CURSOR_UP)),
        b"cursor_right" => Some(Def(K_TERM_CURSOR_RIGHT)),
        b"delete_line" => Some(Def(K_TERM_DELETE_LINE)),
        b"enter_blink_mode" => Some(Def(K_TERM_ENTER_BLINK_MODE)),
        b"enter_bold_mode" => Some(Def(K_TERM_ENTER_BOLD_MODE)),
        b"enter_ca_mode" => Some(Def(K_TERM_ENTER_CA_MODE)),
        b"enter_dim_mode" => Some(Def(K_TERM_ENTER_DIM_MODE)),
        b"enter_italics_mode" => Some(Def(K_TERM_ENTER_ITALICS_MODE)),
        b"enter_reverse_mode" => Some(Def(K_TERM_ENTER_REVERSE_MODE)),
        b"enter_secure_mode" => Some(Def(K_TERM_ENTER_SECURE_MODE)),
        b"enter_standout_mode" => Some(Def(K_TERM_ENTER_STANDOUT_MODE)),
        b"enter_underline_mode" => Some(Def(K_TERM_ENTER_UNDERLINE_MODE)),
        b"erase_chars" => Some(Def(K_TERM_ERASE_CHARS)),
        b"exit_attribute_mode" => Some(Def(K_TERM_EXIT_ATTRIBUTE_MODE)),
        b"exit_ca_mode" => Some(Def(K_TERM_EXIT_CA_MODE)),
        b"from_status_line" => Some(Def(K_TERM_FROM_STATUS_LINE)),
        b"insert_line" => Some(Def(K_TERM_INSERT_LINE)),
        b"keypad_local" => Some(Def(K_TERM_KEYPAD_LOCAL)),
        b"keypad_xmit" => Some(Def(K_TERM_KEYPAD_XMIT)),
        b"parm_delete_line" => Some(Def(K_TERM_PARM_DELETE_LINE)),
        b"parm_down_cursor" => Some(Def(K_TERM_PARM_DOWN_CURSOR)),
        b"parm_insert_line" => Some(Def(K_TERM_PARM_INSERT_LINE)),
        b"parm_left_cursor" => Some(Def(K_TERM_PARM_LEFT_CURSOR)),
        b"parm_right_cursor" => Some(Def(K_TERM_PARM_RIGHT_CURSOR)),
        b"parm_up_cursor" => Some(Def(K_TERM_PARM_UP_CURSOR)),
        b"set_a_background" => Some(Def(K_TERM_SET_A_BACKGROUND)),
        b"set_a_foreground" => Some(Def(K_TERM_SET_A_FOREGROUND)),
        b"set_attributes" => Some(Def(K_TERM_SET_ATTRIBUTES)),
        b"set_lr_margin" => Some(Def(K_TERM_SET_LR_MARGIN)),
        b"to_status_line" => Some(Def(K_TERM_TO_STATUS_LINE)),
        b"reset_cursor_style" => Some(Def(K_TERM_RESET_CURSOR_STYLE)),
        b"set_cursor_style" => Some(Def(K_TERM_SET_CURSOR_STYLE)),
        b"enter_strikethrough_mode" => Some(Def(K_TERM_ENTER_STRIKETHROUGH_MODE)),
        b"set_rgb_foreground" => Some(Def(K_TERM_SET_RGB_FOREGROUND)),
        b"set_rgb_background" => Some(Def(K_TERM_SET_RGB_BACKGROUND)),
        b"set_cursor_color" => Some(Def(K_TERM_SET_CURSOR_COLOR)),
        b"reset_cursor_color" => Some(Def(K_TERM_RESET_CURSOR_COLOR)),
        b"set_underline_style" => Some(Def(K_TERM_SET_UNDERLINE_STYLE)),
        b"key_backspace" => Some(KeyStr(K_TERM_KEY_BACKSPACE)),
        b"key_beg" => Some(KeyArr(K_TERM_KEY_BEG)),
        b"key_btab" => Some(KeyStr(K_TERM_KEY_BTAB)),
        b"key_clear" => Some(KeyStr(K_TERM_KEY_CLEAR)),
        b"key_dc" => Some(KeyArr(K_TERM_KEY_DC)),
        b"key_end" => Some(KeyArr(K_TERM_KEY_END)),
        b"key_find" => Some(KeyArr(K_TERM_KEY_FIND)),
        b"key_home" => Some(KeyArr(K_TERM_KEY_HOME)),
        b"key_ic" => Some(KeyArr(K_TERM_KEY_IC)),
        b"key_npage" => Some(KeyStr(K_TERM_KEY_NPAGE)),
        b"key_ppage" => Some(KeyStr(K_TERM_KEY_PPAGE)),
        b"key_select" => Some(KeyStr(K_TERM_KEY_SELECT)),
        b"key_suspend" => Some(KeyArr(K_TERM_KEY_SUSPEND)),
        b"key_undo" => Some(KeyArr(K_TERM_KEY_UNDO)),
        b"key_left" => Some(KeyArr(K_TERM_KEY_LEFT)),
        b"key_right" => Some(KeyArr(K_TERM_KEY_RIGHT)),
        _ => {
            if let Some(rest) = name.strip_prefix(b"key_f")
                && let Ok(s) = std::str::from_utf8(rest)
                && let Ok(n) = s.parse::<usize>()
                && (1..=K_TERMINFO_FUNC_KEY_MAX).contains(&n)
            {
                return Some(FKey(n - 1));
            }
            None
        }
    }
}

fn push_termdef_err(err_arena: &mut Arena, chunks: &mut Vec<Object>, msg: &str) {
    let dup = arena_memdupz(err_arena, msg.as_ptr().cast::<c_char>(), msg.len());
    let str_val = NvimString {
        data: dup,
        size: msg.len(),
    };
    let mut err = arena_array(err_arena, 2);
    *err.next_slot_mut() = string_obj(str_val);
    err.size += 1;
    *err.next_slot_mut() = static_cstr_as_obj(c"ErrorMsg");
    err.size += 1;
    chunks.push(array_obj(err));
}

fn apply_termdefs(tui: &mut TUIData) {
    if !os_env_exists(c"NVIM_TERMDEFS", false) {
        return;
    }

    let mut lua_err = Error::new();
    let empty_args = Array {
        items: ptr::null_mut(),
        size: 0,
        capacity: 0,
    };
    let rv = nlua_exec(
        static_cstr_as_string(c"return require('vim.tty')._get_termdefs()"),
        ptr::null(),
        empty_args,
        K_RET_OBJECT,
        None,
        &mut lua_err,
    );
    if rv.obj_type != ObjectType::ObjectTypeDict {
        api_free_object(rv);
        api_clear_error(&mut lua_err);
        return;
    }

    let mut err_arena = ARENA_EMPTY;
    let mut chunks: Vec<Object> = Vec::new();
    let mut err_count = 0usize;

    let dict = rv.data.dict();
    for i in 0..dict.size {
        let kv = dict[i];
        let key_bytes = if kv.key.data.is_null() {
            &[][..]
        } else {
            crate::memory::cstr_as_bytes(kv.key.data)
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
                tui.ti.back_color_erase = val.data.boolean();
            }
            TermdefFieldTarget::BoolTc => {
                tui.ti.tc = val.data.boolean();
            }
            TermdefFieldTarget::BoolRGB => {
                tui.ti.rgb = val.data.boolean();
            }
            TermdefFieldTarget::BoolSu => {
                tui.ti.su = val.data.boolean();
            }
            TermdefFieldTarget::IntMaxColors => {
                tui.ti.max_colors = val.data.integer() as i32;
            }
            TermdefFieldTarget::IntLines => {
                tui.ti.lines = val.data.integer() as i32;
            }
            TermdefFieldTarget::IntColumns => {
                tui.ti.columns = val.data.integer() as i32;
            }
            TermdefFieldTarget::Def(idx) => {
                tui.ti.defs[idx] = Some(std::borrow::Cow::Owned(crate::memory::cstr_to_cstring(
                    val.data.string().data,
                )));
            }
            TermdefFieldTarget::KeyStr(idx) => {
                tui.ti.keys[idx][0] = Some(std::borrow::Cow::Owned(
                    crate::memory::cstr_to_cstring(val.data.string().data),
                ));
            }
            TermdefFieldTarget::KeyArr(idx) => {
                let arr = val.data.array();
                if arr.size > 2 {
                    let msg = format!(
                        "skipping invalid field definition in $NVIM_TERMDEFS for {key_str}: array should have at most two entries for unshifted and shifted key variants.\n"
                    );
                    push_termdef_err(&mut err_arena, &mut chunks, &msg);
                    err_count += 1;
                    continue;
                }
                tui.ti.keys[idx][0] = None;
                tui.ti.keys[idx][1] = None;
                for key_map_idx in 0..arr.size {
                    let key_mapping = arr[key_map_idx];
                    if key_mapping.obj_type != ObjectType::ObjectTypeString {
                        let msg = format!(
                            "skipping invalid field definition in $NVIM_TERMDEFS for {key_str}[{key_map_idx}]: expected type 'string'\n"
                        );
                        push_termdef_err(&mut err_arena, &mut chunks, &msg);
                        err_count += 1;
                        continue;
                    }
                    tui.ti.keys[idx][key_map_idx] = Some(std::borrow::Cow::Owned(
                        crate::memory::cstr_to_cstring(key_mapping.data.string().data),
                    ));
                }
            }
            TermdefFieldTarget::FKey(idx) => {
                tui.ti.f_keys[idx] = Some(std::borrow::Cow::Owned(crate::memory::cstr_to_cstring(
                    val.data.string().data,
                )));
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
        let chunks_arr = Array {
            items: chunks.as_mut_ptr(),
            size: chunks.len(),
            capacity: chunks.len(),
        };
        let mut args_items = [array_obj(chunks_arr), boolean_obj(true), dict_obj(opts)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        rpc_send_event(g_st().ui_client_channel_id, c"nvim_echo", args);
    }

    arena_mem_free(arena_finish(&mut err_arena));
    api_free_object(rv);
    api_clear_error(&mut lua_err);
}

fn terminfo_start(tui: &mut TUIData) {
    tui.bufpos = 0;
    tui.default_attr = false;
    tui.can_clear_attr = false;
    tui.is_invisible = true;
    tui.want_invisible = false;
    tui.busy = false;
    tui.set_cursor_color_as_str = false;
    tui.cursor_has_color = false;
    tui.resize_events_enabled = false;
    tui.modes_grapheme_clusters = false;
    tui.modes_resize_events = false;
    tui.modes_theme_updates = false;
    tui.showing_mode = SHAPE_IDX_N as ModeShape;
    tui.terminfo_ext.enable_focus_reporting = None;
    tui.terminfo_ext.disable_focus_reporting = None;

    tui.out_fd = STDOUT_FILENO;
    tui.out_isatty = os_isatty(tui.out_fd);
    tui.input.tui_data = &raw mut *tui;

    assert!(tui.term.is_none());

    #[cfg(windows)]
    let mut term_cstr = os_getenv_cstr(c"TERM");
    #[cfg(not(windows))]
    let term_cstr = os_getenv_cstr(c"TERM");
    #[cfg(windows)]
    {
        let mut guessed_term: *const c_char = ptr::null();
        os_tty_guess_term(&raw mut guessed_term, tui.out_fd);
        if term_cstr.is_none() && !guessed_term.is_null() {
            term_cstr = Some(crate::memory::cstr_to_cstring(guessed_term));
            os_setenv(c"TERM", guessed_term, 1);
        }
    }
    let term = term_cstr.as_deref().map(CStr::to_bytes);

    tui.terminfo_found_in_db = false;
    if term.is_some()
        && !os_env_exists(c"NVIM_TERMDEFS", false)
        && terminfo_from_database(&mut tui.ti, term)
    {
        tui.term = term_cstr.clone();
        tui.terminfo_found_in_db = true;
    }

    if !tui.terminfo_found_in_db {
        tui.ti = terminfo_from_builtin(term, &mut tui.term);
    }

    let colorterm_cstr = os_getenv_cstr(c"COLORTERM");
    let termprg_cstr = os_getenv_cstr(c"TERM_PROGRAM");
    let vte_version_cstr = os_getenv_cstr(c"VTE_VERSION");
    let konsolev_cstr = os_getenv_cstr(c"KONSOLE_VERSION");
    let term_program_version_cstr = os_getenv_cstr(c"TERM_PROGRAM_VERSION");

    let colorterm = colorterm_cstr.as_deref().map(CStr::to_bytes);
    let termprg = termprg_cstr.as_deref().map(CStr::to_bytes);
    let vte_version_env = vte_version_cstr
        .as_ref()
        .map_or(ptr::null(), |c| c.as_ptr());
    let konsolev_env = konsolev_cstr.as_ref().map_or(ptr::null(), |c| c.as_ptr());

    let mut vte_version_end = vte_version_env.cast_mut();
    let vtev = if !vte_version_env.is_null() {
        getdigits_int(&raw mut vte_version_end, false, 0)
    } else {
        0
    };
    let iterm_env = slice_contains(termprg, b"iTerm.app");
    let nsterm =
        slice_contains(termprg, b"Apple_Terminal") || terminfo_is_term_family(term, c"nsterm");
    let konsole = terminfo_is_term_family(term, c"konsole")
        || os_env_exists(c"KONSOLE_PROFILE_NAME", true)
        || os_env_exists(c"KONSOLE_DBUS_SESSION", true);
    let mut konsolev_end = konsolev_env.cast_mut();
    let konsolev = if !konsolev_env.is_null() {
        getdigits_int(&raw mut konsolev_end, false, 0)
    } else if konsole {
        1
    } else {
        0
    };
    let wezterm = termprg == Some(b"WezTerm");
    let weztermv = if wezterm {
        term_program_version_cstr.as_deref().map(CStr::to_bytes)
    } else {
        None
    };
    let screen = terminfo_is_term_family(term, c"screen");
    let tmux = terminfo_is_term_family(term, c"tmux") || os_env_exists(c"TMUX", true);
    tui.screen_or_tmux = screen || tmux;

    tui.rgb = term_has_truecolor(tui, colorterm);

    patch_terminfo_bugs(tui, term, colorterm, vtev, konsolev, iterm_env, nsterm);
    augment_terminfo(tui, term, vtev, konsolev, weztermv, iterm_env, nsterm);
    apply_termdefs(tui);

    let has_set_underline_style = tui.ti.defs[K_TERM_SET_UNDERLINE_STYLE].is_some();
    tui.can_change_scroll_region = tui.ti.defs[K_TERM_CHANGE_SCROLL_REGION].is_some();
    tui.can_set_lr_margin = tui.ti.defs[K_TERM_SET_LR_MARGIN].is_some();
    tui.can_scroll = tui.ti.defs[K_TERM_DELETE_LINE].is_some()
        && tui.ti.defs[K_TERM_PARM_DELETE_LINE].is_some()
        && tui.ti.defs[K_TERM_INSERT_LINE].is_some()
        && tui.ti.defs[K_TERM_PARM_INSERT_LINE].is_some();
    tui.can_erase_chars = tui.ti.defs[K_TERM_ERASE_CHARS].is_some();
    tui.immediate_wrap_after_last_column = terminfo_is_term_family(term, c"conemu")
        || terminfo_is_term_family(term, c"cygwin")
        || terminfo_is_term_family(term, c"win32con")
        || terminfo_is_term_family(term, c"interix");
    tui.bce = tui.ti.back_color_erase;
    g_st().t_colors = tui.ti.max_colors;

    terminfo_out(tui, K_TERM_ENTER_CA_MODE);
    terminfo_out(tui, K_TERM_KEYPAD_XMIT);
    terminfo_out(tui, K_TERM_CLEAR_SCREEN);

    tui_set_term_mode(tui, K_TERM_MODE_BRACKETED_PASTE, true);

    tui.has_left_and_right_margin_mode = false;
    tui.has_sync_mode = false;

    if !nsterm {
        tui_request_term_mode(tui, K_TERM_MODE_LEFT_AND_RIGHT_MARGINS);
        tui_request_term_mode(tui, K_TERM_MODE_SYNCHRONIZED_OUTPUT);
        tui_request_term_mode(tui, K_TERM_MODE_GRAPHEME_CLUSTERS);
        tui_request_term_mode(tui, K_TERM_MODE_THEME_UPDATES);
        tui_request_term_mode(tui, K_TERM_MODE_RESIZE_EVENTS);
    }

    if !has_set_underline_style && !(screen || tmux || nsterm) {
        tui_query_extended_underline(tui);
    }

    tui_query_kitty_keyboard(tui);
    tui_query_bg_color_noflush(tui);

    let write_loop = tui_write_loop(tui);
    let output_handle = tui_output_handle(tui);
    debug_assert!(unsafe { crate::event::r#loop::uv_loop_size() } <= size_of::<[u64; 256]>());
    debug_assert!(
        unsafe { crate::event::r#loop::uv_handle_size(crate::event::stream::UV_NAMED_PIPE) }
            .max(unsafe { crate::event::r#loop::uv_handle_size(crate::event::stream::UV_TTY) })
            <= size_of::<[u64; 80]>()
    );
    unsafe { uv_loop_init(write_loop) };
    if tui.out_isatty {
        let ret = unsafe { uv_tty_init(write_loop, output_handle.cast(), tui.out_fd, 0) };
        if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"terminfo_start",
                628,
                true,
                c"uv_tty_init failed: %s",
                unsafe { uv_strerror(ret) },
            );
        }
        #[cfg(not(windows))]
        {
            let mut retry_count = 10;
            let mut ret;
            loop {
                ret = unsafe { uv_tty_set_mode(output_handle.cast(), UV_TTY_MODE_IO) };
                if ret == UV_EINTR && retry_count > 0 {
                    retry_count -= 1;
                } else {
                    break;
                }
            }
            if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"terminfo_start",
                    638,
                    true,
                    c"uv_tty_set_mode failed: %s",
                    unsafe { uv_strerror(ret) },
                );
            }
        }
    } else {
        let ret = unsafe { uv_pipe_init(write_loop, output_handle.cast(), 0) };
        if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"terminfo_start",
                644,
                true,
                c"uv_pipe_init failed: %s",
                unsafe { uv_strerror(ret) },
            );
        }
        let ret = unsafe { uv_pipe_open(output_handle.cast(), tui.out_fd) };
        if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"terminfo_start",
                648,
                true,
                c"uv_pipe_open failed: %s",
                unsafe { uv_strerror(ret) },
            );
        }
    }
    flush_buf(tui, FlushBufFinish::FlushBufFinal);
}

fn terminfo_disable(tui: &mut TUIData) {
    if tui.modes_theme_updates {
        tui_set_term_mode(tui, K_TERM_MODE_THEME_UPDATES, false);
    }

    tui_mode_change(tui, NULL_STRING, SHAPE_IDX_N as Integer);
    tui_mouse_off(tui);
    terminfo_out(tui, K_TERM_EXIT_ATTRIBUTE_MODE);
    terminfo_out(tui, K_TERM_CURSOR_NORMAL);
    terminfo_out(tui, K_TERM_RESET_CURSOR_STYLE);
    terminfo_out(tui, K_TERM_KEYPAD_LOCAL);

    tui_reset_key_encoding(tui);

    if tui.modes_resize_events {
        tui_set_term_mode(tui, K_TERM_MODE_RESIZE_EVENTS, false);
    }

    if tui.modes_grapheme_clusters {
        tui_set_term_mode(tui, K_TERM_MODE_GRAPHEME_CLUSTERS, false);
    }

    tui_set_title(tui, NULL_STRING);
    if tui.cursor_has_color {
        terminfo_out(tui, K_TERM_RESET_CURSOR_COLOR);
    }
    tui_set_term_mode(tui, K_TERM_MODE_BRACKETED_PASTE, false);
    out_len(tui, tui.terminfo_ext.disable_focus_reporting);
    out(tui, b"\x1b[c");

    flush_buf(tui, FlushBufFinish::FlushBufFinal);
}

fn terminfo_stop(tui: &mut TUIData) {
    if g_st().ui_client_exit_status == 0 && g_st().ui_client_error_exit > 0 {
        g_st().ui_client_exit_status = g_st().ui_client_error_exit;
    }

    if g_st().ui_client_exit_status == g_st().ui_client_error_exit.max(0) {
        cursor_goto(tui, tui.height - 1, 0);
        terminfo_out(tui, K_TERM_EXIT_CA_MODE);
    }

    flush_buf(tui, FlushBufFinish::FlushBufFinal);
    unsafe { uv_tty_reset_mode() };
    unsafe { uv_close(tui_output_handle(tui), None) };
    unsafe { uv_run(tui_write_loop(tui), UV_RUN_DEFAULT) };
    if unsafe { uv_loop_close(tui_write_loop(tui)) } != 0 {
        std::process::abort();
    }
    tui.ti = TerminfoEntry::new();
    tui.term = None;
}

fn tui_terminal_start(tui: &mut TUIData) {
    tui.print_attr_id = -1;
    terminfo_start(tui);
    let input = tui_input(tui);
    if input.loop_.is_null() {
        tinput_init(
            &mut *input,
            (&raw mut g_st().main_loop).cast::<Loop>(),
            &tui.ti,
        );
    }
    tui_guess_size(tui);
    tinput_start(&mut *input);
}

unsafe extern "C" fn after_startup_cb(handle: *mut uv_timer_t) {
    unsafe {
        let tui = &mut **handle.cast::<*mut TUIData>();
        tui_terminal_after_startup(tui);
    }
}

fn tui_terminal_after_startup(tui: &mut TUIData) {
    out_len(tui, tui.terminfo_ext.enable_focus_reporting);
    flush_buf(tui, FlushBufFinish::FlushBufFinal);
}

pub fn tui_stop(tui: &mut TUIData) {
    if unsafe { uv_is_closing(tui_output_handle(tui)) } != 0 {
        if log_st().g_min_log_level <= LOGLVL_ERR {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"tui_stop",
                760,
                true,
                c"TUI already stopped (race?)",
            );
        }
        tui.stopped = true;
        return;
    }

    let input = tui_input(tui);
    input.primary_device_attr = Some(PrimaryDeviceAttrAction::Stop);
    terminfo_disable(tui);

    let wait_start = os_hrtime();
    let events = loop_queues(unsafe { &mut *tui.loop_ }).events;
    let rs = &mut input.read_stream;
    let mut remaining: i64 = EXIT_TIMEOUT_MS;
    let mut before = if remaining > 0 { os_hrtime() } else { 0 };
    while !(unsafe { ptr::read_volatile(&raw const tui.stopped) }
        || unsafe { ptr::read_volatile(&raw const rs.did_eof) })
    {
        if !events.is_null() && !multiqueue_empty(unsafe { &*events }) {
            multiqueue_process_events(unsafe { &mut *events });
        } else {
            loop_poll_events(unsafe { &mut *tui.loop_ }, remaining);
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
    if !tui.stopped
        && ((os_hrtime() - wait_start) / 1_000_000) as i64 >= EXIT_TIMEOUT_MS
        && log_st().g_min_log_level <= LOGLVL_WRN
    {
        logmsg!(
            LOGLVL_WRN,
            ptr::null(),
            c"tui_stop",
            773,
            true,
            c"TUI: timed out waiting for DA1 response",
        );
    }
    tui.stopped = true;

    tui_terminal_stop(tui);
    stream_set_blocking(input.in_fd, true);
    tinput_destroy(&mut *input);
    signal_watcher_stop(&mut *tui_winch_handle(tui));
    signal_watcher_close(&mut *tui_winch_handle(tui));
    unsafe { uv_close(tui_startup_delay_timer(tui).cast(), None) };
}

fn tui_stop_cb(tui: &mut TUIData) {
    tui.stopped = true;
}

fn tui_terminal_stop(tui: &mut TUIData) {
    tinput_stop(&mut *tui_input(tui));
    terminfo_stop(tui);
}

pub fn tui_is_stopped(tui: &TUIData) -> bool {
    tui.stopped
}

pub fn tui_free_all_mem(tui: Option<Box<TUIData>>) {
    tui_st().urls = Vec::new();

    drop(tui);
}

pub(crate) fn sigwinch_cb(tui: &mut TUIData) {
    if tui_is_stopped(tui) || tui.resize_events_enabled {
        return;
    }
    tui_guess_size(tui);
}

fn attrs_differ(tui: &TUIData, id1: i32, id2: i32, rgb: bool) -> bool {
    if id1 == id2 {
        return false;
    } else if id1 < 0 || id2 < 0 {
        return true;
    }
    let attrs = &tui.attrs;
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
            || ((a1.cterm_ae_attr & HL_UNDERLINE_MASK) != 0 && a1.rgb_sp_color != a2.rgb_sp_color)
    }
}

fn update_attrs(tui: &mut TUIData, attr_id: i32) {
    if !attrs_differ(tui, attr_id, tui.print_attr_id, tui.rgb) {
        tui.print_attr_id = attr_id;
        return;
    }
    tui.print_attr_id = attr_id;
    let attrs = tui.attrs.as_slice()[attr_id as usize];
    let attr = if tui.rgb {
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
    if tui.ti.defs[K_TERM_SET_UNDERLINE_STYLE].is_some() {
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

    if tui.ti.defs[K_TERM_SET_ATTRIBUTES].is_some() {
        if bold || dim || blink || reverse || underline || standout {
            let mut params = [TPVAR::default(); 9];
            params[0].num = i64::from(standout);
            params[1].num = i64::from(underline);
            params[2].num = i64::from(reverse);
            params[3].num = i64::from(blink);
            params[4].num = i64::from(dim);
            params[5].num = i64::from(bold);
            params[6].num = 0;
            params[7].num = 0;
            params[8].num = 0;
            terminfo_print(tui, K_TERM_SET_ATTRIBUTES, &mut params);
        } else if !tui.default_attr {
            terminfo_out(tui, K_TERM_EXIT_ATTRIBUTE_MODE);
        }
    } else {
        if !tui.default_attr {
            terminfo_out(tui, K_TERM_EXIT_ATTRIBUTE_MODE);
        }
        if bold {
            terminfo_out(tui, K_TERM_ENTER_BOLD_MODE);
        }
        if underline {
            terminfo_out(tui, K_TERM_ENTER_UNDERLINE_MODE);
        }
        if standout {
            terminfo_out(tui, K_TERM_ENTER_STANDOUT_MODE);
        }
        if reverse {
            terminfo_out(tui, K_TERM_ENTER_REVERSE_MODE);
        }
        if dim {
            terminfo_out(tui, K_TERM_ENTER_DIM_MODE);
        }
        if blink {
            terminfo_out(tui, K_TERM_ENTER_BLINK_MODE);
        }
    }
    if italic {
        terminfo_out(tui, K_TERM_ENTER_ITALICS_MODE);
    }
    if altfont {
        out_len(tui, tui.terminfo_ext.enter_altfont_mode);
    }
    if strikethrough {
        terminfo_out(tui, K_TERM_ENTER_STRIKETHROUGH_MODE);
    }
    if conceal {
        terminfo_out(tui, K_TERM_ENTER_SECURE_MODE);
    }
    if overline {
        out(tui, b"\x1b[53m");
    }
    if tui.ti.defs[K_TERM_SET_UNDERLINE_STYLE].is_some() {
        if undercurl {
            terminfo_print_num1(tui, K_TERM_SET_UNDERLINE_STYLE, 3);
        }
        if underdouble {
            terminfo_print_num1(tui, K_TERM_SET_UNDERLINE_STYLE, 2);
        }
        if underdotted {
            terminfo_print_num1(tui, K_TERM_SET_UNDERLINE_STYLE, 4);
        }
        if underdashed {
            terminfo_print_num1(tui, K_TERM_SET_UNDERLINE_STYLE, 5);
        }
    }

    if has_any_underline && tui.can_set_underline_color {
        let color = attrs.rgb_sp_color;
        if color != -1 {
            let r = (color >> 16) & 0xff;
            let g = (color >> 8) & 0xff;
            let b = color & 0xff;
            let s = format!("\x1b[58:2::{r}:{g}:{b}m");
            out_printf_str(tui, 128, &s);
        }
    }

    let fg: i32;
    if tui.rgb && (attr & HL_FG_INDEXED) == 0 {
        fg = if attrs.rgb_fg_color != -1 {
            attrs.rgb_fg_color
        } else {
            tui.clear_attrs.rgb_fg_color
        };
        if fg != -1 {
            terminfo_print_num(
                tui,
                K_TERM_SET_RGB_FOREGROUND,
                (fg >> 16) & 0xff,
                (fg >> 8) & 0xff,
                fg & 0xff,
            );
        }
    } else {
        fg = if attrs.cterm_fg_color != 0 {
            i32::from(attrs.cterm_fg_color) - 1
        } else {
            i32::from(tui.clear_attrs.cterm_fg_color) - 1
        };
        if fg != -1 {
            terminfo_print_num1(tui, K_TERM_SET_A_FOREGROUND, fg);
        }
    }

    let bg: i32;
    if tui.rgb && (attr & HL_BG_INDEXED) == 0 {
        bg = if attrs.rgb_bg_color != -1 {
            attrs.rgb_bg_color
        } else {
            tui.clear_attrs.rgb_bg_color
        };
        if bg != -1 {
            terminfo_print_num(
                tui,
                K_TERM_SET_RGB_BACKGROUND,
                (bg >> 16) & 0xff,
                (bg >> 8) & 0xff,
                bg & 0xff,
            );
        }
    } else {
        bg = if attrs.cterm_bg_color != 0 {
            i32::from(attrs.cterm_bg_color) - 1
        } else {
            i32::from(tui.clear_attrs.cterm_bg_color) - 1
        };
        if bg != -1 {
            terminfo_print_num1(tui, K_TERM_SET_A_BACKGROUND, bg);
        }
    }

    if tui.url != attrs.url {
        if attrs.url >= 0 {
            let url_str = tui_st().urls.as_slice()[attrs.url as usize].to_string_lossy();
            let id: u64 = 0xE1EA_0000u64 + u64::from(attrs.url as u32);
            let formatted = format!("\x1b]8;id={id};{url_str}\x1b\\");
            out(tui, formatted.as_bytes());
        } else {
            out(tui, b"\x1b]8;;\x1b\\");
        }
        tui.url = attrs.url;
    }

    tui.default_attr = fg == -1
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

    tui.can_clear_attr = !reverse
        && !standout
        && !dim
        && !blink
        && !conceal
        && !overline
        && !has_any_underline
        && !strikethrough
        && (tui.bce || bg == -1);
}

fn final_column_wrap(tui: &mut TUIData) {
    let grid = &mut tui.grid;
    if grid.row != -1 && grid.col == tui.width {
        grid.col = 0;
        if grid.row < tui.height.min(grid.height - 1) {
            grid.row += 1;
        }
    }
}

fn print_cell(tui: &mut TUIData, bytes: &[u8], attr: Sattr) {
    if !tui.immediate_wrap_after_last_column {
        final_column_wrap(tui);
    }
    update_attrs(tui, attr);
    out(tui, bytes);
    tui.grid.col += 1;
    if tui.immediate_wrap_after_last_column {
        final_column_wrap(tui);
    }
}

fn cheap_to_print(tui: &TUIData, row: i32, col: i32, mut next: i32) -> bool {
    let cells = &tui.grid.cells;
    let row_cells = &cells[row as usize];
    let mut idx = col as usize;
    while next != 0 {
        next -= 1;
        let cell = row_cells[idx];
        if attrs_differ(tui, cell.attr, tui.print_attr_id, tui.rgb) && tui.default_attr {
            return false;
        }
        if schar_get_ascii(cell.data) == 0 {
            return false;
        }
        idx += 1;
    }
    true
}

fn cursor_goto(tui: &mut TUIData, row: i32, col: i32) {
    if row == tui.grid.row && col == tui.grid.col {
        return;
    }

    if tui.url >= 0 {
        out(tui, b"\x1b]8;;\x1b\\");
        tui.url = -1;
        tui.print_attr_id = -1;
    }

    if row == 0 && col == 0 {
        terminfo_out(tui, K_TERM_CURSOR_HOME);
        ugrid_goto(&mut tui.grid, row, col);
        return;
    }
    if tui.grid.row >= 0 {
        let do_cr = if col == 0 {
            col != tui.grid.col
        } else if row != tui.grid.row {
            false
        } else if col == 1 {
            2 < tui.grid.col && cheap_to_print(tui, tui.grid.row, 0, col)
        } else if col == 2 {
            5 < tui.grid.col && cheap_to_print(tui, tui.grid.row, 0, col)
        } else {
            false
        };
        if do_cr {
            terminfo_out(tui, K_TERM_CARRIAGE_RETURN);
            let cur_row = tui.grid.row;
            ugrid_goto(&mut tui.grid, cur_row, 0);
        }
        if row == tui.grid.row {
            if col < tui.grid.col
                && (tui.immediate_wrap_after_last_column || tui.grid.col < tui.width)
            {
                let n = tui.grid.col - col;
                if n <= 4 {
                    for _ in 0..n {
                        terminfo_out(tui, K_TERM_CURSOR_LEFT);
                    }
                } else {
                    terminfo_print_num1(tui, K_TERM_PARM_LEFT_CURSOR, n);
                }
                ugrid_goto(&mut tui.grid, row, col);
                return;
            } else if col > tui.grid.col {
                let n = col - tui.grid.col;
                if n <= 2 {
                    for _ in 0..n {
                        terminfo_out(tui, K_TERM_CURSOR_RIGHT);
                    }
                } else {
                    terminfo_print_num1(tui, K_TERM_PARM_RIGHT_CURSOR, n);
                }
                ugrid_goto(&mut tui.grid, row, col);
                return;
            }
        }
        if col == tui.grid.col {
            if row > tui.grid.row {
                let n = row - tui.grid.row;
                if n <= 4 {
                    for _ in 0..n {
                        terminfo_out(tui, K_TERM_CURSOR_DOWN);
                    }
                } else {
                    terminfo_print_num1(tui, K_TERM_PARM_DOWN_CURSOR, n);
                }
                ugrid_goto(&mut tui.grid, row, col);
                return;
            } else if row < tui.grid.row {
                let n = tui.grid.row - row;
                if n <= 2 {
                    for _ in 0..n {
                        terminfo_out(tui, K_TERM_CURSOR_UP);
                    }
                } else {
                    terminfo_print_num1(tui, K_TERM_PARM_UP_CURSOR, n);
                }
                ugrid_goto(&mut tui.grid, row, col);
                return;
            }
        }
    }

    terminfo_print_num2(tui, K_TERM_CURSOR_ADDRESS, row, col);
    ugrid_goto(&mut tui.grid, row, col);
}

fn print_spaces(tui: &mut TUIData, width: i32) {
    let mut left = width as usize;
    loop {
        let buf_fit = left.min(tui.buf.len() - tui.bufpos);
        unsafe { ptr::write_bytes(tui.buf.as_mut_ptr().wrapping_add(tui.bufpos), b' ', buf_fit) };
        tui.bufpos += buf_fit;
        left -= buf_fit;
        if left == 0 {
            break;
        }
        flush_buf(tui, FlushBufFinish::FlushBufPartial);
    }

    tui.grid.col += width;
    if tui.immediate_wrap_after_last_column {
        final_column_wrap(tui);
    }
}

fn print_cell_at_pos(tui: &mut TUIData, row: i32, col: i32, cell: UCell, is_doublewidth: bool) {
    if tui.grid.row == -1 && cell.data == 0 {
        return;
    }

    cursor_goto(tui, row, col);

    let mut buf = [0u8; MAX_SCHAR_SIZE];
    let len = crate::grid::schar_get_bytes(&mut buf, cell.data);
    let c = utf_ptr2char(buf.as_ptr().cast::<c_char>());
    let mut is_ambiwidth = utf_ambiguous_width(buf.as_ptr().cast::<c_char>());
    if is_doublewidth && (is_ambiwidth || utf_char2cells(c) == 1) {
        is_ambiwidth = true;
        update_attrs(tui, cell.attr);
        print_spaces(tui, 2);
        cursor_goto(tui, row, col);
    }

    print_cell(tui, &buf[..len], cell.attr);

    if is_ambiwidth {
        tui.grid.row = -1;
    }
}

fn clear_region(tui: &mut TUIData, top: i32, bot: i32, left: i32, right: i32, attr_id: i32) {
    if tui.set_default_colors {
        update_attrs(tui, attr_id);
    } else {
        terminfo_out(tui, K_TERM_EXIT_ATTRIBUTE_MODE);
    }

    if tui.can_clear_attr && left == 0 && right == tui.width && bot == tui.height {
        if top == 0 {
            terminfo_out(tui, K_TERM_CLEAR_SCREEN);
            ugrid_goto(&mut tui.grid, top, left);
        } else {
            cursor_goto(tui, top, 0);
            terminfo_out(tui, K_TERM_CLR_EOS);
        }
    } else {
        let width = right - left;
        for row in top..bot {
            cursor_goto(tui, row, left);
            if tui.can_clear_attr && right == tui.width {
                terminfo_out(tui, K_TERM_CLR_EOL);
            } else if tui.can_erase_chars && tui.can_clear_attr {
                terminfo_print_num1(tui, K_TERM_ERASE_CHARS, width);
            } else {
                print_spaces(tui, width);
            }
        }
    }
}

fn set_scroll_region(tui: &mut TUIData, top: i32, bot: i32, left: i32, right: i32) {
    terminfo_print_num2(tui, K_TERM_CHANGE_SCROLL_REGION, top, bot);
    if left != 0 || right != tui.width - 1 {
        tui_set_term_mode(tui, K_TERM_MODE_LEFT_AND_RIGHT_MARGINS, true);
        terminfo_print_num2(tui, K_TERM_SET_LR_MARGIN, left, right);
    }
    tui.grid.row = -1;
}

fn reset_scroll_region(tui: &mut TUIData, fullwidth: bool) {
    if tui.terminfo_ext.reset_scroll_region.is_some() {
        out_len(tui, tui.terminfo_ext.reset_scroll_region);
    } else {
        terminfo_print_num2(tui, K_TERM_CHANGE_SCROLL_REGION, 0, tui.height - 1);
    }
    if !fullwidth {
        terminfo_print_num2(tui, K_TERM_SET_LR_MARGIN, 0, tui.width - 1);
        tui_set_term_mode(tui, K_TERM_MODE_LEFT_AND_RIGHT_MARGINS, false);
    }
    tui.grid.row = -1;
}

pub fn tui_grid_resize(tui: &mut TUIData, _g: Integer, width: Integer, height: Integer) {
    ugrid_resize(&mut tui.grid, width as i32, height as i32);

    let grid_height = tui.grid.height;
    let grid_width = tui.grid.width;
    for r in &mut tui.invalid_regions {
        r.bot = r.bot.min(grid_height);
        r.right = r.right.min(grid_width);
    }

    if tui.pending_resize_events == 0 && !tui.is_starting {
        let s = format!("\x1b[8;{};{}t", height as i32, width as i32);
        out_printf_str(tui, 64, &s);
    } else {
        tui.pending_resize_events = if tui.pending_resize_events > 0 {
            tui.pending_resize_events - 1
        } else {
            0
        };
        tui.grid.row = -1;
    }
}

pub fn tui_grid_clear(tui: &mut TUIData, _g: Integer) {
    ugrid_clear(&mut tui.grid);
    schar_cache_clear_if_full();
    tui.invalid_regions.clear();
    clear_region(tui, 0, tui.height, 0, tui.width, 0);
}

pub fn tui_grid_cursor_goto(tui: &mut TUIData, _grid: Integer, row: Integer, col: Integer) {
    tui.row = row as i32;
    tui.col = col as i32;
}

fn tui_cursor_decode_shape(shape_str: impl crate::strings::IntoCCharPtr) -> CursorShape {
    let shape_str = shape_str.into_c_char_ptr();
    if crate::memory::cstr_as_opt_bytes(shape_str) == Some(b"block") {
        CursorShape::ShapeBlock
    } else if crate::memory::cstr_as_opt_bytes(shape_str) == Some(b"vertical") {
        CursorShape::ShapeVer
    } else if crate::memory::cstr_as_opt_bytes(shape_str) == Some(b"horizontal") {
        CursorShape::ShapeHor
    } else {
        if log_st().g_min_log_level <= LOGLVL_WRN {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"tui_cursor_decode_shape",
                1268,
                true,
                c"Unknown shape value '%s'",
                shape_str,
            );
        }
        CursorShape::ShapeBlock
    }
}

fn tui_cursor_reset_style(tui: &mut TUIData) {
    terminfo_out(tui, K_TERM_RESET_CURSOR_STYLE);
}

fn decode_cursor_entry(args: Dict) -> Cursorentry {
    let mut r = (*shape_table_mut())[0];
    for i in 0..args.size {
        let item = args[i];
        let key = item.key.data;
        let value = item.value;
        if crate::memory::cstr_as_opt_bytes(key) == Some(b"cursor_shape") {
            r.shape = tui_cursor_decode_shape(value.data.string().data);
        } else if crate::memory::cstr_as_opt_bytes(key) == Some(b"blinkon") {
            r.blinkon = value.data.integer() as i32;
        } else if crate::memory::cstr_as_opt_bytes(key) == Some(b"blinkoff") {
            r.blinkoff = value.data.integer() as i32;
        } else if crate::memory::cstr_as_opt_bytes(key) == Some(b"attr_id") {
            r.id = value.data.integer() as i32;
        }
    }
    r
}

pub fn tui_mode_info_set(tui: &mut TUIData, guicursor_enabled: bool, args: Array) {
    tui_st().cursor_style_enabled = guicursor_enabled;
    if !guicursor_enabled {
        tui_cursor_reset_style(tui);
        return;
    }

    assert!(args.size != 0);

    for i in 0..args.size {
        let item = args[i];
        assert!(item.obj_type == ObjectType::ObjectTypeDict);
        let r = decode_cursor_entry(item.data.dict());
        tui.cursor_shapes[i] = r;
    }

    tui_set_mode(tui, tui.showing_mode);
}

pub fn tui_update_menu(_tui: &mut TUIData) {}

pub fn tui_busy_start(tui: &mut TUIData) {
    tui.busy = true;
}

pub fn tui_busy_stop(tui: &mut TUIData) {
    tui.busy = false;
}

pub fn tui_mouse_on(tui: &mut TUIData) {
    if !tui.mouse_enabled {
        tui_set_term_mode(tui, K_TERM_MODE_MOUSE_BUTTON_EVENT, true);
        tui_set_term_mode(tui, K_TERM_MODE_MOUSE_SGREXT, true);
        if tui.mouse_move_enabled {
            tui_set_term_mode(tui, K_TERM_MODE_MOUSE_ANY_EVENT, true);
        }
        tui.mouse_enabled = true;
    }
}

pub fn tui_mouse_off(tui: &mut TUIData) {
    if tui.mouse_enabled {
        if tui.mouse_move_enabled {
            tui_set_term_mode(tui, K_TERM_MODE_MOUSE_ANY_EVENT, false);
        }
        tui_set_term_mode(tui, K_TERM_MODE_MOUSE_BUTTON_EVENT, false);
        tui_set_term_mode(tui, K_TERM_MODE_MOUSE_SGREXT, false);
        tui.mouse_enabled = false;
    }
}

fn tui_set_mode(tui: &mut TUIData, mode: ModeShape) {
    if !tui_st().cursor_style_enabled {
        tui_cursor_reset_style(tui);
        return;
    }
    let c = tui.cursor_shapes[mode as usize];

    if c.id != 0 && (c.id as usize) < tui.attrs.len() && tui.rgb {
        let aep = tui.attrs.as_slice()[c.id as usize];
        tui.want_invisible = aep.hl_blend == 100;
        if !tui.want_invisible && (aep.rgb_ae_attr & HL_INVERSE) != 0 {
            terminfo_out(tui, K_TERM_RESET_CURSOR_COLOR);
        } else if !tui.want_invisible && aep.rgb_bg_color >= 0 {
            let mut params = [TPVAR::default(); 9];
            if tui.set_cursor_color_as_str {
                let s = format!("#{:06x}", aep.rgb_bg_color as u32 & 0x00ff_ffff);
                let mut hexbuf = [0u8; 8];
                hexbuf[..7].copy_from_slice(&s.as_bytes()[..7]);
                params[0].string = Some(hexbuf);
            } else {
                params[0].num = i64::from(aep.rgb_bg_color);
            }
            terminfo_print(tui, K_TERM_SET_CURSOR_COLOR, &mut params);
            tui.cursor_has_color = true;
        }
    } else if c.id == 0 && (tui.want_invisible || tui.cursor_has_color) {
        tui.want_invisible = false;
        tui.cursor_has_color = false;
        terminfo_out(tui, K_TERM_RESET_CURSOR_COLOR);
    }

    let shape = match c.shape {
        CursorShape::ShapeBlock => 1,
        CursorShape::ShapeHor => 3,
        CursorShape::ShapeVer => 5,
    };
    let steady = i32::from(c.blinkon == 0 || c.blinkoff == 0);
    terminfo_print_num1(tui, K_TERM_SET_CURSOR_STYLE, shape + steady);
}

pub fn tui_mode_change(tui: &mut TUIData, _mode: NvimString, mode_idx: Integer) {
    #[cfg(unix)]
    unsafe {
        if tui.out_isatty && tui.is_starting && !g_st().stdin_isatty {
            let ret = uv_tty_set_mode(tui_output_handle(tui).cast(), UV_TTY_MODE_NORMAL);
            if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"tui_mode_change",
                    1413,
                    true,
                    c"uv_tty_set_mode failed: %s",
                    uv_strerror(ret),
                );
            }
            let ret = uv_tty_set_mode(tui_output_handle(tui).cast(), UV_TTY_MODE_IO);
            if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"tui_mode_change",
                    1417,
                    true,
                    c"uv_tty_set_mode failed: %s",
                    uv_strerror(ret),
                );
            }
        }
    }
    tui_set_mode(tui, mode_idx as ModeShape);
    if tui.is_starting && tui.verbose >= 3 {
        show_verbose_terminfo(tui);
    }
    tui.is_starting = false;
    tui.showing_mode = mode_idx as ModeShape;
}

pub fn tui_grid_scroll(
    tui: &mut TUIData,
    _g: Integer,
    mut startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    endcol: Integer,
    rows: Integer,
    _cols: Integer,
) {
    let top = startrow as i32;
    let bot = (endrow - 1) as i32;
    let left = startcol as i32;
    let right = (endcol - 1) as i32;

    let fullwidth = left == 0 && right == tui.width - 1;
    let full_screen_scroll = fullwidth && top == 0 && bot == tui.height - 1;

    ugrid_scroll(&mut tui.grid, top, bot, left, right, rows as i32);

    let has_lr_margins = tui.has_left_and_right_margin_mode && tui.can_set_lr_margin;
    let can_scroll = tui.can_scroll
        && (full_screen_scroll
            || (tui.can_change_scroll_region
                && ((left == 0 && right == tui.width - 1) || has_lr_margins)));

    if can_scroll {
        if !full_screen_scroll {
            set_scroll_region(tui, top, bot, left, right);
        }
        cursor_goto(tui, top, left);
        update_attrs(tui, 0);

        if rows > 0 {
            if rows == 1 {
                terminfo_out(tui, K_TERM_DELETE_LINE);
            } else {
                terminfo_print_num1(tui, K_TERM_PARM_DELETE_LINE, rows as i32);
            }
        } else if rows == -1 {
            terminfo_out(tui, K_TERM_INSERT_LINE);
        } else {
            terminfo_print_num1(tui, K_TERM_PARM_INSERT_LINE, -(rows as i32));
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
            startrow as i32,
            endrow as i32,
            startcol as i32,
            endcol as i32,
        );
    }
}

pub fn tui_add_url(_tui: &mut TUIData, url: impl crate::strings::IntoCCharPtr) -> i32 {
    let url = url.into_c_char_ptr();
    {
        if url.is_null() {
            return -1;
        }

        let url_cstr = crate::memory::cstr_from_ptr(url);
        if let Some(pos) = tui_st().urls.iter().position(|u| u.as_c_str() == url_cstr) {
            return pos as i32;
        }
        let idx = tui_st().urls.len() as i32;
        tui_st().urls.push(url_cstr.to_owned());
        idx
    }
}

pub fn tui_hl_attr_define(
    tui: &mut TUIData,
    id: Integer,
    mut attrs: HlAttrs,
    cterm_attrs: HlAttrs,
    _info: Array,
) {
    attrs.cterm_ae_attr = cterm_attrs.cterm_ae_attr;
    attrs.cterm_fg_color = cterm_attrs.cterm_fg_color;
    attrs.cterm_bg_color = cterm_attrs.cterm_bg_color;

    let idx = id as usize;
    let tui_attrs = &mut tui.attrs;
    if tui_attrs.len() <= idx {
        tui_attrs.resize(idx + 1, HLATTRS_INIT);
    }
    tui_attrs[idx] = attrs;
}

pub fn tui_bell(tui: &mut TUIData) {
    out(tui, b"\x07");
}

pub fn tui_visual_bell(tui: &mut TUIData) {
    if tui.screen_or_tmux {
        out(tui, b"\x1bg");
    } else {
        out(tui, b"\x1b[?5h");

        flush_buf(tui, FlushBufFinish::FlushBufFinal);
        unsafe { uv_sleep(100) };

        out(tui, b"\x1b[?5l");
    }
    flush_buf(tui, FlushBufFinish::FlushBufFinal);
}

pub fn tui_default_colors_set(
    tui: &mut TUIData,
    rgb_fg: Integer,
    rgb_bg: Integer,
    rgb_sp: Integer,
    cterm_fg: Integer,
    cterm_bg: Integer,
) {
    tui.clear_attrs.rgb_fg_color = rgb_fg as i32;
    tui.clear_attrs.rgb_bg_color = rgb_bg as i32;
    tui.clear_attrs.rgb_sp_color = rgb_sp as i32;
    tui.clear_attrs.cterm_fg_color = cterm_fg as i16;
    tui.clear_attrs.cterm_bg_color = cterm_bg as i16;

    tui.print_attr_id = -1;
    tui.set_default_colors = true;
    invalidate(tui, 0, tui.grid.height, 0, tui.grid.width);
}

pub fn tui_ui_send(tui: &mut TUIData, content: NvimString) {
    if !tui.invalid_regions.is_empty() || tui.bufpos > 0 {
        out(tui, crate::memory::cstr_slice(content.data, content.size));
        return;
    }
    let mut req = [0u64; 64];
    let buf = uv_buf_t::new(content.data, content.size);
    let ret = unsafe {
        uv_write(
            req.as_mut_ptr().cast(),
            tui_output_handle(tui).cast(),
            &raw const buf,
            1,
            None,
        )
    };
    if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
        logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            c"tui_ui_send",
            1569,
            true,
            c"uv_write failed: %s",
            unsafe { uv_strerror(ret) },
        );
    }
    unsafe { uv_run(tui_write_loop(tui), UV_RUN_DEFAULT) };
}

pub fn tui_flush(tui: &mut TUIData) {
    let nrevents = loop_size(unsafe { &mut *tui.loop_ });
    if nrevents > TOO_MANY_EVENTS {
        if log_st().g_min_log_level <= LOGLVL_WRN {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"tui_flush",
                1583,
                true,
                c"TUI event-queue flooded (thread_events=%zu); purging",
                nrevents,
            );
        }
        loop_purge(unsafe { &mut *tui.loop_ });
        tui_busy_stop(tui);
    }

    while let Some(r) = tui.invalid_regions.pop() {
        assert!(r.bot <= tui.grid.height && r.right <= tui.grid.width);

        for row in r.top..r.bot {
            let clear_attr = {
                let cells = &tui.grid.cells;
                cells[row as usize][(r.right - 1) as usize].attr
            };
            let mut clear_col = r.right;
            while clear_col > 0 {
                let cell = {
                    let cells = &tui.grid.cells;
                    cells[row as usize][(clear_col - 1) as usize]
                };
                if !(cell.data == schar_from_ascii(b' ') && cell.attr == clear_attr) {
                    break;
                }
                clear_col -= 1;
            }

            for curcol in r.left..clear_col {
                let (cell, is_double) = {
                    let cells = &tui.grid.cells;
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

    cursor_goto(tui, tui.row, tui.col);
    flush_buf(tui, FlushBufFinish::FlushBufFinal);
}

fn show_verbose_terminfo(tui: &mut TUIData) {
    let mut title_items = [
        cstr_as_obj(c"\n\n--- Terminal info --- {{{\n"),
        cstr_as_obj(c"Title"),
    ];
    let title = Array {
        items: title_items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    let mut str_bytes = terminfo_info_msg(&tui.ti, tui.term.as_deref(), tui.terminfo_found_in_db);
    let str_val = NvimString {
        data: str_bytes.as_mut_ptr().cast(),
        size: str_bytes.len(),
    };
    let mut info_items = [string_obj(str_val)];
    let info = Array {
        items: info_items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    let mut end_fold_items = [cstr_as_obj(c"}}}\n"), cstr_as_obj(c"Title")];
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
    rpc_send_event(g_st().ui_client_channel_id, c"nvim_echo", args);
    drop(str_bytes);
}

pub fn tui_suspend(tui: &mut TUIData) {
    #[cfg(unix)]
    {
        ui_client_detach();
        tui.mouse_enabled_save = tui.mouse_enabled;
        tui.input.primary_device_attr = Some(PrimaryDeviceAttrAction::Suspend);
        terminfo_disable(tui);
    }
    #[cfg(not(unix))]
    let _ = tui;
}

#[cfg(unix)]
fn tui_suspend_cb(tui: &mut TUIData) {
    unsafe {
        let input = tui_input(tui);
        tui_terminal_stop(tui);
        stream_set_blocking(input.in_fd, true);

        let ppid = getppid();
        if getpgid(ppid) == getpgrp() {
            kill(ppid, SIGSTOP);
        }
        kill(0, SIGSTOP);

        tui_terminal_start(tui);
        tui_terminal_after_startup(tui);
        if tui.mouse_enabled_save {
            tui_mouse_on(tui);
        }
        stream_set_blocking(input.in_fd, false);
        ui_client_attach(tui.width, tui.height, tui.term.as_deref(), tui.rgb);
    }
}

pub fn tui_set_title(tui: &mut TUIData, title: NvimString) {
    if !tui.can_set_title {
        return;
    }

    let too_long = title.size > 4096;
    if too_long && log_st().g_min_log_level <= LOGLVL_ERR {
        logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            c"tui_set_title",
            1682,
            true,
            c"set_title: title string too long!",
        );
    }
    if title.size > 0 && !too_long {
        if !tui.title_enabled {
            out(tui, b"\x1b[22;0t");
            tui.title_enabled = true;
        }

        if (tui.buf.len() - tui.bufpos) < title.size + 2 * TERMINFO_SEQ_LIMIT {
            flush_buf(tui, FlushBufFinish::FlushBufFinal);
        }
        terminfo_out(tui, K_TERM_TO_STATUS_LINE);
        out(tui, crate::memory::cstr_slice(title.data, title.size));
        terminfo_out(tui, K_TERM_FROM_STATUS_LINE);
    } else if tui.title_enabled {
        out(tui, b"\x1b[23;0t");
        tui.title_enabled = false;
    }
}

pub fn tui_set_icon(_tui: &mut TUIData, _icon: NvimString) {}

pub fn tui_screenshot(tui: &mut TUIData, path: NvimString) {
    let Some(f) = crate::os::fs::CFile::from_raw(unsafe { fopen(path.data, c"w".as_ptr()) }) else {
        return;
    };

    flush_buf(tui, FlushBufFinish::FlushBufFinal);
    tui.grid.row = 0;
    tui.grid.col = 0;

    let fp = f.as_ptr();
    tui.screenshot = Some(f);
    let header = format!("{},{}\n", tui.grid.height, tui.grid.width);
    unsafe { fwrite(header.as_ptr().cast(), header.len(), 1, fp) };
    terminfo_out(tui, K_TERM_CLEAR_SCREEN);
    for i in 0..tui.grid.height {
        cursor_goto(tui, i, 0);
        for j in 0..tui.grid.width {
            let cell = {
                let cells = &tui.grid.cells;
                cells[i as usize][j as usize]
            };
            let mut buf = [0u8; MAX_SCHAR_SIZE];
            let len = crate::grid::schar_get_bytes(&mut buf, cell.data);
            print_cell(tui, &buf[..len], cell.attr);
        }
    }
    flush_buf(tui, FlushBufFinish::FlushBufFinal);
    tui.screenshot = None;
}

pub fn tui_option_set(tui: &mut TUIData, name: NvimString, value: Object) {
    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"mousemoveevent") {
        if tui.mouse_move_enabled != value.data.boolean() {
            if tui.mouse_enabled {
                tui_mouse_off(tui);
                tui.mouse_move_enabled = value.data.boolean();
                tui_mouse_on(tui);
            } else {
                tui.mouse_move_enabled = value.data.boolean();
            }
        }
    } else if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"termguicolors") {
        tui.rgb = value.data.boolean();
        tui.print_attr_id = -1;
        invalidate(tui, 0, tui.grid.height, 0, tui.grid.width);

        if g_st().ui_client_channel_id != 0 {
            let mut args_items = [cstr_as_obj(c"rgb"), boolean_obj(value.data.boolean())];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            rpc_send_event(g_st().ui_client_channel_id, c"nvim_ui_set_option", args);
        }
    } else if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"ttimeout") {
        tui.input.ttimeout = value.data.boolean();
    } else if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"ttimeoutlen") {
        tui.input.ttimeoutlen = value.data.integer() as OptInt;
    } else if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"verbose") {
        tui.verbose = value.data.integer();
    } else if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"termsync") {
        tui.sync_output = value.data.boolean();
    }
}

pub fn tui_chdir(_tui: &mut TUIData, path: NvimString) {
    let err = unsafe { uv_chdir(path.data) };
    if err != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
        logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            c"tui_chdir",
            1777,
            true,
            c"Failed to chdir to %s: %s",
            path.data,
            unsafe { uv_strerror(err) },
        );
    }
}

pub fn tui_raw_line(
    tui: &mut TUIData,
    _g: Integer,
    linerow: Integer,
    startcol: Integer,
    endcol: Integer,
    clearcol: Integer,
    clearattr: Integer,
    flags: LineFlags,
    chunk: *const Schar,
    attrs: *const Sattr,
) {
    {
        let attr_count = tui.attrs.len();
        let cells = &mut tui.grid.cells;
        let row_cells = &mut cells[linerow as usize];
        for c in startcol..endcol {
            let idx = (c - startcol) as usize;
            let cell = &mut row_cells[c as usize];
            cell.data = unsafe { *chunk.wrapping_add(idx) };
            assert!((unsafe { *attrs.wrapping_add(idx) } as usize) < attr_count);
            cell.attr = unsafe { *attrs.wrapping_add(idx) };
        }
    }
    for curcol in (startcol as i32)..(endcol as i32) {
        let (cell, is_double) = {
            let cells = &tui.grid.cells;
            let row_cells = &cells[linerow as usize];
            (
                row_cells[curcol as usize],
                curcol < (endcol as i32) - 1 && row_cells[(curcol + 1) as usize].data == 0,
            )
        };
        print_cell_at_pos(tui, linerow as i32, curcol, cell, is_double);
    }

    if clearcol > endcol {
        ugrid_clear_chunk(
            &mut tui.grid,
            linerow as i32,
            endcol as i32,
            clearcol as i32,
            clearattr as Sattr,
        );
        clear_region(
            tui,
            linerow as i32,
            (linerow as i32) + 1,
            endcol as i32,
            clearcol as i32,
            clearattr as i32,
        );
    }

    if (flags & K_LINE_FLAG_WRAP) != 0
        && tui.width == tui.grid.width
        && (linerow as i32) + 1 < tui.grid.height
    {
        if endcol as i32 != tui.grid.width {
            let last_col = (tui.grid.width - 1) as usize;
            let (target_col, cell, size) = {
                let cells = &tui.grid.cells;
                let row_cells = &cells[linerow as usize];
                let size = if row_cells[last_col].data == 0 { 2 } else { 1 };
                let target_col = tui.grid.width - size;
                (target_col, row_cells[target_col as usize], size)
            };
            print_cell_at_pos(tui, linerow as i32, target_col, cell, size == 2);
        }

        final_column_wrap(tui);
    }
}

fn invalidate(tui: &mut TUIData, top: i32, bot: i32, left: i32, right: i32) {
    if let Some(intersects) = tui
        .invalid_regions
        .iter_mut()
        .find(|r| !(top > r.bot || bot < r.top) && !(left > r.right || right < r.left))
    {
        intersects.top = top.min(intersects.top);
        intersects.bot = bot.max(intersects.bot);
        intersects.left = left.min(intersects.left);
        intersects.right = right.max(intersects.right);
    } else {
        tui.invalid_regions.push(Rect {
            top,
            bot,
            left,
            right,
        });
    }
}

pub fn tui_set_size(tui: &mut TUIData, width: i32, height: i32) {
    tui.pending_resize_events += 1;
    tui.width = width;
    tui.height = height;
    ui_client_set_size(width, height);
}

fn parse_sscanf_int(bytes: Option<&[u8]>, out: &mut i32) -> bool {
    let Some(bytes) = bytes else {
        return false;
    };
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
        && let Ok(v) = s.parse::<i32>()
    {
        *out = v;
        return true;
    }
    false
}

pub fn tui_guess_size(tui: &mut TUIData) {
    let mut width: i32 = 0;
    let mut height: i32 = 0;

    let mut got_size = false;
    if tui.out_isatty
        && unsafe { uv_tty_get_winsize(tui_output_handle(tui).cast(), &mut width, &mut height) }
            == 0
    {
        got_size = true;
    }

    if !got_size {
        let lines = os_getenv_cstr(c"LINES");
        let columns = os_getenv_cstr(c"COLUMNS");
        if parse_sscanf_int(lines.as_deref().map(CStr::to_bytes), &mut height)
            && parse_sscanf_int(columns.as_deref().map(CStr::to_bytes), &mut width)
        {
            got_size = true;
        }
    }

    if !got_size {
        height = tui.ti.lines;
        width = tui.ti.columns;
    }

    if width <= 0 || height <= 0 {
        width = DFLT_COLS;
        height = DFLT_ROWS;
    }

    tui_set_size(tui, width, height);
}

fn out(tui: &mut TUIData, bytes: &[u8]) {
    let len = bytes.len();
    let available = tui.buf.len() - tui.bufpos;
    if len > available {
        flush_buf(tui, FlushBufFinish::FlushBufPartial);
        if len > tui.buf.len() {
            tui.bufpos = len;
            flush_buf_raw(tui, FlushBufFinish::FlushBufPartial, Some(bytes));
            return;
        }
    }

    tui.buf[tui.bufpos..tui.bufpos + len].copy_from_slice(bytes);
    tui.bufpos += len;
}

fn out_len(tui: &mut TUIData, bytes: Option<&[u8]>) {
    if let Some(bytes) = bytes {
        out(tui, bytes);
    }
}

fn out_printf_str(tui: &mut TUIData, limit: usize, formatted: &str) {
    assert!(limit <= tui.buf.len());
    let available = tui.buf.len() - tui.bufpos;
    if available < limit {
        flush_buf(tui, FlushBufFinish::FlushBufPartial);
    }
    let bytes = formatted.as_bytes();
    let copy_len = bytes.len().min(limit.saturating_sub(1));
    tui.buf[tui.bufpos..tui.bufpos + copy_len].copy_from_slice(&bytes[..copy_len]);
    if limit > 0 {
        tui.buf[tui.bufpos + copy_len] = 0;
    }
    tui.bufpos += bytes.len();
}

fn terminfo_out(tui: &mut TUIData, what: TerminfoDef) {
    let mut null_params = [TPVAR::default(); 9];
    terminfo_print(tui, what, &mut null_params);
}

fn terminfo_print_num1(tui: &mut TUIData, what: TerminfoDef, num: i32) {
    terminfo_print_num(tui, what, num, 0, 0);
}

fn terminfo_print_num2(tui: &mut TUIData, what: TerminfoDef, num1: i32, num2: i32) {
    terminfo_print_num(tui, what, num1, num2, 0);
}

fn terminfo_print_num(tui: &mut TUIData, what: TerminfoDef, num1: i32, num2: i32, num3: i32) {
    let mut params = [TPVAR::default(); 9];
    params[0].num = i64::from(num1);
    params[1].num = i64::from(num2);
    params[2].num = i64::from(num3);
    terminfo_print(tui, what, &mut params);
}

fn terminfo_print(tui: &mut TUIData, what: TerminfoDef, params: &mut [TPVAR; 9]) {
    if what >= K_TERM_COUNT {
        std::process::abort();
    }

    let Some(ref str_cow) = tui.ti.defs[what] else {
        return;
    };
    let str_bytes = str_cow.to_bytes();
    if str_bytes.is_empty() {
        return;
    }

    if tui.buf.len() - tui.bufpos > TERMINFO_SEQ_LIMIT {
        let mut copy_params = *params;
        let out_slice = unsafe {
            std::slice::from_raw_parts_mut(
                tui.buf.as_mut_ptr().wrapping_add(tui.bufpos).cast::<u8>(),
                tui.buf.len() - tui.bufpos,
            )
        };
        let len = terminfo_fmt(out_slice, str_bytes, &mut copy_params);
        if len > 0 {
            tui.bufpos += len;
            return;
        }
    }

    flush_buf(tui, FlushBufFinish::FlushBufPartial);
    if let Some(ref str_cow) = tui.ti.defs[what] {
        let str_bytes = str_cow.to_bytes();
        let out_slice = unsafe {
            std::slice::from_raw_parts_mut(
                tui.buf.as_mut_ptr().wrapping_add(tui.bufpos).cast::<u8>(),
                tui.buf.len() - tui.bufpos,
            )
        };
        let len = terminfo_fmt(out_slice, str_bytes, params);
        if len > 0 {
            tui.bufpos += len;
        }
    }
}

fn terminfo_set_if_empty(tui: &mut TUIData, str_: TerminfoDef, val: &'static CStr) {
    if tui.ti.defs[str_].is_none() {
        tui.ti.defs[str_] = Some(std::borrow::Cow::Borrowed(val));
    }
}

fn terminfo_set_str(tui: &mut TUIData, str_: TerminfoDef, val: &'static CStr) {
    tui.ti.defs[str_] = Some(std::borrow::Cow::Borrowed(val));
}

fn term_has_truecolor(tui: &mut TUIData, colorterm: Option<&[u8]>) -> bool {
    if colorterm == Some(b"truecolor") || colorterm == Some(b"24bit") {
        return true;
    }

    if tui.ti.tc || tui.ti.rgb {
        return true;
    }

    tui.ti.defs[K_TERM_SET_RGB_FOREGROUND].is_some()
        && tui.ti.defs[K_TERM_SET_RGB_BACKGROUND].is_some()
}

fn patch_terminfo_bugs(
    tui: &mut TUIData,
    term: Option<&[u8]>,
    colorterm: Option<&[u8]>,
    vte_version: i32,
    konsolev: i32,
    iterm_env: bool,
    nsterm: bool,
) {
    let xterm_version_cstr = os_getenv_cstr(c"XTERM_VERSION");
    let xterm_version = xterm_version_cstr
        .as_ref()
        .map_or(ptr::null(), |c| c.as_ptr());
    let xterm = terminfo_is_term_family(term, c"xterm") || nsterm;
    let hterm = terminfo_is_term_family(term, c"hterm");
    let kitty = terminfo_is_term_family(term, c"xterm-kitty");
    let linuxvt = terminfo_is_term_family(term, c"linux");
    let bsdvt = terminfo_is_bsd_console(term);
    let rxvt = terminfo_is_term_family(term, c"rxvt");
    let teraterm = terminfo_is_term_family(term, c"teraterm");
    let putty = terminfo_is_term_family(term, c"putty");
    let screen = terminfo_is_term_family(term, c"screen");
    let tmux = terminfo_is_term_family(term, c"tmux") || os_env_exists(c"TMUX", true);
    let st = terminfo_is_term_family(term, c"st");
    let gnome = terminfo_is_term_family(term, c"gnome") || terminfo_is_term_family(term, c"vte");
    let iterm = terminfo_is_term_family(term, c"iterm")
        || terminfo_is_term_family(term, c"iterm2")
        || terminfo_is_term_family(term, c"iTerm.app")
        || terminfo_is_term_family(term, c"iTerm2.app");
    let alacritty = terminfo_is_term_family(term, c"alacritty");
    let foot = terminfo_is_term_family(term, c"foot");
    let iterm_pretending_xterm = xterm && iterm_env;
    let gnome_pretending_xterm = xterm && slice_contains(colorterm, b"gnome-terminal");
    let mate_pretending_xterm = xterm && slice_contains(colorterm, b"mate-terminal");
    let true_xterm = xterm && !xterm_version.is_null() && !bsdvt;
    let cygwin = terminfo_is_term_family(term, c"cygwin");
    let ghostty = terminfo_is_term_family(term, c"xterm-ghostty");

    if let Some(fix_normal) = tui.ti.defs[K_TERM_CURSOR_NORMAL].clone() {
        let mut bytes = fix_normal.to_bytes();
        let mut changed = false;
        if let Some(rest) = bytes.strip_prefix(b"\x1b[?12l") {
            bytes = rest;
            changed = true;
        }
        if linuxvt && let Some(rest) = bytes.strip_suffix(LINUXSET0C) {
            bytes = rest;
            changed = true;
        }
        if changed {
            tui.ti.defs[K_TERM_CURSOR_NORMAL] = Some(std::borrow::Cow::Owned(
                std::ffi::CString::new(bytes).unwrap(),
            ));
        }
    }
    if let Some(fix_invisible) = tui.ti.defs[K_TERM_CURSOR_INVISIBLE].clone()
        && linuxvt
        && let Some(rest) = fix_invisible.to_bytes().strip_suffix(LINUXSET1C)
    {
        tui.ti.defs[K_TERM_CURSOR_INVISIBLE] = Some(std::borrow::Cow::Owned(
            std::ffi::CString::new(rest).unwrap(),
        ));
    }

    if tmux || screen || kitty {
        tui.ti.back_color_erase = false;
    }

    if xterm || hterm {
        if !hterm {
            terminfo_set_if_empty(tui, K_TERM_TO_STATUS_LINE, c"\x1b]0;");
            terminfo_set_if_empty(tui, K_TERM_FROM_STATUS_LINE, c"\x07");
        }
        terminfo_set_if_empty(tui, K_TERM_ENTER_ITALICS_MODE, c"\x1b[3m");
        terminfo_set_if_empty(tui, K_TERM_SET_LR_MARGIN, c"\x1b[%i%p1%d;%p2%ds");
        #[cfg(windows)]
        {
            terminfo_set_str(tui, K_TERM_CURSOR_DOWN, c"\x1b[B");
        }
    } else if rxvt {
        terminfo_set_if_empty(tui, K_TERM_ENTER_ITALICS_MODE, c"\x1b[3m");
        terminfo_set_if_empty(tui, K_TERM_TO_STATUS_LINE, c"\x1b]2");
        terminfo_set_if_empty(tui, K_TERM_FROM_STATUS_LINE, c"\x07");
        terminfo_set_str(tui, K_TERM_ENTER_CA_MODE, c"\x1b[?1049h");
        terminfo_set_str(tui, K_TERM_EXIT_CA_MODE, c"\x1b[?1049l");
        terminfo_set_str(tui, K_TERM_RESET_CURSOR_STYLE, c"\x1b[2 q");
        terminfo_set_str(tui, K_TERM_SET_CURSOR_STYLE, c"\x1b[%p1%d q");
    } else if screen {
        terminfo_set_if_empty(tui, K_TERM_TO_STATUS_LINE, c"\x1b_");
        terminfo_set_if_empty(tui, K_TERM_FROM_STATUS_LINE, c"\x1b\\");
    } else if tmux {
        terminfo_set_if_empty(tui, K_TERM_TO_STATUS_LINE, c"\x1b_");
        terminfo_set_if_empty(tui, K_TERM_FROM_STATUS_LINE, c"\x1b\\");
        terminfo_set_if_empty(tui, K_TERM_ENTER_ITALICS_MODE, c"\x1b[3m");
    } else if terminfo_is_term_family(term, c"interix") {
        terminfo_set_if_empty(tui, K_TERM_CARRIAGE_RETURN, c"\x0d");
    } else if linuxvt {
        terminfo_set_if_empty(tui, K_TERM_PARM_UP_CURSOR, c"\x1b[%p1%dA");
        terminfo_set_if_empty(tui, K_TERM_PARM_DOWN_CURSOR, c"\x1b[%p1%dB");
        terminfo_set_if_empty(tui, K_TERM_PARM_RIGHT_CURSOR, c"\x1b[%p1%dC");
        terminfo_set_if_empty(tui, K_TERM_PARM_LEFT_CURSOR, c"\x1b[%p1%dD");
    } else if putty {
        // No bugs in the vanilla terminfo for our purposes.
    } else if iterm {
        terminfo_set_str(tui, K_TERM_ENTER_CA_MODE, c"\x1b[?1049h");
        terminfo_set_str(tui, K_TERM_EXIT_CA_MODE, c"\x1b[?1049l");
        terminfo_set_if_empty(tui, K_TERM_ENTER_ITALICS_MODE, c"\x1b[3m");
    } else if st {
        // No bugs in the vanilla terminfo for our purposes.
    }

    if tui.ti.max_colors < 256 {
        if true_xterm || iterm || iterm_pretending_xterm {
            tui.ti.max_colors = 256;
            terminfo_set_str(tui, K_TERM_SET_A_FOREGROUND, XTERM_SETAF_256_COLON);
            terminfo_set_str(tui, K_TERM_SET_A_BACKGROUND, XTERM_SETAB_256_COLON);
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
            || slice_contains(colorterm, b"256")
            || slice_contains(term, b"256")
        {
            tui.ti.max_colors = 256;
            terminfo_set_str(tui, K_TERM_SET_A_FOREGROUND, XTERM_SETAF_256);
            terminfo_set_str(tui, K_TERM_SET_A_BACKGROUND, XTERM_SETAB_256);
        }
    }
    if tui.ti.max_colors < 16 && colorterm.is_some() {
        tui.ti.max_colors = 16;
        terminfo_set_if_empty(tui, K_TERM_SET_A_FOREGROUND, XTERM_SETAF_16);
        terminfo_set_if_empty(tui, K_TERM_SET_A_BACKGROUND, XTERM_SETAB_16);
    }

    if st || (vte_version != 0 && vte_version < 3900) || konsolev != 0 {
        tui.ti.defs[K_TERM_RESET_CURSOR_STYLE] = None;
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
            || (linuxvt && (!xterm_version.is_null() || colorterm.is_some())))
    {
        terminfo_set_str(tui, K_TERM_SET_CURSOR_STYLE, c"\x1b[%p1%d q");
        terminfo_set_str(tui, K_TERM_RESET_CURSOR_STYLE, c"\x1b[0 q");
    } else if linuxvt {
        terminfo_set_str(
            tui,
            K_TERM_SET_CURSOR_STYLE,
            c"\x1b[?%?%p1%{2}%<%t%{8}%e%p1%{2}%=%t%{112}%e%p1%{3}%=%t%{4}%e%p1%{4}%=%t%{4}%e%p1%{5}%=%t%{2}%e%p1%{6}%=%t%{2}%e%{0}%;%dc",
        );
        terminfo_set_str(tui, K_TERM_RESET_CURSOR_STYLE, c"\x1b[?c");
    } else if !bsdvt && xterm {
        terminfo_set_if_empty(tui, K_TERM_SET_CURSOR_STYLE, c"\x1b[%p1%d q");
        terminfo_set_if_empty(tui, K_TERM_RESET_CURSOR_STYLE, c"\x1b[2 q");
    }
}

fn augment_terminfo(
    tui: &mut TUIData,
    term: Option<&[u8]>,
    vte_version: i32,
    konsolev: i32,
    weztermv: Option<&[u8]>,
    iterm_env: bool,
    nsterm: bool,
) {
    {
        let xterm_version_cstr = os_getenv_cstr(c"XTERM_VERSION");
        let xterm_version = xterm_version_cstr
            .as_ref()
            .map_or(ptr::null(), |c| c.as_ptr());
        let xterm = terminfo_is_term_family(term, c"xterm") || nsterm;
        let hterm = terminfo_is_term_family(term, c"hterm");
        let bsdvt = terminfo_is_bsd_console(term);
        let dtterm = terminfo_is_term_family(term, c"dtterm");
        let rxvt = terminfo_is_term_family(term, c"rxvt");
        let teraterm = terminfo_is_term_family(term, c"teraterm");
        let putty = terminfo_is_term_family(term, c"putty");
        let screen = terminfo_is_term_family(term, c"screen");
        let tmux = terminfo_is_term_family(term, c"tmux") || os_env_exists(c"TMUX", true);
        let st = terminfo_is_term_family(term, c"st");
        let iterm = terminfo_is_term_family(term, c"iterm")
            || terminfo_is_term_family(term, c"iterm2")
            || terminfo_is_term_family(term, c"iTerm.app")
            || terminfo_is_term_family(term, c"iTerm2.app");
        let alacritty = terminfo_is_term_family(term, c"alacritty");
        let kitty = terminfo_is_term_family(term, c"xterm-kitty");
        let iterm_pretending_xterm = xterm && iterm_env;
        let true_xterm = xterm && !xterm_version.is_null() && !bsdvt;

        if dtterm || xterm || konsolev != 0 || teraterm || rxvt {
            tui.can_resize_screen = true;
        }

        if putty || xterm || hterm || rxvt {
            tui.terminfo_ext.reset_scroll_region = Some(b"\x1b[r");
        }

        tui.terminfo_ext.enter_altfont_mode = Some(b"\x1b[11m");

        let has_colon_rgb =
            !tmux && !screen && vte_version == 0 && (iterm || iterm_pretending_xterm || true_xterm);

        if tui.ti.defs[K_TERM_SET_RGB_FOREGROUND].is_none() {
            tui.ti.defs[K_TERM_SET_RGB_FOREGROUND] =
                Some(std::borrow::Cow::Borrowed(if has_colon_rgb {
                    c"\x1b[38:2:%p1%d:%p2%d:%p3%dm"
                } else {
                    c"\x1b[38;2;%p1%d;%p2%d;%p3%dm"
                }));
        }
        if tui.ti.defs[K_TERM_SET_RGB_BACKGROUND].is_none() {
            tui.ti.defs[K_TERM_SET_RGB_BACKGROUND] =
                Some(std::borrow::Cow::Borrowed(if has_colon_rgb {
                    c"\x1b[48:2:%p1%d:%p2%d:%p3%dm"
                } else {
                    c"\x1b[48;2;%p1%d;%p2%d;%p3%dm"
                }));
        }

        if tui.ti.defs[K_TERM_SET_CURSOR_COLOR].is_none() {
            if iterm || iterm_pretending_xterm {
                tui.ti.defs[K_TERM_SET_CURSOR_COLOR] = Some(std::borrow::Cow::Borrowed(if tmux {
                    c"\x1bPtmux;\x1b\x1b]Pl%p1%06x\x1b\\\x1b\\"
                } else {
                    c"\x1b]Pl%p1%06x\x1b\\"
                }));
            } else if (xterm || hterm || rxvt || tmux || alacritty || st)
                && (vte_version == 0 || vte_version >= 3900)
            {
                tui.ti.defs[K_TERM_SET_CURSOR_COLOR] =
                    Some(std::borrow::Cow::Borrowed(c"\x1b]12;%p1%s\x07"));
            }
        }
        if let Some(ref set_cursor_color) = tui.ti.defs[K_TERM_SET_CURSOR_COLOR] {
            tui.set_cursor_color_as_str = slice_contains(Some(set_cursor_color.to_bytes()), b"%s");
            terminfo_set_if_empty(tui, K_TERM_RESET_CURSOR_COLOR, c"\x1b]112\x07");
        }

        if tui.ti.defs[K_TERM_TO_STATUS_LINE].is_some()
            && tui.ti.defs[K_TERM_FROM_STATUS_LINE].is_some()
        {
            tui.can_set_title = true;
        }

        tui.terminfo_ext.enable_focus_reporting = Some(if rxvt {
            b"\x1b[?1004h\x1b]777;focus;on\x07"
        } else {
            b"\x1b[?1004h"
        });
        tui.terminfo_ext.disable_focus_reporting = Some(if rxvt {
            b"\x1b[?1004l\x1b]777;focus;off\x07"
        } else {
            b"\x1b[?1004l"
        });

        if tui.ti.defs[K_TERM_SET_UNDERLINE_STYLE].is_none() {
            let wezterm_ok = weztermv.is_some_and(|v| v > &b"20210203-095643"[..]);
            if vte_version >= 5102 || konsolev >= 221170 || tui.ti.su || wezterm_ok {
                tui_enable_extended_underline(tui);
            }
        } else {
            tui_enable_extended_underline(tui);
        }

        if kitty || (vte_version != 0 && vte_version < 5400) {
            tui.input.key_encoding = K_KEY_ENCODING_LEGACY;
        } else {
            tui.input.key_encoding = K_KEY_ENCODING_XTERM;
        }
    }
}

#[inline]
fn should_invisible(tui: &TUIData) -> bool {
    tui.busy || tui.want_invisible
}

fn flush_buf(tui: &mut TUIData, finish: FlushBufFinish) {
    flush_buf_raw(tui, finish, None);
}

fn flush_buf_raw(tui: &mut TUIData, finish: FlushBufFinish, buf_to_flush: Option<&[u8]>) {
    let should_be_invisible = should_invisible(tui);

    if tui.bufpos == 0
        && tui.is_invisible == should_be_invisible
        && !(finish == FlushBufFinish::FlushBufFinal && tui.sync_output_active)
    {
        return;
    }

    let mut req = [0u64; 64];
    let mut pre = [0u8; 32];
    let mut post = [0u8; 32];
    let mut pre_len = 0usize;
    let mut post_len = 0usize;
    let mut null_params = [TPVAR::default(); 9];

    if !tui.sync_output_active && tui.sync_output && tui.has_sync_mode {
        let sync_start = b"\x1b[?2026h\0";
        pre[..sync_start.len()].copy_from_slice(sync_start);
        pre_len += sync_start.len() - 1;
        tui.sync_output_active = true;
    } else if !tui.sync_output_active && !tui.is_invisible {
        tui.is_invisible = true;
        if let Some(ref str_) = tui.ti.defs[K_TERM_CURSOR_INVISIBLE] {
            pre_len += terminfo_fmt(&mut pre[pre_len..], str_.to_bytes(), &mut null_params);
        }
    }

    if finish == FlushBufFinish::FlushBufFinal && tui.sync_output_active {
        let sync_end = b"\x1b[?2026l\0";
        post[..sync_end.len()].copy_from_slice(sync_end);
        post_len += sync_end.len() - 1;
        tui.sync_output_active = false;
    }

    if !tui.sync_output_active {
        let mut str_: Option<&CStr> = None;
        if tui.is_invisible && !should_be_invisible {
            str_ = tui.ti.defs[K_TERM_CURSOR_NORMAL].as_deref();
            tui.is_invisible = false;
        } else if !tui.is_invisible && should_be_invisible {
            str_ = tui.ti.defs[K_TERM_CURSOR_INVISIBLE].as_deref();
            tui.is_invisible = true;
        }
        if let Some(str_) = str_ {
            post_len += terminfo_fmt(&mut post[post_len..], str_.to_bytes(), &mut null_params);
        }
    }

    let mid_base = buf_to_flush.map_or(tui.buf.as_mut_ptr(), |b| b.as_ptr().cast_mut());
    let bufs = [
        uv_buf_t::new(pre.as_mut_ptr().cast(), pre_len),
        uv_buf_t::new(mid_base.cast(), tui.bufpos),
        uv_buf_t::new(post.as_mut_ptr().cast(), post_len),
    ];

    if let Some(ref screenshot) = tui.screenshot {
        let fp = screenshot.as_ptr();
        for b in &bufs {
            unsafe { fwrite(b.base.cast(), b.len as _, 1, fp) };
        }
    } else {
        let mut nbufs = bufs.len();
        while nbufs > 0 && bufs[nbufs - 1].len == 0 {
            nbufs -= 1;
        }
        if nbufs > 0 {
            let ret = unsafe {
                uv_write(
                    req.as_mut_ptr().cast(),
                    tui_output_handle(tui).cast(),
                    bufs.as_ptr(),
                    nbufs as u32,
                    None,
                )
            };
            if ret != 0 && log_st().g_min_log_level <= LOGLVL_ERR {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"flush_buf",
                    2660,
                    true,
                    c"uv_write failed: %s",
                    unsafe { uv_strerror(ret) },
                );
            }
            unsafe { uv_run(tui_write_loop(tui), UV_RUN_DEFAULT) };
        }
    }
    tui.bufpos = 0;
}

fn tui_get_stty_erase(input: &mut TermInput) -> *const c_char {
    #[cfg(unix)]
    {
        let mut t = unix_termios::termios::new();
        if unsafe { unix_termios::tcgetattr(input.in_fd, &raw mut t) } != -1 {
            tui_st().stty_erase_buf[0] = t.c_cc[unix_termios::VERASE] as c_char;
            tui_st().stty_erase_buf[1] = NUL;
            if log_st().g_min_log_level <= LOGLVL_DBG {
                logmsg!(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"tui_get_stty_erase",
                    2684,
                    true,
                    c"stty/termios:erase=%s",
                    (&raw const tui_st().stty_erase_buf).cast::<c_char>(),
                );
            }
        }
    }
    #[cfg(windows)]
    {
        let uv_ptr = &raw mut input.read_stream.s.uv;
        let mut dw_mode: u32 = 0;
        if unsafe { uv_handle_get_type(uv_ptr.cast()) } == UV_TTY {
            let tty_handle = unsafe {
                *uv_ptr
                    .cast::<u8>()
                    .wrapping_add(uv_handle_size(UV_STREAM))
                    .cast::<HANDLE>()
            };
            if unsafe { GetConsoleMode(tty_handle, &raw mut dw_mode) } != 0
                && (dw_mode & ENABLE_VIRTUAL_TERMINAL_INPUT) != 0
            {
                tui_st().stty_erase_buf[0] = 0x7f;
                tui_st().stty_erase_buf[1] = NUL;
            }
        }
    }
    (&raw const tui_st().stty_erase_buf).cast::<c_char>()
}

pub(crate) fn tui_tk_ti_getstr<'a>(
    name: &str,
    value: Option<&'a CStr>,
    input: &mut TermInput,
) -> Option<&'a [u8]> {
    let val_ptr = value.map_or(ptr::null(), CStr::as_ptr);
    if tui_st().stty_erase_ptr.is_null() {
        tui_st().stty_erase_ptr = tui_get_stty_erase(input);
    }

    if name == "key_backspace" {
        if log_st().g_min_log_level <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"tui_tk_ti_getstr",
                2709,
                true,
                c"libtermkey:kbs=%s",
                val_ptr,
            );
        }
        if unsafe { *tui_st().stty_erase_ptr } != 0 {
            return Some(crate::memory::cstr_as_bytes(tui_st().stty_erase_ptr));
        }
    } else if name == "key_dc" {
        if log_st().g_min_log_level <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"tui_tk_ti_getstr",
                2714,
                true,
                c"libtermkey:kdch1=%s",
                val_ptr,
            );
        }
        if let Some(val_cstr) = value
            && crate::memory::cstr_as_opt_bytes(tui_st().stty_erase_ptr)
                == Some(val_cstr.to_bytes())
        {
            return Some(if unsafe { *tui_st().stty_erase_ptr } == DEL {
                b"\x08"
            } else {
                b"\x7f"
            });
        }
    } else if name == "key_mouse" {
        if log_st().g_min_log_level <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"tui_tk_ti_getstr",
                2720,
                true,
                c"libtermkey:kmous=%s",
                val_ptr,
            );
        }
        return None;
    }

    value.map(CStr::to_bytes)
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
            Some(TermdefFieldTarget::Def(K_TERM_ENTER_CA_MODE))
        ));
        assert!(matches!(
            lookup_termdef_field(b"key_home"),
            Some(TermdefFieldTarget::KeyArr(K_TERM_KEY_HOME))
        ));
        assert!(matches!(
            lookup_termdef_field(b"key_f63"),
            Some(TermdefFieldTarget::FKey(62))
        ));
        assert!(lookup_termdef_field(b"key_f64").is_none());

        let mut v = 0;
        assert!(parse_sscanf_int(Some(b"  120cols"), &mut v));
        assert_eq!(v, 120);
        assert!(!parse_sscanf_int(Some(b"abc"), &mut v));
    }
}
