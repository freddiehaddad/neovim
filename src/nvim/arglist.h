#pragma once

#include "nvim/arglist_defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep

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

DLLEXPORT void alist_clear(alist_T *al);
DLLEXPORT void alist_init(alist_T *al);
DLLEXPORT void alist_unlink(alist_T *al);
DLLEXPORT void alist_new(void);
DLLEXPORT void alist_expand(int *fnum_list, int fnum_len);
DLLEXPORT void alist_set(alist_T *al, int count, char **files, int use_curbuf, int *fnum_list,
                         int fnum_len);
DLLEXPORT void alist_add(alist_T *al, char *fname, int set_fnum);
DLLEXPORT int get_arglist_exp(char *str, int *fcountp, char ***fnamesp, bool wig);
DLLEXPORT void set_arglist(char *str);
DLLEXPORT bool editing_arg_idx(win_T *win);
DLLEXPORT void check_arg_idx(win_T *win);
DLLEXPORT void ex_args(exarg_T *eap);
DLLEXPORT void ex_previous(exarg_T *eap);
DLLEXPORT void ex_rewind(exarg_T *eap);
DLLEXPORT void ex_last(exarg_T *eap);
DLLEXPORT void ex_argument(exarg_T *eap);
DLLEXPORT void do_argfile(exarg_T *eap, int argn);
DLLEXPORT void ex_next(exarg_T *eap);
DLLEXPORT void ex_argdedupe(exarg_T *eap FUNC_ATTR_UNUSED);
DLLEXPORT void ex_argedit(exarg_T *eap);
DLLEXPORT void ex_argadd(exarg_T *eap);
DLLEXPORT void ex_argdelete(exarg_T *eap);
DLLEXPORT char *get_arglist_name(expand_T *xp FUNC_ATTR_UNUSED, int idx);
DLLEXPORT char *alist_name(aentry_T *aep);
DLLEXPORT void ex_all(exarg_T *eap);
DLLEXPORT char *arg_all(void);
DLLEXPORT void f_argc(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_argidx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_arglistid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_argv(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
