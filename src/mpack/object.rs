//! MessagePack object tree parser and unparser (`src/mpack/object.c`).

use std::ffi::{c_char, c_int};
use std::ptr;

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_ERROR, MPACK_OK, MPACK_TOKEN_CHUNK, MPACK_TOKEN_MAP, mpack_read,
    mpack_sintmax_t, mpack_tokbuf_init, mpack_tokbuf_t, mpack_token_t, mpack_uint32_t, mpack_write,
};

pub const MPACK_MAX_OBJECT_DEPTH: usize = 32;
pub const MPACK_EXCEPTION: c_int = -1;
pub const MPACK_NOMEM: c_int = MPACK_ERROR + 1;

#[derive(Clone, Default)]
pub enum mpack_data_t {
    #[default]
    Null,
    Int(mpack_sintmax_t),
    LmpackUnpacker(*mut crate::mpack::lmpack::LmpackUnpacker),
    LmpackPacker(*mut crate::mpack::lmpack::Packer),
    Typval(*mut crate::eval::typval::typval_T),
    List(*mut crate::eval::typval::list_T),
    MapPairs(Vec<[crate::eval::typval::typval_T; 2]>),
    OwnedBytes(Vec<u8>),
    Bytes(*mut u8),
    Unpacker(*mut crate::msgpack_rpc::unpacker::Unpacker),
    Object(*mut crate::api::private::validate::Object),
}

impl mpack_data_t {
    pub const fn new() -> Self {
        Self::Null
    }

    #[inline]
    pub const fn i(&self) -> mpack_sintmax_t {
        match *self {
            Self::Int(i) => i,
            _ => 0,
        }
    }

    #[inline]
    pub const fn lmpack_unpacker(&self) -> *mut crate::mpack::lmpack::LmpackUnpacker {
        match *self {
            Self::LmpackUnpacker(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn lmpack_packer(&self) -> *mut crate::mpack::lmpack::Packer {
        match *self {
            Self::LmpackPacker(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn typval(&self) -> *mut crate::eval::typval::typval_T {
        match *self {
            Self::Typval(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn list(&self) -> *mut crate::eval::typval::list_T {
        match *self {
            Self::List(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub fn map_pairs_mut(&mut self) -> &mut [[crate::eval::typval::typval_T; 2]] {
        match self {
            Self::MapPairs(v) => v.as_mut_slice(),
            _ => &mut [],
        }
    }

    #[inline]
    pub fn take_map_pairs(&mut self) -> Vec<[crate::eval::typval::typval_T; 2]> {
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
pub struct mpack_node_t {
    pub tok: mpack_token_t,
    pub pos: usize,
    pub key_visited: c_int,
    pub data: [mpack_data_t; 2],
}

impl mpack_node_t {
    pub const fn new() -> Self {
        Self {
            tok: mpack_token_t::new(),
            pos: 0,
            key_visited: 0,
            data: [mpack_data_t::new(), mpack_data_t::new()],
        }
    }
}

impl Default for mpack_node_t {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct mpack_parser_t {
    pub data: mpack_data_t,
    pub size: mpack_uint32_t,
    pub capacity: mpack_uint32_t,
    pub status: c_int,
    pub exiting: c_int,
    pub tokbuf: mpack_tokbuf_t,
    pub items: Vec<mpack_node_t>,
}

impl mpack_parser_t {
    pub const fn new(capacity: mpack_uint32_t) -> Self {
        Self {
            data: mpack_data_t::new(),
            size: 0,
            capacity: if capacity != 0 {
                capacity
            } else {
                MPACK_MAX_OBJECT_DEPTH as mpack_uint32_t
            },
            status: 0,
            exiting: 0,
            tokbuf: mpack_tokbuf_t::new(),
            items: Vec::new(),
        }
    }
}

pub type mpack_walk_cb = Option<fn(w: &mut mpack_parser_t, n: &mut mpack_node_t)>;

#[inline]
pub fn mpack_parent_node(n: *mut mpack_node_t) -> *mut mpack_node_t {
    unsafe {
        let prev = n.sub(1);
        if (*prev).pos == usize::MAX {
            ptr::null_mut()
        } else {
            prev
        }
    }
}

pub fn mpack_parser_init(parser: &mut mpack_parser_t, capacity: mpack_uint32_t) {
    mpack_tokbuf_init(&mut parser.tokbuf);
    parser.data = mpack_data_t::Null;
    parser.capacity = if capacity != 0 {
        capacity
    } else {
        MPACK_MAX_OBJECT_DEPTH as mpack_uint32_t
    };
    parser.size = 0;
    parser.exiting = 0;
    parser.items.clear();
    parser
        .items
        .resize((parser.capacity as usize) + 1, mpack_node_t::new());
    parser.items[0].pos = usize::MAX;
    parser.status = 0;
}

pub fn mpack_parser_grow(p: &mut mpack_parser_t) {
    let new_capacity = p.capacity * 2;
    p.items
        .resize((new_capacity as usize) + 1, mpack_node_t::new());
    p.capacity = new_capacity;
}

pub fn mpack_parse_tok(
    parser: &mut mpack_parser_t,
    tok: mpack_token_t,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
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
    parser: &mut mpack_parser_t,
    tok: &mut mpack_token_t,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
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
    parser: &mut mpack_parser_t,
    buf: &mut *const c_char,
    buflen: &mut usize,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
) -> c_int {
    let mut status = MPACK_EOF;
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    while *buflen != 0 && status != 0 {
        let mut tok = mpack_token_t::new();
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
    parser: &mut mpack_parser_t,
    buf: &mut *mut c_char,
    buflen: &mut usize,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
) -> c_int {
    let mut status = MPACK_EOF;
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    while *buflen != 0 && status != 0 {
        let mut tok = mpack_token_t::new();

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
const fn mpack_parser_full(parser: &mpack_parser_t) -> bool {
    parser.size == parser.capacity
}

fn mpack_parser_push(parser: &mut mpack_parser_t) -> *mut mpack_node_t {
    debug_assert!(parser.size < parser.capacity);
    let top_idx = (parser.size as usize) + 1;
    let top = &raw mut parser.items[top_idx];
    unsafe {
        (*top).data[0] = mpack_data_t::Null;
        (*top).data[1] = mpack_data_t::Null;
        (*top).pos = 0;
        (*top).key_visited = 0;
    }
    parser.size += 1;
    top
}

fn mpack_parser_pop(parser: &mut mpack_parser_t) -> *mut mpack_node_t {
    unsafe {
        debug_assert!(parser.size != 0);
        let top_idx = parser.size as usize;
        let top = &raw mut parser.items[top_idx];

        if (*top).tok.r#type > MPACK_TOKEN_CHUNK && (*top).pos < ((*top).tok.length as usize) {
            return ptr::null_mut();
        }

        let parent = mpack_parent_node(top);
        if !parent.is_null() {
            if (*top).tok.r#type == MPACK_TOKEN_CHUNK {
                (*parent).pos += (*top).tok.length as usize;
            } else if (*parent).tok.r#type == MPACK_TOKEN_MAP {
                if (*parent).key_visited != 0 {
                    (*parent).pos += 1;
                }
                (*parent).key_visited = c_int::from((*parent).key_visited == 0);
            } else {
                (*parent).pos += 1;
            }
        }

        parser.size -= 1;
        top
    }
}
