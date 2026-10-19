//! Functions for moving the cursor and scrolling text (`src/nvim/move.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::errors::*;
use crate::globals::*;
use crate::option_vars::*;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::api::tabpage::{ScreenGrid, pos_T, win_T};
use crate::clipboard::typval_T;
use crate::cursor::{check_cursor, check_cursor_lnum, coladvance};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::eval::buffer::varnumber_T;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::buf_T;
use crate::eval::window::{find_win_by_nr_or_id, viewstate_T};
use crate::grid::wline_T;
use crate::input_cmdatom::cmdarg_T;
use crate::mouse::vcol2col;
use crate::plines::{
    colnr_T, getvcol, getvvcol, linenr_T, linetabsize_eol, plines_m_win, plines_win,
    plines_win_full, plines_win_nofill, win_may_fill,
};
use crate::textformat::oparg_T;
use crate::winfloat::win_check_anchored_floats;

pub type OptInt = i64;
pub type Direction = c_int;
pub type EvalFuncData = *mut c_void;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const FORWARD: Direction = 1;
const BACKWARD: Direction = -1;
const MAXCOL: colnr_T = 0x7fffffff;
const SIGN_WIDTH: c_int = 2;

const VALID_WROW: c_int = 0x01;
const VALID_WCOL: c_int = 0x02;
const VALID_VIRTCOL: c_int = 0x04;
const VALID_CHEIGHT: c_int = 0x08;
const VALID_CROW: c_int = 0x10;
const VALID_BOTLINE: c_int = 0x20;
const VALID_BOTLINE_AP: c_int = 0x40;
const VALID_TOPLINE: c_int = 0x80;

const UPD_VALID: c_int = 10;
const UPD_INVERTED: c_int = 20;
const UPD_SOME_VALID: c_int = 35;
const UPD_NOT_VALID: c_int = 40;

const BL_SOL: c_int = 2;
const BL_FIX: c_int = 4;

const kOptCuloptFlagScreenline: u8 = 0x02;
const kCpoNumcol: c_int = b'n' as c_int;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct lineoff_T {
    lnum: linenr_T,
    fill: c_int,
    height: c_int,
}

unsafe extern "C" {
    static mut default_grid: ScreenGrid;

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn redrawing() -> bool;
    fn win_cursorline_standout(wp: *const win_T) -> bool;
    fn redraw_later(wp: *mut win_T, type_: c_int);
    fn conceal_cursor_line(wp: *const win_T) -> bool;
    fn redrawWinline(wp: *mut win_T, lnum: linenr_T);
    fn redraw_buf_later(buf: *mut buf_T, type_: c_int);
    fn get_showbreak_value(win: *const win_T) -> *mut c_char;
    fn get_scrolloff_value(wp: *mut win_T) -> i64;
    fn get_scrolloffpad_value(wp: *mut win_T) -> i64;
    fn get_sidescrolloff_value(wp: *mut win_T) -> i64;
    fn buf_is_empty(buf: *mut buf_T) -> bool;
    fn win_get_fill(wp: *mut win_T, lnum: linenr_T) -> c_int;
    fn hasFolding(
        win: *mut win_T,
        lnum: linenr_T,
        firstp: *mut linenr_T,
        lastp: *mut linenr_T,
    ) -> bool;
    fn number_width(wp: *mut win_T) -> c_int;
    fn win_fdccol_count(wp: *mut win_T) -> c_int;
    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;
    fn win_scroll_lines(wp: *mut win_T, row: c_int, line_count: c_int);
    fn tv_dict_alloc_ret(ret_tv: *mut typval_T);
    fn tv_get_number(varp: *const typval_T) -> varnumber_T;
    fn tv_get_number_chk(varp: *const typval_T, denote: *mut bool) -> varnumber_T;
    fn tv_check_for_number_arg(args: *mut typval_T, idx: c_int) -> c_int;
    fn tv_dict_add_nr(d: *mut dict_T, key: *const c_char, key_len: usize, nr: varnumber_T)
    -> c_int;
    fn ml_get_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char;
    fn utf_head_off(base_in: *const c_char, p_in: *const c_char) -> c_int;
    fn cursor_down(n: c_int, upd_topline: bool) -> c_int;
    fn cursor_up(n: linenr_T, upd_topline: bool) -> c_int;
    fn cursor_down_inner(wp: *mut win_T, n: c_int, skip_conceal: bool);
    fn cursor_up_inner(wp: *mut win_T, n: linenr_T, skip_conceal: bool);
    fn foldAdjustCursor(wp: *mut win_T);
    fn nv_screengo(oap: *mut oparg_T, dir: c_int, dist: c_int, skip_conceal: bool) -> bool;
    fn beep_flush();
    fn beginline(flags: c_int);
    fn nv_g_home_m_cmd(cap: *mut cmdarg_T);
    fn diff_get_corresponding_line(buf1: *mut buf_T, lnum1: linenr_T) -> linenr_T;
    fn mb_adjust_cursor();
}

#[inline]
unsafe fn tr(s: *const c_char) -> *const c_char {
    unsafe { gettext(s) }
}

#[inline]
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
unsafe fn buf_get_changedtick(buf: *const buf_T) -> varnumber_T {
    unsafe { (*buf).changedtick_di.di_tv.vval.v_number }
}

/// Get the number of screen lines skipped with `wp->w_skipcol`.
unsafe fn adjust_plines_for_skipcol(wp: *mut win_T) -> c_int {
    unsafe {
        if (*wp).w_skipcol == 0 {
            return 0;
        }

        let width = (*wp).w_view_width - win_col_off(wp);
        let w2 = width + win_col_off2(wp);
        if (*wp).w_skipcol >= width && w2 > 0 {
            return ((*wp).w_skipcol - width) / w2 + 1;
        }

        0
    }
}

/// Return how many lines `lnum` will take on the screen, taking into account
/// whether it is the first line, whether `w_skipcol` is non-zero and limiting
/// to the window height.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_correct_topline(
    wp: *mut win_T,
    lnum: linenr_T,
    nextp: *mut linenr_T,
    limit_winheight: bool,
    foldedp: *mut bool,
) -> c_int {
    unsafe {
        let mut n = plines_win_full(wp, lnum, nextp, foldedp, true, false);
        if lnum == (*wp).w_topline {
            n -= adjust_plines_for_skipcol(wp);
        }
        if limit_winheight && n > (*wp).w_view_height {
            return (*wp).w_view_height;
        }
        n
    }
}

/// Compute `wp->w_botline` for the current `wp->w_topline`.
unsafe fn comp_botline(wp: *mut win_T) {
    unsafe {
        check_cursor_moved(wp);

        let w_lines = (*wp).w_lines.cast::<wline_T>();
        let use_cache = redrawing()
            && !(*(*wp).w_buffer).b_mod_set
            && dollar_vcol == -1
            && (*wp).w_skipcol == 0
            && !win_may_fill(wp)
            && (*wp).w_lines_valid > 0
            && (*w_lines).wl_lnum <= (*wp).w_topline;

        let (mut lnum, mut done) = if ((*wp).w_valid & VALID_CROW) != 0 {
            ((*wp).w_cursor.lnum, (*wp).w_cline_row)
        } else {
            ((*wp).w_topline, 0)
        };

        let mut i: c_int = 0;
        if use_cache {
            while i < (*wp).w_lines_valid && (*w_lines.add(i as usize)).wl_lnum < lnum {
                i += 1;
            }
        }

        while lnum <= (*(*wp).w_buffer).b_ml.ml_line_count {
            let mut valid = false;
            if use_cache && i < (*wp).w_lines_valid {
                let wl = w_lines.add(i as usize);
                if (*wl).wl_lnum < lnum || !(*wl).wl_valid {
                    i += 1;
                    continue;
                }
                if (*wl).wl_lnum == lnum {
                    valid = true;
                } else {
                    i -= 1;
                }
            }
            let mut last = lnum;
            let mut folded = false;
            let n = if valid {
                let wl = w_lines.add(i as usize);
                folded = (*wl).wl_folded;
                last = (*wl).wl_lastlnum;
                c_int::from((*wl).wl_size)
            } else {
                plines_correct_topline(wp, lnum, &raw mut last, true, &raw mut folded)
            };
            if lnum <= (*wp).w_cursor.lnum && last >= (*wp).w_cursor.lnum {
                (*wp).w_cline_row = done;
                (*wp).w_cline_height = n;
                (*wp).w_cline_folded = folded;
                redraw_for_cursorline(wp);
                (*wp).w_valid |= VALID_CROW | VALID_CHEIGHT;
            }
            if done + n > (*wp).w_view_height {
                break;
            }
            done += n;
            lnum = last + 1;
            i += 1;
        }

        (*wp).w_botline = lnum;
        (*wp).w_valid |= VALID_BOTLINE | VALID_BOTLINE_AP;
        (*wp).w_viewport_invalid = true;

        set_empty_rows(wp, done);

        win_check_anchored_floats(wp);
    }
}

/// Redraw when `w_cline_row` changes and `'relativenumber'` or `'cursorline'` is set.
unsafe fn redraw_for_cursorline(wp: *mut win_T) {
    unsafe {
        if ((*wp).w_valid & VALID_CROW) != 0 {
            return;
        }

        if (*wp).w_onebuf_opt.wo_rnu != 0 || win_cursorline_standout(wp) {
            redraw_later(wp, UPD_VALID);
        }
    }
}

/// Redraw when `'concealcursor'` is active, or when `w_virtcol` changes.
unsafe fn redraw_for_cursorcolumn(wp: *mut win_T) {
    unsafe {
        if wp == curwin && (*wp).w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(wp) {
            redrawWinline(wp, (*wp).w_cursor.lnum);
        }

        if ((*wp).w_valid & VALID_VIRTCOL) != 0 {
            return;
        }

        if (*wp).w_onebuf_opt.wo_cuc != 0 {
            redraw_later(wp, UPD_SOME_VALID);
        } else if (*wp).w_onebuf_opt.wo_cul != 0
            && ((*wp).w_p_culopt_flags & kOptCuloptFlagScreenline) != 0
        {
            redraw_later(wp, UPD_VALID);
        }

        if Visual.active && (*wp).w_buffer == curbuf {
            redraw_buf_later(curbuf, UPD_INVERTED);
        }
    }
}

/// Set `wp->w_virtcol` to a value (`vcol`) that is already valid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_valid_virtcol(wp: *mut win_T, vcol: colnr_T) {
    unsafe {
        (*wp).w_virtcol = vcol;
        redraw_for_cursorcolumn(wp);
        (*wp).w_valid |= VALID_VIRTCOL;
    }
}

/// Calculates how much the `'listchars'` `"precedes"` or `'smoothscroll'` `"<<<"`
/// marker overlaps with buffer text for window `wp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn sms_marker_overlap(wp: *mut win_T, mut extra2: c_int) -> c_int {
    unsafe {
        if extra2 == -1 {
            extra2 = win_col_off(wp) - win_col_off2(wp);
        }
        if *get_showbreak_value(wp) != NUL {
            return 0;
        }

        if (*wp).w_onebuf_opt.wo_list != 0 && (*wp).w_p_lcs_chars.prec != 0 {
            return 1;
        }

        if extra2 > 3 { 0 } else { 3 - extra2 }
    }
}

/// Calculates the skipcol offset for window `wp` given how many physical lines
/// we want to scroll down.
unsafe fn skipcol_from_plines(wp: *mut win_T, plines_off: c_int) -> c_int {
    unsafe {
        let width1 = (*wp).w_view_width - win_col_off(wp);

        let mut skipcol: c_int = 0;
        if plines_off > 0 {
            skipcol += width1;
        }
        if plines_off > 1 {
            skipcol += (width1 + win_col_off2(wp)) * (plines_off - 1);
        }
        skipcol
    }
}

/// Set `wp->w_skipcol` to zero and redraw later if needed.
unsafe fn reset_skipcol(wp: *mut win_T) {
    unsafe {
        if (*wp).w_skipcol == 0 {
            return;
        }

        (*wp).w_skipcol = 0;
        redraw_later(wp, UPD_SOME_VALID);
    }
}

/// Return true when `'scrolloffpad'` may augment `'scrolloff'`.
unsafe fn use_scrolloffpad(wp: *mut win_T) -> bool {
    unsafe { get_scrolloff_value(wp) > 0 && get_scrolloffpad_value(wp) > 0 }
}

/// Return true when there are not enough real buffer lines below `lnum` to
/// satisfy the requested `so` context.
unsafe fn scrolloffpad_eof_pressure(wp: *mut win_T, lnum: linenr_T, so: OptInt) -> bool {
    unsafe {
        if !use_scrolloffpad(wp) || so <= 0 {
            return false;
        }

        i64::from(lnum) > i64::from((*(*wp).w_buffer).b_ml.ml_line_count) - so
    }
}

/// Update `wp->w_topline` to move the cursor onto the screen.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_topline(wp: *mut win_T) {
    unsafe {
        let mut check_botline = false;
        let so_ptr: *mut OptInt = if (*wp).w_onebuf_opt.wo_so >= 0 {
            &raw mut (*wp).w_onebuf_opt.wo_so
        } else {
            &raw mut p_so
        };
        let save_so = *so_ptr;

        if skip_update_topline {
            return;
        }

        if default_grid.chars.is_null() || (*wp).w_view_height == 0 {
            check_cursor_lnum(wp);
            (*wp).w_topline = (*wp).w_cursor.lnum;
            (*wp).w_botline = (*wp).w_topline;
            (*wp).w_viewport_invalid = true;
            (*wp).w_scbind_pos = 1;
            return;
        }

        check_cursor_moved(wp);
        if ((*wp).w_valid & VALID_TOPLINE) != 0 {
            return;
        }

        if mouse_dragging > 0 {
            *so_ptr = i64::from(mouse_dragging - 1);
        }
        let eof_pressure = scrolloffpad_eof_pressure(wp, (*wp).w_cursor.lnum, *so_ptr);

        let old_topline = (*wp).w_topline;
        let old_topfill = (*wp).w_topfill;

        if buf_is_empty((*wp).w_buffer) {
            if (*wp).w_topline != 1 {
                redraw_later(wp, UPD_NOT_VALID);
            }
            (*wp).w_topline = 1;
            (*wp).w_botline = 2;
            (*wp).w_skipcol = 0;
            (*wp).w_valid |= VALID_BOTLINE | VALID_BOTLINE_AP;
            (*wp).w_viewport_invalid = true;
            (*wp).w_scbind_pos = 1;
        } else {
            let mut check_topline = false;
            if (*wp).w_topline > 1 || (*wp).w_skipcol > 0 {
                if (*wp).w_cursor.lnum < (*wp).w_topline || check_top_offset(wp) {
                    check_topline = true;
                } else if (*wp).w_skipcol > 0 && (*wp).w_cursor.lnum == (*wp).w_topline {
                    let mut vcol: colnr_T = 0;
                    getvvcol(
                        wp,
                        &raw mut (*wp).w_cursor,
                        &raw mut vcol,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        0,
                    );
                    let overlap = sms_marker_overlap(wp, -1);
                    if (*wp).w_skipcol + overlap > vcol {
                        check_topline = true;
                    }
                }
            }
            if !check_topline && (*wp).w_topfill > win_get_fill(wp, (*wp).w_topline) {
                check_topline = true;
            }

            if check_topline {
                let mut halfheight = (*wp).w_view_height / 2 - 1;
                if halfheight < 2 {
                    halfheight = 2;
                }
                let mut n: i64;
                if win_lines_concealed(wp) {
                    n = 0;
                    let mut lnum = (*wp).w_cursor.lnum;
                    while i64::from(lnum) < i64::from((*wp).w_topline) + *so_ptr {
                        if lnum >= (*(*wp).w_buffer).b_ml.ml_line_count {
                            break;
                        }
                        n += i64::from(!decor_conceal_line(wp, lnum, false));
                        if n >= i64::from(halfheight) {
                            break;
                        }
                        hasFolding(wp, lnum, ptr::null_mut(), &raw mut lnum);
                        lnum += 1;
                    }
                } else {
                    n = i64::from((*wp).w_topline) + *so_ptr - i64::from((*wp).w_cursor.lnum);
                }

                let min_scroll = scrolljump_value(wp);
                if eof_pressure {
                    scroll_cursor_halfway(wp, true, true);
                } else if n >= i64::from(halfheight) && min_scroll < halfheight {
                    scroll_cursor_halfway(wp, false, false);
                } else {
                    scroll_cursor_top(wp, min_scroll, 0);
                    check_botline = true;
                }
            } else {
                hasFolding(
                    wp,
                    (*wp).w_topline,
                    &raw mut (*wp).w_topline,
                    ptr::null_mut(),
                );
                check_botline = true;
            }
        }

        if check_botline {
            if ((*wp).w_valid & VALID_BOTLINE_AP) == 0 {
                validate_botline_win(wp);
            }

            if (*wp).w_botline <= (*(*wp).w_buffer).b_ml.ml_line_count || use_scrolloffpad(wp) {
                if (*wp).w_cursor.lnum < (*wp).w_botline {
                    if i64::from((*wp).w_cursor.lnum) >= i64::from((*wp).w_botline) - *so_ptr
                        || win_lines_concealed(wp)
                    {
                        let mut loff = lineoff_T {
                            lnum: (*wp).w_cursor.lnum,
                            fill: 0,
                            height: 0,
                        };
                        let mut n = if eof_pressure { 0 } else { (*wp).w_empty_rows };
                        hasFolding(wp, loff.lnum, ptr::null_mut(), &raw mut loff.lnum);
                        n += (*wp).w_filler_rows;
                        while loff.lnum < (*wp).w_botline
                            && (loff.lnum + 1 < (*wp).w_botline || loff.fill == 0)
                        {
                            n += loff.height;
                            if i64::from(n) >= *so_ptr {
                                break;
                            }
                            botline_forw(wp, &raw mut loff);
                        }
                        if i64::from(n) >= *so_ptr && !eof_pressure {
                            check_botline = false;
                        }
                    } else {
                        check_botline = false;
                    }
                }
                if check_botline {
                    let mut n: i64 = 0;
                    if win_lines_concealed(wp) {
                        let mut lnum = (*wp).w_cursor.lnum;
                        while i64::from(lnum) >= i64::from((*wp).w_botline) - *so_ptr {
                            if lnum <= 0 {
                                break;
                            }
                            n += i64::from(!decor_conceal_line(wp, lnum - 1, false));
                            if n > i64::from((*wp).w_view_height + 1) {
                                break;
                            }
                            hasFolding(wp, lnum, &raw mut lnum, ptr::null_mut());
                            lnum -= 1;
                        }
                    } else {
                        n = i64::from((*wp).w_cursor.lnum) - i64::from((*wp).w_botline)
                            + 1
                            + *so_ptr;
                    }
                    if n <= i64::from((*wp).w_view_height + 1) {
                        if eof_pressure {
                            scroll_cursor_halfway(wp, true, true);
                        } else {
                            scroll_cursor_bot(wp, scrolljump_value(wp), false);
                        }
                    } else {
                        scroll_cursor_halfway(wp, eof_pressure, eof_pressure);
                    }
                }
            }
        }
        (*wp).w_valid |= VALID_TOPLINE;
        (*wp).w_viewport_invalid = true;
        win_check_anchored_floats(wp);

        if (*wp).w_topline != old_topline || (*wp).w_topfill != old_topfill {
            dollar_vcol = -1;
            redraw_later(wp, UPD_VALID);

            if (*wp).w_onebuf_opt.wo_sms == 0 {
                reset_skipcol(wp);
            } else if (*wp).w_skipcol != 0 {
                redraw_later(wp, UPD_SOME_VALID);
            }

            if (*wp).w_cursor.lnum == (*wp).w_topline {
                validate_cursor(wp);
            }
        }

        *so_ptr = save_so;
    }
}

/// Return the scrolljump value to use for the window `wp`.
unsafe fn scrolljump_value(wp: *mut win_T) -> c_int {
    unsafe {
        if p_sj >= 0 {
            p_sj as c_int
        } else {
            ((*wp).w_view_height * (-p_sj as c_int)) / 100
        }
    }
}

/// Return true when there are not `'scrolloff'` lines above the cursor for window `wp`.
unsafe fn check_top_offset(wp: *mut win_T) -> bool {
    unsafe {
        let so = get_scrolloff_value(wp);
        if i64::from((*wp).w_cursor.lnum) < i64::from((*wp).w_topline) + so
            || win_lines_concealed(wp)
        {
            let mut loff = lineoff_T {
                lnum: (*wp).w_cursor.lnum,
                fill: 0,
                height: 0,
            };
            let mut n = i64::from((*wp).w_topfill);
            while n < so {
                topline_back(wp, &raw mut loff);
                if loff.lnum < (*wp).w_topline || (loff.lnum == (*wp).w_topline && loff.fill > 0) {
                    break;
                }
                n += i64::from(loff.height);
            }
            if n < so {
                return true;
            }
        }
        false
    }
}

/// Update `w_curswant`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_curswant_force() {
    unsafe {
        validate_virtcol(curwin);
        (*curwin).w_curswant = (*curwin).w_virtcol;
        (*curwin).w_set_curswant = false;
    }
}

/// Update `w_curswant` if `w_set_curswant` is set.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_curswant() {
    unsafe {
        if (*curwin).w_set_curswant {
            update_curswant_force();
        }
    }
}

/// Check if the cursor has moved. Set the `w_valid` flag accordingly.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_cursor_moved(wp: *mut win_T) {
    unsafe {
        if (*wp).w_cursor.lnum != (*wp).w_valid_cursor.lnum {
            (*wp).w_valid &= !(VALID_WROW
                | VALID_WCOL
                | VALID_VIRTCOL
                | VALID_CHEIGHT
                | VALID_CROW
                | VALID_TOPLINE);

            if wp == curwin
                && (*wp).w_valid_cursor.lnum > 0
                && (*wp).w_onebuf_opt.wo_cole >= 2
                && !conceal_cursor_line(wp)
                && (decor_conceal_line(wp, (*wp).w_cursor.lnum - 1, true)
                    || decor_conceal_line(wp, (*wp).w_valid_cursor.lnum - 1, true))
            {
                changed_window_setting(wp);
            }
            (*wp).w_valid_cursor = (*wp).w_cursor;
            (*wp).w_valid_leftcol = (*wp).w_leftcol;
            (*wp).w_valid_skipcol = (*wp).w_skipcol;
            (*wp).w_viewport_invalid = true;
        } else if (*wp).w_skipcol != (*wp).w_valid_skipcol {
            (*wp).w_valid &= !(VALID_WROW
                | VALID_WCOL
                | VALID_VIRTCOL
                | VALID_CHEIGHT
                | VALID_CROW
                | VALID_BOTLINE
                | VALID_BOTLINE_AP);
            (*wp).w_valid_cursor = (*wp).w_cursor;
            (*wp).w_valid_leftcol = (*wp).w_leftcol;
            (*wp).w_valid_skipcol = (*wp).w_skipcol;
        } else if (*wp).w_cursor.col != (*wp).w_valid_cursor.col
            || (*wp).w_leftcol != (*wp).w_valid_leftcol
            || (*wp).w_cursor.coladd != (*wp).w_valid_cursor.coladd
        {
            (*wp).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
            (*wp).w_valid_cursor.col = (*wp).w_cursor.col;
            (*wp).w_valid_leftcol = (*wp).w_leftcol;
            (*wp).w_valid_cursor.coladd = (*wp).w_cursor.coladd;
            (*wp).w_viewport_invalid = true;
        }
    }
}

/// Call this function when some window settings have changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn changed_window_setting(wp: *mut win_T) {
    unsafe {
        (*wp).w_lines_valid = 0;
        changed_line_abv_curs_win(wp);
        (*wp).w_valid &= !(VALID_BOTLINE | VALID_BOTLINE_AP | VALID_TOPLINE);
        redraw_later(wp, UPD_NOT_VALID);
    }
}

/// Call `changed_window_setting()` for every window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn changed_window_setting_all() {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                changed_window_setting(wp);
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// Set `wp->w_topline` to a certain number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_topline(wp: *mut win_T, mut lnum: linenr_T) {
    unsafe {
        let prev_topline = (*wp).w_topline;

        hasFolding(wp, lnum, &raw mut lnum, ptr::null_mut());
        (*wp).w_botline += lnum - (*wp).w_topline;
        if (*wp).w_botline > (*(*wp).w_buffer).b_ml.ml_line_count + 1 {
            (*wp).w_botline = (*(*wp).w_buffer).b_ml.ml_line_count + 1;
        }
        (*wp).w_topline = lnum;
        (*wp).w_topline_was_set = true;
        if lnum != prev_topline {
            (*wp).w_topfill = 0;
        }
        (*wp).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_TOPLINE);
        redraw_later(wp, UPD_VALID);
    }
}

/// Saves the view of `wp`, for `restore_viewstate()`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn save_viewstate(wp: *mut win_T, vs: *mut viewstate_T) {
    unsafe {
        (*vs).vs_curswant = (*wp).w_curswant;
        (*vs).vs_set_curswant = (*wp).w_set_curswant;
        (*vs).vs_leftcol = (*wp).w_leftcol;
        (*vs).vs_skipcol = (*wp).w_skipcol;
        (*vs).vs_topline = (*wp).w_topline;
        (*vs).vs_topfill = (*wp).w_topfill;
        (*vs).vs_botline = (*wp).w_botline;
        (*vs).vs_empty_rows = (*wp).w_empty_rows;
    }
}

/// Restores the view saved by `save_viewstate()`. No validation.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn restore_viewstate(wp: *mut win_T, vs: *const viewstate_T) {
    unsafe {
        (*wp).w_curswant = (*vs).vs_curswant;
        (*wp).w_set_curswant = (*vs).vs_set_curswant;
        (*wp).w_leftcol = (*vs).vs_leftcol;
        (*wp).w_skipcol = (*vs).vs_skipcol;
        (*wp).w_topline = (*vs).vs_topline;
        (*wp).w_topfill = (*vs).vs_topfill;
        (*wp).w_botline = (*vs).vs_botline;
        (*wp).w_empty_rows = (*vs).vs_empty_rows;
    }
}

/// Call this function when the length of the cursor line (in screen
/// characters) has changed, and the change is before the cursor.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn changed_cline_bef_curs(wp: *mut win_T) {
    unsafe {
        (*wp).w_valid &=
            !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL | VALID_CROW | VALID_CHEIGHT | VALID_TOPLINE);
    }
}

/// Call this function when the length of a line above the cursor has changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn changed_line_abv_curs() {
    unsafe {
        (*curwin).w_valid &=
            !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL | VALID_CROW | VALID_CHEIGHT | VALID_TOPLINE);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn changed_line_abv_curs_win(wp: *mut win_T) {
    unsafe {
        (*wp).w_valid &=
            !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL | VALID_CROW | VALID_CHEIGHT | VALID_TOPLINE);
    }
}

/// Make sure the value of `wp->w_botline` is valid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn validate_botline_win(wp: *mut win_T) {
    unsafe {
        if ((*wp).w_valid & VALID_BOTLINE) == 0 {
            comp_botline(wp);
        }
    }
}

/// Mark `wp->w_botline` as invalid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn invalidate_botline_win(wp: *mut win_T) {
    unsafe {
        (*wp).w_valid &= !(VALID_BOTLINE | VALID_BOTLINE_AP);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn approximate_botline_win(wp: *mut win_T) {
    unsafe {
        (*wp).w_valid &= !VALID_BOTLINE;
    }
}

/// Return true if `wp->w_wrow` and `wp->w_wcol` are valid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_valid(wp: *mut win_T) -> c_int {
    unsafe {
        check_cursor_moved(wp);
        c_int::from(((*wp).w_valid & (VALID_WROW | VALID_WCOL)) == (VALID_WROW | VALID_WCOL))
    }
}

/// Validate cursor position. Makes sure `w_wrow` and `w_wcol` are valid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn validate_cursor(wp: *mut win_T) {
    unsafe {
        check_cursor_lnum(wp);
        check_cursor_moved(wp);
        if ((*wp).w_valid & (VALID_WCOL | VALID_WROW)) != (VALID_WCOL | VALID_WROW) {
            curs_columns(wp, c_int::from(true));
        }
    }
}

/// Compute `wp->w_cline_row` and `wp->w_cline_height`, based on `wp->w_topline`.
unsafe fn curs_rows(wp: *mut win_T) {
    unsafe {
        let w_lines = (*wp).w_lines.cast::<wline_T>();
        let all_invalid =
            !redrawing() || (*wp).w_lines_valid == 0 || (*w_lines).wl_lnum > (*wp).w_topline;
        let mut i: c_int = 0;
        (*wp).w_cline_row = 0;
        let mut lnum = (*wp).w_topline;
        while lnum < (*wp).w_cursor.lnum {
            let mut valid = false;
            if !all_invalid && i < (*wp).w_lines_valid {
                let wl = w_lines.add(i as usize);
                if (*wl).wl_lnum < lnum || !(*wl).wl_valid {
                    i += 1;
                    continue;
                }
                if (*wl).wl_lnum == lnum {
                    if !(*(*wp).w_buffer).b_mod_set
                        || (*wl).wl_lastlnum < (*wp).w_cursor.lnum
                        || (*(*wp).w_buffer).b_mod_top > (*wl).wl_lastlnum + 1
                    {
                        valid = true;
                    }
                } else if (*wl).wl_lnum > lnum {
                    i -= 1;
                }
            }
            if valid && (lnum != (*wp).w_topline || ((*wp).w_skipcol == 0 && !win_may_fill(wp))) {
                let wl = w_lines.add(i as usize);
                lnum = (*wl).wl_lastlnum + 1;
                if lnum > (*wp).w_cursor.lnum {
                    i += 1;
                    break;
                }
                (*wp).w_cline_row += c_int::from((*wl).wl_size);
            } else {
                let mut last = lnum;
                let mut folded = false;
                let n = plines_correct_topline(wp, lnum, &raw mut last, true, &raw mut folded);
                lnum = last + 1;
                if lnum + c_int::from(decor_conceal_line(wp, lnum - 1, false)) > (*wp).w_cursor.lnum
                {
                    i += 1;
                    break;
                }
                (*wp).w_cline_row += n;
            }
            i += 1;
        }

        check_cursor_moved(wp);
        if ((*wp).w_valid & VALID_CHEIGHT) == 0 {
            if all_invalid
                || i == (*wp).w_lines_valid
                || (i < (*wp).w_lines_valid
                    && (!(*w_lines.add(i as usize)).wl_valid
                        || (*w_lines.add(i as usize)).wl_lnum != (*wp).w_cursor.lnum))
            {
                (*wp).w_cline_height = plines_win_full(
                    wp,
                    (*wp).w_cursor.lnum,
                    ptr::null_mut(),
                    &raw mut (*wp).w_cline_folded,
                    true,
                    true,
                );
            } else if i > (*wp).w_lines_valid {
                (*wp).w_cline_height = 0;
                (*wp).w_cline_folded =
                    hasFolding(wp, (*wp).w_cursor.lnum, ptr::null_mut(), ptr::null_mut());
            } else {
                (*wp).w_cline_height = c_int::from((*w_lines.add(i as usize)).wl_size);
                (*wp).w_cline_folded = (*w_lines.add(i as usize)).wl_folded;
            }
        }

        redraw_for_cursorline(wp);
        (*wp).w_valid |= VALID_CROW | VALID_CHEIGHT;
    }
}

/// Validate `wp->w_virtcol` only.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn validate_virtcol(wp: *mut win_T) {
    unsafe {
        check_cursor_moved(wp);

        if ((*wp).w_valid & VALID_VIRTCOL) != 0 {
            return;
        }

        getvvcol(
            wp,
            &raw mut (*wp).w_cursor,
            ptr::null_mut(),
            &raw mut (*wp).w_virtcol,
            ptr::null_mut(),
            0,
        );
        redraw_for_cursorcolumn(wp);
        (*wp).w_valid |= VALID_VIRTCOL;
    }
}

/// Validate `wp->w_cline_height` only.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn validate_cheight(wp: *mut win_T) {
    unsafe {
        check_cursor_moved(wp);

        if ((*wp).w_valid & VALID_CHEIGHT) != 0 {
            return;
        }

        (*wp).w_cline_height = plines_win_full(
            wp,
            (*wp).w_cursor.lnum,
            ptr::null_mut(),
            &raw mut (*wp).w_cline_folded,
            true,
            true,
        );
        (*wp).w_valid |= VALID_CHEIGHT;
    }
}

/// Validate `w_wcol` and `w_virtcol` only.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn validate_cursor_col(wp: *mut win_T) {
    unsafe {
        validate_virtcol(wp);

        if ((*wp).w_valid & VALID_WCOL) != 0 {
            return;
        }

        let mut col = (*wp).w_virtcol;
        let off = win_col_off(wp);
        col += off;
        let width = (*wp).w_view_width - off + win_col_off2(wp);

        if (*wp).w_onebuf_opt.wo_wrap != 0 && col >= (*wp).w_view_width && width > 0 {
            col -= ((col - (*wp).w_view_width) / width + 1) * width;
        }
        if col > (*wp).w_leftcol {
            col -= (*wp).w_leftcol;
        } else {
            col = 0;
        }
        (*wp).w_wcol = col;

        (*wp).w_valid |= VALID_WCOL;
    }
}

/// Compute offset of a window, occupied by absolute or relative line number,
/// fold column and sign column.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_col_off(wp: *mut win_T) -> c_int {
    unsafe {
        let num_off = if (*wp).w_onebuf_opt.wo_nu != 0
            || (*wp).w_onebuf_opt.wo_rnu != 0
            || *(*wp).w_onebuf_opt.wo_stc != NUL
        {
            number_width(wp) + c_int::from(*(*wp).w_onebuf_opt.wo_stc == NUL)
        } else {
            0
        };
        num_off + win_fdccol_count(wp) + ((*wp).w_scwidth * SIGN_WIDTH)
    }
}

/// Return the difference in column offset for the second screen line of a
/// wrapped line.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_col_off2(wp: *mut win_T) -> c_int {
    unsafe {
        if ((*wp).w_onebuf_opt.wo_nu != 0
            || (*wp).w_onebuf_opt.wo_rnu != 0
            || *(*wp).w_onebuf_opt.wo_stc != NUL)
            && !vim_strchr(p_cpo, kCpoNumcol).is_null()
        {
            return number_width(wp) + c_int::from(*(*wp).w_onebuf_opt.wo_stc == NUL);
        }
        0
    }
}

/// Compute `wp->w_wcol` and `wp->w_virtcol`, and update `wp->w_wrow`,
/// `wp->w_cline_row`, and `wp->w_leftcol`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn curs_columns(wp: *mut win_T, may_scroll: c_int) {
    unsafe {
        let mut startcol: colnr_T = 0;
        let mut endcol: colnr_T = 0;

        update_topline(wp);

        if ((*wp).w_valid & VALID_CROW) == 0 {
            curs_rows(wp);
        }

        if (*wp).w_cline_folded {
            startcol = (*wp).w_leftcol;
            (*wp).w_virtcol = startcol;
            endcol = startcol;
        } else {
            getvvcol(
                wp,
                &raw mut (*wp).w_cursor,
                &raw mut startcol,
                &raw mut (*wp).w_virtcol,
                &raw mut endcol,
                0,
            );
        }

        if startcol > dollar_vcol {
            dollar_vcol = -1;
        }

        let mut extra = win_col_off(wp);
        (*wp).w_wcol = (*wp).w_virtcol + extra;
        endcol += extra;

        (*wp).w_wrow = (*wp).w_cline_row;

        let width1 = (*wp).w_view_width - extra;
        let mut width2: c_int = 0;
        let mut did_sub_skipcol = false;
        if width1 <= 0 {
            (*wp).w_wcol = (*wp).w_view_width - 1;
            if (*wp).w_onebuf_opt.wo_wrap != 0 {
                (*wp).w_wrow = (*wp).w_view_height - 1;
            } else {
                (*wp).w_wrow = (*wp).w_view_height - 1 - (*wp).w_empty_rows;
            }
        } else if (*wp).w_onebuf_opt.wo_wrap != 0 && (*wp).w_view_width != 0 {
            width2 = width1 + win_col_off2(wp);

            if (*wp).w_cursor.lnum == (*wp).w_topline
                && (*wp).w_skipcol > 0
                && (*wp).w_wcol >= (*wp).w_skipcol
            {
                if (*wp).w_skipcol <= width1 {
                    (*wp).w_wcol -= width2;
                } else {
                    (*wp).w_wcol -= width2 * ((((*wp).w_skipcol - width1) / width2) + 1);
                }

                did_sub_skipcol = true;
            }

            if (*wp).w_wcol >= (*wp).w_view_width {
                let n = ((*wp).w_wcol - (*wp).w_view_width) / width2 + 1;
                (*wp).w_wcol -= n * width2;
                (*wp).w_wrow += n;
            }
        } else if may_scroll != 0 && !(*wp).w_cline_folded {
            let siso = get_sidescrolloff_value(wp);
            let off_left = i64::from(startcol) - i64::from((*wp).w_leftcol) - siso;
            let off_right = i64::from(endcol)
                - i64::from((*wp).w_leftcol)
                - (i64::from((*wp).w_view_width) - siso)
                + 1;
            if off_left < 0 || off_right > 0 {
                let mut diff = if off_left < 0 { -off_left } else { off_right };

                let mut new_leftcol =
                    if p_ss == 0 || diff >= i64::from(width1 / 2) || off_right >= off_left {
                        (*wp).w_wcol - extra - width1 / 2
                    } else {
                        if diff < p_ss {
                            diff = p_ss;
                        }
                        if off_left < 0 {
                            (*wp).w_leftcol - diff as c_int
                        } else {
                            (*wp).w_leftcol + diff as c_int
                        }
                    };
                new_leftcol = new_leftcol.max(0);
                if new_leftcol != (*wp).w_leftcol {
                    (*wp).w_leftcol = new_leftcol;
                    win_check_anchored_floats(wp);
                    redraw_later(wp, UPD_NOT_VALID);
                }
            }
            (*wp).w_wcol -= (*wp).w_leftcol;
        } else if (*wp).w_wcol > (*wp).w_leftcol {
            (*wp).w_wcol -= (*wp).w_leftcol;
        } else {
            (*wp).w_wcol = 0;
        }

        if (*wp).w_cursor.lnum == (*wp).w_topline {
            (*wp).w_wrow += (*wp).w_topfill;
        } else {
            (*wp).w_wrow += win_get_fill(wp, (*wp).w_cursor.lnum);
        }

        let mut plines: c_int = 0;
        let so = get_scrolloff_value(wp);
        let prev_skipcol = (*wp).w_skipcol;
        if ((*wp).w_wrow >= (*wp).w_view_height
            || ((prev_skipcol > 0
                || i64::from((*wp).w_wrow) + so >= i64::from((*wp).w_view_height))
                && {
                    plines = plines_win_nofill(wp, (*wp).w_cursor.lnum, false);
                    plines > (*wp).w_view_height
                }))
            && (*wp).w_view_height != 0
            && (*wp).w_cursor.lnum == (*wp).w_topline
            && width2 > 0
            && (*wp).w_view_width != 0
        {
            extra = 0;
            if i64::from((*wp).w_skipcol) + so * i64::from(width2) > i64::from((*wp).w_virtcol) {
                extra = 1;
            }
            if plines == 0 {
                plines = plines_win(wp, (*wp).w_cursor.lnum, false);
            }
            plines -= 1;
            let mut n = if i64::from(plines) > i64::from((*wp).w_wrow) + so {
                ((*wp).w_wrow as i64 + so) as c_int
            } else {
                plines
            };
            if i64::from(n)
                >= i64::from((*wp).w_view_height) + i64::from((*wp).w_skipcol / width2) - so
            {
                extra += 2;
            }

            if extra == 3 || i64::from((*wp).w_view_height) <= so * 2 {
                n = (*wp).w_virtcol / width2;
                if n > (*wp).w_view_height / 2 {
                    n -= (*wp).w_view_height / 2;
                } else {
                    n = 0;
                }
                if n > plines - (*wp).w_view_height + 1 {
                    n = plines - (*wp).w_view_height + 1;
                }
                (*wp).w_skipcol = if n > 0 { width1 + (n - 1) * width2 } else { 0 };
            } else if extra == 1 {
                extra = ((i64::from((*wp).w_skipcol) + so * i64::from(width2)
                    - i64::from((*wp).w_virtcol)
                    + i64::from(width2)
                    - 1)
                    / i64::from(width2)) as c_int;
                if extra > 0 {
                    if extra * width2 > (*wp).w_skipcol {
                        extra = (*wp).w_skipcol / width2;
                    }
                    (*wp).w_skipcol -= extra * width2;
                }
            } else if extra == 2 {
                endcol = (n - (*wp).w_view_height + 1) * width2;
                while endcol > (*wp).w_virtcol {
                    endcol -= width2;
                }
                (*wp).w_skipcol = (*wp).w_skipcol.max(endcol);
            }

            if did_sub_skipcol {
                (*wp).w_wrow -= ((*wp).w_skipcol - prev_skipcol) / width2;
            } else {
                (*wp).w_wrow -= (*wp).w_skipcol / width2;
            }

            if (*wp).w_wrow >= (*wp).w_view_height {
                extra = (*wp).w_wrow - (*wp).w_view_height + 1;
                (*wp).w_skipcol += extra * width2;
                (*wp).w_wrow -= extra;
            }

            extra = (prev_skipcol - (*wp).w_skipcol) / width2;
            if !(*wp).w_grid.target.is_null() {
                win_scroll_lines(wp, 0, extra);
            }
        } else if (*wp).w_onebuf_opt.wo_sms == 0 {
            (*wp).w_skipcol = 0;
        }
        if prev_skipcol != (*wp).w_skipcol {
            redraw_later(wp, UPD_SOME_VALID);
        }

        redraw_for_cursorcolumn(wp);

        (*wp).w_valid_leftcol = (*wp).w_leftcol;
        (*wp).w_valid_skipcol = (*wp).w_skipcol;

        (*wp).w_valid |= VALID_WCOL | VALID_WROW | VALID_VIRTCOL;
    }
}

/// Compute the screen position of text character at `pos` in window `wp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn textpos2screenpos(
    wp: *mut win_T,
    pos: *mut pos_T,
    rowp: *mut c_int,
    scolp: *mut c_int,
    ccolp: *mut c_int,
    ecolp: *mut c_int,
    local: bool,
) {
    unsafe {
        let mut scol: colnr_T = 0;
        let mut ccol: colnr_T = 0;
        let mut ecol: colnr_T = 0;
        let mut row: c_int;
        let mut coloff: colnr_T = 0;
        let mut visible_row = false;
        let mut is_folded = false;

        let mut lnum = (*pos).lnum;
        if lnum >= (*wp).w_topline && lnum <= (*wp).w_botline {
            is_folded = hasFolding(wp, lnum, &raw mut lnum, ptr::null_mut());
            row = plines_m_win(wp, (*wp).w_topline, lnum - 1, c_int::MAX);
            row -= adjust_plines_for_skipcol(wp);
            row += if lnum == (*wp).w_topline {
                (*wp).w_topfill
            } else {
                win_get_fill(wp, lnum)
            };
            visible_row = true;
        } else if !local || lnum < (*wp).w_topline {
            row = 0;
        } else {
            row = (*wp).w_view_height - 1;
        }

        let existing_row = lnum > 0 && lnum <= (*(*wp).w_buffer).b_ml.ml_line_count;

        if (local || visible_row) && existing_row {
            let off = win_col_off(wp);
            if is_folded {
                row += if local {
                    0
                } else {
                    (*wp).w_winrow + (*wp).w_winrow_off
                } + 1;
                coloff = if local {
                    0
                } else {
                    (*wp).w_wincol + (*wp).w_wincol_off
                } + 1
                    + off;
            } else {
                getvcol(wp, pos, &raw mut scol, &raw mut ccol, &raw mut ecol, 0);

                let mut col = scol + off;
                let width = (*wp).w_view_width - off + win_col_off2(wp);

                if (*wp).w_onebuf_opt.wo_wrap != 0 && col >= (*wp).w_view_width && width > 0 {
                    let rowoff = if visible_row {
                        (col - (*wp).w_view_width) / width + 1
                    } else {
                        0
                    };
                    col -= rowoff * width;
                    row += rowoff;
                }

                col -= (*wp).w_leftcol;

                if col >= 0 && col < (*wp).w_view_width && row >= 0 && row < (*wp).w_view_height {
                    coloff = col - scol
                        + if local {
                            0
                        } else {
                            (*wp).w_wincol + (*wp).w_wincol_off
                        }
                        + 1;
                    row += if local {
                        0
                    } else {
                        (*wp).w_winrow + (*wp).w_winrow_off
                    } + 1;
                } else {
                    scol = 0;
                    ccol = 0;
                    ecol = 0;
                    if local {
                        coloff = if col < 0 { -1 } else { (*wp).w_view_width + 1 };
                    } else {
                        row = 0;
                    }
                }
            }
        }
        *rowp = row;
        *scolp = scol + coloff;
        *ccolp = ccol + coloff;
        *ecolp = ecol + coloff;
    }
}

/// `"screenpos({winid}, {lnum}, {col})"` function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screenpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        let dict = (*rettv).vval.v_dict;

        let wp = find_win_by_nr_or_id(argvars);
        if wp.is_null() {
            return;
        }

        let mut pos = pos_T {
            lnum: tv_get_number(argvars.add(1)) as linenr_T,
            col: (tv_get_number(argvars.add(2)) as colnr_T - 1).max(0),
            coladd: 0,
        };
        if pos.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
            semsg(tr(e_invalid_line_number_nr.as_ptr()), pos.lnum);
            return;
        }
        let mut row: c_int = 0;
        let mut scol: c_int = 0;
        let mut ccol: c_int = 0;
        let mut ecol: c_int = 0;
        textpos2screenpos(
            wp,
            &raw mut pos,
            &raw mut row,
            &raw mut scol,
            &raw mut ccol,
            &raw mut ecol,
            false,
        );

        tv_dict_add_nr(dict, c"row".as_ptr(), "row".len(), varnumber_T::from(row));
        tv_dict_add_nr(dict, c"col".as_ptr(), "col".len(), varnumber_T::from(scol));
        tv_dict_add_nr(
            dict,
            c"curscol".as_ptr(),
            "curscol".len(),
            varnumber_T::from(ccol),
        );
        tv_dict_add_nr(
            dict,
            c"endcol".as_ptr(),
            "endcol".len(),
            varnumber_T::from(ecol),
        );
    }
}

/// Convert a virtual (screen) column to a character column (1-based).
unsafe fn virtcol2col(wp: *mut win_T, lnum: linenr_T, vcol: c_int) -> c_int {
    unsafe {
        let offset = vcol2col(wp, lnum, vcol - 1, ptr::null_mut());
        let line = ml_get_buf((*wp).w_buffer, lnum);
        let mut p = line.add(offset as usize);

        if *p == NUL {
            if p == line {
                return 0;
            }
            p = p.sub(utf_head_off(line, p.sub(1)) as usize + 1);
        }
        p.offset_from(line) as c_int + 1
    }
}

/// `"virtcol2col({winid}, {lnum}, {col})"` function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_virtcol2col(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;

        if tv_check_for_number_arg(argvars, 0) == FAIL
            || tv_check_for_number_arg(argvars, 1) == FAIL
            || tv_check_for_number_arg(argvars, 2) == FAIL
        {
            return;
        }

        let wp = find_win_by_nr_or_id(argvars);
        if wp.is_null() {
            return;
        }

        let mut error = false;
        let lnum = tv_get_number_chk(argvars.add(1), &raw mut error) as linenr_T;
        if error || lnum < 0 || lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
            return;
        }

        let screencol = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
        if error || screencol < 0 {
            return;
        }

        (*rettv).vval.v_number = varnumber_T::from(virtcol2col(wp, lnum, screencol));
    }
}

/// Make sure the cursor is in the visible part of the topline after scrolling
/// the screen with `'smoothscroll'`.
unsafe fn cursor_correct_sms(wp: *mut win_T) {
    unsafe {
        if (*wp).w_onebuf_opt.wo_sms == 0
            || (*wp).w_onebuf_opt.wo_wrap == 0
            || (*wp).w_cursor.lnum != (*wp).w_topline
        {
            return;
        }

        let so = get_scrolloff_value(wp);
        let width1 = (*wp).w_view_width - win_col_off(wp);
        let width2 = width1 + win_col_off2(wp);
        let mut so_cols = if so == 0 {
            0
        } else {
            i64::from(width1) + (so - 1) * i64::from(width2)
        };
        let space_cols = ((*wp).w_view_height - 1) * width2;
        let size = if so == 0 {
            0
        } else {
            linetabsize_eol(wp, (*wp).w_topline)
        };

        if (*wp).w_topline == 1 && (*wp).w_skipcol == 0 {
            so_cols = 0;
        } else if so_cols > i64::from(space_cols / 2) {
            so_cols = i64::from(space_cols / 2);
        }

        while so_cols > i64::from(size)
            && so_cols - i64::from(width2) >= i64::from(width1)
            && width1 > 0
        {
            so_cols -= i64::from(width2);
        }
        if so_cols >= i64::from(width1) && so_cols > i64::from(size) {
            so_cols -= i64::from(width1);
        }

        let overlap = if (*wp).w_skipcol == 0 {
            0
        } else {
            sms_marker_overlap(wp, (*wp).w_view_width - width2)
        };
        let top = i64::from((*wp).w_skipcol)
            + if so_cols != 0 {
                so_cols
            } else {
                i64::from(overlap)
            };
        let bot = i64::from((*wp).w_skipcol)
            + i64::from(width1)
            + i64::from(((*wp).w_view_height - 1) * width2)
            - so_cols;

        validate_virtcol(wp);
        let mut col = (*wp).w_virtcol;

        if i64::from(col) < top {
            if col < width1 {
                col += width1;
            }
            while width2 > 0 && i64::from(col) < top {
                col += width2;
            }
        } else {
            while width2 > 0 && i64::from(col) >= bot {
                col -= width2;
            }
        }

        if col != (*wp).w_virtcol {
            (*wp).w_curswant = col;
            let rc = coladvance(wp, (*wp).w_curswant);
            (*wp).w_valid &=
                !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL);
            if rc == FAIL
                && (*wp).w_skipcol > 0
                && (*wp).w_cursor.lnum < (*(*wp).w_buffer).b_ml.ml_line_count
            {
                validate_virtcol(wp);
                if (*wp).w_virtcol < (*wp).w_skipcol + overlap {
                    (*wp).w_cursor.lnum += 1;
                    (*wp).w_cursor.col = 0;
                    (*wp).w_cursor.coladd = 0;
                    (*wp).w_curswant = 0;
                    (*wp).w_valid &= !VALID_VIRTCOL;
                }
            }
        }
    }
}

/// Scroll `count` lines up or down, and redraw.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scroll_redraw(up: c_int, count: linenr_T) {
    unsafe {
        let prev_topline = (*curwin).w_topline;
        let prev_skipcol = (*curwin).w_skipcol;
        let prev_topfill = (*curwin).w_topfill;
        let prev_lnum = (*curwin).w_cursor.lnum;

        let moved = if up != 0 {
            scrollup(curwin, count, true)
        } else {
            scrolldown(curwin, count, c_int::from(true))
        };

        if get_scrolloff_value(curwin) > 0 {
            cursor_correct(curwin);
            check_cursor_moved(curwin);
            (*curwin).w_valid |= VALID_TOPLINE;

            while (*curwin).w_topline == prev_topline
                && (*curwin).w_skipcol == prev_skipcol
                && (*curwin).w_topfill == prev_topfill
            {
                if up != 0 {
                    if (*curwin).w_cursor.lnum > prev_lnum || cursor_down(1, false) == FAIL {
                        break;
                    }
                } else if (*curwin).w_cursor.lnum < prev_lnum
                    || prev_topline == 1
                    || cursor_up(1, false) == FAIL
                {
                    break;
                }
                check_cursor_moved(curwin);
                (*curwin).w_valid |= VALID_TOPLINE;
            }
        }

        if moved {
            (*curwin).w_viewport_invalid = true;
        }

        cursor_correct_sms(curwin);
        if (*curwin).w_cursor.lnum != prev_lnum {
            coladvance(curwin, (*curwin).w_curswant);
        }
        redraw_later(curwin, UPD_VALID);
    }
}

/// Scroll a window down by `line_count` logical lines (`CTRL-Y`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scrolldown(wp: *mut win_T, line_count: linenr_T, byfold: c_int) -> bool {
    unsafe {
        let mut done: c_int = 0;
        let mut width1: c_int = 0;
        let mut width2: c_int = 0;
        let do_sms = (*wp).w_onebuf_opt.wo_wrap != 0 && (*wp).w_onebuf_opt.wo_sms != 0;

        if do_sms {
            width1 = (*wp).w_view_width - win_col_off(wp);
            width2 = width1 + win_col_off2(wp);
        }

        hasFolding(
            wp,
            (*wp).w_topline,
            &raw mut (*wp).w_topline,
            ptr::null_mut(),
        );
        validate_cursor(wp);
        let mut todo = line_count;
        while todo > 0 {
            let can_fill = (*wp).w_topfill < (*wp).w_view_height - 1
                && (*wp).w_topfill < win_get_fill(wp, (*wp).w_topline);
            if (*wp).w_topline == 1 && !can_fill && (!do_sms || (*wp).w_skipcol < width1) {
                break;
            }
            if do_sms && (*wp).w_skipcol >= width1 {
                if (*wp).w_skipcol >= width1 + width2 {
                    (*wp).w_skipcol -= width2;
                } else {
                    (*wp).w_skipcol -= width1;
                }
                redraw_later(wp, UPD_NOT_VALID);
                done += 1;
            } else if can_fill {
                (*wp).w_topfill += 1;
                done += 1;
            } else {
                (*wp).w_topline -= 1;
                (*wp).w_skipcol = 0;
                (*wp).w_topfill = 0;
                let mut first: linenr_T = 0;
                if hasFolding(wp, (*wp).w_topline, &raw mut first, ptr::null_mut()) {
                    done += c_int::from(!decor_conceal_line(wp, first - 1, false));
                    if byfold == 0 {
                        todo -= (*wp).w_topline - first - 1;
                    }
                    (*wp).w_botline -= (*wp).w_topline - first;
                    (*wp).w_topline = first;
                } else if decor_conceal_line(wp, (*wp).w_topline - 1, false) {
                    todo += 1;
                } else if do_sms {
                    let mut size = linetabsize_eol(wp, (*wp).w_topline);
                    if size > width1 {
                        (*wp).w_skipcol = width1;
                        size -= width1;
                        redraw_later(wp, UPD_NOT_VALID);
                    }
                    while size > width2 {
                        (*wp).w_skipcol += width2;
                        size -= width2;
                    }
                    done += 1;
                } else {
                    done += plines_win_nofill(wp, (*wp).w_topline, true);
                }
            }
            (*wp).w_botline -= 1;
            invalidate_botline_win(wp);
            todo -= 1;
        }

        while (*wp).w_topline > 1 && decor_conceal_line(wp, (*wp).w_topline - 2, false) {
            (*wp).w_topline -= 1;
            hasFolding(
                wp,
                (*wp).w_topline,
                &raw mut (*wp).w_topline,
                ptr::null_mut(),
            );
        }

        (*wp).w_wrow += done;
        (*wp).w_cline_row += done;

        if (*wp).w_cursor.lnum == (*wp).w_topline {
            (*wp).w_cline_row = 0;
        }
        check_topfill(wp, true);

        let mut wrow = (*wp).w_wrow;
        if (*wp).w_onebuf_opt.wo_wrap != 0 && (*wp).w_view_width != 0 {
            validate_virtcol(wp);
            validate_cheight(wp);
            wrow += (*wp).w_cline_height - 1 - (*wp).w_virtcol / (*wp).w_view_width;
        }
        let mut moved = false;
        while wrow >= (*wp).w_view_height && (*wp).w_cursor.lnum > 1 {
            let mut first: linenr_T = 0;
            if hasFolding(wp, (*wp).w_cursor.lnum, &raw mut first, ptr::null_mut()) {
                wrow -= c_int::from(!decor_conceal_line(wp, (*wp).w_cursor.lnum - 1, false));
                (*wp).w_cursor.lnum = (first - 1).max(1);
            } else {
                wrow -= plines_win(wp, (*wp).w_cursor.lnum, true);
                (*wp).w_cursor.lnum -= 1;
            }
            (*wp).w_valid &=
                !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL);
            moved = true;
        }
        if moved {
            foldAdjustCursor(wp);
            coladvance(wp, (*wp).w_curswant);
        }
        (*wp).w_cursor.lnum = (*wp).w_cursor.lnum.max((*wp).w_topline);

        moved
    }
}

/// Scroll a window up by `line_count` logical lines (`CTRL-E`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scrollup(wp: *mut win_T, line_count: linenr_T, byfold: bool) -> bool {
    unsafe {
        let topline = (*wp).w_topline;
        let botline = (*wp).w_botline;
        let do_sms = (*wp).w_onebuf_opt.wo_wrap != 0 && (*wp).w_onebuf_opt.wo_sms != 0;

        if do_sms || (byfold && win_lines_concealed(wp)) || win_may_fill(wp) {
            let width1 = (*wp).w_view_width - win_col_off(wp);
            let width2 = width1 + win_col_off2(wp);
            let mut size = if do_sms {
                linetabsize_eol(wp, (*wp).w_topline)
            } else {
                0
            };
            let prev_skipcol = (*wp).w_skipcol;

            let mut todo = line_count;
            while todo > 0 {
                todo += c_int::from(decor_conceal_line(wp, (*wp).w_topline - 1, false));
                if (*wp).w_topfill > 0 {
                    (*wp).w_topfill -= 1;
                } else {
                    let mut lnum = (*wp).w_topline;
                    if byfold {
                        hasFolding(wp, lnum, ptr::null_mut(), &raw mut lnum);
                    }
                    if lnum == (*wp).w_topline && do_sms {
                        let add = if (*wp).w_skipcol > 0 { width2 } else { width1 };
                        (*wp).w_skipcol += add;
                        if (*wp).w_skipcol >= size {
                            if lnum == (*(*wp).w_buffer).b_ml.ml_line_count {
                                (*wp).w_skipcol -= add;
                                break;
                            }
                            lnum += 1;
                        }
                    } else {
                        if lnum >= (*(*wp).w_buffer).b_ml.ml_line_count {
                            break;
                        }
                        lnum += 1;
                    }

                    if lnum > (*wp).w_topline {
                        (*wp).w_botline += lnum - (*wp).w_topline;
                        (*wp).w_topline = lnum;
                        (*wp).w_topfill = win_get_fill(wp, lnum);
                        (*wp).w_skipcol = 0;
                        if todo > 1 && do_sms {
                            size = linetabsize_eol(wp, (*wp).w_topline);
                        }
                    }
                }
                todo -= 1;
            }

            if prev_skipcol > 0 || (*wp).w_skipcol > 0 {
                redraw_later(wp, UPD_NOT_VALID);
            }
        } else {
            (*wp).w_topline += line_count;
            (*wp).w_botline += line_count;
        }

        (*wp).w_topline = (*wp).w_topline.min((*(*wp).w_buffer).b_ml.ml_line_count);
        (*wp).w_botline = (*wp)
            .w_botline
            .min((*(*wp).w_buffer).b_ml.ml_line_count + 1);

        check_topfill(wp, false);

        hasFolding(
            wp,
            (*wp).w_topline,
            &raw mut (*wp).w_topline,
            ptr::null_mut(),
        );

        (*wp).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE);
        if (*wp).w_cursor.lnum < (*wp).w_topline {
            (*wp).w_cursor.lnum = (*wp).w_topline;
            (*wp).w_valid &=
                !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL);
            coladvance(wp, (*wp).w_curswant);
        }

        topline != (*wp).w_topline || botline != (*wp).w_botline
    }
}

/// Make sure that `curwin->w_skipcol` is valid for `'smoothscroll'`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn adjust_skipcol() {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_wrap == 0
            || (*curwin).w_onebuf_opt.wo_sms == 0
            || (*curwin).w_cursor.lnum != (*curwin).w_topline
        {
            return;
        }

        let width1 = (*curwin).w_view_width - win_col_off(curwin);
        if width1 <= 0 {
            return;
        }
        let width2 = width1 + win_col_off2(curwin);
        let so = get_scrolloff_value(curwin);
        let scrolloff_cols = if so == 0 {
            0
        } else {
            i64::from(width1) + (so - 1) * i64::from(width2)
        };
        let mut scrolled = false;

        validate_cheight(curwin);
        if (*curwin).w_cline_height == (*curwin).w_view_height
            && plines_win(curwin, (*curwin).w_cursor.lnum, false) <= (*curwin).w_view_height
        {
            reset_skipcol(curwin);
            return;
        }

        validate_virtcol(curwin);
        let overlap = sms_marker_overlap(curwin, (*curwin).w_view_width - width2);
        while (*curwin).w_skipcol > 0
            && i64::from((*curwin).w_virtcol)
                < i64::from((*curwin).w_skipcol + overlap) + scrolloff_cols
        {
            if (*curwin).w_skipcol >= width1 + width2 {
                (*curwin).w_skipcol -= width2;
            } else {
                (*curwin).w_skipcol -= width1;
            }
            scrolled = true;
        }
        if scrolled {
            validate_virtcol(curwin);
            redraw_later(curwin, UPD_NOT_VALID);
            return;
        }
        let mut row: c_int = 0;
        let mut col = i64::from((*curwin).w_virtcol) + scrolloff_cols;

        if scrolloff_cols > 0 {
            let mut size = linetabsize_eol(curwin, (*curwin).w_topline);
            size = width1 + width2 * ((size - width1 + width2 - 1) / width2);
            while col > i64::from(size) {
                col -= i64::from(width2);
            }
        }
        col -= i64::from((*curwin).w_skipcol);

        if col >= i64::from(width1) {
            col -= i64::from(width1);
            row += 1;
        }
        if col > i64::from(width2) {
            row += (col / i64::from(width2)) as c_int;
        }
        if row >= (*curwin).w_view_height {
            if (*curwin).w_skipcol == 0 {
                (*curwin).w_skipcol += width1;
                row -= 1;
            }
            if row >= (*curwin).w_view_height {
                (*curwin).w_skipcol += (row - (*curwin).w_view_height) * width2;
            }
            redraw_later(curwin, UPD_NOT_VALID);
        }
    }
}

/// Don't end up with too many filler lines in the window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_topfill(wp: *mut win_T, down: bool) {
    unsafe {
        if (*wp).w_topfill > 0 {
            let n = plines_win_nofill(wp, (*wp).w_topline, true);
            if (*wp).w_topfill + n > (*wp).w_view_height {
                if down && (*wp).w_topline > 1 {
                    (*wp).w_topline -= 1;
                    (*wp).w_topfill = 0;
                } else {
                    (*wp).w_topfill = ((*wp).w_view_height - n).max(0);
                }
            }
        }
        win_check_anchored_floats(wp);
    }
}

/// Scroll the screen one line down, without moving the cursor off the screen.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scrolldown_clamp() {
    unsafe {
        let can_fill = (*curwin).w_topfill < win_get_fill(curwin, (*curwin).w_topline);

        if (*curwin).w_topline <= 1 && !can_fill {
            return;
        }

        validate_cursor(curwin);

        let mut end_row = (*curwin).w_wrow;
        if can_fill {
            end_row += 1;
        } else {
            end_row += plines_win_nofill(curwin, (*curwin).w_topline - 1, true);
        }
        if (*curwin).w_onebuf_opt.wo_wrap != 0 && (*curwin).w_view_width != 0 {
            validate_cheight(curwin);
            validate_virtcol(curwin);
            end_row += (*curwin).w_cline_height - 1 - (*curwin).w_virtcol / (*curwin).w_view_width;
        }
        if i64::from(end_row) < i64::from((*curwin).w_view_height) - get_scrolloff_value(curwin) {
            if can_fill {
                (*curwin).w_topfill += 1;
                check_topfill(curwin, true);
            } else {
                (*curwin).w_topline -= 1;
                (*curwin).w_topfill = 0;
            }
            hasFolding(
                curwin,
                (*curwin).w_topline,
                &raw mut (*curwin).w_topline,
                ptr::null_mut(),
            );
            (*curwin).w_botline -= 1;
            (*curwin).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE);
        }
    }
}

/// Scroll the screen one line up, without moving the cursor off the screen.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scrollup_clamp() {
    unsafe {
        if (*curwin).w_topline == (*curbuf).b_ml.ml_line_count && (*curwin).w_topfill == 0 {
            return;
        }

        validate_cursor(curwin);

        let mut start_row = (*curwin).w_wrow
            - plines_win_nofill(curwin, (*curwin).w_topline, true)
            - (*curwin).w_topfill;
        if (*curwin).w_onebuf_opt.wo_wrap != 0 && (*curwin).w_view_width != 0 {
            validate_virtcol(curwin);
            start_row -= (*curwin).w_virtcol / (*curwin).w_view_width;
        }
        if i64::from(start_row) >= get_scrolloff_value(curwin) {
            if (*curwin).w_topfill > 0 {
                (*curwin).w_topfill -= 1;
            } else {
                hasFolding(
                    curwin,
                    (*curwin).w_topline,
                    ptr::null_mut(),
                    &raw mut (*curwin).w_topline,
                );
                (*curwin).w_topline += 1;
            }
            (*curwin).w_botline += 1;
            (*curwin).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE);
        }
    }
}

unsafe fn topline_back_winheight(wp: *mut win_T, lp: *mut lineoff_T, winheight: bool) {
    unsafe {
        if (*lp).fill < win_get_fill(wp, (*lp).lnum) {
            (*lp).fill += 1;
            (*lp).height = 1;
        } else {
            (*lp).lnum -= 1;
            (*lp).fill = 0;
            if (*lp).lnum < 1 {
                (*lp).height = MAXCOL;
            } else if hasFolding(wp, (*lp).lnum, &raw mut (*lp).lnum, ptr::null_mut()) {
                (*lp).height = c_int::from(!decor_conceal_line(wp, (*lp).lnum - 1, false));
            } else {
                (*lp).height = plines_win_nofill(wp, (*lp).lnum, winheight);
            }
        }
    }
}

unsafe fn topline_back(wp: *mut win_T, lp: *mut lineoff_T) {
    unsafe {
        topline_back_winheight(wp, lp, true);
    }
}

unsafe fn botline_forw(wp: *mut win_T, lp: *mut lineoff_T) {
    unsafe {
        if (*lp).fill < win_get_fill(wp, (*lp).lnum + 1) {
            (*lp).fill += 1;
            (*lp).height = 1;
        } else {
            (*lp).lnum += 1;
            (*lp).fill = 0;
            if (*lp).lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
                (*lp).height = MAXCOL;
            } else if hasFolding(wp, (*lp).lnum, ptr::null_mut(), &raw mut (*lp).lnum) {
                (*lp).height = c_int::from(!decor_conceal_line(wp, (*lp).lnum - 1, false));
            } else {
                (*lp).height = plines_win_nofill(wp, (*lp).lnum, true);
            }
        }
    }
}

/// Recompute topline to put the cursor at the top of the window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scroll_cursor_top(wp: *mut win_T, min_scroll: c_int, always: c_int) {
    unsafe {
        let old_topline = (*wp).w_topline;
        let old_skipcol = (*wp).w_skipcol;
        let old_topfill = (*wp).w_topfill;
        let off = if mouse_dragging > 0 {
            i64::from(mouse_dragging - 1)
        } else {
            get_scrolloff_value(wp)
        };

        validate_cheight(wp);
        let mut scrolled = if (*wp).w_cursor.lnum < (*wp).w_topline {
            (*wp).w_cline_height
        } else {
            0
        };
        let mut used = (*wp).w_cline_height;

        let mut top: linenr_T = 0;
        let mut bot: linenr_T = 0;
        if hasFolding(wp, (*wp).w_cursor.lnum, &raw mut top, &raw mut bot) {
            top -= 1;
            bot += 1;
        } else {
            top = (*wp).w_cursor.lnum - 1;
            bot = (*wp).w_cursor.lnum + 1;
        }
        let mut new_topline = top + 1;

        let mut extra = win_get_fill(wp, (*wp).w_cursor.lnum);

        while top > 0 {
            let i = plines_win_nofill(wp, top, true);
            hasFolding(wp, top, &raw mut top, ptr::null_mut());
            if top < (*wp).w_topline {
                scrolled += i;
            }

            if (new_topline >= (*wp).w_topline || scrolled > min_scroll) && i64::from(extra) >= off
            {
                break;
            }

            used += i;
            if i64::from(extra + i) <= off && bot < (*(*wp).w_buffer).b_ml.ml_line_count {
                used += plines_win_full(wp, bot, &raw mut bot, ptr::null_mut(), true, true);
            }
            if used > (*wp).w_view_height {
                break;
            }

            extra += i;
            new_topline = top;
            top -= 1;
            bot += 1;
        }

        if used > (*wp).w_view_height {
            scroll_cursor_halfway(wp, false, false);
        } else {
            if new_topline < (*wp).w_topline || always != 0 {
                (*wp).w_topline = new_topline;
            }
            (*wp).w_topline = (*wp).w_topline.min((*wp).w_cursor.lnum);
            (*wp).w_topfill = win_get_fill(wp, (*wp).w_topline);
            if (*wp).w_topfill > 0 && i64::from(extra) > off {
                (*wp).w_topfill -= extra - off as c_int;
                (*wp).w_topfill = (*wp).w_topfill.max(0);
            }
            check_topfill(wp, false);
            if (*wp).w_topline != old_topline {
                reset_skipcol(wp);
            } else if (*wp).w_topline == (*wp).w_cursor.lnum {
                validate_virtcol(wp);
                if (*wp).w_skipcol >= (*wp).w_virtcol {
                    let width1 = (*wp).w_view_width - win_col_off(wp);
                    let width2 = width1 + win_col_off2(wp);
                    let plines_off = if width2 > 0 && (*wp).w_virtcol >= width1 {
                        ((*wp).w_virtcol - width1) / width2 + 1
                    } else {
                        0
                    };
                    let skipcol = skipcol_from_plines(wp, plines_off);
                    if skipcol != (*wp).w_skipcol {
                        (*wp).w_skipcol = skipcol;
                        redraw_later(wp, UPD_SOME_VALID);
                    }
                }
            }
            if (*wp).w_topline != old_topline
                || (*wp).w_skipcol != old_skipcol
                || (*wp).w_topfill != old_topfill
            {
                (*wp).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
            }
            (*wp).w_valid |= VALID_TOPLINE;
            (*wp).w_viewport_invalid = true;
        }
    }
}

/// Set `w_empty_rows` and `w_filler_rows` for window `wp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_empty_rows(wp: *mut win_T, used: c_int) {
    unsafe {
        (*wp).w_filler_rows = 0;
        if used == 0 {
            (*wp).w_empty_rows = 0;
        } else {
            (*wp).w_empty_rows = (*wp).w_view_height - used;
            if (*wp).w_botline <= (*(*wp).w_buffer).b_ml.ml_line_count {
                (*wp).w_filler_rows = win_get_fill(wp, (*wp).w_botline);
                if (*wp).w_empty_rows > (*wp).w_filler_rows {
                    (*wp).w_empty_rows -= (*wp).w_filler_rows;
                } else {
                    (*wp).w_filler_rows = (*wp).w_empty_rows;
                    (*wp).w_empty_rows = 0;
                }
            }
        }
    }
}

/// Recompute topline to put the cursor at the bottom of the window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scroll_cursor_bot(wp: *mut win_T, min_scroll: c_int, set_topbot: bool) {
    unsafe {
        let mut loff = lineoff_T::default();
        let old_topline = (*wp).w_topline;
        let old_skipcol = (*wp).w_skipcol;
        let old_topfill = (*wp).w_topfill;
        let old_botline = (*wp).w_botline;
        let old_valid = (*wp).w_valid;
        let old_empty_rows = (*wp).w_empty_rows;
        let cln = (*wp).w_cursor.lnum;
        let do_sms = (*wp).w_onebuf_opt.wo_wrap != 0 && (*wp).w_onebuf_opt.wo_sms != 0;

        if set_topbot {
            let mut used: c_int = 0;
            let mut cln_last = cln;
            hasFolding(wp, cln, ptr::null_mut(), &raw mut cln_last);
            (*wp).w_botline = cln_last + 1;
            loff.lnum = cln_last + 1;
            loff.fill = 0;
            loop {
                topline_back_winheight(wp, &raw mut loff, false);
                if loff.height == MAXCOL {
                    break;
                }
                if used + loff.height > (*wp).w_view_height {
                    if do_sms && used < (*wp).w_view_height {
                        let plines_offset = used + loff.height - (*wp).w_view_height;
                        used = (*wp).w_view_height;
                        (*wp).w_topfill = loff.fill;
                        (*wp).w_topline = loff.lnum;
                        (*wp).w_skipcol = skipcol_from_plines(wp, plines_offset);
                    }
                    break;
                }
                (*wp).w_topfill = loff.fill;
                (*wp).w_topline = loff.lnum;
                used += loff.height;
            }

            set_empty_rows(wp, used);
            (*wp).w_valid |= VALID_BOTLINE | VALID_BOTLINE_AP;
            if (*wp).w_topline != old_topline
                || (*wp).w_topfill != old_topfill
                || (*wp).w_skipcol != old_skipcol
                || (*wp).w_skipcol != 0
            {
                (*wp).w_valid &= !(VALID_WROW | VALID_CROW);
                if (*wp).w_skipcol != old_skipcol {
                    redraw_later(wp, UPD_NOT_VALID);
                } else {
                    reset_skipcol(wp);
                }
            }
        } else {
            validate_botline_win(wp);
        }

        let mut used = plines_win_nofill(wp, cln, true);

        let mut scrolled: c_int = 0;
        if cln >= (*wp).w_botline {
            scrolled = used;
            if cln == (*wp).w_botline {
                scrolled -= (*wp).w_empty_rows;
            }
            if do_sms {
                let mut top_plines = plines_win_nofill(wp, (*wp).w_topline, false);
                let width1 = (*wp).w_view_width - win_col_off(wp);

                if width1 > 0 {
                    let width2 = width1 + win_col_off2(wp);
                    let skip_lines = if (*wp).w_skipcol > width1 {
                        ((*wp).w_skipcol - width1) / width2 + 1
                    } else if (*wp).w_skipcol > 0 {
                        1
                    } else {
                        0
                    };

                    top_plines -= skip_lines;
                    if top_plines > (*wp).w_view_height {
                        scrolled += top_plines - (*wp).w_view_height;
                    }
                }
            }
        }

        let mut boff = lineoff_T::default();
        if !hasFolding(
            wp,
            (*wp).w_cursor.lnum,
            &raw mut loff.lnum,
            &raw mut boff.lnum,
        ) {
            loff.lnum = cln;
            boff.lnum = cln;
        }
        loff.fill = 0;
        boff.fill = 0;
        let fill_below_window = win_get_fill(wp, (*wp).w_botline) - (*wp).w_filler_rows;

        let mut extra: c_int = 0;
        let so = get_scrolloff_value(wp);
        let eff_so = if mouse_dragging > 0 {
            i64::from(mouse_dragging - 1)
        } else {
            so
        };
        while loff.lnum > 1 {
            if (((scrolled <= 0 || scrolled >= min_scroll) && i64::from(extra) >= eff_so)
                || boff.lnum + 1 > (*(*wp).w_buffer).b_ml.ml_line_count)
                && loff.lnum <= (*wp).w_botline
                && (loff.lnum < (*wp).w_botline || loff.fill >= fill_below_window)
            {
                break;
            }

            let loff_lnum_before = loff.lnum;
            topline_back(wp, &raw mut loff);
            if loff.height == MAXCOL {
                used = MAXCOL;
            } else {
                used += loff.height;
            }
            if used > (*wp).w_view_height {
                break;
            }
            if loff.lnum >= (*wp).w_botline
                && (loff.lnum > (*wp).w_botline || loff.fill <= fill_below_window)
            {
                scrolled += loff.height;
                if loff.lnum == (*wp).w_botline && loff_lnum_before > (*curwin).w_botline {
                    scrolled -= (*wp).w_empty_rows;
                }
            }

            if boff.lnum < (*(*wp).w_buffer).b_ml.ml_line_count {
                let boff_lnum_before = boff.lnum;
                botline_forw(wp, &raw mut boff);
                used += boff.height;
                if used > (*wp).w_view_height {
                    break;
                }
                if i64::from(extra) < eff_so || scrolled < min_scroll {
                    extra += boff.height;
                    if boff.lnum >= (*wp).w_botline
                        || (boff.lnum + 1 == (*wp).w_botline && boff.fill > (*wp).w_filler_rows)
                    {
                        scrolled += boff.height;
                        if boff.lnum >= (*curwin).w_botline
                            && boff_lnum_before < (*curwin).w_botline
                        {
                            scrolled -= (*wp).w_empty_rows;
                        }
                    }
                }
            }
        }

        let mut line_count: linenr_T = if scrolled <= 0 {
            0
        } else if used > (*wp).w_view_height {
            used
        } else {
            let mut lc = 0;
            boff.fill = (*wp).w_topfill;
            boff.lnum = (*wp).w_topline - 1;
            let mut i = 0;
            while i < scrolled && boff.lnum < (*wp).w_botline {
                botline_forw(wp, &raw mut boff);
                i += boff.height;
                lc += 1;
            }
            if i < scrolled {
                lc = 9999;
            }
            lc
        };

        let eof_pressure = scrolloffpad_eof_pressure(wp, cln, so);
        if line_count >= (*wp).w_view_height && line_count > min_scroll {
            scroll_cursor_halfway(wp, eof_pressure, true);
        } else if line_count > 0 {
            if do_sms {
                scrollup(wp, scrolled, true);
            } else {
                scrollup(wp, line_count, true);
            }
        }
        let _ = &mut line_count;

        if (*wp).w_topline == old_topline && (*wp).w_skipcol == old_skipcol && set_topbot {
            (*wp).w_botline = old_botline;
            (*wp).w_empty_rows = old_empty_rows;
            (*wp).w_valid = old_valid;
        }
        (*wp).w_valid |= VALID_TOPLINE;
        (*wp).w_viewport_invalid = true;

        if set_topbot {
            cursor_correct_sms(wp);
        }
    }
}

/// Recompute topline to put the cursor halfway across the window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn scroll_cursor_halfway(wp: *mut win_T, atend: bool, prefer_above: bool) {
    unsafe {
        let old_topline = (*wp).w_topline;
        let mut loff = lineoff_T {
            lnum: (*wp).w_cursor.lnum,
            fill: 0,
            height: 0,
        };
        let mut boff = lineoff_T {
            lnum: (*wp).w_cursor.lnum,
            fill: 0,
            height: 0,
        };
        hasFolding(wp, loff.lnum, &raw mut loff.lnum, &raw mut boff.lnum);
        let mut used = plines_win_nofill(wp, loff.lnum, true);
        let mut topline = loff.lnum;
        let mut skipcol: colnr_T = 0;

        let mut want_height: c_int = 0;
        let do_sms = (*wp).w_onebuf_opt.wo_wrap != 0 && (*wp).w_onebuf_opt.wo_sms != 0;
        if do_sms {
            if atend {
                want_height = ((*wp).w_view_height - used) / 2;
                used = 0;
            } else {
                want_height = (*wp).w_view_height;
            }
        }

        let mut topfill: c_int = 0;
        let mut above: c_int = 0;
        let mut below: c_int = 0;
        while topline > 1 {
            if do_sms {
                topline_back_winheight(wp, &raw mut loff, false);
                if loff.height == MAXCOL {
                    break;
                }
                used += loff.height;
                if !atend && boff.lnum < (*(*wp).w_buffer).b_ml.ml_line_count {
                    botline_forw(wp, &raw mut boff);
                    used += boff.height;
                }
                if used > want_height {
                    if used - loff.height < want_height {
                        topline = loff.lnum;
                        topfill = loff.fill;
                        skipcol = skipcol_from_plines(wp, used - want_height);
                    }
                    break;
                }
                topline = loff.lnum;
                topfill = loff.fill;
                continue;
            }

            let mut done = false;
            for round in 1..=2 {
                if if prefer_above {
                    round == 2 && below < above
                } else {
                    round == 1 && below <= above
                } {
                    if boff.lnum < (*(*wp).w_buffer).b_ml.ml_line_count {
                        botline_forw(wp, &raw mut boff);
                        used += boff.height;
                        if used > (*wp).w_view_height {
                            done = true;
                            break;
                        }
                        below += boff.height;
                    } else {
                        below += 1;
                        if atend {
                            used += 1;
                        }
                    }
                }

                if if prefer_above {
                    round == 1 && below >= above
                } else {
                    round == 1 && below > above
                } {
                    topline_back(wp, &raw mut loff);
                    if loff.height == MAXCOL {
                        used = MAXCOL;
                    } else {
                        used += loff.height;
                    }
                    if used > (*wp).w_view_height {
                        done = true;
                        break;
                    }
                    above += loff.height;
                    topline = loff.lnum;
                    topfill = loff.fill;
                }
            }
            if done {
                break;
            }
        }

        if !hasFolding(wp, topline, &raw mut (*wp).w_topline, ptr::null_mut())
            && ((*wp).w_topline != topline || skipcol != 0 || (*wp).w_skipcol != 0)
        {
            (*wp).w_topline = topline;
            if skipcol != 0 {
                (*wp).w_skipcol = skipcol;
                redraw_later(wp, UPD_NOT_VALID);
            } else if do_sms {
                reset_skipcol(wp);
            }
        }
        (*wp).w_topfill = topfill;
        if old_topline > (*wp).w_topline + (*wp).w_view_height {
            (*wp).w_botfill = false;
        }
        check_topfill(wp, false);
        (*wp).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
        (*wp).w_valid |= VALID_TOPLINE;
    }
}

/// Correct the cursor position so that it is in a part of the screen at least
/// `'so'` lines from the top and bottom, if possible.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_correct(wp: *mut win_T) {
    unsafe {
        let mut above_wanted = get_scrolloff_value(wp);
        let mut below_wanted = get_scrolloff_value(wp);
        if mouse_dragging > 0 {
            above_wanted = i64::from(mouse_dragging - 1);
            below_wanted = i64::from(mouse_dragging - 1);
        }
        if (*wp).w_topline == 1 {
            above_wanted = 0;
            let max_off = i64::from((*wp).w_view_height / 2);
            below_wanted = below_wanted.min(max_off);
        }
        validate_botline_win(wp);
        if (*wp).w_botline == (*(*wp).w_buffer).b_ml.ml_line_count + 1 && mouse_dragging == 0 {
            below_wanted = 0;
            let max_off = i64::from(((*wp).w_view_height - 1) / 2);
            above_wanted = above_wanted.min(max_off);
        }

        let cln = (*wp).w_cursor.lnum;
        if i64::from(cln) >= i64::from((*wp).w_topline) + above_wanted
            && i64::from(cln) < i64::from((*wp).w_botline) - below_wanted
            && !win_lines_concealed(wp)
        {
            return;
        }

        let mut topline = (*wp).w_topline;
        let mut botline = (*wp).w_botline - 1;
        let mut above = (*wp).w_topfill;
        let mut below = (*wp).w_filler_rows;
        while (i64::from(above) < above_wanted || i64::from(below) < below_wanted)
            && topline < botline
        {
            if i64::from(below) < below_wanted
                && (below <= above || i64::from(above) >= above_wanted)
            {
                below += plines_win_full(wp, botline, ptr::null_mut(), ptr::null_mut(), true, true);
                hasFolding(wp, botline, &raw mut botline, ptr::null_mut());
                botline -= 1;
            }
            if i64::from(above) < above_wanted
                && (above < below || i64::from(below) >= below_wanted)
            {
                above += plines_win_nofill(wp, topline, true);
                hasFolding(wp, topline, ptr::null_mut(), &raw mut topline);
                if topline < botline {
                    above += win_get_fill(wp, topline + 1);
                }
                topline += 1;
            }
        }
        if topline == botline || botline == 0 {
            (*wp).w_cursor.lnum = topline;
        } else if topline > botline {
            (*wp).w_cursor.lnum = botline;
        } else {
            if cln < topline && (*wp).w_topline > 1 {
                (*wp).w_cursor.lnum = topline;
                (*wp).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW);
            }
            if cln > botline && (*wp).w_botline <= (*(*wp).w_buffer).b_ml.ml_line_count {
                (*wp).w_cursor.lnum = botline;
                (*wp).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW);
            }
        }
        check_cursor_moved(wp);
        (*wp).w_valid |= VALID_TOPLINE;
        (*wp).w_viewport_invalid = true;
    }
}

/// Decide how much overlap to use for page-up or page-down scrolling.
unsafe fn get_scroll_overlap(dir: Direction) -> c_int {
    unsafe {
        let min_height = (*curwin).w_view_height - 2;

        validate_botline_win(curwin);
        if (dir == BACKWARD && (*curwin).w_topline == 1)
            || (dir == FORWARD && (*curwin).w_botline > (*curbuf).b_ml.ml_line_count)
        {
            return min_height + 2;
        }

        let lnum = if dir == FORWARD {
            (*curwin).w_botline
        } else {
            (*curwin).w_topline - 1
        };
        let fill = win_get_fill(curwin, lnum + c_int::from(dir == BACKWARD))
            - if dir == FORWARD {
                (*curwin).w_filler_rows
            } else {
                (*curwin).w_topfill
            };
        let mut loff = lineoff_T {
            lnum,
            fill,
            height: if fill > 0 {
                1
            } else {
                plines_win_nofill(curwin, lnum, true)
            },
        };

        let h1 = loff.height;
        if h1 > min_height {
            return min_height + 2;
        }
        if dir == FORWARD {
            topline_back(curwin, &raw mut loff);
        } else {
            botline_forw(curwin, &raw mut loff);
        }

        let h2 = loff.height;
        if h2 == MAXCOL || h2 + h1 > min_height {
            return min_height + 2;
        }
        if dir == FORWARD {
            topline_back(curwin, &raw mut loff);
        } else {
            botline_forw(curwin, &raw mut loff);
        }

        let h3 = loff.height;
        if h3 == MAXCOL || h3 + h2 > min_height {
            return min_height + 2;
        }
        if dir == FORWARD {
            topline_back(curwin, &raw mut loff);
        } else {
            botline_forw(curwin, &raw mut loff);
        }

        let h4 = loff.height;
        if h4 == MAXCOL || h4 + h3 + h2 > min_height || h3 + h2 + h1 > min_height {
            min_height + 1
        } else {
            min_height
        }
    }
}

/// Scroll `count` lines with `'smoothscroll'` in direction `dir`.
unsafe fn scroll_with_sms(dir: Direction, mut count: c_int, curscount: *mut c_int) -> bool {
    unsafe {
        let prev_sms = (*curwin).w_onebuf_opt.wo_sms;
        let prev_skipcol = (*curwin).w_skipcol;
        let prev_topline = (*curwin).w_topline;
        let prev_topfill = (*curwin).w_topfill;

        (*curwin).w_onebuf_opt.wo_sms = 1;
        scroll_redraw(c_int::from(dir == FORWARD), count);

        if prev_sms == 0 && (*curwin).w_skipcol > 0 {
            let mut fixdir = dir;
            if ((*curwin).w_topline - prev_topline).abs() > c_int::from(dir == BACKWARD) {
                fixdir = -dir;
            }

            let width1 = (*curwin).w_view_width - win_col_off(curwin);
            let width2 = width1 + win_col_off2(curwin);
            count = 1 + ((*curwin).w_skipcol - width1 - 1) / width2;
            if fixdir == FORWARD {
                count = 1
                    + (linetabsize_eol(curwin, (*curwin).w_topline) - (*curwin).w_skipcol - width1
                        + width2
                        - 1)
                        / width2;
            }
            scroll_redraw(c_int::from(fixdir == FORWARD), count);
            *curscount += count * if fixdir == dir { 1 } else { -1 };
        }
        (*curwin).w_onebuf_opt.wo_sms = prev_sms;

        (*curwin).w_topline != prev_topline
            || (*curwin).w_topfill != prev_topfill
            || (*curwin).w_skipcol != prev_skipcol
    }
}

/// Move screen `count` (half) pages up (`dir` is `BACKWARD`) or down (`dir` is `FORWARD`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn pagescroll(dir: Direction, mut count: c_int, half: bool) -> c_int {
    unsafe {
        let mut did_move = false;
        let buflen = (*curbuf).b_ml.ml_line_count;
        let prev_col = (*curwin).w_cursor.col;
        let prev_curswant = (*curwin).w_curswant;
        let prev_lnum = (*curwin).w_cursor.lnum;
        let mut oa: oparg_T = std::mem::zeroed();
        let mut ca: cmdarg_T = std::mem::zeroed();
        ca.oap = &raw mut oa;

        if half {
            if count != 0 {
                (*curwin).w_onebuf_opt.wo_scr = i64::from((*curwin).w_view_height.min(count));
            }
            count = (*curwin)
                .w_view_height
                .min((*curwin).w_onebuf_opt.wo_scr as c_int);

            let mut curscount = count;
            if dir == FORWARD
                && ((*curwin).w_topline + (*curwin).w_view_height + count > buflen
                    || win_lines_concealed(curwin))
            {
                let mut n = plines_correct_topline(
                    curwin,
                    (*curwin).w_topline,
                    ptr::null_mut(),
                    false,
                    ptr::null_mut(),
                );
                if n - count < (*curwin).w_view_height && (*curwin).w_topline < buflen {
                    n += plines_m_win(
                        curwin,
                        (*curwin).w_topline + 1,
                        buflen,
                        (*curwin).w_view_height + count,
                    );
                }
                if n < (*curwin).w_view_height + count {
                    count = n - (*curwin).w_view_height;
                }
            }

            if count > 0 {
                did_move = scroll_with_sms(dir, count, &raw mut curscount);
                (*curwin).w_cursor.lnum = prev_lnum;
                (*curwin).w_cursor.col = prev_col;
                (*curwin).w_curswant = prev_curswant;
            }

            if (*curwin).w_onebuf_opt.wo_wrap != 0 {
                nv_screengo(&raw mut oa, dir, curscount, true);
            } else if dir == FORWARD {
                cursor_down_inner(curwin, curscount, true);
            } else {
                cursor_up_inner(curwin, curscount, true);
            }
        } else {
            let one_window = firstwin == lastwin;
            count *= if one_window && p_window > 0 && p_window < i64::from(Rows - 1) {
                (p_window as c_int - 2).max(1)
            } else {
                get_scroll_overlap(dir)
            };
            did_move = scroll_with_sms(dir, count, &raw mut count);

            if did_move {
                validate_botline_win(curwin);
                let lnum = if dir == FORWARD {
                    (*curwin).w_topline
                } else {
                    (*curwin).w_botline - 1
                };
                (*curwin).w_cursor.lnum = lnum.max(1);
            }
        }

        if get_scrolloff_value(curwin) > 0 {
            cursor_correct(curwin);
        }
        foldAdjustCursor(curwin);

        did_move =
            did_move || prev_col != (*curwin).w_cursor.col || prev_lnum != (*curwin).w_cursor.lnum;

        if !did_move {
            beep_flush();
        } else if (*curwin).w_onebuf_opt.wo_sms == 0 {
            beginline(BL_SOL | BL_FIX);
        } else if p_sol != 0 {
            nv_g_home_m_cmd(&raw mut ca);
        }

        if did_move { OK } else { FAIL }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_check_cursorbind() {
    unsafe {
        static mut prev_curwin: *mut win_T = ptr::null_mut();
        static mut prev_curbuf: *mut buf_T = ptr::null_mut();
        static mut prev_changedtick: varnumber_T = 0;
        static mut prev_cursor: pos_T = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };

        debug_assert!(!curwin.is_null() && !curbuf.is_null());
        if curwin == prev_curwin
            && curbuf == prev_curbuf
            && buf_get_changedtick(curbuf) == prev_changedtick
            && equalpos((*curwin).w_cursor, prev_cursor)
        {
            return;
        }
        prev_curwin = curwin;
        prev_curbuf = curbuf;
        prev_changedtick = buf_get_changedtick(curbuf);
        prev_cursor = (*curwin).w_cursor;

        let line = (*curwin).w_cursor.lnum;
        let col = (*curwin).w_cursor.col;
        let coladd = (*curwin).w_cursor.coladd;
        let curswant = (*curwin).w_curswant;
        let set_curswant = (*curwin).w_set_curswant;
        let old_curwin = curwin;
        let old_curbuf = curbuf;
        let old_VIsual_select = Visual.select;
        let old_VIsual_active = Visual.active;

        Visual.select = false;
        Visual.active = false;
        let mut wp = firstwin;
        while !wp.is_null() {
            curwin = wp;
            curbuf = (*curwin).w_buffer;
            if curwin != old_curwin && (*curwin).w_onebuf_opt.wo_crb != 0 {
                if (*curwin).w_onebuf_opt.wo_diff != 0 {
                    (*curwin).w_cursor.lnum = diff_get_corresponding_line(old_curbuf, line);
                } else {
                    (*curwin).w_cursor.lnum = line;
                }
                (*curwin).w_cursor.col = col;
                (*curwin).w_cursor.coladd = coladd;
                (*curwin).w_curswant = curswant;
                (*curwin).w_set_curswant = set_curswant;

                {
                    let restart_edit_save = restart_edit;
                    restart_edit = c_int::from(true);
                    check_cursor(curwin);

                    if (*curwin).w_onebuf_opt.wo_scb == 0 {
                        validate_cursor(curwin);
                    }

                    restart_edit = restart_edit_save;
                }
                mb_adjust_cursor();
                redraw_later(curwin, UPD_VALID);

                if (*curwin).w_onebuf_opt.wo_scb == 0 {
                    update_topline(curwin);
                }
                (*curwin).w_redr_status = true;
            }
            wp = (*wp).w_next;
        }

        Visual.select = old_VIsual_select;
        Visual.active = old_VIsual_active;
        curwin = old_curwin;
        curbuf = old_curbuf;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_and_viewstate_roundtrip() {
        let p1 = pos_T {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        let p2 = pos_T {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        assert!(equalpos(p1, p2));

        unsafe {
            let mut wp: win_T = std::mem::zeroed();
            wp.w_curswant = 42;
            wp.w_set_curswant = true;
            wp.w_leftcol = 4;
            wp.w_skipcol = 8;
            wp.w_topline = 10;
            wp.w_topfill = 2;
            wp.w_botline = 20;
            wp.w_empty_rows = 3;

            let mut vs = viewstate_T::default();
            save_viewstate(&raw mut wp, &raw mut vs);
            let mut wp2: win_T = std::mem::zeroed();
            restore_viewstate(&raw mut wp2, &raw const vs);
            assert_eq!(wp2.w_curswant, 42);
            assert!(wp2.w_set_curswant);
            assert_eq!(wp2.w_leftcol, 4);
            assert_eq!(wp2.w_skipcol, 8);
            assert_eq!(wp2.w_topline, 10);
            assert_eq!(wp2.w_topfill, 2);
            assert_eq!(wp2.w_botline, 20);
            assert_eq!(wp2.w_empty_rows, 3);
        }
    }
}
