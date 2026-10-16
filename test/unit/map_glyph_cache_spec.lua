local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)
local to_cstr = t.to_cstr

local map = cimport('./src/nvim/map_defs.h', './src/nvim/memory.h')

describe('map_glyph_cache functions', function()
  itp('interns, looks up, and deduplicates glyph strings', function()
    local set = ffi.new('Set_glyph[1]')
    local status = ffi.new('MHPutStatus[1]')

    for i = 1, 25 do
      local str = 'g_' .. i
      local s = ffi.new('String', { data = to_cstr(str), size = #str })
      local pos1 = map.mh_put_glyph(set, s, status)
      local pos2 = map.mh_put_glyph(set, s, status)
      eq(0, tonumber(status[0]))
      eq(pos1, pos2)
      eq(pos1, map.mh_get_glyph(set, s))
      eq(str, ffi.string(set[0].keys + pos1))
    end

    map.xfree(set[0].h.hash)
    map.xfree(set[0].keys)
  end)
end)
