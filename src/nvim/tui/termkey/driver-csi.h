#pragma once

#include "nvim/tui/termkey/termkey_defs.h"  // IWYU pragma: keep

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
DLLEXPORT TermKeyResult termkey_interpret_mouse(TermKey *tk, const TermKeyKey *key,
                                                TermKeyMouseEvent *event, int *button, int *line,
                                                int *col);
DLLEXPORT TermKeyResult termkey_interpret_position(TermKey *tk, const TermKeyKey *key, int *line,
                                                   int *col);
DLLEXPORT TermKeyResult termkey_interpret_modereport(TermKey *tk, const TermKeyKey *key,
                                                     int *initial, int *mode, int *value);
DLLEXPORT TermKeyResult termkey_interpret_csi(TermKey *tk, const TermKeyKey *key,
                                              TermKeyCsiParam params[], size_t *nparams,
                                              unsigned *cmd);
DLLEXPORT TermKeyResult termkey_interpret_csi_param(TermKeyCsiParam param, int *paramp,
                                                    int subparams[], size_t *nsubparams);
DLLEXPORT void *new_driver_csi(TermKey *tk, TerminfoEntry *term);
DLLEXPORT void free_driver_csi(void *info);
DLLEXPORT TermKeyResult peekkey_csi(TermKey *tk, void *info, TermKeyKey *key, int force,
                                    size_t *nbytep);
#include "nvim/func_attr.h"
