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
use std::ffi::c_int;
use std::ptr;

use crate::charset::NUL;
use crate::drawline::HLF_COUNT;
use crate::eval::funcs::{HLF_SPB, HLF_SPC, HLF_SPL, HLF_SPR};
use crate::ex_getln::hlf_T;

/// Check spelling in `str` (`nlua_spell_check`).
pub unsafe extern "C-unwind" fn nlua_spell_check(lstate: *mut lua_State) -> c_int {
    unsafe {
        if lua_gettop(lstate) < 1 {
            return luaL_error(lstate, c"Expected 1 argument".as_ptr());
        }

        if lua_type(lstate, 1) != LUA_TSTRING {
            luaL_argerror(lstate, 1, c"expected string");
        }

        let mut str_ = lua_tolstring(lstate, 1, ptr::null_mut());

        let wo_spell_save = curwin_mut().w_onebuf_opt.wo_spell;

        if curwin_mut().w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin_mut());
            curwin_mut().w_onebuf_opt.wo_spell = 1;
        }

        let w_s = curwin_mut().w_s_ptr();
        if *(*w_s).b_p_spl == NUL {
            emsg(gettext(e_no_spell));
            curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
            return 0;
        }

        let mut pos: usize = 0;
        let mut capcol: c_int = -1;
        let mut no_res: c_int = 0;

        lua_createtable(lstate, 0, 0);

        while *str_ != NUL {
            let mut attr: hlf_T = HLF_COUNT;
            let len = spell_check(
                curwin_mut(),
                str_.cast_mut(),
                &mut attr,
                Some(&mut capcol),
                false,
            );
            assert!(len <= c_int::MAX as usize);

            if attr != HLF_COUNT {
                lua_createtable(lstate, 3, 0);

                lua_pushlstring(lstate, str_, len);
                lua_rawseti(lstate, -2, 1);

                let result = match attr {
                    HLF_SPB => c"bad".as_ptr(),
                    HLF_SPR => c"rare".as_ptr(),
                    HLF_SPL => c"local".as_ptr(),
                    HLF_SPC => c"caps".as_ptr(),
                    _ => ptr::null(),
                };
                assert!(!result.is_null());

                lua_pushstring(lstate, result);
                lua_rawseti(lstate, -2, 2);

                lua_pushinteger(lstate, (pos as lua_Integer) + 1);
                lua_rawseti(lstate, -2, 3);

                no_res += 1;
                lua_rawseti(lstate, -2, no_res);
            }

            str_ = str_.add(len);
            pos += len;
            capcol -= len as c_int;
        }

        curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
        1
    }
}

/// Open the `vim.spell` Lua library (`luaopen_spell`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaopen_spell(lstate: *mut lua_State) -> c_int {
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
    unsafe {
        lua_createtable(lstate, 0, 0);
        luaL_register(lstate, ptr::null(), SPELL_FUNCTIONS.as_ptr());
        1
    }
}
