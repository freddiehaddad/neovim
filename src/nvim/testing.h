#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

void f_assert_beeps(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_nobeep(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_equal(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_equalfile(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_notequal(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_exception(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_fails(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_false(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_inrange(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_match(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_notmatch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_report(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_assert_true(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_test_garbagecollect_now(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_test_write_list_log(typval_T *const argvars, typval_T *const rettv, EvalFuncData fptr);
