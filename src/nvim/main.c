#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#ifdef ENABLE_ASAN_UBSAN
# include <sanitizer/asan_interface.h>
# ifndef MSWIN
#  include <sanitizer/ubsan_interface.h>
# endif
#endif

#include "auto/config.h"  // IWYU pragma: keep
#include "auto/pathdef.h"
#include "auto/versiondef.h"
#include "auto/versiondef_git.h"
#include "nvim/api/private/helpers.h"
#include "nvim/lua/executor.h"
#include "nvim/macros_defs.h"
#include "nvim/main.h"
#include "nvim/os/os_defs.h"

#ifdef MSWIN
# ifndef _UCRT
#  error UCRT is the only supported C runtime on windows
# endif
#endif

typedef struct {
  char *name;
  const uint8_t *data;
  size_t size;
} ModuleDef;

#include "api/private/api_metadata.generated.h"
#include "lua/vim_module.generated.h"
#include "main.c.generated.h"

// for ":version", ":intro", and "nvim --version"
#ifndef NVIM_VERSION_MEDIUM
# define NVIM_VERSION_MEDIUM "v" STR(NVIM_VERSION_MAJOR) \
  "." STR(NVIM_VERSION_MINOR) "." STR(NVIM_VERSION_PATCH) \
  NVIM_VERSION_PRERELEASE
#endif
#define NVIM_VERSION_LONG "NVIM " NVIM_VERSION_MEDIUM  // NOLINT(bugprone-suspicious-missing-comma)

char *longVersion = NVIM_VERSION_LONG;
char *version_buildtype = "Build type: " NVIM_VERSION_BUILD_TYPE;
#ifndef NDEBUG
char *version_cflags = "Compilation: " NVIM_VERSION_CFLAGS;
#else
char *version_cflags = NULL;
#endif
#ifdef SYS_VIMRC_FILE
const char *version_sys_vimrc_file = SYS_VIMRC_FILE;
#else
const char *version_sys_vimrc_file = NULL;
#endif
const int nvim_version_major = NVIM_VERSION_MAJOR;
const int nvim_version_minor = NVIM_VERSION_MINOR;
const int nvim_version_patch = NVIM_VERSION_PATCH;

#ifdef EXITFREE
const bool main_exitfree = true;
#else
const bool main_exitfree = false;
#endif

const char *const eval_has_list[] = {
#ifdef __ANDROID__
  "android",
#endif
#if defined(BSD) && !defined(__APPLE__) && !defined(__GNU__)
  "bsd",
#endif
#ifdef __GNU__
  "hurd",
#endif
#ifdef __linux__
  "linux",
#endif
#ifdef SUN_SYSTEM
  "sun",
#endif
#ifdef __TERMUX__
  "termux",
#endif
#ifdef UNIX
  "unix",
#endif
#ifdef MSWIN
  "win32",
#endif
#ifdef _WIN64
  "win64",
#endif
#ifndef CASE_INSENSITIVE_FILENAME
  "fname_case",
#endif
#ifdef HAVE_ACL
  "acl",
#endif
  "autochdir",
  "arabic",
  "autocmd",
  "browsefilter",
  "byte_offset",
  "cindent",
  "cmdline_compl",
  "cmdline_hist",
  "cmdwin",
  "comments",
  "conceal",
  "cursorbind",
  "cursorshape",
  "dialog_con",
  "diff",
  "digraphs",
  "eval",
  "ex_extra",
  "extra_search",
  "file_in_path",
  "filterpipe",
  "find_in_path",
  "float",
  "folding",
#ifdef UNIX
  "fork",
#endif
  "gettext",
  "iconv",
  "insert_expand",
  "jumplist",
  "keymap",
  "lambda",
  "langmap",
  "libcall",
  "linebreak",
  "lispindent",
  "listcmds",
  "localmap",
#ifdef __APPLE__
  "mac",
  "macunix",
  "osx",
  "osxdarwin",
#endif
  "menu",
  "mksession",
  "modify_fname",
  "mouse",
  "multi_byte",
  "multi_lang",
  "nanotime",
  "num64",
  "packages",
  "path_extra",
  "persistent_undo",
  "profile",
  "reltime",
  "quickfix",
  "rightleft",
  "scrollbind",
  "showcmd",
  "cmdline_info",
  "shada",
  "signs",
  "smartindent",
  "startuptime",
  "statusline",
  "spell",
  "syntax",
#ifndef UNIX
  "system",
#endif
  "tablineat",
  "tag_binary",
  "termguicolors",
#ifdef HAVE_UNIBILIUM
  "terminfo",
#endif
  "termresponse",
  "textobjects",
  "timers",
  "title",
  "user-commands",
  "user_commands",
  "vartabs",
  "vertsplit",
  "vimscript-1",
  "virtualedit",
  "visual",
  "visualextra",
  "vreplace",
  "wildignore",
  "wildmenu",
  "windows",
  "winaltkeys",
  "writebackup",
#ifdef HAVE_XATTR
  "xattr",
#endif
  "nvim",
  NULL,
};

const void *nlua_get_builtin_modules(size_t *count)
{
  *count = ARRAY_SIZE(builtin_modules);
  return builtin_modules;
}

const uint8_t *api_get_packed_metadata(size_t *len)
{
  *len = sizeof(packed_api_metadata);
  return packed_api_metadata;
}

#ifndef MAKE_LIB
int main(int argc, char **argv)
{
  return nvim_main(argc, argv);
}
#endif

#ifdef ENABLE_ASAN_UBSAN
const char *__ubsan_default_options(void)
{
  return "print_stacktrace=1";
}

const char *__asan_default_options(void)
{
  return "handle_abort=1,handle_sigill=1";
}
#endif
