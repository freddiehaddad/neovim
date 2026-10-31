//! Low-level functions to manipulate individual character cells on the screen grid (`src/nvim/grid.c`).

use crate::arabic::arabic_shape;
use crate::cursor::{Colnr, Linenr};
use crate::decoration::{VirtTextChunk, decor_check_invalid_glyphs, next_virt_text_chunk};
use crate::globals::*;
use crate::highlight::{hl_apply_winblend, hl_combine_attr};
use crate::mbyte::{
    mb_string2cells, utf_char2bytes, utf_char2len, utf_cluster_info_len, utf_cp_bounds,
    utf_ptr2_str_char_info_len, utf_ptr2cells, utf_slice2char, utf_slice2len, utfc_ptr2len,
};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::option_vars::*;
use crate::optionstr::check_chars_options;
use crate::plines::{ClusterInfo, StrCharInfo};
use crate::ui::{
    ui_call_grid_resize, ui_call_grid_scroll, ui_check_cursor_grid, ui_grid_cursor_goto, ui_has,
    ui_line,
};
use crate::window::{WinConfig, WinInfoInfo};
use std::ptr;

pub type Schar = u32;
pub type Sattr = i32;
pub type Sscratch = i32;
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::win_config::{
    AlignTextPos, BorderTextType, K_ALIGN_CENTER, K_ALIGN_LEFT, K_ALIGN_RIGHT,
    K_BORDER_TEXT_FOOTER, K_BORDER_TEXT_TITLE, K_UIMULTIGRID,
};

use crate::drawline::{SLF_INC_VCOL, SLF_RIGHTLEFT, SLF_WRAP};
use crate::drawscreen::DEFAULT_GRID_HANDLE;

pub(crate) const K_OPT_RDB_FLAG_INVALID: u32 = 0x04;
const K_OPT_RDB_FLAG_NODELTA: u32 = 0x08;

const HLF_BTITLE: usize = 68;
const HLF_BFOOTER: usize = 69;

#[repr(C)]
#[derive(Clone, Default)]
pub struct ScreenGrid {
    pub handle: ObjHandle,
    pub chars: Vec<u32>,
    pub attrs: Vec<i32>,
    pub vcols: Vec<i32>,
    pub line_offset: Vec<usize>,
    pub dirty_col: Vec<i32>,
    pub rows: i32,
    pub cols: i32,
    pub valid: bool,
    pub throttled: bool,
    pub blending: bool,
    pub mouse_enabled: bool,
    pub zindex: i32,
    pub comp_row: i32,
    pub comp_col: i32,
    pub comp_width: i32,
    pub comp_height: i32,
    pub comp_index: usize,
    pub comp_disabled: bool,
    pub pending_comp_index_update: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GridView {
    pub target: *mut ScreenGrid,
    pub row_offset: i32,
    pub col_offset: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharBoundsOff {
    pub begin_off: i8,
    pub end_off: i8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Wline {
    pub wl_lnum: Linenr,
    pub wl_size: u16,
    pub wl_valid: bool,
    pub wl_folded: bool,
    pub wl_foldend: Linenr,
    pub wl_lastlnum: Linenr,
}

struct GridModuleState {
    linebuf_scratch: Vec<Sscratch>,
    glyph_cache_keys: Vec<u8>,
    glyph_cache_offsets: std::collections::BTreeMap<Vec<u8>, u32>,
    grid_line_grid: *mut ScreenGrid,
    grid_line_row: i32,
    grid_line_coloff: i32,
    grid_line_maxcol: i32,
    grid_line_first: i32,
    grid_line_last: i32,
    grid_line_clear_to: i32,
    grid_line_bg_attr: i32,
    grid_line_clear_attr: i32,
    grid_line_flags: i32,
    last_grid_handle: i32,
}

impl GridModuleState {
    const fn new() -> Self {
        Self {
            linebuf_scratch: Vec::new(),
            glyph_cache_keys: Vec::new(),
            glyph_cache_offsets: std::collections::BTreeMap::new(),
            grid_line_grid: ptr::null_mut(),
            grid_line_row: -1,
            grid_line_coloff: 0,
            grid_line_maxcol: 0,
            grid_line_first: i32::MAX,
            grid_line_last: 0,
            grid_line_clear_to: 0,
            grid_line_bg_attr: 0,
            grid_line_clear_attr: 0,
            grid_line_flags: 0,
            last_grid_handle: DEFAULT_GRID_HANDLE,
        }
    }
}

static GRID_STATE: GlobalState<GridModuleState> = GlobalState::new(GridModuleState::new());

#[inline]
fn grid_st() -> &'static mut GridModuleState {
    unsafe { &mut *GRID_STATE.as_ptr() }
}

#[inline]
pub const fn schar_from_ascii(x: u8) -> Schar {
    #[cfg(target_endian = "big")]
    {
        (x as Schar) << 24
    }
    #[cfg(not(target_endian = "big"))]
    {
        x as Schar
    }
}

#[inline]
const fn schar_idx(sc: Schar) -> u32 {
    #[cfg(target_endian = "big")]
    {
        sc & 0x00FF_FFFF
    }
    #[cfg(not(target_endian = "big"))]
    {
        sc >> 8
    }
}

#[inline]
const fn arabic_char(ch: i32) -> bool {
    (ch & 0xFF00) == 0x0600
}

#[inline]
fn strnlen_4(p: *const u8) -> usize {
    for i in 0..4 {
        if unsafe { *p.wrapping_add(i) } == 0 {
            return i;
        }
    }
    4
}

/// Determine if dedicated window grid should be used or the `default_grid` (`grid_adjust`).
pub fn grid_adjust(grid: &mut GridView, row_off: &mut i32, col_off: &mut i32) -> *mut ScreenGrid {
    *row_off += grid.row_offset;
    *col_off += grid.col_offset;
    grid.target
}

pub fn schar_from_str(str_: impl crate::strings::IntoCCharPtr) -> Schar {
    let str_ = str_.into_c_char_ptr();
    {
        if str_.is_null() {
            return 0;
        }
        schar_from_buf(str_, crate::memory::cstr_len(str_))
    }
}

pub fn schar_from_bytes(bytes: &[u8]) -> Schar {
    let len = bytes.len();
    assert!(len < MAX_SCHAR_SIZE);
    if len <= 4 {
        let mut sc: Schar = 0;
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), (&raw mut sc).cast::<u8>(), len) };
        sc
    } else {
        let idx = if let Some(&existing_idx) = grid_st().glyph_cache_offsets.get(bytes) {
            existing_idx
        } else {
            let new_idx = grid_st().glyph_cache_keys.len() as u32;
            grid_st().glyph_cache_keys.extend_from_slice(bytes);
            grid_st().glyph_cache_keys.push(0);
            grid_st()
                .glyph_cache_offsets
                .insert(bytes.to_vec(), new_idx);
            new_idx
        };
        assert!(idx < 0xFF_FFFF);
        #[cfg(target_endian = "big")]
        {
            idx + (0xFFu32 << 24)
        }
        #[cfg(not(target_endian = "big"))]
        {
            0xFF + (idx << 8)
        }
    }
}

pub fn schar_from_buf(buf: impl crate::strings::IntoCCharPtr, len: usize) -> Schar {
    let buf = buf.into_c_char_ptr();
    schar_from_bytes(crate::memory::cstr_slice(buf, len))
}

pub fn schar_cache_clear_if_full() -> bool {
    if grid_st().glyph_cache_keys.len() > (1 << 21) {
        schar_cache_clear();
        return true;
    }
    false
}

pub fn schar_cache_clear() {
    decor_check_invalid_glyphs();
    grid_st().glyph_cache_keys.clear();
    grid_st().glyph_cache_offsets.clear();

    if check_chars_options().is_some() {
        std::process::abort();
    }
}

pub fn schar_high(sc: Schar) -> bool {
    #[cfg(target_endian = "big")]
    {
        (sc & 0xFF00_0000) == 0xFF00_0000
    }
    #[cfg(not(target_endian = "big"))]
    {
        (sc & 0xFF) == 0xFF
    }
}

/// Sets final NUL (`schar_get`).
pub fn schar_get(mut buf_out: *mut u8, sc: Schar) -> usize {
    let len = schar_get_adv(&raw mut buf_out, sc);
    unsafe {
        *buf_out = 0;
    }
    len
}

pub fn schar_get_bytes(buf_out: &mut [u8], sc: Schar) -> usize {
    schar_get(buf_out.as_mut_ptr(), sc)
}

/// Advance `buf_out`. Do NOT set final NUL (`schar_get_adv`).
pub fn schar_get_adv(buf_out: *mut *mut u8, sc: Schar) -> usize {
    unsafe {
        let len: usize;
        if schar_high(sc) {
            let idx = schar_idx(sc) as usize;
            assert!(idx < grid_st().glyph_cache_keys.len());
            let key_ptr = grid_st().glyph_cache_keys.as_ptr().wrapping_add(idx);
            len = crate::memory::cstr_len(key_ptr.cast());
            ptr::copy_nonoverlapping(key_ptr, *buf_out, len);
        } else {
            let sc_ptr = (&raw const sc).cast::<u8>();
            len = strnlen_4(sc_ptr);
            ptr::copy_nonoverlapping(sc_ptr, *buf_out, len);
        }
        *buf_out = (*buf_out).wrapping_add(len);
        len
    }
}

pub fn schar_cells(sc: Schar) -> i32 {
    let mut buf = [0u8; MAX_SCHAR_SIZE];
    schar_get(buf.as_mut_ptr(), sc);
    crate::mbyte::utf_ptr2cells(buf.as_ptr().cast())
}

pub fn schar_len(sc: Schar) -> usize {
    if schar_high(sc) {
        let idx = schar_idx(sc) as usize;
        assert!(idx < grid_st().glyph_cache_keys.len());
        crate::memory::cstr_len(grid_st().glyph_cache_keys.as_ptr().wrapping_add(idx).cast())
    } else {
        strnlen_4((&raw const sc).cast::<u8>())
    }
}

#[inline]
fn schar_get_first_byte(sc: Schar) -> u8 {
    assert!(!(schar_high(sc) && (schar_idx(sc) as usize) >= grid_st().glyph_cache_keys.len()));
    if schar_high(sc) {
        grid_st().glyph_cache_keys.as_slice()[schar_idx(sc) as usize]
    } else {
        unsafe { *(&raw const sc).cast::<u8>() }
    }
}

pub fn schar_get_first_codepoint(sc: Schar) -> i32 {
    let mut sc_buf = [0u8; MAX_SCHAR_SIZE];
    schar_get_bytes(&mut sc_buf, sc);
    utf_slice2char(&sc_buf)
}

pub fn schar_get_ascii(sc: Schar) -> u8 {
    #[cfg(target_endian = "big")]
    unsafe {
        if (sc & 0x80FF_FFFF) == 0 {
            *(&raw const sc).cast::<u8>()
        } else {
            0
        }
    }
    #[cfg(not(target_endian = "big"))]
    {
        if sc < 0x80 { sc as u8 } else { 0 }
    }
}

#[inline]
fn schar_in_arabic_block(sc: Schar) -> bool {
    let first_byte = schar_get_first_byte(sc);
    (first_byte & 0xFE) == 0xD8
}

#[inline]
fn schar_get_first_two_codepoints(sc: Schar, c0: &mut i32, c1: &mut i32) {
    let mut sc_buf = [0u8; MAX_SCHAR_SIZE];
    schar_get_bytes(&mut sc_buf, sc);

    *c0 = utf_slice2char(&sc_buf);
    let len = utf_slice2len(&sc_buf);
    if *c0 == 0 {
        *c1 = 0;
    } else {
        *c1 = utf_slice2char(&sc_buf[len..]);
    }
}

pub fn line_do_arabic_shape(buf: *mut Schar, cols: i32) {
    let mut i: i32 = 0;
    while i < cols {
        if schar_in_arabic_block(unsafe { *buf.wrapping_add(i as usize) }) {
            break;
        }
        i += 1;
    }

    if i == cols {
        return;
    }

    let mut c0prev: i32 = 0;
    let mut c0: i32 = 0;
    let mut c1: i32 = 0;
    schar_get_first_two_codepoints(unsafe { *buf.wrapping_add(i as usize) }, &mut c0, &mut c1);

    while i < cols {
        let mut c0next: i32 = 0;
        let mut c1next: i32 = 0;
        let next_sc = if i + 1 < cols {
            unsafe { *buf.wrapping_add((i + 1) as usize) }
        } else {
            0
        };
        schar_get_first_two_codepoints(next_sc, &mut c0next, &mut c1next);

        if arabic_char(c0) {
            let mut c1new = c1;
            let c0new = arabic_shape(c0, &mut c1new, c0next, c1next, c0prev);

            if !(c0new == c0 && c1new == c1) {
                let mut scbuf = [0u8; MAX_SCHAR_SIZE];
                schar_get(scbuf.as_mut_ptr(), unsafe { *buf.wrapping_add(i as usize) });

                let mut scbuf_new = [0u8; MAX_SCHAR_SIZE];
                let mut len = utf_char2bytes(c0new, scbuf_new.as_mut_ptr().cast()) as usize;
                if c1new != 0 {
                    len += utf_char2bytes(c1new, scbuf_new.as_mut_ptr().wrapping_add(len).cast())
                        as usize;
                }

                let off = (utf_char2len(c0) + if c1 != 0 { utf_char2len(c1) } else { 0 }) as usize;
                let mut rest = crate::memory::cstr_len(scbuf.as_ptr().wrapping_add(off).cast());
                if rest + len + 1 > MAX_SCHAR_SIZE {
                    let bounds = utf_cp_bounds(
                        scbuf.as_ptr().wrapping_add(off).cast(),
                        scbuf.as_ptr().wrapping_add(off + rest - 1).cast(),
                    );
                    rest = (rest as isize - (isize::from(bounds.begin_off) + 1)) as usize;
                }
                unsafe {
                    ptr::copy_nonoverlapping(
                        scbuf.as_ptr().wrapping_add(off),
                        scbuf_new.as_mut_ptr().wrapping_add(len),
                        rest,
                    )
                };
                unsafe {
                    *buf.wrapping_add(i as usize) =
                        schar_from_buf(scbuf_new.as_ptr().cast(), len + rest);
                }
            }
        }

        c0prev = c0;
        c0 = c0next;
        c1 = c1next;
        i += 1;
    }
}

#[inline]
pub fn grid_allocated(grid: &ScreenGrid) -> bool {
    !grid.chars.is_empty()
}

/// Clear a line in the grid starting at `off` until `width` characters are cleared (`grid_clear_line`).
pub fn grid_clear_line(grid: &mut ScreenGrid, off: usize, width: i32, valid: bool) {
    let len = width as usize;
    grid.chars[off..off + len].fill(schar_from_ascii(b' '));
    let fill: Sattr = if valid { 0 } else { -1 };
    grid.attrs[off..off + len].fill(fill);
    grid.vcols[off..off + len].fill(-1);
}

pub fn grid_invalidate(grid: &mut ScreenGrid) {
    let len = grid.rows as usize * grid.cols as usize;
    grid.attrs[..len].fill(-1);
}

#[inline]
fn grid_invalid_row(grid: &mut ScreenGrid, row: i32) -> bool {
    grid.attrs[grid.line_offset[row as usize]] < 0
}

/// Get a single character directly from `grid.chars` (`grid_getchar`).
pub fn grid_getchar(grid: &ScreenGrid, row: i32, col: i32, attrp: Option<&mut i32>) -> Schar {
    if grid.chars.is_empty() || row >= grid.rows || col >= grid.cols {
        return 0;
    }

    let off = grid.line_offset[row as usize] + col as usize;
    if let Some(attrp) = attrp {
        *attrp = grid.attrs[off];
    }
    grid.chars[off]
}

pub fn grid_line_start(view: *mut GridView, mut row: i32) {
    let mut col: i32 = 0;
    let grid = grid_adjust(unsafe { &mut *view }, &mut row, &mut col);
    screengrid_line_start(unsafe { &mut *grid }, row, col);
}

pub fn screengrid_line_start(grid: &mut ScreenGrid, row: i32, col: i32) {
    let linebuf_size = g_st().linebuf_char.len();
    grid_st().grid_line_maxcol = grid.cols;
    assert!(grid_st().grid_line_grid.is_null());
    grid_st().grid_line_row = row;
    grid_st().grid_line_grid = (&raw mut *grid);
    grid_st().grid_line_coloff = col;
    grid_st().grid_line_first = linebuf_size as i32;
    grid_st().grid_line_maxcol = grid_st()
        .grid_line_maxcol
        .min(grid.cols - grid_st().grid_line_coloff);
    grid_st().grid_line_last = 0;
    grid_st().grid_line_clear_to = 0;
    grid_st().grid_line_bg_attr = 0;
    grid_st().grid_line_clear_attr = 0;
    grid_st().grid_line_flags = 0;

    assert!(grid_st().grid_line_maxcol as usize <= linebuf_size);

    if g_st().full_screen && (opt_vars().rdb_flags & K_OPT_RDB_FLAG_INVALID) != 0 {
        assert!(linebuf_size > 0);
        g_st().linebuf_char.fill(u32::MAX);
        g_st().linebuf_attr.fill(-1);
    }
}

pub fn grid_line_getchar(mut col: i32, mut attr: Option<&mut i32>) -> Schar {
    if col < grid_st().grid_line_maxcol {
        col += grid_st().grid_line_coloff;
        let grid = unsafe { &*grid_st().grid_line_grid };
        let off = grid.line_offset[grid_st().grid_line_row as usize] + col as usize;
        if let Some(ref mut p) = attr {
            **p = grid.attrs[off];
        }
        grid.chars[off]
    } else {
        schar_from_ascii(b' ')
    }
}

pub fn grid_line_put_schar(col: i32, schar: Schar, attr: i32) {
    assert!(!grid_st().grid_line_grid.is_null());
    if col >= grid_st().grid_line_maxcol {
        return;
    }

    g_st().linebuf_char[col as usize] = schar;
    g_st().linebuf_attr[col as usize] = attr;

    grid_st().grid_line_first = grid_st().grid_line_first.min(col);
    grid_st().grid_line_last = grid_st().grid_line_last.max(col + 1);
    g_st().linebuf_vcol[col as usize] = -1;
}

pub fn grid_line_puts(
    mut col: i32,
    text: impl crate::strings::IntoCCharPtr,
    textlen: i32,
    attr: i32,
) -> i32 {
    let text = text.into_c_char_ptr();
    {
        assert!(!grid_st().grid_line_grid.is_null());
        let mut len = if textlen >= 0 { textlen } else { i32::MAX };

        let start_col = col;
        let max_col = grid_st().grid_line_maxcol;
        let mut ci = utf_ptr2_str_char_info_len(text, len);
        while col < max_col && len > 0 && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_cluster_info_len(ci, &mut len);
            let mut mbyte_cells = cli.cells;
            let mut schar = if mbyte_cells > 2 || ci.chr.value < 0x20 {
                mbyte_cells = 1;
                schar_from_char(0xFFFD)
            } else {
                schar_from_cluster(ci, cli)
            };

            if col + mbyte_cells > max_col {
                schar = schar_from_ascii(b'>');
                mbyte_cells = 1;
            }

            if ptr::eq(ci.ptr, text)
                && col > grid_st().grid_line_first
                && col < grid_st().grid_line_last
                && g_st().linebuf_char[col as usize] == 0
            {
                g_st().linebuf_char[(col - 1) as usize] = schar_from_ascii(b'>');
            }

            g_st().linebuf_char[col as usize] = schar;
            g_st().linebuf_attr[col as usize] = attr;
            g_st().linebuf_vcol[col as usize] = -1;
            if mbyte_cells == 2 {
                g_st().linebuf_char[(col + 1) as usize] = 0;
                g_st().linebuf_attr[(col + 1) as usize] = attr;
                g_st().linebuf_vcol[(col + 1) as usize] = -1;
            }

            col += mbyte_cells;
            ci = cli.next;
        }

        if col > start_col {
            grid_st().grid_line_first = grid_st().grid_line_first.min(start_col);
            grid_st().grid_line_last = grid_st().grid_line_last.max(col);
        }

        col - start_col
    }
}

pub fn grid_line_fill(start_col: i32, mut end_col: i32, sc: Schar, attr: i32) -> i32 {
    end_col = end_col.min(grid_st().grid_line_maxcol);
    if start_col >= end_col {
        return end_col;
    }

    for col in start_col..end_col {
        g_st().linebuf_char[col as usize] = sc;
        g_st().linebuf_attr[col as usize] = attr;
        g_st().linebuf_vcol[col as usize] = -1;
    }

    grid_st().grid_line_first = grid_st().grid_line_first.min(start_col);
    grid_st().grid_line_last = grid_st().grid_line_last.max(end_col);
    end_col
}

pub fn grid_line_clear_end(start_col: i32, end_col: i32, bg_attr: i32, clear_attr: i32) {
    if grid_st().grid_line_first > start_col {
        grid_st().grid_line_first = start_col;
        grid_st().grid_line_last = start_col;
    }
    grid_st().grid_line_clear_to = end_col;
    grid_st().grid_line_bg_attr = bg_attr;
    grid_st().grid_line_clear_attr = clear_attr;
}

pub fn grid_line_cursor_goto(col: i32) {
    ui_grid_cursor_goto(
        unsafe { (*grid_st().grid_line_grid).handle },
        grid_st().grid_line_row,
        col,
    );
}

pub fn grid_line_mirror(width: i32) {
    grid_st().grid_line_clear_to = grid_st().grid_line_last.max(grid_st().grid_line_clear_to);
    if grid_st().grid_line_first >= grid_st().grid_line_clear_to {
        return;
    }
    linebuf_mirror(
        &mut grid_st().grid_line_first,
        &mut grid_st().grid_line_last,
        &mut grid_st().grid_line_clear_to,
        width,
    );
    grid_st().grid_line_flags |= SLF_RIGHTLEFT;
}

pub fn linebuf_mirror(firstp: &mut i32, lastp: &mut i32, clearp: &mut i32, width: i32) {
    let first = *firstp;
    let last = *lastp;

    let n = (last - first) as usize;
    let mirror = width - 1;
    let scratch_ptr = grid_st().linebuf_scratch.as_mut_ptr();
    let scratch_char = scratch_ptr.cast::<Schar>();
    unsafe {
        ptr::copy_nonoverlapping(
            &raw const g_st().linebuf_char[first as usize],
            scratch_char.wrapping_add(first as usize),
            n,
        )
    };
    let mut col = first;
    while col < last {
        let rev = mirror - col;
        if col + 1 < last && unsafe { *scratch_char.wrapping_add((col + 1) as usize) } == 0 {
            g_st().linebuf_char[(rev - 1) as usize] =
                unsafe { *scratch_char.wrapping_add(col as usize) };
            g_st().linebuf_char[rev as usize] = 0;
            col += 1;
        } else {
            g_st().linebuf_char[rev as usize] = unsafe { *scratch_char.wrapping_add(col as usize) };
        }
        col += 1;
    }

    let scratch_attr = scratch_ptr.cast::<Sattr>();
    unsafe {
        ptr::copy_nonoverlapping(
            &raw const g_st().linebuf_attr[first as usize],
            scratch_attr.wrapping_add(first as usize),
            n,
        )
    };
    for c in first..last {
        g_st().linebuf_attr[(mirror - c) as usize] =
            unsafe { *scratch_attr.wrapping_add(c as usize) };
    }

    let scratch_vcol = scratch_ptr.cast::<Colnr>();
    unsafe {
        ptr::copy_nonoverlapping(
            &raw const g_st().linebuf_vcol[first as usize],
            scratch_vcol.wrapping_add(first as usize),
            n,
        )
    };
    for c in first..last {
        g_st().linebuf_vcol[(mirror - c) as usize] =
            unsafe { *scratch_vcol.wrapping_add(c as usize) };
    }

    *firstp = width - *clearp;
    *clearp = width - first;
    *lastp = width - last;
}

pub fn grid_line_flush() {
    let grid = grid_st().grid_line_grid;
    grid_st().grid_line_grid = ptr::null_mut();
    grid_st().grid_line_clear_to = grid_st().grid_line_last.max(grid_st().grid_line_clear_to);
    assert!(grid_st().grid_line_clear_to <= grid_st().grid_line_maxcol);
    if grid_st().grid_line_first >= grid_st().grid_line_clear_to {
        return;
    }

    grid_put_linebuf(
        unsafe { &mut *grid },
        grid_st().grid_line_row,
        grid_st().grid_line_coloff,
        grid_st().grid_line_first,
        grid_st().grid_line_last,
        grid_st().grid_line_clear_to,
        grid_st().grid_line_bg_attr,
        grid_st().grid_line_clear_attr,
        -1,
        grid_st().grid_line_flags,
    );
}

pub fn grid_line_flush_if_valid_row() {
    if grid_st().grid_line_row < 0
        || grid_st().grid_line_row >= unsafe { (*grid_st().grid_line_grid).rows }
    {
        if (opt_vars().rdb_flags & K_OPT_RDB_FLAG_INVALID) != 0 {
            std::process::abort();
        } else {
            grid_st().grid_line_grid = ptr::null_mut();
            return;
        }
    }
    grid_line_flush();
}

pub fn grid_clear(
    grid: *mut GridView,
    start_row: i32,
    end_row: i32,
    start_col: i32,
    mut end_col: i32,
    attr: i32,
) {
    for row in start_row..end_row {
        grid_line_start(grid, row);
        end_col = end_col.min(grid_st().grid_line_maxcol);
        if grid_st().grid_line_row >= unsafe { (*grid_st().grid_line_grid).rows }
            || start_col >= end_col
        {
            grid_st().grid_line_grid = ptr::null_mut();
            return;
        }
        grid_line_clear_end(start_col, end_col, attr, 0);
        grid_line_flush();
    }
}

#[inline]
fn grid_char_needs_redraw(grid: &mut ScreenGrid, col: i32, off_to: usize, cols: i32) -> bool {
    cols > 0
        && ((g_st().linebuf_char[col as usize] != grid.chars[off_to]
            || g_st().linebuf_attr[col as usize] != grid.attrs[off_to]
            || (cols > 1
                && g_st().linebuf_char[(col + 1) as usize] == 0
                && g_st().linebuf_char[(col + 1) as usize] != grid.chars[off_to + 1]))
            || (opt_vars().rdb_flags & K_OPT_RDB_FLAG_NODELTA) != 0)
}

pub fn grid_put_linebuf(
    grid: &mut ScreenGrid,
    row: i32,
    coloff: i32,
    mut col: i32,
    mut endcol: i32,
    mut clear_width: i32,
    bg_attr: i32,
    mut clear_attr: i32,
    mut last_vcol: Colnr,
    flags: i32,
) {
    let mut clear_next = false;
    assert!(0 <= row && row < grid.rows);
    if endcol > grid.cols {
        endcol = grid.cols;
    }

    if grid.chars.is_empty() || row >= grid.rows || coloff >= grid.cols {
        return;
    }

    let invalid_row =
        !ptr::eq(grid, &raw const g_st().default_grid) && grid_invalid_row(grid, row) && col == 0;
    let off_to = grid.line_offset[row as usize] + coloff as usize;
    let max_off_to = grid.line_offset[row as usize] + grid.cols as usize;

    if col > 0 && grid.chars[off_to + col as usize] == 0 {
        g_st().linebuf_char[(col - 1) as usize] = schar_from_ascii(b'>');
        g_st().linebuf_attr[(col - 1) as usize] = grid.attrs[off_to + col as usize - 1];
        col -= 1;
    }

    let mut clear_start = endcol;
    if (flags & SLF_RIGHTLEFT) != 0 {
        clear_start = col;
        col = endcol;
        endcol = clear_width;
        clear_width = col;
    }

    if opt_vars().p_arshape != 0 && opt_vars().p_tbidi == 0 && endcol > col {
        line_do_arabic_shape(&raw mut g_st().linebuf_char[col as usize], endcol - col);
    }

    if bg_attr != 0 {
        for c in col..endcol {
            g_st().linebuf_attr[c as usize] =
                hl_combine_attr(bg_attr, g_st().linebuf_attr[c as usize]);
        }
    }

    let mut redraw_next = grid_char_needs_redraw(grid, col, off_to + col as usize, endcol - col);

    let mut start_dirty: i32 = -1;
    let mut end_dirty: i32 = 0;

    while col < endcol {
        let mut char_cells: i32 = 1;
        if col + 1 < endcol && g_st().linebuf_char[(col + 1) as usize] == 0 {
            char_cells = 2;
        }
        let redraw_this = redraw_next;
        let off = off_to + col as usize;
        redraw_next = grid_char_needs_redraw(
            grid,
            col + char_cells,
            off + char_cells as usize,
            endcol - col - char_cells,
        );

        if redraw_this {
            if start_dirty == -1 {
                start_dirty = col;
            }
            end_dirty = col + char_cells;
            if col + char_cells == endcol
                && off + (char_cells as usize) < max_off_to
                && grid.chars[off + char_cells as usize] == 0
            {
                clear_next = true;
            }

            grid.chars[off] = g_st().linebuf_char[col as usize];
            if char_cells == 2 {
                grid.chars[off + 1] = g_st().linebuf_char[(col + 1) as usize];
            }

            grid.attrs[off] = g_st().linebuf_attr[col as usize];
            if char_cells == 2 {
                grid.attrs[off + 1] = g_st().linebuf_attr[col as usize];
            }
        }

        grid.vcols[off] = g_st().linebuf_vcol[col as usize];
        if char_cells == 2 {
            grid.vcols[off + 1] = g_st().linebuf_vcol[(col + 1) as usize];
        }

        col += char_cells;
    }

    if clear_next {
        grid.chars[off_to + col as usize] = schar_from_ascii(b' ');
        end_dirty += 1;
    }

    if off_to + (clear_width as usize) < max_off_to
        && grid.chars[off_to + clear_width as usize] == 0
    {
        clear_width += 1;
    }

    let mut clear_dirty_start: i32 = -1;
    let mut clear_end: i32 = -1;
    if (flags & SLF_RIGHTLEFT) != 0 {
        let mut c = clear_width - 1;
        while c >= clear_start {
            let off = off_to + c as usize;
            grid.vcols[off] = if (flags & SLF_INC_VCOL) != 0 {
                last_vcol += 1;
                last_vcol
            } else {
                last_vcol
            };
            c -= 1;
        }
    }
    clear_attr = hl_combine_attr(bg_attr, clear_attr);
    for c in clear_start..clear_width {
        let off = off_to + c as usize;
        if grid.chars[off] != schar_from_ascii(b' ')
            || grid.attrs[off] != clear_attr
            || (opt_vars().rdb_flags & K_OPT_RDB_FLAG_NODELTA) != 0
        {
            grid.chars[off] = schar_from_ascii(b' ');
            grid.attrs[off] = clear_attr;
            if clear_dirty_start == -1 {
                clear_dirty_start = c;
            }
            clear_end = c + 1;
        }
        if (flags & SLF_RIGHTLEFT) == 0 {
            grid.vcols[off] = if (flags & SLF_INC_VCOL) != 0 {
                last_vcol += 1;
                last_vcol
            } else {
                last_vcol
            };
        }
    }

    if (flags & SLF_RIGHTLEFT) != 0 && start_dirty != -1 && clear_dirty_start != -1 {
        if grid.throttled || clear_dirty_start >= start_dirty - 5 {
            start_dirty = clear_dirty_start;
        } else {
            ui_line(
                grid,
                row,
                invalid_row,
                coloff + clear_dirty_start,
                coloff + clear_dirty_start,
                coloff + clear_end,
                clear_attr,
                (flags & SLF_WRAP) != 0,
            );
        }
        clear_end = end_dirty;
    } else if start_dirty == -1 {
        start_dirty = clear_dirty_start;
        end_dirty = clear_dirty_start;
    } else if clear_end < end_dirty {
        clear_end = end_dirty;
    } else {
        end_dirty = endcol;
    }

    if clear_end > start_dirty {
        if !grid.throttled {
            ui_line(
                grid,
                row,
                invalid_row,
                coloff + start_dirty,
                coloff + end_dirty,
                coloff + clear_end,
                clear_attr,
                (flags & SLF_WRAP) != 0,
            );
        } else if !grid.dirty_col.is_empty() && clear_end > grid.dirty_col[row as usize] {
            grid.dirty_col[row as usize] = clear_end;
        }
    }
}

pub fn grid_alloc(grid: &mut ScreenGrid, rows: i32, columns: i32, copy: bool, valid: bool) {
    assert!(rows >= 0 && columns >= 0);
    let ncells = rows as usize * columns as usize;
    let mut ngrid = ScreenGrid {
        handle: grid.handle,
        chars: vec![0; ncells],
        attrs: vec![0; ncells],
        vcols: vec![-1; ncells],
        line_offset: vec![0; rows as usize],
        dirty_col: std::mem::take(&mut grid.dirty_col),
        rows,
        cols: columns,
        valid: grid.valid,
        throttled: grid.throttled,
        blending: grid.blending,
        mouse_enabled: grid.mouse_enabled,
        zindex: grid.zindex,
        comp_row: grid.comp_row,
        comp_col: grid.comp_col,
        comp_width: grid.comp_width,
        comp_height: grid.comp_height,
        comp_index: grid.comp_index,
        comp_disabled: grid.comp_disabled,
        pending_comp_index_update: grid.pending_comp_index_update,
    };

    for new_row in 0..ngrid.rows {
        let row_off = new_row as usize * ngrid.cols as usize;
        ngrid.line_offset[new_row as usize] = row_off;

        grid_clear_line(&mut ngrid, row_off, columns, valid);

        if copy && new_row < grid.rows && !grid.chars.is_empty() {
            let len = grid.cols.min(ngrid.cols) as usize;
            let old_off = grid.line_offset[new_row as usize];
            ngrid.chars[row_off..row_off + len]
                .copy_from_slice(&grid.chars[old_off..old_off + len]);
            ngrid.attrs[row_off..row_off + len]
                .copy_from_slice(&grid.attrs[old_off..old_off + len]);
            ngrid.vcols[row_off..row_off + len]
                .copy_from_slice(&grid.vcols[old_off..old_off + len]);
        }
    }
    *grid = ngrid;

    let cols = columns as usize;
    if g_st().linebuf_char.len() < cols {
        g_st().linebuf_char = vec![0; cols];
        g_st().linebuf_attr = vec![0; cols];
        g_st().linebuf_vcol = vec![0; cols];
        grid_st().linebuf_scratch = vec![0; cols];
    }
}

pub fn grid_free(grid: &mut ScreenGrid) {
    grid.chars = Vec::new();
    grid.attrs = Vec::new();
    grid.vcols = Vec::new();
    grid.line_offset = Vec::new();
}

pub fn grid_free_all_mem() {
    grid_free(&mut g_st().default_grid);
    grid_free(&mut g_st().msg_grid);
    g_st().msg_grid.dirty_col = Vec::new();
    g_st().linebuf_char = Vec::new();
    g_st().linebuf_attr = Vec::new();
    g_st().linebuf_vcol = Vec::new();
    grid_st().linebuf_scratch = Vec::new();
    grid_st().glyph_cache_keys = Vec::new();
    grid_st().glyph_cache_offsets = std::collections::BTreeMap::new();
}

pub fn win_grid_alloc(wp: &mut WinInfoInfo) {
    let grid = &mut wp.w_grid;
    let grid_allocated: *mut ScreenGrid = &raw mut wp.w_grid_alloc;
    let grid_allocated = unsafe { &mut *grid_allocated };

    let total_rows = wp.w_height_outer;
    let total_cols = wp.w_width_outer;

    let want_allocation = ui_has(K_UIMULTIGRID) || wp.w_floating;
    let has_allocation = !grid_allocated.chars.is_empty();

    if (wp.w_view_height as usize) + 1 > wp.w_lines.len() {
        wp.w_lines_valid = 0;
        wp.w_lines.clear();
        wp.w_lines
            .resize(wp.w_view_height as usize + 1, Wline::default());
    }

    let mut was_resized = false;
    if want_allocation
        && (!has_allocation
            || grid_allocated.rows != total_rows
            || grid_allocated.cols != total_cols)
    {
        let is_valid = wp.w_grid_alloc.valid;
        grid_alloc(
            &mut *grid_allocated,
            total_rows,
            total_cols,
            is_valid,
            false,
        );
        grid_allocated.valid = true;
        if wp.w_floating && wp.w_config.border {
            wp.w_redr_border = true;
        }
        was_resized = true;
    } else if !want_allocation && has_allocation {
        grid_free(&mut *grid_allocated);
        grid_allocated.valid = false;
        was_resized = true;
    } else if want_allocation && has_allocation && !wp.w_grid_alloc.valid {
        grid_invalidate(&mut *grid_allocated);
        grid_allocated.valid = true;
    }

    if want_allocation {
        grid.target = grid_allocated;
        grid.row_offset = wp.w_winrow_off;
        grid.col_offset = wp.w_wincol_off;
    } else {
        grid.target = &raw mut g_st().default_grid;
        grid.row_offset = wp.w_winrow + wp.w_winrow_off;
        grid.col_offset = wp.w_wincol + wp.w_wincol_off;
    }

    if (g_st().resizing_screen || was_resized) && want_allocation {
        ui_call_grid_resize(
            i64::from(grid_allocated.handle),
            i64::from(grid_allocated.cols),
            i64::from(grid_allocated.rows),
        );
        ui_check_cursor_grid(grid_allocated.handle);
    }
}

pub fn grid_assign_handle(grid: &mut ScreenGrid) {
    if grid.handle == 0 {
        grid_st().last_grid_handle += 1;
        grid.handle = grid_st().last_grid_handle;
    }
}

pub fn grid_ins_lines(
    grid: &mut ScreenGrid,
    row: i32,
    line_count: i32,
    end: i32,
    col: i32,
    width: i32,
) {
    if line_count <= 0 {
        return;
    }

    for i in 0..line_count {
        if width != grid.cols {
            let mut j = end - 1 - i;
            while {
                j -= line_count;
                j >= row
            } {
                linecopy(grid, j + line_count, j, col, width);
            }
            j += line_count;
            let clear_off = grid.line_offset[j as usize] + col as usize;
            grid_clear_line(grid, clear_off, width, false);
        } else {
            let mut j = end - 1 - i;
            let temp = grid.line_offset[j as usize] as u32 as usize;
            while {
                j -= line_count;
                j >= row
            } {
                grid.line_offset[(j + line_count) as usize] = grid.line_offset[j as usize];
            }
            grid.line_offset[(j + line_count) as usize] = temp;
            let cols = grid.cols;
            grid_clear_line(grid, temp, cols, false);
        }
    }

    if !grid.throttled {
        ui_call_grid_scroll(
            i64::from(grid.handle),
            i64::from(row),
            i64::from(end),
            i64::from(col),
            i64::from(col + width),
            i64::from(-line_count),
            0,
        );
    }
}

pub fn grid_del_lines(
    grid: &mut ScreenGrid,
    row: i32,
    line_count: i32,
    end: i32,
    col: i32,
    width: i32,
) {
    if line_count <= 0 {
        return;
    }

    for i in 0..line_count {
        if width != grid.cols {
            let mut j = row + i;
            while {
                j += line_count;
                j < end
            } {
                linecopy(grid, j - line_count, j, col, width);
            }
            j -= line_count;
            let clear_off = grid.line_offset[j as usize] + col as usize;
            grid_clear_line(grid, clear_off, width, false);
        } else {
            let mut j = row + i;
            let temp = grid.line_offset[j as usize] as u32 as usize;
            while {
                j += line_count;
                j < end
            } {
                grid.line_offset[(j - line_count) as usize] = grid.line_offset[j as usize];
            }
            grid.line_offset[(j - line_count) as usize] = temp;
            let cols = grid.cols;
            grid_clear_line(grid, temp, cols, false);
        }
    }

    if !grid.throttled {
        ui_call_grid_scroll(
            i64::from(grid.handle),
            i64::from(row),
            i64::from(end),
            i64::from(col),
            i64::from(col + width),
            i64::from(line_count),
            0,
        );
    }
}

fn grid_draw_bordertext(
    vt: &[VirtTextChunk],
    mut col: i32,
    winbl: i32,
    hl_attr: Option<&crate::highlight::NSHlAttr>,
    win_hl_attr_normal: i32,
    bt: BorderTextType,
    mut overflow: i32,
) {
    let hlf_idx = if bt == K_BORDER_TEXT_TITLE {
        HLF_BTITLE
    } else {
        HLF_BFOOTER
    };
    let mut default_attr = if let Some(tbl) = hl_attr {
        tbl[hlf_idx].get()
    } else {
        crate::highlight::hl_attr(hlf_idx as i32)
    };
    default_attr = hl_apply_winblend(winbl, default_attr);
    default_attr = hl_combine_attr(win_hl_attr_normal, default_attr);

    if overflow > 0 {
        grid_line_puts(1, c"<", -1, default_attr);
        col += 1;
        overflow += 1;
    }

    let mut i: usize = 0;
    while i < vt.len() {
        let mut attr: i32 = -1;
        let Some(text_cstr) = next_virt_text_chunk(vt, &mut i, &mut attr) else {
            break;
        };
        let mut text = text_cstr.as_ptr().cast_mut();
        if overflow > 0 {
            let cells = mb_string2cells(text) as i32;
            if overflow >= cells {
                overflow -= cells;
                continue;
            }
            let mut p = text;
            while !crate::memory::cstr_is_empty(p) && overflow > 0 {
                overflow -= utf_ptr2cells(p);
                p = p.wrapping_add(utfc_ptr2len(p) as usize);
            }
            text = p;
        }
        if attr == -1 {
            attr = default_attr;
        } else {
            attr = hl_apply_winblend(winbl, attr);
            attr = hl_combine_attr(win_hl_attr_normal, attr);
        }
        col += grid_line_puts(col, text, -1, attr);
    }
}

fn get_bordertext_col(total_col: i32, text_width: i32, align: AlignTextPos) -> i32 {
    match align {
        K_ALIGN_LEFT => 1,
        K_ALIGN_CENTER => ((total_col - text_width) / 2 + 1).max(1),
        K_ALIGN_RIGHT => (total_col - text_width + 1).max(1),
        _ => unreachable!(),
    }
}

/// Draw border on floating window grid (`grid_draw_border`).
pub fn grid_draw_border(
    grid: &mut ScreenGrid,
    config: &mut WinConfig,
    mut adj: *mut i32,
    winbl: i32,
    hl_attr: Option<&crate::highlight::NSHlAttr>,
    win_hl_attr_normal: i32,
) {
    let mut default_adj: [i32; 4] = [1, 1, 1, 1];
    if adj.is_null() {
        adj = default_adj.as_mut_ptr();
    }
    let mut chars = [0 as Schar; 8];
    let mut border_attrs = [0_i32; 8];

    for i in 0..8 {
        let bchars = &config.border_chars[i];
        let blen = bchars.iter().position(|&b| b == 0).unwrap_or(bchars.len());
        chars[i] = schar_from_bytes(&bchars[..blen]);
        border_attrs[i] = hl_combine_attr(win_hl_attr_normal, config.border_attr[i]);
    }

    let adj0 = unsafe { *adj };
    let adj1 = unsafe { *adj.wrapping_add(1) };
    let adj2 = unsafe { *adj.wrapping_add(2) };
    let adj3 = unsafe { *adj.wrapping_add(3) };

    let irow = grid.rows - adj0 - adj2;
    let icol = grid.cols - adj1 - adj3;

    if adj0 != 0 {
        screengrid_line_start(grid, 0, 0);
        if adj3 != 0 {
            grid_line_put_schar(0, chars[0], border_attrs[0]);
        }

        for i in 0..icol {
            grid_line_put_schar(i + adj3, chars[1], border_attrs[1]);
        }

        if config.title {
            let title_col = get_bordertext_col(icol, config.title_width, config.title_pos);
            grid_draw_bordertext(
                &config.title_chunks,
                title_col,
                winbl,
                hl_attr,
                win_hl_attr_normal,
                K_BORDER_TEXT_TITLE,
                config.title_width - icol,
            );
        }
        if adj1 != 0 {
            grid_line_put_schar(icol + adj3, chars[2], border_attrs[2]);
        }
        grid_line_flush();
    }

    for i in 0..irow {
        if adj3 != 0 {
            screengrid_line_start(grid, i + adj0, 0);
            grid_line_put_schar(0, chars[7], border_attrs[7]);
            grid_line_flush();
        }
        if adj1 != 0 {
            let ic = if i == 0 && adj0 == 0 && chars[2] != 0 {
                2
            } else {
                3
            };
            screengrid_line_start(grid, i + adj0, 0);
            grid_line_put_schar(icol + adj3, chars[ic], border_attrs[ic]);
            grid_line_flush();
        }
    }

    if adj2 != 0 {
        screengrid_line_start(grid, irow + adj0, 0);
        if adj3 != 0 {
            grid_line_put_schar(0, chars[6], border_attrs[6]);
        }

        for i in 0..icol {
            let ic = if i == 0 && adj3 == 0 && chars[6] != 0 {
                6
            } else {
                5
            };
            grid_line_put_schar(i + adj3, chars[ic], border_attrs[ic]);
        }

        if config.footer {
            let footer_col = get_bordertext_col(icol, config.footer_width, config.footer_pos);
            grid_draw_bordertext(
                &config.footer_chunks,
                footer_col,
                winbl,
                hl_attr,
                win_hl_attr_normal,
                K_BORDER_TEXT_FOOTER,
                config.footer_width - icol,
            );
        }
        if adj1 != 0 {
            grid_line_put_schar(icol + adj3, chars[4], border_attrs[4]);
        }
        grid_line_flush();
    }
}

fn linecopy(grid: &mut ScreenGrid, to: i32, from: i32, col: i32, width: i32) {
    let off_to = (grid.line_offset[to as usize] + col as usize) as u32 as usize;
    let off_from = (grid.line_offset[from as usize] + col as usize) as u32 as usize;
    let len = width as usize;

    grid.chars.copy_within(off_from..off_from + len, off_to);
    grid.attrs.copy_within(off_from..off_from + len, off_to);
    grid.vcols.copy_within(off_from..off_from + len, off_to);
}

pub fn get_win_by_grid_handle(handle: ObjHandle) -> *mut WinInfoInfo {
    for wp in for_all_windows_ptr() {
        if wp.w_grid_alloc.handle == handle {
            return wp;
        }
    }
    ptr::null_mut()
}

/// Put a unicode character in a screen cell (`schar_from_char`).
pub fn schar_from_char(mut c: i32) -> Schar {
    let mut sc: Schar = 0;
    if c >= 0x20_0000 {
        c = 0xFFFD;
    }
    utf_char2bytes(c, (&raw mut sc).cast());
    sc
}

#[cfg(target_endian = "big")]
pub const BRAILLE_SPACE: Schar = 0xe2a0_8000;
#[cfg(not(target_endian = "big"))]
pub const BRAILLE_SPACE: Schar = 0x0080_a0e2;

#[inline]
pub fn schar_invisible(sc: Schar) -> bool {
    sc == schar_from_ascii(b' ') || sc == BRAILLE_SPACE
}

pub fn schar_from_cluster_impl(
    buf: impl crate::strings::IntoCCharPtr,
    mut len: usize,
    first_compose: bool,
) -> Schar {
    let buf = buf.into_c_char_ptr();
    {
        let maxlen = MAX_SCHAR_SIZE - 1 - usize::from(first_compose);
        if len > maxlen {
            len = maxlen;
            while (unsafe { *buf.wrapping_add(len).cast::<u8>() } & 0xC0) != 0xC0 {
                len -= 1;
            }
        }

        if first_compose {
            let mut cbuf = [0u8; MAX_SCHAR_SIZE];
            cbuf[0] = b' ';
            unsafe {
                ptr::copy_nonoverlapping(buf.cast::<u8>(), cbuf.as_mut_ptr().wrapping_add(1), len)
            };
            schar_from_buf(cbuf.as_ptr().cast(), len + 1)
        } else {
            schar_from_buf(buf, len)
        }
    }
}

#[inline]
pub fn schar_from_cluster(ci: StrCharInfo, cli: ClusterInfo) -> Schar {
    assert!(ci.chr.value >= 0);
    let len = crate::memory::ptr_offset_from(cli.next.ptr, ci.ptr) as usize;
    if len == 1 {
        return schar_from_ascii(ci.chr.value as u8);
    }
    if cli.first_compose || len > 4 {
        return schar_from_cluster_impl(ci.ptr, len, cli.first_compose);
    }
    let mut sc: Schar = 0;
    unsafe {
        ptr::copy_nonoverlapping(ci.ptr.cast::<u8>(), (&raw mut sc).cast::<u8>(), len);
    }
    sc
}

#[cfg(test)]
pub fn unittest_schar_from_cluster(buf: &std::ffi::CStr, firstc: &mut i32) -> Schar {
    let ci = crate::mbyte::utf_ptr2_str_char_info(buf.as_ptr());
    let cli = crate::mbyte::utf_cluster_info(ci);
    *firstc = ci.chr.value;
    if ci.chr.value >= 0 {
        schar_from_cluster(ci, cli)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schar_roundtrip_ascii_and_unicode() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let sc_a = schar_from_ascii(b'A');
            assert_eq!(schar_get_ascii(sc_a), b'A');

            let sc_u = schar_from_char(0x2603);
            let mut buf = [0u8; 16];
            let len = schar_get(buf.as_mut_ptr(), sc_u);
            assert_eq!(len, 3);
            assert_eq!(
                std::ffi::CStr::from_bytes_until_nul(&buf)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "☃"
            );
        }
    }
}
