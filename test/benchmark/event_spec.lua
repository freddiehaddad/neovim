local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each

describe('event ownership perf', function()
  before_each(n.clear)

  it('synchronous process creation and waiting', function()
    local iterations = 500
    local elapsed = n.exec_lua(function(shell, count)
      for _ = 1, 50 do
        vim.fn.system({ shell, 'EXIT', '0' })
        assert(vim.v.shell_error == 0)
      end
      local start = vim.uv.hrtime()
      for _ = 1, count do
        vim.fn.system({ shell, 'EXIT', '0' })
        assert(vim.v.shell_error == 0)
      end
      return (vim.uv.hrtime() - start) / 1e6
    end, n.testprg('shell-test'), iterations)
    print(('\nevent spawn+wait: %.6f ms/op'):format(elapsed / iterations))
  end)

  it('persistent RPC round trips', function()
    local iterations = 5000
    local elapsed = n.exec_lua(function(count)
      local channel = vim.fn.jobstart(
        { vim.v.progpath, '-u', 'NONE', '-i', 'NONE', '--embed', '--headless' },
        { rpc = true }
      )
      local ok, result = pcall(function()
        for _ = 1, 100 do
          assert(vim.rpcrequest(channel, 'nvim_eval', '1+1') == 2)
        end
        local start = vim.uv.hrtime()
        for _ = 1, count do
          assert(vim.rpcrequest(channel, 'nvim_eval', '1+1') == 2)
        end
        return (vim.uv.hrtime() - start) / 1e6
      end)
      vim.fn.jobstop(channel)
      vim.fn.jobwait({ channel }, 10000)
      assert(ok, result)
      return result
    end, iterations)
    print(('\nevent RPC roundtrip: %.6f us/op'):format(elapsed * 1000 / iterations))
  end)

  it('stream output draining', function()
    local elapsed = n.exec_lua(function(shell)
      local start = vim.uv.hrtime()
      for _ = 1, 10 do
        local output = { stdout = 0, stderr = 0 }
        local function collect(_, lines, event)
          output[event] = output[event] + #table.concat(lines, '\n')
        end
        local channel = vim.fn.jobstart({ shell, 'DRAIN', '8192' }, {
          on_stdout = collect,
          on_stderr = collect,
        })
        assert(vim.fn.jobwait({ channel }, 10000)[1] == 17)
        assert(output.stdout == 39 * 8192 and output.stderr == 39 * 8192)
      end
      return (vim.uv.hrtime() - start) / 1e6
    end, n.testprg('shell-test'))
    print(('\nevent output drain: %.6f ms/MiB'):format(elapsed / (10 * 2 * 39 * 8192 / 1048576)))
  end)
end)
