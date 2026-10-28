//! Functions for Insert mode (`src/nvim/insert.c`).

use crate::api::private::validate::NvimString;
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
    hex2nr, skipwhite, vim_isprintc, vim_iswordc,
};
use crate::cmdexpand::{CTRL_N, CTRL_P, MB_MAXBYTES};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    Colnr, CursorPos, K_OPT_VE_FLAG_ALL, Linenr, VALID_VIRTCOL, char_before_cursor, check_cursor,
    check_cursor_col, check_visual_pos, coladvance, dec_cursor, equalpos, gchar_cursor,
    get_cursor_line_len, get_cursor_line_ptr, get_cursor_pos_len, get_cursor_pos_ptr, getviscol,
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
    GraphemeState, mb_adjust_cursor, mb_get_class, ptr2cells, utf_char2bytes, utf_char2len,
    utf_cluster_info, utf_composinglike, utf_head_off, utf_ptr2_str_char_info, utf_ptr2char,
    utf_ptr2len, utfc_ptr2len,
};
use crate::mcursor::{mc_ins_cascade, mc_ins_cascade_restart, mc_ins_join, mc_ins_replaying};
use crate::memline::{
    gchar_pos, ml_append, ml_get, ml_get_buf, ml_get_len, ml_replace, ml_replace_dirty_line,
};
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
    PUT_CURSEND, PUT_FIXINDENT, YREG_PASTE, Yankreg, do_put, get_expr_register, get_yank_register,
    insert_reg, is_literal_register, valid_yank_reg,
};
use crate::search::{BACKWARD, FORWARD};
use crate::state::{
    EditorState, MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL, MODE_REPLACE, MODE_VREPLACE,
    REPLACE_FLAG, VREPLACE_FLAG, may_trigger_modechanged, may_trigger_safestate, state_enter,
    state_handle_k_event, virtual_active,
};
use crate::strings::{gettext, vim_snprintf, vim_strchr};
use crate::syntax::syntax_present;
use crate::terminal::terminal_enter;
use crate::textformat::{
    auto_format, check_auto_format, comp_textwidth, fex_format, has_format_option, internal_format,
};
use crate::textobject::{bck_word, fwd_word};
use crate::ui::{ui_cursor_shape, ui_flush, ui_has, vim_beep};
use crate::undo::{u_clearallandblockfree, u_save, u_save_cursor, u_sync};
use crate::window::{WinInfoInfo, goto_tabpage, may_trigger_win_scrolled_resized};
use std::ffi::{c_char, c_int, c_long, c_uint};
use std::ptr;

use crate::change::COM_MIDDLE;
const INPUT_BUFLEN: usize = 100;

pub(crate) const MOD_MASK_SHIFT: c_int = 0x02;
pub(crate) const MOD_MASK_CTRL: c_int = 0x04;
const MOD_MASK_CMD: c_int = 0x80;

pub(crate) const CTRL_J_CONST: c_int = 10;
pub(crate) const CTRL_S_CONST: c_int = 19;
pub(crate) const CTRL_X_CONST: c_int = 24;

pub(crate) const K_CHARSIZE_FAST: CSType = true;

#[inline]
const fn is_special_fn(c: c_int) -> bool {
    c < 0
}

const K_INS_ARROW: c_int = 1;
const K_INS_JUMP: c_int = 2;

pub(crate) const HLF_8: usize = 1;

pub(crate) const K_OPT_FDO_FLAG_ALL: c_uint = 0x01;
pub(crate) const K_OPT_FDO_FLAG_HOR: c_uint = 0x04;
const K_OPT_FDO_FLAG_INSERT: c_uint = 0x100;

const K_OPT_BO_FLAG_BACKSPACE: c_uint = 0x02;
const K_OPT_BO_FLAG_CURSOR: c_uint = 0x04;
const K_OPT_BO_FLAG_COPY: c_uint = 0x10;
const K_OPT_BO_FLAG_CTRLG: c_uint = 0x20;
const K_OPT_BO_FLAG_REGISTER: c_uint = 0x8000;

pub(crate) const K_FO_AUTO: c_int = b'a' as c_int;
pub(crate) const K_FO_INS_BLANK: c_int = b'b' as c_int;
const K_FO_INS_LONG: c_int = b'l' as c_int;
const K_FO_RET_COMS: c_int = b'r' as c_int;
pub(crate) const K_FO_WHITE_PAR: c_int = b'w' as c_int;

const K_CPO_BACKSPACE: c_int = b'v' as c_int;
const K_CPO_INDENT: c_int = b'I' as c_int;
const K_CPO_REPLCNT: c_int = b'X' as c_int;

const BS_INDENT: c_int = b'i' as c_int;
pub(crate) const BS_EOL: c_int = b'l' as c_int;
pub(crate) const BS_START: c_int = b's' as c_int;
pub(crate) const BS_NOSTOP: c_int = b'p' as c_int;

const INDENT_INC: c_int = 2;

pub(crate) const INSCHAR_FORMAT: c_int = 1;
const INSCHAR_CTRLV: c_int = 4;
pub(crate) const INSCHAR_NO_FEX: c_int = 8;

const BACKSPACE_CHAR: c_int = 1;
const BACKSPACE_WORD: c_int = 2;
const BACKSPACE_WORD_NOT_SPACE: c_int = 3;
const BACKSPACE_LINE: c_int = 4;

const PC_STATUS_UNSET: c_int = 0;
const PC_STATUS_RIGHT: c_int = 1;
const PC_STATUS_LEFT: c_int = 2;
const PC_STATUS_SET: c_int = 3;

#[repr(C)]
struct InsertState {
    mincol: c_int,
    cmdchar: c_int,
    cmdchar_todo: c_int,
    ins_just_started: bool,
    startln: c_int,
    count: c_int,
    c: c_int,
    lastc: c_int,
    i: c_int,
    did_backspace: bool,
    line_is_white: bool,
    old_topline: Linenr,
    old_topfill: c_int,
    inserted_space: c_int,
    replace_state: c_int,
    did_restart_edit: c_int,
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
    fn check(&mut self) -> c_int {
        insert_check(self)
    }

    fn execute(&mut self, key: c_int) -> c_int {
        insert_execute(self, key)
    }
}

impl Default for InsertState {
    fn default() -> Self {
        Self::new()
    }
}

struct InsertModuleState {
    compl_busy: bool,
    last_insert: Option<std::ffi::CString>,
    last_insert_skip: c_int,
    replace_stack: Vec<c_char>,
    pc_status: c_int,
    pc_schar: Schar,
    pc_attr: c_int,
    pc_row: c_int,
    pc_col: c_int,
    ins_esc_disabled_redraw: bool,
}

impl InsertModuleState {
    const fn new() -> Self {
        Self {
            compl_busy: false,
            last_insert: None,
            last_insert_skip: 0,
            replace_stack: Vec::new(),
            pc_status: PC_STATUS_UNSET,
            pc_schar: 0,
            pc_attr: 0,
            pc_row: 0,
            pc_col: 0,
            ins_esc_disabled_redraw: false,
        }
    }
}

static INS_MOD_STATE: GlobalState<InsertModuleState> = GlobalState::new(InsertModuleState::new());

#[inline]
fn ins_st() -> &'static mut InsertModuleState {
    unsafe { &mut *INS_MOD_STATE.as_ptr() }
}

#[inline]
fn replace_stack_mut() -> &'static mut Vec<c_char> {
    unsafe { &mut (*INS_MOD_STATE.as_ptr()).replace_stack }
}

#[inline]
const fn is_special_ins(c: c_int) -> bool {
    c < b' ' as c_int || c >= DEL || c == b'0' as c_int || c == b'^' as c_int
}

#[inline]
pub fn linetabsize_str(s: *mut c_char) -> c_int {
    linetabsize_col(0, s)
}

use crate::mbyte::utf_byte2len;

#[inline]
fn trigger_autocomplete(s: &mut InsertState) -> bool {
    {
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
}

#[inline]
fn may_trigger_autocomplete(s: &mut InsertState) {
    {
        if ins_compl_has_autocomplete() && !char_avail() && curwin_mut().w_cursor.col > 0 {
            s.c = char_before_cursor();
            if vim_isprintc(s.c) {
                let _ = trigger_autocomplete(&mut *s);
            }
        }
    }
}

fn insert_enter(s: &mut InsertState) {
    unsafe {
        s.did_backspace = true;
        s.old_topfill = -1;
        s.replace_state = MODE_REPLACE;
        s.cmdchar_todo = s.cmdchar;
        s.ins_just_started = true;
        g_st().ins_global.did_restart_edit = g_st().restart_edit;
        msg_check_for_delay(true);
        g_st().ins_global.update_start_orig = true;

        ins_compl_clear();

        if s.cmdchar != b'r' as c_int && s.cmdchar != b'v' as c_int {
            let save_cursor = curwin_mut().w_cursor;
            let ptr = if s.cmdchar == b'R' as c_int {
                c"r".as_ptr()
            } else if s.cmdchar == b'V' as c_int {
                c"v".as_ptr()
            } else {
                c"i".as_ptr()
            };
            set_vim_var_string(VV_INSERTMODE, ptr, 1);
            set_vim_var_string(VV_CHAR, ptr::null(), -1);
            ins_apply_autocmds(EVENT_INSERTENTER);

            if g_st().need_highlight_changed {
                highlight_changed();
            }

            if !equalpos(curwin_mut().w_cursor, save_cursor)
                && *get_vim_var_str(VV_CHAR) == NUL
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

        g_st().ins_global.start_textlen = linetabsize_str(get_cursor_line_ptr());
        g_st().ins_global.start_blank_vcol = MAXCOL;

        if !g_st().ins_global.did_ai {
            g_st().ins_global.ai_col = 0;
        }

        if s.cmdchar != NUL as c_int && g_st().restart_edit == 0 {
            if s.cmdchar == b'V' as c_int || s.cmdchar == b'v' as c_int {
                prep_redo(
                    false,
                    false,
                    CmdSpec {
                        count: s.count as c_long,
                        cmd: b'g' as c_int,
                        cmd2: if s.cmdchar == b'v' as c_int {
                            b'r' as c_int
                        } else {
                            b'R' as c_int
                        },
                        ..CmdSpec::default()
                    },
                );
            } else {
                prep_redo(
                    false,
                    false,
                    CmdSpec {
                        count: s.count as c_long,
                        cmd: s.cmdchar,
                        cmd2: if s.cmdchar == b'g' as c_int {
                            b'I' as c_int
                        } else {
                            NUL as c_int
                        },
                        ..CmdSpec::default()
                    },
                );
                if s.cmdchar == b'r' as c_int {
                    s.count = 1;
                }
            }
        }

        if s.cmdchar == b'R' as c_int {
            g_st().state_global = MODE_REPLACE;
        } else if s.cmdchar == b'V' as c_int || s.cmdchar == b'v' as c_int {
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
        g_st().ins_global.revins_on = g_st().state_global == MODE_INSERT && opt_vars().p_ri != 0;
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
                let ptr = get_cursor_line_ptr().offset(curwin_mut().w_cursor.col as isize);
                if !crate::memory::cstr_is_empty(ptr) {
                    if crate::memory::cstr_byte_at(ptr, 1) == 0 {
                        curwin_mut().w_cursor.col += 1;
                    } else {
                        s.i = utfc_ptr2len(ptr);
                        if *ptr.offset(s.i as isize) == NUL {
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
        if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
            s.i = showmode();
        }

        if g_st().ins_global.did_restart_edit == 0 {
            change_warning(curbuf_mut(), if s.i == 0 { 0 } else { s.i + 1 });
        }

        ui_cursor_shape();
        do_digraph(-1);

        let redo = redo_keys(ptr::null());
        g_st().ins_global.new_insert_skip =
            redo.as_ref().map_or(0, |s| s.as_bytes().len() as c_int);

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
        if s.cmdchar != b'r' as c_int && s.cmdchar != b'v' as c_int && s.c != CTRL_C_CONST {
            ins_apply_autocmds(EVENT_INSERTLEAVE);
        }
        g_st().did_cursorhold = false;

        if !char_avail() && curbuf_mut().b_last_changedtick_i == buf_get_changedtick(curbuf_mut()) {
            curbuf_mut().b_last_changedtick = buf_get_changedtick(curbuf_mut());
        }
    }
}

fn insert_check(s: &mut InsertState) -> c_int {
    {
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
            g_st().restart_edit = b'I' as c_int;
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
            s.cmdchar_todo = NUL as c_int;
        }

        if curbuf_mut().b_mod_set
            && curwin_mut().w_onebuf_opt.wo_wrap != 0
            && curwin_mut().w_onebuf_opt.wo_sms == 0
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
                    == curwin_mut().w_view_height - 1 - (get_scrolloff_value(curwin_mut()) as c_int)
                && (curwin_mut().w_cursor.lnum != curwin_mut().w_topline
                    || curwin_mut().w_topfill > 0)
            {
                if curwin_mut().w_topfill > 0 {
                    curwin_mut().w_topfill -= 1;
                } else if has_folding(
                    curwin_ptr(),
                    curwin_mut().w_topline,
                    None,
                    &raw mut s.old_topline,
                ) {
                    set_topline(curwin_mut(), s.old_topline + 1);
                } else {
                    set_topline(curwin_mut(), curwin_mut().w_topline + 1);
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

        if curwin_mut().w_onebuf_opt.wo_scb != 0 {
            do_check_scrollbind(true);
        }

        if curwin_mut().w_onebuf_opt.wo_crb != 0 {
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
}

fn insert_execute(s: &mut InsertState, key: c_int) -> c_int {
    unsafe {
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
                if s.c == NUL as c_int {
                    return 1;
                }
            }

            if !ins_compl_used_match() {
                if s.c == CTRL_L_CONST
                    && (!ctrl_x_mode_line_or_eval() || ins_compl_long_shown_match())
                {
                    ins_compl_addfrommatch();
                    return 1;
                }

                let is_commit = ins_compl_commit_char(s.c);
                if !is_commit && ins_compl_accept_char(s.c) {
                    if let Some(str_) = do_insert_char_pre(s.c) {
                        let mut p = str_.as_ptr().cast_mut();
                        while !crate::memory::cstr_is_empty(p) {
                            ins_compl_addleader(utf_ptr2char(p));
                            p = p.add(utfc_ptr2len(p) as usize);
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
            if in_cinkeys(s.c, b'!' as c_int, s.line_is_white) && stop_arrow() {
                do_c_expr_indent();
                return 1;
            }
            if g_st().ins_global.can_cindent
                && in_cinkeys(s.c, b'*' as c_int, s.line_is_white)
                && stop_arrow()
            {
                do_c_expr_indent();
            }
        }

        if curwin_mut().w_onebuf_opt.wo_rl != 0 {
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
}

fn insert_handle_key(s: &mut InsertState) -> c_int {
    unsafe {
        let mut goto_normalchar = false;
        let mut goto_do_complete = false;

        match s.c {
            ESC => {
                if !echeck_abbr(ESC + ABBR_OFF) {
                    return 0;
                }
            }
            CTRL_C_CONST => {
                if bt_cmdwin(g_st().curbuf) {
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
            c if c == b' ' as c_int && g_st().mod_mask != MOD_MASK_CTRL => {
                goto_normalchar = true;
            }
            c if c == b' ' as c_int || c == K_ZERO || c == NUL as c_int || c == CTRL_A => {
                if !stuff_inserted(NUL as c_int, 1, c_int::from(s.c == CTRL_A)) && s.c != CTRL_A {
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
                if opt_vars().p_ari == 0 {
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
                    g_st().restart_edit = b'A' as c_int;
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
                    br_buf: ptr::null_mut(),
                    br_fnum: 0,
                    br_buf_free_count: 0,
                };
                let tick = buf_get_changedtick(curbuf_mut());
                set_bufref(&mut save_curbuf, g_st().curbuf);

                if s.c == K_PASTE_START {
                    paste_repeat(1);
                } else if s.c == K_EVENT {
                    state_handle_k_event();
                    if g_st().ins_global.dont_sync_undo == K_TRUE {
                        g_st().ins_global.dont_sync_undo = K_NONE;
                    }
                } else if s.c == K_COMMAND {
                    do_cmdline(ptr::null_mut(), crate::cmdexpand::LineGetter::CmdKeyCmd, 0);
                } else {
                    map_execute_lua(false, false);
                }

                if g_st().pum_want.active {
                    if pum_visible() {
                        g_st().edit_submode_extra = ptr::null_mut();
                        insert_do_complete(&mut *s);
                        if g_st().pum_want.finish {
                            ins_compl_prep(CTRL_Y);
                        }
                    }
                    g_st().pum_want.active = false;
                }

                let changed = bufref_valid(Some(&save_curbuf))
                    && g_st().curbuf == save_curbuf.br_buf
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
                } else if s.c == CAR && bt_cmdwin(g_st().curbuf) {
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
                    if s.c != NUL as c_int {
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
            c if c == b's' as c_int || c == CTRL_S_CONST => {
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
            if crate::memory::cstr_is_empty(curbuf_mut().b_p_cpt)
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
            if opt_vars().p_paste == 0 {
                if let Some(str_) = do_insert_char_pre(s.c) {
                    let str_ptr = str_.as_ptr().cast_mut();
                    if !crate::memory::cstr_is_empty(str_ptr) && stop_arrow() {
                        let mut p = str_ptr;
                        while !crate::memory::cstr_is_empty(p) {
                            s.c = utf_ptr2char(p);
                            if s.c == CAR || s.c == K_KENTER || s.c == NL {
                                ins_eol(s.c);
                            } else {
                                ins_char(s.c);
                            }
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                        redo_append_lit(str_ptr, -1);
                    }
                    s.c = NUL as c_int;
                }
                if s.c == NUL as c_int {
                    skip_char = true;
                }
            }

            if !skip_char {
                ins_try_si(s.c);

                if s.c == b' ' as c_int {
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
}

fn insert_do_complete(s: &mut InsertState) {
    {
        ins_compl_clear_autocomplete_delay();
        ins_st().compl_busy = true;
        g_st().disable_fold_update += 1;
        if !ins_complete(s.c, true) {
            compl_status_clear();
        }
        g_st().disable_fold_update -= 1;
        ins_st().compl_busy = false;
        g_st().ins_global.can_si = may_do_si();
    }
}

fn insert_do_cindent(s: &mut InsertState) {
    if in_cinkeys(s.c, b' ' as c_int, s.line_is_white) && stop_arrow() {
        do_c_expr_indent();
    }
}

fn insert_handle_key_post(s: &mut InsertState) {
    {
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
}

/// Start inserting text.
pub fn edit(cmdchar: c_int, startln: bool, count: c_int) -> bool {
    {
        if !curbuf_mut().terminal.is_null() {
            if g_st().ex_normal_busy != 0 {
                g_st().restart_edit = b'i' as c_int;
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
            || ins_st().compl_busy
            || pum_visible()
            || expr_map_locked()
        {
            emsg(gettext(E_TEXTLOCK));
            return false;
        }

        let mut s = InsertState::new();
        s.cmdchar = cmdchar;
        s.startln = c_int::from(startln);
        s.count = count;
        insert_enter(&mut s);
        s.c == CTRL_O
    }
}

/// Redraw for Insert mode.
pub fn ins_redraw(ready: bool) {
    {
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
            g_st().last_cursormoved_win = curwin_ptr();
            g_st().last_cursormoved = curwin_mut().w_cursor;
        }

        if ready
            && has_event(EVENT_TEXTCHANGEDI)
            && curbuf_mut().b_last_changedtick_i != buf_get_changedtick(curbuf_mut())
            && !pum_visible()
        {
            let mut aco = CtxSwitch::new();
            let tick = buf_get_changedtick(curbuf_mut());
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), g_st().curbuf, 0);
            apply_autocmds(
                EVENT_TEXTCHANGEDI,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
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
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), g_st().curbuf, 0);
            apply_autocmds(
                EVENT_TEXTCHANGEDP,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
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
}

fn ins_ctrl_v() {
    let mut did_putchar = false;

    ins_redraw(false);

    if redrawing() && !char_avail() {
        edit_putchar(b'^' as c_int, true);
        did_putchar = true;
    }
    redo_append_str(c"\x16", 1);

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

pub fn edit_putchar(c: c_int, highlight: bool) {
    unsafe {
        if curwin_mut().w_grid_alloc.chars.is_empty()
            && !crate::grid::grid_allocated(&g_st().default_grid)
        {
            return;
        }

        update_topline(curwin_mut());
        validate_cursor(curwin_mut());
        let attr = if highlight {
            crate::highlight::hl_attr(HLF_8 as c_int)
        } else {
            0
        };
        ins_st().pc_row = curwin_mut().w_wrow;
        ins_st().pc_status = PC_STATUS_UNSET;
        grid_line_start(&raw mut (*curwin_ptr()).w_grid, ins_st().pc_row);
        if curwin_mut().w_onebuf_opt.wo_rl != 0 {
            ins_st().pc_col = curwin_mut().w_view_width - 1 - curwin_mut().w_wcol;
            if grid_line_getchar(ins_st().pc_col, None) == 0 {
                grid_line_put_schar(ins_st().pc_col - 1, schar_from_ascii(b' '), attr);
                curwin_mut().w_wcol -= 1;
                ins_st().pc_status = PC_STATUS_RIGHT;
            }
        } else {
            ins_st().pc_col = curwin_mut().w_wcol;
            if grid_line_getchar(ins_st().pc_col + 1, None) == 0 {
                ins_st().pc_status = PC_STATUS_LEFT;
            }
        }

        if ins_st().pc_status == PC_STATUS_UNSET {
            ins_st().pc_schar = grid_line_getchar(ins_st().pc_col, Some(&mut ins_st().pc_attr));
            ins_st().pc_status = PC_STATUS_SET;
        }

        let mut buf = [0 as c_char; MB_MAXCHAR + 1];
        let len = utf_char2bytes(c, (&raw mut buf).cast());
        grid_line_puts(ins_st().pc_col, (&raw const buf).cast(), len, attr);
        grid_line_flush();
    }
}

/// Return the effective prompt for the specified buffer.
pub fn buf_prompt_text(buf: &FileBuf) -> *mut c_char {
    buf.b_prompt_text
        .as_ref()
        .map_or(c"% ".as_ptr().cast_mut(), |s| s.as_ptr().cast_mut())
}

/// Return the effective prompt for the current buffer.
pub fn prompt_text() -> *mut c_char {
    buf_prompt_text(curbuf_mut())
}

fn init_prompt(cmdchar_todo: c_int) {
    unsafe {
        let prompt = prompt_text();
        let prompt_len = crate::memory::cstr_len(prompt) as Colnr;

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
        let text = ml_get(curbuf_mut().b_prompt_start.mark.lnum);
        let text_len = ml_get_len(curbuf_mut().b_prompt_start.mark.lnum);

        if curbuf_mut().b_prompt_start.mark.lnum == curwin_mut().w_cursor.lnum
            && (curbuf_mut().b_prompt_start.mark.col < prompt_len
                || curbuf_mut().b_prompt_start.mark.col > text_len
                || std::slice::from_raw_parts(
                    text.offset((curbuf_mut().b_prompt_start.mark.col - prompt_len) as isize)
                        .cast::<u8>(),
                    prompt_len as usize,
                ) != crate::memory::cstr_slice(prompt, prompt_len as usize))
        {
            if crate::memory::cstr_is_empty(text) {
                ml_replace(curbuf_mut().b_prompt_start.mark.lnum, prompt, true);
                inserted_bytes(curbuf_mut().b_prompt_start.mark.lnum, 0, 0, prompt_len);
            } else {
                let lnum = curbuf_mut().b_ml.ml_line_count;
                ml_append(lnum, prompt, 0, false);
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

        if cmdchar_todo == b'A' as c_int {
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
}

/// Return true if the cursor is in the editable position of the prompt line.
pub fn prompt_curpos_editable() -> bool {
    {
        curwin_mut().w_cursor.lnum > curbuf_mut().b_prompt_start.mark.lnum
            || (curwin_mut().w_cursor.lnum == curbuf_mut().b_prompt_start.mark.lnum
                && curwin_mut().w_cursor.col >= curbuf_mut().b_prompt_start.mark.col)
    }
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
    unsafe {
        if ins_st().pc_status != PC_STATUS_UNSET && ins_st().pc_row >= 0 {
            if ins_st().pc_status == PC_STATUS_RIGHT {
                curwin_mut().w_wcol += 1;
            }
            if ins_st().pc_status == PC_STATUS_RIGHT || ins_st().pc_status == PC_STATUS_LEFT {
                redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
            } else {
                grid_line_start(&raw mut (*curwin_ptr()).w_grid, ins_st().pc_row);
                grid_line_put_schar(ins_st().pc_col, ins_st().pc_schar, ins_st().pc_attr);
                grid_line_flush();
            }
        }
    }
}

/// Display a `$` at the end of changed text when `$` is in `cpoptions`.
pub fn display_dollar(col_arg: Colnr) {
    unsafe {
        let col = col_arg.max(0);
        if !redrawing() {
            return;
        }
        let save_col = curwin_mut().w_cursor.col;
        curwin_mut().w_cursor.col = col;

        let p = get_cursor_line_ptr();
        curwin_mut().w_cursor.col -= utf_head_off(p, p.offset(col as isize));
        curs_columns(curwin_mut(), 0);
        if curwin_mut().w_wcol < curwin_mut().w_view_width {
            edit_putchar(b'$' as c_int, false);
            g_st().dollar_vcol = curwin_mut().w_virtcol;
        }
        curwin_mut().w_cursor.col = save_col;
    }
}

pub fn undisplay_dollar() {
    {
        if g_st().dollar_vcol < 0 {
            return;
        }
        g_st().dollar_vcol = -1;
        redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
    }
}

/// Truncate the space at the end of a line.
pub fn truncate_spaces(line: *mut c_char, len: usize) {
    unsafe {
        let mut i = len as isize - 1;
        while i >= 0 && ascii_iswhite(*line.offset(i) as u8 as c_int) {
            if (g_st().state_global & REPLACE_FLAG) != 0 {
                replace_join(0);
            }
            i -= 1;
        }
        *line.offset(i + 1) = NUL;
    }
}

/// Backspace the cursor until the given column.
pub fn backspace_until_column(col: c_int) {
    {
        while curwin_mut().w_cursor.col > col {
            curwin_mut().w_cursor.col -= 1;
            if (g_st().state_global & REPLACE_FLAG) != 0 {
                replace_do_bs(col);
            } else if !del_char_after_col(col) {
                break;
            }
        }
    }
}

fn del_char_after_col(limit_col: c_int) -> bool {
    {
        if limit_col >= 0 {
            let ecol = curwin_mut().w_cursor.col + 1;
            mb_adjust_cursor();
            while curwin_mut().w_cursor.col < limit_col {
                let l = utf_ptr2len(get_cursor_pos_ptr());
                if l == 0 {
                    break;
                }
                curwin_mut().w_cursor.col += l;
            }
            if crate::memory::cstr_is_empty(get_cursor_pos_ptr())
                || curwin_mut().w_cursor.col == ecol
            {
                return false;
            }
            del_bytes(ecol - curwin_mut().w_cursor.col, false, true);
        } else {
            del_char(false);
        }
        true
    }
}

/// Next character is interpreted literally.
pub fn get_literal(no_simplify: bool) -> c_int {
    let mut nc: c_int;
    let mut hex = false;
    let mut octal = false;
    let mut unicode: c_int = 0;

    if g_st().got_int {
        return CTRL_C_CONST;
    }

    g_st().no_mapping += 1;
    let mut cc: c_int = 0;
    let mut i: c_int = 0;
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
        if nc == b'x' as c_int || nc == b'X' as c_int {
            hex = true;
        } else if nc == b'o' as c_int || nc == b'O' as c_int {
            octal = true;
        } else if nc == b'u' as c_int || nc == b'U' as c_int {
            unicode = nc;
        } else {
            if hex || unicode != 0 {
                if !ascii_isxdigit(nc) {
                    break;
                }
                cc = cc.wrapping_mul(16).wrapping_add(hex2nr(nc));
            } else if octal {
                if nc < b'0' as c_int || nc > b'7' as c_int {
                    break;
                }
                cc = cc * 8 + nc - (b'0' as c_int);
            } else {
                if !ascii_isdigit(nc) {
                    break;
                }
                cc = cc * 10 + nc - (b'0' as c_int);
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
            if (unicode == b'u' as c_int && i >= 4) || (unicode == b'U' as c_int && i >= 8) {
                break;
            }
        } else if i >= 3 {
            break;
        }
    }
    if i == 0 {
        if nc == K_ZERO {
            cc = b'\n' as c_int;
            nc = 0;
        } else {
            cc = nc;
            nc = 0;
        }
    }

    if cc == 0 {
        cc = b'\n' as c_int;
    }

    g_st().no_mapping -= 1;
    if nc != 0 {
        vungetc(nc);
        g_st().mod_mask = 0;
    }
    g_st().got_int = false;
    cc
}

fn insert_special(mut c: c_int, mut allow_modmask: c_int, mut ctrlv: c_int) {
    unsafe {
        if (g_st().mod_mask & MOD_MASK_CMD) != 0 {
            allow_modmask = 1;
        }
        if is_special_fn(c) || (g_st().mod_mask != 0 && allow_modmask != 0) {
            let p = get_special_key_name(c, g_st().mod_mask);
            let len = crate::memory::cstr_len(p);
            c = crate::memory::cstr_byte_at(p, len - 1) as c_int;
            if len > 2 {
                if !stop_arrow() {
                    return;
                }
                *p.add(len - 1) = NUL;
                ins_str(p, len - 1);
                redo_append_lit(p, -1);
                ctrlv = 0;
            }
        }
        if stop_arrow() {
            insertchar(c, if ctrlv != 0 { INSCHAR_CTRLV } else { 0 }, -1);
        }
    }
}

pub fn insertchar(mut c: c_int, flags: c_int, second_indent: c_int) {
    unsafe {
        let force_format = (flags & INSCHAR_FORMAT) != 0;
        let textwidth = comp_textwidth(force_format);
        let fo_ins_blank = has_format_option(K_FO_INS_BLANK);

        if textwidth > 0
            && (force_format
                || (!ascii_iswhite(c)
                    && !((g_st().state_global & REPLACE_FLAG) != 0
                        && (g_st().state_global & VREPLACE_FLAG) == 0
                        && !crate::memory::cstr_is_empty(get_cursor_pos_ptr()))
                    && (curwin_mut().w_cursor.lnum != g_st().ins_global.start.lnum
                        || ((!has_format_option(K_FO_INS_LONG)
                            || g_st().ins_global.start_textlen <= textwidth)
                            && (!fo_ins_blank
                                || g_st().ins_global.start_blank_vcol <= textwidth)))))
        {
            let mut do_internal = true;
            let virtcol = get_nolist_virtcol()
                + char2cells(if c != NUL as c_int { c } else { gchar_cursor() });

            if curbuf_mut().b_p_fex.cb_type() != K_CALLBACK_NONE_CONST
                && (flags & INSCHAR_NO_FEX) == 0
                && (force_format || virtcol > textwidth)
            {
                do_internal = fex_format(curwin_mut().w_cursor.lnum, 1, c) != 0;
                g_st().ins_global.need_undo = true;
            }
            if do_internal {
                internal_format(textwidth, second_indent, flags, c == NUL as c_int, c);
            }
        }

        if c == NUL as c_int {
            return;
        }

        if g_st().ins_global.did_ai && c == g_st().ins_global.end_comment_pending {
            let mut lead_end = [0 as c_char; COM_MAX_LEN];
            let mut p: *mut c_char = ptr::null_mut();
            let line = get_cursor_line_ptr();
            let mut i = get_leader_len(line, &raw mut p, false, true);
            if i > 0 && !vim_strchr(p, c_int::from(COM_MIDDLE as u8)).is_null() {
                while !crate::memory::cstr_is_empty(p) && *p.offset(-1) != b':' as c_char {
                    p = p.add(1);
                }
                let mut middle_len =
                    copy_option_part(&raw mut p, (&raw mut lead_end).cast(), COM_MAX_LEN, c",")
                        as c_int;
                while middle_len > 0
                    && ascii_iswhite(lead_end[(middle_len - 1) as usize] as u8 as c_int)
                {
                    middle_len -= 1;
                }

                while !crate::memory::cstr_is_empty(p) && *p.offset(-1) != b':' as c_char {
                    p = p.add(1);
                }
                let end_len =
                    copy_option_part(&raw mut p, (&raw mut lead_end).cast(), COM_MAX_LEN, c",")
                        as c_int;

                i = curwin_mut().w_cursor.col;
                while {
                    i -= 1;
                    i >= 0 && ascii_iswhite(crate::memory::cstr_byte_at(line, i as usize) as c_int)
                } {}
                i += 1;
                i -= middle_len;

                if i >= 0
                    && end_len > 0
                    && (lead_end[(end_len - 1) as usize] as u8 as c_int)
                        == g_st().ins_global.end_comment_pending
                {
                    backspace_until_column(i);
                    ins_bytes_len((&raw mut lead_end).cast(), (end_len - 1) as usize);
                }
            }
        }
        g_st().ins_global.end_comment_pending = NUL as c_int;

        g_st().ins_global.did_ai = false;
        g_st().ins_global.did_si = false;
        g_st().ins_global.can_si = false;
        g_st().ins_global.can_si_back = false;

        if !is_special_ins(c)
            && utf_char2len(c) == 1
            && !has_event(EVENT_INSERTCHARPRE)
            && !*test_disable_char_avail.as_ptr()
            && vpeekc() != NUL as c_int
            && (g_st().state_global & REPLACE_FLAG) == 0
            && !cindent_on()
            && opt_vars().p_ri == 0
        {
            let mut buf = [0 as c_char; INPUT_BUFLEN + 1];
            let mut virtcol: Colnr = 0;

            buf[0] = c as c_char;
            let mut i: usize = 1;
            if textwidth > 0 {
                virtcol = get_nolist_virtcol();
            }
            while {
                c = vpeekc();
                c != NUL as c_int
                    && !is_special_ins(c)
                    && utf_byte2len(c) == 1
                    && i < INPUT_BUFLEN
                    && (textwidth == 0 || {
                        virtcol += ascii2cells(buf[i - 1] as u8 as c_int);
                        virtcol < textwidth
                    })
                    && !(!g_st().no_abbr
                        && !vim_iswordc(c)
                        && vim_iswordc(buf[i - 1] as u8 as c_int))
            } {
                c = vgetc();
                buf[i] = c as c_char;
                i += 1;
            }

            do_digraph(-1);
            do_digraph(buf[i - 1] as u8 as c_int);
            buf[i] = NUL;
            ins_str((&raw mut buf).cast(), i);
            let start_idx = if (flags & INSCHAR_CTRLV) != 0 {
                redo_literal(buf[0] as u8 as c_int);
                1
            } else {
                0
            };
            if buf[start_idx] != NUL {
                redo_append_lit((&raw const buf).cast::<c_char>().add(start_idx), -1);
            }
        } else {
            let cc = utf_char2len(c);
            if cc > 1 {
                let mut buf = [0 as c_char; MB_MAXCHAR + 1];
                utf_char2bytes(c, (&raw mut buf).cast());
                buf[cc as usize] = NUL;
                ins_char_bytes((&raw mut buf).cast(), cc as usize);
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
}

fn redo_literal(c: c_int) {
    if ascii_isdigit(c) {
        let mut buf = [0 as c_char; 10];
        vim_snprintf!((&raw mut buf).cast(), size_of_val(&buf), c"%03d", c,);
        redo_append_str((&raw const buf).cast(), -1);
    } else {
        redo_append_char(c);
    }
}

/// Called when the cursor moves in insert-mode without editing text.
pub fn start_arrow(end_insert_pos: *mut CursorPos, end_change: bool, move_key: c_int) {
    if g_st().ins_global.moved == K_INS_NONE && end_change {
        stop_insert(end_insert_pos, 0, 0);
    }
    if g_st().ins_global.moved != K_INS_JUMP {
        if move_key == NUL as c_int {
            redo_append_str(c"\x1b", 1);
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

fn start_arrow_with_change(end_insert_pos: &mut CursorPos, end_change: bool, move_key: c_int) {
    if !end_change && g_st().ins_global.moved != K_INS_JUMP {
        redo_append_char(CTRL_G);
        redo_append_char(b'U' as c_int);
    }
    start_arrow(&raw mut *end_insert_pos, end_change, move_key);
}

fn check_spell_redraw() {
    {
        if g_st().spell_redraw_lnum != 0 {
            let lnum = g_st().spell_redraw_lnum;
            g_st().spell_redraw_lnum = 0;
            redraw_winline(crate::globals::curwin_mut(), lnum);
        }
    }
}

pub fn stop_arrow() -> bool {
    {
        if g_st().ins_global.moved != K_INS_NONE {
            g_st().ins_global.start = curwin_mut().w_cursor;
            if g_st().ins_global.start.col > g_st().ins_global.start_orig.col
                && !g_st().ins_global.need_undo
            {
                g_st().ins_global.update_start_orig = false;
            }
            g_st().ins_global.start_textlen = linetabsize_str(get_cursor_line_ptr());

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
                        cmd: b'i' as c_int,
                        ..CmdSpec::default()
                    },
                );
                g_st().ins_global.new_insert_skip = 2;
                mc_ins_cascade_restart();
            } else {
                let redo = redo_keys(ptr::null());
                g_st().ins_global.new_insert_skip =
                    redo.as_ref().map_or(0, |s| s.as_bytes().len() as c_int);
            }
        } else if g_st().ins_global.need_undo && u_save_cursor() {
            if lt(curwin_mut().w_cursor, g_st().ins_global.start) {
                g_st().ins_global.start = curwin_mut().w_cursor;
                g_st().ins_global.start_textlen = linetabsize_str(get_cursor_line_ptr());
            }
            g_st().ins_global.need_undo = false;
        }

        fold_open_cursor();

        !(g_st().ins_global.moved != K_INS_NONE || g_st().ins_global.need_undo)
    }
}

fn stop_insert(end_insert_pos: *mut CursorPos, esc: c_int, nomove: c_int) {
    unsafe {
        stop_redo_ins();
        ins_st().replace_stack = Vec::new();

        let redo_cstr = redo_keys(ptr::null());
        let added = match redo_cstr {
            None => 0,
            Some(ref s) => (s.as_bytes().len() as c_int) - g_st().ins_global.new_insert_skip,
        };
        if g_st().ins_global.did_restart_edit == 0 || added > 0 {
            ins_st().last_insert = redo_cstr;
            ins_st().last_insert_skip = if added < 0 {
                0
            } else {
                g_st().ins_global.new_insert_skip
            };
        }

        if g_st().ins_global.moved == K_INS_NONE && !end_insert_pos.is_null() {
            let mut cc: c_int;
            if !g_st().ins_global.need_undo && has_format_option(K_FO_AUTO) {
                let tpos = curwin_mut().w_cursor;
                cc = b'x' as c_int;
                if curwin_mut().w_cursor.col > 0 && gchar_cursor() == NUL as c_int {
                    dec_cursor();
                    cc = gchar_cursor();
                    if !ascii_iswhite(cc) {
                        curwin_mut().w_cursor = tpos;
                    }
                }

                auto_format(true, false);

                if ascii_iswhite(cc) {
                    if gchar_cursor() != NUL as c_int {
                        inc_cursor();
                    }
                    if gchar_cursor() == NUL as c_int
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
                    || (vim_strchr(opt_vars().p_cpo, K_CPO_INDENT).is_null()
                        && curwin_mut().w_cursor.lnum != (*end_insert_pos).lnum))
                && (*end_insert_pos).lnum <= curbuf_mut().b_ml.ml_line_count
            {
                let mut tpos = curwin_mut().w_cursor;
                let prev_col = (*end_insert_pos).col;

                curwin_mut().w_cursor = *end_insert_pos;
                check_cursor_col(curwin_mut());

                let mut strip_col = curwin_mut().w_cursor.col;
                if gchar_cursor() == NUL as c_int && strip_col > 0 {
                    strip_col -= 1;
                }

                if *skipwhite(get_cursor_line_ptr().offset(strip_col as isize)) == NUL {
                    curwin_mut().w_cursor.col = strip_col;
                    loop {
                        if gchar_cursor() == NUL as c_int && curwin_mut().w_cursor.col > 0 {
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
                        if cc != NUL as c_int && gchar_pos(&mut tpos) == NUL as c_int {
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

        if !end_insert_pos.is_null() {
            curbuf_mut().b_op_start = g_st().ins_global.start;
            curbuf_mut().b_op_start_orig = g_st().ins_global.start_orig;
            curbuf_mut().b_op_end = *end_insert_pos;
        }
    }
}

/// Set the last inserted text to a single character.
pub fn set_last_insert(c: c_int) {
    unsafe {
        let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 5];
        let mut s = buf.as_mut_ptr();
        if c < b' ' as c_int || c == DEL {
            *s = CTRL_V_CONST as c_char;
            s = s.add(1);
        }
        s = add_char2buf(c, s);
        *s = ESC as c_char;
        s = s.add(1);
        *s = NUL;
        ins_st().last_insert = Some(crate::memory::cstr_to_cstring(buf.as_ptr()));
        ins_st().last_insert_skip = 0;
    }
}

pub fn free_last_insert() {
    ins_st().last_insert = None;
}

pub fn beginline(flags: c_int) {
    unsafe {
        if (flags & BL_SOL) != 0 && opt_vars().p_sol == 0 {
            coladvance(curwin_mut(), curwin_mut().w_curswant);
        } else {
            curwin_mut().w_cursor.col = 0;
            curwin_mut().w_cursor.coladd = 0;

            if (flags & (BL_WHITE | BL_SOL)) != 0 {
                let mut ptr = get_cursor_line_ptr();
                while ascii_iswhite(crate::memory::cstr_first_byte(ptr) as c_int)
                    && !((flags & BL_FIX) != 0 && crate::memory::cstr_byte_at(ptr, 1) == 0)
                {
                    curwin_mut().w_cursor.col += 1;
                    ptr = ptr.add(1);
                }
            }
            curwin_mut().w_set_curswant = true;
        }
        adjust_skipcol();
    }
}

pub fn oneright() -> bool {
    unsafe {
        if virtual_active(curwin_mut()) {
            let prevpos = curwin_mut().w_cursor;
            let ptr = get_cursor_pos_ptr();
            let step = if *ptr != TAB as c_char && vim_isprintc(utf_ptr2char(ptr)) {
                ptr2cells(ptr)
            } else {
                1
            };
            coladvance(curwin_mut(), getviscol() + step);
            curwin_mut().w_set_curswant = true;
            return prevpos.col != curwin_mut().w_cursor.col
                || prevpos.coladd != curwin_mut().w_cursor.coladd;
        }

        let ptr = get_cursor_pos_ptr();
        if crate::memory::cstr_is_empty(ptr) {
            return false;
        }

        let l = utfc_ptr2len(ptr);
        if crate::memory::cstr_byte_at(ptr, l as usize) == 0
            && (get_ve_flags(curwin_mut()) & K_OPT_VE_FLAG_ONEMORE) == 0
        {
            return false;
        }
        curwin_mut().w_cursor.col += l;
        curwin_mut().w_set_curswant = true;
        adjust_skipcol();
        true
    }
}

pub fn oneleft() -> bool {
    unsafe {
        if virtual_active(curwin_mut()) {
            let v = getviscol();
            if v == 0 {
                return false;
            }
            let mut width: c_int = 1;
            loop {
                coladvance(curwin_mut(), v - width);
                if getviscol() < v {
                    break;
                }
                width += 1;
            }

            if curwin_mut().w_cursor.coladd == 1 {
                let ptr = get_cursor_pos_ptr();
                if *ptr != TAB as c_char && vim_isprintc(utf_ptr2char(ptr)) && ptr2cells(ptr) > 1 {
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
}

/// Move the cursor up `n` lines in window `wp`.
pub fn cursor_up_inner(wp: &mut WinInfoInfo, mut n: Linenr, skip_conceal: bool) {
    let mut lnum = wp.w_cursor.lnum;

    if n >= lnum {
        lnum = 1;
    } else if win_lines_concealed(&mut *wp) {
        has_folding(&raw mut *wp, lnum, Some(&mut lnum), ptr::null_mut());
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
                has_folding(&raw mut *wp, lnum, Some(&mut lnum), ptr::null_mut());
            }
        }
        lnum = lnum.max(1);
    } else {
        lnum -= n;
    }

    wp.w_cursor.lnum = lnum;
}

pub fn cursor_up(n: Linenr, upd_topline: bool) -> bool {
    {
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
}

/// Move the cursor down `n` lines in window `wp`.
pub fn cursor_down_inner(wp: &mut WinInfoInfo, mut n: c_int, skip_conceal: bool) {
    {
        let mut lnum = wp.w_cursor.lnum;
        let line_count = wp.buf_mut().b_ml.ml_line_count;

        if lnum + n >= line_count {
            lnum = line_count;
        } else if win_lines_concealed(&mut *wp) {
            let mut last: Linenr = 0;
            while n > 0 {
                n -= 1;
                if has_folding_win(wp, lnum, None, &raw mut last, true, None) {
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
}

pub fn cursor_down(n: c_int, upd_topline: bool) -> bool {
    {
        let mut lnum = curwin_mut().w_cursor.lnum;
        has_folding_win(curwin_mut(), lnum, None, &raw mut lnum, true, None);
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
}

/// Stuff the last inserted text in the read buffer.
pub fn stuff_inserted(c: c_int, mut count: c_int, no_esc: c_int) -> bool {
    unsafe {
        let mut last: c_char = NUL;
        let mut insert = get_last_insert();
        if insert.data.is_null() {
            emsg(gettext(E_NOINSTEXT));
            return false;
        }

        if c != NUL as c_int {
            stuffchar_readbuff(c);
        }

        if insert.size > 0 {
            let mut p = insert.data.add(insert.size - 1);
            while p >= insert.data {
                if *p == ESC as c_char {
                    insert.size = p.offset_from(insert.data) as usize;
                    break;
                }
                if p == insert.data {
                    break;
                }
                p = p.sub(1);
            }
        }

        if insert.size > 0 {
            let p = insert.data.add(insert.size - 1);
            if (crate::memory::cstr_first_byte(p) == b'0'
                || crate::memory::cstr_first_byte(p) == b'^')
                && (no_esc != 0 || (*insert.data == CTRL_D_CONST as c_char && count > 1))
            {
                last = *p;
                insert.size -= 1;
            }
        }

        loop {
            stuff_readbuff_len(insert.data, insert.size as isize);
            if last == b'0' as c_char {
                stuff_readbuff_len(c"\x16048", 4);
            } else if last == b'^' as c_char {
                stuff_readbuff_len(c"\x16^", 2);
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
}

pub fn get_last_insert() -> NvimString {
    unsafe {
        if let Some(li) = &ins_st().last_insert {
            let skip = ins_st().last_insert_skip as usize;
            let bytes = li.as_bytes();
            NvimString {
                data: li.as_ptr().add(skip).cast_mut(),
                size: bytes.len().saturating_sub(skip),
            }
        } else {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        }
    }
}

pub fn get_last_insert_save() -> Option<std::ffi::CString> {
    unsafe {
        let mut insert = get_last_insert();
        if insert.data.is_null() {
            return None;
        }
        if insert.size > 0 && *insert.data.add(insert.size - 1) == ESC as c_char {
            insert.size -= 1;
        }
        let slice = crate::memory::cstr_slice(insert.data, insert.size);
        Some(std::ffi::CString::new(slice).unwrap_or_default())
    }
}

fn echeck_abbr(c: c_int) -> bool {
    {
        if opt_vars().p_paste != 0 || g_st().no_abbr || g_st().ins_global.moved != K_INS_NONE {
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
}

/// Push character that is replaced onto the replace stack.
pub fn replace_push(str_: impl crate::strings::IntoCCharPtr, len: usize) {
    let str_ = str_.into_c_char_ptr().cast_mut();
    unsafe {
        let stack = replace_stack_mut();
        let off = g_st().replace_offset as usize;
        if stack.len() < off {
            return;
        }
        let pos = stack.len() - off;
        let slice = std::slice::from_raw_parts(str_, len);
        stack.splice(pos..pos, slice.iter().copied());
    }
}

pub fn replace_push_nul() {
    replace_push(c"", 1);
}

fn replace_pop_if_nul() -> c_int {
    let stack = replace_stack_mut();
    let ch = stack.last().map_or(-1, |&b| c_int::from(b as u8));
    if ch == c_int::from(NUL as u8) {
        stack.pop();
    }
    ch
}

pub fn replace_join(mut off: c_int) {
    let stack = replace_stack_mut();
    let mut i = stack.len();
    while i > 0 {
        i -= 1;
        if stack[i] == NUL {
            let cur_off = off;
            off -= 1;
            if cur_off <= 0 {
                stack.remove(i);
                return;
            }
        }
    }
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
    unsafe {
        let stack = replace_stack_mut();
        let ptr = stack.as_ptr();
        let size = stack.len();
        let len = (utf_head_off(ptr, ptr.add(size - 1)) + 1) as usize;
        let new_size = size - len;
        let mut popped = stack.split_off(new_size);
        ins_bytes_len(popped.as_mut_ptr(), len);
    }
}

fn replace_do_bs(limit_col: c_int) {
    unsafe {
        let mut start_vcol: Colnr = 0;
        let l_state = g_st().state_global;

        let cc = replace_pop_if_nul();
        if cc > 0 {
            let mut orig_len: c_int = 0;
            let mut orig_vcols: c_int = 0;
            if (l_state & VREPLACE_FLAG) != 0 {
                getvcol(
                    curwin_mut(),
                    &mut curwin_mut().w_cursor,
                    None,
                    Some(&mut start_vcol),
                    None,
                    0,
                );
                orig_vcols = win_chartabsize(curwin_mut(), get_cursor_pos_ptr(), start_vcol);
            }
            del_char_after_col(limit_col);
            if (l_state & VREPLACE_FLAG) != 0 {
                orig_len = get_cursor_pos_len();
            }
            replace_pop_ins();

            if (l_state & VREPLACE_FLAG) != 0 {
                let p = get_cursor_pos_ptr();
                let ins_len = get_cursor_pos_len() - orig_len;
                let mut vcol = start_vcol;
                let mut i: c_int = 0;
                while i < ins_len {
                    vcol += win_chartabsize(curwin_mut(), p.offset(i as isize), vcol);
                    i += utfc_ptr2len(p.offset(i as isize));
                }
                vcol -= start_vcol;

                curwin_mut().w_cursor.col += ins_len;
                while vcol > orig_vcols && gchar_cursor() == b' ' as c_int {
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
}

fn ins_reg() {
    unsafe {
        let mut need_redraw = false;
        let mut literally: c_int = 0;
        let vis_active = g_st().visual_global.active;

        ins_st().pc_status = PC_STATUS_UNSET;
        if redrawing() && !char_avail() {
            ins_redraw(false);
            edit_putchar(b'"' as c_int, true);
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
        if regname == b'=' as c_int {
            let curpos = curwin_mut().w_cursor;
            g_st().u_sync_once = 2;
            regname = get_expr_register();
            curwin_mut().w_cursor = curpos;
            check_cursor(curwin_mut());
        }
        if regname == NUL as c_int || !valid_yank_reg(regname, false) {
            vim_beep(K_OPT_BO_FLAG_REGISTER);
            need_redraw = true;
        } else {
            let reg: *mut Yankreg = get_yank_register(regname, YREG_PASTE);

            if literally == CTRL_O || literally == CTRL_P {
                redo_append_char(CTRL_R_CONST);
                redo_append_char(literally);
                redo_append_char(regname);

                do_put(
                    regname,
                    ptr::null_mut(),
                    BACKWARD,
                    1,
                    (if literally == CTRL_P {
                        PUT_FIXINDENT
                    } else {
                        0
                    }) | PUT_CURSEND,
                );
            } else if (*reg).y_array.len() > 1 && is_literal_register(regname) {
                redo_append_char(CTRL_R_CONST);
                redo_append_char(regname);
                do_put(regname, ptr::null_mut(), BACKWARD, 1, PUT_CURSEND);
            } else if !insert_reg(regname, ptr::null_mut(), literally != 0) {
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
}

fn ins_ctrl_g() {
    {
        setcursor();

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        let c = plain_vgetc();
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
        match c {
            K_UP | CTRL_K_CONST => ins_up(true),
            c if c == b'k' as c_int => ins_up(true),
            K_DOWN | CTRL_J_CONST => ins_down(true),
            c if c == b'j' as c_int => ins_down(true),
            c if c == b'u' as c_int => {
                u_sync(true);
                g_st().ins_global.need_undo = true;
                g_st().ins_global.update_start_orig = false;
                g_st().ins_global.start = curwin_mut().w_cursor;
            }
            c if c == b'U' as c_int => {
                g_st().ins_global.dont_sync_undo = K_NONE;
            }
            ESC => {}
            _ => {
                vim_beep(K_OPT_BO_FLAG_CTRLG);
            }
        }
    }
}

fn ins_ctrl_hat() {
    {
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
}

fn ins_esc(count: &mut c_int, cmdchar: c_int, nomove: bool) -> bool {
    unsafe {
        check_spell_redraw();

        let temp = curwin_mut().w_cursor.col;
        if ins_st().ins_esc_disabled_redraw {
            g_st().redrawing_disabled -= 1;
            ins_st().ins_esc_disabled_redraw = false;
        }
        if g_st().ins_global.moved == K_INS_NONE {
            if cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
                redo_append_str(c"\x1b", 1);
            }

            if *count > 0 {
                line_breakcheck();
                if g_st().got_int {
                    *count = 0;
                }
            }

            *count -= 1;
            if *count > 0 {
                if !vim_strchr(opt_vars().p_cpo, K_CPO_REPLCNT).is_null() {
                    g_st().state_global &= !REPLACE_FLAG;
                }

                start_redo_ins();
                if cmdchar == b'r' as c_int || cmdchar == b'v' as c_int {
                    stuff_redo_readbuff(c"\x1b");
                }
                g_st().redrawing_disabled += 1;
                ins_st().ins_esc_disabled_redraw = true;
                return false;
            }
            stop_insert(&raw mut (*curwin_ptr()).w_cursor, 1, c_int::from(nomove));
            undisplay_dollar();
        } else if g_st().ins_global.moved == K_INS_ARROW
            && cmdchar != b'r' as c_int
            && cmdchar != b'v' as c_int
        {
            redo_append_str(c"\x1b", 1);
        }

        if cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
            ins_apply_autocmds(EVENT_INSERTLEAVEPRE);
        }

        if g_st().restart_edit == NUL as c_int && temp == curwin_mut().w_cursor.col {
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
            && (g_st().restart_edit == NUL as c_int
                || (gchar_cursor() == NUL as c_int && !g_st().visual_global.active))
            && !g_st().ins_global.revins_on
        {
            if curwin_mut().w_cursor.coladd > 0 || get_ve_flags(curwin_mut()) == K_OPT_VE_FLAG_ALL {
                oneleft();
                if g_st().restart_edit != NUL as c_int {
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

        if g_st().reg_recording != 0 || g_st().restart_edit != NUL as c_int {
            showmode();
        } else if opt_vars().p_smd != 0
            && (g_st().got_int || !skip_showmode())
            && !(opt_vars().p_ch == 0 && !ui_has(K_UIMESSAGES))
        {
            unshowmode(false);
        }
        true
    }
}

fn ins_ctrl_() {
    {
        if g_st().ins_global.revins_on
            && g_st().ins_global.revins_chars != 0
            && g_st().ins_global.revins_scol >= 0
        {
            while gchar_cursor() != NUL as c_int && g_st().ins_global.revins_chars != 0 {
                g_st().ins_global.revins_chars -= 1;
                curwin_mut().w_cursor.col += 1;
            }
        }
        opt_vars().p_ri = c_int::from(opt_vars().p_ri == 0);
        g_st().ins_global.revins_on = g_st().state_global == MODE_INSERT && opt_vars().p_ri != 0;
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
}

fn ins_start_select(c: c_int) -> bool {
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
                let buf = [
                    K_SPECIAL as c_char,
                    KS_MODIFIER as c_char,
                    g_st().mod_mask as u8 as c_char,
                    NUL,
                ];
                stuff_readbuff_len((&raw const buf).cast(), 3);
            }
            stuffchar_readbuff(c);
            true
        }
        _ => false,
    }
}

fn ins_insert(replace_state: c_int) {
    let mode_str = if (g_st().state_global & REPLACE_FLAG) != 0 {
        c"i".as_ptr()
    } else if replace_state == MODE_VREPLACE {
        c"v".as_ptr()
    } else {
        c"r".as_ptr()
    };
    set_vim_var_string(VV_INSERTMODE, mode_str, 1);
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
    {
        g_st().visual_global.restart_select = 0;
        if (g_st().state_global & VREPLACE_FLAG) != 0 {
            g_st().restart_edit = b'V' as c_int;
        } else if (g_st().state_global & REPLACE_FLAG) != 0 {
            g_st().restart_edit = b'R' as c_int;
        } else {
            g_st().restart_edit = b'I' as c_int;
        }
        if virtual_active(curwin_mut()) {
            g_st().ins_at_eol = false;
        } else {
            g_st().ins_at_eol = gchar_cursor() == NUL as c_int;
        }
    }
}

fn ins_shift(c: c_int, lastc: c_int) {
    unsafe {
        if !stop_arrow() {
            return;
        }
        redo_append_char(c);

        if c == CTRL_D_CONST
            && (lastc == b'0' as c_int || lastc == b'^' as c_int)
            && curwin_mut().w_cursor.col > 0
        {
            curwin_mut().w_cursor.col -= 1;
            del_char(false);
            if (g_st().state_global & REPLACE_FLAG) != 0 {
                replace_pop_ins();
            }
            if lastc == b'^' as c_int {
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

        if g_st().ins_global.did_ai && *skipwhite(get_cursor_line_ptr()) != NUL {
            g_st().ins_global.did_ai = false;
        }
        g_st().ins_global.did_si = false;
        g_st().ins_global.can_si = false;
        g_st().ins_global.can_si_back = false;
        g_st().ins_global.can_cindent = false;
    }
}

fn ins_del() {
    {
        if !stop_arrow() {
            return;
        }
        if gchar_cursor() == NUL as c_int {
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
}

fn ins_bs(c: c_int, mut mode: c_int, inserted_space_p: &mut c_int) -> bool {
    unsafe {
        let mut cc: c_int;
        let mut temp: c_int = 0;
        let mut did_backspace = false;
        let mut call_fix_indent = false;

        if buf_is_empty(curbuf_mut())
            || (!g_st().ins_global.revins_on
                && ((curwin_mut().w_cursor.lnum == 1 && curwin_mut().w_cursor.col == 0)
                    || (!can_bs(BS_START)
                        && ((g_st().ins_global.moved != K_INS_NONE
                            && !bt_prompt(Some(crate::globals::curbuf_mut())))
                            || (curwin_mut().w_cursor.lnum == g_st().ins_global.start_orig.lnum
                                && curwin_mut().w_cursor.col
                                    <= g_st().ins_global.start_orig.col)))
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
        g_st().ins_global.end_comment_pending = NUL as c_int;
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
                        let ptr =
                            ml_get_buf(crate::globals::curbuf_mut(), curwin_mut().w_cursor.lnum);
                        let len = get_cursor_line_len();
                        if len > 0 && *ptr.add((len - 1) as usize) == b' ' as c_char {
                            let new_line =
                                std::slice::from_raw_parts(ptr, (len - 1) as usize).to_vec();
                            ml_replace_dirty_line(curbuf_mut(), &new_line, false);
                        }
                    }

                    do_join(2, false, false, false, false);
                    if temp == NUL as c_int && gchar_cursor() != NUL as c_int {
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
                && (curbuf_mut().b_p_ai != 0 || cindent_on())
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
                && ((opt_vars().p_sta != 0 && in_indent)
                    || ((get_sts_value() != 0 || tabstop_count(&curbuf_mut().b_p_vsts_array) != 0)
                        && curwin_mut().w_cursor.col > 0
                        && (*get_cursor_pos_ptr().offset(-1) == TAB as c_char
                            || (*get_cursor_pos_ptr().offset(-1) == b' ' as c_char
                                && (*inserted_space_p == 0
                                    || g_st().ins_global.moved != K_INS_NONE)))))
            {
                *inserted_space_p = 0;

                let use_ts =
                    curwin_mut().w_onebuf_opt.wo_list == 0 || curwin_mut().w_p_lcs_chars.tab1 != 0;
                let line = get_cursor_line_ptr();
                let cursor_ptr = line.offset(curwin_mut().w_cursor.col as isize).cast_const();

                let mut vcol: Colnr = 0;
                let mut space_vcol: Colnr = 0;
                let mut sci = utf_ptr2_str_char_info(line);
                let mut space_sci = sci;
                let mut prev_space = false;

                while sci.ptr < cursor_ptr {
                    let cur_space = ascii_iswhite(sci.chr.value);
                    if !prev_space && cur_space {
                        space_sci = sci;
                        space_vcol = vcol;
                    }
                    let cli = utf_cluster_info(sci);
                    vcol += charsize_nowrap(
                        curbuf_mut(),
                        sci.ptr,
                        use_ts,
                        vcol,
                        sci.chr.value,
                        cli.cells,
                    );
                    sci = cli.next;
                    prev_space = cur_space;
                }

                let mut want_vcol = if vcol > 0 { vcol - 1 } else { 0 };
                if opt_vars().p_sta != 0 && in_indent {
                    want_vcol -= want_vcol % (get_sw_value(curbuf_mut()) as Colnr);
                } else {
                    want_vcol = tabstop_start(
                        want_vcol,
                        get_sts_value() as c_int,
                        &curbuf_mut().b_p_vsts_array,
                    );
                }

                loop {
                    let cli = utf_cluster_info(space_sci);
                    let size = charsize_nowrap(
                        curbuf_mut(),
                        space_sci.ptr,
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
                let want_col = space_sci.ptr.offset_from(line.cast_const()) as Colnr;

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
                        ins_char(b' ' as c_int);
                    } else {
                        ins_str(c" ", 1);
                        if (g_st().state_global & REPLACE_FLAG) != 0 {
                            replace_push_nul();
                        }
                    }
                    space_vcol += 1;
                }
            } else {
                let mut cclass = mb_get_class(get_cursor_pos_ptr());
                loop {
                    if !g_st().ins_global.revins_on {
                        dec_cursor();
                    }
                    cc = gchar_cursor();
                    let prev_cclass = cclass;
                    cclass = mb_get_class(get_cursor_pos_ptr());
                    if mode == BACKSPACE_WORD && !ascii_isspace(cc) {
                        mode = BACKSPACE_WORD_NOT_SPACE;
                        temp = c_int::from(vim_iswordc(cc));
                    } else if mode == BACKSPACE_WORD_NOT_SPACE
                        && (ascii_isspace(cc)
                            || c_int::from(vim_iswordc(cc)) != temp
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
                        if opt_vars().p_deco != 0 {
                            let p0 = get_cursor_pos_ptr();
                            has_composing = utf_composinglike(
                                p0,
                                p0.add(utf_ptr2len(p0) as usize),
                                (ptr::null_mut::<GraphemeState>()).as_mut(),
                            );
                        }
                        del_char(false);
                        if has_composing {
                            inc_cursor();
                        }
                        if g_st().ins_global.revins_chars != 0 {
                            g_st().ins_global.revins_chars -= 1;
                            g_st().ins_global.revins_legal += 1;
                        }
                        if g_st().ins_global.revins_on && gchar_cursor() == NUL as c_int {
                            break;
                        }
                    }
                    if mode == BACKSPACE_CHAR {
                        break;
                    }
                    if !(g_st().ins_global.revins_on
                        || (curwin_mut().w_cursor.col > mincol
                            && (can_bs(BS_NOSTOP)
                                || curwin_mut().w_cursor.lnum
                                    != g_st().ins_global.start_orig.lnum
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

        if !vim_strchr(opt_vars().p_cpo, K_CPO_BACKSPACE).is_null() && g_st().dollar_vcol == -1 {
            g_st().dollar_vcol = curwin_mut().w_virtcol;
        }

        if did_backspace {
            fold_open_cursor();
        }
        did_backspace
    }
}

fn ins_left() {
    {
        let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
            fold_open_cursor();
        }
        undisplay_dollar();
        let mut tpos = curwin_mut().w_cursor;
        if oneleft() {
            start_arrow_with_change(&mut tpos, end_change, K_LEFT);
            if g_st().ins_global.revins_scol != -1
                && curwin_mut().w_cursor.col >= g_st().ins_global.revins_scol
            {
                g_st().ins_global.revins_legal += 1;
            }
            g_st().ins_global.revins_chars += 1;
        } else if !vim_strchr(opt_vars().p_ww, b'[' as c_int).is_null()
            && curwin_mut().w_cursor.lnum > 1
        {
            start_arrow(&raw mut tpos, true, K_LEFT);
            curwin_mut().w_cursor.lnum -= 1;
            coladvance(curwin_mut(), MAXCOL);
            curwin_mut().w_set_curswant = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
        g_st().ins_global.dont_sync_undo = K_FALSE;
    }
}

fn ins_home(c: c_int) {
    {
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
            fold_open_cursor();
        }
        undisplay_dollar();
        let mut tpos = curwin_mut().w_cursor;
        if c == K_C_HOME {
            curwin_mut().w_cursor.lnum = 1;
        }
        curwin_mut().w_cursor.col = 0;
        curwin_mut().w_cursor.coladd = 0;
        curwin_mut().w_curswant = 0;
        start_arrow(
            &raw mut tpos,
            true,
            if c == K_C_HOME { NUL as c_int } else { K_HOME },
        );
    }
}

fn ins_end(c: c_int) {
    {
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
            fold_open_cursor();
        }
        undisplay_dollar();
        let mut tpos = curwin_mut().w_cursor;
        if c == K_C_END {
            curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
        }
        coladvance(curwin_mut(), MAXCOL);
        curwin_mut().w_curswant = MAXCOL;
        start_arrow(
            &raw mut tpos,
            true,
            if c == K_C_END { NUL as c_int } else { K_END },
        );
    }
}

fn ins_s_left() {
    {
        let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
            fold_open_cursor();
        }
        undisplay_dollar();
        if curwin_mut().w_cursor.lnum > 1 || curwin_mut().w_cursor.col > 0 {
            start_arrow_with_change(&mut curwin_mut().w_cursor, end_change, K_S_LEFT);
            bck_word(1, false, false);
            curwin_mut().w_set_curswant = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
        g_st().ins_global.dont_sync_undo = K_FALSE;
    }
}

fn ins_right() {
    unsafe {
        let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
            fold_open_cursor();
        }
        undisplay_dollar();
        if gchar_cursor() != NUL as c_int || virtual_active(curwin_mut()) {
            start_arrow_with_change(&mut curwin_mut().w_cursor, end_change, K_RIGHT);
            curwin_mut().w_set_curswant = true;
            if virtual_active(curwin_mut()) {
                oneright();
            } else {
                curwin_mut().w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
            }

            g_st().ins_global.revins_legal += 1;
            if g_st().ins_global.revins_chars != 0 {
                g_st().ins_global.revins_chars -= 1;
            }
        } else if !vim_strchr(opt_vars().p_ww, b']' as c_int).is_null()
            && curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count
        {
            start_arrow(&raw mut (*curwin_ptr()).w_cursor, true, K_RIGHT);
            curwin_mut().w_set_curswant = true;
            curwin_mut().w_cursor.lnum += 1;
            curwin_mut().w_cursor.col = 0;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
        g_st().ins_global.dont_sync_undo = K_FALSE;
    }
}

fn ins_s_right() {
    {
        let end_change = g_st().ins_global.dont_sync_undo == K_FALSE;
        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_HOR) != 0 && g_st().key_typed {
            fold_open_cursor();
        }
        undisplay_dollar();
        if curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count
            || gchar_cursor() != NUL as c_int
        {
            start_arrow_with_change(&mut curwin_mut().w_cursor, end_change, K_S_RIGHT);
            fwd_word(1, false, false);
            curwin_mut().w_set_curswant = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
        g_st().ins_global.dont_sync_undo = K_FALSE;
    }
}

fn ins_up(startcol: bool) {
    {
        let old_topline = curwin_mut().w_topline;
        let old_topfill = curwin_mut().w_topfill;

        undisplay_dollar();
        let mut tpos = curwin_mut().w_cursor;
        if cursor_up(1, true) {
            if startcol {
                coladvance(curwin_mut(), getvcol_nolist(&mut g_st().ins_global.start));
            }
            if old_topline != curwin_mut().w_topline || old_topfill != curwin_mut().w_topfill {
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
            }
            start_arrow(
                &raw mut tpos,
                true,
                if startcol { b'k' as c_int } else { K_UP },
            );
            g_st().ins_global.can_cindent = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
    }
}

fn ins_pageup() {
    unsafe {
        undisplay_dollar();

        if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            if !(*g_st().first_tabpage).tp_next.is_null() {
                start_arrow(&raw mut (*curwin_ptr()).w_cursor, true, NUL as c_int);
                goto_tabpage(-1);
            }
            return;
        }

        let mut tpos = curwin_mut().w_cursor;
        if pagescroll(BACKWARD, 1, false) {
            start_arrow(&raw mut tpos, true, NUL as c_int);
            g_st().ins_global.can_cindent = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
    }
}

fn ins_down(startcol: bool) {
    {
        let old_topline = curwin_mut().w_topline;
        let old_topfill = curwin_mut().w_topfill;

        undisplay_dollar();
        let mut tpos = curwin_mut().w_cursor;
        if cursor_down(1, true) {
            if startcol {
                coladvance(curwin_mut(), getvcol_nolist(&mut g_st().ins_global.start));
            }
            if old_topline != curwin_mut().w_topline || old_topfill != curwin_mut().w_topfill {
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
            }
            start_arrow(
                &raw mut tpos,
                true,
                if startcol { b'j' as c_int } else { K_DOWN },
            );
            g_st().ins_global.can_cindent = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
    }
}

fn ins_pagedown() {
    unsafe {
        undisplay_dollar();

        if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            if !(*g_st().first_tabpage).tp_next.is_null() {
                start_arrow(&raw mut (*curwin_ptr()).w_cursor, true, NUL as c_int);
                goto_tabpage(0);
            }
            return;
        }

        let mut tpos = curwin_mut().w_cursor;
        if pagescroll(FORWARD, 1, false) {
            start_arrow(&raw mut tpos, true, NUL as c_int);
            g_st().ins_global.can_cindent = true;
        } else {
            vim_beep(K_OPT_BO_FLAG_CURSOR);
        }
    }
}

fn ins_tab() -> bool {
    unsafe {
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

        if curbuf_mut().b_p_et == 0
            && !(opt_vars().p_sta != 0
                && ind
                && ((tabstop_count(&curbuf_mut().b_p_vts_array) > 1)
                    || (tabstop_count(&curbuf_mut().b_p_vts_array) == 1
                        && tabstop_first(&curbuf_mut().b_p_vts_array)
                            != get_sw_value(curbuf_mut()))
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
        redo_append_str(c"\t", 1);

        let mut temp: c_int = if opt_vars().p_sta != 0 && ind {
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

        ins_char(b' ' as c_int);
        while {
            temp -= 1;
            temp > 0
        } {
            if (g_st().state_global & VREPLACE_FLAG) != 0 {
                ins_char(b' ' as c_int);
            } else {
                ins_str(c" ", 1);
                if (g_st().state_global & REPLACE_FLAG) != 0 {
                    replace_push_nul();
                }
            }
        }

        if curbuf_mut().b_p_et == 0
            && (tabstop_count(&curbuf_mut().b_p_vsts_array) > 0
                || get_sts_value() > 0
                || (opt_vars().p_sta != 0 && ind))
        {
            let mut pos: CursorPos;
            let cursor: *mut CursorPos;
            let mut saved_line_vec: Vec<c_char>;
            let mut saved_line: *mut c_char = ptr::null_mut();
            let mut ptr: *mut c_char;
            let mut vcol: Colnr = 0;
            let mut want_vcol: Colnr = 0;
            let mut change_col: c_int = -1;
            let save_list = curwin_mut().w_onebuf_opt.wo_list;

            if (g_st().state_global & VREPLACE_FLAG) != 0 {
                pos = curwin_mut().w_cursor;
                cursor = &raw mut pos;
                let line_len = get_cursor_line_len() as usize;
                saved_line_vec =
                    std::slice::from_raw_parts(get_cursor_line_ptr(), line_len + 1).to_vec();
                saved_line_vec[line_len] = NUL;
                saved_line = saved_line_vec.as_mut_ptr();
                ptr = saved_line.offset(pos.col as isize);
            } else {
                ptr = get_cursor_pos_ptr();
                cursor = &raw mut (*curwin_ptr()).w_cursor;
            }

            if vim_strchr(opt_vars().p_cpo, K_CPO_LISTWM).is_null() {
                curwin_mut().w_onebuf_opt.wo_list = 0;
            }

            let mut fpos = curwin_mut().w_cursor;
            while fpos.col > 0 && ascii_iswhite(*ptr.offset(-1) as u8 as c_int) {
                fpos.col -= 1;
                ptr = ptr.offset(-1);
            }

            if (g_st().state_global & REPLACE_FLAG) != 0
                && fpos.lnum == g_st().ins_global.start.lnum
                && fpos.col < g_st().ins_global.start.col
            {
                ptr = ptr.offset((g_st().ins_global.start.col - fpos.col) as isize);
                fpos.col = g_st().ins_global.start.col;
            }

            getvcol(curwin_mut(), &mut fpos, Some(&mut vcol), None, None, 0);
            getvcol(
                curwin_mut(),
                &mut *cursor,
                Some(&mut want_vcol),
                None,
                None,
                0,
            );

            let tab = c"\t".as_ptr().cast_mut();
            let tab_v = b'\t' as i32;
            let mut csarg = CharsizeArg::new();
            let mut cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, tab);

            while ascii_iswhite(crate::memory::cstr_first_byte(ptr) as c_int) {
                let i = win_charsize(cstype, vcol, tab, tab_v, &mut csarg, 1).width;
                if vcol + i > want_vcol {
                    break;
                }
                if *ptr != TAB as c_char {
                    *ptr = TAB as c_char;
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
                ptr = ptr.add(1);
                vcol += i;
            }

            if change_col >= 0 {
                let mut repl_off: c_int = 0;
                cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, ptr);
                while vcol < want_vcol && crate::memory::cstr_first_byte(ptr) == b' ' {
                    vcol += win_charsize(cstype, vcol, ptr, b' ' as i32, &mut csarg, 1).width;
                    ptr = ptr.add(1);
                    repl_off += 1;
                }

                if vcol > want_vcol {
                    ptr = ptr.offset(-1);
                    repl_off -= 1;
                }
                fpos.col += repl_off;

                let i = (*cursor).col - fpos.col;
                if i > 0 {
                    if (g_st().state_global & VREPLACE_FLAG) == 0 {
                        let newp_len = (curbuf_mut().b_ml.ml_line_textlen - i) as usize;
                        let col = ptr.offset_from(curbuf_mut().b_ml.ml_line_ptr) as usize;
                        let mut new_line = Vec::with_capacity(newp_len.saturating_sub(1));
                        if col > 0 {
                            new_line
                                .extend_from_slice(std::slice::from_raw_parts(ptr.sub(col), col));
                        }
                        new_line.extend_from_slice(std::slice::from_raw_parts(
                            ptr.offset(i as isize),
                            newp_len.saturating_sub(col + 1),
                        ));
                        ml_replace_dirty_line(curbuf_mut(), &new_line, true);
                        inserted_bytes(
                            fpos.lnum,
                            change_col,
                            (*cursor).col - change_col,
                            fpos.col - change_col,
                        );
                    } else {
                        let rem = crate::memory::cstr_len(ptr.offset(i as isize)) + 1;
                        ptr::copy(ptr.offset(i as isize), ptr, rem);
                    }
                    if (g_st().state_global & REPLACE_FLAG) != 0
                        && (g_st().state_global & VREPLACE_FLAG) == 0
                    {
                        for _ in 0..i {
                            replace_join(repl_off);
                        }
                    }
                }
                (*cursor).col -= i;

                if (g_st().state_global & VREPLACE_FLAG) != 0 {
                    backspace_until_column(change_col);
                    ins_bytes_len(
                        saved_line.offset(change_col as isize),
                        ((*cursor).col - change_col) as usize,
                    );
                }
            }

            curwin_mut().w_onebuf_opt.wo_list = save_list;
        }

        false
    }
}

/// Handle CR or NL in insert mode.
pub fn ins_eol(c: c_int) -> bool {
    {
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

        redo_append_str(c"\n", 1);
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
}

fn ins_digraph() -> c_int {
    let mut did_putchar = false;

    ins_st().pc_status = PC_STATUS_UNSET;
    if redrawing() && !char_avail() {
        ins_redraw(false);
        edit_putchar(b'?' as c_int, true);
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
        return NUL as c_int;
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
            redo_append_str(c"\x16", 1);
            c = digraph_get(c, cc, true);
            clear_showcmd();
            return c;
        }
    }
    clear_showcmd();
    NUL as c_int
}

/// Handle CTRL-E and CTRL-Y in Insert mode: copy char from other line.
pub fn ins_copychar(lnum: Linenr) -> c_int {
    {
        if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count {
            vim_beep(K_OPT_BO_FLAG_COPY);
            return NUL as c_int;
        }

        validate_virtcol(curwin_mut());
        let end_vcol = curwin_mut().w_virtcol;
        let line = ml_get(lnum);

        let mut csarg = CharsizeArg::new();
        let cstype = init_charsize_arg(&mut csarg, curwin_mut(), lnum, line);
        let mut ci = utf_ptr2_str_char_info(line);
        let mut vcol: c_int = 0;
        while vcol < end_vcol && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_cluster_info(ci);
            vcol += win_charsize(cstype, vcol, ci.ptr, ci.chr.value, &mut csarg, cli.cells).width;
            if vcol > end_vcol {
                break;
            }
            ci = cli.next;
        }

        let c = if ci.chr.value < 0 {
            crate::memory::cstr_first_byte(ci.ptr) as c_int
        } else {
            ci.chr.value
        };
        if c == NUL as c_int {
            vim_beep(K_OPT_BO_FLAG_COPY);
        }
        c
    }
}

fn ins_ctrl_ey(tc: c_int) -> c_int {
    {
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
            if c != NUL as c_int {
                if c < 256 && !(c as u8).is_ascii_alphanumeric() {
                    redo_append_str(c"\x16", 1);
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
}

/// Get the value that `w_virtcol` would have when `'list'` is off.
pub fn get_nolist_virtcol() -> Colnr {
    {
        if curwin_mut().w_buffer.is_null()
            || curwin_mut().buf_mut().b_ml.ml_mfp.is_none()
            || curwin_mut().w_cursor.lnum > curwin_mut().buf_mut().b_ml.ml_line_count
        {
            return 0;
        }
        if curwin_mut().w_onebuf_opt.wo_list != 0
            && vim_strchr(opt_vars().p_cpo, K_CPO_LISTWM).is_null()
        {
            return getvcol_nolist(&mut curwin_mut().w_cursor);
        }
        validate_virtcol(curwin_mut());
        curwin_mut().w_virtcol
    }
}

fn do_insert_char_pre(c: c_int) -> Option<std::ffi::CString> {
    {
        if c == CTRL_RSB_CONST || g_st().key_stuffed != 0 || !has_event(EVENT_INSERTCHARPRE) {
            return None;
        }

        let mut buf = [0 as c_char; MB_MAXBYTES + 1];
        let save_state = g_st().state_global;
        let buflen = utf_char2bytes(c, (&raw mut buf).cast()) as usize;
        buf[buflen] = NUL;

        g_st().textlock += 1;
        set_vim_var_string(VV_CHAR, (&raw const buf).cast(), buflen as isize);

        let mut res: Option<std::ffi::CString> = None;
        if ins_apply_autocmds(EVENT_INSERTCHARPRE) != 0 {
            let vchar = get_vim_var_str(VV_CHAR);
            if crate::memory::cstr_as_bytes((&raw const buf).cast())
                != crate::memory::cstr_as_bytes(vchar)
            {
                res = Some(crate::memory::cstr_to_cstring(vchar));
            }
        }

        set_vim_var_string(VV_CHAR, ptr::null(), -1);
        g_st().textlock -= 1;
        g_st().state_global = save_state;
        res
    }
}

pub fn get_can_cindent() -> bool {
    g_st().ins_global.can_cindent
}

pub fn set_can_cindent(val: bool) {
    g_st().ins_global.can_cindent = val;
}

/// Trigger `event` and take care of fixing undo.
pub fn ins_apply_autocmds(event: AuEvent) -> c_int {
    {
        let tick = buf_get_changedtick(curbuf_mut());
        let r = c_int::from(apply_autocmds(
            event,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        ));
        if event != EVENT_INSERTLEAVE && tick != buf_get_changedtick(curbuf_mut()) {
            u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1);
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_spaces_and_can_cindent_work() {
        {
            let mut line = *b"hello   \0";
            truncate_spaces(line.as_mut_ptr().cast(), 8);
            assert_eq!(crate::memory::cstr_as_bytes(line.as_ptr().cast()), b"hello");

            set_can_cindent(true);
            assert!(get_can_cindent());
            set_can_cindent(false);
            assert!(!get_can_cindent());
        }
    }
}
