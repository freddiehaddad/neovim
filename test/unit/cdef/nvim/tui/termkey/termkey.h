#pragma once

#include <stdint.h>  // IWYU pragma: keep
#include <stdlib.h>  // IWYU pragma: keep

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
DLLEXPORT TermKeyResult termkey_interpret_string(TermKey *tk, const TermKeyKey *key,
                                                 const char **strp);
DLLEXPORT TermKey *termkey_new_abstract(TerminfoEntry *term, int flags);
DLLEXPORT void termkey_free(TermKey *tk);
DLLEXPORT void termkey_destroy(TermKey *tk);
DLLEXPORT void termkey_hook_terminfo_getstr(TermKey *tk, TermKey_Terminfo_Getstr_Hook *hookfn,
                                            void *data);
DLLEXPORT int termkey_start(TermKey *tk);
DLLEXPORT int termkey_stop(TermKey *tk);
DLLEXPORT void termkey_set_flags(TermKey *tk, int newflags);
DLLEXPORT int termkey_get_canonflags(TermKey *tk);
DLLEXPORT void termkey_set_canonflags(TermKey *tk, int flags);
DLLEXPORT size_t termkey_get_buffer_size(TermKey *tk);
DLLEXPORT int termkey_set_buffer_size(TermKey *tk, size_t size);
DLLEXPORT size_t termkey_get_buffer_remaining(TermKey *tk);
DLLEXPORT int fill_utf8(int codepoint, char *str);
DLLEXPORT void termkey_canonicalise(TermKey *tk, TermKeyKey *key);
DLLEXPORT TermKeyResult termkey_getkey(TermKey *tk, TermKeyKey *key);
DLLEXPORT TermKeyResult termkey_getkey_force(TermKey *tk, TermKeyKey *key);
DLLEXPORT size_t termkey_push_bytes(TermKey *tk, const char *bytes, size_t len);
DLLEXPORT TermKeySym termkey_register_keyname(TermKey *tk, TermKeySym sym, const char *name);
DLLEXPORT const char *termkey_get_keyname(TermKey *tk, TermKeySym sym);
DLLEXPORT const char *termkey_lookup_keyname(TermKey *tk, const char *str, TermKeySym *sym);
DLLEXPORT size_t termkey_strfkey(TermKey *tk, char *buffer, size_t len, TermKeyKey *key,
                                 TermKeyFormat format);
#include "nvim/func_attr.h"
