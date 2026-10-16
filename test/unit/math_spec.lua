local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)

local math_lib = cimport('./src/nvim/math.h')

describe('math functions', function()
  itp('xisinf and xisnan classify special floats', function()
    eq(1, math_lib.xisinf(math.huge))
    eq(1, math_lib.xisinf(-math.huge))
    eq(0, math_lib.xisinf(0.0))
    eq(0, math_lib.xisinf(0.0 / 0.0))

    eq(1, math_lib.xisnan(0.0 / 0.0))
    eq(0, math_lib.xisnan(math.huge))
    eq(0, math_lib.xisnan(42.5))
  end)

  itp('xctz and xpopcount count bits accurately', function()
    eq(64, math_lib.xctz(0ULL))
    eq(0, math_lib.xctz(1ULL))
    eq(12, math_lib.xctz(4096ULL))

    eq(0, math_lib.xpopcount(0ULL))
    eq(64, math_lib.xpopcount(0xffffffffffffffffULL))
    eq(4, math_lib.xpopcount(0xf0ULL))
  end)

  itp('vim_append_digit_int and trim_to_int handle overflow', function()
    local val = ffi.new('int[1]', 214748364)
    eq(1, math_lib.vim_append_digit_int(val, 7))
    eq(2147483647, val[0])
    eq(0, math_lib.vim_append_digit_int(val, 0))
    eq(2147483647, val[0])

    eq(2147483647, math_lib.trim_to_int(999999999999LL))
    eq(-2147483648, math_lib.trim_to_int(-999999999999LL))
    eq(42, math_lib.trim_to_int(42LL))
  end)
end)
