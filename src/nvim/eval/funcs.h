#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"

/// Prototype of C function that implements Vimscript function
typedef void (*VimLFunc)(typval_T *args, typval_T *rvar, EvalFuncData data);

/// Special flags for base_arg @see EvalFuncDef
enum {
  BASE_NONE = 0,          ///< Not a method (no base argument).
  BASE_LAST = UINT8_MAX,  ///< Use the last argument as the method base.
};

/// Structure holding Vimscript function definition
typedef struct {
  char *name;         ///< Name of the function.
  uint8_t min_argc;   ///< Minimal number of arguments.
  uint8_t max_argc;   ///< Maximal number of arguments.
  uint8_t base_arg;   ///< Method base arg # (1-indexed), BASE_NONE or BASE_LAST.
  bool fast;          ///< Can be run in |api-fast| events
  VimLFunc func;      ///< Function implementation.
  EvalFuncData data;  ///< Userdata for function implementation.
} EvalFuncDef;

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

DLLEXPORT char *get_function_name(expand_T *xp, int idx);
DLLEXPORT char *get_expr_name(expand_T *xp, int idx);
DLLEXPORT const EvalFuncDef *find_internal_func(const char *const name)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *find_internal_func_lua(const char *const name)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int check_internal_func(const EvalFuncDef *const fdef, const int argcount)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int call_internal_func(const char *const fname, const int argcount,
                                 typval_T *const argvars, typval_T *const rettv)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int call_internal_method(const char *const fname, const int argcount,
                                   typval_T *const argvars, typval_T *const rettv,
                                   typval_T *const basetv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT buf_T *tv_get_buf(typval_T *tv, int curtab_only);
DLLEXPORT buf_T *tv_get_buf_from_arg(typval_T *const tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT buf_T *get_buf_arg(typval_T *arg);
DLLEXPORT win_T *get_optional_window(typval_T *argvars, int idx);
DLLEXPORT char *get_list_line(int c, void *cookie, int indent, bool do_concat);
DLLEXPORT void execute_common(typval_T *argvars, typval_T *rettv, int arg_off);
DLLEXPORT dict_T *create_environment(const dictitem_T *job_env, const bool clear_env,
                                     const bool pty, const bool set_nvim_addr,
                                     const char *const pty_term_name);
DLLEXPORT void f_jobstart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_jobstop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT int do_searchpair(const char *spat, const char *mpat, const char *epat, int dir,
                            const typval_T *skip, int flags, pos_T *match_pos, linenr_T lnum_stop,
                            int64_t time_limit) FUNC_ATTR_NONNULL_ARG(1, 2, 3);
DLLEXPORT const EvalFuncDef *eval_func_def_at(int idx);
DLLEXPORT void float_op_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void api_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void lua_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_abs(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_and(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_api_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_atan2(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_byte2line(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_call(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_changenr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_chanclose(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_chansend(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_char2nr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_charcol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_col(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_confirm(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_copy(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_cursor(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_debugbreak(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_deepcopy(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_dictwatcheradd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_dictwatcherdel(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_did_filetype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_empty(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_escape(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getenv(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_eval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_eventhandler(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_execute(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_exists(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_expand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_menu_get(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_expandcmd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_flatten(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_flattennew(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_feedkeys(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_float2nr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_fmod(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_fnameescape(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_foreground(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_funcref(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_function(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_garbagecollect(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_get(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getchangelist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcharsearch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getfontname(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getjumplist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getmarklist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getpid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcurpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcursorcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getregion(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getregionpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getreg(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getregtype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_gettagstack(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_wait(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_gettext(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_has(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_hlID(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_hlexists(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_index(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_indexof(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_input(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_inputdialog(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_inputlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_inputrestore(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_inputsave(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_inputsecret(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_interrupt(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_invert(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_islocked(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_isinf(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_isnan(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_id(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_jobpid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_jobresize(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_jobwait(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_json_decode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_json_encode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_keytrans(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_len(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_libcall(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_libcallnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_line(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_line2byte(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_localtime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_luaeval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchbufline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_match(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchend(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchstr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchstrlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_matchstrpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_max(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_min(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_mode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_state(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_msgpackdump(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_msgpackparse(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_nextnonblank(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_nr2char(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_or(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_pow(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_prevnonblank(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_printf(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_pum_getpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_pumvisible(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_py3eval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_rand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_srand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_perleval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_rubyeval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_range(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getreginfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reg_executing(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reg_recording(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reg_recorded(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reltime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reltimestr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_repeat(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reduce(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_rpcnotify(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_rpcrequest(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_screenattr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_screenchar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_screenchars(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_screencol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_screenrow(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_screenstring(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_search(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_searchdecl(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_searchpair(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_searchpairpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_searchpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_serverlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_serverstart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_serverstop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setcharsearch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setcursorcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setenv(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setfperm(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setreg(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_settagstack(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sha256(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_shellescape(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_shiftwidth(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_sockconnect(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_stdioopen(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_reltimefloat(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_soundfold(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_spellbadword(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_spellsuggest(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_split(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_stdpath(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_str2float(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strftime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strptime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_submatch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_substitute(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_swapfilelist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_swapinfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_swapname(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_synID(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_synIDattr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_synIDtrans(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_synconcealed(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_synstack(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_tabpagebuflist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_tagfiles(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_taglist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_timer_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_timer_pause(typval_T *argvars, typval_T *unused, EvalFuncData fptr);
DLLEXPORT void f_timer_start(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_timer_stop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_timer_stopall(typval_T *argvars, typval_T *unused, EvalFuncData fptr);
DLLEXPORT void f_type(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_virtcol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_visualmode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_wildmenumode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_windowsversion(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_wordcount(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_xor(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#include "nvim/func_attr.h"

#include "eval/funcs.h.inline.generated.h"
