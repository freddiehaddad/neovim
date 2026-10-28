//! Normal and Visual mode command processing (`src/nvim/normal.c`).

use crate::api::private::helpers::{array_obj, boolean_obj, cstr_as_obj, integer_obj};
use crate::api::private::validate::{Array, Object};
use crate::arglist::ML_EMPTY;
use crate::autocmd::{EVENT_CURSORMOVED, EVENT_TEXTCHANGED, apply_autocmds, has_event};
use crate::buffer::{
    FileBuf, GETF_ALT, GETF_SETMARK, K_OPT_JOP_FLAG_VIEW, K_SHM_FILEINFO, K_UIMESSAGES, MAXLNUM,
    buf_get_changedtick, buf_hide, buflist_getfile, fileinfo, modifiable,
};
use crate::change::{
    COM_MAX_LEN, K_OPT_VE_FLAG_ONEMORE, MB_MAXCHAR, ML_DEL_MESSAGE, OPENLINE_DO_COM, changed_lines,
    del_chars, deleted_lines, get_leader_len, ins_char, ins_char_bytes, open_line,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, ascii_iswhite_or_nul, is_special, skipwhite, transchar,
    vim_isprintc, vim_iswordp, vim_strsize,
};
use crate::cmdexpand::{CTRL_N, CTRL_P, RE_LAST, SEARCH_OPT, SEARCH_START};
use crate::cmdhist::{HIST_SEARCH, add_to_history, init_history};
use crate::cursor::{
    Colnr, CursorPos, K_OPT_VE_FLAG_ALL, Linenr, adjust_cursor_col, check_cursor, check_cursor_col,
    check_cursor_lnum, clearpos, coladvance, coladvance_force, dec_cursor, equalpos, gchar_cursor,
    get_cursor_line_len, get_cursor_line_ptr, get_cursor_pos_len, get_cursor_pos_ptr, getviscol,
    inc_cursor, lt, set_leftcol,
};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::diff::{diff_move_to, diff_set_topline, ex_diffupdate, nv_diffgetput};
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST, get_digraph};
use crate::drawline::{SMT_ALL, VALID_CROW, VALID_WCOL};
use crate::drawscreen::{
    GETVCOL_END_EXCL_LBR, HLF_MSG, K_OPT_VE_FLAG_BLOCK, UPD_CLEAR, UPD_INVERTED, UPD_NOT_VALID,
    UPD_SOME_VALID, UPD_VALID, conceal_check_cursor_line, redraw_curbuf_later, redraw_later,
    redraw_statuslines, setcursor, show_cursor_info_later, showmode, update_screen,
    win_cursorline_standout,
};
use crate::errors::*;
use crate::eval::K_MARK_ALL;
use crate::eval::buffer::prompt_invoke_callback;
use crate::eval::funcs::{MT_CHAR_WISE, MT_LINE_WISE, MT_UNKNOWN};
use crate::eval::typval::{TypVal, TypvalVval, VarType, Varnumber, tv_clear};
use crate::eval::vars::{VV_OP, set_reg_var, set_vcount, set_vim_var_string};
use crate::ex_cmds::{
    B_IMODE_LMAP, BL_FIX, BL_SOL, BL_WHITE, CTRL_C_CONST, CTRL_E, CTRL_Y, ECMD_HIDE, ECMD_LAST,
    do_ascii, do_ecmd,
};
use crate::ex_cmds2::autowrite;
use crate::ex_docmd::{
    ACTION_GOTO, ACTION_SHOW, ACTION_SHOW_ALL, DOCMD_KEEPLINE, DT_POP, FIND_ANY, FIND_DEFINE,
    SEARCH_MSG, do_cmdline, do_cmdline_cmd, do_sleep, update_topline_cursor,
};
use crate::ex_eval::discard_current_exception;
use crate::ex_getln::{
    CTRL_A, CTRL_B_CONST, CTRL_BSL, CTRL_CONST_CONST, CTRL_D_CONST, CTRL_F_CONST, CTRL_G,
    CTRL_HAT_CONST, CTRL_K_CONST, CTRL_L_CONST, CTRL_O, CTRL_Q_CONST, CTRL_R_CONST, CTRL_RSB_CONST,
    CTRL_T_CONST, CTRL_U, CTRL_W_CONST, CTRL_Z_CONST, K_EVENT, K_LUA, K_SELECT, VSE_NONE,
    cmdwin_do_action, compute_cmdrow, curbuf_locked, getcmdline, text_locked, text_locked_msg,
    vim_strsave_fnameescape,
};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE};
use crate::file_search::grab_file_name;
use crate::fileio::check_timestamps;
use crate::fold::{
    clear_folding, close_fold, close_fold_recurse, delete_fold, fold_adjust_visual,
    fold_check_close, fold_manual_allowed, fold_move_to, fold_open_cursor,
    fold_update_after_insert, foldmethod_is_manual, foldmethod_is_marker, get_deepest_nesting,
    has_any_folding, has_folding, open_fold, open_fold_recurse,
};
use crate::globals::*;
use crate::grid::{grid_line_flush, grid_line_puts, grid_line_start};
use crate::help::ex_help;
use crate::indent_c::{FM_BACKWARD, FM_SKIPCOMM, check_linecomment};
use crate::input::{
    K_INS_NONE, K_PASTE_START, beep_flush, char_avail, gotchars_ignore, langmap_adjust,
    map_execute_lua, paste_repeat, plain_vgetc, prep_redo, readbuf1_empty, redo_append_char,
    redo_append_str, requeue_key, safe_vgetc, start_redo, stuff_empty, stuff_readbuff,
    stuffchar_readbuff, stuffnum_readbuff, typebuf_maplen, typebuf_typed, vgetc, vpeekc, vungetc,
};
use crate::input_cmdatom::{
    CmdSpec, Cmdarg, InsSession, K_VINS_NONE, atom_capture_op, atom_cmd_end, atom_cmd_spec,
    atom_cmd_start, atom_ins_end, atom_ins_start, atom_macro_start, atom_stuff_start,
    atom_suppress, atom_visual_end,
};
use crate::insert::{
    CTRL_S_CONST, CTRL_X_CONST, K_OPT_FDO_FLAG_ALL, K_OPT_FDO_FLAG_HOR, MOD_MASK_CTRL, beginline,
    cursor_down, cursor_down_inner, cursor_up, cursor_up_inner, edit, get_literal, ins_copychar,
    oneleft, oneright, prompt_curpos_editable, set_last_insert,
};
use crate::insexpand::CTRL_I_CONST;
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
    Fmark, K_MARK_BEGIN_LINE, K_MARK_CHANGED_CURSOR, K_MARK_CHANGED_LINE, K_MARK_CONTEXT,
    K_MARK_JUMP_LIST, K_MARK_MOVE_FAILED, K_MARK_MOVE_SUCCESS, K_MARK_SET_VIEW,
    K_MARK_SWITCHED_BUF, MarkMove, MarkMoveRes, checkpcmark, get_changelist, get_jumplist,
    getnextmark, mark_get, mark_mb_adjustpos, mark_move_to, pos_to_mark, setmark, setpcmark,
};
use crate::r#match::{HLF_L, HLF_LC};
use crate::math::vim_append_digit_int;
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
use crate::regexp::{CTRL_H_CONST, CTRL_V_CONST, MAXCOL};
use crate::register::{
    PUT_BLOCK_INNER, PUT_CURSEND, PUT_FIXINDENT, PUT_LINE, PUT_LINE_FORWARD, PUT_LINE_SPLIT,
    Yankreg, copy_register, do_execreg, do_put, do_record, get_default_register_name,
    get_expr_register, valid_yank_reg,
};
use crate::search::{
    BACKWARD, FORWARD, SearchitArg, current_search, do_search, find_pattern_in_path,
    findmatchlimit, reset_search_dir, searchc, searchit,
};
use crate::spell::spell_move_to;
use crate::spellfile::{SPELL_ADD_BAD, SPELL_ADD_GOOD, spell_add_word};
use crate::spellsuggest::spell_suggest;
use crate::state::{
    EditorState, MODE_INSERT, MODE_LANGMAP, MODE_LREPLACE, MODE_NORMAL, MODE_NORMAL_BUSY,
    MODE_REPLACE, MODE_SELECT, MODE_TERMINAL, get_real_state, may_trigger_modechanged,
    may_trigger_safestate, state_enter, state_handle_k_event, state_no_longer_safe, virtual_active,
};
use crate::statusline::{draw_tabline, win_redr_status};
use crate::strings::{tr, vim_snprintf, vim_strchr, vim_strsave_shellescape};
use crate::syntax::syn_stack_free_all;
use crate::tag::do_tag;
use crate::terminal::terminal_check_refresh;
use crate::textformat::{Oparg, auto_format, has_format_option};
use crate::textobject::{
    bck_word, bckend_word, current_block, current_line, current_par, current_quote, current_sent,
    current_tagblock, current_word, end_word, findpar, findsent, fwd_word,
};
use crate::ui::{
    ui_call_msg_showcmd, ui_cursor_shape, ui_cursor_shape_no_check_conceal, ui_flush, ui_has,
    vim_beep,
};
use crate::undo::{
    any_buf_is_changed, curbuf_is_changed_fn, u_clearline, u_redo, u_save, u_save_cursor,
    u_savesub, u_undo, u_undoline, undo_time,
};
use crate::window::{
    WinInfoInfo, check_can_set_curbuf_disabled, do_window, goto_tabpage, goto_tabpage_lastused,
    may_make_initial_scroll_size_snapshot, may_trigger_win_scrolled_resized, set_fraction,
    win_setheight,
};
use std::ffi::{CStr, c_char};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Visualinfo {
    pub vi_start: CursorPos,
    pub vi_end: CursorPos,
    pub vi_mode: i32,
    pub vi_curswant: i32,
}

impl Visualinfo {
    pub const fn new() -> Self {
        Self {
            vi_start: CursorPos::new(),
            vi_end: CursorPos::new(),
            vi_mode: 0,
            vi_curswant: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VisualExtent {
    pub mode: i32,
    pub line_count: Linenr,
    pub vcol: Colnr,
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
    pub start: CursorPos,
    pub active: bool,
    pub select: bool,
    pub select_reg: i32,
    pub select_exclu_adj: bool,
    pub restart_select: i32,
    pub need_end: bool,
    pub reselect: i32,
    pub mode: i32,
    pub resel: VisualExtent,
}

impl VisualState {
    pub const fn new() -> Self {
        Self {
            start: CursorPos::new(),
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

pub const FIND_IDENT: i32 = 1;
pub const FIND_STRING: i32 = 2;
pub const FIND_EVAL: i32 = 4;

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

pub const CA_COMMAND_BUSY: i32 = 1;
pub const CA_NO_ADJ_OP_END: i32 = 2;

pub const REPLACE_CR_NCHAR: i32 = -1;
pub const REPLACE_NL_NCHAR: i32 = -2;

pub const SHOWCMD_COLS: usize = 10;
pub const SHOWCMD_BUFLEN: usize = SHOWCMD_COLS + 1 + 30;

const POUND: i32 = 0xA3;

pub(crate) const K_OPT_FOLDENABLE: OptIndex = 103;
pub(crate) const K_OPT_FOLDLEVEL: OptIndex = 106;

const K_CPO_CHANGEW: i32 = b'_' as i32;
const K_CPO_DIGRAPH: i32 = b'D' as i32;
pub(crate) const K_CPO_MATCH: i32 = b'%' as i32;

const K_FO_OPEN_COMS: i32 = b'o' as i32;

pub(crate) const K_SHM_SEARCHCOUNT: i32 = b'S' as i32;

const K_OPT_BO_FLAG_ESC: u32 = 0x80;
pub(crate) const K_OPT_CB_FLAG_UNNAMED_CONST: u32 = 0x01;
pub(crate) const K_OPT_CB_FLAG_UNNAMEDPLUS_CONST: u32 = 0x02;

const K_OPT_FDO_FLAG_BLOCK: u32 = 0x02;
const K_OPT_FDO_FLAG_MARK: u32 = 0x08;
const K_OPT_FDO_FLAG_PERCENT: u32 = 0x10;
pub(crate) const K_OPT_FDO_FLAG_SEARCH: u32 = 0x40;
const K_OPT_FDO_FLAG_JUMP: u32 = 0x400;

const KMARK_NO_CONTEXT: i32 = 4;

pub(crate) const FM_FORWARD: i32 = 0x02;
pub(crate) const SEARCH_REV: i32 = 0x01;
pub(crate) const SEARCH_ECHO: i32 = 0x02;
pub(crate) const SEARCH_MARK: i32 = 0x200;
pub(crate) const SMT_BAD: i32 = 1;
pub(crate) const SMT_RARE: i32 = 2;

type NvFunc = fn(cap: &mut Cmdarg);

fn nv_scroll_line_cmd(cap: &mut Cmdarg) {
    nv_scroll_line(cap)
}

fn nv_mousescroll_cmd(cap: &mut Cmdarg) {
    nv_mousescroll(cap)
}

fn nv_mouse_cmd(cap: &mut Cmdarg) {
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
    oa: Oparg,
    ca: Cmdarg,
    mapped_len: i32,
    old_mapped_len: i32,
    idx: i32,
    c: i32,
    old_col: i32,
    old_pos: CursorPos,
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
            oa: Oparg::new(),
            ca: Cmdarg::new(),
            mapped_len: 0,
            old_mapped_len: 0,
            idx: 0,
            c: 0,
            old_col: 0,
            old_pos: CursorPos::new(),
        }
    }
}

impl EditorState for NormalState {
    fn check(&mut self) -> i32 {
        normal_check(self)
    }

    fn execute(&mut self, key: i32) -> i32 {
        normal_execute(self, key)
    }
}

impl Default for NormalState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct NvCmd {
    cmd_char: i32,
    cmd_func: NvFunc,
    cmd_flags: u16,
    cmd_arg: i16,
}

struct NormalModuleState {
    visual_mode_orig: i32,
    current_oap: *mut Oparg,
    old_showcmd_buf: [c_char; SHOWCMD_BUFLEN],
    showcmd_is_clear: bool,
    showcmd_visual: bool,
    do_check_scrollbind_old_curwin: *mut WinInfoInfo,
    do_check_scrollbind_old_vtopline: Linenr,
    do_check_scrollbind_old_buf: *mut FileBuf,
    do_check_scrollbind_old_leftcol: Colnr,
    nv_cmd_idx: [i16; NV_CMDS_SIZE],
    nv_max_linear: i32,
}

impl NormalModuleState {
    const fn new() -> Self {
        Self {
            visual_mode_orig: 0,
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
pub fn lineempty(lnum: Linenr) -> bool {
    unsafe { *ml_get(lnum) == NUL }
}

use crate::buffer::{bt_cmdwin, bt_prompt, bt_quickfix};
use crate::highlight::{hl_attr, win_hl_attr};

#[inline]
fn normal_state_init(s: &mut NormalState) {
    *s = NormalState::new();
}

const NV_CMDS_SIZE: usize = 188;

const NV_CMDS: [NvCmd; NV_CMDS_SIZE] = [
    NvCmd {
        cmd_char: NUL as i32,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_A,
        cmd_func: nv_addsub,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_B_CONST,
        cmd_func: nv_page,
        cmd_flags: NV_STS,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: CTRL_C_CONST,
        cmd_func: nv_esc,
        cmd_flags: 0,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: CTRL_D_CONST,
        cmd_func: nv_halfpage,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_E,
        cmd_func: nv_scroll_line_cmd,
        cmd_flags: 0,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: CTRL_F_CONST,
        cmd_func: nv_page,
        cmd_flags: NV_STS,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: CTRL_G,
        cmd_func: nv_ctrlg,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_H_CONST,
        cmd_func: nv_ctrlh,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_I_CONST,
        cmd_func: nv_pcmark,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: NL,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_K_CONST,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_L_CONST,
        cmd_func: nv_clear,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CAR,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: CTRL_N,
        cmd_func: nv_down,
        cmd_flags: NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_O,
        cmd_func: nv_ctrlo,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_P,
        cmd_func: nv_up,
        cmd_flags: NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_Q_CONST,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_R_CONST,
        cmd_func: nv_redo_or_register,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_S_CONST,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_T_CONST,
        cmd_func: nv_tagpop,
        cmd_flags: NV_NCW | NV_JUMP,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_U,
        cmd_func: nv_halfpage,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_V_CONST,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'V' as i32,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'v' as i32,
        cmd_func: nv_visual,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_W_CONST,
        cmd_func: nv_window,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_X_CONST,
        cmd_func: nv_addsub,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_Y,
        cmd_func: nv_scroll_line_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_Z_CONST,
        cmd_func: nv_suspend,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: ESC,
        cmd_func: nv_esc,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_BSL,
        cmd_func: nv_normal,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_RSB_CONST,
        cmd_func: nv_ident,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_HAT_CONST,
        cmd_func: nv_hat,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: CTRL_CONST_CONST,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b' ' as i32,
        cmd_func: nv_right,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'!' as i32,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'"' as i32,
        cmd_func: nv_regname,
        cmd_flags: NV_NCH_NOP | NV_NCH_ARG | NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'#' as i32,
        cmd_func: nv_ident,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'$' as i32,
        cmd_func: nv_dollar,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'%' as i32,
        cmd_func: nv_percent,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'&' as i32,
        cmd_func: nv_optrans,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'\'' as i32,
        cmd_func: nv_gomark,
        cmd_flags: NV_NCH_ALW | NV_NCH_ARG,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'(' as i32,
        cmd_func: nv_brace,
        cmd_flags: NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: b')' as i32,
        cmd_func: nv_brace,
        cmd_flags: NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: b'*' as i32,
        cmd_func: nv_ident,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'+' as i32,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b',' as i32,
        cmd_func: nv_csearch,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'-' as i32,
        cmd_func: nv_up,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'.' as i32,
        cmd_func: nv_dot,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'/' as i32,
        cmd_func: nv_search,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'0' as i32,
        cmd_func: nv_beginline,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'1' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'2' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'3' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'4' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'5' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'6' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'7' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'8' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'9' as i32,
        cmd_func: nv_ignore,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b':' as i32,
        cmd_func: nv_colon,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b';' as i32,
        cmd_func: nv_csearch,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'<' as i32,
        cmd_func: nv_operator,
        cmd_flags: NV_RL,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'=' as i32,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'>' as i32,
        cmd_func: nv_operator,
        cmd_flags: NV_RL,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'?' as i32,
        cmd_func: nv_search,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'@' as i32,
        cmd_func: nv_at,
        cmd_flags: NV_NCH_NOP | NV_NCH_ARG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'A' as i32,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'B' as i32,
        cmd_func: nv_bck_word,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'C' as i32,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'D' as i32,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'E' as i32,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'F' as i32,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: b'G' as i32,
        cmd_func: nv_goto,
        cmd_flags: NV_JUMP,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'H' as i32,
        cmd_func: nv_scroll,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'I' as i32,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'J' as i32,
        cmd_func: nv_join,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'K' as i32,
        cmd_func: nv_ident,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'L' as i32,
        cmd_func: nv_scroll,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'M' as i32,
        cmd_func: nv_scroll,
        cmd_flags: NV_JUMP,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'N' as i32,
        cmd_func: nv_next,
        cmd_flags: NV_MOTION,
        cmd_arg: SEARCH_REV as i16,
    },
    NvCmd {
        cmd_char: b'O' as i32,
        cmd_func: nv_open,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'P' as i32,
        cmd_func: nv_put,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'Q' as i32,
        cmd_func: nv_q_fn,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'R' as i32,
        cmd_func: nv_replace_fn,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'S' as i32,
        cmd_func: nv_subst,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'T' as i32,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: b'U' as i32,
        cmd_func: nv_undo_fn,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'W' as i32,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: b'X' as i32,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'Y' as i32,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'Z' as i32,
        cmd_func: nv_zet_fn,
        cmd_flags: NV_NCH_NOP | NV_NCW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'[' as i32,
        cmd_func: nv_brackets,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: b'\\' as i32,
        cmd_func: nv_error,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b']' as i32,
        cmd_func: nv_brackets,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: b'^' as i32,
        cmd_func: nv_beginline,
        cmd_flags: NV_MOTION,
        cmd_arg: (BL_WHITE | BL_FIX) as i16,
    },
    NvCmd {
        cmd_char: b'_' as i32,
        cmd_func: nv_lineop,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'`' as i32,
        cmd_func: nv_gomark,
        cmd_flags: NV_NCH_ALW | NV_NCH_ARG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'a' as i32,
        cmd_func: nv_edit,
        cmd_flags: NV_NCH,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'b' as i32,
        cmd_func: nv_bck_word,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'c' as i32,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'd' as i32,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'e' as i32,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'f' as i32,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: b'g' as i32,
        cmd_func: nv_g_cmd,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'h' as i32,
        cmd_func: nv_left,
        cmd_flags: NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'i' as i32,
        cmd_func: nv_edit,
        cmd_flags: NV_NCH,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'j' as i32,
        cmd_func: nv_down,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'k' as i32,
        cmd_func: nv_up,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'l' as i32,
        cmd_func: nv_right,
        cmd_flags: NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'm' as i32,
        cmd_func: nv_mark,
        cmd_flags: NV_NCH_NOP | NV_NCH_ARG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'n' as i32,
        cmd_func: nv_next,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'o' as i32,
        cmd_func: nv_open,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'p' as i32,
        cmd_func: nv_put,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'q' as i32,
        cmd_func: nv_q,
        cmd_flags: NV_NCH | NV_NCH_ARG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'r' as i32,
        cmd_func: nv_replace,
        cmd_flags: NV_NCH_NOP | NV_LANG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b's' as i32,
        cmd_func: nv_subst,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b't' as i32,
        cmd_func: nv_csearch,
        cmd_flags: NV_NCH_ALW | NV_LANG | NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: b'u' as i32,
        cmd_func: nv_undo,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'w' as i32,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'x' as i32,
        cmd_func: nv_abbrev,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'y' as i32,
        cmd_func: nv_operator,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'z' as i32,
        cmd_func: nv_zet,
        cmd_flags: NV_NCH_ALW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'{' as i32,
        cmd_func: nv_findpar,
        cmd_flags: NV_MOTION,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: b'|' as i32,
        cmd_func: nv_pipe,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: b'}' as i32,
        cmd_func: nv_findpar,
        cmd_flags: NV_MOTION,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: b'~' as i32,
        cmd_func: nv_tilde,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: POUND,
        cmd_func: nv_ident,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_MOUSEUP,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_UP as i16,
    },
    NvCmd {
        cmd_char: K_MOUSEDOWN,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_DOWN as i16,
    },
    NvCmd {
        cmd_char: K_MOUSELEFT,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_LEFT as i16,
    },
    NvCmd {
        cmd_char: K_MOUSERIGHT,
        cmd_func: nv_mousescroll_cmd,
        cmd_flags: 0,
        cmd_arg: MSCR_RIGHT as i16,
    },
    NvCmd {
        cmd_char: K_LEFTMOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_LEFTMOUSE_NM,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_LEFTDRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_LEFTRELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_LEFTRELEASE_NM,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_MOUSEMOVE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_MIDDLEMOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_MIDDLEDRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_MIDDLERELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_RIGHTMOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_RIGHTDRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_RIGHTRELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_X1MOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_X1DRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_X1RELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_X2MOUSE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_X2DRAG,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_X2RELEASE,
        cmd_func: nv_mouse_cmd,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_IGNORE,
        cmd_func: nv_ignore,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_NOP,
        cmd_func: nv_nop,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_INS,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_KINS,
        cmd_func: nv_edit,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_BS,
        cmd_func: nv_ctrlh,
        cmd_flags: NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_UP,
        cmd_func: nv_up,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_S_UP,
        cmd_func: nv_page,
        cmd_flags: NV_SS,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: K_DOWN,
        cmd_func: nv_down,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_S_DOWN,
        cmd_func: nv_page,
        cmd_flags: NV_SS,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: K_LEFT,
        cmd_func: nv_left,
        cmd_flags: NV_SSS | NV_STS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_S_LEFT,
        cmd_func: nv_bck_word,
        cmd_flags: NV_SS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_C_LEFT,
        cmd_func: nv_bck_word,
        cmd_flags: NV_SSS | NV_RL | NV_STS | NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: K_RIGHT,
        cmd_func: nv_right,
        cmd_flags: NV_SSS | NV_STS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_S_RIGHT,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_SS | NV_RL | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_C_RIGHT,
        cmd_func: nv_wordcmd,
        cmd_flags: NV_SSS | NV_RL | NV_STS | NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: K_PAGEUP,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: K_KPAGEUP,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: BACKWARD as i16,
    },
    NvCmd {
        cmd_char: K_PAGEDOWN,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: K_KPAGEDOWN,
        cmd_func: nv_page,
        cmd_flags: NV_SSS | NV_STS,
        cmd_arg: FORWARD as i16,
    },
    NvCmd {
        cmd_char: K_END,
        cmd_func: nv_end,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_KEND,
        cmd_func: nv_end,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_S_END,
        cmd_func: nv_end,
        cmd_flags: NV_SS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_C_END,
        cmd_func: nv_end,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 1,
    },
    NvCmd {
        cmd_char: K_HOME,
        cmd_func: nv_home,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_KHOME,
        cmd_func: nv_home,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_S_HOME,
        cmd_func: nv_home,
        cmd_flags: NV_SS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_C_HOME,
        cmd_func: nv_goto,
        cmd_flags: NV_SSS | NV_STS | NV_MOTION,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_DEL,
        cmd_func: nv_abbrev,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_KDEL,
        cmd_func: nv_abbrev,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_UNDO,
        cmd_func: nv_kundo,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_HELP,
        cmd_func: nv_help,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_F1,
        cmd_func: nv_help,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_XF1,
        cmd_func: nv_help,
        cmd_flags: NV_NCW,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_SELECT,
        cmd_func: nv_select,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_PASTE_START,
        cmd_func: nv_paste,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_EVENT,
        cmd_func: nv_event,
        cmd_flags: NV_KEEPREG,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_COMMAND,
        cmd_func: nv_colon,
        cmd_flags: 0,
        cmd_arg: 0,
    },
    NvCmd {
        cmd_char: K_LUA,
        cmd_func: nv_colon,
        cmd_flags: 0,
        cmd_arg: 0,
    },
];

static E_CHANGELIST_IS_EMPTY: &CStr = c"E664: Changelist is empty";
static E_CMDLINE_WINDOW_ALREADY_OPEN: &CStr = c"E1292: Command-line window is already open";

/// Initialize the norm_st().nv_cmd_idx[] table.
pub fn init_normal_cmds() {
    unsafe {
        assert!(NV_CMDS_SIZE <= i16::MAX as usize);
        let idx_ptr = (&raw mut norm_st().nv_cmd_idx).cast::<i16>();

        for i in 0..(NV_CMDS_SIZE as i16) {
            *idx_ptr.wrapping_add(i as usize) = i;
        }

        let slice = std::slice::from_raw_parts_mut(idx_ptr, NV_CMDS_SIZE);
        slice.sort_unstable_by_key(|&s| NV_CMDS[s as usize].cmd_char.abs());

        let mut i: i16 = 0;
        while (i as usize) < NV_CMDS_SIZE {
            if i32::from(i) != NV_CMDS[*idx_ptr.wrapping_add(i as usize) as usize].cmd_char {
                break;
            }
            i += 1;
        }
        norm_st().nv_max_linear = i32::from(i) - 1;
    }
}

/// True if command `cmdchar` has any of the given NV_XX `flags`.
pub fn nv_is(cmdchar: i32, flags: u16) -> bool {
    let idx = find_command(cmdchar);
    idx >= 0 && (NV_CMDS[idx as usize].cmd_flags & flags) != 0
}

fn find_command(mut cmdchar: i32) -> i32 {
    unsafe {
        if cmdchar >= 0x100 {
            return -1;
        }
        if cmdchar < 0 {
            cmdchar = -cmdchar;
        }
        assert!(norm_st().nv_max_linear < NV_CMDS_SIZE as i32);
        let idx_ptr = (&raw const norm_st().nv_cmd_idx).cast::<i16>();
        if cmdchar <= norm_st().nv_max_linear {
            return i32::from(*idx_ptr.wrapping_add(cmdchar as usize));
        }

        let mut bot = norm_st().nv_max_linear + 1;
        let mut top = NV_CMDS_SIZE as i32 - 1;
        let mut idx = -1;
        while bot <= top {
            let i = (top + bot) / 2;
            let mut c = NV_CMDS[*idx_ptr.wrapping_add(i as usize) as usize].cmd_char;
            if c < 0 {
                c = -c;
            }
            if cmdchar == c {
                idx = i32::from(*idx_ptr.wrapping_add(i as usize));
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

fn check_text_locked(oap: Option<&mut Oparg>) -> bool {
    {
        if !text_locked() {
            return false;
        }
        if let Some(oap) = oap {
            clearopbeep(oap);
        }
        text_locked_msg();
        true
    }
}

pub fn check_text_or_curbuf_locked(mut oap: Option<&mut Oparg>) -> bool {
    {
        if check_text_locked(oap.as_deref_mut()) {
            return true;
        }
        if !curbuf_locked() {
            return false;
        }
        if let Some(oap) = oap {
            clearop(oap);
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
            && (*norm_st().current_oap).regname == NUL as i32)
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
    s.ca = Cmdarg::new();
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
    g_st().state_global = MODE_NORMAL_BUSY;

    if s.toplevel && readbuf1_empty() {
        set_vcount_ca(&mut s.ca, &mut s.set_prevcount);
    }
}

fn normal_handle_special_visual_command(s: &mut NormalState) -> bool {
    let flags = NV_CMDS[s.idx as usize].cmd_flags;
    if g_st().km_stopsel
        && (flags & NV_STS) != 0
        && (g_st().mod_mask & i32::from(MOD_MASK_SHIFT)) == 0
    {
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
        } else if (flags & NV_SSS) != 0 && (g_st().mod_mask & i32::from(MOD_MASK_SHIFT)) != 0 {
            g_st().mod_mask &= !i32::from(MOD_MASK_SHIFT);
        }
    }
    false
}

fn normal_need_additional_char(s: &mut NormalState) -> bool {
    let flags = NV_CMDS[s.idx as usize].cmd_flags;
    let pending_op = s.oa.op_type != OP_NOP;
    let cmdchar = s.ca.cmdchar;
    (flags & NV_NCH) != 0
        && (((flags & NV_NCH_NOP) == NV_NCH_NOP && !pending_op)
            || (flags & NV_NCH_ALW) == NV_NCH_ALW
            || (cmdchar == b'q' as i32
                && !pending_op
                && g_st().reg_recording == 0
                && g_st().reg_executing == 0)
            || ((cmdchar == b'a' as i32 || cmdchar == b'i' as i32)
                && (pending_op || g_st().visual_global.active)))
}

fn normal_need_redraw_mode_message(s: &mut NormalState) -> bool {
    {
        ((opt_vars().p_smd != 0
            && g_st().msg_silent == 0
            && (g_st().restart_edit != 0
                || (g_st().visual_global.active
                    && s.old_pos.lnum == curwin_mut().w_cursor.lnum
                    && s.old_pos.col == curwin_mut().w_cursor.col))
            && (g_st().clear_cmdline || g_st().redraw_cmdline)
            && (g_st().msg_didout || (g_st().msg_didany && g_st().msg_scroll != 0))
            && !g_st().msg_nowait
            && g_st().key_typed)
            || (g_st().restart_edit != 0
                && !g_st().visual_global.active
                && g_st().msg_scroll != 0
                && g_st().emsg_on_display))
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
    let save_state = g_st().state_global;

    if g_st().restart_edit != 0 {
        g_st().state_global = MODE_INSERT;
    }

    if g_st().must_redraw != 0 && g_st().keep_msg.is_some() && !g_st().emsg_on_display {
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
    g_st().state_global = save_state;

    g_st().msg_scroll = 0;
    g_st().emsg_on_display = false;
}

fn normal_get_additional_char(s: &mut NormalState) {
    unsafe {
        let cp: *mut i32;
        let mut repl = false;
        let mut lit = false;

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        g_st().did_cursorhold = true;
        if s.ca.cmdchar == b'g' as i32 {
            s.ca.nchar = plain_vgetc();
            langmap_adjust(&mut s.ca.nchar, true);
            s.need_flushbuf |= add_to_showcmd(s.ca.nchar);
            if s.ca.nchar == b'r' as i32
                || s.ca.nchar == b'\'' as i32
                || s.ca.nchar == b'`' as i32
                || s.ca.nchar == CTRL_BSL
            {
                cp = &mut s.ca.extra_char;
                if s.ca.nchar != b'r' as i32 {
                    lit = true;
                } else {
                    repl = true;
                }
            } else {
                cp = ptr::null_mut();
            }
        } else {
            if s.ca.cmdchar == b'r' as i32 {
                repl = true;
            }
            cp = &mut s.ca.nchar;
        }
        let lang = repl || (NV_CMDS[s.idx as usize].cmd_flags & NV_LANG) != 0;

        if !cp.is_null() {
            let mut langmap_active = false;
            if repl {
                g_st().state_global = MODE_REPLACE;
                ui_cursor_shape_no_check_conceal();
            }
            if lang && curbuf_mut().b_p_iminsert == B_IMODE_LMAP {
                g_st().no_mapping -= 1;
                g_st().allow_keys -= 1;
                if repl {
                    g_st().state_global = MODE_LREPLACE;
                } else {
                    g_st().state_global = MODE_LANGMAP;
                }
                langmap_active = true;
            }

            *cp = plain_vgetc();

            if langmap_active {
                g_st().no_mapping += 1;
                g_st().allow_keys += 1;
            }
            g_st().state_global = MODE_NORMAL_BUSY;
            s.need_flushbuf |= add_to_showcmd(*cp);

            if !lit {
                if *cp == CTRL_K_CONST
                    && ((NV_CMDS[s.idx as usize].cmd_flags & NV_LANG) != 0
                        || ptr::eq(cp, &s.ca.extra_char))
                    && !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_DIGRAPH)
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
                && s.ca.nchar == CTRL_BSL
                && (s.ca.extra_char == CTRL_N || s.ca.extra_char == CTRL_G)
            {
                s.ca.cmdchar = CTRL_BSL;
                s.ca.nchar = s.ca.extra_char;
                s.idx = find_command(s.ca.cmdchar);
            } else if (s.ca.nchar == b'n' as i32 || s.ca.nchar == b'N' as i32)
                && s.ca.cmdchar == b'g' as i32
            {
                (*s.ca.oap).op_type = get_op_type(*cp, NUL as i32);
            } else if *cp == CTRL_BSL {
                let mut towait = if opt_vars().p_ttm >= 0 {
                    opt_vars().p_ttm as i32
                } else {
                    opt_vars().p_tm as i32
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
                    if s.c != CTRL_N && s.c != CTRL_G {
                        vungetc(s.c);
                    } else {
                        s.ca.cmdchar = CTRL_BSL;
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

                    if !utf_iscomposing(prev_code, s.c, &mut state) {
                        vungetc(s.c);
                        break;
                    }

                    if s.ca.nchar_len == 0 {
                        s.ca.nchar_len =
                            utf_char2bytes(s.ca.nchar, s.ca.nchar_composing.as_mut_ptr());
                    }

                    if ((s.ca.nchar_len + utf_char2len(s.c)) as usize) < s.ca.nchar_composing.len()
                    {
                        s.ca.nchar_len += utf_char2bytes(
                            s.c,
                            s.ca.nchar_composing
                                .as_mut_ptr()
                                .wrapping_add(s.ca.nchar_len as usize),
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
        c if c == b'l' as i32 => b'h' as i32,
        K_RIGHT => K_LEFT,
        K_S_RIGHT => K_S_LEFT,
        K_C_RIGHT => K_C_LEFT,
        c if c == b'h' as i32 => b'l' as i32,
        K_LEFT => K_RIGHT,
        K_S_LEFT => K_S_RIGHT,
        K_C_LEFT => K_C_RIGHT,
        c if c == b'>' as i32 => b'<' as i32,
        c if c == b'<' as i32 => b'>' as i32,
        c => c,
    };
    s.idx = find_command(s.ca.cmdchar);
}

fn normal_get_command_count(s: &mut NormalState) -> bool {
    if g_st().visual_global.active && g_st().visual_global.select {
        return false;
    }
    while (s.c >= b'1' as i32 && s.c <= b'9' as i32)
        || (s.ca.count0 != 0 && (s.c == K_DEL || s.c == K_KDEL || s.c == b'0' as i32))
    {
        if s.c == K_DEL || s.c == K_KDEL {
            s.ca.count0 /= 10;
            del_from_showcmd(4);
        } else if s.ca.count0 > 99_999_999 {
            s.ca.count0 = 999_999_999;
        } else {
            s.ca.count0 = s.ca.count0 * 10 + (s.c - b'0' as i32);
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

    if s.c == CTRL_W_CONST && !s.ctrl_w && s.oa.op_type == OP_NOP {
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
    {
        let mut did_visual_op = false;

        if !s.command_finished {
            if !g_st().finish_op
                && s.oa.op_type == 0
                && (s.idx < 0 || (NV_CMDS[s.idx as usize].cmd_flags & NV_KEEPREG) == 0)
            {
                clearop(&mut s.oa);
                set_reg_var(get_default_register_name());
            }

            if s.old_mapped_len > 0 {
                s.old_mapped_len = typebuf_maplen();
            }

            if s.ca.cmdchar != K_IGNORE && s.ca.cmdchar != K_MOUSEMOVE {
                did_visual_op = g_st().visual_global.active
                    && s.oa.op_type != OP_NOP
                    && s.oa.op_type != OP_COLON;
                do_pending_operator(&mut s.ca, s.old_col, false);
            }

            if normal_need_redraw_mode_message(s) {
                normal_redraw_mode_message(s);
            }
        }

        if g_st().visual_global.need_end {
            g_st().visual_global.need_end = false;
            reset_visual();
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
            || s.ca.cmdchar == b'r' as i32
            || (s.ca.cmdchar == b'g' as i32 && s.ca.nchar == b'r' as i32)
        {
            ui_cursor_shape();
        }

        checkpcmark();
        mb_check_adjust_col(curwin_mut());

        if curwin_mut().w_onebuf_opt.wo_scb != 0 && s.toplevel {
            validate_cursor(curwin_mut());
            do_check_scrollbind(true);
        }

        if curwin_mut().w_onebuf_opt.wo_crb != 0 && s.toplevel {
            validate_cursor(curwin_mut());
            do_check_cursorbind();
        }

        if s.oa.op_type == OP_NOP
            && ((g_st().restart_edit != 0 && !g_st().visual_global.active && s.old_mapped_len == 0)
                || g_st().visual_global.restart_select == 1)
            && (s.ca.retval & CA_COMMAND_BUSY) == 0
            && stuff_empty()
            && s.oa.regname == 0
        {
            if g_st().visual_global.restart_select == 1 {
                g_st().visual_global.select = true;
                g_st().visual_global.select_reg = 0;
                may_trigger_modechanged();
                showmode();
                g_st().visual_global.restart_select = 0;
            }
            if g_st().restart_edit != 0 && !g_st().visual_global.active && s.old_mapped_len == 0 {
                edit(g_st().restart_edit, false, 1);
            }
        }

        if g_st().visual_global.restart_select == 2 {
            g_st().visual_global.restart_select = 1;
        }

        g_st().opcount = s.ca.opcount;

        atom_cmd_end(&mut s.ca);

        if s.oa.op_type == OP_NOP && s.oa.regname == 0 && s.ca.cmdchar != K_EVENT {
            clear_showcmd();
        }
    }
}

fn normal_execute(s: &mut NormalState, key: i32) -> i32 {
    {
        atom_cmd_start(key);

        s.command_finished = false;
        s.ctrl_w = false;
        s.old_col = curwin_mut().w_curswant;
        s.c = key;

        langmap_adjust(&mut s.c, get_real_state() != MODE_SELECT);

        if g_st().restart_edit == 0 {
            s.old_mapped_len = 0;
        } else if s.old_mapped_len != 0
            || (g_st().visual_global.active && s.mapped_len == 0 && typebuf_maplen() > 0)
        {
            s.old_mapped_len = typebuf_maplen();
        }

        if s.c == NUL as i32 {
            s.c = K_ZERO;
        }

        if g_st().visual_global.active
            && g_st().visual_global.select
            && (vim_isprintc(s.c) || s.c == NL || s.c == CAR || s.c == K_KENTER)
        {
            requeue_key(g_st().vgetc_char, g_st().vgetc_mod_mask, 0, true);
            if g_st().restart_edit != 0 {
                s.c = b'd' as i32;
            } else {
                s.c = b'c' as i32;
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
        s.ca.count1 = if s.ca.count0 == 0 { 1 } else { s.ca.count0 };

        if s.toplevel && readbuf1_empty() {
            set_vcount(
                i64::from(s.ca.count0),
                i64::from(s.ca.count1),
                s.set_prevcount,
            );
        }

        if s.ctrl_w {
            s.ca.nchar = s.c;
            s.ca.cmdchar = CTRL_W_CONST;
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

            if (NV_CMDS[s.idx as usize].cmd_flags & NV_NCW) != 0
                && check_text_or_curbuf_locked(Some(&mut s.oa))
            {
                s.command_finished = true;
                break 'finish;
            }

            if g_st().visual_global.active && normal_handle_special_visual_command(s) {
                s.command_finished = true;
                break 'finish;
            }

            if curwin_mut().w_onebuf_opt.wo_rl != 0
                && g_st().key_typed
                && g_st().key_stuffed == 0
                && (NV_CMDS[s.idx as usize].cmd_flags & NV_RL) != 0
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

            g_st().state_global = MODE_NORMAL;

            if s.ca.nchar == ESC || s.ca.extra_char == ESC {
                clearop(&mut s.oa);
                s.command_finished = true;
                break 'finish;
            }

            if s.ca.cmdchar != K_IGNORE {
                g_st().msg_didout = false;
                g_st().msg_col = 0;
            }

            s.old_pos = curwin_mut().w_cursor;

            if !g_st().visual_global.active && g_st().km_startsel {
                let flags = NV_CMDS[s.idx as usize].cmd_flags;
                if (flags & NV_SS) != 0 {
                    start_selection();
                    unshift_special(&mut s.ca);
                    s.idx = find_command(s.ca.cmdchar);
                    assert!(s.idx >= 0);
                } else if (flags & NV_SSS) != 0
                    && (g_st().mod_mask & i32::from(MOD_MASK_SHIFT)) != 0
                {
                    start_selection();
                    g_st().mod_mask &= !i32::from(MOD_MASK_SHIFT);
                }
            }

            s.ca.arg = i32::from(NV_CMDS[s.idx as usize].cmd_arg);
            (NV_CMDS[s.idx as usize].cmd_func)(&mut s.ca);
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
    {
        if !g_st().finish_op
            && has_event(EVENT_CURSORMOVED)
            && (!ptr::eq(g_st().last_cursormoved_win, curwin_ptr())
                || !equalpos(g_st().last_cursormoved, curwin_mut().w_cursor))
        {
            apply_autocmds(
                EVENT_CURSORMOVED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            g_st().last_cursormoved_win = curwin_ptr();
            g_st().last_cursormoved = curwin_mut().w_cursor;
        }
    }
}

fn normal_check_text_changed(_s: &mut NormalState) {
    {
        if !g_st().finish_op
            && has_event(EVENT_TEXTCHANGED)
            && curbuf_mut().b_last_changedtick != buf_get_changedtick(curbuf_mut())
        {
            apply_autocmds(
                EVENT_TEXTCHANGED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            curbuf_mut().b_last_changedtick = buf_get_changedtick(curbuf_mut());
        }
    }
}

fn normal_check_safe_state(_s: &mut NormalState) {
    may_trigger_safestate(!op_pending() && g_st().restart_edit == 0);
}

fn normal_check_folds(_s: &mut NormalState) {
    {
        fold_adjust_visual();

        if has_any_folding(curwin_mut()) != 0 && !char_avail() {
            fold_check_close();

            if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_ALL) != 0 {
                fold_open_cursor();
            }
        }
    }
}

fn normal_redraw(_s: &mut NormalState) {
    {
        update_topline(curwin_mut());
        validate_cursor(curwin_mut());

        show_cursor_info_later(false);

        if g_st().must_redraw != 0 {
            update_screen();
        } else {
            redraw_statuslines();
            if g_st().redraw_cmdline || g_st().clear_cmdline || g_st().redraw_mode {
                showmode();
            }
        }

        curbuf_mut().b_last_used = os_time() as time_t;

        if let Some(p) = g_st().keep_msg.clone() {
            g_st().msg_hist_off = true;
            msg(p.as_ptr(), g_st().keep_msg_hl_id);
            g_st().msg_hist_off = false;
        }

        if g_st().need_fileinfo && !shortmess(K_SHM_FILEINFO) {
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

fn normal_check(s: &mut NormalState) -> i32 {
    {
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

            update_topline(curwin_mut());
            validate_cursor(curwin_mut());

            normal_check_cursor_moved(s);
            normal_check_text_changed(s);
            normal_check_window_scrolled(s);
            normal_check_safe_state(s);

            if curtab_mut().tp_diff_update != 0 || curtab_mut().tp_diff_invalid != 0 {
                ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
                curtab_mut().tp_diff_update = 0;
            }

            if g_st().diff_need_scrollbind {
                check_scrollbind(0, 0);
                g_st().diff_need_scrollbind = false;
            }

            normal_check_folds(s);
            normal_redraw(s);
            g_st().do_redraw = false;

            if has_time_fd() {
                time_msg(c"first screen update", None);
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

fn set_vcount_ca(cap: &mut Cmdarg, set_prevcount: &mut bool) {
    let mut count = i64::from(cap.count0);
    if cap.opcount != 0 {
        count = i64::from(cap.opcount) * if count == 0 { 1 } else { count };
    }
    set_vcount(count, if count == 0 { 1 } else { count }, *set_prevcount);
    *set_prevcount = false;
}

pub fn visualinfo() -> Visualinfo {
    {
        Visualinfo {
            vi_start: g_st().visual_global.start,
            vi_end: curwin_mut().w_cursor,
            vi_mode: g_st().visual_global.mode,
            vi_curswant: curwin_mut().w_curswant,
        }
    }
}

pub fn end_visual_mode() {
    {
        g_st().visual_global.select_exclu_adj = false;
        g_st().visual_global.active = false;
        setmouse();
        g_st().mouse_dragging = 0;

        curbuf_mut().b_visual = visualinfo();
        curbuf_mut().b_visual_mode_eval = g_st().visual_global.mode;
        if !virtual_active(curwin_mut()) {
            curwin_mut().w_cursor.coladd = 0;
        }

        may_clear_cmdline();

        adjust_cursor_eol();
        may_trigger_modechanged();
    }
}

pub fn reset_visual_and_resel() {
    if g_st().visual_global.active {
        end_visual_mode();
        redraw_curbuf_later(UPD_INVERTED);
    }
    g_st().visual_global.reselect = 0;
}

pub fn reset_visual() {
    if g_st().visual_global.active {
        end_visual_mode();
        redraw_curbuf_later(UPD_INVERTED);
        g_st().visual_global.reselect = 0;
    }
}

pub fn restore_visual_mode() {
    {
        if norm_st().visual_mode_orig != NUL as i32 {
            curbuf_mut().b_visual.vi_mode = norm_st().visual_mode_orig;
            norm_st().visual_mode_orig = NUL as i32;
        }
    }
}

fn find_is_eval_item(
    ptr: impl crate::strings::IntoCCharPtr,
    colp: &mut i32,
    bnp: &mut i32,
    dir: i32,
) -> bool {
    let ptr = ptr.into_c_char_ptr();
    unsafe {
        if (crate::memory::cstr_first_byte(ptr) == b']' && dir == BACKWARD)
            || (crate::memory::cstr_first_byte(ptr) == b'[' && dir == FORWARD)
        {
            *bnp += 1;
        }
        if *bnp > 0 {
            if (crate::memory::cstr_first_byte(ptr) == b'[' && dir == BACKWARD)
                || (crate::memory::cstr_first_byte(ptr) == b']' && dir == FORWARD)
            {
                *bnp -= 1;
            }
            return true;
        }

        if crate::memory::cstr_first_byte(ptr) == b'.' {
            return true;
        }

        let gt_off: isize = if dir == BACKWARD { 0 } else { 1 };
        let dash_off: isize = if dir == BACKWARD { -1 } else { 0 };
        if *ptr.wrapping_offset(gt_off) == b'>' as c_char
            && *ptr.wrapping_offset(dash_off) == b'-' as c_char
        {
            *colp += dir;
            return true;
        }
        false
    }
}

pub fn find_ident_under_cursor(
    text: *mut *mut c_char,
    find_type: i32,
    offset: Option<&mut i32>,
) -> usize {
    {
        let mut textcol: i32 = 0;
        let len = find_ident_at_pos(
            curwin_mut(),
            curwin_mut().w_cursor.lnum,
            curwin_mut().w_cursor.col,
            text,
            if offset.is_some() {
                &raw mut textcol
            } else {
                ptr::null_mut()
            },
            find_type,
        );
        if let Some(offset) = offset {
            *offset = curwin_mut().w_cursor.col - textcol;
        }
        len
    }
}

pub fn find_ident_at_pos(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    mut startcol: Colnr,
    text: *mut *mut c_char,
    textcol: *mut i32,
    find_type: i32,
) -> usize {
    unsafe {
        let mut col: i32 = 0;
        let mut this_class: i32 = 0;
        let mut prev_class: i32;
        let mut prevcol: i32;
        let mut bn: i32;

        let mut ptr = ml_get_buf(wp.buf_mut(), lnum);
        let mut i = if (find_type & FIND_IDENT) != 0 { 0 } else { 1 };
        while i < 2 {
            col = startcol;
            while crate::memory::cstr_byte_at(ptr, col as usize) != 0 {
                if (find_type & FIND_EVAL) != 0
                    && crate::memory::cstr_byte_at(ptr, col as usize) == b']'
                {
                    break;
                }
                this_class = mb_get_class(ptr.wrapping_add(col as usize));
                if this_class != 0 && (i == 1 || this_class != 1) {
                    break;
                }
                col += utfc_ptr2len(ptr.wrapping_add(col as usize));
            }

            bn = i32::from(crate::memory::cstr_byte_at(ptr, col as usize) == b']');

            if (find_type & FIND_EVAL) != 0
                && crate::memory::cstr_byte_at(ptr, col as usize) == b']'
            {
                this_class = mb_get_class(c"a");
            } else {
                this_class = mb_get_class(ptr.wrapping_add(col as usize));
            }
            while col > 0 && this_class != 0 {
                prevcol = col - 1 - utf_head_off(ptr, ptr.wrapping_add((col - 1) as usize));
                prev_class = mb_get_class(ptr.wrapping_add(prevcol as usize));
                if this_class != prev_class
                    && (i == 0 || prev_class == 0 || (find_type & FIND_IDENT) != 0)
                    && ((find_type & FIND_EVAL) == 0
                        || prevcol == 0
                        || !find_is_eval_item(
                            ptr.wrapping_add(prevcol as usize),
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

        if crate::memory::cstr_byte_at(ptr, col as usize) == 0 || (i == 0 && this_class != 2) {
            if (find_type & FIND_STRING) != 0 {
                emsg(tr(c"E348: No string under cursor"));
            } else {
                emsg(tr(E_NOIDENT));
            }
            return 0;
        }
        ptr = ptr.wrapping_add(col as usize);
        *text = ptr;
        if !textcol.is_null() {
            *textcol = col;
        }

        bn = 0;
        startcol -= col;
        col = 0;
        this_class = mb_get_class(ptr);
        while crate::memory::cstr_byte_at(ptr, col as usize) != 0
            && ((if i == 0 {
                mb_get_class(ptr.wrapping_add(col as usize)) == this_class
            } else {
                mb_get_class(ptr.wrapping_add(col as usize)) != 0
            }) || ((find_type & FIND_EVAL) != 0
                && col <= startcol
                && find_is_eval_item(ptr.wrapping_add(col as usize), &mut col, &mut bn, FORWARD)))
        {
            col += utfc_ptr2len(ptr.wrapping_add(col as usize));
        }

        assert!(col >= 0);
        col as usize
    }
}

fn prep_redo_cmd(cap: &mut Cmdarg) {
    let composing = cap.nchar_len > 0;
    prep_redo(true, composing, atom_cmd_spec(cap));
    if composing {
        redo_append_str(cap.nchar_composing.as_ptr(), -1);
    }
}

fn checkclearop(oap: &mut Oparg) -> bool {
    if oap.op_type == OP_NOP {
        return false;
    }
    clearopbeep(&mut *oap);
    true
}

fn checkclearopq(oap: &mut Oparg) -> bool {
    if oap.op_type == OP_NOP && !g_st().visual_global.active {
        return false;
    }
    clearopbeep(&mut *oap);
    true
}

pub fn clearop(oap: &mut Oparg) {
    oap.op_type = OP_NOP;
    oap.regname = 0;
    oap.motion_force = NUL as i32;
    oap.use_reg_one = false;
    oap.restore_cursor = false;
    g_st().motion_force = NUL as i32;
}

pub fn clearopbeep(oap: &mut Oparg) {
    if oap.op_type == OP_MCURSOR && oap.motion_type != MT_UNKNOWN {
        return;
    }
    clearop(&mut *oap);
    beep_flush();
}

fn unshift_special(cap: &mut Cmdarg) {
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

        if g_st().visual_global.active && !char_avail() {
            let cursor_bot = lt(g_st().visual_global.start, curwin_mut().w_cursor);
            let mut leftcol: Colnr = 0;
            let mut rightcol: Colnr = 0;
            let mut top: Linenr;
            let mut bot: Linenr;

            if cursor_bot {
                top = g_st().visual_global.start.lnum;
                bot = curwin_mut().w_cursor.lnum;
            } else {
                top = curwin_mut().w_cursor.lnum;
                bot = g_st().visual_global.start.lnum;
            }
            has_folding(curwin_ptr(), top, Some(&mut top), ptr::null_mut());
            has_folding(curwin_ptr(), bot, None, &raw mut bot);
            let lines = bot - top + 1;

            if g_st().visual_global.mode == CTRL_V_CONST {
                let saved_sbr = opt_vars().p_sbr;
                let saved_w_sbr = curwin_mut().w_onebuf_opt.wo_sbr;

                opt_vars().p_sbr = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
                curwin_mut().w_onebuf_opt.wo_sbr = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
                getvcols(
                    curwin_ptr(),
                    &mut curwin_mut().w_cursor,
                    &mut g_st().visual_global.start,
                    &mut leftcol,
                    &mut rightcol,
                    GETVCOL_END_EXCL_LBR,
                );
                opt_vars().p_sbr = saved_sbr;
                curwin_mut().w_onebuf_opt.wo_sbr = saved_w_sbr;
                vim_snprintf!(
                    sc_buf,
                    SHOWCMD_BUFLEN,
                    c"%lldx%lld",
                    lines as i64,
                    (i64::from(rightcol) - i64::from(leftcol) + 1) as i64,
                );
            } else if g_st().visual_global.mode == b'V' as i32
                || g_st().visual_global.start.lnum != curwin_mut().w_cursor.lnum
            {
                vim_snprintf!(sc_buf, SHOWCMD_BUFLEN, c"%lld", lines as i64,);
            } else {
                let mut s: *mut c_char;
                let e: *mut c_char;
                let mut bytes: i32 = 0;
                let mut chars: i32 = 0;

                if cursor_bot {
                    s = ml_get_pos(&g_st().visual_global.start);
                    e = get_cursor_pos_ptr();
                } else {
                    s = get_cursor_pos_ptr();
                    e = ml_get_pos(&g_st().visual_global.start);
                }
                while if crate::memory::cstr_first_byte(opt_vars().p_sel) != b'e' {
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
                    s = s.wrapping_add(l as usize);
                }
                if bytes == chars {
                    vim_snprintf!(sc_buf, SHOWCMD_BUFLEN, c"%d", chars);
                } else {
                    vim_snprintf!(sc_buf, SHOWCMD_BUFLEN, c"%d-%d", chars, bytes);
                }
            }
            let limit = if ui_has(K_UIMESSAGES) {
                SHOWCMD_BUFLEN - 1
            } else {
                SHOWCMD_COLS
            };
            *sc_buf.wrapping_add(limit) = NUL;
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

pub fn add_to_showcmd(c: i32) -> bool {
    unsafe {
        const IGNORE: &[i32] = &[
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

        if opt_vars().p_sc == 0
            || g_st().msg_silent != 0
            || g_st().ex_normal_busy != 0
            || g_st().key_stuffed != 0
        {
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
            if crate::memory::cstr_first_byte(tc) == b' ' {
                crate::memory::cstr_cpy(tc, c"<20>");
            }
            tc
        } else {
            let len = utf_char2bytes(c, mbyte_buf.as_mut_ptr()) as usize;
            mbyte_buf[len] = NUL;
            mbyte_buf.as_mut_ptr()
        };

        let old_len = crate::memory::cstr_len(sc_buf);
        let extra_len = crate::memory::cstr_len(p);
        let limit = if ui_has(K_UIMESSAGES) {
            SHOWCMD_BUFLEN - 1
        } else {
            SHOWCMD_COLS
        };
        if old_len + extra_len > limit {
            let overflow = old_len + extra_len - limit;
            ptr::copy(
                sc_buf.wrapping_add(overflow),
                sc_buf,
                old_len - overflow + 1,
            );
        }
        crate::memory::cstr_cat(sc_buf, p);

        if char_avail() {
            return false;
        }

        display_showcmd();
        true
    }
}

pub fn add_to_showcmd_c(c: i32) {
    add_to_showcmd(c);
    setcursor();
}

fn del_from_showcmd(mut len: i32) {
    unsafe {
        if opt_vars().p_sc == 0 {
            return;
        }
        let sc_buf = (&raw mut g_st().showcmd_buf).cast::<c_char>();
        let old_len = crate::memory::cstr_len(sc_buf) as i32;
        len = len.min(old_len);
        *sc_buf.wrapping_add((old_len - len) as usize) = NUL;

        if !char_avail() {
            display_showcmd();
        }
    }
}

pub fn push_showcmd() {
    {
        if opt_vars().p_sc != 0 {
            crate::memory::cstr_cpy(
                (&raw mut norm_st().old_showcmd_buf).cast::<c_char>(),
                (&raw const g_st().showcmd_buf).cast::<c_char>(),
            );
        }
    }
}

pub fn pop_showcmd() {
    {
        if opt_vars().p_sc == 0 {
            return;
        }
        crate::memory::cstr_cpy(
            (&raw mut g_st().showcmd_buf).cast::<c_char>(),
            (&raw const norm_st().old_showcmd_buf).cast::<c_char>(),
        );
        display_showcmd();
    }
}

pub fn showcmd_update_clear_state() {
    norm_st().showcmd_is_clear = g_st().showcmd_buf[0] == NUL;
}

pub fn display_showcmd() {
    {
        if mc_replaying() {
            return;
        }

        showcmd_update_clear_state();

        if crate::memory::cstr_first_byte(opt_vars().p_sloc) == b's' {
            if norm_st().showcmd_is_clear && g_st().vgetc_busy == 0 {
                curwin_mut().w_redr_status = true;
            } else {
                win_redr_status(curwin_mut());
                setcursor();
            }
            return;
        }
        if crate::memory::cstr_first_byte(opt_vars().p_sloc) == b't' {
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

        if ui_has(K_UIMESSAGES) {
            let mut content_items: [Object; 1] = [NIL_OBJ];
            let mut chunk_items: [Object; 3] = [NIL_OBJ; 3];
            let mut ext_buf: [c_char; 16 + SHOWCMD_BUFLEN] = [0; 16 + SHOWCMD_BUFLEN];
            crate::memory::cstr_cpy(ext_buf.as_mut_ptr(), mc_buf.as_ptr());
            xstrlcpy(
                ext_buf.as_mut_ptr().wrapping_add(mc_len),
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
        let showcmd_row = g_st().rows_global - 1;
        grid_line_start(&raw mut g_st().msg_grid_adj, showcmd_row);

        let mut len = if mc_len > 0 {
            grid_line_puts(g_st().sc_col, mc_buf.as_ptr(), -1, 0)
        } else {
            0
        };
        if !norm_st().showcmd_is_clear {
            len += grid_line_puts(g_st().sc_col + len, sc_buf, -1, hl_attr(HLF_MSG as i32));
        }
        len = len.min(SHOWCMD_COLS as i32);

        grid_line_puts(
            g_st().sc_col + len,
            c"          ".as_ptr().wrapping_add(len as usize),
            -1,
            hl_attr(HLF_MSG as i32),
        );

        grid_line_flush();
    }
}

pub fn get_vtopline(wp: &mut WinInfoInfo) -> i32 {
    plines_m_win_fill(wp, 1, wp.w_topline) - wp.w_topfill
}

pub fn do_check_scrollbind(check: bool) {
    {
        let vtopline = get_vtopline(curwin_mut());

        if check && curwin_mut().w_onebuf_opt.wo_scb != 0 {
            if g_st().did_syncbind {
                g_st().did_syncbind = false;
            } else if ptr::eq(curwin_ptr(), norm_st().do_check_scrollbind_old_curwin) {
                if (ptr::eq(curwin_mut().w_buffer, norm_st().do_check_scrollbind_old_buf)
                    || curwin_mut().w_onebuf_opt.wo_diff != 0)
                    && (vtopline != norm_st().do_check_scrollbind_old_vtopline
                        || curwin_mut().w_leftcol != norm_st().do_check_scrollbind_old_leftcol)
                {
                    check_scrollbind(
                        vtopline - norm_st().do_check_scrollbind_old_vtopline,
                        curwin_mut().w_leftcol - norm_st().do_check_scrollbind_old_leftcol,
                    );
                }
            } else if crate::strings::vim_haschr(opt_vars().p_sbo, b'j' as i32) {
                check_scrollbind(vtopline - curwin_mut().w_scbind_pos, 0);
            }
            curwin_mut().w_scbind_pos = vtopline;
        }

        norm_st().do_check_scrollbind_old_curwin = curwin_ptr();
        norm_st().do_check_scrollbind_old_vtopline = vtopline;
        norm_st().do_check_scrollbind_old_buf = curwin_mut().w_buffer;
        norm_st().do_check_scrollbind_old_leftcol = curwin_mut().w_leftcol;
    }
}

pub fn check_scrollbind(vtopline_diff: Linenr, leftcol_diff: i32) {
    unsafe {
        let old_curwin = curwin_ptr();
        let old_curbuf = g_st().curbuf;
        let old_visual_select = g_st().visual_global.select;
        let old_visual_active = g_st().visual_global.active;
        let tgt_leftcol = curwin_mut().w_leftcol;

        let want_ver = (*old_curwin).w_onebuf_opt.wo_diff != 0
            || (crate::strings::vim_haschr(opt_vars().p_sbo, b'v' as i32) && vtopline_diff != 0);
        let want_hor = crate::strings::vim_haschr(opt_vars().p_sbo, b'h' as i32)
            && (leftcol_diff != 0 || vtopline_diff != 0);

        g_st().visual_global.select = false;
        g_st().visual_global.active = false;
        for wp in for_all_windows_ptr() {
            set_curwin(wp);
            g_st().curbuf = curwin_mut().w_buffer;
            if !ptr::eq(curwin_ptr(), old_curwin) && curwin_mut().w_onebuf_opt.wo_scb != 0 {
                if want_ver {
                    if (*old_curwin).w_onebuf_opt.wo_diff != 0
                        && curwin_mut().w_onebuf_opt.wo_diff != 0
                    {
                        diff_set_topline(&mut *old_curwin, curwin_mut());
                    } else {
                        curwin_mut().w_scbind_pos += vtopline_diff;
                        let curr_vtopline = get_vtopline(curwin_mut());
                        let max_vtopline = curr_vtopline
                            + curwin_mut().w_topfill
                            + plines_m_win_fill(
                                crate::globals::curwin_mut(),
                                curwin_mut().w_topline + 1,
                                curbuf_mut().b_ml.ml_line_count,
                            );
                        let new_vtopline = curwin_mut().w_scbind_pos.min(max_vtopline).max(1);
                        let y = new_vtopline - curr_vtopline;
                        if y > 0 {
                            scrollup(curwin_mut(), y, false);
                        } else {
                            scrolldown(curwin_mut(), -y, 0);
                        }
                    }

                    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                    cursor_correct(curwin_mut());
                    curwin_mut().w_redr_status = true;
                }

                if want_hor {
                    set_leftcol(tgt_leftcol);
                }
            }
        }

        g_st().visual_global.select = old_visual_select;
        g_st().visual_global.active = old_visual_active;
        set_curwin(old_curwin);
        g_st().curbuf = old_curbuf;
    }
}

fn nv_ignore(cap: &mut Cmdarg) {
    cap.retval |= CA_COMMAND_BUSY;
}

fn nv_nop(_cap: &mut Cmdarg) {}

fn nv_error(cap: &mut Cmdarg) {
    {
        clearopbeep(cap.oap_mut());
    }
}

fn nv_help(cap: &mut Cmdarg) {
    {
        if !checkclearopq(cap.oap_mut()) {
            ex_help(&mut crate::ex_docmd::Exarg::new());
        }
    }
}

fn nv_addsub(cap: &mut Cmdarg) {
    {
        if bt_prompt(Some(crate::globals::curbuf_mut())) && !prompt_curpos_editable() {
            clearopbeep(cap.oap_mut());
        } else if !g_st().visual_global.active && cap.oap_mut().op_type == OP_NOP {
            prep_redo_cmd(cap);
            cap.oap_mut().op_type = if cap.cmdchar == CTRL_A {
                OP_NR_ADD
            } else {
                OP_NR_SUB
            };
            op_addsub(cap.oap_mut(), cap.count1, cap.arg != 0);
            cap.oap_mut().op_type = OP_NOP;
        } else if g_st().visual_global.active {
            nv_operator(cap);
        } else {
            clearop(cap.oap_mut());
        }
    }
}

fn nv_page(cap: &mut Cmdarg) {
    {
        if checkclearop(cap.oap_mut()) {
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

fn nv_gd(oap: &mut Oparg, nchar: i32, thisblock: i32) {
    let mut ptr: *mut c_char = ptr::null_mut();
    let len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
    if len == 0 || !find_decl(ptr, len, nchar == b'd' as i32, thisblock != 0, SEARCH_START) {
        clearopbeep(&mut *oap);
        return;
    }

    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_SEARCH) != 0
        && g_st().key_typed
        && oap.op_type == OP_NOP
    {
        fold_open_cursor();
    }
    if messaging() && g_st().msg_silent == 0 && !shortmess(K_SHM_SEARCHCOUNT) {
        g_st().clear_cmdline = true;
    }
}

fn is_ident(line: impl crate::strings::IntoCCharPtr, offset: i32) -> bool {
    let line = line.into_c_char_ptr();
    {
        let mut incomment = false;
        let mut instring: i32 = 0;
        let mut prev: i32 = 0;

        let mut i: i32 = 0;
        while i < offset && crate::memory::cstr_byte_at(line, i as usize) != 0 {
            let cur = i32::from(crate::memory::cstr_byte_at(line, i as usize));
            if instring != 0 {
                if prev != b'\\' as i32 && cur == instring {
                    instring = 0;
                }
            } else if (cur == b'"' as i32 || cur == b'\'' as i32) && !incomment {
                instring = cur;
            } else if incomment {
                if prev == b'*' as i32 && cur == b'/' as i32 {
                    incomment = false;
                }
            } else if prev == b'/' as i32 && cur == b'*' as i32 {
                incomment = true;
            } else if prev == b'/' as i32 && cur == b'/' as i32 {
                return false;
            }

            prev = cur;
            i += 1;
        }

        !incomment && instring == 0
    }
}

pub fn find_decl(
    ptr: impl crate::strings::IntoCCharPtr,
    len: usize,
    locally: bool,
    thisblock: bool,
    flags_arg: i32,
) -> bool {
    let ptr = ptr.into_c_char_ptr();
    unsafe {
        let par_pos: CursorPos;
        let mut found_pos: CursorPos = CursorPos {
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

        assert!(patsize <= i32::MAX as usize);
        let patlen = vim_snprintf!(
            pat,
            patsize,
            if vim_iswordp(ptr) {
                c"\\V\\<%.*s\\>".as_ptr()
            } else {
                c"\\V%.*s".as_ptr()
            },
            len as i32,
            ptr,
        ) as usize;

        let old_pos = curwin_mut().w_cursor;
        let save_p_ws = opt_vars().p_ws;
        let save_p_scs = opt_vars().p_scs;
        opt_vars().p_ws = 0;
        opt_vars().p_scs = 0;

        if !locally || !findpar(&mut incll, BACKWARD, 1, b'{' as i32, false) {
            setpcmark();
            curwin_mut().w_cursor.lnum = 1;
            par_pos = curwin_mut().w_cursor;
        } else {
            par_pos = curwin_mut().w_cursor;
            while curwin_mut().w_cursor.lnum > 1 && *skipwhite(get_cursor_line_ptr()) != NUL {
                curwin_mut().w_cursor.lnum -= 1;
            }
        }
        curwin_mut().w_cursor.col = 0;

        clearpos(&mut found_pos);
        loop {
            t = searchit(
                curwin_ptr(),
                curbuf_mut(),
                &mut curwin_mut().w_cursor,
                None,
                FORWARD,
                pat,
                patlen,
                1,
                searchflags,
                RE_LAST,
                false,
                None,
            ) != 0;
            if curwin_mut().w_cursor.lnum >= old_pos.lnum {
                t = false;
            }

            if thisblock && t {
                let maxtravel = i64::from(old_pos.lnum - curwin_mut().w_cursor.lnum + 1);
                let pos = findmatchlimit(None, b'}' as i32, FM_FORWARD, maxtravel);

                if !pos.is_null() && (*pos).lnum < old_pos.lnum {
                    curwin_mut().w_cursor = *pos;
                    continue;
                }
            }

            if !t {
                if found_pos.lnum != 0 {
                    curwin_mut().w_cursor = found_pos;
                    t = true;
                }
                break;
            }
            if get_leader_len(get_cursor_line_ptr(), ptr::null_mut(), false, true) > 0 {
                curwin_mut().w_cursor.lnum += 1;
                curwin_mut().w_cursor.col = 0;
                continue;
            }
            let valid = is_ident(get_cursor_line_ptr(), curwin_mut().w_cursor.col);

            if !valid && found_pos.lnum != 0 {
                curwin_mut().w_cursor = found_pos;
                break;
            }
            if valid && !locally {
                break;
            }
            if valid && curwin_mut().w_cursor.lnum >= par_pos.lnum {
                if found_pos.lnum != 0 {
                    curwin_mut().w_cursor = found_pos;
                }
                break;
            }

            if !valid {
                clearpos(&mut found_pos);
            } else {
                found_pos = curwin_mut().w_cursor;
            }
            searchflags &= !SEARCH_START;
        }

        if !t {
            retval = false;
            curwin_mut().w_cursor = old_pos;
        } else {
            curwin_mut().w_set_curswant = true;
            reset_search_dir();
        }

        drop(pat_vec);
        opt_vars().p_ws = save_p_ws;
        opt_vars().p_scs = save_p_scs;

        retval
    }
}

pub fn nv_screengo(oap: &mut Oparg, dir: i32, mut dist: i32, skip_conceal: bool) -> bool {
    unsafe {
        let mut linelen = linetabsize(curwin_mut(), curwin_mut().w_cursor.lnum);
        let mut retval = true;
        let mut atend = false;

        oap.motion_type = MT_CHAR_WISE;
        oap.inclusive = curwin_mut().w_curswant == MAXCOL;

        let col_off1 = win_col_off(curwin_ptr());
        let col_off2 = col_off1 - win_col_off2(curwin_mut());
        let width1 = curwin_mut().w_view_width - col_off1;
        let mut width2 = curwin_mut().w_view_width - col_off2;

        if width2 == 0 {
            width2 = 1;
        }

        if curwin_mut().w_view_width != 0 {
            let mut n: i32;
            if curwin_mut().w_curswant == MAXCOL {
                atend = true;
                validate_virtcol(curwin_mut());
                if width1 <= 0 {
                    curwin_mut().w_curswant = 0;
                } else {
                    curwin_mut().w_curswant = width1 - 1;
                    if curwin_mut().w_virtcol > curwin_mut().w_curswant {
                        curwin_mut().w_curswant +=
                            ((curwin_mut().w_virtcol - curwin_mut().w_curswant - 1) / width2 + 1)
                                * width2;
                    }
                }
            } else {
                if linelen > width1 {
                    n = ((linelen - width1 - 1) / width2 + 1) * width2 + width1;
                } else {
                    n = width1;
                }
                curwin_mut().w_curswant = curwin_mut().w_curswant.min(n - 1);
            }

            while dist > 0 {
                dist -= 1;
                if dir == BACKWARD {
                    if curwin_mut().w_curswant >= width1
                        && !has_folding(
                            curwin_ptr(),
                            curwin_mut().w_cursor.lnum,
                            None,
                            ptr::null_mut(),
                        )
                    {
                        curwin_mut().w_curswant -= width2;
                    } else {
                        if curwin_mut().w_cursor.lnum <= 1 {
                            retval = false;
                            break;
                        }
                        cursor_up_inner(curwin_mut(), 1, skip_conceal);

                        linelen = linetabsize(curwin_mut(), curwin_mut().w_cursor.lnum);
                        if linelen > width1 {
                            let w = (((linelen - width1 - 1) / width2) + 1) * width2;
                            assert!(w <= 0 || curwin_mut().w_curswant <= i32::MAX - w);
                            curwin_mut().w_curswant += w;
                        }
                    }
                } else {
                    if linelen > width1 {
                        n = ((linelen - width1 - 1) / width2 + 1) * width2 + width1;
                    } else {
                        n = width1;
                    }
                    if curwin_mut().w_curswant + width2 < n
                        && !has_folding(
                            curwin_ptr(),
                            curwin_mut().w_cursor.lnum,
                            None,
                            ptr::null_mut(),
                        )
                    {
                        curwin_mut().w_curswant += width2;
                    } else {
                        if curwin_mut().w_cursor.lnum >= curwin_mut().buf_mut().b_ml.ml_line_count {
                            retval = false;
                            break;
                        }
                        cursor_down_inner(curwin_mut(), 1, skip_conceal);
                        curwin_mut().w_curswant %= width2;

                        if curwin_mut().w_curswant >= width1 {
                            curwin_mut().w_curswant -= width2;
                        }
                        linelen = linetabsize(curwin_mut(), curwin_mut().w_cursor.lnum);
                    }
                }
            }
        }

        if virtual_active(curwin_mut()) && atend {
            coladvance(curwin_mut(), MAXCOL);
        } else {
            coladvance(curwin_mut(), curwin_mut().w_curswant);
        }

        if curwin_mut().w_cursor.col > 0 && curwin_mut().w_onebuf_opt.wo_wrap != 0 {
            validate_virtcol(curwin_mut());
            let mut virtcol = curwin_mut().w_virtcol;
            if virtcol > width1 && *get_showbreak_value(curwin_mut()) != NUL {
                virtcol -= vim_strsize(get_showbreak_value(curwin_mut()));
            }

            let c = utf_ptr2char(get_cursor_pos_ptr());
            if dir == FORWARD
                && virtcol < curwin_mut().w_curswant
                && curwin_mut().w_curswant <= width1
                && !vim_isprintc(c)
                && c > 255
            {
                oneright();
            }

            if virtcol > curwin_mut().w_curswant
                && (if curwin_mut().w_curswant < width1 {
                    curwin_mut().w_curswant > width1 / 2
                } else {
                    (curwin_mut().w_curswant - width1) % width2 > width2 / 2
                })
            {
                curwin_mut().w_cursor.col -= 1;
            }
        }

        if atend {
            curwin_mut().w_curswant = MAXCOL;
        }
        adjust_skipcol();

        retval
    }
}

pub fn nv_scroll_line(cap: &mut Cmdarg) {
    {
        if !checkclearop(cap.oap_mut()) {
            scroll_redraw(cap.arg, cap.count1);
        }
    }
}

fn nv_z_get_count(cap: &mut Cmdarg, nchar_arg: &mut i32) -> bool {
    {
        let mut nchar = *nchar_arg;

        if checkclearop(cap.oap_mut()) {
            return false;
        }
        let mut n = nchar - b'0' as i32;

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
                if !vim_append_digit_int(&mut n, nchar - b'0' as i32) {
                    clearopbeep(cap.oap_mut());
                    break;
                }
            } else if nchar == CAR {
                win_setheight(n);
                break;
            } else if nchar == b'l' as i32
                || nchar == b'h' as i32
                || nchar == K_LEFT
                || nchar == K_RIGHT
            {
                cap.count1 = if n != 0 { n * cap.count1 } else { cap.count1 };
                *nchar_arg = nchar;
                return true;
            } else {
                clearopbeep(cap.oap_mut());
                break;
            }
        }
        cap.oap_mut().op_type = OP_NOP;
        false
    }
}

fn nv_zg_zw(cap: &mut Cmdarg, mut nchar: i32) -> bool {
    {
        let mut undo = false;

        if nchar == b'u' as i32 {
            g_st().no_mapping += 1;
            g_st().allow_keys += 1;
            nchar = plain_vgetc();
            langmap_adjust(&mut nchar, true);
            g_st().no_mapping -= 1;
            g_st().allow_keys -= 1;
            add_to_showcmd(nchar);

            if !crate::strings::vim_haschr(c"gGwW", nchar) {
                clearopbeep(cap.oap_mut());
                return true;
            }
            undo = true;
        }

        if checkclearop(cap.oap_mut()) {
            return true;
        }
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        if g_st().visual_global.active && !get_visual_text(Some(cap), &mut ptr, &mut len) {
            return false;
        }
        if ptr.is_null() {
            let pos = curwin_mut().w_cursor;

            g_st().emsg_off += 1;
            len = spell_move_to(curwin_mut(), FORWARD, SMT_ALL, true, ptr::null_mut());
            g_st().emsg_off -= 1;
            if len != 0 && curwin_mut().w_cursor.col <= pos.col {
                ptr = ml_get_pos(&curwin_mut().w_cursor);
            }
            curwin_mut().w_cursor = pos;
        }

        if ptr.is_null() {
            len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
            if len == 0 {
                return false;
            }
        }
        assert!(len <= i32::MAX as usize);
        spell_add_word(
            ptr,
            len as i32,
            if nchar == b'w' as i32 || nchar == b'W' as i32 {
                SPELL_ADD_BAD
            } else {
                SPELL_ADD_GOOD
            },
            if nchar == b'G' as i32 || nchar == b'W' as i32 {
                0
            } else {
                cap.count1
            },
            undo,
        );

        true
    }
}

fn nv_zet(cap: &mut Cmdarg) {
    {
        let mut col: Colnr = 0;
        let mut nchar = cap.nchar;
        let mut foldlevel: OptInt = -1;
        let mut foldenable: i32 = K_NONE;

        let siso = get_sidescrolloff_value(curwin_mut());

        if ascii_isdigit(nchar) && !nv_z_get_count(cap, &mut nchar) {
            return;
        }

        if cap.nchar != b'f' as i32
            && cap.nchar != b'F' as i32
            && !(g_st().visual_global.active && crate::strings::vim_haschr(c"dcCoO", cap.nchar))
            && cap.nchar != b'j' as i32
            && cap.nchar != b'k' as i32
            && checkclearop(cap.oap_mut())
        {
            return;
        }

        if crate::strings::vim_haschr(c"+\r\nt.z^-b", nchar)
            && cap.count0 != 0
            && cap.count0 != curwin_mut().w_cursor.lnum
        {
            setpcmark();
            if cap.count0 > curbuf_mut().b_ml.ml_line_count {
                curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
            } else {
                curwin_mut().w_cursor.lnum = cap.count0;
            }
            check_cursor_col(curwin_mut());
        }

        match nchar {
            c if c == b'+' as i32 || c == NL || c == CAR || c == K_KENTER || c == b't' as i32 => {
                if nchar == b'+' as i32 && cap.count0 == 0 {
                    validate_botline_win(curwin_mut());
                    curwin_mut().w_cursor.lnum =
                        curwin_mut().w_botline.min(curbuf_mut().b_ml.ml_line_count);
                }
                if nchar != b't' as i32 {
                    beginline(BL_WHITE | BL_FIX);
                }
                scroll_cursor_top(curwin_mut(), 0, 1);
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                set_fraction(curwin_mut());
            }
            c if c == b'.' as i32 || c == b'z' as i32 => {
                if nchar == b'.' as i32 {
                    beginline(BL_WHITE | BL_FIX);
                }
                scroll_cursor_halfway(curwin_mut(), true, false);
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                set_fraction(curwin_mut());
            }
            c if c == b'^' as i32 || c == b'-' as i32 || c == b'b' as i32 => {
                if nchar == b'^' as i32 {
                    if cap.count0 != 0 {
                        scroll_cursor_bot(curwin_mut(), 0, true);
                        curwin_mut().w_cursor.lnum = curwin_mut().w_topline;
                    } else if curwin_mut().w_topline == 1 {
                        curwin_mut().w_cursor.lnum = 1;
                    } else {
                        curwin_mut().w_cursor.lnum = curwin_mut().w_topline - 1;
                    }
                }
                if nchar != b'b' as i32 {
                    beginline(BL_WHITE | BL_FIX);
                }
                scroll_cursor_bot(curwin_mut(), 0, true);
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                set_fraction(curwin_mut());
            }
            c if c == b'H' as i32 || c == b'h' as i32 || c == K_LEFT => {
                if nchar == b'H' as i32 {
                    cap.count1 *= curwin_mut().w_view_width / 2;
                }
                if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
                    set_leftcol(if cap.count1 > curwin_mut().w_leftcol {
                        0
                    } else {
                        curwin_mut().w_leftcol - cap.count1
                    });
                }
            }
            c if c == b'L' as i32 || c == b'l' as i32 || c == K_RIGHT => {
                if nchar == b'L' as i32 {
                    cap.count1 *= curwin_mut().w_view_width / 2;
                }
                if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
                    set_leftcol(curwin_mut().w_leftcol + cap.count1);
                }
            }
            c if c == b's' as i32 => {
                if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
                    if has_folding(
                        curwin_ptr(),
                        curwin_mut().w_cursor.lnum,
                        None,
                        ptr::null_mut(),
                    ) {
                        col = 0;
                    } else {
                        getvcol(
                            curwin_mut(),
                            &mut curwin_mut().w_cursor,
                            Some(&mut col),
                            None,
                            None,
                            0,
                        );
                    }
                    if i64::from(col) > siso {
                        col -= siso as Colnr;
                    } else {
                        col = 0;
                    }
                    if curwin_mut().w_leftcol != col {
                        curwin_mut().w_leftcol = col;
                        redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
                    }
                }
            }
            c if c == b'e' as i32 => {
                if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
                    if has_folding(
                        curwin_ptr(),
                        curwin_mut().w_cursor.lnum,
                        None,
                        ptr::null_mut(),
                    ) {
                        col = 0;
                    } else {
                        getvcol(
                            curwin_mut(),
                            &mut curwin_mut().w_cursor,
                            None,
                            None,
                            Some(&mut col),
                            0,
                        );
                    }
                    let n = i64::from(curwin_mut().w_view_width - win_col_off(curwin_ptr()));
                    if i64::from(col) + siso < n {
                        col = 0;
                    } else if siso - n < i64::from(i32::MAX - col) {
                        col = (i64::from(col) + siso - n + 1) as Colnr;
                    } else {
                        col = i32::MAX;
                    }
                    if curwin_mut().w_leftcol != col {
                        curwin_mut().w_leftcol = col;
                        redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
                    }
                }
            }
            c if c == b'P' as i32 || c == b'p' as i32 => {
                nv_put(cap);
            }
            c if c == b'y' as i32 => {
                nv_operator(cap);
            }
            c if c == b'q' as i32 => {
                if g_st().reg_recording != 0 || g_st().reg_executing != 0 {
                    clearopbeep(cap.oap_mut());
                } else {
                    mc_zq_start(cap);
                    nv_operator(cap);
                }
            }
            c if c == b'F' as i32 || c == b'f' as i32 => {
                if fold_manual_allowed(true) != 0 {
                    cap.nchar = b'f' as i32;
                    nv_operator(cap);
                    foldenable = 1;
                } else {
                    clearopbeep(cap.oap_mut());
                }
            }
            c if c == b'd' as i32 || c == b'D' as i32 => {
                if fold_manual_allowed(false) != 0 {
                    if g_st().visual_global.active {
                        nv_operator(cap);
                    } else {
                        delete_fold(
                            curwin_mut(),
                            curwin_mut().w_cursor.lnum,
                            curwin_mut().w_cursor.lnum,
                            i32::from(nchar == b'D' as i32),
                            false,
                        );
                    }
                }
            }
            c if c == b'E' as i32 => {
                if foldmethod_is_manual(curwin_mut()) {
                    clear_folding(curwin_mut());
                    changed_window_setting(curwin_mut());
                } else if foldmethod_is_marker(curwin_mut()) {
                    delete_fold(curwin_mut(), 1, curbuf_mut().b_ml.ml_line_count, 1, false);
                } else {
                    emsg(tr(c"E352: Cannot erase folds with current 'foldmethod'"));
                }
            }
            c if c == b'n' as i32 => {
                foldenable = 0;
            }
            c if c == b'N' as i32 => {
                foldenable = 1;
            }
            c if c == b'i' as i32 => {
                foldenable = i32::from(curwin_mut().w_onebuf_opt.wo_fen == 0);
            }
            c if c == b'a' as i32 => {
                if has_folding(
                    curwin_ptr(),
                    curwin_mut().w_cursor.lnum,
                    None,
                    ptr::null_mut(),
                ) {
                    open_fold(curwin_mut().w_cursor, cap.count1);
                } else {
                    close_fold(curwin_mut().w_cursor, cap.count1);
                    foldenable = 1;
                }
            }
            c if c == b'A' as i32 => {
                if has_folding(
                    curwin_ptr(),
                    curwin_mut().w_cursor.lnum,
                    None,
                    ptr::null_mut(),
                ) {
                    open_fold_recurse(curwin_mut().w_cursor);
                } else {
                    close_fold_recurse(curwin_mut().w_cursor);
                    foldenable = 1;
                }
            }
            c if c == b'o' as i32 => {
                if g_st().visual_global.active {
                    nv_operator(cap);
                } else {
                    open_fold(curwin_mut().w_cursor, cap.count1);
                }
            }
            c if c == b'O' as i32 => {
                if g_st().visual_global.active {
                    nv_operator(cap);
                } else {
                    open_fold_recurse(curwin_mut().w_cursor);
                }
            }
            c if c == b'c' as i32 => {
                if g_st().visual_global.active {
                    nv_operator(cap);
                } else {
                    close_fold(curwin_mut().w_cursor, cap.count1);
                }
                foldenable = 1;
            }
            c if c == b'C' as i32 => {
                if g_st().visual_global.active {
                    nv_operator(cap);
                } else {
                    close_fold_recurse(curwin_mut().w_cursor);
                }
                foldenable = 1;
            }
            c if c == b'v' as i32 => {
                fold_open_cursor();
            }
            c if c == b'x' as i32 || c == b'X' as i32 => {
                foldenable = 1;
                curwin_mut().w_foldinvalid = true;
                foldlevel = curwin_mut().w_onebuf_opt.wo_fdl;
            }
            c if c == b'm' as i32 => {
                foldlevel = (curwin_mut().w_onebuf_opt.wo_fdl - i64::from(cap.count1)).max(0);
                foldenable = 1;
            }
            c if c == b'M' as i32 => {
                foldlevel = 0;
                foldenable = 1;
            }
            c if c == b'r' as i32 => {
                let depth = i64::from(get_deepest_nesting(curwin_mut()));
                let level = (curwin_mut().w_onebuf_opt.wo_fdl + i64::from(cap.count1)).min(depth);
                if level != curwin_mut().w_onebuf_opt.wo_fdl {
                    foldlevel = level;
                }
            }
            c if c == b'R' as i32 => {
                foldlevel = i64::from(get_deepest_nesting(curwin_mut()));
            }
            c if c == b'j' as i32 || c == b'k' as i32 => {
                if !fold_move_to(
                    true,
                    if nchar == b'j' as i32 {
                        FORWARD
                    } else {
                        BACKWARD
                    },
                    cap.count1,
                ) {
                    clearopbeep(cap.oap_mut());
                }
            }
            c if c == b'u' as i32
                || c == b'g' as i32
                || c == b'w' as i32
                || c == b'G' as i32
                || c == b'W' as i32 =>
            {
                if !nv_zg_zw(cap, nchar) {
                    return;
                }
            }
            c if c == b'=' as i32 => {
                if !checkclearop(cap.oap_mut()) {
                    spell_suggest(cap.count0);
                }
            }
            _ => {
                clearopbeep(cap.oap_mut());
            }
        }

        let win_handle = curwin_mut().handle;
        let buf_handle = curbuf_mut().b_fnum;
        if foldenable != K_NONE && foldenable != curwin_mut().w_onebuf_opt.wo_fen {
            set_option_value(
                K_OPT_FOLDENABLE,
                boolean_obj(foldenable != 0),
                OPT_LOCAL,
                true,
                None,
            );
            if curwin_mut().handle != win_handle || curbuf_mut().b_fnum != buf_handle {
                clearop(cap.oap_mut());
                return;
            }
        }
        if nchar == b'F' as i32 && cap.oap_mut().op_type == OP_FOLD {
            nv_operator(cap);
            g_st().finish_op = true;
        }
        if foldlevel >= 0 {
            set_option_value(
                K_OPT_FOLDLEVEL,
                integer_obj(foldlevel),
                OPT_LOCAL,
                true,
                None,
            );
        }
        if nchar == b'x' as i32
            && curwin_mut().handle == win_handle
            && curbuf_mut().b_fnum == buf_handle
        {
            fold_open_cursor();
        }
    }
}

fn nv_q_fn(cap: &mut Cmdarg) {
    {
        if g_st().reg_recording != 0
            || g_st().reg_executing != 0
            || (!g_st().visual_global.active && cap.count0 > 2)
        {
            vim_beep(0);
        } else if !checkclearop(cap.oap_mut()) {
            if g_st().visual_global.active {
                do_cmdline_cmd(c"normal! zqj");
                mc_follow_set(K_TRUE);
            } else {
                mc_toggle(
                    curbuf_mut(),
                    curwin_mut().w_cursor,
                    true,
                    if cap.count0 == 0 {
                        K_NONE
                    } else if cap.count0 == 1 {
                        K_TRUE
                    } else {
                        K_FALSE
                    },
                );
            }
        }
    }
}

fn nv_colon(cap: &mut Cmdarg) {
    {
        let is_cmdkey = cap.cmdchar == K_COMMAND;
        let is_lua = cap.cmdchar == K_LUA;

        if g_st().visual_global.active && !is_cmdkey && !is_lua {
            nv_operator(cap);
            return;
        }

        if cap.oap_mut().op_type != OP_NOP {
            cap.oap_mut().motion_type = MT_CHAR_WISE;
            cap.oap_mut().inclusive = false;
        } else if cap.count0 != 0 && !is_cmdkey && !is_lua {
            stuffchar_readbuff(b'.' as i32);
            if cap.count0 > 1 {
                stuff_readbuff(c",.+");
                stuffnum_readbuff(cap.count0 - 1);
            }
        }

        if g_st().key_typed {
            msg_ext_set_trigger(c"typed_cmd");
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
                if cap.oap_mut().op_type != OP_NOP {
                    DOCMD_KEEPLINE
                } else {
                    0
                },
            )
        };
        msg_ext_set_trigger(c"");

        if !cmd_result {
            clearop(cap.oap_mut());
        } else if cap.oap_mut().op_type != OP_NOP
            && (cap.oap_mut().start.lnum > curbuf_mut().b_ml.ml_line_count
                || cap.oap_mut().start.col > ml_get_len(cap.oap_mut().start.lnum)
                || g_st().did_emsg != 0)
        {
            clearopbeep(cap.oap_mut());
        }
    }
}

fn nv_ctrlg(cap: &mut Cmdarg) {
    {
        if g_st().visual_global.active {
            g_st().visual_global.select = !g_st().visual_global.select;
            may_trigger_modechanged();
            showmode();
        } else if !checkclearop(cap.oap_mut()) {
            fileinfo(cap.count0, 0, true);
        }
    }
}

fn nv_ctrlh(cap: &mut Cmdarg) {
    if g_st().visual_global.active && g_st().visual_global.select {
        cap.cmdchar = b'x' as i32;
        v_visop(cap);
    } else {
        nv_left(cap);
    }
}

fn nv_clear(cap: &mut Cmdarg) {
    {
        if checkclearop(cap.oap_mut()) {
            return;
        }

        syn_stack_free_all(curwin_mut().w_s_ptr());
        for wp in for_all_windows_mut() {
            wp.w_s_ptr().b_syn_slow = false;
        }
        redraw_later(Some(crate::globals::curwin_mut()), UPD_CLEAR);
    }
}

fn nv_ctrlo(cap: &mut Cmdarg) {
    if g_st().visual_global.active && g_st().visual_global.select {
        g_st().visual_global.select = false;
        may_trigger_modechanged();
        showmode();
        g_st().visual_global.restart_select = 2;
    } else {
        cap.count1 = -cap.count1;
        nv_pcmark(cap);
    }
}

fn nv_hat(cap: &mut Cmdarg) {
    {
        if !checkclearopq(cap.oap_mut()) {
            buflist_getfile(cap.count0, 0, GETF_SETMARK | GETF_ALT, 0);
        }
    }
}

fn nv_zet_fn(cap: &mut Cmdarg) {
    {
        if checkclearopq(cap.oap_mut()) {
            return;
        }

        match cap.nchar {
            c if c == b'Z' as i32 => {
                do_cmdline_cmd(c"x");
            }
            c if c == b'Q' as i32 => {
                do_cmdline_cmd(c"q!");
            }
            c if c == b'R' as i32 => {
                if cap.count0 >= 1 && cap.count0 <= 8 {
                    do_cmdline_cmd(c"restart!");
                } else if cap.count0 == 9 {
                    do_cmdline_cmd(c"restart! +qall!");
                } else {
                    do_cmdline_cmd(c"restart");
                }
            }
            _ => {
                clearopbeep(cap.oap_mut());
            }
        }
    }
}

pub fn do_nv_ident(c1: i32, c2: i32) {
    let mut oa = Oparg::new();
    let mut ca = Cmdarg::new();

    ca.oap = &raw mut oa;
    ca.cmdchar = c1;
    ca.nchar = c2;
    nv_ident(&mut ca);
}

fn nv_k_getcmd(
    cap: &mut Cmdarg,
    kp: impl crate::strings::IntoCCharPtr,
    kp_help: bool,
    kp_ex: bool,
    ptr_arg: *mut *mut c_char,
    mut n: usize,
    buf: *mut c_char,
    bufsize: usize,
    buflen: &mut usize,
) -> usize {
    let kp = kp.into_c_char_ptr();
    unsafe {
        if kp_help {
            crate::memory::cstr_cpy(buf, c"help! ");
            *buflen = 6;
            return n;
        }

        if kp_ex {
            *buflen = vim_snprintf!(buf, bufsize, c"%s ", kp) as usize;
            if cap.count0 != 0 {
                *buflen += vim_snprintf!(
                    buf.wrapping_add(*buflen),
                    bufsize - *buflen,
                    c"%lld ",
                    cap.count0 as i64,
                ) as usize;
            }
            return n;
        }

        let mut ptr = *ptr_arg;

        while crate::memory::cstr_first_byte(ptr) == b'-' && n > 0 {
            ptr = ptr.wrapping_add(1);
            n -= 1;
        }
        if n == 0 {
            emsg(tr(E_NOIDENT));
            *ptr_arg = ptr;
            return 0;
        }

        let isman = crate::memory::cstr_as_opt_bytes(kp) == Some(b"man");
        let isman_s = crate::memory::cstr_as_opt_bytes(kp) == Some(b"man -s");
        if cap.count0 != 0 && !(isman || isman_s) {
            *buflen = vim_snprintf!(buf, bufsize, c".,.+%lld", (cap.count0 - 1) as i64,) as usize;
        }

        do_cmdline_cmd(c"tabnew");
        *buflen +=
            vim_snprintf!(buf.wrapping_add(*buflen), bufsize - *buflen, c"terminal ") as usize;
        if cap.count0 == 0 && isman_s {
            *buflen +=
                vim_snprintf!(buf.wrapping_add(*buflen), bufsize - *buflen, c"man ") as usize;
        } else {
            *buflen +=
                vim_snprintf!(buf.wrapping_add(*buflen), bufsize - *buflen, c"%s ", kp) as usize;
        }
        if cap.count0 != 0 && (isman || isman_s) {
            *buflen += vim_snprintf!(
                buf.wrapping_add(*buflen),
                bufsize - *buflen,
                c"%lld ",
                cap.count0 as i64,
            ) as usize;
        }

        *ptr_arg = ptr;
        n
    }
}

fn nv_ident(cap: &mut Cmdarg) {
    unsafe {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut n: usize = 0;
        let mut cmdchar: i32;
        let g_cmd: bool;
        let mut tag_cmd = false;

        if cap.cmdchar == b'g' as i32 {
            cmdchar = cap.nchar;
            g_cmd = true;
        } else {
            cmdchar = cap.cmdchar;
            g_cmd = false;
        }

        if cmdchar == POUND {
            cmdchar = b'#' as i32;
        }

        let mut visual_sel = false;
        if cmdchar == b']' as i32 || cmdchar == CTRL_RSB_CONST || cmdchar == b'K' as i32 {
            if g_st().visual_global.active && !get_visual_text(Some(cap), &mut ptr, &mut n) {
                return;
            }
            visual_sel = !ptr.is_null();
            if checkclearopq(cap.oap_mut()) {
                return;
            }
        }

        let mut ident_offset: i32 = 0;
        if ptr.is_null() {
            n = find_ident_under_cursor(
                &raw mut ptr,
                if cmdchar == b'*' as i32 || cmdchar == b'#' as i32 {
                    FIND_IDENT | FIND_STRING
                } else {
                    FIND_IDENT
                },
                Some(&mut ident_offset),
            );
            if n == 0 {
                clearop(cap.oap_mut());
                return;
            }
        }

        let kp = if crate::memory::cstr_is_empty(curbuf_mut().b_p_kp) {
            opt_vars().p_kp
        } else {
            curbuf_mut().b_p_kp
        };
        let kp_helpbang = crate::memory::cstr_as_opt_bytes(kp) == Some(b":help!");
        let kp_help = kp_helpbang
            || crate::memory::cstr_is_empty(kp)
            || (crate::memory::cstr_as_opt_bytes(kp) == Some(b":he"))
            || (crate::memory::cstr_as_opt_bytes(kp) == Some(b":help"));
        if kp_help && !kp_helpbang && *skipwhite(ptr) == NUL {
            emsg(tr(E_NOIDENT));
            return;
        }
        let kp_ex = crate::memory::cstr_first_byte(kp) == b':';
        let bufsize = n * 2 + 30 + crate::memory::cstr_len(kp);
        let mut buf_vec = vec![0 as c_char; bufsize];
        let mut buflen: usize = 0;

        match cmdchar {
            c if c == b'*' as i32 || c == b'#' as i32 => {
                setpcmark();
                curwin_mut().w_cursor.col = ptr.offset_from(get_cursor_line_ptr()) as Colnr;

                if !g_cmd && vim_iswordp(ptr) {
                    crate::memory::cstr_cpy(buf_vec.as_mut_ptr(), c"\\<");
                    buflen = 2;
                }
                g_st().search_global.no_smartcase = true;
            }
            c if c == b'K' as i32 => {
                n = nv_k_getcmd(
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
            c if c == b']' as i32 => {
                tag_cmd = true;
                crate::memory::cstr_cpy(buf_vec.as_mut_ptr(), c"tselect ");
                buflen = 8;
            }
            _ => {
                tag_cmd = true;
                if curbuf_mut().b_help {
                    crate::memory::cstr_cpy(buf_vec.as_mut_ptr(), c"help! ");
                    buflen = 6;
                } else if g_cmd {
                    crate::memory::cstr_cpy(buf_vec.as_mut_ptr(), c"tjump ");
                    buflen = 6;
                } else if cap.count0 == 0 {
                    crate::memory::cstr_cpy(buf_vec.as_mut_ptr(), c"tag ");
                    buflen = 4;
                } else {
                    buflen = vim_snprintf!(
                        buf_vec.as_mut_ptr(),
                        bufsize,
                        c":%lldtag ",
                        cap.count0 as i64,
                    ) as usize;
                }
            }
        }

        if cmdchar == b'K' as i32 && kp_helpbang && !visual_sel {
            crate::memory::cstr_cpy(buf_vec.as_mut_ptr(), c"help!");
            buflen = 5;
        } else if cmdchar == b'K' as i32 && !kp_help {
            let ptr_cstr =
                std::ffi::CString::new(crate::memory::cstr_slice(ptr, n)).unwrap_or_default();
            let p = if kp_ex {
                vim_strsave_fnameescape(ptr_cstr.as_ptr(), VSE_NONE)
            } else {
                vim_strsave_shellescape(&ptr_cstr, true, true)
            };
            let plen = p.to_bytes().len();
            buf_vec.resize(buflen + plen + 1, 0);
            crate::memory::cstr_cpy(buf_vec.as_mut_ptr().wrapping_add(buflen), p.as_ptr());
            buflen += plen;
        } else {
            let aux_ptr = if cmdchar == b'*' as i32 {
                if opt_vars().p_magic != 0 {
                    c"/.*~[^$\\".as_ptr()
                } else {
                    c"/^$\\".as_ptr()
                }
            } else if cmdchar == b'#' as i32 {
                if opt_vars().p_magic != 0 {
                    c"/?.*~[^$\\".as_ptr()
                } else {
                    c"/?^$\\".as_ptr()
                }
            } else if tag_cmd {
                if crate::memory::cstr_as_opt_bytes(curbuf_mut().b_p_ft) == Some(b"help") {
                    c"".as_ptr()
                } else {
                    c"\\|\"\n[".as_ptr()
                }
            } else {
                c"\\|\"\n*?[".as_ptr()
            };

            let buf = buf_vec.as_mut_ptr();
            let mut p = buf.wrapping_add(buflen);
            while n > 0 {
                n -= 1;
                if crate::strings::vim_haschr(
                    aux_ptr,
                    i32::from(crate::memory::cstr_first_byte(ptr)),
                ) {
                    *p = b'\\' as c_char;
                    p = p.wrapping_add(1);
                }
                let len = (utfc_ptr2len(ptr) - 1) as usize;
                let mut i = 0;
                while i < len && n > 0 {
                    *p = *ptr;
                    p = p.wrapping_add(1);
                    ptr = ptr.wrapping_add(1);
                    i += 1;
                    n -= 1;
                }
                *p = *ptr;
                p = p.wrapping_add(1);
                ptr = ptr.wrapping_add(1);
            }
            *p = NUL;
            buflen = p.offset_from(buf) as usize;
        }

        let buf = buf_vec.as_mut_ptr();
        if cmdchar == b'*' as i32 || cmdchar == b'#' as i32 {
            if !g_cmd && vim_iswordp(mb_prevptr(get_cursor_line_ptr(), ptr)) {
                crate::memory::cstr_cpy(buf.wrapping_add(buflen), c"\\>");
                buflen += 2;
            }

            init_history();
            add_to_history(HIST_SEARCH, buf, buflen, true, NUL as i32);

            normal_search(
                cap,
                if cmdchar == b'*' as i32 {
                    b'/' as i32
                } else {
                    b'?' as i32
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

            if cmdchar == b'K' as i32 && !kp_ex && !kp_help {
                g_st().restart_edit = b'i' as i32;
                add_map(c"<esc>", c"<Cmd>bdelete!<CR>", MODE_TERMINAL, true);
            }
        }
    }
}

pub fn get_visual_text(cap: Option<&mut Cmdarg>, pp: &mut *mut c_char, lenp: &mut usize) -> bool {
    unsafe {
        if g_st().visual_global.mode != b'V' as i32 {
            unadjust_for_sel();
        }
        if g_st().visual_global.start.lnum != curwin_mut().w_cursor.lnum {
            if let Some(cap) = cap {
                clearopbeep(cap.oap_mut());
            }
            return false;
        }
        if g_st().visual_global.mode == b'V' as i32 {
            *pp = get_cursor_line_ptr();
            *lenp = get_cursor_line_len() as usize;
        } else {
            if lt(curwin_mut().w_cursor, g_st().visual_global.start) {
                *pp = ml_get_pos(&curwin_mut().w_cursor);
                *lenp = (g_st().visual_global.start.col - curwin_mut().w_cursor.col + 1) as usize;
            } else {
                *pp = ml_get_pos(&g_st().visual_global.start);
                *lenp = (curwin_mut().w_cursor.col - g_st().visual_global.start.col + 1) as usize;
            }
            if **pp == NUL {
                *lenp = 0;
            }
            if *lenp > 0 {
                *lenp = (*lenp).wrapping_add_signed(
                    (utfc_ptr2len((*pp).wrapping_add(*lenp - 1)) - 1) as isize,
                );
            }
        }
        reset_visual_and_resel();
        true
    }
}

fn nv_tagpop(cap: &mut Cmdarg) {
    {
        if !checkclearopq(cap.oap_mut()) {
            do_tag(None, c"", DT_POP, cap.count1, 0, true);
        }
    }
}

fn nv_scroll(cap: &mut Cmdarg) {
    {
        let mut n: i32;
        let mut lnum: Linenr = 0;

        cap.oap_mut().motion_type = MT_LINE_WISE;
        setpcmark();

        if cap.cmdchar == b'L' as i32 {
            validate_botline_win(curwin_mut());
            curwin_mut().w_cursor.lnum = curwin_mut().w_botline - 1;
            if cap.count1 > curwin_mut().w_cursor.lnum {
                curwin_mut().w_cursor.lnum = 1;
            } else if win_lines_concealed(curwin_mut()) {
                n = cap.count1 - 1;
                while n > 0 && curwin_mut().w_cursor.lnum > curwin_mut().w_topline {
                    has_folding(
                        curwin_ptr(),
                        curwin_mut().w_cursor.lnum,
                        Some(&mut curwin_mut().w_cursor.lnum),
                        ptr::null_mut(),
                    );
                    n += i32::from(decor_conceal_line(
                        unsafe { &mut *(curwin_ptr()) },
                        curwin_mut().w_cursor.lnum,
                        true,
                    ));
                    if curwin_mut().w_cursor.lnum > curwin_mut().w_topline {
                        curwin_mut().w_cursor.lnum -= 1;
                    }
                    n -= 1;
                }
            } else {
                curwin_mut().w_cursor.lnum -= cap.count1 - 1;
            }
        } else {
            if cap.cmdchar == b'M' as i32 {
                let mut used =
                    -(win_get_fill(crate::globals::curwin_mut(), curwin_mut().w_topline)
                        - curwin_mut().w_topfill);
                validate_botline_win(curwin_mut());
                let half = (curwin_mut().w_view_height - curwin_mut().w_empty_rows + 1) / 2;
                n = 0;
                while curwin_mut().w_topline + n < curbuf_mut().b_ml.ml_line_count {
                    if n > 0
                        && used
                            + win_get_fill(crate::globals::curwin_mut(), curwin_mut().w_topline + n)
                                / 2
                            >= half
                    {
                        n -= 1;
                        break;
                    }
                    used += plines_win(curwin_ptr(), curwin_mut().w_topline + n, true);
                    if used >= half {
                        break;
                    }
                    if has_folding(
                        curwin_ptr(),
                        curwin_mut().w_topline + n,
                        None,
                        &raw mut lnum,
                    ) {
                        n = lnum - curwin_mut().w_topline;
                    }
                    n += 1;
                }
                if n > 0 && used > curwin_mut().w_view_height {
                    n -= 1;
                }
            } else {
                n = cap.count1 - 1;
                if win_lines_concealed(curwin_mut()) {
                    lnum = curwin_mut().w_topline;
                    while (decor_conceal_line(unsafe { &mut *(curwin_ptr()) }, lnum - 1, true) || {
                        let prev = n;
                        n -= 1;
                        prev > 0
                    }) && lnum < curwin_mut().w_botline - 1
                    {
                        has_folding(curwin_ptr(), lnum, None, &raw mut lnum);
                        lnum += 1;
                    }
                    n = lnum - curwin_mut().w_topline;
                }
            }
            curwin_mut().w_cursor.lnum =
                (curwin_mut().w_topline + n).min(curbuf_mut().b_ml.ml_line_count);
        }

        if cap.oap_mut().op_type == OP_NOP {
            cursor_correct(curwin_mut());
        }
        beginline(BL_SOL | BL_FIX);
    }
}

fn nv_right(cap: &mut Cmdarg) {
    {
        if (g_st().mod_mask & (i32::from(MOD_MASK_SHIFT) | MOD_MASK_CTRL)) != 0 {
            if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
                cap.arg = 1;
            }
            nv_wordcmd(cap);
            return;
        }

        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        let mut past_line =
            g_st().visual_global.active && crate::memory::cstr_first_byte(opt_vars().p_sel) != b'o';

        if virtual_active(curwin_mut()) {
            past_line = false;
        }

        let mut n = cap.count1;
        while n > 0 {
            if (!past_line && !oneright())
                || (past_line && crate::memory::cstr_is_empty(get_cursor_pos_ptr()))
            {
                if ((cap.cmdchar == b' ' as i32
                    && crate::strings::vim_haschr(opt_vars().p_ww, b's' as i32))
                    || (cap.cmdchar == b'l' as i32
                        && crate::strings::vim_haschr(opt_vars().p_ww, b'l' as i32))
                    || (cap.cmdchar == K_RIGHT
                        && crate::strings::vim_haschr(opt_vars().p_ww, b'>' as i32)))
                    && curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count
                {
                    if cap.oap_mut().op_type != OP_NOP
                        && !cap.oap_mut().inclusive
                        && !lineempty(curwin_mut().w_cursor.lnum)
                    {
                        cap.oap_mut().inclusive = true;
                    } else {
                        curwin_mut().w_cursor.lnum += 1;
                        curwin_mut().w_cursor.col = 0;
                        curwin_mut().w_cursor.coladd = 0;
                        curwin_mut().w_set_curswant = true;
                        cap.oap_mut().inclusive = false;
                    }
                    n -= 1;
                    continue;
                }
                if cap.oap_mut().op_type == OP_NOP {
                    if n == cap.count1 {
                        beep_flush();
                    }
                } else if !lineempty(curwin_mut().w_cursor.lnum) {
                    cap.oap_mut().inclusive = true;
                }
                break;
            } else if past_line {
                curwin_mut().w_set_curswant = true;
                if virtual_active(curwin_mut()) {
                    oneright();
                } else {
                    curwin_mut().w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                }
            }
            n -= 1;
        }
        if n != cap.count1
            && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_left(cap: &mut Cmdarg) {
    {
        if (g_st().mod_mask & (i32::from(MOD_MASK_SHIFT) | MOD_MASK_CTRL)) != 0 {
            if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
                cap.arg = 1;
            }
            nv_bck_word(cap);
            return;
        }

        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        let mut n = cap.count1;
        while n > 0 {
            if !oneleft() {
                if (((cap.cmdchar == K_BS || cap.cmdchar == CTRL_H_CONST)
                    && crate::strings::vim_haschr(opt_vars().p_ww, b'b' as i32))
                    || (cap.cmdchar == b'h' as i32
                        && crate::strings::vim_haschr(opt_vars().p_ww, b'h' as i32))
                    || (cap.cmdchar == K_LEFT
                        && crate::strings::vim_haschr(opt_vars().p_ww, b'<' as i32)))
                    && curwin_mut().w_cursor.lnum > 1
                {
                    curwin_mut().w_cursor.lnum -= 1;
                    coladvance(curwin_mut(), MAXCOL);
                    curwin_mut().w_set_curswant = true;

                    if (cap.oap_mut().op_type == OP_DELETE || cap.oap_mut().op_type == OP_CHANGE)
                        && !lineempty(curwin_mut().w_cursor.lnum)
                    {
                        let cp = get_cursor_pos_ptr();
                        if !crate::memory::cstr_is_empty(cp) {
                            curwin_mut().w_cursor.col += utfc_ptr2len(cp);
                        }
                        cap.retval |= CA_NO_ADJ_OP_END;
                    }
                    n -= 1;
                    continue;
                } else if cap.oap_mut().op_type == OP_NOP && n == cap.count1 {
                    beep_flush();
                }
                break;
            }
            n -= 1;
        }
        if n != cap.count1
            && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_up(cap: &mut Cmdarg) {
    {
        if (g_st().mod_mask & i32::from(MOD_MASK_SHIFT)) != 0 {
            cap.arg = BACKWARD;
            nv_page(cap);
            return;
        }

        cap.oap_mut().motion_type = MT_LINE_WISE;
        if !cursor_up(cap.count1, cap.oap_mut().op_type == OP_NOP) {
            clearopbeep(cap.oap_mut());
        } else if cap.arg != 0 {
            beginline(BL_WHITE | BL_FIX);
        }
    }
}

fn nv_down(cap: &mut Cmdarg) {
    {
        if (g_st().mod_mask & i32::from(MOD_MASK_SHIFT)) != 0 {
            cap.arg = FORWARD;
            nv_page(cap);
        } else if bt_cmdwin(g_st().curbuf) && cap.cmdchar == CAR {
            cmdwin_do_action(c"confirm");
        } else if bt_quickfix(Some(crate::globals::curbuf_mut())) && cap.cmdchar == CAR {
            qf_view_result(false);
        } else if bt_prompt(Some(crate::globals::curbuf_mut()))
            && cap.cmdchar == CAR
            && curwin_mut().w_cursor.lnum == curbuf_mut().b_ml.ml_line_count
        {
            prompt_invoke_callback();
            if g_st().restart_edit == 0 {
                g_st().restart_edit = b'a' as i32;
            }
        } else {
            cap.oap_mut().motion_type = MT_LINE_WISE;
            if !cursor_down(cap.count1, cap.oap_mut().op_type == OP_NOP) {
                clearopbeep(cap.oap_mut());
            } else if cap.arg != 0 {
                beginline(BL_WHITE | BL_FIX);
            }
        }
    }
}

fn nv_gotofile(cap: &mut Cmdarg) {
    {
        let mut lnum: Linenr = -1;

        if check_text_or_curbuf_locked(Some(cap.oap_mut())) {
            return;
        }

        if !check_can_set_curbuf_disabled() {
            return;
        }

        if let Some(fname_cstr) = grab_file_name(cap.count1, &mut lnum) {
            let ptr = fname_cstr.as_ptr().cast_mut();
            if curbuf_is_changed_fn() && curbuf_mut().b_nwindows <= 1 && !buf_hide(curbuf_mut()) {
                autowrite(curbuf_mut(), false);
            }
            setpcmark();
            if do_ecmd(
                0,
                ptr,
                ptr::null_mut(),
                None,
                ECMD_LAST,
                if buf_hide(curbuf_mut()) { ECMD_HIDE } else { 0 },
                curwin_ptr(),
            ) && cap.nchar == b'F' as i32
                && lnum >= 0
            {
                curwin_mut().w_cursor.lnum = lnum;
                check_cursor_lnum(curwin_mut());
                beginline(BL_SOL | BL_FIX);
            }
        } else {
            clearop(cap.oap_mut());
        }
    }
}

fn nv_end(cap: &mut Cmdarg) {
    if cap.arg != 0 || (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
        cap.arg = 1;
        nv_goto(cap);
        cap.count1 = 1;
    }
    nv_dollar(cap);
}

fn nv_dollar(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = true;
        if !virtual_active(curwin_mut())
            || gchar_cursor() != NUL as i32
            || cap.oap_mut().op_type == OP_NOP
        {
            curwin_mut().w_curswant = MAXCOL;
        }
        if !cursor_down(cap.count1 - 1, cap.oap_mut().op_type == OP_NOP) {
            clearopbeep(cap.oap_mut());
        } else if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_search(cap: &mut Cmdarg) {
    unsafe {
        let oap = &mut *cap.oap;
        let save_cursor = curwin_mut().w_cursor;

        if cap.cmdchar == b'?' as i32 && cap.oap_mut().op_type == OP_ROT13 {
            cap.cmdchar = b'g' as i32;
            cap.nchar = b'?' as i32;
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
            if cap.arg != 0 || !equalpos(save_cursor, curwin_mut().w_cursor) {
                0
            } else {
                SEARCH_MARK
            },
            None,
        );
    }
}

fn nv_next(cap: &mut Cmdarg) {
    {
        let old = curwin_mut().w_cursor;
        let mut wrapped: i32 = 0;
        let i = normal_search(
            cap,
            0,
            ptr::null_mut(),
            0,
            SEARCH_MARK | cap.arg,
            Some(&mut wrapped),
        );

        if i == 1 && wrapped == 0 && equalpos(old, curwin_mut().w_cursor) {
            cap.count1 += 1;
            normal_search(cap, 0, ptr::null_mut(), 0, SEARCH_MARK | cap.arg, None);
            cap.count1 -= 1;
        }

        if i > 0
            && opt_vars().p_hls != 0
            && !g_st().search_global.no_hlsearch
            && win_hl_attr(curwin_mut(), HLF_LC as i32) != win_hl_attr(curwin_mut(), HLF_L as i32)
        {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_SOME_VALID);
        }
    }
}

fn normal_search(
    cap: &mut Cmdarg,
    dir: i32,
    pat: impl crate::strings::IntoCCharPtr,
    patlen: usize,
    opt: i32,
    wrapped: Option<&mut i32>,
) -> i32 {
    let pat = pat.into_c_char_ptr().cast_mut();
    {
        let mut sia = SearchitArg::new();
        let prev_cursor = curwin_mut().w_cursor;

        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        cap.oap_mut().use_reg_one = true;
        curwin_mut().w_set_curswant = true;

        let i = do_search(
            Some(cap.oap_mut()),
            dir,
            dir,
            pat,
            patlen,
            cap.count1,
            opt | SEARCH_OPT | SEARCH_ECHO | SEARCH_MSG,
            opt_vars().p_magic != 0,
            Some(&mut sia),
        );
        if let Some(wrapped) = wrapped {
            *wrapped = sia.sa_wrapped;
        }
        if i == 0 {
            clearop(cap.oap_mut());
        } else {
            if i == 2 {
                cap.oap_mut().motion_type = MT_LINE_WISE;
            }
            curwin_mut().w_cursor.coladd = 0;
            if cap.oap_mut().op_type == OP_NOP
                && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_SEARCH) != 0
                && g_st().key_typed
            {
                fold_open_cursor();
            }
        }
        if !equalpos(curwin_mut().w_cursor, prev_cursor)
            && opt_vars().p_hls != 0
            && !g_st().search_global.no_hlsearch
            && win_hl_attr(curwin_mut(), HLF_LC as i32) != win_hl_attr(curwin_mut(), HLF_L as i32)
        {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_SOME_VALID);
        }

        check_cursor(curwin_mut());
        i
    }
}

fn nv_csearch(cap: &mut Cmdarg) {
    {
        let mut cursor_dec = false;

        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            && g_st().visual_global.active
            && g_st().visual_global.mode == b'v' as i32
            && g_st().visual_global.select_exclu_adj
        {
            unadjust_for_sel();
            cursor_dec = true;
        }

        let t_cmd = cap.cmdchar == b't' as i32 || cap.cmdchar == b'T' as i32;

        cap.oap_mut().motion_type = MT_CHAR_WISE;
        if is_special(cap.nchar) || !searchc(cap, t_cmd) {
            clearopbeep(cap.oap_mut());
            if cursor_dec {
                adjust_for_sel(cap);
            }
            return;
        }

        curwin_mut().w_set_curswant = true;
        if gchar_cursor() == TAB
            && virtual_active(curwin_mut())
            && cap.arg == FORWARD
            && (t_cmd || cap.oap_mut().op_type != OP_NOP)
        {
            let mut scol: Colnr = 0;
            let mut ecol: Colnr = 0;
            getvcol(
                curwin_mut(),
                &mut curwin_mut().w_cursor,
                Some(&mut scol),
                None,
                Some(&mut ecol),
                0,
            );
            curwin_mut().w_cursor.coladd = ecol - scol;
        } else {
            curwin_mut().w_cursor.coladd = 0;
        }
        adjust_for_sel(cap);
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_bracket_block(cap: &mut Cmdarg, old_pos: &CursorPos) {
    unsafe {
        let mut new_pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut pos: *mut CursorPos = ptr::null_mut();
        let mut prev_pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut n: i32;
        let findc: i32;

        if cap.nchar == b'*' as i32 {
            cap.nchar = b'/' as i32;
        }
        if cap.nchar == b'm' as i32 || cap.nchar == b'M' as i32 {
            if cap.cmdchar == b'[' as i32 {
                findc = b'{' as i32;
            } else {
                findc = b'}' as i32;
            }
            n = 9999;
        } else {
            findc = cap.nchar;
            n = cap.count1;
        }
        while n > 0 {
            pos = findmatchlimit(
                Some(cap.oap_mut()),
                findc,
                if cap.cmdchar == b'[' as i32 {
                    FM_BACKWARD
                } else {
                    FM_FORWARD
                },
                0,
            );
            if pos.is_null() {
                if new_pos.lnum == 0 {
                    if cap.nchar != b'm' as i32 && cap.nchar != b'M' as i32 {
                        clearopbeep(cap.oap_mut());
                    }
                } else {
                    pos = &raw mut new_pos;
                }
                break;
            }
            prev_pos = new_pos;
            curwin_mut().w_cursor = *pos;
            new_pos = *pos;
            n -= 1;
        }
        curwin_mut().w_cursor = *old_pos;

        if cap.nchar == b'm' as i32 || cap.nchar == b'M' as i32 {
            let norm = (findc == b'{' as i32) == (cap.nchar == b'm' as i32);

            n = cap.count1;
            if prev_pos.lnum != 0 {
                pos = &raw mut prev_pos;
                curwin_mut().w_cursor = prev_pos;
                if norm {
                    n -= 1;
                }
            } else {
                pos = ptr::null_mut();
            }
            while n > 0 {
                loop {
                    let step_res = if findc == b'{' as i32 {
                        dec_cursor()
                    } else {
                        inc_cursor()
                    };
                    if step_res < 0 {
                        if pos.is_null() {
                            clearopbeep(cap.oap_mut());
                        }
                        n = 0;
                        break;
                    }
                    let c = gchar_cursor();
                    if c == b'{' as i32 || c == b'}' as i32 {
                        if (c == findc && norm) || (n == 1 && !norm) {
                            new_pos = curwin_mut().w_cursor;
                            pos = &raw mut new_pos;
                            n = 0;
                        } else if new_pos.lnum == 0 {
                            new_pos = curwin_mut().w_cursor;
                            pos = &raw mut new_pos;
                        } else {
                            pos = findmatchlimit(
                                Some(cap.oap_mut()),
                                findc,
                                if cap.cmdchar == b'[' as i32 {
                                    FM_BACKWARD
                                } else {
                                    FM_FORWARD
                                },
                                0,
                            );
                            if pos.is_null() {
                                n = 0;
                            } else {
                                curwin_mut().w_cursor = *pos;
                            }
                        }
                        break;
                    }
                }
                n -= 1;
            }
            curwin_mut().w_cursor = *old_pos;
            if pos.is_null() && new_pos.lnum != 0 {
                clearopbeep(cap.oap_mut());
            }
        }
        if !pos.is_null() {
            setpcmark();
            curwin_mut().w_cursor = *pos;
            curwin_mut().w_set_curswant = true;
            if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_BLOCK) != 0
                && g_st().key_typed
                && cap.oap_mut().op_type == OP_NOP
            {
                fold_open_cursor();
            }
        }
    }
}

fn nv_brackets(cap: &mut Cmdarg) {
    unsafe {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        let old_pos = curwin_mut().w_cursor;
        curwin_mut().w_cursor.coladd = 0;

        if cap.nchar == b'f' as i32 {
            nv_gotofile(cap);
        } else if crate::strings::vim_haschr(c"iI\t\x04dD", cap.nchar) {
            let mut ptr: *mut c_char = ptr::null_mut();
            let len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
            if len == 0 {
                clearop(cap.oap_mut());
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
                    if (cap.nchar & 0xf) == (b'd' as i32 & 0xf) {
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
                    if cap.cmdchar == b']' as i32 {
                        curwin_mut().w_cursor.lnum + 1
                    } else {
                        1
                    },
                    MAXLNUM,
                    false,
                    false,
                );
                curwin_mut().w_set_curswant = true;
            }
        } else if (cap.cmdchar == b'[' as i32 && !vim_strchr(c"{(*/#mM", cap.nchar).is_null())
            || (cap.cmdchar == b']' as i32 && !vim_strchr(c"})*/#mM", cap.nchar).is_null())
        {
            nv_bracket_block(cap, &old_pos);
        } else if cap.nchar == b'[' as i32 || cap.nchar == b']' as i32 {
            let flag = if cap.nchar == cap.cmdchar {
                b'{' as i32
            } else {
                b'}' as i32
            };
            curwin_mut().w_set_curswant = true;
            if !findpar(
                &mut cap.oap_mut().inclusive,
                cap.arg,
                cap.count1,
                flag,
                cap.oap_mut().op_type != OP_NOP && cap.arg == FORWARD && flag == b'{' as i32,
            ) {
                clearopbeep(cap.oap_mut());
            } else {
                if cap.oap_mut().op_type == OP_NOP {
                    beginline(BL_WHITE | BL_FIX);
                }
                if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_BLOCK) != 0
                    && g_st().key_typed
                    && cap.oap_mut().op_type == OP_NOP
                {
                    fold_open_cursor();
                }
            }
        } else if cap.nchar == b'p' as i32 || cap.nchar == b'P' as i32 {
            nv_put_opt(cap, true);
        } else if cap.nchar == b'\'' as i32 || cap.nchar == b'`' as i32 {
            let mut fm: *mut Fmark = pos_to_mark(curbuf_mut(), None, curwin_mut().w_cursor);
            let mut prev_fm = fm;
            let mut n = cap.count1;
            while n > 0 {
                prev_fm = fm;
                fm = getnextmark(
                    &mut (*fm).mark,
                    if cap.cmdchar == b'[' as i32 {
                        BACKWARD
                    } else {
                        FORWARD
                    },
                    i32::from(cap.nchar == b'\'' as i32),
                );
                if fm.is_null() {
                    break;
                }
                n -= 1;
            }
            if fm.is_null() {
                fm = prev_fm;
            }
            let mut flags = K_MARK_CONTEXT;
            if cap.nchar == b'\'' as i32 {
                flags |= K_MARK_BEGIN_LINE;
            }
            nv_mark_move_to(cap, flags, &mut *fm);
        } else if cap.nchar >= K_RIGHTRELEASE && cap.nchar <= K_LEFTMOUSE {
            do_mouse(
                Some(cap.oap_mut()),
                cap.nchar,
                if cap.cmdchar == b']' as i32 {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count1,
                PUT_FIXINDENT != 0,
            );
        } else if cap.nchar == b'z' as i32 {
            if !fold_move_to(
                false,
                if cap.cmdchar == b']' as i32 {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count1,
            ) {
                clearopbeep(cap.oap_mut());
            }
        } else if cap.nchar == b'c' as i32 {
            if !diff_move_to(
                if cap.cmdchar == b']' as i32 {
                    FORWARD
                } else {
                    BACKWARD
                },
                cap.count1,
            ) {
                clearopbeep(cap.oap_mut());
            }
        } else if cap.nchar == b'C' as i32 {
            if cap.oap_mut().op_type != OP_NOP && mc_buf_has_cursors(curbuf_mut()) {
                clearopbeep(cap.oap_mut());
            } else {
                let mut tv_args: [TypVal; 3] = [
                    TypVal {
                        v_type: VarType::VarBool,
                        v_lock: 0,
                        vval: TypvalVval::Bool(if cap.cmdchar == b']' as i32 {
                            K_BOOL_VAR_TRUE_CONST
                        } else {
                            K_BOOL_VAR_FALSE_CONST
                        }),
                    },
                    TypVal {
                        v_type: VarType::VarNumber,
                        v_lock: 0,
                        vval: TypvalVval::Number(Varnumber::from(cap.count1)),
                    },
                    TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: 0,
                        vval: TypvalVval::Number(0),
                    },
                ];
                let mut rettv = TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                };
                nlua_call_typval(
                    c"vim._core.mcursor",
                    c"jump",
                    tv_args.as_mut_ptr(),
                    Some(&mut rettv),
                );
                if rettv.v_type != VarType::VarBool || rettv.vval.v_bool() != K_BOOL_VAR_TRUE_CONST
                {
                    clearopbeep(cap.oap_mut());
                }
                tv_clear(&mut rettv);
            }
        } else if cap.nchar == b'r' as i32 || cap.nchar == b's' as i32 || cap.nchar == b'S' as i32 {
            setpcmark();
            for _ in 0..cap.count1 {
                if spell_move_to(
                    curwin_mut(),
                    if cap.cmdchar == b']' as i32 {
                        FORWARD
                    } else {
                        BACKWARD
                    },
                    if cap.nchar == b's' as i32 {
                        SMT_ALL
                    } else if cap.nchar == b'r' as i32 {
                        SMT_RARE
                    } else {
                        SMT_BAD
                    },
                    false,
                    ptr::null_mut(),
                ) == 0
                {
                    clearopbeep(cap.oap_mut());
                    break;
                }
                curwin_mut().w_set_curswant = true;
            }
            if cap.oap_mut().op_type == OP_NOP
                && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_SEARCH) != 0
                && g_st().key_typed
            {
                fold_open_cursor();
            }
        } else {
            clearopbeep(cap.oap_mut());
        }
    }
}

fn buf_has_cstyle_comments() -> bool {
    {
        let mut part_buf: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
        let mut list = curbuf_mut().b_p_com;
        while !crate::memory::cstr_is_empty(list) {
            copy_option_part(&raw mut list, part_buf.as_mut_ptr(), COM_MAX_LEN, c",");
            let string = vim_strchr(part_buf.as_ptr(), b':' as i32);
            if !string.is_null()
                && crate::memory::cstr_byte_at(string, 1) == b'/'
                && (crate::memory::cstr_byte_at(string, 2) == b'/'
                    || crate::memory::cstr_byte_at(string, 2) == b'*')
            {
                return true;
            }
        }
        false
    }
}

fn nv_percent(cap: &mut Cmdarg) {
    unsafe {
        let lnum = curwin_mut().w_cursor.lnum;

        cap.oap_mut().inclusive = true;
        if cap.count0 != 0 {
            if cap.count0 > 100 {
                clearopbeep(cap.oap_mut());
            } else {
                cap.oap_mut().motion_type = MT_LINE_WISE;
                setpcmark();
                if curbuf_mut().b_ml.ml_line_count >= 21_474_836 {
                    curwin_mut().w_cursor.lnum =
                        (curbuf_mut().b_ml.ml_line_count + 99) / 100 * cap.count0;
                } else {
                    curwin_mut().w_cursor.lnum =
                        (curbuf_mut().b_ml.ml_line_count * cap.count0 + 99) / 100;
                }
                curwin_mut().w_cursor.lnum = curwin_mut()
                    .w_cursor
                    .lnum
                    .max(1)
                    .min(curbuf_mut().b_ml.ml_line_count);
                beginline(BL_SOL | BL_FIX);
            }
        } else {
            let mut flags = 0;
            if !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_MATCH)
                && buf_has_cstyle_comments()
            {
                let comment_col = check_linecomment(get_cursor_line_ptr());
                if comment_col == MAXCOL || curwin_mut().w_cursor.col < comment_col {
                    flags = FM_SKIPCOMM;
                }
            }

            cap.oap_mut().motion_type = MT_CHAR_WISE;
            cap.oap_mut().use_reg_one = true;
            let pos = findmatchlimit(Some(cap.oap_mut()), NUL as i32, flags, 0);
            if pos.is_null() {
                clearopbeep(cap.oap_mut());
            } else {
                setpcmark();
                curwin_mut().w_cursor = *pos;
                curwin_mut().w_set_curswant = true;
                curwin_mut().w_cursor.coladd = 0;
                adjust_for_sel(cap);
            }
        }
        if cap.oap_mut().op_type == OP_NOP
            && lnum != curwin_mut().w_cursor.lnum
            && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_PERCENT) != 0
            && g_st().key_typed
        {
            fold_open_cursor();
        }
    }
}

fn nv_brace(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().use_reg_one = true;
        cap.oap_mut().inclusive = false;
        curwin_mut().w_set_curswant = true;

        if !findsent(cap.arg, cap.count1) {
            clearopbeep(cap.oap_mut());
            return;
        }

        adjust_cursor(cap.oap_mut());
        curwin_mut().w_cursor.coladd = 0;
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_BLOCK) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_mark(cap: &mut Cmdarg) {
    {
        if checkclearop(cap.oap_mut()) {
            return;
        }

        if !setmark(cap.nchar) {
            clearopbeep(cap.oap_mut());
        }
    }
}

fn nv_findpar(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        cap.oap_mut().use_reg_one = true;
        curwin_mut().w_set_curswant = true;
        if !findpar(
            &mut cap.oap_mut().inclusive,
            cap.arg,
            cap.count1,
            NUL as i32,
            false,
        ) {
            clearopbeep(cap.oap_mut());
            return;
        }

        curwin_mut().w_cursor.coladd = 0;
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_BLOCK) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_undo(cap: &mut Cmdarg) {
    {
        if cap.oap_mut().op_type == OP_LOWER || g_st().visual_global.active {
            cap.cmdchar = b'g' as i32;
            cap.nchar = b'u' as i32;
            nv_operator(cap);
        } else {
            nv_kundo(cap);
        }
    }
}

fn nv_kundo(cap: &mut Cmdarg) {
    {
        if checkclearopq(cap.oap_mut()) {
            return;
        }

        u_undo(cap.count1);
        curwin_mut().w_set_curswant = true;
    }
}

fn nv_replace(cap: &mut Cmdarg) {
    {
        let mut had_ctrl_v: i32;

        if checkclearop(cap.oap_mut()) {
            return;
        }
        if bt_prompt(Some(crate::globals::curbuf_mut())) && !prompt_curpos_editable() {
            clearopbeep(cap.oap_mut());
            return;
        }

        if cap.nchar == CTRL_V_CONST || cap.nchar == CTRL_Q_CONST {
            had_ctrl_v = CTRL_V_CONST;
            cap.nchar = get_literal(false);
            if cap.nchar > DEL {
                had_ctrl_v = NUL as i32;
            }
        } else {
            had_ctrl_v = NUL as i32;
        }

        if is_special(cap.nchar) {
            clearopbeep(cap.oap_mut());
            return;
        }

        if g_st().visual_global.active {
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

        if virtual_active(curwin_mut()) {
            if !u_save_cursor() {
                return;
            }
            if gchar_cursor() == NUL as i32 {
                coladvance_force(getviscol() + cap.count1);
                curwin_mut().w_cursor.col -= cap.count1;
            } else if gchar_cursor() == TAB {
                coladvance_force(getviscol());
            }
        }

        if (get_cursor_pos_len() as usize) < (cap.count1 as u32 as usize)
            || mb_charlen(get_cursor_pos_ptr()) < cap.count1
        {
            clearopbeep(cap.oap_mut());
            return;
        }

        if had_ctrl_v != CTRL_V_CONST
            && cap.nchar == b'\t' as i32
            && (curbuf_mut().b_p_et != 0 || opt_vars().p_sta != 0)
        {
            stuffnum_readbuff(cap.count1);
            stuffchar_readbuff(b'R' as i32);
            stuffchar_readbuff(b'\t' as i32);
            stuffchar_readbuff(ESC);
            exec_stuffed(Some(cap));
            return;
        }

        if !u_save_cursor() {
            return;
        }

        if had_ctrl_v != CTRL_V_CONST && (cap.nchar == b'\r' as i32 || cap.nchar == b'\n' as i32) {
            del_chars(cap.count1, 0);
            stuffchar_readbuff(b'\r' as i32);
            stuffchar_readbuff(ESC);

            invoke_edit(cap, 1, b'r' as i32, 0);
        } else {
            let mut spec = CmdSpec::new();
            spec.regname = cap.oap_mut().regname;
            spec.count = i64::from(cap.count1);
            spec.cmd = b'r' as i32;
            spec.cmdarg = cap.nchar;
            prep_redo(true, true, spec);
            if had_ctrl_v != NUL as i32 {
                redo_append_char(had_ctrl_v);
            }

            curbuf_mut().b_op_start = curwin_mut().w_cursor;
            let old_state = g_st().state_global;

            if cap.nchar_len > 0 {
                redo_append_str(cap.nchar_composing.as_ptr(), -1);
            } else {
                redo_append_char(cap.nchar);
            }

            let mut n = cap.count1;
            while n > 0 {
                g_st().state_global = MODE_REPLACE;
                if cap.nchar == CTRL_E || cap.nchar == CTRL_Y {
                    let c = ins_copychar(
                        curwin_mut().w_cursor.lnum + if cap.nchar == CTRL_Y { -1 } else { 1 },
                    );
                    if c != NUL as i32 {
                        ins_char(c);
                    } else {
                        curwin_mut().w_cursor.col += 1;
                    }
                } else if cap.nchar_len != 0 {
                    ins_char_bytes(cap.nchar_composing.as_mut_ptr(), cap.nchar_len as usize);
                } else {
                    ins_char(cap.nchar);
                }
                g_st().state_global = old_state;
                n -= 1;
            }
            curwin_mut().w_cursor.col -= 1;
            mb_adjust_cursor();
            curbuf_mut().b_op_end = curwin_mut().w_cursor;
            curwin_mut().w_set_curswant = true;
            set_last_insert(cap.nchar);
        }

        fold_update_after_insert();
    }
}

fn v_swap_corners(cmdchar: i32) {
    unsafe {
        if cmdchar == b'O' as i32 && g_st().visual_global.mode == CTRL_V_CONST {
            let mut left: Colnr = 0;
            let mut right: Colnr = 0;
            let mut old_cursor = curwin_mut().w_cursor;
            getvcols(
                curwin_ptr(),
                &mut old_cursor,
                &mut g_st().visual_global.start,
                &mut left,
                &mut right,
                0,
            );
            curwin_mut().w_cursor.lnum = g_st().visual_global.start.lnum;
            coladvance(curwin_mut(), left);
            g_st().visual_global.start = curwin_mut().w_cursor;

            curwin_mut().w_cursor.lnum = old_cursor.lnum;
            curwin_mut().w_curswant = right;
            if old_cursor.lnum >= g_st().visual_global.start.lnum
                && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            {
                curwin_mut().w_curswant += 1;
            }
            coladvance(curwin_mut(), curwin_mut().w_curswant);
            if curwin_mut().w_cursor.col == old_cursor.col
                && (!virtual_active(curwin_mut())
                    || curwin_mut().w_cursor.coladd == old_cursor.coladd)
            {
                curwin_mut().w_cursor.lnum = g_st().visual_global.start.lnum;
                if old_cursor.lnum <= g_st().visual_global.start.lnum
                    && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
                {
                    right += 1;
                }
                coladvance(curwin_mut(), right);
                g_st().visual_global.start = curwin_mut().w_cursor;

                curwin_mut().w_cursor.lnum = old_cursor.lnum;
                coladvance(curwin_mut(), left);
                curwin_mut().w_curswant = left;
            }
        } else {
            ptr::swap(
                &raw mut curwin_mut().w_cursor,
                &raw mut g_st().visual_global.start,
            );
            curwin_mut().w_set_curswant = true;
        }
    }
}

fn nv_replace_fn(cap: &mut Cmdarg) {
    {
        if g_st().visual_global.active {
            cap.cmdchar = b'c' as i32;
            cap.nchar = NUL as i32;
            norm_st().visual_mode_orig = g_st().visual_global.mode;
            g_st().visual_global.mode = b'V' as i32;
            nv_operator(cap);
            return;
        }

        if checkclearopq(cap.oap_mut()) {
            return;
        }

        if !modifiable(curbuf_mut()) {
            emsg(tr(E_MODIFIABLE));
        } else {
            if virtual_active(curwin_mut()) {
                coladvance(curwin_mut(), getviscol());
            }
            invoke_edit(
                cap,
                0,
                if cap.arg != 0 {
                    b'V' as i32
                } else {
                    b'R' as i32
                },
                0,
            );
        }
    }
}

fn nv_vreplace(cap: &mut Cmdarg) {
    {
        if g_st().visual_global.active {
            cap.cmdchar = b'r' as i32;
            cap.nchar = cap.extra_char;
            nv_replace(cap);
            return;
        }

        if checkclearopq(cap.oap_mut()) {
            return;
        }

        if !modifiable(curbuf_mut()) {
            emsg(tr(E_MODIFIABLE));
        } else {
            if cap.extra_char == CTRL_V_CONST || cap.extra_char == CTRL_Q_CONST {
                cap.extra_char = get_literal(false);
            }
            if cap.extra_char < b' ' as i32 {
                stuffchar_readbuff(CTRL_V_CONST);
            }
            stuffchar_readbuff(cap.extra_char);
            stuffchar_readbuff(ESC);
            if virtual_active(curwin_mut()) {
                coladvance(curwin_mut(), getviscol());
            }
            invoke_edit(cap, 1, b'v' as i32, 0);
        }
    }
}

fn n_swapchar(cap: &mut Cmdarg) {
    {
        let mut did_change = false;

        if checkclearopq(cap.oap_mut()) {
            return;
        }

        if lineempty(curwin_mut().w_cursor.lnum)
            && !crate::strings::vim_haschr(opt_vars().p_ww, b'~' as i32)
        {
            clearopbeep(cap.oap_mut());
            return;
        }

        prep_redo_cmd(cap);

        if !u_save_cursor() {
            return;
        }

        let startpos = curwin_mut().w_cursor;
        let mut n = cap.count1;
        while n > 0 {
            did_change |= swapchar(cap.oap_mut().op_type, &mut curwin_mut().w_cursor);
            inc_cursor();
            if gchar_cursor() == NUL as i32 {
                if crate::strings::vim_haschr(opt_vars().p_ww, b'~' as i32)
                    && curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count
                {
                    curwin_mut().w_cursor.lnum += 1;
                    curwin_mut().w_cursor.col = 0;
                    if n > 1 {
                        if !u_savesub(curwin_mut().w_cursor.lnum) {
                            break;
                        }
                        u_clearline(curbuf_mut());
                    }
                } else {
                    break;
                }
            }
            n -= 1;
        }

        check_cursor(curwin_mut());
        curwin_mut().w_set_curswant = true;
        if did_change {
            changed_lines(
                curbuf_mut(),
                startpos.lnum,
                startpos.col,
                curwin_mut().w_cursor.lnum + 1,
                0,
                true,
            );
            curbuf_mut().b_op_start = startpos;
            curbuf_mut().b_op_end = curwin_mut().w_cursor;
            if curbuf_mut().b_op_end.col > 0 {
                curbuf_mut().b_op_end.col -= 1;
            }
        }
    }
}

fn nv_mark_move_to(cap: &mut Cmdarg, flags: MarkMove, fm: &mut Fmark) -> MarkMoveRes {
    {
        let res = mark_move_to(&raw mut *fm, flags);
        if (res & K_MARK_MOVE_FAILED) != 0 {
            clearop(cap.oap_mut());
        }
        cap.oap_mut().motion_type = if (flags & K_MARK_BEGIN_LINE) != 0 {
            MT_LINE_WISE
        } else {
            MT_CHAR_WISE
        };
        if cap.cmdchar == b'`' as i32 {
            cap.oap_mut().use_reg_one = true;
        }
        cap.oap_mut().inclusive = false;
        curwin_mut().w_set_curswant = true;
        res
    }
}

fn v_visop(cap: &mut Cmdarg) {
    unsafe {
        let trans = c"YyDdCcxdXdAAIIrr".as_ptr();

        if (cap.cmdchar as u8).is_ascii_uppercase() {
            if g_st().visual_global.mode != CTRL_V_CONST {
                norm_st().visual_mode_orig = g_st().visual_global.mode;
                g_st().visual_global.mode = b'V' as i32;
            } else if cap.cmdchar == b'C' as i32 || cap.cmdchar == b'D' as i32 {
                curwin_mut().w_curswant = MAXCOL;
            }
        }
        cap.cmdchar = i32::from(*vim_strchr(trans, cap.cmdchar).wrapping_add(1) as u8);
        nv_operator(cap);
    }
}

fn nv_subst(cap: &mut Cmdarg) {
    {
        if bt_prompt(Some(crate::globals::curbuf_mut())) && !prompt_curpos_editable() {
            clearopbeep(cap.oap_mut());
            return;
        }
        if g_st().visual_global.active {
            if cap.cmdchar == b'S' as i32 {
                norm_st().visual_mode_orig = g_st().visual_global.mode;
                g_st().visual_global.mode = b'V' as i32;
            }
            cap.cmdchar = b'c' as i32;
            nv_operator(cap);
        } else {
            nv_optrans(cap);
        }
    }
}

fn nv_abbrev(cap: &mut Cmdarg) {
    if cap.cmdchar == K_DEL || cap.cmdchar == K_KDEL {
        cap.cmdchar = b'x' as i32;
    }
    if g_st().visual_global.active {
        v_visop(cap);
    } else {
        nv_optrans(cap);
    }
}

fn nv_optrans(cap: &mut Cmdarg) {
    {
        const AR: [&CStr; 8] = [c"dl", c"dh", c"d$", c"c$", c"cl", c"cc", c"yy", c":s\r"];
        let str_p = c"xXDCsSY&".as_ptr();

        if !checkclearopq(cap.oap_mut()) {
            if cap.count0 != 0 {
                stuffnum_readbuff(cap.count0);
            }
            let idx = crate::memory::cstr_as_bytes(str_p)
                .iter()
                .position(|&b| b == cap.cmdchar as u8)
                .unwrap();
            stuff_readbuff(AR[idx].as_ptr());
            exec_stuffed(Some(cap));
        }
        cap.opcount = 0;
    }
}

fn nv_gomark(cap: &mut Cmdarg) {
    unsafe {
        let mut flags: MarkMove = if (opt_vars().jop_flags & K_OPT_JOP_FLAG_VIEW) != 0 {
            K_MARK_SET_VIEW
        } else {
            0
        };
        if cap.oap_mut().op_type != OP_NOP {
            flags = 0;
        }
        let old_key_typed = g_st().key_typed;

        let name = if cap.cmdchar == b'g' as i32 {
            flags |= KMARK_NO_CONTEXT;
            cap.extra_char
        } else {
            flags |= K_MARK_CONTEXT;
            cap.nchar
        };
        if cap.arg != 0 {
            flags |= K_MARK_BEGIN_LINE;
        }
        if cap.count0 != 0 {
            flags |= K_MARK_SET_VIEW;
        }

        let fm = mark_get(curbuf_mut(), curwin_mut(), None, K_MARK_ALL, name);
        let move_res = nv_mark_move_to(cap, flags, &mut *fm);

        if !virtual_active(curwin_mut()) {
            curwin_mut().w_cursor.coladd = 0;
        }

        if cap.oap_mut().op_type == OP_NOP
            && (move_res & K_MARK_MOVE_SUCCESS) != 0
            && ((move_res & K_MARK_SWITCHED_BUF) != 0 || (move_res & K_MARK_CHANGED_CURSOR) != 0)
            && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_MARK) != 0
            && old_key_typed
        {
            fold_open_cursor();
        }
    }
}

fn nv_pcmark(cap: &mut Cmdarg) {
    unsafe {
        let mut flags: MarkMove = if (opt_vars().jop_flags & K_OPT_JOP_FLAG_VIEW) != 0 {
            K_MARK_SET_VIEW
        } else {
            0
        };
        let mut move_res: MarkMoveRes = 0;
        let old_key_typed = g_st().key_typed;

        if checkclearopq(cap.oap_mut()) {
            return;
        }

        if cap.cmdchar == TAB && g_st().mod_mask == MOD_MASK_CTRL {
            if !goto_tabpage_lastused() {
                clearopbeep(cap.oap_mut());
            }
            return;
        }

        let fm = if cap.cmdchar == b'g' as i32 {
            get_changelist(curbuf_mut(), curwin_mut(), cap.count1)
        } else {
            flags |= KMARK_NO_CONTEXT | K_MARK_JUMP_LIST;
            get_jumplist(curwin_mut(), cap.count1)
        };

        if !fm.is_null() {
            move_res = nv_mark_move_to(cap, flags, &mut *fm);
        } else if cap.cmdchar == b'g' as i32 {
            if curbuf_mut().b_changelistlen == 0 {
                emsg(tr(E_CHANGELIST_IS_EMPTY));
            } else if cap.count1 < 0 {
                emsg(tr(c"E662: At start of changelist"));
            } else {
                emsg(tr(c"E663: At end of changelist"));
            }
        } else {
            clearopbeep(cap.oap_mut());
        }

        if cap.oap_mut().op_type == OP_NOP
            && ((move_res & K_MARK_SWITCHED_BUF) != 0 || (move_res & K_MARK_CHANGED_LINE) != 0)
            && (opt_vars().fdo_flags & K_OPT_FDO_FLAG_MARK) != 0
            && old_key_typed
        {
            fold_open_cursor();
        }
    }
}

fn nv_regname(cap: &mut Cmdarg) {
    {
        if checkclearop(cap.oap_mut()) {
            return;
        }
        if cap.nchar == b'=' as i32 {
            cap.nchar = get_expr_register();
        }
        if cap.nchar != NUL as i32 && valid_yank_reg(cap.nchar, false) {
            cap.oap_mut().regname = cap.nchar;
            cap.opcount = cap.count0;
            set_reg_var(cap.oap_mut().regname);
        } else {
            clearopbeep(cap.oap_mut());
        }
    }
}

fn nv_visual(cap: &mut Cmdarg) {
    {
        if cap.cmdchar == CTRL_Q_CONST {
            cap.cmdchar = CTRL_V_CONST;
        }

        if cap.oap_mut().op_type != OP_NOP {
            cap.oap_mut().motion_force = cap.cmdchar;
            g_st().motion_force = cap.cmdchar;
            g_st().finish_op = false;
            return;
        }

        g_st().visual_global.select = cap.arg != 0;
        if g_st().visual_global.active {
            if g_st().visual_global.mode == cap.cmdchar {
                end_visual_mode();
            } else {
                g_st().visual_global.mode = cap.cmdchar;
                showmode();
                may_trigger_modechanged();
            }
            redraw_curbuf_later(UPD_INVERTED);
        } else if cap.count0 > 0 && g_st().visual_global.resel.mode != NUL as i32 {
            g_st().visual_global.start = curwin_mut().w_cursor;

            g_st().visual_global.active = true;
            g_st().visual_global.reselect = 1;
            if cap.arg == 0 {
                may_start_select(b'c' as i32);
            }
            setmouse();
            if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
                g_st().redraw_cmdline = true;
            }
            if g_st().visual_global.resel.mode != b'v' as i32
                || g_st().visual_global.resel.line_count > 1
            {
                curwin_mut().w_cursor.lnum +=
                    g_st().visual_global.resel.line_count * cap.count0 - 1;
                check_cursor(curwin_mut());
            }
            g_st().visual_global.mode = g_st().visual_global.resel.mode;
            if g_st().visual_global.mode == b'v' as i32 {
                if g_st().visual_global.resel.line_count <= 1 {
                    update_curswant_force();
                    curwin_mut().w_curswant += g_st().visual_global.resel.vcol * cap.count0;
                    if crate::memory::cstr_first_byte(opt_vars().p_sel) != b'e' {
                        curwin_mut().w_curswant -= 1;
                    }
                } else {
                    curwin_mut().w_curswant = g_st().visual_global.resel.vcol;
                }
                coladvance(curwin_mut(), curwin_mut().w_curswant);
            }
            if g_st().visual_global.resel.vcol == MAXCOL {
                curwin_mut().w_curswant = MAXCOL;
                coladvance(curwin_mut(), MAXCOL);
            } else if g_st().visual_global.mode == CTRL_V_CONST {
                let tmp_cursor = curwin_mut().w_cursor;
                curwin_mut().w_cursor = g_st().visual_global.start;
                update_curswant_force();
                curwin_mut().w_curswant += g_st().visual_global.resel.vcol * cap.count0 - 1;
                curwin_mut().w_cursor = tmp_cursor;
                if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                    curwin_mut().w_curswant += 1;
                }
                coladvance(curwin_mut(), curwin_mut().w_curswant);
            } else {
                curwin_mut().w_set_curswant = true;
            }
            redraw_curbuf_later(UPD_INVERTED);
        } else {
            if cap.arg == 0 {
                may_start_select(b'c' as i32);
            }
            n_start_visual_mode(cap.cmdchar);
            if g_st().visual_global.mode != b'V' as i32
                && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            {
                cap.count1 += 1;
            } else {
                g_st().visual_global.select_exclu_adj = false;
            }
            if cap.count0 > 0 {
                cap.count1 -= 1;
                if cap.count1 > 0 {
                    if g_st().visual_global.mode == b'v' as i32
                        || g_st().visual_global.mode == CTRL_V_CONST
                    {
                        nv_right(cap);
                    } else if g_st().visual_global.mode == b'V' as i32 {
                        nv_down(cap);
                    }
                }
            }
        }
    }
}

pub fn start_selection() {
    may_start_select(b'k' as i32);
    n_start_visual_mode(b'v' as i32);
}

pub fn may_start_select(c: i32) {
    g_st().visual_global.select = (c == b'o' as i32 || (stuff_empty() && typebuf_typed() != 0))
        && crate::strings::vim_haschr(opt_vars().p_slm, c);
}

fn n_start_visual_mode(c: i32) {
    {
        g_st().visual_global.mode = c;
        g_st().visual_global.active = true;
        g_st().visual_global.reselect = 1;
        if c == CTRL_V_CONST
            && (get_ve_flags(curwin_mut()) & K_OPT_VE_FLAG_BLOCK) != 0
            && gchar_cursor() == TAB
        {
            validate_virtcol(curwin_mut());
            coladvance(curwin_mut(), curwin_mut().w_virtcol);
        }
        g_st().visual_global.start = curwin_mut().w_cursor;

        fold_adjust_visual();

        may_trigger_modechanged();
        setmouse();
        conceal_check_cursor_line();

        if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
            g_st().redraw_cmdline = true;
        }
        if curwin_mut().w_redr_type < UPD_INVERTED {
            curwin_mut().w_old_cursor_lnum = curwin_mut().w_cursor.lnum;
            curwin_mut().w_old_visual_lnum = curwin_mut().w_cursor.lnum;
        }
        redraw_curbuf_later(UPD_VALID);
    }
}

fn nv_window(cap: &mut Cmdarg) {
    {
        if cap.nchar == b':' as i32 {
            cap.cmdchar = b':' as i32;
            cap.nchar = NUL as i32;
            nv_colon(cap);
        } else if !checkclearop(cap.oap_mut()) {
            do_window(cap.nchar, cap.count0, NUL as i32);
        }
    }
}

fn nv_suspend(cap: &mut Cmdarg) {
    {
        clearop(cap.oap_mut());
        if g_st().visual_global.active {
            end_visual_mode();
        }
        do_cmdline_cmd(c"stop");
    }
}

fn nv_gv_cmd(cap: &mut Cmdarg) {
    {
        if curbuf_mut().b_visual.vi_start.lnum == 0
            || curbuf_mut().b_visual.vi_start.lnum > curbuf_mut().b_ml.ml_line_count
            || curbuf_mut().b_visual.vi_end.lnum == 0
        {
            beep_flush();
            return;
        }

        let prev = curbuf_mut().b_visual;
        if g_st().visual_global.active {
            curbuf_mut().b_visual = visualinfo();
            curbuf_mut().b_visual_mode_eval = g_st().visual_global.mode;
        }
        g_st().visual_global.mode = prev.vi_mode;
        curwin_mut().w_curswant = prev.vi_curswant;
        let tpos = prev.vi_end;
        curwin_mut().w_cursor = prev.vi_start;

        g_st().visual_global.active = true;
        g_st().visual_global.reselect = 1;

        check_cursor(curwin_mut());
        g_st().visual_global.start = curwin_mut().w_cursor;
        curwin_mut().w_cursor = tpos;
        check_cursor(curwin_mut());
        update_topline(curwin_mut());

        if cap.arg != 0 {
            g_st().visual_global.select = true;
            g_st().visual_global.select_reg = 0;
        } else {
            may_start_select(b'c' as i32);
        }
        setmouse();
        redraw_curbuf_later(UPD_INVERTED);
        showmode();
    }
}

pub fn nv_g_home_m_cmd(cap: &mut Cmdarg) {
    unsafe {
        let mut i: i32;
        let flag = cap.nchar == b'^' as i32;

        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        if curwin_mut().w_onebuf_opt.wo_wrap != 0 && curwin_mut().w_view_width != 0 {
            let width1 = curwin_mut().w_view_width - win_col_off(curwin_ptr());
            let width2 = width1 + win_col_off2(curwin_mut());

            validate_virtcol(curwin_mut());
            i = 0;
            if curwin_mut().w_virtcol >= width1 && width2 > 0 {
                i = (curwin_mut().w_virtcol - width1) / width2 * width2 + width1;
            }

            if curwin_mut().w_skipcol > 0 && curwin_mut().w_cursor.lnum == curwin_mut().w_topline {
                let overlap = crate::r#move::sms_marker_overlap(
                    &mut *(curwin_ptr()),
                    curwin_mut().w_view_width - width2,
                );
                if overlap > 0 && i == curwin_mut().w_skipcol {
                    i += overlap;
                }
            }
        } else {
            i = curwin_mut().w_leftcol;
        }
        if cap.nchar == b'm' as i32 {
            i += (curwin_mut().w_view_width - win_col_off(curwin_ptr())
                + if curwin_mut().w_onebuf_opt.wo_wrap != 0 && i > 0 {
                    win_col_off2(curwin_mut())
                } else {
                    0
                })
                / 2;
        }
        coladvance(curwin_mut(), i);
        if flag {
            loop {
                i = gchar_cursor();
                if !(ascii_iswhite(i) && oneright()) {
                    break;
                }
            }
            curwin_mut().w_valid &= !VALID_WCOL;
        }
        curwin_mut().w_set_curswant = true;
        if has_any_folding(curwin_mut()) != 0 {
            validate_cheight(curwin_mut());
            if curwin_mut().w_cline_folded {
                update_curswant_force();
            }
        }
        adjust_skipcol();
    }
}

fn nv_g_underscore_cmd(cap: &mut Cmdarg) {
    unsafe {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = true;
        curwin_mut().w_curswant = MAXCOL;
        if !cursor_down(cap.count1 - 1, cap.oap_mut().op_type == OP_NOP) {
            clearopbeep(cap.oap_mut());
            return;
        }

        let ptr = get_cursor_line_ptr();

        if curwin_mut().w_cursor.col > 0
            && *ptr.wrapping_add(curwin_mut().w_cursor.col as usize) == NUL
        {
            curwin_mut().w_cursor.col -= 1;
        }

        while curwin_mut().w_cursor.col > 0
            && ascii_iswhite(i32::from(
                *ptr.wrapping_add(curwin_mut().w_cursor.col as usize) as u8,
            ))
        {
            curwin_mut().w_cursor.col -= 1;
        }
        curwin_mut().w_set_curswant = true;
        adjust_for_sel(cap);
    }
}

fn nv_g_dollar_cmd(cap: &mut Cmdarg) {
    unsafe {
        let oap = &mut *cap.oap;
        let mut i: i32;
        let col_off = win_col_off(curwin_ptr());
        let flag = cap.nchar == K_END || cap.nchar == K_KEND;

        oap.motion_type = MT_CHAR_WISE;
        oap.inclusive = true;
        if curwin_mut().w_onebuf_opt.wo_wrap != 0 && curwin_mut().w_view_width != 0 {
            curwin_mut().w_curswant = MAXCOL;
            if cap.count1 == 1 {
                let width1 = curwin_mut().w_view_width - col_off;
                let width2 = width1 + win_col_off2(curwin_mut());

                validate_virtcol(curwin_mut());
                i = width1 - 1;
                if curwin_mut().w_virtcol >= width1 {
                    i += ((curwin_mut().w_virtcol - width1) / width2 + 1) * width2;
                }
                coladvance(curwin_mut(), i);

                update_curswant_force();
                if curwin_mut().w_cursor.col > 0
                    && curwin_mut().w_onebuf_opt.wo_wrap != 0
                    && curwin_mut().w_virtcol > i
                {
                    curwin_mut().w_cursor.col -= 1;
                }
            } else if !nv_screengo(&mut *oap, FORWARD, cap.count1 - 1, false) {
                clearopbeep(&mut *oap);
            }
        } else {
            if cap.count1 > 1 {
                cursor_down(cap.count1 - 1, false);
            }
            i = curwin_mut().w_leftcol + curwin_mut().w_view_width - col_off - 1;
            coladvance(curwin_mut(), i);

            if curwin_mut().w_cursor.col > 0 && utf_ptr2cells(get_cursor_pos_ptr()) > 1 {
                let mut vcol: Colnr = 0;
                getvvcol(
                    &mut *(curwin_ptr()),
                    &mut curwin_mut().w_cursor,
                    None,
                    None,
                    Some(&mut vcol),
                    0,
                );
                if vcol >= curwin_mut().w_leftcol + curwin_mut().w_view_width - col_off {
                    curwin_mut().w_cursor.col -= 1;
                }
            }

            update_curswant_force();
        }
        if flag {
            loop {
                i = gchar_cursor();
                if !(ascii_iswhite_or_nul(i) && oneleft()) {
                    break;
                }
            }
            curwin_mut().w_valid &= !VALID_WCOL;
        }
    }
}

fn nv_gi_cmd(cap: &mut Cmdarg) {
    {
        if curbuf_mut().b_last_insert.mark.lnum != 0 {
            curwin_mut().w_cursor = curbuf_mut().b_last_insert.mark;
            check_cursor_lnum(curwin_mut());
            let i = get_cursor_line_len();
            if curwin_mut().w_cursor.col > i {
                if virtual_active(curwin_mut()) {
                    curwin_mut().w_cursor.coladd += curwin_mut().w_cursor.col - i;
                }
                curwin_mut().w_cursor.col = i;
            }
        }
        cap.cmdchar = b'i' as i32;
        nv_edit(cap);
    }
}

fn nv_g_cmd(cap: &mut Cmdarg) {
    unsafe {
        let oap = &mut *cap.oap;

        match cap.nchar {
            CTRL_A | CTRL_X_CONST => {
                if g_st().visual_global.active {
                    cap.arg = 1;
                    cap.cmdchar = cap.nchar;
                    cap.nchar = NUL as i32;
                    nv_addsub(cap);
                } else if cap.nchar == CTRL_A
                    && cap.oap_mut().op_type == OP_NOP
                    && mc_buf_has_cursors(curbuf_mut())
                {
                    mc_counter(i64::from(cap.count1));
                } else {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'R' as i32 => {
                cap.arg = 1;
                nv_replace_fn(cap);
            }
            c if c == b'r' as i32 => {
                nv_vreplace(cap);
            }
            c if c == b'&' as i32 => {
                do_cmdline_cmd(c"%s//~/&");
            }
            c if c == b'v' as i32 => {
                nv_gv_cmd(cap);
            }
            c if c == b'V' as i32 => {
                g_st().visual_global.reselect = 0;
            }
            c if c == b'Q' as i32 => {
                if g_st().reg_recording != 0 || g_st().reg_executing != 0 {
                    vim_beep(0);
                } else {
                    let mut tv_args: [TypVal; 1] = [TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: 0,
                        vval: TypvalVval::Number(0),
                    }];
                    nlua_call_typval(c"vim._core.mcursor", c"restore", tv_args.as_mut_ptr(), None);
                }
            }
            K_BS => {
                cap.nchar = CTRL_H_CONST;
                cap.cmdchar = cap.nchar + (b'v' as i32 - b'h' as i32);
                cap.arg = 1;
                nv_visual(cap);
            }
            c if c == b'h' as i32 || c == b'H' as i32 || c == CTRL_H_CONST => {
                cap.cmdchar = cap.nchar + (b'v' as i32 - b'h' as i32);
                cap.arg = 1;
                nv_visual(cap);
            }
            c if c == b'N' as i32 || c == b'n' as i32 => {
                if !current_search(cap.count1, cap.nchar == b'n' as i32) {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'j' as i32 || c == K_DOWN => {
                let ok = if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
                    oap.motion_type = MT_LINE_WISE;
                    cursor_down(cap.count1, oap.op_type == OP_NOP)
                } else {
                    nv_screengo(&mut *oap, FORWARD, cap.count1, false)
                };
                if !ok {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'k' as i32 || c == K_UP => {
                let ok = if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
                    oap.motion_type = MT_LINE_WISE;
                    cursor_up(cap.count1, oap.op_type == OP_NOP)
                } else {
                    nv_screengo(&mut *oap, BACKWARD, cap.count1, false)
                };
                if !ok {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'J' as i32 => {
                nv_join(cap);
            }
            c if c == b'^' as i32
                || c == b'0' as i32
                || c == b'm' as i32
                || c == K_HOME
                || c == K_KHOME =>
            {
                nv_g_home_m_cmd(cap);
            }
            c if c == b'M' as i32 => {
                oap.motion_type = MT_CHAR_WISE;
                oap.inclusive = false;
                let i = linetabsize(curwin_mut(), curwin_mut().w_cursor.lnum);
                if cap.count0 > 0 && cap.count0 <= 100 {
                    coladvance(curwin_mut(), i * cap.count0 / 100);
                } else {
                    coladvance(curwin_mut(), i / 2);
                }
                curwin_mut().w_set_curswant = true;
            }
            c if c == b'_' as i32 => {
                nv_g_underscore_cmd(cap);
            }
            c if c == b'$' as i32 || c == K_END || c == K_KEND => {
                nv_g_dollar_cmd(cap);
            }
            c if c == b'*' as i32
                || c == b'#' as i32
                || c == POUND
                || c == CTRL_RSB_CONST
                || c == b']' as i32 =>
            {
                nv_ident(cap);
            }
            c if c == b'e' as i32 || c == b'E' as i32 => {
                oap.motion_type = MT_CHAR_WISE;
                curwin_mut().w_set_curswant = true;
                oap.inclusive = true;
                if !bckend_word(cap.count1, cap.nchar == b'E' as i32, false) {
                    clearopbeep(&mut *oap);
                }
            }
            CTRL_G => {
                cursor_pos_info(None);
            }
            c if c == b'i' as i32 => {
                nv_gi_cmd(cap);
            }
            c if c == b'I' as i32 => {
                beginline(0);
                if !checkclearopq(&mut *oap) {
                    invoke_edit(cap, 0, b'g' as i32, 0);
                }
            }
            c if c == b'f' as i32 || c == b'F' as i32 => {
                nv_gotofile(cap);
            }
            c if c == b'\'' as i32 => {
                cap.arg = 1;
                nv_gomark(cap);
            }
            c if c == b'`' as i32 => {
                nv_gomark(cap);
            }
            c if c == b's' as i32 => {
                do_sleep(i64::from(cap.count1) * 1000, false);
            }
            c if c == b'a' as i32 => {
                do_ascii(&mut crate::ex_docmd::Exarg::new());
            }
            c if c == b'8' as i32 => {
                if cap.count0 == 8 {
                    utf_find_illegal();
                } else {
                    show_utf8();
                }
            }
            c if c == b'<' as i32 => {
                show_sb_text();
            }
            c if c == b'g' as i32 => {
                cap.arg = 0;
                nv_goto(cap);
            }
            c if c == b'q' as i32 || c == b'w' as i32 => {
                oap.cursor_start = curwin_mut().w_cursor;
                nv_operator(cap);
            }
            c if c == b'~' as i32
                || c == b'u' as i32
                || c == b'U' as i32
                || c == b'?' as i32
                || c == b'@' as i32 =>
            {
                nv_operator(cap);
            }
            c if c == b'd' as i32 || c == b'D' as i32 => {
                nv_gd(&mut *oap, cap.nchar, cap.count0);
            }
            K_LEFTMOUSE => {
                if do_mouse(Some(&mut *oap), cap.nchar, BACKWARD, cap.count1, false) {
                    stuffchar_readbuff(CTRL_RSB_CONST);
                    exec_stuffed(Some(cap));
                }
            }
            K_MIDDLEMOUSE | K_MIDDLEDRAG | K_MIDDLERELEASE | K_LEFTDRAG | K_LEFTRELEASE
            | K_MOUSEMOVE | K_RIGHTMOUSE | K_RIGHTDRAG | K_RIGHTRELEASE | K_X1MOUSE | K_X1DRAG
            | K_X1RELEASE | K_X2MOUSE | K_X2DRAG | K_X2RELEASE => {
                g_st().mod_mask = MOD_MASK_CTRL;
                do_mouse(Some(&mut *oap), cap.nchar, BACKWARD, cap.count1, false);
            }
            K_IGNORE => {}
            c if c == b'p' as i32 || c == b'P' as i32 => {
                nv_put(cap);
            }
            c if c == b'o' as i32 => {
                oap.inclusive = false;
                goto_byte(cap.count0);
            }
            c if c == b',' as i32 => {
                nv_pcmark(cap);
            }
            c if c == b';' as i32 => {
                cap.count1 = -cap.count1;
                nv_pcmark(cap);
            }
            c if c == b't' as i32 => {
                if !checkclearop(&mut *oap) {
                    goto_tabpage(cap.count0);
                }
            }
            c if c == b'T' as i32 => {
                if !checkclearop(&mut *oap) {
                    goto_tabpage(-cap.count1);
                }
            }
            TAB => {
                if !checkclearop(&mut *oap) && !goto_tabpage_lastused() {
                    clearopbeep(&mut *oap);
                }
            }
            c if c == b'+' as i32 || c == b'-' as i32 => {
                if !checkclearopq(&mut *oap) {
                    undo_time(
                        if cap.nchar == b'-' as i32 {
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

fn n_opencmd(cap: &mut Cmdarg) {
    {
        if checkclearopq(cap.oap_mut()) {
            return;
        }

        if cap.cmdchar == b'O' as i32 {
            has_folding(
                curwin_ptr(),
                curwin_mut().w_cursor.lnum,
                Some(&mut curwin_mut().w_cursor.lnum),
                ptr::null_mut(),
            );
        } else {
            has_folding(
                curwin_ptr(),
                curwin_mut().w_cursor.lnum,
                None,
                &raw mut curwin_mut().w_cursor.lnum,
            );
        }
        curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());
        if u_save(
            curwin_mut().w_cursor.lnum - i32::from(cap.cmdchar == b'O' as i32),
            curwin_mut().w_cursor.lnum + i32::from(cap.cmdchar == b'o' as i32),
        ) && open_line(
            if cap.cmdchar == b'O' as i32 {
                BACKWARD
            } else {
                FORWARD
            },
            if has_format_option(K_FO_OPEN_COMS) {
                OPENLINE_DO_COM
            } else {
                0
            },
            0,
            None,
        ) {
            if win_cursorline_standout(curwin_mut()) {
                curwin_mut().w_valid &= !VALID_CROW;
            }
            invoke_edit(cap, 0, cap.cmdchar, 1);
        }
    }
}

fn nv_dot(cap: &mut Cmdarg) {
    {
        if checkclearopq(cap.oap_mut()) {
            return;
        }

        if !start_redo(
            cap.count0,
            g_st().restart_edit != 0 && g_st().ins_global.moved == K_INS_NONE,
        ) {
            clearopbeep(cap.oap_mut());
            return;
        }
        exec_stuffed(Some(cap));
    }
}

fn nv_redo_or_register(cap: &mut Cmdarg) {
    {
        if g_st().visual_global.select && g_st().visual_global.active {
            g_st().no_mapping += 1;
            let mut reg = plain_vgetc();
            langmap_adjust(&mut reg, true);
            g_st().no_mapping -= 1;

            if reg == b'"' as i32 {
                reg = 0;
            }

            g_st().visual_global.select_reg = if valid_yank_reg(reg, true) { reg } else { 0 };
            return;
        }

        if checkclearopq(cap.oap_mut()) {
            return;
        }

        u_redo(cap.count1);
        curwin_mut().w_set_curswant = true;
    }
}

fn nv_undo_fn(cap: &mut Cmdarg) {
    {
        if cap.oap_mut().op_type == OP_UPPER || g_st().visual_global.active {
            cap.cmdchar = b'g' as i32;
            cap.nchar = b'U' as i32;
            nv_operator(cap);
            return;
        }

        if checkclearopq(cap.oap_mut()) {
            return;
        }

        u_undoline();
        curwin_mut().w_set_curswant = true;
    }
}

fn nv_tilde(cap: &mut Cmdarg) {
    {
        if opt_vars().p_to == 0 && !g_st().visual_global.active && cap.oap_mut().op_type != OP_TILDE
        {
            if bt_prompt(Some(crate::globals::curbuf_mut())) && !prompt_curpos_editable() {
                clearopbeep(cap.oap_mut());
                return;
            }
            n_swapchar(cap);
        } else {
            nv_operator(cap);
        }
    }
}

fn nv_operator(cap: &mut Cmdarg) {
    {
        let op_type = get_op_type(cap.cmdchar, cap.nchar);

        if bt_prompt(Some(crate::globals::curbuf_mut()))
            && op_is_change(op_type) != 0
            && !prompt_curpos_editable()
        {
            clearopbeep(cap.oap_mut());
            return;
        }

        if op_type == cap.oap_mut().op_type {
            nv_lineop(cap);
        } else if !checkclearop(cap.oap_mut()) {
            cap.oap_mut().start = curwin_mut().w_cursor;
            cap.oap_mut().op_type = op_type;
            set_op_var(op_type);
        }
    }
}

fn set_op_var(optype: i32) {
    if optype == OP_NOP {
        set_vim_var_string(VV_OP, ptr::null(), 0);
    } else {
        let opchar0 = get_op_char(optype);
        assert!((0..=u8::MAX as i32).contains(&opchar0));
        let opchar1 = get_extra_op_char(optype);
        assert!((0..=u8::MAX as i32).contains(&opchar1));
        let opchars: [c_char; 3] = [opchar0 as c_char, opchar1 as c_char, NUL];
        set_vim_var_string(VV_OP, opchars.as_ptr(), 2);
    }
}

fn nv_lineop(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_LINE_WISE;
        if !cursor_down(cap.count1 - 1, cap.oap_mut().op_type == OP_NOP) {
            clearopbeep(cap.oap_mut());
        } else if (cap.oap_mut().op_type == OP_DELETE
            && cap.oap_mut().motion_force != b'v' as i32
            && cap.oap_mut().motion_force != CTRL_V_CONST)
            || cap.oap_mut().op_type == OP_LSHIFT
            || cap.oap_mut().op_type == OP_RSHIFT
        {
            beginline(BL_SOL | BL_FIX);
        } else if cap.oap_mut().op_type != OP_YANK {
            beginline(BL_WHITE | BL_FIX);
        }
    }
}

fn nv_home(cap: &mut Cmdarg) {
    if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
        nv_goto(cap);
    } else {
        cap.count0 = 1;
        nv_pipe(cap);
    }
    g_st().ins_at_eol = false;
}

fn nv_pipe(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        curwin_mut().w_cursor.col = 0;
        curwin_mut().w_cursor.coladd = 0;
        if cap.count0 > 0 {
            coladvance(curwin_mut(), cap.count0 - 1);
            curwin_mut().w_curswant = cap.count0 - 1;
        } else {
            curwin_mut().w_curswant = 0;
        }
        curwin_mut().w_set_curswant = false;
        adjust_skipcol();
    }
}

fn nv_bck_word(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        curwin_mut().w_set_curswant = true;
        if !bck_word(cap.count1, cap.arg != 0, false) {
            clearopbeep(cap.oap_mut());
        } else if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_wordcmd(cap: &mut Cmdarg) {
    {
        let mut word_end = cap.cmdchar == b'e' as i32 || cap.cmdchar == b'E' as i32;
        let mut flag = false;
        let startpos = curwin_mut().w_cursor;

        cap.oap_mut().inclusive = word_end;

        if !word_end && cap.oap_mut().op_type == OP_CHANGE {
            let n = gchar_cursor();
            if n != NUL as i32 && !ascii_iswhite(n) {
                if crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_CHANGEW) {
                    cap.oap_mut().inclusive = true;
                    word_end = true;
                }
                flag = true;
            }
        }

        cap.oap_mut().motion_type = MT_CHAR_WISE;
        curwin_mut().w_set_curswant = true;
        let n = if word_end {
            end_word(cap.count1, cap.arg != 0, flag, false)
        } else {
            fwd_word(cap.count1, cap.arg != 0, cap.oap_mut().op_type != OP_NOP)
        };

        if lt(startpos, curwin_mut().w_cursor) {
            adjust_cursor(cap.oap_mut());
        }

        if !n && cap.oap_mut().op_type == OP_NOP {
            clearopbeep(cap.oap_mut());
        } else {
            adjust_for_sel(cap);
            if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
                && g_st().key_typed
                && cap.oap_mut().op_type == OP_NOP
            {
                fold_open_cursor();
            }
        }
    }
}

fn adjust_cursor(oap: &mut Oparg) {
    {
        if curwin_mut().w_cursor.col > 0
            && gchar_cursor() == NUL as i32
            && (!g_st().visual_global.active
                || crate::memory::cstr_first_byte(opt_vars().p_sel) == b'o')
            && !virtual_active(curwin_mut())
            && (get_ve_flags(curwin_mut()) & K_OPT_VE_FLAG_ONEMORE) == 0
        {
            curwin_mut().w_cursor.col -= 1;
            mb_adjust_cursor();
            oap.inclusive = true;
        }
    }
}

fn nv_beginline(cap: &mut Cmdarg) {
    {
        cap.oap_mut().motion_type = MT_CHAR_WISE;
        cap.oap_mut().inclusive = false;
        beginline(cap.arg);
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
        g_st().ins_at_eol = false;
    }
}

fn adjust_for_sel(cap: &mut Cmdarg) {
    {
        if g_st().visual_global.active
            && cap.oap_mut().inclusive
            && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            && gchar_cursor() != NUL as i32
            && lt(g_st().visual_global.start, curwin_mut().w_cursor)
        {
            inc_cursor();
            cap.oap_mut().inclusive = false;
            g_st().visual_global.select_exclu_adj = true;
        }
    }
}

pub fn unadjust_for_sel() -> bool {
    {
        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            && !equalpos(g_st().visual_global.start, curwin_mut().w_cursor)
        {
            return unadjust_for_sel_inner(
                if lt(g_st().visual_global.start, curwin_mut().w_cursor) {
                    &mut curwin_mut().w_cursor
                } else {
                    &mut g_st().visual_global.start
                },
            );
        }
        false
    }
}

pub fn unadjust_for_sel_inner(pp: &mut CursorPos) -> bool {
    {
        g_st().visual_global.select_exclu_adj = false;

        if pp.coladd > 0 {
            pp.coladd -= 1;
        } else if pp.col > 0 {
            pp.col -= 1;
            mark_mb_adjustpos(g_st().curbuf, pp);
            if virtual_active(curwin_mut()) {
                let mut cs: Colnr = 0;
                let mut ce: Colnr = 0;
                getvcol(curwin_mut(), pp, Some(&mut cs), None, Some(&mut ce), 0);
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

fn nv_select(cap: &mut Cmdarg) {
    if g_st().visual_global.active {
        g_st().visual_global.select = true;
        g_st().visual_global.select_reg = 0;
    } else if g_st().visual_global.reselect != 0 {
        cap.nchar = b'v' as i32;
        cap.arg = 1;
        nv_g_cmd(cap);
    }
}

fn nv_goto(cap: &mut Cmdarg) {
    {
        let mut lnum = if cap.arg != 0 {
            curbuf_mut().b_ml.ml_line_count
        } else {
            1
        };
        cap.oap_mut().motion_type = MT_LINE_WISE;
        setpcmark();

        if cap.count0 != 0 {
            lnum = cap.count0;
        }
        lnum = lnum.max(1).min(curbuf_mut().b_ml.ml_line_count);
        curwin_mut().w_cursor.lnum = lnum;
        beginline(BL_SOL | BL_FIX);
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_JUMP) != 0
            && g_st().key_typed
            && cap.oap_mut().op_type == OP_NOP
        {
            fold_open_cursor();
        }
    }
}

fn nv_normal(cap: &mut Cmdarg) {
    {
        if cap.nchar == CTRL_N || cap.nchar == CTRL_G {
            clearop(cap.oap_mut());
            if g_st().restart_edit != 0 && g_st().mode_displayed {
                g_st().clear_cmdline = true;
            }
            g_st().restart_edit = 0;
            if g_st().visual_global.active {
                end_visual_mode();
                redraw_curbuf_later(UPD_INVERTED);
            }
        } else {
            clearopbeep(cap.oap_mut());
        }
    }
}

fn nv_esc(cap: &mut Cmdarg) {
    {
        let no_reason = cap.oap_mut().op_type == OP_NOP
            && cap.opcount == 0
            && cap.count0 == 0
            && cap.oap_mut().regname == 0;
        let cmdwin_cancel = cap.arg != 0 && bt_cmdwin(g_st().curbuf);

        if cmdwin_cancel {
            g_st().got_int = false;
            cmdwin_do_action(c"cancel");
        } else if cap.arg != 0 {
            if g_st().restart_edit == 0 && !g_st().visual_global.active && no_reason {
                if any_buf_is_changed() {
                    msg(
                        tr(c"Type  :qa!  and press <Enter> to abandon all changes and exit Nvim"),
                        0,
                    );
                } else {
                    msg(tr(c"Type  :qa  and press <Enter> to exit Nvim"), 0);
                }
            }

            if g_st().restart_edit != 0 {
                g_st().redraw_mode = true;
            }

            g_st().restart_edit = 0;
        }

        if g_st().visual_global.active {
            end_visual_mode();
            check_cursor_col(curwin_mut());
            curwin_mut().w_set_curswant = true;
            redraw_curbuf_later(UPD_INVERTED);
        } else if no_reason && !cmdwin_cancel {
            vim_beep(K_OPT_BO_FLAG_ESC);
        }
        clearop(cap.oap_mut());
    }
}

pub fn set_cursor_for_append_to_line() {
    {
        curwin_mut().w_set_curswant = true;
        if get_ve_flags(curwin_mut()) == K_OPT_VE_FLAG_ALL {
            let save_state = g_st().state_global;
            g_st().state_global = MODE_INSERT;
            coladvance(curwin_mut(), MAXCOL);
            g_st().state_global = save_state;
        } else {
            curwin_mut().w_cursor.col += crate::memory::cstr_len(get_cursor_pos_ptr()) as Colnr;
        }
    }
}

fn nv_edit(cap: &mut Cmdarg) {
    {
        if cap.cmdchar == K_INS || cap.cmdchar == K_KINS {
            cap.cmdchar = b'i' as i32;
        }

        if g_st().visual_global.active && (cap.cmdchar == b'A' as i32 || cap.cmdchar == b'I' as i32)
        {
            v_visop(cap);
        } else if (cap.cmdchar == b'a' as i32 || cap.cmdchar == b'i' as i32)
            && (cap.oap_mut().op_type != OP_NOP || g_st().visual_global.active)
        {
            nv_object(cap);
        } else if curbuf_mut().b_p_ma == 0 && curbuf_mut().terminal.is_null() {
            emsg(tr(E_MODIFIABLE));
            clearop(cap.oap_mut());
        } else if !checkclearopq(cap.oap_mut()) {
            match cap.cmdchar {
                c if c == b'A' as i32 => {
                    set_cursor_for_append_to_line();
                }
                c if c == b'I' as i32 => {
                    beginline(BL_WHITE);
                }
                c if c == b'a' as i32 => {
                    if virtual_active(curwin_mut())
                        && (curwin_mut().w_cursor.coladd > 0
                            || crate::memory::cstr_is_empty(get_cursor_pos_ptr())
                            || crate::memory::cstr_first_byte(get_cursor_pos_ptr()) == b'\t')
                    {
                        curwin_mut().w_cursor.coladd += 1;
                    } else if !crate::memory::cstr_is_empty(get_cursor_pos_ptr()) {
                        inc_cursor();
                    }
                }
                _ => {}
            }

            if curwin_mut().w_cursor.coladd != 0 && cap.cmdchar != b'A' as i32 {
                let save_state = g_st().state_global;
                g_st().state_global = MODE_INSERT;
                coladvance(curwin_mut(), getviscol());
                g_st().state_global = save_state;
            }

            invoke_edit(cap, 0, cap.cmdchar, 0);
        }
    }
}

fn invoke_edit(cap: &mut Cmdarg, repl: i32, cmd: i32, startln: i32) {
    {
        let restart_edit_save = if repl != 0 || !stuff_empty() {
            g_st().restart_edit
        } else {
            0
        };

        g_st().restart_edit = 0;

        if cap.cmdchar != b'O' as i32 && cap.cmdchar != b'o' as i32 {
            curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());
        }
        let mut session: InsSession =
            atom_ins_start(cmd, i64::from(cap.count1), K_VINS_NONE, false);
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

fn nv_object(cap: &mut Cmdarg) {
    {
        let include = cap.cmdchar != b'i' as i32;
        let mps_save = curbuf_mut().b_p_mps;
        curbuf_mut().b_p_mps = c"(:),{:},[:],<:>".as_ptr().cast_mut();

        let flag = match cap.nchar {
            c if c == b'w' as i32 => current_word(cap.oap_mut(), cap.count1, include, false),
            c if c == b'W' as i32 => current_word(cap.oap_mut(), cap.count1, include, true),
            c if c == b'b' as i32 || c == b'(' as i32 || c == b')' as i32 => {
                current_block(cap.oap_mut(), cap.count1, include, b'(' as i32, b')' as i32)
            }
            c if c == b'B' as i32 || c == b'{' as i32 || c == b'}' as i32 => {
                current_block(cap.oap_mut(), cap.count1, include, b'{' as i32, b'}' as i32)
            }
            c if c == b'[' as i32 || c == b']' as i32 => {
                current_block(cap.oap_mut(), cap.count1, include, b'[' as i32, b']' as i32)
            }
            c if c == b'<' as i32 || c == b'>' as i32 => {
                current_block(cap.oap_mut(), cap.count1, include, b'<' as i32, b'>' as i32)
            }
            c if c == b't' as i32 => {
                cap.retval |= CA_NO_ADJ_OP_END;
                current_tagblock(cap.oap_mut(), cap.count1, include)
            }
            c if c == b'p' as i32 => current_par(cap.oap_mut(), cap.count1, include, b'p' as i32),
            c if c == b'l' as i32 => current_line(cap.oap_mut(), include),
            c if c == b's' as i32 => current_sent(cap.oap_mut(), cap.count1, include),
            c if c == b'"' as i32 || c == b'\'' as i32 || c == b'`' as i32 => {
                current_quote(cap.oap_mut(), cap.count1, include, cap.nchar)
            }
            _ => false,
        };

        curbuf_mut().b_p_mps = mps_save;
        if !flag {
            clearopbeep(cap.oap_mut());
        }
        adjust_cursor_col();
        curwin_mut().w_set_curswant = true;
    }
}

fn nv_q(cap: &mut Cmdarg) {
    {
        if cap.oap_mut().op_type == OP_FORMAT {
            cap.cmdchar = b'g' as i32;
            cap.nchar = b'q' as i32;
            nv_operator(cap);
            return;
        }

        if checkclearop(cap.oap_mut()) {
            return;
        }

        if cap.nchar == b'=' as i32 && cap.count0 > 2 {
            clearopbeep(cap.oap_mut());
        } else if cap.nchar == b'=' as i32 {
            mc_follow_set(if cap.count0 == 0 {
                K_NONE
            } else if cap.count0 == 1 {
                K_TRUE
            } else {
                K_FALSE
            });
        } else if cap.nchar == b':' as i32 || cap.nchar == b'/' as i32 || cap.nchar == b'?' as i32 {
            if !g_st().cmdwin_buf.is_null() {
                emsg(tr(E_CMDLINE_WINDOW_ALREADY_OPEN));
                return;
            }
            if cap.nchar == b':' as i32 && cap.count0 > 0 {
                do_cmdline_cmd(c"exmode");
                return;
            }
            let insert_range = g_st().visual_global.active && cap.nchar == b':' as i32;
            let mut fc: [c_char; 2] = [cap.nchar as c_char, 0];
            let mut tv_args: [TypVal; 4] = [
                TypVal {
                    v_type: VarType::VarString,
                    v_lock: 0,
                    vval: TypvalVval::String(fc.as_mut_ptr()),
                },
                TypVal {
                    v_type: VarType::VarString,
                    v_lock: 0,
                    vval: TypvalVval::String(if insert_range {
                        c"'<,'>".as_ptr().cast_mut()
                    } else {
                        c"".as_ptr().cast_mut()
                    }),
                },
                TypVal {
                    v_type: VarType::VarNumber,
                    v_lock: 0,
                    vval: TypvalVval::Number(if insert_range { 5 } else { 1 }),
                },
                TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                },
            ];
            nlua_call_typval(c"vim._core.cmdwin", c"open", tv_args.as_mut_ptr(), None);
        } else if g_st().reg_executing == 0 && !do_record(cap.nchar) {
            clearopbeep(cap.oap_mut());
        }
    }
}

fn nv_at(cap: &mut Cmdarg) {
    {
        if checkclearop(cap.oap_mut()) {
            return;
        }
        if cap.nchar == b'=' as i32 && get_expr_register() == NUL as i32 {
            return;
        }
        atom_macro_start(cap.nchar);
        while {
            let prev = cap.count1;
            cap.count1 -= 1;
            prev != 0 && !g_st().got_int
        } {
            if !do_execreg(cap.nchar, 0, 0, 0) {
                clearopbeep(cap.oap_mut());
                break;
            }
            line_breakcheck();
        }
    }
}

fn nv_halfpage(cap: &mut Cmdarg) {
    {
        if !checkclearop(cap.oap_mut()) {
            pagescroll(
                if cap.cmdchar == CTRL_D_CONST {
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

fn nv_join(cap: &mut Cmdarg) {
    {
        if g_st().visual_global.active {
            nv_operator(cap);
            return;
        }

        if checkclearop(cap.oap_mut()) {
            return;
        }

        cap.count0 = cap.count0.max(2);

        if curwin_mut().w_cursor.lnum + cap.count0 - 1 > curbuf_mut().b_ml.ml_line_count {
            if cap.count0 <= 2 {
                clearopbeep(cap.oap_mut());
                return;
            }
            cap.count0 = curbuf_mut().b_ml.ml_line_count - curwin_mut().w_cursor.lnum + 1;
        }

        prep_redo_cmd(cap);
        do_join(
            cap.count0 as usize,
            cap.nchar == NUL as i32,
            true,
            true,
            true,
        );
    }
}

fn nv_put(cap: &mut Cmdarg) {
    nv_put_opt(cap, false);
}

fn nv_put_opt(cap: &mut Cmdarg, fix_indent: bool) {
    unsafe {
        let mut savereg: Option<Yankreg> = None;
        let mut empty = false;
        let mut was_visual = false;
        let mut dir: i32;
        let mut flags: i32 = 0;
        let save_fen = curwin_mut().w_onebuf_opt.wo_fen;

        if cap.oap_mut().op_type != OP_NOP {
            if cap.oap_mut().op_type == OP_DELETE && cap.cmdchar == b'p' as i32 {
                atom_capture_op(cap.oap_mut(), cap, false);
                clearop(cap.oap_mut());
                assert!(cap.opcount >= 0);
                nv_diffgetput(true, cap.opcount as usize);
            } else {
                clearopbeep(cap.oap_mut());
            }
            return;
        }

        if bt_prompt(Some(crate::globals::curbuf_mut())) && !prompt_curpos_editable() {
            if curwin_mut().w_cursor.lnum == curbuf_mut().b_prompt_start.mark.lnum {
                curwin_mut().w_cursor.col = curbuf_mut().b_prompt_start.mark.col;
                cap.cmdchar = b'P' as i32;
            } else {
                clearopbeep(cap.oap_mut());
                return;
            }
        }

        if fix_indent {
            dir = if cap.cmdchar == b']' as i32 && cap.nchar == b'p' as i32 {
                FORWARD
            } else {
                BACKWARD
            };
            flags |= PUT_FIXINDENT;
        } else {
            dir = if cap.cmdchar == b'P' as i32
                || ((cap.cmdchar == b'g' as i32 || cap.cmdchar == b'z' as i32)
                    && cap.nchar == b'P' as i32)
            {
                BACKWARD
            } else {
                FORWARD
            };
        }
        let mut vatom_prepped = false;
        if g_st().visual_global.active {
            let mut spec = CmdSpec::new();
            spec.regname = cap.oap_mut().regname;
            spec.count = i64::from(cap.count0);
            spec.cmd = cap.cmdchar;
            spec.cmd2 = cap.nchar;
            vatom_prepped = atom_visual_end(spec, true);
        }
        if !vatom_prepped {
            prep_redo_cmd(cap);
        }
        if cap.cmdchar == b'g' as i32 {
            flags |= PUT_CURSEND;
        } else if cap.cmdchar == b'z' as i32 {
            flags |= PUT_BLOCK_INNER;
        }

        if g_st().visual_global.active {
            was_visual = true;
            let regname = cap.oap_mut().regname;
            let keep_registers = cap.cmdchar == b'P' as i32;
            let clipoverwrite = (regname == b'+' as i32 || regname == b'*' as i32)
                && (opt_vars().cb_flags
                    & (K_OPT_CB_FLAG_UNNAMED_CONST | K_OPT_CB_FLAG_UNNAMEDPLUS_CONST))
                    != 0;
            if regname == 0
                || regname == b'"' as i32
                || clipoverwrite
                || ascii_isdigit(regname)
                || regname == b'-' as i32
            {
                savereg = Some(copy_register(regname));
            }

            curwin_mut().w_onebuf_opt.wo_fen = 0;

            if !g_st().visual_global.active
                || g_st().visual_global.mode == b'V' as i32
                || regname != b'.' as i32
            {
                cap.cmdchar = b'd' as i32;
                cap.nchar = NUL as i32;
                cap.oap_mut().regname = if keep_registers {
                    b'_' as i32
                } else {
                    NUL as i32
                };
                g_st().msg_silent += 1;
                atom_suppress(true);
                nv_operator(cap);
                do_pending_operator(cap, 0, false);
                atom_suppress(false);
                empty = (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0;
                g_st().msg_silent -= 1;

                cap.oap_mut().regname = regname;
            }

            if g_st().visual_global.mode == b'V' as i32 {
                flags |= PUT_LINE;
            } else if g_st().visual_global.mode == b'v' as i32 {
                flags |= PUT_LINE_SPLIT;
            }
            if g_st().visual_global.mode == CTRL_V_CONST && dir == FORWARD {
                flags |= PUT_LINE_FORWARD;
            }
            dir = BACKWARD;
            if (g_st().visual_global.mode != b'V' as i32
                && curwin_mut().w_cursor.col < curbuf_mut().b_op_start.col)
                || (g_st().visual_global.mode == b'V' as i32
                    && curwin_mut().w_cursor.lnum < curbuf_mut().b_op_start.lnum)
            {
                dir = FORWARD;
            }
            g_st().visual_global.active = true;
        }
        let savereg_ptr = savereg
            .as_mut()
            .map_or(ptr::null_mut(), |r| r as *mut Yankreg);
        do_put(cap.oap_mut().regname, savereg_ptr, dir, cap.count1, flags);
        drop(savereg);

        if was_visual {
            if save_fen != 0 {
                curwin_mut().w_onebuf_opt.wo_fen = 1;
            }
            curbuf_mut().b_visual.vi_start = curbuf_mut().b_op_start;
            curbuf_mut().b_visual.vi_end = curbuf_mut().b_op_end;
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                inc(&mut curbuf_mut().b_visual.vi_end);
            }
        }

        if empty && *ml_get(curbuf_mut().b_ml.ml_line_count) == NUL {
            ml_delete_flags(curbuf_mut().b_ml.ml_line_count, ML_DEL_MESSAGE);
            deleted_lines(curbuf_mut().b_ml.ml_line_count + 1, 1);

            if curwin_mut().w_cursor.lnum > curbuf_mut().b_ml.ml_line_count {
                curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
                coladvance(curwin_mut(), MAXCOL);
            }
        }
        auto_format(false, true);
    }
}

fn nv_open(cap: &mut Cmdarg) {
    {
        if cap.oap_mut().op_type == OP_DELETE && cap.cmdchar == b'o' as i32 {
            atom_capture_op(cap.oap_mut(), cap, false);
            clearop(cap.oap_mut());
            assert!(cap.opcount >= 0);
            nv_diffgetput(false, cap.opcount as usize);
        } else if g_st().visual_global.active {
            v_swap_corners(cap.cmdchar);
        } else if bt_prompt(Some(crate::globals::curbuf_mut()))
            && curwin_mut().w_cursor.lnum < curbuf_mut().b_prompt_start.mark.lnum
        {
            clearopbeep(cap.oap_mut());
        } else {
            n_opencmd(cap);
        }
    }
}

fn nv_paste(cap: &mut Cmdarg) {
    paste_repeat(cap.count1);
}

fn nv_event(cap: &mut Cmdarg) {
    g_st().may_garbage_collect = false;
    let may_restart = g_st().restart_edit != 0 || g_st().visual_global.restart_select != 0;
    state_handle_k_event();
    g_st().finish_op = false;
    if may_restart {
        cap.retval |= CA_COMMAND_BUSY;
    }
}

pub fn normal_cmd(oap: &mut Oparg, toplevel: bool) {
    let mut s = NormalState::new();
    normal_state_init(&mut s);
    s.toplevel = toplevel;
    s.oa = *oap;
    normal_prepare(&mut s);
    normal_execute(&mut s, safe_vgetc());
    *oap = s.oa;
}

pub fn exec_stuffed(mut cap: Option<&mut Cmdarg>) {
    unsafe {
        if let Some(ref cap) = cap {
            atom_stuff_start(cap);
        }
        if text_locked() || curbuf_locked() {
            return;
        }
        let mut oa = Oparg::new();
        let oap = if let Some(ref cap) = cap {
            &mut *cap.oap
        } else {
            &mut oa
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
        let mut p1 = CursorPos {
            lnum: 1,
            col: 5,
            coladd: 0,
        };
        let p2 = CursorPos {
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
        assert!(ascii_iswhite(b' ' as i32));
        assert!(ascii_isdigit(b'5' as i32));
    }

    #[test]
    fn test_is_ident() {
        let line = c"int foo = 1; /* bar */ // baz";
        assert!(is_ident(line.as_ptr(), 4));
        assert!(!is_ident(line.as_ptr(), 16));
        assert!(!is_ident(line.as_ptr(), 26));
    }
}
