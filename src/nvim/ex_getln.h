#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/ex_getln_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// flags used by vim_strsave_fnameescape()
enum {
  VSE_NONE   = 0,
  VSE_SHELL  = 1,  ///< escape for a shell command
  VSE_BUFFER = 2,  ///< escape for a ":buffer" command
};

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
DLLEXPORT bool parse_pattern_and_range(pos_T *incsearch_start, int *search_delim, int *skiplen,
                                       int *patlen, bool *is_magic) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT handle_T cmdpreview_get_bufnr(void);
DLLEXPORT int cmdpreview_get_ns(void);
DLLEXPORT char *getcmdline(int firstc, int count, int indent, bool do_concat FUNC_ATTR_UNUSED);
DLLEXPORT char *getcmdline_prompt(const int firstc, const char *const prompt, const int hl_id,
                                  const int xp_context, const char *const xp_arg,
                                  const Callback highlight_callback, bool one_key, bool *mouse_used)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC;
DLLEXPORT int check_opt_wim(const char *value, uint8_t *flags);
DLLEXPORT bool text_locked(void);
DLLEXPORT void text_locked_msg(void);
DLLEXPORT const char *get_text_locked_msg(void);
DLLEXPORT bool text_or_buf_locked(void);
DLLEXPORT bool curbuf_locked(void);
DLLEXPORT bool allbuf_locked(void);
DLLEXPORT int cmd_screencol(int bytepos);
DLLEXPORT char *getexline(int c, void *cookie, int indent, bool do_concat);
DLLEXPORT bool cmdline_overstrike(void) FUNC_ATTR_PURE;
DLLEXPORT bool cmdline_at_end(void) FUNC_ATTR_PURE;
DLLEXPORT void realloc_cmdbuff(int len);
DLLEXPORT void ui_ext_cmdline_block_append(size_t indent, const char *line);
DLLEXPORT void ui_ext_cmdline_block_leave(void);
DLLEXPORT void cmdline_screen_cleared(void);
DLLEXPORT void cmdline_ui_flush(void);
DLLEXPORT void putcmdline(char c, bool shift);
DLLEXPORT void unputcmdline(void);
DLLEXPORT void put_on_cmdline(const char *str, int len, bool redraw);
DLLEXPORT void cmdline_paste_str(const char *s, bool literally);
DLLEXPORT void redrawcmdline(void);
DLLEXPORT void redrawcmd(void);
DLLEXPORT void compute_cmdrow(void);
DLLEXPORT void cursorcmd(void);
DLLEXPORT void gotocmdline(bool clr);
DLLEXPORT char *vim_strsave_fnameescape(const char *const fname, const int what)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void escape_fname(char **pp);
DLLEXPORT void tilde_replace(char *orig_pat, int num_files, char **files);
DLLEXPORT CmdlineInfo *get_cmdline_info(void);
DLLEXPORT unsigned get_cmdline_last_prompt_id(void);
DLLEXPORT void f_getcmdcomplpat(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdcompltype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdprompt(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdscreenpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcmdtype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setcmdline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setcmdpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT int get_cmdline_firstc(void);
DLLEXPORT int get_list_range(char **str, int *num1, int *num2);
DLLEXPORT void cmdline_init(void);
DLLEXPORT const char *validate_cedit(optset_T *args);
DLLEXPORT const char *did_set_cedit(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT void cmdwin_do_action(const char *action);
DLLEXPORT bool is_in_cmdwin(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *script_get(exarg_T *const eap, size_t *const lenp)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC;
DLLEXPORT void get_user_input(const typval_T *const argvars, typval_T *const rettv,
                              const bool inputdialog, const bool secret) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void f_wildtrigger(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
#include "nvim/func_attr.h"
