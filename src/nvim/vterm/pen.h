#pragma once

#include <stddef.h>

#include "nvim/vterm/vterm_defs.h"

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

DLLEXPORT void vterm_state_newpen(VTermState *state);
DLLEXPORT void vterm_state_resetpen(VTermState *state);
DLLEXPORT void vterm_state_savepen(VTermState *state, int save);
DLLEXPORT void vterm_state_set_default_colors(VTermState *state, const VTermColor *default_fg,
                                              const VTermColor *default_bg);
DLLEXPORT void vterm_state_set_palette_color(VTermState *state, int index, const VTermColor *col);
DLLEXPORT void vterm_state_convert_color_to_rgb(const VTermState *state, VTermColor *col);
DLLEXPORT void vterm_state_setpen(VTermState *state, const long args[], int argcount);
DLLEXPORT int vterm_state_getpen(VTermState *state, long args[], int argcount);
DLLEXPORT int vterm_state_set_penattr(VTermState *state, VTermAttr attr, VTermValueType type,
                                      VTermValue *val);

#include "nvim/func_attr.h"
