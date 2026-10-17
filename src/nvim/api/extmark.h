#pragma once

#include <stdbool.h>
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/buffer_defs.h"
#include "nvim/decoration_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/map_defs.h"
#include "nvim/types_defs.h"

EXTERN Map(String, int) namespace_ids INIT( = MAP_INIT);
/// Non-global namespaces. A locally-scoped namespace may be "orphaned" if all
/// window(s) it was scoped to, are destroyed. Such orphans are tracked here to
/// avoid being mistaken as "global scope".
EXTERN Set(uint32_t) namespace_localscope INIT( = SET_INIT);
EXTERN handle_T next_namespace_id INIT( = 1);

/// Returns true if the namespace is global or scoped in the given window.
static inline bool ns_in_win(uint32_t ns_id, win_T *wp)
{
  if (!set_has(uint32_t, &namespace_localscope, ns_id)) {
    return true;
  }

  return set_has(uint32_t, &wp->w_ns_set, ns_id);
}

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

DLLEXPORT void api_extmark_free_all_mem(void);

/// Creates a new namespace or gets an existing one. [namespace]()
///
/// @param name Namespace name or empty string
/// @return Namespace id
DLLEXPORT Integer nvim_create_namespace(String name)
  FUNC_API_SINCE(5);

/// Gets existing, non-anonymous |namespace|s.
///
/// @return dict that maps from names to namespace ids.
DLLEXPORT DictOf(Integer) nvim_get_namespaces(Arena *arena)
  FUNC_API_SINCE(5);

DLLEXPORT const char *describe_ns(NS ns_id, const char *unknown);
DLLEXPORT bool ns_initialized(uint32_t ns);
DLLEXPORT Array virt_text_to_array(VirtText vt, bool hl_name, Arena *arena);

/// Gets the position (0-indexed) of an |extmark|.
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id
/// @param id  Extmark id
/// @param opts  Optional parameters.
/// @param[out] err   Error details, if any
/// @return 0-indexed (row, col, details?) tuple or empty list () if extmark id was absent.
DLLEXPORT Tuple(Integer, Integer, *DictAs(extmark_details))
nvim_buf_get_extmark_by_id(Buffer buf, Integer ns_id, Integer id, Dict(get_extmark) *opts,
                           Arena *arena, Error *err)
  FUNC_API_SINCE(7);

/// Gets |extmarks| in "traversal order" from a |charwise| region defined by buffer positions
/// (inclusive, 0-indexed |api-indexing|).
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id, or -1 for all namespaces
/// @param start  Start of range
/// @param end  End of range (inclusive)
/// @param opts  Optional parameters.
/// @param[out] err   Error details, if any
/// @return List of `[extmark_id, row, col, details?]` tuples in "traversal order".
DLLEXPORT ArrayOf(DictAs(get_extmark_item)) nvim_buf_get_extmarks(Buffer buf, Integer ns_id,
                                                                  Object start, Object end,
                                                                  Dict(get_extmarks) *opts,
                                                                  Arena *arena, Error *err)
  FUNC_API_SINCE(7);

/// Creates or updates an |extmark|.
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id
/// @param line  Line where to place the mark, 0-based. |api-indexing|
/// @param col  Column where to place the mark, 0-based. |api-indexing|
/// @param opts  Optional parameters
/// @param[out]  err   Error details, if any
/// @return Id of the created/updated extmark
DLLEXPORT Integer nvim_buf_set_extmark(Buffer buf, Integer ns_id, Integer line, Integer col,
                                       Dict(set_extmark) *opts, Error *err)
  FUNC_API_SINCE(7);

/// Removes an |extmark|.
///
/// @param buf Buffer id, or 0 for current buffer
/// @param ns_id |namespace| id
/// @param id Extmark id
/// @param[out] err   Error details, if any
/// @return true if the extmark was found, else false
DLLEXPORT Boolean nvim_buf_del_extmark(Buffer buf, Integer ns_id, Integer id, Error *err)
  FUNC_API_SINCE(7);

/// Clears |namespace|d objects (highlights, |extmarks|, virtual text) from
/// a region.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param ns_id      Namespace to clear, or -1 to clear all namespaces.
/// @param line_start Start of range of lines to clear
/// @param line_end   End of range of lines to clear (exclusive) or -1 to clear
///                   to end of buffer.
/// @param[out] err   Error details, if any
DLLEXPORT void nvim_buf_clear_namespace(Buffer buf, Integer ns_id, Integer line_start,
                                        Integer line_end, Error *err)
  FUNC_API_SINCE(5);

/// Set or change decoration provider for a |namespace|
///
/// @param ns_id  |namespace| id
/// @param opts  Table of callbacks
DLLEXPORT void nvim_set_decoration_provider(Integer ns_id, Dict(set_decoration_provider) *opts,
                                            Error *err)
  FUNC_API_SINCE(7) FUNC_API_LUA_ONLY;

DLLEXPORT VirtText parse_virt_text(Array chunks, Error *err, int *width, bool untab);

/// @nodoc
DLLEXPORT String nvim__buf_debug_extmarks(Buffer buf, Boolean keys, Boolean dot, Error *err)
  FUNC_API_SINCE(7) FUNC_API_RET_ALLOC;

/// Set some properties for namespace
///
/// @param ns_id Namespace
/// @param opts Optional parameters to set
DLLEXPORT void nvim__ns_set(Integer ns_id, Dict(ns_opts) *opts, Error *err);

/// Get the properties for namespace
///
/// @param ns_id Namespace
/// @return  Map defining the namespace properties, see |nvim__ns_set()|
DLLEXPORT Dict(ns_opts) nvim__ns_get(Integer ns_id, Arena *arena, Error *err);

#include "nvim/func_attr.h"
