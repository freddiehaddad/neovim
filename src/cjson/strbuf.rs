//! String buffer routines (`src/cjson/strbuf.c`).

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::memory::{xfree, xmalloc, xrealloc};

pub const STRBUF_DEFAULT_SIZE: usize = 1023;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct strbuf_t {
    pub buf: *mut c_char,
    pub size: usize,
    pub length: usize,
}

impl strbuf_t {
    pub const fn new() -> Self {
        Self {
            buf: ptr::null_mut(),
            size: 0,
            length: 0,
        }
    }
}

impl Default for strbuf_t {
    fn default() -> Self {
        Self::new()
    }
}

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
pub unsafe fn strbuf_set_length(s: *mut strbuf_t, len: c_int) {
    unsafe {
        (*s).length = len as usize;
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

pub unsafe fn strbuf_init(s: *mut strbuf_t, len: usize) {
    let size = if len == 0 {
        STRBUF_DEFAULT_SIZE
    } else {
        match len.checked_add(1) {
            Some(sz) => sz,
            None => die(&format!("Overflow, len: {len}")),
        }
    };

    unsafe {
        (*s).size = size;
        (*s).length = 0;
        (*s).buf = xmalloc(size).cast::<c_char>();
        strbuf_ensure_null(s);
    }
}

pub unsafe fn strbuf_free(s: *mut strbuf_t) {
    unsafe {
        if !(*s).buf.is_null() {
            xfree((*s).buf.cast::<c_void>());
            (*s).buf = ptr::null_mut();
        }
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

pub unsafe fn strbuf_resize(s: *mut strbuf_t, len: usize) {
    unsafe {
        let newsize = calculate_new_size(s, len);
        (*s).size = newsize;
        (*s).buf = xrealloc((*s).buf.cast::<c_void>(), (*s).size).cast::<c_char>();
    }
}

pub unsafe fn strbuf_append_string(s: *mut strbuf_t, str_in: *const c_char) {
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
