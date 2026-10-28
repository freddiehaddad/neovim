//! MessagePack object tree parser and unparser (`src/mpack/object.c`).

use std::ffi::{c_char, c_int};
use std::ptr;

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_ERROR, MPACK_OK, MpackSintmax, MpackTokbuf, MpackToken, MpackTokenChunk,
    MpackTokenMap, MpackUint32, mpack_read, mpack_tokbuf_init, mpack_write,
};

pub const MPACK_MAX_OBJECT_DEPTH: usize = 32;
pub const MPACK_EXCEPTION: c_int = -1;
pub const MPACK_NOMEM: c_int = MPACK_ERROR + 1;

#[derive(Clone, Default)]
pub enum MpackData {
    #[default]
    Null,
    Int(MpackSintmax),
    LmpackUnpacker(*mut crate::mpack::lmpack::LmpackUnpacker),
    LmpackPacker(*mut crate::mpack::lmpack::Packer),
    Typval(*mut crate::eval::typval::TypVal),
    List(*mut crate::eval::typval::TvList),
    MapPairs(Vec<[crate::eval::typval::TypVal; 2]>),
    OwnedBytes(Vec<u8>),
    Bytes(*mut u8),
    Unpacker(*mut crate::msgpack_rpc::unpacker::Unpacker),
    Object(*mut crate::api::private::validate::Object),
}

impl MpackData {
    pub const fn new() -> Self {
        Self::Null
    }

    #[inline]
    pub const fn i(&self) -> MpackSintmax {
        match *self {
            Self::Int(i) => i,
            _ => 0,
        }
    }

    #[inline]
    pub fn lmpack_unpacker(&self) -> &'static mut crate::mpack::lmpack::LmpackUnpacker {
        match *self {
            Self::LmpackUnpacker(p) => unsafe { &mut *p },
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn lmpack_packer(&self) -> &'static mut crate::mpack::lmpack::Packer {
        match *self {
            Self::LmpackPacker(p) => unsafe { &mut *p },
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn typval(&self) -> *mut crate::eval::typval::TypVal {
        match *self {
            Self::Typval(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn list(&self) -> *mut crate::eval::typval::TvList {
        match *self {
            Self::List(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub fn map_pairs_mut(&mut self) -> &mut [[crate::eval::typval::TypVal; 2]] {
        match self {
            Self::MapPairs(v) => v.as_mut_slice(),
            _ => &mut [],
        }
    }

    #[inline]
    pub fn take_map_pairs(&mut self) -> Vec<[crate::eval::typval::TypVal; 2]> {
        match std::mem::take(self) {
            Self::MapPairs(v) => v,
            _ => Vec::new(),
        }
    }

    #[inline]
    pub const fn bytes(&self) -> *mut u8 {
        match *self {
            Self::Bytes(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub fn owned_bytes_mut(&mut self) -> &mut [u8] {
        match self {
            Self::OwnedBytes(v) => v.as_mut_slice(),
            _ => &mut [],
        }
    }

    #[inline]
    pub fn take_owned_bytes(&mut self) -> Vec<u8> {
        match std::mem::take(self) {
            Self::OwnedBytes(v) => v,
            _ => Vec::new(),
        }
    }

    #[inline]
    pub const fn unpacker(&self) -> *mut crate::msgpack_rpc::unpacker::Unpacker {
        match *self {
            Self::Unpacker(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn object(&self) -> *mut crate::api::private::validate::Object {
        match *self {
            Self::Object(p) => p,
            _ => ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct MpackNode {
    pub tok: MpackToken,
    pub pos: usize,
    pub key_visited: c_int,
    pub data: [MpackData; 2],
}

impl MpackNode {
    pub const fn new() -> Self {
        Self {
            tok: MpackToken::new(),
            pos: 0,
            key_visited: 0,
            data: [MpackData::new(), MpackData::new()],
        }
    }
}

impl Default for MpackNode {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct MpackParser {
    pub data: MpackData,
    pub size: MpackUint32,
    pub capacity: MpackUint32,
    pub status: c_int,
    pub exiting: c_int,
    pub tokbuf: MpackTokbuf,
    pub items: Vec<MpackNode>,
}

impl MpackParser {
    pub const fn new(capacity: MpackUint32) -> Self {
        Self {
            data: MpackData::new(),
            size: 0,
            capacity: if capacity != 0 {
                capacity
            } else {
                MPACK_MAX_OBJECT_DEPTH as MpackUint32
            },
            status: 0,
            exiting: 0,
            tokbuf: MpackTokbuf::new(),
            items: Vec::new(),
        }
    }
}

pub type MpackWalkCb = Option<fn(w: &mut MpackParser, n: &mut MpackNode)>;

#[inline]
pub fn mpack_parent_node(n: *mut MpackNode) -> *mut MpackNode {
    unsafe {
        let prev = n.sub(1);
        if (*prev).pos == usize::MAX {
            ptr::null_mut()
        } else {
            prev
        }
    }
}

pub fn mpack_parser_init(parser: &mut MpackParser, capacity: MpackUint32) {
    mpack_tokbuf_init(&mut parser.tokbuf);
    parser.data = MpackData::Null;
    parser.capacity = if capacity != 0 {
        capacity
    } else {
        MPACK_MAX_OBJECT_DEPTH as MpackUint32
    };
    parser.size = 0;
    parser.exiting = 0;
    parser.items.clear();
    parser
        .items
        .resize((parser.capacity as usize) + 1, MpackNode::new());
    parser.items[0].pos = usize::MAX;
    parser.status = 0;
}

pub fn mpack_parser_grow(p: &mut MpackParser) {
    let new_capacity = p.capacity * 2;
    p.items
        .resize((new_capacity as usize) + 1, MpackNode::new());
    p.capacity = new_capacity;
}

pub fn mpack_parse_tok(
    parser: &mut MpackParser,
    tok: MpackToken,
    enter_cb: MpackWalkCb,
    exit_cb: MpackWalkCb,
) -> c_int {
    unsafe {
        if parser.status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        if parser.exiting == 0 {
            if mpack_parser_full(parser) {
                return MPACK_NOMEM;
            }
            let n = mpack_parser_push(parser);
            (*n).tok = tok;
            if let Some(cb) = enter_cb {
                cb(parser, &mut *n);
            }
            if parser.status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            parser.exiting = 1;
            return MPACK_EOF;
        }

        parser.exiting = 0;
        loop {
            let n = mpack_parser_pop(parser);
            if n.is_null() {
                break;
            }
            if let Some(cb) = exit_cb {
                cb(parser, &mut *n);
            }
            if parser.status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            if parser.size == 0 {
                return MPACK_OK;
            }
        }

        MPACK_EOF
    }
}

pub fn mpack_unparse_tok(
    parser: &mut MpackParser,
    tok: &mut MpackToken,
    enter_cb: MpackWalkCb,
    exit_cb: MpackWalkCb,
) -> c_int {
    unsafe {
        if parser.status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        if parser.exiting == 0 {
            if mpack_parser_full(parser) {
                return MPACK_NOMEM;
            }
            let n = mpack_parser_push(parser);
            if let Some(cb) = enter_cb {
                cb(parser, &mut *n);
            }
            *tok = (*n).tok;
            if parser.status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            parser.exiting = 1;
            return MPACK_EOF;
        }

        parser.exiting = 0;
        loop {
            let n = mpack_parser_pop(parser);
            if n.is_null() {
                break;
            }
            if let Some(cb) = exit_cb {
                cb(parser, &mut *n);
            }
            if parser.status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            if parser.size == 0 {
                return MPACK_OK;
            }
        }

        MPACK_EOF
    }
}

pub fn mpack_parse(
    parser: &mut MpackParser,
    buf: &mut *const c_char,
    buflen: &mut usize,
    enter_cb: MpackWalkCb,
    exit_cb: MpackWalkCb,
) -> c_int {
    let mut status = MPACK_EOF;
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    while *buflen != 0 && status != 0 {
        let mut tok = MpackToken::new();
        let buf_save = *buf;
        let buflen_save = *buflen;

        status = mpack_read(&mut parser.tokbuf, buf, buflen, &mut tok);
        if status == MPACK_EOF {
            continue;
        } else if status == MPACK_ERROR {
            *buf = buf_save;
            *buflen = buflen_save;
            break;
        }

        loop {
            status = mpack_parse_tok(parser, tok, enter_cb, exit_cb);
            if parser.status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            if parser.exiting == 0 {
                break;
            }
        }

        if status == MPACK_NOMEM {
            *buf = buf_save;
            *buflen = buflen_save;
            break;
        }
    }

    status
}

pub fn mpack_unparse(
    parser: &mut MpackParser,
    buf: &mut *mut c_char,
    buflen: &mut usize,
    enter_cb: MpackWalkCb,
    exit_cb: MpackWalkCb,
) -> c_int {
    let mut status = MPACK_EOF;
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    while *buflen != 0 && status != 0 {
        let mut tok = MpackToken::new();

        if parser.tokbuf.plen == 0 {
            parser.status = mpack_unparse_tok(parser, &mut tok, enter_cb, exit_cb);
        }

        if parser.status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        status = parser.status;
        if status == MPACK_NOMEM {
            break;
        }

        if parser.exiting != 0 {
            let write_status = mpack_write(&mut parser.tokbuf, buf, buflen, &tok);
            if write_status != 0 {
                status = write_status;
            }
        }
    }

    status
}

#[inline]
const fn mpack_parser_full(parser: &MpackParser) -> bool {
    parser.size == parser.capacity
}

fn mpack_parser_push(parser: &mut MpackParser) -> *mut MpackNode {
    debug_assert!(parser.size < parser.capacity);
    let top_idx = (parser.size as usize) + 1;
    let top = &raw mut parser.items[top_idx];
    let top = unsafe { &mut *top };
    {
        top.data[0] = MpackData::Null;
        top.data[1] = MpackData::Null;
        top.pos = 0;
        top.key_visited = 0;
    }
    parser.size += 1;
    top
}

fn mpack_parser_pop(parser: &mut MpackParser) -> *mut MpackNode {
    {
        debug_assert!(parser.size != 0);
        let top_idx = parser.size as usize;
        let top = &raw mut parser.items[top_idx];
        let top = unsafe { &mut *top };

        if top.tok.r#type > MpackTokenChunk && top.pos < (top.tok.length as usize) {
            return ptr::null_mut();
        }

        let parent = mpack_parent_node(top);
        if !parent.is_null() {
            let parent = unsafe { &mut *parent };
            let parent = { &mut *parent };
            if top.tok.r#type == MpackTokenChunk {
                parent.pos += top.tok.length as usize;
            } else if parent.tok.r#type == MpackTokenMap {
                if parent.key_visited != 0 {
                    parent.pos += 1;
                }
                parent.key_visited = c_int::from(parent.key_visited == 0);
            } else {
                parent.pos += 1;
            }
        }

        parser.size -= 1;
        top
    }
}
