local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)
local to_cstr = t.to_cstr

local validate = cimport('./src/nvim/api/private/validate.h', './src/nvim/memory.h')

describe('api/private/validate functions', function()
  itp('formats invalid, exp, required, and conflict errors', function()
    local err = ffi.new('Error[1]')

    validate.api_err_invalid(err, to_cstr('width'), nil, -5, false)
    eq("Invalid 'width': -5", ffi.string(err[0].msg))
    validate.xfree(err[0].msg)

    validate.api_err_exp(err, to_cstr('opts'), to_cstr('Dict'), to_cstr('Array'))
    eq("Invalid 'opts': expected Dict, got Array", ffi.string(err[0].msg))
    validate.xfree(err[0].msg)

    validate.api_err_required(err, to_cstr('height'))
    eq("Required: 'height'", ffi.string(err[0].msg))
    validate.xfree(err[0].msg)

    validate.api_err_conflict(err, to_cstr('buf'), to_cstr('win'))
    eq("Conflict: 'buf' not allowed with 'win'", ffi.string(err[0].msg))
    validate.xfree(err[0].msg)
  end)
end)
