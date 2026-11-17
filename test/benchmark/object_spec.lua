local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each

describe('API Object perf', function()
  before_each(function()
    n.clear()
    n.get_session().request_timeout = 30000
  end)

  it('copies mixed nested values through the Lua API', function()
    local iterations = 100000
    local elapsed = n.exec_lua(function(count)
      local value = {
        items = { 42, true, false, 1.25, vim.NIL, 'a\0b' },
        text = string.rep('x', 64),
        list = {},
        dict = vim.empty_dict(),
      }
      for _ = 1, 1000 do
        assert(vim.api.nvim__id(value).items[1] == 42)
      end
      collectgarbage('collect')
      local start = vim.uv.hrtime()
      for _ = 1, count do
        assert(vim.api.nvim__id(value).items[1] == 42)
      end
      return (vim.uv.hrtime() - start) / count / 1000
    end, iterations)
    print(('\nobject Lua API copy: %.6f us/op'):format(elapsed))
  end)

  it('round trips nested values through Vimscript', function()
    local iterations = 50000
    local elapsed = n.exec_lua(function(count)
      local value = {
        items = { 42, true, false, 1.25, vim.NIL },
        text = string.rep('x', 64),
        list = {},
        dict = vim.empty_dict(),
      }
      for _ = 1, 1000 do
        assert(vim.api.nvim_call_function('copy', { value }).items[1] == 42)
      end
      collectgarbage('collect')
      local start = vim.uv.hrtime()
      for _ = 1, count do
        assert(vim.api.nvim_call_function('copy', { value }).items[1] == 42)
      end
      return (vim.uv.hrtime() - start) / count / 1000
    end, iterations)
    print(('\nobject Vimscript roundtrip: %.6f us/op'):format(elapsed))
  end)

  it('round trips mixed payloads over persistent RPC', function()
    local iterations = 5000
    local elapsed = n.exec_lua(function(count)
      local value = {
        items = { 42, true, false, 1.25, vim.NIL, 'a\0b' },
        text = string.rep('x', 64),
        list = {},
        dict = vim.empty_dict(),
      }
      local channel = vim.fn.jobstart(
        { vim.v.progpath, '-u', 'NONE', '-i', 'NONE', '--embed', '--headless' },
        { rpc = true }
      )
      local ok, result = pcall(function()
        for _ = 1, 100 do
          assert(vim.rpcrequest(channel, 'nvim__id', value).items[1] == 42)
        end
        collectgarbage('collect')
        local start = vim.uv.hrtime()
        for _ = 1, count do
          assert(vim.rpcrequest(channel, 'nvim__id', value).items[1] == 42)
        end
        return (vim.uv.hrtime() - start) / count / 1000
      end)
      vim.fn.jobstop(channel)
      vim.fn.jobwait({ channel }, 10000)
      assert(ok, result)
      return result
    end, iterations)
    print(('\nobject RPC payload roundtrip: %.6f us/op'):format(elapsed))
  end)
end)
