#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/vim_defs.h"  // IWYU pragma: keep
#include "nvim/window_defs.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int plines_correct_topline(win_T *wp, linenr_T lnum, linenr_T *nextp,
                                     bool limit_winheight, bool *foldedp);
DLLEXPORT void set_valid_virtcol(win_T *wp, colnr_T vcol);
DLLEXPORT int sms_marker_overlap(win_T *wp, int extra2);
DLLEXPORT void update_topline(win_T *wp);
DLLEXPORT void update_curswant_force(void);
DLLEXPORT void update_curswant(void);
DLLEXPORT void check_cursor_moved(win_T *wp);
DLLEXPORT void changed_window_setting(win_T *wp);
DLLEXPORT void changed_window_setting_all(void);
DLLEXPORT void set_topline(win_T *wp, linenr_T lnum);
DLLEXPORT void save_viewstate(win_T *wp, viewstate_T *vs) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void restore_viewstate(win_T *wp, const viewstate_T *vs) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void changed_cline_bef_curs(win_T *wp);
DLLEXPORT void changed_line_abv_curs(void);
DLLEXPORT void changed_line_abv_curs_win(win_T *wp);
DLLEXPORT void validate_botline_win(win_T *wp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void invalidate_botline_win(win_T *wp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void approximate_botline_win(win_T *wp);
DLLEXPORT int cursor_valid(win_T *wp);
DLLEXPORT void validate_cursor(win_T *wp);
DLLEXPORT void validate_virtcol(win_T *wp);
DLLEXPORT void validate_cheight(win_T *wp);
DLLEXPORT void validate_cursor_col(win_T *wp);
DLLEXPORT int win_col_off(win_T *wp);
DLLEXPORT int win_col_off2(win_T *wp);
DLLEXPORT void curs_columns(win_T *wp, int may_scroll);
DLLEXPORT void textpos2screenpos(win_T *wp, pos_T *pos, int *rowp, int *scolp, int *ccolp,
                                 int *ecolp, bool local);
DLLEXPORT void f_screenpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_virtcol2col(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void scroll_redraw(int up, linenr_T count);
DLLEXPORT bool scrolldown(win_T *wp, linenr_T line_count, int byfold);
DLLEXPORT bool scrollup(win_T *wp, linenr_T line_count, bool byfold);
DLLEXPORT void adjust_skipcol(void);
DLLEXPORT void check_topfill(win_T *wp, bool down);
DLLEXPORT void scrolldown_clamp(void);
DLLEXPORT void scrollup_clamp(void);
DLLEXPORT void scroll_cursor_top(win_T *wp, int min_scroll, int always);
DLLEXPORT void set_empty_rows(win_T *wp, int used);
DLLEXPORT void scroll_cursor_bot(win_T *wp, int min_scroll, bool set_topbot);
DLLEXPORT void scroll_cursor_halfway(win_T *wp, bool atend, bool prefer_above);
DLLEXPORT void cursor_correct(win_T *wp);
DLLEXPORT int pagescroll(Direction dir, int count, bool half);
DLLEXPORT void do_check_cursorbind(void);
