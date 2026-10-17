#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/state_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void state_enter(VimState *s) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void state_handle_k_event(void);
DLLEXPORT bool virtual_active(win_T *wp);
DLLEXPORT int get_real_state(void);
DLLEXPORT void get_mode(char *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void may_trigger_modechanged(void);
DLLEXPORT void may_trigger_safestate(bool safe);
DLLEXPORT void state_no_longer_safe(const char *reason);
DLLEXPORT bool get_was_safe_state(void);
