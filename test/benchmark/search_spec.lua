local n = require('test.functional.testnvim')()
local t = require('test.testutil')

local describe, it, before_each = t.describe, t.it, t.before_each

describe('parallel search, fuzzy, and diff perf', function()
  before_each(n.clear)

  it(':vimgrep across 100k buffer lines', function()
    local samples = n.exec_lua(function()
      local words = { 'alpha', 'beta', 'gamma', 'delta', 'epsilon', 'zeta', 'eta', 'theta' }
      math.randomseed(1)
      local lines = {} --- @type string[]
      for i = 1, 100000 do
        local w1 = words[math.random(#words)]
        local w2 = words[math.random(#words)]
        if i % 250 == 0 then
          lines[i] = ('%s_%03d = %s_handler(%d)'):format(w1, i % 1000, w2, i)
        else
          lines[i] = ('local %s_%s = %d + %d'):format(w1, w2, i, i * 2)
        end
      end
      vim.api.nvim_buf_set_name(0, 'bench_vimgrep.lua')
      vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)

      vim.cmd([[silent vimgrep /\v(alpha|beta)_[0-9]{3} \= [a-z]+_handler/j %]])
      assert(#vim.fn.getqflist() > 0)

      local out = {} --- @type number[]
      for i = 1, 5 do
        local t0 = vim.uv.hrtime()
        vim.cmd([[silent vimgrep /\v(alpha|beta)_[0-9]{3} \= [a-z]+_handler/j %]])
        out[i] = vim.uv.hrtime() - t0
      end
      return out
    end)
    t.bench_report(samples, { label = 'vimgrep_100k' })
  end)

  it('matchfuzzy() across 50k file paths', function()
    local samples = n.exec_lua(function()
      local dirs = { 'src/nvim', 'runtime/lua/vim', 'test/functional', 'src/xdiff', 'src/vterm' }
      local subs = { 'api', 'eval', 'event', 'lsp', 'treesitter', 'ui', 'os', 'tui' }
      local names = {
        'buffer',
        'window',
        'extmark',
        'regexp',
        'memline',
        'options',
        'autocmd',
        'channel',
        'scanner',
        'parser',
      }
      local paths = {} --- @type string[]
      for i = 1, 50000 do
        local d = dirs[(i % #dirs) + 1]
        local s = subs[(math.floor(i / #dirs) % #subs) + 1]
        local nm = names[(math.floor(i / (#dirs * #subs)) % #names) + 1]
        paths[i] = ('%s/%s/%s_%05d.rs'):format(d, s, nm, i)
      end

      local res = vim.fn.matchfuzzy(paths, 'src/nvim/eval/buffer')
      assert(#res > 0)

      local out = {} --- @type number[]
      for i = 1, 5 do
        local t0 = vim.uv.hrtime()
        vim.fn.matchfuzzy(paths, 'src/nvim/eval/buffer')
        out[i] = vim.uv.hrtime() - t0
      end
      return out
    end)
    t.bench_report(samples, { label = 'matchfuzzy_50k' })
  end)

  it('vim.diff() with linematch across modified blocks', function()
    local samples = n.exec_lua(function()
      local a_lines = {} --- @type string[]
      local b_lines = {} --- @type string[]
      for hunk = 1, 8 do
        for j = 1, 120 do
          a_lines[#a_lines + 1] = ('  local value_%03d_%03d = compute_item(%d, "old_state_value", %d)'):format(
            hunk,
            j,
            j,
            j * 3
          )
          if j % 15 ~= 0 then
            b_lines[#b_lines + 1] = ('  local value_%03d_%03d = compute_item(%d, "new_state_value", %d)'):format(
              hunk,
              j,
              j + 1,
              j * 5
            )
          end
        end
        a_lines[#a_lines + 1] = ('-- unchanged anchor line %d'):format(hunk)
        b_lines[#b_lines + 1] = ('-- unchanged anchor line %d'):format(hunk)
      end
      local text_a = table.concat(a_lines, '\n') .. '\n'
      local text_b = table.concat(b_lines, '\n') .. '\n'

      local hunks = vim.diff(text_a, text_b, { result_type = 'indices', linematch = 300 })
      assert(#hunks > 8)

      local out = {} --- @type number[]
      for i = 1, 5 do
        local t0 = vim.uv.hrtime()
        vim.diff(text_a, text_b, { result_type = 'indices', linematch = 300 })
        out[i] = vim.uv.hrtime() - t0
      end
      return out
    end)
    t.bench_report(samples, { label = 'diff_linematch' })
  end)
end)
