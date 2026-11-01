//! Cursor positioning, validation, and line access (`src/nvim/cursor.c`).

use crate::buffer::FileBuf;
use crate::change::{K_OPT_VE_FLAG_ONEMORE, TAB, inserted_bytes};
use crate::drawscreen::{UPD_NOT_VALID, redraw_later};
use crate::fold::{has_any_folding, has_folding};
use crate::globals::*;
use crate::mark::mark_mb_adjustpos;
use crate::mbyte::{utf_cluster_info, utf_head_off, utf_ptr2_str_char_info, utf_ptr2char};
use crate::memline::{
    dec, inc, ml_get_buf, ml_get_buf_len, ml_get_buf_mut, ml_get_buf_slice, ml_get_len,
    ml_get_pos_slice, ml_replace,
};
use crate::r#move::{changed_cline_bef_curs, set_valid_virtcol, validate_virtcol, win_col_off};
use crate::option::{get_sidescrolloff_value, get_ve_flags};
use crate::option_vars::*;
use crate::plines::{
    CSType, CharSize, CharsizeArg, disp_vcol2vcol, getvcol, getvvcol, init_charsize_arg,
    linetabsize, linetabsize_disp_eol, vcol2disp_vcol, win_charsize,
};
use crate::regexp::MAXCOL;
use crate::state::{MODE_INSERT, MODE_TERMINAL, virtual_active};
use crate::strings::RawCCharPtrMut;
use crate::window::WinInfoInfo;
use std::ptr;

pub type Colnr = i32;
pub type Linenr = i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CursorPos {
    pub lnum: i32,
    pub col: i32,
    pub coladd: i32,
}

impl CursorPos {
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
pub struct Lpos {
    pub lnum: i32,
    pub col: i32,
}

impl Lpos {
    #[inline]
    pub const fn new() -> Self {
        Self { lnum: 0, col: 0 }
    }
}

#[inline]
pub const fn equalpos(a: CursorPos, b: CursorPos) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
pub const fn lt(a: CursorPos, b: CursorPos) -> bool {
    if a.lnum != b.lnum {
        a.lnum < b.lnum
    } else if a.col != b.col {
        a.col < b.col
    } else {
        a.coladd < b.coladd
    }
}

#[inline]
pub const fn clearpos(a: &mut CursorPos) {
    a.lnum = 0;
    a.col = 0;
    a.coladd = 0;
}

#[inline]
pub const fn ltoreq(a: CursorPos, b: CursorPos) -> bool {
    lt(a, b) || equalpos(a, b)
}

pub(crate) const VALID_VIRTCOL: i32 = 0x04;

pub(crate) const K_OPT_VE_FLAG_ALL: u32 = 0x04;

/// Return the screen position of the cursor (`getviscol`).
pub fn getviscol() -> i32 {
    let mut x: Colnr = 0;
    getvvcol(
        curwin_mut(),
        &mut curwin_mut().w_cursor,
        Some(&mut x),
        None,
        None,
        0,
    );
    x as i32
}

/// Return the screen position of character `col` with a `coladd` in the cursor line (`getviscol2`).
pub fn getviscol2(col: Colnr, coladd: Colnr) -> i32 {
    let mut x: Colnr = 0;
    let mut pos = CursorPos {
        lnum: curwin_mut().w_cursor.lnum,
        col,
        coladd,
    };
    getvvcol(curwin_mut(), &mut pos, Some(&mut x), None, None, 0);
    x as i32
}

/// Go to column `wcol`, and add/insert white space as necessary (`coladvance_force`).
pub fn coladvance_force(wcol: Colnr) -> bool {
    let rc = coladvance2(curwin_mut(), &mut curwin_mut().w_cursor, true, false, wcol);

    if wcol == MAXCOL {
        curwin_mut().w_valid &= !VALID_VIRTCOL;
    } else {
        set_valid_virtcol(curwin_mut(), wcol);
    }
    rc
}

/// Try to advance the Cursor to the specified screen column (`coladvance`).
pub fn coladvance(wp: &mut WinInfoInfo, wcol: Colnr) -> bool {
    let rc = getvpos(wp, &mut wp.w_cursor, wcol);

    if wcol == MAXCOL || !rc {
        wp.w_valid &= !VALID_VIRTCOL;
    } else if crate::memory::cstr_byte_at(
        ml_get_buf(wp.buf_mut(), wp.w_cursor.lnum),
        wp.w_cursor.col as usize,
    ) != TAB
    {
        set_valid_virtcol(curwin_mut(), wcol);
    }
    rc
}

fn coladvance2(
    wp: &mut WinInfoInfo,
    pos: &mut CursorPos,
    addspaces: bool,
    finetune: bool,
    wcol_arg: Colnr,
) -> bool {
    debug_assert!(ptr::eq(wp, curwin_ptr()) || !addspaces);
    let mut wcol = wcol_arg;
    let mut idx: i32;
    let mut col: Colnr;
    let mut head: i32 = 0;

    let one_more = i32::from(
        (g_st().state_global & MODE_INSERT) != 0
            || (g_st().state_global & MODE_TERMINAL) != 0
            || g_st().restart_edit != 0
            || (g_st().visual_global.active
                && crate::memory::cstr_first_byte(opt_vars().p_sel) != b'o')
            || ((get_ve_flags(&mut *wp) & K_OPT_VE_FLAG_ONEMORE) != 0 && wcol < MAXCOL),
    );

    if wcol == MAXCOL {
        idx = ml_get_buf_len(wp.buf_mut(), pos.lnum) - 1 + one_more;
        col = wcol;

        if (addspaces || finetune) && !g_st().visual_global.active {
            wp.w_curswant = linetabsize(wp, pos.lnum) + one_more;
            if wp.w_curswant > 0 {
                wp.w_curswant -= 1;
            }
        }
    } else {
        let width = wp.w_view_width - win_col_off(wp);
        let mut csize: i32 = 0;

        if finetune
            && wp.w_onebuf_opt.wo_wrap != 0
            && wp.w_view_width != 0
            && wcol >= width
            && width > 0
        {
            csize = linetabsize_disp_eol(&mut *wp, pos.lnum);
            if csize > 0 {
                csize -= 1;
            }

            let disp_wcol = vcol2disp_vcol(&mut *wp, pos.lnum, wcol);
            if disp_wcol / width > csize / width
                && ((g_st().state_global & MODE_INSERT) == 0 || disp_wcol > csize + 1)
            {
                wcol = disp_vcol2vcol(&mut *wp, pos.lnum, (csize / width + 1) * width - 1);
            }
        }

        let line = ml_get_buf(wp.buf_mut(), pos.lnum);
        let linelen = ml_get_buf_len(wp.buf_mut(), pos.lnum);
        let mut csarg = CharsizeArg::new();
        let cstype: CSType = init_charsize_arg(&mut csarg, &mut *wp, pos.lnum, line);
        let mut ci = utf_ptr2_str_char_info(line);
        col = 0;
        while col <= wcol && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_cluster_info(ci);
            let cs: CharSize =
                win_charsize(cstype, col, ci.ptr, ci.chr.value, &mut csarg, cli.cells);
            csize = cs.width;
            head = cs.head;
            col += cs.width;
            ci = cli.next;
        }
        idx = crate::memory::ptr_offset_from(ci.ptr, line) as i32;

        if col > wcol || (!virtual_active(&mut *wp) && one_more == 0) {
            idx -= 1;
            csize -= head;
            col -= csize;
        }

        if virtual_active(&mut *wp)
            && addspaces
            && wcol >= 0
            && ((col != wcol && col != wcol + 1) || csize > 1)
        {
            if crate::memory::cstr_byte_at(line, idx as usize) == 0 {
                let correct = wcol - col;
                let idx_usize = idx as usize;
                let correct_usize = correct as usize;
                let newline_size = idx_usize
                    .checked_add(correct_usize)
                    .expect("STRICT_ADD overflow");
                let mut newline = vec![0u8; newline_size + 1];
                newline[..idx_usize].copy_from_slice(crate::memory::cstr_slice(line, idx_usize));
                newline[idx_usize..idx_usize + correct_usize].fill(b' ');

                ml_replace(pos.lnum, newline.as_mut_ptr().cast(), true);
                inserted_bytes(pos.lnum, idx, 0, correct);
                idx += correct;
                col = wcol;
            } else {
                let correct = wcol - col - csize + 1;
                if -correct > csize {
                    return false;
                }

                let idx_usize = idx as usize;
                let csize_usize = csize as usize;
                let alloc_n = ((linelen - 1) as usize)
                    .checked_add(csize_usize)
                    .expect("STRICT_ADD overflow");
                let mut newline = vec![0u8; alloc_n + 1];
                let line_slice = crate::memory::cstr_slice(line, linelen as usize);
                newline[..idx_usize].copy_from_slice(&line_slice[..idx_usize]);
                newline[idx_usize..idx_usize + csize_usize].fill(b' ');
                let rest_n = (linelen as usize)
                    .checked_sub(idx_usize)
                    .and_then(|v| v.checked_sub(1))
                    .expect("STRICT_SUB underflow");
                newline[idx_usize + csize_usize..idx_usize + csize_usize + rest_n]
                    .copy_from_slice(&line_slice[idx_usize + 1..idx_usize + 1 + rest_n]);

                ml_replace(pos.lnum, newline.as_mut_ptr().cast(), true);
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
                let mut scol: Colnr = 0;
                let mut ecol: Colnr = 0;
                getvcol(wp, pos, Some(&mut scol), None, Some(&mut ecol), 0);
                pos.coladd = ecol - scol;
            }
        } else {
            let b = wcol - col;
            if b > 0 && b < (MAXCOL - 2 * wp.w_view_width) {
                pos.coladd = b;
            }
            col += b;
        }
    }

    mark_mb_adjustpos(wp.w_buffer, pos);

    if wcol < 0 || col < wcol {
        return false;
    }
    true
}

/// Return in `pos` the position of the cursor advanced to screen column `wcol` (`getvpos`).
pub fn getvpos(wp: *mut WinInfoInfo, pos: &mut CursorPos, wcol: Colnr) -> bool {
    unsafe { coladvance2(&mut *wp, pos, false, virtual_active(&mut *wp), wcol) }
}

/// Increment the cursor position (`inc_cursor`).
pub fn inc_cursor() -> i32 {
    inc(&mut curwin_mut().w_cursor)
}

/// Decrement the cursor position (`dec_cursor`).
pub fn dec_cursor() -> i32 {
    dec(&mut curwin_mut().w_cursor)
}

/// Get the line number relative to the current cursor position (`get_cursor_rel_lnum`).
pub fn get_cursor_rel_lnum(wp: &mut WinInfoInfo, lnum: Linenr) -> Linenr {
    let cursor = wp.w_cursor.lnum;
    if lnum == cursor || has_any_folding(wp) == 0 {
        return lnum - cursor;
    }

    let mut from_line = lnum.min(cursor);
    let to_line = lnum.max(cursor);
    let mut retval: Linenr = 0;

    while from_line < to_line {
        has_folding(wp, from_line, None, &raw mut from_line);
        from_line += 1;
        retval += 1;
    }

    if from_line > to_line {
        retval -= 1;
    }

    if lnum < cursor { -retval } else { retval }
}

/// Make sure `pos.lnum` and `pos.col` are valid in `buf` (`check_pos`).
pub fn check_pos(buf: &mut FileBuf, pos: &mut CursorPos) {
    pos.lnum = pos.lnum.min(buf.b_ml.ml_line_count);
    if pos.col > 0 {
        pos.col = pos.col.min(ml_get_buf_len(buf, pos.lnum));
    }
}

/// Make sure `win->w_cursor.lnum` is valid (`check_cursor_lnum`).
pub fn check_cursor_lnum(win: &mut WinInfoInfo) {
    let line_count = win.buf_mut().b_ml.ml_line_count;
    let mut cursor_lnum = win.w_cursor.lnum;
    if cursor_lnum > line_count
        && !has_folding(win, line_count, Some(&mut cursor_lnum), ptr::null_mut())
    {
        cursor_lnum = line_count;
    }
    win.w_cursor.lnum = cursor_lnum.max(1);
}

/// Make sure `win->w_cursor.col` is valid (`check_cursor_col`).
pub fn check_cursor_col(win: &mut WinInfoInfo) {
    let oldcol = win.w_cursor.col;
    let oldcoladd = win.w_cursor.col + win.w_cursor.coladd;
    let cur_ve_flags = get_ve_flags(&mut *win);

    let len = ml_get_buf_len(win.buf_mut(), win.w_cursor.lnum);
    if len == 0 {
        win.w_cursor.col = 0;
    } else if win.w_cursor.col >= len {
        if (g_st().state_global & MODE_INSERT) != 0
            || g_st().restart_edit != 0
            || (g_st().state_global & MODE_TERMINAL) != 0
            || (g_st().visual_global.active
                && crate::memory::cstr_first_byte(opt_vars().p_sel) != b'o')
            || (cur_ve_flags & K_OPT_VE_FLAG_ONEMORE) != 0
            || virtual_active(&mut *win)
        {
            win.w_cursor.col = len;
        } else {
            win.w_cursor.col = len - 1;
            mark_mb_adjustpos(win.w_buffer, &mut win.w_cursor);
        }
    } else if win.w_cursor.col < 0 {
        win.w_cursor.col = 0;
    }

    if oldcol == MAXCOL {
        win.w_cursor.coladd = 0;
    } else if cur_ve_flags == K_OPT_VE_FLAG_ALL {
        if oldcoladd > win.w_cursor.col {
            win.w_cursor.coladd = oldcoladd - win.w_cursor.col;

            if win.w_cursor.col + 1 < len {
                debug_assert!(win.w_cursor.coladd > 0);
                let mut cs: Colnr = 0;
                let mut ce: Colnr = 0;
                let mut w_cursor = win.w_cursor;
                getvcol(win, &mut w_cursor, Some(&mut cs), None, Some(&mut ce), 0);
                win.w_cursor = w_cursor;
                win.w_cursor.coladd = win.w_cursor.coladd.min(ce - cs);
            }
        } else {
            win.w_cursor.coladd = 0;
        }
    }
}

/// Make sure `curwin->w_cursor` is on a valid character (`check_cursor`).
pub fn check_cursor(wp: &mut WinInfoInfo) {
    check_cursor_lnum(wp);
    check_cursor_col(wp);
}

/// Check if `Visual.start` position is valid, correct it if not (`check_visual_pos`).
pub fn check_visual_pos() {
    if g_st().visual_global.start.lnum > curbuf_mut().b_ml.ml_line_count {
        g_st().visual_global.start.lnum = curbuf_mut().b_ml.ml_line_count;
        g_st().visual_global.start.col = 0;
        g_st().visual_global.start.coladd = 0;
    } else {
        let len = ml_get_len(g_st().visual_global.start.lnum);
        if g_st().visual_global.start.col > len {
            g_st().visual_global.start.col = len;
            g_st().visual_global.start.coladd = 0;
        }
    }
}

/// Make sure `curwin->w_cursor` is not on the NUL at the end of the line (`adjust_cursor_col`).
pub fn adjust_cursor_col() {
    if curwin_mut().w_cursor.col > 0
        && (!g_st().visual_global.active
            || crate::memory::cstr_first_byte(opt_vars().p_sel) == b'o')
        && gchar_cursor() == 0
    {
        curwin_mut().w_cursor.col -= 1;
    }
}

/// Set `curwin->w_leftcol` to `leftcol` (`set_leftcol`).
pub fn set_leftcol(leftcol: Colnr) -> bool {
    if curwin_mut().w_leftcol == leftcol {
        return false;
    }
    curwin_mut().w_leftcol = leftcol;

    changed_cline_bef_curs(curwin_mut());
    let lastcol = i64::from(curwin_mut().w_leftcol) + i64::from(curwin_mut().w_view_width)
        - i64::from(win_col_off(curwin_ptr()))
        - 1;
    validate_virtcol(curwin_mut());

    let mut retval = false;
    let siso = get_sidescrolloff_value(curwin_mut());
    if curwin_mut().w_virtcol > (lastcol - siso) as Colnr {
        retval = true;
        coladvance(curwin_mut(), (lastcol - siso) as Colnr);
    } else if curwin_mut().w_virtcol < (i64::from(curwin_mut().w_leftcol) + siso) as Colnr {
        retval = true;
        coladvance(
            curwin_mut(),
            (i64::from(curwin_mut().w_leftcol) + siso) as Colnr,
        );
    }

    let mut s: Colnr = 0;
    let mut e: Colnr = 0;
    getvvcol(
        curwin_mut(),
        &mut curwin_mut().w_cursor,
        Some(&mut s),
        None,
        Some(&mut e),
        0,
    );
    if e > lastcol as Colnr {
        retval = true;
        coladvance(curwin_mut(), s - 1);
    } else if s < curwin_mut().w_leftcol {
        retval = true;
        if !coladvance(curwin_mut(), e + 1) {
            curwin_mut().w_leftcol = s;
            changed_cline_bef_curs(curwin_mut());
        }
    }

    if retval {
        curwin_mut().w_set_curswant = true;
    }
    redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
    retval
}

pub fn gchar_cursor() -> i32 {
    utf_ptr2char(get_cursor_pos_ptr())
}

/// Return the character immediately before the cursor (`char_before_cursor`).
pub fn char_before_cursor() -> i32 {
    if curwin_mut().w_cursor.col == 0 {
        return -1;
    }

    let line = get_cursor_line_ptr();
    let p = line.wrapping_add(curwin_mut().w_cursor.col as usize);
    let prev_len = utf_head_off(line, p.wrapping_sub(1)) + 1;
    utf_ptr2char(p.wrapping_sub(prev_len as usize))
}

/// Write a character at the current cursor position (`pchar_cursor`).
pub fn pchar_cursor(c: u8) {
    unsafe {
        *ml_get_buf_mut(curbuf_mut(), curwin_mut().w_cursor.lnum)
            .wrapping_add(curwin_mut().w_cursor.col as usize)
            .cast::<u8>() = c;
    }
}

/// Return pointer to cursor line (`get_cursor_line_ptr`).
pub fn get_cursor_line_ptr() -> RawCCharPtrMut {
    ml_get_buf(crate::globals::curbuf_mut(), curwin_mut().w_cursor.lnum)
}

/// Return pointer to cursor position (`get_cursor_pos_ptr`).
pub fn get_cursor_pos_ptr() -> RawCCharPtrMut {
    ml_get_buf(crate::globals::curbuf_mut(), curwin_mut().w_cursor.lnum)
        .wrapping_add(curwin_mut().w_cursor.col as usize)
}

/// Return the bytes (excluding the NUL) of the cursor line.
#[inline]
pub fn get_cursor_line_slice() -> &'static [u8] {
    ml_get_buf_slice(crate::globals::curbuf_mut(), curwin_mut().w_cursor.lnum)
}

/// Return the bytes (excluding the NUL) starting at the cursor position.
#[inline]
pub fn get_cursor_pos_slice() -> &'static [u8] {
    let cursor = curwin_mut().w_cursor;
    ml_get_pos_slice(&cursor)
}

/// Return length (excluding the NUL) of the cursor line (`get_cursor_line_len`).
pub fn get_cursor_line_len() -> Colnr {
    ml_get_buf_len(crate::globals::curbuf_mut(), curwin_mut().w_cursor.lnum)
}

/// Return length (excluding the NUL) of the cursor position (`get_cursor_pos_len`).
pub fn get_cursor_pos_len() -> Colnr {
    ml_get_buf_len(crate::globals::curbuf_mut(), curwin_mut().w_cursor.lnum)
        - curwin_mut().w_cursor.col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_ordering_and_clearpos_work() {
        let mut p1 = CursorPos {
            lnum: 3,
            col: 5,
            coladd: 1,
        };
        let p2 = CursorPos {
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
            CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0
            }
        ));
    }
}
