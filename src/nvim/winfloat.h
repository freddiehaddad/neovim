#pragma once

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"

/// NW -> 0
/// NE -> kFloatAnchorEast
/// SW -> kFloatAnchorSouth
/// SE -> kFloatAnchorSouth | kFloatAnchorEast
EXTERN const char *const float_anchor_str[] INIT( = { "NW", "NE", "SW", "SE" });

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

DLLEXPORT win_T *win_new_float(win_T *wp, bool last, WinConfig fconfig, Error *err);
DLLEXPORT void win_set_minimal_style(win_T *wp);
DLLEXPORT int win_border_height(win_T *wp);
DLLEXPORT int win_border_width(win_T *wp);
DLLEXPORT void win_config_float(win_T *wp, WinConfig fconfig);
DLLEXPORT void win_float_remove(bool bang, int count);
DLLEXPORT void win_check_anchored_floats(win_T *win);
DLLEXPORT void win_float_update_statusline(void);
DLLEXPORT void win_float_anchor_laststatus(void);
DLLEXPORT void win_reconfig_floats(void);
DLLEXPORT bool win_float_valid(const win_T *win) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT win_T *win_float_find(WinKind kind) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT win_T *win_float_find_altwin(const win_T *win, const tabpage_T *tp)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void win_float_anchor_preview(win_T *wp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT win_T *win_float_special(bool enter, bool new_buf, WinKind kind);
DLLEXPORT void win_float_close(WinKind kind);
DLLEXPORT void win_float_update_preview(win_T *wp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool win_previewpopup_config(const char *value, WinConfig *config) FUNC_ATTR_NONNULL_ALL;

#include "nvim/func_attr.h"
