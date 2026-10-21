//! Msgpack-RPC and ShaDa deserializer (`src/nvim/msgpack_rpc/unpacker.c`).

#![allow(non_camel_case_types, non_snake_case, clippy::missing_safety_doc)]

#[cfg(not(test))]
use crate::api::private::dispatch::msgpack_rpc_get_handler_for;
use crate::api::private::helpers::{api_set_error, nil_obj};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::grid::schar_from_buf;
use crate::kvec::KVec;
use crate::lua::converter::FieldHashfn;
#[cfg(not(test))]
use crate::lua::converter::{Boolean, OptKeySet};
#[cfg(not(test))]
use crate::memory::xstrdup;
use crate::memory::{Arena, arena_alloc, arena_finish, arena_mem_free};
use crate::mpack::mpack_core::{
    MPACK_OK, mpack_rtoken, mpack_tokbuf_t, mpack_token_t, mpack_token_type_t,
};
use crate::mpack::object::{
    mpack_node_t, mpack_parent_node, mpack_parse, mpack_parser_init, mpack_parser_t,
};
use crate::msgpack_rpc::packer::{EXT_OBJECT_TYPE_SHIFT, Integer};
#[cfg(not(test))]
use crate::ui_client::{
    handle_ui_client_redraw, ui_client_event_grid_line, ui_client_get_redraw_handler,
};
use std::ffi::{c_char, c_int, c_void};

const MAX_EXT_LEN: usize = 9;
const EXT_OBJECT_TYPE_MAX: c_int =
    (ObjectType::kObjectTypeTabpage as c_int) - EXT_OBJECT_TYPE_SHIFT;
#[cfg(not(test))]
const K_UNPACK_TYPE_STRING_ARRAY: c_int = -1;

pub type AdditionalDataBuilder = KVec<c_char>;
pub type StringArray = KVec<NvimString>;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    kMessageTypeUnknown = -1,
    kMessageTypeRequest = 0,
    kMessageTypeResponse = 1,
    kMessageTypeNotification = 2,
    kMessageTypeRedrawEvent = 3,
}

use crate::api::private::dispatch::MsgpackRpcRequestHandler;
use crate::ui_client::UIClientHandler;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GridLineEvent {
    pub args: [c_int; 3],
    pub icell: c_int,
    pub ncells: c_int,
    pub coloff: c_int,
    pub cur_attr: c_int,
    pub clear_width: c_int,
    pub wrap: bool,
}

#[repr(C)]
pub struct Unpacker {
    pub parser: mpack_parser_t,
    pub reader: mpack_tokbuf_t,
    pub read_ptr: *const c_char,
    pub read_size: usize,
    pub ext_buf: [c_char; MAX_EXT_LEN],
    pub state: c_int,
    pub msg_type: MessageType,
    pub request_id: u32,
    pub method_name_len: usize,
    pub handler: MsgpackRpcRequestHandler,
    pub error: Object,
    pub result: Object,
    pub unpack_error: Error,
    pub arena: Arena,
    pub nevents: c_int,
    pub ncalls: c_int,
    pub ui_handler: UIClientHandler,
    pub grid_line_event: GridLineEvent,
    pub has_grid_line_event: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AdditionalDataHeader {
    pub nitems: u32,
    pub nbytes: u32,
}

use crate::eval::funcs::{MPACK_EOF, MPACK_ERROR, MPACK_NOMEM};
use crate::mpack::conv::{
    mpack_unpack_boolean, mpack_unpack_float_fast, mpack_unpack_sint, mpack_unpack_uint,
};
#[cfg(not(test))]
use crate::mpack::mpack_core::{mpack_read, mpack_tokbuf_init};

#[cfg(not(test))]
#[inline]
const fn empty_arena() -> Arena {
    Arena {
        cur_blk: std::ptr::null_mut(),
        pos: 0,
        size: 0,
    }
}

unsafe extern "C-unwind" fn api_parse_enter(parser: *mut mpack_parser_t, node: *mut mpack_node_t) {
    unsafe {
        let p = (*parser).data.p.cast::<Unpacker>();
        let mut result: *mut Object = std::ptr::null_mut();
        let mut key_location: *mut NvimString = std::ptr::null_mut();

        let parent = mpack_parent_node(node);
        if !parent.is_null() {
            match (*parent).tok.r#type {
                mpack_token_type_t::MPACK_TOKEN_ARRAY => {
                    let obj = (*parent).data[0].p.cast::<Object>();
                    result = (*obj).data.array.items.add((*parent).pos);
                }
                mpack_token_type_t::MPACK_TOKEN_MAP => {
                    let obj = (*parent).data[0].p.cast::<Object>();
                    let kv = &mut *(*obj).data.dict.items.add((*parent).pos);
                    if (*parent).key_visited == 0 {
                        kv.key = NvimString {
                            data: std::ptr::null_mut(),
                            size: 0,
                        };
                        key_location = &mut kv.key;
                    }
                    result = &mut kv.value;
                }
                mpack_token_type_t::MPACK_TOKEN_STR
                | mpack_token_type_t::MPACK_TOKEN_BIN
                | mpack_token_type_t::MPACK_TOKEN_EXT => {
                    assert_eq!((*node).tok.r#type, mpack_token_type_t::MPACK_TOKEN_CHUNK);
                }
                _ => std::process::abort(),
            }
        } else {
            result = &mut (*p).result;
        }

        let tok = (*node).tok;
        match tok.r#type {
            mpack_token_type_t::MPACK_TOKEN_NIL => {
                *result = nil_obj();
            }
            mpack_token_type_t::MPACK_TOKEN_BOOLEAN => {
                *result = Object {
                    obj_type: ObjectType::kObjectTypeBoolean,
                    data: ObjectData {
                        boolean: mpack_unpack_boolean(tok),
                    },
                };
            }
            mpack_token_type_t::MPACK_TOKEN_SINT => {
                *result = Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: mpack_unpack_sint(tok),
                    },
                };
            }
            mpack_token_type_t::MPACK_TOKEN_UINT => {
                *result = Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: mpack_unpack_uint(tok) as Integer,
                    },
                };
            }
            mpack_token_type_t::MPACK_TOKEN_FLOAT => {
                *result = Object {
                    obj_type: ObjectType::kObjectTypeFloat,
                    data: ObjectData {
                        floating: mpack_unpack_float_fast(tok),
                    },
                };
            }
            mpack_token_type_t::MPACK_TOKEN_BIN | mpack_token_type_t::MPACK_TOKEN_STR => {
                let len = tok.length as usize;
                let mem = arena_alloc(&mut (*p).arena, len + 1, false).cast::<c_char>();
                *mem.add(len) = 0;
                let s = NvimString {
                    data: mem,
                    size: len,
                };
                if !key_location.is_null() {
                    *key_location = s;
                } else {
                    *result = Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData { string: s },
                    };
                }
                (*node).data[0].p = mem.cast::<c_void>();
            }
            mpack_token_type_t::MPACK_TOKEN_EXT => {
                (*node).data[0].p = result.cast::<c_void>();
            }
            mpack_token_type_t::MPACK_TOKEN_CHUNK => {
                assert!(!parent.is_null());
                let chunk_len = tok.length as usize;
                if (*parent).tok.r#type == mpack_token_type_t::MPACK_TOKEN_STR
                    || (*parent).tok.r#type == mpack_token_type_t::MPACK_TOKEN_BIN
                {
                    let data = (*parent).data[0].p.cast::<u8>();
                    if chunk_len > 0 {
                        std::ptr::copy_nonoverlapping(
                            tok.data.chunk_ptr.cast::<u8>(),
                            data.add((*parent).pos),
                            chunk_len,
                        );
                    }
                } else {
                    let res = (*parent).data[0].p.cast::<Object>();
                    let endlen = (*parent).pos + chunk_len;
                    if endlen > MAX_EXT_LEN {
                        *res = nil_obj();
                        return;
                    }
                    if chunk_len > 0 {
                        std::ptr::copy_nonoverlapping(
                            tok.data.chunk_ptr.cast::<u8>(),
                            (*p).ext_buf.as_mut_ptr().cast::<u8>().add((*parent).pos),
                            chunk_len,
                        );
                    }
                    if endlen < ((*parent).tok.length as usize) {
                        return;
                    }
                    let mut buf: *const c_char = (*p).ext_buf.as_ptr();
                    let mut size = (*parent).tok.length as usize;
                    let mut ext_tok = std::mem::zeroed::<mpack_token_t>();
                    let status = mpack_rtoken(&mut buf, &mut size, &mut ext_tok);
                    if status != 0 || ext_tok.r#type != mpack_token_type_t::MPACK_TOKEN_UINT {
                        *res = nil_obj();
                        return;
                    }
                    let ext_type = (*parent).tok.data.ext_type;
                    if (0..=EXT_OBJECT_TYPE_MAX).contains(&ext_type) {
                        (*res).obj_type = std::mem::transmute::<c_int, ObjectType>(
                            ext_type + EXT_OBJECT_TYPE_SHIFT,
                        );
                        (*res).data.integer = mpack_unpack_uint(ext_tok) as i64;
                    } else {
                        *res = nil_obj();
                    }
                }
            }
            mpack_token_type_t::MPACK_TOKEN_ARRAY => {
                let len = tok.length as usize;
                let items = arena_alloc(&mut (*p).arena, std::mem::size_of::<Object>() * len, true)
                    .cast::<Object>();
                *result = Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData {
                        array: Array {
                            size: len,
                            capacity: len,
                            items,
                        },
                    },
                };
                (*node).data[0].p = result.cast::<c_void>();
            }
            mpack_token_type_t::MPACK_TOKEN_MAP => {
                let len = tok.length as usize;
                let items = arena_alloc(
                    &mut (*p).arena,
                    std::mem::size_of::<KeyValuePair>() * len,
                    true,
                )
                .cast::<KeyValuePair>();
                *result = Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: Dict {
                            size: len,
                            capacity: len,
                            items,
                        },
                    },
                };
                (*node).data[0].p = result.cast::<c_void>();
            }
            mpack_token_type_t::MPACK_TOKEN_NONE => std::process::abort(),
        }
    }
}

unsafe extern "C-unwind" fn api_parse_exit(_parser: *mut mpack_parser_t, _node: *mut mpack_node_t) {
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parse_nop(_parser: *mut mpack_parser_t, _node: *mut mpack_node_t) {}

/// Unpack a single MessagePack value into an `Object` (`unpack`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpack(
    mut data: *const c_char,
    mut size: usize,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut unpacker = std::mem::zeroed::<Unpacker>();
        mpack_parser_init(&mut unpacker.parser, 0);
        unpacker.parser.data.p = (&mut unpacker as *mut Unpacker).cast::<c_void>();
        unpacker.arena = *arena;

        let result = mpack_parse(
            &mut unpacker.parser,
            &mut data,
            &mut size,
            Some(api_parse_enter),
            Some(api_parse_exit),
        );

        *arena = unpacker.arena;

        if result == MPACK_NOMEM {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"object was too deep to unpack".as_ptr(),
            );
        } else if result == MPACK_EOF {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"incomplete msgpack string".as_ptr(),
            );
        } else if result == MPACK_ERROR {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"invalid msgpack string".as_ptr(),
            );
        } else if result == MPACK_OK && size != 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"trailing data in msgpack string".as_ptr(),
            );
        }

        unpacker.result
    }
}

/// Initialize an `Unpacker` (`unpacker_init`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpacker_init(p: *mut Unpacker) {
    #[cfg(test)]
    let _ = p;

    #[cfg(not(test))]
    unsafe {
        let p_ref = &mut *p;
        mpack_parser_init(&mut p_ref.parser, 0);
        p_ref.parser.data.p = p.cast::<c_void>();
        mpack_tokbuf_init(&mut p_ref.reader);
        p_ref.unpack_error = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: std::ptr::null_mut(),
        };
        p_ref.arena = empty_arena();
        p_ref.has_grid_line_event = false;
    }
}

/// Teardown an `Unpacker` (`unpacker_teardown`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpacker_teardown(p: *mut Unpacker) {
    unsafe {
        let mem = arena_finish(&mut (*p).arena);
        arena_mem_free(mem);
    }
}

/// Parse the header of a msgpack-rpc message (`unpacker_parse_header`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpacker_parse_header(p: *mut Unpacker) -> bool {
    #[cfg(test)]
    {
        let _ = p;
        false
    }
    #[cfg(not(test))]
    unsafe {
        let p = &mut *p;
        let mut tok = std::mem::zeroed::<mpack_token_t>();
        let mut data = p.read_ptr;
        let mut size = p.read_size;

        assert_eq!(p.unpack_error.err_type, ErrorType::kErrorTypeNone);

        let mut result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
        if result == 0
            && (tok.r#type != mpack_token_type_t::MPACK_TOKEN_ARRAY
                || tok.length < 3
                || tok.length > 4)
        {
            result = MPACK_ERROR;
        }
        if result == 0 {
            let array_length = tok.length as usize;
            result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
            if result == 0 && tok.r#type != mpack_token_type_t::MPACK_TOKEN_UINT {
                result = MPACK_ERROR;
            }
            if result == 0 {
                let msg_type = mpack_unpack_uint(tok) as u32;
                let bad_type = if array_length == 3 {
                    msg_type != 2
                } else {
                    msg_type >= 2
                };
                if bad_type {
                    result = MPACK_ERROR;
                } else {
                    p.msg_type = std::mem::transmute::<i32, MessageType>(msg_type as i32);
                    p.request_id = 0;

                    if p.msg_type != MessageType::kMessageTypeNotification {
                        result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
                        if result == 0 && tok.r#type != mpack_token_type_t::MPACK_TOKEN_UINT {
                            result = MPACK_ERROR;
                        }
                        if result == 0 {
                            p.request_id = mpack_unpack_uint(tok) as u32;
                        }
                    }

                    if result == 0 && p.msg_type != MessageType::kMessageTypeResponse {
                        result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
                        if result == 0
                            && ((tok.r#type != mpack_token_type_t::MPACK_TOKEN_STR
                                && tok.r#type != mpack_token_type_t::MPACK_TOKEN_BIN)
                                || tok.length > 100)
                        {
                            result = MPACK_ERROR;
                        }
                        if result == 0 {
                            p.method_name_len = tok.length as usize;
                            if p.method_name_len > 0 {
                                result = mpack_read(&mut p.reader, &mut data, &mut size, &mut tok);
                                if result == 0 {
                                    assert_eq!(tok.r#type, mpack_token_type_t::MPACK_TOKEN_CHUNK);
                                }
                            }
                            if result == 0 && (tok.length as usize) < p.method_name_len {
                                result = MPACK_EOF;
                            }
                            if result == 0 {
                                let name_ptr = if tok.length != 0 {
                                    tok.data.chunk_ptr
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
        }

        if result == MPACK_EOF {
            mpack_tokbuf_init(&mut p.reader);
        } else {
            api_set_error!(
                &mut p.unpack_error,
                ErrorType::kErrorTypeValidation,
                c"failed to decode msgpack".as_ptr(),
            );
            p.state = -1;
        }
        false
    }
}

/// Advance the unpacker state machine (`unpacker_advance`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpacker_advance(p: *mut Unpacker) -> bool {
    #[cfg(test)]
    {
        let _ = p;
        false
    }
    #[cfg(not(test))]
    unsafe {
        let p_ref = &mut *p;
        assert!(p_ref.state >= 0);
        p_ref.has_grid_line_event = false;
        if p_ref.state == 0 {
            if !unpacker_parse_header(p) {
                return false;
            }
            if p_ref.msg_type == MessageType::kMessageTypeNotification
                && p_ref.handler.r#fn.map(|f| f as *const c_void)
                    == Some(handle_ui_client_redraw as *const c_void)
            {
                p_ref.msg_type = MessageType::kMessageTypeRedrawEvent;
                p_ref.state = 10;
            } else {
                p_ref.state = if p_ref.msg_type == MessageType::kMessageTypeResponse {
                    1
                } else {
                    2
                };
                p_ref.arena = empty_arena();
            }
        }

        let mut need_parse = true;
        if p_ref.state >= 10 && p_ref.state != 13 {
            if !unpacker_parse_redraw(p) {
                if p_ref.state < 0 {
                    api_set_error!(
                        &mut p_ref.unpack_error,
                        ErrorType::kErrorTypeValidation,
                        c"failed to parse redraw event".as_ptr(),
                    );
                }
                return false;
            }

            if p_ref.state == 16 {
                p_ref.has_grid_line_event = true;
                need_parse = false;
            } else {
                assert_eq!(p_ref.state, 12);
                p_ref.arena = empty_arena();
                p_ref.state = 13;
            }
        }

        loop {
            if need_parse {
                let result = mpack_parse(
                    &mut p_ref.parser,
                    &mut p_ref.read_ptr,
                    &mut p_ref.read_size,
                    Some(api_parse_enter),
                    Some(api_parse_exit),
                );
                if result == MPACK_EOF {
                    return false;
                } else if result != MPACK_OK {
                    api_set_error!(
                        &mut p_ref.unpack_error,
                        ErrorType::kErrorTypeValidation,
                        c"failed to parse msgpack".as_ptr(),
                    );
                    p_ref.state = -1;
                    return false;
                }
            }

            match p_ref.state {
                1 => {
                    p_ref.error = p_ref.result;
                    p_ref.state = 2;
                    need_parse = true;
                }
                2 => {
                    p_ref.state = 0;
                    return true;
                }
                13 | 16 => {
                    p_ref.ncalls -= 1;
                    if p_ref.ncalls > 0 {
                        p_ref.state = if p_ref.state == 16 { 14 } else { 12 };
                    } else if p_ref.nevents > 0 {
                        p_ref.state = 11;
                    } else {
                        p_ref.state = 0;
                    }
                    return true;
                }
                _ => std::process::abort(),
            }
        }
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn next_type(
    p: &mut Unpacker,
    data: &mut *const c_char,
    size: &mut usize,
    tok: &mut mpack_token_t,
    expected: mpack_token_type_t,
) -> bool {
    let result = unsafe { mpack_rtoken(data, size, tok) };
    if result == MPACK_EOF {
        return false;
    }
    let valid_type = tok.r#type == expected
        || (expected == mpack_token_type_t::MPACK_TOKEN_STR
            && tok.r#type == mpack_token_type_t::MPACK_TOKEN_BIN)
        || (expected == mpack_token_type_t::MPACK_TOKEN_SINT
            && tok.r#type == mpack_token_type_t::MPACK_TOKEN_UINT);
    if result != 0 || !valid_type {
        p.state = -1;
        return false;
    }
    true
}

/// Specialized redraw/grid_line parser (`unpacker_parse_redraw`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpacker_parse_redraw(p: *mut Unpacker) -> bool {
    #[cfg(test)]
    {
        let _ = p;
        false
    }
    #[cfg(not(test))]
    unsafe {
        let p = &mut *p;
        let mut tok = std::mem::zeroed::<mpack_token_t>();
        let mut data = p.read_ptr;
        let mut size = p.read_size;

        if p.state == 10 {
            if !next_type(
                p,
                &mut data,
                &mut size,
                &mut tok,
                mpack_token_type_t::MPACK_TOKEN_ARRAY,
            ) {
                return false;
            }
            p.nevents = tok.length as c_int;
            p.state = 11;
        }

        if p.state == 11 {
            if !next_type(
                p,
                &mut data,
                &mut size,
                &mut tok,
                mpack_token_type_t::MPACK_TOKEN_ARRAY,
            ) {
                return false;
            }
            p.ncalls = tok.length as c_int;
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
                mpack_token_type_t::MPACK_TOKEN_STR,
            ) {
                return false;
            }
            let str_len = tok.length as usize;
            if str_len > size {
                return false;
            }

            p.ui_handler = ui_client_get_redraw_handler(data, str_len, std::ptr::null_mut());
            data = data.add(str_len);
            size -= str_len;

            p.nevents -= 1;
            p.read_ptr = data;
            p.read_size = size;
            if p.ui_handler.r#fn.map(|f| f as *const c_void)
                != Some(ui_client_event_grid_line as *const c_void)
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
                mpack_token_type_t::MPACK_TOKEN_ARRAY,
            ) {
                return false;
            }
            let eventarrsize = tok.length as c_int;
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
                    mpack_token_type_t::MPACK_TOKEN_UINT,
                ) {
                    return false;
                }
                p.grid_line_event.args[i] = tok.data.value.lo as c_int;
            }

            if !next_type(
                p,
                &mut data,
                &mut size,
                &mut tok,
                mpack_token_type_t::MPACK_TOKEN_ARRAY,
            ) {
                return false;
            }
            p.grid_line_event.ncells = tok.length as c_int;
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
                    mpack_token_type_t::MPACK_TOKEN_ARRAY,
                ) {
                    return false;
                }
                let cellarrsize = tok.length as c_int;
                if !(1..=3).contains(&cellarrsize) {
                    p.state = -1;
                    return false;
                }

                if !next_type(
                    p,
                    &mut data,
                    &mut size,
                    &mut tok,
                    mpack_token_type_t::MPACK_TOKEN_STR,
                ) {
                    return false;
                }
                let cellsize = tok.length as usize;
                if cellsize > size {
                    return false;
                }

                let cellbuf = data;
                data = data.add(cellsize);
                size -= cellsize;

                if cellarrsize >= 2 {
                    if !next_type(
                        p,
                        &mut data,
                        &mut size,
                        &mut tok,
                        mpack_token_type_t::MPACK_TOKEN_SINT,
                    ) {
                        return false;
                    }
                    p.grid_line_event.cur_attr = tok.data.value.lo as c_int;
                }

                let mut repeat = 1 as c_int;
                if cellarrsize >= 3 {
                    if !next_type(
                        p,
                        &mut data,
                        &mut size,
                        &mut tok,
                        mpack_token_type_t::MPACK_TOKEN_UINT,
                    ) {
                        return false;
                    }
                    repeat = tok.data.value.lo as c_int;
                }

                p.grid_line_event.clear_width = 0;
                if p.grid_line_event.icell == p.grid_line_event.ncells - 1
                    && cellsize == 1
                    && *cellbuf as u8 == b' '
                    && repeat > 1
                {
                    p.grid_line_event.clear_width = repeat;
                } else {
                    let sc = schar_from_buf(cellbuf, cellsize);
                    for _ in 0..repeat {
                        if p.grid_line_event.coloff >= (grid_line_buf_size as c_int) {
                            p.state = -1;
                            return false;
                        }
                        let off = p.grid_line_event.coloff as usize;
                        *grid_line_buf_char.add(off) = sc;
                        *grid_line_buf_attr.add(off) = p.grid_line_event.cur_attr;
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
                mpack_token_type_t::MPACK_TOKEN_BOOLEAN,
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
}

/// Unpack a complete msgpack string/bin token (`unpack_string`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpack_string(data: *mut *const c_char, size: *mut usize) -> NvimString {
    #[cfg(test)]
    {
        let _ = (data, size);
        NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        }
    }
    #[cfg(not(test))]
    unsafe {
        let mut data2 = *data;
        let mut size2 = *size;
        let mut tok = std::mem::zeroed::<mpack_token_t>();

        let result = mpack_rtoken(&mut data2, &mut size2, &mut tok);
        if result != 0
            || (tok.r#type != mpack_token_type_t::MPACK_TOKEN_STR
                && tok.r#type != mpack_token_type_t::MPACK_TOKEN_BIN)
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
        *data = data2.add(len);
        *size = size2 - len;
        NvimString {
            data: data2.cast_mut(),
            size: len,
        }
    }
}

/// Unpack a msgpack array header (`unpack_array`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpack_array(data: *mut *const c_char, size: *mut usize) -> isize {
    #[cfg(test)]
    {
        let _ = (data, size);
        -1
    }
    #[cfg(not(test))]
    unsafe {
        let mut tok = std::mem::zeroed::<mpack_token_t>();
        let result = mpack_rtoken(data, size, &mut tok);
        if result != 0 || tok.r#type != mpack_token_type_t::MPACK_TOKEN_ARRAY {
            return -1;
        }
        tok.length as isize
    }
}

/// Unpack a msgpack integer (`unpack_integer`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpack_integer(
    data: *mut *const c_char,
    size: *mut usize,
    res: *mut Integer,
) -> bool {
    #[cfg(test)]
    {
        let _ = (data, size, res);
        false
    }
    #[cfg(not(test))]
    unsafe {
        let mut tok = std::mem::zeroed::<mpack_token_t>();
        let result = mpack_rtoken(data, size, &mut tok);
        if result != 0 {
            return false;
        }
        unpack_uint_or_sint(tok, res)
    }
}

/// Unpack a uint or sint token into `Integer` (`unpack_uint_or_sint`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpack_uint_or_sint(tok: mpack_token_t, res: *mut Integer) -> bool {
    #[cfg(test)]
    {
        let _ = (tok, res);
        false
    }
    #[cfg(not(test))]
    unsafe {
        if tok.r#type == mpack_token_type_t::MPACK_TOKEN_UINT {
            *res = mpack_unpack_uint(tok) as Integer;
            true
        } else if tok.r#type == mpack_token_type_t::MPACK_TOKEN_SINT {
            *res = mpack_unpack_sint(tok) as Integer;
            true
        } else {
            false
        }
    }
}

/// Skip one msgpack value (`unpack_skip`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unpack_skip(data: *mut *const c_char, size: *mut usize) -> c_int {
    #[cfg(test)]
    {
        let _ = (data, size);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let mut parser = std::mem::zeroed::<mpack_parser_t>();
        mpack_parser_init(&mut parser, 0);
        mpack_parse(&mut parser, data, size, Some(parse_nop), Some(parse_nop))
    }
}

/// Append raw msgpack key-value bytes to `AdditionalDataBuilder` (`push_additional_data`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn push_additional_data(
    ad: *mut AdditionalDataBuilder,
    data: *const c_char,
    size: usize,
) {
    let ad = unsafe { &mut *ad };
    if ad.size == 0 {
        let init = AdditionalDataHeader {
            nitems: 0,
            nbytes: 0,
        };
        unsafe {
            ad.concat_len(
                (&init as *const AdditionalDataHeader).cast::<c_char>(),
                std::mem::size_of::<AdditionalDataHeader>(),
            );
        }
    }
    unsafe {
        let header = ad.items.cast::<AdditionalDataHeader>();
        (*header).nitems += 1;
        (*header).nbytes += size as u32;
        ad.concat_len(data, size);
    }
}

/// Unpack a msgpack map into a `KeySet` struct (`unpack_keydict`).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn unpack_keydict(
    retval: *mut c_void,
    hashy: FieldHashfn,
    ad: *mut AdditionalDataBuilder,
    data: *mut *const c_char,
    size: *mut usize,
    error: *mut *mut c_char,
) -> bool {
    #[cfg(test)]
    {
        let _ = (retval, hashy, ad, data, size, error);
        false
    }
    #[cfg(not(test))]
    unsafe {
        let ks = retval.cast::<OptKeySet>();
        let mut tok = std::mem::zeroed::<mpack_token_t>();

        let result = mpack_rtoken(data, size, &mut tok);
        if result != 0 || tok.r#type != mpack_token_type_t::MPACK_TOKEN_MAP {
            *error = xstrdup(c"is not a dict".as_ptr());
            return false;
        }

        let map_size = tok.length as usize;
        let hashy_fn = hashy.unwrap();

        let alloc_key_error = |key: NvimString, suffix: &[u8]| -> *mut c_char {
            let key_bytes = std::slice::from_raw_parts(key.data.cast::<u8>(), key.size);
            let mut out = Vec::with_capacity(4 + key.size + suffix.len());
            out.extend_from_slice(b"has ");
            out.extend_from_slice(key_bytes);
            out.extend_from_slice(suffix);
            crate::memory::xmemdupz(out.as_ptr().cast(), out.len()).cast()
        };

        for _ in 0..map_size {
            let item_start = *data;
            let key = unpack_string(data, size);
            if key.data.is_null() {
                *error = xstrdup(c"has key value which is not a string".as_ptr());
                return false;
            } else if key.size == 0 {
                *error = xstrdup(c"has empty key".as_ptr());
                return false;
            }
            let field_ptr = hashy_fn(key.data, key.size);

            if field_ptr.is_null() {
                let status = unpack_skip(data, size);
                if status != 0 {
                    return false;
                }
                if !ad.is_null() {
                    push_additional_data(ad, item_start, (*data as usize) - (item_start as usize));
                }
                continue;
            }

            let field = &*field_ptr;
            assert!(field.opt_index >= 0);
            let flag = 1u64 << (field.opt_index as u32);
            if ((*ks).is_set_ & flag) != 0 {
                *error = xstrdup(c"duplicate key".as_ptr());
                return false;
            }
            (*ks).is_set_ |= flag;

            let mem = retval.cast::<u8>().add(field.ptr_off);
            if field.key_type == (ObjectType::kObjectTypeBoolean as c_int) {
                let cur_byte = if *size > 0 { **data as u8 } else { 0 };
                if *size == 0 || (cur_byte & 0xfe) != 0xc2 {
                    *error = alloc_key_error(key, b" key value which is not a boolean");
                    return false;
                }
                *mem.cast::<Boolean>() = (cur_byte & 0x01) != 0;
                *data = (*data).add(1);
                *size -= 1;
            } else if field.key_type == (ObjectType::kObjectTypeInteger as c_int) {
                if !unpack_integer(data, size, mem.cast::<Integer>()) {
                    *error = alloc_key_error(key, b" key value which is not an integer");
                    return false;
                }
            } else if field.key_type == (ObjectType::kObjectTypeString as c_int) {
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
                a.ensure_space(len as usize);
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
    use crate::memory::xfree;

    #[test]
    fn push_additional_data_populates_header_and_payload() {
        let mut ad = AdditionalDataBuilder::new();
        unsafe {
            push_additional_data(&mut ad, c"abc".as_ptr(), 3);
            push_additional_data(&mut ad, c"de".as_ptr(), 2);
            let hdr = &*(ad.items.cast::<AdditionalDataHeader>());
            assert_eq!(hdr.nitems, 2);
            assert_eq!(hdr.nbytes, 5);
            assert_eq!(ad.size, std::mem::size_of::<AdditionalDataHeader>() + 5);
            xfree(ad.items.cast::<c_void>());
        }
    }
}
