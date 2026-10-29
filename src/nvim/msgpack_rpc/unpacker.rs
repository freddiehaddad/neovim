//! Msgpack-RPC and ShaDa deserializer (`src/nvim/msgpack_rpc/unpacker.c`).

use crate::api::private::dispatch::{MsgpackRpcRequestHandler, msgpack_rpc_get_handler_for};
use crate::api::private::helpers::{api_set_error, nil_obj};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::globals::*;
use crate::lua::converter::{Boolean, FieldHashfn, OptKeySet};
use crate::memory::{Arena, arena_alloc, arena_finish, arena_mem_free};
use crate::mpack::mpack_core::{
    MPACK_OK, MpackTokbuf, MpackToken, MpackTokenType, mpack_read, mpack_rtoken, mpack_tokbuf_init,
};
use crate::mpack::object::{
    MpackNode, MpackParser, mpack_parent_node, mpack_parse, mpack_parser_init,
};
use crate::msgpack_rpc::packer::{EXT_OBJECT_TYPE_SHIFT, Integer};
use crate::ui_client::{
    UIClientHandler, handle_ui_client_redraw, ui_client_event_grid_line,
    ui_client_get_redraw_handler,
};
use std::ffi::c_char;

const MAX_EXT_LEN: usize = 9;
const EXT_OBJECT_TYPE_MAX: i32 = (ObjectType::ObjectTypeTabpage as i32) - EXT_OBJECT_TYPE_SHIFT;
const K_UNPACK_TYPE_STRING_ARRAY: i32 = -1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdditionalData {
    pub nitems: u32,
    pub bytes: Vec<u8>,
}

pub type AdditionalDataBuilder = Option<AdditionalData>;
pub type StringArray = Vec<NvimString>;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    MessageTypeRequest = 0,
    MessageTypeResponse = 1,
    MessageTypeNotification = 2,
    MessageTypeRedrawEvent = 3,
}

impl MessageType {
    #[inline]
    pub const fn from_raw(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::MessageTypeRequest),
            1 => Some(Self::MessageTypeResponse),
            2 => Some(Self::MessageTypeNotification),
            3 => Some(Self::MessageTypeRedrawEvent),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GridLineEvent {
    pub args: [i32; 3],
    pub icell: i32,
    pub ncells: i32,
    pub coloff: i32,
    pub cur_attr: i32,
    pub clear_width: i32,
    pub wrap: bool,
}

#[repr(C)]
pub struct Unpacker {
    pub parser: MpackParser,
    pub reader: MpackTokbuf,
    pub read_ptr: *const u8,
    pub read_size: usize,
    pub ext_buf: [u8; MAX_EXT_LEN],
    pub state: i32,
    pub msg_type: MessageType,
    pub request_id: u32,
    pub method_name_len: usize,
    pub handler: MsgpackRpcRequestHandler,
    pub error: Object,
    pub result: Object,
    pub unpack_error: Error,
    pub arena: Arena,
    pub nevents: i32,
    pub ncalls: i32,
    pub ui_handler: UIClientHandler,
    pub grid_line_event: GridLineEvent,
    pub has_grid_line_event: bool,
}

impl Default for Unpacker {
    fn default() -> Self {
        Self::new()
    }
}

use crate::eval::funcs::{MPACK_EOF, MPACK_ERROR, MPACK_NOMEM};
use crate::mpack::conv::{
    mpack_unpack_boolean, mpack_unpack_float_fast, mpack_unpack_sint, mpack_unpack_uint,
};

#[inline]
const fn empty_arena() -> Arena {
    Arena {
        cur_blk: std::ptr::null_mut(),
        pos: 0,
        size: 0,
    }
}

fn api_parse_enter(parser: &mut MpackParser, node: &mut MpackNode) {
    unsafe {
        let parser = &mut *parser;
        let node = &mut *node;
        let p = parser.data.unpacker();
        let mut result: *mut Object = std::ptr::null_mut();
        let mut key_location: *mut NvimString = std::ptr::null_mut();

        let parent = mpack_parent_node(&raw mut *node);
        if !parent.is_null() {
            let parent = &mut *parent;
            match parent.tok.r#type {
                MpackTokenType::MpackTokenArray => {
                    let obj = parent.data[0].object();
                    result = obj.data.array().items.wrapping_add(parent.pos);
                }
                MpackTokenType::MpackTokenMap => {
                    let obj = parent.data[0].object();
                    let kv = &mut obj.data.dict()[parent.pos];
                    if parent.key_visited == 0 {
                        kv.key = NvimString {
                            data: std::ptr::null_mut(),
                            size: 0,
                        };
                        key_location = &mut kv.key;
                    }
                    result = &mut kv.value;
                }
                MpackTokenType::MpackTokenStr
                | MpackTokenType::MpackTokenBin
                | MpackTokenType::MpackTokenExt => {
                    assert_eq!(node.tok.r#type, MpackTokenType::MpackTokenChunk);
                }
                _ => std::process::abort(),
            }
        } else {
            result = &mut p.result;
        }

        let tok = node.tok;
        match tok.r#type {
            MpackTokenType::MpackTokenNil => {
                *result = nil_obj();
            }
            MpackTokenType::MpackTokenBoolean => {
                *result = Object {
                    obj_type: ObjectType::ObjectTypeBoolean,
                    data: ObjectData::Boolean(mpack_unpack_boolean(tok)),
                };
            }
            MpackTokenType::MpackTokenSint => {
                *result = Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(mpack_unpack_sint(tok)),
                };
            }
            MpackTokenType::MpackTokenUint => {
                *result = Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(mpack_unpack_uint(tok) as Integer),
                };
            }
            MpackTokenType::MpackTokenFloat => {
                *result = Object {
                    obj_type: ObjectType::ObjectTypeFloat,
                    data: ObjectData::Floating(mpack_unpack_float_fast(tok)),
                };
            }
            MpackTokenType::MpackTokenBin | MpackTokenType::MpackTokenStr => {
                let len = tok.length as usize;
                let mem = arena_alloc(&mut p.arena, len + 1, false).cast::<c_char>();
                *mem.wrapping_add(len) = 0;
                let s = NvimString {
                    data: mem,
                    size: len,
                };
                if !key_location.is_null() {
                    *key_location = s;
                } else {
                    *result = Object {
                        obj_type: ObjectType::ObjectTypeString,
                        data: ObjectData::String(s),
                    };
                }
                node.data[0] = crate::mpack::object::MpackData::Bytes(mem.cast());
            }
            MpackTokenType::MpackTokenExt => {
                node.data[0] = crate::mpack::object::MpackData::Object(result);
            }
            MpackTokenType::MpackTokenChunk => {
                assert!(!parent.is_null());
                let chunk_len = tok.length as usize;
                if (*parent).tok.r#type == MpackTokenType::MpackTokenStr
                    || (*parent).tok.r#type == MpackTokenType::MpackTokenBin
                {
                    let data = (*parent).data[0].bytes();
                    if chunk_len > 0 {
                        std::ptr::copy_nonoverlapping(
                            tok.data.chunk_ptr().cast::<u8>(),
                            data.wrapping_add((*parent).pos),
                            chunk_len,
                        );
                    }
                } else {
                    let res = (*parent).data[0].object();
                    let endlen = (*parent).pos + chunk_len;
                    if endlen > MAX_EXT_LEN {
                        *res = nil_obj();
                        return;
                    }
                    if chunk_len > 0 {
                        std::ptr::copy_nonoverlapping(
                            tok.data.chunk_ptr().cast::<u8>(),
                            p.ext_buf
                                .as_mut_ptr()
                                .cast::<u8>()
                                .wrapping_add((*parent).pos),
                            chunk_len,
                        );
                    }
                    if endlen < ((*parent).tok.length as usize) {
                        return;
                    }
                    let mut buf: *const u8 = p.ext_buf.as_ptr();
                    let mut size = (*parent).tok.length as usize;
                    let mut ext_tok = MpackToken::new();
                    let status = mpack_rtoken(&mut buf, &mut size, &mut ext_tok);
                    if status != 0 || ext_tok.r#type != MpackTokenType::MpackTokenUint {
                        *res = nil_obj();
                        return;
                    }
                    let ext_type = (*parent).tok.data.ext_type();
                    if (0..=EXT_OBJECT_TYPE_MAX).contains(&ext_type)
                        && let Some(obj_type) =
                            ObjectType::from_raw(ext_type + EXT_OBJECT_TYPE_SHIFT)
                    {
                        res.obj_type = obj_type;
                        res.data = ObjectData::Integer(mpack_unpack_uint(ext_tok) as i64);
                    } else {
                        *res = nil_obj();
                    }
                }
            }
            MpackTokenType::MpackTokenArray => {
                let len = tok.length as usize;
                let items = arena_alloc(&mut p.arena, std::mem::size_of::<Object>() * len, true)
                    .cast::<Object>();
                *result = Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(Array {
                        size: len,
                        capacity: len,
                        items,
                    }),
                };
                node.data[0] = crate::mpack::object::MpackData::Object(result);
            }
            MpackTokenType::MpackTokenMap => {
                let len = tok.length as usize;
                let items = arena_alloc(
                    &mut p.arena,
                    std::mem::size_of::<KeyValuePair>() * len,
                    true,
                )
                .cast::<KeyValuePair>();
                *result = Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(Dict {
                        size: len,
                        capacity: len,
                        items,
                    }),
                };
                node.data[0] = crate::mpack::object::MpackData::Object(result);
            }
            MpackTokenType::MpackTokenNone => std::process::abort(),
        }
    }
}

fn api_parse_exit(_parser: &mut MpackParser, _node: &mut MpackNode) {}

fn parse_nop(_parser: &mut MpackParser, _node: &mut MpackNode) {}

/// Unpack a single MessagePack value into an `Object` (`unpack`).
pub fn unpack(
    data: impl crate::strings::IntoCCharPtr,
    mut size: usize,
    arena: &mut Arena,
    err: &mut Error,
) -> Object {
    let mut data = data.into_c_char_ptr().cast::<u8>();
    let mut unpacker = Unpacker::new();
    mpack_parser_init(&mut unpacker.parser, 0);
    unpacker.parser.data = crate::mpack::object::MpackData::Unpacker(&mut unpacker);
    unpacker.arena = *arena;

    let result = mpack_parse(
        &mut unpacker.parser,
        &mut data,
        &mut size,
        Some(api_parse_enter),
        Some(api_parse_exit),
    );

    *arena = std::mem::replace(&mut unpacker.arena, empty_arena());

    if result == MPACK_NOMEM {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"object was too deep to unpack",
        );
    } else if result == MPACK_EOF {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"incomplete msgpack string",
        );
    } else if result == MPACK_ERROR {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"invalid msgpack string",
        );
    } else if result == MPACK_OK && size != 0 {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"trailing data in msgpack string",
        );
    }

    unpacker.result
}

/// Initialize an `Unpacker` (`unpacker_init`).
impl Unpacker {
    pub const fn new() -> Self {
        Self {
            parser: MpackParser::new(0),
            reader: MpackTokbuf::new(),
            read_ptr: std::ptr::null(),
            read_size: 0,
            ext_buf: [0; MAX_EXT_LEN],
            state: 0,
            msg_type: MessageType::MessageTypeRequest,
            request_id: 0,
            method_name_len: 0,
            handler: MsgpackRpcRequestHandler {
                name: c"",
                r#fn: None,
                fast: false,
                ret_alloc: false,
            },
            error: nil_obj(),
            result: nil_obj(),
            unpack_error: Error::new(),
            arena: empty_arena(),
            nevents: 0,
            ncalls: 0,
            ui_handler: UIClientHandler {
                name: c"",
                r#fn: None,
            },
            grid_line_event: GridLineEvent {
                args: [0; 3],
                icell: 0,
                ncells: 0,
                coloff: 0,
                cur_attr: 0,
                clear_width: 0,
                wrap: false,
            },
            has_grid_line_event: false,
        }
    }
}

impl Drop for Unpacker {
    fn drop(&mut self) {
        let mem = arena_finish(&mut self.arena);
        arena_mem_free(mem);
    }
}

pub fn unpacker_init(p: &mut Unpacker) {
    let p_ptr = &raw mut *p;
    mpack_parser_init(&mut p.parser, 0);
    p.parser.data = crate::mpack::object::MpackData::Unpacker(p_ptr);
    mpack_tokbuf_init(&mut p.reader);
    p.unpack_error = Error::new();
    p.arena = empty_arena();
    p.has_grid_line_event = false;
}

/// Parse the header of a msgpack-rpc message (`unpacker_parse_header`).
pub fn unpacker_parse_header(p: &mut Unpacker) -> bool {
    let mut tok = MpackToken::new();
    let mut data = p.read_ptr;
    let mut size = p.read_size;

    assert_eq!(p.unpack_error.err_type, ErrorType::ErrorTypeNone);

    let mut result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
    if result == 0
        && (tok.r#type != MpackTokenType::MpackTokenArray || tok.length < 3 || tok.length > 4)
    {
        result = MPACK_ERROR;
    }
    if result == 0 {
        let array_length = tok.length as usize;
        result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
        if result == 0 && tok.r#type != MpackTokenType::MpackTokenUint {
            result = MPACK_ERROR;
        }
        if result == 0 {
            let msg_type = mpack_unpack_uint(tok) as u32;
            let bad_type = if array_length == 3 {
                msg_type != 2
            } else {
                msg_type >= 2
            };
            let Some(mt) = (if bad_type {
                None
            } else {
                MessageType::from_raw(msg_type as i32)
            }) else {
                api_set_error!(
                    &mut p.unpack_error,
                    ErrorType::ErrorTypeValidation,
                    c"failed to decode msgpack",
                );
                p.state = -1;
                return false;
            };
            p.msg_type = mt;
            p.request_id = 0;

            if p.msg_type != MessageType::MessageTypeNotification {
                result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
                if result == 0 && tok.r#type != MpackTokenType::MpackTokenUint {
                    result = MPACK_ERROR;
                }
                if result == 0 {
                    p.request_id = mpack_unpack_uint(tok) as u32;
                }
            }

            if result == 0 && p.msg_type != MessageType::MessageTypeResponse {
                result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
                if result == 0
                    && ((tok.r#type != MpackTokenType::MpackTokenStr
                        && tok.r#type != MpackTokenType::MpackTokenBin)
                        || tok.length > 100)
                {
                    result = MPACK_ERROR;
                }
                if result == 0 {
                    p.method_name_len = tok.length as usize;
                    if p.method_name_len > 0 {
                        result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
                        if result == 0 {
                            assert_eq!(tok.r#type, MpackTokenType::MpackTokenChunk);
                        }
                    }
                    if result == 0 && (tok.length as usize) < p.method_name_len {
                        result = MPACK_EOF;
                    }
                    if result == 0 {
                        let name_ptr = if tok.length != 0 {
                            tok.data.chunk_ptr().cast::<c_char>()
                        } else {
                            c"".as_ptr()
                        };
                        p.handler = msgpack_rpc_get_handler_for(
                            name_ptr,
                            tok.length as usize,
                            &mut p.unpack_error,
                        );
                    }
                }
            }

            if result == 0 {
                p.read_ptr = data;
                p.read_size = size;
                return true;
            }
        }
    }

    if result == MPACK_EOF {
        mpack_tokbuf_init(&mut p.reader);
    } else {
        api_set_error!(
            &mut p.unpack_error,
            ErrorType::ErrorTypeValidation,
            c"failed to decode msgpack",
        );
        p.state = -1;
    }
    false
}

/// Advance the unpacker state machine (`unpacker_advance`).
pub fn unpacker_advance(p: &mut Unpacker) -> bool {
    p.parser.data = crate::mpack::object::MpackData::Unpacker(&raw mut *p);
    assert!(p.state >= 0);
    p.has_grid_line_event = false;
    if p.state == 0 {
        if !unpacker_parse_header(p) {
            return false;
        }
        if p.msg_type == MessageType::MessageTypeNotification
            && p.handler
                .r#fn
                .is_some_and(|f| f as *const () == handle_ui_client_redraw as *const ())
        {
            p.msg_type = MessageType::MessageTypeRedrawEvent;
            p.state = 10;
        } else {
            p.state = if p.msg_type == MessageType::MessageTypeResponse {
                1
            } else {
                2
            };
            p.arena = empty_arena();
        }
    }

    let mut need_parse = true;
    if p.state >= 10 && p.state != 13 {
        if !unpacker_parse_redraw(p) {
            if p.state < 0 {
                api_set_error!(
                    &mut p.unpack_error,
                    ErrorType::ErrorTypeValidation,
                    c"failed to parse redraw event",
                );
            }
            return false;
        }

        if p.state == 16 {
            p.has_grid_line_event = true;
            need_parse = false;
        } else {
            assert_eq!(p.state, 12);
            p.arena = empty_arena();
            p.state = 13;
        }
    }

    loop {
        if need_parse {
            let result = mpack_parse(
                &mut p.parser,
                &mut p.read_ptr,
                &mut p.read_size,
                Some(api_parse_enter),
                Some(api_parse_exit),
            );
            if result == MPACK_EOF {
                return false;
            } else if result != MPACK_OK {
                api_set_error!(
                    &mut p.unpack_error,
                    ErrorType::ErrorTypeValidation,
                    c"failed to parse msgpack",
                );
                p.state = -1;
                return false;
            }
        }

        match p.state {
            1 => {
                p.error = p.result;
                p.state = 2;
                need_parse = true;
            }
            2 => {
                p.state = 0;
                return true;
            }
            13 | 16 => {
                p.ncalls -= 1;
                if p.ncalls > 0 {
                    p.state = if p.state == 16 { 14 } else { 12 };
                } else if p.nevents > 0 {
                    p.state = 11;
                } else {
                    p.state = 0;
                }
                return true;
            }
            _ => std::process::abort(),
        }
    }
}

#[inline]
fn next_type(
    p: &mut Unpacker,
    data: &mut *const u8,
    size: &mut usize,
    tok: &mut MpackToken,
    expected: MpackTokenType,
) -> bool {
    let result = mpack_rtoken(data, size, tok);
    if result == MPACK_EOF {
        return false;
    }
    let valid_type = tok.r#type == expected
        || (expected == MpackTokenType::MpackTokenStr
            && tok.r#type == MpackTokenType::MpackTokenBin)
        || (expected == MpackTokenType::MpackTokenSint
            && tok.r#type == MpackTokenType::MpackTokenUint);
    if result != 0 || !valid_type {
        p.state = -1;
        return false;
    }
    true
}

/// Specialized redraw/grid_line parser (`unpacker_parse_redraw`).
pub fn unpacker_parse_redraw(p: &mut Unpacker) -> bool {
    let mut tok = MpackToken::new();
    let mut data = p.read_ptr;
    let mut size = p.read_size;

    if p.state == 10 {
        if !next_type(
            p,
            &mut data,
            &mut size,
            &mut tok,
            MpackTokenType::MpackTokenArray,
        ) {
            return false;
        }
        p.nevents = tok.length as i32;
        p.state = 11;
    }

    if p.state == 11 {
        if !next_type(
            p,
            &mut data,
            &mut size,
            &mut tok,
            MpackTokenType::MpackTokenArray,
        ) {
            return false;
        }
        p.ncalls = tok.length as i32;
        let prev_ncalls = p.ncalls;
        p.ncalls -= 1;
        if prev_ncalls == 0 {
            p.state = -1;
            return false;
        }

        if !next_type(
            p,
            &mut data,
            &mut size,
            &mut tok,
            MpackTokenType::MpackTokenStr,
        ) {
            return false;
        }
        let str_len = tok.length as usize;
        if str_len > size {
            return false;
        }

        p.ui_handler = ui_client_get_redraw_handler(data.cast::<c_char>(), str_len);
        data = data.wrapping_add(str_len);
        size -= str_len;

        p.nevents -= 1;
        p.read_ptr = data;
        p.read_size = size;
        if p.ui_handler
            .r#fn
            .is_none_or(|f| !std::ptr::fn_addr_eq(f, ui_client_event_grid_line as fn(Array)))
        {
            p.state = 12;
            return true;
        } else {
            p.state = 14;
            p.arena = empty_arena();
        }
    }

    if p.state == 14 {
        if !next_type(
            p,
            &mut data,
            &mut size,
            &mut tok,
            MpackTokenType::MpackTokenArray,
        ) {
            return false;
        }
        let eventarrsize = tok.length as i32;
        if eventarrsize != 5 {
            p.state = -1;
            return false;
        }

        for i in 0..3 {
            if !next_type(
                p,
                &mut data,
                &mut size,
                &mut tok,
                MpackTokenType::MpackTokenUint,
            ) {
                return false;
            }
            p.grid_line_event.args[i] = tok.data.value().lo as i32;
        }

        if !next_type(
            p,
            &mut data,
            &mut size,
            &mut tok,
            MpackTokenType::MpackTokenArray,
        ) {
            return false;
        }
        p.grid_line_event.ncells = tok.length as i32;
        p.grid_line_event.icell = 0;
        p.grid_line_event.coloff = 0;
        p.grid_line_event.cur_attr = -1;

        p.read_ptr = data;
        p.read_size = size;
        p.state = 15;
    }

    if p.state == 15 {
        while p.grid_line_event.icell != p.grid_line_event.ncells {
            assert!(p.grid_line_event.icell < p.grid_line_event.ncells);

            if !next_type(
                p,
                &mut data,
                &mut size,
                &mut tok,
                MpackTokenType::MpackTokenArray,
            ) {
                return false;
            }
            let cellarrsize = tok.length as i32;
            if !(1..=3).contains(&cellarrsize) {
                p.state = -1;
                return false;
            }

            if !next_type(
                p,
                &mut data,
                &mut size,
                &mut tok,
                MpackTokenType::MpackTokenStr,
            ) {
                return false;
            }
            let cellsize = tok.length as usize;
            if cellsize > size {
                return false;
            }

            let cellbuf = data;
            data = data.wrapping_add(cellsize);
            size -= cellsize;

            if cellarrsize >= 2 {
                if !next_type(
                    p,
                    &mut data,
                    &mut size,
                    &mut tok,
                    MpackTokenType::MpackTokenSint,
                ) {
                    return false;
                }
                p.grid_line_event.cur_attr = tok.data.value().lo as i32;
            }

            let mut repeat = 1_i32;
            if cellarrsize >= 3 {
                if !next_type(
                    p,
                    &mut data,
                    &mut size,
                    &mut tok,
                    MpackTokenType::MpackTokenUint,
                ) {
                    return false;
                }
                repeat = tok.data.value().lo as i32;
            }

            p.grid_line_event.clear_width = 0;
            let cell_bytes = unsafe { std::slice::from_raw_parts(cellbuf, cellsize) };
            if p.grid_line_event.icell == p.grid_line_event.ncells - 1
                && cell_bytes == b" "
                && repeat > 1
            {
                p.grid_line_event.clear_width = repeat;
            } else {
                let sc = crate::grid::schar_from_bytes(cell_bytes);
                for _ in 0..repeat {
                    if p.grid_line_event.coloff >= (g_st().grid_line_buf_char.len() as i32) {
                        p.state = -1;
                        return false;
                    }
                    let off = p.grid_line_event.coloff as usize;
                    g_st().grid_line_buf_char[off] = sc;
                    g_st().grid_line_buf_attr[off] = p.grid_line_event.cur_attr;
                    p.grid_line_event.coloff += 1;
                }
            }

            p.read_ptr = data;
            p.read_size = size;
            p.grid_line_event.icell += 1;
        }
        p.state = 16;
    }

    if p.state == 16 {
        if !next_type(
            p,
            &mut data,
            &mut size,
            &mut tok,
            MpackTokenType::MpackTokenBoolean,
        ) {
            return false;
        }
        p.grid_line_event.wrap = mpack_unpack_boolean(tok);
        p.read_ptr = data;
        p.read_size = size;
        return true;
    }

    if p.state == 12 {
        return true;
    }

    std::process::abort()
}

/// Unpack a complete msgpack string/bin token (`unpack_string`).
pub fn unpack_string(data: &mut *const c_char, size: &mut usize) -> NvimString {
    let mut data2 = (*data).cast::<u8>();
    let mut size2 = *size;
    let mut tok = MpackToken::new();

    let result = mpack_rtoken(&mut data2, &mut size2, &mut tok);
    if result != 0
        || (tok.r#type != MpackTokenType::MpackTokenStr
            && tok.r#type != MpackTokenType::MpackTokenBin)
    {
        return NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        };
    }
    let len = tok.length as usize;
    if size2 < len {
        return NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        };
    }
    *data = data2.wrapping_add(len).cast::<c_char>();
    *size = size2 - len;
    NvimString {
        data: data2.cast::<c_char>().cast_mut(),
        size: len,
    }
}

/// Unpack a msgpack array header (`unpack_array`).
pub fn unpack_array(data: &mut *const c_char, size: &mut usize) -> isize {
    let mut tok = MpackToken::new();
    let mut u8_ptr = (*data).cast::<u8>();
    let result = mpack_rtoken(&mut u8_ptr, size, &mut tok);
    *data = u8_ptr.cast::<c_char>();
    if result != 0 || tok.r#type != MpackTokenType::MpackTokenArray {
        return -1;
    }
    tok.length as isize
}

/// Unpack a msgpack integer (`unpack_integer`).
pub fn unpack_integer(data: &mut *const c_char, size: &mut usize, res: &mut Integer) -> bool {
    let mut tok = MpackToken::new();
    let mut u8_ptr = (*data).cast::<u8>();
    let result = mpack_rtoken(&mut u8_ptr, size, &mut tok);
    *data = u8_ptr.cast::<c_char>();
    if result != 0 {
        return false;
    }
    unpack_uint_or_sint(tok, res)
}

/// Unpack a uint or sint token into `Integer` (`unpack_uint_or_sint`).
pub fn unpack_uint_or_sint(tok: MpackToken, res: &mut Integer) -> bool {
    if tok.r#type == MpackTokenType::MpackTokenUint {
        *res = mpack_unpack_uint(tok) as Integer;
        true
    } else if tok.r#type == MpackTokenType::MpackTokenSint {
        *res = mpack_unpack_sint(tok) as Integer;
        true
    } else {
        false
    }
}

/// Skip one msgpack value (`unpack_skip`).
pub fn unpack_skip(data: &mut *const c_char, size: &mut usize) -> i32 {
    let mut parser = MpackParser::new(0);
    mpack_parser_init(&mut parser, 0);
    let mut u8_ptr = (*data).cast::<u8>();
    let ret = mpack_parse(
        &mut parser,
        &mut u8_ptr,
        size,
        Some(parse_nop),
        Some(parse_nop),
    );
    *data = u8_ptr.cast::<c_char>();
    ret
}

/// Append raw msgpack key-value bytes to `AdditionalDataBuilder` (`push_additional_data`).
pub fn push_additional_data(ad: &mut AdditionalDataBuilder, bytes: &[u8]) {
    let entry = ad.get_or_insert_with(AdditionalData::default);
    entry.nitems += 1;
    entry.bytes.extend_from_slice(bytes);
}

#[inline]
pub fn additional_data_take(ad: &mut AdditionalDataBuilder) -> Option<AdditionalData> {
    ad.take()
}

/// Unpack a msgpack map into a `KeySet` struct (`unpack_keydict`).
pub fn unpack_keydict(
    retval: *mut OptKeySet,
    hashy: FieldHashfn,
    ad: &mut AdditionalDataBuilder,
    data: &mut *const c_char,
    size: &mut usize,
    error: &mut Option<std::ffi::CString>,
) -> bool {
    unsafe {
        let ks = retval;
        let mut tok = MpackToken::new();

        let mut u8_ptr = (*data).cast::<u8>();
        let result = mpack_rtoken(&mut u8_ptr, size, &mut tok);
        *data = u8_ptr.cast::<c_char>();
        if result != 0 || tok.r#type != MpackTokenType::MpackTokenMap {
            *error = Some(c"is not a dict".to_owned());
            return false;
        }

        let map_size = tok.length as usize;
        let hashy_fn = hashy.unwrap();

        let alloc_key_error = |key: NvimString, suffix: &[u8]| -> Option<std::ffi::CString> {
            let key_bytes = crate::memory::cstr_slice(key.data, key.size);
            let mut out = Vec::with_capacity(4 + key.size + suffix.len());
            out.extend_from_slice(b"has ");
            out.extend_from_slice(key_bytes);
            out.extend_from_slice(suffix);
            Some(std::ffi::CString::new(out).unwrap_or_default())
        };

        for _ in 0..map_size {
            let item_start = *data;
            let key = unpack_string(data, size);
            if key.data.is_null() {
                *error = Some(c"has key value which is not a string".to_owned());
                return false;
            } else if key.size == 0 {
                *error = Some(c"has empty key".to_owned());
                return false;
            }
            let key_bytes = crate::memory::cstr_slice(key.data, key.size);
            let field_opt = hashy_fn(key_bytes);

            let Some(field) = field_opt else {
                let status = unpack_skip(data, size);
                if status != 0 {
                    return false;
                }
                let skipped =
                    crate::memory::cstr_slice(item_start, (*data as usize) - (item_start as usize));
                push_additional_data(ad, skipped);
                continue;
            };

            assert!(field.opt_index >= 0);
            let flag = 1u64 << (field.opt_index as u32);
            if ((*ks).is_set_ & flag) != 0 {
                *error = Some(c"duplicate key".to_owned());
                return false;
            }
            (*ks).is_set_ |= flag;

            let mem = retval.cast::<u8>().wrapping_add(field.ptr_off);
            if field.key_type == (ObjectType::ObjectTypeBoolean as i32) {
                let cur_byte = if *size > 0 { **data as u8 } else { 0 };
                if *size == 0 || (cur_byte & 0xfe) != 0xc2 {
                    *error = alloc_key_error(key, b" key value which is not a boolean");
                    return false;
                }
                *mem.cast::<Boolean>() = (cur_byte & 0x01) != 0;
                *data = (*data).wrapping_add(1);
                *size -= 1;
            } else if field.key_type == (ObjectType::ObjectTypeInteger as i32) {
                if !unpack_integer(data, size, &mut *mem.cast::<Integer>()) {
                    *error = alloc_key_error(key, b" key value which is not an integer");
                    return false;
                }
            } else if field.key_type == (ObjectType::ObjectTypeString as i32) {
                let val = unpack_string(data, size);
                if val.data.is_null() {
                    *error = alloc_key_error(key, b" key value which is not a binary");
                    return false;
                }
                *mem.cast::<NvimString>() = val;
            } else if field.key_type == K_UNPACK_TYPE_STRING_ARRAY {
                let len = unpack_array(data, size);
                if len < 0 {
                    *error = alloc_key_error(key, b" key with non-array value");
                    return false;
                }
                let a = &mut *mem.cast::<StringArray>();
                a.reserve(len as usize);
                for _ in 0..(len as usize) {
                    let item = unpack_string(data, size);
                    if item.data.is_null() {
                        *error = alloc_key_error(key, b" array with non-binary value");
                        return false;
                    }
                    a.push(item);
                }
            } else {
                std::process::abort();
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_additional_data_populates_header_and_payload() {
        let mut ad = AdditionalDataBuilder::default();
        push_additional_data(&mut ad, b"abc");
        push_additional_data(&mut ad, b"de");
        let hdr = ad.as_ref().unwrap();
        assert_eq!(hdr.nitems, 2);
        assert_eq!(hdr.bytes.len(), 5);
    }

    #[test]
    fn unpacker_handles_paused_and_empty_grid_line() {
        let payload1: &[u8] = b"\x93\x02\xa6\x72\x65\x64\x72\x61\x77\x91\x92\xa9\x67\x72\x69\x64\x5f\x6c\x69\x6e\x65\x95\x02\x00\x00\x91\x93\xa1\x20\x00\x4d\xc2";
        let mut unpacker = Unpacker::new();
        unpacker_init(&mut unpacker);
        unpacker.read_ptr = payload1.as_ptr().cast();
        unpacker.read_size = payload1.len() - 1;
        assert!(!unpacker_advance(&mut unpacker));
        unpacker.read_size += 1;
        assert!(unpacker_advance(&mut unpacker));

        let payload2: &[u8] = b"\x93\x02\xa6\x72\x65\x64\x72\x61\x77\x91\x92\xa9\x67\x72\x69\x64\x5f\x6c\x69\x6e\x65\x95\x02\x00\x00\x90\xc2";
        unpacker = Unpacker::new();
        unpacker_init(&mut unpacker);
        unpacker.read_ptr = payload2.as_ptr().cast();
        unpacker.read_size = payload2.len();
        assert!(unpacker_advance(&mut unpacker));
    }
}
