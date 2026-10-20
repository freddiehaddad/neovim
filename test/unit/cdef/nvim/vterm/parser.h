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

DLLEXPORT size_t vterm_input_write(VTerm *vt, const char *bytes, size_t len);
DLLEXPORT void vterm_parser_set_callbacks(VTerm *vt, const VTermParserCallbacks *callbacks,
                                          void *user);

#include "nvim/func_attr.h"
