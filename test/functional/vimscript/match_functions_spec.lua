local t = require('test.testutil')
local n = require('test.functional.testnvim')()
local Screen = require('test.functional.ui.screen')

local describe, it, before_each = t.describe, t.it, t.before_each
local eq = t.eq
local pcall_err = t.pcall_err
local clear = n.clear
local fn = n.fn
local command = n.command

before_each(clear)

describe('setmatches()', function()
  it('correctly handles case when both group and pattern entries are numbers', function()
    command('hi def link 1 PreProc')
    eq(0, fn.setmatches({ { group = 1, pattern = 2, id = 3, priority = 4 } }))
    eq({
      {
        group = '1',
        pattern = '2',
        id = 3,
        priority = 4,
      },
    }, fn.getmatches())
    eq(0, fn.setmatches({ { group = 1, pattern = 2, id = 3, priority = 4, conceal = 5 } }))
    eq({
      {
        group = '1',
        pattern = '2',
        id = 3,
        priority = 4,
        conceal = '5',
      },
    }, fn.getmatches())
    eq(
      0,
      fn.setmatches({
        { group = 1, pos1 = { 2 }, pos2 = { 6 }, id = 3, priority = 4, conceal = 5 },
      })
    )
    eq({
      {
        group = '1',
        pos1 = { 2 },
        pos2 = { 6 },
        id = 3,
        priority = 4,
        conceal = '5',
      },
    }, fn.getmatches())
  end)

  it('does not fail if highlight group is not defined', function()
    eq(0, fn.setmatches { { group = 1, pattern = 2, id = 3, priority = 4 } })
    eq({ { group = '1', pattern = '2', id = 3, priority = 4 } }, fn.getmatches())
    eq(
      0,
      fn.setmatches {
        { group = 1, pos1 = { 2 }, pos2 = { 6 }, id = 3, priority = 4, conceal = 5 },
      }
    )
    eq(
      { { group = '1', pos1 = { 2 }, pos2 = { 6 }, id = 3, priority = 4, conceal = '5' } },
      fn.getmatches()
    )
  end)
end)

describe('matchadd()', function()
  it('correctly works when first two arguments and conceal are numbers at once', function()
    command('hi def link 1 PreProc')
    eq(4, fn.matchadd(1, 2, 3, 4, { conceal = 5 }))
    eq({
      {
        group = '1',
        pattern = '2',
        priority = 3,
        id = 4,
        conceal = '5',
      },
    }, fn.getmatches())
  end)
end)

describe('matchaddpos()', function()
  local function expect_match(pos)
    eq({ { group = 'PreProc', pos1 = pos, priority = 3, id = 4 } }, fn.getmatches())
  end

  it('errors out on invalid input', function()
    command('hi clear PreProc')
    eq(
      'Vim(let):E5030: Empty list at position 0',
      pcall_err(command, 'let val = matchaddpos("PreProc", [[]])')
    )
    eq(
      'Vim(let):E5030: Empty list at position 1',
      pcall_err(command, 'let val = matchaddpos("PreProc", [1, v:_null_list])')
    )
    eq(
      'Vim(let):E5031: List or number required at position 1',
      pcall_err(command, 'let val = matchaddpos("PreProc", [1, v:_null_dict])')
    )
  end)
  it('works with 0 lnum', function()
    command('hi clear PreProc')
    eq(4, fn.matchaddpos('PreProc', { 1 }, 3, 4))
    expect_match({ 1 })
    fn.matchdelete(4)
    eq(4, fn.matchaddpos('PreProc', { { 0 }, 1 }, 3, 4))
    expect_match({ 1 })
    fn.matchdelete(4)
    eq(4, fn.matchaddpos('PreProc', { 0, 1 }, 3, 4))
    expect_match({ 1 })
  end)
  it('works with negative numbers', function()
    command('hi clear PreProc')
    eq(4, fn.matchaddpos('PreProc', { -10, 1 }, 3, 4))
    expect_match({ 1 })
    fn.matchdelete(4)
    eq(4, fn.matchaddpos('PreProc', { { -10 }, 1 }, 3, 4))
    expect_match({ 1 })
    fn.matchdelete(4)
    eq(4, fn.matchaddpos('PreProc', { { 2, -1 }, 1 }, 3, 4))
    expect_match({ 1 })
    fn.matchdelete(4)
    eq(4, fn.matchaddpos('PreProc', { { 2, 0, -1 }, 1 }, 3, 4))
    expect_match({ 1 })
  end)
  it('works with zero length', function()
    local screen = Screen.new(40, 5)
    fn.setline(1, 'abcdef')
    command('hi PreProc guifg=Red')
    eq(4, fn.matchaddpos('PreProc', { { 1, 2, 0 } }, 3, 4))
    expect_match({ 1, 2, 0 })
    screen:expect(
      [[
      ^a{1:b}cdef                                  |
      {2:~                                       }|*3
                                              |
    ]],
      {
        [1] = { foreground = Screen.colors.Red },
        [2] = { bold = true, foreground = Screen.colors.Blue1 },
      }
    )
  end)

  it('reentrant on_lines callback can call getmatches/matchadd/vartabstop safely', function()
    n.exec_lua(function()
      vim.api.nvim_buf_set_lines(0, 0, -1, false, { '  first line', '  second line' })
      vim.bo.vartabstop = '2,4'
      local m1 = vim.fn.matchaddpos('Search', { { 1, 1, 5 }, { 2, 1, 6 } })
      local m2 = vim.fn.matchadd('ErrorMsg', 'line')
      local callbacks = 0
      vim.api.nvim_buf_attach(0, false, {
        on_lines = function()
          callbacks = callbacks + 1
          local m = vim.fn.getmatches()
          assert(#m >= 1)
          local id = vim.fn.matchadd('Todo', 'first')
          vim.fn.matchdelete(id)
          if m1 then
            pcall(vim.fn.matchdelete, m1)
            m1 = nil
          elseif m2 then
            vim.fn.clearmatches()
            m2 = nil
          end
          vim.bo.vartabstop = '4,2'
          local ut = vim.fn.undotree()
          assert(type(ut.seq_cur) == 'number')
          local jl = vim.fn.getjumplist()
          assert(type(jl) == 'table')
        end,
      })
      vim.cmd('normal! ggm`G>>gg<<')
      vim.cmd('silent jumps')
      vim.cmd('redraw!')
      assert(callbacks >= 2)
    end)
  end)

  it(
    'reentrant on_lines callback in diff mode can query diff_hlID/diff_filler and undotree safely',
    function()
      n.exec_lua(function()
        vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'alpha', 'beta', 'gamma' })
        vim.cmd('diffthis')
        vim.cmd('vnew')
        vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'alpha', 'beta changed', 'gamma' })
        vim.cmd('diffthis')
        local diff_cbs = 0
        vim.api.nvim_buf_attach(0, false, {
          on_lines = function()
            diff_cbs = diff_cbs + 1
            local _ = vim.fn.diff_hlID(2, 1)
            local _ = vim.fn.diff_filler(2)
            local ut = vim.fn.undotree()
            assert(type(ut.entries) == 'table')
          end,
        })
        vim.cmd('normal! 2Gccbeta updated')
        vim.cmd('redraw!')
        vim.cmd('diffoff!')
        assert(diff_cbs >= 1)
      end)
    end
  )

  it(
    'reentrant foldexpr during multiline match prepare_search_hl can mutate matches safely',
    function()
      n.exec_lua(function()
        vim.api.nvim_buf_set_lines(
          0,
          0,
          -1,
          false,
          { 'line 1', 'line 2', 'line 3', 'line 4', 'line 5', 'line 6' }
        )
        local m_early = vim.fn.matchadd('Todo', 'line 1')
        local _ = vim.fn.matchadd('Search', 'line 2\\nline 3')
        local _ = vim.fn.matchadd('ErrorMsg', 'line 4\\nline 5')
        local fold_calls = 0
        _G.test_reentrant_foldexpr = function()
          fold_calls = fold_calls + 1
          local cur = vim.fn.getmatches()
          assert(type(cur) == 'table')
          if fold_calls == 1 and m_early then
            -- Delete earlier match to shift w_match_head indices mid-iteration
            pcall(vim.fn.matchdelete, m_early)
            m_early = nil
          elseif fold_calls == 3 then
            -- Clear all matches while prepare_search_hl is walking w_match_head
            vim.fn.clearmatches()
          end
          return 0
        end
        vim.wo.foldmethod = 'expr'
        vim.wo.foldexpr = 'v:lua.test_reentrant_foldexpr()'
        vim.cmd('normal! 2Gzt4G')
        vim.cmd('redraw!')
        assert(fold_calls >= 3)
      end)
    end
  )
end)
