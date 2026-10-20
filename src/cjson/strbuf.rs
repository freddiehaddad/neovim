//! String buffer routines (`src/cjson/strbuf.c`).

use std::ffi::{c_char, c_int, c_void};
use std::mem::size_of;
use std::ptr;

pub const STRBUF_DEFAULT_SIZE: usize = 1023;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct strbuf_t {
    pub buf: *mut c_char,
    pub size: usize,
    pub length: usize,
    pub dynamic: c_int,
    pub reallocs: c_int,
    pub debug: c_int,
}

use crate::memory::{crt_free as free, crt_malloc as malloc, crt_realloc as realloc};

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::abort()
}

#[inline]
pub unsafe fn strbuf_reset(s: *mut strbuf_t) {
    unsafe {
        (*s).length = 0;
    }
}

#[inline]
pub unsafe fn strbuf_allocated(s: *mut strbuf_t) -> c_int {
    c_int::from(unsafe { !(*s).buf.is_null() })
}

#[inline]
pub unsafe fn strbuf_empty_length(s: *mut strbuf_t) -> usize {
    unsafe { (*s).size - (*s).length - 1 }
}

#[inline]
pub unsafe fn strbuf_ensure_empty_length(s: *mut strbuf_t, len: usize) {
    unsafe {
        if len > strbuf_empty_length(s) {
            strbuf_resize(s, (*s).length + len);
        }
    }
}

#[inline]
pub unsafe fn strbuf_empty_ptr(s: *mut strbuf_t) -> *mut c_char {
    unsafe { (*s).buf.add((*s).length) }
}

#[inline]
pub unsafe fn strbuf_set_length(s: *mut strbuf_t, len: c_int) {
    unsafe {
        (*s).length = len as usize;
    }
}

#[inline]
pub unsafe fn strbuf_extend_length(s: *mut strbuf_t, len: usize) {
    unsafe {
        (*s).length += len;
    }
}

#[inline]
pub unsafe fn strbuf_length(s: *mut strbuf_t) -> usize {
    unsafe { (*s).length }
}

#[inline]
pub unsafe fn strbuf_append_char(s: *mut strbuf_t, c: c_char) {
    unsafe {
        strbuf_ensure_empty_length(s, 1);
        *(*s).buf.add((*s).length) = c;
        (*s).length += 1;
    }
}

#[inline]
pub unsafe fn strbuf_append_char_unsafe(s: *mut strbuf_t, c: c_char) {
    unsafe {
        *(*s).buf.add((*s).length) = c;
        (*s).length += 1;
    }
}

#[inline]
pub unsafe fn strbuf_append_mem(s: *mut strbuf_t, c: *const c_char, len: usize) {
    unsafe {
        strbuf_ensure_empty_length(s, len);
        ptr::copy_nonoverlapping(c, (*s).buf.add((*s).length), len);
        (*s).length += len;
    }
}

#[inline]
pub unsafe fn strbuf_append_mem_unsafe(s: *mut strbuf_t, c: *const c_char, len: usize) {
    unsafe {
        ptr::copy_nonoverlapping(c, (*s).buf.add((*s).length), len);
        (*s).length += len;
    }
}

#[inline]
pub unsafe fn strbuf_ensure_null(s: *mut strbuf_t) {
    unsafe {
        *(*s).buf.add((*s).length) = 0;
    }
}

#[inline]
pub unsafe fn strbuf_string(s: *mut strbuf_t, len: *mut usize) -> *mut c_char {
    unsafe {
        if !len.is_null() {
            *len = (*s).length;
        }
        (*s).buf
    }
}

#[inline]
unsafe fn debug_stats(s: *mut strbuf_t) {
    unsafe {
        if (*s).debug != 0 {
            eprintln!(
                "strbuf({:x}) reallocs: {}, length: {}, size: {}",
                s as usize,
                (*s).reallocs,
                (*s).length,
                (*s).size
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strbuf_init(s: *mut strbuf_t, len: usize) {
    let size = if len == 0 {
        STRBUF_DEFAULT_SIZE
    } else {
        match len.checked_add(1) {
            Some(sz) => sz,
            None => die(&format!("Overflow, len: {len}")),
        }
    };

    unsafe {
        (*s).buf = ptr::null_mut();
        (*s).size = size;
        (*s).length = 0;
        (*s).dynamic = 0;
        (*s).reallocs = 0;
        (*s).debug = 0;

        (*s).buf = malloc(size).cast::<c_char>();
        if (*s).buf.is_null() {
            die("Out of memory");
        }

        strbuf_ensure_null(s);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strbuf_new(len: usize) -> *mut strbuf_t {
    unsafe {
        let s = malloc(size_of::<strbuf_t>()).cast::<strbuf_t>();
        if s.is_null() {
            die("Out of memory");
        }

        strbuf_init(s, len);
        (*s).dynamic = 1;
        s
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strbuf_free(s: *mut strbuf_t) {
    unsafe {
        debug_stats(s);

        if !(*s).buf.is_null() {
            free((*s).buf.cast::<c_void>());
            (*s).buf = ptr::null_mut();
        }
        if (*s).dynamic != 0 {
            free(s.cast::<c_void>());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strbuf_free_to_string(s: *mut strbuf_t, len: *mut usize) -> *mut c_char {
    unsafe {
        debug_stats(s);
        strbuf_ensure_null(s);

        let buf = (*s).buf;
        if !len.is_null() {
            *len = (*s).length;
        }

        if (*s).dynamic != 0 {
            free(s.cast::<c_void>());
        }

        buf
    }
}

unsafe fn calculate_new_size(s: *mut strbuf_t, len: usize) -> usize {
    if len == 0 {
        die("BUG: Invalid strbuf length requested");
    }

    let reqsize = match len.checked_add(1) {
        Some(sz) => sz,
        None => die(&format!("Overflow, len: {len}")),
    };

    unsafe {
        if (*s).size > reqsize {
            return reqsize;
        }

        let mut newsize = (*s).size;
        if reqsize >= usize::MAX / 2 {
            newsize = reqsize;
        } else {
            while newsize < reqsize {
                newsize = newsize.wrapping_mul(2);
            }
        }

        if newsize < reqsize {
            die(&format!("BUG: strbuf length would overflow, len: {len}"));
        }

        newsize
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strbuf_resize(s: *mut strbuf_t, len: usize) {
    unsafe {
        let newsize = calculate_new_size(s, len);

        if (*s).debug > 1 {
            eprintln!(
                "strbuf({:x}) resize: {} => {}",
                s as usize,
                (*s).size,
                newsize
            );
        }

        (*s).size = newsize;
        (*s).buf = realloc((*s).buf.cast::<c_void>(), (*s).size).cast::<c_char>();
        if (*s).buf.is_null() {
            die(&format!("Out of memory, len: {len}"));
        }
        (*s).reallocs += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strbuf_append_string(s: *mut strbuf_t, str_in: *const c_char) {
    unsafe {
        let mut space = strbuf_empty_length(s);
        let mut i = 0usize;
        while *str_in.add(i) != 0 {
            if space < 1 {
                strbuf_resize(s, (*s).length + 1);
                space = strbuf_empty_length(s);
            }
            *(*s).buf.add((*s).length) = *str_in.add(i);
            (*s).length += 1;
            space -= 1;
            i += 1;
        }
    }
}
