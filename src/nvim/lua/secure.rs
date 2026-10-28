//! Secure file reading and `:trust` command (`src/nvim/lua/secure.c`).

use crate::charset::{skiptowhite, skipwhite};
use crate::errors::{E_INVARG2, E_TRUSTFILE};
use crate::lua::converter::{
    LUA_GLOBALSINDEX, lua_createtable, lua_getfield, lua_gettop, lua_pushnumber, lua_pushstring,
    lua_settable, lua_settop, lua_toboolean, lua_tolstring,
};
use crate::lua::executor::{get_global_lstate, nlua_error, nlua_pcall};
use crate::message::{semsg, smsg};
use std::ffi::CString;

use crate::ex_docmd::Exarg;

fn nlua_trust(
    action: impl crate::strings::IntoCCharPtr,
    path: impl crate::strings::IntoCCharPtr,
) -> bool {
    let action = action.into_c_char_ptr();
    let path = path.into_c_char_ptr();
    {
        let lstate = get_global_lstate();
        let top = unsafe { lua_gettop(lstate) };

        lua_getfield(lstate, LUA_GLOBALSINDEX, c"vim");
        lua_getfield(lstate, -1, c"secure");
        lua_getfield(lstate, -1, c"trust");

        unsafe { lua_createtable(lstate, 0, 0) };
        lua_pushstring(lstate, c"action");
        lua_pushstring(lstate, action);
        unsafe { lua_settable(lstate, -3) };
        if path.is_null() {
            lua_pushstring(lstate, c"bufnr");
            unsafe { lua_pushnumber(lstate, 0.0) };
            unsafe { lua_settable(lstate, -3) };
        } else {
            lua_pushstring(lstate, c"path");
            lua_pushstring(lstate, path);
            unsafe { lua_settable(lstate, -3) };
        }

        if nlua_pcall(lstate, 1, 2) != 0 {
            nlua_error(lstate, c"vim.secure.trust: %.*s");
            unsafe { lua_settop(lstate, top) };
            return false;
        }

        let success = unsafe { lua_toboolean(lstate, -2) } != 0;
        let msg = unsafe { lua_tolstring(lstate, -1, std::ptr::null_mut()) };
        if !msg.is_null() {
            if success {
                let action_bytes = crate::memory::cstr_as_bytes(action);
                if action_bytes == b"allow" {
                    smsg!(0, c"Allowed in trust database: \"%s\"", msg);
                } else if action_bytes == b"deny" {
                    smsg!(0, c"Denied in trust database: \"%s\"", msg);
                } else if action_bytes == b"remove" {
                    smsg!(0, c"Removed from trust database: \"%s\"", msg);
                }
            } else {
                semsg!(E_TRUSTFILE, msg);
            }
        }

        unsafe { lua_settop(lstate, top) };
        success
    }
}

/// `:trust` command implementation (`ex_trust`).
pub fn ex_trust(eap: &mut Exarg) {
    let arg = eap.arg;
    let p = skiptowhite(arg);
    let arg1_len = (p as usize) - (arg as usize);
    let arg1 = CString::new(crate::memory::cstr_slice(arg, arg1_len)).unwrap();
    let mut action = c"allow".as_ptr();
    let mut path = skipwhite(p);

    let arg1_bytes = arg1.as_bytes();
    if arg1_bytes == b"++deny" {
        action = c"deny".as_ptr();
    } else if arg1_bytes == b"++remove" {
        action = c"remove".as_ptr();
    } else if !arg1_bytes.is_empty() {
        semsg!(E_INVARG2, arg1.as_ptr());
        return;
    }

    if unsafe { *path } == 0 {
        path = std::ptr::null_mut();
    }

    nlua_trust(action, path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipwhite_and_skiptowhite_scan_ascii_whitespace() {
        let s = c"++deny   file.lua";
        {
            let p = skiptowhite(s.as_ptr());
            assert_eq!((p as usize) - (s.as_ptr() as usize), 6);
            let q = skipwhite(p);
            assert_eq!(crate::memory::cstr_as_bytes(q), b"file.lua");
        }
    }
}
