local t = require('test.testutil')
local n = require('test.functional.testnvim')()

if t.skip(t.is_os('win'), 'N/A: POSIX signal delivery') then
  return
end

local describe, it, before_each = t.describe, t.it, t.before_each

describe('signal watcher ownership perf', function()
  before_each(function()
    n.clear()
    n.get_session().request_timeout = 30000
  end)

  it('acknowledged signal delivery and autocommand dispatch', function()
    local iterations = 10000
    local elapsed = n.exec_lua(function(count)
      local delivered = 0
      local handler = vim.api.nvim_create_autocmd('Signal', {
        pattern = 'SIGUSR1',
        callback = function()
          delivered = delivered + 1
        end,
      })
      local pid = vim.fn.getpid()
      local function workload(repetitions)
        for _ = 1, repetitions do
          local expected = delivered + 1
          assert(vim.uv.kill(pid, 'sigusr1'))
          assert(vim.wait(10000, function()
            return delivered == expected
          end, 0))
        end
      end
      workload(100)
      local start = vim.uv.hrtime()
      workload(count)
      local result = (vim.uv.hrtime() - start) / 1000 / count
      vim.api.nvim_del_autocmd(handler)
      return result
    end, iterations)
    print(('\nsignal delivery+dispatch: %.6f us/signal'):format(elapsed))
  end)
end)
