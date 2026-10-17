#pragma once

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

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

DLLEXPORT void clear_matches(win_T *wp);
DLLEXPORT void init_search_hl(win_T *wp, match_T *search_hl) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void prepare_search_hl(win_T *wp, match_T *search_hl,
                                 linenr_T lnum) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool prepare_search_hl_line(win_T *wp, linenr_T lnum, colnr_T mincol, char **line,
                                      match_T *search_hl, int *search_attr,
                                      bool *search_attr_from_match);
DLLEXPORT int update_search_hl(win_T *wp, linenr_T lnum, colnr_T col, char **line,
                               match_T *search_hl, int *has_match_conc, int *match_conc,
                               bool lcs_eol_todo, bool *on_last_col, bool *search_attr_from_match);
DLLEXPORT bool get_prevcol_hl_flag(win_T *wp, match_T *search_hl, colnr_T curcol);
DLLEXPORT void get_search_match_hl(win_T *wp, match_T *search_hl, colnr_T col, int *char_attr);
DLLEXPORT void f_clearmatches(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getmatches(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setmatches(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchadd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchaddpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matcharg(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchdelete(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void ex_match(exarg_T *eap);

#include "nvim/func_attr.h"
