//! Low-level functions to manipulate individual character cells on the screen grid (`src/nvim/grid.c`).
#![forbid(unsafe_code)]

use crate::arabic::arabic_shape;
use crate::cursor::{Colnr, Linenr};
use crate::decoration::{VirtTextChunk, decor_check_invalid_glyphs, next_virt_text_chunk};
use crate::globals::*;
use crate::highlight::{hl_apply_winblend, hl_combine_attr};
use crate::mbyte::{
    mb_string2cells, utf_char2bytes, utf_char2len, utf_cluster_info_len, utf_cp_bounds,
    utf_ptr2_str_char_info_len, utf_ptr2cells, utf_slice2char, utf_slice2len,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridTarget {
    #[default]
    None,
    Default,
    Msg,
    Pum,
    Win(ObjHandle),
}

impl GridTarget {
    #[inline]
    pub const fn is_null(self) -> bool {
        matches!(self, Self::None)
    }

    #[inline]
    pub fn opt(self) -> Option<&'static ScreenGrid> {
        match self {
            Self::None => None,
            Self::Default => Some(&g_st().borrow().default_grid),
            Self::Msg => Some(&g_st().borrow().msg_grid),
            Self::Pum => Some(&g_st().borrow().pum_grid),
            Self::Win(h) => g_st()
                .window_handles
                .get(&h)
                .copied()
                .and_then(WinPtr::opt)
                .or_else(|| {
                    for_all_windows_ptr()
                        .find(|wp| wp.handle == h)
                        .map(|wp| &*wp)
                })
                .map(|wp| &wp.w_grid_alloc),
        }
    }

    #[inline]
    pub fn opt_mut(self) -> Option<&'static mut ScreenGrid> {
        match self {
            Self::None => None,
            Self::Default => Some(&mut g_st().borrow_mut().default_grid),
            Self::Msg => Some(&mut g_st().borrow_mut().msg_grid),
            Self::Pum => Some(&mut g_st().borrow_mut().pum_grid),
            Self::Win(h) => g_st()
                .window_handles
                .get(&h)
                .copied()
                .and_then(WinPtr::opt_mut)
                .or_else(|| for_all_windows_ptr().find(|wp| wp.handle == h))
                .map(|wp| &mut wp.w_grid_alloc),
        }
    }

    #[inline]
    pub fn borrow(self) -> &'static ScreenGrid {
        self.opt().expect("null GridTarget")
    }

    #[inline]
    pub fn borrow_mut(self) -> &'static mut ScreenGrid {
        self.opt_mut().expect("null GridTarget")
    }
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct ScreenGrid {
    pub handle: std::cell::Cell<ObjHandle>,
    pub target_kind: std::cell::Cell<GridTarget>,
    pub chars: std::cell::RefCell<Vec<u32>>,
    pub attrs: std::cell::RefCell<Vec<i32>>,
    pub vcols: std::cell::RefCell<Vec<i32>>,
    pub line_offset: std::cell::RefCell<Vec<usize>>,
    pub dirty_col: std::cell::RefCell<Vec<i32>>,
    pub rows: std::cell::Cell<i32>,
    pub cols: std::cell::Cell<i32>,
    pub valid: std::cell::Cell<bool>,
    pub throttled: std::cell::Cell<bool>,
    pub blending: std::cell::Cell<bool>,
    pub mouse_enabled: std::cell::Cell<bool>,
    pub zindex: std::cell::Cell<i32>,
    pub comp_row: std::cell::Cell<i32>,
    pub comp_col: std::cell::Cell<i32>,
    pub comp_width: std::cell::Cell<i32>,
    pub comp_height: std::cell::Cell<i32>,
    pub comp_index: std::cell::Cell<usize>,
    pub comp_disabled: std::cell::Cell<bool>,
    pub pending_comp_index_update: std::cell::Cell<bool>,
}

impl ScreenGrid {
    pub const fn with_target(target_kind: GridTarget) -> Self {
        Self {
            handle: std::cell::Cell::new(0),
            target_kind: std::cell::Cell::new(target_kind),
            chars: std::cell::RefCell::new(Vec::new()),
            attrs: std::cell::RefCell::new(Vec::new()),
            vcols: std::cell::RefCell::new(Vec::new()),
            line_offset: std::cell::RefCell::new(Vec::new()),
            dirty_col: std::cell::RefCell::new(Vec::new()),
            rows: std::cell::Cell::new(0),
            cols: std::cell::Cell::new(0),
            valid: std::cell::Cell::new(false),
            throttled: std::cell::Cell::new(false),
            blending: std::cell::Cell::new(false),
            mouse_enabled: std::cell::Cell::new(true),
            zindex: std::cell::Cell::new(0),
            comp_row: std::cell::Cell::new(0),
            comp_col: std::cell::Cell::new(0),
            comp_width: std::cell::Cell::new(0),
            comp_height: std::cell::Cell::new(0),
            comp_index: std::cell::Cell::new(0),
            comp_disabled: std::cell::Cell::new(false),
            pending_comp_index_update: std::cell::Cell::new(true),
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct GridView {
    pub target: std::cell::Cell<GridTarget>,
    pub row_offset: std::cell::Cell<i32>,
    pub col_offset: std::cell::Cell<i32>,
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
    linebuf_scratch: std::cell::RefCell<Vec<Sscratch>>,
    glyph_cache_keys: std::cell::RefCell<Vec<u8>>,
    glyph_cache_offsets: std::cell::RefCell<std::collections::BTreeMap<Vec<u8>, u32>>,
    grid_line_grid: std::cell::Cell<GridTarget>,
    grid_line_row: std::cell::Cell<i32>,
    grid_line_coloff: std::cell::Cell<i32>,
    grid_line_maxcol: std::cell::Cell<i32>,
    grid_line_first: std::cell::Cell<i32>,
    grid_line_last: std::cell::Cell<i32>,
    grid_line_clear_to: std::cell::Cell<i32>,
    grid_line_bg_attr: std::cell::Cell<i32>,
    grid_line_clear_attr: std::cell::Cell<i32>,
    grid_line_flags: std::cell::Cell<i32>,
    last_grid_handle: std::cell::Cell<i32>,
}

impl GridModuleState {
    const fn new() -> Self {
        Self {
            linebuf_scratch: std::cell::RefCell::new(Vec::new()),
            glyph_cache_keys: std::cell::RefCell::new(Vec::new()),
            glyph_cache_offsets: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            grid_line_grid: std::cell::Cell::new(GridTarget::None),
            grid_line_row: std::cell::Cell::new(-1),
            grid_line_coloff: std::cell::Cell::new(0),
            grid_line_maxcol: std::cell::Cell::new(0),
            grid_line_first: std::cell::Cell::new(i32::MAX),
            grid_line_last: std::cell::Cell::new(0),
            grid_line_clear_to: std::cell::Cell::new(0),
            grid_line_bg_attr: std::cell::Cell::new(0),
            grid_line_clear_attr: std::cell::Cell::new(0),
            grid_line_flags: std::cell::Cell::new(0),
            last_grid_handle: std::cell::Cell::new(DEFAULT_GRID_HANDLE),
        }
    }
}

std::thread_local! {
    static GRID_STATE: GridModuleState = const { GridModuleState::new() };
}

#[inline]
fn grid_st<R>(f: impl FnOnce(&GridModuleState) -> R) -> R {
    GRID_STATE.with(f)
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
fn strnlen_4(bytes: &[u8; 4]) -> usize {
    bytes.iter().position(|&b| b == 0).unwrap_or(4)
}

/// Determine if dedicated window grid should be used or the `default_grid` (`grid_adjust`).
pub fn grid_adjust_opt(
    grid: &mut GridView,
    row_off: &mut i32,
    col_off: &mut i32,
) -> Option<&'static mut ScreenGrid> {
    *row_off += grid.row_offset.get();
    *col_off += grid.col_offset.get();
    grid.target.get().opt_mut()
}

/// Determine if dedicated window grid should be used or the `default_grid` (`grid_adjust`).
pub fn grid_adjust(
    grid: &mut GridView,
    row_off: &mut i32,
    col_off: &mut i32,
) -> &'static mut ScreenGrid {
    grid_adjust_opt(grid, row_off, col_off).unwrap()
}

pub fn schar_from_str(str_: Option<&std::ffi::CStr>) -> Schar {
    let Some(str_) = str_ else {
        return 0;
    };
    schar_from_bytes(str_.to_bytes())
}

pub fn schar_from_bytes(bytes: &[u8]) -> Schar {
    let len = bytes.len();
    assert!(len < MAX_SCHAR_SIZE);
    if len <= 4 {
        let mut buf = [0u8; 4];
        buf[..len].copy_from_slice(bytes);
        u32::from_ne_bytes(buf)
    } else {
        let idx = grid_st(|st| {
            if let Some(&existing_idx) = st.glyph_cache_offsets.borrow().get(bytes) {
                existing_idx
            } else {
                let mut keys = st.glyph_cache_keys.borrow_mut();
                let new_idx = keys.len() as u32;
                keys.extend_from_slice(bytes);
                keys.push(0);
                st.glyph_cache_offsets
                    .borrow_mut()
                    .insert(bytes.to_vec(), new_idx);
                new_idx
            }
        });
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

pub fn schar_from_buf(buf: &[u8]) -> Schar {
    schar_from_bytes(buf)
}

pub fn schar_cache_clear_if_full() -> bool {
    if grid_st(|st| st.glyph_cache_keys.borrow().len()) > (1 << 21) {
        schar_cache_clear();
        return true;
    }
    false
}

pub fn schar_cache_clear() {
    decor_check_invalid_glyphs();
    grid_st(|st| {
        st.glyph_cache_keys.borrow_mut().clear();
        st.glyph_cache_offsets.borrow_mut().clear();
    });

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
pub fn schar_get(buf_out: &mut [u8], sc: Schar) -> usize {
    let len = schar_get_adv(buf_out, sc);
    buf_out[len] = 0;
    len
}

pub fn schar_get_bytes(buf_out: &mut [u8], sc: Schar) -> usize {
    schar_get(buf_out, sc)
}

/// Write `sc` into `buf_out`. Do NOT set final NUL (`schar_get_adv`).
pub fn schar_get_adv(buf_out: &mut [u8], sc: Schar) -> usize {
    let len: usize;
    if schar_high(sc) {
        let idx = schar_idx(sc) as usize;
        len = grid_st(|st| {
            let keys = st.glyph_cache_keys.borrow();
            assert!(idx < keys.len());
            let key_bytes = crate::memory::bytes_until_nul(&keys[idx..]);
            let l = key_bytes.len();
            buf_out[..l].copy_from_slice(key_bytes);
            l
        });
    } else {
        let sc_bytes = sc.to_ne_bytes();
        len = strnlen_4(&sc_bytes);
        buf_out[..len].copy_from_slice(&sc_bytes[..len]);
    }
    len
}

pub fn schar_cells(sc: Schar) -> i32 {
    let mut buf = [0u8; MAX_SCHAR_SIZE];
    schar_get(&mut buf, sc);
    crate::mbyte::utf_ptr2cells(&buf)
}

pub fn schar_len(sc: Schar) -> usize {
    if schar_high(sc) {
        let idx = schar_idx(sc) as usize;
        grid_st(|st| {
            let keys = st.glyph_cache_keys.borrow();
            assert!(idx < keys.len());
            crate::memory::bytes_until_nul(&keys[idx..]).len()
        })
    } else {
        strnlen_4(&sc.to_ne_bytes())
    }
}

#[inline]
fn schar_get_first_byte(sc: Schar) -> u8 {
    if schar_high(sc) {
        let idx = schar_idx(sc) as usize;
        grid_st(|st| {
            let keys = st.glyph_cache_keys.borrow();
            assert!(idx < keys.len());
            keys[idx]
        })
    } else {
        sc.to_ne_bytes()[0]
    }
}

pub fn schar_get_first_codepoint(sc: Schar) -> i32 {
    let mut sc_buf = [0u8; MAX_SCHAR_SIZE];
    schar_get_bytes(&mut sc_buf, sc);
    utf_slice2char(&sc_buf)
}

pub fn schar_get_ascii(sc: Schar) -> u8 {
    #[cfg(target_endian = "big")]
    if (sc & 0x80FF_FFFF) == 0 {
        *(&raw const sc).cast::<u8>()
    } else {
        0
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

pub fn line_do_arabic_shape(buf: &mut [Schar]) {
    let cols = buf.len();
    let mut i: usize = 0;
    while i < cols {
        if schar_in_arabic_block(buf[i]) {
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
    schar_get_first_two_codepoints(buf[i], &mut c0, &mut c1);

    while i < cols {
        let mut c0next: i32 = 0;
        let mut c1next: i32 = 0;
        let next_sc = if i + 1 < cols { buf[i + 1] } else { 0 };
        schar_get_first_two_codepoints(next_sc, &mut c0next, &mut c1next);

        if arabic_char(c0) {
            let mut c1new = c1;
            let c0new = arabic_shape(c0, &mut c1new, c0next, c1next, c0prev);

            if !(c0new == c0 && c1new == c1) {
                let mut scbuf = [0u8; MAX_SCHAR_SIZE];
                schar_get(&mut scbuf, buf[i]);

                let mut scbuf_new = [0u8; MAX_SCHAR_SIZE];
                let mut len = crate::mbyte::utf_char2slice(c0new, &mut scbuf_new);
                if c1new != 0 {
                    len += crate::mbyte::utf_char2slice(c1new, &mut scbuf_new[len..]);
                }

                let off = (utf_char2len(c0) + if c1 != 0 { utf_char2len(c1) } else { 0 }) as usize;
                let rest_slice = &scbuf[off..];
                let mut rest = rest_slice
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(rest_slice.len());
                if rest + len + 1 > MAX_SCHAR_SIZE {
                    let bounds = utf_cp_bounds(&rest_slice[..rest], rest - 1);
                    rest = (rest as isize - (isize::from(bounds.begin_off) + 1)) as usize;
                }
                scbuf_new[len..len + rest].copy_from_slice(&rest_slice[..rest]);

                buf[i] = schar_from_buf(&scbuf_new[..len + rest]);
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
    !grid.chars.borrow().is_empty()
}

/// Clear a line in the grid starting at `off` until `width` characters are cleared (`grid_clear_line`).
pub fn grid_clear_line(grid: &ScreenGrid, off: usize, width: i32, valid: bool) {
    let len = width as usize;
    grid.chars.borrow_mut()[off..off + len].fill(schar_from_ascii(b' '));
    let fill: Sattr = if valid { 0 } else { -1 };
    grid.attrs.borrow_mut()[off..off + len].fill(fill);
    grid.vcols.borrow_mut()[off..off + len].fill(-1);
}

pub fn grid_invalidate(grid: &ScreenGrid) {
    let len = grid.rows.get() as usize * grid.cols.get() as usize;
    grid.attrs.borrow_mut()[..len].fill(-1);
}

#[inline]
fn grid_invalid_row(grid: &ScreenGrid, row: i32) -> bool {
    let off = grid.line_offset.borrow()[row as usize];
    grid.attrs.borrow()[off] < 0
}

/// Get a single character directly from `grid.chars` (`grid_getchar`).
pub fn grid_getchar(grid: &ScreenGrid, row: i32, col: i32, attrp: Option<&mut i32>) -> Schar {
    if grid.chars.borrow().is_empty() || row >= grid.rows.get() || col >= grid.cols.get() {
        return 0;
    }

    let off = grid.line_offset.borrow()[row as usize] + col as usize;
    if let Some(attrp) = attrp {
        *attrp = grid.attrs.borrow()[off];
    }
    grid.chars.borrow()[off]
}

pub fn grid_line_start(view: &mut GridView, mut row: i32) {
    let mut col: i32 = 0;
    let grid = grid_adjust(view, &mut row, &mut col);
    screengrid_line_start(grid, row, col);
}

pub fn screengrid_line_start(grid: &ScreenGrid, row: i32, col: i32) {
    let linebuf_size = g_st().linebuf_char.len();
    let grid_cols = grid.cols.get();
    let grid_target = grid.target_kind.get();
    grid_st(|st| {
        assert!(st.grid_line_grid.get().is_null());
        st.grid_line_row.set(row);
        st.grid_line_grid.set(grid_target);
        st.grid_line_coloff.set(col);
        st.grid_line_first.set(linebuf_size as i32);
        let maxcol = grid_cols.min(grid_cols - col);
        st.grid_line_maxcol.set(maxcol);
        st.grid_line_last.set(0);
        st.grid_line_clear_to.set(0);
        st.grid_line_bg_attr.set(0);
        st.grid_line_clear_attr.set(0);
        st.grid_line_flags.set(0);
        assert!(maxcol as usize <= linebuf_size);
    });

    if g_st().full_screen.get() && (opt_vars().rdb_flags.get() & K_OPT_RDB_FLAG_INVALID) != 0 {
        assert!(linebuf_size > 0);
        g_st().linebuf_char.fill(u32::MAX);
        g_st().linebuf_attr.fill(-1);
    }
}

pub fn grid_line_getchar(mut col: i32, mut attr: Option<&mut i32>) -> Schar {
    grid_st(|st| {
        if col < st.grid_line_maxcol.get() {
            col += st.grid_line_coloff.get();
            let grid = st.grid_line_grid.get().borrow();
            let off = grid.line_offset.borrow()[st.grid_line_row.get() as usize] + col as usize;
            if let Some(ref mut p) = attr {
                **p = grid.attrs.borrow()[off];
            }
            grid.chars.borrow()[off]
        } else {
            schar_from_ascii(b' ')
        }
    })
}

pub fn grid_line_put_schar(col: i32, schar: Schar, attr: i32) {
    let maxcol = grid_st(|st| {
        assert!(!st.grid_line_grid.get().is_null());
        st.grid_line_maxcol.get()
    });
    if col >= maxcol {
        return;
    }

    g_st().linebuf_char[col as usize] = schar;
    g_st().linebuf_attr[col as usize] = attr;

    grid_st(|st| {
        st.grid_line_first.set(st.grid_line_first.get().min(col));
        st.grid_line_last.set(st.grid_line_last.get().max(col + 1));
    });
    g_st().linebuf_vcol[col as usize] = -1;
}

pub fn grid_line_puts(mut col: i32, text: &std::ffi::CStr, textlen: i32, attr: i32) -> i32 {
    let text_bytes = text.to_bytes();
    let (max_col, line_first, line_last) = grid_st(|st| {
        assert!(!st.grid_line_grid.get().is_null());
        (
            st.grid_line_maxcol.get(),
            st.grid_line_first.get(),
            st.grid_line_last.get(),
        )
    });
    let mut len = if textlen >= 0 { textlen } else { i32::MAX };

    let start_col = col;
    let mut ci = utf_ptr2_str_char_info_len(text_bytes, len);
    let init_ptr_len = ci.ptr.len();
    while col < max_col && len > 0 && !ci.ptr.is_empty() {
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

        if ci.ptr.len() == init_ptr_len
            && col > line_first
            && col < line_last
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
        grid_st(|st| {
            st.grid_line_first
                .set(st.grid_line_first.get().min(start_col));
            st.grid_line_last.set(st.grid_line_last.get().max(col));
        });
    }

    col - start_col
}

pub fn grid_line_fill(start_col: i32, mut end_col: i32, sc: Schar, attr: i32) -> i32 {
    end_col = end_col.min(grid_st(|st| st.grid_line_maxcol.get()));
    if start_col >= end_col {
        return end_col;
    }

    for col in start_col..end_col {
        g_st().linebuf_char[col as usize] = sc;
        g_st().linebuf_attr[col as usize] = attr;
        g_st().linebuf_vcol[col as usize] = -1;
    }

    grid_st(|st| {
        st.grid_line_first
            .set(st.grid_line_first.get().min(start_col));
        st.grid_line_last.set(st.grid_line_last.get().max(end_col));
    });
    end_col
}

pub fn grid_line_clear_end(start_col: i32, end_col: i32, bg_attr: i32, clear_attr: i32) {
    grid_st(|st| {
        if st.grid_line_first.get() > start_col {
            st.grid_line_first.set(start_col);
            st.grid_line_last.set(start_col);
        }
        st.grid_line_clear_to.set(end_col);
        st.grid_line_bg_attr.set(bg_attr);
        st.grid_line_clear_attr.set(clear_attr);
    });
}

pub fn grid_line_cursor_goto(col: i32) {
    let (handle, row) = grid_st(|st| {
        (
            st.grid_line_grid.get().borrow().handle.get(),
            st.grid_line_row.get(),
        )
    });
    ui_grid_cursor_goto(handle, row, col);
}

pub fn grid_line_mirror(width: i32) {
    let (mut first, mut last, mut clear_to) = grid_st(|st| {
        let last = st.grid_line_last.get();
        let clear = last.max(st.grid_line_clear_to.get());
        st.grid_line_clear_to.set(clear);
        (st.grid_line_first.get(), last, clear)
    });
    if first >= clear_to {
        return;
    }
    linebuf_mirror(&mut first, &mut last, &mut clear_to, width);
    grid_st(|st| {
        st.grid_line_first.set(first);
        st.grid_line_last.set(last);
        st.grid_line_clear_to.set(clear_to);
        st.grid_line_flags
            .set(st.grid_line_flags.get() | SLF_RIGHTLEFT);
    });
}

pub fn linebuf_mirror(firstp: &mut i32, lastp: &mut i32, clearp: &mut i32, width: i32) {
    let first = *firstp;
    let last = *lastp;
    let first_u = first as usize;
    let last_u = last as usize;

    let mirror = width - 1;
    grid_st(|st| {
        let mut scratch = st.linebuf_scratch.borrow_mut();
        for (dst, &src) in scratch[first_u..last_u]
            .iter_mut()
            .zip(&g_st().linebuf_char[first_u..last_u])
        {
            *dst = src as i32;
        }
        let mut col = first;
        while col < last {
            let rev = mirror - col;
            if col + 1 < last && scratch[(col + 1) as usize] == 0 {
                g_st().linebuf_char[(rev - 1) as usize] = scratch[col as usize] as Schar;
                g_st().linebuf_char[rev as usize] = 0;
                col += 1;
            } else {
                g_st().linebuf_char[rev as usize] = scratch[col as usize] as Schar;
            }
            col += 1;
        }

        for (dst, &src) in scratch[first_u..last_u]
            .iter_mut()
            .zip(&g_st().linebuf_attr[first_u..last_u])
        {
            *dst = src;
        }
        for c in first..last {
            g_st().linebuf_attr[(mirror - c) as usize] = scratch[c as usize] as Sattr;
        }

        for (dst, &src) in scratch[first_u..last_u]
            .iter_mut()
            .zip(&g_st().linebuf_vcol[first_u..last_u])
        {
            *dst = src;
        }
        for c in first..last {
            g_st().linebuf_vcol[(mirror - c) as usize] = scratch[c as usize] as Colnr;
        }
    });

    *firstp = width - *clearp;
    *clearp = width - first;
    *lastp = width - last;
}

pub fn grid_line_flush() {
    let (grid, row, coloff, first, last, clear_to, bg_attr, clear_attr, flags) = grid_st(|st| {
        let g = st.grid_line_grid.replace(GridTarget::None);
        let last = st.grid_line_last.get();
        let clear = last.max(st.grid_line_clear_to.get());
        st.grid_line_clear_to.set(clear);
        assert!(clear <= st.grid_line_maxcol.get());
        (
            g,
            st.grid_line_row.get(),
            st.grid_line_coloff.get(),
            st.grid_line_first.get(),
            last,
            clear,
            st.grid_line_bg_attr.get(),
            st.grid_line_clear_attr.get(),
            st.grid_line_flags.get(),
        )
    });
    if first >= clear_to {
        return;
    }

    grid_put_linebuf(
        grid.borrow_mut(),
        row,
        coloff,
        first,
        last,
        clear_to,
        bg_attr,
        clear_attr,
        -1,
        flags,
    );
}

pub fn grid_line_flush_if_valid_row() {
    let invalid = grid_st(|st| {
        st.grid_line_row.get() < 0
            || st.grid_line_row.get() >= st.grid_line_grid.get().borrow().rows.get()
    });
    if invalid {
        if (opt_vars().rdb_flags.get() & K_OPT_RDB_FLAG_INVALID) != 0 {
            std::process::abort();
        } else {
            grid_st(|st| st.grid_line_grid.set(GridTarget::None));
            return;
        }
    }
    grid_line_flush();
}

pub fn grid_clear(
    grid: &mut GridView,
    start_row: i32,
    end_row: i32,
    start_col: i32,
    mut end_col: i32,
    attr: i32,
) {
    for row in start_row..end_row {
        grid_line_start(grid, row);
        let bail = grid_st(|st| {
            end_col = end_col.min(st.grid_line_maxcol.get());
            if st.grid_line_row.get() >= st.grid_line_grid.get().borrow().rows.get()
                || start_col >= end_col
            {
                st.grid_line_grid.set(GridTarget::None);
                true
            } else {
                false
            }
        });
        if bail {
            return;
        }
        grid_line_clear_end(start_col, end_col, attr, 0);
        grid_line_flush();
    }
}

#[inline]
fn grid_char_needs_redraw(grid: &ScreenGrid, col: i32, off_to: usize, cols: i32) -> bool {
    let chars = grid.chars.borrow();
    let attrs = grid.attrs.borrow();
    cols > 0
        && ((g_st().linebuf_char[col as usize] != chars[off_to]
            || g_st().linebuf_attr[col as usize] != attrs[off_to]
            || (cols > 1
                && g_st().linebuf_char[(col + 1) as usize] == 0
                && g_st().linebuf_char[(col + 1) as usize] != chars[off_to + 1]))
            || (opt_vars().rdb_flags.get() & K_OPT_RDB_FLAG_NODELTA) != 0)
}

pub fn grid_put_linebuf(
    grid: &ScreenGrid,
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
    assert!(0 <= row && row < grid.rows.get());
    if endcol > grid.cols.get() {
        endcol = grid.cols.get();
    }

    if grid.chars.borrow().is_empty() || row >= grid.rows.get() || coloff >= grid.cols.get() {
        return;
    }

    let invalid_row =
        !ptr::eq(grid, &raw const g_st().default_grid) && grid_invalid_row(grid, row) && col == 0;
    let line_off = grid.line_offset.borrow()[row as usize];
    let off_to = line_off + coloff as usize;
    let max_off_to = line_off + grid.cols.get() as usize;

    if col > 0 && grid.chars.borrow()[off_to + col as usize] == 0 {
        g_st().linebuf_char[(col - 1) as usize] = schar_from_ascii(b'>');
        g_st().linebuf_attr[(col - 1) as usize] = grid.attrs.borrow()[off_to + col as usize - 1];
        col -= 1;
    }

    let mut clear_start = endcol;
    if (flags & SLF_RIGHTLEFT) != 0 {
        clear_start = col;
        col = endcol;
        endcol = clear_width;
        clear_width = col;
    }

    if opt_vars().p_arshape.get() && !opt_vars().p_tbidi.get() && endcol > col {
        line_do_arabic_shape(&mut g_st().linebuf_char[col as usize..endcol as usize]);
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
                && grid.chars.borrow()[off + char_cells as usize] == 0
            {
                clear_next = true;
            }

            {
                let mut chars = grid.chars.borrow_mut();
                chars[off] = g_st().linebuf_char[col as usize];
                if char_cells == 2 {
                    chars[off + 1] = g_st().linebuf_char[(col + 1) as usize];
                }
            }

            {
                let mut attrs = grid.attrs.borrow_mut();
                attrs[off] = g_st().linebuf_attr[col as usize];
                if char_cells == 2 {
                    attrs[off + 1] = g_st().linebuf_attr[col as usize];
                }
            }
        }

        {
            let mut vcols = grid.vcols.borrow_mut();
            vcols[off] = g_st().linebuf_vcol[col as usize];
            if char_cells == 2 {
                vcols[off + 1] = g_st().linebuf_vcol[(col + 1) as usize];
            }
        }

        col += char_cells;
    }

    if clear_next {
        grid.chars.borrow_mut()[off_to + col as usize] = schar_from_ascii(b' ');
        end_dirty += 1;
    }

    if off_to + (clear_width as usize) < max_off_to
        && grid.chars.borrow()[off_to + clear_width as usize] == 0
    {
        clear_width += 1;
    }

    let mut clear_dirty_start: i32 = -1;
    let mut clear_end: i32 = -1;
    if (flags & SLF_RIGHTLEFT) != 0 {
        let mut c = clear_width - 1;
        let mut vcols = grid.vcols.borrow_mut();
        while c >= clear_start {
            let off = off_to + c as usize;
            vcols[off] = if (flags & SLF_INC_VCOL) != 0 {
                last_vcol += 1;
                last_vcol
            } else {
                last_vcol
            };
            c -= 1;
        }
    }
    clear_attr = hl_combine_attr(bg_attr, clear_attr);
    {
        let mut chars = grid.chars.borrow_mut();
        let mut attrs = grid.attrs.borrow_mut();
        let mut vcols = grid.vcols.borrow_mut();
        for c in clear_start..clear_width {
            let off = off_to + c as usize;
            if chars[off] != schar_from_ascii(b' ')
                || attrs[off] != clear_attr
                || (opt_vars().rdb_flags.get() & K_OPT_RDB_FLAG_NODELTA) != 0
            {
                chars[off] = schar_from_ascii(b' ');
                attrs[off] = clear_attr;
                if clear_dirty_start == -1 {
                    clear_dirty_start = c;
                }
                clear_end = c + 1;
            }
            if (flags & SLF_RIGHTLEFT) == 0 {
                vcols[off] = if (flags & SLF_INC_VCOL) != 0 {
                    last_vcol += 1;
                    last_vcol
                } else {
                    last_vcol
                };
            }
        }
    }

    if (flags & SLF_RIGHTLEFT) != 0 && start_dirty != -1 && clear_dirty_start != -1 {
        if grid.throttled.get() || clear_dirty_start >= start_dirty - 5 {
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
        if !grid.throttled.get() {
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
        } else if !grid.dirty_col.borrow().is_empty()
            && clear_end > grid.dirty_col.borrow()[row as usize]
        {
            grid.dirty_col.borrow_mut()[row as usize] = clear_end;
        }
    }
}

pub fn grid_alloc(grid: &ScreenGrid, rows: i32, columns: i32, copy: bool, valid: bool) {
    assert!(rows >= 0 && columns >= 0);
    let ncells = rows as usize * columns as usize;
    let ngrid = ScreenGrid {
        handle: grid.handle.clone(),
        target_kind: grid.target_kind.clone(),
        chars: std::cell::RefCell::new(vec![0; ncells]),
        attrs: std::cell::RefCell::new(vec![0; ncells]),
        vcols: std::cell::RefCell::new(vec![-1; ncells]),
        line_offset: std::cell::RefCell::new(vec![0; rows as usize]),
        dirty_col: std::cell::RefCell::new(std::mem::take(&mut *grid.dirty_col.borrow_mut())),
        rows: std::cell::Cell::new(rows),
        cols: std::cell::Cell::new(columns),
        valid: grid.valid.clone(),
        throttled: grid.throttled.clone(),
        blending: grid.blending.clone(),
        mouse_enabled: grid.mouse_enabled.clone(),
        zindex: grid.zindex.clone(),
        comp_row: grid.comp_row.clone(),
        comp_col: grid.comp_col.clone(),
        comp_width: grid.comp_width.clone(),
        comp_height: grid.comp_height.clone(),
        comp_index: grid.comp_index.clone(),
        comp_disabled: grid.comp_disabled.clone(),
        pending_comp_index_update: grid.pending_comp_index_update.clone(),
    };

    for new_row in 0..ngrid.rows.get() {
        let row_off = new_row as usize * ngrid.cols.get() as usize;
        ngrid.line_offset.borrow_mut()[new_row as usize] = row_off;

        grid_clear_line(&ngrid, row_off, columns, valid);

        if copy && new_row < grid.rows.get() && !grid.chars.borrow().is_empty() {
            let len = grid.cols.get().min(ngrid.cols.get()) as usize;
            let old_off = grid.line_offset.borrow()[new_row as usize];
            ngrid.chars.borrow_mut()[row_off..row_off + len]
                .copy_from_slice(&grid.chars.borrow()[old_off..old_off + len]);
            ngrid.attrs.borrow_mut()[row_off..row_off + len]
                .copy_from_slice(&grid.attrs.borrow()[old_off..old_off + len]);
            ngrid.vcols.borrow_mut()[row_off..row_off + len]
                .copy_from_slice(&grid.vcols.borrow()[old_off..old_off + len]);
        }
    }
    *grid.chars.borrow_mut() = ngrid.chars.into_inner();
    *grid.attrs.borrow_mut() = ngrid.attrs.into_inner();
    *grid.vcols.borrow_mut() = ngrid.vcols.into_inner();
    *grid.line_offset.borrow_mut() = ngrid.line_offset.into_inner();
    *grid.dirty_col.borrow_mut() = ngrid.dirty_col.into_inner();
    grid.rows.set(rows);
    grid.cols.set(columns);

    let cols = columns as usize;
    if g_st().linebuf_char.len() < cols {
        g_st().linebuf_char = vec![0; cols];
        g_st().linebuf_attr = vec![0; cols];
        g_st().linebuf_vcol = vec![0; cols];
        grid_st(|st| *st.linebuf_scratch.borrow_mut() = vec![0; cols]);
    }
}

pub fn grid_free(grid: &ScreenGrid) {
    *grid.chars.borrow_mut() = Vec::new();
    *grid.attrs.borrow_mut() = Vec::new();
    *grid.vcols.borrow_mut() = Vec::new();
    *grid.line_offset.borrow_mut() = Vec::new();
}

pub fn grid_free_all_mem() {
    grid_free(&g_st().default_grid);
    grid_free(&g_st().msg_grid);
    *g_st().msg_grid.dirty_col.borrow_mut() = Vec::new();
    g_st().linebuf_char = Vec::new();
    g_st().linebuf_attr = Vec::new();
    g_st().linebuf_vcol = Vec::new();
    grid_st(|st| {
        *st.linebuf_scratch.borrow_mut() = Vec::new();
        *st.glyph_cache_keys.borrow_mut() = Vec::new();
        *st.glyph_cache_offsets.borrow_mut() = std::collections::BTreeMap::new();
    });
}

pub fn win_grid_alloc(wp: &mut WinInfoInfo) {
    wp.w_grid_alloc.target_kind.set(GridTarget::Win(wp.handle));

    let total_rows = wp.w_height_outer.get();
    let total_cols = wp.w_width_outer.get();

    let want_allocation = ui_has(K_UIMULTIGRID) || wp.w_floating.get();
    let has_allocation = !wp.w_grid_alloc.chars.borrow().is_empty();

    if (wp.w_view_height.get() as usize) + 1 > wp.w_lines.len() {
        wp.w_lines_valid.set(0);
        wp.w_lines.clear();
        wp.w_lines
            .resize(wp.w_view_height.get() as usize + 1, Wline::default());
    }

    let mut was_resized = false;
    if want_allocation
        && (!has_allocation
            || wp.w_grid_alloc.rows.get() != total_rows
            || wp.w_grid_alloc.cols.get() != total_cols)
    {
        let is_valid = wp.w_grid_alloc.valid.get();
        grid_alloc(&wp.w_grid_alloc, total_rows, total_cols, is_valid, false);
        wp.w_grid_alloc.valid.set(true);
        if wp.w_floating.get() && wp.w_config.border {
            wp.w_redr_border.set(true);
        }
        was_resized = true;
    } else if !want_allocation && has_allocation {
        grid_free(&wp.w_grid_alloc);
        wp.w_grid_alloc.valid.set(false);
        was_resized = true;
    } else if want_allocation && has_allocation && !wp.w_grid_alloc.valid.get() {
        grid_invalidate(&wp.w_grid_alloc);
        wp.w_grid_alloc.valid.set(true);
    }

    if want_allocation {
        wp.w_grid.target.set(GridTarget::Win(wp.handle));
        wp.w_grid.row_offset.set(wp.w_winrow_off.get());
        wp.w_grid.col_offset.set(wp.w_wincol_off.get());
    } else {
        wp.w_grid.target.set(GridTarget::Default);
        wp.w_grid
            .row_offset
            .set(wp.w_winrow.get() + wp.w_winrow_off.get());
        wp.w_grid
            .col_offset
            .set(wp.w_wincol.get() + wp.w_wincol_off.get());
    }

    if (g_st().resizing_screen.get() || was_resized) && want_allocation {
        ui_call_grid_resize(
            i64::from(wp.w_grid_alloc.handle.get()),
            i64::from(wp.w_grid_alloc.cols.get()),
            i64::from(wp.w_grid_alloc.rows.get()),
        );
        ui_check_cursor_grid(wp.w_grid_alloc.handle.get());
    }
}

pub fn grid_assign_handle(grid: &mut ScreenGrid) {
    if grid.handle.get() == 0 {
        let h = grid_st(|st| {
            let h = st.last_grid_handle.get() + 1;
            st.last_grid_handle.set(h);
            h
        });
        grid.handle.set(h);
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
        if width != grid.cols.get() {
            let mut j = end - 1 - i;
            while {
                j -= line_count;
                j >= row
            } {
                linecopy(grid, j + line_count, j, col, width);
            }
            j += line_count;
            let clear_off = grid.line_offset.borrow()[j as usize] + col as usize;
            grid_clear_line(grid, clear_off, width, false);
        } else {
            let mut j = end - 1 - i;
            let temp = grid.line_offset.borrow()[j as usize] as u32 as usize;
            while {
                j -= line_count;
                j >= row
            } {
                let v = grid.line_offset.borrow()[j as usize];
                grid.line_offset.borrow_mut()[(j + line_count) as usize] = v;
            }
            grid.line_offset.borrow_mut()[(j + line_count) as usize] = temp;
            let cols = grid.cols.get();
            grid_clear_line(grid, temp, cols, false);
        }
    }

    if !grid.throttled.get() {
        ui_call_grid_scroll(
            i64::from(grid.handle.get()),
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
    grid: &ScreenGrid,
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
        if width != grid.cols.get() {
            let mut j = row + i;
            while {
                j += line_count;
                j < end
            } {
                linecopy(grid, j - line_count, j, col, width);
            }
            j -= line_count;
            let clear_off = grid.line_offset.borrow()[j as usize] + col as usize;
            grid_clear_line(grid, clear_off, width, false);
        } else {
            let mut j = row + i;
            let temp = grid.line_offset.borrow()[j as usize] as u32 as usize;
            while {
                j += line_count;
                j < end
            } {
                let v = grid.line_offset.borrow()[j as usize];
                grid.line_offset.borrow_mut()[(j - line_count) as usize] = v;
            }
            grid.line_offset.borrow_mut()[(j - line_count) as usize] = temp;
            let cols = grid.cols.get();
            grid_clear_line(grid, temp, cols, false);
        }
    }

    if !grid.throttled.get() {
        ui_call_grid_scroll(
            i64::from(grid.handle.get()),
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
        let mut text = text_cstr;
        if overflow > 0 {
            let cells = mb_string2cells(text_cstr.to_bytes()) as i32;
            if overflow >= cells {
                overflow -= cells;
                continue;
            }
            let text_bytes = text_cstr.to_bytes();
            let mut off = 0usize;
            while off < text_bytes.len() && overflow > 0 {
                overflow -= utf_ptr2cells(&text_bytes[off..]);
                off += crate::mbyte::utfc_slice2len(&text_bytes[off..]).max(1);
            }
            text = &text_cstr[off.min(text_bytes.len())..];
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
    adj: Option<&[i32; 4]>,
    winbl: i32,
    hl_attr: Option<&crate::highlight::NSHlAttr>,
    win_hl_attr_normal: i32,
) {
    let default_adj: [i32; 4] = [1, 1, 1, 1];
    let adj = adj.unwrap_or(&default_adj);
    let mut chars = [0 as Schar; 8];
    let mut border_attrs = [0_i32; 8];

    for i in 0..8 {
        let bchars = &config.border_chars[i];
        let blen = bchars.iter().position(|&b| b == 0).unwrap_or(bchars.len());
        chars[i] = schar_from_bytes(&bchars[..blen]);
        border_attrs[i] = hl_combine_attr(win_hl_attr_normal, config.border_attr[i]);
    }

    let adj0 = adj[0];
    let adj1 = adj[1];
    let adj2 = adj[2];
    let adj3 = adj[3];

    let irow = grid.rows.get() - adj0 - adj2;
    let icol = grid.cols.get() - adj1 - adj3;

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

fn linecopy(grid: &ScreenGrid, to: i32, from: i32, col: i32, width: i32) {
    let off_to = (grid.line_offset.borrow()[to as usize] + col as usize) as u32 as usize;
    let off_from = (grid.line_offset.borrow()[from as usize] + col as usize) as u32 as usize;
    let len = width as usize;

    grid.chars
        .borrow_mut()
        .copy_within(off_from..off_from + len, off_to);
    grid.attrs
        .borrow_mut()
        .copy_within(off_from..off_from + len, off_to);
    grid.vcols
        .borrow_mut()
        .copy_within(off_from..off_from + len, off_to);
}

pub fn get_win_by_grid_handle(handle: ObjHandle) -> Option<&'static mut WinInfoInfo> {
    for_all_windows_ptr().find(|wp| wp.w_grid_alloc.handle.get() == handle)
}

/// Put a unicode character in a screen cell (`schar_from_char`).
pub fn schar_from_char(mut c: i32) -> Schar {
    if c >= 0x20_0000 {
        c = 0xFFFD;
    }
    let mut buf = [0u8; 4];
    utf_char2bytes(c, &mut buf);
    u32::from_ne_bytes(buf)
}

#[cfg(target_endian = "big")]
pub const BRAILLE_SPACE: Schar = 0xe2a0_8000;
#[cfg(not(target_endian = "big"))]
pub const BRAILLE_SPACE: Schar = 0x0080_a0e2;

#[inline]
pub fn schar_invisible(sc: Schar) -> bool {
    sc == schar_from_ascii(b' ') || sc == BRAILLE_SPACE
}

pub fn schar_from_cluster_impl(buf: &[u8], first_compose: bool) -> Schar {
    let mut len = buf.len();
    let maxlen = MAX_SCHAR_SIZE - 1 - usize::from(first_compose);
    if len > maxlen {
        len = maxlen;
        while (buf[len] & 0xC0) != 0xC0 {
            len -= 1;
        }
    }

    if first_compose {
        let mut cbuf = [0u8; MAX_SCHAR_SIZE];
        cbuf[0] = b' ';
        cbuf[1..1 + len].copy_from_slice(&buf[..len]);
        schar_from_buf(&cbuf[..len + 1])
    } else {
        schar_from_buf(&buf[..len])
    }
}

#[inline]
pub fn schar_from_cluster(ci: StrCharInfo<'_>, cli: ClusterInfo<'_>) -> Schar {
    assert!(ci.chr.value >= 0);
    let len = ci.ptr.len() - cli.next.ptr.len();
    if len == 1 {
        return schar_from_ascii(ci.chr.value as u8);
    }
    if cli.first_compose || len > 4 {
        return schar_from_cluster_impl(&ci.ptr[..len], cli.first_compose);
    }
    let mut bytes = [0u8; 4];
    bytes[..len].copy_from_slice(&ci.ptr[..len]);
    u32::from_ne_bytes(bytes)
}

#[cfg(test)]
pub fn unittest_schar_from_cluster(buf: &std::ffi::CStr, firstc: &mut i32) -> Schar {
    let ci = crate::mbyte::utf_ptr2_str_char_info(buf);
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
            let len = schar_get(&mut buf, sc_u);
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
