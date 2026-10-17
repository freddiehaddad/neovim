#pragma once

#include <stddef.h>

#include "nvim/highlight_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/statusline_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Array defining what should be done when tabline is clicked
EXTERN StlClickDefinition *tab_page_click_defs INIT( = NULL);
/// Size of the tab_page_click_defs array
EXTERN size_t tab_page_click_defs_size INIT( = 0);

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void win_redr_status(win_T *wp);
DLLEXPORT void get_trans_bufname(buf_T *buf);
DLLEXPORT bool stl_connected(win_T *wp);
DLLEXPORT void stl_clear_click_defs(StlClickDefinition *const click_defs,
                                    const size_t click_defs_size);
DLLEXPORT StlClickDefinition *stl_alloc_click_defs(StlClickDefinition *cdp, int width,
                                                   size_t *size);
DLLEXPORT void stl_fill_click_defs(StlClickDefinition *click_defs, StlClickRecord *click_recs,
                                   const char *buf, int width, bool tabline);
DLLEXPORT void win_redr_winbar(win_T *wp);
DLLEXPORT void redraw_ruler(void);
DLLEXPORT schar_T fillchar_status(hlf_T *group, win_T *wp);
DLLEXPORT void draw_tabline(void);
DLLEXPORT int build_statuscol_str(win_T *wp, linenr_T lnum, int relnum, int virtnum, CharBuf buf,
                                  statuscol_T *stcp);
DLLEXPORT int build_stl_str_hl(win_T *wp, CharBuf out, char *fmt, OptIndex opt_idx, int opt_scope,
                               schar_T fillchar, int maxwidth, stl_hlrec_t **hltab,
                               size_t *hltab_len, StlClickRecord **tabtab, statuscol_T *stcp);
