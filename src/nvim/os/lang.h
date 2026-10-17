#pragma once

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT char *get_mess_lang(void);
DLLEXPORT void set_lang_var(void);
DLLEXPORT void init_locale(void);
DLLEXPORT void ex_language(exarg_T *eap);
DLLEXPORT void free_locales(void);
DLLEXPORT char *get_lang_arg(expand_T *xp, int idx);
DLLEXPORT char *get_locales(expand_T *xp, int idx);
DLLEXPORT void lang_init(void);
