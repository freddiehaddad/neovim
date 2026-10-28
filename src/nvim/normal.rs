//! Normal and Visual mode command processing (`src/nvim/normal.c`).

use crate::api::private::helpers::{array_obj, boolean_obj, cstr_as_obj, integer_obj};
use crate::api::private::validate::{Array, Object};
use crate::arglist::ML_EMPTY;
use crate::autocmd::{EVENT_CURSORMOVED, EVENT_TEXTCHANGED, apply_autocmds, has_event};
use crate::buffer::{
    GETF_ALT, GETF_SETMARK, MAXLNUM, buf_T, buf_get_changedtick, buf_hide, buflist_getfile,
    fileinfo, kOptJopFlagView, kShmFileinfo, kUIMessages, modifiable,
};
use crate::change::{
    COM_MAX_LEN, MB_MAXCHAR, ML_DEL_MESSAGE, OPENLINE_DO_COM, changed_lines, del_chars,
    deleted_lines, get_leader_len, ins_char, ins_char_bytes, kOptVeFlagOnemore, open_line,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, ascii_iswhite_or_nul, is_special, skipwhite, transchar,
    vim_isprintc, vim_iswordp, vim_strsize,
};
use crate::cmdexpand::{Ctrl_N, Ctrl_P, RE_LAST, SEARCH_OPT, SEARCH_START};
use crate::cmdhist::{HIST_SEARCH, add_to_history, init_history};
use crate::cursor::{
    adjust_cursor_col, check_cursor, check_cursor_col, check_cursor_lnum, clearpos, coladvance,
    coladvance_force, colnr_T, dec_cursor, equalpos, gchar_cursor, get_cursor_line_len,
    get_cursor_line_ptr, get_cursor_pos_len, get_cursor_pos_ptr, getviscol, inc_cursor,
    kOptVeFlagAll, linenr_T, lt, pos_T, set_leftcol,
};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::diff::{diff_move_to, diff_set_topline, ex_diffupdate, nv_diffgetput};
use crate::digraph::{get_digraph, kBoolVarFalse, kBoolVarTrue};
use crate::drawline::{SMT_ALL, VALID_CROW, VALID_WCOL};
use crate::drawscreen::{
    GETVCOL_END_EXCL_LBR, HLF_MSG, UPD_CLEAR, UPD_INVERTED, UPD_NOT_VALID, UPD_SOME_VALID,
    UPD_VALID, conceal_check_cursor_line, kOptVeFlagBlock, redraw_curbuf_later, redraw_later,
    redraw_statuslines, setcursor, show_cursor_info_later, showmode, update_screen,
    win_cursorline_standout,
};
use crate::errors::*;
use crate::eval::buffer::prompt_invoke_callback;
use crate::eval::funcs::{kMTCharWise, kMTLineWise, kMTUnknown};
use crate::eval::kMarkAll;
use crate::eval::typval::{TypvalVval, VarType, tv_clear, typval_T, varnumber_T};
use crate::eval::vars::{VV_OP, set_reg_var, set_vcount, set_vim_var_string};
use crate::ex_cmds::{
    B_IMODE_LMAP, BL_FIX, BL_SOL, BL_WHITE, Ctrl_C, Ctrl_E, Ctrl_Y, ECMD_HIDE, ECMD_LAST, do_ascii,
    do_ecmd,
};
use crate::ex_cmds2::autowrite;
use crate::ex_docmd::{
    ACTION_GOTO, ACTION_SHOW, ACTION_SHOW_ALL, DOCMD_KEEPLINE, DT_POP, FIND_ANY, FIND_DEFINE,
    SEARCH_MSG, do_cmdline, do_cmdline_cmd, do_sleep, update_topline_cursor,
};
use crate::ex_eval::discard_current_exception;
use crate::ex_getln::{
    Ctrl__, Ctrl_A, Ctrl_B, Ctrl_BSL, Ctrl_D, Ctrl_F, Ctrl_G, Ctrl_HAT, Ctrl_K, Ctrl_L, Ctrl_O,
    Ctrl_Q, Ctrl_R, Ctrl_RSB, Ctrl_T, Ctrl_U, Ctrl_W, Ctrl_Z, K_EVENT, K_LUA, K_SELECT, VSE_NONE,
    cmdwin_do_action, compute_cmdrow, curbuf_locked, getcmdline, text_locked, text_locked_msg,
    vim_strsave_fnameescape,
};
use crate::extmark::{kFalse, kNone, kTrue};
use crate::file_search::grab_file_name;
use crate::fileio::check_timestamps;
use crate::fold::{
    clearFolding, closeFold, closeFoldRecurse, deleteFold, foldAdjustVisual, foldCheckClose,
    foldManualAllowed, foldMoveTo, foldOpenCursor, foldUpdateAfterInsert, foldmethodIsManual,
    foldmethodIsMarker, getDeepestNesting, hasAnyFolding, hasFolding, openFold, openFoldRecurse,
};
use crate::globals::*;
use crate::grid::{grid_line_flush, grid_line_puts, grid_line_start};
use crate::help::ex_help;
use crate::indent_c::{FM_BACKWARD, FM_SKIPCOMM, check_linecomment};
use crate::input::{
    K_PASTE_START, beep_flush, char_avail, gotchars_ignore, kInsNone, langmap_adjust,
    map_execute_lua, paste_repeat, plain_vgetc, prep_redo, readbuf1_empty, redo_append_char,
    redo_append_str, requeue_key, safe_vgetc, start_redo, stuff_empty, stuffReadbuff,
    stuffcharReadbuff, stuffnumReadbuff, typebuf_maplen, typebuf_typed, vgetc, vpeekc, vungetc,
};
use crate::input_cmdatom::{
    CmdSpec, InsSession, atom_capture_op, atom_cmd_end, atom_cmd_spec, atom_cmd_start,
    atom_ins_end, atom_ins_start, atom_macro_start, atom_stuff_start, atom_suppress,
    atom_visual_end, cmdarg_T, kVInsNone,
};
use crate::insert::{
    Ctrl_S, Ctrl_X, MOD_MASK_CTRL, beginline, cursor_down, cursor_down_inner, cursor_up,
    cursor_up_inner, edit, get_literal, ins_copychar, kOptFdoFlagAll, kOptFdoFlagHor, oneleft,
    oneright, prompt_curpos_editable, set_last_insert,
};
use crate::insexpand::Ctrl_I;
use crate::keycodes::{
    CAR, DEL, ESC, K_BS, K_C_END, K_C_HOME, K_C_LEFT, K_C_RIGHT, K_COMMAND, K_DEL, K_DOWN, K_END,
    K_F1, K_HELP, K_HOME, K_IGNORE, K_INS, K_KDEL, K_KEND, K_KENTER, K_KHOME, K_KINS, K_KPAGEDOWN,
    K_KPAGEUP, K_LEFT, K_LEFTDRAG, K_LEFTMOUSE, K_LEFTMOUSE_NM, K_LEFTRELEASE, K_LEFTRELEASE_NM,
    K_MIDDLEDRAG, K_MIDDLEMOUSE, K_MIDDLERELEASE, K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE,
    K_MOUSERIGHT, K_MOUSEUP, K_NOP, K_PAGEDOWN, K_PAGEUP, K_RIGHT, K_RIGHTDRAG, K_RIGHTMOUSE,
    K_RIGHTRELEASE, K_S_DOWN, K_S_END, K_S_HOME, K_S_LEFT, K_S_RIGHT, K_S_UP, K_UNDO, K_UP,
    K_X1DRAG, K_X1MOUSE, K_X1RELEASE, K_X2DRAG, K_X2MOUSE, K_X2RELEASE, K_XF1, K_ZERO,
    MOD_MASK_SHIFT, NL, TAB, simplify_key,
};
use crate::lua::executor::nlua_call_typval;
use crate::mapping::add_map;
use crate::mark::{
    MarkMove, MarkMoveRes, checkpcmark, fmark_T, get_changelist, get_jumplist, getnextmark,
    kMarkBeginLine, kMarkChangedCursor, kMarkChangedLine, kMarkContext, kMarkJumpList,
    kMarkMoveFailed, kMarkMoveSuccess, kMarkSetView, kMarkSwitchedBuf, mark_get, mark_mb_adjustpos,
    mark_move_to, pos_to_mark, setmark, setpcmark,
};
use crate::r#match::{HLF_L, HLF_LC};
use crate::math::{FAIL, OK, vim_append_digit_int};
use crate::mbyte::{
    GraphemeState, mb_adjust_cursor, mb_charlen, mb_check_adjust_col, mb_get_class, mb_prevptr,
    show_utf8, utf_byte2len, utf_char2bytes, utf_char2len, utf_find_illegal, utf_head_off,
    utf_iscomposing, utf_ptr2cells, utf_ptr2char, utfc_ptr2len,
};
use crate::mcursor::{
    mc_buf_has_cursors, mc_counter, mc_follow_set, mc_replaying, mc_showcmd, mc_toggle, mc_zq_start,
};
use crate::memline::{goto_byte, inc, ml_delete_flags, ml_get, ml_get_buf, ml_get_len, ml_get_pos};
use crate::memory::xstrlcpy;
use crate::message::{
    NIL_OBJ, emsg, may_clear_sb_text, messaging, msg, msg_delay, msg_ext_set_trigger,
    msg_grid_validate, show_sb_text, wait_return,
};
use crate::mouse::{
    MSCR_DOWN, MSCR_LEFT, MSCR_RIGHT, MSCR_UP, do_mouse, nv_mouse, nv_mousescroll, setmouse,
};
use crate::r#move::{
    adjust_skipcol, changed_window_setting, cursor_correct, do_check_cursorbind, pagescroll,
    scroll_cursor_bot, scroll_cursor_halfway, scroll_cursor_top, scroll_redraw, scrolldown,
    scrollup, update_curswant, update_curswant_force, update_topline, validate_botline_win,
    validate_cheight, validate_cursor, validate_virtcol, win_col_off, win_col_off2,
};
use crate::ops::{
    OP_CHANGE, OP_COLON, OP_DELETE, OP_FOLD, OP_FORMAT, OP_LOWER, OP_LSHIFT, OP_MCURSOR, OP_NOP,
    OP_NR_ADD, OP_NR_SUB, OP_ROT13, OP_RSHIFT, OP_TILDE, OP_UPPER, OP_YANK, adjust_cursor_eol,
    cursor_pos_info, do_join, do_pending_operator, get_extra_op_char, get_op_char, get_op_type,
    op_addsub, op_is_change, swapchar,
};
use crate::option::{
    OPT_LOCAL, OptInt, copy_option_part, get_showbreak_value, get_sidescrolloff_value,
    get_ve_flags, set_option_value, shortmess,
};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::input::line_breakcheck;
use crate::os::time::{os_time, time_t};
use crate::plines::{
    getvcol, getvcols, getvvcol, linetabsize, plines_m_win_fill, plines_win, win_get_fill,
};
use crate::profile::{time_finish, time_msg};
use crate::quickfix::qf_view_result;
use crate::regexp::{Ctrl_H, Ctrl_V, MAXCOL};
use crate::register::{
    PUT_BLOCK_INNER, PUT_CURSEND, PUT_FIXINDENT, PUT_LINE, PUT_LINE_FORWARD, PUT_LINE_SPLIT,
    copy_register, do_execreg, do_put, do_record, get_default_register_name, get_expr_register,
    valid_yank_reg, yankreg_T,
};
use crate::search::{
    BACKWARD, FORWARD, current_search, do_search, find_pattern_in_path, findmatchlimit,
    reset_search_dir, searchc, searchit, searchit_arg_T,
};
use crate::spell::spell_move_to;
use crate::spellfile::{SPELL_ADD_BAD, SPELL_ADD_GOOD, spell_add_word};
use crate::spellsuggest::spell_suggest;
use crate::state::{
    MODE_INSERT, MODE_LANGMAP, MODE_LREPLACE, MODE_NORMAL, MODE_NORMAL_BUSY, MODE_REPLACE,
    EditorState, MODE_SELECT, MODE_TERMINAL, get_real_state, may_trigger_modechanged,
    may_trigger_safestate, state_enter, state_handle_k_event, state_no_longer_safe, virtual_active,
};
use crate::statusline::{draw_tabline, win_redr_status};
use crate::strings::{
    tr, vim_snprintf, vim_strchr, vim_strsave_shellescape,
};
use crate::syntax::syn_stack_free_all;
use crate::tag::do_tag;
use crate::terminal::terminal_check_refresh;
use crate::textformat::{auto_format, has_format_option, oparg_T};
use crate::textobject::{
    bck_word, bckend_word, current_block, current_line, current_par, current_quote, current_sent,
    current_tagblock, current_word, end_word, findpar, findsent, fwd_word,
};
use crate::ui::{
    ui_call_msg_showcmd, ui_cursor_shape, ui_cursor_shape_no_check_conceal, ui_flush, ui_has,
    vim_beep,
};
use crate::undo::{
    anyBufIsChanged, curbufIsChanged, u_clearline, u_redo, u_save, u_save_cursor, u_savesub,
    u_undo, u_undoline, undo_time,
};
use crate::window::{
    check_can_set_curbuf_disabled, do_window, goto_tabpage, goto_tabpage_lastused,
    may_make_initial_scroll_size_snapshot, may_trigger_win_scrolled_resized, set_fraction, win_T,
    win_setheight,
};
use std::ffi::{CStr, c_char, c_int, c_long, c_longlong, c_uint};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct visualinfo_T {
    pub vi_start: pos_T,
    pub vi_end: pos_T,
    pub vi_mode: c_int,
    pub vi_curswant: i32,
}

impl visualinfo_T {
    pub const fn new() -> Self {
        Self {
            vi_start: pos_T::new(),
            vi_end: pos_T::new(),
            vi_mode: 0,
            vi_curswant: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VisualExtent {
    pub mode: c_int,
    pub line_count: linenr_T,
    pub vcol: colnr_T,
}

impl VisualExtent {
    pub const fn new() -> Self {
        Self {
            mode: 0,
            line_count: 0,
            vcol: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VisualState {
    pub start: pos_T,
    pub active: bool,
    pub select: bool,
    pub select_reg: c_int,
    pub select_exclu_adj: bool,
    pub restart_select: c_int,
    pub need_end: bool,
    pub reselect: c_int,
    pub mode: c_int,
    pub resel: VisualExtent,
}

impl VisualState {
    pub const fn new() -> Self {
        Self {
            start: pos_T::new(),
            active: false,
            select: false,
            select_reg: 0,
            select_exclu_adj: false,
            restart_select: 0,
            need_end: false,
            reselect: 0,
            mode: 0,
            resel: VisualExtent::new(),
        }
    }
}

pub const FIND_IDENT: c_int = 1;
pub const FIND_STRING: c_int = 2;
pub const FIND_EVAL: c_int = 4;

pub const NV_NCH: u16 = 0x01;
pub const NV_NCH_NOP: u16 = 0x02 | NV_NCH;
pub const NV_NCH_ALW: u16 = 0x04 | NV_NCH;
pub const NV_LANG: u16 = 0x08;
pub const NV_SS: u16 = 0x10;
pub const NV_SSS: u16 = 0x20;
pub const NV_STS: u16 = 0x40;
pub const NV_RL: u16 = 0x80;
pub const NV_KEEPREG: u16 = 0x100;
pub const NV_NCW: u16 = 0x200;
pub const NV_NCH_ARG: u16 = 0x400;
pub const NV_MOTION: u16 = 0x800;
pub const NV_JUMP: u16 = 0x1000;

pub const CA_COMMAND_BUSY: c_int = 1;
pub const CA_NO_ADJ_OP_END: c_int = 2;

pub const REPLACE_CR_NCHAR: c_int = -1;
pub const REPLACE_NL_NCHAR: c_int = -2;

pub const SHOWCMD_COLS: usize = 10;
pub const SHOWCMD_BUFLEN: usize = SHOWCMD_COLS + 1 + 30;

const POUND: c_int = 0xA3;

pub(crate) const kOptFoldenable: OptIndex = 103;
pub(crate) const kOptFoldlevel: OptIndex = 106;

const kCpoChangew: c_int = b'_' as c_int;
const kCpoDigraph: c_int = b'D' as c_int;
pub(crate) const kCpoMatch: c_int = b'%' as c_int;

const kFoOpenComs: c_int = b'o' as c_int;

pub(crate) const kShmSearchcount: c_int = b'S' as c_int;

const kOptBoFlagEsc: c_uint = 0x80;
pub(crate) const kOptCbFlagUnnamed: c_uint = 0x01;
pub(crate) const kOptCbFlagUnnamedplus: c_uint = 0x02;

const kOptFdoFlagBlock: c_uint = 0x02;
const kOptFdoFlagMark: c_uint = 0x08;
const kOptFdoFlagPercent: c_uint = 0x10;
pub(crate) const kOptFdoFlagSearch: c_uint = 0x40;
const kOptFdoFlagJump: c_uint = 0x400;

const KMarkNoContext: c_int = 4;

pub(crate) const FM_FORWARD: c_int = 0x02;
pub(crate) const SEARCH_REV: c_int = 0x01;
pub(crate) const SEARCH_ECHO: c_int = 0x02;
pub(crate) const SEARCH_MARK: c_int = 0x200;
pub(crate) const SMT_BAD: c_int = 1;
pub(crate) const SMT_RARE: c_int = 2;

type nv_func_T = fn(cap: &mut cmdarg_T);

fn nv_scroll_line_cmd(cap: &mut cmdarg_T) {
    nv_scroll_line(cap)
}

fn nv_mousescroll_cmd(cap: &mut cmdarg_T) {
    nv_mousescroll(cap)
}

fn nv_mouse_cmd(cap: &mut cmdarg_T) {
    nv_mouse(cap)
}

#[repr(C)]
struct NormalState {
    command_finished: bool,
    ctrl_w: bool,
    need_flushbuf: bool,
    set_prevcount: bool,
    previous_got_int: bool,
    toplevel: bool,
    oa: oparg_T,
    ca: cmdarg_T,
    mapped_len: c_int,
    old_mapped_len: c_int,
    idx: c_int,
    c: c_int,
    old_col: c_int,
    old_pos: pos_T,
}

impl NormalState {
    const fn new() -> Self {
        Self {
            command_finished: false,
            ctrl_w: false,
            need_flushbuf: false,
            set_prevcount: false,
            previous_got_int: false,
            toplevel: false,
            oa: oparg_T::new(),
            ca: cmdarg_T::new(),
            mapped_len: 0,
            old_mapped_len: 0,
            idx: 0,
            c: 0,
            old_col: 0,
            old_pos: pos_T::new(),
        }
    }
}

impl EditorState for NormalState {
    fn check(&mut self) -> c_int {
        normal_check(self)
    }

    fn execute(&mut self, key: c_int) -> c_int {
        normal_execute(self, key)
    }
}

impl Default for NormalState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct nv_cmd {
    cmd_char: c_int,
    cmd_func: nv_func_T,
    cmd_flags: u16,
    cmd_arg: i16,
}

struct NormalModuleState {
    VIsual_mode_orig: c_int,
    current_oap: *mut oparg_T,
    old_showcmd_buf: [c_char; SHOWCMD_BUFLEN],
    showcmd_is_clear: bool,
    showcmd_visual: bool,
    do_check_scrollbind_old_curwin: *mut win_T,
    do_check_scrollbind_old_vtopline: linenr_T,
    do_check_scrollbind_old_buf: *mut buf_T,
    do_check_scrollbind_old_leftcol: colnr_T,
    nv_cmd_idx: [i16; NV_CMDS_SIZE],
    nv_max_linear: c_int,
}

impl NormalModuleState {
    const fn new() -> Self {
        Self {
            VIsual_mode_orig: 0,
            current_oap: ptr::null_mut(),
            old_showcmd_buf: [0; SHOWCMD_BUFLEN],
            showcmd_is_clear: true,
            showcmd_visual: false,
            do_check_scrollbind_old_curwin: ptr::null_mut(),
            do_check_scrollbind_old_vtopline: 0,
            do_check_scrollbind_old_buf: ptr::null_mut(),
            do_check_scrollbind_old_leftcol: 0,
            nv_cmd_idx: [0; NV_CMDS_SIZE],
            nv_max_linear: 0,
        }
    }
}

static NORM_STATE: GlobalState<NormalModuleState> = GlobalState::new(NormalModuleState::new());

#[inline]
fn norm_st() -> &'static mut NormalModuleState {
    unsafe { &mut *NORM_STATE.as_ptr() }
}

#[inline]
pub fn lineempty(lnum: linenr_T) -> bool {
    unsafe { *ml_get(lnum) == NUL }
}

use crate::buffer::{bt_cmdwin, bt_prompt, bt_quickfix};
use crate::highlight::{hl_attr, win_hl_attr};

#[inline]
fn normal_state_init(s: &mut NormalState) {
    *s = NormalState::new();
}

const NV_CMDS_SIZE: usize = 188;

const nv_cmds: [nv_cmd; NV_CMDS_SIZE] = [
    nv_cmd {
        cmd_char: NUL as c_int,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_A,
        cmd_func: nv_addsub,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_B,
        cmd_func: nv_page,
        cmd_flags: NV_STS,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: Ctrl_C,
        cmd_func: nv_esc,
        cmd_flags: 0,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: Ctrl_D,
        cmd_func: nv_halfpage,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_E,
        cmd_func: nv_scroll_line_cmd,
        cmd_flags: 0,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: Ctrl_F,
        cmd_func: nv_page,
        cmd_flags: NV_STS,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: Ctrl_G,
        cmd_func: nv_ctrlg,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_H,
        cmd_func: nv_ctrlh,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_I,
        cmd_func: nv_pcmark,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: NL,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_K,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_L,
        cmd_func: nv_clear,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: CAR,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: Ctrl_N,
        cmd_func: nv_down,
        cmd_flags: NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_O,
        cmd_func: nv_ctrlo,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_P,
        cmd_func: nv_up,
        cmd_flags: NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_Q,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_R,
        cmd_func: nv_redo_or_register,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_S,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_T,
        cmd_func: nv_tagpop,
        cmd_flags: NV_NCW | NV_JUMP,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_U,
        cmd_func: nv_halfpage,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_V,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'V' as c_int,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'v' as c_int,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_W,
        cmd_func: nv_window,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_X,
        cmd_func: nv_addsub,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_Y,
        cmd_func: nv_scroll_line_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_Z,
        cmd_func: nv_suspend,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: ESC,
        cmd_func: nv_esc,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_BSL,
        cmd_func: nv_normal,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_RSB,
        cmd_func: nv_ident,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl_HAT,
        cmd_func: nv_hat,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: Ctrl__,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b' ' as c_int,
        cmd_func: nv_right,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'!' as c_int,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'"' as c_int,
        cmd_func: nv_regname,
        cmd_flags: NV_NCH_NOP | NV_NCH_ARG | NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'#' as c_int,
        cmd_func: nv_ident,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'$' as c_int,
        cmd_func: nv_dollar,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'%' as c_int,
        cmd_func: nv_percent,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'&' as c_int,
        cmd_func: nv_optrans,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'\'' as c_int,
        cmd_func: nv_gomark,
        cmd_flags: NV_NCH_ALW | NV_NCH_ARG,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'(' as c_int,
        cmd_func: nv_brace,
        cmd_flags: NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: b')' as c_int,
        cmd_func: nv_brace,
        cmd_flags: NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: b'*' as c_int,
        cmd_func: nv_ident,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'+' as c_int,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b',' as c_int,
        cmd_func: nv_csearch,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'-' as c_int,
        cmd_func: nv_up,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'.' as c_int,
        cmd_func: nv_dot,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'/' as c_int,
        cmd_func: nv_search,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'0' as c_int,
        cmd_func: nv_beginline,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'1' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'2' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'3' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'4' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'5' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'6' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'7' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'8' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'9' as c_int,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b':' as c_int,
        cmd_func: nv_colon,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b';' as c_int,
        cmd_func: nv_csearch,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'<' as c_int,
        cmd_func: nv_operator,
        cmd_flags: NV_RL,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'=' as c_int,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'>' as c_int,
        cmd_func: nv_operator,
        cmd_flags: NV_RL,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'?' as c_int,
        cmd_func: nv_search,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'@' as c_int,
        cmd_func: nv_at,
        cmd_flags: NV_NCH_NOP | NV_NCH_ARG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'A' as c_int,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'B' as c_int,
        cmd_func: nv_bck_word,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'C' as c_int,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'D' as c_int,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'E' as c_int,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'F' as c_int,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: b'G' as c_int,
        cmd_func: nv_goto,
        cmd_flags: NV_JUMP,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'H' as c_int,
        cmd_func: nv_scroll,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'I' as c_int,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'J' as c_int,
        cmd_func: nv_join,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'K' as c_int,
        cmd_func: nv_ident,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'L' as c_int,
        cmd_func: nv_scroll,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'M' as c_int,
        cmd_func: nv_scroll,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'N' as c_int,
        cmd_func: nv_next,
        cmd_flags: NV_MOTION,
        cmd_arg: SEARCH_REV as i16,
    },
    nv_cmd {
        cmd_char: b'O' as c_int,
        cmd_func: nv_open,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'P' as c_int,
        cmd_func: nv_put,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'Q' as c_int,
        cmd_func: nv_Q,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'R' as c_int,
        cmd_func: nv_Replace,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'S' as c_int,
        cmd_func: nv_subst,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'T' as c_int,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: b'U' as c_int,
        cmd_func: nv_Undo,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'W' as c_int,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: b'X' as c_int,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'Y' as c_int,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'Z' as c_int,
        cmd_func: nv_Zet,
        cmd_flags: NV_NCH_NOP | NV_NCW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'[' as c_int,
        cmd_func: nv_brackets,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: b'\\' as c_int,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b']' as c_int,
        cmd_func: nv_brackets,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: b'^' as c_int,
        cmd_func: nv_beginline,
        cmd_flags: NV_MOTION,
        cmd_arg: (BL_WHITE | BL_FIX) as i16,
    },
    nv_cmd {
        cmd_char: b'_' as c_int,
        cmd_func: nv_lineop,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'`' as c_int,
        cmd_func: nv_gomark,
        cmd_flags: NV_NCH_ALW | NV_NCH_ARG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'a' as c_int,
        cmd_func: nv_edit,
        cmd_flags: NV_NCH,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'b' as c_int,
        cmd_func: nv_bck_word,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'c' as c_int,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'd' as c_int,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'e' as c_int,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'f' as c_int,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: b'g' as c_int,
        cmd_func: nv_g_cmd,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'h' as c_int,
        cmd_func: nv_left,
        cmd_flags: NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'i' as c_int,
        cmd_func: nv_edit,
        cmd_flags: NV_NCH,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'j' as c_int,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'k' as c_int,
        cmd_func: nv_up,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'l' as c_int,
        cmd_func: nv_right,
        cmd_flags: NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'm' as c_int,
        cmd_func: nv_mark,
        cmd_flags: NV_NCH_NOP | NV_NCH_ARG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'n' as c_int,
        cmd_func: nv_next,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'o' as c_int,
        cmd_func: nv_open,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'p' as c_int,
        cmd_func: nv_put,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'q' as c_int,
        cmd_func: nv_q,
        cmd_flags: NV_NCH | NV_NCH_ARG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'r' as c_int,
        cmd_func: nv_replace,
        cmd_flags: NV_NCH_NOP | NV_LANG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b's' as c_int,
        cmd_func: nv_subst,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b't' as c_int,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: b'u' as c_int,
        cmd_func: nv_undo,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'w' as c_int,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'x' as c_int,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'y' as c_int,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'z' as c_int,
        cmd_func: nv_zet,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'{' as c_int,
        cmd_func: nv_findpar,
        cmd_flags: NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: b'|' as c_int,
        cmd_func: nv_pipe,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: b'}' as c_int,
        cmd_func: nv_findpar,
        cmd_flags: NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: b'~' as c_int,
        cmd_func: nv_tilde,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: POUND,
        cmd_func: nv_ident,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_MOUSEUP,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_UP as i16,
    },
    nv_cmd {
        cmd_char: K_MOUSEDOWN,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_DOWN as i16,
    },
    nv_cmd {
        cmd_char: K_MOUSELEFT,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_LEFT as i16,
    },
    nv_cmd {
        cmd_char: K_MOUSERIGHT,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_RIGHT as i16,
    },
    nv_cmd {
        cmd_char: K_LEFTMOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_LEFTMOUSE_NM,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_LEFTDRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_LEFTRELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_LEFTRELEASE_NM,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_MOUSEMOVE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_MIDDLEMOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_MIDDLEDRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_MIDDLERELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_RIGHTMOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_RIGHTDRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_RIGHTRELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_X1MOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_X1DRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_X1RELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_X2MOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_X2DRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_X2RELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_IGNORE,
        cmd_func: nv_ignore,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_NOP,
        cmd_func: nv_nop,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_INS,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_KINS,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_BS,
        cmd_func: nv_ctrlh,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_UP,
        cmd_func: nv_up,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_S_UP,
        cmd_func: nv_page,
        cmd_flags: NV_SS,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: K_DOWN,
        cmd_func: nv_down,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_S_DOWN,
        cmd_func: nv_page,
        cmd_flags: NV_SS,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: K_LEFT,
        cmd_func: nv_left,
        cmd_flags: NV_SSS | NV_STS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_S_LEFT,
        cmd_func: nv_bck_word,
        cmd_flags: NV_SS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_C_LEFT,
        cmd_func: nv_bck_word,
        cmd_flags: NV_SSS | NV_RL | NV_STS | NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: K_RIGHT,
        cmd_func: nv_right,
        cmd_flags: NV_SSS | NV_STS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_S_RIGHT,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_SS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_C_RIGHT,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_SSS | NV_RL | NV_STS | NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: K_PAGEUP,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: K_KPAGEUP,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: BACKWARD as i16,
    },
    nv_cmd {
        cmd_char: K_PAGEDOWN,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: K_KPAGEDOWN,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: FORWARD as i16,
    },
    nv_cmd {
        cmd_char: K_END,
        cmd_func: nv_end,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_KEND,
        cmd_func: nv_end,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_S_END,
        cmd_func: nv_end,
        cmd_flags: NV_SS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_C_END,
        cmd_func: nv_end,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 1,
    },
    nv_cmd {
        cmd_char: K_HOME,
        cmd_func: nv_home,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_KHOME,
        cmd_func: nv_home,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_S_HOME,
        cmd_func: nv_home,
        cmd_flags: NV_SS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_C_HOME,
        cmd_func: nv_goto,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_DEL,
        cmd_func: nv_abbrev,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_KDEL,
        cmd_func: nv_abbrev,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_UNDO,
        cmd_func: nv_kundo,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_HELP,
        cmd_func: nv_help,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_F1,
        cmd_func: nv_help,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_XF1,
        cmd_func: nv_help,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_SELECT,
        cmd_func: nv_select,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_PASTE_START,
        cmd_func: nv_paste,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_EVENT,
        cmd_func: nv_event,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_COMMAND,
        cmd_func: nv_colon,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    nv_cmd {
        cmd_char: K_LUA,
        cmd_func: nv_colon,
        cmd_flags: 0,
        cmd_arg: 0,
    },
];

static e_changelist_is_empty: &CStr = c"E664: Changelist is empty";
static e_cmdline_window_already_open: &CStr = c"E1292: Command-line window is already open";

/// Initialize the norm_st().nv_cmd_idx[] table.
pub fn init_normal_cmds() {
    unsafe {
        assert!(NV_CMDS_SIZE <= i16::MAX as usize);
        let idx_ptr = (&raw mut norm_st().nv_cmd_idx).cast::<i16>();

        for i in 0..(NV_CMDS_SIZE as i16) {
            *idx_ptr.add(i as usize) = i;
        }

        let slice = std::slice::from_raw_parts_mut(idx_ptr, NV_CMDS_SIZE);
        slice.sort_unstable_by_key(|&s| nv_cmds[s as usize].cmd_char.abs());

        let mut i: i16 = 0;
        while (i as usize) < NV_CMDS_SIZE {
            if c_int::from(i) != nv_cmds[*idx_ptr.add(i as usize) as usize].cmd_char {
                break;
            }
            i += 1;
        }
        norm_st().nv_max_linear = c_int::from(i) - 1;
    }
}

/// True if command `cmdchar` has any of the given NV_XX `flags`.
pub fn nv_is(cmdchar: c_int, flags: u16) -> bool {
    let idx = find_command(cmdchar);
        idx >= 0 && (nv_cmds[idx as usize].cmd_flags & flags) != 0
}

fn find_command(mut cmdchar: c_int) -> c_int {
    unsafe {
        if cmdchar >= 0x100 {
            return -1;
        }
        if cmdchar < 0 {
            cmdchar = -cmdchar;
        }
        assert!(norm_st().nv_max_linear < NV_CMDS_SIZE as c_int);
        let idx_ptr = (&raw const norm_st().nv_cmd_idx).cast::<i16>();
        if cmdchar <= norm_st().nv_max_linear {
            return c_int::from(*idx_ptr.add(cmdchar as usize));
        }

        let mut bot = norm_st().nv_max_linear + 1;
        let mut top = NV_CMDS_SIZE as c_int - 1;
        let mut idx = -1;
        while bot <= top {
            let i = (top + bot) / 2;
            let mut c = nv_cmds[*idx_ptr.add(i as usize) as usize].cmd_char;
            if c < 0 {
                c = -c;
            }
            if cmdchar == c {
                idx = c_int::from(*idx_ptr.add(i as usize));
                break;
            }
            if cmdchar > c {
                bot = i + 1;
            } else {
                top = i - 1;
            }
        }
        idx
    }
}

fn check_text_locked(oap: *mut oparg_T) -> bool {
    unsafe {
        if !text_locked() {
            return false;
        }
        if !oap.is_null() {
            clearopbeep(&mut *oap);
        }
        text_locked_msg();
        true
    }
}

pub fn check_text_or_curbuf_locked(oap: *mut oparg_T) -> bool {
    unsafe {
        if check_text_locked(oap) {
            return true;
        }
        if !curbuf_locked() {
            return false;
        }
        if !oap.is_null() {
            clearop(&mut *oap);
        }
        true
    }
}

/// Check if an operator was started but not finished yet.
pub fn op_pending() -> bool {
    unsafe {
        !(!norm_st().current_oap.is_null()
            && !g_st().finish_op
            && (*norm_st().current_oap).prev_opcount == 0
            && (*norm_st().current_oap).prev_count0 == 0
            && (*norm_st().current_oap).op_type == OP_NOP
            && (*norm_st().current_oap).regname == NUL as c_int)
    }
}

/// Normal state entry point: the main loop. Never returns.
pub fn normal_enter() {
    let mut state = NormalState::new();
        normal_state_init(&mut state);
        let prev_oap = norm_st().current_oap;
        norm_st().current_oap = &raw mut state.oa;
        state.toplevel = true;
        state_enter(&mut state);
        norm_st().current_oap = prev_oap;
}

fn normal_prepare(s: &mut NormalState) {
    s.ca = cmdarg_T::new();
        s.ca.oap = &mut s.oa;

        s.ca.opcount = g_st().opcount;

        let c = g_st().finish_op;
        g_st().finish_op = s.oa.op_type != OP_NOP;
        if g_st().finish_op != c {
            ui_cursor_shape();
        }
        may_trigger_modechanged();

        s.set_prevcount = false;
        if !g_st().finish_op && s.oa.regname == 0 {
            s.ca.opcount = 0;
            s.set_prevcount = true;
        }

        if s.oa.prev_opcount > 0 || s.oa.prev_count0 > 0 {
            s.ca.opcount = s.oa.prev_opcount;
            s.ca.count0 = s.oa.prev_count0;
            s.oa.prev_opcount = 0;
            s.oa.prev_count0 = 0;
        }

        s.mapped_len = typebuf_maplen();
        g_st().State = MODE_NORMAL_BUSY;

        if s.toplevel && readbuf1_empty() {
            set_vcount_ca(&mut s.ca, &mut s.set_prevcount);
        }
}

fn normal_handle_special_visual_command(s: &mut NormalState) -> bool {
    let flags = nv_cmds[s.idx as usize].cmd_flags;
        if g_st().km_stopsel && (flags & NV_STS) != 0 && (g_st().mod_mask & c_int::from(MOD_MASK_SHIFT)) == 0 {
            end_visual_mode();
            redraw_curbuf_later(UPD_INVERTED);
        }

        if g_st().km_startsel {
            if (flags & NV_SS) != 0 {
                unshift_special(&mut s.ca);
                s.idx = find_command(s.ca.cmdchar);
                if s.idx < 0 {
                    clearopbeep(&mut s.oa);
                    return true;
                }
            } else if (flags & NV_SSS) != 0 && (g_st().mod_mask & c_int::from(MOD_MASK_SHIFT)) != 0 {
                g_st().mod_mask &= !c_int::from(MOD_MASK_SHIFT);
            }
        }
        false
}

fn normal_need_additional_char(s: &mut NormalState) -> bool {
    let flags = nv_cmds[s.idx as usize].cmd_flags;
        let pending_op = s.oa.op_type != OP_NOP;
        let cmdchar = s.ca.cmdchar;
        (flags & NV_NCH) != 0
            && (((flags & NV_NCH_NOP) == NV_NCH_NOP && !pending_op)
                || (flags & NV_NCH_ALW) == NV_NCH_ALW
                || (cmdchar == b'q' as c_int
                    && !pending_op
                    && g_st().reg_recording == 0
                    && g_st().reg_executing == 0)
                || ((cmdchar == b'a' as c_int || cmdchar == b'i' as c_int)
                    && (pending_op || g_st().Visual.active)))
}

fn normal_need_redraw_mode_message(s: &mut NormalState) -> bool {
    unsafe {
        ((opt_vars().p_smd != 0
            && g_st().msg_silent == 0
            && (g_st().restart_edit != 0
                || (g_st().Visual.active
                    && s.old_pos.lnum == (**curwin.as_ptr()).w_cursor.lnum
                    && s.old_pos.col == (**curwin.as_ptr()).w_cursor.col))
            && (g_st().clear_cmdline || g_st().redraw_cmdline)
            && (g_st().msg_didout || (g_st().msg_didany && g_st().msg_scroll != 0))
            && !g_st().msg_nowait
            && g_st().KeyTyped)
            || (g_st().restart_edit != 0 && !g_st().Visual.active && g_st().msg_scroll != 0 && g_st().emsg_on_display))
            && s.oa.regname == 0
            && (s.ca.retval & CA_COMMAND_BUSY) == 0
            && stuff_empty()
            && typebuf_typed() != 0
            && g_st().emsg_silent == 0
            && !g_st().in_assert_fails
            && !g_st().did_wait_return
            && s.oa.op_type == OP_NOP
    }
}

fn normal_redraw_mode_message(_s: &mut NormalState) {
    let save_State = g_st().State;

        if g_st().restart_edit != 0 {
            g_st().State = MODE_INSERT;
        }

        if g_st().must_redraw != 0
            && g_st().keep_msg.is_some()
            && !g_st().emsg_on_display
        {
            let kmsg = g_st().keep_msg.take();
            setcursor();
            update_screen();
            if let Some(kmsg_cstr) = kmsg {
                let ptr = kmsg_cstr.as_ptr();
                g_st().keep_msg = Some(kmsg_cstr.clone());
                msg(ptr, g_st().keep_msg_hl_id);
            }
        }
        setcursor();
        ui_cursor_shape();
        ui_flush();
        if g_st().msg_scroll != 0 || g_st().emsg_on_display {
            msg_delay(1003, true);
        }
        msg_delay(3003, false);
        g_st().State = save_State;

        g_st().msg_scroll = 0;
        g_st().emsg_on_display = false;
}

fn normal_get_additional_char(s: &mut NormalState) {
    unsafe {
        let cp: *mut c_int;
        let mut repl = false;
        let mut lit = false;

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        g_st().did_cursorhold = true;
        if s.ca.cmdchar == b'g' as c_int {
            s.ca.nchar = plain_vgetc();
            langmap_adjust(&mut s.ca.nchar, true);
            s.need_flushbuf |= add_to_showcmd(s.ca.nchar);
            if s.ca.nchar == b'r' as c_int
                || s.ca.nchar == b'\'' as c_int
                || s.ca.nchar == b'`' as c_int
                || s.ca.nchar == Ctrl_BSL
            {
                cp = &mut s.ca.extra_char;
                if s.ca.nchar != b'r' as c_int {
                    lit = true;
                } else {
                    repl = true;
                }
            } else {
                cp = ptr::null_mut();
            }
        } else {
            if s.ca.cmdchar == b'r' as c_int {
                repl = true;
            }
            cp = &mut s.ca.nchar;
        }
        let lang = repl || (nv_cmds[s.idx as usize].cmd_flags & NV_LANG) != 0;

        if !cp.is_null() {
            let mut langmap_active = false;
            if repl {
                g_st().State = MODE_REPLACE;
                ui_cursor_shape_no_check_conceal();
            }
            if lang && (*g_st().curbuf).b_p_iminsert == B_IMODE_LMAP {
                g_st().no_mapping -= 1;
                g_st().allow_keys -= 1;
                if repl {
                    g_st().State = MODE_LREPLACE;
                } else {
                    g_st().State = MODE_LANGMAP;
                }
                langmap_active = true;
            }

            *cp = plain_vgetc();

            if langmap_active {
                g_st().no_mapping += 1;
                g_st().allow_keys += 1;
            }
            g_st().State = MODE_NORMAL_BUSY;
            s.need_flushbuf |= add_to_showcmd(*cp);

            if !lit {
                if *cp == Ctrl_K
                    && ((nv_cmds[s.idx as usize].cmd_flags & NV_LANG) != 0
                        || ptr::eq(cp, &s.ca.extra_char))
                    && vim_strchr(opt_vars().p_cpo, kCpoDigraph).is_null()
                {
                    s.c = get_digraph(false);
                    if s.c > 0 {
                        *cp = s.c;
                        del_from_showcmd(3);
                        s.need_flushbuf |= add_to_showcmd(*cp);
                    }
                }

                langmap_adjust(&mut *cp, !lang);
            }

            if ptr::eq(cp, &s.ca.extra_char)
                && s.ca.nchar == Ctrl_BSL
                && (s.ca.extra_char == Ctrl_N || s.ca.extra_char == Ctrl_G)
            {
                s.ca.cmdchar = Ctrl_BSL;
                s.ca.nchar = s.ca.extra_char;
                s.idx = find_command(s.ca.cmdchar);
            } else if (s.ca.nchar == b'n' as c_int || s.ca.nchar == b'N' as c_int)
                && s.ca.cmdchar == b'g' as c_int
            {
                (*s.ca.oap).op_type = get_op_type(*cp, NUL as c_int);
            } else if *cp == Ctrl_BSL {
                let mut towait = if opt_vars().p_ttm >= 0 {
                    opt_vars().p_ttm as c_int
                } else {
                    opt_vars().p_tm as c_int
                };
                while {
                    s.c = vpeekc();
                    s.c <= 0 && towait > 0
                } {
                    do_sleep(i64::from(if towait > 50 { 50 } else { towait }), false);
                    towait -= 50;
                }
                if s.c > 0 {
                    s.c = plain_vgetc();
                    if s.c != Ctrl_N && s.c != Ctrl_G {
                        vungetc(s.c);
                    } else {
                        s.ca.cmdchar = Ctrl_BSL;
                        s.ca.nchar = s.c;
                        s.idx = find_command(s.ca.cmdchar);
                        assert!(s.idx >= 0);
                    }
                }
            }

            if lang {
                g_st().no_mapping -= 1;
                let mut state = GraphemeState::default();
                let mut prev_code = s.ca.nchar;

                while {
                    s.c = vpeekc();
                    s.c > 0 && (s.c >= 0x100 || utf_byte2len(vpeekc()) > 1)
                } {
                    s.c = plain_vgetc();

                    if !utf_iscomposing(prev_code, s.c, &raw mut state) {
                        vungetc(s.c);
                        break;
                    }

                    if s.ca.nchar_len == 0 {
                        s.ca.nchar_len =
                            utf_char2bytes(s.ca.nchar, s.ca.nchar_composing.as_mut_ptr());
                    }

                    if ((s.ca.nchar_len + utf_char2len(s.c)) as usize)
                        < s.ca.nchar_composing.len()
                    {
                        s.ca.nchar_len += utf_char2bytes(
                            s.c,
                            s.ca
                                .nchar_composing
                                .as_mut_ptr()
                                .add(s.ca.nchar_len as usize),
                        );
                    }
                    prev_code = s.c;
                }
                s.ca.nchar_composing[s.ca.nchar_len as usize] = NUL;
                g_st().no_mapping += 1;
                g_st().no_u_sync += 1;
                gotchars_ignore();
                g_st().no_u_sync -= 1;
            }
        }
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
    }
}

fn normal_invert_horizontal(s: &mut NormalState) {
    s.ca.cmdchar = match s.ca.cmdchar {
            c if c == b'l' as c_int => b'h' as c_int,
            K_RIGHT => K_LEFT,
            K_S_RIGHT => K_S_LEFT,
            K_C_RIGHT => K_C_LEFT,
            c if c == b'h' as c_int => b'l' as c_int,
            K_LEFT => K_RIGHT,
            K_S_LEFT => K_S_RIGHT,
            K_C_LEFT => K_C_RIGHT,
            c if c == b'>' as c_int => b'<' as c_int,
            c if c == b'<' as c_int => b'>' as c_int,
            c => c,
        };
        s.idx = find_command(s.ca.cmdchar);
}

fn normal_get_command_count(s: &mut NormalState) -> bool {
    if g_st().Visual.active && g_st().Visual.select {
            return false;
        }
        while (s.c >= b'1' as c_int && s.c <= b'9' as c_int)
            || (s.ca.count0 != 0
                && (s.c == K_DEL || s.c == K_KDEL || s.c == b'0' as c_int))
        {
            if s.c == K_DEL || s.c == K_KDEL {
                s.ca.count0 /= 10;
                del_from_showcmd(4);
            } else if s.ca.count0 > 99_999_999 {
                s.ca.count0 = 999_999_999;
            } else {
                s.ca.count0 = s.ca.count0 * 10 + (s.c - b'0' as c_int);
            }

            if s.toplevel && readbuf1_empty() {
                set_vcount_ca(&mut s.ca, &mut s.set_prevcount);
            }

            if s.ctrl_w {
                g_st().no_mapping += 1;
                g_st().allow_keys += 1;
            }

            g_st().no_zero_mapping += 1;
            s.c = plain_vgetc();
            langmap_adjust(&mut s.c, true);
            g_st().no_zero_mapping -= 1;
            if s.ctrl_w {
                g_st().no_mapping -= 1;
                g_st().allow_keys -= 1;
            }
            s.need_flushbuf |= add_to_showcmd(s.c);
        }

        if s.c == Ctrl_W && !s.ctrl_w && s.oa.op_type == OP_NOP {
            s.ctrl_w = true;
            s.ca.opcount = s.ca.count0;
            s.ca.count0 = 0;
            g_st().no_mapping += 1;
            g_st().allow_keys += 1;
            s.c = plain_vgetc();
            langmap_adjust(&mut s.c, true);
            g_st().no_mapping -= 1;
            g_st().allow_keys -= 1;
            s.need_flushbuf |= add_to_showcmd(s.c);
            return true;
        }

        false
}

fn normal_finish_command(s: &mut NormalState) {
    unsafe {
        let mut did_visual_op = false;

        if !s.command_finished {
            if !g_st().finish_op
                && s.oa.op_type == 0
                && (s.idx < 0 || (nv_cmds[s.idx as usize].cmd_flags & NV_KEEPREG) == 0)
            {
                clearop(&mut s.oa);
                set_reg_var(get_default_register_name());
            }

            if s.old_mapped_len > 0 {
                s.old_mapped_len = typebuf_maplen();
            }

            if s.ca.cmdchar != K_IGNORE && s.ca.cmdchar != K_MOUSEMOVE {
                did_visual_op =
                    g_st().Visual.active && s.oa.op_type != OP_NOP && s.oa.op_type != OP_COLON;
                do_pending_operator(&mut s.ca, s.old_col, false);
            }

            if normal_need_redraw_mode_message(s) {
                normal_redraw_mode_message(s);
            }
        }

        if g_st().Visual.need_end {
            g_st().Visual.need_end = false;
            reset_VIsual();
        }

        g_st().msg_nowait = false;

        if g_st().finish_op || did_visual_op {
            set_reg_var(get_default_register_name());
        }

        let prev_finish_op = g_st().finish_op;
        if s.oa.op_type == OP_NOP {
            g_st().finish_op = false;
            may_trigger_modechanged();
        }
        if prev_finish_op
            || s.ca.cmdchar == b'r' as c_int
            || (s.ca.cmdchar == b'g' as c_int && s.ca.nchar == b'r' as c_int)
        {
            ui_cursor_shape();
        }

        checkpcmark();
        mb_check_adjust_col(&mut **curwin.as_ptr());

        if (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 && s.toplevel {
            validate_cursor(*curwin.as_ptr());
            do_check_scrollbind(true);
        }

        if (**curwin.as_ptr()).w_onebuf_opt.wo_crb != 0 && s.toplevel {
            validate_cursor(*curwin.as_ptr());
            do_check_cursorbind();
        }

        if s.oa.op_type == OP_NOP
            && ((g_st().restart_edit != 0 && !g_st().Visual.active && s.old_mapped_len == 0)
                || g_st().Visual.restart_select == 1)
            && (s.ca.retval & CA_COMMAND_BUSY) == 0
            && stuff_empty()
            && s.oa.regname == 0
        {
            if g_st().Visual.restart_select == 1 {
                g_st().Visual.select = true;
                g_st().Visual.select_reg = 0;
                may_trigger_modechanged();
                showmode();
                g_st().Visual.restart_select = 0;
            }
            if g_st().restart_edit != 0 && !g_st().Visual.active && s.old_mapped_len == 0 {
                edit(g_st().restart_edit, false, 1);
            }
        }

        if g_st().Visual.restart_select == 2 {
            g_st().Visual.restart_select = 1;
        }

        g_st().opcount = s.ca.opcount;

        atom_cmd_end(&mut s.ca);

        if s.oa.op_type == OP_NOP && s.oa.regname == 0 && s.ca.cmdchar != K_EVENT {
            clear_showcmd();
        }
    }
}

fn normal_execute(s: &mut NormalState, key: c_int) -> c_int {
    unsafe {
        atom_cmd_start(key);

        s.command_finished = false;
        s.ctrl_w = false;
        s.old_col = (**curwin.as_ptr()).w_curswant;
        s.c = key;

        langmap_adjust(&mut s.c, get_real_state() != MODE_SELECT);

        if g_st().restart_edit == 0 {
            s.old_mapped_len = 0;
        } else if s.old_mapped_len != 0
            || (g_st().Visual.active && s.mapped_len == 0 && typebuf_maplen() > 0)
        {
            s.old_mapped_len = typebuf_maplen();
        }

        if s.c == NUL as c_int {
            s.c = K_ZERO;
        }

        if g_st().Visual.active
            && g_st().Visual.select
            && (vim_isprintc(s.c) || s.c == NL || s.c == CAR || s.c == K_KENTER)
        {
            requeue_key(g_st().vgetc_char, g_st().vgetc_mod_mask, 0, true);
            if g_st().restart_edit != 0 {
                s.c = b'd' as c_int;
            } else {
                s.c = b'c' as c_int;
            }
            g_st().msg_nowait = true;
            s.old_mapped_len = 0;
        }

        s.need_flushbuf = add_to_showcmd(s.c);

        while normal_get_command_count(s) {}

        if s.c == K_EVENT {
            s.oa.prev_opcount = s.ca.opcount;
            s.oa.prev_count0 = s.ca.count0;
        } else if s.ca.opcount != 0 {
            if s.ca.count0 != 0 {
                if s.ca.opcount >= 999_999_999 / s.ca.count0 {
                    s.ca.count0 = 999_999_999;
                } else {
                    s.ca.count0 *= s.ca.opcount;
                }
            } else {
                s.ca.count0 = s.ca.opcount;
            }
        }

        s.ca.opcount = s.ca.count0;
        s.ca.count1 = if s.ca.count0 == 0 {
            1
        } else {
            s.ca.count0
        };

        if s.toplevel && readbuf1_empty() {
            set_vcount(
                i64::from(s.ca.count0),
                i64::from(s.ca.count1),
                s.set_prevcount,
            );
        }

        if s.ctrl_w {
            s.ca.nchar = s.c;
            s.ca.cmdchar = Ctrl_W;
        } else {
            s.ca.cmdchar = s.c;
        }

        s.idx = find_command(s.ca.cmdchar);

        'finish: {
            if s.idx < 0 {
                clearopbeep(&mut s.oa);
                s.command_finished = true;
                break 'finish;
            }

            if (nv_cmds[s.idx as usize].cmd_flags & NV_NCW) != 0
                && check_text_or_curbuf_locked(&mut s.oa)
            {
                s.command_finished = true;
                break 'finish;
            }

            if g_st().Visual.active && normal_handle_special_visual_command(s) {
                s.command_finished = true;
                break 'finish;
            }

            if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0
                && g_st().KeyTyped
                && g_st().KeyStuffed == 0
                && (nv_cmds[s.idx as usize].cmd_flags & NV_RL) != 0
            {
                normal_invert_horizontal(s);
            }

            if normal_need_additional_char(s) {
                normal_get_additional_char(s);
            }

            if s.need_flushbuf {
                ui_flush();
            }

            if s.ca.cmdchar != K_IGNORE && s.ca.cmdchar != K_EVENT {
                g_st().did_cursorhold = false;
            }

            g_st().State = MODE_NORMAL;

            if s.ca.nchar == ESC || s.ca.extra_char == ESC {
                clearop(&mut s.oa);
                s.command_finished = true;
                break 'finish;
            }

            if s.ca.cmdchar != K_IGNORE {
                g_st().msg_didout = false;
                g_st().msg_col = 0;
            }

            s.old_pos = (**curwin.as_ptr()).w_cursor;

            if !g_st().Visual.active && g_st().km_startsel {
                let flags = nv_cmds[s.idx as usize].cmd_flags;
                if (flags & NV_SS) != 0 {
                    start_selection();
                    unshift_special(&mut s.ca);
                    s.idx = find_command(s.ca.cmdchar);
                    assert!(s.idx >= 0);
                } else if (flags & NV_SSS) != 0 && (g_st().mod_mask & c_int::from(MOD_MASK_SHIFT)) != 0 {
                    start_selection();
                    g_st().mod_mask &= !c_int::from(MOD_MASK_SHIFT);
                }
            }

            s.ca.arg = c_int::from(nv_cmds[s.idx as usize].cmd_arg);
            (nv_cmds[s.idx as usize].cmd_func)(&mut s.ca);
        }

        normal_finish_command(s);
        s.ca.searchbuf = None;
        1
    }
}

fn normal_check_stuff_buffer(_s: &mut NormalState) {
    if stuff_empty() {
            g_st().did_check_timestamps = false;

            if g_st().need_check_timestamps {
                check_timestamps(0);
            }

            if g_st().need_wait_return {
                wait_return(0);
            }
        }
}

fn normal_check_interrupt(s: &mut NormalState) {
    if g_st().got_int {
            if !g_st().quit_more {
                vgetc();
            }
            g_st().got_int = false;
            s.previous_got_int = true;
        } else {
            s.previous_got_int = false;
        }
}

fn normal_check_window_scrolled(_s: &mut NormalState) {
    if !g_st().finish_op {
            may_trigger_win_scrolled_resized();
        }
}

fn normal_check_cursor_moved(_s: &mut NormalState) {
    unsafe {
        if !g_st().finish_op
            && has_event(EVENT_CURSORMOVED)
            && (!ptr::eq(g_st().last_cursormoved_win, *curwin.as_ptr())
                || !equalpos(g_st().last_cursormoved, (**curwin.as_ptr()).w_cursor))
        {
            apply_autocmds(
                EVENT_CURSORMOVED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            g_st().last_cursormoved_win = *curwin.as_ptr();
            g_st().last_cursormoved = (**curwin.as_ptr()).w_cursor;
        }
    }
}

fn normal_check_text_changed(_s: &mut NormalState) {
    unsafe {
        if !g_st().finish_op
            && has_event(EVENT_TEXTCHANGED)
            && (*g_st().curbuf).b_last_changedtick != buf_get_changedtick(&*g_st().curbuf)
        {
            apply_autocmds(
                EVENT_TEXTCHANGED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            (*g_st().curbuf).b_last_changedtick = buf_get_changedtick(&*g_st().curbuf);
        }
    }
}

fn normal_check_safe_state(_s: &mut NormalState) {
    may_trigger_safestate(!op_pending() && g_st().restart_edit == 0);
}

fn normal_check_folds(_s: &mut NormalState) {
    unsafe {
        foldAdjustVisual();

        if hasAnyFolding(&mut **curwin.as_ptr()) != 0 && !char_avail() {
            foldCheckClose();

            if (opt_vars().fdo_flags & kOptFdoFlagAll) != 0 {
                foldOpenCursor();
            }
        }
    }
}

fn normal_redraw(_s: &mut NormalState) {
    unsafe {
        update_topline(*curwin.as_ptr());
        validate_cursor(*curwin.as_ptr());

        show_cursor_info_later(false);

        if g_st().must_redraw != 0 {
            update_screen();
        } else {
            redraw_statuslines();
            if g_st().redraw_cmdline || g_st().clear_cmdline || g_st().redraw_mode {
                showmode();
            }
        }

        (*g_st().curbuf).b_last_used = os_time() as time_t;

        if let Some(p) = g_st().keep_msg.clone() {
            g_st().msg_hist_off = true;
            msg(p.as_ptr(), g_st().keep_msg_hl_id);
            g_st().msg_hist_off = false;
        }

        if g_st().need_fileinfo && !shortmess(kShmFileinfo) {
            fileinfo(0, 1, false);
            g_st().need_fileinfo = false;
        }

        g_st().emsg_on_display = false;
        g_st().did_emsg = 0;
        g_st().msg_didany = false;
        may_clear_sb_text();

        setcursor();
    }
}

fn normal_check(s: &mut NormalState) -> c_int {
    unsafe {
        normal_check_stuff_buffer(s);
        normal_check_interrupt(s);

        if g_st().did_throw && g_st().ex_normal_busy == 0 {
            discard_current_exception();
        }

        g_st().msg_scroll = 0;
        g_st().quit_more = false;

        state_no_longer_safe(ptr::null());

        if g_st().skip_redraw {
            g_st().skip_redraw = false;
            setcursor();
        } else if g_st().do_redraw || stuff_empty() {
            terminal_check_refresh();

            update_topline(*curwin.as_ptr());
            validate_cursor(*curwin.as_ptr());

            normal_check_cursor_moved(s);
            normal_check_text_changed(s);
            normal_check_window_scrolled(s);
            normal_check_safe_state(s);

            if (*g_st().curtab).tp_diff_update != 0 || (*g_st().curtab).tp_diff_invalid != 0 {
                ex_diffupdate(&mut crate::ex_docmd::exarg_T::new());
                (*g_st().curtab).tp_diff_update = 0;
            }

            if g_st().diff_need_scrollbind {
                check_scrollbind(0, 0);
                g_st().diff_need_scrollbind = false;
            }

            normal_check_folds(s);
            normal_redraw(s);
            g_st().do_redraw = false;

            if has_time_fd() {
                time_msg(c"first screen update".as_ptr(), None);
                time_finish();
            }
            may_make_initial_scroll_size_snapshot();
        }

        g_st().may_garbage_collect = true;
        update_curswant();

        normal_prepare(s);
        1
    }
}

fn set_vcount_ca(cap: &mut cmdarg_T, set_prevcount: &mut bool) {
    let mut count = i64::from(cap.count0);
        if cap.opcount != 0 {
            count = i64::from(cap.opcount) * if count == 0 { 1 } else { count };
        }
        set_vcount(count, if count == 0 { 1 } else { count }, *set_prevcount);
        *set_prevcount = false;
}

pub fn visualinfo() -> visualinfo_T {
    unsafe {
        visualinfo_T {
            vi_start: g_st().Visual.start,
            vi_end: (**curwin.as_ptr()).w_cursor,
            vi_mode: g_st().Visual.mode,
            vi_curswant: (**curwin.as_ptr()).w_curswant,
        }
    }
}

pub fn end_visual_mode() {
    unsafe {
        g_st().Visual.select_exclu_adj = false;
        g_st().Visual.active = false;
        setmouse();
        g_st().mouse_dragging = 0;

        (*g_st().curbuf).b_visual = visualinfo();
        (*g_st().curbuf).b_visual_mode_eval = g_st().Visual.mode;
        if !virtual_active(*curwin.as_ptr()) {
            (**curwin.as_ptr()).w_cursor.coladd = 0;
        }

        may_clear_cmdline();

        adjust_cursor_eol();
        may_trigger_modechanged();
    }
}

pub fn reset_VIsual_and_resel() {
    if g_st().Visual.active {
            end_visual_mode();
            redraw_curbuf_later(UPD_INVERTED);
        }
        g_st().Visual.reselect = 0;
}

pub fn reset_VIsual() {
    if g_st().Visual.active {
            end_visual_mode();
            redraw_curbuf_later(UPD_INVERTED);
            g_st().Visual.reselect = 0;
        }
}

pub fn restore_visual_mode() {
    unsafe {
        if norm_st().VIsual_mode_orig != NUL as c_int {
            (*g_st().curbuf).b_visual.vi_mode = norm_st().VIsual_mode_orig;
            norm_st().VIsual_mode_orig = NUL as c_int;
        }
    }
}

fn find_is_eval_item(
    ptr: *const c_char,
    colp: &mut c_int,
    bnp: &mut c_int,
    dir: c_int,
) -> bool {
    unsafe {
        if (*ptr == b']' as c_char && dir == BACKWARD) || (*ptr == b'[' as c_char && dir == FORWARD)
        {
            *bnp += 1;
        }
        if *bnp > 0 {
            if (*ptr == b'[' as c_char && dir == BACKWARD)
                || (*ptr == b']' as c_char && dir == FORWARD)
            {
                *bnp -= 1;
            }
            return true;
        }

        if *ptr == b'.' as c_char {
            return true;
        }

        let gt_off: isize = if dir == BACKWARD { 0 } else { 1 };
        let dash_off: isize = if dir == BACKWARD { -1 } else { 0 };
        if *ptr.offset(gt_off) == b'>' as c_char && *ptr.offset(dash_off) == b'-' as c_char {
            *colp += dir;
            return true;
        }
        false
    }
}

pub fn find_ident_under_cursor(
    text: *mut *mut c_char,
    find_type: c_int,
    offset: Option<&mut c_int>,
) -> usize {
    unsafe {
        let mut textcol: c_int = 0;
        let len = find_ident_at_pos(
            &mut **curwin.as_ptr(),
            (**curwin.as_ptr()).w_cursor.lnum,
            (**curwin.as_ptr()).w_cursor.col,
            text,
            if offset.is_some() {
                &raw mut textcol
            } else {
                ptr::null_mut()
            },
            find_type,
        );
        if let Some(offset) = offset {
            *offset = (**curwin.as_ptr()).w_cursor.col - textcol;
        }
        len
    }
}

pub fn find_ident_at_pos(
    wp: &mut win_T,
    lnum: linenr_T,
    mut startcol: colnr_T,
    text: *mut *mut c_char,
    textcol: *mut c_int,
    find_type: c_int,
) -> usize {
    unsafe {
        let mut col: c_int = 0;
        let mut this_class: c_int = 0;
        let mut prev_class: c_int;
        let mut prevcol: c_int;
        let mut bn: c_int;

        let mut ptr = ml_get_buf(wp.w_buffer, lnum);
        let mut i = if (find_type & FIND_IDENT) != 0 { 0 } else { 1 };
        while i < 2 {
            col = startcol;
            while *ptr.add(col as usize) != NUL {
                if (find_type & FIND_EVAL) != 0 && *ptr.add(col as usize) == b']' as c_char {
                    break;
                }
                this_class = mb_get_class(ptr.add(col as usize));
                if this_class != 0 && (i == 1 || this_class != 1) {
                    break;
                }
                col += utfc_ptr2len(ptr.add(col as usize));
            }

            bn = c_int::from(*ptr.add(col as usize) == b']' as c_char);

            if (find_type & FIND_EVAL) != 0 && *ptr.add(col as usize) == b']' as c_char {
                this_class = mb_get_class(c"a".as_ptr());
            } else {
                this_class = mb_get_class(ptr.add(col as usize));
            }
            while col > 0 && this_class != 0 {
                prevcol = col - 1 - utf_head_off(ptr, ptr.add((col - 1) as usize));
                prev_class = mb_get_class(ptr.add(prevcol as usize));
                if this_class != prev_class
                    && (i == 0 || prev_class == 0 || (find_type & FIND_IDENT) != 0)
                    && ((find_type & FIND_EVAL) == 0
                        || prevcol == 0
                        || !find_is_eval_item(
                            ptr.add(prevcol as usize),
                            &mut prevcol,
                            &mut bn,
                            BACKWARD,
                        ))
                {
                    break;
                }
                col = prevcol;
            }

            this_class = this_class.min(2);
            if (find_type & FIND_STRING) == 0 || this_class == 2 {
                break;
            }
            i += 1;
        }

        if *ptr.add(col as usize) == NUL || (i == 0 && this_class != 2) {
            if (find_type & FIND_STRING) != 0 {
                emsg(tr(c"E348: No string under cursor".as_ptr()));
            } else {
                emsg(tr(e_noident.as_ptr()));
            }
            return 0;
        }
        ptr = ptr.add(col as usize);
        *text = ptr;
        if !textcol.is_null() {
            *textcol = col;
        }

        bn = 0;
        startcol -= col;
        col = 0;
        this_class = mb_get_class(ptr);
        while *ptr.add(col as usize) != NUL
            && ((if i == 0 {
                mb_get_class(ptr.add(col as usize)) == this_class
            } else {
                mb_get_class(ptr.add(col as usize)) != 0
            }) || ((find_type & FIND_EVAL) != 0
                && col <= startcol
                && find_is_eval_item(ptr.add(col as usize), &mut col, &mut bn, FORWARD)))
        {
            col += utfc_ptr2len(ptr.add(col as usize));
        }

        assert!(col >= 0);
        col as usize
    }
}

fn prep_redo_cmd(cap: &mut cmdarg_T) {
    let composing = cap.nchar_len > 0;
        prep_redo(true, composing, atom_cmd_spec(cap));
        if composing {
            redo_append_str(cap.nchar_composing.as_ptr(), -1);
        }
}

fn checkclearop(oap: &mut oparg_T) -> bool {
    if oap.op_type == OP_NOP {
            return false;
        }
        clearopbeep(&mut *oap);
        true
}

fn checkclearopq(oap: &mut oparg_T) -> bool {
    if oap.op_type == OP_NOP && !g_st().Visual.active {
            return false;
        }
        clearopbeep(&mut *oap);
        true
}

pub fn clearop(oap: &mut oparg_T) {
    oap.op_type = OP_NOP;
        oap.regname = 0;
        oap.motion_force = NUL as c_int;
        oap.use_reg_one = false;
        oap.restore_cursor = false;
        g_st().motion_force = NUL as c_int;
}

pub fn clearopbeep(oap: &mut oparg_T) {
    if oap.op_type == OP_MCURSOR && oap.motion_type != kMTUnknown {
            return;
        }
        clearop(&mut *oap);
        beep_flush();
}

fn unshift_special(cap: &mut cmdarg_T) {
    cap.cmdchar = match cap.cmdchar {
            K_S_RIGHT => K_RIGHT,
            K_S_LEFT => K_LEFT,
            K_S_UP => K_UP,
            K_S_DOWN => K_DOWN,
            K_S_HOME => K_HOME,
            K_S_END => K_END,
            c => c,
        };
        cap.cmdchar = simplify_key(cap.cmdchar, &mut g_st().mod_mask);
}

pub fn may_clear_cmdline() {
    if g_st().mode_displayed {
            g_st().clear_cmdline = true;
        } else {
            clear_showcmd();
        }
}

pub fn clear_showcmd() {
    unsafe {
        if opt_vars().p_sc == 0 {
            return;
        }

        let sc_buf = (&raw mut g_st().showcmd_buf).cast::<c_char>();

        if g_st().Visual.active && !char_avail() {
            let cursor_bot = lt(g_st().Visual.start, (**curwin.as_ptr()).w_cursor);
            let mut leftcol: colnr_T = 0;
            let mut rightcol: colnr_T = 0;
            let mut top: linenr_T;
            let mut bot: linenr_T;

            if cursor_bot {
                top = g_st().Visual.start.lnum;
                bot = (**curwin.as_ptr()).w_cursor.lnum;
            } else {
                top = (**curwin.as_ptr()).w_cursor.lnum;
                bot = g_st().Visual.start.lnum;
            }
            hasFolding(*curwin.as_ptr(), top, &raw mut top, ptr::null_mut());
            hasFolding(*curwin.as_ptr(), bot, ptr::null_mut(), &raw mut bot);
            let lines = bot - top + 1;

            if g_st().Visual.mode == Ctrl_V {
                let saved_sbr = opt_vars().p_sbr;
                let saved_w_sbr = (**curwin.as_ptr()).w_onebuf_opt.wo_sbr;

                opt_vars().p_sbr = empty_string_option.as_ptr().cast::<c_char>();
                (**curwin.as_ptr()).w_onebuf_opt.wo_sbr = empty_string_option.as_ptr().cast::<c_char>();
                getvcols(
                    *curwin.as_ptr(),
                    &mut (**curwin.as_ptr()).w_cursor,
                    &mut g_st().Visual.start,
                    &mut leftcol,
                    &mut rightcol,
                    GETVCOL_END_EXCL_LBR,
                );
                opt_vars().p_sbr = saved_sbr;
                (**curwin.as_ptr()).w_onebuf_opt.wo_sbr = saved_w_sbr;
                vim_snprintf!(
                    sc_buf,
                    SHOWCMD_BUFLEN,
                    c"%lldx%lld".as_ptr(),
                    lines as c_longlong,
                    (i64::from(rightcol) - i64::from(leftcol) + 1) as c_longlong,
                );
            } else if g_st().Visual.mode == b'V' as c_int || g_st().Visual.start.lnum != (**curwin.as_ptr()).w_cursor.lnum {
                vim_snprintf!(
                    sc_buf,
                    SHOWCMD_BUFLEN,
                    c"%lld".as_ptr(),
                    lines as c_longlong,
                );
            } else {
                let mut s: *mut c_char;
                let e: *mut c_char;
                let mut bytes: c_int = 0;
                let mut chars: c_int = 0;

                if cursor_bot {
                    s = ml_get_pos(&g_st().Visual.start);
                    e = get_cursor_pos_ptr();
                } else {
                    s = get_cursor_pos_ptr();
                    e = ml_get_pos(&g_st().Visual.start);
                }
                while if *opt_vars().p_sel != b'e' as c_char {
                    s <= e
                } else {
                    s < e
                } {
                    let l = utfc_ptr2len(s);
                    if l == 0 {
                        bytes += 1;
                        chars += 1;
                        break;
                    }
                    bytes += l;
                    chars += 1;
                    s = s.add(l as usize);
                }
                if bytes == chars {
                    vim_snprintf!(sc_buf, SHOWCMD_BUFLEN, c"%d".as_ptr(), chars);
                } else {
                    vim_snprintf!(sc_buf, SHOWCMD_BUFLEN, c"%d-%d".as_ptr(), chars, bytes);
                }
            }
            let limit = if ui_has(kUIMessages) {
                SHOWCMD_BUFLEN - 1
            } else {
                SHOWCMD_COLS
            };
            *sc_buf.add(limit) = NUL;
            norm_st().showcmd_visual = true;
        } else {
            *sc_buf = NUL;
            norm_st().showcmd_visual = false;

            if norm_st().showcmd_is_clear {
                return;
            }
        }

        display_showcmd();
    }
}

pub fn add_to_showcmd(c: c_int) -> bool {
    unsafe {
        const IGNORE: &[c_int] = &[
            K_IGNORE,
            K_LEFTMOUSE,
            K_LEFTDRAG,
            K_LEFTRELEASE,
            K_MOUSEMOVE,
            K_MIDDLEMOUSE,
            K_MIDDLEDRAG,
            K_MIDDLERELEASE,
            K_RIGHTMOUSE,
            K_RIGHTDRAG,
            K_RIGHTRELEASE,
            K_MOUSEDOWN,
            K_MOUSEUP,
            K_MOUSELEFT,
            K_MOUSERIGHT,
            K_X1MOUSE,
            K_X1DRAG,
            K_X1RELEASE,
            K_X2MOUSE,
            K_X2DRAG,
            K_X2RELEASE,
            K_EVENT,
            K_COMMAND,
            K_LUA,
        ];

        if opt_vars().p_sc == 0 || g_st().msg_silent != 0 || g_st().ex_normal_busy != 0 || g_st().KeyStuffed != 0 {
            return false;
        }

        let sc_buf = (&raw mut g_st().showcmd_buf).cast::<c_char>();

        if norm_st().showcmd_visual {
            *sc_buf = NUL;
            norm_st().showcmd_visual = false;
        }

        if is_special(c) && IGNORE.contains(&c) {
            return false;
        }

        let mut mbyte_buf: [c_char; MB_MAXCHAR + 1] = [0; MB_MAXCHAR + 1];
        let p: *mut c_char = if c <= 0x7f || !vim_isprintc(c) {
            let tc = transchar(c);
            if *tc == b' ' as c_char {
                { let s = c"<20>".as_ptr(); std::ptr::copy_nonoverlapping(s, tc, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            }
            tc
        } else {
            let len = utf_char2bytes(c, mbyte_buf.as_mut_ptr()) as usize;
            mbyte_buf[len] = NUL;
            mbyte_buf.as_mut_ptr()
        };

        let old_len = std::ffi::CStr::from_ptr(sc_buf).to_bytes().len();
        let extra_len = std::ffi::CStr::from_ptr(p).to_bytes().len();
        let limit = if ui_has(kUIMessages) {
            SHOWCMD_BUFLEN - 1
        } else {
            SHOWCMD_COLS
        };
        if old_len + extra_len > limit {
            let overflow = old_len + extra_len - limit;
            ptr::copy(sc_buf.add(overflow), sc_buf, old_len - overflow + 1);
        }
        { let d = sc_buf; let s = p; std::ptr::copy_nonoverlapping(s, d.add(std::ffi::CStr::from_ptr(d).to_bytes().len()), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };

        if char_avail() {
            return false;
        }

        display_showcmd();
        true
    }
}

pub fn add_to_showcmd_c(c: c_int) {
    add_to_showcmd(c);
        setcursor();
}

fn del_from_showcmd(mut len: c_int) {
    unsafe {
        if opt_vars().p_sc == 0 {
            return;
        }
        let sc_buf = (&raw mut g_st().showcmd_buf).cast::<c_char>();
        let old_len = std::ffi::CStr::from_ptr(sc_buf).to_bytes().len() as c_int;
        len = len.min(old_len);
        *sc_buf.add((old_len - len) as usize) = NUL;

        if !char_avail() {
            display_showcmd();
        }
    }
}

pub fn push_showcmd() {
    unsafe {
        if opt_vars().p_sc != 0 {
            { let s = (&raw const g_st().showcmd_buf).cast::<c_char>(); std::ptr::copy_nonoverlapping(s, (&raw mut norm_st().old_showcmd_buf).cast::<c_char>(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
        }
    }
}

pub fn pop_showcmd() {
    unsafe {
        if opt_vars().p_sc == 0 {
            return;
        }
        { let s = (&raw const norm_st().old_showcmd_buf).cast::<c_char>(); std::ptr::copy_nonoverlapping(s, (&raw mut g_st().showcmd_buf).cast::<c_char>(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
        display_showcmd();
    }
}

pub fn showcmd_update_clear_state() {
    norm_st().showcmd_is_clear = g_st().showcmd_buf[0] == NUL;
}

pub fn display_showcmd() {
    unsafe {
        if mc_replaying() {
            return;
        }

        showcmd_update_clear_state();

        if *opt_vars().p_sloc == b's' as c_char {
            if norm_st().showcmd_is_clear && g_st().vgetc_busy == 0 {
                (**curwin.as_ptr()).w_redr_status = true;
            } else {
                win_redr_status(&mut **curwin.as_ptr());
                setcursor();
            }
            return;
        }
        if *opt_vars().p_sloc == b't' as c_char {
            if norm_st().showcmd_is_clear && g_st().vgetc_busy == 0 {
                g_st().redraw_tabline = true;
            } else {
                draw_tabline();
                setcursor();
            }
            return;
        }

        let mut mc_buf: [c_char; 16] = [0; 16];
        let mc_len = mc_showcmd(mc_buf.as_mut_ptr(), mc_buf.len());
        let sc_buf = (&raw const g_st().showcmd_buf).cast::<c_char>();

        if ui_has(kUIMessages) {
            let mut content_items: [Object; 1] = [NIL_OBJ];
            let mut chunk_items: [Object; 3] = [NIL_OBJ; 3];
            let mut ext_buf: [c_char; 16 + SHOWCMD_BUFLEN] = [0; 16 + SHOWCMD_BUFLEN];
            { let s = mc_buf.as_ptr(); std::ptr::copy_nonoverlapping(s, ext_buf.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            xstrlcpy(
                ext_buf.as_mut_ptr().add(mc_len),
                if norm_st().showcmd_is_clear {
                    c"".as_ptr()
                } else {
                    sc_buf
                },
                ext_buf.len() - mc_len,
            );
            let mut size = 0;
            if ext_buf[0] != NUL {
                chunk_items[0] = integer_obj(0);
                chunk_items[1] = cstr_as_obj(ext_buf.as_ptr());
                chunk_items[2] = integer_obj(0);
                let chunk = Array {
                    items: chunk_items.as_mut_ptr(),
                    size: 3,
                    capacity: 3,
                };
                content_items[0] = array_obj(chunk);
                size = 1;
            }
            let content = Array {
                items: content_items.as_mut_ptr(),
                size,
                capacity: 1,
            };
            ui_call_msg_showcmd(content);
            return;
        }
        if opt_vars().p_ch == 0 {
            return;
        }

        msg_grid_validate();
        let showcmd_row = g_st().Rows - 1;
        grid_line_start(&raw mut g_st().msg_grid_adj, showcmd_row);

        let mut len = if mc_len > 0 {
            grid_line_puts(g_st().sc_col, mc_buf.as_ptr(), -1, 0)
        } else {
            0
        };
        if !norm_st().showcmd_is_clear {
            len += grid_line_puts(g_st().sc_col + len, sc_buf, -1, hl_attr(HLF_MSG as c_int));
        }
        len = len.min(SHOWCMD_COLS as c_int);

        grid_line_puts(
            g_st().sc_col + len,
            c"          ".as_ptr().add(len as usize),
            -1,
            hl_attr(HLF_MSG as c_int),
        );

        grid_line_flush();
    }
}

pub fn get_vtopline(wp: &mut win_T) -> c_int {
    plines_m_win_fill(&raw mut *wp, 1, wp.w_topline) - wp.w_topfill
}

pub fn do_check_scrollbind(check: bool) {
    unsafe {
        let vtopline = get_vtopline(&mut **curwin.as_ptr());

        if check && (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
            if g_st().did_syncbind {
                g_st().did_syncbind = false;
            } else if ptr::eq(*curwin.as_ptr(), norm_st().do_check_scrollbind_old_curwin) {
                if (ptr::eq((**curwin.as_ptr()).w_buffer, norm_st().do_check_scrollbind_old_buf)
                    || (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0)
                    && (vtopline != norm_st().do_check_scrollbind_old_vtopline
                        || (**curwin.as_ptr()).w_leftcol != norm_st().do_check_scrollbind_old_leftcol)
                {
                    check_scrollbind(
                        vtopline - norm_st().do_check_scrollbind_old_vtopline,
                        (**curwin.as_ptr()).w_leftcol - norm_st().do_check_scrollbind_old_leftcol,
                    );
                }
            } else if !vim_strchr(opt_vars().p_sbo, b'j' as c_int).is_null() {
                check_scrollbind(vtopline - (**curwin.as_ptr()).w_scbind_pos, 0);
            }
            (**curwin.as_ptr()).w_scbind_pos = vtopline;
        }

        norm_st().do_check_scrollbind_old_curwin = *curwin.as_ptr();
        norm_st().do_check_scrollbind_old_vtopline = vtopline;
        norm_st().do_check_scrollbind_old_buf = (**curwin.as_ptr()).w_buffer;
        norm_st().do_check_scrollbind_old_leftcol = (**curwin.as_ptr()).w_leftcol;
    }
}

pub fn check_scrollbind(vtopline_diff: linenr_T, leftcol_diff: c_int) {
    unsafe {
        let old_curwin = *curwin.as_ptr();
        let old_curbuf = g_st().curbuf;
        let old_VIsual_select = g_st().Visual.select;
        let old_VIsual_active = g_st().Visual.active;
        let tgt_leftcol = (**curwin.as_ptr()).w_leftcol;

        let want_ver = (*old_curwin).w_onebuf_opt.wo_diff != 0
            || (!vim_strchr(opt_vars().p_sbo, b'v' as c_int).is_null() && vtopline_diff != 0);
        let want_hor = !vim_strchr(opt_vars().p_sbo, b'h' as c_int).is_null()
            && (leftcol_diff != 0 || vtopline_diff != 0);

        g_st().Visual.select = false;
        g_st().Visual.active = false;
        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            *curwin.as_ptr() = wp;
            g_st().curbuf = (**curwin.as_ptr()).w_buffer;
            if !ptr::eq(*curwin.as_ptr(), old_curwin) && (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
                if want_ver {
                    if (*old_curwin).w_onebuf_opt.wo_diff != 0
                        && (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0
                    {
                        diff_set_topline(&mut *old_curwin, &mut **curwin.as_ptr());
                    } else {
                        (**curwin.as_ptr()).w_scbind_pos += vtopline_diff;
                        let curr_vtopline = get_vtopline(&mut **curwin.as_ptr());
                        let max_vtopline = curr_vtopline
                            + (**curwin.as_ptr()).w_topfill
                            + plines_m_win_fill(
                                *curwin.as_ptr(),
                                (**curwin.as_ptr()).w_topline + 1,
                                (*g_st().curbuf).b_ml.ml_line_count,
                            );
                        let new_vtopline = (**curwin.as_ptr()).w_scbind_pos.min(max_vtopline).max(1);
                        let y = new_vtopline - curr_vtopline;
                        if y > 0 {
                            scrollup(&mut **curwin.as_ptr(), y, false);
                        } else {
                            scrolldown(&mut **curwin.as_ptr(), -y, 0);
                        }
                    }

                    redraw_later(*curwin.as_ptr(), UPD_VALID);
                    cursor_correct(&mut **curwin.as_ptr());
                    (**curwin.as_ptr()).w_redr_status = true;
                }

                if want_hor {
                    set_leftcol(tgt_leftcol);
                }
            }
            wp = (*wp).w_next;
        }

        g_st().Visual.select = old_VIsual_select;
        g_st().Visual.active = old_VIsual_active;
        *curwin.as_ptr() = old_curwin;
        g_st().curbuf = old_curbuf;
    }
}

fn nv_ignore(cap: &mut cmdarg_T) {
    cap.retval |= CA_COMMAND_BUSY;
}

fn nv_nop(_cap: &mut cmdarg_T) {}

fn nv_error(cap: &mut cmdarg_T) {
    unsafe {
        clearopbeep(&mut *cap.oap);
    }
}

fn nv_help(cap: &mut cmdarg_T) {
    unsafe {
        if !checkclearopq(&mut *cap.oap) {
            ex_help(&mut crate::ex_docmd::exarg_T::new());
        }
    }
}

fn nv_addsub(cap: &mut cmdarg_T) {
    unsafe {
        if bt_prompt(g_st().curbuf) && !prompt_curpos_editable() {
            clearopbeep(&mut *cap.oap);
        } else if !g_st().Visual.active && (*cap.oap).op_type == OP_NOP {
            prep_redo_cmd(cap);
            (*cap.oap).op_type = if cap.cmdchar == Ctrl_A {
                OP_NR_ADD
            } else {
                OP_NR_SUB
            };
            op_addsub(&mut *cap.oap, cap.count1, cap.arg != 0);
            (*cap.oap).op_type = OP_NOP;
        } else if g_st().Visual.active {
            nv_operator(cap);
        } else {
            clearop(&mut *cap.oap);
        }
    }
}

fn nv_page(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearop(&mut *cap.oap) {
            return;
        }

        if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            if cap.arg == BACKWARD {
                goto_tabpage(-cap.count1);
            } else {
                goto_tabpage(cap.count0);
            }
        } else {
            pagescroll(cap.arg, cap.count1, false);
        }
    }
}

fn nv_gd(oap: &mut oparg_T, nchar: c_int, thisblock: c_int) {
    let mut ptr: *mut c_char = ptr::null_mut();
        let len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
        if len == 0
            || !find_decl(
                ptr,
                len,
                nchar == b'd' as c_int,
                thisblock != 0,
                SEARCH_START,
            )
        {
            clearopbeep(&mut *oap);
            return;
        }

        if (opt_vars().fdo_flags & kOptFdoFlagSearch) != 0 && g_st().KeyTyped && oap.op_type == OP_NOP {
            foldOpenCursor();
        }
        if messaging() && g_st().msg_silent == 0 && !shortmess(kShmSearchcount) {
            g_st().clear_cmdline = true;
        }
}

fn is_ident(line: *const c_char, offset: c_int) -> bool {
    unsafe {
        let mut incomment = false;
        let mut instring: c_int = 0;
        let mut prev: c_int = 0;

        let mut i: c_int = 0;
        while i < offset && *line.add(i as usize) != NUL {
            let cur = c_int::from(*line.add(i as usize) as u8);
            if instring != 0 {
                if prev != b'\\' as c_int && cur == instring {
                    instring = 0;
                }
            } else if (cur == b'"' as c_int || cur == b'\'' as c_int) && !incomment {
                instring = cur;
            } else if incomment {
                if prev == b'*' as c_int && cur == b'/' as c_int {
                    incomment = false;
                }
            } else if prev == b'/' as c_int && cur == b'*' as c_int {
                incomment = true;
            } else if prev == b'/' as c_int && cur == b'/' as c_int {
                return false;
            }

            prev = cur;
            i += 1;
        }

        !incomment && instring == 0
    }
}

pub fn find_decl(
    ptr: *mut c_char,
    len: usize,
    locally: bool,
    thisblock: bool,
    flags_arg: c_int,
) -> bool {
    unsafe {
        let par_pos: pos_T;
        let mut found_pos: pos_T = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut t: bool;
        let mut retval = true;
        let mut incll = false;
        let mut searchflags = flags_arg;

        let patsize = len + 7;
        let mut pat_vec = vec![0 as c_char; patsize];
        let pat: *mut c_char = pat_vec.as_mut_ptr();

        assert!(patsize <= c_int::MAX as usize);
        let patlen = vim_snprintf!(
            pat,
            patsize,
            if vim_iswordp(ptr) {
                c"\\V\\<%.*s\\>".as_ptr()
            } else {
                c"\\V%.*s".as_ptr()
            },
            len as c_int,
            ptr,
        ) as usize;

        let old_pos = (**curwin.as_ptr()).w_cursor;
        let save_p_ws = opt_vars().p_ws;
        let save_p_scs = opt_vars().p_scs;
        opt_vars().p_ws = 0;
        opt_vars().p_scs = 0;

        if !locally || !findpar(&raw mut incll, BACKWARD, 1, b'{' as c_int, false) {
            setpcmark();
            (**curwin.as_ptr()).w_cursor.lnum = 1;
            par_pos = (**curwin.as_ptr()).w_cursor;
        } else {
            par_pos = (**curwin.as_ptr()).w_cursor;
            while (**curwin.as_ptr()).w_cursor.lnum > 1 && *skipwhite(get_cursor_line_ptr()) != NUL {
                (**curwin.as_ptr()).w_cursor.lnum -= 1;
            }
        }
        (**curwin.as_ptr()).w_cursor.col = 0;

        clearpos(&mut found_pos);
        loop {
            t = searchit(
                *curwin.as_ptr(),
                g_st().curbuf,
                &raw mut (**curwin.as_ptr()).w_cursor,
                None,
                FORWARD,
                pat,
                patlen,
                1,
                searchflags,
                RE_LAST,
                false,
                ptr::null_mut(),
            ) != 0;
            if (**curwin.as_ptr()).w_cursor.lnum >= old_pos.lnum {
                t = false;
            }

            if thisblock && t {
                let maxtravel = i64::from(old_pos.lnum - (**curwin.as_ptr()).w_cursor.lnum + 1);
                let pos = findmatchlimit(ptr::null_mut(), b'}' as c_int, FM_FORWARD, maxtravel);

                if !pos.is_null() && (*pos).lnum < old_pos.lnum {
                    (**curwin.as_ptr()).w_cursor = *pos;
                    continue;
                }
            }

            if !t {
                if found_pos.lnum != 0 {
                    (**curwin.as_ptr()).w_cursor = found_pos;
                    t = true;
                }
                break;
            }
            if get_leader_len(get_cursor_line_ptr(), ptr::null_mut(), false, true) > 0 {
                (**curwin.as_ptr()).w_cursor.lnum += 1;
                (**curwin.as_ptr()).w_cursor.col = 0;
                continue;
            }
            let valid = is_ident(get_cursor_line_ptr(), (**curwin.as_ptr()).w_cursor.col);

            if !valid && found_pos.lnum != 0 {
                (**curwin.as_ptr()).w_cursor = found_pos;
                break;
            }
            if valid && !locally {
                break;
            }
            if valid && (**curwin.as_ptr()).w_cursor.lnum >= par_pos.lnum {
                if found_pos.lnum != 0 {
                    (**curwin.as_ptr()).w_cursor = found_pos;
                }
                break;
            }

            if !valid {
                clearpos(&mut found_pos);
            } else {
                found_pos = (**curwin.as_ptr()).w_cursor;
            }
            searchflags &= !SEARCH_START;
        }

        if !t {
            retval = false;
            (**curwin.as_ptr()).w_cursor = old_pos;
        } else {
            (**curwin.as_ptr()).w_set_curswant = true;
            reset_search_dir();
        }

        drop(pat_vec);
        opt_vars().p_ws = save_p_ws;
        opt_vars().p_scs = save_p_scs;

        retval
    }
}

pub fn nv_screengo(
    oap: &mut oparg_T,
    dir: c_int,
    mut dist: c_int,
    skip_conceal: bool,
) -> bool {
    unsafe {
        let mut linelen = linetabsize(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
        let mut retval = true;
        let mut atend = false;

        oap.motion_type = kMTCharWise;
        oap.inclusive = (**curwin.as_ptr()).w_curswant == MAXCOL;

        let col_off1 = win_col_off(*curwin.as_ptr());
        let col_off2 = col_off1 - win_col_off2(*curwin.as_ptr());
        let width1 = (**curwin.as_ptr()).w_view_width - col_off1;
        let mut width2 = (**curwin.as_ptr()).w_view_width - col_off2;

        if width2 == 0 {
            width2 = 1;
        }

        if (**curwin.as_ptr()).w_view_width != 0 {
            let mut n: c_int;
            if (**curwin.as_ptr()).w_curswant == MAXCOL {
                atend = true;
                validate_virtcol(*curwin.as_ptr());
                if width1 <= 0 {
                    (**curwin.as_ptr()).w_curswant = 0;
                } else {
                    (**curwin.as_ptr()).w_curswant = width1 - 1;
                    if (**curwin.as_ptr()).w_virtcol > (**curwin.as_ptr()).w_curswant {
                        (**curwin.as_ptr()).w_curswant +=
                            (((**curwin.as_ptr()).w_virtcol - (**curwin.as_ptr()).w_curswant - 1) / width2 + 1)
                                * width2;
                    }
                }
            } else {
                if linelen > width1 {
                    n = ((linelen - width1 - 1) / width2 + 1) * width2 + width1;
                } else {
                    n = width1;
                }
                (**curwin.as_ptr()).w_curswant = (**curwin.as_ptr()).w_curswant.min(n - 1);
            }

            while dist > 0 {
                dist -= 1;
                if dir == BACKWARD {
                    if (**curwin.as_ptr()).w_curswant >= width1
                        && !hasFolding(
                            *curwin.as_ptr(),
                            (**curwin.as_ptr()).w_cursor.lnum,
                            ptr::null_mut(),
                            ptr::null_mut(),
                        )
                    {
                        (**curwin.as_ptr()).w_curswant -= width2;
                    } else {
                        if (**curwin.as_ptr()).w_cursor.lnum <= 1 {
                            retval = false;
                            break;
                        }
                        cursor_up_inner(&mut **curwin.as_ptr(), 1, skip_conceal);

                        linelen = linetabsize(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
                        if linelen > width1 {
                            let w = (((linelen - width1 - 1) / width2) + 1) * width2;
                            assert!(w <= 0 || (**curwin.as_ptr()).w_curswant <= c_int::MAX - w);
                            (**curwin.as_ptr()).w_curswant += w;
                        }
                    }
                } else {
                    if linelen > width1 {
                        n = ((linelen - width1 - 1) / width2 + 1) * width2 + width1;
                    } else {
                        n = width1;
                    }
                    if (**curwin.as_ptr()).w_curswant + width2 < n
                        && !hasFolding(
                            *curwin.as_ptr(),
                            (**curwin.as_ptr()).w_cursor.lnum,
                            ptr::null_mut(),
                            ptr::null_mut(),
                        )
                    {
                        (**curwin.as_ptr()).w_curswant += width2;
                    } else {
                        if (**curwin.as_ptr()).w_cursor.lnum >= (*(**curwin.as_ptr()).w_buffer).b_ml.ml_line_count {
                            retval = false;
                            break;
                        }
                        cursor_down_inner(&mut **curwin.as_ptr(), 1, skip_conceal);
                        (**curwin.as_ptr()).w_curswant %= width2;

                        if (**curwin.as_ptr()).w_curswant >= width1 {
                            (**curwin.as_ptr()).w_curswant -= width2;
                        }
                        linelen = linetabsize(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
                    }
                }
            }
        }

        if virtual_active(*curwin.as_ptr()) && atend {
            coladvance(&mut **curwin.as_ptr(), MAXCOL);
        } else {
            coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
        }

        if (**curwin.as_ptr()).w_cursor.col > 0 && (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 {
            validate_virtcol(*curwin.as_ptr());
            let mut virtcol = (**curwin.as_ptr()).w_virtcol;
            if virtcol > width1 && *get_showbreak_value(&mut **curwin.as_ptr()) != NUL {
                virtcol -= vim_strsize(get_showbreak_value(&mut **curwin.as_ptr()));
            }

            let c = utf_ptr2char(get_cursor_pos_ptr());
            if dir == FORWARD
                && virtcol < (**curwin.as_ptr()).w_curswant
                && (**curwin.as_ptr()).w_curswant <= width1
                && !vim_isprintc(c)
                && c > 255
            {
                oneright();
            }

            if virtcol > (**curwin.as_ptr()).w_curswant
                && (if (**curwin.as_ptr()).w_curswant < width1 {
                    (**curwin.as_ptr()).w_curswant > width1 / 2
                } else {
                    ((**curwin.as_ptr()).w_curswant - width1) % width2 > width2 / 2
                })
            {
                (**curwin.as_ptr()).w_cursor.col -= 1;
            }
        }

        if atend {
            (**curwin.as_ptr()).w_curswant = MAXCOL;
        }
        adjust_skipcol();

        retval
    }
}

pub fn nv_scroll_line(cap: &mut cmdarg_T) {
    unsafe {
        if !checkclearop(&mut *cap.oap) {
            scroll_redraw(cap.arg, cap.count1);
        }
    }
}

fn nv_z_get_count(cap: &mut cmdarg_T, nchar_arg: &mut c_int) -> bool {
    unsafe {
        let mut nchar = *nchar_arg;

        if checkclearop(&mut *cap.oap) {
            return false;
        }
        let mut n = nchar - b'0' as c_int;

        loop {
            g_st().no_mapping += 1;
            g_st().allow_keys += 1;
            nchar = plain_vgetc();
            langmap_adjust(&mut nchar, true);
            g_st().no_mapping -= 1;
            g_st().allow_keys -= 1;
            add_to_showcmd(nchar);

            if nchar == K_DEL || nchar == K_KDEL {
                n /= 10;
            } else if ascii_isdigit(nchar) {
                if vim_append_digit_int(&mut n, nchar - b'0' as c_int) == FAIL {
                    clearopbeep(&mut *cap.oap);
                    break;
                }
            } else if nchar == CAR {
                win_setheight(n);
                break;
            } else if nchar == b'l' as c_int
                || nchar == b'h' as c_int
                || nchar == K_LEFT
                || nchar == K_RIGHT
            {
                cap.count1 = if n != 0 {
                    n * cap.count1
                } else {
                    cap.count1
                };
                *nchar_arg = nchar;
                return true;
            } else {
                clearopbeep(&mut *cap.oap);
                break;
            }
        }
        (*cap.oap).op_type = OP_NOP;
        false
    }
}

fn nv_zg_zw(cap: &mut cmdarg_T, mut nchar: c_int) -> c_int {
    unsafe {
        let mut undo = false;

        if nchar == b'u' as c_int {
            g_st().no_mapping += 1;
            g_st().allow_keys += 1;
            nchar = plain_vgetc();
            langmap_adjust(&mut nchar, true);
            g_st().no_mapping -= 1;
            g_st().allow_keys -= 1;
            add_to_showcmd(nchar);

            if vim_strchr(c"gGwW".as_ptr(), nchar).is_null() {
                clearopbeep(&mut *cap.oap);
                return OK;
            }
            undo = true;
        }

        if checkclearop(&mut *cap.oap) {
            return OK;
        }
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        if g_st().Visual.active && !get_visual_text(Some(cap), &mut ptr, &mut len) {
            return FAIL;
        }
        if ptr.is_null() {
            let pos = (**curwin.as_ptr()).w_cursor;

            g_st().emsg_off += 1;
            len = spell_move_to(&mut **curwin.as_ptr(), FORWARD, SMT_ALL, true, ptr::null_mut());
            g_st().emsg_off -= 1;
            if len != 0 && (**curwin.as_ptr()).w_cursor.col <= pos.col {
                ptr = ml_get_pos(&(**curwin.as_ptr()).w_cursor);
            }
            (**curwin.as_ptr()).w_cursor = pos;
        }

        if ptr.is_null() {
            len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
            if len == 0 {
                return FAIL;
            }
        }
        assert!(len <= c_int::MAX as usize);
        spell_add_word(
            ptr,
            len as c_int,
            if nchar == b'w' as c_int || nchar == b'W' as c_int {
                SPELL_ADD_BAD
            } else {
                SPELL_ADD_GOOD
            },
            if nchar == b'G' as c_int || nchar == b'W' as c_int {
                0
            } else {
                cap.count1
            },
            undo,
        );

        OK
    }
}

fn nv_zet(cap: &mut cmdarg_T) {
    unsafe {
        let mut col: colnr_T = 0;
        let mut nchar = cap.nchar;
        let mut foldlevel: OptInt = -1;
        let mut foldenable: c_int = kNone;

        let siso = get_sidescrolloff_value(*curwin.as_ptr());

        if ascii_isdigit(nchar) && !nv_z_get_count(cap, &mut nchar) {
            return;
        }

        if cap.nchar != b'f' as c_int
            && cap.nchar != b'F' as c_int
            && !(g_st().Visual.active && !vim_strchr(c"dcCoO".as_ptr(), cap.nchar).is_null())
            && cap.nchar != b'j' as c_int
            && cap.nchar != b'k' as c_int
            && checkclearop(&mut *cap.oap)
        {
            return;
        }

        if !vim_strchr(c"+\r\nt.z^-b".as_ptr(), nchar).is_null()
            && cap.count0 != 0
            && cap.count0 != (**curwin.as_ptr()).w_cursor.lnum
        {
            setpcmark();
            if cap.count0 > (*g_st().curbuf).b_ml.ml_line_count {
                (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_ml.ml_line_count;
            } else {
                (**curwin.as_ptr()).w_cursor.lnum = cap.count0;
            }
            check_cursor_col(&mut **curwin.as_ptr());
        }

        match nchar {
            c if c == b'+' as c_int
                || c == NL
                || c == CAR
                || c == K_KENTER
                || c == b't' as c_int =>
            {
                if nchar == b'+' as c_int && cap.count0 == 0 {
                    validate_botline_win(*curwin.as_ptr());
                    (**curwin.as_ptr()).w_cursor.lnum = (**curwin.as_ptr()).w_botline.min((*g_st().curbuf).b_ml.ml_line_count);
                }
                if nchar != b't' as c_int {
                    beginline(BL_WHITE | BL_FIX);
                }
                scroll_cursor_top(&mut **curwin.as_ptr(), 0, 1);
                redraw_later(*curwin.as_ptr(), UPD_VALID);
                set_fraction(*curwin.as_ptr());
            }
            c if c == b'.' as c_int || c == b'z' as c_int => {
                if nchar == b'.' as c_int {
                    beginline(BL_WHITE | BL_FIX);
                }
                scroll_cursor_halfway(&mut **curwin.as_ptr(), true, false);
                redraw_later(*curwin.as_ptr(), UPD_VALID);
                set_fraction(*curwin.as_ptr());
            }
            c if c == b'^' as c_int || c == b'-' as c_int || c == b'b' as c_int => {
                if nchar == b'^' as c_int {
                    if cap.count0 != 0 {
                        scroll_cursor_bot(&mut **curwin.as_ptr(), 0, true);
                        (**curwin.as_ptr()).w_cursor.lnum = (**curwin.as_ptr()).w_topline;
                    } else if (**curwin.as_ptr()).w_topline == 1 {
                        (**curwin.as_ptr()).w_cursor.lnum = 1;
                    } else {
                        (**curwin.as_ptr()).w_cursor.lnum = (**curwin.as_ptr()).w_topline - 1;
                    }
                }
                if nchar != b'b' as c_int {
                    beginline(BL_WHITE | BL_FIX);
                }
                scroll_cursor_bot(&mut **curwin.as_ptr(), 0, true);
                redraw_later(*curwin.as_ptr(), UPD_VALID);
                set_fraction(*curwin.as_ptr());
            }
            c if c == b'H' as c_int || c == b'h' as c_int || c == K_LEFT => {
                if nchar == b'H' as c_int {
                    cap.count1 *= (**curwin.as_ptr()).w_view_width / 2;
                }
                if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
                    set_leftcol(if cap.count1 > (**curwin.as_ptr()).w_leftcol {
                        0
                    } else {
                        (**curwin.as_ptr()).w_leftcol - cap.count1
                    });
                }
            }
            c if c == b'L' as c_int || c == b'l' as c_int || c == K_RIGHT => {
                if nchar == b'L' as c_int {
                    cap.count1 *= (**curwin.as_ptr()).w_view_width / 2;
                }
                if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
                    set_leftcol((**curwin.as_ptr()).w_leftcol + cap.count1);
                }
            }
            c if c == b's' as c_int => {
                if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
                    if hasFolding(
                        *curwin.as_ptr(),
                        (**curwin.as_ptr()).w_cursor.lnum,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    ) {
                        col = 0;
                    } else {
                        getvcol(
                            &mut **curwin.as_ptr(),
                            &mut (**curwin.as_ptr()).w_cursor,
                            &raw mut col,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            0,
                        );
                    }
                    if i64::from(col) > siso {
                        col -= siso as colnr_T;
                    } else {
                        col = 0;
                    }
                    if (**curwin.as_ptr()).w_leftcol != col {
                        (**curwin.as_ptr()).w_leftcol = col;
                        redraw_later(*curwin.as_ptr(), UPD_NOT_VALID);
                    }
                }
            }
            c if c == b'e' as c_int => {
                if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
                    if hasFolding(
                        *curwin.as_ptr(),
                        (**curwin.as_ptr()).w_cursor.lnum,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    ) {
                        col = 0;
                    } else {
                        getvcol(
                            &mut **curwin.as_ptr(),
                            &mut (**curwin.as_ptr()).w_cursor,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            &raw mut col,
                            0,
                        );
                    }
                    let n = i64::from((**curwin.as_ptr()).w_view_width - win_col_off(*curwin.as_ptr()));
                    if i64::from(col) + siso < n {
                        col = 0;
                    } else if siso - n < i64::from(c_int::MAX - col) {
                        col = (i64::from(col) + siso - n + 1) as colnr_T;
                    } else {
                        col = c_int::MAX;
                    }
                    if (**curwin.as_ptr()).w_leftcol != col {
                        (**curwin.as_ptr()).w_leftcol = col;
                        redraw_later(*curwin.as_ptr(), UPD_NOT_VALID);
                    }
                }
            }
            c if c == b'P' as c_int || c == b'p' as c_int => {
                nv_put(cap);
            }
            c if c == b'y' as c_int => {
                nv_operator(cap);
            }
            c if c == b'q' as c_int => {
                if g_st().reg_recording != 0 || g_st().reg_executing != 0 {
                    clearopbeep(&mut *cap.oap);
                } else {
                    mc_zq_start(cap);
                    nv_operator(cap);
                }
            }
            c if c == b'F' as c_int || c == b'f' as c_int => {
                if foldManualAllowed(true) != 0 {
                    cap.nchar = b'f' as c_int;
                    nv_operator(cap);
                    foldenable = 1;
                } else {
                    clearopbeep(&mut *cap.oap);
                }
            }
            c if c == b'd' as c_int || c == b'D' as c_int => {
                if foldManualAllowed(false) != 0 {
                    if g_st().Visual.active {
                        nv_operator(cap);
                    } else {
                        deleteFold(
                            &mut **curwin.as_ptr(),
                            (**curwin.as_ptr()).w_cursor.lnum,
                            (**curwin.as_ptr()).w_cursor.lnum,
                            c_int::from(nchar == b'D' as c_int),
                            false,
                        );
                    }
                }
            }
            c if c == b'E' as c_int => {
                if foldmethodIsManual(&mut **curwin.as_ptr()) {
                    clearFolding(&mut **curwin.as_ptr());
                    changed_window_setting(&mut **curwin.as_ptr());
                } else if foldmethodIsMarker(&mut **curwin.as_ptr()) {
                    deleteFold(&mut **curwin.as_ptr(), 1, (*g_st().curbuf).b_ml.ml_line_count, 1, false);
                } else {
                    emsg(tr(
                        c"E352: Cannot erase folds with current 'foldmethod'".as_ptr()
                    ));
                }
            }
            c if c == b'n' as c_int => {
                foldenable = 0;
            }
            c if c == b'N' as c_int => {
                foldenable = 1;
            }
            c if c == b'i' as c_int => {
                foldenable = c_int::from((**curwin.as_ptr()).w_onebuf_opt.wo_fen == 0);
            }
            c if c == b'a' as c_int => {
                if hasFolding(
                    *curwin.as_ptr(),
                    (**curwin.as_ptr()).w_cursor.lnum,
                    ptr::null_mut(),
                    ptr::null_mut(),
                ) {
                    openFold((**curwin.as_ptr()).w_cursor, cap.count1);
                } else {
                    closeFold((**curwin.as_ptr()).w_cursor, cap.count1);
                    foldenable = 1;
                }
            }
            c if c == b'A' as c_int => {
                if hasFolding(
                    *curwin.as_ptr(),
                    (**curwin.as_ptr()).w_cursor.lnum,
                    ptr::null_mut(),
                    ptr::null_mut(),
                ) {
                    openFoldRecurse((**curwin.as_ptr()).w_cursor);
                } else {
                    closeFoldRecurse((**curwin.as_ptr()).w_cursor);
                    foldenable = 1;
                }
            }
            c if c == b'o' as c_int => {
                if g_st().Visual.active {
                    nv_operator(cap);
                } else {
                    openFold((**curwin.as_ptr()).w_cursor, cap.count1);
                }
            }
            c if c == b'O' as c_int => {
                if g_st().Visual.active {
                    nv_operator(cap);
                } else {
                    openFoldRecurse((**curwin.as_ptr()).w_cursor);
                }
            }
            c if c == b'c' as c_int => {
                if g_st().Visual.active {
                    nv_operator(cap);
                } else {
                    closeFold((**curwin.as_ptr()).w_cursor, cap.count1);
                }
                foldenable = 1;
            }
            c if c == b'C' as c_int => {
                if g_st().Visual.active {
                    nv_operator(cap);
                } else {
                    closeFoldRecurse((**curwin.as_ptr()).w_cursor);
                }
                foldenable = 1;
            }
            c if c == b'v' as c_int => {
                foldOpenCursor();
            }
            c if c == b'x' as c_int || c == b'X' as c_int => {
                foldenable = 1;
                (**curwin.as_ptr()).w_foldinvalid = true;
                foldlevel = (**curwin.as_ptr()).w_onebuf_opt.wo_fdl;
            }
            c if c == b'm' as c_int => {
                foldlevel = ((**curwin.as_ptr()).w_onebuf_opt.wo_fdl - i64::from(cap.count1)).max(0);
                foldenable = 1;
            }
            c if c == b'M' as c_int => {
                foldlevel = 0;
                foldenable = 1;
            }
            c if c == b'r' as c_int => {
                let depth = i64::from(getDeepestNesting(&mut **curwin.as_ptr()));
                let level = ((**curwin.as_ptr()).w_onebuf_opt.wo_fdl + i64::from(cap.count1)).min(depth);
                if level != (**curwin.as_ptr()).w_onebuf_opt.wo_fdl {
                    foldlevel = level;
                }
            }
            c if c == b'R' as c_int => {
                foldlevel = i64::from(getDeepestNesting(&mut **curwin.as_ptr()));
            }
            c if c == b'j' as c_int || c == b'k' as c_int => {
                if foldMoveTo(
                    true,
                    if nchar == b'j' as c_int {
                        FORWARD
                    } else {
                        BACKWARD
                    },
                    cap.count1,
                ) == 0
                {
                    clearopbeep(&mut *cap.oap);
                }
            }
            c if c == b'u' as c_int
                || c == b'g' as c_int
                || c == b'w' as c_int
                || c == b'G' as c_int
                || c == b'W' as c_int =>
            {
                if nv_zg_zw(cap, nchar) == FAIL {
                    return;
                }
            }
            c if c == b'=' as c_int => {
                if !checkclearop(&mut *cap.oap) {
                    spell_suggest(cap.count0);
                }
            }
            _ => {
                clearopbeep(&mut *cap.oap);
            }
        }

        let win_handle = (**curwin.as_ptr()).handle;
        let buf_handle = (*g_st().curbuf).b_fnum;
        if foldenable != kNone && foldenable != (**curwin.as_ptr()).w_onebuf_opt.wo_fen {
            set_option_value(
                kOptFoldenable,
                boolean_obj(foldenable != 0),
                OPT_LOCAL,
                true,
                None,
            );
            if (**curwin.as_ptr()).handle != win_handle || (*g_st().curbuf).b_fnum != buf_handle {
                clearop(&mut *cap.oap);
                return;
            }
        }
        if nchar == b'F' as c_int && (*cap.oap).op_type == OP_FOLD {
            nv_operator(cap);
            g_st().finish_op = true;
        }
        if foldlevel >= 0 {
            set_option_value(
                kOptFoldlevel,
                integer_obj(foldlevel),
                OPT_LOCAL,
                true,
                None,
            );
        }
        if nchar == b'x' as c_int
            && (**curwin.as_ptr()).handle == win_handle
            && (*g_st().curbuf).b_fnum == buf_handle
        {
            foldOpenCursor();
        }
    }
}

fn nv_Q(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().reg_recording != 0 || g_st().reg_executing != 0 || (!g_st().Visual.active && cap.count0 > 2) {
            vim_beep(0);
        } else if !checkclearop(&mut *cap.oap) {
            if g_st().Visual.active {
                do_cmdline_cmd(c"normal! zqj".as_ptr());
                mc_follow_set(kTrue);
            } else {
                mc_toggle(
                    g_st().curbuf,
                    (**curwin.as_ptr()).w_cursor,
                    true,
                    if cap.count0 == 0 {
                        kNone
                    } else if cap.count0 == 1 {
                        kTrue
                    } else {
                        kFalse
                    },
                );
            }
        }
    }
}

fn nv_colon(cap: &mut cmdarg_T) {
    unsafe {
        let is_cmdkey = cap.cmdchar == K_COMMAND;
        let is_lua = cap.cmdchar == K_LUA;

        if g_st().Visual.active && !is_cmdkey && !is_lua {
            nv_operator(cap);
            return;
        }

        if (*cap.oap).op_type != OP_NOP {
            (*cap.oap).motion_type = kMTCharWise;
            (*cap.oap).inclusive = false;
        } else if cap.count0 != 0 && !is_cmdkey && !is_lua {
            stuffcharReadbuff(b'.' as c_int);
            if cap.count0 > 1 {
                stuffReadbuff(c",.+".as_ptr());
                stuffnumReadbuff(cap.count0 - 1);
            }
        }

        if g_st().KeyTyped {
            msg_ext_set_trigger(c"typed_cmd".as_ptr());
            compute_cmdrow();
        }

        let cmd_result = if is_lua {
            map_execute_lua(true, false)
        } else {
            do_cmdline(
                ptr::null_mut(),
                if is_cmdkey {
                    crate::cmdexpand::LineGetter::CmdKeyCmd
                } else {
                    crate::cmdexpand::LineGetter::ExLine
                },
                if (*cap.oap).op_type != OP_NOP {
                    DOCMD_KEEPLINE
                } else {
                    0
                },
            ) != 0
        };
        msg_ext_set_trigger(c"".as_ptr());

        if !cmd_result {
            clearop(&mut *cap.oap);
        } else if (*cap.oap).op_type != OP_NOP
            && ((*cap.oap).start.lnum > (*g_st().curbuf).b_ml.ml_line_count
                || (*cap.oap).start.col > ml_get_len((*cap.oap).start.lnum)
                || g_st().did_emsg != 0)
        {
            clearopbeep(&mut *cap.oap);
        }
    }
}

fn nv_ctrlg(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().Visual.active {
            g_st().Visual.select = !g_st().Visual.select;
            may_trigger_modechanged();
            showmode();
        } else if !checkclearop(&mut *cap.oap) {
            fileinfo(cap.count0, 0, true);
        }
    }
}

fn nv_ctrlh(cap: &mut cmdarg_T) {
    if g_st().Visual.active && g_st().Visual.select {
            cap.cmdchar = b'x' as c_int;
            v_visop(cap);
        } else {
            nv_left(cap);
        }
}

fn nv_clear(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearop(&mut *cap.oap) {
            return;
        }

        syn_stack_free_all((**curwin.as_ptr()).w_s_ptr());
        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            (*(*wp).w_s_ptr()).b_syn_slow = false;
            wp = (*wp).w_next;
        }
        redraw_later(*curwin.as_ptr(), UPD_CLEAR);
    }
}

fn nv_ctrlo(cap: &mut cmdarg_T) {
    if g_st().Visual.active && g_st().Visual.select {
            g_st().Visual.select = false;
            may_trigger_modechanged();
            showmode();
            g_st().Visual.restart_select = 2;
        } else {
            cap.count1 = -cap.count1;
            nv_pcmark(cap);
        }
}

fn nv_hat(cap: &mut cmdarg_T) {
    unsafe {
        if !checkclearopq(&mut *cap.oap) {
            buflist_getfile(cap.count0, 0, GETF_SETMARK | GETF_ALT, 0);
        }
    }
}

fn nv_Zet(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearopq(&mut *cap.oap) {
            return;
        }

        match cap.nchar {
            c if c == b'Z' as c_int => {
                do_cmdline_cmd(c"x".as_ptr());
            }
            c if c == b'Q' as c_int => {
                do_cmdline_cmd(c"q!".as_ptr());
            }
            c if c == b'R' as c_int => {
                if cap.count0 >= 1 && cap.count0 <= 8 {
                    do_cmdline_cmd(c"restart!".as_ptr());
                } else if cap.count0 == 9 {
                    do_cmdline_cmd(c"restart! +qall!".as_ptr());
                } else {
                    do_cmdline_cmd(c"restart".as_ptr());
                }
            }
            _ => {
                clearopbeep(&mut *cap.oap);
            }
        }
    }
}

pub fn do_nv_ident(c1: c_int, c2: c_int) {
    let mut oa = oparg_T::new();
        let mut ca = cmdarg_T::new();

        ca.oap = &raw mut oa;
        ca.cmdchar = c1;
        ca.nchar = c2;
        nv_ident(&mut ca);
}

fn nv_K_getcmd(
    cap: &mut cmdarg_T,
    kp: *mut c_char,
    kp_help: bool,
    kp_ex: bool,
    ptr_arg: *mut *mut c_char,
    mut n: usize,
    buf: *mut c_char,
    bufsize: usize,
    buflen: &mut usize,
) -> usize {
    unsafe {
        if kp_help {
            { let s = c"help! ".as_ptr(); std::ptr::copy_nonoverlapping(s, buf, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            *buflen = 6;
            return n;
        }

        if kp_ex {
            *buflen = vim_snprintf!(buf, bufsize, c"%s ".as_ptr(), kp) as usize;
            if cap.count0 != 0 {
                *buflen += vim_snprintf!(
                    buf.add(*buflen),
                    bufsize - *buflen,
                    c"%lld ".as_ptr(),
                    cap.count0 as c_longlong,
                ) as usize;
            }
            return n;
        }

        let mut ptr = *ptr_arg;

        while *ptr == b'-' as c_char && n > 0 {
            ptr = ptr.add(1);
            n -= 1;
        }
        if n == 0 {
            emsg(tr(e_noident.as_ptr()));
            *ptr_arg = ptr;
            return 0;
        }

        let isman = crate::memory::cstr_as_opt_bytes(kp) == Some(b"man");
        let isman_s = crate::memory::cstr_as_opt_bytes(kp) == Some(b"man -s");
        if cap.count0 != 0 && !(isman || isman_s) {
            *buflen = vim_snprintf!(
                buf,
                bufsize,
                c".,.+%lld".as_ptr(),
                (cap.count0 - 1) as c_longlong,
            ) as usize;
        }

        do_cmdline_cmd(c"tabnew".as_ptr());
        *buflen +=
            vim_snprintf!(buf.add(*buflen), bufsize - *buflen, c"terminal ".as_ptr()) as usize;
        if cap.count0 == 0 && isman_s {
            *buflen +=
                vim_snprintf!(buf.add(*buflen), bufsize - *buflen, c"man ".as_ptr()) as usize;
        } else {
            *buflen +=
                vim_snprintf!(buf.add(*buflen), bufsize - *buflen, c"%s ".as_ptr(), kp) as usize;
        }
        if cap.count0 != 0 && (isman || isman_s) {
            *buflen += vim_snprintf!(
                buf.add(*buflen),
                bufsize - *buflen,
                c"%lld ".as_ptr(),
                cap.count0 as c_longlong,
            ) as usize;
        }

        *ptr_arg = ptr;
        n
    }
}

fn nv_ident(cap: &mut cmdarg_T) {
    unsafe {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut n: usize = 0;
        let mut cmdchar: c_int;
        let g_cmd: bool;
        let mut tag_cmd = false;

        if cap.cmdchar == b'g' as c_int {
            cmdchar = cap.nchar;
            g_cmd = true;
        } else {
            cmdchar = cap.cmdchar;
            g_cmd = false;
        }

        if cmdchar == POUND {
            cmdchar = b'#' as c_int;
        }

        let mut visual_sel = false;
        if cmdchar == b']' as c_int || cmdchar == Ctrl_RSB || cmdchar == b'K' as c_int {
            if g_st().Visual.active && !get_visual_text(Some(cap), &mut ptr, &mut n) {
                return;
            }
            visual_sel = !ptr.is_null();
            if checkclearopq(&mut *cap.oap) {
                return;
            }
        }

        let mut ident_offset: c_int = 0;
        if ptr.is_null() {
            n = find_ident_under_cursor(
                &raw mut ptr,
                if cmdchar == b'*' as c_int || cmdchar == b'#' as c_int {
                    FIND_IDENT | FIND_STRING
                } else {
                    FIND_IDENT
                },
                Some(&mut ident_offset),
            );
            if n == 0 {
                clearop(&mut *cap.oap);
                return;
            }
        }

        let kp = if *(*g_st().curbuf).b_p_kp == NUL {
            opt_vars().p_kp
        } else {
            (*g_st().curbuf).b_p_kp
        };
        let kp_helpbang = crate::memory::cstr_as_opt_bytes(kp) == Some(b":help!");
        let kp_help = kp_helpbang
            || *kp == NUL
            || (crate::memory::cstr_as_opt_bytes(kp) == Some(b":he"))
            || (crate::memory::cstr_as_opt_bytes(kp) == Some(b":help"));
        if kp_help && !kp_helpbang && *skipwhite(ptr) == NUL {
            emsg(tr(e_noident.as_ptr()));
            return;
        }
        let kp_ex = *kp == b':' as c_char;
        let bufsize = n * 2 + 30 + std::ffi::CStr::from_ptr(kp).to_bytes().len();
        let mut buf_vec = vec![0 as c_char; bufsize];
        let mut buflen: usize = 0;

        match cmdchar {
            c if c == b'*' as c_int || c == b'#' as c_int => {
                setpcmark();
                (**curwin.as_ptr()).w_cursor.col = ptr.offset_from(get_cursor_line_ptr()) as colnr_T;

                if !g_cmd && vim_iswordp(ptr) {
                    { let s = c"\\<".as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    buflen = 2;
                }
                g_st().Search.no_smartcase = true;
            }
            c if c == b'K' as c_int => {
                n = nv_K_getcmd(
                    cap,
                    kp,
                    kp_help,
                    kp_ex,
                    &raw mut ptr,
                    n,
                    buf_vec.as_mut_ptr(),
                    bufsize,
                    &mut buflen,
                );
                if n == 0 {
                    return;
                }
            }
            c if c == b']' as c_int => {
                tag_cmd = true;
                { let s = c"tselect ".as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                buflen = 8;
            }
            _ => {
                tag_cmd = true;
                if (*g_st().curbuf).b_help {
                    { let s = c"help! ".as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    buflen = 6;
                } else if g_cmd {
                    { let s = c"tjump ".as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    buflen = 6;
                } else if cap.count0 == 0 {
                    { let s = c"tag ".as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    buflen = 4;
                } else {
                    buflen = vim_snprintf!(
                        buf_vec.as_mut_ptr(),
                        bufsize,
                        c":%lldtag ".as_ptr(),
                        cap.count0 as c_longlong,
                    ) as usize;
                }
            }
        }

        if cmdchar == b'K' as c_int && kp_helpbang && !visual_sel {
            { let s = c"help!".as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            buflen = 5;
        } else if cmdchar == b'K' as c_int && !kp_help {
            let ptr_cstr = std::ffi::CString::new(std::slice::from_raw_parts(ptr.cast::<u8>(), n))
                .unwrap_or_default();
            let p = if kp_ex {
                vim_strsave_fnameescape(ptr_cstr.as_ptr(), VSE_NONE)
            } else {
                vim_strsave_shellescape(&ptr_cstr, true, true)
            };
            let plen = p.to_bytes().len();
            buf_vec.resize(buflen + plen + 1, 0);
            { let s = p.as_ptr(); std::ptr::copy_nonoverlapping(s, buf_vec.as_mut_ptr().add(buflen), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            buflen += plen;
        } else {
            let aux_ptr = if cmdchar == b'*' as c_int {
                if opt_vars().p_magic != 0 {
                    c"/.*~[^$\\".as_ptr()
                } else {
                    c"/^$\\".as_ptr()
                }
            } else if cmdchar == b'#' as c_int {
                if opt_vars().p_magic != 0 {
                    c"/?.*~[^$\\".as_ptr()
                } else {
                    c"/?^$\\".as_ptr()
                }
            } else if tag_cmd {
                if crate::memory::cstr_as_opt_bytes((*g_st().curbuf).b_p_ft) == Some(b"help") {
                    c"".as_ptr()
                } else {
                    c"\\|\"\n[".as_ptr()
                }
            } else {
                c"\\|\"\n*?[".as_ptr()
            };

            let buf = buf_vec.as_mut_ptr();
            let mut p = buf.add(buflen);
            while n > 0 {
                n -= 1;
                if !vim_strchr(aux_ptr, c_int::from(*ptr as u8)).is_null() {
                    *p = b'\\' as c_char;
                    p = p.add(1);
                }
                let len = (utfc_ptr2len(ptr) - 1) as usize;
                let mut i = 0;
                while i < len && n > 0 {
                    *p = *ptr;
                    p = p.add(1);
                    ptr = ptr.add(1);
                    i += 1;
                    n -= 1;
                }
                *p = *ptr;
                p = p.add(1);
                ptr = ptr.add(1);
            }
            *p = NUL;
            buflen = p.offset_from(buf) as usize;
        }

        let buf = buf_vec.as_mut_ptr();
        if cmdchar == b'*' as c_int || cmdchar == b'#' as c_int {
            if !g_cmd && vim_iswordp(mb_prevptr(get_cursor_line_ptr(), ptr)) {
                { let s = c"\\>".as_ptr(); std::ptr::copy_nonoverlapping(s, buf.add(buflen), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                buflen += 2;
            }

            init_history();
            add_to_history(HIST_SEARCH, buf, buflen, true, NUL as c_int);

            normal_search(
                cap,
                if cmdchar == b'*' as c_int {
                    b'/' as c_int
                } else {
                    b'?' as c_int
                },
                buf,
                buflen,
                0,
                None,
            );
        } else {
            g_st().g_tag_at_cursor = true;
            do_cmdline_cmd(buf);
            g_st().g_tag_at_cursor = false;

            if cmdchar == b'K' as c_int && !kp_ex && !kp_help {
                g_st().restart_edit = b'i' as c_int;
                add_map(
                    c"<esc>".as_ptr().cast_mut(),
                    c"<Cmd>bdelete!<CR>".as_ptr().cast_mut(),
                    MODE_TERMINAL,
                    true,
                );
            }
        }
    }
}

pub fn get_visual_text(cap: Option<&mut cmdarg_T>, pp: &mut *mut c_char, lenp: &mut usize) -> bool {
    unsafe {
        if g_st().Visual.mode != b'V' as c_int {
            unadjust_for_sel();
        }
        if g_st().Visual.start.lnum != (**curwin.as_ptr()).w_cursor.lnum {
            if let Some(cap) = cap {
                clearopbeep(&mut *cap.oap);
            }
            return false;
        }
        if g_st().Visual.mode == b'V' as c_int {
            *pp = get_cursor_line_ptr();
            *lenp = get_cursor_line_len() as usize;
        } else {
            if lt((**curwin.as_ptr()).w_cursor, g_st().Visual.start) {
                *pp = ml_get_pos(&(**curwin.as_ptr()).w_cursor);
                *lenp = (g_st().Visual.start.col - (**curwin.as_ptr()).w_cursor.col + 1) as usize;
            } else {
                *pp = ml_get_pos(&g_st().Visual.start);
                *lenp = ((**curwin.as_ptr()).w_cursor.col - g_st().Visual.start.col + 1) as usize;
            }
            if **pp == NUL {
                *lenp = 0;
            }
            if *lenp > 0 {
                *lenp =
                    (*lenp).wrapping_add_signed((utfc_ptr2len((*pp).add(*lenp - 1)) - 1) as isize);
            }
        }
        reset_VIsual_and_resel();
        true
    }
}

fn nv_tagpop(cap: &mut cmdarg_T) {
    unsafe {
        if !checkclearopq(&mut *cap.oap) {
            do_tag(
                ptr::null_mut(),
                c"".as_ptr().cast_mut(),
                DT_POP,
                cap.count1,
                0,
                true,
            );
        }
    }
}

fn nv_scroll(cap: &mut cmdarg_T) {
    unsafe {
        let mut n: c_int;
        let mut lnum: linenr_T = 0;

        (*cap.oap).motion_type = kMTLineWise;
        setpcmark();

        if cap.cmdchar == b'L' as c_int {
            validate_botline_win(*curwin.as_ptr());
            (**curwin.as_ptr()).w_cursor.lnum = (**curwin.as_ptr()).w_botline - 1;
            if cap.count1 > (**curwin.as_ptr()).w_cursor.lnum {
                (**curwin.as_ptr()).w_cursor.lnum = 1;
            } else if win_lines_concealed(&mut **curwin.as_ptr()) {
                n = cap.count1 - 1;
                while n > 0 && (**curwin.as_ptr()).w_cursor.lnum > (**curwin.as_ptr()).w_topline {
                    hasFolding(
                        *curwin.as_ptr(),
                        (**curwin.as_ptr()).w_cursor.lnum,
                        &raw mut (**curwin.as_ptr()).w_cursor.lnum,
                        ptr::null_mut(),
                    );
                    n += c_int::from(decor_conceal_line(*curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum, true));
                    if (**curwin.as_ptr()).w_cursor.lnum > (**curwin.as_ptr()).w_topline {
                        (**curwin.as_ptr()).w_cursor.lnum -= 1;
                    }
                    n -= 1;
                }
            } else {
                (**curwin.as_ptr()).w_cursor.lnum -= cap.count1 - 1;
            }
        } else {
            if cap.cmdchar == b'M' as c_int {
                let mut used = -(win_get_fill(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline) - (**curwin.as_ptr()).w_topfill);
                validate_botline_win(*curwin.as_ptr());
                let half = ((**curwin.as_ptr()).w_view_height - (**curwin.as_ptr()).w_empty_rows + 1) / 2;
                n = 0;
                while (**curwin.as_ptr()).w_topline + n < (*g_st().curbuf).b_ml.ml_line_count {
                    if n > 0 && used + win_get_fill(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline + n) / 2 >= half {
                        n -= 1;
                        break;
                    }
                    used += plines_win(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline + n, true);
                    if used >= half {
                        break;
                    }
                    if hasFolding(
                        *curwin.as_ptr(),
                        (**curwin.as_ptr()).w_topline + n,
                        ptr::null_mut(),
                        &raw mut lnum,
                    ) {
                        n = lnum - (**curwin.as_ptr()).w_topline;
                    }
                    n += 1;
                }
                if n > 0 && used > (**curwin.as_ptr()).w_view_height {
                    n -= 1;
                }
            } else {
                n = cap.count1 - 1;
                if win_lines_concealed(&mut **curwin.as_ptr()) {
                    lnum = (**curwin.as_ptr()).w_topline;
                    while (decor_conceal_line(*curwin.as_ptr(), lnum - 1, true) || {
                        let prev = n;
                        n -= 1;
                        prev > 0
                    }) && lnum < (**curwin.as_ptr()).w_botline - 1
                    {
                        hasFolding(*curwin.as_ptr(), lnum, ptr::null_mut(), &raw mut lnum);
                        lnum += 1;
                    }
                    n = lnum - (**curwin.as_ptr()).w_topline;
                }
            }
            (**curwin.as_ptr()).w_cursor.lnum = ((**curwin.as_ptr()).w_topline + n).min((*g_st().curbuf).b_ml.ml_line_count);
        }

        if (*cap.oap).op_type == OP_NOP {
            cursor_correct(&mut **curwin.as_ptr());
        }
        beginline(BL_SOL | BL_FIX);
    }
}

fn nv_right(cap: &mut cmdarg_T) {
    unsafe {
        if (g_st().mod_mask & (c_int::from(MOD_MASK_SHIFT) | MOD_MASK_CTRL)) != 0 {
            if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
                cap.arg = 1;
            }
            nv_wordcmd(cap);
            return;
        }

        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        let mut past_line = g_st().Visual.active && *opt_vars().p_sel != b'o' as c_char;

        if virtual_active(*curwin.as_ptr()) {
            past_line = false;
        }

        let mut n = cap.count1;
        while n > 0 {
            if (!past_line && oneright() == FAIL) || (past_line && *get_cursor_pos_ptr() == NUL) {
                if ((cap.cmdchar == b' ' as c_int && !vim_strchr(opt_vars().p_ww, b's' as c_int).is_null())
                    || (cap.cmdchar == b'l' as c_int
                        && !vim_strchr(opt_vars().p_ww, b'l' as c_int).is_null())
                    || (cap.cmdchar == K_RIGHT && !vim_strchr(opt_vars().p_ww, b'>' as c_int).is_null()))
                    && (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_ml.ml_line_count
                {
                    if (*cap.oap).op_type != OP_NOP
                        && !(*cap.oap).inclusive
                        && !lineempty((**curwin.as_ptr()).w_cursor.lnum)
                    {
                        (*cap.oap).inclusive = true;
                    } else {
                        (**curwin.as_ptr()).w_cursor.lnum += 1;
                        (**curwin.as_ptr()).w_cursor.col = 0;
                        (**curwin.as_ptr()).w_cursor.coladd = 0;
                        (**curwin.as_ptr()).w_set_curswant = true;
                        (*cap.oap).inclusive = false;
                    }
                    n -= 1;
                    continue;
                }
                if (*cap.oap).op_type == OP_NOP {
                    if n == cap.count1 {
                        beep_flush();
                    }
                } else if !lineempty((**curwin.as_ptr()).w_cursor.lnum) {
                    (*cap.oap).inclusive = true;
                }
                break;
            } else if past_line {
                (**curwin.as_ptr()).w_set_curswant = true;
                if virtual_active(*curwin.as_ptr()) {
                    oneright();
                } else {
                    (**curwin.as_ptr()).w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                }
            }
            n -= 1;
        }
        if n != cap.count1
            && (opt_vars().fdo_flags & kOptFdoFlagHor) != 0
            && g_st().KeyTyped
            && (*cap.oap).op_type == OP_NOP
        {
            foldOpenCursor();
        }
    }
}

fn nv_left(cap: &mut cmdarg_T) {
    unsafe {
        if (g_st().mod_mask & (c_int::from(MOD_MASK_SHIFT) | MOD_MASK_CTRL)) != 0 {
            if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
                cap.arg = 1;
            }
            nv_bck_word(cap);
            return;
        }

        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        let mut n = cap.count1;
        while n > 0 {
            if oneleft() == FAIL {
                if (((cap.cmdchar == K_BS || cap.cmdchar == Ctrl_H)
                    && !vim_strchr(opt_vars().p_ww, b'b' as c_int).is_null())
                    || (cap.cmdchar == b'h' as c_int
                        && !vim_strchr(opt_vars().p_ww, b'h' as c_int).is_null())
                    || (cap.cmdchar == K_LEFT && !vim_strchr(opt_vars().p_ww, b'<' as c_int).is_null()))
                    && (**curwin.as_ptr()).w_cursor.lnum > 1
                {
                    (**curwin.as_ptr()).w_cursor.lnum -= 1;
                    coladvance(&mut **curwin.as_ptr(), MAXCOL);
                    (**curwin.as_ptr()).w_set_curswant = true;

                    if ((*cap.oap).op_type == OP_DELETE || (*cap.oap).op_type == OP_CHANGE)
                        && !lineempty((**curwin.as_ptr()).w_cursor.lnum)
                    {
                        let cp = get_cursor_pos_ptr();
                        if *cp != NUL {
                            (**curwin.as_ptr()).w_cursor.col += utfc_ptr2len(cp);
                        }
                        cap.retval |= CA_NO_ADJ_OP_END;
                    }
                    n -= 1;
                    continue;
                } else if (*cap.oap).op_type == OP_NOP && n == cap.count1 {
                    beep_flush();
                }
                break;
            }
            n -= 1;
        }
        if n != cap.count1
            && (opt_vars().fdo_flags & kOptFdoFlagHor) != 0
            && g_st().KeyTyped
            && (*cap.oap).op_type == OP_NOP
        {
            foldOpenCursor();
        }
    }
}

fn nv_up(cap: &mut cmdarg_T) {
    unsafe {
        if (g_st().mod_mask & c_int::from(MOD_MASK_SHIFT)) != 0 {
            cap.arg = BACKWARD;
            nv_page(cap);
            return;
        }

        (*cap.oap).motion_type = kMTLineWise;
        if cursor_up(cap.count1, (*cap.oap).op_type == OP_NOP) == 0 {
            clearopbeep(&mut *cap.oap);
        } else if cap.arg != 0 {
            beginline(BL_WHITE | BL_FIX);
        }
    }
}

fn nv_down(cap: &mut cmdarg_T) {
    unsafe {
        if (g_st().mod_mask & c_int::from(MOD_MASK_SHIFT)) != 0 {
            cap.arg = FORWARD;
            nv_page(cap);
        } else if bt_cmdwin(g_st().curbuf) && cap.cmdchar == CAR {
            cmdwin_do_action(c"confirm".as_ptr());
        } else if bt_quickfix(g_st().curbuf) && cap.cmdchar == CAR {
            qf_view_result(false);
        } else if bt_prompt(g_st().curbuf)
            && cap.cmdchar == CAR
            && (**curwin.as_ptr()).w_cursor.lnum == (*g_st().curbuf).b_ml.ml_line_count
        {
            prompt_invoke_callback();
            if g_st().restart_edit == 0 {
                g_st().restart_edit = b'a' as c_int;
            }
        } else {
            (*cap.oap).motion_type = kMTLineWise;
            if cursor_down(cap.count1, (*cap.oap).op_type == OP_NOP) == 0 {
                clearopbeep(&mut *cap.oap);
            } else if cap.arg != 0 {
                beginline(BL_WHITE | BL_FIX);
            }
        }
    }
}

fn nv_gotofile(cap: &mut cmdarg_T) {
    unsafe {
        let mut lnum: linenr_T = -1;

        if check_text_or_curbuf_locked(cap.oap) {
            return;
        }

        if !check_can_set_curbuf_disabled() {
            return;
        }

        if let Some(fname_cstr) = grab_file_name(cap.count1, &mut lnum) {
            let ptr = fname_cstr.as_ptr().cast_mut();
            if curbufIsChanged() && (*g_st().curbuf).b_nwindows <= 1 && !buf_hide(&*g_st().curbuf) {
                autowrite(&mut *g_st().curbuf, false);
            }
            setpcmark();
            if do_ecmd(
                0,
                ptr,
                ptr::null_mut(),
                ptr::null_mut(),
                ECMD_LAST,
                if buf_hide(&*g_st().curbuf) { ECMD_HIDE } else { 0 },
                *curwin.as_ptr(),
            ) == OK
                && cap.nchar == b'F' as c_int
                && lnum >= 0
            {
                (**curwin.as_ptr()).w_cursor.lnum = lnum;
                check_cursor_lnum(&mut **curwin.as_ptr());
                beginline(BL_SOL | BL_FIX);
            }
        } else {
            clearop(&mut *cap.oap);
        }
    }
}

fn nv_end(cap: &mut cmdarg_T) {
    if cap.arg != 0 || (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            cap.arg = 1;
            nv_goto(cap);
            cap.count1 = 1;
        }
        nv_dollar(cap);
}

fn nv_dollar(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = true;
        if !virtual_active(*curwin.as_ptr())
            || gchar_cursor() != NUL as c_int
            || (*cap.oap).op_type == OP_NOP
        {
            (**curwin.as_ptr()).w_curswant = MAXCOL;
        }
        if cursor_down(cap.count1 - 1, (*cap.oap).op_type == OP_NOP) == 0 {
            clearopbeep(&mut *cap.oap);
        } else if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
    }
}

fn nv_search(cap: &mut cmdarg_T) {
    unsafe {
        let oap = cap.oap;
        let save_cursor = (**curwin.as_ptr()).w_cursor;

        if cap.cmdchar == b'?' as c_int && (*cap.oap).op_type == OP_ROT13 {
            cap.cmdchar = b'g' as c_int;
            cap.nchar = b'?' as c_int;
            nv_operator(cap);
            return;
        }

        cap.searchbuf = getcmdline(cap.cmdchar, cap.count1, 0, true);

        let Some(ref sbuf) = cap.searchbuf else {
            clearop(&mut *oap);
            return;
        };
        let sbuf_ptr = sbuf.as_ptr().cast_mut();
        let sbuf_len = sbuf.to_bytes().len();

        normal_search(
            cap,
            cap.cmdchar,
            sbuf_ptr,
            sbuf_len,
            if cap.arg != 0 || !equalpos(save_cursor, (**curwin.as_ptr()).w_cursor) {
                0
            } else {
                SEARCH_MARK
            },
            None,
        );
    }
}

fn nv_next(cap: &mut cmdarg_T) {
    unsafe {
        let old = (**curwin.as_ptr()).w_cursor;
        let mut wrapped: c_int = 0;
        let i = normal_search(
            cap,
            0,
            ptr::null_mut(),
            0,
            SEARCH_MARK | cap.arg,
            Some(&mut wrapped),
        );

        if i == 1 && wrapped == 0 && equalpos(old, (**curwin.as_ptr()).w_cursor) {
            cap.count1 += 1;
            normal_search(
                cap,
                0,
                ptr::null_mut(),
                0,
                SEARCH_MARK | cap.arg,
                None,
            );
            cap.count1 -= 1;
        }

        if i > 0
            && opt_vars().p_hls != 0
            && !g_st().Search.no_hlsearch
            && win_hl_attr(*curwin.as_ptr(), HLF_LC as c_int) != win_hl_attr(*curwin.as_ptr(), HLF_L as c_int)
        {
            redraw_later(*curwin.as_ptr(), UPD_SOME_VALID);
        }
    }
}

fn normal_search(
    cap: &mut cmdarg_T,
    dir: c_int,
    pat: *mut c_char,
    patlen: usize,
    opt: c_int,
    wrapped: Option<&mut c_int>,
) -> c_int {
    unsafe {
        let mut sia = searchit_arg_T::new();
        let prev_cursor = (**curwin.as_ptr()).w_cursor;

        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        (*cap.oap).use_reg_one = true;
        (**curwin.as_ptr()).w_set_curswant = true;

        let i = do_search(
            cap.oap,
            dir,
            dir,
            pat,
            patlen,
            cap.count1,
            opt | SEARCH_OPT | SEARCH_ECHO | SEARCH_MSG,
            opt_vars().p_magic != 0,
            &raw mut sia,
        );
        if let Some(wrapped) = wrapped {
            *wrapped = sia.sa_wrapped;
        }
        if i == 0 {
            clearop(&mut *cap.oap);
        } else {
            if i == 2 {
                (*cap.oap).motion_type = kMTLineWise;
            }
            (**curwin.as_ptr()).w_cursor.coladd = 0;
            if (*cap.oap).op_type == OP_NOP && (opt_vars().fdo_flags & kOptFdoFlagSearch) != 0 && g_st().KeyTyped {
                foldOpenCursor();
            }
        }
        if !equalpos((**curwin.as_ptr()).w_cursor, prev_cursor)
            && opt_vars().p_hls != 0
            && !g_st().Search.no_hlsearch
            && win_hl_attr(*curwin.as_ptr(), HLF_LC as c_int) != win_hl_attr(*curwin.as_ptr(), HLF_L as c_int)
        {
            redraw_later(*curwin.as_ptr(), UPD_SOME_VALID);
        }

        check_cursor(*curwin.as_ptr());
        i
    }
}

fn nv_csearch(cap: &mut cmdarg_T) {
    unsafe {
        let mut cursor_dec = false;

        if *opt_vars().p_sel == b'e' as c_char
            && g_st().Visual.active
            && g_st().Visual.mode == b'v' as c_int
            && g_st().Visual.select_exclu_adj
        {
            unadjust_for_sel();
            cursor_dec = true;
        }

        let t_cmd = cap.cmdchar == b't' as c_int || cap.cmdchar == b'T' as c_int;

        (*cap.oap).motion_type = kMTCharWise;
        if is_special(cap.nchar) || searchc(cap, t_cmd) == 0 {
            clearopbeep(&mut *cap.oap);
            if cursor_dec {
                adjust_for_sel(cap);
            }
            return;
        }

        (**curwin.as_ptr()).w_set_curswant = true;
        if gchar_cursor() == TAB
            && virtual_active(*curwin.as_ptr())
            && cap.arg == FORWARD
            && (t_cmd || (*cap.oap).op_type != OP_NOP)
        {
            let mut scol: colnr_T = 0;
            let mut ecol: colnr_T = 0;
            getvcol(
                &mut **curwin.as_ptr(),
                &mut (**curwin.as_ptr()).w_cursor,
                &raw mut scol,
                ptr::null_mut(),
                &raw mut ecol,
                0,
            );
            (**curwin.as_ptr()).w_cursor.coladd = ecol - scol;
        } else {
            (**curwin.as_ptr()).w_cursor.coladd = 0;
        }
        adjust_for_sel(cap);
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
    }
}

fn nv_bracket_block(cap: &mut cmdarg_T, old_pos: &pos_T) {
    unsafe {
        let mut new_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut pos: *mut pos_T = ptr::null_mut();
        let mut prev_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut n: c_int;
        let findc: c_int;

        if cap.nchar == b'*' as c_int {
            cap.nchar = b'/' as c_int;
        }
        if cap.nchar == b'm' as c_int || cap.nchar == b'M' as c_int {
            if cap.cmdchar == b'[' as c_int {
                findc = b'{' as c_int;
            } else {
                findc = b'}' as c_int;
            }
            n = 9999;
        } else {
            findc = cap.nchar;
            n = cap.count1;
        }
        while n > 0 {
            pos = findmatchlimit(
                cap.oap,
                findc,
                if cap.cmdchar == b'[' as c_int {
                    FM_BACKWARD
                } else {
                    FM_FORWARD
                },
                0,
            );
            if pos.is_null() {
                if new_pos.lnum == 0 {
                    if cap.nchar != b'm' as c_int && cap.nchar != b'M' as c_int {
                        clearopbeep(&mut *cap.oap);
                    }
                } else {
                    pos = &raw mut new_pos;
                }
                break;
            }
            prev_pos = new_pos;
            (**curwin.as_ptr()).w_cursor = *pos;
            new_pos = *pos;
            n -= 1;
        }
        (**curwin.as_ptr()).w_cursor = *old_pos;

        if cap.nchar == b'm' as c_int || cap.nchar == b'M' as c_int {
            let norm = (findc == b'{' as c_int) == (cap.nchar == b'm' as c_int);

            n = cap.count1;
            if prev_pos.lnum != 0 {
                pos = &raw mut prev_pos;
                (**curwin.as_ptr()).w_cursor = prev_pos;
                if norm {
                    n -= 1;
                }
            } else {
                pos = ptr::null_mut();
            }
            while n > 0 {
                loop {
                    let step_res = if findc == b'{' as c_int {
                        dec_cursor()
                    } else {
                        inc_cursor()
                    };
                    if step_res < 0 {
                        if pos.is_null() {
                            clearopbeep(&mut *cap.oap);
                        }
                        n = 0;
                        break;
                    }
                    let c = gchar_cursor();
                    if c == b'{' as c_int || c == b'}' as c_int {
                        if (c == findc && norm) || (n == 1 && !norm) {
                            new_pos = (**curwin.as_ptr()).w_cursor;
                            pos = &raw mut new_pos;
                            n = 0;
                        } else if new_pos.lnum == 0 {
                            new_pos = (**curwin.as_ptr()).w_cursor;
                            pos = &raw mut new_pos;
                        } else {
                            pos = findmatchlimit(
                                cap.oap,
                                findc,
                                if cap.cmdchar == b'[' as c_int {
                                    FM_BACKWARD
                                } else {
                                    FM_FORWARD
                                },
                                0,
                            );
                            if pos.is_null() {
                                n = 0;
                            } else {
                                (**curwin.as_ptr()).w_cursor = *pos;
                            }
                        }
                        break;
                    }
                }
                n -= 1;
            }
            (**curwin.as_ptr()).w_cursor = *old_pos;
            if pos.is_null() && new_pos.lnum != 0 {
                clearopbeep(&mut *cap.oap);
            }
        }
        if !pos.is_null() {
            setpcmark();
            (**curwin.as_ptr()).w_cursor = *pos;
            (**curwin.as_ptr()).w_set_curswant = true;
            if (opt_vars().fdo_flags & kOptFdoFlagBlock) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
                foldOpenCursor();
            }
        }
    }
}

fn nv_brackets(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        let old_pos = (**curwin.as_ptr()).w_cursor;
        (**curwin.as_ptr()).w_cursor.coladd = 0;

        if cap.nchar == b'f' as c_int {
            nv_gotofile(cap);
        } else if !vim_strchr(c"iI\t\x04dD".as_ptr(), cap.nchar).is_null() {
            let mut ptr: *mut c_char = ptr::null_mut();
            let len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
            if len == 0 {
                clearop(&mut *cap.oap);
            } else {
                let mut ptr_vec = Vec::with_capacity(len + 1);
                ptr_vec.extend_from_slice(std::slice::from_raw_parts(ptr, len));
                ptr_vec.push(NUL);
                let is_upper = (cap.nchar as u8).is_ascii_uppercase();
                let is_lower = (cap.nchar as u8).is_ascii_lowercase();
                find_pattern_in_path(
                    ptr_vec.as_mut_ptr(),
                    0,
                    len,
                    true,
                    if cap.count0 == 0 { !is_upper } else { false },
                    if (cap.nchar & 0xf) == (b'd' as c_int & 0xf) {
                        FIND_DEFINE
                    } else {
                        FIND_ANY
                    },
                    cap.count1,
                    if is_upper {
                        ACTION_SHOW_ALL
                    } else if is_lower {
                        ACTION_SHOW
                    } else {
                        ACTION_GOTO
                    },
                    if cap.cmdchar == b']' as c_int {
                        (**curwin.as_ptr()).w_cursor.lnum + 1
                    } else {
                        1
                    },
                    MAXLNUM,
                    false,
                    false,
                );
                (**curwin.as_ptr()).w_set_curswant = true;
            }
        } else if (cap.cmdchar == b'[' as c_int
            && !vim_strchr(c"{(*/#mM".as_ptr(), cap.nchar).is_null())
            || (cap.cmdchar == b']' as c_int
                && !vim_strchr(c"})*/#mM".as_ptr(), cap.nchar).is_null())
        {
            nv_bracket_block(cap, &old_pos);
        } else if cap.nchar == b'[' as c_int || cap.nchar == b']' as c_int {
            let flag = if cap.nchar == cap.cmdchar {
                b'{' as c_int
            } else {
                b'}' as c_int
            };
            (**curwin.as_ptr()).w_set_curswant = true;
            if !findpar(
                &raw mut (*cap.oap).inclusive,
                cap.arg,
                cap.count1,
                flag,
                (*cap.oap).op_type != OP_NOP && cap.arg == FORWARD && flag == b'{' as c_int,
            ) {
                clearopbeep(&mut *cap.oap);
            } else {
                if (*cap.oap).op_type == OP_NOP {
                    beginline(BL_WHITE | BL_FIX);
                }
                if (opt_vars().fdo_flags & kOptFdoFlagBlock) != 0
                    && g_st().KeyTyped
                    && (*cap.oap).op_type == OP_NOP
                {
                    foldOpenCursor();
                }
            }
        } else if cap.nchar == b'p' as c_int || cap.nchar == b'P' as c_int {
            nv_put_opt(cap, true);
        } else if cap.nchar == b'\'' as c_int || cap.nchar == b'`' as c_int {
            let mut fm = pos_to_mark(g_st().curbuf, ptr::null_mut(), (**curwin.as_ptr()).w_cursor);
            assert!(!fm.is_null());
            let mut prev_fm = fm;
            let mut n = cap.count1;
            while n > 0 {
                prev_fm = fm;
                fm = getnextmark(
                    &mut (*fm).mark,
                    if cap.cmdchar == b'[' as c_int {
                        BACKWARD
                    } else {
                        FORWARD
                    },
                    c_int::from(cap.nchar == b'\'' as c_int),
                );
                if fm.is_null() {
                    break;
                }
                n -= 1;
            }
            if fm.is_null() {
                fm = prev_fm;
            }
            let mut flags = kMarkContext;
            if cap.nchar == b'\'' as c_int {
                flags |= kMarkBeginLine;
            }
            nv_mark_move_to(cap, flags, &mut *fm);
        } else if cap.nchar >= K_RIGHTRELEASE && cap.nchar <= K_LEFTMOUSE {
            do_mouse(
                cap.oap,
                cap.nchar,
                if cap.cmdchar == b']' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count1,
                PUT_FIXINDENT != 0,
            );
        } else if cap.nchar == b'z' as c_int {
            if foldMoveTo(
                false,
                if cap.cmdchar == b']' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count1,
            ) == 0
            {
                clearopbeep(&mut *cap.oap);
            }
        } else if cap.nchar == b'c' as c_int {
            if diff_move_to(
                if cap.cmdchar == b']' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count1,
            ) == 0
            {
                clearopbeep(&mut *cap.oap);
            }
        } else if cap.nchar == b'C' as c_int {
            if (*cap.oap).op_type != OP_NOP && mc_buf_has_cursors(&mut *g_st().curbuf) {
                clearopbeep(&mut *cap.oap);
            } else {
                let mut tv_args: [typval_T; 3] = [
                    typval_T {
                        v_type: VarType::VAR_BOOL,
                        v_lock: 0,
                        vval: TypvalVval::Bool(if cap.cmdchar == b']' as c_int {
                            kBoolVarTrue
                        } else {
                            kBoolVarFalse
                        }),
                    },
                    typval_T {
                        v_type: VarType::VAR_NUMBER,
                        v_lock: 0,
                        vval: TypvalVval::Number(varnumber_T::from(cap.count1)),
                    },
                    typval_T {
                        v_type: VarType::VAR_UNKNOWN,
                        v_lock: 0,
                        vval: TypvalVval::Number(0),
                    },
                ];
                let mut rettv = typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                };
                nlua_call_typval(
                    c"vim._core.mcursor".as_ptr(),
                    c"jump".as_ptr(),
                    tv_args.as_mut_ptr(),
                    &raw mut rettv,
                );
                if rettv.v_type != VarType::VAR_BOOL || rettv.vval.v_bool() != kBoolVarTrue {
                    clearopbeep(&mut *cap.oap);
                }
                tv_clear(&raw mut rettv);
            }
        } else if cap.nchar == b'r' as c_int
            || cap.nchar == b's' as c_int
            || cap.nchar == b'S' as c_int
        {
            setpcmark();
            for _ in 0..cap.count1 {
                if spell_move_to(
                    &mut **curwin.as_ptr(),
                    if cap.cmdchar == b']' as c_int {
                        FORWARD
                    } else {
                        BACKWARD
                    },
                    if cap.nchar == b's' as c_int {
                        SMT_ALL
                    } else if cap.nchar == b'r' as c_int {
                        SMT_RARE
                    } else {
                        SMT_BAD
                    },
                    false,
                    ptr::null_mut(),
                ) == 0
                {
                    clearopbeep(&mut *cap.oap);
                    break;
                }
                (**curwin.as_ptr()).w_set_curswant = true;
            }
            if (*cap.oap).op_type == OP_NOP && (opt_vars().fdo_flags & kOptFdoFlagSearch) != 0 && g_st().KeyTyped {
                foldOpenCursor();
            }
        } else {
            clearopbeep(&mut *cap.oap);
        }
    }
}

fn buf_has_cstyle_comments() -> bool {
    unsafe {
        let mut part_buf: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
        let mut list = (*g_st().curbuf).b_p_com;
        while *list != NUL {
            copy_option_part(
                &raw mut list,
                part_buf.as_mut_ptr(),
                COM_MAX_LEN,
                c",".as_ptr(),
            );
            let string = vim_strchr(part_buf.as_ptr(), b':' as c_int);
            if !string.is_null()
                && *string.add(1) == b'/' as c_char
                && (*string.add(2) == b'/' as c_char || *string.add(2) == b'*' as c_char)
            {
                return true;
            }
        }
        false
    }
}

fn nv_percent(cap: &mut cmdarg_T) {
    unsafe {
        let lnum = (**curwin.as_ptr()).w_cursor.lnum;

        (*cap.oap).inclusive = true;
        if cap.count0 != 0 {
            if cap.count0 > 100 {
                clearopbeep(&mut *cap.oap);
            } else {
                (*cap.oap).motion_type = kMTLineWise;
                setpcmark();
                if (*g_st().curbuf).b_ml.ml_line_count >= 21_474_836 {
                    (**curwin.as_ptr()).w_cursor.lnum =
                        ((*g_st().curbuf).b_ml.ml_line_count + 99) / 100 * cap.count0;
                } else {
                    (**curwin.as_ptr()).w_cursor.lnum =
                        ((*g_st().curbuf).b_ml.ml_line_count * cap.count0 + 99) / 100;
                }
                (**curwin.as_ptr()).w_cursor.lnum = (**curwin.as_ptr())
                    .w_cursor
                    .lnum
                    .max(1)
                    .min((*g_st().curbuf).b_ml.ml_line_count);
                beginline(BL_SOL | BL_FIX);
            }
        } else {
            let mut flags = 0;
            if vim_strchr(opt_vars().p_cpo, kCpoMatch).is_null() && buf_has_cstyle_comments() {
                let comment_col = check_linecomment(get_cursor_line_ptr());
                if comment_col == MAXCOL || (**curwin.as_ptr()).w_cursor.col < comment_col {
                    flags = FM_SKIPCOMM;
                }
            }

            (*cap.oap).motion_type = kMTCharWise;
            (*cap.oap).use_reg_one = true;
            let pos = findmatchlimit(cap.oap, NUL as c_int, flags, 0);
            if pos.is_null() {
                clearopbeep(&mut *cap.oap);
            } else {
                setpcmark();
                (**curwin.as_ptr()).w_cursor = *pos;
                (**curwin.as_ptr()).w_set_curswant = true;
                (**curwin.as_ptr()).w_cursor.coladd = 0;
                adjust_for_sel(cap);
            }
        }
        if (*cap.oap).op_type == OP_NOP
            && lnum != (**curwin.as_ptr()).w_cursor.lnum
            && (opt_vars().fdo_flags & kOptFdoFlagPercent) != 0
            && g_st().KeyTyped
        {
            foldOpenCursor();
        }
    }
}

fn nv_brace(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).use_reg_one = true;
        (*cap.oap).inclusive = false;
        (**curwin.as_ptr()).w_set_curswant = true;

        if findsent(cap.arg, cap.count1) == FAIL {
            clearopbeep(&mut *cap.oap);
            return;
        }

        adjust_cursor(&mut *cap.oap);
        (**curwin.as_ptr()).w_cursor.coladd = 0;
        if (opt_vars().fdo_flags & kOptFdoFlagBlock) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
    }
}

fn nv_mark(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearop(&mut *cap.oap) {
            return;
        }

        if setmark(cap.nchar) == 0 {
            clearopbeep(&mut *cap.oap);
        }
    }
}

fn nv_findpar(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        (*cap.oap).use_reg_one = true;
        (**curwin.as_ptr()).w_set_curswant = true;
        if !findpar(
            &raw mut (*cap.oap).inclusive,
            cap.arg,
            cap.count1,
            NUL as c_int,
            false,
        ) {
            clearopbeep(&mut *cap.oap);
            return;
        }

        (**curwin.as_ptr()).w_cursor.coladd = 0;
        if (opt_vars().fdo_flags & kOptFdoFlagBlock) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
    }
}

fn nv_undo(cap: &mut cmdarg_T) {
    unsafe {
        if (*cap.oap).op_type == OP_LOWER || g_st().Visual.active {
            cap.cmdchar = b'g' as c_int;
            cap.nchar = b'u' as c_int;
            nv_operator(cap);
        } else {
            nv_kundo(cap);
        }
    }
}

fn nv_kundo(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearopq(&mut *cap.oap) {
            return;
        }

        u_undo(cap.count1);
        (**curwin.as_ptr()).w_set_curswant = true;
    }
}

fn nv_replace(cap: &mut cmdarg_T) {
    unsafe {
        let mut had_ctrl_v: c_int;

        if checkclearop(&mut *cap.oap) {
            return;
        }
        if bt_prompt(g_st().curbuf) && !prompt_curpos_editable() {
            clearopbeep(&mut *cap.oap);
            return;
        }

        if cap.nchar == Ctrl_V || cap.nchar == Ctrl_Q {
            had_ctrl_v = Ctrl_V;
            cap.nchar = get_literal(false);
            if cap.nchar > DEL {
                had_ctrl_v = NUL as c_int;
            }
        } else {
            had_ctrl_v = NUL as c_int;
        }

        if is_special(cap.nchar) {
            clearopbeep(&mut *cap.oap);
            return;
        }

        if g_st().Visual.active {
            if g_st().got_int {
                g_st().got_int = false;
            }
            if had_ctrl_v != 0 {
                if cap.nchar == CAR {
                    cap.nchar = REPLACE_CR_NCHAR;
                } else if cap.nchar == NL {
                    cap.nchar = REPLACE_NL_NCHAR;
                }
            }
            nv_operator(cap);
            return;
        }

        if virtual_active(*curwin.as_ptr()) {
            if u_save_cursor() == 0 {
                return;
            }
            if gchar_cursor() == NUL as c_int {
                coladvance_force(getviscol() + cap.count1);
                (**curwin.as_ptr()).w_cursor.col -= cap.count1;
            } else if gchar_cursor() == TAB {
                coladvance_force(getviscol());
            }
        }

        if (get_cursor_pos_len() as usize) < (cap.count1 as c_uint as usize)
            || mb_charlen(get_cursor_pos_ptr()) < cap.count1
        {
            clearopbeep(&mut *cap.oap);
            return;
        }

        if had_ctrl_v != Ctrl_V
            && cap.nchar == b'\t' as c_int
            && ((*g_st().curbuf).b_p_et != 0 || opt_vars().p_sta != 0)
        {
            stuffnumReadbuff(cap.count1);
            stuffcharReadbuff(b'R' as c_int);
            stuffcharReadbuff(b'\t' as c_int);
            stuffcharReadbuff(ESC);
            exec_stuffed(Some(cap));
            return;
        }

        if u_save_cursor() == 0 {
            return;
        }

        if had_ctrl_v != Ctrl_V
            && (cap.nchar == b'\r' as c_int || cap.nchar == b'\n' as c_int)
        {
            del_chars(cap.count1, 0);
            stuffcharReadbuff(b'\r' as c_int);
            stuffcharReadbuff(ESC);

            invoke_edit(cap, 1, b'r' as c_int, 0);
        } else {
            let mut spec = CmdSpec::new();
            spec.regname = (*cap.oap).regname;
            spec.count = c_long::from(cap.count1);
            spec.cmd = b'r' as c_int;
            spec.cmdarg = cap.nchar;
            prep_redo(true, true, spec);
            if had_ctrl_v != NUL as c_int {
                redo_append_char(had_ctrl_v);
            }

            (*g_st().curbuf).b_op_start = (**curwin.as_ptr()).w_cursor;
            let old_State = g_st().State;

            if cap.nchar_len > 0 {
                redo_append_str(cap.nchar_composing.as_ptr(), -1);
            } else {
                redo_append_char(cap.nchar);
            }

            let mut n = cap.count1;
            while n > 0 {
                g_st().State = MODE_REPLACE;
                if cap.nchar == Ctrl_E || cap.nchar == Ctrl_Y {
                    let c = ins_copychar(
                        (**curwin.as_ptr()).w_cursor.lnum + if cap.nchar == Ctrl_Y { -1 } else { 1 },
                    );
                    if c != NUL as c_int {
                        ins_char(c);
                    } else {
                        (**curwin.as_ptr()).w_cursor.col += 1;
                    }
                } else if cap.nchar_len != 0 {
                    ins_char_bytes(
                        cap.nchar_composing.as_mut_ptr(),
                        cap.nchar_len as usize,
                    );
                } else {
                    ins_char(cap.nchar);
                }
                g_st().State = old_State;
                n -= 1;
            }
            (**curwin.as_ptr()).w_cursor.col -= 1;
            mb_adjust_cursor();
            (*g_st().curbuf).b_op_end = (**curwin.as_ptr()).w_cursor;
            (**curwin.as_ptr()).w_set_curswant = true;
            set_last_insert(cap.nchar);
        }

        foldUpdateAfterInsert();
    }
}

fn v_swap_corners(cmdchar: c_int) {
    unsafe {
        if cmdchar == b'O' as c_int && g_st().Visual.mode == Ctrl_V {
            let mut left: colnr_T = 0;
            let mut right: colnr_T = 0;
            let mut old_cursor = (**curwin.as_ptr()).w_cursor;
            getvcols(
                *curwin.as_ptr(),
                &mut old_cursor,
                &mut g_st().Visual.start,
                &mut left,
                &mut right,
                0,
            );
            (**curwin.as_ptr()).w_cursor.lnum = g_st().Visual.start.lnum;
            coladvance(&mut **curwin.as_ptr(), left);
            g_st().Visual.start = (**curwin.as_ptr()).w_cursor;

            (**curwin.as_ptr()).w_cursor.lnum = old_cursor.lnum;
            (**curwin.as_ptr()).w_curswant = right;
            if old_cursor.lnum >= g_st().Visual.start.lnum && *opt_vars().p_sel == b'e' as c_char {
                (**curwin.as_ptr()).w_curswant += 1;
            }
            coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
            if (**curwin.as_ptr()).w_cursor.col == old_cursor.col
                && (!virtual_active(*curwin.as_ptr()) || (**curwin.as_ptr()).w_cursor.coladd == old_cursor.coladd)
            {
                (**curwin.as_ptr()).w_cursor.lnum = g_st().Visual.start.lnum;
                if old_cursor.lnum <= g_st().Visual.start.lnum && *opt_vars().p_sel == b'e' as c_char {
                    right += 1;
                }
                coladvance(&mut **curwin.as_ptr(), right);
                g_st().Visual.start = (**curwin.as_ptr()).w_cursor;

                (**curwin.as_ptr()).w_cursor.lnum = old_cursor.lnum;
                coladvance(&mut **curwin.as_ptr(), left);
                (**curwin.as_ptr()).w_curswant = left;
            }
        } else {
            ptr::swap(&raw mut (**curwin.as_ptr()).w_cursor, &raw mut g_st().Visual.start);
            (**curwin.as_ptr()).w_set_curswant = true;
        }
    }
}

fn nv_Replace(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().Visual.active {
            cap.cmdchar = b'c' as c_int;
            cap.nchar = NUL as c_int;
            norm_st().VIsual_mode_orig = g_st().Visual.mode;
            g_st().Visual.mode = b'V' as c_int;
            nv_operator(cap);
            return;
        }

        if checkclearopq(&mut *cap.oap) {
            return;
        }

        if !modifiable(&*g_st().curbuf) {
            emsg(tr(e_modifiable.as_ptr()));
        } else {
            if virtual_active(*curwin.as_ptr()) {
                coladvance(&mut **curwin.as_ptr(), getviscol());
            }
            invoke_edit(
                cap,
                0,
                if cap.arg != 0 {
                    b'V' as c_int
                } else {
                    b'R' as c_int
                },
                0,
            );
        }
    }
}

fn nv_vreplace(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().Visual.active {
            cap.cmdchar = b'r' as c_int;
            cap.nchar = cap.extra_char;
            nv_replace(cap);
            return;
        }

        if checkclearopq(&mut *cap.oap) {
            return;
        }

        if !modifiable(&*g_st().curbuf) {
            emsg(tr(e_modifiable.as_ptr()));
        } else {
            if cap.extra_char == Ctrl_V || cap.extra_char == Ctrl_Q {
                cap.extra_char = get_literal(false);
            }
            if cap.extra_char < b' ' as c_int {
                stuffcharReadbuff(Ctrl_V);
            }
            stuffcharReadbuff(cap.extra_char);
            stuffcharReadbuff(ESC);
            if virtual_active(*curwin.as_ptr()) {
                coladvance(&mut **curwin.as_ptr(), getviscol());
            }
            invoke_edit(cap, 1, b'v' as c_int, 0);
        }
    }
}

fn n_swapchar(cap: &mut cmdarg_T) {
    unsafe {
        let mut did_change = false;

        if checkclearopq(&mut *cap.oap) {
            return;
        }

        if lineempty((**curwin.as_ptr()).w_cursor.lnum) && vim_strchr(opt_vars().p_ww, b'~' as c_int).is_null() {
            clearopbeep(&mut *cap.oap);
            return;
        }

        prep_redo_cmd(cap);

        if u_save_cursor() == 0 {
            return;
        }

        let startpos = (**curwin.as_ptr()).w_cursor;
        let mut n = cap.count1;
        while n > 0 {
            did_change |= swapchar((*cap.oap).op_type, &mut (**curwin.as_ptr()).w_cursor);
            inc_cursor();
            if gchar_cursor() == NUL as c_int {
                if !vim_strchr(opt_vars().p_ww, b'~' as c_int).is_null()
                    && (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_ml.ml_line_count
                {
                    (**curwin.as_ptr()).w_cursor.lnum += 1;
                    (**curwin.as_ptr()).w_cursor.col = 0;
                    if n > 1 {
                        if u_savesub((**curwin.as_ptr()).w_cursor.lnum) == 0 {
                            break;
                        }
                        u_clearline(&mut *g_st().curbuf);
                    }
                } else {
                    break;
                }
            }
            n -= 1;
        }

        check_cursor(*curwin.as_ptr());
        (**curwin.as_ptr()).w_set_curswant = true;
        if did_change {
            changed_lines(
                g_st().curbuf,
                startpos.lnum,
                startpos.col,
                (**curwin.as_ptr()).w_cursor.lnum + 1,
                0,
                true,
            );
            (*g_st().curbuf).b_op_start = startpos;
            (*g_st().curbuf).b_op_end = (**curwin.as_ptr()).w_cursor;
            if (*g_st().curbuf).b_op_end.col > 0 {
                (*g_st().curbuf).b_op_end.col -= 1;
            }
        }
    }
}

fn nv_mark_move_to(cap: &mut cmdarg_T, flags: MarkMove, fm: &mut fmark_T) -> MarkMoveRes {
    unsafe {
        let res = mark_move_to(&raw mut *fm, flags);
        if (res & kMarkMoveFailed) != 0 {
            clearop(&mut *cap.oap);
        }
        (*cap.oap).motion_type = if (flags & kMarkBeginLine) != 0 {
            kMTLineWise
        } else {
            kMTCharWise
        };
        if cap.cmdchar == b'`' as c_int {
            (*cap.oap).use_reg_one = true;
        }
        (*cap.oap).inclusive = false;
        (**curwin.as_ptr()).w_set_curswant = true;
        res
    }
}

fn v_visop(cap: &mut cmdarg_T) {
    unsafe {
        let trans = c"YyDdCcxdXdAAIIrr".as_ptr();

        if (cap.cmdchar as u8).is_ascii_uppercase() {
            if g_st().Visual.mode != Ctrl_V {
                norm_st().VIsual_mode_orig = g_st().Visual.mode;
                g_st().Visual.mode = b'V' as c_int;
            } else if cap.cmdchar == b'C' as c_int || cap.cmdchar == b'D' as c_int {
                (**curwin.as_ptr()).w_curswant = MAXCOL;
            }
        }
        cap.cmdchar = c_int::from(*vim_strchr(trans, cap.cmdchar).add(1) as u8);
        nv_operator(cap);
    }
}

fn nv_subst(cap: &mut cmdarg_T) {
    unsafe {
        if bt_prompt(g_st().curbuf) && !prompt_curpos_editable() {
            clearopbeep(&mut *cap.oap);
            return;
        }
        if g_st().Visual.active {
            if cap.cmdchar == b'S' as c_int {
                norm_st().VIsual_mode_orig = g_st().Visual.mode;
                g_st().Visual.mode = b'V' as c_int;
            }
            cap.cmdchar = b'c' as c_int;
            nv_operator(cap);
        } else {
            nv_optrans(cap);
        }
    }
}

fn nv_abbrev(cap: &mut cmdarg_T) {
    if cap.cmdchar == K_DEL || cap.cmdchar == K_KDEL {
            cap.cmdchar = b'x' as c_int;
        }
        if g_st().Visual.active {
            v_visop(cap);
        } else {
            nv_optrans(cap);
        }
}

fn nv_optrans(cap: &mut cmdarg_T) {
    unsafe {
        const AR: [&CStr; 8] = [c"dl", c"dh", c"d$", c"c$", c"cl", c"cc", c"yy", c":s\r"];
        let str_p = c"xXDCsSY&".as_ptr();

        if !checkclearopq(&mut *cap.oap) {
            if cap.count0 != 0 {
                stuffnumReadbuff(cap.count0);
            }
            let idx = CStr::from_ptr(str_p).to_bytes().iter().position(|&b| b == cap.cmdchar as u8).unwrap();
            stuffReadbuff(AR[idx].as_ptr());
            exec_stuffed(Some(cap));
        }
        cap.opcount = 0;
    }
}

fn nv_gomark(cap: &mut cmdarg_T) {
    unsafe {
        let mut flags: MarkMove = if (opt_vars().jop_flags & kOptJopFlagView) != 0 {
            kMarkSetView
        } else {
            0
        };
        if (*cap.oap).op_type != OP_NOP {
            flags = 0;
        }
        let old_KeyTyped = g_st().KeyTyped;

        let name = if cap.cmdchar == b'g' as c_int {
            flags |= KMarkNoContext;
            cap.extra_char
        } else {
            flags |= kMarkContext;
            cap.nchar
        };
        if cap.arg != 0 {
            flags |= kMarkBeginLine;
        }
        if cap.count0 != 0 {
            flags |= kMarkSetView;
        }

        let fm = mark_get(&mut *g_st().curbuf, *curwin.as_ptr(), ptr::null_mut(), kMarkAll, name);
        let move_res = nv_mark_move_to(cap, flags, &mut *fm);

        if !virtual_active(*curwin.as_ptr()) {
            (**curwin.as_ptr()).w_cursor.coladd = 0;
        }

        if (*cap.oap).op_type == OP_NOP
            && (move_res & kMarkMoveSuccess) != 0
            && ((move_res & kMarkSwitchedBuf) != 0 || (move_res & kMarkChangedCursor) != 0)
            && (opt_vars().fdo_flags & kOptFdoFlagMark) != 0
            && old_KeyTyped
        {
            foldOpenCursor();
        }
    }
}

fn nv_pcmark(cap: &mut cmdarg_T) {
    unsafe {
        let mut flags: MarkMove = if (opt_vars().jop_flags & kOptJopFlagView) != 0 {
            kMarkSetView
        } else {
            0
        };
        let mut move_res: MarkMoveRes = 0;
        let old_KeyTyped = g_st().KeyTyped;

        if checkclearopq(&mut *cap.oap) {
            return;
        }

        if cap.cmdchar == TAB && g_st().mod_mask == MOD_MASK_CTRL {
            if !goto_tabpage_lastused() {
                clearopbeep(&mut *cap.oap);
            }
            return;
        }

        let fm = if cap.cmdchar == b'g' as c_int {
            get_changelist(&mut *g_st().curbuf, &mut **curwin.as_ptr(), cap.count1)
        } else {
            flags |= KMarkNoContext | kMarkJumpList;
            get_jumplist(&mut **curwin.as_ptr(), cap.count1)
        };

        if !fm.is_null() {
            move_res = nv_mark_move_to(cap, flags, &mut *fm);
        } else if cap.cmdchar == b'g' as c_int {
            if (*g_st().curbuf).b_changelistlen == 0 {
                emsg(tr(e_changelist_is_empty.as_ptr()));
            } else if cap.count1 < 0 {
                emsg(tr(c"E662: At start of changelist".as_ptr()));
            } else {
                emsg(tr(c"E663: At end of changelist".as_ptr()));
            }
        } else {
            clearopbeep(&mut *cap.oap);
        }

        if (*cap.oap).op_type == OP_NOP
            && ((move_res & kMarkSwitchedBuf) != 0 || (move_res & kMarkChangedLine) != 0)
            && (opt_vars().fdo_flags & kOptFdoFlagMark) != 0
            && old_KeyTyped
        {
            foldOpenCursor();
        }
    }
}

fn nv_regname(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearop(&mut *cap.oap) {
            return;
        }
        if cap.nchar == b'=' as c_int {
            cap.nchar = get_expr_register();
        }
        if cap.nchar != NUL as c_int && valid_yank_reg(cap.nchar, false) {
            (*cap.oap).regname = cap.nchar;
            cap.opcount = cap.count0;
            set_reg_var((*cap.oap).regname);
        } else {
            clearopbeep(&mut *cap.oap);
        }
    }
}

fn nv_visual(cap: &mut cmdarg_T) {
    unsafe {
        if cap.cmdchar == Ctrl_Q {
            cap.cmdchar = Ctrl_V;
        }

        if (*cap.oap).op_type != OP_NOP {
            (*cap.oap).motion_force = cap.cmdchar;
            g_st().motion_force = cap.cmdchar;
            g_st().finish_op = false;
            return;
        }

        g_st().Visual.select = cap.arg != 0;
        if g_st().Visual.active {
            if g_st().Visual.mode == cap.cmdchar {
                end_visual_mode();
            } else {
                g_st().Visual.mode = cap.cmdchar;
                showmode();
                may_trigger_modechanged();
            }
            redraw_curbuf_later(UPD_INVERTED);
        } else if cap.count0 > 0 && g_st().Visual.resel.mode != NUL as c_int {
            g_st().Visual.start = (**curwin.as_ptr()).w_cursor;

            g_st().Visual.active = true;
            g_st().Visual.reselect = 1;
            if cap.arg == 0 {
                may_start_select(b'c' as c_int);
            }
            setmouse();
            if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
                g_st().redraw_cmdline = true;
            }
            if g_st().Visual.resel.mode != b'v' as c_int || g_st().Visual.resel.line_count > 1 {
                (**curwin.as_ptr()).w_cursor.lnum += g_st().Visual.resel.line_count * cap.count0 - 1;
                check_cursor(*curwin.as_ptr());
            }
            g_st().Visual.mode = g_st().Visual.resel.mode;
            if g_st().Visual.mode == b'v' as c_int {
                if g_st().Visual.resel.line_count <= 1 {
                    update_curswant_force();
                    (**curwin.as_ptr()).w_curswant += g_st().Visual.resel.vcol * cap.count0;
                    if *opt_vars().p_sel != b'e' as c_char {
                        (**curwin.as_ptr()).w_curswant -= 1;
                    }
                } else {
                    (**curwin.as_ptr()).w_curswant = g_st().Visual.resel.vcol;
                }
                coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
            }
            if g_st().Visual.resel.vcol == MAXCOL {
                (**curwin.as_ptr()).w_curswant = MAXCOL;
                coladvance(&mut **curwin.as_ptr(), MAXCOL);
            } else if g_st().Visual.mode == Ctrl_V {
                let tmp_cursor = (**curwin.as_ptr()).w_cursor;
                (**curwin.as_ptr()).w_cursor = g_st().Visual.start;
                update_curswant_force();
                (**curwin.as_ptr()).w_curswant += g_st().Visual.resel.vcol * cap.count0 - 1;
                (**curwin.as_ptr()).w_cursor = tmp_cursor;
                if *opt_vars().p_sel == b'e' as c_char {
                    (**curwin.as_ptr()).w_curswant += 1;
                }
                coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
            } else {
                (**curwin.as_ptr()).w_set_curswant = true;
            }
            redraw_curbuf_later(UPD_INVERTED);
        } else {
            if cap.arg == 0 {
                may_start_select(b'c' as c_int);
            }
            n_start_visual_mode(cap.cmdchar);
            if g_st().Visual.mode != b'V' as c_int && *opt_vars().p_sel == b'e' as c_char {
                cap.count1 += 1;
            } else {
                g_st().Visual.select_exclu_adj = false;
            }
            if cap.count0 > 0 {
                cap.count1 -= 1;
                if cap.count1 > 0 {
                    if g_st().Visual.mode == b'v' as c_int || g_st().Visual.mode == Ctrl_V {
                        nv_right(cap);
                    } else if g_st().Visual.mode == b'V' as c_int {
                        nv_down(cap);
                    }
                }
            }
        }
    }
}

pub fn start_selection() {
    may_start_select(b'k' as c_int);
        n_start_visual_mode(b'v' as c_int);
}

pub fn may_start_select(c: c_int) {
    g_st().Visual.select = (c == b'o' as c_int || (stuff_empty() && typebuf_typed() != 0))
            && !vim_strchr(opt_vars().p_slm, c).is_null();
}

fn n_start_visual_mode(c: c_int) {
    unsafe {
        g_st().Visual.mode = c;
        g_st().Visual.active = true;
        g_st().Visual.reselect = 1;
        if c == Ctrl_V && (get_ve_flags(*curwin.as_ptr()) & kOptVeFlagBlock) != 0 && gchar_cursor() == TAB {
            validate_virtcol(*curwin.as_ptr());
            coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_virtcol);
        }
        g_st().Visual.start = (**curwin.as_ptr()).w_cursor;

        foldAdjustVisual();

        may_trigger_modechanged();
        setmouse();
        conceal_check_cursor_line();

        if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
            g_st().redraw_cmdline = true;
        }
        if (**curwin.as_ptr()).w_redr_type < UPD_INVERTED {
            (**curwin.as_ptr()).w_old_cursor_lnum = (**curwin.as_ptr()).w_cursor.lnum;
            (**curwin.as_ptr()).w_old_visual_lnum = (**curwin.as_ptr()).w_cursor.lnum;
        }
        redraw_curbuf_later(UPD_VALID);
    }
}

fn nv_window(cap: &mut cmdarg_T) {
    unsafe {
        if cap.nchar == b':' as c_int {
            cap.cmdchar = b':' as c_int;
            cap.nchar = NUL as c_int;
            nv_colon(cap);
        } else if !checkclearop(&mut *cap.oap) {
            do_window(cap.nchar, cap.count0, NUL as c_int);
        }
    }
}

fn nv_suspend(cap: &mut cmdarg_T) {
    unsafe {
        clearop(&mut *cap.oap);
        if g_st().Visual.active {
            end_visual_mode();
        }
        do_cmdline_cmd(c"stop".as_ptr());
    }
}

fn nv_gv_cmd(cap: &mut cmdarg_T) {
    unsafe {
        if (*g_st().curbuf).b_visual.vi_start.lnum == 0
            || (*g_st().curbuf).b_visual.vi_start.lnum > (*g_st().curbuf).b_ml.ml_line_count
            || (*g_st().curbuf).b_visual.vi_end.lnum == 0
        {
            beep_flush();
            return;
        }

        let prev = (*g_st().curbuf).b_visual;
        if g_st().Visual.active {
            (*g_st().curbuf).b_visual = visualinfo();
            (*g_st().curbuf).b_visual_mode_eval = g_st().Visual.mode;
        }
        g_st().Visual.mode = prev.vi_mode;
        (**curwin.as_ptr()).w_curswant = prev.vi_curswant;
        let tpos = prev.vi_end;
        (**curwin.as_ptr()).w_cursor = prev.vi_start;

        g_st().Visual.active = true;
        g_st().Visual.reselect = 1;

        check_cursor(*curwin.as_ptr());
        g_st().Visual.start = (**curwin.as_ptr()).w_cursor;
        (**curwin.as_ptr()).w_cursor = tpos;
        check_cursor(*curwin.as_ptr());
        update_topline(*curwin.as_ptr());

        if cap.arg != 0 {
            g_st().Visual.select = true;
            g_st().Visual.select_reg = 0;
        } else {
            may_start_select(b'c' as c_int);
        }
        setmouse();
        redraw_curbuf_later(UPD_INVERTED);
        showmode();
    }
}

pub fn nv_g_home_m_cmd(cap: &mut cmdarg_T) {
    unsafe {
        let mut i: c_int;
        let flag = cap.nchar == b'^' as c_int;

        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 && (**curwin.as_ptr()).w_view_width != 0 {
            let width1 = (**curwin.as_ptr()).w_view_width - win_col_off(*curwin.as_ptr());
            let width2 = width1 + win_col_off2(*curwin.as_ptr());

            validate_virtcol(*curwin.as_ptr());
            i = 0;
            if (**curwin.as_ptr()).w_virtcol >= width1 && width2 > 0 {
                i = ((**curwin.as_ptr()).w_virtcol - width1) / width2 * width2 + width1;
            }

            if (**curwin.as_ptr()).w_skipcol > 0 && (**curwin.as_ptr()).w_cursor.lnum == (**curwin.as_ptr()).w_topline {
                let overlap =
                    crate::r#move::sms_marker_overlap(*curwin.as_ptr(), (**curwin.as_ptr()).w_view_width - width2);
                if overlap > 0 && i == (**curwin.as_ptr()).w_skipcol {
                    i += overlap;
                }
            }
        } else {
            i = (**curwin.as_ptr()).w_leftcol;
        }
        if cap.nchar == b'm' as c_int {
            i += ((**curwin.as_ptr()).w_view_width - win_col_off(*curwin.as_ptr())
                + if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 && i > 0 {
                    win_col_off2(*curwin.as_ptr())
                } else {
                    0
                })
                / 2;
        }
        coladvance(&mut **curwin.as_ptr(), i);
        if flag {
            loop {
                i = gchar_cursor();
                if !(ascii_iswhite(i) && oneright() == OK) {
                    break;
                }
            }
            (**curwin.as_ptr()).w_valid &= !VALID_WCOL;
        }
        (**curwin.as_ptr()).w_set_curswant = true;
        if hasAnyFolding(&mut **curwin.as_ptr()) != 0 {
            validate_cheight(*curwin.as_ptr());
            if (**curwin.as_ptr()).w_cline_folded {
                update_curswant_force();
            }
        }
        adjust_skipcol();
    }
}

fn nv_g_underscore_cmd(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = true;
        (**curwin.as_ptr()).w_curswant = MAXCOL;
        if cursor_down(cap.count1 - 1, (*cap.oap).op_type == OP_NOP) == 0 {
            clearopbeep(&mut *cap.oap);
            return;
        }

        let ptr = get_cursor_line_ptr();

        if (**curwin.as_ptr()).w_cursor.col > 0 && *ptr.add((**curwin.as_ptr()).w_cursor.col as usize) == NUL {
            (**curwin.as_ptr()).w_cursor.col -= 1;
        }

        while (**curwin.as_ptr()).w_cursor.col > 0
            && ascii_iswhite(c_int::from(*ptr.add((**curwin.as_ptr()).w_cursor.col as usize) as u8))
        {
            (**curwin.as_ptr()).w_cursor.col -= 1;
        }
        (**curwin.as_ptr()).w_set_curswant = true;
        adjust_for_sel(cap);
    }
}

fn nv_g_dollar_cmd(cap: &mut cmdarg_T) {
    unsafe {
        let oap = cap.oap;
        let mut i: c_int;
        let col_off = win_col_off(*curwin.as_ptr());
        let flag = cap.nchar == K_END || cap.nchar == K_KEND;

        (*oap).motion_type = kMTCharWise;
        (*oap).inclusive = true;
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 && (**curwin.as_ptr()).w_view_width != 0 {
            (**curwin.as_ptr()).w_curswant = MAXCOL;
            if cap.count1 == 1 {
                let width1 = (**curwin.as_ptr()).w_view_width - col_off;
                let width2 = width1 + win_col_off2(*curwin.as_ptr());

                validate_virtcol(*curwin.as_ptr());
                i = width1 - 1;
                if (**curwin.as_ptr()).w_virtcol >= width1 {
                    i += (((**curwin.as_ptr()).w_virtcol - width1) / width2 + 1) * width2;
                }
                coladvance(&mut **curwin.as_ptr(), i);

                update_curswant_force();
                if (**curwin.as_ptr()).w_cursor.col > 0
                    && (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0
                    && (**curwin.as_ptr()).w_virtcol > i
                {
                    (**curwin.as_ptr()).w_cursor.col -= 1;
                }
            } else if !nv_screengo(&mut *oap, FORWARD, cap.count1 - 1, false) {
                clearopbeep(&mut *oap);
            }
        } else {
            if cap.count1 > 1 {
                cursor_down(cap.count1 - 1, false);
            }
            i = (**curwin.as_ptr()).w_leftcol + (**curwin.as_ptr()).w_view_width - col_off - 1;
            coladvance(&mut **curwin.as_ptr(), i);

            if (**curwin.as_ptr()).w_cursor.col > 0 && utf_ptr2cells(get_cursor_pos_ptr()) > 1 {
                let mut vcol: colnr_T = 0;
                getvvcol(
                    *curwin.as_ptr(),
                    &mut (**curwin.as_ptr()).w_cursor,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &raw mut vcol,
                    0,
                );
                if vcol >= (**curwin.as_ptr()).w_leftcol + (**curwin.as_ptr()).w_view_width - col_off {
                    (**curwin.as_ptr()).w_cursor.col -= 1;
                }
            }

            update_curswant_force();
        }
        if flag {
            loop {
                i = gchar_cursor();
                if !(ascii_iswhite_or_nul(i) && oneleft() == OK) {
                    break;
                }
            }
            (**curwin.as_ptr()).w_valid &= !VALID_WCOL;
        }
    }
}

fn nv_gi_cmd(cap: &mut cmdarg_T) {
    unsafe {
        if (*g_st().curbuf).b_last_insert.mark.lnum != 0 {
            (**curwin.as_ptr()).w_cursor = (*g_st().curbuf).b_last_insert.mark;
            check_cursor_lnum(&mut **curwin.as_ptr());
            let i = get_cursor_line_len();
            if (**curwin.as_ptr()).w_cursor.col > i {
                if virtual_active(*curwin.as_ptr()) {
                    (**curwin.as_ptr()).w_cursor.coladd += (**curwin.as_ptr()).w_cursor.col - i;
                }
                (**curwin.as_ptr()).w_cursor.col = i;
            }
        }
        cap.cmdchar = b'i' as c_int;
        nv_edit(cap);
    }
}

fn nv_g_cmd(cap: &mut cmdarg_T) {
    unsafe {
        let oap = cap.oap;

        match cap.nchar {
            Ctrl_A | Ctrl_X => {
                if g_st().Visual.active {
                    cap.arg = 1;
                    cap.cmdchar = cap.nchar;
                    cap.nchar = NUL as c_int;
                    nv_addsub(cap);
                } else if cap.nchar == Ctrl_A
                    && (*cap.oap).op_type == OP_NOP
                    && mc_buf_has_cursors(&mut *g_st().curbuf)
                {
                    mc_counter(c_long::from(cap.count1));
                } else {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'R' as c_int => {
                cap.arg = 1;
                nv_Replace(cap);
            }
            c if c == b'r' as c_int => {
                nv_vreplace(cap);
            }
            c if c == b'&' as c_int => {
                do_cmdline_cmd(c"%s//~/&".as_ptr());
            }
            c if c == b'v' as c_int => {
                nv_gv_cmd(cap);
            }
            c if c == b'V' as c_int => {
                g_st().Visual.reselect = 0;
            }
            c if c == b'Q' as c_int => {
                if g_st().reg_recording != 0 || g_st().reg_executing != 0 {
                    vim_beep(0);
                } else {
                    let mut tv_args: [typval_T; 1] = [typval_T {
                        v_type: VarType::VAR_UNKNOWN,
                        v_lock: 0,
                        vval: TypvalVval::Number(0),
                    }];
                    nlua_call_typval(
                        c"vim._core.mcursor".as_ptr(),
                        c"restore".as_ptr(),
                        tv_args.as_mut_ptr(),
                        ptr::null_mut(),
                    );
                }
            }
            K_BS => {
                cap.nchar = Ctrl_H;
                cap.cmdchar = cap.nchar + (b'v' as c_int - b'h' as c_int);
                cap.arg = 1;
                nv_visual(cap);
            }
            c if c == b'h' as c_int || c == b'H' as c_int || c == Ctrl_H => {
                cap.cmdchar = cap.nchar + (b'v' as c_int - b'h' as c_int);
                cap.arg = 1;
                nv_visual(cap);
            }
            c if c == b'N' as c_int || c == b'n' as c_int => {
                if current_search(cap.count1, cap.nchar == b'n' as c_int) == 0 {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'j' as c_int || c == K_DOWN => {
                let ok = if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
                    (*oap).motion_type = kMTLineWise;
                    cursor_down(cap.count1, (*oap).op_type == OP_NOP) != 0
                } else {
                    nv_screengo(&mut *oap, FORWARD, cap.count1, false)
                };
                if !ok {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'k' as c_int || c == K_UP => {
                let ok = if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
                    (*oap).motion_type = kMTLineWise;
                    cursor_up(cap.count1, (*oap).op_type == OP_NOP) != 0
                } else {
                    nv_screengo(&mut *oap, BACKWARD, cap.count1, false)
                };
                if !ok {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'J' as c_int => {
                nv_join(cap);
            }
            c if c == b'^' as c_int
                || c == b'0' as c_int
                || c == b'm' as c_int
                || c == K_HOME
                || c == K_KHOME =>
            {
                nv_g_home_m_cmd(cap);
            }
            c if c == b'M' as c_int => {
                (*oap).motion_type = kMTCharWise;
                (*oap).inclusive = false;
                let i = linetabsize(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
                if cap.count0 > 0 && cap.count0 <= 100 {
                    coladvance(&mut **curwin.as_ptr(), i * cap.count0 / 100);
                } else {
                    coladvance(&mut **curwin.as_ptr(), i / 2);
                }
                (**curwin.as_ptr()).w_set_curswant = true;
            }
            c if c == b'_' as c_int => {
                nv_g_underscore_cmd(cap);
            }
            c if c == b'$' as c_int || c == K_END || c == K_KEND => {
                nv_g_dollar_cmd(cap);
            }
            c if c == b'*' as c_int
                || c == b'#' as c_int
                || c == POUND
                || c == Ctrl_RSB
                || c == b']' as c_int =>
            {
                nv_ident(cap);
            }
            c if c == b'e' as c_int || c == b'E' as c_int => {
                (*oap).motion_type = kMTCharWise;
                (**curwin.as_ptr()).w_set_curswant = true;
                (*oap).inclusive = true;
                if bckend_word(cap.count1, cap.nchar == b'E' as c_int, false) == 0 {
                    clearopbeep(&mut *oap);
                }
            }
            Ctrl_G => {
                cursor_pos_info(ptr::null_mut());
            }
            c if c == b'i' as c_int => {
                nv_gi_cmd(cap);
            }
            c if c == b'I' as c_int => {
                beginline(0);
                if !checkclearopq(&mut *oap) {
                    invoke_edit(cap, 0, b'g' as c_int, 0);
                }
            }
            c if c == b'f' as c_int || c == b'F' as c_int => {
                nv_gotofile(cap);
            }
            c if c == b'\'' as c_int => {
                cap.arg = 1;
                nv_gomark(cap);
            }
            c if c == b'`' as c_int => {
                nv_gomark(cap);
            }
            c if c == b's' as c_int => {
                do_sleep(i64::from(cap.count1) * 1000, false);
            }
            c if c == b'a' as c_int => {
                do_ascii(&mut crate::ex_docmd::exarg_T::new());
            }
            c if c == b'8' as c_int => {
                if cap.count0 == 8 {
                    utf_find_illegal();
                } else {
                    show_utf8();
                }
            }
            c if c == b'<' as c_int => {
                show_sb_text();
            }
            c if c == b'g' as c_int => {
                cap.arg = 0;
                nv_goto(cap);
            }
            c if c == b'q' as c_int || c == b'w' as c_int => {
                (*oap).cursor_start = (**curwin.as_ptr()).w_cursor;
                nv_operator(cap);
            }
            c if c == b'~' as c_int
                || c == b'u' as c_int
                || c == b'U' as c_int
                || c == b'?' as c_int
                || c == b'@' as c_int =>
            {
                nv_operator(cap);
            }
            c if c == b'd' as c_int || c == b'D' as c_int => {
                nv_gd(&mut *oap, cap.nchar, cap.count0);
            }
            K_LEFTMOUSE => {
                if do_mouse(oap, cap.nchar, BACKWARD, cap.count1, false) {
                    stuffcharReadbuff(Ctrl_RSB);
                    exec_stuffed(Some(cap));
                }
            }
            K_MIDDLEMOUSE | K_MIDDLEDRAG | K_MIDDLERELEASE | K_LEFTDRAG | K_LEFTRELEASE
            | K_MOUSEMOVE | K_RIGHTMOUSE | K_RIGHTDRAG | K_RIGHTRELEASE | K_X1MOUSE | K_X1DRAG
            | K_X1RELEASE | K_X2MOUSE | K_X2DRAG | K_X2RELEASE => {
                g_st().mod_mask = MOD_MASK_CTRL;
                do_mouse(oap, cap.nchar, BACKWARD, cap.count1, false);
            }
            K_IGNORE => {}
            c if c == b'p' as c_int || c == b'P' as c_int => {
                nv_put(cap);
            }
            c if c == b'o' as c_int => {
                (*oap).inclusive = false;
                goto_byte(cap.count0);
            }
            c if c == b',' as c_int => {
                nv_pcmark(cap);
            }
            c if c == b';' as c_int => {
                cap.count1 = -cap.count1;
                nv_pcmark(cap);
            }
            c if c == b't' as c_int => {
                if !checkclearop(&mut *oap) {
                    goto_tabpage(cap.count0);
                }
            }
            c if c == b'T' as c_int => {
                if !checkclearop(&mut *oap) {
                    goto_tabpage(-cap.count1);
                }
            }
            TAB => {
                if !checkclearop(&mut *oap) && !goto_tabpage_lastused() {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'+' as c_int || c == b'-' as c_int => {
                if !checkclearopq(&mut *oap) {
                    undo_time(
                        if cap.nchar == b'-' as c_int {
                            -cap.count1
                        } else {
                            cap.count1
                        },
                        false,
                        false,
                        false,
                    );
                }
            }
            _ => {
                clearopbeep(&mut *oap);
            }
        }
    }
}

fn n_opencmd(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearopq(&mut *cap.oap) {
            return;
        }

        if cap.cmdchar == b'O' as c_int {
            hasFolding(
                *curwin.as_ptr(),
                (**curwin.as_ptr()).w_cursor.lnum,
                &raw mut (**curwin.as_ptr()).w_cursor.lnum,
                ptr::null_mut(),
            );
        } else {
            hasFolding(
                *curwin.as_ptr(),
                (**curwin.as_ptr()).w_cursor.lnum,
                ptr::null_mut(),
                &raw mut (**curwin.as_ptr()).w_cursor.lnum,
            );
        }
        (*g_st().curbuf).b_last_changedtick_i = buf_get_changedtick(&*g_st().curbuf);
        if u_save(
            (**curwin.as_ptr()).w_cursor.lnum - c_int::from(cap.cmdchar == b'O' as c_int),
            (**curwin.as_ptr()).w_cursor.lnum + c_int::from(cap.cmdchar == b'o' as c_int),
        ) != 0
            && open_line(
                if cap.cmdchar == b'O' as c_int {
                    BACKWARD
                } else {
                    FORWARD
                },
                if has_format_option(kFoOpenComs) {
                    OPENLINE_DO_COM
                } else {
                    0
                },
                0,
                ptr::null_mut(),
            )
        {
            if win_cursorline_standout(&**curwin.as_ptr()) {
                (**curwin.as_ptr()).w_valid &= !VALID_CROW;
            }
            invoke_edit(cap, 0, cap.cmdchar, 1);
        }
    }
}

fn nv_dot(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearopq(&mut *cap.oap) {
            return;
        }

        if start_redo(cap.count0, g_st().restart_edit != 0 && g_st().Ins.moved == kInsNone) == 0 {
            clearopbeep(&mut *cap.oap);
            return;
        }
        exec_stuffed(Some(cap));
    }
}

fn nv_redo_or_register(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().Visual.select && g_st().Visual.active {
            g_st().no_mapping += 1;
            let mut reg = plain_vgetc();
            langmap_adjust(&mut reg, true);
            g_st().no_mapping -= 1;

            if reg == b'"' as c_int {
                reg = 0;
            }

            g_st().Visual.select_reg = if valid_yank_reg(reg, true) { reg } else { 0 };
            return;
        }

        if checkclearopq(&mut *cap.oap) {
            return;
        }

        u_redo(cap.count1);
        (**curwin.as_ptr()).w_set_curswant = true;
    }
}

fn nv_Undo(cap: &mut cmdarg_T) {
    unsafe {
        if (*cap.oap).op_type == OP_UPPER || g_st().Visual.active {
            cap.cmdchar = b'g' as c_int;
            cap.nchar = b'U' as c_int;
            nv_operator(cap);
            return;
        }

        if checkclearopq(&mut *cap.oap) {
            return;
        }

        u_undoline();
        (**curwin.as_ptr()).w_set_curswant = true;
    }
}

fn nv_tilde(cap: &mut cmdarg_T) {
    unsafe {
        if opt_vars().p_to == 0 && !g_st().Visual.active && (*cap.oap).op_type != OP_TILDE {
            if bt_prompt(g_st().curbuf) && !prompt_curpos_editable() {
                clearopbeep(&mut *cap.oap);
                return;
            }
            n_swapchar(cap);
        } else {
            nv_operator(cap);
        }
    }
}

fn nv_operator(cap: &mut cmdarg_T) {
    unsafe {
        let op_type = get_op_type(cap.cmdchar, cap.nchar);

        if bt_prompt(g_st().curbuf) && op_is_change(op_type) != 0 && !prompt_curpos_editable() {
            clearopbeep(&mut *cap.oap);
            return;
        }

        if op_type == (*cap.oap).op_type {
            nv_lineop(cap);
        } else if !checkclearop(&mut *cap.oap) {
            (*cap.oap).start = (**curwin.as_ptr()).w_cursor;
            (*cap.oap).op_type = op_type;
            set_op_var(op_type);
        }
    }
}

fn set_op_var(optype: c_int) {
    if optype == OP_NOP {
            set_vim_var_string(VV_OP, ptr::null(), 0);
        } else {
            let opchar0 = get_op_char(optype);
            assert!((0..=u8::MAX as c_int).contains(&opchar0));
            let opchar1 = get_extra_op_char(optype);
            assert!((0..=u8::MAX as c_int).contains(&opchar1));
            let opchars: [c_char; 3] = [opchar0 as c_char, opchar1 as c_char, NUL];
            set_vim_var_string(VV_OP, opchars.as_ptr(), 2);
        }
}

fn nv_lineop(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTLineWise;
        if cursor_down(cap.count1 - 1, (*cap.oap).op_type == OP_NOP) == 0 {
            clearopbeep(&mut *cap.oap);
        } else if ((*cap.oap).op_type == OP_DELETE
            && (*cap.oap).motion_force != b'v' as c_int
            && (*cap.oap).motion_force != Ctrl_V)
            || (*cap.oap).op_type == OP_LSHIFT
            || (*cap.oap).op_type == OP_RSHIFT
        {
            beginline(BL_SOL | BL_FIX);
        } else if (*cap.oap).op_type != OP_YANK {
            beginline(BL_WHITE | BL_FIX);
        }
    }
}

fn nv_home(cap: &mut cmdarg_T) {
    if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            nv_goto(cap);
        } else {
            cap.count0 = 1;
            nv_pipe(cap);
        }
        g_st().ins_at_eol = false;
}

fn nv_pipe(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        (**curwin.as_ptr()).w_cursor.col = 0;
        (**curwin.as_ptr()).w_cursor.coladd = 0;
        if cap.count0 > 0 {
            coladvance(&mut **curwin.as_ptr(), cap.count0 - 1);
            (**curwin.as_ptr()).w_curswant = cap.count0 - 1;
        } else {
            (**curwin.as_ptr()).w_curswant = 0;
        }
        (**curwin.as_ptr()).w_set_curswant = false;
        adjust_skipcol();
    }
}

fn nv_bck_word(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        (**curwin.as_ptr()).w_set_curswant = true;
        if bck_word(cap.count1, cap.arg != 0, false) == 0 {
            clearopbeep(&mut *cap.oap);
        } else if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
    }
}

fn nv_wordcmd(cap: &mut cmdarg_T) {
    unsafe {
        let mut word_end = cap.cmdchar == b'e' as c_int || cap.cmdchar == b'E' as c_int;
        let mut flag = false;
        let startpos = (**curwin.as_ptr()).w_cursor;

        (*cap.oap).inclusive = word_end;

        if !word_end && (*cap.oap).op_type == OP_CHANGE {
            let n = gchar_cursor();
            if n != NUL as c_int && !ascii_iswhite(n) {
                if !vim_strchr(opt_vars().p_cpo, kCpoChangew).is_null() {
                    (*cap.oap).inclusive = true;
                    word_end = true;
                }
                flag = true;
            }
        }

        (*cap.oap).motion_type = kMTCharWise;
        (**curwin.as_ptr()).w_set_curswant = true;
        let n = if word_end {
            end_word(cap.count1, cap.arg != 0, flag, false)
        } else {
            fwd_word(
                cap.count1,
                cap.arg != 0,
                (*cap.oap).op_type != OP_NOP,
            )
        };

        if lt(startpos, (**curwin.as_ptr()).w_cursor) {
            adjust_cursor(&mut *cap.oap);
        }

        if n == 0 && (*cap.oap).op_type == OP_NOP {
            clearopbeep(&mut *cap.oap);
        } else {
            adjust_for_sel(cap);
            if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
                foldOpenCursor();
            }
        }
    }
}

fn adjust_cursor(oap: &mut oparg_T) {
    unsafe {
        if (**curwin.as_ptr()).w_cursor.col > 0
            && gchar_cursor() == NUL as c_int
            && (!g_st().Visual.active || *opt_vars().p_sel == b'o' as c_char)
            && !virtual_active(*curwin.as_ptr())
            && (get_ve_flags(*curwin.as_ptr()) & kOptVeFlagOnemore) == 0
        {
            (**curwin.as_ptr()).w_cursor.col -= 1;
            mb_adjust_cursor();
            oap.inclusive = true;
        }
    }
}

fn nv_beginline(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).motion_type = kMTCharWise;
        (*cap.oap).inclusive = false;
        beginline(cap.arg);
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
        g_st().ins_at_eol = false;
    }
}

fn adjust_for_sel(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().Visual.active
            && (*cap.oap).inclusive
            && *opt_vars().p_sel == b'e' as c_char
            && gchar_cursor() != NUL as c_int
            && lt(g_st().Visual.start, (**curwin.as_ptr()).w_cursor)
        {
            inc_cursor();
            (*cap.oap).inclusive = false;
            g_st().Visual.select_exclu_adj = true;
        }
    }
}

pub fn unadjust_for_sel() -> bool {
    unsafe {
        if *opt_vars().p_sel == b'e' as c_char && !equalpos(g_st().Visual.start, (**curwin.as_ptr()).w_cursor) {
            return unadjust_for_sel_inner(if lt(g_st().Visual.start, (**curwin.as_ptr()).w_cursor) {
                &mut (**curwin.as_ptr()).w_cursor
            } else {
                &mut g_st().Visual.start
            });
        }
        false
    }
}

pub fn unadjust_for_sel_inner(pp: &mut pos_T) -> bool {
    unsafe {
        g_st().Visual.select_exclu_adj = false;

        if pp.coladd > 0 {
            pp.coladd -= 1;
        } else if pp.col > 0 {
            pp.col -= 1;
            mark_mb_adjustpos(g_st().curbuf, pp);
            if virtual_active(*curwin.as_ptr()) {
                let mut cs: colnr_T = 0;
                let mut ce: colnr_T = 0;
                getvcol(&mut **curwin.as_ptr(), pp, &raw mut cs, ptr::null_mut(), &raw mut ce, 0);
                pp.coladd = ce - cs;
            }
        } else if pp.lnum > 1 {
            pp.lnum -= 1;
            pp.col = ml_get_len(pp.lnum);
            return true;
        }

        false
    }
}

fn nv_select(cap: &mut cmdarg_T) {
    if g_st().Visual.active {
            g_st().Visual.select = true;
            g_st().Visual.select_reg = 0;
        } else if g_st().Visual.reselect != 0 {
            cap.nchar = b'v' as c_int;
            cap.arg = 1;
            nv_g_cmd(cap);
        }
}

fn nv_goto(cap: &mut cmdarg_T) {
    unsafe {
        let mut lnum = if cap.arg != 0 {
            (*g_st().curbuf).b_ml.ml_line_count
        } else {
            1
        };
        (*cap.oap).motion_type = kMTLineWise;
        setpcmark();

        if cap.count0 != 0 {
            lnum = cap.count0;
        }
        lnum = lnum.max(1).min((*g_st().curbuf).b_ml.ml_line_count);
        (**curwin.as_ptr()).w_cursor.lnum = lnum;
        beginline(BL_SOL | BL_FIX);
        if (opt_vars().fdo_flags & kOptFdoFlagJump) != 0 && g_st().KeyTyped && (*cap.oap).op_type == OP_NOP {
            foldOpenCursor();
        }
    }
}

fn nv_normal(cap: &mut cmdarg_T) {
    unsafe {
        if cap.nchar == Ctrl_N || cap.nchar == Ctrl_G {
            clearop(&mut *cap.oap);
            if g_st().restart_edit != 0 && g_st().mode_displayed {
                g_st().clear_cmdline = true;
            }
            g_st().restart_edit = 0;
            if g_st().Visual.active {
                end_visual_mode();
                redraw_curbuf_later(UPD_INVERTED);
            }
        } else {
            clearopbeep(&mut *cap.oap);
        }
    }
}

fn nv_esc(cap: &mut cmdarg_T) {
    unsafe {
        let no_reason = (*cap.oap).op_type == OP_NOP
            && cap.opcount == 0
            && cap.count0 == 0
            && (*cap.oap).regname == 0;
        let cmdwin_cancel = cap.arg != 0 && bt_cmdwin(g_st().curbuf);

        if cmdwin_cancel {
            g_st().got_int = false;
            cmdwin_do_action(c"cancel".as_ptr());
        } else if cap.arg != 0 {
            if g_st().restart_edit == 0 && !g_st().Visual.active && no_reason {
                if anyBufIsChanged() {
                    msg(
                        tr(
                            c"Type  :qa!  and press <Enter> to abandon all changes and exit Nvim"
                                .as_ptr(),
                        ),
                        0,
                    );
                } else {
                    msg(tr(c"Type  :qa  and press <Enter> to exit Nvim".as_ptr()), 0);
                }
            }

            if g_st().restart_edit != 0 {
                g_st().redraw_mode = true;
            }

            g_st().restart_edit = 0;
        }

        if g_st().Visual.active {
            end_visual_mode();
            check_cursor_col(&mut **curwin.as_ptr());
            (**curwin.as_ptr()).w_set_curswant = true;
            redraw_curbuf_later(UPD_INVERTED);
        } else if no_reason && !cmdwin_cancel {
            vim_beep(kOptBoFlagEsc);
        }
        clearop(&mut *cap.oap);
    }
}

pub fn set_cursor_for_append_to_line() {
    unsafe {
        (**curwin.as_ptr()).w_set_curswant = true;
        if get_ve_flags(*curwin.as_ptr()) == kOptVeFlagAll {
            let save_State = g_st().State;
            g_st().State = MODE_INSERT;
            coladvance(&mut **curwin.as_ptr(), MAXCOL);
            g_st().State = save_State;
        } else {
            (**curwin.as_ptr()).w_cursor.col += std::ffi::CStr::from_ptr(get_cursor_pos_ptr()).to_bytes().len() as colnr_T;
        }
    }
}

fn nv_edit(cap: &mut cmdarg_T) {
    unsafe {
        if cap.cmdchar == K_INS || cap.cmdchar == K_KINS {
            cap.cmdchar = b'i' as c_int;
        }

        if g_st().Visual.active && (cap.cmdchar == b'A' as c_int || cap.cmdchar == b'I' as c_int) {
            v_visop(cap);
        } else if (cap.cmdchar == b'a' as c_int || cap.cmdchar == b'i' as c_int)
            && ((*cap.oap).op_type != OP_NOP || g_st().Visual.active)
        {
            nv_object(cap);
        } else if (*g_st().curbuf).b_p_ma == 0 && (*g_st().curbuf).terminal.is_null() {
            emsg(tr(e_modifiable.as_ptr()));
            clearop(&mut *cap.oap);
        } else if !checkclearopq(&mut *cap.oap) {
            match cap.cmdchar {
                c if c == b'A' as c_int => {
                    set_cursor_for_append_to_line();
                }
                c if c == b'I' as c_int => {
                    beginline(BL_WHITE);
                }
                c if c == b'a' as c_int => {
                    if virtual_active(*curwin.as_ptr())
                        && ((**curwin.as_ptr()).w_cursor.coladd > 0
                            || *get_cursor_pos_ptr() == NUL
                            || *get_cursor_pos_ptr() == b'\t' as c_char)
                    {
                        (**curwin.as_ptr()).w_cursor.coladd += 1;
                    } else if *get_cursor_pos_ptr() != NUL {
                        inc_cursor();
                    }
                }
                _ => {}
            }

            if (**curwin.as_ptr()).w_cursor.coladd != 0 && cap.cmdchar != b'A' as c_int {
                let save_State = g_st().State;
                g_st().State = MODE_INSERT;
                coladvance(&mut **curwin.as_ptr(), getviscol());
                g_st().State = save_State;
            }

            invoke_edit(cap, 0, cap.cmdchar, 0);
        }
    }
}

fn invoke_edit(cap: &mut cmdarg_T, repl: c_int, cmd: c_int, startln: c_int) {
    unsafe {
        let restart_edit_save = if repl != 0 || !stuff_empty() {
            g_st().restart_edit
        } else {
            0
        };

        g_st().restart_edit = 0;

        if cap.cmdchar != b'O' as c_int && cap.cmdchar != b'o' as c_int {
            (*g_st().curbuf).b_last_changedtick_i = buf_get_changedtick(&*g_st().curbuf);
        }
        let mut session: InsSession =
            atom_ins_start(cmd, c_long::from(cap.count1), kVInsNone, false);
        let busy = edit(cmd, startln != 0, cap.count1);
        if busy {
            cap.retval |= CA_COMMAND_BUSY;
        }

        if g_st().restart_edit == 0 {
            g_st().restart_edit = restart_edit_save;
        }

        atom_ins_end(&mut session, busy);
    }
}

fn nv_object(cap: &mut cmdarg_T) {
    unsafe {
        let include = cap.cmdchar != b'i' as c_int;
        let mps_save = (*g_st().curbuf).b_p_mps;
        (*g_st().curbuf).b_p_mps = c"(:),{:},[:],<:>".as_ptr().cast_mut();

        let flag = match cap.nchar {
            c if c == b'w' as c_int => current_word(&mut *cap.oap, cap.count1, include, false) != 0,
            c if c == b'W' as c_int => current_word(&mut *cap.oap, cap.count1, include, true) != 0,
            c if c == b'b' as c_int || c == b'(' as c_int || c == b')' as c_int => {
                current_block(
                    &mut *cap.oap,
                    cap.count1,
                    include,
                    b'(' as c_int,
                    b')' as c_int,
                ) != 0
            }
            c if c == b'B' as c_int || c == b'{' as c_int || c == b'}' as c_int => {
                current_block(
                    &mut *cap.oap,
                    cap.count1,
                    include,
                    b'{' as c_int,
                    b'}' as c_int,
                ) != 0
            }
            c if c == b'[' as c_int || c == b']' as c_int => {
                current_block(
                    &mut *cap.oap,
                    cap.count1,
                    include,
                    b'[' as c_int,
                    b']' as c_int,
                ) != 0
            }
            c if c == b'<' as c_int || c == b'>' as c_int => {
                current_block(
                    &mut *cap.oap,
                    cap.count1,
                    include,
                    b'<' as c_int,
                    b'>' as c_int,
                ) != 0
            }
            c if c == b't' as c_int => {
                cap.retval |= CA_NO_ADJ_OP_END;
                current_tagblock(&mut *cap.oap, cap.count1, include) != 0
            }
            c if c == b'p' as c_int => {
                current_par(&mut *cap.oap, cap.count1, include, b'p' as c_int) != 0
            }
            c if c == b'l' as c_int => current_line(&mut *cap.oap, include) != 0,
            c if c == b's' as c_int => current_sent(&mut *cap.oap, cap.count1, include) != 0,
            c if c == b'"' as c_int || c == b'\'' as c_int || c == b'`' as c_int => {
                current_quote(&mut *cap.oap, cap.count1, include, cap.nchar)
            }
            _ => false,
        };

        (*g_st().curbuf).b_p_mps = mps_save;
        if !flag {
            clearopbeep(&mut *cap.oap);
        }
        adjust_cursor_col();
        (**curwin.as_ptr()).w_set_curswant = true;
    }
}

fn nv_q(cap: &mut cmdarg_T) {
    unsafe {
        if (*cap.oap).op_type == OP_FORMAT {
            cap.cmdchar = b'g' as c_int;
            cap.nchar = b'q' as c_int;
            nv_operator(cap);
            return;
        }

        if checkclearop(&mut *cap.oap) {
            return;
        }

        if cap.nchar == b'=' as c_int && cap.count0 > 2 {
            clearopbeep(&mut *cap.oap);
        } else if cap.nchar == b'=' as c_int {
            mc_follow_set(if cap.count0 == 0 {
                kNone
            } else if cap.count0 == 1 {
                kTrue
            } else {
                kFalse
            });
        } else if cap.nchar == b':' as c_int
            || cap.nchar == b'/' as c_int
            || cap.nchar == b'?' as c_int
        {
            if !g_st().cmdwin_buf.is_null() {
                emsg(tr(e_cmdline_window_already_open.as_ptr()));
                return;
            }
            if cap.nchar == b':' as c_int && cap.count0 > 0 {
                do_cmdline_cmd(c"exmode".as_ptr());
                return;
            }
            let insert_range = g_st().Visual.active && cap.nchar == b':' as c_int;
            let mut fc: [c_char; 2] = [cap.nchar as c_char, 0];
            let mut tv_args: [typval_T; 4] = [
                typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval::String(fc.as_mut_ptr()),
                },
                typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval::String(if insert_range {
                        c"'<,'>".as_ptr().cast_mut()
                    } else {
                        c"".as_ptr().cast_mut()
                    }),
                },
                typval_T {
                    v_type: VarType::VAR_NUMBER,
                    v_lock: 0,
                    vval: TypvalVval::Number(if insert_range { 5 } else { 1 }),
                },
                typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                },
            ];
            nlua_call_typval(
                c"vim._core.cmdwin".as_ptr(),
                c"open".as_ptr(),
                tv_args.as_mut_ptr(),
                ptr::null_mut(),
            );
        } else if g_st().reg_executing == 0 && do_record(cap.nchar) == FAIL {
            clearopbeep(&mut *cap.oap);
        }
    }
}

fn nv_at(cap: &mut cmdarg_T) {
    unsafe {
        if checkclearop(&mut *cap.oap) {
            return;
        }
        if cap.nchar == b'=' as c_int && get_expr_register() == NUL as c_int {
            return;
        }
        atom_macro_start(cap.nchar);
        while {
            let prev = cap.count1;
            cap.count1 -= 1;
            prev != 0 && !g_st().got_int
        } {
            if do_execreg(cap.nchar, 0, 0, 0) == 0 {
                clearopbeep(&mut *cap.oap);
                break;
            }
            line_breakcheck();
        }
    }
}

fn nv_halfpage(cap: &mut cmdarg_T) {
    unsafe {
        if !checkclearop(&mut *cap.oap) {
            pagescroll(
                if cap.cmdchar == Ctrl_D {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count0,
                true,
            );
        }
    }
}

fn nv_join(cap: &mut cmdarg_T) {
    unsafe {
        if g_st().Visual.active {
            nv_operator(cap);
            return;
        }

        if checkclearop(&mut *cap.oap) {
            return;
        }

        cap.count0 = cap.count0.max(2);

        if (**curwin.as_ptr()).w_cursor.lnum + cap.count0 - 1 > (*g_st().curbuf).b_ml.ml_line_count {
            if cap.count0 <= 2 {
                clearopbeep(&mut *cap.oap);
                return;
            }
            cap.count0 = (*g_st().curbuf).b_ml.ml_line_count - (**curwin.as_ptr()).w_cursor.lnum + 1;
        }

        prep_redo_cmd(cap);
        do_join(
            cap.count0 as usize,
            cap.nchar == NUL as c_int,
            true,
            true,
            true,
        );
    }
}

fn nv_put(cap: &mut cmdarg_T) {
    nv_put_opt(cap, false);
}

fn nv_put_opt(cap: &mut cmdarg_T, fix_indent: bool) {
    unsafe {
        let mut savereg: Option<yankreg_T> = None;
        let mut empty = false;
        let mut was_visual = false;
        let mut dir: c_int;
        let mut flags: c_int = 0;
        let save_fen = (**curwin.as_ptr()).w_onebuf_opt.wo_fen;

        if (*cap.oap).op_type != OP_NOP {
            if (*cap.oap).op_type == OP_DELETE && cap.cmdchar == b'p' as c_int {
                atom_capture_op(&mut *cap.oap, cap, false);
                clearop(&mut *cap.oap);
                assert!(cap.opcount >= 0);
                nv_diffgetput(true, cap.opcount as usize);
            } else {
                clearopbeep(&mut *cap.oap);
            }
            return;
        }

        if bt_prompt(g_st().curbuf) && !prompt_curpos_editable() {
            if (**curwin.as_ptr()).w_cursor.lnum == (*g_st().curbuf).b_prompt_start.mark.lnum {
                (**curwin.as_ptr()).w_cursor.col = (*g_st().curbuf).b_prompt_start.mark.col;
                cap.cmdchar = b'P' as c_int;
            } else {
                clearopbeep(&mut *cap.oap);
                return;
            }
        }

        if fix_indent {
            dir = if cap.cmdchar == b']' as c_int && cap.nchar == b'p' as c_int {
                FORWARD
            } else {
                BACKWARD
            };
            flags |= PUT_FIXINDENT;
        } else {
            dir = if cap.cmdchar == b'P' as c_int
                || ((cap.cmdchar == b'g' as c_int || cap.cmdchar == b'z' as c_int)
                    && cap.nchar == b'P' as c_int)
            {
                BACKWARD
            } else {
                FORWARD
            };
        }
        let mut vatom_prepped = false;
        if g_st().Visual.active {
            let mut spec = CmdSpec::new();
            spec.regname = (*cap.oap).regname;
            spec.count = c_long::from(cap.count0);
            spec.cmd = cap.cmdchar;
            spec.cmd2 = cap.nchar;
            vatom_prepped = atom_visual_end(spec, true);
        }
        if !vatom_prepped {
            prep_redo_cmd(cap);
        }
        if cap.cmdchar == b'g' as c_int {
            flags |= PUT_CURSEND;
        } else if cap.cmdchar == b'z' as c_int {
            flags |= PUT_BLOCK_INNER;
        }

        if g_st().Visual.active {
            was_visual = true;
            let regname = (*cap.oap).regname;
            let keep_registers = cap.cmdchar == b'P' as c_int;
            let clipoverwrite = (regname == b'+' as c_int || regname == b'*' as c_int)
                && (opt_vars().cb_flags & (kOptCbFlagUnnamed | kOptCbFlagUnnamedplus)) != 0;
            if regname == 0
                || regname == b'"' as c_int
                || clipoverwrite
                || ascii_isdigit(regname)
                || regname == b'-' as c_int
            {
                savereg = Some(copy_register(regname));
            }

            (**curwin.as_ptr()).w_onebuf_opt.wo_fen = 0;

            if !g_st().Visual.active || g_st().Visual.mode == b'V' as c_int || regname != b'.' as c_int {
                cap.cmdchar = b'd' as c_int;
                cap.nchar = NUL as c_int;
                (*cap.oap).regname = if keep_registers {
                    b'_' as c_int
                } else {
                    NUL as c_int
                };
                g_st().msg_silent += 1;
                atom_suppress(true);
                nv_operator(cap);
                do_pending_operator(cap, 0, false);
                atom_suppress(false);
                empty = ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0;
                g_st().msg_silent -= 1;

                (*cap.oap).regname = regname;
            }

            if g_st().Visual.mode == b'V' as c_int {
                flags |= PUT_LINE;
            } else if g_st().Visual.mode == b'v' as c_int {
                flags |= PUT_LINE_SPLIT;
            }
            if g_st().Visual.mode == Ctrl_V && dir == FORWARD {
                flags |= PUT_LINE_FORWARD;
            }
            dir = BACKWARD;
            if (g_st().Visual.mode != b'V' as c_int && (**curwin.as_ptr()).w_cursor.col < (*g_st().curbuf).b_op_start.col)
                || (g_st().Visual.mode == b'V' as c_int
                    && (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_op_start.lnum)
            {
                dir = FORWARD;
            }
            g_st().Visual.active = true;
        }
        let savereg_ptr = savereg
            .as_mut()
            .map_or(ptr::null_mut(), |r| r as *mut yankreg_T);
        do_put(
            (*cap.oap).regname,
            savereg_ptr,
            dir,
            cap.count1,
            flags,
        );
        drop(savereg);

        if was_visual {
            if save_fen != 0 {
                (**curwin.as_ptr()).w_onebuf_opt.wo_fen = 1;
            }
            (*g_st().curbuf).b_visual.vi_start = (*g_st().curbuf).b_op_start;
            (*g_st().curbuf).b_visual.vi_end = (*g_st().curbuf).b_op_end;
            if *opt_vars().p_sel == b'e' as c_char {
                inc(&mut (*g_st().curbuf).b_visual.vi_end);
            }
        }

        if empty && *ml_get((*g_st().curbuf).b_ml.ml_line_count) == NUL {
            ml_delete_flags((*g_st().curbuf).b_ml.ml_line_count, ML_DEL_MESSAGE);
            deleted_lines((*g_st().curbuf).b_ml.ml_line_count + 1, 1);

            if (**curwin.as_ptr()).w_cursor.lnum > (*g_st().curbuf).b_ml.ml_line_count {
                (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_ml.ml_line_count;
                coladvance(&mut **curwin.as_ptr(), MAXCOL);
            }
        }
        auto_format(false, true);
    }
}

fn nv_open(cap: &mut cmdarg_T) {
    unsafe {
        if (*cap.oap).op_type == OP_DELETE && cap.cmdchar == b'o' as c_int {
            atom_capture_op(&mut *cap.oap, cap, false);
            clearop(&mut *cap.oap);
            assert!(cap.opcount >= 0);
            nv_diffgetput(false, cap.opcount as usize);
        } else if g_st().Visual.active {
            v_swap_corners(cap.cmdchar);
        } else if bt_prompt(g_st().curbuf) && (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_prompt_start.mark.lnum
        {
            clearopbeep(&mut *cap.oap);
        } else {
            n_opencmd(cap);
        }
    }
}

fn nv_paste(cap: &mut cmdarg_T) {
    paste_repeat(cap.count1);
}

fn nv_event(cap: &mut cmdarg_T) {
    g_st().may_garbage_collect = false;
        let may_restart = g_st().restart_edit != 0 || g_st().Visual.restart_select != 0;
        state_handle_k_event();
        g_st().finish_op = false;
        if may_restart {
            cap.retval |= CA_COMMAND_BUSY;
        }
}

pub fn normal_cmd(oap: &mut oparg_T, toplevel: bool) {
    let mut s = NormalState::new();
        normal_state_init(&mut s);
        s.toplevel = toplevel;
        s.oa = *oap;
        normal_prepare(&mut s);
        normal_execute(&mut s, safe_vgetc());
        *oap = s.oa;
}

pub fn exec_stuffed(mut cap: Option<&mut cmdarg_T>) {
    unsafe {
        if let Some(ref cap) = cap {
            atom_stuff_start(cap);
        }
        if text_locked() || curbuf_locked() {
            return;
        }
        let mut oa = oparg_T::new();
        let oap = if let Some(ref cap) = cap {
            cap.oap
        } else {
            &raw mut oa
        };
        g_st().finish_op = false;
        while !stuff_empty() && !g_st().got_int {
            update_topline_cursor();
            normal_cmd(&mut *oap, true);
        }
        g_st().finish_op = false;
        if let Some(ref mut cap) = cap
            && g_st().restart_edit != 0
        {
            cap.retval |= CA_COMMAND_BUSY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pos_helpers() {
        let mut p1 = pos_T {
            lnum: 1,
            col: 5,
            coladd: 0,
        };
        let p2 = pos_T {
            lnum: 2,
            col: 0,
            coladd: 0,
        };
        assert!(lt(p1, p2));
        assert!(!equalpos(p1, p2));
        clearpos(&mut p1);
        assert_eq!(p1.lnum, 0);
        assert_eq!(p1.col, 0);
        assert_eq!(p1.coladd, 0);
        assert!(ascii_iswhite(b' ' as c_int));
        assert!(ascii_isdigit(b'5' as c_int));
    }

    #[test]
    fn test_is_ident() {
        let line = c"int foo = 1; /* bar */ // baz";
            assert!(is_ident(line.as_ptr(), 4));
            assert!(!is_ident(line.as_ptr(), 16));
            assert!(!is_ident(line.as_ptr(), 26));
    }
}
