#pragma once

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// flags for skip_vimgrep_pat()
enum {
  VGR_GLOBAL = 1,
  VGR_NOJUMP = 2,
  VGR_FUZZY  = 4,
};

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

DLLEXPORT int qf_init(win_T *wp, const char *restrict efile, char *restrict errorformat,
                      int newlist, const char *restrict qf_title, char *restrict enc);
DLLEXPORT int qf_stack_get_bufnr(void);
DLLEXPORT void qf_free_all(win_T *wp);
DLLEXPORT void check_quickfix_busy(void);
DLLEXPORT void qf_resize_stack(int n);
DLLEXPORT void ll_resize_stack(win_T *wp, int n);
DLLEXPORT void qf_init_stack(void);
DLLEXPORT void copy_loclist_stack(win_T *from, win_T *to) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void qf_jump(qf_info_T *qi, int dir, int errornr, int forceit);
DLLEXPORT void qf_list(exarg_T *eap);
DLLEXPORT void qf_age(exarg_T *eap);
DLLEXPORT void qf_history(exarg_T *eap);
DLLEXPORT bool qf_mark_adjust(buf_T *buf, win_T *wp, linenr_T line1, linenr_T line2,
                              linenr_T amount, linenr_T amount_after);
DLLEXPORT void qf_view_result(bool split);
DLLEXPORT void ex_cwindow(exarg_T *eap);
DLLEXPORT void ex_cclose(exarg_T *eap);
DLLEXPORT void ex_copen(exarg_T *eap);
DLLEXPORT void ex_cbottom(exarg_T *eap);
DLLEXPORT linenr_T qf_current_entry(win_T *wp);
DLLEXPORT int grep_internal(cmdidx_T cmdidx);
DLLEXPORT void ex_make(exarg_T *eap);
DLLEXPORT size_t qf_get_size(exarg_T *eap) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t qf_get_valid_size(exarg_T *eap);
DLLEXPORT size_t qf_get_cur_idx(exarg_T *eap) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int qf_get_cur_valid_idx(exarg_T *eap) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ex_cc(exarg_T *eap);
DLLEXPORT void ex_cnext(exarg_T *eap);
DLLEXPORT void ex_cbelow(exarg_T *eap);
DLLEXPORT void ex_cfile(exarg_T *eap);
DLLEXPORT void ex_vimgrep(exarg_T *eap);
DLLEXPORT int set_errorlist(win_T *wp, list_T *list, int action, char *title, dict_T *what);
DLLEXPORT bool set_ref_in_quickfix(int copyID);
DLLEXPORT void ex_cbuffer(exarg_T *eap);
DLLEXPORT int cexpr_core(const exarg_T *eap, typval_T *tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ex_cexpr(exarg_T *eap);
DLLEXPORT void ex_helpgrep(exarg_T *eap);
DLLEXPORT void free_quickfix(void);
DLLEXPORT void f_getloclist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getqflist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setloclist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setqflist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
