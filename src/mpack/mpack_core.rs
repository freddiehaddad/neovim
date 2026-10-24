//! Low-level MessagePack token reader and writer (`src/mpack/mpack_core.c`).

use std::ffi::{c_char, c_int, c_longlong, c_ulonglong};
use std::ptr;

pub type mpack_uint32_t = u32;
pub type mpack_sintmax_t = c_longlong;
pub type mpack_uintmax_t = c_ulonglong;

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct mpack_value_t {
    pub lo: mpack_uint32_t,
    pub hi: mpack_uint32_t,
}

pub const MPACK_OK: c_int = 0;
pub const MPACK_EOF: c_int = 1;
pub const MPACK_ERROR: c_int = 2;

pub const MPACK_MAX_TOKEN_LEN: usize = 9;

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum mpack_token_type_t {
    #[default]
    MPACK_TOKEN_NONE = 0,
    MPACK_TOKEN_NIL = 1,
    MPACK_TOKEN_BOOLEAN = 2,
    MPACK_TOKEN_UINT = 3,
    MPACK_TOKEN_SINT = 4,
    MPACK_TOKEN_FLOAT = 5,
    MPACK_TOKEN_CHUNK = 6,
    MPACK_TOKEN_ARRAY = 7,
    MPACK_TOKEN_MAP = 8,
    MPACK_TOKEN_BIN = 9,
    MPACK_TOKEN_STR = 10,
    MPACK_TOKEN_EXT = 11,
}

pub use mpack_token_type_t::*;

#[repr(C)]
#[derive(Copy, Clone)]
pub union mpack_token_data_t {
    pub value: mpack_value_t,
    pub chunk_ptr: *const c_char,
    pub ext_type: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpack_token_t {
    pub r#type: mpack_token_type_t,
    pub length: mpack_uint32_t,
    pub data: mpack_token_data_t,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpack_tokbuf_t {
    pub pending: [c_char; MPACK_MAX_TOKEN_LEN],
    pub pending_tok: mpack_token_t,
    pub ppos: usize,
    pub plen: usize,
    pub passthrough: mpack_uint32_t,
}

impl mpack_tokbuf_t {
    pub const fn new() -> Self {
        Self {
            pending: [0; MPACK_MAX_TOKEN_LEN],
            pending_tok: mpack_token_t {
                r#type: MPACK_TOKEN_NONE,
                length: 0,
                data: mpack_token_data_t {
                    value: mpack_value_t { lo: 0, hi: 0 },
                },
            },
            ppos: 0,
            plen: 0,
            passthrough: 0,
        }
    }
}

#[inline]
unsafe fn advance(buf: *mut *const c_char, buflen: *mut usize) -> u8 {
    unsafe {
        *buflen -= 1;
        let b = **buf as u8;
        *buf = (*buf).add(1);
        b
    }
}

#[inline]
const fn tlen(val: u8, range_start: u8) -> mpack_uint32_t {
    1u32 << (val - range_start)
}

#[inline]
const fn mpack_byte(byte: u8) -> mpack_value_t {
    mpack_value_t {
        lo: byte as mpack_uint32_t,
        hi: 0,
    }
}

#[inline]
unsafe fn mpack_value(
    r#type: mpack_token_type_t,
    length: mpack_uint32_t,
    value: mpack_value_t,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        (*tok).r#type = r#type;
        (*tok).length = length;
        (*tok).data.value = value;
    }
    MPACK_OK
}

#[inline]
unsafe fn mpack_blob(
    r#type: mpack_token_type_t,
    length: mpack_uint32_t,
    ext_type: c_int,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        (*tok).r#type = r#type;
        (*tok).length = length;
        (*tok).data.ext_type = ext_type;
    }
    MPACK_OK
}

pub unsafe fn mpack_tokbuf_init(tokbuf: *mut mpack_tokbuf_t) {
    unsafe {
        (*tokbuf).ppos = 0;
        (*tokbuf).plen = 0;
        (*tokbuf).passthrough = 0;
    }
}

pub unsafe fn mpack_read(
    tokbuf: *mut mpack_tokbuf_t,
    buf: *mut *const c_char,
    buflen: *mut usize,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        debug_assert!(!(*buf).is_null());
        if *buflen == 0 {
            return MPACK_EOF;
        }

        if (*tokbuf).passthrough != 0 {
            (*tok).r#type = MPACK_TOKEN_CHUNK;
            (*tok).data.chunk_ptr = *buf;
            let len = ((*buflen).min((*tokbuf).passthrough as usize)) as mpack_uint32_t;
            (*tok).length = len;
            (*tokbuf).passthrough -= len;
            *buf = (*buf).add(len as usize);
            *buflen -= len as usize;
            return MPACK_OK;
        }

        let initial_ppos = (*tokbuf).ppos;
        let mut ptr: *const c_char;
        let mut ptrlen: usize;

        if (*tokbuf).plen != 0 {
            if mpack_rpending(buf, buflen, tokbuf) == 0 {
                return MPACK_EOF;
            }
            ptr = (*tokbuf).pending.as_ptr();
            ptrlen = (*tokbuf).ppos;
        } else {
            ptr = *buf;
            ptrlen = *buflen;
        }

        let ptr_save = ptr;
        let status = mpack_rtoken(&mut ptr, &mut ptrlen, tok);
        if status != 0 {
            if status != MPACK_EOF {
                return MPACK_ERROR;
            }
            debug_assert_eq!((*tokbuf).plen, 0);
            (*tokbuf).plen = ((*tok).length as usize) + 1;
            debug_assert!((*tokbuf).plen <= (*tokbuf).pending.len());
            (*tokbuf).ppos = 0;
            let pend_status = mpack_rpending(buf, buflen, tokbuf);
            debug_assert_eq!(pend_status, 0);
            return MPACK_EOF;
        }

        let advanced = (ptr.offset_from(ptr_save) as usize) - initial_ppos;
        (*tokbuf).plen = 0;
        (*tokbuf).ppos = 0;
        *buflen -= advanced;
        *buf = (*buf).add(advanced);

        if (*tok).r#type > MPACK_TOKEN_MAP {
            (*tokbuf).passthrough = (*tok).length;
        }

        MPACK_OK
    }
}

pub unsafe fn mpack_write(
    tokbuf: *mut mpack_tokbuf_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
    t: *const mpack_token_t,
) -> c_int {
    unsafe {
        let tok = if (*tokbuf).plen != 0 {
            (*tokbuf).pending_tok
        } else {
            *t
        };
        debug_assert!(!(*buf).is_null() && *buflen != 0);

        if tok.r#type == MPACK_TOKEN_CHUNK {
            if (*tokbuf).plen == 0 {
                (*tokbuf).ppos = 0;
            }
            let written = (*tokbuf).ppos;
            let pending = (tok.length as usize) - written;
            let count = pending.min(*buflen);
            ptr::copy_nonoverlapping(tok.data.chunk_ptr.add(written), *buf, count);
            *buf = (*buf).add(count);
            *buflen -= count;
            (*tokbuf).ppos += count;
            (*tokbuf).plen = if count == pending {
                0
            } else {
                tok.length as usize
            };
            if count == pending {
                return MPACK_OK;
            } else {
                (*tokbuf).pending_tok = tok;
                return MPACK_EOF;
            }
        }

        if (*tokbuf).plen != 0 {
            return mpack_wpending(buf, buflen, tokbuf);
        }

        let mut ptr: *mut c_char;
        let mut ptrlen: usize;
        if *buflen < MPACK_MAX_TOKEN_LEN {
            ptr = (*tokbuf).pending.as_mut_ptr();
            ptrlen = (*tokbuf).pending.len();
        } else {
            ptr = *buf;
            ptrlen = *buflen;
        }

        let status = mpack_wtoken(&tok, &mut ptr, &mut ptrlen);
        if status != 0 {
            return status;
        }

        if *buflen < MPACK_MAX_TOKEN_LEN {
            let toklen = (*tokbuf).pending.len() - ptrlen;
            let write_cnt = toklen.min(*buflen);
            ptr::copy_nonoverlapping((*tokbuf).pending.as_ptr(), *buf, write_cnt);
            *buf = (*buf).add(write_cnt);
            *buflen -= write_cnt;
            if write_cnt < toklen {
                debug_assert_eq!(*buflen, 0);
                (*tokbuf).plen = toklen;
                (*tokbuf).ppos = write_cnt;
                (*tokbuf).pending_tok = tok;
                return MPACK_EOF;
            }
        } else {
            *buflen -= ptr.offset_from(*buf) as usize;
            *buf = ptr;
        }

        MPACK_OK
    }
}

pub unsafe fn mpack_rtoken(
    buf: *mut *const c_char,
    buflen: *mut usize,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        if *buflen == 0 {
            return MPACK_EOF;
        }
        let t = advance(buf, buflen);
        if t < 0x80 {
            mpack_value(MPACK_TOKEN_UINT, 1, mpack_byte(t), tok)
        } else if t < 0x90 {
            mpack_blob(MPACK_TOKEN_MAP, u32::from(t & 0x0f), 0, tok)
        } else if t < 0xa0 {
            mpack_blob(MPACK_TOKEN_ARRAY, u32::from(t & 0x0f), 0, tok)
        } else if t < 0xc0 {
            mpack_blob(MPACK_TOKEN_STR, u32::from(t & 0x1f), 0, tok)
        } else if t < 0xe0 {
            match t {
                0xc0 => mpack_value(MPACK_TOKEN_NIL, 0, mpack_byte(0), tok),
                0xc2 => mpack_value(MPACK_TOKEN_BOOLEAN, 1, mpack_byte(0), tok),
                0xc3 => mpack_value(MPACK_TOKEN_BOOLEAN, 1, mpack_byte(1), tok),
                0xc4..=0xc6 => mpack_rblob(MPACK_TOKEN_BIN, tlen(t, 0xc4), buf, buflen, tok),
                0xc7..=0xc9 => mpack_rblob(MPACK_TOKEN_EXT, tlen(t, 0xc7), buf, buflen, tok),
                0xca | 0xcb => mpack_rvalue(MPACK_TOKEN_FLOAT, tlen(t, 0xc8), buf, buflen, tok),
                0xcc..=0xcf => mpack_rvalue(MPACK_TOKEN_UINT, tlen(t, 0xcc), buf, buflen, tok),
                0xd0..=0xd3 => mpack_rvalue(MPACK_TOKEN_SINT, tlen(t, 0xd0), buf, buflen, tok),
                0xd4..=0xd8 => {
                    if *buflen == 0 {
                        (*tok).length = 1;
                        return MPACK_EOF;
                    }
                    (*tok).length = tlen(t, 0xd4);
                    (*tok).r#type = MPACK_TOKEN_EXT;
                    (*tok).data.ext_type = c_int::from(advance(buf, buflen));
                    MPACK_OK
                }
                0xd9..=0xdb => mpack_rblob(MPACK_TOKEN_STR, tlen(t, 0xd9), buf, buflen, tok),
                0xdc | 0xdd => mpack_rblob(MPACK_TOKEN_ARRAY, tlen(t, 0xdb), buf, buflen, tok),
                0xde | 0xdf => mpack_rblob(MPACK_TOKEN_MAP, tlen(t, 0xdd), buf, buflen, tok),
                _ => MPACK_ERROR,
            }
        } else {
            mpack_value(MPACK_TOKEN_SINT, 1, mpack_byte(t), tok)
        }
    }
}

unsafe fn mpack_rpending(
    buf: *mut *const c_char,
    buflen: *mut usize,
    state: *mut mpack_tokbuf_t,
) -> c_int {
    unsafe {
        debug_assert!((*state).ppos < (*state).plen);
        let count = ((*state).plen - (*state).ppos).min(*buflen);
        ptr::copy_nonoverlapping(
            *buf,
            (*state).pending.as_mut_ptr().add((*state).ppos),
            count,
        );
        (*state).ppos += count;
        if (*state).ppos < (*state).plen {
            *buf = (*buf).add(*buflen);
            *buflen = 0;
            return 0;
        }
        1
    }
}

unsafe fn mpack_rvalue(
    r#type: mpack_token_type_t,
    mut remaining: mpack_uint32_t,
    buf: *mut *const c_char,
    buflen: *mut usize,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        if *buflen < remaining as usize {
            (*tok).length = remaining;
            return MPACK_EOF;
        }

        mpack_value(r#type, remaining, mpack_byte(0), tok);

        while remaining != 0 {
            let byte = u32::from(advance(buf, buflen));
            remaining -= 1;
            let byte_idx = remaining;
            let byte_shift = (byte_idx % 4) * 8;
            (*tok).data.value.lo |= byte << byte_shift;
            if remaining == 4 {
                (*tok).data.value.hi = (*tok).data.value.lo;
                (*tok).data.value.lo = 0;
            }
        }

        if r#type == MPACK_TOKEN_SINT {
            let hi = (*tok).data.value.hi;
            let lo = (*tok).data.value.lo;
            let msb = ((*tok).length == 8 && (hi >> 31) != 0)
                || ((*tok).length == 4 && (lo >> 31) != 0)
                || ((*tok).length == 2 && (lo >> 15) != 0)
                || ((*tok).length == 1 && (lo >> 7) != 0);
            if !msb {
                (*tok).r#type = MPACK_TOKEN_UINT;
            }
        }

        MPACK_OK
    }
}

unsafe fn mpack_rblob(
    r#type: mpack_token_type_t,
    tlen: mpack_uint32_t,
    buf: *mut *const c_char,
    buflen: *mut usize,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        let required = tlen + u32::from(r#type == MPACK_TOKEN_EXT);
        if *buflen < required as usize {
            (*tok).length = required;
            return MPACK_EOF;
        }

        let mut l: mpack_token_t = std::mem::zeroed();
        mpack_rvalue(MPACK_TOKEN_UINT, tlen, buf, buflen, &mut l);
        (*tok).r#type = r#type;
        (*tok).length = l.data.value.lo;

        if r#type == MPACK_TOKEN_EXT {
            (*tok).data.ext_type = c_int::from(advance(buf, buflen));
        }

        MPACK_OK
    }
}

unsafe fn mpack_wtoken(
    tok: *const mpack_token_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
) -> c_int {
    unsafe {
        match (*tok).r#type {
            MPACK_TOKEN_NIL => mpack_w1(buf, buflen, 0xc0),
            MPACK_TOKEN_BOOLEAN => mpack_w1(
                buf,
                buflen,
                if (*tok).data.value.lo != 0 {
                    0xc3
                } else {
                    0xc2
                },
            ),
            MPACK_TOKEN_UINT => mpack_wpint(buf, buflen, (*tok).data.value),
            MPACK_TOKEN_SINT => mpack_wnint(buf, buflen, (*tok).data.value),
            MPACK_TOKEN_FLOAT => mpack_wfloat(buf, buflen, tok),
            MPACK_TOKEN_BIN => mpack_wbin(buf, buflen, (*tok).length),
            MPACK_TOKEN_STR => mpack_wstr(buf, buflen, (*tok).length),
            MPACK_TOKEN_EXT => mpack_wext(buf, buflen, (*tok).data.ext_type, (*tok).length),
            MPACK_TOKEN_ARRAY => mpack_warray(buf, buflen, (*tok).length),
            MPACK_TOKEN_MAP => mpack_wmap(buf, buflen, (*tok).length),
            _ => MPACK_ERROR,
        }
    }
}

unsafe fn mpack_wpending(
    buf: *mut *mut c_char,
    buflen: *mut usize,
    state: *mut mpack_tokbuf_t,
) -> c_int {
    unsafe {
        debug_assert!((*state).ppos < (*state).plen);
        let count = ((*state).plen - (*state).ppos).min(*buflen);
        ptr::copy_nonoverlapping((*state).pending.as_ptr().add((*state).ppos), *buf, count);
        (*state).ppos += count;
        *buf = (*buf).add(count);
        *buflen -= count;
        if (*state).ppos == (*state).plen {
            (*state).plen = 0;
            return MPACK_OK;
        }
        MPACK_EOF
    }
}

unsafe fn mpack_wpint(buf: *mut *mut c_char, buflen: *mut usize, val: mpack_value_t) -> c_int {
    let hi = val.hi;
    let lo = val.lo;
    unsafe {
        if hi != 0 {
            mpack_w1(buf, buflen, 0xcf);
            mpack_w4(buf, buflen, hi);
            mpack_w4(buf, buflen, lo)
        } else if lo > 0xffff {
            mpack_w1(buf, buflen, 0xce);
            mpack_w4(buf, buflen, lo)
        } else if lo > 0xff {
            mpack_w1(buf, buflen, 0xcd);
            mpack_w2(buf, buflen, lo)
        } else if lo > 0x7f {
            mpack_w1(buf, buflen, 0xcc);
            mpack_w1(buf, buflen, lo)
        } else {
            mpack_w1(buf, buflen, lo)
        }
    }
}

unsafe fn mpack_wnint(buf: *mut *mut c_char, buflen: *mut usize, val: mpack_value_t) -> c_int {
    let hi = val.hi;
    let lo = val.lo;
    unsafe {
        if (hi != 0 && hi != 0xffffffff) || lo < 0x80000000 {
            mpack_w1(buf, buflen, 0xd3);
            mpack_w4(buf, buflen, hi);
            mpack_w4(buf, buflen, lo)
        } else if lo < 0xffff8000 {
            mpack_w1(buf, buflen, 0xd2);
            mpack_w4(buf, buflen, lo)
        } else if lo < 0xffffff80 {
            mpack_w1(buf, buflen, 0xd1);
            mpack_w2(buf, buflen, lo)
        } else if lo < 0xffffffe0 {
            mpack_w1(buf, buflen, 0xd0);
            mpack_w1(buf, buflen, lo)
        } else {
            mpack_w1(buf, buflen, 0x100u32.wrapping_add(lo))
        }
    }
}

unsafe fn mpack_wfloat(
    buf: *mut *mut c_char,
    buflen: *mut usize,
    tok: *const mpack_token_t,
) -> c_int {
    unsafe {
        if (*tok).length == 4 {
            mpack_w1(buf, buflen, 0xca);
            mpack_w4(buf, buflen, (*tok).data.value.lo)
        } else if (*tok).length == 8 {
            mpack_w1(buf, buflen, 0xcb);
            mpack_w4(buf, buflen, (*tok).data.value.hi);
            mpack_w4(buf, buflen, (*tok).data.value.lo)
        } else {
            MPACK_ERROR
        }
    }
}

unsafe fn mpack_wstr(buf: *mut *mut c_char, buflen: *mut usize, len: mpack_uint32_t) -> c_int {
    unsafe {
        if len < 0x20 {
            mpack_w1(buf, buflen, 0xa0 | len)
        } else if len < 0x100 {
            mpack_w1(buf, buflen, 0xd9);
            mpack_w1(buf, buflen, len)
        } else if len < 0x10000 {
            mpack_w1(buf, buflen, 0xda);
            mpack_w2(buf, buflen, len)
        } else {
            mpack_w1(buf, buflen, 0xdb);
            mpack_w4(buf, buflen, len)
        }
    }
}

unsafe fn mpack_wbin(buf: *mut *mut c_char, buflen: *mut usize, len: mpack_uint32_t) -> c_int {
    unsafe {
        if len < 0x100 {
            mpack_w1(buf, buflen, 0xc4);
            mpack_w1(buf, buflen, len)
        } else if len < 0x10000 {
            mpack_w1(buf, buflen, 0xc5);
            mpack_w2(buf, buflen, len)
        } else {
            mpack_w1(buf, buflen, 0xc6);
            mpack_w4(buf, buflen, len)
        }
    }
}

unsafe fn mpack_wext(
    buf: *mut *mut c_char,
    buflen: *mut usize,
    r#type: c_int,
    len: mpack_uint32_t,
) -> c_int {
    debug_assert!((0..0x80).contains(&r#type));
    let t = r#type as mpack_uint32_t;
    unsafe {
        match len {
            1 => {
                mpack_w1(buf, buflen, 0xd4);
                mpack_w1(buf, buflen, t)
            }
            2 => {
                mpack_w1(buf, buflen, 0xd5);
                mpack_w1(buf, buflen, t)
            }
            4 => {
                mpack_w1(buf, buflen, 0xd6);
                mpack_w1(buf, buflen, t)
            }
            8 => {
                mpack_w1(buf, buflen, 0xd7);
                mpack_w1(buf, buflen, t)
            }
            16 => {
                mpack_w1(buf, buflen, 0xd8);
                mpack_w1(buf, buflen, t)
            }
            _ => {
                if len < 0x100 {
                    mpack_w1(buf, buflen, 0xc7);
                    mpack_w1(buf, buflen, len);
                    mpack_w1(buf, buflen, t)
                } else if len < 0x10000 {
                    mpack_w1(buf, buflen, 0xc8);
                    mpack_w2(buf, buflen, len);
                    mpack_w1(buf, buflen, t)
                } else {
                    mpack_w1(buf, buflen, 0xc9);
                    mpack_w4(buf, buflen, len);
                    mpack_w1(buf, buflen, t)
                }
            }
        }
    }
}

unsafe fn mpack_warray(buf: *mut *mut c_char, buflen: *mut usize, len: mpack_uint32_t) -> c_int {
    unsafe {
        if len < 0x10 {
            mpack_w1(buf, buflen, 0x90 | len)
        } else if len < 0x10000 {
            mpack_w1(buf, buflen, 0xdc);
            mpack_w2(buf, buflen, len)
        } else {
            mpack_w1(buf, buflen, 0xdd);
            mpack_w4(buf, buflen, len)
        }
    }
}

unsafe fn mpack_wmap(buf: *mut *mut c_char, buflen: *mut usize, len: mpack_uint32_t) -> c_int {
    unsafe {
        if len < 0x10 {
            mpack_w1(buf, buflen, 0x80 | len)
        } else if len < 0x10000 {
            mpack_w1(buf, buflen, 0xde);
            mpack_w2(buf, buflen, len)
        } else {
            mpack_w1(buf, buflen, 0xdf);
            mpack_w4(buf, buflen, len)
        }
    }
}

#[inline]
unsafe fn mpack_w1(b: *mut *mut c_char, bl: *mut usize, v: mpack_uint32_t) -> c_int {
    unsafe {
        *bl -= 1;
        **b = (v & 0xff) as u8 as c_char;
        *b = (*b).add(1);
    }
    MPACK_OK
}

#[inline]
unsafe fn mpack_w2(b: *mut *mut c_char, bl: *mut usize, v: mpack_uint32_t) -> c_int {
    unsafe {
        *bl -= 2;
        **b = ((v >> 8) & 0xff) as u8 as c_char;
        *b = (*b).add(1);
        **b = (v & 0xff) as u8 as c_char;
        *b = (*b).add(1);
    }
    MPACK_OK
}

#[inline]
unsafe fn mpack_w4(b: *mut *mut c_char, bl: *mut usize, v: mpack_uint32_t) -> c_int {
    unsafe {
        *bl -= 4;
        **b = ((v >> 24) & 0xff) as u8 as c_char;
        *b = (*b).add(1);
        **b = ((v >> 16) & 0xff) as u8 as c_char;
        *b = (*b).add(1);
        **b = ((v >> 8) & 0xff) as u8 as c_char;
        *b = (*b).add(1);
        **b = (v & 0xff) as u8 as c_char;
        *b = (*b).add(1);
    }
    MPACK_OK
}
