local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local eq = t.eq
local ffi = t.ffi
local itp = t.gen_itp(t.it)
local to_cstr = t.to_cstr

local hashtab = cimport('./src/nvim/hashtab.h')

describe('hashtab functions', function()
  itp('inserts, looks up, resizes, and removes keys', function()
    local ht = ffi.new('hashtab_T[1]')
    hashtab.hash_init(ht)
    eq(0, tonumber(ht[0].ht_used))

    local keys = {}
    for i = 1, 32 do
      keys[i] = to_cstr('key_' .. i)
      eq(1, hashtab.hash_add(ht, keys[i]))
    end
    eq(32, tonumber(ht[0].ht_used))

    for i = 1, 32 do
      local hi = hashtab.hash_find(ht, keys[i])
      eq('key_' .. i, ffi.string(hi.hi_key))
    end

    for i = 1, 32 do
      local hi = hashtab.hash_find(ht, keys[i])
      hashtab.hash_remove(ht, hi)
    end
    eq(0, tonumber(ht[0].ht_used))

    hashtab.hash_clear(ht)
  end)
end)
