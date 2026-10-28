//! Low-level MessagePack token reader and writer (`src/mpack/mpack_core.c`).

use std::ffi::c_char;
use std::ptr;

pub type MpackUint32 = u32;
pub type MpackSintmax = i64;
pub type MpackUintmax = u64;

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct MpackValue {
    pub lo: MpackUint32,
    pub hi: MpackUint32,
}

pub const MPACK_OK: i32 = 0;
pub const MPACK_EOF: i32 = 1;
pub const MPACK_ERROR: i32 = 2;

pub const MPACK_MAX_TOKEN_LEN: usize = 9;

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MpackTokenType {
    #[default]
    MpackTokenNone = 0,
    MpackTokenNil = 1,
    MpackTokenBoolean = 2,
    MpackTokenUint = 3,
    MpackTokenSint = 4,
    MpackTokenFloat = 5,
    MpackTokenChunk = 6,
    MpackTokenArray = 7,
    MpackTokenMap = 8,
    MpackTokenBin = 9,
    MpackTokenStr = 10,
    MpackTokenExt = 11,
}

pub use MpackTokenType::*;

#[derive(Copy, Clone)]
pub enum MpackTokenData {
    Value(MpackValue),
    ChunkPtr(*const c_char),
    ExtType(i32),
}

impl MpackTokenData {
    pub const fn new() -> Self {
        Self::Value(MpackValue { lo: 0, hi: 0 })
    }

    #[inline]
    pub const fn value(self) -> MpackValue {
        match self {
            Self::Value(v) => v,
            _ => MpackValue { lo: 0, hi: 0 },
        }
    }

    #[inline]
    pub fn value_mut(&mut self) -> &mut MpackValue {
        if !matches!(self, Self::Value(_)) {
            *self = Self::Value(MpackValue { lo: 0, hi: 0 });
        }
        match self {
            Self::Value(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn chunk_ptr(self) -> *const c_char {
        match self {
            Self::ChunkPtr(p) => p,
            _ => ptr::null(),
        }
    }

    #[inline]
    pub const fn ext_type(self) -> i32 {
        match self {
            Self::ExtType(t) => t,
            _ => 0,
        }
    }
}

impl Default for MpackTokenData {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct MpackToken {
    pub r#type: MpackTokenType,
    pub length: MpackUint32,
    pub data: MpackTokenData,
}

impl MpackToken {
    pub const fn new() -> Self {
        Self {
            r#type: MpackTokenNone,
            length: 0,
            data: MpackTokenData::new(),
        }
    }
}

impl Default for MpackToken {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct MpackTokbuf {
    pub pending: [c_char; MPACK_MAX_TOKEN_LEN],
    pub pending_tok: MpackToken,
    pub ppos: usize,
    pub plen: usize,
    pub passthrough: MpackUint32,
}

impl MpackTokbuf {
    pub const fn new() -> Self {
        Self {
            pending: [0; MPACK_MAX_TOKEN_LEN],
            pending_tok: MpackToken::new(),
            ppos: 0,
            plen: 0,
            passthrough: 0,
        }
    }
}

#[inline]
fn advance(buf: &mut *const c_char, buflen: &mut usize) -> u8 {
    unsafe {
        *buflen -= 1;
        let b = **buf as u8;
        *buf = (*buf).add(1);
        b
    }
}

#[inline]
const fn tlen(val: u8, range_start: u8) -> MpackUint32 {
    1u32 << (val - range_start)
}

#[inline]
const fn mpack_byte(byte: u8) -> MpackValue {
    MpackValue {
        lo: byte as MpackUint32,
        hi: 0,
    }
}

#[inline]
fn mpack_value(
    r#type: MpackTokenType,
    length: MpackUint32,
    value: MpackValue,
    tok: &mut MpackToken,
) -> i32 {
    tok.r#type = r#type;
    tok.length = length;
    tok.data = MpackTokenData::Value(value);
    MPACK_OK
}

#[inline]
fn mpack_blob(
    r#type: MpackTokenType,
    length: MpackUint32,
    ext_type: i32,
    tok: &mut MpackToken,
) -> i32 {
    tok.r#type = r#type;
    tok.length = length;
    tok.data = MpackTokenData::ExtType(ext_type);
    MPACK_OK
}

pub fn mpack_tokbuf_init(tokbuf: &mut MpackTokbuf) {
    tokbuf.ppos = 0;
    tokbuf.plen = 0;
    tokbuf.passthrough = 0;
}

pub fn mpack_read(
    tokbuf: &mut MpackTokbuf,
    buf: &mut *const c_char,
    buflen: &mut usize,
    tok: &mut MpackToken,
) -> i32 {
    unsafe {
        debug_assert!(!(*buf).is_null());
        if *buflen == 0 {
            return MPACK_EOF;
        }

        if tokbuf.passthrough != 0 {
            tok.r#type = MpackTokenChunk;
            tok.data = MpackTokenData::ChunkPtr(*buf);
            let len = ((*buflen).min(tokbuf.passthrough as usize)) as MpackUint32;
            tok.length = len;
            tokbuf.passthrough -= len;
            *buf = (*buf).add(len as usize);
            *buflen -= len as usize;
            return MPACK_OK;
        }

        let initial_ppos = tokbuf.ppos;
        let mut ptr: *const c_char;
        let mut ptrlen: usize;

        if tokbuf.plen != 0 {
            if mpack_rpending(buf, buflen, tokbuf) == 0 {
                return MPACK_EOF;
            }
            ptr = tokbuf.pending.as_ptr();
            ptrlen = tokbuf.ppos;
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
            debug_assert_eq!(tokbuf.plen, 0);
            tokbuf.plen = (tok.length as usize) + 1;
            debug_assert!(tokbuf.plen <= tokbuf.pending.len());
            tokbuf.ppos = 0;
            let pend_status = mpack_rpending(buf, buflen, tokbuf);
            debug_assert_eq!(pend_status, 0);
            return MPACK_EOF;
        }

        let advanced = (ptr.offset_from(ptr_save) as usize) - initial_ppos;
        tokbuf.plen = 0;
        tokbuf.ppos = 0;
        *buflen -= advanced;
        *buf = (*buf).add(advanced);

        if tok.r#type > MpackTokenMap {
            tokbuf.passthrough = tok.length;
        }

        MPACK_OK
    }
}

pub fn mpack_write(
    tokbuf: &mut MpackTokbuf,
    buf: &mut *mut c_char,
    buflen: &mut usize,
    t: &MpackToken,
) -> i32 {
    unsafe {
        let tok = if tokbuf.plen != 0 {
            tokbuf.pending_tok
        } else {
            *t
        };
        debug_assert!(!(*buf).is_null() && *buflen != 0);

        if tok.r#type == MpackTokenChunk {
            if tokbuf.plen == 0 {
                tokbuf.ppos = 0;
            }
            let written = tokbuf.ppos;
            let pending = (tok.length as usize) - written;
            let count = pending.min(*buflen);
            ptr::copy_nonoverlapping(tok.data.chunk_ptr().add(written), *buf, count);
            *buf = (*buf).add(count);
            *buflen -= count;
            tokbuf.ppos += count;
            tokbuf.plen = if count == pending {
                0
            } else {
                tok.length as usize
            };
            if count == pending {
                return MPACK_OK;
            } else {
                tokbuf.pending_tok = tok;
                return MPACK_EOF;
            }
        }

        if tokbuf.plen != 0 {
            return mpack_wpending(buf, buflen, tokbuf);
        }

        let mut ptr: *mut c_char;
        let mut ptrlen: usize;
        if *buflen < MPACK_MAX_TOKEN_LEN {
            ptr = tokbuf.pending.as_mut_ptr();
            ptrlen = tokbuf.pending.len();
        } else {
            ptr = *buf;
            ptrlen = *buflen;
        }

        let status = mpack_wtoken(&tok, &mut ptr, &mut ptrlen);
        if status != 0 {
            return status;
        }

        if *buflen < MPACK_MAX_TOKEN_LEN {
            let toklen = tokbuf.pending.len() - ptrlen;
            let write_cnt = toklen.min(*buflen);
            ptr::copy_nonoverlapping(tokbuf.pending.as_ptr(), *buf, write_cnt);
            *buf = (*buf).add(write_cnt);
            *buflen -= write_cnt;
            if write_cnt < toklen {
                debug_assert_eq!(*buflen, 0);
                tokbuf.plen = toklen;
                tokbuf.ppos = write_cnt;
                tokbuf.pending_tok = tok;
                return MPACK_EOF;
            }
        } else {
            *buflen -= ptr.offset_from(*buf) as usize;
            *buf = ptr;
        }

        MPACK_OK
    }
}

pub fn mpack_rtoken(buf: &mut *const c_char, buflen: &mut usize, tok: &mut MpackToken) -> i32 {
    if *buflen == 0 {
        return MPACK_EOF;
    }
    let t = advance(buf, buflen);
    if t < 0x80 {
        mpack_value(MpackTokenUint, 1, mpack_byte(t), tok)
    } else if t < 0x90 {
        mpack_blob(MpackTokenMap, u32::from(t & 0x0f), 0, tok)
    } else if t < 0xa0 {
        mpack_blob(MpackTokenArray, u32::from(t & 0x0f), 0, tok)
    } else if t < 0xc0 {
        mpack_blob(MpackTokenStr, u32::from(t & 0x1f), 0, tok)
    } else if t < 0xe0 {
        match t {
            0xc0 => mpack_value(MpackTokenNil, 0, mpack_byte(0), tok),
            0xc2 => mpack_value(MpackTokenBoolean, 1, mpack_byte(0), tok),
            0xc3 => mpack_value(MpackTokenBoolean, 1, mpack_byte(1), tok),
            0xc4..=0xc6 => mpack_rblob(MpackTokenBin, tlen(t, 0xc4), buf, buflen, tok),
            0xc7..=0xc9 => mpack_rblob(MpackTokenExt, tlen(t, 0xc7), buf, buflen, tok),
            0xca | 0xcb => mpack_rvalue(MpackTokenFloat, tlen(t, 0xc8), buf, buflen, tok),
            0xcc..=0xcf => mpack_rvalue(MpackTokenUint, tlen(t, 0xcc), buf, buflen, tok),
            0xd0..=0xd3 => mpack_rvalue(MpackTokenSint, tlen(t, 0xd0), buf, buflen, tok),
            0xd4..=0xd8 => {
                if *buflen == 0 {
                    tok.length = 1;
                    return MPACK_EOF;
                }
                tok.length = tlen(t, 0xd4);
                tok.r#type = MpackTokenExt;
                tok.data = MpackTokenData::ExtType(i32::from(advance(buf, buflen)));
                MPACK_OK
            }
            0xd9..=0xdb => mpack_rblob(MpackTokenStr, tlen(t, 0xd9), buf, buflen, tok),
            0xdc | 0xdd => mpack_rblob(MpackTokenArray, tlen(t, 0xdb), buf, buflen, tok),
            0xde | 0xdf => mpack_rblob(MpackTokenMap, tlen(t, 0xdd), buf, buflen, tok),
            _ => MPACK_ERROR,
        }
    } else {
        mpack_value(MpackTokenSint, 1, mpack_byte(t), tok)
    }
}

fn mpack_rpending(buf: &mut *const c_char, buflen: &mut usize, state: &mut MpackTokbuf) -> i32 {
    unsafe {
        debug_assert!(state.ppos < state.plen);
        let count = (state.plen - state.ppos).min(*buflen);
        ptr::copy_nonoverlapping(*buf, state.pending.as_mut_ptr().add(state.ppos), count);
        state.ppos += count;
        if state.ppos < state.plen {
            *buf = (*buf).add(*buflen);
            *buflen = 0;
            return 0;
        }
        1
    }
}

fn mpack_rvalue(
    r#type: MpackTokenType,
    mut remaining: MpackUint32,
    buf: &mut *const c_char,
    buflen: &mut usize,
    tok: &mut MpackToken,
) -> i32 {
    if *buflen < remaining as usize {
        tok.length = remaining;
        return MPACK_EOF;
    }

    mpack_value(r#type, remaining, mpack_byte(0), tok);

    while remaining != 0 {
        let byte = u32::from(advance(buf, buflen));
        remaining -= 1;
        let byte_idx = remaining;
        let byte_shift = (byte_idx % 4) * 8;
        let v = tok.data.value_mut();
        v.lo |= byte << byte_shift;
        if remaining == 4 {
            v.hi = v.lo;
            v.lo = 0;
        }
    }

    if r#type == MpackTokenSint {
        let v = tok.data.value();
        let hi = v.hi;
        let lo = v.lo;
        let msb = (tok.length == 8 && (hi >> 31) != 0)
            || (tok.length == 4 && (lo >> 31) != 0)
            || (tok.length == 2 && (lo >> 15) != 0)
            || (tok.length == 1 && (lo >> 7) != 0);
        if !msb {
            tok.r#type = MpackTokenUint;
        }
    }

    MPACK_OK
}

fn mpack_rblob(
    r#type: MpackTokenType,
    tlen: MpackUint32,
    buf: &mut *const c_char,
    buflen: &mut usize,
    tok: &mut MpackToken,
) -> i32 {
    let required = tlen + u32::from(r#type == MpackTokenExt);
    if *buflen < required as usize {
        tok.length = required;
        return MPACK_EOF;
    }

    let mut l = MpackToken::new();
    mpack_rvalue(MpackTokenUint, tlen, buf, buflen, &mut l);
    tok.r#type = r#type;
    tok.length = l.data.value().lo;

    if r#type == MpackTokenExt {
        tok.data = MpackTokenData::ExtType(i32::from(advance(buf, buflen)));
    }

    MPACK_OK
}

fn mpack_wtoken(tok: &MpackToken, buf: &mut *mut c_char, buflen: &mut usize) -> i32 {
    match tok.r#type {
        MpackTokenNil => mpack_w1(buf, buflen, 0xc0),
        MpackTokenBoolean => mpack_w1(
            buf,
            buflen,
            if tok.data.value().lo != 0 { 0xc3 } else { 0xc2 },
        ),
        MpackTokenUint => mpack_wpint(buf, buflen, tok.data.value()),
        MpackTokenSint => mpack_wnint(buf, buflen, tok.data.value()),
        MpackTokenFloat => mpack_wfloat(buf, buflen, tok),
        MpackTokenBin => mpack_wbin(buf, buflen, tok.length),
        MpackTokenStr => mpack_wstr(buf, buflen, tok.length),
        MpackTokenExt => mpack_wext(buf, buflen, tok.data.ext_type(), tok.length),
        MpackTokenArray => mpack_warray(buf, buflen, tok.length),
        MpackTokenMap => mpack_wmap(buf, buflen, tok.length),
        _ => MPACK_ERROR,
    }
}

fn mpack_wpending(buf: &mut *mut c_char, buflen: &mut usize, state: &mut MpackTokbuf) -> i32 {
    unsafe {
        debug_assert!(state.ppos < state.plen);
        let count = (state.plen - state.ppos).min(*buflen);
        ptr::copy_nonoverlapping(state.pending.as_ptr().add(state.ppos), *buf, count);
        state.ppos += count;
        *buf = (*buf).add(count);
        *buflen -= count;
        if state.ppos == state.plen {
            state.plen = 0;
            return MPACK_OK;
        }
        MPACK_EOF
    }
}

fn mpack_wpint(buf: &mut *mut c_char, buflen: &mut usize, val: MpackValue) -> i32 {
    let hi = val.hi;
    let lo = val.lo;
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

fn mpack_wnint(buf: &mut *mut c_char, buflen: &mut usize, val: MpackValue) -> i32 {
    let hi = val.hi;
    let lo = val.lo;
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

fn mpack_wfloat(buf: &mut *mut c_char, buflen: &mut usize, tok: &MpackToken) -> i32 {
    let v = tok.data.value();
    if tok.length == 4 {
        mpack_w1(buf, buflen, 0xca);
        mpack_w4(buf, buflen, v.lo)
    } else if tok.length == 8 {
        mpack_w1(buf, buflen, 0xcb);
        mpack_w4(buf, buflen, v.hi);
        mpack_w4(buf, buflen, v.lo)
    } else {
        MPACK_ERROR
    }
}

fn mpack_wstr(buf: &mut *mut c_char, buflen: &mut usize, len: MpackUint32) -> i32 {
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

fn mpack_wbin(buf: &mut *mut c_char, buflen: &mut usize, len: MpackUint32) -> i32 {
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

fn mpack_wext(buf: &mut *mut c_char, buflen: &mut usize, r#type: i32, len: MpackUint32) -> i32 {
    debug_assert!((0..0x80).contains(&r#type));
    let t = r#type as MpackUint32;
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

fn mpack_warray(buf: &mut *mut c_char, buflen: &mut usize, len: MpackUint32) -> i32 {
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

fn mpack_wmap(buf: &mut *mut c_char, buflen: &mut usize, len: MpackUint32) -> i32 {
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

#[inline]
fn mpack_w1(b: &mut *mut c_char, bl: &mut usize, v: MpackUint32) -> i32 {
    unsafe {
        *bl -= 1;
        **b = (v & 0xff) as u8 as c_char;
        *b = (*b).add(1);
    }
    MPACK_OK
}

#[inline]
fn mpack_w2(b: &mut *mut c_char, bl: &mut usize, v: MpackUint32) -> i32 {
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
fn mpack_w4(b: &mut *mut c_char, bl: &mut usize, v: MpackUint32) -> i32 {
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
