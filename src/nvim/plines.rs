//! Vertical and horizontal text display size calculations (`src/nvim/plines.c`).
#![forbid(unsafe_code)]

use crate::api::extmark::ns_in_win;
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::buffer::{FileBuf, buf_meta_total};
use crate::charset::{NUL, vim_isprintc, vim_strsize};
use crate::cursor::{Colnr, CursorPos, Linenr, lt, ltoreq};
use crate::decoration::{
    DecorState, K_MTFILTER_SELECT, K_VPOS_INLINE, K_VTIS_LINES, decor_conceal_char,
    decor_conceal_line, decor_redraw_col, decor_redraw_line, decor_redraw_reset,
    decor_row_has_conceal, decor_virt_lines,
};
use crate::decoration_provider::decor_providers_invoke_conceal;
use crate::diff::{diff_check_fill, diffopt_filler};
use crate::drawscreen::{GETVCOL_END_EXCL_LBR, conceal_cursor_reveals_line};
use crate::fold::{has_folding, has_folding_win, line_folded};
use crate::globals::*;
use crate::grid::{Schar, schar_cells, schar_get, schar_get_first_codepoint};
use crate::indent::{get_breakindent_win, tabstop_padding};
use crate::keycodes::TAB;
use crate::marktree::{
    K_MTMETA_CONCEAL, K_MTMETA_COUNT, K_MTMETA_INLINE, K_MTMETA_LINES, MarkTreeIter,
    marktree_itr_current, marktree_itr_get_filter, marktree_itr_next_filter, mt_decor, mt_invalid,
    mt_right,
};
pub use crate::mbyte::{CharInfo, ClusterInfo, StrCharInfo};
use crate::mbyte::{utf_cluster_info, utf_ptr2_str_char_info};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::r#move::{win_col_off, win_col_off2};
use crate::option::get_showbreak_value;
use crate::option_vars::*;
use crate::state::{MODE_NORMAL, virtual_active};
use crate::window::WinInfoInfo;

use crate::insert::K_CHARSIZE_FAST;
use crate::regexp::MAXCOL;
pub type CSType = bool;

pub(crate) const GETVCOL_DISP: i32 = 2;

const K_INVALID_BYTE_CELLS: i32 = 4;

const K_CHARSIZE_REGULAR: CSType = false;

static INLINE_FILTER: [u32; K_MTMETA_COUNT] = [K_MTFILTER_SELECT, 0, 0, 0, 0, 0];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConcealSpan {
    pub start_col: i32,
    pub end_col: i32,
    pub replacement: Schar,
}

pub type ConcealSpans = Vec<ConcealSpan>;

#[repr(C)]
pub struct CharsizeArg<'a> {
    pub win: crate::globals::WinPtr,
    pub line: &'a std::ffi::CStr,
    pub use_tabstop: bool,
    pub indent_width: i32,
    pub virt_row: i32,
    pub skip_cur_text: bool,
    pub cur_text_width_left: i32,
    pub cur_text_width_right: i32,
    pub max_head_vcol: i32,
    pub extconceal_off: i32,
    pub iter: MarkTreeIter,
    pub extconceal: Option<ConcealSpans>,
}

impl CharsizeArg<'_> {
    pub const fn new() -> Self {
        Self {
            win: crate::globals::WinPtr::NULL,
            line: c"",
            use_tabstop: false,
            indent_width: 0,
            virt_row: 0,
            skip_cur_text: false,
            cur_text_width_left: 0,
            cur_text_width_right: 0,
            max_head_vcol: 0,
            extconceal_off: 0,
            iter: MarkTreeIter::new(),
            extconceal: None,
        }
    }

    #[inline]
    pub fn win_mut(&self) -> &'static mut WinInfoInfo {
        self.win.borrow_mut()
    }

    #[inline]
    pub fn extconceal_opt(&self) -> Option<&ConcealSpans> {
        self.extconceal.as_ref()
    }
}

impl Default for CharsizeArg<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharSize {
    pub width: i32,
    pub body: i32,
    pub head: i32,
    pub tail: i32,
    pub linebreak: bool,
    pub hidden: i32,
}

#[inline]
pub fn vim_isbreak(c: i32) -> bool {
    opt_vars().breakat_flags[(c as u8) as usize].get()
}

#[inline]
pub(crate) fn win_charsize(
    cstype: CSType,
    vcol: Colnr,
    ptr_val: &[u8],
    chr: i32,
    csarg: &mut CharsizeArg<'_>,
    mb_width: i32,
) -> CharSize {
    if cstype == K_CHARSIZE_FAST {
        charsize_fast(csarg, vcol, chr, mb_width)
    } else {
        charsize_regular(&mut *csarg, ptr_val, vcol, chr, mb_width)
    }
}

#[inline]
pub fn win_linetabsize(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    line: &std::ffi::CStr,
    len: Colnr,
) -> i32 {
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
    p: impl crate::mbyte::AsMbyteSlice,
    col: Colnr,
) -> i32 {
    p.with_cluster_bytes(|p| {
        if p.first().copied() == Some(TAB as u8)
            && (!wp.w_onebuf_opt.wo_list.get() || wp.w_p_lcs_chars.borrow().tab1 != 0)
        {
            let buf = wp.buf_mut();
            return tabstop_padding(col, buf.b_p_ts.get(), &buf.b_p_vts_array.borrow());
        }
        crate::mbyte::ptr2cells(p)
    })
}

pub fn linetabsize_col(startvcol: i32, s: &std::ffi::CStr) -> i32 {
    let mut csarg = CharsizeArg::new();
    let cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, s);
    if cstype == K_CHARSIZE_FAST {
        linesize_fast(&csarg, startvcol, MAXCOL)
    } else {
        linesize_regular(&mut csarg, startvcol, MAXCOL)
    }
}

pub fn linetabsize(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    win_linetabsize(wp, lnum, ml_get_buf(wp.buf_mut(), lnum), MAXCOL)
}

pub fn linetabsize_eol(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    linetabsize(wp, lnum)
        + if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol != 0 {
            1
        } else {
            0
        }
}

/// Whether extmark-conceal may hide cells on line `lnum` of `wp` (see has_extconceal()), and
/// 'concealcursor' does not reveal the line. Cheap: does not walk the marktree.
pub fn line_has_extconceal(wp: &mut WinInfoInfo, lnum: Linenr) -> bool {
    lnum > 0 && has_extconceal(wp) && !conceal_cursor_reveals_line(wp, lnum)
}

/// Whether extmark-conceal (see CharsizeArg.extconceal_off), including marks an `_on_conceal`
/// provider adds on demand, may change the layout of `wp`.
pub fn has_extconceal(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_wrap.get()
        && wp.w_onebuf_opt.wo_cole.get() > 0
        && (buf_meta_total(wp.buf_mut(), K_MTMETA_CONCEAL) > 0
            || wp.buf_mut().b_conceal_providers.get() > 0)
}

pub fn init_charsize_arg<'a>(
    csarg: &mut CharsizeArg<'a>,
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    line: &'a std::ffi::CStr,
) -> CSType {
    csarg.win = crate::globals::WinPtr::from(&mut *wp);
    csarg.line = line;
    csarg.max_head_vcol = 0;
    csarg.extconceal_off = 0;
    csarg.cur_text_width_left = 0;
    csarg.cur_text_width_right = 0;
    csarg.virt_row = -1;
    csarg.skip_cur_text = false;
    csarg.indent_width = i32::MIN;
    csarg.use_tabstop = !wp.w_onebuf_opt.wo_list.get() || wp.w_p_lcs_chars.borrow().tab1 != 0;
    csarg.extconceal = None;

    if lnum > 0
        && marktree_itr_get_filter(
            &wp.buf_mut().b_marktree.borrow(),
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
        || (wp.w_onebuf_opt.wo_wrap.get()
            && (wp.w_onebuf_opt.wo_lbr.get()
                || wp.w_onebuf_opt.wo_bri.get()
                || !get_showbreak_value(wp).is_empty()))
    {
        K_CHARSIZE_REGULAR
    } else {
        K_CHARSIZE_FAST
    }
}

/// Like init_charsize_arg(), but extmark-conceal hides cells (see CharsizeArg.extconceal_off).
/// Collects the conceal of line `lnum` into `spans`, after running its
/// `_on_conceal` provider, which may free the line buffer: read the line from `csarg.line`.
pub fn init_charsize_arg_extconceal(
    csarg: &mut CharsizeArg<'_>,
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    spans: &mut ConcealSpans,
) -> CSType {
    spans.clear();
    if line_has_extconceal(wp, lnum) {
        decor_providers_invoke_conceal(wp, lnum - 1, false);
        let len = ml_get_buf_len(wp.buf_mut(), lnum);
        conceal_spans_collect(wp, lnum - 1, len, spans);
    }
    let line = ml_get_buf(wp.buf_mut(), lnum);
    let cstype = init_charsize_arg(csarg, wp, lnum, line);
    if spans.is_empty() {
        return cstype;
    }
    csarg.extconceal = Some(spans.clone());
    K_CHARSIZE_REGULAR
}

pub fn init_charsize_arg_skip_cur_text<'a>(
    csarg: &mut CharsizeArg<'a>,
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    line: &'a std::ffi::CStr,
) -> CSType {
    let mut cstype = init_charsize_arg(csarg, wp, 0, line);
    csarg.skip_cur_text = true;
    if lnum > 0 && wp.buf_mut().b_marktree.borrow().meta_root[K_MTMETA_INLINE] > 0 {
        csarg.virt_row = lnum - 1;
        cstype = K_CHARSIZE_REGULAR;
    }
    cstype
}

fn inline_virt_text_width(
    wp: &mut WinInfoInfo,
    iter: &mut MarkTreeIter,
    row: i32,
    col: i32,
) -> i32 {
    let mut width: i32 = 0;
    loop {
        let mark = marktree_itr_current(&wp.buf_mut().b_marktree.borrow(), iter);
        if mark.pos.row != row || mark.pos.col > col {
            break;
        } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, &mut *wp) {
            let decor = mt_decor(mark);
            crate::decoration::for_each_virt_text(
                if decor.is_ext() {
                    decor.data.ext().vt
                } else {
                    crate::decoration::DecorVirtId::NULL
                },
                |_, vt| {
                    if (vt.flags & K_VTIS_LINES) == 0 && vt.pos == K_VPOS_INLINE {
                        width += vt.width;
                    }
                },
            );
        }
        marktree_itr_next_filter(
            &wp.buf_mut().b_marktree.borrow(),
            iter,
            row + 1,
            0,
            &INLINE_FILTER,
        );
    }
    width
}

/// Collects the extmark-conceal of row `row` (`len` bytes) into `spans`, without invoking
/// `_on_conceal`.
pub fn conceal_spans_collect(wp: &mut WinInfoInfo, row: i32, len: i32, spans: &mut ConcealSpans) {
    if !decor_row_has_conceal(wp, row) {
        return;
    }
    let mut state = DecorState::new();
    state.conceal_only = true;
    decor_redraw_reset(wp, &mut state);
    decor_redraw_line(wp, row, &mut state);
    let mut col = 0;
    while col <= len {
        decor_redraw_col(wp, col, -1, false, &mut state, len);
        if state.conceal != 0 {
            if state.conceal == 1
                && let Some(last) = spans.last_mut()
                && last.end_col == col
            {
                last.end_col = state.col_last + 1;
            } else {
                spans.push(ConcealSpan {
                    start_col: col,
                    end_col: state.col_last + 1,
                    replacement: decor_conceal_char(wp, &state),
                });
            }
        }
        col = state.col_last + 1;
    }
}

fn conceal_span_at(spans: &ConcealSpans, col: i32) -> Option<&ConcealSpan> {
    let mut lo = 0;
    let mut hi = spans.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let span = &spans[mid];
        if col < span.start_col {
            hi = mid;
        } else if col >= span.end_col {
            lo = mid + 1;
        } else {
            return Some(span);
        }
    }
    None
}

fn conceal_span_char(span: &ConcealSpan, col: i32) -> Schar {
    if span.start_col == col {
        span.replacement
    } else {
        0
    }
}

fn conceal_span_hidden(span: Option<&ConcealSpan>, col: i32, width: i32) -> i32 {
    let Some(span) = span else {
        return 0;
    };
    let replacement = conceal_span_char(span, col);
    if replacement == 0 {
        return width;
    }
    width - schar_cells(replacement)
}

pub fn conceal_disp_byte(csarg: &CharsizeArg<'_>, col: i32) -> i32 {
    let span = csarg
        .extconceal_opt()
        .and_then(|spans| conceal_span_at(spans, col));
    let Some(span) = span else {
        return i32::from(
            csarg
                .line
                .to_bytes()
                .get(col as usize)
                .copied()
                .unwrap_or(0),
        );
    };
    let replacement = conceal_span_char(span, col);
    if replacement == 0 {
        return i32::from(NUL);
    }
    let mut buf = [0u8; MAX_SCHAR_SIZE];
    schar_get(&mut buf, replacement);
    i32::from(buf[0])
}

fn conceal_next_byte(csarg: &CharsizeArg<'_>, col: i32) -> i32 {
    let line_bytes = csarg.line.to_bytes();
    let mut idx = (col.max(0) as usize).min(line_bytes.len());
    while idx < line_bytes.len() {
        let c = conceal_disp_byte(csarg, idx as i32);
        if c != i32::from(NUL) {
            return c;
        }
        idx += crate::mbyte::utfc_slice2len(&line_bytes[idx..]).max(1);
    }
    i32::from(NUL)
}

pub fn charsize_regular(
    csarg: &mut CharsizeArg<'_>,
    cur: &[u8],
    vcol: Colnr,
    cur_char: i32,
    mb_width: i32,
) -> CharSize {
    let line_bytes = csarg.line.to_bytes();
    let cur_bytes = crate::memory::bytes_until_nul(cur);
    let cur_off = if cur_bytes.is_empty() {
        line_bytes.len()
    } else {
        (cur_bytes.as_ptr() as usize)
            .saturating_sub(line_bytes.as_ptr() as usize)
            .min(line_bytes.len())
    };
    {
        csarg.cur_text_width_left = 0;
        csarg.cur_text_width_right = 0;

        let wp = csarg.win_mut();
        let buf = wp.buf_mut();
        let use_tabstop = cur_char == TAB && csarg.use_tabstop;
        let mut mb_added = 0;

        let disp_vcol = vcol - csarg.extconceal_off;

        let has_lcs_eol = wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol != 0;

        let mut size: i32;
        let mut is_doublewidth = false;
        if use_tabstop {
            size = tabstop_padding(vcol, buf.b_p_ts.get(), &buf.b_p_vts_array.borrow());
        } else if cur_bytes.is_empty() {
            size = if has_lcs_eol { 1 } else { 0 };
        } else if cur_char < 0 {
            size = K_INVALID_BYTE_CELLS;
        } else {
            size = mb_width;
            is_doublewidth = size == 2 && cur_char >= 0x80;
        }

        if csarg.virt_row >= 0 && !csarg.skip_cur_text {
            let mut tab_size = size;
            let col = cur_off as i32;
            loop {
                let mark = marktree_itr_current(&buf.b_marktree.borrow(), &csarg.iter);
                if mark.pos.row != csarg.virt_row || mark.pos.col > col {
                    break;
                } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, &mut *wp) {
                    let decor = mt_decor(mark);
                    crate::decoration::for_each_virt_text(
                        if decor.is_ext() {
                            decor.data.ext().vt
                        } else {
                            crate::decoration::DecorVirtId::NULL
                        },
                        |_, vt| {
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
                                        buf.b_p_ts.get(),
                                        &buf.b_p_vts_array.borrow(),
                                    );
                                    size += tab_size;
                                }
                            }
                        },
                    );
                }
                marktree_itr_next_filter(
                    &(*wp).buf_mut().b_marktree.borrow(),
                    &mut csarg.iter,
                    csarg.virt_row + 1,
                    0,
                    &INLINE_FILTER,
                );
            }
        }

        let body = size;
        let mut hidden = 0;
        let mut lbr_char = i32::from(cur_bytes.first().copied().unwrap_or(0));
        if let Some(spans) = csarg.extconceal_opt() {
            let col = cur_off as i32;
            let own = body - csarg.cur_text_width_left - csarg.cur_text_width_right;
            let span = conceal_span_at(spans, col);
            if let Some(span_ref) = span {
                hidden = conceal_span_hidden(Some(span_ref), col, own);
                let replacement = conceal_span_char(span_ref, col);
                is_doublewidth = replacement != 0 && schar_cells(replacement) == 2;
                lbr_char = if replacement == 0 {
                    i32::from(NUL)
                } else {
                    schar_get_first_codepoint(replacement)
                };
            } else if use_tabstop {
                hidden = (-csarg.extconceal_off).min(own - 1);
            }
        }
        size -= hidden;

        if is_doublewidth
            && wp.w_onebuf_opt.wo_wrap.get()
            && in_win_border(&mut *wp, disp_vcol + size - 2)
        {
            size += 1;
            mb_added = 1;
        }

        let sbr = get_showbreak_value(&*wp);

        let mut head = mb_added;
        if size > 0
            && wp.w_onebuf_opt.wo_wrap.get()
            && (!sbr.to_bytes().is_empty() || wp.w_onebuf_opt.wo_bri.get())
        {
            let mut col_off_prev = win_col_off(wp);
            let width2 = wp.w_view_width.get() - col_off_prev + win_col_off2(&mut *wp);
            let mut wcol = disp_vcol + col_off_prev;
            let max_head_vcol = csarg.max_head_vcol;
            let mut added = 0;

            let mut head_prev = 0;
            if wcol >= wp.w_view_width.get() {
                wcol -= wp.w_view_width.get();
                col_off_prev = wp.w_view_width.get() - width2;
                if wcol >= width2 && width2 > 0 {
                    wcol %= width2;
                }
                head_prev = csarg.indent_width;
                if head_prev == i32::MIN {
                    head_prev = 0;
                    if !sbr.to_bytes().is_empty() {
                        head_prev += vim_strsize(&sbr);
                    }
                    if wp.w_onebuf_opt.wo_bri.get() {
                        head_prev += get_breakindent_win(&mut *wp, line_bytes);
                    }
                    csarg.indent_width = head_prev;
                }
                if wcol < head_prev {
                    head_prev -= wcol;
                    wcol += head_prev;
                    added += head_prev;
                    if max_head_vcol <= 0 || disp_vcol < max_head_vcol {
                        head += head_prev;
                    }
                } else {
                    head_prev = 0;
                }
                wcol += col_off_prev;
            }

            if wcol + size > wp.w_view_width.get() {
                let mut head_mid = csarg.indent_width;
                if head_mid == i32::MIN {
                    head_mid = 0;
                    if !sbr.to_bytes().is_empty() {
                        head_mid += vim_strsize(&sbr);
                    }
                    if wp.w_onebuf_opt.wo_bri.get() {
                        head_mid += get_breakindent_win(&mut *wp, line_bytes);
                    }
                    csarg.indent_width = head_mid;
                }
                if head_mid > 0 {
                    let prev_rem = wp.w_view_width.get() - wcol;
                    let mut width = width2 - head_mid;
                    if width <= 0 {
                        width = 1;
                    }
                    let cnt = (size - prev_rem + width - 1) / width;
                    added += cnt * head_mid;

                    if max_head_vcol == 0 || disp_vcol + size + added < max_head_vcol {
                        head += cnt * head_mid;
                    } else if width2 > 0 && max_head_vcol > disp_vcol + head_prev + prev_rem {
                        head += (max_head_vcol - (disp_vcol + head_prev + prev_rem) + width2 - 1)
                            / width2
                            * head_mid;
                    } else if max_head_vcol < 0 {
                        let off = mb_added + virt_text_cursor_off(&*csarg, cur_bytes.is_empty());
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
        if wp.w_onebuf_opt.wo_lbr.get()
            && wp.w_onebuf_opt.wo_wrap.get()
            && wp.w_view_width.get() != 0
            && lbr_char > 0
            && lbr_char < 256
            && vim_isbreak(lbr_char)
        {
            let next_col = cur_off as i32 + crate::mbyte::utfc_slice2len(cur_bytes) as i32;
            let next_c = conceal_next_byte(csarg, next_col);
            if !vim_isbreak(i32::from(next_c as u8)) {
                let lead_breaks = line_bytes
                    .iter()
                    .take_while(|&&b| vim_isbreak(i32::from(b)))
                    .count();
                need_lbr = cur_off >= lead_breaks;
            }
        }
        if need_lbr {
            let mut s_off = cur_off;
            let numberextra = win_col_off(wp);
            let col_adj = size - 1;
            let mut colmax: Colnr = wp.w_view_width.get() - numberextra - col_adj;
            if disp_vcol >= colmax {
                colmax += col_adj;
                let n = colmax + win_col_off2(&mut *wp);
                if n > 0 {
                    colmax += (((disp_vcol + col_adj - colmax) / n) + 1) * n - col_adj;
                }
            }

            let mut virt_iter = MarkTreeIter::new();
            let mut has_virt = if csarg.virt_row < 0 { 0 } else { -1 };
            let mut lbr_off = csarg.extconceal_off + hidden;
            let mut use_raw_width = lbr_off != 0;
            let mut raw_vcol2 = vcol + body;
            let mut vcol2 = disp_vcol;
            let mut prev_break = true;
            loop {
                s_off += crate::mbyte::utfc_slice2len(&line_bytes[s_off..]).max(1);
                if s_off >= line_bytes.len() {
                    break;
                }
                let mut c = i32::from(line_bytes[s_off]);
                if c == 0 {
                    break;
                }

                let col = s_off as i32;
                if has_virt < 0 {
                    has_virt = i32::from(marktree_itr_get_filter(
                        &buf.b_marktree.borrow(),
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

                let s_bytes = &line_bytes[s_off..];
                let s_first = i32::from(s_bytes[0]);
                let raw_w = if csarg.extconceal.is_none() {
                    0
                } else if s_first == TAB && csarg.use_tabstop {
                    tabstop_padding(
                        raw_vcol2 + virt_width,
                        buf.b_p_ts.get(),
                        &buf.b_p_vts_array.borrow(),
                    )
                } else {
                    win_chartabsize(&mut *wp, s_bytes, raw_vcol2)
                };
                let mut w = if use_raw_width {
                    raw_w
                } else {
                    win_chartabsize(&mut *wp, s_bytes, vcol2 + virt_width)
                };

                let span = csarg
                    .extconceal_opt()
                    .and_then(|sp| conceal_span_at(sp, col));
                let mut lookahead_hidden = conceal_span_hidden(span, col, w);
                if !use_raw_width && lookahead_hidden != 0 {
                    use_raw_width = true;
                    if s_first == TAB && csarg.use_tabstop {
                        lookahead_hidden += raw_w - w;
                        w = raw_w;
                    }
                }
                if use_raw_width && s_first == TAB && csarg.use_tabstop && span.is_none() {
                    lookahead_hidden = (-lbr_off).min(w - 1);
                }
                lbr_off += lookahead_hidden;
                raw_vcol2 += raw_w + virt_width;

                if w == lookahead_hidden && virt_width == 0 {
                    continue;
                }

                let replacement = span.map_or(0, |sp| conceal_span_char(sp, col));
                if replacement != 0 {
                    c = schar_get_first_codepoint(replacement);
                }
                let is_break = c < 256 && vim_isbreak(c);
                if !is_break && prev_break && vcol2 != disp_vcol {
                    break;
                }
                prev_break = is_break;
                vcol2 += w + virt_width - lookahead_hidden;
                if vcol2 >= colmax {
                    size = colmax - disp_vcol + col_adj;
                    break;
                }
            }
        }

        let tail = size - size_before_lbr;

        csarg.extconceal_off += hidden;
        CharSize {
            width: size + hidden,
            body,
            head,
            tail,
            linebreak: need_lbr,
            hidden,
        }
    }
}

#[inline]
fn charsize_fast_impl(
    wp: &mut WinInfoInfo,
    use_tabstop: bool,
    vcol: Colnr,
    cur_char: i32,
    mb_width: i32,
) -> CharSize {
    if cur_char == TAB && use_tabstop {
        let width = tabstop_padding(
            vcol,
            wp.buf_mut().b_p_ts.get(),
            &wp.buf_mut().b_p_vts_array.borrow(),
        );
        CharSize {
            width,
            body: width,
            head: 0,
            tail: 0,
            linebreak: false,
            hidden: 0,
        }
    } else {
        let width = if cur_char < 0 {
            K_INVALID_BYTE_CELLS
        } else {
            mb_width
        };

        if width == 2
            && cur_char >= 0x80
            && wp.w_onebuf_opt.wo_wrap.get()
            && in_win_border(wp, vcol)
        {
            CharSize {
                width: 3,
                body: 2,
                head: 1,
                tail: 0,
                linebreak: false,
                hidden: 0,
            }
        } else {
            CharSize {
                width,
                body: width,
                head: 0,
                tail: 0,
                linebreak: false,
                hidden: 0,
            }
        }
    }
}

pub fn charsize_fast(
    csarg: &mut CharsizeArg<'_>,
    vcol: Colnr,
    cur_char: i32,
    mb_width: i32,
) -> CharSize {
    charsize_fast_impl(csarg.win_mut(), csarg.use_tabstop, vcol, cur_char, mb_width)
}

pub fn charsize_nowrap(
    buf: &mut FileBuf,
    use_tabstop: bool,
    vcol: Colnr,
    cur_char: i32,
    mb_width: i32,
) -> i32 {
    if cur_char == TAB && use_tabstop {
        tabstop_padding(vcol, buf.b_p_ts.get(), &buf.b_p_vts_array.borrow())
    } else if cur_char < 0 {
        K_INVALID_BYTE_CELLS
    } else {
        mb_width
    }
}

fn in_win_border(wp: &mut WinInfoInfo, vcol: Colnr) -> bool {
    if wp.w_view_width.get() == 0 {
        return false;
    }
    let width1 = wp.w_view_width.get() - win_col_off(wp);
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

/// Converts the displayed cell at `disp_vcol` (see CharsizeArg.extconceal_off) to its raw vcol.
/// MAXCOL stays MAXCOL.
pub fn disp_vcol2vcol(wp: &mut WinInfoInfo, lnum: Linenr, disp_vcol: Colnr) -> Colnr {
    if disp_vcol == MAXCOL {
        MAXCOL
    } else {
        extconceal_vcol_map(wp, lnum, i64::from(disp_vcol), false, true).min(i64::from(MAXCOL - 1))
            as Colnr
    }
}

/// Converts the raw cell at `vcol` to its displayed vcol (see disp_vcol2vcol()).
pub fn vcol2disp_vcol(wp: &mut WinInfoInfo, lnum: Linenr, vcol: Colnr) -> Colnr {
    if vcol == MAXCOL {
        MAXCOL
    } else {
        extconceal_vcol_map(wp, lnum, i64::from(vcol), true, true).min(i64::from(MAXCOL - 1))
            as Colnr
    }
}

/// Like linetabsize(), but excludes cells hidden by extmark-conceal.
pub fn linetabsize_disp(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    let mut csarg = CharsizeArg::new();
    let mut spans = ConcealSpans::new();
    let cstype = init_charsize_arg_extconceal(&mut csarg, wp, lnum, &mut spans);
    if cstype == K_CHARSIZE_FAST {
        linesize_fast(&csarg, 0, MAXCOL)
    } else {
        linesize_regular(&mut csarg, 0, MAXCOL)
    }
}

/// Like linetabsize_disp(), but counts the size of 'listchars' "eol".
pub fn linetabsize_disp_eol(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    linetabsize_disp(wp, lnum)
        + if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol != 0 {
            1
        } else {
            0
        }
}

/// Maps `target` on line `lnum` between raw and displayed vcols (see CharsizeArg.extconceal_off).
fn extconceal_vcol_map(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    target: i64,
    to_disp: bool,
    cell: bool,
) -> i64 {
    if target < if cell { 0 } else { 1 } || !line_has_extconceal(wp, lnum) {
        return target;
    }

    let mut csarg = CharsizeArg::new();
    let mut spans = ConcealSpans::new();
    init_charsize_arg_extconceal(&mut csarg, wp, lnum, &mut spans);
    if spans.is_empty() {
        return target;
    }

    let mut rawarg = CharsizeArg::new();
    init_charsize_arg(&mut rawarg, wp, lnum, csarg.line);
    let mut raw_vcol: i64 = 0;
    let mut vcol: i64 = 0;
    let mut from: i64 = 0;
    let mut to: i64 = 0;
    let mut ci = utf_ptr2_str_char_info(csarg.line);
    while (from < target || !to_disp) && !ci.ptr.is_empty() {
        let cli = utf_cluster_info(ci);
        let raw = charsize_regular(
            &mut rawarg,
            ci.cstr(),
            raw_vcol as Colnr,
            ci.chr.value,
            cli.cells,
        );
        let cs = charsize_regular(
            &mut csarg,
            ci.cstr(),
            vcol as Colnr,
            ci.chr.value,
            cli.cells,
        );
        let hidden = cs.hidden;
        let from_width = i64::from(if to_disp {
            raw.width
        } else {
            cs.width - hidden
        });
        let to_width = i64::from(if to_disp {
            cs.width - hidden
        } else {
            raw.width
        });

        if target - from >= from_width {
            from += from_width;
            to += to_width;
            raw_vcol += i64::from(raw.width);
            vcol += i64::from(cs.width);
            ci = cli.next;
            continue;
        }

        let tab_end = ci.chr.value == TAB
            && csarg.use_tabstop
            && conceal_span_at(&spans, ci.offset_from(csarg.line) as i32).is_none();
        let own_raw = raw.body - rawarg.cur_text_width_left - rawarg.cur_text_width_right;
        let own_cs = cs.body - csarg.cur_text_width_left - csarg.cur_text_width_right;
        let raw_parts = [raw.width - raw.tail - own_raw, own_raw, raw.tail];
        let disp_parts = [cs.width - cs.tail - own_cs, own_cs - hidden, cs.tail];
        let from_parts = if to_disp { &raw_parts } else { &disp_parts };
        let to_parts = if to_disp { &disp_parts } else { &raw_parts };
        let to_start = to;
        let mut remaining = target - from;
        for i in 0..3 {
            let fp = i64::from(from_parts[i]);
            let tp = i64::from(to_parts[i]);
            if remaining < fp {
                to += if i == 1 && tab_end {
                    (remaining + tp - fp).max(0)
                } else {
                    remaining.min(tp)
                };
                break;
            }
            remaining -= fp;
            to += tp;
        }
        if cell && to_width > 0 {
            to = if !to_disp && hidden > 0 {
                to_start + to_width - 1
            } else {
                to.min(to_start + to_width - 1)
            };
        }
        from = target;
        break;
    }

    to + target - from
}

pub fn linesize_regular(csarg: &mut CharsizeArg<'_>, mut vcol_arg: i32, len: Colnr) -> i32 {
    let mut vcol = i64::from(vcol_arg);
    let mut raw_vcol = i64::from(vcol_arg);

    let line_cstr = csarg.line;

    let mut ci = utf_ptr2_str_char_info(line_cstr);
    while (ci.offset_from(line_cstr) as Colnr) < len && !ci.ptr.is_empty() {
        let cli = utf_cluster_info(ci);
        let cs = charsize_regular(
            &mut *csarg,
            ci.cstr(),
            raw_vcol as Colnr,
            ci.chr.value,
            cli.cells,
        );
        vcol += i64::from(cs.width - cs.hidden);
        raw_vcol += i64::from(cs.width);
        ci = cli.next;
        if vcol > i64::from(MAXCOL) {
            vcol_arg = MAXCOL;
            break;
        } else {
            vcol_arg = vcol as i32;
        }
    }

    if len == MAXCOL && csarg.virt_row >= 0 && ci.ptr.is_empty() {
        let head =
            charsize_regular(&mut *csarg, ci.cstr(), raw_vcol as Colnr, ci.chr.value, 0).head;
        vcol += i64::from(csarg.cur_text_width_left + csarg.cur_text_width_right + head);
        vcol_arg = if vcol > i64::from(MAXCOL) {
            MAXCOL
        } else {
            vcol as i32
        };
    }

    vcol_arg
}

pub fn linesize_fast(csarg: &CharsizeArg<'_>, mut vcol_arg: i32, len: Colnr) -> i32 {
    let use_tabstop = csarg.use_tabstop;
    let line_cstr = csarg.line;
    let mut vcol = i64::from(vcol_arg);

    let mut ci = utf_ptr2_str_char_info(line_cstr);
    while (ci.offset_from(line_cstr) as Colnr) < len && !ci.ptr.is_empty() {
        let cli = utf_cluster_info(ci);
        vcol += i64::from(
            charsize_fast_impl(
                csarg.win_mut(),
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
            vcol_arg = vcol as i32;
        }
    }

    vcol_arg
}

fn virt_text_cursor_off(csarg: &CharsizeArg<'_>, on_nul: bool) -> i32 {
    let mut off = 0;
    if !on_nul || (g_st().state_global.get() & MODE_NORMAL) == 0 {
        off += csarg.cur_text_width_left;
    }
    if !on_nul && (g_st().state_global.get() & MODE_NORMAL) != 0 {
        off += csarg.cur_text_width_right;
    }
    off
}

pub fn getvcol(
    wp: &mut WinInfoInfo,
    pos: &mut CursorPos,
    mut start: Option<&mut Colnr>,
    mut cursor: Option<&mut Colnr>,
    end: Option<&mut Colnr>,
    flags: i32,
) {
    let end_col = pos.col;

    let mut csarg = CharsizeArg::new();
    let mut on_nul = false;
    let mut spans = ConcealSpans::new();
    let cstype = if (flags & GETVCOL_DISP) != 0 {
        init_charsize_arg_extconceal(&mut csarg, &mut *wp, pos.lnum, &mut spans)
    } else {
        let line_ptr = ml_get_buf(wp.buf_mut(), pos.lnum);
        init_charsize_arg(&mut csarg, &mut *wp, pos.lnum, line_ptr)
    };
    csarg.max_head_vcol = -1;

    let mut vcol: Colnr = 0;
    let mut off: i32 = 0;
    let mut char_size: CharSize;
    let mut ci = utf_ptr2_str_char_info(csarg.line);
    if cstype == K_CHARSIZE_FAST {
        let use_tabstop = csarg.use_tabstop;
        loop {
            if ci.ptr.is_empty() {
                char_size = CharSize {
                    width: 1,
                    body: 1,
                    head: 0,
                    tail: 0,
                    linebreak: false,
                    hidden: 0,
                };
                break;
            }
            let cli = utf_cluster_info(ci);
            char_size = charsize_fast_impl(&mut *wp, use_tabstop, vcol, ci.chr.value, cli.cells);
            let next = cli.next;
            if (next.offset_from(csarg.line) as Colnr) > end_col {
                break;
            }
            ci = next;
            vcol += char_size.width;
        }
    } else {
        loop {
            let cli = utf_cluster_info(ci);
            off = csarg.extconceal_off;
            char_size = charsize_regular(&mut csarg, ci.cstr(), vcol, ci.chr.value, cli.cells);
            if ci.ptr.is_empty() {
                char_size.width = 1 + csarg.cur_text_width_left + csarg.cur_text_width_right;
                char_size.hidden = 0;
                on_nul = true;
                break;
            }
            let next = cli.next;
            if (next.offset_from(csarg.line) as Colnr) > end_col {
                break;
            }
            ci = next;
            vcol += char_size.width;
        }
    }
    let ci_col = ci.offset_from(csarg.line) as Colnr;
    let concealed = conceal_span_at(&spans, ci_col).is_some();

    if ci.ptr.is_empty() && end_col < MAXCOL && end_col > ci_col {
        pos.col = ci_col;
    }

    let incr = char_size.width;
    let head = char_size.head;
    let tail = char_size.tail;

    if let Some(ref mut p) = start {
        **p = vcol + head - off;
    }
    if let Some(end) = end {
        *end = vcol + incr
            - char_size.hidden
            - if (flags & GETVCOL_END_EXCL_LBR) != 0 {
                tail
            } else {
                0
            }
            - 1
            - off;
    }
    if cursor.is_some() {
        if ci.chr.value == TAB
            && !concealed
            && (g_st().state_global.get() & MODE_NORMAL) != 0
            && !wp.w_onebuf_opt.wo_list.get()
            && !virtual_active(&mut *wp)
            && !(g_st().visual_global.borrow().active
                && (opt_vars().p_sel.first_byte() == b'e'
                    || ltoreq(*pos, g_st().visual_global.borrow().start)))
        {
            *cursor.as_deref_mut().unwrap() = vcol + incr - char_size.hidden - 1 - off;
        } else {
            vcol += virt_text_cursor_off(&csarg, on_nul);
            *cursor.unwrap() = vcol + head - off;
        }
    }
}

pub fn getvcol_nolist(posp: &mut CursorPos) -> Colnr {
    let list_save = curwin_mut().w_onebuf_opt.wo_list.get();
    let mut vcol: Colnr = 0;
    let mut pos = *posp;

    curwin_mut().w_onebuf_opt.wo_list.set(false);
    if pos.coladd != 0 {
        getvvcol(curwin_mut(), &mut pos, None, Some(&mut vcol), None, 0);
    } else {
        getvcol(curwin_mut(), &mut pos, None, Some(&mut vcol), None, 0);
    }
    *posp = pos;
    curwin_mut().w_onebuf_opt.wo_list.set(list_save);
    vcol
}

pub fn getvvcol(
    wp: &mut WinInfoInfo,
    pos: &mut CursorPos,
    mut start: Option<&mut Colnr>,
    mut cursor: Option<&mut Colnr>,
    mut end: Option<&mut Colnr>,
    flags: i32,
) {
    let mut col: Colnr = 0;

    if virtual_active(&mut *wp) {
        getvcol(&mut *wp, pos, Some(&mut col), None, None, flags);
        let mut coladd = pos.coladd;
        let mut endadd: Colnr = 0;

        let line_slice = crate::memline::ml_get_buf_slice((*wp).buf_mut(), pos.lnum);
        if (pos.col as usize) < line_slice.len() {
            let cur_slice = &line_slice[pos.col as usize..];
            let c = crate::mbyte::utf_slice2char(cur_slice);
            if c != TAB && vim_isprintc(c) {
                endadd = crate::mbyte::slice2cells(cur_slice) - 1;
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

pub fn getvcols(
    wp: &mut WinInfoInfo,
    pos1: &mut CursorPos,
    pos2: &mut CursorPos,
    left: &mut Colnr,
    right: &mut Colnr,
    flags: i32,
) {
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
        if opt_vars().p_sel.first_byte() == b'e' && from2 > to1 {
            *right = from2 - 1;
        } else {
            *right = to2;
        }
    } else {
        *right = to1;
    }
}

pub fn win_may_fill(wp: &mut WinInfoInfo) -> bool {
    (wp.w_onebuf_opt.wo_diff.get() && diffopt_filler())
        || wp.buf_mut().b_marktree.borrow().meta_root[K_MTMETA_LINES] != 0
}

pub fn win_get_fill(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    decor_virt_lines(wp, lnum - 1, lnum, None, None, true) + diff_check_fill(wp, lnum)
}

pub fn plines_win(wp: &mut WinInfoInfo, lnum: Linenr, limit_winheight: bool) -> i32 {
    plines_win_nofill(wp, lnum, limit_winheight) + win_get_fill(wp, lnum)
}

pub fn plines_win_nofill(wp: &mut WinInfoInfo, lnum: Linenr, limit_winheight: bool) -> i32 {
    if decor_conceal_line(wp, lnum - 1, false) {
        return 0;
    }
    if !wp.w_onebuf_opt.wo_wrap.get() || wp.w_view_width.get() == 0 || line_folded(&mut *wp, lnum) {
        return 1;
    }
    let lines = plines_win_nofold(&mut *wp, lnum);
    if limit_winheight && lines > wp.w_view_height.get() {
        wp.w_view_height.get()
    } else {
        lines
    }
}

pub fn plines_win_nofold(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    let mut csarg = CharsizeArg::new();
    let mut spans = ConcealSpans::new();
    let cstype = init_charsize_arg_extconceal(&mut csarg, &mut *wp, lnum, &mut spans);
    if csarg.line.to_bytes().is_empty() && csarg.virt_row < 0 {
        return 1;
    }

    let mut col = if cstype == K_CHARSIZE_FAST {
        i64::from(linesize_fast(&csarg, 0, MAXCOL))
    } else {
        i64::from(linesize_regular(&mut csarg, 0, MAXCOL))
    };

    if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol != 0 {
        col += 1;
    }

    let mut width = wp.w_view_width.get() - win_col_off(wp);
    if width <= 0 {
        return 32000;
    }
    if col <= i64::from(width) {
        return 1;
    }
    col -= i64::from(width);
    width += win_col_off2(&mut *wp);
    let lines = (col + (i64::from(width) - 1)) / i64::from(width) + 1;
    if lines > 0 && lines <= i64::from(i32::MAX) {
        lines as i32
    } else {
        i32::MAX
    }
}

pub fn plines_win_col(wp: &mut WinInfoInfo, lnum: Linenr, column: i64) -> i32 {
    let mut lines = win_get_fill(wp, lnum);
    if !wp.w_onebuf_opt.wo_wrap.get() || wp.w_view_width.get() == 0 {
        return lines + 1;
    }

    let mut csarg = CharsizeArg::new();
    let mut spans = ConcealSpans::new();
    let cstype = init_charsize_arg_extconceal(&mut csarg, &mut *wp, lnum, &mut spans);
    let line_cstr = csarg.line;

    let mut vcol: Colnr = 0;
    let mut raw_vcol: Colnr = 0;
    let mut ci = utf_ptr2_str_char_info(line_cstr);
    let limit_col = column as usize;
    if cstype == K_CHARSIZE_FAST {
        let use_tabstop = csarg.use_tabstop;
        while !ci.ptr.is_empty() && ci.offset_from(line_cstr) < limit_col {
            let cli = utf_cluster_info(ci);
            vcol += charsize_fast_impl(&mut *wp, use_tabstop, vcol, ci.chr.value, cli.cells).width;
            ci = cli.next;
        }
        raw_vcol = vcol;
    } else {
        while !ci.ptr.is_empty() && ci.offset_from(line_cstr) < limit_col {
            let cli = utf_cluster_info(ci);
            let cs = charsize_regular(&mut csarg, ci.cstr(), raw_vcol, ci.chr.value, cli.cells);
            let hidden = cs.hidden;
            vcol += cs.width - hidden;
            raw_vcol += cs.width;
            ci = cli.next;
        }
    }

    let mut col = vcol;
    if ci.chr.value == TAB && (g_st().state_global.get() & MODE_NORMAL) != 0 && csarg.use_tabstop {
        let cs = win_charsize(cstype, raw_vcol, ci.cstr(), ci.chr.value, &mut csarg, 1);
        let hidden = cs.hidden;
        col += (cs.width - hidden - 1).max(0);
    }

    let width = wp.w_view_width.get() - win_col_off(wp);
    if width <= 0 {
        return 9999;
    }

    lines += 1;
    if col > width {
        lines += (col - width) / (width + win_col_off2(&mut *wp)) + 1;
    }
    lines
}

pub fn plines_win_full(
    wp: &mut WinInfoInfo,
    mut lnum: Linenr,
    nextp: Option<&mut Linenr>,
    mut foldedp: Option<&mut bool>,
    cache: bool,
    limit_winheight: bool,
) -> i32 {
    let folded = has_folding_win(&mut *wp, lnum, Some(&mut lnum), nextp, cache, None);
    if let Some(ref mut p) = foldedp {
        **p = folded;
    }

    let filler_lines = if lnum == wp.w_topline.get() {
        wp.w_topfill.get()
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

pub fn plines_m_win(wp: &mut WinInfoInfo, mut first: Linenr, last: Linenr, max: i32) -> i32 {
    let mut count = 0;
    while first <= last && count < max {
        let mut next = first;
        count += plines_win_full(&mut *wp, first, Some(&mut next), None, false, false);
        first = next + 1;
    }
    if first == wp.buf_mut().b_ml.ml_line_count.get() + 1 {
        count += win_get_fill(wp, first);
    }
    max.min(count)
}

pub fn plines_m_win_fill(wp: &mut WinInfoInfo, first: Linenr, last: Linenr) -> i32 {
    let mut count = last - first + 1 + decor_virt_lines(wp, first - 1, last, None, None, false);

    if diffopt_filler() {
        for lnum in first..=last {
            let n = diff_check_fill(wp, lnum);
            count += n.max(0);
        }
    }

    count.max(0)
}

pub fn win_text_height(
    wp: &mut WinInfoInfo,
    start_lnum: Linenr,
    start_vcol: i64,
    end_lnum: &mut Linenr,
    end_vcol: &mut i64,
    fill: Option<&mut i64>,
    max: i64,
    vcol_is_disp: bool,
) -> i64 {
    let mut width1 = wp.w_view_width.get() - win_col_off(wp);
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
        cur_folded = has_folding(&mut *wp, lnum, Some(&mut lnum), Some(&mut lnum_next));
        height_cur_nofill = i64::from(plines_win_nofill(&mut *wp, lnum, false));
        height_sum_nofill += height_cur_nofill;
        let start_disp = if !vcol_is_disp && !cur_folded {
            extconceal_vcol_map(&mut *wp, lnum, start_vcol, true, false)
        } else {
            start_vcol
        };
        let row_off = if start_disp < i64::from(width1) || width2 <= 0 {
            0
        } else {
            1 + (start_disp - i64::from(width1)) / i64::from(width2)
        };
        height_sum_nofill -= row_off.min(height_cur_nofill);
        lnum = lnum_next + 1;
    }

    while lnum <= *end_lnum && height_sum_nofill + height_sum_fill < max {
        let mut lnum_next = lnum;
        cur_folded = has_folding(&mut *wp, lnum, Some(&mut lnum), Some(&mut lnum_next));
        height_sum_fill += i64::from(win_get_fill(wp, lnum));
        height_cur_nofill = i64::from(plines_win_nofill(&mut *wp, lnum, false));
        height_sum_nofill += height_cur_nofill;
        cur_lnum = lnum;
        lnum = lnum_next + 1;
    }

    let mut vcol_end = *end_vcol;
    let use_vcol = vcol_end >= 0 && lnum > *end_lnum;
    if use_vcol {
        height_sum_nofill -= height_cur_nofill;
        let end_disp = if !vcol_is_disp && !cur_folded {
            extconceal_vcol_map(&mut *wp, cur_lnum, vcol_end, true, false)
        } else {
            vcol_end
        };
        let row_off = if end_disp == 0 {
            0
        } else if end_disp <= i64::from(width1) || width2 <= 0 {
            1
        } else {
            1 + (end_disp - i64::from(width1) + i64::from(width2) - 1) / i64::from(width2)
        };
        height_sum_nofill += row_off.min(height_cur_nofill);
    }

    if cur_folded {
        vcol_end = 0;
    } else {
        let linesize = i64::from(if vcol_is_disp {
            linetabsize_disp_eol(&mut *wp, cur_lnum)
        } else {
            linetabsize_eol(&mut *wp, cur_lnum)
        });
        vcol_end = (if use_vcol { vcol_end } else { i64::MAX }).min(linesize);
    }

    let overflow = height_sum_nofill + height_sum_fill - max;
    if overflow > 0 && width2 > 0 {
        let mut end_disp = if vcol_is_disp {
            vcol_end
        } else {
            extconceal_vcol_map(&mut *wp, cur_lnum, vcol_end, true, false)
        };
        if end_disp > i64::from(width2) {
            end_disp -= (end_disp - i64::from(width1)) % i64::from(width2)
                + (overflow - 1) * i64::from(width2);
            vcol_end = if vcol_is_disp {
                end_disp
            } else {
                extconceal_vcol_map(&mut *wp, cur_lnum, end_disp, false, false)
            };
        }
    }

    *end_lnum = cur_lnum;
    *end_vcol = vcol_end;
    if let Some(fill) = fill {
        *fill = height_sum_fill;
    }
    height_sum_fill + height_sum_nofill
}

pub fn win_max_displaywidth(wp: &mut WinInfoInfo, first: Linenr, last: Linenr, max: i32) -> i32 {
    let mut width = 0;
    let mut lnum = first;
    while lnum <= last && width < max {
        width = width.max(linetabsize(wp, lnum));
        lnum += 1;
    }
    max.min(width)
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
