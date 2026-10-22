//! Functions for handling growing arrays (`src/nvim/garray.c`).

use crate::memory::{xfree, xmallocz, xrealloc};
use crate::path::path_equal;
use crate::strings::sort_strings;
use std::ffi::{CStr, c_char, c_int, c_void};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct garray_T {
    pub ga_len: c_int,
    pub ga_maxlen: c_int,
    pub ga_itemsize: c_int,
    pub ga_growsize: c_int,
    pub ga_data: *mut c_void,
}

pub(crate) const K_PATH_CMP_LITERAL: c_int = 1 << 0;

/// Clear an allocated growing array (`ga_clear`).
///
/// # Safety
/// `gap` must be a valid writable pointer to a `garray_T`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ga_clear(gap: *mut garray_T) {
    let gap = unsafe { &mut *gap };
    unsafe {
        xfree(gap.ga_data);
    }
    gap.ga_data = std::ptr::null_mut();
    gap.ga_maxlen = 0;
    gap.ga_len = 0;
}

/// Clear a growing array that contains a list of allocated pointers (`ga_clear_strings`).
///
/// # Safety
/// `gap` must be a valid writable pointer to a `garray_T` holding `*mut c_void` elements.
pub unsafe fn ga_clear_strings(gap: *mut garray_T) {
    let gap_ref = unsafe { &mut *gap };
    if !gap_ref.ga_data.is_null() {
        let items = gap_ref.ga_data.cast::<*mut c_void>();
        for i in 0..gap_ref.ga_len.max(0) as usize {
            unsafe {
                xfree(*items.add(i));
            }
        }
    }
    unsafe {
        ga_clear(gap);
    }
}

/// Initialize a growing array (`ga_init`).
///
/// # Safety
/// `gap` must be a valid writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ga_init(gap: *mut garray_T, itemsize: c_int, growsize: c_int) {
    let gap_ref = unsafe { &mut *gap };
    gap_ref.ga_data = std::ptr::null_mut();
    gap_ref.ga_maxlen = 0;
    gap_ref.ga_len = 0;
    gap_ref.ga_itemsize = itemsize;
    unsafe {
        ga_set_growsize(gap, growsize);
    }
}

/// Setter for `ga_growsize` that guarantees it is at least 1 (`ga_set_growsize`).
///
/// # Safety
/// `gap` must be a valid writable pointer.
pub unsafe fn ga_set_growsize(gap: *mut garray_T, growsize: c_int) {
    let gap = unsafe { &mut *gap };
    gap.ga_growsize = growsize.max(1);
}

/// Make room in growing array `gap` for at least `n` additional items (`ga_grow`).
///
/// # Safety
/// `gap` must be a valid writable pointer to an initialized `garray_T`.
pub unsafe fn ga_grow(gap: *mut garray_T, n: c_int) {
    let gap = unsafe { &mut *gap };
    if gap.ga_maxlen - gap.ga_len >= n {
        return;
    }

    let grow = n.max(gap.ga_growsize).max(gap.ga_len / 2);
    let new_maxlen = gap.ga_len + grow;

    let new_size = (gap.ga_itemsize as usize) * (new_maxlen as usize);
    let old_size = (gap.ga_itemsize as usize) * (gap.ga_maxlen as usize);

    unsafe {
        let pp = xrealloc(gap.ga_data, new_size).cast::<u8>();
        if new_size > old_size {
            std::ptr::write_bytes(pp.add(old_size), 0, new_size - old_size);
        }
        gap.ga_maxlen = new_maxlen;
        gap.ga_data = pp.cast::<c_void>();
    }
}

/// Sort `gap` and remove duplicate string entries (`ga_remove_duplicate_strings`).
///
/// # Safety
/// `gap` must be a valid writable pointer to a `garray_T` of allocated C strings.
pub unsafe fn ga_remove_duplicate_strings(gap: *mut garray_T) {
    let gap = unsafe { &mut *gap };
    let fnames = gap.ga_data.cast::<*mut c_char>();
    unsafe {
        sort_strings(fnames, gap.ga_len);
    }

    let mut i = gap.ga_len - 1;
    while i > 0 {
        let prev = unsafe { *fnames.add((i - 1) as usize) };
        let curr = unsafe { *fnames.add(i as usize) };
        if unsafe { path_equal(prev, curr, K_PATH_CMP_LITERAL) } {
            unsafe {
                xfree(curr.cast::<c_void>());
                for j in (i + 1)..gap.ga_len {
                    *fnames.add((j - 1) as usize) = *fnames.add(j as usize);
                }
            }
            gap.ga_len -= 1;
        }
        i -= 1;
    }
}

/// Concatenate all strings in `gap` separated by `sep` (`ga_concat_strings`).
///
/// # Safety
/// `gap` must be a valid pointer to a `garray_T` of valid C strings, and `sep` must be a valid C string.
pub unsafe fn ga_concat_strings(gap: *const garray_T, sep: *const c_char) -> *mut c_char {
    let gap = unsafe { &*gap };
    let nelem = gap.ga_len.max(0) as usize;
    let strings = gap.ga_data.cast::<*const c_char>();

    if nelem == 0 {
        return unsafe { xmallocz(0).cast::<c_char>() };
    }

    let sep_bytes = unsafe { CStr::from_ptr(sep).to_bytes() };
    let mut len = (nelem - 1) * sep_bytes.len();
    for i in 0..nelem {
        let s = unsafe { CStr::from_ptr(*strings.add(i)).to_bytes() };
        len += s.len();
    }

    let ret = unsafe { xmallocz(len).cast::<u8>() };
    let mut offset = 0usize;
    for i in 0..nelem {
        let s = unsafe { CStr::from_ptr(*strings.add(i)).to_bytes() };
        unsafe {
            if !s.is_empty() {
                std::ptr::copy_nonoverlapping(s.as_ptr(), ret.add(offset), s.len());
                offset += s.len();
            }
            if i + 1 < nelem && !sep_bytes.is_empty() {
                std::ptr::copy_nonoverlapping(sep_bytes.as_ptr(), ret.add(offset), sep_bytes.len());
                offset += sep_bytes.len();
            }
        }
    }
    unsafe {
        *ret.add(len) = 0;
    }
    ret.cast::<c_char>()
}

/// Concatenate C string `s` (without trailing NUL) to byte growarray `gap` (`ga_concat`).
///
/// # Safety
/// `gap` must be a valid writable pointer to a byte `garray_T`, and `s` must be null or a C string.
pub unsafe fn ga_concat(gap: *mut garray_T, s: *const c_char) {
    if s.is_null() {
        return;
    }
    let len = unsafe { CStr::from_ptr(s).to_bytes().len() };
    unsafe {
        ga_concat_len(gap, s, len);
    }
}

/// Concatenate `len` bytes from `s` to byte growarray `gap` (`ga_concat_len`).
///
/// # Safety
/// `gap` must be a valid writable pointer to a byte `garray_T`, and `s` must be valid for `len` bytes.
pub unsafe fn ga_concat_len(gap: *mut garray_T, s: *const c_char, len: usize) {
    if len == 0 {
        return;
    }
    unsafe {
        ga_grow(gap, len as c_int);
        let gap_ref = &mut *gap;
        let data = gap_ref.ga_data.cast::<u8>();
        std::ptr::copy_nonoverlapping(s.cast::<u8>(), data.add(gap_ref.ga_len as usize), len);
        gap_ref.ga_len += len as c_int;
    }
}

/// Append one byte `c` to byte growarray `gap` (`ga_append`).
///
/// # Safety
/// `gap` must be a valid writable pointer to a byte `garray_T`.
pub unsafe fn ga_append(gap: *mut garray_T, c: u8) {
    unsafe {
        ga_grow(gap, 1);
        let gap_ref = &mut *gap;
        *gap_ref.ga_data.cast::<u8>().add(gap_ref.ga_len as usize) = c;
        gap_ref.ga_len += 1;
    }
}

/// Grow `gap` by 1 item of `item_size` and return a pointer to the new slot (`ga_append_via_ptr`).
///
/// # Safety
/// `gap` must be a valid writable pointer to an initialized `garray_T`.
pub unsafe fn ga_append_via_ptr(gap: *mut garray_T, item_size: usize) -> *mut c_void {
    unsafe {
        ga_grow(gap, 1);
        let gap_ref = &mut *gap;
        let idx = gap_ref.ga_len as usize;
        gap_ref.ga_len += 1;
        gap_ref
            .ga_data
            .cast::<u8>()
            .add(item_size * idx)
            .cast::<c_void>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garray_byte_and_string_operations() {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: std::ptr::null_mut(),
        };

        unsafe {
            ga_init(&mut ga, 1, 4);
            assert_eq!(ga.ga_growsize, 4);
            ga_concat(&mut ga, c"hello".as_ptr());
            ga_append(&mut ga, b' ');
            ga_concat(&mut ga, c"world".as_ptr());
            ga_append(&mut ga, 0);

            let s = CStr::from_ptr(ga.ga_data.cast::<c_char>())
                .to_str()
                .unwrap();
            assert_eq!(s, "hello world");
            ga_clear(&mut ga);
            assert!(ga.ga_data.is_null());

            // Grow < growsize, > growsize, and 0
            ga_init(&mut ga, 16, 4);
            ga_grow(&mut ga, 0);
            assert!(ga.ga_data.is_null());
            assert_eq!(ga.ga_maxlen, 0);
            ga_grow(&mut ga, 3);
            assert_eq!(ga.ga_maxlen, 4);
            ga_clear(&mut ga);

            ga_init(&mut ga, 16, 4);
            ga_grow(&mut ga, 5);
            assert_eq!(ga.ga_maxlen, 5);
            ga_clear(&mut ga);

            // String array concat and duplicate removal
            ga_init(&mut ga, std::mem::size_of::<*mut c_char>() as c_int, 1);
            let empty_concat = ga_concat_strings(&ga, c",".as_ptr());
            assert_eq!(CStr::from_ptr(empty_concat).to_str().unwrap(), "");
            xfree(empty_concat.cast());

            for w in ["ccc", "aaa", "bbb", "aaa", "ccc"] {
                let slot = ga_append_via_ptr(&mut ga, std::mem::size_of::<*mut c_char>())
                    .cast::<*mut c_char>();
                let dup = xmallocz(w.len()).cast::<u8>();
                std::ptr::copy_nonoverlapping(w.as_ptr(), dup, w.len());
                *slot = dup.cast::<c_char>();
            }
            ga_remove_duplicate_strings(&mut ga);
            assert_eq!(ga.ga_len, 3);
            let joined = ga_concat_strings(&ga, c",".as_ptr());
            assert_eq!(CStr::from_ptr(joined).to_str().unwrap(), "aaa,bbb,ccc");
            xfree(joined.cast());
            ga_clear_strings(&mut ga);
        }
    }
}
