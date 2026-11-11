//! MessagePack object tree parser and unparser (`src/mpack/object.c`).
#![forbid(unsafe_code)]

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_ERROR, MPACK_OK, MpackSintmax, MpackTokbuf, MpackToken, MpackTokenChunk,
    MpackTokenMap, MpackUint32, mpack_read, mpack_tokbuf_init, mpack_write,
};

pub const MPACK_MAX_OBJECT_DEPTH: usize = 32;
pub const MPACK_EXCEPTION: i32 = -1;
pub const MPACK_NOMEM: i32 = MPACK_ERROR + 1;

#[derive(Clone, Default)]
pub enum MpackData {
    #[default]
    Null,
    Int(MpackSintmax),
    RootTypval,
    ListItem(crate::eval::typval::ListitemId),
    MapSlot(usize, usize),
    List(crate::eval::typval::ListId),
    MapPairs(Vec<[crate::eval::typval::TypVal; 2]>),
    OwnedBytes(Vec<u8>),
    Object(crate::api::private::validate::Object),
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
    pub fn list(&self) -> &'static mut crate::eval::typval::TvList {
        match *self {
            Self::List(p) => p.borrow_mut(),
            _ => unreachable!(),
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
    pub fn object_mut(&mut self) -> &mut crate::api::private::validate::Object {
        match self {
            Self::Object(obj) => obj,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn take_object(&mut self) -> crate::api::private::validate::Object {
        match std::mem::take(self) {
            Self::Object(obj) => obj,
            _ => crate::api::private::validate::Object::new(),
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct MpackNode {
    pub tok: MpackToken,
    pub pos: usize,
    pub key_visited: i32,
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
    pub status: i32,
    pub exiting: i32,
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

    #[inline]
    pub fn node_and_parent_mut(
        &mut self,
        node_idx: usize,
    ) -> (&mut MpackNode, Option<&mut MpackNode>) {
        let (before, after) = self.items.split_at_mut(node_idx);
        let node = &mut after[0];
        let parent = before.last_mut().filter(|p| p.pos != usize::MAX);
        (node, parent)
    }
}

pub type MpackWalkCb<C> = Option<fn(w: &mut MpackParser, ctx: &mut C, node_idx: usize)>;

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

pub fn mpack_parse_tok<C: ?Sized>(
    parser: &mut MpackParser,
    ctx: &mut C,
    tok: MpackToken,
    enter_cb: MpackWalkCb<C>,
    exit_cb: MpackWalkCb<C>,
) -> i32 {
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    if parser.exiting == 0 {
        if mpack_parser_full(parser) {
            return MPACK_NOMEM;
        }
        let n_idx = mpack_parser_push(parser);
        parser.items[n_idx].tok = tok;
        if let Some(cb) = enter_cb {
            cb(parser, ctx, n_idx);
        }
        if parser.status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }
        parser.exiting = 1;
        return MPACK_EOF;
    }

    parser.exiting = 0;
    while let Some(n_idx) = mpack_parser_pop(parser) {
        if let Some(cb) = exit_cb {
            cb(parser, ctx, n_idx);
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

pub fn mpack_unparse_tok<C: ?Sized>(
    parser: &mut MpackParser,
    ctx: &mut C,
    tok: &mut MpackToken,
    enter_cb: MpackWalkCb<C>,
    exit_cb: MpackWalkCb<C>,
) -> i32 {
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    if parser.exiting == 0 {
        if mpack_parser_full(parser) {
            return MPACK_NOMEM;
        }
        let n_idx = mpack_parser_push(parser);
        if let Some(cb) = enter_cb {
            cb(parser, ctx, n_idx);
        }
        *tok = parser.items[n_idx].tok.clone();
        if parser.status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }
        parser.exiting = 1;
        return MPACK_EOF;
    }

    parser.exiting = 0;
    while let Some(n_idx) = mpack_parser_pop(parser) {
        if let Some(cb) = exit_cb {
            cb(parser, ctx, n_idx);
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

pub fn mpack_parse<C: ?Sized>(
    parser: &mut MpackParser,
    ctx: &mut C,
    buf: &mut &[u8],
    enter_cb: MpackWalkCb<C>,
    exit_cb: MpackWalkCb<C>,
) -> i32 {
    let mut status = MPACK_EOF;
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    while !buf.is_empty() && status != 0 {
        let mut tok = MpackToken::new();
        let buf_save = *buf;

        status = mpack_read(&mut parser.tokbuf, buf, &mut tok);
        if status == MPACK_EOF {
            continue;
        } else if status == MPACK_ERROR {
            *buf = buf_save;
            break;
        }

        loop {
            status = mpack_parse_tok(parser, ctx, tok.clone(), enter_cb, exit_cb);
            if parser.status == MPACK_EXCEPTION {
                return MPACK_EXCEPTION;
            }
            if parser.exiting == 0 {
                break;
            }
        }

        if status == MPACK_NOMEM {
            *buf = buf_save;
            break;
        }
    }

    status
}

pub fn mpack_unparse<C: ?Sized>(
    parser: &mut MpackParser,
    ctx: &mut C,
    buf: &mut &mut [u8],
    enter_cb: MpackWalkCb<C>,
    exit_cb: MpackWalkCb<C>,
) -> i32 {
    let mut status = MPACK_EOF;
    if parser.status == MPACK_EXCEPTION {
        return MPACK_EXCEPTION;
    }

    while !buf.is_empty() && status != 0 {
        let mut tok = MpackToken::new();

        if parser.tokbuf.plen == 0 {
            parser.status = mpack_unparse_tok(parser, ctx, &mut tok, enter_cb, exit_cb);
        }

        if parser.status == MPACK_EXCEPTION {
            return MPACK_EXCEPTION;
        }

        status = parser.status;
        if status == MPACK_NOMEM {
            break;
        }

        if parser.exiting != 0 {
            let write_status = mpack_write(&mut parser.tokbuf, buf, &tok);
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

fn mpack_parser_push(parser: &mut MpackParser) -> usize {
    debug_assert!(parser.size < parser.capacity);
    let top_idx = (parser.size as usize) + 1;
    let top = &mut parser.items[top_idx];
    top.data[0] = MpackData::Null;
    top.data[1] = MpackData::Null;
    top.pos = 0;
    top.key_visited = 0;
    parser.size += 1;
    top_idx
}

fn mpack_parser_pop(parser: &mut MpackParser) -> Option<usize> {
    debug_assert!(parser.size != 0);
    let top_idx = parser.size as usize;
    let (top, parent) = parser.node_and_parent_mut(top_idx);

    if top.tok.r#type > MpackTokenChunk && top.pos < (top.tok.length as usize) {
        return None;
    }

    if let Some(parent) = parent {
        if top.tok.r#type == MpackTokenChunk {
            parent.pos += top.tok.length as usize;
        } else if parent.tok.r#type == MpackTokenMap {
            if parent.key_visited != 0 {
                parent.pos += 1;
            }
            parent.key_visited = i32::from(parent.key_visited == 0);
        } else {
            parent.pos += 1;
        }
    }

    parser.size -= 1;
    Some(top_idx)
}
