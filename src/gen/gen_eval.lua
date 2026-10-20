---@diagnostic disable: no-unknown
local mpack = vim.mpack

local metadata_file = arg[1]
local funcs_file = arg[2]
local eval_file = arg[3]

local funcs = loadfile(eval_file)().funcs
for name, func in pairs(funcs) do
  local n = (func.func_float and 1 or 0) + (func.func_lua and 1 or 0) + (func.func and 1 or 0)
  assert(n <= 1, name .. ': only one of func, func_float, func_lua can be set')
end

local metadata = mpack.decode(io.open(metadata_file, 'rb'):read('*all'))
for _, fun in ipairs(metadata) do
  if fun.eval then
    funcs[fun.name] = true
  end
end

local func_names = vim.tbl_filter(function(name)
  return name:match('__%d*$') == nil
end, vim.tbl_keys(funcs))

table.sort(func_names)

local funcsdata = assert(io.open(funcs_file, 'w'))
funcsdata:write(mpack.encode(func_names))
funcsdata:close()
