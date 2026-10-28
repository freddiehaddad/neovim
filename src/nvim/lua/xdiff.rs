//! Lua `vim.diff` bindings (`src/nvim/lua/xdiff.c`).

use crate::api::private::helpers::{api_clear_error, api_free_string, api_set_error};
use crate::api::private::validate::{Error, ErrorType, NvimString, Object, ObjectData, ObjectType};
use crate::linematch::{fastforward_buf_to_lnum, linematch_nbuffers};
use crate::lua::converter::{
    LUA_TFUNCTION, LUA_TTABLE, lua_State, lua_concat, lua_createtable, lua_error, lua_gettop,
    lua_isnumber, lua_objlen, lua_pcall, lua_pushinteger, lua_pushlstring, lua_pushstring,
    lua_pushvalue, lua_rawseti, lua_settop, lua_tolstring, lua_tonumber, lua_type, luaL_argerror,
    luaL_error, luaL_where, nlua_pop_keydict_opt,
};
use crate::lua::executor::{api_free_luaref, nlua_pushref};
use crate::xdiff::xdiffi::xdl_diff;
use crate::xdiff::{
    Mmbuffer, Mmfile, XDF_HISTOGRAM_DIFF, XDF_IGNORE_CR_AT_EOL, XDF_INDENT_HEURISTIC,
    XDF_NEED_MINIMAL, XDF_PATIENCE_DIFF, Xdemitcb, Xdemitconf, Xpparam,
};
use std::ffi::c_char;

const COMPARED_BUFFER0: i32 = 1 << 0;
const COMPARED_BUFFER1: i32 = 1 << 1;

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
#[derive(Default)]
pub struct KeyDict_xdl_diff {
    pub is_set_xdl_diff: u64,
    pub on_hunk: i32,
    pub result_type: NvimString,
    pub algorithm: NvimString,
    pub ctxlen: i64,
    pub interhunkctxlen: i64,
    pub linematch: Object,
    pub ignore_whitespace: bool,
    pub ignore_whitespace_change: bool,
    pub ignore_whitespace_change_at_eol: bool,
    pub ignore_cr_at_eol: bool,
    pub ignore_blank_lines: bool,
    pub indent_heuristic: bool,
}

impl KeyDict_xdl_diff {
    fn init() -> Self {
        Self {
            is_set_xdl_diff: 0,
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
                obj_type: ObjectType::ObjectTypeNil,
                data: ObjectData::Integer(0),
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
        (self.is_set_xdl_diff & (1u64 << optidx)) != 0
    }
}

pub(crate) struct HunkPriv {
    lstate: *mut lua_State,
    err: Error,
    ma: *mut Mmfile,
    mb: *mut Mmfile,
    linematch: i64,
    iwhite: bool,
}

use crate::api::private::dispatch::key_dict_xdl_diff_get_field;
use crate::diff::{
    XDF_IGNORE_BLANK_LINES, XDF_IGNORE_WHITESPACE, XDF_IGNORE_WHITESPACE_AT_EOL,
    XDF_IGNORE_WHITESPACE_CHANGE,
};
use crate::lua::converter::{LUA_NOREF, LUA_TSTRING};

fn lua_pushhunk(
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
        let next_idx = (lua_objlen(lstate, -2) as i32) + 1;
        lua_rawseti(lstate, -2, next_idx);
    }
}

fn get_linematch_results(
    lstate: *mut lua_State,
    ma: *mut Mmfile,
    mb: *mut Mmfile,
    start_a: i32,
    count_a: i32,
    start_b: i32,
    count_b: i32,
    iwhite: bool,
) {
    let ma0 = fastforward_buf_to_lnum(unsafe { *ma }, start_a + 1);
    let mb0 = fastforward_buf_to_lnum(unsafe { *mb }, start_b + 1);

    let decisions = linematch_nbuffers(&[&ma0, &mb0], &[count_a, count_b], iwhite);

    let mut lnuma = start_a;
    let mut lnumb = start_b;
    let mut hunkstarta = lnuma;
    let mut hunkstartb = lnumb;
    let mut hunkcounta = 0;
    let mut hunkcountb = 0;

    for (i, &dec) in decisions.iter().enumerate() {
        if i > 0 && decisions[i - 1] != dec {
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
}

pub(crate) fn write_string(out: &mut Vec<u8>, mb: &[Mmbuffer]) -> i32 {
    for chunk in mb {
        if chunk.size > 0 && !chunk.ptr.is_null() {
            let slice = { crate::memory::cstr_slice(chunk.ptr, chunk.size as usize) };
            out.extend_from_slice(slice);
        }
    }
    0
}

pub(crate) fn hunk_locations_cb(
    start_a: i32,
    count_a: i32,
    start_b: i32,
    count_b: i32,
    cb_data: &mut HunkPriv,
) -> i32 {
    let priv_ref = { cb_data };
    let lstate = priv_ref.lstate;
    if priv_ref.linematch > 0 && i64::from(count_a) + i64::from(count_b) <= priv_ref.linematch {
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
    } else {
        lua_pushhunk(
            lstate,
            start_a as isize,
            count_a as isize,
            start_b as isize,
            count_b as isize,
        );
    }
    0
}

pub(crate) fn call_on_hunk_cb(
    mut start_a: i32,
    count_a: i32,
    mut start_b: i32,
    count_b: i32,
    cb_data: &mut HunkPriv,
) -> i32 {
    if count_a > 0 {
        start_a += 1;
    }
    if count_b > 0 {
        start_b += 1;
    }

    let priv_ref = { cb_data };
    let lstate = priv_ref.lstate;
    let err = &mut priv_ref.err;

    unsafe {
        let fidx = lua_gettop(lstate);
        lua_pushvalue(lstate, fidx);
        lua_pushinteger(lstate, start_a as isize);
        lua_pushinteger(lstate, count_a as isize);
        lua_pushinteger(lstate, start_b as isize);
        lua_pushinteger(lstate, count_b as isize);

        if lua_pcall(lstate, 4, 1, 0) != 0 {
            let msg_ptr = lua_tolstring(lstate, -1, std::ptr::null_mut());
            api_set_error!(err, ErrorType::ErrorTypeException, c"on_hunk: %s", msg_ptr,);
            return -1;
        }

        let mut r = 0;
        if lua_isnumber(lstate, -1) != 0 {
            r = lua_tonumber(lstate, -1) as i32;
        }
        lua_settop(lstate, fidx);
        r
    }
}

fn get_string_arg(lstate: *mut lua_State, idx: i32) -> Mmfile {
    if unsafe { lua_type(lstate, idx) } != LUA_TSTRING {
        luaL_argerror(lstate, idx, c"expected string");
    }
    let mut size: usize = 0;
    let ptr = unsafe { lua_tolstring(lstate, idx, &mut size) }.cast_mut();
    if size > (i32::MAX as usize) {
        luaL_argerror(lstate, idx, c"string too long");
    }
    Mmfile {
        ptr,
        size: size as i32,
    }
}

fn process_xdl_diff_opts(
    lstate: *mut lua_State,
    cfg: &mut Xdemitconf,
    params: &mut Xpparam,
    linematch: &mut i64,
    err: &mut Error,
) -> NluaXdiffMode {
    let mut opts = KeyDict_xdl_diff::init();
    let mut err_param: Option<&'static std::ffi::CStr> = None;

    {
        nlua_pop_keydict_opt(
            lstate,
            (&raw mut opts).cast(),
            Some(key_dict_xdl_diff_get_field),
            &mut err_param,
            None,
            err,
        );
        if err.err_type != ErrorType::ErrorTypeNone {
            api_free_string(opts.result_type);
            api_free_string(opts.algorithm);
            api_free_luaref(opts.on_hunk);
            return NluaXdiffMode::Unified;
        }

        let mut mode = NluaXdiffMode::Unified;
        let mut had_result_type_indices = false;

        'validate: {
            if opts.has_key(KEYSET_OPTIDX_RESULT_TYPE) {
                if crate::memory::cstr_as_opt_bytes(opts.result_type.data) == Some(b"unified") {
                    // default
                } else if crate::memory::cstr_as_opt_bytes(opts.result_type.data)
                    == Some(b"indices")
                {
                    had_result_type_indices = true;
                } else {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"not a valid result_type",
                    );
                    break 'validate;
                }
            }

            if opts.has_key(KEYSET_OPTIDX_ALGORITHM) {
                if crate::memory::cstr_as_opt_bytes(opts.algorithm.data) == Some(b"myers") {
                    // default
                } else if crate::memory::cstr_as_opt_bytes(opts.algorithm.data) == Some(b"minimal")
                {
                    params.flags |= XDF_NEED_MINIMAL;
                } else if crate::memory::cstr_as_opt_bytes(opts.algorithm.data) == Some(b"patience")
                {
                    params.flags |= XDF_PATIENCE_DIFF;
                } else if crate::memory::cstr_as_opt_bytes(opts.algorithm.data)
                    == Some(b"histogram")
                {
                    params.flags |= XDF_HISTOGRAM_DIFF;
                } else {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"not a valid algorithm",
                    );
                    break 'validate;
                }
            }

            if opts.has_key(KEYSET_OPTIDX_CTXLEN) {
                cfg.ctxlen = opts.ctxlen;
            }
            if opts.has_key(KEYSET_OPTIDX_INTERHUNKCTXLEN) {
                cfg.interhunkctxlen = opts.interhunkctxlen;
            }

            if opts.has_key(KEYSET_OPTIDX_LINEMATCH) {
                if opts.linematch.obj_type == ObjectType::ObjectTypeBoolean {
                    *linematch = if opts.linematch.data.boolean() {
                        i64::MAX
                    } else {
                        0
                    };
                } else if opts.linematch.obj_type == ObjectType::ObjectTypeInteger {
                    *linematch = opts.linematch.data.integer();
                } else {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"linematch must be a boolean or integer",
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
                if unsafe { lua_type(lstate, -1) } != LUA_TFUNCTION {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"on_hunk is not a function",
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
pub unsafe extern "C-unwind" fn nlua_xdl_diff(lstate: *mut lua_State) -> i32 {
    if unsafe { lua_gettop(lstate) } < 2 {
        return unsafe { luaL_error(lstate, c"Expected at least 2 arguments".as_ptr()) };
    }
    let mut ma = get_string_arg(lstate, 1);
    let mut mb = get_string_arg(lstate, 2);

    let mut err = Error::new();

    let mut cfg = Xdemitconf::default();
    let mut params = Xpparam::default();
    let mut linematch: i64 = 0;
    let mut mode = NluaXdiffMode::Unified;

    if unsafe { lua_gettop(lstate) } == 3 {
        if unsafe { lua_type(lstate, 3) } != LUA_TTABLE {
            return luaL_argerror(lstate, 3, c"expected table");
        }
        mode = process_xdl_diff_opts(lstate, &mut cfg, &mut params, &mut linematch, &mut err);
    }

    let mut unified_buf: Vec<u8> = Vec::new();
    let mut priv_state = HunkPriv {
        lstate,
        err,
        ma: &mut ma,
        mb: &mut mb,
        linematch,
        iwhite: (params.flags & XDF_IGNORE_WHITESPACE) != 0,
    };

    if priv_state.err.err_type == ErrorType::ErrorTypeNone {
        let mut ecb = match mode {
            NluaXdiffMode::Unified => Xdemitcb::Unified(&raw mut unified_buf),
            NluaXdiffMode::OnHunkCb => Xdemitcb::OnHunk(&raw mut priv_state),
            NluaXdiffMode::Locations => {
                unsafe { lua_createtable(lstate, 0, 0) };
                Xdemitcb::Locations(&raw mut priv_state)
            }
        };

        if xdl_diff(&mut ma, &mut mb, &params, &cfg, &mut ecb) == -1
            && priv_state.err.err_type == ErrorType::ErrorTypeNone
        {
            api_set_error!(
                &mut priv_state.err,
                ErrorType::ErrorTypeException,
                c"diff operation failed",
            );
        }
    }

    if priv_state.err.err_type != ErrorType::ErrorTypeNone {
        unsafe { luaL_where(lstate, 1) };
        lua_pushstring(lstate, priv_state.err.msg_ptr());
        api_clear_error(&mut priv_state.err);
        unsafe { lua_concat(lstate, 2) };
        return unsafe { lua_error(lstate) };
    } else if mode == NluaXdiffMode::Unified {
        unsafe {
            lua_pushlstring(
                lstate,
                unified_buf.as_ptr().cast::<c_char>(),
                unified_buf.len(),
            )
        };
        return 1;
    } else if mode == NluaXdiffMode::Locations {
        return 1;
    }
    0
}
