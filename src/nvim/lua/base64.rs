//! Lua `vim.base64` bindings (`src/nvim/lua/base64.c`).

use crate::base64::{decode_bytes, encode_bytes};
use crate::lua::converter::{
    LUA_TSTRING, lua_State, lua_createtable, lua_gettop, lua_pushlstring, lua_tolstring, lua_type,
    luaL_Reg, luaL_argerror, luaL_error, luaL_register,
};
use std::ffi::c_char;

unsafe extern "C-unwind" fn nlua_base64_encode(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            if lua_gettop(lstate) < 1 {
                return luaL_error(lstate, c"Expected 1 argument".as_ptr());
            }
            if lua_type(lstate, 1) != LUA_TSTRING {
                luaL_argerror(lstate, 1, c"expected string");
            }

            let mut src_len: usize = 0;
            let src = lua_tolstring(lstate, 1, &mut src_len);
            let slice = if src_len == 0 || src.is_null() {
                &[]
            } else {
                crate::memory::cstr_slice(src, src_len)
            };

            let encoded = encode_bytes(slice);
            lua_pushlstring(lstate, encoded.as_ptr().cast::<c_char>(), encoded.len());
            1
        }
    }
}

unsafe extern "C-unwind" fn nlua_base64_decode(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            if lua_gettop(lstate) < 1 {
                return luaL_error(lstate, c"Expected 1 argument".as_ptr());
            }
            if lua_type(lstate, 1) != LUA_TSTRING {
                luaL_argerror(lstate, 1, c"expected string");
            }

            let mut src_len: usize = 0;
            let src = lua_tolstring(lstate, 1, &mut src_len);
            let slice = if src_len == 0 || src.is_null() {
                &[]
            } else {
                crate::memory::cstr_slice(src, src_len)
            };

            let Some(decoded) = decode_bytes(slice) else {
                return luaL_error(lstate, c"Invalid input".as_ptr());
            };

            lua_pushlstring(lstate, decoded.as_ptr().cast::<c_char>(), decoded.len());
            1
        }
    }
}

static BASE64_FUNCTIONS: [luaL_Reg; 3] = [
    luaL_Reg {
        name: c"encode".as_ptr(),
        func: Some(nlua_base64_encode),
    },
    luaL_Reg {
        name: c"decode".as_ptr(),
        func: Some(nlua_base64_decode),
    },
    luaL_Reg {
        name: std::ptr::null(),
        func: None,
    },
];

/// Open the `vim.base64` Lua library (`luaopen_base64`).
///
/// # Safety
/// `lstate` must be a valid `lua_State` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaopen_base64(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            lua_createtable(lstate, 0, 0);
            luaL_register(lstate, std::ptr::null(), BASE64_FUNCTIONS.as_ptr());
        }
        1
    }
}
