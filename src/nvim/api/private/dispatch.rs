//! API Msgpack-RPC request handler lookup (`src/nvim/api/private/dispatch.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::api::private::validate::{Array, Error, ErrorType, Object};
use crate::memory::Arena;

pub type ApiDispatchWrapper = Option<
    unsafe extern "C" fn(
        channel_id: u64,
        args: Array,
        arena: *mut Arena,
        error: *mut Error,
    ) -> Object,
>;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct MsgpackRpcRequestHandler {
    pub name: *const c_char,
    pub r#fn: ApiDispatchWrapper,
    pub fast: bool,
    pub ret_alloc: bool,
}

unsafe extern "C" {
    fn msgpack_rpc_get_handler_for_hash(str_in: *const c_char, len: usize) -> c_int;
    fn msgpack_rpc_get_handler_at(hash: c_int) -> MsgpackRpcRequestHandler;
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn msgpack_rpc_get_handler_for(
    name: *const c_char,
    name_len: usize,
    error: *mut Error,
) -> MsgpackRpcRequestHandler {
    unsafe {
        let hash = msgpack_rpc_get_handler_for_hash(name, name_len);
        if hash < 0 {
            let empty = c"<empty>";
            api_set_error!(
                error,
                ErrorType::kErrorTypeException,
                c"Invalid method: %.*s".as_ptr(),
                if name_len > 0 {
                    name_len as c_int
                } else {
                    (empty.to_bytes_with_nul().len()) as c_int
                },
                if name_len > 0 { name } else { empty.as_ptr() },
            );
            return MsgpackRpcRequestHandler {
                name: ptr::null(),
                r#fn: None,
                fast: false,
                ret_alloc: false,
            };
        }
        msgpack_rpc_get_handler_at(hash)
    }
}
