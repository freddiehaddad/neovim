#pragma once

#include "nvim/buffer_defs.h"
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int do_digraph(int c);
DLLEXPORT char *get_digraph_for_char(int val_arg);
DLLEXPORT int get_digraph(bool cmdline);
DLLEXPORT int digraph_get(int char1, int char2, bool meta_char) FUNC_ATTR_PURE;
DLLEXPORT bool check_digraph_chars_valid(int char1, int char2);
DLLEXPORT void putdigraph(char *str);
DLLEXPORT void listdigraphs(bool use_headers);
DLLEXPORT void digraph_getlist_common(bool list_all, typval_T *rettv);
DLLEXPORT void f_digraph_get(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_digraph_getlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_digraph_set(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_digraph_setlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT char *keymap_init(void);
DLLEXPORT void ex_loadkeymap(exarg_T *eap);
DLLEXPORT void keymap_ga_clear(garray_T *kmap_ga);
DLLEXPORT int get_keymap_str(win_T *wp, char *fmt, char *buf, int len);
