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
            .map(crate::eval::typval::Dictitem::key_cstr)
    }

    #[inline]
    pub fn key_bytes(&self) -> Option<&[u8]> {
        self.hi_di
            .opt()
            .map(crate::eval::typval::Dictitem::key_bytes)
    }

    #[inline]
    pub fn di_opt(&self) -> Option<&crate::eval::typval::Dictitem> {
        self.hi_di.opt()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Hashtab {
    pub ht_mask: std::cell::Cell<HashVal>,
    pub ht_used: std::cell::Cell<usize>,
    pub ht_filled: std::cell::Cell<usize>,
    pub ht_changed: std::cell::Cell<i32>,
    pub ht_locked: std::cell::Cell<i32>,
    pub ht_array: std::cell::RefCell<Vec<Hashitem>>,
}

impl Hashtab {
    pub const fn new() -> Self {
        Self {
            ht_mask: std::cell::Cell::new(HT_INIT_SIZE - 1),
            ht_used: std::cell::Cell::new(0),
            ht_filled: std::cell::Cell::new(0),
            ht_changed: std::cell::Cell::new(0),
            ht_locked: std::cell::Cell::new(0),
            ht_array: std::cell::RefCell::new(Vec::new()),
        }
    }

    #[inline]
    pub fn slot(&self, idx: usize) -> Hashitem {
        let arr = self.ht_array.borrow();
        if arr.is_empty() {
            EMPTY_HASHARRAY[idx].clone()
        } else {
            arr[idx].clone()
        }
    }

    #[inline]
    pub fn slot_is_empty(&self, idx: usize) -> bool {
        let arr = self.ht_array.borrow();
        if arr.is_empty() {
            true
        } else {
            arr[idx].hi_di.is_null()
        }
    }

    #[inline]
    pub fn next_used_di(&self, mut idx: usize) -> (usize, crate::eval::typval::DictitemId) {
        let arr = self.ht_array.borrow();
        while arr[idx].hi_di.is_null() {
            idx += 1;
        }
        (idx + 1, arr[idx].hi_di.clone())
    }

    #[inline]
    pub fn array_len(&self) -> usize {
        self.ht_array.borrow().len()
    }

    #[inline]
    pub fn take(&self) -> Self {
        Self {
            ht_mask: std::cell::Cell::new(self.ht_mask.replace(HT_INIT_SIZE - 1)),
            ht_used: std::cell::Cell::new(self.ht_used.replace(0)),
            ht_filled: std::cell::Cell::new(self.ht_filled.replace(0)),
            ht_changed: std::cell::Cell::new(self.ht_changed.replace(0)),
            ht_locked: std::cell::Cell::new(self.ht_locked.replace(0)),
            ht_array: std::cell::RefCell::new(std::mem::take(&mut *self.ht_array.borrow_mut())),
        }
    }

    #[inline]
    pub fn replace(&self, other: Self) {
        self.ht_mask.set(other.ht_mask.get());
        self.ht_used.set(other.ht_used.get());
        self.ht_filled.set(other.ht_filled.get());
        self.ht_changed.set(other.ht_changed.get());
        self.ht_locked.set(other.ht_locked.get());
        *self.ht_array.borrow_mut() = other.ht_array.into_inner();
    }

    #[inline]
    pub fn iter_items(&self) -> std::vec::IntoIter<(Hashitem, crate::eval::typval::DictitemId)> {
        self.ht_array
            .borrow()
            .iter()
            .filter(|hi| !hi.hi_di.is_null())
            .take(self.ht_used.get())
            .map(|hi| (hi.clone(), hi.hi_di.clone()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[inline]
    pub fn iter_items_mut(
        &self,
    ) -> std::vec::IntoIter<(Hashitem, crate::eval::typval::DictitemId)> {
        self.iter_items()
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
        (!self.slot_is_empty(idx)).then_some(idx)
    }

    #[inline]
    pub fn remove_at(&self, idx: usize) {
        {
            let mut arr = self.ht_array.borrow_mut();
            let hi = &mut arr[idx];
            self.ht_used.set(self.ht_used.get() - 1);
            self.ht_changed.set(self.ht_changed.get() + 1);
            hi.hi_removed = true;
            hi.hi_di = crate::eval::typval::DictitemId::NULL;
        }
        hash_may_resize(self, 0);
    }

    #[inline]
    pub fn take_di_for_clear(&self, idx: usize) -> Option<crate::eval::typval::DictitemId> {
        let mut arr = self.ht_array.borrow_mut();
        let hi = &mut arr[idx];
        if hi.hi_di.is_null() {
            None
        } else {
            self.ht_used.set(self.ht_used.get() - 1);
            self.ht_changed.set(self.ht_changed.get() + 1);
            hi.hi_removed = true;
            Some(std::mem::take(&mut hi.hi_di))
        }
    }
    #[inline]
    pub fn reset_for_pool(&self) {
        self.ht_mask.set(HT_INIT_SIZE - 1);
        self.ht_used.set(0);
        self.ht_filled.set(0);
        self.ht_changed.set(0);
        self.ht_locked.set(0);
        let mut arr = self.ht_array.borrow_mut();
        if arr.len() == HT_INIT_SIZE {
            for hi in arr.iter_mut() {
                hi.hi_hash = 0;
                hi.hi_removed = false;
                hi.hi_di = crate::eval::typval::DictitemId::NULL;
            }
        } else {
            *arr = Vec::new();
        }
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

/// Free the array of a hash table without freeing contained values (`hash_clear`).
pub fn hash_clear(ht: &Hashtab) {
    ht.replace(Hashtab::new());
}

/// Find slot index for NUL-terminated `key` in `ht`.
pub fn hash_find_idx(ht: &Hashtab, key: &CStr) -> usize {
    hash_find_len_idx(ht, key.to_bytes())
}

/// Find slot index for `key` slice in `ht`.
pub fn hash_find_len_idx(ht: &Hashtab, key: &[u8]) -> usize {
    let hash = hash_hash_len(key);
    hash_lookup_idx(ht, key, hash)
}

/// Find item for NUL-terminated `key` in `ht` (`hash_find`).
pub fn hash_find(ht: &Hashtab, key: &CStr) -> Hashitem {
    ht.slot(hash_find_idx(ht, key))
}

/// Find item for `key` slice in `ht` (`hash_find_len`).
pub fn hash_find_len(ht: &Hashtab, key: &[u8]) -> Hashitem {
    ht.slot(hash_find_len_idx(ht, key))
}

/// Find slot index for `key` slice with precomputed `hash` in `ht` (`hash_lookup`).
pub fn hash_lookup_idx(ht: &Hashtab, key: &[u8], hash: HashVal) -> usize {
    let arr_guard = ht.ht_array.borrow();
    if arr_guard.is_empty() {
        return hash & ht.ht_mask.get();
    }
    let arr: &[Hashitem] = &arr_guard;
    let mask = ht.ht_mask.get();

    let mut idx = hash & mask;
    if arr[idx].is_null_slot() {
        return idx;
    }

    let mut free_idx: Option<usize> = None;
    if arr[idx].hi_removed {
        free_idx = Some(idx);
    } else if arr[idx].hi_hash == hash && arr[idx].key_bytes() == Some(key) {
        return idx;
    }

    let mut perturb = hash;
    loop {
        idx = idx.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1);
        let slot_idx = idx & mask;
        let item = &arr[slot_idx];

        if item.is_null_slot() {
            return free_idx.unwrap_or(slot_idx);
        }

        if item.hi_hash == hash && !item.hi_removed && item.key_bytes() == Some(key) {
            return slot_idx;
        }

        if item.hi_removed && free_idx.is_none() {
            free_idx = Some(slot_idx);
        }

        perturb >>= PERTURB_SHIFT;
    }
}

/// Add dictionary item `di` to hashtable `ht` (`hash_add`).
pub fn hash_add(ht: &Hashtab, di: &crate::eval::typval::Dictitem) -> bool {
    let key = di.key_bytes();
    let hash = hash_hash_len(key);
    let idx = hash_lookup_idx(ht, key, hash);
    if !ht.slot_is_empty(idx) {
        siemsg!(
            c"E685: Internal error: hash_add(): duplicate key \"%s\"",
            di.key_cstr(),
        );
        return false;
    }
    hash_add_item(ht, idx, di, hash);
    true
}

/// Add item at `idx` with precomputed `hash` to `ht` (`hash_add_item`).
pub fn hash_add_item(ht: &Hashtab, idx: usize, di: &crate::eval::typval::Dictitem, hash: HashVal) {
    {
        let mut arr = ht.ht_array.borrow_mut();
        if arr.is_empty() {
            *arr = vec![Hashitem::default(); HT_INIT_SIZE];
        }
        let hi_ref = &mut arr[idx];
        ht.ht_used.set(ht.ht_used.get() + 1);
        ht.ht_changed.set(ht.ht_changed.get() + 1);
        if hi_ref.is_null_slot() {
            ht.ht_filled.set(ht.ht_filled.get() + 1);
        }
        hi_ref.hi_removed = false;
        hi_ref.hi_di = crate::eval::typval::DictitemId::from(di);
        hi_ref.hi_hash = hash;
    }

    hash_may_resize(ht, 0);
}

/// Remove item at `idx` from `ht` (`hash_remove`).
pub fn hash_remove(ht: &Hashtab, idx: usize) {
    ht.remove_at(idx);
}

/// Lock hashtable to prevent `ht_array` reallocation (`hash_lock`).
pub fn hash_lock(ht: &Hashtab) {
    ht.ht_locked.set(ht.ht_locked.get() + 1);
}

/// Unlock hashtable and resize if needed (`hash_unlock`).
pub fn hash_unlock(ht: &Hashtab) {
    ht.ht_locked.set(ht.ht_locked.get() - 1);
    hash_may_resize(ht, 0);
}

fn hash_may_resize(ht: &Hashtab, mut minitems: usize) {
    if ht.ht_locked.get() > 0 {
        return;
    }

    let oldsize = ht.ht_mask.get() + 1;
    let is_small = oldsize == HT_INIT_SIZE;
    let used = ht.ht_used.get();
    let filled = ht.ht_filled.get();

    let minsize = if minitems == 0 {
        if filled < HT_INIT_SIZE - 1 && is_small {
            return;
        }
        if filled * 3 < oldsize * 2 && used > oldsize / 5 {
            return;
        }
        if used > 1000 { used * 2 } else { used * 4 }
    } else {
        if minitems < used {
            minitems = used;
        }
        (minitems * 3).div_ceil(2)
    };

    let mut newsize = HT_INIT_SIZE;
    while newsize < minsize {
        newsize <<= 1;
        assert!(newsize != 0);
    }

    let newarray_is_small = newsize == HT_INIT_SIZE;
    if !newarray_is_small && newsize == oldsize && filled * 3 < oldsize * 2 {
        return;
    }

    if used == 0 && newarray_is_small {
        *ht.ht_array.borrow_mut() = Vec::new();
        ht.ht_mask.set(HT_INIT_SIZE - 1);
        ht.ht_filled.set(0);
        ht.ht_changed.set(ht.ht_changed.get() + 1);
        return;
    }

    let mut arr = ht.ht_array.borrow_mut();
    let oldarray = std::mem::replace(&mut *arr, vec![Hashitem::default(); newsize]);

    let newmask = newsize - 1;
    let mut todo = used;

    for item in oldarray {
        if todo == 0 {
            break;
        }
        if !hashitem_empty(&item) {
            let mut newi = item.hi_hash & newmask;
            if !arr[newi].is_null_slot() {
                let mut perturb = item.hi_hash;
                loop {
                    newi = newi.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1);
                    if arr[newi & newmask].is_null_slot() {
                        break;
                    }
                    perturb >>= PERTURB_SHIFT;
                }
            }
            arr[newi & newmask] = item;
            todo -= 1;
        }
    }

    ht.ht_mask.set(newmask);
    ht.ht_filled.set(used);
    ht.ht_changed.set(ht.ht_changed.get() + 1);
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
            assert_eq!(ht.ht_used.get(), 0);

            let items: Vec<crate::eval::typval::DictitemId> = keys
                .iter()
                .map(|k| crate::eval::typval::tv_dict_item_alloc(k.as_c_str()))
                .collect();

            for di in &items {
                assert!(hash_add(ht, di));
            }
            assert_eq!(ht.ht_used.get(), 50);
            // Duplicate key returns FAIL
            crate::globals::g_st()
                .emsg_skip
                .set(crate::globals::g_st().emsg_skip.get() + (1));
            let first_di = &items[0];
            assert!(!hash_add(ht, first_di));
            crate::globals::g_st()
                .emsg_skip
                .set(crate::globals::g_st().emsg_skip.get() - (1));

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
            assert_eq!(ht.ht_used.get(), 0);
            assert_eq!(ht.ht_mask.get(), HT_INIT_SIZE - 1);
            hash_clear(ht);
            assert!(ht.ht_array.borrow().is_empty());

            for di in &items {
                crate::eval::typval::tv_dict_item_free(di);
            }
        }
    }
}
