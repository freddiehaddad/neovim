#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/macros_defs.h"
#include "nvim/normal_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Values for find_ident_under_cursor()
enum {
  FIND_IDENT  = 1,  ///< find identifier (word)
  FIND_STRING = 2,  ///< find any string (WORD)
  FIND_EVAL   = 4,  ///< include "->", "[]" and "."
};

/// 'showcmd' buffer shared between normal.c and statusline.c
EXTERN char showcmd_buf[SHOWCMD_BUFLEN];

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

DLLEXPORT void init_normal_cmds(void);
DLLEXPORT bool nv_is(int cmdchar, int flags);
DLLEXPORT bool check_text_or_curbuf_locked(oparg_T *oap);
DLLEXPORT bool op_pending(void);
DLLEXPORT void normal_enter(void);
DLLEXPORT visualinfo_T visualinfo(void);
DLLEXPORT void end_visual_mode(void);
DLLEXPORT void reset_VIsual_and_resel(void);
DLLEXPORT void reset_VIsual(void);
DLLEXPORT void restore_visual_mode(void);
DLLEXPORT size_t find_ident_under_cursor(char **text, int find_type, int *offset)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT size_t find_ident_at_pos(win_T *wp, linenr_T lnum, colnr_T startcol, char **text,
                                   int *textcol, int find_type)
  FUNC_ATTR_NONNULL_ARG(1, 4);
DLLEXPORT void clearop(oparg_T *oap);
DLLEXPORT void clearopbeep(oparg_T *oap);
DLLEXPORT void may_clear_cmdline(void);
DLLEXPORT void clear_showcmd(void);
DLLEXPORT bool add_to_showcmd(int c);
DLLEXPORT void add_to_showcmd_c(int c);
DLLEXPORT void push_showcmd(void);
DLLEXPORT void pop_showcmd(void);
DLLEXPORT void showcmd_update_clear_state(void);
DLLEXPORT void display_showcmd(void);
DLLEXPORT int get_vtopline(win_T *wp);
DLLEXPORT void do_check_scrollbind(bool check);
DLLEXPORT void check_scrollbind(linenr_T vtopline_diff, int leftcol_diff);
DLLEXPORT bool find_decl(char *ptr, size_t len, bool locally, bool thisblock, int flags_arg);
DLLEXPORT bool nv_screengo(oparg_T *oap, int dir, int dist, bool skip_conceal);
DLLEXPORT void nv_scroll_line(cmdarg_T *cap);
DLLEXPORT void do_nv_ident(int c1, int c2);
DLLEXPORT bool get_visual_text(cmdarg_T *cap, char **pp, size_t *lenp);
DLLEXPORT void start_selection(void);
DLLEXPORT void may_start_select(int c);
DLLEXPORT void nv_g_home_m_cmd(cmdarg_T *cap);
DLLEXPORT bool unadjust_for_sel(void);
DLLEXPORT bool unadjust_for_sel_inner(pos_T *pp);
DLLEXPORT void set_cursor_for_append_to_line(void);
DLLEXPORT void normal_cmd(oparg_T *oap, bool toplevel);
DLLEXPORT void exec_stuffed(cmdarg_T *cap);

#include "nvim/func_attr.h"
