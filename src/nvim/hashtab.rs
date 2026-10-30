//! Handling of a hashtable with Vim-specific properties (`src/nvim/hashtab.c`).

use crate::message::siemsg;
use std::ffi::CStr;

pub type HashVal = usize;

pub const HT_INIT_SIZE: usize = 16;
const PERTURB_SHIFT: usize = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Hashitem {
    pub hi_hash: HashVal,
    pub hi_removed: bool,
    pub hi_di: *mut crate::eval::typval::Dictitem,
}

impl Hashitem {
    #[inline]
    pub const fn is_null_slot(&self) -> bool {
        self.hi_di.is_null() && !self.hi_removed
    }

    #[inline]
    pub fn key_cstr(&self) -> Option<&CStr> {
        if self.hi_di.is_null() {
            None
        } else {
            unsafe { CStr::from_bytes_until_nul(&(*self.hi_di).di_key).ok() }
        }
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
    pub fn array_ptr(&self) -> *mut Hashitem {
        if self.ht_array.is_empty() {
            empty_hasharray_ptr()
        } else {
            self.ht_array.as_ptr().cast_mut()
        }
    }
}

impl Default for Hashtab {
    fn default() -> Self {
        Self::new()
    }
}

static EMPTY_HASHARRAY: crate::globals::GlobalState<[Hashitem; HT_INIT_SIZE]> =
    crate::globals::GlobalState::new(
        [Hashitem {
            hi_hash: 0,
            hi_removed: false,
            hi_di: std::ptr::null_mut(),
        }; HT_INIT_SIZE],
    );

#[inline]
pub const fn empty_hasharray_ptr() -> *mut Hashitem {
    EMPTY_HASHARRAY.as_ptr().cast::<Hashitem>()
}

#[inline]
pub fn hashitem_empty(hi: &Hashitem) -> bool {
    hi.hi_di.is_null()
}

#[inline]
fn key_matches(
    hi_key: impl crate::strings::IntoCCharPtr,
    key: impl crate::strings::IntoCCharPtr,
    key_len: usize,
) -> bool {
    let hi_key = hi_key.into_c_char_ptr();
    let key = key.into_c_char_ptr();
    for i in 0..key_len {
        let a = crate::memory::cstr_byte_at(hi_key, i);
        let b = crate::memory::cstr_byte_at(key, i);
        if a != b || a == 0 {
            return false;
        }
    }
    unsafe { *hi_key.wrapping_add(key_len) == 0 }
}

/// Free the array of a hash table without freeing contained values (`hash_clear`).
///
/// # Safety
/// `ht` must be a valid writable pointer to an initialized `Hashtab`.
pub fn hash_clear(ht: &mut Hashtab) {
    *ht = Hashtab::new();
}

/// Find item for NUL-terminated `key` in `ht` (`hash_find`).
pub fn hash_find(ht: &Hashtab, key: impl crate::strings::IntoCCharPtr) -> *mut Hashitem {
    let key = key.into_c_char_ptr();
    let len = crate::memory::cstr_len(key);
    let hash = hash_hash(key);
    hash_lookup(ht, key, len, hash)
}

/// Find item for `key[..len]` in `ht` (`hash_find_len`).
pub fn hash_find_len(
    ht: &Hashtab,
    key: impl crate::strings::IntoCCharPtr,
    len: usize,
) -> *mut Hashitem {
    let key = key.into_c_char_ptr();
    let hash = hash_hash_len(key, len);
    hash_lookup(ht, key, len, hash)
}

/// Find item for `key[..key_len]` with precomputed `hash` in `ht` (`hash_lookup`).
pub fn hash_lookup(
    ht: &Hashtab,
    key: impl crate::strings::IntoCCharPtr,
    key_len: usize,
    hash: HashVal,
) -> *mut Hashitem {
    let key = key.into_c_char_ptr();
    let arr_ptr = ht.array_ptr();

    let mut idx = hash & ht.ht_mask;
    let mut hi = arr_ptr.wrapping_add(idx);
    let hi_ref = unsafe { &*hi };

    if hi_ref.is_null_slot() {
        return hi;
    }

    let mut freeitem: *mut Hashitem = std::ptr::null_mut();
    if hi_ref.hi_removed {
        freeitem = hi;
    } else if hi_ref.hi_hash == hash && key_matches(hi_ref.key_cstr(), key, key_len) {
        return hi;
    }

    let mut perturb = hash;
    loop {
        idx = idx.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1);
        hi = arr_ptr.wrapping_add(idx & ht.ht_mask);
        let item = unsafe { &*hi };

        if item.is_null_slot() {
            return if freeitem.is_null() { hi } else { freeitem };
        }

        if item.hi_hash == hash && !item.hi_removed && key_matches(item.key_cstr(), key, key_len) {
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
pub fn hash_add(ht: &mut Hashtab, di: &mut crate::eval::typval::Dictitem) -> bool {
    let key: crate::strings::RawCCharPtr = di.di_key.as_ptr().cast();
    let len = crate::memory::cstr_len(key);
    let hash = hash_hash(key);
    let hi = hash_lookup(ht, key, len, hash);
    if unsafe { !hashitem_empty(&*hi) } {
        siemsg!(
            c"E685: Internal error: hash_add(): duplicate key \"%s\"",
            key,
        );
        return false;
    }
    hash_add_item(ht, hi, &mut *di, hash);
    true
}

/// Add item `hi` with precomputed `hash` to `ht` (`hash_add_item`).
///
/// # Safety
/// `ht`, `hi`, and `di` must be valid pointers, with `hi` obtained from `hash_lookup(ht, ...)`.
pub fn hash_add_item(
    ht: &mut Hashtab,
    mut hi: *mut Hashitem,
    di: &mut crate::eval::typval::Dictitem,
    hash: HashVal,
) {
    if ht.ht_array.is_empty() {
        let offset = crate::memory::ptr_offset_from(hi, empty_hasharray_ptr()) as usize;
        ht.ht_array = vec![Hashitem::default(); HT_INIT_SIZE];
        hi = ht.ht_array.as_mut_ptr().wrapping_add(offset);
    }
    let hi_ref = unsafe { &mut *hi };
    ht.ht_used += 1;
    ht.ht_changed += 1;
    if hi_ref.is_null_slot() {
        ht.ht_filled += 1;
    }
    hi_ref.hi_removed = false;
    hi_ref.hi_di = di;
    hi_ref.hi_hash = hash;

    hash_may_resize(ht, 0);
}

/// Remove item `hi` from `ht` (`hash_remove`).
///
/// # Safety
/// `ht` and `hi` must be valid pointers, with `hi` pointing to an active item in `ht`.
pub fn hash_remove(ht: &mut Hashtab, hi: &mut Hashitem) {
    let hi_ref = hi;
    ht.ht_used -= 1;
    ht.ht_changed += 1;
    hi_ref.hi_removed = true;
    hi_ref.hi_di = std::ptr::null_mut();
    hash_may_resize(ht, 0);
}

/// Lock hashtable to prevent `ht_array` reallocation (`hash_lock`).
///
/// # Safety
/// `ht` must be a valid writable pointer to a `Hashtab`.
pub fn hash_lock(ht: &mut Hashtab) {
    ht.ht_locked += 1;
}

/// Unlock hashtable and resize if needed (`hash_unlock`).
///
/// # Safety
/// `ht` must be a valid writable pointer to a `Hashtab`.
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
///
/// # Safety
/// `key` must be a valid NUL-terminated C string.
pub fn hash_hash(key: impl crate::strings::IntoCCharPtr) -> HashVal {
    let key = key.into_c_char_ptr();
    let first = unsafe { *key.cast::<u8>() };
    if first == 0 {
        return 0;
    }
    let mut hash = usize::from(first);
    let mut p = key.cast::<u8>().wrapping_add(1);
    loop {
        let b = unsafe { *p };
        if b == 0 {
            break;
        }
        hash = hash.wrapping_mul(101).wrapping_add(usize::from(b));
        p = p.wrapping_add(1);
    }
    hash
}

/// Compute hash for `key[..len]` (`hash_hash_len`).
///
/// # Safety
/// `key` must be valid for `len` bytes when `len > 0`.
pub fn hash_hash_len(key: impl crate::strings::IntoCCharPtr, len: usize) -> HashVal {
    let key = key.into_c_char_ptr();
    if len == 0 {
        return 0;
    }
    let slice = crate::memory::cstr_slice(key, len);
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
        let mut ht_val = Hashtab::new();
        let keys: Vec<CString> = (0..50)
            .map(|i| CString::new(format!("key_{i}")).unwrap())
            .collect();

        unsafe {
            let ht = &mut ht_val;
            assert_eq!(ht.ht_used, 0);

            let mut items: Vec<*mut crate::eval::typval::Dictitem> = keys
                .iter()
                .map(|k| &raw mut *crate::eval::typval::tv_dict_item_alloc(k.as_ptr()))
                .collect();

            for &di in &items {
                assert!(hash_add(ht, &mut *di));
            }
            assert_eq!(ht.ht_used, 50);
            // Duplicate key returns FAIL
            crate::globals::g_st().emsg_skip += 1;
            assert!(!hash_add(ht, &mut *items[0]));
            crate::globals::g_st().emsg_skip -= 1;

            for (k, &di) in keys.iter().zip(&items) {
                let hi = hash_find(ht, k.as_ptr());
                assert!(!hashitem_empty(&*hi));
                assert_eq!((*hi).key_cstr(), Some(k.as_c_str()));
                assert_eq!((*hi).hi_di, di);
            }

            // Remove all keys and verify table shrinks back to HT_INIT_SIZE
            for k in &keys {
                let hi = hash_find(ht, k.as_ptr());
                hash_remove(ht, &mut *hi);
            }
            assert_eq!(ht.ht_used, 0);
            assert_eq!(ht.ht_mask, HT_INIT_SIZE - 1);
            hash_clear(ht);
            assert!(std::ptr::eq(ht.array_ptr(), empty_hasharray_ptr()));

            for di in items {
                crate::eval::typval::tv_dict_item_free(di);
            }
        }
    }
}
