local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each
local eq = t.eq

describe('Vimscript value representation', function()
  before_each(n.clear)

  it('preserves scalar and container kinds through Lua and API conversion', function()
    eq(
      { 0, 6, 6, 7, 5, 1, 3, 4, 10 },
      n.exec_lua(function()
        local values = { 42, false, true, vim.NIL, 1.5, '', {}, vim.empty_dict(), 'a\0b' }
        local kinds = {}
        for i, value in ipairs(values) do
          kinds[i] = vim.fn.type(vim.fn.copy(value))
        end
        return kinds
      end)
    )
    eq(
      { 3, 4, 1, 7 },
      n.eval('[type(v:_null_list), type(v:_null_dict), type(v:_null_string), type(v:null)]')
    )
    eq({ 0, vim.NIL, false }, n.eval('[get([], 0), get([], 0, v:null), get([], 0, v:false)]'))
  end)

  it('retains bound function references across copies and collection', function()
    n.exec([[
      function! ValueBound(left, right) dict
        return [a:left + a:right, self.name]
      endfunction
      let Owner = {'name': 'retained'}
      let Partial = function('ValueBound', [4], Owner)
      let Original = {'callback': Partial}
      let Copied = deepcopy(Original)
      unlet Original Partial Owner
      call garbagecollect(1)
    ]])
    eq({ 7, 'retained' }, n.eval('Copied.callback(3)'))
    eq(
      { 2, 'ValueBound', { 4 } },
      n.eval(
        '[type(get(Copied.callback, "func")), get(Copied.callback, "name"), get(Copied.callback, "args")]'
      )
    )
  end)

  it('distinguishes nested list and dictionary identities while decoding JSON', function()
    eq(
      { edits = { { start = 0, deleteCount = 1, data = { 1 } } }, resultId = '2' },
      n.fn.json_decode('{"edits":[{"start":0,"deleteCount":1,"data":[1]}],"resultId":"2"}')
    )
    eq(
      { left = { values = { { 1 }, {} } }, right = { empty = vim.empty_dict() } },
      n.fn.json_decode('{"left":{"values":[[1],[]]},"right":{"empty":{}}}')
    )
  end)

  it('preserves equality and identity for named functions and bound partials', function()
    n.exec([=[
      let BaseFunc = function('len')
      let Bound = function('len', [[1, 2]])
      let OtherBound = function('len', [[1, 2]])
    ]=])
    eq(
      { 1, 1, 0, 0, 0, 1, 0 },
      n.eval(
        '[BaseFunc == BaseFunc, BaseFunc is BaseFunc, BaseFunc == Bound, Bound == BaseFunc,'
          .. ' BaseFunc is Bound, Bound == OtherBound, Bound is OtherBound]'
      )
    )
  end)

  it('restores v:key and v:val kinds after nested evaluation and errors', function()
    n.exec([[
      function! ValueNested(index, value)
        let before = [v:key, v:val]
        call map([10, 20], {i, v -> i + v})
        call assert_equal(before, [v:key, v:val])
        return type(v:val)
      endfunction
      let Kinds = map([0, v:false, v:null, 'text', [], {}], function('ValueNested'))
    ]])
    eq({ 0, 6, 7, 1, 3, 4 }, n.eval('Kinds'))
    eq({}, n.eval('v:errors'))
    eq({ 0, 0 }, n.eval('[exists("v:key"), exists("v:val")]'))
    t.matches('E121:', t.pcall_err(n.command, 'call map([1], "MissingValueVariable")'))
    eq({ 0, 0 }, n.eval('[exists("v:key"), exists("v:val")]'))
  end)

  it('keeps numeric coercions and lock metadata separate from value kinds', function()
    n.exec([[
      let Values = [v:true, v:false, v:null, 1.5, '7']
      lockvar! Values
      let Copied = deepcopy(Values)
    ]])
    eq({ 1, 0, 0, 5 }, n.eval('[v:true + 0, v:false + 0, v:null + 0, type(str2float("-1.5"))]'))
    eq(-1.5, n.eval('str2float("-1.5")'))
    eq(1, n.eval('islocked("Values[0]")'))
    eq(0, n.eval('islocked("Copied[0]")'))
    eq({ 6, 6, 7, 5, 1 }, n.eval('map(Copied, {i, v -> type(v)})'))
  end)

  it('accepts Boolean results from blob map and mapnew', function()
    n.exec([[
      let Original = 0z020304
      let Mapped = mapnew(Original, {i, v -> [v:true, v:false, 7][i]})
    ]])
    eq({ '0z020304', '0z010007' }, n.eval('[string(Original), string(Mapped)]'))
    eq('0z010007', n.eval('string(map(Original, {i, v -> [v:true, v:false, 7][i]}))'))

    for _, name in ipairs({ 'map', 'mapnew' }) do
      for _, value in ipairs({ 'v:null', "'1'" }) do
        t.matches('E978:', t.pcall_err(n.eval, name .. '(0z00, {i, v -> ' .. value .. '})'))
      end
    end
  end)
end)
