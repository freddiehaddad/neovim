//! Lua standard library and internal `vim.*` C bindings (`src/nvim/lua/stdlib.c`).

use crate::api::private::helpers::{
    TryState, api_clear_error, dict_check_writable, find_buffer_by_handle, find_tab_by_handle,
    find_window_by_handle, try_enter, try_leave, tv_dict_is_watched,
};
use crate::api::private::validate::{Error, ErrorType, NvimString};
use crate::api::win_config::kCtxNoDisplay;
use crate::api::window::{kCtxKeepCwd, kCtxValidate};
use crate::buffer::buf_T;
use crate::charset::NUL;
use crate::context::kCtxKeepDirs;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, handle_get_buffer, handle_get_window};
use crate::cursor::linenr_T;
use crate::eval::typval::{
    dict_T, tv_clear, tv_copy, tv_dict_add, tv_dict_find, tv_dict_item_alloc_len,
    tv_dict_item_remove, tv_dict_watcher_notify, typval_T,
};
use crate::eval::vars::{before_set_vvar, get_globvar_dict, get_vimvar_dict};
use crate::ex_docmd::{
    CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS, CMOD_KEEPPATTERNS,
    CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, apply_cmdmod,
    undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::extmark::kTrue;
use crate::fold::foldUpdate;
use crate::globals::*;
use crate::keycodes::{keychord, mod_mask_table, vim_strsave_escape_ks};
use crate::log::log_st;
use crate::lua::base64::luaopen_base64;
use crate::lua::converter::{
    LUA_MULTRET, LUA_REGISTRYINDEX, LUA_TSTRING, lua_Integer, lua_State, lua_concat,
    lua_createtable, lua_error, lua_getfield, lua_getglobal, lua_gettop, lua_isnil,
    lua_isnoneornil, lua_istable, lua_newtable, lua_newuserdata, lua_next, lua_objlen, lua_pcall,
    lua_pop, lua_pushcfunction, lua_pushinteger, lua_pushlstring, lua_pushnil, lua_pushnumber,
    lua_pushstring, lua_pushvalue, lua_rawseti, lua_setfield, lua_setmetatable, lua_toboolean,
    lua_tolstring, lua_tostring, lua_type, luaL_Reg, luaL_argerror, luaL_checkinteger,
    luaL_checklstring, luaL_checkstring, luaL_checkudata, luaL_error, luaL_newmetatable,
    luaL_register, luaL_where, nlua_pop_typval, nlua_push_typval,
};
use crate::lua::spell::luaopen_spell;
use crate::lua::xdiff::nlua_xdl_diff;
use crate::mbyte::{
    convert_setup_ext, enc_canonize, enc_skip, mb_stricmp, mb_utf_index_to_bytes, mb_utflen,
    string_convert, utf_cp_bounds_len, utf_ptr2len_len, vimconv_T,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::str2special;
use crate::msgpack_rpc::packer::handle_T;
use crate::regexp::{
    RE_AUTO, RE_MAGIC, RE_NOBREAK, RE_STRICT, regmatch_T, regprog_T, vim_regcomp, vim_regexec,
    vim_regfree,
};
use crate::runtime::script_autoload;
use crate::viml::parser::parser::CONV_NONE;
use crate::window::{win_T, win_find_tabpage};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

static REGEX_META: [luaL_Reg; 5] = [
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(regex_gc),
    },
    luaL_Reg {
        name: c"__tostring".as_ptr(),
        func: Some(regex_tostring),
    },
    luaL_Reg {
        name: c"match_str".as_ptr(),
        func: Some(regex_match_str),
    },
    luaL_Reg {
        name: c"match_line".as_ptr(),
        func: Some(regex_match_line),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

use crate::cjson::lua_cjson::lua_cjson_new;
use crate::lpeg::lptree::luaopen_lpeg;
use crate::lua::converter::lua_isstring;
use crate::mpack::lmpack::luaopen_mpack;

unsafe fn regex_check(lstate: *mut lua_State) -> *mut *mut regprog_T {
    unsafe { luaL_checkudata(lstate, 1, c"nvim_regex".as_ptr()).cast::<*mut regprog_T>() }
}

unsafe fn regex_match(
    lstate: *mut lua_State,
    prog: *mut *mut regprog_T,
    str_: *mut c_char,
) -> c_int {
    unsafe {
        let mut rm: regmatch_T = regmatch_T::new();
        rm.regprog = *prog;
        rm.rm_ic = false;
        let matched = vim_regexec(&mut rm, str_, 0);
        *prog = rm.regprog;

        if matched {
            lua_pushinteger(
                lstate,
                ((rm.startp[0] as isize) - (str_ as isize)) as lua_Integer,
            );
            lua_pushinteger(
                lstate,
                ((rm.endp[0] as isize) - (str_ as isize)) as lua_Integer,
            );
            return 2;
        }
        0
    }
}

unsafe extern "C-unwind" fn regex_match_str(lstate: *mut lua_State) -> c_int {
    unsafe {
        let prog = regex_check(lstate);
        let str_ = luaL_checkstring(lstate, 2);
        let nret = regex_match(lstate, prog, str_.cast_mut());

        if (*prog).is_null() {
            return luaL_error(lstate, c"regex: internal error".as_ptr());
        }

        nret
    }
}

unsafe extern "C-unwind" fn regex_match_line(lstate: *mut lua_State) -> c_int {
    unsafe {
        let prog = regex_check(lstate);

        let narg = lua_gettop(lstate);
        if narg < 3 {
            return luaL_error(lstate, c"not enough args".as_ptr());
        }

        let bufnr = luaL_checkinteger(lstate, 2) as handle_T;
        let rownr = luaL_checkinteger(lstate, 3) as linenr_T;
        let mut start: c_int = 0;
        let mut end: c_int = -1;
        if narg >= 4 {
            start = luaL_checkinteger(lstate, 4) as c_int;
        }
        if narg >= 5 {
            end = luaL_checkinteger(lstate, 5) as c_int;
            if end < 0 {
                return luaL_error(lstate, c"invalid end".as_ptr());
            }
        }

        let buf = if bufnr != 0 {
            handle_get_buffer(bufnr)
        } else {
            g_st().curbuf
        };
        if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
            return luaL_error(lstate, c"invalid buffer".as_ptr());
        }

        if rownr >= (*buf).b_ml.ml_line_count {
            return luaL_error(lstate, c"invalid row".as_ptr());
        }

        let line = ml_get_buf(buf, rownr + 1);
        let len = ml_get_buf_len(buf, rownr + 1);

        if start < 0 || start > len {
            return luaL_error(lstate, c"invalid start".as_ptr());
        }

        let mut save = NUL;
        if end >= 0 {
            if end > len || end < start {
                return luaL_error(lstate, c"invalid end".as_ptr());
            }
            save = *line.add(end as usize);
            *line.add(end as usize) = NUL;
        }

        let nret = regex_match(lstate, prog, line.add(start as usize));

        if end >= 0 {
            *line.add(end as usize) = save;
        }

        if (*prog).is_null() {
            return luaL_error(lstate, c"regex: internal error".as_ptr());
        }

        nret
    }
}

unsafe extern "C-unwind" fn regex_gc(lstate: *mut lua_State) -> c_int {
    unsafe {
        let prog = regex_check(lstate);
        vim_regfree(*prog);
    }
    0
}

unsafe extern "C-unwind" fn regex_tostring(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_pushstring(lstate, c"<regex>".as_ptr());
    }
    1
}

/// Convert byte index to UTF-32 and UTF-16 indices.
pub unsafe extern "C-unwind" fn nlua_str_utfindex(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let idx: isize = if lua_isnoneornil(lstate, 2) {
            s1_len as isize
        } else {
            let i = luaL_checkinteger(lstate, 2);
            if i < 0 || (i as usize) > s1_len {
                lua_pushnil(lstate);
                lua_pushnil(lstate);
                return 2;
            }
            i
        };

        let mut codepoints: usize = 0;
        let mut codeunits: usize = 0;
        mb_utflen(s1, idx as usize, &raw mut codepoints, &raw mut codeunits);

        lua_pushinteger(lstate, codepoints as lua_Integer);
        lua_pushinteger(lstate, codeunits as lua_Integer);

        2
    }
}

unsafe extern "C-unwind" fn nlua_str_utf_pos(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        lua_newtable(lstate);

        let mut idx: c_int = 1;
        let mut i: usize = 0;
        while i < s1_len && *s1.add(i) != NUL {
            let clen = utf_ptr2len_len(s1.add(i), (s1_len - i) as c_int) as usize;
            lua_pushinteger(lstate, (i as lua_Integer) + 1);
            lua_rawseti(lstate, -2, idx);
            idx += 1;
            i += clen;
        }

        1
    }
}

unsafe extern "C-unwind" fn nlua_str_utf_start(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let offset = luaL_checkinteger(lstate, 2);
        if offset <= 0 || (offset as usize) > s1_len {
            return luaL_error(lstate, c"index out of range".as_ptr());
        }
        let off = (offset as usize) - 1;
        let head_off =
            -c_int::from(utf_cp_bounds_len(s1, s1.add(off), (s1_len - off) as c_int).begin_off);
        lua_pushinteger(lstate, head_off as lua_Integer);
        1
    }
}

unsafe extern "C-unwind" fn nlua_str_utf_end(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let offset = luaL_checkinteger(lstate, 2);
        if offset <= 0 || (offset as usize) > s1_len {
            return luaL_error(lstate, c"index out of range".as_ptr());
        }
        let off = (offset as usize) - 1;
        let tail_off =
            c_int::from(utf_cp_bounds_len(s1, s1.add(off), (s1_len - off) as c_int).end_off) - 1;
        lua_pushinteger(lstate, tail_off as lua_Integer);
        1
    }
}

/// Convert UTF-32 or UTF-16 indices to byte index.
pub unsafe extern "C-unwind" fn nlua_str_byteindex(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let idx = luaL_checkinteger(lstate, 2);
        if idx < 0 {
            lua_pushnil(lstate);
            return 1;
        }
        let mut use_utf16 = false;
        if lua_gettop(lstate) >= 3 {
            use_utf16 = lua_toboolean(lstate, 3) != 0;
        }

        let byteidx = mb_utf_index_to_bytes(s1, s1_len, idx as usize, use_utf16);
        if byteidx == -1 {
            lua_pushnil(lstate);
            return 1;
        }

        lua_pushinteger(lstate, byteidx as lua_Integer);
        1
    }
}

pub unsafe extern "C-unwind" fn nlua_regex(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut err = Error::new();
        let text = luaL_checkstring(lstate, 1);

        let mut tstate = TryState::new();
        try_enter(&raw mut tstate);
        let prog = vim_regcomp(text, RE_AUTO | RE_MAGIC | RE_STRICT | RE_NOBREAK);
        try_leave(&raw const tstate, &mut err);

        if err.err_type != ErrorType::kErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushlstring(lstate, b"couldn't parse regex: ".as_ptr().cast(), 22);
            lua_pushstring(lstate, err.msg_ptr());
            lua_concat(lstate, 3);
            api_clear_error(&mut err);
            return lua_error(lstate);
        } else if prog.is_null() {
            luaL_where(lstate, 1);
            lua_pushlstring(lstate, b"couldn't parse regex".as_ptr().cast(), 20);
            lua_concat(lstate, 2);
            return lua_error(lstate);
        }

        let p = lua_newuserdata(lstate, size_of::<*mut regprog_T>()).cast::<*mut regprog_T>();
        *p = prog;

        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim_regex".as_ptr());
        let _ = lua_setmetatable(lstate, -2);
        1
    }
}

unsafe fn nlua_get_var_scope(lstate: *mut lua_State) -> *mut dict_T {
    unsafe {
        let scope = luaL_checkstring(lstate, 1);
        let handle = luaL_checkinteger(lstate, 2) as handle_T;
        let mut dict: *mut dict_T = ptr::null_mut();
        let mut err = Error::new();

        match CStr::from_ptr(scope).to_bytes() {
            b"g" => dict = get_globvar_dict(),
            b"v" => dict = get_vimvar_dict(),
            b"b" => {
                let buf = find_buffer_by_handle(handle, &mut err);
                if !buf.is_null() {
                    dict = (*buf).b_vars();
                }
            }
            b"w" => {
                let win = find_window_by_handle(handle, &mut err);
                if !win.is_null() {
                    dict = (*win).w_vars();
                }
            }
            b"t" => {
                let tabpage = find_tab_by_handle(handle, &mut err);
                if !tabpage.is_null() {
                    dict = (*tabpage).tp_vars();
                }
            }
            _ => {
                luaL_error(lstate, c"invalid scope".as_ptr());
                return ptr::null_mut();
            }
        }

        if err.err_type != ErrorType::kErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushlstring(lstate, b"scoped variable: ".as_ptr().cast(), 17);
            lua_pushstring(lstate, err.msg_ptr());
            lua_concat(lstate, 3);
            api_clear_error(&mut err);
            lua_error(lstate);
            return ptr::null_mut();
        }
        dict
    }
}

pub unsafe extern "C-unwind" fn nlua_setvar(lstate: *mut lua_State) -> c_int {
    unsafe {
        let dict = nlua_get_var_scope(lstate);
        let mut key_size: usize = 0;
        let key_data = luaL_checklstring(lstate, 3, &raw mut key_size).cast_mut();
        let key = NvimString {
            data: key_data,
            size: key_size,
        };

        let del = lua_gettop(lstate) < 4 || lua_isnil(lstate, 4);

        let mut err = Error::new();
        let mut di = dict_check_writable(dict, key, del, &mut err);
        if err.err_type != ErrorType::kErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg_ptr());
            lua_concat(lstate, 2);
            api_clear_error(&mut err);
            lua_error(lstate);
            return 0;
        }

        let watched = tv_dict_is_watched(dict);

        if del {
            if di.is_null() {
                return 0;
            }
            if watched {
                tv_dict_watcher_notify(dict, key.data, ptr::null_mut(), &raw mut (*di).di_tv);
            }
            tv_dict_item_remove(dict, di);
        } else {
            let mut tv: typval_T = typval_T::new();

            lua_pushvalue(lstate, 4);
            if !nlua_pop_typval(lstate, &raw mut tv) {
                return luaL_error(lstate, c"Couldn't convert lua value".as_ptr());
            }

            let mut oldtv: typval_T = typval_T::new();

            if di.is_null() {
                di = tv_dict_item_alloc_len(key.data, key.size);
                let _ = tv_dict_add(dict, di);
            } else {
                let mut type_error = false;
                if dict == get_vimvar_dict()
                    && !before_set_vvar(
                        key.data,
                        di,
                        &raw mut tv,
                        true,
                        watched,
                        &raw mut type_error,
                    )
                {
                    tv_clear(&raw mut tv);
                    if type_error {
                        return luaL_error(
                            lstate,
                            c"Setting v:%s to value with wrong type".as_ptr(),
                            key.data,
                        );
                    }
                    return 0;
                }
                if watched {
                    tv_copy(&raw const (*di).di_tv, &raw mut oldtv);
                }
                tv_clear(&raw mut (*di).di_tv);
            }

            tv_copy(&raw const tv, &raw mut (*di).di_tv);

            if watched {
                tv_dict_watcher_notify(dict, key.data, &raw mut tv, &raw mut oldtv);
                tv_clear(&raw mut oldtv);
            }

            tv_clear(&raw mut tv);
        }
        0
    }
}

pub unsafe extern "C-unwind" fn nlua_getvar(lstate: *mut lua_State) -> c_int {
    unsafe {
        let dict = nlua_get_var_scope(lstate);
        let mut len: usize = 0;
        let name = luaL_checklstring(lstate, 3, &raw mut len);

        let mut di = tv_dict_find(dict, name, len as isize);
        if di.is_null() && dict == get_globvar_dict() {
            if !script_autoload(name, len, false) || aborting() {
                return 0;
            }
            di = tv_dict_find(dict, name, len as isize);
        }
        if di.is_null() {
            return 0;
        }
        nlua_push_typval(lstate, &raw mut (*di).di_tv, 0);
        1
    }
}

unsafe extern "C-unwind" fn nlua_stricmp(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut s1_len: usize = 0;
        let mut s2_len: usize = 0;
        let mut s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let mut s2 = luaL_checklstring(lstate, 2, &raw mut s2_len);
        let mut ret: c_int;
        assert_eq!(*s1.add(s1_len), NUL);
        assert_eq!(*s2.add(s2_len), NUL);

        loop {
            let slice1 = std::slice::from_raw_parts(s1.cast::<u8>(), s1_len);
            let slice2 = std::slice::from_raw_parts(s2.cast::<u8>(), s2_len);
            let nul1 = slice1.iter().position(|&b| b == 0);
            let nul2 = slice2.iter().position(|&b| b == 0);
            ret = mb_stricmp(s1, s2);
            if ret == 0 {
                if nul1.is_some() != nul2.is_some() {
                    ret = c_int::from(nul1.is_some()) - c_int::from(nul2.is_some());
                    break;
                }
                if let (Some(p1), Some(p2)) = (nul1, nul2) {
                    s1_len -= p1 + 1;
                    s2_len -= p2 + 1;
                    s1 = s1.add(p1 + 1);
                    s2 = s2.add(p2 + 1);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        lua_pop(lstate, 2);
        lua_pushnumber(
            lstate,
            f64::from(c_int::from(ret > 0) - c_int::from(ret < 0)),
        );
        1
    }
}

unsafe extern "C-unwind" fn nlua_iconv(lstate: *mut lua_State) -> c_int {
    unsafe {
        let narg = lua_gettop(lstate);
        if narg < 3 {
            return luaL_error(lstate, c"Expected at least 3 arguments".as_ptr());
        }

        for i in 1..=3 {
            if lua_type(lstate, i) != LUA_TSTRING {
                return luaL_argerror(lstate, i, c"expected string".as_ptr());
            }
        }

        let mut str_len: usize = 0;
        let str_ = lua_tolstring(lstate, 1, &raw mut str_len);

        let from = enc_canonize(enc_skip(
            lua_tolstring(lstate, 2, ptr::null_mut()).cast_mut(),
        ));
        let to = enc_canonize(enc_skip(
            lua_tolstring(lstate, 3, ptr::null_mut()).cast_mut(),
        ));

        let mut vimconv = vimconv_T {
            vc_type: CONV_NONE,
            vc_factor: 0,
            vc_fd: None,
            vc_fail: false,
        };
        let _ = convert_setup_ext(
            &mut vimconv,
            from.as_ptr().cast_mut(),
            false,
            to.as_ptr().cast_mut(),
            false,
        );

        let ret = string_convert(&vimconv, str_.cast_mut(), &raw mut str_len);

        match ret {
            None => lua_pushnil(lstate),
            Some(bytes) => lua_pushlstring(lstate, bytes.as_ptr().cast(), str_len),
        }

        1
    }
}

unsafe extern "C-unwind" fn nlua_foldupdate(lstate: *mut lua_State) -> c_int {
    unsafe {
        let window = luaL_checkinteger(lstate, 1) as handle_T;
        let win = handle_get_window(window);
        if win.is_null() {
            return luaL_error(lstate, c"invalid window".as_ptr());
        }
        let top = (luaL_checkinteger(lstate, 2) as linenr_T) + 1;
        if top < 1 {
            return luaL_error(lstate, c"invalid top".as_ptr());
        }
        let bot = luaL_checkinteger(lstate, 3) as linenr_T;
        if top > bot {
            return luaL_error(lstate, c"invalid bot".as_ptr());
        }

        foldUpdate(win, top, bot);
        0
    }
}

unsafe extern "C-unwind" fn nlua_with(lstate: *mut lua_State) -> c_int {
    unsafe {
        let mut flags: c_int = 0;
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut win: *mut win_T = ptr::null_mut();
        let mut keepcwd = false;
        let mut log_level: c_int = -1;

        if !lua_istable(lstate, 1) {
            return luaL_argerror(lstate, 1, c"table expected".as_ptr());
        }
        lua_pushnil(lstate);
        while lua_next(lstate, 1) != 0 {
            if lua_type(lstate, -2) == LUA_TSTRING {
                let k = CStr::from_ptr(lua_tostring(lstate, -2)).to_bytes();
                let v = lua_toboolean(lstate, -1) != 0;
                match k {
                    b"buf" => buf = handle_get_buffer(luaL_checkinteger(lstate, -1) as c_int),
                    b"win" => win = handle_get_window(luaL_checkinteger(lstate, -1) as c_int),
                    b"keepcwd" => keepcwd = v,
                    b"log_level" => log_level = luaL_checkinteger(lstate, -1) as c_int,
                    b"sandbox" if v => flags |= CMOD_SANDBOX,
                    b"silent" if v => flags |= CMOD_SILENT,
                    b"emsg_silent" if v => flags |= CMOD_ERRSILENT,
                    b"unsilent" if v => flags |= CMOD_UNSILENT,
                    b"noautocmd" if v => flags |= CMOD_NOAUTOCMD,
                    b"hide" if v => flags |= CMOD_HIDE,
                    b"keepalt" if v => flags |= CMOD_KEEPALT,
                    b"keepmarks" if v => flags |= CMOD_KEEPMARKS,
                    b"keepjumps" if v => flags |= CMOD_KEEPJUMPS,
                    b"lockmarks" if v => flags |= CMOD_LOCKMARKS,
                    b"keeppatterns" if v => flags |= CMOD_KEEPPATTERNS,
                    _ => {}
                }
            }
            lua_pop(lstate, 1);
        }

        let mut status: c_int = 0;
        let mut rets: c_int = 0;

        if (flags & CMOD_ERRSILENT) != 0 {
            flags |= CMOD_SILENT;
        }

        let save_min_log_level = log_st().g_min_log_level;
        if log_level >= 0 {
            log_st().g_min_log_level = log_level;
        }
        let save_cmdmod = ptr::replace(&raw mut g_st().cmdmod, crate::ex_docmd::cmdmod_T::new());
        g_st().cmdmod.cmod_flags = flags;
        apply_cmdmod(&raw mut g_st().cmdmod);

        let mut err = Error::new();
        let mut tstate = TryState::new();
        try_enter(&raw mut tstate);
        {
            let mut cs: CtxSwitch = CtxSwitch::new();
            let mut switched = true;

            if !win.is_null() {
                buf = ptr::null_mut();
            }

            if !win.is_null() || !buf.is_null() || keepcwd {
                let dirs = if keepcwd { kCtxKeepDirs } else { kCtxKeepCwd };
                let tab = if !win.is_null() {
                    win_find_tabpage(win)
                } else {
                    ptr::null_mut()
                };
                switched = ctx_switch(
                    &raw mut cs,
                    win,
                    tab,
                    buf,
                    kCtxNoDisplay | dirs | if !win.is_null() { kCtxValidate } else { 0 },
                );
            }

            if switched {
                let s = lua_gettop(lstate);
                lua_pushvalue(lstate, 2);
                status = lua_pcall(lstate, 0, LUA_MULTRET, 0);
                rets = lua_gettop(lstate) - s;
            }

            ctx_restore(&raw mut cs);
        }
        try_leave(&raw const tstate, &mut err);

        undo_cmdmod(&raw mut g_st().cmdmod);
        g_st().cmdmod = save_cmdmod;
        if log_level >= 0 {
            log_st().g_min_log_level = save_min_log_level;
        }

        if status != 0 {
            return lua_error(lstate);
        } else if err.err_type != ErrorType::kErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg_ptr());
            lua_concat(lstate, 2);
            api_clear_error(&mut err);
            return lua_error(lstate);
        }

        rets
    }
}

unsafe extern "C-unwind" fn nlua_keyparse(lstate: *mut lua_State) -> c_int {
    unsafe {
        if lua_isstring(lstate, 1) == 0 {
            return luaL_argerror(lstate, 1, c"string expected".as_ptr());
        }

        let escaped = vim_strsave_escape_ks(lua_tostring(lstate, 1));
        let mut p: *const c_char = escaped.as_ptr();

        lua_newtable(lstate);

        let mut data = keychord::new();

        while *p != NUL {
            let keys = str2special(&raw mut p, true, kTrue, &raw mut data);

            lua_createtable(lstate, 0, 3);

            lua_newtable(lstate);
            let mut i: usize = 0;
            while mod_mask_table[i].name != b'A' as c_char {
                if (data.mods & c_int::from(mod_mask_table[i].mod_mask))
                    == c_int::from(mod_mask_table[i].mod_flag)
                {
                    lua_pushlstring(lstate, &raw const mod_mask_table[i].name, 1);
                    lua_rawseti(lstate, -2, (lua_objlen(lstate, -2) as c_int) + 1);
                }
                i += 1;
            }
            lua_setfield(lstate, -2, c"mod".as_ptr());

            lua_pushlstring(lstate, data.key.data, data.key.size);
            lua_setfield(lstate, -2, c"key".as_ptr());

            lua_pushstring(lstate, keys);
            lua_setfield(lstate, -2, c"keys".as_ptr());

            if data.key_alt.size != 0 {
                lua_pushlstring(lstate, data.key_alt.data, data.key_alt.size);
                lua_setfield(lstate, -2, c"key_alt".as_ptr());
            }

            lua_rawseti(lstate, -2, (lua_objlen(lstate, -2) as c_int) + 1);
        }

        1
    }
}

unsafe fn nlua_state_add_internal(lstate: *mut lua_State) {
    unsafe {
        lua_pushcfunction(lstate, Some(nlua_getvar));
        lua_setfield(lstate, -2, c"_getvar".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_setvar));
        lua_setfield(lstate, -2, c"_setvar".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_foldupdate));
        lua_setfield(lstate, -2, c"_foldupdate".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_with));
        lua_setfield(lstate, -2, c"_with_c".as_ptr());

        lua_getfield(lstate, -1, c"_core".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_keyparse));
        lua_setfield(lstate, -2, c"keyparse".as_ptr());
        lua_pop(lstate, 1);
    }
}

pub unsafe extern "C-unwind" fn nlua_state_add_stdlib(lstate: *mut lua_State, is_thread: bool) {
    unsafe {
        if !is_thread {
            lua_pushcfunction(lstate, Some(nlua_stricmp));
            lua_setfield(lstate, -2, c"stricmp".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_str_utfindex));
            lua_setfield(lstate, -2, c"_str_utfindex".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_str_byteindex));
            lua_setfield(lstate, -2, c"_str_byteindex".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_str_utf_pos));
            lua_setfield(lstate, -2, c"str_utf_pos".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_str_utf_start));
            lua_setfield(lstate, -2, c"str_utf_start".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_str_utf_end));
            lua_setfield(lstate, -2, c"str_utf_end".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_regex));
            lua_setfield(lstate, -2, c"regex".as_ptr());
            let _ = luaL_newmetatable(lstate, c"nvim_regex".as_ptr());
            luaL_register(lstate, ptr::null(), (&raw const REGEX_META).cast());

            lua_pushvalue(lstate, -1);
            lua_setfield(lstate, -2, c"__index".as_ptr());
            lua_pop(lstate, 1);

            let _ = luaopen_spell(lstate);
            lua_setfield(lstate, -2, c"spell".as_ptr());

            lua_pushcfunction(lstate, Some(nlua_iconv));
            lua_setfield(lstate, -2, c"iconv".as_ptr());

            let _ = luaopen_base64(lstate);
            lua_setfield(lstate, -2, c"base64".as_ptr());

            nlua_state_add_internal(lstate);
        }

        let _ = luaopen_mpack(lstate);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, -3, c"mpack".as_ptr());

        lua_getglobal(lstate, c"package".as_ptr());
        lua_getfield(lstate, -1, c"loaded".as_ptr());
        lua_pushvalue(lstate, -3);
        lua_setfield(lstate, -2, c"mpack".as_ptr());
        lua_pop(lstate, 3);

        let _ = luaopen_lpeg(lstate);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, -4, c"lpeg".as_ptr());

        lua_getglobal(lstate, c"package".as_ptr());
        lua_getfield(lstate, -1, c"loaded".as_ptr());
        lua_pushvalue(lstate, -3);
        lua_setfield(lstate, -2, c"lpeg".as_ptr());
        lua_pop(lstate, 4);

        lua_pushcfunction(lstate, Some(nlua_xdl_diff));
        lua_setfield(lstate, -2, c"diff".as_ptr());

        let _ = lua_cjson_new(lstate);
        lua_setfield(lstate, -2, c"json".as_ptr());

        #[cfg(feature = "vendor-bit")]
        {
            let top = lua_gettop(lstate);
            let _ = crate::bit::nvim_luaopen_bit(lstate);
            crate::lua::converter::lua_settop(lstate, top);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_meta_table_has_expected_entries() {
        assert_eq!(REGEX_META.len(), 5);
        assert!(REGEX_META[4].name.is_null());
        assert!(REGEX_META[4].func.is_none());
    }
}
