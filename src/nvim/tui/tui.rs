//! Terminal UI functions invoked by the UI client process (`src/nvim/tui/tui.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_unsafe,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::module_inception,
    clippy::upper_case_acronyms,
    clippy::collapsible_if
)]

use crate::globals::*;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;

use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::charset::getdigits_int;
use crate::cursor_shape::{CursorShape, SHAPE_IDX_COUNT, SHAPE_IDX_N, cursorentry_T, shape_table};
use crate::event::r#loop::{Loop, loop_poll_events, loop_purge, loop_queues, loop_size};
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
use crate::event::rstream::rstream_fields;
use crate::event::signal::{
    SignalWatcher, signal_watcher_close, signal_watcher_init, signal_watcher_start,
    signal_watcher_stop,
};
use crate::event::stream::{
    UV_STREAM, UV_TTY, stream_set_blocking, stream_uv_ptr, uv_handle_t, uv_loop_t, uv_pipe_t,
    uv_stream_t, uv_tty_t,
};
use crate::event::time::uv_timer_t;
use crate::event::wstream::{uv_buf_t, uv_write_t};
use crate::grid::{schar_cache_clear_if_full, schar_get, schar_get_ascii};
use crate::kvec::KVec;
use crate::map::{HlAttrs, Set, cstr_t, mh_put_cstr_t};
use crate::map_glyph_cache::{MHPutStatus, MapHash, NvimString};
use crate::memory::{
    Arena, arena_finish, arena_mem_free, arena_memdupz, arena_strdup, strequal, xcalloc, xfree,
    xrealloc, xstrdup,
};
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::os::env::{os_env_exists, os_getenv, os_getenv_noalloc, os_setenv};
use crate::os::input::os_isatty;
#[cfg(windows)]
use crate::os::os_win_console::os_tty_guess_term;
use crate::os::time::os_hrtime;
use crate::tui::input::{
    OptInt, TermInput, input_loop_ptr, input_prefix, input_read_stream, input_suffix,
    kKeyEncodingKitty, kKeyEncodingLegacy, kKeyEncodingXterm, tinput_destroy, tinput_init,
    tinput_start, tinput_stop,
};
use crate::tui::terminfo::*;
use crate::tui::termkey::driver_ti::{
    TerminfoEntry, kTermCount, kTermKeyCount, kTerminfoFuncKeyMax,
};
use crate::tui::ugrid::{
    UCell, UGrid, sattr_T, schar_T, schar_from_ascii, ugrid_clear, ugrid_clear_chunk, ugrid_free,
    ugrid_goto, ugrid_init, ugrid_resize, ugrid_scroll,
};

pub type Integer = i64;
pub type TermMode = c_int;
pub type TermModeState = c_int;
pub type ModeShape = c_int;
pub type LineFlags = c_int;
pub type TerminfoDef = usize;
type FILE = c_void;
type LuaRetMode = c_int;
type uv_timer_cb = Option<unsafe extern "C" fn(handle: *mut uv_timer_t)>;
type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;
type uv_write_cb = Option<unsafe extern "C" fn(req: *mut uv_write_t, status: c_int)>;

const ARENA_EMPTY: Arena = Arena {
    cur_blk: ptr::null_mut(),
    pos: 0,
    size: 0,
};

const kRetObject: LuaRetMode = 0;

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

const kLineFlagWrap: LineFlags = 1;

const EXIT_TIMEOUT_MS: i64 = 1000;
const OUTBUF_SIZE: usize = 0xffff;
const TOO_MANY_EVENTS: usize = 1_000_000;
const TERMINFO_SEQ_LIMIT: usize = 128;
const MAX_SCHAR_SIZE: usize = 32;
const STDOUT_FILENO: c_int = 1;
const DFLT_COLS: c_int = 80;
const DFLT_ROWS: c_int = 24;
const NUL: c_char = 0;
const DEL: c_char = 0x7f;

const UV_RUN_DEFAULT: c_int = 0;
const UV_TTY_MODE_NORMAL: c_int = 0;
const UV_TTY_MODE_IO: c_int = 2;
#[cfg(not(windows))]
const UV_EINTR: c_int = -4;
const SIGWINCH: c_int = 28;
#[cfg(target_os = "linux")]
const SIGSTOP: c_int = 19;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGSTOP: c_int = 17;

#[cfg(windows)]
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

const LOGLVL_DBG: c_int = 1;
const LOGLVL_WRN: c_int = 3;
const LOGLVL_ERR: c_int = 4;

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

const HL_INVERSE: i32 = 0x01;
const HL_BOLD: i32 = 0x02;
const HL_ITALIC: i32 = 0x04;
const HL_UNDERLINE_MASK: i32 = 0x38;
const HL_UNDERLINE: i32 = 0x08;
const HL_UNDERCURL: i32 = 0x10;
const HL_UNDERDOUBLE: i32 = 0x18;
const HL_UNDERDOTTED: i32 = 0x20;
const HL_UNDERDASHED: i32 = 0x28;
const HL_STANDOUT: i32 = 0x0040;
const HL_STRIKETHROUGH: i32 = 0x0080;
const HL_ALTFONT: i32 = 0x0100;
const HL_DIM: i32 = 0x0200;
const HL_BLINK: i32 = 0x8000;
const HL_CONCEALED: i32 = 0x10000;
const HL_OVERLINE: i32 = 0x20000;
const HL_BG_INDEXED: i32 = 0x0800;
const HL_FG_INDEXED: i32 = 0x1000;

const HLATTRS_INIT: HlAttrs = HlAttrs {
    rgb_ae_attr: 0,
    cterm_ae_attr: 0,
    rgb_fg_color: -1,
    rgb_bg_color: -1,
    rgb_sp_color: -1,
    cterm_fg_color: 0,
    cterm_bg_color: 0,
    hl_blend: -1,
    url: -1,
    font: -1,
};

const NULL_STRING: NvimString = NvimString {
    data: ptr::null_mut(),
    size: 0,
};

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
    pub input_storage: [u64; 768],
    pub write_loop: [u64; 256],
    pub ti: TerminfoEntry,
    pub term: *mut c_char,
    pub output_handle: [u64; 64],
    pub out_isatty: bool,
    pub winch_handle: [u64; 40],
    pub startup_delay_timer: [u64; 32],
    pub grid: UGrid,
    pub invalid_regions: KVec<Rect>,
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
    pub attrs: KVec<HlAttrs>,
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
    pub urlbuf: StringBuilder,
    pub ti_arena: Arena,
}

static mut cursor_style_enabled: bool = false;
static mut stty_erase_buf: [c_char; 2] = [0, 0];
static mut stty_erase_ptr: *const c_char = ptr::null();

static mut urls: Set<cstr_t> = Set {
    h: MapHash {
        hash: ptr::null_mut(),
        n_buckets: 0,
        size: 0,
        n_occupied: 0,
        upper_bound: 0,
        n_keys: 0,
        keys_capacity: 0,
    },
    keys: ptr::null_mut(),
};

unsafe extern "C" {
    static mut main_loop: u8;
    static mut nvim_testing: bool;
    static mut ui_client_channel_id: u64;
    static mut ui_client_exit_status: c_int;
    static mut ui_client_error_exit: c_int;
    static mut g_min_log_level: c_int;

    fn uv_loop_init(loop_: *mut uv_loop_t) -> c_int;
    fn uv_loop_close(loop_: *mut uv_loop_t) -> c_int;
    fn uv_run(loop_: *mut uv_loop_t, mode: c_int) -> c_int;
    fn uv_tty_init(loop_: *mut uv_loop_t, tty: *mut uv_tty_t, fd: c_int, readable: c_int) -> c_int;
    fn uv_tty_set_mode(tty: *mut uv_tty_t, mode: c_int) -> c_int;
    fn uv_tty_reset_mode() -> c_int;
    fn uv_tty_get_winsize(tty: *mut uv_tty_t, width: *mut c_int, height: *mut c_int) -> c_int;
    fn uv_pipe_init(loop_: *mut uv_loop_t, handle: *mut uv_pipe_t, ipc: c_int) -> c_int;
    fn uv_pipe_open(handle: *mut uv_pipe_t, file: c_int) -> c_int;
    fn uv_timer_init(loop_: *mut uv_loop_t, handle: *mut uv_timer_t) -> c_int;
    fn uv_timer_start(handle: *mut uv_timer_t, cb: uv_timer_cb, timeout: u64, repeat: u64)
    -> c_int;
    fn uv_is_closing(handle: *const uv_handle_t) -> c_int;
    fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);
    fn uv_write(
        req: *mut uv_write_t,
        handle: *mut uv_stream_t,
        bufs: *const uv_buf_t,
        nbufs: c_uint,
        cb: uv_write_cb,
    ) -> c_int;
    fn uv_sleep(msec: c_uint);
    fn uv_chdir(dir: *const c_char) -> c_int;
    fn uv_strerror(err: c_int) -> *const c_char;
    #[cfg(windows)]
    fn uv_handle_get_type(handle: *const uv_handle_t) -> c_int;
    #[cfg(windows)]
    fn uv_handle_size(h_type: c_int) -> usize;

    fn utf_ptr2char(p_in: *const c_char) -> c_int;
    fn utf_ambiguous_width(p: *const c_char) -> bool;
    fn utf_char2cells(c: c_int) -> c_int;

    fn nlua_exec(
        str_: NvimString,
        chunkname: *const c_char,
        args: Array,
        mode: LuaRetMode,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn api_free_object(value: Object);
    fn api_clear_error(value: *mut Error);

    fn ui_client_set_size(width: c_int, height: c_int);
    #[cfg(unix)]
    fn ui_client_detach();
    #[cfg(unix)]
    fn ui_client_attach(width: c_int, height: c_int, term: *mut c_char, rgb: bool);

    fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    fn fclose(stream: *mut FILE) -> c_int;

    #[cfg(unix)]
    fn kill(pid: c_int, sig: c_int) -> c_int;

    fn logmsg(
        log_level: c_int,
        context: *const c_char,
        func_name: *const c_char,
        line_num: c_int,
        eol: bool,
        fmt: *const c_char,
        ...
    ) -> bool;
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> c_int;
}

#[cfg(unix)]
mod unix_termios {
    use std::ffi::{c_int, c_ulong};

    #[cfg(target_os = "linux")]
    pub type tcflag_t = u32;
    #[cfg(not(target_os = "linux"))]
    pub type tcflag_t = c_ulong;

    #[cfg(target_os = "linux")]
    pub type speed_t = u32;
    #[cfg(not(target_os = "linux"))]
    pub type speed_t = c_ulong;

    #[cfg(target_os = "linux")]
    pub const NCCS: usize = 32;
    #[cfg(not(target_os = "linux"))]
    pub const NCCS: usize = 20;

    #[cfg(target_os = "linux")]
    pub const VERASE: usize = 2;
    #[cfg(not(target_os = "linux"))]
    pub const VERASE: usize = 3;

    #[repr(C)]
    pub struct termios {
        pub c_iflag: tcflag_t,
        pub c_oflag: tcflag_t,
        pub c_cflag: tcflag_t,
        pub c_lflag: tcflag_t,
        #[cfg(target_os = "linux")]
        pub c_line: u8,
        pub c_cc: [u8; NCCS],
        pub c_ispeed: speed_t,
        pub c_ospeed: speed_t,
    }

    unsafe extern "C" {
        pub fn tcgetattr(fd: c_int, termios_p: *mut termios) -> c_int;
    }
}

#[inline]
unsafe fn tui_input(tui: *mut TUIData) -> *mut TermInput {
    unsafe { (*tui).input_storage.as_mut_ptr().cast::<TermInput>() }
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
    unsafe { (*tui).winch_handle.as_mut_ptr().cast::<SignalWatcher>() }
}

#[inline]
unsafe fn tui_startup_delay_timer(tui: *mut TUIData) -> *mut uv_timer_t {
    unsafe { (*tui).startup_delay_timer.as_mut_ptr().cast::<uv_timer_t>() }
}

#[inline]
const fn boolean_obj(b: bool) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

#[inline]
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
fn static_cstr_as_string(s: &'static CStr) -> NvimString {
    NvimString {
        data: s.as_ptr().cast_mut(),
        size: s.to_bytes().len(),
    }
}

#[inline]
fn static_cstr_as_obj(s: &'static CStr) -> Object {
    string_obj(static_cstr_as_string(s))
}

#[inline]
unsafe fn cstr_as_obj(s: *const c_char) -> Object {
    unsafe {
        if s.is_null() {
            string_obj(NULL_STRING)
        } else {
            string_obj(NvimString {
                data: s.cast_mut(),
                size: CStr::from_ptr(s).to_bytes().len(),
            })
        }
    }
}

#[inline]
const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
unsafe fn cstr_contains(haystack: *const c_char, needle: &[u8]) -> bool {
    if haystack.is_null() {
        return false;
    }
    let bytes = unsafe { CStr::from_ptr(haystack).to_bytes() };
    bytes.windows(needle.len()).any(|w| w == needle)
}

#[inline]
unsafe fn kv_a<T: Copy>(v: &mut KVec<T>, i: usize) -> *mut T {
    if v.capacity <= i {
        v.size = i + 1;
        v.capacity = (i + 1).next_power_of_two().max(1);
        v.items = unsafe {
            xrealloc(
                v.items.cast::<c_void>(),
                std::mem::size_of::<T>() * v.capacity,
            )
            .cast::<T>()
        };
    } else if v.size <= i {
        v.size = i + 1;
    }
    unsafe { v.items.add(i) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_start(
    tui_p: *mut *mut TUIData,
    width: *mut c_int,
    height: *mut c_int,
    term: *mut *mut c_char,
    rgb: *mut bool,
) {
    unsafe {
        let tui = xcalloc(1, size_of::<TUIData>()).cast::<TUIData>();
        (*tui).is_starting = true;
        (*tui).screenshot = ptr::null_mut();
        (*tui).stopped = false;
        (*tui).loop_ = (&raw mut main_loop).cast::<Loop>();
        (*tui).url = -1;

        (*tui).invalid_regions.init();
        (*tui).urlbuf.init();
        signal_watcher_init((*tui).loop_, tui_winch_handle(tui), tui.cast());
        signal_watcher_start(tui_winch_handle(tui), Some(sigwinch_cb), SIGWINCH);

        (*tui).attrs.push(HLATTRS_INIT);

        (*input_prefix(tui_input(tui))).tk_ti_hook_fn = Some(tui_tk_ti_getstr);
        ugrid_init(&raw mut (*tui).grid);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_handle_term_mode(
    tui: *mut TUIData,
    mode: TermMode,
    state: TermModeState,
) {
    unsafe {
        let mut is_set = false;
        match state {
            kTermModeNotRecognized | kTermModePermanentlyReset => {
                if !nvim_testing && g_min_log_level <= LOGLVL_WRN {
                    logmsg(
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
                    logmsg(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_enable_extended_underline(tui: *mut TUIData) {
    unsafe {
        terminfo_set_if_empty(tui, kTerm_set_underline_style, c"\x1b[4:%p1%dm".as_ptr());
        (*tui).can_set_underline_color = true;
    }
}

unsafe extern "C" fn tui_set_key_encoding_cb(tui: *mut c_void) {
    unsafe {
        tui_set_key_encoding(tui.cast::<TUIData>());
    }
}

unsafe fn tui_query_kitty_keyboard(tui: *mut TUIData) {
    unsafe {
        (*input_prefix(tui_input(tui)))
            .callbacks
            .primary_device_attr = Some(tui_set_key_encoding_cb);
        let seq = b"\x1b[?u\x1b[c";
        out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_set_key_encoding(tui: *mut TUIData) {
    unsafe {
        match (*input_prefix(tui_input(tui))).key_encoding {
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
        match (*input_prefix(tui_input(tui))).key_encoding {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_query_bg_color(tui: *mut TUIData) {
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

unsafe fn push_termdef_err(err_arena: &mut Arena, chunks: &mut KVec<Object>, msg: &str) {
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
        let mut chunks: KVec<Object> = KVec::new();
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
            let chunks_arr = Array {
                items: chunks.items,
                size: chunks.size,
                capacity: chunks.capacity,
            };
            let mut args_items = [array_obj(chunks_arr), boolean_obj(true), dict_obj(opts)];
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
        (*input_suffix(tui_input(tui))).tui_data = tui.cast();

        (*tui).ti_arena = ARENA_EMPTY;
        assert!((*tui).term.is_null());

        #[allow(unused_mut)]
        let mut term = os_getenv(c"TERM".as_ptr());
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
        if !term.is_null() && !os_env_exists(c"NVIM_TERMDEFS".as_ptr(), false) {
            if terminfo_from_database(&raw mut (*tui).ti, term, &raw mut (*tui).ti_arena) {
                (*tui).term = arena_strdup(&raw mut (*tui).ti_arena, term);
                (*tui).terminfo_found_in_db = true;
            }
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
        uv_loop_init(write_loop);
        if (*tui).out_isatty {
            let ret = uv_tty_init(write_loop, output_handle.cast(), (*tui).out_fd, 0);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg(
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
                    logmsg(
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
                logmsg(
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
                logmsg(
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
        if (*input_loop_ptr(input)).is_null() {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_stop(tui: *mut TUIData) {
    unsafe {
        if uv_is_closing(tui_output_handle(tui)) != 0 {
            if g_min_log_level <= LOGLVL_ERR {
                logmsg(
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
        (*input_prefix(input)).callbacks.primary_device_attr = Some(tui_stop_cb);
        terminfo_disable(tui);

        let wait_start = os_hrtime();
        let events = (*loop_queues((*tui).loop_)).events;
        let rs = input_read_stream(input);
        let mut remaining: i64 = EXIT_TIMEOUT_MS;
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !((*tui).stopped || (*rstream_fields(rs)).did_eof) {
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
        if !(*tui).stopped && ((os_hrtime() - wait_start) / 1_000_000) as i64 >= EXIT_TIMEOUT_MS {
            if g_min_log_level <= LOGLVL_WRN {
                logmsg(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"tui_stop".as_ptr(),
                    773,
                    true,
                    c"TUI: timed out waiting for DA1 response".as_ptr(),
                );
            }
        }
        (*tui).stopped = true;

        tui_terminal_stop(tui);
        stream_set_blocking((*input_prefix(input)).in_fd, true);
        tinput_destroy(input);
        signal_watcher_stop(tui_winch_handle(tui));
        signal_watcher_close(tui_winch_handle(tui), None);
        uv_close(tui_startup_delay_timer(tui).cast(), None);
    }
}

unsafe extern "C" fn tui_stop_cb(tui: *mut c_void) {
    unsafe {
        (*tui.cast::<TUIData>()).stopped = true;
    }
}

unsafe fn tui_terminal_stop(tui: *mut TUIData) {
    unsafe {
        tinput_stop(tui_input(tui));
        terminfo_stop(tui);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_is_stopped(tui: *mut TUIData) -> bool {
    unsafe { (*tui).stopped }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_free_all_mem(tui: *mut TUIData) {
    unsafe {
        ugrid_free(&raw mut (*tui).grid);
        (*tui).invalid_regions.destroy();

        for i in 0..urls.h.n_keys as usize {
            xfree((*urls.keys.add(i)).cast_mut().cast());
        }
        xfree(urls.h.hash.cast());
        xfree(urls.keys.cast());
        urls = Set {
            h: MapHash {
                hash: ptr::null_mut(),
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
            },
            keys: ptr::null_mut(),
        };

        (*tui).attrs.destroy();
        (*tui).urlbuf.destroy();
        xfree(tui.cast());
    }
}

unsafe extern "C" fn sigwinch_cb(
    _watcher: *mut SignalWatcher,
    _signum: c_int,
    cbdata: *mut c_void,
) {
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
        let a1 = *(*tui).attrs.items.add(id1 as usize);
        let a2 = *(*tui).attrs.items.add(id2 as usize);

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
        let attrs = *(*tui).attrs.items.add(attr_id as usize);
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
                let url = *urls.keys.add(attrs.url as usize);
                let url_str = CStr::from_ptr(url).to_string_lossy();
                (*tui).urlbuf.size = 0;
                let id: u64 = 0xE1EA_0000u64 + u64::from(attrs.url as u32);
                let formatted = format!("\x1b]8;id={id};{url_str}\x1b\\");
                (*tui)
                    .urlbuf
                    .concat_len(formatted.as_ptr().cast::<c_char>(), formatted.len());
                out(tui, (*tui).urlbuf.items, (*tui).urlbuf.size);
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
        let mut cell = (*(*tui).grid.cells.add(row as usize)).add(col as usize);
        while next != 0 {
            next -= 1;
            if attrs_differ(tui, (*cell).attr, (*tui).print_attr_id, (*tui).rgb)
                && (*tui).default_attr
            {
                return false;
            }
            if schar_get_ascii((*cell).data) == 0 {
                return false;
            }
            cell = cell.add(1);
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
            ugrid_goto(&raw mut (*tui).grid, row, col);
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
                ugrid_goto(&raw mut (*tui).grid, cur_row, 0);
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
                    ugrid_goto(&raw mut (*tui).grid, row, col);
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
                    ugrid_goto(&raw mut (*tui).grid, row, col);
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
                    ugrid_goto(&raw mut (*tui).grid, row, col);
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
                    ugrid_goto(&raw mut (*tui).grid, row, col);
                    return;
                }
            }
        }

        terminfo_print_num2(tui, kTerm_cursor_address, row, col);
        ugrid_goto(&raw mut (*tui).grid, row, col);
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
    cell: *mut UCell,
    is_doublewidth: bool,
) {
    unsafe {
        if (*tui).grid.row == -1 && (*cell).data == 0 {
            return;
        }

        cursor_goto(tui, row, col);

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(buf.as_mut_ptr(), (*cell).data);
        let c = utf_ptr2char(buf.as_ptr());
        let mut is_ambiwidth = utf_ambiguous_width(buf.as_ptr());
        if is_doublewidth && (is_ambiwidth || utf_char2cells(c) == 1) {
            is_ambiwidth = true;
            update_attrs(tui, (*cell).attr);
            print_spaces(tui, 2);
            cursor_goto(tui, row, col);
        }

        print_cell(tui, buf.as_ptr(), (*cell).attr);

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
                ugrid_goto(&raw mut (*tui).grid, top, left);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_grid_resize(
    tui: *mut TUIData,
    _g: Integer,
    width: Integer,
    height: Integer,
) {
    unsafe {
        ugrid_resize(&raw mut (*tui).grid, width as c_int, height as c_int);

        for i in 0..(*tui).invalid_regions.size {
            let r = &mut *(*tui).invalid_regions.items.add(i);
            r.bot = r.bot.min((*tui).grid.height);
            r.right = r.right.min((*tui).grid.width);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_grid_clear(tui: *mut TUIData, _g: Integer) {
    unsafe {
        ugrid_clear(&raw mut (*tui).grid);
        schar_cache_clear_if_full();
        (*tui).invalid_regions.size = 0;
        clear_region(tui, 0, (*tui).height, 0, (*tui).width, 0);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_grid_cursor_goto(
    tui: *mut TUIData,
    _grid: Integer,
    row: Integer,
    col: Integer,
) {
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
                logmsg(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_mode_info_set(
    tui: *mut TUIData,
    guicursor_enabled: bool,
    args: Array,
) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_update_menu(_tui: *mut TUIData) {}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_busy_start(tui: *mut TUIData) {
    unsafe {
        (*tui).busy = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_busy_stop(tui: *mut TUIData) {
    unsafe {
        (*tui).busy = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_mouse_on(tui: *mut TUIData) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_mouse_off(tui: *mut TUIData) {
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

        if c.id != 0 && (c.id as usize) < (*tui).attrs.size && (*tui).rgb {
            let aep = *(*tui).attrs.items.add(c.id as usize);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_mode_change(tui: *mut TUIData, _mode: NvimString, mode_idx: Integer) {
    unsafe {
        #[cfg(unix)]
        if (*tui).out_isatty && (*tui).is_starting && !stdin_isatty {
            let ret = uv_tty_set_mode(tui_output_handle(tui).cast(), UV_TTY_MODE_NORMAL);
            if ret != 0 && g_min_log_level <= LOGLVL_ERR {
                logmsg(
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
                logmsg(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_grid_scroll(
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

        ugrid_scroll(&raw mut (*tui).grid, top, bot, left, right, rows as c_int);

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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_add_url(_tui: *mut TUIData, url: *const c_char) -> i32 {
    unsafe {
        if url.is_null() {
            return -1;
        }

        let mut status = MHPutStatus::kMHExisting;
        let k = mh_put_cstr_t(&raw mut urls, url, &raw mut status);
        if status != MHPutStatus::kMHExisting {
            *urls.keys.add(k as usize) = xstrdup(url);
        }
        k as i32
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_hl_attr_define(
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

        *kv_a(&mut (*tui).attrs, id as usize) = attrs;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_bell(tui: *mut TUIData) {
    unsafe {
        let seq = b"\x07";
        out(tui, seq.as_ptr().cast::<c_char>(), seq.len());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_visual_bell(tui: *mut TUIData) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_default_colors_set(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_ui_send(tui: *mut TUIData, content: NvimString) {
    unsafe {
        if (*tui).invalid_regions.size != 0 || (*tui).bufpos > 0 {
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
            logmsg(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_flush(tui: *mut TUIData) {
    unsafe {
        let nrevents = loop_size((*tui).loop_);
        if nrevents > TOO_MANY_EVENTS {
            if g_min_log_level <= LOGLVL_WRN {
                logmsg(
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

        while (*tui).invalid_regions.size != 0 {
            (*tui).invalid_regions.size -= 1;
            let r = *(*tui)
                .invalid_regions
                .items
                .add((*tui).invalid_regions.size);
            assert!(r.bot <= (*tui).grid.height && r.right <= (*tui).grid.width);

            for row in r.top..r.bot {
                let row_cells = *(*tui).grid.cells.add(row as usize);
                let clear_attr = (*row_cells.add((r.right - 1) as usize)).attr;
                let mut clear_col = r.right;
                while clear_col > 0 {
                    let cell = row_cells.add((clear_col - 1) as usize);
                    if !((*cell).data == schar_from_ascii(b' ') && (*cell).attr == clear_attr) {
                        break;
                    }
                    clear_col -= 1;
                }

                for curcol in r.left..clear_col {
                    let cell = row_cells.add(curcol as usize);
                    let is_double = curcol < clear_col - 1 && (*cell.add(1)).data == 0;
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_suspend(tui: *mut TUIData) {
    #[cfg(unix)]
    unsafe {
        ui_client_detach();
        (*tui).mouse_enabled_save = (*tui).mouse_enabled;
        (*input_prefix(tui_input(tui)))
            .callbacks
            .primary_device_attr = Some(tui_suspend_cb);
        terminfo_disable(tui);
    }
    #[cfg(not(unix))]
    let _ = tui;
}

#[cfg(unix)]
unsafe extern "C" fn tui_suspend_cb(tui_ptr: *mut c_void) {
    unsafe {
        let tui = tui_ptr.cast::<TUIData>();
        let input = tui_input(tui);
        tui_terminal_stop(tui);
        stream_set_blocking((*input_prefix(input)).in_fd, true);

        kill(0, SIGSTOP);

        tui_terminal_start(tui);
        tui_terminal_after_startup(tui);
        if (*tui).mouse_enabled_save {
            tui_mouse_on(tui);
        }
        stream_set_blocking((*input_prefix(input)).in_fd, false);
        ui_client_attach((*tui).width, (*tui).height, (*tui).term, (*tui).rgb);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_set_title(tui: *mut TUIData, title: NvimString) {
    unsafe {
        if !(*tui).can_set_title {
            return;
        }

        let too_long = title.size > 4096;
        if too_long && g_min_log_level <= LOGLVL_ERR {
            logmsg(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_set_icon(_tui: *mut TUIData, _icon: NvimString) {}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_screenshot(tui: *mut TUIData, path: NvimString) {
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
                let cell = *(*(*tui).grid.cells.add(i as usize)).add(j as usize);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_option_set(tui: *mut TUIData, name: NvimString, value: Object) {
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
            (*input_prefix(tui_input(tui))).ttimeout = value.data.boolean;
        } else if strequal(name.data, c"ttimeoutlen".as_ptr()) {
            (*input_prefix(tui_input(tui))).ttimeoutlen = value.data.integer as OptInt;
        } else if strequal(name.data, c"verbose".as_ptr()) {
            (*tui).verbose = value.data.integer;
        } else if strequal(name.data, c"termsync".as_ptr()) {
            (*tui).sync_output = value.data.boolean;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_chdir(_tui: *mut TUIData, path: NvimString) {
    unsafe {
        let err = uv_chdir(path.data);
        if err != 0 && g_min_log_level <= LOGLVL_ERR {
            logmsg(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_raw_line(
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
        let row_cells = *(*tui).grid.cells.add(linerow as usize);
        for c in startcol..endcol {
            let idx = (c - startcol) as usize;
            let cell = row_cells.add(c as usize);
            (*cell).data = *chunk.add(idx);
            assert!((*attrs.add(idx) as usize) < (*tui).attrs.size);
            (*cell).attr = *attrs.add(idx);
        }
        for curcol in (startcol as c_int)..(endcol as c_int) {
            let cell = row_cells.add(curcol as usize);
            let is_double = curcol < (endcol as c_int) - 1 && (*cell.add(1)).data == 0;
            print_cell_at_pos(tui, linerow as c_int, curcol, cell, is_double);
        }

        if clearcol > endcol {
            ugrid_clear_chunk(
                &raw mut (*tui).grid,
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
                let size = if (*row_cells.add(((*tui).grid.width - 1) as usize)).data == 0 {
                    2
                } else {
                    1
                };
                let target_col = (*tui).grid.width - size;
                print_cell_at_pos(
                    tui,
                    linerow as c_int,
                    target_col,
                    row_cells.add(target_col as usize),
                    size == 2,
                );
            }

            final_column_wrap(tui);
        }
    }
}

unsafe fn invalidate(tui: *mut TUIData, top: c_int, bot: c_int, left: c_int, right: c_int) {
    unsafe {
        let mut intersects: *mut Rect = ptr::null_mut();
        for i in 0..(*tui).invalid_regions.size {
            let r = (*tui).invalid_regions.items.add(i);
            if !(top > (*r).bot || bot < (*r).top) && !(left > (*r).right || right < (*r).left) {
                intersects = r;
                break;
            }
        }

        if !intersects.is_null() {
            (*intersects).top = top.min((*intersects).top);
            (*intersects).bot = bot.max((*intersects).bot);
            (*intersects).left = left.min((*intersects).left);
            (*intersects).right = right.max((*intersects).right);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_set_size(tui: *mut TUIData, width: c_int, height: c_int) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tui_guess_size(tui: *mut TUIData) {
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
            let lines = os_getenv_noalloc(c"LINES".as_ptr());
            let columns = os_getenv_noalloc(c"COLUMNS".as_ptr());
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
            (*input_prefix(tui_input(tui))).key_encoding = kKeyEncodingLegacy;
        } else {
            (*input_prefix(tui_input(tui))).key_encoding = kKeyEncodingXterm;
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
                fwrite(b.base.cast(), b.len as usize, 1, (*tui).screenshot);
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
                    logmsg(
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
            if unix_termios::tcgetattr((*input_prefix(input)).in_fd, &raw mut t) != -1 {
                stty_erase_buf[0] = t.c_cc[unix_termios::VERASE] as c_char;
                stty_erase_buf[1] = NUL;
                if g_min_log_level <= LOGLVL_DBG {
                    logmsg(
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
            let rs = input_read_stream(input);
            let uv_ptr = stream_uv_ptr(rs.cast());
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

unsafe extern "C" fn tui_tk_ti_getstr(
    name: *const c_char,
    value: *const c_char,
    data: *mut c_void,
) -> *const c_char {
    unsafe {
        let input = data.cast::<TermInput>();
        if stty_erase_ptr.is_null() {
            stty_erase_ptr = tui_get_stty_erase(input);
        }

        if strequal(name, c"key_backspace".as_ptr()) {
            if g_min_log_level <= LOGLVL_DBG {
                logmsg(
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
                logmsg(
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
                logmsg(
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
