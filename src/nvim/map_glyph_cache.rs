//! Specialized `Set(glyph)` where interned strings are stored in a compact,
//! NUL-separated `char` array (`src/nvim/map_glyph_cache.c`).

use std::ffi::{CStr, c_char, c_void};

use crate::memory::xrealloc;

pub const MH_TOMBSTONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NvimString {
    pub data: *mut c_char,
    pub size: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum MHPutStatus {
    kMHExisting = 0,
    kMHNewKeyDidFit = 1,
    kMHNewKeyRealloc = 2,
}

#[repr(C)]
#[derive(Debug)]
pub struct MapHash {
    pub n_buckets: u32,
    pub size: u32,
    pub n_occupied: u32,
    pub upper_bound: u32,
    pub n_keys: u32,
    pub keys_capacity: u32,
    pub hash: *mut u32,
}

#[repr(C)]
#[derive(Debug)]
#[allow(non_camel_case_types)]
pub struct Set_glyph {
    pub h: MapHash,
    pub keys: *mut c_char,
}

#[cfg(not(test))]
unsafe extern "C" {
    fn mh_realloc(h: *mut MapHash, n_min_buckets: u32);
}

#[cfg(test)]
unsafe fn mh_realloc(h: *mut MapHash, n_min_buckets: u32) {
    use crate::memory::{xcalloc, xfree};
    let h = unsafe { &mut *h };
    unsafe {
        xfree(h.hash.cast::<c_void>());
    }
    let n_buckets = n_min_buckets.max(16).next_power_of_two();
    h.hash = unsafe { xcalloc(n_buckets as usize, std::mem::size_of::<u32>()).cast::<u32>() };
    h.n_occupied = 0;
    h.size = 0;
    h.n_buckets = n_buckets;
    h.upper_bound = (f64::from(n_buckets) * 0.77 + 0.5) as u32;
}

#[inline]
unsafe fn mh_is_empty(h: &MapHash, i: u32) -> bool {
    unsafe { *h.hash.add(i as usize) == 0 }
}

#[inline]
unsafe fn mh_is_del(h: &MapHash, i: u32) -> bool {
    unsafe { *h.hash.add(i as usize) == MH_TOMBSTONE }
}

#[inline]
unsafe fn mh_is_either(h: &MapHash, i: u32) -> bool {
    let v = unsafe { *h.hash.add(i as usize) };
    v.wrapping_add(1) <= 1
}

#[inline]
unsafe fn hash_string(s: NvimString) -> u32 {
    let mut h: u32 = 0;
    if s.size > 0 && !s.data.is_null() {
        let bytes = unsafe { std::slice::from_raw_parts(s.data.cast::<u8>(), s.size) };
        for &b in bytes {
            h = (h << 5).wrapping_sub(h).wrapping_add(u32::from(b));
        }
    }
    h
}

#[inline]
unsafe fn cstr_as_string(str_ptr: *const c_char) -> NvimString {
    if str_ptr.is_null() {
        NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        }
    } else {
        let len = unsafe { CStr::from_ptr(str_ptr).to_bytes().len() };
        NvimString {
            data: str_ptr.cast_mut(),
            size: len,
        }
    }
}

#[inline]
unsafe fn equal_string(a: NvimString, b: NvimString) -> bool {
    if a.size != b.size {
        return false;
    }
    if a.size == 0 {
        return true;
    }
    let sa = unsafe { std::slice::from_raw_parts(a.data.cast::<u8>(), a.size) };
    let sb = unsafe { std::slice::from_raw_parts(b.data.cast::<u8>(), b.size) };
    sa == sb
}

/// Find the bucket for `key` in `set` (`mh_find_bucket_glyph`).
///
/// # Safety
/// `set` must be a valid pointer to an initialized `Set_glyph` with `n_buckets > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mh_find_bucket_glyph(
    set: *mut Set_glyph,
    key: NvimString,
    put: bool,
) -> u32 {
    let set = unsafe { &mut *set };
    let h = &mut set.h;
    let mut step: u32 = 0;
    let mask = h.n_buckets - 1;
    let k = unsafe { hash_string(key) };
    let mut i = k & mask;
    let last = i;
    let mut site = if put { last } else { MH_TOMBSTONE };

    while unsafe { !mh_is_empty(h, i) } {
        if unsafe { mh_is_del(h, i) } {
            if site == last {
                site = i;
            }
        } else {
            let key_offset = unsafe { *h.hash.add(i as usize) - 1 } as usize;
            let existing = unsafe { cstr_as_string(set.keys.add(key_offset)) };
            if unsafe { equal_string(existing, key) } {
                return i;
            }
        }
        step = step.wrapping_add(1);
        i = i.wrapping_add(step) & mask;
        if i == last {
            std::process::abort();
        }
    }
    if site == last {
        site = i;
    }
    site
}

/// Return index into `set->keys` if found, `MH_TOMBSTONE` otherwise (`mh_get_glyph`).
///
/// # Safety
/// `set` must be a valid pointer to a `Set_glyph`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mh_get_glyph(set: *mut Set_glyph, key: NvimString) -> u32 {
    let set_ref = unsafe { &mut *set };
    if set_ref.h.n_buckets == 0 {
        return MH_TOMBSTONE;
    }
    let idx = unsafe { mh_find_bucket_glyph(set, key, false) };
    if idx != MH_TOMBSTONE {
        unsafe { *set_ref.h.hash.add(idx as usize) - 1 }
    } else {
        MH_TOMBSTONE
    }
}

/// Rehash all NUL-separated keys in `set->keys` (`mh_rehash_glyph`).
///
/// # Safety
/// `set` must be a valid pointer to a `Set_glyph`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mh_rehash_glyph(set: *mut Set_glyph) {
    let set_ref = unsafe { &mut *set };
    let mut k: u32 = 0;
    while k < set_ref.h.n_keys {
        let key_ptr = unsafe { set_ref.keys.add(k as usize) };
        let key_str = unsafe { cstr_as_string(key_ptr) };
        let idx = unsafe { mh_find_bucket_glyph(set, key_str, true) };
        if unsafe { !mh_is_empty(&set_ref.h, idx) } {
            std::process::abort();
        }
        unsafe {
            *set_ref.h.hash.add(idx as usize) = k + 1;
        }
        k += (key_str.size as u32) + 1;
    }
    set_ref.h.n_occupied = set_ref.h.n_keys;
    set_ref.h.size = set_ref.h.n_keys;
}

/// Intern `key` into `set`, returning its byte offset in `set->keys` (`mh_put_glyph`).
///
/// # Safety
/// `set` and `new_status` must be valid writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mh_put_glyph(
    set: *mut Set_glyph,
    key: NvimString,
    new_status: *mut MHPutStatus,
) -> u32 {
    let set_ref = unsafe { &mut *set };
    if set_ref.h.n_occupied >= set_ref.h.upper_bound {
        let next_min = set_ref.h.n_buckets + 1;
        unsafe {
            mh_realloc(&mut set_ref.h, next_min);
            mh_rehash_glyph(set);
        }
    }

    let idx = unsafe { mh_find_bucket_glyph(set, key, true) };
    let h = &mut set_ref.h;

    if unsafe { mh_is_either(h, idx) } {
        h.size += 1;
        h.n_occupied += 1;

        let size = (key.size as u32) + 1;
        let pos = h.n_keys;
        h.n_keys += size;
        if h.n_keys > h.keys_capacity {
            while h.n_keys > h.keys_capacity {
                h.keys_capacity = (h.keys_capacity * 2).max(64);
            }
            set_ref.keys = unsafe {
                xrealloc(set_ref.keys.cast::<c_void>(), h.keys_capacity as usize).cast::<c_char>()
            };
            unsafe {
                *new_status = MHPutStatus::kMHNewKeyRealloc;
            }
        } else {
            unsafe {
                *new_status = MHPutStatus::kMHNewKeyDidFit;
            }
        }

        unsafe {
            if key.size > 0 && !key.data.is_null() {
                std::ptr::copy_nonoverlapping(
                    key.data.cast::<u8>(),
                    set_ref.keys.add(pos as usize).cast::<u8>(),
                    key.size,
                );
            }
            *set_ref.keys.add((pos as usize) + key.size) = 0;
            *h.hash.add(idx as usize) = pos + 1;
        }
        pos
    } else {
        unsafe {
            *new_status = MHPutStatus::kMHExisting;
            let pos = *h.hash.add(idx as usize) - 1;
            debug_assert!(equal_string(
                cstr_as_string(set_ref.keys.add(pos as usize)),
                key
            ));
            pos
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::xfree;

    #[test]
    fn put_get_and_rehash_glyphs() {
        let mut set = Set_glyph {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: std::ptr::null_mut(),
            },
            keys: std::ptr::null_mut(),
        };

        unsafe {
            for i in 0..40 {
                let s = format!("glyph_{i}");
                let ns = NvimString {
                    data: s.as_ptr().cast::<c_char>().cast_mut(),
                    size: s.len(),
                };
                let mut status = MHPutStatus::kMHExisting;
                let pos1 = mh_put_glyph(&mut set, ns, &mut status);
                assert_ne!(status, MHPutStatus::kMHExisting);

                let pos2 = mh_put_glyph(&mut set, ns, &mut status);
                assert_eq!(status, MHPutStatus::kMHExisting);
                assert_eq!(pos1, pos2);
                assert_eq!(mh_get_glyph(&mut set, ns), pos1);
            }

            xfree(set.h.hash.cast::<c_void>());
            xfree(set.keys.cast::<c_void>());
        }
    }
}
