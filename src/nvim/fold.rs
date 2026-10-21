//! Code for folding (`src/nvim/fold.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::extmark::parse_virt_text;
use crate::api::private::helpers::{api_clear_error, api_free_object};
use crate::api::private::validate::{Error, ErrorType, Object, ObjectData, ObjectType};
use crate::buffer::{MAXLNUM, buf_T};
use crate::buffer_updates::buf_updates_send_changes;
use crate::change::{TAB, changed_lines};
use crate::charset::{NUL, ascii_isdigit, ascii_iswhite, skipwhite, transstr, vim_isprintc};
use crate::cursor::{check_cursor_col, colnr_T, linenr_T, ltoreq, pos_T};
use crate::decoration::{VirtText, clear_virttext, next_virt_text_chunk};
use crate::diff::{diff_infold, diff_lnum_win};
use crate::drawscreen::{
    UPD_INVERTED, UPD_NOT_VALID, redraw_buf_later, redraw_later, redraw_win_range_later,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, VarType, kCallbackNone, tv_get_lnum, typval_T, varnumber_T,
};
use crate::eval::vars::{
    VV_FOLDDASHES, VV_FOLDEND, VV_FOLDLEVEL, VV_FOLDSTART, VV_LNUM, get_vim_var_nr,
    get_vim_var_str, set_vim_var_nr, set_vim_var_string,
};
use crate::eval::{eval_foldexpr, eval_foldtext};
use crate::ex_session::{put_eol, put_line};
use crate::extmark::{TriState, extmark_splice_cols, kExtmarkUndo, kFalse, kNone, kTrue};
use crate::fileio::{FILE, fputs};
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::grid::wline_T;
use crate::indent::{get_indent_buf, get_sw_value};
use crate::mark::setpcmark;
use crate::math::{FAIL, OK};
use crate::mbyte::{mb_adjust_cursor, ptr2cells, utf_ptr2char, utfc_ptr2len};
use crate::mcursor::mc_replaying;
use crate::memline::{ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_replace_buf};
use crate::memory::{xfree, xmalloc, xmemcpyz, xstrdup};
use crate::message::emsg;
use crate::r#move::changed_window_setting;
use crate::ops::skip_comment;
use crate::option_vars::*;
use crate::os::input::line_breakcheck;
use crate::plines::plines_win_nofold;
use crate::search::{FORWARD, linewhite};
use crate::statusline::foldinfo_T;
use crate::strings::{atoi, concat_str, gettext, strstr, vim_snprintf, vim_strchr};
use crate::syntax::syn_get_foldlevel;
use crate::undo::u_save;
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use std::ptr;

use crate::state::MODE_INSERT;

pub const FOLD_TEXT_LEN: usize = 51;
const MAX_LEVEL: c_int = 20;

const FD_OPEN: c_char = 0;
const FD_CLOSED: c_char = 1;
const FD_LEVEL: c_char = 2;

const DONE_NOTHING: c_int = 0;
const DONE_ACTION: c_int = 1;
const DONE_FOLD: c_int = 2;

const kWinOptFoldtext: usize = 22;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LevelGetterKind {
    Marker,
    Expr,
    Syntax,
    Diff,
    Indent,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct fold_T {
    fd_top: linenr_T,
    fd_len: linenr_T,
    fd_nested: garray_T,
    fd_flags: c_char,
    fd_small: TriState,
}

struct fline_T {
    wp: *mut win_T,
    lnum: linenr_T,
    off: linenr_T,
    lnum_save: linenr_T,
    lvl: c_int,
    lvl_next: c_int,
    start: c_int,
    end: c_int,
    had_end: c_int,
}

static mut fold_changed: bool = false;
static mut invalid_top: linenr_T = 0;
static mut invalid_bot: linenr_T = 0;
static mut prev_lnum: linenr_T = 0;
static mut prev_lnum_lvl: c_int = -1;

static mut foldstartmarkerlen: usize = 0;
static mut foldendmarker: *mut c_char = ptr::null_mut();
static mut foldendmarkerlen: usize = 0;

static mut got_fdt_error: bool = false;
static mut last_wp: *mut win_T = ptr::null_mut();
static mut fdt_last_lnum: linenr_T = 0;
static mut fdt_result_entered: bool = false;

static E_NOFOLD: &CStr = c"E490: No fold found";

#[inline]
unsafe fn ngettext_wrap(msgid1: *const c_char, msgid2: *const c_char, n: c_ulong) -> *mut c_char {
    unsafe { crate::strings::ngettext(msgid1, msgid2, n) }
}

#[inline]
unsafe fn redraw_curbuf_later(type_: c_int) {
    unsafe {
        redraw_buf_later((*curwin).w_buffer, type_);
    }
}

/// Copy that folding state from window "wp_from" to window "wp_to".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copyFoldingState(wp_from: *mut win_T, wp_to: *mut win_T) {
    unsafe {
        (*wp_to).w_fold_manual = (*wp_from).w_fold_manual;
        (*wp_to).w_foldinvalid = (*wp_from).w_foldinvalid;
        cloneFoldGrowArray(&mut (*wp_from).w_folds, &mut (*wp_to).w_folds);
    }
}

/// True if there may be folded lines in window `win`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hasAnyFolding(win: *mut win_T) -> c_int {
    unsafe {
        c_int::from(
            (*(*win).w_buffer).terminal.is_null()
                && (*win).w_onebuf_opt.wo_fen != 0
                && !mc_replaying()
                && (!foldmethodIsManual(win) || (*win).w_folds.ga_len > 0),
        )
    }
}

/// Return true if line "lnum" in window "win" is part of a closed fold.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hasFolding(
    win: *mut win_T,
    lnum: linenr_T,
    firstp: *mut linenr_T,
    lastp: *mut linenr_T,
) -> bool {
    unsafe { hasFoldingWin(win, lnum, firstp, lastp, true, ptr::null_mut()) }
}

/// Search folds starting at lnum.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hasFoldingWin(
    win: *mut win_T,
    lnum: linenr_T,
    firstp: *mut linenr_T,
    lastp: *mut linenr_T,
    cache: bool,
    infop: *mut foldinfo_T,
) -> bool {
    unsafe {
        checkupdate(win);

        if hasAnyFolding(win) == 0 {
            if !infop.is_null() {
                (*infop).fi_level = 0;
            }
            return false;
        }

        let mut had_folded = false;
        let mut first: linenr_T = 0;
        let mut last: linenr_T = 0;

        if cache {
            let x = find_wl_entry(win, lnum);
            if x >= 0 {
                let wl = &*((*win).w_lines as *const wline_T).add(x as usize);
                first = wl.wl_lnum;
                last = wl.wl_foldend;
                had_folded = wl.wl_folded;
            }
        }

        let mut lnum_rel = lnum;
        let mut level: c_int = 0;
        let mut low_level: c_int = 0;
        let mut fp: *mut fold_T = ptr::null_mut();
        let mut maybe_small = false;
        let mut use_level = false;

        if first == 0 {
            let mut gap = &mut (*win).w_folds as *mut garray_T;
            loop {
                if !foldFind(gap, lnum_rel, &mut fp) {
                    break;
                }

                if lnum_rel == (*fp).fd_top && low_level == 0 {
                    low_level = level + 1;
                }

                first += (*fp).fd_top;
                last += (*fp).fd_top;

                had_folded = check_closed(
                    win,
                    fp,
                    &mut use_level,
                    level,
                    &mut maybe_small,
                    lnum - lnum_rel,
                );
                if had_folded {
                    last += (*fp).fd_len - 1;
                    break;
                }

                gap = &mut (*fp).fd_nested;
                lnum_rel -= (*fp).fd_top;
                level += 1;
            }
        }

        if !had_folded {
            if !infop.is_null() {
                (*infop).fi_level = level;
                (*infop).fi_lnum = lnum - lnum_rel;
                (*infop).fi_low_level = if low_level == 0 { level } else { low_level };
            }
            return false;
        }

        last = last.min((*(*win).w_buffer).b_ml.ml_line_count);
        if !lastp.is_null() {
            *lastp = last;
        }
        if !firstp.is_null() {
            *firstp = first;
        }
        if !infop.is_null() {
            (*infop).fi_level = level + 1;
            (*infop).fi_lnum = first;
            (*infop).fi_low_level = if low_level == 0 { level + 1 } else { low_level };
        }
        true
    }
}

unsafe fn foldLevel(lnum: linenr_T) -> c_int {
    unsafe {
        if invalid_top == 0 {
            checkupdate(curwin);
        } else if lnum == prev_lnum && prev_lnum_lvl >= 0 {
            return prev_lnum_lvl;
        } else if lnum >= invalid_top && lnum <= invalid_bot {
            return -1;
        }

        if hasAnyFolding(curwin) == 0 {
            return 0;
        }

        foldLevelWin(curwin, lnum)
    }
}

/// Low level function to check if a line is folded.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn lineFolded(win: *mut win_T, lnum: linenr_T) -> bool {
    unsafe { fold_info(win, lnum).fi_lines != 0 }
}

/// Count the number of lines that are folded at line number "lnum".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fold_info(win: *mut win_T, lnum: linenr_T) -> foldinfo_T {
    unsafe {
        let mut info = foldinfo_T {
            fi_lnum: 0,
            fi_level: 0,
            fi_low_level: 0,
            fi_lines: 0,
        };
        let mut last: linenr_T = 0;

        if hasFoldingWin(win, lnum, ptr::null_mut(), &mut last, false, &mut info) {
            info.fi_lines = last - lnum + 1;
        } else {
            info.fi_lines = 0;
        }

        info
    }
}

/// Return true if 'foldmethod' is "manual".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldmethodIsManual(wp: *mut win_T) -> bool {
    unsafe {
        let fdm = (*wp).w_onebuf_opt.wo_fdm;
        *fdm != NUL && *fdm.add(3) == b'u' as c_char
    }
}

/// Return true if 'foldmethod' is "indent".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldmethodIsIndent(wp: *mut win_T) -> bool {
    unsafe { *(*wp).w_onebuf_opt.wo_fdm == b'i' as c_char }
}

/// Return true if 'foldmethod' is "expr".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldmethodIsExpr(wp: *mut win_T) -> bool {
    unsafe {
        let fdm = (*wp).w_onebuf_opt.wo_fdm;
        *fdm != NUL && *fdm.add(1) == b'x' as c_char
    }
}

/// Return true if 'foldmethod' is "marker".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldmethodIsMarker(wp: *mut win_T) -> bool {
    unsafe {
        let fdm = (*wp).w_onebuf_opt.wo_fdm;
        *fdm != NUL && *fdm.add(2) == b'r' as c_char
    }
}

/// Return true if 'foldmethod' is "syntax".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldmethodIsSyntax(wp: *mut win_T) -> bool {
    unsafe { *(*wp).w_onebuf_opt.wo_fdm == b's' as c_char }
}

/// Return true if 'foldmethod' is "diff".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldmethodIsDiff(wp: *mut win_T) -> bool {
    unsafe { *(*wp).w_onebuf_opt.wo_fdm == b'd' as c_char }
}

/// Close fold for current window at position "pos".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn closeFold(pos: pos_T, count: c_int) {
    unsafe {
        setFoldRepeat(pos, count, false);
    }
}

/// Close fold for current window at position `pos` recursively.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn closeFoldRecurse(pos: pos_T) {
    unsafe {
        setManualFold(pos, false, true, ptr::null_mut());
    }
}

/// Open or Close folds for current window in lines "first" to "last".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn opFoldRange(
    firstpos: pos_T,
    lastpos: pos_T,
    opening: c_int,
    recurse: c_int,
    had_visual: bool,
) {
    unsafe {
        let mut done = DONE_NOTHING;
        let first = firstpos.lnum;
        let last = lastpos.lnum;
        let mut lnum = first;

        while lnum <= last {
            let temp = pos_T {
                lnum,
                col: 0,
                coladd: 0,
            };
            let mut lnum_next = lnum;
            if opening != 0 && recurse == 0 {
                hasFolding(curwin, lnum, ptr::null_mut(), &mut lnum_next);
            }
            setManualFold(temp, opening != 0, recurse != 0, &mut done);
            if opening == 0 && recurse == 0 {
                hasFolding(curwin, lnum, ptr::null_mut(), &mut lnum_next);
            }
            lnum = lnum_next + 1;
        }
        if done == DONE_NOTHING {
            emsg(gettext(E_NOFOLD.as_ptr()));
        }
        if had_visual {
            redraw_curbuf_later(UPD_INVERTED);
        }
    }
}

/// Open fold for current window at position "pos".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn openFold(pos: pos_T, count: c_int) {
    unsafe {
        setFoldRepeat(pos, count, true);
    }
}

/// Open fold for current window at position `pos` recursively.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn openFoldRecurse(pos: pos_T) {
    unsafe {
        setManualFold(pos, true, true, ptr::null_mut());
    }
}

/// Open folds until the cursor line is not in a closed fold.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldOpenCursor() {
    unsafe {
        checkupdate(curwin);
        if hasAnyFolding(curwin) != 0 {
            loop {
                let mut done = DONE_NOTHING;
                setManualFold((*curwin).w_cursor, true, false, &mut done);
                if (done & DONE_ACTION) == 0 {
                    break;
                }
            }
        }
    }
}

/// Set new foldlevel for current window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn newFoldLevel() {
    unsafe {
        newFoldLevelWin(curwin);

        if foldmethodIsDiff(curwin) && (*curwin).w_onebuf_opt.wo_scb != 0 {
            let mut wp = firstwin;
            while !wp.is_null() {
                if !ptr::eq(wp, curwin) && foldmethodIsDiff(wp) && (*wp).w_onebuf_opt.wo_scb != 0 {
                    (*wp).w_onebuf_opt.wo_fdl = (*curwin).w_onebuf_opt.wo_fdl;
                    newFoldLevelWin(wp);
                }
                wp = (*wp).w_next;
            }
        }
    }
}

unsafe fn newFoldLevelWin(wp: *mut win_T) {
    unsafe {
        checkupdate(wp);
        if (*wp).w_fold_manual {
            let fp = (*wp).w_folds.ga_data.cast::<fold_T>();
            for i in 0..(*wp).w_folds.ga_len {
                (*fp.add(i as usize)).fd_flags = FD_LEVEL;
            }
            (*wp).w_fold_manual = false;
        }
        changed_window_setting(wp);
    }
}

/// Apply 'foldlevel' to all folds that don't contain the cursor.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldCheckClose() {
    unsafe {
        if *p_fcl == NUL {
            return;
        }

        checkupdate(curwin);
        if checkCloseRec(
            &mut (*curwin).w_folds,
            (*curwin).w_cursor.lnum,
            (*curwin).w_onebuf_opt.wo_fdl as c_int,
        ) {
            changed_window_setting(curwin);
        }
    }
}

unsafe fn checkCloseRec(gap: *mut garray_T, lnum: linenr_T, level: c_int) -> bool {
    unsafe {
        let mut retval = false;
        let fp = (*gap).ga_data.cast::<fold_T>();
        for i in 0..(*gap).ga_len {
            let entry = &mut *fp.add(i as usize);
            if entry.fd_flags == FD_OPEN {
                if level <= 0 && (lnum < entry.fd_top || lnum >= entry.fd_top + entry.fd_len) {
                    entry.fd_flags = FD_LEVEL;
                    retval = true;
                } else {
                    retval |= checkCloseRec(&mut entry.fd_nested, lnum - entry.fd_top, level - 1);
                }
            }
        }
        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldManualAllowed(create: bool) -> c_int {
    unsafe {
        if foldmethodIsManual(curwin) || foldmethodIsMarker(curwin) {
            return 1;
        }
        if create {
            emsg(gettext(
                c"E350: Cannot create fold with current 'foldmethod'".as_ptr(),
            ));
        } else {
            emsg(gettext(
                c"E351: Cannot delete fold with current 'foldmethod'".as_ptr(),
            ));
        }
        0
    }
}

/// Create a fold from line "start" to line "end" (inclusive) in the current
/// window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldCreate(wp: *mut win_T, mut start: pos_T, mut end: pos_T) {
    unsafe {
        let mut use_level = false;
        let mut closed = false;
        let mut level: c_int = 0;
        let mut start_rel = start;
        let mut end_rel = end;

        if start.lnum > end.lnum {
            end = start_rel;
            start = end_rel;
            start_rel = start;
            end_rel = end;
        }

        if foldmethodIsMarker(wp) {
            foldCreateMarkers(wp, start, end);
            return;
        }

        checkupdate(wp);

        let mut i: c_int;
        let mut gap = &mut (*wp).w_folds as *mut garray_T;
        if (*gap).ga_len == 0 {
            i = 0;
        } else {
            let mut fp: *mut fold_T = ptr::null_mut();
            loop {
                if !foldFind(gap, start_rel.lnum, &mut fp) {
                    break;
                }
                if (*fp).fd_top + (*fp).fd_len > end_rel.lnum {
                    gap = &mut (*fp).fd_nested;
                    start_rel.lnum -= (*fp).fd_top;
                    end_rel.lnum -= (*fp).fd_top;
                    if use_level || (*fp).fd_flags == FD_LEVEL {
                        use_level = true;
                        if i64::from(level) >= (*wp).w_onebuf_opt.wo_fdl {
                            closed = true;
                        }
                    } else if (*fp).fd_flags == FD_CLOSED {
                        closed = true;
                    }
                    level += 1;
                } else {
                    break;
                }
            }
            if (*gap).ga_len == 0 {
                i = 0;
            } else {
                i = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
            }
        }

        ga_grow(gap, 1);
        {
            let fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
            let mut fold_ga: garray_T = std::mem::zeroed();
            ga_init(&mut fold_ga, std::mem::size_of::<fold_T>() as c_int, 10);

            let mut cont: c_int = 0;
            while i + cont < (*gap).ga_len {
                if (*fp.add(cont as usize)).fd_top > end_rel.lnum {
                    break;
                }
                cont += 1;
            }
            if cont > 0 {
                ga_grow(&mut fold_ga, cont);
                start_rel.lnum = start_rel.lnum.min((*fp).fd_top);
                let last_cont = &*fp.add((cont - 1) as usize);
                end_rel.lnum = end_rel.lnum.max(last_cont.fd_top + last_cont.fd_len - 1);
                ptr::copy(fp, fold_ga.ga_data.cast::<fold_T>(), cont as usize);
                fold_ga.ga_len += cont;
                i += cont;

                for j in 0..cont {
                    (*fold_ga.ga_data.cast::<fold_T>().add(j as usize)).fd_top -= start_rel.lnum;
                }
            }
            if i < (*gap).ga_len {
                ptr::copy(
                    (*gap).ga_data.cast::<fold_T>().add(i as usize),
                    fp.add(1),
                    ((*gap).ga_len - i) as usize,
                );
            }
            (*gap).ga_len = (*gap).ga_len + 1 - cont;

            (*fp).fd_nested = fold_ga;
            (*fp).fd_top = start_rel.lnum;
            (*fp).fd_len = end_rel.lnum - start_rel.lnum + 1;

            if use_level && !closed && i64::from(level) < (*wp).w_onebuf_opt.wo_fdl {
                closeFold(start, 1);
            }
            if !use_level {
                (*wp).w_fold_manual = true;
            }
            (*fp).fd_flags = FD_CLOSED;
            (*fp).fd_small = kNone;

            changed_window_setting(wp);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn deleteFold(
    wp: *mut win_T,
    start: linenr_T,
    end: linenr_T,
    recursive: c_int,
    had_visual: bool,
) {
    unsafe {
        let mut found_fp: *mut fold_T = ptr::null_mut();
        let mut found_off: linenr_T = 0;
        let mut maybe_small = false;
        let mut level: c_int = 0;
        let mut lnum = start;
        let mut did_one = false;
        let mut first_lnum = MAXLNUM;
        let mut last_lnum: linenr_T = 0;

        checkupdate(wp);

        while lnum <= end {
            let mut gap = &mut (*wp).w_folds as *mut garray_T;
            let mut found_ga: *mut garray_T = ptr::null_mut();
            let mut lnum_off: linenr_T = 0;
            let mut use_level = false;
            loop {
                let mut fp: *mut fold_T = ptr::null_mut();
                if !foldFind(gap, lnum - lnum_off, &mut fp) {
                    break;
                }
                found_ga = gap;
                found_fp = fp;
                found_off = lnum_off;

                if check_closed(wp, fp, &mut use_level, level, &mut maybe_small, lnum_off) {
                    break;
                }

                gap = &mut (*fp).fd_nested;
                lnum_off += (*fp).fd_top;
                level += 1;
            }
            if found_ga.is_null() {
                lnum += 1;
            } else {
                lnum = (*found_fp).fd_top + (*found_fp).fd_len + found_off;

                if foldmethodIsManual(wp) {
                    let idx = found_fp.offset_from((*found_ga).ga_data.cast::<fold_T>()) as c_int;
                    deleteFoldEntry(wp, found_ga, idx, recursive != 0);
                } else {
                    first_lnum = first_lnum.min((*found_fp).fd_top + found_off);
                    last_lnum = last_lnum.max(lnum);
                    if !did_one {
                        parseMarker(wp);
                    }
                    deleteFoldMarkers(wp, found_fp, recursive != 0, found_off);
                }
                did_one = true;

                changed_window_setting(wp);
            }
        }
        if !did_one {
            emsg(gettext(E_NOFOLD.as_ptr()));
            if had_visual {
                redraw_buf_later((*wp).w_buffer, UPD_INVERTED);
            }
        } else {
            check_cursor_col(wp);
        }

        if last_lnum > 0 {
            changed_lines((*wp).w_buffer, first_lnum, 0, last_lnum, 0, false);

            let num_changed = i64::from(last_lnum - first_lnum);
            buf_updates_send_changes((*wp).w_buffer, first_lnum, num_changed, num_changed);
        }
    }
}

/// Remove all folding for window "win".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clearFolding(win: *mut win_T) {
    unsafe {
        deleteFoldRecurse((*win).w_buffer, &mut (*win).w_folds);
        (*win).w_foldinvalid = false;
    }
}

/// Update folds for changes in the buffer of a window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldUpdate(wp: *mut win_T, top: linenr_T, bot: linenr_T) {
    unsafe {
        if disable_fold_update != 0 || ((State & MODE_INSERT) != 0 && !foldmethodIsIndent(wp)) {
            return;
        }

        if need_diff_redraw {
            return;
        }

        if (*wp).w_folds.ga_len > 0 {
            let maybe_small_start = top.min(bot);
            let maybe_small_end = top.max(bot);

            let mut fp: *mut fold_T = ptr::null_mut();
            foldFind(&(*wp).w_folds, maybe_small_start, &mut fp);
            let end_fp = (*wp)
                .w_folds
                .ga_data
                .cast::<fold_T>()
                .add((*wp).w_folds.ga_len as usize);
            while !fp.is_null()
                && (fp as usize) < (end_fp as usize)
                && (*fp).fd_top <= maybe_small_end
            {
                (*fp).fd_small = kNone;
                fp = fp.add(1);
            }
        }

        if foldmethodIsIndent(wp)
            || foldmethodIsExpr(wp)
            || foldmethodIsMarker(wp)
            || foldmethodIsDiff(wp)
            || foldmethodIsSyntax(wp)
        {
            let save_got_int = got_int;
            got_int = false;
            foldUpdateIEMS(wp, top, bot);
            got_int |= save_got_int;
        }
    }
}

/// Updates folds when leaving insert-mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldUpdateAfterInsert() {
    unsafe {
        if foldmethodIsManual(curwin) || foldmethodIsSyntax(curwin) || foldmethodIsExpr(curwin) {
            return;
        }

        foldUpdateAll(curwin);
        foldOpenCursor();
    }
}

/// Update all lines in a window for folding.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldUpdateAll(win: *mut win_T) {
    unsafe {
        (*win).w_foldinvalid = true;
        redraw_later(win, UPD_NOT_VALID);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldMoveTo(updown: bool, dir: c_int, count: c_int) -> c_int {
    unsafe {
        let mut retval = FAIL;
        let mut fp: *mut fold_T = ptr::null_mut();

        checkupdate(curwin);

        for _ in 0..count {
            let mut lnum_off: linenr_T = 0;
            let mut gap = &mut (*curwin).w_folds as *mut garray_T;
            if (*gap).ga_len == 0 {
                break;
            }
            let mut use_level = false;
            let mut maybe_small = false;
            let mut lnum_found = (*curwin).w_cursor.lnum;
            let mut level: c_int = 0;
            let mut last = false;
            loop {
                if !foldFind(gap, (*curwin).w_cursor.lnum - lnum_off, &mut fp) {
                    if !updown || (*gap).ga_len == 0 {
                        break;
                    }

                    let ga_start = (*gap).ga_data.cast::<fold_T>();
                    if dir == FORWARD {
                        if fp.offset_from(ga_start) as c_int >= (*gap).ga_len {
                            break;
                        }
                        fp = fp.sub(1);
                    } else if ptr::eq(fp, ga_start) {
                        break;
                    }
                    last = true;
                }

                if !last {
                    if check_closed(
                        curwin,
                        fp,
                        &mut use_level,
                        level,
                        &mut maybe_small,
                        lnum_off,
                    ) {
                        last = true;
                    }
                    if last && !updown {
                        break;
                    }
                }

                let ga_start = (*gap).ga_data.cast::<fold_T>();
                if updown {
                    if dir == FORWARD {
                        if (fp.add(1).offset_from(ga_start) as c_int) < (*gap).ga_len {
                            let lnum = (*fp.add(1)).fd_top + lnum_off;
                            if lnum > (*curwin).w_cursor.lnum {
                                lnum_found = lnum;
                            }
                        }
                    } else if (fp as usize) > (ga_start as usize) {
                        let prev_fp = &*fp.sub(1);
                        let lnum = prev_fp.fd_top + lnum_off + prev_fp.fd_len - 1;
                        if lnum < (*curwin).w_cursor.lnum {
                            lnum_found = lnum;
                        }
                    }
                } else if dir == FORWARD {
                    let lnum = (*fp).fd_top + lnum_off + (*fp).fd_len - 1;
                    if lnum > (*curwin).w_cursor.lnum {
                        lnum_found = lnum;
                    }
                } else {
                    let lnum = (*fp).fd_top + lnum_off;
                    if lnum < (*curwin).w_cursor.lnum {
                        lnum_found = lnum;
                    }
                }

                if last {
                    break;
                }

                gap = &mut (*fp).fd_nested;
                lnum_off += (*fp).fd_top;
                level += 1;
            }
            if lnum_found != (*curwin).w_cursor.lnum {
                if retval == FAIL {
                    setpcmark();
                }
                (*curwin).w_cursor.lnum = lnum_found;
                (*curwin).w_cursor.col = 0;
                retval = OK;
            } else {
                break;
            }
        }

        retval
    }
}

/// Init the fold info in a new window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldInitWin(new_win: *mut win_T) {
    unsafe {
        ga_init(
            &mut (*new_win).w_folds,
            std::mem::size_of::<fold_T>() as c_int,
            10,
        );
    }
}

/// Find an entry in the win->w_lines[] array for buffer line "lnum".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_wl_entry(win: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        let w_lines = (*win).w_lines as *const wline_T;
        for i in 0..(*win).w_lines_valid {
            let wl = &*w_lines.add(i as usize);
            if wl.wl_valid {
                if lnum < wl.wl_lnum {
                    return -1;
                }
                if lnum <= wl.wl_foldend {
                    return i;
                }
            }
        }
        -1
    }
}

/// Adjust the Visual area to include any fold at the start or end completely.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldAdjustVisual() {
    unsafe {
        if !Visual.active || hasAnyFolding(curwin) == 0 {
            return;
        }

        let (start, end) = if ltoreq(Visual.start, (*curwin).w_cursor) {
            (&raw mut Visual.start, &raw mut (*curwin).w_cursor)
        } else {
            (&raw mut (*curwin).w_cursor, &raw mut Visual.start)
        };
        if hasFolding(
            curwin,
            (*start).lnum,
            &raw mut (*start).lnum,
            ptr::null_mut(),
        ) {
            (*start).col = 0;
        }

        if !hasFolding(curwin, (*end).lnum, ptr::null_mut(), &raw mut (*end).lnum) {
            return;
        }

        (*end).col = ml_get_len((*end).lnum);
        if (*end).col > 0 && *p_sel == b'o' as c_char {
            (*end).col -= 1;
        }
        mb_adjust_cursor();
    }
}

/// Move the cursor to the first line of a closed fold.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldAdjustCursor(wp: *mut win_T) {
    unsafe {
        hasFolding(
            wp,
            (*wp).w_cursor.lnum,
            &raw mut (*wp).w_cursor.lnum,
            ptr::null_mut(),
        );
    }
}

/// Will "clone" (i.e deep copy) a garray_T of folds.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cloneFoldGrowArray(from: *mut garray_T, to: *mut garray_T) {
    unsafe {
        ga_init(to, (*from).ga_itemsize, (*from).ga_growsize);

        if (*from).ga_len == 0 {
            return;
        }

        ga_grow(to, (*from).ga_len);

        let mut from_p = (*from).ga_data.cast::<fold_T>();
        let mut to_p = (*to).ga_data.cast::<fold_T>();

        for _ in 0..(*from).ga_len {
            (*to_p).fd_top = (*from_p).fd_top;
            (*to_p).fd_len = (*from_p).fd_len;
            (*to_p).fd_flags = (*from_p).fd_flags;
            (*to_p).fd_small = (*from_p).fd_small;
            cloneFoldGrowArray(&mut (*from_p).fd_nested, &mut (*to_p).fd_nested);
            (*to).ga_len += 1;
            from_p = from_p.add(1);
            to_p = to_p.add(1);
        }
    }
}

unsafe fn foldFind(gap: *const garray_T, lnum: linenr_T, fpp: *mut *mut fold_T) -> bool {
    unsafe {
        if (*gap).ga_len == 0 {
            *fpp = ptr::null_mut();
            return false;
        }

        let fp = (*gap).ga_data.cast::<fold_T>();
        let mut low: linenr_T = 0;
        let mut high: linenr_T = (*gap).ga_len - 1;
        while low <= high {
            let i = (low + high) / 2;
            let entry = &*fp.add(i as usize);
            if entry.fd_top > lnum {
                high = i - 1;
            } else if entry.fd_top + entry.fd_len <= lnum {
                low = i + 1;
            } else {
                *fpp = fp.add(i as usize);
                return true;
            }
        }
        *fpp = fp.add(low as usize);
        false
    }
}

unsafe fn foldLevelWin(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        let mut fp: *mut fold_T = ptr::null_mut();
        let mut lnum_rel = lnum;
        let mut level: c_int = 0;

        let mut gap = &(*wp).w_folds as *const garray_T;
        loop {
            if !foldFind(gap, lnum_rel, &mut fp) {
                break;
            }
            gap = &(*fp).fd_nested;
            lnum_rel -= (*fp).fd_top;
            level += 1;
        }

        level
    }
}

unsafe fn checkupdate(wp: *mut win_T) {
    unsafe {
        if !(*wp).w_foldinvalid {
            return;
        }

        foldUpdate(wp, 1, MAXLNUM);
        (*wp).w_foldinvalid = false;
    }
}

unsafe fn setFoldRepeat(pos: pos_T, count: c_int, do_open: bool) {
    unsafe {
        for n in 0..count {
            let mut done = DONE_NOTHING;
            setManualFold(pos, do_open, false, &mut done);
            if (done & DONE_ACTION) == 0 {
                if n == 0 && (done & DONE_FOLD) == 0 {
                    emsg(gettext(E_NOFOLD.as_ptr()));
                }
                break;
            }
        }
    }
}

unsafe fn setManualFold(pos: pos_T, opening: bool, recurse: bool, donep: *mut c_int) -> linenr_T {
    unsafe {
        if foldmethodIsDiff(curwin) && (*curwin).w_onebuf_opt.wo_scb != 0 {
            let mut wp = firstwin;
            while !wp.is_null() {
                if !ptr::eq(wp, curwin) && foldmethodIsDiff(wp) && (*wp).w_onebuf_opt.wo_scb != 0 {
                    let dlnum = diff_lnum_win((*curwin).w_cursor.lnum, wp);
                    if dlnum != 0 {
                        setManualFoldWin(wp, dlnum, opening, recurse, ptr::null_mut());
                    }
                }
                wp = (*wp).w_next;
            }
        }

        setManualFoldWin(curwin, pos.lnum, opening, recurse, donep)
    }
}

unsafe fn setManualFoldWin(
    wp: *mut win_T,
    mut lnum: linenr_T,
    opening: bool,
    recurse: bool,
    donep: *mut c_int,
) -> linenr_T {
    unsafe {
        let mut fp: *mut fold_T = ptr::null_mut();
        let mut found: *mut fold_T = ptr::null_mut();
        let mut level: c_int = 0;
        let mut use_level = false;
        let mut found_fold = false;
        let mut next = MAXLNUM;
        let mut off: linenr_T = 0;
        let mut done: c_int = 0;

        checkupdate(wp);

        let mut gap = &mut (*wp).w_folds as *mut garray_T;
        loop {
            let ga_end = (*gap).ga_data.cast::<fold_T>().add((*gap).ga_len as usize);
            if !foldFind(gap, lnum, &mut fp) {
                if !fp.is_null() && (fp as usize) < (ga_end as usize) {
                    next = (*fp).fd_top + off;
                }
                break;
            }

            found_fold = true;

            if (fp.add(1) as usize) < (ga_end as usize) {
                next = (*fp.add(1)).fd_top + off;
            }

            if use_level || (*fp).fd_flags == FD_LEVEL {
                use_level = true;
                (*fp).fd_flags = if i64::from(level) >= (*wp).w_onebuf_opt.wo_fdl {
                    FD_CLOSED
                } else {
                    FD_OPEN
                };
                let fp2 = (*fp).fd_nested.ga_data.cast::<fold_T>();
                for j in 0..(*fp).fd_nested.ga_len {
                    (*fp2.add(j as usize)).fd_flags = FD_LEVEL;
                }
            }

            if !opening && recurse {
                if (*fp).fd_flags != FD_CLOSED {
                    done |= DONE_ACTION;
                    (*fp).fd_flags = FD_CLOSED;
                }
            } else if (*fp).fd_flags == FD_CLOSED {
                if opening {
                    (*fp).fd_flags = FD_OPEN;
                    done |= DONE_ACTION;
                    if recurse {
                        foldOpenNested(fp);
                    }
                }
                break;
            }

            found = fp;
            gap = &mut (*fp).fd_nested;
            lnum -= (*fp).fd_top;
            off += (*fp).fd_top;
            level += 1;
        }
        if found_fold {
            if !opening && !found.is_null() {
                (*found).fd_flags = FD_CLOSED;
                done |= DONE_ACTION;
            }
            (*wp).w_fold_manual = true;
            if (done & DONE_ACTION) != 0 {
                changed_window_setting(wp);
            }
            done |= DONE_FOLD;
        } else if donep.is_null() && ptr::eq(wp, curwin) {
            emsg(gettext(E_NOFOLD.as_ptr()));
        }

        if !donep.is_null() {
            *donep |= done;
        }

        next
    }
}

unsafe fn foldOpenNested(fpr: *mut fold_T) {
    unsafe {
        let fp = (*fpr).fd_nested.ga_data.cast::<fold_T>();
        for i in 0..(*fpr).fd_nested.ga_len {
            let child = fp.add(i as usize);
            foldOpenNested(child);
            (*child).fd_flags = FD_OPEN;
        }
    }
}

unsafe fn deleteFoldEntry(wp: *mut win_T, gap: *mut garray_T, idx: c_int, recursive: bool) {
    unsafe {
        let mut fp = (*gap).ga_data.cast::<fold_T>().add(idx as usize);
        if recursive || (*fp).fd_nested.ga_len == 0 {
            deleteFoldRecurse((*wp).w_buffer, &mut (*fp).fd_nested);
            (*gap).ga_len -= 1;
            if idx < (*gap).ga_len {
                ptr::copy(fp.add(1), fp, ((*gap).ga_len - idx) as usize);
            }
        } else {
            let moved = (*fp).fd_nested.ga_len;
            ga_grow(gap, moved - 1);
            fp = (*gap).ga_data.cast::<fold_T>().add(idx as usize);

            let nfp = (*fp).fd_nested.ga_data.cast::<fold_T>();
            for i in 0..moved {
                let child = &mut *nfp.add(i as usize);
                child.fd_top += (*fp).fd_top;
                if (*fp).fd_flags == FD_LEVEL {
                    child.fd_flags = FD_LEVEL;
                }
                if (*fp).fd_small == kNone {
                    child.fd_small = kNone;
                }
            }

            if idx + 1 < (*gap).ga_len {
                ptr::copy(
                    fp.add(1),
                    fp.add(moved as usize),
                    ((*gap).ga_len - (idx + 1)) as usize,
                );
            }
            ptr::copy(nfp, fp, moved as usize);
            xfree(nfp.cast::<c_void>());
            (*gap).ga_len += moved - 1;
        }
    }
}

/// Delete nested folds in a fold.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn deleteFoldRecurse(_bp: *mut buf_T, gap: *mut garray_T) {
    unsafe {
        let fp = (*gap).ga_data.cast::<fold_T>();
        for i in 0..(*gap).ga_len {
            deleteFoldRecurse(_bp, &mut (*fp.add(i as usize)).fd_nested);
        }
        ga_clear(gap);
    }
}

/// Update line numbers of folds for inserted/deleted lines.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldMarkAdjust(
    wp: *mut win_T,
    mut line1: linenr_T,
    mut line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
        if amount == MAXLNUM && line2 >= line1 && line2 - line1 >= -amount_after {
            line2 = line1 - amount_after - 1;
        }
        if line2 < line1 {
            line2 = line1;
        }
        if (State & MODE_INSERT) != 0 && amount == 1 && line2 == MAXLNUM {
            line1 -= 1;
        }
        foldMarkAdjustRecurse(wp, &mut (*wp).w_folds, line1, line2, amount, amount_after);
    }
}

unsafe fn foldMarkAdjustRecurse(
    wp: *mut win_T,
    gap: *mut garray_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
        if (*gap).ga_len == 0 {
            return;
        }

        let top = if (State & MODE_INSERT) != 0 && amount == 1 && line2 == MAXLNUM {
            line1 + 1
        } else {
            line1
        };

        let mut fp: *mut fold_T = ptr::null_mut();
        foldFind(gap, line1, &mut fp);

        let mut i = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
        while i < (*gap).ga_len {
            fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
            let last = (*fp).fd_top + (*fp).fd_len - 1;

            if last < line1 {
                i += 1;
                continue;
            }

            if (*fp).fd_top > line2 {
                if amount_after == 0 {
                    break;
                }
                (*fp).fd_top += amount_after;
            } else if (*fp).fd_top >= top && last <= line2 {
                if amount == MAXLNUM {
                    deleteFoldEntry(wp, gap, i, true);
                    continue;
                } else {
                    (*fp).fd_top += amount;
                }
            } else if (*fp).fd_top < top {
                foldMarkAdjustRecurse(
                    wp,
                    &mut (*fp).fd_nested,
                    line1 - (*fp).fd_top,
                    line2 - (*fp).fd_top,
                    amount,
                    amount_after,
                );
                if last <= line2 {
                    if amount == MAXLNUM {
                        (*fp).fd_len = line1 - (*fp).fd_top;
                    } else {
                        (*fp).fd_len += amount;
                    }
                } else {
                    (*fp).fd_len += amount_after;
                }
            } else if amount == MAXLNUM {
                foldMarkAdjustRecurse(
                    wp,
                    &mut (*fp).fd_nested,
                    0,
                    line2 - (*fp).fd_top,
                    amount,
                    amount_after + ((*fp).fd_top - top),
                );
                (*fp).fd_len -= line2 - (*fp).fd_top + 1;
                (*fp).fd_top = line1;
            } else {
                foldMarkAdjustRecurse(
                    wp,
                    &mut (*fp).fd_nested,
                    0,
                    line2 - (*fp).fd_top,
                    amount,
                    amount_after - amount,
                );
                (*fp).fd_len += amount_after - amount;
                (*fp).fd_top += amount;
            }

            i += 1;
        }
    }
}

/// Get the lowest 'foldlevel' value that makes the deepest nested fold in
/// window `wp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getDeepestNesting(wp: *mut win_T) -> c_int {
    unsafe {
        checkupdate(wp);
        getDeepestNestingRecurse(&(*wp).w_folds)
    }
}

unsafe fn getDeepestNestingRecurse(gap: *const garray_T) -> c_int {
    unsafe {
        let mut maxlevel: c_int = 0;
        let fp = (*gap).ga_data.cast::<fold_T>();
        for i in 0..(*gap).ga_len {
            let level = getDeepestNestingRecurse(&(*fp.add(i as usize)).fd_nested) + 1;
            maxlevel = maxlevel.max(level);
        }
        maxlevel
    }
}

unsafe fn check_closed(
    wp: *mut win_T,
    fp: *mut fold_T,
    use_levelp: &mut bool,
    level: c_int,
    maybe_smallp: &mut bool,
    lnum_off: linenr_T,
) -> bool {
    unsafe {
        let mut closed = false;

        if *use_levelp || (*fp).fd_flags == FD_LEVEL {
            *use_levelp = true;
            if i64::from(level) >= (*wp).w_onebuf_opt.wo_fdl {
                closed = true;
            }
        } else if (*fp).fd_flags == FD_CLOSED {
            closed = true;
        }

        if (*fp).fd_small == kNone {
            *maybe_smallp = true;
        }
        if closed {
            if *maybe_smallp {
                (*fp).fd_small = kNone;
            }
            checkSmall(wp, fp, lnum_off);
            if (*fp).fd_small == kTrue {
                closed = false;
            }
        }
        closed
    }
}

unsafe fn checkSmall(wp: *mut win_T, fp: *mut fold_T, lnum_off: linenr_T) {
    unsafe {
        if (*fp).fd_small != kNone {
            return;
        }

        setSmallMaybe(&mut (*fp).fd_nested);

        if i64::from((*fp).fd_len) > (*wp).w_onebuf_opt.wo_fml {
            (*fp).fd_small = kFalse;
        } else {
            let mut count: c_int = 0;
            for n in 0..(*fp).fd_len {
                count += plines_win_nofold(wp, (*fp).fd_top + lnum_off + n);
                if i64::from(count) > (*wp).w_onebuf_opt.wo_fml {
                    (*fp).fd_small = kFalse;
                    return;
                }
            }
            (*fp).fd_small = kTrue;
        }
    }
}

unsafe fn setSmallMaybe(gap: *mut garray_T) {
    unsafe {
        let fp = (*gap).ga_data.cast::<fold_T>();
        for i in 0..(*gap).ga_len {
            (*fp.add(i as usize)).fd_small = kNone;
        }
    }
}

unsafe fn foldCreateMarkers(wp: *mut win_T, start: pos_T, end: pos_T) {
    unsafe {
        let buf = (*wp).w_buffer;
        if (*buf).b_p_ma == 0 {
            emsg(gettext(e_modifiable.as_ptr()));
            return;
        }
        parseMarker(wp);

        foldAddMarker(buf, start, (*wp).w_onebuf_opt.wo_fmr, foldstartmarkerlen);
        foldAddMarker(buf, end, foldendmarker, foldendmarkerlen);

        changed_lines(buf, start.lnum, 0, end.lnum, 0, false);

        let num_changed = 1 + i64::from(end.lnum - start.lnum);
        buf_updates_send_changes(buf, start.lnum, num_changed, num_changed);
    }
}

unsafe fn foldAddMarker(buf: *mut buf_T, pos: pos_T, marker: *const c_char, markerlen: usize) {
    unsafe {
        let cms = (*buf).b_p_cms;
        let p = strstr(cms, c"%s".as_ptr());
        let mut line_is_comment = false;
        let lnum = pos.lnum;

        let line = ml_get_buf(buf, lnum);
        let line_len = ml_get_buf_len(buf, lnum) as usize;

        if u_save(lnum - 1, lnum + 1) != OK {
            return;
        }

        skip_comment(line, false, false, &mut line_is_comment);
        let cms_len = CStr::from_ptr(cms).to_bytes().len();
        let newline = xmalloc(line_len + markerlen + cms_len + 1).cast::<c_char>();
        ptr::copy_nonoverlapping(line, newline, line_len + 1);
        let added = if p.is_null() || line_is_comment {
            xmemcpyz(
                newline.add(line_len).cast::<c_void>(),
                marker.cast::<c_void>(),
                markerlen,
            );
            markerlen
        } else {
            ptr::copy_nonoverlapping(cms, newline.add(line_len), cms_len + 1);
            let p_off = (p as usize) - (cms as usize);
            ptr::copy_nonoverlapping(marker, newline.add(line_len + p_off), markerlen);
            let tail_len = CStr::from_ptr(p.add(2)).to_bytes().len();
            ptr::copy_nonoverlapping(
                p.add(2),
                newline.add(line_len + p_off + markerlen),
                tail_len + 1,
            );
            markerlen + cms_len - 2
        };
        ml_replace_buf(buf, lnum, newline, false, false);
        if added != 0 {
            extmark_splice_cols(
                buf,
                lnum - 1,
                line_len as colnr_T,
                0,
                added as colnr_T,
                kExtmarkUndo,
            );
        }
    }
}

unsafe fn deleteFoldMarkers(wp: *mut win_T, fp: *mut fold_T, recursive: bool, lnum_off: linenr_T) {
    unsafe {
        if recursive {
            let nfp = (*fp).fd_nested.ga_data.cast::<fold_T>();
            for i in 0..(*fp).fd_nested.ga_len {
                deleteFoldMarkers(wp, nfp.add(i as usize), true, lnum_off + (*fp).fd_top);
            }
        }
        foldDelMarker(
            (*wp).w_buffer,
            (*fp).fd_top + lnum_off,
            (*wp).w_onebuf_opt.wo_fmr,
            foldstartmarkerlen,
        );
        foldDelMarker(
            (*wp).w_buffer,
            (*fp).fd_top + lnum_off + (*fp).fd_len - 1,
            foldendmarker,
            foldendmarkerlen,
        );
    }
}

unsafe fn foldDelMarker(buf: *mut buf_T, lnum: linenr_T, marker: *const c_char, markerlen: usize) {
    unsafe {
        if lnum > (*buf).b_ml.ml_line_count {
            return;
        }

        let cms = (*buf).b_p_cms;
        let line = ml_get_buf(buf, lnum);
        let marker_slice = std::slice::from_raw_parts(marker.cast::<u8>(), markerlen);
        let mut p = line;
        while *p != NUL {
            let rem_len = CStr::from_ptr(p).to_bytes().len();
            if rem_len < markerlen
                || std::slice::from_raw_parts(p.cast::<u8>(), markerlen) != marker_slice
            {
                p = p.add(1);
                continue;
            }
            let mut len = markerlen;
            if ascii_isdigit(*p.add(len) as u8 as c_int) {
                len += 1;
            }
            if *cms != NUL {
                let cms2 = strstr(cms, c"%s".as_ptr());
                if !cms2.is_null() {
                    let prefix_len = (cms2 as usize) - (cms as usize);
                    let suffix = cms2.add(2);
                    let suffix_len = CStr::from_ptr(suffix).to_bytes().len();
                    if (p as usize) - (line as usize) >= prefix_len
                        && std::slice::from_raw_parts(p.sub(prefix_len).cast::<u8>(), prefix_len)
                            == std::slice::from_raw_parts(cms.cast::<u8>(), prefix_len)
                        && CStr::from_ptr(p.add(len)).to_bytes().starts_with(
                            std::slice::from_raw_parts(suffix.cast::<u8>(), suffix_len),
                        )
                    {
                        p = p.sub(prefix_len);
                        len += CStr::from_ptr(cms).to_bytes().len() - 2;
                    }
                }
            }
            if u_save(lnum - 1, lnum + 1) == OK {
                let orig_len = ml_get_buf_len(buf, lnum) as usize;
                let newline = xmalloc(orig_len - len + 1).cast::<c_char>();
                let before_len = (p as usize) - (line as usize);
                ptr::copy_nonoverlapping(line, newline, before_len);
                let after = p.add(len);
                let after_len = CStr::from_ptr(after).to_bytes().len();
                ptr::copy_nonoverlapping(after, newline.add(before_len), after_len + 1);
                ml_replace_buf(buf, lnum, newline, false, false);
                extmark_splice_cols(
                    buf,
                    lnum - 1,
                    before_len as colnr_T,
                    len as colnr_T,
                    0,
                    kExtmarkUndo,
                );
            }
            break;
        }
    }
}

/// Generates text to display.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_foldtext(
    wp: *mut win_T,
    lnum: linenr_T,
    lnume: linenr_T,
    foldinfo: foldinfo_T,
    buf: *mut c_char,
    vt: *mut VirtText,
) -> *mut c_char {
    unsafe {
        let mut text: *mut c_char = ptr::null_mut();
        let save_did_emsg = did_emsg;

        if last_wp.is_null() || !ptr::eq(last_wp, wp) || fdt_last_lnum > lnum || fdt_last_lnum == 0
        {
            got_fdt_error = false;
        }

        if !got_fdt_error {
            did_emsg = 0;
        }

        if (*wp).w_onebuf_opt.wo_fdt.cb_type != kCallbackNone {
            let mut dashes = [0 as c_char; (MAX_LEVEL as usize) + 2];

            set_vim_var_nr(VV_FOLDSTART, varnumber_T::from(lnum));
            set_vim_var_nr(VV_FOLDEND, varnumber_T::from(lnume));

            let level = foldinfo.fi_level.min((dashes.len() - 1) as c_int).max(0);
            ptr::write_bytes(dashes.as_mut_ptr().cast::<u8>(), b'-', level as usize);
            dashes[level as usize] = NUL;
            set_vim_var_string(VV_FOLDDASHES, dashes.as_ptr(), level as isize);
            set_vim_var_nr(VV_FOLDLEVEL, varnumber_T::from(level));

            if !got_fdt_error {
                let save_curwin = curwin;
                let saved_sctx = current_sctx;

                curwin = wp;
                curbuf = (*wp).w_buffer;
                current_sctx = (*wp).w_onebuf_opt.wo_script_ctx[kWinOptFoldtext];

                emsg_off += 1;

                let mut obj = eval_foldtext(wp);
                if obj.obj_type == ObjectType::kObjectTypeArray {
                    let mut err = Error {
                        err_type: ErrorType::kErrorTypeNone,
                        msg: ptr::null_mut(),
                    };
                    *vt = parse_virt_text(obj.data.array, &mut err, ptr::null_mut(), false);
                    if err.err_type == ErrorType::kErrorTypeNone {
                        *buf = NUL;
                        text = buf;
                    }
                    api_clear_error(&mut err);
                } else if obj.obj_type == ObjectType::kObjectTypeString {
                    text = obj.data.string.data;
                    obj = Object {
                        obj_type: ObjectType::kObjectTypeNil,
                        data: ObjectData { integer: 0 },
                    };
                }
                api_free_object(obj);

                emsg_off -= 1;

                if text.is_null() || did_emsg != 0 {
                    got_fdt_error = true;
                }

                curwin = save_curwin;
                curbuf = (*curwin).w_buffer;
                current_sctx = saved_sctx;
            }
            fdt_last_lnum = lnum;
            last_wp = wp;
            set_vim_var_string(VV_FOLDDASHES, ptr::null(), -1);

            if did_emsg == 0 && save_did_emsg != 0 {
                did_emsg = save_did_emsg;
            }

            if !text.is_null() {
                let mut p = text;
                while *p != NUL {
                    let len = utfc_ptr2len(p);
                    if len > 1 {
                        if !vim_isprintc(utf_ptr2char(p)) {
                            break;
                        }
                        p = p.add((len - 1) as usize);
                    } else if *p == TAB {
                        *p = b' ' as c_char;
                    } else if ptr2cells(p) > 1 {
                        break;
                    }
                    p = p.add(1);
                }
                if *p != NUL {
                    let new_p = transstr(text, true);
                    xfree(text.cast::<c_void>());
                    text = new_p;
                }
            }
        }
        if text.is_null() {
            let count = lnume - lnum + 1;
            vim_snprintf!(
                buf,
                FOLD_TEXT_LEN,
                ngettext_wrap(
                    c"+--%3d line folded".as_ptr(),
                    c"+--%3d lines folded ".as_ptr(),
                    count as c_ulong,
                ),
                count,
            );
            text = buf;
        }
        text
    }
}

unsafe fn foldtext_cleanup(str_ptr: *mut c_char) {
    unsafe {
        let cms_start = skipwhite((*curbuf).b_p_cms);
        let mut cms_slen = CStr::from_ptr(cms_start).to_bytes().len();
        while cms_slen > 0 && ascii_iswhite(*cms_start.add(cms_slen - 1) as u8 as c_int) {
            cms_slen -= 1;
        }

        let mut cms_end = strstr(cms_start, c"%s".as_ptr());
        let mut cms_elen: usize = 0;
        if !cms_end.is_null() {
            let start_off = (cms_end as usize) - (cms_start as usize);
            cms_elen = cms_slen - start_off;
            cms_slen = start_off;

            while cms_slen > 0 && ascii_iswhite(*cms_start.add(cms_slen - 1) as u8 as c_int) {
                cms_slen -= 1;
            }

            let s = skipwhite(cms_end.add(2));
            cms_elen -= (s as usize) - (cms_end as usize);
            cms_end = s;
        }
        parseMarker(curwin);

        let mut did1 = false;
        let mut did2 = false;

        let fmr_slice = std::slice::from_raw_parts(
            (*curwin).w_onebuf_opt.wo_fmr.cast::<u8>(),
            foldstartmarkerlen,
        );
        let end_slice = std::slice::from_raw_parts(foldendmarker.cast::<u8>(), foldendmarkerlen);

        let mut s = str_ptr;
        while *s != NUL {
            let s_bytes = CStr::from_ptr(s).to_bytes();
            let mut len: usize = 0;
            if s_bytes.starts_with(fmr_slice) {
                len = foldstartmarkerlen;
            } else if s_bytes.starts_with(end_slice) {
                len = foldendmarkerlen;
            }
            if len > 0 {
                if ascii_isdigit(*s.add(len) as u8 as c_int) {
                    len += 1;
                }

                let mut p = s;
                while (p as usize) > (str_ptr as usize) && ascii_iswhite(*p.sub(1) as u8 as c_int) {
                    p = p.sub(1);
                }
                if (p as usize) >= (str_ptr as usize) + cms_slen
                    && std::slice::from_raw_parts(p.sub(cms_slen).cast::<u8>(), cms_slen)
                        == std::slice::from_raw_parts(cms_start.cast::<u8>(), cms_slen)
                {
                    len += ((s as usize) - (p as usize)) + cms_slen;
                    s = p.sub(cms_slen);
                }
            } else if !cms_end.is_null() {
                if !did1
                    && cms_slen > 0
                    && s_bytes
                        .starts_with(std::slice::from_raw_parts(cms_start.cast::<u8>(), cms_slen))
                {
                    len = cms_slen;
                    did1 = true;
                } else if !did2
                    && cms_elen > 0
                    && s_bytes
                        .starts_with(std::slice::from_raw_parts(cms_end.cast::<u8>(), cms_elen))
                {
                    len = cms_elen;
                    did2 = true;
                }
            }
            if len != 0 {
                while ascii_iswhite(*s.add(len) as u8 as c_int) {
                    len += 1;
                }
                let rem = CStr::from_ptr(s.add(len)).to_bytes().len() + 1;
                ptr::copy(s.add(len), s, rem);
            } else {
                s = s.add(utfc_ptr2len(s) as usize);
            }
        }
    }
}

unsafe fn call_getlevel(kind: LevelGetterKind, flp: &mut fline_T) {
    unsafe {
        match kind {
            LevelGetterKind::Marker => foldlevelMarker(flp),
            LevelGetterKind::Expr => foldlevelExpr(flp),
            LevelGetterKind::Syntax => foldlevelSyntax(flp),
            LevelGetterKind::Diff => foldlevelDiff(flp),
            LevelGetterKind::Indent => foldlevelIndent(flp),
        }
    }
}

unsafe fn foldUpdateIEMS(wp: *mut win_T, mut top: linenr_T, mut bot: linenr_T) {
    unsafe {
        if invalid_top != 0 {
            return;
        }

        if (*wp).w_foldinvalid {
            top = 1;
            bot = (*(*wp).w_buffer).b_ml.ml_line_count;
            (*wp).w_foldinvalid = false;

            setSmallMaybe(&mut (*wp).w_folds);
        }

        if foldmethodIsDiff(wp) {
            if top > diff_context {
                top -= diff_context;
            } else {
                top = 1;
            }
            bot += diff_context;
        }

        top = top.min((*(*wp).w_buffer).b_ml.ml_line_count);

        fold_changed = false;
        let mut fline = fline_T {
            wp,
            lnum: 0,
            off: 0,
            lnum_save: 0,
            lvl: 0,
            lvl_next: -1,
            start: 0,
            end: MAX_LEVEL + 1,
            had_end: MAX_LEVEL + 1,
        };

        invalid_top = top;
        invalid_bot = bot;

        let getlevel: LevelGetterKind;

        if foldmethodIsMarker(wp) {
            getlevel = LevelGetterKind::Marker;

            parseMarker(wp);

            if top > 1 {
                let level = foldLevelWin(wp, top - 1);

                fline.lnum = top - 1;
                fline.lvl = level;
                call_getlevel(getlevel, &mut fline);

                if fline.lvl > level {
                    fline.lvl = level - (fline.lvl - fline.lvl_next);
                } else {
                    fline.lvl = fline.lvl_next;
                }
            }
            fline.lnum = top;
            call_getlevel(getlevel, &mut fline);
        } else {
            fline.lnum = top;
            if foldmethodIsExpr(wp) {
                getlevel = LevelGetterKind::Expr;
                if top > 1 {
                    fline.lnum -= 1;
                }
            } else if foldmethodIsSyntax(wp) {
                getlevel = LevelGetterKind::Syntax;
            } else if foldmethodIsDiff(wp) {
                getlevel = LevelGetterKind::Diff;
            } else {
                getlevel = LevelGetterKind::Indent;
                if top > 1 {
                    fline.lnum -= 1;
                }
            }

            fline.lvl = -1;
            while !got_int {
                fline.lvl_next = -1;
                call_getlevel(getlevel, &mut fline);
                if fline.lvl >= 0 {
                    break;
                }
                fline.lnum -= 1;
            }
        }

        if getlevel == LevelGetterKind::Syntax {
            let mut gap = &(*wp).w_folds as *const garray_T;
            let mut fpn: *mut fold_T = ptr::null_mut();
            let mut current_fdl: c_int = 0;
            let mut fold_start_lnum: linenr_T = 0;
            let mut lnum_rel = fline.lnum;

            while current_fdl < fline.lvl {
                if !foldFind(gap, lnum_rel, &mut fpn) {
                    break;
                }
                current_fdl += 1;

                fold_start_lnum += (*fpn).fd_top;
                gap = &(*fpn).fd_nested;
                lnum_rel -= (*fpn).fd_top;
            }
            if !fpn.is_null() && current_fdl == fline.lvl {
                let fold_end_lnum = fold_start_lnum + (*fpn).fd_len;
                bot = bot.max(fold_end_lnum);
            }
        }

        let mut start = fline.lnum;
        let mut end = bot;
        if start > end && end < (*(*wp).w_buffer).b_ml.ml_line_count {
            end = start;
        }

        let mut fp: *mut fold_T = ptr::null_mut();

        while !got_int {
            if fline.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
                break;
            }
            if fline.lnum > end {
                if getlevel != LevelGetterKind::Marker
                    && getlevel != LevelGetterKind::Syntax
                    && getlevel != LevelGetterKind::Expr
                {
                    break;
                }
                if (start <= end
                    && foldFind(&(*wp).w_folds, end, &mut fp)
                    && (*fp).fd_top + (*fp).fd_len - 1 > end)
                    || (fline.lvl == 0
                        && foldFind(&(*wp).w_folds, fline.lnum, &mut fp)
                        && (*fp).fd_top < fline.lnum)
                {
                    end = (*fp).fd_top + (*fp).fd_len - 1;
                } else if getlevel == LevelGetterKind::Syntax
                    && foldLevelWin(wp, fline.lnum) != fline.lvl
                {
                    end = fline.lnum;
                } else {
                    break;
                }
            }

            if fline.lvl > 0 {
                invalid_top = fline.lnum;
                invalid_bot = end;
                end = foldUpdateIEMSRecurse(
                    &mut (*wp).w_folds,
                    1,
                    start,
                    &mut fline,
                    getlevel,
                    end,
                    FD_LEVEL,
                );
                start = fline.lnum;
            } else {
                if fline.lnum == (*(*wp).w_buffer).b_ml.ml_line_count {
                    break;
                }
                fline.lnum += 1;
                fline.lvl = fline.lvl_next;
                call_getlevel(getlevel, &mut fline);
            }
        }

        foldRemove(wp, &mut (*wp).w_folds, start, end);

        if fold_changed && (*wp).w_onebuf_opt.wo_fen != 0 {
            changed_window_setting(wp);
        }

        if end != bot {
            redraw_win_range_later(wp, top, end);
        }

        invalid_top = 0;
    }
}

unsafe fn foldUpdateIEMSRecurse(
    gap: *mut garray_T,
    level: c_int,
    startlnum: linenr_T,
    flp: &mut fline_T,
    getlevel: LevelGetterKind,
    mut bot: linenr_T,
    topflags: c_char,
) -> linenr_T {
    unsafe {
        let mut fp: *mut fold_T = ptr::null_mut();

        if getlevel == LevelGetterKind::Marker && flp.start <= flp.lvl - level && flp.lvl > 0 {
            foldFind(gap, startlnum - 1, &mut fp);
            let ga_end = (*gap).ga_data.cast::<fold_T>().add((*gap).ga_len as usize);
            if !fp.is_null() && ((fp as usize) >= (ga_end as usize) || (*fp).fd_top >= startlnum) {
                fp = ptr::null_mut();
            }
        }

        let mut fp2: *mut fold_T;
        let mut lvl = level;
        let mut startlnum2 = startlnum;
        let firstlnum = flp.lnum;
        let mut finish = false;
        let linecount = (*(*flp.wp).w_buffer).b_ml.ml_line_count - flp.off;

        flp.lnum_save = flp.lnum;
        while !got_int {
            line_breakcheck();

            lvl = flp.lvl.min(MAX_LEVEL);
            if flp.lnum > firstlnum && (level > lvl - flp.start || level >= flp.had_end) {
                lvl = 0;
            }

            if flp.lnum > bot && !finish && !fp.is_null() {
                if getlevel != LevelGetterKind::Marker
                    && getlevel != LevelGetterKind::Expr
                    && getlevel != LevelGetterKind::Syntax
                {
                    break;
                }
                let mut i: c_int = 0;
                fp2 = fp;
                if lvl >= level {
                    let mut ll = flp.lnum - (*fp).fd_top;
                    while foldFind(&(*fp2).fd_nested, ll, &mut fp2) {
                        i += 1;
                        ll -= (*fp2).fd_top;
                    }
                }
                if lvl < level + i {
                    foldFind(&(*fp).fd_nested, flp.lnum - (*fp).fd_top, &mut fp2);
                    if !fp2.is_null() {
                        bot = (*fp2).fd_top + (*fp2).fd_len - 1 + (*fp).fd_top;
                    }
                } else if (*fp).fd_top + (*fp).fd_len <= flp.lnum && lvl >= level {
                    finish = true;
                } else {
                    break;
                }
            }

            if fp.is_null()
                && (lvl != level
                    || flp.lnum_save >= bot
                    || flp.start != 0
                    || flp.had_end <= MAX_LEVEL
                    || flp.lnum == linecount)
            {
                while !got_int {
                    let concat: linenr_T = if flp.start != 0 || flp.had_end <= MAX_LEVEL {
                        0
                    } else {
                        1
                    };

                    let ga_end = (*gap).ga_data.cast::<fold_T>().add((*gap).ga_len as usize);
                    if (*gap).ga_len > 0
                        && (foldFind(gap, startlnum, &mut fp)
                            || ((fp as usize) < (ga_end as usize) && (*fp).fd_top <= firstlnum)
                            || foldFind(gap, firstlnum - concat, &mut fp)
                            || ((fp as usize) < (ga_end as usize)
                                && ((lvl < level && (*fp).fd_top < flp.lnum)
                                    || (lvl >= level && (*fp).fd_top <= flp.lnum_save))))
                    {
                        if (*fp).fd_top + (*fp).fd_len + concat > firstlnum {
                            if (*fp).fd_top == firstlnum {
                                // exact match
                            } else if (*fp).fd_top >= startlnum {
                                if (*fp).fd_top > firstlnum {
                                    foldMarkAdjustRecurse(
                                        flp.wp,
                                        &mut (*fp).fd_nested,
                                        0,
                                        MAXLNUM,
                                        (*fp).fd_top - firstlnum,
                                        0,
                                    );
                                } else {
                                    foldMarkAdjustRecurse(
                                        flp.wp,
                                        &mut (*fp).fd_nested,
                                        0,
                                        firstlnum - (*fp).fd_top - 1,
                                        MAXLNUM,
                                        (*fp).fd_top - firstlnum,
                                    );
                                }
                                (*fp).fd_len += (*fp).fd_top - firstlnum;
                                (*fp).fd_top = firstlnum;
                                (*fp).fd_small = kNone;
                                fold_changed = true;
                            } else if (flp.start != 0 && lvl == level) || firstlnum != startlnum {
                                let (breakstart, breakend) = if firstlnum != startlnum {
                                    (startlnum, firstlnum)
                                } else {
                                    (flp.lnum, flp.lnum)
                                };
                                foldRemove(
                                    flp.wp,
                                    &mut (*fp).fd_nested,
                                    breakstart - (*fp).fd_top,
                                    breakend - (*fp).fd_top,
                                );
                                let i = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                                foldSplit((*flp.wp).w_buffer, gap, i, breakstart, breakend - 1);
                                fp = (*gap).ga_data.cast::<fold_T>().add((i + 1) as usize);
                                if getlevel == LevelGetterKind::Marker
                                    || getlevel == LevelGetterKind::Expr
                                    || getlevel == LevelGetterKind::Syntax
                                {
                                    finish = true;
                                }
                            }
                            if (*fp).fd_top == startlnum && concat != 0 {
                                let i = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                                if i != 0 {
                                    fp2 = fp.sub(1);
                                    if (*fp2).fd_top + (*fp2).fd_len == (*fp).fd_top {
                                        foldMerge(flp.wp, fp2, gap, fp);
                                        fp = fp2;
                                    }
                                }
                            }
                            break;
                        }
                        if (*fp).fd_top >= startlnum {
                            let idx = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                            deleteFoldEntry(flp.wp, gap, idx, true);
                        } else {
                            (*fp).fd_len = startlnum - (*fp).fd_top;
                            foldMarkAdjustRecurse(
                                flp.wp,
                                &mut (*fp).fd_nested,
                                (*fp).fd_len,
                                MAXLNUM,
                                MAXLNUM,
                                0,
                            );
                            fold_changed = true;
                        }
                    } else {
                        let i = if (*gap).ga_len == 0 {
                            0
                        } else {
                            fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int
                        };
                        foldInsert(gap, i);
                        fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
                        (*fp).fd_top = firstlnum;
                        (*fp).fd_len = bot - firstlnum + 1;
                        if topflags == FD_OPEN {
                            (*flp.wp).w_fold_manual = true;
                            (*fp).fd_flags = FD_OPEN;
                        } else if i <= 0 {
                            (*fp).fd_flags = topflags;
                            if topflags != FD_LEVEL {
                                (*flp.wp).w_fold_manual = true;
                            }
                        } else {
                            (*fp).fd_flags = (*fp.sub(1)).fd_flags;
                        }
                        (*fp).fd_small = kNone;
                        if getlevel == LevelGetterKind::Marker
                            || getlevel == LevelGetterKind::Expr
                            || getlevel == LevelGetterKind::Syntax
                        {
                            finish = true;
                        }
                        fold_changed = true;
                        break;
                    }
                }
            }

            if lvl < level || flp.lnum > linecount {
                break;
            }

            if lvl > level && !fp.is_null() {
                bot = bot.max(flp.lnum);

                flp.lnum = flp.lnum_save - (*fp).fd_top;
                flp.off += (*fp).fd_top;
                let i = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                bot = foldUpdateIEMSRecurse(
                    &mut (*fp).fd_nested,
                    level + 1,
                    startlnum2 - (*fp).fd_top,
                    flp,
                    getlevel,
                    bot - (*fp).fd_top,
                    (*fp).fd_flags,
                );
                fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
                flp.lnum += (*fp).fd_top;
                flp.lnum_save += (*fp).fd_top;
                flp.off -= (*fp).fd_top;
                bot += (*fp).fd_top;
                startlnum2 = flp.lnum;
            } else {
                flp.lnum = flp.lnum_save;
                let ll = flp.lnum + 1;
                while !got_int {
                    prev_lnum = flp.lnum;
                    prev_lnum_lvl = flp.lvl;

                    flp.lnum += 1;
                    if flp.lnum > linecount {
                        break;
                    }
                    flp.lvl = flp.lvl_next;
                    call_getlevel(getlevel, flp);
                    if flp.lvl >= 0 || flp.had_end <= MAX_LEVEL {
                        break;
                    }
                }
                prev_lnum = 0;
                if flp.lnum > linecount {
                    break;
                }

                flp.lnum_save = flp.lnum;
                flp.lnum = ll;
            }
        }

        if fp.is_null() {
            return bot;
        }

        if (*fp).fd_len < flp.lnum - (*fp).fd_top {
            (*fp).fd_len = flp.lnum - (*fp).fd_top;
            (*fp).fd_small = kNone;
            fold_changed = true;
        } else if (*fp).fd_top + (*fp).fd_len > linecount {
            (*fp).fd_len = linecount - (*fp).fd_top + 1;
        }

        foldRemove(
            flp.wp,
            &mut (*fp).fd_nested,
            startlnum2 - (*fp).fd_top,
            flp.lnum - 1 - (*fp).fd_top,
        );

        if lvl < level && (*fp).fd_len != flp.lnum - (*fp).fd_top {
            if (*fp).fd_top + (*fp).fd_len - 1 > bot {
                if getlevel == LevelGetterKind::Marker
                    || getlevel == LevelGetterKind::Expr
                    || getlevel == LevelGetterKind::Syntax
                {
                    bot = (*fp).fd_top + (*fp).fd_len - 1;
                    (*fp).fd_len = flp.lnum - (*fp).fd_top;
                } else {
                    let i = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                    foldSplit((*flp.wp).w_buffer, gap, i, flp.lnum, bot);
                    fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
                }
            } else {
                (*fp).fd_len = flp.lnum - (*fp).fd_top;
            }
            fold_changed = true;
        }

        loop {
            fp2 = fp.add(1);
            let ga_end = (*gap).ga_data.cast::<fold_T>().add((*gap).ga_len as usize);
            if (fp2 as usize) >= (ga_end as usize) || (*fp2).fd_top > flp.lnum {
                break;
            }
            if (*fp2).fd_top + (*fp2).fd_len > flp.lnum {
                if (*fp2).fd_top < flp.lnum {
                    foldMarkAdjustRecurse(
                        flp.wp,
                        &mut (*fp2).fd_nested,
                        0,
                        flp.lnum - (*fp2).fd_top - 1,
                        MAXLNUM,
                        (*fp2).fd_top - flp.lnum,
                    );
                    (*fp2).fd_len -= flp.lnum - (*fp2).fd_top;
                    (*fp2).fd_top = flp.lnum;
                    fold_changed = true;
                }

                if lvl >= level {
                    foldMerge(flp.wp, fp, gap, fp2);
                }
                break;
            }
            fold_changed = true;
            let idx = fp2.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
            deleteFoldEntry(flp.wp, gap, idx, true);
        }

        bot = bot.max(flp.lnum - 1);
        bot
    }
}

unsafe fn foldInsert(gap: *mut garray_T, i: c_int) {
    unsafe {
        ga_grow(gap, 1);

        let fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
        if (*gap).ga_len > 0 && i < (*gap).ga_len {
            ptr::copy(fp, fp.add(1), ((*gap).ga_len - i) as usize);
        }
        (*gap).ga_len += 1;
        ga_init(
            &mut (*fp).fd_nested,
            std::mem::size_of::<fold_T>() as c_int,
            10,
        );
    }
}

unsafe fn foldSplit(_buf: *mut buf_T, gap: *mut garray_T, i: c_int, top: linenr_T, bot: linenr_T) {
    unsafe {
        let mut fp2: *mut fold_T = ptr::null_mut();

        foldInsert(gap, i + 1);

        let fp = (*gap).ga_data.cast::<fold_T>().add(i as usize);
        let next_fp = fp.add(1);
        (*next_fp).fd_top = bot + 1;
        debug_assert!((*next_fp).fd_top > bot);
        (*next_fp).fd_len = (*fp).fd_len - ((*next_fp).fd_top - (*fp).fd_top);
        (*next_fp).fd_flags = (*fp).fd_flags;
        (*next_fp).fd_small = kNone;
        (*fp).fd_small = kNone;

        let gap1 = &mut (*fp).fd_nested;
        let gap2 = &mut (*next_fp).fd_nested;
        foldFind(gap1, bot + 1 - (*fp).fd_top, &mut fp2);
        if !fp2.is_null() {
            let ga1_end = gap1.ga_data.cast::<fold_T>().add(gap1.ga_len as usize);
            let len = ga1_end.offset_from(fp2) as c_int;
            if len > 0 {
                ga_grow(gap2, len);
                let g2_data = gap2.ga_data.cast::<fold_T>();
                for idx in 0..(len as usize) {
                    *g2_data.add(idx) = *fp2.add(idx);
                    (*g2_data.add(idx)).fd_top -= (*next_fp).fd_top - (*fp).fd_top;
                }
                gap2.ga_len = len;
                gap1.ga_len -= len;
            }
        }
        (*fp).fd_len = top - (*fp).fd_top;
        fold_changed = true;
    }
}

unsafe fn foldRemove(wp: *mut win_T, gap: *mut garray_T, top: linenr_T, bot: linenr_T) {
    unsafe {
        if bot < top {
            return;
        }

        let mut fp: *mut fold_T = ptr::null_mut();

        loop {
            if (*gap).ga_len <= 0 {
                break;
            }
            if foldFind(gap, top, &mut fp) && (*fp).fd_top < top {
                foldRemove(
                    wp,
                    &mut (*fp).fd_nested,
                    top - (*fp).fd_top,
                    bot - (*fp).fd_top,
                );
                if (*fp).fd_top + (*fp).fd_len - 1 > bot {
                    let idx = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                    foldSplit((*wp).w_buffer, gap, idx, top, bot);
                } else {
                    (*fp).fd_len = top - (*fp).fd_top;
                }
                fold_changed = true;
                continue;
            }
            let ga_end = (*gap).ga_data.cast::<fold_T>().add((*gap).ga_len as usize);
            if (*gap).ga_data.is_null() || (fp as usize) >= (ga_end as usize) || (*fp).fd_top > bot
            {
                break;
            }
            if (*fp).fd_top >= top {
                fold_changed = true;
                if (*fp).fd_top + (*fp).fd_len - 1 > bot {
                    foldMarkAdjustRecurse(
                        wp,
                        &mut (*fp).fd_nested,
                        0,
                        bot - (*fp).fd_top,
                        MAXLNUM,
                        (*fp).fd_top - bot - 1,
                    );
                    (*fp).fd_len -= bot - (*fp).fd_top + 1;
                    (*fp).fd_top = bot + 1;
                    break;
                }

                let idx = fp.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
                deleteFoldEntry(wp, gap, idx, true);
            }
        }
    }
}

unsafe fn foldReverseOrder(gap: *mut garray_T, mut start: linenr_T, mut end: linenr_T) {
    unsafe {
        let base = (*gap).ga_data.cast::<fold_T>();
        while start < end {
            let left = base.add(start as usize);
            let right = base.add(end as usize);
            ptr::swap(left, right);
            start += 1;
            end -= 1;
        }
    }
}

unsafe fn truncate_fold(wp: *mut win_T, fp: *mut fold_T, mut end: linenr_T) {
    unsafe {
        end += 1;
        foldRemove(wp, &mut (*fp).fd_nested, end - (*fp).fd_top, MAXLNUM);
        (*fp).fd_len = end - (*fp).fd_top;
    }
}

#[inline]
unsafe fn fold_end(fp: *const fold_T) -> linenr_T {
    unsafe { (*fp).fd_top + (*fp).fd_len - 1 }
}

#[inline]
unsafe fn valid_fold(fp: *const fold_T, gap: *const garray_T) -> bool {
    unsafe {
        (*gap).ga_len > 0
            && (fp as usize)
                < ((*gap).ga_data.cast::<fold_T>().add((*gap).ga_len as usize) as usize)
    }
}

#[inline]
unsafe fn fold_index(fp: *const fold_T, gap: *const garray_T) -> usize {
    unsafe { fp.offset_from((*gap).ga_data.cast::<fold_T>()) as usize }
}

/// Move folds within the inclusive range "line1" to "line2" to after "dest".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn foldMoveRange(
    wp: *mut win_T,
    gap: *mut garray_T,
    line1: linenr_T,
    line2: linenr_T,
    dest: linenr_T,
) {
    unsafe {
        let mut fp: *mut fold_T = ptr::null_mut();
        let range_len = line2 - line1 + 1;
        let move_len = dest - line2;
        let at_start = foldFind(gap, line1 - 1, &mut fp);

        if at_start {
            if fold_end(fp) > dest {
                foldMoveRange(
                    wp,
                    &mut (*fp).fd_nested,
                    line1 - (*fp).fd_top,
                    line2 - (*fp).fd_top,
                    dest - (*fp).fd_top,
                );
                return;
            } else if fold_end(fp) > line2 {
                foldMarkAdjustRecurse(
                    wp,
                    &mut (*fp).fd_nested,
                    line1 - (*fp).fd_top,
                    line2 - (*fp).fd_top,
                    MAXLNUM,
                    -range_len,
                );
                (*fp).fd_len -= range_len;
            } else {
                truncate_fold(wp, fp, line1 - 1);
            }
            fp = fp.add(1);
        }

        if !valid_fold(fp, gap) || (*fp).fd_top > dest {
            return;
        } else if (*fp).fd_top > line2 {
            while valid_fold(fp, gap) && fold_end(fp) <= dest {
                (*fp).fd_top -= range_len;
                fp = fp.add(1);
            }
            if valid_fold(fp, gap) && (*fp).fd_top <= dest {
                truncate_fold(wp, fp, dest);
                (*fp).fd_top -= range_len;
            }
            return;
        } else if fold_end(fp) > dest {
            foldMarkAdjustRecurse(
                wp,
                &mut (*fp).fd_nested,
                line2 + 1 - (*fp).fd_top,
                dest - (*fp).fd_top,
                MAXLNUM,
                -move_len,
            );
            (*fp).fd_len -= move_len;
            (*fp).fd_top += move_len;
            return;
        }

        let move_start = fold_index(fp, gap);
        let mut move_end: usize = 0;
        while valid_fold(fp, gap) && (*fp).fd_top <= dest {
            if (*fp).fd_top <= line2 {
                if fold_end(fp) > line2 {
                    truncate_fold(wp, fp, line2);
                }
                (*fp).fd_top += move_len;
                fp = fp.add(1);
                continue;
            }

            if move_end == 0 {
                move_end = fold_index(fp, gap);
            }

            if fold_end(fp) > dest {
                truncate_fold(wp, fp, dest);
            }

            (*fp).fd_top -= range_len;
            fp = fp.add(1);
        }
        let dest_index = fold_index(fp, gap);

        if move_end == 0 {
            return;
        }
        foldReverseOrder(gap, move_start as linenr_T, (dest_index - 1) as linenr_T);
        foldReverseOrder(
            gap,
            move_start as linenr_T,
            (move_start + dest_index - move_end - 1) as linenr_T,
        );
        foldReverseOrder(
            gap,
            (move_start + dest_index - move_end) as linenr_T,
            (dest_index - 1) as linenr_T,
        );
    }
}

unsafe fn foldMerge(wp: *mut win_T, fp1: *mut fold_T, gap: *mut garray_T, fp2: *mut fold_T) {
    unsafe {
        let mut fp3: *mut fold_T = ptr::null_mut();
        let mut fp4: *mut fold_T = ptr::null_mut();
        let gap1 = &mut (*fp1).fd_nested;
        let gap2 = &mut (*fp2).fd_nested;

        if foldFind(gap1, (*fp1).fd_len - 1, &mut fp3) && foldFind(gap2, 0, &mut fp4) {
            foldMerge(wp, fp3, gap2, fp4);
        }

        if gap2.ga_len > 0 {
            ga_grow(gap1, gap2.ga_len);
            let g1_data = gap1.ga_data.cast::<fold_T>();
            let g2_data = gap2.ga_data.cast::<fold_T>();
            for idx in 0..(gap2.ga_len as usize) {
                let dst = g1_data.add(gap1.ga_len as usize);
                *dst = *g2_data.add(idx);
                (*dst).fd_top += (*fp1).fd_len;
                gap1.ga_len += 1;
            }
            gap2.ga_len = 0;
        }

        (*fp1).fd_len += (*fp2).fd_len;
        let idx = fp2.offset_from((*gap).ga_data.cast::<fold_T>()) as c_int;
        deleteFoldEntry(wp, gap, idx, true);
        fold_changed = true;
    }
}

unsafe fn foldlevelIndent(flp: &mut fline_T) {
    unsafe {
        let lnum = flp.lnum + flp.off;
        let buf = (*flp.wp).w_buffer;
        let s = skipwhite(ml_get_buf(buf, lnum));

        if *s == NUL || !vim_strchr((*flp.wp).w_onebuf_opt.wo_fdi, *s as u8 as c_int).is_null() {
            flp.lvl = if lnum == 1 || lnum == (*buf).b_ml.ml_line_count {
                0
            } else {
                -1
            };
        } else {
            flp.lvl = get_indent_buf(buf, lnum) / get_sw_value(buf);
        }
        flp.lvl = flp.lvl.min((*flp.wp).w_onebuf_opt.wo_fdn.max(0) as c_int);
    }
}

unsafe fn foldlevelDiff(flp: &mut fline_T) {
    unsafe {
        flp.lvl = c_int::from(diff_infold(flp.wp, flp.lnum + flp.off));
    }
}

unsafe fn foldlevelExpr(flp: &mut fline_T) {
    unsafe {
        let lnum = flp.lnum + flp.off;

        let win = curwin;
        curwin = flp.wp;
        curbuf = (*flp.wp).w_buffer;
        set_vim_var_nr(VV_LNUM, varnumber_T::from(lnum));

        flp.start = 0;
        flp.had_end = flp.end;
        flp.end = MAX_LEVEL + 1;
        if lnum <= 1 {
            flp.lvl = 0;
        }

        let save_keytyped = KeyTyped;
        let mut c: c_int = 0;
        let n = eval_foldexpr(flp.wp, &mut c);
        KeyTyped = save_keytyped;

        match c as u8 {
            b'a' => {
                if flp.lvl >= 0 {
                    flp.lvl += n;
                    flp.lvl_next = flp.lvl;
                }
                flp.start = n;
            }
            b's' => {
                if flp.lvl >= 0 {
                    if n > flp.lvl {
                        flp.lvl_next = 0;
                    } else {
                        flp.lvl_next = flp.lvl - n;
                    }
                    flp.end = flp.lvl_next + 1;
                }
            }
            b'>' => {
                flp.lvl = n;
                flp.lvl_next = n;
                flp.start = 1;
            }
            b'<' => {
                flp.lvl_next = flp.lvl.min(n - 1);
                flp.end = n;
            }
            b'=' => {
                flp.lvl_next = flp.lvl;
            }
            _ => {
                if n < 0 {
                    flp.lvl_next = flp.lvl;
                } else {
                    flp.lvl_next = n;
                }
                flp.lvl = n;
            }
        }

        if flp.lvl < 0 {
            if lnum <= 1 {
                flp.lvl = 0;
                flp.lvl_next = 0;
            }
            if lnum == (*curbuf).b_ml.ml_line_count {
                flp.lvl_next = 0;
            }
        }

        curwin = win;
        curbuf = (*curwin).w_buffer;
    }
}

unsafe fn parseMarker(wp: *mut win_T) {
    unsafe {
        let fmr = (*wp).w_onebuf_opt.wo_fmr;
        let comma = vim_strchr(fmr, b',' as c_int);
        foldstartmarkerlen = (comma as usize) - (fmr as usize);
        foldendmarker = comma.add(1);
        foldendmarkerlen = CStr::from_ptr(foldendmarker).to_bytes().len();
    }
}

unsafe fn foldlevelMarker(flp: &mut fline_T) {
    unsafe {
        let start_lvl = flp.lvl;
        let startmarker_full = (*flp.wp).w_onebuf_opt.wo_fmr;
        let cstart = *startmarker_full;
        let startmarker = startmarker_full.add(1);
        let cend = *foldendmarker;

        let start_rest =
            std::slice::from_raw_parts(startmarker.cast::<u8>(), foldstartmarkerlen - 1);
        let end_rest =
            std::slice::from_raw_parts(foldendmarker.add(1).cast::<u8>(), foldendmarkerlen - 1);

        flp.start = 0;
        flp.lvl_next = flp.lvl;

        let mut s = ml_get_buf((*flp.wp).w_buffer, flp.lnum + flp.off);
        while *s != NUL {
            let rem_bytes = CStr::from_ptr(s.add(1)).to_bytes();
            if *s == cstart && rem_bytes.starts_with(start_rest) {
                s = s.add(foldstartmarkerlen);
                if ascii_isdigit(*s as u8 as c_int) {
                    let n = atoi(s);
                    if n > 0 {
                        flp.lvl = n;
                        flp.lvl_next = n;
                        flp.start = (n - start_lvl).max(1);
                    }
                } else {
                    flp.lvl += 1;
                    flp.lvl_next += 1;
                    flp.start += 1;
                }
            } else if *s == cend && rem_bytes.starts_with(end_rest) {
                s = s.add(foldendmarkerlen);
                if ascii_isdigit(*s as u8 as c_int) {
                    let n = atoi(s);
                    if n > 0 {
                        flp.lvl = n;
                        flp.lvl_next = (n - 1).min(start_lvl);
                    }
                } else {
                    flp.lvl_next -= 1;
                }
            } else {
                s = s.add(utfc_ptr2len(s) as usize);
            }
        }

        flp.lvl_next = flp.lvl_next.max(0);
    }
}

unsafe fn foldlevelSyntax(flp: &mut fline_T) {
    unsafe {
        let lnum = flp.lnum + flp.off;

        flp.lvl = syn_get_foldlevel(flp.wp, lnum);
        flp.start = 0;
        if lnum < (*(*flp.wp).w_buffer).b_ml.ml_line_count {
            let n = syn_get_foldlevel(flp.wp, lnum + 1);
            if n > flp.lvl {
                flp.start = n - flp.lvl;
                flp.lvl = n;
            }
        }
    }
}

/// Write commands to "fd" to restore the manual folds in window "wp".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn put_folds(fd: *mut FILE, wp: *mut win_T) -> c_int {
    unsafe {
        if foldmethodIsManual(wp)
            && (put_line(fd, c"silent! normal! zE".as_ptr() as *mut c_char) == FAIL
                || put_folds_recurse(fd, &(*wp).w_folds, 0) == FAIL
                || put_line(fd, c"let &fdl = &fdl".as_ptr() as *mut c_char) == FAIL)
        {
            return FAIL;
        }

        if (*wp).w_fold_manual {
            return put_foldopen_recurse(fd, wp, &(*wp).w_folds, 0);
        }

        OK
    }
}

unsafe fn put_folds_recurse(fd: *mut FILE, gap: *const garray_T, off: linenr_T) -> c_int {
    unsafe {
        let mut fp = (*gap).ga_data.cast::<fold_T>();
        for _ in 0..(*gap).ga_len {
            if put_folds_recurse(fd, &(*fp).fd_nested, off + (*fp).fd_top) == FAIL {
                return FAIL;
            }
            let mut buf = [0 as c_char; 64];
            vim_snprintf!(
                buf.as_mut_ptr(),
                buf.len(),
                c"sil! %lld,%lldfold".as_ptr(),
                i64::from((*fp).fd_top + off),
                i64::from((*fp).fd_top + off + (*fp).fd_len - 1),
            );
            if fputs(buf.as_ptr(), fd) < 0 || put_eol(fd) == FAIL {
                return FAIL;
            }
            fp = fp.add(1);
        }
        OK
    }
}

unsafe fn put_foldopen_recurse(
    fd: *mut FILE,
    wp: *mut win_T,
    gap: *const garray_T,
    off: linenr_T,
) -> c_int {
    unsafe {
        let mut fp = (*gap).ga_data.cast::<fold_T>();
        for _ in 0..(*gap).ga_len {
            if (*fp).fd_flags != FD_LEVEL {
                if (*fp).fd_nested.ga_len > 0 {
                    let mut buf = [0 as c_char; 32];
                    vim_snprintf!(
                        buf.as_mut_ptr(),
                        buf.len(),
                        c"%lld".as_ptr(),
                        i64::from((*fp).fd_top + off),
                    );
                    if fputs(buf.as_ptr(), fd) < 0
                        || put_eol(fd) == FAIL
                        || put_line(fd, c"sil! normal! zo".as_ptr() as *mut c_char) == FAIL
                    {
                        return FAIL;
                    }
                    if put_foldopen_recurse(fd, wp, &(*fp).fd_nested, off + (*fp).fd_top) == FAIL {
                        return FAIL;
                    }
                    if (*fp).fd_flags == FD_CLOSED && put_fold_open_close(fd, fp, off) == FAIL {
                        return FAIL;
                    }
                } else {
                    let level = i64::from(foldLevelWin(wp, off + (*fp).fd_top));
                    if (((*fp).fd_flags == FD_CLOSED && (*wp).w_onebuf_opt.wo_fdl >= level)
                        || ((*fp).fd_flags != FD_CLOSED && (*wp).w_onebuf_opt.wo_fdl < level))
                        && put_fold_open_close(fd, fp, off) == FAIL
                    {
                        return FAIL;
                    }
                }
            }
            fp = fp.add(1);
        }

        OK
    }
}

unsafe fn put_fold_open_close(fd: *mut FILE, fp: *const fold_T, off: linenr_T) -> c_int {
    unsafe {
        let mut buf = [0 as c_char; 32];
        vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"%d".as_ptr(),
            (*fp).fd_top + off,
        );
        if fputs(buf.as_ptr(), fd) < 0 || put_eol(fd) == FAIL {
            return FAIL;
        }
        let cmd = if (*fp).fd_flags == FD_CLOSED {
            c"sil! normal! zc".as_ptr()
        } else {
            c"sil! normal! zo".as_ptr()
        };
        if fputs(cmd, fd) < 0 || put_eol(fd) == FAIL {
            return FAIL;
        }
        OK
    }
}

unsafe fn foldclosed_both(argvars: *mut typval_T, rettv: *mut typval_T, end: bool) {
    unsafe {
        let lnum = tv_get_lnum(argvars);
        if lnum >= 1 && lnum <= (*curbuf).b_ml.ml_line_count {
            let mut first: linenr_T = 0;
            let mut last: linenr_T = 0;
            if hasFoldingWin(curwin, lnum, &mut first, &mut last, false, ptr::null_mut()) {
                (*rettv).vval.v_number = varnumber_T::from(if end { last } else { first });
                return;
            }
        }
        (*rettv).vval.v_number = -1;
    }
}

/// "foldclosed()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_foldclosed(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        foldclosed_both(argvars, rettv, false);
    }
}

/// "foldclosedend()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_foldclosedend(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        foldclosed_both(argvars, rettv, true);
    }
}

/// "foldlevel()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_foldlevel(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let lnum = tv_get_lnum(argvars);
        if lnum >= 1 && lnum <= (*curbuf).b_ml.ml_line_count {
            (*rettv).vval.v_number = varnumber_T::from(foldLevel(lnum));
        }
    }
}

/// "foldtext()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_foldtext(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let foldstart = get_vim_var_nr(VV_FOLDSTART) as linenr_T;
        let foldend = get_vim_var_nr(VV_FOLDEND) as linenr_T;
        let dashes = get_vim_var_str(VV_FOLDDASHES);
        if foldstart > 0 && foldend <= (*curbuf).b_ml.ml_line_count {
            let mut lnum = foldstart;
            while lnum < foldend {
                if !linewhite(lnum) {
                    break;
                }
                lnum += 1;
            }

            let mut s = skipwhite(ml_get(lnum));
            if *s == b'/' as c_char && (*s.add(1) == b'*' as c_char || *s.add(1) == b'/' as c_char)
            {
                s = skipwhite(s.add(2));
                if *skipwhite(s) == NUL && lnum + 1 < foldend {
                    s = skipwhite(ml_get(lnum + 1));
                    if *s == b'*' as c_char {
                        s = skipwhite(s.add(1));
                    }
                }
            }
            let count = foldend - foldstart + 1;
            let txt = ngettext_wrap(
                c"+-%s%3d line: ".as_ptr(),
                c"+-%s%3d lines: ".as_ptr(),
                count as c_ulong,
            );
            let s_len = CStr::from_ptr(s).to_bytes().len();
            let len = CStr::from_ptr(txt).to_bytes().len()
                + CStr::from_ptr(dashes).to_bytes().len()
                + 20
                + s_len;
            let r = xmalloc(len).cast::<c_char>();
            vim_snprintf!(r, len, txt, dashes, count);
            let r_len = CStr::from_ptr(r).to_bytes().len();
            ptr::copy_nonoverlapping(s, r.add(r_len), s_len + 1);
            foldtext_cleanup(r.add(r_len));
            (*rettv).vval.v_string = r;
        }
    }
}

/// "foldtextresult(lnum)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_foldtextresult(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut buf = [0 as c_char; FOLD_TEXT_LEN];

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if fdt_result_entered {
            return;
        }
        fdt_result_entered = true;
        let lnum = tv_get_lnum(argvars).max(0);

        let info = fold_info(curwin, lnum);
        if info.fi_lines > 0 {
            let mut vt = VirtText {
                size: 0,
                capacity: 0,
                items: ptr::null_mut(),
            };
            let mut text = get_foldtext(
                curwin,
                lnum,
                lnum + info.fi_lines - 1,
                info,
                buf.as_mut_ptr(),
                &mut vt,
            );
            if ptr::eq(text, buf.as_ptr()) {
                text = xstrdup(text);
            }
            if vt.size > 0 {
                debug_assert_eq!(*text, NUL);
                let mut i: usize = 0;
                while i < vt.size {
                    let mut attr: c_int = 0;
                    let new_text = next_virt_text_chunk(vt, &mut i, &mut attr);
                    if new_text.is_null() {
                        break;
                    }
                    let joined = concat_str(text, new_text);
                    xfree(text.cast::<c_void>());
                    text = joined;
                }
            }
            clear_virttext(&mut vt);
            (*rettv).vval.v_string = text;
        }

        fdt_result_entered = false;
    }
}
