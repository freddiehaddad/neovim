--  Updates version.rs list of applied Vim patches.
--
--  Usage:
--    VIM_SOURCE_DIR=~/neovim/.vim-src/ nvim -l ./scripts/vimpatch.lua

local nvim = vim.api

local function systemlist(...)
  local rv = nvim.nvim_call_function('systemlist', ...)
  local err = nvim.nvim_get_vvar('shell_error')
  local args_str = nvim.nvim_call_function('string', ...)
  if 0 ~= err then
    error('command failed: ' .. args_str)
  end
  return rv
end

local function vimpatch_sh_list_tokens()
  return systemlist({ { 'bash', '-c', 'scripts/vim-patch.sh -M' } })
end

-- Generate the lines to update src/nvim/version.rs:
-- - `VIM_VERSIONS`
-- - `NUM_PATCHES`
-- - `PATCHES_*`
-- - `INCLUDED_PATCHSETS`
-- - `Versions`
local function gen_version_rs_lines()
  -- List of version sets where each set contains:
  -- 1. major_minor_version (int)
  -- 2. major_minor_version (string)
  -- 3. set of merged patch numbers
  local merged_version_list = {}
  for _, token in ipairs(vimpatch_sh_list_tokens()) do
    local major_version, minor_version, patch_num = string.match(token, '^(%d+).(%d+).(%d+)$')
    local n = tonumber(patch_num)
    if n then
      local major_minor_version = major_version * 100 + minor_version
      local len = #merged_version_list
      if len == 0 or merged_version_list[len][1] ~= major_minor_version then
        local vstr = major_version .. '.' .. minor_version
        table.insert(merged_version_list, { major_minor_version, vstr, { n } })
      else
        table.insert(merged_version_list[len][3], n)
      end
    end
  end

  local major_vim_versions = {}
  local major_vim_versions_str = {}
  local num_patches = {}
  local patchset_names = {}
  local patch_lines = {}
  for _, version_set in ipairs(merged_version_list) do
    local major_minor_version, major_minor_version_str, patch_set = unpack(version_set)
    local patch_const = 'PATCHES_' .. major_minor_version
    table.insert(major_vim_versions, major_minor_version)
    table.insert(major_vim_versions_str, major_minor_version_str)
    table.insert(num_patches, #patch_set)
    table.insert(patchset_names, patch_const)
    table.insert(patch_lines, '#[rustfmt::skip]')
    table.insert(patch_lines, 'static ' .. patch_const .. ': &[c_int] = &[')

    local patchset_set = {}
    for i = #patch_set, 1, -1 do
      local patch = patch_set[i]
      local next_patch = patch_set[i - 1]
      local patch_diff = patch - (next_patch or 0)
      table.insert(patchset_set, patch)

      if #patchset_set > 15 or i == 1 or patch_diff > 1 then
        table.insert(patch_lines, '    ' .. table.concat(patchset_set, ', ') .. ',')
        patchset_set = {}
      end
      if i == 1 and patch > 0 then
        local line = '    // 0'
        if patch > 1 then
          line = line .. '-' .. (patch - 1)
        end
        table.insert(patch_lines, line)
      elseif patch_diff > 1 then
        local line = '    // ' .. (next_patch + 1)
        if patch_diff > 2 then
          line = line .. '-' .. (patch - 1)
        end
        table.insert(patch_lines, line)
      end
    end

    table.insert(patch_lines, '];')
  end

  local count = #major_vim_versions
  local lines = {
    string.format(
      'static VIM_VERSIONS: [c_int; %d] = [%s];',
      count,
      table.concat(major_vim_versions, ', ')
    ),
    string.format(
      'static NUM_PATCHES: [usize; %d] = [%s];',
      count,
      table.concat(num_patches, ', ')
    ),
  }
  vim.list_extend(lines, patch_lines)
  table.insert(lines, string.format('static INCLUDED_PATCHSETS: [&[c_int]; %d] = [', count))
  for _, name in ipairs(patchset_names) do
    table.insert(lines, '    ' .. name .. ',')
  end
  table.insert(lines, '];')
  table.insert(lines, '')
  table.insert(lines, '#[unsafe(no_mangle)]')
  table.insert(lines, string.format('pub static mut Versions: [*mut c_char; %d] = [', count))
  for _, vstr in ipairs(major_vim_versions_str) do
    table.insert(lines, string.format('    c"%s".as_ptr().cast_mut(),', vstr))
  end
  table.insert(lines, '];')

  return lines
end

local function patch_version_rs()
  local lines = gen_version_rs_lines()

  nvim.nvim_command('silent noswapfile noautocmd edit src/nvim/version.rs')
  nvim.nvim_command([[/^static VIM_VERSIONS:]])
  local start_lnum = nvim.nvim_eval('line(".")')
  nvim.nvim_command([[/^pub static mut Versions:]])
  nvim.nvim_command([[/^];]])
  local end_lnum = nvim.nvim_eval('line(".")')
  nvim.nvim_buf_set_lines(0, start_lnum - 1, end_lnum, true, lines)
  nvim.nvim_command('silent write')
end

patch_version_rs()
