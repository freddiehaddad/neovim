#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/channel_defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/eval_defs.h"  // IWYU pragma: keep
#include "nvim/event/defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/grid_defs.h"  // IWYU pragma: keep
#include "nvim/hashtab_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/mbyte_defs.h"  // IWYU pragma: keep
#include "nvim/msgpack_rpc/channel_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/os/fileio_defs.h"  // IWYU pragma: keep
#include "nvim/os/stdpaths_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/vim_defs.h"  // IWYU pragma: keep

#define COPYID_INC 2
#define COPYID_MASK (~0x1)

// Structure returned by get_lval() and used by set_var_lval().
// For a plain name:
//      "name"      points to the variable name.
//      "exp_name"  is NULL.
//      "tv"        is NULL
// For a magic braces name:
//      "name"      points to the expanded variable name.
//      "exp_name"  is non-NULL, to be freed later.
//      "tv"        is NULL
// For an index in a list:
//      "name"      points to the (expanded) variable name.
//      "exp_name"  NULL or non-NULL, to be freed later.
//      "tv"        points to the (first) list item value
//      "li"        points to the (first) list item
//      "range", "n1", "n2" and "empty2" indicate what items are used.
// For an existing Dict item:
//      "name"      points to the (expanded) variable name.
//      "exp_name"  NULL or non-NULL, to be freed later.
//      "tv"        points to the dict item value
//      "newkey"    is NULL
// For a non-existing Dict item:
//      "name"      points to the (expanded) variable name.
//      "exp_name"  NULL or non-NULL, to be freed later.
//      "tv"        points to the Dictionary typval_T
//      "newkey"    is the key for the new item.
typedef struct {
  const char *ll_name;  ///< Start of variable name (can be NULL).
  size_t ll_name_len;   ///< Length of the .ll_name.
  char *ll_exp_name;    ///< NULL or expanded name in allocated memory.
  typval_T *ll_tv;      ///< Typeval of item being used.  If "newkey"
  ///< isn't NULL it's the Dict to which to add the item.
  listitem_T *ll_li;  ///< The list item or NULL.
  list_T *ll_list;    ///< The list or NULL.
  bool ll_range;      ///< true when a [i:j] range was used.
  bool ll_empty2;     ///< Second index is empty: [i:].
  int ll_n1;          ///< First index for list.
  int ll_n2;          ///< Second index for list range.
  dict_T *ll_dict;    ///< The Dict or NULL.
  dictitem_T *ll_di;  ///< The dictitem or NULL.
  char *ll_newkey;    ///< New key for Dict in allocated memory or NULL.
  blob_T *ll_blob;    ///< The Blob or NULL.
} lval_T;

/// enum used by var_flavour()
typedef enum {
  VAR_FLAVOUR_DEFAULT = 1,   // doesn't start with uppercase
  VAR_FLAVOUR_SESSION = 2,   // starts with uppercase, some lower
  VAR_FLAVOUR_SHADA   = 4,  // all uppercase
} var_flavour_T;

// Struct passed to get_v_event() and restore_v_event().
typedef struct {
  bool sve_did_save;
  hashtab_T sve_hashtab;
} save_v_event_T;

/// trans_function_name() flags
typedef enum {
  TFN_INT = 1,  ///< May use internal function name
  TFN_QUIET = 2,  ///< Do not emit error messages.
  TFN_NO_AUTOLOAD = 4,  ///< Do not use script autoloading.
  TFN_NO_DEREF = 8,  ///< Do not dereference a Funcref.
  TFN_READ_ONLY = 16,  ///< Will not change the variable.
} TransFunctionNameFlags;

/// get_lval() flags
typedef enum {
  GLV_QUIET = TFN_QUIET,  ///< Do not emit error messages.
  GLV_NO_AUTOLOAD = TFN_NO_AUTOLOAD,  ///< Do not use script autoloading.
  GLV_READ_ONLY = TFN_READ_ONLY,  ///< Indicates that caller will not change
                                  ///< the value (prevents error message).
} GetLvalFlags;

/// flags for find_name_end()
#define FNE_INCL_BR     1       // find_name_end(): include [] in name
#define FNE_CHECK_START 2       // find_name_end(): check name starts with
                                // valid character

typedef struct {
  TimeWatcher tw;
  int timer_id;
  int repeat_count;
  int refcount;
  int emsg_count;  ///< Errors in a repeating timer.
  int64_t timeout;
  bool stopped;
  bool paused;
  Callback callback;
} timer_T;

/// types for expressions.
typedef enum {
  EXPR_UNKNOWN = 0,
  EXPR_EQUAL,         ///< ==
  EXPR_NEQUAL,        ///< !=
  EXPR_GREATER,       ///< >
  EXPR_GEQUAL,        ///< >=
  EXPR_SMALLER,       ///< <
  EXPR_SEQUAL,        ///< <=
  EXPR_MATCH,         ///< =~
  EXPR_NOMATCH,       ///< !~
  EXPR_IS,            ///< is
  EXPR_ISNOT,         ///< isnot
} exprtype_T;

// Used for checking if local variables or arguments used in a lambda.
extern bool *eval_lavars_used;

// Character used as separated in autoload function/variable names.
#define AUTOLOAD_CHAR '#'

/// Flag for expression evaluation.
enum {
  EVAL_EVALUATE = 1,  ///< when missing don't actually evaluate
};

/// Passed to an eval() function to enable evaluation.
EXTERN evalarg_T EVALARG_EVALUATE INIT( = { EVAL_EVALUATE, NULL, NULL, NULL });

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

DLLEXPORT dict_T *get_v_event(save_v_event_T *sve);
DLLEXPORT void restore_v_event(dict_T *v_event, save_v_event_T *sve);
DLLEXPORT varnumber_T num_divide(varnumber_T n1,
                                 varnumber_T n2) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T num_modulus(varnumber_T n1,
                                  varnumber_T n2) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void eval_init(void);
DLLEXPORT void eval_clear(void);
DLLEXPORT void fill_evalarg_from_eap(evalarg_T *evalarg, exarg_T *eap, bool skip);
DLLEXPORT bool eval_to_bool(char *arg, bool *error, exarg_T *eap, const bool skip,
                            const bool use_simple_function);
DLLEXPORT bool eval_expr_valid_arg(const typval_T *const tv) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_CONST;
DLLEXPORT int eval_expr_typval(const typval_T *expr, bool want_func, typval_T *argv, int argc,
                               typval_T *rettv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool eval_expr_to_bool(const typval_T *expr, bool *error) FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT char *eval_to_string_skip(char *arg, exarg_T *eap,
                                    const bool skip) FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ARG(1)
                                                                                                                        FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int skip_expr(char **pp, evalarg_T *const evalarg);
DLLEXPORT char *eval_to_string_eap(char *arg, const bool join_list, exarg_T *eap,
                                   const bool use_simple_function);
DLLEXPORT char *eval_to_string(char *arg, const bool join_list, const bool use_simple_function);
DLLEXPORT char *eval_to_string_safe(char *arg, const bool use_sandbox,
                                    const bool use_simple_function);
DLLEXPORT varnumber_T eval_to_number(char *expr, const bool use_simple_function);
DLLEXPORT typval_T *eval_expr(char *arg, exarg_T *eap);
DLLEXPORT typval_T *eval_expr_ext(char *arg, exarg_T *eap, const bool use_simple_function);
DLLEXPORT int call_vim_function(const char *func, int argc, typval_T *argv,
                                typval_T *rettv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void *call_func_retstr(const char *const func, int argc,
                                 typval_T *argv) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT
                                                                                                                                      FUNC_ATTR_MALLOC;
DLLEXPORT void *call_func_retlist(const char *func, int argc, typval_T *argv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int eval_expr_option_number(Callback *cb, bool use_sandbox);
DLLEXPORT bool eval_expr_option_tv(Callback *cb, bool use_sandbox, typval_T *ret_tv);
DLLEXPORT int eval_foldexpr(win_T *wp, int *cp);
DLLEXPORT Object eval_foldtext(win_T *wp);
DLLEXPORT char *get_lval(char *const name, typval_T *const rettv, lval_T *const lp,
                         const bool unlet, const bool skip, const int flags,
                         const int fne_flags) FUNC_ATTR_NONNULL_ARG(1, 3);
DLLEXPORT void clear_lval(lval_T *lp);
DLLEXPORT void set_var_lval(lval_T *lp, char *endp, typval_T *rettv, bool copy, const bool is_const,
                            const char *op);
DLLEXPORT void *eval_for_line(const char *arg, bool *errp, exarg_T *eap, evalarg_T *const evalarg);
DLLEXPORT bool next_for_item(void *fi_void, char *arg);
DLLEXPORT void free_for_info(void *fi_void);
DLLEXPORT void set_context_for_expression(expand_T *xp, char *arg,
                                          cmdidx_T cmdidx) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int pattern_match(const char *pat, const char *text, bool ic);
DLLEXPORT void clear_evalarg(evalarg_T *evalarg, exarg_T *eap);
DLLEXPORT int eval0(char *arg, typval_T *rettv, exarg_T *eap, evalarg_T *const evalarg);
DLLEXPORT int may_call_simple_func(const char *arg, typval_T *rettv);
DLLEXPORT int eval1(char **arg, typval_T *rettv, evalarg_T *const evalarg);
DLLEXPORT int grow_string_tv(typval_T *tv1, const char *s2);
DLLEXPORT void f_slice(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT int eval_option(const char **const arg, typval_T *const rettv,
                          const bool evaluate) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int eval_interp_string(char **arg, typval_T *rettv, bool evaluate);
DLLEXPORT char *partial_name(partial_T *pt) FUNC_ATTR_PURE;
DLLEXPORT void partial_unref(partial_T *pt);
DLLEXPORT bool func_equal(typval_T *tv1, typval_T *tv2, bool ic);
DLLEXPORT int get_copyID(void) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool garbage_collect(bool testing);
DLLEXPORT bool set_ref_in_ht(hashtab_T *ht, int copyID,
                             list_stack_T **list_stack) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool set_ref_in_list_items(list_T *l, int copyID,
                                     ht_stack_T **ht_stack) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool set_ref_in_item(typval_T *tv, int copyID, ht_stack_T **ht_stack,
                               list_stack_T **list_stack) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT size_t string2float(const char *const text,
                              float_T *const ret_value) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char **tv_to_argv(typval_T *cmd_tv, const char **cmd, bool *executable);
DLLEXPORT void f_system(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_systemlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT bool callback_from_typval(Callback *const callback,
                                    const typval_T *const arg) FUNC_ATTR_NONNULL_ALL
                                                                                                               FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int get_callback_depth(void);
DLLEXPORT bool callback_call(Callback *const callback, const int argcount_in,
                             typval_T *const argvars_in,
                             typval_T *const rettv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool set_ref_in_callback(Callback *callback, int copyID, ht_stack_T **ht_stack,
                                   list_stack_T **list_stack);
DLLEXPORT timer_T *find_timer_by_nr(varnumber_T xx);
DLLEXPORT void add_timer_info(typval_T *rettv, timer_T *timer);
DLLEXPORT void add_timer_info_all(typval_T *rettv);
DLLEXPORT void timer_due_cb(TimeWatcher *tw, void *data);
DLLEXPORT uint64_t timer_start(const int64_t timeout, const int repeat_count,
                               const Callback *const callback);
DLLEXPORT void timer_stop(timer_T *timer);
DLLEXPORT void timer_stop_all(void);
DLLEXPORT void timer_teardown(void);
DLLEXPORT char *save_tv_as_string(typval_T *tv, ptrdiff_t *const len, bool endnl,
                                  bool crlf) FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int buf_byteidx_to_charidx(buf_T *buf, linenr_T lnum, int byteidx);
DLLEXPORT int buf_charidx_to_byteidx(buf_T *buf, linenr_T lnum, int charidx);
DLLEXPORT pos_T *var2fpos(const typval_T *const tv, const bool dollar_lnum, int *const ret_fnum,
                          const bool charcol,
                          win_T *wp) FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int list2fpos(typval_T *arg, pos_T *posp, int *fnump, colnr_T *curswantp, bool charcol);
DLLEXPORT int get_env_len(const char **arg);
DLLEXPORT int get_id_len(const char **const arg);
DLLEXPORT int get_name_len(const char **const arg, char **alias, bool evaluate, bool verbose);
DLLEXPORT const char *find_name_end(const char *arg, const char **expr_start, const char **expr_end,
                                    int flags);
DLLEXPORT bool eval_isnamec(int c);
DLLEXPORT bool eval_isnamec1(int c);
DLLEXPORT bool eval_isdictc(int c);
DLLEXPORT void set_argv_var(char **argv, int argc);
DLLEXPORT bool is_luafunc(partial_T *partial) FUNC_ATTR_PURE;
DLLEXPORT const char *skip_luafunc_name(const char *p) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE
                                                                                            FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int check_luafunc_name(const char *const str,
                                 const bool paren) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE
                                                                                                               FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *char_from_string(const char *str, varnumber_T index);
DLLEXPORT char *string_slice(const char *str, varnumber_T first, varnumber_T last, bool exclusive);
DLLEXPORT int handle_subscript(const char **const arg, typval_T *rettv, evalarg_T *const evalarg,
                               bool verbose);
DLLEXPORT void set_selfdict(typval_T *const rettv, dict_T *const selfdict);
DLLEXPORT int var_item_copy(const vimconv_T *const conv, typval_T *const from, typval_T *const to,
                            const bool deep, const int copyID) FUNC_ATTR_NONNULL_ARG(2, 3);
DLLEXPORT void ex_echo(exarg_T *eap);
DLLEXPORT void ex_echohl(exarg_T *eap);
DLLEXPORT int get_echo_hl_id(void);
DLLEXPORT void ex_execute(exarg_T *eap);
DLLEXPORT const char *find_option_var_end(const char **const arg, OptIndex *const opt_idxp,
                                          int *const opt_flags);
DLLEXPORT var_flavour_T var_flavour(char *varname) FUNC_ATTR_PURE;
DLLEXPORT void var_set_global(const char *const name, typval_T vartv);
DLLEXPORT void last_set_msg(sctx_T script_ctx);
DLLEXPORT char *do_string_sub(char *str, size_t len, char *pat, char *sub, typval_T *expr,
                              const char *flags, size_t *ret_len);
DLLEXPORT bool common_job_callbacks(dict_T *vopts, CallbackReader *on_stdout,
                                    CallbackReader *on_stderr, Callback *on_exit);
DLLEXPORT Channel *find_job(uint64_t id, bool show_error);
DLLEXPORT void script_host_eval(char *name, typval_T *argvars, typval_T *rettv);
DLLEXPORT typval_T eval_call_provider(char *provider, char *method, list_T *arguments,
                                      bool discard);
DLLEXPORT bool eval_has_provider(const char *feat, bool throw_if_fast);
DLLEXPORT void eval_fmt_source_name_line(char *buf, size_t bufsize);
DLLEXPORT char *prompt_get_input(buf_T *buf);
DLLEXPORT int typval_compare(typval_T *typ1, typval_T *typ2, exprtype_T type,
                             bool ic) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *typval_tostring(typval_T *arg, bool quotes);

#include "nvim/func_attr.h"
