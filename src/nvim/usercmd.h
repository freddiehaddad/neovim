#pragma once

#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/ex_cmds_defs.h"
#include "nvim/garray_defs.h"
#include "nvim/types_defs.h"

typedef struct {
  char *uc_name;             ///< The command name
  uint32_t uc_argt;          ///< The argument type
  char *uc_rep;              ///< The command's replacement string
  int64_t uc_def;            ///< The default value for a range/count
  int uc_compl;              ///< completion type
  cmd_addr_T uc_addr_type;   ///< The command's address type
  sctx_T uc_script_ctx;      ///< SCTX where the command was defined
  char *uc_compl_arg;        ///< completion argument if any
  LuaRef uc_compl_luaref;    ///< Reference to Lua completion function
  LuaRef uc_preview_luaref;  ///< Reference to Lua preview function
  LuaRef uc_luaref;          ///< Reference to Lua function
  char *uc_desc;             ///< Command description
} ucmd_T;

enum { UC_BUFFER = 1, };  ///< -buffer: local to current buffer

extern garray_T ucmds;

#define USER_CMD(i) (&((ucmd_T *)(ucmds.ga_data))[i])
#define USER_CMD_GA(gap, i) (&((ucmd_T *)((gap)->ga_data))[i])

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT char *find_ucmd(exarg_T *eap, char *p, int *full, expand_T *xp, int *complp);
DLLEXPORT const char *set_context_in_user_cmd(expand_T *xp, const char *arg_in);
DLLEXPORT const char *set_context_in_user_cmdarg(const char *cmd FUNC_ATTR_UNUSED, const char *arg,
                                                 uint32_t argt, int context, expand_T *xp,
                                                 bool forceit);
DLLEXPORT char *expand_user_command_name(int idx);
DLLEXPORT char *get_user_commands(expand_T *xp FUNC_ATTR_UNUSED, int idx)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *get_user_command_name(int idx, int cmdidx);
DLLEXPORT char *get_user_cmd_addr_type(expand_T *xp, int idx);
DLLEXPORT char *get_user_cmd_flags(expand_T *xp, int idx);
DLLEXPORT char *get_user_cmd_nargs(expand_T *xp, int idx);
DLLEXPORT char *get_user_cmd_complete(expand_T *xp, int idx);
DLLEXPORT char *cmdcomplete_type_to_str(int expand, const char *compl_arg);
DLLEXPORT int cmdcomplete_str_to_type(const char *complete_str);
DLLEXPORT int parse_addr_type_arg(char *value, int vallen, cmd_addr_T *addr_type_arg)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int parse_compl_arg(const char *value, int vallen, int *complp, uint32_t *argt,
                              char **compl_arg) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *uc_validate_name(char *name);
DLLEXPORT int uc_add_command(char *name, size_t name_len, const char *rep, uint32_t argt,
                             int64_t def, int flags, int context, char *compl_arg,
                             LuaRef compl_luaref, LuaRef preview_luaref, cmd_addr_T addr_type,
                             LuaRef luaref, const char *desc, bool force) FUNC_ATTR_NONNULL_ARG(1,
                                                                                                3);
DLLEXPORT void ex_command(exarg_T *eap);
DLLEXPORT void ex_comclear(exarg_T *eap);
DLLEXPORT void free_ucmd(ucmd_T *cmd);
DLLEXPORT void uc_clear(garray_T *gap);
DLLEXPORT void ex_delcommand(exarg_T *eap);
DLLEXPORT bool uc_split_args_iter(const char *arg, size_t arglen, size_t *end, char *buf,
                                  size_t *len);
DLLEXPORT size_t uc_nargs_upper_bound(const char *arg, size_t arglen);
DLLEXPORT size_t add_win_cmd_modifiers(char *buf, const cmdmod_T *cmod, bool *multi_mods);
DLLEXPORT size_t uc_mods(char *buf, const cmdmod_T *cmod, bool quote);
DLLEXPORT int do_ucmd(exarg_T *eap, bool preview);
DLLEXPORT Dict commands_array(buf_T *buf, Arena *arena);
