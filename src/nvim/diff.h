#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

// Value set from 'diffopt'.
EXTERN int diff_context INIT( = 6);  ///< context for folds
EXTERN int diff_foldcolumn INIT( = 2);  ///< 'foldcolumn' for diff mode
EXTERN bool diff_need_scrollbind INIT( = false);

EXTERN bool need_diff_redraw INIT( = false);  ///< need to call diff_redraw()

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

DLLEXPORT void diff_buf_delete(buf_T *buf);
DLLEXPORT void diff_buf_adjust(win_T *win);
DLLEXPORT void diff_buf_add(buf_T *buf);
DLLEXPORT void diff_invalidate(buf_T *buf);
DLLEXPORT void diff_mark_adjust(buf_T *buf, linenr_T line1, linenr_T line2, linenr_T amount,
                                linenr_T amount_after);
DLLEXPORT void diff_redraw(bool dofold);
DLLEXPORT int diff_internal(void) FUNC_ATTR_PURE;
DLLEXPORT void ex_diffupdate(exarg_T *eap);
DLLEXPORT void ex_diffpatch(exarg_T *eap);
DLLEXPORT void ex_diffsplit(exarg_T *eap);
DLLEXPORT void ex_diffthis(exarg_T *eap);
DLLEXPORT void diff_win_options(win_T *wp, bool addbuf);
DLLEXPORT void ex_diffoff(exarg_T *eap);
DLLEXPORT void diff_clear(tabpage_T *tp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool diff_linematch(diff_T *dp);
DLLEXPORT int diff_check_with_linestatus(win_T *wp, linenr_T lnum, int *linestatus);
DLLEXPORT int diff_check_fill(win_T *wp, linenr_T lnum);
DLLEXPORT void diff_set_topline(win_T *fromwin, win_T *towin);
DLLEXPORT int diffanchors_changed(bool buflocal);
DLLEXPORT int diffopt_changed(void);
DLLEXPORT bool diffopt_horizontal(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool diffopt_hiddenoff(void) FUNC_ATTR_PURE;
DLLEXPORT bool diffopt_closeoff(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool diffopt_filler(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void diff_update_line(linenr_T lnum);
DLLEXPORT bool diff_change_parse(diffline_T *diffline, diffline_change_T *change, int *change_start,
                                 int *change_end);
DLLEXPORT bool diff_find_change(win_T *wp, linenr_T lnum, diffline_T *diffline)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool diff_infold(win_T *wp, linenr_T lnum)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void nv_diffgetput(bool put, size_t count);
DLLEXPORT void ex_diffgetput(exarg_T *eap);
DLLEXPORT bool diff_mode_buf(buf_T *buf)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int diff_move_to(int dir, int count);
DLLEXPORT linenr_T diff_get_corresponding_line(buf_T *buf1, linenr_T lnum1);
DLLEXPORT linenr_T diff_lnum_win(linenr_T lnum, win_T *wp);
DLLEXPORT void f_diff_filler(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_diff_hlID(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
