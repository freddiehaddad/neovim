#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"
#include "nvim/fold_defs.h"
#include "nvim/func_attr.h"
#include "nvim/highlight_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"

/// flags for update_screen()
/// The higher the value, the higher the priority
enum {
  UPD_VALID        = 10,  ///< buffer not changed, or changes marked with b_mod_*
  UPD_INVERTED     = 20,  ///< redisplay inverted part that changed
  UPD_INVERTED_ALL = 25,  ///< redisplay whole inverted part
  UPD_REDRAW_TOP   = 30,  ///< display first w_upd_rows screen lines
  UPD_SOME_VALID   = 35,  ///< like UPD_NOT_VALID but may scroll
  UPD_NOT_VALID    = 40,  ///< buffer needs complete redraw
  UPD_CLEAR        = 50,  ///< screen messed up, clear it
};

/// While redrawing the screen this flag is set.  It means the screen size
/// ('lines' and 'rows') must not be changed.
EXTERN bool updating_screen INIT( = false);

/// While computing a statusline and the like we do not want any w_redr_type or
/// must_redraw to be set.
EXTERN bool redraw_not_allowed INIT( = false);

/// used for 'hlsearch' highlight matching
EXTERN match_T screen_search_hl INIT( = { 0 });

/// last lnum where CurSearch was displayed
EXTERN linenr_T search_hl_has_cursor_lnum INIT( = 0);

#define W_ENDCOL(wp)   ((wp)->w_wincol + (wp)->w_width)
#define W_ENDROW(wp)   ((wp)->w_winrow + (wp)->w_height)

void conceal_check_cursor_line(void);
bool default_grid_alloc(void);
void screenclear(void);
void screen_resize(int width, int height);
void check_screensize(void);
bool redrawing(void);
int update_screen(void);
void start_search_hl(void);
void end_search_hl(void);
void setcursor(void);
void setcursor_mayforce(win_T *wp, bool force);
bool redraw_custom_title_later(void);
void show_cursor_info_later(bool force);
bool skip_showmode(void);
int showmode(void);
void unshowmode(bool force);
void clearmode(void);
void comp_col(void);
void win_scroll_lines(win_T *wp, int row, int line_count);
void win_draw_end(win_T *wp, schar_T c1, bool draw_margin, int startrow, int endrow, hlf_T hl);
int compute_foldcolumn(win_T *wp, int col);
int number_width(win_T *wp);
void redraw_later(win_T *wp, int type);
void redraw_all_later(int type);
void set_must_redraw(int type);
void screen_invalidate_highlights(void);
void redraw_curbuf_later(int type);
void redraw_buf_later(buf_T *buf, int type);
void redraw_buf_line_later(buf_T *buf, linenr_T line, bool force);
void redraw_win_range_later(win_T *wp, linenr_T first, linenr_T last);
void redrawWinline(win_T *wp, linenr_T lnum) FUNC_ATTR_NONNULL_ALL;
void redraw_buf_range_later(buf_T *buf, linenr_T first, linenr_T last);
void redraw_buf_status_later(buf_T *buf);
void status_redraw_all(void);
void status_redraw_curbuf(void);
void status_redraw_buf(buf_T *buf);
void redraw_statuslines(void);
void win_redraw_last_status(const frame_T *frp) FUNC_ATTR_NONNULL_ARG(1);
bool conceal_cursor_line(const win_T *wp) FUNC_ATTR_NONNULL_ALL;
bool win_cursorline_standout(const win_T *wp) FUNC_ATTR_NONNULL_ALL;
void win_update_cursorline(win_T *wp, foldinfo_T *foldinfo);
