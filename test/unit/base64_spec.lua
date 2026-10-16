local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)
local to_cstr = t.to_cstr

local base64 = cimport('./src/nvim/base64.h', './src/nvim/memory.h')

describe('base64 functions', function()
  local function encode(s)
    local res = base64.base64_encode(to_cstr(s), #s)
    assert(res ~= nil)
    local ret = ffi.string(res)
    base64.xfree(res)
    return ret
  end

  local function decode(s)
    local out_len = ffi.new('size_t[1]')
    local res = base64.base64_decode(to_cstr(s), #s, out_len)
    if res == nil then
      return nil, tonumber(out_len[0])
    end
    local ret = ffi.string(res, out_len[0])
    base64.xfree(res)
    return ret, tonumber(out_len[0])
  end

  itp('encodes and decodes RFC 4648 vectors and embedded NULs', function()
    local vectors = {
      { '', '' },
      { 'f', 'Zg==' },
      { 'fo', 'Zm8=' },
      { 'foo', 'Zm9v' },
      { 'foob', 'Zm9vYg==' },
      { 'fooba', 'Zm9vYmE=' },
      { 'foobar', 'Zm9vYmFy' },
      { 'Hello\0world', 'SGVsbG8Ad29ybGQ=' },
    }

    for _, pair in ipairs(vectors) do
      local raw, b64 = pair[1], pair[2]
      eq(b64, encode(raw))
      local dec, len = decode(b64)
      eq(raw, dec)
      eq(#raw, len)
    end
  end)

  itp('returns NULL and out_len=0 on invalid input', function()
    for _, invalid in ipairs({ 'A', 'AA', 'AAA', 'AA=A', 'AA/=', 'A/==', 'A===', '====' }) do
      local dec, len = decode(invalid)
      eq(nil, dec)
      eq(0, len)
    end
  end)
end)
