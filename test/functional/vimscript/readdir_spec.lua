local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each, after_each = t.describe, t.it, t.before_each, t.after_each
local eq = t.eq

describe('readdir()', function()
  local root

  before_each(function()
    n.clear()
    root = t.tmpname(false)
    n.mkdir_p(root)
    for _, name in ipairs({ 'alpha', 'beta', 'skip' }) do
      t.write_file(vim.fs.joinpath(root, name), '')
    end
  end)

  after_each(function()
    n.rmdir(root)
  end)

  it('keeps the current entry alive while a filter performs another scan', function()
    eq(
      { 'alpha', 'beta' },
      n.exec_lua(function(path)
        return vim.fn.readdir(path, function(entry)
          assert(#vim.fn.readdir(path) == 3)
          collectgarbage('collect')
          return entry ~= 'skip'
        end)
      end, root)
    )
  end)

  it('releases scans stopped early and preserves subsequent enumeration', function()
    eq(
      { 'alpha', 'beta', 'skip' },
      n.exec_lua(function(path)
        for _ = 1, 20 do
          assert(#vim.fn.readdir(path, function()
            return -1
          end) == 0)
        end
        return vim.fn.readdir(path)
      end, root)
    )
  end)

  it('keeps failed-open diagnostics and does not affect a later scan', function()
    eq(
      { {}, true, { 'alpha', 'beta', 'skip' } },
      n.exec_lua(function(path)
        vim.g.scan_path = vim.fs.joinpath(path, 'alpha')
        local output = vim.api.nvim_exec2('let g:scan_result = readdir(g:scan_path)', {
          output = true,
        }).output
        return { vim.g.scan_result, output:find('E484:', 1, true) ~= nil, vim.fn.readdir(path) }
      end, root)
    )
  end)
end)
