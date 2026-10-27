//! Buffer decorations, virtual text/lines, and sign/highlight range state (`src/nvim/decoration.c`).

use crate::charset::NUL;

use crate::api::extmark::{ns_in_win, virt_text_to_array};
use crate::api::private::helpers::{
    arena_array, array_obj, boolean_obj, cstr_as_obj, cstr_to_arena_obj, integer_obj, put_c,
};
use crate::api::private::validate::{Dict, Object};
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::buffer::{buf_T, buf_meta_total};
use crate::change::{TAB, changed_lines_invalidate_buf};
use crate::cursor::{colnr_T, lpos_T};
use crate::decoration_provider::decor_providers_invoke_conceal_line;
use crate::drawscreen::{conceal_cursor_line, redraw_buf_line_later, redraw_buf_range_later};
use crate::extmark::{TriState, extmark_set, kFalse, kNone, kTrue};
use crate::fold::{hasAnyFolding, hasFolding};
use crate::globals::*;
use crate::grid::{schar_T, schar_from_char, schar_get, schar_get_first_codepoint, schar_high};
use crate::highlight::{hl_add_url, hl_combine_attr};
use crate::highlight_group::{syn_id2attr, syn_id2name};
use crate::indent::tabstop_padding;
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DecorHighlightInline, DecorInlineData,
    MT_FLAG_DECOR_CONCEAL_LINES, MT_FLAG_DECOR_EXT, MT_FLAG_DECOR_HL, MT_FLAG_DECOR_SIGNTEXT,
    MTKey, MTPair, MarkTreeIter, kMTMetaConcealLines, kMTMetaCount, kMTMetaLines, kMTMetaSignText,
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
use crate::window::win_T;
use std::cmp::Ordering;
use std::ffi::{c_char, c_int};
use std::ptr;

pub type NS = crate::msgpack_rpc::packer::handle_T;
pub type HLGroupID = Integer;
pub type DecorPriority = u16;
pub type DecorPriorityInternal = u32;
pub type VirtTextPos = c_int;
pub type VirtLineOverflow = c_int;

pub(crate) const DECOR_ID_INVALID: u32 = u32::MAX;
pub(crate) const kHlModeUnknown: u8 = 0;
pub(crate) const kVPosEndOfLine: VirtTextPos = 0;
pub(crate) const kVPosEndOfLineRightAlign: VirtTextPos = 1;
pub(crate) const kVPosInline: VirtTextPos = 2;
pub(crate) const kVPosOverlay: VirtTextPos = 3;
pub(crate) const kVPosRightAlign: VirtTextPos = 4;
pub(crate) const kVPosWinCol: VirtTextPos = 5;

pub(crate) const kVLLeftcol: c_int = 1;

pub(crate) const kVLOverflowTrunc: VirtLineOverflow = 0;
pub(crate) const kVLOverflowScroll: VirtLineOverflow = 1;
pub(crate) const kVLOverflowWrap: VirtLineOverflow = 2;
pub(crate) const kVLOverflowAuto: VirtLineOverflow = 3;

pub(crate) const kSHIsSign: u16 = 1;
pub(crate) const kSHHlEol: u16 = 2;
pub(crate) const kSHUIWatched: u16 = 4;
pub(crate) const kSHUIWatchedOverlay: u16 = 8;
pub(crate) const kSHSpellOn: u16 = 16;
pub(crate) const kSHSpellOff: u16 = 32;
pub(crate) const kSHConceal: u16 = 64;
pub(crate) const kSHConcealLines: u16 = 128;
pub(crate) const kSHConcealOff: u16 = 256;

pub(crate) const kVTIsLines: u8 = 1;
pub(crate) const kVTHide: u8 = 2;
pub(crate) const kVTLinesAbove: u8 = 4;
pub(crate) const kVTRepeatLinebreak: u8 = 8;

#[repr(C)]
#[derive(Clone)]
pub struct DecorSignHighlight {
    pub flags: u16,
    pub priority: DecorPriority,
    pub hl_id: c_int,
    pub text: [u32; 2],
    pub sign_name: Option<std::ffi::CString>,
    pub sign_add_id: c_int,
    pub number_hl_id: c_int,
    pub line_hl_id: c_int,
    pub cursorline_hl_id: c_int,
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
    pub start_row: c_int,
    pub start_col: c_int,
    pub end_row: c_int,
    pub end_col: c_int,
    pub ordering: c_int,
    pub priority_internal: u32,
    pub owned: bool,
    pub data: DecorRangeData,
    pub attr_id: c_int,
    pub draw_col: c_int,
}

impl Drop for DecorRange {
    fn drop(&mut self) {
        if self.owned {
            unsafe {
                if let DecorRangeData::Vt(vt) | DecorRangeData::VirtLines(vt) = self.data
                    && !vt.is_null()
                {
                    drop(Box::from_raw(vt));
                }
            }
        }
    }
}

pub enum DecorRangeSlot {
    Occupied(DecorRange),
    Free(c_int),
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
    pub const fn next_free_i(&self) -> c_int {
        match self {
            Self::Free(i) => *i,
            Self::Occupied(_) => unreachable!(),
        }
    }
}

pub struct DecorState {
    pub itr: MarkTreeIter,
    pub slots: Vec<DecorRangeSlot>,
    pub ranges_i: Vec<c_int>,
    pub current_end: c_int,
    pub future_begin: c_int,
    pub free_slot_i: c_int,
    pub new_range_ordering: c_int,
    pub win: *mut win_T,
    pub top_row: c_int,
    pub row: c_int,
    pub col_last: c_int,
    pub current: c_int,
    pub current_hl_eol: c_int,
    pub eol_col: c_int,
    pub conceal: c_int,
    pub conceal_char: u32,
    pub conceal_attr: c_int,
    pub spell: c_int,
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
    pub hl_id: c_int,
}

pub type VirtText = Vec<VirtTextChunk>;

#[repr(C)]
#[derive(Clone)]
pub struct VirtLine {
    pub line: VirtText,
    pub flags: c_int,
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
    pub width: c_int,
    pub col: c_int,
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

use crate::extmark::{
    kExtmarkHighlight, kExtmarkNone, kExtmarkSign, kExtmarkVirtLines, kExtmarkVirtText,
};

pub(crate) const SIGN_WIDTH: usize = 2;
const SIGN_SHOW_MAX: c_int = 9;
pub(crate) const SCL_NUM: c_int = -2;

pub(crate) const kMTFilterSelect: u32 = u32::MAX;

static CONCEAL_FILTER: [u32; kMTMetaCount] = [0, 0, 0, 0, kMTFilterSelect];
static SIGN_FILTER: [u32; kMTMetaCount] = [0, 0, kMTFilterSelect, kMTFilterSelect, 0];
static SIGNTEXT_FILTER: [u32; kMTMetaCount] = [0, 0, 0, kMTFilterSelect, 0];
static LINES_FILTER: [u32; kMTMetaCount] = [0, kMTFilterSelect, 0, 0, 0];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SignTextAttrs {
    pub text: [schar_T; SIGN_WIDTH],
    pub hl_id: c_int,
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
    sign_add_id: c_int,
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

static DECOR_MOD_STATE: GlobalState<DecorationModuleState> = GlobalState::new(DecorationModuleState::new());

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

pub unsafe fn bufhl_add_hl_pos_offset(
    buf: *mut buf_T,
    src_id: c_int,
    hl_id: c_int,
    pos_start: lpos_T,
    pos_end: lpos_T,
    offset: colnr_T,
) {
    unsafe {
        let mut hl_start: colnr_T = 0;
        let mut hl_end: colnr_T = 0;
        let decor = DecorInline {
            data: DecorInlineData::Hl(DecorHighlightInline {
                hl_id,
                ..DECOR_HIGHLIGHT_INLINE_INIT
            }),
        };

        for lnum in pos_start.lnum..=pos_end.lnum {
            let mut end_off: c_int = 0;
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
                ptr::null_mut(),
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
                ptr::null_mut(),
            );
        }
    }
}

pub unsafe fn decor_redraw(
    buf: *mut buf_T,
    row1: c_int,
    row2: c_int,
    col1: c_int,
    decor: DecorInline,
) {
    unsafe {
        if decor.is_ext() {
            let mut vt = decor.data.ext().vt;
            while !vt.is_null() {
                let below = ((*vt).flags & kVTIsLines) != 0 && ((*vt).flags & kVTLinesAbove) == 0;
                let vt_lnum = row1 + 1 + c_int::from(below);
                redraw_buf_line_later(buf, vt_lnum, true);
                if ((*vt).flags & kVTIsLines) != 0 || (*vt).pos == kVPosInline {
                    let vt_col = if ((*vt).flags & kVTIsLines) != 0 {
                        0
                    } else {
                        col1
                    };
                    changed_lines_invalidate_buf(buf, vt_lnum, vt_col, vt_lnum + 1, 0);
                }
                vt = (*vt).next_ptr();
            }

            let items = &raw const g_st().decor_items;
            let mut idx = decor.data.ext().sh_idx;
            while idx != DECOR_ID_INVALID {
                let sh = &(*items).as_slice()[idx as usize];
                let next = sh.next;
                decor_redraw_sh(buf, row1, row2, sh.clone());
                idx = next;
            }
        } else {
            decor_redraw_sh(buf, row1, row2, decor_sh_from_inline(decor.data.hl()));
        }
    }
}

pub unsafe fn decor_redraw_sh(buf: *mut buf_T, row1: c_int, row2: c_int, sh: DecorSignHighlight) {
    unsafe {
        if (sh.hl_id != 0
            || sh.url.is_some()
            || (sh.flags & (kSHIsSign | kSHSpellOn | kSHSpellOff | kSHConceal | kSHConcealOff))
                != 0)
            && row2 >= row1
        {
            redraw_buf_range_later(buf, row1 + 1, row2 + 1);
        }
        if (sh.flags & kSHConcealLines) != 0 {
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    changed_window_setting(wp);
                }
                wp = (*wp).w_next;
            }
        }
        if (sh.flags & kSHUIWatched) != 0 {
            redraw_buf_line_later(buf, row1 + 1, false);
        }
    }
}

pub unsafe fn decor_put_sh(item: DecorSignHighlight) -> u32 {
    unsafe {
        let items = &raw mut g_st().decor_items;
        if decor_st().decor_freelist != u32::MAX {
            let pos = decor_st().decor_freelist;
            decor_st().decor_freelist = (*items).as_slice()[decor_st().decor_freelist as usize].next;
            (*items).as_mut_slice()[pos as usize] = item;
            pos
        } else {
            let pos = (*items).len() as u32;
            (*items).push(item);
            pos
        }
    }
}

pub unsafe fn decor_put_vt(mut vt: DecorVirtText, next: *mut DecorVirtText) -> *mut DecorVirtText {
    vt.next = (!next.is_null()).then(|| unsafe { Box::from_raw(next) });
    Box::into_raw(Box::new(vt))
}

pub fn decor_sh_from_inline(item: DecorHighlightInline) -> DecorSignHighlight {
    assert!((item.flags & kSHIsSign) == 0);
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

pub unsafe fn buf_put_decor(buf: *mut buf_T, decor: DecorInline, row: c_int, mut row2: c_int) {
    unsafe {
        if decor.is_ext() && row < (*buf).b_ml.ml_line_count {
            let items = &raw mut g_st().decor_items;
            let mut idx = decor.data.ext().sh_idx;
            row2 = row2.min((*buf).b_ml.ml_line_count - 1);
            while idx != DECOR_ID_INVALID {
                let sh = &mut (&mut *items)[idx as usize];
                let next = sh.next;
                buf_put_decor_sh(buf, sh, row, row2);
                idx = next;
            }
        }
    }
}

unsafe fn may_force_numberwidth_recompute(buf: *mut buf_T, unplace: bool) {
    unsafe {
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if ptr::eq(tp, g_st().curtab) {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if (*wp).w_buffer == buf
                    && (*wp).w_minscwidth == SCL_NUM
                    && ((*wp).w_onebuf_opt.wo_nu != 0 || (*wp).w_onebuf_opt.wo_rnu != 0)
                    && (unplace || (*wp).w_nrwidth_width < 2)
                {
                    (*wp).w_nrwidth_line_count = 0;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

pub unsafe fn buf_put_decor_sh(
    buf: *mut buf_T,
    sh: &mut DecorSignHighlight,
    row1: c_int,
    row2: c_int,
) {
    unsafe {
        if (sh.flags & kSHIsSign) != 0 {
            sh.sign_add_id = decor_st().sign_add_id;
            decor_st().sign_add_id += 1;
            if sh.text[0] != 0 {
                buf_signcols_count_range(buf, row1, row2, 1, kFalse);
                may_force_numberwidth_recompute(buf, false);
            }
        }
    }
}

pub unsafe fn buf_decor_remove(
    buf: *mut buf_T,
    row1: c_int,
    mut row2: c_int,
    col1: c_int,
    decor: DecorInline,
    free: bool,
) {
    unsafe {
        decor_redraw(buf, row1, row2, col1, decor);
        if decor.is_ext() && row1 < (*buf).b_ml.ml_line_count {
            let items = &raw const g_st().decor_items;
            let mut idx = decor.data.ext().sh_idx;
            row2 = row2.min((*buf).b_ml.ml_line_count - 1);
            while idx != DECOR_ID_INVALID {
                let sh = &(&*items)[idx as usize];
                let next = sh.next;
                buf_remove_decor_sh(buf, row1, row2, sh);
                idx = next;
            }
        }
        if free {
            decor_free(decor);
        }
    }
}

pub unsafe fn buf_remove_decor_sh(
    buf: *mut buf_T,
    row1: c_int,
    row2: c_int,
    sh: &DecorSignHighlight,
) {
    unsafe {
        if (sh.flags & kSHIsSign) != 0 && sh.text[0] != 0 {
            if buf_meta_total(buf, kMTMetaSignText) != 0 {
                buf_signcols_count_range(buf, row1, row2, -1, kFalse);
            } else {
                may_force_numberwidth_recompute(buf, true);
                (*buf).b_signcols.count[0] = 0;
                (*buf).b_signcols.max = 0;
            }
        }
    }
}

pub unsafe fn decor_free(decor: DecorInline) {
    unsafe {
        if !decor.is_ext() {
            return;
        }
        let ext = decor.data.ext();
        let vt = ext.vt;
        let mut idx = ext.sh_idx;

        if g_st().decor_state.running_decor_provider {
            if !vt.is_null() {
                let list = &raw mut decor_st().to_free_virt;
                (*list).push(*Box::from_raw(vt));
            }
            let items = &raw mut g_st().decor_items;
            while idx != DECOR_ID_INVALID {
                let sh = &mut (&mut *items)[idx as usize];
                if sh.next == DECOR_ID_INVALID {
                    sh.next = decor_st().to_free_sh;
                    decor_st().to_free_sh = ext.sh_idx;
                    break;
                }
                idx = sh.next;
            }
        } else {
            if !vt.is_null() {
                drop(Box::from_raw(vt));
            }
            decor_free_sh(idx);
        }
    }
}

unsafe fn decor_free_sh(first_idx: u32) {
    unsafe {
        let items = &raw mut g_st().decor_items;
        let mut idx = first_idx;
        while idx != DECOR_ID_INVALID {
            let sh = &mut (&mut *items)[idx as usize];
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
}

pub unsafe fn decor_state_invalidate(buf: *mut buf_T) {
    unsafe {
        if !g_st().decor_state.win.is_null() && (*g_st().decor_state.win).w_buffer == buf {
            g_st().decor_state.itr_valid = false;
        }
    }
}

pub unsafe fn decor_check_to_be_deleted() {
    unsafe {
        assert!(!g_st().decor_state.running_decor_provider);
        let list = &raw mut decor_st().to_free_virt;
        (*list).clear();
        decor_free_sh(decor_st().to_free_sh);
        decor_st().to_free_sh = DECOR_ID_INVALID;
        g_st().decor_state.win = ptr::null_mut();
    }
}

pub unsafe fn decor_check_invalid_glyphs() {
    unsafe {
        let items = &raw mut g_st().decor_items;
        for it in (*items).iter_mut() {
            let width = if (it.flags & kSHIsSign) != 0 {
                SIGN_WIDTH
            } else if (it.flags & kSHConceal) != 0 {
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
}

pub unsafe fn next_virt_text_chunk(
    vt: &[VirtTextChunk],
    pos: &mut usize,
    attr: &mut c_int,
) -> *mut c_char {
    unsafe {
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
}

pub unsafe fn decor_find_virttext(buf: &mut buf_T, row: c_int, ns_id: u64) -> *mut DecorVirtText {
    unsafe {
        let mut itr = MarkTreeIter::new();
        marktree_itr_get(&buf.b_marktree, row, 0, &mut itr);
        loop {
            let mark = marktree_itr_current(&buf.b_marktree, &itr);
            if mark.pos.row < 0 || mark.pos.row > row {
                break;
            } else if !mt_invalid(mark) {
                let mut decor = mt_decor_virt(mark);
                while !decor.is_null() && ((*decor).flags & kVTIsLines) != 0 {
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

pub unsafe fn decor_redraw_reset(wp: *mut win_T, state: &mut DecorState) -> bool {
    unsafe {
        state.row = -1;
        state.win = wp;

        state.slots.clear();
        state.ranges_i.clear();
        state.free_slot_i = -1;
        state.current_end = 0;
        state.future_begin = 0;
        state.new_range_ordering = 0;

        (*(*wp).w_buffer).b_marktree.n_keys != 0
    }
}

pub fn decor_virt_pos(decor: &DecorRange) -> bool {
    decor.data.is_vt() || decor.data.is_ui()
}

pub unsafe fn decor_virt_pos_kind(decor: &DecorRange) -> VirtTextPos {
    unsafe {
        match decor.data {
            DecorRangeData::Vt(vt) => (*vt).pos,
            DecorRangeData::Ui(ui) => ui.pos,
            _ => kVPosEndOfLine,
        }
    }
}

pub unsafe fn decor_redraw_start(wp: *mut win_T, top_row: c_int, state: &mut DecorState) -> bool {
    unsafe {
        let buf = (*wp).w_buffer;
        state.top_row = top_row;
        state.itr_valid = true;

        if !marktree_itr_get_overlap(&(*buf).b_marktree, top_row, 0, &mut state.itr) {
            return false;
        }
        let mut pair: MTPair = MTPair::new();

        while marktree_itr_step_overlap(&(*buf).b_marktree, &mut state.itr, &mut pair) {
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
}

fn decor_state_pack(state: &mut DecorState) {
    let mut count = state.ranges_i.len() as c_int;
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

pub unsafe fn decor_redraw_line(wp: *mut win_T, row: c_int, state: &mut DecorState) {
    unsafe {
        decor_state_pack(state);

        if state.row == -1 {
            decor_redraw_start(wp, row, state);
        } else if !state.itr_valid {
            marktree_itr_get(&(*(*wp).w_buffer).b_marktree, row, 0, &mut state.itr);
            state.itr_valid = true;
        }

        state.row = row;
        state.col_last = -1;
        state.eol_col = -1;
    }
}

pub unsafe fn decor_has_more_decorations(state: &mut DecorState, row: c_int) -> bool {
    unsafe {
        if state.current_end != 0 || state.future_begin != state.ranges_i.len() as c_int {
            return true;
        }

        let k = marktree_itr_current(&(*(*state.win).w_buffer).b_marktree, &state.itr);
        k.pos.row >= 0 && k.pos.row <= row
    }
}

unsafe fn decor_range_add_from_inline(
    state: &mut DecorState,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
    decor: DecorInline,
    owned: bool,
    ns: u32,
    mark_id: u32,
) {
    unsafe {
        if decor.is_ext() {
            let mut vt = decor.data.ext().vt;
            while !vt.is_null() {
                decor_range_add_virt(state, start_row, start_col, end_row, end_col, vt, owned);
                vt = (*vt).next_ptr();
            }
            let items = &raw const g_st().decor_items;
            let mut idx = decor.data.ext().sh_idx;
            while idx != DECOR_ID_INVALID {
                let sh = &(&*items)[idx as usize];
                let next = sh.next;
                decor_range_add_sh(
                    state, start_row, start_col, end_row, end_col, sh, owned, ns, mark_id, 0,
                );
                idx = next;
            }
        } else {
            let sh = decor_sh_from_inline(decor.data.hl());
            decor_range_add_sh(
                state,
                start_row,
                start_col,
                end_row,
                end_col,
                &sh,
                owned,
                ns,
                mark_id,
                0,
            );
        }
    }
}

fn decor_range_insert(state: &mut DecorState, mut range: DecorRange) {
    let row = range.start_row;
        let col = range.start_col;
        range.ordering = state.new_range_ordering;
        state.new_range_ordering += 1;

        let index: c_int;
        if state.free_slot_i >= 0 {
            index = state.free_slot_i;
            let slot = &mut (&mut state.slots)[index as usize];
            state.free_slot_i = slot.next_free_i();
            *slot = DecorRangeSlot::Occupied(range);
        } else {
            index = state.slots.len() as c_int;
            state.slots.push(DecorRangeSlot::Occupied(range));
        }

        let count = state.ranges_i.len() as c_int;

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

pub unsafe fn decor_range_add_virt(
    state: &mut DecorState,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
    vt: *mut DecorVirtText,
    owned: bool,
) {
    unsafe {
        let is_lines = ((*vt).flags & kVTIsLines) != 0;
        let range = DecorRange {
            start_row,
            start_col,
            end_row,
            end_col,
            ordering: 0,
            priority_internal: DecorPriorityInternal::from((*vt).priority) << 16,
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
}

pub unsafe fn decor_range_add_sh(
    state: &mut DecorState,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
    sh: &DecorSignHighlight,
    owned: bool,
    ns: u32,
    mark_id: u32,
    subpriority: DecorPriority,
) {
    unsafe {
        if (sh.flags & kSHIsSign) != 0 {
            return;
        }

        let priority_internal = (DecorPriorityInternal::from(sh.priority) << 16)
            + DecorPriorityInternal::from(subpriority);

        if sh.hl_id != 0
            || sh.url.is_some()
            || (sh.flags & (kSHConcealOff | kSHConceal | kSHSpellOn | kSHSpellOff)) != 0
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

        if (sh.flags & kSHUIWatched) != 0 {
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
                        pos: if (sh.flags & kSHUIWatchedOverlay) != 0 {
                            kVPosOverlay
                        } else {
                            kVPosEndOfLine
                        },
                    }),
                    attr_id: 0,
                    draw_col: -10,
                },
            );
        }
    }
}

pub unsafe fn decor_init_draw_col(win_col: c_int, hidden: bool, item: &mut DecorRange) {
    unsafe {
        let vt = if item.data.is_vt() {
            item.data.vt()
        } else {
            ptr::null_mut()
        };
        let pos = decor_virt_pos_kind(item);
        if win_col < 0 && pos != kVPosInline {
            item.draw_col = win_col;
        } else if pos == kVPosOverlay {
            item.draw_col = if !vt.is_null() && ((*vt).flags & kVTHide) != 0 && hidden {
                c_int::MIN
            } else {
                win_col
            };
        } else {
            item.draw_col = -1;
        }
    }
}

pub unsafe fn decor_recheck_draw_col(win_col: c_int, hidden: bool, state: &mut DecorState) {
    unsafe {
        let end = state.current_end;

        for i in 0..end {
            let slot_idx = (&state.ranges_i)[i as usize] as usize;
            let r: *mut DecorRange = (&mut state.slots)[slot_idx].range_mut();
            if (*r).draw_col == -3 {
                decor_init_draw_col(win_col, hidden, &mut *r);
            }
        }
    }
}

pub unsafe fn decor_redraw_col_impl(
    wp: *mut win_T,
    col: c_int,
    win_col: c_int,
    hidden: bool,
    state: &mut DecorState,
    max_col_last: c_int,
) -> c_int {
    unsafe {
        let buf = (*wp).w_buffer;
        let row = state.row;
        let mut col_last = max_col_last;

        loop {
            let mark = marktree_itr_current(&(*buf).b_marktree, &state.itr);
            if mark.pos.row < 0 || mark.pos.row > row {
                break;
            } else if mark.pos.row == row && mark.pos.col > col {
                col_last = col_last.min(mark.pos.col - 1);
                break;
            }

            if !mt_invalid(mark) && !mt_end(mark) && mt_decor_any(mark) && ns_in_win(mark.ns, wp) {
                let endpos = marktree_get_altpos(&(*buf).b_marktree, mark, None);
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

            marktree_itr_next(&(*buf).b_marktree, &mut state.itr);
        }

        let mut count = state.ranges_i.len() as c_int;
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

            let mut begin: c_int = 0;
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

        let mut new_cur_end: c_int = 0;

        let mut attr: c_int = 0;
        let mut hl_eol_attr: c_int = 0;
        let mut conceal: c_int = 0;
        let mut conceal_char: schar_T = 0;
        let mut conceal_attr: c_int = 0;
        let mut spell: TriState = kNone;

        for i in 0..cur_end {
            let index = (&state.ranges_i)[i as usize];
            let slot = &raw mut (&mut state.slots)[index as usize];
            let r: *mut DecorRange = (*slot).range_mut();

            let keep: bool;
            if (*r).end_row < row || ((*r).end_row == row && (*r).end_col <= col) {
                keep = (*r).start_row >= row && decor_virt_pos(&*r);
            } else {
                keep = true;

                if (*r).end_row == row && (*r).end_col > col {
                    col_last = col_last.min((*r).end_col - 1);
                }

                if (*r).attr_id > 0 {
                    attr = hl_combine_attr(attr, (*r).attr_id);
                    if (*r).data.is_sh() && ((*r).data.sh().flags & kSHHlEol) != 0 {
                        hl_eol_attr = hl_combine_attr(hl_eol_attr, (*r).attr_id);
                    }
                }

                if (*r).data.is_sh() && ((*r).data.sh().flags & kSHConceal) != 0 {
                    conceal = 1;
                    if (*r).start_row == row && (*r).start_col == col {
                        conceal = 2;
                        conceal_char = (*r).data.sh().text[0];
                        col_last = col_last.min((*r).start_col);
                        conceal_attr = (*r).attr_id;
                    }
                }

                if (*r).data.is_sh() && ((*r).data.sh().flags & kSHConcealOff) != 0 {
                    conceal = 0;
                }

                if (*r).data.is_sh() {
                    if ((*r).data.sh().flags & kSHSpellOn) != 0 {
                        spell = kTrue;
                    } else if ((*r).data.sh().flags & kSHSpellOff) != 0 {
                        spell = kFalse;
                    }
                    if let Some(ref url) = (*r).data.sh().url {
                        attr = hl_add_url(attr, url.as_ptr());
                    }
                }
            }

            if (*r).start_row == row
                && (*r).start_col <= col
                && decor_virt_pos(&*r)
                && (*r).draw_col == -10
            {
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

pub unsafe fn decor_conceal_line(wp: *mut win_T, row: c_int, check_cursor: bool) -> bool {
    unsafe {
        if row < 0
            || (*wp).w_onebuf_opt.wo_cole < 2
            || (!check_cursor
                && ptr::eq(wp, *curwin.as_ptr())
                && row + 1 == (*wp).w_cursor.lnum
                && !conceal_cursor_line(wp))
        {
            return false;
        }

        if buf_meta_total((*wp).w_buffer, kMTMetaConcealLines) == 0 {
            return decor_providers_invoke_conceal_line(wp, row);
        }

        let mut pair: MTPair = MTPair::new();
        let mut itr = MarkTreeIter::new();
        let b = &(*(*wp).w_buffer).b_marktree;
        marktree_itr_get_overlap(b, row, 0, &mut itr);
        while marktree_itr_step_overlap(b, &mut itr, &mut pair) {
            if mt_conceal_lines(pair.start) && ns_in_win(pair.start.ns, wp) {
                return true;
            }
        }

        marktree_itr_step_out_filter(b, &mut itr, &CONCEAL_FILTER);

        while itr.x.is_some() {
            let mark = marktree_itr_current(b, &itr);
            if mark.pos.row > row {
                break;
            }
            if mt_conceal_lines(mark) && ns_in_win(mark.ns, wp) {
                return true;
            }
            marktree_itr_next_filter(b, &mut itr, row + 1, 0, &CONCEAL_FILTER);
        }

        decor_providers_invoke_conceal_line(wp, row)
    }
}

pub unsafe fn win_lines_concealed(wp: *mut win_T) -> bool {
    unsafe { hasAnyFolding(wp) != 0 || (*wp).w_onebuf_opt.wo_cole >= 2 }
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

pub unsafe fn decor_redraw_signs(
    wp: *mut win_T,
    buf: *mut buf_T,
    row: c_int,
    sattrs: *mut SignTextAttrs,
    line_id: *mut c_int,
    cul_id: *mut c_int,
    num_id: *mut c_int,
) {
    unsafe {
        if !buf_has_signs(buf) {
            return;
        }

        let mut pair: MTPair = MTPair::new();
        let mut num_text: c_int = 0;
        let mut itr = MarkTreeIter::new();
        let mut signs: Vec<SignItem<'_>> = Vec::new();
        let b = &(*buf).b_marktree;

        marktree_itr_get_overlap(b, row, 0, &mut itr);
        while marktree_itr_step_overlap(b, &mut itr, &mut pair) {
            if !mt_invalid(pair.start) && mt_decor_sign(pair.start) && ns_in_win(pair.start.ns, wp)
            {
                let sh = decor_find_sign(mt_decor(pair.start)).unwrap();
                num_text += c_int::from(sh.text[0] != 0);
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
            if !mt_invalid(mark) && !mt_end(mark) && mt_decor_sign(mark) && ns_in_win(mark.ns, wp) {
                let sh = decor_find_sign(mt_decor(mark)).unwrap();
                num_text += c_int::from(sh.text[0] != 0);
                signs.push(SignItem { sh, id: mark.id });
            }

            marktree_itr_next_filter(b, &mut itr, row + 1, 0, &SIGN_FILTER);
        }

        if !signs.is_empty() {
            let width = if (*wp).w_minscwidth == SCL_NUM {
                1
            } else {
                (*wp).w_scwidth
            };
            let len = width.min(num_text);
            let mut idx: usize = 0;
            signs.sort_by(|a, b| sign_item_cmp(a, b));

            for item in &signs {
                let sh = item.sh;
                if !sattrs.is_null() && (idx as c_int) < len && sh.text[0] != 0 {
                    (*sattrs.add(idx)).text = sh.text;
                    (*sattrs.add(idx)).hl_id = sh.hl_id;
                    idx += 1;
                }
                if !num_id.is_null() && *num_id <= 0 {
                    *num_id = sh.number_hl_id;
                }
                if !line_id.is_null() && *line_id <= 0 {
                    *line_id = sh.line_hl_id;
                }
                if !cul_id.is_null() && *cul_id <= 0 {
                    *cul_id = sh.cursorline_hl_id;
                }
            }
        }
    }
}

pub unsafe fn decor_find_sign(decor: DecorInline) -> Option<&'static DecorSignHighlight> {
    unsafe {
        if !decor.is_ext() {
            return None;
        }
        let items = &raw const g_st().decor_items;
        let mut decor_id = decor.data.ext().sh_idx;
        loop {
            if decor_id == DECOR_ID_INVALID {
                return None;
            }
            let sh = &(&*items)[decor_id as usize];
            if (sh.flags & kSHIsSign) != 0 {
                return Some(sh);
            }
            decor_id = sh.next;
        }
    }
}

pub unsafe fn buf_signcols_count_range(
    buf: *mut buf_T,
    row1: c_int,
    row2: c_int,
    add: c_int,
    clear: TriState,
) {
    unsafe {
        if !(*buf).b_signcols.autom || row2 < row1 || buf_meta_total(buf, kMTMetaSignText) == 0 {
            return;
        }

        let mut count = vec![0 as c_int; (row2 + 1 - row1) as usize];
        let mut itr = MarkTreeIter::new();
        let mut pair: MTPair = MTPair::new();
        let b = &(*buf).b_marktree;

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
            if clear != kNone && prevwidth > 0 {
                (*buf).b_signcols.count[(prevwidth - 1) as usize] -= 1;
            }
            let width = SIGN_SHOW_MAX.min(c_val);
            if clear != kTrue && width > 0 {
                (*buf).b_signcols.count[(width - 1) as usize] += 1;
                if width > (*buf).b_signcols.max {
                    (*buf).b_signcols.max = width;
                }
            }
        }
    }
}

#[inline]
pub unsafe fn decor_redraw_col(
    wp: *mut win_T,
    col: c_int,
    win_col: c_int,
    hidden: bool,
    state: &mut DecorState,
    max_col_last: c_int,
) -> c_int {
    unsafe {
        if col <= state.col_last {
            return state.current;
        }
        decor_redraw_col_impl(wp, col, win_col, hidden, state, max_col_last)
    }
}

pub unsafe fn decor_redraw_eol(
    wp: *mut win_T,
    state: &mut DecorState,
    eol_attr: &mut c_int,
    eol_col: c_int,
) -> bool {
    unsafe {
        decor_redraw_col(wp, MAXCOL, MAXCOL, false, state, MAXCOL);
        state.eol_col = eol_col;

        let count = state.current_end;

        let mut has_virt_pos = false;
        for i in 0..count {
            let slot_idx = (&state.ranges_i)[i as usize] as usize;
            let r: *const DecorRange = (&state.slots)[slot_idx].range();
            has_virt_pos |= (*r).start_row == state.row && decor_virt_pos(&*r);

            if (*r).data.is_sh() && ((*r).data.sh().flags & kSHHlEol) != 0 {
                *eol_attr = hl_combine_attr(*eol_attr, (*r).attr_id);
            }
        }
        has_virt_pos
    }
}

#[inline]
unsafe fn decor_virt_line_wrap(wp: *mut win_T, overflow: VirtLineOverflow) -> bool {
    unsafe {
        overflow == kVLOverflowWrap
            || (overflow == kVLOverflowAuto && (*wp).w_onebuf_opt.wo_wrap != 0)
    }
}

pub unsafe fn decor_virt_line_rows(
    wp: *mut win_T,
    vl: &VirtLine,
    target_row: c_int,
    skip_cells: *mut c_int,
) -> c_int {
    unsafe {
        if !skip_cells.is_null() {
            *skip_cells = 0;
        }
        if !decor_virt_line_wrap(wp, vl.overflow) {
            return 1;
        }
        let row_width = (*wp).w_view_width
            - if (vl.flags & kVLLeftcol) != 0 {
                0
            } else {
                win_col_off(wp)
            };
        if row_width <= 0 {
            return 1;
        }
        let vt = &vl.line;
        let buf = (*wp).w_buffer;
        let mut vcol: c_int = 0;
        let mut row_cells: c_int = 0;
        let mut row: c_int = 0;
        for chunk in vt {
            let Some(ref text) = chunk.text else {
                continue;
            };
            let mut virt_str: *const c_char = text.as_ptr();
            while *virt_str != NUL {
                let bytes_to_next_char = utfc_ptr2len(virt_str);
                let cells = if *virt_str == TAB {
                    tabstop_padding(vcol, (*buf).b_p_ts, &(*buf).b_p_vts_array)
                } else {
                    utf_ptr2cells(virt_str)
                };
                virt_str = virt_str.add(bytes_to_next_char as usize);
                if row_cells + cells > row_width {
                    row += 1;
                    if !skip_cells.is_null() && row == target_row {
                        *skip_cells = vcol;
                    }
                    row_cells = 0;
                }
                row_cells += cells;
                vcol += cells;
            }
        }

        row + 1
    }
}

pub unsafe fn decor_virt_lines(
    wp: *mut win_T,
    start_row: c_int,
    end_row: c_int,
    num_below: *mut c_int,
    lines: *mut VirtLines,
    apply_folds: bool,
) -> c_int {
    unsafe {
        let buf = (*wp).w_buffer;
        if buf_meta_total(buf, kMTMetaLines) == 0 {
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

        let mut n_virt_lines: c_int = 0;
        loop {
            let mark = marktree_itr_current(b, &itr);
            let mut vt = mt_decor_virt(mark);
            if !mt_invalid(mark) && ns_in_win(mark.ns, wp) {
                while !vt.is_null() {
                    let virt_lines = (*vt).data.virt_lines();
                    if ((*vt).flags & kVTIsLines) != 0 && !virt_lines.is_empty() {
                        let above = ((*vt).flags & kVTLinesAbove) != 0;
                        let mrow = mark.pos.row;
                        let draw_row = mrow + if above { 0 } else { 1 };
                        if draw_row >= start_row
                            && draw_row < end_row
                            && (!apply_folds
                                || !(hasFolding(wp, mrow + 1, ptr::null_mut(), ptr::null_mut())
                                    || decor_conceal_line(wp, mrow, false)))
                        {
                            if decor_virt_line_wrap(wp, virt_lines[0].overflow) {
                                for item in virt_lines {
                                    let rows = decor_virt_line_rows(
                                        wp,
                                        item,
                                        0,
                                        ptr::null_mut(),
                                    );
                                    n_virt_lines += rows;
                                    if !num_below.is_null() && !above {
                                        *num_below += rows;
                                    }
                                }
                            } else {
                                n_virt_lines += virt_lines.len() as c_int;
                                if !num_below.is_null() && !above {
                                    *num_below += virt_lines.len() as c_int;
                                }
                            }
                            if !lines.is_null() {
                                (*lines).extend_from_slice(virt_lines);
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

pub unsafe fn decor_to_dict_legacy(
    dict: *mut Dict,
    decor: DecorInline,
    hl_name: bool,
    arena: *mut Arena,
) {
    unsafe {
        let inline_sh;
        let mut sh_hl = &DECOR_SIGN_HIGHLIGHT_INIT;
        let mut sh_sign = &DECOR_SIGN_HIGHLIGHT_INIT;
        let mut virt_text: *mut DecorVirtText = ptr::null_mut();
        let mut virt_lines: *mut DecorVirtText = ptr::null_mut();
        let mut priority: i32 = -1;

        if decor.is_ext() {
            let mut vt = decor.data.ext().vt;
            while !vt.is_null() {
                if ((*vt).flags & kVTIsLines) != 0 {
                    virt_lines = vt;
                } else {
                    virt_text = vt;
                }
                vt = (*vt).next_ptr();
            }

            let items = &raw const g_st().decor_items;
            let mut idx = decor.data.ext().sh_idx;
            while idx != DECOR_ID_INVALID {
                let sh = &(*items).as_slice()[idx as usize];
                let next = sh.next;
                if (sh.flags & kSHIsSign) != 0 {
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
            put_c(
                &mut *dict,
                c"hl_group".as_ptr(),
                hl_group_name(sh_hl.hl_id, hl_name),
            );
            put_c(
                &mut *dict,
                c"hl_eol".as_ptr(),
                boolean_obj((sh_hl.flags & kSHHlEol) != 0),
            );
            priority = i32::from(sh_hl.priority);
        }

        if (sh_hl.flags & kSHConcealOff) != 0 {
            put_c(&mut *dict, c"conceal".as_ptr(), boolean_obj(false));
        } else if (sh_hl.flags & kSHConceal) != 0 {
            let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
            schar_get(buf.as_mut_ptr(), sh_hl.text[0]);
            put_c(
                &mut *dict,
                c"conceal".as_ptr(),
                cstr_to_arena_obj(arena, buf.as_ptr()),
            );
        }

        if (sh_hl.flags & kSHConcealLines) != 0 {
            put_c(
                &mut *dict,
                c"conceal_lines".as_ptr(),
                cstr_as_obj(c"".as_ptr()),
            );
        }

        if (sh_hl.flags & kSHSpellOn) != 0 {
            put_c(&mut *dict, c"spell".as_ptr(), boolean_obj(true));
        } else if (sh_hl.flags & kSHSpellOff) != 0 {
            put_c(&mut *dict, c"spell".as_ptr(), boolean_obj(false));
        }

        if (sh_hl.flags & kSHUIWatched) != 0 {
            put_c(&mut *dict, c"ui_watched".as_ptr(), boolean_obj(true));
        }

        if let Some(ref url) = sh_hl.url {
            put_c(&mut *dict, c"url".as_ptr(), cstr_as_obj(url.as_ptr()));
        }

        if !virt_text.is_null() {
            if (*virt_text).hl_mode != 0 {
                let mode_str = hl_mode_str[(*virt_text).hl_mode as usize].as_ptr();
                put_c(&mut *dict, c"hl_mode".as_ptr(), cstr_as_obj(mode_str));
            }

            let chunks = virt_text_to_array((*virt_text).data.virt_text(), hl_name, arena);
            put_c(&mut *dict, c"virt_text".as_ptr(), array_obj(chunks));
            put_c(
                &mut *dict,
                c"virt_text_hide".as_ptr(),
                boolean_obj(((*virt_text).flags & kVTHide) != 0),
            );
            put_c(
                &mut *dict,
                c"virt_text_repeat_linebreak".as_ptr(),
                boolean_obj(((*virt_text).flags & kVTRepeatLinebreak) != 0),
            );
            if (*virt_text).pos == kVPosWinCol {
                put_c(
                    &mut *dict,
                    c"virt_text_win_col".as_ptr(),
                    integer_obj(Integer::from((*virt_text).col)),
                );
            }
            let pos_str = virt_text_pos_str[(*virt_text).pos as usize].as_ptr();
            put_c(&mut *dict, c"virt_text_pos".as_ptr(), cstr_as_obj(pos_str));
            priority = i32::from((*virt_text).priority);
        }

        if !virt_lines.is_null() {
            let vl_vec = (*virt_lines).data.virt_lines();
            let mut all_chunks = arena_array(arena, vl_vec.len());
            let mut virt_lines_flags: c_int = 0;
            let mut virt_lines_overflow: VirtLineOverflow = kVLOverflowTrunc;
            for vl in vl_vec {
                virt_lines_flags = vl.flags;
                virt_lines_overflow = vl.overflow;
                let chunks = virt_text_to_array(&vl.line, hl_name, arena);
                all_chunks.push(array_obj(chunks));
            }
            put_c(&mut *dict, c"virt_lines".as_ptr(), array_obj(all_chunks));
            put_c(
                &mut *dict,
                c"virt_lines_above".as_ptr(),
                boolean_obj(((*virt_lines).flags & kVTLinesAbove) != 0),
            );
            put_c(
                &mut *dict,
                c"virt_lines_leftcol".as_ptr(),
                boolean_obj((virt_lines_flags & kVLLeftcol) != 0),
            );

            let overflow = match virt_lines_overflow {
                kVLOverflowScroll => c"scroll".as_ptr(),
                kVLOverflowWrap => c"wrap".as_ptr(),
                kVLOverflowAuto => c"auto".as_ptr(),
                _ => c"trunc".as_ptr(),
            };
            put_c(
                &mut *dict,
                c"virt_lines_overflow".as_ptr(),
                cstr_as_obj(overflow),
            );
            priority = i32::from((*virt_lines).priority);
        }

        if (sh_sign.flags & kSHIsSign) != 0 {
            if sh_sign.text[0] != 0 {
                let mut buf = [0 as c_char; SIGN_WIDTH * MAX_SCHAR_SIZE];
                describe_sign_text(buf.as_mut_ptr(), sh_sign.text.as_ptr().cast_mut());
                put_c(
                    &mut *dict,
                    c"sign_text".as_ptr(),
                    cstr_to_arena_obj(arena, buf.as_ptr()),
                );
            }

            if let Some(ref sign_name) = sh_sign.sign_name {
                put_c(
                    &mut *dict,
                    c"sign_name".as_ptr(),
                    cstr_as_obj(sign_name.as_ptr()),
                );
            }

            let hls = [
                (c"sign_hl_group".as_ptr(), sh_sign.hl_id),
                (c"number_hl_group".as_ptr(), sh_sign.number_hl_id),
                (c"line_hl_group".as_ptr(), sh_sign.line_hl_id),
                (c"cursorline_hl_group".as_ptr(), sh_sign.cursorline_hl_id),
            ];
            for (name, val) in hls {
                if val != 0 {
                    put_c(&mut *dict, name, hl_group_name(val, hl_name));
                }
            }
            priority = i32::from(sh_sign.priority);
        }

        if priority != -1 {
            put_c(
                &mut *dict,
                c"priority".as_ptr(),
                integer_obj(Integer::from(priority)),
            );
        }
    }
}

pub unsafe fn decor_type_flags(decor: DecorInline) -> u16 {
    unsafe {
        if decor.is_ext() {
            let mut type_flags: u16 = kExtmarkNone;
            let mut vt = decor.data.ext().vt;
            while !vt.is_null() {
                type_flags |= if ((*vt).flags & kVTIsLines) != 0 {
                    kExtmarkVirtLines
                } else {
                    kExtmarkVirtText
                };
                vt = (*vt).next_ptr();
            }
            let items = &raw const g_st().decor_items;
            let mut idx = decor.data.ext().sh_idx;
            while idx != DECOR_ID_INVALID {
                let sh = &(*items).as_slice()[idx as usize];
                type_flags |= if (sh.flags & kSHIsSign) != 0 {
                    kExtmarkSign
                } else {
                    kExtmarkHighlight
                };
                idx = sh.next;
            }
            type_flags
        } else if (decor.data.hl().flags & kSHIsSign) != 0 {
            kExtmarkSign
        } else {
            kExtmarkHighlight
        }
    }
}

pub unsafe fn hl_group_name(hl_id: c_int, hl_name: bool) -> Object {
    unsafe {
        if hl_name {
            cstr_as_obj(syn_id2name(hl_id))
        } else {
            integer_obj(Integer::from(hl_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decor_sh_from_inline_and_type_flags_work() {
        unsafe {
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
            assert_eq!(decor_type_flags(di), kExtmarkHighlight);
        }
    }
}
