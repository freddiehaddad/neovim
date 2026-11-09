#![forbid(unsafe_code)]
//! Lua `vim.base64` bindings (`src/nvim/lua/base64.c`).

use crate::base64::{decode_bytes, encode_bytes};
use crate::lua::converter::{
    LUA_TSTRING, lua_State, lua_createtable_safe, lua_push_bytes, lua_register_funcs,
    lua_tolstring_bytes, lua_type_safe, luaL_argerror, luaL_error_cstr, nlua_gettop,
};
use std::ffi::CStr;

extern "C-unwind" fn nlua_base64_encode(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) < 1 {
        return luaL_error_cstr(lstate, c"Expected 1 argument");
    }
    if lua_type_safe(lstate, 1) != LUA_TSTRING {
        luaL_argerror(lstate, 1, c"expected string");
    }

    let slice = lua_tolstring_bytes(lstate, 1);
    let encoded = encode_bytes(slice);
    lua_push_bytes(lstate, &encoded);
    1
}

extern "C-unwind" fn nlua_base64_decode(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) < 1 {
        return luaL_error_cstr(lstate, c"Expected 1 argument");
    }
    if lua_type_safe(lstate, 1) != LUA_TSTRING {
        luaL_argerror(lstate, 1, c"expected string");
    }

    let slice = lua_tolstring_bytes(lstate, 1);
    let Some(decoded) = decode_bytes(slice) else {
        return luaL_error_cstr(lstate, c"Invalid input");
    };

    lua_push_bytes(lstate, &decoded);
    1
}

static BASE64_FUNCTIONS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 2] = [
    (c"encode", nlua_base64_encode),
    (c"decode", nlua_base64_decode),
];

/// Open the `vim.base64` Lua library (`luaopen_base64`).
pub extern "C" fn luaopen_base64(lstate: *mut lua_State) -> i32 {
    lua_createtable_safe(lstate, 0, 0);
    lua_register_funcs(lstate, &BASE64_FUNCTIONS);
    1
}
