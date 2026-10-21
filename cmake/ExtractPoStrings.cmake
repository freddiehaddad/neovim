set(EXTRACT_LUA "${CMAKE_CURRENT_BINARY_DIR}/extract_po_strings.lua")
file(WRITE "${EXTRACT_LUA}" [=[
local src_dir, out_file = arg[1], arg[2]
local out = assert(io.open(out_file, 'w'))

local function parse_str_at(text, quote_pos)
  local i = quote_pos + 1
  local n = #text
  while i <= n do
    local b = text:byte(i)
    if b == 92 then -- backslash
      i = i + 2
    elseif b == 34 then -- closing quote
      return text:sub(quote_pos + 1, i - 1), i + 1
    else
      i = i + 1
    end
  end
  return nil, i
end

local rs_files = vim.fs.find(function(name)
  return name:match('%.rs$') ~= nil
end, { path = src_dir, limit = math.huge, type = 'file' })
table.sort(rs_files)

for _, path in ipairs(rs_files) do
  local f = assert(io.open(path, 'r'))
  local text = f:read('*a')
  f:close()

  if path:match('[/\\]errors%.rs$') then
    local pos = 1
    while true do
      local _, q = text:find('pub static e_[%w_]+:%s*%[c_char;%s*%d+%]%s*=%s*%*b"', pos)
      if not q then
        break
      end
      local s, next_pos = parse_str_at(text, q)
      if s then
        s = s:gsub('\\0$', '')
        out:write(('N_("%s");\n'):format(s))
      end
      pos = next_pos
    end
  end

  local pos = 1
  while true do
    local s_idx, _, fn_name, q_pos = text:find('([_%w]+)%s*%(%s*c()"', pos)
    if not s_idx then
      break
    end
    local s, next_pos = parse_str_at(text, q_pos)
    if s then
      if fn_name == 'gettext' or fn_name == 'tr' or fn_name == '_' or fn_name == 'N_' then
        out:write(('%s("%s");\n'):format(fn_name == 'tr' and 'gettext' or fn_name, s))
      elseif fn_name == 'ngettext' or fn_name == 'NGETTEXT' then
        local _, _, q2 = text:find('^%s*%.as_ptr%(%)%s*,%s*c()"', next_pos)
        if not q2 then
          _, _, q2 = text:find('^%s*,%s*c()"', next_pos)
        end
        if q2 then
          local s2, next_pos2 = parse_str_at(text, q2)
          if s2 then
            out:write(('ngettext("%s", "%s", 1);\n'):format(s, s2))
            next_pos = next_pos2
          end
        end
      end
    end
    pos = next_pos
  end
end

out:close()
]=])

execute_process(
  COMMAND "${NVIM_BIN_FILE}" --clean -l "${EXTRACT_LUA}" "${SRC_DIR}" "${OUTPUT_FILE}"
  RESULT_VARIABLE res)
if(res)
  message(FATAL_ERROR "Failed to extract PO strings from Rust sources")
endif()
