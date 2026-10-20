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

/// Gets a highlight group by name
///
/// similar to |hlID()|, but allocates a new ID if not present.
/// @param[out] err Error details, if any
DLLEXPORT Integer nvim_get_hl_id_by_name(String name, Error *err) FUNC_API_SINCE(7);

/// Gets all or specific highlight groups in a namespace.
///
/// @note When the `link` attribute is defined in the highlight definition
///       map, other attributes will not be taking effect (see |:hi-link|).
///
/// @param ns_id Get highlight groups for namespace ns_id |nvim_get_namespaces()|.
///              Use 0 to get global highlight groups |:highlight|.
/// @param opts  Options dict:
///                 - create: (boolean, default true) When highlight group doesn't exist create it.
///                 - id: (integer) Get a highlight definition by id.
///                 - link: (boolean, default true) Show linked group name instead of effective definition |:hi-link|.
///                 - name: (string) Get a highlight definition by name.
///
/// @param[out] err Error details, if any.
/// @return Highlight groups as a map from group name to a highlight definition map as in |nvim_set_hl()|,
///                   or only a single highlight definition map if requested by name or id.
DLLEXPORT DictAs(get_hl_info) nvim_get_hl(Integer ns_id, Dict(get_highlight) *opts, Arena *arena,
                                          Error *err) FUNC_API_SINCE(11);

/// Sets a highlight group. By default, replaces the entire definition (e.g. `nvim_set_hl(0, 'Visual', {})`
/// will clear the "Visual" group), unless `update` is specified.
///
/// @note The fg and bg keys also accept the string values `"fg"` or `"bg"`
///       which act as aliases to the corresponding foreground and background
///       values of the Normal group. If the Normal group has not been defined,
///       using these values results in an error.
///
/// @note If `link` is used in combination with other attributes; only the
///       `link` will take effect (see |:hi-link|).
///
/// @param ns_id Namespace id for this highlight |nvim_create_namespace()|.
///              Use 0 to set a highlight group globally |:highlight|.
///              Highlights from non-global namespaces are not active by default, use
///              |nvim_set_hl_ns()| or |nvim_win_set_hl_ns()| to activate them.
/// @param name  Highlight group name, e.g. "ErrorMsg"
/// @param val   Highlight definition map, accepts the following keys:
///                - altfont: boolean
///                - bg: color name or "#RRGGBB", see note.
///                - bg_indexed: boolean. If true, `bg` is an RGB approximation of `ctermbg`
///                  (a palette index). UIs rendering cterm natively may prefer `ctermbg`.
///                - blend: integer between 0 and 100
///                - blink: boolean
///                - bold: boolean
///                - conceal: boolean Concealment at the UI level (terminal SGR), unrelated to |:syn-conceal|.
///                - cterm: cterm attribute map, like |highlight-args|. If not set, cterm attributes
///                  will match those from the attribute map documented above.
///                - ctermbg: Sets background of cterm color |ctermbg|
///                - ctermfg: Sets foreground of cterm color |ctermfg|
///                - default: boolean Don't override existing definition |:hi-default|
///                - dim: boolean
///                - fg: Color name or "#RRGGBB", see note.
///                - fg_indexed: boolean. Same as `bg_indexed`, for `fg` and `ctermfg`.
///                - font: GUI font name (string). Sets |highlight-font|. Use "NONE" to clear.
///                - force: (boolean, default: false) Update the highlight group even if it already exists.
///                - italic: boolean
///                - link: Name of highlight group to link to. |:hi-link|
///                - link_global: Like "link", but always resolved in the global namespace (ns=0).
///                - nocombine: boolean
///                - overline: boolean
///                - reverse: boolean
///                - sp: Color name or "#RRGGBB"
///                - standout: boolean
///                - strikethrough: boolean
///                - undercurl: boolean
///                - underdashed: boolean
///                - underdotted: boolean
///                - underdouble: boolean
///                - underline: boolean
///                - update: (boolean, default: false) Update specified attributes only, leave others unchanged.
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_hl(uint64_t channel_id, Integer ns_id, String name, Dict(highlight) *val,
                           Error *err) FUNC_API_SINCE(7);

/// Gets the active highlight namespace.
///
/// @param opts Optional parameters
///           - winid: (number) |window-ID| for retrieving a window's highlight
///             namespace. A value of -1 is returned when |nvim_win_set_hl_ns()|
///             has not been called for the window (or was called with a namespace
///             of -1).
/// @param[out] err Error details, if any
/// @return Namespace id, or -1
DLLEXPORT Integer nvim_get_hl_ns(Dict(get_ns) *opts, Error *err) FUNC_API_SINCE(12);

/// Set active namespace for highlights defined with |nvim_set_hl()|. This can be set for
/// a single window, see |nvim_win_set_hl_ns()|.
///
/// @param ns_id the namespace to use
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_hl_ns(Integer ns_id, Error *err) FUNC_API_SINCE(10);

/// Set active namespace for highlights defined with |nvim_set_hl()| while redrawing.
///
/// This function is meant to be called while redrawing, primarily from
/// |nvim_set_decoration_provider()| on_win and on_line callbacks, which
/// are allowed to change the namespace during a redraw cycle.
///
/// @param ns_id the namespace to activate
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_hl_ns_fast(Integer ns_id, Error *err) FUNC_API_SINCE(10) FUNC_API_FAST;

/// Sends input-keys to Nvim, subject to various quirks controlled by `mode` flags. This is
/// a blocking call, unlike |nvim_input()|.
///
/// On execution error: does not fail, but updates v:errmsg.
///
/// To input keycodes like [<C-o>], pass the result of |nvim_replace_termcodes()|:
///
/// ```vim
/// :let key = nvim_replace_termcodes('<C-o>', v:true, v:false, v:true)
/// :call nvim_feedkeys(key, 'n', v:false)
/// ```
///
/// @param keys         Keys to send as input.
/// @param mode         Behavior flags, see |feedkeys()|.
/// @param escape_ks    If true, escape K_SPECIAL bytes in `keys`. Should be false if you used
///                     |nvim_replace_termcodes()|, else true.
/// @see feedkeys()
/// @see vim_strsave_escape_ks
DLLEXPORT void nvim_feedkeys(String keys, String mode, Boolean escape_ks) FUNC_API_SINCE(1);

/// Queues raw user-input. Unlike |nvim_feedkeys()|, this uses a low-level input buffer and the call
/// is non-blocking (input is processed asynchronously by the eventloop).
///
/// To input blocks of text, |nvim_paste()| is much faster and should be preferred.
///
/// On execution error: does not fail, but updates v:errmsg.
///
/// @note |keycodes| like [<CR>] are translated, so "<" is special.
///       To input a literal "<", send [<LT>].
///
/// @note For mouse events use |nvim_input_mouse()|. The pseudokey form
///       `<LeftMouse><col,row>` is deprecated since |api-level| 6.
///
/// @param keys to be typed
/// @return Number of bytes actually written (can be fewer than
///         requested if the buffer becomes full).
DLLEXPORT Integer nvim_input(uint64_t channel_id, String keys) FUNC_API_SINCE(1) FUNC_API_FAST;

/// Send mouse event from GUI.
///
/// Non-blocking: does not wait on any result, but queues the event to be
/// processed soon by the event loop.
///
/// @note Currently this doesn't support "scripting" multiple mouse events
///       by calling it multiple times in a loop: the intermediate mouse
///       positions will be ignored. It should be used to implement real-time
///       mouse input in a GUI. The deprecated pseudokey form
///       (`<LeftMouse><col,row>`) of |nvim_input()| has the same limitation.
///
/// @param button Mouse button: one of "left", "right", "middle", "wheel", "move",
///               "x1", "x2".
/// @param action For ordinary buttons, one of "press", "drag", "release".
///               For the wheel, one of "up", "down", "left", "right". Ignored for "move".
/// @param modifier String of modifiers each represented by a single char.
///                 The same specifiers are used as for a key press, except
///                 that the "-" separator is optional, so "C-A-", "c-a"
///                 and "CA" can all be used to specify Ctrl+Alt+click.
/// @param grid Grid number (used by |ui-multigrid| client), or 0 to let Nvim decide positioning of
///             windows. For more information, see [dev-ui-multigrid]
/// @param row Mouse row-position (zero-based, like redraw events)
/// @param col Mouse column-position (zero-based, like redraw events)
/// @param[out] err Error details, if any
DLLEXPORT void nvim_input_mouse(String button, String action, String modifier, Integer grid,
                                Integer row, Integer col, Error *err) FUNC_API_SINCE(6)
  FUNC_API_FAST;

/// Converts terminal codes and |keycodes| ([<CR>], [<Esc>], …) in a key sequence, to the internal
/// representation. See also Lua |vim.keycode()|.
///
/// @see replace_termcodes
/// @see cpoptions
///
/// @param str        String to be converted.
/// @param from_part  Legacy Vim parameter. Usually true.
/// @param do_lt      Also translate [<lt>]. Ignored if `special` is false.
/// @param special    Replace |keycodes|, e.g. [<CR>] becomes a "\r" char.
DLLEXPORT String nvim_replace_termcodes(String str, Boolean from_part, Boolean do_lt,
                                        Boolean special) FUNC_API_SINCE(1) FUNC_API_FAST
  FUNC_API_RET_ALLOC;

/// Executes Lua code. Arguments are available as `...` inside the chunk. The chunk can return
/// a value.
///
/// Only statements are executed. To evaluate an expression, prefix it with "return": `return
/// my_function(...)`
///
/// Example:
/// ```lua
/// local peer = vim.fn.jobstart({ vim.v.progpath, '--clean', '--embed' }, { rpc=true })
/// vim.print(vim.rpcrequest(peer, 'nvim_exec_lua', [[
///       local a, b = ...
///       return ('result: %s'):format(a + b)
///     ]],
///     { 1, 3 }
///   )
/// )
/// ```
///
/// @param code       Lua code to execute.
/// @param args       Arguments to the Lua code.
/// @param[out] err   Lua error raised while parsing or executing the Lua code.
///
/// @return           Value returned by the Lua code (if any), or NIL.
DLLEXPORT Object nvim_exec_lua(String code, Array args, Arena *arena,
                               Error *err) FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY;

/// Like |nvim_exec_lua()|, but can be called during |api-fast| contexts.
///
/// Execute Lua code. Parameters (if any) are available as `...` inside the
/// chunk. The chunk can return a value.
///
/// Only statements are executed. To evaluate an expression, prefix it
/// with `return`: return my_function(...)
///
/// @param code       Lua code to execute
/// @param args       Arguments to the code
/// @param[out] err   Details of an error encountered while parsing
///                   or executing the Lua code.
///
/// @return           Return value of Lua code if present or NIL.
DLLEXPORT Object nvim__exec_lua_fast(String code, Array args, Arena *arena,
                                     Error *err) FUNC_API_SINCE(14) FUNC_API_REMOTE_ONLY
  FUNC_API_FAST;

/// Calculates the number of display cells occupied by `text`.
/// Control characters including [<Tab>] count as one cell.
///
/// @param text       Some text
/// @param[out] err   Error details, if any
/// @return Number of cells
DLLEXPORT Integer nvim_strwidth(String text, Error *err) FUNC_API_SINCE(1);

/// Gets the paths contained in |runtime-search-path|.
///
/// @return List of paths
DLLEXPORT ArrayOf(String) nvim_list_runtime_paths(Arena *arena, Error *err) FUNC_API_SINCE(1);

/// @nodoc
DLLEXPORT Array nvim__runtime_inspect(Arena *arena);

/// Finds files in runtime directories, in 'runtimepath' order.
///
/// "name" can contain wildcards. For example
/// `nvim_get_runtime_file("colors/*.{vim,lua}", true)` will return all color
/// scheme files. Always use forward slashes (/) in the search pattern for
/// subdirectories regardless of platform.
///
/// It is not an error to not find any files. An empty array is returned then.
///
/// @param name pattern of files to search for
/// @param all whether to return all matches or only the first
/// @return list of absolute paths to the found files
DLLEXPORT ArrayOf(String) nvim_get_runtime_file(String name, Boolean all, Arena *arena,
                                                Error *err) FUNC_API_SINCE(7) FUNC_API_FAST;

/// @nodoc
DLLEXPORT String nvim__get_lib_dir(void) FUNC_API_RET_ALLOC;

/// Find files in runtime directories
///
/// @param pat pattern of files to search for
/// @param all whether to return all matches or only the first
/// @param opts is_lua: only search Lua subdirs
/// @return list of absolute paths to the found files
DLLEXPORT ArrayOf(String) nvim__get_runtime(ArrayOf(String) pat, Boolean all, Dict(runtime) *opts,
                                            Arena *arena, Error *err) FUNC_API_SINCE(8)
  FUNC_API_FAST;

/// Changes the global working directory.
///
/// @param dir      Directory path
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_current_dir(String dir, Error *err) FUNC_API_SINCE(1);

/// Gets the current line.
///
/// @param[out] err Error details, if any
/// @return Current line string
DLLEXPORT String nvim_get_current_line(Arena *arena, Error *err) FUNC_API_SINCE(1);

/// Sets the text on the current line.
///
/// @param line     Line contents
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_current_line(String line, Arena *arena,
                                     Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;

/// Deletes the current line.
///
/// @param[out] err Error details, if any
DLLEXPORT void nvim_del_current_line(Arena *arena, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;

/// Gets a global (g:) variable.
///
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
DLLEXPORT Object nvim_get_var(String name, Arena *arena, Error *err) FUNC_API_SINCE(1);

/// Sets a global (g:) variable.
///
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_var(String name, Object value, Error *err) FUNC_API_SINCE(1);

/// Removes a global (g:) variable.
///
/// @param name     Variable name
/// @param[out] err Error details, if any
DLLEXPORT void nvim_del_var(String name, Error *err) FUNC_API_SINCE(1);

/// Gets a v: variable.
///
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return         Variable value
DLLEXPORT Object nvim_get_vvar(String name, Arena *arena, Error *err) FUNC_API_SINCE(1);

/// Sets a v: variable, if it is not readonly.
///
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_vvar(String name, Object value, Error *err) FUNC_API_SINCE(6);

/// Prints a message given by a list of `[text, hl_group]` "chunks". Emits a |Progress| event if
/// `kind='progress'`.
///
/// Returns a message-id, which can be given in later calls to update an existing message. The
/// message-id is an autogenerated integer, or a user-defined string. The id "address space" is
/// global, so plugins specifying a string id should use qualified names such as "my.msg.id" to
/// avoid unintentional conflicts.
///
/// Example:
/// ```lua
/// vim.api.nvim_echo({ { 'chunk1-line1\nchunk1-line2\n' }, { 'chunk2-line1' } }, true)
/// ```
///
/// @param chunks List of `[text, hl_group]` pairs, where each is a `text` string highlighted by
///               the (optional) name or ID `hl_group`.
/// @param history  if true, add to |message-history|.
/// @param opts  Optional parameters.
///          - data (`table?`) Dict of arbitrary data, available in |Progress| |event-data|.
///          - err (`boolean?`)  Treat the message like `:echoerr`. Sets `hl_group` to |hl-ErrorMsg| by default.
///          - id (`integer|string?`) Message-id returned by a previous `nvim_echo` call, or
///            a user-defined id (string). If existing message has this id, it will be updated
///            instead of creating a new message.
///          - kind (`string?`) Decides the |ui-messages| kind in the emitted message. Set "progress"
///            to emit a |progress-message|.
///          - percent (`integer?`) |progress-message| percentage, or nil to signal "unknown progress".
///          - source (`string?`) |progress-message| source.
///          - status (`string?`) |progress-message| status:
///            - "success": Process completed successfully.
///            - "running": Process is ongoing.
///            - "failed": Process failed.
///            - "cancel": Process should be cancelled. Progress owner must handle the |Progress|
///              event to perform the cancellation.
///          - title (`string?`) Message title. Only for |progress-message| currently.
///          - verbose (`boolean?`) Message is controlled by the 'verbose' option. `nvim -V3log` will write the
///            message to the "log" file instead of standard output.
/// @return Message-id, or -1 if message wasn't shown.
DLLEXPORT Union(Integer, String) nvim_echo(ArrayOf(Tuple(String, *HLGroupID)) chunks,
                                           Boolean history, Dict(echo_opts) *opts,
                                           Error *err) FUNC_API_SINCE(7);

/// Gets the current list of buffers.
///
/// Includes unlisted (unloaded/deleted) buffers, like `:ls!`.
/// Use |nvim_buf_is_loaded()| to check if a buffer is loaded.
///
/// @return List of buffer ids
DLLEXPORT ArrayOf(Buffer) nvim_list_bufs(Arena *arena) FUNC_API_SINCE(1);

/// Gets the current buffer.
///
/// @return Buffer id
DLLEXPORT Buffer nvim_get_current_buf(void) FUNC_API_SINCE(1);

/// Sets the current window's buffer to `buf`.
///
/// @param buf   Buffer id
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_current_buf(Buffer buf, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;

/// Gets the current list of all |window-ID|s in all tabpages.
///
/// @return List of |window-ID|s
DLLEXPORT ArrayOf(Window) nvim_list_wins(Arena *arena) FUNC_API_SINCE(1);

/// Gets the current window.
///
/// @return |window-ID|
DLLEXPORT Window nvim_get_current_win(void) FUNC_API_SINCE(1);

/// Navigates to the given window (and tabpage, implicitly).
///
/// @param win |window-ID| to focus
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_current_win(Window win, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;

/// Creates a new, empty, unnamed buffer.
///
/// @param listed Sets 'buflisted'
/// @param scratch Creates a "throwaway" |scratch-buffer| for temporary work
///                (always 'nomodified'). Also sets 'nomodeline' on the buffer.
/// @param[out] err Error details, if any
/// @return Buffer id, or 0 on error
DLLEXPORT Buffer nvim_create_buf(Boolean listed, Boolean scratch, Error *err) FUNC_API_SINCE(6);

/// Open a terminal instance in a buffer
///
/// By default (and currently the only option) the terminal will not be
/// connected to an external process. Instead, input sent on the channel
/// will be echoed directly by the terminal. This is useful to display
/// ANSI terminal sequences returned as part of an RPC message, or similar.
///
/// Note: |:ls| reports the buffer as "R" (running) until the channel is closed. |chanclose()|
///
/// Note: To initialize the terminal size, display the buffer in a window first. E.g. for a floating display,
/// 1. Create an empty buffer using |nvim_create_buf()|.
/// 2. Display it with |nvim_open_win()|.
/// 3. Call nvim_open_term().
/// 4. Then calling |nvim_chan_send()| will process sequences in a virtual terminal with the
///    intended size (defined by the window width/height).
///
/// Example: this `TermHl` command can be used to display and highlight raw ANSI termcodes, so you
/// can use Nvim as a "scrollback pager" (for terminals like kitty): [ansi-colorize]()
/// [terminal-scrollback-pager]()
///
/// ```lua
/// vim.api.nvim_create_user_command('TermHl', function()
///   vim.api.nvim_open_term(0)
/// end, { desc = 'Highlights ANSI termcodes in curbuf' })
/// ```
///
/// @param buf Buffer which displays the PTY output. The initial buffer contents (if any) will be
///            written to the PTY.
/// @param opts   Optional parameters.
///          - force_crlf: (boolean, default: true) Convert "\n" to "\r\n".
///          - on_input: (`fun("input", chan: integer, buf: integer, data: string)`) Function invoked
///            when the terminal emits bytes, see above. |textlock| applies. May call
///            |nvim_chan_send()| directly.
/// @param[out] err Error details, if any
/// @return Channel id, or 0 on error
DLLEXPORT Integer nvim_open_term(Buffer buf, Dict(open_term) *opts,
                                 Error *err) FUNC_API_SINCE(7) FUNC_API_TEXTLOCK;

/// Sends raw data to channel `chan`. |channel-bytes|
/// - For a job, it writes it to the stdin of the process.
/// - For the stdio channel |channel-stdio|, it writes to Nvim's stdout.
/// - For an internal terminal instance (|nvim_open_term()|) it writes directly to terminal output.
///
/// This function writes raw data, not RPC messages. Use |vim.rpcrequest()| and |vim.rpcnotify()| if
/// the channel expects RPC messages (i.e. it was created with `rpc=true`).
///
/// To write data to the |TUI| host terminal, see |nvim_ui_send()|.
///
/// @param chan Channel id
/// @param data Data to write. 8-bit clean: may contain NUL bytes.
/// @param[out] err Error details, if any
DLLEXPORT void nvim_chan_send(Integer chan, String data,
                              Error *err) FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY FUNC_API_LUA_ONLY;

/// Gets the current list of |tab-ID|s.
///
/// @return List of |tab-ID|s
DLLEXPORT ArrayOf(Tabpage) nvim_list_tabpages(Arena *arena) FUNC_API_SINCE(1);

/// Gets the current tabpage.
///
/// @return |tab-ID|
DLLEXPORT Tabpage nvim_get_current_tabpage(void) FUNC_API_SINCE(1);

/// Sets the current tabpage.
///
/// @param tabpage  |tab-ID| to focus
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_current_tabpage(Tabpage tabpage,
                                        Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;

/// Pastes at cursor (in any mode), and sets "redo" so dot (|.|) will repeat the input. UIs call
/// this to implement "paste", but it's also intended for use by scripts to input large,
/// dot-repeatable blocks of text (as opposed to |nvim_input()| which is subject to mappings/events
/// and is thus much slower).
///
/// Invokes the |vim.paste()| handler, which handles each mode appropriately.
///
/// Errors ('nomodifiable', `vim.paste()` failure, …) are reflected in `err` but do not affect the
/// return value (which is strictly decided by `vim.paste()`).  On error or cancel, subsequent calls
/// are ignored ("drained") until the next paste is initiated (phase 1 or -1).
///
/// Useful in mappings and scripts to insert multiline text. Example:
///
/// ```lua
/// vim.keymap.set('n', 'x', function()
///   vim.api.nvim_paste([[
///     line1
///     line2
///     line3
///   ]], false, -1)
/// end, { buf = 0 })
/// ```
///
/// @param data  Multiline input. Lines break at LF ("\n"). May be binary (containing NUL bytes).
/// @param crlf  Also break lines at CR and CRLF.
/// @param phase  -1: paste in a single call (i.e. without streaming).
///               To "stream" a paste, call `nvim_paste` sequentially with
///               these `phase` values:
///                 - 1: starts the paste (exactly once)
///                 - 2: continues the paste (zero or more times)
///                 - 3: ends the paste (exactly once)
/// @param[out] err Error details, if any
/// @return
///     - true: Client may continue pasting.
///     - false: Client should cancel the paste.
DLLEXPORT Boolean nvim_paste(uint64_t channel_id, String data, Boolean crlf, Integer phase,
                             Arena *arena, Error *err) FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;

/// Puts text at cursor, in any mode. For dot-repeatable input, use |nvim_paste()|.
///
/// Compare |:put| and |p| which are always linewise.
///
/// @param lines  |readfile()|-style list of lines. |channel-lines|
/// @param type  Edit behavior: any |getregtype()| result, or:
///              - "b" |blockwise-visual| mode (may include width, e.g. "b3")
///              - "c" |charwise| mode
///              - "l" |linewise| mode
///              - ""  guess by contents, see |setreg()|
/// @param after  If true insert after cursor (like |p|), or before (like |P|).
/// @param follow  If true place cursor at end of inserted text.
/// @param[out] err Error details, if any
DLLEXPORT void nvim_put(ArrayOf(String) lines, String type, Boolean after, Boolean follow,
                        Arena *arena, Error *err) FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;

/// Returns the 24-bit RGB value of a |nvim_get_color_map()| color name or
/// "#rrggbb" hexadecimal string.
///
/// Example:
///
/// ```vim
/// :echo nvim_get_color_by_name("Pink")
/// :echo nvim_get_color_by_name("#cbcbcb")
/// ```
///
/// @param name Color name or "#rrggbb" string
/// @return 24-bit RGB value, or -1 for invalid argument.
DLLEXPORT Integer nvim_get_color_by_name(String name) FUNC_API_SINCE(1);

/// Returns a map of color names and RGB values.
///
/// Keys are color names (e.g. "Aqua") and values are 24-bit RGB color values
/// (e.g. 65535).
///
/// @return Map of color names and RGB values.
DLLEXPORT DictOf(Integer) nvim_get_color_map(Arena *arena) FUNC_API_SINCE(1);

/// Gets a map (msgpack encoding) of the current editor state:
///
/// - "bufs"    [buffer-list]
/// - "funcs"   Vimscript global and [script-local] functions
/// - "gvars"   [global-variable]s
/// - "jumps"   [jumplist]
/// - "regs"    [registers]
/// - "sfuncs"  Vimscript [script-local] functions
///
/// @param opts  Optional parameters.
///               - types:  List of context types (see above) to gather, or empty for "all".
/// @param[out]  err  Error details, if any
///
/// @return Editor state as a msgpack-encoded map.
DLLEXPORT Dict nvim_get_context(Dict(context) *opts, Arena *arena, Error *err) FUNC_API_SINCE(6);

/// Sets the current editor state from the given map.
///
/// @param dict  Msgpack-encoded editor state, in the form returned by [nvim_get_context()].
DLLEXPORT Object nvim_load_context(Dict dict, Error *err) FUNC_API_SINCE(6);

/// Gets the current mode. |mode()|
/// "blocking" is true if Nvim is waiting for input.
///
/// @returns Dict { "mode": String, "blocking": Boolean }
DLLEXPORT DictAs(get_mode) nvim_get_mode(Arena *arena) FUNC_API_SINCE(2) FUNC_API_FAST;

/// Gets a list of global (non-buffer-local) |mapping| definitions.
///
/// @param  mode       Mode short-name ("n", "i", "v", ...)
/// @returns Array of |maparg()|-like dictionaries describing mappings.
///          The "buf" key is always zero.
DLLEXPORT ArrayOf(DictAs(get_keymap)) nvim_get_keymap(String mode, Arena *arena) FUNC_API_SINCE(3);

/// Sets a global |mapping| for the given mode.
///
/// To set a buffer-local mapping, use |nvim_buf_set_keymap()|.
///
/// Unlike |:map|, leading/trailing whitespace is accepted as part of the {lhs} or {rhs}.
/// Empty {rhs} is [<Nop>]. |keycodes| are replaced as usual.
///
/// Example:
///
/// ```vim
/// call nvim_set_keymap('n', ' <NL>', '', {'nowait': v:true})
/// ```
///
/// is equivalent to:
///
/// ```vim
/// nmap <nowait> <Space><NL> <Nop>
/// ```
///
/// @param channel_id Channel id (implicit dispatcher arg)
/// @param  mode  Mode short-name (map command prefix: "n", "i", "v", "x", …)
///               or "!" for |:map!|, or empty string for |:map|.
///               "ia", "ca" or "!a" for abbreviation in Insert mode, Cmdline mode, or both, respectively
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  rhs   Right-hand-side |{rhs}| of the mapping.
/// @param  opts  Optional parameters map: Accepts all |:map-arguments| as keys except [<buffer>],
///               values are booleans (default false). Also:
///               - "callback" Lua function called in place of {rhs}.
///               - "desc" human-readable description.
///               - "noremap" disables |recursive_mapping|, like |:noremap|
///               - "replace_keycodes" (boolean) When "expr" is true, replace keycodes in the
///                 resulting string (see |nvim_replace_termcodes()|). Returning nil from the Lua
///                 "callback" is equivalent to returning an empty string.
/// @param[out]   err   Error details, if any.
DLLEXPORT void nvim_set_keymap(uint64_t channel_id, String mode, String lhs, String rhs,
                               Dict(keymap) *opts, Error *err) FUNC_API_SINCE(6);

/// Unmaps a global |mapping| for the given mode.
///
/// To unmap a buffer-local mapping, use |nvim_buf_del_keymap()|.
///
/// @param  mode  Mode short-name ("n", "i", "v", ...)
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  opts  Optional parameters.
///               - lhs: When true, only match {lhs}, not {rhs}.
///
/// @see |nvim_set_keymap()|
DLLEXPORT void nvim_del_keymap(uint64_t channel_id, String mode, String lhs, Dict(keymap_del) *opts,
                               Error *err) FUNC_API_SINCE(6);

/// Returns a 2-tuple (Array), where item 0 is the current channel id and item
/// 1 is the |api-metadata| map (Dict).
///
/// @returns 2-tuple `[{channel-id}, {api-metadata}]`
DLLEXPORT ArrayOf(Object, 2) nvim_get_api_info(uint64_t channel_id,
                                               Arena *arena) FUNC_API_SINCE(1) FUNC_API_FAST
  FUNC_API_REMOTE_ONLY;

/// Self-identifies the client, and sets optional flags on the channel. Defines the `client` object
/// returned by |nvim_get_chan_info()|.
///
/// Clients should call this just after connecting, to provide hints for debugging and
/// orchestration. (Note: Something is better than nothing! Fields are optional, but at least set
/// `name`.)
///
/// Can be called more than once; caller should merge old info if appropriate. Example: a library
/// first identifies the channel, then a plugin using that library later identifies itself.
///
/// @param channel_id Channel id (implicit dispatcher arg)
/// @param name Client short-name. Sets the `client.name` field of |nvim_get_chan_info()|.
/// @param version  Dict describing the version, with these
///     (optional) keys:
///     - "commit" hash or similar identifier of commit
///     - "major" major version (defaults to 0 if not set, for no release yet)
///     - "minor" minor version
///     - "patch" patch number
///     - "prerelease" string describing a prerelease, like "dev" or "beta1"
/// @param type Must be one of the following values. Client libraries should
///     default to "remote" unless overridden by the user.
///     - "embedder" application using Nvim as a component (for example,
///                  IDE/editor implementing a vim mode).
///     - "host" plugin host, typically started by nvim
///     - "msgpack-rpc" remote client connected to Nvim via fully MessagePack-RPC
///                     compliant protocol.
///     - "plugin" single plugin, started by nvim
///     - "remote" remote client connected "Nvim flavored" MessagePack-RPC (responses
///                must be in reverse order of requests). |msgpack-rpc|
///     - "ui" gui frontend
/// @param methods Builtin methods in the client. For a host, this does not
///     include plugin methods which will be discovered later.
///     The key should be the method name, the values are dicts with
///     these (optional) keys (more keys may be added in future
///     versions of Nvim, thus unknown keys are ignored. Clients
///     must only use keys defined in this or later versions of
///     Nvim):
///     - "async"  if true, send as a notification. If false or unspecified,
///                use a blocking request
///     - "nargs" Number of arguments. Could be a single integer or an array
///                of two integers, minimum and maximum inclusive.
///
/// @param attributes Arbitrary string:string map of informal client properties.
///     Suggested keys:
///     - "license": License description ("Apache 2", "GPLv3", "MIT", …)
///     - "logo":    URI or path to image, preferably small logo or icon.
///                  .png or .svg format is preferred.
///     - "pid":     Process id.
///     - "website": Client homepage URL (e.g. GitHub repository)
///
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_client_info(uint64_t channel_id, String name, Dict version, String type,
                                    Dict methods, Dict attributes, Arena *arena,
                                    Error *err) FUNC_API_SINCE(4) FUNC_API_REMOTE_ONLY;

/// Sets channel properties.
///
/// @param chan Channel id, or 0 for current (RPC) channel.
/// @param config Properties:
///     - detach: (boolean, default: true) Closing the channel does not self-exit Nvim. Nvim
///       self-exits when the last `detach=false` |RPC| channel closes. See also |:detach!|.
/// @param[out] err Error details, if any
DLLEXPORT void nvim_chan_set(uint64_t channel_id, Integer chan, Dict(chan_set) *config,
                             Error *err) FUNC_API_SINCE(15) FUNC_API_FAST;

/// Records the cmdwin scratchbuf and type, or clears both when type="" / buf=0. Internal use only.
///
/// @param type  ':', '/', '?' (first char only); empty to clear.
/// @param buf   cmdwin buffer id, or 0 to clear.
/// @param[out] err Error details, if any.
DLLEXPORT void nvim__cmdwin_set(String type, Buffer buf, Error *err) FUNC_API_SINCE(14);

/// Gets information about a channel.
///
/// The docs for |nvim_list_uis()| show an example.
///
/// @param chan Channel id, or 0 for current (RPC) channel
/// @returns Channel info dict with these keys:
///    - "argv"     (optional) Job arguments list.
///    - "buf"      (optional) Buffer connected to |terminal| instance.
///    - "buffer"   (optional) Deprecated alias for `buf`.
///    - "client"   (optional) Info about the peer (client on the other end of the channel), as set
///                 by |nvim_set_client_info()|.
///    - "detach"   (optional) Closing the |RPC| channel does not exit Nvim. |nvim_chan_set()|
///    - "exitcode" (optional) Exit code of the |terminal| process.
///    - "id"       Channel id.
///    - "internal" (optional) In-process channel (stream=socket).
///    - "mode"     How data received on the channel is interpreted.
///         - "bytes"      Send and receive raw bytes.
///         - "terminal"   |terminal| instance interprets ASCII sequences.
///         - "rpc"        |RPC| communication on the channel is active.
///    - "pty"      (optional) Name of pseudoterminal. On a POSIX system this is a device path like
///                 "/dev/pts/1". If unknown, the key will still be present if a pty is used (e.g.
///                 for conpty on Windows).
///    - "stream"   Stream underlying the channel.
///         - "stdio"      stdin and stdout of this Nvim instance
///         - "stderr"     stderr of this Nvim instance
///         - "socket"     TCP/IP socket or named pipe
///         - "job"        Job with communication over its stdio.
DLLEXPORT Dict nvim_get_chan_info(uint64_t channel_id, Integer chan, Arena *arena,
                                  Error *err) FUNC_API_SINCE(4);

/// Get information about all open channels.
///
/// @returns Array of Dictionaries, each describing a channel with
///          the format specified at |nvim_get_chan_info()|.
DLLEXPORT ArrayOf(Dict) nvim_list_chans(Arena *arena) FUNC_API_SINCE(4);

/// Returns object given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  obj  Object to return.
///
/// @return its argument.
DLLEXPORT Object nvim__id(Object obj, Arena *arena);

/// Returns array given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  arr  Array to return.
///
/// @return its argument.
DLLEXPORT Array nvim__id_array(Array arr, Arena *arena);

/// Returns dict given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  dct  Dict to return.
///
/// @return its argument.
DLLEXPORT Dict nvim__id_dict(Dict dct, Arena *arena);

/// Returns floating-point value given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  flt  Value to return.
///
/// @return its argument.
DLLEXPORT Float nvim__id_float(Float flt);

/// Gets internal stats.
///
/// @return Map of various internal stats.
DLLEXPORT Dict nvim__stats(Arena *arena);

/// Gets a list of dictionaries representing attached UIs.
///
/// Example: The Nvim builtin |TUI| sets its channel info as described in |startup-tui|. In
/// particular, it sets `client.name` to "nvim-tui". So you can check if the TUI is running by
/// inspecting the client name of each UI:
///
/// ```lua
/// vim.print(vim.api.nvim_get_chan_info(vim.api.nvim_list_uis()[1].chan).client.name)
/// ```
///
/// @return Array of UI dictionaries, each with these keys:
///   - "height"  Requested height of the UI
///   - "width"   Requested width of the UI
///   - "rgb"     true if the UI uses RGB colors (false implies |cterm-colors|)
///   - "ext_..." Requested UI extensions, see |ui-option|
///   - "chan"    |channel-id| of remote UI
DLLEXPORT ArrayOf(Dict) nvim_list_uis(Arena *arena) FUNC_API_SINCE(4);

/// Gets the immediate children of process `pid`.
///
/// @return Array of child process ids, empty if process not found.
DLLEXPORT Array nvim_get_proc_children(Integer pid, Arena *arena, Error *err) FUNC_API_SINCE(4);

/// Gets info describing process `pid`.
///
/// @return Map of process properties, or NIL if process not found.
DLLEXPORT Object nvim_get_proc(Integer pid, Arena *arena, Error *err) FUNC_API_SINCE(4);

/// Selects an item in the completion popup menu.
///
/// If neither |ins-completion| nor |cmdline-completion| popup menu is active
/// this API call is silently ignored.
/// Useful for an external UI using |ui-popupmenu| to control the popup menu with the mouse.
/// Can also be used in a mapping; use [<Cmd>] |:map-cmd| or a Lua mapping to ensure the mapping
/// doesn't end completion mode.
///
/// @param item    Index (zero-based) of the item to select. Value of -1 selects nothing
///                and restores the original text.
/// @param insert  For |ins-completion|, whether the selection should be inserted in the buffer.
///                Ignored for |cmdline-completion|.
/// @param finish  Finish the completion and dismiss the popup menu. Implies {insert}.
/// @param opts    Optional parameters. Reserved for future use.
/// @param[out] err Error details, if any
DLLEXPORT void nvim_select_popupmenu_item(Integer item, Boolean insert, Boolean finish,
                                          Dict(empty) *opts, Error *err) FUNC_API_SINCE(6);

/// NB: if your UI doesn't use hlstate, this will not return hlstate first time.
DLLEXPORT Array nvim__inspect_cell(Integer grid, Integer row, Integer col, Arena *arena,
                                   Error *err);

/// @nodoc
DLLEXPORT void nvim__screenshot(String path) FUNC_API_FAST;

/// For testing. The condition in schar_cache_clear_if_full is hard to
/// reach, so this function can be used to force a cache clear in a test.
DLLEXPORT void nvim__invalidate_glyph_cache(void);

/// @nodoc
DLLEXPORT Object nvim__unpack(String str, Arena *arena, Error *err) FUNC_API_FAST;

/// Deletes an uppercase/file named mark. See |mark-motions|.
///
/// @note Lowercase name (or other buffer-local mark) is an error.
/// @param name       Mark name
/// @return true if the mark was deleted, else false.
/// @see |nvim_buf_del_mark()|
/// @see |nvim_get_mark()|
DLLEXPORT Boolean nvim_del_mark(String name, Error *err) FUNC_API_SINCE(8);

/// Returns a `(row, col, buffer, buffername)` tuple representing the position
/// of the uppercase/file named mark. "End of line" column position is returned
/// as |v:maxcol| (big number). See |mark-motions|.
///
/// Marks are (1,0)-indexed. |api-indexing|
///
/// @note Lowercase name (or other buffer-local mark) is an error.
/// @param name       Mark name
/// @param opts       Optional parameters. Reserved for future use.
/// @return 4-tuple (row, col, buffer, buffername), (0, 0, 0, '') if the mark is
/// not set.
/// @see |nvim_buf_set_mark()|
/// @see |nvim_del_mark()|
DLLEXPORT Tuple(Integer, Integer, Buffer, String) nvim_get_mark(String name, Dict(empty) *opts,
                                                                Arena *arena,
                                                                Error *err) FUNC_API_SINCE(8);

/// Evaluates statusline string.
///
/// @param str Statusline string (see 'statusline').
/// @param opts Optional parameters.
///           - fillchar: (string) Character to fill blank spaces in the statusline (see
///                                'fillchars'). Treated as single-width even if it isn't.
///           - highlights: (boolean) Return highlight information.
///           - maxwidth: (number) Maximum width of statusline.
///           - use_statuscol_lnum: (number) Evaluate statuscolumn for this line number instead of statusline.
///           - use_tabline: (boolean) Evaluate tabline instead of statusline. When true, {winid}
///                                    is ignored. Mutually exclusive with {use_winbar}.
///           - use_winbar: (boolean) Evaluate winbar instead of statusline.
///           - winid: (number) |window-ID| of the window to use as context for statusline.
///
/// @param[out] err Error details, if any.
/// @return Dict containing statusline information, with these keys:
///       - str: (string) Characters that will be displayed on the statusline.
///       - width: (number) Display width of the statusline.
///       - highlights: Array containing highlight information of the statusline. Only included when
///                     the "highlights" key in {opts} is true. Each element of the array is a
///                     |Dict| with these keys:
///           - start: (number) Byte index (0-based) of first character that uses the highlight.
///           - group: (string) Deprecated. Use `groups` instead.
///           - groups: (array) Names of stacked highlight groups (highest priority last).
DLLEXPORT DictAs(eval_statusline_ret) nvim_eval_statusline(String str, Dict(eval_statusline) *opts,
                                                           Arena *arena,
                                                           Error *err) FUNC_API_SINCE(8)
  FUNC_API_FAST;

/// Sets info for the completion item at the given index. If the info text was shown in a window,
/// returns the window and buffer ids, or empty dict if not shown.
///
/// @param index  Completion candidate index
/// @param opts   Optional parameters.
///       - info: (string) info text.
/// @return Dict containing these keys:
///       - winid: (integer) floating window id
///       - bufnr: (integer) buffer id in floating window
DLLEXPORT DictOf(Integer) nvim__complete_set(Integer index, Dict(complete_set) *opts, Arena *arena,
                                             Error *err);

/// Instruct Nvim to redraw various components.
///
/// @see |:redraw|
///
/// @param opts  Optional parameters.
///               - buf: Target a specific buffer number as described below.
///               - cursor: Immediately update cursor position on the screen in
///                 `win` or the current window.
///               - flush: Update the screen with pending updates.
///               - range: Redraw a range in `buf`, the buffer in `win` or the
///                 current buffer (useful for decoration providers). Expects a
///                 tuple `[first, last]` with the first and last line number
///                 of the range, 0-based end-exclusive |api-indexing|.
///               - statuscolumn: Redraw the 'statuscolumn' in `buf`, `win` or
///                 all windows.
///               - statusline: Redraw the 'statusline' in `buf`, `win` or all
///                 windows.
///               - tabline: Redraw the 'tabline'.
///               - valid: When present mark `win`, `buf`, or all windows for
///                 redraw. When `true`, only redraw changed lines (useful for
///                 decoration providers). When `false`, forcefully redraw.
///               - win: Target a specific |window-ID| as described below.
///               - winbar: Redraw the 'winbar' in `buf`, `win` or all windows.
DLLEXPORT void nvim__redraw(Dict(redraw) *opts, Error *err) FUNC_API_SINCE(12);
DLLEXPORT void nvim__set_restart_on_crash(String progpath, Array argv);

#include "nvim/func_attr.h"
