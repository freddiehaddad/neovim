local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each

describe('Vimscript value perf', function()
  before_each(function()
    n.clear()
    n.get_session().request_timeout = 30000
    n.exec([[
      function! ValueScalarWork(count)
        let total = 0
        for i in range(a:count)
          let total += i + v:true
        endfor
        return total
      endfunction

      function! ValueCopyWork(count)
        let value = {'items': [42, v:true, v:false, v:null, 1.5],
              \ 'short': 'short text', 'long': repeat('x', 64),
              \ 'list': [], 'dict': {}}
        let total = 0
        for i in range(a:count)
          let copied = deepcopy(value)
          let total += copied.items[0]
        endfor
        return total
      endfunction
    ]])
  end)

  it('evaluates scalar values and boolean coercions', function()
    local elapsed = n.exec_lua(function()
      vim.fn.ValueScalarWork(1000)
      collectgarbage('collect')
      local count = 100000
      local start = vim.uv.hrtime()
      assert(vim.fn.ValueScalarWork(count) == count * (count + 1) / 2)
      return (vim.uv.hrtime() - start) / count / 1000
    end)
    print(('\ntypval scalar evaluation: %.6f us/iteration'):format(elapsed))
  end)

  it('copies mixed nested Vimscript values', function()
    local elapsed = n.exec_lua(function()
      vim.fn.ValueCopyWork(100)
      collectgarbage('collect')
      local count = 10000
      local start = vim.uv.hrtime()
      assert(vim.fn.ValueCopyWork(count) == count * 42)
      return (vim.uv.hrtime() - start) / count / 1000
    end)
    print(('\ntypval nested deepcopy: %.6f us/iteration'):format(elapsed))
  end)
end)
