#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/vterm/vterm_defs.h"
#include "nvim/vterm/vterm_keycodes_defs.h"

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

DLLEXPORT void vterm_mouse_move(VTerm *vt, int row, int col, VTermModifier mod);
DLLEXPORT void vterm_mouse_button(VTerm *vt, int button, bool pressed, VTermModifier mod);

#include "nvim/func_attr.h"
