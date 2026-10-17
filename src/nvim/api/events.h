#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep

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
DLLEXPORT void nvim_error_event(uint64_t channel_id, Integer type, String msg) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_ui_term_event(uint64_t channel_id, String event, Object value,
                                  Error *err) FUNC_API_SINCE(12) FUNC_API_REMOTE_ONLY;
#include "nvim/func_attr.h"
