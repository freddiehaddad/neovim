#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/buffer_defs.h"  // IWYU pragma: keep

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

/// Opens a new split window, floating window, or external window.
///
/// @param buf Buffer to display, or 0 for current buffer
/// @param enter  Enter the window (make it the current window)
/// @param config Map defining the window configuration.
/// @param[out] err Error details, if any
/// @return |window-ID|, or 0 on error
DLLEXPORT Window nvim_open_win(Buffer buf, Boolean enter, Dict(win_config) *config, Error *err)
  FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;

/// Reconfigures the layout and properties of a window.
///
/// @param      win  |window-ID|, or 0 for current window
/// @param      config  Map defining the window configuration, see [nvim_open_win()]
/// @param[out] err     Error details, if any
DLLEXPORT void nvim_win_set_config(Window win, Dict(win_config) *config, Error *err)
  FUNC_API_SINCE(6);

/// Gets window config as a dict which can be passed to |nvim_open_win()| as the `config` parameter.
///
/// @param      win |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return     Map defining the window configuration, see |nvim_open_win()|
DLLEXPORT Dict(win_config) nvim_win_get_config(Window win, Arena *arena, Error *err)
  FUNC_API_SINCE(6);

DLLEXPORT void parse_bordertext(Object bordertext, BorderTextType bordertext_type,
                                WinConfig *fconfig, Error *err);
DLLEXPORT void parse_border_style(Object style, WinConfig *fconfig, Error *err);
DLLEXPORT bool parse_winborder(WinConfig *fconfig, char *border_opt, Error *err);

#include "nvim/func_attr.h"
