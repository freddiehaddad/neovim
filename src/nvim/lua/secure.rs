//! Secure file reading and `:trust` command (`src/nvim/lua/secure.c`).

use crate::charset::{skiptowhite, skipwhite};
use crate::errors::{e_invarg2, e_trustfile};
use crate::lua::converter::{
    LUA_GLOBALSINDEX, lua_createtable, lua_getfield, lua_gettop, lua_pushnumber, lua_pushstring,
    lua_settable, lua_settop, lua_toboolean, lua_tolstring,
};
use crate::lua::executor::{get_global_lstate, nlua_error, nlua_pcall};
use crate::message::{semsg, smsg};
use std::ffi::{CStr, CString, c_char};

use crate::ex_docmd::exarg_T;

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
                semsg!(e_trustfile.as_ptr(), msg);
            }
        }

        lua_settop(lstate, top);
        success
    }
}

/// `:trust` command implementation (`ex_trust`).
pub unsafe fn ex_trust(eap: *mut exarg_T) {
    unsafe {
        let arg = (*eap).arg;
        let p = skiptowhite(arg);
        let arg1_len = (p as usize) - (arg as usize);
        let arg1 = CString::new(std::slice::from_raw_parts(arg.cast::<u8>(), arg1_len)).unwrap();
        let mut action = c"allow".as_ptr();
        let mut path = skipwhite(p);

        let arg1_bytes = arg1.as_bytes();
        if arg1_bytes == b"++deny" {
            action = c"deny".as_ptr();
        } else if arg1_bytes == b"++remove" {
            action = c"remove".as_ptr();
        } else if !arg1_bytes.is_empty() {
            semsg!(e_invarg2.as_ptr(), arg1.as_ptr());
            return;
        }

        if *path == 0 {
            path = std::ptr::null_mut();
        }

        nlua_trust(action, path);
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
