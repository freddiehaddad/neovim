local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each

describe('Directory ownership perf', function()
  before_each(n.clear)

  it('enumerates and sorts directory entries through readdir()', function()
    local elapsed = n.exec_lua(function()
      local path = vim.fs.joinpath(vim.env.NVIM_TEST_SOURCE_DIR, 'src', 'nvim')
      local expected = vim.fn.readdir(path)
      assert(vim.list_contains(expected, 'lib.rs'))
      for _ = 1, 100 do
        assert(#vim.fn.readdir(path) == #expected)
      end
      collectgarbage('collect')
      local count = 2000
      local start = vim.uv.hrtime()
      for _ = 1, count do
        assert(#vim.fn.readdir(path) == #expected)
      end
      return (vim.uv.hrtime() - start) / count / 1000
    end)
    print(('\ndirectory readdir/sort/convert: %.6f us/scan'):format(elapsed))
  end)
end)
