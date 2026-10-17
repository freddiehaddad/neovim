#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "nvim/marktree_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"

typedef bool CSType;

enum {
  kCharsizeRegular,
  kCharsizeFast,
};

/// Argument for char size functions.
typedef struct {
  win_T *win;
  char *line;                ///< Start of the line.

  bool use_tabstop;          ///< Use 'tabstop' instead of char2cells() for a TAB.
  int indent_width;          ///< Width of 'showbreak' and 'breakindent' on wrapped
                             ///< parts of lines, INT_MIN if not yet calculated.

  int virt_row;              ///< Row for virtual text, -1 if no virtual text.
  bool skip_cur_text;        ///< Don't count inline text at the measured character or advance iter.
                             ///< A CharsizeArg with this set cannot be reused for a later character.
  int cur_text_width_left;   ///< Width of virtual text left of cursor.
  int cur_text_width_right;  ///< Width of virtual text right of cursor.

  int max_head_vcol;         ///< See charsize_regular().
  MarkTreeIter iter[1];
} CharsizeArg;

typedef struct {
  int width;
  int head;  ///< Size of 'breakindent' etc. before the character (included in width).
  int tail;  ///< Size of 'linebreak' after the character (included in width).
} CharSize;

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

DLLEXPORT int win_chartabsize(win_T *wp, const char *p, colnr_T col);
DLLEXPORT int linetabsize_col(int startvcol, char *s);
DLLEXPORT int linetabsize(win_T *wp, linenr_T lnum);
DLLEXPORT int linetabsize_eol(win_T *wp, linenr_T lnum);
DLLEXPORT CSType init_charsize_arg(CharsizeArg *csarg, win_T *wp, linenr_T lnum, char *line);
DLLEXPORT CSType init_charsize_arg_skip_cur_text(CharsizeArg *csarg, win_T *wp, linenr_T lnum,
                                                 char *line);
DLLEXPORT CharSize charsize_regular(CharsizeArg *csarg, const char *cur, colnr_T const vcol,
                                    int32_t const cur_char, int mb_width);
DLLEXPORT CharSize charsize_fast(CharsizeArg *csarg, const char *cur, colnr_T vcol,
                                 int32_t cur_char, int mb_width) FUNC_ATTR_PURE;
DLLEXPORT int charsize_nowrap(buf_T *buf, const char *cur, bool use_tabstop, colnr_T vcol,
                              int32_t cur_char, int mb_width);
DLLEXPORT int linesize_regular(CharsizeArg *const csarg, int vcol_arg, colnr_T const len);
DLLEXPORT int linesize_fast(CharsizeArg const *const csarg, int vcol_arg, colnr_T const len);
DLLEXPORT void getvcol(win_T *wp, pos_T *pos, colnr_T *start, colnr_T *cursor, colnr_T *end,
                       int flags);
DLLEXPORT colnr_T getvcol_nolist(pos_T *posp);
DLLEXPORT void getvvcol(win_T *wp, pos_T *pos, colnr_T *start, colnr_T *cursor, colnr_T *end,
                        int flags);
DLLEXPORT void getvcols(win_T *wp, pos_T *pos1, pos_T *pos2, colnr_T *left, colnr_T *right,
                        int flags);
DLLEXPORT bool win_may_fill(win_T *wp);
DLLEXPORT int win_get_fill(win_T *wp, linenr_T lnum);
DLLEXPORT int plines_win(win_T *wp, linenr_T lnum, bool limit_winheight);
DLLEXPORT int plines_win_nofill(win_T *wp, linenr_T lnum, bool limit_winheight);
DLLEXPORT int plines_win_nofold(win_T *wp, linenr_T lnum);
DLLEXPORT int plines_win_col(win_T *wp, linenr_T lnum, long column);
DLLEXPORT int plines_win_full(win_T *wp, linenr_T lnum, linenr_T *const nextp, bool *const foldedp,
                              const bool cache, const bool limit_winheight);
DLLEXPORT int plines_m_win(win_T *wp, linenr_T first, linenr_T last, int max);
DLLEXPORT int plines_m_win_fill(win_T *wp, linenr_T first, linenr_T last);
DLLEXPORT int64_t win_text_height(win_T *const wp, const linenr_T start_lnum,
                                  const int64_t start_vcol, linenr_T *const end_lnum,
                                  int64_t *const end_vcol, int64_t *const fill, int64_t const max);
DLLEXPORT int win_max_displaywidth(win_T *wp, linenr_T first, linenr_T last, int max);

#include "nvim/func_attr.h"

#include "plines.h.inline.generated.h"

/// Get the number of cells taken up on the screen by the given character at vcol.
static inline CharSize win_charsize(CSType cstype, int vcol, const char *ptr, int32_t chr,
                                    CharsizeArg *csarg, int mb_width)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_ALWAYS_INLINE
{
  if (cstype == kCharsizeFast) {
    return charsize_fast(csarg, ptr, vcol, chr, mb_width);
  } else {
    return charsize_regular(csarg, ptr, vcol, chr, mb_width);
  }
}

/// Return the number of cells the string "s" will take on the screen,
/// taking into account the size of a tab.
static inline int linetabsize_str(char *s)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_ALWAYS_INLINE
{
  return linetabsize_col(0, s);
}

/// Like linetabsize_str(), but for a given window instead of the current one.
static inline int win_linetabsize(win_T *wp, linenr_T lnum, char *line, colnr_T len)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_ALWAYS_INLINE
{
  CharsizeArg csarg;
  CSType const cstype = init_charsize_arg(&csarg, wp, lnum, line);
  if (cstype == kCharsizeFast) {
    return linesize_fast(&csarg, 0, len);
  } else {
    return linesize_regular(&csarg, 0, len);
  }
}

/// Flags used by getvcol()
enum {
  GETVCOL_END_EXCL_LBR = 1,
};
