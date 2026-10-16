local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)
local to_cstr = t.to_cstr

local linematch = cimport('./src/nvim/linematch.h', './src/nvim/memory.h')

describe('linematch functions', function()
  itp('fastforward_buf_to_lnum and linematch_nbuffers align diff blocks', function()
    local s1 = 'line1\nline2_abc\nline3\n'
    local s2 = 'line1\nline2_abd\nline3\n'
    local mm1 = ffi.new('mmfile_t[1]', { { ptr = to_cstr(s1), size = #s1 } })
    local mm2 = ffi.new('mmfile_t[1]', { { ptr = to_cstr(s2), size = #s2 } })

    local ff = linematch.fastforward_buf_to_lnum(mm1[0], 2)
    eq('line2_abc\nline3\n', ffi.string(ff.ptr, ff.size))

    local blks = ffi.new('const mmfile_t *[2]', { mm1, mm2 })
    local lens = ffi.new('int[2]', { 3, 3 })
    local decisions = ffi.new('int *[1]')

    local n = tonumber(linematch.linematch_nbuffers(blks, lens, 2, decisions, false))
    eq(3, n)
    eq(3, decisions[0][0])
    eq(3, decisions[0][1])
    eq(3, decisions[0][2])

    linematch.xfree(decisions[0])
  end)
end)
