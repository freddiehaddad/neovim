//! Buffer decorations, virtual text/lines, and sign/highlight range state (`src/nvim/decoration.c`).

use crate::api::extmark::{ns_in_win, virt_text_to_array};
use crate::api::private::helpers::{
    arena_array, array_obj, boolean_obj, cstr_as_obj, cstr_to_arena_obj, integer_obj, put_c,
};
use crate::api::private::validate::{Dict, Object};
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::buffer::{FileBuf, buf_meta_total};
use crate::change::{TAB, changed_lines_invalidate_buf};
use crate::cursor::{Colnr, Lpos};
use crate::decoration_provider::decor_providers_invoke_conceal_line;
use crate::drawscreen::{conceal_cursor_line, redraw_buf_line_later, redraw_buf_range_later};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState, extmark_set};
use crate::fold::{has_any_folding, has_folding};
use crate::globals::*;
use crate::grid::{Schar, schar_from_char, schar_get, schar_get_first_codepoint, schar_high};
use crate::highlight::{hl_add_url, hl_combine_attr};
use crate::highlight_group::{syn_id2attr, syn_id2name};
use crate::indent::tabstop_padding;
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DecorHighlightInline, DecorInlineData, K_MTMETA_CONCEAL_LINES,
    K_MTMETA_COUNT, K_MTMETA_LINES, K_MTMETA_SIGN_TEXT, MT_FLAG_DECOR_CONCEAL_LINES,
    MT_FLAG_DECOR_EXT, MT_FLAG_DECOR_HL, MT_FLAG_DECOR_SIGNTEXT, MTKey, MTPair, MarkTreeIter,
    marktree_get_altpos, marktree_itr_current, marktree_itr_get, marktree_itr_get_filter,
    marktree_itr_get_overlap, marktree_itr_next, marktree_itr_next_filter,
    marktree_itr_step_out_filter, marktree_itr_step_overlap, mt_decor, mt_decor_any, mt_decor_sign,
    mt_end, mt_invalid,
};
use crate::mbyte::{utf_ptr2cells, utfc_ptr2len};
use crate::memory::Arena;
use crate::r#move::{changed_window_setting, win_col_off};
use crate::msgpack_rpc::packer::Integer;
use crate::regexp::MAXCOL;
use crate::sign::{buf_has_signs, describe_sign_text};
use crate::window::WinInfoInfo;
use std::cmp::Ordering;
use std::ffi::c_char;
use std::ptr;

pub type NS = crate::msgpack_rpc::packer::ObjHandle;
pub type HLGroupID = Integer;
pub type DecorPriority = u16;
pub type DecorPriorityInternal = u32;
pub type VirtTextPos = i32;
pub type VirtLineOverflow = i32;

pub(crate) const DECOR_ID_INVALID: u32 = u32::MAX;
pub(crate) const K_HL_MODE_UNKNOWN: u8 = 0;
pub(crate) const K_VPOS_END_OF_LINE: VirtTextPos = 0;
pub(crate) const K_VPOS_END_OF_LINE_RIGHT_ALIGN: VirtTextPos = 1;
pub(crate) const K_VPOS_INLINE: VirtTextPos = 2;
pub(crate) const K_VPOS_OVERLAY: VirtTextPos = 3;
pub(crate) const K_VPOS_RIGHT_ALIGN: VirtTextPos = 4;
pub(crate) const K_VPOS_WIN_COL: VirtTextPos = 5;

pub(crate) const K_VLLEFTCOL: i32 = 1;

pub(crate) const K_VLOVERFLOW_TRUNC: VirtLineOverflow = 0;
pub(crate) const K_VLOVERFLOW_SCROLL: VirtLineOverflow = 1;
pub(crate) const K_VLOVERFLOW_WRAP: VirtLineOverflow = 2;
pub(crate) const K_VLOVERFLOW_AUTO: VirtLineOverflow = 3;

pub(crate) const K_SHIS_SIGN: u16 = 1;
pub(crate) const K_SHHL_EOL: u16 = 2;
pub(crate) const K_SHUIWATCHED: u16 = 4;
pub(crate) const K_SHUIWATCHED_OVERLAY: u16 = 8;
pub(crate) const K_SHSPELL_ON: u16 = 16;
pub(crate) const K_SHSPELL_OFF: u16 = 32;
pub(crate) const K_SHCONCEAL: u16 = 64;
pub(crate) const K_SHCONCEAL_LINES: u16 = 128;
pub(crate) const K_SHCONCEAL_OFF: u16 = 256;

pub(crate) const K_VTIS_LINES: u8 = 1;
pub(crate) const K_VTHIDE: u8 = 2;
pub(crate) const K_VTLINES_ABOVE: u8 = 4;
pub(crate) const K_VTREPEAT_LINEBREAK: u8 = 8;

#[repr(C)]
#[derive(Clone)]
pub struct DecorSignHighlight {
    pub flags: u16,
    pub priority: DecorPriority,
    pub hl_id: i32,
    pub text: [u32; 2],
    pub sign_name: Option<std::ffi::CString>,
    pub sign_add_id: i32,
    pub number_hl_id: i32,
    pub line_hl_id: i32,
    pub cursorline_hl_id: i32,
    pub next: u32,
    pub url: Option<std::ffi::CString>,
}

pub const DECOR_SIGN_HIGHLIGHT_INIT: DecorSignHighlight = DecorSignHighlight {
    flags: 0,
    priority: crate::marktree::DECOR_PRIORITY_BASE,
    hl_id: 0,
    text: [0, 0],
    sign_name: None,
    sign_add_id: 0,
    number_hl_id: 0,
    line_hl_id: 0,
    cursorline_hl_id: 0,
    next: DECOR_ID_INVALID,
    url: None,
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorRangeUI {
    pub ns_id: u32,
    pub mark_id: u32,
    pub pos: VirtTextPos,
}

#[derive(Clone)]
pub enum DecorRangeData {
    Sh(DecorSignHighlight),
    Vt(*mut DecorVirtText),
    VirtLines(*mut DecorVirtText),
    Ui(DecorRangeUI),
}

impl DecorRangeData {
    #[inline]
    pub const fn is_sh(&self) -> bool {
        matches!(self, Self::Sh(_))
    }

    #[inline]
    pub const fn is_vt(&self) -> bool {
        matches!(self, Self::Vt(_))
    }

    #[inline]
    pub const fn is_ui(&self) -> bool {
        matches!(self, Self::Ui(_))
    }

    #[inline]
    pub const fn sh(&self) -> &DecorSignHighlight {
        match self {
            Self::Sh(sh) => sh,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn vt(&self) -> *mut DecorVirtText {
        match self {
            Self::Vt(vt) | Self::VirtLines(vt) => *vt,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn ui(&self) -> DecorRangeUI {
        match self {
            Self::Ui(ui) => *ui,
            _ => unreachable!(),
        }
    }
}

pub struct DecorRange {
    pub start_row: i32,
    pub start_col: i32,
    pub end_row: i32,
    pub end_col: i32,
    pub ordering: i32,
    pub priority_internal: u32,
    pub owned: bool,
    pub data: DecorRangeData,
    pub attr_id: i32,
    pub draw_col: i32,
}

impl Drop for DecorRange {
    fn drop(&mut self) {
        if self.owned
            && let DecorRangeData::Vt(vt) | DecorRangeData::VirtLines(vt) = self.data
            && !vt.is_null()
        {
            drop(unsafe { Box::from_raw(vt) });
        }
    }
}

pub enum DecorRangeSlot {
    Occupied(DecorRange),
    Free(i32),
}

impl DecorRangeSlot {
    #[inline]
    pub const fn range(&self) -> &DecorRange {
        match self {
            Self::Occupied(r) => r,
            Self::Free(_) => unreachable!(),
        }
    }

    #[inline]
    pub fn range_mut(&mut self) -> &mut DecorRange {
        match self {
            Self::Occupied(r) => r,
            Self::Free(_) => unreachable!(),
        }
    }

    #[inline]
    pub const fn next_free_i(&self) -> i32 {
        match self {
            Self::Free(i) => *i,
            Self::Occupied(_) => unreachable!(),
        }
    }
}

pub struct DecorState {
    pub itr: MarkTreeIter,
    pub slots: Vec<DecorRangeSlot>,
    pub ranges_i: Vec<i32>,
    pub current_end: i32,
    pub future_begin: i32,
    pub free_slot_i: i32,
    pub new_range_ordering: i32,
    pub win: *mut WinInfoInfo,
    pub top_row: i32,
    pub row: i32,
    pub col_last: i32,
    pub current: i32,
    pub current_hl_eol: i32,
    pub eol_col: i32,
    pub conceal: i32,
    pub conceal_char: u32,
    pub conceal_attr: i32,
    pub spell: i32,
    pub running_decor_provider: bool,
    pub itr_valid: bool,
}

impl DecorState {
    pub const fn new() -> Self {
        Self {
            itr: MarkTreeIter::new(),
            slots: Vec::new(),
            ranges_i: Vec::new(),
            current_end: 0,
            future_begin: 0,
            free_slot_i: 0,
            new_range_ordering: 0,
            win: ptr::null_mut(),
            top_row: 0,
            row: 0,
            col_last: 0,
            current: 0,
            current_hl_eol: 0,
            eol_col: 0,
            conceal: 0,
            conceal_char: 0,
            conceal_attr: 0,
            spell: 0,
            running_decor_provider: false,
            itr_valid: false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorInline {
    pub data: DecorInlineData,
}

impl DecorInline {
    #[inline]
    pub const fn is_ext(&self) -> bool {
        matches!(self.data, DecorInlineData::Ext(_))
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct VirtTextChunk {
    pub text: Option<std::ffi::CString>,
    pub hl_id: i32,
}

pub type VirtText = Vec<VirtTextChunk>;

#[repr(C)]
#[derive(Clone)]
pub struct VirtLine {
    pub line: VirtText,
    pub flags: i32,
    pub overflow: VirtLineOverflow,
}

pub type VirtLines = Vec<VirtLine>;

#[derive(Clone)]
pub enum DecorVirtTextData {
    VirtText(VirtText),
    VirtLines(VirtLines),
}

impl DecorVirtTextData {
    pub const fn empty() -> Self {
        Self::VirtText(Vec::new())
    }

    pub const fn empty_lines() -> Self {
        Self::VirtLines(Vec::new())
    }

    #[inline]
    pub fn virt_text(&self) -> &VirtText {
        static EMPTY: VirtText = Vec::new();
        match self {
            Self::VirtText(vt) => vt,
            Self::VirtLines(_) => &EMPTY,
        }
    }

    #[inline]
    pub fn virt_text_mut(&mut self) -> &mut VirtText {
        if !matches!(self, Self::VirtText(_)) {
            *self = Self::VirtText(Vec::new());
        }
        match self {
            Self::VirtText(vt) => vt,
            Self::VirtLines(_) => unreachable!(),
        }
    }

    #[inline]
    pub fn virt_lines(&self) -> &VirtLines {
        static EMPTY: VirtLines = Vec::new();
        match self {
            Self::VirtLines(vl) => vl,
            Self::VirtText(_) => &EMPTY,
        }
    }

    #[inline]
    pub fn virt_lines_mut(&mut self) -> &mut VirtLines {
        if !matches!(self, Self::VirtLines(_)) {
            *self = Self::VirtLines(Vec::new());
        }
        match self {
            Self::VirtLines(vl) => vl,
            Self::VirtText(_) => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct DecorVirtText {
    pub flags: u8,
    pub hl_mode: u8,
    pub priority: u16,
    pub width: i32,
    pub col: i32,
    pub pos: VirtTextPos,
    pub data: DecorVirtTextData,
    pub next: Option<Box<DecorVirtText>>,
}

impl DecorVirtText {
    #[inline]
    pub fn next_ptr(&mut self) -> *mut DecorVirtText {
        self.next
            .as_deref_mut()
            .map_or(ptr::null_mut(), |n| &raw mut *n)
    }
}

pub struct VirtTextMutIter(*mut DecorVirtText);

impl Iterator for VirtTextMutIter {
    type Item = &'static mut DecorVirtText;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0;
        if cur.is_null() {
            None
        } else {
            unsafe {
                self.0 = (*cur).next_ptr();
                Some(&mut *cur)
            }
        }
    }
}

pub struct VirtTextPtrIter(*mut DecorVirtText);

impl Iterator for VirtTextPtrIter {
    type Item = &'static mut DecorVirtText;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0;
        if cur.is_null() {
            None
        } else {
            unsafe {
                self.0 = (*cur).next_ptr();
                Some(&mut *cur)
            }
        }
    }
}

#[inline]
pub fn virt_text_iter_mut(start: *mut DecorVirtText) -> VirtTextMutIter {
    VirtTextMutIter(start)
}

#[inline]
pub fn virt_text_iter_ptr(start: *mut DecorVirtText) -> VirtTextPtrIter {
    VirtTextPtrIter(start)
}

use crate::extmark::{
    K_EXTMARK_HIGHLIGHT, K_EXTMARK_NONE, K_EXTMARK_SIGN, K_EXTMARK_VIRT_LINES, K_EXTMARK_VIRT_TEXT,
};

pub(crate) const SIGN_WIDTH: usize = 2;
const SIGN_SHOW_MAX: i32 = 9;
pub(crate) const SCL_NUM: i32 = -2;

pub(crate) const K_MTFILTER_SELECT: u32 = u32::MAX;

static CONCEAL_FILTER: [u32; K_MTMETA_COUNT] = [0, 0, 0, 0, K_MTFILTER_SELECT];
static SIGN_FILTER: [u32; K_MTMETA_COUNT] = [0, 0, K_MTFILTER_SELECT, K_MTFILTER_SELECT, 0];
static SIGNTEXT_FILTER: [u32; K_MTMETA_COUNT] = [0, 0, 0, K_MTFILTER_SELECT, 0];
static LINES_FILTER: [u32; K_MTMETA_COUNT] = [0, K_MTFILTER_SELECT, 0, 0, 0];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SignTextAttrs {
    pub text: [Schar; SIGN_WIDTH],
    pub hl_id: i32,
}

#[derive(Clone, Copy)]
pub struct SignItem<'a> {
    pub sh: &'a DecorSignHighlight,
    pub id: u32,
}

struct DecorationModuleState {
    decor_freelist: u32,
    to_free_virt: Vec<DecorVirtText>,
    to_free_sh: u32,
    sign_add_id: i32,
}

impl DecorationModuleState {
    const fn new() -> Self {
        Self {
            decor_freelist: u32::MAX,
            to_free_virt: Vec::new(),
            to_free_sh: u32::MAX,
            sign_add_id: 0,
        }
    }
}

static DECOR_MOD_STATE: GlobalState<DecorationModuleState> =
    GlobalState::new(DecorationModuleState::new());

#[inline]
fn decor_st() -> &'static mut DecorationModuleState {
    unsafe { &mut *DECOR_MOD_STATE.as_ptr() }
}

#[inline]
const fn mt_conceal_lines(key: MTKey) -> bool {
    (key.flags & MT_FLAG_DECOR_CONCEAL_LINES) != 0
}

#[inline]
const fn mt_decor_virt(mark: MTKey) -> *mut DecorVirtText {
    if (mark.flags & MT_FLAG_DECOR_EXT) != 0 {
        mark.decor_data.ext().vt
    } else {
        ptr::null_mut()
    }
}

pub fn bufhl_add_hl_pos_offset(
    buf: &mut FileBuf,
    src_id: i32,
    hl_id: i32,
    pos_start: Lpos,
    pos_end: Lpos,
    offset: Colnr,
) {
    let mut hl_start: Colnr = 0;
    let mut hl_end: Colnr = 0;
    let decor = DecorInline {
        data: DecorInlineData::Hl(DecorHighlightInline {
            hl_id,
            ..DECOR_HIGHLIGHT_INLINE_INIT
        }),
    };

    for lnum in pos_start.lnum..=pos_end.lnum {
        let mut end_off: i32 = 0;
        if pos_start.lnum < lnum && lnum < pos_end.lnum {
            hl_start = (offset - 1).max(0);
            end_off = 1;
            hl_end = 0;
        } else if lnum == pos_start.lnum && lnum < pos_end.lnum {
            hl_start = pos_start.col + offset;
            end_off = 1;
            hl_end = 0;
        } else if pos_start.lnum < lnum && lnum == pos_end.lnum {
            hl_start = (offset - 1).max(0);
            hl_end = pos_end.col + offset;
        } else if pos_start.lnum == lnum && pos_end.lnum == lnum {
            hl_start = pos_start.col + offset;
            hl_end = pos_end.col + offset;
        }

        extmark_set(
            buf,
            src_id as u32,
            None,
            lnum - 1,
            hl_start,
            lnum - 1 + end_off,
            hl_end,
            decor,
            MT_FLAG_DECOR_HL,
            true,
            false,
            true,
            false,
        );
    }
}

pub fn decor_redraw(buf: &mut FileBuf, row1: i32, row2: i32, col1: i32, decor: DecorInline) {
    if decor.is_ext() {
        for vt in virt_text_iter_mut(decor.data.ext().vt) {
            let below = (vt.flags & K_VTIS_LINES) != 0 && (vt.flags & K_VTLINES_ABOVE) == 0;
            let vt_lnum = row1 + 1 + i32::from(below);
            redraw_buf_line_later(buf, vt_lnum, true);
            if (vt.flags & K_VTIS_LINES) != 0 || vt.pos == K_VPOS_INLINE {
                let vt_col = if (vt.flags & K_VTIS_LINES) != 0 {
                    0
                } else {
                    col1
                };
                changed_lines_invalidate_buf(buf, vt_lnum, vt_col, vt_lnum + 1, 0);
            }
        }

        let mut idx = decor.data.ext().sh_idx;
        while idx != DECOR_ID_INVALID {
            let sh = &g_st().decor_items.as_slice()[idx as usize];
            let next = sh.next;
            decor_redraw_sh(buf, row1, row2, sh.clone());
            idx = next;
        }
    } else {
        decor_redraw_sh(buf, row1, row2, decor_sh_from_inline(decor.data.hl()));
    }
}

pub fn decor_redraw_sh(buf: &mut FileBuf, row1: i32, row2: i32, sh: DecorSignHighlight) {
    if (sh.hl_id != 0
        || sh.url.is_some()
        || (sh.flags
            & (K_SHIS_SIGN | K_SHSPELL_ON | K_SHSPELL_OFF | K_SHCONCEAL | K_SHCONCEAL_OFF))
            != 0)
        && row2 >= row1
    {
        redraw_buf_range_later(buf, row1 + 1, row2 + 1);
    }
    if (sh.flags & K_SHCONCEAL_LINES) != 0 {
        for wp in for_all_windows_mut() {
            if wp.w_buffer == buf {
                changed_window_setting(wp);
            }
        }
    }
    if (sh.flags & K_SHUIWATCHED) != 0 {
        redraw_buf_line_later(buf, row1 + 1, false);
    }
}

pub fn decor_put_sh(item: DecorSignHighlight) -> u32 {
    if decor_st().decor_freelist != u32::MAX {
        let pos = decor_st().decor_freelist;
        decor_st().decor_freelist =
            g_st().decor_items.as_slice()[decor_st().decor_freelist as usize].next;
        g_st().decor_items.as_mut_slice()[pos as usize] = item;
        pos
    } else {
        let pos = g_st().decor_items.len() as u32;
        g_st().decor_items.push(item);
        pos
    }
}

pub fn decor_put_vt(mut vt: DecorVirtText, next: *mut DecorVirtText) -> &'static mut DecorVirtText {
    vt.next = (!next.is_null()).then(|| unsafe { Box::from_raw(next) });
    Box::leak(Box::new(vt))
}

pub fn decor_sh_from_inline(item: DecorHighlightInline) -> DecorSignHighlight {
    assert!((item.flags & K_SHIS_SIGN) == 0);
    DecorSignHighlight {
        flags: item.flags,
        priority: item.priority,
        hl_id: item.hl_id,
        text: [item.conceal_char, 0],
        sign_name: None,
        sign_add_id: 0,
        number_hl_id: 0,
        line_hl_id: 0,
        cursorline_hl_id: 0,
        next: DECOR_ID_INVALID,
        url: None,
    }
}

pub fn buf_put_decor(buf: &mut FileBuf, decor: DecorInline, row: i32, mut row2: i32) {
    if decor.is_ext() && row < buf.b_ml.ml_line_count {
        let mut idx = decor.data.ext().sh_idx;
        row2 = row2.min(buf.b_ml.ml_line_count - 1);
        while idx != DECOR_ID_INVALID {
            let sh = &mut (&mut g_st().decor_items)[idx as usize];
            let next = sh.next;
            buf_put_decor_sh(buf, sh, row, row2);
            idx = next;
        }
    }
}

fn may_force_numberwidth_recompute(buf: &mut FileBuf, unplace: bool) {
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_mut(if ptr::eq(tp, g_st().curtab) {
            g_st().firstwin
        } else {
            tp.tp_firstwin
        }) {
            if wp.w_buffer == buf
                && wp.w_minscwidth == SCL_NUM
                && (wp.w_onebuf_opt.wo_nu != 0 || wp.w_onebuf_opt.wo_rnu != 0)
                && (unplace || wp.w_nrwidth_width < 2)
            {
                wp.w_nrwidth_line_count = 0;
            }
        }
    }
}

pub fn buf_put_decor_sh(buf: &mut FileBuf, sh: &mut DecorSignHighlight, row1: i32, row2: i32) {
    if (sh.flags & K_SHIS_SIGN) != 0 {
        sh.sign_add_id = decor_st().sign_add_id;
        decor_st().sign_add_id += 1;
        if sh.text[0] != 0 {
            buf_signcols_count_range(&mut *buf, row1, row2, 1, K_FALSE);
            may_force_numberwidth_recompute(buf, false);
        }
    }
}

pub fn buf_decor_remove(
    buf: &mut FileBuf,
    row1: i32,
    mut row2: i32,
    col1: i32,
    decor: DecorInline,
    free: bool,
) {
    decor_redraw(&mut *buf, row1, row2, col1, decor);
    if decor.is_ext() && row1 < buf.b_ml.ml_line_count {
        let mut idx = decor.data.ext().sh_idx;
        row2 = row2.min(buf.b_ml.ml_line_count - 1);
        while idx != DECOR_ID_INVALID {
            let sh = &(&g_st().decor_items)[idx as usize];
            let next = sh.next;
            buf_remove_decor_sh(buf, row1, row2, sh);
            idx = next;
        }
    }
    if free {
        decor_free(decor);
    }
}

pub fn buf_remove_decor_sh(buf: &mut FileBuf, row1: i32, row2: i32, sh: &DecorSignHighlight) {
    if (sh.flags & K_SHIS_SIGN) != 0 && sh.text[0] != 0 {
        if buf_meta_total(&*buf, K_MTMETA_SIGN_TEXT) != 0 {
            buf_signcols_count_range(&mut *buf, row1, row2, -1, K_FALSE);
        } else {
            may_force_numberwidth_recompute(&mut *buf, true);
            buf.b_signcols.count[0] = 0;
            buf.b_signcols.max = 0;
        }
    }
}

pub fn decor_free(decor: DecorInline) {
    if !decor.is_ext() {
        return;
    }
    let ext = decor.data.ext();
    let vt = ext.vt;
    let mut idx = ext.sh_idx;

    if g_st().decor_state.running_decor_provider {
        if !vt.is_null() {
            decor_st().to_free_virt.push(*unsafe { Box::from_raw(vt) });
        }
        while idx != DECOR_ID_INVALID {
            let sh = &mut (&mut g_st().decor_items)[idx as usize];
            if sh.next == DECOR_ID_INVALID {
                sh.next = decor_st().to_free_sh;
                decor_st().to_free_sh = ext.sh_idx;
                break;
            }
            idx = sh.next;
        }
    } else {
        if !vt.is_null() {
            drop(unsafe { Box::from_raw(vt) });
        }
        decor_free_sh(idx);
    }
}

fn decor_free_sh(first_idx: u32) {
    let mut idx = first_idx;
    while idx != DECOR_ID_INVALID {
        let sh = &mut (&mut g_st().decor_items)[idx as usize];
        sh.sign_name = None;
        sh.flags = 0;
        sh.url = None;
        if sh.next == DECOR_ID_INVALID {
            sh.next = decor_st().decor_freelist;
            decor_st().decor_freelist = first_idx;
            break;
        }
        idx = sh.next;
    }
}

pub fn decor_state_invalidate(buf: &mut FileBuf) {
    if !g_st().decor_state.win.is_null()
        && std::ptr::eq(unsafe { (*g_st().decor_state.win).w_buffer }, buf)
    {
        g_st().decor_state.itr_valid = false;
    }
}

pub fn decor_check_to_be_deleted() {
    assert!(!g_st().decor_state.running_decor_provider);
    decor_st().to_free_virt.clear();
    decor_free_sh(decor_st().to_free_sh);
    decor_st().to_free_sh = DECOR_ID_INVALID;
    g_st().decor_state.win = ptr::null_mut();
}

pub fn decor_check_invalid_glyphs() {
    for it in g_st().decor_items.iter_mut() {
        let width = if (it.flags & K_SHIS_SIGN) != 0 {
            SIGN_WIDTH
        } else if (it.flags & K_SHCONCEAL) != 0 {
            1
        } else {
            0
        };
        for j in 0..width {
            if schar_high(it.text[j]) {
                it.text[j] = schar_from_char(schar_get_first_codepoint(it.text[j]));
            }
        }
    }
}

pub fn next_virt_text_chunk(vt: &[VirtTextChunk], pos: &mut usize, attr: &mut i32) -> *mut c_char {
    let mut text: *mut c_char = ptr::null_mut();
    while text.is_null() && *pos < vt.len() {
        let chunk = &vt[*pos];
        text = chunk
            .text
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        let hl_id = chunk.hl_id;
        if hl_id >= 0 {
            *attr = (*attr).max(0);
            if hl_id > 0 {
                *attr = hl_combine_attr(*attr, syn_id2attr(hl_id));
            }
        }
        *pos += 1;
    }
    text
}

pub fn decor_find_virttext(buf: &mut FileBuf, row: i32, ns_id: u64) -> *mut DecorVirtText {
    unsafe {
        let mut itr = MarkTreeIter::new();
        marktree_itr_get(&buf.b_marktree, row, 0, &mut itr);
        loop {
            let mark = marktree_itr_current(&buf.b_marktree, &itr);
            if mark.pos.row < 0 || mark.pos.row > row {
                break;
            } else if !mt_invalid(mark) {
                let mut decor = mt_decor_virt(mark);
                while !decor.is_null() && ((*decor).flags & K_VTIS_LINES) != 0 {
                    decor = (*decor).next_ptr();
                }
                if (ns_id == 0 || ns_id == u64::from(mark.ns)) && !decor.is_null() {
                    return decor;
                }
            }
            marktree_itr_next(&buf.b_marktree, &mut itr);
        }
        ptr::null_mut()
    }
}

pub fn decor_redraw_reset(wp: &mut WinInfoInfo, state: &mut DecorState) -> bool {
    state.row = -1;
    state.win = wp;

    state.slots.clear();
    state.ranges_i.clear();
    state.free_slot_i = -1;
    state.current_end = 0;
    state.future_begin = 0;
    state.new_range_ordering = 0;

    wp.buf_mut().b_marktree.n_keys != 0
}

pub fn decor_virt_pos(decor: &DecorRange) -> bool {
    decor.data.is_vt() || decor.data.is_ui()
}

pub fn decor_virt_pos_kind(decor: &DecorRange) -> VirtTextPos {
    match decor.data {
        DecorRangeData::Vt(vt) => unsafe { (*vt).pos },
        DecorRangeData::Ui(ui) => ui.pos,
        _ => K_VPOS_END_OF_LINE,
    }
}

pub fn decor_redraw_start(wp: &mut WinInfoInfo, top_row: i32, state: &mut DecorState) -> bool {
    let buf = wp.w_buffer;
    state.top_row = top_row;
    state.itr_valid = true;

    if !marktree_itr_get_overlap(unsafe { &(*buf).b_marktree }, top_row, 0, &mut state.itr) {
        return false;
    }
    let mut pair: MTPair = MTPair::new();

    while marktree_itr_step_overlap(unsafe { &(*buf).b_marktree }, &mut state.itr, &mut pair) {
        let m = pair.start;
        if mt_invalid(m) || !mt_decor_any(m) {
            continue;
        }

        decor_range_add_from_inline(
            state,
            pair.start.pos.row,
            pair.start.pos.col,
            pair.end_pos.row,
            pair.end_pos.col,
            mt_decor(m),
            false,
            m.ns,
            m.id,
        );
    }

    true
}

fn decor_state_pack(state: &mut DecorState) {
    let mut count = state.ranges_i.len() as i32;
    let cur_end = state.current_end;
    let mut fut_beg = state.future_begin;

    if fut_beg == count {
        fut_beg = cur_end;
        count = cur_end;
    } else if fut_beg != cur_end {
        state
            .ranges_i
            .copy_within(fut_beg as usize..count as usize, cur_end as usize);
        count = cur_end + (count - fut_beg);
        fut_beg = cur_end;
    }

    state.ranges_i.truncate(count as usize);
    state.future_begin = fut_beg;
}

pub fn decor_redraw_line(wp: &mut WinInfoInfo, row: i32, state: &mut DecorState) {
    decor_state_pack(state);

    if state.row == -1 {
        decor_redraw_start(wp, row, state);
    } else if !state.itr_valid {
        marktree_itr_get(&wp.buf_mut().b_marktree, row, 0, &mut state.itr);
        state.itr_valid = true;
    }

    state.row = row;
    state.col_last = -1;
    state.eol_col = -1;
}

pub fn decor_has_more_decorations(state: &mut DecorState, row: i32) -> bool {
    unsafe {
        if state.current_end != 0 || state.future_begin != state.ranges_i.len() as i32 {
            return true;
        }

        let k = marktree_itr_current(&(*(*state.win).w_buffer).b_marktree, &state.itr);
        k.pos.row >= 0 && k.pos.row <= row
    }
}

fn decor_range_add_from_inline(
    state: &mut DecorState,
    start_row: i32,
    start_col: i32,
    end_row: i32,
    end_col: i32,
    decor: DecorInline,
    owned: bool,
    ns: u32,
    mark_id: u32,
) {
    if decor.is_ext() {
        for vt in virt_text_iter_ptr(decor.data.ext().vt) {
            decor_range_add_virt(state, start_row, start_col, end_row, end_col, vt, owned);
        }
        let mut idx = decor.data.ext().sh_idx;
        while idx != DECOR_ID_INVALID {
            let sh = &(&g_st().decor_items)[idx as usize];
            let next = sh.next;
            decor_range_add_sh(
                state, start_row, start_col, end_row, end_col, sh, owned, ns, mark_id, 0,
            );
            idx = next;
        }
    } else {
        let sh = decor_sh_from_inline(decor.data.hl());
        decor_range_add_sh(
            state, start_row, start_col, end_row, end_col, &sh, owned, ns, mark_id, 0,
        );
    }
}

fn decor_range_insert(state: &mut DecorState, mut range: DecorRange) {
    let row = range.start_row;
    let col = range.start_col;
    range.ordering = state.new_range_ordering;
    state.new_range_ordering += 1;

    let index: i32;
    if state.free_slot_i >= 0 {
        index = state.free_slot_i;
        let slot = &mut (&mut state.slots)[index as usize];
        state.free_slot_i = slot.next_free_i();
        *slot = DecorRangeSlot::Occupied(range);
    } else {
        index = state.slots.len() as i32;
        state.slots.push(DecorRangeSlot::Occupied(range));
    }

    let count = state.ranges_i.len() as i32;

    let mut begin = state.future_begin;
    let mut end = count;
    while begin < end {
        let mid = begin + ((end - begin) >> 1);
        let mid_idx = (&state.ranges_i)[mid as usize] as usize;
        let mr = (&state.slots)[mid_idx].range();

        let mrow = mr.start_row;
        let mcol = mr.start_col;
        if mrow < row || (mrow == row && mcol <= col) {
            begin = mid + 1;
            if mrow == row && mcol == col {
                break;
            }
        } else {
            end = mid;
        }
    }

    state.ranges_i.insert(begin as usize, index);
}

pub fn decor_range_add_virt(
    state: &mut DecorState,
    start_row: i32,
    start_col: i32,
    end_row: i32,
    end_col: i32,
    vt: *mut DecorVirtText,
    owned: bool,
) {
    let is_lines = (unsafe { (*vt).flags } & K_VTIS_LINES) != 0;
    let range = DecorRange {
        start_row,
        start_col,
        end_row,
        end_col,
        ordering: 0,
        priority_internal: DecorPriorityInternal::from(unsafe { (*vt).priority }) << 16,
        owned,
        data: if is_lines {
            DecorRangeData::VirtLines(vt)
        } else {
            DecorRangeData::Vt(vt)
        },
        attr_id: 0,
        draw_col: -10,
    };
    decor_range_insert(state, range);
}

pub fn decor_range_add_sh(
    state: &mut DecorState,
    start_row: i32,
    start_col: i32,
    end_row: i32,
    end_col: i32,
    sh: &DecorSignHighlight,
    owned: bool,
    ns: u32,
    mark_id: u32,
    subpriority: DecorPriority,
) {
    if (sh.flags & K_SHIS_SIGN) != 0 {
        return;
    }

    let priority_internal =
        (DecorPriorityInternal::from(sh.priority) << 16) + DecorPriorityInternal::from(subpriority);

    if sh.hl_id != 0
        || sh.url.is_some()
        || (sh.flags & (K_SHCONCEAL_OFF | K_SHCONCEAL | K_SHSPELL_ON | K_SHSPELL_OFF)) != 0
    {
        let attr_id = if sh.hl_id != 0 {
            syn_id2attr(sh.hl_id)
        } else {
            0
        };
        decor_range_insert(
            state,
            DecorRange {
                start_row,
                start_col,
                end_row,
                end_col,
                ordering: 0,
                priority_internal,
                owned,
                data: DecorRangeData::Sh(sh.clone()),
                attr_id,
                draw_col: -10,
            },
        );
    }

    if (sh.flags & K_SHUIWATCHED) != 0 {
        decor_range_insert(
            state,
            DecorRange {
                start_row,
                start_col,
                end_row,
                end_col,
                ordering: 0,
                priority_internal,
                owned: false,
                data: DecorRangeData::Ui(DecorRangeUI {
                    ns_id: ns,
                    mark_id,
                    pos: if (sh.flags & K_SHUIWATCHED_OVERLAY) != 0 {
                        K_VPOS_OVERLAY
                    } else {
                        K_VPOS_END_OF_LINE
                    },
                }),
                attr_id: 0,
                draw_col: -10,
            },
        );
    }
}

pub fn decor_init_draw_col(win_col: i32, hidden: bool, item: &mut DecorRange) {
    let vt = if item.data.is_vt() {
        item.data.vt()
    } else {
        ptr::null_mut()
    };
    let pos = decor_virt_pos_kind(item);
    if win_col < 0 && pos != K_VPOS_INLINE {
        item.draw_col = win_col;
    } else if pos == K_VPOS_OVERLAY {
        item.draw_col = if !vt.is_null() && (unsafe { (*vt).flags } & K_VTHIDE) != 0 && hidden {
            i32::MIN
        } else {
            win_col
        };
    } else {
        item.draw_col = -1;
    }
}

pub fn decor_recheck_draw_col(win_col: i32, hidden: bool, state: &mut DecorState) {
    let end = state.current_end;

    for i in 0..end {
        let slot_idx = (&state.ranges_i)[i as usize] as usize;
        let r = (&mut state.slots)[slot_idx].range_mut();
        if r.draw_col == -3 {
            decor_init_draw_col(win_col, hidden, &mut *r);
        }
    }
}

pub fn decor_redraw_col_impl(
    wp: &mut WinInfoInfo,
    col: i32,
    win_col: i32,
    hidden: bool,
    state: &mut DecorState,
    max_col_last: i32,
) -> i32 {
    unsafe {
        let buf = wp.w_buffer;
        let buf = &mut *buf;
        let row = state.row;
        let mut col_last = max_col_last;

        loop {
            let mark = marktree_itr_current(&buf.b_marktree, &state.itr);
            if mark.pos.row < 0 || mark.pos.row > row {
                break;
            } else if mark.pos.row == row && mark.pos.col > col {
                col_last = col_last.min(mark.pos.col - 1);
                break;
            }

            if !mt_invalid(mark)
                && !mt_end(mark)
                && mt_decor_any(mark)
                && ns_in_win(mark.ns, &mut *wp)
            {
                let endpos = marktree_get_altpos(&buf.b_marktree, mark, None);
                decor_range_add_from_inline(
                    state,
                    mark.pos.row,
                    mark.pos.col,
                    endpos.row,
                    endpos.col,
                    mt_decor(mark),
                    false,
                    mark.ns,
                    mark.id,
                );
            }

            marktree_itr_next(&buf.b_marktree, &mut state.itr);
        }

        let mut count = state.ranges_i.len() as i32;
        let mut cur_end = state.current_end;
        let mut fut_beg = state.future_begin;

        while fut_beg < count {
            let index = (&state.ranges_i)[fut_beg as usize];
            let r = (&state.slots)[index as usize].range();
            if r.start_row > row || (r.start_row == row && r.start_col > col) {
                break;
            }
            let ordering = r.ordering;
            let priority = r.priority_internal;

            let mut begin: i32 = 0;
            let mut end = cur_end;
            while begin < end {
                let mid = begin + ((end - begin) >> 1);
                let mi = (&state.ranges_i)[mid as usize] as usize;
                let mr = (&state.slots)[mi].range();
                if mr.priority_internal < priority
                    || (mr.priority_internal == priority && mr.ordering < ordering)
                {
                    begin = mid + 1;
                } else {
                    end = mid;
                }
            }

            state
                .ranges_i
                .copy_within(begin as usize..cur_end as usize, (begin + 1) as usize);
            (&mut state.ranges_i)[begin as usize] = index;
            cur_end += 1;
            fut_beg += 1;
        }

        if fut_beg < count {
            let r_idx = (&state.ranges_i)[fut_beg as usize] as usize;
            let r = (&state.slots)[r_idx].range();
            if r.start_row == row {
                col_last = col_last.min(r.start_col - 1);
            }
        }

        let mut new_cur_end: i32 = 0;

        let mut attr: i32 = 0;
        let mut hl_eol_attr: i32 = 0;
        let mut conceal: i32 = 0;
        let mut conceal_char: Schar = 0;
        let mut conceal_attr: i32 = 0;
        let mut spell: TriState = K_NONE;

        for i in 0..cur_end {
            let index = (&state.ranges_i)[i as usize];
            let slot = &raw mut (&mut state.slots)[index as usize];
            let r = (*slot).range_mut();

            let keep: bool;
            if r.end_row < row || (r.end_row == row && r.end_col <= col) {
                keep = r.start_row >= row && decor_virt_pos(r);
            } else {
                keep = true;

                if r.end_row == row && r.end_col > col {
                    col_last = col_last.min(r.end_col - 1);
                }

                if r.attr_id > 0 {
                    attr = hl_combine_attr(attr, r.attr_id);
                    if r.data.is_sh() && (r.data.sh().flags & K_SHHL_EOL) != 0 {
                        hl_eol_attr = hl_combine_attr(hl_eol_attr, r.attr_id);
                    }
                }

                if r.data.is_sh() && (r.data.sh().flags & K_SHCONCEAL) != 0 {
                    conceal = 1;
                    if r.start_row == row && r.start_col == col {
                        conceal = 2;
                        conceal_char = r.data.sh().text[0];
                        col_last = col_last.min(r.start_col);
                        conceal_attr = r.attr_id;
                    }
                }

                if r.data.is_sh() && (r.data.sh().flags & K_SHCONCEAL_OFF) != 0 {
                    conceal = 0;
                }

                if r.data.is_sh() {
                    if (r.data.sh().flags & K_SHSPELL_ON) != 0 {
                        spell = K_TRUE;
                    } else if (r.data.sh().flags & K_SHSPELL_OFF) != 0 {
                        spell = K_FALSE;
                    }
                    if let Some(ref url) = r.data.sh().url {
                        attr = hl_add_url(attr, url.as_ptr());
                    }
                }
            }

            if r.start_row == row && r.start_col <= col && decor_virt_pos(r) && r.draw_col == -10 {
                decor_init_draw_col(win_col, hidden, &mut *r);
            }

            if keep {
                (&mut state.ranges_i)[new_cur_end as usize] = index;
                new_cur_end += 1;
            } else {
                let fi = &raw mut state.free_slot_i;
                *slot = DecorRangeSlot::Free(*fi);
                *fi = index;
            }
        }
        cur_end = new_cur_end;

        if fut_beg == count {
            fut_beg = cur_end;
            count = cur_end;
        }

        state.ranges_i.truncate(count as usize);
        state.future_begin = fut_beg;
        state.current_end = cur_end;
        state.col_last = col_last;

        state.current = attr;
        state.current_hl_eol = hl_eol_attr;
        state.conceal = conceal;
        state.conceal_char = conceal_char;
        state.conceal_attr = conceal_attr;
        state.spell = spell;
        attr
    }
}

pub fn decor_conceal_line(wp: &mut WinInfoInfo, row: i32, check_cursor: bool) -> bool {
    if row < 0
        || wp.w_onebuf_opt.wo_cole < 2
        || (!check_cursor
            && ptr::eq(wp, curwin_ptr())
            && row + 1 == wp.w_cursor.lnum
            && !conceal_cursor_line(&*wp))
    {
        return false;
    }

    if buf_meta_total((*wp).buf_mut(), K_MTMETA_CONCEAL_LINES) == 0 {
        return decor_providers_invoke_conceal_line(&mut *wp, row);
    }

    let mut pair: MTPair = MTPair::new();
    let mut itr = MarkTreeIter::new();
    let b = &(*wp).buf_mut().b_marktree;
    marktree_itr_get_overlap(b, row, 0, &mut itr);
    while marktree_itr_step_overlap(b, &mut itr, &mut pair) {
        if mt_conceal_lines(pair.start) && ns_in_win(pair.start.ns, &mut *wp) {
            return true;
        }
    }

    marktree_itr_step_out_filter(b, &mut itr, &CONCEAL_FILTER);

    while itr.x.is_some() {
        let mark = marktree_itr_current(b, &itr);
        if mark.pos.row > row {
            break;
        }
        if mt_conceal_lines(mark) && ns_in_win(mark.ns, &mut *wp) {
            return true;
        }
        marktree_itr_next_filter(b, &mut itr, row + 1, 0, &CONCEAL_FILTER);
    }

    decor_providers_invoke_conceal_line(&mut *wp, row)
}

pub fn win_lines_concealed(wp: &mut WinInfoInfo) -> bool {
    has_any_folding(wp) != 0 || wp.w_onebuf_opt.wo_cole >= 2
}

pub fn sign_item_cmp(s1: &SignItem<'_>, s2: &SignItem<'_>) -> Ordering {
    let p1 = s1.sh.priority;
    let p2 = s2.sh.priority;
    if p1 != p2 {
        return if p1 < p2 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if s1.id != s2.id {
        return if s1.id < s2.id {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    let a1 = s1.sh.sign_add_id;
    let a2 = s2.sh.sign_add_id;
    if a1 != a2 {
        return if a1 < a2 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    Ordering::Equal
}

pub fn decor_redraw_signs(
    wp: *mut WinInfoInfo,
    buf: *mut FileBuf,
    row: i32,
    sattrs: *mut SignTextAttrs,
    mut line_id: Option<&mut i32>,
    mut cul_id: Option<&mut i32>,
    mut num_id: Option<&mut i32>,
) {
    let wp = unsafe { &mut *wp };
    let buf = unsafe { &mut *buf };
    if !buf_has_signs(&*buf) {
        return;
    }

    let mut pair: MTPair = MTPair::new();
    let mut num_text: i32 = 0;
    let mut itr = MarkTreeIter::new();
    let mut signs: Vec<SignItem<'_>> = Vec::new();
    let b = &buf.b_marktree;

    marktree_itr_get_overlap(b, row, 0, &mut itr);
    while marktree_itr_step_overlap(b, &mut itr, &mut pair) {
        if !mt_invalid(pair.start)
            && mt_decor_sign(pair.start)
            && ns_in_win(pair.start.ns, &mut *wp)
        {
            let sh = decor_find_sign(mt_decor(pair.start)).unwrap();
            num_text += i32::from(sh.text[0] != 0);
            signs.push(SignItem {
                sh,
                id: pair.start.id,
            });
        }
    }

    marktree_itr_step_out_filter(b, &mut itr, &SIGN_FILTER);

    while itr.x.is_some() {
        let mark = marktree_itr_current(b, &itr);
        if mark.pos.row != row {
            break;
        }
        if !mt_invalid(mark) && !mt_end(mark) && mt_decor_sign(mark) && ns_in_win(mark.ns, &mut *wp)
        {
            let sh = decor_find_sign(mt_decor(mark)).unwrap();
            num_text += i32::from(sh.text[0] != 0);
            signs.push(SignItem { sh, id: mark.id });
        }

        marktree_itr_next_filter(b, &mut itr, row + 1, 0, &SIGN_FILTER);
    }

    if !signs.is_empty() {
        let width = if wp.w_minscwidth == SCL_NUM {
            1
        } else {
            wp.w_scwidth
        };
        let len = width.min(num_text);
        let mut idx: usize = 0;
        signs.sort_by(|a, b| sign_item_cmp(a, b));

        for item in &signs {
            let sh = item.sh;
            if !sattrs.is_null() && (idx as i32) < len && sh.text[0] != 0 {
                unsafe {
                    (*sattrs.wrapping_add(idx)).text = sh.text;
                }
                unsafe {
                    (*sattrs.wrapping_add(idx)).hl_id = sh.hl_id;
                }
                idx += 1;
            }
            if num_id.is_some() && *num_id.as_deref_mut().unwrap() <= 0 {
                *num_id.as_deref_mut().unwrap() = sh.number_hl_id;
            }
            if line_id.is_some() && *line_id.as_deref_mut().unwrap() <= 0 {
                *line_id.as_deref_mut().unwrap() = sh.line_hl_id;
            }
            if cul_id.is_some() && *cul_id.as_deref_mut().unwrap() <= 0 {
                *cul_id.as_deref_mut().unwrap() = sh.cursorline_hl_id;
            }
        }
    }
}

pub fn decor_find_sign(decor: DecorInline) -> Option<&'static DecorSignHighlight> {
    if !decor.is_ext() {
        return None;
    }
    let mut decor_id = decor.data.ext().sh_idx;
    loop {
        if decor_id == DECOR_ID_INVALID {
            return None;
        }
        let sh = &(&g_st().decor_items)[decor_id as usize];
        if (sh.flags & K_SHIS_SIGN) != 0 {
            return Some(sh);
        }
        decor_id = sh.next;
    }
}

pub fn buf_signcols_count_range(
    buf: &mut FileBuf,
    row1: i32,
    row2: i32,
    add: i32,
    clear: TriState,
) {
    if !buf.b_signcols.autom || row2 < row1 || buf_meta_total(buf, K_MTMETA_SIGN_TEXT) == 0 {
        return;
    }

    let mut count = vec![0_i32; (row2 + 1 - row1) as usize];
    let mut itr = MarkTreeIter::new();
    let mut pair: MTPair = MTPair::new();
    let b = &buf.b_marktree;

    marktree_itr_get_overlap(b, row1, 0, &mut itr);
    while marktree_itr_step_overlap(b, &mut itr, &mut pair) {
        if (pair.start.flags & MT_FLAG_DECOR_SIGNTEXT) != 0 && !mt_invalid(pair.start) {
            for i in row1..=row2.min(pair.end_pos.row) {
                count[(i - row1) as usize] += 1;
            }
        }
    }

    marktree_itr_step_out_filter(b, &mut itr, &SIGNTEXT_FILTER);

    while itr.x.is_some() {
        let mark = marktree_itr_current(b, &itr);
        if mark.pos.row > row2 {
            break;
        }
        if (mark.flags & MT_FLAG_DECOR_SIGNTEXT) != 0 && !mt_invalid(mark) && !mt_end(mark) {
            let end = marktree_get_altpos(b, mark, None);
            for i in mark.pos.row..=row2.min(end.row) {
                count[(i - row1) as usize] += 1;
            }
        }

        marktree_itr_next_filter(b, &mut itr, row2 + 1, 0, &SIGNTEXT_FILTER);
    }

    for &c_val in &count {
        let prevwidth = SIGN_SHOW_MAX.min(c_val - add);
        if clear != K_NONE && prevwidth > 0 {
            buf.b_signcols.count[(prevwidth - 1) as usize] -= 1;
        }
        let width = SIGN_SHOW_MAX.min(c_val);
        if clear != K_TRUE && width > 0 {
            buf.b_signcols.count[(width - 1) as usize] += 1;
            if width > buf.b_signcols.max {
                buf.b_signcols.max = width;
            }
        }
    }
}

#[inline]
pub fn decor_redraw_col(
    wp: &mut WinInfoInfo,
    col: i32,
    win_col: i32,
    hidden: bool,
    state: &mut DecorState,
    max_col_last: i32,
) -> i32 {
    if col <= state.col_last {
        return state.current;
    }
    decor_redraw_col_impl(wp, col, win_col, hidden, state, max_col_last)
}

pub fn decor_redraw_eol(
    wp: &mut WinInfoInfo,
    state: &mut DecorState,
    eol_attr: &mut i32,
    eol_col: i32,
) -> bool {
    decor_redraw_col(wp, MAXCOL, MAXCOL, false, state, MAXCOL);
    state.eol_col = eol_col;

    let count = state.current_end;

    let mut has_virt_pos = false;
    for i in 0..count {
        let slot_idx = (&state.ranges_i)[i as usize] as usize;
        let r = (&state.slots)[slot_idx].range();
        has_virt_pos |= r.start_row == state.row && decor_virt_pos(r);

        if r.data.is_sh() && (r.data.sh().flags & K_SHHL_EOL) != 0 {
            *eol_attr = hl_combine_attr(*eol_attr, r.attr_id);
        }
    }
    has_virt_pos
}

#[inline]
fn decor_virt_line_wrap(wp: &mut WinInfoInfo, overflow: VirtLineOverflow) -> bool {
    overflow == K_VLOVERFLOW_WRAP || (overflow == K_VLOVERFLOW_AUTO && wp.w_onebuf_opt.wo_wrap != 0)
}

pub fn decor_virt_line_rows(
    wp: &mut WinInfoInfo,
    vl: &VirtLine,
    target_row: i32,
    mut skip_cells: Option<&mut i32>,
) -> i32 {
    if let Some(ref mut p) = skip_cells {
        **p = 0;
    }
    if !decor_virt_line_wrap(wp, vl.overflow) {
        return 1;
    }
    let row_width = wp.w_view_width
        - if (vl.flags & K_VLLEFTCOL) != 0 {
            0
        } else {
            win_col_off(wp)
        };
    if row_width <= 0 {
        return 1;
    }
    let vt = &vl.line;
    let buf = wp.w_buffer;
    let mut vcol: i32 = 0;
    let mut row_cells: i32 = 0;
    let mut row: i32 = 0;
    for chunk in vt {
        let Some(ref text) = chunk.text else {
            continue;
        };
        let mut virt_str: *const c_char = text.as_ptr();
        while !crate::memory::cstr_is_empty(virt_str) {
            let bytes_to_next_char = utfc_ptr2len(virt_str);
            let cells = if unsafe { *virt_str } == TAB {
                tabstop_padding(vcol, unsafe { (*buf).b_p_ts }, unsafe {
                    &(*buf).b_p_vts_array
                })
            } else {
                utf_ptr2cells(virt_str)
            };
            virt_str = virt_str.wrapping_add(bytes_to_next_char as usize);
            if row_cells + cells > row_width {
                row += 1;
                if skip_cells.is_some() && row == target_row {
                    *skip_cells.as_deref_mut().unwrap() = vcol;
                }
                row_cells = 0;
            }
            row_cells += cells;
            vcol += cells;
        }
    }

    row + 1
}

pub fn decor_virt_lines(
    wp: &mut WinInfoInfo,
    start_row: i32,
    end_row: i32,
    mut num_below: Option<&mut i32>,
    mut lines: Option<&mut VirtLines>,
    apply_folds: bool,
) -> i32 {
    unsafe {
        let buf = wp.w_buffer;
        if buf_meta_total(&*buf, K_MTMETA_LINES) == 0 {
            return 0;
        }

        let mut itr = MarkTreeIter::new();
        let b = &(*buf).b_marktree;
        if !marktree_itr_get_filter(
            b,
            (start_row - 1).max(0),
            0,
            end_row,
            0,
            &LINES_FILTER,
            &mut itr,
        ) {
            return 0;
        }

        assert!(start_row >= 0);

        let mut n_virt_lines: i32 = 0;
        loop {
            let mark = marktree_itr_current(b, &itr);
            let mut vt = mt_decor_virt(mark);
            if !mt_invalid(mark) && ns_in_win(mark.ns, &mut *wp) {
                while !vt.is_null() {
                    let virt_lines = (*vt).data.virt_lines();
                    if ((*vt).flags & K_VTIS_LINES) != 0 && !virt_lines.is_empty() {
                        let above = ((*vt).flags & K_VTLINES_ABOVE) != 0;
                        let mrow = mark.pos.row;
                        let draw_row = mrow + if above { 0 } else { 1 };
                        if draw_row >= start_row
                            && draw_row < end_row
                            && (!apply_folds
                                || !(has_folding(wp, mrow + 1, None, ptr::null_mut())
                                    || decor_conceal_line(wp, mrow, false)))
                        {
                            if decor_virt_line_wrap(wp, virt_lines[0].overflow) {
                                for item in virt_lines {
                                    let rows = decor_virt_line_rows(&mut *wp, item, 0, None);
                                    n_virt_lines += rows;
                                    if num_below.is_some() && !above {
                                        *num_below.as_deref_mut().unwrap() += rows;
                                    }
                                }
                            } else {
                                n_virt_lines += virt_lines.len() as i32;
                                if num_below.is_some() && !above {
                                    *num_below.as_deref_mut().unwrap() += virt_lines.len() as i32;
                                }
                            }
                            if lines.is_some() {
                                lines.as_deref_mut().unwrap().extend_from_slice(virt_lines);
                            }
                        }
                    }
                    vt = (*vt).next_ptr();
                }
            }

            if !marktree_itr_next_filter(b, &mut itr, end_row, 0, &LINES_FILTER) {
                break;
            }
        }

        n_virt_lines
    }
}

pub fn decor_to_dict_legacy(dict: &mut Dict, decor: DecorInline, hl_name: bool, arena: &mut Arena) {
    let inline_sh;
    let mut sh_hl = &DECOR_SIGN_HIGHLIGHT_INIT;
    let mut sh_sign = &DECOR_SIGN_HIGHLIGHT_INIT;
    let mut virt_text: *mut DecorVirtText = ptr::null_mut();
    let mut virt_lines: *mut DecorVirtText = ptr::null_mut();
    let mut priority: i32 = -1;

    if decor.is_ext() {
        for vt in virt_text_iter_ptr(decor.data.ext().vt) {
            if (vt.flags & K_VTIS_LINES) != 0 {
                virt_lines = vt;
            } else {
                virt_text = vt;
            }
        }

        let mut idx = decor.data.ext().sh_idx;
        while idx != DECOR_ID_INVALID {
            let sh = &g_st().decor_items.as_slice()[idx as usize];
            let next = sh.next;
            if (sh.flags & K_SHIS_SIGN) != 0 {
                sh_sign = sh;
            } else {
                sh_hl = sh;
            }
            idx = next;
        }
    } else {
        inline_sh = decor_sh_from_inline(decor.data.hl());
        sh_hl = &inline_sh;
    }

    if sh_hl.hl_id != 0 {
        put_c(dict, c"hl_group", hl_group_name(sh_hl.hl_id, hl_name));
        put_c(
            dict,
            c"hl_eol",
            boolean_obj((sh_hl.flags & K_SHHL_EOL) != 0),
        );
        priority = i32::from(sh_hl.priority);
    }

    if (sh_hl.flags & K_SHCONCEAL_OFF) != 0 {
        put_c(dict, c"conceal", boolean_obj(false));
    } else if (sh_hl.flags & K_SHCONCEAL) != 0 {
        let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(buf.as_mut_ptr(), sh_hl.text[0]);
        put_c(dict, c"conceal", cstr_to_arena_obj(arena, buf.as_ptr()));
    }

    if (sh_hl.flags & K_SHCONCEAL_LINES) != 0 {
        put_c(dict, c"conceal_lines", cstr_as_obj(c""));
    }

    if (sh_hl.flags & K_SHSPELL_ON) != 0 {
        put_c(dict, c"spell", boolean_obj(true));
    } else if (sh_hl.flags & K_SHSPELL_OFF) != 0 {
        put_c(dict, c"spell", boolean_obj(false));
    }

    if (sh_hl.flags & K_SHUIWATCHED) != 0 {
        put_c(dict, c"ui_watched", boolean_obj(true));
    }

    if let Some(ref url) = sh_hl.url {
        put_c(dict, c"url", cstr_as_obj(url.as_ptr()));
    }

    if !virt_text.is_null() {
        let virt_text = unsafe { &mut *virt_text };
        if virt_text.hl_mode != 0 {
            let mode_str = HL_MODE_STR[virt_text.hl_mode as usize].as_ptr();
            put_c(dict, c"hl_mode", cstr_as_obj(mode_str));
        }

        let chunks = virt_text_to_array(virt_text.data.virt_text(), hl_name, arena);
        put_c(dict, c"virt_text", array_obj(chunks));
        put_c(
            dict,
            c"virt_text_hide",
            boolean_obj((virt_text.flags & K_VTHIDE) != 0),
        );
        put_c(
            dict,
            c"virt_text_repeat_linebreak",
            boolean_obj((virt_text.flags & K_VTREPEAT_LINEBREAK) != 0),
        );
        if virt_text.pos == K_VPOS_WIN_COL {
            put_c(
                dict,
                c"virt_text_win_col",
                integer_obj(Integer::from(virt_text.col)),
            );
        }
        let pos_str = VIRT_TEXT_POS_STR[virt_text.pos as usize].as_ptr();
        put_c(dict, c"virt_text_pos", cstr_as_obj(pos_str));
        priority = i32::from(virt_text.priority);
    }

    if !virt_lines.is_null() {
        let virt_lines = unsafe { &mut *virt_lines };
        let vl_vec = virt_lines.data.virt_lines();
        let mut all_chunks = arena_array(arena, vl_vec.len());
        let mut virt_lines_flags: i32 = 0;
        let mut virt_lines_overflow: VirtLineOverflow = K_VLOVERFLOW_TRUNC;
        for vl in vl_vec {
            virt_lines_flags = vl.flags;
            virt_lines_overflow = vl.overflow;
            let chunks = virt_text_to_array(&vl.line, hl_name, arena);
            all_chunks.push(array_obj(chunks));
        }
        put_c(dict, c"virt_lines", array_obj(all_chunks));
        put_c(
            dict,
            c"virt_lines_above",
            boolean_obj((virt_lines.flags & K_VTLINES_ABOVE) != 0),
        );
        put_c(
            dict,
            c"virt_lines_leftcol",
            boolean_obj((virt_lines_flags & K_VLLEFTCOL) != 0),
        );

        let overflow = match virt_lines_overflow {
            K_VLOVERFLOW_SCROLL => c"scroll".as_ptr(),
            K_VLOVERFLOW_WRAP => c"wrap".as_ptr(),
            K_VLOVERFLOW_AUTO => c"auto".as_ptr(),
            _ => c"trunc".as_ptr(),
        };
        put_c(dict, c"virt_lines_overflow", cstr_as_obj(overflow));
        priority = i32::from(virt_lines.priority);
    }

    if (sh_sign.flags & K_SHIS_SIGN) != 0 {
        if sh_sign.text[0] != 0 {
            let mut buf = [0 as c_char; SIGN_WIDTH * MAX_SCHAR_SIZE];
            describe_sign_text(buf.as_mut_ptr(), sh_sign.text.as_ptr().cast_mut());
            put_c(dict, c"sign_text", cstr_to_arena_obj(arena, buf.as_ptr()));
        }

        if let Some(ref sign_name) = sh_sign.sign_name {
            put_c(dict, c"sign_name", cstr_as_obj(sign_name.as_ptr()));
        }

        let hls = [
            (c"sign_hl_group", sh_sign.hl_id),
            (c"number_hl_group", sh_sign.number_hl_id),
            (c"line_hl_group", sh_sign.line_hl_id),
            (c"cursorline_hl_group", sh_sign.cursorline_hl_id),
        ];
        for (name, val) in hls {
            if val != 0 {
                put_c(dict, name, hl_group_name(val, hl_name));
            }
        }
        priority = i32::from(sh_sign.priority);
    }

    if priority != -1 {
        put_c(dict, c"priority", integer_obj(Integer::from(priority)));
    }
}

pub fn decor_type_flags(decor: DecorInline) -> u16 {
    if decor.is_ext() {
        let mut type_flags: u16 = K_EXTMARK_NONE;
        for vt in virt_text_iter_mut(decor.data.ext().vt) {
            type_flags |= if (vt.flags & K_VTIS_LINES) != 0 {
                K_EXTMARK_VIRT_LINES
            } else {
                K_EXTMARK_VIRT_TEXT
            };
        }
        let mut idx = decor.data.ext().sh_idx;
        while idx != DECOR_ID_INVALID {
            let sh = &g_st().decor_items.as_slice()[idx as usize];
            type_flags |= if (sh.flags & K_SHIS_SIGN) != 0 {
                K_EXTMARK_SIGN
            } else {
                K_EXTMARK_HIGHLIGHT
            };
            idx = sh.next;
        }
        type_flags
    } else if (decor.data.hl().flags & K_SHIS_SIGN) != 0 {
        K_EXTMARK_SIGN
    } else {
        K_EXTMARK_HIGHLIGHT
    }
}

pub fn hl_group_name(hl_id: i32, hl_name: bool) -> Object {
    if hl_name {
        cstr_as_obj(syn_id2name(hl_id))
    } else {
        integer_obj(Integer::from(hl_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decor_sh_from_inline_and_type_flags_work() {
        let inline = DecorHighlightInline {
            flags: 0,
            priority: 50,
            hl_id: 7,
            conceal_char: 0x41,
        };
        let sh = decor_sh_from_inline(inline);
        assert_eq!(sh.priority, 50);
        assert_eq!(sh.hl_id, 7);
        assert_eq!(sh.text[0], 0x41);

        let di = DecorInline {
            data: DecorInlineData::Hl(inline),
        };
        assert_eq!(decor_type_flags(di), K_EXTMARK_HIGHLIGHT);
    }
}
