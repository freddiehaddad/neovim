//! Secure file reading and `:trust` command (`src/nvim/lua/secure.c`).

use crate::errors::{E_INVARG2, E_TRUSTFILE};
use crate::lua::converter::{
    LUA_GLOBALSINDEX, lua_createtable, lua_getfield, lua_gettop, lua_pushnumber, lua_pushstring,
    lua_settable, lua_settop, lua_toboolean, lua_tolstring,
};
use crate::lua::executor::{get_global_lstate, nlua_error, nlua_pcall};
use crate::message::{semsg, smsg};
use std::ffi::CString;

use crate::ex_docmd::Exarg;

fn nlua_trust(action: &std::ffi::CStr, path: Option<&std::ffi::CStr>) -> bool {
    let lstate = get_global_lstate();
    let top = unsafe { lua_gettop(lstate) };

    lua_getfield(lstate, LUA_GLOBALSINDEX, c"vim");
    lua_getfield(lstate, -1, c"secure");
    lua_getfield(lstate, -1, c"trust");

    unsafe { lua_createtable(lstate, 0, 0) };
    lua_pushstring(lstate, c"action");
    lua_pushstring(lstate, action);
    unsafe { lua_settable(lstate, -3) };
    if let Some(path) = path {
        lua_pushstring(lstate, c"path");
        lua_pushstring(lstate, path);
        unsafe { lua_settable(lstate, -3) };
    } else {
        lua_pushstring(lstate, c"bufnr");
        unsafe { lua_pushnumber(lstate, 0.0) };
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
        let msg_cstr = unsafe { std::ffi::CStr::from_ptr(msg) };
        if success {
            let action_bytes = action.to_bytes();
            if action_bytes == b"allow" {
                smsg!(0, c"Allowed in trust database: \"%s\"", msg_cstr);
            } else if action_bytes == b"deny" {
                smsg!(0, c"Denied in trust database: \"%s\"", msg_cstr);
            } else if action_bytes == b"remove" {
                smsg!(0, c"Removed from trust database: \"%s\"", msg_cstr);
            }
        } else {
            semsg!(E_TRUSTFILE, msg_cstr);
        }
    }

    unsafe { lua_settop(lstate, top) };
    success
}

/// `:trust` command implementation (`ex_trust`).
pub fn ex_trust(eap: &mut Exarg) {
    let arg_cstr = eap.arg.as_cstr();
    let arg_bytes = arg_cstr.to_bytes();
    let arg1_len = crate::charset::skiptowhite_len(arg_bytes);
    let arg1_bytes = &arg_bytes[..arg1_len];
    let mut action = c"allow";
    let rest_off = arg1_len + crate::charset::skipwhite_len(&arg_bytes[arg1_len..]);

    if arg1_bytes == b"++deny" {
        action = c"deny";
    } else if arg1_bytes == b"++remove" {
        action = c"remove";
    } else if !arg1_bytes.is_empty() {
        let arg1 = CString::new(arg1_bytes).unwrap();
        semsg!(E_INVARG2, &arg1);
        return;
    }

    let path = if rest_off >= arg_bytes.len() {
        None
    } else {
        Some(&arg_cstr[rest_off..])
    };

    nlua_trust(action, path);
}

#[cfg(test)]
mod tests {
    #[test]
    fn skipwhite_and_skiptowhite_scan_ascii_whitespace() {
        let s = c"++deny   file.lua";
        let len = crate::charset::skiptowhite_len(s.to_bytes());
        assert_eq!(len, 6);
        let p = crate::memory::CStrSubslice::advance(s, len);
        let q = crate::charset::skipwhite(p);
        assert_eq!(q.to_bytes(), b"file.lua");
    }
}
