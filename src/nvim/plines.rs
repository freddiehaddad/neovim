//! Vertical and horizontal text display size calculations (`src/nvim/plines.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::api::extmark::{kVPosInline, kVTIsLines, ns_in_win};
use crate::buffer::buf_T;
use crate::charset::{vim_isprintc, vim_strsize};
use crate::cursor::{colnr_T, linenr_T, lt, ltoreq, pos_T};
use crate::decoration::{DecorVirtText, decor_conceal_line, decor_virt_lines, kMTFilterSelect};
use crate::diff::{diff_check_fill, diffopt_filler};
use crate::fold::{hasFolding, hasFoldingWin, lineFolded};
use crate::globals::*;
use crate::indent::{get_breakindent_win, tabstop_padding};
use crate::marktree::{
    MarkTreeIter, kMTMetaCount, kMTMetaInline, kMTMetaLines, marktree_itr_current,
    marktree_itr_get_filter, marktree_itr_next_filter, mt_decor, mt_invalid, mt_right,
};
use crate::mbyte::{ptr2cells, utf_ClusterInfo, utf_ptr2StrCharInfo, utf_ptr2char, utfc_ptr2len};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::r#move::{win_col_off, win_col_off2};
use crate::option::get_showbreak_value;
use crate::option_vars::*;
use crate::state::{MODE_NORMAL, virtual_active};
use crate::window::win_T;
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

use crate::api::buffer::NUL;
use crate::drawscreen::GETVCOL_END_EXCL_LBR;
use crate::insert::kCharsizeFast;
use crate::regexp::MAXCOL;
pub type CSType = bool;

const TAB: c_int = b'\t' as c_int;
const kInvalidByteCells: c_int = 4;

const kCharsizeRegular: CSType = false;

static INLINE_FILTER: [u32; kMTMetaCount] = [kMTFilterSelect, 0, 0, 0, 0];

#[repr(C)]
pub struct CharsizeArg {
    pub win: *mut win_T,
    pub line: *mut c_char,
    pub use_tabstop: bool,
    pub indent_width: c_int,
    pub virt_row: c_int,
    pub skip_cur_text: bool,
    pub cur_text_width_left: c_int,
    pub cur_text_width_right: c_int,
    pub max_head_vcol: c_int,
    pub iter: [MarkTreeIter; 1],
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
}

#[inline]
pub unsafe fn vim_isbreak(c: c_int) -> bool {
    unsafe { breakat_flags[(c as u8) as usize] != 0 }
}

#[inline]
pub(crate) unsafe fn win_charsize(
    cstype: CSType,
    vcol: colnr_T,
    ptr_val: *const c_char,
    chr: i32,
    csarg: *mut CharsizeArg,
    mb_width: c_int,
) -> CharSize {
    unsafe {
        if cstype == kCharsizeFast {
            charsize_fast(csarg, ptr_val, vcol, chr, mb_width)
        } else {
            charsize_regular(csarg, ptr_val, vcol, chr, mb_width)
        }
    }
}

#[inline]
pub unsafe fn win_linetabsize(
    wp: *mut win_T,
    lnum: linenr_T,
    line: *mut c_char,
    len: colnr_T,
) -> c_int {
    unsafe {
        let mut csarg: CharsizeArg = std::mem::zeroed();
        let cstype = init_charsize_arg(&mut csarg, wp, lnum, line);
        if cstype == kCharsizeFast {
            linesize_fast(&csarg, 0, len)
        } else {
            linesize_regular(&mut csarg, 0, len)
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_chartabsize(wp: *mut win_T, p: *const c_char, col: colnr_T) -> c_int {
    unsafe {
        let buf = (*wp).w_buffer;
        if c_int::from(*p) == TAB
            && ((*wp).w_onebuf_opt.wo_list == 0 || (*wp).w_p_lcs_chars.tab1 != 0)
        {
            return tabstop_padding(col, (*buf).b_p_ts, (*buf).b_p_vts_array);
        }
        ptr2cells(p)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn linetabsize_col(startvcol: c_int, s: *mut c_char) -> c_int {
    unsafe {
        let mut csarg: CharsizeArg = std::mem::zeroed();
        let cstype = init_charsize_arg(&mut csarg, curwin, 0, s);
        if cstype == kCharsizeFast {
            linesize_fast(&csarg, startvcol, MAXCOL)
        } else {
            linesize_regular(&mut csarg, startvcol, MAXCOL)
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn linetabsize(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe { win_linetabsize(wp, lnum, ml_get_buf((*wp).w_buffer, lnum), MAXCOL) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn linetabsize_eol(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        linetabsize(wp, lnum)
            + if (*wp).w_onebuf_opt.wo_list != 0 && (*wp).w_p_lcs_chars.eol != 0 {
                1
            } else {
                0
            }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn init_charsize_arg(
    csarg: *mut CharsizeArg,
    wp: *mut win_T,
    lnum: linenr_T,
    line: *mut c_char,
) -> CSType {
    unsafe {
        (*csarg).win = wp;
        (*csarg).line = line;
        (*csarg).max_head_vcol = 0;
        (*csarg).cur_text_width_left = 0;
        (*csarg).cur_text_width_right = 0;
        (*csarg).virt_row = -1;
        (*csarg).skip_cur_text = false;
        (*csarg).indent_width = c_int::MIN;
        (*csarg).use_tabstop = (*wp).w_onebuf_opt.wo_list == 0 || (*wp).w_p_lcs_chars.tab1 != 0;

        if lnum > 0
            && marktree_itr_get_filter(
                (*(*wp).w_buffer).b_marktree.as_mut_ptr(),
                lnum - 1,
                0,
                lnum,
                0,
                INLINE_FILTER.as_ptr(),
                (*csarg).iter.as_mut_ptr(),
            )
        {
            (*csarg).virt_row = lnum - 1;
        }

        if (*csarg).virt_row >= 0
            || ((*wp).w_onebuf_opt.wo_wrap != 0
                && ((*wp).w_onebuf_opt.wo_lbr != 0
                    || (*wp).w_onebuf_opt.wo_bri != 0
                    || *get_showbreak_value(wp) != NUL))
        {
            kCharsizeRegular
        } else {
            kCharsizeFast
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn init_charsize_arg_skip_cur_text(
    csarg: *mut CharsizeArg,
    wp: *mut win_T,
    lnum: linenr_T,
    line: *mut c_char,
) -> CSType {
    unsafe {
        let mut cstype = init_charsize_arg(csarg, wp, 0, line);
        (*csarg).skip_cur_text = true;
        if lnum > 0 && (*(*wp).w_buffer).b_marktree[0].meta_root[kMTMetaInline] > 0 {
            (*csarg).virt_row = lnum - 1;
            cstype = kCharsizeRegular;
        }
        cstype
    }
}

unsafe fn inline_virt_text_width(
    wp: *mut win_T,
    iter: *mut MarkTreeIter,
    row: c_int,
    col: c_int,
) -> c_int {
    unsafe {
        let buf = (*wp).w_buffer;
        let mut width: c_int = 0;
        loop {
            let mark = marktree_itr_current(iter);
            if mark.pos.row != row || mark.pos.col > col {
                break;
            } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, wp) {
                let decor = mt_decor(mark);
                let mut vt = if decor.ext {
                    decor.data.ext.vt.cast::<DecorVirtText>()
                } else {
                    ptr::null_mut()
                };
                while !vt.is_null() {
                    if ((*vt).flags & kVTIsLines) == 0 && (*vt).pos == kVPosInline {
                        width += (*vt).width;
                    }
                    vt = (*vt).next;
                }
            }
            marktree_itr_next_filter(
                (*buf).b_marktree.as_mut_ptr(),
                iter,
                row + 1,
                0,
                INLINE_FILTER.as_ptr(),
            );
        }
        width
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn charsize_regular(
    csarg: *mut CharsizeArg,
    cur: *const c_char,
    vcol: colnr_T,
    cur_char: i32,
    mb_width: c_int,
) -> CharSize {
    unsafe {
        (*csarg).cur_text_width_left = 0;
        (*csarg).cur_text_width_right = 0;

        let wp = (*csarg).win;
        let buf = (*wp).w_buffer;
        let line = (*csarg).line;
        let use_tabstop = cur_char == TAB && (*csarg).use_tabstop;
        let mut mb_added = 0;

        let has_lcs_eol = (*wp).w_onebuf_opt.wo_list != 0 && (*wp).w_p_lcs_chars.eol != 0;

        let mut size: c_int;
        let mut is_doublewidth = false;
        if use_tabstop {
            size = tabstop_padding(vcol, (*buf).b_p_ts, (*buf).b_p_vts_array);
        } else if *cur == NUL {
            size = if has_lcs_eol { 1 } else { 0 };
        } else if cur_char < 0 {
            size = kInvalidByteCells;
        } else {
            size = mb_width;
            is_doublewidth = size == 2 && cur_char >= 0x80;
        }

        if (*csarg).virt_row >= 0 && !(*csarg).skip_cur_text {
            let mut tab_size = size;
            let col = cur.offset_from(line) as c_int;
            loop {
                let mark = marktree_itr_current((*csarg).iter.as_mut_ptr());
                if mark.pos.row != (*csarg).virt_row || mark.pos.col > col {
                    break;
                } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, wp) {
                    let decor = mt_decor(mark);
                    let mut vt = if decor.ext {
                        decor.data.ext.vt.cast::<DecorVirtText>()
                    } else {
                        ptr::null_mut()
                    };
                    while !vt.is_null() {
                        if ((*vt).flags & kVTIsLines) == 0 && (*vt).pos == kVPosInline {
                            if mt_right(mark) {
                                (*csarg).cur_text_width_right += (*vt).width;
                            } else {
                                (*csarg).cur_text_width_left += (*vt).width;
                            }
                            size += (*vt).width;
                            if use_tabstop {
                                size -= tab_size;
                                tab_size = tabstop_padding(
                                    vcol + size,
                                    (*buf).b_p_ts,
                                    (*buf).b_p_vts_array,
                                );
                                size += tab_size;
                            }
                        }
                        vt = (*vt).next;
                    }
                }
                marktree_itr_next_filter(
                    (*(*wp).w_buffer).b_marktree.as_mut_ptr(),
                    (*csarg).iter.as_mut_ptr(),
                    (*csarg).virt_row + 1,
                    0,
                    INLINE_FILTER.as_ptr(),
                );
            }
        }

        if is_doublewidth && (*wp).w_onebuf_opt.wo_wrap != 0 && in_win_border(wp, vcol + size - 2) {
            size += 1;
            mb_added = 1;
        }

        let sbr = get_showbreak_value(wp);

        let mut head = mb_added;
        if size > 0
            && (*wp).w_onebuf_opt.wo_wrap != 0
            && (*sbr != NUL || (*wp).w_onebuf_opt.wo_bri != 0)
        {
            let mut col_off_prev = win_col_off(wp);
            let width2 = (*wp).w_view_width - col_off_prev + win_col_off2(wp);
            let mut wcol = vcol + col_off_prev;
            let max_head_vcol = (*csarg).max_head_vcol;
            let mut added = 0;

            let mut head_prev = 0;
            if wcol >= (*wp).w_view_width {
                wcol -= (*wp).w_view_width;
                col_off_prev = (*wp).w_view_width - width2;
                if wcol >= width2 && width2 > 0 {
                    wcol %= width2;
                }
                head_prev = (*csarg).indent_width;
                if head_prev == c_int::MIN {
                    head_prev = 0;
                    if *sbr != NUL {
                        head_prev += vim_strsize(sbr);
                    }
                    if (*wp).w_onebuf_opt.wo_bri != 0 {
                        head_prev += get_breakindent_win(wp, line);
                    }
                    (*csarg).indent_width = head_prev;
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

            if wcol + size > (*wp).w_view_width {
                let mut head_mid = (*csarg).indent_width;
                if head_mid == c_int::MIN {
                    head_mid = 0;
                    if *sbr != NUL {
                        head_mid += vim_strsize(sbr);
                    }
                    if (*wp).w_onebuf_opt.wo_bri != 0 {
                        head_mid += get_breakindent_win(wp, line);
                    }
                    (*csarg).indent_width = head_mid;
                }
                if head_mid > 0 {
                    let prev_rem = (*wp).w_view_width - wcol;
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
                        let off = mb_added + virt_text_cursor_off(csarg, *cur == NUL);
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
        if (*wp).w_onebuf_opt.wo_lbr != 0
            && (*wp).w_onebuf_opt.wo_wrap != 0
            && (*wp).w_view_width != 0
            && vim_isbreak(c_int::from(*cur.add(0) as u8))
            && !vim_isbreak(c_int::from(*cur.add(1) as u8))
        {
            let mut t = (*csarg).line;
            while vim_isbreak(c_int::from(*t as u8)) {
                t = t.add(1);
            }
            need_lbr = cur >= t;
        }
        if need_lbr {
            let mut s = cur;
            let numberextra = win_col_off(wp);
            let col_adj = size - 1;
            let mut colmax: colnr_T = (*wp).w_view_width - numberextra - col_adj;
            if vcol >= colmax {
                colmax += col_adj;
                let n = colmax + win_col_off2(wp);
                if n > 0 {
                    colmax += (((vcol + col_adj - colmax) / n) + 1) * n - col_adj;
                }
            }

            let mut virt_iter: [MarkTreeIter; 1] = std::mem::zeroed();
            let mut has_virt = if (*csarg).virt_row < 0 { 0 } else { -1 };

            let mut vcol2 = vcol;
            loop {
                let ps = s;
                s = s.add(utfc_ptr2len(s) as usize);
                let c = c_int::from(*s as u8);
                if !(c != 0
                    && (vim_isbreak(c) || vcol2 == vcol || !vim_isbreak(c_int::from(*ps as u8))))
                {
                    break;
                }

                let col = s.offset_from(line) as c_int;
                if has_virt < 0 {
                    has_virt = c_int::from(marktree_itr_get_filter(
                        (*buf).b_marktree.as_mut_ptr(),
                        (*csarg).virt_row,
                        col,
                        (*csarg).virt_row + 1,
                        0,
                        INLINE_FILTER.as_ptr(),
                        virt_iter.as_mut_ptr(),
                    ));
                }
                let virt_width = if has_virt != 0 {
                    inline_virt_text_width(wp, virt_iter.as_mut_ptr(), (*csarg).virt_row, col)
                } else {
                    0
                };
                vcol2 += virt_width;
                vcol2 += win_chartabsize(wp, s, vcol2);
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
unsafe fn charsize_fast_impl(
    wp: *mut win_T,
    _cur: *const c_char,
    use_tabstop: bool,
    vcol: colnr_T,
    cur_char: i32,
    mb_width: c_int,
) -> CharSize {
    unsafe {
        if cur_char == TAB && use_tabstop {
            CharSize {
                width: tabstop_padding(
                    vcol,
                    (*(*wp).w_buffer).b_p_ts,
                    (*(*wp).w_buffer).b_p_vts_array,
                ),
                head: 0,
                tail: 0,
            }
        } else {
            let width = if cur_char < 0 {
                kInvalidByteCells
            } else {
                mb_width
            };

            if width == 2
                && cur_char >= 0x80
                && (*wp).w_onebuf_opt.wo_wrap != 0
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn charsize_fast(
    csarg: *mut CharsizeArg,
    cur: *const c_char,
    vcol: colnr_T,
    cur_char: i32,
    mb_width: c_int,
) -> CharSize {
    unsafe {
        charsize_fast_impl(
            (*csarg).win,
            cur,
            (*csarg).use_tabstop,
            vcol,
            cur_char,
            mb_width,
        )
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn charsize_nowrap(
    buf: *mut buf_T,
    _cur: *const c_char,
    use_tabstop: bool,
    vcol: colnr_T,
    cur_char: i32,
    mb_width: c_int,
) -> c_int {
    unsafe {
        if cur_char == TAB && use_tabstop {
            tabstop_padding(vcol, (*buf).b_p_ts, (*buf).b_p_vts_array)
        } else if cur_char < 0 {
            kInvalidByteCells
        } else {
            mb_width
        }
    }
}

unsafe fn in_win_border(wp: *mut win_T, vcol: colnr_T) -> bool {
    unsafe {
        if (*wp).w_view_width == 0 {
            return false;
        }
        let width1 = (*wp).w_view_width - win_col_off(wp);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn linesize_regular(
    csarg: *mut CharsizeArg,
    mut vcol_arg: c_int,
    len: colnr_T,
) -> c_int {
    unsafe {
        let line: *const c_char = (*csarg).line;
        let mut vcol = i64::from(vcol_arg);

        let mut ci = utf_ptr2StrCharInfo(line);
        while (ci.ptr.offset_from(line) as colnr_T) < len && *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            vcol +=
                i64::from(charsize_regular(csarg, ci.ptr, vcol_arg, ci.chr.value, cli.cells).width);
            ci = cli.next;
            if vcol > i64::from(MAXCOL) {
                vcol_arg = MAXCOL;
                break;
            } else {
                vcol_arg = vcol as c_int;
            }
        }

        if len == MAXCOL && (*csarg).virt_row >= 0 && *ci.ptr == NUL {
            let head = charsize_regular(csarg, ci.ptr, vcol_arg, ci.chr.value, 0).head;
            vcol += i64::from((*csarg).cur_text_width_left + (*csarg).cur_text_width_right + head);
            vcol_arg = if vcol > i64::from(MAXCOL) {
                MAXCOL
            } else {
                vcol as c_int
            };
        }

        vcol_arg
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn linesize_fast(
    csarg: *const CharsizeArg,
    mut vcol_arg: c_int,
    len: colnr_T,
) -> c_int {
    unsafe {
        let wp = (*csarg).win;
        let use_tabstop = (*csarg).use_tabstop;
        let line: *const c_char = (*csarg).line;
        let mut vcol = i64::from(vcol_arg);

        let mut ci = utf_ptr2StrCharInfo(line);
        while (ci.ptr.offset_from(line) as colnr_T) < len && *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            vcol += i64::from(
                charsize_fast_impl(wp, ci.ptr, use_tabstop, vcol_arg, ci.chr.value, cli.cells)
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

unsafe fn virt_text_cursor_off(csarg: *const CharsizeArg, on_nul: bool) -> c_int {
    unsafe {
        let mut off = 0;
        if !on_nul || (State & MODE_NORMAL) == 0 {
            off += (*csarg).cur_text_width_left;
        }
        if !on_nul && (State & MODE_NORMAL) != 0 {
            off += (*csarg).cur_text_width_right;
        }
        off
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getvcol(
    wp: *mut win_T,
    pos: *mut pos_T,
    start: *mut colnr_T,
    cursor: *mut colnr_T,
    end: *mut colnr_T,
    flags: c_int,
) {
    unsafe {
        let line = ml_get_buf((*wp).w_buffer, (*pos).lnum);
        let end_col = (*pos).col;

        let mut csarg: CharsizeArg = std::mem::zeroed();
        let mut on_nul = false;
        let cstype = init_charsize_arg(&mut csarg, wp, (*pos).lnum, line);
        csarg.max_head_vcol = -1;

        let mut vcol: colnr_T = 0;
        let mut char_size: CharSize;
        let mut ci = utf_ptr2StrCharInfo(line);
        if cstype == kCharsizeFast {
            let use_tabstop = csarg.use_tabstop;
            loop {
                if *ci.ptr == NUL {
                    char_size = CharSize {
                        width: 1,
                        head: 0,
                        tail: 0,
                    };
                    break;
                }
                let cli = utf_ClusterInfo(ci);
                char_size =
                    charsize_fast_impl(wp, ci.ptr, use_tabstop, vcol, ci.chr.value, cli.cells);
                let next = cli.next;
                if (next.ptr.offset_from(line) as colnr_T) > end_col {
                    break;
                }
                ci = next;
                vcol += char_size.width;
            }
        } else {
            loop {
                let cli = utf_ClusterInfo(ci);
                char_size = charsize_regular(&mut csarg, ci.ptr, vcol, ci.chr.value, cli.cells);
                if *ci.ptr == NUL {
                    char_size.width = 1 + csarg.cur_text_width_left + csarg.cur_text_width_right;
                    on_nul = true;
                    break;
                }
                let next = cli.next;
                if (next.ptr.offset_from(line) as colnr_T) > end_col {
                    break;
                }
                ci = next;
                vcol += char_size.width;
            }
        }

        if *ci.ptr == NUL && end_col < MAXCOL && end_col > (ci.ptr.offset_from(line) as colnr_T) {
            (*pos).col = ci.ptr.offset_from(line) as colnr_T;
        }

        let incr = char_size.width;
        let head = char_size.head;
        let tail = char_size.tail;

        if !start.is_null() {
            *start = vcol + head;
        }
        if !end.is_null() {
            *end = vcol + incr
                - if (flags & GETVCOL_END_EXCL_LBR) != 0 {
                    tail
                } else {
                    0
                }
                - 1;
        }
        if !cursor.is_null() {
            if ci.chr.value == TAB
                && (State & MODE_NORMAL) != 0
                && (*wp).w_onebuf_opt.wo_list == 0
                && !virtual_active(wp)
                && !(Visual.active && (*p_sel == b'e' as c_char || ltoreq(*pos, Visual.start)))
            {
                *cursor = vcol + incr - 1;
            } else {
                vcol += virt_text_cursor_off(&csarg, on_nul);
                *cursor = vcol + head;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getvcol_nolist(posp: *mut pos_T) -> colnr_T {
    unsafe {
        let list_save = (*curwin).w_onebuf_opt.wo_list;
        let mut vcol: colnr_T = 0;

        (*curwin).w_onebuf_opt.wo_list = 0;
        if (*posp).coladd != 0 {
            getvvcol(curwin, posp, ptr::null_mut(), &mut vcol, ptr::null_mut(), 0);
        } else {
            getvcol(curwin, posp, ptr::null_mut(), &mut vcol, ptr::null_mut(), 0);
        }
        (*curwin).w_onebuf_opt.wo_list = list_save;
        vcol
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getvvcol(
    wp: *mut win_T,
    pos: *mut pos_T,
    start: *mut colnr_T,
    cursor: *mut colnr_T,
    end: *mut colnr_T,
    flags: c_int,
) {
    unsafe {
        let mut col: colnr_T = 0;

        if virtual_active(wp) {
            getvcol(wp, pos, &mut col, ptr::null_mut(), ptr::null_mut(), flags);
            let mut coladd = (*pos).coladd;
            let mut endadd: colnr_T = 0;

            let ptr_val = ml_get_buf((*wp).w_buffer, (*pos).lnum);
            if (*pos).col < ml_get_buf_len((*wp).w_buffer, (*pos).lnum) {
                let c = utf_ptr2char(ptr_val.add((*pos).col as usize));
                if c != TAB && vim_isprintc(c) {
                    endadd = ptr2cells(ptr_val.add((*pos).col as usize)) - 1;
                    if coladd > endadd {
                        endadd = 0;
                    } else {
                        coladd = 0;
                    }
                }
            }
            col += coladd;

            if !start.is_null() {
                *start = col;
            }
            if !cursor.is_null() {
                *cursor = col;
            }
            if !end.is_null() {
                *end = col + endadd;
            }
        } else {
            getvcol(wp, pos, start, cursor, end, flags);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getvcols(
    wp: *mut win_T,
    pos1: *mut pos_T,
    pos2: *mut pos_T,
    left: *mut colnr_T,
    right: *mut colnr_T,
    flags: c_int,
) {
    unsafe {
        let mut from1: colnr_T = 0;
        let mut from2: colnr_T = 0;
        let mut to1: colnr_T = 0;
        let mut to2: colnr_T = 0;

        if lt(*pos1, *pos2) {
            getvvcol(wp, pos1, &mut from1, ptr::null_mut(), &mut to1, flags);
            getvvcol(wp, pos2, &mut from2, ptr::null_mut(), &mut to2, flags);
        } else {
            getvvcol(wp, pos2, &mut from1, ptr::null_mut(), &mut to1, flags);
            getvvcol(wp, pos1, &mut from2, ptr::null_mut(), &mut to2, flags);
        }

        *left = from1.min(from2);

        if to2 > to1 {
            if *p_sel == b'e' as c_char && from2 > to1 {
                *right = from2 - 1;
            } else {
                *right = to2;
            }
        } else {
            *right = to1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_may_fill(wp: *mut win_T) -> bool {
    unsafe {
        ((*wp).w_onebuf_opt.wo_diff != 0 && diffopt_filler())
            || (*(*wp).w_buffer).b_marktree[0].meta_root[kMTMetaLines] != 0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_get_fill(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        decor_virt_lines(wp, lnum - 1, lnum, ptr::null_mut(), ptr::null_mut(), true)
            + diff_check_fill(wp, lnum)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_win(
    wp: *mut win_T,
    lnum: linenr_T,
    limit_winheight: bool,
) -> c_int {
    unsafe { plines_win_nofill(wp, lnum, limit_winheight) + win_get_fill(wp, lnum) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_win_nofill(
    wp: *mut win_T,
    lnum: linenr_T,
    limit_winheight: bool,
) -> c_int {
    unsafe {
        if decor_conceal_line(wp, lnum - 1, false) {
            return 0;
        }
        if (*wp).w_onebuf_opt.wo_wrap == 0 || (*wp).w_view_width == 0 || lineFolded(wp, lnum) {
            return 1;
        }
        let lines = plines_win_nofold(wp, lnum);
        if limit_winheight && lines > (*wp).w_view_height {
            (*wp).w_view_height
        } else {
            lines
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_win_nofold(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        let s = ml_get_buf((*wp).w_buffer, lnum);
        let mut csarg: CharsizeArg = std::mem::zeroed();
        let cstype = init_charsize_arg(&mut csarg, wp, lnum, s);
        if *s == NUL && csarg.virt_row < 0 {
            return 1;
        }

        let mut col = if cstype == kCharsizeFast {
            i64::from(linesize_fast(&csarg, 0, MAXCOL))
        } else {
            i64::from(linesize_regular(&mut csarg, 0, MAXCOL))
        };

        if (*wp).w_onebuf_opt.wo_list != 0 && (*wp).w_p_lcs_chars.eol != 0 {
            col += 1;
        }

        let mut width = (*wp).w_view_width - win_col_off(wp);
        if width <= 0 {
            return 32000;
        }
        if col <= i64::from(width) {
            return 1;
        }
        col -= i64::from(width);
        width += win_col_off2(wp);
        let lines = (col + (i64::from(width) - 1)) / i64::from(width) + 1;
        if lines > 0 && lines <= i64::from(c_int::MAX) {
            lines as c_int
        } else {
            c_int::MAX
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_win_col(wp: *mut win_T, lnum: linenr_T, column: c_long) -> c_int {
    unsafe {
        let mut lines = win_get_fill(wp, lnum);
        if (*wp).w_onebuf_opt.wo_wrap == 0 || (*wp).w_view_width == 0 {
            return lines + 1;
        }

        let line = ml_get_buf((*wp).w_buffer, lnum);
        let mut csarg: CharsizeArg = std::mem::zeroed();
        let cstype = init_charsize_arg(&mut csarg, wp, lnum, line);

        let mut vcol: colnr_T = 0;
        let mut ci = utf_ptr2StrCharInfo(line);
        let limit_ptr = (line as *const c_char).add(column as usize);
        if cstype == kCharsizeFast {
            let use_tabstop = csarg.use_tabstop;
            while *ci.ptr != NUL && ci.ptr < limit_ptr {
                let cli = utf_ClusterInfo(ci);
                vcol += charsize_fast_impl(wp, ci.ptr, use_tabstop, vcol, ci.chr.value, cli.cells)
                    .width;
                ci = cli.next;
            }
        } else {
            while *ci.ptr != NUL && ci.ptr < limit_ptr {
                let cli = utf_ClusterInfo(ci);
                vcol += charsize_regular(&mut csarg, ci.ptr, vcol, ci.chr.value, cli.cells).width;
                ci = cli.next;
            }
        }

        let mut col = vcol;
        if ci.chr.value == TAB && (State & MODE_NORMAL) != 0 && csarg.use_tabstop {
            col += win_charsize(cstype, col, ci.ptr, ci.chr.value, &mut csarg, 1).width - 1;
        }

        let width = (*wp).w_view_width - win_col_off(wp);
        if width <= 0 {
            return 9999;
        }

        lines += 1;
        if col > width {
            lines += (col - width) / (width + win_col_off2(wp)) + 1;
        }
        lines
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_win_full(
    wp: *mut win_T,
    mut lnum: linenr_T,
    nextp: *mut linenr_T,
    foldedp: *mut bool,
    cache: bool,
    limit_winheight: bool,
) -> c_int {
    unsafe {
        let folded = hasFoldingWin(wp, lnum, &mut lnum, nextp, cache, ptr::null_mut());
        if !foldedp.is_null() {
            *foldedp = folded;
        }

        let filler_lines = if lnum == (*wp).w_topline {
            (*wp).w_topfill
        } else {
            win_get_fill(wp, lnum)
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_m_win(
    wp: *mut win_T,
    mut first: linenr_T,
    last: linenr_T,
    max: c_int,
) -> c_int {
    unsafe {
        let mut count = 0;
        while first <= last && count < max {
            let mut next = first;
            count += plines_win_full(wp, first, &mut next, ptr::null_mut(), false, false);
            first = next + 1;
        }
        if first == (*(*wp).w_buffer).b_ml.ml_line_count + 1 {
            count += win_get_fill(wp, first);
        }
        max.min(count)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn plines_m_win_fill(
    wp: *mut win_T,
    first: linenr_T,
    last: linenr_T,
) -> c_int {
    unsafe {
        let mut count = last - first
            + 1
            + decor_virt_lines(wp, first - 1, last, ptr::null_mut(), ptr::null_mut(), false);

        if diffopt_filler() {
            for lnum in first..=last {
                let n = diff_check_fill(wp, lnum);
                count += n.max(0);
            }
        }

        count.max(0)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_text_height(
    wp: *mut win_T,
    start_lnum: linenr_T,
    start_vcol: i64,
    end_lnum: *mut linenr_T,
    end_vcol: *mut i64,
    fill: *mut i64,
    max: i64,
) -> i64 {
    unsafe {
        let mut width1 = (*wp).w_view_width - win_col_off(wp);
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
            cur_folded = hasFolding(wp, lnum, &mut lnum, &mut lnum_next);
            height_cur_nofill = i64::from(plines_win_nofill(wp, lnum, false));
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
            cur_folded = hasFolding(wp, lnum, &mut lnum, &mut lnum_next);
            height_sum_fill += i64::from(win_get_fill(wp, lnum));
            height_cur_nofill = i64::from(plines_win_nofill(wp, lnum, false));
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
            let linesize = i64::from(linetabsize_eol(wp, cur_lnum));
            vcol_end = (if use_vcol { vcol_end } else { i64::MAX }).min(linesize);
        }

        let overflow = height_sum_nofill + height_sum_fill - max;
        if overflow > 0 && width2 > 0 && vcol_end > i64::from(width2) {
            vcol_end -= (vcol_end - i64::from(width1)) % i64::from(width2)
                + (overflow - 1) * i64::from(width2);
        }

        *end_lnum = cur_lnum;
        *end_vcol = vcol_end;
        if !fill.is_null() {
            *fill = height_sum_fill;
        }
        height_sum_fill + height_sum_nofill
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_max_displaywidth(
    wp: *mut win_T,
    first: linenr_T,
    last: linenr_T,
    max: c_int,
) -> c_int {
    unsafe {
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
        let p1 = pos_T {
            lnum: 1,
            col: 2,
            coladd: 0,
        };
        let p2 = pos_T {
            lnum: 1,
            col: 2,
            coladd: 1,
        };
        let p3 = pos_T {
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
