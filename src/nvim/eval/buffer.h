#pragma once

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
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

DLLEXPORT buf_T *find_buffer(typval_T *avar);
DLLEXPORT void f_append(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_appendbufline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void prompt_invoke_callback(void);
DLLEXPORT bool invoke_prompt_interrupt(void);
DLLEXPORT void f_prompt_appendbuf(typval_T *argvars, typval_T *rettv, EvalFuncData fptr)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void f_bufadd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_bufexists(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_buflisted(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_bufload(typval_T *argvars, typval_T *unused, EvalFuncData fptr);
DLLEXPORT void f_bufloaded(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_bufname(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_bufnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_bufwinid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_bufwinnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_deletebufline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getbufinfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getbufline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getbufoneline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setbufline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_prompt_setcallback(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_prompt_setinterrupt(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_prompt_getprompt(typval_T *argvars, typval_T *rettv, EvalFuncData fptr)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void f_prompt_getinput(typval_T *argvars, typval_T *rettv, EvalFuncData fptr)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void f_prompt_setprompt(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
