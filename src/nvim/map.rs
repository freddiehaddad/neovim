//! Hash maps and sets (`src/nvim/map.c`, `src/nvim/map_key_impl.c.h`, `src/nvim/map_value_impl.c.h`).

use crate::api::private::validate::NvimString;
use crate::memory::{xcalloc, xfree, xrealloc};
use std::ffi::{CStr, c_char, c_int, c_void};

const UPPER_FILL: f64 = 0.77;
pub const MH_TOMBSTONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MHPutStatus {
    kMHExisting = 0,
    kMHNewKeyDidFit = 1,
    kMHNewKeyRealloc = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MapHash {
    pub n_buckets: u32,
    pub size: u32,
    pub n_occupied: u32,
    pub upper_bound: u32,
    pub n_keys: u32,
    pub keys_capacity: u32,
    pub hash: *mut u32,
}

pub type cstr_t = *const c_char;
pub type ptr_t = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Set<K> {
    pub h: MapHash,
    pub keys: *mut K,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Map<K, V> {
    pub set: Set<K>,
    pub values: *mut V,
}

#[inline]
pub const fn empty_map_hash() -> MapHash {
    MapHash {
        n_buckets: 0,
        size: 0,
        n_occupied: 0,
        upper_bound: 0,
        n_keys: 0,
        keys_capacity: 0,
        hash: std::ptr::null_mut(),
    }
}

#[inline]
pub const fn empty_set<K>() -> Set<K> {
    Set {
        h: empty_map_hash(),
        keys: std::ptr::null_mut(),
    }
}

#[cfg(test)]
#[inline]
const fn empty_map<K, V>() -> Map<K, V> {
    Map {
        set: empty_set(),
        values: std::ptr::null_mut(),
    }
}

#[inline]
pub unsafe fn pmap_get_int(map: *mut Map<c_int, ptr_t>, key: c_int) -> ptr_t {
    unsafe {
        let ptr = map_ref(map, key, std::ptr::null_mut());
        if ptr.is_null() {
            std::ptr::null_mut()
        } else {
            *ptr
        }
    }
}

#[inline]
pub unsafe fn set_destroy<K>(set: *mut Set<K>) {
    unsafe {
        xfree((*set).keys.cast());
        xfree((*set).h.hash.cast());
        *set = empty_set();
    }
}

#[inline]
pub unsafe fn map_destroy<K, V>(map: *mut Map<K, V>) {
    unsafe {
        set_destroy(&raw mut (*map).set);
        xfree((*map).values.cast());
        (*map).values = std::ptr::null_mut();
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HlAttrs {
    pub rgb_ae_attr: i32,
    pub cterm_ae_attr: i32,
    pub rgb_fg_color: i32,
    pub rgb_bg_color: i32,
    pub rgb_sp_color: i32,
    pub cterm_fg_color: i16,
    pub cterm_bg_color: i16,
    pub hl_blend: i32,
    pub url: i32,
    pub font: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HlEntry {
    pub attr: HlAttrs,
    pub kind: c_int,
    pub id1: c_int,
    pub id2: c_int,
    pub winid: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorKey {
    pub ns_id: c_int,
    pub syn_id: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ColorItem {
    pub attr_id: c_int,
    pub link_id: c_int,
    pub version: c_int,
    pub is_default: bool,
    pub link_global: bool,
}

#[inline]
const fn hash_u64(key: u64) -> u32 {
    ((key >> 33) ^ key ^ (key << 11)) as u32
}

#[inline]
pub(crate) fn hash_bytes(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in bytes {
        h = (h << 5).wrapping_sub(h).wrapping_add(u32::from(b));
    }
    h
}

#[inline]
unsafe fn hash_cstr(s: *const c_char) -> u32 {
    let mut h: u32 = 0;
    let mut p = s.cast::<u8>();
    loop {
        let b = unsafe { *p };
        if b == 0 {
            break;
        }
        h = (h << 5).wrapping_sub(h).wrapping_add(u32::from(b));
        p = unsafe { p.add(1) };
    }
    h
}

#[inline]
unsafe fn equal_cstr(a: *const c_char, b: *const c_char) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == CStr::from_ptr(b).to_bytes() }
}

#[inline]
const fn roundup32(mut x: u32) -> u32 {
    x = x.wrapping_sub(1);
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x |= x >> 8;
    x |= x >> 16;
    x.wrapping_add(1)
}

/// Reallocate `h->hash` to at least `n_min_buckets` power-of-2 buckets (`mh_realloc`).
///
/// # Safety
/// `h` must be a valid writable pointer to a `MapHash`.
pub unsafe fn mh_realloc(h: *mut MapHash, n_min_buckets: u32) {
    let h = unsafe { &mut *h };
    unsafe {
        xfree(h.hash.cast::<c_void>());
    }
    let n_buckets = roundup32(n_min_buckets.max(16));
    h.hash = unsafe { xcalloc(n_buckets as usize, std::mem::size_of::<u32>()).cast::<u32>() };
    h.n_occupied = 0;
    h.size = 0;
    h.n_buckets = n_buckets;
    h.upper_bound = (f64::from(n_buckets) * UPPER_FILL + 0.5) as u32;
}

/// Clear all buckets in `h` without freeing `h->hash` (`mh_clear`).
///
/// # Safety
/// `h` must be a valid writable pointer to a `MapHash`.
pub unsafe fn mh_clear(h: *mut MapHash) {
    let h = unsafe { &mut *h };
    if !h.hash.is_null() {
        unsafe {
            std::ptr::write_bytes(h.hash, 0, h.n_buckets as usize);
        }
        h.size = 0;
        h.n_occupied = 0;
        h.n_keys = 0;
    }
}

#[inline]
pub unsafe fn mh_is_empty(h: &MapHash, i: u32) -> bool {
    unsafe { *h.hash.add(i as usize) == 0 }
}

#[inline]
pub unsafe fn mh_is_del(h: &MapHash, i: u32) -> bool {
    unsafe { *h.hash.add(i as usize) == MH_TOMBSTONE }
}

#[inline]
pub unsafe fn mh_is_either(h: &MapHash, i: u32) -> bool {
    let v = unsafe { *h.hash.add(i as usize) };
    v.wrapping_add(1) <= 1
}

pub trait MapKey: Copy {
    unsafe fn mh_hash(self) -> u32;
    unsafe fn mh_eq(self, other: Self) -> bool;
}

impl MapKey for c_int {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        self as u32
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        self == other
    }
}

impl MapKey for u32 {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        self
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        self == other
    }
}

impl MapKey for u64 {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        hash_u64(self)
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        self == other
    }
}

impl MapKey for ptr_t {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        #[cfg(target_pointer_width = "64")]
        {
            hash_u64(self as u64)
        }
        #[cfg(target_pointer_width = "32")]
        {
            self as u32
        }
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl MapKey for cstr_t {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        unsafe { hash_cstr(self) }
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        unsafe { equal_cstr(self, other) }
    }
}

impl MapKey for NvimString {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        if self.size == 0 || self.data.is_null() {
            0
        } else {
            let slice = unsafe { std::slice::from_raw_parts(self.data.cast::<u8>(), self.size) };
            hash_bytes(slice)
        }
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        if self.size != other.size {
            return false;
        }
        if self.size == 0 {
            return true;
        }
        let sa = unsafe { std::slice::from_raw_parts(self.data.cast::<u8>(), self.size) };
        let sb = unsafe { std::slice::from_raw_parts(other.data.cast::<u8>(), other.size) };
        sa == sb
    }
}

impl MapKey for HlEntry {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&raw const self).cast::<u8>(),
                std::mem::size_of::<HlEntry>(),
            )
        };
        hash_bytes(bytes)
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        self == other
    }
}

impl MapKey for ColorKey {
    #[inline]
    unsafe fn mh_hash(self) -> u32 {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&raw const self).cast::<u8>(),
                std::mem::size_of::<ColorKey>(),
            )
        };
        hash_bytes(bytes)
    }
    #[inline]
    unsafe fn mh_eq(self, other: Self) -> bool {
        self == other
    }
}

pub trait MapValue: Copy {
    const INIT: Self;
}

impl MapValue for c_int {
    const INIT: Self = 0;
}

impl MapValue for u32 {
    const INIT: Self = 0;
}

impl MapValue for i64 {
    const INIT: Self = 0;
}

impl MapValue for ptr_t {
    const INIT: Self = std::ptr::null_mut();
}

impl MapValue for ColorItem {
    const INIT: Self = ColorItem {
        attr_id: -1,
        link_id: -1,
        version: -1,
        is_default: false,
        link_global: false,
    };
}

unsafe fn set_find_bucket<K: MapKey>(set: &mut Set<K>, key: K, put: bool) -> u32 {
    let h = &mut set.h;
    let mut step: u32 = 0;
    let mask = h.n_buckets - 1;
    let k = unsafe { key.mh_hash() };
    let mut i = k & mask;
    let last = i;
    let mut site = if put { last } else { MH_TOMBSTONE };

    while unsafe { !mh_is_empty(h, i) } {
        if unsafe { mh_is_del(h, i) } {
            if site == last {
                site = i;
            }
        } else {
            let existing = unsafe { *set.keys.add((*h.hash.add(i as usize) - 1) as usize) };
            if unsafe { existing.mh_eq(key) } {
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

unsafe fn set_rehash<K: MapKey>(set: &mut Set<K>) {
    for k in 0..set.h.n_keys {
        let key = unsafe { *set.keys.add(k as usize) };
        let idx = unsafe { set_find_bucket(set, key, true) };
        if unsafe { !mh_is_empty(&set.h, idx) } {
            std::process::abort();
        }
        unsafe {
            *set.h.hash.add(idx as usize) = k + 1;
        }
    }
    set.h.n_occupied = set.h.n_keys;
    set.h.size = set.h.n_keys;
}

pub unsafe fn mh_get<K: MapKey>(set: *mut Set<K>, key: K) -> u32 {
    let set = unsafe { &mut *set };
    if set.h.n_buckets == 0 {
        return MH_TOMBSTONE;
    }
    let idx = unsafe { set_find_bucket(set, key, false) };
    if idx != MH_TOMBSTONE {
        unsafe { *set.h.hash.add(idx as usize) - 1 }
    } else {
        MH_TOMBSTONE
    }
}

pub unsafe fn mh_put<K: MapKey>(set: *mut Set<K>, key: K, new_status: *mut MHPutStatus) -> u32 {
    let set = unsafe { &mut *set };
    if set.h.n_occupied >= set.h.upper_bound {
        if f64::from(set.h.size) >= f64::from(set.h.upper_bound) * 0.9 {
            let next_min = set.h.n_buckets + 1;
            unsafe {
                mh_realloc(&mut set.h, next_min);
            }
        } else {
            unsafe {
                std::ptr::write_bytes(set.h.hash, 0, set.h.n_buckets as usize);
            }
            set.h.size = 0;
            set.h.n_occupied = 0;
        }
        unsafe {
            set_rehash(set);
        }
    }

    let idx = unsafe { set_find_bucket(set, key, true) };
    let h = &mut set.h;

    if unsafe { mh_is_either(h, idx) } {
        h.size += 1;
        if unsafe { mh_is_empty(h, idx) } {
            h.n_occupied += 1;
        }

        let pos = h.n_keys;
        h.n_keys += 1;
        if pos >= h.keys_capacity {
            h.keys_capacity = (h.keys_capacity * 2).max(8);
            set.keys = unsafe {
                xrealloc(
                    set.keys.cast::<c_void>(),
                    (h.keys_capacity as usize) * std::mem::size_of::<K>(),
                )
                .cast::<K>()
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
            *set.keys.add(pos as usize) = key;
            *h.hash.add(idx as usize) = pos + 1;
        }
        pos
    } else {
        unsafe {
            *new_status = MHPutStatus::kMHExisting;
            let pos = *h.hash.add(idx as usize) - 1;
            if !(*set.keys.add(pos as usize)).mh_eq(key) {
                std::process::abort();
            }
            pos
        }
    }
}

pub unsafe fn map_ref<K: MapKey, V>(map: *mut Map<K, V>, key: K, key_alloc: *mut *mut K) -> *mut V {
    let map = unsafe { &mut *map };
    let k = unsafe { mh_get(&mut map.set, key) };
    if k == MH_TOMBSTONE {
        return std::ptr::null_mut();
    }
    if !key_alloc.is_null() {
        unsafe {
            *key_alloc = map.set.keys.add(k as usize);
        }
    }
    unsafe { map.values.add(k as usize) }
}

pub unsafe fn map_put_ref<K: MapKey, V: MapValue>(
    map: *mut Map<K, V>,
    key: K,
    key_alloc: *mut *mut K,
    new_item: *mut bool,
) -> *mut V {
    let map = unsafe { &mut *map };
    let mut status = MHPutStatus::kMHExisting;
    let k = unsafe { mh_put(&mut map.set, key, &mut status) };
    if status != MHPutStatus::kMHExisting {
        if status == MHPutStatus::kMHNewKeyRealloc {
            map.values = unsafe {
                xrealloc(
                    map.values.cast::<c_void>(),
                    (map.set.h.keys_capacity as usize) * std::mem::size_of::<V>(),
                )
                .cast::<V>()
            };
        }
        unsafe {
            *map.values.add(k as usize) = V::INIT;
        }
    }
    if !new_item.is_null() {
        unsafe {
            *new_item = status != MHPutStatus::kMHExisting;
        }
    }
    if !key_alloc.is_null() {
        unsafe {
            *key_alloc = map.set.keys.add(k as usize);
        }
    }
    unsafe { map.values.add(k as usize) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_and_cstr_maps_put_get_delete() {
        let mut m: Map<u32, u32> = empty_map();

        unsafe {
            for i in 0..30 {
                let mut is_new = false;
                let vp = map_put_ref(&raw mut m, i as u32, std::ptr::null_mut(), &mut is_new);
                assert!(is_new);
                *vp = (i * 10) as u32;
            }
            for i in 0..30 {
                let vp = map_ref(&raw mut m, i as u32, std::ptr::null_mut());
                assert!(!vp.is_null());
                assert_eq!(*vp, (i * 10) as u32);
            }

            map_destroy(&raw mut m);
        }
    }
}
