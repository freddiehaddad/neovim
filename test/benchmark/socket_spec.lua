local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each

describe('socket listener ownership perf', function()
  before_each(function()
    n.clear()
    n.get_session().request_timeout = 30000
  end)

  it('listener creation and native close', function()
    local iterations = 5000
    local elapsed = n.exec_lua(function(count)
      local function drain()
        vim.uv.run('nowait')
      end
      local function workload(repetitions)
        for i = 1, repetitions do
          local address = vim.fn.serverstart('127.0.0.1:0')
          assert(vim.fn.serverstop(address) == 1)
          if i % 100 == 0 then
            drain()
          end
        end
        drain()
      end
      workload(100)
      local start = vim.uv.hrtime()
      workload(count)
      return (vim.uv.hrtime() - start) / 1000 / count
    end, iterations)
    print(('\nsocket listen+close: %.6f us/op'):format(elapsed))
  end)

  for _, transport in ipairs({ 'tcp', 'pipe' }) do
    it(transport .. ' connection, RPC handshake, and close', function()
      local iterations = 1000
      local address = transport == 'tcp' and n.fn.serverstart('127.0.0.1:0') or n.fn.serverstart()
      local base_channels = #n.api.nvim_list_chans()
      local function drain()
        n.exec_lua(function(expected)
          assert(vim.wait(10000, function()
            return #vim.api.nvim_list_chans() == expected
          end, 0))
          vim.uv.run('nowait')
        end, base_channels)
      end
      local function workload(count)
        for i = 1, count do
          local client = n.connect(address)
          client.request_timeout = 10000
          local ok, value = client:request('nvim_eval', '6 * 7')
          client:close()
          assert(ok and value == 42)
          if i % 100 == 0 then
            drain()
          end
        end
        drain()
      end
      workload(100)
      local start = vim.uv.hrtime()
      workload(iterations)
      local elapsed = (vim.uv.hrtime() - start) / 1000 / iterations
      assert(n.fn.serverstop(address) == 1)
      n.exec_lua("vim.uv.run('nowait')")
      print(('\nsocket %s connect+RPC+close: %.6f us/op'):format(transport, elapsed))
    end)
  end
end)
