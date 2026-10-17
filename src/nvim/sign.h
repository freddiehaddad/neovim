#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/sign_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT bool buf_has_signs(const buf_T *buf);
DLLEXPORT size_t describe_sign_text(char *buf, schar_T *sign_text);
DLLEXPORT int init_sign_text(sign_T *sp, schar_T *sign_text, char *text);
DLLEXPORT void ex_sign(exarg_T *eap);
DLLEXPORT list_T *get_buffer_signs(buf_T *buf)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void free_signs(void);
DLLEXPORT char *get_sign_name(expand_T *xp, int idx);
DLLEXPORT void set_context_in_sign_cmd(expand_T *xp, char *arg);
DLLEXPORT void f_sign_define(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_getdefined(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_getplaced(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_jump(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_place(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_placelist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_undefine(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_unplace(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sign_unplacelist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
