//! Hash maps and sets (`src/nvim/map.c`, `src/nvim/map_key_impl.c.h`, `src/nvim/map_value_impl.c.h`).

use std::ffi::{CStr, c_char, c_int, c_void};

use crate::map_glyph_cache::{MH_TOMBSTONE, MHPutStatus, MapHash, NvimString};
use crate::memory::{xcalloc, xfree, xrealloc};

const UPPER_FILL: f64 = 0.77;
const MAXPATHL: usize = 4096;

#[allow(non_camel_case_types)]
pub type cstr_t = *const c_char;
#[allow(non_camel_case_types)]
pub type path_t = *const c_char;
#[allow(non_camel_case_types)]
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

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MTDamage {
    pub old: *mut c_void,
    pub new: *mut c_void,
    pub old_i: c_int,
    pub new_i: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MTDamagePair {
    pub start: MTDamage,
    pub end: MTDamage,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StcClick {
    pub def: *mut c_void,
    pub size: usize,
}

pub type StcClicks = Map<c_int, StcClick>;

#[cfg(not(test))]
unsafe extern "C" {
    fn vim_ispathsep(c: c_int) -> bool;
    fn vim_ispathsep_nocolon(c: c_int) -> bool;
    fn path_fold_char(ic: bool, p: *const c_char, len: *mut c_int) -> c_int;
    fn path_cmp(ic: bool, p: *const c_char, q: *const c_char, maxlen: usize) -> c_int;
}

#[cfg(test)]
unsafe fn vim_ispathsep(c: c_int) -> bool {
    c == b'/' as c_int || c == b'\\' as c_int
}

#[cfg(test)]
unsafe fn vim_ispathsep_nocolon(c: c_int) -> bool {
    unsafe { vim_ispathsep(c) }
}

#[cfg(test)]
unsafe fn path_fold_char(ic: bool, p: *const c_char, len: *mut c_int) -> c_int {
    unsafe {
        *len = 1;
        let b = *p as u8;
        if ic {
            c_int::from(b.to_ascii_lowercase())
        } else {
            c_int::from(b)
        }
    }
}

#[cfg(test)]
unsafe fn path_cmp(ic: bool, p: *const c_char, q: *const c_char, _maxlen: usize) -> c_int {
    let bp = unsafe { CStr::from_ptr(p).to_bytes() };
    let bq = unsafe { CStr::from_ptr(q).to_bytes() };
    if ic {
        bp.to_ascii_lowercase().cmp(&bq.to_ascii_lowercase()) as c_int
    } else {
        bp.cmp(bq) as c_int
    }
}

#[inline]
const fn hash_u64(key: u64) -> u32 {
    ((key >> 33) ^ key ^ (key << 11)) as u32
}

#[inline]
unsafe fn hash_bytes(bytes: &[u8]) -> u32 {
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

const CASE_INSENSITIVE_FILENAME: bool = cfg!(any(target_os = "windows", target_os = "macos"));

unsafe fn hash_path(mut p: *const c_char) -> u32 {
    let mut h: u32 = 0;
    if cfg!(target_os = "windows") {
        let c0 = unsafe { *p as u8 };
        if c0.is_ascii_alphabetic() && unsafe { *p.add(1) as u8 } == b':' {
            p = unsafe { p.add(2) };
        }
    }
    let ic = CASE_INSENSITIVE_FILENAME;
    let start = p;
    let mut len: c_int = 0;
    while unsafe { *p != 0 } {
        let cur = unsafe { *p };
        let next = unsafe { *p.add(1) };
        if unsafe { vim_ispathsep_nocolon(c_int::from(cur as u8)) }
            && next == 0
            && !std::ptr::eq(start, p)
            && unsafe { !vim_ispathsep(c_int::from(*p.sub(1) as u8)) }
        {
            break;
        }
        let c = unsafe { path_fold_char(ic, p, &mut len) };
        h = (h << 5).wrapping_sub(h).wrapping_add(c as u32);
        p = unsafe { p.add(len as usize) };
    }
    h
}

unsafe fn equal_path(a: *const c_char, b: *const c_char) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }
    unsafe { path_cmp(CASE_INSENSITIVE_FILENAME, a, b, MAXPATHL) == 0 }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mh_realloc(h: *mut MapHash, n_min_buckets: u32) {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mh_clear(h: *mut MapHash) {
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

unsafe fn set_find_bucket<K: Copy>(
    set: &mut Set<K>,
    key: K,
    put: bool,
    hash_fn: unsafe fn(K) -> u32,
    eq_fn: unsafe fn(K, K) -> bool,
) -> u32 {
    let h = &mut set.h;
    let mut step: u32 = 0;
    let mask = h.n_buckets - 1;
    let k = unsafe { hash_fn(key) };
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
            if unsafe { eq_fn(existing, key) } {
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

unsafe fn set_get<K: Copy>(
    set: &mut Set<K>,
    key: K,
    hash_fn: unsafe fn(K) -> u32,
    eq_fn: unsafe fn(K, K) -> bool,
) -> u32 {
    if set.h.n_buckets == 0 {
        return MH_TOMBSTONE;
    }
    let idx = unsafe { set_find_bucket(set, key, false, hash_fn, eq_fn) };
    if idx != MH_TOMBSTONE {
        unsafe { *set.h.hash.add(idx as usize) - 1 }
    } else {
        MH_TOMBSTONE
    }
}

unsafe fn set_rehash<K: Copy>(
    set: &mut Set<K>,
    hash_fn: unsafe fn(K) -> u32,
    eq_fn: unsafe fn(K, K) -> bool,
) {
    for k in 0..set.h.n_keys {
        let key = unsafe { *set.keys.add(k as usize) };
        let idx = unsafe { set_find_bucket(set, key, true, hash_fn, eq_fn) };
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

unsafe fn set_put<K: Copy>(
    set: &mut Set<K>,
    key: K,
    new_status: *mut MHPutStatus,
    hash_fn: unsafe fn(K) -> u32,
    eq_fn: unsafe fn(K, K) -> bool,
) -> u32 {
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
            set_rehash(set, hash_fn, eq_fn);
        }
    }

    let idx = unsafe { set_find_bucket(set, key, true, hash_fn, eq_fn) };
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
            if !eq_fn(*set.keys.add(pos as usize), key) {
                std::process::abort();
            }
            pos
        }
    }
}

unsafe fn set_delete<K: Copy>(
    set: &mut Set<K>,
    key: *mut K,
    hash_fn: unsafe fn(K) -> u32,
    eq_fn: unsafe fn(K, K) -> bool,
) -> u32 {
    if set.h.size == 0 {
        return MH_TOMBSTONE;
    }
    let query = unsafe { *key };
    let idx = unsafe { set_find_bucket(set, query, false, hash_fn, eq_fn) };
    if idx != MH_TOMBSTONE {
        let k = unsafe { *set.h.hash.add(idx as usize) - 1 };
        unsafe {
            *set.h.hash.add(idx as usize) = MH_TOMBSTONE;
        }

        set.h.n_keys -= 1;
        let last = set.h.n_keys;
        unsafe {
            *key = *set.keys.add(k as usize);
        }
        set.h.size -= 1;
        if last != k {
            let last_key = unsafe { *set.keys.add(last as usize) };
            let idx2 = unsafe { set_find_bucket(set, last_key, false, hash_fn, eq_fn) };
            if unsafe { *set.h.hash.add(idx2 as usize) } != last + 1 {
                std::process::abort();
            }
            unsafe {
                *set.h.hash.add(idx2 as usize) = k + 1;
                *set.keys.add(k as usize) = last_key;
            }
        }
        return k;
    }
    MH_TOMBSTONE
}

macro_rules! impl_c_set {
    ($K:ty, $find_fn:ident, $get_fn:ident, $rehash_fn:ident, $put_fn:ident, $del_fn:ident, $hash_fn:expr, $eq_fn:expr) => {
        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $find_fn(set: *mut Set<$K>, key: $K, put: bool) -> u32 {
            unsafe { set_find_bucket(&mut *set, key, put, $hash_fn, $eq_fn) }
        }

        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $get_fn(set: *mut Set<$K>, key: $K) -> u32 {
            unsafe { set_get(&mut *set, key, $hash_fn, $eq_fn) }
        }

        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $rehash_fn(set: *mut Set<$K>) {
            unsafe { set_rehash(&mut *set, $hash_fn, $eq_fn) }
        }

        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $put_fn(
            set: *mut Set<$K>,
            key: $K,
            new_status: *mut MHPutStatus,
        ) -> u32 {
            unsafe { set_put(&mut *set, key, new_status, $hash_fn, $eq_fn) }
        }

        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $del_fn(set: *mut Set<$K>, key: *mut $K) -> u32 {
            unsafe { set_delete(&mut *set, key, $hash_fn, $eq_fn) }
        }
    };
}

macro_rules! impl_c_map {
    ($K:ty, $V:ty, $get_set_fn:ident, $put_set_fn:ident, $del_set_fn:ident, $ref_fn:ident, $put_ref_fn:ident, $del_fn:ident, $init_val:expr) => {
        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $ref_fn(
            map: *mut Map<$K, $V>,
            key: $K,
            key_alloc: *mut *mut $K,
        ) -> *mut $V {
            let map = unsafe { &mut *map };
            let k = unsafe { $get_set_fn(&mut map.set, key) };
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

        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $put_ref_fn(
            map: *mut Map<$K, $V>,
            key: $K,
            key_alloc: *mut *mut $K,
            new_item: *mut bool,
        ) -> *mut $V {
            let map = unsafe { &mut *map };
            let mut status = MHPutStatus::kMHExisting;
            let k = unsafe { $put_set_fn(&mut map.set, key, &mut status) };
            if status != MHPutStatus::kMHExisting {
                if status == MHPutStatus::kMHNewKeyRealloc {
                    map.values = unsafe {
                        xrealloc(
                            map.values.cast::<c_void>(),
                            (map.set.h.keys_capacity as usize) * std::mem::size_of::<$V>(),
                        )
                        .cast::<$V>()
                    };
                }
                unsafe {
                    *map.values.add(k as usize) = $init_val;
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

        #[unsafe(no_mangle)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn $del_fn(
            map: *mut Map<$K, $V>,
            mut key: $K,
            key_alloc: *mut $K,
        ) -> $V {
            let map = unsafe { &mut *map };
            let mut rv: $V = $init_val;
            let k = unsafe { $del_set_fn(&mut map.set, &mut key) };
            if k == MH_TOMBSTONE {
                return rv;
            }
            if !key_alloc.is_null() {
                unsafe {
                    *key_alloc = key;
                }
            }
            unsafe {
                rv = *map.values.add(k as usize);
                if k != map.set.h.n_keys {
                    *map.values.add(k as usize) = *map.values.add(map.set.h.n_keys as usize);
                }
            }
            rv
        }
    };
}

unsafe fn hash_int_fn(x: c_int) -> u32 {
    x as u32
}
unsafe fn eq_int_fn(a: c_int, b: c_int) -> bool {
    a == b
}

unsafe fn hash_u32_fn(x: u32) -> u32 {
    x
}
unsafe fn eq_u32_fn(a: u32, b: u32) -> bool {
    a == b
}

unsafe fn hash_u64_fn(x: u64) -> u32 {
    hash_u64(x)
}
unsafe fn eq_u64_fn(a: u64, b: u64) -> bool {
    a == b
}

unsafe fn hash_i64_fn(x: i64) -> u32 {
    hash_u64(x as u64)
}
unsafe fn eq_i64_fn(a: i64, b: i64) -> bool {
    a == b
}

unsafe fn hash_ptr_fn(x: ptr_t) -> u32 {
    #[cfg(target_pointer_width = "64")]
    {
        hash_u64(x as u64)
    }
    #[cfg(target_pointer_width = "32")]
    {
        x as u32
    }
}
unsafe fn eq_ptr_fn(a: ptr_t, b: ptr_t) -> bool {
    std::ptr::eq(a, b)
}

unsafe fn hash_string_fn(s: NvimString) -> u32 {
    if s.size == 0 || s.data.is_null() {
        0
    } else {
        let slice = unsafe { std::slice::from_raw_parts(s.data.cast::<u8>(), s.size) };
        unsafe { hash_bytes(slice) }
    }
}
unsafe fn eq_string_fn(a: NvimString, b: NvimString) -> bool {
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

unsafe fn hash_hl_entry_fn(ae: HlEntry) -> u32 {
    let bytes = unsafe {
        std::slice::from_raw_parts((&raw const ae).cast::<u8>(), std::mem::size_of::<HlEntry>())
    };
    unsafe { hash_bytes(bytes) }
}
unsafe fn eq_hl_entry_fn(a: HlEntry, b: HlEntry) -> bool {
    a == b
}

unsafe fn hash_color_key_fn(ae: ColorKey) -> u32 {
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&raw const ae).cast::<u8>(),
            std::mem::size_of::<ColorKey>(),
        )
    };
    unsafe { hash_bytes(bytes) }
}
unsafe fn eq_color_key_fn(a: ColorKey, b: ColorKey) -> bool {
    a == b
}

impl_c_set!(
    c_int,
    mh_find_bucket_int,
    mh_get_int,
    mh_rehash_int,
    mh_put_int,
    mh_delete_int,
    hash_int_fn,
    eq_int_fn
);
impl_c_set!(
    ptr_t,
    mh_find_bucket_ptr_t,
    mh_get_ptr_t,
    mh_rehash_ptr_t,
    mh_put_ptr_t,
    mh_delete_ptr_t,
    hash_ptr_fn,
    eq_ptr_fn
);
impl_c_set!(
    cstr_t,
    mh_find_bucket_cstr_t,
    mh_get_cstr_t,
    mh_rehash_cstr_t,
    mh_put_cstr_t,
    mh_delete_cstr_t,
    hash_cstr,
    equal_cstr
);
impl_c_set!(
    path_t,
    mh_find_bucket_path_t,
    mh_get_path_t,
    mh_rehash_path_t,
    mh_put_path_t,
    mh_delete_path_t,
    hash_path,
    equal_path
);
impl_c_set!(
    NvimString,
    mh_find_bucket_String,
    mh_get_String,
    mh_rehash_String,
    mh_put_String,
    mh_delete_String,
    hash_string_fn,
    eq_string_fn
);
impl_c_set!(
    u32,
    mh_find_bucket_uint32_t,
    mh_get_uint32_t,
    mh_rehash_uint32_t,
    mh_put_uint32_t,
    mh_delete_uint32_t,
    hash_u32_fn,
    eq_u32_fn
);
impl_c_set!(
    u64,
    mh_find_bucket_uint64_t,
    mh_get_uint64_t,
    mh_rehash_uint64_t,
    mh_put_uint64_t,
    mh_delete_uint64_t,
    hash_u64_fn,
    eq_u64_fn
);
impl_c_set!(
    i64,
    mh_find_bucket_int64_t,
    mh_get_int64_t,
    mh_rehash_int64_t,
    mh_put_int64_t,
    mh_delete_int64_t,
    hash_i64_fn,
    eq_i64_fn
);
impl_c_set!(
    HlEntry,
    mh_find_bucket_HlEntry,
    mh_get_HlEntry,
    mh_rehash_HlEntry,
    mh_put_HlEntry,
    mh_delete_HlEntry,
    hash_hl_entry_fn,
    eq_hl_entry_fn
);
impl_c_set!(
    ColorKey,
    mh_find_bucket_ColorKey,
    mh_get_ColorKey,
    mh_rehash_ColorKey,
    mh_put_ColorKey,
    mh_delete_ColorKey,
    hash_color_key_fn,
    eq_color_key_fn
);

const INIT_STRING: NvimString = NvimString {
    data: std::ptr::null_mut(),
    size: 0,
};
const INIT_STC_CLICK: StcClick = StcClick {
    def: std::ptr::null_mut(),
    size: 0,
};
const INIT_STC_CLICKS: StcClicks = Map {
    set: Set {
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
    },
    values: std::ptr::null_mut(),
};
const INIT_MT_DAMAGE: MTDamage = MTDamage {
    old: std::ptr::null_mut(),
    new: std::ptr::null_mut(),
    old_i: 0,
    new_i: 0,
};
const INIT_MT_DAMAGE_PAIR: MTDamagePair = MTDamagePair {
    start: INIT_MT_DAMAGE,
    end: INIT_MT_DAMAGE,
};
const INIT_COLOR_ITEM: ColorItem = ColorItem {
    attr_id: -1,
    link_id: -1,
    version: -1,
    is_default: false,
    link_global: false,
};

impl_c_map!(
    c_int,
    ptr_t,
    mh_get_int,
    mh_put_int,
    mh_delete_int,
    map_ref_int_ptr_t,
    map_put_ref_int_ptr_t,
    map_del_int_ptr_t,
    std::ptr::null_mut()
);
impl_c_map!(
    c_int,
    NvimString,
    mh_get_int,
    mh_put_int,
    mh_delete_int,
    map_ref_int_String,
    map_put_ref_int_String,
    map_del_int_String,
    INIT_STRING
);
impl_c_map!(
    c_int,
    StcClick,
    mh_get_int,
    mh_put_int,
    mh_delete_int,
    map_ref_int_StcClick,
    map_put_ref_int_StcClick,
    map_del_int_StcClick,
    INIT_STC_CLICK
);
impl_c_map!(
    c_int,
    StcClicks,
    mh_get_int,
    mh_put_int,
    mh_delete_int,
    map_ref_int_StcClicks,
    map_put_ref_int_StcClicks,
    map_del_int_StcClicks,
    INIT_STC_CLICKS
);
impl_c_map!(
    ptr_t,
    ptr_t,
    mh_get_ptr_t,
    mh_put_ptr_t,
    mh_delete_ptr_t,
    map_ref_ptr_t_ptr_t,
    map_put_ref_ptr_t_ptr_t,
    map_del_ptr_t_ptr_t,
    std::ptr::null_mut()
);
impl_c_map!(
    cstr_t,
    ptr_t,
    mh_get_cstr_t,
    mh_put_cstr_t,
    mh_delete_cstr_t,
    map_ref_cstr_t_ptr_t,
    map_put_ref_cstr_t_ptr_t,
    map_del_cstr_t_ptr_t,
    std::ptr::null_mut()
);
impl_c_map!(
    cstr_t,
    c_int,
    mh_get_cstr_t,
    mh_put_cstr_t,
    mh_delete_cstr_t,
    map_ref_cstr_t_int,
    map_put_ref_cstr_t_int,
    map_del_cstr_t_int,
    0
);
impl_c_map!(
    NvimString,
    c_int,
    mh_get_String,
    mh_put_String,
    mh_delete_String,
    map_ref_String_int,
    map_put_ref_String_int,
    map_del_String_int,
    0
);
impl_c_map!(
    u32,
    ptr_t,
    mh_get_uint32_t,
    mh_put_uint32_t,
    mh_delete_uint32_t,
    map_ref_uint32_t_ptr_t,
    map_put_ref_uint32_t_ptr_t,
    map_del_uint32_t_ptr_t,
    std::ptr::null_mut()
);
impl_c_map!(
    u32,
    u32,
    mh_get_uint32_t,
    mh_put_uint32_t,
    mh_delete_uint32_t,
    map_ref_uint32_t_uint32_t,
    map_put_ref_uint32_t_uint32_t,
    map_del_uint32_t_uint32_t,
    0
);
impl_c_map!(
    u64,
    ptr_t,
    mh_get_uint64_t,
    mh_put_uint64_t,
    mh_delete_uint64_t,
    map_ref_uint64_t_ptr_t,
    map_put_ref_uint64_t_ptr_t,
    map_del_uint64_t_ptr_t,
    std::ptr::null_mut()
);
impl_c_map!(
    u64,
    c_int,
    mh_get_uint64_t,
    mh_put_uint64_t,
    mh_delete_uint64_t,
    map_ref_uint64_t_int,
    map_put_ref_uint64_t_int,
    map_del_uint64_t_int,
    0
);
impl_c_map!(
    u64,
    MTDamagePair,
    mh_get_uint64_t,
    mh_put_uint64_t,
    mh_delete_uint64_t,
    map_ref_uint64_t_MTDamagePair,
    map_put_ref_uint64_t_MTDamagePair,
    map_del_uint64_t_MTDamagePair,
    INIT_MT_DAMAGE_PAIR
);
impl_c_map!(
    i64,
    ptr_t,
    mh_get_int64_t,
    mh_put_int64_t,
    mh_delete_int64_t,
    map_ref_int64_t_ptr_t,
    map_put_ref_int64_t_ptr_t,
    map_del_int64_t_ptr_t,
    std::ptr::null_mut()
);
impl_c_map!(
    i64,
    i64,
    mh_get_int64_t,
    mh_put_int64_t,
    mh_delete_int64_t,
    map_ref_int64_t_int64_t,
    map_put_ref_int64_t_int64_t,
    map_del_int64_t_int64_t,
    0
);
impl_c_map!(
    ColorKey,
    ColorItem,
    mh_get_ColorKey,
    mh_put_ColorKey,
    mh_delete_ColorKey,
    map_ref_ColorKey_ColorItem,
    map_put_ref_ColorKey_ColorItem,
    map_del_ColorKey_ColorItem,
    INIT_COLOR_ITEM
);

/// Delete a key:value pair from a `PMap(cstr_t)` and free both key and value (`pmap_del2`).
///
/// # Safety
/// `map` and `key` must be valid pointers, and the stored key and value must be `xfree`-able.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_del2(map: *mut Map<cstr_t, ptr_t>, key: *const c_char) {
    let mut key_alloc: cstr_t = std::ptr::null();
    let val = unsafe { map_del_cstr_t_ptr_t(map, key, &mut key_alloc) };
    unsafe {
        xfree(key_alloc.cast_mut().cast::<c_void>());
        xfree(val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_and_cstr_maps_put_get_delete() {
        let mut m: Map<c_int, u32> = Map {
            set: Set {
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
            },
            values: std::ptr::null_mut(),
        };

        unsafe {
            for i in 0..30 {
                let mut is_new = false;
                let vp = map_put_ref_uint32_t_uint32_t(
                    (&raw mut m).cast::<Map<u32, u32>>(),
                    i as u32,
                    std::ptr::null_mut(),
                    &mut is_new,
                );
                assert!(is_new);
                *vp = (i * 10) as u32;
            }
            for i in 0..30 {
                let vp = map_ref_uint32_t_uint32_t(
                    (&raw mut m).cast::<Map<u32, u32>>(),
                    i as u32,
                    std::ptr::null_mut(),
                );
                assert!(!vp.is_null());
                assert_eq!(*vp, (i * 10) as u32);
            }
            for i in 0..30 {
                let old = map_del_uint32_t_uint32_t(
                    (&raw mut m).cast::<Map<u32, u32>>(),
                    i as u32,
                    std::ptr::null_mut(),
                );
                assert_eq!(old, (i * 10) as u32);
            }
            assert_eq!(m.set.h.size, 0);

            xfree(m.set.keys.cast::<c_void>());
            xfree(m.set.h.hash.cast::<c_void>());
            xfree(m.values.cast::<c_void>());
        }
    }
}
