#pragma once

#include <stdbool.h>
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

DLLEXPORT ScreenCell *getcell(const VTermScreen *screen, int row, int col);
DLLEXPORT void vterm_screen_free(VTermScreen *screen);
DLLEXPORT void vterm_screen_reset(VTermScreen *screen, int hard);
DLLEXPORT int vterm_screen_get_cell(const VTermScreen *screen, VTermPos pos, VTermScreenCell *cell);
DLLEXPORT VTermScreen *vterm_obtain_screen(VTerm *vt);
DLLEXPORT void vterm_screen_enable_reflow(VTermScreen *screen, bool reflow);
DLLEXPORT void vterm_screen_set_reflow(VTermScreen *screen, bool reflow);
DLLEXPORT void vterm_screen_enable_altscreen(VTermScreen *screen, int altscreen);
DLLEXPORT void vterm_screen_set_callbacks(VTermScreen *screen,
                                          const VTermScreenCallbacks *callbacks, void *user);
DLLEXPORT void vterm_screen_set_unrecognised_fallbacks(VTermScreen *screen,
                                                       const VTermStateFallbacks *fallbacks,
                                                       void *user);
DLLEXPORT void vterm_screen_flush_damage(VTermScreen *screen);
DLLEXPORT void vterm_screen_set_damage_merge(VTermScreen *screen, VTermDamageSize size);
DLLEXPORT void vterm_screen_convert_color_to_rgb(const VTermScreen *screen, VTermColor *col);
DLLEXPORT void rect_expand(VTermRect *dst, VTermRect *src);
DLLEXPORT void rect_clip(VTermRect *dst, VTermRect *bounds);
DLLEXPORT int rect_equal(VTermRect *a, VTermRect *b);
DLLEXPORT int rect_contains(VTermRect *big, VTermRect *small);
DLLEXPORT int rect_intersects(VTermRect *a, VTermRect *b);

#include "nvim/func_attr.h"
