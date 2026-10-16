local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)
local to_cstr = t.to_cstr

local sha256 = cimport('./src/nvim/sha256.h')

describe('sha256 functions', function()
  itp('sha256_self_test() succeeds', function()
    eq(true, sha256.sha256_self_test())
  end)

  itp('sha256_bytes() computes expected digests with and without salt', function()
    local function digest_hex(s, salt)
      local buf = to_cstr(s)
      local salt_ptr = salt and to_cstr(salt) or nil
      local salt_len = salt and #salt or 0
      return ffi.string(sha256.sha256_bytes(buf, #s, salt_ptr, salt_len))
    end

    eq('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', digest_hex(''))
    eq('ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb', digest_hex('a'))
    eq('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad', digest_hex('abc'))
    eq(digest_hex('foobar'), digest_hex('foo', 'bar'))
  end)

  itp('sha256_start/update/finish streaming matches one-shot sha256_bytes', function()
    local ctx = ffi.new('context_sha256_T[1]')
    local raw_digest = ffi.new('uint8_t[32]')

    sha256.sha256_start(ctx)
    local chunk1 = 'abcdbcdecdefdefgefghfghighij'
    local chunk2 = 'hijkijkljklmklmnlmnomnopnopq'
    sha256.sha256_update(ctx, to_cstr(chunk1), #chunk1)
    sha256.sha256_update(ctx, to_cstr(chunk2), #chunk2)
    sha256.sha256_finish(ctx, raw_digest)

    local hex = {}
    for i = 0, 31 do
      hex[i + 1] = string.format('%02x', raw_digest[i])
    end
    eq('248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1', table.concat(hex))
  end)
end)
