#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
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

DLLEXPORT int check_linecomment(const char *line);
DLLEXPORT pos_T *find_start_comment(int ind_maxcomment);
DLLEXPORT int is_pos_in_string(const char *line, colnr_T col);
DLLEXPORT bool cin_is_cinword(const char *line);
DLLEXPORT bool cindent_on(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void parse_cino(buf_T *buf);
DLLEXPORT int get_c_indent(void);
DLLEXPORT bool in_cinkeys(int keytyped, int when, bool line_is_empty);
DLLEXPORT void do_c_expr_indent(void);
DLLEXPORT void f_cindent(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
