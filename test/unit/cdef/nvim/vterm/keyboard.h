#pragma once

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

DLLEXPORT void vterm_keyboard_unichar(VTerm *vt, uint32_t c, VTermModifier mod);
DLLEXPORT void vterm_keyboard_key(VTerm *vt, VTermKey key, VTermModifier mod);
DLLEXPORT void vterm_keyboard_start_paste(VTerm *vt);
DLLEXPORT void vterm_keyboard_end_paste(VTerm *vt);

#include "nvim/func_attr.h"
