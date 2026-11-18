local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each
local eq = t.eq
local api = n.api

describe('API Object conversion', function()
  before_each(n.clear)

  it('preserves nested wire values and empty container kinds', function()
    local value = {
      scalar = { true, false, 42, -7, 1.5, vim.NIL },
      text = 'a\0b',
      list = {},
      dict = vim.empty_dict(),
    }
    local result = api.nvim__id(value)
    eq(value, result)
    eq(true, vim.islist(result.list))
    eq(false, vim.islist(result.dict))
    eq(
      { true, false, true, true, 'a\0b' },
      n.exec_lua(function(input)
        local copied = vim.api.nvim__id(input)
        return {
          vim.islist(copied.list),
          vim.islist(copied.dict),
          input.scalar[6] == vim.NIL,
          copied.scalar[6] == nil,
          copied.text,
        }
      end, value)
    )
  end)

  it('retains independently copied nested Lua functions', function()
    eq(
      { 6, 9, 'nested', true },
      n.exec_lua(function()
        local refs = vim.api.nvim__stats().lua_refcount
        local calls = 0
        local value = {
          callback = function(amount)
            calls = calls + amount
            return calls
          end,
          nested = {
            function()
              return 'nested'
            end,
          },
        }
        local copied = vim.api.nvim__id(value)
        value = nil
        collectgarbage('collect')
        local second_copy = vim.api.nvim__id(copied)
        copied = nil
        collectgarbage('collect')
        return {
          second_copy.callback(6),
          second_copy.callback(3),
          second_copy.nested[1](),
          vim.api.nvim__stats().lua_refcount == refs,
        }
      end)
    )
  end)

  it('transfers Lua functions to Vimscript variables and back', function()
    eq(
      { 15, 21, 27 },
      n.exec_lua(function()
        local callback = function(value)
          return value * 3
        end
        vim.api.nvim_set_var('ObjectCallback', { callback = callback })
        callback = nil
        collectgarbage('collect')
        local first = vim.api.nvim_eval('g:ObjectCallback.callback(5)')
        local copied = vim.api.nvim_get_var('ObjectCallback')
        vim.api.nvim_del_var('ObjectCallback')
        collectgarbage('collect')
        return { first, copied.callback(7), copied.callback(9) }
      end)
    )
  end)

  it('still rejects Lua functions inside Array-typed arguments', function()
    eq(
      { false, true },
      n.exec_lua(function()
        local ok, err = pcall(vim.api.nvim_call_function, 'call', {
          function()
            return 1
          end,
          {},
        })
        return { ok, err:find("Invalid 'args': Cannot convert given Lua type", 1, true) ~= nil }
      end)
    )
  end)

  it('keeps callback-option ownership without broadening supported types', function()
    eq(
      { 'function', 7, 7, true },
      n.exec_lua(function()
        vim.api.nvim_set_option_value('foldexpr', function()
          return 7
        end, {})
        local callback = vim.api.nvim_get_option_value('foldexpr', {})
        local result = callback()
        vim.api.nvim_set_option_value('foldexpr', '0', {})
        collectgarbage('collect')
        local ok, err = pcall(vim.api.nvim_set_option_value, 'statusline', callback, {})
        return {
          type(callback),
          result,
          callback(),
          not ok and err:find('expected a valid type, got Function', 1, true) ~= nil,
        }
      end)
    )
  end)

  it('keeps accepted integer and empty-array argument coercions', function()
    eq(3, api.nvim__id_float(3))
    eq(false, vim.islist(api.nvim__id_dict({})))
    eq({ '' }, api.nvim_buf_get_lines(0, 0, -1, 0))
    eq(
      { 3, false, '' },
      n.exec_lua(function()
        return {
          vim.api.nvim__id_float(3),
          vim.islist(vim.api.nvim__id_dict({})),
          vim.api.nvim_buf_get_lines(0, 0, -1, 0)[1],
        }
      end)
    )
  end)

  it('keeps non-string context function entries as empty commands', function()
    api.nvim_load_context({
      funcs = { false, 42, 'function! ObjectContextValue()\nreturn 7\nendfunction' },
    })
    eq(7, api.nvim_call_function('ObjectContextValue', {}))
  end)

  it('keeps namespace window-list coercions at the API boundary', function()
    local ns = api.nvim_create_namespace('object-window-list')
    local win = api.nvim_get_current_win()
    for _, value in ipairs({ 0, false, 'current' }) do
      api.nvim__ns_set(ns, { wins = { value } })
      eq({ win }, api.nvim__ns_get(ns).wins)
    end
  end)
end)
