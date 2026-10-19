#pragma once

#include <stdbool.h>

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"

#define DEFINE_FUNC_ATTRIBUTES
#include "nvim/func_attr.h"
#undef DEFINE_FUNC_ATTRIBUTES
#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

// defined in version.c
extern char *Versions[];
extern char *longVersion;
extern char *version_buildtype;
extern char *version_cflags;
extern const char *version_sys_vimrc_file;
extern const int nvim_version_major;
extern const int nvim_version_minor;
extern const int nvim_version_patch;

DLLEXPORT bool has_nvim_version(const char *version_str)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int min_vim_version(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int highest_patch(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool has_vim_patch(int n, int major_minor_version)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void ex_version(exarg_T *eap);
DLLEXPORT void list_in_columns(char **items, int size, int current);
DLLEXPORT void list_lua_version(void);
DLLEXPORT void list_version(void);
DLLEXPORT bool may_show_intro(void);
DLLEXPORT void intro_message(bool colon);
DLLEXPORT void ex_intro(exarg_T *eap);

#include "nvim/func_attr.h"

#include "version.h.inline.generated.h"
