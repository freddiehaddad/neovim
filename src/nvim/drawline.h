#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "klib/kvec.h"
#include "nvim/fold_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"

enum { TERM_ATTRS_MAX = 1024, };  ///< Maximum columns for terminal highlight attributes

typedef struct {
  NS ns_id;
  uint64_t mark_id;
  int win_row;
  int win_col;
} WinExtmark;
EXTERN kvec_t(WinExtmark) win_extmark_arr INIT( = KV_INITIAL_VALUE);

/// Spell checking variables passed from win_update() to win_line().
typedef struct {
  bool spv_has_spell;         ///< drawn window has spell checking
  bool spv_unchanged;         ///< not updating for changed text
  int spv_checked_col;        ///< column in "checked_lnum" up to
                              ///< which there are no spell errors
  linenr_T spv_checked_lnum;  ///< line number for "checked_col"
  int spv_cap_col;            ///< column to check for Cap word
  linenr_T spv_capcol_lnum;   ///< line number for "cap_col"
} spellvars_T;

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

#ifdef EXITFREE
DLLEXPORT void drawline_free_all_mem(void);
#endif
DLLEXPORT bool use_cursor_line_highlight(win_T *wp, linenr_T lnum);
DLLEXPORT void fill_foldcolumn(win_T *wp, foldinfo_T foldinfo, linenr_T lnum, int attr, int fdc,
                               bool is_virt, int *wlv_off, colnr_T *out_vcol, schar_T *out_buffer);
DLLEXPORT int win_line(win_T *wp, linenr_T lnum, int startrow, int endrow, int col_rows,
                       bool concealed, spellvars_T *spv, foldinfo_T foldinfo);

#include "nvim/func_attr.h"
