//! MessagePack object tree parser and unparser (`src/mpack/object.c`).

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_ERROR, MPACK_OK, MPACK_TOKEN_CHUNK, MPACK_TOKEN_MAP, mpack_read,
    mpack_sintmax_t, mpack_tokbuf_init, mpack_tokbuf_t, mpack_token_t, mpack_uint32_t,
    mpack_write,
};

pub const MPACK_MAX_OBJECT_DEPTH: usize = 32;
pub const MPACK_EXCEPTION: c_int = -1;
pub const MPACK_NOMEM: c_int = MPACK_ERROR + 1;

#[derive(Copy, Clone)]
pub enum mpack_data_t {
    Null,
    Ptr(*mut c_void),
    Int(mpack_sintmax_t),
}

impl mpack_data_t {
    pub const fn new() -> Self {
        Self::Null
    }

    #[inline]
    pub const fn p(self) -> *mut c_void {
        match self {
            Self::Ptr(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn i(self) -> mpack_sintmax_t {
        match self {
            Self::Int(i) => i,
            _ => 0,
        }
    }
}

impl Default for mpack_data_t {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
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
            data: [mpack_data_t::new(); 2],
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
    pub items: [mpack_node_t; MPACK_MAX_OBJECT_DEPTH + 1],
    pub items_vec: Vec<mpack_node_t>,
}

impl mpack_parser_t {
    pub const fn new(capacity: mpack_uint32_t) -> Self {
        let zero_node = mpack_node_t::new();
        let mut items = [zero_node; MPACK_MAX_OBJECT_DEPTH + 1];
        items[0].pos = usize::MAX;
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
            items,
            items_vec: Vec::new(),
        }
    }
}

pub type mpack_walk_cb = Option<unsafe fn(w: *mut mpack_parser_t, n: *mut mpack_node_t)>;

#[inline]
pub unsafe fn mpack_parent_node(n: *mut mpack_node_t) -> *mut mpack_node_t {
    unsafe {
        let prev = n.sub(1);
        if (*prev).pos == usize::MAX {
            ptr::null_mut()
        } else {
            prev
        }
    }
}

#[inline]
unsafe fn parser_items_ptr(parser: *mut mpack_parser_t) -> *mut mpack_node_t {
    unsafe {
        if (*parser).items_vec.is_empty() {
            (&raw mut (*parser).items).cast::<mpack_node_t>()
        } else {
            (*parser).items_vec.as_mut_ptr()
        }
    }
}

pub unsafe fn mpack_parser_init(parser: *mut mpack_parser_t, capacity: mpack_uint32_t) {
    unsafe {
        let parser = &mut *parser;
        mpack_tokbuf_init(&raw mut parser.tokbuf);
        parser.data = mpack_data_t::Null;
        parser.capacity = if capacity != 0 {
            capacity
        } else {
            MPACK_MAX_OBJECT_DEPTH as mpack_uint32_t
        };
        parser.size = 0;
        parser.exiting = 0;
        if (parser.capacity as usize) <= MPACK_MAX_OBJECT_DEPTH {
            parser.items_vec = Vec::new();
            parser.items[..(parser.capacity as usize) + 1].fill(mpack_node_t::new());
            parser.items[0].pos = usize::MAX;
        } else {
            parser.items_vec = vec![mpack_node_t::new(); (parser.capacity as usize) + 1];
            parser.items_vec[0].pos = usize::MAX;
        }
        parser.status = 0;
    }
}

pub unsafe fn mpack_parser_grow(parser: *mut mpack_parser_t) {
    unsafe {
        let p = &mut *parser;
        let new_capacity = p.capacity * 2;
        if p.items_vec.is_empty() {
            let mut v = vec![mpack_node_t::new(); (new_capacity as usize) + 1];
            let copy_len = ((p.capacity as usize) + 1).min(p.items.len());
            v[..copy_len].copy_from_slice(&p.items[..copy_len]);
            p.items_vec = v;
        } else {
            p.items_vec.resize((new_capacity as usize) + 1, mpack_node_t::new());
        }
        p.capacity = new_capacity;
    }
}

pub unsafe fn mpack_parse_tok(
    parser: *mut mpack_parser_t,
    tok: mpack_token_t,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
) -> c_int {
    unsafe {
        if (*parser).status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        if (*parser).exiting == 0 {
            if mpack_parser_full(parser) {
                return MPACK_NOMEM;
            }
            let n = mpack_parser_push(parser);
            (*n).tok = tok;
            if let Some(cb) = enter_cb {
                cb(parser, n);
            }
            if (*parser).status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            (*parser).exiting = 1;
            return MPACK_EOF;
        }

        (*parser).exiting = 0;
        loop {
            let n = mpack_parser_pop(parser);
            if n.is_null() {
                break;
            }
            if let Some(cb) = exit_cb {
                cb(parser, n);
            }
            if (*parser).status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            if (*parser).size == 0 {
                return MPACK_OK;
            }
        }

        MPACK_EOF
    }
}

pub unsafe fn mpack_unparse_tok(
    parser: *mut mpack_parser_t,
    tok: *mut mpack_token_t,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
) -> c_int {
    unsafe {
        if (*parser).status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        if (*parser).exiting == 0 {
            if mpack_parser_full(parser) {
                return MPACK_NOMEM;
            }
            let n = mpack_parser_push(parser);
            if let Some(cb) = enter_cb {
                cb(parser, n);
            }
            *tok = (*n).tok;
            if (*parser).status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            (*parser).exiting = 1;
            return MPACK_EOF;
        }

        (*parser).exiting = 0;
        loop {
            let n = mpack_parser_pop(parser);
            if n.is_null() {
                break;
            }
            if let Some(cb) = exit_cb {
                cb(parser, n);
            }
            if (*parser).status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            if (*parser).size == 0 {
                return MPACK_OK;
            }
        }

        MPACK_EOF
    }
}

pub unsafe fn mpack_parse(
    parser: *mut mpack_parser_t,
    buf: *mut *const c_char,
    buflen: *mut usize,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        if (*parser).status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        while *buflen != 0 && status != 0 {
            let mut tok = mpack_token_t::new();
            let tb = &raw mut (*parser).tokbuf;
            let buf_save = *buf;
            let buflen_save = *buflen;

            status = mpack_read(tb, buf, buflen, &mut tok);
            if status == MPACK_EOF {
                continue;
            } else if status == MPACK_ERROR {
                *buf = buf_save;
                *buflen = buflen_save;
                break;
            }

            loop {
                status = mpack_parse_tok(parser, tok, enter_cb, exit_cb);
                if (*parser).status == MPACK_EXCEPTION {
                    return MPACK_EXCEPTION;
                }
                if (*parser).exiting == 0 {
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
}

pub unsafe fn mpack_unparse(
    parser: *mut mpack_parser_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
    enter_cb: mpack_walk_cb,
    exit_cb: mpack_walk_cb,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        if (*parser).status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        while *buflen != 0 && status != 0 {
            let mut tok = mpack_token_t::new();
            let tb = &raw mut (*parser).tokbuf;

            if (*tb).plen == 0 {
                (*parser).status = mpack_unparse_tok(parser, &mut tok, enter_cb, exit_cb);
            }

            if (*parser).status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }

            status = (*parser).status;
            if status == MPACK_NOMEM {
                break;
            }

            if (*parser).exiting != 0 {
                let write_status = mpack_write(tb, buf, buflen, &tok);
                if write_status != 0 {
                    status = write_status;
                }
            }
        }

        status
    }
}

#[inline]
unsafe fn mpack_parser_full(parser: *mut mpack_parser_t) -> bool {
    unsafe { (*parser).size == (*parser).capacity }
}

unsafe fn mpack_parser_push(parser: *mut mpack_parser_t) -> *mut mpack_node_t {
    unsafe {
        debug_assert!((*parser).size < (*parser).capacity);
        let top = parser_items_ptr(parser).add(((*parser).size as usize) + 1);
        (*top).data[0] = mpack_data_t::Null;
        (*top).data[1] = mpack_data_t::Null;
        (*top).pos = 0;
        (*top).key_visited = 0;
        (*parser).size += 1;
        top
    }
}

unsafe fn mpack_parser_pop(parser: *mut mpack_parser_t) -> *mut mpack_node_t {
    unsafe {
        debug_assert!((*parser).size != 0);
        let top = parser_items_ptr(parser).add((*parser).size as usize);

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

        (*parser).size -= 1;
        top
    }
}
