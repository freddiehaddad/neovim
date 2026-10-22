//! Specialized `Set(glyph)` where interned strings are stored in a compact,
//! NUL-separated `char` array (`src/nvim/map_glyph_cache.c`).

use crate::api::private::helpers::cstr_as_string;
use crate::api::private::validate::NvimString;
use crate::map::{
    MH_TOMBSTONE, MHPutStatus, MapHash, hash_bytes, mh_is_del, mh_is_either, mh_is_empty,
    mh_realloc,
};
use crate::memory::xrealloc;
use std::ffi::{c_char, c_void};

#[repr(C)]
#[derive(Debug)]
pub struct Set_glyph {
    pub h: MapHash,
    pub keys: *mut c_char,
}

#[inline]
unsafe fn hash_string(s: NvimString) -> u32 {
    if s.size > 0 && !s.data.is_null() {
        let bytes = unsafe { std::slice::from_raw_parts(s.data.cast::<u8>(), s.size) };
        hash_bytes(bytes)
    } else {
        0
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
pub unsafe fn mh_find_bucket_glyph(set: &mut Set_glyph, key: NvimString, put: bool) -> u32 {
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
#[cfg(test)]
pub unsafe fn mh_get_glyph(set: &mut Set_glyph, key: NvimString) -> u32 {
    if set.h.n_buckets == 0 {
        return MH_TOMBSTONE;
    }
    let idx = unsafe { mh_find_bucket_glyph(set, key, false) };
    if idx != MH_TOMBSTONE {
        unsafe { *set.h.hash.add(idx as usize) - 1 }
    } else {
        MH_TOMBSTONE
    }
}

/// Rehash all NUL-separated keys in `set->keys` (`mh_rehash_glyph`).
pub unsafe fn mh_rehash_glyph(set: &mut Set_glyph) {
    let mut k: u32 = 0;
    while k < set.h.n_keys {
        let key_ptr = unsafe { set.keys.add(k as usize) };
        let key_str = unsafe { cstr_as_string(key_ptr) };
        let idx = unsafe { mh_find_bucket_glyph(set, key_str, true) };
        if unsafe { !mh_is_empty(&set.h, idx) } {
            std::process::abort();
        }
        unsafe {
            *set.h.hash.add(idx as usize) = k + 1;
        }
        k += (key_str.size as u32) + 1;
    }
    set.h.n_occupied = set.h.n_keys;
    set.h.size = set.h.n_keys;
}

/// Intern `key` into `set`, returning its byte offset in `set->keys` (`mh_put_glyph`).
pub unsafe fn mh_put_glyph(
    set: &mut Set_glyph,
    key: NvimString,
    new_status: &mut MHPutStatus,
) -> u32 {
    if set.h.n_occupied >= set.h.upper_bound {
        let next_min = set.h.n_buckets + 1;
        unsafe {
            mh_realloc(&mut set.h, next_min);
            mh_rehash_glyph(set);
        }
    }

    let idx = unsafe { mh_find_bucket_glyph(set, key, true) };
    let h = &mut set.h;

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
            set.keys = unsafe {
                xrealloc(set.keys.cast::<c_void>(), h.keys_capacity as usize).cast::<c_char>()
            };
            *new_status = MHPutStatus::kMHNewKeyRealloc;
        } else {
            *new_status = MHPutStatus::kMHNewKeyDidFit;
        }

        unsafe {
            if key.size > 0 && !key.data.is_null() {
                std::ptr::copy_nonoverlapping(
                    key.data.cast::<u8>(),
                    set.keys.add(pos as usize).cast::<u8>(),
                    key.size,
                );
            }
            *set.keys.add((pos as usize) + key.size) = 0;
            *h.hash.add(idx as usize) = pos + 1;
        }
        pos
    } else {
        *new_status = MHPutStatus::kMHExisting;
        unsafe {
            let pos = *h.hash.add(idx as usize) - 1;
            debug_assert!(equal_string(
                cstr_as_string(set.keys.add(pos as usize)),
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
