//! TUI screen cell grid (`src/nvim/tui/ugrid.c`).

pub use crate::grid::schar_from_ascii;
use std::ffi::{c_int, c_void};

use crate::memory::{xcalloc, xfree, xmalloc};

pub use crate::grid::sattr_T;
pub use crate::grid::schar_T;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UCell {
    pub data: schar_T,
    pub attr: sattr_T,
}

#[repr(C)]
#[derive(Debug)]
pub struct UGrid {
    pub row: c_int,
    pub col: c_int,
    pub width: c_int,
    pub height: c_int,
    pub cells: *mut *mut UCell,
}

unsafe fn destroy_cells(grid: &mut UGrid) {
    if !grid.cells.is_null() {
        for i in 0..grid.height.max(0) as usize {
            unsafe {
                let row_ptr = *grid.cells.add(i);
                xfree(row_ptr.cast::<c_void>());
            }
        }
        unsafe {
            xfree(grid.cells.cast::<c_void>());
        }
        grid.cells = std::ptr::null_mut();
    }
}

unsafe fn clear_region(
    grid: &mut UGrid,
    top: c_int,
    bot: c_int,
    left: c_int,
    right: c_int,
    attr: sattr_T,
) {
    let space = schar_from_ascii(b' ');
    if right < left || bot < top {
        return;
    }
    let start_col = left as usize;
    let end_col = (right + 1) as usize;
    for row in top..=bot {
        unsafe {
            let row_cells = *grid.cells.add(row as usize);
            let slice =
                std::slice::from_raw_parts_mut(row_cells.add(start_col), end_col - start_col);
            slice.fill(UCell { data: space, attr });
        }
    }
}

/// Initialize a `UGrid` (`ugrid_init`).
///
/// # Safety
/// `grid` must be a valid writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_init(grid: *mut UGrid) {
    unsafe {
        (*grid).cells = std::ptr::null_mut();
    }
}

/// Free all cell memory in `grid` (`ugrid_free`).
///
/// # Safety
/// `grid` must be a valid writable pointer to an initialized `UGrid`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_free(grid: *mut UGrid) {
    unsafe {
        destroy_cells(&mut *grid);
    }
}

/// Resize `grid` to `width` x `height` cells (`ugrid_resize`).
///
/// # Safety
/// `grid` must be a valid writable pointer to an initialized `UGrid`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_resize(grid: *mut UGrid, width: c_int, height: c_int) {
    let grid = unsafe { &mut *grid };
    unsafe {
        destroy_cells(grid);
        let h = height as usize;
        let w = width as usize;
        grid.cells = xmalloc(h * std::mem::size_of::<*mut UCell>()).cast::<*mut UCell>();
        for i in 0..h {
            *grid.cells.add(i) = xcalloc(w, std::mem::size_of::<UCell>()).cast::<UCell>();
        }
    }
    grid.width = width;
    grid.height = height;
}

/// Clear the entire `grid` (`ugrid_clear`).
///
/// # Safety
/// `grid` must be a valid writable pointer to a resized `UGrid`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_clear(grid: *mut UGrid) {
    let grid = unsafe { &mut *grid };
    unsafe {
        clear_region(grid, 0, grid.height - 1, 0, grid.width - 1, 0);
    }
}

/// Clear `row` from `col` to `endcol - 1` with `attr` (`ugrid_clear_chunk`).
///
/// # Safety
/// `grid` must be a valid writable pointer to a resized `UGrid`, and coordinates must be in bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_clear_chunk(
    grid: *mut UGrid,
    row: c_int,
    col: c_int,
    endcol: c_int,
    attr: sattr_T,
) {
    let grid = unsafe { &mut *grid };
    unsafe {
        clear_region(grid, row, row, col, endcol - 1, attr);
    }
}

/// Move `grid` cursor to `(row, col)` (`ugrid_goto`).
///
/// # Safety
/// `grid` must be a valid writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_goto(grid: *mut UGrid, row: c_int, col: c_int) {
    let grid = unsafe { &mut *grid };
    grid.row = row;
    grid.col = col;
}

/// Scroll region `(top..=bot, left..=right)` by `count` rows (`ugrid_scroll`).
///
/// # Safety
/// `grid` must be a valid writable pointer to a resized `UGrid`, and coordinates must be in bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ugrid_scroll(
    grid: *mut UGrid,
    top: c_int,
    bot: c_int,
    left: c_int,
    right: c_int,
    count: c_int,
) {
    let grid = unsafe { &mut *grid };
    let (start, stop, step) = if count > 0 {
        (top, bot - count + 1, 1)
    } else {
        (bot, top - count - 1, -1)
    };

    assert!(right >= left && left >= 0);
    let len = (right as usize) - (left as usize) + 1;

    let mut i = start;
    while i != stop {
        unsafe {
            let target_row = (*grid.cells.add(i as usize)).add(left as usize);
            let source_row = (*grid.cells.add((i + count) as usize)).add(left as usize);
            std::ptr::copy_nonoverlapping(source_row, target_row, len);
        }
        i += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ugrid_lifecycle_clear_and_scroll() {
        let mut grid = UGrid {
            row: 0,
            col: 0,
            width: 0,
            height: 0,
            cells: std::ptr::null_mut(),
        };

        unsafe {
            ugrid_init(&mut grid);
            assert!(grid.cells.is_null());

            ugrid_resize(&mut grid, 4, 3);
            assert_eq!(grid.width, 4);
            assert_eq!(grid.height, 3);

            ugrid_clear(&mut grid);
            let space = schar_from_ascii(b' ');
            for r in 0..3 {
                let row_ptr = *grid.cells.add(r);
                for c in 0..4 {
                    let cell = *row_ptr.add(c);
                    assert_eq!(
                        cell,
                        UCell {
                            data: space,
                            attr: 0
                        }
                    );
                }
            }

            ugrid_clear_chunk(&mut grid, 1, 1, 3, 7);
            let row1 = *grid.cells.add(1);
            assert_eq!((*row1.add(0)).attr, 0);
            assert_eq!((*row1.add(1)).attr, 7);
            assert_eq!((*row1.add(2)).attr, 7);
            assert_eq!((*row1.add(3)).attr, 0);

            // Scroll up by 1 row (`count = 1`): row 0 gets row 1's contents.
            ugrid_scroll(&mut grid, 0, 2, 0, 3, 1);
            let row0 = *grid.cells.add(0);
            assert_eq!((*row0.add(1)).attr, 7);
            assert_eq!((*row0.add(2)).attr, 7);

            ugrid_goto(&mut grid, 2, 3);
            assert_eq!(grid.row, 2);
            assert_eq!(grid.col, 3);

            ugrid_free(&mut grid);
            assert!(grid.cells.is_null());
        }
    }
}
