// Make sure extern symbols are exported on Windows
#ifdef WIN32
# define EXTERN __declspec(dllexport)
#else
# define EXTERN
#endif
#include <assert.h>
#include <lauxlib.h>  // IWYU pragma: keep
#include <lua.h>  // IWYU pragma: keep
#include <lualib.h>  // IWYU pragma: keep
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef ENABLE_ASAN_UBSAN
# include <sanitizer/asan_interface.h>
# ifndef MSWIN
#  include <sanitizer/ubsan_interface.h>
# endif
#endif

#include "auto/config.h"  // IWYU pragma: keep
#include "auto/pathdef.h"
#include "nvim/option_defs.h"
#include "nvim/option_vars.h"

#undef EXTERN
#undef INIT
#define EXTERN extern
#define INIT(...)

#include "auto/versiondef.h"
#include "auto/versiondef_git.h"
#include "nvim/api/private/defs.h"
#include "nvim/api/private/dispatch.h"
#include "nvim/api/private/helpers.h"
#include "nvim/ascii_defs.h"
#include "nvim/errors.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/event/loop.h"
#include "nvim/ex_docmd.h"  // IWYU pragma: keep
#include "nvim/ex_getln.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"  // IWYU pragma: keep
#include "nvim/globals.h"  // IWYU pragma: keep
#include "nvim/lua/converter.h"  // IWYU pragma: keep
#include "nvim/lua/executor.h"
#include "nvim/macros_defs.h"
#include "nvim/main.h"
#include "nvim/memory.h"
#include "nvim/memory_defs.h"
#include "nvim/msgpack_rpc/unpacker.h"
#include "nvim/os/os_defs.h"
#include "nvim/strings.h"
#include "nvim/types_defs.h"
#include "nvim/version.h"

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

typedef struct {
  va_list ap_start;
  va_list ap;
} VaState;

#include "api/private/api_metadata.generated.h"
#include "api/private/dispatch_wrappers.generated.h"
#include "lua/vim_module.generated.h"
#include "lua_api_c_bindings.generated.h"  // IWYU pragma: keep
#include "main.c.generated.h"

// for ":version", ":intro", and "nvim --version"
#ifndef NVIM_VERSION_MEDIUM
# define NVIM_VERSION_MEDIUM "v" STR(NVIM_VERSION_MAJOR) \
  "." STR(NVIM_VERSION_MINOR) "." STR(NVIM_VERSION_PATCH) \
  NVIM_VERSION_PRERELEASE
#endif
#define NVIM_VERSION_LONG "NVIM " NVIM_VERSION_MEDIUM  // NOLINT(bugprone-suspicious-missing-comma)

char *Versions[] = { "8.1", "8.2", "9.0", "9.1", "9.2" };
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

Loop main_loop;

#ifdef EXITFREE
const bool main_exitfree = true;
#else
const bool main_exitfree = false;
#endif

const void *nlua_get_builtin_modules(size_t *count)
{
  *count = ARRAY_SIZE(builtin_modules);
  return builtin_modules;
}

MsgpackRpcRequestHandler msgpack_rpc_get_handler_at(int hash)
{
  return method_handlers[hash];
}

const uint8_t *api_get_packed_metadata(size_t *len)
{
  *len = sizeof(packed_api_metadata);
  return packed_api_metadata;
}

void api_set_error(Error *err, ErrorType errType, const char *format, ...)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PRINTF(3, 4)
{
  assert(kErrorTypeNone != errType);
  va_list args1;
  va_list args2;
  va_start(args1, format);
  va_copy(args2, args1);
  int len = vsnprintf(NULL, 0, format, args1);
  va_end(args1);
  assert(len >= 0);
  // Limit error message to 1 MB.
  size_t bufsize = MIN((size_t)len + 1, 1024 * 1024);
  err->msg = xmalloc(bufsize);
  vsnprintf(err->msg, bufsize, format, args2);
  va_end(args2);

  err->type = errType;
}

int vim_vsnprintf_typval_rs(char *str, size_t str_m, const char *fmt, void *va_ptr,
                            typval_T *const tvs);
long long va_state_llong(void *va_ptr);  // NOLINT(runtime/int)
unsigned long va_state_ulong(void *va_ptr);  // NOLINT(runtime/int)
unsigned long long va_state_ullong(void *va_ptr);  // NOLINT(runtime/int)

void va_state_reset(void *va_ptr)
{
  VaState *va = va_ptr;
  va_end(va->ap);
  va_copy(va->ap, va->ap_start);
}

int va_state_int(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, int);
}

long va_state_long(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, long);  // NOLINT(runtime/int)
}

long long va_state_llong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, long long);  // NOLINT(runtime/int)
}

ptrdiff_t va_state_ptrdiff(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, ptrdiff_t);
}

unsigned va_state_uint(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned);
}

unsigned long va_state_ulong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned long);  // NOLINT(runtime/int)
}

unsigned long long va_state_ullong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned long long);  // NOLINT(runtime/int)
}

size_t va_state_size(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, size_t);
}

const char *va_state_str(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, const char *);
}

void *va_state_ptr(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, void *);
}

double va_state_double(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, double);
}

void va_report_null_string(void)
{
#ifdef ENABLE_ASAN_UBSAN
  __sanitizer_print_stack_trace();
  __sanitizer_report_error_summary("SUMMARY: NULL argument passed to %s");
#endif
}

int vim_snprintf(char *str, size_t str_m, const char *fmt, ...)
  FUNC_ATTR_PRINTF(3, 4)
{
  va_list ap;
  va_start(ap, fmt);
  VaState va;
  va_copy(va.ap_start, ap);
  va_copy(va.ap, ap);
  const int str_l = vim_vsnprintf_typval_rs(str, str_m, fmt, &va, NULL);
  va_end(va.ap);
  va_end(va.ap_start);
  va_end(ap);
  return str_l;
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
