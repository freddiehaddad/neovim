local fileio_enum_file = arg[1]
local auevents_file = arg[2]

---@type {events: table<string, boolean>, aliases: table<string, string>}
local auevents = assert(loadfile(auevents_file))()
local events = auevents.events
local aliases = auevents.aliases

--- @type string[]
local names = vim.tbl_keys(vim.tbl_extend('error', events, aliases))
table.sort(names, function(a, b)
  return a:lower() < b:lower()
end)

local enum_tgt = assert(io.open(fileio_enum_file, 'w'))

enum_tgt:write([[
// IWYU pragma: private, include "nvim/autocmd_defs.h"

typedef enum auto_event {]])

for i, name in ipairs(names) do
  enum_tgt:write(('\n  EVENT_%s = %u,'):format(name:upper(), i - 1))
  local pref_name = aliases[name] ~= nil and aliases[name] or name
  local win_local = events[pref_name]
  assert(win_local ~= nil)
end

enum_tgt:write(('\n  NUM_EVENTS = %u,'):format(#names))
enum_tgt:write('\n} event_T;\n')
enum_tgt:close()
