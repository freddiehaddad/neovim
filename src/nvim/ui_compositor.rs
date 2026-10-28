//! Compositor: merge floating grids with the main grid for display in TUI and
//! non-multigrid UIs (`src/nvim/ui_compositor.c`).

use crate::api::private::validate::NvimString;
use crate::globals::*;
use crate::grid::{
    ScreenGrid, braille_space, kOptRdbFlagInvalid, sattr_T, schar_T, schar_from_ascii,
    schar_from_buf, schar_from_char, schar_invisible,
};
use crate::highlight::hl_blend_attrs;
use crate::highlight_group::{syn_check_group, syn_id2attr};
use crate::lua::converter::Boolean;
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::option_vars::*;
use crate::os::time::os_sleep;
use crate::ui::{
    kLineFlagInvalid, ui_call_flush, ui_composed_call_grid_cursor_goto,
    ui_composed_call_grid_resize, ui_composed_call_grid_scroll, ui_composed_call_raw_line, ui_has,
};
use std::ffi::{c_int, c_uint};
use std::ptr;

use crate::tui::tui::LineFlags;

const kOptRdbFlagCompositor: c_uint = 0x01;

const HLF_MSGSEP: usize = 61;

use crate::api::ui::kLineFlagWrap;
use crate::api::win_config::kUIMultigrid;
use crate::msgpack_rpc::channel::RemoteUI;

struct UiCompositorState {
    composed_uis: c_int,
    layers: Vec<*mut ScreenGrid>,
    linebuf: Vec<schar_T>,
    attrbuf: Vec<sattr_T>,
    chk_width: c_int,
    chk_height: c_int,
    curgrid: *mut ScreenGrid,
    valid_screen: bool,
    msg_current_row: c_int,
    msg_was_scrolled: bool,
    msg_sep_row: c_int,
    msg_sep_char: schar_T,
    dbghl_normal: c_int,
    dbghl_clear: c_int,
    dbghl_composed: c_int,
    dbghl_recompose: c_int,
}

impl UiCompositorState {
    const fn new() -> Self {
        Self {
            composed_uis: 0,
            layers: Vec::new(),
            linebuf: Vec::new(),
            attrbuf: Vec::new(),
            chk_width: 0,
            chk_height: 0,
            curgrid: ptr::null_mut(),
            valid_screen: true,
            msg_current_row: c_int::MAX,
            msg_was_scrolled: false,
            msg_sep_row: -1,
            msg_sep_char: schar_from_ascii(b' '),
            dbghl_normal: 0,
            dbghl_clear: 0,
            dbghl_composed: 0,
            dbghl_recompose: 0,
        }
    }
}

static COMP_STATE: GlobalState<UiCompositorState> = GlobalState::new(UiCompositorState::new());

#[inline]
fn comp() -> &'static mut UiCompositorState {
    unsafe { &mut *COMP_STATE.as_ptr() }
}

#[inline]
fn layers_mut() -> &'static mut Vec<*mut ScreenGrid> {
    unsafe { &mut (*COMP_STATE.as_ptr()).layers }
}

#[inline]
fn linebuf_mut() -> &'static mut Vec<schar_T> {
    unsafe { &mut (*COMP_STATE.as_ptr()).linebuf }
}

#[inline]
fn attrbuf_mut() -> &'static mut Vec<sattr_T> {
    unsafe { &mut (*COMP_STATE.as_ptr()).attrbuf }
}

pub fn ui_comp_init() {
    layers_mut().push(&raw mut g_st().default_grid);
        comp().curgrid = &raw mut g_st().default_grid;
}

pub fn ui_comp_free_all_mem() {
    comp().layers = Vec::new();
        comp().linebuf = Vec::new();
        comp().attrbuf = Vec::new();
}

pub fn ui_comp_syn_init() {
    comp().dbghl_normal = syn_check_group(c"RedrawDebugNormal".as_ptr(), 17);
        comp().dbghl_clear = syn_check_group(c"RedrawDebugClear".as_ptr(), 16);
        comp().dbghl_composed = syn_check_group(c"RedrawDebugComposed".as_ptr(), 19);
        comp().dbghl_recompose = syn_check_group(c"RedrawDebugRecompose".as_ptr(), 20);
}

pub fn ui_comp_attach(ui: &mut RemoteUI) {
    comp().composed_uis += 1;
    ui.composed = true;
    debug_assert_eq!(braille_space, schar_from_char(0x2800));
}

pub fn ui_comp_detach(ui: &mut RemoteUI) {
    comp().composed_uis -= 1;
        if comp().composed_uis == 0 {
            comp().linebuf = Vec::new();
            comp().attrbuf = Vec::new();
        }
        ui.composed = false;
}

pub fn ui_comp_should_draw() -> bool {
    comp().composed_uis != 0 && comp().valid_screen
}

/// Raises or lowers the layer, syncing `comp_index` with `zindex` (`ui_comp_layers_adjust`).
pub fn ui_comp_layers_adjust(mut layer_idx: usize, raise: bool) {
    unsafe {
        let l = layers_mut();
        let size = l.len();
        let layer = l[layer_idx];

        if raise {
            while layer_idx < size - 1 && (*layer).zindex > (*l[layer_idx + 1]).zindex {
                let next = l[layer_idx + 1];
                l[layer_idx] = next;
                (*next).comp_index = layer_idx;
                (*next).pending_comp_index_update = true;
                layer_idx += 1;
            }
        } else {
            while layer_idx > 0 && (*layer).zindex < (*l[layer_idx - 1]).zindex {
                let prev = l[layer_idx - 1];
                l[layer_idx] = prev;
                (*prev).comp_index = layer_idx;
                (*prev).pending_comp_index_update = true;
                layer_idx -= 1;
            }
        }
        l[layer_idx] = layer;
        (*layer).comp_index = layer_idx;
        (*layer).pending_comp_index_update = true;
    }
}

/// Places `grid` at `(col, row)` position with `(width * height)` size (`ui_comp_put_grid`).
pub fn ui_comp_put_grid(
    grid: &mut ScreenGrid,
    row: c_int,
    col: c_int,
    height: c_int,
    width: c_int,
    valid: bool,
    on_top: bool,
) -> bool {
    unsafe {
        let moved: bool;
        grid.pending_comp_index_update = true;

        if grid.comp_index != 0 {
            moved = (row != grid.comp_row) || (col != grid.comp_col);
            if ui_comp_should_draw() {
                grid.comp_disabled = true;
                compose_area(
                    grid.comp_row as Integer,
                    row as Integer,
                    grid.comp_col as Integer,
                    (grid.comp_col + grid.comp_width) as Integer,
                );
                if grid.comp_col < col {
                    compose_area(
                        row.max(grid.comp_row) as Integer,
                        (row + height).min(grid.comp_row + grid.comp_height) as Integer,
                        grid.comp_col as Integer,
                        col as Integer,
                    );
                }
                if col + width < grid.comp_col + grid.comp_width {
                    compose_area(
                        row.max(grid.comp_row) as Integer,
                        (row + height).min(grid.comp_row + grid.comp_height) as Integer,
                        (col + width) as Integer,
                        (grid.comp_col + grid.comp_width) as Integer,
                    );
                }
                compose_area(
                    (row + height) as Integer,
                    (grid.comp_row + grid.comp_height) as Integer,
                    grid.comp_col as Integer,
                    (grid.comp_col + grid.comp_width) as Integer,
                );
                grid.comp_disabled = false;
            }
            grid.comp_row = row;
            grid.comp_col = col;
        } else {
            moved = true;
            let l = layers_mut();
            let grid_ptr = &raw mut *grid;
            debug_assert!(!l.contains(&grid_ptr));

            let mut insert_at = l.len();
            while insert_at > 0 && (*l[insert_at - 1]).zindex > grid.zindex {
                insert_at -= 1;
            }

            if !(*curwin.as_ptr()).is_null()
                && l[insert_at - 1] == &raw mut (**curwin.as_ptr()).w_grid_alloc
                && (*l[insert_at - 1]).zindex == grid.zindex
                && !on_top
            {
                insert_at -= 1;
            }
            l.insert(insert_at, grid);
            for (idx, &g) in l.iter().enumerate().skip(insert_at + 1) {
                (*g).comp_index = idx;
                (*g).pending_comp_index_update = true;
            }

            grid.comp_row = row;
            grid.comp_col = col;
            grid.comp_index = insert_at;
            grid.pending_comp_index_update = true;
        }

        grid.comp_height = height;
        grid.comp_width = width;
        if moved && valid && ui_comp_should_draw() {
            compose_area(
                grid.comp_row as Integer,
                (grid.comp_row + grid.rows) as Integer,
                grid.comp_col as Integer,
                (grid.comp_col + grid.cols) as Integer,
            );
        }
        moved
    }
}

pub fn ui_comp_remove_grid(grid: &mut ScreenGrid) {
    unsafe {
        debug_assert!(!ptr::eq(grid, &raw const g_st().default_grid));
        if grid.comp_index == 0 {
            return;
        }

        if comp().curgrid == grid {
            comp().curgrid = &raw mut g_st().default_grid;
        }

        let l = layers_mut();
        let removed_idx = grid.comp_index;
        l.remove(removed_idx);
        for (idx, &g) in l.iter().enumerate().skip(removed_idx) {
            (*g).comp_index = idx;
            (*g).pending_comp_index_update = true;
        }
        grid.comp_index = 0;
        grid.pending_comp_index_update = true;

        ui_comp_compose_grid(grid);
    }
}

pub fn ui_comp_set_grid(handle: handle_T) -> bool {
    unsafe {
        if (*comp().curgrid).handle == handle {
            return true;
        }
        for &g in layers_mut().iter() {
            if (*g).handle == handle {
                comp().curgrid = g;
                return true;
            }
        }
        false
    }
}

pub fn ui_comp_raise_grid(grid: &mut ScreenGrid, new_index: usize) {
    unsafe {
        let l = layers_mut();
        let old_index = grid.comp_index;
        for i in old_index..new_index {
            let next = l[i + 1];
            l[i] = next;
            (*next).comp_index = i;
            (*next).pending_comp_index_update = true;
        }
        l[new_index] = grid;
        grid.comp_index = new_index;
        grid.pending_comp_index_update = true;
        for &grid2 in l.iter().take(new_index).skip(old_index) {
            let startcol = grid.comp_col.max((*grid2).comp_col);
            let endcol = (grid.comp_col + grid.cols).min((*grid2).comp_col + (*grid2).cols);
            compose_area(
                grid.comp_row.max((*grid2).comp_row) as Integer,
                (grid.comp_row + grid.rows).min((*grid2).comp_row + (*grid2).rows) as Integer,
                startcol as Integer,
                endcol as Integer,
            );
        }
    }
}

pub fn ui_comp_grid_cursor_goto(grid_handle: Integer, r: Integer, c: Integer) {
    unsafe {
        if !ui_comp_set_grid(grid_handle as c_int) {
            return;
        }
        let cursor_row = (*comp().curgrid).comp_row + (r as c_int);
        let cursor_col = (*comp().curgrid).comp_col + (c as c_int);

        if !ptr::eq(comp().curgrid, &raw const g_st().default_grid) {
            let l = layers_mut();
            let mut new_index = l.len() - 1;
            while new_index > 1 && (*l[new_index]).zindex > (*comp().curgrid).zindex {
                new_index -= 1;
            }
            if (*comp().curgrid).comp_index < new_index {
                ui_comp_raise_grid(&mut *comp().curgrid, new_index);
            }
        }

        if cursor_col >= g_st().default_grid.cols || cursor_row >= g_st().default_grid.rows {
            return;
        }
        ui_composed_call_grid_cursor_goto(1, cursor_row as Integer, cursor_col as Integer);
    }
}

pub fn ui_comp_mouse_focus(row: c_int, col: c_int) -> *mut ScreenGrid {
    unsafe {
        let l = layers_mut();
        for &grid in l.iter().skip(1).rev() {
            if (*grid).mouse_enabled
                && row >= (*grid).comp_row
                && row < (*grid).comp_row + (*grid).rows
                && col >= (*grid).comp_col
                && col < (*grid).comp_col + (*grid).cols
            {
                return grid;
            }
        }
        if ui_has(kUIMultigrid) {
            let mut wp = g_st().firstwin;
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
pub fn ui_comp_get_grid_at_coord(row: c_int, col: c_int) -> *mut ScreenGrid {
    unsafe {
        let l = layers_mut();
        for &grid in l.iter().skip(1).rev() {
            if row >= (*grid).comp_row
                && row < (*grid).comp_row + (*grid).rows
                && col >= (*grid).comp_col
                && col < (*grid).comp_col + (*grid).cols
            {
                return grid;
            }
        }

        let mut wp = g_st().firstwin;
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
        &raw mut g_st().default_grid
    }
}

fn compose_line(
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
        if endcol < (g_st().default_grid.cols as Integer) && (flags & kLineFlagInvalid) != 0 {
            endcol += 1;
            skipend = 1;
        }

        let mut col = startcol as c_int;
        let mut grid: *mut ScreenGrid = ptr::null_mut();
        let line_off = g_st().default_grid.line_offset[row as usize] + (startcol as usize);
        let bg_line = &raw mut g_st().default_grid.chars[line_off];
        let bg_attrs = &raw mut g_st().default_grid.attrs[line_off];
        let lbuf = linebuf_mut().as_mut_ptr();
        let abuf = attrbuf_mut().as_mut_ptr();
        let l = layers_mut();

        while (col as Integer) < endcol {
            let mut until: c_int = 0;
            for &g in l.iter() {
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
            debug_assert!(until <= g_st().default_grid.cols);
            let n = (until - col) as usize;

            if row == (comp().msg_sep_row as Integer) && (*grid).comp_index <= g_st().msg_grid.comp_index {
                grid = &raw mut g_st().msg_grid;
                let msg_sep_attr = crate::highlight::hl_attr(HLF_MSGSEP as c_int) as sattr_T;
                for i in col..until {
                    let idx = (i - startcol as c_int) as usize;
                    *lbuf.add(idx) = comp().msg_sep_char;
                    *abuf.add(idx) = msg_sep_attr;
                }
            } else {
                let g = &*grid;
                let off = g.line_offset[(row as c_int - g.comp_row) as usize]
                    + (col - g.comp_col) as usize;
                let dst_idx = (col - startcol as c_int) as usize;
                ptr::copy_nonoverlapping(g.chars.as_ptr().add(off), lbuf.add(dst_idx), n);
                ptr::copy_nonoverlapping(g.attrs.as_ptr().add(off), abuf.add(dst_idx), n);
                if g.comp_col + g.cols > until && g.chars[off + n] == 0 {
                    *lbuf.add((until - 1 - startcol as c_int) as usize) = schar_from_ascii(b' ');
                    if (col as Integer) == startcol && n == 1 {
                        skipstart = 0;
                    }
                }
            }

            if (*grid).blending {
                let mut i = col - (startcol as c_int);
                let limit = until - (startcol as c_int);
                let end_rel = (endcol - startcol) as c_int;
                while i < limit {
                    let mut width: c_int = 1;
                    let idx = i as usize;
                    let mut thru = schar_invisible(*lbuf.add(idx)) && *bg_line.add(idx) != 0;
                    if i + 1 < end_rel && *bg_line.add(idx + 1) == 0 {
                        width = 2;
                        thru &= schar_invisible(*lbuf.add(idx + 1));
                    }
                    *abuf.add(idx) =
                        hl_blend_attrs(*bg_attrs.add(idx), *abuf.add(idx), &mut thru)
                            as sattr_T;
                    if width == 2 {
                        *abuf.add(idx + 1) = hl_blend_attrs(
                            *bg_attrs.add(idx + 1),
                            *abuf.add(idx + 1),
                            &mut thru,
                        ) as sattr_T;
                    }
                    if thru {
                        ptr::copy_nonoverlapping(bg_line.add(idx), lbuf.add(idx), width as usize);
                    }
                    i += width;
                }
            }

            let col_idx = (col - startcol as c_int) as usize;
            if *lbuf.add(col_idx) == 0 {
                *lbuf.add(col_idx) = schar_from_ascii(b' ');
                if (col as Integer) == endcol - 1 {
                    skipend = 0;
                }
            } else if (col as Integer) == startcol && n > 1 && *lbuf.add(1) == 0 {
                skipstart = 0;
            }

            col = until;
        }
        if *lbuf.add((endcol - startcol - 1) as usize) == 0 {
            skipend = 0;
        }

        debug_assert!(endcol <= comp().chk_width as Integer);
        debug_assert!(row < comp().chk_height as Integer);

        if !(!grid.is_null()
            && (ptr::eq(grid, &raw const g_st().default_grid)
                || ((*grid).comp_col == 0 && (*grid).cols == g_st().Columns)))
        {
            flags &= !kLineFlagWrap;
        }

        let end_idx = ((endcol - skipend as Integer) - startcol) as c_int;
        for i in skipstart..end_idx {
            let p = abuf.add(i as usize);
            if *p < 0 {
                if (opt_vars().rdb_flags & kOptRdbFlagInvalid) != 0 {
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
            lbuf.add(skipstart as usize),
            abuf.add(skipstart as usize),
        );
    }
}

fn compose_debug(
    startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    mut endcol: Integer,
    syn_id: c_int,
    delay: bool,
) {
    if (opt_vars().rdb_flags & kOptRdbFlagCompositor) == 0 || startcol >= endcol {
            return;
        }

        endrow = endrow.min(g_st().default_grid.rows as Integer);
        endcol = endcol.min(g_st().default_grid.cols as Integer);
        let attr = syn_id2attr(syn_id);

        if delay {
            debug_delay(endrow - startrow);
        }

        let lbuf = linebuf_mut().as_ptr();
        let abuf = attrbuf_mut().as_ptr();
        for row in (startrow as c_int)..(endrow as c_int) {
            ui_composed_call_raw_line(
                1,
                row as Integer,
                startcol,
                startcol,
                endcol,
                attr as Integer,
                0,
                lbuf,
                abuf,
            );
        }

        if delay {
            debug_delay(endrow - startrow);
        }
}

fn debug_delay(lines: Integer) {
    ui_call_flush();
        let wd = opt_vars().p_wd.unsigned_abs();
        let factor = lines.clamp(1, 5) as u64;
        os_sleep(factor * wd);
}

fn compose_area(
    startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    mut endcol: Integer,
) {
    compose_debug(startrow, endrow, startcol, endcol, comp().dbghl_recompose, true);
        endrow = endrow.min(g_st().default_grid.rows as Integer);
        endcol = endcol.min(g_st().default_grid.cols as Integer);
        if endcol <= startcol {
            return;
        }
        for r in (startrow as c_int)..(endrow as c_int) {
            compose_line(r as Integer, startcol, endcol, kLineFlagInvalid);
        }
}

/// Compose the area under the grid (`ui_comp_compose_grid`).
pub fn ui_comp_compose_grid(grid: &mut ScreenGrid) {
    if ui_comp_should_draw() {
            compose_area(
                grid.comp_row as Integer,
                (grid.comp_row + grid.rows) as Integer,
                grid.comp_col as Integer,
                (grid.comp_col + grid.cols) as Integer,
            );
        }
}

pub fn ui_comp_raw_line(
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

        row += (*comp().curgrid).comp_row as Integer;
        startcol += (*comp().curgrid).comp_col as Integer;
        endcol += (*comp().curgrid).comp_col as Integer;
        clearcol += (*comp().curgrid).comp_col as Integer;
        if !ptr::eq(comp().curgrid, &raw const g_st().default_grid) {
            flags &= !kLineFlagWrap;
        }

        debug_assert!(endcol <= clearcol);

        if row >= g_st().default_grid.rows as Integer {
            return;
        }
        if clearcol > g_st().default_grid.cols as Integer {
            if startcol >= g_st().default_grid.cols as Integer {
                return;
            }
            clearcol = g_st().default_grid.cols as Integer;
            endcol = endcol.min(clearcol);
        }

        let covered = curgrid_covered_above(
            row as c_int,
            row as c_int,
            startcol as c_int,
            clearcol as c_int,
        );
        if (flags & kLineFlagInvalid) != 0 || covered || (*comp().curgrid).blending {
            compose_debug(row, row + 1, startcol, clearcol, comp().dbghl_composed, true);
            compose_line(row, startcol, clearcol, flags);
        } else {
            compose_debug(
                row,
                row + 1,
                startcol,
                endcol,
                comp().dbghl_normal,
                endcol >= clearcol,
            );
            compose_debug(row, row + 1, endcol, clearcol, comp().dbghl_clear, true);
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
pub fn ui_comp_set_screen_valid(valid: bool) -> bool {
    let old_val = comp().valid_screen;
        comp().valid_screen = valid;
        if !valid {
            comp().msg_sep_row = -1;
        }
        old_val
}

pub fn ui_comp_msg_set_pos(
    _grid: Integer,
    row: Integer,
    scrolled: Boolean,
    sep_char: NvimString,
    _zindex: Integer,
    _compindex: Integer,
) {
    g_st().msg_grid.pending_comp_index_update = true;
        g_st().msg_grid.comp_row = row as c_int;
        if scrolled && row > 0 {
            comp().msg_sep_row = (row as c_int) - 1;
            if !sep_char.data.is_null() {
                comp().msg_sep_char = schar_from_buf(sep_char.data, sep_char.size);
            }
        } else {
            comp().msg_sep_row = -1;
        }

        if row > (comp().msg_current_row as Integer) && ui_comp_should_draw() {
            compose_area(
                (comp().msg_current_row - 1).max(0) as Integer,
                row,
                0,
                g_st().default_grid.cols as Integer,
            );
        } else if row < (comp().msg_current_row as Integer)
            && ui_comp_should_draw()
            && (comp().msg_current_row < g_st().Rows || (scrolled && !comp().msg_was_scrolled))
        {
            let delta = comp().msg_current_row - (row as c_int);
            if g_st().msg_grid.blending {
                let first_row = ((row as c_int) - if scrolled { 1 } else { 0 }).max(0);
                compose_area(
                    first_row as Integer,
                    (g_st().Rows - delta) as Integer,
                    0,
                    g_st().Columns as Integer,
                );
            } else {
                let first_row = ((row as c_int) - if comp().msg_was_scrolled { 1 } else { 0 }).max(0);
                ui_composed_call_grid_scroll(
                    1,
                    first_row as Integer,
                    g_st().Rows as Integer,
                    0,
                    g_st().Columns as Integer,
                    delta as Integer,
                    0,
                );
                if scrolled && !comp().msg_was_scrolled && row > 0 {
                    compose_area(row - 1, row, 0, g_st().Columns as Integer);
                }
            }
        }

        comp().msg_current_row = row as c_int;
        comp().msg_was_scrolled = scrolled;
}

/// Check if `comp().curgrid` is covered by any other grid within the rectangle (`curgrid_covered_above`).
fn curgrid_covered_above(top: c_int, bot: c_int, left: c_int, right: c_int) -> bool {
    unsafe {
        for &g in layers_mut().iter().skip((*comp().curgrid).comp_index + 1) {
            let grid_top = (*g).comp_row
                - if ptr::eq(g, &raw const g_st().msg_grid) && comp().msg_was_scrolled {
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

pub fn ui_comp_grid_scroll(
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
        top += (*comp().curgrid).comp_row as Integer;
        bot += (*comp().curgrid).comp_row as Integer;
        left += (*comp().curgrid).comp_col as Integer;
        right += (*comp().curgrid).comp_col as Integer;
        let covered =
            curgrid_covered_above(top as c_int, bot as c_int, left as c_int, right as c_int);

        if covered || (*comp().curgrid).blending {
            compose_debug(top, bot, left, right, comp().dbghl_recompose, true);
            let start_r = (top + (-rows).max(0)) as c_int;
            let end_r = (bot - rows.max(0)) as c_int;
            for r in start_r..end_r {
                let cg = &*comp().curgrid;
                let off = cg.line_offset[(r - cg.comp_row) as usize] + (left as usize)
                    - (cg.comp_col as usize);
                if cg.attrs[off] >= 0 {
                    compose_line(r as Integer, left, right, 0);
                }
            }
        } else {
            ui_composed_call_grid_scroll(1, top, bot, left, right, rows, cols);
            if (opt_vars().rdb_flags & kOptRdbFlagCompositor) != 0 {
                debug_delay(2);
            }
        }
    }
}

pub fn ui_comp_grid_resize(grid: Integer, width: Integer, height: Integer) {
    if grid == 1 {
            ui_composed_call_grid_resize(1, width, height);
            comp().chk_width = width as c_int;
            comp().chk_height = height as c_int;
            let new_bufsize = width as usize;
            let lbuf = linebuf_mut();
            let abuf = attrbuf_mut();
            if lbuf.len() != new_bufsize {
                lbuf.resize(new_bufsize, 0);
                abuf.resize(new_bufsize, 0);
            }
        }
}
