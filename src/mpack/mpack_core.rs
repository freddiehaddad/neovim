//! Low-level MessagePack token reader and writer (`src/mpack/mpack_core.c`).
#![forbid(unsafe_code)]

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MpackTokenData {
    Value(MpackValue),
    Chunk(Vec<u8>),
    ExtType(i32),
}

impl MpackTokenData {
    pub const fn new() -> Self {
        Self::Value(MpackValue { lo: 0, hi: 0 })
    }

    #[inline]
    pub const fn value(&self) -> MpackValue {
        match *self {
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
    pub fn chunk_bytes(&self) -> &[u8] {
        match self {
            Self::Chunk(b) => b.as_slice(),
            _ => &[],
        }
    }

    #[inline]
    pub const fn ext_type(&self) -> i32 {
        match *self {
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Default)]
pub struct MpackTokbuf {
    pub pending: [u8; MPACK_MAX_TOKEN_LEN],
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
fn advance(buf: &mut &[u8]) -> u8 {
    let b = buf[0];
    *buf = &buf[1..];
    b
}

#[inline]
fn write_slice_adv(buf: &mut &mut [u8], src: &[u8]) {
    let out = std::mem::take(buf);
    out[..src.len()].copy_from_slice(src);
    *buf = &mut out[src.len()..];
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

pub fn mpack_read(tokbuf: &mut MpackTokbuf, buf: &mut &[u8], tok: &mut MpackToken) -> i32 {
    if buf.is_empty() {
        return MPACK_EOF;
    }

    if tokbuf.passthrough != 0 {
        let len = (buf.len().min(tokbuf.passthrough as usize)) as MpackUint32;
        tok.r#type = MpackTokenChunk;
        tok.data = MpackTokenData::Chunk(buf[..len as usize].to_vec());
        tok.length = len;
        tokbuf.passthrough -= len;
        *buf = &buf[len as usize..];
        return MPACK_OK;
    }

    let initial_ppos = tokbuf.ppos;
    let status = if tokbuf.plen != 0 {
        if mpack_rpending(buf, tokbuf) == 0 {
            return MPACK_EOF;
        }
        let mut slice = &tokbuf.pending[..tokbuf.ppos];
        let st = mpack_rtoken(&mut slice, tok);
        if st == 0 {
            let consumed_total = tokbuf.ppos - slice.len();
            let advanced = consumed_total - initial_ppos;
            tokbuf.plen = 0;
            tokbuf.ppos = 0;
            *buf = &buf[advanced..];
        }
        st
    } else {
        let mut slice = *buf;
        let st = mpack_rtoken(&mut slice, tok);
        if st == 0 {
            tokbuf.plen = 0;
            tokbuf.ppos = 0;
            *buf = slice;
        }
        st
    };

    if status != 0 {
        if status != MPACK_EOF {
            return MPACK_ERROR;
        }
        debug_assert_eq!(tokbuf.plen, 0);
        tokbuf.plen = (tok.length as usize) + 1;
        debug_assert!(tokbuf.plen <= tokbuf.pending.len());
        tokbuf.ppos = 0;
        let pend_status = mpack_rpending(buf, tokbuf);
        debug_assert_eq!(pend_status, 0);
        return MPACK_EOF;
    }

    if tok.r#type > MpackTokenMap {
        tokbuf.passthrough = tok.length;
    }

    MPACK_OK
}

pub fn mpack_write(tokbuf: &mut MpackTokbuf, buf: &mut &mut [u8], t: &MpackToken) -> i32 {
    let tok = if tokbuf.plen != 0 {
        tokbuf.pending_tok.clone()
    } else {
        t.clone()
    };
    debug_assert!(!buf.is_empty());

    if tok.r#type == MpackTokenChunk {
        if tokbuf.plen == 0 {
            tokbuf.ppos = 0;
        }
        let written = tokbuf.ppos;
        let total = tok.length as usize;
        let pending = total - written;
        let count = pending.min(buf.len());
        let chunk = &tok.data.chunk_bytes()[written..written + count];
        write_slice_adv(buf, chunk);
        tokbuf.ppos += count;
        tokbuf.plen = if count == pending { 0 } else { total };
        if count == pending {
            return MPACK_OK;
        } else {
            tokbuf.pending_tok = tok;
            return MPACK_EOF;
        }
    }

    if tokbuf.plen != 0 {
        return mpack_wpending(buf, tokbuf);
    }

    if buf.len() < MPACK_MAX_TOKEN_LEN {
        let pending_cap = tokbuf.pending.len();
        let mut tmp = &mut tokbuf.pending[..];
        let status = mpack_wtoken(&tok, &mut tmp);
        if status != 0 {
            return status;
        }
        let toklen = pending_cap - tmp.len();
        let write_cnt = toklen.min(buf.len());
        write_slice_adv(buf, &tokbuf.pending[..write_cnt]);
        if write_cnt < toklen {
            debug_assert!(buf.is_empty());
            tokbuf.plen = toklen;
            tokbuf.ppos = write_cnt;
            tokbuf.pending_tok = tok;
            return MPACK_EOF;
        }
    } else {
        let status = mpack_wtoken(&tok, buf);
        if status != 0 {
            return status;
        }
    }

    MPACK_OK
}

pub fn mpack_rtoken(buf: &mut &[u8], tok: &mut MpackToken) -> i32 {
    if buf.is_empty() {
        return MPACK_EOF;
    }
    let t = advance(buf);
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
            0xc4..=0xc6 => mpack_rblob(MpackTokenBin, tlen(t, 0xc4), buf, tok),
            0xc7..=0xc9 => mpack_rblob(MpackTokenExt, tlen(t, 0xc7), buf, tok),
            0xca | 0xcb => mpack_rvalue(MpackTokenFloat, tlen(t, 0xc8), buf, tok),
            0xcc..=0xcf => mpack_rvalue(MpackTokenUint, tlen(t, 0xcc), buf, tok),
            0xd0..=0xd3 => mpack_rvalue(MpackTokenSint, tlen(t, 0xd0), buf, tok),
            0xd4..=0xd8 => {
                if buf.is_empty() {
                    tok.length = 1;
                    return MPACK_EOF;
                }
                tok.length = tlen(t, 0xd4);
                tok.r#type = MpackTokenExt;
                tok.data = MpackTokenData::ExtType(i32::from(advance(buf)));
                MPACK_OK
            }
            0xd9..=0xdb => mpack_rblob(MpackTokenStr, tlen(t, 0xd9), buf, tok),
            0xdc | 0xdd => mpack_rblob(MpackTokenArray, tlen(t, 0xdb), buf, tok),
            0xde | 0xdf => mpack_rblob(MpackTokenMap, tlen(t, 0xdd), buf, tok),
            _ => MPACK_ERROR,
        }
    } else {
        mpack_value(MpackTokenSint, 1, mpack_byte(t), tok)
    }
}

fn mpack_rpending(buf: &mut &[u8], state: &mut MpackTokbuf) -> i32 {
    debug_assert!(state.ppos < state.plen);
    let count = (state.plen - state.ppos).min(buf.len());
    state.pending[state.ppos..state.ppos + count].copy_from_slice(&buf[..count]);
    state.ppos += count;
    if state.ppos < state.plen {
        *buf = &buf[count..];
        return 0;
    }
    1
}

fn mpack_rvalue(
    r#type: MpackTokenType,
    mut remaining: MpackUint32,
    buf: &mut &[u8],
    tok: &mut MpackToken,
) -> i32 {
    if buf.len() < remaining as usize {
        tok.length = remaining;
        return MPACK_EOF;
    }

    mpack_value(r#type, remaining, mpack_byte(0), tok);

    while remaining != 0 {
        let byte = u32::from(advance(buf));
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
    buf: &mut &[u8],
    tok: &mut MpackToken,
) -> i32 {
    let required = tlen + u32::from(r#type == MpackTokenExt);
    if buf.len() < required as usize {
        tok.length = required;
        return MPACK_EOF;
    }

    let mut l = MpackToken::new();
    mpack_rvalue(MpackTokenUint, tlen, buf, &mut l);
    tok.r#type = r#type;
    tok.length = l.data.value().lo;

    if r#type == MpackTokenExt {
        tok.data = MpackTokenData::ExtType(i32::from(advance(buf)));
    }

    MPACK_OK
}

fn mpack_wtoken(tok: &MpackToken, buf: &mut &mut [u8]) -> i32 {
    match tok.r#type {
        MpackTokenNil => mpack_w1(buf, 0xc0),
        MpackTokenBoolean => mpack_w1(buf, if tok.data.value().lo != 0 { 0xc3 } else { 0xc2 }),
        MpackTokenUint => mpack_wpint(buf, tok.data.value()),
        MpackTokenSint => mpack_wnint(buf, tok.data.value()),
        MpackTokenFloat => mpack_wfloat(buf, tok),
        MpackTokenBin => mpack_wbin(buf, tok.length),
        MpackTokenStr => mpack_wstr(buf, tok.length),
        MpackTokenExt => mpack_wext(buf, tok.data.ext_type(), tok.length),
        MpackTokenArray => mpack_warray(buf, tok.length),
        MpackTokenMap => mpack_wmap(buf, tok.length),
        _ => MPACK_ERROR,
    }
}

fn mpack_wpending(buf: &mut &mut [u8], state: &mut MpackTokbuf) -> i32 {
    debug_assert!(state.ppos < state.plen);
    let count = (state.plen - state.ppos).min(buf.len());
    write_slice_adv(buf, &state.pending[state.ppos..state.ppos + count]);
    state.ppos += count;
    if state.ppos == state.plen {
        state.plen = 0;
        return MPACK_OK;
    }
    MPACK_EOF
}

fn mpack_wpint(buf: &mut &mut [u8], val: MpackValue) -> i32 {
    let hi = val.hi;
    let lo = val.lo;
    if hi != 0 {
        mpack_w1(buf, 0xcf);
        mpack_w4(buf, hi);
        mpack_w4(buf, lo)
    } else if lo > 0xffff {
        mpack_w1(buf, 0xce);
        mpack_w4(buf, lo)
    } else if lo > 0xff {
        mpack_w1(buf, 0xcd);
        mpack_w2(buf, lo)
    } else if lo > 0x7f {
        mpack_w1(buf, 0xcc);
        mpack_w1(buf, lo)
    } else {
        mpack_w1(buf, lo)
    }
}

fn mpack_wnint(buf: &mut &mut [u8], val: MpackValue) -> i32 {
    let hi = val.hi;
    let lo = val.lo;
    if (hi != 0 && hi != 0xffffffff) || lo < 0x80000000 {
        mpack_w1(buf, 0xd3);
        mpack_w4(buf, hi);
        mpack_w4(buf, lo)
    } else if lo < 0xffff8000 {
        mpack_w1(buf, 0xd2);
        mpack_w4(buf, lo)
    } else if lo < 0xffffff80 {
        mpack_w1(buf, 0xd1);
        mpack_w2(buf, lo)
    } else if lo < 0xffffffe0 {
        mpack_w1(buf, 0xd0);
        mpack_w1(buf, lo)
    } else {
        mpack_w1(buf, 0x100u32.wrapping_add(lo))
    }
}

fn mpack_wfloat(buf: &mut &mut [u8], tok: &MpackToken) -> i32 {
    let v = tok.data.value();
    if tok.length == 4 {
        mpack_w1(buf, 0xca);
        mpack_w4(buf, v.lo)
    } else if tok.length == 8 {
        mpack_w1(buf, 0xcb);
        mpack_w4(buf, v.hi);
        mpack_w4(buf, v.lo)
    } else {
        MPACK_ERROR
    }
}

fn mpack_wstr(buf: &mut &mut [u8], len: MpackUint32) -> i32 {
    if len < 0x20 {
        mpack_w1(buf, 0xa0 | len)
    } else if len < 0x100 {
        mpack_w1(buf, 0xd9);
        mpack_w1(buf, len)
    } else if len < 0x10000 {
        mpack_w1(buf, 0xda);
        mpack_w2(buf, len)
    } else {
        mpack_w1(buf, 0xdb);
        mpack_w4(buf, len)
    }
}

fn mpack_wbin(buf: &mut &mut [u8], len: MpackUint32) -> i32 {
    if len < 0x100 {
        mpack_w1(buf, 0xc4);
        mpack_w1(buf, len)
    } else if len < 0x10000 {
        mpack_w1(buf, 0xc5);
        mpack_w2(buf, len)
    } else {
        mpack_w1(buf, 0xc6);
        mpack_w4(buf, len)
    }
}

fn mpack_wext(buf: &mut &mut [u8], r#type: i32, len: MpackUint32) -> i32 {
    debug_assert!((0..0x80).contains(&r#type));
    let t = r#type as MpackUint32;
    match len {
        1 => {
            mpack_w1(buf, 0xd4);
            mpack_w1(buf, t)
        }
        2 => {
            mpack_w1(buf, 0xd5);
            mpack_w1(buf, t)
        }
        4 => {
            mpack_w1(buf, 0xd6);
            mpack_w1(buf, t)
        }
        8 => {
            mpack_w1(buf, 0xd7);
            mpack_w1(buf, t)
        }
        16 => {
            mpack_w1(buf, 0xd8);
            mpack_w1(buf, t)
        }
        _ => {
            if len < 0x100 {
                mpack_w1(buf, 0xc7);
                mpack_w1(buf, len);
                mpack_w1(buf, t)
            } else if len < 0x10000 {
                mpack_w1(buf, 0xc8);
                mpack_w2(buf, len);
                mpack_w1(buf, t)
            } else {
                mpack_w1(buf, 0xc9);
                mpack_w4(buf, len);
                mpack_w1(buf, t)
            }
        }
    }
}

fn mpack_warray(buf: &mut &mut [u8], len: MpackUint32) -> i32 {
    if len < 0x10 {
        mpack_w1(buf, 0x90 | len)
    } else if len < 0x10000 {
        mpack_w1(buf, 0xdc);
        mpack_w2(buf, len)
    } else {
        mpack_w1(buf, 0xdd);
        mpack_w4(buf, len)
    }
}

fn mpack_wmap(buf: &mut &mut [u8], len: MpackUint32) -> i32 {
    if len < 0x10 {
        mpack_w1(buf, 0x80 | len)
    } else if len < 0x10000 {
        mpack_w1(buf, 0xde);
        mpack_w2(buf, len)
    } else {
        mpack_w1(buf, 0xdf);
        mpack_w4(buf, len)
    }
}

#[inline]
fn mpack_w1(b: &mut &mut [u8], v: MpackUint32) -> i32 {
    write_slice_adv(b, &[(v & 0xff) as u8]);
    MPACK_OK
}

#[inline]
fn mpack_w2(b: &mut &mut [u8], v: MpackUint32) -> i32 {
    write_slice_adv(b, &[((v >> 8) & 0xff) as u8, (v & 0xff) as u8]);
    MPACK_OK
}

#[inline]
fn mpack_w4(b: &mut &mut [u8], v: MpackUint32) -> i32 {
    write_slice_adv(
        b,
        &[
            ((v >> 24) & 0xff) as u8,
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        ],
    );
    MPACK_OK
}
