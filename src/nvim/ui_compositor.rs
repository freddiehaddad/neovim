//! Compositor: merge floating grids with the main grid for display in TUI and
//! non-multigrid UIs (`src/nvim/ui_compositor.c`).
#![forbid(unsafe_code)]

use crate::api::private::validate::NvimString;
use crate::globals::*;
use crate::grid::{
    BRAILLE_SPACE, K_OPT_RDB_FLAG_INVALID, Sattr, Schar, ScreenGrid, schar_from_ascii,
    schar_from_buf, schar_from_char, schar_invisible,
};
use crate::highlight::hl_blend_attrs;
use crate::highlight_group::{syn_check_group, syn_id2attr};
use crate::lua::converter::Boolean;
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::option_vars::*;
use crate::os::time::os_sleep;
use crate::ui::{
    K_LINE_FLAG_INVALID, ui_call_flush, ui_composed_call_grid_cursor_goto,
    ui_composed_call_grid_resize, ui_composed_call_grid_scroll, ui_composed_call_raw_line, ui_has,
};

use std::ptr;

use crate::tui::tui::LineFlags;

const K_OPT_RDB_FLAG_COMPOSITOR: u32 = 0x01;

const HLF_MSGSEP: usize = 61;

use crate::api::ui::K_LINE_FLAG_WRAP;
use crate::api::win_config::K_UIMULTIGRID;
use crate::msgpack_rpc::channel::RemoteUI;

struct UiCompositorState {
    composed_uis: std::cell::Cell<i32>,
    layers: std::cell::RefCell<Vec<crate::globals::GraphPtr<ScreenGrid>>>,
    linebuf: std::cell::RefCell<Vec<Schar>>,
    attrbuf: std::cell::RefCell<Vec<Sattr>>,
    chk_width: std::cell::Cell<i32>,
    chk_height: std::cell::Cell<i32>,
    curgrid: std::cell::Cell<crate::globals::GraphPtr<ScreenGrid>>,
    valid_screen: std::cell::Cell<bool>,
    msg_current_row: std::cell::Cell<i32>,
    msg_was_scrolled: std::cell::Cell<bool>,
    msg_sep_row: std::cell::Cell<i32>,
    msg_sep_char: std::cell::Cell<Schar>,
    dbghl_normal: std::cell::Cell<i32>,
    dbghl_clear: std::cell::Cell<i32>,
    dbghl_composed: std::cell::Cell<i32>,
    dbghl_recompose: std::cell::Cell<i32>,
}

impl UiCompositorState {
    #[inline]
    fn curgrid_mut(&self) -> &'static mut ScreenGrid {
        self.curgrid.get().borrow_mut()
    }

    const fn new() -> Self {
        Self {
            composed_uis: std::cell::Cell::new(0),
            layers: std::cell::RefCell::new(Vec::new()),
            linebuf: std::cell::RefCell::new(Vec::new()),
            attrbuf: std::cell::RefCell::new(Vec::new()),
            chk_width: std::cell::Cell::new(0),
            chk_height: std::cell::Cell::new(0),
            curgrid: std::cell::Cell::new(crate::globals::GraphPtr::NULL),
            valid_screen: std::cell::Cell::new(true),
            msg_current_row: std::cell::Cell::new(i32::MAX),
            msg_was_scrolled: std::cell::Cell::new(false),
            msg_sep_row: std::cell::Cell::new(-1),
            msg_sep_char: std::cell::Cell::new(schar_from_ascii(b' ')),
            dbghl_normal: std::cell::Cell::new(0),
            dbghl_clear: std::cell::Cell::new(0),
            dbghl_composed: std::cell::Cell::new(0),
            dbghl_recompose: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static COMP_STATE: UiCompositorState = const { UiCompositorState::new() };
}

#[inline]
fn with_comp<R>(f: impl FnOnce(&UiCompositorState) -> R) -> R {
    COMP_STATE.with(f)
}

#[inline]
fn layer_mut(idx: usize) -> &'static mut ScreenGrid {
    with_comp(|c| c.layers.borrow()[idx]).borrow_mut()
}

#[inline]
fn layers_len() -> usize {
    with_comp(|c| c.layers.borrow().len())
}

pub fn ui_comp_init() {
    with_comp(|c| {
        c.layers
            .borrow_mut()
            .push((&mut g_st().default_grid).into());
        c.curgrid.set((&mut g_st().default_grid).into());
    });
}

pub fn ui_comp_free_all_mem() {
    with_comp(|c| {
        *c.layers.borrow_mut() = Vec::new();
        *c.linebuf.borrow_mut() = Vec::new();
        *c.attrbuf.borrow_mut() = Vec::new();
    });
}

pub fn ui_comp_syn_init() {
    let normal = syn_check_group(b"RedrawDebugNormal");
    let clear = syn_check_group(b"RedrawDebugClear");
    let composed = syn_check_group(b"RedrawDebugComposed");
    let recompose = syn_check_group(b"RedrawDebugRecompose");
    with_comp(|c| {
        c.dbghl_normal.set(normal);
        c.dbghl_clear.set(clear);
        c.dbghl_composed.set(composed);
        c.dbghl_recompose.set(recompose);
    });
}

pub fn ui_comp_attach(ui: &mut RemoteUI) {
    with_comp(|c| c.composed_uis.set(c.composed_uis.get() + 1));
    ui.composed = true;
    debug_assert_eq!(BRAILLE_SPACE, schar_from_char(0x2800));
}

pub fn ui_comp_detach(ui: &mut RemoteUI) {
    with_comp(|c| {
        let count = c.composed_uis.get() - 1;
        c.composed_uis.set(count);
        if count == 0 {
            *c.linebuf.borrow_mut() = Vec::new();
            *c.attrbuf.borrow_mut() = Vec::new();
        }
    });
    ui.composed = false;
}

pub fn ui_comp_should_draw() -> bool {
    with_comp(|c| c.composed_uis.get() != 0 && c.valid_screen.get())
}

/// Raises or lowers the layer, syncing `comp_index` with `zindex` (`ui_comp_layers_adjust`).
pub fn ui_comp_layers_adjust(mut layer_idx: usize, raise: bool) {
    let size = layers_len();
    let layer = layer_mut(layer_idx);

    if raise {
        while layer_idx < size - 1 && layer.zindex > layer_mut(layer_idx + 1).zindex {
            let next = layer_mut(layer_idx + 1);
            with_comp(|c| c.layers.borrow_mut()[layer_idx] = next.into());
            next.comp_index = layer_idx;
            next.pending_comp_index_update = true;
            layer_idx += 1;
        }
    } else {
        while layer_idx > 0 && layer.zindex < layer_mut(layer_idx - 1).zindex {
            let prev = layer_mut(layer_idx - 1);
            with_comp(|c| c.layers.borrow_mut()[layer_idx] = prev.into());
            prev.comp_index = layer_idx;
            prev.pending_comp_index_update = true;
            layer_idx -= 1;
        }
    }
    with_comp(|c| c.layers.borrow_mut()[layer_idx] = layer.into());
    layer.comp_index = layer_idx;
    layer.pending_comp_index_update = true;
}

/// Places `grid` at `(col, row)` position with `(width * height)` size (`ui_comp_put_grid`).
pub fn ui_comp_put_grid(
    grid: &mut ScreenGrid,
    row: i32,
    col: i32,
    height: i32,
    width: i32,
    valid: bool,
    on_top: bool,
) -> bool {
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
        let grid_ptr = &raw mut *grid;
        debug_assert!(!with_comp(|c| c.layers.borrow().contains(&grid_ptr.into())));

        let mut insert_at = layers_len();
        while insert_at > 0 && layer_mut(insert_at - 1).zindex > grid.zindex {
            insert_at -= 1;
        }

        if !(curwin_ptr()).is_null()
            && with_comp(|c| c.layers.borrow()[insert_at - 1] == &curwin_mut().w_grid_alloc)
            && layer_mut(insert_at - 1).zindex == grid.zindex
            && !on_top
        {
            insert_at -= 1;
        }
        let len = with_comp(|c| {
            let mut layers = c.layers.borrow_mut();
            layers.insert(insert_at, grid.into());
            layers.len()
        });
        for idx in (insert_at + 1)..len {
            let g = layer_mut(idx);
            g.comp_index = idx;
            g.pending_comp_index_update = true;
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

pub fn ui_comp_remove_grid(grid: &mut ScreenGrid) {
    debug_assert!(!ptr::eq(grid, &raw const g_st().default_grid));
    if grid.comp_index == 0 {
        return;
    }

    with_comp(|c| {
        if c.curgrid.get() == grid {
            c.curgrid.set((&mut g_st().default_grid).into());
        }
    });

    let removed_idx = grid.comp_index;
    let len = with_comp(|c| {
        let mut layers = c.layers.borrow_mut();
        layers.remove(removed_idx);
        layers.len()
    });
    for idx in removed_idx..len {
        let g = layer_mut(idx);
        g.comp_index = idx;
        g.pending_comp_index_update = true;
    }
    grid.comp_index = 0;
    grid.pending_comp_index_update = true;

    ui_comp_compose_grid(grid);
}

pub fn ui_comp_set_grid(handle: ObjHandle) -> bool {
    if with_comp(|c| c.curgrid_mut().handle == handle) {
        return true;
    }
    for idx in 0..layers_len() {
        let g = layer_mut(idx);
        if g.handle == handle {
            with_comp(|c| c.curgrid.set(g.into()));
            return true;
        }
    }
    false
}

pub fn ui_comp_raise_grid(grid: &mut ScreenGrid, new_index: usize) {
    let old_index = grid.comp_index;
    for i in old_index..new_index {
        let next = layer_mut(i + 1);
        with_comp(|c| c.layers.borrow_mut()[i] = next.into());
        next.comp_index = i;
        next.pending_comp_index_update = true;
    }
    with_comp(|c| c.layers.borrow_mut()[new_index] = grid.into());
    grid.comp_index = new_index;
    grid.pending_comp_index_update = true;
    for idx in old_index..new_index {
        let grid2 = layer_mut(idx);
        let startcol = grid.comp_col.max(grid2.comp_col);
        let endcol = (grid.comp_col + grid.cols).min(grid2.comp_col + grid2.cols);
        compose_area(
            grid.comp_row.max(grid2.comp_row) as Integer,
            (grid.comp_row + grid.rows).min(grid2.comp_row + grid2.rows) as Integer,
            startcol as Integer,
            endcol as Integer,
        );
    }
}

pub fn ui_comp_grid_cursor_goto(grid_handle: Integer, r: Integer, c: Integer) {
    if !ui_comp_set_grid(grid_handle as i32) {
        return;
    }
    let curgrid = with_comp(UiCompositorState::curgrid_mut);
    let cursor_row = curgrid.comp_row + (r as i32);
    let cursor_col = curgrid.comp_col + (c as i32);

    if with_comp(|c| c.curgrid.get() != &g_st().default_grid) {
        let mut new_index = layers_len() - 1;
        while new_index > 1 && layer_mut(new_index).zindex > curgrid.zindex {
            new_index -= 1;
        }
        if curgrid.comp_index < new_index {
            ui_comp_raise_grid(curgrid, new_index);
        }
    }

    if cursor_col >= g_st().default_grid.cols || cursor_row >= g_st().default_grid.rows {
        return;
    }
    ui_composed_call_grid_cursor_goto(1, cursor_row as Integer, cursor_col as Integer);
}

pub fn ui_comp_mouse_focus(row: i32, col: i32) -> Option<&'static mut ScreenGrid> {
    for idx in (1..layers_len()).rev() {
        let g = layer_mut(idx);
        if g.mouse_enabled
            && row >= g.comp_row
            && row < g.comp_row + g.rows
            && col >= g.comp_col
            && col < g.comp_col + g.cols
        {
            return Some(g);
        }
    }
    if ui_has(K_UIMULTIGRID) {
        for wp in for_all_windows_mut() {
            let grid = &mut wp.w_grid_alloc;
            if grid.mouse_enabled
                && row >= wp.w_winrow
                && row < wp.w_winrow + grid.rows
                && col >= wp.w_wincol
                && col < wp.w_wincol + grid.cols
            {
                return Some(grid);
            }
        }
    }
    None
}

/// Compute which grid is on top at supplied screen coordinates (`ui_comp_get_grid_at_coord`).
pub fn ui_comp_get_grid_at_coord(row: i32, col: i32) -> &'static mut ScreenGrid {
    for idx in (1..layers_len()).rev() {
        let g = layer_mut(idx);
        if row >= g.comp_row
            && row < g.comp_row + g.rows
            && col >= g.comp_col
            && col < g.comp_col + g.cols
        {
            return g;
        }
    }

    for wp in for_all_windows_mut() {
        let grid = &mut wp.w_grid_alloc;
        if row >= grid.comp_row
            && row < grid.comp_row + grid.rows
            && col >= grid.comp_col
            && col < grid.comp_col + grid.cols
            && !wp.w_config.hide
        {
            return grid;
        }
    }
    &mut g_st().borrow_mut().default_grid
}

fn compose_line(row: Integer, mut startcol: Integer, mut endcol: Integer, mut flags: LineFlags) {
    startcol = startcol.max(0);
    let mut skipstart: i32 = 0;
    let mut skipend: i32 = 0;
    if startcol > 0 && (flags & K_LINE_FLAG_INVALID) != 0 {
        startcol -= 1;
        skipstart = 1;
    }
    if endcol < (g_st().default_grid.cols as Integer) && (flags & K_LINE_FLAG_INVALID) != 0 {
        endcol += 1;
        skipend = 1;
    }

    let mut col = startcol as i32;
    let mut grid: Option<&'static ScreenGrid> = None;
    let line_off = g_st().default_grid.line_offset[row as usize] + (startcol as usize);
    let bg_line = &g_st().default_grid.chars[line_off..];
    let bg_attrs = &g_st().default_grid.attrs[line_off..];
    let (msg_sep_row, msg_sep_char, chk_width, chk_height) = with_comp(|c| {
        (
            c.msg_sep_row.get(),
            c.msg_sep_char.get(),
            c.chk_width.get(),
            c.chk_height.get(),
        )
    });

    with_comp(|c| {
        let mut linebuf = c.linebuf.borrow_mut();
        let mut attrbuf = c.attrbuf.borrow_mut();
        let lbuf = &mut linebuf[..];
        let abuf = &mut attrbuf[..];

        while (col as Integer) < endcol {
            let mut until: i32 = 0;
            for idx in 0..c.layers.borrow().len() {
                let g = layer_mut(idx);
                let grid_width = g.cols.min(g.comp_width);
                let grid_height = g.rows.min(g.comp_height);
                if (g.comp_row as Integer) > row
                    || row >= (g.comp_row + grid_height) as Integer
                    || g.comp_disabled
                {
                    continue;
                }
                if g.comp_col <= col && col < g.comp_col + grid_width {
                    grid = Some(g);
                    until = g.comp_col + grid_width;
                } else if g.comp_col > col {
                    until = until.min(g.comp_col);
                }
            }
            until = until.min(endcol as i32);

            debug_assert!(grid.is_some());
            debug_assert!(until > col);
            debug_assert!(until <= g_st().default_grid.cols);
            let n = (until - col) as usize;

            if row == (msg_sep_row as Integer)
                && grid.unwrap().comp_index <= g_st().msg_grid.comp_index
            {
                grid = Some(&g_st().borrow_mut().msg_grid);
                let msg_sep_attr = crate::highlight::hl_attr(HLF_MSGSEP as i32) as Sattr;
                for i in col..until {
                    let idx = (i - startcol as i32) as usize;
                    lbuf[idx] = msg_sep_char;
                    abuf[idx] = msg_sep_attr;
                }
            } else {
                let g = grid.unwrap();
                let off =
                    g.line_offset[(row as i32 - g.comp_row) as usize] + (col - g.comp_col) as usize;
                let dst_idx = (col - startcol as i32) as usize;
                lbuf[dst_idx..dst_idx + n].copy_from_slice(&g.chars[off..off + n]);
                abuf[dst_idx..dst_idx + n].copy_from_slice(&g.attrs[off..off + n]);
                if g.comp_col + g.cols > until && g.chars[off + n] == 0 {
                    lbuf[(until - 1 - startcol as i32) as usize] = schar_from_ascii(b' ');
                    if (col as Integer) == startcol && n == 1 {
                        skipstart = 0;
                    }
                }
            }

            if grid.unwrap().blending {
                let mut i = col - (startcol as i32);
                let limit = until - (startcol as i32);
                let end_rel = (endcol - startcol) as i32;
                while i < limit {
                    let mut width: i32 = 1;
                    let idx = i as usize;
                    let mut thru = schar_invisible(lbuf[idx]) && bg_line[idx] != 0;
                    if i + 1 < end_rel && bg_line[idx + 1] == 0 {
                        width = 2;
                        thru &= schar_invisible(lbuf[idx + 1]);
                    }
                    abuf[idx] = hl_blend_attrs(bg_attrs[idx], abuf[idx], &mut thru) as Sattr;
                    if width == 2 {
                        abuf[idx + 1] =
                            hl_blend_attrs(bg_attrs[idx + 1], abuf[idx + 1], &mut thru) as Sattr;
                    }
                    if thru {
                        let w = width as usize;
                        lbuf[idx..idx + w].copy_from_slice(&bg_line[idx..idx + w]);
                    }
                    i += width;
                }
            }

            let col_idx = (col - startcol as i32) as usize;
            if lbuf[col_idx] == 0 {
                lbuf[col_idx] = schar_from_ascii(b' ');
                if (col as Integer) == endcol - 1 {
                    skipend = 0;
                }
            } else if (col as Integer) == startcol && n > 1 && lbuf[1] == 0 {
                skipstart = 0;
            }

            col = until;
        }
        if lbuf[(endcol - startcol - 1) as usize] == 0 {
            skipend = 0;
        }

        debug_assert!(endcol <= chk_width as Integer);
        debug_assert!(row < chk_height as Integer);

        if !grid.is_some_and(|g| {
            ptr::eq(g, &raw const g_st().default_grid)
                || (g.comp_col == 0 && g.cols == g_st().columns_global)
        }) {
            flags &= !K_LINE_FLAG_WRAP;
        }

        let end_idx = ((endcol - skipend as Integer) - startcol) as i32;
        for i in skipstart..end_idx {
            let p = &mut abuf[i as usize];
            if *p < 0 {
                if (opt_vars().rdb_flags & K_OPT_RDB_FLAG_INVALID) != 0 {
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
            &lbuf[skipstart as usize..],
            &abuf[skipstart as usize..],
        );
    });
}

fn compose_debug(
    startrow: Integer,
    mut endrow: Integer,
    startcol: Integer,
    mut endcol: Integer,
    syn_id: i32,
    delay: bool,
) {
    if (opt_vars().rdb_flags & K_OPT_RDB_FLAG_COMPOSITOR) == 0 || startcol >= endcol {
        return;
    }

    endrow = endrow.min(g_st().default_grid.rows as Integer);
    endcol = endcol.min(g_st().default_grid.cols as Integer);
    let attr = syn_id2attr(syn_id);

    if delay {
        debug_delay(endrow - startrow);
    }

    with_comp(|c| {
        let linebuf = c.linebuf.borrow();
        let attrbuf = c.attrbuf.borrow();
        let lbuf = linebuf.as_slice();
        let abuf = attrbuf.as_slice();
        for row in (startrow as i32)..(endrow as i32) {
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
    });

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

fn compose_area(startrow: Integer, mut endrow: Integer, startcol: Integer, mut endcol: Integer) {
    compose_debug(
        startrow,
        endrow,
        startcol,
        endcol,
        with_comp(|c| c.dbghl_recompose.get()),
        true,
    );
    endrow = endrow.min(g_st().default_grid.rows as Integer);
    endcol = endcol.min(g_st().default_grid.cols as Integer);
    if endcol <= startcol {
        return;
    }
    for r in (startrow as i32)..(endrow as i32) {
        compose_line(r as Integer, startcol, endcol, K_LINE_FLAG_INVALID);
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
    chunk: &[Schar],
    attrs: &[Sattr],
) {
    if !ui_comp_should_draw() || !ui_comp_set_grid(grid as i32) {
        return;
    }

    let curgrid = with_comp(UiCompositorState::curgrid_mut);
    row += curgrid.comp_row as Integer;
    startcol += curgrid.comp_col as Integer;
    endcol += curgrid.comp_col as Integer;
    clearcol += curgrid.comp_col as Integer;
    if with_comp(|c| c.curgrid.get() != &g_st().default_grid) {
        flags &= !K_LINE_FLAG_WRAP;
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

    let covered = curgrid_covered_above(row as i32, row as i32, startcol as i32, clearcol as i32);
    if (flags & K_LINE_FLAG_INVALID) != 0 || covered || curgrid.blending {
        compose_debug(
            row,
            row + 1,
            startcol,
            clearcol,
            with_comp(|c| c.dbghl_composed.get()),
            true,
        );
        compose_line(row, startcol, clearcol, flags);
    } else {
        compose_debug(
            row,
            row + 1,
            startcol,
            endcol,
            with_comp(|c| c.dbghl_normal.get()),
            endcol >= clearcol,
        );
        compose_debug(
            row,
            row + 1,
            endcol,
            clearcol,
            with_comp(|c| c.dbghl_clear.get()),
            true,
        );
        for &attr in &attrs[..((endcol - startcol) as usize)] {
            debug_assert!(attr >= 0);
        }
        ui_composed_call_raw_line(
            1, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        );
    }
}

/// The screen is invalid and will soon be cleared (`ui_comp_set_screen_valid`).
pub fn ui_comp_set_screen_valid(valid: bool) -> bool {
    with_comp(|c| {
        let old_val = c.valid_screen.replace(valid);
        if !valid {
            c.msg_sep_row.set(-1);
        }
        old_val
    })
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
    g_st().msg_grid.comp_row = row as i32;
    if scrolled && row > 0 {
        with_comp(|c| {
            c.msg_sep_row.set((row as i32) - 1);
            if !sep_char.data.is_null() {
                c.msg_sep_char.set(schar_from_buf(sep_char.as_bytes()));
            }
        });
    } else {
        with_comp(|c| c.msg_sep_row.set(-1));
    }

    let (msg_current_row, msg_was_scrolled) =
        with_comp(|c| (c.msg_current_row.get(), c.msg_was_scrolled.get()));

    if row > (msg_current_row as Integer) && ui_comp_should_draw() {
        compose_area(
            (msg_current_row - 1).max(0) as Integer,
            row,
            0,
            g_st().default_grid.cols as Integer,
        );
    } else if row < (msg_current_row as Integer)
        && ui_comp_should_draw()
        && (msg_current_row < g_st().rows_global || (scrolled && !msg_was_scrolled))
    {
        let delta = msg_current_row - (row as i32);
        if g_st().msg_grid.blending {
            let first_row = ((row as i32) - if scrolled { 1 } else { 0 }).max(0);
            compose_area(
                first_row as Integer,
                (g_st().rows_global - delta) as Integer,
                0,
                g_st().columns_global as Integer,
            );
        } else {
            let first_row = ((row as i32) - if msg_was_scrolled { 1 } else { 0 }).max(0);
            ui_composed_call_grid_scroll(
                1,
                first_row as Integer,
                g_st().rows_global as Integer,
                0,
                g_st().columns_global as Integer,
                delta as Integer,
                0,
            );
            if scrolled && !msg_was_scrolled && row > 0 {
                compose_area(row - 1, row, 0, g_st().columns_global as Integer);
            }
        }
    }

    with_comp(|c| {
        c.msg_current_row.set(row as i32);
        c.msg_was_scrolled.set(scrolled);
    });
}

/// Check if `comp().curgrid` is covered by any other grid within the rectangle (`curgrid_covered_above`).
fn curgrid_covered_above(top: i32, bot: i32, left: i32, right: i32) -> bool {
    let (start_idx, msg_was_scrolled) =
        with_comp(|c| (c.curgrid_mut().comp_index + 1, c.msg_was_scrolled.get()));
    for idx in start_idx..layers_len() {
        let g = layer_mut(idx);
        let grid_top = g.comp_row
            - if ptr::eq(g, &raw const g_st().msg_grid) && msg_was_scrolled {
                1
            } else {
                0
            };
        let grid_bot = g.comp_row + g.comp_height - 1;
        let grid_left = g.comp_col;
        let grid_right = g.comp_col + g.comp_width - 1;

        if right >= grid_left && left <= grid_right && bot >= grid_top && top <= grid_bot {
            return true;
        }
    }
    false
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
    if !ui_comp_should_draw() || !ui_comp_set_grid(grid as i32) {
        return;
    }
    let curgrid = with_comp(UiCompositorState::curgrid_mut);
    top += curgrid.comp_row as Integer;
    bot += curgrid.comp_row as Integer;
    left += curgrid.comp_col as Integer;
    right += curgrid.comp_col as Integer;
    let covered = curgrid_covered_above(top as i32, bot as i32, left as i32, right as i32);

    if covered || curgrid.blending {
        compose_debug(
            top,
            bot,
            left,
            right,
            with_comp(|c| c.dbghl_recompose.get()),
            true,
        );
        let start_r = (top + (-rows).max(0)) as i32;
        let end_r = (bot - rows.max(0)) as i32;
        for r in start_r..end_r {
            let cg = with_comp(UiCompositorState::curgrid_mut);
            let off = cg.line_offset[(r - cg.comp_row) as usize] + (left as usize)
                - (cg.comp_col as usize);
            if cg.attrs[off] >= 0 {
                compose_line(r as Integer, left, right, 0);
            }
        }
    } else {
        ui_composed_call_grid_scroll(1, top, bot, left, right, rows, cols);
        if (opt_vars().rdb_flags & K_OPT_RDB_FLAG_COMPOSITOR) != 0 {
            debug_delay(2);
        }
    }
}

pub fn ui_comp_grid_resize(grid: Integer, width: Integer, height: Integer) {
    if grid == 1 {
        ui_composed_call_grid_resize(1, width, height);
        with_comp(|c| {
            c.chk_width.set(width as i32);
            c.chk_height.set(height as i32);
            let new_bufsize = width as usize;
            let mut lbuf = c.linebuf.borrow_mut();
            let mut abuf = c.attrbuf.borrow_mut();
            if lbuf.len() != new_bufsize {
                lbuf.resize(new_bufsize, 0);
                abuf.resize(new_bufsize, 0);
            }
        });
    }
}
