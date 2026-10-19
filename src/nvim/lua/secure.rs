//! Secure file reading and `:trust` command (`src/nvim/lua/secure.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::lua::converter::{
    lua_createtable, lua_getfield, lua_gettop, lua_pushnumber, lua_pushstring, lua_settop,
    lua_toboolean, lua_tolstring,
};
#[allow(unused_imports)]
use crate::lua::executor::{get_global_lstate, nlua_error, nlua_pcall};
use crate::message::{semsg, smsg};
use std::ffi::{CStr, c_char, c_int, c_void};

use crate::lua::base64::lua_State;
use crate::lua::converter::lua_Number;
use crate::memory::{xcalloc, xfree, xmemdupz};

const LUA_GLOBALSINDEX: c_int = -10002;

pub use crate::api::command::exarg_T;

#[cfg(not(test))]
unsafe extern "C-unwind" {
    fn lua_settable(L: *mut lua_State, idx: c_int);
}

#[inline]
unsafe fn skipwhite(mut p: *const c_char) -> *const c_char {
    unsafe {
        while matches!(*p as u8, b' ' | b'\t') {
            p = p.add(1);
        }
        p
    }
}

#[inline]
unsafe fn skiptowhite(mut p: *const c_char) -> *const c_char {
    unsafe {
        while !matches!(*p as u8, b' ' | b'\t' | 0) {
            p = p.add(1);
        }
        p
    }
}

/// Read a file securely via `vim.secure.read(path)` (`nlua_read_secure`).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nlua_read_secure(path: *const c_char) -> *mut c_char {
    #[cfg(test)]
    {
        let _ = path;
        std::ptr::null_mut()
    }
    #[cfg(not(test))]
    unsafe {
        let lstate = get_global_lstate();
        let top = lua_gettop(lstate);

        lua_getfield(lstate, LUA_GLOBALSINDEX, c"vim".as_ptr());
        lua_getfield(lstate, -1, c"secure".as_ptr());
        lua_getfield(lstate, -1, c"read".as_ptr());
        lua_pushstring(lstate, path);
        if nlua_pcall(lstate, 1, 1) != 0 {
            nlua_error(lstate, c"vim.secure.read: %.*s".as_ptr());
            lua_settop(lstate, top);
            return std::ptr::null_mut();
        }

        let mut len = 0usize;
        let contents = lua_tolstring(lstate, -1, &mut len);
        let mut buf: *mut c_char = std::ptr::null_mut();
        if !contents.is_null() {
            buf = xcalloc(len + 1, 1).cast::<c_char>();
            std::ptr::copy_nonoverlapping(contents.cast::<u8>(), buf.cast::<u8>(), len + 1);
        }

        lua_settop(lstate, top);
        buf
    }
}

#[cfg(not(test))]
unsafe fn nlua_trust(action: *const c_char, path: *const c_char) -> bool {
    unsafe {
        let lstate = get_global_lstate();
        let top = lua_gettop(lstate);

        lua_getfield(lstate, LUA_GLOBALSINDEX, c"vim".as_ptr());
        lua_getfield(lstate, -1, c"secure".as_ptr());
        lua_getfield(lstate, -1, c"trust".as_ptr());

        lua_createtable(lstate, 0, 0);
        lua_pushstring(lstate, c"action".as_ptr());
        lua_pushstring(lstate, action);
        lua_settable(lstate, -3);
        if path.is_null() {
            lua_pushstring(lstate, c"bufnr".as_ptr());
            lua_pushnumber(lstate, 0.0);
            lua_settable(lstate, -3);
        } else {
            lua_pushstring(lstate, c"path".as_ptr());
            lua_pushstring(lstate, path);
            lua_settable(lstate, -3);
        }

        if nlua_pcall(lstate, 1, 2) != 0 {
            nlua_error(lstate, c"vim.secure.trust: %.*s".as_ptr());
            lua_settop(lstate, top);
            return false;
        }

        let success = lua_toboolean(lstate, -2) != 0;
        let msg = lua_tolstring(lstate, -1, std::ptr::null_mut());
        if !msg.is_null() {
            if success {
                let action_bytes = CStr::from_ptr(action).to_bytes();
                if action_bytes == b"allow" {
                    smsg!(0, c"Allowed in trust database: \"%s\"".as_ptr(), msg);
                } else if action_bytes == b"deny" {
                    smsg!(0, c"Denied in trust database: \"%s\"".as_ptr(), msg);
                } else if action_bytes == b"remove" {
                    smsg!(0, c"Removed from trust database: \"%s\"".as_ptr(), msg);
                }
            } else {
                semsg!(c"E5570: Cannot update trust file: %s".as_ptr(), msg);
            }
        }

        lua_settop(lstate, top);
        success
    }
}

/// `:trust` command implementation (`ex_trust`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_trust(eap: *mut exarg_T) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
    unsafe {
        let arg = (*eap).arg;
        let p = skiptowhite(arg);
        let arg1_len = (p as usize) - (arg as usize);
        let arg1 = xmemdupz(arg.cast::<c_void>(), arg1_len).cast::<c_char>();
        let mut action = c"allow".as_ptr();
        let mut path = skipwhite(p);

        let arg1_bytes = CStr::from_ptr(arg1).to_bytes();
        if arg1_bytes == b"++deny" {
            action = c"deny".as_ptr();
        } else if arg1_bytes == b"++remove" {
            action = c"remove".as_ptr();
        } else if !arg1_bytes.is_empty() {
            semsg!(c"E475: Invalid argument: %s".as_ptr(), arg1);
            xfree(arg1.cast::<c_void>());
            return;
        }

        if *path == 0 {
            path = std::ptr::null();
        }

        nlua_trust(action, path);
        xfree(arg1.cast::<c_void>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipwhite_and_skiptowhite_scan_ascii_whitespace() {
        let s = c"++deny   file.lua";
        unsafe {
            let p = skiptowhite(s.as_ptr());
            assert_eq!((p as usize) - (s.as_ptr() as usize), 6);
            let q = skipwhite(p);
            assert_eq!(CStr::from_ptr(q).to_bytes(), b"file.lua");
        }
    }
}
