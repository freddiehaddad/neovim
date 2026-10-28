//! Functions for moving the cursor and scrolling text (`src/nvim/move.c`).

use crate::buffer::{VALID_TOPLINE, buf_T, buf_get_changedtick, buf_is_empty};
use crate::cursor::{
    VALID_VIRTCOL, check_cursor, check_cursor_lnum, coladvance, colnr_T, equalpos, linenr_T, pos_T,
};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::diff::diff_get_corresponding_line;
use crate::drawscreen::{
    SIGN_WIDTH, UPD_INVERTED, UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, VALID_BOTLINE,
    conceal_cursor_line, number_width, redraw_buf_later, redraw_later, redrawWinline, redrawing,
    win_cursorline_standout, win_scroll_lines,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, tv_check_for_number_arg, tv_dict_add_nr, tv_dict_alloc_ret, tv_get_number,
    tv_get_number_chk, typval_T, varnumber_T,
};
use crate::eval::window::{find_win_by_nr_or_id, viewstate_T};
use crate::fold::{foldAdjustCursor, hasFolding};
use crate::globals::*;
use crate::input::beep_flush;
use crate::input_cmdatom::cmdarg_T;
use crate::insert::{beginline, cursor_down, cursor_down_inner, cursor_up, cursor_up_inner};
use crate::mbyte::{mb_adjust_cursor, utf_head_off};
use crate::memline::ml_get_buf;
use crate::message::semsg;
use crate::mouse::vcol2col;
use crate::normal::{nv_g_home_m_cmd, nv_screengo};
use crate::option::{
    OptInt, get_scrolloff_value, get_scrolloffpad_value, get_showbreak_value,
    get_sidescrolloff_value,
};
use crate::option_vars::*;
use crate::plines::{
    getvcol, getvvcol, linetabsize_eol, plines_m_win, plines_win, plines_win_full,
    plines_win_nofill, win_get_fill, win_may_fill,
};
use crate::strings::{tr, vim_strchr};
use crate::textformat::oparg_T;
use crate::window::{VALID_BOTLINE_AP, win_T, win_fdccol_count};
use crate::winfloat::win_check_anchored_floats;
use std::ffi::c_int;
use std::ptr;

use crate::charset::NUL;
use crate::drawline::{
    VALID_CHEIGHT, VALID_CROW, VALID_WCOL, VALID_WROW, kCpoNumcol, kOptCuloptFlagScreenline,
};
use crate::ex_cmds::{BL_FIX, BL_SOL};
use crate::regexp::MAXCOL;
use crate::search::{BACKWARD, Direction, FORWARD};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct lineoff_T {
    lnum: linenr_T,
    fill: c_int,
    height: c_int,
}

/// Get the number of screen lines skipped with `wp->w_skipcol`.
fn adjust_plines_for_skipcol(wp: *mut win_T) -> c_int {
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
pub fn plines_correct_topline(
    wp: *mut win_T,
    lnum: linenr_T,nextp: Option<&mut linenr_T>,
    limit_winheight: bool,foldedp: Option<&mut bool>,
) -> c_int {
    unsafe {
        let mut n = plines_win_full(wp, lnum, (nextp.is_some()).then(||  { &mut *nextp.unwrap() }), (foldedp.is_some()).then(||  { &mut *foldedp.unwrap() }), true, false);
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
fn comp_botline(wp: &mut win_T) {
    unsafe {
        check_cursor_moved(wp);

        let use_cache = redrawing()
            && !(*wp.w_buffer).b_mod_set
            && g_st().dollar_vcol == -1
            && wp.w_skipcol == 0
            && !win_may_fill(wp)
            && wp.w_lines_valid > 0
            && (&wp.w_lines)[0].wl_lnum <= wp.w_topline;

        let (mut lnum, mut done) = if (wp.w_valid & VALID_CROW) != 0 {
            (wp.w_cursor.lnum, wp.w_cline_row)
        } else {
            (wp.w_topline, 0)
        };

        let mut i: c_int = 0;
        if use_cache {
            while i < wp.w_lines_valid && (&wp.w_lines)[i as usize].wl_lnum < lnum {
                i += 1;
            }
        }

        while lnum <= (*wp.w_buffer).b_ml.ml_line_count {
            let mut valid = false;
            if use_cache && i < wp.w_lines_valid {
                let wl = (&wp.w_lines)[i as usize];
                if wl.wl_lnum < lnum || !wl.wl_valid {
                    i += 1;
                    continue;
                }
                if wl.wl_lnum == lnum {
                    valid = true;
                } else {
                    i -= 1;
                }
            }
            let mut last = lnum;
            let mut folded = false;
            let n = if valid {
                let wl = (&wp.w_lines)[i as usize];
                folded = wl.wl_folded;
                last = wl.wl_lastlnum;
                c_int::from(wl.wl_size)
            } else {
                plines_correct_topline(wp, lnum, Some(&mut last), true, Some(&mut folded))
            };
            if lnum <= wp.w_cursor.lnum && last >= wp.w_cursor.lnum {
                wp.w_cline_row = done;
                wp.w_cline_height = n;
                wp.w_cline_folded = folded;
                redraw_for_cursorline(wp);
                wp.w_valid |= VALID_CROW | VALID_CHEIGHT;
            }
            if done + n > wp.w_view_height {
                break;
            }
            done += n;
            lnum = last + 1;
            i += 1;
        }

        wp.w_botline = lnum;
        wp.w_valid |= VALID_BOTLINE | VALID_BOTLINE_AP;
        wp.w_viewport_invalid = true;

        set_empty_rows(wp, done);

        win_check_anchored_floats(wp);
    }
}

/// Redraw when `w_cline_row` changes and `'relativenumber'` or `'cursorline'` is set.
fn redraw_for_cursorline(wp: &mut win_T) {
    {
        if (wp.w_valid & VALID_CROW) != 0 {
            return;
        }

        if wp.w_onebuf_opt.wo_rnu != 0 || win_cursorline_standout(&*wp) {
            redraw_later(wp, UPD_VALID);
        }
    }
}

/// Redraw when `'concealcursor'` is active, or when `w_virtcol` changes.
fn redraw_for_cursorcolumn(wp: *mut win_T) {
    unsafe {
        if wp == *curwin.as_ptr() && (*wp).w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(wp) {
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

        if g_st().Visual.active && (*wp).w_buffer == g_st().curbuf {
            redraw_buf_later(g_st().curbuf, UPD_INVERTED);
        }
    }
}

/// Set `wp->w_virtcol` to a value (`vcol`) that is already valid.
pub fn set_valid_virtcol(wp: &mut win_T, vcol: colnr_T) {
    wp.w_virtcol = vcol;
        redraw_for_cursorcolumn(&raw mut *wp);
        wp.w_valid |= VALID_VIRTCOL;
}

/// Calculates how much the `'listchars'` `"precedes"` or `'smoothscroll'` `"<<<"`
/// marker overlaps with buffer text for window `wp`.
pub fn sms_marker_overlap(wp: *mut win_T, mut extra2: c_int) -> c_int {
    unsafe {
        if extra2 == -1 {
            extra2 = win_col_off(wp) - win_col_off2(wp);
        }
        if *get_showbreak_value(&mut *wp) != NUL {
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
fn skipcol_from_plines(wp: &mut win_T, plines_off: c_int) -> c_int {
    unsafe {
        let width1 = wp.w_view_width - win_col_off(wp);

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
fn reset_skipcol(wp: *mut win_T) {
    unsafe {
        if (*wp).w_skipcol == 0 {
            return;
        }

        (*wp).w_skipcol = 0;
        redraw_later(wp, UPD_SOME_VALID);
    }
}

/// Return true when `'scrolloffpad'` may augment `'scrolloff'`.
fn use_scrolloffpad(wp: *mut win_T) -> bool {
    get_scrolloff_value(wp) > 0 && get_scrolloffpad_value(wp) > 0
}

/// Return true when there are not enough real buffer lines below `lnum` to
/// satisfy the requested `so` context.
fn scrolloffpad_eof_pressure(wp: *mut win_T, lnum: linenr_T, so: OptInt) -> bool {
    unsafe {
        if !use_scrolloffpad(wp) || so <= 0 {
            return false;
        }

        i64::from(lnum) > i64::from((*(*wp).w_buffer).b_ml.ml_line_count) - so
    }
}

/// Update `wp->w_topline` to move the cursor onto the screen.
pub fn update_topline(wp: *mut win_T) {
    unsafe {
        let mut check_botline = false;
        let so_ptr: *mut OptInt = if (*wp).w_onebuf_opt.wo_so >= 0 {
            &raw mut (*wp).w_onebuf_opt.wo_so
        } else {
            &raw mut opt_vars().p_so
        };
        let save_so = *so_ptr;

        if g_st().skip_update_topline {
            return;
        }

        if !crate::grid::grid_allocated(&g_st().default_grid) || (*wp).w_view_height == 0 {
            check_cursor_lnum(&mut *wp);
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

        if g_st().mouse_dragging > 0 {
            *so_ptr = i64::from(g_st().mouse_dragging - 1);
        }
        let eof_pressure = scrolloffpad_eof_pressure(wp, (*wp).w_cursor.lnum, *so_ptr);

        let old_topline = (*wp).w_topline;
        let old_topfill = (*wp).w_topfill;

        if buf_is_empty(&mut *(*wp).w_buffer) {
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
                        &mut (*wp).w_cursor,
                        Some(&mut vcol),
                        None,
                        None,
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
                if win_lines_concealed(&mut *wp) {
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
                        hasFolding(wp, lnum, None, &raw mut lnum);
                        lnum += 1;
                    }
                } else {
                    n = i64::from((*wp).w_topline) + *so_ptr - i64::from((*wp).w_cursor.lnum);
                }

                let min_scroll = scrolljump_value(wp);
                if eof_pressure {
                    scroll_cursor_halfway(&mut *wp, true, true);
                } else if n >= i64::from(halfheight) && min_scroll < halfheight {
                    scroll_cursor_halfway(&mut *wp, false, false);
                } else {
                    scroll_cursor_top(&mut *wp, min_scroll, 0);
                    check_botline = true;
                }
            } else {
                hasFolding(
                    wp,
                    (*wp).w_topline,
                    Some(&mut (*wp).w_topline),
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
                        || win_lines_concealed(&mut *wp)
                    {
                        let mut loff = lineoff_T {
                            lnum: (*wp).w_cursor.lnum,
                            fill: 0,
                            height: 0,
                        };
                        let mut n = if eof_pressure { 0 } else { (*wp).w_empty_rows };
                        hasFolding(wp, loff.lnum, None, &raw mut loff.lnum);
                        n += (*wp).w_filler_rows;
                        while loff.lnum < (*wp).w_botline
                            && (loff.lnum + 1 < (*wp).w_botline || loff.fill == 0)
                        {
                            n += loff.height;
                            if i64::from(n) >= *so_ptr {
                                break;
                            }
                            botline_forw(wp, &mut loff);
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
                    if win_lines_concealed(&mut *wp) {
                        let mut lnum = (*wp).w_cursor.lnum;
                        while i64::from(lnum) >= i64::from((*wp).w_botline) - *so_ptr {
                            if lnum <= 0 {
                                break;
                            }
                            n += i64::from(!decor_conceal_line(wp, lnum - 1, false));
                            if n > i64::from((*wp).w_view_height + 1) {
                                break;
                            }
                            hasFolding(wp, lnum, Some(&mut lnum), ptr::null_mut());
                            lnum -= 1;
                        }
                    } else {
                        n = i64::from((*wp).w_cursor.lnum) - i64::from((*wp).w_botline)
                            + 1
                            + *so_ptr;
                    }
                    if n <= i64::from((*wp).w_view_height + 1) {
                        if eof_pressure {
                            scroll_cursor_halfway(&mut *wp, true, true);
                        } else {
                            scroll_cursor_bot(&mut *wp, scrolljump_value(wp), false);
                        }
                    } else {
                        scroll_cursor_halfway(&mut *wp, eof_pressure, eof_pressure);
                    }
                }
            }
        }
        (*wp).w_valid |= VALID_TOPLINE;
        (*wp).w_viewport_invalid = true;
        win_check_anchored_floats(wp);

        if (*wp).w_topline != old_topline || (*wp).w_topfill != old_topfill {
            g_st().dollar_vcol = -1;
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
fn scrolljump_value(wp: *mut win_T) -> c_int {
    unsafe {
        if opt_vars().p_sj >= 0 {
            opt_vars().p_sj as c_int
        } else {
            ((*wp).w_view_height * (-opt_vars().p_sj as c_int)) / 100
        }
    }
}

/// Return true when there are not `'scrolloff'` lines above the cursor for window `wp`.
fn check_top_offset(wp: *mut win_T) -> bool {
    unsafe {
        let so = get_scrolloff_value(wp);
        if i64::from((*wp).w_cursor.lnum) < i64::from((*wp).w_topline) + so
            || win_lines_concealed(&mut *wp)
        {
            let mut loff = lineoff_T {
                lnum: (*wp).w_cursor.lnum,
                fill: 0,
                height: 0,
            };
            let mut n = i64::from((*wp).w_topfill);
            while n < so {
                topline_back(wp, &mut loff);
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
pub fn update_curswant_force() {
    unsafe {
        validate_virtcol(*curwin.as_ptr());
        (**curwin.as_ptr()).w_curswant = (**curwin.as_ptr()).w_virtcol;
        (**curwin.as_ptr()).w_set_curswant = false;
    }
}

/// Update `w_curswant` if `w_set_curswant` is set.
pub fn update_curswant() {
    unsafe {
        if (**curwin.as_ptr()).w_set_curswant {
            update_curswant_force();
        }
    }
}

/// Check if the cursor has moved. Set the `w_valid` flag accordingly.
pub fn check_cursor_moved(wp: *mut win_T) {
    unsafe {
        if (*wp).w_cursor.lnum != (*wp).w_valid_cursor.lnum {
            (*wp).w_valid &= !(VALID_WROW
                | VALID_WCOL
                | VALID_VIRTCOL
                | VALID_CHEIGHT
                | VALID_CROW
                | VALID_TOPLINE);

            if wp == *curwin.as_ptr()
                && (*wp).w_valid_cursor.lnum > 0
                && (*wp).w_onebuf_opt.wo_cole >= 2
                && !conceal_cursor_line(wp)
                && (decor_conceal_line(wp, (*wp).w_cursor.lnum - 1, true)
                    || decor_conceal_line(wp, (*wp).w_valid_cursor.lnum - 1, true))
            {
                changed_window_setting(&mut *wp);
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
pub fn changed_window_setting(wp: &mut win_T) {
    wp.w_lines_valid = 0;
        changed_line_abv_curs_win(wp);
        wp.w_valid &= !(VALID_BOTLINE | VALID_BOTLINE_AP | VALID_TOPLINE);
        redraw_later(wp, UPD_NOT_VALID);
}

/// Call `changed_window_setting()` for every window.
pub fn changed_window_setting_all() {
    unsafe {
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                changed_window_setting(&mut *wp);
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// Set `wp->w_topline` to a certain number.
pub fn set_topline(wp: &mut win_T, mut lnum: linenr_T) {
    unsafe {
        let prev_topline = wp.w_topline;

        hasFolding(wp, lnum, Some(&mut lnum), ptr::null_mut());
        wp.w_botline += lnum - wp.w_topline;
        if wp.w_botline > (*wp.w_buffer).b_ml.ml_line_count + 1 {
            wp.w_botline = (*wp.w_buffer).b_ml.ml_line_count + 1;
        }
        wp.w_topline = lnum;
        wp.w_topline_was_set = true;
        if lnum != prev_topline {
            wp.w_topfill = 0;
        }
        wp.w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_TOPLINE);
        redraw_later(wp, UPD_VALID);
    }
}

/// Saves the view of `wp`, for `restore_viewstate()`.
pub fn save_viewstate(wp: &mut win_T, vs: &mut viewstate_T) {
    vs.vs_curswant = wp.w_curswant;
        vs.vs_set_curswant = wp.w_set_curswant;
        vs.vs_leftcol = wp.w_leftcol;
        vs.vs_skipcol = wp.w_skipcol;
        vs.vs_topline = wp.w_topline;
        vs.vs_topfill = wp.w_topfill;
        vs.vs_botline = wp.w_botline;
        vs.vs_empty_rows = wp.w_empty_rows;
}

/// Restores the view saved by `save_viewstate()`. No validation.
pub fn restore_viewstate(wp: &mut win_T, vs: &viewstate_T) {
    wp.w_curswant = vs.vs_curswant;
        wp.w_set_curswant = vs.vs_set_curswant;
        wp.w_leftcol = vs.vs_leftcol;
        wp.w_skipcol = vs.vs_skipcol;
        wp.w_topline = vs.vs_topline;
        wp.w_topfill = vs.vs_topfill;
        wp.w_botline = vs.vs_botline;
        wp.w_empty_rows = vs.vs_empty_rows;
}

/// Call this function when the length of the cursor line (in screen
/// characters) has changed, and the change is before the cursor.
pub fn changed_cline_bef_curs(wp: &mut win_T) {
    wp.w_valid &=
            !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL | VALID_CROW | VALID_CHEIGHT | VALID_TOPLINE);
}

/// Call this function when the length of a line above the cursor has changed.
pub fn changed_line_abv_curs() {
    unsafe {
        (**curwin.as_ptr()).w_valid &=
            !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL | VALID_CROW | VALID_CHEIGHT | VALID_TOPLINE);
    }
}

pub fn changed_line_abv_curs_win(wp: &mut win_T) {
    wp.w_valid &=
            !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL | VALID_CROW | VALID_CHEIGHT | VALID_TOPLINE);
}

/// Make sure the value of `wp->w_botline` is valid.
pub fn validate_botline_win(wp: *mut win_T) {
    unsafe {
        if ((*wp).w_valid & VALID_BOTLINE) == 0 {
            comp_botline(&mut *wp);
        }
    }
}

/// Mark `wp->w_botline` as invalid.
pub fn invalidate_botline_win(wp: &mut win_T) {
    wp.w_valid &= !(VALID_BOTLINE | VALID_BOTLINE_AP);
}

pub fn approximate_botline_win(wp: &mut win_T) {
    wp.w_valid &= !VALID_BOTLINE;
}

/// Return true if `wp->w_wrow` and `wp->w_wcol` are valid.
pub fn cursor_valid(wp: &mut win_T) -> c_int {
    check_cursor_moved(&raw mut *wp);
        c_int::from((wp.w_valid & (VALID_WROW | VALID_WCOL)) == (VALID_WROW | VALID_WCOL))
}

/// Validate cursor position. Makes sure `w_wrow` and `w_wcol` are valid.
pub fn validate_cursor(wp: *mut win_T) {
    unsafe {
        check_cursor_lnum(&mut *wp);
        check_cursor_moved(wp);
        if ((*wp).w_valid & (VALID_WCOL | VALID_WROW)) != (VALID_WCOL | VALID_WROW) {
            curs_columns(wp, c_int::from(true));
        }
    }
}

/// Compute `wp->w_cline_row` and `wp->w_cline_height`, based on `wp->w_topline`.
fn curs_rows(wp: &mut win_T) {
    unsafe {
        let all_invalid = !redrawing()
            || wp.w_lines_valid == 0
            || (&wp.w_lines)[0].wl_lnum > wp.w_topline;
        let mut i: c_int = 0;
        wp.w_cline_row = 0;
        let mut lnum = wp.w_topline;
        while lnum < wp.w_cursor.lnum {
            let mut valid = false;
            if !all_invalid && i < wp.w_lines_valid {
                let wl = (&wp.w_lines)[i as usize];
                if wl.wl_lnum < lnum || !wl.wl_valid {
                    i += 1;
                    continue;
                }
                if wl.wl_lnum == lnum {
                    if !(*wp.w_buffer).b_mod_set
                        || wl.wl_lastlnum < wp.w_cursor.lnum
                        || (*wp.w_buffer).b_mod_top > wl.wl_lastlnum + 1
                    {
                        valid = true;
                    }
                } else if wl.wl_lnum > lnum {
                    i -= 1;
                }
            }
            if valid && (lnum != wp.w_topline || (wp.w_skipcol == 0 && !win_may_fill(wp))) {
                let wl = (&wp.w_lines)[i as usize];
                lnum = wl.wl_lastlnum + 1;
                if lnum > wp.w_cursor.lnum {
                    i += 1;
                    break;
                }
                wp.w_cline_row += c_int::from(wl.wl_size);
            } else {
                let mut last = lnum;
                let mut folded = false;
                let n = plines_correct_topline(wp, lnum, Some(&mut last), true, Some(&mut folded));
                lnum = last + 1;
                if lnum + c_int::from(decor_conceal_line(wp, lnum - 1, false)) > wp.w_cursor.lnum
                {
                    i += 1;
                    break;
                }
                wp.w_cline_row += n;
            }
            i += 1;
        }

        check_cursor_moved(wp);
        if (wp.w_valid & VALID_CHEIGHT) == 0 {
            if all_invalid
                || i == wp.w_lines_valid
                || (i < wp.w_lines_valid
                    && (!(&wp.w_lines)[i as usize].wl_valid
                        || (&wp.w_lines)[i as usize].wl_lnum != wp.w_cursor.lnum))
            {
                wp.w_cline_height = plines_win_full(
                    wp,
                    wp.w_cursor.lnum,
                    None,
                    Some(&mut wp.w_cline_folded),
                    true,
                    true,
                );
            } else if i > wp.w_lines_valid {
                wp.w_cline_height = 0;
                wp.w_cline_folded =
                    hasFolding(wp, wp.w_cursor.lnum, None, ptr::null_mut());
            } else {
                let wl = (&wp.w_lines)[i as usize];
                wp.w_cline_height = c_int::from(wl.wl_size);
                wp.w_cline_folded = wl.wl_folded;
            }
        }

        redraw_for_cursorline(wp);
        wp.w_valid |= VALID_CROW | VALID_CHEIGHT;
    }
}

/// Validate `wp->w_virtcol` only.
pub fn validate_virtcol(wp: *mut win_T) {
    unsafe {
        check_cursor_moved(wp);

        if ((*wp).w_valid & VALID_VIRTCOL) != 0 {
            return;
        }

        getvvcol(
            wp,
            &mut (*wp).w_cursor,
            None,
            Some(&mut (*wp).w_virtcol),
            None,
            0,
        );
        redraw_for_cursorcolumn(wp);
        (*wp).w_valid |= VALID_VIRTCOL;
    }
}

/// Validate `wp->w_cline_height` only.
pub fn validate_cheight(wp: *mut win_T) {
    unsafe {
        check_cursor_moved(wp);

        if ((*wp).w_valid & VALID_CHEIGHT) != 0 {
            return;
        }

        (*wp).w_cline_height = plines_win_full(
            wp,
            (*wp).w_cursor.lnum,
            None,
            Some(&mut (*wp).w_cline_folded),
            true,
            true,
        );
        (*wp).w_valid |= VALID_CHEIGHT;
    }
}

/// Validate `w_wcol` and `w_virtcol` only.
pub fn validate_cursor_col(wp: &mut win_T) {
    unsafe {
        validate_virtcol(&raw mut *wp);

        if (wp.w_valid & VALID_WCOL) != 0 {
            return;
        }

        let mut col = wp.w_virtcol;
        let off = win_col_off(&raw mut *wp);
        col += off;
        let width = wp.w_view_width - off + win_col_off2(&raw mut *wp);

        if wp.w_onebuf_opt.wo_wrap != 0 && col >= wp.w_view_width && width > 0 {
            col -= ((col - wp.w_view_width) / width + 1) * width;
        }
        if col > wp.w_leftcol {
            col -= wp.w_leftcol;
        } else {
            col = 0;
        }
        wp.w_wcol = col;

        wp.w_valid |= VALID_WCOL;
    }
}

/// Compute offset of a window, occupied by absolute or relative line number,
/// fold column and sign column.
#[unsafe(no_mangle)]
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
        num_off + win_fdccol_count(&mut *wp) + ((*wp).w_scwidth * SIGN_WIDTH)
    }
}

/// Return the difference in column offset for the second screen line of a
/// wrapped line.
pub fn win_col_off2(wp: *mut win_T) -> c_int {
    unsafe {
        if ((*wp).w_onebuf_opt.wo_nu != 0
            || (*wp).w_onebuf_opt.wo_rnu != 0
            || *(*wp).w_onebuf_opt.wo_stc != NUL)
            && !vim_strchr(opt_vars().p_cpo, kCpoNumcol).is_null()
        {
            return number_width(wp) + c_int::from(*(*wp).w_onebuf_opt.wo_stc == NUL);
        }
        0
    }
}

/// Compute `wp->w_wcol` and `wp->w_virtcol`, and update `wp->w_wrow`,
/// `wp->w_cline_row`, and `wp->w_leftcol`.
pub fn curs_columns(wp: *mut win_T, may_scroll: c_int) {
    unsafe {
        let mut startcol: colnr_T = 0;
        let mut endcol: colnr_T = 0;

        update_topline(wp);

        if ((*wp).w_valid & VALID_CROW) == 0 {
            curs_rows(&mut *wp);
        }

        if (*wp).w_cline_folded {
            startcol = (*wp).w_leftcol;
            (*wp).w_virtcol = startcol;
            endcol = startcol;
        } else {
            getvvcol(
                wp,
                &mut (*wp).w_cursor,
                Some(&mut startcol),
                Some(&mut (*wp).w_virtcol),
                Some(&mut endcol),
                0,
            );
        }

        if startcol > g_st().dollar_vcol {
            g_st().dollar_vcol = -1;
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
                    if opt_vars().p_ss == 0 || diff >= i64::from(width1 / 2) || off_right >= off_left {
                        (*wp).w_wcol - extra - width1 / 2
                    } else {
                        if diff < opt_vars().p_ss {
                            diff = opt_vars().p_ss;
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
pub fn textpos2screenpos(
    wp: &mut win_T,
    pos: &mut pos_T,
    rowp: &mut c_int,
    scolp: &mut c_int,
    ccolp: &mut c_int,
    ecolp: &mut c_int,
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

        let mut lnum = pos.lnum;
        if lnum >= wp.w_topline && lnum <= wp.w_botline {
            is_folded = hasFolding(&raw mut *wp, lnum, Some(&mut lnum), ptr::null_mut());
            row = plines_m_win(wp, wp.w_topline, lnum - 1, c_int::MAX);
            row -= adjust_plines_for_skipcol(&raw mut *wp);
            row += if lnum == wp.w_topline {
                wp.w_topfill
            } else {
                win_get_fill(&raw mut *wp, lnum)
            };
            visible_row = true;
        } else if !local || lnum < wp.w_topline {
            row = 0;
        } else {
            row = wp.w_view_height - 1;
        }

        let existing_row = lnum > 0 && lnum <= (*wp.w_buffer).b_ml.ml_line_count;

        if (local || visible_row) && existing_row {
            let off = win_col_off(&raw mut *wp);
            if is_folded {
                row += if local {
                    0
                } else {
                    wp.w_winrow + wp.w_winrow_off
                } + 1;
                coloff = if local {
                    0
                } else {
                    wp.w_wincol + wp.w_wincol_off
                } + 1
                    + off;
            } else {
                getvcol(&mut *wp, pos, Some(&mut scol), Some(&mut ccol), Some(&mut ecol), 0);

                let mut col = scol + off;
                let width = wp.w_view_width - off + win_col_off2(&raw mut *wp);

                if wp.w_onebuf_opt.wo_wrap != 0 && col >= wp.w_view_width && width > 0 {
                    let rowoff = if visible_row {
                        (col - wp.w_view_width) / width + 1
                    } else {
                        0
                    };
                    col -= rowoff * width;
                    row += rowoff;
                }

                col -= wp.w_leftcol;

                if col >= 0 && col < wp.w_view_width && row >= 0 && row < wp.w_view_height {
                    coloff = col - scol
                        + if local {
                            0
                        } else {
                            wp.w_wincol + wp.w_wincol_off
                        }
                        + 1;
                    row += if local {
                        0
                    } else {
                        wp.w_winrow + wp.w_winrow_off
                    } + 1;
                } else {
                    scol = 0;
                    ccol = 0;
                    ecol = 0;
                    if local {
                        coloff = if col < 0 { -1 } else { wp.w_view_width + 1 };
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
pub fn f_screenpos(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        tv_dict_alloc_ret(rettv);
        let dict = rettv.vval.v_dict();

        let wp = find_win_by_nr_or_id(&argvars[0]);
        if wp.is_null() {
            return;
        }

        let mut pos = pos_T {
            lnum: tv_get_number(&argvars[1]) as linenr_T,
            col: (tv_get_number(&argvars[2]) as colnr_T - 1).max(0),
            coladd: 0,
        };
        if pos.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
            semsg!(tr(e_invalid_line_number_nr.as_ptr()), pos.lnum);
            return;
        }
        let mut row: c_int = 0;
        let mut scol: c_int = 0;
        let mut ccol: c_int = 0;
        let mut ecol: c_int = 0;
        textpos2screenpos(
            &mut *wp,
            &mut pos,
            &mut row,
            &mut scol,
            &mut ccol,
            &mut ecol,
            false,
        );

        tv_dict_add_nr(dict, c"row", varnumber_T::from(row));
        tv_dict_add_nr(dict, c"col", varnumber_T::from(scol));
        tv_dict_add_nr(
            dict,
            c"curscol",
            varnumber_T::from(ccol),
        );
        tv_dict_add_nr(
            dict,
            c"endcol",
            varnumber_T::from(ecol),
        );
    }
}

/// Convert a virtual (screen) column to a character column (1-based).
fn virtcol2col(wp: &mut win_T, lnum: linenr_T, vcol: c_int) -> c_int {
    unsafe {
        let offset = vcol2col(wp, lnum, vcol - 1, None);
        let line = ml_get_buf(wp.w_buffer, lnum);
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
pub fn f_virtcol2col(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if !tv_check_for_number_arg(argvars, 0)
            || !tv_check_for_number_arg(argvars, 1)
            || !tv_check_for_number_arg(argvars, 2)
        {
            return;
        }

        let wp = find_win_by_nr_or_id(&argvars[0]);
        if wp.is_null() {
            return;
        }

        let mut error = false;
        let lnum = tv_get_number_chk(&argvars[1], Some(&mut error)) as linenr_T;
        if error || lnum < 0 || lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
            return;
        }

        let screencol = tv_get_number_chk(&argvars[2], Some(&mut error)) as c_int;
        if error || screencol < 0 {
            return;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(virtcol2col(&mut *wp, lnum, screencol)));
    }
}

/// Make sure the cursor is in the visible part of the topline after scrolling
/// the screen with `'smoothscroll'`.
fn cursor_correct_sms(wp: *mut win_T) {
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
            let rc = coladvance(&mut *wp, (*wp).w_curswant);
            (*wp).w_valid &=
                !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL);
            if !rc
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
pub fn scroll_redraw(up: c_int, count: linenr_T) {
    unsafe {
        let prev_topline = (**curwin.as_ptr()).w_topline;
        let prev_skipcol = (**curwin.as_ptr()).w_skipcol;
        let prev_topfill = (**curwin.as_ptr()).w_topfill;
        let prev_lnum = (**curwin.as_ptr()).w_cursor.lnum;

        let moved = if up != 0 {
            scrollup(&mut **curwin.as_ptr(), count, true)
        } else {
            scrolldown(&mut **curwin.as_ptr(), count, c_int::from(true))
        };

        if get_scrolloff_value(*curwin.as_ptr()) > 0 {
            cursor_correct(&mut **curwin.as_ptr());
            check_cursor_moved(*curwin.as_ptr());
            (**curwin.as_ptr()).w_valid |= VALID_TOPLINE;

            while (**curwin.as_ptr()).w_topline == prev_topline
                && (**curwin.as_ptr()).w_skipcol == prev_skipcol
                && (**curwin.as_ptr()).w_topfill == prev_topfill
            {
                if up != 0 {
                    if (**curwin.as_ptr()).w_cursor.lnum > prev_lnum || !cursor_down(1, false) {
                        break;
                    }
                } else if (**curwin.as_ptr()).w_cursor.lnum < prev_lnum
                    || prev_topline == 1
                    || !cursor_up(1, false)
                {
                    break;
                }
                check_cursor_moved(*curwin.as_ptr());
                (**curwin.as_ptr()).w_valid |= VALID_TOPLINE;
            }
        }

        if moved {
            (**curwin.as_ptr()).w_viewport_invalid = true;
        }

        cursor_correct_sms(*curwin.as_ptr());
        if (**curwin.as_ptr()).w_cursor.lnum != prev_lnum {
            coladvance(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_curswant);
        }
        redraw_later(*curwin.as_ptr(), UPD_VALID);
    }
}

/// Scroll a window down by `line_count` logical lines (`CTRL-Y`).
pub fn scrolldown(wp: &mut win_T, line_count: linenr_T, byfold: c_int) -> bool {
    unsafe {
        let mut done: c_int = 0;
        let mut width1: c_int = 0;
        let mut width2: c_int = 0;
        let do_sms = wp.w_onebuf_opt.wo_wrap != 0 && wp.w_onebuf_opt.wo_sms != 0;

        if do_sms {
            width1 = wp.w_view_width - win_col_off(&raw mut *wp);
            width2 = width1 + win_col_off2(&raw mut *wp);
        }

        hasFolding(
            &raw mut *wp,
            wp.w_topline,
            Some(&mut wp.w_topline),
            ptr::null_mut(),
        );
        validate_cursor(&raw mut *wp);
        let mut todo = line_count;
        while todo > 0 {
            let can_fill = wp.w_topfill < wp.w_view_height - 1
                && wp.w_topfill < win_get_fill(&raw mut *wp, wp.w_topline);
            if wp.w_topline == 1 && !can_fill && (!do_sms || wp.w_skipcol < width1) {
                break;
            }
            if do_sms && wp.w_skipcol >= width1 {
                if wp.w_skipcol >= width1 + width2 {
                    wp.w_skipcol -= width2;
                } else {
                    wp.w_skipcol -= width1;
                }
                redraw_later(&raw mut *wp, UPD_NOT_VALID);
                done += 1;
            } else if can_fill {
                wp.w_topfill += 1;
                done += 1;
            } else {
                wp.w_topline -= 1;
                wp.w_skipcol = 0;
                wp.w_topfill = 0;
                let mut first: linenr_T = 0;
                if hasFolding(&raw mut *wp, wp.w_topline, Some(&mut first), ptr::null_mut()) {
                    done += c_int::from(!decor_conceal_line(&raw mut *wp, first - 1, false));
                    if byfold == 0 {
                        todo -= wp.w_topline - first - 1;
                    }
                    wp.w_botline -= wp.w_topline - first;
                    wp.w_topline = first;
                } else if decor_conceal_line(&raw mut *wp, wp.w_topline - 1, false) {
                    todo += 1;
                } else if do_sms {
                    let mut size = linetabsize_eol(&raw mut *wp, wp.w_topline);
                    if size > width1 {
                        wp.w_skipcol = width1;
                        size -= width1;
                        redraw_later(&raw mut *wp, UPD_NOT_VALID);
                    }
                    while size > width2 {
                        wp.w_skipcol += width2;
                        size -= width2;
                    }
                    done += 1;
                } else {
                    done += plines_win_nofill(&raw mut *wp, wp.w_topline, true);
                }
            }
            wp.w_botline -= 1;
            invalidate_botline_win(&mut *wp);
            todo -= 1;
        }

        if wp.w_topfill == win_get_fill(&raw mut *wp, wp.w_topline) {
            while wp.w_topline > 1 && decor_conceal_line(&raw mut *wp, wp.w_topline - 2, false) {
                wp.w_topline -= 1;
                wp.w_topfill = 0;
                hasFolding(
                    &raw mut *wp,
                    wp.w_topline,
                    Some(&mut wp.w_topline),
                    ptr::null_mut(),
                );
            }
        }

        wp.w_wrow += done;
        wp.w_cline_row += done;

        if wp.w_cursor.lnum == wp.w_topline {
            wp.w_cline_row = 0;
        }
        check_topfill(&raw mut *wp, true);

        let mut wrow = wp.w_wrow;
        if wp.w_onebuf_opt.wo_wrap != 0 && wp.w_view_width != 0 {
            validate_virtcol(&raw mut *wp);
            validate_cheight(&raw mut *wp);
            wrow += wp.w_cline_height - 1 - wp.w_virtcol / wp.w_view_width;
        }
        let mut moved = false;
        while wrow >= wp.w_view_height && wp.w_cursor.lnum > 1 {
            let mut first: linenr_T = 0;
            if hasFolding(&raw mut *wp, wp.w_cursor.lnum, Some(&mut first), ptr::null_mut()) {
                wrow -= c_int::from(!decor_conceal_line(&raw mut *wp, wp.w_cursor.lnum - 1, false));
                wp.w_cursor.lnum = (first - 1).max(1);
            } else {
                wrow -= plines_win(&raw mut *wp, wp.w_cursor.lnum, true);
                wp.w_cursor.lnum -= 1;
            }
            wp.w_valid &=
                !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL);
            moved = true;
        }
        if moved {
            foldAdjustCursor(wp);
            let curswant = wp.w_curswant;
            coladvance(&mut *wp, curswant);
        }
        wp.w_cursor.lnum = wp.w_cursor.lnum.max(wp.w_topline);

        moved
    }
}

/// Scroll a window up by `line_count` logical lines (`CTRL-E`).
pub fn scrollup(wp: &mut win_T, line_count: linenr_T, byfold: bool) -> bool {
    unsafe {
        let topline = wp.w_topline;
        let botline = wp.w_botline;
        let do_sms = wp.w_onebuf_opt.wo_wrap != 0 && wp.w_onebuf_opt.wo_sms != 0;

        if do_sms || (byfold && win_lines_concealed(wp)) || win_may_fill(wp) {
            let width1 = wp.w_view_width - win_col_off(wp);
            let width2 = width1 + win_col_off2(wp);
            let mut size = if do_sms {
                linetabsize_eol(wp, wp.w_topline)
            } else {
                0
            };
            let prev_skipcol = wp.w_skipcol;

            let mut todo = line_count;
            while todo > 0 {
                todo += c_int::from(decor_conceal_line(wp, wp.w_topline - 1, false));
                if wp.w_topfill > 0 {
                    wp.w_topfill -= 1;
                } else {
                    let mut lnum = wp.w_topline;
                    if byfold {
                        hasFolding(wp, lnum, None, &raw mut lnum);
                    }
                    if lnum == wp.w_topline && do_sms {
                        let add = if wp.w_skipcol > 0 { width2 } else { width1 };
                        wp.w_skipcol += add;
                        if wp.w_skipcol >= size {
                            if lnum == (*wp.w_buffer).b_ml.ml_line_count {
                                wp.w_skipcol -= add;
                                break;
                            }
                            lnum += 1;
                        }
                    } else {
                        if lnum >= (*wp.w_buffer).b_ml.ml_line_count {
                            break;
                        }
                        lnum += 1;
                    }

                    if lnum > wp.w_topline {
                        wp.w_botline += lnum - wp.w_topline;
                        wp.w_topline = lnum;
                        wp.w_topfill = win_get_fill(wp, lnum);
                        wp.w_skipcol = 0;
                        if todo > 1 && do_sms {
                            size = linetabsize_eol(wp, wp.w_topline);
                        }
                    }
                }
                todo -= 1;
            }

            if prev_skipcol > 0 || wp.w_skipcol > 0 {
                redraw_later(wp, UPD_NOT_VALID);
            }
        } else {
            wp.w_topline += line_count;
            wp.w_botline += line_count;
        }

        wp.w_topline = wp.w_topline.min((*wp.w_buffer).b_ml.ml_line_count);
        wp.w_botline = wp
            .w_botline
            .min((*wp.w_buffer).b_ml.ml_line_count + 1);

        check_topfill(wp, false);

        hasFolding(
            wp,
            wp.w_topline,
            Some(&mut wp.w_topline),
            ptr::null_mut(),
        );

        wp.w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE);
        if wp.w_cursor.lnum < wp.w_topline {
            wp.w_cursor.lnum = wp.w_topline;
            wp.w_valid &=
                !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL);
            coladvance(wp, wp.w_curswant);
        }

        topline != wp.w_topline || botline != wp.w_botline
    }
}

/// Make sure that `curwin->w_skipcol` is valid for `'smoothscroll'`.
pub fn adjust_skipcol() {
    unsafe {
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0
            || (**curwin.as_ptr()).w_onebuf_opt.wo_sms == 0
            || (**curwin.as_ptr()).w_cursor.lnum != (**curwin.as_ptr()).w_topline
        {
            return;
        }

        let width1 = (**curwin.as_ptr()).w_view_width - win_col_off(*curwin.as_ptr());
        if width1 <= 0 {
            return;
        }
        let width2 = width1 + win_col_off2(*curwin.as_ptr());
        let so = get_scrolloff_value(*curwin.as_ptr());
        let scrolloff_cols = if so == 0 {
            0
        } else {
            i64::from(width1) + (so - 1) * i64::from(width2)
        };
        let mut scrolled = false;

        validate_cheight(*curwin.as_ptr());
        if (**curwin.as_ptr()).w_cline_height == (**curwin.as_ptr()).w_view_height
            && plines_win(*curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum, false) <= (**curwin.as_ptr()).w_view_height
        {
            reset_skipcol(*curwin.as_ptr());
            return;
        }

        validate_virtcol(*curwin.as_ptr());
        let overlap = sms_marker_overlap(*curwin.as_ptr(), (**curwin.as_ptr()).w_view_width - width2);
        while (**curwin.as_ptr()).w_skipcol > 0
            && i64::from((**curwin.as_ptr()).w_virtcol)
                < i64::from((**curwin.as_ptr()).w_skipcol + overlap) + scrolloff_cols
        {
            if (**curwin.as_ptr()).w_skipcol >= width1 + width2 {
                (**curwin.as_ptr()).w_skipcol -= width2;
            } else {
                (**curwin.as_ptr()).w_skipcol -= width1;
            }
            scrolled = true;
        }
        if scrolled {
            validate_virtcol(*curwin.as_ptr());
            redraw_later(*curwin.as_ptr(), UPD_NOT_VALID);
            return;
        }
        let mut row: c_int = 0;
        let mut col = i64::from((**curwin.as_ptr()).w_virtcol) + scrolloff_cols;

        if scrolloff_cols > 0 {
            let mut size = linetabsize_eol(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline);
            size = width1 + width2 * ((size - width1 + width2 - 1) / width2);
            while col > i64::from(size) {
                col -= i64::from(width2);
            }
        }
        col -= i64::from((**curwin.as_ptr()).w_skipcol);

        if col >= i64::from(width1) {
            col -= i64::from(width1);
            row += 1;
        }
        if col > i64::from(width2) {
            row += (col / i64::from(width2)) as c_int;
        }
        if row >= (**curwin.as_ptr()).w_view_height {
            if (**curwin.as_ptr()).w_skipcol == 0 {
                (**curwin.as_ptr()).w_skipcol += width1;
                row -= 1;
            }
            if row >= (**curwin.as_ptr()).w_view_height {
                (**curwin.as_ptr()).w_skipcol += (row - (**curwin.as_ptr()).w_view_height) * width2;
            }
            redraw_later(*curwin.as_ptr(), UPD_NOT_VALID);
        }
    }
}

/// Don't end up with too many filler lines in the window.
pub fn check_topfill(wp: *mut win_T, down: bool) {
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
pub fn scrolldown_clamp() {
    unsafe {
        let can_fill = (**curwin.as_ptr()).w_topfill < win_get_fill(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline);

        if (**curwin.as_ptr()).w_topline <= 1 && !can_fill {
            return;
        }

        validate_cursor(*curwin.as_ptr());

        let mut end_row = (**curwin.as_ptr()).w_wrow;
        if can_fill {
            end_row += 1;
        } else {
            end_row += plines_win_nofill(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline - 1, true);
        }
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 && (**curwin.as_ptr()).w_view_width != 0 {
            validate_cheight(*curwin.as_ptr());
            validate_virtcol(*curwin.as_ptr());
            end_row += (**curwin.as_ptr()).w_cline_height - 1 - (**curwin.as_ptr()).w_virtcol / (**curwin.as_ptr()).w_view_width;
        }
        if i64::from(end_row) < i64::from((**curwin.as_ptr()).w_view_height) - get_scrolloff_value(*curwin.as_ptr()) {
            if can_fill {
                (**curwin.as_ptr()).w_topfill += 1;
                check_topfill(*curwin.as_ptr(), true);
            } else {
                (**curwin.as_ptr()).w_topline -= 1;
                (**curwin.as_ptr()).w_topfill = 0;
            }
            hasFolding(
                *curwin.as_ptr(),
                (**curwin.as_ptr()).w_topline,
                Some(&mut (**curwin.as_ptr()).w_topline),
                ptr::null_mut(),
            );
            (**curwin.as_ptr()).w_botline -= 1;
            (**curwin.as_ptr()).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE);
        }
    }
}

/// Scroll the screen one line up, without moving the cursor off the screen.
pub fn scrollup_clamp() {
    unsafe {
        if (**curwin.as_ptr()).w_topline == (*g_st().curbuf).b_ml.ml_line_count && (**curwin.as_ptr()).w_topfill == 0 {
            return;
        }

        validate_cursor(*curwin.as_ptr());

        let mut start_row = (**curwin.as_ptr()).w_wrow
            - plines_win_nofill(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline, true)
            - (**curwin.as_ptr()).w_topfill;
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 && (**curwin.as_ptr()).w_view_width != 0 {
            validate_virtcol(*curwin.as_ptr());
            start_row -= (**curwin.as_ptr()).w_virtcol / (**curwin.as_ptr()).w_view_width;
        }
        if i64::from(start_row) >= get_scrolloff_value(*curwin.as_ptr()) {
            if (**curwin.as_ptr()).w_topfill > 0 {
                (**curwin.as_ptr()).w_topfill -= 1;
            } else {
                hasFolding(
                    *curwin.as_ptr(),
                    (**curwin.as_ptr()).w_topline,
                    None,
                    &raw mut (**curwin.as_ptr()).w_topline,
                );
                (**curwin.as_ptr()).w_topline += 1;
            }
            (**curwin.as_ptr()).w_botline += 1;
            (**curwin.as_ptr()).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE);
        }
    }
}

fn topline_back_winheight(wp: *mut win_T, lp: &mut lineoff_T, winheight: bool) {
    {
        if lp.fill < win_get_fill(wp, lp.lnum) {
            lp.fill += 1;
            lp.height = 1;
        } else {
            lp.lnum -= 1;
            lp.fill = 0;
            if lp.lnum < 1 {
                lp.height = MAXCOL;
            } else if hasFolding(wp, lp.lnum, Some(&mut lp.lnum), ptr::null_mut()) {
                lp.height = c_int::from(!decor_conceal_line(wp, lp.lnum - 1, false));
            } else {
                lp.height = plines_win_nofill(wp, lp.lnum, winheight);
            }
        }
    }
}

fn topline_back(wp: *mut win_T, lp: &mut lineoff_T) {
    topline_back_winheight(wp, lp, true);
}

fn botline_forw(wp: *mut win_T, lp: &mut lineoff_T) {
    unsafe {
        if lp.fill < win_get_fill(wp, lp.lnum + 1) {
            lp.fill += 1;
            lp.height = 1;
        } else {
            lp.lnum += 1;
            lp.fill = 0;
            if lp.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
                lp.height = MAXCOL;
            } else if hasFolding(wp, lp.lnum, None, &raw mut lp.lnum) {
                lp.height = c_int::from(!decor_conceal_line(wp, lp.lnum - 1, false));
            } else {
                lp.height = plines_win_nofill(wp, lp.lnum, true);
            }
        }
    }
}

/// Recompute topline to put the cursor at the top of the window.
pub fn scroll_cursor_top(wp: &mut win_T, min_scroll: c_int, always: c_int) {
    unsafe {
        let old_topline = wp.w_topline;
        let old_skipcol = wp.w_skipcol;
        let old_topfill = wp.w_topfill;
        let off = if g_st().mouse_dragging > 0 {
            i64::from(g_st().mouse_dragging - 1)
        } else {
            get_scrolloff_value(wp)
        };

        validate_cheight(wp);
        let mut scrolled = if wp.w_cursor.lnum < wp.w_topline {
            wp.w_cline_height
        } else {
            0
        };
        let mut used = wp.w_cline_height;

        let mut top: linenr_T = 0;
        let mut bot: linenr_T = 0;
        if hasFolding(wp, wp.w_cursor.lnum, Some(&mut top), &raw mut bot) {
            top -= 1;
            bot += 1;
        } else {
            top = wp.w_cursor.lnum - 1;
            bot = wp.w_cursor.lnum + 1;
        }
        let mut new_topline = top + 1;

        let mut extra = win_get_fill(wp, wp.w_cursor.lnum);

        while top > 0 {
            let i = plines_win_nofill(wp, top, true);
            hasFolding(wp, top, Some(&mut top), ptr::null_mut());
            if top < wp.w_topline {
                scrolled += i;
            }

            if (new_topline >= wp.w_topline || scrolled > min_scroll) && i64::from(extra) >= off
            {
                break;
            }

            used += i;
            if i64::from(extra + i) <= off && bot < (*wp.w_buffer).b_ml.ml_line_count {
                used += plines_win_full(wp, bot, Some(&mut bot), None, true, true);
            }
            if used > wp.w_view_height {
                break;
            }

            extra += i;
            new_topline = top;
            top -= 1;
            bot += 1;
        }

        if used > wp.w_view_height {
            scroll_cursor_halfway(wp, false, false);
        } else {
            if new_topline < wp.w_topline || always != 0 {
                wp.w_topline = new_topline;
            }
            wp.w_topline = wp.w_topline.min(wp.w_cursor.lnum);
            wp.w_topfill = win_get_fill(wp, wp.w_topline);
            if wp.w_topfill > 0 && i64::from(extra) > off {
                wp.w_topfill -= extra - off as c_int;
                wp.w_topfill = wp.w_topfill.max(0);
            }
            check_topfill(wp, false);
            if wp.w_topline != old_topline {
                reset_skipcol(wp);
            } else if wp.w_topline == wp.w_cursor.lnum {
                validate_virtcol(wp);
                if wp.w_skipcol >= wp.w_virtcol {
                    let width1 = wp.w_view_width - win_col_off(wp);
                    let width2 = width1 + win_col_off2(wp);
                    let plines_off = if width2 > 0 && wp.w_virtcol >= width1 {
                        (wp.w_virtcol - width1) / width2 + 1
                    } else {
                        0
                    };
                    let skipcol = skipcol_from_plines(wp, plines_off);
                    if skipcol != wp.w_skipcol {
                        wp.w_skipcol = skipcol;
                        redraw_later(wp, UPD_SOME_VALID);
                    }
                }
            }
            if wp.w_topline != old_topline
                || wp.w_skipcol != old_skipcol
                || wp.w_topfill != old_topfill
            {
                wp.w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
            }
            wp.w_valid |= VALID_TOPLINE;
            wp.w_viewport_invalid = true;
        }
    }
}

/// Set `w_empty_rows` and `w_filler_rows` for window `wp`.
pub fn set_empty_rows(wp: &mut win_T, used: c_int) {
    unsafe {
        wp.w_filler_rows = 0;
        if used == 0 {
            wp.w_empty_rows = 0;
        } else {
            wp.w_empty_rows = wp.w_view_height - used;
            if wp.w_botline <= (*wp.w_buffer).b_ml.ml_line_count {
                wp.w_filler_rows = win_get_fill(wp, wp.w_botline);
                if wp.w_empty_rows > wp.w_filler_rows {
                    wp.w_empty_rows -= wp.w_filler_rows;
                } else {
                    wp.w_filler_rows = wp.w_empty_rows;
                    wp.w_empty_rows = 0;
                }
            }
        }
    }
}

/// Recompute topline to put the cursor at the bottom of the window.
pub fn scroll_cursor_bot(wp: &mut win_T, min_scroll: c_int, set_topbot: bool) {
    unsafe {
        let mut loff = lineoff_T::default();
        let old_topline = wp.w_topline;
        let old_skipcol = wp.w_skipcol;
        let old_topfill = wp.w_topfill;
        let old_botline = wp.w_botline;
        let old_valid = wp.w_valid;
        let old_empty_rows = wp.w_empty_rows;
        let cln = wp.w_cursor.lnum;
        let do_sms = wp.w_onebuf_opt.wo_wrap != 0 && wp.w_onebuf_opt.wo_sms != 0;

        if set_topbot {
            let mut used: c_int = 0;
            let mut cln_last = cln;
            hasFolding(wp, cln, None, &raw mut cln_last);
            wp.w_botline = cln_last + 1;
            loff.lnum = cln_last + 1;
            loff.fill = 0;
            loop {
                topline_back_winheight(wp, &mut loff, false);
                if loff.height == MAXCOL {
                    break;
                }
                if used + loff.height > wp.w_view_height {
                    if do_sms && used < wp.w_view_height {
                        let plines_offset = used + loff.height - wp.w_view_height;
                        used = wp.w_view_height;
                        wp.w_topfill = loff.fill;
                        wp.w_topline = loff.lnum;
                        wp.w_skipcol = skipcol_from_plines(wp, plines_offset);
                    }
                    break;
                }
                wp.w_topfill = loff.fill;
                wp.w_topline = loff.lnum;
                used += loff.height;
            }

            set_empty_rows(wp, used);
            wp.w_valid |= VALID_BOTLINE | VALID_BOTLINE_AP;
            if wp.w_topline != old_topline
                || wp.w_topfill != old_topfill
                || wp.w_skipcol != old_skipcol
                || wp.w_skipcol != 0
            {
                wp.w_valid &= !(VALID_WROW | VALID_CROW);
                if wp.w_skipcol != old_skipcol {
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
        if cln >= wp.w_botline {
            scrolled = used;
            if cln == wp.w_botline {
                scrolled -= wp.w_empty_rows;
            }
            if do_sms {
                let mut top_plines = plines_win_nofill(wp, wp.w_topline, false);
                let width1 = wp.w_view_width - win_col_off(wp);

                if width1 > 0 {
                    let width2 = width1 + win_col_off2(wp);
                    let skip_lines = if wp.w_skipcol > width1 {
                        (wp.w_skipcol - width1) / width2 + 1
                    } else if wp.w_skipcol > 0 {
                        1
                    } else {
                        0
                    };

                    top_plines -= skip_lines;
                    if top_plines > wp.w_view_height {
                        scrolled += top_plines - wp.w_view_height;
                    }
                }
            }
        }

        let mut boff = lineoff_T::default();
        if !hasFolding(
            wp,
            wp.w_cursor.lnum,
            Some(&mut loff.lnum),
            &raw mut boff.lnum,
        ) {
            loff.lnum = cln;
            boff.lnum = cln;
        }
        loff.fill = 0;
        boff.fill = 0;
        let fill_below_window = win_get_fill(wp, wp.w_botline) - wp.w_filler_rows;

        let mut extra: c_int = 0;
        let so = get_scrolloff_value(wp);
        let eff_so = if g_st().mouse_dragging > 0 {
            i64::from(g_st().mouse_dragging - 1)
        } else {
            so
        };
        while loff.lnum > 1 {
            if (((scrolled <= 0 || scrolled >= min_scroll) && i64::from(extra) >= eff_so)
                || boff.lnum + 1 > (*wp.w_buffer).b_ml.ml_line_count)
                && loff.lnum <= wp.w_botline
                && (loff.lnum < wp.w_botline || loff.fill >= fill_below_window)
            {
                break;
            }

            let loff_lnum_before = loff.lnum;
            topline_back(wp, &mut loff);
            if loff.height == MAXCOL {
                used = MAXCOL;
            } else {
                used += loff.height;
            }
            if used > wp.w_view_height {
                break;
            }
            if loff.lnum >= wp.w_botline
                && (loff.lnum > wp.w_botline || loff.fill <= fill_below_window)
            {
                scrolled += loff.height;
                if loff.lnum == wp.w_botline && loff_lnum_before > (**curwin.as_ptr()).w_botline {
                    scrolled -= wp.w_empty_rows;
                }
            }

            if boff.lnum < (*wp.w_buffer).b_ml.ml_line_count {
                let boff_lnum_before = boff.lnum;
                botline_forw(wp, &mut boff);
                used += boff.height;
                if used > wp.w_view_height {
                    break;
                }
                if i64::from(extra) < eff_so || scrolled < min_scroll {
                    extra += boff.height;
                    if boff.lnum >= wp.w_botline
                        || (boff.lnum + 1 == wp.w_botline && boff.fill > wp.w_filler_rows)
                    {
                        scrolled += boff.height;
                        if boff.lnum >= (**curwin.as_ptr()).w_botline
                            && boff_lnum_before < (**curwin.as_ptr()).w_botline
                        {
                            scrolled -= wp.w_empty_rows;
                        }
                    }
                }
            }
        }

        let mut line_count: linenr_T = if scrolled <= 0 {
            0
        } else if used > wp.w_view_height {
            used
        } else {
            let mut lc = 0;
            boff.fill = wp.w_topfill;
            boff.lnum = wp.w_topline - 1;
            let mut i = 0;
            while i < scrolled && boff.lnum < wp.w_botline {
                botline_forw(wp, &mut boff);
                i += boff.height;
                lc += 1;
            }
            if i < scrolled {
                lc = 9999;
            }
            lc
        };

        let eof_pressure = scrolloffpad_eof_pressure(wp, cln, so);
        if line_count >= wp.w_view_height && line_count > min_scroll {
            scroll_cursor_halfway(wp, eof_pressure, true);
        } else if line_count > 0 {
            if do_sms {
                scrollup(wp, scrolled, true);
            } else {
                scrollup(wp, line_count, true);
            }
        }
        let _ = &mut line_count;

        if wp.w_topline == old_topline && wp.w_skipcol == old_skipcol && set_topbot {
            wp.w_botline = old_botline;
            wp.w_empty_rows = old_empty_rows;
            wp.w_valid = old_valid;
        }
        wp.w_valid |= VALID_TOPLINE;
        wp.w_viewport_invalid = true;

        if set_topbot {
            cursor_correct_sms(wp);
        }
    }
}

/// Recompute topline to put the cursor halfway across the window.
pub fn scroll_cursor_halfway(wp: &mut win_T, atend: bool, prefer_above: bool) {
    unsafe {
        let old_topline = wp.w_topline;
        let mut loff = lineoff_T {
            lnum: wp.w_cursor.lnum,
            fill: 0,
            height: 0,
        };
        let mut boff = lineoff_T {
            lnum: wp.w_cursor.lnum,
            fill: 0,
            height: 0,
        };
        hasFolding(wp, loff.lnum, Some(&mut loff.lnum), &raw mut boff.lnum);
        let mut used = plines_win_nofill(wp, loff.lnum, true);
        let mut topline = loff.lnum;
        let mut skipcol: colnr_T = 0;

        let mut want_height: c_int = 0;
        let do_sms = wp.w_onebuf_opt.wo_wrap != 0 && wp.w_onebuf_opt.wo_sms != 0;
        if do_sms {
            if atend {
                want_height = (wp.w_view_height - used) / 2;
                used = 0;
            } else {
                want_height = wp.w_view_height;
            }
        }

        let mut topfill: c_int = 0;
        let mut above: c_int = 0;
        let mut below: c_int = 0;
        while topline > 1 {
            if do_sms {
                topline_back_winheight(wp, &mut loff, false);
                if loff.height == MAXCOL {
                    break;
                }
                used += loff.height;
                if !atend && boff.lnum < (*wp.w_buffer).b_ml.ml_line_count {
                    botline_forw(wp, &mut boff);
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
                    if boff.lnum < (*wp.w_buffer).b_ml.ml_line_count {
                        botline_forw(wp, &mut boff);
                        used += boff.height;
                        if used > wp.w_view_height {
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
                    topline_back(wp, &mut loff);
                    if loff.height == MAXCOL {
                        used = MAXCOL;
                    } else {
                        used += loff.height;
                    }
                    if used > wp.w_view_height {
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

        if !hasFolding(wp, topline, Some(&mut wp.w_topline), ptr::null_mut())
            && (wp.w_topline != topline || skipcol != 0 || wp.w_skipcol != 0)
        {
            wp.w_topline = topline;
            if skipcol != 0 {
                wp.w_skipcol = skipcol;
                redraw_later(wp, UPD_NOT_VALID);
            } else if do_sms {
                reset_skipcol(wp);
            }
        }
        wp.w_topfill = topfill;
        if old_topline > wp.w_topline + wp.w_view_height {
            wp.w_botfill = false;
        }
        check_topfill(wp, false);
        wp.w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
        wp.w_valid |= VALID_TOPLINE;
    }
}

/// Correct the cursor position so that it is in a part of the screen at least
/// `'so'` lines from the top and bottom, if possible.
pub fn cursor_correct(wp: &mut win_T) {
    unsafe {
        let mut above_wanted = get_scrolloff_value(&raw mut *wp);
        let mut below_wanted = get_scrolloff_value(&raw mut *wp);
        if g_st().mouse_dragging > 0 {
            above_wanted = i64::from(g_st().mouse_dragging - 1);
            below_wanted = i64::from(g_st().mouse_dragging - 1);
        }
        if wp.w_topline == 1 {
            above_wanted = 0;
            let max_off = i64::from(wp.w_view_height / 2);
            below_wanted = below_wanted.min(max_off);
        }
        validate_botline_win(&raw mut *wp);
        if wp.w_botline == (*wp.w_buffer).b_ml.ml_line_count + 1 && g_st().mouse_dragging == 0 {
            below_wanted = 0;
            let max_off = i64::from((wp.w_view_height - 1) / 2);
            above_wanted = above_wanted.min(max_off);
        }

        let cln = wp.w_cursor.lnum;
        if i64::from(cln) >= i64::from(wp.w_topline) + above_wanted
            && i64::from(cln) < i64::from(wp.w_botline) - below_wanted
            && !win_lines_concealed(&mut *wp)
        {
            return;
        }

        let mut topline = wp.w_topline;
        let mut botline = wp.w_botline - 1;
        let mut above = wp.w_topfill;
        let mut below = wp.w_filler_rows;
        while (i64::from(above) < above_wanted || i64::from(below) < below_wanted)
            && topline < botline
        {
            if i64::from(below) < below_wanted
                && (below <= above || i64::from(above) >= above_wanted)
            {
                below += plines_win_full(&raw mut *wp, botline, None, None, true, true);
                hasFolding(&raw mut *wp, botline, Some(&mut botline), ptr::null_mut());
                botline -= 1;
            }
            if i64::from(above) < above_wanted
                && (above < below || i64::from(below) >= below_wanted)
            {
                above += plines_win_nofill(&raw mut *wp, topline, true);
                hasFolding(&raw mut *wp, topline, None, &raw mut topline);
                if topline < botline {
                    above += win_get_fill(&raw mut *wp, topline + 1);
                }
                topline += 1;
            }
        }
        if topline == botline || botline == 0 {
            wp.w_cursor.lnum = topline;
        } else if topline > botline {
            wp.w_cursor.lnum = botline;
        } else {
            if cln < topline && wp.w_topline > 1 {
                wp.w_cursor.lnum = topline;
                wp.w_valid &= !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW);
            }
            if cln > botline && wp.w_botline <= (*wp.w_buffer).b_ml.ml_line_count {
                wp.w_cursor.lnum = botline;
                wp.w_valid &= !(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW);
            }
        }
        check_cursor_moved(&raw mut *wp);
        wp.w_valid |= VALID_TOPLINE;
        wp.w_viewport_invalid = true;
    }
}

/// Decide how much overlap to use for page-up or page-down scrolling.
fn get_scroll_overlap(dir: Direction) -> c_int {
    unsafe {
        let min_height = (**curwin.as_ptr()).w_view_height - 2;

        validate_botline_win(*curwin.as_ptr());
        if (dir == BACKWARD && (**curwin.as_ptr()).w_topline == 1)
            || (dir == FORWARD && (**curwin.as_ptr()).w_botline > (*g_st().curbuf).b_ml.ml_line_count)
        {
            return min_height + 2;
        }

        let lnum = if dir == FORWARD {
            (**curwin.as_ptr()).w_botline
        } else {
            (**curwin.as_ptr()).w_topline - 1
        };
        let fill = win_get_fill(*curwin.as_ptr(), lnum + c_int::from(dir == BACKWARD))
            - if dir == FORWARD {
                (**curwin.as_ptr()).w_filler_rows
            } else {
                (**curwin.as_ptr()).w_topfill
            };
        let mut loff = lineoff_T {
            lnum,
            fill,
            height: if fill > 0 {
                1
            } else {
                plines_win_nofill(*curwin.as_ptr(), lnum, true)
            },
        };

        let h1 = loff.height;
        if h1 > min_height {
            return min_height + 2;
        }
        if dir == FORWARD {
            topline_back(*curwin.as_ptr(), &mut loff);
        } else {
            botline_forw(*curwin.as_ptr(), &mut loff);
        }

        let h2 = loff.height;
        if h2 == MAXCOL || h2 + h1 > min_height {
            return min_height + 2;
        }
        if dir == FORWARD {
            topline_back(*curwin.as_ptr(), &mut loff);
        } else {
            botline_forw(*curwin.as_ptr(), &mut loff);
        }

        let h3 = loff.height;
        if h3 == MAXCOL || h3 + h2 > min_height {
            return min_height + 2;
        }
        if dir == FORWARD {
            topline_back(*curwin.as_ptr(), &mut loff);
        } else {
            botline_forw(*curwin.as_ptr(), &mut loff);
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
fn scroll_with_sms(dir: Direction, mut count: c_int, curscount: &mut c_int) -> bool {
    unsafe {
        let prev_sms = (**curwin.as_ptr()).w_onebuf_opt.wo_sms;
        let prev_skipcol = (**curwin.as_ptr()).w_skipcol;
        let prev_topline = (**curwin.as_ptr()).w_topline;
        let prev_topfill = (**curwin.as_ptr()).w_topfill;

        (**curwin.as_ptr()).w_onebuf_opt.wo_sms = 1;
        scroll_redraw(c_int::from(dir == FORWARD), count);

        if prev_sms == 0 && (**curwin.as_ptr()).w_skipcol > 0 {
            let mut fixdir = dir;
            if ((**curwin.as_ptr()).w_topline - prev_topline).abs() > c_int::from(dir == BACKWARD) {
                fixdir = -dir;
            }

            let width1 = (**curwin.as_ptr()).w_view_width - win_col_off(*curwin.as_ptr());
            let width2 = width1 + win_col_off2(*curwin.as_ptr());
            count = 1 + ((**curwin.as_ptr()).w_skipcol - width1 - 1) / width2;
            if fixdir == FORWARD {
                count = 1
                    + (linetabsize_eol(*curwin.as_ptr(), (**curwin.as_ptr()).w_topline) - (**curwin.as_ptr()).w_skipcol - width1
                        + width2
                        - 1)
                        / width2;
            }
            scroll_redraw(c_int::from(fixdir == FORWARD), count);
            *curscount += count * if fixdir == dir { 1 } else { -1 };
        }
        (**curwin.as_ptr()).w_onebuf_opt.wo_sms = prev_sms;

        (**curwin.as_ptr()).w_topline != prev_topline
            || (**curwin.as_ptr()).w_topfill != prev_topfill
            || (**curwin.as_ptr()).w_skipcol != prev_skipcol
    }
}

/// Move screen `count` (half) pages up (`dir` is `BACKWARD`) or down (`dir` is `FORWARD`).
pub fn pagescroll(dir: Direction, mut count: c_int, half: bool) -> bool {
    unsafe {
        let mut did_move = false;
        let buflen = (*g_st().curbuf).b_ml.ml_line_count;
        let prev_col = (**curwin.as_ptr()).w_cursor.col;
        let prev_curswant = (**curwin.as_ptr()).w_curswant;
        let prev_lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let mut oa = oparg_T::new();
        let mut ca = cmdarg_T::new();
        ca.oap = &raw mut oa;

        if half {
            if count != 0 {
                (**curwin.as_ptr()).w_onebuf_opt.wo_scr = i64::from((**curwin.as_ptr()).w_view_height.min(count));
            }
            count = (**curwin.as_ptr())
                .w_view_height
                .min((**curwin.as_ptr()).w_onebuf_opt.wo_scr as c_int);

            let mut curscount = count;
            if dir == FORWARD
                && ((**curwin.as_ptr()).w_topline + (**curwin.as_ptr()).w_view_height + count > buflen
                    || win_lines_concealed(&mut **curwin.as_ptr()))
            {
                let mut n = plines_correct_topline(
                    *curwin.as_ptr(),
                    (**curwin.as_ptr()).w_topline,
                    None,
                    false,
                    None,
                );
                if n - count < (**curwin.as_ptr()).w_view_height && (**curwin.as_ptr()).w_topline < buflen {
                    n += plines_m_win(
                        &mut **curwin.as_ptr(),
                        (**curwin.as_ptr()).w_topline + 1,
                        buflen,
                        (**curwin.as_ptr()).w_view_height + count,
                    );
                }
                if n < (**curwin.as_ptr()).w_view_height + count {
                    count = n - (**curwin.as_ptr()).w_view_height;
                }
            }

            if count > 0 {
                did_move = scroll_with_sms(dir, count, &mut curscount);
                (**curwin.as_ptr()).w_cursor.lnum = prev_lnum;
                (**curwin.as_ptr()).w_cursor.col = prev_col;
                (**curwin.as_ptr()).w_curswant = prev_curswant;
            }

            if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0 {
                nv_screengo(&mut oa, dir, curscount, true);
            } else if dir == FORWARD {
                cursor_down_inner(&mut **curwin.as_ptr(), curscount, true);
            } else {
                cursor_up_inner(&mut **curwin.as_ptr(), curscount, true);
            }
        } else {
            let one_window = g_st().firstwin == g_st().lastwin;
            count *= if one_window && opt_vars().p_window > 0 && opt_vars().p_window < i64::from(g_st().Rows - 1) {
                (opt_vars().p_window as c_int - 2).max(1)
            } else {
                get_scroll_overlap(dir)
            };
            did_move = scroll_with_sms(dir, count, &mut count);

            if did_move {
                validate_botline_win(*curwin.as_ptr());
                let lnum = if dir == FORWARD {
                    (**curwin.as_ptr()).w_topline
                } else {
                    (**curwin.as_ptr()).w_botline - 1
                };
                (**curwin.as_ptr()).w_cursor.lnum = lnum.max(1);
            }
        }

        if get_scrolloff_value(*curwin.as_ptr()) > 0 {
            cursor_correct(&mut **curwin.as_ptr());
        }
        foldAdjustCursor(&mut **curwin.as_ptr());

        did_move =
            did_move || prev_col != (**curwin.as_ptr()).w_cursor.col || prev_lnum != (**curwin.as_ptr()).w_cursor.lnum;

        if !did_move {
            beep_flush();
        } else if (**curwin.as_ptr()).w_onebuf_opt.wo_sms == 0 {
            beginline(BL_SOL | BL_FIX);
        } else if opt_vars().p_sol != 0 {
            nv_g_home_m_cmd(&mut ca);
        }

        did_move
    }
}

pub fn do_check_cursorbind() {
    unsafe {
struct MoveCursorbindState {
    prev_curwin: *mut win_T,
    prev_curbuf: *mut buf_T,
    prev_changedtick: varnumber_T,
    prev_cursor: pos_T,
}

impl MoveCursorbindState {
    const fn new() -> Self {
        Self {
            prev_curwin: ptr::null_mut(),
            prev_curbuf: ptr::null_mut(),
            prev_changedtick: 0,
            prev_cursor: pos_T { lnum: 0, col: 0, coladd: 0 },
        }
    }
}

static MOVE_CB_STATE: GlobalState<MoveCursorbindState> = GlobalState::new(MoveCursorbindState::new());

#[inline]
fn move_cb_st() -> &'static mut MoveCursorbindState {
    unsafe { &mut *MOVE_CB_STATE.as_ptr() }
}

        debug_assert!(!(*curwin.as_ptr()).is_null() && !g_st().curbuf.is_null());
        if *curwin.as_ptr() == move_cb_st().prev_curwin
            && g_st().curbuf == move_cb_st().prev_curbuf
            && buf_get_changedtick(&*g_st().curbuf) == move_cb_st().prev_changedtick
            && equalpos((**curwin.as_ptr()).w_cursor, move_cb_st().prev_cursor)
        {
            return;
        }
        move_cb_st().prev_curwin = *curwin.as_ptr();
        move_cb_st().prev_curbuf = g_st().curbuf;
        move_cb_st().prev_changedtick = buf_get_changedtick(&*g_st().curbuf);
        move_cb_st().prev_cursor = (**curwin.as_ptr()).w_cursor;

        let line = (**curwin.as_ptr()).w_cursor.lnum;
        let col = (**curwin.as_ptr()).w_cursor.col;
        let coladd = (**curwin.as_ptr()).w_cursor.coladd;
        let curswant = (**curwin.as_ptr()).w_curswant;
        let set_curswant = (**curwin.as_ptr()).w_set_curswant;
        let old_curwin = *curwin.as_ptr();
        let old_curbuf = g_st().curbuf;
        let old_VIsual_select = g_st().Visual.select;
        let old_VIsual_active = g_st().Visual.active;

        g_st().Visual.select = false;
        g_st().Visual.active = false;
        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            *curwin.as_ptr() = wp;
            g_st().curbuf = (**curwin.as_ptr()).w_buffer;
            if *curwin.as_ptr() != old_curwin && (**curwin.as_ptr()).w_onebuf_opt.wo_crb != 0 {
                if (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0 {
                    (**curwin.as_ptr()).w_cursor.lnum = diff_get_corresponding_line(old_curbuf, line);
                } else {
                    (**curwin.as_ptr()).w_cursor.lnum = line;
                }
                (**curwin.as_ptr()).w_cursor.col = col;
                (**curwin.as_ptr()).w_cursor.coladd = coladd;
                (**curwin.as_ptr()).w_curswant = curswant;
                (**curwin.as_ptr()).w_set_curswant = set_curswant;

                {
                    let restart_edit_save = g_st().restart_edit;
                    g_st().restart_edit = c_int::from(true);
                    check_cursor(*curwin.as_ptr());

                    if (**curwin.as_ptr()).w_onebuf_opt.wo_scb == 0 {
                        validate_cursor(*curwin.as_ptr());
                    }

                    g_st().restart_edit = restart_edit_save;
                }
                mb_adjust_cursor();
                redraw_later(*curwin.as_ptr(), UPD_VALID);

                if (**curwin.as_ptr()).w_onebuf_opt.wo_scb == 0 {
                    update_topline(*curwin.as_ptr());
                }
                (**curwin.as_ptr()).w_redr_status = true;
            }
            wp = (*wp).w_next;
        }

        g_st().Visual.select = old_VIsual_select;
        g_st().Visual.active = old_VIsual_active;
        *curwin.as_ptr() = old_curwin;
        g_st().curbuf = old_curbuf;
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
            let mut wp_box = win_T::new();
            let wp: *mut win_T = &raw mut wp_box;
            (*wp).w_curswant = 42;
            (*wp).w_set_curswant = true;
            (*wp).w_leftcol = 4;
            (*wp).w_skipcol = 8;
            (*wp).w_topline = 10;
            (*wp).w_topfill = 2;
            (*wp).w_botline = 20;
            (*wp).w_empty_rows = 3;

            let mut vs = viewstate_T::default();
            save_viewstate(&mut *wp, &mut vs);
            let mut wp2_box = win_T::new();
            let wp2: *mut win_T = &raw mut wp2_box;
            restore_viewstate(&mut *wp2, &vs);
            assert_eq!((*wp2).w_curswant, 42);
            assert!((*wp2).w_set_curswant);
            assert_eq!((*wp2).w_leftcol, 4);
            assert_eq!((*wp2).w_skipcol, 8);
            assert_eq!((*wp2).w_topline, 10);
            assert_eq!((*wp2).w_topfill, 2);
            assert_eq!((*wp2).w_botline, 20);
            assert_eq!((*wp2).w_empty_rows, 3);
        }
    }
}
