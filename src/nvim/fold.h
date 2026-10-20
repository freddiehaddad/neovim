#pragma once

#include <stdio.h>  // IWYU pragma: keep

#include "nvim/decoration_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/fold_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

EXTERN int disable_fold_update INIT( = 0);

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

DLLEXPORT void copyFoldingState(win_T *wp_from, win_T *wp_to);
DLLEXPORT int hasAnyFolding(win_T *win);
DLLEXPORT bool hasFolding(win_T *win, linenr_T lnum, linenr_T *firstp, linenr_T *lastp);
DLLEXPORT bool hasFoldingWin(win_T *const win, const linenr_T lnum, linenr_T *const firstp,
                             linenr_T *const lastp, const bool cache, foldinfo_T *const infop);
DLLEXPORT bool lineFolded(win_T *const win, const linenr_T lnum);
DLLEXPORT foldinfo_T fold_info(win_T *win, linenr_T lnum);
DLLEXPORT bool foldmethodIsManual(win_T *wp);
DLLEXPORT bool foldmethodIsIndent(win_T *wp);
DLLEXPORT bool foldmethodIsExpr(win_T *wp);
DLLEXPORT bool foldmethodIsMarker(win_T *wp);
DLLEXPORT bool foldmethodIsSyntax(win_T *wp);
DLLEXPORT bool foldmethodIsDiff(win_T *wp);
DLLEXPORT void closeFold(pos_T pos, int count);
DLLEXPORT void closeFoldRecurse(pos_T pos);
DLLEXPORT void opFoldRange(pos_T firstpos, pos_T lastpos, int opening, int recurse,
                           bool had_visual);
DLLEXPORT void openFold(pos_T pos, int count);
DLLEXPORT void openFoldRecurse(pos_T pos);
DLLEXPORT void foldOpenCursor(void);
DLLEXPORT void newFoldLevel(void);
DLLEXPORT void foldCheckClose(void);
DLLEXPORT int foldManualAllowed(bool create);
DLLEXPORT void foldCreate(win_T *wp, pos_T start, pos_T end);
DLLEXPORT void deleteFold(win_T *const wp, const linenr_T start, const linenr_T end,
                          const int recursive, const bool had_visual);
DLLEXPORT void clearFolding(win_T *win);
DLLEXPORT void foldUpdate(win_T *wp, linenr_T top, linenr_T bot);
DLLEXPORT void foldUpdateAfterInsert(void);
DLLEXPORT void foldUpdateAll(win_T *win);
DLLEXPORT int foldMoveTo(const bool updown, const int dir, const int count);
DLLEXPORT void foldInitWin(win_T *new_win);
DLLEXPORT int find_wl_entry(win_T *win, linenr_T lnum);
DLLEXPORT void foldAdjustVisual(void);
DLLEXPORT void foldAdjustCursor(win_T *wp);
DLLEXPORT void cloneFoldGrowArray(garray_T *from, garray_T *to);
DLLEXPORT void deleteFoldRecurse(buf_T *bp, garray_T *gap);
DLLEXPORT void foldMarkAdjust(win_T *wp, linenr_T line1, linenr_T line2, linenr_T amount,
                              linenr_T amount_after);
DLLEXPORT int getDeepestNesting(win_T *wp);
DLLEXPORT char *get_foldtext(win_T *wp, linenr_T lnum, linenr_T lnume, foldinfo_T foldinfo,
                             char *buf, VirtText *vt) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void foldMoveRange(win_T *const wp, garray_T *gap, const linenr_T line1,
                             const linenr_T line2, const linenr_T dest);
DLLEXPORT int put_folds(FILE *fd, win_T *wp);
DLLEXPORT void f_foldclosed(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_foldclosedend(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_foldlevel(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_foldtext(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_foldtextresult(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"
