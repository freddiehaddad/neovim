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
use crate::mbyte::{utf_cluster_info, utf_slice2_str_char_info};
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

#[derive(Clone)]
pub enum CsLine<'a> {
    Bytes(&'a [u8]),
    Borrowed(&'a std::ffi::CStr),
    Rc(crate::option_vars::RcCStr),
}

impl<'a> CsLine<'a> {
    #[inline(always)]
    pub fn to_bytes(&self) -> &[u8] {
        match self {
            Self::Bytes(b) => b,
            Self::Borrowed(s) => s.to_bytes(),
            Self::Rc(s) => s.to_bytes(),
        }
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const std::ffi::c_char {
        match self {
            Self::Bytes(b) => b.as_ptr().cast(),
            Self::Borrowed(s) => s.as_ptr(),
            Self::Rc(s) => s.as_ptr(),
        }
    }
}

impl<'a> From<&'a [u8]> for CsLine<'a> {
    #[inline(always)]
    fn from(b: &'a [u8]) -> Self {
        Self::Bytes(b)
    }
}

impl<'a> From<&'a std::ffi::CStr> for CsLine<'a> {
    #[inline(always)]
    fn from(s: &'a std::ffi::CStr) -> Self {
        Self::Borrowed(s)
    }
}

impl From<crate::option_vars::RcCStr> for CsLine<'_> {
    #[inline(always)]
    fn from(s: crate::option_vars::RcCStr) -> Self {
        Self::Rc(s)
    }
}

impl<'a> From<&'a crate::option_vars::RcCStr> for CsLine<'a> {
    #[inline(always)]
    fn from(s: &'a crate::option_vars::RcCStr) -> Self {
        Self::Borrowed(s.as_cstr())
    }
}

#[repr(C)]
pub struct CharsizeArg<'a> {
    pub win: Option<&'a WinInfoInfo>,
    pub line: CsLine<'a>,
    pub use_tabstop: bool,
    pub has_sbr: bool,
    pub has_vts: bool,
    pub wo_wrap: bool,
    pub wo_bri: bool,
    pub wo_lbr: bool,
    pub has_lcs_eol: bool,
    pub b_p_ts: i64,
    pub w_view_width: i32,
    pub col_off: i32,
    pub width2: i32,
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

impl<'a> CharsizeArg<'a> {
    pub const fn new() -> Self {
        Self {
            win: None,
            line: CsLine::Bytes(b""),
            use_tabstop: false,
            has_sbr: false,
            has_vts: false,
            wo_wrap: false,
            wo_bri: false,
            wo_lbr: false,
            has_lcs_eol: false,
            b_p_ts: 8,
            w_view_width: 0,
            col_off: 0,
            width2: 0,
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

    #[inline(always)]
    pub fn win(&self) -> &'a WinInfoInfo {
        self.win.expect("CharsizeArg::win is None")
    }

    #[inline]
    pub fn extconceal_opt(&self) -> Option<&ConcealSpans> {
        self.extconceal.as_ref()
    }

    #[inline(always)]
    fn can_use_fast_loop(&self, cstype: CSType) -> bool {
        cstype == K_CHARSIZE_FAST
            || (self.virt_row < 0
                && self.extconceal.is_none()
                && !self.wo_lbr
                && self.indent_width == 0)
    }
}

/// Returns the number of leading printable ASCII bytes (`0x20..=0x7E`) in `ptr`
/// that are not followed by a non-ASCII UTF-8 byte (`>= 0x80`).
#[inline(always)]
fn printable_ascii_run_len(ptr: &[u8], max_bytes: usize) -> usize {
    let limit = max_bytes.min(ptr.len());
    let slice = &ptr[..limit];
    let mut n = 0usize;
    while n + 8 <= slice.len() {
        let chunk = &slice[n..n + 8];
        if chunk.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
            n += 8;
        } else {
            break;
        }
    }
    while n < slice.len() && (0x20..=0x7E).contains(&slice[n]) {
        n += 1;
    }
    if n > 0 && ptr.get(n).is_some_and(|&b| b >= 0x80) {
        n - 1
    } else {
        n
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
pub fn win_linetabsize(wp: &WinInfoInfo, lnum: Linenr, line: &std::ffi::CStr, len: Colnr) -> i32 {
    win_linetabsize_bytes(wp, lnum, line.to_bytes(), len)
}

#[inline]
pub fn win_linetabsize_bytes(wp: &WinInfoInfo, lnum: Linenr, line: &[u8], len: Colnr) -> i32 {
    let mut csarg = CharsizeArg::new();
    let cstype = init_charsize_arg(&mut csarg, wp, lnum, line);
    if csarg.can_use_fast_loop(cstype) {
        linesize_fast(&csarg, 0, len)
    } else {
        linesize_regular(&mut csarg, 0, len)
    }
}

pub fn win_chartabsize(wp: &WinInfoInfo, p: impl crate::mbyte::AsMbyteSlice, col: Colnr) -> i32 {
    p.with_cluster_bytes(|p| {
        if p.first().copied() == Some(TAB as u8)
            && (!wp.w_onebuf_opt.wo_list.get() || wp.w_p_lcs_chars.borrow().tab1 != 0)
        {
            return wp.with_buf(|buf| {
                tabstop_padding(col, buf.b_p_ts.get(), &buf.b_p_vts_array.borrow())
            });
        }
        crate::mbyte::ptr2cells(p)
    })
}

pub fn linetabsize_col(startvcol: i32, s: &std::ffi::CStr) -> i32 {
    let wp = curwin();
    let mut csarg = CharsizeArg::new();
    let cstype = init_charsize_arg(&mut csarg, &wp, 0, s.to_bytes());
    if csarg.can_use_fast_loop(cstype) {
        linesize_fast(&csarg, startvcol, MAXCOL)
    } else {
        linesize_regular(&mut csarg, startvcol, MAXCOL)
    }
}

pub fn linetabsize(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    wp.with_buf(|buf| {
        crate::memline::with_ml_buf_line(buf, lnum, |line| {
            win_linetabsize_bytes(wp, lnum, line, MAXCOL)
        })
    })
}

pub fn linetabsize_eol(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    linetabsize(wp, lnum)
        + if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol != 0 {
            1
        } else {
            0
        }
}

/// Whether extmark-conceal may hide cells on line `lnum` of `wp` (see has_extconceal()), and
/// 'concealcursor' does not reveal the line. Cheap: does not walk the marktree.
pub fn line_has_extconceal(wp: &WinInfoInfo, lnum: Linenr) -> bool {
    lnum > 0 && has_extconceal(wp) && !conceal_cursor_reveals_line(wp, lnum)
}

/// Whether extmark-conceal (see CharsizeArg.extconceal_off), including marks an `_on_conceal`
/// provider adds on demand, may change the layout of `wp`.
pub fn has_extconceal(wp: &WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_wrap.get()
        && wp.w_onebuf_opt.wo_cole.get() > 0
        && wp.with_buf(|buf| {
            buf_meta_total(buf, K_MTMETA_CONCEAL) > 0 || buf.b_conceal_providers.get() > 0
        })
}

pub fn init_charsize_arg<'a>(
    csarg: &mut CharsizeArg<'a>,
    wp: &'a WinInfoInfo,
    lnum: Linenr,
    line: impl Into<CsLine<'a>>,
) -> CSType {
    csarg.win = Some(wp);
    csarg.line = line.into();
    csarg.max_head_vcol = 0;
    csarg.extconceal_off = 0;
    csarg.cur_text_width_left = 0;
    csarg.cur_text_width_right = 0;
    csarg.virt_row = -1;
    csarg.skip_cur_text = false;
    csarg.indent_width = i32::MIN;
    let wo_list = wp.w_onebuf_opt.wo_list.get();
    if wo_list {
        let lcs = wp.w_p_lcs_chars.borrow();
        csarg.use_tabstop = lcs.tab1 != 0;
        csarg.has_lcs_eol = lcs.eol != 0;
    } else {
        csarg.use_tabstop = true;
        csarg.has_lcs_eol = false;
    }
    let sbr = get_showbreak_value(wp);
    csarg.has_sbr = !sbr.is_empty();
    csarg.wo_wrap = wp.w_onebuf_opt.wo_wrap.get();
    csarg.wo_bri = wp.w_onebuf_opt.wo_bri.get();
    csarg.wo_lbr = wp.w_onebuf_opt.wo_lbr.get();
    csarg.w_view_width = wp.w_view_width.get();
    csarg.extconceal = None;

    wp.with_buf(|buf| {
        csarg.b_p_ts = buf.b_p_ts.get();
        csarg.has_vts = !buf.b_p_vts_array.borrow().is_empty();
        if lnum > 0
            && buf.b_marktree.borrow().meta_root[K_MTMETA_INLINE] > 0
            && marktree_itr_get_filter(
                &buf.b_marktree.borrow(),
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
    });

    if csarg.wo_wrap && (csarg.wo_lbr || csarg.wo_bri || csarg.has_sbr) {
        csarg.col_off = win_col_off(wp);
        csarg.width2 = csarg.w_view_width - csarg.col_off + win_col_off2(wp);
        if (csarg.wo_bri || csarg.has_sbr)
            && (!csarg.wo_bri || (wp.w_briopt_list.get() == 0 && wp.w_briopt_vcol.get() == 0))
        {
            let mut indent_width = 0;
            if csarg.has_sbr {
                indent_width += vim_strsize(&sbr);
            }
            if csarg.wo_bri {
                indent_width += get_breakindent_win(wp, csarg.line.to_bytes());
            }
            csarg.indent_width = indent_width;
        }
    } else {
        csarg.col_off = 0;
        csarg.width2 = 0;
    }

    if csarg.virt_row >= 0 || (csarg.wo_wrap && (csarg.wo_lbr || csarg.wo_bri || csarg.has_sbr)) {
        K_CHARSIZE_REGULAR
    } else {
        K_CHARSIZE_FAST
    }
}

/// Like init_charsize_arg(), but extmark-conceal hides cells (see CharsizeArg.extconceal_off).
/// Collects the conceal of line `lnum` into `spans`, after running its
/// `_on_conceal` provider, which may free the line buffer: read the line from `csarg.line`.
pub fn init_charsize_arg_extconceal<'a>(
    csarg: &mut CharsizeArg<'a>,
    wp: &'a WinInfoInfo,
    lnum: Linenr,
    spans: &mut ConcealSpans,
) -> CSType {
    spans.clear();
    if line_has_extconceal(wp, lnum) {
        decor_providers_invoke_conceal(wp, lnum - 1, false);
        let len = wp.with_buf(|buf| ml_get_buf_len(buf, lnum));
        conceal_spans_collect(wp, lnum - 1, len, spans);
    }
    let line = wp.with_buf(|buf| ml_get_buf(buf, lnum));
    let cstype = init_charsize_arg(csarg, wp, lnum, line);
    if spans.is_empty() {
        return cstype;
    }
    csarg.extconceal = Some(spans.clone());
    K_CHARSIZE_REGULAR
}

#[inline]
fn with_charsize_arg_extconceal<R>(
    wp: &WinInfoInfo,
    lnum: Linenr,
    spans: &mut ConcealSpans,
    f: impl FnOnce(&mut CharsizeArg<'_>, CSType) -> R,
) -> R {
    spans.clear();
    if line_has_extconceal(wp, lnum) {
        decor_providers_invoke_conceal(wp, lnum - 1, false);
        let len = wp.with_buf(|buf| ml_get_buf_len(buf, lnum));
        conceal_spans_collect(wp, lnum - 1, len, spans);
    }
    wp.with_buf(|buf| {
        crate::memline::with_ml_buf_line(buf, lnum, |line| {
            let mut csarg = CharsizeArg::new();
            let mut cstype = init_charsize_arg(&mut csarg, wp, lnum, line);
            if !spans.is_empty() {
                csarg.extconceal = Some(spans.clone());
                cstype = K_CHARSIZE_REGULAR;
            }
            f(&mut csarg, cstype)
        })
    })
}

pub fn init_charsize_arg_skip_cur_text<'a>(
    csarg: &mut CharsizeArg<'a>,
    wp: &'a WinInfoInfo,
    lnum: Linenr,
    line: &'a std::ffi::CStr,
) -> CSType {
    let mut cstype = init_charsize_arg(csarg, wp, 0, line);
    csarg.skip_cur_text = true;
    if lnum > 0 && wp.with_buf(|buf| buf.b_marktree.borrow().meta_root[K_MTMETA_INLINE] > 0) {
        csarg.virt_row = lnum - 1;
        cstype = K_CHARSIZE_REGULAR;
    }
    cstype
}

fn inline_virt_text_width(wp: &WinInfoInfo, iter: &mut MarkTreeIter, row: i32, col: i32) -> i32 {
    let mut width: i32 = 0;
    wp.with_buf(|buf| {
        let mt = buf.b_marktree.borrow();
        loop {
            let mark = marktree_itr_current(&mt, iter);
            if mark.pos.row != row || mark.pos.col > col {
                break;
            } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, wp) {
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
            marktree_itr_next_filter(&mt, iter, row + 1, 0, &INLINE_FILTER);
        }
    });
    width
}

/// Collects the extmark-conceal of row `row` (`len` bytes) into `spans`, without invoking
/// `_on_conceal`.
pub fn conceal_spans_collect(wp: &WinInfoInfo, row: i32, len: i32, spans: &mut ConcealSpans) {
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
    let cur_bytes = if cur.last() == Some(&0) {
        crate::memory::bytes_until_nul(cur)
    } else {
        cur
    };
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

        let wp = csarg.win();
        let use_tabstop = cur_char == TAB && csarg.use_tabstop;
        let mut mb_added = 0;

        let disp_vcol = vcol - csarg.extconceal_off;

        let mut size: i32;
        let mut is_doublewidth = false;
        if use_tabstop {
            size = if csarg.has_vts {
                wp.with_buf(|buf| tabstop_padding(vcol, csarg.b_p_ts, &buf.b_p_vts_array.borrow()))
            } else {
                tabstop_padding(vcol, csarg.b_p_ts, &[])
            };
        } else if cur_bytes.is_empty() {
            size = if csarg.has_lcs_eol { 1 } else { 0 };
        } else if cur_char < 0 {
            size = K_INVALID_BYTE_CELLS;
        } else {
            size = mb_width;
            is_doublewidth = size == 2 && cur_char >= 0x80;
        }

        if csarg.virt_row >= 0 && !csarg.skip_cur_text {
            let mut tab_size = size;
            let col = cur_off as i32;
            wp.with_buf(|buf| {
                let mt = buf.b_marktree.borrow();
                loop {
                    let mark = marktree_itr_current(&mt, &csarg.iter);
                    if mark.pos.row != csarg.virt_row || mark.pos.col > col {
                        break;
                    } else if mark.pos.col == col && !mt_invalid(mark) && ns_in_win(mark.ns, wp) {
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
                                            csarg.b_p_ts,
                                            &buf.b_p_vts_array.borrow(),
                                        );
                                        size += tab_size;
                                    }
                                }
                            },
                        );
                    }
                    marktree_itr_next_filter(
                        &mt,
                        &mut csarg.iter,
                        csarg.virt_row + 1,
                        0,
                        &INLINE_FILTER,
                    );
                }
            });
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

        if is_doublewidth && csarg.wo_wrap && in_win_border(wp, disp_vcol + size - 2) {
            size += 1;
            mb_added = 1;
        }

        let mut head = mb_added;
        if size > 0 && csarg.wo_wrap && (csarg.has_sbr || csarg.wo_bri) && csarg.indent_width != 0 {
            let view_width = csarg.w_view_width;
            let mut col_off_prev = csarg.col_off;
            let width2 = csarg.width2;
            let mut wcol = disp_vcol + col_off_prev;
            let max_head_vcol = csarg.max_head_vcol;
            let mut added = 0;

            let mut head_prev = 0;
            if wcol >= view_width {
                wcol -= view_width;
                col_off_prev = view_width - width2;
                if wcol >= width2 && width2 > 0 {
                    wcol %= width2;
                }
                head_prev = csarg.indent_width;
                if head_prev == i32::MIN {
                    head_prev = 0;
                    if csarg.has_sbr {
                        let sbr = get_showbreak_value(wp);
                        head_prev += vim_strsize(&sbr);
                    }
                    if csarg.wo_bri {
                        head_prev += get_breakindent_win(wp, line_bytes);
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

            if wcol + size > view_width {
                let mut head_mid = csarg.indent_width;
                if head_mid == i32::MIN {
                    head_mid = 0;
                    if csarg.has_sbr {
                        let sbr = get_showbreak_value(wp);
                        head_mid += vim_strsize(&sbr);
                    }
                    if csarg.wo_bri {
                        head_mid += get_breakindent_win(wp, line_bytes);
                    }
                    csarg.indent_width = head_mid;
                }
                if head_mid > 0 {
                    let prev_rem = view_width - wcol;
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
        if csarg.wo_lbr
            && csarg.wo_wrap
            && csarg.w_view_width != 0
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
            let numberextra = csarg.col_off;
            let col_adj = size - 1;
            let mut colmax: Colnr = csarg.w_view_width - numberextra - col_adj;
            if disp_vcol >= colmax {
                colmax += col_adj;
                let n = csarg.width2;
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
                    has_virt = i32::from(wp.with_buf(|buf| {
                        marktree_itr_get_filter(
                            &buf.b_marktree.borrow(),
                            csarg.virt_row,
                            col,
                            csarg.virt_row + 1,
                            0,
                            &INLINE_FILTER,
                            &mut virt_iter,
                        )
                    }));
                }
                let virt_width = if has_virt != 0 {
                    inline_virt_text_width(wp, &mut virt_iter, csarg.virt_row, col)
                } else {
                    0
                };

                let s_bytes = &line_bytes[s_off..];
                let s_first = i32::from(s_bytes[0]);
                let raw_w = if csarg.extconceal.is_none() {
                    0
                } else if s_first == TAB && csarg.use_tabstop {
                    if csarg.has_vts {
                        wp.with_buf(|buf| {
                            tabstop_padding(
                                raw_vcol2 + virt_width,
                                csarg.b_p_ts,
                                &buf.b_p_vts_array.borrow(),
                            )
                        })
                    } else {
                        tabstop_padding(raw_vcol2 + virt_width, csarg.b_p_ts, &[])
                    }
                } else {
                    win_chartabsize(wp, s_bytes, raw_vcol2)
                };
                let mut w = if use_raw_width {
                    raw_w
                } else {
                    win_chartabsize(wp, s_bytes, vcol2 + virt_width)
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
    wp: &WinInfoInfo,
    use_tabstop: bool,
    has_vts: bool,
    b_p_ts: i64,
    vcol: Colnr,
    cur_char: i32,
    mb_width: i32,
) -> CharSize {
    if cur_char == TAB && use_tabstop {
        let width = if has_vts {
            wp.with_buf(|buf| tabstop_padding(vcol, b_p_ts, &buf.b_p_vts_array.borrow()))
        } else {
            tabstop_padding(vcol, b_p_ts, &[])
        };
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
    charsize_fast_impl(
        csarg.win(),
        csarg.use_tabstop,
        csarg.has_vts,
        csarg.b_p_ts,
        vcol,
        cur_char,
        mb_width,
    )
}

pub fn charsize_nowrap(
    buf: &FileBuf,
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

fn in_win_border(wp: &WinInfoInfo, vcol: Colnr) -> bool {
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
pub fn disp_vcol2vcol(wp: &WinInfoInfo, lnum: Linenr, disp_vcol: Colnr) -> Colnr {
    if disp_vcol == MAXCOL {
        MAXCOL
    } else {
        extconceal_vcol_map(wp, lnum, i64::from(disp_vcol), false, true).min(i64::from(MAXCOL - 1))
            as Colnr
    }
}

/// Converts the raw cell at `vcol` to its displayed vcol (see disp_vcol2vcol()).
pub fn vcol2disp_vcol(wp: &WinInfoInfo, lnum: Linenr, vcol: Colnr) -> Colnr {
    if vcol == MAXCOL {
        MAXCOL
    } else {
        extconceal_vcol_map(wp, lnum, i64::from(vcol), true, true).min(i64::from(MAXCOL - 1))
            as Colnr
    }
}

/// Like linetabsize(), but excludes cells hidden by extmark-conceal.
pub fn linetabsize_disp(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    let mut spans = ConcealSpans::new();
    with_charsize_arg_extconceal(wp, lnum, &mut spans, |csarg, cstype| {
        if csarg.can_use_fast_loop(cstype) {
            linesize_fast(csarg, 0, MAXCOL)
        } else {
            linesize_regular(csarg, 0, MAXCOL)
        }
    })
}

/// Like linetabsize_disp(), but counts the size of 'listchars' "eol".
pub fn linetabsize_disp_eol(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    linetabsize_disp(wp, lnum)
        + if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol != 0 {
            1
        } else {
            0
        }
}

/// Maps `target` on line `lnum` between raw and displayed vcols (see CharsizeArg.extconceal_off).
fn extconceal_vcol_map(
    wp: &WinInfoInfo,
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
    init_charsize_arg(&mut rawarg, wp, lnum, csarg.line.clone());
    let mut raw_vcol: i64 = 0;
    let mut vcol: i64 = 0;
    let mut from: i64 = 0;
    let mut to: i64 = 0;
    let line_cstr = csarg.line.clone();
    let line_bytes = line_cstr.to_bytes();
    let mut ci = utf_slice2_str_char_info(line_bytes);
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
            && conceal_span_at(&spans, ci.offset_from_bytes(line_bytes) as i32).is_none();
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
    if csarg.can_use_fast_loop(K_CHARSIZE_REGULAR) {
        return linesize_fast(csarg, vcol_arg, len);
    }
    let mut vcol = i64::from(vcol_arg);
    let mut raw_vcol = i64::from(vcol_arg);

    let line_cstr = csarg.line.clone();
    let line_bytes = line_cstr.to_bytes();

    let mut ci = utf_slice2_str_char_info(line_bytes);
    while (ci.offset_from_bytes(line_bytes) as Colnr) < len && !ci.ptr.is_empty() {
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
    let has_vts = csarg.has_vts;
    let b_p_ts = csarg.b_p_ts;
    let line_bytes = csarg.line.to_bytes();
    let mut vcol = i64::from(vcol_arg);
    let wp = csarg.win();

    let mut ci = utf_slice2_str_char_info(line_bytes);
    while (ci.offset_from_bytes(line_bytes) as Colnr) < len && !ci.ptr.is_empty() {
        let cur_off = ci.offset_from_bytes(line_bytes);
        let rem_limit = (len.max(0) as usize).saturating_sub(cur_off);
        let ascii_run = printable_ascii_run_len(ci.ptr, rem_limit);
        if ascii_run > 0 {
            vcol += ascii_run as i64;
            ci = utf_slice2_str_char_info(&ci.ptr[ascii_run..]);
            if vcol > i64::from(MAXCOL) {
                vcol_arg = MAXCOL;
                break;
            }
            vcol_arg = vcol as i32;
            continue;
        }
        let cli = utf_cluster_info(ci);
        vcol += i64::from(
            charsize_fast_impl(
                wp,
                use_tabstop,
                has_vts,
                b_p_ts,
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
    wp: &WinInfoInfo,
    pos: &mut CursorPos,
    mut start: Option<&mut Colnr>,
    mut cursor: Option<&mut Colnr>,
    end: Option<&mut Colnr>,
    flags: i32,
) {
    let end_col = pos.col;
    let mut spans = ConcealSpans::new();
    if (flags & GETVCOL_DISP) != 0 && line_has_extconceal(wp, pos.lnum) {
        decor_providers_invoke_conceal(wp, pos.lnum - 1, false);
        let len = wp.with_buf(|buf| ml_get_buf_len(buf, pos.lnum));
        conceal_spans_collect(wp, pos.lnum - 1, len, &mut spans);
    }
    wp.with_buf(|buf| {
        crate::memline::with_ml_buf_line(buf, pos.lnum, |line_bytes| {
            let mut csarg = CharsizeArg::new();
            let mut on_nul = false;
            let mut cstype = init_charsize_arg(&mut csarg, wp, pos.lnum, line_bytes);
            if !spans.is_empty() {
                csarg.extconceal = Some(spans.clone());
                cstype = K_CHARSIZE_REGULAR;
            }
            csarg.max_head_vcol = -1;

            let mut vcol: Colnr = 0;
            let mut off: i32 = 0;
            let mut char_size: CharSize;
            let mut ci = utf_slice2_str_char_info(line_bytes);
            if csarg.can_use_fast_loop(cstype) {
                let use_tabstop = csarg.use_tabstop;
                let has_vts = csarg.has_vts;
                let b_p_ts = csarg.b_p_ts;
                loop {
                    let cur_off = ci.offset_from_bytes(line_bytes);
                    let max_adv = (end_col.max(0) as usize).saturating_sub(cur_off);
                    if max_adv > 0 {
                        let ascii_run = printable_ascii_run_len(ci.ptr, max_adv);
                        if ascii_run > 0 {
                            vcol += ascii_run as Colnr;
                            ci = utf_slice2_str_char_info(&ci.ptr[ascii_run..]);
                        }
                    }
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
                    char_size = charsize_fast_impl(
                        wp,
                        use_tabstop,
                        has_vts,
                        b_p_ts,
                        vcol,
                        ci.chr.value,
                        cli.cells,
                    );
                    let next = cli.next;
                    if (next.offset_from_bytes(line_bytes) as Colnr) > end_col {
                        break;
                    }
                    ci = next;
                    vcol += char_size.width;
                }
            } else {
                loop {
                    let cli = utf_cluster_info(ci);
                    off = csarg.extconceal_off;
                    char_size =
                        charsize_regular(&mut csarg, ci.cstr(), vcol, ci.chr.value, cli.cells);
                    if ci.ptr.is_empty() {
                        char_size.width =
                            1 + csarg.cur_text_width_left + csarg.cur_text_width_right;
                        char_size.hidden = 0;
                        on_nul = true;
                        break;
                    }
                    let next = cli.next;
                    if (next.offset_from_bytes(line_bytes) as Colnr) > end_col {
                        break;
                    }
                    ci = next;
                    vcol += char_size.width;
                }
            }
            let ci_col = ci.offset_from_bytes(line_bytes) as Colnr;
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
                    && !virtual_active(wp)
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
        });
    });
}

pub fn getvcol_nolist(posp: &mut CursorPos) -> Colnr {
    let list_save = curwin().w_onebuf_opt.wo_list.get();
    let mut vcol: Colnr = 0;
    let mut pos = *posp;

    curwin().w_onebuf_opt.wo_list.set(false);
    if pos.coladd != 0 {
        getvvcol(&curwin(), &mut pos, None, Some(&mut vcol), None, 0);
    } else {
        getvcol(&curwin(), &mut pos, None, Some(&mut vcol), None, 0);
    }
    *posp = pos;
    curwin().w_onebuf_opt.wo_list.set(list_save);
    vcol
}

pub fn getvvcol(
    wp: &WinInfoInfo,
    pos: &mut CursorPos,
    mut start: Option<&mut Colnr>,
    mut cursor: Option<&mut Colnr>,
    mut end: Option<&mut Colnr>,
    flags: i32,
) {
    let mut col: Colnr = 0;

    if virtual_active(wp) {
        getvcol(wp, pos, Some(&mut col), None, None, flags);
        let mut coladd = pos.coladd;
        let mut endadd: Colnr = 0;

        wp.with_buf(|buf| {
            crate::memline::with_ml_buf_line(buf, pos.lnum, |line_slice| {
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
            });
        });
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
            wp,
            pos,
            (start.is_some()).then(|| &mut *start.unwrap()),
            (cursor.is_some()).then(|| &mut *cursor.unwrap()),
            (end.is_some()).then(|| &mut *end.unwrap()),
            flags,
        );
    }
}

pub fn getvcols(
    wp: &WinInfoInfo,
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

pub fn win_may_fill(wp: &WinInfoInfo) -> bool {
    (wp.w_onebuf_opt.wo_diff.get() && diffopt_filler())
        || wp.with_buf(|buf| buf.b_marktree.borrow().meta_root[K_MTMETA_LINES] != 0)
}

pub fn win_get_fill(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    decor_virt_lines(wp, lnum - 1, lnum, None, None, true) + diff_check_fill(wp, lnum)
}

pub fn plines_win(wp: &WinInfoInfo, lnum: Linenr, limit_winheight: bool) -> i32 {
    plines_win_nofill(wp, lnum, limit_winheight) + win_get_fill(wp, lnum)
}

pub fn plines_win_nofill(wp: &WinInfoInfo, lnum: Linenr, limit_winheight: bool) -> i32 {
    if decor_conceal_line(wp, lnum - 1, false) {
        return 0;
    }
    if !wp.w_onebuf_opt.wo_wrap.get() || wp.w_view_width.get() == 0 || line_folded(wp, lnum) {
        return 1;
    }
    let lines = plines_win_nofold(wp, lnum);
    if limit_winheight && lines > wp.w_view_height.get() {
        wp.w_view_height.get()
    } else {
        lines
    }
}

pub fn plines_win_nofold(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    let mut spans = ConcealSpans::new();
    with_charsize_arg_extconceal(wp, lnum, &mut spans, |csarg, cstype| {
        if csarg.line.to_bytes().is_empty() && csarg.virt_row < 0 {
            return 1;
        }

        let mut col = if csarg.can_use_fast_loop(cstype) {
            i64::from(linesize_fast(csarg, 0, MAXCOL))
        } else {
            i64::from(linesize_regular(csarg, 0, MAXCOL))
        };

        if csarg.has_lcs_eol {
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
        width += win_col_off2(wp);
        let lines = (col + (i64::from(width) - 1)) / i64::from(width) + 1;
        if lines > 0 && lines <= i64::from(i32::MAX) {
            lines as i32
        } else {
            i32::MAX
        }
    })
}

pub fn plines_win_col(wp: &WinInfoInfo, lnum: Linenr, column: i64) -> i32 {
    let mut lines = win_get_fill(wp, lnum);
    if !wp.w_onebuf_opt.wo_wrap.get() || wp.w_view_width.get() == 0 {
        return lines + 1;
    }

    let mut spans = ConcealSpans::new();
    with_charsize_arg_extconceal(wp, lnum, &mut spans, |csarg, cstype| {
        let line_cstr = csarg.line.clone();
        let line_bytes = line_cstr.to_bytes();

        let mut vcol: Colnr = 0;
        let mut raw_vcol: Colnr = 0;
        let mut ci = utf_slice2_str_char_info(line_bytes);
        let limit_col = column as usize;
        if csarg.can_use_fast_loop(cstype) {
            let use_tabstop = csarg.use_tabstop;
            let has_vts = csarg.has_vts;
            let b_p_ts = csarg.b_p_ts;
            while !ci.ptr.is_empty() && ci.offset_from_bytes(line_bytes) < limit_col {
                let cur_off = ci.offset_from_bytes(line_bytes);
                let max_adv = limit_col.saturating_sub(cur_off);
                let ascii_run = printable_ascii_run_len(ci.ptr, max_adv);
                if ascii_run > 0 {
                    vcol += ascii_run as Colnr;
                    ci = utf_slice2_str_char_info(&ci.ptr[ascii_run..]);
                    continue;
                }
                let cli = utf_cluster_info(ci);
                vcol += charsize_fast_impl(
                    wp,
                    use_tabstop,
                    has_vts,
                    b_p_ts,
                    vcol,
                    ci.chr.value,
                    cli.cells,
                )
                .width;
                ci = cli.next;
            }
            raw_vcol = vcol;
        } else {
            while !ci.ptr.is_empty() && ci.offset_from_bytes(line_bytes) < limit_col {
                let cli = utf_cluster_info(ci);
                let cs = charsize_regular(csarg, ci.cstr(), raw_vcol, ci.chr.value, cli.cells);
                let hidden = cs.hidden;
                vcol += cs.width - hidden;
                raw_vcol += cs.width;
                ci = cli.next;
            }
        }

        let mut col = vcol;
        if ci.chr.value == TAB
            && (g_st().state_global.get() & MODE_NORMAL) != 0
            && csarg.use_tabstop
        {
            let cs = win_charsize(cstype, raw_vcol, ci.cstr(), ci.chr.value, csarg, 1);
            let hidden = cs.hidden;
            col += (cs.width - hidden - 1).max(0);
        }

        let width = wp.w_view_width.get() - win_col_off(wp);
        if width <= 0 {
            return 9999;
        }

        lines += 1;
        if col > width {
            lines += (col - width) / (width + win_col_off2(wp)) + 1;
        }
        lines
    })
}

pub fn plines_win_full(
    wp: &WinInfoInfo,
    mut lnum: Linenr,
    nextp: Option<&mut Linenr>,
    mut foldedp: Option<&mut bool>,
    cache: bool,
    limit_winheight: bool,
) -> i32 {
    let folded = has_folding_win(wp, lnum, Some(&mut lnum), nextp, cache, None);
    if let Some(ref mut p) = foldedp {
        **p = folded;
    }

    let filler_lines = if lnum == wp.w_topline.get() {
        wp.w_topfill.get()
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

pub fn plines_m_win(wp: &WinInfoInfo, mut first: Linenr, last: Linenr, max: i32) -> i32 {
    let mut count = 0;
    while first <= last && count < max {
        let mut next = first;
        count += plines_win_full(wp, first, Some(&mut next), None, false, false);
        first = next + 1;
    }
    if first == wp.with_buf(|buf| buf.b_ml.ml_line_count.get()) + 1 {
        count += win_get_fill(wp, first);
    }
    max.min(count)
}

pub fn plines_m_win_fill(wp: &WinInfoInfo, first: Linenr, last: Linenr) -> i32 {
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
    wp: &WinInfoInfo,
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
        cur_folded = has_folding(wp, lnum, Some(&mut lnum), Some(&mut lnum_next));
        height_cur_nofill = i64::from(plines_win_nofill(wp, lnum, false));
        height_sum_nofill += height_cur_nofill;
        let start_disp = if !vcol_is_disp && !cur_folded {
            extconceal_vcol_map(wp, lnum, start_vcol, true, false)
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
        cur_folded = has_folding(wp, lnum, Some(&mut lnum), Some(&mut lnum_next));
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
        let end_disp = if !vcol_is_disp && !cur_folded {
            extconceal_vcol_map(wp, cur_lnum, vcol_end, true, false)
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
            linetabsize_disp_eol(wp, cur_lnum)
        } else {
            linetabsize_eol(wp, cur_lnum)
        });
        vcol_end = (if use_vcol { vcol_end } else { i64::MAX }).min(linesize);
    }

    let overflow = height_sum_nofill + height_sum_fill - max;
    if overflow > 0 && width2 > 0 {
        let mut end_disp = if vcol_is_disp {
            vcol_end
        } else {
            extconceal_vcol_map(wp, cur_lnum, vcol_end, true, false)
        };
        if end_disp > i64::from(width2) {
            end_disp -= (end_disp - i64::from(width1)) % i64::from(width2)
                + (overflow - 1) * i64::from(width2);
            vcol_end = if vcol_is_disp {
                end_disp
            } else {
                extconceal_vcol_map(wp, cur_lnum, end_disp, false, false)
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

pub fn win_max_displaywidth(wp: &WinInfoInfo, first: Linenr, last: Linenr, max: i32) -> i32 {
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
