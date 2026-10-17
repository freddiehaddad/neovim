#pragma once

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/normal_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

typedef int (*IndentGetter)(void);

/// flags for set_indent()
enum {
  SIN_CHANGED = 1,  ///< call changed_bytes() when line changed
  SIN_INSERT  = 2,  ///< insert indent before existing text
  SIN_UNDO    = 4,  ///< save line for undo before changing it
  SIN_NOMARK  = 8,  ///< don't adjust extmarks
};

typedef int (*Indenter)(void);

#include "nvim/buffer_defs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/func_attr.h"
#include "nvim/option_defs.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT const char *tabstop_set(char *var, colnr_T **array, const CharBuf *errbuf);
DLLEXPORT int tabstop_padding(colnr_T col, OptInt ts_arg, const colnr_T *vts) FUNC_ATTR_PURE;
DLLEXPORT int tabstop_at(colnr_T col, OptInt ts, const colnr_T *vts, bool left);
DLLEXPORT colnr_T tabstop_start(colnr_T col, int ts, colnr_T *vts);
DLLEXPORT void tabstop_fromto(colnr_T start_col, colnr_T end_col, int ts_arg, const colnr_T *vts,
                              int *ntabs, int *nspcs);
DLLEXPORT int *tabstop_copy(const int *oldts);
DLLEXPORT int tabstop_count(colnr_T *ts);
DLLEXPORT int tabstop_first(colnr_T *ts);
DLLEXPORT int get_sw_value(buf_T *buf);
DLLEXPORT int get_sw_value_indent(buf_T *buf, bool left);
DLLEXPORT int get_sw_value_col(buf_T *buf, colnr_T col, bool left);
DLLEXPORT int get_sts_value(void);
DLLEXPORT int get_indent(void);
DLLEXPORT int get_indent_lnum(linenr_T lnum);
DLLEXPORT int get_indent_buf(buf_T *buf, linenr_T lnum);
DLLEXPORT int indent_size_no_ts(char const *ptr) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE;
DLLEXPORT int indent_size_ts(char const *ptr, OptInt ts, colnr_T *vts)
  FUNC_ATTR_NONNULL_ARG(1) FUNC_ATTR_PURE;
DLLEXPORT bool set_indent(int size, int flags);
DLLEXPORT int get_number_indent(linenr_T lnum);
DLLEXPORT void briopt_check(win_T *wp);
DLLEXPORT int get_breakindent_win(win_T *wp, char *line) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool inindent(int extra);
DLLEXPORT void op_reindent(oparg_T *oap, Indenter how);
DLLEXPORT bool preprocs_left(void);
DLLEXPORT bool may_do_si(void);
DLLEXPORT void ins_try_si(int c);
DLLEXPORT void change_indent(int type, int amount, int round, bool call_changed_bytes);
DLLEXPORT bool copy_indent(int size, char *src);
DLLEXPORT void ex_retab(exarg_T *eap);
DLLEXPORT int get_expr_indent(void);
DLLEXPORT int get_lisp_indent(void);
DLLEXPORT void fixthisline(IndentGetter get_the_indent);
DLLEXPORT bool use_indentexpr_for_lisp(void);
DLLEXPORT void fix_indent(void);
DLLEXPORT void f_indent(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_lispindent(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
