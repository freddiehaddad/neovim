#pragma once

#include <stdint.h>  // IWYU pragma: keep
#include <stdio.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/mapping_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/regexp_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

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

DLLEXPORT mapblock_T *get_maphash_list(int state, int c);
DLLEXPORT mapblock_T *get_buf_maphash_list(int state, int c);
DLLEXPORT LuaRef map_luaid_get(int n);
DLLEXPORT void map_mode_to_chars(int mode, char *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int do_map(int maptype, char *arg, int mode, bool is_abbrev);
DLLEXPORT void map_clear_mode(buf_T *buf, int mode, bool local, bool abbr);
DLLEXPORT bool map_to_exists(const char *const str, const char *const modechars, const bool abbr)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT bool map_to_exists_mode(const char *const rhs, const int mode, const bool abbr);
DLLEXPORT char *set_context_in_map_cmd(expand_T *xp, char *cmd, char *arg, bool forceit,
                                       bool isabbrev, bool isunmap, cmdidx_T cmdidx);
DLLEXPORT int ExpandMappings(char *pat, regmatch_T *regmatch, int *numMatches, char ***matches);
DLLEXPORT bool check_abbr(int c, char *ptr, int col, int mincol);
DLLEXPORT char *eval_map_expr(mapblock_T *mp, int c);
DLLEXPORT int makemap(FILE *fd, buf_T *buf);
DLLEXPORT int put_escstr(FILE *fd, const char *strstart, int what);
DLLEXPORT char *check_map(char *keys, int mode, int exact, int ign_mod, int abbr,
                          mapblock_T **mp_ptr, int *local_ptr, int *rhs_lua);
DLLEXPORT void f_hasmapto(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_mapset(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_maplist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_maparg(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_mapcheck(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void add_map(char *lhs, char *rhs, int mode, bool buffer);
DLLEXPORT int langmap_adjust_mb(int c);
DLLEXPORT void langmap_init(void);
DLLEXPORT const char *did_set_langmap(optset_T *args);
DLLEXPORT void ex_abbreviate(exarg_T *eap);
DLLEXPORT void ex_map(exarg_T *eap);
DLLEXPORT void ex_unmap(exarg_T *eap);
DLLEXPORT void ex_mapclear(exarg_T *eap);
DLLEXPORT void ex_abclear(exarg_T *eap);
DLLEXPORT void modify_keymap(uint64_t channel_id, Buffer buffer, int maptype, String mode,
                             String lhs, String rhs, Dict(keymap) *opts, Error *err);
DLLEXPORT ArrayOf(Dict) keymap_array(String mode, buf_T *buf, Arena *arena);

#include "nvim/func_attr.h"

EXTERN int repeat_luamap INIT( = 0);  ///< Lua-mapping id for dot-repeat. 0: none.

/// Used for the first argument of do_map()
enum {
  MAPTYPE_MAP       = 0,
  MAPTYPE_UNMAP     = 1,
  MAPTYPE_NOREMAP   = 2,
  MAPTYPE_UNMAP_LHS = 3,
};

/// Adjust chars in a language according to 'langmap' option.
/// NOTE that there is no noticeable overhead if 'langmap' is not set.
/// When set the overhead for characters < 256 is small.
/// Don't apply 'langmap' if the character comes from the Stuff buffer or from a
/// mapping and the langnoremap option was set.
/// The do-while is just to ignore a ';' after the macro.
#define LANGMAP_ADJUST(c, condition) \
  do { \
    if (*p_langmap \
        && (condition) \
        && (p_lrm || (vgetc_busy ? typebuf_maplen() == 0 : KeyTyped)) \
        && !KeyStuffed \
        && (c) >= 0) \
    { \
      if ((c) < 256) \
      c = langmap_mapchar[c]; \
      else \
      c = langmap_adjust_mb(c); \
    } \
  } while (0)
