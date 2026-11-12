#![forbid(unsafe_code)]
//! Lua `vim.spell` bindings (`src/nvim/lua/spell.c`).

use crate::errors::*;
use crate::globals::*;
use crate::lua::converter::{
    LUA_TSTRING, lua_Integer, lua_State, lua_createtable_safe, lua_push_bytes,
    lua_pushinteger_safe, lua_pushstring, lua_rawseti_safe, lua_register_funcs, lua_tolstring_cstr,
    lua_type_safe, luaL_argerror, luaL_error_cstr, nlua_gettop,
};
use crate::message::emsg;
use crate::spell::{parse_spelllang, spell_check};
use crate::strings::gettext;
use std::ffi::CStr;

use crate::drawline::HLF_COUNT;
use crate::eval::funcs::{HLF_SPB, HLF_SPC, HLF_SPL, HLF_SPR};
use crate::ex_getln::Hlf;

/// Check spelling in `str` (`nlua_spell_check`).
pub extern "C-unwind" fn nlua_spell_check(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) < 1 {
        return luaL_error_cstr(lstate, c"Expected 1 argument");
    }

    if lua_type_safe(lstate, 1) != LUA_TSTRING {
        luaL_argerror(lstate, 1, c"expected string");
    }

    let mut str_cstr = lua_tolstring_cstr(lstate, 1);

    let wo_spell_save = curwin_mut().w_onebuf_opt.wo_spell.get();

    if !curwin_mut().w_onebuf_opt.wo_spell.get() {
        parse_spelllang(&curwin_mut());
        curwin_mut().w_onebuf_opt.wo_spell.set(true);
    }

    let w_s = curwin_mut().w_s_ptr();
    if w_s.b_p_spl.is_empty() {
        emsg(gettext(E_NO_SPELL));
        curwin_mut().w_onebuf_opt.wo_spell.set(wo_spell_save);
        return 0;
    }

    let mut pos: usize = 0;
    let mut capcol: i32 = -1;
    let mut no_res: i32 = 0;

    lua_createtable_safe(lstate, 0, 0);

    while !str_cstr.is_empty() {
        let mut attr: Hlf = HLF_COUNT;
        let len = spell_check(&curwin_mut(), str_cstr, &mut attr, Some(&mut capcol), false);
        assert!(len <= i32::MAX as usize);

        if attr != HLF_COUNT {
            lua_createtable_safe(lstate, 3, 0);

            lua_push_bytes(lstate, &str_cstr.to_bytes()[..len]);
            lua_rawseti_safe(lstate, -2, 1);

            let result = match attr {
                HLF_SPB => Some(c"bad"),
                HLF_SPR => Some(c"rare"),
                HLF_SPL => Some(c"local"),
                HLF_SPC => Some(c"caps"),
                _ => None,
            };
            let result = result.expect("spell_check returned unexpected Hlf");

            lua_pushstring(lstate, result);
            lua_rawseti_safe(lstate, -2, 2);

            lua_pushinteger_safe(lstate, (pos as lua_Integer) + 1);
            lua_rawseti_safe(lstate, -2, 3);

            no_res += 1;
            lua_rawseti_safe(lstate, -2, no_res);
        }

        str_cstr = &str_cstr[len..];
        pos += len;
        capcol -= len as i32;
    }

    curwin_mut().w_onebuf_opt.wo_spell.set(wo_spell_save);
    1
}

/// Open the `vim.spell` Lua library (`luaopen_spell`).
pub extern "C" fn luaopen_spell(lstate: *mut lua_State) -> i32 {
    const SPELL_FUNCTIONS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 1] =
        [(c"check", nlua_spell_check)];
    lua_createtable_safe(lstate, 0, 0);
    lua_register_funcs(lstate, &SPELL_FUNCTIONS);
    1
}
