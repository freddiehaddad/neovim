//! Functions for Insert mode (`src/nvim/insert.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_CURSORMOVEDI, EVENT_INSERTCHANGE, EVENT_INSERTCHARPRE, EVENT_INSERTENTER,
    EVENT_INSERTLEAVE, EVENT_INSERTLEAVEPRE, EVENT_TEXTCHANGEDI, EVENT_TEXTCHANGEDP,
    apply_autocmds, event_T, has_event,
};
use crate::buffer::{
    bt_cmdwin, bt_prompt, bt_quickfix, buf_T, buf_get_changedtick, buf_is_empty, buf_meta_total,
    bufref_T, bufref_valid, kUIMessages, set_bufref,
};
use crate::change::{
    COM_MAX_LEN, MB_MAXCHAR, OPENLINE_DO_COM, appended_lines_mark, change_warning, changed_bytes,
    del_bytes, del_char, get_leader_len, ins_bytes_len, ins_char, ins_char_bytes, ins_str,
    inserted_bytes, kCpoListwm, kOptVeFlagOnemore, open_line,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_isspace, ascii_iswhite, ascii_isxdigit, ascii2cells, char2cells,
    hex2nr, skipwhite, vim_isprintc, vim_iswordc,
};
use crate::cmdexpand::{Ctrl_N, Ctrl_P, MB_MAXBYTES};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    VALID_VIRTCOL, char_before_cursor, check_cursor, check_cursor_col, check_visual_pos,
    coladvance, colnr_T, dec_cursor, equalpos, gchar_cursor, get_cursor_line_len,
    get_cursor_line_ptr, get_cursor_pos_len, get_cursor_pos_ptr, getviscol, inc_cursor,
    kOptVeFlagAll, linenr_T, lt, pos_T,
};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::digraph::{digraph_get, do_digraph};
use crate::drawline::{VALID_WCOL, VALID_WROW};
use crate::drawscreen::{
    UPD_VALID, redraw_later, redraw_statuslines, redrawWinline, redrawing, setcursor,
    show_cursor_info_later, showmode, skip_showmode, status_redraw_curbuf, unshowmode,
    update_screen,
};
use crate::errors::*;
use crate::eval::buffer::{invoke_prompt_interrupt, prompt_invoke_callback};
use crate::eval::typval::kCallbackNone;
use crate::eval::userfunc::ascii_iswhite_nl_or_nul;
use crate::eval::vars::{VV_CHAR, VV_INSERTMODE, get_vim_var_str, set_vim_var_string};
use crate::ex_cmds::{B_IMODE_LMAP, BL_FIX, BL_SOL, BL_WHITE, Ctrl_C, Ctrl_E, Ctrl_Y};
use crate::ex_docmd::{CMOD_KEEPJUMPS, do_cmdline, do_cmdline_cmd, expr_map_locked};
use crate::ex_getln::{
    ABBR_OFF, B_IMODE_NONE, Ctrl__, Ctrl_A, Ctrl_BSL, Ctrl_D, Ctrl_F, Ctrl_G, Ctrl_HAT, Ctrl_K,
    Ctrl_L, Ctrl_O, Ctrl_Q, Ctrl_R, Ctrl_RSB, Ctrl_T, Ctrl_U, Ctrl_W, Ctrl_Z, K_EVENT, K_LUA,
    K_SELECT, cmdwin_do_action,
};
use crate::extmark::{kFalse, kNone, kTrue};
use crate::fileio::check_timestamps;
use crate::fold::{
    foldCheckClose, foldOpenCursor, foldUpdateAfterInsert, hasFolding, hasFoldingWin,
};
use crate::globals::*;
use crate::grid::{
    grid_line_flush, grid_line_getchar, grid_line_put_schar, grid_line_puts, grid_line_start,
    schar_T, schar_from_ascii,
};
use crate::highlight_group::highlight_changed;
use crate::indent::{
    INDENT_DEC, INDENT_SET, change_indent, fix_indent, get_indent, get_sts_value, get_sw_value,
    inindent, ins_try_si, may_do_si, tabstop_at, tabstop_count, tabstop_first, tabstop_padding,
    tabstop_start,
};
use crate::indent_c::{cindent_on, do_c_expr_indent, in_cinkeys};
use crate::input::{
    K_COMPLETE_DELAY, K_PASTE_START, char_avail, kInsNone, langmap_adjust, map_execute_lua,
    mb_byte2len_check, merge_modifiers, paste_repeat, plain_vgetc, prep_redo, redo_append_char,
    redo_append_lit, redo_append_str, redo_keys, start_redo_ins, stop_redo_ins, stuff_empty,
    stuffReadbuffLen, stuffRedoReadbuff, stuffcharReadbuff, vgetc, vpeekc, vungetc,
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
use crate::marktree::kMTMetaInline;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    GraphemeState, mb_adjust_cursor, mb_get_class, ptr2cells, utf_ClusterInfo, utf_char2bytes,
    utf_char2len, utf_composinglike, utf_head_off, utf_ptr2StrCharInfo, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
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
use crate::regexp::{Ctrl_H, Ctrl_V, MAXCOL};
use crate::register::{
    PUT_CURSEND, PUT_FIXINDENT, YREG_PASTE, do_put, get_expr_register, get_yank_register,
    insert_reg, is_literal_register, valid_yank_reg, yankreg_T,
};
use crate::search::{BACKWARD, FORWARD};
use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL, MODE_REPLACE, MODE_VREPLACE,
    EditorState, REPLACE_FLAG, VREPLACE_FLAG, may_trigger_modechanged, may_trigger_safestate,
    state_enter, state_handle_k_event, virtual_active,
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
use crate::window::{goto_tabpage, may_trigger_win_scrolled_resized, win_T};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint};
use std::ptr;

use crate::change::COM_MIDDLE;
const INPUT_BUFLEN: usize = 100;

pub(crate) const MOD_MASK_SHIFT: c_int = 0x02;
pub(crate) const MOD_MASK_CTRL: c_int = 0x04;
const MOD_MASK_CMD: c_int = 0x80;

pub(crate) const Ctrl_J: c_int = 10;
pub(crate) const Ctrl_S: c_int = 19;
pub(crate) const Ctrl_X: c_int = 24;

pub(crate) const kCharsizeFast: CSType = true;

#[inline]
const fn IS_SPECIAL(c: c_int) -> bool {
    c < 0
}

const kInsArrow: c_int = 1;
const kInsJump: c_int = 2;

pub(crate) const HLF_8: usize = 1;

pub(crate) const kOptFdoFlagAll: c_uint = 0x01;
pub(crate) const kOptFdoFlagHor: c_uint = 0x04;
const kOptFdoFlagInsert: c_uint = 0x100;

const kOptBoFlagBackspace: c_uint = 0x02;
const kOptBoFlagCursor: c_uint = 0x04;
const kOptBoFlagCopy: c_uint = 0x10;
const kOptBoFlagCtrlg: c_uint = 0x20;
const kOptBoFlagRegister: c_uint = 0x8000;

pub(crate) const kFoAuto: c_int = b'a' as c_int;
pub(crate) const kFoInsBlank: c_int = b'b' as c_int;
const kFoInsLong: c_int = b'l' as c_int;
const kFoRetComs: c_int = b'r' as c_int;
pub(crate) const kFoWhitePar: c_int = b'w' as c_int;

const kCpoBackspace: c_int = b'v' as c_int;
const kCpoIndent: c_int = b'I' as c_int;
const kCpoReplcnt: c_int = b'X' as c_int;

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
    old_topline: linenr_T,
    old_topfill: c_int,
    inserted_space: c_int,
    replaceState: c_int,
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
            replaceState: 0,
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
    pc_schar: schar_T,
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
pub unsafe fn linetabsize_str(s: *mut c_char) -> c_int {
    unsafe { linetabsize_col(0, s) }
}

use crate::mbyte::utf_byte2len;

#[inline]
unsafe fn trigger_autocomplete(s: *mut InsertState) -> bool {
    unsafe {
        mc_ins_cascade();
        redraw_later(*curwin.as_ptr(), UPD_VALID);
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
    unsafe {
        if ins_compl_has_autocomplete() && !char_avail() && (**curwin.as_ptr()).w_cursor.col > 0 {
            s.c = char_before_cursor();
            if vim_isprintc(s.c) {
                let _ = trigger_autocomplete(&raw mut *s);
            }
        }
    }
}

fn insert_enter(s: &mut InsertState) {
    unsafe {
        s.did_backspace = true;
        s.old_topfill = -1;
        s.replaceState = MODE_REPLACE;
        s.cmdchar_todo = s.cmdchar;
        s.ins_just_started = true;
        g_st().Ins.did_restart_edit = g_st().restart_edit;
        msg_check_for_delay(true);
        g_st().Ins.update_start_orig = true;

        ins_compl_clear();

        if s.cmdchar != b'r' as c_int && s.cmdchar != b'v' as c_int {
            let save_cursor = (**curwin.as_ptr()).w_cursor;
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

            if !equalpos((**curwin.as_ptr()).w_cursor, save_cursor)
                && *get_vim_var_str(VV_CHAR) == NUL
                && save_cursor.lnum <= (*g_st().curbuf).b_ml.ml_line_count
            {
                let save_state = g_st().State;
                (**curwin.as_ptr()).w_cursor = save_cursor;
                g_st().State = MODE_INSERT;
                check_cursor_col(&mut **curwin.as_ptr());
                g_st().State = save_state;
            }
        }

        if g_st().where_paste_started.lnum != 0 {
            g_st().Ins.start = g_st().where_paste_started;
        } else {
            g_st().Ins.start = (**curwin.as_ptr()).w_cursor;
            if s.startln != 0 {
                g_st().Ins.start.col = 0;
            }
        }

        g_st().Ins.start_textlen = linetabsize_str(get_cursor_line_ptr());
        g_st().Ins.start_blank_vcol = MAXCOL;

        if !g_st().Ins.did_ai {
            g_st().Ins.ai_col = 0;
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
            g_st().State = MODE_REPLACE;
        } else if s.cmdchar == b'V' as c_int || s.cmdchar == b'v' as c_int {
            g_st().State = MODE_VREPLACE;
            s.replaceState = MODE_VREPLACE;
            g_st().orig_line_count = (*g_st().curbuf).b_ml.ml_line_count;
            g_st().vr_lines_changed = 1;
        } else {
            g_st().State = MODE_INSERT;
        }

        may_trigger_modechanged();
        g_st().Ins.stop_insert_mode = false;

        if gchar_cursor() == TAB || buf_meta_total(&*g_st().curbuf, kMTMetaInline) > 0 {
            (**curwin.as_ptr()).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
        }

        if (*g_st().curbuf).b_p_iminsert == B_IMODE_LMAP {
            g_st().State |= MODE_LANGMAP;
        }

        setmouse();
        clear_showcmd();
        g_st().Ins.revins_on = g_st().State == MODE_INSERT && opt_vars().p_ri != 0;
        if g_st().Ins.revins_on {
            undisplay_dollar();
        }
        g_st().Ins.revins_chars = 0;
        g_st().Ins.revins_legal = 0;
        g_st().Ins.revins_scol = -1;

        if g_st().restart_edit != 0 && stuff_empty() {
            g_st().Ins.moved = if g_st().where_paste_started.lnum == 0 {
                kInsJump
            } else {
                kInsNone
            };
            g_st().restart_edit = 0;

            validate_virtcol(*curwin.as_ptr());
            update_curswant();
            if (g_st().ins_at_eol && (**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.o_lnum)
                || (**curwin.as_ptr()).w_curswant > (**curwin.as_ptr()).w_virtcol
            {
                let ptr = get_cursor_line_ptr().offset((**curwin.as_ptr()).w_cursor.col as isize);
                if *ptr != NUL {
                    if *ptr.add(1) == NUL {
                        (**curwin.as_ptr()).w_cursor.col += 1;
                    } else {
                        s.i = utfc_ptr2len(ptr);
                        if *ptr.offset(s.i as isize) == NUL {
                            (**curwin.as_ptr()).w_cursor.col += s.i;
                        }
                    }
                }
            }
            g_st().ins_at_eol = false;
        } else {
            g_st().Ins.moved = kInsNone;
        }

        g_st().need_start_insertmode = false;
        g_st().Ins.need_undo = true;
        g_st().where_paste_started.lnum = 0;
        g_st().Ins.can_cindent = true;
        if g_st().Ins.did_restart_edit == 0 {
            foldOpenCursor();
        }

        s.i = 0;
        if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
            s.i = showmode();
        }

        if g_st().Ins.did_restart_edit == 0 {
            change_warning(g_st().curbuf, if s.i == 0 { 0 } else { s.i + 1 });
        }

        ui_cursor_shape();
        do_digraph(-1);

        let redo = redo_keys(ptr::null());
        g_st().Ins.new_insert_skip = redo.as_ref().map_or(0, |s| s.as_bytes().len() as c_int);

        g_st().old_indent = 0;

        loop {
            state_enter(s);
            if ins_esc(&mut s.count, s.cmdchar, s.nomove) {
                break;
            }
        }

        ins_compl_clear_autocomplete_delay();
        if g_st().ins_at_eol {
            g_st().Ins.o_lnum = (**curwin.as_ptr()).w_cursor.lnum;
        }

        pum_check_clear();
        foldUpdateAfterInsert();
        if s.cmdchar != b'r' as c_int && s.cmdchar != b'v' as c_int && s.c != Ctrl_C {
            ins_apply_autocmds(EVENT_INSERTLEAVE);
        }
        g_st().did_cursorhold = false;

        if !char_avail() && (*g_st().curbuf).b_last_changedtick_i == buf_get_changedtick(&*g_st().curbuf) {
            (*g_st().curbuf).b_last_changedtick = buf_get_changedtick(&*g_st().curbuf);
        }
    }
}

fn insert_check(s: &mut InsertState) -> c_int {
    unsafe {
        mc_ins_cascade();

        if g_st().Ins.revins_legal == 0 {
            g_st().Ins.revins_scol = -1;
        } else {
            g_st().Ins.revins_legal = 0;
        }

        if g_st().Ins.moved != kInsNone {
            s.count = 0;
        }

        if g_st().Ins.update_start_orig {
            g_st().Ins.start_orig = g_st().Ins.start;
        }

        if !(*g_st().curbuf).terminal.is_null() && !g_st().Ins.stop_insert_mode {
            g_st().Ins.stop_insert_mode = true;
            g_st().restart_edit = b'I' as c_int;
            stuffcharReadbuff(K_NOP);
        }

        if g_st().Ins.stop_insert_mode && !ins_compl_active() {
            s.count = 0;
            return 0;
        }

        if g_st().Ins.moved == kInsNone {
            (**curwin.as_ptr()).w_set_curswant = true;
        }

        if stuff_empty() {
            g_st().did_check_timestamps = false;
            if g_st().need_check_timestamps {
                check_timestamps(0);
            }
        }

        g_st().msg_scroll = 0;

        if (opt_vars().fdo_flags & kOptFdoFlagInsert) != 0 {
            foldOpenCursor();
        }

        if !char_avail() {
            foldCheckClose();
        }

        if bt_prompt(g_st().curbuf) {
            init_prompt(s.cmdchar_todo);
            s.cmdchar_todo = NUL as c_int;
        }

        if (*g_st().curbuf).b_mod_set
            && (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0
            && (**curwin.as_ptr()).w_onebuf_opt.wo_sms == 0
            && !s.did_backspace
            && (**curwin.as_ptr()).w_topline == s.old_topline
            && (**curwin.as_ptr()).w_topfill == s.old_topfill
            && s.count <= 1
        {
            s.mincol = (**curwin.as_ptr()).w_wcol;
            validate_cursor_col(&mut **curwin.as_ptr());

            if (**curwin.as_ptr()).w_wcol
                < s.mincol
                    - tabstop_at(
                        get_nolist_virtcol(),
                        (*g_st().curbuf).b_p_ts,
                        &(*g_st().curbuf).b_p_vts_array,
                        false,
                    )
                && (**curwin.as_ptr()).w_wrow
                    == (**curwin.as_ptr()).w_view_height - 1 - (get_scrolloff_value(*curwin.as_ptr()) as c_int)
                && ((**curwin.as_ptr()).w_cursor.lnum != (**curwin.as_ptr()).w_topline || (**curwin.as_ptr()).w_topfill > 0)
            {
                if (**curwin.as_ptr()).w_topfill > 0 {
                    (**curwin.as_ptr()).w_topfill -= 1;
                } else if hasFolding(
                    *curwin.as_ptr(),
                    (**curwin.as_ptr()).w_topline,
                    ptr::null_mut(),
                    &raw mut s.old_topline,
                ) {
                    set_topline(&mut **curwin.as_ptr(), s.old_topline + 1);
                } else {
                    set_topline(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_topline + 1);
                }
            }
        }

        if s.count <= 1 {
            update_topline(*curwin.as_ptr());
        }

        s.did_backspace = false;

        if s.count <= 1 {
            validate_cursor(*curwin.as_ptr());
        }

        ins_redraw(true);

        if (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
            do_check_scrollbind(true);
        }

        if (**curwin.as_ptr()).w_onebuf_opt.wo_crb != 0 {
            do_check_cursorbind();
        }

        if s.count <= 1 {
            update_curswant();
        }
        s.old_topline = (**curwin.as_ptr()).w_topline;
        s.old_topfill = (**curwin.as_ptr()).w_topfill;

        if s.c != K_EVENT && s.c != K_COMPLETE_DELAY {
            s.lastc = s.c;
        }

        if g_st().Ins.dont_sync_undo == kNone {
            g_st().Ins.dont_sync_undo = kTrue;
        } else {
            g_st().Ins.dont_sync_undo = kFalse;
        }

        if s.ins_just_started {
            s.ins_just_started = false;
            if ins_compl_has_autocomplete() && !char_avail() && (**curwin.as_ptr()).w_cursor.col > 0 {
                s.c = char_before_cursor();
                if vim_isprintc(s.c) {
                    ins_compl_enable_autocomplete();
                    ins_compl_arm_autostart();
                    ins_compl_init_get_longest();
                    if !ins_compl_arm_autocomplete_delay() {
                        insert_do_complete(&raw mut *s);
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
        if g_st().Ins.stop_insert_mode {
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
            && (**curwin.as_ptr()).w_cursor.col >= ins_compl_col()
            && ins_compl_has_shown_match()
            && pum_wanted()
        {
            if (s.c == K_BS || s.c == Ctrl_H) && (**curwin.as_ptr()).w_cursor.col > ins_compl_col() {
                s.c = ins_compl_bs();
                if s.c == NUL as c_int {
                    return 1;
                }
            }

            if !ins_compl_used_match() {
                if s.c == Ctrl_L && (!ctrl_x_mode_line_or_eval() || ins_compl_long_shown_match())
                {
                    ins_compl_addfrommatch();
                    return 1;
                }

                let is_commit = ins_compl_commit_char(s.c);
                if !is_commit && ins_compl_accept_char(s.c) {
                    if let Some(str_) = do_insert_char_pre(s.c) {
                        let mut p = str_.as_ptr().cast_mut();
                        while *p != NUL {
                            ins_compl_addleader(utf_ptr2char(p));
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                    } else {
                        ins_compl_addleader(s.c);
                    }
                    return 1;
                }

                if (s.c == Ctrl_Y
                    || is_commit
                    || (ins_compl_enter_selects()
                        && (s.c == CAR || s.c == K_KENTER || s.c == NL)))
                    && stop_arrow() == OK
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

        if s.c == Ctrl_BSL {
            ins_redraw(false);
            g_st().no_mapping += 1;
            g_st().allow_keys += 1;
            s.c = plain_vgetc();
            g_st().no_mapping -= 1;
            g_st().allow_keys -= 1;
            if s.c != Ctrl_N && s.c != Ctrl_G && s.c != Ctrl_O {
                vungetc(s.c);
                s.c = Ctrl_BSL;
            } else {
                if s.c == Ctrl_O {
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

        if (s.c == Ctrl_V || s.c == Ctrl_Q) && ctrl_x_mode_cmdline() {
            insert_do_complete(&raw mut *s);
            insert_handle_key_post(s);
            return 1;
        }

        if s.c == Ctrl_V || s.c == Ctrl_Q {
            ins_ctrl_v();
            s.c = Ctrl_V;
            return 1;
        }

        if cindent_on() && ctrl_x_mode_none() {
            s.line_is_white = inindent(0);
            if in_cinkeys(s.c, b'!' as c_int, s.line_is_white) && stop_arrow() == OK {
                do_c_expr_indent();
                return 1;
            }
            if g_st().Ins.can_cindent
                && in_cinkeys(s.c, b'*' as c_int, s.line_is_white)
                && stop_arrow() == OK
            {
                do_c_expr_indent();
            }
        }

        if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0 {
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
            Ctrl_C => {
                if bt_cmdwin(g_st().curbuf) {
                    g_st().got_int = false;
                    cmdwin_do_action(c"cancel".as_ptr());
                    return 0;
                }
                if bt_prompt(g_st().curbuf) && invoke_prompt_interrupt() {
                    if !bt_prompt(g_st().curbuf) {
                        return 0;
                    }
                } else {
                    return 0;
                }
            }
            Ctrl_Z => {
                goto_normalchar = true;
            }
            Ctrl_O => {
                if ctrl_x_mode_omni() {
                    insert_do_complete(&raw mut *s);
                } else if !echeck_abbr(Ctrl_O + ABBR_OFF) {
                    ins_ctrl_o();
                    if (get_ve_flags(*curwin.as_ptr()) & kOptVeFlagOnemore) != 0 {
                        g_st().ins_at_eol = false;
                        s.nomove = true;
                    }
                    s.count = 0;
                    return 0;
                }
            }
            K_INS | K_KINS => {
                ins_insert(s.replaceState);
            }
            K_SELECT | K_IGNORE => {}
            K_HELP | K_F1 | K_XF1 => {
                stuffcharReadbuff(K_HELP);
                return 0;
            }
            c if c == b' ' as c_int && g_st().mod_mask != MOD_MASK_CTRL => {
                goto_normalchar = true;
            }
            c if c == b' ' as c_int || c == K_ZERO || c == NUL as c_int || c == Ctrl_A => {
                if stuff_inserted(NUL as c_int, 1, c_int::from(s.c == Ctrl_A)) == FAIL
                    && s.c != Ctrl_A
                {
                    return 0;
                }
                s.inserted_space = 0;
            }
            Ctrl_R => {
                if ctrl_x_mode_register() && !ins_compl_active() {
                    insert_do_complete(&raw mut *s);
                } else {
                    ins_reg();
                    auto_format(false, true);
                    s.inserted_space = 0;
                }
            }
            Ctrl_G => {
                ins_ctrl_g();
            }
            Ctrl_HAT => {
                ins_ctrl_hat();
            }
            Ctrl__ => {
                if opt_vars().p_ari == 0 {
                    goto_normalchar = true;
                } else {
                    ins_ctrl_();
                }
            }
            Ctrl_D => {
                if ctrl_x_mode_path_defines() {
                    insert_do_complete(&raw mut *s);
                } else {
                    ins_shift(s.c, s.lastc);
                    auto_format(false, true);
                    s.inserted_space = 0;
                }
            }
            Ctrl_T => {
                if ctrl_x_mode_thesaurus() {
                    if check_compl_option(false) {
                        insert_do_complete(&raw mut *s);
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
            K_BS | Ctrl_H => {
                s.did_backspace = ins_bs(s.c, BACKSPACE_CHAR, &raw mut s.inserted_space);
                auto_format(false, true);
                if s.did_backspace {
                    may_trigger_autocomplete(s);
                }
            }
            Ctrl_W => {
                if bt_prompt(g_st().curbuf) && (g_st().mod_mask & MOD_MASK_SHIFT) == 0 {
                    stuffcharReadbuff(Ctrl_W);
                    g_st().restart_edit = b'A' as c_int;
                    s.nomove = true;
                    s.count = 0;
                    return 0;
                }
                s.did_backspace = ins_bs(s.c, BACKSPACE_WORD, &raw mut s.inserted_space);
                auto_format(false, true);
                if s.did_backspace {
                    may_trigger_autocomplete(s);
                }
            }
            Ctrl_U => {
                if ctrl_x_mode_function() {
                    insert_do_complete(&raw mut *s);
                } else {
                    s.did_backspace =
                        ins_bs(s.c, BACKSPACE_LINE, &raw mut s.inserted_space);
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
                let mut save_curbuf = bufref_T {
                    br_buf: ptr::null_mut(),
                    br_fnum: 0,
                    br_buf_free_count: 0,
                };
                let tick = buf_get_changedtick(&*g_st().curbuf);
                set_bufref(&mut save_curbuf, g_st().curbuf);

                if s.c == K_PASTE_START {
                    paste_repeat(1);
                } else if s.c == K_EVENT {
                    state_handle_k_event();
                    if g_st().Ins.dont_sync_undo == kTrue {
                        g_st().Ins.dont_sync_undo = kNone;
                    }
                } else if s.c == K_COMMAND {
                    do_cmdline(ptr::null_mut(), crate::cmdexpand::LineGetter::CmdKeyCmd, 0);
                } else {
                    map_execute_lua(false, false);
                }

                if g_st().pum_want.active {
                    if pum_visible() {
                        g_st().edit_submode_extra = ptr::null_mut();
                        insert_do_complete(&raw mut *s);
                        if g_st().pum_want.finish {
                            ins_compl_prep(Ctrl_Y);
                        }
                    }
                    g_st().pum_want.active = false;
                }

                let changed = bufref_valid(&raw mut save_curbuf)
                    && g_st().curbuf == save_curbuf.br_buf
                    && tick != buf_get_changedtick(&*g_st().curbuf);
                if (*g_st().curbuf).b_u_synced || changed {
                    g_st().Ins.need_undo = true;
                }
                if changed && !ins_compl_active() {
                    mc_ins_cascade();
                    mc_ins_cascade_restart();
                }
            }
            K_COMPLETE_DELAY => {
                if g_st().Ins.dont_sync_undo == kTrue {
                    g_st().Ins.dont_sync_undo = kNone;
                }
                ins_compl_clear_autocomplete_delay();
                if ins_compl_has_autocomplete() && !char_avail() && (**curwin.as_ptr()).w_cursor.col != 0 {
                    s.c = char_before_cursor();
                    if vim_isprintc(s.c) {
                        ins_compl_enable_autocomplete();
                        ins_compl_arm_autostart();
                        insert_do_complete(&raw mut *s);
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
                    insert_do_complete(&raw mut *s);
                } else if (g_st().mod_mask & MOD_MASK_SHIFT) != 0 {
                    ins_pageup();
                } else {
                    ins_up(false);
                }
            }
            K_S_UP | K_PAGEUP | K_KPAGEUP => {
                if pum_visible() {
                    insert_do_complete(&raw mut *s);
                } else {
                    ins_pageup();
                }
            }
            K_DOWN => {
                if pum_visible() {
                    insert_do_complete(&raw mut *s);
                } else if (g_st().mod_mask & MOD_MASK_SHIFT) != 0 {
                    ins_pagedown();
                } else {
                    ins_down(false);
                }
            }
            K_S_DOWN | K_PAGEDOWN | K_KPAGEDOWN => {
                if pum_visible() {
                    insert_do_complete(&raw mut *s);
                } else {
                    ins_pagedown();
                }
            }
            K_S_TAB | TAB => {
                if s.c == K_S_TAB {
                    s.c = TAB;
                }
                if ctrl_x_mode_path_patterns() {
                    insert_do_complete(&raw mut *s);
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
                if bt_quickfix(g_st().curbuf) && s.c == CAR {
                    if (**curwin.as_ptr()).w_llist_ref.is_null() {
                        do_cmdline_cmd(c".cc".as_ptr());
                    } else {
                        do_cmdline_cmd(c".ll".as_ptr());
                    }
                } else if s.c == CAR && bt_cmdwin(g_st().curbuf) {
                    cmdwin_do_action(c"confirm".as_ptr());
                    return 0;
                } else if (g_st().mod_mask & MOD_MASK_SHIFT) == 0 && bt_prompt(g_st().curbuf) {
                    prompt_invoke_callback();
                    if !bt_prompt(g_st().curbuf) {
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
            Ctrl_K => {
                if ctrl_x_mode_dictionary() {
                    if check_compl_option(true) {
                        insert_do_complete(&raw mut *s);
                    }
                } else {
                    s.c = ins_digraph();
                    if s.c != NUL as c_int {
                        goto_normalchar = true;
                    }
                }
            }
            Ctrl_X => {
                ins_ctrl_x();
            }
            Ctrl_RSB => {
                if !ctrl_x_mode_tags() {
                    goto_normalchar = true;
                } else {
                    insert_do_complete(&raw mut *s);
                }
            }
            Ctrl_F => {
                if !ctrl_x_mode_files() {
                    goto_normalchar = true;
                } else {
                    insert_do_complete(&raw mut *s);
                }
            }
            c if c == b's' as c_int || c == Ctrl_S => {
                if !ctrl_x_mode_spell() {
                    goto_normalchar = true;
                } else {
                    insert_do_complete(&raw mut *s);
                }
            }
            Ctrl_L => {
                if !ctrl_x_mode_whole_line() {
                    goto_normalchar = true;
                } else {
                    goto_do_complete = true;
                }
            }
            Ctrl_P | Ctrl_N => {
                goto_do_complete = true;
            }
            Ctrl_Y | Ctrl_E => {
                s.c = ins_ctrl_ey(s.c);
            }
            _ => {
                goto_normalchar = true;
            }
        }

        if goto_do_complete {
            if *(*g_st().curbuf).b_p_cpt == NUL
                && (ctrl_x_mode_normal() || ctrl_x_mode_whole_line())
                && !compl_status_local()
            {
                goto_normalchar = true;
            } else {
                insert_do_complete(&raw mut *s);
            }
        }

        if goto_normalchar {
            let mut skip_char = false;
            if opt_vars().p_paste == 0 {
                if let Some(str_) = do_insert_char_pre(s.c) {
                    let str_ptr = str_.as_ptr().cast_mut();
                    if *str_ptr != NUL && stop_arrow() != FAIL {
                        let mut p = str_ptr;
                        while *p != NUL {
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
                        g_st().Ins.can_cindent = false;
                    }
                    if g_st().Ins.start_blank_vcol == MAXCOL && (**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.start.lnum {
                        g_st().Ins.start_blank_vcol = get_nolist_virtcol();
                    }
                }

                if vim_iswordc(s.c)
                    || (!echeck_abbr(if s.c >= 0x100 {
                        s.c + ABBR_OFF
                    } else {
                        s.c
                    }) && s.c != Ctrl_RSB)
                {
                    insert_special(s.c, 0, 0);
                    g_st().Ins.revins_legal += 1;
                    g_st().Ins.revins_chars += 1;
                }

                auto_format(false, true);
                foldOpenCursor();
                if ins_compl_has_autocomplete() && !char_avail() && vim_isprintc(s.c) {
                    let _ = trigger_autocomplete(&raw mut *s);
                }
            }
        }

        insert_handle_key_post(s);
        1
    }
}

unsafe fn insert_do_complete(s: *mut InsertState) {
    unsafe {
        ins_compl_clear_autocomplete_delay();
        ins_st().compl_busy = true;
        g_st().disable_fold_update += 1;
        if ins_complete((*s).c, true) == FAIL {
            compl_status_clear();
        }
        g_st().disable_fold_update -= 1;
        ins_st().compl_busy = false;
        g_st().Ins.can_si = may_do_si();
    }
}

fn insert_do_cindent(s: &mut InsertState) {
    if in_cinkeys(s.c, b' ' as c_int, s.line_is_white) && stop_arrow() == OK {
            do_c_expr_indent();
        }
}

fn insert_handle_key_post(s: &mut InsertState) {
    unsafe {
        if s.c != K_EVENT && s.c != K_COMPLETE_DELAY && ctrl_x_mode_normal() {
            g_st().did_cursorhold = false;
        }

        if ins_compl_active() && !ins_compl_win_active(&mut **curwin.as_ptr()) {
            ins_compl_cancel();
        }

        if g_st().Ins.moved != kInsNone {
            s.inserted_space = 0;
        }

        if g_st().Ins.can_cindent && cindent_on() && ctrl_x_mode_normal() {
            insert_do_cindent(s);
        }
    }
}

/// Start inserting text.
pub fn edit(cmdchar: c_int, startln: bool, count: c_int) -> bool {
    unsafe {
        if !(*g_st().curbuf).terminal.is_null() {
            if g_st().ex_normal_busy != 0 {
                g_st().restart_edit = b'i' as c_int;
                g_st().force_restart_edit = true;
                return false;
            }
            return terminal_enter();
        }

        if g_st().sandbox != 0 {
            emsg(gettext(e_sandbox.as_ptr()));
            return false;
        }

        if g_st().textlock != 0 || ins_compl_active() || ins_st().compl_busy || pum_visible() || expr_map_locked() {
            emsg(gettext(e_textlock.as_ptr()));
            return false;
        }

        let mut s = InsertState::new();
        s.cmdchar = cmdchar;
        s.startln = c_int::from(startln);
        s.count = count;
        insert_enter(&mut s);
        s.c == Ctrl_O
    }
}

/// Redraw for Insert mode.
pub fn ins_redraw(ready: bool) {
    unsafe {
        if char_avail() {
            return;
        }

        if ready
            && has_event(EVENT_CURSORMOVEDI)
            && (g_st().last_cursormoved_win != *curwin.as_ptr() || !equalpos(g_st().last_cursormoved, (**curwin.as_ptr()).w_cursor))
            && !pum_visible()
        {
            if syntax_present(*curwin.as_ptr()) && g_st().must_redraw != 0 {
                update_screen();
            }
            update_curswant();
            ins_apply_autocmds(EVENT_CURSORMOVEDI);
            g_st().last_cursormoved_win = *curwin.as_ptr();
            g_st().last_cursormoved = (**curwin.as_ptr()).w_cursor;
        }

        if ready
            && has_event(EVENT_TEXTCHANGEDI)
            && (*g_st().curbuf).b_last_changedtick_i != buf_get_changedtick(&*g_st().curbuf)
            && !pum_visible()
        {
            let mut aco = CtxSwitch::new();
            let tick = buf_get_changedtick(&*g_st().curbuf);
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), g_st().curbuf, 0);
            apply_autocmds(
                EVENT_TEXTCHANGEDI,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            ctx_restore(&mut aco);
            (*g_st().curbuf).b_last_changedtick_i = buf_get_changedtick(&*g_st().curbuf);
            if tick != buf_get_changedtick(&*g_st().curbuf) {
                u_save((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.lnum + 1);
            }
        }

        if ready
            && has_event(EVENT_TEXTCHANGEDP)
            && (*g_st().curbuf).b_last_changedtick_pum != buf_get_changedtick(&*g_st().curbuf)
            && pum_visible()
        {
            let mut aco = CtxSwitch::new();
            let tick = buf_get_changedtick(&*g_st().curbuf);
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), g_st().curbuf, 0);
            apply_autocmds(
                EVENT_TEXTCHANGEDP,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            ctx_restore(&mut aco);
            (*g_st().curbuf).b_last_changedtick_pum = buf_get_changedtick(&*g_st().curbuf);
            if tick != buf_get_changedtick(&*g_st().curbuf) {
                u_save((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.lnum + 1);
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
    unsafe {
        let mut did_putchar = false;

        ins_redraw(false);

        if redrawing() && !char_avail() {
            edit_putchar(b'^' as c_int, true);
            did_putchar = true;
        }
        redo_append_str(c"\x16".as_ptr(), 1);

        add_to_showcmd_c(Ctrl_V);

        let c = get_literal((g_st().mod_mask & MOD_MASK_SHIFT) != 0);
        if did_putchar {
            edit_unputchar();
        }
        clear_showcmd();
        insert_special(c, 1, 1);
        g_st().Ins.revins_chars += 1;
        g_st().Ins.revins_legal += 1;
    }
}

pub fn edit_putchar(c: c_int, highlight: bool) {
    unsafe {
        if (**curwin.as_ptr()).w_grid_alloc.chars.is_empty()
            && !crate::grid::grid_allocated(&g_st().default_grid)
        {
            return;
        }

        update_topline(*curwin.as_ptr());
        validate_cursor(*curwin.as_ptr());
        let attr = if highlight {
            crate::highlight::hl_attr(HLF_8 as c_int)
        } else {
            0
        };
        ins_st().pc_row = (**curwin.as_ptr()).w_wrow;
        ins_st().pc_status = PC_STATUS_UNSET;
        grid_line_start(&raw mut (**curwin.as_ptr()).w_grid, ins_st().pc_row);
        if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0 {
            ins_st().pc_col = (**curwin.as_ptr()).w_view_width - 1 - (**curwin.as_ptr()).w_wcol;
            if grid_line_getchar(ins_st().pc_col, ptr::null_mut()) == 0 {
                grid_line_put_schar(ins_st().pc_col - 1, schar_from_ascii(b' '), attr);
                (**curwin.as_ptr()).w_wcol -= 1;
                ins_st().pc_status = PC_STATUS_RIGHT;
            }
        } else {
            ins_st().pc_col = (**curwin.as_ptr()).w_wcol;
            if grid_line_getchar(ins_st().pc_col + 1, ptr::null_mut()) == 0 {
                ins_st().pc_status = PC_STATUS_LEFT;
            }
        }

        if ins_st().pc_status == PC_STATUS_UNSET {
            ins_st().pc_schar = grid_line_getchar(ins_st().pc_col, &raw mut ins_st().pc_attr);
            ins_st().pc_status = PC_STATUS_SET;
        }

        let mut buf = [0 as c_char; MB_MAXCHAR + 1];
        let len = utf_char2bytes(c, (&raw mut buf).cast());
        grid_line_puts(ins_st().pc_col, (&raw const buf).cast(), len, attr);
        grid_line_flush();
    }
}

/// Return the effective prompt for the specified buffer.
pub fn buf_prompt_text(buf: &buf_T) -> *mut c_char {
    buf
            .b_prompt_text
            .as_ref()
            .map_or(c"% ".as_ptr().cast_mut(), |s| s.as_ptr().cast_mut())
}

/// Return the effective prompt for the current buffer.
pub fn prompt_text() -> *mut c_char {
    unsafe { buf_prompt_text(&*g_st().curbuf) }
}

fn init_prompt(cmdchar_todo: c_int) {
    unsafe {
        let prompt = prompt_text();
        let prompt_len = std::ffi::CStr::from_ptr(prompt).to_bytes().len() as colnr_T;

        if (*g_st().curbuf).b_prompt_start.mark.lnum < 1
            || (*g_st().curbuf).b_prompt_start.mark.lnum > (*g_st().curbuf).b_ml.ml_line_count
        {
            (*g_st().curbuf).b_prompt_start.mark.lnum = (*g_st().curbuf)
                .b_prompt_start
                .mark
                .lnum
                .min((*g_st().curbuf).b_ml.ml_line_count)
                .max(1);
            (*g_st().curbuf).b_prompt_append_new_line = true;
        }

        (**curwin.as_ptr()).w_cursor.lnum = (**curwin.as_ptr())
            .w_cursor
            .lnum
            .max((*g_st().curbuf).b_prompt_start.mark.lnum);
        let text = ml_get((*g_st().curbuf).b_prompt_start.mark.lnum);
        let text_len = ml_get_len((*g_st().curbuf).b_prompt_start.mark.lnum);

        if (*g_st().curbuf).b_prompt_start.mark.lnum == (**curwin.as_ptr()).w_cursor.lnum
            && ((*g_st().curbuf).b_prompt_start.mark.col < prompt_len
                || (*g_st().curbuf).b_prompt_start.mark.col > text_len
                || std::slice::from_raw_parts(
                    text.offset(((*g_st().curbuf).b_prompt_start.mark.col - prompt_len) as isize).cast::<u8>(),
                    prompt_len as usize,
                ) != std::slice::from_raw_parts(prompt.cast::<u8>(), prompt_len as usize))
        {
            if *text == NUL {
                ml_replace((*g_st().curbuf).b_prompt_start.mark.lnum, prompt, true);
                inserted_bytes((*g_st().curbuf).b_prompt_start.mark.lnum, 0, 0, prompt_len);
            } else {
                let lnum = (*g_st().curbuf).b_ml.ml_line_count;
                ml_append(lnum, prompt, 0, false);
                appended_lines_mark(lnum, 1);
                (*g_st().curbuf).b_prompt_start.mark.lnum = (*g_st().curbuf).b_ml.ml_line_count;
                (*g_st().curbuf).b_prompt_append_new_line = true;
                u_clearallandblockfree(&mut *g_st().curbuf);
            }
            (*g_st().curbuf).b_prompt_start.mark.col = prompt_len;
            (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_ml.ml_line_count;
            coladvance(&mut **curwin.as_ptr(), MAXCOL);
        }

        if g_st().Ins.start_orig.lnum != (*g_st().curbuf).b_prompt_start.mark.lnum
            || g_st().Ins.start_orig.col != (*g_st().curbuf).b_prompt_start.mark.col
        {
            set_insstart(
                (*g_st().curbuf).b_prompt_start.mark.lnum,
                (*g_st().curbuf).b_prompt_start.mark.col,
            );
        }

        if cmdchar_todo == b'A' as c_int {
            coladvance(&mut **curwin.as_ptr(), MAXCOL);
        }
        if (*g_st().curbuf).b_prompt_start.mark.lnum == (**curwin.as_ptr()).w_cursor.lnum {
            (**curwin.as_ptr()).w_cursor.col = (**curwin.as_ptr())
                .w_cursor
                .col
                .max((*g_st().curbuf).b_prompt_start.mark.col);
        }
        check_cursor(*curwin.as_ptr());
    }
}

/// Return true if the cursor is in the editable position of the prompt line.
pub fn prompt_curpos_editable() -> bool {
    unsafe {
        (**curwin.as_ptr()).w_cursor.lnum > (*g_st().curbuf).b_prompt_start.mark.lnum
            || ((**curwin.as_ptr()).w_cursor.lnum == (*g_st().curbuf).b_prompt_start.mark.lnum
                && (**curwin.as_ptr()).w_cursor.col >= (*g_st().curbuf).b_prompt_start.mark.col)
    }
}

fn set_insstart(lnum: linenr_T, col: colnr_T) {
    g_st().Ins.start.lnum = lnum;
        g_st().Ins.start.col = col;
        g_st().Ins.start_orig = g_st().Ins.start;
        g_st().Ins.start_textlen = g_st().Ins.start.col;
        g_st().Ins.start_blank_vcol = MAXCOL;
        g_st().Ins.moved = kInsNone;
}

pub fn edit_unputchar() {
    unsafe {
        if ins_st().pc_status != PC_STATUS_UNSET && ins_st().pc_row >= 0 {
            if ins_st().pc_status == PC_STATUS_RIGHT {
                (**curwin.as_ptr()).w_wcol += 1;
            }
            if ins_st().pc_status == PC_STATUS_RIGHT || ins_st().pc_status == PC_STATUS_LEFT {
                redrawWinline(*curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
            } else {
                grid_line_start(&raw mut (**curwin.as_ptr()).w_grid, ins_st().pc_row);
                grid_line_put_schar(ins_st().pc_col, ins_st().pc_schar, ins_st().pc_attr);
                grid_line_flush();
            }
        }
    }
}

/// Display a `$` at the end of changed text when `$` is in `cpoptions`.
pub fn display_dollar(col_arg: colnr_T) {
    unsafe {
        let col = col_arg.max(0);
        if !redrawing() {
            return;
        }
        let save_col = (**curwin.as_ptr()).w_cursor.col;
        (**curwin.as_ptr()).w_cursor.col = col;

        let p = get_cursor_line_ptr();
        (**curwin.as_ptr()).w_cursor.col -= utf_head_off(p, p.offset(col as isize));
        curs_columns(*curwin.as_ptr(), 0);
        if (**curwin.as_ptr()).w_wcol < (**curwin.as_ptr()).w_view_width {
            edit_putchar(b'$' as c_int, false);
            g_st().dollar_vcol = (**curwin.as_ptr()).w_virtcol;
        }
        (**curwin.as_ptr()).w_cursor.col = save_col;
    }
}

pub fn undisplay_dollar() {
    unsafe {
        if g_st().dollar_vcol < 0 {
            return;
        }
        g_st().dollar_vcol = -1;
        redrawWinline(*curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
    }
}

/// Truncate the space at the end of a line.
pub unsafe fn truncate_spaces(line: *mut c_char, len: usize) {
    unsafe {
        let mut i = len as isize - 1;
        while i >= 0 && ascii_iswhite(*line.offset(i) as u8 as c_int) {
            if (g_st().State & REPLACE_FLAG) != 0 {
                replace_join(0);
            }
            i -= 1;
        }
        *line.offset(i + 1) = NUL;
    }
}

/// Backspace the cursor until the given column.
pub fn backspace_until_column(col: c_int) {
    unsafe {
        while (**curwin.as_ptr()).w_cursor.col > col {
            (**curwin.as_ptr()).w_cursor.col -= 1;
            if (g_st().State & REPLACE_FLAG) != 0 {
                replace_do_bs(col);
            } else if !del_char_after_col(col) {
                break;
            }
        }
    }
}

fn del_char_after_col(limit_col: c_int) -> bool {
    unsafe {
        if limit_col >= 0 {
            let ecol = (**curwin.as_ptr()).w_cursor.col + 1;
            mb_adjust_cursor();
            while (**curwin.as_ptr()).w_cursor.col < limit_col {
                let l = utf_ptr2len(get_cursor_pos_ptr());
                if l == 0 {
                    break;
                }
                (**curwin.as_ptr()).w_cursor.col += l;
            }
            if *get_cursor_pos_ptr() == NUL || (**curwin.as_ptr()).w_cursor.col == ecol {
                return false;
            }
            del_bytes(ecol - (**curwin.as_ptr()).w_cursor.col, false, true);
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
            return Ctrl_C;
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
            if (g_st().State & MODE_CMDLINE) == 0 && mb_byte2len_check(nc) == 1 {
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
        if IS_SPECIAL(c) || (g_st().mod_mask != 0 && allow_modmask != 0) {
            let p = get_special_key_name(c, g_st().mod_mask);
            let len = std::ffi::CStr::from_ptr(p).to_bytes().len();
            c = *p.add(len - 1) as u8 as c_int;
            if len > 2 {
                if stop_arrow() == FAIL {
                    return;
                }
                *p.add(len - 1) = NUL;
                ins_str(p, len - 1);
                redo_append_lit(p, -1);
                ctrlv = 0;
            }
        }
        if stop_arrow() == OK {
            insertchar(c, if ctrlv != 0 { INSCHAR_CTRLV } else { 0 }, -1);
        }
    }
}

pub fn insertchar(mut c: c_int, flags: c_int, second_indent: c_int) {
    unsafe {
        let force_format = (flags & INSCHAR_FORMAT) != 0;
        let textwidth = comp_textwidth(force_format);
        let fo_ins_blank = has_format_option(kFoInsBlank);

        if textwidth > 0
            && (force_format
                || (!ascii_iswhite(c)
                    && !((g_st().State & REPLACE_FLAG) != 0
                        && (g_st().State & VREPLACE_FLAG) == 0
                        && *get_cursor_pos_ptr() != NUL)
                    && ((**curwin.as_ptr()).w_cursor.lnum != g_st().Ins.start.lnum
                        || ((!has_format_option(kFoInsLong) || g_st().Ins.start_textlen <= textwidth)
                            && (!fo_ins_blank || g_st().Ins.start_blank_vcol <= textwidth)))))
        {
            let mut do_internal = true;
            let virtcol = get_nolist_virtcol()
                + char2cells(if c != NUL as c_int { c } else { gchar_cursor() });

            if (*g_st().curbuf).b_p_fex.cb_type() != kCallbackNone
                && (flags & INSCHAR_NO_FEX) == 0
                && (force_format || virtcol > textwidth)
            {
                do_internal = fex_format((**curwin.as_ptr()).w_cursor.lnum, 1, c) != 0;
                g_st().Ins.need_undo = true;
            }
            if do_internal {
                internal_format(textwidth, second_indent, flags, c == NUL as c_int, c);
            }
        }

        if c == NUL as c_int {
            return;
        }

        if g_st().Ins.did_ai && c == g_st().Ins.end_comment_pending {
            let mut lead_end = [0 as c_char; COM_MAX_LEN];
            let mut p: *mut c_char = ptr::null_mut();
            let line = get_cursor_line_ptr();
            let mut i = get_leader_len(line, &raw mut p, false, true);
            if i > 0 && !vim_strchr(p, c_int::from(COM_MIDDLE as u8)).is_null() {
                while *p != NUL && *p.offset(-1) != b':' as c_char {
                    p = p.add(1);
                }
                let mut middle_len = copy_option_part(
                    &raw mut p,
                    (&raw mut lead_end).cast(),
                    COM_MAX_LEN,
                    c",".as_ptr(),
                ) as c_int;
                while middle_len > 0
                    && ascii_iswhite(lead_end[(middle_len - 1) as usize] as u8 as c_int)
                {
                    middle_len -= 1;
                }

                while *p != NUL && *p.offset(-1) != b':' as c_char {
                    p = p.add(1);
                }
                let end_len = copy_option_part(
                    &raw mut p,
                    (&raw mut lead_end).cast(),
                    COM_MAX_LEN,
                    c",".as_ptr(),
                ) as c_int;

                i = (**curwin.as_ptr()).w_cursor.col;
                while {
                    i -= 1;
                    i >= 0 && ascii_iswhite(*line.add(i as usize) as u8 as c_int)
                } {}
                i += 1;
                i -= middle_len;

                if i >= 0
                    && end_len > 0
                    && (lead_end[(end_len - 1) as usize] as u8 as c_int) == g_st().Ins.end_comment_pending
                {
                    backspace_until_column(i);
                    ins_bytes_len((&raw mut lead_end).cast(), (end_len - 1) as usize);
                }
            }
        }
        g_st().Ins.end_comment_pending = NUL as c_int;

        g_st().Ins.did_ai = false;
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;

        if !is_special_ins(c)
            && utf_char2len(c) == 1
            && !has_event(EVENT_INSERTCHARPRE)
            && !*test_disable_char_avail.as_ptr()
            && vpeekc() != NUL as c_int
            && (g_st().State & REPLACE_FLAG) == 0
            && !cindent_on()
            && opt_vars().p_ri == 0
        {
            let mut buf = [0 as c_char; INPUT_BUFLEN + 1];
            let mut virtcol: colnr_T = 0;

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
                    && !(!g_st().no_abbr && !vim_iswordc(c) && vim_iswordc(buf[i - 1] as u8 as c_int))
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
    unsafe {
        if ascii_isdigit(c) {
            let mut buf = [0 as c_char; 10];
            vim_snprintf!(
                (&raw mut buf).cast(),
                size_of_val(&buf),
                c"%03d".as_ptr(),
                c,
            );
            redo_append_str((&raw const buf).cast(), -1);
        } else {
            redo_append_char(c);
        }
    }
}

/// Called when the cursor moves in insert-mode without editing text.
pub unsafe fn start_arrow(end_insert_pos: *mut pos_T, end_change: bool, move_key: c_int) {
    unsafe {
        if g_st().Ins.moved == kInsNone && end_change {
            stop_insert(end_insert_pos, 0, 0);
        }
        if g_st().Ins.moved != kInsJump {
            if move_key == NUL as c_int {
                redo_append_str(c"\x1b".as_ptr(), 1);
                g_st().Ins.moved = kInsJump;
            } else {
                if !IS_SPECIAL(move_key) {
                    redo_append_char(Ctrl_G);
                }
                redo_append_char(move_key);
                if end_change {
                    g_st().Ins.moved = kInsArrow;
                }
            }
        }
        check_spell_redraw();
    }
}

fn start_arrow_with_change(end_insert_pos: &mut pos_T, end_change: bool, move_key: c_int) {
    unsafe {
        if !end_change && g_st().Ins.moved != kInsJump {
            redo_append_char(Ctrl_G);
            redo_append_char(b'U' as c_int);
        }
        start_arrow(&raw mut *end_insert_pos, end_change, move_key);
    }
}

fn check_spell_redraw() {
    unsafe {
        if g_st().spell_redraw_lnum != 0 {
            let lnum = g_st().spell_redraw_lnum;
            g_st().spell_redraw_lnum = 0;
            redrawWinline(*curwin.as_ptr(), lnum);
        }
    }
}

pub fn stop_arrow() -> c_int {
    unsafe {
        if g_st().Ins.moved != kInsNone {
            g_st().Ins.start = (**curwin.as_ptr()).w_cursor;
            if g_st().Ins.start.col > g_st().Ins.start_orig.col && !g_st().Ins.need_undo {
                g_st().Ins.update_start_orig = false;
            }
            g_st().Ins.start_textlen = linetabsize_str(get_cursor_line_ptr());

            let jumped = g_st().Ins.moved == kInsJump;
            if u_save_cursor() == OK {
                g_st().Ins.moved = kInsNone;
                g_st().Ins.need_undo = false;
            }
            g_st().Ins.ai_col = 0;
            if (g_st().State & VREPLACE_FLAG) != 0 {
                g_st().orig_line_count = (*g_st().curbuf).b_ml.ml_line_count;
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
                g_st().Ins.new_insert_skip = 2;
                mc_ins_cascade_restart();
            } else {
                let redo = redo_keys(ptr::null());
                g_st().Ins.new_insert_skip = redo.as_ref().map_or(0, |s| s.as_bytes().len() as c_int);
            }
        } else if g_st().Ins.need_undo && u_save_cursor() == OK {
            if lt((**curwin.as_ptr()).w_cursor, g_st().Ins.start) {
                g_st().Ins.start = (**curwin.as_ptr()).w_cursor;
                g_st().Ins.start_textlen = linetabsize_str(get_cursor_line_ptr());
            }
            g_st().Ins.need_undo = false;
        }

        foldOpenCursor();

        if g_st().Ins.moved != kInsNone || g_st().Ins.need_undo {
            FAIL
        } else {
            OK
        }
    }
}

unsafe fn stop_insert(end_insert_pos: *mut pos_T, esc: c_int, nomove: c_int) {
    unsafe {
        stop_redo_ins();
        ins_st().replace_stack = Vec::new();

        let redo_cstr = redo_keys(ptr::null());
        let added = match redo_cstr {
            None => 0,
            Some(ref s) => (s.as_bytes().len() as c_int) - g_st().Ins.new_insert_skip,
        };
        if g_st().Ins.did_restart_edit == 0 || added > 0 {
            ins_st().last_insert = redo_cstr;
            ins_st().last_insert_skip = if added < 0 { 0 } else { g_st().Ins.new_insert_skip };
        }

        if g_st().Ins.moved == kInsNone && !end_insert_pos.is_null() {
            let mut cc: c_int;
            if !g_st().Ins.need_undo && has_format_option(kFoAuto) {
                let tpos = (**curwin.as_ptr()).w_cursor;
                cc = b'x' as c_int;
                if (**curwin.as_ptr()).w_cursor.col > 0 && gchar_cursor() == NUL as c_int {
                    dec_cursor();
                    cc = gchar_cursor();
                    if !ascii_iswhite(cc) {
                        (**curwin.as_ptr()).w_cursor = tpos;
                    }
                }

                auto_format(true, false);

                if ascii_iswhite(cc) {
                    if gchar_cursor() != NUL as c_int {
                        inc_cursor();
                    }
                    if gchar_cursor() == NUL as c_int
                        && (**curwin.as_ptr()).w_cursor.lnum == tpos.lnum
                        && (**curwin.as_ptr()).w_cursor.col == tpos.col
                    {
                        (**curwin.as_ptr()).w_cursor.coladd = tpos.coladd;
                    }
                }
            }

            check_auto_format(true);

            if nomove == 0
                && g_st().Ins.did_ai
                && !mc_ins_replaying()
                && (esc != 0
                    || (vim_strchr(opt_vars().p_cpo, kCpoIndent).is_null()
                        && (**curwin.as_ptr()).w_cursor.lnum != (*end_insert_pos).lnum))
                && (*end_insert_pos).lnum <= (*g_st().curbuf).b_ml.ml_line_count
            {
                let mut tpos = (**curwin.as_ptr()).w_cursor;
                let prev_col = (*end_insert_pos).col;

                (**curwin.as_ptr()).w_cursor = *end_insert_pos;
                check_cursor_col(&mut **curwin.as_ptr());

                let mut strip_col = (**curwin.as_ptr()).w_cursor.col;
                if gchar_cursor() == NUL as c_int && strip_col > 0 {
                    strip_col -= 1;
                }

                if *skipwhite(get_cursor_line_ptr().offset(strip_col as isize)) == NUL {
                    (**curwin.as_ptr()).w_cursor.col = strip_col;
                    loop {
                        if gchar_cursor() == NUL as c_int && (**curwin.as_ptr()).w_cursor.col > 0 {
                            (**curwin.as_ptr()).w_cursor.col -= 1;
                        }
                        cc = gchar_cursor();
                        if !ascii_iswhite(cc) {
                            break;
                        }
                        if del_char(true) == FAIL {
                            break;
                        }
                    }
                    if (**curwin.as_ptr()).w_cursor.lnum != tpos.lnum {
                        (**curwin.as_ptr()).w_cursor = tpos;
                    } else if (**curwin.as_ptr()).w_cursor.col < prev_col {
                        tpos = (**curwin.as_ptr()).w_cursor;
                        tpos.col += 1;
                        if cc != NUL as c_int && gchar_pos(&mut tpos) == NUL as c_int {
                            (**curwin.as_ptr()).w_cursor.col += 1;
                        }
                    }

                    if g_st().Visual.active {
                        check_visual_pos();
                    }
                } else {
                    (**curwin.as_ptr()).w_cursor = tpos;
                }
            }
        }
        g_st().Ins.did_ai = false;
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;

        if !end_insert_pos.is_null() {
            (*g_st().curbuf).b_op_start = g_st().Ins.start;
            (*g_st().curbuf).b_op_start_orig = g_st().Ins.start_orig;
            (*g_st().curbuf).b_op_end = *end_insert_pos;
        }
    }
}

/// Set the last inserted text to a single character.
pub fn set_last_insert(c: c_int) {
    unsafe {
        let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 5];
        let mut s = buf.as_mut_ptr();
        if c < b' ' as c_int || c == DEL {
            *s = Ctrl_V as c_char;
            s = s.add(1);
        }
        s = add_char2buf(c, s);
        *s = ESC as c_char;
        s = s.add(1);
        *s = NUL;
        ins_st().last_insert = Some(CStr::from_ptr(buf.as_ptr()).to_owned());
        ins_st().last_insert_skip = 0;
    }
}

pub fn free_last_insert() {
    ins_st().last_insert = None;
}

pub fn beginline(flags: c_int) {
    unsafe {
        if (flags & BL_SOL) != 0 && opt_vars().p_sol == 0 {
            coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
        } else {
            (**curwin.as_ptr()).w_cursor.col = 0;
            (**curwin.as_ptr()).w_cursor.coladd = 0;

            if (flags & (BL_WHITE | BL_SOL)) != 0 {
                let mut ptr = get_cursor_line_ptr();
                while ascii_iswhite(*ptr as u8 as c_int)
                    && !((flags & BL_FIX) != 0 && *ptr.add(1) == NUL)
                {
                    (**curwin.as_ptr()).w_cursor.col += 1;
                    ptr = ptr.add(1);
                }
            }
            (**curwin.as_ptr()).w_set_curswant = true;
        }
        adjust_skipcol();
    }
}

pub fn oneright() -> c_int {
    unsafe {
        if virtual_active(*curwin.as_ptr()) {
            let prevpos = (**curwin.as_ptr()).w_cursor;
            let ptr = get_cursor_pos_ptr();
            let step = if *ptr != TAB as c_char && vim_isprintc(utf_ptr2char(ptr)) {
                ptr2cells(ptr)
            } else {
                1
            };
            coladvance(&mut **curwin.as_ptr(), getviscol() + step);
            (**curwin.as_ptr()).w_set_curswant = true;
            return if prevpos.col != (**curwin.as_ptr()).w_cursor.col
                || prevpos.coladd != (**curwin.as_ptr()).w_cursor.coladd
            {
                OK
            } else {
                FAIL
            };
        }

        let ptr = get_cursor_pos_ptr();
        if *ptr == NUL {
            return FAIL;
        }

        let l = utfc_ptr2len(ptr);
        if *ptr.add(l as usize) == NUL && (get_ve_flags(*curwin.as_ptr()) & kOptVeFlagOnemore) == 0 {
            return FAIL;
        }
        (**curwin.as_ptr()).w_cursor.col += l;
        (**curwin.as_ptr()).w_set_curswant = true;
        adjust_skipcol();
        OK
    }
}

pub fn oneleft() -> c_int {
    unsafe {
        if virtual_active(*curwin.as_ptr()) {
            let v = getviscol();
            if v == 0 {
                return FAIL;
            }
            let mut width: c_int = 1;
            loop {
                coladvance(&mut **curwin.as_ptr(), v - width);
                if getviscol() < v {
                    break;
                }
                width += 1;
            }

            if (**curwin.as_ptr()).w_cursor.coladd == 1 {
                let ptr = get_cursor_pos_ptr();
                if *ptr != TAB as c_char && vim_isprintc(utf_ptr2char(ptr)) && ptr2cells(ptr) > 1 {
                    (**curwin.as_ptr()).w_cursor.coladd = 0;
                }
            }

            (**curwin.as_ptr()).w_set_curswant = true;
            adjust_skipcol();
            return OK;
        }

        if (**curwin.as_ptr()).w_cursor.col == 0 {
            return FAIL;
        }

        (**curwin.as_ptr()).w_set_curswant = true;
        (**curwin.as_ptr()).w_cursor.col -= 1;

        mb_adjust_cursor();
        adjust_skipcol();
        OK
    }
}

/// Move the cursor up `n` lines in window `wp`.
pub fn cursor_up_inner(wp: &mut win_T, mut n: linenr_T, skip_conceal: bool) {
    unsafe {
        let mut lnum = wp.w_cursor.lnum;

        if n >= lnum {
            lnum = 1;
        } else if win_lines_concealed(&mut *wp) {
            hasFolding(&raw mut *wp, lnum, &raw mut lnum, ptr::null_mut());
            while n > 0 {
                n -= 1;
                lnum -= 1;
                if lnum <= 1 {
                    break;
                }
                if skip_conceal && decor_conceal_line(&raw mut *wp, lnum - 1, true) {
                    n += 1;
                }
                if n > 0 || !((g_st().State & MODE_INSERT) != 0 || (opt_vars().fdo_flags & kOptFdoFlagAll) != 0) {
                    hasFolding(&raw mut *wp, lnum, &raw mut lnum, ptr::null_mut());
                }
            }
            lnum = lnum.max(1);
        } else {
            lnum -= n;
        }

        wp.w_cursor.lnum = lnum;
    }
}

pub fn cursor_up(n: linenr_T, upd_topline: bool) -> c_int {
    unsafe {
        if n > 0 && (**curwin.as_ptr()).w_cursor.lnum <= 1 {
            return FAIL;
        }
        cursor_up_inner(&mut **curwin.as_ptr(), n, false);
        coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
        if upd_topline {
            update_topline(*curwin.as_ptr());
        }
        OK
    }
}

/// Move the cursor down `n` lines in window `wp`.
pub fn cursor_down_inner(wp: &mut win_T, mut n: c_int, skip_conceal: bool) {
    unsafe {
        let mut lnum = wp.w_cursor.lnum;
        let line_count = (*wp.w_buffer).b_ml.ml_line_count;

        if lnum + n >= line_count {
            lnum = line_count;
        } else if win_lines_concealed(&mut *wp) {
            let mut last: linenr_T = 0;
            while n > 0 {
                n -= 1;
                if hasFoldingWin(
                    &raw mut *wp,
                    lnum,
                    ptr::null_mut(),
                    &raw mut last,
                    true,
                    ptr::null_mut(),
                ) {
                    lnum = last + 1;
                } else {
                    lnum += 1;
                }
                if lnum >= line_count {
                    break;
                }
                if skip_conceal && decor_conceal_line(&raw mut *wp, lnum - 1, true) {
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

pub fn cursor_down(n: c_int, upd_topline: bool) -> c_int {
    unsafe {
        let mut lnum = (**curwin.as_ptr()).w_cursor.lnum;
        hasFoldingWin(
            *curwin.as_ptr(),
            lnum,
            ptr::null_mut(),
            &raw mut lnum,
            true,
            ptr::null_mut(),
        );
        if n > 0 && lnum >= (*(**curwin.as_ptr()).w_buffer).b_ml.ml_line_count {
            return FAIL;
        }
        cursor_down_inner(&mut **curwin.as_ptr(), n, false);
        coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
        if upd_topline {
            update_topline(*curwin.as_ptr());
        }
        OK
    }
}

/// Stuff the last inserted text in the read buffer.
pub fn stuff_inserted(c: c_int, mut count: c_int, no_esc: c_int) -> c_int {
    unsafe {
        let mut last: c_char = NUL;
        let mut insert = get_last_insert();
        if insert.data.is_null() {
            emsg(gettext(e_noinstext.as_ptr()));
            return FAIL;
        }

        if c != NUL as c_int {
            stuffcharReadbuff(c);
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
            if (*p == b'0' as c_char || *p == b'^' as c_char)
                && (no_esc != 0 || (*insert.data == Ctrl_D as c_char && count > 1))
            {
                last = *p;
                insert.size -= 1;
            }
        }

        loop {
            stuffReadbuffLen(insert.data, insert.size as isize);
            if last == b'0' as c_char {
                stuffReadbuffLen(c"\x16048".as_ptr(), 4);
            } else if last == b'^' as c_char {
                stuffReadbuffLen(c"\x16^".as_ptr(), 2);
            }
            count -= 1;
            if count <= 0 {
                break;
            }
        }

        if no_esc == 0 {
            stuffcharReadbuff(ESC);
        }

        OK
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
        let slice = std::slice::from_raw_parts(insert.data.cast::<u8>(), insert.size);
        Some(std::ffi::CString::new(slice).unwrap_or_default())
    }
}

fn echeck_abbr(c: c_int) -> bool {
    unsafe {
        if opt_vars().p_paste != 0 || g_st().no_abbr || g_st().Ins.moved != kInsNone {
            return false;
        }
        check_abbr(
            c,
            get_cursor_line_ptr(),
            (**curwin.as_ptr()).w_cursor.col,
            if (**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.start.lnum {
                g_st().Ins.start.col
            } else {
                0
            },
        )
    }
}

/// Push character that is replaced onto the replace stack.
pub unsafe fn replace_push(str_: *mut c_char, len: usize) {
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
    unsafe {
        replace_push(c"".as_ptr().cast_mut(), 1);
    }
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
    let old_state = g_st().State;
        g_st().State = MODE_NORMAL;
        while replace_pop_if_nul() > 0 {
            mb_replace_pop_ins();
            dec_cursor();
        }
        g_st().State = old_state;
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
        let mut start_vcol: colnr_T = 0;
        let l_state = g_st().State;

        let cc = replace_pop_if_nul();
        if cc > 0 {
            let mut orig_len: c_int = 0;
            let mut orig_vcols: c_int = 0;
            if (l_state & VREPLACE_FLAG) != 0 {
                getvcol(
                    &mut **curwin.as_ptr(),
                    &mut (**curwin.as_ptr()).w_cursor,
                    ptr::null_mut(),
                    &raw mut start_vcol,
                    ptr::null_mut(),
                    0,
                );
                orig_vcols = win_chartabsize(&mut **curwin.as_ptr(), get_cursor_pos_ptr(), start_vcol);
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
                    vcol += win_chartabsize(&mut **curwin.as_ptr(), p.offset(i as isize), vcol);
                    i += utfc_ptr2len(p.offset(i as isize));
                }
                vcol -= start_vcol;

                (**curwin.as_ptr()).w_cursor.col += ins_len;
                while vcol > orig_vcols && gchar_cursor() == b' ' as c_int {
                    del_char(false);
                    orig_vcols += 1;
                }
                (**curwin.as_ptr()).w_cursor.col -= ins_len;
            }

            changed_bytes((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.col);
        } else if cc == 0 {
            del_char_after_col(limit_col);
        }
    }
}

fn ins_reg() {
    unsafe {
        let mut need_redraw = false;
        let mut literally: c_int = 0;
        let vis_active = g_st().Visual.active;

        ins_st().pc_status = PC_STATUS_UNSET;
        if redrawing() && !char_avail() {
            ins_redraw(false);
            edit_putchar(b'"' as c_int, true);
            add_to_showcmd_c(Ctrl_R);
        }

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        let mut regname = plain_vgetc();
        langmap_adjust(&mut regname, true);
        if regname == Ctrl_R || regname == Ctrl_O || regname == Ctrl_P {
            literally = regname;
            add_to_showcmd_c(literally);
            regname = plain_vgetc();
            langmap_adjust(&mut regname, true);
        }
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;

        g_st().no_u_sync += 1;
        if regname == b'=' as c_int {
            let curpos = (**curwin.as_ptr()).w_cursor;
            g_st().u_sync_once = 2;
            regname = get_expr_register();
            (**curwin.as_ptr()).w_cursor = curpos;
            check_cursor(*curwin.as_ptr());
        }
        if regname == NUL as c_int || !valid_yank_reg(regname, false) {
            vim_beep(kOptBoFlagRegister);
            need_redraw = true;
        } else {
            let reg: *mut yankreg_T = get_yank_register(regname, YREG_PASTE);

            if literally == Ctrl_O || literally == Ctrl_P {
                redo_append_char(Ctrl_R);
                redo_append_char(literally);
                redo_append_char(regname);

                do_put(
                    regname,
                    ptr::null_mut(),
                    BACKWARD,
                    1,
                    (if literally == Ctrl_P {
                        PUT_FIXINDENT
                    } else {
                        0
                    }) | PUT_CURSEND,
                );
            } else if (*reg).y_array.len() > 1 && is_literal_register(regname) {
                redo_append_char(Ctrl_R);
                redo_append_char(regname);
                do_put(regname, ptr::null_mut(), BACKWARD, 1, PUT_CURSEND);
            } else if insert_reg(regname, ptr::null_mut(), literally != 0) == FAIL {
                vim_beep(kOptBoFlagRegister);
                need_redraw = true;
            } else if g_st().Ins.stop_insert_mode {
                need_redraw = true;
            }
        }
        g_st().no_u_sync -= 1;
        if g_st().u_sync_once == 1 {
            g_st().Ins.need_undo = true;
        }
        g_st().u_sync_once = 0;

        if need_redraw || stuff_empty() {
            edit_unputchar();
        }
        clear_showcmd();

        if !vis_active && g_st().Visual.active {
            end_visual_mode();
        }
    }
}

fn ins_ctrl_g() {
    unsafe {
        setcursor();

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        let c = plain_vgetc();
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
        match c {
            K_UP | Ctrl_K => ins_up(true),
            c if c == b'k' as c_int => ins_up(true),
            K_DOWN | Ctrl_J => ins_down(true),
            c if c == b'j' as c_int => ins_down(true),
            c if c == b'u' as c_int => {
                u_sync(true);
                g_st().Ins.need_undo = true;
                g_st().Ins.update_start_orig = false;
                g_st().Ins.start = (**curwin.as_ptr()).w_cursor;
            }
            c if c == b'U' as c_int => {
                g_st().Ins.dont_sync_undo = kNone;
            }
            ESC => {}
            _ => {
                vim_beep(kOptBoFlagCtrlg);
            }
        }
    }
}

fn ins_ctrl_hat() {
    unsafe {
        if map_to_exists_mode(c"".as_ptr(), MODE_LANGMAP, false) {
            if (g_st().State & MODE_LANGMAP) != 0 {
                (*g_st().curbuf).b_p_iminsert = B_IMODE_NONE;
                g_st().State &= !MODE_LANGMAP;
            } else {
                (*g_st().curbuf).b_p_iminsert = B_IMODE_LMAP;
                g_st().State |= MODE_LANGMAP;
            }
        }
        set_iminsert_global(&mut *g_st().curbuf);
        showmode();
        status_redraw_curbuf();
    }
}

fn ins_esc(count: &mut c_int, cmdchar: c_int, nomove: bool) -> bool {
    unsafe {
        check_spell_redraw();

        let temp = (**curwin.as_ptr()).w_cursor.col;
        if ins_st().ins_esc_disabled_redraw {
            g_st().RedrawingDisabled -= 1;
            ins_st().ins_esc_disabled_redraw = false;
        }
        if g_st().Ins.moved == kInsNone {
            if cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
                redo_append_str(c"\x1b".as_ptr(), 1);
            }

            if *count > 0 {
                line_breakcheck();
                if g_st().got_int {
                    *count = 0;
                }
            }

            *count -= 1;
            if *count > 0 {
                if !vim_strchr(opt_vars().p_cpo, kCpoReplcnt).is_null() {
                    g_st().State &= !REPLACE_FLAG;
                }

                start_redo_ins();
                if cmdchar == b'r' as c_int || cmdchar == b'v' as c_int {
                    stuffRedoReadbuff(c"\x1b".as_ptr());
                }
                g_st().RedrawingDisabled += 1;
                ins_st().ins_esc_disabled_redraw = true;
                return false;
            }
            stop_insert(&raw mut (**curwin.as_ptr()).w_cursor, 1, c_int::from(nomove));
            undisplay_dollar();
        } else if g_st().Ins.moved == kInsArrow && cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
            redo_append_str(c"\x1b".as_ptr(), 1);
        }

        if cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
            ins_apply_autocmds(EVENT_INSERTLEAVEPRE);
        }

        if g_st().restart_edit == NUL as c_int && temp == (**curwin.as_ptr()).w_cursor.col {
            (**curwin.as_ptr()).w_set_curswant = true;
        }

        if (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 {
            let view = mark_view_make(&**curwin.as_ptr(), (**curwin.as_ptr()).w_cursor);
            reset_fmark(
                &mut (*g_st().curbuf).b_last_insert,
                (**curwin.as_ptr()).w_cursor,
                (*g_st().curbuf).b_fnum,
                view,
            );
        }

        if !nomove
            && ((**curwin.as_ptr()).w_cursor.col != 0 || (**curwin.as_ptr()).w_cursor.coladd > 0)
            && (g_st().restart_edit == NUL as c_int || (gchar_cursor() == NUL as c_int && !g_st().Visual.active))
            && !g_st().Ins.revins_on
        {
            if (**curwin.as_ptr()).w_cursor.coladd > 0 || get_ve_flags(*curwin.as_ptr()) == kOptVeFlagAll {
                oneleft();
                if g_st().restart_edit != NUL as c_int {
                    (**curwin.as_ptr()).w_cursor.coladd += 1;
                }
            } else {
                (**curwin.as_ptr()).w_cursor.col -= 1;
                (**curwin.as_ptr()).w_valid &= !(VALID_WCOL | VALID_VIRTCOL);
                mb_adjust_cursor();
            }
        }

        g_st().State = MODE_NORMAL;
        may_trigger_modechanged();
        if gchar_cursor() == TAB || buf_meta_total(&*g_st().curbuf, kMTMetaInline) > 0 {
            (**curwin.as_ptr()).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
        }

        setmouse();
        ui_cursor_shape();

        if g_st().reg_recording != 0 || g_st().restart_edit != NUL as c_int {
            showmode();
        } else if opt_vars().p_smd != 0
            && (g_st().got_int || !skip_showmode())
            && !(opt_vars().p_ch == 0 && !ui_has(kUIMessages))
        {
            unshowmode(false);
        }
        true
    }
}

fn ins_ctrl_() {
    unsafe {
        if g_st().Ins.revins_on && g_st().Ins.revins_chars != 0 && g_st().Ins.revins_scol >= 0 {
            while gchar_cursor() != NUL as c_int && g_st().Ins.revins_chars != 0 {
                g_st().Ins.revins_chars -= 1;
                (**curwin.as_ptr()).w_cursor.col += 1;
            }
        }
        opt_vars().p_ri = c_int::from(opt_vars().p_ri == 0);
        g_st().Ins.revins_on = g_st().State == MODE_INSERT && opt_vars().p_ri != 0;
        if g_st().Ins.revins_on {
            g_st().Ins.revins_scol = (**curwin.as_ptr()).w_cursor.col;
            g_st().Ins.revins_legal += 1;
            g_st().Ins.revins_chars = 0;
            undisplay_dollar();
        } else {
            g_st().Ins.revins_scol = -1;
        }
        showmode();
    }
}

fn ins_start_select(c: c_int) -> bool {
    unsafe {
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
                stuffcharReadbuff(Ctrl_O);
                if g_st().mod_mask != 0 {
                    let buf = [
                        K_SPECIAL as c_char,
                        KS_MODIFIER as c_char,
                        g_st().mod_mask as u8 as c_char,
                        NUL,
                    ];
                    stuffReadbuffLen((&raw const buf).cast(), 3);
                }
                stuffcharReadbuff(c);
                true
            }
            _ => false,
        }
    }
}

fn ins_insert(replace_state: c_int) {
    let mode_str = if (g_st().State & REPLACE_FLAG) != 0 {
            c"i".as_ptr()
        } else if replace_state == MODE_VREPLACE {
            c"v".as_ptr()
        } else {
            c"r".as_ptr()
        };
        set_vim_var_string(VV_INSERTMODE, mode_str, 1);
        ins_apply_autocmds(EVENT_INSERTCHANGE);
        if (g_st().State & REPLACE_FLAG) != 0 {
            g_st().State = MODE_INSERT | (g_st().State & MODE_LANGMAP);
        } else {
            g_st().State = replace_state | (g_st().State & MODE_LANGMAP);
        }
        may_trigger_modechanged();
        redo_append_char(K_INS);
        showmode();
        ui_cursor_shape();
}

fn ins_ctrl_o() {
    unsafe {
        g_st().Visual.restart_select = 0;
        if (g_st().State & VREPLACE_FLAG) != 0 {
            g_st().restart_edit = b'V' as c_int;
        } else if (g_st().State & REPLACE_FLAG) != 0 {
            g_st().restart_edit = b'R' as c_int;
        } else {
            g_st().restart_edit = b'I' as c_int;
        }
        if virtual_active(*curwin.as_ptr()) {
            g_st().ins_at_eol = false;
        } else {
            g_st().ins_at_eol = gchar_cursor() == NUL as c_int;
        }
    }
}

fn ins_shift(c: c_int, lastc: c_int) {
    unsafe {
        if stop_arrow() == FAIL {
            return;
        }
        redo_append_char(c);

        if c == Ctrl_D
            && (lastc == b'0' as c_int || lastc == b'^' as c_int)
            && (**curwin.as_ptr()).w_cursor.col > 0
        {
            (**curwin.as_ptr()).w_cursor.col -= 1;
            del_char(false);
            if (g_st().State & REPLACE_FLAG) != 0 {
                replace_pop_ins();
            }
            if lastc == b'^' as c_int {
                g_st().old_indent = get_indent();
            }
            change_indent(INDENT_SET, 0, 1, true);
        } else {
            change_indent(
                if c == Ctrl_D { INDENT_DEC } else { INDENT_INC },
                0,
                1,
                true,
            );
        }

        if g_st().Ins.did_ai && *skipwhite(get_cursor_line_ptr()) != NUL {
            g_st().Ins.did_ai = false;
        }
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;
        g_st().Ins.can_cindent = false;
    }
}

fn ins_del() {
    unsafe {
        if stop_arrow() == FAIL {
            return;
        }
        if gchar_cursor() == NUL as c_int {
            let temp = (**curwin.as_ptr()).w_cursor.col;
            if !can_bs(BS_EOL) || do_join(2, false, true, false, false) == FAIL {
                vim_beep(kOptBoFlagBackspace);
            } else {
                mc_ins_join();
                (**curwin.as_ptr()).w_cursor.col = temp;
                if (g_st().State & VREPLACE_FLAG) != 0 && g_st().orig_line_count > (*g_st().curbuf).b_ml.ml_line_count {
                    g_st().orig_line_count = (*g_st().curbuf).b_ml.ml_line_count;
                }
            }
        } else if del_char(false) == FAIL {
            vim_beep(kOptBoFlagBackspace);
        }
        g_st().Ins.did_ai = false;
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;
        redo_append_char(K_DEL);
    }
}

unsafe fn ins_bs(c: c_int, mut mode: c_int, inserted_space_p: *mut c_int) -> bool {
    unsafe {
        let mut cc: c_int;
        let mut temp: c_int = 0;
        let mut did_backspace = false;
        let mut call_fix_indent = false;

        if buf_is_empty(&mut *g_st().curbuf)
            || (!g_st().Ins.revins_on
                && (((**curwin.as_ptr()).w_cursor.lnum == 1 && (**curwin.as_ptr()).w_cursor.col == 0)
                    || (!can_bs(BS_START)
                        && ((g_st().Ins.moved != kInsNone && !bt_prompt(g_st().curbuf))
                            || ((**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.start_orig.lnum
                                && (**curwin.as_ptr()).w_cursor.col <= g_st().Ins.start_orig.col)))
                    || (!can_bs(BS_INDENT)
                        && g_st().Ins.moved == kInsNone
                        && g_st().Ins.ai_col > 0
                        && (**curwin.as_ptr()).w_cursor.col <= g_st().Ins.ai_col)
                    || (!can_bs(BS_EOL) && (**curwin.as_ptr()).w_cursor.col == 0)))
        {
            vim_beep(kOptBoFlagBackspace);
            return false;
        }

        if stop_arrow() == FAIL {
            return false;
        }
        let in_indent = inindent(0);
        if in_indent {
            g_st().Ins.can_cindent = false;
        }
        g_st().Ins.end_comment_pending = NUL as c_int;
        if g_st().Ins.revins_on {
            inc_cursor();
        }
        if (**curwin.as_ptr()).w_cursor.coladd > 0 {
            if mode == BACKSPACE_CHAR {
                (**curwin.as_ptr()).w_cursor.coladd -= 1;
                return true;
            }
            if mode == BACKSPACE_WORD {
                (**curwin.as_ptr()).w_cursor.coladd = 0;
                return true;
            }
            (**curwin.as_ptr()).w_cursor.coladd = 0;
        }

        if (**curwin.as_ptr()).w_cursor.col == 0 {
            mc_ins_join();
            let lnum = g_st().Ins.start.lnum;
            if (**curwin.as_ptr()).w_cursor.lnum == lnum || g_st().Ins.revins_on {
                if u_save((**curwin.as_ptr()).w_cursor.lnum - 2, (**curwin.as_ptr()).w_cursor.lnum + 1) == FAIL {
                    return false;
                }
                g_st().Ins.start.lnum -= 1;
                g_st().Ins.start.col = ml_get_len(g_st().Ins.start.lnum);
            }
            cc = -1;
            if (g_st().State & REPLACE_FLAG) != 0 {
                cc = replace_pop_if_nul();
            }
            if (g_st().State & REPLACE_FLAG) != 0 && (**curwin.as_ptr()).w_cursor.lnum <= lnum {
                dec_cursor();
            } else {
                if (g_st().State & VREPLACE_FLAG) == 0 || (**curwin.as_ptr()).w_cursor.lnum > g_st().orig_line_count {
                    temp = gchar_cursor();
                    (**curwin.as_ptr()).w_cursor.lnum -= 1;

                    if has_format_option(kFoAuto) && has_format_option(kFoWhitePar) {
                        let ptr = ml_get_buf(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum);
                        let len = get_cursor_line_len();
                        if len > 0 && *ptr.add((len - 1) as usize) == b' ' as c_char {
                            let new_line =
                                std::slice::from_raw_parts(ptr, (len - 1) as usize).to_vec();
                            ml_replace_dirty_line(&mut *g_st().curbuf, &new_line, false);
                        }
                    }

                    do_join(2, false, false, false, false);
                    if temp == NUL as c_int && gchar_cursor() != NUL as c_int {
                        inc_cursor();
                    }
                } else {
                    dec_cursor();
                }

                if (g_st().State & REPLACE_FLAG) != 0 {
                    let old_state = g_st().State;
                    g_st().State = MODE_NORMAL;
                    while cc > 0 {
                        let save_col = (**curwin.as_ptr()).w_cursor.col;
                        mb_replace_pop_ins();
                        (**curwin.as_ptr()).w_cursor.col = save_col;
                        cc = replace_pop_if_nul();
                    }
                    replace_pop_ins();
                    g_st().State = old_state;
                }
            }
            g_st().Ins.did_ai = false;
        } else {
            if g_st().Ins.revins_on {
                dec_cursor();
            }
            let mut mincol: colnr_T = 0;
            if mode == BACKSPACE_LINE && ((*g_st().curbuf).b_p_ai != 0 || cindent_on()) && !g_st().Ins.revins_on {
                let save_col = (**curwin.as_ptr()).w_cursor.col;
                beginline(BL_WHITE);
                if (**curwin.as_ptr()).w_cursor.col < save_col {
                    mincol = (**curwin.as_ptr()).w_cursor.col;
                    call_fix_indent = true;
                }
                (**curwin.as_ptr()).w_cursor.col = save_col;
            }

            if mode == BACKSPACE_CHAR
                && ((opt_vars().p_sta != 0 && in_indent)
                    || ((get_sts_value() != 0 || tabstop_count(&(*g_st().curbuf).b_p_vsts_array) != 0)
                        && (**curwin.as_ptr()).w_cursor.col > 0
                        && (*get_cursor_pos_ptr().offset(-1) == TAB as c_char
                            || (*get_cursor_pos_ptr().offset(-1) == b' ' as c_char
                                && (*inserted_space_p == 0 || g_st().Ins.moved != kInsNone)))))
            {
                *inserted_space_p = 0;

                let use_ts =
                    (**curwin.as_ptr()).w_onebuf_opt.wo_list == 0 || (**curwin.as_ptr()).w_p_lcs_chars.tab1 != 0;
                let line = get_cursor_line_ptr();
                let cursor_ptr = line.offset((**curwin.as_ptr()).w_cursor.col as isize).cast_const();

                let mut vcol: colnr_T = 0;
                let mut space_vcol: colnr_T = 0;
                let mut sci = utf_ptr2StrCharInfo(line);
                let mut space_sci = sci;
                let mut prev_space = false;

                while sci.ptr < cursor_ptr {
                    let cur_space = ascii_iswhite(sci.chr.value);
                    if !prev_space && cur_space {
                        space_sci = sci;
                        space_vcol = vcol;
                    }
                    let cli = utf_ClusterInfo(sci);
                    vcol +=
                        charsize_nowrap(&mut *g_st().curbuf, sci.ptr, use_ts, vcol, sci.chr.value, cli.cells);
                    sci = cli.next;
                    prev_space = cur_space;
                }

                let mut want_vcol = if vcol > 0 { vcol - 1 } else { 0 };
                if opt_vars().p_sta != 0 && in_indent {
                    want_vcol -= want_vcol % (get_sw_value(g_st().curbuf) as colnr_T);
                } else {
                    want_vcol = tabstop_start(
                        want_vcol,
                        get_sts_value() as c_int,
                        &(*g_st().curbuf).b_p_vsts_array,
                    );
                }

                loop {
                    let cli = utf_ClusterInfo(space_sci);
                    let size = charsize_nowrap(
                        &mut *g_st().curbuf,
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
                let want_col = space_sci.ptr.offset_from(line.cast_const()) as colnr_T;

                while (**curwin.as_ptr()).w_cursor.col > want_col {
                    dec_cursor();
                    if (g_st().State & REPLACE_FLAG) != 0 {
                        if (**curwin.as_ptr()).w_cursor.lnum != g_st().Ins.start.lnum
                            || (**curwin.as_ptr()).w_cursor.col >= g_st().Ins.start.col
                        {
                            replace_do_bs(-1);
                        }
                    } else {
                        del_char(false);
                    }
                }

                while space_vcol < want_vcol {
                    if (**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.start_orig.lnum
                        && (**curwin.as_ptr()).w_cursor.col < g_st().Ins.start_orig.col
                    {
                        g_st().Ins.start_orig.col = (**curwin.as_ptr()).w_cursor.col;
                    }

                    if (g_st().State & VREPLACE_FLAG) != 0 {
                        ins_char(b' ' as c_int);
                    } else {
                        ins_str(c" ".as_ptr().cast_mut(), 1);
                        if (g_st().State & REPLACE_FLAG) != 0 {
                            replace_push_nul();
                        }
                    }
                    space_vcol += 1;
                }
            } else {
                let mut cclass = mb_get_class(get_cursor_pos_ptr());
                loop {
                    if !g_st().Ins.revins_on {
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
                        if !g_st().Ins.revins_on {
                            inc_cursor();
                        } else if (g_st().State & REPLACE_FLAG) != 0 {
                            dec_cursor();
                        }
                        break;
                    }
                    if (g_st().State & REPLACE_FLAG) != 0 {
                        replace_do_bs(-1);
                    } else {
                        let mut has_composing = false;
                        if opt_vars().p_deco != 0 {
                            let p0 = get_cursor_pos_ptr();
                            has_composing = utf_composinglike(
                                p0,
                                p0.add(utf_ptr2len(p0) as usize),
                                ptr::null_mut::<GraphemeState>(),
                            );
                        }
                        del_char(false);
                        if has_composing {
                            inc_cursor();
                        }
                        if g_st().Ins.revins_chars != 0 {
                            g_st().Ins.revins_chars -= 1;
                            g_st().Ins.revins_legal += 1;
                        }
                        if g_st().Ins.revins_on && gchar_cursor() == NUL as c_int {
                            break;
                        }
                    }
                    if mode == BACKSPACE_CHAR {
                        break;
                    }
                    if !(g_st().Ins.revins_on
                        || ((**curwin.as_ptr()).w_cursor.col > mincol
                            && (can_bs(BS_NOSTOP)
                                || (**curwin.as_ptr()).w_cursor.lnum != g_st().Ins.start_orig.lnum
                                || (**curwin.as_ptr()).w_cursor.col != g_st().Ins.start_orig.col)))
                    {
                        break;
                    }
                }
            }
            did_backspace = true;
        }
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;
        if (**curwin.as_ptr()).w_cursor.col <= 1 {
            g_st().Ins.did_ai = false;
        }

        if call_fix_indent {
            fix_indent();
        }

        redo_append_char(c);

        if (**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.start_orig.lnum
            && (**curwin.as_ptr()).w_cursor.col < g_st().Ins.start_orig.col
        {
            g_st().Ins.start_orig.col = (**curwin.as_ptr()).w_cursor.col;
        }

        if !vim_strchr(opt_vars().p_cpo, kCpoBackspace).is_null() && g_st().dollar_vcol == -1 {
            g_st().dollar_vcol = (**curwin.as_ptr()).w_virtcol;
        }

        if did_backspace {
            foldOpenCursor();
        }
        did_backspace
    }
}

fn ins_left() {
    unsafe {
        let end_change = g_st().Ins.dont_sync_undo == kFalse;
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if oneleft() == OK {
            start_arrow_with_change(&mut tpos, end_change, K_LEFT);
            if g_st().Ins.revins_scol != -1 && (**curwin.as_ptr()).w_cursor.col >= g_st().Ins.revins_scol {
                g_st().Ins.revins_legal += 1;
            }
            g_st().Ins.revins_chars += 1;
        } else if !vim_strchr(opt_vars().p_ww, b'[' as c_int).is_null() && (**curwin.as_ptr()).w_cursor.lnum > 1 {
            start_arrow(&raw mut tpos, true, K_LEFT);
            (**curwin.as_ptr()).w_cursor.lnum -= 1;
            coladvance(&mut **curwin.as_ptr(), MAXCOL);
            (**curwin.as_ptr()).w_set_curswant = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        g_st().Ins.dont_sync_undo = kFalse;
    }
}

fn ins_home(c: c_int) {
    unsafe {
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if c == K_C_HOME {
            (**curwin.as_ptr()).w_cursor.lnum = 1;
        }
        (**curwin.as_ptr()).w_cursor.col = 0;
        (**curwin.as_ptr()).w_cursor.coladd = 0;
        (**curwin.as_ptr()).w_curswant = 0;
        start_arrow(
            &raw mut tpos,
            true,
            if c == K_C_HOME { NUL as c_int } else { K_HOME },
        );
    }
}

fn ins_end(c: c_int) {
    unsafe {
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if c == K_C_END {
            (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_ml.ml_line_count;
        }
        coladvance(&mut **curwin.as_ptr(), MAXCOL);
        (**curwin.as_ptr()).w_curswant = MAXCOL;
        start_arrow(
            &raw mut tpos,
            true,
            if c == K_C_END { NUL as c_int } else { K_END },
        );
    }
}

fn ins_s_left() {
    unsafe {
        let end_change = g_st().Ins.dont_sync_undo == kFalse;
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        if (**curwin.as_ptr()).w_cursor.lnum > 1 || (**curwin.as_ptr()).w_cursor.col > 0 {
            start_arrow_with_change(&mut (**curwin.as_ptr()).w_cursor, end_change, K_S_LEFT);
            bck_word(1, false, false);
            (**curwin.as_ptr()).w_set_curswant = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        g_st().Ins.dont_sync_undo = kFalse;
    }
}

fn ins_right() {
    unsafe {
        let end_change = g_st().Ins.dont_sync_undo == kFalse;
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        if gchar_cursor() != NUL as c_int || virtual_active(*curwin.as_ptr()) {
            start_arrow_with_change(&mut (**curwin.as_ptr()).w_cursor, end_change, K_RIGHT);
            (**curwin.as_ptr()).w_set_curswant = true;
            if virtual_active(*curwin.as_ptr()) {
                oneright();
            } else {
                (**curwin.as_ptr()).w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
            }

            g_st().Ins.revins_legal += 1;
            if g_st().Ins.revins_chars != 0 {
                g_st().Ins.revins_chars -= 1;
            }
        } else if !vim_strchr(opt_vars().p_ww, b']' as c_int).is_null()
            && (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_ml.ml_line_count
        {
            start_arrow(&raw mut (**curwin.as_ptr()).w_cursor, true, K_RIGHT);
            (**curwin.as_ptr()).w_set_curswant = true;
            (**curwin.as_ptr()).w_cursor.lnum += 1;
            (**curwin.as_ptr()).w_cursor.col = 0;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        g_st().Ins.dont_sync_undo = kFalse;
    }
}

fn ins_s_right() {
    unsafe {
        let end_change = g_st().Ins.dont_sync_undo == kFalse;
        if (opt_vars().fdo_flags & kOptFdoFlagHor) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        if (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_ml.ml_line_count || gchar_cursor() != NUL as c_int
        {
            start_arrow_with_change(&mut (**curwin.as_ptr()).w_cursor, end_change, K_S_RIGHT);
            fwd_word(1, false, false);
            (**curwin.as_ptr()).w_set_curswant = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        g_st().Ins.dont_sync_undo = kFalse;
    }
}

fn ins_up(startcol: bool) {
    unsafe {
        let old_topline = (**curwin.as_ptr()).w_topline;
        let old_topfill = (**curwin.as_ptr()).w_topfill;

        undisplay_dollar();
        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if cursor_up(1, true) == OK {
            if startcol {
                coladvance(&mut **curwin.as_ptr(), getvcol_nolist(&mut g_st().Ins.start));
            }
            if old_topline != (**curwin.as_ptr()).w_topline || old_topfill != (**curwin.as_ptr()).w_topfill {
                redraw_later(*curwin.as_ptr(), UPD_VALID);
            }
            start_arrow(
                &raw mut tpos,
                true,
                if startcol { b'k' as c_int } else { K_UP },
            );
            g_st().Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

fn ins_pageup() {
    unsafe {
        undisplay_dollar();

        if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            if !(*g_st().first_tabpage).tp_next.is_null() {
                start_arrow(&raw mut (**curwin.as_ptr()).w_cursor, true, NUL as c_int);
                goto_tabpage(-1);
            }
            return;
        }

        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if pagescroll(BACKWARD, 1, false) == OK {
            start_arrow(&raw mut tpos, true, NUL as c_int);
            g_st().Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

fn ins_down(startcol: bool) {
    unsafe {
        let old_topline = (**curwin.as_ptr()).w_topline;
        let old_topfill = (**curwin.as_ptr()).w_topfill;

        undisplay_dollar();
        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if cursor_down(1, true) == OK {
            if startcol {
                coladvance(&mut **curwin.as_ptr(), getvcol_nolist(&mut g_st().Ins.start));
            }
            if old_topline != (**curwin.as_ptr()).w_topline || old_topfill != (**curwin.as_ptr()).w_topfill {
                redraw_later(*curwin.as_ptr(), UPD_VALID);
            }
            start_arrow(
                &raw mut tpos,
                true,
                if startcol { b'j' as c_int } else { K_DOWN },
            );
            g_st().Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

fn ins_pagedown() {
    unsafe {
        undisplay_dollar();

        if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
            if !(*g_st().first_tabpage).tp_next.is_null() {
                start_arrow(&raw mut (**curwin.as_ptr()).w_cursor, true, NUL as c_int);
                goto_tabpage(0);
            }
            return;
        }

        let mut tpos = (**curwin.as_ptr()).w_cursor;
        if pagescroll(FORWARD, 1, false) == OK {
            start_arrow(&raw mut tpos, true, NUL as c_int);
            g_st().Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

fn ins_tab() -> bool {
    unsafe {
        if g_st().Ins.start_blank_vcol == MAXCOL && (**curwin.as_ptr()).w_cursor.lnum == g_st().Ins.start.lnum {
            g_st().Ins.start_blank_vcol = get_nolist_virtcol();
        }
        if echeck_abbr(TAB + ABBR_OFF) {
            return false;
        }

        let ind = inindent(0);
        if ind {
            g_st().Ins.can_cindent = false;
        }

        if (*g_st().curbuf).b_p_et == 0
            && !(opt_vars().p_sta != 0
                && ind
                && ((tabstop_count(&(*g_st().curbuf).b_p_vts_array) > 1)
                    || (tabstop_count(&(*g_st().curbuf).b_p_vts_array) == 1
                        && tabstop_first(&(*g_st().curbuf).b_p_vts_array) != get_sw_value(g_st().curbuf))
                    || (tabstop_count(&(*g_st().curbuf).b_p_vts_array) == 0
                        && (*g_st().curbuf).b_p_ts != i64::from(get_sw_value(g_st().curbuf)))))
            && tabstop_count(&(*g_st().curbuf).b_p_vsts_array) == 0
            && get_sts_value() == 0
        {
            return true;
        }

        if stop_arrow() == FAIL {
            return true;
        }

        g_st().Ins.did_ai = false;
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;
        redo_append_str(c"\t".as_ptr(), 1);

        let mut temp: c_int = if opt_vars().p_sta != 0 && ind {
            let sw = get_sw_value(g_st().curbuf);
            sw - (get_nolist_virtcol() % sw)
        } else if tabstop_count(&(*g_st().curbuf).b_p_vsts_array) > 0 || (*g_st().curbuf).b_p_sts != 0 {
            tabstop_padding(
                get_nolist_virtcol(),
                i64::from(get_sts_value()),
                &(*g_st().curbuf).b_p_vsts_array,
            )
        } else {
            tabstop_padding(
                get_nolist_virtcol(),
                (*g_st().curbuf).b_p_ts,
                &(*g_st().curbuf).b_p_vts_array,
            )
        };

        ins_char(b' ' as c_int);
        while {
            temp -= 1;
            temp > 0
        } {
            if (g_st().State & VREPLACE_FLAG) != 0 {
                ins_char(b' ' as c_int);
            } else {
                ins_str(c" ".as_ptr().cast_mut(), 1);
                if (g_st().State & REPLACE_FLAG) != 0 {
                    replace_push_nul();
                }
            }
        }

        if (*g_st().curbuf).b_p_et == 0
            && (tabstop_count(&(*g_st().curbuf).b_p_vsts_array) > 0
                || get_sts_value() > 0
                || (opt_vars().p_sta != 0 && ind))
        {
            let mut pos: pos_T;
            let cursor: *mut pos_T;
            let mut saved_line_vec: Vec<c_char>;
            let mut saved_line: *mut c_char = ptr::null_mut();
            let mut ptr: *mut c_char;
            let mut vcol: colnr_T = 0;
            let mut want_vcol: colnr_T = 0;
            let mut change_col: c_int = -1;
            let save_list = (**curwin.as_ptr()).w_onebuf_opt.wo_list;

            if (g_st().State & VREPLACE_FLAG) != 0 {
                pos = (**curwin.as_ptr()).w_cursor;
                cursor = &raw mut pos;
                let line_len = get_cursor_line_len() as usize;
                saved_line_vec =
                    std::slice::from_raw_parts(get_cursor_line_ptr(), line_len + 1).to_vec();
                saved_line_vec[line_len] = NUL;
                saved_line = saved_line_vec.as_mut_ptr();
                ptr = saved_line.offset(pos.col as isize);
            } else {
                ptr = get_cursor_pos_ptr();
                cursor = &raw mut (**curwin.as_ptr()).w_cursor;
            }

            if vim_strchr(opt_vars().p_cpo, kCpoListwm).is_null() {
                (**curwin.as_ptr()).w_onebuf_opt.wo_list = 0;
            }

            let mut fpos = (**curwin.as_ptr()).w_cursor;
            while fpos.col > 0 && ascii_iswhite(*ptr.offset(-1) as u8 as c_int) {
                fpos.col -= 1;
                ptr = ptr.offset(-1);
            }

            if (g_st().State & REPLACE_FLAG) != 0
                && fpos.lnum == g_st().Ins.start.lnum
                && fpos.col < g_st().Ins.start.col
            {
                ptr = ptr.offset((g_st().Ins.start.col - fpos.col) as isize);
                fpos.col = g_st().Ins.start.col;
            }

            getvcol(
                &mut **curwin.as_ptr(),
                &mut fpos,
                &raw mut vcol,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            );
            getvcol(
                &mut **curwin.as_ptr(),
                &mut *cursor,
                &raw mut want_vcol,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            );

            let tab = c"\t".as_ptr().cast_mut();
            let tab_v = b'\t' as i32;
            let mut csarg = CharsizeArg::new();
            let mut cstype = init_charsize_arg(&mut csarg, *curwin.as_ptr(), 0, tab);

            while ascii_iswhite(*ptr as u8 as c_int) {
                let i = win_charsize(cstype, vcol, tab, tab_v, &raw mut csarg, 1).width;
                if vcol + i > want_vcol {
                    break;
                }
                if *ptr != TAB as c_char {
                    *ptr = TAB as c_char;
                    if change_col < 0 {
                        change_col = fpos.col;
                        if fpos.lnum == g_st().Ins.start.lnum && fpos.col < g_st().Ins.start.col {
                            g_st().Ins.start.col = fpos.col;
                        }
                    }
                }
                fpos.col += 1;
                ptr = ptr.add(1);
                vcol += i;
            }

            if change_col >= 0 {
                let mut repl_off: c_int = 0;
                cstype = init_charsize_arg(&mut csarg, *curwin.as_ptr(), 0, ptr);
                while vcol < want_vcol && *ptr == b' ' as c_char {
                    vcol += win_charsize(cstype, vcol, ptr, b' ' as i32, &raw mut csarg, 1).width;
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
                    if (g_st().State & VREPLACE_FLAG) == 0 {
                        let newp_len = ((*g_st().curbuf).b_ml.ml_line_textlen - i) as usize;
                        let col = ptr.offset_from((*g_st().curbuf).b_ml.ml_line_ptr) as usize;
                        let mut new_line = Vec::with_capacity(newp_len.saturating_sub(1));
                        if col > 0 {
                            new_line
                                .extend_from_slice(std::slice::from_raw_parts(ptr.sub(col), col));
                        }
                        new_line.extend_from_slice(std::slice::from_raw_parts(
                            ptr.offset(i as isize),
                            newp_len.saturating_sub(col + 1),
                        ));
                        ml_replace_dirty_line(&mut *g_st().curbuf, &new_line, true);
                        inserted_bytes(
                            fpos.lnum,
                            change_col,
                            (*cursor).col - change_col,
                            fpos.col - change_col,
                        );
                    } else {
                        let rem = std::ffi::CStr::from_ptr(ptr.offset(i as isize)).to_bytes().len() + 1;
                        ptr::copy(ptr.offset(i as isize), ptr, rem);
                    }
                    if (g_st().State & REPLACE_FLAG) != 0 && (g_st().State & VREPLACE_FLAG) == 0 {
                        for _ in 0..i {
                            replace_join(repl_off);
                        }
                    }
                }
                (*cursor).col -= i;

                if (g_st().State & VREPLACE_FLAG) != 0 {
                    backspace_until_column(change_col);
                    ins_bytes_len(
                        saved_line.offset(change_col as isize),
                        ((*cursor).col - change_col) as usize,
                    );
                }
            }

            (**curwin.as_ptr()).w_onebuf_opt.wo_list = save_list;
        }

        false
    }
}

/// Handle CR or NL in insert mode.
pub fn ins_eol(c: c_int) -> bool {
    unsafe {
        if echeck_abbr(c + ABBR_OFF) {
            return true;
        }
        if stop_arrow() == FAIL {
            return false;
        }
        undisplay_dollar();

        if (g_st().State & REPLACE_FLAG) != 0 && (g_st().State & VREPLACE_FLAG) == 0 {
            replace_push_nul();
        }

        if virtual_active(*curwin.as_ptr()) && (**curwin.as_ptr()).w_cursor.coladd > 0 {
            coladvance(&mut **curwin.as_ptr(), getviscol());
        }

        if g_st().Ins.revins_on {
            (**curwin.as_ptr()).w_cursor.col += get_cursor_pos_len();
        }

        redo_append_str(c"\n".as_ptr(), 1);
        let i = open_line(
            FORWARD,
            if has_format_option(kFoRetComs) {
                OPENLINE_DO_COM
            } else {
                0
            },
            g_st().old_indent,
            ptr::null_mut(),
        );
        g_st().old_indent = 0;
        g_st().Ins.can_cindent = true;
        foldOpenCursor();
        i
    }
}

fn ins_digraph() -> c_int {
    unsafe {
        let mut did_putchar = false;

        ins_st().pc_status = PC_STATUS_UNSET;
        if redrawing() && !char_avail() {
            ins_redraw(false);
            edit_putchar(b'?' as c_int, true);
            did_putchar = true;
            add_to_showcmd_c(Ctrl_K);
        }

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        let mut c = plain_vgetc();
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
        if did_putchar {
            edit_unputchar();
        }

        if IS_SPECIAL(c) || g_st().mod_mask != 0 {
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
                redo_append_str(c"\x16".as_ptr(), 1);
                c = digraph_get(c, cc, true);
                clear_showcmd();
                return c;
            }
        }
        clear_showcmd();
        NUL as c_int
    }
}

/// Handle CTRL-E and CTRL-Y in Insert mode: copy char from other line.
pub fn ins_copychar(lnum: linenr_T) -> c_int {
    unsafe {
        if lnum < 1 || lnum > (*g_st().curbuf).b_ml.ml_line_count {
            vim_beep(kOptBoFlagCopy);
            return NUL as c_int;
        }

        validate_virtcol(*curwin.as_ptr());
        let end_vcol = (**curwin.as_ptr()).w_virtcol;
        let line = ml_get(lnum);

        let mut csarg = CharsizeArg::new();
        let cstype = init_charsize_arg(&mut csarg, *curwin.as_ptr(), lnum, line);
        let mut ci = utf_ptr2StrCharInfo(line);
        let mut vcol: c_int = 0;
        while vcol < end_vcol && *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            vcol += win_charsize(
                cstype,
                vcol,
                ci.ptr,
                ci.chr.value,
                &raw mut csarg,
                cli.cells,
            )
            .width;
            if vcol > end_vcol {
                break;
            }
            ci = cli.next;
        }

        let c = if ci.chr.value < 0 {
            *ci.ptr as u8 as c_int
        } else {
            ci.chr.value
        };
        if c == NUL as c_int {
            vim_beep(kOptBoFlagCopy);
        }
        c
    }
}

fn ins_ctrl_ey(tc: c_int) -> c_int {
    unsafe {
        let mut c = tc;
        if ctrl_x_mode_scroll() {
            if c == Ctrl_Y {
                scrolldown_clamp();
            } else {
                scrollup_clamp();
            }
            redraw_later(*curwin.as_ptr(), UPD_VALID);
        } else {
            c = ins_copychar((**curwin.as_ptr()).w_cursor.lnum + if c == Ctrl_Y { -1 } else { 1 });
            if c != NUL as c_int {
                if c < 256 && !(c as u8).is_ascii_alphanumeric() {
                    redo_append_str(c"\x16".as_ptr(), 1);
                }
                let tw_save = (*g_st().curbuf).b_p_tw;
                (*g_st().curbuf).b_p_tw = -1;
                insert_special(c, 1, 0);
                (*g_st().curbuf).b_p_tw = tw_save;
                g_st().Ins.revins_chars += 1;
                g_st().Ins.revins_legal += 1;
                c = Ctrl_V;
                auto_format(false, true);
            }
        }
        c
    }
}

/// Get the value that `w_virtcol` would have when `'list'` is off.
pub fn get_nolist_virtcol() -> colnr_T {
    unsafe {
        if (**curwin.as_ptr()).w_buffer.is_null()
            || (*(**curwin.as_ptr()).w_buffer).b_ml.ml_mfp.is_none()
            || (**curwin.as_ptr()).w_cursor.lnum > (*(**curwin.as_ptr()).w_buffer).b_ml.ml_line_count
        {
            return 0;
        }
        if (**curwin.as_ptr()).w_onebuf_opt.wo_list != 0 && vim_strchr(opt_vars().p_cpo, kCpoListwm).is_null() {
            return getvcol_nolist(&mut (**curwin.as_ptr()).w_cursor);
        }
        validate_virtcol(*curwin.as_ptr());
        (**curwin.as_ptr()).w_virtcol
    }
}

fn do_insert_char_pre(c: c_int) -> Option<std::ffi::CString> {
    unsafe {
        if c == Ctrl_RSB || g_st().KeyStuffed != 0 || !has_event(EVENT_INSERTCHARPRE) {
            return None;
        }

        let mut buf = [0 as c_char; MB_MAXBYTES + 1];
        let save_state = g_st().State;
        let buflen = utf_char2bytes(c, (&raw mut buf).cast()) as usize;
        buf[buflen] = NUL;

        g_st().textlock += 1;
        set_vim_var_string(VV_CHAR, (&raw const buf).cast(), buflen as isize);

        let mut res: Option<std::ffi::CString> = None;
        if ins_apply_autocmds(EVENT_INSERTCHARPRE) != 0 {
            let vchar = get_vim_var_str(VV_CHAR);
            if CStr::from_ptr((&raw const buf).cast()) != CStr::from_ptr(vchar) {
                res = Some(CStr::from_ptr(vchar).to_owned());
            }
        }

        set_vim_var_string(VV_CHAR, ptr::null(), -1);
        g_st().textlock -= 1;
        g_st().State = save_state;
        res
    }
}

pub fn get_can_cindent() -> bool {
    g_st().Ins.can_cindent
}

pub fn set_can_cindent(val: bool) {
    g_st().Ins.can_cindent = val;
}

/// Trigger `event` and take care of fixing undo.
pub fn ins_apply_autocmds(event: event_T) -> c_int {
    unsafe {
        let tick = buf_get_changedtick(&*g_st().curbuf);
        let r = c_int::from(apply_autocmds(
            event,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        ));
        if event != EVENT_INSERTLEAVE && tick != buf_get_changedtick(&*g_st().curbuf) {
            u_save((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.lnum + 1);
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_spaces_and_can_cindent_work() {
        unsafe {
            let mut line = *b"hello   \0";
            truncate_spaces(line.as_mut_ptr().cast(), 8);
            assert_eq!(CStr::from_ptr(line.as_ptr().cast()).to_bytes(), b"hello");

            set_can_cindent(true);
            assert!(get_can_cindent());
            set_can_cindent(false);
            assert!(!get_can_cindent());
        }
    }
}
