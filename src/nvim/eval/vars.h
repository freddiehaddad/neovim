#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/eval_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/hashtab_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Array mapping values from MessagePackType to corresponding list pointers
extern const list_T *eval_msgpack_type_lists[NUM_MSGPACK_TYPES];

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void evalvars_init(void);
DLLEXPORT void evalvars_clear(void);
DLLEXPORT int garbage_collect_globvars(int copyID);
DLLEXPORT bool garbage_collect_vimvars(int copyID);
DLLEXPORT bool garbage_collect_scriptvars(int copyID);
DLLEXPORT void set_internal_string_var(const char *name, char *value) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int eval_charconvert(const char *const enc_from, const char *const enc_to,
                               const char *const fname_from, const char *const fname_to);
DLLEXPORT void eval_diff(const char *const origfile, const char *const newfile,
                         const char *const outfile);
DLLEXPORT void eval_patch(const char *const origfile, const char *const difffile,
                          const char *const outfile);
DLLEXPORT list_T *eval_spell_expr(char *badword, char *expr);
DLLEXPORT int get_spellword(list_T *const list, const char **ret_word);
DLLEXPORT void prepare_vimvar(int idx, typval_T *save_tv);
DLLEXPORT void restore_vimvar(int idx, typval_T *save_tv);
DLLEXPORT char *eval_one_expr_in_str(char *p, garray_T *gap, bool evaluate);
DLLEXPORT list_T *heredoc_get(exarg_T *eap, char *cmd, bool script_get);
DLLEXPORT void ex_let(exarg_T *eap);
DLLEXPORT int ex_let_vars(char *arg_start, typval_T *tv, int copy, int semicolon, int var_count,
                          int is_const, char *op);
DLLEXPORT const char *skip_var_list(const char *arg, int *var_count, int *semicolon, bool silent);
DLLEXPORT void list_hashtable_vars(hashtab_T *ht, const char *prefix, int empty, int *first);
DLLEXPORT void ex_unlet(exarg_T *eap);
DLLEXPORT void ex_lockvar(exarg_T *eap);
DLLEXPORT int do_unlet(const char *const name, const size_t name_len, const bool forceit)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void del_menutrans_vars(void);
DLLEXPORT dict_T *get_globvar_dict(void) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_RET;
DLLEXPORT hashtab_T *get_globvar_ht(void);
DLLEXPORT dict_T *get_vimvar_dict(void) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_RET;
DLLEXPORT void set_vim_var_tv(const VimVarIndex idx, typval_T *const tv);
DLLEXPORT char *get_vim_var_name(const VimVarIndex idx) FUNC_ATTR_NONNULL_RET;
DLLEXPORT typval_T *get_vim_var_tv(const VimVarIndex idx);
DLLEXPORT varnumber_T get_vim_var_nr(const VimVarIndex idx) FUNC_ATTR_PURE;
DLLEXPORT list_T *get_vim_var_list(const VimVarIndex idx) FUNC_ATTR_PURE;
DLLEXPORT dict_T *get_vim_var_dict(const VimVarIndex idx) FUNC_ATTR_PURE;
DLLEXPORT char *get_vim_var_str(const VimVarIndex idx) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_RET;
DLLEXPORT partial_T *get_vim_var_partial(const VimVarIndex idx) FUNC_ATTR_PURE;
DLLEXPORT char *cat_prefix_varname(int prefix, const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *get_user_var_name(expand_T *xp, int idx);
DLLEXPORT void set_vim_var_type(const VimVarIndex idx, const VarType type);
DLLEXPORT void set_vim_var_nr(const VimVarIndex idx, const varnumber_T val);
DLLEXPORT void set_vim_var_bool(const VimVarIndex idx, const BoolVarValue val);
DLLEXPORT void set_vim_var_special(const VimVarIndex idx, const SpecialVarValue val);
DLLEXPORT void set_vim_var_char(int c);
DLLEXPORT void set_vim_var_string(const VimVarIndex idx, const char *const val,
                                  const ptrdiff_t len);
DLLEXPORT void set_vim_var_list(const VimVarIndex idx, list_T *const val);
DLLEXPORT void set_vim_var_dict(const VimVarIndex idx, dict_T *const val);
DLLEXPORT void set_vim_var_partial(const VimVarIndex idx, partial_T *val);
DLLEXPORT void set_reg_var(int c);
DLLEXPORT char *v_exception(char *oldval);
DLLEXPORT char *set_cmdarg(exarg_T *eap, char *oldarg);
DLLEXPORT char *v_throwpoint(char *oldval);
DLLEXPORT void set_vcount(int64_t count, int64_t count1, bool set_prevcount);
DLLEXPORT int eval_variable(const char *name, int len, typval_T *rettv, dictitem_T **dip,
                            bool verbose, bool no_autoload);
DLLEXPORT void check_vars(const char *name, size_t len);
DLLEXPORT dictitem_T *find_var(const char *const name, const size_t name_len, hashtab_T **htp,
                               int no_autoload);
DLLEXPORT dictitem_T *find_var_in_ht(hashtab_T *const ht, int htname, const char *const varname,
                                     const size_t varname_len, int no_autoload)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT hashtab_T *find_var_ht(const char *name, const size_t name_len, const char **varname);
DLLEXPORT char *get_var_value(const char *const name);
DLLEXPORT void new_script_vars(scid_T id);
DLLEXPORT void init_var_dict(dict_T *dict, ScopeDictDictItem *dict_var, ScopeType scope);
DLLEXPORT void unref_var_dict(dict_T *dict);
DLLEXPORT void vars_clear(hashtab_T *ht);
DLLEXPORT void vars_clear_ext(hashtab_T *ht, bool free_val);
DLLEXPORT bool before_set_vvar(const char *const varname, dictitem_T *const di, typval_T *const tv,
                               const bool copy, const bool watched, bool *const type_error);
DLLEXPORT void set_var(const char *name, const size_t name_len, typval_T *const tv,
                       const bool copy) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void set_var_const(const char *name, const size_t name_len, typval_T *const tv,
                             const bool copy, const bool is_const) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool var_check_ro(const int flags, const char *name, size_t name_len)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool var_check_lock(const int flags, const char *name, size_t name_len);
DLLEXPORT bool var_check_fixed(const int flags, const char *name, size_t name_len)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool var_wrong_func_name(const char *const name, const bool new_var)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool valid_varname(const char *varname)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT typval_T opt_to_tv(Object value, bool numbool);
DLLEXPORT void reset_v_option_vars(void);
DLLEXPORT void assert_error(garray_T *gap);
DLLEXPORT bool var_exists(const char *var) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int var_redir_start(char *name, bool append);
DLLEXPORT void var_redir_str(const char *value, int value_len);
DLLEXPORT void var_redir_stop(void);
DLLEXPORT void f_gettabvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_gettabwinvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getwinvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getbufvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_settabvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_settabwinvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setwinvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_setbufvar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
