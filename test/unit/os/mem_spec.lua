local t = require('test.unit.testutil')
local cimport = t.cimport
local describe = t.describe
local itp = t.gen_itp(t.it)
local ok = t.ok

local os_lib = cimport('./src/nvim/os/os.h')

describe('os memory functions', function()
  itp('os_get_total_mem_kib returns positive physical memory size', function()
    ok(os_lib.os_get_total_mem_kib() > 0ULL)
  end)
end)
