#pragma once

#include <stddef.h>

#include "nvim/hashtab_defs.h"  // IWYU pragma: keep

/// Magic number used for hashitem "hi_key" value indicating a deleted item
///
/// Only the address is used.
extern char hash_removed;

/// The address of "hash_removed" is used as a magic number
/// for hi_key to indicate a removed item.
#define HI_KEY_REMOVED (&hash_removed)
#define HASHITEM_EMPTY(hi) ((hi)->hi_key == NULL || (hi)->hi_key == &hash_removed)

/// Iterate over a hashtab
///
/// @param[in]  ht  Hashtab to iterate over.
/// @param  hi  Name of the variable with current hashtab entry.
/// @param  code  Cycle body.
#define HASHTAB_ITER(ht, hi, code) \
  do { \
    hashtab_T *const hi##ht_ = (ht); \
    size_t hi##todo_ = hi##ht_->ht_used; \
    for (hashitem_T *hi = hi##ht_->ht_array; hi##todo_; hi++) { \
      if (!HASHITEM_EMPTY(hi)) { \
        hi##todo_--; \
        { \
          code \
        } \
      } \
    } \
  } while (0)

void hash_init(hashtab_T *ht);
void hash_clear(hashtab_T *ht);
void hash_clear_all(hashtab_T *ht, unsigned off);
hashitem_T *hash_find(const hashtab_T *const ht, const char *const key);
hashitem_T *hash_find_len(const hashtab_T *const ht, const char *const key, const size_t len);
hashitem_T *hash_lookup(const hashtab_T *const ht, const char *const key, const size_t key_len,
                        const hash_T hash);
void hash_debug_results(void);
int hash_add(hashtab_T *ht, char *key);
void hash_add_item(hashtab_T *ht, hashitem_T *hi, char *key, hash_T hash);
void hash_remove(hashtab_T *ht, hashitem_T *hi);
void hash_lock(hashtab_T *ht);
void hash_unlock(hashtab_T *ht);
hash_T hash_hash(const char *key);
hash_T hash_hash_len(const char *key, const size_t len);
const char *_hash_key_removed(void);
