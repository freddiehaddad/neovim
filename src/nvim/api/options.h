#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep

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

/// Gets the value of an option. The behavior of this function matches that of
/// |:set|: the local value of an option is returned if it exists; otherwise,
/// the global value is returned. Local values always correspond to the current
/// buffer or window, unless "buf" or "win" is set in {opts}.
///
/// @param name      Option name
/// @param opts      Optional parameters
///                  - buf: Buffer number. Used for getting buffer local options.
///                         Implies {scope} is "local".
///                  - filetype: |filetype|. Used to get the default option for a
///                    specific filetype. Cannot be used with any other option.
///                    Note: this will trigger |ftplugin| and all |FileType|
///                    autocommands for the corresponding filetype.
///                  - scope: One of "global" or "local". Analogous to
///                  |:setglobal| and |:setlocal|, respectively.
///                  - tab: |tab-ID| for tab-local options. Currently only
///                    supports "cmdheight". Tabpage `0` means the current tabpage.
///                  - win: |window-ID|. Used for getting window local options.
/// @param[out] err  Error details, if any
/// @return          Option value
DLLEXPORT Object nvim_get_option_value(String name, Dict(option) *opts, Error *err)
  FUNC_API_SINCE(9) FUNC_API_RET_ALLOC;

/// Sets the value of an option. The behavior of this function matches that of
/// |:set|: for global-local options, both the global and local value are set
/// unless otherwise specified with {scope}.
///
/// Note that `win` and `buf` cannot be used together.
///
/// @param name      Option name
/// @param value     New option value. May be structured for dict/list options, e.g. 'formatoptions'
///                  accepts `{ 'c', 'r' }`, 'listchars' accepts `{ eol = '~' }`.
/// @param opts      Optional parameters
///                  - buf: Buffer number. Used for setting buffer local option.
///                  - dry_run: (`boolean?`, default: false) If true, validates the
///                    option value without setting it.
///                  - operation: (`string?`, default: "set") One of "set", "append", "prepend", or
///                    "remove". Corresponds to |:set=|, |:set+=|, |:set^=|, and |:set-=|.
///                    With "remove", each item in a structured `value` is removed regardless of
///                    order/adjacency.
///                  - scope: One of "global" or "local". Analogous to
///                  |:setglobal| and |:setlocal|, respectively.
///                  - tab: |tab-ID| for tab-local options (currently only 'cmdheight'). Tabpage 0
///                    means the current tabpage. If a non-current tab is given, the value will take
///                    effect when it is switched-to.
///                  - win: |window-ID|. Used for setting window local option.
/// @param[out] err  Error details, if any
/// @return          Option value
DLLEXPORT Object nvim_set_option_value(uint64_t channel_id, String name, Object value,
                                       Dict(option) *opts, Arena *arena, Error *err)
  FUNC_API_SINCE(9);

/// Gets the option information for all options.
///
/// The dict has the full option names as keys and option metadata dicts as detailed at
/// |nvim_get_option_info2()|.
///
/// @see |nvim_get_commands()|
///
/// @return dict of all options
DLLEXPORT Dict nvim_get_all_options_info(Arena *arena, Error *err)
  FUNC_API_SINCE(7);

/// Gets the option information for one option from arbitrary buffer or window
///
/// Resulting dict has keys:
/// - name: Name of the option (like 'filetype')
/// - shortname: Shortened name of the option (like 'ft')
/// - type: type of option ("string", "number" or "boolean")
/// - default: The default value for the option
/// - was_set: Whether the option was set.
///
/// - last_set_sid: Last set script id (if any)
/// - last_set_linenr: line number where option was set
/// - last_set_chan: Channel where option was set (0 for local)
///
/// - scope: one of "global", "win", "buf", or "tab"
/// - global_local: whether win or buf option has a global value
///
/// - commalist: List of comma separated values
/// - flaglist: List of single char flags
///
/// When {scope} is not provided, the last set information applies to the local
/// value in the current buffer or window if it is available, otherwise the
/// global value information is returned. This behavior can be disabled by
/// explicitly specifying {scope} in the {opts} table.
///
/// @param name      Option name
/// @param opts      Optional parameters
///                  - buf: Buffer number. Used for getting buffer local options.
///                         Implies {scope} is "local".
///                  - scope: One of "global" or "local". Analogous to
///                  |:setglobal| and |:setlocal|, respectively.
///                  - win: |window-ID|. Used for getting window local options.
/// @param[out] err Error details, if any
/// @return         Option Information
DLLEXPORT DictAs(get_option_info) nvim_get_option_info2(String name, Dict(option) *opts,
                                                        Arena *arena, Error *err)
  FUNC_API_SINCE(11);

#include "nvim/func_attr.h"
