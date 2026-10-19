//! Lua `vim.diff` bindings (`src/nvim/lua/xdiff.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::private::helpers::{api_clear_error, api_free_string};
#[allow(unused_imports)]
use crate::lua::converter::nlua_pop_keydict;
#[allow(unused_imports)]
use crate::lua::converter::{
    lua_concat, lua_createtable, lua_error, lua_gettop, lua_objlen, lua_pcall, lua_pushinteger,
    lua_pushlstring, lua_pushstring, lua_pushvalue, lua_rawseti, lua_settop, lua_tolstring,
    lua_tonumber, lua_type, luaL_argerror, luaL_error, luaL_where,
};
#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, nlua_pushref};
use std::ffi::{c_char, c_int, c_long, c_ulong, c_void};

use crate::api::private::validate::{Error, ErrorType, Object, ObjectData, ObjectType};
use crate::linematch::{fastforward_buf_to_lnum, linematch_nbuffers, mmfile_t};
use crate::lua::base64::lua_State;
use crate::map_glyph_cache::NvimString;
use crate::memory::{strequal, xfree};
use crate::xdiff::xdiffi::xdl_diff;
use crate::xdiff::{mmbuffer_t, xdemitcb_t, xdemitconf_t, xpparam_t};

const COMPARED_BUFFER0: c_int = 1 << 0;
const COMPARED_BUFFER1: c_int = 1 << 1;

const XDF_NEED_MINIMAL: c_ulong = 1 << 0;
const XDF_IGNORE_WHITESPACE: c_ulong = 1 << 1;
const XDF_IGNORE_WHITESPACE_CHANGE: c_ulong = 1 << 2;
const XDF_IGNORE_WHITESPACE_AT_EOL: c_ulong = 1 << 3;
const XDF_IGNORE_CR_AT_EOL: c_ulong = 1 << 4;
const XDF_IGNORE_BLANK_LINES: c_ulong = 1 << 7;
const XDF_PATIENCE_DIFF: c_ulong = 1 << 14;
const XDF_HISTOGRAM_DIFF: c_ulong = 1 << 15;
const XDF_INDENT_HEURISTIC: c_ulong = 1 << 23;

const LUA_TSTRING: c_int = 4;
const LUA_TTABLE: c_int = 5;
const LUA_TFUNCTION: c_int = 6;
const LUA_NOREF: c_int = -2;

const KEYSET_OPTIDX_CTXLEN: u64 = 1;
const KEYSET_OPTIDX_ON_HUNK: u64 = 2;
const KEYSET_OPTIDX_ALGORITHM: u64 = 3;
const KEYSET_OPTIDX_LINEMATCH: u64 = 4;
const KEYSET_OPTIDX_RESULT_TYPE: u64 = 5;
const KEYSET_OPTIDX_INTERHUNKCTXLEN: u64 = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum NluaXdiffMode {
    Unified,
    OnHunkCb,
    Locations,
}

#[repr(C)]
struct KeyDict_xdl_diff {
    is_set__xdl_diff_: u64,
    on_hunk: c_int,
    result_type: NvimString,
    algorithm: NvimString,
    ctxlen: i64,
    interhunkctxlen: i64,
    linematch: Object,
    ignore_whitespace: bool,
    ignore_whitespace_change: bool,
    ignore_whitespace_change_at_eol: bool,
    ignore_cr_at_eol: bool,
    ignore_blank_lines: bool,
    indent_heuristic: bool,
}

impl KeyDict_xdl_diff {
    fn init() -> Self {
        Self {
            is_set__xdl_diff_: 0,
            on_hunk: LUA_NOREF,
            result_type: NvimString {
                data: std::ptr::null_mut(),
                size: 0,
            },
            algorithm: NvimString {
                data: std::ptr::null_mut(),
                size: 0,
            },
            ctxlen: 0,
            interhunkctxlen: 0,
            linematch: Object {
                obj_type: ObjectType::kObjectTypeNil,
                data: ObjectData { integer: 0 },
            },
            ignore_whitespace: false,
            ignore_whitespace_change: false,
            ignore_whitespace_change_at_eol: false,
            ignore_cr_at_eol: false,
            ignore_blank_lines: false,
            indent_heuristic: false,
        }
    }

    #[inline]
    fn has_key(&self, optidx: u64) -> bool {
        (self.is_set__xdl_diff_ & (1u64 << optidx)) != 0
    }
}

struct HunkPriv {
    lstate: *mut lua_State,
    err: *mut Error,
    ma: *mut mmfile_t,
    mb: *mut mmfile_t,
    linematch: i64,
    iwhite: bool,
}

type FieldHashfn = unsafe extern "C" fn(str_ptr: *const c_char, len: usize) -> *const c_void;

#[cfg(not(test))]
unsafe extern "C-unwind" {
    fn lua_isnumber(lstate: *mut lua_State, idx: c_int) -> c_int;
}

#[cfg(not(test))]
unsafe extern "C" {
    fn KeyDict_xdl_diff_get_field(
        str_ptr: *const c_char,
        len: usize,
    ) -> *const crate::lua::converter::KeySetLink;
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn xdl_diff_get_field_cb(
    str_ptr: *const c_char,
    len: usize,
) -> *const crate::lua::converter::KeySetLink {
    unsafe { KeyDict_xdl_diff_get_field(str_ptr, len) }
}

#[cfg(not(test))]
unsafe fn lua_pushhunk(
    lstate: *mut lua_State,
    mut start_a: isize,
    count_a: isize,
    mut start_b: isize,
    count_b: isize,
) {
    if count_a > 0 {
        start_a += 1;
    }
    if count_b > 0 {
        start_b += 1;
    }
    unsafe {
        lua_createtable(lstate, 0, 0);
        lua_pushinteger(lstate, start_a);
        lua_rawseti(lstate, -2, 1);
        lua_pushinteger(lstate, count_a);
        lua_rawseti(lstate, -2, 2);
        lua_pushinteger(lstate, start_b);
        lua_rawseti(lstate, -2, 3);
        lua_pushinteger(lstate, count_b);
        lua_rawseti(lstate, -2, 4);
        let next_idx = (lua_objlen(lstate, -2) as c_int) + 1;
        lua_rawseti(lstate, -2, next_idx);
    }
}

#[cfg(not(test))]
unsafe fn get_linematch_results(
    lstate: *mut lua_State,
    ma: *mut mmfile_t,
    mb: *mut mmfile_t,
    start_a: c_int,
    count_a: c_int,
    start_b: c_int,
    count_b: c_int,
    iwhite: bool,
) {
    unsafe {
        let ma0 = fastforward_buf_to_lnum(*ma, start_a + 1);
        let mb0 = fastforward_buf_to_lnum(*mb, start_b + 1);

        let diff_begin: [*const mmfile_t; 2] = [&ma0, &mb0];
        let diff_length: [c_int; 2] = [count_a, count_b];

        let mut decisions: *mut c_int = std::ptr::null_mut();
        let decisions_length = linematch_nbuffers(
            diff_begin.as_ptr(),
            diff_length.as_ptr(),
            2,
            &mut decisions,
            iwhite,
        );

        let mut lnuma = start_a;
        let mut lnumb = start_b;
        let mut hunkstarta = lnuma;
        let mut hunkstartb = lnumb;
        let mut hunkcounta = 0;
        let mut hunkcountb = 0;

        for i in 0..decisions_length {
            let dec = *decisions.add(i);
            if i > 0 && *decisions.add(i - 1) != dec {
                lua_pushhunk(
                    lstate,
                    hunkstarta as isize,
                    hunkcounta as isize,
                    hunkstartb as isize,
                    hunkcountb as isize,
                );
                hunkstarta = lnuma;
                hunkstartb = lnumb;
                hunkcounta = 0;
                hunkcountb = 0;
            }
            if (dec & COMPARED_BUFFER0) != 0 {
                lnuma += 1;
                hunkcounta += 1;
            }
            if (dec & COMPARED_BUFFER1) != 0 {
                lnumb += 1;
                hunkcountb += 1;
            }
        }

        lua_pushhunk(
            lstate,
            hunkstarta as isize,
            hunkcounta as isize,
            hunkstartb as isize,
            hunkcountb as isize,
        );
        xfree(decisions.cast::<c_void>());
    }
}

#[cfg(not(test))]
unsafe extern "C" fn write_string(
    priv_data: *mut c_void,
    mb: *mut mmbuffer_t,
    nbuf: c_int,
) -> c_int {
    let out = unsafe { &mut *priv_data.cast::<Vec<u8>>() };
    for i in 0..nbuf.max(0) as usize {
        unsafe {
            let chunk = &*mb.add(i);
            if chunk.size > 0 && !chunk.ptr.is_null() {
                let slice = std::slice::from_raw_parts(chunk.ptr.cast::<u8>(), chunk.size as usize);
                out.extend_from_slice(slice);
            }
        }
    }
    0
}

#[cfg(not(test))]
unsafe extern "C" fn hunk_locations_cb(
    start_a: c_int,
    count_a: c_int,
    start_b: c_int,
    count_b: c_int,
    cb_data: *mut c_void,
) -> c_int {
    let priv_ref = unsafe { &mut *cb_data.cast::<HunkPriv>() };
    let lstate = priv_ref.lstate;
    if priv_ref.linematch > 0 && i64::from(count_a) + i64::from(count_b) <= priv_ref.linematch {
        unsafe {
            get_linematch_results(
                lstate,
                priv_ref.ma,
                priv_ref.mb,
                start_a,
                count_a,
                start_b,
                count_b,
                priv_ref.iwhite,
            );
        }
    } else {
        unsafe {
            lua_pushhunk(
                lstate,
                start_a as isize,
                count_a as isize,
                start_b as isize,
                count_b as isize,
            );
        }
    }
    0
}

#[cfg(not(test))]
unsafe extern "C" fn call_on_hunk_cb(
    mut start_a: c_int,
    count_a: c_int,
    mut start_b: c_int,
    count_b: c_int,
    cb_data: *mut c_void,
) -> c_int {
    if count_a > 0 {
        start_a += 1;
    }
    if count_b > 0 {
        start_b += 1;
    }

    let priv_ref = unsafe { &mut *cb_data.cast::<HunkPriv>() };
    let lstate = priv_ref.lstate;
    let err = priv_ref.err;

    unsafe {
        let fidx = lua_gettop(lstate);
        lua_pushvalue(lstate, fidx);
        lua_pushinteger(lstate, start_a as isize);
        lua_pushinteger(lstate, count_a as isize);
        lua_pushinteger(lstate, start_b as isize);
        lua_pushinteger(lstate, count_b as isize);

        if lua_pcall(lstate, 4, 1, 0) != 0 {
            let msg_ptr = lua_tolstring(lstate, -1, std::ptr::null_mut());
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"on_hunk: %s".as_ptr(),
                msg_ptr,
            );
            return -1;
        }

        let mut r = 0;
        if lua_isnumber(lstate, -1) != 0 {
            r = lua_tonumber(lstate, -1) as c_int;
        }
        lua_settop(lstate, fidx);
        r
    }
}

#[cfg(not(test))]
unsafe fn get_string_arg(lstate: *mut lua_State, idx: c_int) -> mmfile_t {
    unsafe {
        if lua_type(lstate, idx) != LUA_TSTRING {
            luaL_argerror(lstate, idx, c"expected string".as_ptr());
        }
        let mut size: usize = 0;
        let ptr = lua_tolstring(lstate, idx, &mut size).cast_mut();
        if size > (c_int::MAX as usize) {
            luaL_argerror(lstate, idx, c"string too long".as_ptr());
        }
        mmfile_t {
            ptr,
            size: size as c_int,
        }
    }
}

#[cfg(not(test))]
unsafe fn process_xdl_diff_opts(
    lstate: *mut lua_State,
    cfg: &mut xdemitconf_t,
    params: &mut xpparam_t,
    linematch: &mut i64,
    err: *mut Error,
) -> NluaXdiffMode {
    let mut opts = KeyDict_xdl_diff::init();
    let mut err_param: *mut c_char = std::ptr::null_mut();

    unsafe {
        nlua_pop_keydict(
            lstate,
            (&raw mut opts).cast::<c_void>(),
            Some(xdl_diff_get_field_cb),
            &mut err_param,
            std::ptr::null_mut(),
            err,
        );
        if (*err).err_type != ErrorType::kErrorTypeNone {
            api_free_string(opts.result_type);
            api_free_string(opts.algorithm);
            api_free_luaref(opts.on_hunk);
            return NluaXdiffMode::Unified;
        }

        let mut mode = NluaXdiffMode::Unified;
        let mut had_result_type_indices = false;

        'validate: {
            if opts.has_key(KEYSET_OPTIDX_RESULT_TYPE) {
                if strequal(c"unified".as_ptr(), opts.result_type.data) {
                    // default
                } else if strequal(c"indices".as_ptr(), opts.result_type.data) {
                    had_result_type_indices = true;
                } else {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"not a valid result_type".as_ptr(),
                    );
                    break 'validate;
                }
            }

            if opts.has_key(KEYSET_OPTIDX_ALGORITHM) {
                if strequal(c"myers".as_ptr(), opts.algorithm.data) {
                    // default
                } else if strequal(c"minimal".as_ptr(), opts.algorithm.data) {
                    params.flags |= XDF_NEED_MINIMAL;
                } else if strequal(c"patience".as_ptr(), opts.algorithm.data) {
                    params.flags |= XDF_PATIENCE_DIFF;
                } else if strequal(c"histogram".as_ptr(), opts.algorithm.data) {
                    params.flags |= XDF_HISTOGRAM_DIFF;
                } else {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"not a valid algorithm".as_ptr(),
                    );
                    break 'validate;
                }
            }

            if opts.has_key(KEYSET_OPTIDX_CTXLEN) {
                cfg.ctxlen = opts.ctxlen as c_long;
            }
            if opts.has_key(KEYSET_OPTIDX_INTERHUNKCTXLEN) {
                cfg.interhunkctxlen = opts.interhunkctxlen as c_long;
            }

            if opts.has_key(KEYSET_OPTIDX_LINEMATCH) {
                if opts.linematch.obj_type == ObjectType::kObjectTypeBoolean {
                    *linematch = if opts.linematch.data.boolean {
                        i64::MAX
                    } else {
                        0
                    };
                } else if opts.linematch.obj_type == ObjectType::kObjectTypeInteger {
                    *linematch = opts.linematch.data.integer;
                } else {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"linematch must be a boolean or integer".as_ptr(),
                    );
                    break 'validate;
                }
            }

            if opts.ignore_whitespace {
                params.flags |= XDF_IGNORE_WHITESPACE;
            }
            if opts.ignore_whitespace_change {
                params.flags |= XDF_IGNORE_WHITESPACE_CHANGE;
            }
            if opts.ignore_whitespace_change_at_eol {
                params.flags |= XDF_IGNORE_WHITESPACE_AT_EOL;
            }
            if opts.ignore_cr_at_eol {
                params.flags |= XDF_IGNORE_CR_AT_EOL;
            }
            if opts.ignore_blank_lines {
                params.flags |= XDF_IGNORE_BLANK_LINES;
            }
            if opts.indent_heuristic {
                params.flags |= XDF_INDENT_HEURISTIC;
            }

            if opts.has_key(KEYSET_OPTIDX_ON_HUNK) {
                mode = NluaXdiffMode::OnHunkCb;
                nlua_pushref(lstate, opts.on_hunk);
                if lua_type(lstate, -1) != LUA_TFUNCTION {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"on_hunk is not a function".as_ptr(),
                    );
                }
            } else if had_result_type_indices {
                mode = NluaXdiffMode::Locations;
            }
        }

        api_free_string(opts.result_type);
        api_free_string(opts.algorithm);
        api_free_luaref(opts.on_hunk);
        mode
    }
}

/// Lua `vim.diff(a, b, opts)` implementation (`nlua_xdl_diff`).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nlua_xdl_diff(lstate: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = lstate;
        0
    }

    #[cfg(not(test))]
    unsafe {
        if lua_gettop(lstate) < 2 {
            return luaL_error(lstate, c"Expected at least 2 arguments".as_ptr());
        }
        let mut ma = get_string_arg(lstate, 1);
        let mut mb = get_string_arg(lstate, 2);

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: std::ptr::null_mut(),
        };

        let mut cfg = xdemitconf_t {
            ctxlen: 0,
            interhunkctxlen: 0,
            flags: 0,
            find_func: None,
            find_func_priv: std::ptr::null_mut(),
            hunk_func: None,
        };
        let mut params = xpparam_t {
            flags: 0,
            anchors: std::ptr::null_mut(),
            anchors_nr: 0,
        };
        let mut ecb = xdemitcb_t {
            priv_data: std::ptr::null_mut(),
            out_hunk: None,
            out_line: None,
        };
        let mut linematch: i64 = 0;
        let mut mode = NluaXdiffMode::Unified;

        if lua_gettop(lstate) == 3 {
            if lua_type(lstate, 3) != LUA_TTABLE {
                return luaL_argerror(lstate, 3, c"expected table".as_ptr());
            }
            mode = process_xdl_diff_opts(lstate, &mut cfg, &mut params, &mut linematch, &mut err);
        }

        let mut unified_buf: Vec<u8> = Vec::new();
        let mut priv_state = HunkPriv {
            lstate,
            err: &mut err,
            ma: &mut ma,
            mb: &mut mb,
            linematch,
            iwhite: (params.flags & XDF_IGNORE_WHITESPACE) != 0,
        };

        if err.err_type == ErrorType::kErrorTypeNone {
            match mode {
                NluaXdiffMode::Unified => {
                    ecb.priv_data = (&raw mut unified_buf).cast::<c_void>();
                    ecb.out_line = Some(write_string);
                }
                NluaXdiffMode::OnHunkCb => {
                    cfg.hunk_func = Some(call_on_hunk_cb);
                    ecb.priv_data = (&raw mut priv_state).cast::<c_void>();
                }
                NluaXdiffMode::Locations => {
                    cfg.hunk_func = Some(hunk_locations_cb);
                    ecb.priv_data = (&raw mut priv_state).cast::<c_void>();
                    lua_createtable(lstate, 0, 0);
                }
            }

            if xdl_diff(&mut ma, &mut mb, &params, &cfg, &mut ecb) == -1
                && err.err_type == ErrorType::kErrorTypeNone
            {
                api_set_error!(
                    &mut err,
                    ErrorType::kErrorTypeException,
                    c"diff operation failed".as_ptr(),
                );
            }
        }

        if err.err_type != ErrorType::kErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg);
            api_clear_error(&mut err);
            lua_concat(lstate, 2);
            return lua_error(lstate);
        } else if mode == NluaXdiffMode::Unified {
            lua_pushlstring(
                lstate,
                unified_buf.as_ptr().cast::<c_char>(),
                unified_buf.len(),
            );
            return 1;
        } else if mode == NluaXdiffMode::Locations {
            return 1;
        }
        0
    }
}
