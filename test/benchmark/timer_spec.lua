local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each

describe('timer ownership perf', function()
  before_each(n.clear)

  it('creation, dispatch, and wait teardown', function()
    local iterations = 100000
    local elapsed = n.exec_lua(function(count)
      local function drain()
        vim.uv.run('nowait')
        local done = false
        vim.schedule(function()
          done = true
        end)
        assert(vim.wait(10000, function()
          return done
        end, 0))
      end
      local function noop() end
      local workloads = {
        function(repetitions)
          for _ = 1, repetitions do
            vim.fn.timer_stop(vim.fn.timer_start(60000, noop))
          end
        end,
        function(repetitions)
          local fired = 0
          vim.fn.timer_start(0, function()
            fired = fired + 1
          end, { ['repeat'] = repetitions })
          assert(vim.wait(10000, function()
            return fired == repetitions
          end, 0))
        end,
        function(repetitions)
          for _ = 1, repetitions do
            assert(vim.fn.wait(0, 1) == 0)
          end
        end,
      }
      local results = {}
      for i, workload in ipairs(workloads) do
        workload(200)
        drain()
        local start = vim.uv.hrtime()
        workload(count)
        drain()
        results[i] = (vim.uv.hrtime() - start) / 1000 / count
      end
      return results
    end, iterations)
    print(('\ntimer create+cancel: %.6f us/op'):format(elapsed[1]))
    print(('\ntimer zero-delay dispatch: %.6f us/tick'):format(elapsed[2]))
    print(('\ntimer wait lifecycle: %.6f us/op'):format(elapsed[3]))
  end)
end)
