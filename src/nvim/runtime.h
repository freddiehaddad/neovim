#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/runtime_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Stack of execution contexts.  Each entry is an estack_T.
/// Current context is at ga_len - 1.
extern garray_T exestack;
#define HAVE_SOURCING_INFO  (exestack.ga_data != NULL && exestack.ga_len > 0)
/// name of error message source
#define SOURCING_NAME (((estack_T *)exestack.ga_data)[exestack.ga_len - 1].es_name)
/// line number in the message source or zero
#define SOURCING_LNUM (((estack_T *)exestack.ga_data)[exestack.ga_len - 1].es_lnum)

/// Growarray to store info about already sourced scripts.
extern garray_T script_items;
#define SCRIPT_ITEM(id) (((scriptitem_T **)script_items.ga_data)[(id) - 1])
#define SCRIPT_ID_VALID(id) ((id) > 0 && (id) <= script_items.ga_len)

/// last argument for do_source()
enum {
  DOSO_NONE = 0,
  DOSO_VIMRC = 1,  ///< loading vimrc file
};

/// Used for flags in do_in_path()
enum {
  DIP_ALL     = 0x01,   ///< all matches, not just the first one
  DIP_DIR     = 0x02,   ///< find directories instead of files
  DIP_ERR     = 0x04,   ///< give an error message when none found
  DIP_START   = 0x08,   ///< also use "start" directory in 'packpath'
  DIP_OPT     = 0x10,   ///< also use "opt" directory in 'packpath'
  DIP_NORTP   = 0x20,   ///< do not use 'runtimepath'
  DIP_NOAFTER = 0x40,   ///< skip "after" directories
  DIP_AFTER   = 0x80,   ///< only use "after" directories
  DIP_DIRFILE = 0x200,  ///< find both files and directories
};

#ifdef ABORT_ON_INTERNAL_ERROR
# define ESTACK_CHECK_DECLARATION int estack_len_before
# define ESTACK_CHECK_SETUP do { estack_len_before = exestack.ga_len; } while (0)
# define ESTACK_CHECK_NOW \
  do { \
    if (estack_len_before != exestack.ga_len) { \
      siemsg("Exestack length expected: %d, actual: %d", estack_len_before, exestack.ga_len); \
    } \
  } while (0)
#else
# define ESTACK_CHECK_DECLARATION do {} while (0)
# define ESTACK_CHECK_SETUP do {} while (0)
# define ESTACK_CHECK_NOW do {} while (0)
#endif

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

DLLEXPORT void estack_init(void);
DLLEXPORT estack_T *estack_push(etype_T type, char *name, linenr_T lnum);
DLLEXPORT void estack_push_ufunc(ufunc_T *ufunc, linenr_T lnum);
DLLEXPORT void estack_pop(void);
DLLEXPORT char *estack_sfile(estack_arg_T which);
DLLEXPORT list_T *stacktrace_create(void);
DLLEXPORT void f_getstacktrace(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void runtime_init(void);
DLLEXPORT void ex_runtime(exarg_T *eap);
DLLEXPORT void set_context_in_runtime_cmd(expand_T *xp, const char *arg);
DLLEXPORT int do_in_path(const char *path, const char *prefix, char *name, int flags,
                         DoInRuntimepathCB callback, void *cookie) FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT Array runtime_inspect(Arena *arena);
DLLEXPORT ArrayOf(String) runtime_get_named(bool lua, Array pat, bool all, Arena *arena);
DLLEXPORT ArrayOf(String) runtime_get_named_thread(bool lua, Array pat, bool all);
DLLEXPORT int do_in_path_and_pp(char *path, char *name, int flags, DoInRuntimepathCB callback,
                                void *cookie);
DLLEXPORT const char *did_set_runtimepackpath(optset_T *args);
DLLEXPORT void runtime_search_path_validate(void);
DLLEXPORT void update_runtime_search_path_thread(bool force);
DLLEXPORT int do_in_runtimepath(char *name, int flags, DoInRuntimepathCB callback, void *cookie);
DLLEXPORT int source_runtime(char *name, int flags);
DLLEXPORT int source_runtime_vim_lua(char *name, int flags);
DLLEXPORT int source_in_path_vim_lua(char *path, char *name, int flags);
DLLEXPORT void add_pack_start_dirs(void);
DLLEXPORT void load_start_packages(void);
DLLEXPORT void ex_packloadall(exarg_T *eap);
DLLEXPORT void load_plugins(void);
DLLEXPORT void ex_packadd(exarg_T *eap);
DLLEXPORT int ExpandRTDir(char *pat, int flags, int *num_file, char ***file, char *dirnames[]);
DLLEXPORT int expand_runtime_cmd(char *pat, int *numMatches, char ***matches);
DLLEXPORT int ExpandPackAddDir(char *pat, int *num_file, char ***file);
DLLEXPORT char *get_lib_dir(void);
DLLEXPORT char *runtimepath_default(bool clean_arg);
DLLEXPORT void ex_source(exarg_T *eap);
DLLEXPORT void ex_options(exarg_T *eap);
DLLEXPORT linenr_T *source_breakpoint(void *cookie);
DLLEXPORT int *source_dbg_tick(void *cookie);
DLLEXPORT int source_level(void *cookie) FUNC_ATTR_PURE;
DLLEXPORT scriptitem_T *new_script_item(char *const name, scid_T *const sid_out)
  FUNC_ATTR_NONNULL_RET;
DLLEXPORT void cmd_source_buffer(const exarg_T *const eap, bool ex_lua) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int do_source_str(const char *str, char *traceback_name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int do_source(char *fname, bool check_other, int is_vimrc, int *ret_sid);
DLLEXPORT bool script_is_lua(scid_T sid);
DLLEXPORT int find_script_by_name(char *name);
DLLEXPORT void ex_scriptnames(exarg_T *eap);
DLLEXPORT char *get_scriptname(sctx_T script_ctx, bool *should_free);
DLLEXPORT void free_scriptnames(void);
DLLEXPORT void free_autoload_scriptnames(void);
DLLEXPORT linenr_T get_sourced_lnum(LineGetter fgetline, void *cookie) FUNC_ATTR_PURE;
DLLEXPORT void f_getscriptinfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT char *getsourceline(int c, void *cookie, int indent, bool do_concat);
DLLEXPORT int sourcing_a_script(exarg_T *eap);
DLLEXPORT void ex_scriptencoding(exarg_T *eap);
DLLEXPORT void ex_finish(exarg_T *eap);
DLLEXPORT void do_finish(exarg_T *eap, bool reanimate);
DLLEXPORT bool source_finished(LineGetter fgetline, void *cookie);
DLLEXPORT char *autoload_name(const char *const name, const size_t name_len)
  FUNC_ATTR_MALLOC FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool script_autoload(const char *const name, const size_t name_len, const bool reload);

#include "nvim/func_attr.h"
