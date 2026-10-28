//! Low-level functions to manipulate individual character cells on the screen grid (`src/nvim/grid.c`).

use crate::arabic::arabic_shape;
use crate::cursor::{colnr_T, linenr_T};
use crate::decoration::{VirtTextChunk, decor_check_invalid_glyphs, next_virt_text_chunk};
use crate::globals::*;
use crate::highlight::{hl_apply_winblend, hl_combine_attr};
use crate::mbyte::{
    mb_string2cells, utf_ClusterInfo_len, utf_char2bytes, utf_char2len, utf_cp_bounds,
    utf_ptr2StrCharInfo_len, utf_ptr2cells, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::msgpack_rpc::packer::handle_T;
use crate::option_vars::*;
use crate::optionstr::check_chars_options;
use crate::plines::{ClusterInfo, StrCharInfo};
use crate::ui::{
    ui_call_grid_resize, ui_call_grid_scroll, ui_check_cursor_grid, ui_grid_cursor_goto, ui_has,
    ui_line,
};
use crate::window::{WinConfig, win_T};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

pub type schar_T = u32;
pub type sattr_T = i32;
pub type sscratch_T = i32;
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::win_config::{
    AlignTextPos, BorderTextType, kAlignCenter, kAlignLeft, kAlignRight, kBorderTextFooter,
    kBorderTextTitle, kUIMultigrid,
};
use crate::charset::NUL;
use crate::drawline::{SLF_INC_VCOL, SLF_RIGHTLEFT, SLF_WRAP};
use crate::drawscreen::DEFAULT_GRID_HANDLE;

pub(crate) const kOptRdbFlagInvalid: c_uint = 0x04;
const kOptRdbFlagNodelta: c_uint = 0x08;

const HLF_BTITLE: usize = 68;
const HLF_BFOOTER: usize = 69;

#[repr(C)]
#[derive(Clone, Default)]
pub struct ScreenGrid {
    pub handle: handle_T,
    pub chars: Vec<u32>,
    pub attrs: Vec<i32>,
    pub vcols: Vec<i32>,
    pub line_offset: Vec<usize>,
    pub dirty_col: Vec<c_int>,
    pub rows: c_int,
    pub cols: c_int,
    pub valid: bool,
    pub throttled: bool,
    pub blending: bool,
    pub mouse_enabled: bool,
    pub zindex: c_int,
    pub comp_row: c_int,
    pub comp_col: c_int,
    pub comp_width: c_int,
    pub comp_height: c_int,
    pub comp_index: usize,
    pub comp_disabled: bool,
    pub pending_comp_index_update: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GridView {
    pub target: *mut ScreenGrid,
    pub row_offset: c_int,
    pub col_offset: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharBoundsOff {
    pub begin_off: i8,
    pub end_off: i8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct wline_T {
    pub wl_lnum: linenr_T,
    pub wl_size: u16,
    pub wl_valid: bool,
    pub wl_folded: bool,
    pub wl_foldend: linenr_T,
    pub wl_lastlnum: linenr_T,
}

struct GridModuleState {
    linebuf_scratch: Vec<sscratch_T>,
    glyph_cache_keys: Vec<c_char>,
    glyph_cache_offsets: std::collections::BTreeMap<Vec<u8>, u32>,
    grid_line_grid: *mut ScreenGrid,
    grid_line_row: c_int,
    grid_line_coloff: c_int,
    grid_line_maxcol: c_int,
    grid_line_first: c_int,
    grid_line_last: c_int,
    grid_line_clear_to: c_int,
    grid_line_bg_attr: c_int,
    grid_line_clear_attr: c_int,
    grid_line_flags: c_int,
    last_grid_handle: c_int,
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
            grid_line_first: c_int::MAX,
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
pub const fn schar_from_ascii(x: u8) -> schar_T {
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
const fn schar_idx(sc: schar_T) -> u32 {
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
const fn arabic_char(ch: c_int) -> bool {
    (ch & 0xFF00) == 0x0600
}

#[inline]
fn strnlen_4(p: *const u8) -> usize {
    unsafe {
        for i in 0..4 {
            if *p.add(i) == 0 {
                return i;
            }
        }
        4
    }
}

/// Determine if dedicated window grid should be used or the `default_grid` (`grid_adjust`).
pub fn grid_adjust(
    grid: *mut GridView,
    row_off: &mut c_int,
    col_off: &mut c_int,
) -> *mut ScreenGrid {
    unsafe {
        *row_off += (*grid).row_offset;
        *col_off += (*grid).col_offset;
        (*grid).target
    }
}

pub fn schar_from_str(str_: *const c_char) -> schar_T {
    unsafe {
        if str_.is_null() {
            return 0;
        }
        schar_from_buf(str_, CStr::from_ptr(str_).to_bytes().len())
    }
}

pub fn schar_from_buf(buf: *const c_char, len: usize) -> schar_T {
    unsafe {
        assert!(len < MAX_SCHAR_SIZE);
        if len <= 4 {
            let mut sc: schar_T = 0;
            ptr::copy_nonoverlapping(buf.cast::<u8>(), (&raw mut sc).cast::<u8>(), len);
            sc
        } else {
            let bytes = std::slice::from_raw_parts(buf.cast::<u8>(), len);
            let keys_ptr = &raw mut grid_st().glyph_cache_keys;
            let offsets_ptr = &raw mut grid_st().glyph_cache_offsets;
            let idx = if let Some(&existing_idx) = (*offsets_ptr).get(bytes) {
                existing_idx
            } else {
                let new_idx = (*keys_ptr).len() as u32;
                (*keys_ptr).extend(bytes.iter().map(|&b| b as c_char));
                (*keys_ptr).push(0);
                (*offsets_ptr).insert(bytes.to_vec(), new_idx);
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
}

pub fn schar_cache_clear_if_full() -> bool {
    unsafe {
        let keys_ptr = &raw const grid_st().glyph_cache_keys;
        if (*keys_ptr).len() > (1 << 21) {
            schar_cache_clear();
            return true;
        }
        false
    }
}

pub fn schar_cache_clear() {
    unsafe {
        decor_check_invalid_glyphs();
        let keys_ptr = &raw mut grid_st().glyph_cache_keys;
        (*keys_ptr).clear();
        let offsets_ptr = &raw mut grid_st().glyph_cache_offsets;
        (*offsets_ptr).clear();

        if !check_chars_options().is_null() {
            std::process::abort();
        }
    }
}

pub fn schar_high(sc: schar_T) -> bool {
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
pub fn schar_get(mut buf_out: *mut c_char, sc: schar_T) -> usize {
    unsafe {
        let len = schar_get_adv(&raw mut buf_out, sc);
        *buf_out = NUL;
        len
    }
}

/// Advance `buf_out`. Do NOT set final NUL (`schar_get_adv`).
pub fn schar_get_adv(buf_out: *mut *mut c_char, sc: schar_T) -> usize {
    unsafe {
        let len: usize;
        if schar_high(sc) {
            let idx = schar_idx(sc) as usize;
            let keys_ptr = &raw const grid_st().glyph_cache_keys;
            assert!(idx < (*keys_ptr).len());
            let key_ptr = (*keys_ptr).as_ptr().add(idx);
            len = CStr::from_ptr(key_ptr).to_bytes().len();
            ptr::copy_nonoverlapping(key_ptr, *buf_out, len);
        } else {
            let sc_ptr = (&raw const sc).cast::<u8>();
            len = strnlen_4(sc_ptr);
            ptr::copy_nonoverlapping(sc_ptr.cast::<c_char>(), *buf_out, len);
        }
        *buf_out = (*buf_out).add(len);
        len
    }
}

pub fn schar_len(sc: schar_T) -> usize {
    unsafe {
        if schar_high(sc) {
            let idx = schar_idx(sc) as usize;
            let keys_ptr = &raw const grid_st().glyph_cache_keys;
            assert!(idx < (*keys_ptr).len());
            CStr::from_ptr((*keys_ptr).as_ptr().add(idx))
                .to_bytes()
                .len()
        } else {
            strnlen_4((&raw const sc).cast::<u8>())
        }
    }
}

#[inline]
fn schar_get_first_byte(sc: schar_T) -> c_char {
    unsafe {
        let keys_ptr = &raw const grid_st().glyph_cache_keys;
        assert!(!(schar_high(sc) && (schar_idx(sc) as usize) >= (*keys_ptr).len()));
        if schar_high(sc) {
            (*keys_ptr).as_slice()[schar_idx(sc) as usize]
        } else {
            *(&raw const sc).cast::<c_char>()
        }
    }
}

pub fn schar_get_first_codepoint(sc: schar_T) -> c_int {
    let mut sc_buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(sc_buf.as_mut_ptr(), sc);
        utf_ptr2char(sc_buf.as_ptr())
}

pub fn schar_get_ascii(sc: schar_T) -> c_char {
    #[cfg(target_endian = "big")]
    unsafe {
        if (sc & 0x80FF_FFFF) == 0 {
            *(&raw const sc).cast::<c_char>()
        } else {
            NUL
        }
    }
    #[cfg(not(target_endian = "big"))]
    {
        if sc < 0x80 { sc as c_char } else { NUL }
    }
}

#[inline]
fn schar_in_arabic_block(sc: schar_T) -> bool {
    let first_byte = schar_get_first_byte(sc) as u8;
        (first_byte & 0xFE) == 0xD8
}

#[inline]
fn schar_get_first_two_codepoints(sc: schar_T, c0: &mut c_int, c1: &mut c_int) {
    unsafe {
        let mut sc_buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(sc_buf.as_mut_ptr(), sc);

        *c0 = utf_ptr2char(sc_buf.as_ptr());
        let len = utf_ptr2len(sc_buf.as_ptr());
        if *c0 == 0 {
            *c1 = 0;
        } else {
            *c1 = utf_ptr2char(sc_buf.as_ptr().add(len as usize));
        }
    }
}

pub fn line_do_arabic_shape(buf: *mut schar_T, cols: c_int) {
    unsafe {
        let mut i: c_int = 0;
        while i < cols {
            if schar_in_arabic_block(*buf.add(i as usize)) {
                break;
            }
            i += 1;
        }

        if i == cols {
            return;
        }

        let mut c0prev: c_int = 0;
        let mut c0: c_int = 0;
        let mut c1: c_int = 0;
        schar_get_first_two_codepoints(*buf.add(i as usize), &mut c0, &mut c1);

        while i < cols {
            let mut c0next: c_int = 0;
            let mut c1next: c_int = 0;
            let next_sc = if i + 1 < cols {
                *buf.add((i + 1) as usize)
            } else {
                0
            };
            schar_get_first_two_codepoints(next_sc, &mut c0next, &mut c1next);

            if arabic_char(c0) {
                let mut c1new = c1;
                let c0new = arabic_shape(c0, &mut c1new, c0next, c1next, c0prev);

                if !(c0new == c0 && c1new == c1) {
                    let mut scbuf = [0 as c_char; MAX_SCHAR_SIZE];
                    schar_get(scbuf.as_mut_ptr(), *buf.add(i as usize));

                    let mut scbuf_new = [0 as c_char; MAX_SCHAR_SIZE];
                    let mut len = utf_char2bytes(c0new, scbuf_new.as_mut_ptr()) as usize;
                    if c1new != 0 {
                        len += utf_char2bytes(c1new, scbuf_new.as_mut_ptr().add(len)) as usize;
                    }

                    let off =
                        (utf_char2len(c0) + if c1 != 0 { utf_char2len(c1) } else { 0 }) as usize;
                    let mut rest = CStr::from_ptr(scbuf.as_ptr().add(off)).to_bytes().len();
                    if rest + len + 1 > MAX_SCHAR_SIZE {
                        let bounds = utf_cp_bounds(
                            scbuf.as_ptr().add(off),
                            scbuf.as_ptr().add(off + rest - 1),
                        );
                        rest = (rest as isize - (isize::from(bounds.begin_off) + 1)) as usize;
                    }
                    ptr::copy_nonoverlapping(
                        scbuf.as_ptr().add(off),
                        scbuf_new.as_mut_ptr().add(len),
                        rest,
                    );
                    *buf.add(i as usize) = schar_from_buf(scbuf_new.as_ptr(), len + rest);
                }
            }

            c0prev = c0;
            c0 = c0next;
            c1 = c1next;
            i += 1;
        }
    }
}

#[inline]
pub fn grid_allocated(grid: &ScreenGrid) -> bool {
    !grid.chars.is_empty()
}

/// Clear a line in the grid starting at `off` until `width` characters are cleared (`grid_clear_line`).
pub fn grid_clear_line(grid: &mut ScreenGrid, off: usize, width: c_int, valid: bool) {
    {
        let grid = &mut *grid;
        let len = width as usize;
        grid.chars[off..off + len].fill(schar_from_ascii(b' '));
        let fill: sattr_T = if valid { 0 } else { -1 };
        grid.attrs[off..off + len].fill(fill);
        grid.vcols[off..off + len].fill(-1);
    }
}

pub fn grid_invalidate(grid: *mut ScreenGrid) {
    unsafe {
        let grid = &mut *grid;
        let len = grid.rows as usize * grid.cols as usize;
        grid.attrs[..len].fill(-1);
    }
}

#[inline]
fn grid_invalid_row(grid: &mut ScreenGrid, row: c_int) -> bool {
    {
        let grid = &*grid;
        grid.attrs[grid.line_offset[row as usize]] < 0
    }
}

/// Get a single character directly from `grid.chars` (`grid_getchar`).
pub fn grid_getchar(
    grid: *mut ScreenGrid,
    row: c_int,
    col: c_int,
    attrp: Option<&mut c_int>,
) -> schar_T {
    unsafe {
        let grid = &*grid;
        if grid.chars.is_empty() || row >= grid.rows || col >= grid.cols {
            return 0;
        }

        let off = grid.line_offset[row as usize] + col as usize;
        if let Some(attrp) = attrp {
            *attrp = grid.attrs[off];
        }
        grid.chars[off]
    }
}

pub fn grid_line_start(view: *mut GridView, mut row: c_int) {
    let mut col: c_int = 0;
        let grid = grid_adjust(view, &mut row, &mut col);
        screengrid_line_start(grid, row, col);
}

pub fn screengrid_line_start(grid: *mut ScreenGrid, row: c_int, col: c_int) {
    unsafe {
        let lbuf_char_ptr = &raw mut g_st().linebuf_char;
        let lbuf_attr_ptr = &raw mut g_st().linebuf_attr;
        let linebuf_size = (*lbuf_char_ptr).len();
        grid_st().grid_line_maxcol = (*grid).cols;
        assert!(grid_st().grid_line_grid.is_null());
        grid_st().grid_line_row = row;
        grid_st().grid_line_grid = grid;
        grid_st().grid_line_coloff = col;
        grid_st().grid_line_first = linebuf_size as c_int;
        grid_st().grid_line_maxcol = grid_st().grid_line_maxcol.min((*grid).cols - grid_st().grid_line_coloff);
        grid_st().grid_line_last = 0;
        grid_st().grid_line_clear_to = 0;
        grid_st().grid_line_bg_attr = 0;
        grid_st().grid_line_clear_attr = 0;
        grid_st().grid_line_flags = 0;

        assert!(grid_st().grid_line_maxcol as usize <= linebuf_size);

        if g_st().full_screen && (opt_vars().rdb_flags & kOptRdbFlagInvalid) != 0 {
            assert!(linebuf_size > 0);
            (&mut *lbuf_char_ptr).fill(u32::MAX);
            (&mut *lbuf_attr_ptr).fill(-1);
        }
    }
}

pub fn grid_line_getchar(mut col: c_int,mut attr: Option<&mut c_int>) -> schar_T {
    unsafe {
        if col < grid_st().grid_line_maxcol {
            col += grid_st().grid_line_coloff;
            let grid = &*grid_st().grid_line_grid;
            let off = grid.line_offset[grid_st().grid_line_row as usize] + col as usize;
            if let Some(ref mut p) = attr {
            **p = grid.attrs[off];
        }
            grid.chars[off]
        } else {
            schar_from_ascii(b' ')
        }
    }
}

pub fn grid_line_put_schar(col: c_int, schar: schar_T, attr: c_int) {
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
    mut col: c_int,
    text: *const c_char,
    textlen: c_int,
    attr: c_int,
) -> c_int {
    unsafe {
        assert!(!grid_st().grid_line_grid.is_null());
        let mut len = if textlen >= 0 { textlen } else { c_int::MAX };

        let start_col = col;
        let max_col = grid_st().grid_line_maxcol;
        let mut ci = utf_ptr2StrCharInfo_len(text, len);
        while col < max_col && len > 0 && *ci.ptr != NUL {
            let cli = utf_ClusterInfo_len(ci, &mut len);
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

pub fn grid_line_fill(
    start_col: c_int,
    mut end_col: c_int,
    sc: schar_T,
    attr: c_int,
) -> c_int {
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

pub fn grid_line_clear_end(
    start_col: c_int,
    end_col: c_int,
    bg_attr: c_int,
    clear_attr: c_int,
) {
    if grid_st().grid_line_first > start_col {
            grid_st().grid_line_first = start_col;
            grid_st().grid_line_last = start_col;
        }
        grid_st().grid_line_clear_to = end_col;
        grid_st().grid_line_bg_attr = bg_attr;
        grid_st().grid_line_clear_attr = clear_attr;
}

pub fn grid_line_cursor_goto(col: c_int) {
    unsafe {
        ui_grid_cursor_goto((*grid_st().grid_line_grid).handle, grid_st().grid_line_row, col);
    }
}

pub fn grid_line_mirror(width: c_int) {
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

pub fn linebuf_mirror(
    firstp: &mut c_int,
    lastp: &mut c_int,
    clearp: &mut c_int,
    width: c_int,
) {
    unsafe {
        let first = *firstp;
        let last = *lastp;

        let n = (last - first) as usize;
        let mirror = width - 1;
        let scratch = &raw mut grid_st().linebuf_scratch;
        let scratch_ptr = (*scratch).as_mut_ptr();
        let scratch_char = scratch_ptr.cast::<schar_T>();
        ptr::copy_nonoverlapping(
            &raw const g_st().linebuf_char[first as usize],
            scratch_char.add(first as usize),
            n,
        );
        let mut col = first;
        while col < last {
            let rev = mirror - col;
            if col + 1 < last && *scratch_char.add((col + 1) as usize) == 0 {
                g_st().linebuf_char[(rev - 1) as usize] = *scratch_char.add(col as usize);
                g_st().linebuf_char[rev as usize] = 0;
                col += 1;
            } else {
                g_st().linebuf_char[rev as usize] = *scratch_char.add(col as usize);
            }
            col += 1;
        }

        let scratch_attr = scratch_ptr.cast::<sattr_T>();
        ptr::copy_nonoverlapping(
            &raw const g_st().linebuf_attr[first as usize],
            scratch_attr.add(first as usize),
            n,
        );
        for c in first..last {
            g_st().linebuf_attr[(mirror - c) as usize] = *scratch_attr.add(c as usize);
        }

        let scratch_vcol = scratch_ptr.cast::<colnr_T>();
        ptr::copy_nonoverlapping(
            &raw const g_st().linebuf_vcol[first as usize],
            scratch_vcol.add(first as usize),
            n,
        );
        for c in first..last {
            g_st().linebuf_vcol[(mirror - c) as usize] = *scratch_vcol.add(c as usize);
        }

        *firstp = width - *clearp;
        *clearp = width - first;
        *lastp = width - last;
    }
}

pub fn grid_line_flush() {
    unsafe {
        let grid = grid_st().grid_line_grid;
        grid_st().grid_line_grid = ptr::null_mut();
        grid_st().grid_line_clear_to = grid_st().grid_line_last.max(grid_st().grid_line_clear_to);
        assert!(grid_st().grid_line_clear_to <= grid_st().grid_line_maxcol);
        if grid_st().grid_line_first >= grid_st().grid_line_clear_to {
            return;
        }

        grid_put_linebuf(
            &mut *grid,
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
}

pub fn grid_line_flush_if_valid_row() {
    unsafe {
        if grid_st().grid_line_row < 0 || grid_st().grid_line_row >= (*grid_st().grid_line_grid).rows {
            if (opt_vars().rdb_flags & kOptRdbFlagInvalid) != 0 {
                std::process::abort();
            } else {
                grid_st().grid_line_grid = ptr::null_mut();
                return;
            }
        }
        grid_line_flush();
    }
}

pub fn grid_clear(
    grid: *mut GridView,
    start_row: c_int,
    end_row: c_int,
    start_col: c_int,
    mut end_col: c_int,
    attr: c_int,
) {
    unsafe {
        for row in start_row..end_row {
            grid_line_start(grid, row);
            end_col = end_col.min(grid_st().grid_line_maxcol);
            if grid_st().grid_line_row >= (*grid_st().grid_line_grid).rows || start_col >= end_col {
                grid_st().grid_line_grid = ptr::null_mut();
                return;
            }
            grid_line_clear_end(start_col, end_col, attr, 0);
            grid_line_flush();
        }
    }
}

#[inline]
fn grid_char_needs_redraw(
    grid: &mut ScreenGrid,
    col: c_int,
    off_to: usize,
    cols: c_int,
) -> bool {
    {
        let grid = &*grid;
        cols > 0
            && ((g_st().linebuf_char[col as usize] != grid.chars[off_to]
                || g_st().linebuf_attr[col as usize] != grid.attrs[off_to]
                || (cols > 1
                    && g_st().linebuf_char[(col + 1) as usize] == 0
                    && g_st().linebuf_char[(col + 1) as usize] != grid.chars[off_to + 1]))
                || (opt_vars().rdb_flags & kOptRdbFlagNodelta) != 0)
    }
}

pub fn grid_put_linebuf(
    grid: &mut ScreenGrid,
    row: c_int,
    coloff: c_int,
    mut col: c_int,
    mut endcol: c_int,
    mut clear_width: c_int,
    bg_attr: c_int,
    mut clear_attr: c_int,
    mut last_vcol: colnr_T,
    flags: c_int,
) {
    let mut clear_next = false;
        assert!(0 <= row && row < grid.rows);
        if endcol > grid.cols {
            endcol = grid.cols;
        }

        if grid.chars.is_empty() || row >= grid.rows || coloff >= grid.cols {
            return;
        }

        let invalid_row = !ptr::eq(grid, &raw const g_st().default_grid)
            && grid_invalid_row(grid, row)
            && col == 0;
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
                g_st().linebuf_attr[c as usize] = hl_combine_attr(bg_attr, g_st().linebuf_attr[c as usize]);
            }
        }

        let mut redraw_next =
            grid_char_needs_redraw(grid, col, off_to + col as usize, endcol - col);

        let mut start_dirty: c_int = -1;
        let mut end_dirty: c_int = 0;

        while col < endcol {
            let mut char_cells: c_int = 1;
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

        let mut clear_dirty_start: c_int = -1;
        let mut clear_end: c_int = -1;
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
                || (opt_vars().rdb_flags & kOptRdbFlagNodelta) != 0
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

pub fn grid_alloc(
    grid: &mut ScreenGrid,
    rows: c_int,
    columns: c_int,
    copy: bool,
    valid: bool,
) {
    unsafe {
        
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
        let lbuf_char = &raw const g_st().linebuf_char;
        if (*lbuf_char).len() < cols {
            g_st().linebuf_char = vec![0; cols];
            g_st().linebuf_attr = vec![0; cols];
            g_st().linebuf_vcol = vec![0; cols];
            grid_st().linebuf_scratch = vec![0; cols];
        }
    }
}

pub fn grid_free(grid: &mut ScreenGrid) {
    grid.chars = Vec::new();
        grid.attrs = Vec::new();
        grid.vcols = Vec::new();
        grid.line_offset = Vec::new();
}

pub fn grid_free_all_mem() {
    unsafe {
        grid_free(&mut g_st().default_grid);
        grid_free(&mut g_st().msg_grid);
        g_st().msg_grid.dirty_col = Vec::new();
        g_st().linebuf_char = Vec::new();
        g_st().linebuf_attr = Vec::new();
        g_st().linebuf_vcol = Vec::new();
        grid_st().linebuf_scratch = Vec::new();
        let keys_ptr = &raw mut grid_st().glyph_cache_keys;
        *keys_ptr = Vec::new();
        let offsets_ptr = &raw mut grid_st().glyph_cache_offsets;
        *offsets_ptr = std::collections::BTreeMap::new();
    }
}

pub fn win_grid_alloc(wp: &mut win_T) {
    unsafe {
        let grid = &raw mut wp.w_grid;
        let grid_allocated = &raw mut wp.w_grid_alloc;

        let total_rows = wp.w_height_outer;
        let total_cols = wp.w_width_outer;

        let want_allocation = ui_has(kUIMultigrid) || wp.w_floating;
        let has_allocation = !(*grid_allocated).chars.is_empty();

        if (wp.w_view_height as usize) + 1 > wp.w_lines.len() {
            wp.w_lines_valid = 0;
            wp.w_lines.clear();
            wp
                .w_lines
                .resize(wp.w_view_height as usize + 1, wline_T::default());
        }

        let mut was_resized = false;
        if want_allocation
            && (!has_allocation
                || (*grid_allocated).rows != total_rows
                || (*grid_allocated).cols != total_cols)
        {
            grid_alloc(
                &mut *grid_allocated,
                total_rows,
                total_cols,
                wp.w_grid_alloc.valid,
                false,
            );
            (*grid_allocated).valid = true;
            if wp.w_floating && wp.w_config.border {
                wp.w_redr_border = true;
            }
            was_resized = true;
        } else if !want_allocation && has_allocation {
            grid_free(&mut *grid_allocated);
            (*grid_allocated).valid = false;
            was_resized = true;
        } else if want_allocation && has_allocation && !wp.w_grid_alloc.valid {
            grid_invalidate(grid_allocated);
            (*grid_allocated).valid = true;
        }

        if want_allocation {
            (*grid).target = grid_allocated;
            (*grid).row_offset = wp.w_winrow_off;
            (*grid).col_offset = wp.w_wincol_off;
        } else {
            (*grid).target = &raw mut g_st().default_grid;
            (*grid).row_offset = wp.w_winrow + wp.w_winrow_off;
            (*grid).col_offset = wp.w_wincol + wp.w_wincol_off;
        }

        if (g_st().resizing_screen || was_resized) && want_allocation {
            ui_call_grid_resize(
                i64::from((*grid_allocated).handle),
                i64::from((*grid_allocated).cols),
                i64::from((*grid_allocated).rows),
            );
            ui_check_cursor_grid((*grid_allocated).handle);
        }
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
    row: c_int,
    line_count: c_int,
    end: c_int,
    col: c_int,
    width: c_int,
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
                let temp = grid.line_offset[j as usize] as c_uint as usize;
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
    row: c_int,
    line_count: c_int,
    end: c_int,
    col: c_int,
    width: c_int,
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
                let temp = grid.line_offset[j as usize] as c_uint as usize;
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
    mut col: c_int,
    winbl: c_int,
    hl_attr: Option<&crate::highlight::NSHlAttr>,
    win_hl_attr_normal: c_int,
    bt: BorderTextType,
    mut overflow: c_int,
) {
    unsafe {
        let hlf_idx = if bt == kBorderTextTitle {
            HLF_BTITLE
        } else {
            HLF_BFOOTER
        };
        let mut default_attr = if let Some(tbl) = hl_attr {
            tbl[hlf_idx].get()
        } else {
            crate::highlight::hl_attr(hlf_idx as c_int)
        };
        default_attr = hl_apply_winblend(winbl, default_attr);
        default_attr = hl_combine_attr(win_hl_attr_normal, default_attr);

        if overflow > 0 {
            grid_line_puts(1, c"<".as_ptr(), -1, default_attr);
            col += 1;
            overflow += 1;
        }

        let mut i: usize = 0;
        while i < vt.len() {
            let mut attr: c_int = -1;
            let mut text = next_virt_text_chunk(vt, &mut i, &mut attr);
            if text.is_null() {
                break;
            }
            if overflow > 0 {
                let cells = mb_string2cells(text) as c_int;
                if overflow >= cells {
                    overflow -= cells;
                    continue;
                }
                let mut p = text;
                while *p != NUL && overflow > 0 {
                    overflow -= utf_ptr2cells(p);
                    p = p.add(utfc_ptr2len(p) as usize);
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
}

fn get_bordertext_col(total_col: c_int, text_width: c_int, align: AlignTextPos) -> c_int {
    match align {
        kAlignLeft => 1,
        kAlignCenter => ((total_col - text_width) / 2 + 1).max(1),
        kAlignRight => (total_col - text_width + 1).max(1),
        _ => unreachable!(),
    }
}

/// Draw border on floating window grid (`grid_draw_border`).
pub fn grid_draw_border(
    grid: &mut ScreenGrid,
    config: &mut WinConfig,
    mut adj: *mut c_int,
    winbl: c_int,
    hl_attr: Option<&crate::highlight::NSHlAttr>,
    win_hl_attr_normal: c_int,
) {
    unsafe {
        let mut default_adj: [c_int; 4] = [1, 1, 1, 1];
        if adj.is_null() {
            adj = default_adj.as_mut_ptr();
        }
        let mut chars = [0 as schar_T; 8];
        let mut border_attrs = [0 as c_int; 8];

        for i in 0..8 {
            chars[i] = schar_from_str(config.border_chars[i].as_ptr());
            border_attrs[i] = hl_combine_attr(win_hl_attr_normal, config.border_attr[i]);
        }

        let adj0 = *adj;
        let adj1 = *adj.add(1);
        let adj2 = *adj.add(2);
        let adj3 = *adj.add(3);

        let irow = grid.rows - adj0 - adj2;
        let icol = grid.cols - adj1 - adj3;

        if adj0 != 0 {
            screengrid_line_start(&raw mut *grid, 0, 0);
            if adj3 != 0 {
                grid_line_put_schar(0, chars[0], border_attrs[0]);
            }

            for i in 0..icol {
                grid_line_put_schar(i + adj3, chars[1], border_attrs[1]);
            }

            if config.title {
                let title_col =
                    get_bordertext_col(icol, config.title_width, config.title_pos);
                grid_draw_bordertext(
                    &config.title_chunks,
                    title_col,
                    winbl,
                    hl_attr,
                    win_hl_attr_normal,
                    kBorderTextTitle,
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
                screengrid_line_start(&raw mut *grid, i + adj0, 0);
                grid_line_put_schar(0, chars[7], border_attrs[7]);
                grid_line_flush();
            }
            if adj1 != 0 {
                let ic = if i == 0 && adj0 == 0 && chars[2] != 0 {
                    2
                } else {
                    3
                };
                screengrid_line_start(&raw mut *grid, i + adj0, 0);
                grid_line_put_schar(icol + adj3, chars[ic], border_attrs[ic]);
                grid_line_flush();
            }
        }

        if adj2 != 0 {
            screengrid_line_start(&raw mut *grid, irow + adj0, 0);
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
                let footer_col =
                    get_bordertext_col(icol, config.footer_width, config.footer_pos);
                grid_draw_bordertext(
                    &config.footer_chunks,
                    footer_col,
                    winbl,
                    hl_attr,
                    win_hl_attr_normal,
                    kBorderTextFooter,
                    config.footer_width - icol,
                );
            }
            if adj1 != 0 {
                grid_line_put_schar(icol + adj3, chars[4], border_attrs[4]);
            }
            grid_line_flush();
        }
    }
}

fn linecopy(grid: &mut ScreenGrid, to: c_int, from: c_int, col: c_int, width: c_int) {
    {
        let grid = &mut *grid;
        let off_to = (grid.line_offset[to as usize] + col as usize) as c_uint as usize;
        let off_from = (grid.line_offset[from as usize] + col as usize) as c_uint as usize;
        let len = width as usize;

        grid.chars.copy_within(off_from..off_from + len, off_to);
        grid.attrs.copy_within(off_from..off_from + len, off_to);
        grid.vcols.copy_within(off_from..off_from + len, off_to);
    }
}

pub fn get_win_by_grid_handle(handle: handle_T) -> *mut win_T {
    unsafe {
        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            if (*wp).w_grid_alloc.handle == handle {
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

/// Put a unicode character in a screen cell (`schar_from_char`).
pub fn schar_from_char(mut c: c_int) -> schar_T {
    let mut sc: schar_T = 0;
    if c >= 0x20_0000 {
        c = 0xFFFD;
    }
    utf_char2bytes(c, (&raw mut sc).cast::<c_char>());
    sc
}

#[cfg(target_endian = "big")]
pub const braille_space: schar_T = 0xe2a0_8000;
#[cfg(not(target_endian = "big"))]
pub const braille_space: schar_T = 0x0080_a0e2;

#[inline]
pub fn schar_invisible(sc: schar_T) -> bool {
    sc == schar_from_ascii(b' ') || sc == braille_space
}

pub fn schar_from_cluster_impl(
    buf: *const c_char,
    mut len: usize,
    first_compose: bool,
) -> schar_T {
    unsafe {
        let maxlen = MAX_SCHAR_SIZE - 1 - usize::from(first_compose);
        if len > maxlen {
            len = maxlen;
            while (*buf.add(len).cast::<u8>() & 0xC0) != 0xC0 {
                len -= 1;
            }
        }

        if first_compose {
            let mut cbuf = [0 as c_char; MAX_SCHAR_SIZE];
            cbuf[0] = b' ' as c_char;
            ptr::copy_nonoverlapping(buf, cbuf.as_mut_ptr().add(1), len);
            schar_from_buf(cbuf.as_ptr(), len + 1)
        } else {
            schar_from_buf(buf, len)
        }
    }
}

#[inline]
pub fn schar_from_cluster(ci: StrCharInfo, cli: ClusterInfo) -> schar_T {
    assert!(ci.chr.value >= 0);
    let len = unsafe { cli.next.ptr.offset_from(ci.ptr) as usize };
    if len == 1 {
        return schar_from_ascii(ci.chr.value as u8);
    }
    if cli.first_compose || len > 4 {
        return schar_from_cluster_impl(ci.ptr, len, cli.first_compose);
    }
    let mut sc: schar_T = 0;
    unsafe {
        ptr::copy_nonoverlapping(ci.ptr, (&raw mut sc).cast::<c_char>(), len);
    }
    sc
}

#[cfg(test)]
pub fn unittest_schar_from_cluster(buf: *const c_char, firstc: &mut c_int) -> schar_T {
    let ci = crate::mbyte::utf_ptr2StrCharInfo(buf);
    let cli = crate::mbyte::utf_ClusterInfo(ci);
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
        unsafe {
            let sc_a = schar_from_ascii(b'A');
            assert_eq!(schar_get_ascii(sc_a), b'A' as c_char);

            let sc_u = schar_from_char(0x2603);
            let mut buf = [0 as c_char; 16];
            let len = schar_get(buf.as_mut_ptr(), sc_u);
            assert_eq!(len, 3);
            assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "☃");
        }
    }
}
