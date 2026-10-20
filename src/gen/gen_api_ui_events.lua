---@diagnostic disable: no-unknown
local mpack = vim.mpack

assert(#arg == 2)
local input = assert(io.open(arg[1], 'rb'))
local metadata_output = assert(io.open(arg[2], 'wb'))

local c_grammar = require('gen.c_grammar')
local events = c_grammar.grammar:match(input:read('*all'))
input:close()

events = vim.tbl_filter(function(ev)
  return ev[1] ~= 'empty' and ev[1] ~= 'preproc'
end, events)

for i = 1, #events do
  local ev = events[i]
  assert(ev.return_type == 'void')

  -- Allow unstabilized events starting with "_". Compare "nvim__" for methods.
  if ev.since == nil and not ev.noexport and not vim.startswith(ev.name, '_') then
    print('Ui event ' .. ev.name .. ' lacks since field.\n')
    os.exit(1)
  end
  ev.since = tonumber(ev.since)
end

-- don't expose internal attributes like "impl_name" in public metadata
local exported_attributes = { 'name', 'parameters', 'since', 'deprecated_since' }
local exported_events = {}
for _, ev in ipairs(events) do
  local ev_exported = {}
  for _, attr in ipairs(exported_attributes) do
    ev_exported[attr] = ev[attr]
  end
  for _, p in ipairs(ev_exported.parameters) do
    if p[1] == 'HlAttrs' or p[1] == 'Dict' then
      -- TODO(justinmk): for back-compat, but do clients actually look at this?
      p[1] = 'Dictionary'
    end
  end
  if not ev.noexport and not vim.startswith(ev.name, '_') then
    exported_events[#exported_events + 1] = ev_exported
  end
end

metadata_output:write(mpack.encode(exported_events))
metadata_output:close()
