#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/input_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/vim_defs.h"  // IWYU pragma: keep

/// flags for do_cmdline()
enum {
  DOCMD_VERBOSE  = 0x01,  ///< included command in error message
  DOCMD_NOWAIT   = 0x02,  ///< don't call wait_return() and friends
  DOCMD_REPEAT   = 0x04,  ///< repeat exec. until getline() returns NULL
  DOCMD_KEYTYPED = 0x08,  ///< don't reset KeyTyped
  DOCMD_EXCRESET = 0x10,  ///< reset exception environment (for debugging
  DOCMD_KEEPLINE = 0x20,  ///< keep typed line for repeating with "."
};

/// defines for eval_vars()
enum {
  VALID_PATH = 1,
  VALID_HEAD = 2,
};

// Whether a command index indicates a user command.
#define IS_USER_CMDIDX(idx) ((int)(idx) < 0)

enum { DIALOG_MSG_SIZE = 1000, };  ///< buffer size for dialog messages

/// Structure used to save the current state.  Used when executing Normal mode
/// commands while in any other mode.
typedef struct {
  int save_msg_scroll;
  int save_restart_edit;
  bool save_msg_didout;
  int save_State;
  bool save_finish_op;
  int save_opcount;
  int save_reg_executing;
  bool save_pending_end_reg_executing;
  tasave_T tabuf;
} save_state_T;

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

DLLEXPORT int do_cmdline_cmd(const char *cmd);
DLLEXPORT int do_cmdline(char *cmdline, LineGetter fgetline, void *cookie, int flags);
DLLEXPORT void handle_did_throw(void);
DLLEXPORT bool getline_equal(LineGetter fgetline, void *cookie, LineGetter func);
DLLEXPORT void *getline_cookie(LineGetter fgetline, void *cookie);
DLLEXPORT void set_cmd_addr_type(exarg_T *eap, char *p);
DLLEXPORT linenr_T get_cmd_default_range(exarg_T *eap);
DLLEXPORT void set_cmd_dflall_range(exarg_T *eap);
DLLEXPORT void set_cmd_count(exarg_T *eap, linenr_T count, bool validate);
DLLEXPORT bool is_cmd_ni(cmdidx_T cmdidx);
DLLEXPORT bool cmd_has_expr_args(cmdidx_T cmdidx);
DLLEXPORT bool parse_cmdline(char **cmdline, exarg_T *eap, cmdmod_T *cmod, const char **errormsg,
                             bool skipmods);
DLLEXPORT int execute_cmd(exarg_T *eap, cmdmod_T *cmod, bool preview);
DLLEXPORT int parse_command_modifiers(exarg_T *eap, const char **errormsg, cmdmod_T *cmod,
                                      bool skip_only);
DLLEXPORT void apply_cmdmod(cmdmod_T *cmod);
DLLEXPORT void undo_cmdmod(cmdmod_T *cmod) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int parse_cmd_address(exarg_T *eap, const char **errormsg, bool silent)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool checkforcmd(char **pp, const char *cmd, int len);
DLLEXPORT char *find_ex_command(exarg_T *eap, int *full) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int modifier_len(char *cmd);
DLLEXPORT int cmd_exists(const char *const name);
DLLEXPORT void f_fullcommand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT cmdidx_T excmd_get_cmdidx(const char *cmd, size_t len);
DLLEXPORT uint32_t excmd_get_argt(cmdidx_T idx);
DLLEXPORT char *skip_range(const char *cmd, int *ctx);
DLLEXPORT linenr_T get_address(exarg_T *eap, char **ptr, cmd_addr_T addr_type, bool skip,
                               bool silent, int to_other_file, int address_count,
                               const char **errormsg) FUNC_ATTR_NONNULL_ARG(2, 8);
DLLEXPORT void ex_ni(exarg_T *eap);
DLLEXPORT char *invalid_range(exarg_T *eap);
DLLEXPORT char *replace_makeprg(exarg_T *eap, char *arg, char **cmdlinep);
DLLEXPORT int expand_filename(exarg_T *eap, char **cmdlinep, const char **errormsgp);
DLLEXPORT void separate_nextcmd(exarg_T *eap);
DLLEXPORT char *getargcmd(char **argp);
DLLEXPORT char *skip_cmd_arg(char *p, bool rembs);
DLLEXPORT int get_bad_opt(const char *p, exarg_T *eap) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int getargopt(exarg_T *eap);
DLLEXPORT int expand_argopt(char *pat, expand_T *xp, regmatch_T *rmp, char ***matches,
                            int *numMatches);
DLLEXPORT int ends_excmd(int c) FUNC_ATTR_CONST;
DLLEXPORT char *find_nextcmd(const char *p);
DLLEXPORT char *check_nextcmd(char *p);
DLLEXPORT char *get_command_name(expand_T *xp, int idx);
DLLEXPORT void not_exiting(bool save_exiting);
DLLEXPORT bool before_quit_autocmds(win_T *wp, bool quit_all, bool forceit);
DLLEXPORT int before_quit_all(exarg_T *eap);
DLLEXPORT void ex_win_close(int forceit, win_T *win, tabpage_T *tp);
DLLEXPORT void tabpage_close(int forceit);
DLLEXPORT void tabpage_close_other(tabpage_T *tp, int forceit);
DLLEXPORT int expand_findfunc(expand_T *xp, char *pat, char ***files, int *numMatches);
DLLEXPORT void free_findfunc_option(void);
DLLEXPORT bool set_ref_in_findfunc(int copyID);
DLLEXPORT void ex_splitview(exarg_T *eap);
DLLEXPORT void tabpage_new(void);
DLLEXPORT void do_exedit(exarg_T *eap, win_T *old_curwin);
DLLEXPORT void free_cd_dir(void);
DLLEXPORT bool do_chdir(char *new_dir, CdScope scope);
DLLEXPORT void ex_cd(exarg_T *eap);
DLLEXPORT void do_sleep(int64_t msec, bool hide_cursor);
DLLEXPORT void ex_may_print(exarg_T *eap);
DLLEXPORT int vim_mkdir_emsg(const char *const name, const int prot) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT FILE *open_exfile(char *fname, int forceit, char *mode);
DLLEXPORT void update_topline_cursor(void);
DLLEXPORT void save_current_state(save_state_T *sst) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void restore_current_state(save_state_T *sst) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool expr_map_locked(void);
DLLEXPORT void exec_normal_cmd(char *cmd, int remap, bool silent);
DLLEXPORT void exec_normal(bool was_typed, bool use_vpeekc);
DLLEXPORT ssize_t find_cmdline_var(const char *src, size_t *usedlen) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *eval_vars(char *src, const char *srcstart, size_t *usedlen, linenr_T *lnump,
                          const char **errormsg, int *escaped, bool empty_is_error);
DLLEXPORT char *expand_sfile(char *arg);
DLLEXPORT void filetype_plugin_enable(void);
DLLEXPORT void filetype_maybe_enable(void);
DLLEXPORT void set_no_hlsearch(bool flag);
DLLEXPORT bool is_loclist_cmd(int cmdidx) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void verify_command(char *cmd);
DLLEXPORT bool is_map_cmd(cmdidx_T cmdidx);
DLLEXPORT char *ex_errmsg(const char *const msg, const char *arg);

#include "nvim/func_attr.h"
