local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)

local arabic = cimport('./src/nvim/arabic.h', './src/nvim/option_vars.h')

describe('arabic functions', function()
  itp('shapes isolated, initial, medial, final, and lam-alef ligatures', function()
    arabic.p_arshape = 1
    arabic.p_tbidi = 0

    eq(true, arabic.arabic_maycombine(0x0627))
    eq(true, arabic.arabic_combine(0x0644, 0x0627))
    eq(false, arabic.arabic_combine(0x0628, 0x0627))

    local c1 = ffi.new('int[1]', 0x0627)
    eq(0xfefb, arabic.arabic_shape(0x0644, c1, 0, 0, 0))
    eq(0, c1[0])

    local zero = ffi.new('int[1]', 0)
    eq(0xfe8f, arabic.arabic_shape(0x0628, zero, 0, 0, 0))
    eq(0xfe91, arabic.arabic_shape(0x0628, zero, 0, 0, 0x062a))
    eq(0xfe92, arabic.arabic_shape(0x0628, zero, 0x062a, 0, 0x062a))
    eq(0xfe90, arabic.arabic_shape(0x0628, zero, 0x062a, 0, 0))
  end)
end)
