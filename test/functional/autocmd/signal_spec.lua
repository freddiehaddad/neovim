local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each, finally = t.describe, t.it, t.before_each, t.finally
local clear = n.clear
local command = n.command
local eq = t.eq
local fn = n.fn
local next_msg = n.next_msg
local is_os = t.is_os
local skip = t.skip
local read_file = t.read_file
local feed = n.feed
local retry = t.retry

if skip(is_os('win'), 'N/A: Only applies to POSIX systems') then
  return
end

local function bound_requests()
  local session = n.get_session()
  local previous_timeout = session.request_timeout
  session.request_timeout = 10000
  finally(function()
    session.request_timeout = previous_timeout
  end)
end

describe("'autowriteall' on signal exit", function()
  before_each(clear)

  local function test_deadly_sig(signame, awa, should_write)
    local testfile = 'Xtest_SIG' .. signame .. (awa and '_awa' or '_noawa')
    local teststr = 'Testaaaaaaa'
    finally(function()
      os.remove(testfile)
    end)

    if awa then
      command('set autowriteall')
    end

    command('edit ' .. testfile)
    feed('i' .. teststr)
    vim.uv.kill(fn.getpid(), signame)

    retry(nil, 5000, function()
      eq((should_write and (teststr .. '\n') or nil), read_file(testfile))
    end)
  end

  it('dont write if SIGTERM & awa on', function()
    test_deadly_sig('sigterm', true, false)
  end)
  it('dont write if SIGTERM & awa off', function()
    test_deadly_sig('sigterm', false, false)
  end)

  it('write if SIGHUP & awa on', function()
    test_deadly_sig('sighup', true, true)
  end)
  it('dont write if SIGHUP & awa off', function()
    test_deadly_sig('sighup', false, false)
  end)

  it('write if SIGTSTP & awa on', function()
    test_deadly_sig('sigtstp', true, true)
  end)
  it('dont write if SIGTSTP & awa off', function()
    test_deadly_sig('sigtstp', false, false)
  end)

  it('write if SIGQUIT & awa on', function()
    test_deadly_sig('sigquit', true, true)
  end)
  it('dont write if SIGQUIT & awa off', function()
    test_deadly_sig('sigquit', false, false)
  end)
  it('dont write if SIGINT & awa on', function()
    test_deadly_sig('sigint', true, false)
  end)
end)

describe('autocmd Signal', function()
  before_each(clear)

  it('matches *', function()
    command('autocmd Signal * call rpcnotify(1, "foo")')
    vim.uv.kill(fn.getpid(), 'sigusr1')
    eq({ 'notification', 'foo', {} }, next_msg())
  end)

  it('matches SIGUSR1', function()
    command('autocmd Signal SIGUSR1 call rpcnotify(1, "foo")')
    vim.uv.kill(fn.getpid(), 'sigusr1')
    eq({ 'notification', 'foo', {} }, next_msg())
  end)

  it('matches SIGWINCH', function()
    command('autocmd Signal SIGWINCH call rpcnotify(1, "foo")')
    vim.uv.kill(fn.getpid(), 'sigwinch')
    eq({ 'notification', 'foo', {} }, next_msg())
  end)

  it('preserves signal routing through nested polling', function()
    bound_requests()
    n.exec_lua(function()
      local order = {}
      vim.api.nvim_create_autocmd('Signal', {
        pattern = 'SIGWINCH',
        callback = function()
          order[#order + 1] = 'winch'
        end,
      })
      vim.api.nvim_create_autocmd('Signal', {
        pattern = 'SIGUSR1',
        nested = true,
        callback = function()
          order[#order + 1] = 'usr1-enter'
          assert(vim.uv.kill(vim.fn.getpid(), 'sigwinch'))
          assert(vim.wait(2000, function()
            return order[#order] == 'winch'
          end, 0))
          order[#order + 1] = 'usr1-leave'
          vim.rpcnotify(1, 'signal-order', order)
        end,
      })
    end)
    vim.uv.kill(fn.getpid(), 'sigusr1')
    eq(
      { 'notification', 'signal-order', { { 'usr1-enter', 'winch', 'usr1-leave' } } },
      next_msg(5000)
    )
  end)

  it('can finish shutdown from a signal handler', function()
    bound_requests()
    command('autocmd Signal SIGUSR1 qall!')
    n.expect_exit(5000, vim.uv.kill, fn.getpid(), 'sigusr1')
  end)

  it('does not match unknown patterns', function()
    command('autocmd Signal SIGUSR2 call rpcnotify(1, "foo")')
    vim.uv.kill(fn.getpid(), 'sigusr2')
    eq(nil, next_msg(500))
  end)
end)
