#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"

/// Prototype of C function that implements Vimscript function
typedef void (*VimLFunc)(typval_T *args, typval_T *rvar, EvalFuncData data);

/// Special flags for base_arg @see EvalFuncDef
enum {
  BASE_NONE = 0,          ///< Not a method (no base argument).
  BASE_LAST = UINT8_MAX,  ///< Use the last argument as the method base.
};

/// Structure holding Vimscript function definition
typedef struct {
  char *name;         ///< Name of the function.
  uint8_t min_argc;   ///< Minimal number of arguments.
  uint8_t max_argc;   ///< Maximal number of arguments.
  uint8_t base_arg;   ///< Method base arg # (1-indexed), BASE_NONE or BASE_LAST.
  bool fast;          ///< Can be run in |api-fast| events
  VimLFunc func;      ///< Function implementation.
  EvalFuncData data;  ///< Userdata for function implementation.
} EvalFuncDef;

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

DLLEXPORT char *get_function_name(expand_T *xp, int idx);
DLLEXPORT char *get_expr_name(expand_T *xp, int idx);
DLLEXPORT const EvalFuncDef *find_internal_func(const char *const name)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *find_internal_func_lua(const char *const name)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int check_internal_func(const EvalFuncDef *const fdef, const int argcount)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int call_internal_func(const char *const fname, const int argcount,
                                 typval_T *const argvars, typval_T *const rettv)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int call_internal_method(const char *const fname, const int argcount,
                                   typval_T *const argvars, typval_T *const rettv,
                                   typval_T *const basetv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT buf_T *tv_get_buf(typval_T *tv, int curtab_only);
DLLEXPORT buf_T *tv_get_buf_from_arg(typval_T *const tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT buf_T *get_buf_arg(typval_T *arg);
DLLEXPORT win_T *get_optional_window(typval_T *argvars, int idx);
DLLEXPORT char *get_list_line(int c, void *cookie, int indent, bool do_concat);
DLLEXPORT void execute_common(typval_T *argvars, typval_T *rettv, int arg_off);
DLLEXPORT dict_T *create_environment(const dictitem_T *job_env, const bool clear_env,
                                     const bool pty, const bool set_nvim_addr,
                                     const char *const pty_term_name);
DLLEXPORT void f_jobstart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_jobstop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT int do_searchpair(const char *spat, const char *mpat, const char *epat, int dir,
                            const typval_T *skip, int flags, pos_T *match_pos, linenr_T lnum_stop,
                            int64_t time_limit) FUNC_ATTR_NONNULL_ARG(1, 2, 3);
DLLEXPORT const EvalFuncDef *eval_func_def_at(int idx);
DLLEXPORT void float_op_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void api_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void lua_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
