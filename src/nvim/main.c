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

const uint32_t nvim_build_has_flags =
#ifdef HAVE_ACL
  (1U << 0) |
#endif
#ifdef HAVE_UNIBILIUM
  (1U << 1) |
#endif
#ifdef HAVE_XATTR
  (1U << 2) |
#endif
#ifdef __TERMUX__
  (1U << 3) |
#endif
  0U;

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
