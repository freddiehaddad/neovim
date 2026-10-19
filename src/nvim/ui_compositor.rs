//! Compositor: merge floating grids with the main grid for display in TUI and
//! non-multigrid UIs (`src/nvim/ui_compositor.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::globals::*;
#[allow(unused_imports)]
use crate::grid::{schar_from_buf, schar_from_char};
#[allow(unused_imports)]
use crate::highlight::hl_blend_attrs;
#[allow(unused_imports)]
use crate::highlight_group::{syn_check_group, syn_id2attr};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::ui::{
    ui_call_flush, ui_composed_call_grid_cursor_goto, ui_composed_call_grid_resize,
    ui_composed_call_grid_scroll, ui_has,
};
use std::ffi::{c_int, c_uint};
use std::ptr;

use crate::api::tabpage::ScreenGrid;
use crate::api::win_config::UIExtension;
use crate::kvec::KVec;
use crate::lua::converter::Boolean;
use crate::map_glyph_cache::NvimString;
use crate::memory::{xfree, xmalloc, xrealloc};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::os::time::os_sleep;

pub type schar_T = u32;
pub type sattr_T = i32;
pub type LineFlags = c_int;
pub type OptInt = i64;

const kUIMultigrid: UIExtension = 6;
const kLineFlagWrap: LineFlags = 1;
const kLineFlagInvalid: LineFlags = 2;

const kOptRdbFlagCompositor: c_uint = 0x01;
const kOptRdbFlagInvalid: c_uint = 0x04;

const HLF_MSGSEP: usize = 61;

pub use crate::msgpack_rpc::channel::RemoteUI;

#[inline]
const fn schar_from_ascii(c: u8) -> schar_T {
    if cfg!(target_endian = "big") {
        (c as schar_T) << 24
    } else {
        c as schar_T
    }
}

static mut composed_uis: c_int = 0;
static mut layers: KVec<*mut ScreenGrid> = KVec {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};

static mut bufsize: usize = 0;
static mut linebuf: *mut schar_T = ptr::null_mut();
static mut attrbuf: *mut sattr_T = ptr::null_mut();

static mut chk_width: c_int = 0;
static mut chk_height: c_int = 0;

static mut curgrid: *mut ScreenGrid = ptr::null_mut();

static mut valid_screen: bool = true;
static mut msg_current_row: c_int = c_int::MAX;
static mut msg_was_scrolled: bool = false;

static mut msg_sep_row: c_int = -1;
static mut msg_sep_char: schar_T = schar_from_ascii(b' ');

static mut dbghl_normal: c_int = 0;
static mut dbghl_clear: c_int = 0;
static mut dbghl_composed: c_int = 0;
static mut dbghl_recompose: c_int = 0;

unsafe extern "C" {

    fn ui_composed_call_raw_line(
        grid: Integer,
        row: Integer,
        startcol: Integer,
        endcol: Integer,
        clearcol: Integer,
        clearattr: Integer,
        flags: LineFlags,
        chunk: *const schar_T,
        attrs: *const sattr_T,
    );
}

#[inline]
unsafe fn push_layer(g: *mut ScreenGrid) {
    unsafe {
        if layers.size == layers.capacity {
            layers.capacity = if layers.capacity > 0 {
                layers.capacity * 2
            } else {
                8
            };
            layers.items = xrealloc(
                layers.items.cast(),
                layers.capacity * size_of::<*mut ScreenGrid>(),
            )
            .cast();
        }
        *layers.items.add(layers.size) = g;
        layers.size += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_init() {
    unsafe {
        push_layer(&raw mut default_grid);
        curgrid = &raw mut default_grid;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_free_all_mem() {
    unsafe {
        xfree(layers.items.cast());
        layers = KVec {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        xfree(linebuf.cast());
        linebuf = ptr::null_mut();
        xfree(attrbuf.cast());
        attrbuf = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_syn_init() {
    unsafe {
        dbghl_normal = syn_check_group(c"RedrawDebugNormal".as_ptr(), 17);
        dbghl_clear = syn_check_group(c"RedrawDebugClear".as_ptr(), 16);
        dbghl_composed = syn_check_group(c"RedrawDebugComposed".as_ptr(), 19);
        dbghl_recompose = syn_check_group(c"RedrawDebugRecompose".as_ptr(), 20);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_attach(ui: *mut RemoteUI) {
    unsafe {
        composed_uis += 1;
        (*ui).composed = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_detach(ui: *mut RemoteUI) {
    unsafe {
        composed_uis -= 1;
        if composed_uis == 0 {
            xfree(linebuf.cast());
            linebuf = ptr::null_mut();
            xfree(attrbuf.cast());
            attrbuf = ptr::null_mut();
            bufsize = 0;
        }
        (*ui).composed = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_should_draw() -> bool {
    unsafe { composed_uis != 0 && valid_screen }
}

/// Raises or lowers the layer, syncing `comp_index` with `zindex` (`ui_comp_layers_adjust`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_layers_adjust(mut layer_idx: usize, raise: bool) {
    unsafe {
        let size = layers.size;
        let layer = *layers.items.add(layer_idx);

        if raise {
            while layer_idx < size - 1
                && (*layer).zindex > (**layers.items.add(layer_idx + 1)).zindex
            {
                let next = *layers.items.add(layer_idx + 1);
                *layers.items.add(layer_idx) = next;
                (*next).comp_index = layer_idx;
                (*next).pending_comp_index_update = true;
                layer_idx += 1;
            }
        } else {
            while layer_idx > 0 && (*layer).zindex < (**layers.items.add(layer_idx - 1)).zindex {
                let prev = *layers.items.add(layer_idx - 1);
                *layers.items.add(layer_idx) = prev;
                (*prev).comp_index = layer_idx;
                (*prev).pending_comp_index_update = true;
                layer_idx -= 1;
            }
        }
        *layers.items.add(layer_idx) = layer;
        (*layer).comp_index = layer_idx;
        (*layer).pending_comp_index_update = true;
    }
}

/// Places `grid` at `(col, row)` position with `(width * height)` size (`ui_comp_put_grid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_put_grid(
    grid: *mut ScreenGrid,
    row: c_int,
    col: c_int,
    height: c_int,
    width: c_int,
    valid: bool,
    on_top: bool,
) -> bool {
    unsafe {
        let moved: bool;
        (*grid).pending_comp_index_update = true;

        if (*grid).comp_index != 0 {
            moved = (row != (*grid).comp_row) || (col != (*grid).comp_col);
            if ui_comp_should_draw() {
                (*grid).comp_disabled = true;
                compose_area(
                    (*grid).comp_row as Integer,
                    row as Integer,
                    (*grid).comp_col as Integer,
                    ((*grid).comp_col + (*grid).comp_width) as Integer,
                );
                if (*grid).comp_col < col {
                    compose_area(
                        row.max((*grid).comp_row) as Integer,
                        (row + height).min((*grid).comp_row + (*grid).comp_height) as Integer,
                        (*grid).comp_col as Integer,
                        col as Integer,
                    );
                }
                if col + width < (*grid).comp_col + (*grid).comp_width {
                    compose_area(
                        row.max((*grid).comp_row) as Integer,
                        (row + height).min((*grid).comp_row + (*grid).comp_height) as Integer,
                        (col + width) as Integer,
                        ((*grid).comp_col + (*grid).comp_width) as Integer,
                    );
                }
                compose_area(
                    (row + height) as Integer,
                    ((*grid).comp_row + (*grid).comp_height) as Integer,
                    (*grid).comp_col as Integer,
                    ((*grid).comp_col + (*grid).comp_width) as Integer,
                );
                (*grid).comp_disabled = false;
            }
            (*grid).comp_row = row;
            (*grid).comp_col = col;
        } else {
            moved = true;
            for i in 0..layers.size {
                debug_assert!(*layers.items.add(i) != grid);
            }

            let mut insert_at = layers.size;
            while insert_at > 0 && (**layers.items.add(insert_at - 1)).zindex > (*grid).zindex {
                insert_at -= 1;
            }

            if !curwin.is_null()
                && *layers.items.add(insert_at - 1) == &raw mut (*curwin).w_grid_alloc
                && (**layers.items.add(insert_at - 1)).zindex == (*grid).zindex
                && !on_top
            {
                insert_at -= 1;
            }
            push_layer(ptr::null_mut());
            let mut i = layers.size - 1;
            while i > insert_at {
                let prev = *layers.items.add(i - 1);
                *layers.items.add(i) = prev;
                (*prev).comp_index = i;
                (*prev).pending_comp_index_update = true;
                i -= 1;
            }
            *layers.items.add(insert_at) = grid;

            (*grid).comp_row = row;
            (*grid).comp_col = col;
            (*grid).comp_index = insert_at;
            (*grid).pending_comp_index_update = true;
        }

        (*grid).comp_height = height;
        (*grid).comp_width = width;
        if moved && valid && ui_comp_should_draw() {
            compose_area(
                (*grid).comp_row as Integer,
                ((*grid).comp_row + (*grid).rows) as Integer,
                (*grid).comp_col as Integer,
                ((*grid).comp_col + (*grid).cols) as Integer,
            );
        }
        moved
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_remove_grid(grid: *mut ScreenGrid) {
    unsafe {
        debug_assert!(!ptr::eq(grid, &raw const default_grid));
        if (*grid).comp_index == 0 {
            return;
        }

        if curgrid == grid {
            curgrid = &raw mut default_grid;
        }

        for i in (*grid).comp_index..(layers.size - 1) {
            let next = *layers.items.add(i + 1);
            *layers.items.add(i) = next;
            (*next).comp_index = i;
            (*next).pending_comp_index_update = true;
        }
        layers.size -= 1;
        (*grid).comp_index = 0;
        (*grid).pending_comp_index_update = true;

        ui_comp_compose_grid(grid);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_set_grid(handle: handle_T) -> bool {
    unsafe {
        if (*curgrid).handle == handle {
            return true;
        }
        let mut grid: *mut ScreenGrid = ptr::null_mut();
        for i in 0..layers.size {
            let g = *layers.items.add(i);
            if (*g).handle == handle {
                grid = g;
                break;
            }
        }
        if !grid.is_null() {
            curgrid = grid;
            return true;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_raise_grid(grid: *mut ScreenGrid, new_index: usize) {
    unsafe {
        let old_index = (*grid).comp_index;
        for i in old_index..new_index {
            let next = *layers.items.add(i + 1);
            *layers.items.add(i) = next;
            (*next).comp_index = i;
            (*next).pending_comp_index_update = true;
        }
        *layers.items.add(new_index) = grid;
        (*grid).comp_index = new_index;
        (*grid).pending_comp_index_update = true;
        for i in old_index..new_index {
            let grid2 = *layers.items.add(i);
            let startcol = (*grid).comp_col.max((*grid2).comp_col);
            let endcol = ((*grid).comp_col + (*grid).cols).min((*grid2).comp_col + (*grid2).cols);
            compose_area(
                (*grid).comp_row.max((*grid2).comp_row) as Integer,
                ((*grid).comp_row + (*grid).rows).min((*grid2).comp_row + (*grid2).rows) as Integer,
                startcol as Integer,
                endcol as Integer,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_grid_cursor_goto(grid_handle: Integer, r: Integer, c: Integer) {
    unsafe {
        if !ui_comp_set_grid(grid_handle as c_int) {
            return;
        }
        let cursor_row = (*curgrid).comp_row + (r as c_int);
        let cursor_col = (*curgrid).comp_col + (c as c_int);

        if !ptr::eq(curgrid, &raw const default_grid) {
            let mut new_index = layers.size - 1;
            while new_index > 1 && (**layers.items.add(new_index)).zindex > (*curgrid).zindex {
                new_index -= 1;
            }
            if (*curgrid).comp_index < new_index {
                ui_comp_raise_grid(curgrid, new_index);
            }
        }

        if cursor_col >= default_grid.cols || cursor_row >= default_grid.rows {
            return;
        }
        ui_composed_call_grid_cursor_goto(1, cursor_row as Integer, cursor_col as Integer);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_mouse_focus(row: c_int, col: c_int) -> *mut ScreenGrid {
    unsafe {
        let mut i = layers.size as isize - 1;
        while i > 0 {
            let grid = *layers.items.add(i as usize);
            if (*grid).mouse_enabled
                && row >= (*grid).comp_row
                && row < (*grid).comp_row + (*grid).rows
                && col >= (*grid).comp_col
                && col < (*grid).comp_col + (*grid).cols
            {
                return grid;
            }
            i -= 1;
        }
        if ui_has(kUIMultigrid) {
            let mut wp = firstwin;
            while !wp.is_null() {
                let grid = &raw mut (*wp).w_grid_alloc;
                if (*grid).mouse_enabled
                    && row >= (*wp).w_winrow
                    && row < (*wp).w_winrow + (*grid).rows
                    && col >= (*wp).w_wincol
                    && col < (*wp).w_wincol + (*grid).cols
                {
                    return grid;
                }
                wp = (*wp).w_next;
            }
        }
        ptr::null_mut()
    }
}

/// Compute which grid is on top at supplied screen coordinates (`ui_comp_get_grid_at_coord`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_get_grid_at_coord(row: c_int, col: c_int) -> *mut ScreenGrid {
    unsafe {
        let mut i = layers.size as isize - 1;
        while i > 0 {
            let grid = *layers.items.add(i as usize);
            if row >= (*grid).comp_row
                && row < (*grid).comp_row + (*grid).rows
                && col >= (*grid).comp_col
                && col < (*grid).comp_col + (*grid).cols
            {
                return grid;
            }
            i -= 1;
        }

        let mut wp = firstwin;
        while !wp.is_null() {
            let grid = &raw mut (*wp).w_grid_alloc;
            if row >= (*grid).comp_row
                && row < (*grid).comp_row + (*grid).rows
                && col >= (*grid).comp_col
                && col < (*grid).comp_col + (*grid).cols
                && !(*wp).w_config.hide
            {
                return grid;
            }
            wp = (*wp).w_next;
        }
        &raw mut default_grid
    }
}

unsafe fn compose_line(
    row: Integer,
    mut startcol: Integer,
    mut endcol: Integer,
    mut flags: LineFlags,
) {
    unsafe {
        startcol = startcol.max(0);
        let mut skipstart: c_int = 0;
        let mut skipend: c_int = 0;
        if startcol > 0 && (flags & kLineFlagInvalid) != 0 {
            startcol -= 1;
            skipstart = 1;
        }
        if endcol < (default_grid.cols as Integer) && (flags & kLineFlagInvalid) != 0 {
            endcol += 1;
            skipend = 1;
        }

        let mut col = startcol as c_int;
        let mut grid: *mut ScreenGrid = ptr::null_mut();
        let line_off = *default_grid.line_offset.add(row as usize) + (startcol as usize);
        let bg_line = default_grid.chars.add(line_off);
        let bg_attrs = default_grid.attrs.add(line_off);

        while (col as Integer) < endcol {
            let mut until: c_int = 0;
            for i in 0..layers.size {
                let g = *layers.items.add(i);
                let grid_width = (*g).cols.min((*g).comp_width);
                let grid_height = (*g).rows.min((*g).comp_height);
                if ((*g).comp_row as Integer) > row
                    || row >= ((*g).comp_row + grid_height) as Integer
                    || (*g).comp_disabled
                {
                    continue;
                }
                if (*g).comp_col <= col && col < (*g).comp_col + grid_width {
                    grid = g;
                    until = (*g).comp_col + grid_width;
                } else if (*g).comp_col > col {
                    until = until.min((*g).comp_col);
                }
            }
            until = until.min(endcol as c_int);

            debug_assert!(!grid.is_null());
            debug_assert!(until > col);
            debug_assert!(until <= default_grid.cols);
            let n = (until - col) as usize;

            if row == (msg_sep_row as Integer) && (*grid).comp_index <= msg_grid.comp_index {
                grid = &raw mut msg_grid;
                let msg_sep_attr = *hl_attr_active.add(HLF_MSGSEP) as sattr_T;
                for i in col..until {
                    let idx = (i - startcol as c_int) as usize;
                    *linebuf.add(idx) = msg_sep_char;
                    *attrbuf.add(idx) = msg_sep_attr;
                }
            } else {
                let off = *(*grid)
                    .line_offset
                    .add((row as c_int - (*grid).comp_row) as usize)
                    + (col - (*grid).comp_col) as usize;
                let dst_idx = (col - startcol as c_int) as usize;
                ptr::copy_nonoverlapping((*grid).chars.add(off), linebuf.add(dst_idx), n);
                ptr::copy_nonoverlapping((*grid).attrs.add(off), attrbuf.add(dst_idx), n);
                if (*grid).comp_col + (*grid).cols > until && *(*grid).chars.add(off + n) == 0 {
                    *linebuf.add((until - 1 - startcol as c_int) as usize) = schar_from_ascii(b' ');
                    if (col as Integer) == startcol && n == 1 {
                        skipstart = 0;
                    }
                }
            }

            if (*grid).blending {
                let braille_blank = schar_from_char(0x2800);
                let mut i = col - (startcol as c_int);
                let limit = until - (startcol as c_int);
                let end_rel = (endcol - startcol) as c_int;
                while i < limit {
                    let mut width: c_int = 1;
                    let idx = i as usize;
                    let mut thru = (*linebuf.add(idx) == schar_from_ascii(b' ')
                        || *linebuf.add(idx) == braille_blank)
                        && *bg_line.add(idx) != 0;
                    if i + 1 < end_rel && *bg_line.add(idx + 1) == 0 {
                        width = 2;
                        thru &= *linebuf.add(idx + 1) == schar_from_ascii(b' ')
                            || *linebuf.add(idx + 1) == braille_blank;
                    }
                    *attrbuf.add(idx) =
                        hl_blend_attrs(*bg_attrs.add(idx), *attrbuf.add(idx), &raw mut thru)
                            as sattr_T;
                    if width == 2 {
                        *attrbuf.add(idx + 1) = hl_blend_attrs(
                            *bg_attrs.add(idx + 1),
                            *attrbuf.add(idx + 1),
                            &raw mut thru,
                        ) as sattr_T;
                    }
                    if thru {
                        ptr::copy_nonoverlapping(
                            bg_line.add(idx),
                            linebuf.add(idx),
                            width as usize,
                        );
                    }
                    i += width;
                }
            }

            let col_idx = (col - startcol as c_int) as usize;
            if *linebuf.add(col_idx) == 0 {
                *linebuf.add(col_idx) = schar_from_ascii(b' ');
                if (col as Integer) == endcol - 1 {
                    skipend = 0;
                }
            } else if (col as Integer) == startcol && n > 1 && *linebuf.add(1) == 0 {
                skipstart = 0;
            }

            col = until;
        }
        if *linebuf.add((endcol - startcol - 1) as usize) == 0 {
            skipend = 0;
        }

        debug_assert!(endcol <= chk_width as Integer);
        debug_assert!(row < chk_height as Integer);

        if !(!grid.is_null()
            && (ptr::eq(grid, &raw const default_grid)
                || ((*grid).comp_col == 0 && (*grid).cols == Columns)))
        {
            flags &= !kLineFlagWrap;
        }

        let end_idx = ((endcol - skipend as Integer) - startcol) as c_int;
        for i in skipstart..end_idx {
            let p = attrbuf.add(i as usize);
            if *p < 0 {
                if (rdb_flags & kOptRdbFlagInvalid) != 0 {
                    std::process::abort();
                } else {
                    *p = 0;
                }
            }
        }
        ui_composed_call_raw_line(
            1,
            row,
            startcol + (skipstart as Integer),
            endcol - (skipend as Integer),
            endcol - (skipend as Integer),
            0,
            flags,
            linebuf.add(skipstart as usize),
            attrbuf.add(skipstart as usize),
        );
    }
}

unsafe fn compose_debug(
    startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    mut endcol: Integer,
    syn_id: c_int,
    delay: bool,
) {
    unsafe {
        if (rdb_flags & kOptRdbFlagCompositor) == 0 || startcol >= endcol {
            return;
        }

        endrow = endrow.min(default_grid.rows as Integer);
        endcol = endcol.min(default_grid.cols as Integer);
        let attr = syn_id2attr(syn_id);

        if delay {
            debug_delay(endrow - startrow);
        }

        for row in (startrow as c_int)..(endrow as c_int) {
            ui_composed_call_raw_line(
                1,
                row as Integer,
                startcol,
                startcol,
                endcol,
                attr as Integer,
                0,
                linebuf,
                attrbuf,
            );
        }

        if delay {
            debug_delay(endrow - startrow);
        }
    }
}

unsafe fn debug_delay(lines: Integer) {
    unsafe {
        ui_call_flush();
        let wd = p_wd.unsigned_abs();
        let factor = lines.clamp(1, 5) as u64;
        os_sleep(factor * wd);
    }
}

unsafe fn compose_area(
    startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    mut endcol: Integer,
) {
    unsafe {
        compose_debug(startrow, endrow, startcol, endcol, dbghl_recompose, true);
        endrow = endrow.min(default_grid.rows as Integer);
        endcol = endcol.min(default_grid.cols as Integer);
        if endcol <= startcol {
            return;
        }
        for r in (startrow as c_int)..(endrow as c_int) {
            compose_line(r as Integer, startcol, endcol, kLineFlagInvalid);
        }
    }
}

/// Compose the area under the grid (`ui_comp_compose_grid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_compose_grid(grid: *mut ScreenGrid) {
    unsafe {
        if ui_comp_should_draw() {
            compose_area(
                (*grid).comp_row as Integer,
                ((*grid).comp_row + (*grid).rows) as Integer,
                (*grid).comp_col as Integer,
                ((*grid).comp_col + (*grid).cols) as Integer,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_raw_line(
    grid: Integer,
    mut row: Integer,
    mut startcol: Integer,
    mut endcol: Integer,
    mut clearcol: Integer,
    clearattr: Integer,
    mut flags: LineFlags,
    chunk: *const schar_T,
    attrs: *const sattr_T,
) {
    unsafe {
        if !ui_comp_should_draw() || !ui_comp_set_grid(grid as c_int) {
            return;
        }

        row += (*curgrid).comp_row as Integer;
        startcol += (*curgrid).comp_col as Integer;
        endcol += (*curgrid).comp_col as Integer;
        clearcol += (*curgrid).comp_col as Integer;
        if !ptr::eq(curgrid, &raw const default_grid) {
            flags &= !kLineFlagWrap;
        }

        debug_assert!(endcol <= clearcol);

        if row >= default_grid.rows as Integer {
            return;
        }
        if clearcol > default_grid.cols as Integer {
            if startcol >= default_grid.cols as Integer {
                return;
            }
            clearcol = default_grid.cols as Integer;
            endcol = endcol.min(clearcol);
        }

        let covered = curgrid_covered_above(
            row as c_int,
            row as c_int,
            startcol as c_int,
            clearcol as c_int,
        );
        if (flags & kLineFlagInvalid) != 0 || covered || (*curgrid).blending {
            compose_debug(row, row + 1, startcol, clearcol, dbghl_composed, true);
            compose_line(row, startcol, clearcol, flags);
        } else {
            compose_debug(
                row,
                row + 1,
                startcol,
                endcol,
                dbghl_normal,
                endcol >= clearcol,
            );
            compose_debug(row, row + 1, endcol, clearcol, dbghl_clear, true);
            for i in 0..((endcol - startcol) as usize) {
                debug_assert!(*attrs.add(i) >= 0);
            }
            ui_composed_call_raw_line(
                1, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
            );
        }
    }
}

/// The screen is invalid and will soon be cleared (`ui_comp_set_screen_valid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_set_screen_valid(valid: bool) -> bool {
    unsafe {
        let old_val = valid_screen;
        valid_screen = valid;
        if !valid {
            msg_sep_row = -1;
        }
        old_val
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_msg_set_pos(
    _grid: Integer,
    row: Integer,
    scrolled: Boolean,
    sep_char: NvimString,
    _zindex: Integer,
    _compindex: Integer,
) {
    unsafe {
        msg_grid.pending_comp_index_update = true;
        msg_grid.comp_row = row as c_int;
        if scrolled && row > 0 {
            msg_sep_row = (row as c_int) - 1;
            if !sep_char.data.is_null() {
                msg_sep_char = schar_from_buf(sep_char.data, sep_char.size);
            }
        } else {
            msg_sep_row = -1;
        }

        if row > (msg_current_row as Integer) && ui_comp_should_draw() {
            compose_area(
                (msg_current_row - 1).max(0) as Integer,
                row,
                0,
                default_grid.cols as Integer,
            );
        } else if row < (msg_current_row as Integer)
            && ui_comp_should_draw()
            && (msg_current_row < Rows || (scrolled && !msg_was_scrolled))
        {
            let delta = msg_current_row - (row as c_int);
            if msg_grid.blending {
                let first_row = ((row as c_int) - if scrolled { 1 } else { 0 }).max(0);
                compose_area(
                    first_row as Integer,
                    (Rows - delta) as Integer,
                    0,
                    Columns as Integer,
                );
            } else {
                let first_row = ((row as c_int) - if msg_was_scrolled { 1 } else { 0 }).max(0);
                ui_composed_call_grid_scroll(
                    1,
                    first_row as Integer,
                    Rows as Integer,
                    0,
                    Columns as Integer,
                    delta as Integer,
                    0,
                );
                if scrolled && !msg_was_scrolled && row > 0 {
                    compose_area(row - 1, row, 0, Columns as Integer);
                }
            }
        }

        msg_current_row = row as c_int;
        msg_was_scrolled = scrolled;
    }
}

/// Check if `curgrid` is covered by any other grid within the rectangle (`curgrid_covered_above`).
unsafe fn curgrid_covered_above(top: c_int, bot: c_int, left: c_int, right: c_int) -> bool {
    unsafe {
        for i in ((*curgrid).comp_index + 1)..layers.size {
            let g = *layers.items.add(i);
            let grid_top = (*g).comp_row
                - if ptr::eq(g, &raw const msg_grid) && msg_was_scrolled {
                    1
                } else {
                    0
                };
            let grid_bot = (*g).comp_row + (*g).comp_height - 1;
            let grid_left = (*g).comp_col;
            let grid_right = (*g).comp_col + (*g).comp_width - 1;

            if right >= grid_left && left <= grid_right && bot >= grid_top && top <= grid_bot {
                return true;
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_grid_scroll(
    grid: Integer,
    mut top: Integer,
    mut bot: Integer,
    mut left: Integer,
    mut right: Integer,
    rows: Integer,
    cols: Integer,
) {
    unsafe {
        if !ui_comp_should_draw() || !ui_comp_set_grid(grid as c_int) {
            return;
        }
        top += (*curgrid).comp_row as Integer;
        bot += (*curgrid).comp_row as Integer;
        left += (*curgrid).comp_col as Integer;
        right += (*curgrid).comp_col as Integer;
        let covered =
            curgrid_covered_above(top as c_int, bot as c_int, left as c_int, right as c_int);

        if covered || (*curgrid).blending {
            compose_debug(top, bot, left, right, dbghl_recompose, true);
            let start_r = (top + (-rows).max(0)) as c_int;
            let end_r = (bot - rows.max(0)) as c_int;
            for r in start_r..end_r {
                let off = *(*curgrid)
                    .line_offset
                    .add((r - (*curgrid).comp_row) as usize)
                    + (left as usize)
                    - ((*curgrid).comp_col as usize);
                if *(*curgrid).attrs.add(off) >= 0 {
                    compose_line(r as Integer, left, right, 0);
                }
            }
        } else {
            ui_composed_call_grid_scroll(1, top, bot, left, right, rows, cols);
            if (rdb_flags & kOptRdbFlagCompositor) != 0 {
                debug_delay(2);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_comp_grid_resize(grid: Integer, width: Integer, height: Integer) {
    unsafe {
        if grid == 1 {
            ui_composed_call_grid_resize(1, width, height);
            chk_width = width as c_int;
            chk_height = height as c_int;
            let new_bufsize = width as usize;
            if bufsize != new_bufsize {
                xfree(linebuf.cast());
                xfree(attrbuf.cast());
                linebuf = xmalloc(new_bufsize * size_of::<schar_T>()).cast();
                attrbuf = xmalloc(new_bufsize * size_of::<sattr_T>()).cast();
                bufsize = new_bufsize;
            }
        }
    }
}
