//! Vertical and horizontal text display size calculations (`src/nvim/plines.c`).

use crate::api::extmark::ns_in_win;
use crate::buffer::FileBuf;
use crate::charset::{NUL, vim_isprintc, vim_strsize};
use crate::cursor::{Colnr, CursorPos, Linenr, lt, ltoreq};
use crate::decoration::{
    K_MTFILTER_SELECT, K_VPOS_INLINE, K_VTIS_LINES, decor_conceal_line, decor_virt_lines,
};
use crate::diff::{diff_check_fill, diffopt_filler};
use crate::fold::{has_folding, has_folding_win, line_folded};
use crate::globals::*;
use crate::indent::{get_breakindent_win, tabstop_padding};
use crate::keycodes::TAB;
use crate::marktree::{
    K_MTMETA_COUNT, K_MTMETA_INLINE, K_MTMETA_LINES, MarkTreeIter, marktree_itr_current,
    marktree_itr_get_filter, marktree_itr_next_filter, mt_decor, mt_invalid, mt_right,
};
use crate::mbyte::{
    ptr2cells, utf_cluster_info, utf_ptr2_str_char_info, utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::r#move::{win_col_off, win_col_off2};
use crate::option::get_showbreak_value;
use crate::option_vars::*;
use crate::state::{MODE_NORMAL, virtual_active};
use crate::window::WinInfoInfo;
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

use crate::drawscreen::GETVCOL_END_EXCL_LBR;
use crate::insert::K_CHARSIZE_FAST;
use crate::regexp::MAXCOL;
pub type CSType = bool;

const K_INVALID_BYTE_CELLS: c_int = 4;

const K_CHARSIZE_REGULAR: CSType = false;

static INLINE_FILTER: [u32; K_MTMETA_COUNT] = [K_MTFILTER_SELECT, 0, 0, 0, 0];

#[repr(C)]
pub struct CharsizeArg {
    pub win: *mut WinInfoInfo,
    pub line: *mut c_char,
    pub use_tabstop: bool,
    pub indent_width: c_int,
    pub virt_row: c_int,
    pub skip_cur_text: bool,
    pub cur_text_width_left: c_int,
    pub cur_text_width_right: c_int,
    pub max_head_vcol: c_int,
    pub iter: MarkTreeIter,
}

impl CharsizeArg {
    pub const fn new() -> Self {
        Self {
            win: ptr::null_mut(),
            line: ptr::null_mut(),
            use_tabstop: false,
            indent_width: 0,
            virt_row: 0,
            skip_cur_text: false,
            cur_text_width_left: 0,
            cur_text_width_right: 0,
            max_head_vcol: 0,
            iter: MarkTreeIter::new(),
        }
    }
}

impl Default for CharsizeArg {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharSize {
    pub width: c_int,
    pub head: c_int,
    pub tail: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharInfo {
    pub value: i32,
    pub len: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StrCharInfo {
    pub ptr: *const c_char,
    pub chr: CharInfo,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClusterInfo {
    pub next: StrCharInfo,
    pub cells: c_int,
    pub first_compose: bool,
}

#[inline]
pub fn vim_isbreak(c: c_int) -> bool {
    opt_vars().breakat_flags[(c as u8) as usize] != 0
}

#[inline]
pub(crate) fn win_charsize(
    cstype: CSType,
    vcol: Colnr,
    ptr_val: impl crate::strings::IntoCCharPtr,
    chr: i32,
    csarg: &mut CharsizeArg,
    mb_width: c_int,
) -> CharSize {
    let ptr_val = ptr_val.into_c_char_ptr();
    if cstype == K_CHARSIZE_FAST {
        charsize_fast(csarg, ptr_val, vcol, chr, mb_width)
    } else {
        charsize_regular(&mut *csarg, ptr_val, vcol, chr, mb_width)
    }
}

#[inline]
pub fn win_linetabsize(wp: &mut WinInfoInfo, lnum: Linenr, line: *mut c_char, len: Colnr) -> c_int {
    let mut csarg = CharsizeArg::new();
    let cstype = init_charsize_arg(&mut csarg, wp, lnum, line);
    if cstype == K_CHARSIZE_FAST {
        linesize_fast(&csarg, 0, len)
    } else {
        linesize_regular(&mut csarg, 0, len)
    }
}

pub fn win_chartabsize(
    wp: &mut WinInfoInfo,
    p: impl crate::strings::IntoCCharPtr,
    col: Colnr,
) -> c_int {
    let p = p.into_c_char_ptr();
    unsafe {
        let buf = wp.w_buffer;
        if crate::memory::cstr_first_char(p) == TAB
            && (wp.w_onebuf_opt.wo_list == 0 || wp.w_p_lcs_chars.tab1 != 0)
        {
            return tabstop_padding(col, (*buf).b_p_ts, &(*buf).b_p_vts_array);
        }
        ptr2cells(p)
    }
}

pub fn linetabsize_col(startvcol: c_int, s: *mut c_char) -> c_int {
    {
        let mut csarg = CharsizeArg::new();
        let cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, s);
        if cstype == K_CHARSIZE_FAST {
            linesize_fast(&csarg, startvcol, MAXCOL)
        } else {
            linesize_regular(&mut csarg, startvcol, MAXCOL)
        }
    }
}

pub fn linetabsize(wp: &mut WinInfoInfo, lnum: Linenr) -> c_int {
    win_linetabsize(wp, lnum, ml_get_buf(wp.buf_mut(), lnum), MAXCOL)
}

pub fn linetabsize_eol(wp: &mut WinInfoInfo, lnum: Linenr) -> c_int {
    {
        linetabsize(wp, lnum)
            + if wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.eol != 0 {
                1
            } else {
                0
            }
    }
}

pub fn init_charsize_arg(
    csarg: &mut CharsizeArg,
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    line: *mut c_char,
) -> CSType {
    unsafe {
        csarg.win = (&raw mut *wp);
        csarg.line = line;
        csarg.max_head_vcol = 0;
        csarg.cur_text_width_left = 0;
        csarg.cur_text_width_right = 0;
        csarg.virt_row = -1;
        csarg.skip_cur_text = false;
        csarg.indent_width = c_int::MIN;
        csarg.use_tabstop = wp.w_onebuf_opt.wo_list == 0 || wp.w_p_lcs_chars.tab1 != 0;

        if lnum > 0
            && marktree_itr_get_filter(
                &wp.buf_mut().b_marktree,
                lnum - 1,
                0,
                lnum,
                0,
                &INLINE_FILTER,
                &mut csarg.iter,
            )
        {
            csarg.virt_row = lnum - 1;
        }

        if csarg.virt_row >= 0
            || (wp.w_onebuf_opt.wo_wrap != 0
                && (wp.w_onebuf_opt.wo_lbr != 0
                    || wp.w_onebuf_opt.wo_bri != 0
                    || *get_showbreak_value(wp) != NUL))
        {
            K_CHARSIZE_REGULAR
        } else {
            K_CHARSIZE_FAST
        }
    }
}

pub fn init_charsize_arg_skip_cur_text(
    csarg: &mut CharsizeArg,
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    line: *mut c_char,
) -> CSType {
    {
        let mut cstype = init_charsize_arg(csarg, wp, 0, line);
        csarg.skip_cur_text = true;
        if lnum > 0 && wp.buf_mut().b_marktree.meta_root[K_MTMETA_INLINE] > 0 {
            csarg.virt_row = lnum - 1;
            cstype = K_CHARSIZE_REGULAR;
        }
        cstype
    }
}

fn inline_virt_text_width(
    wp: &mut WinInfoInfo,
    iter: &mut MarkTreeIter,
    row: c_int,
    col: c_int,
) -> c_int {
    unsafe {
        let buf = wp.w_buffer;
        let mut width: c_int = 0;
        loop {
            let mark = marktree_itr_current(&(*buf).b_marktree, iter);
            if mark.pos.row != row || mark.pos.col > col {
                break;
            } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, &mut *wp) {
                let decor = mt_decor(mark);
                for vt in crate::decoration::virt_text_iter_mut(if decor.is_ext() {
                    decor.data.ext().vt
                } else {
                    ptr::null_mut()
                }) {
                    if (vt.flags & K_VTIS_LINES) == 0 && vt.pos == K_VPOS_INLINE {
                        width += vt.width;
                    }
                }
            }
            marktree_itr_next_filter(&(*buf).b_marktree, iter, row + 1, 0, &INLINE_FILTER);
        }
        width
    }
}

pub fn charsize_regular(
    csarg: &mut CharsizeArg,
    cur: impl crate::strings::IntoCCharPtr,
    vcol: Colnr,
    cur_char: i32,
    mb_width: c_int,
) -> CharSize {
    let cur = cur.into_c_char_ptr();
    unsafe {
        csarg.cur_text_width_left = 0;
        csarg.cur_text_width_right = 0;

        let wp = &mut *csarg.win;
        let buf = wp.w_buffer;
        let line = csarg.line;
        let use_tabstop = cur_char == TAB && csarg.use_tabstop;
        let mut mb_added = 0;

        let has_lcs_eol = wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.eol != 0;

        let mut size: c_int;
        let mut is_doublewidth = false;
        if use_tabstop {
            size = tabstop_padding(vcol, (*buf).b_p_ts, &(*buf).b_p_vts_array);
        } else if crate::memory::cstr_is_empty(cur) {
            size = if has_lcs_eol { 1 } else { 0 };
        } else if cur_char < 0 {
            size = K_INVALID_BYTE_CELLS;
        } else {
            size = mb_width;
            is_doublewidth = size == 2 && cur_char >= 0x80;
        }

        if csarg.virt_row >= 0 && !csarg.skip_cur_text {
            let mut tab_size = size;
            let col = cur.offset_from(line) as c_int;
            loop {
                let mark = marktree_itr_current(&(*buf).b_marktree, &csarg.iter);
                if mark.pos.row != csarg.virt_row || mark.pos.col > col {
                    break;
                } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, &mut *wp) {
                    let decor = mt_decor(mark);
                    for vt in crate::decoration::virt_text_iter_mut(if decor.is_ext() {
                        decor.data.ext().vt
                    } else {
                        ptr::null_mut()
                    }) {
                        if (vt.flags & K_VTIS_LINES) == 0 && vt.pos == K_VPOS_INLINE {
                            if mt_right(mark) {
                                csarg.cur_text_width_right += vt.width;
                            } else {
                                csarg.cur_text_width_left += vt.width;
                            }
                            size += vt.width;
                            if use_tabstop {
                                size -= tab_size;
                                tab_size = tabstop_padding(
                                    vcol + size,
                                    (*buf).b_p_ts,
                                    &(*buf).b_p_vts_array,
                                );
                                size += tab_size;
                            }
                        }
                    }
                }
                marktree_itr_next_filter(
                    &(*wp).buf_mut().b_marktree,
                    &mut csarg.iter,
                    csarg.virt_row + 1,
                    0,
                    &INLINE_FILTER,
                );
            }
        }

        if is_doublewidth
            && wp.w_onebuf_opt.wo_wrap != 0
            && in_win_border(&mut *wp, vcol + size - 2)
        {
            size += 1;
            mb_added = 1;
        }

        let sbr = get_showbreak_value(&mut *wp);

        let mut head = mb_added;
        if size > 0
            && wp.w_onebuf_opt.wo_wrap != 0
            && (!crate::memory::cstr_is_empty(sbr) || wp.w_onebuf_opt.wo_bri != 0)
        {
            let mut col_off_prev = win_col_off(wp);
            let width2 = wp.w_view_width - col_off_prev + win_col_off2(&mut *wp);
            let mut wcol = vcol + col_off_prev;
            let max_head_vcol = csarg.max_head_vcol;
            let mut added = 0;

            let mut head_prev = 0;
            if wcol >= wp.w_view_width {
                wcol -= wp.w_view_width;
                col_off_prev = wp.w_view_width - width2;
                if wcol >= width2 && width2 > 0 {
                    wcol %= width2;
                }
                head_prev = csarg.indent_width;
                if head_prev == c_int::MIN {
                    head_prev = 0;
                    if !crate::memory::cstr_is_empty(sbr) {
                        head_prev += vim_strsize(sbr);
                    }
                    if wp.w_onebuf_opt.wo_bri != 0 {
                        head_prev += get_breakindent_win(&mut *wp, line);
                    }
                    csarg.indent_width = head_prev;
                }
                if wcol < head_prev {
                    head_prev -= wcol;
                    wcol += head_prev;
                    added += head_prev;
                    if max_head_vcol <= 0 || vcol < max_head_vcol {
                        head += head_prev;
                    }
                } else {
                    head_prev = 0;
                }
                wcol += col_off_prev;
            }

            if wcol + size > wp.w_view_width {
                let mut head_mid = csarg.indent_width;
                if head_mid == c_int::MIN {
                    head_mid = 0;
                    if !crate::memory::cstr_is_empty(sbr) {
                        head_mid += vim_strsize(sbr);
                    }
                    if wp.w_onebuf_opt.wo_bri != 0 {
                        head_mid += get_breakindent_win(&mut *wp, line);
                    }
                    csarg.indent_width = head_mid;
                }
                if head_mid > 0 {
                    let prev_rem = wp.w_view_width - wcol;
                    let mut width = width2 - head_mid;
                    if width <= 0 {
                        width = 1;
                    }
                    let cnt = (size - prev_rem + width - 1) / width;
                    added += cnt * head_mid;

                    if max_head_vcol == 0 || vcol + size + added < max_head_vcol {
                        head += cnt * head_mid;
                    } else if width2 > 0 && max_head_vcol > vcol + head_prev + prev_rem {
                        head += (max_head_vcol - (vcol + head_prev + prev_rem) + width2 - 1)
                            / width2
                            * head_mid;
                    } else if max_head_vcol < 0 {
                        let off = mb_added
                            + virt_text_cursor_off(&*csarg, crate::memory::cstr_is_empty(cur));
                        if off >= prev_rem {
                            if size > off {
                                head += (1 + (off - prev_rem) / width) * head_mid;
                            } else {
                                head += (off - prev_rem + width - 1) / width * head_mid;
                            }
                        }
                    }
                }
            }

            size += added;
        }

        let size_before_lbr = size;
        let mut need_lbr = false;
        if wp.w_onebuf_opt.wo_lbr != 0
            && wp.w_onebuf_opt.wo_wrap != 0
            && wp.w_view_width != 0
            && vim_isbreak(c_int::from(crate::memory::cstr_byte_at(cur, 0)))
            && !vim_isbreak(c_int::from(crate::memory::cstr_byte_at(cur, 1)))
        {
            let mut t = csarg.line;
            while vim_isbreak(c_int::from(crate::memory::cstr_first_byte(t))) {
                t = t.add(1);
            }
            need_lbr = cur >= t;
        }
        if need_lbr {
            let mut s = cur;
            let numberextra = win_col_off(wp);
            let col_adj = size - 1;
            let mut colmax: Colnr = wp.w_view_width - numberextra - col_adj;
            if vcol >= colmax {
                colmax += col_adj;
                let n = colmax + win_col_off2(&mut *wp);
                if n > 0 {
                    colmax += (((vcol + col_adj - colmax) / n) + 1) * n - col_adj;
                }
            }

            let mut virt_iter = MarkTreeIter::new();
            let mut has_virt = if csarg.virt_row < 0 { 0 } else { -1 };

            let mut vcol2 = vcol;
            loop {
                let ps = s;
                s = s.add(utfc_ptr2len(s) as usize);
                let c = c_int::from(crate::memory::cstr_first_byte(s));
                if !(c != 0
                    && (vim_isbreak(c)
                        || vcol2 == vcol
                        || !vim_isbreak(c_int::from(crate::memory::cstr_first_byte(ps)))))
                {
                    break;
                }

                let col = s.offset_from(line) as c_int;
                if has_virt < 0 {
                    has_virt = c_int::from(marktree_itr_get_filter(
                        &(*buf).b_marktree,
                        csarg.virt_row,
                        col,
                        csarg.virt_row + 1,
                        0,
                        &INLINE_FILTER,
                        &mut virt_iter,
                    ));
                }
                let virt_width = if has_virt != 0 {
                    inline_virt_text_width(&mut *wp, &mut virt_iter, csarg.virt_row, col)
                } else {
                    0
                };
                vcol2 += virt_width;
                vcol2 += win_chartabsize(&mut *wp, s, vcol2);
                if vcol2 >= colmax {
                    size = colmax - vcol + col_adj;
                    break;
                }
            }
        }

        let tail = size - size_before_lbr;
        CharSize {
            width: size,
            head,
            tail,
        }
    }
}

#[inline]
fn charsize_fast_impl(
    wp: &mut WinInfoInfo,
    _cur: *const c_char,
    use_tabstop: bool,
    vcol: Colnr,
    cur_char: i32,
    mb_width: c_int,
) -> CharSize {
    {
        if cur_char == TAB && use_tabstop {
            CharSize {
                width: tabstop_padding(vcol, wp.buf_mut().b_p_ts, &wp.buf_mut().b_p_vts_array),
                head: 0,
                tail: 0,
            }
        } else {
            let width = if cur_char < 0 {
                K_INVALID_BYTE_CELLS
            } else {
                mb_width
            };

            if width == 2
                && cur_char >= 0x80
                && wp.w_onebuf_opt.wo_wrap != 0
                && in_win_border(wp, vcol)
            {
                CharSize {
                    width: 3,
                    head: 1,
                    tail: 0,
                }
            } else {
                CharSize {
                    width,
                    head: 0,
                    tail: 0,
                }
            }
        }
    }
}

pub fn charsize_fast(
    csarg: &mut CharsizeArg,
    cur: impl crate::strings::IntoCCharPtr,
    vcol: Colnr,
    cur_char: i32,
    mb_width: c_int,
) -> CharSize {
    let cur = cur.into_c_char_ptr();
    {
        charsize_fast_impl(
            unsafe { &mut *csarg.win },
            cur,
            csarg.use_tabstop,
            vcol,
            cur_char,
            mb_width,
        )
    }
}

pub fn charsize_nowrap(
    buf: &mut FileBuf,
    _cur: *const c_char,
    use_tabstop: bool,
    vcol: Colnr,
    cur_char: i32,
    mb_width: c_int,
) -> c_int {
    if cur_char == TAB && use_tabstop {
        tabstop_padding(vcol, buf.b_p_ts, &buf.b_p_vts_array)
    } else if cur_char < 0 {
        K_INVALID_BYTE_CELLS
    } else {
        mb_width
    }
}

fn in_win_border(wp: &mut WinInfoInfo, vcol: Colnr) -> bool {
    unsafe {
        if wp.w_view_width == 0 {
            return false;
        }
        let width1 = wp.w_view_width - win_col_off(&raw mut *wp);
        if vcol < width1 - 1 {
            return false;
        }
        if vcol == width1 - 1 {
            return true;
        }
        let width2 = width1 + win_col_off2(wp);
        if width2 <= 0 {
            return false;
        }
        (vcol - width1) % width2 == width2 - 1
    }
}

pub fn linesize_regular(csarg: &mut CharsizeArg, mut vcol_arg: c_int, len: Colnr) -> c_int {
    unsafe {
        let line: *const c_char = csarg.line;
        let mut vcol = i64::from(vcol_arg);

        let mut ci = utf_ptr2_str_char_info(line);
        while (ci.ptr.offset_from(line) as Colnr) < len && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_cluster_info(ci);
            vcol += i64::from(
                charsize_regular(&mut *csarg, ci.ptr, vcol_arg, ci.chr.value, cli.cells).width,
            );
            ci = cli.next;
            if vcol > i64::from(MAXCOL) {
                vcol_arg = MAXCOL;
                break;
            } else {
                vcol_arg = vcol as c_int;
            }
        }

        if len == MAXCOL && csarg.virt_row >= 0 && crate::memory::cstr_is_empty(ci.ptr) {
            let head = charsize_regular(&mut *csarg, ci.ptr, vcol_arg, ci.chr.value, 0).head;
            vcol += i64::from(csarg.cur_text_width_left + csarg.cur_text_width_right + head);
            vcol_arg = if vcol > i64::from(MAXCOL) {
                MAXCOL
            } else {
                vcol as c_int
            };
        }

        vcol_arg
    }
}

pub fn linesize_fast(csarg: &CharsizeArg, mut vcol_arg: c_int, len: Colnr) -> c_int {
    unsafe {
        let wp = csarg.win;
        let use_tabstop = csarg.use_tabstop;
        let line: *const c_char = csarg.line;
        let mut vcol = i64::from(vcol_arg);

        let mut ci = utf_ptr2_str_char_info(line);
        while (ci.ptr.offset_from(line) as Colnr) < len && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_cluster_info(ci);
            vcol += i64::from(
                charsize_fast_impl(
                    &mut *wp,
                    ci.ptr,
                    use_tabstop,
                    vcol_arg,
                    ci.chr.value,
                    cli.cells,
                )
                .width,
            );
            ci = cli.next;
            if vcol > i64::from(MAXCOL) {
                vcol_arg = MAXCOL;
                break;
            } else {
                vcol_arg = vcol as c_int;
            }
        }

        vcol_arg
    }
}

fn virt_text_cursor_off(csarg: &CharsizeArg, on_nul: bool) -> c_int {
    {
        let mut off = 0;
        if !on_nul || (g_st().state_global & MODE_NORMAL) == 0 {
            off += csarg.cur_text_width_left;
        }
        if !on_nul && (g_st().state_global & MODE_NORMAL) != 0 {
            off += csarg.cur_text_width_right;
        }
        off
    }
}

pub fn getvcol(
    wp: &mut WinInfoInfo,
    pos: &mut CursorPos,
    mut start: Option<&mut Colnr>,
    mut cursor: Option<&mut Colnr>,
    end: Option<&mut Colnr>,
    flags: c_int,
) {
    unsafe {
        let line = ml_get_buf(wp.buf_mut(), pos.lnum);
        let end_col = pos.col;

        let mut csarg = CharsizeArg::new();
        let mut on_nul = false;
        let cstype = init_charsize_arg(&mut csarg, &mut *wp, pos.lnum, line);
        csarg.max_head_vcol = -1;

        let mut vcol: Colnr = 0;
        let mut char_size: CharSize;
        let mut ci = utf_ptr2_str_char_info(line);
        if cstype == K_CHARSIZE_FAST {
            let use_tabstop = csarg.use_tabstop;
            loop {
                if crate::memory::cstr_is_empty(ci.ptr) {
                    char_size = CharSize {
                        width: 1,
                        head: 0,
                        tail: 0,
                    };
                    break;
                }
                let cli = utf_cluster_info(ci);
                char_size = charsize_fast_impl(
                    &mut *wp,
                    ci.ptr,
                    use_tabstop,
                    vcol,
                    ci.chr.value,
                    cli.cells,
                );
                let next = cli.next;
                if (next.ptr.offset_from(line) as Colnr) > end_col {
                    break;
                }
                ci = next;
                vcol += char_size.width;
            }
        } else {
            loop {
                let cli = utf_cluster_info(ci);
                char_size = charsize_regular(&mut csarg, ci.ptr, vcol, ci.chr.value, cli.cells);
                if crate::memory::cstr_is_empty(ci.ptr) {
                    char_size.width = 1 + csarg.cur_text_width_left + csarg.cur_text_width_right;
                    on_nul = true;
                    break;
                }
                let next = cli.next;
                if (next.ptr.offset_from(line) as Colnr) > end_col {
                    break;
                }
                ci = next;
                vcol += char_size.width;
            }
        }

        if crate::memory::cstr_is_empty(ci.ptr)
            && end_col < MAXCOL
            && end_col > (ci.ptr.offset_from(line) as Colnr)
        {
            pos.col = ci.ptr.offset_from(line) as Colnr;
        }

        let incr = char_size.width;
        let head = char_size.head;
        let tail = char_size.tail;

        if let Some(ref mut p) = start {
            **p = vcol + head;
        }
        if let Some(end) = end {
            *end = vcol + incr
                - if (flags & GETVCOL_END_EXCL_LBR) != 0 {
                    tail
                } else {
                    0
                }
                - 1;
        }
        if cursor.is_some() {
            if ci.chr.value == TAB
                && (g_st().state_global & MODE_NORMAL) != 0
                && wp.w_onebuf_opt.wo_list == 0
                && !virtual_active(&mut *wp)
                && !(g_st().visual_global.active
                    && (crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
                        || ltoreq(*pos, g_st().visual_global.start)))
            {
                *cursor.as_deref_mut().unwrap() = vcol + incr - 1;
            } else {
                vcol += virt_text_cursor_off(&csarg, on_nul);
                *cursor.unwrap() = vcol + head;
            }
        }
    }
}

pub fn getvcol_nolist(posp: &mut CursorPos) -> Colnr {
    {
        let list_save = curwin_mut().w_onebuf_opt.wo_list;
        let mut vcol: Colnr = 0;

        curwin_mut().w_onebuf_opt.wo_list = 0;
        if posp.coladd != 0 {
            getvvcol(curwin_ptr(), posp, None, Some(&mut vcol), None, 0);
        } else {
            getvcol(curwin_mut(), posp, None, Some(&mut vcol), None, 0);
        }
        curwin_mut().w_onebuf_opt.wo_list = list_save;
        vcol
    }
}

pub fn getvvcol(
    wp: *mut WinInfoInfo,
    pos: &mut CursorPos,
    mut start: Option<&mut Colnr>,
    mut cursor: Option<&mut Colnr>,
    mut end: Option<&mut Colnr>,
    flags: c_int,
) {
    unsafe {
        let mut col: Colnr = 0;

        if virtual_active(&mut *wp) {
            getvcol(&mut *wp, pos, Some(&mut col), None, None, flags);
            let mut coladd = pos.coladd;
            let mut endadd: Colnr = 0;

            let ptr_val = ml_get_buf((*wp).buf_mut(), pos.lnum);
            if pos.col < ml_get_buf_len((*wp).buf_mut(), pos.lnum) {
                let c = utf_ptr2char(ptr_val.add(pos.col as usize));
                if c != TAB && vim_isprintc(c) {
                    endadd = ptr2cells(ptr_val.add(pos.col as usize)) - 1;
                    if coladd > endadd {
                        endadd = 0;
                    } else {
                        coladd = 0;
                    }
                }
            }
            col += coladd;

            if let Some(ref mut p) = start {
                **p = col;
            }
            if let Some(ref mut p) = cursor {
                **p = col;
            }
            if let Some(ref mut p) = end {
                **p = col + endadd;
            }
        } else {
            getvcol(
                &mut *wp,
                pos,
                (start.is_some()).then(|| &mut *start.unwrap()),
                (cursor.is_some()).then(|| &mut *cursor.unwrap()),
                (end.is_some()).then(|| &mut *end.unwrap()),
                flags,
            );
        }
    }
}

pub fn getvcols(
    wp: *mut WinInfoInfo,
    pos1: &mut CursorPos,
    pos2: &mut CursorPos,
    left: &mut Colnr,
    right: &mut Colnr,
    flags: c_int,
) {
    {
        let mut from1: Colnr = 0;
        let mut from2: Colnr = 0;
        let mut to1: Colnr = 0;
        let mut to2: Colnr = 0;

        if lt(*pos1, *pos2) {
            getvvcol(wp, pos1, Some(&mut from1), None, Some(&mut to1), flags);
            getvvcol(wp, pos2, Some(&mut from2), None, Some(&mut to2), flags);
        } else {
            getvvcol(wp, pos2, Some(&mut from1), None, Some(&mut to1), flags);
            getvvcol(wp, pos1, Some(&mut from2), None, Some(&mut to2), flags);
        }

        *left = from1.min(from2);

        if to2 > to1 {
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' && from2 > to1 {
                *right = from2 - 1;
            } else {
                *right = to2;
            }
        } else {
            *right = to1;
        }
    }
}

pub fn win_may_fill(wp: &mut WinInfoInfo) -> bool {
    {
        (wp.w_onebuf_opt.wo_diff != 0 && diffopt_filler())
            || wp.buf_mut().b_marktree.meta_root[K_MTMETA_LINES] != 0
    }
}

pub fn win_get_fill(wp: &mut WinInfoInfo, lnum: Linenr) -> c_int {
    decor_virt_lines(wp, lnum - 1, lnum, None, None, true) + diff_check_fill(wp, lnum)
}

pub fn plines_win(wp: *mut WinInfoInfo, lnum: Linenr, limit_winheight: bool) -> c_int {
    plines_win_nofill(wp, lnum, limit_winheight) + win_get_fill(unsafe { &mut *wp }, lnum)
}

pub fn plines_win_nofill(wp: *mut WinInfoInfo, lnum: Linenr, limit_winheight: bool) -> c_int {
    unsafe {
        if decor_conceal_line(wp, lnum - 1, false) {
            return 0;
        }
        if (*wp).w_onebuf_opt.wo_wrap == 0 || (*wp).w_view_width == 0 || line_folded(&mut *wp, lnum)
        {
            return 1;
        }
        let lines = plines_win_nofold(&mut *wp, lnum);
        if limit_winheight && lines > (*wp).w_view_height {
            (*wp).w_view_height
        } else {
            lines
        }
    }
}

pub fn plines_win_nofold(wp: &mut WinInfoInfo, lnum: Linenr) -> c_int {
    unsafe {
        let s = ml_get_buf(wp.buf_mut(), lnum);
        let mut csarg = CharsizeArg::new();
        let cstype = init_charsize_arg(&mut csarg, &mut *wp, lnum, s);
        if crate::memory::cstr_is_empty(s) && csarg.virt_row < 0 {
            return 1;
        }

        let mut col = if cstype == K_CHARSIZE_FAST {
            i64::from(linesize_fast(&csarg, 0, MAXCOL))
        } else {
            i64::from(linesize_regular(&mut csarg, 0, MAXCOL))
        };

        if wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.eol != 0 {
            col += 1;
        }

        let mut width = wp.w_view_width - win_col_off(wp);
        if width <= 0 {
            return 32000;
        }
        if col <= i64::from(width) {
            return 1;
        }
        col -= i64::from(width);
        width += win_col_off2(&mut *wp);
        let lines = (col + (i64::from(width) - 1)) / i64::from(width) + 1;
        if lines > 0 && lines <= i64::from(c_int::MAX) {
            lines as c_int
        } else {
            c_int::MAX
        }
    }
}

pub fn plines_win_col(wp: &mut WinInfoInfo, lnum: Linenr, column: c_long) -> c_int {
    unsafe {
        let mut lines = win_get_fill(wp, lnum);
        if wp.w_onebuf_opt.wo_wrap == 0 || wp.w_view_width == 0 {
            return lines + 1;
        }

        let line = ml_get_buf(wp.buf_mut(), lnum);
        let mut csarg = CharsizeArg::new();
        let cstype = init_charsize_arg(&mut csarg, &mut *wp, lnum, line);

        let mut vcol: Colnr = 0;
        let mut ci = utf_ptr2_str_char_info(line);
        let limit_ptr = (line as *const c_char).add(column as usize);
        if cstype == K_CHARSIZE_FAST {
            let use_tabstop = csarg.use_tabstop;
            while !crate::memory::cstr_is_empty(ci.ptr) && ci.ptr < limit_ptr {
                let cli = utf_cluster_info(ci);
                vcol += charsize_fast_impl(
                    &mut *wp,
                    ci.ptr,
                    use_tabstop,
                    vcol,
                    ci.chr.value,
                    cli.cells,
                )
                .width;
                ci = cli.next;
            }
        } else {
            while !crate::memory::cstr_is_empty(ci.ptr) && ci.ptr < limit_ptr {
                let cli = utf_cluster_info(ci);
                vcol += charsize_regular(&mut csarg, ci.ptr, vcol, ci.chr.value, cli.cells).width;
                ci = cli.next;
            }
        }

        let mut col = vcol;
        if ci.chr.value == TAB && (g_st().state_global & MODE_NORMAL) != 0 && csarg.use_tabstop {
            col += win_charsize(cstype, col, ci.ptr, ci.chr.value, &mut csarg, 1).width - 1;
        }

        let width = wp.w_view_width - win_col_off(wp);
        if width <= 0 {
            return 9999;
        }

        lines += 1;
        if col > width {
            lines += (col - width) / (width + win_col_off2(&mut *wp)) + 1;
        }
        lines
    }
}

pub fn plines_win_full(
    wp: *mut WinInfoInfo,
    mut lnum: Linenr,
    nextp: Option<&mut Linenr>,
    mut foldedp: Option<&mut bool>,
    cache: bool,
    limit_winheight: bool,
) -> c_int {
    unsafe {
        let folded = has_folding_win(
            &mut *wp,
            lnum,
            Some(&mut lnum),
            nextp.map_or(std::ptr::null_mut(), |p| &raw mut *p),
            cache,
            None,
        );
        if let Some(ref mut p) = foldedp {
            **p = folded;
        }

        let filler_lines = if lnum == (*wp).w_topline {
            (*wp).w_topfill
        } else {
            win_get_fill(&mut *wp, lnum)
        };

        if decor_conceal_line(wp, lnum - 1, false) {
            return filler_lines;
        }

        (if folded {
            1
        } else {
            plines_win_nofill(wp, lnum, limit_winheight)
        }) + filler_lines
    }
}

pub fn plines_m_win(wp: &mut WinInfoInfo, mut first: Linenr, last: Linenr, max: c_int) -> c_int {
    {
        let mut count = 0;
        while first <= last && count < max {
            let mut next = first;
            count += plines_win_full(&raw mut *wp, first, Some(&mut next), None, false, false);
            first = next + 1;
        }
        if first == wp.buf_mut().b_ml.ml_line_count + 1 {
            count += win_get_fill(wp, first);
        }
        max.min(count)
    }
}

pub fn plines_m_win_fill(wp: &mut WinInfoInfo, first: Linenr, last: Linenr) -> c_int {
    {
        let mut count = last - first + 1 + decor_virt_lines(wp, first - 1, last, None, None, false);

        if diffopt_filler() {
            for lnum in first..=last {
                let n = diff_check_fill(wp, lnum);
                count += n.max(0);
            }
        }

        count.max(0)
    }
}

pub fn win_text_height(
    wp: &mut WinInfoInfo,
    start_lnum: Linenr,
    start_vcol: i64,
    end_lnum: &mut Linenr,
    end_vcol: &mut i64,
    fill: Option<&mut i64>,
    max: i64,
) -> i64 {
    unsafe {
        let mut width1 = wp.w_view_width - win_col_off(&raw mut *wp);
        let mut width2 = width1 + win_col_off2(wp);
        width1 = width1.max(0);
        width2 = width2.max(0);
        let mut height_sum_fill: i64 = 0;
        let mut height_cur_nofill: i64 = 0;
        let mut height_sum_nofill: i64 = 0;
        let mut lnum = start_lnum;
        let mut cur_lnum = lnum;
        let mut cur_folded = false;

        if start_vcol >= 0 {
            let mut lnum_next = lnum;
            cur_folded = has_folding(&raw mut *wp, lnum, Some(&mut lnum), &mut lnum_next);
            height_cur_nofill = i64::from(plines_win_nofill(&raw mut *wp, lnum, false));
            height_sum_nofill += height_cur_nofill;
            let row_off = if start_vcol < i64::from(width1) || width2 <= 0 {
                0
            } else {
                1 + (start_vcol - i64::from(width1)) / i64::from(width2)
            };
            height_sum_nofill -= row_off.min(height_cur_nofill);
            lnum = lnum_next + 1;
        }

        while lnum <= *end_lnum && height_sum_nofill + height_sum_fill < max {
            let mut lnum_next = lnum;
            cur_folded = has_folding(&raw mut *wp, lnum, Some(&mut lnum), &mut lnum_next);
            height_sum_fill += i64::from(win_get_fill(wp, lnum));
            height_cur_nofill = i64::from(plines_win_nofill(&raw mut *wp, lnum, false));
            height_sum_nofill += height_cur_nofill;
            cur_lnum = lnum;
            lnum = lnum_next + 1;
        }

        let mut vcol_end = *end_vcol;
        let use_vcol = vcol_end >= 0 && lnum > *end_lnum;
        if use_vcol {
            height_sum_nofill -= height_cur_nofill;
            let row_off = if vcol_end == 0 {
                0
            } else if vcol_end <= i64::from(width1) || width2 <= 0 {
                1
            } else {
                1 + (vcol_end - i64::from(width1) + i64::from(width2) - 1) / i64::from(width2)
            };
            height_sum_nofill += row_off.min(height_cur_nofill);
        }

        if cur_folded {
            vcol_end = 0;
        } else {
            let linesize = i64::from(linetabsize_eol(&mut *wp, cur_lnum));
            vcol_end = (if use_vcol { vcol_end } else { i64::MAX }).min(linesize);
        }

        let overflow = height_sum_nofill + height_sum_fill - max;
        if overflow > 0 && width2 > 0 && vcol_end > i64::from(width2) {
            vcol_end -= (vcol_end - i64::from(width1)) % i64::from(width2)
                + (overflow - 1) * i64::from(width2);
        }

        *end_lnum = cur_lnum;
        *end_vcol = vcol_end;
        if let Some(fill) = fill {
            *fill = height_sum_fill;
        }
        height_sum_fill + height_sum_nofill
    }
}

pub fn win_max_displaywidth(
    wp: &mut WinInfoInfo,
    first: Linenr,
    last: Linenr,
    max: c_int,
) -> c_int {
    {
        let mut width = 0;
        let mut lnum = first;
        while lnum <= last && width < max {
            width = width.max(linetabsize(wp, lnum));
            lnum += 1;
        }
        max.min(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_comparisons_respect_lnum_col_coladd() {
        let p1 = CursorPos {
            lnum: 1,
            col: 2,
            coladd: 0,
        };
        let p2 = CursorPos {
            lnum: 1,
            col: 2,
            coladd: 1,
        };
        let p3 = CursorPos {
            lnum: 2,
            col: 0,
            coladd: 0,
        };
        assert!(lt(p1, p2));
        assert!(lt(p2, p3));
        assert!(ltoreq(p1, p1));
        assert!(ltoreq(p1, p2));
        assert!(!lt(p3, p1));
    }
}
