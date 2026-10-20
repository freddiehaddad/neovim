---@diagnostic disable: no-unknown
-- Generates API metadata (.mpack) for eval generators.

local c_grammar = require('gen.c_grammar')

-- output mpack file with raw metadata, for use by gen_eval.lua and gen_eval_files.lua
local eval_funcs_metadata_outputf = arg[1]
local dispatch_deprecated_inputf = arg[2]
local pre_args = 2
assert(#arg >= 2)

local function real_type(type, exported)
  local ptype = c_grammar.typed_container:match(type)
  if ptype then
    local container = ptype[1]
    if container == 'Union' then
      return 'Object'
    elseif container == 'Tuple' then
      return 'Array'
    elseif container == 'ArrayOf' then
      if exported then
        local elem = real_type(ptype[2], true)
        if ptype[3] then
          return ('ArrayOf(%s, %s)'):format(elem, ptype[3])
        end
        return ('ArrayOf(%s)'):format(elem)
      end
      return 'Array'
    elseif container == 'DictOf' or container == 'DictAs' then
      return 'Dict'
    elseif container == 'LuaRefOf' then
      return 'LuaRef'
    elseif container == 'Enum' then
      return 'String'
    elseif container == 'Dict' then
      if exported then
        return 'Dict'
      end
      -- internal type, used for keysets
      return 'KeyDict_' .. ptype[2]
    end
  end
  return type
end

--- @class gen_api_dispatch.Function : nvim.c_grammar.Proto
--- @field method boolean
--- @field receives_array_args? true
--- @field receives_channel_id? true
--- @field can_fail? true
--- @field has_lua_imp? true
--- @field receives_arena? true
--- @field opts_idx? integer Index of the `opts` param: starts optional params. #31903
--- @field impl_name? string
--- @field remote? boolean
--- @field lua? boolean
--- @field eval? boolean
--- @field handler_id? integer

--- @type gen_api_dispatch.Function[]
local functions = {}

--- Names of all headers relative to the source root (for inclusion in the
--- generated file)
--- @type string[]
local headers = {}

--- Set of function names, used to detect duplicates
--- @type table<string, true>
local function_names = {}

local startswith = vim.startswith

--- @param fn gen_api_dispatch.Function
local function add_function(fn)
  local public = startswith(fn.name, 'nvim_') or fn.deprecated_since
  if public and not fn.noexport then
    functions[#functions + 1] = fn
    function_names[fn.name] = true
    if
      #fn.parameters >= 2
      and fn.parameters[2][1] == 'Array'
      and fn.parameters[2][2] == 'uidata'
    then
      -- function receives the "args" as a parameter
      fn.receives_array_args = true
      -- remove the args parameter
      table.remove(fn.parameters, 2)
    end
    if #fn.parameters ~= 0 and fn.parameters[1][2] == 'channel_id' then
      -- this function should receive the channel id
      fn.receives_channel_id = true
      -- remove the parameter since it won't be passed by the api client
      table.remove(fn.parameters, 1)
    end
    if #fn.parameters ~= 0 and fn.parameters[#fn.parameters][1] == 'error' then
      -- function can fail if the last parameter type is 'Error'
      fn.can_fail = true
      -- remove the error parameter, msgpack has it's own special field
      -- for specifying errors
      fn.parameters[#fn.parameters] = nil
    end
    if #fn.parameters ~= 0 and fn.parameters[#fn.parameters][1] == 'lstate' then
      fn.has_lua_imp = true
      fn.parameters[#fn.parameters] = nil
    end
    if #fn.parameters ~= 0 and fn.parameters[#fn.parameters][1] == 'arena' then
      fn.receives_arena = true
      fn.parameters[#fn.parameters] = nil
    end
    -- `opts` (a KeyDict) and every parameter following it are optional: clients may omit them.
    fn.opts_idx = c_grammar.opts_index(fn.parameters)
    -- Omitted optional param is zero-initialized, which assumes Dict/Array.
    if fn.opts_idx then
      for k = fn.opts_idx + 1, #fn.parameters do
        local rt = real_type(fn.parameters[k][1])
        if not (rt == 'Dict' or rt == 'Dictionary' or rt == 'Array' or rt:match('^KeyDict_')) then
          local msg = '%s: optional param "%s" has type "%s" but (currently) we assume Dict/Array'
          error(msg:format(fn.name, fn.parameters[k][2], fn.parameters[k][1]))
        end
      end
    end
  end
end

--- @class gen_api_dispatch.Keyset
--- @field name string
--- @field keys string[]
--- @field c_names table<string, string>
--- @field types table<string, string>
--- @field has_optional boolean

--- @type gen_api_dispatch.Keyset[]
local keysets = {}

--- @param val nvim.c_grammar.Keyset
local function add_keyset(val)
  local keys = {} --- @type string[]
  local types = {} --- @type table<string, string>
  local c_names = {} --- @type table<string, string>
  local is_set_name = 'is_set__' .. val.keyset_name .. '_'
  local has_optional = false
  for i, field in ipairs(val.fields) do
    local dict_key = field.dict_key or field.name
    if field.type ~= 'Object' then
      types[dict_key] = field.type
    end
    if field.name ~= is_set_name and field.type ~= 'OptionalKeys' then
      table.insert(keys, dict_key)
      if dict_key ~= field.name then
        c_names[dict_key] = field.name
      end
    else
      if i > 1 then
        error("'is_set__{type}_' must be first if present")
      elseif field.name ~= is_set_name then
        error(val.keyset_name .. ': name of first key should be ' .. is_set_name)
      elseif field.type ~= 'OptionalKeys' then
        error("'" .. is_set_name .. "' must have type 'OptionalKeys'")
      end
      has_optional = true
    end
  end
  keysets[#keysets + 1] = {
    name = val.keyset_name,
    keys = keys,
    c_names = c_names,
    types = types,
    has_optional = has_optional,
  }
end

-- read each input file, parse and append to the api metadata
for i = pre_args + 1, #arg do
  local full_path = arg[i]
  local parts = {} --- @type string[]
  for part in full_path:gmatch('[^/\\]+') do
    parts[#parts + 1] = part
  end
  headers[#headers + 1] = parts[#parts - 1] .. '/' .. parts[#parts]

  local input = assert(io.open(full_path, 'rb'))

  --- @type string
  local text = input:read('*all')
  for _, val in ipairs(c_grammar.grammar:match(text)) do
    if val.keyset_name then
      --- @cast val nvim.c_grammar.Keyset
      add_keyset(val)
    elseif val.name then
      --- @cast val gen_api_dispatch.Function
      add_function(val)
    end
  end

  input:close()
end

--- @generic T: table
--- @param orig T
--- @return T
local function shallowcopy(orig)
  local copy = {}
  for orig_key, orig_value in pairs(orig) do
    copy[orig_key] = orig_value
  end
  return copy
end

--- Export functions under older deprecated names.
--- These will be removed eventually.
--- @type table<string, string>
local deprecated_aliases = loadfile(dispatch_deprecated_inputf)()

for _, f in ipairs(shallowcopy(functions)) do
  local ismethod = false
  if startswith(f.name, 'nvim_') then
    if startswith(f.name, 'nvim__') or f.name == 'nvim_error_event' then
      f.since = -1
    elseif f.since == nil then
      print('Function ' .. f.name .. ' lacks since field.\n')
      os.exit(1)
    end
    f.since = tonumber(f.since)
    if f.deprecated_since ~= nil then
      f.deprecated_since = tonumber(f.deprecated_since)
    end

    if startswith(f.name, 'nvim_buf_') then
      ismethod = true
    elseif startswith(f.name, 'nvim_win_') then
      ismethod = true
    elseif startswith(f.name, 'nvim_tabpage_') then
      ismethod = true
    end
    f.remote = f.remote_only or not f.lua_only
    f.lua = f.lua_only or not f.remote_only
    f.eval = (not f.lua_only) and not f.remote_only
  else
    f.deprecated_since = tonumber(f.deprecated_since)
    assert(f.deprecated_since == 1)
    f.remote = true
    f.since = 0
  end
  f.method = ismethod
  local newname = deprecated_aliases[f.name]
  if newname ~= nil then
    if function_names[newname] then
      -- duplicate
      print(
        'Function '
          .. f.name
          .. ' has deprecated alias\n'
          .. newname
          .. ' which has a separate implementation.\n'
          .. 'Remove it from src/nvim/api/dispatch_deprecated.lua'
      )
      os.exit(1)
    end
    local newf = shallowcopy(f)
    newf.name = newname
    if newname == 'ui_try_resize' then
      -- The return type was incorrectly set to Object in 0.1.5.
      -- Keep it that way for clients that rely on this.
      newf.return_type = 'Object'
    end
    newf.impl_name = f.name
    newf.lua = false
    newf.eval = false
    newf.since = 0
    newf.deprecated_since = 1
    functions[#functions + 1] = newf
  end
end

--- @cast functions {[integer]: gen_api_dispatch.Function, keysets: gen_api_dispatch.Keyset[]}
functions.keysets = keysets
local mpack_output = assert(io.open(eval_funcs_metadata_outputf, 'wb'))
mpack_output:write(vim.mpack.encode(functions))
mpack_output:close()
