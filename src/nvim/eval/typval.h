#pragma once

#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/gettext_defs.h"
#include "nvim/hashtab.h"
#include "nvim/lib/queue_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/mbyte_defs.h"  // IWYU pragma: keep
#include "nvim/message.h"
#include "nvim/types_defs.h"

#include "eval/typval.h.inline.generated.h"

// In a hashtab item "hi_key" points to "di_key" in a dictitem.
// This avoids adding a pointer to the hashtab item.

/// Convert a hashitem pointer to a dictitem pointer
#define TV_DICT_HI2DI(hi) \
  ((dictitem_T *)((hi)->hi_key - offsetof(dictitem_T, di_key)))

/// Increase reference count for a given list
///
/// Does nothing for NULL lists.
///
/// @param[in,out]  l  List to modify.
static inline void tv_list_ref(list_T *const l)
  FUNC_ATTR_ALWAYS_INLINE
{
  if (l == NULL) {
    return;
  }
  l->lv_refcount++;
}

/// Set a list as the return value.  Increments the reference count.
///
/// @param[out]  tv  Object to receive the list
/// @param[in,out]  l  List to pass to the object
static inline void tv_list_set_ret(typval_T *const tv, list_T *const l)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ARG(1)
{
  tv->v_type = VAR_LIST;
  tv->vval.v_list = l;
  tv_list_ref(l);
}

/// Get list lock status
///
/// Returns VAR_FIXED for NULL lists.
///
/// @param[in]  l  List to check.
static inline VarLockStatus tv_list_locked(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (l == NULL) {
    return VAR_FIXED;
  }
  return l->lv_lock;
}

/// Set list lock status
///
/// May only “set” VAR_FIXED for NULL lists.
///
/// @param[out]  l  List to modify.
/// @param[in]  lock  New lock status.
static inline void tv_list_set_lock(list_T *const l, const VarLockStatus lock)
{
  if (l == NULL) {
    assert(lock == VAR_FIXED);
    return;
  }
  l->lv_lock = lock;
}

/// Set list copyID
///
/// Does not expect NULL list, be careful.
///
/// @param[out]  l  List to modify.
/// @param[in]  copyid  New copyID.
static inline void tv_list_set_copyid(list_T *const l, const int copyid)
  FUNC_ATTR_NONNULL_ALL
{
  l->lv_copyID = copyid;
}

/// Get the number of items in a list
///
/// @param[in]  l  List to check.
static inline int tv_list_len(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (l == NULL) {
    return 0;
  }
  return l->lv_len;
}

/// Get list copyID
///
/// Does not expect NULL list, be careful.
///
/// @param[in]  l  List to check.
static inline int tv_list_copyid(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL
{
  return l->lv_copyID;
}

/// Get latest list copy
///
/// Gets lv_copylist field assigned by tv_list_copy() earlier.
///
/// Does not expect NULL list, be careful.
///
/// @param[in]  l  List to check.
static inline list_T *tv_list_latest_copy(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL
{
  return l->lv_copylist;
}

/// Normalize index: that is, return either -1 or non-negative index
///
/// @param[in]  l  List to index. Used to get length.
/// @param[in]  n  List index, possibly negative.
///
/// @return -1 or list index in range [0, tv_list_len(l)).
static inline int tv_list_uidx(const list_T *const l, int n)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  // Negative index is relative to the end.
  if (n < 0) {
    n += tv_list_len(l);
  }

  // Check for index out of range.
  if (n < 0 || n >= tv_list_len(l)) {
    return -1;
  }
  return n;
}

/// Check whether list has watchers
///
/// E.g. is referenced by a :for loop.
///
/// @param[in]  l  List to check.
///
/// @return true if there are watchers, false otherwise.
static inline bool tv_list_has_watchers(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  return l && l->lv_watch;
}

/// Get first list item
///
/// @param[in]  l  List to get item from.
///
/// @return List item or NULL in case of an empty list.
static inline listitem_T *tv_list_first(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (l == NULL) {
    return NULL;
  }
  return l->lv_first;
}

/// Get last list item
///
/// @param[in]  l  List to get item from.
///
/// @return List item or NULL in case of an empty list.
static inline listitem_T *tv_list_last(const list_T *const l)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (l == NULL) {
    return NULL;
  }
  return l->lv_last;
}

/// Set a dictionary as the return value
///
/// @param[out]  tv  Object to receive the dictionary
/// @param[in,out]  d  Dictionary to pass to the object
static inline void tv_dict_set_ret(typval_T *const tv, dict_T *const d)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ARG(1)
{
  tv->v_type = VAR_DICT;
  tv->vval.v_dict = d;
  if (d != NULL) {
    d->dv_refcount++;
  }
}

/// Get the number of items in a Dictionary
///
/// @param[in]  d  Dictionary to check.
static inline long tv_dict_len(const dict_T *const d)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (d == NULL) {
    return 0;
  }
  return (long)d->dv_hashtab.ht_used;
}

/// Check if dictionary is watched
///
/// @param[in]  d  Dictionary to check.
///
/// @return true if there is at least one watcher.
static inline bool tv_dict_is_watched(const dict_T *const d)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  return d && !QUEUE_EMPTY(&d->watchers);
}

/// Set a blob as the return value.
///
/// Increments the reference count.
///
/// @param[out]  tv  Object to receive the blob.
/// @param[in,out]  b  Blob to pass to the object.
static inline void tv_blob_set_ret(typval_T *const tv, blob_T *const b)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ARG(1)
{
  tv->v_type = VAR_BLOB;
  tv->vval.v_blob = b;
  if (b != NULL) {
    b->bv_refcount++;
  }
}

/// Get the length of the data in the blob, in bytes.
///
/// @param[in]  b  Blob to check.
static inline int tv_blob_len(const blob_T *const b)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (b == NULL) {
    return 0;
  }
  return b->bv_ga.ga_len;
}

/// Get the byte at index `idx` in the blob.
///
/// @param[in]  b  Blob to index. Cannot be NULL.
/// @param[in]  idx  Index in a blob. Must be valid.
///
/// @return Byte value at the given index.
static inline uint8_t tv_blob_get(const blob_T *const b, int idx)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT
{
  return ((uint8_t *)b->bv_ga.ga_data)[idx];
}

/// Store the byte `c` at index `idx` in the blob.
///
/// @param[in]  blob  Blob to index. Cannot be NULL.
/// @param[in]  idx  Index in a blob. Must be valid.
/// @param[in]  c  Value to store.
static inline void tv_blob_set(blob_T *const blob, int idx, uint8_t c)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ALL
{
  ((uint8_t *)blob->bv_ga.ga_data)[idx] = c;
}

/// Initialize Vimscript object
///
/// Initializes to unlocked VAR_UNKNOWN object.
///
/// @param[out]  tv  Object to initialize.
static inline void tv_init(typval_T *const tv)
{
  if (tv != NULL) {
    memset(tv, 0, sizeof(*tv));
  }
}

/// Empty string
///
/// Needed for hack which allows not allocating empty string and still not
/// crashing when freeing it.
extern const char *const tv_empty_string;

/// Specifies that free_unref_items() function has (not) been entered
extern bool tv_in_free_unref_items;

/// Iterate over a list
///
/// @param  modifier  Modifier: expected to be const or nothing, volatile should
///                   also work if you have any uses for the volatile list.
/// @param[in]  l  List to iterate over.
/// @param  li  Name of the variable with current listitem_T entry.
/// @param  code  Cycle body.
#define TV_LIST_ITER_MOD(modifier, l, li, code) \
  do { \
    modifier list_T *const l_ = (l); \
    if (l_ != NULL) { \
      for (modifier listitem_T *li = l_->lv_first; \
           li != NULL; li = li->li_next) { \
        code \
      } \
    } \
  } while (0)

/// Iterate over a list
///
/// To be used when you need to modify list or values you iterate over, use
/// #TV_LIST_ITER_CONST if you don’t.
///
/// @param[in]  l  List to iterate over.
/// @param  li  Name of the variable with current listitem_T entry.
/// @param  code  Cycle body.
#define TV_LIST_ITER(l, li, code) \
  TV_LIST_ITER_MOD( , l, li, code)

/// Iterate over a list
///
/// To be used when you don’t need to modify list or values you iterate over,
/// use #TV_LIST_ITER if you do.
///
/// @param[in]  l  List to iterate over.
/// @param  li  Name of the variable with current listitem_T entry.
/// @param  code  Cycle body.
#define TV_LIST_ITER_CONST(l, li, code) \
  TV_LIST_ITER_MOD(const, l, li, code)

// Below macros are macros to avoid duplicating code for functionally identical
// const and non-const function variants.

/// Get typval_T out of list item
///
/// @param[in]  li  List item to get typval_T from, must not be NULL.
///
/// @return Pointer to typval_T.
#define TV_LIST_ITEM_TV(li) (&(li)->li_tv)

/// Get next list item given the current one
///
/// @param[in]  l  List to get item from.
/// @param[in]  li  List item to get typval_T from.
///
/// @return Pointer to the next item or NULL.
#define TV_LIST_ITEM_NEXT(l, li) ((li)->li_next)

/// Get previous list item given the current one
///
/// @param[in]  l  List to get item from.
/// @param[in]  li  List item to get typval_T from.
///
/// @return Pointer to the previous item or NULL.
#define TV_LIST_ITEM_PREV(l, li) ((li)->li_prev)
// List argument is not used currently, but it is a must for lists implemented
// as a pair (size(in list), array) without terminator - basically for lists on
// top of kvec.

/// Iterate over a dictionary
///
/// @param[in]  d  Dictionary to iterate over.
/// @param  di  Name of the variable with current dictitem_T entry.
/// @param  code  Cycle body.
#define TV_DICT_ITER(d, di, code) \
  HASHTAB_ITER(&(d)->dv_hashtab, di##hi_, { \
    { \
      dictitem_T *const di = TV_DICT_HI2DI(di##hi_); \
      { \
        code \
      } \
    } \
  })

/// Get the float value
///
/// Raises an error if object is not number or floating-point.
///
/// @param[in]  tv  Vimscript object to get value from.
/// @param[out]  ret_f  Location where resulting float is stored.
///
/// @return true in case of success, false if tv is not a number or float.
static inline bool tv_get_float_chk(const typval_T *const tv, float_T *const ret_f)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT
{
  if (tv->v_type == VAR_FLOAT) {
    *ret_f = tv->vval.v_float;
    return true;
  }
  if (tv->v_type == VAR_NUMBER) {
    *ret_f = (float_T)tv->vval.v_number;
    return true;
  }
  semsg("%s", _("E808: Number or Float required"));
  return false;
}

/// Compute the `DictWatcher` address from a QUEUE node.
///
/// This only exists for .asan-blacklist (ASAN doesn't handle QUEUE_DATA pointer
/// arithmetic).
static inline DictWatcher *tv_dict_watcher_node_data(QUEUE *q)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET
    FUNC_ATTR_NO_SANITIZE_ADDRESS FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT
{
  return QUEUE_DATA(q, DictWatcher, node);
}

/// Check whether given typval_T contains a function
///
/// That is, whether it contains VAR_FUNC or VAR_PARTIAL.
///
/// @param[in]  tv  Typval to check.
///
/// @return True if it is a function or a partial, false otherwise.
static inline bool tv_is_func(const typval_T tv)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_CONST
{
  return tv.v_type == VAR_FUNC || tv.v_type == VAR_PARTIAL;
}

/// Specify that argument needs to be translated
///
/// Used for size_t length arguments to avoid calling gettext() and strlen()
/// unless needed.
#define TV_TRANSLATE (SIZE_MAX)

/// Specify that argument is a NUL-terminated C string
///
/// Used for size_t length arguments to avoid calling strlen() unless needed.
#define TV_CSTRING (SIZE_MAX - 1)

#ifdef UNIT_TESTING
// Do not use enum constants, see commit message.
EXTERN const size_t kTVCstring INIT( = TV_CSTRING);
EXTERN const size_t kTVTranslate INIT( = TV_TRANSLATE);
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

DLLEXPORT listitem_T *tv_list_item_remove(list_T *const l, listitem_T *const item)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_watch_add(list_T *const l, listwatch_T *const lw) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_watch_remove(list_T *const l, listwatch_T *const lwrem)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT list_T *tv_list_alloc(const ptrdiff_t len) FUNC_ATTR_NONNULL_RET;
DLLEXPORT void tv_list_init_static10(staticList10_T *const sl) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_init_static(list_T *const l) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_free_contents(list_T *const l) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_free_list(list_T *const l) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_free(list_T *const l) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_unref(list_T *const l);
DLLEXPORT void tv_list_drop_items(list_T *const l, listitem_T *const item, listitem_T *const item2)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_remove_items(list_T *const l, listitem_T *const item,
                                    listitem_T *const item2) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_move_items(list_T *const l, listitem_T *const item, listitem_T *const item2,
                                  list_T *const tgt_l, const int cnt) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_insert(list_T *const l, listitem_T *const ni, listitem_T *const item)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT void tv_list_insert_tv(list_T *const l, typval_T *const tv, listitem_T *const item);
DLLEXPORT void tv_list_append(list_T *const l, listitem_T *const item) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_append_tv(list_T *const l, typval_T *const tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT typval_T *tv_list_append_owned_tv(list_T *const l, typval_T tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_list_append_list(list_T *const l, list_T *const itemlist)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void tv_list_append_dict(list_T *const l, dict_T *const dict) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void tv_list_append_string(list_T *const l, const char *const str, const ssize_t len)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void tv_list_append_allocated_string(list_T *const l, char *const str)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void tv_list_append_number(list_T *const l, const varnumber_T n);
DLLEXPORT list_T *tv_list_copy(const vimconv_T *const conv, list_T *const orig, const bool deep,
                               const int copyID) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT listitem_T *tv_list_check_range_index_one(list_T *const l, int *const n1,
                                                    const bool quiet);
DLLEXPORT int tv_list_check_range_index_two(list_T *const l, int *const n1,
                                            const listitem_T *const li1, int *const n2,
                                            const bool quiet);
DLLEXPORT int tv_list_assign_range(list_T *const dest, list_T *const src, const int idx1_arg,
                                   const int idx2, const bool empty_idx2, const char *const op,
                                   const char *const varname);
DLLEXPORT void tv_list_flatten(list_T *list, listitem_T *first, int64_t maxitems, int64_t maxdepth)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void tv_list_extend(list_T *const l1, list_T *const l2, listitem_T *const bef)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int tv_list_concat(list_T *const l1, list_T *const l2, typval_T *const tv)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int tv_list_slice_or_index(list_T *list, bool range, varnumber_T n1_arg,
                                     varnumber_T n2_arg, bool exclusive, typval_T *rettv,
                                     bool verbose);
DLLEXPORT int tv_list_join(garray_T *const gap, list_T *const l, const char *const sep)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void f_join(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_list2str(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void tv_list_remove(typval_T *argvars, typval_T *rettv, const char *arg_errmsg);
DLLEXPORT void f_sort(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_uniq(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT bool tv_list_equal(list_T *const l1, list_T *const l2, const bool ic)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void tv_list_reverse(list_T *const l);
DLLEXPORT listitem_T *tv_list_find(list_T *const l, int n)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T tv_list_find_nr(list_T *const l, const int n, bool *const ret_error)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const char *tv_list_find_str(list_T *const l, const int n) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int tv_list_idx_of_item(const list_T *const l, const listitem_T *const item)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT void tv_dict_watcher_add(dict_T *const dict, const char *const key_pattern,
                                   const size_t key_pattern_len, Callback callback)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT bool tv_callback_equal(const Callback *cb1, const Callback *cb2)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void callback_free(Callback *callback) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void callback_put(Callback *cb, typval_T *tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void callback_copy(Callback *dest, Callback *src) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *callback_to_string(Callback *cb, Arena *arena);
DLLEXPORT bool tv_dict_watcher_remove(dict_T *const dict, const char *const key_pattern,
                                      const size_t key_pattern_len, Callback callback)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void tv_dict_watcher_notify(dict_T *const dict, const char *const key,
                                      typval_T *const newtv, typval_T *const oldtv)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT dictitem_T *tv_dict_item_alloc_len(const char *const key, const size_t key_len)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC;
DLLEXPORT dictitem_T *tv_dict_item_alloc(const char *const key)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC;
DLLEXPORT void tv_dict_item_free(dictitem_T *const item) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT dictitem_T *tv_dict_item_copy(dictitem_T *const di)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void tv_dict_item_remove(dict_T *const dict, dictitem_T *const item)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT dict_T *tv_dict_alloc(void) FUNC_ATTR_NONNULL_RET FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void tv_dict_free_contents(dict_T *const d) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_dict_free_dict(dict_T *const d) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_dict_free(dict_T *const d) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_dict_unref(dict_T *const d);
DLLEXPORT dictitem_T *tv_dict_find(const dict_T *const d, const char *const key,
                                   const ptrdiff_t len)
  FUNC_ATTR_NONNULL_ARG(2) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool tv_dict_has_key(const dict_T *const d, const char *const key)
  FUNC_ATTR_NONNULL_ARG(2) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int tv_dict_get_tv(dict_T *d, const char *const key, typval_T *rettv);
DLLEXPORT varnumber_T tv_dict_get_number(const dict_T *const d, const char *const key)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T tv_dict_get_number_def(const dict_T *const d, const char *const key,
                                             const int def)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T tv_dict_get_bool(const dict_T *const d, const char *const key, const int def)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char **tv_dict_to_env(dict_T *denv);
DLLEXPORT char *tv_dict_get_string(const dict_T *const d, const char *const key, const bool save)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const char *tv_dict_get_string_buf(const dict_T *const d, const char *const key,
                                             char *const numbuf) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const char *tv_dict_get_string_buf_chk(const dict_T *const d, const char *const key,
                                                 const ptrdiff_t key_len, char *const numbuf,
                                                 const char *const def)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool tv_dict_get_callback(dict_T *const d, const char *const key, const ptrdiff_t key_len,
                                    Callback *const result)
  FUNC_ATTR_NONNULL_ARG(2, 4) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int tv_dict_wrong_func_name(dict_T *d, typval_T *tv, const char *name);
DLLEXPORT int tv_dict_add(dict_T *const d, dictitem_T *const item) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int tv_dict_add_list(dict_T *const d, const char *const key, const size_t key_len,
                               list_T *const list) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int tv_dict_add_tv(dict_T *d, const char *key, const size_t key_len, typval_T *tv);
DLLEXPORT int tv_dict_add_dict(dict_T *const d, const char *const key, const size_t key_len,
                               dict_T *const dict) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int tv_dict_add_nr(dict_T *const d, const char *const key, const size_t key_len,
                             const varnumber_T nr);
DLLEXPORT int tv_dict_add_float(dict_T *const d, const char *const key, const size_t key_len,
                                const float_T nr);
DLLEXPORT int tv_dict_add_bool(dict_T *const d, const char *const key, const size_t key_len,
                               BoolVarValue val);
DLLEXPORT int tv_dict_add_str(dict_T *const d, const char *const key, const size_t key_len,
                              const char *const val) FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT int tv_dict_add_str_len(dict_T *const d, const char *const key, const size_t key_len,
                                  const char *const val, int len) FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT int tv_dict_add_allocated_str(dict_T *const d, const char *const key,
                                        const size_t key_len, char *const val)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT int tv_dict_add_func(dict_T *const d, const char *const key, const size_t key_len,
                               ufunc_T *const fp) FUNC_ATTR_NONNULL_ARG(1, 2, 4);
DLLEXPORT void tv_dict_clear(dict_T *const d) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_dict_extend(dict_T *const d1, dict_T *const d2, const char *const action)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool tv_dict_equal(dict_T *const d1, dict_T *const d2, const bool ic)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT dict_T *tv_dict_copy(const vimconv_T *const conv, dict_T *const orig, const bool deep,
                               const int copyID);
DLLEXPORT void tv_dict_set_keys_readonly(dict_T *const dict) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT blob_T *tv_blob_alloc(void) FUNC_ATTR_NONNULL_RET;
DLLEXPORT void tv_blob_free(blob_T *const b) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void tv_blob_unref(blob_T *const b);
DLLEXPORT bool tv_blob_equal(const blob_T *const b1, const blob_T *const b2)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int tv_blob_slice_or_index(const blob_T *blob, bool is_range, varnumber_T n1,
                                     varnumber_T n2, bool exclusive, typval_T *rettv);
DLLEXPORT int tv_blob_check_index(int bloblen, varnumber_T n1, bool quiet);
DLLEXPORT int tv_blob_check_range(int bloblen, varnumber_T n1, varnumber_T n2, bool quiet);
DLLEXPORT int tv_blob_set_range(blob_T *dest, varnumber_T n1, varnumber_T n2, typval_T *src);
DLLEXPORT void tv_blob_set_append(blob_T *blob, int idx, uint8_t byte);
DLLEXPORT void tv_blob_remove(typval_T *argvars, typval_T *rettv, const char *arg_errmsg);
DLLEXPORT void f_blob2list(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_list2blob(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT list_T *tv_list_alloc_ret(typval_T *const ret_tv, const ptrdiff_t len)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT dict_T *tv_dict_alloc_lock(VarLockStatus lock) FUNC_ATTR_NONNULL_RET;
DLLEXPORT void tv_dict_alloc_ret(typval_T *const ret_tv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void f_items(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_keys(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_values(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_has_key(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void tv_dict_remove(typval_T *argvars, typval_T *rettv, const char *arg_errmsg);
DLLEXPORT blob_T *tv_blob_alloc_ret(typval_T *const ret_tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT void tv_blob_copy(blob_T *const from, typval_T *const to) FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void tv_clear(typval_T *const tv);
DLLEXPORT void tv_free(typval_T *tv);
DLLEXPORT void tv_copy(const typval_T *const from, typval_T *const to);
DLLEXPORT void tv_item_lock(typval_T *const tv, const int deep, const bool lock,
                            const bool check_refcount) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool tv_islocked(const typval_T *const tv)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool tv_check_lock(const typval_T *tv, const char *name, size_t name_len)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool value_check_lock(VarLockStatus lock, const char *name, size_t name_len);
DLLEXPORT bool tv_equal(typval_T *const tv1, typval_T *const tv2, const bool ic)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool tv_check_str_or_nr(const typval_T *const tv)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool tv_check_num(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool tv_check_str(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T tv_get_number(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T tv_get_number_chk(const typval_T *const tv, bool *const ret_error)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT varnumber_T tv_get_bool(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT varnumber_T tv_get_bool_chk(const typval_T *const tv, bool *const ret_error)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT linenr_T tv_get_lnum(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT linenr_T tv_get_lnum_buf(const typval_T *const tv, const buf_T *const buf)
  FUNC_ATTR_NONNULL_ARG(1) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT float_T tv_get_float(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int tv_check_for_string_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_nonempty_string_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_opt_string_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_number_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_opt_number_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_float_or_nr_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_bool_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_opt_bool_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_blob_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_list_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_dict_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_nonnull_dict_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_opt_dict_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_string_or_number_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_buffer_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_lnum_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_string_or_list_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_string_or_list_or_blob_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_opt_string_or_list_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_string_or_func_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT int tv_check_for_list_or_blob_arg(const typval_T *const args, const int idx)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT const char *tv_get_string_buf_chk(const typval_T *const tv, char *const buf)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const char *tv_get_string_chk(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const char *tv_get_string(const typval_T *const tv)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const char *tv_get_string_buf(const typval_T *const tv, char *const buf)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool tv2bool(const typval_T *const tv);
DLLEXPORT int check_repeat_count(varnumber_T slen, varnumber_T n);
#include "nvim/func_attr.h"
