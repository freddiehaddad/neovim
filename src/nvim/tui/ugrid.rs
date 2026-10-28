//! TUI screen cell grid (`src/nvim/tui/ugrid.c`).

use crate::grid::{Sattr, Schar, schar_from_ascii};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UCell {
    pub data: Schar,
    pub attr: Sattr,
}

#[derive(Debug, Default)]
pub struct UGrid {
    pub row: i32,
    pub col: i32,
    pub width: i32,
    pub height: i32,
    pub cells: Vec<Vec<UCell>>,
}

fn clear_region(grid: &mut UGrid, top: i32, bot: i32, left: i32, right: i32, attr: Sattr) {
    let space = schar_from_ascii(b' ');
    if right < left || bot < top {
        return;
    }
    let start_col = left as usize;
    let end_col = (right + 1) as usize;
    for row in (top as usize)..=(bot as usize) {
        grid.cells[row][start_col..end_col].fill(UCell { data: space, attr });
    }
}

/// Resize `grid` to `width` x `height` cells (`ugrid_resize`).
pub fn ugrid_resize(grid: &mut UGrid, width: i32, height: i32) {
    let h = height.max(0) as usize;
    let w = width.max(0) as usize;
    grid.cells = vec![vec![UCell::default(); w]; h];
    grid.width = width;
    grid.height = height;
}

/// Clear the entire `grid` (`ugrid_clear`).
pub fn ugrid_clear(grid: &mut UGrid) {
    clear_region(grid, 0, grid.height - 1, 0, grid.width - 1, 0);
}

/// Clear `row` from `col` to `endcol - 1` with `attr` (`ugrid_clear_chunk`).
pub fn ugrid_clear_chunk(grid: &mut UGrid, row: i32, col: i32, endcol: i32, attr: Sattr) {
    clear_region(grid, row, row, col, endcol - 1, attr);
}

/// Move `grid` cursor to `(row, col)` (`ugrid_goto`).
pub fn ugrid_goto(grid: &mut UGrid, row: i32, col: i32) {
    grid.row = row;
    grid.col = col;
}

/// Scroll region `(top..=bot, left..=right)` by `count` rows (`ugrid_scroll`).
pub fn ugrid_scroll(grid: &mut UGrid, top: i32, bot: i32, left: i32, right: i32, count: i32) {
    let (start, stop, step) = if count > 0 {
        (top, bot - count + 1, 1)
    } else {
        (bot, top - count - 1, -1)
    };

    assert!(right >= left && left >= 0);
    let start_col = left as usize;
    let end_col = (right as usize) + 1;

    let mut i = start;
    while i != stop {
        let dst = i as usize;
        let src = (i + count) as usize;
        if dst != src {
            let (dst_row, src_row) = if dst < src {
                let (head, tail) = grid.cells.split_at_mut(src);
                (&mut head[dst], &tail[0])
            } else {
                let (head, tail) = grid.cells.split_at_mut(dst);
                (&mut tail[0], &head[src])
            };
            dst_row[start_col..end_col].copy_from_slice(&src_row[start_col..end_col]);
        }
        i += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ugrid_lifecycle_clear_and_scroll() {
        let mut grid = UGrid::default();
        assert!(grid.cells.is_empty());

        ugrid_resize(&mut grid, 4, 3);
        assert_eq!(grid.width, 4);
        assert_eq!(grid.height, 3);

        ugrid_clear(&mut grid);
        let space = schar_from_ascii(b' ');
        for r in 0..3 {
            for c in 0..4 {
                assert_eq!(
                    grid.cells[r][c],
                    UCell {
                        data: space,
                        attr: 0
                    }
                );
            }
        }

        ugrid_clear_chunk(&mut grid, 1, 1, 3, 7);
        assert_eq!(grid.cells[1][0].attr, 0);
        assert_eq!(grid.cells[1][1].attr, 7);
        assert_eq!(grid.cells[1][2].attr, 7);
        assert_eq!(grid.cells[1][3].attr, 0);

        // Scroll up by 1 row (`count = 1`): row 0 gets row 1's contents.
        ugrid_scroll(&mut grid, 0, 2, 0, 3, 1);
        assert_eq!(grid.cells[0][1].attr, 7);
        assert_eq!(grid.cells[0][2].attr, 7);

        ugrid_goto(&mut grid, 2, 3);
        assert_eq!(grid.row, 2);
        assert_eq!(grid.col, 3);
    }
}
