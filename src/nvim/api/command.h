#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep

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

/// Parse command line.
///
/// @param str       Command line string to parse. Cannot contain "\n".
/// @param opts      Optional parameters. Reserved for future use.
/// @param[out] err  Error details, if any.
/// @return Dict containing command information
DLLEXPORT Dict(cmd) nvim_parse_cmd(String str, Dict(empty) *opts, Arena *arena, Error *err)
  FUNC_API_SINCE(10) FUNC_API_FAST;

/// Executes an Ex command `cmd`, specified as a Dict with the same structure as returned by
/// |nvim_parse_cmd()|.
///
/// @param cmd       Command to execute
/// @param opts      Optional parameters.
/// @param[out] err  Error details, if any.
/// @return Command output (non-error, non-shell |:!|) if `output` is true, else empty string.
DLLEXPORT String nvim_cmd(uint64_t channel_id, Dict(cmd) *cmd, Dict(cmd_opts) *opts, Arena *arena,
                          Error *err)
  FUNC_API_SINCE(10);

// uncrustify:off

/// Creates a global |user-commands| command.
///
/// @param  name    Command name. First char must be uppercase.
/// @param  cmd     Command or Lua function, executed when the command is invoked.
/// @param  opts    Optional flags
/// @param[out] err Error details, if any.
DLLEXPORT void nvim_create_user_command(uint64_t channel_id,
                                        String name,
                                        Union(String, LuaRefOf((DictAs(create_user_command__command_args) args))) cmd,
                                        Dict(user_command) *opts,
                                        Error *err)
  FUNC_API_SINCE(9);

// uncrustify:on

/// Delete a user-defined command.
///
/// @param  name    Name of the command to delete.
/// @param[out] err Error details, if any.
DLLEXPORT void nvim_del_user_command(String name, Error *err)
  FUNC_API_SINCE(9);

/// Creates a buffer-local command |user-commands|.
///
/// @param  buf  Buffer id, or 0 for current buffer.
/// @param[out] err Error details, if any.
/// @see nvim_create_user_command
DLLEXPORT void nvim_buf_create_user_command(uint64_t channel_id, Buffer buf, String name,
                                            Object cmd, Dict(user_command) *opts, Error *err)
  FUNC_API_SINCE(9);

/// Delete a buffer-local user-defined command.
///
/// @param  buf  Buffer id, or 0 for current buffer.
/// @param  name    Name of the command to delete.
/// @param[out] err Error details, if any.
DLLEXPORT void nvim_buf_del_user_command(Buffer buf, String name, Error *err)
  FUNC_API_SINCE(9);

DLLEXPORT void create_user_command(uint64_t channel_id, String name, Union(String, LuaRef) cmd,
                                   Dict(user_command) *opts, int flags, Error *err);

/// Gets a map of global (non-buffer-local) Ex commands.
///
/// @param  opts  Optional parameters.
/// @param[out]  err   Error details, if any.
/// @returns Map of maps describing commands.
DLLEXPORT DictOf(DictAs(command_info)) nvim_get_commands(Dict(get_commands) *opts, Arena *arena,
                                                         Error *err)
  FUNC_API_SINCE(4);

/// Gets a map of buffer-local |user-commands|.
///
/// @param  buf  Buffer id, or 0 for current buffer
/// @param  opts  Optional parameters. Currently not used.
/// @param[out]  err   Error details, if any.
/// @returns Map of maps describing commands.
DLLEXPORT DictAs(command_info) nvim_buf_get_commands(Buffer buf, Dict(get_commands) *opts,
                                                     Arena *arena, Error *err)
  FUNC_API_SINCE(4);

#include "nvim/func_attr.h"
