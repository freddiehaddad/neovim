//! Cursor positioning, validation, and line access (`src/nvim/cursor.c`).
#![forbid(unsafe_code)]

use crate::buffer::FileBuf;
use crate::change::{K_OPT_VE_FLAG_ONEMORE, TAB, inserted_bytes};
use crate::drawscreen::{UPD_NOT_VALID, redraw_later};
use crate::fold::{has_any_folding, has_folding};
use crate::globals::*;
use crate::mark::mark_mb_adjustpos;
use crate::mbyte::{utf_cluster_info, utf_ptr2_str_char_info};
use crate::memline::{
    dec, inc, ml_get_buf, ml_get_buf_len, ml_get_buf_slice, ml_get_len, ml_get_pos_slice,
    ml_replace,
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
use crate::window::WinInfoInfo;

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

pub trait CursorPosCellExt {
    fn set_lnum(&self, v: Linenr);
    fn set_col(&self, v: Colnr);
    fn set_coladd(&self, v: Colnr);
    fn add_lnum(&self, d: Linenr);
    fn sub_lnum(&self, d: Linenr);
    fn add_col(&self, d: Colnr);
    fn sub_col(&self, d: Colnr);
    fn add_coladd(&self, d: Colnr);
    fn sub_coladd(&self, d: Colnr);
    fn update_mut<R>(&self, f: impl FnOnce(&mut CursorPos) -> R) -> R;
}

impl CursorPosCellExt for std::cell::Cell<CursorPos> {
    #[inline(always)]
    fn set_lnum(&self, v: Linenr) {
        let mut p = self.get();
        p.lnum = v;
        self.set(p);
    }
    #[inline(always)]
    fn set_col(&self, v: Colnr) {
        let mut p = self.get();
        p.col = v;
        self.set(p);
    }
    #[inline(always)]
    fn set_coladd(&self, v: Colnr) {
        let mut p = self.get();
        p.coladd = v;
        self.set(p);
    }
    #[inline(always)]
    fn add_lnum(&self, d: Linenr) {
        let mut p = self.get();
        p.lnum += d;
        self.set(p);
    }
    #[inline(always)]
    fn sub_lnum(&self, d: Linenr) {
        let mut p = self.get();
        p.lnum -= d;
        self.set(p);
    }
    #[inline(always)]
    fn add_col(&self, d: Colnr) {
        let mut p = self.get();
        p.col += d;
        self.set(p);
    }
    #[inline(always)]
    fn sub_col(&self, d: Colnr) {
        let mut p = self.get();
        p.col -= d;
        self.set(p);
    }
    #[inline(always)]
    fn add_coladd(&self, d: Colnr) {
        let mut p = self.get();
        p.coladd += d;
        self.set(p);
    }
    #[inline(always)]
    fn sub_coladd(&self, d: Colnr) {
        let mut p = self.get();
        p.coladd -= d;
        self.set(p);
    }
    #[inline(always)]
    fn update_mut<R>(&self, f: impl FnOnce(&mut CursorPos) -> R) -> R {
        let mut p = self.get();
        let r = f(&mut p);
        self.set(p);
        r
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
    let mut cursor = curwin_mut().w_cursor.get();
    getvvcol(curwin_mut(), &mut cursor, Some(&mut x), None, None, 0);
    curwin_mut().w_cursor.set(cursor);
    x as i32
}

/// Return the screen position of character `col` with a `coladd` in the cursor line (`getviscol2`).
pub fn getviscol2(col: Colnr, coladd: Colnr) -> i32 {
    let mut x: Colnr = 0;
    let mut pos = CursorPos {
        lnum: curwin_mut().w_cursor.get().lnum,
        col,
        coladd,
    };
    getvvcol(curwin_mut(), &mut pos, Some(&mut x), None, None, 0);
    x as i32
}

/// Go to column `wcol`, and add/insert white space as necessary (`coladvance_force`).
pub fn coladvance_force(wcol: Colnr) -> bool {
    let mut cursor = curwin_mut().w_cursor.get();
    let rc = coladvance2(curwin_mut(), &mut cursor, true, false, wcol);
    curwin_mut().w_cursor.set(cursor);

    if wcol == MAXCOL {
        curwin_mut()
            .w_valid
            .set(curwin_mut().w_valid.get() & (!VALID_VIRTCOL));
    } else {
        set_valid_virtcol(curwin_mut(), wcol);
    }
    rc
}

/// Try to advance the Cursor to the specified screen column (`coladvance`).
pub fn coladvance(wp: &WinInfoInfo, wcol: Colnr) -> bool {
    let mut cursor = wp.w_cursor.get();
    let rc = getvpos(wp, &mut cursor, wcol);
    wp.w_cursor.set(cursor);

    if wcol == MAXCOL || !rc {
        wp.w_valid.set(wp.w_valid.get() & (!VALID_VIRTCOL));
    } else if crate::memline::ml_get_buf_slice(wp.buf_mut(), wp.w_cursor.get().lnum)
        .get(wp.w_cursor.get().col as usize)
        != Some(&TAB)
    {
        set_valid_virtcol(curwin_mut(), wcol);
    }
    rc
}

fn coladvance2(
    wp: &WinInfoInfo,
    pos: &mut CursorPos,
    addspaces: bool,
    finetune: bool,
    wcol_arg: Colnr,
) -> bool {
    debug_assert!(wp == curwin_ptr() || !addspaces);
    let mut wcol = wcol_arg;
    let mut idx: i32;
    let mut col: Colnr;
    let mut head: i32 = 0;

    let one_more = i32::from(
        (g_st().state_global.get() & MODE_INSERT) != 0
            || (g_st().state_global.get() & MODE_TERMINAL) != 0
            || g_st().restart_edit.get() != 0
            || (g_st().visual_global.borrow().active && opt_vars().p_sel.first_byte() != b'o')
            || ((get_ve_flags(wp) & K_OPT_VE_FLAG_ONEMORE) != 0 && wcol < MAXCOL),
    );

    if wcol == MAXCOL {
        idx = ml_get_buf_len(wp.buf_mut(), pos.lnum) - 1 + one_more;
        col = wcol;

        if (addspaces || finetune) && !g_st().visual_global.borrow().active {
            let cw = linetabsize(wp, pos.lnum) + one_more;
            wp.w_curswant.set(cw);
            if wp.w_curswant.get() > 0 {
                wp.w_curswant.set(wp.w_curswant.get() - (1));
            }
        }
    } else {
        let width = wp.w_view_width.get() - win_col_off(wp);
        let mut csize: i32 = 0;

        if finetune
            && wp.w_onebuf_opt.wo_wrap.get()
            && wp.w_view_width.get() != 0
            && wcol >= width
            && width > 0
        {
            csize = linetabsize_disp_eol(wp, pos.lnum);
            if csize > 0 {
                csize -= 1;
            }

            let disp_wcol = vcol2disp_vcol(wp, pos.lnum, wcol);
            if disp_wcol / width > csize / width
                && ((g_st().state_global.get() & MODE_INSERT) == 0 || disp_wcol > csize + 1)
            {
                wcol = disp_vcol2vcol(wp, pos.lnum, (csize / width + 1) * width - 1);
            }
        }

        let line_cstr = ml_get_buf(wp.buf_mut(), pos.lnum);
        let linelen = ml_get_buf_len(wp.buf_mut(), pos.lnum);
        let mut csarg = CharsizeArg::new();
        let cstype: CSType = init_charsize_arg(&mut csarg, wp, pos.lnum, line_cstr);
        let mut ci = utf_ptr2_str_char_info(line_cstr);
        col = 0;
        while col <= wcol && !ci.ptr.is_empty() {
            let cli = utf_cluster_info(ci);
            let cs: CharSize =
                win_charsize(cstype, col, ci.cstr(), ci.chr.value, &mut csarg, cli.cells);
            csize = cs.width;
            head = cs.head;
            col += cs.width;
            ci = cli.next;
        }
        idx = ci.offset_from(line_cstr) as i32;

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
            let line_bytes = line_cstr.to_bytes();
            if line_bytes.get(idx as usize..).is_none_or(<[u8]>::is_empty) {
                let correct = wcol - col;
                let idx_usize = idx as usize;
                let correct_usize = correct as usize;
                let newline_size = idx_usize
                    .checked_add(correct_usize)
                    .expect("STRICT_ADD overflow");
                let mut newline = vec![0u8; newline_size + 1];
                newline[..idx_usize].copy_from_slice(&line_bytes[..idx_usize]);
                newline[idx_usize..idx_usize + correct_usize].fill(b' ');

                ml_replace(pos.lnum, crate::memory::bytes_until_nul(&newline), true);
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
                let line_slice = &line_cstr.to_bytes_with_nul()
                    [..(linelen as usize).min(line_cstr.to_bytes_with_nul().len())];
                newline[..idx_usize].copy_from_slice(&line_slice[..idx_usize]);
                newline[idx_usize..idx_usize + csize_usize].fill(b' ');
                let rest_n = (linelen as usize)
                    .checked_sub(idx_usize)
                    .and_then(|v| v.checked_sub(1))
                    .expect("STRICT_SUB underflow");
                newline[idx_usize + csize_usize..idx_usize + csize_usize + rest_n]
                    .copy_from_slice(&line_slice[idx_usize + 1..idx_usize + 1 + rest_n]);

                ml_replace(pos.lnum, crate::memory::bytes_until_nul(&newline), true);
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
            if b > 0 && b < (MAXCOL - 2 * wp.w_view_width.get()) {
                pos.coladd = b;
            }
            col += b;
        }
    }

    mark_mb_adjustpos(wp.buf_mut(), pos);

    if wcol < 0 || col < wcol {
        return false;
    }
    true
}

/// Return in `pos` the position of the cursor advanced to screen column `wcol` (`getvpos`).
pub fn getvpos(wp: &WinInfoInfo, pos: &mut CursorPos, wcol: Colnr) -> bool {
    let addspaces = virtual_active(wp);
    coladvance2(wp, pos, false, addspaces, wcol)
}

/// Increment the cursor position (`inc_cursor`).
pub fn inc_cursor() -> i32 {
    curwin_mut().w_cursor.update_mut(inc)
}

/// Decrement the cursor position (`dec_cursor`).
pub fn dec_cursor() -> i32 {
    curwin_mut().w_cursor.update_mut(dec)
}

/// Get the line number relative to the current cursor position (`get_cursor_rel_lnum`).
pub fn get_cursor_rel_lnum(wp: &WinInfoInfo, lnum: Linenr) -> Linenr {
    let cursor = wp.w_cursor.get().lnum;
    if lnum == cursor || has_any_folding(wp) == 0 {
        return lnum - cursor;
    }

    let mut from_line = lnum.min(cursor);
    let to_line = lnum.max(cursor);
    let mut retval: Linenr = 0;

    while from_line < to_line {
        has_folding(wp, from_line, None, Some(&mut from_line));
        from_line += 1;
        retval += 1;
    }

    if from_line > to_line {
        retval -= 1;
    }

    if lnum < cursor { -retval } else { retval }
}

/// Make sure `pos.lnum` and `pos.col` are valid in `buf` (`check_pos`).
pub fn check_pos(buf: &FileBuf, pos: &mut CursorPos) {
    pos.lnum = pos.lnum.min(buf.b_ml.ml_line_count.get());
    if pos.col > 0 {
        pos.col = pos.col.min(ml_get_buf_len(buf, pos.lnum));
    }
}

/// Make sure `win->w_cursor.lnum` is valid (`check_cursor_lnum`).
pub fn check_cursor_lnum(win: &WinInfoInfo) {
    let line_count = win.buf_mut().b_ml.ml_line_count.get();
    let mut cursor_lnum = win.w_cursor.get().lnum;
    if cursor_lnum > line_count && !has_folding(win, line_count, Some(&mut cursor_lnum), None) {
        cursor_lnum = line_count;
    }
    win.w_cursor.set_lnum(cursor_lnum.max(1));
}

/// Make sure `win->w_cursor.col` is valid (`check_cursor_col`).
pub fn check_cursor_col(win: &WinInfoInfo) {
    let oldcol = win.w_cursor.get().col;
    let oldcoladd = win.w_cursor.get().col + win.w_cursor.get().coladd;
    let cur_ve_flags = get_ve_flags(win);

    let len = ml_get_buf_len(win.buf_mut(), win.w_cursor.get().lnum);
    if len == 0 {
        win.w_cursor.set_col(0);
    } else if win.w_cursor.get().col >= len {
        if (g_st().state_global.get() & MODE_INSERT) != 0
            || g_st().restart_edit.get() != 0
            || (g_st().state_global.get() & MODE_TERMINAL) != 0
            || (g_st().visual_global.borrow().active && opt_vars().p_sel.first_byte() != b'o')
            || (cur_ve_flags & K_OPT_VE_FLAG_ONEMORE) != 0
            || virtual_active(win)
        {
            win.w_cursor.set_col(len);
        } else {
            win.w_cursor.set_col(len - 1);
            win.w_cursor
                .update_mut(|c| mark_mb_adjustpos(win.buf_mut(), c));
        }
    } else if win.w_cursor.get().col < 0 {
        win.w_cursor.set_col(0);
    }

    if oldcol == MAXCOL {
        win.w_cursor.set_coladd(0);
    } else if cur_ve_flags == K_OPT_VE_FLAG_ALL {
        if oldcoladd > win.w_cursor.get().col {
            win.w_cursor.set_coladd(oldcoladd - win.w_cursor.get().col);

            if win.w_cursor.get().col + 1 < len {
                debug_assert!(win.w_cursor.get().coladd > 0);
                let mut cs: Colnr = 0;
                let mut ce: Colnr = 0;
                let mut w_cursor = win.w_cursor.get();
                getvcol(win, &mut w_cursor, Some(&mut cs), None, Some(&mut ce), 0);
                win.w_cursor.set(w_cursor);
                win.w_cursor
                    .set_coladd(win.w_cursor.get().coladd.min(ce - cs));
            }
        } else {
            win.w_cursor.set_coladd(0);
        }
    }
}

/// Make sure `curwin->w_cursor` is on a valid character (`check_cursor`).
pub fn check_cursor(wp: &WinInfoInfo) {
    check_cursor_lnum(wp);
    check_cursor_col(wp);
}

/// Check if `Visual.start` position is valid, correct it if not (`check_visual_pos`).
pub fn check_visual_pos() {
    if g_st().visual_global.borrow().start.lnum > curbuf_mut().b_ml.ml_line_count.get() {
        g_st().visual_global.borrow_mut().start.lnum = curbuf_mut().b_ml.ml_line_count.get();
        g_st().visual_global.borrow_mut().start.col = 0;
        g_st().visual_global.borrow_mut().start.coladd = 0;
    } else {
        let len = ml_get_len(g_st().visual_global.borrow().start.lnum);
        if g_st().visual_global.borrow().start.col > len {
            g_st().visual_global.borrow_mut().start.col = len;
            g_st().visual_global.borrow_mut().start.coladd = 0;
        }
    }
}

/// Make sure `curwin->w_cursor` is not on the NUL at the end of the line (`adjust_cursor_col`).
pub fn adjust_cursor_col() {
    if curwin_mut().w_cursor.get().col > 0
        && (!g_st().visual_global.borrow().active || opt_vars().p_sel.first_byte() == b'o')
        && gchar_cursor() == 0
    {
        curwin_mut().w_cursor.sub_col(1);
    }
}

/// Set `curwin->w_leftcol` to `leftcol` (`set_leftcol`).
pub fn set_leftcol(leftcol: Colnr) -> bool {
    if curwin_mut().w_leftcol.get() == leftcol {
        return false;
    }
    curwin_mut().w_leftcol.set(leftcol);

    changed_cline_bef_curs(curwin_mut());
    let lastcol = i64::from(curwin_mut().w_leftcol.get())
        + i64::from(curwin_mut().w_view_width.get())
        - i64::from(win_col_off(curwin_mut()))
        - 1;
    validate_virtcol(curwin_mut());

    let mut retval = false;
    let siso = get_sidescrolloff_value(curwin_mut());
    if curwin_mut().w_virtcol.get() > (lastcol - siso) as Colnr {
        retval = true;
        coladvance(curwin_mut(), (lastcol - siso) as Colnr);
    } else if curwin_mut().w_virtcol.get()
        < (i64::from(curwin_mut().w_leftcol.get()) + siso) as Colnr
    {
        retval = true;
        coladvance(
            curwin_mut(),
            (i64::from(curwin_mut().w_leftcol.get()) + siso) as Colnr,
        );
    }

    let mut s: Colnr = 0;
    let mut e: Colnr = 0;
    let mut cursor = curwin_mut().w_cursor.get();
    getvvcol(
        curwin_mut(),
        &mut cursor,
        Some(&mut s),
        None,
        Some(&mut e),
        0,
    );
    curwin_mut().w_cursor.set(cursor);
    if e > lastcol as Colnr {
        retval = true;
        coladvance(curwin_mut(), s - 1);
    } else if s < curwin_mut().w_leftcol.get() {
        retval = true;
        if !coladvance(curwin_mut(), e + 1) {
            curwin_mut().w_leftcol.set(s);
            changed_cline_bef_curs(curwin_mut());
        }
    }

    if retval {
        curwin_mut().w_set_curswant.set(true);
    }
    redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
    retval
}

pub fn gchar_cursor() -> i32 {
    crate::mbyte::utf_slice2char(get_cursor_pos_slice())
}

/// Return the character immediately before the cursor (`char_before_cursor`).
pub fn char_before_cursor() -> i32 {
    let col = curwin_mut().w_cursor.get().col as usize;
    if col == 0 {
        return -1;
    }

    let line = get_cursor_line_slice();
    let prev_len = crate::mbyte::utf_head_off_slice(line, col - 1) as usize + 1;
    crate::mbyte::utf_slice2char(&line[col - prev_len..])
}

/// Write a character at the current cursor position (`pchar_cursor`).
pub fn pchar_cursor(c: u8) {
    let col = curwin_mut().w_cursor.get().col as usize;
    with_cursor_line_mut_slice(|s| s[col] = c);
}

/// Return the cursor line as a CStr (`get_cursor_line_ptr`).
pub fn get_cursor_line_ptr() -> &'static std::ffi::CStr {
    let lnum = curwin_mut().w_cursor.get().lnum;
    ml_get_buf(crate::globals::curbuf_mut(), lnum)
}

/// Return the cursor position suffix as a CStr (`get_cursor_pos_ptr`).
pub fn get_cursor_pos_ptr() -> &'static std::ffi::CStr {
    let cursor = curwin_mut().w_cursor.get();
    let line = ml_get_buf(crate::globals::curbuf_mut(), cursor.lnum);
    let col = (cursor.col.max(0) as usize).min(line.to_bytes().len());
    &line[col..]
}

/// Return the bytes (excluding the NUL) of the cursor line.
#[inline]
pub fn get_cursor_line_slice() -> &'static [u8] {
    ml_get_buf_slice(
        crate::globals::curbuf_mut(),
        curwin_mut().w_cursor.get().lnum,
    )
}

/// Mutate the bytes (excluding the NUL) of the cursor line.
#[inline]
pub fn with_cursor_line_mut_slice<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    crate::memline::with_ml_get_buf_mut_slice(
        crate::globals::curbuf_mut(),
        curwin_mut().w_cursor.get().lnum,
        f,
    )
}

/// Return the bytes (excluding the NUL) starting at the cursor position.
#[inline]
pub fn get_cursor_pos_slice() -> &'static [u8] {
    let cursor = curwin_mut().w_cursor.get();
    ml_get_pos_slice(&cursor)
}

/// Return length (excluding the NUL) of the cursor line (`get_cursor_line_len`).
pub fn get_cursor_line_len() -> Colnr {
    ml_get_buf_len(
        crate::globals::curbuf_mut(),
        curwin_mut().w_cursor.get().lnum,
    )
}

/// Return length (excluding the NUL) of the cursor position (`get_cursor_pos_len`).
pub fn get_cursor_pos_len() -> Colnr {
    ml_get_buf_len(
        crate::globals::curbuf_mut(),
        curwin_mut().w_cursor.get().lnum,
    ) - curwin_mut().w_cursor.get().col
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
