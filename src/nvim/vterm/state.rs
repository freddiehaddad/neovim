//! VTerm terminal state machine (`src/nvim/vterm/state.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::ptr;

use crate::grid::{schar_T, schar_from_buf};
use crate::plines::{CharInfo, ClusterInfo, StrCharInfo};
use crate::vterm::encoding::{VTermEncoding, VTermEncodingType, vterm_lookup_encoding};
use crate::vterm::keyboard::VTermStateFull;
use crate::vterm::mouse::{VTerm, VTermPos};
use crate::vterm::parser::{
    VTermParserCallbacks, VTermStringFragment, VTermTerminator, vterm_parser_set_callbacks,
};
use crate::vterm::pen::{
    VTermProp, VTermRect, VTermStateCallbacks, VTermValue, vterm_state_getpen, vterm_state_newpen,
    vterm_state_resetpen, vterm_state_savepen, vterm_state_setpen,
};
use crate::vterm::screen::{VTermGlyphInfo, VTermLineInfo, VTermStateFields};
use crate::vterm::vterm::{
    vterm_allocator_free, vterm_allocator_malloc, vterm_push_output_bytes,
    vterm_push_output_ctrl_bytes, vterm_push_output_str_bytes, vterm_scroll_rect,
};

pub type VTermSelectionMask = c_int;

const BUFIDX_PRIMARY: usize = 0;
const BUFIDX_ALTSCREEN: usize = 1;

const C1_DCS: u8 = 0x90;
const C1_CSI: u8 = 0x9b;

const VTERM_VERSION_MAJOR: c_int = 0;
const VTERM_VERSION_MINOR: c_int = 3;

const KEY_ENCODING_DISAMBIGUATE: c_int = 0x01;
const KEY_ENCODING_REPORT_EVENTS: c_int = 0x02;
const KEY_ENCODING_REPORT_ALTERNATE: c_int = 0x04;
const KEY_ENCODING_REPORT_ALL_KEYS: c_int = 0x08;
const KEY_ENCODING_REPORT_ASSOCIATED: c_int = 0x10;

const MOUSE_WANT_CLICK: c_int = 0x01;
const MOUSE_WANT_DRAG: c_int = 0x02;
const MOUSE_WANT_MOVE: c_int = 0x04;

const MOUSE_X10: c_int = 0;
const MOUSE_UTF8: c_int = 1;
const MOUSE_SGR: c_int = 2;
const MOUSE_RXVT: c_int = 3;

const VTERM_PROP_CURSORVISIBLE: VTermProp = 1;
const VTERM_PROP_CURSORBLINK: VTermProp = 2;
const VTERM_PROP_ALTSCREEN: VTermProp = 3;
const VTERM_PROP_TITLE: VTermProp = 4;
const VTERM_PROP_ICONNAME: VTermProp = 5;
const VTERM_PROP_REVERSE: VTermProp = 6;
const VTERM_PROP_CURSORSHAPE: VTermProp = 7;
const VTERM_PROP_MOUSE: VTermProp = 8;
const VTERM_PROP_FOCUSREPORT: VTermProp = 9;
const VTERM_PROP_THEMEUPDATES: VTermProp = 10;
const VTERM_PROP_SYNCOUTPUT: VTermProp = 11;
const VTERM_N_PROPS: VTermProp = 12;

const VTERM_PROP_CURSORSHAPE_BLOCK: c_int = 1;
const VTERM_PROP_CURSORSHAPE_UNDERLINE: c_int = 2;
const VTERM_PROP_CURSORSHAPE_BAR_LEFT: c_int = 3;

const VTERM_PROP_MOUSE_NONE: c_int = 0;
const VTERM_PROP_MOUSE_CLICK: c_int = 1;
const VTERM_PROP_MOUSE_DRAG: c_int = 2;
const VTERM_PROP_MOUSE_MOVE: c_int = 3;

const VTERM_SELECTION_CLIPBOARD: u16 = 1 << 0;
const VTERM_SELECTION_PRIMARY: u16 = 1 << 1;
const VTERM_SELECTION_SECONDARY: u16 = 1 << 2;
const VTERM_SELECTION_SELECT: u16 = 1 << 3;
const VTERM_SELECTION_CUT0: u16 = 1 << 4;

const SELECTION_INITIAL: u8 = 0;
const SELECTION_SELECTED: u8 = 1;
const SELECTION_QUERY: u8 = 2;
const SELECTION_SET_INITIAL: u8 = 3;
const SELECTION_SET: u8 = 4;
const SELECTION_INVALID: u8 = 5;

const NO_FORCE: c_int = 0;
const FORCE: c_int = 1;

const DWL_OFF: c_int = 0;
const DWL_ON: c_int = 1;

const DHL_OFF: c_int = 0;
const DHL_TOP: c_int = 1;
const DHL_BOTTOM: c_int = 2;

const MODE_KEYPAD: u32 = 1 << 0;
const MODE_CURSOR: u32 = 1 << 1;
const MODE_AUTOWRAP: u32 = 1 << 2;
const MODE_INSERT: u32 = 1 << 3;
const MODE_NEWLINE: u32 = 1 << 4;
const MODE_CURSOR_VISIBLE: u32 = 1 << 5;
const MODE_CURSOR_BLINK: u32 = 1 << 6;
const MODE_CURSOR_SHAPE_SHIFT: u32 = 7;
const MODE_CURSOR_SHAPE_MASK: u32 = 0x03 << 7;
const MODE_ALT_SCREEN: u32 = 1 << 9;
const MODE_ORIGIN: u32 = 1 << 10;
const MODE_SCREEN: u32 = 1 << 11;
const MODE_LEFTRIGHTMARGIN: u32 = 1 << 12;
const MODE_BRACKETPASTE: u32 = 1 << 13;
const MODE_REPORT_FOCUS: u32 = 1 << 14;
const MODE_THEME_UPDATES: u32 = 1 << 15;
const MODE_SYNCHRONIZED_OUTPUT: u32 = 1 << 16;

const VT_MODE_UTF8: u32 = 1 << 0;
const VT_MODE_CTRL8BIT: u32 = 1 << 1;

const CSI_ARG_FLAG_MORE_U32: u32 = 1u32 << 31;
const CSI_ARG_MASK_U32: u32 = !(1u32 << 31);
const CSI_ARG_MISSING: c_int = ((1u32 << 31) - 1) as c_int;

const GRAPHEME_STATE_INIT: i32 = 0;

/// Primary Device Attributes (DA1) response.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vterm_primary_device_attr: [c_char; 9] = [
    b'6' as c_char,
    b'1' as c_char,
    b';' as c_char,
    b'2' as c_char,
    b'2' as c_char,
    b';' as c_char,
    b'5' as c_char,
    b'2' as c_char,
    0,
];

#[repr(C)]
pub struct VTermStateFallbacks {
    pub control: Option<unsafe extern "C" fn(control: u8, user: *mut c_void) -> c_int>,
    pub csi: Option<
        unsafe extern "C" fn(
            leader: *const c_char,
            args: *const c_long,
            argcount: c_int,
            intermed: *const c_char,
            command: c_char,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub osc: Option<
        unsafe extern "C" fn(command: c_int, frag: VTermStringFragment, user: *mut c_void) -> c_int,
    >,
    pub dcs: Option<
        unsafe extern "C" fn(
            command: *const c_char,
            commandlen: usize,
            frag: VTermStringFragment,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub apc: Option<unsafe extern "C" fn(frag: VTermStringFragment, user: *mut c_void) -> c_int>,
    pub pm: Option<unsafe extern "C" fn(frag: VTermStringFragment, user: *mut c_void) -> c_int>,
    pub sos: Option<unsafe extern "C" fn(frag: VTermStringFragment, user: *mut c_void) -> c_int>,
}

#[repr(C)]
pub struct VTermSelectionCallbacks {
    pub set: Option<
        unsafe extern "C" fn(
            mask: VTermSelectionMask,
            frag: VTermStringFragment,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub query: Option<unsafe extern "C" fn(mask: VTermSelectionMask, user: *mut c_void) -> c_int>,
}

#[cfg(not(test))]
unsafe extern "C" {
    static mut dy_escape_width: c_int;
    static utf8len_tab: [u8; 256];

    fn utf_iscomposing(c1: c_int, c2: c_int, state: *mut i32) -> bool;
    fn utf_char2bytes(c: c_int, buf: *mut c_char) -> c_int;
    fn utf_char2cells(c: c_int) -> c_int;
    fn utf_ptr2CharInfo_impl(p: *const u8, len: usize) -> i32;
    fn utf_ClusterInfo_impl(base: StrCharInfo, max_len: *mut c_int) -> ClusterInfo;
}

#[cfg(test)]
unsafe fn utf_iscomposing(_c1: c_int, _c2: c_int, _state: *mut i32) -> bool {
    false
}
#[cfg(test)]
unsafe fn utf_char2bytes(_c: c_int, _buf: *mut c_char) -> c_int {
    0
}
#[cfg(test)]
unsafe fn utf_char2cells(_c: c_int) -> c_int {
    1
}
#[cfg(test)]
unsafe fn utf_ptr2CharInfo_impl(_p: *const u8, _len: usize) -> i32 {
    0
}
#[cfg(test)]
unsafe fn utf_ClusterInfo_impl(base: StrCharInfo, _max_len: *mut c_int) -> ClusterInfo {
    ClusterInfo {
        next: base,
        cells: 1,
    }
}

#[inline]
const fn schar_from_ascii(x: u8) -> schar_T {
    #[cfg(target_endian = "big")]
    {
        (x as schar_T) << 24
    }
    #[cfg(not(target_endian = "big"))]
    {
        x as schar_T
    }
}

#[inline]
unsafe fn ascii2cells(b: c_int) -> c_int {
    if (b' ' as c_int..=b'~' as c_int).contains(&b) {
        1
    } else {
        #[cfg(not(test))]
        {
            unsafe { dy_escape_width }
        }
        #[cfg(test)]
        {
            2
        }
    }
}

#[inline]
unsafe fn basechar_cells_impl(c: CharInfo) -> c_int {
    unsafe {
        if c.value < 0 {
            4
        } else if c.value < 0x80 {
            ascii2cells(c.value)
        } else {
            utf_char2cells(c.value)
        }
    }
}

#[inline]
unsafe fn utf_ptr2CharInfo_len(p_in: *const c_char, size: c_int) -> CharInfo {
    unsafe {
        let p = p_in.cast::<u8>();
        debug_assert!(size >= 0);
        if size == 0 {
            return CharInfo { value: -1, len: 1 };
        }
        let first = *p;
        if first < 0x80 {
            CharInfo {
                value: i32::from(first),
                len: 1,
            }
        } else {
            #[cfg(not(test))]
            let mut len = c_int::from(utf8len_tab[first as usize]);
            #[cfg(test)]
            let mut len = 1;
            if len > size {
                return CharInfo { value: -1, len: 1 };
            }
            let code_point = utf_ptr2CharInfo_impl(p, len as usize);
            if code_point < 0 {
                len = 1;
            }
            CharInfo {
                value: code_point,
                len,
            }
        }
    }
}

#[inline]
unsafe fn utf_ptr2StrCharInfo_len(ptr: *const c_char, size: c_int) -> StrCharInfo {
    unsafe {
        StrCharInfo {
            ptr,
            chr: utf_ptr2CharInfo_len(ptr, size),
        }
    }
}

#[inline]
unsafe fn utf_ClusterInfo_len(cur: StrCharInfo, size: *mut c_int) -> ClusterInfo {
    unsafe {
        *size -= cur.chr.len;
        let next = cur.ptr.add(cur.chr.len as usize).cast::<u8>();
        if *size == 0 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo { value: -1, len: 1 },
                },
                cells: basechar_cells_impl(cur.chr),
            };
        }
        if *next < 0x80 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo {
                        value: i32::from(*next),
                        len: 1,
                    },
                },
                cells: basechar_cells_impl(cur.chr),
            };
        }
        utf_ClusterInfo_impl(cur, size)
    }
}

#[inline]
const fn csi_arg(a: c_long) -> c_int {
    ((a as u32) & CSI_ARG_MASK_U32) as c_int
}

#[inline]
const fn csi_arg_has_more(a: c_long) -> bool {
    ((a as u32) & CSI_ARG_FLAG_MORE_U32) != 0
}

#[inline]
const fn csi_arg_is_missing(a: c_long) -> bool {
    csi_arg(a) == CSI_ARG_MISSING
}

#[inline]
const fn csi_arg_or(a: c_long, def: c_int) -> c_int {
    if csi_arg_is_missing(a) {
        def
    } else {
        csi_arg(a)
    }
}

#[inline]
const fn csi_arg_count(a: c_long) -> c_int {
    let v = csi_arg(a);
    if v == CSI_ARG_MISSING || v == 0 { 1 } else { v }
}

#[inline]
unsafe fn scrollregion_bottom(state: *const VTermStateFull) -> c_int {
    unsafe {
        if (*state).scrollregion_bottom > -1 {
            (*state).scrollregion_bottom
        } else {
            (*state).rows
        }
    }
}

#[inline]
unsafe fn scrollregion_left(state: *const VTermStateFull) -> c_int {
    unsafe {
        if ((*state).mode & MODE_LEFTRIGHTMARGIN) != 0 {
            (*state).scrollregion_left
        } else {
            0
        }
    }
}

#[inline]
unsafe fn scrollregion_right(state: *const VTermStateFull) -> c_int {
    unsafe {
        if ((*state).mode & MODE_LEFTRIGHTMARGIN) != 0 && (*state).scrollregion_right > -1 {
            (*state).scrollregion_right
        } else {
            (*state).cols
        }
    }
}

#[inline]
unsafe fn rowwidth(state: *const VTermStateFull, row: c_int) -> c_int {
    unsafe {
        let lineinfo = (*state).lineinfo.cast::<VTermLineInfo>();
        if (*lineinfo.add(row as usize)).doublewidth() != 0 {
            (*state).cols / 2
        } else {
            (*state).cols
        }
    }
}

#[inline]
unsafe fn thisrowwidth(state: *const VTermStateFull) -> c_int {
    unsafe { rowwidth(state, (*state).pos.row) }
}

#[inline]
fn set_mode_bit(mode: &mut u32, bit: u32, val: bool) {
    if val {
        *mode |= bit;
    } else {
        *mode &= !bit;
    }
}

#[inline]
fn get_cursor_shape(mode: u32) -> u32 {
    (mode >> MODE_CURSOR_SHAPE_SHIFT) & 0x03
}

#[inline]
fn set_cursor_shape(mode: &mut u32, shape: u32) {
    *mode = (*mode & !MODE_CURSOR_SHAPE_MASK) | ((shape & 0x03) << MODE_CURSOR_SHAPE_SHIFT);
}

#[inline]
unsafe fn tmp_selection_mask(state: *const VTermStateFull) -> u16 {
    unsafe { ((*state).tmp.words[0] & 0xffff) as u16 }
}

#[inline]
unsafe fn set_tmp_selection_mask(state: *mut VTermStateFull, mask: u16) {
    unsafe {
        (*state).tmp.words[0] = ((*state).tmp.words[0] & !0xffff) | u32::from(mask);
    }
}

#[inline]
unsafe fn tmp_selection_state(state: *const VTermStateFull) -> u8 {
    unsafe { (((*state).tmp.words[0] >> 16) & 0xff) as u8 }
}

#[inline]
unsafe fn set_tmp_selection_state(state: *mut VTermStateFull, st: u8) {
    unsafe {
        (*state).tmp.words[0] = ((*state).tmp.words[0] & !(0xff << 16)) | (u32::from(st) << 16);
    }
}

#[inline]
unsafe fn tmp_selection_recvpartial(state: *const VTermStateFull) -> u32 {
    unsafe { (*state).tmp.words[1] }
}

#[inline]
unsafe fn set_tmp_selection_recvpartial(state: *mut VTermStateFull, val: u32) {
    unsafe {
        (*state).tmp.words[1] = val;
    }
}

unsafe fn putglyph(state: *mut VTermStateFull, schar: schar_T, width: c_int, pos: VTermPos) {
    unsafe {
        let lineinfo = (*state).lineinfo.cast::<VTermLineInfo>();
        let li = *lineinfo.add(pos.row as usize);
        let flags = ((*state).protected_cell & 0x01)
            | ((li.doublewidth() & 0x01) << 1)
            | ((li.doubleheight() & 0x03) << 2);
        let mut info = VTermGlyphInfo {
            schar,
            width,
            flags,
        };

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).putglyph
        {
            let _ = cb((&raw mut info).cast(), pos, (*state).cbdata);
        }
    }
}

unsafe fn updatecursor(state: *mut VTermStateFull, oldpos: *const VTermPos, cancel_phantom: c_int) {
    unsafe {
        if (*state).pos.col == (*oldpos).col && (*state).pos.row == (*oldpos).row {
            return;
        }

        if cancel_phantom != 0 {
            (*state).at_phantom = 0;
        }

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).movecursor
        {
            let visible = c_int::from(((*state).mode & MODE_CURSOR_VISIBLE) != 0);
            let _ = cb((*state).pos, *oldpos, visible, (*state).cbdata);
        }
    }
}

unsafe fn erase(state: *mut VTermStateFull, rect: VTermRect, selective: c_int) {
    unsafe {
        if rect.end_col == (*state).cols {
            let lineinfo = (*state).lineinfo.cast::<VTermLineInfo>();
            let mut row = rect.start_row + 1;
            while row < rect.end_row + 1 && row < (*state).rows {
                (*lineinfo.add(row as usize)).set_continuation(false);
                row += 1;
            }
        }

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).erase
        {
            let _ = cb(rect, selective, (*state).cbdata);
        }
    }
}

unsafe fn vterm_state_new(vt: *mut VTerm) -> *mut VTermStateFull {
    unsafe {
        let state =
            vterm_allocator_malloc(vt, size_of::<VTermStateFull>()).cast::<VTermStateFull>();

        (*state).vt = vt;

        (*state).rows = (*vt).rows;
        (*state).cols = (*vt).cols;

        (*state).mouse_col = 0;
        (*state).mouse_row = 0;
        (*state).mouse_buttons = 0;

        (*state).mouse_protocol = MOUSE_X10;

        (*state).callbacks = ptr::null();
        (*state).cbdata = ptr::null_mut();

        (*state).selection.callbacks = ptr::null();
        (*state).selection.user = ptr::null_mut();
        (*state).selection.buffer = ptr::null_mut();

        vterm_state_newpen(state);

        (*state).bold_is_highbright = 0;

        (*state).combine_pos.row = -1;

        (*state).tabstops =
            vterm_allocator_malloc((*state).vt, ((*state).cols as usize).div_ceil(8)).cast::<u8>();

        (*state).lineinfos[BUFIDX_PRIMARY] = vterm_allocator_malloc(
            (*state).vt,
            ((*state).rows as usize) * size_of::<VTermLineInfo>(),
        );
        (*state).lineinfos[BUFIDX_ALTSCREEN] = vterm_allocator_malloc(
            (*state).vt,
            ((*state).rows as usize) * size_of::<VTermLineInfo>(),
        );
        (*state).lineinfo = (*state).lineinfos[BUFIDX_PRIMARY];

        let utf8_enc = vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'u' as c_char);
        (*state).encoding_utf8.enc = utf8_enc.cast();
        if let Some(init_fn) = (*utf8_enc).init {
            init_fn(utf8_enc, (*state).encoding_utf8.data.as_mut_ptr().cast());
        }

        for stack in &mut (*state).key_encoding_stacks {
            stack.items = [0; 16];
            stack.size = 1;
        }

        state
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_free(state: *mut VTermStateFull) {
    unsafe {
        vterm_allocator_free((*state).vt, (*state).tabstops.cast());
        vterm_allocator_free((*state).vt, (*state).lineinfos[BUFIDX_PRIMARY]);
        if !(*state).lineinfos[BUFIDX_ALTSCREEN].is_null() {
            vterm_allocator_free((*state).vt, (*state).lineinfos[BUFIDX_ALTSCREEN]);
        }
        vterm_allocator_free((*state).vt, state.cast());
    }
}

unsafe fn scroll(
    state: *mut VTermStateFull,
    rect: VTermRect,
    mut downward: c_int,
    mut rightward: c_int,
) {
    unsafe {
        if downward == 0 && rightward == 0 {
            return;
        }

        let rows = rect.end_row - rect.start_row;
        if downward > rows {
            downward = rows;
        } else if downward < -rows {
            downward = -rows;
        }

        let cols = rect.end_col - rect.start_col;
        if rightward > cols {
            rightward = cols;
        } else if rightward < -cols {
            rightward = -cols;
        }

        // Update lineinfo if full line
        if rect.start_col == 0 && rect.end_col == (*state).cols && rightward == 0 {
            let height = rect.end_row - rect.start_row - downward.abs();
            let lineinfo = (*state).lineinfo.cast::<VTermLineInfo>();

            if downward > 0 {
                ptr::copy(
                    lineinfo.add((rect.start_row + downward) as usize),
                    lineinfo.add(rect.start_row as usize),
                    height as usize,
                );
                for row in (rect.end_row - downward)..rect.end_row {
                    *lineinfo.add(row as usize) = VTermLineInfo::default();
                }
            } else {
                ptr::copy(
                    lineinfo.add(rect.start_row as usize),
                    lineinfo.add((rect.start_row - downward) as usize),
                    height as usize,
                );
                for row in rect.start_row..(rect.start_row - downward) {
                    *lineinfo.add(row as usize) = VTermLineInfo::default();
                }
            }
        }

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).scrollrect
            && cb(rect, downward, rightward, (*state).cbdata) != 0
        {
            return;
        }

        if !callbacks.is_null() {
            vterm_scroll_rect(
                rect,
                downward,
                rightward,
                (*callbacks).moverect,
                (*callbacks).erase,
                (*state).cbdata,
            );
        }
    }
}

unsafe fn linefeed(state: *mut VTermStateFull) {
    unsafe {
        if (*state).pos.row == scrollregion_bottom(state) - 1 {
            let rect = VTermRect {
                start_row: (*state).scrollregion_top,
                end_row: scrollregion_bottom(state),
                start_col: scrollregion_left(state),
                end_col: scrollregion_right(state),
            };

            scroll(state, rect, 1, 0);
        } else if (*state).pos.row < (*state).rows - 1 {
            (*state).pos.row += 1;
        }
    }
}

#[inline]
unsafe fn set_col_tabstop(state: *mut VTermStateFull, col: c_int) {
    unsafe {
        let mask = 1u8 << (col & 7);
        *(*state).tabstops.add((col >> 3) as usize) |= mask;
    }
}

#[inline]
unsafe fn clear_col_tabstop(state: *mut VTermStateFull, col: c_int) {
    unsafe {
        let mask = 1u8 << (col & 7);
        *(*state).tabstops.add((col >> 3) as usize) &= !mask;
    }
}

#[inline]
unsafe fn is_col_tabstop(state: *const VTermStateFull, col: c_int) -> bool {
    unsafe {
        let mask = 1u8 << (col & 7);
        (*(*state).tabstops.add((col >> 3) as usize) & mask) != 0
    }
}

#[inline]
unsafe fn is_cursor_in_scrollregion(state: *const VTermStateFull) -> bool {
    unsafe {
        if (*state).pos.row < (*state).scrollregion_top
            || (*state).pos.row >= scrollregion_bottom(state)
        {
            return false;
        }
        if (*state).pos.col < scrollregion_left(state)
            || (*state).pos.col >= scrollregion_right(state)
        {
            return false;
        }

        true
    }
}

unsafe fn tab(state: *mut VTermStateFull, mut count: c_int, direction: c_int) {
    unsafe {
        while count > 0 {
            if direction > 0 {
                if (*state).pos.col >= thisrowwidth(state) - 1 {
                    return;
                }
                (*state).pos.col += 1;
            } else if direction < 0 {
                if (*state).pos.col < 1 {
                    return;
                }
                (*state).pos.col -= 1;
            }

            if is_col_tabstop(state, (*state).pos.col) {
                count -= 1;
            }
        }
    }
}

unsafe fn set_lineinfo(
    state: *mut VTermStateFull,
    row: c_int,
    force: c_int,
    dwl: c_int,
    dhl: c_int,
) {
    unsafe {
        let lineinfo = (*state).lineinfo.cast::<VTermLineInfo>();
        let mut info = *lineinfo.add(row as usize);

        if dwl == DWL_OFF {
            info.set_doublewidth(DWL_OFF as u32);
        } else if dwl == DWL_ON {
            info.set_doublewidth(DWL_ON as u32);
        }

        if dhl == DHL_OFF {
            info.set_doubleheight(DHL_OFF as u32);
        } else if dhl == DHL_TOP {
            info.set_doubleheight(DHL_TOP as u32);
        } else if dhl == DHL_BOTTOM {
            info.set_doubleheight(DHL_BOTTOM as u32);
        }

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        let cb_ok = !callbacks.is_null()
            && (*callbacks).setlineinfo.is_some_and(|cb| {
                cb(
                    row,
                    (&raw const info).cast(),
                    lineinfo.add(row as usize).cast(),
                    (*state).cbdata,
                ) != 0
            });

        if cb_ok || force != 0 {
            *lineinfo.add(row as usize) = info;
        }
    }
}

unsafe extern "C" fn on_text(bytes: *const c_char, len: usize, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let oldpos = (*state).pos;

        let codepoints = (*(*state).vt).tmpbuffer.cast::<u32>();
        let maxpoints = (*(*state).vt).tmpbuffer_len / size_of::<u32>();

        let mut npoints: c_int = 0;
        let mut eaten: usize = 0;

        let mut encoding = if (*state).gsingle_set != 0 {
            &raw mut (*state).encoding[(*state).gsingle_set as usize]
        } else if ((*bytes.add(eaten) as u8) & 0x80) == 0 {
            &raw mut (*state).encoding[(*state).gl_set as usize]
        } else if ((*(*state).vt).mode & VT_MODE_UTF8) != 0 {
            &raw mut (*state).encoding_utf8
        } else {
            &raw mut (*state).encoding[(*state).gr_set as usize]
        };

        if (*encoding).enc == (*state).encoding_utf8.enc {
            encoding = &raw mut (*state).encoding_utf8;
        }

        let enc = (*encoding).enc.cast::<VTermEncoding>();
        if let Some(decode_fn) = (*enc).decode {
            decode_fn(
                enc,
                (*encoding).data.as_mut_ptr().cast(),
                codepoints,
                &raw mut npoints,
                if (*state).gsingle_set != 0 {
                    1
                } else {
                    maxpoints as c_int
                },
                bytes,
                &raw mut eaten,
                len,
            );
        }

        if npoints == 0 {
            return eaten as c_int;
        }

        if (*state).gsingle_set != 0 && npoints != 0 {
            (*state).gsingle_set = 0;
        }

        let mut i: usize = 0;
        let mut grapheme_state: i32 = GRAPHEME_STATE_INIT;
        let mut grapheme_len: usize = 0;
        let mut recombine = false;

        if (*state).pos.row == (*state).combine_pos.row
            && (*state).pos.col >= (*state).combine_pos.col
            && (*state).pos.col <= (*state).combine_pos.col + (*state).combine_width
            && utf_iscomposing(
                (*state).grapheme_last as c_int,
                *codepoints.add(i) as c_int,
                &raw mut (*state).grapheme_state,
            )
        {
            grapheme_len = (*state).grapheme_len;
            grapheme_state = (*state).grapheme_state;
            (*state).pos.col = (*state).combine_pos.col;
            (*state).at_phantom = 0;
            recombine = true;
        }

        while i < npoints as usize {
            loop {
                if grapheme_len < (*state).grapheme_buf.len() - 4 {
                    grapheme_len += utf_char2bytes(
                        *codepoints.add(i) as c_int,
                        (*state).grapheme_buf.as_mut_ptr().add(grapheme_len),
                    ) as usize;
                }
                i += 1;
                if i >= npoints as usize
                    || !utf_iscomposing(
                        *codepoints.add(i - 1) as c_int,
                        *codepoints.add(i) as c_int,
                        &raw mut grapheme_state,
                    )
                {
                    break;
                }
            }

            let mut scratch = grapheme_len as c_int;
            let ci = utf_ptr2StrCharInfo_len((*state).grapheme_buf.as_ptr(), scratch);
            let cli = utf_ClusterInfo_len(ci, &raw mut scratch);
            let width = cli.cells;

            if (*state).at_phantom != 0 || (*state).pos.col + width > thisrowwidth(state) {
                linefeed(state);
                (*state).pos.col = 0;
                (*state).at_phantom = 0;
                let lineinfo = (*state).lineinfo.cast::<VTermLineInfo>();
                (*lineinfo.add((*state).pos.row as usize)).set_continuation(true);
            }

            if ((*state).mode & MODE_INSERT) != 0 && !recombine {
                let rect = VTermRect {
                    start_row: (*state).pos.row,
                    end_row: (*state).pos.row + 1,
                    start_col: (*state).pos.col,
                    end_col: thisrowwidth(state),
                };
                scroll(state, rect, 0, -1);
            }

            let sc = schar_from_buf((*state).grapheme_buf.as_ptr(), grapheme_len);
            putglyph(state, sc, width, (*state).pos);

            if i == npoints as usize {
                (*state).grapheme_len = grapheme_len;
                (*state).grapheme_last = *codepoints.add(i - 1);
                (*state).grapheme_state = grapheme_state;
                (*state).combine_width = width;
                (*state).combine_pos = (*state).pos;
            } else {
                grapheme_len = 0;
                recombine = false;
            }

            if (*state).pos.col + width >= thisrowwidth(state) {
                if ((*state).mode & MODE_AUTOWRAP) != 0 {
                    (*state).at_phantom = 1;
                }
            } else {
                (*state).pos.col += width;
            }
        }

        updatecursor(state, &raw const oldpos, 0);

        eaten as c_int
    }
}

unsafe extern "C" fn on_control(control: u8, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let oldpos = (*state).pos;

        match control {
            0x07 => {
                // BEL - ECMA-48 8.3.3
                let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).bell
                {
                    let _ = cb((*state).cbdata);
                }
            }
            0x08 => {
                // BS - ECMA-48 8.3.5
                if (*state).pos.col > 0 {
                    (*state).pos.col -= 1;
                }
            }
            0x09 => {
                // HT - ECMA-48 8.3.60
                tab(state, 1, 1);
            }
            0x0a..=0x0c => {
                // LF, VT, FF - ECMA-48 8.3.74
                linefeed(state);
                if ((*state).mode & MODE_NEWLINE) != 0 {
                    (*state).pos.col = 0;
                }
            }
            0x0d => {
                // CR - ECMA-48 8.3.15
                (*state).pos.col = 0;
            }
            0x0e => {
                // LS1 - ECMA-48 8.3.76
                (*state).gl_set = 1;
            }
            0x0f => {
                // LS0 - ECMA-48 8.3.75
                (*state).gl_set = 0;
            }
            0x84 => {
                // IND
                linefeed(state);
            }
            0x85 => {
                // NEL - ECMA-48 8.3.86
                linefeed(state);
                (*state).pos.col = 0;
            }
            0x88 => {
                // HTS - ECMA-48 8.3.62
                set_col_tabstop(state, (*state).pos.col);
            }
            0x8d => {
                // RI - ECMA-48 8.3.104
                if (*state).pos.row == (*state).scrollregion_top {
                    let rect = VTermRect {
                        start_row: (*state).scrollregion_top,
                        end_row: scrollregion_bottom(state),
                        start_col: scrollregion_left(state),
                        end_col: scrollregion_right(state),
                    };

                    scroll(state, rect, -1, 0);
                } else if (*state).pos.row > 0 {
                    (*state).pos.row -= 1;
                }
            }
            0x8e => {
                // SS2 - ECMA-48 8.3.141
                (*state).gsingle_set = 2;
            }
            0x8f => {
                // SS3 - ECMA-48 8.3.142
                (*state).gsingle_set = 3;
            }
            _ => {
                let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
                if !fallbacks.is_null()
                    && let Some(cb) = (*fallbacks).control
                    && cb(control, (*state).fbdata) != 0
                {
                    return 1;
                }
                return 0;
            }
        }

        updatecursor(state, &raw const oldpos, 1);

        1
    }
}

#[inline]
unsafe fn settermprop_bool(state: *mut VTermStateFull, prop: VTermProp, v: c_int) -> c_int {
    unsafe {
        let mut val = VTermValue { boolean: v };
        vterm_state_set_termprop(state, prop, &raw mut val)
    }
}

#[inline]
unsafe fn settermprop_int(state: *mut VTermStateFull, prop: VTermProp, v: c_int) -> c_int {
    unsafe {
        let mut val = VTermValue { number: v };
        vterm_state_set_termprop(state, prop, &raw mut val)
    }
}

#[inline]
unsafe fn settermprop_string(
    state: *mut VTermStateFull,
    prop: VTermProp,
    frag: VTermStringFragment,
) -> c_int {
    unsafe {
        let mut val = VTermValue { string: frag };
        vterm_state_set_termprop(state, prop, &raw mut val)
    }
}

unsafe fn savecursor(state: *mut VTermStateFull, save: c_int) {
    unsafe {
        if save != 0 {
            (*state).saved.pos = (*state).pos;
            let cv = u32::from(((*state).mode & MODE_CURSOR_VISIBLE) != 0);
            let cb = u32::from(((*state).mode & MODE_CURSOR_BLINK) != 0);
            let cs = get_cursor_shape((*state).mode);
            (*state).saved.mode = (cv & 0x01) | ((cb & 0x01) << 1) | ((cs & 0x03) << 2);

            vterm_state_savepen(state, 1);
        } else {
            let oldpos = (*state).pos;

            (*state).pos = (*state).saved.pos;

            let cv = ((*state).saved.mode & 0x01) as c_int;
            let cb = (((*state).saved.mode >> 1) & 0x01) as c_int;
            let cs = (((*state).saved.mode >> 2) & 0x03) as c_int;

            let _ = settermprop_bool(state, VTERM_PROP_CURSORVISIBLE, cv);
            let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, cb);
            let _ = settermprop_int(state, VTERM_PROP_CURSORSHAPE, cs);

            vterm_state_savepen(state, 0);

            updatecursor(state, &raw const oldpos, 1);
        }
    }
}

unsafe extern "C" fn on_escape(bytes: *const c_char, len: usize, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();

        match *bytes as u8 {
            b' ' => {
                if len != 2 {
                    return 0;
                }
                match *bytes.add(1) as u8 {
                    b'F' => {
                        // S7C1T
                        (*(*state).vt).mode &= !VT_MODE_CTRL8BIT;
                    }
                    b'G' => {
                        // S8C1T
                        (*(*state).vt).mode |= VT_MODE_CTRL8BIT;
                    }
                    _ => return 0,
                }
                2
            }
            b'#' => {
                if len != 2 {
                    return 0;
                }
                match *bytes.add(1) as u8 {
                    b'3' => {
                        // DECDHL top
                        if ((*state).mode & MODE_LEFTRIGHTMARGIN) == 0 {
                            set_lineinfo(state, (*state).pos.row, NO_FORCE, DWL_ON, DHL_TOP);
                        }
                    }
                    b'4' => {
                        // DECDHL bottom
                        if ((*state).mode & MODE_LEFTRIGHTMARGIN) == 0 {
                            set_lineinfo(state, (*state).pos.row, NO_FORCE, DWL_ON, DHL_BOTTOM);
                        }
                    }
                    b'5' => {
                        // DECSWL
                        if ((*state).mode & MODE_LEFTRIGHTMARGIN) == 0 {
                            set_lineinfo(state, (*state).pos.row, NO_FORCE, DWL_OFF, DHL_OFF);
                        }
                    }
                    b'6' => {
                        // DECDWL
                        if ((*state).mode & MODE_LEFTRIGHTMARGIN) == 0 {
                            set_lineinfo(state, (*state).pos.row, NO_FORCE, DWL_ON, DHL_OFF);
                        }
                    }
                    b'8' => {
                        // DECALN
                        let e = schar_from_ascii(b'E');
                        for row in 0..(*state).rows {
                            let rw = rowwidth(state, row);
                            for col in 0..rw {
                                putglyph(state, e, 1, VTermPos { row, col });
                            }
                        }
                    }
                    _ => return 0,
                }
                2
            }
            b'(' | b')' | b'*' | b'+' => {
                // SCS
                if len != 2 {
                    return 0;
                }
                let setnum = ((*bytes as u8) - 0x28) as usize;
                let newenc = vterm_lookup_encoding(VTermEncodingType::ENC_SINGLE_94, *bytes.add(1));
                if !newenc.is_null() {
                    (*state).encoding[setnum].enc = newenc.cast();
                    if let Some(init_fn) = (*newenc).init {
                        init_fn(newenc, (*state).encoding[setnum].data.as_mut_ptr().cast());
                    }
                }
                2
            }
            b'7' => {
                // DECSC
                savecursor(state, 1);
                1
            }
            b'8' => {
                // DECRC
                savecursor(state, 0);
                1
            }
            b'<' => 1,
            b'=' => {
                // DECKPAM
                (*state).mode |= MODE_KEYPAD;
                1
            }
            b'>' => {
                // DECKPNM
                (*state).mode &= !MODE_KEYPAD;
                1
            }
            b'c' => {
                // RIS - ECMA-48 8.3.105
                let oldpos = (*state).pos;
                vterm_state_reset(state, 1);
                let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).movecursor
                {
                    let visible = c_int::from(((*state).mode & MODE_CURSOR_VISIBLE) != 0);
                    let _ = cb((*state).pos, oldpos, visible, (*state).cbdata);
                }
                1
            }
            b'n' => {
                // LS2 - ECMA-48 8.3.78
                (*state).gl_set = 2;
                1
            }
            b'o' => {
                // LS3 - ECMA-48 8.3.80
                (*state).gl_set = 3;
                1
            }
            b'~' => {
                // LS1R - ECMA-48 8.3.77
                (*state).gr_set = 1;
                1
            }
            b'}' => {
                // LS2R - ECMA-48 8.3.79
                (*state).gr_set = 2;
                1
            }
            b'|' => {
                // LS3R - ECMA-48 8.3.81
                (*state).gr_set = 3;
                1
            }
            _ => 0,
        }
    }
}

unsafe fn set_mode(state: *mut VTermStateFull, num: c_int, val: c_int) {
    unsafe {
        match num {
            4 => {
                // IRM - ECMA-48 7.2.10
                set_mode_bit(&mut (*state).mode, MODE_INSERT, val != 0);
            }
            20 => {
                // LNM - ANSI X3.4-1977
                set_mode_bit(&mut (*state).mode, MODE_NEWLINE, val != 0);
            }
            _ => {}
        }
    }
}

unsafe fn set_dec_mode(state: *mut VTermStateFull, num: c_int, val: c_int) {
    unsafe {
        match num {
            1 => {
                set_mode_bit(&mut (*state).mode, MODE_CURSOR, val != 0);
            }
            5 => {
                let _ = settermprop_bool(state, VTERM_PROP_REVERSE, val);
            }
            6 => {
                let oldpos = (*state).pos;
                set_mode_bit(&mut (*state).mode, MODE_ORIGIN, val != 0);
                (*state).pos.row = if ((*state).mode & MODE_ORIGIN) != 0 {
                    (*state).scrollregion_top
                } else {
                    0
                };
                (*state).pos.col = if ((*state).mode & MODE_ORIGIN) != 0 {
                    scrollregion_left(state)
                } else {
                    0
                };
                updatecursor(state, &raw const oldpos, 1);
            }
            7 => {
                set_mode_bit(&mut (*state).mode, MODE_AUTOWRAP, val != 0);
            }
            12 => {
                let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, val);
            }
            25 => {
                let _ = settermprop_bool(state, VTERM_PROP_CURSORVISIBLE, val);
            }
            69 => {
                set_mode_bit(&mut (*state).mode, MODE_LEFTRIGHTMARGIN, val != 0);
                if val != 0 {
                    for row in 0..(*state).rows {
                        set_lineinfo(state, row, FORCE, DWL_OFF, DHL_OFF);
                    }
                }
            }
            1000 | 1002 | 1003 => {
                let mouse_prop = if val == 0 {
                    VTERM_PROP_MOUSE_NONE
                } else if num == 1000 {
                    VTERM_PROP_MOUSE_CLICK
                } else if num == 1002 {
                    VTERM_PROP_MOUSE_DRAG
                } else {
                    VTERM_PROP_MOUSE_MOVE
                };
                let _ = settermprop_int(state, VTERM_PROP_MOUSE, mouse_prop);
            }
            1004 => {
                let _ = settermprop_bool(state, VTERM_PROP_FOCUSREPORT, val);
                set_mode_bit(&mut (*state).mode, MODE_REPORT_FOCUS, val != 0);
            }
            1005 => {
                (*state).mouse_protocol = if val != 0 { MOUSE_UTF8 } else { MOUSE_X10 };
            }
            1006 => {
                (*state).mouse_protocol = if val != 0 { MOUSE_SGR } else { MOUSE_X10 };
            }
            1015 => {
                (*state).mouse_protocol = if val != 0 { MOUSE_RXVT } else { MOUSE_X10 };
            }
            1047 => {
                let _ = settermprop_bool(state, VTERM_PROP_ALTSCREEN, val);
            }
            1048 => {
                savecursor(state, val);
            }
            1049 => {
                let _ = settermprop_bool(state, VTERM_PROP_ALTSCREEN, val);
                savecursor(state, val);
            }
            2004 => {
                set_mode_bit(&mut (*state).mode, MODE_BRACKETPASTE, val != 0);
            }
            2026 => {
                let _ = settermprop_bool(state, VTERM_PROP_SYNCOUTPUT, val);
            }
            2031 => {
                let _ = settermprop_bool(state, VTERM_PROP_THEMEUPDATES, val);
            }
            _ => {}
        }
    }
}

unsafe fn request_dec_mode(state: *mut VTermStateFull, num: c_int) {
    unsafe {
        let reply = match num {
            1 => ((*state).mode & MODE_CURSOR) != 0,
            5 => ((*state).mode & MODE_SCREEN) != 0,
            6 => ((*state).mode & MODE_ORIGIN) != 0,
            7 => ((*state).mode & MODE_AUTOWRAP) != 0,
            12 => ((*state).mode & MODE_CURSOR_BLINK) != 0,
            25 => ((*state).mode & MODE_CURSOR_VISIBLE) != 0,
            69 => ((*state).mode & MODE_LEFTRIGHTMARGIN) != 0,
            1000 => (*state).mouse_flags == MOUSE_WANT_CLICK,
            1002 => (*state).mouse_flags == (MOUSE_WANT_CLICK | MOUSE_WANT_DRAG),
            1003 => (*state).mouse_flags == (MOUSE_WANT_CLICK | MOUSE_WANT_MOVE),
            1004 => ((*state).mode & MODE_REPORT_FOCUS) != 0,
            1005 => (*state).mouse_protocol == MOUSE_UTF8,
            1006 => (*state).mouse_protocol == MOUSE_SGR,
            1015 => (*state).mouse_protocol == MOUSE_RXVT,
            1047 => ((*state).mode & MODE_ALT_SCREEN) != 0,
            2004 => ((*state).mode & MODE_BRACKETPASTE) != 0,
            2026 => ((*state).mode & MODE_SYNCHRONIZED_OUTPUT) != 0,
            2031 => ((*state).mode & MODE_THEME_UPDATES) != 0,
            _ => {
                let s = format!("?{num};0$y");
                vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
                return;
            }
        };

        let s = format!("?{num};{}$y", if reply { 1 } else { 2 });
        vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
    }
}

unsafe fn request_version_string(state: *mut VTermStateFull) {
    unsafe {
        let s = format!(">|libvterm({VTERM_VERSION_MAJOR}.{VTERM_VERSION_MINOR})");
        vterm_push_output_str_bytes((*state).vt, C1_DCS, true, s.as_bytes());
    }
}

unsafe fn request_key_encoding_flags(state: *mut VTermStateFull) {
    unsafe {
        let screen = if ((*state).mode & MODE_ALT_SCREEN) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        let stack = &(*state).key_encoding_stacks[screen];
        debug_assert!(stack.size > 0);
        let flags = stack.items[(stack.size - 1) as usize];
        let mut reply: c_int = 0;

        if (flags & (KEY_ENCODING_DISAMBIGUATE as u8)) != 0 {
            reply |= KEY_ENCODING_DISAMBIGUATE;
        }
        if (flags & (KEY_ENCODING_REPORT_EVENTS as u8)) != 0 {
            reply |= KEY_ENCODING_REPORT_EVENTS;
        }
        if (flags & (KEY_ENCODING_REPORT_ALTERNATE as u8)) != 0 {
            reply |= KEY_ENCODING_REPORT_ALTERNATE;
        }
        if (flags & (KEY_ENCODING_REPORT_ALL_KEYS as u8)) != 0 {
            reply |= KEY_ENCODING_REPORT_ALL_KEYS;
        }
        if (flags & (KEY_ENCODING_REPORT_ASSOCIATED as u8)) != 0 {
            reply |= KEY_ENCODING_REPORT_ASSOCIATED;
        }

        let s = format!("?{reply}u");
        vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
    }
}

unsafe fn set_key_encoding_flags(state: *mut VTermStateFull, arg: c_int, mode: c_int) {
    unsafe {
        let set = mode != 3;
        let reset_unset = mode == 1;
        let _ = reset_unset;

        let mut flags: u8 = 0;
        if (arg & KEY_ENCODING_DISAMBIGUATE) != 0 && set {
            flags |= KEY_ENCODING_DISAMBIGUATE as u8;
        }
        if (arg & KEY_ENCODING_REPORT_EVENTS) != 0 && set {
            flags |= KEY_ENCODING_REPORT_EVENTS as u8;
        }
        if (arg & KEY_ENCODING_REPORT_ALTERNATE) != 0 && set {
            flags |= KEY_ENCODING_REPORT_ALTERNATE as u8;
        }
        if (arg & KEY_ENCODING_REPORT_ALL_KEYS) != 0 && set {
            flags |= KEY_ENCODING_REPORT_ALL_KEYS as u8;
        }
        if (arg & KEY_ENCODING_REPORT_ASSOCIATED) != 0 && set {
            flags |= KEY_ENCODING_REPORT_ASSOCIATED as u8;
        }

        let screen = if ((*state).mode & MODE_ALT_SCREEN) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        let stack = &mut (*state).key_encoding_stacks[screen];
        debug_assert!(stack.size > 0);
        stack.items[(stack.size - 1) as usize] = flags;
    }
}

unsafe fn push_key_encoding_flags(state: *mut VTermStateFull, arg: c_int) {
    unsafe {
        let screen = if ((*state).mode & MODE_ALT_SCREEN) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        let stack = &mut (*state).key_encoding_stacks[screen];
        debug_assert!((stack.size as usize) <= stack.items.len());

        if (stack.size as usize) == stack.items.len() {
            for i in 0..(stack.items.len() - 1) {
                stack.items[i] = stack.items[i + 1];
            }
        } else {
            stack.size += 1;
        }

        set_key_encoding_flags(state, arg, 1);
    }
}

unsafe fn pop_key_encoding_flags(state: *mut VTermStateFull, arg: c_int) {
    unsafe {
        let screen = if ((*state).mode & MODE_ALT_SCREEN) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        let stack = &mut (*state).key_encoding_stacks[screen];
        if arg >= c_int::from(stack.size) {
            stack.size = 1;
            stack.items[0] = 0;
        } else if arg > 0 {
            stack.size -= arg as u8;
        }
    }
}

#[inline]
const fn leader(l: c_int, b: c_int) -> c_int {
    (l << 8) | b
}

#[inline]
const fn intermed(i: c_int, b: c_int) -> c_int {
    (i << 16) | b
}

unsafe extern "C" fn on_csi(
    leader_ptr: *const c_char,
    args: *const c_long,
    argcount: c_int,
    intermed_ptr: *const c_char,
    command: c_char,
    user: *mut c_void,
) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let mut leader_byte: c_int = 0;
        let mut intermed_byte: c_int = 0;
        let mut cancel_phantom: c_int = 1;

        if !leader_ptr.is_null() && *leader_ptr != 0 {
            if *leader_ptr.add(1) != 0 {
                return 0;
            }
            match *leader_ptr as u8 {
                b'?' | b'>' | b'<' | b'=' => {
                    leader_byte = c_int::from(*leader_ptr as u8);
                }
                _ => return 0,
            }
        }

        if !intermed_ptr.is_null() && *intermed_ptr != 0 {
            if *intermed_ptr.add(1) != 0 {
                return 0;
            }
            match *intermed_ptr as u8 {
                b' ' | b'!' | b'"' | b'$' | b'\'' => {
                    intermed_byte = c_int::from(*intermed_ptr as u8);
                }
                _ => return 0,
            }
        }

        let oldpos = (*state).pos;
        let cmd_val = (intermed_byte << 16) | (leader_byte << 8) | c_int::from(command as u8);

        const CMD_DECSED: c_int = leader(b'?' as c_int, 0x4a);
        const CMD_DECSEL: c_int = leader(b'?' as c_int, 0x4b);
        const CMD_DA2: c_int = leader(b'>' as c_int, 0x63);
        const CMD_DECSET: c_int = leader(b'?' as c_int, 0x68);
        const CMD_DECRST: c_int = leader(b'?' as c_int, 0x6c);
        const CMD_DECSGR: c_int = leader(b'?' as c_int, 0x6d);
        const CMD_DECDSR: c_int = leader(b'?' as c_int, 0x6e);
        const CMD_DECSTR: c_int = intermed(b'!' as c_int, 0x70);
        const CMD_DECRQM: c_int = leader(b'?' as c_int, intermed(b'$' as c_int, 0x70));
        const CMD_XTVERSION: c_int = leader(b'>' as c_int, 0x71);
        const CMD_DECSCUSR: c_int = intermed(b' ' as c_int, 0x71);
        const CMD_DECSCA: c_int = intermed(b'"' as c_int, 0x71);
        const CMD_KITTY_QUERY: c_int = leader(b'?' as c_int, 0x75);
        const CMD_KITTY_PUSH: c_int = leader(b'>' as c_int, 0x75);
        const CMD_KITTY_POP: c_int = leader(b'<' as c_int, 0x75);
        const CMD_KITTY_SET: c_int = leader(b'=' as c_int, 0x75);
        const CMD_DECIC: c_int = intermed(b'\'' as c_int, 0x7d);
        const CMD_DECDC: c_int = intermed(b'\'' as c_int, 0x7e);

        match cmd_val {
            0x40 => {
                // ICH - ECMA-48 8.3.64
                let count = csi_arg_count(*args);
                if is_cursor_in_scrollregion(state) {
                    let end_col = if ((*state).mode & MODE_LEFTRIGHTMARGIN) != 0 {
                        scrollregion_right(state)
                    } else {
                        thisrowwidth(state)
                    };
                    let rect = VTermRect {
                        start_row: (*state).pos.row,
                        end_row: (*state).pos.row + 1,
                        start_col: (*state).pos.col,
                        end_col,
                    };
                    scroll(state, rect, 0, -count);
                }
            }
            0x41 => {
                // CUU - ECMA-48 8.3.22
                let count = csi_arg_count(*args);
                (*state).pos.row -= count;
                (*state).at_phantom = 0;
            }
            0x42 => {
                // CUD - ECMA-48 8.3.19
                let count = csi_arg_count(*args);
                (*state).pos.row += count;
                (*state).at_phantom = 0;
            }
            0x43 => {
                // CUF - ECMA-48 8.3.20
                let count = csi_arg_count(*args);
                (*state).pos.col += count;
                (*state).at_phantom = 0;
            }
            0x44 => {
                // CUB - ECMA-48 8.3.18
                let count = csi_arg_count(*args);
                (*state).pos.col -= count;
                (*state).at_phantom = 0;
            }
            0x45 => {
                // CNL - ECMA-48 8.3.12
                let count = csi_arg_count(*args);
                (*state).pos.col = 0;
                (*state).pos.row += count;
                (*state).at_phantom = 0;
            }
            0x46 => {
                // CPL - ECMA-48 8.3.13
                let count = csi_arg_count(*args);
                (*state).pos.col = 0;
                (*state).pos.row -= count;
                (*state).at_phantom = 0;
            }
            0x47 => {
                // CHA - ECMA-48 8.3.9
                let val = csi_arg_or(*args, 1);
                (*state).pos.col = val - 1;
                (*state).at_phantom = 0;
            }
            0x48 => {
                // CUP - ECMA-48 8.3.21
                let row = csi_arg_or(*args, 1);
                let col = if argcount < 2 || csi_arg_is_missing(*args.add(1)) {
                    1
                } else {
                    csi_arg(*args.add(1))
                };
                (*state).pos.row = row - 1;
                (*state).pos.col = col - 1;
                if ((*state).mode & MODE_ORIGIN) != 0 {
                    (*state).pos.row += (*state).scrollregion_top;
                    (*state).pos.col += scrollregion_left(state);
                }
                (*state).at_phantom = 0;
            }
            0x49 => {
                // CHT - ECMA-48 8.3.10
                let count = csi_arg_count(*args);
                tab(state, count, 1);
            }
            0x4a | CMD_DECSED => {
                // ED / DECSED
                let selective = c_int::from(leader_byte == b'?' as c_int);
                match csi_arg(*args) {
                    CSI_ARG_MISSING | 0 => {
                        let mut rect = VTermRect {
                            start_row: (*state).pos.row,
                            end_row: (*state).pos.row + 1,
                            start_col: (*state).pos.col,
                            end_col: (*state).cols,
                        };
                        if rect.end_col > rect.start_col {
                            erase(state, rect, selective);
                        }

                        rect.start_row = (*state).pos.row + 1;
                        rect.end_row = (*state).rows;
                        rect.start_col = 0;
                        for row_ in rect.start_row..rect.end_row {
                            set_lineinfo(state, row_, FORCE, DWL_OFF, DHL_OFF);
                        }
                        if rect.end_row > rect.start_row {
                            erase(state, rect, selective);
                        }
                    }
                    1 => {
                        let mut rect = VTermRect {
                            start_row: 0,
                            end_row: (*state).pos.row,
                            start_col: 0,
                            end_col: (*state).cols,
                        };
                        for row_ in rect.start_row..rect.end_row {
                            set_lineinfo(state, row_, FORCE, DWL_OFF, DHL_OFF);
                        }
                        if rect.end_col > rect.start_col {
                            erase(state, rect, selective);
                        }

                        rect.start_row = (*state).pos.row;
                        rect.end_row = (*state).pos.row + 1;
                        rect.end_col = (*state).pos.col + 1;
                        if rect.end_row > rect.start_row {
                            erase(state, rect, selective);
                        }
                    }
                    2 => {
                        let rect = VTermRect {
                            start_row: 0,
                            end_row: (*state).rows,
                            start_col: 0,
                            end_col: (*state).cols,
                        };
                        for row_ in rect.start_row..rect.end_row {
                            set_lineinfo(state, row_, FORCE, DWL_OFF, DHL_OFF);
                        }
                        erase(state, rect, selective);
                    }
                    3 => {
                        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
                        if !callbacks.is_null()
                            && let Some(cb) = (*callbacks).sb_clear
                            && cb((*state).cbdata) != 0
                        {
                            return 1;
                        }
                    }
                    _ => {}
                }
            }
            0x4b | CMD_DECSEL => {
                // EL / DECSEL
                let selective = c_int::from(leader_byte == b'?' as c_int);
                let mut rect = VTermRect {
                    start_row: (*state).pos.row,
                    end_row: (*state).pos.row + 1,
                    start_col: 0,
                    end_col: 0,
                };

                match csi_arg(*args) {
                    CSI_ARG_MISSING | 0 => {
                        rect.start_col = (*state).pos.col;
                        rect.end_col = thisrowwidth(state);
                    }
                    1 => {
                        rect.start_col = 0;
                        rect.end_col = (*state).pos.col + 1;
                    }
                    2 => {
                        rect.start_col = 0;
                        rect.end_col = thisrowwidth(state);
                    }
                    _ => return 0,
                }

                if rect.end_col > rect.start_col {
                    erase(state, rect, selective);
                }
            }
            0x4c => {
                // IL - ECMA-48 8.3.67
                let count = csi_arg_count(*args);
                if is_cursor_in_scrollregion(state) {
                    let rect = VTermRect {
                        start_row: (*state).pos.row,
                        end_row: scrollregion_bottom(state),
                        start_col: scrollregion_left(state),
                        end_col: scrollregion_right(state),
                    };
                    scroll(state, rect, -count, 0);
                }
            }
            0x4d => {
                // DL - ECMA-48 8.3.32
                let count = csi_arg_count(*args);
                if is_cursor_in_scrollregion(state) {
                    let rect = VTermRect {
                        start_row: (*state).pos.row,
                        end_row: scrollregion_bottom(state),
                        start_col: scrollregion_left(state),
                        end_col: scrollregion_right(state),
                    };
                    scroll(state, rect, count, 0);
                }
            }
            0x50 => {
                // DCH - ECMA-48 8.3.26
                let count = csi_arg_count(*args);
                if is_cursor_in_scrollregion(state) {
                    let end_col = if ((*state).mode & MODE_LEFTRIGHTMARGIN) != 0 {
                        scrollregion_right(state)
                    } else {
                        thisrowwidth(state)
                    };
                    let rect = VTermRect {
                        start_row: (*state).pos.row,
                        end_row: (*state).pos.row + 1,
                        start_col: (*state).pos.col,
                        end_col,
                    };
                    scroll(state, rect, 0, count);
                }
            }
            0x53 => {
                // SU - ECMA-48 8.3.147
                let count = csi_arg_count(*args);
                let rect = VTermRect {
                    start_row: (*state).scrollregion_top,
                    end_row: scrollregion_bottom(state),
                    start_col: scrollregion_left(state),
                    end_col: scrollregion_right(state),
                };
                scroll(state, rect, count, 0);
            }
            0x54 => {
                // SD - ECMA-48 8.3.113
                let count = csi_arg_count(*args);
                let rect = VTermRect {
                    start_row: (*state).scrollregion_top,
                    end_row: scrollregion_bottom(state),
                    start_col: scrollregion_left(state),
                    end_col: scrollregion_right(state),
                };
                scroll(state, rect, -count, 0);
            }
            0x58 => {
                // ECH - ECMA-48 8.3.38
                let count = csi_arg_count(*args);
                let rw = thisrowwidth(state);
                let rect = VTermRect {
                    start_row: (*state).pos.row,
                    end_row: (*state).pos.row + 1,
                    start_col: (*state).pos.col,
                    end_col: ((*state).pos.col + count).min(rw),
                };
                erase(state, rect, 0);
            }
            0x5a => {
                // CBT - ECMA-48 8.3.7
                let count = csi_arg_count(*args);
                tab(state, count, -1);
            }
            0x60 => {
                // HPA - ECMA-48 8.3.57
                let col = csi_arg_or(*args, 1);
                (*state).pos.col = col - 1;
                (*state).at_phantom = 0;
            }
            0x61 => {
                // HPR - ECMA-48 8.3.59
                let count = csi_arg_count(*args);
                (*state).pos.col += count;
                (*state).at_phantom = 0;
            }
            0x62 => {
                // REP - ECMA-48 8.3.103
                let row_width = thisrowwidth(state);
                let count = csi_arg_count(*args);
                let col = ((*state).pos.col + count).min(row_width);
                let sc = schar_from_buf((*state).grapheme_buf.as_ptr(), (*state).grapheme_len);
                while (*state).pos.col < col {
                    putglyph(state, sc, (*state).combine_width, (*state).pos);
                    (*state).pos.col += (*state).combine_width;
                }
                if (*state).pos.col + (*state).combine_width >= row_width
                    && ((*state).mode & MODE_AUTOWRAP) != 0
                {
                    (*state).at_phantom = 1;
                    cancel_phantom = 0;
                }
            }
            0x63 => {
                // DA - ECMA-48 8.3.24
                let val = csi_arg_or(*args, 0);
                if val == 0 {
                    let attr =
                        CStr::from_ptr((&raw const vterm_primary_device_attr).cast::<c_char>())
                            .to_bytes();
                    let mut buf = Vec::with_capacity(attr.len() + 2);
                    buf.push(b'?');
                    buf.extend_from_slice(attr);
                    buf.push(b'c');
                    vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, &buf);
                }
            }
            CMD_DA2 => {
                // DEC secondary Device Attributes
                vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, b">0;100;0c");
            }
            0x64 => {
                // VPA - ECMA-48 8.3.158
                let row = csi_arg_or(*args, 1);
                (*state).pos.row = row - 1;
                if ((*state).mode & MODE_ORIGIN) != 0 {
                    (*state).pos.row += (*state).scrollregion_top;
                }
                (*state).at_phantom = 0;
            }
            0x65 => {
                // VPR - ECMA-48 8.3.160
                let count = csi_arg_count(*args);
                (*state).pos.row += count;
                (*state).at_phantom = 0;
            }
            0x66 => {
                // HVP - ECMA-48 8.3.63
                let row = csi_arg_or(*args, 1);
                let col = if argcount < 2 || csi_arg_is_missing(*args.add(1)) {
                    1
                } else {
                    csi_arg(*args.add(1))
                };
                (*state).pos.row = row - 1;
                (*state).pos.col = col - 1;
                if ((*state).mode & MODE_ORIGIN) != 0 {
                    (*state).pos.row += (*state).scrollregion_top;
                    (*state).pos.col += scrollregion_left(state);
                }
                (*state).at_phantom = 0;
            }
            0x67 => {
                // TBC - ECMA-48 8.3.154
                let val = csi_arg_or(*args, 0);
                match val {
                    0 => clear_col_tabstop(state, (*state).pos.col),
                    3 | 5 => {
                        for col in 0..(*state).cols {
                            clear_col_tabstop(state, col);
                        }
                    }
                    1 | 2 | 4 => {}
                    _ => return 0,
                }
            }
            0x68 => {
                // SM - ECMA-48 8.3.125
                if !csi_arg_is_missing(*args) {
                    set_mode(state, csi_arg(*args), 1);
                }
            }
            CMD_DECSET => {
                // DEC private mode set
                for i in 0..(argcount as usize) {
                    if !csi_arg_is_missing(*args.add(i)) {
                        set_dec_mode(state, csi_arg(*args.add(i)), 1);
                    }
                }
            }
            0x6a => {
                // HPB - ECMA-48 8.3.58
                let count = csi_arg_count(*args);
                (*state).pos.col -= count;
                (*state).at_phantom = 0;
            }
            0x6b => {
                // VPB - ECMA-48 8.3.159
                let count = csi_arg_count(*args);
                (*state).pos.row -= count;
                (*state).at_phantom = 0;
            }
            0x6c => {
                // RM - ECMA-48 8.3.106
                if !csi_arg_is_missing(*args) {
                    set_mode(state, csi_arg(*args), 0);
                }
            }
            CMD_DECRST => {
                // DEC private mode reset
                for i in 0..(argcount as usize) {
                    if !csi_arg_is_missing(*args.add(i)) {
                        set_dec_mode(state, csi_arg(*args.add(i)), 0);
                    }
                }
            }
            0x6d => {
                // SGR - ECMA-48 8.3.117
                vterm_state_setpen(state, args, argcount);
            }
            CMD_DECSGR => {
                // DECSGR
                for argi in 0..(argcount as usize) {
                    match csi_arg(*args.add(argi)) {
                        4 => {
                            let arg: c_long = 73;
                            vterm_state_setpen(state, &raw const arg, 1);
                        }
                        5 => {
                            let arg: c_long = 74;
                            vterm_state_setpen(state, &raw const arg, 1);
                        }
                        24 => {
                            let arg: c_long = 75;
                            vterm_state_setpen(state, &raw const arg, 1);
                        }
                        _ => {}
                    }
                }
            }
            0x6e | CMD_DECDSR => {
                // DSR / DECDSR
                let val = csi_arg_or(*args, 0);
                let qmark = if leader_byte == b'?' as c_int {
                    "?"
                } else {
                    ""
                };
                let mut dark = false;

                match val {
                    0..=4 => {}
                    5 => {
                        let s = format!("{qmark}0n");
                        vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
                    }
                    6 => {
                        let s =
                            format!("{qmark}{};{}R", (*state).pos.row + 1, (*state).pos.col + 1);
                        vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
                    }
                    996 => {
                        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
                        if !callbacks.is_null()
                            && let Some(cb) = (*callbacks).theme
                            && cb(&raw mut dark, (*state).cbdata) != 0
                        {
                            let s: &[u8] = if dark { b"?997;1n" } else { b"?997;2n" };
                            vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s);
                        }
                    }
                    _ => {}
                }
            }
            CMD_DECSTR => {
                // DECSTR - DEC soft terminal reset
                vterm_state_reset(state, 0);
            }
            CMD_DECRQM => {
                request_dec_mode(state, csi_arg(*args));
            }
            CMD_XTVERSION => {
                // XTVERSION - xterm query version string
                request_version_string(state);
            }
            CMD_DECSCUSR => {
                // DECSCUSR - DEC set cursor shape
                let val = csi_arg_or(*args, 1);
                match val {
                    0 | 1 => {
                        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 1);
                        let _ = settermprop_int(
                            state,
                            VTERM_PROP_CURSORSHAPE,
                            VTERM_PROP_CURSORSHAPE_BLOCK,
                        );
                    }
                    2 => {
                        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 0);
                        let _ = settermprop_int(
                            state,
                            VTERM_PROP_CURSORSHAPE,
                            VTERM_PROP_CURSORSHAPE_BLOCK,
                        );
                    }
                    3 => {
                        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 1);
                        let _ = settermprop_int(
                            state,
                            VTERM_PROP_CURSORSHAPE,
                            VTERM_PROP_CURSORSHAPE_UNDERLINE,
                        );
                    }
                    4 => {
                        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 0);
                        let _ = settermprop_int(
                            state,
                            VTERM_PROP_CURSORSHAPE,
                            VTERM_PROP_CURSORSHAPE_UNDERLINE,
                        );
                    }
                    5 => {
                        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 1);
                        let _ = settermprop_int(
                            state,
                            VTERM_PROP_CURSORSHAPE,
                            VTERM_PROP_CURSORSHAPE_BAR_LEFT,
                        );
                    }
                    6 => {
                        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 0);
                        let _ = settermprop_int(
                            state,
                            VTERM_PROP_CURSORSHAPE,
                            VTERM_PROP_CURSORSHAPE_BAR_LEFT,
                        );
                    }
                    _ => {}
                }
            }
            CMD_DECSCA => {
                // DECSCA - DEC select character protection attribute
                let val = csi_arg_or(*args, 0);
                match val {
                    0 | 2 => (*state).protected_cell = 0,
                    1 => (*state).protected_cell = 1,
                    _ => {}
                }
            }
            0x72 => {
                // DECSTBM - DEC custom
                (*state).scrollregion_top = csi_arg_or(*args, 1) - 1;
                (*state).scrollregion_bottom = if argcount < 2 || csi_arg_is_missing(*args.add(1)) {
                    -1
                } else {
                    csi_arg(*args.add(1))
                };
                (*state).scrollregion_top = (*state).scrollregion_top.clamp(0, (*state).rows);
                if (*state).scrollregion_bottom < -1 {
                    (*state).scrollregion_bottom = -1;
                }
                if (*state).scrollregion_top == 0 && (*state).scrollregion_bottom == (*state).rows {
                    (*state).scrollregion_bottom = -1;
                } else if (*state).scrollregion_bottom > (*state).rows {
                    (*state).scrollregion_bottom = (*state).rows;
                }

                if scrollregion_bottom(state) <= (*state).scrollregion_top {
                    (*state).scrollregion_top = 0;
                    (*state).scrollregion_bottom = -1;
                }

                (*state).pos.row = 0;
                (*state).pos.col = 0;
                if ((*state).mode & MODE_ORIGIN) != 0 {
                    (*state).pos.row += (*state).scrollregion_top;
                    (*state).pos.col += scrollregion_left(state);
                }
            }
            0x73 => {
                // DECSLRM - DEC custom
                (*state).scrollregion_left = csi_arg_or(*args, 1) - 1;
                (*state).scrollregion_right = if argcount < 2 || csi_arg_is_missing(*args.add(1)) {
                    -1
                } else {
                    csi_arg(*args.add(1))
                };
                (*state).scrollregion_left = (*state).scrollregion_left.clamp(0, (*state).cols);
                if (*state).scrollregion_right < -1 {
                    (*state).scrollregion_right = -1;
                }
                if (*state).scrollregion_left == 0 && (*state).scrollregion_right == (*state).cols {
                    (*state).scrollregion_right = -1;
                } else if (*state).scrollregion_right > (*state).cols {
                    (*state).scrollregion_right = (*state).cols;
                }

                if (*state).scrollregion_right > -1
                    && (*state).scrollregion_right <= (*state).scrollregion_left
                {
                    (*state).scrollregion_left = 0;
                    (*state).scrollregion_right = -1;
                }

                (*state).pos.row = 0;
                (*state).pos.col = 0;
                if ((*state).mode & MODE_ORIGIN) != 0 {
                    (*state).pos.row += (*state).scrollregion_top;
                    (*state).pos.col += scrollregion_left(state);
                }
            }
            CMD_KITTY_QUERY => {
                request_key_encoding_flags(state);
            }
            CMD_KITTY_PUSH => {
                push_key_encoding_flags(state, csi_arg_or(*args, 0));
            }
            CMD_KITTY_POP => {
                pop_key_encoding_flags(state, csi_arg_or(*args, 1));
            }
            CMD_KITTY_SET => {
                let val = if argcount < 2 || csi_arg_is_missing(*args.add(1)) {
                    1
                } else {
                    csi_arg(*args.add(1))
                };
                set_key_encoding_flags(state, csi_arg_or(*args, 0), val);
            }
            CMD_DECIC => {
                // DECIC
                let count = csi_arg_count(*args);
                if is_cursor_in_scrollregion(state) {
                    let rect = VTermRect {
                        start_row: (*state).scrollregion_top,
                        end_row: scrollregion_bottom(state),
                        start_col: (*state).pos.col,
                        end_col: scrollregion_right(state),
                    };
                    scroll(state, rect, 0, -count);
                }
            }
            CMD_DECDC => {
                // DECDC
                let count = csi_arg_count(*args);
                if is_cursor_in_scrollregion(state) {
                    let rect = VTermRect {
                        start_row: (*state).scrollregion_top,
                        end_row: scrollregion_bottom(state),
                        start_col: (*state).pos.col,
                        end_col: scrollregion_right(state),
                    };
                    scroll(state, rect, 0, count);
                }
            }
            _ => {
                let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
                if !fallbacks.is_null()
                    && let Some(cb) = (*fallbacks).csi
                    && cb(
                        leader_ptr,
                        args,
                        argcount,
                        intermed_ptr,
                        command,
                        (*state).fbdata,
                    ) != 0
                {
                    return 1;
                }
                return 0;
            }
        }

        if ((*state).mode & MODE_ORIGIN) != 0 {
            let min_row = (*state).scrollregion_top;
            let max_row = scrollregion_bottom(state) - 1;
            let min_col = scrollregion_left(state);
            let max_col = scrollregion_right(state) - 1;
            if (*state).pos.row < min_row {
                (*state).pos.row = min_row;
            }
            if (*state).pos.row > max_row {
                (*state).pos.row = max_row;
            }
            if (*state).pos.col < min_col {
                (*state).pos.col = min_col;
            }
            if (*state).pos.col > max_col {
                (*state).pos.col = max_col;
            }
        } else {
            let max_row = (*state).rows - 1;
            if (*state).pos.row < 0 {
                (*state).pos.row = 0;
            }
            if (*state).pos.row > max_row {
                (*state).pos.row = max_row;
            }
            let max_col = thisrowwidth(state) - 1;
            if (*state).pos.col < 0 {
                (*state).pos.col = 0;
            }
            if (*state).pos.col > max_col {
                (*state).pos.col = max_col;
            }
        }

        updatecursor(state, &raw const oldpos, cancel_phantom);

        1
    }
}

#[inline]
const fn unbase64one(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0xff,
    }
}

unsafe fn osc_selection(state: *mut VTermStateFull, frag: VTermStringFragment) {
    unsafe {
        let mut frag_str = frag.str_;
        let mut frag_len = frag.len();
        let frag_initial = frag.initial();
        let frag_final = frag.final_();

        if frag_initial {
            set_tmp_selection_mask(state, 0);
            set_tmp_selection_state(state, SELECTION_INITIAL);
        }

        while tmp_selection_state(state) == 0 && frag_len != 0 {
            match *frag_str as u8 {
                b'c' => {
                    let m = tmp_selection_mask(state) | VTERM_SELECTION_CLIPBOARD;
                    set_tmp_selection_mask(state, m);
                }
                b'p' => {
                    let m = tmp_selection_mask(state) | VTERM_SELECTION_PRIMARY;
                    set_tmp_selection_mask(state, m);
                }
                b'q' => {
                    let m = tmp_selection_mask(state) | VTERM_SELECTION_SECONDARY;
                    set_tmp_selection_mask(state, m);
                }
                b's' => {
                    let m = tmp_selection_mask(state) | VTERM_SELECTION_SELECT;
                    set_tmp_selection_mask(state, m);
                }
                b'0'..=b'7' => {
                    let bit = VTERM_SELECTION_CUT0 << ((*frag_str as u8) - b'0');
                    let m = tmp_selection_mask(state) | bit;
                    set_tmp_selection_mask(state, m);
                }
                b';' => {
                    set_tmp_selection_state(state, SELECTION_SELECTED);
                    if tmp_selection_mask(state) == 0 {
                        set_tmp_selection_mask(
                            state,
                            VTERM_SELECTION_SELECT | VTERM_SELECTION_CUT0,
                        );
                    }
                }
                _ => {}
            }

            frag_str = frag_str.add(1);
            frag_len -= 1;
        }

        let sel_callbacks = (*state)
            .selection
            .callbacks
            .cast::<VTermSelectionCallbacks>();

        if frag_len == 0 {
            if frag_final
                && !sel_callbacks.is_null()
                && let Some(set_cb) = (*sel_callbacks).set
            {
                let empty_frag = VTermStringFragment::new(
                    ptr::null(),
                    0,
                    tmp_selection_state(state) != SELECTION_SET,
                    true,
                    0 as VTermTerminator,
                );
                let _ = set_cb(
                    c_int::from(tmp_selection_mask(state)),
                    empty_frag,
                    (*state).selection.user,
                );
            }
            return;
        }

        if tmp_selection_state(state) == SELECTION_SELECTED {
            if (*frag_str as u8) == b'?' {
                set_tmp_selection_state(state, SELECTION_QUERY);
            } else {
                set_tmp_selection_state(state, SELECTION_SET_INITIAL);
                set_tmp_selection_recvpartial(state, 0);
            }
        }

        if tmp_selection_state(state) == SELECTION_QUERY {
            if !sel_callbacks.is_null()
                && let Some(query_cb) = (*sel_callbacks).query
            {
                let _ = query_cb(
                    c_int::from(tmp_selection_mask(state)),
                    (*state).selection.user,
                );
            }
            return;
        }

        if tmp_selection_state(state) == SELECTION_INVALID {
            return;
        }

        if !sel_callbacks.is_null()
            && let Some(set_cb) = (*sel_callbacks).set
        {
            let mut bufcur: usize = 0;
            let mut buffer = (*state).selection.buffer;

            let mut x: u32 = 0;
            let mut n: c_int = 0;

            let recvpartial = tmp_selection_recvpartial(state);
            if recvpartial != 0 {
                n = (recvpartial >> 24) as c_int;
                x = recvpartial & 0x03ffff;
                set_tmp_selection_recvpartial(state, 0);
            }

            while ((*state).selection.buflen - bufcur) >= 3 && frag_len != 0 {
                if (*frag_str as u8) == b'=' {
                    if n == 2 {
                        *buffer = ((x >> 4) & 0xff) as c_char;
                        buffer = buffer.add(1);
                        bufcur += 1;
                    }
                    if n == 3 {
                        *buffer = ((x >> 10) & 0xff) as c_char;
                        *buffer.add(1) = ((x >> 2) & 0xff) as c_char;
                        buffer = buffer.add(2);
                        bufcur += 2;
                    }

                    while frag_len != 0 && (*frag_str as u8) == b'=' {
                        frag_str = frag_str.add(1);
                        frag_len -= 1;
                    }

                    n = 0;
                } else {
                    let b = unbase64one(*frag_str as u8);
                    if b == 0xff {
                        set_tmp_selection_state(state, SELECTION_INVALID);
                        let err_frag = VTermStringFragment::new(
                            ptr::null(),
                            0,
                            true,
                            true,
                            0 as VTermTerminator,
                        );
                        let _ = set_cb(
                            c_int::from(tmp_selection_mask(state)),
                            err_frag,
                            (*state).selection.user,
                        );
                        break;
                    }

                    x = (x << 6) | u32::from(b);
                    n += 1;
                    frag_str = frag_str.add(1);
                    frag_len -= 1;

                    if n == 4 {
                        *buffer = ((x >> 16) & 0xff) as c_char;
                        *buffer.add(1) = ((x >> 8) & 0xff) as c_char;
                        *buffer.add(2) = (x & 0xff) as c_char;

                        buffer = buffer.add(3);
                        bufcur += 3;
                        x = 0;
                        n = 0;
                    }
                }

                if frag_len == 0 || ((*state).selection.buflen - bufcur) < 3 {
                    if bufcur != 0 {
                        let chunk_frag = VTermStringFragment::new(
                            (*state).selection.buffer,
                            bufcur,
                            tmp_selection_state(state) == SELECTION_SET_INITIAL,
                            frag_final && frag_len == 0,
                            0 as VTermTerminator,
                        );
                        let _ = set_cb(
                            c_int::from(tmp_selection_mask(state)),
                            chunk_frag,
                            (*state).selection.user,
                        );
                        set_tmp_selection_state(state, SELECTION_SET);
                    }

                    buffer = (*state).selection.buffer;
                    bufcur = 0;
                }
            }

            if n != 0 {
                set_tmp_selection_recvpartial(state, ((n as u32) << 24) | x);
            }
        }
    }
}

unsafe extern "C" fn on_osc(command: c_int, frag: VTermStringFragment, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();

        match command {
            0 => {
                let _ = settermprop_string(state, VTERM_PROP_ICONNAME, frag);
                let _ = settermprop_string(state, VTERM_PROP_TITLE, frag);
            }
            1 => {
                let _ = settermprop_string(state, VTERM_PROP_ICONNAME, frag);
            }
            2 => {
                let _ = settermprop_string(state, VTERM_PROP_TITLE, frag);
            }
            52 if !(*state).selection.callbacks.is_null() => {
                osc_selection(state, frag);
            }
            _ => {}
        }

        let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
        if !fallbacks.is_null()
            && let Some(cb) = (*fallbacks).osc
            && cb(command, frag, (*state).fbdata) != 0
        {
            return 1;
        }

        0
    }
}

unsafe fn request_status_string(state: *mut VTermStateFull, frag: VTermStringFragment) {
    unsafe {
        let vt = (*state).vt;
        let tmp = (&raw mut (*state).tmp.words[0]).cast::<u8>();

        let mut frag_str = frag.str_;
        let mut frag_len = frag.len();

        if frag.initial() {
            *tmp = 0;
            *tmp.add(1) = 0;
            *tmp.add(2) = 0;
            *tmp.add(3) = 0;
        }

        let mut i: usize = 0;
        while i < 3 && *tmp.add(i) != 0 {
            i += 1;
        }
        while i < 3 && frag_len != 0 {
            *tmp.add(i) = *frag_str as u8;
            i += 1;
            frag_str = frag_str.add(1);
            frag_len -= 1;
        }
        *tmp.add(i) = 0;

        if !frag.final_() {
            return;
        }

        let key = u32::from(*tmp) | (u32::from(*tmp.add(1)) << 8) | (u32::from(*tmp.add(2)) << 16);
        const KEY_M: u32 = b'm' as u32;
        const KEY_R: u32 = b'r' as u32;
        const KEY_S: u32 = b's' as u32;
        const KEY_SP_Q: u32 = (b' ' as u32) | ((b'q' as u32) << 8);
        const KEY_QT_Q: u32 = (b'"' as u32) | ((b'q' as u32) << 8);

        match key {
            KEY_M => {
                // Query SGR
                let mut args = [0 as c_long; 20];
                let argc = vterm_state_getpen(state, args.as_mut_ptr(), args.len() as c_int);
                let mut cur: usize = 0;

                let prefix: &[u8] = if ((*vt).mode & VT_MODE_CTRL8BIT) != 0 {
                    b"\x901$r"
                } else {
                    b"\x1bP1$r"
                };
                if cur + prefix.len() >= (*vt).tmpbuffer_len {
                    return;
                }
                ptr::copy_nonoverlapping(
                    prefix.as_ptr().cast::<c_char>(),
                    (*vt).tmpbuffer.add(cur),
                    prefix.len(),
                );
                cur += prefix.len();

                for argi in 0..(argc as usize) {
                    let sep = if argi == (argc as usize) - 1 {
                        ""
                    } else if csi_arg_has_more(args[argi]) {
                        ":"
                    } else {
                        ";"
                    };
                    let piece = format!("{}{sep}", csi_arg(args[argi]));
                    if cur + piece.len() >= (*vt).tmpbuffer_len {
                        return;
                    }
                    ptr::copy_nonoverlapping(
                        piece.as_ptr().cast::<c_char>(),
                        (*vt).tmpbuffer.add(cur),
                        piece.len(),
                    );
                    cur += piece.len();
                }

                let suffix: &[u8] = if ((*vt).mode & VT_MODE_CTRL8BIT) != 0 {
                    b"m\x9c"
                } else {
                    b"m\x1b\\"
                };
                if cur + suffix.len() >= (*vt).tmpbuffer_len {
                    return;
                }
                ptr::copy_nonoverlapping(
                    suffix.as_ptr().cast::<c_char>(),
                    (*vt).tmpbuffer.add(cur),
                    suffix.len(),
                );
                cur += suffix.len();
                *(*vt).tmpbuffer.add(cur) = 0;

                vterm_push_output_bytes(vt, (*vt).tmpbuffer, cur);
            }
            KEY_R => {
                // Query DECSTBM
                let s = format!(
                    "1$r{};{}r",
                    (*state).scrollregion_top + 1,
                    scrollregion_bottom(state)
                );
                vterm_push_output_str_bytes(vt, C1_DCS, true, s.as_bytes());
            }
            KEY_S => {
                // Query DECSLRM
                let s = format!(
                    "1$r{};{}s",
                    scrollregion_left(state) + 1,
                    scrollregion_right(state)
                );
                vterm_push_output_str_bytes(vt, C1_DCS, true, s.as_bytes());
            }
            KEY_SP_Q => {
                // Query DECSCUSR
                let mut reply: c_int = match get_cursor_shape((*state).mode) as c_int {
                    VTERM_PROP_CURSORSHAPE_BLOCK => 2,
                    VTERM_PROP_CURSORSHAPE_UNDERLINE => 4,
                    VTERM_PROP_CURSORSHAPE_BAR_LEFT => 6,
                    _ => 0,
                };
                if ((*state).mode & MODE_CURSOR_BLINK) != 0 {
                    reply -= 1;
                }
                let s = format!("1$r{reply} q");
                vterm_push_output_str_bytes(vt, C1_DCS, true, s.as_bytes());
            }
            KEY_QT_Q => {
                // Query DECSCA
                let reply: c_int = if (*state).protected_cell != 0 { 1 } else { 2 };
                let s = format!("1$r{reply}\"q");
                vterm_push_output_str_bytes(vt, C1_DCS, true, s.as_bytes());
            }
            _ => {
                vterm_push_output_str_bytes((*state).vt, C1_DCS, true, b"0$r");
            }
        }
    }
}

unsafe extern "C" fn on_dcs(
    command: *const c_char,
    commandlen: usize,
    frag: VTermStringFragment,
    user: *mut c_void,
) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();

        if commandlen == 2 && (*command as u8) == b'$' && (*command.add(1) as u8) == b'q' {
            request_status_string(state, frag);
            return 1;
        } else {
            let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
            if !fallbacks.is_null()
                && let Some(cb) = (*fallbacks).dcs
                && cb(command, commandlen, frag, (*state).fbdata) != 0
            {
                return 1;
            }
        }

        0
    }
}

unsafe extern "C" fn on_apc(frag: VTermStringFragment, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
        if !fallbacks.is_null()
            && let Some(cb) = (*fallbacks).apc
            && cb(frag, (*state).fbdata) != 0
        {
            return 1;
        }
        0
    }
}

unsafe extern "C" fn on_pm(frag: VTermStringFragment, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
        if !fallbacks.is_null()
            && let Some(cb) = (*fallbacks).pm
            && cb(frag, (*state).fbdata) != 0
        {
            return 1;
        }
        0
    }
}

unsafe extern "C" fn on_sos(frag: VTermStringFragment, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let fallbacks = (*state).fallbacks.cast::<VTermStateFallbacks>();
        if !fallbacks.is_null()
            && let Some(cb) = (*fallbacks).sos
            && cb(frag, (*state).fbdata) != 0
        {
            return 1;
        }
        0
    }
}

unsafe extern "C" fn on_resize(rows: c_int, cols: c_int, user: *mut c_void) -> c_int {
    unsafe {
        let state = user.cast::<VTermStateFull>();
        let oldpos = (*state).pos;

        if cols != (*state).cols {
            let newtabstops =
                vterm_allocator_malloc((*state).vt, (cols as usize).div_ceil(8)).cast::<u8>();

            let mut col: c_int = 0;
            while col < (*state).cols && col < cols {
                let mask = 1u8 << (col & 7);
                if (*(*state).tabstops.add((col >> 3) as usize) & mask) != 0 {
                    *newtabstops.add((col >> 3) as usize) |= mask;
                } else {
                    *newtabstops.add((col >> 3) as usize) &= !mask;
                }
                col += 1;
            }

            while col < cols {
                let mask = 1u8 << (col & 7);
                if col % 8 == 0 {
                    *newtabstops.add((col >> 3) as usize) |= mask;
                } else {
                    *newtabstops.add((col >> 3) as usize) &= !mask;
                }
                col += 1;
            }

            vterm_allocator_free((*state).vt, (*state).tabstops.cast());
            (*state).tabstops = newtabstops;
        }

        (*state).rows = rows;
        (*state).cols = cols;

        if (*state).scrollregion_bottom > -1 && (*state).scrollregion_bottom > (*state).rows {
            (*state).scrollregion_bottom = (*state).rows;
        }
        if (*state).scrollregion_right > -1 && (*state).scrollregion_right > (*state).cols {
            (*state).scrollregion_right = (*state).cols;
        }

        let mut fields = VTermStateFields {
            pos: (*state).pos,
            lineinfos: [
                (*state).lineinfos[0].cast::<VTermLineInfo>(),
                (*state).lineinfos[1].cast::<VTermLineInfo>(),
            ],
        };

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(resize_cb) = (*callbacks).resize
        {
            let _ = resize_cb(rows, cols, (&raw mut fields).cast(), (*state).cbdata);
            (*state).pos = fields.pos;
            (*state).lineinfos[0] = fields.lineinfos[0].cast();
            (*state).lineinfos[1] = fields.lineinfos[1].cast();
        } else if rows != (*state).rows {
            for bufidx in BUFIDX_PRIMARY..=BUFIDX_ALTSCREEN {
                let oldlineinfo = (*state).lineinfos[bufidx].cast::<VTermLineInfo>();
                if oldlineinfo.is_null() {
                    continue;
                }

                let newlineinfo = vterm_allocator_malloc(
                    (*state).vt,
                    (rows as usize) * size_of::<VTermLineInfo>(),
                )
                .cast::<VTermLineInfo>();

                let mut row: c_int = 0;
                while row < (*state).rows && row < rows {
                    *newlineinfo.add(row as usize) = *oldlineinfo.add(row as usize);
                    row += 1;
                }

                while row < rows {
                    *newlineinfo.add(row as usize) = VTermLineInfo::default();
                    row += 1;
                }

                vterm_allocator_free((*state).vt, (*state).lineinfos[bufidx]);
                (*state).lineinfos[bufidx] = newlineinfo.cast();
            }
        }

        (*state).lineinfo = (*state).lineinfos[if ((*state).mode & MODE_ALT_SCREEN) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        }];

        if (*state).at_phantom != 0 && (*state).pos.col < cols - 1 {
            (*state).at_phantom = 0;
            (*state).pos.col += 1;
        }

        if (*state).pos.row < 0 {
            (*state).pos.row = 0;
        }
        if (*state).pos.row >= rows {
            (*state).pos.row = rows - 1;
        }
        if (*state).pos.col < 0 {
            (*state).pos.col = 0;
        }
        if (*state).pos.col >= cols {
            (*state).pos.col = cols - 1;
        }

        updatecursor(state, &raw const oldpos, 1);

        1
    }
}

static PARSER_CALLBACKS: VTermParserCallbacks = VTermParserCallbacks {
    text: Some(on_text),
    control: Some(on_control),
    escape: Some(on_escape),
    csi: Some(on_csi),
    osc: Some(on_osc),
    dcs: Some(on_dcs),
    apc: Some(on_apc),
    pm: Some(on_pm),
    sos: Some(on_sos),
    resize: Some(on_resize),
};

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_obtain_state(vt: *mut VTerm) -> *mut VTermStateFull {
    unsafe {
        if !(*vt).state.is_null() {
            return (*vt).state.cast();
        }

        let state = vterm_state_new(vt);
        (*vt).state = state.cast();

        vterm_parser_set_callbacks(vt, &raw const PARSER_CALLBACKS, state.cast());

        state
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_reset(state: *mut VTermStateFull, hard: c_int) {
    unsafe {
        (*state).scrollregion_top = 0;
        (*state).scrollregion_bottom = -1;
        (*state).scrollregion_left = 0;
        (*state).scrollregion_right = -1;

        set_mode_bit(&mut (*state).mode, MODE_KEYPAD, false);
        set_mode_bit(&mut (*state).mode, MODE_CURSOR, false);
        set_mode_bit(&mut (*state).mode, MODE_AUTOWRAP, true);
        set_mode_bit(&mut (*state).mode, MODE_INSERT, false);
        set_mode_bit(&mut (*state).mode, MODE_NEWLINE, false);
        set_mode_bit(&mut (*state).mode, MODE_ALT_SCREEN, false);
        set_mode_bit(&mut (*state).mode, MODE_ORIGIN, false);
        set_mode_bit(&mut (*state).mode, MODE_LEFTRIGHTMARGIN, false);
        set_mode_bit(&mut (*state).mode, MODE_BRACKETPASTE, false);
        set_mode_bit(&mut (*state).mode, MODE_REPORT_FOCUS, false);

        (*state).mouse_flags = 0;

        (*(*state).vt).mode &= !VT_MODE_CTRL8BIT;

        for col in 0..(*state).cols {
            if col % 8 == 0 {
                set_col_tabstop(state, col);
            } else {
                clear_col_tabstop(state, col);
            }
        }

        for row in 0..(*state).rows {
            set_lineinfo(state, row, FORCE, DWL_OFF, DHL_OFF);
        }

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).initpen
        {
            let _ = cb((*state).cbdata);
        }

        vterm_state_resetpen(state);

        let default_enc = if ((*(*state).vt).mode & VT_MODE_UTF8) != 0 {
            vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'u' as c_char)
        } else {
            vterm_lookup_encoding(VTermEncodingType::ENC_SINGLE_94, b'B' as c_char)
        };

        for enc_inst in &mut (*state).encoding {
            enc_inst.enc = default_enc.cast();
            if let Some(init_fn) = (*default_enc).init {
                init_fn(default_enc, enc_inst.data.as_mut_ptr().cast());
            }
        }

        (*state).gl_set = 0;
        (*state).gr_set = 1;
        (*state).gsingle_set = 0;

        (*state).protected_cell = 0;

        let _ = settermprop_bool(state, VTERM_PROP_CURSORVISIBLE, 1);
        let _ = settermprop_bool(state, VTERM_PROP_CURSORBLINK, 1);
        let _ = settermprop_int(state, VTERM_PROP_CURSORSHAPE, VTERM_PROP_CURSORSHAPE_BLOCK);

        if hard != 0 {
            (*state).pos.row = 0;
            (*state).pos.col = 0;
            (*state).at_phantom = 0;

            let rect = VTermRect {
                start_row: 0,
                end_row: (*state).rows,
                start_col: 0,
                end_col: (*state).cols,
            };
            erase(state, rect, 0);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_callbacks(
    state: *mut VTermStateFull,
    callbacks: *const VTermStateCallbacks,
    user: *mut c_void,
) {
    unsafe {
        if !callbacks.is_null() {
            (*state).callbacks = callbacks.cast();
            (*state).cbdata = user;

            if let Some(cb) = (*callbacks).initpen {
                let _ = cb((*state).cbdata);
            }
        } else {
            (*state).callbacks = ptr::null();
            (*state).cbdata = ptr::null_mut();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_unrecognised_fallbacks(
    state: *mut VTermStateFull,
    fallbacks: *const VTermStateFallbacks,
    user: *mut c_void,
) {
    unsafe {
        if !fallbacks.is_null() {
            (*state).fallbacks = fallbacks.cast();
            (*state).fbdata = user;
        } else {
            (*state).fallbacks = ptr::null();
            (*state).fbdata = ptr::null_mut();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_termprop(
    state: *mut VTermStateFull,
    prop: VTermProp,
    val: *mut VTermValue,
) -> c_int {
    unsafe {
        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).settermprop
            && cb(prop, val, (*state).cbdata) == 0
        {
            return 0;
        }

        match prop {
            VTERM_PROP_TITLE | VTERM_PROP_ICONNAME => 1,
            VTERM_PROP_CURSORVISIBLE => {
                set_mode_bit(&mut (*state).mode, MODE_CURSOR_VISIBLE, (*val).boolean != 0);
                1
            }
            VTERM_PROP_CURSORBLINK => {
                set_mode_bit(&mut (*state).mode, MODE_CURSOR_BLINK, (*val).boolean != 0);
                1
            }
            VTERM_PROP_CURSORSHAPE => {
                set_cursor_shape(&mut (*state).mode, (*val).number as u32);
                1
            }
            VTERM_PROP_REVERSE => {
                set_mode_bit(&mut (*state).mode, MODE_SCREEN, (*val).boolean != 0);
                1
            }
            VTERM_PROP_ALTSCREEN => {
                set_mode_bit(&mut (*state).mode, MODE_ALT_SCREEN, (*val).boolean != 0);
                (*state).lineinfo = (*state).lineinfos[if ((*state).mode & MODE_ALT_SCREEN) != 0 {
                    BUFIDX_ALTSCREEN
                } else {
                    BUFIDX_PRIMARY
                }];
                if ((*state).mode & MODE_ALT_SCREEN) != 0 {
                    let rect = VTermRect {
                        start_row: 0,
                        end_row: (*state).rows,
                        start_col: 0,
                        end_col: (*state).cols,
                    };
                    erase(state, rect, 0);
                }
                1
            }
            VTERM_PROP_MOUSE => {
                (*state).mouse_flags = 0;
                if (*val).number != 0 {
                    (*state).mouse_flags |= MOUSE_WANT_CLICK;
                }
                if (*val).number == VTERM_PROP_MOUSE_DRAG {
                    (*state).mouse_flags |= MOUSE_WANT_DRAG;
                }
                if (*val).number == VTERM_PROP_MOUSE_MOVE {
                    (*state).mouse_flags |= MOUSE_WANT_MOVE;
                }
                1
            }
            VTERM_PROP_FOCUSREPORT => {
                set_mode_bit(&mut (*state).mode, MODE_REPORT_FOCUS, (*val).boolean != 0);
                1
            }
            VTERM_PROP_THEMEUPDATES => {
                set_mode_bit(&mut (*state).mode, MODE_THEME_UPDATES, (*val).boolean != 0);
                1
            }
            VTERM_PROP_SYNCOUTPUT => {
                set_mode_bit(
                    &mut (*state).mode,
                    MODE_SYNCHRONIZED_OUTPUT,
                    (*val).boolean != 0,
                );
                1
            }
            _ => 0,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_focus_in(state: *mut VTermStateFull) {
    unsafe {
        if ((*state).mode & MODE_REPORT_FOCUS) != 0 {
            vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, b"I");
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_focus_out(state: *mut VTermStateFull) {
    unsafe {
        if ((*state).mode & MODE_REPORT_FOCUS) != 0 {
            vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, b"O");
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_get_lineinfo(
    state: *const VTermStateFull,
    row: c_int,
) -> *const VTermLineInfo {
    unsafe { (*state).lineinfo.cast::<VTermLineInfo>().add(row as usize) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_selection_callbacks(
    state: *mut VTermStateFull,
    callbacks: *const VTermSelectionCallbacks,
    user: *mut c_void,
    mut buffer: *mut c_char,
    buflen: usize,
) {
    unsafe {
        if buflen != 0 && buffer.is_null() {
            buffer = vterm_allocator_malloc((*state).vt, buflen).cast::<c_char>();
        }

        (*state).selection.callbacks = callbacks.cast();
        (*state).selection.user = user;
        (*state).selection.buffer = buffer;
        (*state).selection.buflen = buflen;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbase64one_and_csi_arg_helpers_work() {
        assert_eq!(unbase64one(b'A'), 0);
        assert_eq!(unbase64one(b'Z'), 25);
        assert_eq!(unbase64one(b'a'), 26);
        assert_eq!(unbase64one(b'z'), 51);
        assert_eq!(unbase64one(b'0'), 52);
        assert_eq!(unbase64one(b'9'), 61);
        assert_eq!(unbase64one(b'+'), 62);
        assert_eq!(unbase64one(b'/'), 63);
        assert_eq!(unbase64one(b'='), 0xff);

        let missing = c_long::from(CSI_ARG_MISSING);
        assert!(csi_arg_is_missing(missing));
        assert_eq!(csi_arg_or(missing, 5), 5);
        assert_eq!(csi_arg_count(missing), 1);
        assert_eq!(csi_arg_count(0), 1);
        assert_eq!(csi_arg_count(4), 4);
    }
}
