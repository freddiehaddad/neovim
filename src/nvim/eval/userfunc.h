#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/eval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/hashtab_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

// From user function to hashitem and back.
#define UF2HIKEY(fp) ((fp)->uf_name)
#define HIKEY2UF(p)  ((ufunc_T *)((p) - offsetof(ufunc_T, uf_name)))
#define HI2UF(hi)    HIKEY2UF((hi)->hi_key)

// flags used in uf_flags
#define FC_ABORT    0x01          // abort function on error
#define FC_RANGE    0x02          // function accepts range
#define FC_DICT     0x04          // Dict function, uses "self"
#define FC_CLOSURE  0x08          // closure, uses outer scope variables
#define FC_DELETED  0x10          // :delfunction used while uf_refcount > 0
#define FC_REMOVED  0x20          // function redefined while uf_refcount > 0
#define FC_SANDBOX  0x40          // function defined in the sandbox
// #define FC_DEAD     0x80          // function kept only for reference to dfunc
// #define FC_EXPORT   0x100         // "export def Func()"
#define FC_NOARGS   0x200         // no a: variables in lambda
// #define FC_VIM9     0x400         // defined in vim9 script file
#define FC_LUAREF  0x800          // luaref callback

/// Structure used by trans_function_name()
typedef struct {
  dict_T *fd_dict;    ///< Dict used.
  char *fd_newkey;    ///< New key in "dict" in allocated memory.
  dictitem_T *fd_di;  ///< Dict item used.
} funcdict_T;

typedef struct funccal_entry funccal_entry_T;
struct funccal_entry {
  void *top_funccal;
  funccal_entry_T *next;
};

/// errors for when calling a function
typedef enum {
  FCERR_UNKNOWN = 0,
  FCERR_TOOMANY = 1,
  FCERR_TOOFEW = 2,
  FCERR_SCRIPT = 3,
  FCERR_DICT = 4,
  FCERR_NONE = 5,
  FCERR_OTHER = 6,
  FCERR_DELETED = 7,
  FCERR_NOTMETHOD = 8,  ///< function cannot be used as a method
} FnameTransError;

/// Used in funcexe_T. Returns the new argcount.
typedef int (*ArgvFunc)(int current_argcount, typval_T *argv, int partial_argcount,
                        ufunc_T *called_func);

/// Structure passed between functions dealing with function call execution.
typedef struct {
  ArgvFunc fe_argv_func;  ///< when not NULL, can be used to fill in arguments only
                          ///< when the invoked function uses them
  linenr_T fe_firstline;  ///< first line of range
  linenr_T fe_lastline;   ///< last line of range
  bool *fe_doesrange;     ///< [out] if not NULL: function handled range
  bool fe_evaluate;       ///< actually evaluate expressions
  partial_T *fe_partial;  ///< for extra arguments
  dict_T *fe_selfdict;    ///< Dict for "self"
  typval_T *fe_basetv;    ///< base for base->method()
  bool fe_found_var;      ///< if the function is not found then give an
                          ///< error that a variable is not callable.
} funcexe_T;

#define FUNCEXE_INIT (funcexe_T) { \
  .fe_argv_func = NULL, \
  .fe_firstline = 0, \
  .fe_lastline = 0, \
  .fe_doesrange = NULL, \
  .fe_evaluate = false, \
  .fe_partial = NULL, \
  .fe_selfdict = NULL, \
  .fe_basetv = NULL, \
  .fe_found_var = false, \
}

#define FUNCARG(fp, j)  ((char **)(fp->uf_args.ga_data))[j]
#define FUNCLINE(fp, j) ((char **)(fp->uf_lines.ga_data))[j]

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void func_init(void);
DLLEXPORT hashtab_T *func_tbl_get(void);
DLLEXPORT int get_lambda_tv(char **arg, typval_T *rettv, evalarg_T *evalarg);
DLLEXPORT char *deref_func_name(const char *name, int *lenp, partial_T **const partialp,
                                bool no_autoload, bool *found_var)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT void emsg_funcname(const char *errmsg, const char *name);
DLLEXPORT int get_func_tv(const char *name, int len, typval_T *rettv, char **arg,
                          evalarg_T *const evalarg, funcexe_T *funcexe);
DLLEXPORT int get_func_arity(const char *name, int *required, int *optional, bool *varargs);
DLLEXPORT ufunc_T *find_func(const char *name);
DLLEXPORT funccall_T *create_funccal(ufunc_T *fp, typval_T *rettv);
DLLEXPORT void remove_funccal(void);
DLLEXPORT void call_user_func(ufunc_T *fp, int argcount, typval_T *argvars, typval_T *rettv,
                              linenr_T firstline, linenr_T lastline, dict_T *selfdict)
  FUNC_ATTR_NONNULL_ARG(1, 3, 4);
DLLEXPORT void save_funccal(funccal_entry_T *entry);
DLLEXPORT void restore_funccal(void);
DLLEXPORT funccall_T *get_current_funccal(void);
DLLEXPORT void set_current_funccal(funccall_T *fc);
DLLEXPORT void free_all_functions(void);
DLLEXPORT int func_call(char *name, typval_T *args, partial_T *partial, dict_T *selfdict,
                        typval_T *rettv);
DLLEXPORT varnumber_T callback_call_retnr(Callback *callback, int argcount, typval_T *argvars);
DLLEXPORT int call_func(const char *funcname, int len, typval_T *rettv, int argcount_in,
                        typval_T *argvars_in, funcexe_T *funcexe)
  FUNC_ATTR_NONNULL_ARG(1, 3, 5, 6);
DLLEXPORT int call_simple_luafunc(const char *funcname, size_t len, typval_T *rettv)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int call_simple_func(const char *funcname, size_t len, typval_T *rettv)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *printable_func_name(ufunc_T *fp);
DLLEXPORT char *trans_function_name(char **pp, bool skip, int flags, funcdict_T *fdp,
                                    partial_T **partial)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT char *get_scriptlocal_funcname(char *funcname);
DLLEXPORT char *save_function_name(char **name, bool skip, int flags, funcdict_T *fudi);
DLLEXPORT void ex_function(exarg_T *eap);
DLLEXPORT int eval_fname_script(const char *const p);
DLLEXPORT bool translated_function_exists(const char *name);
DLLEXPORT bool function_exists(const char *const name, bool no_deref);
DLLEXPORT char *get_user_func_name(expand_T *xp, int idx);
DLLEXPORT void ex_delfunction(exarg_T *eap);
DLLEXPORT void func_unref(char *name);
DLLEXPORT void func_ptr_unref(ufunc_T *fp);
DLLEXPORT void func_ref(char *name);
DLLEXPORT void func_ptr_ref(ufunc_T *fp);
DLLEXPORT void ex_return(exarg_T *eap);
DLLEXPORT bool can_add_defer(void);
DLLEXPORT void add_defer(char *name, int argcount_arg, typval_T *argvars);
DLLEXPORT void invoke_all_defer(void);
DLLEXPORT void ex_call(exarg_T *eap);
DLLEXPORT bool do_return(exarg_T *eap, bool reanimate, bool is_cmd, void *rettv);
DLLEXPORT char *get_return_cmd(void *rettv);
DLLEXPORT char *get_func_line(int c, void *cookie, int indent, bool do_concat);
DLLEXPORT int func_has_ended(void *cookie);
DLLEXPORT int func_has_abort(void *cookie);
DLLEXPORT void make_partial(dict_T *const selfdict, typval_T *const rettv);
DLLEXPORT char *func_name(void *cookie);
DLLEXPORT linenr_T *func_breakpoint(void *cookie);
DLLEXPORT int *func_dbg_tick(void *cookie);
DLLEXPORT int func_level(void *cookie);
DLLEXPORT int current_func_returned(void);
DLLEXPORT bool free_unref_funccal(int copyID, int testing);
DLLEXPORT funccall_T *get_funccal(void);
DLLEXPORT dict_T *get_funccal_local_dict(void);
DLLEXPORT hashtab_T *get_funccal_local_ht(void);
DLLEXPORT dictitem_T *get_funccal_local_var(void);
DLLEXPORT dict_T *get_funccal_args_dict(void);
DLLEXPORT hashtab_T *get_funccal_args_ht(void);
DLLEXPORT dictitem_T *get_funccal_args_var(void);
DLLEXPORT void list_func_vars(int *first);
DLLEXPORT dict_T *get_current_funccal_dict(hashtab_T *ht);
DLLEXPORT hashitem_T *find_hi_in_scoped_ht(const char *name, hashtab_T **pht);
DLLEXPORT dictitem_T *find_var_in_scoped_ht(const char *name, const size_t namelen,
                                            int no_autoload);
DLLEXPORT bool set_ref_in_previous_funccal(int copyID);
DLLEXPORT bool set_ref_in_call_stack(int copyID);
DLLEXPORT bool set_ref_in_functions(int copyID);
DLLEXPORT bool set_ref_in_func_args(int copyID);
DLLEXPORT bool set_ref_in_func(char *name, ufunc_T *fp_in, int copyID);
DLLEXPORT char *register_luafunc(LuaRef ref);
