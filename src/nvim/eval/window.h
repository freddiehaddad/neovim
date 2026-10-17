#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/os/os_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"

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

DLLEXPORT bool win_has_winnr(win_T *wp, tabpage_T *tp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT win_T *win_find_nr(int nr, tabpage_T *tp);
DLLEXPORT win_T *win_id2wp(int id);
DLLEXPORT win_T *win_id2wp_tp(int id, tabpage_T **tpp);
DLLEXPORT void win_findbuf(typval_T *argvars, list_T *list);
DLLEXPORT win_T *find_win_by_nr(typval_T *vp, tabpage_T *tp);
DLLEXPORT win_T *find_win_by_nr_or_id(typval_T *vp);
DLLEXPORT win_T *find_tabwin(typval_T *wvp, typval_T *tvp);
DLLEXPORT void f_gettabinfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getwininfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getwinpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getwinposx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getwinposy(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_tabpagenr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_tabpagewinnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_execute(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_findbuf(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_getid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_gotoid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_id2tabwin(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_id2win(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_move_separator(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_move_statusline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_screenpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_splitmove(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_win_gettype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdwintype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winbufnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_wincol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winheight(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winlayout(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winrestcmd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winrestview(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winsaveview(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_winwidth(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
