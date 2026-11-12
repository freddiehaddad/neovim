#![deny(unsafe_code)]

//! Functions for moving the cursor and scrolling text (`src/nvim/move.c`).

use crate::buffer::{VALID_TOPLINE, buf_get_changedtick, buf_is_empty};
use crate::cursor::{
    Colnr, CursorPos, Linenr, VALID_VIRTCOL, check_cursor, check_cursor_lnum, coladvance, equalpos,
};
use crate::decoration::{decor_conceal_line, win_lines_concealed};
use crate::diff::diff_get_corresponding_line;
use crate::drawscreen::{
    SIGN_WIDTH, UPD_INVERTED, UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, VALID_BOTLINE,
    conceal_check_visual, conceal_cursor_line, conceal_reveal_changed, number_width,
    redraw_buf_later, redraw_later, redraw_winline, redrawing, win_cursorline_standout,
    win_scroll_lines,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypVal, Varnumber, tv_check_for_number_arg, tv_dict_add_nr, tv_dict_alloc_ret,
    tv_get_number, tv_get_number_chk,
};
use crate::eval::window::{Viewstate, find_win_by_nr_or_id_opt};
use crate::fold::{fold_adjust_cursor, has_folding};
use crate::globals::*;
use crate::input::beep_flush;
use crate::input_cmdatom::Cmdarg;
use crate::insert::{beginline, cursor_down, cursor_down_inner, cursor_up, cursor_up_inner};
use crate::mbyte::mb_adjust_cursor;
use crate::message::semsg;
use crate::mouse::vcol2col;
use crate::normal::{nv_g_home_m_cmd, nv_screengo};
use crate::option::{
    OptInt, get_scrolloff_value, get_scrolloffpad_value, get_showbreak_value,
    get_sidescrolloff_value,
};
use crate::option_vars::*;
use crate::plines::{
    GETVCOL_DISP, disp_vcol2vcol, getvcol, getvvcol, has_extconceal, line_has_extconceal,
    linetabsize_disp_eol, plines_m_win, plines_win, plines_win_full, plines_win_nofill,
    win_get_fill, win_may_fill,
};
use crate::strings::tr;

use crate::window::{VALID_BOTLINE_AP, WinInfoInfo, win_fdccol_count};
use crate::winfloat::win_check_anchored_floats;

use crate::drawline::{
    K_CPO_NUMCOL, K_OPT_CULOPT_FLAG_SCREENLINE, VALID_CHEIGHT, VALID_CROW, VALID_WCOL, VALID_WROW,
};
use crate::ex_cmds::{BL_FIX, BL_SOL};
use crate::regexp::MAXCOL;
use crate::search::{BACKWARD, Direction, FORWARD};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Lineoff {
    lnum: Linenr,
    fill: i32,
    height: i32,
}

/// Get the number of screen lines skipped with `wp->w_skipcol`.
fn adjust_plines_for_skipcol(wp: &WinInfoInfo) -> i32 {
    if wp.w_skipcol.get() == 0 {
        return 0;
    }

    let width = wp.w_view_width.get() - win_col_off(wp);
    let w2 = width + win_col_off2(wp);
    if wp.w_skipcol.get() >= width && w2 > 0 {
        return (wp.w_skipcol.get() - width) / w2 + 1;
    }

    0
}

/// Return how many lines `lnum` will take on the screen, taking into account
/// whether it is the first line, whether `w_skipcol` is non-zero and limiting
/// to the window height.
pub fn plines_correct_topline(
    wp: &WinInfoInfo,
    lnum: Linenr,
    nextp: Option<&mut Linenr>,
    limit_winheight: bool,
    foldedp: Option<&mut bool>,
) -> i32 {
    let mut n = plines_win_full(
        wp,
        lnum,
        (nextp.is_some()).then(|| &mut *nextp.unwrap()),
        (foldedp.is_some()).then(|| &mut *foldedp.unwrap()),
        true,
        false,
    );
    if lnum == wp.w_topline.get() {
        n -= adjust_plines_for_skipcol(wp);
    }
    if limit_winheight && n > wp.w_view_height.get() {
        return wp.w_view_height.get();
    }
    n
}

/// Compute `wp->w_botline` for the current `wp->w_topline`.
fn comp_botline(wp: &WinInfoInfo) {
    check_cursor_moved(wp);

    let use_cache = redrawing()
        && !wp.buf_mut().b_mod_set.get()
        && g_st().dollar_vcol.get() == -1
        && wp.w_skipcol.get() == 0
        && !win_may_fill(wp)
        && wp.w_lines_valid.get() > 0
        && (&wp.w_lines.borrow())[0].wl_lnum <= wp.w_topline.get();

    let (mut lnum, mut done) = if (wp.w_valid.get() & VALID_CROW) != 0 {
        (wp.w_cursor.get().lnum, wp.w_cline_row.get())
    } else {
        (wp.w_topline.get(), 0)
    };

    let mut i: i32 = 0;
    if use_cache {
        while i < wp.w_lines_valid.get() && (&wp.w_lines.borrow())[i as usize].wl_lnum < lnum {
            i += 1;
        }
    }

    while lnum <= wp.buf_mut().b_ml.ml_line_count.get() {
        let mut valid = false;
        if use_cache && i < wp.w_lines_valid.get() {
            let wl = (&wp.w_lines.borrow())[i as usize];
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
            let wl = (&wp.w_lines.borrow())[i as usize];
            folded = wl.wl_folded;
            last = wl.wl_lastlnum;
            i32::from(wl.wl_size)
        } else {
            plines_correct_topline(wp, lnum, Some(&mut last), true, Some(&mut folded))
        };
        if lnum <= wp.w_cursor.get().lnum && last >= wp.w_cursor.get().lnum {
            wp.w_cline_row.set(done);
            wp.w_cline_height.set(n);
            wp.w_cline_folded.set(folded);
            redraw_for_cursorline(wp);
            wp.w_valid
                .set(wp.w_valid.get() | (VALID_CROW | VALID_CHEIGHT));
        }
        if done + n > wp.w_view_height.get() {
            break;
        }
        done += n;
        lnum = last + 1;
        i += 1;
    }

    wp.w_botline.set(lnum);
    wp.w_valid
        .set(wp.w_valid.get() | (VALID_BOTLINE | VALID_BOTLINE_AP));
    wp.w_viewport_invalid.set(true);

    set_empty_rows(wp, done);

    win_check_anchored_floats(wp);
}

/// Redraw when `w_cline_row` changes and `'relativenumber'` or `'cursorline'` is set.
fn redraw_for_cursorline(wp: &WinInfoInfo) {
    if (wp.w_valid.get() & VALID_CROW) != 0 {
        return;
    }

    if wp.w_onebuf_opt.wo_rnu.get() || win_cursorline_standout(wp) {
        redraw_later(Some(wp), UPD_VALID);
    }
}

/// Redraw when `'concealcursor'` is active, or when `w_virtcol` changes.
fn redraw_for_cursorcolumn(wp: &WinInfoInfo) {
    if (std::ptr::from_ref(wp).cast_mut()) == curwin_ptr()
        && wp.w_onebuf_opt.wo_cole.get() > 0
        && conceal_cursor_line(wp)
    {
        redraw_winline(wp, wp.w_cursor.get().lnum);
    }

    if (wp.w_valid.get() & VALID_VIRTCOL) != 0 {
        return;
    }

    if wp.w_onebuf_opt.wo_cuc.get() {
        redraw_later(Some(wp), UPD_SOME_VALID);
    } else if wp.w_onebuf_opt.wo_cul.get()
        && (wp.w_p_culopt_flags.get() & K_OPT_CULOPT_FLAG_SCREENLINE) != 0
    {
        redraw_later(Some(wp), UPD_VALID);
    }

    if g_st().visual_global.borrow().active && wp.w_buffer.get() == g_st().curbuf.get() {
        redraw_buf_later(&curbuf_mut(), UPD_INVERTED);
    }
}

/// Set `wp->w_virtcol` to a value (`vcol`) that is already valid.
pub fn set_valid_virtcol(wp: &WinInfoInfo, vcol: Colnr) {
    wp.w_virtcol.set(vcol);
    redraw_for_cursorcolumn(wp);
    wp.w_valid.set(wp.w_valid.get() | (VALID_VIRTCOL));
}

/// Calculates how much the `'listchars'` `"precedes"` or `'smoothscroll'` `"<<<"`
/// marker overlaps with buffer text for window `wp`.
pub fn sms_marker_overlap(wp: &WinInfoInfo, mut extra2: i32) -> i32 {
    if extra2 == -1 {
        extra2 = win_col_off(wp) - win_col_off2(wp);
    }
    if !get_showbreak_value(wp).is_empty() {
        return 0;
    }

    if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().prec != 0 {
        return 1;
    }

    if extra2 > 3 { 0 } else { 3 - extra2 }
}

/// Calculates the skipcol offset for window `wp` given how many physical lines
/// we want to scroll down.
fn skipcol_from_plines(wp: &WinInfoInfo, plines_off: i32) -> i32 {
    let width1 = wp.w_view_width.get() - win_col_off(wp);

    let mut skipcol: i32 = 0;
    if plines_off > 0 {
        skipcol += width1;
    }
    if plines_off > 1 {
        skipcol += (width1 + win_col_off2(wp)) * (plines_off - 1);
    }
    skipcol
}

/// Set `wp->w_skipcol` to zero and redraw later if needed.
fn reset_skipcol(wp: &WinInfoInfo) {
    if wp.w_skipcol.get() == 0 {
        return;
    }

    wp.w_skipcol.set(0);
    redraw_later(Some(wp), UPD_SOME_VALID);
}

/// Return true when `'scrolloffpad'` may augment `'scrolloff'`.
fn use_scrolloffpad(wp: &WinInfoInfo) -> bool {
    get_scrolloff_value(wp) > 0 && get_scrolloffpad_value(wp) > 0
}

/// Return true when there are not enough real buffer lines below `lnum` to
/// satisfy the requested `so` context.
fn scrolloffpad_eof_pressure(wp: &WinInfoInfo, lnum: Linenr, so: OptInt) -> bool {
    if !use_scrolloffpad(wp) || so <= 0 {
        return false;
    }

    i64::from(lnum) > i64::from(wp.buf_mut().b_ml.ml_line_count.get()) - so
}

/// Update `wp->w_topline` to move the cursor onto the screen.
pub fn update_topline(wp: &WinInfoInfo) {
    let mut check_botline = false;
    let use_win_so = wp.w_onebuf_opt.wo_so.get() >= 0;
    let save_so = if use_win_so {
        wp.w_onebuf_opt.wo_so.get()
    } else {
        opt_vars().p_so.get()
    };

    if g_st().skip_update_topline.get() {
        return;
    }

    if !crate::grid::grid_allocated(&g_st().default_grid) || wp.w_view_height.get() == 0 {
        check_cursor_lnum(wp);
        wp.w_topline.set(wp.w_cursor.get().lnum);
        wp.w_botline.set(wp.w_topline.get());
        wp.w_viewport_invalid.set(true);
        wp.w_scbind_pos.set(1);
        return;
    }

    check_cursor_moved(wp);
    if (wp.w_valid.get() & VALID_TOPLINE) != 0 {
        return;
    }

    let so = if g_st().mouse_dragging.get() > 0 {
        let drag_so = i64::from(g_st().mouse_dragging.get() - 1);
        if use_win_so {
            wp.w_onebuf_opt.wo_so.set(drag_so);
        } else {
            opt_vars().p_so.set(drag_so);
        }
        drag_so
    } else {
        save_so
    };
    let eof_pressure = scrolloffpad_eof_pressure(wp, wp.w_cursor.get().lnum, so);

    let old_topline = wp.w_topline.get();
    let old_topfill = wp.w_topfill.get();

    if buf_is_empty(&wp.buf_mut()) {
        if wp.w_topline.get() != 1 {
            redraw_later(Some(wp), UPD_NOT_VALID);
        }
        wp.w_topline.set(1);
        wp.w_botline.set(2);
        wp.w_skipcol.set(0);
        wp.w_valid
            .set(wp.w_valid.get() | (VALID_BOTLINE | VALID_BOTLINE_AP));
        wp.w_viewport_invalid.set(true);
        wp.w_scbind_pos.set(1);
    } else {
        let mut check_topline = false;
        if wp.w_topline.get() > 1 || wp.w_skipcol.get() > 0 {
            if wp.w_cursor.get().lnum < wp.w_topline.get() || check_top_offset(wp) {
                check_topline = true;
            } else if wp.w_skipcol.get() > 0 && wp.w_cursor.get().lnum == wp.w_topline.get() {
                let mut vcol: Colnr = 0;
                let mut cursor = wp.w_cursor.get();
                getvvcol(wp, &mut cursor, Some(&mut vcol), None, None, GETVCOL_DISP);
                wp.w_cursor.set(cursor);
                let overlap = sms_marker_overlap(wp, -1);
                if wp.w_skipcol.get() + overlap > vcol {
                    check_topline = true;
                }
            }
        }
        if !check_topline && wp.w_topfill.get() > win_get_fill(wp, wp.w_topline.get()) {
            check_topline = true;
        }

        if check_topline {
            let mut halfheight = wp.w_view_height.get() / 2 - 1;
            if halfheight < 2 {
                halfheight = 2;
            }
            let mut n: i64;
            if win_lines_concealed(wp) {
                n = 0;
                let mut lnum = wp.w_cursor.get().lnum;
                while i64::from(lnum) < i64::from(wp.w_topline.get()) + so {
                    if lnum >= wp.buf_mut().b_ml.ml_line_count.get() {
                        break;
                    }
                    n += i64::from(!decor_conceal_line(wp, lnum, false));
                    if n >= i64::from(halfheight) {
                        break;
                    }
                    has_folding(wp, lnum, None, Some(&mut lnum));
                    lnum += 1;
                }
            } else {
                n = i64::from(wp.w_topline.get()) + so - i64::from(wp.w_cursor.get().lnum);
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
            let mut topline = wp.w_topline.get();
            has_folding(wp, topline, Some(&mut topline), None);
            wp.w_topline.set(topline);
            check_botline = true;
        }
    }

    if check_botline {
        if (wp.w_valid.get() & VALID_BOTLINE_AP) == 0 {
            validate_botline_win(wp);
        }

        if wp.w_botline.get() <= wp.buf_mut().b_ml.ml_line_count.get() || use_scrolloffpad(wp) {
            if wp.w_cursor.get().lnum < wp.w_botline.get() {
                if i64::from(wp.w_cursor.get().lnum) >= i64::from(wp.w_botline.get()) - so
                    || win_lines_concealed(wp)
                {
                    let mut loff = Lineoff {
                        lnum: wp.w_cursor.get().lnum,
                        fill: 0,
                        height: 0,
                    };
                    let mut n = if eof_pressure {
                        0
                    } else {
                        wp.w_empty_rows.get()
                    };
                    has_folding(wp, loff.lnum, None, Some(&mut loff.lnum));
                    n += wp.w_filler_rows.get();
                    while loff.lnum < wp.w_botline.get()
                        && (loff.lnum + 1 < wp.w_botline.get() || loff.fill == 0)
                    {
                        n += loff.height;
                        if i64::from(n) >= so {
                            break;
                        }
                        botline_forw(wp, &mut loff);
                    }
                    if i64::from(n) >= so && !eof_pressure {
                        check_botline = false;
                    }
                } else {
                    check_botline = false;
                }
            }
            if check_botline {
                let mut n: i64 = 0;
                if win_lines_concealed(wp) {
                    let mut lnum = wp.w_cursor.get().lnum;
                    while i64::from(lnum) >= i64::from(wp.w_botline.get()) - so {
                        if lnum <= 0 {
                            break;
                        }
                        n += i64::from(!decor_conceal_line(wp, lnum - 1, false));
                        if n > i64::from(wp.w_view_height.get() + 1) {
                            break;
                        }
                        has_folding(wp, lnum, Some(&mut lnum), None);
                        lnum -= 1;
                    }
                } else {
                    n = i64::from(wp.w_cursor.get().lnum) - i64::from(wp.w_botline.get()) + 1 + so;
                }
                if n <= i64::from(wp.w_view_height.get() + 1) {
                    if eof_pressure {
                        scroll_cursor_halfway(wp, true, true);
                    } else {
                        let sj = scrolljump_value(wp);
                        scroll_cursor_bot(wp, sj, false);
                    }
                } else {
                    scroll_cursor_halfway(wp, eof_pressure, eof_pressure);
                }
            }
        }
    }
    wp.w_valid.set(wp.w_valid.get() | (VALID_TOPLINE));
    wp.w_viewport_invalid.set(true);
    win_check_anchored_floats(wp);

    if wp.w_topline.get() != old_topline || wp.w_topfill.get() != old_topfill {
        g_st().dollar_vcol.set(-1);
        redraw_later(Some(wp), UPD_VALID);

        if !wp.w_onebuf_opt.wo_sms.get() {
            reset_skipcol(wp);
        } else if wp.w_skipcol.get() != 0 {
            redraw_later(Some(wp), UPD_SOME_VALID);
        }

        if wp.w_cursor.get().lnum == wp.w_topline.get() {
            validate_cursor(wp);
        }
    }

    if use_win_so {
        wp.w_onebuf_opt.wo_so.set(save_so);
    } else {
        opt_vars().p_so.set(save_so);
    }
}

/// Return the scrolljump value to use for the window `wp`.
fn scrolljump_value(wp: &WinInfoInfo) -> i32 {
    if opt_vars().p_sj.get() >= 0 {
        opt_vars().p_sj.get() as i32
    } else {
        (wp.w_view_height.get() * (-opt_vars().p_sj.get() as i32)) / 100
    }
}

/// Return true when there are not `'scrolloff'` lines above the cursor for window `wp`.
fn check_top_offset(wp: &WinInfoInfo) -> bool {
    let so = get_scrolloff_value(wp);
    if i64::from(wp.w_cursor.get().lnum) < i64::from(wp.w_topline.get()) + so
        || win_lines_concealed(wp)
    {
        let mut loff = Lineoff {
            lnum: wp.w_cursor.get().lnum,
            fill: 0,
            height: 0,
        };
        let mut n = i64::from(wp.w_topfill.get());
        while n < so {
            topline_back(wp, &mut loff);
            if loff.lnum < wp.w_topline.get() || (loff.lnum == wp.w_topline.get() && loff.fill > 0)
            {
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

/// Update `w_curswant`.
pub fn update_curswant_force() {
    validate_virtcol(&curwin_mut());
    curwin_mut().w_curswant.set(curwin_mut().w_virtcol.get());
    curwin_mut().w_set_curswant.set(false);
}

/// Update `w_curswant` if `w_set_curswant` is set.
pub fn update_curswant() {
    if curwin_mut().w_set_curswant.get() {
        update_curswant_force();
    }
}

/// Check if the cursor has moved. Set the `w_valid` flag accordingly.
pub fn check_cursor_moved(wp: &WinInfoInfo) {
    if (std::ptr::from_ref(wp).cast_mut()) == curwin_ptr() {
        conceal_check_visual();
    }
    if wp.w_cursor.get().lnum != wp.w_valid_cursor.get().lnum {
        wp.w_valid.set(
            wp.w_valid.get()
                & (!(VALID_WROW
                    | VALID_WCOL
                    | VALID_VIRTCOL
                    | VALID_CHEIGHT
                    | VALID_CROW
                    | VALID_TOPLINE)),
        );

        if (std::ptr::from_ref(wp).cast_mut()) == curwin_ptr()
            && wp.w_valid_cursor.get().lnum > 0
            && wp.w_onebuf_opt.wo_cole.get() >= 1
            && !conceal_cursor_line(wp)
        {
            let old = wp.w_valid_cursor.get().lnum;
            let new = wp.w_cursor.get().lnum;
            if decor_conceal_line(wp, new - 1, true) || decor_conceal_line(wp, old - 1, true) {
                changed_window_setting(wp);
            } else if has_extconceal(wp) {
                conceal_reveal_changed(wp, old, old);
                conceal_reveal_changed(wp, new, new);
            }
        }
        wp.w_valid_cursor.set(wp.w_cursor.get());
        wp.w_valid_leftcol.set(wp.w_leftcol.get());
        wp.w_valid_skipcol.set(wp.w_skipcol.get());
        wp.w_viewport_invalid.set(true);
    } else if wp.w_skipcol.get() != wp.w_valid_skipcol.get() {
        wp.w_valid.set(
            wp.w_valid.get()
                & !(VALID_WROW
                    | VALID_WCOL
                    | VALID_VIRTCOL
                    | VALID_CHEIGHT
                    | VALID_CROW
                    | VALID_BOTLINE
                    | VALID_BOTLINE_AP),
        );
        wp.w_valid_cursor.set(wp.w_cursor.get());
        wp.w_valid_leftcol.set(wp.w_leftcol.get());
        wp.w_valid_skipcol.set(wp.w_skipcol.get());
    } else if wp.w_cursor.get().col != wp.w_valid_cursor.get().col
        || wp.w_leftcol.get() != wp.w_valid_leftcol.get()
        || wp.w_cursor.get().coladd != wp.w_valid_cursor.get().coladd
    {
        wp.w_valid
            .set(wp.w_valid.get() & (!(VALID_WROW | VALID_WCOL | VALID_VIRTCOL)));
        wp.w_valid_cursor.set_col(wp.w_cursor.get().col);
        wp.w_valid_leftcol.set(wp.w_leftcol.get());
        wp.w_valid_cursor.set_coladd(wp.w_cursor.get().coladd);
        wp.w_viewport_invalid.set(true);
    }
}

/// Call this function when some window settings have changed.
pub fn changed_window_setting(wp: &WinInfoInfo) {
    wp.w_lines_valid.set(0);
    changed_line_abv_curs_win(wp);
    wp.w_valid
        .set(wp.w_valid.get() & (!(VALID_BOTLINE | VALID_BOTLINE_AP | VALID_TOPLINE)));
    redraw_later(Some(wp), UPD_NOT_VALID);
}

/// Call `changed_window_setting()` for every window.
pub fn changed_window_setting_all() {
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_mut(&tp) {
            changed_window_setting(&wp);
        }
    }
}

/// Set `wp->w_topline` to a certain number.
pub fn set_topline(wp: &WinInfoInfo, mut lnum: Linenr) {
    let prev_topline = wp.w_topline.get();

    has_folding(wp, lnum, Some(&mut lnum), None);
    wp.w_botline
        .set(wp.w_botline.get() + (lnum - wp.w_topline.get()));
    if wp.w_botline.get() > wp.buf_mut().b_ml.ml_line_count.get() + 1 {
        wp.w_botline.set(wp.buf_mut().b_ml.ml_line_count.get() + 1);
    }
    wp.w_topline.set(lnum);
    wp.w_topline_was_set.set(true);
    if lnum != prev_topline {
        wp.w_topfill.set(0);
    }
    wp.w_valid
        .set(wp.w_valid.get() & (!(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_TOPLINE)));
    redraw_later(Some(wp), UPD_VALID);
}

/// Saves the view of `wp`, for `restore_viewstate()`.
pub fn save_viewstate(wp: &WinInfoInfo, vs: &mut Viewstate) {
    vs.vs_curswant = wp.w_curswant.get();
    vs.vs_set_curswant = wp.w_set_curswant.get();
    vs.vs_leftcol = wp.w_leftcol.get();
    vs.vs_skipcol = wp.w_skipcol.get();
    vs.vs_topline = wp.w_topline.get();
    vs.vs_topfill = wp.w_topfill.get();
    vs.vs_botline = wp.w_botline.get();
    vs.vs_empty_rows = wp.w_empty_rows.get();
}

/// Restores the view saved by `save_viewstate()`. No validation.
pub fn restore_viewstate(wp: &WinInfoInfo, vs: &Viewstate) {
    wp.w_curswant.set(vs.vs_curswant);
    wp.w_set_curswant.set(vs.vs_set_curswant);
    wp.w_leftcol.set(vs.vs_leftcol);
    wp.w_skipcol.set(vs.vs_skipcol);
    wp.w_topline.set(vs.vs_topline);
    wp.w_topfill.set(vs.vs_topfill);
    wp.w_botline.set(vs.vs_botline);
    wp.w_empty_rows.set(vs.vs_empty_rows);
}

/// Call this function when the length of the cursor line (in screen
/// characters) has changed, and the change is before the cursor.
pub fn changed_cline_bef_curs(wp: &WinInfoInfo) {
    wp.w_valid.set(
        wp.w_valid.get()
            & (!(VALID_WROW
                | VALID_WCOL
                | VALID_VIRTCOL
                | VALID_CROW
                | VALID_CHEIGHT
                | VALID_TOPLINE)),
    );
}

/// Call this function when the length of a line above the cursor has changed.
pub fn changed_line_abv_curs() {
    curwin_mut().w_valid.set(
        curwin_mut().w_valid.get()
            & (!(VALID_WROW
                | VALID_WCOL
                | VALID_VIRTCOL
                | VALID_CROW
                | VALID_CHEIGHT
                | VALID_TOPLINE)),
    );
}

pub fn changed_line_abv_curs_win(wp: &WinInfoInfo) {
    wp.w_valid.set(
        wp.w_valid.get()
            & (!(VALID_WROW
                | VALID_WCOL
                | VALID_VIRTCOL
                | VALID_CROW
                | VALID_CHEIGHT
                | VALID_TOPLINE)),
    );
}

/// Make sure the value of `wp->w_botline` is valid.
pub fn validate_botline_win(wp: &WinInfoInfo) {
    if (wp.w_valid.get() & VALID_BOTLINE) == 0 {
        comp_botline(wp);
    }
}

/// Mark `wp->w_botline` as invalid.
pub fn invalidate_botline_win(wp: &WinInfoInfo) {
    wp.w_valid
        .set(wp.w_valid.get() & (!(VALID_BOTLINE | VALID_BOTLINE_AP)));
}

pub fn approximate_botline_win(wp: &WinInfoInfo) {
    wp.w_valid.set(wp.w_valid.get() & (!VALID_BOTLINE));
}

/// Return true if `wp->w_wrow` and `wp->w_wcol` are valid.
pub fn cursor_valid(wp: &WinInfoInfo) -> i32 {
    check_cursor_moved(wp);
    i32::from((wp.w_valid.get() & (VALID_WROW | VALID_WCOL)) == (VALID_WROW | VALID_WCOL))
}

/// Validate cursor position. Makes sure `w_wrow` and `w_wcol` are valid.
pub fn validate_cursor(wp: &WinInfoInfo) {
    check_cursor_lnum(wp);
    check_cursor_moved(wp);
    if (wp.w_valid.get() & (VALID_WCOL | VALID_WROW)) != (VALID_WCOL | VALID_WROW) {
        curs_columns(wp, true);
    }
}

/// Compute `wp->w_cline_row` and `wp->w_cline_height`, based on `wp->w_topline`.
fn curs_rows(wp: &WinInfoInfo) {
    let all_invalid = !redrawing()
        || wp.w_lines_valid.get() == 0
        || (&wp.w_lines.borrow())[0].wl_lnum > wp.w_topline.get();
    let mut i: i32 = 0;
    wp.w_cline_row.set(0);
    let mut lnum = wp.w_topline.get();
    while lnum < wp.w_cursor.get().lnum {
        let mut valid = false;
        if !all_invalid && i < wp.w_lines_valid.get() {
            let wl = (&wp.w_lines.borrow())[i as usize];
            if wl.wl_lnum < lnum || !wl.wl_valid {
                i += 1;
                continue;
            }
            if wl.wl_lnum == lnum {
                if !wp.buf_mut().b_mod_set.get()
                    || wl.wl_lastlnum < wp.w_cursor.get().lnum
                    || wp.buf_mut().b_mod_top.get() > wl.wl_lastlnum + 1
                {
                    valid = true;
                }
            } else if wl.wl_lnum > lnum {
                i -= 1;
            }
        }
        if valid && (lnum != wp.w_topline.get() || (wp.w_skipcol.get() == 0 && !win_may_fill(wp))) {
            let wl = (&wp.w_lines.borrow())[i as usize];
            lnum = wl.wl_lastlnum + 1;
            if lnum > wp.w_cursor.get().lnum {
                i += 1;
                break;
            }
            wp.w_cline_row
                .set(wp.w_cline_row.get() + (i32::from(wl.wl_size)));
        } else {
            let mut last = lnum;
            let mut folded = false;
            let n = plines_correct_topline(wp, lnum, Some(&mut last), true, Some(&mut folded));
            lnum = last + 1;
            if lnum + i32::from(decor_conceal_line(wp, lnum - 1, false)) > wp.w_cursor.get().lnum {
                i += 1;
                break;
            }
            wp.w_cline_row.set(wp.w_cline_row.get() + (n));
        }
        i += 1;
    }

    check_cursor_moved(wp);
    if (wp.w_valid.get() & VALID_CHEIGHT) == 0 {
        if all_invalid
            || i == wp.w_lines_valid.get()
            || (i < wp.w_lines_valid.get()
                && (!(&wp.w_lines.borrow())[i as usize].wl_valid
                    || (&wp.w_lines.borrow())[i as usize].wl_lnum != wp.w_cursor.get().lnum))
        {
            let mut cline_folded = wp.w_cline_folded.get();
            let cline_height = plines_win_full(
                wp,
                wp.w_cursor.get().lnum,
                None,
                Some(&mut cline_folded),
                true,
                true,
            );
            wp.w_cline_folded.set(cline_folded);
            wp.w_cline_height.set(cline_height);
        } else if i > wp.w_lines_valid.get() {
            wp.w_cline_height.set(0);
            let cursor_lnum = wp.w_cursor.get().lnum;
            let folded = has_folding(wp, cursor_lnum, None, None);
            wp.w_cline_folded.set(folded);
        } else {
            let wl = (&wp.w_lines.borrow())[i as usize];
            wp.w_cline_height.set(i32::from(wl.wl_size));
            wp.w_cline_folded.set(wl.wl_folded);
        }
    }

    redraw_for_cursorline(wp);
    wp.w_valid
        .set(wp.w_valid.get() | (VALID_CROW | VALID_CHEIGHT));
}

/// Validate `wp->w_virtcol` only.
pub fn validate_virtcol(wp: &WinInfoInfo) {
    check_cursor_moved(wp);

    if (wp.w_valid.get() & VALID_VIRTCOL) != 0 {
        return;
    }

    let mut cursor = wp.w_cursor.get();
    let mut virtcol = wp.w_virtcol.get();
    getvvcol(wp, &mut cursor, None, Some(&mut virtcol), None, 0);
    wp.w_cursor.set(cursor);
    wp.w_virtcol.set(virtcol);
    redraw_for_cursorcolumn(wp);
    wp.w_valid.set(wp.w_valid.get() | (VALID_VIRTCOL));
}

/// Validate `wp->w_cline_height` only.
pub fn validate_cheight(wp: &WinInfoInfo) {
    check_cursor_moved(wp);

    if (wp.w_valid.get() & VALID_CHEIGHT) != 0 {
        return;
    }

    let cursor_lnum = wp.w_cursor.get().lnum;
    let mut cline_folded = wp.w_cline_folded.get();
    let ch = plines_win_full(wp, cursor_lnum, None, Some(&mut cline_folded), true, true);
    wp.w_cline_height.set(ch);
    wp.w_cline_folded.set(cline_folded);
    wp.w_valid.set(wp.w_valid.get() | (VALID_CHEIGHT));
}

/// Cursor vcol, displayed (see CharsizeArg.extconceal_off). Requires valid `w_virtcol`.
pub fn cursor_disp_vcol(wp: &WinInfoInfo) -> Colnr {
    if !line_has_extconceal(wp, wp.w_cursor.get().lnum) {
        return wp.w_virtcol.get();
    }
    let mut vcol: Colnr = 0;
    let mut cursor = wp.w_cursor.get();
    getvvcol(wp, &mut cursor, None, Some(&mut vcol), None, GETVCOL_DISP);
    wp.w_cursor.set(cursor);
    vcol
}

/// Validate `w_wcol` and `w_virtcol` only.
pub fn validate_cursor_col(wp: &WinInfoInfo) {
    validate_virtcol(wp);

    if (wp.w_valid.get() & VALID_WCOL) != 0 {
        return;
    }

    let mut col = cursor_disp_vcol(wp);
    let off = win_col_off(wp);
    col += off;
    let width = wp.w_view_width.get() - off + win_col_off2(wp);

    if wp.w_onebuf_opt.wo_wrap.get() && col >= wp.w_view_width.get() && width > 0 {
        col -= ((col - wp.w_view_width.get()) / width + 1) * width;
    }
    if col > wp.w_leftcol.get() {
        col -= wp.w_leftcol.get();
    } else {
        col = 0;
    }
    wp.w_wcol.set(col);

    wp.w_valid.set(wp.w_valid.get() | (VALID_WCOL));
}

/// Compute offset of a window, occupied by absolute or relative line number,
/// fold column and sign column.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
#[allow(improper_ctypes_definitions)]
pub extern "C" fn win_col_off(wp: &WinInfoInfo) -> i32 {
    let num_off = if wp.w_onebuf_opt.wo_nu.get()
        || wp.w_onebuf_opt.wo_rnu.get()
        || !wp.w_onebuf_opt.wo_stc.is_empty()
    {
        number_width(wp) + i32::from(wp.w_onebuf_opt.wo_stc.is_empty())
    } else {
        0
    };
    num_off + win_fdccol_count(wp) + (wp.w_scwidth.get() * SIGN_WIDTH)
}

/// Return the difference in column offset for the second screen line of a
/// wrapped line.
pub fn win_col_off2(wp: &WinInfoInfo) -> i32 {
    if (wp.w_onebuf_opt.wo_nu.get()
        || wp.w_onebuf_opt.wo_rnu.get()
        || !wp.w_onebuf_opt.wo_stc.is_empty())
        && crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_NUMCOL)
    {
        return number_width(wp) + i32::from(wp.w_onebuf_opt.wo_stc.is_empty());
    }
    0
}

/// Compute `wp->w_wcol` and `wp->w_virtcol`, and update `wp->w_wrow`,
/// `wp->w_cline_row`, and `wp->w_leftcol`.
pub fn curs_columns(wp: &WinInfoInfo, may_scroll: bool) {
    let mut startcol: Colnr = 0;
    let mut endcol: Colnr = 0;

    update_topline(wp);

    if (wp.w_valid.get() & VALID_CROW) == 0 {
        curs_rows(wp);
    }

    let disp_virtcol: Colnr;
    if wp.w_cline_folded.get() {
        startcol = wp.w_leftcol.get();
        wp.w_virtcol.set(startcol);
        endcol = startcol;
        disp_virtcol = startcol;
    } else {
        let mut cursor = wp.w_cursor.get();
        let mut virtcol = wp.w_virtcol.get();
        getvvcol(
            wp,
            &mut cursor,
            Some(&mut startcol),
            Some(&mut virtcol),
            Some(&mut endcol),
            0,
        );
        wp.w_cursor.set(cursor);
        wp.w_virtcol.set(virtcol);
        disp_virtcol = cursor_disp_vcol(wp);
    }

    if startcol > g_st().dollar_vcol.get() {
        g_st().dollar_vcol.set(-1);
    }

    let mut extra = win_col_off(wp);
    wp.w_wcol.set(disp_virtcol + extra);
    endcol += extra;

    wp.w_wrow.set(wp.w_cline_row.get());

    let width1 = wp.w_view_width.get() - extra;
    let mut width2: i32 = 0;
    let mut did_sub_skipcol = false;
    if width1 <= 0 {
        wp.w_wcol.set(wp.w_view_width.get() - 1);
        if wp.w_onebuf_opt.wo_wrap.get() {
            wp.w_wrow.set(wp.w_view_height.get() - 1);
        } else {
            wp.w_wrow
                .set(wp.w_view_height.get() - 1 - wp.w_empty_rows.get());
        }
    } else if wp.w_onebuf_opt.wo_wrap.get() && wp.w_view_width.get() != 0 {
        width2 = width1 + win_col_off2(wp);

        if wp.w_cursor.get().lnum == wp.w_topline.get()
            && wp.w_skipcol.get() > 0
            && wp.w_wcol.get() >= wp.w_skipcol.get()
        {
            if wp.w_skipcol.get() <= width1 {
                wp.w_wcol.set(wp.w_wcol.get() - (width2));
            } else {
                wp.w_wcol.set(
                    wp.w_wcol.get() - (width2 * (((wp.w_skipcol.get() - width1) / width2) + 1)),
                );
            }

            did_sub_skipcol = true;
        }

        if wp.w_wcol.get() >= wp.w_view_width.get() {
            let n = (wp.w_wcol.get() - wp.w_view_width.get()) / width2 + 1;
            wp.w_wcol.set(wp.w_wcol.get() - (n * width2));
            wp.w_wrow.set(wp.w_wrow.get() + (n));
        }
    } else if may_scroll && !wp.w_cline_folded.get() {
        let siso = get_sidescrolloff_value(wp);
        let off_left = i64::from(startcol) - i64::from(wp.w_leftcol.get()) - siso;
        let off_right = i64::from(endcol)
            - i64::from(wp.w_leftcol.get())
            - (i64::from(wp.w_view_width.get()) - siso)
            + 1;
        if off_left < 0 || off_right > 0 {
            let mut diff = if off_left < 0 { -off_left } else { off_right };

            let mut new_leftcol = if opt_vars().p_ss.get() == 0
                || diff >= i64::from(width1 / 2)
                || off_right >= off_left
            {
                wp.w_wcol.get() - extra - width1 / 2
            } else {
                if diff < opt_vars().p_ss.get() {
                    diff = opt_vars().p_ss.get();
                }
                if off_left < 0 {
                    wp.w_leftcol.get() - diff as i32
                } else {
                    wp.w_leftcol.get() + diff as i32
                }
            };
            new_leftcol = new_leftcol.max(0);
            if new_leftcol != wp.w_leftcol.get() {
                wp.w_leftcol.set(new_leftcol);
                win_check_anchored_floats(wp);
                redraw_later(Some(wp), UPD_NOT_VALID);
            }
        }
        wp.w_wcol.set(wp.w_wcol.get() - (wp.w_leftcol.get()));
    } else if wp.w_wcol.get() > wp.w_leftcol.get() {
        wp.w_wcol.set(wp.w_wcol.get() - (wp.w_leftcol.get()));
    } else {
        wp.w_wcol.set(0);
    }

    if wp.w_cursor.get().lnum == wp.w_topline.get() {
        wp.w_wrow.set(wp.w_wrow.get() + (wp.w_topfill.get()));
    } else {
        let fill = win_get_fill(wp, wp.w_cursor.get().lnum);
        wp.w_wrow.set(wp.w_wrow.get() + fill);
    }

    let mut plines: i32 = 0;
    let so = get_scrolloff_value(wp);
    let prev_skipcol = wp.w_skipcol.get();
    if (wp.w_wrow.get() >= wp.w_view_height.get()
        || ((prev_skipcol > 0
            || i64::from(wp.w_wrow.get()) + so >= i64::from(wp.w_view_height.get()))
            && {
                plines = plines_win_nofill(wp, wp.w_cursor.get().lnum, false);
                plines > wp.w_view_height.get()
            }))
        && wp.w_view_height.get() != 0
        && wp.w_cursor.get().lnum == wp.w_topline.get()
        && width2 > 0
        && wp.w_view_width.get() != 0
    {
        extra = 0;
        if i64::from(wp.w_skipcol.get()) + so * i64::from(width2) > i64::from(disp_virtcol) {
            extra = 1;
        }
        if plines == 0 {
            let cursor_lnum = wp.w_cursor.get().lnum;
            plines = plines_win(wp, cursor_lnum, false);
        }
        plines -= 1;
        let mut n = if i64::from(plines) > i64::from(wp.w_wrow.get()) + so {
            (wp.w_wrow.get() as i64 + so) as i32
        } else {
            plines
        };
        if i64::from(n)
            >= i64::from(wp.w_view_height.get()) + i64::from(wp.w_skipcol.get() / width2) - so
        {
            extra += 2;
        }

        if extra == 3 || i64::from(wp.w_view_height.get()) <= so * 2 {
            n = disp_virtcol / width2;
            if n > wp.w_view_height.get() / 2 {
                n -= wp.w_view_height.get() / 2;
            } else {
                n = 0;
            }
            if n > plines - wp.w_view_height.get() + 1 {
                n = plines - wp.w_view_height.get() + 1;
            }
            wp.w_skipcol
                .set(if n > 0 { width1 + (n - 1) * width2 } else { 0 });
        } else if extra == 1 {
            extra = ((i64::from(wp.w_skipcol.get()) + so * i64::from(width2)
                - i64::from(disp_virtcol)
                + i64::from(width2)
                - 1)
                / i64::from(width2)) as i32;
            if extra > 0 {
                if extra * width2 > wp.w_skipcol.get() {
                    extra = wp.w_skipcol.get() / width2;
                }
                wp.w_skipcol.set(wp.w_skipcol.get() - (extra * width2));
            }
        } else if extra == 2 {
            endcol = (n - wp.w_view_height.get() + 1) * width2;
            while endcol > disp_virtcol {
                endcol -= width2;
            }
            wp.w_skipcol.set(wp.w_skipcol.get().max(endcol));
        }

        if did_sub_skipcol {
            wp.w_wrow
                .set(wp.w_wrow.get() - ((wp.w_skipcol.get() - prev_skipcol) / width2));
        } else {
            wp.w_wrow
                .set(wp.w_wrow.get() - (wp.w_skipcol.get() / width2));
        }

        if wp.w_wrow.get() >= wp.w_view_height.get() {
            extra = wp.w_wrow.get() - wp.w_view_height.get() + 1;
            wp.w_skipcol.set(wp.w_skipcol.get() + (extra * width2));
            wp.w_wrow.set(wp.w_wrow.get() - (extra));
        }

        extra = (prev_skipcol - wp.w_skipcol.get()) / width2;
        if !wp.w_grid.target.get().is_null() && wp.w_redr_type.get() < UPD_NOT_VALID {
            win_scroll_lines(wp, 0, extra);
        }
    } else if !wp.w_onebuf_opt.wo_sms.get() {
        wp.w_skipcol.set(0);
    }
    if prev_skipcol != wp.w_skipcol.get() {
        redraw_later(Some(wp), UPD_SOME_VALID);
    }

    redraw_for_cursorcolumn(wp);

    wp.w_valid_leftcol.set(wp.w_leftcol.get());
    wp.w_valid_skipcol.set(wp.w_skipcol.get());

    wp.w_valid
        .set(wp.w_valid.get() | (VALID_WCOL | VALID_WROW | VALID_VIRTCOL));
}

/// Compute the screen position of text character at `pos` in window `wp`.
pub fn textpos2screenpos(
    wp: &WinInfoInfo,
    pos: &mut CursorPos,
    rowp: &mut i32,
    scolp: &mut i32,
    ccolp: &mut i32,
    ecolp: &mut i32,
    local: bool,
) {
    let mut scol: Colnr = 0;
    let mut ccol: Colnr = 0;
    let mut ecol: Colnr = 0;
    let mut row: i32;
    let mut coloff: Colnr = 0;
    let mut visible_row = false;
    let mut is_folded = false;

    let mut lnum = pos.lnum;
    if lnum >= wp.w_topline.get() && lnum <= wp.w_botline.get() {
        is_folded = has_folding(wp, lnum, Some(&mut lnum), None);
        row = plines_m_win(wp, wp.w_topline.get(), lnum - 1, i32::MAX);
        row -= adjust_plines_for_skipcol(wp);
        row += if lnum == wp.w_topline.get() {
            wp.w_topfill.get()
        } else {
            win_get_fill(wp, lnum)
        };
        visible_row = true;
    } else if !local || lnum < wp.w_topline.get() {
        row = 0;
    } else {
        row = wp.w_view_height.get() - 1;
    }

    let existing_row = lnum > 0 && lnum <= wp.buf_mut().b_ml.ml_line_count.get();

    if (local || visible_row) && existing_row {
        let off = win_col_off(wp);
        if is_folded {
            row += if local {
                0
            } else {
                wp.w_winrow.get() + wp.w_winrow_off.get()
            } + 1;
            coloff = if local {
                0
            } else {
                wp.w_wincol.get() + wp.w_wincol_off.get()
            } + 1
                + off;
        } else {
            getvcol(
                wp,
                pos,
                Some(&mut scol),
                Some(&mut ccol),
                Some(&mut ecol),
                GETVCOL_DISP,
            );
            ecol = ecol.max(scol);

            let mut col = ccol + off;
            let width = wp.w_view_width.get() - off + win_col_off2(wp);
            let mut row_vcol = wp.w_leftcol.get();

            if wp.w_onebuf_opt.wo_wrap.get() && col >= wp.w_view_width.get() && width > 0 {
                let rowoff = if visible_row {
                    (col - wp.w_view_width.get()) / width + 1
                } else {
                    0
                };
                col -= rowoff * width;
                row += rowoff;
                row_vcol += rowoff * width - win_col_off2(wp);
            }

            col -= wp.w_leftcol.get();
            scol = scol.max(row_vcol);

            if col >= 0 && col < wp.w_view_width.get() && row >= 0 && row < wp.w_view_height.get() {
                coloff = col - ccol
                    + if local {
                        0
                    } else {
                        wp.w_wincol.get() + wp.w_wincol_off.get()
                    }
                    + 1;
                row += if local {
                    0
                } else {
                    wp.w_winrow.get() + wp.w_winrow_off.get()
                } + 1;
            } else {
                scol = 0;
                ccol = 0;
                ecol = 0;
                if local {
                    coloff = if col < 0 {
                        -1
                    } else {
                        wp.w_view_width.get() + 1
                    };
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

/// `"screenpos({winid}, {lnum}, {col})"` function.
pub fn f_screenpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let dict = tv_dict_alloc_ret(rettv);

    let Some(wp) = find_win_by_nr_or_id_opt(&argvars[0]) else {
        return;
    };

    let mut pos = CursorPos {
        lnum: tv_get_number(&argvars[1]) as Linenr,
        col: (tv_get_number(&argvars[2]) as Colnr - 1).max(0),
        coladd: 0,
    };
    if pos.lnum > (*wp).buf_mut().b_ml.ml_line_count.get() {
        semsg!(tr(E_INVALID_LINE_NUMBER_NR), pos.lnum);
        return;
    }
    let mut row: i32 = 0;
    let mut scol: i32 = 0;
    let mut ccol: i32 = 0;
    let mut ecol: i32 = 0;
    textpos2screenpos(
        &wp, &mut pos, &mut row, &mut scol, &mut ccol, &mut ecol, false,
    );

    tv_dict_add_nr(&dict, c"row", Varnumber::from(row));
    tv_dict_add_nr(&dict, c"col", Varnumber::from(scol));
    tv_dict_add_nr(&dict, c"curscol", Varnumber::from(ccol));
    tv_dict_add_nr(&dict, c"endcol", Varnumber::from(ecol));
}

/// Convert a virtual (screen) column to a character column (1-based).
fn virtcol2col(wp: &WinInfoInfo, lnum: Linenr, vcol: i32) -> i32 {
    let offset = vcol2col(wp, lnum, vcol - 1, None);
    let line = crate::memline::ml_get_buf_slice(&wp.buf_mut(), lnum);
    let mut idx = (offset.max(0) as usize).min(line.len());

    if idx >= line.len() {
        if idx == 0 {
            return 0;
        }
        idx -= crate::mbyte::utf_head_off_slice(line, idx - 1) as usize + 1;
    }
    idx as i32 + 1
}

/// `"virtcol2col({winid}, {lnum}, {col})"` function.
pub fn f_virtcol2col(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if !tv_check_for_number_arg(argvars, 0)
        || !tv_check_for_number_arg(argvars, 1)
        || !tv_check_for_number_arg(argvars, 2)
    {
        return;
    }

    let Some(wp) = find_win_by_nr_or_id_opt(&argvars[0]) else {
        return;
    };

    let mut error = false;
    let lnum = tv_get_number_chk(&argvars[1], Some(&mut error)) as Linenr;
    if error || lnum < 0 || lnum > wp.buf_mut().b_ml.ml_line_count.get() {
        return;
    }

    let screencol = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
    if error || screencol < 0 {
        return;
    }

    rettv.vval =
        crate::eval::typval::TypvalVval::Number(Varnumber::from(virtcol2col(&wp, lnum, screencol)));
}

/// Make sure the cursor is in the visible part of the topline after scrolling
/// the screen with `'smoothscroll'`.
fn cursor_correct_sms(wp: &WinInfoInfo) {
    if !wp.w_onebuf_opt.wo_sms.get()
        || !wp.w_onebuf_opt.wo_wrap.get()
        || wp.w_cursor.get().lnum != wp.w_topline.get()
    {
        return;
    }

    let so = get_scrolloff_value(wp);
    let width1 = wp.w_view_width.get() - win_col_off(wp);
    let width2 = width1 + win_col_off2(wp);
    let mut so_cols = if so == 0 {
        0
    } else {
        i64::from(width1) + (so - 1) * i64::from(width2)
    };
    let space_cols = (wp.w_view_height.get() - 1) * width2;
    let size = if so == 0 {
        0
    } else {
        linetabsize_disp_eol(wp, wp.w_topline.get())
    };

    if wp.w_topline.get() == 1 && wp.w_skipcol.get() == 0 {
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

    let overlap = if wp.w_skipcol.get() == 0 {
        0
    } else {
        sms_marker_overlap(wp, wp.w_view_width.get() - width2)
    };
    let top = i64::from(wp.w_skipcol.get())
        + if so_cols != 0 {
            so_cols
        } else {
            i64::from(overlap)
        };
    let bot = i64::from(wp.w_skipcol.get())
        + i64::from(width1)
        + i64::from((wp.w_view_height.get() - 1) * width2)
        - so_cols;

    validate_virtcol(wp);
    let disp_vcol = cursor_disp_vcol(wp);
    let mut col = disp_vcol;

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

    if col != disp_vcol {
        let cw = disp_vcol2vcol(wp, wp.w_cursor.get().lnum, col);
        wp.w_curswant.set(cw);
        let rc = coladvance(wp, wp.w_curswant.get());
        wp.w_valid.set(
            wp.w_valid.get()
                & (!(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL)),
        );
        if !rc
            && wp.w_skipcol.get() > 0
            && wp.w_cursor.get().lnum < wp.buf_mut().b_ml.ml_line_count.get()
        {
            validate_virtcol(wp);
            if cursor_disp_vcol(wp) < wp.w_skipcol.get() + overlap {
                wp.w_cursor.add_lnum(1);
                wp.w_cursor.set_col(0);
                wp.w_cursor.set_coladd(0);
                wp.w_curswant.set(0);
                wp.w_valid.set(wp.w_valid.get() & (!VALID_VIRTCOL));
            }
        }
    }
}

/// Scroll `count` lines up or down, and redraw.
pub fn scroll_redraw(up: i32, count: Linenr) {
    let prev_topline = curwin_mut().w_topline.get();
    let prev_skipcol = curwin_mut().w_skipcol.get();
    let prev_topfill = curwin_mut().w_topfill.get();
    let prev_lnum = curwin_mut().w_cursor.get().lnum;

    let moved = if up != 0 {
        scrollup(&curwin_mut(), count, true)
    } else {
        scrolldown(&curwin_mut(), count, true)
    };

    if get_scrolloff_value(&curwin_mut()) > 0 {
        cursor_correct(&curwin_mut());
        check_cursor_moved(&curwin_mut());
        curwin_mut()
            .w_valid
            .set(curwin_mut().w_valid.get() | (VALID_TOPLINE));

        while curwin_mut().w_topline.get() == prev_topline
            && curwin_mut().w_skipcol.get() == prev_skipcol
            && curwin_mut().w_topfill.get() == prev_topfill
        {
            if up != 0 {
                if curwin_mut().w_cursor.get().lnum > prev_lnum || !cursor_down(1, false) {
                    break;
                }
            } else if curwin_mut().w_cursor.get().lnum < prev_lnum
                || prev_topline == 1
                || !cursor_up(1, false)
            {
                break;
            }
            check_cursor_moved(&curwin_mut());
            curwin_mut()
                .w_valid
                .set(curwin_mut().w_valid.get() | (VALID_TOPLINE));
        }
    }

    if moved {
        curwin_mut().w_viewport_invalid.set(true);
    }

    cursor_correct_sms(&curwin_mut());
    if curwin_mut().w_cursor.get().lnum != prev_lnum {
        coladvance(&curwin_mut(), curwin_mut().w_curswant.get());
    }
    redraw_later(Some(&crate::globals::curwin_mut()), UPD_VALID);
}

/// Scroll a window down by `line_count` logical lines (`CTRL-Y`).
pub fn scrolldown(wp: &WinInfoInfo, line_count: Linenr, byfold: bool) -> bool {
    let mut done: i32 = 0;
    let mut width1: i32 = 0;
    let mut width2: i32 = 0;
    let do_sms = wp.w_onebuf_opt.wo_wrap.get() && wp.w_onebuf_opt.wo_sms.get();

    if do_sms {
        width1 = wp.w_view_width.get() - win_col_off(wp);
        width2 = width1 + win_col_off2(wp);
    }

    let mut topline = wp.w_topline.get();
    has_folding(wp, topline, Some(&mut topline), None);
    wp.w_topline.set(topline);
    validate_cursor(wp);
    let mut todo = line_count;
    while todo > 0 {
        let can_fill = wp.w_topfill.get() < wp.w_view_height.get() - 1
            && wp.w_topfill.get() < win_get_fill(wp, wp.w_topline.get());
        if wp.w_topline.get() == 1 && !can_fill && (!do_sms || wp.w_skipcol.get() < width1) {
            break;
        }
        if do_sms && wp.w_skipcol.get() >= width1 {
            if wp.w_skipcol.get() >= width1 + width2 {
                wp.w_skipcol.set(wp.w_skipcol.get() - (width2));
            } else {
                wp.w_skipcol.set(wp.w_skipcol.get() - (width1));
            }
            redraw_later(Some(wp), UPD_NOT_VALID);
            done += 1;
        } else if can_fill {
            wp.w_topfill.set(wp.w_topfill.get() + (1));
            done += 1;
        } else {
            wp.w_topline.set(wp.w_topline.get() - (1));
            wp.w_skipcol.set(0);
            wp.w_topfill.set(0);
            let mut first: Linenr = 0;
            let topline = wp.w_topline.get();
            if has_folding(wp, topline, Some(&mut first), None) {
                done += i32::from(!decor_conceal_line(wp, first - 1, false));
                if !byfold {
                    todo -= wp.w_topline.get() - first - 1;
                }
                wp.w_botline
                    .set(wp.w_botline.get() - (wp.w_topline.get() - first));
                wp.w_topline.set(first);
            } else if decor_conceal_line(wp, wp.w_topline.get() - 1, false) {
                todo += 1;
            } else if do_sms {
                let tl = wp.w_topline.get();
                let mut size = linetabsize_disp_eol(wp, tl);
                if size > width1 {
                    wp.w_skipcol.set(width1);
                    size -= width1;
                    redraw_later(Some(wp), UPD_NOT_VALID);
                }
                while size > width2 {
                    wp.w_skipcol.set(wp.w_skipcol.get() + (width2));
                    size -= width2;
                }
                done += 1;
            } else {
                done += plines_win_nofill(wp, wp.w_topline.get(), true);
            }
        }
        wp.w_botline.set(wp.w_botline.get() - (1));
        invalidate_botline_win(wp);
        todo -= 1;
    }

    if wp.w_topfill.get() == win_get_fill(wp, wp.w_topline.get()) {
        while wp.w_topline.get() > 1 && decor_conceal_line(wp, wp.w_topline.get() - 2, false) {
            wp.w_topline.set(wp.w_topline.get() - (1));
            wp.w_topfill.set(0);
            let mut topline = wp.w_topline.get();
            has_folding(wp, topline, Some(&mut topline), None);
            wp.w_topline.set(topline);
        }
    }

    wp.w_wrow.set(wp.w_wrow.get() + (done));
    wp.w_cline_row.set(wp.w_cline_row.get() + (done));

    if wp.w_cursor.get().lnum == wp.w_topline.get() {
        wp.w_cline_row.set(0);
    }
    check_topfill(wp, true);

    let mut wrow = wp.w_wrow.get();
    if wp.w_onebuf_opt.wo_wrap.get() && wp.w_view_width.get() != 0 {
        validate_virtcol(wp);
        validate_cheight(wp);
        wrow += wp.w_cline_height.get() - 1 - cursor_disp_vcol(wp) / wp.w_view_width.get();
    }
    let mut moved = false;
    while wrow >= wp.w_view_height.get() && wp.w_cursor.get().lnum > 1 {
        let mut first: Linenr = 0;
        let cursor_lnum = wp.w_cursor.get().lnum;
        if has_folding(wp, cursor_lnum, Some(&mut first), None) {
            wrow -= i32::from(!decor_conceal_line(wp, wp.w_cursor.get().lnum - 1, false));
            wp.w_cursor.set_lnum((first - 1).max(1));
        } else {
            wrow -= plines_win(wp, cursor_lnum, true);
            wp.w_cursor.sub_lnum(1);
        }
        wp.w_valid.set(
            wp.w_valid.get()
                & (!(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL)),
        );
        moved = true;
    }
    if moved {
        fold_adjust_cursor(wp);
        let curswant = wp.w_curswant.get();
        coladvance(wp, curswant);
    }
    wp.w_cursor
        .set_lnum(wp.w_cursor.get().lnum.max(wp.w_topline.get()));

    moved
}

/// Scroll a window up by `line_count` logical lines (`CTRL-E`).
pub fn scrollup(wp: &WinInfoInfo, line_count: Linenr, byfold: bool) -> bool {
    let topline = wp.w_topline.get();
    let botline = wp.w_botline.get();
    let do_sms = wp.w_onebuf_opt.wo_wrap.get() && wp.w_onebuf_opt.wo_sms.get();

    if do_sms || (byfold && win_lines_concealed(wp)) || win_may_fill(wp) {
        let width1 = wp.w_view_width.get() - win_col_off(wp);
        let width2 = width1 + win_col_off2(wp);
        let mut size = if do_sms {
            let tl = wp.w_topline.get();
            linetabsize_disp_eol(wp, tl)
        } else {
            0
        };
        let prev_skipcol = wp.w_skipcol.get();

        let mut todo = line_count;
        while todo > 0 {
            todo += i32::from(decor_conceal_line(wp, wp.w_topline.get() - 1, false));
            if wp.w_topfill.get() > 0 {
                wp.w_topfill.set(wp.w_topfill.get() - (1));
            } else {
                let mut lnum = wp.w_topline.get();
                if byfold {
                    has_folding(wp, lnum, None, Some(&mut lnum));
                }
                if lnum == wp.w_topline.get() && do_sms {
                    let add = if wp.w_skipcol.get() > 0 {
                        width2
                    } else {
                        width1
                    };
                    wp.w_skipcol.set(wp.w_skipcol.get() + (add));
                    if wp.w_skipcol.get() >= size {
                        if lnum == wp.buf_mut().b_ml.ml_line_count.get() {
                            wp.w_skipcol.set(wp.w_skipcol.get() - (add));
                            break;
                        }
                        lnum += 1;
                    }
                } else {
                    if lnum >= wp.buf_mut().b_ml.ml_line_count.get() {
                        break;
                    }
                    lnum += 1;
                }

                if lnum > wp.w_topline.get() {
                    wp.w_botline
                        .set(wp.w_botline.get() + (lnum - wp.w_topline.get()));
                    wp.w_topline.set(lnum);
                    let tf = win_get_fill(wp, lnum);
                    wp.w_topfill.set(tf);
                    wp.w_skipcol.set(0);
                    if todo > 1 && do_sms {
                        let tl = wp.w_topline.get();
                        size = linetabsize_disp_eol(wp, tl);
                    }
                }
            }
            todo -= 1;
        }

        if prev_skipcol > 0 || wp.w_skipcol.get() > 0 {
            redraw_later(Some(wp), UPD_NOT_VALID);
        }
    } else {
        wp.w_topline.set(wp.w_topline.get() + (line_count));
        wp.w_botline.set(wp.w_botline.get() + (line_count));
    }

    wp.w_topline.set(
        wp.w_topline
            .get()
            .min(wp.buf_mut().b_ml.ml_line_count.get()),
    );
    wp.w_botline.set(
        wp.w_botline
            .get()
            .min(wp.buf_mut().b_ml.ml_line_count.get() + 1),
    );

    check_topfill(wp, false);

    let mut new_topline = wp.w_topline.get();
    has_folding(wp, new_topline, Some(&mut new_topline), None);
    wp.w_topline.set(new_topline);

    wp.w_valid
        .set(wp.w_valid.get() & (!(VALID_WROW | VALID_CROW | VALID_BOTLINE)));
    if wp.w_cursor.get().lnum < wp.w_topline.get() {
        wp.w_cursor.set_lnum(wp.w_topline.get());
        wp.w_valid.set(
            wp.w_valid.get()
                & (!(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW | VALID_VIRTCOL)),
        );
        coladvance(wp, wp.w_curswant.get());
    }

    topline != wp.w_topline.get() || botline != wp.w_botline.get()
}

/// Make sure that `curwin->w_skipcol` is valid for `'smoothscroll'`.
pub fn adjust_skipcol() {
    if !curwin_mut().w_onebuf_opt.wo_wrap.get()
        || !curwin_mut().w_onebuf_opt.wo_sms.get()
        || curwin_mut().w_cursor.get().lnum != curwin_mut().w_topline.get()
    {
        return;
    }

    let width1 = curwin_mut().w_view_width.get() - win_col_off(&curwin_mut());
    if width1 <= 0 {
        return;
    }
    let width2 = width1 + win_col_off2(&curwin_mut());
    let so = get_scrolloff_value(&curwin_mut());
    let scrolloff_cols = if so == 0 {
        0
    } else {
        i64::from(width1) + (so - 1) * i64::from(width2)
    };
    let mut scrolled = false;

    validate_cheight(&curwin_mut());
    if curwin_mut().w_cline_height.get() == curwin_mut().w_view_height.get()
        && plines_win(&curwin_mut(), curwin_mut().w_cursor.get().lnum, false)
            <= curwin_mut().w_view_height.get()
    {
        reset_skipcol(&curwin_mut());
        return;
    }

    validate_virtcol(&curwin_mut());
    let disp_vcol = cursor_disp_vcol(&curwin_mut());
    let overlap = sms_marker_overlap(&curwin_mut(), curwin_mut().w_view_width.get() - width2);
    while curwin_mut().w_skipcol.get() > 0
        && i64::from(disp_vcol) < i64::from(curwin_mut().w_skipcol.get() + overlap) + scrolloff_cols
    {
        if curwin_mut().w_skipcol.get() >= width1 + width2 {
            curwin_mut()
                .w_skipcol
                .set(curwin_mut().w_skipcol.get() - (width2));
        } else {
            curwin_mut()
                .w_skipcol
                .set(curwin_mut().w_skipcol.get() - (width1));
        }
        scrolled = true;
    }
    if scrolled {
        validate_virtcol(&curwin_mut());
        redraw_later(Some(&crate::globals::curwin_mut()), UPD_NOT_VALID);
        return;
    }
    let mut row: i32 = 0;
    let mut col = i64::from(disp_vcol) + scrolloff_cols;

    if scrolloff_cols > 0 {
        let mut size = linetabsize_disp_eol(&curwin_mut(), curwin_mut().w_topline.get());
        size = width1 + width2 * ((size - width1 + width2 - 1) / width2);
        while col > i64::from(size) {
            col -= i64::from(width2);
        }
    }
    col -= i64::from(curwin_mut().w_skipcol.get());

    if col >= i64::from(width1) {
        col -= i64::from(width1);
        row += 1;
    }
    if col > i64::from(width2) {
        row += (col / i64::from(width2)) as i32;
    }
    if row >= curwin_mut().w_view_height.get() {
        if curwin_mut().w_skipcol.get() == 0 {
            curwin_mut()
                .w_skipcol
                .set(curwin_mut().w_skipcol.get() + (width1));
            row -= 1;
        }
        if row >= curwin_mut().w_view_height.get() {
            curwin_mut().w_skipcol.set(
                curwin_mut().w_skipcol.get() + ((row - curwin_mut().w_view_height.get()) * width2),
            );
        }
        redraw_later(Some(&crate::globals::curwin_mut()), UPD_NOT_VALID);
    }
}

/// Don't end up with too many filler lines in the window.
pub fn check_topfill(wp: &WinInfoInfo, down: bool) {
    if wp.w_topfill.get() > 0 {
        let n = plines_win_nofill(wp, wp.w_topline.get(), true);
        if wp.w_topfill.get() + n > wp.w_view_height.get() {
            if down && wp.w_topline.get() > 1 {
                wp.w_topline.set(wp.w_topline.get() - (1));
                wp.w_topfill.set(0);
            } else {
                wp.w_topfill.set((wp.w_view_height.get() - n).max(0));
            }
        }
    }
    win_check_anchored_floats(wp);
}

/// Scroll the screen one line down, without moving the cursor off the screen.
pub fn scrolldown_clamp() {
    let can_fill = curwin_mut().w_topfill.get()
        < win_get_fill(&crate::globals::curwin_mut(), curwin_mut().w_topline.get());

    if curwin_mut().w_topline.get() <= 1 && !can_fill {
        return;
    }

    validate_cursor(&curwin_mut());

    let mut end_row = curwin_mut().w_wrow.get();
    if can_fill {
        end_row += 1;
    } else {
        end_row += plines_win_nofill(&curwin_mut(), curwin_mut().w_topline.get() - 1, true);
    }
    if curwin_mut().w_onebuf_opt.wo_wrap.get() && curwin_mut().w_view_width.get() != 0 {
        validate_cheight(&curwin_mut());
        validate_virtcol(&curwin_mut());
        end_row += curwin_mut().w_cline_height.get()
            - 1
            - cursor_disp_vcol(&curwin_mut()) / curwin_mut().w_view_width.get();
    }
    if i64::from(end_row)
        < i64::from(curwin_mut().w_view_height.get()) - get_scrolloff_value(&curwin_mut())
    {
        if can_fill {
            curwin_mut()
                .w_topfill
                .set(curwin_mut().w_topfill.get() + (1));
            check_topfill(&curwin_mut(), true);
        } else {
            curwin_mut()
                .w_topline
                .set(curwin_mut().w_topline.get() - (1));
            curwin_mut().w_topfill.set(0);
        }
        let mut topline = curwin_mut().w_topline.get();
        has_folding(&curwin_mut(), topline, Some(&mut topline), None);
        curwin_mut().w_topline.set(topline);
        curwin_mut()
            .w_botline
            .set(curwin_mut().w_botline.get() - (1));
        curwin_mut()
            .w_valid
            .set(curwin_mut().w_valid.get() & (!(VALID_WROW | VALID_CROW | VALID_BOTLINE)));
    }
}

/// Scroll the screen one line up, without moving the cursor off the screen.
pub fn scrollup_clamp() {
    if curwin_mut().w_topline.get() == curbuf_mut().b_ml.ml_line_count.get()
        && curwin_mut().w_topfill.get() == 0
    {
        return;
    }

    validate_cursor(&curwin_mut());

    let mut start_row = curwin_mut().w_wrow.get()
        - plines_win_nofill(&curwin_mut(), curwin_mut().w_topline.get(), true)
        - curwin_mut().w_topfill.get();
    if curwin_mut().w_onebuf_opt.wo_wrap.get() && curwin_mut().w_view_width.get() != 0 {
        validate_virtcol(&curwin_mut());
        start_row -= cursor_disp_vcol(&curwin_mut()) / curwin_mut().w_view_width.get();
    }
    if i64::from(start_row) >= get_scrolloff_value(&curwin_mut()) {
        if curwin_mut().w_topfill.get() > 0 {
            curwin_mut()
                .w_topfill
                .set(curwin_mut().w_topfill.get() - (1));
        } else {
            let mut topline = curwin_mut().w_topline.get();
            has_folding(&curwin_mut(), topline, None, Some(&mut topline));
            curwin_mut().w_topline.set(topline + 1);
        }
        curwin_mut()
            .w_botline
            .set(curwin_mut().w_botline.get() + (1));
        curwin_mut()
            .w_valid
            .set(curwin_mut().w_valid.get() & (!(VALID_WROW | VALID_CROW | VALID_BOTLINE)));
    }
}

fn topline_back_winheight(wp: &WinInfoInfo, lp: &mut Lineoff, winheight: bool) {
    if lp.fill < win_get_fill(wp, lp.lnum) {
        lp.fill += 1;
        lp.height = 1;
    } else {
        lp.lnum -= 1;
        lp.fill = 0;
        if lp.lnum < 1 {
            lp.height = MAXCOL;
        } else if has_folding(wp, lp.lnum, Some(&mut lp.lnum), None) {
            lp.height = i32::from(!decor_conceal_line(wp, lp.lnum - 1, false));
        } else {
            lp.height = plines_win_nofill(wp, lp.lnum, winheight);
        }
    }
}

fn topline_back(wp: &WinInfoInfo, lp: &mut Lineoff) {
    topline_back_winheight(wp, lp, true);
}

fn botline_forw(wp: &WinInfoInfo, lp: &mut Lineoff) {
    if lp.fill < win_get_fill(wp, lp.lnum + 1) {
        lp.fill += 1;
        lp.height = 1;
    } else {
        lp.lnum += 1;
        lp.fill = 0;
        if lp.lnum > wp.buf_mut().b_ml.ml_line_count.get() {
            lp.height = MAXCOL;
        } else if has_folding(wp, lp.lnum, None, Some(&mut lp.lnum)) {
            lp.height = i32::from(!decor_conceal_line(wp, lp.lnum - 1, false));
        } else {
            lp.height = plines_win_nofill(wp, lp.lnum, true);
        }
    }
}

/// Recompute topline to put the cursor at the top of the window.
pub fn scroll_cursor_top(wp: &WinInfoInfo, min_scroll: i32, always: i32) {
    let old_topline = wp.w_topline.get();
    let old_skipcol = wp.w_skipcol.get();
    let old_topfill = wp.w_topfill.get();
    let off = if g_st().mouse_dragging.get() > 0 {
        i64::from(g_st().mouse_dragging.get() - 1)
    } else {
        get_scrolloff_value(wp)
    };

    validate_cheight(wp);
    let mut scrolled = if wp.w_cursor.get().lnum < wp.w_topline.get() {
        wp.w_cline_height.get()
    } else {
        0
    };
    let mut used = wp.w_cline_height.get();

    let mut top: Linenr = 0;
    let mut bot: Linenr = 0;
    let cursor_lnum = wp.w_cursor.get().lnum;
    if has_folding(wp, cursor_lnum, Some(&mut top), Some(&mut bot)) {
        top -= 1;
        bot += 1;
    } else {
        top = wp.w_cursor.get().lnum - 1;
        bot = wp.w_cursor.get().lnum + 1;
    }
    let mut new_topline = top + 1;

    let mut extra = win_get_fill(wp, wp.w_cursor.get().lnum);

    while top > 0 {
        let i = plines_win_nofill(wp, top, true);
        has_folding(wp, top, Some(&mut top), None);
        if top < wp.w_topline.get() {
            scrolled += i;
        }

        if (new_topline >= wp.w_topline.get() || scrolled > min_scroll) && i64::from(extra) >= off {
            break;
        }

        used += i;
        if i64::from(extra + i) <= off && bot < wp.buf_mut().b_ml.ml_line_count.get() {
            used += plines_win_full(wp, bot, Some(&mut bot), None, true, true);
        }
        if used > wp.w_view_height.get() {
            break;
        }

        extra += i;
        new_topline = top;
        top -= 1;
        bot += 1;
    }

    if used > wp.w_view_height.get() {
        scroll_cursor_halfway(wp, false, false);
    } else {
        if new_topline < wp.w_topline.get() || always != 0 {
            wp.w_topline.set(new_topline);
        }
        wp.w_topline
            .set(wp.w_topline.get().min(wp.w_cursor.get().lnum));
        let tf = win_get_fill(wp, wp.w_topline.get());
        wp.w_topfill.set(tf);
        if wp.w_topfill.get() > 0 && i64::from(extra) > off {
            wp.w_topfill.set(wp.w_topfill.get() - (extra - off as i32));
            wp.w_topfill.set(wp.w_topfill.get().max(0));
        }
        check_topfill(wp, false);
        if wp.w_topline.get() != old_topline {
            reset_skipcol(wp);
        } else if wp.w_topline.get() == wp.w_cursor.get().lnum {
            validate_virtcol(wp);
            let disp_vcol = cursor_disp_vcol(wp);
            if wp.w_skipcol.get() >= disp_vcol {
                let width1 = wp.w_view_width.get() - win_col_off(wp);
                let width2 = width1 + win_col_off2(wp);
                let plines_off = if width2 > 0 && disp_vcol >= width1 {
                    (disp_vcol - width1) / width2 + 1
                } else {
                    0
                };
                let skipcol = skipcol_from_plines(wp, plines_off);
                if skipcol != wp.w_skipcol.get() {
                    wp.w_skipcol.set(skipcol);
                    redraw_later(Some(wp), UPD_SOME_VALID);
                }
            }
        }
        if wp.w_topline.get() != old_topline
            || wp.w_skipcol.get() != old_skipcol
            || wp.w_topfill.get() != old_topfill
        {
            wp.w_valid.set(
                wp.w_valid.get() & (!(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP)),
            );
        }
        wp.w_valid.set(wp.w_valid.get() | (VALID_TOPLINE));
        wp.w_viewport_invalid.set(true);
    }
}

/// Set `w_empty_rows` and `w_filler_rows` for window `wp`.
pub fn set_empty_rows(wp: &WinInfoInfo, used: i32) {
    wp.w_filler_rows.set(0);
    if used == 0 {
        wp.w_empty_rows.set(0);
    } else {
        wp.w_empty_rows.set(wp.w_view_height.get() - used);
        if wp.w_botline.get() <= wp.buf_mut().b_ml.ml_line_count.get() {
            let fr = win_get_fill(wp, wp.w_botline.get());
            wp.w_filler_rows.set(fr);
            if wp.w_empty_rows.get() > wp.w_filler_rows.get() {
                wp.w_empty_rows
                    .set(wp.w_empty_rows.get() - (wp.w_filler_rows.get()));
            } else {
                wp.w_filler_rows.set(wp.w_empty_rows.get());
                wp.w_empty_rows.set(0);
            }
        }
    }
}

/// Recompute topline to put the cursor at the bottom of the window.
pub fn scroll_cursor_bot(wp: &WinInfoInfo, min_scroll: i32, set_topbot: bool) {
    let mut loff = Lineoff::default();
    let old_topline = wp.w_topline.get();
    let old_skipcol = wp.w_skipcol.get();
    let old_topfill = wp.w_topfill.get();
    let old_botline = wp.w_botline.get();
    let old_valid = wp.w_valid.get();
    let old_empty_rows = wp.w_empty_rows.get();
    let cln = wp.w_cursor.get().lnum;
    let do_sms = wp.w_onebuf_opt.wo_wrap.get() && wp.w_onebuf_opt.wo_sms.get();

    if set_topbot {
        let mut used: i32 = 0;
        let mut cln_last = cln;
        has_folding(wp, cln, None, Some(&mut cln_last));
        wp.w_botline.set(cln_last + 1);
        loff.lnum = cln_last + 1;
        loff.fill = 0;
        loop {
            topline_back_winheight(wp, &mut loff, false);
            if loff.height == MAXCOL {
                break;
            }
            if used + loff.height > wp.w_view_height.get() {
                if do_sms && used < wp.w_view_height.get() {
                    let plines_offset = used + loff.height - wp.w_view_height.get();
                    used = wp.w_view_height.get();
                    wp.w_topfill.set(loff.fill);
                    wp.w_topline.set(loff.lnum);
                    let sc = skipcol_from_plines(wp, plines_offset);
                    wp.w_skipcol.set(sc);
                }
                break;
            }
            wp.w_topfill.set(loff.fill);
            wp.w_topline.set(loff.lnum);
            used += loff.height;
        }

        set_empty_rows(wp, used);
        wp.w_valid
            .set(wp.w_valid.get() | (VALID_BOTLINE | VALID_BOTLINE_AP));
        if wp.w_topline.get() != old_topline
            || wp.w_topfill.get() != old_topfill
            || wp.w_skipcol.get() != old_skipcol
            || wp.w_skipcol.get() != 0
        {
            wp.w_valid
                .set(wp.w_valid.get() & (!(VALID_WROW | VALID_CROW)));
            if wp.w_skipcol.get() != old_skipcol {
                redraw_later(Some(wp), UPD_NOT_VALID);
            } else {
                reset_skipcol(wp);
            }
        }
    } else {
        validate_botline_win(wp);
    }

    let mut used = plines_win_nofill(wp, cln, true);

    let mut scrolled: i32 = 0;
    if cln >= wp.w_botline.get() {
        scrolled = used;
        if cln == wp.w_botline.get() {
            scrolled -= wp.w_empty_rows.get();
        }
        if do_sms {
            let mut top_plines = plines_win_nofill(wp, wp.w_topline.get(), false);
            let width1 = wp.w_view_width.get() - win_col_off(wp);

            if width1 > 0 {
                let width2 = width1 + win_col_off2(wp);
                let skip_lines = if wp.w_skipcol.get() > width1 {
                    (wp.w_skipcol.get() - width1) / width2 + 1
                } else if wp.w_skipcol.get() > 0 {
                    1
                } else {
                    0
                };

                top_plines -= skip_lines;
                if top_plines > wp.w_view_height.get() {
                    scrolled += top_plines - wp.w_view_height.get();
                }
            }
        }
    }

    let mut boff = Lineoff::default();
    let cursor_lnum = wp.w_cursor.get().lnum;
    if !has_folding(wp, cursor_lnum, Some(&mut loff.lnum), Some(&mut boff.lnum)) {
        loff.lnum = cln;
        boff.lnum = cln;
    }
    loff.fill = 0;
    boff.fill = 0;
    let fill_below_window = win_get_fill(wp, wp.w_botline.get()) - wp.w_filler_rows.get();

    let mut extra: i32 = 0;
    let so = get_scrolloff_value(wp);
    let eff_so = if g_st().mouse_dragging.get() > 0 {
        i64::from(g_st().mouse_dragging.get() - 1)
    } else {
        so
    };
    while loff.lnum > 1 {
        if (((scrolled <= 0 || scrolled >= min_scroll) && i64::from(extra) >= eff_so)
            || boff.lnum + 1 > wp.buf_mut().b_ml.ml_line_count.get())
            && loff.lnum <= wp.w_botline.get()
            && (loff.lnum < wp.w_botline.get() || loff.fill >= fill_below_window)
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
        if used > wp.w_view_height.get() {
            break;
        }
        if loff.lnum >= wp.w_botline.get()
            && (loff.lnum > wp.w_botline.get() || loff.fill <= fill_below_window)
        {
            scrolled += loff.height;
            if loff.lnum == wp.w_botline.get() && loff_lnum_before > curwin_mut().w_botline.get() {
                scrolled -= wp.w_empty_rows.get();
            }
        }

        if boff.lnum < wp.buf_mut().b_ml.ml_line_count.get() {
            let boff_lnum_before = boff.lnum;
            botline_forw(wp, &mut boff);
            used += boff.height;
            if used > wp.w_view_height.get() {
                break;
            }
            if i64::from(extra) < eff_so || scrolled < min_scroll {
                extra += boff.height;
                if boff.lnum >= wp.w_botline.get()
                    || (boff.lnum + 1 == wp.w_botline.get() && boff.fill > wp.w_filler_rows.get())
                {
                    scrolled += boff.height;
                    if boff.lnum >= curwin_mut().w_botline.get()
                        && boff_lnum_before < curwin_mut().w_botline.get()
                    {
                        scrolled -= wp.w_empty_rows.get();
                    }
                }
            }
        }
    }

    let mut line_count: Linenr = if scrolled <= 0 {
        0
    } else if used > wp.w_view_height.get() {
        used
    } else {
        let mut lc = 0;
        boff.fill = wp.w_topfill.get();
        boff.lnum = wp.w_topline.get() - 1;
        let mut i = 0;
        while i < scrolled && boff.lnum < wp.w_botline.get() {
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
    if line_count >= wp.w_view_height.get() && line_count > min_scroll {
        scroll_cursor_halfway(wp, eof_pressure, true);
    } else if line_count > 0 {
        if do_sms {
            scrollup(wp, scrolled, true);
        } else {
            scrollup(wp, line_count, true);
        }
    }
    let _ = &mut line_count;

    if wp.w_topline.get() == old_topline && wp.w_skipcol.get() == old_skipcol && set_topbot {
        wp.w_botline.set(old_botline);
        wp.w_empty_rows.set(old_empty_rows);
        wp.w_valid.set(old_valid);
    }
    wp.w_valid.set(wp.w_valid.get() | (VALID_TOPLINE));
    wp.w_viewport_invalid.set(true);

    if set_topbot {
        cursor_correct_sms(wp);
    }
}

/// Recompute topline to put the cursor halfway across the window.
pub fn scroll_cursor_halfway(wp: &WinInfoInfo, atend: bool, prefer_above: bool) {
    let old_topline = wp.w_topline.get();
    let mut loff = Lineoff {
        lnum: wp.w_cursor.get().lnum,
        fill: 0,
        height: 0,
    };
    let mut boff = Lineoff {
        lnum: wp.w_cursor.get().lnum,
        fill: 0,
        height: 0,
    };
    let lnum = loff.lnum;
    has_folding(wp, lnum, Some(&mut loff.lnum), Some(&mut boff.lnum));
    let mut used = plines_win_nofill(wp, loff.lnum, true);
    let mut topline = loff.lnum;
    let mut skipcol: Colnr = 0;

    let mut want_height: i32 = 0;
    let do_sms = wp.w_onebuf_opt.wo_wrap.get() && wp.w_onebuf_opt.wo_sms.get();
    if do_sms {
        if atend {
            want_height = (wp.w_view_height.get() - used) / 2;
            used = 0;
        } else {
            want_height = wp.w_view_height.get();
        }
    }

    let mut topfill: i32 = 0;
    let mut above: i32 = 0;
    let mut below: i32 = 0;
    while topline > 1 {
        if do_sms {
            topline_back_winheight(wp, &mut loff, false);
            if loff.height == MAXCOL {
                break;
            }
            used += loff.height;
            if !atend && boff.lnum < wp.buf_mut().b_ml.ml_line_count.get() {
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
                if boff.lnum < wp.buf_mut().b_ml.ml_line_count.get() {
                    botline_forw(wp, &mut boff);
                    used += boff.height;
                    if used > wp.w_view_height.get() {
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
                if used > wp.w_view_height.get() {
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

    let mut new_topline = wp.w_topline.get();
    let folded = has_folding(wp, topline, Some(&mut new_topline), None);
    wp.w_topline.set(new_topline);
    if !folded && (wp.w_topline.get() != topline || skipcol != 0 || wp.w_skipcol.get() != 0) {
        wp.w_topline.set(topline);
        if skipcol != 0 {
            wp.w_skipcol.set(skipcol);
            redraw_later(Some(wp), UPD_NOT_VALID);
        } else if do_sms {
            reset_skipcol(wp);
        }
    }
    wp.w_topfill.set(topfill);
    if old_topline > wp.w_topline.get() + wp.w_view_height.get() {
        wp.w_botfill.set(false);
    }
    check_topfill(wp, false);
    wp.w_valid
        .set(wp.w_valid.get() & (!(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP)));
    wp.w_valid.set(wp.w_valid.get() | (VALID_TOPLINE));
}

/// Correct the cursor position so that it is in a part of the screen at least
/// `'so'` lines from the top and bottom, if possible.
pub fn cursor_correct(wp: &WinInfoInfo) {
    let mut above_wanted = get_scrolloff_value(wp);
    let mut below_wanted = get_scrolloff_value(wp);
    if g_st().mouse_dragging.get() > 0 {
        above_wanted = i64::from(g_st().mouse_dragging.get() - 1);
        below_wanted = i64::from(g_st().mouse_dragging.get() - 1);
    }
    if wp.w_topline.get() == 1 {
        above_wanted = 0;
        let max_off = i64::from(wp.w_view_height.get() / 2);
        below_wanted = below_wanted.min(max_off);
    }
    validate_botline_win(wp);
    if wp.w_botline.get() == wp.buf_mut().b_ml.ml_line_count.get() + 1
        && g_st().mouse_dragging.get() == 0
    {
        below_wanted = 0;
        let max_off = i64::from((wp.w_view_height.get() - 1) / 2);
        above_wanted = above_wanted.min(max_off);
    }

    let cln = wp.w_cursor.get().lnum;
    if i64::from(cln) >= i64::from(wp.w_topline.get()) + above_wanted
        && i64::from(cln) < i64::from(wp.w_botline.get()) - below_wanted
        && !win_lines_concealed(wp)
    {
        return;
    }

    let mut topline = wp.w_topline.get();
    let mut botline = wp.w_botline.get() - 1;
    let mut above = wp.w_topfill.get();
    let mut below = wp.w_filler_rows.get();
    while (i64::from(above) < above_wanted || i64::from(below) < below_wanted) && topline < botline
    {
        if i64::from(below) < below_wanted && (below <= above || i64::from(above) >= above_wanted) {
            below += plines_win_full(wp, botline, None, None, true, true);
            has_folding(wp, botline, Some(&mut botline), None);
            botline -= 1;
        }
        if i64::from(above) < above_wanted && (above < below || i64::from(below) >= below_wanted) {
            above += plines_win_nofill(wp, topline, true);
            has_folding(wp, topline, None, Some(&mut topline));
            if topline < botline {
                above += win_get_fill(wp, topline + 1);
            }
            topline += 1;
        }
    }
    if topline == botline || botline == 0 {
        wp.w_cursor.set_lnum(topline);
    } else if topline > botline {
        wp.w_cursor.set_lnum(botline);
    } else {
        if cln < topline && wp.w_topline.get() > 1 {
            wp.w_cursor.set_lnum(topline);
            wp.w_valid
                .set(wp.w_valid.get() & (!(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW)));
        }
        if cln > botline && wp.w_botline.get() <= wp.buf_mut().b_ml.ml_line_count.get() {
            wp.w_cursor.set_lnum(botline);
            wp.w_valid
                .set(wp.w_valid.get() & (!(VALID_WROW | VALID_WCOL | VALID_CHEIGHT | VALID_CROW)));
        }
    }
    check_cursor_moved(wp);
    wp.w_valid.set(wp.w_valid.get() | (VALID_TOPLINE));
    wp.w_viewport_invalid.set(true);
}

/// Decide how much overlap to use for page-up or page-down scrolling.
fn get_scroll_overlap(dir: Direction) -> i32 {
    let min_height = curwin_mut().w_view_height.get() - 2;

    validate_botline_win(&curwin_mut());
    if (dir == BACKWARD && curwin_mut().w_topline.get() == 1)
        || (dir == FORWARD && curwin_mut().w_botline.get() > curbuf_mut().b_ml.ml_line_count.get())
    {
        return min_height + 2;
    }

    let lnum = if dir == FORWARD {
        curwin_mut().w_botline.get()
    } else {
        curwin_mut().w_topline.get() - 1
    };
    let fill = win_get_fill(
        &crate::globals::curwin_mut(),
        lnum + i32::from(dir == BACKWARD),
    ) - if dir == FORWARD {
        curwin_mut().w_filler_rows.get()
    } else {
        curwin_mut().w_topfill.get()
    };
    let mut loff = Lineoff {
        lnum,
        fill,
        height: if fill > 0 {
            1
        } else {
            plines_win_nofill(&curwin_mut(), lnum, true)
        },
    };

    let h1 = loff.height;
    if h1 > min_height {
        return min_height + 2;
    }
    if dir == FORWARD {
        topline_back(&curwin_mut(), &mut loff);
    } else {
        botline_forw(&curwin_mut(), &mut loff);
    }

    let h2 = loff.height;
    if h2 == MAXCOL || h2 + h1 > min_height {
        return min_height + 2;
    }
    if dir == FORWARD {
        topline_back(&curwin_mut(), &mut loff);
    } else {
        botline_forw(&curwin_mut(), &mut loff);
    }

    let h3 = loff.height;
    if h3 == MAXCOL || h3 + h2 > min_height {
        return min_height + 2;
    }
    if dir == FORWARD {
        topline_back(&curwin_mut(), &mut loff);
    } else {
        botline_forw(&curwin_mut(), &mut loff);
    }

    let h4 = loff.height;
    if h4 == MAXCOL || h4 + h3 + h2 > min_height || h3 + h2 + h1 > min_height {
        min_height + 1
    } else {
        min_height
    }
}

/// Scroll `count` lines with `'smoothscroll'` in direction `dir`.
fn scroll_with_sms(dir: Direction, mut count: i32, curscount: &mut i32) -> bool {
    let prev_sms = curwin_mut().w_onebuf_opt.wo_sms.get();
    let prev_skipcol = curwin_mut().w_skipcol.get();
    let prev_topline = curwin_mut().w_topline.get();
    let prev_topfill = curwin_mut().w_topfill.get();

    curwin_mut().w_onebuf_opt.wo_sms.set(true);
    scroll_redraw(i32::from(dir == FORWARD), count);

    if !prev_sms && curwin_mut().w_skipcol.get() > 0 {
        let mut fixdir = dir;
        if (curwin_mut().w_topline.get() - prev_topline).abs() > i32::from(dir == BACKWARD) {
            fixdir = -dir;
        }

        let width1 = curwin_mut().w_view_width.get() - win_col_off(&curwin_mut());
        let width2 = width1 + win_col_off2(&curwin_mut());
        count = 1 + (curwin_mut().w_skipcol.get() - width1 - 1) / width2;
        if fixdir == FORWARD {
            count = 1
                + (linetabsize_disp_eol(&curwin_mut(), curwin_mut().w_topline.get())
                    - curwin_mut().w_skipcol.get()
                    - width1
                    + width2
                    - 1)
                    / width2;
        }
        scroll_redraw(i32::from(fixdir == FORWARD), count);
        *curscount += count * if fixdir == dir { 1 } else { -1 };
    }
    curwin_mut().w_onebuf_opt.wo_sms.set(prev_sms);

    curwin_mut().w_topline.get() != prev_topline
        || curwin_mut().w_topfill.get() != prev_topfill
        || curwin_mut().w_skipcol.get() != prev_skipcol
}

/// Move screen `count` (half) pages up (`dir` is `BACKWARD`) or down (`dir` is `FORWARD`).
pub fn pagescroll(dir: Direction, mut count: i32, half: bool) -> bool {
    let mut did_move = false;
    let buflen = curbuf_mut().b_ml.ml_line_count.get();
    let prev_col = curwin_mut().w_cursor.get().col;
    let prev_curswant = curwin_mut().w_curswant.get();
    let prev_lnum = curwin_mut().w_cursor.get().lnum;
    let mut ca = Cmdarg::new();

    if half {
        if count != 0 {
            curwin_mut()
                .w_onebuf_opt
                .wo_scr
                .set(i64::from(curwin_mut().w_view_height.get().min(count)));
        }
        count = curwin_mut()
            .w_view_height
            .get()
            .min(curwin_mut().w_onebuf_opt.wo_scr.get() as i32);

        let mut curscount = count;
        if dir == FORWARD
            && (curwin_mut().w_topline.get() + curwin_mut().w_view_height.get() + count > buflen
                || win_lines_concealed(&curwin_mut()))
        {
            let mut n = plines_correct_topline(
                &curwin_mut(),
                curwin_mut().w_topline.get(),
                None,
                false,
                None,
            );
            if n - count < curwin_mut().w_view_height.get() && curwin_mut().w_topline.get() < buflen
            {
                n += plines_m_win(
                    &curwin_mut(),
                    curwin_mut().w_topline.get() + 1,
                    buflen,
                    curwin_mut().w_view_height.get() + count,
                );
            }
            if n < curwin_mut().w_view_height.get() + count {
                count = n - curwin_mut().w_view_height.get();
            }
        }

        if count > 0 {
            did_move = scroll_with_sms(dir, count, &mut curscount);
            curwin_mut().w_cursor.set_lnum(prev_lnum);
            curwin_mut().w_cursor.set_col(prev_col);
            curwin_mut().w_curswant.set(prev_curswant);
        }

        if curwin_mut().w_onebuf_opt.wo_wrap.get() {
            nv_screengo(&mut ca.oap, dir, curscount, true);
        } else if dir == FORWARD {
            cursor_down_inner(&curwin_mut(), curscount, true);
        } else {
            cursor_up_inner(&curwin_mut(), curscount, true);
        }
    } else {
        let one_window = g_st().firstwin.get() == g_st().lastwin.get();
        count *= if one_window
            && opt_vars().p_window.get() > 0
            && opt_vars().p_window.get() < i64::from(g_st().rows_global.get() - 1)
        {
            (opt_vars().p_window.get() as i32 - 2).max(1)
        } else {
            get_scroll_overlap(dir)
        };
        did_move = scroll_with_sms(dir, count, &mut count);

        if did_move {
            validate_botline_win(&curwin_mut());
            let lnum = if dir == FORWARD {
                curwin_mut().w_topline.get()
            } else {
                curwin_mut().w_botline.get() - 1
            };
            curwin_mut().w_cursor.set_lnum(lnum.max(1));
        }
    }

    if get_scrolloff_value(&curwin_mut()) > 0 {
        cursor_correct(&curwin_mut());
    }
    fold_adjust_cursor(&curwin_mut());

    did_move = did_move
        || prev_col != curwin_mut().w_cursor.get().col
        || prev_lnum != curwin_mut().w_cursor.get().lnum;

    if !did_move {
        beep_flush();
    } else if !curwin_mut().w_onebuf_opt.wo_sms.get() {
        beginline(BL_SOL | BL_FIX);
    } else if opt_vars().p_sol.get() {
        nv_g_home_m_cmd(&mut ca);
    }

    did_move
}

pub fn do_check_cursorbind() {
    #[derive(Clone)]
    struct MoveCursorbindState {
        prev_curwin: crate::globals::WinPtr,
        prev_curbuf: crate::globals::BufPtr,
        prev_changedtick: Varnumber,
        prev_cursor: CursorPos,
    }

    impl MoveCursorbindState {
        const fn new() -> Self {
            Self {
                prev_curwin: crate::globals::WinPtr::NULL,
                prev_curbuf: crate::globals::BufPtr::NULL,
                prev_changedtick: 0,
                prev_cursor: CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                },
            }
        }
    }

    std::thread_local! {
        static MOVE_CB_STATE: crate::globals::CloneCell<MoveCursorbindState> =
            const { crate::globals::CloneCell::new(MoveCursorbindState::new()) };
    }

    debug_assert!(!(curwin_ptr()).is_null() && !g_st().curbuf.get().is_null());
    let prev = MOVE_CB_STATE.with(|s| s.get());
    if prev.prev_curwin == curwin_ptr()
        && prev.prev_curbuf == g_st().curbuf.get()
        && buf_get_changedtick(&curbuf_mut()) == prev.prev_changedtick
        && equalpos(curwin_mut().w_cursor.get(), prev.prev_cursor)
    {
        return;
    }
    MOVE_CB_STATE.with(|s| {
        s.set(MoveCursorbindState {
            prev_curwin: crate::globals::curwin_ptr(),
            prev_curbuf: g_st().curbuf.get(),
            prev_changedtick: buf_get_changedtick(&curbuf_mut()),
            prev_cursor: curwin_mut().w_cursor.get(),
        });
    });

    let line = curwin_mut().w_cursor.get().lnum;
    let col = curwin_mut().w_cursor.get().col;
    let coladd = curwin_mut().w_cursor.get().coladd;
    let curswant = curwin_mut().w_curswant.get();
    let set_curswant = curwin_mut().w_set_curswant.get();
    let old_curwin = curwin_ptr();
    let old_curbuf = g_st().curbuf.get();
    let old_visual_select = g_st().visual_global.borrow().select;
    let old_visual_active = g_st().visual_global.borrow().active;

    g_st().visual_global.borrow_mut().select = false;
    g_st().visual_global.borrow_mut().active = false;
    for wp in for_all_windows_ptr() {
        set_curwin(wp);
        g_st().curbuf.set(curwin_mut().w_buffer.get());
        if curwin_ptr() != old_curwin && curwin_mut().w_onebuf_opt.wo_crb.get() {
            if curwin_mut().w_onebuf_opt.wo_diff.get() {
                curwin_mut()
                    .w_cursor
                    .set_lnum(diff_get_corresponding_line(&old_curbuf, line));
            } else {
                curwin_mut().w_cursor.set_lnum(line);
            }
            curwin_mut().w_cursor.set_col(col);
            curwin_mut().w_cursor.set_coladd(coladd);
            curwin_mut().w_curswant.set(curswant);
            curwin_mut().w_set_curswant.set(set_curswant);

            {
                let restart_edit_save = g_st().restart_edit.get();
                g_st().restart_edit.set(i32::from(true));
                check_cursor(&curwin_mut());

                if !curwin_mut().w_onebuf_opt.wo_scb.get() {
                    validate_cursor(&curwin_mut());
                }

                g_st().restart_edit.set(restart_edit_save);
            }
            mb_adjust_cursor();
            redraw_later(Some(&crate::globals::curwin_mut()), UPD_VALID);

            if !curwin_mut().w_onebuf_opt.wo_scb.get() {
                update_topline(&curwin_mut());
            }
            curwin_mut().w_redr_status.set(true);
        }
    }

    g_st().visual_global.borrow_mut().select = old_visual_select;
    g_st().visual_global.borrow_mut().active = old_visual_active;
    set_curwin(old_curwin);
    g_st().curbuf.set(old_curbuf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_and_viewstate_roundtrip() {
        let p1 = CursorPos {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        let p2 = CursorPos {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        assert!(equalpos(p1, p2));

        {
            let mut wp_box = WinInfoInfo::new();
            let wp = &mut wp_box;
            wp.w_curswant.set(42);
            wp.w_set_curswant.set(true);
            wp.w_leftcol.set(4);
            wp.w_skipcol.set(8);
            wp.w_topline.set(10);
            wp.w_topfill.set(2);
            wp.w_botline.set(20);
            wp.w_empty_rows.set(3);

            let mut vs = Viewstate::default();
            save_viewstate(wp, &mut vs);
            let mut wp2_box = WinInfoInfo::new();
            let wp2 = &mut wp2_box;
            restore_viewstate(wp2, &vs);
            assert_eq!(wp2.w_curswant.get(), 42);
            assert!(wp2.w_set_curswant.get());
            assert_eq!(wp2.w_leftcol.get(), 4);
            assert_eq!(wp2.w_skipcol.get(), 8);
            assert_eq!(wp2.w_topline.get(), 10);
            assert_eq!(wp2.w_topfill.get(), 2);
            assert_eq!(wp2.w_botline.get(), 20);
            assert_eq!(wp2.w_empty_rows.get(), 3);
        }
    }
}
