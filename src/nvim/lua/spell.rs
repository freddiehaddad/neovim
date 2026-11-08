//! Lua `vim.spell` bindings (`src/nvim/lua/spell.c`).

use crate::errors::*;
use crate::globals::*;
use crate::lua::converter::{
    LUA_TSTRING, lua_Integer, lua_State, lua_createtable, lua_gettop, lua_pushinteger,
    lua_pushlstring, lua_pushstring, lua_rawseti, lua_tolstring, lua_type, luaL_Reg, luaL_argerror,
    luaL_error, luaL_register,
};
use crate::message::emsg;
use crate::spell::{parse_spelllang, spell_check};
use crate::strings::gettext;
use std::ptr;

use crate::drawline::HLF_COUNT;
use crate::eval::funcs::{HLF_SPB, HLF_SPC, HLF_SPL, HLF_SPR};
use crate::ex_getln::Hlf;

/// Check spelling in `str` (`nlua_spell_check`).
pub unsafe extern "C-unwind" fn nlua_spell_check(lstate: *mut lua_State) -> i32 {
    if unsafe { lua_gettop(lstate) } < 1 {
        return unsafe { luaL_error(lstate, c"Expected 1 argument".as_ptr()) };
    }

    if unsafe { lua_type(lstate, 1) } != LUA_TSTRING {
        luaL_argerror(lstate, 1, c"expected string");
    }

    let mut str_ = unsafe { lua_tolstring(lstate, 1, ptr::null_mut()) };

    let wo_spell_save = curwin_mut().w_onebuf_opt.wo_spell;

    if !curwin_mut().w_onebuf_opt.wo_spell {
        parse_spelllang(curwin_mut());
        curwin_mut().w_onebuf_opt.wo_spell = true;
    }

    let w_s = curwin_mut().w_s_ptr();
    if w_s.b_p_spl.is_empty() {
        emsg(gettext(E_NO_SPELL));
        curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
        return 0;
    }

    let mut pos: usize = 0;
    let mut capcol: i32 = -1;
    let mut no_res: i32 = 0;

    unsafe { lua_createtable(lstate, 0, 0) };

    while !str_.is_null() && unsafe { *str_ } != 0 {
        let mut attr: Hlf = HLF_COUNT;
        let len = spell_check(
            curwin_mut(),
            unsafe { std::ffi::CStr::from_ptr(str_) },
            &mut attr,
            Some(&mut capcol),
            false,
        );
        assert!(len <= i32::MAX as usize);

        if attr != HLF_COUNT {
            unsafe { lua_createtable(lstate, 3, 0) };

            unsafe { lua_pushlstring(lstate, str_, len) };
            unsafe { lua_rawseti(lstate, -2, 1) };

            let result = match attr {
                HLF_SPB => Some(c"bad"),
                HLF_SPR => Some(c"rare"),
                HLF_SPL => Some(c"local"),
                HLF_SPC => Some(c"caps"),
                _ => None,
            };
            let result = result.expect("spell_check returned unexpected Hlf");

            lua_pushstring(lstate, result);
            unsafe { lua_rawseti(lstate, -2, 2) };

            unsafe { lua_pushinteger(lstate, (pos as lua_Integer) + 1) };
            unsafe { lua_rawseti(lstate, -2, 3) };

            no_res += 1;
            unsafe { lua_rawseti(lstate, -2, no_res) };
        }

        str_ = str_.wrapping_add(len);
        pos += len;
        capcol -= len as i32;
    }

    curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
    1
}

/// Open the `vim.spell` Lua library (`luaopen_spell`).
pub unsafe extern "C" fn luaopen_spell(lstate: *mut lua_State) -> i32 {
    unsafe {
        const SPELL_FUNCTIONS: [luaL_Reg; 2] = [
            luaL_Reg {
                name: c"check".as_ptr(),
                func: Some(nlua_spell_check),
            },
            luaL_Reg {
                name: ptr::null(),
                func: None,
            },
        ];
        {
            lua_createtable(lstate, 0, 0);
            luaL_register(lstate, None, &SPELL_FUNCTIONS);
            1
        }
    }
}
