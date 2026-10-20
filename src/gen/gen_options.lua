---@diagnostic disable: no-unknown
local options_input_file = arg[3]

--- @module 'nvim.options'
local options = loadfile(options_input_file)()
local options_meta = options.options
local valid_scopes = options.valid_scopes
local schema_values = options.schema_values

--- True if an option is a dict option: a `dict` schema (typed key:value map, e.g. 'diffopt'), as
--- opposed to a flag/enum/char category.
--- @param o vim.option_meta
--- @return boolean
local function is_dict_option(o)
  return o.schema ~= nil and o.schema.dict ~= nil
end

--- @param s string
--- @return string
local function lowercase_to_titlecase(s)
  return table.concat(vim.tbl_map(function(word) --- @param word string
    return word:sub(1, 1):upper() .. word:sub(2)
  end, vim.split(s, '[-_]')))
end

--- @param scope string
--- @param option_name string
--- @return string
local function get_scope_option(scope, option_name)
  return ('k%sOpt%s'):format(lowercase_to_titlecase(scope), lowercase_to_titlecase(option_name))
end

--- Emit an option's flag-set enum from its `flagchars` or `flags` schema:
---   * `flagchars` (e.g. 'formatoptions'): a name->char map -> `enum { kFoWrap = 't', … }`
---   * `flags` (e.g. 'foldopen', 'virtualedit'): an ordered list of names (bit = 2^i) or of
---     {name, bit} pairs -> `typedef enum { kOptFdoFlagHor = 0x…, … } OptFdoFlags;`. A pair's
---     optional 3rd element overrides the C constant's token (e.g. 'NONE' -> `NoneU`).
--- @param o vim.option_meta
--- @return string
local function gen_token_enum(o)
  local out = {} --- @type string[]
  local function add(s)
    table.insert(out, s)
  end
  local opt_name = lowercase_to_titlecase(o.abbreviation or o.full_name)

  if o.schema.flagchars then
    -- Char flags complete as raw chars, so order is irrelevant; sort by name for a stable enum.
    local names = vim.tbl_keys(o.schema.flagchars) --[[ @as string[] ]]
    table.sort(names)
    add('')
    add('enum {')
    for _, name in ipairs(names) do
      add(
        ("  k%s%s = '%s',"):format(
          opt_name,
          lowercase_to_titlecase(name),
          (o.schema.flagchars[name]:gsub("[\\']", '\\%0'))
        )
      )
    end
    add('};')
    return table.concat(out, '\n')
  end

  -- Bitmask flags: collect (C-token, bit) and emit in bit order.
  local flags = {} --- @type {name:string,bit:integer}[]
  for i, entry in ipairs(o.schema.flags) do
    if type(entry) == 'string' then
      flags[#flags + 1] = { name = entry, bit = math.pow(2, i - 1) }
    else
      local e = entry --[[ @as {[1]:string, [2]:integer, [3]:string?} ]]
      flags[#flags + 1] = { name = e[3] or e[1], bit = e[2] }
    end
  end
  table.sort(flags, function(a, b)
    return a.bit < b.bit
  end)

  add('')
  add('typedef enum {')
  for _, f in ipairs(flags) do
    -- A "key:" token (e.g. 'messagesopt' "wait:") names its flag without the trailing colon.
    add(
      ('  kOpt%sFlag%s = 0x%02x,'):format(
        opt_name,
        lowercase_to_titlecase((f.name:gsub(':$', ''))),
        f.bit
      )
    )
  end
  add(('} Opt%sFlags;'):format(opt_name))
  return table.concat(out, '\n')
end

--- @param output_file string
--- @return table<string,string> options_index Map of option name to option index
local function gen_enums(output_file)
  --- Options for each scope.
  --- @type table<string, vim.option_meta[]>
  local scope_options = {}
  for _, scope in ipairs(valid_scopes) do
    scope_options[scope] = {}
  end

  local fd = assert(io.open(output_file, 'w'))

  --- @param s string
  local function write(s)
    fd:write(s)
    fd:write('\n')
  end

  -- Generate options enum file
  write('// IWYU pragma: private, include "nvim/option_defs.h"')
  write('')

  --- Map of option name to option index
  --- @type table<string, string>
  local option_index = {}

  -- Generate option index enum and populate the `option_index` and `scope_option` dicts.
  write('typedef enum {')
  write('  kOptInvalid = -1,')

  for i, o in ipairs(options_meta) do
    local enum_val_name = 'kOpt' .. lowercase_to_titlecase(o.full_name)
    write(('  %s = %u,'):format(enum_val_name, i - 1))

    option_index[o.full_name] = enum_val_name

    if o.abbreviation then
      option_index[o.abbreviation] = enum_val_name
    end

    local alias = o.alias or {} --[[@as string[] ]]
    for _, v in ipairs(alias) do
      option_index[v] = enum_val_name
    end

    for _, scope in ipairs(o.scope) do
      table.insert(scope_options[scope], o)
    end
  end

  write('  // Option count')
  write('#define kOptCount ' .. tostring(#options_meta))
  write('} OptIndex;')

  -- Generate option index enum for each scope
  for _, scope in ipairs(valid_scopes) do
    write('')

    local scope_name = lowercase_to_titlecase(scope)
    write('typedef enum {')
    write(('  %s = -1,'):format(get_scope_option(scope, 'Invalid')))

    for idx, option in ipairs(scope_options[scope]) do
      write(('  %s = %u,'):format(get_scope_option(scope, option.full_name), idx - 1))
    end

    write(('  // %s option count'):format(scope_name))
    write(('#define %s %d'):format(get_scope_option(scope, 'Count'), #scope_options[scope]))
    write(('} %sOptIndex;'):format(scope_name))
  end

  -- Generate reverse lookup from option scope index to option index for each scope.
  for _, scope in ipairs(valid_scopes) do
    write('')
    write(('EXTERN const OptIndex %s_opt_idx[] INIT( = {'):format(scope))
    for _, option in ipairs(scope_options[scope]) do
      local idx = option_index[option.full_name]
      write(('  [%s] = %s,'):format(get_scope_option(scope, option.full_name), idx))
    end
    write('});')
  end

  fd:close()
end

-- Maps a `dict` key's value-kind to its OptSchemaKind enum. Enum keys carry a values array instead.
local schema_kinds = {
  num = 'kOptSchemaNum',
  snum = 'kOptSchemaSNum',
  str = 'kOptSchemaStr',
}

--- Emit the OptSchemaItem[] that opt_strings_check() validates against, from a `dict` schema.
--- @param prefix string  e.g. "opt_dip"
--- @param schema vim.option_schema
--- @return string
local function gen_opt_schema(prefix, schema)
  local out = {} --- @type string[]
  local function add(s)
    table.insert(out, s)
  end

  add('')
  add(('EXTERN const OptSchemaItem %s_schema[] INIT( = {'):format(prefix))
  for _, item in ipairs(schema.dict) do
    if type(item) == 'string' then
      add(('  { "%s", kOptSchemaFlag, NULL },'):format(item))
    elseif item[2] == 'enum' then
      add(('  { "%s", kOptSchemaEnum, %s_%s_values },'):format(item[1], prefix, item[1]))
    else
      local kind = schema_kinds[item[2]] or error(('%s: bad kind %q'):format(prefix, item[2]))
      add(('  { "%s", %s, NULL },'):format(item[1], kind))
    end
  end
  add('  { NULL, kOptSchemaFlag, NULL },')
  add('});')

  return table.concat(out, '\n')
end

--- Emit the completion values arrays (`opt_<name>_values`, plus `opt_<name>_<key>_values` for each
--- `dict` enum key). Enum/set/flag tokens complete as-is; `dict` keys complete as "key:".
--- @param prefix string  e.g. "opt_dip"
--- @param schema vim.option_schema
--- @return string
local function gen_schema_values(prefix, schema)
  local out = {} --- @type string[]
  local function add(s)
    table.insert(out, s)
  end

  local values = schema_values(schema)
  add('')
  add(('EXTERN const char *(%s_values[%s]) INIT( = {'):format(prefix, #values + 1))
  for _, v in ipairs(values) do
    add(('  "%s",'):format(v))
  end
  add('  NULL')
  add('});')

  for _, item in ipairs(schema.dict or {}) do
    if type(item) == 'table' and item[2] == 'enum' then
      local vals = item[3].values --[[ @as string[] ]]
      add('')
      add(('EXTERN const char *(%s_%s_values[%s]) INIT( = {'):format(prefix, item[1], #vals + 1))
      for _, v in ipairs(vals) do
        add(('  "%s",'):format(v))
      end
      add('  NULL')
      add('});')
    end
  end

  return table.concat(out, '\n')
end

--- @param output_file string
local function gen_vars(output_file)
  local fd = assert(io.open(output_file, 'w'))

  --- @param s string
  local function write(s)
    fd:write(s)
    fd:write('\n')
  end

  write('// IWYU pragma: private, include "nvim/option_vars.h"')

  -- Generate enums for option flags.
  for _, o in ipairs(options_meta) do
    if o.schema and (o.schema.flags or o.schema.flagchars) then
      write(gen_token_enum(o))
    end
  end

  -- Generate valid values (for completion) and, for `dict` schemas, the validation schema.
  for _, option in ipairs(options_meta) do
    local values_var = ('opt_%s'):format(option.abbreviation or option.full_name)
    -- `chars` dispatch tables and `flagchars` (gen_token_enum) have no completion
    -- values; every other schema does.
    if option.schema and #schema_values(option.schema) > 0 then
      write(gen_schema_values(values_var, option.schema))
      if is_dict_option(option) then
        write(gen_opt_schema(values_var, option.schema))
      end
    end
  end

  fd:close()
end

local function main()
  local options_enum_file = arg[1]
  local option_vars_file = arg[2]

  gen_enums(options_enum_file)
  gen_vars(option_vars_file)
end

main()
