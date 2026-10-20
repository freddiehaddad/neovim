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
/// Doesn't check the validity of command arguments.
///
/// @param str       Command line string to parse. Cannot contain "\n".
/// @param opts      Optional parameters. Reserved for future use.
/// @param[out] err  Error details, if any.
/// @return Dict containing command information, with these keys:
///         - cmd: (string) Command name.
///         - range: (array) (optional) Command range ([<line1>] [<line2>]).
///                          Omitted if command doesn't accept a range.
///                          Otherwise, has no elements if no range was specified, one element if
///                          only a single range item was specified, or two elements if both range
///                          items were specified.
///         - count: (number) (optional) Command [<count>].
///                           Omitted if command cannot take a count.
///         - reg: (string) (optional) Command [<register>].
///                         Omitted if command cannot take a register.
///         - bang: (boolean) Whether command contains a [<bang>] (!) modifier.
///         - args: (array) Command arguments.
///         - addr: (string) Value of |:command-addr|. Uses short name or "line" for -addr=lines.
///         - nargs: (string) Value of |:command-nargs|.
///         - nextcmd: (string) Next command if there are multiple commands separated by a |:bar|.
///                             Empty if there isn't a next command.
///         - magic: (dict) Which characters have special meaning in the command arguments.
///             - file: (boolean) The command expands filenames. Which means characters such as "%",
///                               "#" and wildcards are expanded.
///             - bar: (boolean) The "|" character is treated as a command separator and the double
///                              quote character (") is treated as the start of a comment.
///         - mods: (dict) |:command-modifiers|.
///             - filter: (dict) |:filter|.
///                 - pattern: (string) Filter pattern. Empty string if there is no filter.
///                 - force: (boolean) Whether filter is inverted or not.
///             - silent: (boolean) |:silent|.
///             - emsg_silent: (boolean) |:silent!|.
///             - unsilent: (boolean) |:unsilent|.
///             - sandbox: (boolean) |:sandbox|.
///             - noautocmd: (boolean) |:noautocmd|.
///             - browse: (boolean) |:browse|.
///             - confirm: (boolean) |:confirm|.
///             - hide: (boolean) |:hide|.
///             - horizontal: (boolean) |:horizontal|.
///             - keepalt: (boolean) |:keepalt|.
///             - keepjumps: (boolean) |:keepjumps|.
///             - keepmarks: (boolean) |:keepmarks|.
///             - keeppatterns: (boolean) |:keeppatterns|.
///             - lockmarks: (boolean) |:lockmarks|.
///             - noswapfile: (boolean) |:noswapfile|.
///             - tab: (integer) |:tab|. -1 when omitted.
///             - verbose: (integer) |:verbose|. -1 when omitted.
///             - vertical: (boolean) |:vertical|.
///             - split: (string) Split modifier string, is an empty string when there's no split
///                               modifier. If there is a split modifier it can be one of:
///               - "aboveleft": |:aboveleft|.
///               - "belowright": |:belowright|.
///               - "topleft": |:topleft|.
///               - "botright": |:botright|.
DLLEXPORT Dict(cmd) nvim_parse_cmd(String str, Dict(empty) *opts, Arena *arena, Error *err)
  FUNC_API_SINCE(10) FUNC_API_FAST;

/// Executes an Ex command `cmd`, specified as a Dict with the same structure as returned by
/// |nvim_parse_cmd()|.
///
/// Use `magic={…=false}` to disable special chars (see also |open-file|):
/// ```lua
/// vim.api.nvim_cmd({
///     cmd = 'edit',
///     args = { '%foo"|bar#baz"' },
///     magic = { file = false, bar = false }
///   },
///   {}
/// )
/// ```
///
/// - See |nvim_parse_cmd()| to parse a cmdline string (which can then be passed to `nvim_cmd`).
/// - See |nvim_command()| to execute a cmdline string.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @see |nvim_command()|
/// @see |nvim_exec2()|
/// @see |nvim_parse_cmd()|
///
/// @param cmd       Command to execute, a Dict with the same structure as the return value of
///                  |nvim_parse_cmd()| (except "addr", "nargs" and "nextcmd" are ignored).
///                  All keys except "cmd" are optional.
/// @param opts      Optional parameters.
///                  - output: (boolean, default false) Whether to return command output.
/// @param[out] err  Error details, if any.
/// @return Command output (non-error, non-shell |:!|) if `output` is true, else empty string.
DLLEXPORT String nvim_cmd(uint64_t channel_id, Dict(cmd) *cmd, Dict(cmd_opts) *opts, Arena *arena,
                          Error *err)
  FUNC_API_SINCE(10);

// uncrustify:off

/// Creates a global |user-commands| command.
///
/// For Lua usage see |lua-guide-commands-create|.
///
/// Example:
///
/// ```vim
/// :call nvim_create_user_command('SayHello', 'echo "Hello world!"', {'bang': v:true})
/// :SayHello
/// Hello world!
/// ```
///
/// @param  name    Command name. First char must be uppercase.
/// @param  cmd     Command or Lua function, executed when the command is invoked. Lua function
///                 receives a table with keys:
///                 - args: (string) Args passed to the command, if any. [<args>]
///                 - bang: (boolean) true if the command was executed with "!". [<bang>]
///                 - count: (number) Count, if any. [<count>]
///                 - fargs: (table) Args split by unescaped whitespace (when more than one arg is
///                   allowed), if any. [<f-args>]
///                 - line1: (number) Start of the command range. [<line1>]
///                 - line2: (number) End of the command range. [<line2>]
///                 - mods: (string) Command modifiers (unstructured string), if any. [<mods>]
///                 - name: (string) Command name.
///                 - nargs: (string) Number of arguments allowed by the command. |:command-nargs|
///                 - range: (number) Number of items in the command range: 0, 1, or 2. [<range>]
///                 - reg: (string) Register name, if any. [<reg>]
///                 - smods: (table) Command modifiers (structured), same as "mods" in |nvim_parse_cmd()|.
/// @param  opts    Optional flags:
///                 - `addr` |:command-addr|
///                 - `complete` |:command-complete| command or function |:command-completion-customlist|.
///                 - `count` |:command-count|
///                 - `desc` (string) Command description.
///                 - `force` (boolean, default true) Override the existing definition, if any.
///                 - `nargs` Number of arguments allowed by the command. |:command-nargs|
///                 - `preview` (function) Preview handler for 'inccommand'. |:command-preview|
///                 - `range` |:command-range|
///                 - boolean |command-attributes| such as |:command-bang| or |:command-bar| (but
///                   not |:command-buffer|, use |nvim_buf_create_user_command()| instead).
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
/// Only commands created with |:command-buffer| or
/// |nvim_buf_create_user_command()| can be deleted with this function.
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
/// Currently only |user-commands| are supported, not builtin Ex commands.
///
/// @see |nvim_get_all_options_info()|
///
/// @param  opts  Optional parameters. Currently only supports
///               {"builtin":false}
/// @param[out]  err   Error details, if any.
///
/// @returns Map of maps describing commands.
DLLEXPORT DictOf(DictAs(command_info)) nvim_get_commands(Dict(get_commands) *opts, Arena *arena,
                                                         Error *err)
  FUNC_API_SINCE(4);

/// Gets a map of buffer-local |user-commands|.
///
/// @param  buf  Buffer id, or 0 for current buffer
/// @param  opts  Optional parameters. Currently not used.
/// @param[out]  err   Error details, if any.
///
/// @returns Map of maps describing commands.
DLLEXPORT DictAs(command_info) nvim_buf_get_commands(Buffer buf, Dict(get_commands) *opts,
                                                     Arena *arena, Error *err)
  FUNC_API_SINCE(4);

#include "nvim/func_attr.h"
