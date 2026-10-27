//! Handling of a hashtable with Vim-specific properties (`src/nvim/hashtab.c`).

use crate::message::siemsg;
use std::ffi::{CStr, c_char, c_int};

use crate::math::{FAIL, OK};

pub type hash_T = usize;

pub const HT_INIT_SIZE: usize = 16;
const PERTURB_SHIFT: usize = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct hashitem_T {
    pub hi_hash: hash_T,
    pub hi_removed: bool,
    pub hi_di: *mut crate::eval::typval::dictitem_T,
}

impl hashitem_T {
    #[inline]
    pub const fn is_null_slot(&self) -> bool {
        self.hi_di.is_null() && !self.hi_removed
    }

    #[inline]
    pub unsafe fn key_ptr(&self) -> *mut c_char {
        if self.hi_di.is_null() {
            std::ptr::null_mut()
        } else {
            unsafe { (*self.hi_di).di_key.as_ptr().cast_mut() }
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct hashtab_T {
    pub ht_mask: hash_T,
    pub ht_used: usize,
    pub ht_filled: usize,
    pub ht_changed: c_int,
    pub ht_locked: c_int,
    pub ht_array: Vec<hashitem_T>,
}

impl hashtab_T {
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
    pub fn array_ptr(&self) -> *mut hashitem_T {
        if self.ht_array.is_empty() {
            empty_hasharray_ptr()
        } else {
            self.ht_array.as_ptr().cast_mut()
        }
    }
}

impl Default for hashtab_T {
    fn default() -> Self {
        Self::new()
    }
}

static mut EMPTY_HASHARRAY: [hashitem_T; HT_INIT_SIZE] = [hashitem_T {
    hi_hash: 0,
    hi_removed: false,
    hi_di: std::ptr::null_mut(),
}; HT_INIT_SIZE];

#[inline]
pub const fn empty_hasharray_ptr() -> *mut hashitem_T {
    (&raw mut EMPTY_HASHARRAY).cast::<hashitem_T>()
}

#[inline]
pub fn hashitem_empty(hi: &hashitem_T) -> bool {
    hi.hi_di.is_null()
}

#[inline]
unsafe fn key_matches(hi_key: *const c_char, key: *const c_char, key_len: usize) -> bool {
    for i in 0..key_len {
        let a = unsafe { *hi_key.add(i) as u8 };
        let b = unsafe { *key.add(i) as u8 };
        if a != b || a == 0 {
            return false;
        }
    }
    unsafe { *hi_key.add(key_len) == 0 }
}

/// Free the array of a hash table without freeing contained values (`hash_clear`).
///
/// # Safety
/// `ht` must be a valid writable pointer to an initialized `hashtab_T`.
pub unsafe fn hash_clear(ht: *mut hashtab_T) {
    unsafe {
        *ht = hashtab_T::new();
    }
}

/// Find item for NUL-terminated `key` in `ht` (`hash_find`).
///
/// # Safety
/// `ht` and `key` must be valid pointers.
pub unsafe fn hash_find(ht: *const hashtab_T, key: *const c_char) -> *mut hashitem_T {
    let len = unsafe { CStr::from_ptr(key).to_bytes().len() };
    let hash = unsafe { hash_hash(key) };
    unsafe { hash_lookup(ht, key, len, hash) }
}

/// Find item for `key[..len]` in `ht` (`hash_find_len`).
///
/// # Safety
/// `ht` and `key` must be valid pointers.
pub unsafe fn hash_find_len(
    ht: *const hashtab_T,
    key: *const c_char,
    len: usize,
) -> *mut hashitem_T {
    let hash = unsafe { hash_hash_len(key, len) };
    unsafe { hash_lookup(ht, key, len, hash) }
}

/// Find item for `key[..key_len]` with precomputed `hash` in `ht` (`hash_lookup`).
///
/// # Safety
/// `ht` and `key` must be valid pointers.
pub unsafe fn hash_lookup(
    ht: *const hashtab_T,
    key: *const c_char,
    key_len: usize,
    hash: hash_T,
) -> *mut hashitem_T {
    let ht = unsafe { &*ht };
    let arr_ptr = ht.array_ptr();

    let mut idx = hash & ht.ht_mask;
    let mut hi = unsafe { arr_ptr.add(idx) };
    let hi_ref = unsafe { &*hi };

    if hi_ref.is_null_slot() {
        return hi;
    }

    let mut freeitem: *mut hashitem_T = std::ptr::null_mut();
    if hi_ref.hi_removed {
        freeitem = hi;
    } else if hi_ref.hi_hash == hash && unsafe { key_matches(hi_ref.key_ptr(), key, key_len) } {
        return hi;
    }

    let mut perturb = hash;
    loop {
        idx = idx.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1);
        hi = unsafe { arr_ptr.add(idx & ht.ht_mask) };
        let item = unsafe { &*hi };

        if item.is_null_slot() {
            return if freeitem.is_null() { hi } else { freeitem };
        }

        if item.hi_hash == hash
            && !item.hi_removed
            && unsafe { key_matches(item.key_ptr(), key, key_len) }
        {
            return hi;
        }

        if item.hi_removed && freeitem.is_null() {
            freeitem = hi;
        }

        perturb >>= PERTURB_SHIFT;
    }
}

/// Add dictionary item `di` to hashtable `ht` (`hash_add`).
///
/// # Safety
/// `ht` and `di` must be valid pointers, and `di` must outlive the entry in `ht`.
pub unsafe fn hash_add(ht: *mut hashtab_T, di: *mut crate::eval::typval::dictitem_T) -> c_int {
    let key = unsafe { (*di).di_key.as_mut_ptr() };
    let len = unsafe { CStr::from_ptr(key).to_bytes().len() };
    let hash = unsafe { hash_hash(key) };
    let hi = unsafe { hash_lookup(ht, key, len, hash) };
    if unsafe { !hashitem_empty(&*hi) } {
        unsafe {
            siemsg!(
                c"E685: Internal error: hash_add(): duplicate key \"%s\"".as_ptr(),
                key,
            );
        }
        return FAIL;
    }
    unsafe {
        hash_add_item(ht, hi, key, di, hash);
    }
    OK
}

/// Add item `hi` for `key` with precomputed `hash` to `ht` (`hash_add_item`).
///
/// # Safety
/// `ht`, `hi`, `key`, and `di` must be valid pointers, with `hi` obtained from `hash_lookup(ht, ...)`.
pub unsafe fn hash_add_item(
    ht: *mut hashtab_T,
    mut hi: *mut hashitem_T,
    _key: *mut c_char,
    di: *mut crate::eval::typval::dictitem_T,
    hash: hash_T,
) {
    let ht_ref = unsafe { &mut *ht };
    if ht_ref.ht_array.is_empty() {
        let offset = unsafe { hi.offset_from(empty_hasharray_ptr()) as usize };
        ht_ref.ht_array = vec![hashitem_T::default(); HT_INIT_SIZE];
        hi = unsafe { ht_ref.ht_array.as_mut_ptr().add(offset) };
    }
    let hi_ref = unsafe { &mut *hi };
    ht_ref.ht_used += 1;
    ht_ref.ht_changed += 1;
    if hi_ref.is_null_slot() {
        ht_ref.ht_filled += 1;
    }
    hi_ref.hi_removed = false;
    hi_ref.hi_di = di;
    hi_ref.hi_hash = hash;

    unsafe {
        hash_may_resize(ht_ref, 0);
    }
}

/// Remove item `hi` from `ht` (`hash_remove`).
///
/// # Safety
/// `ht` and `hi` must be valid pointers, with `hi` pointing to an active item in `ht`.
pub unsafe fn hash_remove(ht: *mut hashtab_T, hi: *mut hashitem_T) {
    let ht_ref = unsafe { &mut *ht };
    let hi_ref = unsafe { &mut *hi };
    ht_ref.ht_used -= 1;
    ht_ref.ht_changed += 1;
    hi_ref.hi_removed = true;
    hi_ref.hi_di = std::ptr::null_mut();
    unsafe {
        hash_may_resize(ht_ref, 0);
    }
}

/// Lock hashtable to prevent `ht_array` reallocation (`hash_lock`).
///
/// # Safety
/// `ht` must be a valid writable pointer to a `hashtab_T`.
pub unsafe fn hash_lock(ht: *mut hashtab_T) {
    unsafe {
        (*ht).ht_locked += 1;
    }
}

/// Unlock hashtable and resize if needed (`hash_unlock`).
///
/// # Safety
/// `ht` must be a valid writable pointer to a `hashtab_T`.
pub unsafe fn hash_unlock(ht: *mut hashtab_T) {
    let ht_ref = unsafe { &mut *ht };
    ht_ref.ht_locked -= 1;
    unsafe {
        hash_may_resize(ht_ref, 0);
    }
}

unsafe fn hash_may_resize(ht: &mut hashtab_T, mut minitems: usize) {
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

    let oldarray = std::mem::replace(&mut ht.ht_array, vec![hashitem_T::default(); newsize]);

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
///
/// # Safety
/// `key` must be a valid NUL-terminated C string.
pub unsafe fn hash_hash(key: *const c_char) -> hash_T {
    let first = unsafe { *key.cast::<u8>() };
    if first == 0 {
        return 0;
    }
    let mut hash = usize::from(first);
    let mut p = unsafe { key.cast::<u8>().add(1) };
    loop {
        let b = unsafe { *p };
        if b == 0 {
            break;
        }
        hash = hash.wrapping_mul(101).wrapping_add(usize::from(b));
        p = unsafe { p.add(1) };
    }
    hash
}

/// Compute hash for `key[..len]` (`hash_hash_len`).
///
/// # Safety
/// `key` must be valid for `len` bytes when `len > 0`.
pub unsafe fn hash_hash_len(key: *const c_char, len: usize) -> hash_T {
    if len == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(key.cast::<u8>(), len) };
    let mut hash = usize::from(slice[0]);
    for &b in &slice[1..] {
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
        let mut ht_val = hashtab_T::new();
        let keys: Vec<CString> = (0..50)
            .map(|i| CString::new(format!("key_{i}")).unwrap())
            .collect();

        unsafe {
            let ht: *mut hashtab_T = &raw mut ht_val;
            assert_eq!((*ht).ht_used, 0);

            let items: Vec<*mut crate::eval::typval::dictitem_T> = keys
                .iter()
                .map(|k| crate::eval::typval::tv_dict_item_alloc(k.as_ptr()))
                .collect();

            for &di in &items {
                assert_eq!(hash_add(ht, di), OK);
            }
            assert_eq!((*ht).ht_used, 50);
            // Duplicate key returns FAIL
            crate::globals::emsg_skip += 1;
            assert_eq!(hash_add(ht, items[0]), FAIL);
            crate::globals::emsg_skip -= 1;

            for (k, &di) in keys.iter().zip(&items) {
                let hi = hash_find(ht, k.as_ptr());
                assert!(!hashitem_empty(&*hi));
                assert_eq!(CStr::from_ptr((*hi).key_ptr()), k.as_c_str());
                assert_eq!((*hi).hi_di, di);
            }

            // Remove all keys and verify table shrinks back to HT_INIT_SIZE
            for k in &keys {
                let hi = hash_find(ht, k.as_ptr());
                hash_remove(ht, hi);
            }
            assert_eq!((*ht).ht_used, 0);
            assert_eq!((*ht).ht_mask, HT_INIT_SIZE - 1);
            hash_clear(ht);
            assert!(std::ptr::eq((*ht).array_ptr(), empty_hasharray_ptr()));

            for di in items {
                crate::eval::typval::tv_dict_item_free(di);
            }
        }
    }
}
