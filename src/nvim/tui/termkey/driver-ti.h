#pragma once

#include "nvim/tui/termkey/termkey_defs.h"  // IWYU pragma: keep

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
DLLEXPORT void *new_driver_ti(TermKey *tk, TerminfoEntry *term);
DLLEXPORT int start_driver_ti(TermKey *tk, void *info);
DLLEXPORT int stop_driver_ti(TermKey *tk, void *info);
DLLEXPORT void free_driver_ti(void *info);
DLLEXPORT TermKeyResult peekkey_ti(TermKey *tk, void *info, TermKeyKey *key, int force,
                                   size_t *nbytep);
#include "nvim/func_attr.h"
