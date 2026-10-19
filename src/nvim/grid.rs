//! Low-level functions to manipulate individual character cells on the screen grid (`src/nvim/grid.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::decoration::{decor_check_invalid_glyphs, next_virt_text_chunk};
use crate::globals::*;
#[allow(unused_imports)]
use crate::highlight::{hl_apply_winblend, hl_combine_attr};
use crate::mbyte::utf8len_tab;
#[allow(unused_imports)]
use crate::mbyte::{
    mb_string2cells, ptr2cells, utf_ClusterInfo_impl, utf_char2bytes, utf_char2cells, utf_char2len,
    utf_cp_bounds, utf_ptr2CharInfo_impl, utf_ptr2cells, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
    utfc_ptrlen2schar,
};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::optionstr::check_chars_options;
#[allow(unused_imports)]
use crate::ui::{ui_call_grid_resize, ui_call_grid_scroll};
#[allow(unused_imports)]
use crate::ui::{ui_check_cursor_grid, ui_grid_cursor_goto, ui_has, ui_line};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use crate::api::deprecated::VirtText;
use crate::api::tabpage::{GridView, ScreenGrid, WinConfig, win_T};
use crate::arabic::arabic_shape;
use crate::cursor::{colnr_T, linenr_T};
use crate::map::mh_clear;
use crate::map_glyph_cache::{MHPutStatus, MapHash, NvimString, Set_glyph, mh_put_glyph};
use crate::memory::{xcalloc, xfree, xmalloc};
use crate::msgpack_rpc::packer::handle_T;
use crate::plines::{CharInfo, ClusterInfo, StrCharInfo};

pub type schar_T = u32;
pub type sattr_T = i32;
pub type sscratch_T = i32;
pub use crate::api::win_config::AlignTextPos;
pub use crate::api::win_config::BorderTextType;
pub use crate::api::win_config::UIExtension;

const NUL: c_char = 0;
const MAX_SCHAR_SIZE: usize = 32;
const DEFAULT_GRID_HANDLE: handle_T = 1;

const SLF_RIGHTLEFT: c_int = 1;
const SLF_WRAP: c_int = 2;
const SLF_INC_VCOL: c_int = 4;

const kOptRdbFlagInvalid: c_uint = 0x04;
const kOptRdbFlagNodelta: c_uint = 0x08;

const kUIMultigrid: UIExtension = 6;

const kAlignLeft: AlignTextPos = 0;
const kAlignCenter: AlignTextPos = 1;
const kAlignRight: AlignTextPos = 2;

const kBorderTextTitle: BorderTextType = 0;
const kBorderTextFooter: BorderTextType = 1;

const HLF_BTITLE: usize = 68;
const HLF_BFOOTER: usize = 69;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharBoundsOff {
    pub begin_off: i8,
    pub end_off: i8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct wline_T {
    pub wl_lnum: linenr_T,
    pub wl_size: u16,
    pub wl_valid: bool,
    pub wl_folded: bool,
    pub wl_foldend: linenr_T,
    pub wl_lastlnum: linenr_T,
}

static mut linebuf_size: usize = 0;

static mut glyph_cache: Set_glyph = Set_glyph {
    h: MapHash {
        n_buckets: 0,
        size: 0,
        n_occupied: 0,
        upper_bound: 0,
        n_keys: 0,
        keys_capacity: 0,
        hash: ptr::null_mut(),
    },
    keys: ptr::null_mut(),
};

static mut grid_line_grid: *mut ScreenGrid = ptr::null_mut();
static mut grid_line_row: c_int = -1;
static mut grid_line_coloff: c_int = 0;
static mut grid_line_maxcol: c_int = 0;
static mut grid_line_first: c_int = c_int::MAX;
static mut grid_line_last: c_int = 0;
static mut grid_line_clear_to: c_int = 0;
static mut grid_line_bg_attr: c_int = 0;
static mut grid_line_clear_attr: c_int = 0;
static mut grid_line_flags: c_int = 0;

static mut last_grid_handle: c_int = DEFAULT_GRID_HANDLE;

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
unsafe fn strnlen_4(p: *const u8) -> usize {
    unsafe {
        for i in 0..4 {
            if *p.add(i) == 0 {
                return i;
            }
        }
        4
    }
}

#[inline]
unsafe fn ascii2cells(b: c_int) -> c_int {
    if (b' ' as c_int..=b'~' as c_int).contains(&b) {
        1
    } else {
        unsafe { dy_escape_width }
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
            let mut len = c_int::from(utf8len_tab[first as usize]);
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

/// Determine if dedicated window grid should be used or the `default_grid` (`grid_adjust`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_adjust(
    grid: *mut GridView,
    row_off: *mut c_int,
    col_off: *mut c_int,
) -> *mut ScreenGrid {
    unsafe {
        *row_off += (*grid).row_offset;
        *col_off += (*grid).col_offset;
        (*grid).target
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_from_str(str_: *const c_char) -> schar_T {
    unsafe {
        if str_.is_null() {
            return 0;
        }
        schar_from_buf(str_, CStr::from_ptr(str_).to_bytes().len())
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_from_buf(buf: *const c_char, len: usize) -> schar_T {
    unsafe {
        assert!(len < MAX_SCHAR_SIZE);
        if len <= 4 {
            let mut sc: schar_T = 0;
            ptr::copy_nonoverlapping(buf.cast::<u8>(), (&raw mut sc).cast::<u8>(), len);
            sc
        } else {
            let str_ = NvimString {
                data: buf.cast_mut(),
                size: len,
            };
            let mut status = MHPutStatus::kMHExisting;
            let idx = mh_put_glyph(&raw mut glyph_cache, str_, &raw mut status);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_cache_clear_if_full() -> bool {
    unsafe {
        if glyph_cache.h.n_keys > (1 << 21) {
            schar_cache_clear();
            return true;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_cache_clear() {
    unsafe {
        decor_check_invalid_glyphs();
        mh_clear(&raw mut glyph_cache.h);

        if !check_chars_options().is_null() {
            std::process::abort();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn schar_high(sc: schar_T) -> bool {
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_get(mut buf_out: *mut c_char, sc: schar_T) -> usize {
    unsafe {
        let len = schar_get_adv(&raw mut buf_out, sc);
        *buf_out = NUL;
        len
    }
}

/// Advance `buf_out`. Do NOT set final NUL (`schar_get_adv`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_get_adv(buf_out: *mut *mut c_char, sc: schar_T) -> usize {
    unsafe {
        let len: usize;
        if schar_high(sc) {
            let idx = schar_idx(sc);
            assert!(idx < glyph_cache.h.n_keys);
            let key_ptr = glyph_cache.keys.add(idx as usize);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_len(sc: schar_T) -> usize {
    unsafe {
        if schar_high(sc) {
            let idx = schar_idx(sc);
            assert!(idx < glyph_cache.h.n_keys);
            CStr::from_ptr(glyph_cache.keys.add(idx as usize))
                .to_bytes()
                .len()
        } else {
            strnlen_4((&raw const sc).cast::<u8>())
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_cells(sc: schar_T) -> c_int {
    unsafe {
        #[cfg(target_endian = "big")]
        if (sc & 0x80FF_FFFF) == 0 {
            return 1;
        }
        #[cfg(not(target_endian = "big"))]
        if sc < 0x80 {
            return 1;
        }

        let mut sc_buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(sc_buf.as_mut_ptr(), sc);
        ptr2cells(sc_buf.as_ptr())
    }
}

#[inline]
unsafe fn schar_get_first_byte(sc: schar_T) -> c_char {
    unsafe {
        assert!(!(schar_high(sc) && schar_idx(sc) >= glyph_cache.h.n_keys));
        if schar_high(sc) {
            *glyph_cache.keys.add(schar_idx(sc) as usize)
        } else {
            *(&raw const sc).cast::<c_char>()
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_get_first_codepoint(sc: schar_T) -> c_int {
    unsafe {
        let mut sc_buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(sc_buf.as_mut_ptr(), sc);
        utf_ptr2char(sc_buf.as_ptr())
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_get_ascii(sc: schar_T) -> c_char {
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
unsafe fn schar_in_arabic_block(sc: schar_T) -> bool {
    unsafe {
        let first_byte = schar_get_first_byte(sc) as u8;
        (first_byte & 0xFE) == 0xD8
    }
}

#[inline]
unsafe fn schar_get_first_two_codepoints(sc: schar_T, c0: *mut c_int, c1: *mut c_int) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn line_do_arabic_shape(buf: *mut schar_T, cols: c_int) {
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
        schar_get_first_two_codepoints(*buf.add(i as usize), &raw mut c0, &raw mut c1);

        while i < cols {
            let mut c0next: c_int = 0;
            let mut c1next: c_int = 0;
            let next_sc = if i + 1 < cols {
                *buf.add((i + 1) as usize)
            } else {
                0
            };
            schar_get_first_two_codepoints(next_sc, &raw mut c0next, &raw mut c1next);

            if arabic_char(c0) {
                let mut c1new = c1;
                let c0new = arabic_shape(c0, &raw mut c1new, c0next, c1next, c0prev);

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

/// Clear a line in the grid starting at `off` until `width` characters are cleared (`grid_clear_line`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_clear_line(
    grid: *mut ScreenGrid,
    off: usize,
    width: c_int,
    valid: bool,
) {
    unsafe {
        for col in 0..width {
            *(*grid).chars.add(off + col as usize) = schar_from_ascii(b' ');
        }
        let fill: u8 = if valid { 0 } else { 0xFF };
        ptr::write_bytes(
            (*grid).attrs.add(off).cast::<u8>(),
            fill,
            width as usize * size_of::<sattr_T>(),
        );
        ptr::write_bytes(
            (*grid).vcols.add(off).cast::<u8>(),
            0xFF,
            width as usize * size_of::<colnr_T>(),
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_invalidate(grid: *mut ScreenGrid) {
    unsafe {
        ptr::write_bytes(
            (*grid).attrs.cast::<u8>(),
            0xFF,
            size_of::<sattr_T>() * (*grid).rows as usize * (*grid).cols as usize,
        );
    }
}

#[inline]
unsafe fn grid_invalid_row(grid: *mut ScreenGrid, row: c_int) -> bool {
    unsafe { *(*grid).attrs.add(*(*grid).line_offset.add(row as usize)) < 0 }
}

/// Get a single character directly from `grid.chars` (`grid_getchar`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_getchar(
    grid: *mut ScreenGrid,
    row: c_int,
    col: c_int,
    attrp: *mut c_int,
) -> schar_T {
    unsafe {
        if (*grid).chars.is_null() || row >= (*grid).rows || col >= (*grid).cols {
            return 0;
        }

        let off = *(*grid).line_offset.add(row as usize) + col as usize;
        if !attrp.is_null() {
            *attrp = *(*grid).attrs.add(off);
        }
        *(*grid).chars.add(off)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_start(view: *mut GridView, mut row: c_int) {
    unsafe {
        let mut col: c_int = 0;
        let grid = grid_adjust(view, &raw mut row, &raw mut col);
        screengrid_line_start(grid, row, col);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn screengrid_line_start(grid: *mut ScreenGrid, row: c_int, col: c_int) {
    unsafe {
        grid_line_maxcol = (*grid).cols;
        assert!(grid_line_grid.is_null());
        grid_line_row = row;
        grid_line_grid = grid;
        grid_line_coloff = col;
        grid_line_first = linebuf_size as c_int;
        grid_line_maxcol = grid_line_maxcol.min((*grid).cols - grid_line_coloff);
        grid_line_last = 0;
        grid_line_clear_to = 0;
        grid_line_bg_attr = 0;
        grid_line_clear_attr = 0;
        grid_line_flags = 0;

        assert!(grid_line_maxcol as usize <= linebuf_size);

        if full_screen && (rdb_flags & kOptRdbFlagInvalid) != 0 {
            assert!(!linebuf_char.is_null());
            ptr::write_bytes(
                linebuf_char.cast::<u8>(),
                0xFF,
                size_of::<schar_T>() * linebuf_size,
            );
            ptr::write_bytes(
                linebuf_attr.cast::<u8>(),
                0xFF,
                size_of::<sattr_T>() * linebuf_size,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_getchar(mut col: c_int, attr: *mut c_int) -> schar_T {
    unsafe {
        if col < grid_line_maxcol {
            col += grid_line_coloff;
            let off = *(*grid_line_grid).line_offset.add(grid_line_row as usize) + col as usize;
            if !attr.is_null() {
                *attr = *(*grid_line_grid).attrs.add(off);
            }
            *(*grid_line_grid).chars.add(off)
        } else {
            schar_from_ascii(b' ')
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_put_schar(col: c_int, schar: schar_T, attr: c_int) {
    unsafe {
        assert!(!grid_line_grid.is_null());
        if col >= grid_line_maxcol {
            return;
        }

        *linebuf_char.add(col as usize) = schar;
        *linebuf_attr.add(col as usize) = attr;

        grid_line_first = grid_line_first.min(col);
        grid_line_last = grid_line_last.max(col + 1);
        *linebuf_vcol.add(col as usize) = -1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_puts(
    mut col: c_int,
    text: *const c_char,
    textlen: c_int,
    attr: c_int,
) -> c_int {
    unsafe {
        assert!(!grid_line_grid.is_null());
        let mut len = if textlen >= 0 { textlen } else { c_int::MAX };

        let start_col = col;
        let max_col = grid_line_maxcol;
        let mut ci = utf_ptr2StrCharInfo_len(text, len);
        while col < max_col && len > 0 && *ci.ptr != NUL {
            let cli = utf_ClusterInfo_len(ci, &raw mut len);
            let mut mbyte_cells = cli.cells;
            let mut firstc: c_int = 0;
            let mut schar = utfc_ptrlen2schar(
                ci.ptr,
                cli.next.ptr.offset_from(ci.ptr) as c_int,
                &raw mut firstc,
            );
            if mbyte_cells > 2 || schar == 0 {
                mbyte_cells = 1;
                schar = schar_from_char(0xFFFD);
            }

            if col + mbyte_cells > max_col {
                schar = schar_from_ascii(b'>');
                mbyte_cells = 1;
            }

            if ptr::eq(ci.ptr, text)
                && col > grid_line_first
                && col < grid_line_last
                && *linebuf_char.add(col as usize) == 0
            {
                *linebuf_char.add((col - 1) as usize) = schar_from_ascii(b'>');
            }

            *linebuf_char.add(col as usize) = schar;
            *linebuf_attr.add(col as usize) = attr;
            *linebuf_vcol.add(col as usize) = -1;
            if mbyte_cells == 2 {
                *linebuf_char.add((col + 1) as usize) = 0;
                *linebuf_attr.add((col + 1) as usize) = attr;
                *linebuf_vcol.add((col + 1) as usize) = -1;
            }

            col += mbyte_cells;
            ci = cli.next;
        }

        if col > start_col {
            grid_line_first = grid_line_first.min(start_col);
            grid_line_last = grid_line_last.max(col);
        }

        col - start_col
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_fill(
    start_col: c_int,
    mut end_col: c_int,
    sc: schar_T,
    attr: c_int,
) -> c_int {
    unsafe {
        end_col = end_col.min(grid_line_maxcol);
        if start_col >= end_col {
            return end_col;
        }

        for col in start_col..end_col {
            *linebuf_char.add(col as usize) = sc;
            *linebuf_attr.add(col as usize) = attr;
            *linebuf_vcol.add(col as usize) = -1;
        }

        grid_line_first = grid_line_first.min(start_col);
        grid_line_last = grid_line_last.max(end_col);
        end_col
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_clear_end(
    start_col: c_int,
    end_col: c_int,
    bg_attr: c_int,
    clear_attr: c_int,
) {
    unsafe {
        if grid_line_first > start_col {
            grid_line_first = start_col;
            grid_line_last = start_col;
        }
        grid_line_clear_to = end_col;
        grid_line_bg_attr = bg_attr;
        grid_line_clear_attr = clear_attr;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_cursor_goto(col: c_int) {
    unsafe {
        ui_grid_cursor_goto((*grid_line_grid).handle, grid_line_row, col);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_mirror(width: c_int) {
    unsafe {
        grid_line_clear_to = grid_line_last.max(grid_line_clear_to);
        if grid_line_first >= grid_line_clear_to {
            return;
        }
        linebuf_mirror(
            &raw mut grid_line_first,
            &raw mut grid_line_last,
            &raw mut grid_line_clear_to,
            width,
        );
        grid_line_flags |= SLF_RIGHTLEFT;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn linebuf_mirror(
    firstp: *mut c_int,
    lastp: *mut c_int,
    clearp: *mut c_int,
    width: c_int,
) {
    unsafe {
        let first = *firstp;
        let last = *lastp;

        let n = (last - first) as usize;
        let mirror = width - 1;
        let scratch_char = linebuf_scratch.cast::<schar_T>();
        ptr::copy_nonoverlapping(
            linebuf_char.add(first as usize),
            scratch_char.add(first as usize),
            n,
        );
        let mut col = first;
        while col < last {
            let rev = mirror - col;
            if col + 1 < last && *scratch_char.add((col + 1) as usize) == 0 {
                *linebuf_char.add((rev - 1) as usize) = *scratch_char.add(col as usize);
                *linebuf_char.add(rev as usize) = 0;
                col += 1;
            } else {
                *linebuf_char.add(rev as usize) = *scratch_char.add(col as usize);
            }
            col += 1;
        }

        let scratch_attr = linebuf_scratch.cast::<sattr_T>();
        ptr::copy_nonoverlapping(
            linebuf_attr.add(first as usize),
            scratch_attr.add(first as usize),
            n,
        );
        for c in first..last {
            *linebuf_attr.add((mirror - c) as usize) = *scratch_attr.add(c as usize);
        }

        let scratch_vcol = linebuf_scratch.cast::<colnr_T>();
        ptr::copy_nonoverlapping(
            linebuf_vcol.add(first as usize),
            scratch_vcol.add(first as usize),
            n,
        );
        for c in first..last {
            *linebuf_vcol.add((mirror - c) as usize) = *scratch_vcol.add(c as usize);
        }

        *firstp = width - *clearp;
        *clearp = width - first;
        *lastp = width - last;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_flush() {
    unsafe {
        let grid = grid_line_grid;
        grid_line_grid = ptr::null_mut();
        grid_line_clear_to = grid_line_last.max(grid_line_clear_to);
        assert!(grid_line_clear_to <= grid_line_maxcol);
        if grid_line_first >= grid_line_clear_to {
            return;
        }

        grid_put_linebuf(
            grid,
            grid_line_row,
            grid_line_coloff,
            grid_line_first,
            grid_line_last,
            grid_line_clear_to,
            grid_line_bg_attr,
            grid_line_clear_attr,
            -1,
            grid_line_flags,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_line_flush_if_valid_row() {
    unsafe {
        if grid_line_row < 0 || grid_line_row >= (*grid_line_grid).rows {
            if (rdb_flags & kOptRdbFlagInvalid) != 0 {
                std::process::abort();
            } else {
                grid_line_grid = ptr::null_mut();
                return;
            }
        }
        grid_line_flush();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_clear(
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
            end_col = end_col.min(grid_line_maxcol);
            if grid_line_row >= (*grid_line_grid).rows || start_col >= end_col {
                grid_line_grid = ptr::null_mut();
                return;
            }
            grid_line_clear_end(start_col, end_col, attr, 0);
            grid_line_flush();
        }
    }
}

#[inline]
unsafe fn grid_char_needs_redraw(
    grid: *mut ScreenGrid,
    col: c_int,
    off_to: usize,
    cols: c_int,
) -> bool {
    unsafe {
        cols > 0
            && ((*linebuf_char.add(col as usize) != *(*grid).chars.add(off_to)
                || *linebuf_attr.add(col as usize) != *(*grid).attrs.add(off_to)
                || (cols > 1
                    && *linebuf_char.add((col + 1) as usize) == 0
                    && *linebuf_char.add((col + 1) as usize) != *(*grid).chars.add(off_to + 1)))
                || (rdb_flags & kOptRdbFlagNodelta) != 0)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_put_linebuf(
    grid: *mut ScreenGrid,
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
    unsafe {
        let mut clear_next = false;
        assert!(0 <= row && row < (*grid).rows);
        if endcol > (*grid).cols {
            endcol = (*grid).cols;
        }

        if (*grid).chars.is_null() || row >= (*grid).rows || coloff >= (*grid).cols {
            return;
        }

        let invalid_row =
            !ptr::eq(grid, &raw const default_grid) && grid_invalid_row(grid, row) && col == 0;
        let off_to = *(*grid).line_offset.add(row as usize) + coloff as usize;
        let max_off_to = *(*grid).line_offset.add(row as usize) + (*grid).cols as usize;

        if col > 0 && *(*grid).chars.add(off_to + col as usize) == 0 {
            *linebuf_char.add((col - 1) as usize) = schar_from_ascii(b'>');
            *linebuf_attr.add((col - 1) as usize) = *(*grid).attrs.add(off_to + col as usize - 1);
            col -= 1;
        }

        let mut clear_start = endcol;
        if (flags & SLF_RIGHTLEFT) != 0 {
            clear_start = col;
            col = endcol;
            endcol = clear_width;
            clear_width = col;
        }

        if p_arshape != 0 && p_tbidi == 0 && endcol > col {
            line_do_arabic_shape(linebuf_char.add(col as usize), endcol - col);
        }

        if bg_attr != 0 {
            for c in col..endcol {
                *linebuf_attr.add(c as usize) =
                    hl_combine_attr(bg_attr, *linebuf_attr.add(c as usize));
            }
        }

        let mut redraw_next =
            grid_char_needs_redraw(grid, col, off_to + col as usize, endcol - col);

        let mut start_dirty: c_int = -1;
        let mut end_dirty: c_int = 0;

        while col < endcol {
            let mut char_cells: c_int = 1;
            if col + 1 < endcol && *linebuf_char.add((col + 1) as usize) == 0 {
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
                    && *(*grid).chars.add(off + char_cells as usize) == 0
                {
                    clear_next = true;
                }

                *(*grid).chars.add(off) = *linebuf_char.add(col as usize);
                if char_cells == 2 {
                    *(*grid).chars.add(off + 1) = *linebuf_char.add((col + 1) as usize);
                }

                *(*grid).attrs.add(off) = *linebuf_attr.add(col as usize);
                if char_cells == 2 {
                    *(*grid).attrs.add(off + 1) = *linebuf_attr.add(col as usize);
                }
            }

            *(*grid).vcols.add(off) = *linebuf_vcol.add(col as usize);
            if char_cells == 2 {
                *(*grid).vcols.add(off + 1) = *linebuf_vcol.add((col + 1) as usize);
            }

            col += char_cells;
        }

        if clear_next {
            *(*grid).chars.add(off_to + col as usize) = schar_from_ascii(b' ');
            end_dirty += 1;
        }

        if off_to + (clear_width as usize) < max_off_to
            && *(*grid).chars.add(off_to + clear_width as usize) == 0
        {
            clear_width += 1;
        }

        let mut clear_dirty_start: c_int = -1;
        let mut clear_end: c_int = -1;
        if (flags & SLF_RIGHTLEFT) != 0 {
            let mut c = clear_width - 1;
            while c >= clear_start {
                let off = off_to + c as usize;
                *(*grid).vcols.add(off) = if (flags & SLF_INC_VCOL) != 0 {
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
            if *(*grid).chars.add(off) != schar_from_ascii(b' ')
                || *(*grid).attrs.add(off) != clear_attr
                || (rdb_flags & kOptRdbFlagNodelta) != 0
            {
                *(*grid).chars.add(off) = schar_from_ascii(b' ');
                *(*grid).attrs.add(off) = clear_attr;
                if clear_dirty_start == -1 {
                    clear_dirty_start = c;
                }
                clear_end = c + 1;
            }
            if (flags & SLF_RIGHTLEFT) == 0 {
                *(*grid).vcols.add(off) = if (flags & SLF_INC_VCOL) != 0 {
                    last_vcol += 1;
                    last_vcol
                } else {
                    last_vcol
                };
            }
        }

        if (flags & SLF_RIGHTLEFT) != 0 && start_dirty != -1 && clear_dirty_start != -1 {
            if (*grid).throttled || clear_dirty_start >= start_dirty - 5 {
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
            if !(*grid).throttled {
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
            } else if !(*grid).dirty_col.is_null()
                && clear_end > *(*grid).dirty_col.add(row as usize)
            {
                *(*grid).dirty_col.add(row as usize) = clear_end;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_alloc(
    grid: *mut ScreenGrid,
    rows: c_int,
    columns: c_int,
    copy: bool,
    valid: bool,
) {
    unsafe {
        let mut ngrid = *grid;
        assert!(rows >= 0 && columns >= 0);
        let ncells = rows as usize * columns as usize;
        ngrid.chars = xmalloc(ncells * size_of::<schar_T>()).cast();
        ngrid.attrs = xmalloc(ncells * size_of::<sattr_T>()).cast();
        ngrid.vcols = xmalloc(ncells * size_of::<colnr_T>()).cast();
        ptr::write_bytes(
            ngrid.vcols.cast::<u8>(),
            0xFF,
            ncells * size_of::<colnr_T>(),
        );
        ngrid.line_offset = xmalloc(rows as usize * size_of::<usize>()).cast();

        ngrid.rows = rows;
        ngrid.cols = columns;

        for new_row in 0..ngrid.rows {
            let row_off = new_row as usize * ngrid.cols as usize;
            *ngrid.line_offset.add(new_row as usize) = row_off;

            grid_clear_line(&raw mut ngrid, row_off, columns, valid);

            if copy && new_row < (*grid).rows && !(*grid).chars.is_null() {
                let len = (*grid).cols.min(ngrid.cols) as usize;
                let old_off = *(*grid).line_offset.add(new_row as usize);
                ptr::copy((*grid).chars.add(old_off), ngrid.chars.add(row_off), len);
                ptr::copy((*grid).attrs.add(old_off), ngrid.attrs.add(row_off), len);
                ptr::copy((*grid).vcols.add(old_off), ngrid.vcols.add(row_off), len);
            }
        }
        grid_free(grid);
        *grid = ngrid;

        if linebuf_size < columns as usize {
            xfree(linebuf_char.cast());
            xfree(linebuf_attr.cast());
            xfree(linebuf_vcol.cast());
            xfree(linebuf_scratch.cast());
            linebuf_char = xmalloc(columns as usize * size_of::<schar_T>()).cast();
            linebuf_attr = xmalloc(columns as usize * size_of::<sattr_T>()).cast();
            linebuf_vcol = xmalloc(columns as usize * size_of::<colnr_T>()).cast();
            linebuf_scratch = xmalloc(columns as usize * size_of::<sscratch_T>()).cast();
            linebuf_size = columns as usize;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_free(grid: *mut ScreenGrid) {
    unsafe {
        xfree((*grid).chars.cast());
        xfree((*grid).attrs.cast());
        xfree((*grid).vcols.cast());
        xfree((*grid).line_offset.cast());

        (*grid).chars = ptr::null_mut();
        (*grid).attrs = ptr::null_mut();
        (*grid).vcols = ptr::null_mut();
        (*grid).line_offset = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_free_all_mem() {
    unsafe {
        grid_free(&raw mut default_grid);
        grid_free(&raw mut msg_grid);
        xfree(msg_grid.dirty_col.cast());
        msg_grid.dirty_col = ptr::null_mut();
        xfree(linebuf_char.cast());
        xfree(linebuf_attr.cast());
        xfree(linebuf_vcol.cast());
        xfree(linebuf_scratch.cast());
        xfree(glyph_cache.keys.cast());
        xfree(glyph_cache.h.hash.cast());
        glyph_cache = Set_glyph {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: ptr::null_mut(),
            },
            keys: ptr::null_mut(),
        };
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_grid_alloc(wp: *mut win_T) {
    unsafe {
        let grid = &raw mut (*wp).w_grid;
        let grid_allocated = &raw mut (*wp).w_grid_alloc;

        let total_rows = (*wp).w_height_outer;
        let total_cols = (*wp).w_width_outer;

        let want_allocation = ui_has(kUIMultigrid) || (*wp).w_floating;
        let has_allocation = !(*grid_allocated).chars.is_null();

        if (*wp).w_view_height > (*wp).w_lines_size {
            (*wp).w_lines_valid = 0;
            xfree((*wp).w_lines.cast());
            (*wp).w_lines = xcalloc((*wp).w_view_height as usize + 1, size_of::<wline_T>()).cast();
            (*wp).w_lines_size = (*wp).w_view_height;
        }

        let mut was_resized = false;
        if want_allocation
            && (!has_allocation
                || (*grid_allocated).rows != total_rows
                || (*grid_allocated).cols != total_cols)
        {
            grid_alloc(
                grid_allocated,
                total_rows,
                total_cols,
                (*wp).w_grid_alloc.valid,
                false,
            );
            (*grid_allocated).valid = true;
            if (*wp).w_floating && (*wp).w_config.border {
                (*wp).w_redr_border = true;
            }
            was_resized = true;
        } else if !want_allocation && has_allocation {
            grid_free(grid_allocated);
            (*grid_allocated).valid = false;
            was_resized = true;
        } else if want_allocation && has_allocation && !(*wp).w_grid_alloc.valid {
            grid_invalidate(grid_allocated);
            (*grid_allocated).valid = true;
        }

        if want_allocation {
            (*grid).target = grid_allocated;
            (*grid).row_offset = (*wp).w_winrow_off;
            (*grid).col_offset = (*wp).w_wincol_off;
        } else {
            (*grid).target = &raw mut default_grid;
            (*grid).row_offset = (*wp).w_winrow + (*wp).w_winrow_off;
            (*grid).col_offset = (*wp).w_wincol + (*wp).w_wincol_off;
        }

        if (resizing_screen || was_resized) && want_allocation {
            ui_call_grid_resize(
                i64::from((*grid_allocated).handle),
                i64::from((*grid_allocated).cols),
                i64::from((*grid_allocated).rows),
            );
            ui_check_cursor_grid((*grid_allocated).handle);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_assign_handle(grid: *mut ScreenGrid) {
    unsafe {
        if (*grid).handle == 0 {
            last_grid_handle += 1;
            (*grid).handle = last_grid_handle;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_ins_lines(
    grid: *mut ScreenGrid,
    row: c_int,
    line_count: c_int,
    end: c_int,
    col: c_int,
    width: c_int,
) {
    unsafe {
        if line_count <= 0 {
            return;
        }

        for i in 0..line_count {
            if width != (*grid).cols {
                let mut j = end - 1 - i;
                while {
                    j -= line_count;
                    j >= row
                } {
                    linecopy(grid, j + line_count, j, col, width);
                }
                j += line_count;
                grid_clear_line(
                    grid,
                    *(*grid).line_offset.add(j as usize) + col as usize,
                    width,
                    false,
                );
            } else {
                let mut j = end - 1 - i;
                let temp = *(*grid).line_offset.add(j as usize) as c_uint as usize;
                while {
                    j -= line_count;
                    j >= row
                } {
                    *(*grid).line_offset.add((j + line_count) as usize) =
                        *(*grid).line_offset.add(j as usize);
                }
                *(*grid).line_offset.add((j + line_count) as usize) = temp;
                grid_clear_line(grid, temp, (*grid).cols, false);
            }
        }

        if !(*grid).throttled {
            ui_call_grid_scroll(
                i64::from((*grid).handle),
                i64::from(row),
                i64::from(end),
                i64::from(col),
                i64::from(col + width),
                i64::from(-line_count),
                0,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_del_lines(
    grid: *mut ScreenGrid,
    row: c_int,
    line_count: c_int,
    end: c_int,
    col: c_int,
    width: c_int,
) {
    unsafe {
        if line_count <= 0 {
            return;
        }

        for i in 0..line_count {
            if width != (*grid).cols {
                let mut j = row + i;
                while {
                    j += line_count;
                    j < end
                } {
                    linecopy(grid, j - line_count, j, col, width);
                }
                j -= line_count;
                grid_clear_line(
                    grid,
                    *(*grid).line_offset.add(j as usize) + col as usize,
                    width,
                    false,
                );
            } else {
                let mut j = row + i;
                let temp = *(*grid).line_offset.add(j as usize) as c_uint as usize;
                while {
                    j += line_count;
                    j < end
                } {
                    *(*grid).line_offset.add((j - line_count) as usize) =
                        *(*grid).line_offset.add(j as usize);
                }
                *(*grid).line_offset.add((j - line_count) as usize) = temp;
                grid_clear_line(grid, temp, (*grid).cols, false);
            }
        }

        if !(*grid).throttled {
            ui_call_grid_scroll(
                i64::from((*grid).handle),
                i64::from(row),
                i64::from(end),
                i64::from(col),
                i64::from(col + width),
                i64::from(line_count),
                0,
            );
        }
    }
}

unsafe fn grid_draw_bordertext(
    vt: VirtText,
    mut col: c_int,
    winbl: c_int,
    hl_attr: *const c_int,
    win_hl_attr_normal: c_int,
    bt: BorderTextType,
    mut overflow: c_int,
) {
    unsafe {
        let mut default_attr = *hl_attr.add(if bt == kBorderTextTitle {
            HLF_BTITLE
        } else {
            HLF_BFOOTER
        });
        default_attr = hl_apply_winblend(winbl, default_attr);
        default_attr = hl_combine_attr(win_hl_attr_normal, default_attr);

        if overflow > 0 {
            grid_line_puts(1, c"<".as_ptr(), -1, default_attr);
            col += 1;
            overflow += 1;
        }

        let mut i: usize = 0;
        while i < vt.size {
            let mut attr: c_int = -1;
            let mut text = next_virt_text_chunk(vt, &raw mut i, &raw mut attr);
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grid_draw_border(
    grid: *mut ScreenGrid,
    config: *mut WinConfig,
    mut adj: *mut c_int,
    winbl: c_int,
    mut hl_attr: *mut c_int,
    win_hl_attr_normal: c_int,
) {
    unsafe {
        let mut default_adj: [c_int; 4] = [1, 1, 1, 1];
        if adj.is_null() {
            adj = default_adj.as_mut_ptr();
        }
        let mut chars = [0 as schar_T; 8];
        let mut border_attrs = [0 as c_int; 8];
        if hl_attr.is_null() {
            hl_attr = hl_attr_active;
        }

        for i in 0..8 {
            chars[i] = schar_from_str((*config).border_chars[i].as_ptr());
            border_attrs[i] = hl_combine_attr(win_hl_attr_normal, (*config).border_attr[i]);
        }

        let adj0 = *adj;
        let adj1 = *adj.add(1);
        let adj2 = *adj.add(2);
        let adj3 = *adj.add(3);

        let irow = (*grid).rows - adj0 - adj2;
        let icol = (*grid).cols - adj1 - adj3;

        if adj0 != 0 {
            screengrid_line_start(grid, 0, 0);
            if adj3 != 0 {
                grid_line_put_schar(0, chars[0], border_attrs[0]);
            }

            for i in 0..icol {
                grid_line_put_schar(i + adj3, chars[1], border_attrs[1]);
            }

            if (*config).title {
                let title_col =
                    get_bordertext_col(icol, (*config).title_width, (*config).title_pos);
                grid_draw_bordertext(
                    (*config).title_chunks,
                    title_col,
                    winbl,
                    hl_attr,
                    win_hl_attr_normal,
                    kBorderTextTitle,
                    (*config).title_width - icol,
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

            if (*config).footer {
                let footer_col =
                    get_bordertext_col(icol, (*config).footer_width, (*config).footer_pos);
                grid_draw_bordertext(
                    (*config).footer_chunks,
                    footer_col,
                    winbl,
                    hl_attr,
                    win_hl_attr_normal,
                    kBorderTextFooter,
                    (*config).footer_width - icol,
                );
            }
            if adj1 != 0 {
                grid_line_put_schar(icol + adj3, chars[4], border_attrs[4]);
            }
            grid_line_flush();
        }
    }
}

unsafe fn linecopy(grid: *mut ScreenGrid, to: c_int, from: c_int, col: c_int, width: c_int) {
    unsafe {
        let off_to = (*(*grid).line_offset.add(to as usize) + col as usize) as c_uint as usize;
        let off_from = (*(*grid).line_offset.add(from as usize) + col as usize) as c_uint as usize;

        ptr::copy(
            (*grid).chars.add(off_from),
            (*grid).chars.add(off_to),
            width as usize,
        );
        ptr::copy(
            (*grid).attrs.add(off_from),
            (*grid).attrs.add(off_to),
            width as usize,
        );
        ptr::copy(
            (*grid).vcols.add(off_from),
            (*grid).vcols.add(off_to),
            width as usize,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_win_by_grid_handle(handle: handle_T) -> *mut win_T {
    unsafe {
        let mut wp = firstwin;
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn schar_from_char(mut c: c_int) -> schar_T {
    unsafe {
        let mut sc: schar_T = 0;
        if c >= 0x20_0000 {
            c = 0xFFFD;
        }
        utf_char2bytes(c, (&raw mut sc).cast::<c_char>());
        sc
    }
}
