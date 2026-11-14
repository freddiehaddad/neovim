#![forbid(unsafe_code)]
//! Lua standard library and internal `vim.*` C bindings (`src/nvim/lua/stdlib.c`).

use crate::api::private::helpers::{
    TryState, api_clear_error, dict_check_writable, find_buffer_by_handle, find_tab_by_handle,
    find_window_by_handle, try_enter, try_leave, tv_dict_is_watched,
};
use crate::api::private::validate::{Error, ErrorType};
use crate::api::win_config::K_CTX_NO_DISPLAY;
use crate::api::window::{K_CTX_KEEP_CWD, K_CTX_VALIDATE};
use crate::charset::NUL;
use crate::context::K_CTX_KEEP_DIRS;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, handle_get_buffer, handle_get_window};
use crate::cursor::Linenr;
use crate::eval::typval::{
    TypVal, tv_clear, tv_copy, tv_dict_add, tv_dict_find, tv_dict_item_alloc_len,
    tv_dict_item_remove, tv_dict_watcher_notify,
};
use crate::eval::vars::{before_set_vvar, get_globvar_dict, get_vimvar_dict};
use crate::ex_docmd::{
    CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS, CMOD_KEEPPATTERNS,
    CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, apply_cmdmod,
    undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::extmark::K_TRUE;
use crate::fold::fold_update;
use crate::globals::*;
use crate::keycodes::{KeyChord, MOD_MASK_TABLE, vim_strsave_escape_ks};
use crate::log::{min_log_level, set_min_log_level};
use crate::lua::base64::luaopen_base64;
use crate::lua::converter::{
    LUA_MULTRET, LUA_REGISTRYINDEX, LUA_TSTRING, lua_Integer, lua_State, lua_concat_safe,
    lua_createtable_safe, lua_error_safe, lua_getfield, lua_getglobal, lua_isnil, lua_isnoneornil,
    lua_isstring_safe, lua_istable, lua_newtable, lua_newuserdata_typed, lua_next_safe,
    lua_objlen_safe, lua_pcall_safe, lua_pop, lua_push_bytes, lua_pushboolean_safe,
    lua_pushcfunction_safe, lua_pushinteger_safe, lua_pushnil_safe, lua_pushnumber_safe,
    lua_pushstring, lua_pushvalue_safe, lua_rawseti_safe, lua_register_funcs, lua_setfield,
    lua_setmetatable_safe, lua_toboolean_safe, lua_tolstring_bytes, lua_tolstring_cstr,
    lua_tostring, lua_type_safe, luaL_argerror, luaL_check_nvim_string, luaL_checkbytes,
    luaL_checkbytes_with_nul, luaL_checkinteger_safe, luaL_checkstring, luaL_checkudata_typed,
    luaL_error_cstr, luaL_error_cstr_arg, luaL_newmetatable, luaL_where_safe, nlua_gettop,
    nlua_pop_typval, nlua_push_typval,
};
use crate::lua::spell::luaopen_spell;
use crate::lua::xdiff::nlua_xdl_diff;
use crate::mbyte::{
    Vimconv, convert_setup_ext, enc_canonize, enc_skip, mb_stricmp, mb_utf_index_to_bytes,
    mb_utflen, string_convert, utf_cp_bounds, utf_ptr2len_len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len, with_ml_get_buf_mut_slice};
use crate::message::str2special;
use crate::msgpack_rpc::packer::ObjHandle;
use crate::path::path_with_url;
use crate::regexp::{RE_AUTO, RE_MAGIC, RE_NOBREAK, RE_STRICT, Regmatch, vim_regcomp, vim_regexec};
use crate::runtime::script_autoload;
use crate::viml::parser::parser::CONV_NONE;
use crate::window::win_find_tabpage;
use std::ffi::CStr;

static REGEX_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 4] = [
    (c"__gc", regex_gc),
    (c"__tostring", regex_tostring),
    (c"match_str", regex_match_str),
    (c"match_line", regex_match_line),
];

use crate::cjson::lua_cjson::luaopen_cjson;
use crate::lpeg::lptree::luaopen_lpeg;
use crate::mpack::lmpack::luaopen_mpack;

struct LuaRegex {
    prog: crate::regexp::RegprogPtr,
}

fn regex_check(lstate: *mut lua_State) -> &'static mut LuaRegex {
    luaL_checkudata_typed::<LuaRegex>(lstate, 1, c"nvim_regex")
}

fn regex_match(lstate: *mut lua_State, regex: &mut LuaRegex, str_: &CStr) -> i32 {
    let mut rm: Regmatch = Regmatch::new();
    rm.regprog = regex.prog.clone();
    rm.rm_ic = false;
    let matched = vim_regexec(&mut rm, str_, 0);

    if matched {
        lua_pushinteger_safe(lstate, rm.start_offset(0, str_) as lua_Integer);
        lua_pushinteger_safe(lstate, rm.end_offset(0, str_) as lua_Integer);
        return 2;
    }
    0
}

extern "C-unwind" fn regex_match_str(lstate: *mut lua_State) -> i32 {
    let regex = regex_check(lstate);
    let str_ = luaL_checkstring(lstate, 2);
    let nret = regex_match(lstate, regex, str_);

    if regex.prog.is_null() {
        return luaL_error_cstr(lstate, c"regex: internal error");
    }

    nret
}

extern "C-unwind" fn regex_match_line(lstate: *mut lua_State) -> i32 {
    let regex = regex_check(lstate);

    let narg = nlua_gettop(lstate);
    if narg < 3 {
        return luaL_error_cstr(lstate, c"not enough args");
    }

    let bufnr = luaL_checkinteger_safe(lstate, 2) as ObjHandle;
    let rownr = luaL_checkinteger_safe(lstate, 3) as Linenr;
    let mut start: i32 = 0;
    let mut end: i32 = -1;
    if narg >= 4 {
        start = luaL_checkinteger_safe(lstate, 4) as i32;
    }
    if narg >= 5 {
        end = luaL_checkinteger_safe(lstate, 5) as i32;
        if end < 0 {
            return luaL_error_cstr(lstate, c"invalid end");
        }
    }

    let buf = if bufnr != 0 {
        crate::globals::BufPtr::from(handle_get_buffer(bufnr))
    } else {
        g_st().curbuf.get()
    };
    let Some(buf_ref) = buf.opt() else {
        return luaL_error_cstr(lstate, c"invalid buffer");
    };
    if buf_ref.b_ml.ml_mfp.borrow().is_none() {
        return luaL_error_cstr(lstate, c"invalid buffer");
    }

    if rownr >= buf_ref.b_ml.ml_line_count.get() {
        return luaL_error_cstr(lstate, c"invalid row");
    }

    let len = ml_get_buf_len(buf_ref, rownr + 1);

    if start < 0 || start > len {
        return luaL_error_cstr(lstate, c"invalid start");
    }

    let mut save = 0u8;
    if end >= 0 {
        if end > len || end < start {
            return luaL_error_cstr(lstate, c"invalid end");
        }
        if end < len {
            with_ml_get_buf_mut_slice(buf_ref, rownr + 1, |line_slice| {
                save = line_slice[end as usize];
                line_slice[end as usize] = 0;
            });
        }
    }

    let line_cstr = ml_get_buf(buf_ref, rownr + 1);
    let nret = regex_match(lstate, regex, &line_cstr[start as usize..]);

    if end >= 0 && end < len {
        with_ml_get_buf_mut_slice(buf_ref, rownr + 1, |line_restore| {
            line_restore[end as usize] = save;
        });
    }

    if regex.prog.is_null() {
        return luaL_error_cstr(lstate, c"regex: internal error");
    }

    nret
}

extern "C-unwind" fn regex_gc(lstate: *mut lua_State) -> i32 {
    let regex = regex_check(lstate);
    regex.prog = crate::regexp::RegprogPtr::NULL;
    0
}

extern "C-unwind" fn regex_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"<regex>");
    1
}

/// Convert byte index to UTF-32 and UTF-16 indices.
pub extern "C-unwind" fn nlua_str_utfindex(lstate: *mut lua_State) -> i32 {
    let s1_bytes = luaL_checkbytes(lstate, 1);
    let s1_len = s1_bytes.len();
    let idx: isize = if lua_isnoneornil(lstate, 2) {
        s1_len as isize
    } else {
        let i = luaL_checkinteger_safe(lstate, 2);
        if i < 0 || (i as usize) > s1_len {
            lua_pushnil_safe(lstate);
            lua_pushnil_safe(lstate);
            return 2;
        }
        i
    };

    let mut codepoints: usize = 0;
    let mut codeunits: usize = 0;
    mb_utflen(s1_bytes, idx as usize, &mut codepoints, &mut codeunits);

    lua_pushinteger_safe(lstate, codepoints as lua_Integer);
    lua_pushinteger_safe(lstate, codeunits as lua_Integer);

    2
}

extern "C-unwind" fn nlua_str_utf_pos(lstate: *mut lua_State) -> i32 {
    let s1_raw = luaL_checkbytes(lstate, 1);
    let s1_len = s1_raw.len();
    lua_newtable(lstate);

    let s1_bytes = crate::memory::bytes_until_nul(s1_raw);
    let mut idx: i32 = 1;
    let mut i: usize = 0;
    while i < s1_bytes.len() {
        let clen = utf_ptr2len_len(&s1_bytes[i..], (s1_len - i) as i32) as usize;
        lua_pushinteger_safe(lstate, (i as lua_Integer) + 1);
        lua_rawseti_safe(lstate, -2, idx);
        idx += 1;
        i += clen;
    }

    1
}

extern "C-unwind" fn nlua_str_utf_start(lstate: *mut lua_State) -> i32 {
    let s1_bytes = luaL_checkbytes(lstate, 1);
    let s1_len = s1_bytes.len();
    let offset = luaL_checkinteger_safe(lstate, 2);
    if offset <= 0 || (offset as usize) > s1_len {
        return luaL_error_cstr(lstate, c"index out of range");
    }
    let off = (offset as usize) - 1;
    let head_off = -i32::from(utf_cp_bounds(s1_bytes, off).begin_off);
    lua_pushinteger_safe(lstate, head_off as lua_Integer);
    1
}

extern "C-unwind" fn nlua_str_utf_end(lstate: *mut lua_State) -> i32 {
    let s1_bytes = luaL_checkbytes(lstate, 1);
    let s1_len = s1_bytes.len();
    let offset = luaL_checkinteger_safe(lstate, 2);
    if offset <= 0 || (offset as usize) > s1_len {
        return luaL_error_cstr(lstate, c"index out of range");
    }
    let off = (offset as usize) - 1;
    let tail_off = i32::from(utf_cp_bounds(s1_bytes, off).end_off) - 1;
    lua_pushinteger_safe(lstate, tail_off as lua_Integer);
    1
}

/// Convert UTF-32 or UTF-16 indices to byte index.
pub extern "C-unwind" fn nlua_str_byteindex(lstate: *mut lua_State) -> i32 {
    let s1_bytes = luaL_checkbytes(lstate, 1);
    let idx = luaL_checkinteger_safe(lstate, 2);
    if idx < 0 {
        lua_pushnil_safe(lstate);
        return 1;
    }
    let mut use_utf16 = false;
    if nlua_gettop(lstate) >= 3 {
        use_utf16 = lua_toboolean_safe(lstate, 3);
    }

    let byteidx = mb_utf_index_to_bytes(s1_bytes, idx as usize, use_utf16);
    if byteidx == -1 {
        lua_pushnil_safe(lstate);
        return 1;
    }

    lua_pushinteger_safe(lstate, byteidx as lua_Integer);
    1
}

pub extern "C-unwind" fn nlua_regex(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let text = luaL_checkstring(lstate, 1);

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let prog = vim_regcomp(text, RE_AUTO | RE_MAGIC | RE_STRICT | RE_NOBREAK);
    try_leave(&tstate, &mut err);

    if err.err_type != ErrorType::ErrorTypeNone {
        luaL_where_safe(lstate, 1);
        lua_push_bytes(lstate, b"couldn't parse regex: ");
        crate::lua::converter::lua_pushstring_opt(lstate, err.msg_cstr());
        lua_concat_safe(lstate, 3);
        api_clear_error(&mut err);
        return lua_error_safe(lstate);
    } else if prog.is_none() {
        luaL_where_safe(lstate, 1);
        lua_push_bytes(lstate, b"couldn't parse regex");
        lua_concat_safe(lstate, 2);
        return lua_error_safe(lstate);
    }

    let _ = lua_newuserdata_typed(
        lstate,
        LuaRegex {
            prog: crate::regexp::RegprogPtr::from_box(prog),
        },
    );

    lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim_regex");
    let _ = lua_setmetatable_safe(lstate, -2);
    1
}

fn nlua_get_var_scope(lstate: *mut lua_State) -> Option<crate::eval::typval::DictId> {
    let scope = luaL_checkstring(lstate, 1);
    let handle = luaL_checkinteger_safe(lstate, 2) as ObjHandle;
    let mut dict = crate::eval::typval::DictId::NULL;
    let mut err = Error::new();

    match scope.to_bytes() {
        b"g" => dict = get_globvar_dict(),
        b"v" => dict = get_vimvar_dict(),
        b"b" => {
            if let Some(buf) = find_buffer_by_handle(handle, &mut err) {
                dict = buf.b_vars();
            }
        }
        b"w" => {
            if let Some(win) = find_window_by_handle(handle, &mut err) {
                dict = win.w_vars();
            }
        }
        b"t" => {
            if let Some(tabpage) = find_tab_by_handle(handle, &mut err) {
                dict = tabpage.tp_vars();
            }
        }
        _ => {
            luaL_error_cstr(lstate, c"invalid scope");
            return None;
        }
    }

    if err.err_type != ErrorType::ErrorTypeNone {
        luaL_where_safe(lstate, 1);
        lua_push_bytes(lstate, b"scoped variable: ");
        crate::lua::converter::lua_pushstring_opt(lstate, err.msg_cstr());
        lua_concat_safe(lstate, 3);
        api_clear_error(&mut err);
        lua_error_safe(lstate);
        return None;
    }
    (!dict.is_null()).then_some(dict)
}

pub extern "C-unwind" fn nlua_setvar(lstate: *mut lua_State) -> i32 {
    let Some(dict) = nlua_get_var_scope(lstate) else {
        return 0;
    };
    let key = luaL_check_nvim_string(lstate, 3);

    let del = nlua_gettop(lstate) < 4 || lua_isnil(lstate, 4);

    let mut err = Error::new();
    let di_opt = dict_check_writable(&dict, &key, del, &mut err);
    if err.err_type != ErrorType::ErrorTypeNone {
        luaL_where_safe(lstate, 1);
        crate::lua::converter::lua_pushstring_opt(lstate, err.msg_cstr());
        lua_concat_safe(lstate, 2);
        api_clear_error(&mut err);
        lua_error_safe(lstate);
        return 0;
    }

    let watched = tv_dict_is_watched(Some(&dict));

    if del {
        let Some(di) = di_opt else {
            return 0;
        };
        if watched {
            let di_tv = di.tv().clone();
            tv_dict_watcher_notify(&dict, key.as_opt_cstr().unwrap_or(c""), None, Some(&di_tv));
        }
        tv_dict_item_remove(&dict, &di);
    } else {
        let mut tv: TypVal = TypVal::new();

        lua_pushvalue_safe(lstate, 4);
        if !nlua_pop_typval(lstate, &mut tv) {
            return luaL_error_cstr(lstate, c"Couldn't convert lua value");
        }

        let mut oldtv: TypVal = TypVal::new();

        let di = if let Some(di) = di_opt {
            let mut type_error = false;
            if dict == get_vimvar_dict()
                && !before_set_vvar(key.as_cstr(), &di, &mut tv, true, watched, &mut type_error)
            {
                tv_clear(&mut tv);
                if type_error {
                    return luaL_error_cstr_arg(
                        lstate,
                        c"Setting v:%s to value with wrong type",
                        key.as_cstr(),
                    );
                }
                return 0;
            }
            if watched {
                tv_copy(&di.tv(), &mut oldtv);
            }
            di.clear_tv();
            di
        } else {
            let new_di = tv_dict_item_alloc_len(key.as_bytes());
            let _ = tv_dict_add(&dict, &new_di);
            new_di
        };

        tv_copy(&tv, &mut di.tv_mut());

        if watched {
            tv_dict_watcher_notify(
                &dict,
                key.as_opt_cstr().unwrap_or(c""),
                Some(&tv),
                Some(&oldtv),
            );
            tv_clear(&mut oldtv);
        }

        tv_clear(&mut tv);
    }
    0
}

pub extern "C-unwind" fn nlua_getvar(lstate: *mut lua_State) -> i32 {
    let Some(dict) = nlua_get_var_scope(lstate) else {
        return 0;
    };
    let (name_bytes, name_with_nul) = luaL_checkbytes_with_nul(lstate, 3);
    let len = name_bytes.len();
    let name_cstr = crate::memory::bytes_as_cstr(name_with_nul);

    let mut di = tv_dict_find(Some(&dict), name_cstr, len as isize);
    if di.is_none() && dict == get_globvar_dict() {
        if !script_autoload(name_bytes, false) || aborting() {
            return 0;
        }
        di = tv_dict_find(Some(&dict), name_cstr, len as isize);
    }
    let Some(di) = di else {
        return 0;
    };
    let mut di_tv = di.tv().clone();
    nlua_push_typval(lstate, &mut di_tv, 0);
    1
}

extern "C-unwind" fn nlua_stricmp(lstate: *mut lua_State) -> i32 {
    let (mut slice1, mut with_nul1) = luaL_checkbytes_with_nul(lstate, 1);
    let (mut slice2, mut with_nul2) = luaL_checkbytes_with_nul(lstate, 2);
    let mut ret: i32;
    assert_eq!(with_nul1[slice1.len()], NUL as u8);
    assert_eq!(with_nul2[slice2.len()], NUL as u8);

    loop {
        let nul1 = slice1.iter().position(|&b| b == 0);
        let nul2 = slice2.iter().position(|&b| b == 0);
        ret = mb_stricmp(
            crate::memory::bytes_as_cstr(with_nul1),
            crate::memory::bytes_as_cstr(with_nul2),
        );
        if ret == 0 {
            if nul1.is_some() != nul2.is_some() {
                ret = i32::from(nul1.is_some()) - i32::from(nul2.is_some());
                break;
            }
            if let (Some(p1), Some(p2)) = (nul1, nul2) {
                slice1 = &slice1[p1 + 1..];
                slice2 = &slice2[p2 + 1..];
                with_nul1 = &with_nul1[p1 + 1..];
                with_nul2 = &with_nul2[p2 + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    lua_pop(lstate, 2);
    lua_pushnumber_safe(lstate, f64::from(i32::from(ret > 0) - i32::from(ret < 0)));
    1
}

extern "C-unwind" fn nlua_iconv(lstate: *mut lua_State) -> i32 {
    let narg = nlua_gettop(lstate);
    if narg < 3 {
        return luaL_error_cstr(lstate, c"Expected at least 3 arguments");
    }

    for i in 1..=3 {
        if lua_type_safe(lstate, i) != LUA_TSTRING {
            return luaL_argerror(lstate, i, c"expected string");
        }
    }

    let str_slice = lua_tolstring_bytes(lstate, 1);
    let mut str_len = str_slice.len();
    let from = enc_canonize(enc_skip(lua_tolstring_cstr(lstate, 2)));
    let to = enc_canonize(enc_skip(lua_tolstring_cstr(lstate, 3)));

    let mut vimconv = Vimconv {
        vc_type: CONV_NONE,
        vc_factor: 0,
        vc_fd: None,
        vc_fail: false,
    };
    let _ = convert_setup_ext(
        &mut vimconv,
        Some(from.as_c_str()),
        false,
        Some(to.as_c_str()),
        false,
    );

    let ret = string_convert(&vimconv, str_slice, Some(&mut str_len));

    match ret {
        None => lua_pushnil_safe(lstate),
        Some(bytes) => lua_push_bytes(lstate, &bytes[..str_len]),
    }

    1
}

extern "C-unwind" fn nlua_foldupdate(lstate: *mut lua_State) -> i32 {
    let window = luaL_checkinteger_safe(lstate, 1) as ObjHandle;
    let Some(win) = handle_get_window(window) else {
        return luaL_error_cstr(lstate, c"invalid window");
    };
    let top = (luaL_checkinteger_safe(lstate, 2) as Linenr) + 1;
    if top < 1 {
        return luaL_error_cstr(lstate, c"invalid top");
    }
    let bot = luaL_checkinteger_safe(lstate, 3) as Linenr;
    if top > bot {
        return luaL_error_cstr(lstate, c"invalid bot");
    }

    fold_update(&win, top, bot);
    0
}

extern "C-unwind" fn nlua_with(lstate: *mut lua_State) -> i32 {
    let mut flags: i32 = 0;
    let mut buf = crate::globals::BufPtr::NULL;
    let mut win = crate::globals::WinPtr::NULL;
    let mut keepcwd = false;
    let mut log_level: i32 = -1;

    if !lua_istable(lstate, 1) {
        return luaL_argerror(lstate, 1, c"table expected");
    }
    lua_pushnil_safe(lstate);
    while lua_next_safe(lstate, 1) != 0 {
        if lua_type_safe(lstate, -2) == LUA_TSTRING {
            let k = lua_tostring(lstate, -2).map_or(&[][..], CStr::to_bytes);
            let v = lua_toboolean_safe(lstate, -1);
            match k {
                b"buf" => {
                    buf = crate::globals::BufPtr::from(handle_get_buffer(luaL_checkinteger_safe(
                        lstate, -1,
                    )
                        as i32));
                }
                b"win" => {
                    win = crate::globals::WinPtr::from(handle_get_window(luaL_checkinteger_safe(
                        lstate, -1,
                    )
                        as i32));
                }
                b"keepcwd" => keepcwd = v,
                b"log_level" => log_level = luaL_checkinteger_safe(lstate, -1) as i32,
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

    let mut status: i32 = 0;
    let mut rets: i32 = 0;

    if (flags & CMOD_ERRSILENT) != 0 {
        flags |= CMOD_SILENT;
    }

    let save_min_log_level = min_log_level();
    if log_level >= 0 {
        set_min_log_level(log_level);
    }
    let save_cmdmod = g_st().cmdmod.take();
    let mut cur_cmod = crate::ex_docmd::Cmdmod::new();
    cur_cmod.cmod_flags = flags;
    apply_cmdmod(&mut cur_cmod);
    *g_st().cmdmod.borrow_mut() = cur_cmod;

    let mut err = Error::new();
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    {
        let mut cs: CtxSwitch = CtxSwitch::new();
        let mut switched = true;

        if !win.is_null() {
            buf = crate::globals::BufPtr::NULL;
        }

        if !win.is_null() || !buf.is_null() || keepcwd {
            let dirs = if keepcwd {
                K_CTX_KEEP_DIRS
            } else {
                K_CTX_KEEP_CWD
            };
            let tab = if let Some(win_ref) = win.opt() {
                win_find_tabpage(win_ref)
            } else {
                None
            };
            switched = ctx_switch(
                &mut cs,
                win.opt(),
                tab.as_deref(),
                buf.opt(),
                K_CTX_NO_DISPLAY | dirs | if !win.is_null() { K_CTX_VALIDATE } else { 0 },
            );
        }

        if switched {
            let s = nlua_gettop(lstate);
            lua_pushvalue_safe(lstate, 2);
            status = lua_pcall_safe(lstate, 0, LUA_MULTRET, 0);
            rets = nlua_gettop(lstate) - s;
        }

        ctx_restore(&mut cs);
    }
    try_leave(&tstate, &mut err);

    let mut cur_cmod = g_st().cmdmod.take();
    undo_cmdmod(&mut cur_cmod);
    *g_st().cmdmod.borrow_mut() = save_cmdmod;
    if log_level >= 0 {
        set_min_log_level(save_min_log_level);
    }

    if status != 0 {
        return lua_error_safe(lstate);
    } else if err.err_type != ErrorType::ErrorTypeNone {
        luaL_where_safe(lstate, 1);
        crate::lua::converter::lua_pushstring_opt(lstate, err.msg_cstr());
        lua_concat_safe(lstate, 2);
        api_clear_error(&mut err);
        return lua_error_safe(lstate);
    }

    rets
}

extern "C-unwind" fn nlua_keyparse(lstate: *mut lua_State) -> i32 {
    if !lua_isstring_safe(lstate, 1) {
        return luaL_argerror(lstate, 1, c"string expected");
    }

    let escaped = vim_strsave_escape_ks(lua_tostring(lstate, 1).map_or(&[], CStr::to_bytes));
    let mut rem = escaped.as_bytes();

    lua_newtable(lstate);

    let mut data = KeyChord::new();

    while !rem.is_empty() {
        let keys = str2special(&mut rem, true, K_TRUE, Some(&mut data));

        lua_createtable_safe(lstate, 0, 3);

        lua_newtable(lstate);
        let mut i: usize = 0;
        while MOD_MASK_TABLE[i].name != b'A' {
            if (data.mods & i32::from(MOD_MASK_TABLE[i].mod_mask))
                == i32::from(MOD_MASK_TABLE[i].mod_flag)
            {
                lua_push_bytes(lstate, std::slice::from_ref(&MOD_MASK_TABLE[i].name));
                lua_rawseti_safe(lstate, -2, (lua_objlen_safe(lstate, -2) as i32) + 1);
            }
            i += 1;
        }
        lua_setfield(lstate, -2, c"mod");

        lua_push_bytes(lstate, &data.key);
        lua_setfield(lstate, -2, c"key");

        lua_pushstring(lstate, &keys);
        lua_setfield(lstate, -2, c"keys");

        if !data.key_alt.is_empty() {
            lua_push_bytes(lstate, &data.key_alt);
            lua_setfield(lstate, -2, c"key_alt");
        }

        lua_rawseti_safe(lstate, -2, (lua_objlen_safe(lstate, -2) as i32) + 1);
    }

    1
}

extern "C-unwind" fn nlua_path_is_url(lstate: *mut lua_State) -> i32 {
    lua_pushboolean_safe(
        lstate,
        path_with_url(luaL_checkstring(lstate, 1).to_bytes()),
    );
    1
}

fn nlua_state_add_internal(lstate: *mut lua_State) {
    lua_pushcfunction_safe(lstate, nlua_getvar);
    lua_setfield(lstate, -2, c"_getvar");

    lua_pushcfunction_safe(lstate, nlua_setvar);
    lua_setfield(lstate, -2, c"_setvar");

    lua_pushcfunction_safe(lstate, nlua_foldupdate);
    lua_setfield(lstate, -2, c"_foldupdate");

    lua_pushcfunction_safe(lstate, nlua_with);
    lua_setfield(lstate, -2, c"_with_c");

    lua_getfield(lstate, -1, c"_core");
    lua_pushcfunction_safe(lstate, nlua_keyparse);
    lua_setfield(lstate, -2, c"keyparse");
    lua_pop(lstate, 1);
}

pub fn nlua_state_add_stdlib(lstate: *mut lua_State, is_thread: bool) {
    if !is_thread {
        lua_pushcfunction_safe(lstate, nlua_stricmp);
        lua_setfield(lstate, -2, c"stricmp");

        lua_pushcfunction_safe(lstate, nlua_str_utfindex);
        lua_setfield(lstate, -2, c"_str_utfindex");

        lua_pushcfunction_safe(lstate, nlua_str_byteindex);
        lua_setfield(lstate, -2, c"_str_byteindex");

        lua_pushcfunction_safe(lstate, nlua_str_utf_pos);
        lua_setfield(lstate, -2, c"str_utf_pos");

        lua_pushcfunction_safe(lstate, nlua_str_utf_start);
        lua_setfield(lstate, -2, c"str_utf_start");

        lua_pushcfunction_safe(lstate, nlua_str_utf_end);
        lua_setfield(lstate, -2, c"str_utf_end");

        lua_pushcfunction_safe(lstate, nlua_regex);
        lua_setfield(lstate, -2, c"regex");
        let _ = luaL_newmetatable(lstate, c"nvim_regex");
        lua_register_funcs(lstate, &REGEX_META);

        lua_pushvalue_safe(lstate, -1);
        lua_setfield(lstate, -2, c"__index");
        lua_pop(lstate, 1);

        let _ = luaopen_spell(lstate);
        lua_setfield(lstate, -2, c"spell");

        lua_pushcfunction_safe(lstate, nlua_iconv);
        lua_setfield(lstate, -2, c"iconv");

        let _ = luaopen_base64(lstate);
        lua_setfield(lstate, -2, c"base64");

        nlua_state_add_internal(lstate);
    }

    let _ = luaopen_mpack(lstate);
    lua_pushvalue_safe(lstate, -1);
    lua_setfield(lstate, -3, c"mpack");

    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"loaded");
    lua_pushvalue_safe(lstate, -3);
    lua_setfield(lstate, -2, c"mpack");
    lua_pop(lstate, 3);

    let _ = luaopen_lpeg(lstate);
    lua_pushvalue_safe(lstate, -1);
    lua_setfield(lstate, -4, c"lpeg");

    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"loaded");
    lua_pushvalue_safe(lstate, -3);
    lua_setfield(lstate, -2, c"lpeg");
    lua_pop(lstate, 4);

    lua_pushcfunction_safe(lstate, nlua_xdl_diff);
    lua_setfield(lstate, -2, c"diff");

    let _ = luaopen_cjson(lstate);
    lua_setfield(lstate, -2, c"json");

    lua_getfield(lstate, -1, c"_core");
    lua_pushcfunction_safe(lstate, nlua_path_is_url);
    lua_setfield(lstate, -2, c"path_is_url");
    lua_pop(lstate, 1);

    #[cfg(feature = "vendor-bit")]
    {
        let top = nlua_gettop(lstate);
        let _ = crate::bit::nvim_luaopen_bit(lstate);
        crate::lua::converter::lua_settop_safe(lstate, top);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_meta_table_has_expected_entries() {
        assert_eq!(REGEX_META.len(), 4);
        assert_eq!(REGEX_META[0].0, c"__gc");
        assert_eq!(REGEX_META[3].0, c"match_line");
    }
}
