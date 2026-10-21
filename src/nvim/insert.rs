//! Functions for Insert mode (`src/nvim/insert.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::private::validate::NvimString;
use crate::arglist::ML_EMPTY;
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
    K_COMPLETE_DELAY, K_PASTE_START, char_avail, getcmdkeycmd, kInsNone, langmap_adjust,
    map_execute_lua, mb_byte2len_check, merge_modifiers, paste_repeat, plain_vgetc, prep_redo,
    redo_append_char, redo_append_lit, redo_append_str, redo_keys, start_redo_ins, stop_redo_ins,
    stuff_empty, stuffReadbuffLen, stuffRedoReadbuff, stuffcharReadbuff, vgetc, vpeekc, vungetc,
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
use crate::kvec::KVec;
use crate::mapping::{check_abbr, map_to_exists_mode};
use crate::mark::{mark_view_make, reset_fmark};
use crate::marktree::kMTMetaInline;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    GraphemeState, mb_adjust_cursor, mb_get_class, ptr2cells, utf_ClusterInfo, utf_char2bytes,
    utf_char2len, utf_composinglike, utf_head_off, utf_ptr2StrCharInfo, utf_ptr2char, utf_ptr2len,
    utf8len_tab, utfc_ptr2len,
};
use crate::mcursor::{mc_ins_cascade, mc_ins_cascade_restart, mc_ins_join, mc_ins_replaying};
use crate::memline::{
    ML_LINE_DIRTY, gchar_pos, ml_append, ml_get, ml_get_buf, ml_get_len, ml_replace,
};
use crate::memory::{strnequal, xfree, xmalloc, xmemdupz, xstrdup, xstrnsave};
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
    REPLACE_FLAG, VREPLACE_FLAG, VimState, may_trigger_modechanged, may_trigger_safestate,
    state_enter, state_handle_k_event, virtual_active,
};
use crate::strings::{gettext, strlen as c_strlen, vim_snprintf, vim_strchr};
use crate::syntax::syntax_present;
use crate::terminal::terminal_enter;
use crate::textformat::{
    auto_format, check_auto_format, comp_textwidth, fex_format, has_format_option, internal_format,
};
use crate::textobject::{bck_word, fwd_word};
use crate::ui::{ui_cursor_shape, ui_flush, ui_has, vim_beep};
use crate::undo::{u_clearallandblockfree, u_save, u_save_cursor, u_sync};
use crate::window::{goto_tabpage, may_trigger_win_scrolled_resized, win_T};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;

const COM_MIDDLE: c_int = b'm' as c_int;
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

const HLF_8: usize = 1;

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

pub(crate) const ML_ALLOCATED: c_int = 0x10;

#[repr(C)]
struct InsertState {
    state: VimState,
    ca: *mut c_void,
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

static mut compl_busy: bool = false;
static mut last_insert: NvimString = NvimString {
    data: ptr::null_mut(),
    size: 0,
};
static mut last_insert_skip: c_int = 0;
static mut replace_stack: KVec<c_char> = KVec::new();

static mut pc_status: c_int = PC_STATUS_UNSET;
static mut pc_schar: schar_T = 0;
static mut pc_attr: c_int = 0;
static mut pc_row: c_int = 0;
static mut pc_col: c_int = 0;
static mut ins_esc_disabled_redraw: bool = false;

#[inline]
const fn is_special_ins(c: c_int) -> bool {
    c < b' ' as c_int || c >= DEL || c == b'0' as c_int || c == b'^' as c_int
}

#[inline]
pub unsafe fn linetabsize_str(s: *mut c_char) -> c_int {
    unsafe { linetabsize_col(0, s) }
}

#[inline]
fn mb_byte2len(b: c_int) -> c_int {
    c_int::from(utf8len_tab[(b as u8) as usize])
}

#[inline]
unsafe fn trigger_autocomplete(s: *mut InsertState) -> bool {
    unsafe {
        mc_ins_cascade();
        redraw_later(curwin, UPD_VALID);
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
unsafe fn may_trigger_autocomplete(s: *mut InsertState) {
    unsafe {
        if ins_compl_has_autocomplete() && !char_avail() && (*curwin).w_cursor.col > 0 {
            (*s).c = char_before_cursor();
            if vim_isprintc((*s).c) {
                let _ = trigger_autocomplete(s);
            }
        }
    }
}

unsafe fn insert_enter(s: *mut InsertState) {
    unsafe {
        (*s).did_backspace = true;
        (*s).old_topfill = -1;
        (*s).replaceState = MODE_REPLACE;
        (*s).cmdchar_todo = (*s).cmdchar;
        (*s).ins_just_started = true;
        Ins.did_restart_edit = restart_edit;
        msg_check_for_delay(true);
        Ins.update_start_orig = true;

        ins_compl_clear();

        if (*s).cmdchar != b'r' as c_int && (*s).cmdchar != b'v' as c_int {
            let save_cursor = (*curwin).w_cursor;
            let ptr = if (*s).cmdchar == b'R' as c_int {
                c"r".as_ptr()
            } else if (*s).cmdchar == b'V' as c_int {
                c"v".as_ptr()
            } else {
                c"i".as_ptr()
            };
            set_vim_var_string(VV_INSERTMODE, ptr, 1);
            set_vim_var_string(VV_CHAR, ptr::null(), -1);
            ins_apply_autocmds(EVENT_INSERTENTER);

            if need_highlight_changed {
                highlight_changed();
            }

            if !equalpos((*curwin).w_cursor, save_cursor)
                && *get_vim_var_str(VV_CHAR) == NUL
                && save_cursor.lnum <= (*curbuf).b_ml.ml_line_count
            {
                let save_state = State;
                (*curwin).w_cursor = save_cursor;
                State = MODE_INSERT;
                check_cursor_col(curwin);
                State = save_state;
            }
        }

        if where_paste_started.lnum != 0 {
            Ins.start = where_paste_started;
        } else {
            Ins.start = (*curwin).w_cursor;
            if (*s).startln != 0 {
                Ins.start.col = 0;
            }
        }

        Ins.start_textlen = linetabsize_str(get_cursor_line_ptr());
        Ins.start_blank_vcol = MAXCOL;

        if !Ins.did_ai {
            Ins.ai_col = 0;
        }

        if (*s).cmdchar != NUL as c_int && restart_edit == 0 {
            if (*s).cmdchar == b'V' as c_int || (*s).cmdchar == b'v' as c_int {
                prep_redo(
                    false,
                    false,
                    CmdSpec {
                        count: (*s).count as c_long,
                        cmd: b'g' as c_int,
                        cmd2: if (*s).cmdchar == b'v' as c_int {
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
                        count: (*s).count as c_long,
                        cmd: (*s).cmdchar,
                        cmd2: if (*s).cmdchar == b'g' as c_int {
                            b'I' as c_int
                        } else {
                            NUL as c_int
                        },
                        ..CmdSpec::default()
                    },
                );
                if (*s).cmdchar == b'r' as c_int {
                    (*s).count = 1;
                }
            }
        }

        if (*s).cmdchar == b'R' as c_int {
            State = MODE_REPLACE;
        } else if (*s).cmdchar == b'V' as c_int || (*s).cmdchar == b'v' as c_int {
            State = MODE_VREPLACE;
            (*s).replaceState = MODE_VREPLACE;
            orig_line_count = (*curbuf).b_ml.ml_line_count;
            vr_lines_changed = 1;
        } else {
            State = MODE_INSERT;
        }

        may_trigger_modechanged();
        Ins.stop_insert_mode = false;

        if gchar_cursor() == TAB || buf_meta_total(curbuf, kMTMetaInline) > 0 {
            (*curwin).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
        }

        if (*curbuf).b_p_iminsert == B_IMODE_LMAP {
            State |= MODE_LANGMAP;
        }

        setmouse();
        clear_showcmd();
        Ins.revins_on = State == MODE_INSERT && p_ri != 0;
        if Ins.revins_on {
            undisplay_dollar();
        }
        Ins.revins_chars = 0;
        Ins.revins_legal = 0;
        Ins.revins_scol = -1;

        if restart_edit != 0 && stuff_empty() {
            Ins.moved = if where_paste_started.lnum == 0 {
                kInsJump
            } else {
                kInsNone
            };
            restart_edit = 0;

            validate_virtcol(curwin);
            update_curswant();
            if (ins_at_eol && (*curwin).w_cursor.lnum == Ins.o_lnum)
                || (*curwin).w_curswant > (*curwin).w_virtcol
            {
                let ptr = get_cursor_line_ptr().offset((*curwin).w_cursor.col as isize);
                if *ptr != NUL {
                    if *ptr.add(1) == NUL {
                        (*curwin).w_cursor.col += 1;
                    } else {
                        (*s).i = utfc_ptr2len(ptr);
                        if *ptr.offset((*s).i as isize) == NUL {
                            (*curwin).w_cursor.col += (*s).i;
                        }
                    }
                }
            }
            ins_at_eol = false;
        } else {
            Ins.moved = kInsNone;
        }

        need_start_insertmode = false;
        Ins.need_undo = true;
        where_paste_started.lnum = 0;
        Ins.can_cindent = true;
        if Ins.did_restart_edit == 0 {
            foldOpenCursor();
        }

        (*s).i = 0;
        if p_smd != 0 && msg_silent == 0 {
            (*s).i = showmode();
        }

        if Ins.did_restart_edit == 0 {
            change_warning(curbuf, if (*s).i == 0 { 0 } else { (*s).i + 1 });
        }

        ui_cursor_shape();
        do_digraph(-1);

        let redo = redo_keys(ptr::null());
        Ins.new_insert_skip = redo.size as c_int;
        if !redo.data.is_null() {
            xfree(redo.data.cast());
        }

        old_indent = 0;

        loop {
            state_enter(&raw mut (*s).state);
            if ins_esc(&raw mut (*s).count, (*s).cmdchar, (*s).nomove) {
                break;
            }
        }

        ins_compl_clear_autocomplete_delay();
        if ins_at_eol {
            Ins.o_lnum = (*curwin).w_cursor.lnum;
        }

        pum_check_clear();
        foldUpdateAfterInsert();
        if (*s).cmdchar != b'r' as c_int && (*s).cmdchar != b'v' as c_int && (*s).c != Ctrl_C {
            ins_apply_autocmds(EVENT_INSERTLEAVE);
        }
        did_cursorhold = false;

        if !char_avail() && (*curbuf).b_last_changedtick_i == buf_get_changedtick(curbuf) {
            (*curbuf).b_last_changedtick = buf_get_changedtick(curbuf);
        }
    }
}

unsafe extern "C" fn insert_check(state: *mut VimState) -> c_int {
    unsafe {
        let s = state.cast::<InsertState>();

        mc_ins_cascade();

        if Ins.revins_legal == 0 {
            Ins.revins_scol = -1;
        } else {
            Ins.revins_legal = 0;
        }

        if Ins.moved != kInsNone {
            (*s).count = 0;
        }

        if Ins.update_start_orig {
            Ins.start_orig = Ins.start;
        }

        if !(*curbuf).terminal.is_null() && !Ins.stop_insert_mode {
            Ins.stop_insert_mode = true;
            restart_edit = b'I' as c_int;
            stuffcharReadbuff(K_NOP);
        }

        if Ins.stop_insert_mode && !ins_compl_active() {
            (*s).count = 0;
            return 0;
        }

        if Ins.moved == kInsNone {
            (*curwin).w_set_curswant = true;
        }

        if stuff_empty() {
            did_check_timestamps = false;
            if need_check_timestamps {
                check_timestamps(0);
            }
        }

        msg_scroll = 0;

        if (fdo_flags & kOptFdoFlagInsert) != 0 {
            foldOpenCursor();
        }

        if !char_avail() {
            foldCheckClose();
        }

        if bt_prompt(curbuf) {
            init_prompt((*s).cmdchar_todo);
            (*s).cmdchar_todo = NUL as c_int;
        }

        if (*curbuf).b_mod_set
            && (*curwin).w_onebuf_opt.wo_wrap != 0
            && (*curwin).w_onebuf_opt.wo_sms == 0
            && !(*s).did_backspace
            && (*curwin).w_topline == (*s).old_topline
            && (*curwin).w_topfill == (*s).old_topfill
            && (*s).count <= 1
        {
            (*s).mincol = (*curwin).w_wcol;
            validate_cursor_col(curwin);

            if (*curwin).w_wcol
                < (*s).mincol
                    - tabstop_at(
                        get_nolist_virtcol(),
                        (*curbuf).b_p_ts,
                        (*curbuf).b_p_vts_array,
                        false,
                    )
                && (*curwin).w_wrow
                    == (*curwin).w_view_height - 1 - (get_scrolloff_value(curwin) as c_int)
                && ((*curwin).w_cursor.lnum != (*curwin).w_topline || (*curwin).w_topfill > 0)
            {
                if (*curwin).w_topfill > 0 {
                    (*curwin).w_topfill -= 1;
                } else if hasFolding(
                    curwin,
                    (*curwin).w_topline,
                    ptr::null_mut(),
                    &raw mut (*s).old_topline,
                ) {
                    set_topline(curwin, (*s).old_topline + 1);
                } else {
                    set_topline(curwin, (*curwin).w_topline + 1);
                }
            }
        }

        if (*s).count <= 1 {
            update_topline(curwin);
        }

        (*s).did_backspace = false;

        if (*s).count <= 1 {
            validate_cursor(curwin);
        }

        ins_redraw(true);

        if (*curwin).w_onebuf_opt.wo_scb != 0 {
            do_check_scrollbind(true);
        }

        if (*curwin).w_onebuf_opt.wo_crb != 0 {
            do_check_cursorbind();
        }

        if (*s).count <= 1 {
            update_curswant();
        }
        (*s).old_topline = (*curwin).w_topline;
        (*s).old_topfill = (*curwin).w_topfill;

        if (*s).c != K_EVENT && (*s).c != K_COMPLETE_DELAY {
            (*s).lastc = (*s).c;
        }

        if Ins.dont_sync_undo == kNone {
            Ins.dont_sync_undo = kTrue;
        } else {
            Ins.dont_sync_undo = kFalse;
        }

        if (*s).ins_just_started {
            (*s).ins_just_started = false;
            if ins_compl_has_autocomplete() && !char_avail() && (*curwin).w_cursor.col > 0 {
                (*s).c = char_before_cursor();
                if vim_isprintc((*s).c) {
                    ins_compl_enable_autocomplete();
                    ins_compl_arm_autostart();
                    ins_compl_init_get_longest();
                    if !ins_compl_arm_autocomplete_delay() {
                        insert_do_complete(s);
                        insert_handle_key_post(s);
                        return 1;
                    }
                }
            }
        }

        1
    }
}

unsafe extern "C" fn insert_execute(state: *mut VimState, key: c_int) -> c_int {
    unsafe {
        let s = state.cast::<InsertState>();
        if Ins.stop_insert_mode {
            if key != K_IGNORE && key != K_NOP {
                vungetc(key);
            }
            (*s).count = 0;
            (*s).nomove = true;
            ins_compl_prep(ESC);
            return 0;
        }

        if key == K_IGNORE || key == K_NOP {
            return -1;
        }
        (*s).c = key;

        if key != K_EVENT {
            did_cursorhold = true;
        }
        if key != K_EVENT && key != K_COMPLETE_DELAY {
            ins_compl_clear_autocomplete_delay();
            ins_compl_disarm_autostart();
            if !ins_compl_active() {
                ins_compl_disable_autocomplete();
            }
        }

        if ins_compl_active()
            && (*curwin).w_cursor.col >= ins_compl_col()
            && ins_compl_has_shown_match()
            && pum_wanted()
        {
            if ((*s).c == K_BS || (*s).c == Ctrl_H) && (*curwin).w_cursor.col > ins_compl_col() {
                (*s).c = ins_compl_bs();
                if (*s).c == NUL as c_int {
                    return 1;
                }
            }

            if !ins_compl_used_match() {
                if (*s).c == Ctrl_L && (!ctrl_x_mode_line_or_eval() || ins_compl_long_shown_match())
                {
                    ins_compl_addfrommatch();
                    return 1;
                }

                let is_commit = ins_compl_commit_char((*s).c);
                if !is_commit && ins_compl_accept_char((*s).c) {
                    let str_ = do_insert_char_pre((*s).c);
                    if !str_.is_null() {
                        let mut p = str_;
                        while *p != NUL {
                            ins_compl_addleader(utf_ptr2char(p));
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                        xfree(str_.cast());
                    } else {
                        ins_compl_addleader((*s).c);
                    }
                    return 1;
                }

                if ((*s).c == Ctrl_Y
                    || is_commit
                    || (ins_compl_enter_selects()
                        && ((*s).c == CAR || (*s).c == K_KENTER || (*s).c == NL)))
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
                } else if ascii_iswhite_nl_or_nul((*s).c) && ins_compl_preinsert_effect() {
                    ins_compl_delete(false);
                }
            }
        }

        ins_compl_init_get_longest();
        if ins_compl_prep((*s).c) {
            return 1;
        }

        if (*s).c == Ctrl_BSL {
            ins_redraw(false);
            no_mapping += 1;
            allow_keys += 1;
            (*s).c = plain_vgetc();
            no_mapping -= 1;
            allow_keys -= 1;
            if (*s).c != Ctrl_N && (*s).c != Ctrl_G && (*s).c != Ctrl_O {
                vungetc((*s).c);
                (*s).c = Ctrl_BSL;
            } else {
                if (*s).c == Ctrl_O {
                    ins_ctrl_o();
                    ins_at_eol = false;
                    (*s).nomove = true;
                }
                (*s).count = 0;
                return 0;
            }
        }

        if (*s).c != K_EVENT {
            (*s).c = do_digraph((*s).c);
        }

        if ((*s).c == Ctrl_V || (*s).c == Ctrl_Q) && ctrl_x_mode_cmdline() {
            insert_do_complete(s);
            insert_handle_key_post(s);
            return 1;
        }

        if (*s).c == Ctrl_V || (*s).c == Ctrl_Q {
            ins_ctrl_v();
            (*s).c = Ctrl_V;
            return 1;
        }

        if cindent_on() && ctrl_x_mode_none() {
            (*s).line_is_white = inindent(0);
            if in_cinkeys((*s).c, b'!' as c_int, (*s).line_is_white) && stop_arrow() == OK {
                do_c_expr_indent();
                return 1;
            }
            if Ins.can_cindent
                && in_cinkeys((*s).c, b'*' as c_int, (*s).line_is_white)
                && stop_arrow() == OK
            {
                do_c_expr_indent();
            }
        }

        if (*curwin).w_onebuf_opt.wo_rl != 0 {
            (*s).c = match (*s).c {
                K_LEFT => K_RIGHT,
                K_S_LEFT => K_S_RIGHT,
                K_C_LEFT => K_C_RIGHT,
                K_RIGHT => K_LEFT,
                K_S_RIGHT => K_S_LEFT,
                K_C_RIGHT => K_C_LEFT,
                other => other,
            };
        }

        if ins_start_select((*s).c) {
            return 1;
        }

        insert_handle_key(s)
    }
}

unsafe fn insert_handle_key(s: *mut InsertState) -> c_int {
    unsafe {
        let mut goto_normalchar = false;
        let mut goto_do_complete = false;

        match (*s).c {
            ESC => {
                if !echeck_abbr(ESC + ABBR_OFF) {
                    return 0;
                }
            }
            Ctrl_C => {
                if bt_cmdwin(curbuf) {
                    got_int = false;
                    cmdwin_do_action(c"cancel".as_ptr());
                    return 0;
                }
                if bt_prompt(curbuf) && invoke_prompt_interrupt() {
                    if !bt_prompt(curbuf) {
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
                    insert_do_complete(s);
                } else if !echeck_abbr(Ctrl_O + ABBR_OFF) {
                    ins_ctrl_o();
                    if (get_ve_flags(curwin) & kOptVeFlagOnemore) != 0 {
                        ins_at_eol = false;
                        (*s).nomove = true;
                    }
                    (*s).count = 0;
                    return 0;
                }
            }
            K_INS | K_KINS => {
                ins_insert((*s).replaceState);
            }
            K_SELECT | K_IGNORE => {}
            K_HELP | K_F1 | K_XF1 => {
                stuffcharReadbuff(K_HELP);
                return 0;
            }
            c if c == b' ' as c_int && mod_mask != MOD_MASK_CTRL => {
                goto_normalchar = true;
            }
            c if c == b' ' as c_int || c == K_ZERO || c == NUL as c_int || c == Ctrl_A => {
                if stuff_inserted(NUL as c_int, 1, c_int::from((*s).c == Ctrl_A)) == FAIL
                    && (*s).c != Ctrl_A
                {
                    return 0;
                }
                (*s).inserted_space = 0;
            }
            Ctrl_R => {
                if ctrl_x_mode_register() && !ins_compl_active() {
                    insert_do_complete(s);
                } else {
                    ins_reg();
                    auto_format(false, true);
                    (*s).inserted_space = 0;
                }
            }
            Ctrl_G => {
                ins_ctrl_g();
            }
            Ctrl_HAT => {
                ins_ctrl_hat();
            }
            Ctrl__ => {
                if p_ari == 0 {
                    goto_normalchar = true;
                } else {
                    ins_ctrl_();
                }
            }
            Ctrl_D => {
                if ctrl_x_mode_path_defines() {
                    insert_do_complete(s);
                } else {
                    ins_shift((*s).c, (*s).lastc);
                    auto_format(false, true);
                    (*s).inserted_space = 0;
                }
            }
            Ctrl_T => {
                if ctrl_x_mode_thesaurus() {
                    if check_compl_option(false) {
                        insert_do_complete(s);
                    }
                } else {
                    ins_shift((*s).c, (*s).lastc);
                    auto_format(false, true);
                    (*s).inserted_space = 0;
                }
            }
            K_DEL | K_KDEL => {
                ins_del();
                auto_format(false, true);
            }
            K_BS | Ctrl_H => {
                (*s).did_backspace = ins_bs((*s).c, BACKSPACE_CHAR, &raw mut (*s).inserted_space);
                auto_format(false, true);
                if (*s).did_backspace {
                    may_trigger_autocomplete(s);
                }
            }
            Ctrl_W => {
                if bt_prompt(curbuf) && (mod_mask & MOD_MASK_SHIFT) == 0 {
                    stuffcharReadbuff(Ctrl_W);
                    restart_edit = b'A' as c_int;
                    (*s).nomove = true;
                    (*s).count = 0;
                    return 0;
                }
                (*s).did_backspace = ins_bs((*s).c, BACKSPACE_WORD, &raw mut (*s).inserted_space);
                auto_format(false, true);
                if (*s).did_backspace {
                    may_trigger_autocomplete(s);
                }
            }
            Ctrl_U => {
                if ctrl_x_mode_function() {
                    insert_do_complete(s);
                } else {
                    (*s).did_backspace =
                        ins_bs((*s).c, BACKSPACE_LINE, &raw mut (*s).inserted_space);
                    auto_format(false, true);
                    (*s).inserted_space = 0;
                    if (*s).did_backspace {
                        may_trigger_autocomplete(s);
                    }
                }
            }
            K_LEFTMOUSE | K_LEFTMOUSE_NM | K_LEFTDRAG | K_LEFTRELEASE | K_LEFTRELEASE_NM
            | K_MOUSEMOVE | K_MIDDLEMOUSE | K_MIDDLEDRAG | K_MIDDLERELEASE | K_RIGHTMOUSE
            | K_RIGHTDRAG | K_RIGHTRELEASE | K_X1MOUSE | K_X1DRAG | K_X1RELEASE | K_X2MOUSE
            | K_X2DRAG | K_X2RELEASE => {
                ins_mouse((*s).c);
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
                let tick = buf_get_changedtick(curbuf);
                set_bufref(&raw mut save_curbuf, curbuf);

                if (*s).c == K_PASTE_START {
                    paste_repeat(1);
                } else if (*s).c == K_EVENT {
                    state_handle_k_event();
                    if Ins.dont_sync_undo == kTrue {
                        Ins.dont_sync_undo = kNone;
                    }
                } else if (*s).c == K_COMMAND {
                    do_cmdline(ptr::null_mut(), Some(getcmdkeycmd), ptr::null_mut(), 0);
                } else {
                    map_execute_lua(false, false);
                }

                if pum_want.active {
                    if pum_visible() {
                        edit_submode_extra = ptr::null_mut();
                        insert_do_complete(s);
                        if pum_want.finish {
                            ins_compl_prep(Ctrl_Y);
                        }
                    }
                    pum_want.active = false;
                }

                let changed = bufref_valid(&raw mut save_curbuf)
                    && curbuf == save_curbuf.br_buf
                    && tick != buf_get_changedtick(curbuf);
                if (*curbuf).b_u_synced || changed {
                    Ins.need_undo = true;
                }
                if changed && !ins_compl_active() {
                    mc_ins_cascade();
                    mc_ins_cascade_restart();
                }
            }
            K_COMPLETE_DELAY => {
                if Ins.dont_sync_undo == kTrue {
                    Ins.dont_sync_undo = kNone;
                }
                ins_compl_clear_autocomplete_delay();
                if ins_compl_has_autocomplete() && !char_avail() && (*curwin).w_cursor.col != 0 {
                    (*s).c = char_before_cursor();
                    if vim_isprintc((*s).c) {
                        ins_compl_enable_autocomplete();
                        ins_compl_arm_autostart();
                        insert_do_complete(s);
                    }
                }
            }
            K_HOME | K_KHOME | K_S_HOME | K_C_HOME => {
                ins_home((*s).c);
            }
            K_END | K_KEND | K_S_END | K_C_END => {
                ins_end((*s).c);
            }
            K_LEFT => {
                if (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) != 0 {
                    ins_s_left();
                } else {
                    ins_left();
                }
            }
            K_S_LEFT | K_C_LEFT => {
                ins_s_left();
            }
            K_RIGHT => {
                if (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) != 0 {
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
                    insert_do_complete(s);
                } else if (mod_mask & MOD_MASK_SHIFT) != 0 {
                    ins_pageup();
                } else {
                    ins_up(false);
                }
            }
            K_S_UP | K_PAGEUP | K_KPAGEUP => {
                if pum_visible() {
                    insert_do_complete(s);
                } else {
                    ins_pageup();
                }
            }
            K_DOWN => {
                if pum_visible() {
                    insert_do_complete(s);
                } else if (mod_mask & MOD_MASK_SHIFT) != 0 {
                    ins_pagedown();
                } else {
                    ins_down(false);
                }
            }
            K_S_DOWN | K_PAGEDOWN | K_KPAGEDOWN => {
                if pum_visible() {
                    insert_do_complete(s);
                } else {
                    ins_pagedown();
                }
            }
            K_S_TAB | TAB => {
                if (*s).c == K_S_TAB {
                    (*s).c = TAB;
                }
                if ctrl_x_mode_path_patterns() {
                    insert_do_complete(s);
                } else {
                    (*s).inserted_space = 0;
                    if ins_tab() {
                        goto_normalchar = true;
                    } else {
                        auto_format(false, true);
                    }
                }
            }
            K_KENTER | CAR | NL => {
                if (*s).c == K_KENTER {
                    (*s).c = CAR;
                }
                if bt_quickfix(curbuf) && (*s).c == CAR {
                    if (*curwin).w_llist_ref.is_null() {
                        do_cmdline_cmd(c".cc".as_ptr());
                    } else {
                        do_cmdline_cmd(c".ll".as_ptr());
                    }
                } else if (*s).c == CAR && bt_cmdwin(curbuf) {
                    cmdwin_do_action(c"confirm".as_ptr());
                    return 0;
                } else if (mod_mask & MOD_MASK_SHIFT) == 0 && bt_prompt(curbuf) {
                    prompt_invoke_callback();
                    if !bt_prompt(curbuf) {
                        return 0;
                    }
                } else {
                    if !ins_eol((*s).c) {
                        return 0;
                    }
                    auto_format(false, false);
                    (*s).inserted_space = 0;
                }
            }
            Ctrl_K => {
                if ctrl_x_mode_dictionary() {
                    if check_compl_option(true) {
                        insert_do_complete(s);
                    }
                } else {
                    (*s).c = ins_digraph();
                    if (*s).c != NUL as c_int {
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
                    insert_do_complete(s);
                }
            }
            Ctrl_F => {
                if !ctrl_x_mode_files() {
                    goto_normalchar = true;
                } else {
                    insert_do_complete(s);
                }
            }
            c if c == b's' as c_int || c == Ctrl_S => {
                if !ctrl_x_mode_spell() {
                    goto_normalchar = true;
                } else {
                    insert_do_complete(s);
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
                (*s).c = ins_ctrl_ey((*s).c);
            }
            _ => {
                goto_normalchar = true;
            }
        }

        if goto_do_complete {
            if *(*curbuf).b_p_cpt == NUL
                && (ctrl_x_mode_normal() || ctrl_x_mode_whole_line())
                && !compl_status_local()
            {
                goto_normalchar = true;
            } else {
                insert_do_complete(s);
            }
        }

        if goto_normalchar {
            let mut skip_char = false;
            if p_paste == 0 {
                let str_ = do_insert_char_pre((*s).c);
                if !str_.is_null() {
                    if *str_ != NUL && stop_arrow() != FAIL {
                        let mut p = str_;
                        while *p != NUL {
                            (*s).c = utf_ptr2char(p);
                            if (*s).c == CAR || (*s).c == K_KENTER || (*s).c == NL {
                                ins_eol((*s).c);
                            } else {
                                ins_char((*s).c);
                            }
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                        redo_append_lit(str_, -1);
                    }
                    xfree(str_.cast());
                    (*s).c = NUL as c_int;
                }
                if (*s).c == NUL as c_int {
                    skip_char = true;
                }
            }

            if !skip_char {
                ins_try_si((*s).c);

                if (*s).c == b' ' as c_int {
                    (*s).inserted_space = 1;
                    if inindent(0) {
                        Ins.can_cindent = false;
                    }
                    if Ins.start_blank_vcol == MAXCOL && (*curwin).w_cursor.lnum == Ins.start.lnum {
                        Ins.start_blank_vcol = get_nolist_virtcol();
                    }
                }

                if vim_iswordc((*s).c)
                    || (!echeck_abbr(if (*s).c >= 0x100 {
                        (*s).c + ABBR_OFF
                    } else {
                        (*s).c
                    }) && (*s).c != Ctrl_RSB)
                {
                    insert_special((*s).c, 0, 0);
                    Ins.revins_legal += 1;
                    Ins.revins_chars += 1;
                }

                auto_format(false, true);
                foldOpenCursor();
                if ins_compl_has_autocomplete() && !char_avail() && vim_isprintc((*s).c) {
                    let _ = trigger_autocomplete(s);
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
        compl_busy = true;
        disable_fold_update += 1;
        if ins_complete((*s).c, true) == FAIL {
            compl_status_clear();
        }
        disable_fold_update -= 1;
        compl_busy = false;
        Ins.can_si = may_do_si();
    }
}

unsafe fn insert_do_cindent(s: *mut InsertState) {
    unsafe {
        if in_cinkeys((*s).c, b' ' as c_int, (*s).line_is_white) && stop_arrow() == OK {
            do_c_expr_indent();
        }
    }
}

unsafe fn insert_handle_key_post(s: *mut InsertState) {
    unsafe {
        if (*s).c != K_EVENT && (*s).c != K_COMPLETE_DELAY && ctrl_x_mode_normal() {
            did_cursorhold = false;
        }

        if ins_compl_active() && !ins_compl_win_active(curwin) {
            ins_compl_cancel();
        }

        if Ins.moved != kInsNone {
            (*s).inserted_space = 0;
        }

        if Ins.can_cindent && cindent_on() && ctrl_x_mode_normal() {
            insert_do_cindent(s);
        }
    }
}

/// Start inserting text.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn edit(cmdchar: c_int, startln: bool, count: c_int) -> bool {
    unsafe {
        if !(*curbuf).terminal.is_null() {
            if ex_normal_busy != 0 {
                restart_edit = b'i' as c_int;
                force_restart_edit = true;
                return false;
            }
            return terminal_enter();
        }

        if sandbox != 0 {
            emsg(gettext(e_sandbox.as_ptr()));
            return false;
        }

        if textlock != 0 || ins_compl_active() || compl_busy || pum_visible() || expr_map_locked() {
            emsg(gettext(e_textlock.as_ptr()));
            return false;
        }

        let mut s = std::mem::zeroed::<InsertState>();
        s.state.execute = Some(insert_execute);
        s.state.check = Some(insert_check);
        s.cmdchar = cmdchar;
        s.startln = c_int::from(startln);
        s.count = count;
        insert_enter(&raw mut s);
        s.c == Ctrl_O
    }
}

/// Redraw for Insert mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_redraw(ready: bool) {
    unsafe {
        if char_avail() {
            return;
        }

        if ready
            && has_event(EVENT_CURSORMOVEDI)
            && (last_cursormoved_win != curwin || !equalpos(last_cursormoved, (*curwin).w_cursor))
            && !pum_visible()
        {
            if syntax_present(curwin) && must_redraw != 0 {
                update_screen();
            }
            update_curswant();
            ins_apply_autocmds(EVENT_CURSORMOVEDI);
            last_cursormoved_win = curwin;
            last_cursormoved = (*curwin).w_cursor;
        }

        if ready
            && has_event(EVENT_TEXTCHANGEDI)
            && (*curbuf).b_last_changedtick_i != buf_get_changedtick(curbuf)
            && !pum_visible()
        {
            let mut aco = std::mem::zeroed::<CtxSwitch>();
            let tick = buf_get_changedtick(curbuf);
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), curbuf, 0);
            apply_autocmds(
                EVENT_TEXTCHANGEDI,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            ctx_restore(&raw mut aco);
            (*curbuf).b_last_changedtick_i = buf_get_changedtick(curbuf);
            if tick != buf_get_changedtick(curbuf) {
                u_save((*curwin).w_cursor.lnum, (*curwin).w_cursor.lnum + 1);
            }
        }

        if ready
            && has_event(EVENT_TEXTCHANGEDP)
            && (*curbuf).b_last_changedtick_pum != buf_get_changedtick(curbuf)
            && pum_visible()
        {
            let mut aco = std::mem::zeroed::<CtxSwitch>();
            let tick = buf_get_changedtick(curbuf);
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), curbuf, 0);
            apply_autocmds(
                EVENT_TEXTCHANGEDP,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            ctx_restore(&raw mut aco);
            (*curbuf).b_last_changedtick_pum = buf_get_changedtick(curbuf);
            if tick != buf_get_changedtick(curbuf) {
                u_save((*curwin).w_cursor.lnum, (*curwin).w_cursor.lnum + 1);
            }
        }

        if ready {
            may_trigger_win_scrolled_resized();
        }

        may_trigger_safestate(ready && !ins_compl_active() && !pum_visible());

        pum_check_clear();
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
        emsg_on_display = false;
    }
}

unsafe fn ins_ctrl_v() {
    unsafe {
        let mut did_putchar = false;

        ins_redraw(false);

        if redrawing() && !char_avail() {
            edit_putchar(b'^' as c_int, true);
            did_putchar = true;
        }
        redo_append_str(c"\x16".as_ptr(), 1);

        add_to_showcmd_c(Ctrl_V);

        let c = get_literal((mod_mask & MOD_MASK_SHIFT) != 0);
        if did_putchar {
            edit_unputchar();
        }
        clear_showcmd();
        insert_special(c, 1, 1);
        Ins.revins_chars += 1;
        Ins.revins_legal += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn edit_putchar(c: c_int, highlight: bool) {
    unsafe {
        if (*curwin).w_grid_alloc.chars.is_null() && default_grid.chars.is_null() {
            return;
        }

        update_topline(curwin);
        validate_cursor(curwin);
        let attr = if highlight {
            *hl_attr_active.add(HLF_8)
        } else {
            0
        };
        pc_row = (*curwin).w_wrow;
        pc_status = PC_STATUS_UNSET;
        grid_line_start(&raw mut (*curwin).w_grid, pc_row);
        if (*curwin).w_onebuf_opt.wo_rl != 0 {
            pc_col = (*curwin).w_view_width - 1 - (*curwin).w_wcol;
            if grid_line_getchar(pc_col, ptr::null_mut()) == 0 {
                grid_line_put_schar(pc_col - 1, schar_from_ascii(b' '), attr);
                (*curwin).w_wcol -= 1;
                pc_status = PC_STATUS_RIGHT;
            }
        } else {
            pc_col = (*curwin).w_wcol;
            if grid_line_getchar(pc_col + 1, ptr::null_mut()) == 0 {
                pc_status = PC_STATUS_LEFT;
            }
        }

        if pc_status == PC_STATUS_UNSET {
            pc_schar = grid_line_getchar(pc_col, &raw mut pc_attr);
            pc_status = PC_STATUS_SET;
        }

        let mut buf = [0 as c_char; MB_MAXCHAR + 1];
        let len = utf_char2bytes(c, (&raw mut buf).cast());
        grid_line_puts(pc_col, (&raw const buf).cast(), len, attr);
        grid_line_flush();
    }
}

/// Return the effective prompt for the specified buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_prompt_text(buf: *const buf_T) -> *mut c_char {
    unsafe {
        if (*buf).b_prompt_text.is_null() {
            c"% ".as_ptr().cast_mut()
        } else {
            (*buf).b_prompt_text
        }
    }
}

/// Return the effective prompt for the current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prompt_text() -> *mut c_char {
    unsafe { buf_prompt_text(curbuf) }
}

unsafe fn init_prompt(cmdchar_todo: c_int) {
    unsafe {
        let prompt = prompt_text();
        let prompt_len = c_strlen(prompt) as colnr_T;

        if (*curbuf).b_prompt_start.mark.lnum < 1
            || (*curbuf).b_prompt_start.mark.lnum > (*curbuf).b_ml.ml_line_count
        {
            (*curbuf).b_prompt_start.mark.lnum = (*curbuf)
                .b_prompt_start
                .mark
                .lnum
                .min((*curbuf).b_ml.ml_line_count)
                .max(1);
            (*curbuf).b_prompt_append_new_line = true;
        }

        (*curwin).w_cursor.lnum = (*curwin)
            .w_cursor
            .lnum
            .max((*curbuf).b_prompt_start.mark.lnum);
        let text = ml_get((*curbuf).b_prompt_start.mark.lnum);
        let text_len = ml_get_len((*curbuf).b_prompt_start.mark.lnum);

        if (*curbuf).b_prompt_start.mark.lnum == (*curwin).w_cursor.lnum
            && ((*curbuf).b_prompt_start.mark.col < prompt_len
                || (*curbuf).b_prompt_start.mark.col > text_len
                || !strnequal(
                    text.offset(((*curbuf).b_prompt_start.mark.col - prompt_len) as isize),
                    prompt,
                    prompt_len as usize,
                ))
        {
            if *text == NUL {
                ml_replace((*curbuf).b_prompt_start.mark.lnum, prompt, true);
                inserted_bytes((*curbuf).b_prompt_start.mark.lnum, 0, 0, prompt_len);
            } else {
                let lnum = (*curbuf).b_ml.ml_line_count;
                ml_append(lnum, prompt, 0, false);
                appended_lines_mark(lnum, 1);
                (*curbuf).b_prompt_start.mark.lnum = (*curbuf).b_ml.ml_line_count;
                (*curbuf).b_prompt_append_new_line = true;
                u_clearallandblockfree(curbuf);
            }
            (*curbuf).b_prompt_start.mark.col = prompt_len;
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
            coladvance(curwin, MAXCOL);
        }

        if Ins.start_orig.lnum != (*curbuf).b_prompt_start.mark.lnum
            || Ins.start_orig.col != (*curbuf).b_prompt_start.mark.col
        {
            set_insstart(
                (*curbuf).b_prompt_start.mark.lnum,
                (*curbuf).b_prompt_start.mark.col,
            );
        }

        if cmdchar_todo == b'A' as c_int {
            coladvance(curwin, MAXCOL);
        }
        if (*curbuf).b_prompt_start.mark.lnum == (*curwin).w_cursor.lnum {
            (*curwin).w_cursor.col = (*curwin)
                .w_cursor
                .col
                .max((*curbuf).b_prompt_start.mark.col);
        }
        check_cursor(curwin);
    }
}

/// Return true if the cursor is in the editable position of the prompt line.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prompt_curpos_editable() -> bool {
    unsafe {
        (*curwin).w_cursor.lnum > (*curbuf).b_prompt_start.mark.lnum
            || ((*curwin).w_cursor.lnum == (*curbuf).b_prompt_start.mark.lnum
                && (*curwin).w_cursor.col >= (*curbuf).b_prompt_start.mark.col)
    }
}

unsafe fn set_insstart(lnum: linenr_T, col: colnr_T) {
    unsafe {
        Ins.start.lnum = lnum;
        Ins.start.col = col;
        Ins.start_orig = Ins.start;
        Ins.start_textlen = Ins.start.col;
        Ins.start_blank_vcol = MAXCOL;
        Ins.moved = kInsNone;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn edit_unputchar() {
    unsafe {
        if pc_status != PC_STATUS_UNSET && pc_row >= 0 {
            if pc_status == PC_STATUS_RIGHT {
                (*curwin).w_wcol += 1;
            }
            if pc_status == PC_STATUS_RIGHT || pc_status == PC_STATUS_LEFT {
                redrawWinline(curwin, (*curwin).w_cursor.lnum);
            } else {
                grid_line_start(&raw mut (*curwin).w_grid, pc_row);
                grid_line_put_schar(pc_col, pc_schar, pc_attr);
                grid_line_flush();
            }
        }
    }
}

/// Display a `$` at the end of changed text when `$` is in `cpoptions`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn display_dollar(col_arg: colnr_T) {
    unsafe {
        let col = col_arg.max(0);
        if !redrawing() {
            return;
        }
        let save_col = (*curwin).w_cursor.col;
        (*curwin).w_cursor.col = col;

        let p = get_cursor_line_ptr();
        (*curwin).w_cursor.col -= utf_head_off(p, p.offset(col as isize));
        curs_columns(curwin, 0);
        if (*curwin).w_wcol < (*curwin).w_view_width {
            edit_putchar(b'$' as c_int, false);
            dollar_vcol = (*curwin).w_virtcol;
        }
        (*curwin).w_cursor.col = save_col;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn undisplay_dollar() {
    unsafe {
        if dollar_vcol < 0 {
            return;
        }
        dollar_vcol = -1;
        redrawWinline(curwin, (*curwin).w_cursor.lnum);
    }
}

/// Truncate the space at the end of a line.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn truncate_spaces(line: *mut c_char, len: usize) {
    unsafe {
        let mut i = len as isize - 1;
        while i >= 0 && ascii_iswhite(*line.offset(i) as u8 as c_int) {
            if (State & REPLACE_FLAG) != 0 {
                replace_join(0);
            }
            i -= 1;
        }
        *line.offset(i + 1) = NUL;
    }
}

/// Backspace the cursor until the given column.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn backspace_until_column(col: c_int) {
    unsafe {
        while (*curwin).w_cursor.col > col {
            (*curwin).w_cursor.col -= 1;
            if (State & REPLACE_FLAG) != 0 {
                replace_do_bs(col);
            } else if !del_char_after_col(col) {
                break;
            }
        }
    }
}

unsafe fn del_char_after_col(limit_col: c_int) -> bool {
    unsafe {
        if limit_col >= 0 {
            let ecol = (*curwin).w_cursor.col + 1;
            mb_adjust_cursor();
            while (*curwin).w_cursor.col < limit_col {
                let l = utf_ptr2len(get_cursor_pos_ptr());
                if l == 0 {
                    break;
                }
                (*curwin).w_cursor.col += l;
            }
            if *get_cursor_pos_ptr() == NUL || (*curwin).w_cursor.col == ecol {
                return false;
            }
            del_bytes(ecol - (*curwin).w_cursor.col, false, true);
        } else {
            del_char(false);
        }
        true
    }
}

/// Next character is interpreted literally.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_literal(no_simplify: bool) -> c_int {
    unsafe {
        let mut nc: c_int;
        let mut hex = false;
        let mut octal = false;
        let mut unicode: c_int = 0;

        if got_int {
            return Ctrl_C;
        }

        no_mapping += 1;
        let mut cc: c_int = 0;
        let mut i: c_int = 0;
        loop {
            nc = plain_vgetc();
            if !no_simplify {
                nc = merge_modifiers(nc, &raw mut mod_mask);
            }
            if (mod_mask & !MOD_MASK_SHIFT) != 0 {
                break;
            }
            if (State & MODE_CMDLINE) == 0 && mb_byte2len_check(nc) == 1 {
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

        no_mapping -= 1;
        if nc != 0 {
            vungetc(nc);
            mod_mask = 0;
        }
        got_int = false;
        cc
    }
}

unsafe fn insert_special(mut c: c_int, mut allow_modmask: c_int, mut ctrlv: c_int) {
    unsafe {
        if (mod_mask & MOD_MASK_CMD) != 0 {
            allow_modmask = 1;
        }
        if IS_SPECIAL(c) || (mod_mask != 0 && allow_modmask != 0) {
            let p = get_special_key_name(c, mod_mask);
            let len = c_strlen(p);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn insertchar(mut c: c_int, flags: c_int, second_indent: c_int) {
    unsafe {
        let force_format = (flags & INSCHAR_FORMAT) != 0;
        let textwidth = comp_textwidth(force_format);
        let fo_ins_blank = has_format_option(kFoInsBlank);

        if textwidth > 0
            && (force_format
                || (!ascii_iswhite(c)
                    && !((State & REPLACE_FLAG) != 0
                        && (State & VREPLACE_FLAG) == 0
                        && *get_cursor_pos_ptr() != NUL)
                    && ((*curwin).w_cursor.lnum != Ins.start.lnum
                        || ((!has_format_option(kFoInsLong) || Ins.start_textlen <= textwidth)
                            && (!fo_ins_blank || Ins.start_blank_vcol <= textwidth)))))
        {
            let mut do_internal = true;
            let virtcol = get_nolist_virtcol()
                + char2cells(if c != NUL as c_int { c } else { gchar_cursor() });

            if (*curbuf).b_p_fex.cb_type != kCallbackNone
                && (flags & INSCHAR_NO_FEX) == 0
                && (force_format || virtcol > textwidth)
            {
                do_internal = fex_format((*curwin).w_cursor.lnum, 1, c) != 0;
                Ins.need_undo = true;
            }
            if do_internal {
                internal_format(textwidth, second_indent, flags, c == NUL as c_int, c);
            }
        }

        if c == NUL as c_int {
            return;
        }

        if Ins.did_ai && c == Ins.end_comment_pending {
            let mut lead_end = [0 as c_char; COM_MAX_LEN];
            let mut p: *mut c_char = ptr::null_mut();
            let line = get_cursor_line_ptr();
            let mut i = get_leader_len(line, &raw mut p, false, true);
            if i > 0 && !vim_strchr(p, COM_MIDDLE).is_null() {
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

                i = (*curwin).w_cursor.col;
                while {
                    i -= 1;
                    i >= 0 && ascii_iswhite(*line.add(i as usize) as u8 as c_int)
                } {}
                i += 1;
                i -= middle_len;

                if i >= 0
                    && end_len > 0
                    && (lead_end[(end_len - 1) as usize] as u8 as c_int) == Ins.end_comment_pending
                {
                    backspace_until_column(i);
                    ins_bytes_len((&raw mut lead_end).cast(), (end_len - 1) as usize);
                }
            }
        }
        Ins.end_comment_pending = NUL as c_int;

        Ins.did_ai = false;
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;

        if !is_special_ins(c)
            && utf_char2len(c) == 1
            && !has_event(EVENT_INSERTCHARPRE)
            && !test_disable_char_avail
            && vpeekc() != NUL as c_int
            && (State & REPLACE_FLAG) == 0
            && !cindent_on()
            && p_ri == 0
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
                    && mb_byte2len(c) == 1
                    && i < INPUT_BUFLEN
                    && (textwidth == 0 || {
                        virtcol += ascii2cells(buf[i - 1] as u8 as c_int);
                        virtcol < textwidth
                    })
                    && !(!no_abbr && !vim_iswordc(c) && vim_iswordc(buf[i - 1] as u8 as c_int))
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

unsafe fn redo_literal(c: c_int) {
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn start_arrow(
    end_insert_pos: *mut pos_T,
    end_change: bool,
    move_key: c_int,
) {
    unsafe {
        if Ins.moved == kInsNone && end_change {
            stop_insert(end_insert_pos, 0, 0);
        }
        if Ins.moved != kInsJump {
            if move_key == NUL as c_int {
                redo_append_str(c"\x1b".as_ptr(), 1);
                Ins.moved = kInsJump;
            } else {
                if !IS_SPECIAL(move_key) {
                    redo_append_char(Ctrl_G);
                }
                redo_append_char(move_key);
                if end_change {
                    Ins.moved = kInsArrow;
                }
            }
        }
        check_spell_redraw();
    }
}

unsafe fn start_arrow_with_change(end_insert_pos: *mut pos_T, end_change: bool, move_key: c_int) {
    unsafe {
        if !end_change && Ins.moved != kInsJump {
            redo_append_char(Ctrl_G);
            redo_append_char(b'U' as c_int);
        }
        start_arrow(end_insert_pos, end_change, move_key);
    }
}

unsafe fn check_spell_redraw() {
    unsafe {
        if spell_redraw_lnum != 0 {
            let lnum = spell_redraw_lnum;
            spell_redraw_lnum = 0;
            redrawWinline(curwin, lnum);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stop_arrow() -> c_int {
    unsafe {
        if Ins.moved != kInsNone {
            Ins.start = (*curwin).w_cursor;
            if Ins.start.col > Ins.start_orig.col && !Ins.need_undo {
                Ins.update_start_orig = false;
            }
            Ins.start_textlen = linetabsize_str(get_cursor_line_ptr());

            let jumped = Ins.moved == kInsJump;
            if u_save_cursor() == OK {
                Ins.moved = kInsNone;
                Ins.need_undo = false;
            }
            Ins.ai_col = 0;
            if (State & VREPLACE_FLAG) != 0 {
                orig_line_count = (*curbuf).b_ml.ml_line_count;
                vr_lines_changed = 1;
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
                Ins.new_insert_skip = 2;
                mc_ins_cascade_restart();
            } else {
                let redo = redo_keys(ptr::null());
                Ins.new_insert_skip = redo.size as c_int;
                xfree(redo.data.cast());
            }
        } else if Ins.need_undo && u_save_cursor() == OK {
            if lt((*curwin).w_cursor, Ins.start) {
                Ins.start = (*curwin).w_cursor;
                Ins.start_textlen = linetabsize_str(get_cursor_line_ptr());
            }
            Ins.need_undo = false;
        }

        foldOpenCursor();

        if Ins.moved != kInsNone || Ins.need_undo {
            FAIL
        } else {
            OK
        }
    }
}

unsafe fn stop_insert(end_insert_pos: *mut pos_T, esc: c_int, nomove: c_int) {
    unsafe {
        stop_redo_ins();
        crate::kvec::kv_destroy(&raw mut replace_stack);

        let redo = redo_keys(ptr::null());
        let added = if redo.data.is_null() {
            0
        } else {
            (redo.size as c_int) - Ins.new_insert_skip
        };
        if Ins.did_restart_edit == 0 || added > 0 {
            xfree(last_insert.data.cast());
            last_insert = redo;
            last_insert_skip = if added < 0 { 0 } else { Ins.new_insert_skip };
        } else {
            xfree(redo.data.cast());
        }

        if Ins.moved == kInsNone && !end_insert_pos.is_null() {
            let mut cc: c_int;
            if !Ins.need_undo && has_format_option(kFoAuto) {
                let tpos = (*curwin).w_cursor;
                cc = b'x' as c_int;
                if (*curwin).w_cursor.col > 0 && gchar_cursor() == NUL as c_int {
                    dec_cursor();
                    cc = gchar_cursor();
                    if !ascii_iswhite(cc) {
                        (*curwin).w_cursor = tpos;
                    }
                }

                auto_format(true, false);

                if ascii_iswhite(cc) {
                    if gchar_cursor() != NUL as c_int {
                        inc_cursor();
                    }
                    if gchar_cursor() == NUL as c_int
                        && (*curwin).w_cursor.lnum == tpos.lnum
                        && (*curwin).w_cursor.col == tpos.col
                    {
                        (*curwin).w_cursor.coladd = tpos.coladd;
                    }
                }
            }

            check_auto_format(true);

            if nomove == 0
                && Ins.did_ai
                && !mc_ins_replaying()
                && (esc != 0
                    || (vim_strchr(p_cpo, kCpoIndent).is_null()
                        && (*curwin).w_cursor.lnum != (*end_insert_pos).lnum))
                && (*end_insert_pos).lnum <= (*curbuf).b_ml.ml_line_count
            {
                let mut tpos = (*curwin).w_cursor;
                let prev_col = (*end_insert_pos).col;

                (*curwin).w_cursor = *end_insert_pos;
                check_cursor_col(curwin);

                let mut strip_col = (*curwin).w_cursor.col;
                if gchar_cursor() == NUL as c_int && strip_col > 0 {
                    strip_col -= 1;
                }

                if *skipwhite(get_cursor_line_ptr().offset(strip_col as isize)) == NUL {
                    (*curwin).w_cursor.col = strip_col;
                    loop {
                        if gchar_cursor() == NUL as c_int && (*curwin).w_cursor.col > 0 {
                            (*curwin).w_cursor.col -= 1;
                        }
                        cc = gchar_cursor();
                        if !ascii_iswhite(cc) {
                            break;
                        }
                        if del_char(true) == FAIL {
                            break;
                        }
                    }
                    if (*curwin).w_cursor.lnum != tpos.lnum {
                        (*curwin).w_cursor = tpos;
                    } else if (*curwin).w_cursor.col < prev_col {
                        tpos = (*curwin).w_cursor;
                        tpos.col += 1;
                        if cc != NUL as c_int && gchar_pos(&raw mut tpos) == NUL as c_int {
                            (*curwin).w_cursor.col += 1;
                        }
                    }

                    if Visual.active {
                        check_visual_pos();
                    }
                } else {
                    (*curwin).w_cursor = tpos;
                }
            }
        }
        Ins.did_ai = false;
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;

        if !end_insert_pos.is_null() {
            (*curbuf).b_op_start = Ins.start;
            (*curbuf).b_op_start_orig = Ins.start_orig;
            (*curbuf).b_op_end = *end_insert_pos;
        }
    }
}

/// Set the last inserted text to a single character.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_last_insert(c: c_int) {
    unsafe {
        xfree(last_insert.data.cast());
        last_insert.data = xmalloc(MB_MAXBYTES * 3 + 5).cast();
        let mut s = last_insert.data;
        if c < b' ' as c_int || c == DEL {
            *s = Ctrl_V as c_char;
            s = s.add(1);
        }
        s = add_char2buf(c, s);
        *s = ESC as c_char;
        s = s.add(1);
        *s = NUL;
        last_insert.size = s.offset_from(last_insert.data) as usize;
        last_insert_skip = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_last_insert() {
    unsafe {
        if !last_insert.data.is_null() {
            xfree(last_insert.data.cast());
        }
        last_insert = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn beginline(flags: c_int) {
    unsafe {
        if (flags & BL_SOL) != 0 && p_sol == 0 {
            coladvance(curwin, (*curwin).w_curswant);
        } else {
            (*curwin).w_cursor.col = 0;
            (*curwin).w_cursor.coladd = 0;

            if (flags & (BL_WHITE | BL_SOL)) != 0 {
                let mut ptr = get_cursor_line_ptr();
                while ascii_iswhite(*ptr as u8 as c_int)
                    && !((flags & BL_FIX) != 0 && *ptr.add(1) == NUL)
                {
                    (*curwin).w_cursor.col += 1;
                    ptr = ptr.add(1);
                }
            }
            (*curwin).w_set_curswant = true;
        }
        adjust_skipcol();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn oneright() -> c_int {
    unsafe {
        if virtual_active(curwin) {
            let prevpos = (*curwin).w_cursor;
            let ptr = get_cursor_pos_ptr();
            let step = if *ptr != TAB as c_char && vim_isprintc(utf_ptr2char(ptr)) {
                ptr2cells(ptr)
            } else {
                1
            };
            coladvance(curwin, getviscol() + step);
            (*curwin).w_set_curswant = true;
            return if prevpos.col != (*curwin).w_cursor.col
                || prevpos.coladd != (*curwin).w_cursor.coladd
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
        if *ptr.add(l as usize) == NUL && (get_ve_flags(curwin) & kOptVeFlagOnemore) == 0 {
            return FAIL;
        }
        (*curwin).w_cursor.col += l;
        (*curwin).w_set_curswant = true;
        adjust_skipcol();
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn oneleft() -> c_int {
    unsafe {
        if virtual_active(curwin) {
            let v = getviscol();
            if v == 0 {
                return FAIL;
            }
            let mut width: c_int = 1;
            loop {
                coladvance(curwin, v - width);
                if getviscol() < v {
                    break;
                }
                width += 1;
            }

            if (*curwin).w_cursor.coladd == 1 {
                let ptr = get_cursor_pos_ptr();
                if *ptr != TAB as c_char && vim_isprintc(utf_ptr2char(ptr)) && ptr2cells(ptr) > 1 {
                    (*curwin).w_cursor.coladd = 0;
                }
            }

            (*curwin).w_set_curswant = true;
            adjust_skipcol();
            return OK;
        }

        if (*curwin).w_cursor.col == 0 {
            return FAIL;
        }

        (*curwin).w_set_curswant = true;
        (*curwin).w_cursor.col -= 1;

        mb_adjust_cursor();
        adjust_skipcol();
        OK
    }
}

/// Move the cursor up `n` lines in window `wp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_up_inner(wp: *mut win_T, mut n: linenr_T, skip_conceal: bool) {
    unsafe {
        let mut lnum = (*wp).w_cursor.lnum;

        if n >= lnum {
            lnum = 1;
        } else if win_lines_concealed(wp) {
            hasFolding(wp, lnum, &raw mut lnum, ptr::null_mut());
            while n > 0 {
                n -= 1;
                lnum -= 1;
                if lnum <= 1 {
                    break;
                }
                if skip_conceal && decor_conceal_line(wp, lnum - 1, true) {
                    n += 1;
                }
                if n > 0 || !((State & MODE_INSERT) != 0 || (fdo_flags & kOptFdoFlagAll) != 0) {
                    hasFolding(wp, lnum, &raw mut lnum, ptr::null_mut());
                }
            }
            lnum = lnum.max(1);
        } else {
            lnum -= n;
        }

        (*wp).w_cursor.lnum = lnum;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_up(n: linenr_T, upd_topline: bool) -> c_int {
    unsafe {
        if n > 0 && (*curwin).w_cursor.lnum <= 1 {
            return FAIL;
        }
        cursor_up_inner(curwin, n, false);
        coladvance(curwin, (*curwin).w_curswant);
        if upd_topline {
            update_topline(curwin);
        }
        OK
    }
}

/// Move the cursor down `n` lines in window `wp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_down_inner(wp: *mut win_T, mut n: c_int, skip_conceal: bool) {
    unsafe {
        let mut lnum = (*wp).w_cursor.lnum;
        let line_count = (*(*wp).w_buffer).b_ml.ml_line_count;

        if lnum + n >= line_count {
            lnum = line_count;
        } else if win_lines_concealed(wp) {
            let mut last: linenr_T = 0;
            while n > 0 {
                n -= 1;
                if hasFoldingWin(
                    wp,
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
                if skip_conceal && decor_conceal_line(wp, lnum - 1, true) {
                    n += 1;
                }
            }
            lnum = lnum.min(line_count);
        } else {
            lnum += n;
        }

        (*wp).w_cursor.lnum = lnum;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_down(n: c_int, upd_topline: bool) -> c_int {
    unsafe {
        let mut lnum = (*curwin).w_cursor.lnum;
        hasFoldingWin(
            curwin,
            lnum,
            ptr::null_mut(),
            &raw mut lnum,
            true,
            ptr::null_mut(),
        );
        if n > 0 && lnum >= (*(*curwin).w_buffer).b_ml.ml_line_count {
            return FAIL;
        }
        cursor_down_inner(curwin, n, false);
        coladvance(curwin, (*curwin).w_curswant);
        if upd_topline {
            update_topline(curwin);
        }
        OK
    }
}

/// Stuff the last inserted text in the read buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stuff_inserted(c: c_int, mut count: c_int, no_esc: c_int) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_last_insert() -> NvimString {
    unsafe {
        if last_insert.data.is_null() {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        } else {
            NvimString {
                data: last_insert.data.offset(last_insert_skip as isize),
                size: last_insert.size - (last_insert_skip as usize),
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_last_insert_save() -> *mut c_char {
    unsafe {
        let mut insert = get_last_insert();
        if insert.data.is_null() {
            return ptr::null_mut();
        }
        let s: *mut c_char = xmemdupz(insert.data.cast(), insert.size).cast();
        if insert.size > 0 && *s.add(insert.size - 1) == ESC as c_char {
            insert.size -= 1;
            *s.add(insert.size) = NUL;
        }
        s
    }
}

unsafe fn echeck_abbr(c: c_int) -> bool {
    unsafe {
        if p_paste != 0 || no_abbr || Ins.moved != kInsNone {
            return false;
        }
        check_abbr(
            c,
            get_cursor_line_ptr(),
            (*curwin).w_cursor.col,
            if (*curwin).w_cursor.lnum == Ins.start.lnum {
                Ins.start.col
            } else {
                0
            },
        )
    }
}

/// Push character that is replaced onto the replace stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn replace_push(str_: *mut c_char, len: usize) {
    unsafe {
        if replace_stack.size < replace_offset as usize {
            return;
        }
        crate::kvec::kv_ensure_space(&raw mut replace_stack, len);
        let p = replace_stack
            .items
            .add(replace_stack.size - replace_offset as usize);
        if replace_offset != 0 {
            ptr::copy(p, p.add(len), replace_offset as usize);
        }
        ptr::copy_nonoverlapping(str_, p, len);
        replace_stack.size += len;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn replace_push_nul() {
    unsafe {
        replace_push(c"".as_ptr().cast_mut(), 1);
    }
}

unsafe fn replace_pop_if_nul() -> c_int {
    unsafe {
        let ch = if replace_stack.size != 0 {
            *replace_stack.items.add(replace_stack.size - 1) as u8 as c_int
        } else {
            -1
        };
        if ch == NUL as c_int {
            replace_stack.size -= 1;
        }
        ch
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn replace_join(mut off: c_int) {
    unsafe {
        let mut i = replace_stack.size as isize;
        while {
            i -= 1;
            i >= 0
        } {
            if *replace_stack.items.offset(i) == NUL {
                let cur_off = off;
                off -= 1;
                if cur_off <= 0 {
                    replace_stack.size -= 1;
                    ptr::copy(
                        replace_stack.items.offset(i + 1),
                        replace_stack.items.offset(i),
                        replace_stack.size - (i as usize),
                    );
                    return;
                }
            }
        }
    }
}

unsafe fn replace_pop_ins() {
    unsafe {
        let old_state = State;
        State = MODE_NORMAL;
        while replace_pop_if_nul() > 0 {
            mb_replace_pop_ins();
            dec_cursor();
        }
        State = old_state;
    }
}

unsafe fn mb_replace_pop_ins() {
    unsafe {
        let len = utf_head_off(
            replace_stack.items,
            replace_stack.items.add(replace_stack.size - 1),
        ) + 1;
        replace_stack.size -= len as usize;
        ins_bytes_len(replace_stack.items.add(replace_stack.size), len as usize);
    }
}

unsafe fn replace_do_bs(limit_col: c_int) {
    unsafe {
        let mut start_vcol: colnr_T = 0;
        let l_state = State;

        let cc = replace_pop_if_nul();
        if cc > 0 {
            let mut orig_len: c_int = 0;
            let mut orig_vcols: c_int = 0;
            if (l_state & VREPLACE_FLAG) != 0 {
                getvcol(
                    curwin,
                    &raw mut (*curwin).w_cursor,
                    ptr::null_mut(),
                    &raw mut start_vcol,
                    ptr::null_mut(),
                    0,
                );
                orig_vcols = win_chartabsize(curwin, get_cursor_pos_ptr(), start_vcol);
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
                    vcol += win_chartabsize(curwin, p.offset(i as isize), vcol);
                    i += utfc_ptr2len(p.offset(i as isize));
                }
                vcol -= start_vcol;

                (*curwin).w_cursor.col += ins_len;
                while vcol > orig_vcols && gchar_cursor() == b' ' as c_int {
                    del_char(false);
                    orig_vcols += 1;
                }
                (*curwin).w_cursor.col -= ins_len;
            }

            changed_bytes((*curwin).w_cursor.lnum, (*curwin).w_cursor.col);
        } else if cc == 0 {
            del_char_after_col(limit_col);
        }
    }
}

unsafe fn ins_reg() {
    unsafe {
        let mut need_redraw = false;
        let mut literally: c_int = 0;
        let vis_active = Visual.active;

        pc_status = PC_STATUS_UNSET;
        if redrawing() && !char_avail() {
            ins_redraw(false);
            edit_putchar(b'"' as c_int, true);
            add_to_showcmd_c(Ctrl_R);
        }

        no_mapping += 1;
        allow_keys += 1;
        let mut regname = plain_vgetc();
        langmap_adjust(&mut regname, true);
        if regname == Ctrl_R || regname == Ctrl_O || regname == Ctrl_P {
            literally = regname;
            add_to_showcmd_c(literally);
            regname = plain_vgetc();
            langmap_adjust(&mut regname, true);
        }
        no_mapping -= 1;
        allow_keys -= 1;

        no_u_sync += 1;
        if regname == b'=' as c_int {
            let curpos = (*curwin).w_cursor;
            u_sync_once = 2;
            regname = get_expr_register();
            (*curwin).w_cursor = curpos;
            check_cursor(curwin);
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
            } else if (*reg).y_size > 1 && is_literal_register(regname) {
                redo_append_char(Ctrl_R);
                redo_append_char(regname);
                do_put(regname, ptr::null_mut(), BACKWARD, 1, PUT_CURSEND);
            } else if insert_reg(regname, ptr::null_mut(), literally != 0) == FAIL {
                vim_beep(kOptBoFlagRegister);
                need_redraw = true;
            } else if Ins.stop_insert_mode {
                need_redraw = true;
            }
        }
        no_u_sync -= 1;
        if u_sync_once == 1 {
            Ins.need_undo = true;
        }
        u_sync_once = 0;

        if need_redraw || stuff_empty() {
            edit_unputchar();
        }
        clear_showcmd();

        if !vis_active && Visual.active {
            end_visual_mode();
        }
    }
}

unsafe fn ins_ctrl_g() {
    unsafe {
        setcursor();

        no_mapping += 1;
        allow_keys += 1;
        let c = plain_vgetc();
        no_mapping -= 1;
        allow_keys -= 1;
        match c {
            K_UP | Ctrl_K => ins_up(true),
            c if c == b'k' as c_int => ins_up(true),
            K_DOWN | Ctrl_J => ins_down(true),
            c if c == b'j' as c_int => ins_down(true),
            c if c == b'u' as c_int => {
                u_sync(true);
                Ins.need_undo = true;
                Ins.update_start_orig = false;
                Ins.start = (*curwin).w_cursor;
            }
            c if c == b'U' as c_int => {
                Ins.dont_sync_undo = kNone;
            }
            ESC => {}
            _ => {
                vim_beep(kOptBoFlagCtrlg);
            }
        }
    }
}

unsafe fn ins_ctrl_hat() {
    unsafe {
        if map_to_exists_mode(c"".as_ptr(), MODE_LANGMAP, false) {
            if (State & MODE_LANGMAP) != 0 {
                (*curbuf).b_p_iminsert = B_IMODE_NONE;
                State &= !MODE_LANGMAP;
            } else {
                (*curbuf).b_p_iminsert = B_IMODE_LMAP;
                State |= MODE_LANGMAP;
            }
        }
        set_iminsert_global(curbuf);
        showmode();
        status_redraw_curbuf();
    }
}

unsafe fn ins_esc(count: *mut c_int, cmdchar: c_int, nomove: bool) -> bool {
    unsafe {
        check_spell_redraw();

        let temp = (*curwin).w_cursor.col;
        if ins_esc_disabled_redraw {
            RedrawingDisabled -= 1;
            ins_esc_disabled_redraw = false;
        }
        if Ins.moved == kInsNone {
            if cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
                redo_append_str(c"\x1b".as_ptr(), 1);
            }

            if *count > 0 {
                line_breakcheck();
                if got_int {
                    *count = 0;
                }
            }

            *count -= 1;
            if *count > 0 {
                if !vim_strchr(p_cpo, kCpoReplcnt).is_null() {
                    State &= !REPLACE_FLAG;
                }

                start_redo_ins();
                if cmdchar == b'r' as c_int || cmdchar == b'v' as c_int {
                    stuffRedoReadbuff(c"\x1b".as_ptr());
                }
                RedrawingDisabled += 1;
                ins_esc_disabled_redraw = true;
                return false;
            }
            stop_insert(&raw mut (*curwin).w_cursor, 1, c_int::from(nomove));
            undisplay_dollar();
        } else if Ins.moved == kInsArrow && cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
            redo_append_str(c"\x1b".as_ptr(), 1);
        }

        if cmdchar != b'r' as c_int && cmdchar != b'v' as c_int {
            ins_apply_autocmds(EVENT_INSERTLEAVEPRE);
        }

        if restart_edit == NUL as c_int && temp == (*curwin).w_cursor.col {
            (*curwin).w_set_curswant = true;
        }

        if (cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 {
            let view = mark_view_make(curwin, (*curwin).w_cursor);
            reset_fmark(
                &raw mut (*curbuf).b_last_insert,
                (*curwin).w_cursor,
                (*curbuf).b_fnum,
                view,
            );
        }

        if !nomove
            && ((*curwin).w_cursor.col != 0 || (*curwin).w_cursor.coladd > 0)
            && (restart_edit == NUL as c_int || (gchar_cursor() == NUL as c_int && !Visual.active))
            && !Ins.revins_on
        {
            if (*curwin).w_cursor.coladd > 0 || get_ve_flags(curwin) == kOptVeFlagAll {
                oneleft();
                if restart_edit != NUL as c_int {
                    (*curwin).w_cursor.coladd += 1;
                }
            } else {
                (*curwin).w_cursor.col -= 1;
                (*curwin).w_valid &= !(VALID_WCOL | VALID_VIRTCOL);
                mb_adjust_cursor();
            }
        }

        State = MODE_NORMAL;
        may_trigger_modechanged();
        if gchar_cursor() == TAB || buf_meta_total(curbuf, kMTMetaInline) > 0 {
            (*curwin).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
        }

        setmouse();
        ui_cursor_shape();

        if reg_recording != 0 || restart_edit != NUL as c_int {
            showmode();
        } else if p_smd != 0
            && (got_int || !skip_showmode())
            && !(p_ch == 0 && !ui_has(kUIMessages))
        {
            unshowmode(false);
        }
        true
    }
}

unsafe fn ins_ctrl_() {
    unsafe {
        if Ins.revins_on && Ins.revins_chars != 0 && Ins.revins_scol >= 0 {
            while gchar_cursor() != NUL as c_int && Ins.revins_chars != 0 {
                Ins.revins_chars -= 1;
                (*curwin).w_cursor.col += 1;
            }
        }
        p_ri = c_int::from(p_ri == 0);
        Ins.revins_on = State == MODE_INSERT && p_ri != 0;
        if Ins.revins_on {
            Ins.revins_scol = (*curwin).w_cursor.col;
            Ins.revins_legal += 1;
            Ins.revins_chars = 0;
            undisplay_dollar();
        } else {
            Ins.revins_scol = -1;
        }
        showmode();
    }
}

unsafe fn ins_start_select(c: c_int) -> bool {
    unsafe {
        if !km_startsel {
            return false;
        }
        match c {
            K_KHOME | K_KEND | K_PAGEUP | K_KPAGEUP | K_PAGEDOWN | K_KPAGEDOWN
                if (mod_mask & MOD_MASK_SHIFT) == 0 =>
            {
                false
            }
            K_KHOME | K_KEND | K_PAGEUP | K_KPAGEUP | K_PAGEDOWN | K_KPAGEDOWN | K_S_LEFT
            | K_S_RIGHT | K_S_UP | K_S_DOWN | K_S_END | K_S_HOME => {
                start_selection();
                stuffcharReadbuff(Ctrl_O);
                if mod_mask != 0 {
                    let buf = [
                        K_SPECIAL as c_char,
                        KS_MODIFIER as c_char,
                        mod_mask as u8 as c_char,
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

unsafe fn ins_insert(replace_state: c_int) {
    unsafe {
        let mode_str = if (State & REPLACE_FLAG) != 0 {
            c"i".as_ptr()
        } else if replace_state == MODE_VREPLACE {
            c"v".as_ptr()
        } else {
            c"r".as_ptr()
        };
        set_vim_var_string(VV_INSERTMODE, mode_str, 1);
        ins_apply_autocmds(EVENT_INSERTCHANGE);
        if (State & REPLACE_FLAG) != 0 {
            State = MODE_INSERT | (State & MODE_LANGMAP);
        } else {
            State = replace_state | (State & MODE_LANGMAP);
        }
        may_trigger_modechanged();
        redo_append_char(K_INS);
        showmode();
        ui_cursor_shape();
    }
}

unsafe fn ins_ctrl_o() {
    unsafe {
        Visual.restart_select = 0;
        if (State & VREPLACE_FLAG) != 0 {
            restart_edit = b'V' as c_int;
        } else if (State & REPLACE_FLAG) != 0 {
            restart_edit = b'R' as c_int;
        } else {
            restart_edit = b'I' as c_int;
        }
        if virtual_active(curwin) {
            ins_at_eol = false;
        } else {
            ins_at_eol = gchar_cursor() == NUL as c_int;
        }
    }
}

unsafe fn ins_shift(c: c_int, lastc: c_int) {
    unsafe {
        if stop_arrow() == FAIL {
            return;
        }
        redo_append_char(c);

        if c == Ctrl_D
            && (lastc == b'0' as c_int || lastc == b'^' as c_int)
            && (*curwin).w_cursor.col > 0
        {
            (*curwin).w_cursor.col -= 1;
            del_char(false);
            if (State & REPLACE_FLAG) != 0 {
                replace_pop_ins();
            }
            if lastc == b'^' as c_int {
                old_indent = get_indent();
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

        if Ins.did_ai && *skipwhite(get_cursor_line_ptr()) != NUL {
            Ins.did_ai = false;
        }
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;
        Ins.can_cindent = false;
    }
}

unsafe fn ins_del() {
    unsafe {
        if stop_arrow() == FAIL {
            return;
        }
        if gchar_cursor() == NUL as c_int {
            let temp = (*curwin).w_cursor.col;
            if !can_bs(BS_EOL) || do_join(2, false, true, false, false) == FAIL {
                vim_beep(kOptBoFlagBackspace);
            } else {
                mc_ins_join();
                (*curwin).w_cursor.col = temp;
                if (State & VREPLACE_FLAG) != 0 && orig_line_count > (*curbuf).b_ml.ml_line_count {
                    orig_line_count = (*curbuf).b_ml.ml_line_count;
                }
            }
        } else if del_char(false) == FAIL {
            vim_beep(kOptBoFlagBackspace);
        }
        Ins.did_ai = false;
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;
        redo_append_char(K_DEL);
    }
}

unsafe fn ins_bs(c: c_int, mut mode: c_int, inserted_space_p: *mut c_int) -> bool {
    unsafe {
        let mut cc: c_int;
        let mut temp: c_int = 0;
        let mut did_backspace = false;
        let mut call_fix_indent = false;

        if buf_is_empty(curbuf)
            || (!Ins.revins_on
                && (((*curwin).w_cursor.lnum == 1 && (*curwin).w_cursor.col == 0)
                    || (!can_bs(BS_START)
                        && ((Ins.moved != kInsNone && !bt_prompt(curbuf))
                            || ((*curwin).w_cursor.lnum == Ins.start_orig.lnum
                                && (*curwin).w_cursor.col <= Ins.start_orig.col)))
                    || (!can_bs(BS_INDENT)
                        && Ins.moved == kInsNone
                        && Ins.ai_col > 0
                        && (*curwin).w_cursor.col <= Ins.ai_col)
                    || (!can_bs(BS_EOL) && (*curwin).w_cursor.col == 0)))
        {
            vim_beep(kOptBoFlagBackspace);
            return false;
        }

        if stop_arrow() == FAIL {
            return false;
        }
        let in_indent = inindent(0);
        if in_indent {
            Ins.can_cindent = false;
        }
        Ins.end_comment_pending = NUL as c_int;
        if Ins.revins_on {
            inc_cursor();
        }
        if (*curwin).w_cursor.coladd > 0 {
            if mode == BACKSPACE_CHAR {
                (*curwin).w_cursor.coladd -= 1;
                return true;
            }
            if mode == BACKSPACE_WORD {
                (*curwin).w_cursor.coladd = 0;
                return true;
            }
            (*curwin).w_cursor.coladd = 0;
        }

        if (*curwin).w_cursor.col == 0 {
            mc_ins_join();
            let lnum = Ins.start.lnum;
            if (*curwin).w_cursor.lnum == lnum || Ins.revins_on {
                if u_save((*curwin).w_cursor.lnum - 2, (*curwin).w_cursor.lnum + 1) == FAIL {
                    return false;
                }
                Ins.start.lnum -= 1;
                Ins.start.col = ml_get_len(Ins.start.lnum);
            }
            cc = -1;
            if (State & REPLACE_FLAG) != 0 {
                cc = replace_pop_if_nul();
            }
            if (State & REPLACE_FLAG) != 0 && (*curwin).w_cursor.lnum <= lnum {
                dec_cursor();
            } else {
                if (State & VREPLACE_FLAG) == 0 || (*curwin).w_cursor.lnum > orig_line_count {
                    temp = gchar_cursor();
                    (*curwin).w_cursor.lnum -= 1;

                    if has_format_option(kFoAuto) && has_format_option(kFoWhitePar) {
                        let ptr = ml_get_buf(curbuf, (*curwin).w_cursor.lnum);
                        let len = get_cursor_line_len();
                        if len > 0 && *ptr.add((len - 1) as usize) == b' ' as c_char {
                            let newp: *mut c_char = xmemdupz(ptr.cast(), (len - 1) as usize).cast();
                            if ((*curbuf).b_ml.ml_flags & (ML_LINE_DIRTY | ML_ALLOCATED)) != 0 {
                                xfree((*curbuf).b_ml.ml_line_ptr.cast());
                            }
                            (*curbuf).b_ml.ml_line_ptr = newp;
                            (*curbuf).b_ml.ml_line_textlen -= 1;
                            (*curbuf).b_ml.ml_flags |= ML_LINE_DIRTY;
                        }
                    }

                    do_join(2, false, false, false, false);
                    if temp == NUL as c_int && gchar_cursor() != NUL as c_int {
                        inc_cursor();
                    }
                } else {
                    dec_cursor();
                }

                if (State & REPLACE_FLAG) != 0 {
                    let old_state = State;
                    State = MODE_NORMAL;
                    while cc > 0 {
                        let save_col = (*curwin).w_cursor.col;
                        mb_replace_pop_ins();
                        (*curwin).w_cursor.col = save_col;
                        cc = replace_pop_if_nul();
                    }
                    replace_pop_ins();
                    State = old_state;
                }
            }
            Ins.did_ai = false;
        } else {
            if Ins.revins_on {
                dec_cursor();
            }
            let mut mincol: colnr_T = 0;
            if mode == BACKSPACE_LINE && ((*curbuf).b_p_ai != 0 || cindent_on()) && !Ins.revins_on {
                let save_col = (*curwin).w_cursor.col;
                beginline(BL_WHITE);
                if (*curwin).w_cursor.col < save_col {
                    mincol = (*curwin).w_cursor.col;
                    call_fix_indent = true;
                }
                (*curwin).w_cursor.col = save_col;
            }

            if mode == BACKSPACE_CHAR
                && ((p_sta != 0 && in_indent)
                    || ((get_sts_value() != 0 || tabstop_count((*curbuf).b_p_vsts_array) != 0)
                        && (*curwin).w_cursor.col > 0
                        && (*get_cursor_pos_ptr().offset(-1) == TAB as c_char
                            || (*get_cursor_pos_ptr().offset(-1) == b' ' as c_char
                                && (*inserted_space_p == 0 || Ins.moved != kInsNone)))))
            {
                *inserted_space_p = 0;

                let use_ts =
                    (*curwin).w_onebuf_opt.wo_list == 0 || (*curwin).w_p_lcs_chars.tab1 != 0;
                let line = get_cursor_line_ptr();
                let cursor_ptr = line.offset((*curwin).w_cursor.col as isize).cast_const();

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
                        charsize_nowrap(curbuf, sci.ptr, use_ts, vcol, sci.chr.value, cli.cells);
                    sci = cli.next;
                    prev_space = cur_space;
                }

                let mut want_vcol = if vcol > 0 { vcol - 1 } else { 0 };
                if p_sta != 0 && in_indent {
                    want_vcol -= want_vcol % (get_sw_value(curbuf) as colnr_T);
                } else {
                    want_vcol = tabstop_start(
                        want_vcol,
                        get_sts_value() as c_int,
                        (*curbuf).b_p_vsts_array,
                    );
                }

                loop {
                    let cli = utf_ClusterInfo(space_sci);
                    let size = charsize_nowrap(
                        curbuf,
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

                while (*curwin).w_cursor.col > want_col {
                    dec_cursor();
                    if (State & REPLACE_FLAG) != 0 {
                        if (*curwin).w_cursor.lnum != Ins.start.lnum
                            || (*curwin).w_cursor.col >= Ins.start.col
                        {
                            replace_do_bs(-1);
                        }
                    } else {
                        del_char(false);
                    }
                }

                while space_vcol < want_vcol {
                    if (*curwin).w_cursor.lnum == Ins.start_orig.lnum
                        && (*curwin).w_cursor.col < Ins.start_orig.col
                    {
                        Ins.start_orig.col = (*curwin).w_cursor.col;
                    }

                    if (State & VREPLACE_FLAG) != 0 {
                        ins_char(b' ' as c_int);
                    } else {
                        ins_str(c" ".as_ptr().cast_mut(), 1);
                        if (State & REPLACE_FLAG) != 0 {
                            replace_push_nul();
                        }
                    }
                    space_vcol += 1;
                }
            } else {
                let mut cclass = mb_get_class(get_cursor_pos_ptr());
                loop {
                    if !Ins.revins_on {
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
                        if !Ins.revins_on {
                            inc_cursor();
                        } else if (State & REPLACE_FLAG) != 0 {
                            dec_cursor();
                        }
                        break;
                    }
                    if (State & REPLACE_FLAG) != 0 {
                        replace_do_bs(-1);
                    } else {
                        let mut has_composing = false;
                        if p_deco != 0 {
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
                        if Ins.revins_chars != 0 {
                            Ins.revins_chars -= 1;
                            Ins.revins_legal += 1;
                        }
                        if Ins.revins_on && gchar_cursor() == NUL as c_int {
                            break;
                        }
                    }
                    if mode == BACKSPACE_CHAR {
                        break;
                    }
                    if !(Ins.revins_on
                        || ((*curwin).w_cursor.col > mincol
                            && (can_bs(BS_NOSTOP)
                                || (*curwin).w_cursor.lnum != Ins.start_orig.lnum
                                || (*curwin).w_cursor.col != Ins.start_orig.col)))
                    {
                        break;
                    }
                }
            }
            did_backspace = true;
        }
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;
        if (*curwin).w_cursor.col <= 1 {
            Ins.did_ai = false;
        }

        if call_fix_indent {
            fix_indent();
        }

        redo_append_char(c);

        if (*curwin).w_cursor.lnum == Ins.start_orig.lnum
            && (*curwin).w_cursor.col < Ins.start_orig.col
        {
            Ins.start_orig.col = (*curwin).w_cursor.col;
        }

        if !vim_strchr(p_cpo, kCpoBackspace).is_null() && dollar_vcol == -1 {
            dollar_vcol = (*curwin).w_virtcol;
        }

        if did_backspace {
            foldOpenCursor();
        }
        did_backspace
    }
}

unsafe fn ins_left() {
    unsafe {
        let end_change = Ins.dont_sync_undo == kFalse;
        if (fdo_flags & kOptFdoFlagHor) != 0 && KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        let mut tpos = (*curwin).w_cursor;
        if oneleft() == OK {
            start_arrow_with_change(&raw mut tpos, end_change, K_LEFT);
            if Ins.revins_scol != -1 && (*curwin).w_cursor.col >= Ins.revins_scol {
                Ins.revins_legal += 1;
            }
            Ins.revins_chars += 1;
        } else if !vim_strchr(p_ww, b'[' as c_int).is_null() && (*curwin).w_cursor.lnum > 1 {
            start_arrow(&raw mut tpos, true, K_LEFT);
            (*curwin).w_cursor.lnum -= 1;
            coladvance(curwin, MAXCOL);
            (*curwin).w_set_curswant = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        Ins.dont_sync_undo = kFalse;
    }
}

unsafe fn ins_home(c: c_int) {
    unsafe {
        if (fdo_flags & kOptFdoFlagHor) != 0 && KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        let mut tpos = (*curwin).w_cursor;
        if c == K_C_HOME {
            (*curwin).w_cursor.lnum = 1;
        }
        (*curwin).w_cursor.col = 0;
        (*curwin).w_cursor.coladd = 0;
        (*curwin).w_curswant = 0;
        start_arrow(
            &raw mut tpos,
            true,
            if c == K_C_HOME { NUL as c_int } else { K_HOME },
        );
    }
}

unsafe fn ins_end(c: c_int) {
    unsafe {
        if (fdo_flags & kOptFdoFlagHor) != 0 && KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        let mut tpos = (*curwin).w_cursor;
        if c == K_C_END {
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
        }
        coladvance(curwin, MAXCOL);
        (*curwin).w_curswant = MAXCOL;
        start_arrow(
            &raw mut tpos,
            true,
            if c == K_C_END { NUL as c_int } else { K_END },
        );
    }
}

unsafe fn ins_s_left() {
    unsafe {
        let end_change = Ins.dont_sync_undo == kFalse;
        if (fdo_flags & kOptFdoFlagHor) != 0 && KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        if (*curwin).w_cursor.lnum > 1 || (*curwin).w_cursor.col > 0 {
            start_arrow_with_change(&raw mut (*curwin).w_cursor, end_change, K_S_LEFT);
            bck_word(1, false, false);
            (*curwin).w_set_curswant = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        Ins.dont_sync_undo = kFalse;
    }
}

unsafe fn ins_right() {
    unsafe {
        let end_change = Ins.dont_sync_undo == kFalse;
        if (fdo_flags & kOptFdoFlagHor) != 0 && KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        if gchar_cursor() != NUL as c_int || virtual_active(curwin) {
            start_arrow_with_change(&raw mut (*curwin).w_cursor, end_change, K_RIGHT);
            (*curwin).w_set_curswant = true;
            if virtual_active(curwin) {
                oneright();
            } else {
                (*curwin).w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
            }

            Ins.revins_legal += 1;
            if Ins.revins_chars != 0 {
                Ins.revins_chars -= 1;
            }
        } else if !vim_strchr(p_ww, b']' as c_int).is_null()
            && (*curwin).w_cursor.lnum < (*curbuf).b_ml.ml_line_count
        {
            start_arrow(&raw mut (*curwin).w_cursor, true, K_RIGHT);
            (*curwin).w_set_curswant = true;
            (*curwin).w_cursor.lnum += 1;
            (*curwin).w_cursor.col = 0;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        Ins.dont_sync_undo = kFalse;
    }
}

unsafe fn ins_s_right() {
    unsafe {
        let end_change = Ins.dont_sync_undo == kFalse;
        if (fdo_flags & kOptFdoFlagHor) != 0 && KeyTyped {
            foldOpenCursor();
        }
        undisplay_dollar();
        if (*curwin).w_cursor.lnum < (*curbuf).b_ml.ml_line_count || gchar_cursor() != NUL as c_int
        {
            start_arrow_with_change(&raw mut (*curwin).w_cursor, end_change, K_S_RIGHT);
            fwd_word(1, false, false);
            (*curwin).w_set_curswant = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
        Ins.dont_sync_undo = kFalse;
    }
}

unsafe fn ins_up(startcol: bool) {
    unsafe {
        let old_topline = (*curwin).w_topline;
        let old_topfill = (*curwin).w_topfill;

        undisplay_dollar();
        let mut tpos = (*curwin).w_cursor;
        if cursor_up(1, true) == OK {
            if startcol {
                coladvance(curwin, getvcol_nolist(&raw mut Ins.start));
            }
            if old_topline != (*curwin).w_topline || old_topfill != (*curwin).w_topfill {
                redraw_later(curwin, UPD_VALID);
            }
            start_arrow(
                &raw mut tpos,
                true,
                if startcol { b'k' as c_int } else { K_UP },
            );
            Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

unsafe fn ins_pageup() {
    unsafe {
        undisplay_dollar();

        if (mod_mask & MOD_MASK_CTRL) != 0 {
            if !(*first_tabpage).tp_next.is_null() {
                start_arrow(&raw mut (*curwin).w_cursor, true, NUL as c_int);
                goto_tabpage(-1);
            }
            return;
        }

        let mut tpos = (*curwin).w_cursor;
        if pagescroll(BACKWARD, 1, false) == OK {
            start_arrow(&raw mut tpos, true, NUL as c_int);
            Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

unsafe fn ins_down(startcol: bool) {
    unsafe {
        let old_topline = (*curwin).w_topline;
        let old_topfill = (*curwin).w_topfill;

        undisplay_dollar();
        let mut tpos = (*curwin).w_cursor;
        if cursor_down(1, true) == OK {
            if startcol {
                coladvance(curwin, getvcol_nolist(&raw mut Ins.start));
            }
            if old_topline != (*curwin).w_topline || old_topfill != (*curwin).w_topfill {
                redraw_later(curwin, UPD_VALID);
            }
            start_arrow(
                &raw mut tpos,
                true,
                if startcol { b'j' as c_int } else { K_DOWN },
            );
            Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

unsafe fn ins_pagedown() {
    unsafe {
        undisplay_dollar();

        if (mod_mask & MOD_MASK_CTRL) != 0 {
            if !(*first_tabpage).tp_next.is_null() {
                start_arrow(&raw mut (*curwin).w_cursor, true, NUL as c_int);
                goto_tabpage(0);
            }
            return;
        }

        let mut tpos = (*curwin).w_cursor;
        if pagescroll(FORWARD, 1, false) == OK {
            start_arrow(&raw mut tpos, true, NUL as c_int);
            Ins.can_cindent = true;
        } else {
            vim_beep(kOptBoFlagCursor);
        }
    }
}

unsafe fn ins_tab() -> bool {
    unsafe {
        if Ins.start_blank_vcol == MAXCOL && (*curwin).w_cursor.lnum == Ins.start.lnum {
            Ins.start_blank_vcol = get_nolist_virtcol();
        }
        if echeck_abbr(TAB + ABBR_OFF) {
            return false;
        }

        let ind = inindent(0);
        if ind {
            Ins.can_cindent = false;
        }

        if (*curbuf).b_p_et == 0
            && !(p_sta != 0
                && ind
                && ((tabstop_count((*curbuf).b_p_vts_array) > 1)
                    || (tabstop_count((*curbuf).b_p_vts_array) == 1
                        && tabstop_first((*curbuf).b_p_vts_array) != get_sw_value(curbuf))
                    || (tabstop_count((*curbuf).b_p_vts_array) == 0
                        && (*curbuf).b_p_ts != i64::from(get_sw_value(curbuf)))))
            && tabstop_count((*curbuf).b_p_vsts_array) == 0
            && get_sts_value() == 0
        {
            return true;
        }

        if stop_arrow() == FAIL {
            return true;
        }

        Ins.did_ai = false;
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;
        redo_append_str(c"\t".as_ptr(), 1);

        let mut temp: c_int = if p_sta != 0 && ind {
            let sw = get_sw_value(curbuf);
            sw - (get_nolist_virtcol() % sw)
        } else if tabstop_count((*curbuf).b_p_vsts_array) > 0 || (*curbuf).b_p_sts != 0 {
            tabstop_padding(
                get_nolist_virtcol(),
                i64::from(get_sts_value()),
                (*curbuf).b_p_vsts_array,
            )
        } else {
            tabstop_padding(
                get_nolist_virtcol(),
                (*curbuf).b_p_ts,
                (*curbuf).b_p_vts_array,
            )
        };

        ins_char(b' ' as c_int);
        while {
            temp -= 1;
            temp > 0
        } {
            if (State & VREPLACE_FLAG) != 0 {
                ins_char(b' ' as c_int);
            } else {
                ins_str(c" ".as_ptr().cast_mut(), 1);
                if (State & REPLACE_FLAG) != 0 {
                    replace_push_nul();
                }
            }
        }

        if (*curbuf).b_p_et == 0
            && (tabstop_count((*curbuf).b_p_vsts_array) > 0
                || get_sts_value() > 0
                || (p_sta != 0 && ind))
        {
            let mut pos: pos_T;
            let cursor: *mut pos_T;
            let mut saved_line: *mut c_char = ptr::null_mut();
            let mut ptr: *mut c_char;
            let mut vcol: colnr_T = 0;
            let mut want_vcol: colnr_T = 0;
            let mut change_col: c_int = -1;
            let save_list = (*curwin).w_onebuf_opt.wo_list;

            if (State & VREPLACE_FLAG) != 0 {
                pos = (*curwin).w_cursor;
                cursor = &raw mut pos;
                saved_line = xstrnsave(get_cursor_line_ptr(), get_cursor_line_len() as usize);
                ptr = saved_line.offset(pos.col as isize);
            } else {
                ptr = get_cursor_pos_ptr();
                cursor = &raw mut (*curwin).w_cursor;
            }

            if vim_strchr(p_cpo, kCpoListwm).is_null() {
                (*curwin).w_onebuf_opt.wo_list = 0;
            }

            let mut fpos = (*curwin).w_cursor;
            while fpos.col > 0 && ascii_iswhite(*ptr.offset(-1) as u8 as c_int) {
                fpos.col -= 1;
                ptr = ptr.offset(-1);
            }

            if (State & REPLACE_FLAG) != 0
                && fpos.lnum == Ins.start.lnum
                && fpos.col < Ins.start.col
            {
                ptr = ptr.offset((Ins.start.col - fpos.col) as isize);
                fpos.col = Ins.start.col;
            }

            getvcol(
                curwin,
                &raw mut fpos,
                &raw mut vcol,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            );
            getvcol(
                curwin,
                cursor,
                &raw mut want_vcol,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            );

            let tab = c"\t".as_ptr().cast_mut();
            let tab_v = b'\t' as i32;
            let mut csarg = std::mem::zeroed::<CharsizeArg>();
            let mut cstype = init_charsize_arg(&raw mut csarg, curwin, 0, tab);

            while ascii_iswhite(*ptr as u8 as c_int) {
                let i = win_charsize(cstype, vcol, tab, tab_v, &raw mut csarg, 1).width;
                if vcol + i > want_vcol {
                    break;
                }
                if *ptr != TAB as c_char {
                    *ptr = TAB as c_char;
                    if change_col < 0 {
                        change_col = fpos.col;
                        if fpos.lnum == Ins.start.lnum && fpos.col < Ins.start.col {
                            Ins.start.col = fpos.col;
                        }
                    }
                }
                fpos.col += 1;
                ptr = ptr.add(1);
                vcol += i;
            }

            if change_col >= 0 {
                let mut repl_off: c_int = 0;
                cstype = init_charsize_arg(&raw mut csarg, curwin, 0, ptr);
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
                    if (State & VREPLACE_FLAG) == 0 {
                        let newp_len = (*curbuf).b_ml.ml_line_textlen - i;
                        let newp = xmalloc(newp_len as usize).cast::<c_char>();
                        let col = ptr.offset_from((*curbuf).b_ml.ml_line_ptr);
                        if col > 0 {
                            ptr::copy(ptr.offset(-col), newp, col as usize);
                        }
                        ptr::copy(
                            ptr.offset(i as isize),
                            newp.offset(col),
                            (newp_len as isize - col) as usize,
                        );
                        if ((*curbuf).b_ml.ml_flags & (ML_LINE_DIRTY | ML_ALLOCATED)) != 0 {
                            xfree((*curbuf).b_ml.ml_line_ptr.cast());
                        }
                        (*curbuf).b_ml.ml_line_ptr = newp;
                        (*curbuf).b_ml.ml_line_textlen = newp_len;
                        (*curbuf).b_ml.ml_flags =
                            ((*curbuf).b_ml.ml_flags | ML_LINE_DIRTY) & !ML_EMPTY;
                        inserted_bytes(
                            fpos.lnum,
                            change_col,
                            (*cursor).col - change_col,
                            fpos.col - change_col,
                        );
                    } else {
                        let rem = c_strlen(ptr.offset(i as isize)) + 1;
                        ptr::copy(ptr.offset(i as isize), ptr, rem);
                    }
                    if (State & REPLACE_FLAG) != 0 && (State & VREPLACE_FLAG) == 0 {
                        for _ in 0..i {
                            replace_join(repl_off);
                        }
                    }
                }
                (*cursor).col -= i;

                if (State & VREPLACE_FLAG) != 0 {
                    backspace_until_column(change_col);
                    ins_bytes_len(
                        saved_line.offset(change_col as isize),
                        ((*cursor).col - change_col) as usize,
                    );
                }
            }

            if (State & VREPLACE_FLAG) != 0 {
                xfree(saved_line.cast());
            }
            (*curwin).w_onebuf_opt.wo_list = save_list;
        }

        false
    }
}

/// Handle CR or NL in insert mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_eol(c: c_int) -> bool {
    unsafe {
        if echeck_abbr(c + ABBR_OFF) {
            return true;
        }
        if stop_arrow() == FAIL {
            return false;
        }
        undisplay_dollar();

        if (State & REPLACE_FLAG) != 0 && (State & VREPLACE_FLAG) == 0 {
            replace_push_nul();
        }

        if virtual_active(curwin) && (*curwin).w_cursor.coladd > 0 {
            coladvance(curwin, getviscol());
        }

        if Ins.revins_on {
            (*curwin).w_cursor.col += get_cursor_pos_len();
        }

        redo_append_str(c"\n".as_ptr(), 1);
        let i = open_line(
            FORWARD,
            if has_format_option(kFoRetComs) {
                OPENLINE_DO_COM
            } else {
                0
            },
            old_indent,
            ptr::null_mut(),
        );
        old_indent = 0;
        Ins.can_cindent = true;
        foldOpenCursor();
        i
    }
}

unsafe fn ins_digraph() -> c_int {
    unsafe {
        let mut did_putchar = false;

        pc_status = PC_STATUS_UNSET;
        if redrawing() && !char_avail() {
            ins_redraw(false);
            edit_putchar(b'?' as c_int, true);
            did_putchar = true;
            add_to_showcmd_c(Ctrl_K);
        }

        no_mapping += 1;
        allow_keys += 1;
        let mut c = plain_vgetc();
        no_mapping -= 1;
        allow_keys -= 1;
        if did_putchar {
            edit_unputchar();
        }

        if IS_SPECIAL(c) || mod_mask != 0 {
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
            no_mapping += 1;
            allow_keys += 1;
            let cc = plain_vgetc();
            no_mapping -= 1;
            allow_keys -= 1;
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_copychar(lnum: linenr_T) -> c_int {
    unsafe {
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count {
            vim_beep(kOptBoFlagCopy);
            return NUL as c_int;
        }

        validate_virtcol(curwin);
        let end_vcol = (*curwin).w_virtcol;
        let line = ml_get(lnum);

        let mut csarg = std::mem::zeroed::<CharsizeArg>();
        let cstype = init_charsize_arg(&raw mut csarg, curwin, lnum, line);
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

unsafe fn ins_ctrl_ey(tc: c_int) -> c_int {
    unsafe {
        let mut c = tc;
        if ctrl_x_mode_scroll() {
            if c == Ctrl_Y {
                scrolldown_clamp();
            } else {
                scrollup_clamp();
            }
            redraw_later(curwin, UPD_VALID);
        } else {
            c = ins_copychar((*curwin).w_cursor.lnum + if c == Ctrl_Y { -1 } else { 1 });
            if c != NUL as c_int {
                if c < 256 && !(c as u8).is_ascii_alphanumeric() {
                    redo_append_str(c"\x16".as_ptr(), 1);
                }
                let tw_save = (*curbuf).b_p_tw;
                (*curbuf).b_p_tw = -1;
                insert_special(c, 1, 0);
                (*curbuf).b_p_tw = tw_save;
                Ins.revins_chars += 1;
                Ins.revins_legal += 1;
                c = Ctrl_V;
                auto_format(false, true);
            }
        }
        c
    }
}

/// Get the value that `w_virtcol` would have when `'list'` is off.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_nolist_virtcol() -> colnr_T {
    unsafe {
        if (*curwin).w_buffer.is_null()
            || (*(*curwin).w_buffer).b_ml.ml_mfp.is_null()
            || (*curwin).w_cursor.lnum > (*(*curwin).w_buffer).b_ml.ml_line_count
        {
            return 0;
        }
        if (*curwin).w_onebuf_opt.wo_list != 0 && vim_strchr(p_cpo, kCpoListwm).is_null() {
            return getvcol_nolist(&raw mut (*curwin).w_cursor);
        }
        validate_virtcol(curwin);
        (*curwin).w_virtcol
    }
}

unsafe fn do_insert_char_pre(c: c_int) -> *mut c_char {
    unsafe {
        if c == Ctrl_RSB || KeyStuffed != 0 || !has_event(EVENT_INSERTCHARPRE) {
            return ptr::null_mut();
        }

        let mut buf = [0 as c_char; MB_MAXBYTES + 1];
        let save_state = State;
        let buflen = utf_char2bytes(c, (&raw mut buf).cast()) as usize;
        buf[buflen] = NUL;

        textlock += 1;
        set_vim_var_string(VV_CHAR, (&raw const buf).cast(), buflen as isize);

        let mut res: *mut c_char = ptr::null_mut();
        if ins_apply_autocmds(EVENT_INSERTCHARPRE) != 0 {
            let vchar = get_vim_var_str(VV_CHAR);
            if CStr::from_ptr((&raw const buf).cast()) != CStr::from_ptr(vchar) {
                res = xstrdup(vchar);
            }
        }

        set_vim_var_string(VV_CHAR, ptr::null(), -1);
        textlock -= 1;
        State = save_state;
        res
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_can_cindent() -> bool {
    unsafe { Ins.can_cindent }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_can_cindent(val: bool) {
    unsafe {
        Ins.can_cindent = val;
    }
}

/// Trigger `event` and take care of fixing undo.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_apply_autocmds(event: event_T) -> c_int {
    unsafe {
        let tick = buf_get_changedtick(curbuf);
        let r = c_int::from(apply_autocmds(
            event,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        ));
        if event != EVENT_INSERTLEAVE && tick != buf_get_changedtick(curbuf) {
            u_save((*curwin).w_cursor.lnum, (*curwin).w_cursor.lnum + 1);
        }
        r
    }
}
