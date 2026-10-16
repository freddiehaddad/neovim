local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)

local ugrid = cimport('./src/nvim/tui/ugrid.h')

describe('ugrid functions', function()
  itp('manages grid lifecycle, clear, goto, and scroll', function()
    local grid = ffi.new('UGrid[1]')
    ugrid.ugrid_init(grid)
    eq(nil, grid[0].cells)

    ugrid.ugrid_resize(grid, 5, 4)
    eq(5, grid[0].width)
    eq(4, grid[0].height)

    ugrid.ugrid_clear(grid)
    ugrid.ugrid_clear_chunk(grid, 2, 1, 4, 42)
    eq(0, grid[0].cells[2][0].attr)
    eq(42, grid[0].cells[2][1].attr)
    eq(42, grid[0].cells[2][3].attr)
    eq(0, grid[0].cells[2][4].attr)

    ugrid.ugrid_scroll(grid, 0, 3, 0, 4, 1)
    eq(42, grid[0].cells[1][1].attr)
    eq(42, grid[0].cells[1][3].attr)

    ugrid.ugrid_goto(grid, 3, 4)
    eq(3, grid[0].row)
    eq(4, grid[0].col)

    ugrid.ugrid_free(grid)
    eq(nil, grid[0].cells)
  end)
end)
