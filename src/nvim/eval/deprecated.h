#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

void f_rpcstart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_rpcstop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_last_buffer_nr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_termopen(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
