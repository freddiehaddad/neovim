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

/// Gets all autocommands matching ALL criteria in the {opts} query.
///
/// Note: When multiple patterns or events are provided they have "OR" semantics (any combination
/// is matched).
///
/// Examples:
///
/// ```lua
/// -- Matches all criteria
/// autocommands = vim.api.nvim_get_autocmds({
///   group = 'MyGroup',
///   event = {'BufEnter', 'BufWinEnter'},
///   pattern = {'*.c', '*.h'}
/// })
///
/// -- All commands from one group
/// autocommands = vim.api.nvim_get_autocmds({
///   group = 'MyGroup',
/// })
/// ```
///
/// @param opts Dict with at least one of these keys:
///        - buf: (`integer[]|integer?`) Buffer id or list of buffer ids, for buffer-local autocommands
///          |autocmd-buflocal|. Not allowed with {pattern}.
///        - event: (`vim.api.keyset.events|vim.api.keyset.events[]?`) Event(s) to match |autocmd-events|.
///        - group: (`string|table?`) Group name or id to match.
///        - id: (`integer?`) Autocommand ID to match.
///        - pattern: (`string|table?`) Pattern(s) to match |autocmd-pattern|. Not allowed with {buf}.
/// @return Array of matching autocommands, where each item has:
///         - buf (`integer?`): Buffer id (only for |autocmd-buffer-local|).
///         - buflocal (`boolean?`): true if the autocommand is buffer-local |autocmd-buffer-local|.
///         - callback: (`function|string?`): Event handler: a Lua function or Vimscript function name.
///         - command: (`string`) Event handler: an Ex-command. Empty if a `callback` is set.
///         - desc: (`string`) Description.
///         - event: (`vim.api.keyset.events`) Event name(s).
///         - group: (`integer`) Group id.
///         - group_name: (`string`) Group name.
///         - id: (`integer`) Autocommand id (only when defined with the API).
///         - once: (`boolean`) true if |autocmd-once| was set.
///         - pattern: (`string`) Autocommand pattern.
DLLEXPORT ArrayOf(DictAs(get_autocmds__ret)) nvim_get_autocmds(Dict(get_autocmds) *opts,
                                                               Arena *arena,
                                                               Error *err) FUNC_API_SINCE(9);

/// Creates an |autocommand| event handler, defined by `callback` (Lua function or Vimscript
/// function _name_ string) or `command` (Ex command string).
///
/// Example using Lua callback:
///
/// ```lua
/// vim.api.nvim_create_autocmd({'BufEnter', 'BufWinEnter'}, {
///   pattern = {'*.c', '*.h'},
///   callback = function(ev)
///     print(string.format('event fired: %s', vim.inspect(ev)))
///   end
/// })
/// ```
///
/// Example using an Ex command as the handler:
///
/// ```lua
/// vim.api.nvim_create_autocmd({'BufEnter', 'BufWinEnter'}, {
///   pattern = {'*.c', '*.h'},
///   command = "echo 'Entering a C or C++ file'",
/// })
/// ```
///
/// Note: `pattern` is NOT automatically expanded (unlike with |:autocmd|), thus names like "$HOME"
/// and "~" must be expanded explicitly:
///
/// ```lua
/// pattern = vim.fn.expand('~') .. '/some/path/*.py'
/// ```
///
/// @param event Event(s) that will trigger the handler (`callback` or `command`): one handler is
///        created for each event name.
/// @param opts Options dict:
///        - buf (`integer?`) Buffer id for buffer-local autocommands |autocmd-buflocal|.
///          Not allowed with {pattern}.
///        - callback (`function|string?`) Lua function (or Vimscript function name, if string)
///          called when the event(s) is triggered. Lua callback can return |lua-truthy| to delete
///          the autocommand. Callback receives one argument, a table with keys: [event-args]()
///            - buf: (`integer`) [<abuf>]
///            - data: (`any`) Arbitrary data passed from [nvim_exec_autocmds()] [event-data]()
///            - event: (`vim.api.keyset.events`) Name of the triggered event |autocmd-events|
///            - file: (`string`) [<afile>] (not expanded to a full path)
///            - group: (`integer?`) Group id, if any
///            - id: (`integer`) Autocommand id
///            - match: (`string`) [<amatch>] (expanded to a full path)
///            - win: (`integer`) id of the window for the event, see |window-ID|
///        - command (string?) Vim command executed on event. Not allowed with {callback}.
///        - desc (`string?`) Description (for documentation and troubleshooting).
///        - group (`string|integer?`) Group name or id to match against.
///        - nested (`boolean?`, default: false) Run nested autocommands |autocmd-nested|.
///        - once (`boolean?`, default: false) Handle the event only once |autocmd-once|. If {event}
///          is a list, each handler will fire once.
///        - pattern (`string|array?`) Pattern(s) to match literally |autocmd-pattern|.
///
/// @return Autocommand id (number)
/// @see |autocommand|
/// @see |nvim_del_autocmd()|
DLLEXPORT Integer nvim_create_autocmd(uint64_t channel_id, Object event,
                                      Dict(create_autocmd) *opts, Arena *arena,
                                      Error *err) FUNC_API_SINCE(9) FUNC_API_FAST;

/// Deletes an autocommand by id.
///
/// @param id Autocommand id returned by |nvim_create_autocmd()|
DLLEXPORT void nvim_del_autocmd(Integer id, Error *err) FUNC_API_SINCE(9);

/// Clears all autocommands matching the {opts} query. To delete autocmds see |nvim_del_autocmd()|.
///
/// @param opts Optional parameters:
///        - buf: (`integer?`) Select |autocmd-buflocal| autocommands. Not allowed with {pattern}.
///        - event: (`vim.api.keyset.events|vim.api.keyset.events[]?`)
///          Examples:
///          - event: "pat1"
///          - event: { "pat1" }
///          - event: { "pat1", "pat2", "pat3" }
///        - group: (`string|int?`) Group name or id.
///          - NOTE: If not given, matches autocmds *not* in any group.
///        - pattern: (`string|table?`) Filter by patterns (exact match). Not allowed with {buf}.
///          - Example: if you have `*.py` as that pattern for the autocmd, you must pass `*.py`
///            exactly to clear it. `test.py` will not match the pattern.
///
DLLEXPORT void nvim_clear_autocmds(Dict(clear_autocmds) *opts, Arena *arena,
                                   Error *err) FUNC_API_SINCE(9);

/// Create or get an autocommand group |autocmd-groups|.
///
/// To get an existing group id, do:
///
/// ```lua
/// local id = vim.api.nvim_create_augroup('my.lsp.config', {
///   clear = false
/// })
/// ```
///
/// @param name Group name
/// @param opts Optional parameters:
///        - clear (`boolean?`, default: true) Clear existing commands in the group |autocmd-groups|.
/// @return Group id.
/// @see |autocmd-groups|
DLLEXPORT Integer nvim_create_augroup(uint64_t channel_id, String name,
                                      Dict(create_augroup) *opts, Error *err) FUNC_API_SINCE(9);

/// Delete an autocommand group by id.
///
/// To get a group id one can use |nvim_get_autocmds()|.
///
/// NOTE: behavior differs from |:augroup-delete|. When deleting a group, autocommands contained in
/// this group will also be deleted and cleared. This group will no longer exist.
/// @param id Group id.
/// @see |nvim_del_augroup_by_name()|
/// @see |nvim_create_augroup()|
DLLEXPORT void nvim_del_augroup_by_id(Integer id, Error *err) FUNC_API_SINCE(9);

/// Delete an autocommand group by name.
///
/// NOTE: behavior differs from |:augroup-delete|. When deleting a group, autocommands contained in
/// this group will also be deleted and cleared. This group will no longer exist.
/// @param name Group name.
/// @see |autocmd-groups|
DLLEXPORT void nvim_del_augroup_by_name(String name, Error *err) FUNC_API_SINCE(9);

/// Executes {event} handlers matching the {opts} query, in the context of {buf} (if given). |autocmd-execute|
///
/// @param event Event(s) to execute.
/// @param opts Optional filters:
///        - buf (`integer?`) Buffer where the event is applied. |autocmd-buflocal| Not allowed with {pattern}.
///        - data (`any`): Arbitrary data passed to the callback. See |nvim_create_autocmd()|.
///        - group (`string|integer?`) Group name or id to match against. |autocmd-groups|.
///        - modeline (`boolean?`, default: true) Process the modeline after the autocommands
///          [<nomodeline>]. Ignored if `buf` is given.
///        - pattern (`string|array?`, default: current file name) |autocmd-pattern|. Not allowed with {buf}.
/// @see |:doautocmd|
DLLEXPORT void nvim_exec_autocmds(Object event, Dict(exec_autocmds) *opts, Arena *arena,
                                  Error *err) FUNC_API_SINCE(9);
#include "nvim/func_attr.h"
