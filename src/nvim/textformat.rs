//! Text formatting functions (`src/nvim/textformat.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::change::{del_bytes, del_char, get_leader_len, ins_bytes, ins_str, open_line};
#[allow(unused_imports)]
use crate::charset::ascii_isspace;
#[allow(unused_imports)]
use crate::charset::ascii_iswhite;
#[allow(unused_imports)]
use crate::charset::{char2cells, getwhitecols_curline, skipwhite};
#[allow(unused_imports)]
use crate::drawscreen::redraw_curbuf_later;
#[allow(unused_imports)]
use crate::eval::eval_expr_option_number;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_char;
#[allow(unused_imports)]
use crate::eval::vars::{set_vim_var_nr, set_vim_var_string};
use crate::globals::*;
#[allow(unused_imports)]
use crate::indent::{
    change_indent, get_expr_indent, get_indent, get_indent_lnum, get_lisp_indent,
    get_number_indent, set_indent,
};
#[allow(unused_imports)]
use crate::indent_c::{check_linecomment, cindent_on, get_c_indent};
#[allow(unused_imports)]
use crate::input::beep_flush;
#[allow(unused_imports)]
use crate::insert::{
    backspace_until_column, beginline, get_nolist_virtcol, insertchar, set_can_cindent,
    undisplay_dollar,
};
#[allow(unused_imports)]
use crate::mark::mark_col_adjust;
#[allow(unused_imports)]
use crate::mbyte::{utf_allow_break, utf_allow_break_before, utf_iscomposing_first, utf_ptr2char};
#[allow(unused_imports)]
use crate::memline::{ml_get, ml_get_len, ml_replace};
#[allow(unused_imports)]
use crate::message::msgmore;
#[allow(unused_imports)]
use crate::r#move::update_topline;
#[allow(unused_imports)]
use crate::ops::do_join;
#[allow(unused_imports)]
use crate::option::was_set_insecurely;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::os::input::line_breakcheck;
#[allow(unused_imports)]
use crate::strings::strncmp;
#[allow(unused_imports)]
use crate::strings::{vim_strchr, xstrnsave};
#[allow(unused_imports)]
use crate::textobject::startPS;
#[allow(unused_imports)]
use crate::ui::ui_cursor_shape;
#[allow(unused_imports)]
use crate::undo::{u_save, u_save_cursor};
#[allow(unused_imports)]
use crate::window::win_fdccol_count;
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

use crate::api::autocmd::kCallbackNone;
use crate::cursor::{
    check_cursor, check_cursor_col, coladvance, colnr_T, dec_cursor, gchar_cursor,
    get_cursor_line_len, get_cursor_line_ptr, get_cursor_pos_len, get_cursor_pos_ptr, inc_cursor,
    linenr_T, pchar_cursor,
};
use crate::eval::deprecated::pos_T;
use crate::memory::xfree;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const FORWARD: c_int = 1;
const MAXCOL: colnr_T = 0x7fffffff;

const MODE_NORMAL: c_int = 0x01;
const MODE_INSERT: c_int = 0x10;
const VREPLACE_FLAG: c_int = 0x200;

const UPD_VALID: c_int = 10;
const UPD_INVERTED: c_int = 20;

const CMOD_LOCKMARKS: c_int = 0x0800;

const INDENT_SET: c_int = 1;
const SIN_CHANGED: c_int = 1;

const BL_WHITE: c_int = 1;
const BL_FIX: c_int = 4;

const INSCHAR_FORMAT: c_int = 1;
const INSCHAR_DO_COM: c_int = 2;
const INSCHAR_NO_FEX: c_int = 8;
const INSCHAR_COM_LIST: c_int = 16;

const OPENLINE_DELSPACES: c_int = 0x01;
const OPENLINE_DO_COM: c_int = 0x02;
const OPENLINE_KEEPTRAIL: c_int = 0x04;
const OPENLINE_MARKFIX: c_int = 0x08;
const OPENLINE_COM_LIST: c_int = 0x10;
const OPENLINE_FORMAT: c_int = 0x20;

const COM_START: c_char = b's' as c_char;
const COM_MIDDLE: c_char = b'm' as c_char;
const COM_END: c_char = b'e' as c_char;
const COM_FIRST: c_char = b'f' as c_char;

const kFoAuto: c_int = b'a' as c_int;
const kFoInsBlank: c_int = b'b' as c_int;
const kFoInsVi: c_int = b'v' as c_int;
const kFoMbyteBreak: c_int = b'm' as c_int;
const kFoOneLetter: c_int = b'1' as c_int;
const kFoPeriodAbbr: c_int = b'p' as c_int;
const kFoQComs: c_int = b'q' as c_int;
const kFoQNumber: c_int = b'n' as c_int;
const kFoQSecond: c_int = b'2' as c_int;
const kFoRigorousTw: c_int = b']' as c_int;
const kFoWhitePar: c_int = b'w' as c_int;
const kFoWrap: c_int = b't' as c_int;
const kFoWrapComs: c_int = b'c' as c_int;

const OPT_LOCAL: c_int = 0x02;
const kOptFormatexpr: c_int = 115;
const kBufOptFormatexpr: usize = 37;

const VV_COUNT: c_int = 0;
const VV_LNUM: c_int = 9;
const VV_CHAR: c_int = 50;

pub type MotionType = c_int;
pub type InsArrow = c_int;
pub use crate::extmark::TriState;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct oparg_T {
    pub op_type: c_int,
    pub regname: c_int,
    pub motion_type: MotionType,
    pub motion_force: c_int,
    pub use_reg_one: bool,
    pub inclusive: bool,
    pub end_adjusted: bool,
    pub start: pos_T,
    pub end: pos_T,
    pub cursor_start: pos_T,
    pub restore_cursor: bool,
    pub line_count: linenr_T,
    pub empty: bool,
    pub is_VIsual: bool,
    pub from_visual: bool,
    pub start_vcol: colnr_T,
    pub end_vcol: colnr_T,
    pub prev_opcount: c_int,
    pub prev_count0: c_int,
    pub excl_tr_ws: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InsState {
    pub start: pos_T,
    pub start_orig: pos_T,
    pub start_textlen: colnr_T,
    pub start_blank_vcol: colnr_T,
    pub moved: InsArrow,
    pub stop_insert_mode: bool,
    pub can_cindent: bool,
    pub need_undo: bool,
    pub did_ai: bool,
    pub ai_col: colnr_T,
    pub end_comment_pending: c_int,
    pub did_si: bool,
    pub can_si: bool,
    pub can_si_back: bool,
    pub update_start_orig: bool,
    pub new_insert_skip: c_int,
    pub did_restart_edit: c_int,
    pub revins_on: bool,
    pub revins_chars: c_int,
    pub revins_legal: c_int,
    pub revins_scol: c_int,
    pub dont_sync_undo: TriState,
    pub o_lnum: linenr_T,
}

static mut did_add_space: bool = false;

#[inline]
unsafe fn whitechar(cc: c_int) -> bool {
    unsafe {
        ascii_iswhite(cc) && !utf_iscomposing_first(utf_ptr2char(get_cursor_pos_ptr().add(1)))
    }
}

/// Return true if format option `x` is in effect (`has_format_option`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn has_format_option(x: c_int) -> bool {
    unsafe {
        if p_paste != 0 {
            return false;
        }
        !vim_strchr((*curbuf).b_p_fo, x).is_null()
    }
}

/// Format text at the current insert position (`internal_format`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn internal_format(
    textwidth: c_int,
    mut second_indent: c_int,
    flags: c_int,
    format_only: bool,
    c: c_int,
) {
    unsafe {
        let mut cc: c_int;
        let mut save_char: c_char = NUL;
        let mut haveto_redraw = false;
        let fo_ins_blank = has_format_option(kFoInsBlank);
        let fo_multibyte = has_format_option(kFoMbyteBreak);
        let fo_rigor_tw = has_format_option(kFoRigorousTw);
        let fo_white_par = has_format_option(kFoWhitePar);
        let mut first_line = true;
        let mut leader_len: colnr_T;
        let mut no_leader = false;
        let mut do_comments = (flags & INSCHAR_DO_COM) != 0;
        let has_lbr = (*curwin).w_onebuf_opt.wo_lbr;

        (*curwin).w_onebuf_opt.wo_lbr = 0;

        if (*curbuf).b_p_ai == 0 && (State & VREPLACE_FLAG) == 0 {
            cc = gchar_cursor();
            if ascii_iswhite(cc) {
                save_char = cc as c_char;
                pchar_cursor(b'x' as c_char);
            }
        }

        while !got_int {
            let mut startcol: c_int;
            let mut foundcol: c_int;
            let mut end_foundcol: c_int = 0;
            let mut orig_col: c_int = 0;
            let mut saved_text: *mut c_char = ptr::null_mut();
            let mut col: colnr_T;
            let mut did_do_comment = false;

            let virtcol =
                get_nolist_virtcol() + char2cells(if c != 0 { c } else { gchar_cursor() });
            if virtcol <= textwidth {
                break;
            }

            if no_leader {
                do_comments = false;
            } else if (flags & INSCHAR_FORMAT) == 0 && has_format_option(kFoWrapComs) {
                do_comments = true;
            }

            if do_comments {
                let line = get_cursor_line_ptr();
                leader_len = get_leader_len(line, ptr::null_mut(), false, true);
                if leader_len == 0 && (*curbuf).b_p_cin != 0 {
                    let comment_start = check_linecomment(line);
                    if comment_start != MAXCOL {
                        leader_len = get_leader_len(
                            line.add(comment_start as usize),
                            ptr::null_mut(),
                            false,
                            true,
                        );
                        if leader_len != 0 {
                            leader_len += comment_start;
                        }
                    }
                }
            } else {
                leader_len = 0;
            }

            if leader_len == 0 {
                no_leader = true;
            }
            if (flags & INSCHAR_FORMAT) == 0 && leader_len == 0 && !has_format_option(kFoWrap) {
                break;
            }
            startcol = (*curwin).w_cursor.col;
            if startcol == 0 {
                break;
            }

            coladvance(curwin, textwidth);
            let wantcol = (*curwin).w_cursor.col;

            (*curwin).w_cursor.col = startcol;
            foundcol = 0;
            let mut skip_pos: c_int = 0;

            while (!fo_ins_blank && !has_format_option(kFoInsVi))
                || (flags & INSCHAR_FORMAT) != 0
                || (*curwin).w_cursor.lnum != Ins.start.lnum
                || (*curwin).w_cursor.col >= Ins.start.col
            {
                if (*curwin).w_cursor.col == startcol && c != 0 {
                    cc = c;
                } else {
                    cc = gchar_cursor();
                }
                if whitechar(cc) {
                    let end_col = (*curwin).w_cursor.col;

                    let mut wcc = 0;
                    while (*curwin).w_cursor.col > 0 && whitechar(cc) {
                        dec_cursor();
                        cc = gchar_cursor();

                        if wcc < 2 {
                            wcc += 1;
                        }
                    }
                    if (*curwin).w_cursor.col == 0 && whitechar(cc) {
                        break;
                    }

                    if has_format_option(kFoPeriodAbbr) && cc == b'.' as c_int && wcc < 2 {
                        continue;
                    }

                    if (*curwin).w_cursor.col < leader_len {
                        break;
                    }

                    if has_format_option(kFoOneLetter) {
                        if (*curwin).w_cursor.col == 0 {
                            break;
                        }
                        if (*curwin).w_cursor.col <= leader_len {
                            break;
                        }
                        col = (*curwin).w_cursor.col;
                        dec_cursor();
                        cc = gchar_cursor();

                        if whitechar(cc) {
                            continue;
                        }
                        (*curwin).w_cursor.col = col;
                    }

                    inc_cursor();

                    end_foundcol = end_col + 1;
                    foundcol = (*curwin).w_cursor.col;
                    if (*curwin).w_cursor.col <= wantcol {
                        break;
                    }
                } else if (cc >= 0x100 || !utf_allow_break_before(cc)) && fo_multibyte {
                    let mut ncc: c_int;
                    let mut allow_break: bool;

                    if (*curwin).w_cursor.col != startcol {
                        if (*curwin).w_cursor.col < leader_len {
                            break;
                        }
                        col = (*curwin).w_cursor.col;
                        inc_cursor();
                        ncc = gchar_cursor();
                        allow_break = utf_allow_break(cc, ncc);

                        if (*curwin).w_cursor.col != skip_pos && allow_break {
                            foundcol = (*curwin).w_cursor.col;
                            end_foundcol = foundcol;
                            if (*curwin).w_cursor.col <= wantcol {
                                break;
                            }
                        }
                        (*curwin).w_cursor.col = col;
                    }

                    if (*curwin).w_cursor.col == 0 {
                        break;
                    }

                    ncc = cc;
                    col = (*curwin).w_cursor.col;

                    dec_cursor();
                    cc = gchar_cursor();

                    if whitechar(cc) {
                        continue;
                    }
                    if (*curwin).w_cursor.col < leader_len {
                        break;
                    }

                    (*curwin).w_cursor.col = col;
                    skip_pos = (*curwin).w_cursor.col;

                    allow_break = utf_allow_break(cc, ncc);

                    if allow_break {
                        foundcol = (*curwin).w_cursor.col;
                        end_foundcol = foundcol;
                    }
                    if (*curwin).w_cursor.col <= wantcol {
                        let ncc_allow_break = utf_allow_break_before(ncc);

                        if allow_break {
                            break;
                        }
                        if !ncc_allow_break && !fo_rigor_tw {
                            if (*curwin).w_cursor.col == startcol {
                                end_foundcol = 0;
                                foundcol = 0;
                                break;
                            }

                            col = (*curwin).w_cursor.col;

                            inc_cursor();
                            cc = ncc;
                            ncc = gchar_cursor();
                            ncc = if ncc != 0 { ncc } else { c };

                            allow_break = utf_allow_break(cc, ncc);

                            if allow_break {
                                foundcol = if ncc == 0 { 0 } else { (*curwin).w_cursor.col };
                                end_foundcol = foundcol;
                                break;
                            }
                            (*curwin).w_cursor.col = col;
                        }
                    }
                }
                if (*curwin).w_cursor.col == 0 {
                    break;
                }
                dec_cursor();
            }

            if foundcol == 0 {
                (*curwin).w_cursor.col = startcol;
                break;
            }

            undisplay_dollar();

            if (State & VREPLACE_FLAG) != 0 {
                orig_col = startcol;
            } else {
                replace_offset = startcol - end_foundcol;
            }

            (*curwin).w_cursor.col = foundcol;
            while {
                cc = gchar_cursor();
                whitechar(cc)
            } && (!fo_white_par || (*curwin).w_cursor.col < startcol)
            {
                inc_cursor();
            }
            startcol -= (*curwin).w_cursor.col;
            startcol = startcol.max(0);

            if (State & VREPLACE_FLAG) != 0 {
                saved_text = xstrnsave(get_cursor_pos_ptr(), get_cursor_pos_len() as usize);
                (*curwin).w_cursor.col = orig_col;
                *saved_text.add(startcol as usize) = NUL;

                if !fo_white_par {
                    backspace_until_column(foundcol);
                }
            } else if !fo_white_par {
                (*curwin).w_cursor.col = foundcol;
            }

            open_line(
                FORWARD,
                OPENLINE_DELSPACES
                    + OPENLINE_MARKFIX
                    + if fo_white_par { OPENLINE_KEEPTRAIL } else { 0 }
                    + if do_comments { OPENLINE_DO_COM } else { 0 }
                    + OPENLINE_FORMAT
                    + if (flags & INSCHAR_COM_LIST) != 0 {
                        OPENLINE_COM_LIST
                    } else {
                        0
                    },
                if (flags & INSCHAR_COM_LIST) != 0 {
                    second_indent
                } else {
                    old_indent
                },
                &raw mut did_do_comment,
            );
            if (flags & INSCHAR_COM_LIST) == 0 {
                old_indent = 0;
            }

            if did_do_comment {
                no_leader = false;
            }

            replace_offset = 0;
            if first_line {
                if (flags & INSCHAR_COM_LIST) == 0 {
                    if second_indent < 0 && has_format_option(kFoQNumber) {
                        second_indent = get_number_indent((*curwin).w_cursor.lnum - 1);
                    }
                    if second_indent >= 0 {
                        if (State & VREPLACE_FLAG) != 0 {
                            change_indent(INDENT_SET, second_indent, 0, true);
                        } else if leader_len > 0 && second_indent - leader_len > 0 {
                            let padding = second_indent - leader_len;
                            let space = c" ".as_ptr() as *mut c_char;
                            for _ in 0..padding {
                                ins_str(space, 1);
                            }
                        } else {
                            set_indent(second_indent, SIN_CHANGED);
                        }
                    }
                }
                first_line = false;
            }

            if (State & VREPLACE_FLAG) != 0 {
                ins_bytes(saved_text);
                xfree(saved_text.cast());
            } else {
                (*curwin).w_cursor.col += startcol;
                let len = get_cursor_line_len();
                (*curwin).w_cursor.col = (*curwin).w_cursor.col.min(len);
            }

            haveto_redraw = true;
            set_can_cindent(true);
            Ins.did_ai = false;
            Ins.did_si = false;
            Ins.can_si = false;
            Ins.can_si_back = false;
            line_breakcheck();
        }

        if save_char != NUL {
            pchar_cursor(save_char);
        }

        (*curwin).w_onebuf_opt.wo_lbr = has_lbr;

        if !format_only && haveto_redraw {
            update_topline(curwin);
            redraw_curbuf_later(UPD_VALID);
        }
    }
}

unsafe fn fmt_check_par(
    lnum: linenr_T,
    leader_len: *mut c_int,
    leader_flags: *mut *mut c_char,
    do_comments: bool,
) -> bool {
    unsafe {
        let mut flags: *mut c_char = ptr::null_mut();
        let ptr = ml_get(lnum);
        if do_comments {
            *leader_len = get_leader_len(ptr, leader_flags, false, true);
        } else {
            *leader_len = 0;
        }

        if *leader_len > 0 {
            flags = *leader_flags;
            while *flags != NUL && *flags != b':' as c_char && *flags != COM_END {
                flags = flags.add(1);
            }
        }

        *skipwhite(ptr.add(*leader_len as usize)) == NUL
            || (*leader_len > 0 && *flags == COM_END)
            || startPS(lnum, 0, false)
    }
}

unsafe fn ends_in_white(lnum: linenr_T) -> bool {
    unsafe {
        let s = ml_get(lnum);
        if *s == NUL {
            return false;
        }
        let l = ml_get_len(lnum) - 1;
        ascii_iswhite(*s.add(l as usize) as u8 as c_int)
    }
}

unsafe fn same_leader(
    lnum: linenr_T,
    leader1_len: c_int,
    leader1_flags: *mut c_char,
    leader2_len: c_int,
    leader2_flags: *mut c_char,
) -> bool {
    unsafe {
        if leader1_len == 0 {
            return leader2_len == 0;
        }

        if !leader1_flags.is_null() {
            let mut p = leader1_flags;
            while *p != NUL && *p != b':' as c_char {
                if *p == COM_FIRST {
                    return leader2_len == 0;
                }
                if *p == COM_END {
                    return false;
                }
                if *p == COM_START {
                    let line_len = ml_get_len(lnum);
                    if line_len <= leader1_len {
                        return false;
                    }
                    if leader2_flags.is_null() || leader2_len == 0 {
                        return false;
                    }
                    let mut p2 = leader2_flags;
                    while *p2 != NUL && *p2 != b':' as c_char {
                        if *p2 == COM_MIDDLE {
                            return true;
                        }
                        p2 = p2.add(1);
                    }
                    return false;
                }
                p = p.add(1);
            }
        }

        let line1 = xstrnsave(ml_get(lnum), ml_get_len(lnum) as usize);
        let mut idx1: c_int = 0;
        while ascii_iswhite(*line1.add(idx1 as usize) as c_int) {
            idx1 += 1;
        }
        let line2 = ml_get(lnum + 1);
        let mut idx2: c_int = 0;
        while idx2 < leader2_len {
            if !ascii_iswhite(*line2.add(idx2 as usize) as c_int) {
                let c1 = *line1.add(idx1 as usize);
                idx1 += 1;
                if c1 != *line2.add(idx2 as usize) {
                    break;
                }
            } else {
                while ascii_iswhite(*line1.add(idx1 as usize) as c_int) {
                    idx1 += 1;
                }
            }
            idx2 += 1;
        }
        xfree(line1.cast());

        idx2 == leader2_len && idx1 == leader1_len
    }
}

unsafe fn paragraph_start(lnum: linenr_T) -> bool {
    unsafe {
        let mut leader_len: c_int = 0;
        let mut leader_flags: *mut c_char = ptr::null_mut();
        let mut next_leader_len: c_int = 0;
        let mut next_leader_flags: *mut c_char = ptr::null_mut();

        if lnum <= 1 {
            return true;
        }
        let p = ml_get(lnum - 1);
        if *p == NUL {
            return true;
        }
        let do_comments = has_format_option(kFoQComs);
        if fmt_check_par(
            lnum - 1,
            &raw mut leader_len,
            &raw mut leader_flags,
            do_comments,
        ) {
            return true;
        }

        if fmt_check_par(
            lnum,
            &raw mut next_leader_len,
            &raw mut next_leader_flags,
            do_comments,
        ) {
            return true;
        }

        if has_format_option(kFoWhitePar) && !ends_in_white(lnum - 1) {
            return true;
        }
        if has_format_option(kFoQNumber) && get_number_indent(lnum) > 0 {
            return true;
        }
        if !same_leader(
            lnum - 1,
            leader_len,
            leader_flags,
            next_leader_len,
            next_leader_flags,
        ) {
            return true;
        }
        false
    }
}

/// Auto-format from the current line until the end of the paragraph (`auto_format`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn auto_format(trailblank: bool, prev_line: bool) {
    unsafe {
        if !has_format_option(kFoAuto) {
            return;
        }

        let pos = (*curwin).w_cursor;
        let old = get_cursor_line_ptr();

        check_auto_format(false);

        let wasatend = pos.col == get_cursor_line_len();
        if *old != NUL && !trailblank && wasatend {
            dec_cursor();
            let mut cc = gchar_cursor();
            if !whitechar(cc) && (*curwin).w_cursor.col > 0 && has_format_option(kFoOneLetter) {
                dec_cursor();
            }
            cc = gchar_cursor();
            if whitechar(cc) {
                (*curwin).w_cursor = pos;
                return;
            }
            (*curwin).w_cursor = pos;
        }

        if *old != NUL && !trailblank && !wasatend && pos.col > 0 && (State & MODE_INSERT) != 0 {
            let line = get_cursor_line_ptr();
            if whitechar(*line.add((pos.col - 1) as usize) as c_int) {
                (*curwin).w_cursor = pos;
                return;
            }
        }

        if has_format_option(kFoWrapComs)
            && !has_format_option(kFoWrap)
            && get_leader_len(old, ptr::null_mut(), false, true) == 0
        {
            return;
        }

        if prev_line && !paragraph_start((*curwin).w_cursor.lnum) {
            (*curwin).w_cursor.lnum -= 1;
            if u_save_cursor() == FAIL {
                return;
            }
        }

        saved_cursor = pos;
        format_lines(-1, false);
        (*curwin).w_cursor = saved_cursor;
        saved_cursor.lnum = 0;

        if (*curwin).w_cursor.lnum > (*curbuf).b_ml.ml_line_count {
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
            coladvance(curwin, MAXCOL);
        } else {
            check_cursor_col(curwin);
        }

        if !wasatend && has_format_option(kFoWhitePar) {
            let linep = get_cursor_line_ptr();
            let len = get_cursor_line_len();
            if (*curwin).w_cursor.col == len {
                let plinep = xstrnsave(linep, len as usize + 2);
                *plinep.add(len as usize) = b' ' as c_char;
                *plinep.add(len as usize + 1) = NUL;
                ml_replace((*curwin).w_cursor.lnum, plinep, false);
                did_add_space = true;
            } else {
                check_auto_format(false);
            }
        }

        check_cursor(curwin);
    }
}

/// Delete extra space added to continue a paragraph for auto-formatting (`check_auto_format`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_auto_format(end_insert: bool) {
    unsafe {
        if !did_add_space {
            return;
        }

        let cc = gchar_cursor();
        if !whitechar(cc) {
            did_add_space = false;
        } else {
            let mut c = b' ' as c_int;
            if !end_insert {
                inc_cursor();
                c = gchar_cursor();
                dec_cursor();
            }
            if c != 0 {
                del_char(false);
                did_add_space = false;
            }
        }
    }
}

/// Find out textwidth to be used for formatting (`comp_textwidth`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn comp_textwidth(ff: bool) -> c_int {
    unsafe {
        let mut textwidth = (*curbuf).b_p_tw as c_int;
        if textwidth == 0 && (*curbuf).b_p_wm != 0 {
            textwidth = (*curwin).w_view_width - (*curbuf).b_p_wm as c_int;
            textwidth -= win_fdccol_count(curwin);
            textwidth -= (*curwin).w_scwidth;

            if (*curwin).w_onebuf_opt.wo_nu != 0 || (*curwin).w_onebuf_opt.wo_rnu != 0 {
                textwidth -= 8;
            }
        }
        textwidth = textwidth.max(0);
        if ff && textwidth == 0 {
            textwidth = ((*curwin).w_view_width - 1).min(79);
        }
        textwidth
    }
}

/// Implementation of the format operator `gq` (`op_format`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_format(oap: *mut oparg_T, keep_cursor: bool) {
    unsafe {
        let mut old_line_count = (*curbuf).b_ml.ml_line_count;

        (*curwin).w_cursor = (*oap).cursor_start;

        if u_save((*oap).start.lnum - 1, (*oap).end.lnum + 1) == FAIL {
            return;
        }
        (*curwin).w_cursor = (*oap).start;

        if (*oap).is_VIsual {
            redraw_curbuf_later(UPD_INVERTED);
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start = (*oap).start;
        }

        if keep_cursor {
            saved_cursor = (*oap).cursor_start;
        }

        format_lines((*oap).line_count, keep_cursor);

        if (*oap).end_adjusted && (*curwin).w_cursor.lnum < (*curbuf).b_ml.ml_line_count {
            (*curwin).w_cursor.lnum += 1;
        }
        beginline(BL_WHITE | BL_FIX);
        old_line_count = (*curbuf).b_ml.ml_line_count - old_line_count;
        msgmore(old_line_count);

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_end = (*curwin).w_cursor;
        }

        if keep_cursor {
            (*curwin).w_cursor = saved_cursor;
            saved_cursor.lnum = 0;
            check_cursor(curwin);
        }

        if (*oap).is_VIsual {
            let mut wp = firstwin;
            while !wp.is_null() {
                if (*wp).w_old_cursor_lnum != 0 {
                    if (*wp).w_old_cursor_lnum > (*wp).w_old_visual_lnum {
                        (*wp).w_old_cursor_lnum += old_line_count;
                    } else {
                        (*wp).w_old_visual_lnum += old_line_count;
                    }
                }
                wp = (*wp).w_next;
            }
        }
    }
}

/// Implementation of the format operator `gq` for when using `'formatexpr'` (`op_formatexpr`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_formatexpr(oap: *mut oparg_T) {
    unsafe {
        if (*oap).is_VIsual {
            redraw_curbuf_later(UPD_INVERTED);
        }

        if fex_format((*oap).start.lnum, c_long::from((*oap).line_count), 0) != 0 {
            op_format(oap, false);
        }
    }
}

/// Evaluate `'formatexpr'` (`fex_format`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fex_format(lnum: linenr_T, count: c_long, c: c_int) -> c_int {
    unsafe {
        let use_sandbox = was_set_insecurely(curwin, kOptFormatexpr, OPT_LOCAL) != 0;
        let save_sctx = current_sctx;

        set_vim_var_nr(VV_LNUM, i64::from(lnum));
        set_vim_var_nr(VV_COUNT, i64::from(count));
        set_vim_var_char(c);

        current_sctx = (*curbuf).b_p_script_ctx[kBufOptFormatexpr];

        let r = eval_expr_option_number(&raw mut (*curbuf).b_p_fex, use_sandbox);

        set_vim_var_string(VV_CHAR, ptr::null(), -1);
        current_sctx = save_sctx;

        r
    }
}

/// Format `line_count` lines starting at the cursor position (`format_lines`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn format_lines(line_count: linenr_T, avoid_fex: bool) {
    unsafe {
        let mut is_not_par: bool;
        let mut next_is_not_par: bool;
        let mut is_end_par: bool;
        let mut prev_is_end_par: bool;
        let mut next_is_start_par = false;
        let mut leader_len: c_int = 0;
        let mut next_leader_len: c_int = 0;
        let mut leader_flags: *mut c_char = ptr::null_mut();
        let mut next_leader_flags: *mut c_char = ptr::null_mut();
        let mut advance = true;
        let mut second_indent: c_int = -1;
        let mut first_par_line = true;
        let mut need_set_indent = true;
        let first_line = (*curwin).w_cursor.lnum;
        let mut force_format = false;
        let old_State = State;

        let max_len = comp_textwidth(true) * 3;

        let do_comments = has_format_option(kFoQComs);
        let mut do_comments_list: c_int = 0;
        let do_second_indent = has_format_option(kFoQSecond);
        let do_number_indent = has_format_option(kFoQNumber);
        let do_trail_white = has_format_option(kFoWhitePar);

        if (*curwin).w_cursor.lnum > 1 {
            is_not_par = fmt_check_par(
                (*curwin).w_cursor.lnum - 1,
                &raw mut leader_len,
                &raw mut leader_flags,
                do_comments,
            );
        } else {
            is_not_par = true;
        }
        next_is_not_par = fmt_check_par(
            (*curwin).w_cursor.lnum,
            &raw mut next_leader_len,
            &raw mut next_leader_flags,
            do_comments,
        );
        is_end_par = is_not_par || next_is_not_par;
        if !is_end_par && do_trail_white {
            is_end_par = !ends_in_white((*curwin).w_cursor.lnum - 1);
        }

        (*curwin).w_cursor.lnum -= 1;
        let mut count = i64::from(line_count);
        while count != 0 && !got_int {
            if advance {
                (*curwin).w_cursor.lnum += 1;
                prev_is_end_par = is_end_par;
                is_not_par = next_is_not_par;
                leader_len = next_leader_len;
                leader_flags = next_leader_flags;
            } else {
                prev_is_end_par = is_end_par;
            }

            if count == 1 || (*curwin).w_cursor.lnum == (*curbuf).b_ml.ml_line_count {
                next_is_not_par = true;
                next_leader_len = 0;
                next_leader_flags = ptr::null_mut();
            } else {
                next_is_not_par = fmt_check_par(
                    (*curwin).w_cursor.lnum + 1,
                    &raw mut next_leader_len,
                    &raw mut next_leader_flags,
                    do_comments,
                );
                if do_number_indent {
                    next_is_start_par = get_number_indent((*curwin).w_cursor.lnum + 1) > 0;
                }
            }
            advance = true;
            is_end_par = is_not_par || next_is_not_par || next_is_start_par;
            if !is_end_par && do_trail_white {
                is_end_par = !ends_in_white((*curwin).w_cursor.lnum);
            }

            if is_not_par {
                if line_count < 0 {
                    break;
                }
            } else {
                if first_par_line
                    && (do_second_indent || do_number_indent)
                    && prev_is_end_par
                    && (*curwin).w_cursor.lnum < (*curbuf).b_ml.ml_line_count
                {
                    if do_second_indent && *ml_get((*curwin).w_cursor.lnum + 1) != NUL {
                        if leader_len == 0 && next_leader_len == 0 {
                            second_indent = get_indent_lnum((*curwin).w_cursor.lnum + 1);
                        } else {
                            second_indent = next_leader_len;
                            do_comments_list = 1;
                        }
                    } else if do_number_indent {
                        if leader_len == 0 && next_leader_len == 0 {
                            second_indent = get_number_indent((*curwin).w_cursor.lnum);
                        } else {
                            second_indent = get_number_indent((*curwin).w_cursor.lnum);
                            do_comments_list = 1;
                        }
                    }
                }

                if ((*curwin).w_cursor.lnum >= (*curbuf).b_ml.ml_line_count
                    || !same_leader(
                        (*curwin).w_cursor.lnum,
                        leader_len,
                        leader_flags,
                        next_leader_len,
                        next_leader_flags,
                    ))
                    && (next_leader_flags.is_null()
                        || strncmp(next_leader_flags, c"://".as_ptr(), 3) != 0
                        || check_linecomment(get_cursor_line_ptr()) == MAXCOL)
                {
                    is_end_par = true;
                }

                if is_end_par || force_format {
                    if need_set_indent {
                        let indent = if (*curwin).w_cursor.lnum == first_line {
                            get_indent()
                        } else if (*curbuf).b_p_lisp != 0 {
                            get_lisp_indent()
                        } else if cindent_on() {
                            if (*curbuf).b_p_inde.cb_type != kCallbackNone {
                                get_expr_indent()
                            } else {
                                get_c_indent()
                            }
                        } else {
                            get_indent()
                        };
                        set_indent(indent, SIN_CHANGED);
                    }

                    State = MODE_NORMAL;
                    coladvance(curwin, MAXCOL);
                    while (*curwin).w_cursor.col != 0 && ascii_isspace(gchar_cursor()) {
                        dec_cursor();
                    }

                    State = MODE_INSERT;
                    let smd_save = p_smd;
                    p_smd = 0;

                    insertchar(
                        0,
                        INSCHAR_FORMAT
                            + if do_comments { INSCHAR_DO_COM } else { 0 }
                            + if do_comments && do_comments_list != 0 {
                                INSCHAR_COM_LIST
                            } else {
                                0
                            }
                            + if avoid_fex { INSCHAR_NO_FEX } else { 0 },
                        second_indent,
                    );

                    State = old_State;
                    p_smd = smd_save;
                    ui_cursor_shape();

                    second_indent = -1;
                    need_set_indent = is_end_par;
                    if is_end_par {
                        if line_count < 0 {
                            break;
                        }
                        first_par_line = true;
                    }
                    force_format = false;
                }

                if !is_end_par {
                    advance = false;
                    (*curwin).w_cursor.lnum += 1;
                    (*curwin).w_cursor.col = 0;
                    if line_count < 0 && u_save_cursor() == FAIL {
                        break;
                    }
                    if next_leader_len > 0 {
                        del_bytes(next_leader_len, false, false);
                        mark_col_adjust((*curwin).w_cursor.lnum, 0, 0, -next_leader_len, 0);
                    } else if second_indent > 0 {
                        let indent = getwhitecols_curline() as c_int;
                        if indent > 0 {
                            del_bytes(indent, false, false);
                            mark_col_adjust((*curwin).w_cursor.lnum, 0, 0, -indent, 0);
                        }
                    }
                    (*curwin).w_cursor.lnum -= 1;
                    if do_join(2, true, false, false, false) == FAIL {
                        beep_flush();
                        break;
                    }
                    first_par_line = false;
                    force_format = get_cursor_line_len() > max_len;
                }
            }
            line_breakcheck();
            count -= 1;
        }
    }
}
