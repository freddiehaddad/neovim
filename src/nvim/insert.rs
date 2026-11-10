//! Functions for Insert mode (`src/nvim/insert.c`).
#![forbid(unsafe_code)]

use crate::autocmd::{
    AuEvent, EVENT_CURSORMOVEDI, EVENT_INSERTCHANGE, EVENT_INSERTCHARPRE, EVENT_INSERTENTER,
    EVENT_INSERTLEAVE, EVENT_INSERTLEAVEPRE, EVENT_TEXTCHANGEDI, EVENT_TEXTCHANGEDP,
    apply_autocmds, has_event,
};
use crate::buffer::{
    Bufref, FileBuf, K_UIMESSAGES, bt_cmdwin, bt_prompt, bt_quickfix, buf_get_changedtick,
    buf_is_empty, buf_meta_total, bufref_valid, set_bufref,
};
use crate::change::{
    COM_MAX_LEN, K_CPO_LISTWM, K_OPT_VE_FLAG_ONEMORE, MB_MAXCHAR, OPENLINE_DO_COM,
    appended_lines_mark, change_warning, changed_bytes, del_bytes, del_char, get_leader_len,
    ins_bytes_len, ins_char, ins_char_bytes, ins_str, inserted_bytes, open_line,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_isspace, ascii_iswhite, ascii_isxdigit, ascii2cells, char2cells,
    hex2nr, vim_isprintc, vim_iswordc,
};
use crate::cmdexpand::{CTRL_N, CTRL_P, MB_MAXBYTES};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    Colnr, CursorPos, K_OPT_VE_FLAG_ALL, Linenr, VALID_VIRTCOL, char_before_cursor, check_cursor,
    check_cursor_col, check_visual_pos, coladvance, dec_cursor, equalpos, gchar_cursor,
    get_cursor_line_ptr, get_cursor_line_slice, get_cursor_pos_len, get_cursor_pos_ptr, getviscol,
    inc_cursor, lt,
};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::digraph::{digraph_get, do_digraph};
use crate::drawline::{VALID_WCOL, VALID_WROW};
use crate::drawscreen::{
    UPD_VALID, redraw_later, redraw_statuslines, redraw_winline, redrawing, setcursor,
    show_cursor_info_later, showmode, skip_showmode, status_redraw_curbuf, unshowmode,
    update_screen,
};
use crate::errors::*;
use crate::eval::buffer::{invoke_prompt_interrupt, prompt_invoke_callback};
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::eval::userfunc::ascii_iswhite_nl_or_nul;
use crate::eval::vars::{VV_CHAR, VV_INSERTMODE, get_vim_var_str, set_vim_var_string};
use crate::ex_cmds::{B_IMODE_LMAP, BL_FIX, BL_SOL, BL_WHITE, CTRL_C_CONST, CTRL_E, CTRL_Y};
use crate::ex_docmd::{CMOD_KEEPJUMPS, do_cmdline, do_cmdline_cmd, expr_map_locked};
use crate::ex_getln::{
    ABBR_OFF, B_IMODE_NONE, CTRL_A, CTRL_BSL, CTRL_CONST_CONST, CTRL_D_CONST, CTRL_F_CONST, CTRL_G,
    CTRL_HAT_CONST, CTRL_K_CONST, CTRL_L_CONST, CTRL_O, CTRL_Q_CONST, CTRL_R_CONST, CTRL_RSB_CONST,
    CTRL_T_CONST, CTRL_U, CTRL_W_CONST, CTRL_Z_CONST, K_EVENT, K_LUA, K_SELECT, cmdwin_do_action,
};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE};
use crate::fileio::check_timestamps;
use crate::fold::{
    fold_check_close, fold_open_cursor, fold_update_after_insert, has_folding, has_folding_win,
};
use crate::globals::*;
use crate::grid::{
    Schar, grid_line_flush, grid_line_getchar, grid_line_put_schar, grid_line_puts,
    grid_line_start, schar_from_ascii,
};
use crate::highlight_group::highlight_changed;
use crate::indent::{
    INDENT_DEC, INDENT_SET, change_indent, fix_indent, get_indent, get_sts_value, get_sw_value,
    inindent, ins_try_si, may_do_si, tabstop_at, tabstop_count, tabstop_first, tabstop_padding,
    tabstop_start,
};
use crate::indent_c::{cindent_on, do_c_expr_indent, in_cinkeys};
use crate::input::{
    K_COMPLETE_DELAY, K_INS_NONE, K_PASTE_START, char_avail, langmap_adjust, map_execute_lua,
    mb_byte2len_check, merge_modifiers, paste_repeat, plain_vgetc, prep_redo, redo_append_char,
    redo_append_lit, redo_append_str, redo_keys, start_redo_ins, stop_redo_ins, stuff_empty,
    stuff_readbuff_len, stuff_redo_readbuff, stuffchar_readbuff, vgetc, vpeekc, vungetc,
};
use crate::input_cmdatom::CmdSpec;
use crate::insexpand::{
    check_compl_option, compl_status_clear, compl_status_local, ctrl_x_mode_cmdline,
    ctrl_x_mode_dictionary, ctrl_x_mode_files, ctrl_x_mode_function, ctrl_x_mode_line_or_eval,
    ctrl_x_mode_none, ctrl_x_mode_normal, ctrl_x_mode_omni, ctrl_x_mode_path_defines,
    ctrl_x_mode_path_patterns, ctrl_x_mode_register, ctrl_x_mode_scroll, ctrl_x_mode_spell,
    ctrl_x_mode_tags, ctrl_x_mode_thesaurus, ctrl_x_mode_whole_line, ins_compl_accept_char,
    ins_compl_active, ins_compl_addfrommatch, ins_compl_addleader,
    ins_compl_arm_autocomplete_delay, ins_compl_arm_autostart, ins_compl_bs, ins_compl_cancel,
    ins_compl_clear, ins_compl_clear_autocomplete_delay, ins_compl_col, ins_compl_commit_char,
    ins_compl_delete, ins_compl_disable_autocomplete, ins_compl_disarm_autostart,
    ins_compl_enable_autocomplete, ins_compl_enter_selects, ins_compl_has_autocomplete,
    ins_compl_has_shown_match, ins_compl_init_get_longest, ins_compl_insert,
    ins_compl_is_match_selected, ins_compl_long_shown_match, ins_compl_preinsert_effect,
    ins_compl_preinsert_longest, ins_compl_prep, ins_compl_used_match, ins_compl_win_active,
    ins_complete, ins_ctrl_x, pum_wanted,
};
use crate::keycodes::{
    CAR, DEL, ESC, K_BS, K_C_END, K_C_HOME, K_C_LEFT, K_C_RIGHT, K_COMMAND, K_DEL, K_DOWN, K_END,
    K_F1, K_HELP, K_HOME, K_IGNORE, K_INS, K_KDEL, K_KEND, K_KENTER, K_KHOME, K_KINS, K_KPAGEDOWN,
    K_KPAGEUP, K_LEFT, K_LEFTDRAG, K_LEFTMOUSE, K_LEFTMOUSE_NM, K_LEFTRELEASE, K_LEFTRELEASE_NM,
    K_MIDDLEDRAG, K_MIDDLEMOUSE, K_MIDDLERELEASE, K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE,
    K_MOUSERIGHT, K_MOUSEUP, K_NOP, K_PAGEDOWN, K_PAGEUP, K_RIGHT, K_RIGHTDRAG, K_RIGHTMOUSE,
    K_RIGHTRELEASE, K_S_DOWN, K_S_END, K_S_HOME, K_S_LEFT, K_S_RIGHT, K_S_TAB, K_S_UP, K_SPECIAL,
    K_UP, K_X1DRAG, K_X1MOUSE, K_X1RELEASE, K_X2DRAG, K_X2MOUSE, K_X2RELEASE, K_XF1, K_ZERO,
    KS_MODIFIER, NL, TAB, add_char2buf, get_special_key_name,
};
use crate::mapping::{check_abbr, map_to_exists_mode};
use crate::mark::{mark_view_make, reset_fmark};
use crate::marktree::K_MTMETA_INLINE;
use crate::mbyte::{
    mb_adjust_cursor, mb_get_class, ptr2cells, utf_char2bytes, utf_char2len, utf_cluster_info,
    utf_composinglike, utf_ptr2_str_char_info, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::mcursor::{mc_ins_cascade, mc_ins_cascade_restart, mc_ins_join, mc_ins_replaying};
use crate::memline::{gchar_pos, ml_append, ml_get, ml_get_len, ml_replace, ml_replace_dirty_line};
use crate::message::{emsg, msg_check_for_delay};
use crate::mouse::{
    MSCR_DOWN, MSCR_LEFT, MSCR_RIGHT, MSCR_UP, ins_mouse, ins_mousescroll, setmouse,
};
use crate::r#move::{
    adjust_skipcol, curs_columns, do_check_cursorbind, pagescroll, scrolldown_clamp,
    scrollup_clamp, set_topline, update_curswant, update_topline, validate_cursor,
    validate_cursor_col, validate_virtcol,
};
use crate::normal::{
    add_to_showcmd, add_to_showcmd_c, clear_showcmd, do_check_scrollbind, end_visual_mode,
    start_selection,
};
use crate::ops::do_join;
use crate::option::{
    can_bs, copy_option_part, get_scrolloff_value, get_ve_flags, set_iminsert_global,
};
use crate::option_vars::*;
use crate::os::input::line_breakcheck;
use crate::plines::{
    CSType, CharsizeArg, charsize_nowrap, getvcol, getvcol_nolist, init_charsize_arg,
    linetabsize_col, win_charsize, win_chartabsize,
};
use crate::popupmenu::{pum_check_clear, pum_visible};
use crate::regexp::{CTRL_H_CONST, CTRL_V_CONST, MAXCOL};
use crate::register::{
    PUT_CURSEND, PUT_FIXINDENT, YankRegMode, do_put, get_expr_register, get_yank_register,
    insert_reg, is_literal_register, valid_yank_reg,
};
use crate::search::{BACKWARD, FORWARD};
use crate::state::{
    EditorState, MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL, MODE_REPLACE, MODE_VREPLACE,
    REPLACE_FLAG, VREPLACE_FLAG, may_trigger_modechanged, may_trigger_safestate, state_enter,
    state_handle_k_event, virtual_active,
};
use crate::strings::{format_to_buf, gettext};
use crate::syntax::syntax_present;
use crate::terminal::terminal_enter;
use crate::textformat::{
    auto_format, check_auto_format, comp_textwidth, fex_format, has_format_option, internal_format,
};
use crate::textobject::{bck_word, fwd_word};
use crate::ui::{ui_cursor_shape, ui_flush, ui_has, vim_beep};
use crate::undo::{u_clearallandblockfree, u_save, u_save_cursor, u_sync};
use crate::window::{WinInfoInfo, goto_tabpage, may_trigger_win_scrolled_resized};
use std::ffi::CStr;

use crate::change::COM_MIDDLE;
const INPUT_BUFLEN: usize = 100;

pub(crate) const MOD_MASK_SHIFT: i32 = 0x02;
pub(crate) const MOD_MASK_CTRL: i32 = 0x04;
const MOD_MASK_CMD: i32 = 0x80;

pub(crate) const CTRL_J_CONST: i32 = 10;
pub(crate) const CTRL_S_CONST: i32 = 19;
pub(crate) const CTRL_X_CONST: i32 = 24;

pub(crate) const K_CHARSIZE_FAST: CSType = true;

#[inline]
const fn is_special_fn(c: i32) -> bool {
    c < 0
}

const K_INS_ARROW: i32 = 1;
const K_INS_JUMP: i32 = 2;

pub(crate) const HLF_8: usize = 1;

pub(crate) const K_OPT_FDO_FLAG_ALL: u32 = 0x01;
pub(crate) const K_OPT_FDO_FLAG_HOR: u32 = 0x04;
const K_OPT_FDO_FLAG_INSERT: u32 = 0x100;

const K_OPT_BO_FLAG_BACKSPACE: u32 = 0x02;
const K_OPT_BO_FLAG_CURSOR: u32 = 0x04;
const K_OPT_BO_FLAG_COPY: u32 = 0x10;
const K_OPT_BO_FLAG_CTRLG: u32 = 0x20;
const K_OPT_BO_FLAG_REGISTER: u32 = 0x8000;

pub(crate) const K_FO_AUTO: i32 = b'a' as i32;
pub(crate) const K_FO_INS_BLANK: i32 = b'b' as i32;
const K_FO_INS_LONG: i32 = b'l' as i32;
const K_FO_RET_COMS: i32 = b'r' as i32;
pub(crate) const K_FO_WHITE_PAR: i32 = b'w' as i32;

const K_CPO_BACKSPACE: i32 = b'v' as i32;
const K_CPO_INDENT: i32 = b'I' as i32;
const K_CPO_REPLCNT: i32 = b'X' as i32;

const BS_INDENT: i32 = b'i' as i32;
pub(crate) const BS_EOL: i32 = b'l' as i32;
pub(crate) const BS_START: i32 = b's' as i32;
pub(crate) const BS_NOSTOP: i32 = b'p' as i32;

const INDENT_INC: i32 = 2;

pub(crate) const INSCHAR_FORMAT: i32 = 1;
const INSCHAR_CTRLV: i32 = 4;
pub(crate) const INSCHAR_NO_FEX: i32 = 8;

const BACKSPACE_CHAR: i32 = 1;
const BACKSPACE_WORD: i32 = 2;
const BACKSPACE_WORD_NOT_SPACE: i32 = 3;
const BACKSPACE_LINE: i32 = 4;

const PC_STATUS_UNSET: i32 = 0;
const PC_STATUS_RIGHT: i32 = 1;
const PC_STATUS_LEFT: i32 = 2;
const PC_STATUS_SET: i32 = 3;

#[repr(C)]
struct InsertState {
    mincol: i32,
    cmdchar: i32,
    cmdchar_todo: i32,
    ins_just_started: bool,
    startln: i32,
    count: i32,
    c: i32,
    lastc: i32,
    i: i32,
    did_backspace: bool,
    line_is_white: bool,
    old_topline: Linenr,
    old_topfill: i32,
    inserted_space: i32,
    replace_state: i32,
    did_restart_edit: i32,
    nomove: bool,
}

impl InsertState {
    const fn new() -> Self {
        Self {
            mincol: 0,
            cmdchar: 0,
            cmdchar_todo: 0,
            ins_just_started: false,
            startln: 0,
            count: 0,
            c: 0,
            lastc: 0,
            i: 0,
            did_backspace: false,
            line_is_white: false,
            old_topline: 0,
            old_topfill: 0,
            inserted_space: 0,
            replace_state: 0,
            did_restart_edit: 0,
            nomove: false,
        }
    }
}

impl EditorState for InsertState {
    fn check(&mut self) -> i32 {
        insert_check(self)
    }

    fn execute(&mut self, key: i32) -> i32 {
        insert_execute(self, key)
    }
}

impl Default for InsertState {
    fn default() -> Self {
        Self::new()
    }
}

struct InsertModuleState {
    compl_busy: std::cell::Cell<bool>,
    last_insert: std::cell::RefCell<Option<std::ffi::CString>>,
    last_insert_skip: std::cell::Cell<i32>,
    replace_stack: std::cell::RefCell<Vec<u8>>,
    pc_status: std::cell::Cell<i32>,
    pc_schar: std::cell::Cell<Schar>,
    pc_attr: std::cell::Cell<i32>,
    pc_row: std::cell::Cell<i32>,
    pc_col: std::cell::Cell<i32>,
    ins_esc_disabled_redraw: std::cell::Cell<bool>,
}

impl InsertModuleState {
    const fn new() -> Self {
        Self {
            compl_busy: std::cell::Cell::new(false),
            last_insert: std::cell::RefCell::new(None),
            last_insert_skip: std::cell::Cell::new(0),
            replace_stack: std::cell::RefCell::new(Vec::new()),
            pc_status: std::cell::Cell::new(PC_STATUS_UNSET),
            pc_schar: std::cell::Cell::new(0),
            pc_attr: std::cell::Cell::new(0),
            pc_row: std::cell::Cell::new(0),
            pc_col: std::cell::Cell::new(0),
            ins_esc_disabled_redraw: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static INS_MOD_STATE: InsertModuleState = const { InsertModuleState::new() };
}

#[inline]
fn ins_st<R>(f: impl FnOnce(&InsertModuleState) -> R) -> R {
    INS_MOD_STATE.with(f)
}

#[inline]
const fn is_special_ins(c: i32) -> bool {
    c < b' ' as i32 || c >= DEL || c == b'0' as i32 || c == b'^' as i32
}

#[inline]
pub fn linetabsize_str(s: &[u8]) -> i32 {
    let cstr = std::ffi::CString::new(crate::memory::bytes_until_nul(s)).unwrap_or_default();
    linetabsize_col(0, &cstr)
}

use crate::mbyte::utf_byte2len;

#[inline]
fn trigger_autocomplete(s: &mut InsertState) -> bool {
    mc_ins_cascade();
    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
    update_screen();
    ui_flush();
    ins_compl_enable_autocomplete();
    ins_compl_arm_autostart();
    if !ins_compl_arm_autocomplete_delay() {
        insert_do_complete(s);
        return true;
    }
    false
}

#[inline]
fn may_trigger_autocomplete(s: &mut InsertState) {
    if ins_compl_has_autocomplete() && !char_avail() && curwin_mut().w_cursor.col > 0 {
        s.c = char_before_cursor();
        if vim_isprintc(s.c) {
            let _ = trigger_autocomplete(&mut *s);
        }
    }
}

fn insert_enter(s: &mut InsertState) {
    s.did_backspace = true;
    s.old_topfill = -1;
    s.replace_state = MODE_REPLACE;
    s.cmdchar_todo = s.cmdchar;
    s.ins_just_started = true;
    g_st().ins_global.did_restart_edit = g_st().restart_edit;
    msg_check_for_delay(true);
    g_st().ins_global.update_start_orig = true;

    ins_compl_clear();

    if s.cmdchar != b'r' as i32 && s.cmdchar != b'v' as i32 {
        let save_cursor = curwin_mut().w_cursor;
        let ptr = if s.cmdchar == b'R' as i32 {
            c"r"
        } else if s.cmdchar == b'V' as i32 {
            c"v"
        } else {
            c"i"
        };
        set_vim_var_string(VV_INSERTMODE, Some(ptr), 1);
        set_vim_var_string(VV_CHAR, None, -1);
        ins_apply_autocmds(EVENT_INSERTENTER);

        if g_st().need_highlight_changed {
            highlight_changed();
        }

        if !equalpos(curwin_mut().w_cursor, save_cursor)
            && get_vim_var_str(VV_CHAR).is_empty()
            && save_cursor.lnum <= curbuf_mut().b_ml.ml_line_count
        {
            let save_state = g_st().state_global;
            curwin_mut().w_cursor = save_cursor;
            g_st().state_global = MODE_INSERT;
            check_cursor_col(curwin_mut());
            g_st().state_global = save_state;
        }
    }

    if g_st().where_paste_started.lnum != 0 {
        g_st().ins_global.start = g_st().where_paste_started;
    } else {
        g_st().ins_global.start = curwin_mut().w_cursor;
        if s.startln != 0 {
            g_st().ins_global.start.col = 0;
        }
    }

    g_st().ins_global.start_textlen = linetabsize_str(crate::cursor::get_cursor_line_slice());
    g_st().ins_global.start_blank_vcol = MAXCOL;

    if !g_st().ins_global.did_ai {
        g_st().ins_global.ai_col = 0;
    }

    if s.cmdchar != NUL as i32 && g_st().restart_edit == 0 {
        if s.cmdchar == b'V' as i32 || s.cmdchar == b'v' as i32 {
            prep_redo(
                false,
                false,
                CmdSpec {
                    count: s.count as i64,
                    cmd: b'g' as i32,
                    cmd2: if s.cmdchar == b'v' as i32 {
                        b'r' as i32
                    } else {
                        b'R' as i32
                    },
                    ..CmdSpec::default()
                },
            );
        } else {
            prep_redo(
                false,
                false,
                CmdSpec {
                    count: s.count as i64,
                    cmd: s.cmdchar,
                    cmd2: if s.cmdchar == b'g' as i32 {
                        b'I' as i32
                    } else {
                        NUL as i32
                    },
                    ..CmdSpec::default()
                },
            );
            if s.cmdchar == b'r' as i32 {
                s.count = 1;
            }
        }
    }

    if s.cmdchar == b'R' as i32 {
        g_st().state_global = MODE_REPLACE;
    } else if s.cmdchar == b'V' as i32 || s.cmdchar == b'v' as i32 {
        g_st().state_global = MODE_VREPLACE;
        s.replace_state = MODE_VREPLACE;
        g_st().orig_line_count = curbuf_mut().b_ml.ml_line_count;
        g_st().vr_lines_changed = 1;
    } else {
        g_st().state_global = MODE_INSERT;
    }

    may_trigger_modechanged();
    g_st().ins_global.stop_insert_mode = false;

    if gchar_cursor() == TAB || buf_meta_total(curbuf_mut(), K_MTMETA_INLINE) > 0 {
        curwin_mut().w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
    }

    if curbuf_mut().b_p_iminsert == B_IMODE_LMAP {
        g_st().state_global |= MODE_LANGMAP;
    }

    setmouse();
    clear_showcmd();
    g_st().ins_global.revins_on = g_st().state_global == MODE_INSERT && opt_vars().p_ri;
    if g_st().ins_global.revins_on {
        undisplay_dollar();
    }
    g_st().ins_global.revins_chars = 0;
    g_st().ins_global.revins_legal = 0;
    g_st().ins_global.revins_scol = -1;

    if g_st().restart_edit != 0 && stuff_empty() {
        g_st().ins_global.moved = if g_st().where_paste_started.lnum == 0 {
            K_INS_JUMP
        } else {
            K_INS_NONE
        };
        g_st().restart_edit = 0;

        validate_virtcol(curwin_mut());
        update_curswant();
        if (g_st().ins_at_eol && curwin_mut().w_cursor.lnum == g_st().ins_global.o_lnum)
            || curwin_mut().w_curswant > curwin_mut().w_virtcol
        {
            let pos_slice = crate::cursor::get_cursor_pos_slice();
            if !pos_slice.is_empty() {
                if pos_slice.len() == 1 {
                    curwin_mut().w_cursor.col += 1;
                } else {
                    s.i = crate::mbyte::utfc_slice2len(pos_slice) as i32;
                    if pos_slice[s.i as usize..].is_empty() {
                        curwin_mut().w_cursor.col += s.i;
                    }
                }
            }
        }
        g_st().ins_at_eol = false;
    } else {
        g_st().ins_global.moved = K_INS_NONE;
    }

    g_st().need_start_insertmode = false;
    g_st().ins_global.need_undo = true;
    g_st().where_paste_started.lnum = 0;
    g_st().ins_global.can_cindent = true;
    if g_st().ins_global.did_restart_edit == 0 {
        fold_open_cursor();
    }

    s.i = 0;
    if opt_vars().p_smd && g_st().msg_silent == 0 {
        s.i = showmode();
    }

    if g_st().ins_global.did_restart_edit == 0 {
        change_warning(curbuf_mut(), if s.i == 0 { 0 } else { s.i + 1 });
    }

    ui_cursor_shape();
    do_digraph(-1);

    let redo = redo_keys(None);
    g_st().ins_global.new_insert_skip = redo.as_ref().map_or(0, |s| s.as_bytes().len() as i32);

    g_st().old_indent = 0;

    loop {
        state_enter(s);
        if ins_esc(&mut s.count, s.cmdchar, s.nomove) {
            break;
        }
    }

    ins_compl_clear_autocomplete_delay();
    if g_st().ins_at_eol {
        g_st().ins_global.o_lnum = curwin_mut().w_cursor.lnum;
    }

    pum_check_clear();
    fold_update_after_insert();
    if s.cmdchar != b'r' as i32 && s.cmdchar != b'v' as i32 && s.c != CTRL_C_CONST {
        ins_apply_autocmds(EVENT_INSERTLEAVE);
    }
    g_st().did_cursorhold = false;

    if !char_avail() && curbuf_mut().b_last_changedtick_i == buf_get_changedtick(curbuf_mut()) {
        curbuf_mut().b_last_changedtick = buf_get_changedtick(curbuf_mut());
    }
}

fn insert_check(s: &mut InsertState) -> i32 {
    mc_ins_cascade();

    if g_st().ins_global.revins_legal == 0 {
        g_st().ins_global.revins_scol = -1;
    } else {
        g_st().ins_global.revins_legal = 0;
    }

    if g_st().ins_global.moved != K_INS_NONE {
        s.count = 0;
    }

    if g_st().ins_global.update_start_orig {
        g_st().ins_global.start_orig = g_st().ins_global.start;
    }

    if !curbuf_mut().terminal.is_null() && !g_st().ins_global.stop_insert_mode {
        g_st().ins_global.stop_insert_mode = true;
        g_st().restart_edit = b'I' as i32;
        stuffchar_readbuff(K_NOP);
    }

    if g_st().ins_global.stop_insert_mode && !ins_compl_active() {
        s.count = 0;
        return 0;
    }

    if g_st().ins_global.moved == K_INS_NONE {
        curwin_mut().w_set_curswant = true;
    }

    if stuff_empty() {
        g_st().did_check_timestamps = false;
        if g_st().need_check_timestamps {
            check_timestamps(0);
        }
    }

    g_st().msg_scroll = 0;

    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_INSERT) != 0 {
        fold_open_cursor();
    }

    if !char_avail() {
        fold_check_close();
    }

    if bt_prompt(Some(crate::globals::curbuf_mut())) {
        init_prompt(s.cmdchar_todo);
        s.cmdchar_todo = NUL as i32;
    }

    if curbuf_mut().b_mod_set
        && curwin_mut().w_onebuf_opt.wo_wrap
        && !curwin_mut().w_onebuf_opt.wo_sms
        && !s.did_backspace
        && curwin_mut().w_topline == s.old_topline
        && curwin_mut().w_topfill == s.old_topfill
        && s.count <= 1
    {
        s.mincol = curwin_mut().w_wcol;
        validate_cursor_col(curwin_mut());

        if curwin_mut().w_wcol
            < s.mincol
                - tabstop_at(
                    get_nolist_virtcol(),
                    curbuf_mut().b_p_ts,
                    &curbuf_mut().b_p_vts_array,
                    false,
                )
            && curwin_mut().w_wrow
                == curwin_mut().w_view_height - 1 - (get_scrolloff_value(curwin_mut()) as i32)
            && (curwin_mut().w_cursor.lnum != curwin_mut().w_topline || curwin_mut().w_topfill > 0)
        {
            if curwin_mut().w_topfill > 0 {
                curwin_mut().w_topfill -= 1;
            } else {
                let topline = curwin_mut().w_topline;
                if has_folding(curwin_mut(), topline, None, Some(&mut s.old_topline)) {
                    set_topline(curwin_mut(), s.old_topline + 1);
                } else {
                    set_topline(curwin_mut(), topline + 1);
                }
            }
        }
    }

    if s.count <= 1 {
        update_topline(curwin_mut());
    }

    s.did_backspace = false;

    if s.count <= 1 {
        validate_cursor(curwin_mut());
    }

    ins_redraw(true);

    if curwin_mut().w_onebuf_opt.wo_scb {
        do_check_scrollbind(true);
    }

    if curwin_mut().w_onebuf_opt.wo_crb {
        do_check_cursorbind();
    }

    if s.count <= 1 {
        update_curswant();
    }
    s.old_topline = curwin_mut().w_topline;
    s.old_topfill = curwin_mut().w_topfill;

    if s.c != K_EVENT && s.c != K_COMPLETE_DELAY {
        s.lastc = s.c;
    }

    if g_st().ins_global.dont_sync_undo == K_NONE {
        g_st().ins_global.dont_sync_undo = K_TRUE;
    } else {
        g_st().ins_global.dont_sync_undo = K_FALSE;
    }

    if s.ins_just_started {
        s.ins_just_started = false;
        if ins_compl_has_autocomplete() && !char_avail() && curwin_mut().w_cursor.col > 0 {
            s.c = char_before_cursor();
            if vim_isprintc(s.c) {
                ins_compl_enable_autocomplete();
                ins_compl_arm_autostart();
                ins_compl_init_get_longest();
                if !ins_compl_arm_autocomplete_delay() {
                    insert_do_complete(&mut *s);
                    insert_handle_key_post(s);
                    return 1;
                }
            }
        }
    }

    1
}

fn insert_execute(s: &mut InsertState, key: i32) -> i32 {
    if g_st().ins_global.stop_insert_mode {
        if key != K_IGNORE && key != K_NOP {
            vungetc(key);
        }
        s.count = 0;
        s.nomove = true;
        ins_compl_prep(ESC);
        return 0;
    }

    if key == K_IGNORE || key == K_NOP {
        return -1;
    }
    s.c = key;

    if key != K_EVENT {
        g_st().did_cursorhold = true;
    }
    if key != K_EVENT && key != K_COMPLETE_DELAY {
        ins_compl_clear_autocomplete_delay();
        ins_compl_disarm_autostart();
        if !ins_compl_active() {
            ins_compl_disable_autocomplete();
        }
    }

    if ins_compl_active()
        && curwin_mut().w_cursor.col >= ins_compl_col()
        && ins_compl_has_shown_match()
        && pum_wanted()
    {
        if (s.c == K_BS || s.c == CTRL_H_CONST) && curwin_mut().w_cursor.col > ins_compl_col() {
            s.c = ins_compl_bs();
            if s.c == NUL as i32 {
                return 1;
            }
        }

        if !ins_compl_used_match() {
            if s.c == CTRL_L_CONST && (!ctrl_x_mode_line_or_eval() || ins_compl_long_shown_match())
            {
                ins_compl_addfrommatch();
                return 1;
            }

            let is_commit = ins_compl_commit_char(s.c);
            if !is_commit && ins_compl_accept_char(s.c) {
                if let Some(str_) = do_insert_char_pre(s.c) {
                    let mut p = str_.as_bytes();
                    while !p.is_empty() {
                        ins_compl_addleader(utf_ptr2char(p));
                        p = &p[(utfc_ptr2len(p) as usize).min(p.len())..];
                    }
                } else {
                    ins_compl_addleader(s.c);
                }
                return 1;
            }

            if (s.c == CTRL_Y
                || is_commit
                || (ins_compl_enter_selects() && (s.c == CAR || s.c == K_KENTER || s.c == NL)))
                && stop_arrow()
            {
                ins_compl_delete(false);
                if ins_compl_preinsert_longest() && !ins_compl_is_match_selected() {
                    ins_compl_insert(false, true);
                    ins_compl_init_get_longest();
                    return 1;
                } else {
                    ins_compl_insert(false, false);
                }
            } else if ascii_iswhite_nl_or_nul(s.c) && ins_compl_preinsert_effect() {
                ins_compl_delete(false);
            }
        }
    }

    ins_compl_init_get_longest();
    if ins_compl_prep(s.c) {
        return 1;
    }

    if s.c == CTRL_BSL {
        ins_redraw(false);
        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        s.c = plain_vgetc();
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
        if s.c != CTRL_N && s.c != CTRL_G && s.c != CTRL_O {
            vungetc(s.c);
            s.c = CTRL_BSL;
        } else {
            if s.c == CTRL_O {
                ins_ctrl_o();
                g_st().ins_at_eol = false;
                s.nomove = true;
            }
            s.count = 0;
            return 0;
        }
    }

    if s.c != K_EVENT {
        s.c = do_digraph(s.c);
    }

    if (s.c == CTRL_V_CONST || s.c == CTRL_Q_CONST) && ctrl_x_mode_cmdline() {
        insert_do_complete(&mut *s);
        insert_handle_key_post(s);
        return 1;
    }

    if s.c == CTRL_V_CONST || s.c == CTRL_Q_CONST {
        ins_ctrl_v();
        s.c = CTRL_V_CONST;
        return 1;
    }

    if cindent_on() && ctrl_x_mode_none() {
        s.line_is_white = inindent(0);
        if in_cinkeys(s.c, b'!' as i32, s.line_is_white) && stop_arrow() {
            do_c_expr_indent();
            return 1;
        }
        if g_st().ins_global.can_cindent
            && in_cinkeys(s.c, b'*' as i32, s.line_is_white)
            && stop_arrow()
        {
            do_c_expr_indent();
        }
    }

    if curwin_mut().w_onebuf_opt.wo_rl {
        s.c = match s.c {
            K_LEFT => K_RIGHT,
            K_S_LEFT => K_S_RIGHT,
            K_C_LEFT => K_C_RIGHT,
            K_RIGHT => K_LEFT,
            K_S_RIGHT => K_S_LEFT,
            K_C_RIGHT => K_C_LEFT,
            other => other,
        };
    }

    if ins_start_select(s.c) {
        return 1;
    }

    insert_handle_key(s)
}

fn insert_handle_key(s: &mut InsertState) -> i32 {
    let mut goto_normalchar = false;
    let mut goto_do_complete = false;

    match s.c {
        ESC => {
            if !echeck_abbr(ESC + ABBR_OFF) {
                return 0;
            }
        }
        CTRL_C_CONST => {
            if bt_cmdwin(g_st().curbuf.opt()) {
                g_st().got_int = false;
                cmdwin_do_action(c"cancel");
                return 0;
            }
            if bt_prompt(Some(crate::globals::curbuf_mut())) && invoke_prompt_interrupt() {
                if !bt_prompt(Some(crate::globals::curbuf_mut())) {
                    return 0;
                }
            } else {
                return 0;
            }
        }
        CTRL_Z_CONST => {
            goto_normalchar = true;
        }
        CTRL_O => {
            if ctrl_x_mode_omni() {
                insert_do_complete(&mut *s);
            } else if !echeck_abbr(CTRL_O + ABBR_OFF) {
                ins_ctrl_o();
                if (get_ve_flags(curwin_mut()) & K_OPT_VE_FLAG_ONEMORE) != 0 {
                    g_st().ins_at_eol = false;
                    s.nomove = true;
                }
                s.count = 0;
                return 0;
            }
        }
        K_INS | K_KINS => {
            ins_insert(s.replace_state);
        }
        K_SELECT | K_IGNORE => {}
        K_HELP | K_F1 | K_XF1 => {
            stuffchar_readbuff(K_HELP);
            return 0;
        }
        c if c == b' ' as i32 && g_st().mod_mask != MOD_MASK_CTRL => {
            goto_normalchar = true;
        }
        c if c == b' ' as i32 || c == K_ZERO || c == NUL as i32 || c == CTRL_A => {
            if !stuff_inserted(NUL as i32, 1, i32::from(s.c == CTRL_A)) && s.c != CTRL_A {
                return 0;
            }
            s.inserted_space = 0;
        }
        CTRL_R_CONST => {
            if ctrl_x_mode_register() && !ins_compl_active() {
                insert_do_complete(&mut *s);
            } else {
                ins_reg();
                auto_format(false, true);
                s.inserted_space = 0;
            }
        }
        CTRL_G => {
            ins_ctrl_g();
        }
        CTRL_HAT_CONST => {
            ins_ctrl_hat();
        }
        CTRL_CONST_CONST => {
            if !opt_vars().p_ari {
                goto_normalchar = true;
            } else {
                ins_ctrl_();
            }
        }
        CTRL_D_CONST => {
            if ctrl_x_mode_path_defines() {
                insert_do_complete(&mut *s);
            } else {
                ins_shift(s.c, s.lastc);
                auto_format(false, true);
                s.inserted_space = 0;
            }
        }
        CTRL_T_CONST => {
            if ctrl_x_mode_thesaurus() {
                if check_compl_option(false) {
                    insert_do_complete(&mut *s);
                }
            } else {
                ins_shift(s.c, s.lastc);
                auto_format(false, true);
                s.inserted_space = 0;
            }
        }
        K_DEL | K_KDEL => {
            ins_del();
            auto_format(false, true);
        }
        K_BS | CTRL_H_CONST => {
            s.did_backspace = ins_bs(s.c, BACKSPACE_CHAR, &mut s.inserted_space);
            auto_format(false, true);
            if s.did_backspace {
                may_trigger_autocomplete(s);
            }
        }
        CTRL_W_CONST => {
            if bt_prompt(Some(crate::globals::curbuf_mut()))
                && (g_st().mod_mask & MOD_MASK_SHIFT) == 0
            {
                stuffchar_readbuff(CTRL_W_CONST);
                g_st().restart_edit = b'A' as i32;
                s.nomove = true;
                s.count = 0;
                return 0;
            }
            s.did_backspace = ins_bs(s.c, BACKSPACE_WORD, &mut s.inserted_space);
            auto_format(false, true);
            if s.did_backspace {
                may_trigger_autocomplete(s);
            }
        }
        CTRL_U => {
            if ctrl_x_mode_function() {
                insert_do_complete(&mut *s);
            } else {
                s.did_backspace = ins_bs(s.c, BACKSPACE_LINE, &mut s.inserted_space);
                auto_format(false, true);
                s.inserted_space = 0;
                if s.did_backspace {
                    may_trigger_autocomplete(s);
                }
            }
        }
        K_LEFTMOUSE | K_LEFTMOUSE_NM | K_LEFTDRAG | K_LEFTRELEASE | K_LEFTRELEASE_NM
        | K_MOUSEMOVE | K_MIDDLEMOUSE | K_MIDDLEDRAG | K_MIDDLERELEASE | K_RIGHTMOUSE
        | K_RIGHTDRAG | K_RIGHTRELEASE | K_X1MOUSE | K_X1DRAG | K_X1RELEASE | K_X2MOUSE
        | K_X2DRAG | K_X2RELEASE => {
            ins_mouse(s.c);
        }
        K_MOUSEDOWN => {
            ins_mousescroll(MSCR_DOWN);
        }
        K_MOUSEUP => {
            ins_mousescroll(MSCR_UP);
        }
        K_MOUSELEFT => {
            ins_mousescroll(MSCR_LEFT);
        }
        K_MOUSERIGHT => {
            ins_mousescroll(MSCR_RIGHT);
        }
        K_PASTE_START | K_EVENT | K_COMMAND | K_LUA => {
            let mut save_curbuf = Bufref {
                br_buf: crate::globals::BufPtr::NULL,
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            let tick = buf_get_changedtick(curbuf_mut());
            set_bufref(&mut save_curbuf, g_st().curbuf.opt_mut());

            if s.c == K_PASTE_START {
                paste_repeat(1);
            } else if s.c == K_EVENT {
                state_handle_k_event();
                if g_st().ins_global.dont_sync_undo == K_TRUE {
                    g_st().ins_global.dont_sync_undo = K_NONE;
                }
            } else if s.c == K_COMMAND {
                do_cmdline(None, crate::cmdexpand::LineGetter::CmdKeyCmd, 0);
            } else {
                map_execute_lua(false, false);
            }

            if g_st().pum_want.active {
                if pum_visible() {
                    g_st().edit_submode_extra = None;
                    insert_do_complete(&mut *s);
                    if g_st().pum_want.finish {
                        ins_compl_prep(CTRL_Y);
                    }
                }
                g_st().pum_want.active = false;
            }

            let changed = bufref_valid(Some(&save_curbuf))
                && save_curbuf.br_buf == g_st().curbuf
                && tick != buf_get_changedtick(curbuf_mut());
            if curbuf_mut().b_u_synced || changed {
                g_st().ins_global.need_undo = true;
            }
            if changed && !ins_compl_active() {
                mc_ins_cascade();
                mc_ins_cascade_restart();
            }
        }
        K_COMPLETE_DELAY => {
            if g_st().ins_global.dont_sync_undo == K_TRUE {
                g_st().ins_global.dont_sync_undo = K_NONE;
            }
            ins_compl_clear_autocomplete_delay();
            if ins_compl_has_autocomplete() && !char_avail() && curwin_mut().w_cursor.col != 0 {
                s.c = char_before_cursor();
                if vim_isprintc(s.c) {
                    ins_compl_enable_autocomplete();
                    ins_compl_arm_autostart();
                    insert_do_complete(&mut *s);
                }
            }
        }
        K_HOME | K_KHOME | K_S_HOME | K_C_HOME => {
            ins_home(s.c);
        }
        K_END | K_KEND | K_S_END | K_C_END => {
            ins_end(s.c);
        }
        K_LEFT => {
            if (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) != 0 {
                ins_s_left();
            } else {
                ins_left();
            }
        }
        K_S_LEFT | K_C_LEFT => {
            ins_s_left();
        }
        K_RIGHT => {
            if (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) != 0 {
                ins_s_right();
            } else {
                ins_right();
            }
        }
        K_S_RIGHT | K_C_RIGHT => {
            ins_s_right();
        }
        K_UP => {
            if pum_visible() {
                insert_do_complete(&mut *s);
            } else if (g_st().mod_mask & MOD_MASK_SHIFT) != 0 {
                ins_pageup();
            } else {
                ins_up(false);
            }
        }
        K_S_UP | K_PAGEUP | K_KPAGEUP => {
            if pum_visible() {
                insert_do_complete(&mut *s);
            } else {
                ins_pageup();
            }
        }
        K_DOWN => {
            if pum_visible() {
                insert_do_complete(&mut *s);
            } else if (g_st().mod_mask & MOD_MASK_SHIFT) != 0 {
                ins_pagedown();
            } else {
                ins_down(false);
            }
        }
        K_S_DOWN | K_PAGEDOWN | K_KPAGEDOWN => {
            if pum_visible() {
                insert_do_complete(&mut *s);
            } else {
                ins_pagedown();
            }
        }
        K_S_TAB | TAB => {
            if s.c == K_S_TAB {
                s.c = TAB;
            }
            if ctrl_x_mode_path_patterns() {
                insert_do_complete(&mut *s);
            } else {
                s.inserted_space = 0;
                if ins_tab() {
                    goto_normalchar = true;
                } else {
                    auto_format(false, true);
                }
            }
        }
        K_KENTER | CAR | NL => {
            if s.c == K_KENTER {
                s.c = CAR;
            }
            if bt_quickfix(Some(crate::globals::curbuf_mut())) && s.c == CAR {
                if curwin_mut().w_llist_ref.is_null() {
                    do_cmdline_cmd(c".cc");
                } else {
                    do_cmdline_cmd(c".ll");
                }
            } else if s.c == CAR && bt_cmdwin(g_st().curbuf.opt()) {
                cmdwin_do_action(c"confirm");
                return 0;
            } else if (g_st().mod_mask & MOD_MASK_SHIFT) == 0
                && bt_prompt(Some(crate::globals::curbuf_mut()))
            {
                prompt_invoke_callback();
                if !bt_prompt(Some(crate::globals::curbuf_mut())) {
                    return 0;
                }
            } else {
                if !ins_eol(s.c) {
                    return 0;
                }
                auto_format(false, false);
                s.inserted_space = 0;
            }
        }
        CTRL_K_CONST => {
            if ctrl_x_mode_dictionary() {
                if check_compl_option(true) {
                    insert_do_complete(&mut *s);
                }
            } else {
                s.c = ins_digraph();
                if s.c != NUL as i32 {
                    goto_normalchar = true;
                }
            }
        }
        CTRL_X_CONST => {
            ins_ctrl_x();
        }
        CTRL_RSB_CONST => {
            if !ctrl_x_mode_tags() {
                goto_normalchar = true;
            } else {
                insert_do_complete(&mut *s);
            }
        }
        CTRL_F_CONST => {
            if !ctrl_x_mode_files() {
                goto_normalchar = true;
            } else {
                insert_do_complete(&mut *s);
            }
        }
        c if c == b's' as i32 || c == CTRL_S_CONST => {
            if !ctrl_x_mode_spell() {
                goto_normalchar = true;
            } else {
                insert_do_complete(&mut *s);
            }
        }
        CTRL_L_CONST => {
            if !ctrl_x_mode_whole_line() {
                goto_normalchar = true;
            } else {
                goto_do_complete = true;
            }
        }
        CTRL_P | CTRL_N => {
            goto_do_complete = true;
        }
        CTRL_Y | CTRL_E => {
            s.c = ins_ctrl_ey(s.c);
        }
        _ => {
            goto_normalchar = true;
        }
    }

    if goto_do_complete {
        if curbuf_mut().b_p_cpt.is_empty()
            && (ctrl_x_mode_normal() || ctrl_x_mode_whole_line())
            && !compl_status_local()
        {
            goto_normalchar = true;
        } else {
            insert_do_complete(&mut *s);
        }
    }

    if goto_normalchar {
        let mut skip_char = false;
        if !opt_vars().p_paste {
            if let Some(str_) = do_insert_char_pre(s.c) {
                let str_bytes = str_.as_bytes();
                if !str_bytes.is_empty() && stop_arrow() {
                    let mut p = str_bytes;
                    while !p.is_empty() {
                        s.c = utf_ptr2char(p);
                        if s.c == CAR || s.c == K_KENTER || s.c == NL {
                            ins_eol(s.c);
                        } else {
                            ins_char(s.c);
                        }
                        p = &p[(utfc_ptr2len(p) as usize).min(p.len())..];
                    }
                    redo_append_lit(str_bytes);
                }
                s.c = NUL as i32;
            }
            if s.c == NUL as i32 {
                skip_char = true;
            }
        }

        if !skip_char {
            ins_try_si(s.c);

            if s.c == b' ' as i32 {
                s.inserted_space = 1;
                if inindent(0) {
                    g_st().ins_global.can_cindent = false;
                }
                if g_st().ins_global.start_blank_vcol == MAXCOL
                    && curwin_mut().w_cursor.lnum == g_st().ins_global.start.lnum
                {
                    g_st().ins_global.start_blank_vcol = get_nolist_virtcol();
                }
            }

            if vim_iswordc(s.c)
                || (!echeck_abbr(if s.c >= 0x100 { s.c + ABBR_OFF } else { s.c })
                    && s.c != CTRL_RSB_CONST)
            {
                insert_special(s.c, 0, 0);
                g_st().ins_global.revins_legal += 1;
                g_st().ins_global.revins_chars += 1;
            }

            auto_format(false, true);
            fold_open_cursor();
            if ins_compl_has_autocomplete() && !char_avail() && vim_isprintc(s.c) {
                let _ = trigger_autocomplete(&mut *s);
            }
        }
    }

    insert_handle_key_post(s);
    1
}

fn insert_do_complete(s: &mut InsertState) {
    ins_compl_clear_autocomplete_delay();
    ins_st(|st| st.compl_busy.set(true));
    g_st().disable_fold_update += 1;
    if !ins_complete(s.c, true) {
        compl_status_clear();
    }
    g_st().disable_fold_update -= 1;
    ins_st(|st| st.compl_busy.set(false));
    g_st().ins_global.can_si = may_do_si();
}

fn insert_do_cindent(s: &mut InsertState) {
    if in_cinkeys(s.c, b' ' as i32, s.line_is_white) && stop_arrow() {
        do_c_expr_indent();
    }
}

fn insert_handle_key_post(s: &mut InsertState) {
    if s.c != K_EVENT && s.c != K_COMPLETE_DELAY && ctrl_x_mode_normal() {
        g_st().did_cursorhold = false;
    }

    if ins_compl_active() && !ins_compl_win_active(curwin_mut()) {
        ins_compl_cancel();
    }

    if g_st().ins_global.moved != K_INS_NONE {
        s.inserted_space = 0;
    }

    if g_st().ins_global.can_cindent && cindent_on() && ctrl_x_mode_normal() {
        insert_do_cindent(s);
    }
}

/// Start inserting text.
pub fn edit(cmdchar: i32, startln: bool, count: i32) -> bool {
    if !curbuf_mut().terminal.is_null() {
        if g_st().ex_normal_busy != 0 {
            g_st().restart_edit = b'i' as i32;
            g_st().force_restart_edit = true;
            return false;
        }
        return terminal_enter();
    }

    if g_st().sandbox != 0 {
        emsg(gettext(E_SANDBOX));
        return false;
    }

    if g_st().textlock != 0
        || ins_compl_active()
        || ins_st(|st| st.compl_busy.get())
        || pum_visible()
        || expr_map_locked()
    {
        emsg(gettext(E_TEXTLOCK));
        return false;
    }

    let mut s = InsertState::new();
    s.cmdchar = cmdchar;
    s.startln = i32::from(startln);
    s.count = count;
    insert_enter(&mut s);
    s.c == CTRL_O
}

/// Redraw for Insert mode.
pub fn ins_redraw(ready: bool) {
    if char_avail() {
        return;
    }

    if ready
        && has_event(EVENT_CURSORMOVEDI)
        && (g_st().last_cursormoved_win != curwin_ptr()
            || !equalpos(g_st().last_cursormoved, curwin_mut().w_cursor))
        && !pum_visible()
    {
        if syntax_present(curwin_mut()) && g_st().must_redraw != 0 {
            update_screen();
        }
        update_curswant();
        ins_apply_autocmds(EVENT_CURSORMOVEDI);
        g_st().last_cursormoved_win = crate::globals::curwin_ptr();
        g_st().last_cursormoved = curwin_mut().w_cursor;
    }

    if ready
        && has_event(EVENT_TEXTCHANGEDI)
        && curbuf_mut().b_last_changedtick_i != buf_get_changedtick(curbuf_mut())
        && !pum_visible()
    {
        let mut aco = CtxSwitch::new();
        let tick = buf_get_changedtick(curbuf_mut());
        ctx_switch(&mut aco, None, None, Some(curbuf_mut()), 0);
        apply_autocmds(EVENT_TEXTCHANGEDI, None, None, false, curbuf_opt_mut());
        ctx_restore(&mut aco);
        curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());
        if tick != buf_get_changedtick(curbuf_mut()) {
            u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1);
        }
    }

    if ready
        && has_event(EVENT_TEXTCHANGEDP)
        && curbuf_mut().b_last_changedtick_pum != buf_get_changedtick(curbuf_mut())
        && pum_visible()
    {
        let mut aco = CtxSwitch::new();
        let tick = buf_get_changedtick(curbuf_mut());
        ctx_switch(&mut aco, None, None, Some(curbuf_mut()), 0);
        apply_autocmds(EVENT_TEXTCHANGEDP, None, None, false, curbuf_opt_mut());
        ctx_restore(&mut aco);
        curbuf_mut().b_last_changedtick_pum = buf_get_changedtick(curbuf_mut());
        if tick != buf_get_changedtick(curbuf_mut()) {
            u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1);
        }
    }

    if ready {
        may_trigger_win_scrolled_resized();
    }

    may_trigger_safestate(ready && !ins_compl_active() && !pum_visible());

    pum_check_clear();
    show_cursor_info_later(false);
    if g_st().must_redraw != 0 {
        update_screen();
    } else {
        redraw_statuslines();
        if g_st().clear_cmdline || g_st().redraw_cmdline || g_st().redraw_mode {
            showmode();
        }
    }
    setcursor();
    g_st().emsg_on_display = false;
}

fn ins_ctrl_v() {
    let mut did_putchar = false;

    ins_redraw(false);

    if redrawing() && !char_avail() {
        edit_putchar(b'^' as i32, true);
        did_putchar = true;
    }
    redo_append_str(b"\x16");

    add_to_showcmd_c(CTRL_V_CONST);

    let c = get_literal((g_st().mod_mask & MOD_MASK_SHIFT) != 0);
    if did_putchar {
        edit_unputchar();
    }
    clear_showcmd();
    insert_special(c, 1, 1);
    g_st().ins_global.revins_chars += 1;
    g_st().ins_global.revins_legal += 1;
}

pub fn edit_putchar(c: i32, highlight: bool) {
    if curwin_mut().w_grid_alloc.chars.is_empty()
        && !crate::grid::grid_allocated(&g_st().default_grid)
    {
        return;
    }

    update_topline(curwin_mut());
    validate_cursor(curwin_mut());
    let attr = if highlight {
        crate::highlight::hl_attr(HLF_8 as i32)
    } else {
        0
    };
    let pc_row = curwin_mut().w_wrow;
    let mut pc_status = PC_STATUS_UNSET;
    grid_line_start(&mut curwin_mut().w_grid, pc_row);
    let pc_col = if curwin_mut().w_onebuf_opt.wo_rl {
        let col = curwin_mut().w_view_width - 1 - curwin_mut().w_wcol;
        if grid_line_getchar(col, None) == 0 {
            grid_line_put_schar(col - 1, schar_from_ascii(b' '), attr);
            curwin_mut().w_wcol -= 1;
            pc_status = PC_STATUS_RIGHT;
        }
        col
    } else {
        let col = curwin_mut().w_wcol;
        if grid_line_getchar(col + 1, None) == 0 {
            pc_status = PC_STATUS_LEFT;
        }
        col
    };

    if pc_status == PC_STATUS_UNSET {
        let mut pc_attr = ins_st(|st| st.pc_attr.get());
        let pc_schar = grid_line_getchar(pc_col, Some(&mut pc_attr));
        pc_status = PC_STATUS_SET;
        ins_st(|st| {
            st.pc_schar.set(pc_schar);
            st.pc_attr.set(pc_attr);
        });
    }
    ins_st(|st| {
        st.pc_row.set(pc_row);
        st.pc_col.set(pc_col);
        st.pc_status.set(pc_status);
    });

    let mut buf = [0u8; MB_MAXCHAR + 1];
    let len = utf_char2bytes(c, &mut buf);
    grid_line_puts(pc_col, crate::memory::bytes_as_cstr(&buf), len, attr);
    grid_line_flush();
}

/// Return the effective prompt for the specified buffer.
pub fn buf_prompt_text(buf: &FileBuf) -> &CStr {
    buf.b_prompt_text.as_deref().unwrap_or(c"% ")
}

/// Return the effective prompt for the current buffer.
pub fn prompt_text() -> &'static CStr {
    buf_prompt_text(curbuf_mut())
}

fn init_prompt(cmdchar_todo: i32) {
    let prompt = prompt_text();
    let prompt_bytes = prompt.to_bytes();
    let prompt_len = prompt_bytes.len() as Colnr;

    if curbuf_mut().b_prompt_start.mark.lnum < 1
        || curbuf_mut().b_prompt_start.mark.lnum > curbuf_mut().b_ml.ml_line_count
    {
        curbuf_mut().b_prompt_start.mark.lnum = curbuf_mut()
            .b_prompt_start
            .mark
            .lnum
            .min(curbuf_mut().b_ml.ml_line_count)
            .max(1);
        curbuf_mut().b_prompt_append_new_line = true;
    }

    curwin_mut().w_cursor.lnum = curwin_mut()
        .w_cursor
        .lnum
        .max(curbuf_mut().b_prompt_start.mark.lnum);
    let prompt_lnum = curbuf_mut().b_prompt_start.mark.lnum;
    let text_bytes = crate::memline::ml_get_slice(prompt_lnum);
    let text_len = text_bytes.len() as Colnr;
    let mark_col = curbuf_mut().b_prompt_start.mark.col;

    if prompt_lnum == curwin_mut().w_cursor.lnum
        && (mark_col < prompt_len
            || mark_col > text_len
            || &text_bytes[(mark_col - prompt_len) as usize..mark_col as usize] != prompt_bytes)
    {
        if text_bytes.is_empty() {
            ml_replace(
                curbuf_mut().b_prompt_start.mark.lnum,
                prompt.to_bytes(),
                true,
            );
            inserted_bytes(curbuf_mut().b_prompt_start.mark.lnum, 0, 0, prompt_len);
        } else {
            let lnum = curbuf_mut().b_ml.ml_line_count;
            ml_append(lnum, prompt.to_bytes(), 0, false);
            appended_lines_mark(lnum, 1);
            curbuf_mut().b_prompt_start.mark.lnum = curbuf_mut().b_ml.ml_line_count;
            curbuf_mut().b_prompt_append_new_line = true;
            u_clearallandblockfree(curbuf_mut());
        }
        curbuf_mut().b_prompt_start.mark.col = prompt_len;
        curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
        coladvance(curwin_mut(), MAXCOL);
    }

    if g_st().ins_global.start_orig.lnum != curbuf_mut().b_prompt_start.mark.lnum
        || g_st().ins_global.start_orig.col != curbuf_mut().b_prompt_start.mark.col
    {
        set_insstart(
            curbuf_mut().b_prompt_start.mark.lnum,
            curbuf_mut().b_prompt_start.mark.col,
        );
    }

    if cmdchar_todo == b'A' as i32 {
        coladvance(curwin_mut(), MAXCOL);
    }
    if curbuf_mut().b_prompt_start.mark.lnum == curwin_mut().w_cursor.lnum {
        curwin_mut().w_cursor.col = curwin_mut()
            .w_cursor
            .col
            .max(curbuf_mut().b_prompt_start.mark.col);
    }
    check_cursor(curwin_mut());
}

/// Return true if the cursor is in the editable position of the prompt line.
pub fn prompt_curpos_editable() -> bool {
    curwin_mut().w_cursor.lnum > curbuf_mut().b_prompt_start.mark.lnum
        || (curwin_mut().w_cursor.lnum == curbuf_mut().b_prompt_start.mark.lnum
            && curwin_mut().w_cursor.col >= curbuf_mut().b_prompt_start.mark.col)
}

fn set_insstart(lnum: Linenr, col: Colnr) {
    g_st().ins_global.start.lnum = lnum;
    g_st().ins_global.start.col = col;
    g_st().ins_global.start_orig = g_st().ins_global.start;
    g_st().ins_global.start_textlen = g_st().ins_global.start.col;
    g_st().ins_global.start_blank_vcol = MAXCOL;
    g_st().ins_global.moved = K_INS_NONE;
}

pub fn edit_unputchar() {
    let (pc_status, pc_row, pc_col, pc_schar, pc_attr) = ins_st(|st| {
        (
            st.pc_status.get(),
            st.pc_row.get(),
            st.pc_col.get(),
            st.pc_schar.get(),
            st.pc_attr.get(),
        )
    });
    if pc_status != PC_STATUS_UNSET && pc_row >= 0 {
        if pc_status == PC_STATUS_RIGHT {
            curwin_mut().w_wcol += 1;
        }
        if pc_status == PC_STATUS_RIGHT || pc_status == PC_STATUS_LEFT {
            redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
        } else {
            grid_line_start(&mut curwin_mut().w_grid, pc_row);
            grid_line_put_schar(pc_col, pc_schar, pc_attr);
            grid_line_flush();
        }
    }
}

/// Display a `$` at the end of changed text when `$` is in `cpoptions`.
pub fn display_dollar(col_arg: Colnr) {
    let col = col_arg.max(0);
    if !redrawing() {
        return;
    }
    let save_col = curwin_mut().w_cursor.col;
    curwin_mut().w_cursor.col = col;

    let line_slice = crate::cursor::get_cursor_line_slice();
    curwin_mut().w_cursor.col -= crate::mbyte::utf_head_off_slice(line_slice, col as usize);
    curs_columns(curwin_mut(), false);
    if curwin_mut().w_wcol < curwin_mut().w_view_width {
        edit_putchar(b'$' as i32, false);
        g_st().dollar_vcol = curwin_mut().w_virtcol;
    }
    curwin_mut().w_cursor.col = save_col;
}

pub fn undisplay_dollar() {
    if g_st().dollar_vcol < 0 {
        return;
    }
    g_st().dollar_vcol = -1;
    redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
}

/// Truncate the space at the end of a line.
pub fn truncate_spaces(line: &mut [u8], len: usize) {
    let mut i = len.min(line.len()) as isize - 1;
    while i >= 0 && ascii_iswhite(i32::from(line[i as usize])) {
        if (g_st().state_global & REPLACE_FLAG) != 0 {
            replace_join(0);
        }
        i -= 1;
    }

    line[(i + 1) as usize] = 0;
}

/// Backspace the cursor until the given column.
pub fn backspace_until_column(col: i32) {
    while curwin_mut().w_cursor.col > col {
        curwin_mut().w_cursor.col -= 1;
        if (g_st().state_global & REPLACE_FLAG) != 0 {
            replace_do_bs(col);
        } else if !del_char_after_col(col) {
            break;
        }
    }
}

fn del_char_after_col(limit_col: i32) -> bool {
    if limit_col >= 0 {
        let ecol = curwin_mut().w_cursor.col + 1;
        mb_adjust_cursor();
        while curwin_mut().w_cursor.col < limit_col {
            let l = crate::mbyte::utf_slice2len(crate::cursor::get_cursor_pos_slice()) as Colnr;
            if l == 0 {
                break;
            }
            curwin_mut().w_cursor.col += l;
        }
        if crate::cursor::get_cursor_pos_slice().is_empty() || curwin_mut().w_cursor.col == ecol {
            return false;
        }
        del_bytes(ecol - curwin_mut().w_cursor.col, false, true);
    } else {
        del_char(false);
    }
    true
}

/// Next character is interpreted literally.
pub fn get_literal(no_simplify: bool) -> i32 {
    let mut nc: i32;
    let mut hex = false;
    let mut octal = false;
    let mut unicode: i32 = 0;

    if g_st().got_int {
        return CTRL_C_CONST;
    }

    g_st().no_mapping += 1;
    let mut cc: i32 = 0;
    let mut i: i32 = 0;
    loop {
        nc = plain_vgetc();
        if !no_simplify {
            nc = merge_modifiers(nc, &mut g_st().mod_mask);
        }
        if (g_st().mod_mask & !MOD_MASK_SHIFT) != 0 {
            break;
        }
        if (g_st().state_global & MODE_CMDLINE) == 0 && mb_byte2len_check(nc) == 1 {
            add_to_showcmd(nc);
        }
        if nc == b'x' as i32 || nc == b'X' as i32 {
            hex = true;
        } else if nc == b'o' as i32 || nc == b'O' as i32 {
            octal = true;
        } else if nc == b'u' as i32 || nc == b'U' as i32 {
            unicode = nc;
        } else {
            if hex || unicode != 0 {
                if !ascii_isxdigit(nc) {
                    break;
                }
                cc = cc.wrapping_mul(16).wrapping_add(hex2nr(nc));
            } else if octal {
                if nc < b'0' as i32 || nc > b'7' as i32 {
                    break;
                }
                cc = cc * 8 + nc - (b'0' as i32);
            } else {
                if !ascii_isdigit(nc) {
                    break;
                }
                cc = cc * 10 + nc - (b'0' as i32);
            }

            i += 1;
        }

        if cc > 255 && unicode == 0 {
            cc = 255;
        }
        nc = 0;

        if hex {
            if i >= 2 {
                break;
            }
        } else if unicode != 0 {
            if (unicode == b'u' as i32 && i >= 4) || (unicode == b'U' as i32 && i >= 8) {
                break;
            }
        } else if i >= 3 {
            break;
        }
    }
    if i == 0 {
        if nc == K_ZERO {
            cc = b'\n' as i32;
            nc = 0;
        } else {
            cc = nc;
            nc = 0;
        }
    }

    if cc == 0 {
        cc = b'\n' as i32;
    }

    g_st().no_mapping -= 1;
    if nc != 0 {
        vungetc(nc);
        g_st().mod_mask = 0;
    }
    g_st().got_int = false;
    cc
}

fn insert_special(mut c: i32, mut allow_modmask: i32, mut ctrlv: i32) {
    if (g_st().mod_mask & MOD_MASK_CMD) != 0 {
        allow_modmask = 1;
    }
    if is_special_fn(c) || (g_st().mod_mask != 0 && allow_modmask != 0) {
        let p_buf = get_special_key_name(c, g_st().mod_mask);
        let p = p_buf.to_bytes();
        let len = p.len();
        c = i32::from(p[len - 1]);
        if len > 2 {
            if !stop_arrow() {
                return;
            }
            ins_str(&p[..len - 1], len - 1);
            redo_append_lit(&p[..len - 1]);
            ctrlv = 0;
        }
    }
    if stop_arrow() {
        insertchar(c, if ctrlv != 0 { INSCHAR_CTRLV } else { 0 }, -1);
    }
}

pub fn insertchar(mut c: i32, flags: i32, second_indent: i32) {
    let force_format = (flags & INSCHAR_FORMAT) != 0;
    let textwidth = comp_textwidth(force_format);
    let fo_ins_blank = has_format_option(K_FO_INS_BLANK);

    if textwidth > 0
        && (force_format
            || (!ascii_iswhite(c)
                && !((g_st().state_global & REPLACE_FLAG) != 0
                    && (g_st().state_global & VREPLACE_FLAG) == 0
                    && !crate::cursor::get_cursor_pos_slice().is_empty())
                && (curwin_mut().w_cursor.lnum != g_st().ins_global.start.lnum
                    || ((!has_format_option(K_FO_INS_LONG)
                        || g_st().ins_global.start_textlen <= textwidth)
                        && (!fo_ins_blank || g_st().ins_global.start_blank_vcol <= textwidth)))))
    {
        let mut do_internal = true;
        let virtcol =
            get_nolist_virtcol() + char2cells(if c != NUL as i32 { c } else { gchar_cursor() });

        if curbuf_mut().b_p_fex.cb_type() != K_CALLBACK_NONE_CONST
            && (flags & INSCHAR_NO_FEX) == 0
            && (force_format || virtcol > textwidth)
        {
            do_internal = fex_format(curwin_mut().w_cursor.lnum, 1, c) != 0;
            g_st().ins_global.need_undo = true;
        }
        if do_internal {
            internal_format(textwidth, second_indent, flags, c == NUL as i32, c);
        }
    }

    if c == NUL as i32 {
        return;
    }

    if g_st().ins_global.did_ai && c == g_st().ins_global.end_comment_pending {
        let mut lead_end = [0u8; COM_MAX_LEN];
        let mut flags_opt: Option<&'static CStr> = None;
        let line_bytes = get_cursor_line_ptr().to_bytes_with_nul();
        let mut i = get_leader_len(
            crate::cursor::get_cursor_line_slice(),
            Some(&mut flags_opt),
            false,
            true,
        );
        if i > 0 && crate::strings::vim_haschr((flags_opt).unwrap_or(c""), i32::from(COM_MIDDLE)) {
            let mut p = flags_opt.map_or(&[][..], CStr::to_bytes);
            if let Some(colon_pos) = p.iter().position(|&b| b == b':') {
                p = &p[colon_pos + 1..];
            } else {
                p = &[];
            }
            let mut middle_len =
                copy_option_part(&mut p, &mut lead_end[..COM_MAX_LEN], c",") as i32;
            while middle_len > 0 && ascii_iswhite(i32::from(lead_end[(middle_len - 1) as usize])) {
                middle_len -= 1;
            }

            if let Some(colon_pos) = p.iter().position(|&b| b == b':') {
                p = &p[colon_pos + 1..];
            } else {
                p = &[];
            }
            let end_len = copy_option_part(&mut p, &mut lead_end[..COM_MAX_LEN], c",") as i32;

            i = curwin_mut().w_cursor.col;
            let line_slice = &line_bytes[..(i.max(0) as usize).min(line_bytes.len())];
            while {
                i -= 1;
                i >= 0 && ascii_iswhite(i32::from(line_slice[i as usize]))
            } {}
            i += 1;
            i -= middle_len;

            if i >= 0
                && end_len > 0
                && i32::from(lead_end[(end_len - 1) as usize])
                    == g_st().ins_global.end_comment_pending
            {
                backspace_until_column(i);
                ins_bytes_len(&lead_end, (end_len - 1) as usize);
            }
        }
    }
    g_st().ins_global.end_comment_pending = NUL as i32;

    g_st().ins_global.did_ai = false;
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;

    if !is_special_ins(c)
        && utf_char2len(c) == 1
        && !has_event(EVENT_INSERTCHARPRE)
        && !test_disable_char_avail.load(std::sync::atomic::Ordering::Relaxed)
        && vpeekc() != NUL as i32
        && (g_st().state_global & REPLACE_FLAG) == 0
        && !cindent_on()
        && !opt_vars().p_ri
    {
        let mut buf = [0u8; INPUT_BUFLEN + 1];
        let mut virtcol: Colnr = 0;

        buf[0] = c as u8;
        let mut i: usize = 1;
        if textwidth > 0 {
            virtcol = get_nolist_virtcol();
        }
        while {
            c = vpeekc();
            c != NUL as i32
                && !is_special_ins(c)
                && utf_byte2len(c) == 1
                && i < INPUT_BUFLEN
                && (textwidth == 0 || {
                    virtcol += ascii2cells(i32::from(buf[i - 1]));
                    virtcol < textwidth
                })
                && !(!g_st().no_abbr && !vim_iswordc(c) && vim_iswordc(i32::from(buf[i - 1])))
        } {
            c = vgetc();
            buf[i] = c as u8;
            i += 1;
        }

        do_digraph(-1);
        do_digraph(i32::from(buf[i - 1]));
        buf[i] = 0;
        ins_str(&buf, i);
        let start_idx = if (flags & INSCHAR_CTRLV) != 0 {
            redo_literal(i32::from(buf[0]));
            1
        } else {
            0
        };
        if buf[start_idx] != 0 {
            redo_append_lit(&buf[start_idx..i]);
        }
    } else {
        let cc = utf_char2len(c);
        if cc > 1 {
            let mut buf = [0u8; MB_MAXCHAR + 1];
            utf_char2bytes(c, &mut buf);
            buf[cc as usize] = 0;
            ins_char_bytes(&buf, cc as usize);
            redo_append_char(c);
        } else {
            ins_char(c);
            if (flags & INSCHAR_CTRLV) != 0 {
                redo_literal(c);
            } else {
                redo_append_char(c);
            }
        }
    }
}

fn redo_literal(c: i32) {
    if ascii_isdigit(c) {
        let mut buf = [0u8; 10];
        let n = format_to_buf!(&mut buf, c"%03d", c);
        redo_append_str(&buf[..n.max(0) as usize]);
    } else {
        redo_append_char(c);
    }
}

/// Called when the cursor moves in insert-mode without editing text.
pub fn start_arrow(end_insert_pos: Option<CursorPos>, end_change: bool, move_key: i32) {
    if g_st().ins_global.moved == K_INS_NONE && end_change {
        stop_insert(end_insert_pos, 0, 0);
    }
    if g_st().ins_global.moved != K_INS_JUMP {
        if move_key == NUL as i32 {
            redo_append_str(b"\x1b");
            g_st().ins_global.moved = K_INS_JUMP;
        } else {
            if !is_special_fn(move_key) {
                redo_append_char(CTRL_G);
            }
            redo_append_char(move_key);
            if end_change {
                g_st().ins_global.moved = K_INS_ARROW;
            }
        }
    }
    check_spell_redraw();
}

fn start_arrow_with_change(end_insert_pos: CursorPos, end_change: bool, move_key: i32) {
    if !end_change && g_st().ins_global.moved != K_INS_JUMP {
        redo_append_char(CTRL_G);
        redo_append_char(b'U' as i32);
    }
    start_arrow(Some(end_insert_pos), end_change, move_key);
}

fn check_spell_redraw() {
    if g_st().spell_redraw_lnum != 0 {
        let lnum = g_st().spell_redraw_lnum;
        g_st().spell_redraw_lnum = 0;
        redraw_winline(crate::globals::curwin_mut(), lnum);
    }
}

pub fn stop_arrow() -> bool {
    if g_st().ins_global.moved != K_INS_NONE {
        g_st().ins_global.start = curwin_mut().w_cursor;
        if g_st().ins_global.start.col > g_st().ins_global.start_orig.col
            && !g_st().ins_global.need_undo
        {
            g_st().ins_global.update_start_orig = false;
        }
        g_st().ins_global.start_textlen = linetabsize_str(crate::cursor::get_cursor_line_slice());

        let jumped = g_st().ins_global.moved == K_INS_JUMP;
        if u_save_cursor() {
            g_st().ins_global.moved = K_INS_NONE;
            g_st().ins_global.need_undo = false;
        }
        g_st().ins_global.ai_col = 0;
        if (g_st().state_global & VREPLACE_FLAG) != 0 {
            g_st().orig_line_count = curbuf_mut().b_ml.ml_line_count;
            g_st().vr_lines_changed = 1;
        }
        if jumped {
            prep_redo(
                false,
                false,
                CmdSpec {
                    count: 1,
                    cmd: b'i' as i32,
                    ..CmdSpec::default()
                },
            );
            g_st().ins_global.new_insert_skip = 2;
            mc_ins_cascade_restart();
        } else {
            let redo = redo_keys(None);
            g_st().ins_global.new_insert_skip =
                redo.as_ref().map_or(0, |s| s.as_bytes().len() as i32);
        }
    } else if g_st().ins_global.need_undo && u_save_cursor() {
        if lt(curwin_mut().w_cursor, g_st().ins_global.start) {
            g_st().ins_global.start = curwin_mut().w_cursor;
            g_st().ins_global.start_textlen =
                linetabsize_str(crate::cursor::get_cursor_line_slice());
        }
        g_st().ins_global.need_undo = false;
    }

    fold_open_cursor();

    !(g_st().ins_global.moved != K_INS_NONE || g_st().ins_global.need_undo)
}

fn stop_insert(end_insert_pos: Option<CursorPos>, esc: i32, nomove: i32) {
    stop_redo_ins();
    ins_st(|st| *st.replace_stack.borrow_mut() = Vec::new());

    let redo_cstr = redo_keys(None);
    let added = match redo_cstr {
        None => 0,
        Some(ref s) => (s.as_bytes().len() as i32) - g_st().ins_global.new_insert_skip,
    };
    if g_st().ins_global.did_restart_edit == 0 || added > 0 {
        let skip = if added < 0 {
            0
        } else {
            g_st().ins_global.new_insert_skip
        };
        ins_st(|st| {
            *st.last_insert.borrow_mut() = redo_cstr;
            st.last_insert_skip.set(skip);
        });
    }

    if g_st().ins_global.moved == K_INS_NONE
        && let Some(end_pos) = end_insert_pos
    {
        let mut cc: i32;
        if !g_st().ins_global.need_undo && has_format_option(K_FO_AUTO) {
            let tpos = curwin_mut().w_cursor;
            cc = b'x' as i32;
            if curwin_mut().w_cursor.col > 0 && gchar_cursor() == NUL as i32 {
                dec_cursor();
                cc = gchar_cursor();
                if !ascii_iswhite(cc) {
                    curwin_mut().w_cursor = tpos;
                }
            }

            auto_format(true, false);

            if ascii_iswhite(cc) {
                if gchar_cursor() != NUL as i32 {
                    inc_cursor();
                }
                if gchar_cursor() == NUL as i32
                    && curwin_mut().w_cursor.lnum == tpos.lnum
                    && curwin_mut().w_cursor.col == tpos.col
                {
                    curwin_mut().w_cursor.coladd = tpos.coladd;
                }
            }
        }

        check_auto_format(true);

        if nomove == 0
            && g_st().ins_global.did_ai
            && !mc_ins_replaying()
            && (esc != 0
                || (!crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_INDENT)
                    && curwin_mut().w_cursor.lnum != end_pos.lnum))
            && end_pos.lnum <= curbuf_mut().b_ml.ml_line_count
        {
            let mut tpos = curwin_mut().w_cursor;
            let prev_col = end_pos.col;

            curwin_mut().w_cursor = end_pos;
            check_cursor_col(curwin_mut());

            let mut strip_col = curwin_mut().w_cursor.col;
            if gchar_cursor() == NUL as i32 && strip_col > 0 {
                strip_col -= 1;
            }

            if crate::charset::skipwhite_slice(
                &crate::cursor::get_cursor_line_slice()[strip_col as usize..],
            )
            .is_empty()
            {
                curwin_mut().w_cursor.col = strip_col;
                loop {
                    if gchar_cursor() == NUL as i32 && curwin_mut().w_cursor.col > 0 {
                        curwin_mut().w_cursor.col -= 1;
                    }
                    cc = gchar_cursor();
                    if !ascii_iswhite(cc) {
                        break;
                    }
                    if !del_char(true) {
                        break;
                    }
                }
                if curwin_mut().w_cursor.lnum != tpos.lnum {
                    curwin_mut().w_cursor = tpos;
                } else if curwin_mut().w_cursor.col < prev_col {
                    tpos = curwin_mut().w_cursor;
                    tpos.col += 1;
                    if cc != NUL as i32 && gchar_pos(&mut tpos) == NUL as i32 {
                        curwin_mut().w_cursor.col += 1;
                    }
                }

                if g_st().visual_global.active {
                    check_visual_pos();
                }
            } else {
                curwin_mut().w_cursor = tpos;
            }
        }
    }
    g_st().ins_global.did_ai = false;
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;

    if let Some(end_pos) = end_insert_pos {
        curbuf_mut().b_op_start = g_st().ins_global.start;
        curbuf_mut().b_op_start_orig = g_st().ins_global.start_orig;
        curbuf_mut().b_op_end = end_pos;
    }
}

/// Set the last inserted text to a single character.
pub fn set_last_insert(c: i32) {
    let mut buf = [0u8; MB_MAXBYTES * 3 + 5];
    let mut len = 0usize;
    if c < b' ' as i32 || c == DEL {
        buf[len] = CTRL_V_CONST as u8;
        len += 1;
    }
    len += add_char2buf(c, &mut buf[len..]);
    buf[len] = ESC as u8;
    len += 1;
    let cstr = crate::memory::bytes_to_cstring(&buf[..len]);
    ins_st(|st| {
        *st.last_insert.borrow_mut() = Some(cstr);
        st.last_insert_skip.set(0);
    });
}

pub fn free_last_insert() {
    ins_st(|st| *st.last_insert.borrow_mut() = None);
}

pub fn beginline(flags: i32) {
    if (flags & BL_SOL) != 0 && !opt_vars().p_sol {
        coladvance(curwin_mut(), curwin_mut().w_curswant);
    } else {
        curwin_mut().w_cursor.col = 0;
        curwin_mut().w_cursor.coladd = 0;

        if (flags & (BL_WHITE | BL_SOL)) != 0 {
            let mut rem = crate::cursor::get_cursor_line_slice();
            while let Some(&b0) = rem.first()
                && ascii_iswhite(i32::from(b0))
                && !((flags & BL_FIX) != 0 && rem.len() == 1)
            {
                curwin_mut().w_cursor.col += 1;
                rem = &rem[1..];
            }
        }
        curwin_mut().w_set_curswant = true;
    }
    adjust_skipcol();
}

pub fn oneright() -> bool {
    if virtual_active(curwin_mut()) {
        let prevpos = curwin_mut().w_cursor;
        let ptr_slice = crate::cursor::get_cursor_pos_slice();
        let step = if ptr_slice.first().copied().unwrap_or(0) != TAB as u8
            && vim_isprintc(utf_ptr2char(ptr_slice))
        {
            ptr2cells(ptr_slice)
        } else {
            1
        };
        coladvance(curwin_mut(), getviscol() + step);
        curwin_mut().w_set_curswant = true;
        return prevpos.col != curwin_mut().w_cursor.col
            || prevpos.coladd != curwin_mut().w_cursor.coladd;
    }

    let ptr_slice = crate::cursor::get_cursor_pos_slice();
    if ptr_slice.is_empty() {
        return false;
    }

    let l = utfc_ptr2len(ptr_slice);
    if ptr_slice.len() <= l as usize && (get_ve_flags(curwin_mut()) & K_OPT_VE_FLAG_ONEMORE) == 0 {
        return false;
    }
    curwin_mut().w_cursor.col += l;
    curwin_mut().w_set_curswant = true;
    adjust_skipcol();
    true
}

pub fn oneleft() -> bool {
    if virtual_active(curwin_mut()) {
        let v = getviscol();
        if v == 0 {
            return false;
        }
        let mut width: i32 = 1;
        loop {
            coladvance(curwin_mut(), v - width);
            if getviscol() < v {
                break;
            }
            width += 1;
        }

        if curwin_mut().w_cursor.coladd == 1 {
            let ptr_slice = crate::cursor::get_cursor_pos_slice();
            if ptr_slice.first().copied().unwrap_or(0) != TAB as u8
                && vim_isprintc(utf_ptr2char(ptr_slice))
                && ptr2cells(ptr_slice) > 1
            {
                curwin_mut().w_cursor.coladd = 0;
            }
        }

        curwin_mut().w_set_curswant = true;
        adjust_skipcol();
        return true;
    }

    if curwin_mut().w_cursor.col == 0 {
        return false;
    }

    curwin_mut().w_set_curswant = true;
    curwin_mut().w_cursor.col -= 1;

    mb_adjust_cursor();
    adjust_skipcol();
    true
}

/// Move the cursor up `n` lines in window `wp`.
pub fn cursor_up_inner(wp: &mut WinInfoInfo, mut n: Linenr, skip_conceal: bool) {
    let mut lnum = wp.w_cursor.lnum;

    if n >= lnum {
        lnum = 1;
    } else if win_lines_concealed(&mut *wp) {
        has_folding(&mut *wp, lnum, Some(&mut lnum), None);
        while n > 0 {
            n -= 1;
            lnum -= 1;
            if lnum <= 1 {
                break;
            }
            if skip_conceal && decor_conceal_line(&mut *wp, lnum - 1, true) {
                n += 1;
            }
            if n > 0
                || !((g_st().state_global & MODE_INSERT) != 0
                    || (opt_vars().fdo_flags & K_OPT_FDO_FLAG_ALL) != 0)
            {
                has_folding(&mut *wp, lnum, Some(&mut lnum), None);
            }
        }
        lnum = lnum.max(1);
    } else {
        lnum -= n;
    }

    wp.w_cursor.lnum = lnum;
}

pub fn cursor_up(n: Linenr, upd_topline: bool) -> bool {
    if n > 0 && curwin_mut().w_cursor.lnum <= 1 {
        return false;
    }
    cursor_up_inner(curwin_mut(), n, false);
    coladvance(curwin_mut(), curwin_mut().w_curswant);
    if upd_topline {
        update_topline(curwin_mut());
    }
    true
}

/// Move the cursor down `n` lines in window `wp`.
pub fn cursor_down_inner(wp: &mut WinInfoInfo, mut n: i32, skip_conceal: bool) {
    let mut lnum = wp.w_cursor.lnum;
    let line_count = wp.buf_mut().b_ml.ml_line_count;

    if lnum + n >= line_count {
        lnum = line_count;
    } else if win_lines_concealed(&mut *wp) {
        let mut last: Linenr = 0;
        while n > 0 {
            n -= 1;
            if has_folding_win(wp, lnum, None, Some(&mut last), true, None) {
                lnum = last + 1;
            } else {
                lnum += 1;
            }
            if lnum >= line_count {
                break;
            }
            if skip_conceal && decor_conceal_line(&mut *wp, lnum - 1, true) {
                n += 1;
            }
        }
        lnum = lnum.min(line_count);
    } else {
        lnum += n;
    }

    wp.w_cursor.lnum = lnum;
}

pub fn cursor_down(n: i32, upd_topline: bool) -> bool {
    let mut lnum = curwin_mut().w_cursor.lnum;
    has_folding_win(curwin_mut(), lnum, None, Some(&mut lnum), true, None);
    if n > 0 && lnum >= curwin_mut().buf_mut().b_ml.ml_line_count {
        return false;
    }
    cursor_down_inner(curwin_mut(), n, false);
    coladvance(curwin_mut(), curwin_mut().w_curswant);
    if upd_topline {
        update_topline(curwin_mut());
    }
    true
}

/// Stuff the last inserted text in the read buffer.
pub fn stuff_inserted(c: i32, mut count: i32, no_esc: i32) -> bool {
    let mut last: u8 = 0;
    let Some(insert_cstr) = get_last_insert() else {
        emsg(gettext(E_NOINSTEXT));
        return false;
    };
    let mut insert = insert_cstr.to_bytes();

    if c != NUL as i32 {
        stuffchar_readbuff(c);
    }

    if let Some(pos) = insert.iter().rposition(|&b| b == ESC as u8) {
        insert = &insert[..pos];
    }

    if let Some(&b) = insert.last()
        && (b == b'0' || b == b'^')
        && (no_esc != 0 || (insert.first() == Some(&(CTRL_D_CONST as u8)) && count > 1))
    {
        last = b;
        insert = &insert[..insert.len() - 1];
    }

    loop {
        stuff_readbuff_len(insert);
        if last == b'0' {
            stuff_readbuff_len(b"\x16048");
        } else if last == b'^' {
            stuff_readbuff_len(b"\x16^");
        }
        count -= 1;
        if count <= 0 {
            break;
        }
    }

    if no_esc == 0 {
        stuffchar_readbuff(ESC);
    }

    true
}

pub fn get_last_insert() -> Option<std::ffi::CString> {
    ins_st(|st| {
        st.last_insert.borrow().as_ref().map(|li| {
            let skip = st.last_insert_skip.get() as usize;
            let bytes = li.to_bytes();
            let skip = skip.min(bytes.len());
            std::ffi::CString::new(&bytes[skip..]).unwrap_or_default()
        })
    })
}

pub fn get_last_insert_save() -> Option<std::ffi::CString> {
    let insert = get_last_insert()?;
    let mut slice = insert.to_bytes();
    if slice.last() == Some(&(ESC as u8)) {
        slice = &slice[..slice.len() - 1];
    }
    Some(std::ffi::CString::new(slice).unwrap_or_default())
}

fn echeck_abbr(c: i32) -> bool {
    if opt_vars().p_paste || g_st().no_abbr || g_st().ins_global.moved != K_INS_NONE {
        return false;
    }
    check_abbr(
        c,
        get_cursor_line_ptr(),
        curwin_mut().w_cursor.col,
        if curwin_mut().w_cursor.lnum == g_st().ins_global.start.lnum {
            g_st().ins_global.start.col
        } else {
            0
        },
    )
}

/// Push character that is replaced onto the replace stack.
pub fn replace_push(slice: &[u8]) {
    let off = g_st().replace_offset as usize;
    ins_st(|st| {
        let mut stack = st.replace_stack.borrow_mut();
        if stack.len() < off {
            return;
        }
        let pos = stack.len() - off;
        stack.splice(pos..pos, slice.iter().copied());
    });
}

pub fn replace_push_nul() {
    replace_push(&[0]);
}

fn replace_pop_if_nul() -> i32 {
    ins_st(|st| {
        let mut stack = st.replace_stack.borrow_mut();
        let ch = stack.last().map_or(-1, |&b| i32::from(b));
        if ch == 0 {
            stack.pop();
        }
        ch
    })
}

pub fn replace_join(mut off: i32) {
    ins_st(|st| {
        let mut stack = st.replace_stack.borrow_mut();
        let mut i = stack.len();
        while i > 0 {
            i -= 1;
            if stack[i] == 0 {
                let cur_off = off;
                off -= 1;
                if cur_off <= 0 {
                    stack.remove(i);
                    return;
                }
            }
        }
    });
}

fn replace_pop_ins() {
    let old_state = g_st().state_global;
    g_st().state_global = MODE_NORMAL;
    while replace_pop_if_nul() > 0 {
        mb_replace_pop_ins();
        dec_cursor();
    }
    g_st().state_global = old_state;
}

fn mb_replace_pop_ins() {
    let popped = ins_st(|st| {
        let mut stack = st.replace_stack.borrow_mut();
        let size = stack.len();
        let len = (crate::mbyte::utf_head_off(&stack, size - 1) + 1) as usize;
        let new_size = size - len;
        stack.split_off(new_size)
    });
    let len = popped.len();
    ins_bytes_len(&popped, len);
}

fn replace_do_bs(limit_col: i32) {
    let mut start_vcol: Colnr = 0;
    let l_state = g_st().state_global;

    let cc = replace_pop_if_nul();
    if cc > 0 {
        let mut orig_len: i32 = 0;
        let mut orig_vcols: i32 = 0;
        if (l_state & VREPLACE_FLAG) != 0 {
            let mut cursor = curwin_mut().w_cursor;
            getvcol(
                curwin_mut(),
                &mut cursor,
                None,
                Some(&mut start_vcol),
                None,
                0,
            );
            curwin_mut().w_cursor = cursor;
            orig_vcols = win_chartabsize(curwin_mut(), get_cursor_pos_ptr().to_bytes(), start_vcol);
        }
        del_char_after_col(limit_col);
        if (l_state & VREPLACE_FLAG) != 0 {
            orig_len = get_cursor_pos_len();
        }
        replace_pop_ins();

        if (l_state & VREPLACE_FLAG) != 0 {
            let p_slice = crate::cursor::get_cursor_pos_slice();
            let ins_len = get_cursor_pos_len() - orig_len;
            let mut vcol = start_vcol;
            let mut i: i32 = 0;
            while i < ins_len {
                let rem = &p_slice[(i as usize).min(p_slice.len())..];
                vcol += win_chartabsize(curwin_mut(), rem, vcol);
                i += utfc_ptr2len(rem);
            }
            vcol -= start_vcol;

            curwin_mut().w_cursor.col += ins_len;
            while vcol > orig_vcols && gchar_cursor() == b' ' as i32 {
                del_char(false);
                orig_vcols += 1;
            }
            curwin_mut().w_cursor.col -= ins_len;
        }

        changed_bytes(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.col);
    } else if cc == 0 {
        del_char_after_col(limit_col);
    }
}

fn ins_reg() {
    let mut need_redraw = false;
    let mut literally: i32 = 0;
    let vis_active = g_st().visual_global.active;

    ins_st(|st| st.pc_status.set(PC_STATUS_UNSET));
    if redrawing() && !char_avail() {
        ins_redraw(false);
        edit_putchar(b'"' as i32, true);
        add_to_showcmd_c(CTRL_R_CONST);
    }

    g_st().no_mapping += 1;
    g_st().allow_keys += 1;
    let mut regname = plain_vgetc();
    langmap_adjust(&mut regname, true);
    if regname == CTRL_R_CONST || regname == CTRL_O || regname == CTRL_P {
        literally = regname;
        add_to_showcmd_c(literally);
        regname = plain_vgetc();
        langmap_adjust(&mut regname, true);
    }
    g_st().no_mapping -= 1;
    g_st().allow_keys -= 1;

    g_st().no_u_sync += 1;
    if regname == b'=' as i32 {
        let curpos = curwin_mut().w_cursor;
        g_st().u_sync_once = 2;
        regname = get_expr_register();
        curwin_mut().w_cursor = curpos;
        check_cursor(curwin_mut());
    }
    if regname == NUL as i32 || !valid_yank_reg(regname, false) {
        vim_beep(K_OPT_BO_FLAG_REGISTER);
        need_redraw = true;
    } else {
        let reg = get_yank_register(regname, YankRegMode::Paste);

        if literally == CTRL_O || literally == CTRL_P {
            redo_append_char(CTRL_R_CONST);
            redo_append_char(literally);
            redo_append_char(regname);

            do_put(
                regname,
                None,
                BACKWARD,
                1,
                (if literally == CTRL_P {
                    PUT_FIXINDENT
                } else {
                    0
                }) | PUT_CURSEND,
            );
        } else if reg.with(|r| r.y_array.len() > 1) && is_literal_register(regname) {
            redo_append_char(CTRL_R_CONST);
            redo_append_char(regname);
            do_put(regname, None, BACKWARD, 1, PUT_CURSEND);
        } else if !insert_reg(regname, None, literally != 0) {
            vim_beep(K_OPT_BO_FLAG_REGISTER);
            need_redraw = true;
        } else if g_st().ins_global.stop_insert_mode {
            need_redraw = true;
        }
    }
    g_st().no_u_sync -= 1;
    if g_st().u_sync_once == 1 {
        g_st().ins_global.need_undo = true;
    }
    g_st().u_sync_once = 0;

    if need_redraw || stuff_empty() {
        edit_unputchar();
    }
    clear_showcmd();

    if !vis_active && g_st().visual_global.active {
        end_visual_mode();
    }
}

fn ins_ctrl_g() {
    setcursor();

    g_st().no_mapping += 1;
    g_st().allow_keys += 1;
    let c = plain_vgetc();
    g_st().no_mapping -= 1;
    g_st().allow_keys -= 1;
    match c {
        K_UP | CTRL_K_CONST => ins_up(true),
        c if c == b'k' as i32 => ins_up(true),
        K_DOWN | CTRL_J_CONST => ins_down(true),
        c if c == b'j' as i32 => ins_down(true),
        c if c == b'u' as i32 => {
            u_sync(true);
            g_st().ins_global.need_undo = true;
            g_st().ins_global.update_start_orig = false;
            g_st().ins_global.start = curwin_mut().w_cursor;
        }
        c if c == b'U' as i32 => {
            g_st().ins_global.dont_sync_undo = K_NONE;
        }
        ESC => {}
        _ => {
            vim_beep(K_OPT_BO_FLAG_CTRLG);
        }
    }
}

fn ins_ctrl_hat() {
    if map_to_exists_mode(c"", MODE_LANGMAP, false) {
        if (g_st().state_global & MODE_LANGMAP) != 0 {
            curbuf_mut().b_p_iminsert = B_IMODE_NONE;
            g_st().state_global &= !MODE_LANGMAP;
        } else {
            curbuf_mut().b_p_iminsert = B_IMODE_LMAP;
            g_st().state_global |= MODE_LANGMAP;
        }
    }
    set_iminsert_global(curbuf_mut());
    showmode();
    status_redraw_curbuf();
}

fn ins_esc(count: &mut i32, cmdchar: i32, nomove: bool) -> bool {
    check_spell_redraw();

    let temp = curwin_mut().w_cursor.col;
    if ins_st(|st| st.ins_esc_disabled_redraw.replace(false)) {
        g_st().redrawing_disabled -= 1;
    }
    if g_st().ins_global.moved == K_INS_NONE {
        if cmdchar != b'r' as i32 && cmdchar != b'v' as i32 {
            redo_append_str(b"\x1b");
        }

        if *count > 0 {
            line_breakcheck();
            if g_st().got_int {
                *count = 0;
            }
        }

        *count -= 1;
        if *count > 0 {
            if crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_REPLCNT) {
                g_st().state_global &= !REPLACE_FLAG;
            }

            start_redo_ins();
            if cmdchar == b'r' as i32 || cmdchar == b'v' as i32 {
                stuff_redo_readbuff(c"\x1b");
            }
            g_st().redrawing_disabled += 1;
            ins_st(|st| st.ins_esc_disabled_redraw.set(true));
            return false;
        }
        stop_insert(Some(curwin_mut().w_cursor), 1, i32::from(nomove));
        undisplay_dollar();
    } else if g_st().ins_global.moved == K_INS_ARROW
        && cmdchar != b'r' as i32
        && cmdchar != b'v' as i32
    {
        redo_append_str(b"\x1b");
    }

    if cmdchar != b'r' as i32 && cmdchar != b'v' as i32 {
        ins_apply_autocmds(EVENT_INSERTLEAVEPRE);
    }

    if g_st().restart_edit == NUL as i32 && temp == curwin_mut().w_cursor.col {
        curwin_mut().w_set_curswant = true;
    }

    if (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 {
        let view = mark_view_make(curwin_mut(), curwin_mut().w_cursor);
        reset_fmark(
            &mut curbuf_mut().b_last_insert,
            curwin_mut().w_cursor,
            curbuf_mut().b_fnum,
            view,
        );
    }

    if !nomove
        && (curwin_mut().w_cursor.col != 0 || curwin_mut().w_cursor.coladd > 0)
        && (g_st().restart_edit == NUL as i32
            || (gchar_cursor() == NUL as i32 && !g_st().visual_global.active))
        && !g_st().ins_global.revins_on
    {
        if curwin_mut().w_cursor.coladd > 0 || get_ve_flags(curwin_mut()) == K_OPT_VE_FLAG_ALL {
            oneleft();
            if g_st().restart_edit != NUL as i32 {
                curwin_mut().w_cursor.coladd += 1;
            }
        } else {
            curwin_mut().w_cursor.col -= 1;
            curwin_mut().w_valid &= !(VALID_WCOL | VALID_VIRTCOL);
            mb_adjust_cursor();
        }
    }

    g_st().state_global = MODE_NORMAL;
    may_trigger_modechanged();
    if gchar_cursor() == TAB || buf_meta_total(curbuf_mut(), K_MTMETA_INLINE) > 0 {
        curwin_mut().w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
    }

    setmouse();
    ui_cursor_shape();

    if g_st().reg_recording != 0 || g_st().restart_edit != NUL as i32 {
        showmode();
    } else if opt_vars().p_smd
        && (g_st().got_int || !skip_showmode())
        && !(opt_vars().p_ch == 0 && !ui_has(K_UIMESSAGES))
    {
        unshowmode(false);
    }
    true
}

fn ins_ctrl_() {
    if g_st().ins_global.revins_on
        && g_st().ins_global.revins_chars != 0
        && g_st().ins_global.revins_scol >= 0
    {
        while gchar_cursor() != NUL as i32 && g_st().ins_global.revins_chars != 0 {
            g_st().ins_global.revins_chars -= 1;
            curwin_mut().w_cursor.col += 1;
        }
    }
    opt_vars().p_ri = !opt_vars().p_ri;
    g_st().ins_global.revins_on = g_st().state_global == MODE_INSERT && opt_vars().p_ri;
    if g_st().ins_global.revins_on {
        g_st().ins_global.revins_scol = curwin_mut().w_cursor.col;
        g_st().ins_global.revins_legal += 1;
        g_st().ins_global.revins_chars = 0;
        undisplay_dollar();
    } else {
        g_st().ins_global.revins_scol = -1;
    }
    showmode();
}

fn ins_start_select(c: i32) -> bool {
    if !g_st().km_startsel {
        return false;
    }
    match c {
        K_KHOME | K_KEND | K_PAGEUP | K_KPAGEUP | K_PAGEDOWN | K_KPAGEDOWN
            if (g_st().mod_mask & MOD_MASK_SHIFT) == 0 =>
        {
            false
        }
        K_KHOME | K_KEND | K_PAGEUP | K_KPAGEUP | K_PAGEDOWN | K_KPAGEDOWN | K_S_LEFT
        | K_S_RIGHT | K_S_UP | K_S_DOWN | K_S_END | K_S_HOME => {
            start_selection();
            stuffchar_readbuff(CTRL_O);
            if g_st().mod_mask != 0 {
                let buf: [u8; 3] = [K_SPECIAL, KS_MODIFIER, g_st().mod_mask as u8];
                stuff_readbuff_len(&buf);
            }
            stuffchar_readbuff(c);
            true
        }
        _ => false,
    }
}

fn ins_insert(replace_state: i32) {
    let mode_str = if (g_st().state_global & REPLACE_FLAG) != 0 {
        c"i"
    } else if replace_state == MODE_VREPLACE {
        c"v"
    } else {
        c"r"
    };
    set_vim_var_string(VV_INSERTMODE, Some(mode_str), 1);
    ins_apply_autocmds(EVENT_INSERTCHANGE);
    if (g_st().state_global & REPLACE_FLAG) != 0 {
        g_st().state_global = MODE_INSERT | (g_st().state_global & MODE_LANGMAP);
    } else {
        g_st().state_global = replace_state | (g_st().state_global & MODE_LANGMAP);
    }
    may_trigger_modechanged();
    redo_append_char(K_INS);
    showmode();
    ui_cursor_shape();
}

fn ins_ctrl_o() {
    g_st().visual_global.restart_select = 0;
    if (g_st().state_global & VREPLACE_FLAG) != 0 {
        g_st().restart_edit = b'V' as i32;
    } else if (g_st().state_global & REPLACE_FLAG) != 0 {
        g_st().restart_edit = b'R' as i32;
    } else {
        g_st().restart_edit = b'I' as i32;
    }
    if virtual_active(curwin_mut()) {
        g_st().ins_at_eol = false;
    } else {
        g_st().ins_at_eol = gchar_cursor() == NUL as i32;
    }
}

fn ins_shift(c: i32, lastc: i32) {
    if !stop_arrow() {
        return;
    }
    redo_append_char(c);

    if c == CTRL_D_CONST
        && (lastc == b'0' as i32 || lastc == b'^' as i32)
        && curwin_mut().w_cursor.col > 0
    {
        curwin_mut().w_cursor.col -= 1;
        del_char(false);
        if (g_st().state_global & REPLACE_FLAG) != 0 {
            replace_pop_ins();
        }
        if lastc == b'^' as i32 {
            g_st().old_indent = get_indent();
        }
        change_indent(INDENT_SET, 0, 1, true);
    } else {
        change_indent(
            if c == CTRL_D_CONST {
                INDENT_DEC
            } else {
                INDENT_INC
            },
            0,
            1,
            true,
        );
    }

    if g_st().ins_global.did_ai
        && !crate::charset::skipwhite_slice(crate::cursor::get_cursor_line_slice()).is_empty()
    {
        g_st().ins_global.did_ai = false;
    }
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;
    g_st().ins_global.can_cindent = false;
}

fn ins_del() {
    if !stop_arrow() {
        return;
    }
    if gchar_cursor() == NUL as i32 {
        let temp = curwin_mut().w_cursor.col;
        if !can_bs(BS_EOL) || !do_join(2, false, true, false, false) {
            vim_beep(K_OPT_BO_FLAG_BACKSPACE);
        } else {
            mc_ins_join();
            curwin_mut().w_cursor.col = temp;
            if (g_st().state_global & VREPLACE_FLAG) != 0
                && g_st().orig_line_count > curbuf_mut().b_ml.ml_line_count
            {
                g_st().orig_line_count = curbuf_mut().b_ml.ml_line_count;
            }
        }
    } else if !del_char(false) {
        vim_beep(K_OPT_BO_FLAG_BACKSPACE);
    }
    g_st().ins_global.did_ai = false;
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;
    redo_append_char(K_DEL);
}

fn ins_bs(c: i32, mut mode: i32, inserted_space_p: &mut i32) -> bool {
    let mut cc: i32;
    let mut temp: i32 = 0;
    let mut did_backspace = false;
    let mut call_fix_indent = false;

    if buf_is_empty(curbuf_mut())
        || (!g_st().ins_global.revins_on
            && ((curwin_mut().w_cursor.lnum == 1 && curwin_mut().w_cursor.col == 0)
                || (!can_bs(BS_START)
                    && ((g_st().ins_global.moved != K_INS_NONE
                        && !bt_prompt(Some(crate::globals::curbuf_mut())))
                        || (curwin_mut().w_cursor.lnum == g_st().ins_global.start_orig.lnum
                            && curwin_mut().w_cursor.col <= g_st().ins_global.start_orig.col)))
                || (!can_bs(BS_INDENT)
                    && g_st().ins_global.moved == K_INS_NONE
                    && g_st().ins_global.ai_col > 0
                    && curwin_mut().w_cursor.col <= g_st().ins_global.ai_col)
                || (!can_bs(BS_EOL) && curwin_mut().w_cursor.col == 0)))
    {
        vim_beep(K_OPT_BO_FLAG_BACKSPACE);
        return false;
    }

    if !stop_arrow() {
        return false;
    }
    let in_indent = inindent(0);
    if in_indent {
        g_st().ins_global.can_cindent = false;
    }
    g_st().ins_global.end_comment_pending = NUL as i32;
    if g_st().ins_global.revins_on {
        inc_cursor();
    }
    if curwin_mut().w_cursor.coladd > 0 {
        if mode == BACKSPACE_CHAR {
            curwin_mut().w_cursor.coladd -= 1;
            return true;
        }
        if mode == BACKSPACE_WORD {
            curwin_mut().w_cursor.coladd = 0;
            return true;
        }
        curwin_mut().w_cursor.coladd = 0;
    }

    if curwin_mut().w_cursor.col == 0 {
        mc_ins_join();
        let lnum = g_st().ins_global.start.lnum;
        if curwin_mut().w_cursor.lnum == lnum || g_st().ins_global.revins_on {
            if !u_save(
                curwin_mut().w_cursor.lnum - 2,
                curwin_mut().w_cursor.lnum + 1,
            ) {
                return false;
            }
            g_st().ins_global.start.lnum -= 1;
            g_st().ins_global.start.col = ml_get_len(g_st().ins_global.start.lnum);
        }
        cc = -1;
        if (g_st().state_global & REPLACE_FLAG) != 0 {
            cc = replace_pop_if_nul();
        }
        if (g_st().state_global & REPLACE_FLAG) != 0 && curwin_mut().w_cursor.lnum <= lnum {
            dec_cursor();
        } else {
            if (g_st().state_global & VREPLACE_FLAG) == 0
                || curwin_mut().w_cursor.lnum > g_st().orig_line_count
            {
                temp = gchar_cursor();
                curwin_mut().w_cursor.lnum -= 1;

                if has_format_option(K_FO_AUTO) && has_format_option(K_FO_WHITE_PAR) {
                    let line_slice = crate::cursor::get_cursor_line_slice();
                    if line_slice.last() == Some(&b' ') {
                        let new_line = line_slice[..line_slice.len() - 1].to_vec();
                        ml_replace_dirty_line(curbuf_mut(), &new_line, false);
                    }
                }

                do_join(2, false, false, false, false);
                if temp == NUL as i32 && gchar_cursor() != NUL as i32 {
                    inc_cursor();
                }
            } else {
                dec_cursor();
            }

            if (g_st().state_global & REPLACE_FLAG) != 0 {
                let old_state = g_st().state_global;
                g_st().state_global = MODE_NORMAL;
                while cc > 0 {
                    let save_col = curwin_mut().w_cursor.col;
                    mb_replace_pop_ins();
                    curwin_mut().w_cursor.col = save_col;
                    cc = replace_pop_if_nul();
                }
                replace_pop_ins();
                g_st().state_global = old_state;
            }
        }
        g_st().ins_global.did_ai = false;
    } else {
        if g_st().ins_global.revins_on {
            dec_cursor();
        }
        let mut mincol: Colnr = 0;
        if mode == BACKSPACE_LINE
            && (curbuf_mut().b_p_ai || cindent_on())
            && !g_st().ins_global.revins_on
        {
            let save_col = curwin_mut().w_cursor.col;
            beginline(BL_WHITE);
            if curwin_mut().w_cursor.col < save_col {
                mincol = curwin_mut().w_cursor.col;
                call_fix_indent = true;
            }
            curwin_mut().w_cursor.col = save_col;
        }

        if mode == BACKSPACE_CHAR
            && ((opt_vars().p_sta && in_indent)
                || ((get_sts_value() != 0 || tabstop_count(&curbuf_mut().b_p_vsts_array) != 0)
                    && curwin_mut().w_cursor.col > 0
                    && {
                        let prev_b = crate::cursor::get_cursor_line_slice()
                            .get((curwin_mut().w_cursor.col - 1) as usize)
                            .copied()
                            .unwrap_or(0);
                        prev_b == TAB as u8
                            || (prev_b == b' '
                                && (*inserted_space_p == 0
                                    || g_st().ins_global.moved != K_INS_NONE))
                    }))
        {
            *inserted_space_p = 0;

            let use_ts = !curwin_mut().w_onebuf_opt.wo_list || curwin_mut().w_p_lcs_chars.tab1 != 0;
            let line_cstr = get_cursor_line_ptr();
            let cursor_col = curwin_mut().w_cursor.col as usize;

            let mut vcol: Colnr = 0;
            let mut space_vcol: Colnr = 0;
            let mut sci = utf_ptr2_str_char_info(line_cstr);
            let mut space_sci = sci;
            let mut prev_space = false;

            while sci.offset_from(line_cstr) < cursor_col {
                let cur_space = ascii_iswhite(sci.chr.value);
                if !prev_space && cur_space {
                    space_sci = sci;
                    space_vcol = vcol;
                }
                let cli = utf_cluster_info(sci);
                vcol += charsize_nowrap(curbuf_mut(), use_ts, vcol, sci.chr.value, cli.cells);
                sci = cli.next;
                prev_space = cur_space;
            }

            let mut want_vcol = if vcol > 0 { vcol - 1 } else { 0 };
            if opt_vars().p_sta && in_indent {
                want_vcol -= want_vcol % (get_sw_value(curbuf_mut()) as Colnr);
            } else {
                want_vcol = tabstop_start(want_vcol, get_sts_value(), &curbuf_mut().b_p_vsts_array);
            }

            loop {
                let cli = utf_cluster_info(space_sci);
                let size = charsize_nowrap(
                    curbuf_mut(),
                    use_ts,
                    space_vcol,
                    space_sci.chr.value,
                    cli.cells,
                );
                if space_vcol + size > want_vcol {
                    break;
                }
                space_vcol += size;
                space_sci = cli.next;
            }
            let want_col = space_sci.offset_from(line_cstr) as Colnr;

            while curwin_mut().w_cursor.col > want_col {
                dec_cursor();
                if (g_st().state_global & REPLACE_FLAG) != 0 {
                    if curwin_mut().w_cursor.lnum != g_st().ins_global.start.lnum
                        || curwin_mut().w_cursor.col >= g_st().ins_global.start.col
                    {
                        replace_do_bs(-1);
                    }
                } else {
                    del_char(false);
                }
            }

            while space_vcol < want_vcol {
                if curwin_mut().w_cursor.lnum == g_st().ins_global.start_orig.lnum
                    && curwin_mut().w_cursor.col < g_st().ins_global.start_orig.col
                {
                    g_st().ins_global.start_orig.col = curwin_mut().w_cursor.col;
                }

                if (g_st().state_global & VREPLACE_FLAG) != 0 {
                    ins_char(b' ' as i32);
                } else {
                    ins_str(b" ", 1);
                    if (g_st().state_global & REPLACE_FLAG) != 0 {
                        replace_push_nul();
                    }
                }
                space_vcol += 1;
            }
        } else {
            let mut cclass = mb_get_class(crate::cursor::get_cursor_pos_slice());
            loop {
                if !g_st().ins_global.revins_on {
                    dec_cursor();
                }
                cc = gchar_cursor();
                let prev_cclass = cclass;
                cclass = mb_get_class(crate::cursor::get_cursor_pos_slice());
                if mode == BACKSPACE_WORD && !ascii_isspace(cc) {
                    mode = BACKSPACE_WORD_NOT_SPACE;
                    temp = i32::from(vim_iswordc(cc));
                } else if mode == BACKSPACE_WORD_NOT_SPACE
                    && (ascii_isspace(cc)
                        || i32::from(vim_iswordc(cc)) != temp
                        || prev_cclass != cclass)
                {
                    if !g_st().ins_global.revins_on {
                        inc_cursor();
                    } else if (g_st().state_global & REPLACE_FLAG) != 0 {
                        dec_cursor();
                    }
                    break;
                }
                if (g_st().state_global & REPLACE_FLAG) != 0 {
                    replace_do_bs(-1);
                } else {
                    let mut has_composing = false;
                    if opt_vars().p_deco {
                        let p0_slice = crate::cursor::get_cursor_pos_slice();
                        let len0 = (utf_ptr2len(p0_slice) as usize).min(p0_slice.len());
                        has_composing = utf_composinglike(p0_slice, &p0_slice[len0..], None);
                    }
                    del_char(false);
                    if has_composing {
                        inc_cursor();
                    }
                    if g_st().ins_global.revins_chars != 0 {
                        g_st().ins_global.revins_chars -= 1;
                        g_st().ins_global.revins_legal += 1;
                    }
                    if g_st().ins_global.revins_on && gchar_cursor() == NUL as i32 {
                        break;
                    }
                }
                if mode == BACKSPACE_CHAR {
                    break;
                }
                if !(g_st().ins_global.revins_on
                    || (curwin_mut().w_cursor.col > mincol
                        && (can_bs(BS_NOSTOP)
                            || curwin_mut().w_cursor.lnum != g_st().ins_global.start_orig.lnum
                            || curwin_mut().w_cursor.col != g_st().ins_global.start_orig.col)))
                {
                    break;
                }
            }
        }
        did_backspace = true;
    }
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;
    if curwin_mut().w_cursor.col <= 1 {
        g_st().ins_global.did_ai = false;
    }

    if call_fix_indent {
        fix_indent();
    }

    redo_append_char(c);

    if curwin_mut().w_cursor.lnum == g_st().ins_global.start_orig.lnum
        && curwin_mut().w_cursor.col < g_st().ins_global.start_orig.col
    {
        g_st().ins_global.start_orig.col = curwin_mut().w_cursor.col;
    }

    if crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_BACKSPACE)
        && g_st().dollar_vcol == -1
    {
        g_st().dollar_vcol = curwin_mut().w_virtcol;
    }

    if did_backspace {
        fold_open_cursor();
    }
    did_backspace
}

fn ins_left() {
    let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
        fold_open_cursor();
    }
    undisplay_dollar();
    let tpos = curwin_mut().w_cursor;
    if oneleft() {
        start_arrow_with_change(tpos, end_change, K_LEFT);
        if g_st().ins_global.revins_scol != -1
            && curwin_mut().w_cursor.col >= g_st().ins_global.revins_scol
        {
            g_st().ins_global.revins_legal += 1;
        }
        g_st().ins_global.revins_chars += 1;
    } else if crate::strings::vim_haschr(opt_vars().p_ww.as_cstr(), b'[' as i32)
        && curwin_mut().w_cursor.lnum > 1
    {
        start_arrow(Some(tpos), true, K_LEFT);
        curwin_mut().w_cursor.lnum -= 1;
        coladvance(curwin_mut(), MAXCOL);
        curwin_mut().w_set_curswant = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
    g_st().ins_global.dont_sync_undo = K_FALSE;
}

fn ins_home(c: i32) {
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
        fold_open_cursor();
    }
    undisplay_dollar();
    let tpos = curwin_mut().w_cursor;
    if c == K_C_HOME {
        curwin_mut().w_cursor.lnum = 1;
    }
    curwin_mut().w_cursor.col = 0;
    curwin_mut().w_cursor.coladd = 0;
    curwin_mut().w_curswant = 0;
    start_arrow(
        Some(tpos),
        true,
        if c == K_C_HOME { NUL as i32 } else { K_HOME },
    );
}

fn ins_end(c: i32) {
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
        fold_open_cursor();
    }
    undisplay_dollar();
    let tpos = curwin_mut().w_cursor;
    if c == K_C_END {
        curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
    }
    coladvance(curwin_mut(), MAXCOL);
    curwin_mut().w_curswant = MAXCOL;
    start_arrow(
        Some(tpos),
        true,
        if c == K_C_END { NUL as i32 } else { K_END },
    );
}

fn ins_s_left() {
    let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
        fold_open_cursor();
    }
    undisplay_dollar();
    if curwin_mut().w_cursor.lnum > 1 || curwin_mut().w_cursor.col > 0 {
        start_arrow_with_change(curwin_mut().w_cursor, end_change, K_S_LEFT);
        bck_word(1, false, false);
        curwin_mut().w_set_curswant = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
    g_st().ins_global.dont_sync_undo = K_FALSE;
}

fn ins_right() {
    let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
        fold_open_cursor();
    }
    undisplay_dollar();
    if gchar_cursor() != NUL as i32 || virtual_active(curwin_mut()) {
        start_arrow_with_change(curwin_mut().w_cursor, end_change, K_RIGHT);
        curwin_mut().w_set_curswant = true;
        if virtual_active(curwin_mut()) {
            oneright();
        } else {
            curwin_mut().w_cursor.col +=
                crate::mbyte::utfc_slice2len(crate::cursor::get_cursor_pos_slice()) as Colnr;
        }

        g_st().ins_global.revins_legal += 1;
        if g_st().ins_global.revins_chars != 0 {
            g_st().ins_global.revins_chars -= 1;
        }
    } else if crate::strings::vim_haschr(opt_vars().p_ww.as_cstr(), b']' as i32)
        && curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count
    {
        start_arrow(Some(curwin_mut().w_cursor), true, K_RIGHT);
        curwin_mut().w_set_curswant = true;
        curwin_mut().w_cursor.lnum += 1;
        curwin_mut().w_cursor.col = 0;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
    g_st().ins_global.dont_sync_undo = K_FALSE;
}

fn ins_s_right() {
    let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
        fold_open_cursor();
    }
    undisplay_dollar();
    if curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count || gchar_cursor() != NUL as i32
    {
        start_arrow_with_change(curwin_mut().w_cursor, end_change, K_S_RIGHT);
        fwd_word(1, false, false);
        curwin_mut().w_set_curswant = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
    g_st().ins_global.dont_sync_undo = K_FALSE;
}

fn ins_up(startcol: bool) {
    let old_topline = curwin_mut().w_topline;
    let old_topfill = curwin_mut().w_topfill;

    undisplay_dollar();
    let tpos = curwin_mut().w_cursor;
    if cursor_up(1, true) {
        if startcol {
            coladvance(curwin_mut(), getvcol_nolist(&mut g_st().ins_global.start));
        }
        if old_topline != curwin_mut().w_topline || old_topfill != curwin_mut().w_topfill {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
        }
        start_arrow(Some(tpos), true, if startcol { b'k' as i32 } else { K_UP });
        g_st().ins_global.can_cindent = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
}

fn ins_pageup() {
    undisplay_dollar();

    if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
        if !crate::globals::first_tabpage_mut().tp_next.is_null() {
            start_arrow(Some(curwin_mut().w_cursor), true, NUL as i32);
            goto_tabpage(-1);
        }
        return;
    }

    let tpos = curwin_mut().w_cursor;
    if pagescroll(BACKWARD, 1, false) {
        start_arrow(Some(tpos), true, NUL as i32);
        g_st().ins_global.can_cindent = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
}

fn ins_down(startcol: bool) {
    let old_topline = curwin_mut().w_topline;
    let old_topfill = curwin_mut().w_topfill;

    undisplay_dollar();
    let tpos = curwin_mut().w_cursor;
    if cursor_down(1, true) {
        if startcol {
            coladvance(curwin_mut(), getvcol_nolist(&mut g_st().ins_global.start));
        }
        if old_topline != curwin_mut().w_topline || old_topfill != curwin_mut().w_topfill {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
        }
        start_arrow(
            Some(tpos),
            true,
            if startcol { b'j' as i32 } else { K_DOWN },
        );
        g_st().ins_global.can_cindent = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
}

fn ins_pagedown() {
    undisplay_dollar();

    if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
        if !crate::globals::first_tabpage_mut().tp_next.is_null() {
            start_arrow(Some(curwin_mut().w_cursor), true, NUL as i32);
            goto_tabpage(0);
        }
        return;
    }

    let tpos = curwin_mut().w_cursor;
    if pagescroll(FORWARD, 1, false) {
        start_arrow(Some(tpos), true, NUL as i32);
        g_st().ins_global.can_cindent = true;
    } else {
        vim_beep(K_OPT_BO_FLAG_CURSOR);
    }
}

fn ins_tab() -> bool {
    if g_st().ins_global.start_blank_vcol == MAXCOL
        && curwin_mut().w_cursor.lnum == g_st().ins_global.start.lnum
    {
        g_st().ins_global.start_blank_vcol = get_nolist_virtcol();
    }
    if echeck_abbr(TAB + ABBR_OFF) {
        return false;
    }

    let ind = inindent(0);
    if ind {
        g_st().ins_global.can_cindent = false;
    }

    if !curbuf_mut().b_p_et
        && !(opt_vars().p_sta
            && ind
            && ((tabstop_count(&curbuf_mut().b_p_vts_array) > 1)
                || (tabstop_count(&curbuf_mut().b_p_vts_array) == 1
                    && tabstop_first(&curbuf_mut().b_p_vts_array) != get_sw_value(curbuf_mut()))
                || (tabstop_count(&curbuf_mut().b_p_vts_array) == 0
                    && curbuf_mut().b_p_ts != i64::from(get_sw_value(curbuf_mut())))))
        && tabstop_count(&curbuf_mut().b_p_vsts_array) == 0
        && get_sts_value() == 0
    {
        return true;
    }

    if !stop_arrow() {
        return true;
    }

    g_st().ins_global.did_ai = false;
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;
    redo_append_str(b"\t");

    let mut temp: i32 = if opt_vars().p_sta && ind {
        let sw = get_sw_value(curbuf_mut());
        sw - (get_nolist_virtcol() % sw)
    } else if tabstop_count(&curbuf_mut().b_p_vsts_array) > 0 || curbuf_mut().b_p_sts != 0 {
        tabstop_padding(
            get_nolist_virtcol(),
            i64::from(get_sts_value()),
            &curbuf_mut().b_p_vsts_array,
        )
    } else {
        tabstop_padding(
            get_nolist_virtcol(),
            curbuf_mut().b_p_ts,
            &curbuf_mut().b_p_vts_array,
        )
    };

    ins_char(b' ' as i32);
    while {
        temp -= 1;
        temp > 0
    } {
        if (g_st().state_global & VREPLACE_FLAG) != 0 {
            ins_char(b' ' as i32);
        } else {
            ins_str(b" ", 1);
            if (g_st().state_global & REPLACE_FLAG) != 0 {
                replace_push_nul();
            }
        }
    }

    if !curbuf_mut().b_p_et
        && (tabstop_count(&curbuf_mut().b_p_vsts_array) > 0
            || get_sts_value() > 0
            || (opt_vars().p_sta && ind))
    {
        let mut cursor_pos = curwin_mut().w_cursor;
        let mut saved_line_vec: Vec<u8> = Vec::new();
        let mut ptr;
        let mut vcol: Colnr = 0;
        let mut want_vcol: Colnr = 0;
        let mut change_col: i32 = -1;
        let save_list = curwin_mut().w_onebuf_opt.wo_list;

        if (g_st().state_global & VREPLACE_FLAG) != 0 {
            saved_line_vec = get_cursor_line_slice().to_vec();
            saved_line_vec.push(0);
            ptr = saved_line_vec
                .as_mut_ptr()
                .wrapping_offset(cursor_pos.col as isize);
        } else {
            ptr = get_cursor_pos_ptr().as_ptr().cast_mut().cast();
        }

        if !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_LISTWM) {
            curwin_mut().w_onebuf_opt.wo_list = false;
        }

        let mut fpos = curwin_mut().w_cursor;
        while fpos.col > 0
            && ascii_iswhite(*crate::globals::GraphPtr::of(ptr.wrapping_offset(-1)).borrow())
        {
            fpos.col -= 1;
            ptr = ptr.wrapping_offset(-1);
        }

        if (g_st().state_global & REPLACE_FLAG) != 0
            && fpos.lnum == g_st().ins_global.start.lnum
            && fpos.col < g_st().ins_global.start.col
        {
            ptr = ptr.wrapping_offset((g_st().ins_global.start.col - fpos.col) as isize);
            fpos.col = g_st().ins_global.start.col;
        }

        getvcol(curwin_mut(), &mut fpos, Some(&mut vcol), None, None, 0);
        getvcol(
            curwin_mut(),
            &mut cursor_pos,
            Some(&mut want_vcol),
            None,
            None,
            0,
        );
        if (g_st().state_global & VREPLACE_FLAG) == 0 {
            curwin_mut().w_cursor = cursor_pos;
        }

        let tab = c"\t";
        let tab_v = b'\t' as i32;
        let mut csarg = CharsizeArg::new();
        let mut cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, tab);

        while ascii_iswhite(*crate::globals::GraphPtr::of(ptr).borrow()) {
            let i = win_charsize(cstype, vcol, tab.to_bytes_with_nul(), tab_v, &mut csarg, 1).width;
            if vcol + i > want_vcol {
                break;
            }
            if *crate::globals::GraphPtr::of(ptr).borrow() != b'\t' {
                *crate::globals::GraphPtr::of(ptr).borrow_mut() = b'\t';

                if change_col < 0 {
                    change_col = fpos.col;
                    if fpos.lnum == g_st().ins_global.start.lnum
                        && fpos.col < g_st().ins_global.start.col
                    {
                        g_st().ins_global.start.col = fpos.col;
                    }
                }
            }
            fpos.col += 1;
            ptr = ptr.wrapping_add(1);
            vcol += i;
        }

        if change_col >= 0 {
            let mut repl_off: i32 = 0;
            let ptr_cstr = if (g_st().state_global & VREPLACE_FLAG) != 0 {
                crate::memory::bytes_as_cstr(&saved_line_vec[fpos.col as usize..])
            } else {
                crate::memory::CStrSubslice::advance(get_cursor_line_ptr(), fpos.col as usize)
            };
            cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, ptr_cstr);
            while vcol < want_vcol && *crate::globals::GraphPtr::of(ptr).borrow() == b' ' {
                vcol += win_charsize(cstype, vcol, b" ", b' ' as i32, &mut csarg, 1).width;
                ptr = ptr.wrapping_add(1);
                repl_off += 1;
            }

            if vcol > want_vcol {
                ptr = ptr.wrapping_offset(-1);
                repl_off -= 1;
            }
            fpos.col += repl_off;

            let i = cursor_pos.col - fpos.col;
            if i > 0 {
                if (g_st().state_global & VREPLACE_FLAG) == 0 {
                    let newp_len = (curbuf_mut().b_ml.ml_line_textlen - i) as usize;
                    let line_slice = curbuf_mut().b_ml.current_line_slice();
                    let col = ((ptr as isize) - (line_slice.as_ptr() as isize)) as usize;
                    let tail_len = newp_len.saturating_sub(col + 1);
                    let mut new_line = Vec::with_capacity(newp_len.saturating_sub(1));
                    new_line.extend_from_slice(&line_slice[..col]);
                    new_line.extend_from_slice(
                        &line_slice[col + i as usize..col + i as usize + tail_len],
                    );
                    ml_replace_dirty_line(curbuf_mut(), &new_line, true);
                    inserted_bytes(
                        fpos.lnum,
                        change_col,
                        cursor_pos.col - change_col,
                        fpos.col - change_col,
                    );
                } else {
                    let col = fpos.col as usize;
                    saved_line_vec.copy_within(col + i as usize.., col);
                }
                if (g_st().state_global & REPLACE_FLAG) != 0
                    && (g_st().state_global & VREPLACE_FLAG) == 0
                {
                    for _ in 0..i {
                        replace_join(repl_off);
                    }
                }
            }
            cursor_pos.col -= i;
            if (g_st().state_global & VREPLACE_FLAG) == 0 {
                curwin_mut().w_cursor.col = cursor_pos.col;
            }

            if (g_st().state_global & VREPLACE_FLAG) != 0 {
                backspace_until_column(change_col);
                ins_bytes_len(
                    &saved_line_vec[change_col as usize..],
                    (cursor_pos.col - change_col) as usize,
                );
            }
        }

        curwin_mut().w_onebuf_opt.wo_list = save_list;
    }

    false
}

/// Handle CR or NL in insert mode.
pub fn ins_eol(c: i32) -> bool {
    if echeck_abbr(c + ABBR_OFF) {
        return true;
    }
    if !stop_arrow() {
        return false;
    }
    undisplay_dollar();

    if (g_st().state_global & REPLACE_FLAG) != 0 && (g_st().state_global & VREPLACE_FLAG) == 0 {
        replace_push_nul();
    }

    if virtual_active(curwin_mut()) && curwin_mut().w_cursor.coladd > 0 {
        coladvance(curwin_mut(), getviscol());
    }

    if g_st().ins_global.revins_on {
        curwin_mut().w_cursor.col += get_cursor_pos_len();
    }

    redo_append_str(b"\n");
    let i = open_line(
        FORWARD,
        if has_format_option(K_FO_RET_COMS) {
            OPENLINE_DO_COM
        } else {
            0
        },
        g_st().old_indent,
        None,
    );
    g_st().old_indent = 0;
    g_st().ins_global.can_cindent = true;
    fold_open_cursor();
    i
}

fn ins_digraph() -> i32 {
    let mut did_putchar = false;

    ins_st(|st| st.pc_status.set(PC_STATUS_UNSET));
    if redrawing() && !char_avail() {
        ins_redraw(false);
        edit_putchar(b'?' as i32, true);
        did_putchar = true;
        add_to_showcmd_c(CTRL_K_CONST);
    }

    g_st().no_mapping += 1;
    g_st().allow_keys += 1;
    let mut c = plain_vgetc();
    g_st().no_mapping -= 1;
    g_st().allow_keys -= 1;
    if did_putchar {
        edit_unputchar();
    }

    if is_special_fn(c) || g_st().mod_mask != 0 {
        clear_showcmd();
        insert_special(c, 1, 0);
        return NUL as i32;
    }
    if c != ESC {
        did_putchar = false;
        if redrawing() && !char_avail() {
            ins_redraw(false);
            if char2cells(c) == 1 {
                ins_redraw(false);
                edit_putchar(c, true);
                did_putchar = true;
            }
            add_to_showcmd_c(c);
        }
        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        let cc = plain_vgetc();
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
        if did_putchar {
            edit_unputchar();
        }
        if cc != ESC {
            redo_append_str(b"\x16");
            c = digraph_get(c, cc, true);
            clear_showcmd();
            return c;
        }
    }
    clear_showcmd();
    NUL as i32
}

/// Handle CTRL-E and CTRL-Y in Insert mode: copy char from other line.
pub fn ins_copychar(lnum: Linenr) -> i32 {
    if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count {
        vim_beep(K_OPT_BO_FLAG_COPY);
        return NUL as i32;
    }

    validate_virtcol(curwin_mut());
    let end_vcol = curwin_mut().w_virtcol;
    let line_cstr = ml_get(lnum);

    let mut csarg = CharsizeArg::new();
    let cstype = init_charsize_arg(&mut csarg, curwin_mut(), lnum, line_cstr);
    let mut ci = utf_ptr2_str_char_info(line_cstr);
    let mut vcol: i32 = 0;
    while vcol < end_vcol && !ci.ptr.is_empty() {
        let cli = utf_cluster_info(ci);
        vcol += win_charsize(cstype, vcol, ci.cstr(), ci.chr.value, &mut csarg, cli.cells).width;
        if vcol > end_vcol {
            break;
        }
        ci = cli.next;
    }

    let c = if ci.chr.value < 0 {
        i32::from(ci.first_byte())
    } else {
        ci.chr.value
    };
    if c == NUL as i32 {
        vim_beep(K_OPT_BO_FLAG_COPY);
    }
    c
}

fn ins_ctrl_ey(tc: i32) -> i32 {
    let mut c = tc;
    if ctrl_x_mode_scroll() {
        if c == CTRL_Y {
            scrolldown_clamp();
        } else {
            scrollup_clamp();
        }
        redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
    } else {
        c = ins_copychar(curwin_mut().w_cursor.lnum + if c == CTRL_Y { -1 } else { 1 });
        if c != NUL as i32 {
            if c < 256 && !(c as u8).is_ascii_alphanumeric() {
                redo_append_str(b"\x16");
            }
            let tw_save = curbuf_mut().b_p_tw;
            curbuf_mut().b_p_tw = -1;
            insert_special(c, 1, 0);
            curbuf_mut().b_p_tw = tw_save;
            g_st().ins_global.revins_chars += 1;
            g_st().ins_global.revins_legal += 1;
            c = CTRL_V_CONST;
            auto_format(false, true);
        }
    }
    c
}

/// Get the value that `w_virtcol` would have when `'list'` is off.
pub fn get_nolist_virtcol() -> Colnr {
    if curwin_mut().w_buffer.is_null()
        || curwin_mut().buf_mut().b_ml.ml_mfp.is_none()
        || curwin_mut().w_cursor.lnum > curwin_mut().buf_mut().b_ml.ml_line_count
    {
        return 0;
    }
    if curwin_mut().w_onebuf_opt.wo_list
        && !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_LISTWM)
    {
        return getvcol_nolist(&mut curwin_mut().w_cursor);
    }
    validate_virtcol(curwin_mut());
    curwin_mut().w_virtcol
}

fn do_insert_char_pre(c: i32) -> Option<std::ffi::CString> {
    if c == CTRL_RSB_CONST || g_st().key_stuffed != 0 || !has_event(EVENT_INSERTCHARPRE) {
        return None;
    }

    let mut buf = [0u8; MB_MAXBYTES + 1];
    let save_state = g_st().state_global;
    let buflen = utf_char2bytes(c, &mut buf) as usize;
    buf[buflen] = 0;

    g_st().textlock += 1;
    set_vim_var_string(
        VV_CHAR,
        Some(crate::memory::bytes_as_cstr(&buf)),
        buflen as isize,
    );

    let mut res: Option<std::ffi::CString> = None;
    if ins_apply_autocmds(EVENT_INSERTCHARPRE) != 0 {
        let vchar = get_vim_var_str(VV_CHAR);
        if &buf[..buflen] != vchar.to_bytes() {
            res = Some((*vchar).to_owned());
        }
    }

    set_vim_var_string(VV_CHAR, None, -1);
    g_st().textlock -= 1;
    g_st().state_global = save_state;
    res
}

pub fn get_can_cindent() -> bool {
    g_st().ins_global.can_cindent
}

pub fn set_can_cindent(val: bool) {
    g_st().ins_global.can_cindent = val;
}

/// Trigger `event` and take care of fixing undo.
pub fn ins_apply_autocmds(event: AuEvent) -> i32 {
    let tick = buf_get_changedtick(curbuf_mut());
    let r = i32::from(apply_autocmds(event, None, None, false, curbuf_opt_mut()));
    if event != EVENT_INSERTLEAVE && tick != buf_get_changedtick(curbuf_mut()) {
        u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_spaces_and_can_cindent_work() {
        let mut line = *b"hello   \0";
        truncate_spaces(&mut line, 8);
        assert_eq!(crate::memory::bytes_until_nul(&line), b"hello");

        set_can_cindent(true);
        assert!(get_can_cindent());
        set_can_cindent(false);
        assert!(!get_can_cindent());
    }
}
