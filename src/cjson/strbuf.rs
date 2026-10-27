//! String buffer routines (`src/cjson/strbuf.c`).

use std::ffi::{CStr, c_char, c_int};

pub const STRBUF_DEFAULT_SIZE: usize = 1023;

#[repr(C)]
pub struct strbuf_t {
    pub buf: Vec<c_char>,
}

impl strbuf_t {
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }
}

impl Default for strbuf_t {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn strbuf_reset(s: *mut strbuf_t) {
    unsafe {
        (*s).buf.clear();
    }
}

#[inline]
pub fn strbuf_ensure_empty_length(s: *mut strbuf_t, len: usize) {
    unsafe {
        (*s).buf.reserve(len + 1);
    }
}

#[inline]
pub fn strbuf_set_length(s: *mut strbuf_t, len: c_int) {
    unsafe {
        (*s).buf.truncate(len as usize);
    }
}

#[inline]
pub fn strbuf_length(s: *mut strbuf_t) -> usize {
    unsafe { (*s).buf.len() }
}

#[inline]
pub fn strbuf_append_char(s: *mut strbuf_t, c: c_char) {
    unsafe {
        (*s).buf.push(c);
    }
}

#[inline]
pub fn strbuf_append_char_unsafe(s: *mut strbuf_t, c: c_char) {
    unsafe {
        (*s).buf.push(c);
    }
}

#[inline]
pub fn strbuf_append_mem(s: *mut strbuf_t, c: *const c_char, len: usize) {
    if len > 0 {
        unsafe {
            (*s).buf
                .extend_from_slice(std::slice::from_raw_parts(c, len));
        }
    }
}

#[inline]
pub fn strbuf_append_mem_unsafe(s: *mut strbuf_t, c: *const c_char, len: usize) {
    if len > 0 {
        unsafe {
            (*s).buf
                .extend_from_slice(std::slice::from_raw_parts(c, len));
        }
    }
}

#[inline]
pub fn strbuf_ensure_null(s: *mut strbuf_t) {
    unsafe {
        (*s).buf.reserve(1);
        let len = (*s).buf.len();
        *(*s).buf.as_mut_ptr().add(len) = 0;
    }
}

#[inline]
pub fn strbuf_string(s: *mut strbuf_t, len: *mut usize) -> *mut c_char {
    unsafe {
        if !len.is_null() {
            *len = (*s).buf.len();
        }
        (*s).buf.as_mut_ptr()
    }
}

pub fn strbuf_init(s: *mut strbuf_t, len: usize) {
    let size = if len == 0 {
        STRBUF_DEFAULT_SIZE
    } else {
        len.saturating_add(1)
    };

    unsafe {
        (*s).buf = Vec::with_capacity(size);
        strbuf_ensure_null(s);
    }
}

pub fn strbuf_append_string(s: *mut strbuf_t, str_in: *const c_char) {
    unsafe {
        let bytes = CStr::from_ptr(str_in).to_bytes();
        strbuf_append_mem(s, bytes.as_ptr().cast::<c_char>(), bytes.len());
    }
}
