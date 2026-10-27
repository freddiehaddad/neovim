//! Cursor positioning, validation, and line access (`src/nvim/cursor.c`).

use crate::buffer::buf_T;
use crate::change::{TAB, inserted_bytes, kOptVeFlagOnemore};
use crate::charset::NUL;
use crate::drawscreen::{UPD_NOT_VALID, redraw_later};
use crate::fold::{hasAnyFolding, hasFolding};
use crate::globals::*;
use crate::mark::mark_mb_adjustpos;
use crate::math::{FAIL, OK};
use crate::mbyte::{utf_ClusterInfo, utf_head_off, utf_ptr2StrCharInfo, utf_ptr2char};
use crate::memline::{
    dec, inc, ml_get_buf, ml_get_buf_len, ml_get_buf_mut, ml_get_len, ml_replace,
};
use crate::r#move::{changed_cline_bef_curs, set_valid_virtcol, validate_virtcol, win_col_off};
use crate::option::{get_sidescrolloff_value, get_ve_flags};
use crate::option_vars::*;
use crate::plines::{
    CSType, CharSize, CharsizeArg, getvcol, getvvcol, init_charsize_arg, linetabsize,
    linetabsize_eol, win_charsize,
};
use crate::regexp::MAXCOL;
use crate::state::{MODE_INSERT, MODE_TERMINAL, virtual_active};
use crate::window::win_T;
use std::ffi::{c_char, c_int, c_uint};
use std::ptr;

pub type colnr_T = i32;
pub type linenr_T = i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pos_T {
    pub lnum: i32,
    pub col: i32,
    pub coladd: i32,
}

impl pos_T {
    #[inline]
    pub const fn new() -> Self {
        Self {
            lnum: 0,
            col: 0,
            coladd: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct lpos_T {
    pub lnum: i32,
    pub col: i32,
}

impl lpos_T {
    #[inline]
    pub const fn new() -> Self {
        Self { lnum: 0, col: 0 }
    }
}

#[inline]
pub const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
pub const fn lt(a: pos_T, b: pos_T) -> bool {
    if a.lnum != b.lnum {
        a.lnum < b.lnum
    } else if a.col != b.col {
        a.col < b.col
    } else {
        a.coladd < b.coladd
    }
}

#[inline]
pub const fn clearpos(a: &mut pos_T) {
    a.lnum = 0;
    a.col = 0;
    a.coladd = 0;
}

#[inline]
pub const fn ltoreq(a: pos_T, b: pos_T) -> bool {
    lt(a, b) || equalpos(a, b)
}

pub(crate) const VALID_VIRTCOL: c_int = 0x04;

pub(crate) const kOptVeFlagAll: c_uint = 0x04;

/// Return the screen position of the cursor (`getviscol`).
pub unsafe fn getviscol() -> c_int {
    unsafe {
        let mut x: colnr_T = 0;
        getvvcol(
            *curwin.as_ptr(),
            &mut (**curwin.as_ptr()).w_cursor,
            &raw mut x,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        );
        x as c_int
    }
}

/// Return the screen position of character `col` with a `coladd` in the cursor line (`getviscol2`).
pub unsafe fn getviscol2(col: colnr_T, coladd: colnr_T) -> c_int {
    unsafe {
        let mut x: colnr_T = 0;
        let mut pos = pos_T {
            lnum: (**curwin.as_ptr()).w_cursor.lnum,
            col,
            coladd,
        };
        getvvcol(
            *curwin.as_ptr(),
            &mut pos,
            &raw mut x,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        );
        x as c_int
    }
}

/// Go to column `wcol`, and add/insert white space as necessary (`coladvance_force`).
pub unsafe fn coladvance_force(wcol: colnr_T) -> c_int {
    unsafe {
        let rc = coladvance2(*curwin.as_ptr(), &mut (**curwin.as_ptr()).w_cursor, true, false, wcol);

        if wcol == MAXCOL {
            (**curwin.as_ptr()).w_valid &= !VALID_VIRTCOL;
        } else {
            set_valid_virtcol(*curwin.as_ptr(), wcol);
        }
        rc
    }
}

/// Try to advance the Cursor to the specified screen column (`coladvance`).
pub unsafe fn coladvance(wp: *mut win_T, wcol: colnr_T) -> c_int {
    unsafe {
        let rc = getvpos(wp, &mut (*wp).w_cursor, wcol);

        if wcol == MAXCOL || rc == FAIL {
            (*wp).w_valid &= !VALID_VIRTCOL;
        } else if *(ml_get_buf((*wp).w_buffer, (*wp).w_cursor.lnum)
            .add((*wp).w_cursor.col as usize))
            != TAB
        {
            set_valid_virtcol(*curwin.as_ptr(), wcol);
        }
        rc
    }
}

unsafe fn coladvance2(
    wp: *mut win_T,
    pos: &mut pos_T,
    addspaces: bool,
    finetune: bool,
    wcol_arg: colnr_T,
) -> c_int {
    unsafe {
        debug_assert!(wp == *curwin.as_ptr() || !addspaces);
        let mut wcol = wcol_arg;
        let mut idx: c_int;
        let mut col: colnr_T;
        let mut head: c_int = 0;

        let one_more = c_int::from(
            (g_st().State & MODE_INSERT) != 0
                || (g_st().State & MODE_TERMINAL) != 0
                || g_st().restart_edit != 0
                || (g_st().Visual.active && *opt_vars().p_sel != b'o' as c_char)
                || ((get_ve_flags(wp) & kOptVeFlagOnemore) != 0 && wcol < MAXCOL),
        );

        let line = ml_get_buf((*wp).w_buffer, pos.lnum);
        let linelen = ml_get_buf_len((*wp).w_buffer, pos.lnum);

        if wcol == MAXCOL {
            idx = linelen - 1 + one_more;
            col = wcol;

            if (addspaces || finetune) && !g_st().Visual.active {
                (*wp).w_curswant = linetabsize(wp, pos.lnum) + one_more;
                if (*wp).w_curswant > 0 {
                    (*wp).w_curswant -= 1;
                }
            }
        } else {
            let width = (*wp).w_view_width - win_col_off(wp);
            let mut csize: c_int = 0;

            if finetune
                && (*wp).w_onebuf_opt.wo_wrap != 0
                && (*wp).w_view_width != 0
                && wcol >= width
                && width > 0
            {
                csize = linetabsize_eol(wp, pos.lnum);
                if csize > 0 {
                    csize -= 1;
                }

                if wcol / width > csize / width && ((g_st().State & MODE_INSERT) == 0 || wcol > csize + 1)
                {
                    wcol = (csize / width + 1) * width - 1;
                }
            }

            let mut csarg = CharsizeArg::new();
            let cstype: CSType = init_charsize_arg(&raw mut csarg, wp, pos.lnum, line);
            let mut ci = utf_ptr2StrCharInfo(line);
            col = 0;
            while col <= wcol && *ci.ptr != NUL {
                let cli = utf_ClusterInfo(ci);
                let cs: CharSize =
                    win_charsize(cstype, col, ci.ptr, ci.chr.value, &raw mut csarg, cli.cells);
                csize = cs.width;
                head = cs.head;
                col += cs.width;
                ci = cli.next;
            }
            idx = ci.ptr.offset_from(line) as c_int;

            if col > wcol || (!virtual_active(wp) && one_more == 0) {
                idx -= 1;
                csize -= head;
                col -= csize;
            }

            if virtual_active(wp)
                && addspaces
                && wcol >= 0
                && ((col != wcol && col != wcol + 1) || csize > 1)
            {
                if *line.add(idx as usize) == NUL {
                    let correct = wcol - col;
                    let newline_size = (idx as usize)
                        .checked_add(correct as usize)
                        .expect("STRICT_ADD overflow");
                    let mut newline = vec![0 as c_char; newline_size + 1];
                    ptr::copy_nonoverlapping(line, newline.as_mut_ptr(), idx as usize);
                    ptr::write_bytes(
                        newline.as_mut_ptr().add(idx as usize),
                        b' ',
                        correct as usize,
                    );

                    ml_replace(pos.lnum, newline.as_mut_ptr(), true);
                    inserted_bytes(pos.lnum, idx, 0, correct);
                    idx += correct;
                    col = wcol;
                } else {
                    let correct = wcol - col - csize + 1;
                    if -correct > csize {
                        return FAIL;
                    }

                    let alloc_n = ((linelen - 1) as usize)
                        .checked_add(csize as usize)
                        .expect("STRICT_ADD overflow");
                    let mut newline = vec![0 as c_char; alloc_n + 1];
                    ptr::copy_nonoverlapping(line, newline.as_mut_ptr(), idx as usize);
                    ptr::write_bytes(newline.as_mut_ptr().add(idx as usize), b' ', csize as usize);
                    let rest_n = (linelen as usize)
                        .checked_sub(idx as usize)
                        .and_then(|v| v.checked_sub(1))
                        .expect("STRICT_SUB underflow");
                    ptr::copy_nonoverlapping(
                        line.add(idx as usize + 1),
                        newline.as_mut_ptr().add((idx + csize) as usize),
                        rest_n,
                    );

                    ml_replace(pos.lnum, newline.as_mut_ptr(), true);
                    inserted_bytes(pos.lnum, idx, 1, csize);
                    idx += csize - 1 + correct;
                    col += correct;
                }
            }
        }

        pos.col = idx.max(0);
        pos.coladd = 0;

        if finetune {
            if wcol == MAXCOL {
                if one_more == 0 {
                    let mut scol: colnr_T = 0;
                    let mut ecol: colnr_T = 0;
                    getvcol(wp, pos, &raw mut scol, ptr::null_mut(), &raw mut ecol, 0);
                    pos.coladd = ecol - scol;
                }
            } else {
                let b = wcol - col;
                if b > 0 && b < (MAXCOL - 2 * (*wp).w_view_width) {
                    pos.coladd = b;
                }
                col += b;
            }
        }

        mark_mb_adjustpos((*wp).w_buffer, pos);

        if wcol < 0 || col < wcol {
            return FAIL;
        }
        OK
    }
}

/// Return in `pos` the position of the cursor advanced to screen column `wcol` (`getvpos`).
pub unsafe fn getvpos(wp: *mut win_T, pos: &mut pos_T, wcol: colnr_T) -> c_int {
    unsafe { coladvance2(wp, pos, false, virtual_active(wp), wcol) }
}

/// Increment the cursor position (`inc_cursor`).
pub unsafe fn inc_cursor() -> c_int {
    unsafe { inc(&mut (**curwin.as_ptr()).w_cursor) }
}

/// Decrement the cursor position (`dec_cursor`).
pub unsafe fn dec_cursor() -> c_int {
    unsafe { dec(&mut (**curwin.as_ptr()).w_cursor) }
}

/// Get the line number relative to the current cursor position (`get_cursor_rel_lnum`).
pub unsafe fn get_cursor_rel_lnum(wp: *mut win_T, lnum: linenr_T) -> linenr_T {
    unsafe {
        let cursor = (*wp).w_cursor.lnum;
        if lnum == cursor || hasAnyFolding(wp) == 0 {
            return lnum - cursor;
        }

        let mut from_line = lnum.min(cursor);
        let to_line = lnum.max(cursor);
        let mut retval: linenr_T = 0;

        while from_line < to_line {
            hasFolding(wp, from_line, ptr::null_mut(), &raw mut from_line);
            from_line += 1;
            retval += 1;
        }

        if from_line > to_line {
            retval -= 1;
        }

        if lnum < cursor { -retval } else { retval }
    }
}

/// Make sure `pos.lnum` and `pos.col` are valid in `buf` (`check_pos`).
pub unsafe fn check_pos(buf: *mut buf_T, pos: &mut pos_T) {
    unsafe {
        pos.lnum = pos.lnum.min((*buf).b_ml.ml_line_count);
        if pos.col > 0 {
            pos.col = pos.col.min(ml_get_buf_len(buf, pos.lnum));
        }
    }
}

/// Make sure `win->w_cursor.lnum` is valid (`check_cursor_lnum`).
pub unsafe fn check_cursor_lnum(win: *mut win_T) {
    unsafe {
        let buf = (*win).w_buffer;
        if (*win).w_cursor.lnum > (*buf).b_ml.ml_line_count
            && !hasFolding(
                win,
                (*buf).b_ml.ml_line_count,
                &raw mut (*win).w_cursor.lnum,
                ptr::null_mut(),
            )
        {
            (*win).w_cursor.lnum = (*buf).b_ml.ml_line_count;
        }
        if (*win).w_cursor.lnum <= 0 {
            (*win).w_cursor.lnum = 1;
        }
    }
}

/// Make sure `win->w_cursor.col` is valid (`check_cursor_col`).
pub unsafe fn check_cursor_col(win: *mut win_T) {
    unsafe {
        let oldcol = (*win).w_cursor.col;
        let oldcoladd = (*win).w_cursor.col + (*win).w_cursor.coladd;
        let cur_ve_flags = get_ve_flags(win);

        let len = ml_get_buf_len((*win).w_buffer, (*win).w_cursor.lnum);
        if len == 0 {
            (*win).w_cursor.col = 0;
        } else if (*win).w_cursor.col >= len {
            if (g_st().State & MODE_INSERT) != 0
                || g_st().restart_edit != 0
                || (g_st().State & MODE_TERMINAL) != 0
                || (g_st().Visual.active && *opt_vars().p_sel != b'o' as c_char)
                || (cur_ve_flags & kOptVeFlagOnemore) != 0
                || virtual_active(win)
            {
                (*win).w_cursor.col = len;
            } else {
                (*win).w_cursor.col = len - 1;
                mark_mb_adjustpos((*win).w_buffer, &mut (*win).w_cursor);
            }
        } else if (*win).w_cursor.col < 0 {
            (*win).w_cursor.col = 0;
        }

        if oldcol == MAXCOL {
            (*win).w_cursor.coladd = 0;
        } else if cur_ve_flags == kOptVeFlagAll {
            if oldcoladd > (*win).w_cursor.col {
                (*win).w_cursor.coladd = oldcoladd - (*win).w_cursor.col;

                if (*win).w_cursor.col + 1 < len {
                    debug_assert!((*win).w_cursor.coladd > 0);
                    let mut cs: colnr_T = 0;
                    let mut ce: colnr_T = 0;
                    getvcol(
                        win,
                        &mut (*win).w_cursor,
                        &raw mut cs,
                        ptr::null_mut(),
                        &raw mut ce,
                        0,
                    );
                    (*win).w_cursor.coladd = (*win).w_cursor.coladd.min(ce - cs);
                }
            } else {
                (*win).w_cursor.coladd = 0;
            }
        }
    }
}

/// Make sure `curwin->w_cursor` is on a valid character (`check_cursor`).
pub unsafe fn check_cursor(wp: *mut win_T) {
    unsafe {
        check_cursor_lnum(wp);
        check_cursor_col(wp);
    }
}

/// Check if `Visual.start` position is valid, correct it if not (`check_visual_pos`).
pub unsafe fn check_visual_pos() {
    unsafe {
        if g_st().Visual.start.lnum > (*g_st().curbuf).b_ml.ml_line_count {
            g_st().Visual.start.lnum = (*g_st().curbuf).b_ml.ml_line_count;
            g_st().Visual.start.col = 0;
            g_st().Visual.start.coladd = 0;
        } else {
            let len = ml_get_len(g_st().Visual.start.lnum);
            if g_st().Visual.start.col > len {
                g_st().Visual.start.col = len;
                g_st().Visual.start.coladd = 0;
            }
        }
    }
}

/// Make sure `curwin->w_cursor` is not on the NUL at the end of the line (`adjust_cursor_col`).
pub unsafe fn adjust_cursor_col() {
    unsafe {
        if (**curwin.as_ptr()).w_cursor.col > 0
            && (!g_st().Visual.active || *opt_vars().p_sel == b'o' as c_char)
            && gchar_cursor() == 0
        {
            (**curwin.as_ptr()).w_cursor.col -= 1;
        }
    }
}

/// Set `curwin->w_leftcol` to `leftcol` (`set_leftcol`).
pub unsafe fn set_leftcol(leftcol: colnr_T) -> bool {
    unsafe {
        if (**curwin.as_ptr()).w_leftcol == leftcol {
            return false;
        }
        (**curwin.as_ptr()).w_leftcol = leftcol;

        changed_cline_bef_curs(*curwin.as_ptr());
        let lastcol = i64::from((**curwin.as_ptr()).w_leftcol) + i64::from((**curwin.as_ptr()).w_view_width)
            - i64::from(win_col_off(*curwin.as_ptr()))
            - 1;
        validate_virtcol(*curwin.as_ptr());

        let mut retval = false;
        let siso = get_sidescrolloff_value(*curwin.as_ptr());
        if (**curwin.as_ptr()).w_virtcol > (lastcol - siso) as colnr_T {
            retval = true;
            coladvance(*curwin.as_ptr(), (lastcol - siso) as colnr_T);
        } else if (**curwin.as_ptr()).w_virtcol < (i64::from((**curwin.as_ptr()).w_leftcol) + siso) as colnr_T {
            retval = true;
            coladvance(*curwin.as_ptr(), (i64::from((**curwin.as_ptr()).w_leftcol) + siso) as colnr_T);
        }

        let mut s: colnr_T = 0;
        let mut e: colnr_T = 0;
        getvvcol(
            *curwin.as_ptr(),
            &mut (**curwin.as_ptr()).w_cursor,
            &raw mut s,
            ptr::null_mut(),
            &raw mut e,
            0,
        );
        if e > lastcol as colnr_T {
            retval = true;
            coladvance(*curwin.as_ptr(), s - 1);
        } else if s < (**curwin.as_ptr()).w_leftcol {
            retval = true;
            if coladvance(*curwin.as_ptr(), e + 1) == FAIL {
                (**curwin.as_ptr()).w_leftcol = s;
                changed_cline_bef_curs(*curwin.as_ptr());
            }
        }

        if retval {
            (**curwin.as_ptr()).w_set_curswant = true;
        }
        redraw_later(*curwin.as_ptr(), UPD_NOT_VALID);
        retval
    }
}

pub unsafe fn gchar_cursor() -> c_int {
    unsafe { utf_ptr2char(get_cursor_pos_ptr()) }
}

/// Return the character immediately before the cursor (`char_before_cursor`).
pub unsafe fn char_before_cursor() -> c_int {
    unsafe {
        if (**curwin.as_ptr()).w_cursor.col == 0 {
            return -1;
        }

        let line = get_cursor_line_ptr();
        let p = line.add((**curwin.as_ptr()).w_cursor.col as usize);
        let prev_len = utf_head_off(line, p.sub(1)) + 1;
        utf_ptr2char(p.sub(prev_len as usize))
    }
}

/// Write a character at the current cursor position (`pchar_cursor`).
pub unsafe fn pchar_cursor(c: c_char) {
    unsafe {
        *ml_get_buf_mut(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum).add((**curwin.as_ptr()).w_cursor.col as usize) = c;
    }
}

/// Return pointer to cursor line (`get_cursor_line_ptr`).
pub unsafe fn get_cursor_line_ptr() -> *mut c_char {
    unsafe { ml_get_buf(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum) }
}

/// Return pointer to cursor position (`get_cursor_pos_ptr`).
pub unsafe fn get_cursor_pos_ptr() -> *mut c_char {
    unsafe { ml_get_buf(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum).add((**curwin.as_ptr()).w_cursor.col as usize) }
}

/// Return length (excluding the NUL) of the cursor line (`get_cursor_line_len`).
pub unsafe fn get_cursor_line_len() -> colnr_T {
    unsafe { ml_get_buf_len(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum) }
}

/// Return length (excluding the NUL) of the cursor position (`get_cursor_pos_len`).
pub unsafe fn get_cursor_pos_len() -> colnr_T {
    unsafe { ml_get_buf_len(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum) - (**curwin.as_ptr()).w_cursor.col }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_ordering_and_clearpos_work() {
        let mut p1 = pos_T {
            lnum: 3,
            col: 5,
            coladd: 1,
        };
        let p2 = pos_T {
            lnum: 3,
            col: 6,
            coladd: 0,
        };
        assert!(lt(p1, p2));
        assert!(ltoreq(p1, p2));
        assert!(ltoreq(p1, p1));
        assert!(!lt(p2, p1));
        clearpos(&mut p1);
        assert!(equalpos(
            p1,
            pos_T {
                lnum: 0,
                col: 0,
                coladd: 0
            }
        ));
    }
}
