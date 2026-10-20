//! VT220/xterm-like terminal emulator (`src/nvim/terminal.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    unused_mut,
    unused_variables,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::while_immutable_condition
)]

#[allow(unused_imports)]
use crate::api::private::helpers::int_obj;
#[allow(unused_imports)]
use crate::api::private::helpers::{api_clear_error, api_free_object};
#[allow(unused_imports)]
use crate::api::private::helpers::{dict_get_value, dict_set_var};
#[allow(unused_imports)]
use crate::autocmd::apply_autocmds_group;
#[allow(unused_imports)]
use crate::autocmd::{
    apply_autocmds, block_autocmds, has_event, is_autocmd_blocked, unblock_autocmds,
};
#[allow(unused_imports)]
use crate::buffer::buf_get_changedtick;
#[allow(unused_imports)]
use crate::buffer::do_buffer;
#[allow(unused_imports)]
use crate::drawscreen::{
    redraw_buf_line_later, redraw_later, redraw_statuslines, setcursor, show_cursor_info_later,
    showmode, status_redraw_buf, unshowmode, update_screen,
};
#[allow(unused_imports)]
use crate::eval::eval_call_provider;
#[allow(unused_imports)]
use crate::eval::get_v_event;
#[allow(unused_imports)]
use crate::eval::restore_v_event;
#[allow(unused_imports)]
use crate::eval::typval::tv_dict_add_nr;
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_list, tv_list_append_string,
};
#[allow(unused_imports)]
use crate::eval::vars::get_globvar_dict;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_string;
#[allow(unused_imports)]
use crate::ex_docmd::do_cmdline;
use crate::globals::*;
#[allow(unused_imports)]
use crate::highlight_group::name_to_color;
#[allow(unused_imports)]
use crate::input::{getcmdkeycmd, map_execute_lua, merge_modifiers, paste_repeat, requeue_key};
#[allow(unused_imports)]
use crate::mbyte::mb_check_adjust_col;
#[allow(unused_imports)]
use crate::mbyte::{utf_ptr2char, utf_ptr2len};
#[allow(unused_imports)]
use crate::memline::ml_append_buf;
#[allow(unused_imports)]
use crate::memline::{ml_delete_buf, ml_replace_buf};
#[allow(unused_imports)]
use crate::memory::strequal;
#[allow(unused_imports)]
use crate::r#move::win_col_off;
#[allow(unused_imports)]
use crate::ops::clear_oparg;
#[allow(unused_imports)]
use crate::option::set_option_value;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::optionstr::free_string_option;
#[allow(unused_imports)]
use crate::state::may_trigger_modechanged;
#[allow(unused_imports)]
use crate::strings::vim_snprintf;
#[allow(unused_imports)]
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_cursor_shape, ui_flush, ui_mode_info_set, vim_beep,
};
#[allow(unused_imports)]
use crate::window::{may_trigger_win_scrolled_resized, win_valid};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::command::exarg_T;
use crate::api::options::{Buffer, CtxSwitch, OptIndex};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::{pos_T as tab_pos_T, tabpage_T, win_T, winopt_T};
use crate::change::{appended_lines_buf, changed_lines, deleted_lines_buf};
use crate::channel::{OptInt, TerminalOptions};
use crate::clipboard::{list_T, typval_T};
use crate::context::{ctx_restore, ctx_switch, handle_get_buffer, handle_get_window, is_ctx_win};
use crate::cursor::{coladvance, linenr_T};
use crate::cursor_shape::{
    CursorShape, SHAPE_CURSOR, SHAPE_IDX_TERM, parse_shape_opt, shape_table,
};
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{buf_T, pos_T};
use crate::event::r#loop::loop_queues;
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_free, multiqueue_move_events,
    multiqueue_new, multiqueue_new_child, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::time::{
    TimeWatcher, time_watcher_close, time_watcher_init, time_watcher_start, time_watcher_stop,
    watcher_fields,
};
use crate::grid::schar_get_adv;
use crate::highlight::{RgbValue, hl_add_url, hl_combine_attr, hl_get_term_attr};
use crate::input_cmdatom::{atom_term_enter, cmdarg_T};
use crate::keycodes::{
    ESC, K_BS, K_COMMAND, K_DEL, K_DOWN, K_END, K_F1, K_F2, K_F3, K_F4, K_F5, K_F6, K_F7, K_F8,
    K_F9, K_F10, K_F11, K_F12, K_F13, K_F14, K_F15, K_F16, K_F17, K_F18, K_F19, K_F20, K_F21,
    K_F22, K_F23, K_F24, K_F25, K_F26, K_F27, K_F28, K_F29, K_F30, K_F31, K_F32, K_F33, K_F34,
    K_F35, K_F36, K_F37, K_F38, K_F39, K_F40, K_F41, K_F42, K_F43, K_F44, K_F45, K_F46, K_F47,
    K_F48, K_F49, K_F50, K_F51, K_F52, K_F53, K_F54, K_F55, K_F56, K_F57, K_F58, K_F59, K_F60,
    K_F61, K_F62, K_F63, K_HOME, K_IGNORE, K_INS, K_K0, K_K1, K_K2, K_K3, K_K4, K_K5, K_K6, K_K7,
    K_K8, K_K9, K_KDEL, K_KDIVIDE, K_KDOWN, K_KEND, K_KENTER, K_KHOME, K_KINS, K_KLEFT, K_KMINUS,
    K_KMULTIPLY, K_KORIGIN, K_KPAGEDOWN, K_KPAGEUP, K_KPLUS, K_KPOINT, K_KRIGHT, K_KUP, K_LEFT,
    K_LEFTDRAG, K_LEFTMOUSE, K_LEFTRELEASE, K_MIDDLEDRAG, K_MIDDLEMOUSE, K_MIDDLERELEASE,
    K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP, K_NOP, K_PAGEDOWN, K_PAGEUP,
    K_RIGHT, K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_TAB, K_UP, K_X1DRAG, K_X1MOUSE,
    K_X1RELEASE, K_X2DRAG, K_X2MOUSE, K_X2RELEASE, K_ZERO, KE_EVENT, KE_LUA, KS_EXTRA,
    MOD_MASK_ALT, MOD_MASK_CTRL, MOD_MASK_SHIFT, TAB, is_special, termcap2key, unsimplify_key,
};
use crate::kvec::KVec;
use crate::map::{HlAttrs, Set, mh_delete_ptr_t, mh_get_ptr_t, mh_put_ptr_t, ptr_t};
use crate::map_glyph_cache::{MH_TOMBSTONE, MHPutStatus, MapHash, NvimString};
use crate::mark::{WinInfo, mark_adjust_buf};
use crate::memory::{xcalloc, xfree, xmalloc, xmemdup, xmemdupz, xrealloc, xstrdup};
use crate::mouse::{do_mousescroll, mouse_find_win_inner};
use crate::r#move::{curs_columns, set_topline, validate_cursor};
use crate::msgpack_rpc::channel::event_T;
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::state::{VimState, save_v_event_T, state_enter, state_handle_k_event};
use crate::textformat::{InsState, oparg_T};
use crate::vterm::keyboard::{
    VTermColor, VTermStateFull as VTermState, vterm_keyboard_end_paste, vterm_keyboard_key,
    vterm_keyboard_start_paste, vterm_keyboard_unichar,
};
use crate::vterm::mouse::{VTerm, VTermModifier, VTermPos, vterm_mouse_button, vterm_mouse_move};
use crate::vterm::parser::{VTermStringFragment, VTermTerminator, vterm_input_write};
use crate::vterm::pen::{
    VTermAttr, VTermProp, VTermRect, VTermValue, VTermValueType, vterm_state_convert_color_to_rgb,
    vterm_state_set_palette_color, vterm_state_set_penattr,
};
use crate::vterm::screen::{
    VTermDamageSize, VTermScreen, VTermScreenCallbacks, VTermScreenCell, VTermScreenCellAttrs,
    vterm_obtain_screen, vterm_screen_enable_altscreen, vterm_screen_enable_reflow,
    vterm_screen_flush_damage, vterm_screen_get_cell, vterm_screen_reset,
    vterm_screen_set_callbacks, vterm_screen_set_damage_merge,
    vterm_screen_set_unrecognised_fallbacks,
};
use crate::vterm::state::{
    VTermSelectionCallbacks, VTermSelectionMask, VTermStateFallbacks, vterm_obtain_state,
    vterm_state_focus_in, vterm_state_focus_out, vterm_state_set_selection_callbacks,
    vterm_state_set_termprop,
};
use crate::vterm::vterm::{
    vterm_free, vterm_get_size, vterm_new, vterm_output_set_callback, vterm_set_size,
    vterm_set_utf8,
};

pub use crate::api::deprecated::StringBuilder;
pub use crate::cmdexpand::LineGetter;
pub use crate::vterm::keyboard::VTermKey;

const NUL: c_char = 0;
const REFRESH_DELAY: u64 = 10;
const TEXTBUF_SIZE: usize = 0x1fff;
const SELECTIONBUF_SIZE: usize = 0x0400;

const SB_MAX: i64 = 1_000_000;
const ML_EMPTY: c_int = 0x01;
const OPT_LOCAL: c_int = 0x02;
const kOptBuftype: OptIndex = 29;
const kOptCuloptFlagNumber: u8 = 0x04;
const kOptBoFlagTerm: c_uint = 0x40000;

const kOptTpfFlagBS: c_uint = 0x01;
const kOptTpfFlagHT: c_uint = 0x02;
const kOptTpfFlagFF: c_uint = 0x04;
const kOptTpfFlagESC: c_uint = 0x08;
const kOptTpfFlagDEL: c_uint = 0x10;
const kOptTpfFlagC0: c_uint = 0x20;
const kOptTpfFlagC1: c_uint = 0x40;

const VV_TERMREQUEST: c_int = 10;
const AUGROUP_ALL: c_int = -3;

const EVENT_TERMCLOSE: event_T = 121;
const EVENT_TERMENTER: event_T = 122;
const EVENT_TERMLEAVE: event_T = 123;
const EVENT_TERMOPEN: event_T = 124;
const EVENT_TERMREQUEST: event_T = 125;
const EVENT_TEXTCHANGEDT: event_T = 130;

const VTERM_TERMINATOR_BEL: VTermTerminator = 0;

const VTERM_ATTR_URI: VTermAttr = 13;
const VTERM_VALUETYPE_INT: VTermValueType = 2;
const VTERM_DAMAGE_SCROLL: VTermDamageSize = 3;

const VTERM_PROP_CURSORVISIBLE: VTermProp = 1;
const VTERM_PROP_CURSORBLINK: VTermProp = 2;
const VTERM_PROP_ALTSCREEN: VTermProp = 3;
const VTERM_PROP_TITLE: VTermProp = 4;
const VTERM_PROP_ICONNAME: VTermProp = 5;
const VTERM_PROP_REVERSE: VTermProp = 6;
const VTERM_PROP_CURSORSHAPE: VTermProp = 7;
const VTERM_PROP_MOUSE: VTermProp = 8;
const VTERM_PROP_FOCUSREPORT: VTermProp = 9;
const VTERM_PROP_THEMEUPDATES: VTermProp = 10;
const VTERM_PROP_SYNCOUTPUT: VTermProp = 11;

const VTERM_PROP_CURSORSHAPE_BLOCK: c_int = 1;
const VTERM_PROP_CURSORSHAPE_UNDERLINE: c_int = 2;
const VTERM_PROP_CURSORSHAPE_BAR_LEFT: c_int = 3;

const VTERM_UNDERLINE_OFF: u32 = 0;
const VTERM_UNDERLINE_SINGLE: u32 = 1;
const VTERM_UNDERLINE_DOUBLE: u32 = 2;
const VTERM_UNDERLINE_CURLY: u32 = 3;

const VTERM_COLOR_INDEXED: u8 = 0x01;
const VTERM_COLOR_DEFAULT_FG: u8 = 0x02;
const VTERM_COLOR_DEFAULT_BG: u8 = 0x04;

const VTERM_SELECTION_CLIPBOARD: VTermSelectionMask = 1 << 0;
const VTERM_SELECTION_PRIMARY: VTermSelectionMask = 1 << 1;

const VTERM_MOD_NONE: VTermModifier = 0x00;
const VTERM_MOD_SHIFT: VTermModifier = 0x01;
const VTERM_MOD_ALT: VTermModifier = 0x02;
const VTERM_MOD_CTRL: VTermModifier = 0x04;

const VTERM_KEY_NONE: VTermKey = 0;
const VTERM_KEY_ENTER: VTermKey = 1;
const VTERM_KEY_TAB: VTermKey = 2;
const VTERM_KEY_BACKSPACE: VTermKey = 3;
const VTERM_KEY_ESCAPE: VTermKey = 4;
const VTERM_KEY_UP: VTermKey = 5;
const VTERM_KEY_DOWN: VTermKey = 6;
const VTERM_KEY_LEFT: VTermKey = 7;
const VTERM_KEY_RIGHT: VTermKey = 8;
const VTERM_KEY_INS: VTermKey = 9;
const VTERM_KEY_DEL: VTermKey = 10;
const VTERM_KEY_HOME: VTermKey = 11;
const VTERM_KEY_END: VTermKey = 12;
const VTERM_KEY_PAGEUP: VTermKey = 13;
const VTERM_KEY_PAGEDOWN: VTermKey = 14;
const VTERM_KEY_FUNCTION_0: VTermKey = 256;
const VTERM_KEY_KP_0: VTermKey = 512;
const VTERM_KEY_KP_1: VTermKey = 513;
const VTERM_KEY_KP_2: VTermKey = 514;
const VTERM_KEY_KP_3: VTermKey = 515;
const VTERM_KEY_KP_4: VTermKey = 516;
const VTERM_KEY_KP_5: VTermKey = 517;
const VTERM_KEY_KP_6: VTermKey = 518;
const VTERM_KEY_KP_7: VTermKey = 519;
const VTERM_KEY_KP_8: VTermKey = 520;
const VTERM_KEY_KP_9: VTermKey = 521;
const VTERM_KEY_KP_MULT: VTermKey = 522;
const VTERM_KEY_KP_PLUS: VTermKey = 523;
const VTERM_KEY_KP_COMMA: VTermKey = 524;
const VTERM_KEY_KP_MINUS: VTermKey = 525;
const VTERM_KEY_KP_PERIOD: VTermKey = 526;
const VTERM_KEY_KP_DIVIDE: VTermKey = 527;
const VTERM_KEY_KP_ENTER: VTermKey = 528;
const VTERM_KEY_KP_EQUAL: VTermKey = 529;

const K_PASTE_START: c_int = termcap2key(b'P' as c_int, b'S' as c_int);
const K_EVENT: c_int = termcap2key(KS_EXTRA as c_int, KE_EVENT as c_int);
const K_LUA: c_int = termcap2key(KS_EXTRA as c_int, KE_LUA as c_int);

const Ctrl_AT: c_int = 0;
const Ctrl_C: c_int = 3;
const Ctrl_M: c_int = 13;
const Ctrl_N: c_int = 14;
const Ctrl_O: c_int = 15;
const Ctrl_BSL: c_int = 28;

const MODE_TERMINAL: c_int = 0x80;

const UPD_VALID: c_int = 10;
const UPD_SOME_VALID: c_int = 35;
const UPD_NOT_VALID: c_int = 40;

const DOBUF_WIPE: c_int = 4;
const DOBUF_FIRST: c_int = 1;
const FORWARD: c_int = 1;

const MSCR_DOWN: c_int = 0;
const MSCR_UP: c_int = 1;
const MSCR_LEFT: c_int = -1;
const MSCR_RIGHT: c_int = -2;

const MAXLNUM: linenr_T = 0x7fffffff;
const kMarkAdjustTerm: c_int = 2;
const kExtmarkUndo: c_int = 1;
const kListLenMayKnow: isize = -3;

const TERM_ATTRS_MAX: c_int = 1024;

const HL_INVERSE: c_int = 0x01;
const HL_BOLD: c_int = 0x02;
const HL_ITALIC: c_int = 0x04;
const HL_UNDERLINE: c_int = 0x08;
const HL_UNDERCURL: c_int = 0x10;
const HL_UNDERDOUBLE: c_int = 0x18;
const HL_STRIKETHROUGH: c_int = 0x0080;
const HL_DIM: c_int = 0x0200;
const HL_BG_INDEXED: c_int = 0x0800;
const HL_FG_INDEXED: c_int = 0x1000;
const HL_BLINK: c_int = 0x8000;
const HL_CONCEALED: c_int = 0x10000;
const HL_OVERLINE: c_int = 0x20000;

#[repr(C)]
struct TerminalState {
    state: VimState,
    term: *mut Terminal,
    save_rd: c_int,
    close: bool,
    got_bsl: bool,
    got_bsl_o: bool,
    cursor_visible: bool,
    save_curwin_handle: handle_T,
    save_w_p_cul: bool,
    save_w_p_culopt: *mut c_char,
    save_w_p_culopt_flags: u8,
    save_w_p_cuc: c_int,
    save_w_p_so: OptInt,
    save_w_p_siso: OptInt,
}

#[repr(C)]
struct ScrollbackLine {
    cols: usize,
    cells: [VTermScreenCell; 0],
}

#[repr(C)]
struct TermCursor {
    row: c_int,
    col: c_int,
    shape: c_int,
    visible: bool,
    blink: bool,
}

#[repr(C)]
struct TermPending {
    resize: bool,
    cursor: bool,
    send: *mut StringBuilder,
    events: *mut MultiQueue,
}

#[repr(C)]
pub struct Terminal {
    opts: TerminalOptions,
    vt: *mut VTerm,
    vts: *mut VTermScreen,
    textbuf: [c_char; TEXTBUF_SIZE],
    sb_buffer: *mut *mut ScrollbackLine,
    sb_current: usize,
    sb_size: usize,
    sb_pending: c_int,
    sb_deleted: usize,
    old_sb_deleted: usize,
    old_height: c_int,
    title: *mut c_char,
    title_len: usize,
    title_size: usize,
    buf_handle: handle_T,
    in_altscreen: bool,
    suspended: bool,
    closed: bool,
    destroy: bool,
    forward_mouse: bool,
    invalid_start: c_int,
    invalid_end: c_int,
    cursor: TermCursor,
    pending: TermPending,
    streamed_paste: bool,
    theme_updates: bool,
    synchronized_output: bool,
    sync_flush_pending: bool,
    color_set: [bool; 16],
    selection_buffer: *mut c_char,
    selection: StringBuilder,
    termrequest_buffer: StringBuilder,
    termrequest_terminator: VTermTerminator,
    refcount: usize,
}

static mut REFRESH_TIMER: [u64; 32] = [0; 32];
static mut REFRESH_PENDING: bool = false;

static mut VTERM_SCREEN_CALLBACKS: VTermScreenCallbacks = VTermScreenCallbacks {
    damage: Some(term_damage),
    moverect: Some(term_moverect),
    movecursor: Some(term_movecursor),
    settermprop: Some(term_settermprop),
    bell: Some(term_bell),
    resize: None,
    theme: Some(term_theme),
    sb_pushline: Some(term_sb_push),
    sb_popline: Some(term_sb_pop),
    sb_clear: Some(term_sb_clear),
};

static mut VTERM_SELECTION_CALLBACKS: VTermSelectionCallbacks = VTermSelectionCallbacks {
    set: Some(term_selection_set),
    query: None,
};

static mut VTERM_FALLBACKS: VTermStateFallbacks = VTermStateFallbacks {
    control: None,
    csi: None,
    osc: Some(on_osc),
    dcs: Some(on_dcs),
    apc: Some(on_apc),
    pm: None,
    sos: None,
};

static mut INVALIDATED_TERMINALS: Set<ptr_t> = Set {
    h: MapHash {
        n_buckets: 0,
        size: 0,
        n_occupied: 0,
        upper_bound: 0,
        n_keys: 0,
        keys_capacity: 0,
        hash: ptr::null_mut(),
    },
    keys: ptr::null_mut(),
};

#[inline]
const fn vterm_key_function(n: c_int) -> VTermKey {
    VTERM_KEY_FUNCTION_0 + n
}

#[inline]
unsafe fn sbrow_cells(sbrow: *mut ScrollbackLine) -> *mut VTermScreenCell {
    unsafe { sbrow.add(1).cast::<VTermScreenCell>() }
}

#[inline]
unsafe fn refresh_timer_ptr() -> *mut TimeWatcher {
    (&raw mut REFRESH_TIMER).cast::<TimeWatcher>()
}

#[inline]
unsafe fn set_has_term(set: *mut Set<ptr_t>, term: *mut Terminal) -> bool {
    unsafe { mh_get_ptr_t(set, term.cast::<c_void>()) != MH_TOMBSTONE }
}

#[inline]
unsafe fn set_put_term(set: *mut Set<ptr_t>, term: *mut Terminal) {
    unsafe {
        let mut status = MHPutStatus::kMHExisting;
        mh_put_ptr_t(set, term.cast::<c_void>(), &mut status);
    }
}

#[inline]
unsafe fn set_del_term(set: *mut Set<ptr_t>, term: *mut Terminal) {
    unsafe {
        let mut key: ptr_t = term.cast::<c_void>();
        mh_delete_ptr_t(set, &mut key);
    }
}

#[inline]
unsafe fn set_destroy_ptr(set: *mut Set<ptr_t>) {
    unsafe {
        xfree((*set).keys.cast::<c_void>());
        xfree((*set).h.hash.cast::<c_void>());
        *set = Set {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: ptr::null_mut(),
            },
            keys: ptr::null_mut(),
        };
    }
}

#[inline]
unsafe fn sb_init(sb: *mut StringBuilder) {
    unsafe {
        *sb = KVec {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
    }
}

#[inline]
unsafe fn sb_destroy(sb: *mut StringBuilder) {
    unsafe {
        xfree((*sb).items.cast::<c_void>());
        *sb = KVec {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
    }
}

#[inline]
unsafe fn sb_push(sb: *mut StringBuilder, c: c_char) {
    unsafe {
        if (*sb).size == (*sb).capacity {
            (*sb).capacity = if (*sb).capacity == 0 {
                8
            } else {
                (*sb).capacity * 2
            };
            (*sb).items = xrealloc(
                (*sb).items.cast::<c_void>(),
                (*sb).capacity * size_of::<c_char>(),
            )
            .cast::<c_char>();
        }
        *(*sb).items.add((*sb).size) = c;
        (*sb).size += 1;
    }
}

#[inline]
unsafe fn sb_concat_len(sb: *mut StringBuilder, data: *const c_char, len: usize) {
    if len == 0 || data.is_null() {
        return;
    }
    unsafe {
        let needed = (*sb).size + len;
        if needed > (*sb).capacity {
            let mut cap = if (*sb).capacity == 0 {
                8
            } else {
                (*sb).capacity
            };
            while cap < needed {
                cap *= 2;
            }
            (*sb).capacity = cap;
            (*sb).items = xrealloc(
                (*sb).items.cast::<c_void>(),
                (*sb).capacity * size_of::<c_char>(),
            )
            .cast::<c_char>();
        }
        ptr::copy_nonoverlapping(data, (*sb).items.add((*sb).size), len);
        (*sb).size += len;
    }
}

#[inline]
const fn str_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn cstr_slice_as_string(bytes: &'static [u8]) -> NvimString {
    NvimString {
        data: bytes.as_ptr().cast::<c_char>().cast_mut(),
        size: bytes.len() - 1,
    }
}

unsafe extern "C" fn emit_termrequest(argv: *mut *mut c_void) {
    #[cfg(test)]
    {
        let _ = argv;
    }
    #[cfg(not(test))]
    unsafe {
        let buf_handle = (*argv.add(0) as isize) as handle_T;
        let sequence = (*argv.add(1)).cast::<c_char>();
        let sequence_length = *argv.add(2) as usize;
        let pending_send = (*argv.add(3)).cast::<StringBuilder>();
        let row = (*argv.add(4) as isize) as c_int;
        let col = (*argv.add(5) as isize) as c_int;
        let sb_deleted = *argv.add(6) as usize;
        let terminator = (*argv.add(7) as isize) as VTermTerminator;

        let buf = handle_get_buffer(buf_handle);
        if buf.is_null() || (*buf).terminal.is_null() {
            xfree(sequence.cast::<c_void>());
            sb_destroy(pending_send);
            xfree(pending_send.cast::<c_void>());
            return;
        }
        let term = (*buf).terminal.cast::<Terminal>();

        if (*term).sb_pending > 0 {
            let mut ev_argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
            for (i, slot) in ev_argv.iter_mut().enumerate().take(8) {
                *slot = *argv.add(i);
            }
            multiqueue_put_event(
                (*term).pending.events,
                Event {
                    handler: Some(emit_termrequest),
                    argv: ev_argv,
                },
            );
            return;
        }

        set_vim_var_string(VV_TERMREQUEST, sequence, sequence_length as isize);

        let mut cursor_items = [
            int_obj(i64::from(row) - ((*term).sb_deleted.wrapping_sub(sb_deleted) as i64)),
            int_obj(i64::from(col)),
        ];
        let cursor = Array {
            items: cursor_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };

        let termrequest = NvimString {
            data: sequence,
            size: sequence_length,
        };
        let term_obj = if terminator == VTERM_TERMINATOR_BEL {
            str_obj(cstr_slice_as_string(b"\x07\0"))
        } else {
            str_obj(cstr_slice_as_string(b"\x1b\\\0"))
        };
        let mut data_items = [
            KeyValuePair {
                key: cstr_slice_as_string(b"sequence\0"),
                value: str_obj(termrequest),
            },
            KeyValuePair {
                key: cstr_slice_as_string(b"cursor\0"),
                value: Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData { array: cursor },
                },
            },
            KeyValuePair {
                key: cstr_slice_as_string(b"terminator\0"),
                value: term_obj,
            },
        ];
        let mut data_obj = Object {
            obj_type: ObjectType::kObjectTypeDict,
            data: ObjectData {
                dict: Dict {
                    items: data_items.as_mut_ptr(),
                    size: 3,
                    capacity: 3,
                },
            },
        };

        (*term).refcount += 1;
        apply_autocmds_group(
            EVENT_TERMREQUEST,
            ptr::null_mut(),
            ptr::null_mut(),
            true,
            AUGROUP_ALL,
            buf,
            curwin,
            ptr::null_mut(),
            &mut data_obj,
            false,
        );
        (*term).refcount -= 1;
        xfree(sequence.cast::<c_void>());

        let term_pending_send = (*term).pending.send;
        (*term).pending.send = ptr::null_mut();
        if (*pending_send).size > 0 {
            terminal_send(term, (*pending_send).items, (*pending_send).size);
            sb_destroy(pending_send);
        }
        if term_pending_send != pending_send {
            (*term).pending.send = term_pending_send;
        }
        xfree(pending_send.cast::<c_void>());

        if (*term).buf_handle == 0 && (*term).refcount == 0 {
            (*term).destroy = true;
            if let Some(close_cb) = (*term).opts.close_cb {
                close_cb((*term).opts.data);
            }
        }
    }
}

#[cfg(not(test))]
unsafe fn schedule_termrequest(term: *mut Terminal) {
    unsafe {
        (*term).pending.send = xmalloc(size_of::<StringBuilder>()).cast::<StringBuilder>();
        sb_init((*term).pending.send);

        let line = row_to_linenr(term, (*term).cursor.row);
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = ((*term).buf_handle as isize) as *mut c_void;
        argv[1] = xmemdup(
            (*term).termrequest_buffer.items.cast::<c_void>(),
            (*term).termrequest_buffer.size,
        );
        argv[2] = (*term).termrequest_buffer.size as *mut c_void;
        argv[3] = (*term).pending.send.cast::<c_void>();
        argv[4] = (line as isize) as *mut c_void;
        argv[5] = ((*term).cursor.col as isize) as *mut c_void;
        argv[6] = (*term).sb_deleted as *mut c_void;
        argv[7] = ((*term).termrequest_terminator as isize) as *mut c_void;

        multiqueue_put_event(
            (*loop_queues((&raw mut main_loop).cast())).events,
            Event {
                handler: Some(emit_termrequest),
                argv,
            },
        );
    }
}

#[cfg(not(test))]
unsafe fn parse_osc8(str_: *const c_char, attr: *mut c_int) -> c_int {
    unsafe {
        let mut i: usize = 0;
        while *str_.add(i) != NUL {
            if *str_.add(i) == b';' as c_char {
                break;
            }
            i += 1;
        }

        if *str_.add(i) != b';' as c_char {
            return 0;
        }

        i += 1;

        if *str_.add(i) == NUL {
            *attr = 0;
            return 1;
        }

        *attr = hl_add_url(0, str_.add(i));
        1
    }
}

unsafe extern "C" fn on_osc(command: c_int, frag: VTermStringFragment, user: *mut c_void) -> c_int {
    #[cfg(test)]
    {
        let _ = (command, frag, user);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let term = user.cast::<Terminal>();
        if frag.str_.is_null() || frag.is_empty() {
            return 0;
        }

        if command != 8 && !has_event(EVENT_TERMREQUEST) {
            return 1;
        }

        if frag.initial() {
            (*term).termrequest_buffer.size = 0;
            let mut prefix = [0 as c_char; 32];
            let n = vim_snprintf!(
                prefix.as_mut_ptr(),
                prefix.len(),
                c"\x1b]%d;".as_ptr(),
                command,
            );
            if n > 0 {
                sb_concat_len(
                    &raw mut (*term).termrequest_buffer,
                    prefix.as_ptr(),
                    n as usize,
                );
            }
        }
        sb_concat_len(&raw mut (*term).termrequest_buffer, frag.str_, frag.len());
        if frag.final_() {
            (*term).termrequest_terminator = frag.terminator;
            if has_event(EVENT_TERMREQUEST) {
                schedule_termrequest(term);
            }
            if command == 8 {
                sb_push(&raw mut (*term).termrequest_buffer, NUL);
                let off = b"\x1b]8;".len();
                let mut attr: c_int = 0;
                if parse_osc8((*term).termrequest_buffer.items.add(off), &mut attr) != 0 {
                    let state = vterm_obtain_state((*term).vt);
                    let mut value = VTermValue { number: attr };
                    vterm_state_set_penattr(state, VTERM_ATTR_URI, VTERM_VALUETYPE_INT, &mut value);
                }
            }
        }
        1
    }
}

unsafe extern "C" fn on_dcs(
    command: *const c_char,
    commandlen: usize,
    frag: VTermStringFragment,
    user: *mut c_void,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (command, commandlen, frag, user);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let term = user.cast::<Terminal>();
        if command.is_null() || frag.str_.is_null() {
            return 0;
        }
        if !has_event(EVENT_TERMREQUEST) {
            return 1;
        }

        if frag.initial() {
            (*term).termrequest_buffer.size = 0;
            sb_concat_len(
                &raw mut (*term).termrequest_buffer,
                b"\x1bP".as_ptr().cast::<c_char>(),
                2,
            );
            sb_concat_len(&raw mut (*term).termrequest_buffer, command, commandlen);
        }
        sb_concat_len(&raw mut (*term).termrequest_buffer, frag.str_, frag.len());
        if frag.final_() {
            (*term).termrequest_terminator = frag.terminator;
            schedule_termrequest(term);
        }
        1
    }
}

unsafe extern "C" fn on_apc(frag: VTermStringFragment, user: *mut c_void) -> c_int {
    #[cfg(test)]
    {
        let _ = (frag, user);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let term = user.cast::<Terminal>();
        if frag.str_.is_null() || frag.is_empty() {
            return 0;
        }

        if !has_event(EVENT_TERMREQUEST) {
            return 1;
        }

        if frag.initial() {
            (*term).termrequest_buffer.size = 0;
            sb_concat_len(
                &raw mut (*term).termrequest_buffer,
                b"\x1b_".as_ptr().cast::<c_char>(),
                2,
            );
        }
        sb_concat_len(&raw mut (*term).termrequest_buffer, frag.str_, frag.len());
        if frag.final_() {
            (*term).termrequest_terminator = frag.terminator;
            schedule_termrequest(term);
        }
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_init() {
    #[cfg(not(test))]
    unsafe {
        let timer = refresh_timer_ptr();
        time_watcher_init((&raw mut main_loop).cast(), timer, ptr::null_mut());
        (*watcher_fields(timer)).events =
            multiqueue_new_child((*loop_queues((&raw mut main_loop).cast())).events);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_teardown() {
    #[cfg(not(test))]
    unsafe {
        let timer = refresh_timer_ptr();
        time_watcher_stop(timer);
        multiqueue_free((*watcher_fields(timer)).events);
        time_watcher_close(timer, None);
        set_destroy_ptr(&raw mut INVALIDATED_TERMINALS);
    }
}

unsafe extern "C" fn term_output_callback(s: *const c_char, len: usize, user_data: *mut c_void) {
    unsafe {
        terminal_send(user_data.cast::<Terminal>(), s, len);
    }
}

#[cfg(not(test))]
unsafe fn term_may_alloc_scrollback(term: *mut Terminal, mut buf: *mut buf_T) -> bool {
    unsafe {
        if !(*term).sb_buffer.is_null() {
            return true;
        }
        if buf.is_null() {
            buf = handle_get_buffer((*term).buf_handle);
            if buf.is_null() {
                return false;
            }
        }

        if (*buf).b_p_scbk < 1 {
            (*buf).b_p_scbk = SB_MAX;
        }
        (*term).sb_size = (*buf).b_p_scbk as usize;
        (*term).sb_buffer = xmalloc(size_of::<*mut ScrollbackLine>() * (*term).sb_size)
            .cast::<*mut ScrollbackLine>();
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_alloc(
    buf: *mut buf_T,
    mut opts: TerminalOptions,
) -> *mut Terminal {
    #[cfg(test)]
    {
        let _ = (buf, opts);
        ptr::null_mut()
    }
    #[cfg(not(test))]
    unsafe {
        if opts.width == 0 {
            opts.width = 80;
        }
        if opts.height == 0 {
            opts.height = 24;
        }

        let term = xcalloc(1, size_of::<Terminal>()).cast::<Terminal>();
        (*term).opts = opts;

        (*term).buf_handle = (*buf).b_fnum;
        (*buf).terminal = term;
        (*term).vt = vterm_new(c_int::from(opts.height), c_int::from(opts.width));
        vterm_set_utf8((*term).vt, 1);

        let state = vterm_obtain_state((*term).vt);
        (*term).vts = vterm_obtain_screen((*term).vt);
        vterm_screen_enable_altscreen((*term).vts, 1);
        vterm_screen_enable_reflow((*term).vts, true);
        vterm_screen_set_callbacks(
            (*term).vts,
            &raw const VTERM_SCREEN_CALLBACKS,
            term.cast::<c_void>(),
        );
        vterm_screen_set_unrecognised_fallbacks(
            (*term).vts,
            &raw const VTERM_FALLBACKS,
            term.cast::<c_void>(),
        );
        vterm_screen_set_damage_merge((*term).vts, VTERM_DAMAGE_SCROLL);
        vterm_screen_reset((*term).vts, 1);
        vterm_output_set_callback(
            (*term).vt,
            Some(term_output_callback),
            term.cast::<c_void>(),
        );

        (*term).selection_buffer = xcalloc(SELECTIONBUF_SIZE, 1).cast::<c_char>();
        vterm_state_set_selection_callbacks(
            state,
            &raw const VTERM_SELECTION_CALLBACKS,
            term.cast::<c_void>(),
            (*term).selection_buffer,
            SELECTIONBUF_SIZE,
        );

        let shape_num = match shape_table[SHAPE_IDX_TERM].shape {
            CursorShape::SHAPE_BLOCK => VTERM_PROP_CURSORSHAPE_BLOCK,
            CursorShape::SHAPE_HOR => VTERM_PROP_CURSORSHAPE_UNDERLINE,
            CursorShape::SHAPE_VER => VTERM_PROP_CURSORSHAPE_BAR_LEFT,
        };
        let mut cursor_shape = VTermValue { number: shape_num };
        vterm_state_set_termprop(state, VTERM_PROP_CURSORSHAPE, &mut cursor_shape);

        let mut cursor_blink = VTermValue {
            boolean: c_int::from(
                shape_table[SHAPE_IDX_TERM].blinkon != 0
                    && shape_table[SHAPE_IDX_TERM].blinkoff != 0,
            ),
        };
        vterm_state_set_termprop(state, VTERM_PROP_CURSORBLINK, &mut cursor_blink);

        (*term).invalid_start = 0;
        (*term).invalid_end = c_int::from(opts.height);

        (*term).pending.events = multiqueue_new(None, ptr::null_mut());

        if ((*buf).b_ml.ml_flags & ML_EMPTY) == 0 {
            let line_count = (*buf).b_ml.ml_line_count;
            while ((*buf).b_ml.ml_flags & ML_EMPTY) == 0 {
                ml_delete_buf(buf, 1, false);
            }
            deleted_lines_buf(buf, 1, line_count);
        }
        (*term).old_height = 1;

        term
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_open(termpp: *mut *mut Terminal, buf: *mut buf_T) {
    #[cfg(test)]
    {
        let _ = (termpp, buf);
    }
    #[cfg(not(test))]
    unsafe {
        let term = *termpp;
        debug_assert!(!term.is_null());

        let mut aco: CtxSwitch = std::mem::zeroed();
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);

        if !(*term).sb_buffer.is_null() {
            refresh_scrollback(term, buf);
        } else {
            debug_assert!((*term).invalid_start >= 0);
        }
        refresh_screen(term, buf);
        (*buf).b_locked += 1;
        set_option_value(
            kOptBuftype,
            str_obj(cstr_slice_as_string(b"terminal\0")),
            OPT_LOCAL,
            true,
            ptr::null(),
        );
        (*buf).b_locked -= 1;

        if !(*buf).b_ffname.is_null() {
            buf_set_term_title(
                buf,
                (*buf).b_ffname,
                CStr::from_ptr((*buf).b_ffname).to_bytes().len(),
            );
        }
        (*curwin).w_onebuf_opt.wo_scb = 0;
        (*curwin).w_onebuf_opt.wo_crb = 0;
        (*curwin).w_cursor = tab_pos_T {
            lnum: 1,
            col: 0,
            coladd: 0,
        };

        apply_autocmds(EVENT_TERMOPEN, ptr::null_mut(), ptr::null_mut(), false, buf);

        ctx_restore(&mut aco);

        if (*termpp).is_null() || (*term).buf_handle == 0 {
            return;
        }

        if !term_may_alloc_scrollback(term, buf) {
            std::process::abort();
        }

        let state = vterm_obtain_state((*term).vt);
        for i in 0..16 {
            let mut var = [0 as c_char; 64];
            vim_snprintf!(
                var.as_mut_ptr(),
                var.len(),
                c"terminal_color_%d".as_ptr(),
                i as c_int,
            );
            let name = get_config_string(buf, var.as_mut_ptr());
            if !name.is_null() {
                let mut dummy: c_int = 0;
                let color_val = name_to_color(name, &mut dummy);
                if color_val != -1 {
                    let color = VTermColor {
                        bytes: [
                            0,
                            ((color_val >> 16) & 0xFF) as u8,
                            ((color_val >> 8) & 0xFF) as u8,
                            (color_val & 0xFF) as u8,
                        ],
                    };
                    vterm_state_set_palette_color(state, i as c_int, &color);
                    (*term).color_set[i] = true;
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_close(termpp: *mut *mut Terminal, status: c_int) {
    #[cfg(test)]
    {
        let _ = (termpp, status);
    }
    #[cfg(not(test))]
    unsafe {
        let term = *termpp;
        if (*term).destroy {
            return;
        }

        let mut only_destroy = false;
        let buf = handle_get_buffer((*term).buf_handle);

        if (*term).closed {
            only_destroy = true;
        } else {
            if !exiting {
                let mut height: c_int = 0;
                let mut width: c_int = 0;
                vterm_get_size((*term).vt, &mut height, &mut width);
                if status != -1 && (*term).cursor.row == height - 1 {
                    fetch_row(term, (*term).cursor.row, width);
                    if (*term).textbuf[0] != NUL {
                        terminal_receive(term, c"\r\n".as_ptr(), 2);
                    }
                }
                block_autocmds();
                refresh_terminal(term);
                unblock_autocmds();
            }
            (*term).closed = true;
        }

        let mut pos = if !buf.is_null() {
            (*buf).b_ml.ml_line_count - 1
        } else {
            0
        };
        if status == -1 || exiting {
            (*term).buf_handle = 0;
            if !buf.is_null() {
                (*buf).terminal = ptr::null_mut();
            }
            if (*term).refcount == 0 {
                (*term).destroy = true;
                if let Some(close_cb) = (*term).opts.close_cb {
                    close_cb((*term).opts.data);
                }
            }
        } else if !only_destroy {
            let mut wp = firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    (*wp).w_redr_status = true;
                }
                wp = (*wp).w_next;
            }
            pos = row_to_linenr(term, (*term).cursor.row).min(pos);
        }

        if only_destroy {
            return;
        }

        if status >= 0 {
            did_check_timestamps = false;
            need_check_timestamps = true;
        }

        if !buf.is_null() && !is_autocmd_blocked() {
            let mut save_v_event: save_v_event_T = std::mem::zeroed();
            let dict = get_v_event(&mut save_v_event);
            tv_dict_add_nr(dict, c"status".as_ptr(), 6, i64::from(status));
            tv_dict_set_keys_readonly(dict);

            let mut data_items = [KeyValuePair {
                key: cstr_slice_as_string(b"pos\0"),
                value: int_obj(i64::from(pos)),
            }];
            let mut data_obj = Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData {
                    dict: Dict {
                        items: data_items.as_mut_ptr(),
                        size: 1,
                        capacity: 1,
                    },
                },
            };

            apply_autocmds_group(
                EVENT_TERMCLOSE,
                ptr::null_mut(),
                ptr::null_mut(),
                status >= 0,
                AUGROUP_ALL,
                buf,
                curwin,
                ptr::null_mut(),
                &mut data_obj,
                false,
            );

            restore_v_event(dict, (&raw const save_v_event).cast_mut());
        }
    }
}

unsafe extern "C" fn terminal_state_change_event(argv: *mut *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        let buf_handle = (*argv as isize) as handle_T;
        let buf = handle_get_buffer(buf_handle);
        if !buf.is_null() && !(*buf).terminal.is_null() {
            redraw_buf_line_later(buf, (*buf).b_ml.ml_line_count, false);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_set_state(term: *mut Terminal, suspended: bool) {
    #[cfg(test)]
    {
        let _ = (term, suspended);
    }
    #[cfg(not(test))]
    unsafe {
        if (*term).suspended != suspended {
            let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
            argv[0] = ((*term).buf_handle as isize) as *mut c_void;
            multiqueue_put_event(
                (*watcher_fields(refresh_timer_ptr())).events,
                Event {
                    handler: Some(terminal_state_change_event),
                    argv,
                },
            );
        }
        (*term).suspended = suspended;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_check_size(term: *mut Terminal) {
    #[cfg(test)]
    {
        let _ = term;
    }
    #[cfg(not(test))]
    unsafe {
        if (*term).closed {
            return;
        }

        let mut curwidth: c_int = 0;
        let mut curheight: c_int = 0;
        vterm_get_size((*term).vt, &mut curheight, &mut curwidth);
        let mut width: u16 = 0;
        let mut height: u16 = 0;

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if !is_ctx_win(wp)
                    && !(*wp).w_buffer.is_null()
                    && (*(*wp).w_buffer).terminal.cast::<Terminal>() == term
                {
                    let win_width = ((*wp).w_view_width - win_col_off(wp)).max(0) as u16;
                    width = width.max(win_width);
                    height = height.max((*wp).w_view_height as u16);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        if (curheight == c_int::from(height) && curwidth == c_int::from(width))
            || height == 0
            || width == 0
        {
            return;
        }

        vterm_set_size((*term).vt, c_int::from(height), c_int::from(width));
        vterm_screen_flush_damage((*term).vts);
        (*term).pending.resize = true;
        invalidate_terminal(term, -1, -1);
    }
}

#[cfg(not(test))]
unsafe fn set_terminal_winopts(s: *mut TerminalState) {
    unsafe {
        debug_assert!((*s).save_curwin_handle == 0);

        (*s).save_curwin_handle = (*curwin).handle;
        (*s).save_w_p_cul = (*curwin).w_onebuf_opt.wo_cul != 0;
        (*s).save_w_p_culopt = ptr::null_mut();
        (*s).save_w_p_culopt_flags = (*curwin).w_p_culopt_flags;
        (*s).save_w_p_cuc = (*curwin).w_onebuf_opt.wo_cuc;
        (*s).save_w_p_so = (*curwin).w_onebuf_opt.wo_so;
        (*s).save_w_p_siso = (*curwin).w_onebuf_opt.wo_siso;

        if (*curwin).w_onebuf_opt.wo_cul != 0
            && ((*curwin).w_p_culopt_flags & kOptCuloptFlagNumber) != 0
        {
            if !strequal((*curwin).w_onebuf_opt.wo_culopt, c"number".as_ptr()) {
                (*s).save_w_p_culopt = (*curwin).w_onebuf_opt.wo_culopt;
                (*curwin).w_onebuf_opt.wo_culopt = xstrdup(c"number".as_ptr());
            }
            (*curwin).w_p_culopt_flags = kOptCuloptFlagNumber;
        } else {
            (*curwin).w_onebuf_opt.wo_cul = 0;
        }
        (*curwin).w_onebuf_opt.wo_cuc = 0;
        (*curwin).w_onebuf_opt.wo_so = 0;
        (*curwin).w_onebuf_opt.wo_siso = 0;

        if (*curwin).w_onebuf_opt.wo_cuc != (*s).save_w_p_cuc {
            redraw_later(curwin, UPD_SOME_VALID);
        } else if ((*curwin).w_onebuf_opt.wo_cul != 0) != (*s).save_w_p_cul
            || ((*curwin).w_onebuf_opt.wo_cul != 0
                && (*curwin).w_p_culopt_flags != (*s).save_w_p_culopt_flags)
        {
            redraw_later(curwin, UPD_VALID);
        }
    }
}

#[cfg(not(test))]
unsafe fn unset_terminal_winopts(s: *mut TerminalState) {
    unsafe {
        debug_assert!((*s).save_curwin_handle != 0);

        let wp = handle_get_window((*s).save_curwin_handle);
        if !wp.is_null() {
            let mut winopts: *mut winopt_T = ptr::null_mut();
            if (*(*wp).w_buffer).b_fnum != (*(*s).term).buf_handle {
                let buf = handle_get_buffer((*(*s).term).buf_handle);
                if !buf.is_null() {
                    for i in 0..(*buf).b_wininfo.size {
                        let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
                        if (*wip).wi_win == wp && (*wip).wi_optset {
                            winopts = &raw mut (*wip).wi_opt;
                            break;
                        }
                    }
                }
            } else {
                winopts = &raw mut (*wp).w_onebuf_opt;
                if win_valid(wp) {
                    if (*s).save_w_p_cuc != (*wp).w_onebuf_opt.wo_cuc {
                        redraw_later(wp, UPD_SOME_VALID);
                    } else if (*s).save_w_p_cul != ((*wp).w_onebuf_opt.wo_cul != 0)
                        || ((*s).save_w_p_cul
                            && (*s).save_w_p_culopt_flags != (*wp).w_p_culopt_flags)
                    {
                        redraw_later(wp, UPD_VALID);
                    }
                }
                (*wp).w_p_culopt_flags = (*s).save_w_p_culopt_flags;
            }

            if !winopts.is_null() {
                if !(*s).save_w_p_culopt.is_null() {
                    free_string_option((*winopts).wo_culopt);
                    (*winopts).wo_culopt = (*s).save_w_p_culopt;
                    (*s).save_w_p_culopt = ptr::null_mut();
                }
                (*winopts).wo_cul = c_int::from((*s).save_w_p_cul);
                (*winopts).wo_cuc = (*s).save_w_p_cuc;
                (*winopts).wo_so = (*s).save_w_p_so;
                (*winopts).wo_siso = (*s).save_w_p_siso;
            }
        }

        free_string_option((*s).save_w_p_culopt);
        (*s).save_curwin_handle = 0;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_enter() -> bool {
    #[cfg(test)]
    {
        false
    }
    #[cfg(not(test))]
    unsafe {
        let buf = curbuf;
        debug_assert!(!(*buf).terminal.is_null());
        let mut s: TerminalState = std::mem::zeroed();
        s.term = (*buf).terminal.cast::<Terminal>();
        s.cursor_visible = true;
        Ins.stop_insert_mode = false;

        terminal_check_size(s.term);

        atom_term_enter();
        let save_state = State;
        s.save_rd = RedrawingDisabled;
        State = MODE_TERMINAL;
        mapped_ctrl_c |= MODE_TERMINAL;
        RedrawingDisabled = 0;

        set_terminal_winopts(&mut s);

        (*s.term).pending.cursor = true;
        adjust_topline_cursor(s.term, buf, 0);
        showmode();
        ui_cursor_shape();

        terminal_focus(s.term, true);
        (*curbuf).b_last_changedtick_i = buf_get_changedtick(curbuf);

        (*s.term).refcount += 1;
        apply_autocmds(
            EVENT_TERMENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        may_trigger_modechanged();
        (*s.term).refcount -= 1;
        if (*s.term).buf_handle == 0 {
            s.close = true;
        }

        s.state.execute = Some(terminal_execute);
        s.state.check = Some(terminal_check);
        state_enter(&mut s.state);

        if !s.got_bsl_o {
            restart_edit = 0;
        }
        State = save_state;
        RedrawingDisabled = s.save_rd;
        if !s.cursor_visible {
            ui_busy_stop();
        }

        let _ = parse_shape_opt(SHAPE_CURSOR);

        unset_terminal_winopts(&mut s);

        terminal_focus(s.term, false);
        (*curbuf).b_last_changedtick = buf_get_changedtick(curbuf);

        if (*curbuf).terminal.cast::<Terminal>() == s.term && !s.close {
            terminal_check_cursor();
        }
        if restart_edit != 0 {
            showmode();
        } else {
            unshowmode(true);
        }
        ui_cursor_shape();

        if s.close {
            (*s.term).refcount += 1;
        }
        apply_autocmds(
            EVENT_TERMLEAVE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        if s.close {
            (*s.term).refcount -= 1;
            let buf_handle = (*s.term).buf_handle;
            (*s.term).destroy = true;
            if let Some(close_cb) = (*s.term).opts.close_cb {
                close_cb((*s.term).opts.data);
            }
            if buf_handle != 0 {
                do_buffer(DOBUF_WIPE, DOBUF_FIRST, FORWARD, buf_handle, 1);
            }
        }

        s.got_bsl_o
    }
}

#[cfg(not(test))]
unsafe fn terminal_check_cursor() {
    unsafe {
        let term = (*curbuf).terminal.cast::<Terminal>();
        (*curwin).w_cursor.lnum = (*curbuf)
            .b_ml
            .ml_line_count
            .min(row_to_linenr(term, (*term).cursor.row));
        let topline = ((*curbuf).b_ml.ml_line_count - (*curwin).w_view_height + 1).max(1);
        if topline != (*curwin).w_topline {
            set_topline(curwin, topline);
        }
        if (*term).suspended && (State & MODE_TERMINAL) != 0 {
            (*curwin).w_cursor = tab_pos_T {
                lnum: (*curbuf).b_ml.ml_line_count,
                col: 0,
                coladd: 0,
            };
        } else {
            let off = if (State & MODE_TERMINAL) != 0 {
                0
            } else if (*curwin).w_onebuf_opt.wo_rl != 0 {
                1
            } else {
                -1
            };
            coladvance(curwin, ((*term).cursor.col + off).max(0));
        }
    }
}

#[cfg(not(test))]
unsafe fn terminal_check_focus(s: *mut TerminalState) -> bool {
    unsafe {
        if (*curbuf).terminal.is_null() {
            return false;
        }

        if (*s).save_curwin_handle != (*curwin).handle {
            unset_terminal_winopts(s);
            set_terminal_winopts(s);
        }
        let cur_term = (*curbuf).terminal.cast::<Terminal>();
        if (*s).term != cur_term {
            terminal_focus((*s).term, false);
            if (*s).close {
                (*(*s).term).destroy = true;
                if let Some(close_cb) = (*(*s).term).opts.close_cb {
                    close_cb((*(*s).term).opts.data);
                }
                (*s).close = false;
            }

            (*s).term = cur_term;
            (*(*s).term).pending.cursor = true;
            invalidate_terminal((*s).term, -1, -1);
            terminal_focus((*s).term, true);
        }
        true
    }
}

unsafe extern "C" fn terminal_check(state: *mut VimState) -> c_int {
    #[cfg(test)]
    {
        let _ = state;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let s = state.cast::<TerminalState>();

        debug_assert!(
            !(*s).close
                || ((*(*s).term).buf_handle == 0
                    && (*s).term != (*curbuf).terminal.cast::<Terminal>())
        );

        if Ins.stop_insert_mode || !terminal_check_focus(s) {
            return 0;
        }

        terminal_check_refresh();

        terminal_check_cursor();
        validate_cursor(curwin);

        (*(*s).term).refcount += 1;
        if has_event(EVENT_TEXTCHANGEDT)
            && (*curbuf).b_last_changedtick_i != buf_get_changedtick(curbuf)
        {
            apply_autocmds(
                EVENT_TEXTCHANGEDT,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            (*curbuf).b_last_changedtick_i = buf_get_changedtick(curbuf);
        }
        may_trigger_win_scrolled_resized();
        (*(*s).term).refcount -= 1;
        if (*(*s).term).buf_handle == 0 {
            (*s).close = true;
        }

        if !terminal_check_focus(s) {
            return 0;
        }
        terminal_check_cursor();
        validate_cursor(curwin);

        show_cursor_info_later(false);
        if must_redraw != 0 {
            update_screen();
        } else {
            redraw_statuslines();
            if clear_cmdline || redraw_cmdline || redraw_mode {
                showmode();
            }
        }

        setcursor();
        refresh_cursor((*s).term, &mut (*s).cursor_visible);
        ui_flush();
        1
    }
}

unsafe extern "C" fn terminal_execute(state: *mut VimState, key: c_int) -> c_int {
    #[cfg(test)]
    {
        let _ = (state, key);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let s = state.cast::<TerminalState>();
        let mut tmp_mod_mask = mod_mask;
        let mod_key = merge_modifiers(key, &mut tmp_mod_mask);

        match mod_key {
            K_LEFTMOUSE | K_LEFTDRAG | K_LEFTRELEASE | K_MIDDLEMOUSE | K_MIDDLEDRAG
            | K_MIDDLERELEASE | K_RIGHTMOUSE | K_RIGHTDRAG | K_RIGHTRELEASE | K_X1MOUSE
            | K_X1DRAG | K_X1RELEASE | K_X2MOUSE | K_X2DRAG | K_X2RELEASE | K_MOUSEDOWN
            | K_MOUSEUP | K_MOUSELEFT | K_MOUSERIGHT | K_MOUSEMOVE => {
                if send_mouse_event((*s).term, key) {
                    return 0;
                }
            }
            K_PASTE_START => {
                paste_repeat(1);
            }
            K_EVENT => {
                (*(*s).term).refcount += 1;
                state_handle_k_event();
                (*(*s).term).refcount -= 1;
                if (*(*s).term).buf_handle == 0 {
                    (*s).close = true;
                }
            }
            K_COMMAND => {
                do_cmdline(ptr::null_mut(), Some(getcmdkeycmd), ptr::null_mut(), 0);
            }
            K_LUA => {
                map_execute_lua(false, false);
            }
            K_IGNORE | K_NOP => {}
            _ => {
                if mod_key == Ctrl_N && (*s).got_bsl {
                    return 0;
                }
                if mod_key == Ctrl_O && (*s).got_bsl {
                    (*s).got_bsl_o = true;
                    restart_edit = b'I' as c_int;
                    return 0;
                }
                if mod_key == Ctrl_C {
                    got_int = false;
                }
                if mod_key == Ctrl_BSL && !(*s).got_bsl {
                    (*s).got_bsl = true;
                    return 1;
                }
                if (*(*s).term).suspended {
                    if let Some(resume_cb) = (*(*s).term).opts.resume_cb {
                        resume_cb((*(*s).term).opts.data);
                    }
                    terminal_set_state((*s).term, false);
                    return 1;
                }
                if (*(*s).term).closed {
                    (*s).close = true;
                    return 0;
                }

                (*s).got_bsl = false;
                terminal_send_key((*s).term, key);
            }
        }

        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_destroy(termpp: *mut *mut Terminal) {
    #[cfg(test)]
    {
        let _ = termpp;
    }
    #[cfg(not(test))]
    unsafe {
        let term = *termpp;
        let buf = handle_get_buffer((*term).buf_handle);
        if !buf.is_null() {
            (*term).buf_handle = 0;
            (*buf).terminal = ptr::null_mut();
        }

        if (*term).refcount == 0 {
            if set_has_term(&raw mut INVALIDATED_TERMINALS, term) {
                block_autocmds();
                refresh_terminal(term);
                unblock_autocmds();
                set_del_term(&raw mut INVALIDATED_TERMINALS, term);
            }
            for i in 0..(*term).sb_current {
                xfree((*(*term).sb_buffer.add(i)).cast::<c_void>());
            }
            xfree((*term).sb_buffer.cast::<c_void>());
            xfree((*term).title.cast::<c_void>());
            xfree((*term).selection_buffer.cast::<c_void>());
            sb_destroy(&raw mut (*term).selection);
            sb_destroy(&raw mut (*term).termrequest_buffer);
            vterm_free((*term).vt);
            multiqueue_free((*term).pending.events);
            xfree(term.cast::<c_void>());
            *termpp = ptr::null_mut();
        }
    }
}

unsafe fn terminal_send(term: *mut Terminal, data: *const c_char, size: usize) {
    unsafe {
        if (*term).closed {
            return;
        }
        if !(*term).pending.send.is_null() {
            sb_concat_len((*term).pending.send, data, size);
            return;
        }
        if let Some(write_cb) = (*term).opts.write_cb {
            write_cb(data, size, (*term).opts.data);
        }
    }
}

#[cfg(not(test))]
unsafe fn is_filter_char(c: c_int) -> bool {
    let flag = match c {
        0x08 => kOptTpfFlagBS,
        0x09 => kOptTpfFlagHT,
        0x0A | 0x0D => 0,
        0x0C => kOptTpfFlagFF,
        0x1b => kOptTpfFlagESC,
        0x7F => kOptTpfFlagDEL,
        _ => {
            if c < b' ' as c_int {
                kOptTpfFlagC0
            } else if (0x80..=0x9F).contains(&c) {
                kOptTpfFlagC1
            } else {
                0
            }
        }
    };
    unsafe { (tpf_flags & flag) != 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_set_streamed_paste(term: *mut Terminal, streamed: bool) {
    #[cfg(test)]
    {
        let _ = (term, streamed);
    }
    #[cfg(not(test))]
    unsafe {
        if (*term).streamed_paste != streamed {
            let cur_term = (*curbuf).terminal.cast::<Terminal>();
            if streamed {
                vterm_keyboard_start_paste((*cur_term).vt);
            } else {
                vterm_keyboard_end_paste((*cur_term).vt);
            }
        }
        (*term).streamed_paste = streamed;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_paste(count: c_int, y_array: *mut NvimString, y_size: usize) {
    #[cfg(test)]
    {
        let _ = (count, y_array, y_size);
    }
    #[cfg(not(test))]
    unsafe {
        if y_size == 0 {
            return;
        }
        let cur_term = (*curbuf).terminal.cast::<Terminal>();
        if !(*cur_term).streamed_paste {
            vterm_keyboard_start_paste((*cur_term).vt);
        }
        let mut buff_len = (*y_array).size;
        let mut buff = xmalloc(buff_len).cast::<c_char>();
        for _ in 0..count {
            for j in 0..y_size {
                if j > 0 {
                    let term_now = (*curbuf).terminal.cast::<Terminal>();
                    #[cfg(windows)]
                    terminal_send(term_now, c"\r\n".as_ptr(), 2);
                    #[cfg(not(windows))]
                    terminal_send(term_now, c"\n".as_ptr(), 1);
                }
                let item = *y_array.add(j);
                if item.size > buff_len {
                    buff = xrealloc(buff.cast::<c_void>(), item.size).cast::<c_char>();
                    buff_len = item.size;
                }
                let mut dst = buff;
                let mut src = item.data;
                while *src != NUL {
                    let len = utf_ptr2len(src) as usize;
                    let c = utf_ptr2char(src);
                    if !is_filter_char(c) {
                        ptr::copy_nonoverlapping(src, dst, len);
                        dst = dst.add(len);
                    }
                    src = src.add(len);
                }
                let term_now = (*curbuf).terminal.cast::<Terminal>();
                terminal_send(term_now, buff, dst.offset_from(buff) as usize);
            }
        }
        xfree(buff.cast::<c_void>());
        let term_end = (*curbuf).terminal.cast::<Terminal>();
        if !(*term_end).streamed_paste {
            vterm_keyboard_end_paste((*term_end).vt);
        }
    }
}

#[cfg(not(test))]
unsafe fn terminal_send_key(term: *mut Terminal, mut c: c_int) {
    unsafe {
        let mut mod_: VTermModifier = VTERM_MOD_NONE;
        if c == K_ZERO {
            c = Ctrl_AT;
        }

        let key = convert_key(&mut c, mod_mask, &mut mod_);
        if key != VTERM_KEY_NONE {
            vterm_keyboard_key((*term).vt, key, mod_);
        } else if !is_special(c) {
            vterm_keyboard_unichar((*term).vt, c as u32, mod_);
        }
    }
}

unsafe extern "C" fn on_sync_flush(argv: *mut *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        if exiting {
            return;
        }
        let buf_handle = (*argv as isize) as handle_T;
        let buf = handle_get_buffer(buf_handle);
        if buf.is_null() || (*buf).terminal.is_null() {
            return;
        }
        block_autocmds();
        refresh_terminal((*buf).terminal.cast::<Terminal>());
        unblock_autocmds();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_receive(term: *mut Terminal, data: *const c_char, len: usize) {
    #[cfg(test)]
    {
        let _ = (term, data, len);
    }
    #[cfg(not(test))]
    unsafe {
        if data.is_null() {
            return;
        }

        if (*term).opts.force_crlf {
            let mut crlf_data = KVec {
                size: 0,
                capacity: 0,
                items: ptr::null_mut(),
            };
            for i in 0..len {
                let b = *data.add(i);
                if b == b'\n' as c_char && (i == 0 || *data.add(i - 1) != b'\r' as c_char) {
                    sb_push(&mut crlf_data, b'\r' as c_char);
                }
                sb_push(&mut crlf_data, b);
            }
            vterm_input_write((*term).vt, crlf_data.items, crlf_data.size);
            sb_destroy(&mut crlf_data);
        } else {
            vterm_input_write((*term).vt, data, len);
        }
        vterm_screen_flush_damage((*term).vts);

        if (*term).sync_flush_pending {
            (*term).sync_flush_pending = false;
            let mut height: c_int = 0;
            vterm_get_size((*term).vt, &mut height, ptr::null_mut());
            (*term).invalid_start = 0;
            (*term).invalid_end = height;
            let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
            argv[0] = ((*term).buf_handle as isize) as *mut c_void;
            multiqueue_put_event(
                (*loop_queues((&raw mut main_loop).cast())).events,
                Event {
                    handler: Some(on_sync_flush),
                    argv,
                },
            );
        }
    }
}

unsafe fn get_rgb(state: *const VTermState, mut color: VTermColor) -> c_int {
    unsafe {
        vterm_state_convert_color_to_rgb(state, &mut color);
        let r = c_int::from(color.bytes[1]);
        let g = c_int::from(color.bytes[2]);
        let b = c_int::from(color.bytes[3]);
        (r << 16) | (g << 8) | b
    }
}

const fn get_underline_hl_flag(attrs: VTermScreenCellAttrs) -> c_int {
    match attrs.underline() {
        VTERM_UNDERLINE_OFF => 0,
        VTERM_UNDERLINE_SINGLE => HL_UNDERLINE,
        VTERM_UNDERLINE_DOUBLE => HL_UNDERDOUBLE,
        VTERM_UNDERLINE_CURLY => HL_UNDERCURL,
        _ => HL_UNDERLINE,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_get_line_attributes(
    term: *mut Terminal,
    _wp: *mut win_T,
    linenr: c_int,
    term_attrs: *mut c_int,
) {
    #[cfg(test)]
    {
        let _ = (term, linenr, term_attrs);
    }
    #[cfg(not(test))]
    unsafe {
        let mut height: c_int = 0;
        let mut width: c_int = 0;
        vterm_get_size((*term).vt, &mut height, &mut width);
        let state = vterm_obtain_state((*term).vt);
        debug_assert!(linenr != 0);
        let row = linenr_to_row(term, linenr);
        if row >= height {
            return;
        }

        width = width.min(TERM_ATTRS_MAX);
        for col in 0..width {
            let mut cell: VTermScreenCell = std::mem::zeroed();
            let color_valid = fetch_cell(term, row, col, &mut cell);
            let fg_type = cell.fg.bytes[0];
            let bg_type = cell.bg.bytes[0];
            let fg_default = !color_valid || (fg_type & VTERM_COLOR_DEFAULT_FG) != 0;
            let bg_default = !color_valid || (bg_type & VTERM_COLOR_DEFAULT_BG) != 0;

            let vt_fg = if fg_default {
                -1
            } else {
                get_rgb(state, cell.fg)
            };
            let vt_bg = if bg_default {
                -1
            } else {
                get_rgb(state, cell.bg)
            };

            let fg_indexed = (fg_type & VTERM_COLOR_INDEXED) != 0;
            let bg_indexed = (bg_type & VTERM_COLOR_INDEXED) != 0;

            let vt_fg_idx: i16 = if !fg_default && fg_indexed {
                i16::from(cell.fg.bytes[1]) + 1
            } else {
                0
            };
            let vt_bg_idx: i16 = if !bg_default && bg_indexed {
                i16::from(cell.bg.bytes[1]) + 1
            } else {
                0
            };

            let fg_set =
                vt_fg_idx != 0 && vt_fg_idx <= 16 && (*term).color_set[(vt_fg_idx - 1) as usize];
            let bg_set =
                vt_bg_idx != 0 && vt_bg_idx <= 16 && (*term).color_set[(vt_bg_idx - 1) as usize];

            let hl_attrs = (if cell.attrs.bold() != 0 { HL_BOLD } else { 0 })
                | (if cell.attrs.dim() != 0 { HL_DIM } else { 0 })
                | (if cell.attrs.blink() != 0 { HL_BLINK } else { 0 })
                | (if cell.attrs.conceal() != 0 {
                    HL_CONCEALED
                } else {
                    0
                })
                | (if cell.attrs.overline() != 0 {
                    HL_OVERLINE
                } else {
                    0
                })
                | (if cell.attrs.italic() != 0 {
                    HL_ITALIC
                } else {
                    0
                })
                | (if cell.attrs.reverse() != 0 {
                    HL_INVERSE
                } else {
                    0
                })
                | get_underline_hl_flag(cell.attrs)
                | (if cell.attrs.strike() != 0 {
                    HL_STRIKETHROUGH
                } else {
                    0
                })
                | (if fg_indexed && !fg_set {
                    HL_FG_INDEXED
                } else {
                    0
                })
                | (if bg_indexed && !bg_set {
                    HL_BG_INDEXED
                } else {
                    0
                });

            let mut attr_id: c_int = 0;
            if hl_attrs != 0 || !fg_default || !bg_default {
                let mut attrs = HlAttrs {
                    cterm_ae_attr: hl_attrs,
                    cterm_fg_color: vt_fg_idx,
                    cterm_bg_color: vt_bg_idx,
                    rgb_ae_attr: hl_attrs,
                    rgb_fg_color: vt_fg,
                    rgb_bg_color: vt_bg,
                    rgb_sp_color: -1,
                    hl_blend: -1,
                    font: 0,
                    url: -1,
                };
                attr_id = hl_get_term_attr(&mut attrs);
            }

            if cell.uri > 0 {
                attr_id = hl_combine_attr(attr_id, cell.uri);
            }

            *term_attrs.add(col as usize) = attr_id;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_buf(term: *const Terminal) -> Buffer {
    unsafe { (*term).buf_handle }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_running(term: *const Terminal) -> bool {
    unsafe { !(*term).closed }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_suspended(term: *const Terminal) -> bool {
    unsafe { (*term).suspended }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_notify_theme(term: *mut Terminal, dark: bool) {
    unsafe {
        if !(*term).theme_updates {
            return;
        }
        let seq: &[u8; 9] = if dark {
            b"\x1b[?997;1n"
        } else {
            b"\x1b[?997;2n"
        };
        terminal_send(term, seq.as_ptr().cast::<c_char>(), seq.len());
    }
}

unsafe fn terminal_focus(term: *const Terminal, focus: bool) {
    unsafe {
        let state = vterm_obtain_state((*term).vt);
        if focus {
            vterm_state_focus_in(state);
        } else {
            vterm_state_focus_out(state);
        }
    }
}

unsafe extern "C" fn term_damage(rect: VTermRect, data: *mut c_void) -> c_int {
    unsafe {
        invalidate_terminal(data.cast::<Terminal>(), rect.start_row, rect.end_row);
        1
    }
}

unsafe extern "C" fn term_moverect(dest: VTermRect, src: VTermRect, data: *mut c_void) -> c_int {
    unsafe {
        invalidate_terminal(
            data.cast::<Terminal>(),
            dest.start_row.min(src.start_row),
            dest.end_row.max(src.end_row),
        );
        1
    }
}

unsafe extern "C" fn term_movecursor(
    new_pos: VTermPos,
    _old_pos: VTermPos,
    _visible: c_int,
    data: *mut c_void,
) -> c_int {
    unsafe {
        let term = data.cast::<Terminal>();
        (*term).cursor.row = new_pos.row;
        (*term).cursor.col = new_pos.col;
        invalidate_terminal(term, -1, -1);
        1
    }
}

#[cfg(not(test))]
unsafe fn buf_set_term_title(buf: *mut buf_T, title: *const c_char, len: usize) {
    unsafe {
        if buf.is_null() {
            return;
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        (*buf).b_locked += 1;
        dict_set_var(
            (*buf).b_vars,
            cstr_slice_as_string(b"term_title\0"),
            str_obj(NvimString {
                data: title.cast_mut(),
                size: len,
            }),
            false,
            false,
            ptr::null_mut(),
            &mut err,
        );
        (*buf).b_locked -= 1;
        api_clear_error(&mut err);
        status_redraw_buf(buf);
    }
}

unsafe extern "C" fn term_settermprop(
    prop: VTermProp,
    val: *mut VTermValue,
    data: *mut c_void,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (prop, val, data);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let term = data.cast::<Terminal>();

        match prop {
            VTERM_PROP_ALTSCREEN => {
                (*term).in_altscreen = (*val).boolean != 0;
            }
            VTERM_PROP_CURSORVISIBLE => {
                (*term).cursor.visible = (*val).boolean != 0;
                invalidate_terminal(term, -1, -1);
            }
            VTERM_PROP_TITLE => {
                let buf = handle_get_buffer((*term).buf_handle);
                let frag = (*val).string;
                let flen = frag.len();

                if frag.initial() && frag.final_() {
                    buf_set_term_title(buf, frag.str_, flen);
                    return 1;
                }

                if frag.initial() {
                    (*term).title_len = 0;
                    (*term).title_size = flen.max(1024);
                    (*term).title =
                        xmalloc(size_of::<*mut c_char>() * (*term).title_size).cast::<c_char>();
                } else if (*term).title_len + flen > (*term).title_size {
                    (*term).title_size *= 2;
                    (*term).title = xrealloc(
                        (*term).title.cast::<c_void>(),
                        size_of::<*mut c_char>() * (*term).title_size,
                    )
                    .cast::<c_char>();
                }

                ptr::copy_nonoverlapping(frag.str_, (*term).title.add((*term).title_len), flen);
                (*term).title_len += flen;

                if frag.final_() {
                    buf_set_term_title(buf, (*term).title, (*term).title_len);
                    xfree((*term).title.cast::<c_void>());
                    (*term).title = ptr::null_mut();
                }
            }
            VTERM_PROP_MOUSE => {
                (*term).forward_mouse = (*val).number != 0;
            }
            VTERM_PROP_CURSORBLINK => {
                (*term).cursor.blink = (*val).boolean != 0;
                (*term).pending.cursor = true;
                invalidate_terminal(term, -1, -1);
            }
            VTERM_PROP_CURSORSHAPE => {
                (*term).cursor.shape = (*val).number;
                (*term).pending.cursor = true;
                invalidate_terminal(term, -1, -1);
            }
            VTERM_PROP_THEMEUPDATES => {
                (*term).theme_updates = (*val).boolean != 0;
            }
            VTERM_PROP_SYNCOUTPUT => {
                (*term).synchronized_output = (*val).boolean != 0;
                if (*val).boolean == 0 {
                    (*term).sync_flush_pending = true;
                }
            }
            _ => return 0,
        }

        1
    }
}

unsafe extern "C" fn term_bell(_data: *mut c_void) -> c_int {
    #[cfg(not(test))]
    unsafe {
        vim_beep(kOptBoFlagTerm);
    }
    1
}

unsafe extern "C" fn term_theme(dark: *mut bool, _data: *mut c_void) -> c_int {
    #[cfg(not(test))]
    unsafe {
        *dark = *p_bg == b'd' as c_char;
    }
    1
}

unsafe extern "C" fn term_sb_push(
    cols: c_int,
    cells: *const VTermScreenCell,
    data: *mut c_void,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (cols, cells, data);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let term = data.cast::<Terminal>();
        if !term_may_alloc_scrollback(term, ptr::null_mut()) {
            return 0;
        }
        debug_assert!((*term).sb_size > 0);

        let c = cols as usize;
        let mut sbrow: *mut ScrollbackLine = ptr::null_mut();
        if (*term).sb_current == (*term).sb_size {
            let last = *(*term).sb_buffer.add((*term).sb_current - 1);
            if (*last).cols == c {
                sbrow = last;
            } else {
                xfree(last.cast::<c_void>());
            }
            (*term).sb_deleted += 1;
            ptr::copy(
                (*term).sb_buffer,
                (*term).sb_buffer.add(1),
                (*term).sb_current - 1,
            );
        } else if (*term).sb_current > 0 {
            ptr::copy(
                (*term).sb_buffer,
                (*term).sb_buffer.add(1),
                (*term).sb_current,
            );
        }

        if sbrow.is_null() {
            sbrow = xmalloc(size_of::<ScrollbackLine>() + c * size_of::<VTermScreenCell>())
                .cast::<ScrollbackLine>();
            (*sbrow).cols = c;
        }

        *(*term).sb_buffer = sbrow;
        if (*term).sb_current < (*term).sb_size {
            (*term).sb_current += 1;
        }

        if (*term).sb_pending < (*term).sb_size as c_int {
            (*term).sb_pending += 1;
        }

        ptr::copy_nonoverlapping(cells, sbrow_cells(sbrow), c);
        if !(*term).synchronized_output {
            set_put_term(&raw mut INVALIDATED_TERMINALS, term);
        }

        1
    }
}

unsafe extern "C" fn term_sb_pop(
    cols: c_int,
    cells: *mut VTermScreenCell,
    data: *mut c_void,
) -> c_int {
    unsafe {
        let term = data.cast::<Terminal>();
        if (*term).sb_current == 0 {
            return 0;
        }

        if (*term).sb_pending > 0 {
            (*term).sb_pending -= 1;
        } else {
            (*term).old_height += 1;
        }

        let sbrow = *(*term).sb_buffer;
        (*term).sb_current -= 1;
        ptr::copy(
            (*term).sb_buffer.add(1),
            (*term).sb_buffer,
            (*term).sb_current,
        );

        let cols_usize = cols as usize;
        let cols_to_copy = cols_usize.min((*sbrow).cols);

        ptr::copy_nonoverlapping(sbrow_cells(sbrow), cells, cols_to_copy);
        for col in cols_to_copy..cols_usize {
            let cell = cells.add(col);
            (*cell).schar = 0;
            (*cell).width = 1;
        }

        xfree(sbrow.cast::<c_void>());
        if !(*term).synchronized_output {
            set_put_term(&raw mut INVALIDATED_TERMINALS, term);
        }

        1
    }
}

unsafe extern "C" fn term_sb_clear(data: *mut c_void) -> c_int {
    unsafe {
        let term = data.cast::<Terminal>();
        if (*term).in_altscreen || (*term).sb_size == 0 || (*term).sb_current == 0 {
            return 1;
        }

        for i in 0..(*term).sb_current {
            xfree((*(*term).sb_buffer.add(i)).cast::<c_void>());
        }

        (*term).sb_deleted += (*term).sb_current;
        (*term).sb_current = 0;
        (*term).sb_pending = 0;
        invalidate_terminal(term, -1, -1);

        1
    }
}

unsafe extern "C" fn term_clipboard_set(argv: *mut *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        let mask = (*argv.add(0) as isize) as VTermSelectionMask;
        let data = (*argv.add(1)).cast::<c_char>();

        let regname: c_char = match mask {
            VTERM_SELECTION_CLIPBOARD => b'+' as c_char,
            VTERM_SELECTION_PRIMARY => b'*' as c_char,
            _ => b'+' as c_char,
        };

        let lines = tv_list_alloc(kListLenMayKnow);
        let mut start = data;
        loop {
            let start_bytes = CStr::from_ptr(start).to_bytes();
            if let Some(pos) = start_bytes.iter().position(|&b| b == b'\n') {
                tv_list_append_string(lines, start, pos as isize);
                start = start.add(pos + 1);
            } else {
                break;
            }
        }
        tv_list_append_string(lines, start, -1);
        xfree(data.cast::<c_void>());

        let args = tv_list_alloc(3);
        tv_list_append_list(args, lines);

        let regtype: c_char = b'v' as c_char;
        tv_list_append_string(args, &regtype, 1);
        tv_list_append_string(args, &regname, 1);
        eval_call_provider(
            c"clipboard".as_ptr().cast_mut(),
            c"set".as_ptr().cast_mut(),
            args,
            true,
        );
    }
}

unsafe extern "C" fn term_selection_set(
    mask: VTermSelectionMask,
    frag: VTermStringFragment,
    user: *mut c_void,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (mask, frag, user);
        1
    }
    #[cfg(not(test))]
    unsafe {
        let term = user.cast::<Terminal>();
        if frag.initial() {
            (*term).selection.size = 0;
        }

        sb_concat_len(&raw mut (*term).selection, frag.str_, frag.len());

        if frag.final_() {
            let data = xmemdupz(
                (*term).selection.items.cast::<c_void>(),
                (*term).selection.size,
            );
            let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
            argv[0] = (mask as isize) as *mut c_void;
            argv[1] = data;
            multiqueue_put_event(
                (*loop_queues((&raw mut main_loop).cast())).events,
                Event {
                    handler: Some(term_clipboard_set),
                    argv,
                },
            );
        }

        1
    }
}

unsafe fn convert_modifiers(key: *mut c_int, modifiers: c_int, statep: *mut VTermModifier) {
    unsafe {
        if (modifiers & c_int::from(MOD_MASK_SHIFT)) != 0 {
            *statep |= VTERM_MOD_SHIFT;
        }
        if (modifiers & c_int::from(MOD_MASK_CTRL)) != 0 {
            *statep |= VTERM_MOD_CTRL;
            if (modifiers & c_int::from(MOD_MASK_SHIFT)) == 0
                && *key >= b'A' as c_int
                && *key <= b'Z' as c_int
            {
                *key += (b'a' - b'A') as c_int;
            }
        }
        if (modifiers & c_int::from(MOD_MASK_ALT)) != 0 {
            *statep |= VTERM_MOD_ALT;
        }
    }
}

unsafe fn convert_key(
    key: *mut c_int,
    mut modifiers: c_int,
    statep: *mut VTermModifier,
) -> VTermKey {
    unsafe {
        *key = unsimplify_key(*key, &mut modifiers);
        convert_modifiers(key, modifiers, statep);

        match *key {
            K_BS => VTERM_KEY_BACKSPACE,
            K_TAB | TAB => VTERM_KEY_TAB,
            Ctrl_M => VTERM_KEY_ENTER,
            ESC => VTERM_KEY_ESCAPE,
            K_UP => VTERM_KEY_UP,
            K_DOWN => VTERM_KEY_DOWN,
            K_LEFT => VTERM_KEY_LEFT,
            K_RIGHT => VTERM_KEY_RIGHT,
            K_INS => VTERM_KEY_INS,
            K_DEL => VTERM_KEY_DEL,
            K_HOME => VTERM_KEY_HOME,
            K_END => VTERM_KEY_END,
            K_PAGEUP => VTERM_KEY_PAGEUP,
            K_PAGEDOWN => VTERM_KEY_PAGEDOWN,
            K_K0 | K_KINS => VTERM_KEY_KP_0,
            K_K1 | K_KEND => VTERM_KEY_KP_1,
            K_K2 | K_KDOWN => VTERM_KEY_KP_2,
            K_K3 | K_KPAGEDOWN => VTERM_KEY_KP_3,
            K_K4 | K_KLEFT => VTERM_KEY_KP_4,
            K_K5 | K_KORIGIN => VTERM_KEY_KP_5,
            K_K6 | K_KRIGHT => VTERM_KEY_KP_6,
            K_K7 | K_KHOME => VTERM_KEY_KP_7,
            K_K8 | K_KUP => VTERM_KEY_KP_8,
            K_K9 | K_KPAGEUP => VTERM_KEY_KP_9,
            K_KDEL | K_KPOINT => VTERM_KEY_KP_PERIOD,
            K_KENTER => VTERM_KEY_KP_ENTER,
            K_KPLUS => VTERM_KEY_KP_PLUS,
            K_KMINUS => VTERM_KEY_KP_MINUS,
            K_KMULTIPLY => VTERM_KEY_KP_MULT,
            K_KDIVIDE => VTERM_KEY_KP_DIVIDE,
            K_F1 => vterm_key_function(1),
            K_F2 => vterm_key_function(2),
            K_F3 => vterm_key_function(3),
            K_F4 => vterm_key_function(4),
            K_F5 => vterm_key_function(5),
            K_F6 => vterm_key_function(6),
            K_F7 => vterm_key_function(7),
            K_F8 => vterm_key_function(8),
            K_F9 => vterm_key_function(9),
            K_F10 => vterm_key_function(10),
            K_F11 => vterm_key_function(11),
            K_F12 => vterm_key_function(12),
            K_F13 => vterm_key_function(13),
            K_F14 => vterm_key_function(14),
            K_F15 => vterm_key_function(15),
            K_F16 => vterm_key_function(16),
            K_F17 => vterm_key_function(17),
            K_F18 => vterm_key_function(18),
            K_F19 => vterm_key_function(19),
            K_F20 => vterm_key_function(20),
            K_F21 => vterm_key_function(21),
            K_F22 => vterm_key_function(22),
            K_F23 => vterm_key_function(23),
            K_F24 => vterm_key_function(24),
            K_F25 => vterm_key_function(25),
            K_F26 => vterm_key_function(26),
            K_F27 => vterm_key_function(27),
            K_F28 => vterm_key_function(28),
            K_F29 => vterm_key_function(29),
            K_F30 => vterm_key_function(30),
            K_F31 => vterm_key_function(31),
            K_F32 => vterm_key_function(32),
            K_F33 => vterm_key_function(33),
            K_F34 => vterm_key_function(34),
            K_F35 => vterm_key_function(35),
            K_F36 => vterm_key_function(36),
            K_F37 => vterm_key_function(37),
            K_F38 => vterm_key_function(38),
            K_F39 => vterm_key_function(39),
            K_F40 => vterm_key_function(40),
            K_F41 => vterm_key_function(41),
            K_F42 => vterm_key_function(42),
            K_F43 => vterm_key_function(43),
            K_F44 => vterm_key_function(44),
            K_F45 => vterm_key_function(45),
            K_F46 => vterm_key_function(46),
            K_F47 => vterm_key_function(47),
            K_F48 => vterm_key_function(48),
            K_F49 => vterm_key_function(49),
            K_F50 => vterm_key_function(50),
            K_F51 => vterm_key_function(51),
            K_F52 => vterm_key_function(52),
            K_F53 => vterm_key_function(53),
            K_F54 => vterm_key_function(54),
            K_F55 => vterm_key_function(55),
            K_F56 => vterm_key_function(56),
            K_F57 => vterm_key_function(57),
            K_F58 => vterm_key_function(58),
            K_F59 => vterm_key_function(59),
            K_F60 => vterm_key_function(60),
            K_F61 => vterm_key_function(61),
            K_F62 => vterm_key_function(62),
            K_F63 => vterm_key_function(63),
            _ => VTERM_KEY_NONE,
        }
    }
}

unsafe fn mouse_action(
    term: *mut Terminal,
    button: c_int,
    row: c_int,
    col: c_int,
    pressed: bool,
    mod_: VTermModifier,
) {
    unsafe {
        vterm_mouse_move((*term).vt, row, col, mod_);
        if button != 0 {
            vterm_mouse_button((*term).vt, button, pressed, mod_);
        }
    }
}

#[cfg(not(test))]
unsafe fn send_mouse_event(term: *mut Terminal, mut c: c_int) -> bool {
    unsafe {
        let mut row = mouse_row;
        let mut col = mouse_col;
        let mut grid = mouse_grid;
        let mouse_win = mouse_find_win_inner(&mut grid, &mut row, &mut col);
        if !mouse_win.is_null() {
            let offset: c_int;
            if !(*term).suspended
                && !(*term).closed
                && (*term).forward_mouse
                && (*(*mouse_win).w_buffer).terminal.cast::<Terminal>() == term
                && row >= 0
                && (grid > 1 || row + (*mouse_win).w_winbar_height < (*mouse_win).w_height)
                && {
                    offset = win_col_off(mouse_win);
                    col >= offset
                }
                && (grid > 1 || col < (*mouse_win).w_width)
            {
                let (button, pressed) = match c {
                    K_LEFTDRAG | K_LEFTMOUSE => (1, true),
                    K_LEFTRELEASE => (1, false),
                    K_MIDDLEDRAG | K_MIDDLEMOUSE => (2, true),
                    K_MIDDLERELEASE => (2, false),
                    K_RIGHTDRAG | K_RIGHTMOUSE => (3, true),
                    K_RIGHTRELEASE => (3, false),
                    K_X1DRAG | K_X1MOUSE => (8, true),
                    K_X1RELEASE => (8, false),
                    K_X2DRAG | K_X2MOUSE => (9, true),
                    K_X2RELEASE => (9, false),
                    K_MOUSEDOWN => (4, true),
                    K_MOUSEUP => (5, true),
                    K_MOUSELEFT => (7, true),
                    K_MOUSERIGHT => (6, true),
                    K_MOUSEMOVE => (0, false),
                    _ => return false,
                };

                let mut mod_: VTermModifier = VTERM_MOD_NONE;
                convert_modifiers(&mut c, mod_mask, &mut mod_);
                mouse_action(term, button, row, col - offset, pressed, mod_);
                return false;
            }

            if c == K_MOUSEUP || c == K_MOUSEDOWN || c == K_MOUSELEFT || c == K_MOUSERIGHT {
                let save_curwin = curwin;
                curwin = mouse_win;
                curbuf = (*curwin).w_buffer;

                let mut cap: cmdarg_T = std::mem::zeroed();
                let mut oa: oparg_T = std::mem::zeroed();
                clear_oparg(&mut oa);
                cap.oap = &mut oa;
                cap.cmdchar = c;
                cap.arg = match c {
                    K_MOUSEUP => MSCR_UP,
                    K_MOUSEDOWN => MSCR_DOWN,
                    K_MOUSELEFT => MSCR_LEFT,
                    K_MOUSERIGHT => MSCR_RIGHT,
                    _ => std::process::abort(),
                };

                do_mousescroll(&mut cap);

                (*curwin).w_redr_status = true;
                curwin = save_curwin;
                curbuf = (*curwin).w_buffer;
                redraw_later(mouse_win, UPD_NOT_VALID);
                invalidate_terminal(term, -1, -1);
                return mouse_win == curwin;
            }
        }

        if (c == K_LEFTRELEASE
            && !mouse_win.is_null()
            && (*(*mouse_win).w_buffer).terminal.cast::<Terminal>() == term)
            || c == K_MOUSEMOVE
        {
            return false;
        }

        requeue_key(vgetc_char, vgetc_mod_mask, 0, true);
        true
    }
}

unsafe fn fetch_row(term: *mut Terminal, row: c_int, end_col: c_int) {
    unsafe {
        let mut col: c_int = 0;
        let mut line_len: usize = 0;
        let textbuf_ptr = (&raw mut (*term).textbuf).cast::<c_char>();
        let mut ptr = textbuf_ptr;

        while col < end_col {
            let mut cell: VTermScreenCell = std::mem::zeroed();
            fetch_cell(term, row, col, &mut cell);
            if cell.schar != 0 {
                schar_get_adv(&mut ptr, cell.schar);
                line_len = ptr.offset_from(textbuf_ptr) as usize;
            } else {
                *ptr = b' ' as c_char;
                ptr = ptr.add(1);
            }
            col += c_int::from(cell.width);
        }

        *textbuf_ptr.add(line_len) = NUL;
    }
}

unsafe fn fetch_cell(
    term: *mut Terminal,
    row: c_int,
    col: c_int,
    cell: *mut VTermScreenCell,
) -> bool {
    unsafe {
        if row < 0 {
            let sbrow = *(*term).sb_buffer.add((-row - 1) as usize);
            if (col as usize) < (*sbrow).cols {
                *cell = *sbrow_cells(sbrow).add(col as usize);
            } else {
                *cell = VTermScreenCell {
                    schar: 0,
                    width: 1,
                    ..Default::default()
                };
                return false;
            }
        } else {
            vterm_screen_get_cell((*term).vts, VTermPos { row, col }, cell);
        }
        true
    }
}

unsafe fn invalidate_terminal(term: *mut Terminal, start_row: c_int, end_row: c_int) {
    unsafe {
        if start_row != -1 && end_row != -1 {
            (*term).invalid_start = (*term).invalid_start.min(start_row);
            (*term).invalid_end = (*term).invalid_end.max(end_row);
        }

        if (*term).synchronized_output {
            return;
        }

        set_put_term(&raw mut INVALIDATED_TERMINALS, term);
        #[cfg(not(test))]
        if !REFRESH_PENDING {
            time_watcher_start(
                refresh_timer_ptr(),
                Some(refresh_timer_cb),
                REFRESH_DELAY,
                0,
            );
            REFRESH_PENDING = true;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terminal_check_refresh() {
    #[cfg(not(test))]
    unsafe {
        multiqueue_process_events((*watcher_fields(refresh_timer_ptr())).events);
    }
}

#[cfg(not(test))]
unsafe fn refresh_terminal(term: *mut Terminal) {
    unsafe {
        let buf = handle_get_buffer((*term).buf_handle);
        if buf.is_null() {
            return;
        }
        let ml_before = (*buf).b_ml.ml_line_count;

        let resized = refresh_size(term, buf);
        refresh_scrollback(term, buf);
        refresh_screen(term, buf);

        let ml_added = (*buf).b_ml.ml_line_count - ml_before;
        adjust_topline_cursor(term, buf, ml_added);

        if resized {
            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if (*wp).w_buffer == buf && (*wp).w_leftcol != 0 {
                        (*wp).w_leftcol = 0;
                        curs_columns(wp, 1);
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
        }

        multiqueue_move_events(
            (*loop_queues((&raw mut main_loop).cast())).events,
            (*term).pending.events,
        );
    }
}

#[cfg(not(test))]
unsafe fn refresh_cursor(term: *mut Terminal, cursor_visible: *mut bool) {
    unsafe {
        if !is_focused(term) {
            return;
        }
        if (*term).cursor.visible != *cursor_visible {
            *cursor_visible = (*term).cursor.visible;
            if *cursor_visible {
                ui_busy_stop();
            } else {
                ui_busy_start();
            }
        }

        if !(*term).pending.cursor {
            return;
        }
        (*term).pending.cursor = false;

        if (*term).cursor.blink {
            shape_table[SHAPE_IDX_TERM].blinkon = 500;
            shape_table[SHAPE_IDX_TERM].blinkoff = 500;
        } else {
            shape_table[SHAPE_IDX_TERM].blinkon = 0;
            shape_table[SHAPE_IDX_TERM].blinkoff = 0;
        }

        match (*term).cursor.shape {
            VTERM_PROP_CURSORSHAPE_BLOCK => {
                shape_table[SHAPE_IDX_TERM].shape = CursorShape::SHAPE_BLOCK;
            }
            VTERM_PROP_CURSORSHAPE_UNDERLINE => {
                shape_table[SHAPE_IDX_TERM].shape = CursorShape::SHAPE_HOR;
                shape_table[SHAPE_IDX_TERM].percentage = 20;
            }
            VTERM_PROP_CURSORSHAPE_BAR_LEFT => {
                shape_table[SHAPE_IDX_TERM].shape = CursorShape::SHAPE_VER;
                shape_table[SHAPE_IDX_TERM].percentage = 25;
            }
            _ => {}
        }

        ui_mode_info_set();
    }
}

unsafe extern "C" fn refresh_timer_cb(_watcher: *mut TimeWatcher, _data: *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        REFRESH_PENDING = false;
        if exiting {
            return;
        }

        block_autocmds();
        let mut to_refresh = INVALIDATED_TERMINALS;
        INVALIDATED_TERMINALS = Set {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: ptr::null_mut(),
            },
            keys: ptr::null_mut(),
        };

        for i in 0..to_refresh.h.n_keys {
            let term = (*to_refresh.keys.add(i as usize)).cast::<Terminal>();
            if !(*term).synchronized_output {
                refresh_terminal(term);
            }
        }

        set_destroy_ptr(&mut to_refresh);
        unblock_autocmds();
    }
}

unsafe fn refresh_size(term: *mut Terminal, _buf: *mut buf_T) -> bool {
    unsafe {
        if !(*term).pending.resize || (*term).closed {
            return false;
        }

        (*term).pending.resize = false;
        let mut width: c_int = 0;
        let mut height: c_int = 0;
        vterm_get_size((*term).vt, &mut height, &mut width);
        (*term).invalid_start = 0;
        (*term).invalid_end = height;
        if let Some(resize_cb) = (*term).opts.resize_cb {
            resize_cb(width as u16, height as u16, (*term).opts.data);
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn on_scrollback_option_changed(term: *mut Terminal) {
    #[cfg(test)]
    {
        let _ = term;
    }
    #[cfg(not(test))]
    unsafe {
        if !(*term).sb_buffer.is_null() {
            refresh_terminal(term);
        }
    }
}

#[cfg(not(test))]
unsafe fn adjust_scrollback(term: *mut Terminal, buf: *mut buf_T) {
    unsafe {
        if (*buf).b_p_scbk < 1 {
            (*buf).b_p_scbk = SB_MAX;
        }
        let scbk = (*buf).b_p_scbk as usize;
        debug_assert!((*term).sb_current < usize::MAX);
        if (*term).sb_pending > 0 {
            std::process::abort();
        }

        if scbk < (*term).sb_current {
            let diff = (*term).sb_current - scbk;
            for _ in 0..diff {
                ml_delete_buf(buf, 1, false);
                (*term).sb_current -= 1;
                xfree((*(*term).sb_buffer.add((*term).sb_current)).cast::<c_void>());
            }
            mark_adjust_buf(
                buf,
                1,
                diff as linenr_T,
                MAXLNUM,
                -(diff as linenr_T),
                true,
                kMarkAdjustTerm,
                kExtmarkUndo,
            );
            deleted_lines_buf(buf, 1, diff as linenr_T);
        }

        let sb_region = size_of::<*mut ScrollbackLine>() * scbk;
        if scbk != (*term).sb_size {
            (*term).sb_buffer = xrealloc((*term).sb_buffer.cast::<c_void>(), sb_region)
                .cast::<*mut ScrollbackLine>();
        }

        (*term).sb_size = scbk;
    }
}

#[cfg(not(test))]
unsafe fn refresh_scrollback(term: *mut Terminal, buf: *mut buf_T) {
    unsafe {
        if let Some(read_pause_cb) = (*term).opts.read_pause_cb {
            read_pause_cb(true, (*term).opts.data);
        }

        let mut deleted = ((*term).sb_deleted - (*term).old_sb_deleted) as linenr_T;
        deleted = deleted.min((*buf).b_ml.ml_line_count);
        mark_adjust_buf(
            buf,
            1,
            deleted,
            MAXLNUM,
            -deleted,
            true,
            kMarkAdjustTerm,
            kExtmarkUndo,
        );
        (*term).old_sb_deleted = (*term).sb_deleted;

        let mut old_height = (*term).old_height;
        let mut width: c_int = 0;
        let mut height: c_int = 0;
        vterm_get_size((*term).vt, &mut height, &mut width);

        while deleted > 0 && (*buf).b_ml.ml_line_count > old_height {
            ml_delete_buf(buf, 1, false);
            deleted_lines_buf(buf, 1, 1);
            deleted -= 1;
        }

        old_height = old_height.min((*buf).b_ml.ml_line_count);
        let textbuf_ptr = (&raw mut (*term).textbuf).cast::<c_char>();
        while (*term).sb_pending > 0 {
            fetch_row(term, -(*term).sb_pending, width);
            let buf_index = (*buf).b_ml.ml_line_count - old_height;
            ml_append_buf(buf, buf_index, textbuf_ptr, 0, false);
            appended_lines_buf(buf, buf_index, 1);
            (*term).sb_pending -= 1;
        }

        let max_line_count = (*term).sb_current as c_int + height;
        while (*buf).b_ml.ml_line_count > max_line_count {
            ml_delete_buf(buf, (*buf).b_ml.ml_line_count, false);
            deleted_lines_buf(buf, (*buf).b_ml.ml_line_count, 1);
        }

        adjust_scrollback(term, buf);

        if let Some(read_pause_cb) = (*term).opts.read_pause_cb {
            read_pause_cb(false, (*term).opts.data);
        }
    }
}

#[cfg(not(test))]
unsafe fn refresh_screen(term: *mut Terminal, buf: *mut buf_T) {
    unsafe {
        let mut changed: c_int = 0;
        let mut added: c_int = 0;
        let mut height: c_int = 0;
        let mut width: c_int = 0;
        vterm_get_size((*term).vt, &mut height, &mut width);
        (*term).invalid_end = (*term).invalid_end.min(height);

        if (*term).invalid_start >= (*term).invalid_end {
            (*term).invalid_start = c_int::MAX;
            (*term).invalid_end = -1;
            return;
        }

        let textbuf_ptr = (&raw mut (*term).textbuf).cast::<c_char>();
        let mut r = (*term).invalid_start;
        let mut linenr = row_to_linenr(term, r);
        while r < (*term).invalid_end {
            fetch_row(term, r, width);

            if linenr <= (*buf).b_ml.ml_line_count {
                ml_replace_buf(buf, linenr, textbuf_ptr, true, false);
                changed += 1;
            } else {
                ml_append_buf(buf, linenr - 1, textbuf_ptr, 0, false);
                added += 1;
            }
            r += 1;
            linenr += 1;
        }
        (*term).old_height = height;

        let change_start = row_to_linenr(term, (*term).invalid_start);
        let change_end = change_start + changed;
        (*term).invalid_start = c_int::MAX;
        (*term).invalid_end = -1;
        changed_lines(buf, change_start, 0, change_end, added, true);
    }
}

#[cfg(not(test))]
unsafe fn adjust_topline_cursor(term: *mut Terminal, buf: *mut buf_T, added: c_int) {
    unsafe {
        let ml_end = (*buf).b_ml.ml_line_count;

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    if wp == curwin && is_focused(term) {
                        terminal_check_cursor();
                        wp = (*wp).w_next;
                        continue;
                    }

                    let following = ml_end == (*wp).w_cursor.lnum + added;
                    if following {
                        (*wp).w_cursor.lnum = ml_end;
                        set_topline(wp, ((*wp).w_cursor.lnum - (*wp).w_view_height + 1).max(1));
                    } else {
                        (*wp).w_cursor.lnum = (*wp).w_cursor.lnum.min(ml_end);
                    }
                    mb_check_adjust_col(wp.cast());
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        if ml_end == (*buf).b_last_cursor.mark.lnum + added {
            (*buf).b_last_cursor.mark.lnum = ml_end;
        }

        for i in 0..(*buf).b_wininfo.size {
            let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
            if ml_end == (*wip).wi_mark.mark.lnum + added {
                (*wip).wi_mark.mark.lnum = ml_end;
            }
        }
    }
}

#[inline]
unsafe fn row_to_linenr(term: *mut Terminal, row: c_int) -> c_int {
    if row != c_int::MAX {
        unsafe { row + (*term).sb_current as c_int + 1 }
    } else {
        c_int::MAX
    }
}

#[inline]
unsafe fn linenr_to_row(term: *mut Terminal, linenr: c_int) -> c_int {
    unsafe { linenr - (*term).sb_current as c_int - 1 }
}

#[cfg(not(test))]
#[inline]
unsafe fn is_focused(term: *mut Terminal) -> bool {
    unsafe { (State & MODE_TERMINAL) != 0 && (*curbuf).terminal.cast::<Terminal>() == term }
}

#[cfg(not(test))]
unsafe fn get_config_string(buf: *mut buf_T, key: *mut c_char) -> *mut c_char {
    unsafe {
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let key_str = NvimString {
            data: key,
            size: CStr::from_ptr(key).to_bytes().len(),
        };
        let mut obj = dict_get_value((*buf).b_vars, key_str, ptr::null_mut(), &mut err);
        api_clear_error(&mut err);
        if obj.obj_type == ObjectType::kObjectTypeNil {
            obj = dict_get_value(get_globvar_dict(), key_str, ptr::null_mut(), &mut err);
            api_clear_error(&mut err);
        }
        if obj.obj_type == ObjectType::kObjectTypeString {
            return obj.data.string.data;
        }
        api_free_object(obj);
        ptr::null_mut()
    }
}
