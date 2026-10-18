#pragma once

#include <stdbool.h>

#include "nvim/garray_defs.h"  // IWYU pragma: keep

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

DLLEXPORT int spell_check_sps(const char *value, int *ret_flags, int *ret_limit);
DLLEXPORT void spell_apply_sps(int flags, int limit);
DLLEXPORT void spell_suggest(int count);
DLLEXPORT void spell_suggest_list(garray_T *gap, char *word, int maxcount, bool need_cap,
                                  bool interactive);

#include "nvim/func_attr.h"
