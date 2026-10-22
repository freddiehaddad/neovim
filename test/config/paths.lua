local M = {}

local this_file = debug.getinfo(1, 'S').source:sub(2)
local real_file = vim.uv.fs_realpath(this_file) or this_file
local default_source_dir = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(real_file)))

M.translations_enabled = os.getenv('NVIM_TEST_TRANSLATIONS') == '1'
M.is_asan = os.getenv('NVIM_TEST_ASAN') == '1'
M.test_source_path = vim.fs.normalize(os.getenv('NVIM_TEST_SOURCE_DIR') or default_source_dir)
M.test_build_dir =
  vim.fs.normalize(os.getenv('NVIM_TEST_BUILD_DIR') or (M.test_source_path .. '/target/nvim-build'))
M.vterm_test_file =
  vim.fs.normalize(os.getenv('VTERM_TEST_FILE') or (M.test_build_dir .. '/test/vterm_test_output'))

local default_luajit = M.test_source_path
  .. '/.deps/usr/bin/luajit'
  .. (vim.fn.has('win32') == 1 and '.exe' or '')
if vim.uv.fs_stat(default_luajit) == nil then
  default_luajit = vim.fn.exepath('luajit')
end
M.test_lua_prg = vim.fs.normalize(os.getenv('NVIM_TEST_LUA_PRG') or default_luajit)
M.test_luajit_prg = M.test_lua_prg:find('luajit') and M.test_lua_prg or nil

return M
