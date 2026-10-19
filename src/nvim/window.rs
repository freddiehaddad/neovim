//! Window and tabpage layout, management, and navigation (`src/nvim/window.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    unused_mut,
    clashing_extern_declarations,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::if_same_then_else,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]

use crate::errors::*;
use crate::globals::*;
use crate::option_vars::*;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::mem::zeroed;
use std::ptr;

use crate::api::autocmd::{OptionalKeys, TryState, event_T};
use crate::api::buffer::VisualState;
use crate::api::command::cmdmod_T;
use crate::api::deprecated::VirtText;
use crate::api::options::{CharBuf, CtxSwitch, optset_T};
use crate::api::private::validate::{
    Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::{
    GridView, ScopeDictDictItem, ScreenGrid, WinConfig, Window, frame_T, lpos_T, pos_T, tabpage_T,
    taggy_T, win_T, winopt_T,
};
use crate::api::win_config::{FloatAnchor, FloatRelative, UIExtension, WinStyle};
use crate::arglist::{alist_T, alist_unlink};
use crate::autocmd::{
    AUGROUP_ALL, EVENT_BUFENTER, EVENT_BUFLEAVE, EVENT_TABCLOSED, EVENT_TABCLOSEDPRE,
    EVENT_TABENTER, EVENT_TABLEAVE, EVENT_TABMOVED, EVENT_TABNEW, EVENT_TABNEWENTERED,
    EVENT_WINCLOSED, EVENT_WINENTER, EVENT_WINLEAVE, EVENT_WINNEW, EVENT_WINNEWPRE,
    EVENT_WINRESIZED, EVENT_WINSCROLLED, aucmd_defer, block_autocmds, event_ignored, has_event,
    unblock_autocmds,
};
use crate::buffer::{
    BLN_LISTED, DOBUF_FIRST, DOBUF_GOTO, DOBUF_UNLOAD, GETF_ALT, buf_valid, bufref_T, bufref_valid,
    set_bufref,
};
use crate::charset::getdigits_int;
use crate::clipboard::{TypvalVval, VarType, list_T, typval_T};
use crate::cmdexpand::{cmdidx_T, exarg_T};
use crate::context::{CtxSwitchFlags, CtxWin, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{check_cursor, check_cursor_lnum, colnr_T, linenr_T};
use crate::decoration::{clear_virttext, decor_conceal_line};
use crate::diff::{diff_clear, diffopt_closeoff};
use crate::drawscreen::{
    UPD_SOME_VALID, redraw_all_later, redraw_later, redrawWinline, status_redraw_all,
};
use crate::eval::decode::dict_T;
use crate::eval::deprecated::buf_T;
use crate::eval::typval::{
    tv_dict_add_dict, tv_dict_add_list, tv_dict_add_tv, tv_dict_alloc, tv_dict_set_keys_readonly,
    tv_dict_unref, tv_list_alloc, tv_list_append_owned_tv,
};
use crate::eval::vars::{init_var_dict, unref_var_dict, vars_clear};
use crate::eval::window::win_has_winnr;
use crate::eval::{get_v_event, restore_v_event};
use crate::ex_cmds::ECMD_LASTL;
use crate::ex_cmds2::dialog_changed;
use crate::ex_eval::aborting;
use crate::ex_getln::{is_in_cmdwin, text_locked, text_locked_msg, text_or_buf_locked};
use crate::file_search::grab_file_name;
use crate::fold::{clearFolding, copyFoldingState, deleteFoldRecurse, foldInitWin};
use crate::garray::{ga_grow, ga_init, garray_T};
use crate::grid::{grid_adjust, grid_assign_handle, grid_clear, grid_free, win_grid_alloc};
use crate::hashtab::{hash_init, hashtab_T};
use crate::input::{InsState, plain_vgetc, typebuf_maplen};
use crate::insert::{beginline, cursor_down_inner, cursor_up_inner};
use crate::keycodes::{K_BS, K_DOWN, K_KENTER, K_LEFT, K_RIGHT, K_UP};
use crate::kvec::KVec;
use crate::lua::converter::Boolean;
use crate::map::{Map, Set, StcClick, StcClicks, map_del_int_ptr_t, map_put_ref_int_ptr_t, ptr_t};
use crate::map_glyph_cache::NvimString;
use crate::mapping::langmap_adjust_mb;
use crate::mark::{WinInfo, copy_jumplist, free_jumplist, setmark, setpcmark};
use crate::r#match::clear_matches;
use crate::math::{FAIL, OK};
use crate::memory::{xcalloc, xfree, xmalloc, xmemdupz, xstrdup, xstrlcat, xstrlcpy};
use crate::mouse::{StlClickDefinition, reset_dragwin, setmouse};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, curs_columns, invalidate_botline_win,
    set_topline, textpos2screenpos, update_topline, validate_botline_win, validate_cursor,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::normal::{
    FIND_IDENT, add_to_showcmd, do_nv_ident, find_ident_under_cursor, reset_VIsual_and_resel,
};
use crate::os::fs::{os_chdir, os_dirname};
use crate::path::{kPathCmpLiteral, path_equal};
use crate::plines::{plines_win, plines_win_col, plines_win_nofill, win_text_height};
use crate::popupmenu::pum_pos_ui_flush;
use crate::quickfix::{copy_loclist_stack, qf_free_all, qf_view_result};
use crate::search::{Direction, find_pattern_in_path};
use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_NORMAL, MODE_TERMINAL, get_real_state, save_v_event_T,
};
use crate::statusline::stl_clear_click_defs;
use crate::syntax::reset_synblock;
use crate::tag::tagstack_clear_entry;
use crate::terminal::{Terminal, terminal_check_size};
use crate::ui_compositor::{ui_comp_layers_adjust, ui_comp_put_grid, ui_comp_remove_grid};
use crate::undo::u_sync;
use crate::winfloat::{
    win_border_height, win_border_width, win_config_float, win_float_anchor_laststatus,
    win_float_find_altwin, win_float_update_statusline, win_new_float, win_reconfig_floats,
};

pub type OptInt = i64;
pub type OptIndex = c_int;
pub type WinKind = c_int;
pub type CdCause = c_int;
pub type CdScope = c_int;
pub type scid_T = i32;

pub const WSP_ROOM: c_int = 0x01;
pub const WSP_VERT: c_int = 0x02;
pub const WSP_HOR: c_int = 0x04;
pub const WSP_TOP: c_int = 0x08;
pub const WSP_BOT: c_int = 0x10;
pub const WSP_HELP: c_int = 0x20;
pub const WSP_BELOW: c_int = 0x40;
pub const WSP_ABOVE: c_int = 0x80;
pub const WSP_NEWLOC: c_int = 0x100;
pub const WSP_NOENTER: c_int = 0x200;
pub const WSP_QUICKFIX: c_int = 0x400;

pub const MIN_COLUMNS: c_int = 12;
pub const MIN_LINES: c_int = 2;
pub const STATUS_HEIGHT: c_int = 1;
pub const LOWEST_WIN_ID: c_int = 1000;

const NUL: c_char = 0;
const TAB: c_int = b'\t' as c_int;
const CAR: c_int = b'\r' as c_int;
const NOTDONE: c_int = 2;
const FORWARD: c_int = 1;
const MAXLNUM: linenr_T = 0x7fffffff;
const MAXCOL: colnr_T = 0x7fffffff;
const MAXPATHL: usize = 4096;
const NUMBUFLEN: usize = 65;
const MAX_SCHAR_SIZE: usize = 32;

const FR_LEAF: c_char = 0;
const FR_ROW: c_char = 1;
const FR_COL: c_char = 2;

const SNAP_HELP_IDX: usize = 0;
const SNAP_AUCMD_IDX: usize = 1;
const SNAP_QUICKFIX_IDX: usize = 2;
const SNAP_COUNT: usize = 3;

const FRACTION_MULT: c_int = 16384;

const WEE_UNDO_SYNC: c_int = 0x01;
const WEE_CURWIN_INVALID: c_int = 0x02;
const WEE_TRIGGER_NEW_AUTOCMDS: c_int = 0x04;
const WEE_TRIGGER_ENTER_AUTOCMDS: c_int = 0x08;
const WEE_TRIGGER_LEAVE_AUTOCMDS: c_int = 0x10;

const Ctrl_B: c_int = 2;
const Ctrl_C: c_int = 3;
const Ctrl_D: c_int = 4;
const Ctrl_F: c_int = 6;
const Ctrl_G: c_int = 7;
const Ctrl_H: c_int = 8;
const Ctrl_I: c_int = 9;
const Ctrl_J: c_int = 10;
const Ctrl_K: c_int = 11;
const Ctrl_L: c_int = 12;
const Ctrl_N: c_int = 14;
const Ctrl_O: c_int = 15;
const Ctrl_P: c_int = 16;
const Ctrl_Q: c_int = 17;
const Ctrl_R: c_int = 18;
const Ctrl_S: c_int = 19;
const Ctrl_T: c_int = 20;
const Ctrl_V: c_int = 22;
const Ctrl_W: c_int = 23;
const Ctrl_X: c_int = 24;
const Ctrl_Z: c_int = 26;
const Ctrl_RSB: c_int = 29;
const Ctrl_HAT: c_int = 30;
const Ctrl__: c_int = 31;

const FIND_ANY: c_int = 1;
const FIND_DEFINE: c_int = 2;
const ACTION_SPLIT: c_int = 3;

const ECMD_HIDE: c_int = 0x01;
const BL_SOL: c_int = 2;
const BL_FIX: c_int = 4;
const BCO_ENTER: c_int = 1;
const BCO_NOHELP: c_int = 4;
const CMOD_CONFIRM: c_int = 0x0080;

const UPD_VALID: c_int = 10;
const UPD_NOT_VALID: c_int = 40;

const VALID_WCOL: c_int = 0x02;
const VALID_CROW: c_int = 0x10;
const VALID_TOPLINE: c_int = 0x80;

const kOptSwbFlagUseopen: c_uint = 0x01;
const kOptSwbFlagUsetab: c_uint = 0x02;
const kOptTclFlagLeft: c_uint = 0x01;
const kOptTclFlagUselast: c_uint = 0x02;

const kOptCmdheight: OptIndex = 44;
const kOptWindow: OptIndex = 362;
const kWinOptScroll: usize = 33;
const SID_WINLAYOUT: scid_T = -7;

const kUITabline: UIExtension = 2;
const kUIMessages: UIExtension = 4;
const kUIMultigrid: UIExtension = 6;

const kZIndexFloatDefault: c_int = 50;
const kZIndexMessages: c_int = 200;

const kFloatAnchorEast: FloatAnchor = 1;
const kFloatAnchorSouth: FloatAnchor = 2;

const kFloatRelativeWindow: FloatRelative = 1;
const kFloatRelativeTabline: FloatRelative = 4;
const kFloatRelativeLaststatus: FloatRelative = 5;

const kWinStyleUnused: WinStyle = 0;
const kWinStyleMinimal: WinStyle = 1;
const kWinNormal: WinKind = 0;

const kCtxKeepCwd: CtxSwitchFlags = 1;
const kCtxNoDisplay: CtxSwitchFlags = 4;
const kCtxNoEvents: CtxSwitchFlags = 8;

const kCdCauseWindow: CdCause = 1;
const kCdScopeBuffer: CdScope = 0;
const kCdScopeWindow: CdScope = 1;
const kCdScopeTabpage: CdScope = 2;
const kCdScopeGlobal: CdScope = 3;

const VAR_UNLOCKED: c_int = 0;
const VAR_SCOPE: c_int = 1;

const WIN_CONFIG_INIT: WinConfig = WinConfig {
    window: 0,
    bufpos: lpos_T { lnum: -1, col: 0 },
    height: 0,
    width: 0,
    row: 0.0,
    col: 0.0,
    anchor: 0,
    relative: 0,
    external: false,
    focusable: true,
    mouse: true,
    split: 0,
    zindex: kZIndexFloatDefault,
    style: kWinStyleUnused,
    border: false,
    shadow: false,
    border_chars: [[0; MAX_SCHAR_SIZE]; 8],
    border_hl_ids: [0; 8],
    border_attr: [0; 8],
    title: false,
    title_pos: 0,
    title_chunks: VirtText::new(),
    title_width: 0,
    footer: false,
    footer_pos: 0,
    footer_chunks: VirtText::new(),
    footer_width: 0,
    noautocmd: false,
    fixed: false,
    hide: false,
    _cmdline_offset: c_int::MAX,
};

const E_CANNOT_CLOSE_LAST_WINDOW: &CStr = c"E444: Cannot close last window";
const E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER: &CStr =
    c"E1159: Cannot split a window when closing the buffer";
const M_ONLYONE: &CStr = c"Already only one window";

static mut split_disallowed: c_int = 0;
static mut close_disallowed: c_int = 0;
static mut frame_locked: c_int = 0;
static mut min_set_ch: OptInt = 1;
static mut command_frame_height: bool = true;
static mut last_tp_handle: c_int = 0;
static mut last_win_id: c_int = LOWEST_WIN_ID - 1;
static mut old_Rows: c_int = 0;
static mut old_Columns: c_int = 0;
static mut did_initial_scroll_size_snapshot: bool = false;
static mut winclosed_recursive: bool = false;
static mut tabclosedpre_recursive: bool = false;
static mut win_scrolled_resized_recursive: bool = false;

unsafe extern "C" {

    fn gettext(str: *const c_char) -> *mut c_char;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(s: *const c_char, ...) -> bool;
    fn iemsg(s: *const c_char);
    fn msg(s: *const c_char, hl_id: c_int) -> bool;
    fn internal_error(where_: *const c_char);
    fn beep_flush();
    fn msg_clr_eos_force();
    fn msg_grid_validate();
    fn msg_ui_flush();
    fn set_keep_msg(s: *const c_char, hl_id: c_int);
    fn showmode() -> c_int;
    fn compute_cmdrow();
    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn cstr_as_string(str: *const c_char) -> NvimString;

    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn api_clear_error(value: *mut Error);
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
    fn find_window_by_handle(window: Window, err: *mut Error) -> *mut win_T;

    fn bt_quickfix(buf: *const buf_T) -> bool;
    fn bt_prompt(buf: *const buf_T) -> bool;
    fn bt_help(buf: *const buf_T) -> bool;
    fn win_copy_options(wp_from: *mut win_T, wp_to: *mut win_T) -> c_int;
    fn buf_copy_options(buf: *mut buf_T, flags: c_int);
    fn clear_winopt(wop: *mut winopt_T);
    fn set_option_value(
        opt_idx: OptIndex,
        val: Object,
        opt_flags: c_int,
        set_sid: bool,
        errbuf: *const CharBuf,
    ) -> *const c_char;
    fn option_was_set(opt_idx: OptIndex) -> bool;
    fn get_scrolloff_value(wp: *mut win_T) -> OptInt;
    fn virtual_active(wp: *mut win_T) -> bool;

    fn curbuf_locked() -> bool;
    fn check_text_or_curbuf_locked(oap: *mut c_void) -> bool;
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn tabpage_close(forceit: bool);
    fn getout(r: c_int) -> !;

    fn ui_has(ext: UIExtension) -> bool;
    fn ui_check_cursor_grid(grid: handle_T);
    fn ui_call_win_pos(
        grid: Integer,
        win: Window,
        startrow: Integer,
        startcol: Integer,
        width: Integer,
        height: Integer,
    );
    fn ui_call_win_float_pos(
        grid: Integer,
        win: Window,
        anchor: NvimString,
        anchor_grid: Integer,
        anchor_row: f64,
        anchor_col: f64,
        mouse_enabled: Boolean,
        zindex: Integer,
        compindex: Integer,
        screen_row: Integer,
        screen_col: Integer,
    );
    fn ui_call_win_external_pos(grid: Integer, win: Window);
    fn ui_call_win_hide(grid: Integer);
    fn ui_call_win_close(grid: Integer);
    fn ui_call_win_viewport(
        grid: Integer,
        win: Window,
        topline: Integer,
        botline: Integer,
        curline: Integer,
        curcol: Integer,
        line_count: Integer,
        scroll_delta: Integer,
    );
    fn ui_call_win_viewport_margins(
        grid: Integer,
        win: Window,
        top: Integer,
        bottom: Integer,
        left: Integer,
        right: Integer,
    );
    fn ui_call_grid_destroy(grid: Integer);

    fn apply_autocmds(
        event: event_T,
        fname: *const c_char,
        fname_io: *const c_char,
        force: bool,
        buf: *mut buf_T,
    ) -> bool;
    fn apply_autocmds_win(
        event: event_T,
        fname: *const c_char,
        fname_io: *const c_char,
        force: bool,
        buf: *mut buf_T,
        win: *mut win_T,
    ) -> bool;
    fn do_autocmd_dirchanged(new_dir: *mut c_char, scope: CdScope, cause: CdCause, pre: bool);
    fn do_ecmd(
        fnum: c_int,
        ffname: *const c_char,
        sfname: *const c_char,
        eap: *mut exarg_T,
        newlnum: linenr_T,
        flags: c_int,
        oldwin: *mut win_T,
    ) -> c_int;
    fn comp_col();
    fn buf_hide(buf: *const buf_T) -> bool;
    fn bufIsChanged(buf: *const buf_T) -> bool;
    fn buflist_findname_exp(fname: *mut c_char) -> *mut buf_T;
    fn buflist_findnr(nr: c_int) -> *mut buf_T;
    fn buflist_getfile(n: c_int, lnum: linenr_T, options: c_int, forceit: c_int) -> c_int;
    fn buflist_new(
        ffname_arg: *mut c_char,
        sfname_arg: *mut c_char,
        lnum: linenr_T,
        flags: c_int,
    ) -> *mut buf_T;
    fn can_abandon(buf: *mut buf_T, forceit: c_int) -> bool;
    fn close_buffer(
        win: *mut win_T,
        buf: *mut buf_T,
        action: c_int,
        abort_if_last: bool,
        ignore_abort: bool,
        set_Plines: bool,
    ) -> bool;
    fn do_autochdir();
    fn do_buffer(action: c_int, start: c_int, dir: c_int, count: c_int, forceit: c_int) -> c_int;
    fn maketitle();
    fn shorten_fnames(force: bool);
    fn tv_dict_extend(d1: *mut dict_T, d2: *mut dict_T, action: *const c_char);
    fn getDeepestNesting(wp: *mut win_T) -> c_int;
    fn hasFolding(
        win: *mut win_T,
        lnum: linenr_T,
        firstp: *mut linenr_T,
        lastp: *mut linenr_T,
    ) -> bool;
    fn win_col_off(wp: *mut win_T) -> c_int;
    fn win_col_off2(wp: *mut win_T) -> c_int;
}

const ERROR_INIT: Error = Error {
    err_type: ErrorType::kErrorTypeNone,
    msg: ptr::null_mut(),
};

#[inline]
const fn error_set(err: &Error) -> bool {
    !matches!(err.err_type, ErrorType::kErrorTypeNone)
}

#[inline]
const fn ascii_isdigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'9' as c_int
}

#[inline]
fn nowin() -> *mut win_T {
    (-1isize) as *mut win_T
}

#[inline]
unsafe fn rows_avail() -> OptInt {
    unsafe {
        OptInt::from(Rows)
            - p_ch
            - OptInt::from(tabline_height())
            - OptInt::from(global_stl_height())
    }
}

#[inline]
unsafe fn one_window_macro() -> bool {
    unsafe { ptr::eq(firstwin, lastwin) }
}

#[inline]
unsafe fn reset_binding(wp: *mut win_T) {
    unsafe {
        (*wp).w_onebuf_opt.wo_scb = 0;
        (*wp).w_onebuf_opt.wo_crb = 0;
    }
}

#[inline]
const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
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
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
unsafe fn langmap_adjust(c: &mut c_int, condition: bool) {
    unsafe {
        if *p_langmap != NUL
            && condition
            && (p_lrm != 0
                || (if vgetc_busy != 0 {
                    typebuf_maplen() == 0
                } else {
                    KeyTyped
                }))
            && KeyStuffed == 0
            && *c >= 0
        {
            *c = langmap_adjust_mb(*c);
        }
    }
}

/// Disallow changing the window layout (split window, close window, move
/// window). Resizing is still allowed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn window_layout_lock() {
    unsafe {
        split_disallowed += 1;
        close_disallowed += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn window_layout_unlock() {
    unsafe {
        split_disallowed -= 1;
        close_disallowed -= 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn frames_locked() -> bool {
    unsafe { frame_locked != 0 }
}

/// When the window layout cannot be changed give an error and return true.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn window_layout_locked(cmd: cmdidx_T) -> bool {
    unsafe {
        let mut err = ERROR_INIT;
        let locked = window_layout_locked_err(cmd, &raw mut err);
        if error_set(&err) {
            emsg(gettext(err.msg));
            api_clear_error(&raw mut err);
        }
        locked
    }
}

/// Like `window_layout_locked`, but set `err` to the (untranslated) error message when locked.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn window_layout_locked_err(cmd: cmdidx_T, err: *mut Error) -> bool {
    unsafe {
        if split_disallowed > 0 || close_disallowed > 0 {
            if close_disallowed == 0 && cmd == cmdidx_T::CMD_tabnew {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER.as_ptr(),
                );
            } else {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    e_not_allowed_to_change_window_layout_in_this_autocmd.as_ptr(),
                );
            }
            return true;
        }
        false
    }
}

/// Check if the current window is allowed to move to a different buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_can_set_curbuf_disabled() -> bool {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_wfb != 0 {
            emsg(gettext(e_winfixbuf_cannot_go_to_buffer.as_ptr()));
            return false;
        }
        true
    }
}

/// Check if the current window is allowed to move to a different buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_can_set_curbuf_forceit(forceit: c_int) -> bool {
    unsafe {
        if forceit == 0 && (*curwin).w_onebuf_opt.wo_wfb != 0 {
            emsg(gettext(e_winfixbuf_cannot_go_to_buffer.as_ptr()));
            return false;
        }
        true
    }
}

/// Return the current window, unless in the cmdline window and "prevwin" is
/// set, then return "prevwin".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prevwin_curwin() -> *mut win_T {
    unsafe {
        if is_in_cmdwin() && !prevwin.is_null() {
            prevwin
        } else {
            curwin
        }
    }
}

/// If the 'switchbuf' option contains "useopen" or "usetab", then try to jump
/// to a window containing "buf".
/// Returns the pointer to the window that was jumped to or NULL.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn swbuf_goto_win_with_buf(buf: *mut buf_T) -> *mut win_T {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();

        if buf.is_null() {
            return wp;
        }

        if (swb_flags & kOptSwbFlagUseopen) != 0 {
            wp = buf_jump_open_win(buf);
        }

        if wp.is_null() && (swb_flags & kOptSwbFlagUsetab) != 0 {
            wp = buf_jump_open_tab(buf);
        }

        wp
    }
}

unsafe fn do_window_newwindow(nchar: c_int, Prenum: c_int, cbuf: &mut [c_char; 40]) {
    unsafe {
        if Prenum != 0 {
            vim_snprintf(
                cbuf.as_mut_ptr(),
                cbuf.len() - 5,
                c"%lld".as_ptr(),
                i64::from(Prenum),
            );
        } else {
            cbuf[0] = NUL;
        }
        if nchar == b'v' as c_int || nchar == Ctrl_V {
            xstrlcat(cbuf.as_mut_ptr(), c"v".as_ptr(), cbuf.len());
        }
        xstrlcat(cbuf.as_mut_ptr(), c"new".as_ptr(), cbuf.len());
        do_cmdline_cmd(cbuf.as_ptr());
    }
}

unsafe fn do_window_gotofile(nchar: c_int, Prenum1: c_int) {
    unsafe {
        if check_text_or_curbuf_locked(ptr::null_mut()) {
            return;
        }

        let mut lnum: linenr_T = -1;
        let ptr = grab_file_name(Prenum1, &raw mut lnum);
        if !ptr.is_null() {
            let oldtab = curtab;
            let oldwin = curwin;
            setpcmark();

            let mut wp: *mut win_T = ptr::null_mut();
            if (swb_flags & (kOptSwbFlagUseopen | kOptSwbFlagUsetab)) != 0 && cmdmod.cmod_tab == 0 {
                wp = swbuf_goto_win_with_buf(buflist_findname_exp(ptr));
            }

            if wp.is_null() && win_split(0, 0) == OK {
                reset_binding(curwin);
                if do_ecmd(
                    0,
                    ptr,
                    ptr::null(),
                    ptr::null_mut(),
                    ECMD_LASTL,
                    ECMD_HIDE,
                    ptr::null_mut(),
                ) == FAIL
                {
                    let old_got_int = got_int;
                    got_int = false;

                    win_close(curwin, false, false);
                    got_int = got_int || old_got_int;

                    goto_tabpage_win(oldtab, oldwin);
                } else {
                    wp = curwin;
                }
            }

            if !wp.is_null() && nchar == b'F' as c_int && lnum >= 0 {
                (*curwin).w_cursor.lnum = lnum;
                check_cursor_lnum(curwin);
                beginline(BL_SOL | BL_FIX);
            }
            xfree(ptr.cast());
        }
    }
}

/// All CTRL-W window commands are handled here, called from normal_cmd().
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_window(mut nchar: c_int, mut Prenum: c_int, mut xchar: c_int) {
    unsafe {
        let mut type_ = FIND_DEFINE;
        let mut cbuf = [0 as c_char; 40];
        let Prenum1 = if Prenum == 0 { 1 } else { Prenum };

        match nchar {
            c if c == b'S' as c_int || c == Ctrl_S || c == b's' as c_int => {
                reset_VIsual_and_resel();
                if bt_quickfix(curbuf) {
                    do_window_newwindow(nchar, Prenum, &mut cbuf);
                } else {
                    win_split(Prenum, 0);
                }
            }
            c if c == Ctrl_V || c == b'v' as c_int => {
                reset_VIsual_and_resel();
                if bt_quickfix(curbuf) {
                    do_window_newwindow(nchar, Prenum, &mut cbuf);
                } else {
                    win_split(Prenum, WSP_VERT);
                }
            }
            c if c == Ctrl_HAT || c == b'^' as c_int => {
                reset_VIsual_and_resel();

                if buflist_findnr(if Prenum == 0 {
                    (*curwin).w_alt_fnum
                } else {
                    Prenum
                })
                .is_null()
                {
                    if Prenum == 0 {
                        emsg(gettext(e_noalt.as_ptr()));
                    } else {
                        semsg(gettext(e_buffer_nr_not_found.as_ptr()), i64::from(Prenum));
                    }
                    return;
                }

                if !curbuf_locked() && win_split(0, 0) == OK {
                    buflist_getfile(
                        if Prenum == 0 {
                            (*curwin).w_alt_fnum
                        } else {
                            Prenum
                        },
                        0,
                        GETF_ALT,
                        0,
                    );
                }
            }
            c if c == Ctrl_N || c == b'n' as c_int => {
                reset_VIsual_and_resel();
                do_window_newwindow(nchar, Prenum, &mut cbuf);
            }
            c if c == Ctrl_Q || c == b'q' as c_int => {
                reset_VIsual_and_resel();
                cmd_with_count(
                    c"quit".as_ptr(),
                    cbuf.as_mut_ptr(),
                    cbuf.len(),
                    i64::from(Prenum),
                );
                do_cmdline_cmd(cbuf.as_ptr());
            }
            c if c == Ctrl_C || c == b'c' as c_int => {
                reset_VIsual_and_resel();
                cmd_with_count(
                    c"close".as_ptr(),
                    cbuf.as_mut_ptr(),
                    cbuf.len(),
                    i64::from(Prenum),
                );
                do_cmdline_cmd(cbuf.as_ptr());
            }
            c if c == Ctrl_Z || c == b'z' as c_int => {
                reset_VIsual_and_resel();
                do_cmdline_cmd(c"pclose".as_ptr());
            }
            c if c == b'P' as c_int => {
                let mut wp: *mut win_T = ptr::null_mut();
                let mut wp2 = firstwin;
                while !wp2.is_null() {
                    if (*wp2).w_onebuf_opt.wo_pvw != 0 {
                        wp = wp2;
                        break;
                    }
                    wp2 = (*wp2).w_next;
                }
                if wp.is_null() {
                    emsg(gettext(c"E441: There is no preview window".as_ptr()));
                } else {
                    win_goto(wp);
                }
            }
            c if c == Ctrl_O || c == b'o' as c_int => {
                reset_VIsual_and_resel();
                cmd_with_count(
                    c"only".as_ptr(),
                    cbuf.as_mut_ptr(),
                    cbuf.len(),
                    i64::from(Prenum),
                );
                do_cmdline_cmd(cbuf.as_ptr());
            }
            c if c == Ctrl_W || c == b'w' as c_int || c == b'W' as c_int => {
                if one_window_macro() && Prenum != 1 {
                    beep_flush();
                } else {
                    let mut wp: *mut win_T;
                    if Prenum != 0 {
                        let mut last_focusable = firstwin;
                        wp = firstwin;
                        while {
                            Prenum -= 1;
                            Prenum > 0
                        } {
                            if !(*wp).w_floating
                                || (!(*wp).w_config.hide && (*wp).w_config.focusable)
                            {
                                last_focusable = wp;
                            }
                            if (*wp).w_next.is_null() {
                                break;
                            }
                            wp = (*wp).w_next;
                        }
                        while !wp.is_null()
                            && (*wp).w_floating
                            && ((*wp).w_config.hide || !(*wp).w_config.focusable)
                        {
                            wp = (*wp).w_next;
                        }
                        if wp.is_null() {
                            wp = last_focusable;
                        }
                    } else if nchar == b'W' as c_int {
                        wp = (*curwin).w_prev;
                        if wp.is_null() {
                            wp = lastwin;
                        }
                        while !wp.is_null()
                            && (*wp).w_floating
                            && ((*wp).w_config.hide || !(*wp).w_config.focusable)
                        {
                            wp = (*wp).w_prev;
                        }
                    } else {
                        wp = (*curwin).w_next;
                        while !wp.is_null()
                            && (*wp).w_floating
                            && ((*wp).w_config.hide || !(*wp).w_config.focusable)
                        {
                            wp = (*wp).w_next;
                        }
                        if wp.is_null() {
                            wp = firstwin;
                        }
                    }
                    win_goto(wp);
                }
            }
            c if c == b'j' as c_int || c == K_DOWN || c == Ctrl_J => {
                win_goto_ver(false, Prenum1);
            }
            c if c == b'k' as c_int || c == K_UP || c == Ctrl_K => {
                win_goto_ver(true, Prenum1);
            }
            c if c == b'h' as c_int || c == K_LEFT || c == Ctrl_H || c == K_BS => {
                win_goto_hor(true, Prenum1);
            }
            c if c == b'l' as c_int || c == K_RIGHT || c == Ctrl_L => {
                win_goto_hor(false, Prenum1);
            }
            c if c == b'T' as c_int => {
                if one_window(curwin, ptr::null_mut()) {
                    msg(gettext(M_ONLYONE.as_ptr()), 0);
                } else {
                    let oldtab = curtab;
                    let wp = curwin;
                    if !win_new_tabpage(Prenum, ptr::null_mut(), true, ptr::null_mut()).is_null()
                        && valid_tabpage(oldtab)
                    {
                        let newtab = curtab;
                        goto_tabpage_tp(oldtab, true, true);
                        if ptr::eq(curwin, wp) {
                            win_close(curwin, false, false);
                        }
                        if valid_tabpage(newtab) {
                            goto_tabpage_tp(newtab, true, true);
                            apply_autocmds(
                                EVENT_TABNEWENTERED,
                                ptr::null(),
                                ptr::null(),
                                false,
                                curbuf,
                            );
                        }
                    }
                }
            }
            c if c == b't' as c_int || c == Ctrl_T => {
                win_goto(firstwin);
            }
            c if c == b'b' as c_int || c == Ctrl_B => {
                win_goto(lastwin_nofloating(ptr::null_mut()));
            }
            c if c == b'p' as c_int || c == Ctrl_P => {
                if !win_valid(prevwin) || (*prevwin).w_config.hide || !(*prevwin).w_config.focusable
                {
                    beep_flush();
                } else {
                    win_goto(prevwin);
                }
            }
            c if c == b'x' as c_int || c == Ctrl_X => {
                win_exchange(Prenum);
            }
            c if c == Ctrl_R || c == b'r' as c_int => {
                reset_VIsual_and_resel();
                win_rotate(false, Prenum1);
            }
            c if c == b'R' as c_int => {
                reset_VIsual_and_resel();
                win_rotate(true, Prenum1);
            }
            c if c == b'K' as c_int
                || c == b'J' as c_int
                || c == b'H' as c_int
                || c == b'L' as c_int =>
            {
                if one_window(curwin, ptr::null_mut()) {
                    beep_flush();
                } else {
                    let dir = (if nchar == b'H' as c_int || nchar == b'L' as c_int {
                        WSP_VERT
                    } else {
                        0
                    }) | (if nchar == b'H' as c_int || nchar == b'K' as c_int {
                        WSP_TOP
                    } else {
                        WSP_BOT
                    });
                    win_splitmove(curwin, Prenum, dir);
                }
            }
            c if c == b'=' as c_int => {
                let mod_ = cmdmod.cmod_split & (WSP_VERT | WSP_HOR);
                win_equal(
                    ptr::null_mut(),
                    false,
                    if mod_ == WSP_VERT {
                        b'v' as c_int
                    } else if mod_ == WSP_HOR {
                        b'h' as c_int
                    } else {
                        b'b' as c_int
                    },
                );
            }
            c if c == b'+' as c_int => {
                win_setheight((*curwin).w_height + Prenum1);
            }
            c if c == b'-' as c_int => {
                win_setheight((*curwin).w_height - Prenum1);
            }
            c if c == Ctrl__ || c == b'_' as c_int => {
                win_setheight(if Prenum != 0 {
                    Prenum
                } else {
                    Rows - min_set_ch as c_int
                });
            }
            c if c == b'>' as c_int => {
                win_setwidth((*curwin).w_width + Prenum1);
            }
            c if c == b'<' as c_int => {
                win_setwidth((*curwin).w_width - Prenum1);
            }
            c if c == b'|' as c_int => {
                win_setwidth(if Prenum != 0 { Prenum } else { Columns });
            }
            c if c == b'}' as c_int || c == b']' as c_int || c == Ctrl_RSB => {
                if nchar == b'}' as c_int {
                    if Prenum != 0 {
                        g_do_tagpreview = Prenum;
                    } else {
                        g_do_tagpreview = p_pvh as c_int;
                    }
                } else {
                    g_do_tagpreview = 0;
                }

                if Prenum != 0 {
                    postponed_split = Prenum;
                } else {
                    postponed_split = -1;
                }

                do_nv_ident(Ctrl_RSB, NUL as c_int);
                postponed_split = 0;
            }
            c if c == b'f' as c_int || c == b'F' as c_int || c == Ctrl_F => {
                do_window_gotofile(nchar, Prenum1);
            }
            c if c == b'i' as c_int || c == Ctrl_I || c == b'd' as c_int || c == Ctrl_D => {
                if nchar == b'i' as c_int || nchar == Ctrl_I {
                    type_ = FIND_ANY;
                }
                let mut ptr: *mut c_char = ptr::null_mut();
                let len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, ptr::null_mut());
                if len == 0 {
                    return;
                }
                ptr = xmemdupz(ptr.cast(), len).cast();
                find_pattern_in_path(
                    ptr,
                    0,
                    len,
                    true,
                    Prenum == 0,
                    type_,
                    Prenum1,
                    ACTION_SPLIT,
                    1,
                    MAXLNUM,
                    false,
                    false,
                );
                xfree(ptr.cast());
                (*curwin).w_set_curswant = true;
            }
            c if c == K_KENTER || c == CAR => {
                if bt_quickfix(curbuf) {
                    qf_view_result(true);
                }
            }
            c if c == b'g' as c_int || c == Ctrl_G => {
                no_mapping += 1;
                allow_keys += 1;
                if xchar == NUL as c_int {
                    xchar = plain_vgetc();
                }
                langmap_adjust(&mut xchar, true);
                no_mapping -= 1;
                allow_keys -= 1;
                add_to_showcmd(xchar);

                match xchar {
                    xc if xc == b'}' as c_int || xc == b']' as c_int || xc == Ctrl_RSB => {
                        if xchar == b'}' as c_int {
                            xchar = Ctrl_RSB;
                            if Prenum != 0 {
                                g_do_tagpreview = Prenum;
                            } else {
                                g_do_tagpreview = p_pvh as c_int;
                            }
                        }
                        if Prenum != 0 {
                            postponed_split = Prenum;
                        } else {
                            postponed_split = -1;
                        }
                        do_nv_ident(b'g' as c_int, xchar);
                        postponed_split = 0;
                    }
                    xc if xc == b'f' as c_int || xc == b'F' as c_int => {
                        cmdmod.cmod_tab = tabpage_index(curtab) + 1;
                        nchar = xchar;
                        do_window_gotofile(nchar, Prenum1);
                    }
                    xc if xc == b't' as c_int => {
                        goto_tabpage(Prenum);
                    }
                    xc if xc == b'T' as c_int => {
                        goto_tabpage(-Prenum1);
                    }
                    xc if xc == TAB => {
                        if !goto_tabpage_lastused() {
                            beep_flush();
                        }
                    }
                    xc if xc == b'e' as c_int => {
                        if (*curwin).w_floating || !ui_has(kUIMultigrid) {
                            beep_flush();
                            return;
                        }
                        let mut config = WIN_CONFIG_INIT;
                        config.width = (*curwin).w_width;
                        config.height = (*curwin).w_height;
                        config.external = true;
                        let mut err = ERROR_INIT;
                        if win_new_float(curwin, false, config, &raw mut err).is_null() {
                            emsg(err.msg);
                            api_clear_error(&raw mut err);
                            beep_flush();
                        }
                    }
                    _ => {
                        beep_flush();
                    }
                }
            }
            _ => {
                beep_flush();
            }
        }
    }
}

unsafe fn cmd_with_count(cmd: *const c_char, bufp: *mut c_char, bufsize: usize, Prenum: i64) {
    unsafe {
        let len = xstrlcpy(bufp, cmd, bufsize);
        if Prenum > 0 && len < bufsize {
            vim_snprintf(bufp.add(len), bufsize - len, c"%lld".as_ptr(), Prenum);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_set_buf(win: *mut win_T, buf: *mut buf_T, err: *mut Error) {
    unsafe {
        let win_handle = (*win).handle;
        let tab = win_find_tabpage(win);

        RedrawingDisabled += 1;

        let mut switchwin: CtxSwitch = zeroed();
        let mut win_ok: bool;

        let mut tstate: TryState = zeroed();
        try_enter(&raw mut tstate);
        win_ok = ctx_switch(
            &raw mut switchwin,
            win,
            tab,
            ptr::null_mut(),
            kCtxNoDisplay | kCtxKeepCwd,
        );
        if win_ok {
            let save_acd = p_acd;
            if !switchwin.cs_same_win {
                p_acd = 0;
            }

            do_buffer(DOBUF_GOTO, DOBUF_FIRST, FORWARD, (*buf).b_fnum, 0);

            if !switchwin.cs_same_win {
                p_acd = save_acd;
            }
        }
        try_leave(&raw const tstate, err);

        if !win_ok && !error_set(&*err) {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to switch to window %d".as_ptr(),
                win_handle,
            );
        }

        validate_cursor(curwin);

        ctx_restore(&raw mut switchwin);
        RedrawingDisabled -= 1;
    }
}

/// Return the number of fold columns to display
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_fdccol_count(wp: *mut win_T) -> c_int {
    unsafe {
        let fdc = (*wp).w_onebuf_opt.wo_fdc;
        if *fdc == b'a' as c_char
            && *fdc.add(1) == b'u' as c_char
            && *fdc.add(2) == b't' as c_char
            && *fdc.add(3) == b'o' as c_char
        {
            let fdccol = if *fdc.add(4) == b':' as c_char {
                c_int::from(*fdc.add(5) as u8) - c_int::from(b'0')
            } else {
                1
            };
            let needed_fdccols = getDeepestNesting(wp);
            return fdccol.min(needed_fdccols);
        }
        c_int::from(*fdc as u8) - c_int::from(b'0')
    }
}

/// Merges two window configs, freeing replaced fields if necessary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn merge_win_config(dst: *mut WinConfig, src: WinConfig) {
    unsafe {
        if !ptr::eq((*dst).title_chunks.items, src.title_chunks.items) {
            clear_virttext(&raw mut (*dst).title_chunks);
        }
        if !ptr::eq((*dst).footer_chunks.items, src.footer_chunks.items) {
            clear_virttext(&raw mut (*dst).footer_chunks);
        }
        *dst = src;
    }
}

/// Clear fields in `fconfig` that are only used for floating windows.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_float_config(fconfig: *mut WinConfig, free_fields: bool) {
    unsafe {
        let saved_style = (*fconfig).style;
        let saved_cmdline_offset = (*fconfig)._cmdline_offset;
        if free_fields {
            merge_win_config(fconfig, WIN_CONFIG_INIT);
        } else {
            *fconfig = WIN_CONFIG_INIT;
        }
        (*fconfig).style = saved_style;
        (*fconfig)._cmdline_offset = saved_cmdline_offset;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_ext_win_position(wp: *mut win_T, validate: bool) {
    unsafe {
        (*wp).w_pos_changed = false;
        if !(*wp).w_floating {
            if ui_has(kUIMultigrid) {
                (*wp).w_grid_alloc.comp_col = (*wp).w_wincol;
                (*wp).w_grid_alloc.comp_row = (*wp).w_winrow;
            }
            ui_call_win_pos(
                Integer::from((*wp).w_grid_alloc.handle),
                (*wp).handle,
                Integer::from((*wp).w_winrow),
                Integer::from((*wp).w_wincol),
                Integer::from((*wp).w_width),
                Integer::from((*wp).w_height),
            );
            return;
        }

        let c = (*wp).w_config;
        if !c.external {
            let mut grid: *mut ScreenGrid = &raw mut default_grid;
            let mut row = c.row;
            let mut col = c.col;
            if c.relative == kFloatRelativeWindow {
                let mut dummy = ERROR_INIT;
                let win = find_window_by_handle(c.window, &raw mut dummy);
                api_clear_error(&raw mut dummy);
                if !win.is_null() {
                    if (*win).w_pos_changed
                        && !(*win).w_grid_alloc.chars.is_null()
                        && win_valid(win)
                    {
                        ui_ext_win_position(win, validate);
                    }
                    let mut row_off: c_int = 0;
                    let mut col_off: c_int = 0;
                    win_grid_alloc(win);
                    grid = grid_adjust(&raw mut (*win).w_grid, &raw mut row_off, &raw mut col_off);
                    row += f64::from(row_off);
                    col += f64::from(col_off);
                    if c.bufpos.lnum >= 0 {
                        let lnum = (c.bufpos.lnum + 1).min((*(*win).w_buffer).b_ml.ml_line_count);
                        let mut pos = pos_T {
                            lnum,
                            col: c.bufpos.col,
                            coladd: 0,
                        };
                        let mut trow: c_int = 0;
                        let mut tcol: c_int = 0;
                        let mut tcolc: c_int = 0;
                        let mut tcole: c_int = 0;
                        textpos2screenpos(
                            win,
                            &raw mut pos,
                            &raw mut trow,
                            &raw mut tcol,
                            &raw mut tcolc,
                            &raw mut tcole,
                            true,
                        );
                        row += f64::from(trow - 1);
                        col += f64::from(tcol - 1);
                    }
                }
            } else if c.relative == kFloatRelativeLaststatus {
                row += f64::from(Rows - p_ch as c_int - last_stl_height(false));
            } else if c.relative == kFloatRelativeTabline {
                row += f64::from(tabline_height());
            }

            let resort = (*wp).w_grid_alloc.comp_index != 0
                && (*wp).w_grid_alloc.zindex != (*wp).w_config.zindex;
            let raise = resort && (*wp).w_grid_alloc.zindex < (*wp).w_config.zindex;
            (*wp).w_grid_alloc.zindex = (*wp).w_config.zindex;
            if resort {
                ui_comp_layers_adjust((*wp).w_grid_alloc.comp_index, raise);
            }
            let valid = (*wp).w_redr_type == 0 || ui_has(kUIMultigrid);
            if !valid && !validate {
                (*wp).w_pos_changed = true;
                return;
            }

            let east = (c.anchor & kFloatAnchorEast) != 0;
            let south = (c.anchor & kFloatAnchorSouth) != 0;

            let mut comp_row = (row as c_int) - if south { (*wp).w_height_outer } else { 0 };
            let mut comp_col = (col as c_int) - if east { (*wp).w_width_outer } else { 0 };
            let above_ch = if (*wp).w_config.zindex < kZIndexMessages {
                p_ch as c_int
            } else {
                0
            };
            comp_row += (*grid).comp_row;
            comp_col += (*grid).comp_col;
            comp_row = comp_row.min(Rows - (*wp).w_height_outer - above_ch).max(0);
            if !c.fixed || east {
                comp_col = comp_col.min(Columns - (*wp).w_width_outer).max(0);
            }
            (*wp).w_winrow = comp_row;
            (*wp).w_wincol = comp_col;

            if !c.hide {
                ui_comp_put_grid(
                    &raw mut (*wp).w_grid_alloc,
                    comp_row,
                    comp_col,
                    (*wp).w_height_outer,
                    (*wp).w_width_outer,
                    valid,
                    false,
                );
                if ui_has(kUIMultigrid) {
                    let anchor_ptr = *(&raw const float_anchor_str)
                        .cast::<*const c_char>()
                        .add(c.anchor as usize);
                    let anchor = cstr_as_string(anchor_ptr);
                    ui_call_win_float_pos(
                        Integer::from((*wp).w_grid_alloc.handle),
                        (*wp).handle,
                        anchor,
                        Integer::from((*grid).handle),
                        row,
                        col,
                        c.mouse,
                        Integer::from((*wp).w_grid_alloc.zindex),
                        (*wp).w_grid_alloc.comp_index as Integer,
                        Integer::from((*wp).w_winrow),
                        Integer::from((*wp).w_wincol),
                    );
                }
                ui_check_cursor_grid((*wp).w_grid_alloc.handle);
                (*wp).w_grid_alloc.mouse_enabled = (*wp).w_config.mouse;
                if !valid {
                    (*wp).w_grid_alloc.valid = false;
                    redraw_later(wp, UPD_NOT_VALID);
                }
            } else {
                if ui_has(kUIMultigrid) {
                    ui_call_win_hide(Integer::from((*wp).w_grid_alloc.handle));
                }
                ui_comp_remove_grid(&raw mut (*wp).w_grid_alloc);
            }
        } else {
            ui_call_win_external_pos(Integer::from((*wp).w_grid_alloc.handle), (*wp).handle);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_ext_win_viewport(wp: *mut win_T) {
    unsafe {
        if (ptr::eq(wp, curwin) || ui_has(kUIMultigrid))
            && (*wp).w_viewport_invalid
            && (*wp).w_redr_type == 0
        {
            let line_count = (*(*wp).w_buffer).b_ml.ml_line_count;
            let cur_topline = (*wp).w_topline.min(line_count);
            let cur_botline = (*wp).w_botline.min(line_count);
            let mut delta: i64 = 0;
            let mut last_topline = (*wp).w_viewport_last_topline;
            let mut last_botline = (*wp).w_viewport_last_botline;
            let mut last_topfill = (*wp).w_viewport_last_topfill;
            let mut last_skipcol = i64::from((*wp).w_viewport_last_skipcol);
            if last_topline > line_count {
                delta -= i64::from(last_topline - line_count);
                last_topline = line_count;
                last_topfill = 0;
                last_skipcol = i64::from(MAXCOL);
            }
            last_botline = last_botline.min(line_count);
            if cur_topline < last_topline
                || (cur_topline == last_topline && i64::from((*wp).w_skipcol) < last_skipcol)
            {
                let mut vcole = last_skipcol;
                let mut lnume = last_topline;
                if last_topline > 0 && cur_botline < last_topline {
                    delta -= i64::from(last_topline - cur_botline);
                    lnume = cur_botline;
                    vcole = 0;
                }
                delta -= win_text_height(
                    wp,
                    cur_topline,
                    i64::from((*wp).w_skipcol),
                    &raw mut lnume,
                    &raw mut vcole,
                    ptr::null_mut(),
                    i64::MAX,
                );
            } else if cur_topline > last_topline
                || (cur_topline == last_topline && i64::from((*wp).w_skipcol) > last_skipcol)
            {
                let mut vcole = i64::from((*wp).w_skipcol);
                let mut lnume = cur_topline;
                if last_botline > 0 && cur_topline > last_botline {
                    delta += i64::from(cur_topline - last_botline);
                    lnume = last_botline;
                    vcole = 0;
                }
                delta += win_text_height(
                    wp,
                    last_topline,
                    last_skipcol,
                    &raw mut lnume,
                    &raw mut vcole,
                    ptr::null_mut(),
                    i64::MAX,
                );
            }
            delta += i64::from(last_topfill);
            delta -= i64::from((*wp).w_topfill);
            let mut ev_botline = (*wp).w_botline;
            if ev_botline == line_count + 1 && (*wp).w_empty_rows == 0 {
                ev_botline = line_count;
            }
            ui_call_win_viewport(
                Integer::from((*wp).w_grid_alloc.handle),
                (*wp).handle,
                Integer::from((*wp).w_topline - 1),
                Integer::from(ev_botline),
                Integer::from((*wp).w_cursor.lnum - 1),
                Integer::from((*wp).w_cursor.col),
                Integer::from(line_count),
                delta,
            );
            (*wp).w_viewport_invalid = false;
            (*wp).w_viewport_last_topline = (*wp).w_topline;
            (*wp).w_viewport_last_botline = (*wp).w_botline;
            (*wp).w_viewport_last_topfill = (*wp).w_topfill;
            (*wp).w_viewport_last_skipcol = (*wp).w_skipcol;
        }
    }
}

/// If "split_disallowed" is set, or "wp"'s buffer is closing, give an error and return FAIL.
/// Otherwise return OK.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_split_disallowed(wp: *const win_T) -> c_int {
    unsafe {
        let mut err = ERROR_INIT;
        let ok = check_split_disallowed_err(wp, &raw mut err);
        if error_set(&err) {
            emsg(gettext(err.msg));
            api_clear_error(&raw mut err);
        }
        if ok { OK } else { FAIL }
    }
}

/// Like `check_split_disallowed`, but set `err` to the (untranslated) error message on failure and
/// return false. Otherwise return true.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_split_disallowed_err(wp: *const win_T, err: *mut Error) -> bool {
    unsafe {
        if split_disallowed > 0 {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"E242: Can't split a window while closing another".as_ptr(),
            );
            return false;
        }
        if (*(*wp).w_buffer).b_locked_split != 0 {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"%s".as_ptr(),
                E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER.as_ptr(),
            );
            return false;
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_split(size: c_int, mut flags: c_int) -> c_int {
    unsafe {
        if check_split_disallowed(curwin) == FAIL {
            return FAIL;
        }

        if may_open_tabpage() == OK {
            return OK;
        }

        flags |= cmdmod.cmod_split;
        if (flags & WSP_TOP) != 0 && (flags & WSP_BOT) != 0 {
            emsg(gettext(
                c"E442: Can't split topleft and botright at the same time".as_ptr(),
            ));
            return FAIL;
        }

        if (flags & WSP_HELP) != 0 {
            make_snapshot(SNAP_HELP_IDX as c_int);
        } else {
            clear_snapshot(curtab, SNAP_HELP_IDX as c_int);
        }

        if (flags & WSP_QUICKFIX) != 0 {
            make_snapshot(SNAP_QUICKFIX_IDX as c_int);
        } else {
            clear_snapshot(curtab, SNAP_QUICKFIX_IDX as c_int);
        }

        if win_split_ins(size, flags, ptr::null_mut(), 0, ptr::null_mut()).is_null() {
            FAIL
        } else {
            OK
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_split_ins(
    size: c_int,
    flags: c_int,
    new_wp: *mut win_T,
    dir: c_int,
    to_flatten: *mut frame_T,
) -> *mut win_T {
    unsafe {
        let mut wp = new_wp;

        if !new_wp.is_null() && is_ctx_win(new_wp) {
            return ptr::null_mut();
        }

        if new_wp.is_null() {
            trigger_winnewpre();
        }

        let oldwin = if (flags & WSP_TOP) != 0 {
            firstwin
        } else if (flags & WSP_BOT) != 0 || (*curwin).w_floating {
            lastwin_nofloating(ptr::null_mut())
        } else {
            curwin
        };

        let mut need_status: c_int = 0;
        let mut new_size = size;
        let vertical = (flags & WSP_VERT) != 0;
        let toplevel = (flags & (WSP_TOP | WSP_BOT)) != 0;

        if one_window(firstwin, ptr::null_mut()) && p_ls == 1 && (*oldwin).w_status_height == 0 {
            if OptInt::from((*oldwin).w_height) <= p_wmh {
                emsg(gettext(e_noroom.as_ptr()));
                return ptr::null_mut();
            }
            need_status = STATUS_HEIGHT;
            win_float_anchor_laststatus();
        }

        let mut do_equal = false;
        let mut oldwin_height: c_int = 0;
        let layout = if vertical { FR_ROW } else { FR_COL };
        let mut did_set_fraction = false;

        if vertical {
            let wmw1 = if p_wmw == 0 { 1 } else { p_wmw as c_int };
            let mut needed = wmw1 + 1;
            if (flags & WSP_ROOM) != 0 {
                needed += p_wiw as c_int - wmw1;
            }
            let mut minwidth: c_int;
            let available: c_int;
            if toplevel {
                minwidth = frame_minwidth(topframe, nowin());
                available = (*topframe).fr_width;
                needed += minwidth;
            } else if p_ea != 0 {
                minwidth = frame_minwidth((*oldwin).w_frame, nowin());
                let mut prevfrp = (*oldwin).w_frame;
                let mut frp = (*(*oldwin).w_frame).fr_parent;
                while !frp.is_null() {
                    if (*frp).fr_layout == FR_ROW {
                        let mut frp2 = (*frp).fr_child;
                        while !frp2.is_null() {
                            if !ptr::eq(frp2, prevfrp) {
                                minwidth += frame_minwidth(frp2, nowin());
                            }
                            frp2 = (*frp2).fr_next;
                        }
                    }
                    prevfrp = frp;
                    frp = (*frp).fr_parent;
                }
                available = (*topframe).fr_width;
                needed += minwidth;
            } else {
                minwidth = frame_minwidth((*oldwin).w_frame, nowin());
                available = (*(*oldwin).w_frame).fr_width;
                needed += minwidth;
            }
            if available < needed {
                emsg(gettext(e_noroom.as_ptr()));
                return ptr::null_mut();
            }
            if new_size == 0 {
                new_size = (*oldwin).w_width / 2;
            }
            new_size = new_size.min(available - minwidth - 1).max(wmw1);

            if OptInt::from((*oldwin).w_width - new_size - 1) < p_wmw {
                do_equal = true;
            }

            if (*oldwin).w_onebuf_opt.wo_wfw != 0 {
                win_setwidth_win((*oldwin).w_width + new_size + 1, oldwin, true);
            }

            if !do_equal
                && p_ea != 0
                && size == 0
                && *p_ead != b'v' as c_char
                && !(*(*oldwin).w_frame).fr_parent.is_null()
            {
                let mut frp = (*(*(*oldwin).w_frame).fr_parent).fr_child;
                while !frp.is_null() {
                    if !ptr::eq((*frp).fr_win, oldwin)
                        && !(*frp).fr_win.is_null()
                        && ((*(*frp).fr_win).w_width > new_size
                            || (*(*frp).fr_win).w_width > ((*oldwin).w_width - new_size - 1))
                    {
                        do_equal = true;
                        break;
                    }
                    frp = (*frp).fr_next;
                }
            }
        } else {
            let wmh1 = (p_wmh as c_int).max(1) + (*oldwin).w_winbar_height;
            let mut needed = wmh1 + STATUS_HEIGHT;
            if (flags & WSP_ROOM) != 0 {
                needed += p_wh as c_int - wmh1 + (*oldwin).w_winbar_height;
            }
            if p_ch < 1 {
                needed += 1;
            }
            let mut minheight: c_int;
            let available: c_int;
            if toplevel {
                minheight = frame_minheight(topframe, nowin()) + need_status;
                available = (*topframe).fr_height;
                needed += minheight;
            } else if p_ea != 0 {
                minheight = frame_minheight((*oldwin).w_frame, nowin()) + need_status;
                let mut prevfrp = (*oldwin).w_frame;
                let mut frp = (*(*oldwin).w_frame).fr_parent;
                while !frp.is_null() {
                    if (*frp).fr_layout == FR_COL {
                        let mut frp2 = (*frp).fr_child;
                        while !frp2.is_null() {
                            if !ptr::eq(frp2, prevfrp) {
                                minheight += frame_minheight(frp2, nowin());
                            }
                            frp2 = (*frp2).fr_next;
                        }
                    }
                    prevfrp = frp;
                    frp = (*frp).fr_parent;
                }
                available = (*topframe).fr_height;
                needed += minheight;
            } else {
                minheight = frame_minheight((*oldwin).w_frame, nowin()) + need_status;
                available = (*(*oldwin).w_frame).fr_height;
                needed += minheight;
            }
            if available < needed {
                emsg(gettext(e_noroom.as_ptr()));
                return ptr::null_mut();
            }
            oldwin_height = (*oldwin).w_height;
            if need_status != 0 {
                (*oldwin).w_status_height = STATUS_HEIGHT;
                oldwin_height -= STATUS_HEIGHT;
            }
            if new_size == 0 {
                new_size = oldwin_height / 2;
            }

            new_size = new_size
                .min(available - minheight - STATUS_HEIGHT)
                .max(wmh1);

            if OptInt::from(oldwin_height - new_size - STATUS_HEIGHT) < p_wmh {
                do_equal = true;
            }

            if (*oldwin).w_onebuf_opt.wo_wfh != 0 {
                set_fraction(oldwin);
                did_set_fraction = true;

                win_setheight_win((*oldwin).w_height + new_size + STATUS_HEIGHT, oldwin, true);
                oldwin_height = (*oldwin).w_height;
            }

            if !do_equal
                && p_ea != 0
                && size == 0
                && *p_ead != b'h' as c_char
                && !(*(*oldwin).w_frame).fr_parent.is_null()
            {
                let mut frp = (*(*(*oldwin).w_frame).fr_parent).fr_child;
                while !frp.is_null() {
                    if !ptr::eq((*frp).fr_win, oldwin)
                        && !(*frp).fr_win.is_null()
                        && ((*(*frp).fr_win).w_height > new_size
                            || (*(*frp).fr_win).w_height > oldwin_height - new_size - STATUS_HEIGHT)
                    {
                        do_equal = true;
                        break;
                    }
                    frp = (*frp).fr_next;
                }
            }
        }

        if (flags & WSP_TOP) == 0
            && ((flags & WSP_BOT) != 0
                || (flags & WSP_BELOW) != 0
                || ((flags & WSP_ABOVE) == 0 && if vertical { p_spr != 0 } else { p_sb != 0 }))
        {
            if new_wp.is_null() {
                wp = win_alloc(oldwin, false);
            } else {
                win_append(oldwin, wp, ptr::null_mut());
            }
        } else if new_wp.is_null() {
            wp = win_alloc((*oldwin).w_prev, false);
        } else {
            win_append((*oldwin).w_prev, wp, ptr::null_mut());
        }

        if new_wp.is_null() {
            if wp.is_null() {
                return ptr::null_mut();
            }

            new_frame(wp);
            win_init(wp, curwin, flags);
        } else if (*wp).w_floating {
            ui_comp_remove_grid(&raw mut (*wp).w_grid_alloc);
            if ui_has(kUIMultigrid) {
                (*wp).w_pos_changed = true;
            } else {
                ui_call_win_hide(Integer::from((*wp).w_grid_alloc.handle));
                win_free_grid(wp, true);
            }

            if (*wp).w_config.external {
                let mut tp = first_tabpage;
                while !tp.is_null() {
                    if !ptr::eq(tp, curtab) && ptr::eq((*tp).tp_curwin, wp) {
                        (*tp).tp_curwin = (*tp).tp_firstwin;
                    }
                    tp = (*tp).tp_next;
                }
            }

            (*wp).w_floating = false;
            new_frame(wp);

            clear_float_config(&raw mut (*wp).w_config, true);
            (*wp).w_border_adj = [0; 4];
        }

        if !to_flatten.is_null() {
            frame_flatten(to_flatten);
        }

        let before: bool;
        let mut curfrp: *mut frame_T;

        if toplevel {
            if ((*topframe).fr_layout == FR_COL && !vertical)
                || ((*topframe).fr_layout == FR_ROW && vertical)
            {
                curfrp = (*topframe).fr_child;
                if (flags & WSP_BOT) != 0 {
                    while !(*curfrp).fr_next.is_null() {
                        curfrp = (*curfrp).fr_next;
                    }
                }
            } else {
                curfrp = topframe;
            }
            before = (flags & WSP_TOP) != 0;
        } else {
            curfrp = (*oldwin).w_frame;
            if (flags & WSP_BELOW) != 0 {
                before = false;
            } else if (flags & WSP_ABOVE) != 0 {
                before = true;
            } else if vertical {
                before = p_spr == 0;
            } else {
                before = p_sb == 0;
            }
        }
        if (*curfrp).fr_parent.is_null() || (*(*curfrp).fr_parent).fr_layout != layout {
            let mut frp = xcalloc(1, size_of::<frame_T>()).cast::<frame_T>();
            ptr::copy_nonoverlapping(curfrp, frp, 1);
            (*curfrp).fr_layout = layout;
            (*frp).fr_parent = curfrp;
            (*frp).fr_next = ptr::null_mut();
            (*frp).fr_prev = ptr::null_mut();
            (*curfrp).fr_child = frp;
            (*curfrp).fr_win = ptr::null_mut();
            curfrp = frp;
            if !(*frp).fr_win.is_null() {
                (*oldwin).w_frame = frp;
            } else {
                frp = (*frp).fr_child;
                while !frp.is_null() {
                    (*frp).fr_parent = curfrp;
                    frp = (*frp).fr_next;
                }
            }
        }

        let frp = if new_wp.is_null() {
            (*wp).w_frame
        } else {
            (*new_wp).w_frame
        };
        (*frp).fr_parent = (*curfrp).fr_parent;

        if before {
            frame_insert(curfrp, frp);
        } else {
            frame_append(curfrp, frp);
        }

        if !did_set_fraction {
            set_fraction(oldwin);
        }
        (*wp).w_fraction = (*oldwin).w_fraction;

        if vertical {
            (*wp).w_onebuf_opt.wo_scr = (*curwin).w_onebuf_opt.wo_scr;

            if need_status != 0 {
                win_new_height(oldwin, (*oldwin).w_height - 1);
                (*oldwin).w_status_height = need_status;
            }
            if toplevel {
                (*wp).w_winrow = tabline_height();
                let ls_h = c_int::from(p_ls == 1 || p_ls == 2);
                win_new_height(wp, (*curfrp).fr_height - ls_h);
                (*wp).w_status_height = ls_h;
                (*wp).w_hsep_height = 0;
            } else {
                (*wp).w_winrow = (*oldwin).w_winrow;
                win_new_height(wp, (*oldwin).w_height);
                (*wp).w_status_height = (*oldwin).w_status_height;
                (*wp).w_hsep_height = (*oldwin).w_hsep_height;
            }
            (*frp).fr_height = (*curfrp).fr_height;

            win_new_width(wp, new_size);
            if before {
                (*wp).w_vsep_width = 1;
            } else {
                (*wp).w_vsep_width = (*oldwin).w_vsep_width;
                (*oldwin).w_vsep_width = 1;
            }
            if toplevel {
                if (flags & WSP_BOT) != 0 {
                    frame_set_vsep(curfrp, true);
                }
                frame_new_width(
                    curfrp,
                    (*curfrp).fr_width - (new_size + c_int::from((flags & WSP_TOP) != 0)),
                    (flags & WSP_TOP) != 0,
                    false,
                );
            } else {
                win_new_width(oldwin, (*oldwin).w_width - (new_size + 1));
            }
            if before {
                (*wp).w_wincol = (*oldwin).w_wincol;
                (*oldwin).w_wincol += new_size + 1;
            } else {
                (*wp).w_wincol = (*oldwin).w_wincol + (*oldwin).w_width + 1;
            }
            frame_fix_width(oldwin);
            frame_fix_width(wp);
        } else {
            let is_stl_global = global_stl_height() > 0;
            if toplevel {
                (*wp).w_wincol = 0;
                win_new_width(wp, Columns);
                (*wp).w_vsep_width = 0;
            } else {
                (*wp).w_wincol = (*oldwin).w_wincol;
                win_new_width(wp, (*oldwin).w_width);
                (*wp).w_vsep_width = (*oldwin).w_vsep_width;
            }
            (*frp).fr_width = (*curfrp).fr_width;

            win_new_height(wp, new_size);
            let old_status_height = (*oldwin).w_status_height;
            if before {
                (*wp).w_hsep_height = c_int::from(is_stl_global);
            } else {
                (*wp).w_hsep_height = (*oldwin).w_hsep_height;
                (*oldwin).w_hsep_height = c_int::from(is_stl_global);
            }
            if toplevel {
                let mut new_fr_height = (*curfrp).fr_height - new_size;
                if is_stl_global {
                    if (flags & WSP_BOT) != 0 {
                        frame_add_hsep(curfrp);
                    } else {
                        new_fr_height -= 1;
                    }
                } else {
                    if !((flags & WSP_BOT) != 0 && p_ls == 0) {
                        new_fr_height -= STATUS_HEIGHT;
                    }
                    if (flags & WSP_BOT) != 0 {
                        frame_add_statusline(curfrp);
                    }
                }
                frame_new_height(curfrp, new_fr_height, (flags & WSP_TOP) != 0, false, false);
            } else {
                win_new_height(oldwin, oldwin_height - (new_size + STATUS_HEIGHT));
            }

            if before {
                (*wp).w_winrow = (*oldwin).w_winrow;
                if is_stl_global {
                    (*wp).w_status_height = 0;
                    (*oldwin).w_winrow += (*wp).w_height + 1;
                } else {
                    (*wp).w_status_height = STATUS_HEIGHT;
                    (*oldwin).w_winrow += (*wp).w_height + STATUS_HEIGHT;
                }
            } else if is_stl_global {
                (*wp).w_winrow = (*oldwin).w_winrow + (*oldwin).w_height + 1;
                (*wp).w_status_height = 0;
            } else {
                (*wp).w_winrow = (*oldwin).w_winrow + (*oldwin).w_height + STATUS_HEIGHT;
                (*wp).w_status_height = old_status_height;
                if (flags & WSP_BOT) == 0 {
                    (*oldwin).w_status_height = STATUS_HEIGHT;
                }
            }
            frame_fix_height(wp);
            frame_fix_height(oldwin);
        }

        if toplevel {
            win_comp_pos();
        }

        redraw_later(wp, UPD_NOT_VALID);
        redraw_later(oldwin, UPD_NOT_VALID);
        status_redraw_all();

        if need_status != 0 {
            msg_row = Rows - 1;
            msg_col = sc_col;
            msg_clr_eos_force();
            comp_col();
            msg_row = Rows - 1;
            msg_col = 0;
        }

        if do_equal || dir != 0 {
            win_equal(
                wp,
                true,
                if vertical {
                    if dir == b'v' as c_int {
                        b'b' as c_int
                    } else {
                        b'h' as c_int
                    }
                } else if dir == b'h' as c_int {
                    b'b' as c_int
                } else {
                    b'v' as c_int
                },
            );
        } else if !is_ctx_win(wp) {
            win_fix_scroll(false);
        }

        let i: c_int;
        if (flags & WSP_VERT) != 0 {
            i = p_wiw as c_int;
            if size != 0 {
                p_wiw = OptInt::from(size);
            }
        } else {
            i = p_wh as c_int;
            if size != 0 {
                p_wh = OptInt::from(size);
            }
        }

        if (flags & WSP_NOENTER) == 0 {
            win_enter_ext(
                wp,
                (if new_wp.is_null() {
                    WEE_TRIGGER_NEW_AUTOCMDS
                } else {
                    0
                }) | WEE_TRIGGER_ENTER_AUTOCMDS
                    | WEE_TRIGGER_LEAVE_AUTOCMDS,
            );
        }
        if vertical {
            p_wiw = OptInt::from(i);
        } else {
            p_wh = OptInt::from(i);
        }

        if win_valid(oldwin) {
            (*oldwin).w_pos_changed = true;
        }

        wp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_init(newp: *mut win_T, oldp: *mut win_T, flags: c_int) {
    unsafe {
        (*newp).w_buffer = (*oldp).w_buffer;
        (*newp).w_s = (&raw mut (*(*oldp).w_buffer).b_s).cast();
        (*(*oldp).w_buffer).b_nwindows += 1;
        (*newp).w_cursor = (*oldp).w_cursor;
        (*newp).w_valid = 0;
        (*newp).w_curswant = (*oldp).w_curswant;
        (*newp).w_set_curswant = (*oldp).w_set_curswant;
        (*newp).w_topline = (*oldp).w_topline;
        (*newp).w_topfill = (*oldp).w_topfill;
        (*newp).w_leftcol = (*oldp).w_leftcol;
        (*newp).w_pcmark = (*oldp).w_pcmark;
        (*newp).w_prev_pcmark = (*oldp).w_prev_pcmark;
        (*newp).w_alt_fnum = (*oldp).w_alt_fnum;
        (*newp).w_wrow = (*oldp).w_wrow;
        (*newp).w_fraction = (*oldp).w_fraction;
        (*newp).w_prev_fraction_row = (*oldp).w_prev_fraction_row;
        copy_jumplist(oldp, newp);
        if (flags & WSP_NEWLOC) != 0 {
            (*newp).w_llist = ptr::null_mut();
            (*newp).w_llist_ref = ptr::null_mut();
        } else {
            copy_loclist_stack(oldp, newp);
        }
        (*newp).w_localdir = if (*oldp).w_localdir.is_null() {
            ptr::null_mut()
        } else {
            xstrdup((*oldp).w_localdir)
        };
        (*newp).w_prevdir = if (*oldp).w_prevdir.is_null() {
            ptr::null_mut()
        } else {
            xstrdup((*oldp).w_prevdir)
        };

        if *p_spk != b'c' as c_char {
            if *p_spk == b't' as c_char {
                (*newp).w_skipcol = (*oldp).w_skipcol;
            }
            (*newp).w_botline = (*oldp).w_botline;
            (*newp).w_prev_height = (*oldp).w_height;
            (*newp).w_prev_winrow = (*oldp).w_winrow;
        }

        for i in 0..(*oldp).w_tagstacklen as usize {
            let tag = &raw mut (*newp).w_tagstack[i];
            *tag = (*oldp).w_tagstack[i];
            if !(*tag).tagname.is_null() {
                (*tag).tagname = xstrdup((*tag).tagname);
            }
            if !(*tag).user_data.is_null() {
                (*tag).user_data = xstrdup((*tag).user_data);
            }
        }
        (*newp).w_tagstackidx = (*oldp).w_tagstackidx;
        (*newp).w_tagstacklen = (*oldp).w_tagstacklen;

        (*newp).w_changelistidx = (*oldp).w_changelistidx;

        copyFoldingState(oldp, newp);

        win_init_some(newp, oldp);

        (*newp).w_winbar_height = (*oldp).w_winbar_height;
    }
}

unsafe fn win_init_some(newp: *mut win_T, oldp: *mut win_T) {
    unsafe {
        (*newp).w_alist = (*oldp).w_alist;
        (*(*newp).w_alist.cast::<alist_T>()).al_refcount += 1;
        (*newp).w_arg_idx = (*oldp).w_arg_idx;

        win_copy_options(oldp, newp);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_valid(win: *const win_T) -> bool {
    unsafe { tabpage_win_valid(curtab, win) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tabpage_win_valid(tp: *const tabpage_T, win: *const win_T) -> bool {
    unsafe {
        if win.is_null() {
            return false;
        }

        let mut wp: *const win_T = if ptr::eq(tp, curtab) {
            firstwin
        } else {
            (*tp).tp_firstwin
        };
        while !wp.is_null() {
            if ptr::eq(wp, win) {
                return true;
            }
            wp = (*wp).w_next;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_find_by_handle(handle: handle_T) -> *mut win_T {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).handle == handle {
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_valid_any_tab(win: *mut win_T) -> bool {
    unsafe {
        if win.is_null() {
            return false;
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if ptr::eq(tp, curtab) {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if ptr::eq(wp, win) {
                    return true;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_count() -> c_int {
    unsafe {
        let mut count: c_int = 0;
        let mut wp = firstwin;
        while !wp.is_null() {
            count += 1;
            wp = (*wp).w_next;
        }
        count
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn make_windows(mut count: c_int, vertical: bool) -> c_int {
    unsafe {
        let mut maxcount = if vertical {
            ((OptInt::from((*curwin).w_width + (*curwin).w_vsep_width) - (p_wiw - p_wmw)) as c_int)
                / (p_wmw as c_int + 1)
        } else {
            ((OptInt::from(
                (*curwin).w_height + (*curwin).w_hsep_height + (*curwin).w_status_height,
            ) - (p_wh - p_wmh)) as c_int)
                / (p_wmh as c_int + STATUS_HEIGHT + global_winbar_height())
        };

        maxcount = maxcount.max(2);
        count = count.min(maxcount);

        if count > 1 {
            last_status(true);
        }

        block_autocmds();

        let mut todo = count - 1;
        while todo > 0 {
            if vertical {
                if win_split(
                    (*curwin).w_width - ((*curwin).w_width - todo) / (todo + 1) - 1,
                    WSP_VERT | WSP_ABOVE,
                ) == FAIL
                {
                    break;
                }
            } else if win_split(
                (*curwin).w_height
                    - ((*curwin).w_height - todo * STATUS_HEIGHT) / (todo + 1)
                    - STATUS_HEIGHT,
                WSP_ABOVE,
            ) == FAIL
            {
                break;
            }
            todo -= 1;
        }

        unblock_autocmds();

        count - todo
    }
}

unsafe fn win_exchange(mut Prenum: c_int) {
    unsafe {
        if (*curwin).w_floating {
            emsg(e_floatexchange.as_ptr());
            return;
        }

        if one_window(curwin, ptr::null_mut()) {
            beep_flush();
            return;
        }
        if text_or_buf_locked() {
            beep_flush();
            return;
        }

        let mut frp: *mut frame_T;
        if Prenum != 0 {
            frp = (*(*(*curwin).w_frame).fr_parent).fr_child;
            while !frp.is_null() && {
                Prenum -= 1;
                Prenum > 0
            } {
                frp = (*frp).fr_next;
            }
        } else if !(*(*curwin).w_frame).fr_next.is_null() {
            frp = (*(*curwin).w_frame).fr_next;
        } else {
            frp = (*(*curwin).w_frame).fr_prev;
        }

        if frp.is_null() || (*frp).fr_win.is_null() || ptr::eq((*frp).fr_win, curwin) {
            return;
        }
        let wp = (*frp).fr_win;

        let wp2 = (*curwin).w_prev;
        let frp2 = (*(*curwin).w_frame).fr_prev;
        if !ptr::eq((*wp).w_prev, curwin) {
            win_remove(curwin, ptr::null_mut());
            frame_remove((*curwin).w_frame);
            win_append((*wp).w_prev, curwin, ptr::null_mut());
            frame_insert(frp, (*curwin).w_frame);
        }
        if !ptr::eq(wp, wp2) {
            win_remove(wp, ptr::null_mut());
            frame_remove((*wp).w_frame);
            win_append(wp2, wp, ptr::null_mut());
            if frp2.is_null() {
                frame_insert((*(*(*wp).w_frame).fr_parent).fr_child, (*wp).w_frame);
            } else {
                frame_append(frp2, (*wp).w_frame);
            }
        }
        std::mem::swap(&mut (*curwin).w_status_height, &mut (*wp).w_status_height);
        std::mem::swap(&mut (*curwin).w_vsep_width, &mut (*wp).w_vsep_width);
        std::mem::swap(&mut (*curwin).w_hsep_height, &mut (*wp).w_hsep_height);

        frame_fix_height(curwin);
        frame_fix_height(wp);
        frame_fix_width(curwin);
        frame_fix_width(wp);

        win_comp_pos();

        if !ptr::eq((*wp).w_buffer, curbuf) {
            reset_VIsual_and_resel();
        } else if Visual.active {
            (*wp).w_cursor = (*curwin).w_cursor;
        }

        win_enter(wp, true);
        redraw_later(curwin, UPD_NOT_VALID);
    }
}

unsafe fn win_rotate(upwards: bool, mut count: c_int) {
    unsafe {
        if (*curwin).w_floating {
            emsg(e_floatexchange.as_ptr());
            return;
        }

        if count <= 0 || one_window(curwin, ptr::null_mut()) {
            beep_flush();
            return;
        }

        let mut frp = (*(*(*curwin).w_frame).fr_parent).fr_child;
        while !frp.is_null() {
            if (*frp).fr_win.is_null() {
                emsg(gettext(
                    c"E443: Cannot rotate when another window is split".as_ptr(),
                ));
                return;
            }
            frp = (*frp).fr_next;
        }

        let mut wp1: *mut win_T = ptr::null_mut();
        let mut wp2: *mut win_T = ptr::null_mut();

        while count > 0 {
            count -= 1;
            if upwards {
                frp = (*(*(*curwin).w_frame).fr_parent).fr_child;
                debug_assert!(!frp.is_null());
                wp1 = (*frp).fr_win;
                win_remove(wp1, ptr::null_mut());
                frame_remove(frp);
                debug_assert!(!(*(*frp).fr_parent).fr_child.is_null());

                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                win_append((*frp).fr_win, wp1, ptr::null_mut());
                frame_append(frp, (*wp1).w_frame);

                wp2 = (*frp).fr_win;
            } else {
                frp = (*curwin).w_frame;
                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                wp1 = (*frp).fr_win;
                wp2 = (*wp1).w_prev;
                win_remove(wp1, ptr::null_mut());
                frame_remove(frp);
                debug_assert!(!(*(*frp).fr_parent).fr_child.is_null());

                win_append(
                    (*(*(*(*frp).fr_parent).fr_child).fr_win).w_prev,
                    wp1,
                    ptr::null_mut(),
                );
                frame_insert((*(*frp).fr_parent).fr_child, frp);
            }

            std::mem::swap(&mut (*wp2).w_status_height, &mut (*wp1).w_status_height);
            std::mem::swap(&mut (*wp2).w_hsep_height, &mut (*wp1).w_hsep_height);
            frame_fix_height(wp1);
            frame_fix_height(wp2);
            std::mem::swap(&mut (*wp2).w_vsep_width, &mut (*wp1).w_vsep_width);
            frame_fix_width(wp1);
            frame_fix_width(wp2);

            win_comp_pos();
        }

        (*wp1).w_pos_changed = true;
        (*wp2).w_pos_changed = true;

        redraw_all_later(UPD_NOT_VALID);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_splitmove(wp: *mut win_T, size: c_int, flags: c_int) -> c_int {
    unsafe {
        let mut dir: c_int = 0;
        let height = (*wp).w_height;

        if one_window(wp, ptr::null_mut()) {
            return OK;
        }
        if is_ctx_win(wp) || check_split_disallowed(wp) == FAIL {
            return FAIL;
        }

        let mut unflat_altfr: *mut frame_T = ptr::null_mut();
        if (*wp).w_floating {
            win_remove(wp, ptr::null_mut());
        } else {
            winframe_remove(wp, &raw mut dir, ptr::null_mut(), &raw mut unflat_altfr);
            debug_assert!(!unflat_altfr.is_null());
            win_remove(wp, ptr::null_mut());
            last_status(false);
            win_comp_pos();
        }

        if win_split_ins(size, flags, wp, dir, unflat_altfr).is_null() {
            if !(*wp).w_floating {
                debug_assert!(!unflat_altfr.is_null());
                winframe_restore(wp, dir, unflat_altfr);
            }
            win_append((*wp).w_prev, wp, ptr::null_mut());
            return FAIL;
        }

        if size == 0 && (flags & WSP_VERT) == 0 && win_valid(wp) && !(*wp).w_floating {
            win_setheight_win(height, wp, true);
            if p_ea != 0 {
                win_equal(curwin, ptr::eq(curwin, wp), b'v' as c_int);
            }
        }

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_move_after(win1: *mut win_T, win2: *mut win_T) {
    unsafe {
        if ptr::eq(win1, win2) {
            return;
        }

        if !ptr::eq((*win2).w_next, win1) {
            if !ptr::eq((*(*win1).w_frame).fr_parent, (*(*win2).w_frame).fr_parent) {
                iemsg(c"INTERNAL: trying to move a window into another frame".as_ptr());
                return;
            }

            if ptr::eq(win1, lastwin) {
                std::mem::swap(
                    &mut (*(*win1).w_prev).w_status_height,
                    &mut (*win1).w_status_height,
                );
                std::mem::swap(
                    &mut (*(*win1).w_prev).w_hsep_height,
                    &mut (*win1).w_hsep_height,
                );

                if (*(*win1).w_prev).w_vsep_width == 1 {
                    (*(*win1).w_prev).w_vsep_width = 0;
                    (*(*(*win1).w_prev).w_frame).fr_width -= 1;
                    (*win1).w_vsep_width = 1;
                    (*(*win1).w_frame).fr_width += 1;
                }
            } else if ptr::eq(win2, lastwin) {
                std::mem::swap(&mut (*win1).w_status_height, &mut (*win2).w_status_height);
                std::mem::swap(&mut (*win1).w_hsep_height, &mut (*win2).w_hsep_height);

                if (*win1).w_vsep_width == 1 {
                    (*win2).w_vsep_width = 1;
                    (*(*win2).w_frame).fr_width += 1;
                    (*win1).w_vsep_width = 0;
                    (*(*win1).w_frame).fr_width -= 1;
                }
            }
            win_remove(win1, ptr::null_mut());
            frame_remove((*win1).w_frame);
            win_append(win2, win1, ptr::null_mut());
            frame_append((*win2).w_frame, (*win1).w_frame);

            win_comp_pos();
            redraw_later(curwin, UPD_NOT_VALID);
        }
        (*win1).w_pos_changed = true;
        (*win2).w_pos_changed = true;

        win_enter(win1, false);
    }
}

unsafe fn get_maximum_wincount(fr: *mut frame_T, mut height: c_int) -> c_int {
    unsafe {
        if (*fr).fr_layout != FR_COL {
            return height / (p_wmh as c_int + STATUS_HEIGHT + (*frame2win(fr)).w_winbar_height);
        } else if global_winbar_height() != 0 {
            return height / (p_wmh as c_int + STATUS_HEIGHT + 1);
        }

        let mut total_wincount: c_int = 0;
        let mut frp = (*fr).fr_child;
        while !frp.is_null() {
            let wp = frame2win(frp);
            if OptInt::from(height) < (p_wmh + OptInt::from(STATUS_HEIGHT + (*wp).w_winbar_height))
            {
                break;
            }
            height -= p_wmh as c_int + STATUS_HEIGHT + (*wp).w_winbar_height;
            total_wincount += 1;
            frp = (*frp).fr_next;
        }

        total_wincount += height / (p_wmh as c_int + STATUS_HEIGHT);
        total_wincount
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_equal(next_curwin: *mut win_T, current: bool, mut dir: c_int) {
    unsafe {
        if dir == 0 {
            dir = c_int::from(*p_ead as u8);
        }
        win_equal_rec(
            if next_curwin.is_null() {
                curwin
            } else {
                next_curwin
            },
            current,
            topframe,
            dir,
            0,
            tabline_height(),
            Columns,
            (*topframe).fr_height,
        );
        if !is_ctx_win(next_curwin) {
            win_fix_scroll(true);
        }
    }
}

unsafe fn win_equal_rec(
    next_curwin: *mut win_T,
    current: bool,
    topfr: *mut frame_T,
    dir: c_int,
    mut col: c_int,
    mut row: c_int,
    mut width: c_int,
    mut height: c_int,
) {
    unsafe {
        let mut extra_sep: c_int = 0;
        let mut totwincount: c_int = 0;
        let mut next_curwin_size: c_int = 0;
        let mut room: c_int = 0;
        let mut has_next_curwin = false;

        if (*topfr).fr_layout == FR_LEAF {
            if (*topfr).fr_height != height
                || (*(*topfr).fr_win).w_winrow != row
                || (*topfr).fr_width != width
                || (*(*topfr).fr_win).w_wincol != col
            {
                (*(*topfr).fr_win).w_winrow = row;
                frame_new_height(topfr, height, false, false, false);
                (*(*topfr).fr_win).w_wincol = col;
                frame_new_width(topfr, width, false, false);
                redraw_all_later(UPD_NOT_VALID);
            }
        } else if (*topfr).fr_layout == FR_ROW {
            (*topfr).fr_width = width;
            (*topfr).fr_height = height;

            if dir != b'v' as c_int {
                let mut n = frame_minwidth(topfr, nowin());
                if col + width == Columns {
                    extra_sep = 1;
                } else {
                    extra_sep = 0;
                }
                totwincount = (n + extra_sep) / (p_wmw as c_int + 1);
                has_next_curwin = frame_has_win(topfr, next_curwin);

                let m = frame_minwidth(topfr, next_curwin);
                room = width - m;
                if room < 0 {
                    next_curwin_size = p_wiw as c_int + room;
                    room = 0;
                } else {
                    next_curwin_size = -1;
                    let mut fr = (*topfr).fr_child;
                    while !fr.is_null() {
                        if frame_fixed_width(fr) {
                            n = frame_minwidth(fr, nowin());
                            let mut new_size = (*fr).fr_width;
                            if frame_has_win(fr, next_curwin) {
                                room += p_wiw as c_int - p_wmw as c_int;
                                next_curwin_size = 0;
                                new_size = new_size.max(p_wiw as c_int);
                            } else {
                                totwincount -= (n + if (*fr).fr_next.is_null() {
                                    extra_sep
                                } else {
                                    0
                                }) / (p_wmw as c_int + 1);
                            }
                            room -= new_size - n;
                            if room < 0 {
                                new_size += room;
                                room = 0;
                            }
                            (*fr).fr_newwidth = new_size;
                        }
                        fr = (*fr).fr_next;
                    }
                    if next_curwin_size == -1 {
                        if !has_next_curwin {
                            next_curwin_size = 0;
                        } else if totwincount > 1
                            && OptInt::from((room + (totwincount - 2)) / (totwincount - 1)) > p_wiw
                        {
                            next_curwin_size = ((OptInt::from(room)
                                + p_wiw
                                + OptInt::from(totwincount - 1) * p_wmw
                                + OptInt::from(totwincount - 1))
                                as c_int)
                                / totwincount;
                            room -= next_curwin_size - p_wiw as c_int;
                        } else {
                            next_curwin_size = p_wiw as c_int;
                        }
                    }
                }

                if has_next_curwin {
                    totwincount -= 1;
                }
            }

            let mut fr = (*topfr).fr_child;
            while !fr.is_null() {
                let mut wincount: c_int = 1;
                let new_size: c_int;
                if (*fr).fr_next.is_null() {
                    new_size = width;
                } else if dir == b'v' as c_int {
                    new_size = (*fr).fr_width;
                } else if frame_fixed_width(fr) {
                    new_size = (*fr).fr_newwidth;
                    wincount = 0;
                } else {
                    let n = frame_minwidth(fr, nowin());
                    wincount = (n + if (*fr).fr_next.is_null() {
                        extra_sep
                    } else {
                        0
                    }) / (p_wmw as c_int + 1);
                    let m = frame_minwidth(fr, next_curwin);
                    let hnc = has_next_curwin && frame_has_win(fr, next_curwin);
                    if hnc {
                        wincount -= 1;
                    }
                    let mut ns = if totwincount == 0 {
                        room
                    } else {
                        (wincount * room + (totwincount / 2)) / totwincount
                    };
                    if hnc {
                        next_curwin_size -= p_wiw as c_int - (m - n);
                        next_curwin_size = next_curwin_size.max(0);
                        ns += next_curwin_size;
                        room -= ns - next_curwin_size;
                    } else {
                        room -= ns;
                    }
                    new_size = ns + n;
                }

                if !current
                    || dir != b'v' as c_int
                    || !(*topfr).fr_parent.is_null()
                    || new_size != (*fr).fr_width
                    || frame_has_win(fr, next_curwin)
                {
                    win_equal_rec(next_curwin, current, fr, dir, col, row, new_size, height);
                }
                col += new_size;
                width -= new_size;
                totwincount -= wincount;
                fr = (*fr).fr_next;
            }
        } else {
            (*topfr).fr_width = width;
            (*topfr).fr_height = height;

            if dir != b'h' as c_int {
                let mut n = frame_minheight(topfr, nowin());
                if row + height >= cmdline_row && p_ls == 0 {
                    extra_sep = STATUS_HEIGHT;
                } else if global_stl_height() > 0 {
                    extra_sep = 1;
                } else {
                    extra_sep = 0;
                }
                totwincount = get_maximum_wincount(topfr, n + extra_sep);
                has_next_curwin = frame_has_win(topfr, next_curwin);

                let m = frame_minheight(topfr, next_curwin);
                room = height - m;
                if room < 0 {
                    next_curwin_size = p_wh as c_int + room;
                    room = 0;
                } else {
                    next_curwin_size = -1;
                    let mut fr = (*topfr).fr_child;
                    while !fr.is_null() {
                        if frame_fixed_height(fr) {
                            n = frame_minheight(fr, nowin());
                            let mut new_size = (*fr).fr_height;
                            if frame_has_win(fr, next_curwin) {
                                room += p_wh as c_int - p_wmh as c_int;
                                next_curwin_size = 0;
                                new_size = new_size.max(p_wh as c_int);
                            } else {
                                totwincount -= get_maximum_wincount(
                                    fr,
                                    n + if (*fr).fr_next.is_null() {
                                        extra_sep
                                    } else {
                                        0
                                    },
                                );
                            }
                            room -= new_size - n;
                            if room < 0 {
                                new_size += room;
                                room = 0;
                            }
                            (*fr).fr_newheight = new_size;
                        }
                        fr = (*fr).fr_next;
                    }
                    if next_curwin_size == -1 {
                        if !has_next_curwin {
                            next_curwin_size = 0;
                        } else if totwincount > 1
                            && OptInt::from((room + (totwincount - 2)) / (totwincount - 1)) > p_wh
                        {
                            next_curwin_size = ((OptInt::from(room)
                                + p_wh
                                + OptInt::from(totwincount - 1) * p_wmh
                                + OptInt::from(totwincount - 1))
                                as c_int)
                                / totwincount;
                            room -= next_curwin_size - p_wh as c_int;
                        } else {
                            next_curwin_size = p_wh as c_int;
                        }
                    }
                }

                if has_next_curwin {
                    totwincount -= 1;
                }
            }

            let mut fr = (*topfr).fr_child;
            while !fr.is_null() {
                let mut wincount: c_int = 1;
                let new_size: c_int;
                if (*fr).fr_next.is_null() {
                    new_size = height;
                } else if dir == b'h' as c_int {
                    new_size = (*fr).fr_height;
                } else if frame_fixed_height(fr) {
                    new_size = (*fr).fr_newheight;
                    wincount = 0;
                } else {
                    let n = frame_minheight(fr, nowin());
                    wincount = get_maximum_wincount(
                        fr,
                        n + if (*fr).fr_next.is_null() {
                            extra_sep
                        } else {
                            0
                        },
                    );
                    let m = frame_minheight(fr, next_curwin);
                    let hnc = has_next_curwin && frame_has_win(fr, next_curwin);
                    if hnc {
                        wincount -= 1;
                    }
                    let mut ns = if totwincount == 0 {
                        room
                    } else {
                        (wincount * room + (totwincount / 2)) / totwincount
                    };
                    if hnc {
                        next_curwin_size -= p_wh as c_int - (m - n);
                        ns += next_curwin_size;
                        room -= ns - next_curwin_size;
                    } else {
                        room -= ns;
                    }
                    new_size = ns + n;
                }
                if !current
                    || dir != b'h' as c_int
                    || !(*topfr).fr_parent.is_null()
                    || new_size != (*fr).fr_height
                    || frame_has_win(fr, next_curwin)
                {
                    win_equal_rec(next_curwin, current, fr, dir, col, row, width, new_size);
                }
                row += new_size;
                height -= new_size;
                totwincount -= wincount;
                fr = (*fr).fr_next;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn leaving_window(win: *mut win_T) {
    unsafe {
        if !bt_prompt((*win).w_buffer) || is_ctx_win(win) {
            return;
        }

        (*(*win).w_buffer).b_prompt_insert = restart_edit;
        if restart_edit != NUL as c_int && mode_displayed {
            clear_cmdline = true;
        }
        restart_edit = NUL as c_int;

        if (State & MODE_INSERT) != 0 && !Ins.stop_insert_mode {
            Ins.stop_insert_mode = true;
            if (*(*win).w_buffer).b_prompt_insert == NUL as c_int {
                (*(*win).w_buffer).b_prompt_insert = b'A' as c_int;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn entering_window(win: *mut win_T) {
    unsafe {
        if !bt_prompt((*win).w_buffer) || is_ctx_win(win) {
            return;
        }

        if (*(*win).w_buffer).b_prompt_insert != NUL as c_int {
            Ins.stop_insert_mode = false;
        }

        if (State & MODE_INSERT) == 0 {
            restart_edit = (*(*win).w_buffer).b_prompt_insert;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_init_empty(wp: *mut win_T) {
    unsafe {
        redraw_later(wp, UPD_NOT_VALID);
        (*wp).w_lines_valid = 0;
        (*wp).w_cursor.lnum = 1;
        (*wp).w_cursor.col = 0;
        (*wp).w_curswant = 0;
        (*wp).w_cursor.coladd = 0;
        (*wp).w_pcmark.lnum = 1;
        (*wp).w_pcmark.col = 0;
        (*wp).w_prev_pcmark.lnum = 0;
        (*wp).w_prev_pcmark.col = 0;
        (*wp).w_topline = 1;
        (*wp).w_topfill = 0;
        (*wp).w_botline = 2;
        (*wp).w_valid = 0;
        (*wp).w_s = (&raw mut (*(*wp).w_buffer).b_s).cast();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn curwin_init() {
    unsafe {
        win_init_empty(curwin);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn close_windows(buf: *mut buf_T, keep_curwin: bool) {
    unsafe {
        RedrawingDisabled += 1;

        let mut wp = lastwin;
        while !wp.is_null() && (is_ctx_win(lastwin) || !one_window(wp, ptr::null_mut())) {
            if ptr::eq((*wp).w_buffer, buf)
                && (!keep_curwin || !ptr::eq(wp, curwin))
                && !(win_locked(wp) != 0 || (*(*wp).w_buffer).b_locked > 0)
            {
                if window_layout_locked(cmdidx_T::CMD_SIZE) {
                    RedrawingDisabled -= 1;
                    return;
                }
                if win_close(wp, false, false) == FAIL {
                    break;
                }
                wp = lastwin;
            } else {
                wp = (*wp).w_prev;
            }
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut nexttp = (*tp).tp_next;
            if !ptr::eq(tp, curtab) {
                let mut wp2 = (*tp).tp_lastwin;
                while !wp2.is_null() {
                    if ptr::eq((*wp2).w_buffer, buf)
                        && !(win_locked(wp2) != 0 || (*(*wp2).w_buffer).b_locked > 0)
                    {
                        if window_layout_locked(cmdidx_T::CMD_SIZE) {
                            RedrawingDisabled -= 1;
                            return;
                        }
                        if !win_close_othertab(wp2, 0, tp, false) {
                            RedrawingDisabled -= 1;
                            return;
                        }
                        nexttp = first_tabpage;
                        break;
                    }
                    wp2 = (*wp2).w_prev;
                }
            }
            tp = nexttp;
        }

        RedrawingDisabled -= 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn last_window(win: *mut win_T) -> bool {
    unsafe { one_window(win, ptr::null_mut()) && (*first_tabpage).tp_next.is_null() }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn one_window(win: *mut win_T, tp: *mut tabpage_T) -> bool {
    unsafe {
        let first = if !tp.is_null() {
            (*tp).tp_firstwin
        } else {
            firstwin
        };
        debug_assert!((tp.is_null() || !ptr::eq(tp, curtab)) && !(*first).w_floating);
        ptr::eq(first, win) && ((*win).w_next.is_null() || (*(*win).w_next).w_floating)
    }
}

unsafe fn can_close_floating_windows(tp: *mut tabpage_T) -> bool {
    unsafe {
        debug_assert!(!ptr::eq(tp, curtab) && (!tp.is_null() || !is_ctx_win(lastwin)));
        let mut wp = if !tp.is_null() {
            (*tp).tp_lastwin
        } else {
            lastwin
        };
        while (*wp).w_floating {
            let buf = (*wp).w_buffer;
            let need_hide = bufIsChanged(buf) && (*buf).b_nwindows <= 1;
            if need_hide && !buf_hide(buf) {
                return false;
            }
            wp = (*wp).w_prev;
        }
        true
    }
}

unsafe fn close_last_window_tabpage(
    win: *mut win_T,
    mut free_buf: bool,
    prev_curtab: *mut tabpage_T,
) -> bool {
    unsafe {
        if !one_window_macro() {
            return false;
        }

        let old_curbuf = curbuf;
        let save_lastused = lastused_tabpage;

        let term = if !(*win).w_buffer.is_null() {
            (*(*win).w_buffer).terminal
        } else {
            ptr::null_mut()
        };
        if !term.is_null() {
            free_buf = false;
        }

        goto_tabpage_tp(alt_tabpage(), false, !(*win).w_buffer.is_null());

        if !ptr::eq(curtab, prev_curtab)
            && valid_tabpage(prev_curtab)
            && ptr::eq((*prev_curtab).tp_firstwin, win)
        {
            win_close_othertab(win, c_int::from(free_buf), prev_curtab, false);
        }

        if valid_tabpage(save_lastused) && !ptr::eq(save_lastused, curtab) {
            lastused_tabpage = save_lastused;
        }
        entering_window(curwin);

        apply_autocmds(EVENT_WINENTER, ptr::null(), ptr::null(), false, curbuf);
        apply_autocmds(EVENT_TABENTER, ptr::null(), ptr::null(), false, curbuf);
        if !ptr::eq(old_curbuf, curbuf) {
            apply_autocmds(EVENT_BUFENTER, ptr::null(), ptr::null(), false, curbuf);
        }
        true
    }
}

unsafe fn win_close_buffer(win: *mut win_T, action: c_int, abort_if_last: bool) -> bool {
    unsafe {
        if !(*win).w_buffer.is_null() {
            reset_synblock(win);
        }

        if !(*win).w_buffer.is_null()
            && bt_quickfix((*win).w_buffer)
            && (*(*win).w_buffer).b_nwindows == 1
        {
            (*(*win).w_buffer).b_p_bl = 0;
        }

        let mut retval = false;
        if !(*win).w_buffer.is_null() {
            let mut bufref: bufref_T = zeroed();
            set_bufref(&raw mut bufref, curbuf);
            (*win).w_locked += 1;
            retval = close_buffer(win, (*win).w_buffer, action, abort_if_last, true, true);
            if win_valid_any_tab(win) {
                (*win).w_locked -= 1;
            }

            if !bufref_valid(&raw mut bufref) {
                curbuf = firstbuf;
            }
        }

        retval
    }
}

unsafe fn win_unclose_buffer(win: *mut win_T, _bufref: *mut bufref_T) {
    unsafe {
        if (*win).w_buffer.is_null() {
            (*win).w_buffer = firstbuf;
            (*firstbuf).b_nwindows += 1;
            if ptr::eq(win, curwin) {
                curbuf = (*curwin).w_buffer;
            }
            win_init_empty(win);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_close(win: *mut win_T, free_buf: bool, force: bool) -> c_int {
    unsafe {
        let prev_curtab = curtab;
        let win_frame = if (*win).w_floating {
            ptr::null_mut()
        } else {
            (*(*win).w_frame).fr_parent
        };
        let had_diffmode = (*win).w_onebuf_opt.wo_diff != 0;

        if last_window(win) {
            emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW.as_ptr()));
            return FAIL;
        }
        if !(*win).w_floating && window_layout_locked(cmdidx_T::CMD_close) {
            return FAIL;
        }

        if win_locked(win) != 0 || (!(*win).w_buffer.is_null() && (*(*win).w_buffer).b_locked > 0) {
            return FAIL;
        }
        if is_ctx_win(win) {
            emsg(gettext(e_autocmd_close.as_ptr()));
            return FAIL;
        }
        if (*lastwin).w_floating && one_window(win, ptr::null_mut()) {
            if is_ctx_win(lastwin) {
                emsg(gettext(
                    c"E814: Cannot close window, only autocmd window would remain".as_ptr(),
                ));
                return FAIL;
            }
            if force || can_close_floating_windows(ptr::null_mut()) {
                while (*lastwin).w_floating {
                    if win_close(lastwin, !buf_hide((*lastwin).w_buffer), true) == FAIL {
                        return FAIL;
                    }
                }
                if !win_valid_any_tab(win) {
                    return FAIL;
                }
                if last_window(win) {
                    emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW.as_ptr()));
                    return FAIL;
                }
            } else {
                emsg(e_floatonly.as_ptr());
                return FAIL;
            }
        }

        if close_last_window_tabpage(win, free_buf, prev_curtab) {
            return FAIL;
        }

        let mut help_window = false;
        let mut quickfix_window = false;

        if bt_help((*win).w_buffer) {
            help_window = true;
        } else {
            clear_snapshot(curtab, SNAP_HELP_IDX as c_int);
        }

        if bt_quickfix((*win).w_buffer) {
            quickfix_window = true;
        } else {
            clear_snapshot(curtab, SNAP_QUICKFIX_IDX as c_int);
        }

        let mut other_buffer = false;

        if ptr::eq(win, curwin) {
            leaving_window(curwin);

            let wp = if (*win).w_floating {
                win_float_find_altwin(win, ptr::null())
            } else {
                frame2win(win_altframe(win, ptr::null_mut()))
            };

            if !ptr::eq((*wp).w_buffer, curbuf) {
                reset_VIsual_and_resel();

                other_buffer = true;
                if !win_valid(win) {
                    return FAIL;
                }
                (*win).w_locked += 1;
                apply_autocmds(EVENT_BUFLEAVE, ptr::null(), ptr::null(), false, curbuf);
                if !win_valid(win) {
                    return FAIL;
                }
                (*win).w_locked -= 1;
                if last_window(win) {
                    return FAIL;
                }
            }
            (*win).w_locked += 1;
            apply_autocmds(EVENT_WINLEAVE, ptr::null(), ptr::null(), false, curbuf);
            if !win_valid(win) {
                return FAIL;
            }
            (*win).w_locked -= 1;
            if last_window(win) {
                return FAIL;
            }
            if aborting() {
                return FAIL;
            }
        }

        do_autocmd_winclosed(win);
        if !win_valid_any_tab(win) {
            return OK;
        }

        let mut bufref: bufref_T = zeroed();
        set_bufref(&raw mut bufref, (*win).w_buffer);

        win_close_buffer(win, if free_buf { DOBUF_UNLOAD } else { 0 }, true);

        if win_valid(win) && (*win).w_buffer.is_null() && !(*win).w_floating && last_window(win) {
            if (*curwin).w_buffer.is_null() {
                (*curwin).w_buffer = curbuf;
            }
            getout(0);
        }
        if !ptr::eq(curtab, prev_curtab) && win_valid_any_tab(win) && (*win).w_buffer.is_null() {
            win_close_othertab(win, 0, prev_curtab, force);
            return FAIL;
        }

        if !win_valid(win) {
            return FAIL;
        }
        if one_window(win, ptr::null_mut())
            && ((*first_tabpage).tp_next.is_null() || (*lastwin).w_floating)
        {
            if !(*first_tabpage).tp_next.is_null() {
                emsg(e_floatonly.as_ptr());
            }
            win_unclose_buffer(win, &raw mut bufref);
            return FAIL;
        }
        if close_last_window_tabpage(win, free_buf, prev_curtab) {
            return FAIL;
        }

        split_disallowed += 1;

        let was_floating = (*win).w_floating;
        if ui_has(kUIMultigrid) {
            ui_call_win_close(Integer::from((*win).w_grid_alloc.handle));
        }

        if (*win).w_floating {
            ui_comp_remove_grid(&raw mut (*win).w_grid_alloc);
            debug_assert!(!first_tabpage.is_null());
            if (*win).w_config.external {
                let mut tp = first_tabpage;
                while !tp.is_null() {
                    if !ptr::eq(tp, curtab) && ptr::eq((*tp).tp_curwin, win) {
                        (*tp).tp_curwin = (*tp).tp_firstwin;
                    }
                    tp = (*tp).tp_next;
                }
            }
        }

        set_bufref(&raw mut bufref, (*win).w_buffer);

        if !(*win).w_buffer.is_null() {
            (*(*win).w_buffer).b_nwindows -= 1;
        }

        let had_cmdline_ruler = p_ru != 0 && ptr::eq(win, curwin) && (*win).w_status_height == 0;
        let mut dir: c_int = 0;
        let mut wp = win_free_mem(win, &raw mut dir, ptr::null_mut());

        if help_window || quickfix_window {
            let prev_win = get_snapshot_curwin(if help_window {
                SNAP_HELP_IDX as c_int
            } else {
                SNAP_QUICKFIX_IDX as c_int
            });
            if win_valid(prev_win) {
                wp = prev_win;
            }
        }

        let mut close_curwin = false;

        if ptr::eq(win, curwin) {
            curwin = wp;
            if (*wp).w_onebuf_opt.wo_pvw != 0 || bt_quickfix((*wp).w_buffer) {
                loop {
                    if (*wp).w_next.is_null() {
                        wp = firstwin;
                    } else {
                        wp = (*wp).w_next;
                    }
                    if ptr::eq(wp, curwin) {
                        break;
                    }
                    if (*wp).w_onebuf_opt.wo_pvw == 0
                        && !bt_quickfix((*wp).w_buffer)
                        && !((*wp).w_floating && ((*wp).w_config.hide || !(*wp).w_config.focusable))
                    {
                        curwin = wp;
                        break;
                    }
                }
            }
            curbuf = (*curwin).w_buffer;
            close_curwin = true;

            check_cursor(curwin);
        }

        if !was_floating {
            last_status(false);

            if !(*curwin).w_floating
                && p_ea != 0
                && (*p_ead == b'b' as c_char || c_int::from(*p_ead as u8) == dir)
            {
                win_equal(
                    curwin,
                    ptr::eq((*(*curwin).w_frame).fr_parent, win_frame),
                    dir,
                );
            } else {
                win_comp_pos();
                win_fix_scroll(false);
            }
        } else if had_cmdline_ruler && (*wp).w_status_height > 0 {
            redraw_cmdline = true;
        }
        if !bufref.br_buf.is_null()
            && bufref_valid(&raw mut bufref)
            && !(*bufref.br_buf).terminal.is_null()
        {
            terminal_check_size((*bufref.br_buf).terminal.cast::<Terminal>());
        }

        if close_curwin {
            win_enter_ext(
                wp,
                WEE_CURWIN_INVALID | WEE_TRIGGER_ENTER_AUTOCMDS | WEE_TRIGGER_LEAVE_AUTOCMDS,
            );
            if other_buffer {
                apply_autocmds(EVENT_BUFENTER, ptr::null(), ptr::null(), false, curbuf);
            }
        }

        if one_window_macro()
            && (*curwin).w_locked != 0
            && (*curbuf).b_locked_split != 0
            && !(*first_tabpage).tp_next.is_null()
        {
            apply_autocmds(EVENT_TABLEAVE, ptr::null(), ptr::null(), false, curbuf);
        }

        split_disallowed -= 1;

        if help_window || quickfix_window {
            restore_snapshot(
                if help_window {
                    SNAP_HELP_IDX as c_int
                } else {
                    SNAP_QUICKFIX_IDX as c_int
                },
                c_int::from(close_curwin),
            );
        }

        if diffopt_closeoff() && had_diffmode && ptr::eq(curtab, prev_curtab) {
            let mut diffcount: c_int = 0;
            let mut dwin = firstwin;
            while !dwin.is_null() {
                if (*dwin).w_onebuf_opt.wo_diff != 0 {
                    diffcount += 1;
                }
                dwin = (*dwin).w_next;
            }
            if diffcount == 1 {
                do_cmdline_cmd(c"diffoff!".as_ptr());
            }
        }

        (*curwin).w_pos_changed = true;
        if !was_floating {
            redraw_all_later(UPD_NOT_VALID);
        }
        OK
    }
}

unsafe fn trigger_winnewpre() {
    unsafe {
        window_layout_lock();
        apply_autocmds(
            EVENT_WINNEWPRE,
            ptr::null(),
            ptr::null(),
            false,
            ptr::null_mut(),
        );
        window_layout_unlock();
    }
}

unsafe fn do_autocmd_winclosed(win: *mut win_T) {
    unsafe {
        if winclosed_recursive || !has_event(EVENT_WINCLOSED) {
            return;
        }
        winclosed_recursive = true;
        let mut winid = [0 as c_char; NUMBUFLEN];
        vim_snprintf(
            winid.as_mut_ptr(),
            winid.len(),
            c"%d".as_ptr(),
            (*win).handle,
        );
        apply_autocmds_win(
            EVENT_WINCLOSED,
            winid.as_ptr(),
            winid.as_ptr(),
            false,
            (*win).w_buffer,
            win,
        );
        winclosed_recursive = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn trigger_tabclosedpre(tp: *mut tabpage_T) {
    unsafe {
        let ptp = curtab;

        if !has_event(EVENT_TABCLOSEDPRE) || tabclosedpre_recursive {
            return;
        }

        if valid_tabpage(tp) {
            goto_tabpage_tp(tp, false, false);
        }
        tabclosedpre_recursive = true;
        window_layout_lock();
        apply_autocmds(
            EVENT_TABCLOSEDPRE,
            ptr::null(),
            ptr::null(),
            false,
            ptr::null_mut(),
        );
        window_layout_unlock();
        tabclosedpre_recursive = false;
        if valid_tabpage(ptp) {
            goto_tabpage_tp(ptp, false, false);
        } else {
            goto_tabpage_tp(first_tabpage, false, false);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_close_othertab(
    win: *mut win_T,
    free_buf: c_int,
    tp: *mut tabpage_T,
    force: bool,
) -> bool {
    unsafe {
        debug_assert!(!ptr::eq(tp, curtab));

        if window_layout_locked(cmdidx_T::CMD_SIZE) {
            return false;
        }
        if win_locked(win) != 0 || (!(*win).w_buffer.is_null() && (*(*win).w_buffer).b_locked > 0) {
            return false;
        }
        if is_ctx_win(win) {
            emsg(gettext(e_autocmd_close.as_ptr()));
            return false;
        }

        if (*(*tp).tp_lastwin).w_floating && one_window(win, tp) {
            if force || can_close_floating_windows(tp) {
                #[allow(clippy::while_immutable_condition)]
                while (*(*tp).tp_lastwin).w_floating {
                    if !win_close_othertab(
                        (*tp).tp_lastwin,
                        c_int::from(!buf_hide((*(*tp).tp_lastwin).w_buffer)),
                        tp,
                        true,
                    ) {
                        return false;
                    }
                }
                if !win_valid_any_tab(win) {
                    return false;
                }
            } else {
                emsg(e_floatonly.as_ptr());
                return false;
            }
        }

        if !(*win).w_buffer.is_null() {
            do_autocmd_winclosed(win);
            if !win_valid_any_tab(win) {
                return false;
            }
        }

        if ptr::eq((*tp).tp_firstwin, (*tp).tp_lastwin) && !(*tp).tp_did_tabclosedpre {
            trigger_tabclosedpre(tp);
            if !win_valid_any_tab(win) {
                return false;
            }
        }

        let mut bufref: bufref_T = zeroed();
        set_bufref(&raw mut bufref, (*win).w_buffer);

        if !(*win).w_buffer.is_null() {
            close_buffer(
                win,
                (*win).w_buffer,
                if free_buf != 0 { DOBUF_UNLOAD } else { 0 },
                false,
                true,
                true,
            );
        }

        if !valid_tabpage(tp) || ptr::eq(tp, curtab) || !tabpage_win_valid(tp, win) {
            if win_valid_any_tab(win) {
                win_unclose_buffer(win, &raw mut bufref);
            }
            return false;
        }
        if (*(*tp).tp_lastwin).w_floating && one_window(win, tp) {
            emsg(e_floatonly.as_ptr());
            if win_valid_any_tab(win) {
                win_unclose_buffer(win, &raw mut bufref);
            }
            return false;
        }

        let mut free_tp_idx: c_int = 0;

        if ptr::eq((*tp).tp_firstwin, (*tp).tp_lastwin) {
            free_tp_idx = tabpage_index(tp);
            let h = tabline_height();

            if ptr::eq(tp, first_tabpage) {
                first_tabpage = (*tp).tp_next;
            } else {
                let mut ptp = first_tabpage;
                while !ptp.is_null() && !ptr::eq((*ptp).tp_next, tp) {
                    ptp = (*ptp).tp_next;
                }
                if ptp.is_null() {
                    internal_error(c"win_close_othertab()".as_ptr());
                    return false;
                }
                (*ptp).tp_next = (*tp).tp_next;
            }
            redraw_tabline = true;
            if h != tabline_height() {
                win_new_screen_rows();
            }
        }

        if ui_has(kUIMultigrid) {
            ui_call_win_close(Integer::from((*win).w_grid_alloc.handle));
        }
        if (*win).w_floating {
            ui_comp_remove_grid(&raw mut (*win).w_grid_alloc);
        }

        set_bufref(&raw mut bufref, (*win).w_buffer);

        if !(*win).w_buffer.is_null() {
            (*(*win).w_buffer).b_nwindows -= 1;
        }

        let mut dir: c_int = 0;
        win_free_mem(win, &raw mut dir, tp);

        if !bufref.br_buf.is_null()
            && bufref_valid(&raw mut bufref)
            && !(*bufref.br_buf).terminal.is_null()
        {
            terminal_check_size((*bufref.br_buf).terminal.cast::<Terminal>());
        }
        if free_tp_idx > 0 {
            free_tabpage(tp);

            if has_event(EVENT_TABCLOSED) {
                let mut prev_idx = [0 as c_char; NUMBUFLEN];
                vim_snprintf(
                    prev_idx.as_mut_ptr(),
                    NUMBUFLEN,
                    c"%i".as_ptr(),
                    free_tp_idx,
                );
                apply_autocmds(
                    EVENT_TABCLOSED,
                    prev_idx.as_ptr(),
                    prev_idx.as_ptr(),
                    false,
                    if !bufref.br_buf.is_null() && bufref_valid(&raw mut bufref) {
                        bufref.br_buf
                    } else {
                        curbuf
                    },
                );
            }
        }
        true
    }
}

unsafe fn win_free_mem(win: *mut win_T, dirp: *mut c_int, tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        let wp: *mut win_T;
        let win_tp = if tp.is_null() { curtab } else { tp };

        if !(*win).w_floating {
            let frp = (*win).w_frame;
            wp = winframe_remove(win, dirp, tp, ptr::null_mut());
            xfree(frp.cast());
        } else {
            *dirp = b'h' as c_int;
            wp = win_float_find_altwin(win, tp);
        }
        win_free(win, tp);

        if ptr::eq(win, (*win_tp).tp_curwin) {
            (*win_tp).tp_curwin = wp;
        }
        if ptr::eq(win, cmdline_win) {
            cmdline_win = ptr::null_mut();
        }

        wp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_free_all() {
    unsafe {
        cmdwin_type = 0;
        cmdwin_buf = ptr::null_mut();

        while !(*first_tabpage).tp_next.is_null() {
            tabpage_close(true);
        }

        while !lastwin.is_null() && (*lastwin).w_floating {
            let wp = lastwin;
            win_remove(lastwin, ptr::null_mut());
            let mut dummy: c_int = 0;
            win_free_mem(wp, &raw mut dummy, ptr::null_mut());
            for i in 0..ctx_win_vec.size {
                if ptr::eq((*ctx_win_vec.items.add(i)).cw_win, wp) {
                    (*ctx_win_vec.items.add(i)).cw_win = ptr::null_mut();
                }
            }
        }

        for i in 0..ctx_win_vec.size {
            let cw = (*ctx_win_vec.items.add(i)).cw_win;
            if !cw.is_null() {
                let mut dummy: c_int = 0;
                win_free_mem(cw, &raw mut dummy, ptr::null_mut());
                (*ctx_win_vec.items.add(i)).cw_win = ptr::null_mut();
            }
        }

        xfree(ctx_win_vec.items.cast());
        ctx_win_vec = KVec::new();

        while !firstwin.is_null() {
            let mut dummy: c_int = 0;
            win_free_mem(firstwin, &raw mut dummy, ptr::null_mut());
        }

        curwin = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn winframe_remove(
    win: *mut win_T,
    dirp: *mut c_int,
    tp: *mut tabpage_T,
    unflat_altfr: *mut *mut frame_T,
) -> *mut win_T {
    unsafe {
        let mut altfr: *mut frame_T = ptr::null_mut();
        let wp = winframe_find_altwin(win, dirp, tp, &raw mut altfr);
        if wp.is_null() {
            return ptr::null_mut();
        }

        let frp_close = (*win).w_frame;

        frame_locked += 1;

        let topleft = frame2win((*frp_close).fr_parent);
        let mut row = (*topleft).w_winrow;
        let mut col = (*topleft).w_wincol;

        if (*win).w_vsep_width == 0
            && (*(*frp_close).fr_parent).fr_layout == FR_ROW
            && !(*frp_close).fr_prev.is_null()
        {
            frame_set_vsep((*frp_close).fr_prev, false);
        }

        frame_remove(frp_close);

        if *dirp == b'v' as c_int {
            frame_new_height(
                altfr,
                (*altfr).fr_height + (*frp_close).fr_height,
                ptr::eq(altfr, (*frp_close).fr_next),
                !frame_fixed_height(altfr),
                false,
            );
        } else {
            debug_assert!(*dirp == b'h' as c_int);
            frame_new_width(
                altfr,
                (*altfr).fr_width + (*frp_close).fr_width,
                ptr::eq(altfr, (*frp_close).fr_next),
                !frame_fixed_height(altfr),
            );
        }

        if !ptr::eq(altfr, (*frp_close).fr_prev) {
            frame_comp_pos((*frp_close).fr_parent, &raw mut row, &raw mut col);
        }

        if unflat_altfr.is_null() {
            frame_flatten(altfr);
        } else {
            *unflat_altfr = altfr;
        }

        frame_locked -= 1;

        wp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn winframe_find_altwin(
    win: *mut win_T,
    dirp: *mut c_int,
    tp: *mut tabpage_T,
    altfr: *mut *mut frame_T,
) -> *mut win_T {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, curtab));

        if one_window(win, tp) {
            return ptr::null_mut();
        }

        let frp_close = (*win).w_frame;
        let mut frp2 = win_altframe(win, tp);
        let mut wp = frame2win(frp2);

        if (*(*frp_close).fr_parent).fr_layout == FR_COL {
            if !(*frp2).fr_win.is_null() && (*(*frp2).fr_win).w_onebuf_opt.wo_wfh != 0 {
                let mut frp = (*frp_close).fr_prev;
                let mut frp3 = (*frp_close).fr_next;
                while !frp.is_null() || !frp3.is_null() {
                    if !frp.is_null() {
                        if !frame_fixed_height(frp) {
                            frp2 = frp;
                            wp = frame2win(frp2);
                            break;
                        }
                        frp = (*frp).fr_prev;
                    }
                    if !frp3.is_null() {
                        if !(*frp3).fr_win.is_null() && (*(*frp3).fr_win).w_onebuf_opt.wo_wfh == 0 {
                            frp2 = frp3;
                            wp = (*frp3).fr_win;
                            break;
                        }
                        frp3 = (*frp3).fr_next;
                    }
                }
            }
            *dirp = b'v' as c_int;
        } else {
            if !(*frp2).fr_win.is_null() && (*(*frp2).fr_win).w_onebuf_opt.wo_wfw != 0 {
                let mut frp = (*frp_close).fr_prev;
                let mut frp3 = (*frp_close).fr_next;
                while !frp.is_null() || !frp3.is_null() {
                    if !frp.is_null() {
                        if !frame_fixed_width(frp) {
                            frp2 = frp;
                            wp = frame2win(frp2);
                            break;
                        }
                        frp = (*frp).fr_prev;
                    }
                    if !frp3.is_null() {
                        if !(*frp3).fr_win.is_null() && (*(*frp3).fr_win).w_onebuf_opt.wo_wfw == 0 {
                            frp2 = frp3;
                            wp = (*frp3).fr_win;
                            break;
                        }
                        frp3 = (*frp3).fr_next;
                    }
                }
            }
            *dirp = b'h' as c_int;
        }

        debug_assert!(!ptr::eq(wp, win) && !ptr::eq(frp2, frp_close));
        if !altfr.is_null() {
            *altfr = frp2;
        }

        wp
    }
}

unsafe fn frame_flatten(mut frp: *mut frame_T) {
    unsafe {
        if !(*frp).fr_next.is_null() || !(*frp).fr_prev.is_null() {
            return;
        }

        (*(*frp).fr_parent).fr_layout = (*frp).fr_layout;
        (*(*frp).fr_parent).fr_child = (*frp).fr_child;
        let mut frp2 = (*frp).fr_child;
        while !frp2.is_null() {
            (*frp2).fr_parent = (*frp).fr_parent;
            frp2 = (*frp2).fr_next;
        }
        (*(*frp).fr_parent).fr_win = (*frp).fr_win;
        if !(*frp).fr_win.is_null() {
            (*(*frp).fr_win).w_frame = (*frp).fr_parent;
        }
        frp2 = (*frp).fr_parent;
        if ptr::eq((*topframe).fr_child, frp) {
            (*topframe).fr_child = frp2;
        }
        xfree(frp.cast());

        frp = (*frp2).fr_parent;
        if !frp.is_null() && (*frp).fr_layout == (*frp2).fr_layout {
            if ptr::eq((*frp).fr_child, frp2) {
                (*frp).fr_child = (*frp2).fr_child;
            }
            debug_assert!(!(*frp2).fr_child.is_null());
            (*(*frp2).fr_child).fr_prev = (*frp2).fr_prev;
            if !(*frp2).fr_prev.is_null() {
                (*(*frp2).fr_prev).fr_next = (*frp2).fr_child;
            }
            let mut frp3 = (*frp2).fr_child;
            loop {
                (*frp3).fr_parent = frp;
                if (*frp3).fr_next.is_null() {
                    (*frp3).fr_next = (*frp2).fr_next;
                    if !(*frp2).fr_next.is_null() {
                        (*(*frp2).fr_next).fr_prev = frp3;
                    }
                    break;
                }
                frp3 = (*frp3).fr_next;
            }
            if ptr::eq((*topframe).fr_child, frp2) {
                (*topframe).fr_child = frp;
            }
            xfree(frp2.cast());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn winframe_restore(wp: *mut win_T, dir: c_int, unflat_altfr: *mut frame_T) {
    unsafe {
        let frp = (*wp).w_frame;

        if !(*frp).fr_prev.is_null() {
            frame_append((*frp).fr_prev, frp);
        } else {
            frame_insert((*frp).fr_next, frp);
        }

        if (*wp).w_vsep_width == 0
            && (*(*frp).fr_parent).fr_layout == FR_ROW
            && !(*frp).fr_prev.is_null()
        {
            frame_set_vsep((*frp).fr_prev, true);
        }

        if (*(*frp).fr_parent).fr_layout == FR_COL && !(*frp).fr_prev.is_null() {
            if global_stl_height() == 0 && (*wp).w_status_height == 0 {
                frame_add_statusline((*frp).fr_prev);
            } else if global_stl_height() > 0 && (*wp).w_hsep_height == 0 {
                frame_add_hsep((*frp).fr_prev);
            }
        }

        if dir == b'v' as c_int {
            frame_new_height(
                unflat_altfr,
                (*unflat_altfr).fr_height - (*frp).fr_height,
                ptr::eq(unflat_altfr, (*frp).fr_next),
                false,
                false,
            );
        } else if dir == b'h' as c_int {
            frame_new_width(
                unflat_altfr,
                (*unflat_altfr).fr_width - (*frp).fr_width,
                ptr::eq(unflat_altfr, (*frp).fr_next),
                false,
            );
        }

        if !ptr::eq(unflat_altfr, (*frp).fr_prev) {
            let topleft = frame2win((*frp).fr_parent);
            let mut row = (*topleft).w_winrow;
            let mut col = (*topleft).w_wincol;

            frame_comp_pos((*frp).fr_parent, &raw mut row, &raw mut col);
        }
    }
}

unsafe fn win_altframe(win: *mut win_T, tp: *mut tabpage_T) -> *mut frame_T {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, curtab));

        if one_window(win, tp) {
            return (*(*alt_tabpage()).tp_curwin).w_frame;
        }

        let frp = (*win).w_frame;

        if (*frp).fr_prev.is_null() {
            return (*frp).fr_next;
        }
        if (*frp).fr_next.is_null() {
            return (*frp).fr_prev;
        }

        let mut target_fr = (*frp).fr_next;
        let mut other_fr = (*frp).fr_prev;

        if !(*frp).fr_parent.is_null() && (*(*frp).fr_parent).fr_layout == FR_COL && p_sb != 0 {
            target_fr = (*frp).fr_prev;
            other_fr = (*frp).fr_next;
        }

        if !(*frp).fr_parent.is_null() && (*(*frp).fr_parent).fr_layout == FR_ROW && p_spr != 0 {
            target_fr = (*frp).fr_prev;
            other_fr = (*frp).fr_next;
        }

        if !(*frp).fr_parent.is_null() && (*(*frp).fr_parent).fr_layout == FR_ROW {
            if frame_fixed_width(target_fr) && !frame_fixed_width(other_fr) {
                target_fr = other_fr;
            }
        } else if frame_fixed_height(target_fr) && !frame_fixed_height(other_fr) {
            target_fr = other_fr;
        }

        target_fr
    }
}

unsafe fn alt_tabpage() -> *mut tabpage_T {
    unsafe {
        if (tcl_flags & kOptTclFlagUselast) != 0 && valid_tabpage(lastused_tabpage) {
            return lastused_tabpage;
        }

        let forward = !(*curtab).tp_next.is_null()
            && ((tcl_flags & kOptTclFlagLeft) == 0 || ptr::eq(curtab, first_tabpage));

        if forward {
            (*curtab).tp_next
        } else {
            let mut tp = first_tabpage;
            while !ptr::eq((*tp).tp_next, curtab) {
                tp = (*tp).tp_next;
            }
            tp
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn frame2win(mut frp: *mut frame_T) -> *mut win_T {
    unsafe {
        while (*frp).fr_win.is_null() {
            frp = (*frp).fr_child;
        }
        (*frp).fr_win
    }
}

unsafe fn frame_has_win(frp: *const frame_T, wp: *const win_T) -> bool {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            return ptr::eq((*frp).fr_win, wp);
        }
        let mut p: *const frame_T = (*frp).fr_child;
        while !p.is_null() {
            if frame_has_win(p, wp) {
                return true;
            }
            p = (*p).fr_next;
        }
        false
    }
}

unsafe fn is_bottom_win(wp: *mut win_T) -> bool {
    unsafe {
        let mut frp = (*wp).w_frame;
        while !(*frp).fr_parent.is_null() {
            if (*(*frp).fr_parent).fr_layout == FR_COL && !(*frp).fr_next.is_null() {
                return false;
            }
            frp = (*frp).fr_parent;
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn frame_new_height(
    topfrp: *mut frame_T,
    mut height: c_int,
    topfirst: bool,
    wfh: bool,
    set_ch: bool,
) {
    unsafe {
        if (*topfrp).fr_parent.is_null() && set_ch {
            let new_ch = min_set_ch.max(p_ch + OptInt::from((*topfrp).fr_height - height));
            if new_ch != p_ch {
                let save_ch = min_set_ch;
                set_option_value(kOptCmdheight, integer_obj(new_ch), 0, true, ptr::null());
                min_set_ch = save_ch;
            }
            height = (rows_avail() as c_int).min(height);
        }
        if !(*topfrp).fr_win.is_null() {
            let wp = (*topfrp).fr_win;
            if is_bottom_win(wp) {
                (*wp).w_hsep_height = 0;
            }
            win_new_height(wp, height - (*wp).w_hsep_height - (*wp).w_status_height);
        } else if (*topfrp).fr_layout == FR_ROW {
            let mut frp: *mut frame_T;
            loop {
                frp = (*topfrp).fr_child;
                while !frp.is_null() {
                    frame_new_height(frp, height, topfirst, wfh, set_ch);
                    if (*frp).fr_height > height {
                        height = (*frp).fr_height;
                        break;
                    }
                    frp = (*frp).fr_next;
                }
                if frp.is_null() {
                    break;
                }
            }
        } else {
            let mut frp = (*topfrp).fr_child;
            if wfh {
                while frame_fixed_height(frp) {
                    frp = (*frp).fr_next;
                    if frp.is_null() {
                        return;
                    }
                }
            }
            if !topfirst {
                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                if wfh {
                    while frame_fixed_height(frp) {
                        frp = (*frp).fr_prev;
                    }
                }
            }

            let mut extra_lines = height - (*topfrp).fr_height;
            if extra_lines < 0 {
                while !frp.is_null() {
                    let h = frame_minheight(frp, ptr::null_mut());
                    if (*frp).fr_height + extra_lines < h {
                        extra_lines += (*frp).fr_height - h;
                        frame_new_height(frp, h, topfirst, wfh, set_ch);
                    } else {
                        frame_new_height(
                            frp,
                            (*frp).fr_height + extra_lines,
                            topfirst,
                            wfh,
                            set_ch,
                        );
                        break;
                    }
                    if topfirst {
                        loop {
                            frp = (*frp).fr_next;
                            if !(wfh && !frp.is_null() && frame_fixed_height(frp)) {
                                break;
                            }
                        }
                    } else {
                        loop {
                            frp = (*frp).fr_prev;
                            if !(wfh && !frp.is_null() && frame_fixed_height(frp)) {
                                break;
                            }
                        }
                    }
                    if frp.is_null() {
                        height -= extra_lines;
                    }
                }
            } else if extra_lines > 0 {
                frame_new_height(frp, (*frp).fr_height + extra_lines, topfirst, wfh, set_ch);
            }
        }
        (*topfrp).fr_height = height;
    }
}

unsafe fn frame_fixed_height(mut frp: *mut frame_T) -> bool {
    unsafe {
        if !(*frp).fr_win.is_null() {
            return (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0;
        }
        if (*frp).fr_layout == FR_ROW {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                if frame_fixed_height(frp) {
                    return true;
                }
                frp = (*frp).fr_next;
            }
            return false;
        }

        frp = (*frp).fr_child;
        while !frp.is_null() {
            if !frame_fixed_height(frp) {
                return false;
            }
            frp = (*frp).fr_next;
        }
        true
    }
}

unsafe fn frame_fixed_width(mut frp: *mut frame_T) -> bool {
    unsafe {
        if !(*frp).fr_win.is_null() {
            return (*(*frp).fr_win).w_onebuf_opt.wo_wfw != 0;
        }
        if (*frp).fr_layout == FR_COL {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                if frame_fixed_width(frp) {
                    return true;
                }
                frp = (*frp).fr_next;
            }
            return false;
        }

        frp = (*frp).fr_child;
        while !frp.is_null() {
            if !frame_fixed_width(frp) {
                return false;
            }
            frp = (*frp).fr_next;
        }
        true
    }
}

unsafe fn frame_add_statusline(mut frp: *mut frame_T) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            let wp = (*frp).fr_win;
            (*wp).w_status_height = STATUS_HEIGHT;
        } else if (*frp).fr_layout == FR_ROW {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                frame_add_statusline(frp);
                frp = (*frp).fr_next;
            }
        } else {
            debug_assert!((*frp).fr_layout == FR_COL);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            frame_add_statusline(frp);
        }
    }
}

unsafe fn frame_new_width(topfrp: *mut frame_T, mut width: c_int, leftfirst: bool, wfw: bool) {
    unsafe {
        if (*topfrp).fr_layout == FR_LEAF {
            let wp = (*topfrp).fr_win;
            let mut frp = topfrp;
            while !(*frp).fr_parent.is_null() {
                if (*(*frp).fr_parent).fr_layout == FR_ROW && !(*frp).fr_next.is_null() {
                    break;
                }
                frp = (*frp).fr_parent;
            }
            if (*frp).fr_parent.is_null() {
                (*wp).w_vsep_width = 0;
            }
            win_new_width(wp, width - (*wp).w_vsep_width);
        } else if (*topfrp).fr_layout == FR_COL {
            let mut frp: *mut frame_T;
            loop {
                frp = (*topfrp).fr_child;
                while !frp.is_null() {
                    frame_new_width(frp, width, leftfirst, wfw);
                    if (*frp).fr_width > width {
                        width = (*frp).fr_width;
                        break;
                    }
                    frp = (*frp).fr_next;
                }
                if frp.is_null() {
                    break;
                }
            }
        } else {
            let mut frp = (*topfrp).fr_child;
            if wfw {
                while frame_fixed_width(frp) {
                    frp = (*frp).fr_next;
                    if frp.is_null() {
                        return;
                    }
                }
            }
            if !leftfirst {
                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                if wfw {
                    while frame_fixed_width(frp) {
                        frp = (*frp).fr_prev;
                    }
                }
            }

            let mut extra_cols = width - (*topfrp).fr_width;
            if extra_cols < 0 {
                while !frp.is_null() {
                    let w = frame_minwidth(frp, ptr::null_mut());
                    if (*frp).fr_width + extra_cols < w {
                        extra_cols += (*frp).fr_width - w;
                        frame_new_width(frp, w, leftfirst, wfw);
                    } else {
                        frame_new_width(frp, (*frp).fr_width + extra_cols, leftfirst, wfw);
                        break;
                    }
                    if leftfirst {
                        loop {
                            frp = (*frp).fr_next;
                            if !(wfw && !frp.is_null() && frame_fixed_width(frp)) {
                                break;
                            }
                        }
                    } else {
                        loop {
                            frp = (*frp).fr_prev;
                            if !(wfw && !frp.is_null() && frame_fixed_width(frp)) {
                                break;
                            }
                        }
                    }
                    if frp.is_null() {
                        width -= extra_cols;
                    }
                }
            } else if extra_cols > 0 {
                frame_new_width(frp, (*frp).fr_width + extra_cols, leftfirst, wfw);
            }
        }
        (*topfrp).fr_width = width;
    }
}

unsafe fn frame_set_vsep(mut frp: *const frame_T, add: bool) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            let wp = (*frp).fr_win;
            if add && (*wp).w_vsep_width == 0 {
                if (*wp).w_width > 0 {
                    win_new_width(wp, (*wp).w_width - 1);
                }
                (*wp).w_vsep_width = 1;
            } else if !add && (*wp).w_vsep_width == 1 {
                win_new_width(wp, (*wp).w_width + 1);
                (*wp).w_vsep_width = 0;
            }
        } else if (*frp).fr_layout == FR_COL {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                frame_set_vsep(frp, add);
                frp = (*frp).fr_next;
            }
        } else {
            debug_assert!((*frp).fr_layout == FR_ROW);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            frame_set_vsep(frp, add);
        }
    }
}

unsafe fn frame_add_hsep(mut frp: *const frame_T) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            let wp = (*frp).fr_win;
            (*wp).w_hsep_height = 1;
        } else if (*frp).fr_layout == FR_ROW {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                frame_add_hsep(frp);
                frp = (*frp).fr_next;
            }
        } else {
            debug_assert!((*frp).fr_layout == FR_COL);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            frame_add_hsep(frp);
        }
    }
}

#[inline]
unsafe fn frame_fix_width(wp: *mut win_T) {
    unsafe {
        (*(*wp).w_frame).fr_width = (*wp).w_width + (*wp).w_vsep_width;
    }
}

#[inline]
unsafe fn frame_fix_height(wp: *mut win_T) {
    unsafe {
        (*(*wp).w_frame).fr_height = (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height;
    }
}

unsafe fn frame_minheight(topfrp: *mut frame_T, next_curwin: *mut win_T) -> c_int {
    unsafe {
        let mut m: c_int;
        if !(*topfrp).fr_win.is_null() {
            let extra_height = (*(*topfrp).fr_win).w_winbar_height
                + (*(*topfrp).fr_win).w_hsep_height
                + (*(*topfrp).fr_win).w_status_height;
            if ptr::eq((*topfrp).fr_win, next_curwin) {
                m = p_wh as c_int + extra_height;
            } else {
                m = p_wmh as c_int + extra_height;
                if ptr::eq((*topfrp).fr_win, curwin) && next_curwin.is_null() && p_wmh == 0 {
                    m += 1;
                }
            }
        } else if (*topfrp).fr_layout == FR_ROW {
            m = 0;
            let mut frp = (*topfrp).fr_child;
            while !frp.is_null() {
                let n = frame_minheight(frp, next_curwin);
                if n > m {
                    m = n;
                }
                frp = (*frp).fr_next;
            }
        } else {
            m = 0;
            let mut frp = (*topfrp).fr_child;
            while !frp.is_null() {
                m += frame_minheight(frp, next_curwin);
                frp = (*frp).fr_next;
            }
        }
        m
    }
}

unsafe fn frame_minwidth(topfrp: *mut frame_T, next_curwin: *mut win_T) -> c_int {
    unsafe {
        let mut m: c_int;
        if !(*topfrp).fr_win.is_null() {
            if ptr::eq((*topfrp).fr_win, next_curwin) {
                m = p_wiw as c_int + (*(*topfrp).fr_win).w_vsep_width;
            } else {
                m = p_wmw as c_int + (*(*topfrp).fr_win).w_vsep_width;
                if p_wmw == 0 && ptr::eq((*topfrp).fr_win, curwin) && next_curwin.is_null() {
                    m += 1;
                }
            }
        } else if (*topfrp).fr_layout == FR_COL {
            m = 0;
            let mut frp = (*topfrp).fr_child;
            while !frp.is_null() {
                let n = frame_minwidth(frp, next_curwin);
                m = m.max(n);
                frp = (*frp).fr_next;
            }
        } else {
            m = 0;
            let mut frp = (*topfrp).fr_child;
            while !frp.is_null() {
                m += frame_minwidth(frp, next_curwin);
                frp = (*frp).fr_next;
            }
        }
        m
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn close_others(message: c_int, forceit: c_int, ignore_pinned: bool) {
    unsafe {
        let old_curwin = curwin;

        if (*curwin).w_floating {
            if message != 0 && !autocmd_busy {
                emsg(e_floatonly.as_ptr());
            }
            return;
        }

        if one_window(firstwin, ptr::null_mut()) && !(*lastwin).w_floating {
            if message != 0 && !autocmd_busy {
                msg(gettext(M_ONLYONE.as_ptr()), 0);
            }
            return;
        }

        let mut nextwp: *mut win_T;
        let mut wp = firstwin;
        while win_valid(wp) {
            nextwp = (*wp).w_next;

            if !ptr::eq(old_curwin, curwin) && win_valid(old_curwin) {
                curwin = old_curwin;
                curbuf = (*curwin).w_buffer;
            }

            if ptr::eq(wp, curwin) || ((*wp).w_onebuf_opt.wo_wp != 0 && !ignore_pinned) {
                wp = nextwp;
                continue;
            }

            if !buf_valid((*wp).w_buffer) && win_valid(wp) {
                (*wp).w_buffer = ptr::null_mut();
                win_close(wp, false, false);
                wp = nextwp;
                continue;
            }

            let r = can_abandon((*wp).w_buffer, forceit);
            if !win_valid(wp) {
                wp = firstwin;
                continue;
            }
            if !r {
                if message != 0
                    && (p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                    && p_write != 0
                {
                    dialog_changed((*wp).w_buffer, false);
                    if !win_valid(wp) {
                        wp = firstwin;
                        continue;
                    }
                }
                if bufIsChanged((*wp).w_buffer) {
                    wp = nextwp;
                    continue;
                }
            }
            win_close(
                wp,
                !buf_hide((*wp).w_buffer) && !bufIsChanged((*wp).w_buffer),
                false,
            );
            wp = nextwp;
        }

        if message != 0 && !one_window_macro() {
            let mut has_non_pinned = false;
            let mut wp2 = firstwin;
            while !wp2.is_null() {
                if !ptr::eq(wp2, curwin) && (*wp2).w_onebuf_opt.wo_wp == 0 {
                    has_non_pinned = true;
                    break;
                }
                wp2 = (*wp2).w_next;
            }
            if has_non_pinned {
                emsg(gettext(c"E445: Other window contains changes".as_ptr()));
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn unuse_tabpage(tp: *mut tabpage_T) {
    unsafe {
        (*tp).tp_topframe = topframe.cast();
        (*tp).tp_firstwin = firstwin;
        (*tp).tp_lastwin = lastwin;
        (*tp).tp_curwin = curwin;
        (*tp).tp_ch_used = p_ch;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn use_tabpage(tp: *mut tabpage_T) {
    unsafe {
        curtab = tp;
        topframe = (*curtab).tp_topframe.cast();
        firstwin = (*curtab).tp_firstwin;
        lastwin = (*curtab).tp_lastwin;
        curwin = (*curtab).tp_curwin;
        p_ch = (*curtab).tp_ch_used;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_alloc_first() {
    unsafe {
        if win_alloc_firstwin(ptr::null_mut()) == FAIL {
            std::process::abort();
        }

        first_tabpage = alloc_tabpage();
        curtab = first_tabpage;
        unuse_tabpage(first_tabpage);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_alloc_ctx_win(idx: c_int) {
    unsafe {
        let mut err = ERROR_INIT;
        let mut fconfig = WIN_CONFIG_INIT;
        fconfig.width = Columns;
        fconfig.height = 5;
        fconfig.focusable = false;
        fconfig.mouse = false;
        fconfig.hide = true;
        let cw_win = win_new_float(ptr::null_mut(), true, fconfig, &raw mut err);
        (*ctx_win_vec.items.add(idx as usize)).cw_win = cw_win;
        (*(*cw_win).w_buffer).b_nwindows -= 1;
        reset_binding(cw_win);
    }
}

unsafe fn win_alloc_firstwin(oldwin: *mut win_T) -> c_int {
    unsafe {
        curwin = win_alloc(ptr::null_mut(), false);
        if oldwin.is_null() {
            curbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_LISTED);
            if curbuf.is_null() {
                return FAIL;
            }
            (*curwin).w_buffer = curbuf;
            (*curwin).w_s = (&raw mut (*curbuf).b_s).cast();
            (*curbuf).b_nwindows = 1;
            (*curwin).w_alist = (&raw mut global_alist).cast();
            curwin_init();
        } else {
            win_init(curwin, oldwin, 0);
            reset_binding(curwin);
        }

        new_frame(curwin);
        topframe = (*curwin).w_frame;
        (*topframe).fr_width = Columns;
        (*topframe).fr_height = Rows - p_ch as c_int - global_stl_height();

        OK
    }
}

unsafe fn new_frame(wp: *mut win_T) {
    unsafe {
        let frp = xcalloc(1, size_of::<frame_T>()).cast::<frame_T>();
        (*wp).w_frame = frp;
        (*frp).fr_layout = FR_LEAF;
        (*frp).fr_win = wp;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_init_size() {
    unsafe {
        let avail = rows_avail() as c_int;
        (*firstwin).w_height = avail;
        (*firstwin).w_prev_height = avail;
        (*firstwin).w_view_height = (*firstwin).w_height - (*firstwin).w_winbar_height;
        (*firstwin).w_height_outer = (*firstwin).w_height;
        (*firstwin).w_winrow_off = (*firstwin).w_winbar_height;
        (*topframe).fr_height = avail;
        (*firstwin).w_width = Columns;
        (*firstwin).w_view_width = (*firstwin).w_width;
        (*firstwin).w_width_outer = (*firstwin).w_width;
        (*topframe).fr_width = Columns;
    }
}

unsafe fn alloc_tabpage() -> *mut tabpage_T {
    unsafe {
        let tp = xcalloc(1, size_of::<tabpage_T>()).cast::<tabpage_T>();
        last_tp_handle += 1;
        (*tp).handle = last_tp_handle;
        *map_put_ref_int_ptr_t(
            &raw mut tabpage_handles,
            (*tp).handle,
            ptr::null_mut(),
            ptr::null_mut(),
        ) = tp.cast();

        (*tp).tp_vars = tv_dict_alloc();
        init_var_dict((*tp).tp_vars, (&raw mut (*tp).tp_winvar).cast(), VAR_SCOPE);
        (*tp).tp_diff_invalid = 1;
        (*tp).tp_ch_used = p_ch;

        tp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_tabpage(tp: *mut tabpage_T) {
    unsafe {
        map_del_int_ptr_t(&raw mut tabpage_handles, (*tp).handle, ptr::null_mut());
        diff_clear(tp);
        for idx in 0..SNAP_COUNT {
            clear_snapshot(tp, idx as c_int);
        }
        vars_clear(&raw mut (*(*tp).tp_vars).dv_hashtab);
        hash_init(&raw mut (*(*tp).tp_vars).dv_hashtab);
        unref_var_dict((*tp).tp_vars);

        if ptr::eq(tp, lastused_tabpage) {
            lastused_tabpage = ptr::null_mut();
        }

        xfree((*tp).tp_localdir.cast());
        xfree((*tp).tp_prevdir.cast());
        xfree(tp.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_new_tabpage(
    after: c_int,
    filename: *mut c_char,
    enter: bool,
    first: *mut *mut win_T,
) -> *mut tabpage_T {
    unsafe {
        let old_curtab = curtab;

        if window_layout_locked(cmdidx_T::CMD_tabnew) {
            return ptr::null_mut();
        }

        let newtp = alloc_tabpage();

        if enter {
            if leave_tabpage(curbuf, true) == FAIL {
                xfree(newtp.cast());
                return ptr::null_mut();
            }
        } else {
            unuse_tabpage(curtab);
            (*curtab).tp_old_Rows_avail = rows_avail();
            firstwin = ptr::null_mut();
            lastwin = ptr::null_mut();
        }

        (*newtp).tp_localdir = if !(*old_curtab).tp_localdir.is_null() {
            xstrdup((*old_curtab).tp_localdir)
        } else {
            ptr::null_mut()
        };

        curtab = newtp;

        let result = win_alloc_firstwin((*old_curtab).tp_curwin);
        debug_assert_eq!(result, OK);
        if !first.is_null() {
            *first = curwin;
        }

        if after == 1 {
            (*newtp).tp_next = first_tabpage;
            first_tabpage = newtp;
        } else {
            let mut tp = old_curtab;
            if after > 0 {
                let mut n: c_int = 2;
                tp = first_tabpage;
                while !(*tp).tp_next.is_null() && n < after {
                    n += 1;
                    tp = (*tp).tp_next;
                }
            }
            (*newtp).tp_next = (*tp).tp_next;
            (*tp).tp_next = newtp;
        }
        (*newtp).tp_firstwin = curwin;
        (*newtp).tp_lastwin = curwin;
        (*newtp).tp_curwin = curwin;

        win_init_size();
        (*firstwin).w_winrow = tabline_height();
        (*firstwin).w_prev_winrow = (*firstwin).w_winrow;
        win_comp_scroll(curwin);

        (*newtp).tp_topframe = topframe.cast();
        last_status(false);

        if !(*curbuf).terminal.is_null() {
            terminal_check_size((*curbuf).terminal.cast::<Terminal>());
        }

        if enter {
            redraw_all_later(UPD_NOT_VALID);
            tabpage_check_windows(old_curtab);
            lastused_tabpage = old_curtab;
            entering_window(curwin);

            apply_autocmds(EVENT_WINNEW, ptr::null(), ptr::null(), false, curbuf);
            apply_autocmds(EVENT_WINENTER, ptr::null(), ptr::null(), false, curbuf);
            apply_autocmds(EVENT_TABNEW, filename, filename, false, curbuf);
            apply_autocmds(EVENT_TABENTER, ptr::null(), ptr::null(), false, curbuf);
        } else {
            unuse_tabpage(curtab);
            use_tabpage(old_curtab);
            redraw_tabline = true;
            if (*curtab).tp_old_Rows_avail != rows_avail() {
                win_new_screen_rows();
            }

            let mut switchwin: CtxSwitch = zeroed();
            let sw_ok = ctx_switch(
                &raw mut switchwin,
                (*newtp).tp_curwin,
                newtp,
                ptr::null_mut(),
                kCtxNoDisplay,
            );
            debug_assert!(sw_ok);

            apply_autocmds(EVENT_WINNEW, ptr::null(), ptr::null(), false, curbuf);
            apply_autocmds(EVENT_TABNEW, filename, filename, false, curbuf);

            ctx_restore(&raw mut switchwin);
        }

        newtp
    }
}

unsafe fn may_open_tabpage() -> c_int {
    unsafe {
        let n = if cmdmod.cmod_tab == 0 {
            postponed_split_tab
        } else {
            cmdmod.cmod_tab
        };

        if n == 0 {
            return FAIL;
        }

        cmdmod.cmod_tab = 0;
        postponed_split_tab = 0;
        let status = if !win_new_tabpage(n, ptr::null_mut(), true, ptr::null_mut()).is_null() {
            OK
        } else {
            FAIL
        };
        if status == OK {
            apply_autocmds(EVENT_TABNEWENTERED, ptr::null(), ptr::null(), false, curbuf);
        }
        status
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn make_tabpages(maxcount: c_int) -> c_int {
    unsafe {
        let count = maxcount.min(p_tpm as c_int);

        block_autocmds();

        let mut todo = count - 1;
        while todo > 0 {
            if win_new_tabpage(0, ptr::null_mut(), true, ptr::null_mut()).is_null() {
                break;
            }
            todo -= 1;
        }

        unblock_autocmds();

        count - todo
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_tabpage(tpc: *mut tabpage_T) -> bool {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            if ptr::eq(tp, tpc) {
                return true;
            }
            tp = (*tp).tp_next;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_tabpage_win(tpc: *mut tabpage_T) -> c_int {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            if ptr::eq(tp, tpc) {
                let mut wp = if ptr::eq(tp, curtab) {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if win_valid_any_tab(wp) {
                        return 1;
                    }
                    wp = (*wp).w_next;
                }
                return 0;
            }
            tp = (*tp).tp_next;
        }
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn close_tabpage(tab: *mut tabpage_T) {
    unsafe {
        let mut ptp: *mut tabpage_T;
        if ptr::eq(tab, first_tabpage) {
            first_tabpage = (*tab).tp_next;
            ptp = first_tabpage;
        } else {
            ptp = first_tabpage;
            while !ptp.is_null() && !ptr::eq((*ptp).tp_next, tab) {
                ptp = (*ptp).tp_next;
            }
            debug_assert!(!ptp.is_null());
            (*ptp).tp_next = (*tab).tp_next;
        }

        goto_tabpage_tp(ptp, false, false);
        free_tabpage(tab);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_tabpage(n: c_int) -> *mut tabpage_T {
    unsafe {
        if n == 0 {
            return curtab;
        }

        let mut i: c_int = 1;
        let mut tp = first_tabpage;
        while !tp.is_null() && i != n {
            i += 1;
            tp = (*tp).tp_next;
        }
        tp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tabpage_index(ftp: *mut tabpage_T) -> c_int {
    unsafe {
        let mut i: c_int = 1;
        let mut tp = first_tabpage;
        while !tp.is_null() && !ptr::eq(tp, ftp) {
            i += 1;
            tp = (*tp).tp_next;
        }
        i
    }
}

unsafe fn leave_tabpage(new_curbuf: *mut buf_T, trigger_leave_autocmds: bool) -> c_int {
    unsafe {
        let tp = curtab;

        leaving_window(curwin);
        reset_VIsual_and_resel();
        if trigger_leave_autocmds {
            if !ptr::eq(new_curbuf, curbuf) {
                apply_autocmds(EVENT_BUFLEAVE, ptr::null(), ptr::null(), false, curbuf);
                if !ptr::eq(curtab, tp) {
                    return FAIL;
                }
            }
            apply_autocmds(EVENT_WINLEAVE, ptr::null(), ptr::null(), false, curbuf);
            if !ptr::eq(curtab, tp) {
                return FAIL;
            }
            apply_autocmds(EVENT_TABLEAVE, ptr::null(), ptr::null(), false, curbuf);
            if !ptr::eq(curtab, tp) {
                return FAIL;
            }
        }

        reset_dragwin();
        (*tp).tp_curwin = curwin;
        (*tp).tp_prevwin = prevwin;
        (*tp).tp_firstwin = firstwin;
        (*tp).tp_lastwin = lastwin;
        (*tp).tp_old_Rows_avail = rows_avail();
        if (*tp).tp_old_Columns != -1 {
            (*tp).tp_old_Columns = i64::from(Columns);
        }
        firstwin = ptr::null_mut();
        lastwin = ptr::null_mut();
        OK
    }
}

unsafe fn enter_tabpage(
    tp: *mut tabpage_T,
    old_curbuf: *mut buf_T,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    unsafe {
        let old_off = (*(*tp).tp_firstwin).w_winrow;
        let next_prevwin = (*tp).tp_prevwin;
        let old_curtab = curtab;
        let prev_p_ch = p_ch;

        use_tabpage(tp);

        if !ptr::eq(old_curtab, curtab) && p_ch != prev_p_ch {
            tabpage_check_windows(old_curtab);
            let new_ch = p_ch;
            p_ch = prev_p_ch;
            command_frame_height = false;
            set_option_value(kOptCmdheight, integer_obj(new_ch), 0, true, ptr::null());
            command_frame_height = true;
        } else if !ptr::eq(old_curtab, curtab) {
            tabpage_check_windows(old_curtab);
        }

        win_enter_ext(
            (*tp).tp_curwin,
            WEE_CURWIN_INVALID
                | (if trigger_enter_autocmds {
                    WEE_TRIGGER_ENTER_AUTOCMDS
                } else {
                    0
                })
                | (if trigger_leave_autocmds {
                    WEE_TRIGGER_LEAVE_AUTOCMDS
                } else {
                    0
                }),
        );
        prevwin = next_prevwin;

        last_status(false);
        win_float_update_statusline();
        win_comp_pos();
        diff_need_scrollbind = true;

        reset_dragwin();

        if (*curtab).tp_old_Rows_avail != rows_avail() || old_off != (*firstwin).w_winrow {
            win_new_screen_rows();
        }
        if (*curtab).tp_old_Columns != i64::from(Columns) {
            if starting == 0 {
                win_new_screen_cols();
                (*curtab).tp_old_Columns = i64::from(Columns);
            } else {
                (*curtab).tp_old_Columns = -1;
            }
        }

        lastused_tabpage = old_curtab;

        if trigger_enter_autocmds {
            apply_autocmds(EVENT_TABENTER, ptr::null(), ptr::null(), false, curbuf);
            if !ptr::eq(old_curbuf, curbuf) {
                apply_autocmds(EVENT_BUFENTER, ptr::null(), ptr::null(), false, curbuf);
            }
        }

        redraw_all_later(UPD_NOT_VALID);
    }
}

unsafe fn tabpage_check_windows(old_curtab: *mut tabpage_T) {
    unsafe {
        let mut wp = (*old_curtab).tp_firstwin;
        while !wp.is_null() {
            let next_wp = (*wp).w_next;
            if (*wp).w_floating {
                if (*wp).w_config.external {
                    win_remove(wp, old_curtab);
                    win_append(lastwin_nofloating(ptr::null_mut()), wp, ptr::null_mut());
                } else {
                    ui_comp_remove_grid(&raw mut (*wp).w_grid_alloc);
                }
            }
            (*wp).w_pos_changed = true;
            wp = next_wp;
        }

        let mut wp2 = firstwin;
        while !wp2.is_null() {
            if (*wp2).w_floating && !(*wp2).w_config.external {
                win_config_float(wp2, (*wp2).w_config);
            }
            (*wp2).w_pos_changed = true;
            wp2 = (*wp2).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn goto_tabpage(n: c_int) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }

        if (*first_tabpage).tp_next.is_null() {
            if n > 1 {
                beep_flush();
            }
            return;
        }

        let mut tp: *mut tabpage_T = ptr::null_mut();

        if n == 0 {
            if (*curtab).tp_next.is_null() {
                tp = first_tabpage;
            } else {
                tp = (*curtab).tp_next;
            }
        } else if n < 0 {
            let mut ttp = curtab;
            for _ in n..0 {
                tp = first_tabpage;
                while !ptr::eq((*tp).tp_next, ttp) && !(*tp).tp_next.is_null() {
                    tp = (*tp).tp_next;
                }
                ttp = tp;
            }
        } else if n == 9999 {
            tp = first_tabpage;
            while !(*tp).tp_next.is_null() {
                tp = (*tp).tp_next;
            }
        } else {
            tp = find_tabpage(n);
            if tp.is_null() {
                beep_flush();
                return;
            }
        }

        goto_tabpage_tp(tp, true, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn goto_tabpage_tp(
    tp: *mut tabpage_T,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    unsafe {
        set_keep_msg(ptr::null(), 0);

        skip_win_fix_scroll = true;
        if !ptr::eq(tp, curtab)
            && leave_tabpage((*(*tp).tp_curwin).w_buffer, trigger_leave_autocmds) == OK
        {
            if valid_tabpage(tp) {
                enter_tabpage(tp, curbuf, trigger_enter_autocmds, trigger_leave_autocmds);
            } else {
                enter_tabpage(
                    curtab,
                    curbuf,
                    trigger_enter_autocmds,
                    trigger_leave_autocmds,
                );
            }
        }
        skip_win_fix_scroll = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn goto_tabpage_lastused() -> bool {
    unsafe {
        if !valid_tabpage(lastused_tabpage) {
            return false;
        }

        goto_tabpage_tp(lastused_tabpage, true, true);
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn goto_tabpage_win(tp: *mut tabpage_T, wp: *mut win_T) {
    unsafe {
        goto_tabpage_tp(tp, true, true);
        if ptr::eq(curtab, tp) && win_valid(wp) {
            win_enter(wp, true);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tabpage_move(nr: c_int) {
    unsafe {
        debug_assert!(!curtab.is_null());

        if (*first_tabpage).tp_next.is_null() {
            return;
        }

        if tabpage_move_disallowed != 0 {
            return;
        }

        let mut n: c_int = 1;
        let mut tp = first_tabpage;
        while !(*tp).tp_next.is_null() && n < nr {
            n += 1;
            tp = (*tp).tp_next;
        }

        if ptr::eq(tp, curtab)
            || (nr > 0 && !(*tp).tp_next.is_null() && ptr::eq((*tp).tp_next, curtab))
        {
            return;
        }

        let old_nr = tabpage_index(curtab);
        let tp_dst = tp;

        if ptr::eq(curtab, first_tabpage) {
            first_tabpage = (*curtab).tp_next;
        } else {
            tp = ptr::null_mut();
            let mut tp2 = first_tabpage;
            while !tp2.is_null() {
                if ptr::eq((*tp2).tp_next, curtab) {
                    tp = tp2;
                    break;
                }
                tp2 = (*tp2).tp_next;
            }
            if tp.is_null() {
                return;
            }
            (*tp).tp_next = (*curtab).tp_next;
        }

        if nr <= 0 {
            (*curtab).tp_next = first_tabpage;
            first_tabpage = curtab;
        } else {
            (*curtab).tp_next = (*tp_dst).tp_next;
            (*tp_dst).tp_next = curtab;
        }

        redraw_tabline = true;

        if has_event(EVENT_TABMOVED) {
            let mut prev_idx = [0 as c_char; NUMBUFLEN];
            vim_snprintf(prev_idx.as_mut_ptr(), NUMBUFLEN, c"%i".as_ptr(), old_nr);
            let mut data_items = [
                KeyValuePair {
                    key: cstr_as_string(c"tabnr_old".as_ptr()),
                    value: integer_obj(Integer::from(old_nr)),
                },
                KeyValuePair {
                    key: cstr_as_string(c"tabnr_new".as_ptr()),
                    value: integer_obj(Integer::from(tabpage_index(curtab))),
                },
            ];
            let data = Dict {
                items: data_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            let mut obj = dict_obj(data);
            aucmd_defer(
                EVENT_TABMOVED,
                prev_idx.as_mut_ptr(),
                ptr::null_mut(),
                AUGROUP_ALL,
                curbuf,
                ptr::null_mut(),
                &raw mut obj,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_goto(wp: *mut win_T) {
    unsafe {
        let owp = curwin;

        if text_or_buf_locked() {
            beep_flush();
            return;
        }

        if !ptr::eq((*wp).w_buffer, curbuf) {
            reset_VIsual_and_resel();
        } else if Visual.active {
            (*wp).w_cursor = (*curwin).w_cursor;
        }

        if !win_valid(wp) {
            return;
        }

        win_enter(wp, true);

        if win_valid(owp) && (*owp).w_onebuf_opt.wo_cole > 0 && msg_scrolled == 0 {
            redrawWinline(owp, (*owp).w_cursor.lnum);
        }
        if (*curwin).w_onebuf_opt.wo_cole > 0 && msg_scrolled == 0 {
            redrawWinline(curwin, (*curwin).w_cursor.lnum);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_find_tabpage(win: *mut win_T) -> *mut tabpage_T {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if ptr::eq(tp, curtab) {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if ptr::eq(wp, win) {
                    return tp;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_vert_neighbor(
    tp: *mut tabpage_T,
    wp: *mut win_T,
    up: bool,
    mut count: c_int,
) -> *mut win_T {
    unsafe {
        let mut foundfr = (*wp).w_frame;

        if (*wp).w_floating {
            return if win_valid(prevwin) && !(*prevwin).w_floating {
                prevwin
            } else {
                firstwin
            };
        }

        'outer: while count > 0 {
            count -= 1;
            let mut nfr: *mut frame_T;
            let mut fr = foundfr;
            loop {
                if ptr::eq(fr, (*tp).tp_topframe.cast()) {
                    break 'outer;
                }
                if up {
                    nfr = (*fr).fr_prev;
                } else {
                    nfr = (*fr).fr_next;
                }
                if (*(*fr).fr_parent).fr_layout == FR_COL && !nfr.is_null() {
                    break;
                }
                fr = (*fr).fr_parent;
            }

            loop {
                if (*nfr).fr_layout == FR_LEAF {
                    foundfr = nfr;
                    break;
                }
                fr = (*nfr).fr_child;
                if (*nfr).fr_layout == FR_ROW {
                    while !(*fr).fr_next.is_null()
                        && (*frame2win(fr)).w_wincol + (*fr).fr_width
                            <= (*wp).w_wincol + (*wp).w_wcol
                    {
                        fr = (*fr).fr_next;
                    }
                }
                if (*nfr).fr_layout == FR_COL && up {
                    while !(*fr).fr_next.is_null() {
                        fr = (*fr).fr_next;
                    }
                }
                nfr = fr;
            }
        }
        if !foundfr.is_null() {
            (*foundfr).fr_win
        } else {
            ptr::null_mut()
        }
    }
}

unsafe fn win_goto_ver(up: bool, count: c_int) {
    unsafe {
        let win = win_vert_neighbor(curtab, curwin, up, count);
        if !win.is_null() {
            win_goto(win);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_horz_neighbor(
    tp: *mut tabpage_T,
    wp: *mut win_T,
    left: bool,
    mut count: c_int,
) -> *mut win_T {
    unsafe {
        let mut foundfr = (*wp).w_frame;

        if (*wp).w_floating {
            return if win_valid(prevwin) && !(*prevwin).w_floating {
                prevwin
            } else {
                firstwin
            };
        }

        'outer: while count > 0 {
            count -= 1;
            let mut nfr: *mut frame_T;
            let mut fr = foundfr;
            loop {
                if ptr::eq(fr, (*tp).tp_topframe.cast()) {
                    break 'outer;
                }
                if left {
                    nfr = (*fr).fr_prev;
                } else {
                    nfr = (*fr).fr_next;
                }
                if (*(*fr).fr_parent).fr_layout == FR_ROW && !nfr.is_null() {
                    break;
                }
                fr = (*fr).fr_parent;
            }

            loop {
                if (*nfr).fr_layout == FR_LEAF {
                    foundfr = nfr;
                    break;
                }
                fr = (*nfr).fr_child;
                if (*nfr).fr_layout == FR_COL {
                    while !(*fr).fr_next.is_null()
                        && (*frame2win(fr)).w_winrow + (*fr).fr_height
                            <= (*wp).w_winrow + (*wp).w_wrow
                    {
                        fr = (*fr).fr_next;
                    }
                }
                if (*nfr).fr_layout == FR_ROW && left {
                    while !(*fr).fr_next.is_null() {
                        fr = (*fr).fr_next;
                    }
                }
                nfr = fr;
            }
        }
        if !foundfr.is_null() {
            (*foundfr).fr_win
        } else {
            ptr::null_mut()
        }
    }
}

unsafe fn win_goto_hor(left: bool, count: c_int) {
    unsafe {
        let win = win_horz_neighbor(curtab, curwin, left, count);
        if !win.is_null() {
            win_goto(win);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_enter(wp: *mut win_T, undo_sync: bool) {
    unsafe {
        win_enter_ext(
            wp,
            (if undo_sync { WEE_UNDO_SYNC } else { 0 })
                | WEE_TRIGGER_ENTER_AUTOCMDS
                | WEE_TRIGGER_LEAVE_AUTOCMDS,
        );
    }
}

unsafe fn win_enter_ext(wp: *mut win_T, flags: c_int) {
    unsafe {
        let mut other_buffer = false;
        let curwin_invalid = (flags & WEE_CURWIN_INVALID) != 0;

        if ptr::eq(wp, curwin) && !curwin_invalid {
            return;
        }

        if !curwin_invalid {
            leaving_window(curwin);
        }

        if !curwin_invalid && (flags & WEE_TRIGGER_LEAVE_AUTOCMDS) != 0 {
            if !ptr::eq((*wp).w_buffer, curbuf) {
                apply_autocmds(EVENT_BUFLEAVE, ptr::null(), ptr::null(), false, curbuf);
                other_buffer = true;
                if !win_valid(wp) {
                    return;
                }
            }
            apply_autocmds(EVENT_WINLEAVE, ptr::null(), ptr::null(), false, curbuf);
            if !win_valid(wp) {
                return;
            }
            if aborting() {
                return;
            }
        }

        if (flags & WEE_UNDO_SYNC) != 0 && !ptr::eq(curbuf, (*wp).w_buffer) {
            u_sync(false);
        }

        if *p_spk == b'c' as c_char && !curwin_invalid {
            update_topline(curwin);
        }

        if !ptr::eq((*wp).w_buffer, curbuf) {
            buf_copy_options((*wp).w_buffer, BCO_ENTER | BCO_NOHELP);
        }
        if !curwin_invalid {
            prevwin = curwin;
            (*curwin).w_redr_status = true;
        }
        curwin = wp;
        curbuf = (*wp).w_buffer;

        check_cursor(curwin);
        if !virtual_active(curwin) {
            (*curwin).w_cursor.coladd = 0;
        }
        if *p_spk == b'c' as c_char {
            changed_line_abv_curs();
        } else {
            win_fix_cursor((get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) != 0);
        }

        update_cwd(kCdCauseWindow);

        entering_window(curwin);
        if (flags & WEE_TRIGGER_NEW_AUTOCMDS) != 0 {
            apply_autocmds(EVENT_WINNEW, ptr::null(), ptr::null(), false, curbuf);
        }
        if (flags & WEE_TRIGGER_ENTER_AUTOCMDS) != 0 {
            apply_autocmds(EVENT_WINENTER, ptr::null(), ptr::null(), false, curbuf);
            if other_buffer {
                apply_autocmds(EVENT_BUFENTER, ptr::null(), ptr::null(), false, curbuf);
            }
        }

        maketitle();
        (*curwin).w_redr_status = true;
        redraw_tabline = true;
        if restart_edit != 0 {
            redraw_later(curwin, UPD_VALID);
        }

        if (*curwin).w_hl_attr_normal != (*curwin).w_hl_attr_normalnc {
            redraw_later(curwin, UPD_NOT_VALID);
        }
        if !prevwin.is_null() && (*prevwin).w_hl_attr_normal != (*prevwin).w_hl_attr_normalnc {
            redraw_later(prevwin, UPD_NOT_VALID);
        }

        if OptInt::from((*curwin).w_height) < p_wh
            && (*curwin).w_onebuf_opt.wo_wfh == 0
            && !(*curwin).w_floating
        {
            win_setheight(p_wh as c_int);
        } else if (*curwin).w_height == 0 {
            win_setheight(1);
        }

        if OptInt::from((*curwin).w_width) < p_wiw
            && (*curwin).w_onebuf_opt.wo_wfw == 0
            && !(*curwin).w_floating
        {
            win_setwidth(p_wiw as c_int);
        }

        setmouse();
        do_autochdir();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_cwd(cause: CdCause) {
    unsafe {
        let new_dir: *mut c_char;
        let scope: CdScope;

        if !(*curbuf).b_localdir.is_null() {
            new_dir = (*curbuf).b_localdir;
            scope = kCdScopeBuffer;
        } else if !(*curwin).w_localdir.is_null() {
            new_dir = (*curwin).w_localdir;
            scope = kCdScopeWindow;
        } else if !(*curtab).tp_localdir.is_null() {
            new_dir = (*curtab).tp_localdir;
            scope = kCdScopeTabpage;
        } else {
            new_dir = ptr::null_mut();
            scope = kCdScopeGlobal;
        }

        let mut cwd = [0 as c_char; MAXPATHL];
        if os_dirname(cwd.as_mut_ptr(), MAXPATHL) != OK {
            cwd[0] = NUL;
        }

        if !new_dir.is_null() {
            if globaldir.is_null() && cwd[0] != NUL {
                globaldir = xstrdup(cwd.as_ptr());
            }
            let dir_differs = !path_equal(new_dir, cwd.as_ptr(), kPathCmpLiteral);
            if p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(new_dir, scope, cause, true);
            }
            if os_chdir(new_dir) == 0 && p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(new_dir, scope, cause, false);
            }
            last_chdir_reason = ptr::null_mut();
            shorten_fnames(true);
        } else if !globaldir.is_null() {
            let dir_differs = !path_equal(globaldir, cwd.as_ptr(), kPathCmpLiteral);
            if p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(globaldir, kCdScopeGlobal, cause, true);
            }
            if os_chdir(globaldir) == 0 && p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(globaldir, kCdScopeGlobal, cause, false);
            }
            xfree(globaldir.cast());
            globaldir = ptr::null_mut();
            last_chdir_reason = ptr::null_mut();
            shorten_fnames(true);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_jump_open_win(buf: *mut buf_T) -> *mut win_T {
    unsafe {
        if ptr::eq((*curwin).w_buffer, buf) {
            win_enter(curwin, false);
            return curwin;
        }
        let mut wp = firstwin;
        while !wp.is_null() {
            if ptr::eq((*wp).w_buffer, buf) {
                win_enter(wp, false);
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_jump_open_tab(buf: *mut buf_T) -> *mut win_T {
    unsafe {
        let wp = buf_jump_open_win(buf);
        if !wp.is_null() {
            return wp;
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            if ptr::eq(tp, curtab) {
                tp = (*tp).tp_next;
                continue;
            }
            let mut wp2 = (*tp).tp_firstwin;
            while !wp2.is_null() {
                if ptr::eq((*wp2).w_buffer, buf) {
                    goto_tabpage_win(tp, wp2);
                    if !ptr::eq(curwin, wp2) {
                        wp2 = ptr::null_mut();
                    }
                    return wp2;
                }
                wp2 = (*wp2).w_next;
            }
            tp = (*tp).tp_next;
        }

        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_alloc(after: *mut win_T, hidden: bool) -> *mut win_T {
    unsafe {
        let new_wp = xcalloc(1, size_of::<win_T>()).cast::<win_T>();

        last_win_id += 1;
        (*new_wp).handle = last_win_id;
        *map_put_ref_int_ptr_t(
            &raw mut window_handles,
            (*new_wp).handle,
            ptr::null_mut(),
            ptr::null_mut(),
        ) = new_wp.cast();

        (*new_wp).w_grid_alloc.mouse_enabled = true;

        grid_assign_handle(&raw mut (*new_wp).w_grid_alloc);

        (*new_wp).w_vars = tv_dict_alloc();
        init_var_dict(
            (*new_wp).w_vars,
            (&raw mut (*new_wp).w_winvar).cast(),
            VAR_SCOPE,
        );

        block_autocmds();
        if !hidden {
            let mut tp: *mut tabpage_T = ptr::null_mut();
            if !after.is_null() {
                tp = win_find_tabpage(after);
                if ptr::eq(tp, curtab) {
                    tp = ptr::null_mut();
                }
            }
            win_append(after, new_wp, tp);
        }

        (*new_wp).w_wincol = 0;
        (*new_wp).w_width = Columns;

        (*new_wp).w_topline = 1;
        (*new_wp).w_topfill = 0;
        (*new_wp).w_botline = 2;
        (*new_wp).w_cursor.lnum = 1;
        (*new_wp).w_scbind_pos = 1;
        (*new_wp).w_floating = false;
        (*new_wp).w_kind = kWinNormal;
        (*new_wp).w_config = WIN_CONFIG_INIT;
        (*new_wp).w_viewport_invalid = true;
        (*new_wp).w_viewport_last_topline = 1;

        (*new_wp).w_ns_hl = -1;

        (*new_wp).w_ns_set = zeroed();

        (*new_wp).w_onebuf_opt.wo_so = -1;
        (*new_wp).w_allbuf_opt.wo_so = -1;
        (*new_wp).w_onebuf_opt.wo_sop = -1;
        (*new_wp).w_allbuf_opt.wo_sop = -1;
        (*new_wp).w_onebuf_opt.wo_siso = -1;
        (*new_wp).w_allbuf_opt.wo_siso = -1;

        (*new_wp).w_fraction = 0;
        (*new_wp).w_prev_fraction_row = -1;

        foldInitWin(new_wp);
        unblock_autocmds();
        (*new_wp).w_next_match_id = 1000;
        new_wp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_wininfo(wip: *mut WinInfo, bp: *mut buf_T) {
    unsafe {
        if (*wip).wi_optset {
            clear_winopt(&raw mut (*wip).wi_opt);
            deleteFoldRecurse(bp, &raw mut (*wip).wi_folds);
        }
        xfree(wip.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_free(wp: *mut win_T, tp: *mut tabpage_T) {
    unsafe {
        map_del_int_ptr_t(&raw mut window_handles, (*wp).handle, ptr::null_mut());
        clearFolding(wp);

        alist_unlink((*wp).w_alist.cast());

        block_autocmds();

        xfree((*wp).w_ns_set.keys.cast());
        xfree((*wp).w_ns_set.h.hash.cast());
        (*wp).w_ns_set = zeroed();

        clear_winopt(&raw mut (*wp).w_onebuf_opt);
        clear_winopt(&raw mut (*wp).w_allbuf_opt);

        xfree((*wp).w_p_lcs_chars.multispace.cast());
        xfree((*wp).w_p_lcs_chars.leadmultispace.cast());

        vars_clear(&raw mut (*(*wp).w_vars).dv_hashtab);
        hash_init(&raw mut (*(*wp).w_vars).dv_hashtab);
        unref_var_dict((*wp).w_vars);

        if ptr::eq(prevwin, wp) {
            prevwin = ptr::null_mut();
        }
        let mut ttp = first_tabpage;
        while !ttp.is_null() {
            if ptr::eq((*ttp).tp_prevwin, wp) {
                (*ttp).tp_prevwin = ptr::null_mut();
            }
            ttp = (*ttp).tp_next;
        }

        xfree((*wp).w_lines);

        for i in 0..(*wp).w_tagstacklen as usize {
            tagstack_clear_entry(&raw mut (*wp).w_tagstack[i]);
        }

        xfree((*wp).w_localdir.cast());
        xfree((*wp).w_prevdir.cast());

        stl_clear_click_defs(
            (*wp).w_status_click_defs.cast::<StlClickDefinition>(),
            (*wp).w_status_click_defs_size,
        );
        xfree((*wp).w_status_click_defs);

        stl_clear_click_defs(
            (*wp).w_winbar_click_defs.cast::<StlClickDefinition>(),
            (*wp).w_winbar_click_defs_size,
        );
        xfree((*wp).w_winbar_click_defs);

        let stc_map = &raw mut (*wp).w_statuscol_click_defs[0];
        for j in 0..(*stc_map).set.h.n_keys as usize {
            let lnum_click_defs: StcClicks = *(*stc_map).values.add(j);
            for i in 0..lnum_click_defs.set.h.n_keys as usize {
                let row_click_defs: StcClick = *lnum_click_defs.values.add(i);
                stl_clear_click_defs(
                    row_click_defs.def.cast::<StlClickDefinition>(),
                    row_click_defs.size,
                );
                xfree(row_click_defs.def);
            }
            xfree(lnum_click_defs.set.keys.cast());
            xfree(lnum_click_defs.set.h.hash.cast());
            xfree(lnum_click_defs.values.cast());
        }
        xfree((*stc_map).set.keys.cast());
        xfree((*stc_map).set.h.hash.cast());
        xfree((*stc_map).values.cast());
        *stc_map = zeroed();

        let mut buf = firstbuf;
        while !buf.is_null() {
            let mut wip_wp: *mut WinInfo = ptr::null_mut();
            let mut pos_wip = (*buf).b_wininfo.size;
            let mut pos_null = (*buf).b_wininfo.size;
            for i in 0..(*buf).b_wininfo.size {
                let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
                if ptr::eq((*wip).wi_win, wp) {
                    wip_wp = wip;
                    pos_wip = i;
                } else if (*wip).wi_win.is_null() {
                    pos_null = i;
                }
            }

            if !wip_wp.is_null() {
                (*wip_wp).wi_win = ptr::null_mut();
                if (*wp).w_config.style == kWinStyleMinimal && (*wip_wp).wi_optset {
                    clear_winopt(&raw mut (*wip_wp).wi_opt);
                    deleteFoldRecurse(buf, &raw mut (*wip_wp).wi_folds);
                    (*wip_wp).wi_optset = false;
                }
                if pos_null < (*buf).b_wininfo.size {
                    let pos_delete = pos_null.max(pos_wip);
                    free_wininfo(
                        (*(*buf).b_wininfo.items.add(pos_delete)).cast::<WinInfo>(),
                        buf,
                    );
                    (*buf).b_wininfo.size -= 1;
                    if pos_delete < (*buf).b_wininfo.size {
                        ptr::copy(
                            (*buf).b_wininfo.items.add(pos_delete + 1),
                            (*buf).b_wininfo.items.add(pos_delete),
                            (*buf).b_wininfo.size - pos_delete,
                        );
                    }
                }
            }
            buf = (*buf).b_next;
        }

        clear_virttext(&raw mut (*wp).w_config.title_chunks);
        clear_virttext(&raw mut (*wp).w_config.footer_chunks);

        clear_matches(wp);
        free_jumplist(wp);
        qf_free_all(wp);

        xfree((*wp).w_p_cc_cols.cast());

        win_free_grid(wp, false);

        if win_valid_any_tab(wp) {
            win_remove(wp, tp);
        }
        if autocmd_busy {
            (*wp).w_next = au_pending_free_win;
            au_pending_free_win = wp;
        } else {
            xfree(wp.cast());
        }

        unblock_autocmds();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_free_grid(wp: *mut win_T, reinit: bool) {
    unsafe {
        if (*wp).w_grid_alloc.handle != 0 && ui_has(kUIMultigrid) {
            ui_call_grid_destroy(Integer::from((*wp).w_grid_alloc.handle));
        }
        grid_free(&raw mut (*wp).w_grid_alloc);
        if reinit {
            (*wp).w_grid_alloc = zeroed();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_append(after: *mut win_T, wp: *mut win_T, tp: *mut tabpage_T) {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, curtab));

        let first = if tp.is_null() {
            &raw mut firstwin
        } else {
            &raw mut (*tp).tp_firstwin
        };
        let last = if tp.is_null() {
            &raw mut lastwin
        } else {
            &raw mut (*tp).tp_lastwin
        };

        let before = if after.is_null() {
            *first
        } else {
            (*after).w_next
        };

        (*wp).w_next = before;
        (*wp).w_prev = after;
        if after.is_null() {
            *first = wp;
        } else {
            (*after).w_next = wp;
        }
        if before.is_null() {
            *last = wp;
        } else {
            (*before).w_prev = wp;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_remove(wp: *mut win_T, tp: *mut tabpage_T) {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, curtab));

        if !(*wp).w_prev.is_null() {
            (*(*wp).w_prev).w_next = (*wp).w_next;
        } else if tp.is_null() {
            firstwin = (*wp).w_next;
            (*curtab).tp_firstwin = (*wp).w_next;
        } else {
            (*tp).tp_firstwin = (*wp).w_next;
        }
        if !(*wp).w_next.is_null() {
            (*(*wp).w_next).w_prev = (*wp).w_prev;
        } else if tp.is_null() {
            lastwin = (*wp).w_prev;
            (*curtab).tp_lastwin = (*wp).w_prev;
        } else {
            (*tp).tp_lastwin = (*wp).w_prev;
        }
    }
}

unsafe fn frame_append(after: *mut frame_T, frp: *mut frame_T) {
    unsafe {
        (*frp).fr_next = (*after).fr_next;
        (*after).fr_next = frp;
        if !(*frp).fr_next.is_null() {
            (*(*frp).fr_next).fr_prev = frp;
        }
        (*frp).fr_prev = after;
    }
}

unsafe fn frame_insert(before: *mut frame_T, frp: *mut frame_T) {
    unsafe {
        (*frp).fr_next = before;
        (*frp).fr_prev = (*before).fr_prev;
        (*before).fr_prev = frp;
        if !(*frp).fr_prev.is_null() {
            (*(*frp).fr_prev).fr_next = frp;
        } else {
            (*(*frp).fr_parent).fr_child = frp;
        }
    }
}

unsafe fn frame_remove(frp: *mut frame_T) {
    unsafe {
        if !(*frp).fr_prev.is_null() {
            (*(*frp).fr_prev).fr_next = (*frp).fr_next;
        } else {
            (*(*frp).fr_parent).fr_child = (*frp).fr_next;
        }
        if !(*frp).fr_next.is_null() {
            (*(*frp).fr_next).fr_prev = (*frp).fr_prev;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_new_screensize() {
    unsafe {
        if old_Rows != Rows {
            if p_window == OptInt::from(old_Rows - 1)
                || (old_Rows == 0 && !option_was_set(kOptWindow))
            {
                p_window = OptInt::from(Rows - 1);
            }
            old_Rows = Rows;
            win_new_screen_rows();
        }
        if old_Columns != Columns {
            old_Columns = Columns;
            win_new_screen_cols();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_new_screen_rows() {
    unsafe {
        if firstwin.is_null() {
            return;
        }
        let h = (rows_avail() as c_int).max(frame_minheight(topframe, ptr::null_mut()));

        frame_new_height(topframe, h, false, true, false);
        if !frame_check_height(topframe, h) {
            frame_new_height(topframe, h, false, false, false);
        }

        win_comp_pos();
        win_reconfig_floats();
        compute_cmdrow();
        (*curtab).tp_ch_used = p_ch;

        if !skip_win_fix_scroll {
            win_fix_scroll(true);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_new_screen_cols() {
    unsafe {
        if firstwin.is_null() {
            return;
        }

        frame_new_width(topframe, Columns, false, true);
        if !frame_check_width(topframe, Columns) {
            frame_new_width(topframe, Columns, false, false);
        }

        win_comp_pos();
        win_reconfig_floats();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn snapshot_windows_scroll_size() {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            (*wp).w_last_topline = (*wp).w_topline;
            (*wp).w_last_topfill = (*wp).w_topfill;
            (*wp).w_last_leftcol = (*wp).w_leftcol;
            (*wp).w_last_skipcol = (*wp).w_skipcol;
            (*wp).w_last_width = (*wp).w_width;
            (*wp).w_last_height = (*wp).w_height;
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn may_make_initial_scroll_size_snapshot() {
    unsafe {
        if !did_initial_scroll_size_snapshot {
            did_initial_scroll_size_snapshot = true;
            snapshot_windows_scroll_size();
        }
    }
}

unsafe fn make_win_info_dict(
    width: c_int,
    height: c_int,
    topline: c_int,
    topfill: c_int,
    leftcol: c_int,
    skipcol: c_int,
) -> *mut dict_T {
    unsafe {
        let d = tv_dict_alloc();
        (*d).dv_refcount = 1;

        let entries: [(&CStr, c_int); 6] = [
            (c"width", width),
            (c"height", height),
            (c"topline", topline),
            (c"topfill", topfill),
            (c"leftcol", leftcol),
            (c"skipcol", skipcol),
        ];

        for (key, val) in entries {
            let mut tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_number: i64::from(val),
                },
            };
            if tv_dict_add_tv(d, key.as_ptr(), key.to_bytes().len(), &raw mut tv) == FAIL {
                tv_dict_unref(d);
                return ptr::null_mut();
            }
        }
        d
    }
}

unsafe fn check_window_scroll_resize(
    size_count: *mut c_int,
    first_scroll_win: *mut *mut win_T,
    first_size_win: *mut *mut win_T,
    winlist: *mut list_T,
    v_event: *mut dict_T,
) {
    unsafe {
        let mut tot_width: c_int = 0;
        let mut tot_height: c_int = 0;
        let mut tot_topline: c_int = 0;
        let mut tot_topfill: c_int = 0;
        let mut tot_leftcol: c_int = 0;
        let mut tot_skipcol: c_int = 0;

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_floating && (*wp).w_last_topline == 0 {
                (*wp).w_last_topline = (*wp).w_topline;
                (*wp).w_last_topfill = (*wp).w_topfill;
                (*wp).w_last_leftcol = (*wp).w_leftcol;
                (*wp).w_last_skipcol = (*wp).w_skipcol;
                (*wp).w_last_width = (*wp).w_width;
                (*wp).w_last_height = (*wp).w_height;
                wp = (*wp).w_next;
                continue;
            }

            let ignore_scroll = event_ignored(EVENT_WINSCROLLED, (*wp).w_onebuf_opt.wo_eiw);
            let size_changed = !event_ignored(EVENT_WINRESIZED, (*wp).w_onebuf_opt.wo_eiw)
                && ((*wp).w_last_width != (*wp).w_width || (*wp).w_last_height != (*wp).w_height);
            if size_changed {
                if !winlist.is_null() {
                    let tv = typval_T {
                        v_type: VarType::VAR_NUMBER,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval {
                            v_number: i64::from((*wp).handle),
                        },
                    };
                    tv_list_append_owned_tv(winlist, tv);
                } else if !size_count.is_null() {
                    debug_assert!(!first_size_win.is_null() && !first_scroll_win.is_null());
                    *size_count += 1;
                    if (*first_size_win).is_null() {
                        *first_size_win = wp;
                    }
                    if (*first_scroll_win).is_null() && !ignore_scroll {
                        *first_scroll_win = wp;
                    }
                }
            }

            let scroll_changed = !ignore_scroll
                && ((*wp).w_last_topline != (*wp).w_topline
                    || (*wp).w_last_topfill != (*wp).w_topfill
                    || (*wp).w_last_leftcol != (*wp).w_leftcol
                    || (*wp).w_last_skipcol != (*wp).w_skipcol);
            if scroll_changed && !first_scroll_win.is_null() && (*first_scroll_win).is_null() {
                *first_scroll_win = wp;
            }

            if (size_changed || scroll_changed) && !v_event.is_null() {
                let width = (*wp).w_width - (*wp).w_last_width;
                let height = (*wp).w_height - (*wp).w_last_height;
                let topline = (*wp).w_topline - (*wp).w_last_topline;
                let topfill = (*wp).w_topfill - (*wp).w_last_topfill;
                let leftcol = (*wp).w_leftcol - (*wp).w_last_leftcol;
                let skipcol = (*wp).w_skipcol - (*wp).w_last_skipcol;
                let d = make_win_info_dict(width, height, topline, topfill, leftcol, skipcol);
                if d.is_null() {
                    break;
                }
                let mut winid = [0 as c_char; NUMBUFLEN];
                let key_len = vim_snprintf(
                    winid.as_mut_ptr(),
                    winid.len(),
                    c"%d".as_ptr(),
                    (*wp).handle,
                );
                if tv_dict_add_dict(v_event, winid.as_ptr(), key_len as usize, d) == FAIL {
                    tv_dict_unref(d);
                    break;
                }
                (*d).dv_refcount -= 1;

                tot_width += width.abs();
                tot_height += height.abs();
                tot_topline += topline.abs();
                tot_topfill += topfill.abs();
                tot_leftcol += leftcol.abs();
                tot_skipcol += skipcol.abs();
            }

            wp = (*wp).w_next;
        }

        if !v_event.is_null() {
            let alldict = make_win_info_dict(
                tot_width,
                tot_height,
                tot_topline,
                tot_topfill,
                tot_leftcol,
                tot_skipcol,
            );
            if !alldict.is_null() {
                if tv_dict_add_dict(v_event, c"all".as_ptr(), 3, alldict) == FAIL {
                    tv_dict_unref(alldict);
                } else {
                    (*alldict).dv_refcount -= 1;
                }
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn may_trigger_win_scrolled_resized() {
    unsafe {
        let do_resize = has_event(EVENT_WINRESIZED);
        let do_scroll = has_event(EVENT_WINSCROLLED);

        if win_scrolled_resized_recursive
            || !(do_scroll || do_resize)
            || !did_initial_scroll_size_snapshot
        {
            return;
        }

        let mut size_count: c_int = 0;
        let mut first_scroll_win: *mut win_T = ptr::null_mut();
        let mut first_size_win: *mut win_T = ptr::null_mut();
        check_window_scroll_resize(
            &raw mut size_count,
            &raw mut first_scroll_win,
            &raw mut first_size_win,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        let trigger_resize = do_resize && size_count > 0;
        let trigger_scroll = do_scroll && !first_scroll_win.is_null();
        if !trigger_resize && !trigger_scroll {
            return;
        }

        let mut windows_list: *mut list_T = ptr::null_mut();
        if trigger_resize {
            windows_list = tv_list_alloc(size_count as isize);
            check_window_scroll_resize(
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                windows_list,
                ptr::null_mut(),
            );
        }

        let mut scroll_dict: *mut dict_T = ptr::null_mut();
        if trigger_scroll {
            scroll_dict = tv_dict_alloc();
            (*scroll_dict).dv_refcount = 1;
            check_window_scroll_resize(
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                scroll_dict,
            );
        }

        snapshot_windows_scroll_size();

        win_scrolled_resized_recursive = true;

        let mut resize_winid = [0 as c_char; NUMBUFLEN];
        let mut resize_win: *mut win_T = ptr::null_mut();
        let mut resize_bufref: bufref_T = zeroed();
        if trigger_resize {
            resize_win = first_size_win;
            vim_snprintf(
                resize_winid.as_mut_ptr(),
                resize_winid.len(),
                c"%d".as_ptr(),
                (*resize_win).handle,
            );
            set_bufref(&raw mut resize_bufref, (*resize_win).w_buffer);
        }

        let mut scroll_winid = [0 as c_char; NUMBUFLEN];
        let mut scroll_win: *mut win_T = ptr::null_mut();
        let mut scroll_bufref: bufref_T = zeroed();
        if trigger_scroll {
            scroll_win = first_scroll_win;
            vim_snprintf(
                scroll_winid.as_mut_ptr(),
                scroll_winid.len(),
                c"%d".as_ptr(),
                (*scroll_win).handle,
            );
            set_bufref(&raw mut scroll_bufref, (*scroll_win).w_buffer);
        }

        if trigger_resize && !windows_list.is_null() {
            let mut save_v_event: save_v_event_T = zeroed();
            let v_event = get_v_event(&raw mut save_v_event);

            if tv_dict_add_list(v_event, c"windows".as_ptr(), 7, windows_list) == OK {
                tv_dict_set_keys_readonly(v_event);
                let buf = if bufref_valid(&raw mut resize_bufref) {
                    resize_bufref.br_buf
                } else {
                    curbuf
                };
                let win = if win_valid_any_tab(resize_win) {
                    resize_win
                } else {
                    curwin
                };
                apply_autocmds_win(
                    EVENT_WINRESIZED,
                    resize_winid.as_ptr(),
                    resize_winid.as_ptr(),
                    false,
                    buf,
                    win,
                );
            }
            restore_v_event(v_event, &raw mut save_v_event);
        }

        if trigger_scroll && !scroll_dict.is_null() {
            let mut save_v_event: save_v_event_T = zeroed();
            let v_event = get_v_event(&raw mut save_v_event);

            tv_dict_extend(v_event, scroll_dict, c"move".as_ptr());
            tv_dict_set_keys_readonly(v_event);
            tv_dict_unref(scroll_dict);

            let buf = if bufref_valid(&raw mut scroll_bufref) {
                scroll_bufref.br_buf
            } else {
                curbuf
            };
            let win = if win_valid_any_tab(scroll_win) {
                scroll_win
            } else {
                curwin
            };
            apply_autocmds_win(
                EVENT_WINSCROLLED,
                scroll_winid.as_ptr(),
                scroll_winid.as_ptr(),
                false,
                buf,
                win,
            );

            restore_v_event(v_event, &raw mut save_v_event);
        }

        win_scrolled_resized_recursive = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_size_save(gap: *mut garray_T) {
    unsafe {
        ga_init(gap, size_of::<c_int>() as c_int, 1);
        ga_grow(gap, win_count() * 2 + 1);
        let data = (*gap).ga_data.cast::<c_int>();
        *data.add((*gap).ga_len as usize) =
            rows_avail() as c_int + global_stl_height() - last_stl_height(false);
        (*gap).ga_len += 1;

        let mut wp = firstwin;
        while !wp.is_null() {
            let data = (*gap).ga_data.cast::<c_int>();
            *data.add((*gap).ga_len as usize) = (*wp).w_width + (*wp).w_vsep_width;
            (*gap).ga_len += 1;
            *data.add((*gap).ga_len as usize) = (*wp).w_height;
            (*gap).ga_len += 1;
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_size_restore(gap: *mut garray_T) {
    unsafe {
        let data = (*gap).ga_data.cast::<c_int>();
        if win_count() * 2 + 1 == (*gap).ga_len
            && *data == rows_avail() as c_int + global_stl_height() - last_stl_height(false)
        {
            for _ in 0..2 {
                let mut i: usize = 1;
                let mut wp = firstwin;
                while !wp.is_null() {
                    let width = *data.add(i);
                    i += 1;
                    let height = *data.add(i);
                    i += 1;
                    if !(*wp).w_floating {
                        frame_setwidth((*wp).w_frame, width, true);
                        win_setheight_win(height, wp, true);
                    }
                    wp = (*wp).w_next;
                }
            }
            win_comp_pos();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_comp_pos() -> c_int {
    unsafe {
        let mut row = tabline_height();
        let mut col: c_int = 0;

        frame_comp_pos(topframe, &raw mut row, &raw mut col);

        let mut wp = lastwin;
        while !wp.is_null() && (*wp).w_floating {
            if (*wp).w_config.relative == kFloatRelativeWindow {
                (*wp).w_pos_changed = true;
            }
            wp = (*wp).w_prev;
        }

        row + global_stl_height()
    }
}

unsafe fn frame_comp_pos(topfrp: *mut frame_T, row: *mut c_int, col: *mut c_int) {
    unsafe {
        let wp = (*topfrp).fr_win;
        if !wp.is_null() {
            if (*wp).w_winrow != *row || (*wp).w_wincol != *col {
                (*wp).w_winrow = *row;
                (*wp).w_wincol = *col;
                redraw_later(wp, UPD_NOT_VALID);
                (*wp).w_redr_status = true;
                (*wp).w_pos_changed = true;
            }
            let h = (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height;
            *row += if h > (*topfrp).fr_height {
                (*topfrp).fr_height
            } else {
                h
            };
            *col += (*wp).w_width + (*wp).w_vsep_width;
        } else {
            let startrow = *row;
            let startcol = *col;
            let mut frp = (*topfrp).fr_child;
            while !frp.is_null() {
                if (*topfrp).fr_layout == FR_ROW {
                    *row = startrow;
                } else {
                    *col = startcol;
                }
                frame_comp_pos(frp, row, col);
                frp = (*frp).fr_next;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_setheight(height: c_int) {
    unsafe {
        win_setheight_win(height, curwin, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_setheight_win(mut height: c_int, win: *mut win_T, from_top: bool) {
    unsafe {
        let min_wmh = if ptr::eq(win, curwin) {
            p_wmh.max(1)
        } else {
            p_wmh
        } as c_int;
        height = height.max(min_wmh + (*win).w_winbar_height);

        if (*win).w_floating {
            (*win).w_config.height = height.max(1);
            win_config_float(win, (*win).w_config);
            redraw_later(win, UPD_VALID);
        } else {
            frame_setheight(
                (*win).w_frame,
                height + (*win).w_hsep_height + (*win).w_status_height,
                from_top,
            );

            win_comp_pos();
            win_fix_scroll(true);

            redraw_all_later(UPD_NOT_VALID);
            redraw_cmdline = true;
        }
    }
}

unsafe fn frame_setheight(curfrp: *mut frame_T, mut height: c_int, from_top: bool) {
    unsafe {
        if (*curfrp).fr_height == height {
            return;
        }

        if (*curfrp).fr_parent.is_null() {
            if height > 0 {
                frame_new_height(curfrp, height, false, false, true);
            }
        } else if (*(*curfrp).fr_parent).fr_layout == FR_ROW {
            let h = frame_minheight((*curfrp).fr_parent, ptr::null_mut());
            height = height.max(h);
            frame_setheight((*curfrp).fr_parent, height, from_top);
        } else {
            let mut room: c_int = 0;
            let mut room_cmdline: c_int = 0;
            let mut room_reserved: c_int = 0;

            for run in 1..=2 {
                room = 0;
                room_reserved = 0;
                let mut frp = (*(*curfrp).fr_parent).fr_child;
                while !frp.is_null() {
                    if !ptr::eq(frp, curfrp)
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0
                    {
                        room_reserved += (*frp).fr_height;
                    }
                    room += (*frp).fr_height;
                    if !ptr::eq(frp, curfrp) {
                        room -= frame_minheight(frp, ptr::null_mut());
                    }
                    frp = (*frp).fr_next;
                }
                if !from_top || (*curfrp).fr_width != Columns {
                    room_cmdline = 0;
                } else {
                    let wp = lastwin_nofloating(ptr::null_mut());
                    room_cmdline = Rows
                        - p_ch as c_int
                        - global_stl_height()
                        - ((*wp).w_winrow
                            + (*wp).w_height
                            + (*wp).w_hsep_height
                            + (*wp).w_status_height);
                    room_cmdline = room_cmdline.max(0);
                }

                if height <= room + room_cmdline {
                    break;
                }
                if run == 2 || (*curfrp).fr_width == Columns {
                    height = room + room_cmdline;
                    break;
                }
                frame_setheight(
                    (*curfrp).fr_parent,
                    height + frame_minheight((*curfrp).fr_parent, nowin()) - p_wmh as c_int - 1,
                    from_top,
                );
            }

            let mut take = height - (*curfrp).fr_height;

            if height > room + room_cmdline - room_reserved {
                room_reserved = room + room_cmdline - height;
            }
            if take < 0 && room - (*curfrp).fr_height <= room_reserved {
                room_reserved = 0;
            }

            if take > 0 && room_cmdline > 0 {
                room_cmdline = room_cmdline.min(take);
                take -= room_cmdline;
                (*topframe).fr_height += room_cmdline;
            }

            frame_new_height(curfrp, height, false, false, true);

            for run in 0..2 {
                let forward = (run == 0) == from_top;
                let mut frp = if forward {
                    (*curfrp).fr_next
                } else {
                    (*curfrp).fr_prev
                };

                while !frp.is_null() && take != 0 {
                    let h = frame_minheight(frp, ptr::null_mut());
                    if room_reserved > 0
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0
                    {
                        if room_reserved >= (*frp).fr_height {
                            room_reserved -= (*frp).fr_height;
                        } else {
                            if (*frp).fr_height - room_reserved > take {
                                room_reserved = (*frp).fr_height - take;
                            }
                            take -= (*frp).fr_height - room_reserved;
                            frame_new_height(frp, room_reserved, false, false, true);
                            room_reserved = 0;
                        }
                    } else if (*frp).fr_height - take < h {
                        take -= (*frp).fr_height - h;
                        frame_new_height(frp, h, false, false, true);
                    } else {
                        frame_new_height(frp, (*frp).fr_height - take, false, false, true);
                        take = 0;
                    }
                    frp = if forward {
                        (*frp).fr_next
                    } else {
                        (*frp).fr_prev
                    };
                }
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_setwidth(width: c_int) {
    unsafe {
        win_setwidth_win(width, curwin, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_setwidth_win(mut width: c_int, wp: *mut win_T, from_left: bool) {
    unsafe {
        if ptr::eq(wp, curwin) {
            width = width.max(p_wmw as c_int).max(1);
        } else if width < 0 {
            width = 0;
        }
        if (*wp).w_floating {
            (*wp).w_config.width = width;
            win_config_float(wp, (*wp).w_config);
            redraw_later(wp, UPD_NOT_VALID);
        } else {
            frame_setwidth((*wp).w_frame, width + (*wp).w_vsep_width, from_left);

            win_comp_pos();
            redraw_all_later(UPD_NOT_VALID);
        }
    }
}

unsafe fn frame_setwidth(curfrp: *mut frame_T, mut width: c_int, from_left: bool) {
    unsafe {
        if (*curfrp).fr_width == width {
            return;
        }

        if (*curfrp).fr_parent.is_null() {
            return;
        }

        if (*(*curfrp).fr_parent).fr_layout == FR_COL {
            let w = frame_minwidth((*curfrp).fr_parent, ptr::null_mut());
            width = width.max(w);
            frame_setwidth((*curfrp).fr_parent, width, from_left);
        } else {
            let mut room: c_int = 0;
            let mut room_reserved: c_int = 0;
            for run in 1..=2 {
                room = 0;
                room_reserved = 0;
                let mut frp = (*(*curfrp).fr_parent).fr_child;
                while !frp.is_null() {
                    if !ptr::eq(frp, curfrp)
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfw != 0
                    {
                        room_reserved += (*frp).fr_width;
                    }
                    room += (*frp).fr_width;
                    if !ptr::eq(frp, curfrp) {
                        room -= frame_minwidth(frp, ptr::null_mut());
                    }
                    frp = (*frp).fr_next;
                }

                if width <= room {
                    break;
                }
                if run == 2 || OptInt::from((*curfrp).fr_height) >= rows_avail() {
                    width = room;
                    break;
                }
                frame_setwidth(
                    (*curfrp).fr_parent,
                    width + frame_minwidth((*curfrp).fr_parent, nowin()) - p_wmw as c_int - 1,
                    from_left,
                );
            }

            let mut take = width - (*curfrp).fr_width;

            if width > room - room_reserved {
                room_reserved = room - width;
            }
            if take < 0 && room - (*curfrp).fr_width < room_reserved {
                room_reserved = 0;
            }

            frame_new_width(curfrp, width, false, false);

            for run in 0..2 {
                let forward = (run == 0) == from_left;
                let mut frp = if forward {
                    (*curfrp).fr_next
                } else {
                    (*curfrp).fr_prev
                };

                while !frp.is_null() && take != 0 {
                    let w = frame_minwidth(frp, ptr::null_mut());
                    if room_reserved > 0
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfw != 0
                    {
                        if room_reserved >= (*frp).fr_width {
                            room_reserved -= (*frp).fr_width;
                        } else {
                            if (*frp).fr_width - room_reserved > take {
                                room_reserved = (*frp).fr_width - take;
                            }
                            take -= (*frp).fr_width - room_reserved;
                            frame_new_width(frp, room_reserved, false, false);
                            room_reserved = 0;
                        }
                    } else if (*frp).fr_width - take < w {
                        take -= (*frp).fr_width - w;
                        frame_new_width(frp, w, false, false);
                    } else {
                        frame_new_width(frp, (*frp).fr_width - take, false, false);
                        take = 0;
                    }
                    frp = if forward {
                        (*frp).fr_next
                    } else {
                        (*frp).fr_prev
                    };
                }
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn did_set_winminheight(_args: *mut optset_T) -> *const c_char {
    unsafe {
        let mut first = true;
        while p_wmh > 0 {
            let room = Rows - p_ch as c_int;
            let needed = min_rows_for_all_tabpages();
            if room >= needed {
                break;
            }
            p_wmh -= 1;
            if first {
                emsg(gettext(e_noroom.as_ptr()));
                first = false;
            }
        }
        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn did_set_winminwidth(_args: *mut optset_T) -> *const c_char {
    unsafe {
        let mut first = true;
        while p_wmw > 0 {
            let room = Columns;
            let needed = frame_minwidth(topframe, ptr::null_mut());
            if room >= needed {
                break;
            }
            p_wmw -= 1;
            if first {
                emsg(gettext(e_noroom.as_ptr()));
                first = false;
            }
        }
        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_drag_status_line(dragwin: *mut win_T, mut offset: c_int) {
    unsafe {
        let mut fr = (*dragwin).w_frame;
        let mut curfr = fr;
        if !ptr::eq(fr, topframe) {
            fr = (*fr).fr_parent;
            if (*fr).fr_layout != FR_COL {
                curfr = fr;
                if !ptr::eq(fr, topframe) {
                    fr = (*fr).fr_parent;
                }
            }
        }

        while !ptr::eq(curfr, topframe) && (*curfr).fr_next.is_null() {
            if !ptr::eq(fr, topframe) {
                fr = (*fr).fr_parent;
            }
            curfr = fr;
            if !ptr::eq(fr, topframe) {
                fr = (*fr).fr_parent;
            }
        }

        let mut room: c_int;
        let up = offset < 0;

        if up {
            offset = -offset;
            if ptr::eq(fr, curfr) {
                room = (*fr).fr_height - frame_minheight(fr, ptr::null_mut());
            } else {
                room = 0;
                fr = (*fr).fr_child;
                loop {
                    room += (*fr).fr_height - frame_minheight(fr, ptr::null_mut());
                    if ptr::eq(fr, curfr) {
                        break;
                    }
                    fr = (*fr).fr_next;
                }
            }
            fr = (*curfr).fr_next;
        } else {
            room = Rows - cmdline_row;
            if !(*curfr).fr_next.is_null() {
                room -= p_ch as c_int + global_stl_height();
            } else if min_set_ch > 0 {
                room -= 1;
            }
            room = room.max(0);
            fr = (*curfr).fr_next;
            while !fr.is_null() {
                room += (*fr).fr_height - frame_minheight(fr, ptr::null_mut());
                fr = (*fr).fr_next;
            }
            fr = curfr;
        }

        offset = offset.min(room);
        if offset <= 0 {
            return;
        }

        if !fr.is_null() {
            frame_new_height(fr, (*fr).fr_height + offset, up, false, true);
        }

        if up {
            fr = curfr;
        } else {
            fr = (*curfr).fr_next;
        }
        while !fr.is_null() && offset > 0 {
            let n = frame_minheight(fr, ptr::null_mut());
            if (*fr).fr_height - offset <= n {
                offset -= (*fr).fr_height - n;
                frame_new_height(fr, n, !up, false, true);
            } else {
                frame_new_height(fr, (*fr).fr_height - offset, !up, false, true);
                break;
            }
            if up {
                fr = (*fr).fr_prev;
            } else {
                fr = (*fr).fr_next;
            }
        }
        win_comp_pos();
        win_fix_scroll(true);

        redraw_all_later(UPD_SOME_VALID);
        showmode();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_drag_vsep_line(dragwin: *mut win_T, mut offset: c_int) {
    unsafe {
        let mut fr = (*dragwin).w_frame;
        if ptr::eq(fr, topframe) {
            return;
        }
        let mut curfr = fr;
        fr = (*fr).fr_parent;
        if (*fr).fr_layout != FR_ROW {
            if ptr::eq(fr, topframe) {
                return;
            }
            curfr = fr;
            fr = (*fr).fr_parent;
        }

        while (*curfr).fr_next.is_null() {
            if ptr::eq(fr, topframe) {
                break;
            }
            curfr = fr;
            fr = (*fr).fr_parent;
            if !ptr::eq(fr, topframe) {
                curfr = fr;
                fr = (*fr).fr_parent;
            }
        }

        let mut room: c_int;
        let left = offset < 0;

        if left {
            offset = -offset;
            room = 0;
            fr = (*fr).fr_child;
            loop {
                room += (*fr).fr_width - frame_minwidth(fr, ptr::null_mut());
                if ptr::eq(fr, curfr) {
                    break;
                }
                fr = (*fr).fr_next;
            }
            fr = (*curfr).fr_next;
        } else {
            room = 0;
            fr = (*curfr).fr_next;
            while !fr.is_null() {
                room += (*fr).fr_width - frame_minwidth(fr, ptr::null_mut());
                fr = (*fr).fr_next;
            }
            fr = curfr;
        }

        offset = offset.min(room);
        if offset <= 0 || fr.is_null() {
            return;
        }

        frame_new_width(fr, (*fr).fr_width + offset, left, false);

        if left {
            fr = curfr;
        } else {
            fr = (*curfr).fr_next;
        }
        while !fr.is_null() && offset > 0 {
            let n = frame_minwidth(fr, ptr::null_mut());
            if (*fr).fr_width - offset <= n {
                offset -= (*fr).fr_width - n;
                frame_new_width(fr, n, !left, false);
            } else {
                frame_new_width(fr, (*fr).fr_width - offset, !left, false);
                break;
            }
            if left {
                fr = (*fr).fr_prev;
            } else {
                fr = (*fr).fr_next;
            }
        }
        win_comp_pos();
        redraw_all_later(UPD_NOT_VALID);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_fraction(wp: *mut win_T) {
    unsafe {
        if (*wp).w_view_height > 1 {
            (*wp).w_fraction =
                ((*wp).w_wrow * FRACTION_MULT + FRACTION_MULT / 2) / (*wp).w_view_height;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_fix_scroll(resize: bool) {
    unsafe {
        if *p_spk == b'c' as c_char {
            return;
        }

        skip_update_topline = true;
        let mut wp = firstwin;
        while !wp.is_null() {
            if !(*wp).w_floating && (*wp).w_height != (*wp).w_prev_height {
                (*wp).w_do_win_fix_cursor = true;

                if *p_spk == b's' as c_char
                    && (*wp).w_winrow != (*wp).w_prev_winrow
                    && (*wp).w_botline - 1 <= (*(*wp).w_buffer).b_ml.ml_line_count
                {
                    let diff = ((*wp).w_winrow - (*wp).w_prev_winrow)
                        + ((*wp).w_height - (*wp).w_prev_height);
                    let cursor = (*wp).w_cursor;
                    (*wp).w_cursor.lnum = (*wp).w_botline - 1;

                    if diff > 0 {
                        cursor_down_inner(wp, diff, false);
                    } else {
                        cursor_up_inner(wp, -diff, false);
                    }

                    (*wp).w_fraction = FRACTION_MULT;
                    scroll_to_fraction(wp, (*wp).w_prev_height);
                    (*wp).w_cursor = cursor;
                    (*wp).w_valid &= !VALID_WCOL;
                } else if ptr::eq(wp, curwin) {
                    (*wp).w_valid &= !VALID_CROW;
                }

                invalidate_botline_win(wp);
                validate_botline_win(wp);
            }
            (*wp).w_prev_height = (*wp).w_height;
            (*wp).w_prev_winrow = (*wp).w_winrow;
            wp = (*wp).w_next;
        }
        skip_update_topline = false;
        if (get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) == 0 {
            win_fix_cursor(false);
        } else if resize {
            win_fix_cursor(true);
        }
    }
}

unsafe fn win_fix_cursor(normal: bool) {
    unsafe {
        let wp = curwin;

        if !(*wp).w_do_win_fix_cursor || (*(*wp).w_buffer).b_ml.ml_line_count < (*wp).w_view_height
        {
            return;
        }

        (*wp).w_do_win_fix_cursor = false;
        let so = (OptInt::from((*wp).w_view_height / 2).min(get_scrolloff_value(wp))) as c_int;
        let lnum = (*wp).w_cursor.lnum;

        (*wp).w_cursor.lnum = (*wp).w_topline;
        cursor_down_inner(wp, so, false);
        let top = (*wp).w_cursor.lnum;

        (*wp).w_cursor.lnum = (*wp).w_botline - 1;
        cursor_up_inner(wp, so, false);
        let bot = (*wp).w_cursor.lnum;

        (*wp).w_cursor.lnum = lnum;
        let mut nlnum: linenr_T = 0;
        if lnum > bot && ((*wp).w_botline - (*(*wp).w_buffer).b_ml.ml_line_count) != 1 {
            nlnum = bot;
        } else if lnum < top && (*wp).w_topline != 1 {
            nlnum = if so == (*wp).w_view_height / 2 {
                bot
            } else {
                top
            };
        }

        if nlnum != 0 {
            if normal {
                setmark(b'\'' as c_int);
                (*wp).w_cursor.lnum = nlnum;
            } else {
                (*wp).w_fraction = if nlnum == bot { FRACTION_MULT } else { 0 };
                scroll_to_fraction(wp, (*wp).w_prev_height);
                validate_botline_win(curwin);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_new_height(wp: *mut win_T, mut height: c_int) {
    unsafe {
        height = height.max(0);
        if (*wp).w_height == height {
            return;
        }

        (*wp).w_height = height;
        (*wp).w_pos_changed = true;
        win_set_inner_size(wp, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scroll_to_fraction(wp: *mut win_T, prev_height: c_int) {
    unsafe {
        let height = (*wp).w_view_height;

        if height > 0
            && ((*wp).w_onebuf_opt.wo_scb == 0 || ptr::eq(wp, curwin))
            && (height < (*(*wp).w_buffer).b_ml.ml_line_count || (*wp).w_topline > 1)
        {
            let mut lnum = (*wp).w_cursor.lnum.max(1);
            (*wp).w_wrow = ((*wp).w_fraction * height - 1) / FRACTION_MULT;
            let mut line_size =
                plines_win_col(wp, lnum, (*wp).w_cursor.col as std::ffi::c_long) - 1;
            let mut sline = (*wp).w_wrow - line_size;

            if sline >= 0 {
                let rows = plines_win(wp, lnum, false);
                if sline > (*wp).w_view_height - rows {
                    sline = (*wp).w_view_height - rows;
                    (*wp).w_wrow -= rows - line_size;
                }
            }

            if sline < 0 {
                (*wp).w_wrow = line_size;
                if (*wp).w_wrow >= (*wp).w_view_height && ((*wp).w_view_width - win_col_off(wp)) > 0
                {
                    let mut skipcol: colnr_T = (*wp).w_view_width - win_col_off(wp);
                    (*wp).w_wrow -= 1;
                    while (*wp).w_wrow >= (*wp).w_view_height {
                        skipcol += (*wp).w_view_width - win_col_off(wp) + win_col_off2(wp);
                        (*wp).w_wrow -= 1;
                    }
                    (*wp).w_skipcol = skipcol;
                }
            } else if sline > 0 {
                while sline > 0 && lnum > 1 {
                    hasFolding(wp, lnum, &raw mut lnum, ptr::null_mut());
                    if lnum == 1 {
                        line_size = c_int::from(!decor_conceal_line(wp, lnum - 1, false));
                        sline -= 1;
                        break;
                    }
                    lnum -= 1;
                    if lnum == (*wp).w_topline {
                        line_size = plines_win_nofill(wp, lnum, true) + (*wp).w_topfill;
                    } else {
                        line_size = plines_win(wp, lnum, true);
                    }
                    sline -= line_size;
                }

                if sline < 0 {
                    hasFolding(wp, lnum, ptr::null_mut(), &raw mut lnum);
                    lnum += 1;
                    (*wp).w_wrow -= line_size + sline;
                } else if sline > 0 {
                    lnum = 1;
                    (*wp).w_wrow -= sline;
                }
            }
            set_topline(wp, lnum);
        }

        if ptr::eq(wp, curwin) {
            curs_columns(wp, 0);
        }
        if prev_height > 0 {
            (*wp).w_prev_fraction_row = (*wp).w_wrow;
        }

        redraw_later(wp, UPD_SOME_VALID);
        invalidate_botline_win(wp);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_set_inner_size(wp: *mut win_T, valid_cursor: bool) {
    unsafe {
        let mut width = (*wp).w_width_request;
        if width == 0 {
            width = (*wp).w_width;
        }

        let prev_height = (*wp).w_view_height;
        let mut height = (*wp).w_height_request;
        if height == 0 {
            height = ((*wp).w_height - (*wp).w_winbar_height).max(0);
        }

        if height != prev_height {
            if height > 0 && valid_cursor {
                if ptr::eq(wp, curwin) && (*p_spk == b'c' as c_char || (*wp).w_floating) {
                    validate_cursor(curwin);
                }
                if (*wp).w_view_height != prev_height {
                    return;
                }
                if (*wp).w_wrow != (*wp).w_prev_fraction_row {
                    set_fraction(wp);
                }
            }
            (*wp).w_view_height = height;
            win_comp_scroll(wp);

            if valid_cursor && !exiting && (*p_spk == b'c' as c_char || (*wp).w_floating) {
                if (*wp).w_onebuf_opt.wo_sms == 0 {
                    (*wp).w_skipcol = 0;
                }
                scroll_to_fraction(wp, prev_height);
            }
            redraw_later(wp, UPD_SOME_VALID);
        }

        if width != (*wp).w_view_width {
            (*wp).w_view_width = width;
            (*wp).w_lines_valid = 0;
            if valid_cursor {
                changed_line_abv_curs_win(wp);
                invalidate_botline_win(wp);
                if ptr::eq(wp, curwin) && (*p_spk == b'c' as c_char || (*wp).w_floating) {
                    curs_columns(wp, 1);
                }
            }
            redraw_later(wp, UPD_NOT_VALID);
        }

        if !(*(*wp).w_buffer).terminal.is_null() {
            terminal_check_size((*(*wp).w_buffer).terminal.cast::<Terminal>());
        }

        let float_stl_height = if (*wp).w_floating && (*wp).w_status_height != 0 {
            STATUS_HEIGHT
        } else {
            0
        };
        (*wp).w_height_outer =
            (*wp).w_view_height + win_border_height(wp) + (*wp).w_winbar_height + float_stl_height;
        (*wp).w_width_outer = (*wp).w_view_width + win_border_width(wp);
        (*wp).w_winrow_off = (*wp).w_border_adj[0] + (*wp).w_winbar_height;
        (*wp).w_wincol_off = (*wp).w_border_adj[3];

        if ui_has(kUIMultigrid) {
            ui_call_win_viewport_margins(
                Integer::from((*wp).w_grid_alloc.handle),
                (*wp).handle,
                Integer::from((*wp).w_winrow_off),
                Integer::from((*wp).w_border_adj[2]),
                Integer::from((*wp).w_wincol_off),
                Integer::from((*wp).w_border_adj[1]),
            );
        }

        (*wp).w_redr_status = true;

        if updating_screen {
            win_grid_alloc(wp);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_new_width(wp: *mut win_T, width: c_int) {
    unsafe {
        (*wp).w_width = if width < 0 { 0 } else { width };
        (*wp).w_pos_changed = true;
        win_set_inner_size(wp, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_default_scroll(wp: *mut win_T) -> OptInt {
    unsafe { OptInt::from((*wp).w_view_height / 2).max(1) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_comp_scroll(wp: *mut win_T) {
    unsafe {
        let old_w_p_scr = (*wp).w_onebuf_opt.wo_scr;
        (*wp).w_onebuf_opt.wo_scr = win_default_scroll(wp);

        if (*wp).w_onebuf_opt.wo_scr != old_w_p_scr {
            (*wp).w_onebuf_opt.wo_script_ctx[kWinOptScroll].sc_sid = SID_WINLAYOUT;
            (*wp).w_onebuf_opt.wo_script_ctx[kWinOptScroll].sc_lnum = 0;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn command_height() {
    unsafe {
        let mut old_p_ch = (*curtab).tp_ch_used as c_int;

        let mut frp = (*lastwin_nofloating(ptr::null_mut())).w_frame;
        while (*frp).fr_width != Columns && !(*frp).fr_parent.is_null() {
            frp = (*frp).fr_parent;
        }

        while !(*frp).fr_prev.is_null()
            && (*frp).fr_layout == FR_LEAF
            && (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0
        {
            frp = (*frp).fr_prev;
        }

        while p_ch > OptInt::from(old_p_ch) && command_frame_height {
            if frp.is_null() {
                emsg(gettext(e_noroom.as_ptr()));
                p_ch = OptInt::from(old_p_ch);
                break;
            }
            let h = ((p_ch - OptInt::from(old_p_ch)) as c_int)
                .min((*frp).fr_height - frame_minheight(frp, ptr::null_mut()));
            frame_add_height(frp, -h);
            old_p_ch += h;
            frp = (*frp).fr_prev;
        }
        if p_ch < OptInt::from(old_p_ch) && command_frame_height && !frp.is_null() {
            frame_add_height(frp, (OptInt::from(old_p_ch) - p_ch) as c_int);
        }

        win_comp_pos();
        win_fix_scroll(true);
        cmdline_row = Rows - p_ch as c_int;
        redraw_cmdline = true;

        if msg_scrolled == 0 && full_screen {
            let mut grid: *mut GridView = &raw mut default_gridview;
            if !ui_has(kUIMessages) {
                msg_grid_validate();
                grid = &raw mut msg_grid_adj;
            }
            grid_clear(grid, cmdline_row, Rows, 0, Columns, 0);
            msg_row = cmdline_row;
        }

        (*curtab).tp_ch_used = p_ch;
        min_set_ch = p_ch;
    }
}

unsafe fn frame_add_height(mut frp: *mut frame_T, n: c_int) {
    unsafe {
        frame_new_height(frp, (*frp).fr_height + n, false, false, false);
        loop {
            frp = (*frp).fr_parent;
            if frp.is_null() {
                break;
            }
            (*frp).fr_height += n;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn last_status(morewin: bool) {
    unsafe {
        last_status_rec(
            topframe,
            last_stl_height(morewin) > 0,
            global_stl_height() > 0,
        );
        win_float_anchor_laststatus();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_remove_status_line(wp: *mut win_T, add_hsep: bool) {
    unsafe {
        (*wp).w_status_height = 0;
        if add_hsep {
            (*wp).w_hsep_height = 1;
        } else {
            win_new_height(
                wp,
                (if (*wp).w_floating {
                    (*wp).w_view_height
                } else {
                    (*wp).w_height
                }) + STATUS_HEIGHT,
            );
        }
        comp_col();

        stl_clear_click_defs(
            (*wp).w_status_click_defs.cast::<StlClickDefinition>(),
            (*wp).w_status_click_defs_size,
        );
        xfree((*wp).w_status_click_defs);
        (*wp).w_status_click_defs_size = 0;
        (*wp).w_status_click_defs = ptr::null_mut();
    }
}

unsafe fn find_horizontally_resizable_frame(fr: *mut frame_T) -> *mut frame_T {
    unsafe {
        let mut fp = fr;
        while (*fp).fr_height <= frame_minheight(fp, ptr::null_mut()) {
            if ptr::eq(fp, topframe) {
                return ptr::null_mut();
            }
            if (*(*fp).fr_parent).fr_layout == FR_COL && !(*fp).fr_prev.is_null() {
                fp = (*fp).fr_prev;
            } else {
                fp = (*fp).fr_parent;
            }
        }
        fp
    }
}

unsafe fn resize_frame_for_status(fr: *mut frame_T) -> bool {
    unsafe {
        let wp = (*fr).fr_win;
        let fp = find_horizontally_resizable_frame(fr);

        if fp.is_null() {
            emsg(gettext(e_noroom.as_ptr()));
            false
        } else if !ptr::eq(fp, fr) {
            let old_height = (*fr).fr_height;
            frame_setheight(fr, old_height + 1, false);
            if (*fr).fr_height == old_height {
                emsg(gettext(e_noroom.as_ptr()));
                return false;
            }
            win_comp_pos();
            true
        } else {
            win_new_height(wp, (*wp).w_height - 1);
            true
        }
    }
}

unsafe fn resize_frame_for_winbar(fr: *mut frame_T) -> bool {
    unsafe {
        let old_height = (*fr).fr_height;
        frame_setheight(fr, old_height + 1, false);
        if (*fr).fr_height == old_height {
            emsg(gettext(e_noroom.as_ptr()));
            return false;
        }
        win_comp_pos();
        true
    }
}

unsafe fn last_status_rec(fr: *mut frame_T, statusline: bool, is_stl_global: bool) {
    unsafe {
        if (*fr).fr_layout == FR_LEAF {
            let wp = (*fr).fr_win;
            let is_last = is_bottom_win(wp);

            if is_last {
                if (*wp).w_status_height != 0 && (!statusline || is_stl_global) {
                    win_remove_status_line(wp, false);
                } else if (*wp).w_status_height == 0 && !is_stl_global && statusline {
                    (*wp).w_status_height = STATUS_HEIGHT;
                    if !resize_frame_for_status(fr) {
                        return;
                    }
                    comp_col();
                }
                if ((*wp).w_height - (*wp).w_prev_height).abs() == 1 {
                    (*wp).w_prev_height = (*wp).w_height;
                }
            } else if (*wp).w_status_height != 0 && is_stl_global {
                win_remove_status_line(wp, true);
            } else if (*wp).w_status_height == 0 && !is_stl_global {
                (*wp).w_status_height = STATUS_HEIGHT;
                (*wp).w_hsep_height = 0;
                comp_col();
            }
        } else {
            let mut fp = (*fr).fr_child;
            while !fp.is_null() {
                last_status_rec(fp, statusline, is_stl_global);
                fp = (*fp).fr_next;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_winbar_win(
    wp: *mut win_T,
    make_room: bool,
    valid_cursor: bool,
) -> c_int {
    unsafe {
        let winbar_height = if (*wp).w_floating {
            c_int::from(*(*wp).w_onebuf_opt.wo_wbr != NUL)
        } else {
            c_int::from(*p_wbr != NUL || *(*wp).w_onebuf_opt.wo_wbr != NUL)
        };

        if (*wp).w_winbar_height != winbar_height {
            if winbar_height == 1 && (*wp).w_view_height <= 1 {
                if (*wp).w_floating {
                    emsg(gettext(e_noroom.as_ptr()));
                    return NOTDONE;
                } else if !make_room || !resize_frame_for_winbar((*wp).w_frame) {
                    return FAIL;
                }
            }
            (*wp).w_winbar_height = winbar_height;
            win_set_inner_size(wp, valid_cursor);

            if winbar_height == 0 {
                stl_clear_click_defs(
                    (*wp).w_winbar_click_defs.cast::<StlClickDefinition>(),
                    (*wp).w_winbar_click_defs_size,
                );
                xfree((*wp).w_winbar_click_defs);
                (*wp).w_winbar_click_defs_size = 0;
                (*wp).w_winbar_click_defs = ptr::null_mut();
            }
        }

        OK
    }
}

unsafe fn set_winbar_curtab(make_room: bool, valid_cursor: bool) -> bool {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if set_winbar_win(wp, make_room, valid_cursor) == FAIL {
                return false;
            }
            wp = (*wp).w_next;
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_winbar(make_room: bool) {
    unsafe {
        set_winbar_curtab(make_room, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_winbar_all(make_room: bool) {
    unsafe {
        let _ = set_winbar_curtab(make_room, true);

        let mut tp = first_tabpage;
        while !tp.is_null() {
            if !ptr::eq(tp, curtab) {
                let mut switchwin: CtxSwitch = zeroed();
                if ctx_switch(
                    &raw mut switchwin,
                    (*tp).tp_curwin,
                    tp,
                    ptr::null_mut(),
                    kCtxNoEvents | kCtxNoDisplay,
                ) {
                    let _ = set_winbar_curtab(make_room, false);
                }
                ctx_restore(&raw mut switchwin);
            }
            tp = (*tp).tp_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tabline_height() -> c_int {
    unsafe {
        if ui_has(kUITabline) {
            return 0;
        }
        debug_assert!(!first_tabpage.is_null());
        match p_stal {
            0 => 0,
            1 => c_int::from(!(*first_tabpage).tp_next.is_null()),
            _ => 1,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn global_winbar_height() -> c_int {
    unsafe { c_int::from(*p_wbr != NUL) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn global_stl_height() -> c_int {
    unsafe { if p_ls == 3 { STATUS_HEIGHT } else { 0 } }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn last_stl_height(morewin: bool) -> c_int {
    unsafe {
        if p_ls > 1 || (p_ls == 1 && (morewin || !one_window(firstwin, ptr::null_mut()))) {
            STATUS_HEIGHT
        } else {
            0
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn min_rows(tp: *mut tabpage_T) -> c_int {
    unsafe {
        if firstwin.is_null() {
            return MIN_LINES;
        }

        let mut total = frame_minheight((*tp).tp_topframe.cast(), ptr::null_mut());
        total += tabline_height() + global_stl_height();
        if (if ptr::eq(tp, curtab) {
            p_ch
        } else {
            (*tp).tp_ch_used
        }) > 0
        {
            total += 1;
        }
        total
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn min_rows_for_all_tabpages() -> c_int {
    unsafe {
        if firstwin.is_null() {
            return MIN_LINES;
        }

        let mut total: c_int = 0;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut n = frame_minheight((*tp).tp_topframe.cast(), ptr::null_mut());
            if (if ptr::eq(tp, curtab) {
                p_ch
            } else {
                (*tp).tp_ch_used
            }) > 0
            {
                n += 1;
            }
            total = total.max(n);
            tp = (*tp).tp_next;
        }
        total += tabline_height() + global_stl_height();
        total
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn only_one_window() -> bool {
    unsafe {
        if !(*first_tabpage).tp_next.is_null() {
            return false;
        }

        let mut count: c_int = 0;
        let mut wp = firstwin;
        while !wp.is_null() {
            if !(*wp).w_buffer.is_null()
                && (!((bt_help((*wp).w_buffer) && !bt_help(curbuf))
                    || (*wp).w_floating
                    || (*wp).w_onebuf_opt.wo_pvw != 0)
                    || ptr::eq(wp, curwin))
                && !is_ctx_win(wp)
            {
                count += 1;
            }
            wp = (*wp).w_next;
        }
        count <= 1
    }
}

unsafe fn check_lnums_both(do_curwin: bool, nested: bool) {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if ptr::eq(tp, curtab) {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if (do_curwin || !ptr::eq(wp, curwin)) && ptr::eq((*wp).w_buffer, curbuf) {
                    if !nested {
                        (*wp).w_save_cursor.w_cursor_save = (*wp).w_cursor;
                        (*wp).w_save_cursor.w_topline_save = (*wp).w_topline;
                    }

                    let mut need_adjust = (*wp).w_cursor.lnum > (*curbuf).b_ml.ml_line_count;
                    if need_adjust {
                        (*wp).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
                    }
                    if need_adjust || !nested {
                        (*wp).w_save_cursor.w_cursor_corr = (*wp).w_cursor;
                    }

                    need_adjust = (*wp).w_topline > (*curbuf).b_ml.ml_line_count;
                    if need_adjust {
                        (*wp).w_topline = (*curbuf).b_ml.ml_line_count;
                    }
                    if need_adjust || !nested {
                        (*wp).w_save_cursor.w_topline_corr = (*wp).w_topline;
                    }
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_lnums(do_curwin: bool) {
    unsafe {
        check_lnums_both(do_curwin, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_lnums_nested(do_curwin: bool) {
    unsafe {
        check_lnums_both(do_curwin, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn reset_lnums() {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if ptr::eq(tp, curtab) {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if ptr::eq((*wp).w_buffer, curbuf) {
                    if equalpos((*wp).w_save_cursor.w_cursor_corr, (*wp).w_cursor)
                        && (*wp).w_save_cursor.w_cursor_save.lnum != 0
                    {
                        (*wp).w_cursor = (*wp).w_save_cursor.w_cursor_save;
                    }
                    if (*wp).w_save_cursor.w_topline_corr == (*wp).w_topline
                        && (*wp).w_save_cursor.w_topline_save != 0
                    {
                        (*wp).w_topline = (*wp).w_save_cursor.w_topline_save;
                    }
                    if (*wp).w_save_cursor.w_topline_save > (*(*wp).w_buffer).b_ml.ml_line_count {
                        (*wp).w_valid &= !VALID_TOPLINE;
                    }
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn make_snapshot(idx: c_int) {
    unsafe {
        clear_snapshot(curtab, idx);
        make_snapshot_rec(
            topframe,
            (&raw mut (*curtab).tp_snapshot[idx as usize]).cast::<*mut frame_T>(),
        );
    }
}

unsafe fn make_snapshot_rec(fr: *mut frame_T, frp: *mut *mut frame_T) {
    unsafe {
        *frp = xcalloc(1, size_of::<frame_T>()).cast::<frame_T>();
        (**frp).fr_layout = (*fr).fr_layout;
        (**frp).fr_width = (*fr).fr_width;
        (**frp).fr_height = (*fr).fr_height;
        if !(*fr).fr_next.is_null() {
            make_snapshot_rec((*fr).fr_next, &raw mut (**frp).fr_next);
        }
        if !(*fr).fr_child.is_null() {
            make_snapshot_rec((*fr).fr_child, &raw mut (**frp).fr_child);
        }
        if (*fr).fr_layout == FR_LEAF && ptr::eq((*fr).fr_win, curwin) {
            (**frp).fr_win = curwin;
        }
    }
}

unsafe fn clear_snapshot(tp: *mut tabpage_T, idx: c_int) {
    unsafe {
        clear_snapshot_rec((*tp).tp_snapshot[idx as usize].cast::<frame_T>());
        (*tp).tp_snapshot[idx as usize] = ptr::null_mut();
    }
}

unsafe fn clear_snapshot_rec(fr: *mut frame_T) {
    unsafe {
        if fr.is_null() {
            return;
        }
        clear_snapshot_rec((*fr).fr_next);
        clear_snapshot_rec((*fr).fr_child);
        xfree(fr.cast());
    }
}

unsafe fn get_snapshot_curwin_rec(ft: *mut frame_T) -> *mut win_T {
    unsafe {
        if !(*ft).fr_next.is_null() {
            let wp = get_snapshot_curwin_rec((*ft).fr_next);
            if !wp.is_null() {
                return wp;
            }
        }
        if !(*ft).fr_child.is_null() {
            let wp = get_snapshot_curwin_rec((*ft).fr_child);
            if !wp.is_null() {
                return wp;
            }
        }
        (*ft).fr_win
    }
}

unsafe fn get_snapshot_curwin(idx: c_int) -> *mut win_T {
    unsafe {
        let sn = (*curtab).tp_snapshot[idx as usize].cast::<frame_T>();
        if sn.is_null() {
            return ptr::null_mut();
        }
        get_snapshot_curwin_rec(sn)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn restore_snapshot(idx: c_int, close_curwin: c_int) {
    unsafe {
        let sn = (*curtab).tp_snapshot[idx as usize].cast::<frame_T>();
        if !sn.is_null()
            && (*sn).fr_width == (*topframe).fr_width
            && (*sn).fr_height == (*topframe).fr_height
            && check_snapshot_rec(sn, topframe) == OK
        {
            let wp = restore_snapshot_rec(sn, topframe);
            win_comp_pos();
            if !wp.is_null() && close_curwin != 0 {
                win_goto(wp);
            }
            redraw_all_later(UPD_NOT_VALID);
        }
        clear_snapshot(curtab, idx);
    }
}

unsafe fn check_snapshot_rec(sn: *mut frame_T, fr: *mut frame_T) -> c_int {
    unsafe {
        if (*sn).fr_layout != (*fr).fr_layout
            || (*sn).fr_next.is_null() != (*fr).fr_next.is_null()
            || (*sn).fr_child.is_null() != (*fr).fr_child.is_null()
            || (!(*sn).fr_next.is_null()
                && check_snapshot_rec((*sn).fr_next, (*fr).fr_next) == FAIL)
            || (!(*sn).fr_child.is_null()
                && check_snapshot_rec((*sn).fr_child, (*fr).fr_child) == FAIL)
            || (!(*sn).fr_win.is_null() && !win_valid((*sn).fr_win))
        {
            FAIL
        } else {
            OK
        }
    }
}

unsafe fn restore_snapshot_rec(sn: *mut frame_T, fr: *mut frame_T) -> *mut win_T {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();

        (*fr).fr_height = (*sn).fr_height;
        (*fr).fr_width = (*sn).fr_width;
        if (*fr).fr_layout == FR_LEAF {
            frame_new_height(fr, (*fr).fr_height, false, false, false);
            frame_new_width(fr, (*fr).fr_width, false, false);
            wp = (*sn).fr_win;
        }
        if !(*sn).fr_next.is_null() {
            let wp2 = restore_snapshot_rec((*sn).fr_next, (*fr).fr_next);
            if !wp2.is_null() {
                wp = wp2;
            }
        }
        if !(*sn).fr_child.is_null() {
            let wp2 = restore_snapshot_rec((*sn).fr_child, (*fr).fr_child);
            if !wp2.is_null() {
                wp = wp2;
            }
        }
        wp
    }
}

unsafe fn frame_check_height(topfrp: *const frame_T, height: c_int) -> bool {
    unsafe {
        if (*topfrp).fr_height != height {
            return false;
        }
        if (*topfrp).fr_layout == FR_ROW {
            let mut frp: *const frame_T = (*topfrp).fr_child;
            while !frp.is_null() {
                if (*frp).fr_height != height {
                    return false;
                }
                frp = (*frp).fr_next;
            }
        }
        true
    }
}

unsafe fn frame_check_width(topfrp: *const frame_T, width: c_int) -> bool {
    unsafe {
        if (*topfrp).fr_width != width {
            return false;
        }
        if (*topfrp).fr_layout == FR_COL {
            let mut frp: *const frame_T = (*topfrp).fr_child;
            while !frp.is_null() {
                if (*frp).fr_width != width {
                    return false;
                }
                frp = (*frp).fr_next;
            }
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_colorcolumn(cc: *mut c_char, wp: *mut win_T) -> *const c_char {
    unsafe {
        if !wp.is_null() && (*wp).w_buffer.is_null() {
            return ptr::null();
        }

        let mut s: *mut c_char = (&raw mut empty_string_option).cast::<c_char>();
        if !cc.is_null() {
            s = cc;
        } else if !wp.is_null() {
            s = (*wp).w_onebuf_opt.wo_cc;
        }

        let tw: OptInt = if !wp.is_null() {
            (*(*wp).w_buffer).b_p_tw
        } else {
            0
        };

        let mut count: usize = 0;
        let mut color_cols = [0 as c_int; 256];
        while *s != NUL && count < 255 {
            let mut col: c_int;
            let mut skip = false;
            if *s == b'-' as c_char || *s == b'+' as c_char {
                col = if *s == b'-' as c_char { -1 } else { 1 };
                s = s.add(1);
                if !ascii_isdigit(c_int::from(*s as u8)) {
                    return e_invarg.as_ptr();
                }
                col *= getdigits_int(&raw mut s, true, 0);
                if tw == 0 {
                    skip = true;
                } else {
                    col += tw as c_int;
                    if col < 0 {
                        skip = true;
                    }
                }
            } else if ascii_isdigit(c_int::from(*s as u8)) {
                col = getdigits_int(&raw mut s, true, 0);
            } else {
                return e_invarg.as_ptr();
            }
            if !skip {
                color_cols[count] = col - 1;
                count += 1;
            }
            if *s == NUL {
                break;
            }
            if *s != b',' as c_char {
                return e_invarg.as_ptr();
            }
            s = s.add(1);
            if *s == NUL {
                return e_invarg.as_ptr();
            }
        }

        if wp.is_null() {
            return ptr::null();
        }

        xfree((*wp).w_p_cc_cols.cast());
        if count == 0 {
            (*wp).w_p_cc_cols = ptr::null_mut();
        } else {
            (*wp).w_p_cc_cols = xmalloc(size_of::<c_int>() * (count + 1)).cast::<c_int>();
            color_cols[..count].sort_unstable();

            let mut j: usize = 0;
            for i in 0..count {
                if j == 0 || *(*wp).w_p_cc_cols.add(j - 1) != color_cols[i] {
                    *(*wp).w_p_cc_cols.add(j) = color_cols[i];
                    j += 1;
                }
            }
            *(*wp).w_p_cc_cols.add(j) = -1;
        }

        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_last_winid() -> c_int {
    unsafe { last_win_id }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_locked(wp: *mut win_T) -> c_int {
    unsafe { (*wp).w_locked }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_get_tabwin(id: handle_T, tabnr: *mut c_int, winnr: *mut c_int) {
    unsafe {
        *tabnr = 0;
        *winnr = 0;

        let mut tnum: c_int = 1;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wnum: c_int = 1;
            let mut wp = if ptr::eq(tp, curtab) {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if (*wp).handle == id {
                    if win_has_winnr(wp, tp) {
                        *winnr = wnum;
                        *tabnr = tnum;
                    }
                    return;
                }
                wnum += c_int::from(win_has_winnr(wp, tp));
                wp = (*wp).w_next;
            }
            tnum += 1;
            tp = (*tp).tp_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_ui_flush(validate: bool) {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if ptr::eq(tp, curtab) {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if ((*wp).w_pos_changed || (*wp).w_grid_alloc.pending_comp_index_update)
                    && !(*wp).w_grid_alloc.chars.is_null()
                {
                    if ptr::eq(tp, curtab) {
                        ui_ext_win_position(wp, validate);
                    } else {
                        ui_call_win_hide(Integer::from((*wp).w_grid_alloc.handle));
                        (*wp).w_pos_changed = false;
                    }
                    (*wp).w_grid_alloc.pending_comp_index_update = false;
                }
                if ptr::eq(tp, curtab) {
                    ui_ext_win_viewport(wp);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        pum_pos_ui_flush(false);
        msg_ui_flush();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn lastwin_nofloating(tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        debug_assert!(!ptr::eq(tp, curtab) || tp.is_null());
        let mut res = if !tp.is_null() {
            (*tp).tp_lastwin
        } else {
            lastwin
        };
        while (*res).w_floating {
            res = (*res).w_prev;
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equalpos() {
        let p1 = pos_T {
            lnum: 5,
            col: 10,
            coladd: 0,
        };
        let p2 = pos_T {
            lnum: 5,
            col: 10,
            coladd: 0,
        };
        let p3 = pos_T {
            lnum: 5,
            col: 11,
            coladd: 0,
        };
        assert!(equalpos(p1, p2));
        assert!(!equalpos(p1, p3));
    }

    #[test]
    fn test_window_layout_lock_unlock_and_frames_locked() {
        unsafe {
            assert!(!frames_locked());
            window_layout_lock();
            assert!(split_disallowed > 0);
            assert!(close_disallowed > 0);
            window_layout_unlock();
            assert!(split_disallowed == 0);
            assert!(close_disallowed == 0);
        }
    }

    #[test]
    fn test_frame_list_helpers() {
        unsafe {
            let mut parent: frame_T = zeroed();
            let mut f1: frame_T = zeroed();
            let mut f2: frame_T = zeroed();
            let mut f3: frame_T = zeroed();
            f1.fr_parent = &raw mut parent;
            f2.fr_parent = &raw mut parent;
            f3.fr_parent = &raw mut parent;
            parent.fr_child = &raw mut f1;

            frame_append(&raw mut f1, &raw mut f2);
            assert!(ptr::eq(f1.fr_next, &raw mut f2));
            assert!(ptr::eq(f2.fr_prev, &raw mut f1));

            frame_insert(&raw mut f1, &raw mut f3);
            assert!(ptr::eq(parent.fr_child, &raw mut f3));
            assert!(ptr::eq(f3.fr_next, &raw mut f1));
            assert!(ptr::eq(f1.fr_prev, &raw mut f3));

            frame_remove(&raw mut f1);
            assert!(ptr::eq(f3.fr_next, &raw mut f2));
            assert!(ptr::eq(f2.fr_prev, &raw mut f3));
        }
    }
}
