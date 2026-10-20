---@diagnostic disable: no-unknown
-- Will generate file ex_cmds_enum.generated.h with cmdidx_T enum.

local enumfname = arg[1] -- '/ex_cmds_enum.generated.h'
local ex_cmds_name = arg[2] -- 'ex_cmds.lua'

local enumfile = assert(io.open(enumfname, 'w'))

local bit = require 'bit'
local ex_cmds = loadfile(ex_cmds_name)()
local defs = ex_cmds.cmds
local flags = ex_cmds.flags

enumfile:write([[
// IWYU pragma: private, include "nvim/ex_cmds_defs.h"

typedef enum CMD_index {
]])

for _, cmd in ipairs(defs) do
  if bit.band(cmd.flags, flags.RANGE) == flags.RANGE then
    assert(
      cmd.addr_type ~= 'ADDR_NONE',
      string.format('ex_cmds.lua:%s: Using RANGE with ADDR_NONE\n', cmd.command)
    )
  else
    assert(
      cmd.addr_type == 'ADDR_NONE',
      string.format('ex_cmds.lua:%s: Missing ADDR_NONE\n', cmd.command)
    )
  end
  if bit.band(cmd.flags, flags.DFLALL) == flags.DFLALL then
    assert(
      cmd.addr_type ~= 'ADDR_OTHER' and cmd.addr_type ~= 'ADDR_NONE',
      string.format('ex_cmds.lua:%s: Missing misplaced DFLALL\n', cmd.command)
    )
  end
  if bit.band(cmd.flags, flags.PREVIEW) == flags.PREVIEW then
    assert(
      cmd.preview_func ~= nil,
      string.format('ex_cmds.lua:%s: Missing preview_func\n', cmd.command)
    )
  end
  if bit.band(cmd.flags, flags.BUFLOCK_OK) == flags.BUFLOCK_OK then
    assert(
      bit.band(cmd.flags, flags.LOCK_OK) == flags.LOCK_OK,
      string.format('ex_cmds.lua:%s: BUFLOCK_OK implies LOCK_OK\n', cmd.command)
    )
  end
  local enumname = cmd.enum or ('CMD_' .. cmd.command)
  enumfile:write('  ' .. enumname .. ',\n')
end

enumfile:write([[
  CMD_SIZE,
  CMD_USER = -1,
  CMD_USER_BUF = -2
} cmdidx_T;
]])
enumfile:close()
