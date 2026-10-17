#pragma once

#include "nvim/eval/typval_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

void f_filter(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_map(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_mapnew(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_foreach(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_add(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_count(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_extend(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_extendnew(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_insert(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_remove(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reverse(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
