#pragma once

#include <locale.h>

#include "nvim/ascii_defs.h"
#include "nvim/buffer_defs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/extmark_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/macros_defs.h"
#include "nvim/mark_defs.h"  // IWYU pragma: keep
#include "nvim/os/time.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int setmark(int c);
DLLEXPORT void free_fmark(fmark_T fm);
DLLEXPORT void free_xfmark(xfmark_T fm);
DLLEXPORT void clear_fmark(fmark_T *const fm, const Timestamp timestamp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int setmark_pos(int c, pos_T *pos, int fnum, fmarkv_T *view_pt);
DLLEXPORT void mark_jumplist_forget_file(win_T *wp, int fnum);
DLLEXPORT void mark_forget_file(win_T *wp, int fnum);
DLLEXPORT void setpcmark(void);
DLLEXPORT void checkpcmark(void);
DLLEXPORT fmark_T *get_jumplist(win_T *win, int count);
DLLEXPORT fmark_T *get_changelist(buf_T *buf, win_T *win, int count);
DLLEXPORT fmark_T *mark_get(buf_T *buf, win_T *win, fmark_T *fmp, MarkGet flag, int name);
DLLEXPORT xfmark_T *mark_get_global(bool resolve, int name);
DLLEXPORT fmark_T *mark_get_local(buf_T *buf, win_T *win, int name);
DLLEXPORT fmark_T *mark_get_motion(buf_T *buf, win_T *win, int name);
DLLEXPORT fmark_T *mark_get_visual(buf_T *buf, int name);
DLLEXPORT fmark_T *pos_to_mark(buf_T *buf, fmark_T *fmp, pos_T pos) FUNC_ATTR_NONNULL_RET;
DLLEXPORT MarkMoveRes mark_move_to(fmark_T *fm, MarkMove flags);
DLLEXPORT void mark_view_restore(fmark_T *fm);
DLLEXPORT fmarkv_T mark_view_make(const win_T *wp, pos_T pos);
DLLEXPORT fmark_T *getnextmark(pos_T *startpos, int dir, int begin_line);
DLLEXPORT void fmarks_check_names(buf_T *buf);
DLLEXPORT bool mark_check(fmark_T *fm, const char **errormsg);
DLLEXPORT bool mark_check_line_bounds(buf_T *buf, fmark_T *fm, const char **errormsg);
DLLEXPORT void clrallmarks(buf_T *const buf, const Timestamp timestamp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *fm_getname(fmark_T *fmark, int lead_len);
DLLEXPORT void ex_marks(exarg_T *eap);
DLLEXPORT void ex_delmarks(exarg_T *eap);
DLLEXPORT void ex_jumps(exarg_T *eap);
DLLEXPORT void ex_clearjumps(exarg_T *eap);
DLLEXPORT void ex_changes(exarg_T *eap);
DLLEXPORT void mark_adjust(linenr_T line1, linenr_T line2, linenr_T amount, linenr_T amount_after,
                           ExtmarkOp op);
DLLEXPORT void mark_adjust_nofold(linenr_T line1, linenr_T line2, linenr_T amount,
                                  linenr_T amount_after, ExtmarkOp op);
DLLEXPORT void mark_adjust_buf(buf_T *buf, linenr_T line1, linenr_T line2, linenr_T amount,
                               linenr_T amount_after, bool adjust_folds, MarkAdjustMode mode,
                               ExtmarkOp op);
DLLEXPORT void mark_col_adjust(linenr_T lnum, colnr_T mincol, linenr_T lnum_amount,
                               colnr_T col_amount, int spaces_removed);
DLLEXPORT void cleanup_jumplist(win_T *wp, bool loadfiles);
DLLEXPORT void copy_jumplist(win_T *from, win_T *to);
DLLEXPORT const void *mark_jumplist_iter(const void *const iter, const win_T *const win,
                                         xfmark_T *const fm)
  FUNC_ATTR_NONNULL_ARG(2, 3) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const void *mark_global_iter(const void *const iter, char *const name, xfmark_T *const fm)
  FUNC_ATTR_NONNULL_ARG(2, 3) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const void *mark_buffer_iter(const void *const iter, const buf_T *const buf,
                                       char *const name, fmark_T *const fm)
  FUNC_ATTR_NONNULL_ARG(2, 3, 4) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool mark_set_global(const char name, const xfmark_T fm, const bool update);
DLLEXPORT bool mark_set_local(const char name, buf_T *const buf, const fmark_T fm,
                              const bool update) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void free_jumplist(win_T *wp);
DLLEXPORT void set_last_cursor(win_T *win);
DLLEXPORT void free_all_marks(void);
DLLEXPORT void mark_mb_adjustpos(buf_T *buf, pos_T *lp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void get_buf_local_marks(const buf_T *buf, list_T *l) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT xfmark_T get_raw_global_mark(char name);
DLLEXPORT void get_global_marks(list_T *l) FUNC_ATTR_NONNULL_ALL;

#include "nvim/func_attr.h"

/// Convert mark name to the offset
static inline int mark_global_index(const char name)
  FUNC_ATTR_CONST
{
  return (ASCII_ISUPPER(name)
          ? (name - 'A')
          : (ascii_isdigit(name)
             ? (NMARKS + (name - '0'))
             : -1));
}

/// Convert local mark name to the offset
static inline int mark_local_index(const char name)
  FUNC_ATTR_CONST
{
  return (ASCII_ISLOWER(name)
          ? (name - 'a')
          : (name == '"'
             ? NMARKS
             : (name == '^'
                ? NMARKS + 1
                : (name == '.'
                   ? NMARKS + 2
                   : -1))));
}

/// Global marks (marks with file number or name)
EXTERN xfmark_T namedfm[NGLOBALMARKS] INIT( = { 0 });

#define SET_FMARK(fmarkp_, mark_, fnum_, view_) \
  do { \
    fmark_T *const fmarkp__ = fmarkp_; \
    fmarkp__->mark = mark_; \
    fmarkp__->fnum = fnum_; \
    fmarkp__->timestamp = os_time(); \
    fmarkp__->view = view_; \
    fmarkp__->additional_data = NULL; \
  } while (0)

/// Free and set fmark using given value
#define RESET_FMARK(fmarkp_, mark_, fnum_, view_) \
  do { \
    fmark_T *const fmarkp___ = fmarkp_; \
    free_fmark(*fmarkp___); \
    SET_FMARK(fmarkp___, mark_, fnum_, view_); \
  } while (0)

/// Set given extended mark (regular mark + file name)
#define SET_XFMARK(xfmarkp_, mark_, fnum_, view_, fname_) \
  do { \
    xfmark_T *const xfmarkp__ = xfmarkp_; \
    xfmarkp__->fname = fname_; \
    SET_FMARK(&(xfmarkp__->fmark), mark_, fnum_, view_); \
  } while (0)

/// Free and set given extended mark (regular mark + file name)
#define RESET_XFMARK(xfmarkp_, mark_, fnum_, view_, fname_) \
  do { \
    xfmark_T *const xfmarkp__ = xfmarkp_; \
    free_xfmark(*xfmarkp__); \
    xfmarkp__->fname = fname_; \
    SET_FMARK(&(xfmarkp__->fmark), mark_, fnum_, view_); \
  } while (0)
