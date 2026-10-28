//! String buffer routines (`src/cjson/strbuf.c`).

use std::ffi::{c_char, c_int};

pub const STRBUF_DEFAULT_SIZE: usize = 1023;

#[repr(C)]
pub struct Strbuf {
    pub buf: Vec<c_char>,
}

impl Strbuf {
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }
}

impl Default for Strbuf {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn strbuf_reset(s: &mut Strbuf) {
    s.buf.clear();
}

#[inline]
pub fn strbuf_ensure_empty_length(s: &mut Strbuf, len: usize) {
    s.buf.reserve(len + 1);
}

#[inline]
pub fn strbuf_set_length(s: &mut Strbuf, len: c_int) {
    s.buf.truncate(len as usize);
}

#[inline]
pub fn strbuf_length(s: &Strbuf) -> usize {
    s.buf.len()
}

#[inline]
pub fn strbuf_append_char(s: &mut Strbuf, c: c_char) {
    s.buf.push(c);
}

#[inline]
pub fn strbuf_append_char_unsafe(s: &mut Strbuf, c: c_char) {
    s.buf.push(c);
}

#[inline]
pub fn strbuf_append_mem(s: &mut Strbuf, c: impl crate::strings::IntoCCharPtr, len: usize) {
    let c = c.into_c_char_ptr();
    if len > 0 {
        unsafe {
            s.buf.extend_from_slice(std::slice::from_raw_parts(c, len));
        }
    }
}

#[inline]
pub fn strbuf_append_mem_unsafe(s: &mut Strbuf, c: impl crate::strings::IntoCCharPtr, len: usize) {
    let c = c.into_c_char_ptr();
    if len > 0 {
        unsafe {
            s.buf.extend_from_slice(std::slice::from_raw_parts(c, len));
        }
    }
}

#[inline]
pub fn strbuf_ensure_null(s: &mut Strbuf) {
    s.buf.reserve(1);
    let len = s.buf.len();
    unsafe {
        *s.buf.as_mut_ptr().add(len) = 0;
    }
}

#[inline]
pub fn strbuf_string(s: &mut Strbuf, len: &mut usize) -> *mut c_char {
    *len = s.buf.len();
    s.buf.as_mut_ptr()
}

pub fn strbuf_init(s: &mut Strbuf, len: usize) {
    let size = if len == 0 {
        STRBUF_DEFAULT_SIZE
    } else {
        len.saturating_add(1)
    };

    s.buf = Vec::with_capacity(size);
    strbuf_ensure_null(s);
}

pub fn strbuf_append_string(s: &mut Strbuf, str_in: impl crate::strings::IntoCCharPtr) {
    let str_in = str_in.into_c_char_ptr();
     {
        let bytes = crate::memory::cstr_as_bytes(str_in);
        strbuf_append_mem(s, bytes.as_ptr().cast::<c_char>(), bytes.len());
    }
}
