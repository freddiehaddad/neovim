//! Handling of a hashtable with Vim-specific properties (`src/nvim/hashtab.c`).
#![forbid(unsafe_code)]

use crate::message::siemsg;
use std::ffi::CStr;

pub type HashVal = usize;

pub const HT_INIT_SIZE: usize = 16;
const PERTURB_SHIFT: usize = 5;

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct Hashitem {
    pub hi_hash: HashVal,
    pub hi_removed: bool,
    pub hi_di: crate::eval::typval::DictitemId,
}

impl Hashitem {
    #[inline]
    pub const fn is_null_slot(&self) -> bool {
        self.hi_di.is_null() && !self.hi_removed
    }

    #[inline]
    pub fn key_cstr(&self) -> Option<&CStr> {
        self.hi_di
            .opt()
            .and_then(|di| CStr::from_bytes_until_nul(&di.di_key).ok())
    }

    #[inline]
    pub fn di_opt(&self) -> Option<&crate::eval::typval::Dictitem> {
        self.hi_di.opt()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Hashtab {
    pub ht_mask: HashVal,
    pub ht_used: usize,
    pub ht_filled: usize,
    pub ht_changed: i32,
    pub ht_locked: i32,
    pub ht_array: Vec<Hashitem>,
}

impl Hashtab {
    pub const fn new() -> Self {
        Self {
            ht_mask: HT_INIT_SIZE - 1,
            ht_used: 0,
            ht_filled: 0,
            ht_changed: 0,
            ht_locked: 0,
            ht_array: Vec::new(),
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[Hashitem] {
        if self.ht_array.is_empty() {
            &EMPTY_HASHARRAY
        } else {
            &self.ht_array
        }
    }

    #[inline]
    pub fn iter_items(&self) -> impl Iterator<Item = (&Hashitem, &crate::eval::typval::Dictitem)> {
        self.ht_array
            .iter()
            .filter_map(|hi| hi.hi_di.opt().map(|di| (hi, di)))
            .take(self.ht_used)
    }

    #[inline]
    pub fn iter_items_mut(
        &self,
    ) -> impl Iterator<Item = (&Hashitem, &crate::eval::typval::Dictitem)> {
        self.ht_array
            .iter()
            .filter_map(|hi| hi.di_opt().map(|di| (hi, di)))
            .take(self.ht_used)
    }

    #[inline]
    pub fn find_hi(&self, key: &CStr) -> Option<Hashitem> {
        let hi = hash_find(self, key);
        (!hashitem_empty(&hi)).then_some(hi)
    }

    #[inline]
    pub fn find_hi_len(&self, key: &[u8]) -> Option<Hashitem> {
        let hi = hash_find_len(self, key);
        (!hashitem_empty(&hi)).then_some(hi)
    }

    #[inline]
    pub fn find_hi_idx(&self, key: &CStr) -> Option<usize> {
        let idx = hash_find_idx(self, key);
        (!hashitem_empty(&self.as_slice()[idx])).then_some(idx)
    }

    #[inline]
    pub fn remove_at(&mut self, idx: usize) {
        let hi = &mut self.ht_array[idx];
        self.ht_used -= 1;
        self.ht_changed += 1;
        hi.hi_removed = true;
        hi.hi_di = crate::eval::typval::DictitemId::NULL;
        hash_may_resize(self, 0);
    }
}

impl Default for Hashtab {
    fn default() -> Self {
        Self::new()
    }
}

const EMPTY_HASHARRAY: [Hashitem; HT_INIT_SIZE] = [const {
    Hashitem {
        hi_hash: 0,
        hi_removed: false,
        hi_di: crate::eval::typval::DictitemId::NULL,
    }
}; HT_INIT_SIZE];

#[inline]
pub fn hashitem_empty(hi: &Hashitem) -> bool {
    hi.hi_di.is_null()
}

#[inline]
fn key_matches(hi_key: Option<&CStr>, key: &[u8]) -> bool {
    hi_key.map_or(&b""[..], CStr::to_bytes) == key
}

/// Free the array of a hash table without freeing contained values (`hash_clear`).
pub fn hash_clear(ht: &mut Hashtab) {
    *ht = Hashtab::new();
}

/// Find slot index for NUL-terminated `key` in `ht`.
pub fn hash_find_idx(ht: &Hashtab, key: &CStr) -> usize {
    let hash = hash_hash(key);
    hash_lookup_idx(ht, key.to_bytes(), hash)
}

/// Find slot index for `key` slice in `ht`.
pub fn hash_find_len_idx(ht: &Hashtab, key: &[u8]) -> usize {
    let hash = hash_hash_len(key);
    hash_lookup_idx(ht, key, hash)
}

/// Find item for NUL-terminated `key` in `ht` (`hash_find`).
pub fn hash_find(ht: &Hashtab, key: &CStr) -> Hashitem {
    ht.as_slice()[hash_find_idx(ht, key)].clone()
}

/// Find item for `key` slice in `ht` (`hash_find_len`).
pub fn hash_find_len(ht: &Hashtab, key: &[u8]) -> Hashitem {
    ht.as_slice()[hash_find_len_idx(ht, key)].clone()
}

/// Find slot index for `key` slice with precomputed `hash` in `ht` (`hash_lookup`).
pub fn hash_lookup_idx(ht: &Hashtab, key: &[u8], hash: HashVal) -> usize {
    let arr = ht.as_slice();

    let mut idx = hash & ht.ht_mask;
    if arr[idx].is_null_slot() {
        return idx;
    }

    let mut free_idx: Option<usize> = None;
    if arr[idx].hi_removed {
        free_idx = Some(idx);
    } else if arr[idx].hi_hash == hash && key_matches(arr[idx].key_cstr(), key) {
        return idx;
    }

    let mut perturb = hash;
    loop {
        idx = idx.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1);
        let slot_idx = idx & ht.ht_mask;
        let item = &arr[slot_idx];

        if item.is_null_slot() {
            return free_idx.unwrap_or(slot_idx);
        }

        if item.hi_hash == hash && !item.hi_removed && key_matches(item.key_cstr(), key) {
            return slot_idx;
        }

        if item.hi_removed && free_idx.is_none() {
            free_idx = Some(slot_idx);
        }

        perturb >>= PERTURB_SHIFT;
    }
}

/// Add dictionary item `di` to hashtable `ht` (`hash_add`).
pub fn hash_add(ht: &mut Hashtab, di: &crate::eval::typval::Dictitem) -> bool {
    let key = di.key_cstr();
    let hash = hash_hash(key);
    let idx = hash_lookup_idx(ht, key.to_bytes(), hash);
    if !hashitem_empty(&ht.as_slice()[idx]) {
        siemsg!(
            c"E685: Internal error: hash_add(): duplicate key \"%s\"",
            key,
        );
        return false;
    }
    hash_add_item(ht, idx, di, hash);
    true
}

/// Add item at `idx` with precomputed `hash` to `ht` (`hash_add_item`).
pub fn hash_add_item(
    ht: &mut Hashtab,
    idx: usize,
    di: &crate::eval::typval::Dictitem,
    hash: HashVal,
) {
    if ht.ht_array.is_empty() {
        ht.ht_array = vec![Hashitem::default(); HT_INIT_SIZE];
    }
    let hi_ref = &mut ht.ht_array[idx];
    ht.ht_used += 1;
    ht.ht_changed += 1;
    if hi_ref.is_null_slot() {
        ht.ht_filled += 1;
    }
    hi_ref.hi_removed = false;
    hi_ref.hi_di = crate::eval::typval::DictitemId::from(di);
    hi_ref.hi_hash = hash;

    hash_may_resize(ht, 0);
}

/// Remove item at `idx` from `ht` (`hash_remove`).
pub fn hash_remove(ht: &mut Hashtab, idx: usize) {
    ht.remove_at(idx);
}

/// Lock hashtable to prevent `ht_array` reallocation (`hash_lock`).
pub fn hash_lock(ht: &mut Hashtab) {
    ht.ht_locked += 1;
}

/// Unlock hashtable and resize if needed (`hash_unlock`).
pub fn hash_unlock(ht: &mut Hashtab) {
    ht.ht_locked -= 1;
    hash_may_resize(ht, 0);
}

fn hash_may_resize(ht: &mut Hashtab, mut minitems: usize) {
    if ht.ht_locked > 0 {
        return;
    }

    let oldsize = ht.ht_mask + 1;
    let is_small = oldsize == HT_INIT_SIZE;

    let minsize = if minitems == 0 {
        if ht.ht_filled < HT_INIT_SIZE - 1 && is_small {
            return;
        }
        if ht.ht_filled * 3 < oldsize * 2 && ht.ht_used > oldsize / 5 {
            return;
        }
        if ht.ht_used > 1000 {
            ht.ht_used * 2
        } else {
            ht.ht_used * 4
        }
    } else {
        if minitems < ht.ht_used {
            minitems = ht.ht_used;
        }
        (minitems * 3).div_ceil(2)
    };

    let mut newsize = HT_INIT_SIZE;
    while newsize < minsize {
        newsize <<= 1;
        assert!(newsize != 0);
    }

    let newarray_is_small = newsize == HT_INIT_SIZE;
    if !newarray_is_small && newsize == oldsize && ht.ht_filled * 3 < oldsize * 2 {
        return;
    }

    if ht.ht_used == 0 && newarray_is_small {
        ht.ht_array = Vec::new();
        ht.ht_mask = HT_INIT_SIZE - 1;
        ht.ht_filled = 0;
        ht.ht_changed += 1;
        return;
    }

    let oldarray = std::mem::replace(&mut ht.ht_array, vec![Hashitem::default(); newsize]);

    let newmask = newsize - 1;
    let mut todo = ht.ht_used;

    for item in oldarray {
        if todo == 0 {
            break;
        }
        if !hashitem_empty(&item) {
            let mut newi = item.hi_hash & newmask;
            if !ht.ht_array[newi].is_null_slot() {
                let mut perturb = item.hi_hash;
                loop {
                    newi = newi.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1);
                    if ht.ht_array[newi & newmask].is_null_slot() {
                        break;
                    }
                    perturb >>= PERTURB_SHIFT;
                }
            }
            ht.ht_array[newi & newmask] = item;
            todo -= 1;
        }
    }

    ht.ht_mask = newmask;
    ht.ht_filled = ht.ht_used;
    ht.ht_changed += 1;
}

/// Compute hash for NUL-terminated `key` (`hash_hash`).
pub fn hash_hash(key: &CStr) -> HashVal {
    hash_hash_len(key.to_bytes())
}

/// Compute hash for `key` slice (`hash_hash_len`).
pub fn hash_hash_len(key: &[u8]) -> HashVal {
    let Some((&first, rest)) = key.split_first() else {
        return 0;
    };
    let mut hash = usize::from(first);
    for &b in rest {
        hash = hash.wrapping_mul(101).wrapping_add(usize::from(b));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::CString;

    #[test]
    fn hash_init_insert_lookup_resize_and_remove() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut ht_val = Hashtab::new();
        let keys: Vec<CString> = (0..50)
            .map(|i| CString::new(format!("key_{i}")).unwrap())
            .collect();

        {
            let ht = &mut ht_val;
            assert_eq!(ht.ht_used, 0);

            let items: Vec<crate::eval::typval::DictitemId> = keys
                .iter()
                .map(|k| crate::eval::typval::tv_dict_item_alloc(k.as_c_str()))
                .collect();

            for di in &items {
                assert!(hash_add(ht, di));
            }
            assert_eq!(ht.ht_used, 50);
            // Duplicate key returns FAIL
            crate::globals::g_st().emsg_skip += 1;
            let first_di = &items[0];
            assert!(!hash_add(ht, first_di));
            crate::globals::g_st().emsg_skip -= 1;

            for (k, di) in keys.iter().zip(&items) {
                let hi = hash_find(ht, k.as_c_str());
                assert!(!hashitem_empty(&hi));
                assert_eq!(hi.key_cstr(), Some(k.as_c_str()));
                assert_eq!(&hi.hi_di, di);
            }

            // Remove all keys and verify table shrinks back to HT_INIT_SIZE
            for k in &keys {
                let idx = hash_find_idx(ht, k.as_c_str());
                hash_remove(ht, idx);
            }
            assert_eq!(ht.ht_used, 0);
            assert_eq!(ht.ht_mask, HT_INIT_SIZE - 1);
            hash_clear(ht);
            assert!(ht.ht_array.is_empty());

            for di in &items {
                crate::eval::typval::tv_dict_item_free(di);
            }
        }
    }
}
