#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// arguments for win_split()
enum {
  WSP_ROOM     = 0x01,   ///< require enough room
  WSP_VERT     = 0x02,   ///< split/equalize vertically
  WSP_HOR      = 0x04,   ///< equalize horizontally
  WSP_TOP      = 0x08,   ///< window at top-left of shell
  WSP_BOT      = 0x10,   ///< window at bottom-right of shell
  WSP_HELP     = 0x20,   ///< creating the help window
  WSP_BELOW    = 0x40,   ///< put new window below/right
  WSP_ABOVE    = 0x80,   ///< put new window above/left
  WSP_NEWLOC   = 0x100,  ///< don't copy location list
  WSP_NOENTER  = 0x200,  ///< don't enter the new window
  WSP_QUICKFIX = 0x400,  ///< creating the quickfix window
};

enum {
  MIN_COLUMNS = 12,   ///< minimal columns for screen
  MIN_LINES   = 2,    ///< minimal lines for screen
  STATUS_HEIGHT = 1,  ///< height of a status line under a window
};

enum {
  /// Lowest number used for window ID. Cannot have this many windows per tab.
  LOWEST_WIN_ID = 1000,
};

EXTERN int tabpage_move_disallowed INIT( = 0);  ///< moving tabpages around disallowed

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

DLLEXPORT void window_layout_lock(void);
DLLEXPORT void window_layout_unlock(void);
DLLEXPORT bool frames_locked(void);
DLLEXPORT bool window_layout_locked(cmdidx_T cmd);
DLLEXPORT bool window_layout_locked_err(cmdidx_T cmd, Error *err);
DLLEXPORT bool check_can_set_curbuf_disabled(void);
DLLEXPORT bool check_can_set_curbuf_forceit(int forceit);
DLLEXPORT win_T *prevwin_curwin(void) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT win_T *swbuf_goto_win_with_buf(buf_T *buf);
DLLEXPORT void do_window(int nchar, int Prenum, int xchar);
DLLEXPORT void win_set_buf(win_T *win, buf_T *buf, Error *err) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int win_fdccol_count(win_T *wp);
DLLEXPORT void merge_win_config(WinConfig *dst, const WinConfig src) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void clear_float_config(WinConfig *fconfig, bool free_fields) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ui_ext_win_position(win_T *wp, bool validate);
DLLEXPORT void ui_ext_win_viewport(win_T *wp);
DLLEXPORT int check_split_disallowed(const win_T *wp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool check_split_disallowed_err(const win_T *wp, Error *err) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int win_split(int size, int flags);
DLLEXPORT win_T *win_split_ins(int size, int flags, win_T *new_wp, int dir, frame_T *to_flatten);
DLLEXPORT void win_init(win_T *newp, win_T *oldp, int flags);
DLLEXPORT bool win_valid(const win_T *win) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool tabpage_win_valid(const tabpage_T *tp, const win_T *win)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT win_T *win_find_by_handle(handle_T handle) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool win_valid_any_tab(win_T *win) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int win_count(void);
DLLEXPORT int make_windows(int count, bool vertical);
DLLEXPORT int win_splitmove(win_T *wp, int size, int flags);
DLLEXPORT void win_move_after(win_T *win1, win_T *win2);
DLLEXPORT void win_equal(win_T *next_curwin, bool current, int dir);
DLLEXPORT void leaving_window(win_T *const win) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void entering_window(win_T *const win) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void win_init_empty(win_T *wp);
DLLEXPORT void curwin_init(void);
DLLEXPORT void close_windows(buf_T *buf, bool keep_curwin);
DLLEXPORT bool last_window(win_T *win) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool one_window(win_T *win, tabpage_T *tp)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int win_close(win_T *win, bool free_buf, bool force) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void trigger_tabclosedpre(tabpage_T *tp);
DLLEXPORT bool win_close_othertab(win_T *win, int free_buf, tabpage_T *tp, bool force)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void win_free_all(void);
DLLEXPORT win_T *winframe_remove(win_T *win, int *dirp, tabpage_T *tp, frame_T **unflat_altfr)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT win_T *winframe_find_altwin(win_T *win, int *dirp, tabpage_T *tp, frame_T **altfr)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT void winframe_restore(win_T *wp, int dir, frame_T *unflat_altfr) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT win_T *frame2win(frame_T *frp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void frame_new_height(frame_T *topfrp, int height, bool topfirst, bool wfh, bool set_ch)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void close_others(int message, int forceit, bool ignore_pinned);
DLLEXPORT void unuse_tabpage(tabpage_T *tp);
DLLEXPORT void use_tabpage(tabpage_T *tp);
DLLEXPORT void win_alloc_first(void);
DLLEXPORT void win_alloc_ctx_win(int idx);
DLLEXPORT void win_init_size(void);
DLLEXPORT void free_tabpage(tabpage_T *tp);
DLLEXPORT tabpage_T *win_new_tabpage(int after, char *filename, bool enter, win_T **first);
DLLEXPORT int make_tabpages(int maxcount);
DLLEXPORT bool valid_tabpage(tabpage_T *tpc) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int valid_tabpage_win(tabpage_T *tpc);
DLLEXPORT void close_tabpage(tabpage_T *tab);
DLLEXPORT tabpage_T *find_tabpage(int n);
DLLEXPORT int tabpage_index(tabpage_T *ftp);
DLLEXPORT void goto_tabpage(int n);
DLLEXPORT void goto_tabpage_tp(tabpage_T *tp, bool trigger_enter_autocmds,
                               bool trigger_leave_autocmds);
DLLEXPORT bool goto_tabpage_lastused(void);
DLLEXPORT void goto_tabpage_win(tabpage_T *tp, win_T *wp);
DLLEXPORT void tabpage_move(int nr);
DLLEXPORT void win_goto(win_T *wp);
DLLEXPORT tabpage_T *win_find_tabpage(win_T *win);
DLLEXPORT win_T *win_vert_neighbor(tabpage_T *tp, win_T *wp, bool up, int count);
DLLEXPORT win_T *win_horz_neighbor(tabpage_T *tp, win_T *wp, bool left, int count);
DLLEXPORT void win_enter(win_T *wp, bool undo_sync);
DLLEXPORT void update_cwd(CdCause cause);
DLLEXPORT win_T *buf_jump_open_win(buf_T *buf);
DLLEXPORT win_T *buf_jump_open_tab(buf_T *buf);
DLLEXPORT win_T *win_alloc(win_T *after, bool hidden);
DLLEXPORT void free_wininfo(WinInfo *wip, buf_T *bp);
DLLEXPORT void win_free(win_T *wp, tabpage_T *tp);
DLLEXPORT void win_free_grid(win_T *wp, bool reinit);
DLLEXPORT void win_append(win_T *after, win_T *wp, tabpage_T *tp) FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void win_remove(win_T *wp, tabpage_T *tp) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void win_new_screensize(void);
DLLEXPORT void win_new_screen_rows(void);
DLLEXPORT void win_new_screen_cols(void);
DLLEXPORT void snapshot_windows_scroll_size(void);
DLLEXPORT void may_make_initial_scroll_size_snapshot(void);
DLLEXPORT void may_trigger_win_scrolled_resized(void);
DLLEXPORT void win_size_save(garray_T *gap);
DLLEXPORT void win_size_restore(garray_T *gap) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int win_comp_pos(void);
DLLEXPORT void win_setheight(int height);
DLLEXPORT void win_setheight_win(int height, win_T *win, bool from_top);
DLLEXPORT void win_setwidth(int width);
DLLEXPORT void win_setwidth_win(int width, win_T *wp, bool from_left);
DLLEXPORT const char *did_set_winminheight(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_winminwidth(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT void win_drag_status_line(win_T *dragwin, int offset);
DLLEXPORT void win_drag_vsep_line(win_T *dragwin, int offset);
DLLEXPORT void set_fraction(win_T *wp);
DLLEXPORT void win_fix_scroll(bool resize);
DLLEXPORT void win_new_height(win_T *wp, int height);
DLLEXPORT void scroll_to_fraction(win_T *wp, int prev_height);
DLLEXPORT void win_set_inner_size(win_T *wp, bool valid_cursor);
DLLEXPORT void win_new_width(win_T *wp, int width);
DLLEXPORT OptInt win_default_scroll(win_T *wp);
DLLEXPORT void win_comp_scroll(win_T *wp);
DLLEXPORT void command_height(void);
DLLEXPORT void last_status(bool morewin);
DLLEXPORT void win_remove_status_line(win_T *wp, bool add_hsep);
DLLEXPORT int set_winbar_win(win_T *wp, bool make_room, bool valid_cursor);
DLLEXPORT void set_winbar(bool make_room);
DLLEXPORT void set_winbar_all(bool make_room);
DLLEXPORT int tabline_height(void);
DLLEXPORT int global_winbar_height(void);
DLLEXPORT int global_stl_height(void);
DLLEXPORT int last_stl_height(bool morewin);
DLLEXPORT int min_rows(tabpage_T *tp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int min_rows_for_all_tabpages(void);
DLLEXPORT bool only_one_window(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void check_lnums(bool do_curwin);
DLLEXPORT void check_lnums_nested(bool do_curwin);
DLLEXPORT void reset_lnums(void);
DLLEXPORT void make_snapshot(int idx);
DLLEXPORT void restore_snapshot(int idx, int close_curwin);
DLLEXPORT const char *check_colorcolumn(char *cc, win_T *wp);
DLLEXPORT int get_last_winid(void);
DLLEXPORT int win_locked(win_T *wp);
DLLEXPORT void win_get_tabwin(handle_T id, int *tabnr, int *winnr);
DLLEXPORT void win_ui_flush(bool validate);
DLLEXPORT win_T *lastwin_nofloating(tabpage_T *tp);
#include "nvim/func_attr.h"
