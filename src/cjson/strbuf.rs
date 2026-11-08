//! String buffer routines (`src/cjson/strbuf.c`).
#![forbid(unsafe_code)]

use std::ffi::CStr;

pub const STRBUF_DEFAULT_SIZE: usize = 1023;

#[repr(C)]
pub struct Strbuf {
    pub buf: Vec<u8>,
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
pub fn strbuf_set_length(s: &mut Strbuf, len: i32) {
    s.buf.truncate(len as usize);
}

#[inline]
pub fn strbuf_length(s: &Strbuf) -> usize {
    s.buf.len()
}

#[inline]
pub fn strbuf_append_char(s: &mut Strbuf, c: u8) {
    s.buf.push(c);
}

#[inline]
pub fn strbuf_append_char_unsafe(s: &mut Strbuf, c: u8) {
    s.buf.push(c);
}

#[inline]
pub fn strbuf_append_mem(s: &mut Strbuf, bytes: &[u8]) {
    s.buf.extend_from_slice(bytes);
}

#[inline]
pub fn strbuf_append_mem_unsafe(s: &mut Strbuf, bytes: &[u8]) {
    s.buf.extend_from_slice(bytes);
}

#[inline]
pub fn strbuf_ensure_null(s: &mut Strbuf) {
    let len = s.buf.len();
    s.buf.push(0);
    s.buf.truncate(len);
}

#[inline]
pub fn strbuf_string<'a>(s: &'a mut Strbuf, len: &mut usize) -> &'a mut [u8] {
    *len = s.buf.len();
    &mut s.buf
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

pub fn strbuf_append_string(s: &mut Strbuf, str_in: &CStr) {
    strbuf_append_mem(s, str_in.to_bytes());
}
