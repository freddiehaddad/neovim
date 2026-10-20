#pragma once

#include <stddef.h>

#include "nvim/vterm/vterm_defs.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

extern char vterm_primary_device_attr[];

DLLEXPORT void vterm_state_free(VTermState *state);
DLLEXPORT VTermState *vterm_obtain_state(VTerm *vt);
DLLEXPORT void vterm_state_reset(VTermState *state, int hard);
DLLEXPORT void vterm_state_set_callbacks(VTermState *state, const VTermStateCallbacks *callbacks,
                                         void *user);
DLLEXPORT void vterm_state_set_unrecognised_fallbacks(VTermState *state,
                                                      const VTermStateFallbacks *fallbacks,
                                                      void *user);
DLLEXPORT int vterm_state_set_termprop(VTermState *state, VTermProp prop, VTermValue *val);
DLLEXPORT void vterm_state_focus_in(VTermState *state);
DLLEXPORT void vterm_state_focus_out(VTermState *state);
DLLEXPORT const VTermLineInfo *vterm_state_get_lineinfo(const VTermState *state, int row);
DLLEXPORT void vterm_state_set_selection_callbacks(VTermState *state,
                                                   const VTermSelectionCallbacks *callbacks,
                                                   void *user, char *buffer, size_t buflen);
