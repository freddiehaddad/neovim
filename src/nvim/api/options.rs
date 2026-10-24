//! Options API functions (`src/nvim/api/options.c`).

use crate::api::private::helpers::{
    OBJECT_NIL, add_c, api_free_object, api_set_error, api_typename, arena_array, arena_dict,
    arena_string, array_obj, boolean_obj, copy_object, cstr_as_string, dict_obj, error_set,
    find_buffer_by_handle, find_tab_by_handle, find_window_by_handle, has_key, put_c,
    static_cstr_as_obj, string_obj, try_wrap, with_script_context,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectData, ObjectType, api_err_conflict,
    api_err_exp,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::autocmd::{
    EVENT_FILETYPE, block_autocmds, do_filetype_autocmd, has_event, unblock_autocmds,
};
use crate::buffer::{
    BF_DUMMY, BLN_DUMMY, buf_T, buflist_new, bufref_T, bufref_valid, set_bufref, wipe_buffer,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::globals::*;
use crate::lua::converter::Boolean;
use crate::memline::ml_open;
use crate::memory::{Arena, arena_memdupz, strequal_str, xfree, xmalloc, xstrdup};
use crate::option::{
    OP_ADDING, OP_NONE, OP_PREPENDING, OP_REMOVING, OPT_GLOBAL, OPT_LOCAL, OptScope, PREFIX_NONE,
    copy_option_part, escape_option_str_cmdline, find_option, free_option_prepared,
    get_all_options_info, get_option, get_option_info, get_option_newval, get_option_value_for,
    get_option_varp_scope_from, get_varp_from, kOptBufhidden, kOptBuftype, kOptFiletype,
    kOptFlagColon, kOptFlagComma, kOptFlagFlagList, kOptInvalid, kOptModeline, kOptScopeBuf,
    kOptScopeGlobal, kOptScopeTab, kOptScopeWin, kOptSwapfile, opt_from_varp, option_has_scope,
    option_has_type, optval_free, optval_from_obj, optval_own, set_op_T, set_option_direct,
    set_option_value_for, validate_and_prepare_option_value,
};
use crate::strings::{strchr, strlen};
use crate::window::{close_windows, win_T};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::highlight::OptionalKeys;
use crate::optionstr::{CharBuf, OptIndex, OptSetPrep, optset_T};
use crate::runtime::SID_NONE;

const KEYSET_OPTIDX_option__buf: u64 = 1;
const KEYSET_OPTIDX_option__tab: u64 = 2;
const KEYSET_OPTIDX_option__win: u64 = 3;
const KEYSET_OPTIDX_option__scope: u64 = 4;
const KEYSET_OPTIDX_option__dry_run: u64 = 5;
const KEYSET_OPTIDX_option__filetype: u64 = 6;
const KEYSET_OPTIDX_option__operation: u64 = 7;

use crate::charset::NUL;
use crate::math::{FAIL, OK};
use crate::memfile::memfile_T;

#[repr(C)]
pub struct KeyDict_option {
    pub is_set__option_: OptionalKeys,
    pub scope: NvimString,
    pub win: Window,
    pub buf: Buffer,
    pub tab: Tabpage,
    pub filetype: NvimString,
    pub operation: NvimString,
    pub dry_run: Boolean,
}

unsafe fn validate_option_value_args(
    opts: *mut KeyDict_option,
    name: *mut c_char,
    allow_tab: bool,
    opt_idxp: *mut OptIndex,
    opt_flags: *mut c_int,
    scope: *mut OptScope,
    from: *mut *mut c_void,
    filetype: *mut *mut c_char,
    operation: *mut set_op_T,
    dry_run: *mut bool,
    err: *mut Error,
) -> c_int {
    unsafe {
        let mask = (*opts).is_set__option_;
        if has_key(mask, KEYSET_OPTIDX_option__filetype)
            && (has_key(mask, KEYSET_OPTIDX_option__scope)
                || has_key(mask, KEYSET_OPTIDX_option__buf)
                || has_key(mask, KEYSET_OPTIDX_option__win)
                || has_key(mask, KEYSET_OPTIDX_option__tab))
        {
            api_err_conflict(
                err,
                c"filetype".as_ptr(),
                c"'scope', 'buf', 'win' or 'tab'".as_ptr(),
            );
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab) && !allow_tab {
            api_err_conflict(err, c"tab".as_ptr(), c"this function".as_ptr());
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab)
            && (has_key(mask, KEYSET_OPTIDX_option__win)
                || has_key(mask, KEYSET_OPTIDX_option__buf)
                || has_key(mask, KEYSET_OPTIDX_option__filetype)
                || has_key(mask, KEYSET_OPTIDX_option__scope))
        {
            api_err_conflict(
                err,
                c"tab".as_ptr(),
                c"'win', 'buf', 'filetype' or 'scope'".as_ptr(),
            );
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__win) && has_key(mask, KEYSET_OPTIDX_option__buf) {
            api_err_conflict(err, c"buf".as_ptr(), c"win".as_ptr());
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__scope) {
            if strequal_str((*opts).scope.data, "local") {
                *opt_flags = OPT_LOCAL;
            } else if strequal_str((*opts).scope.data, "global") {
                *opt_flags = OPT_GLOBAL;
            } else {
                api_err_exp(
                    err,
                    c"scope".as_ptr(),
                    c"'local' or 'global'".as_ptr(),
                    ptr::null(),
                );
                return FAIL;
            }
        }

        *scope = kOptScopeGlobal;

        if !filetype.is_null() && has_key(mask, KEYSET_OPTIDX_option__filetype) {
            *filetype = (*opts).filetype.data;
        }

        if has_key(mask, KEYSET_OPTIDX_option__win) {
            *scope = kOptScopeWin;
            *from = find_window_by_handle((*opts).win, err).cast();
            if error_set(err) {
                return FAIL;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_option__buf) {
            if has_key(mask, KEYSET_OPTIDX_option__scope) && *opt_flags == OPT_GLOBAL {
                api_err_conflict(err, c"buf".as_ptr(), c"global scope".as_ptr());
                return FAIL;
            }
            *opt_flags = OPT_LOCAL;
            *scope = kOptScopeBuf;
            *from = find_buffer_by_handle((*opts).buf, err).cast();
            if error_set(err) {
                return FAIL;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab) {
            *scope = kOptScopeTab;
            *from = find_tab_by_handle((*opts).tab, err).cast();
            if error_set(err) {
                return FAIL;
            }
        }

        *opt_idxp = find_option(name);
        if *opt_idxp == kOptInvalid {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Unknown option '%s'".as_ptr(),
                name,
            );
            return FAIL;
        }

        if !operation.is_null() && has_key(mask, KEYSET_OPTIDX_option__operation) {
            if strequal_str((*opts).operation.data, "set") {
                *operation = OP_NONE;
            } else if strequal_str((*opts).operation.data, "append") {
                *operation = OP_ADDING;
            } else if strequal_str((*opts).operation.data, "prepend") {
                *operation = OP_PREPENDING;
            } else if strequal_str((*opts).operation.data, "remove") {
                *operation = OP_REMOVING;
            } else {
                api_err_exp(
                    err,
                    c"operation".as_ptr(),
                    c"'set', 'append', 'prepend', or 'remove'".as_ptr(),
                    ptr::null(),
                );
                return FAIL;
            }

            if !(*operation == OP_NONE
                || option_has_type(*opt_idxp, ObjectType::kObjectTypeString)
                || option_has_type(*opt_idxp, ObjectType::kObjectTypeInteger))
            {
                api_err_conflict(err, (*opts).operation.data, c"boolean options".as_ptr());
                return FAIL;
            }
        }

        if !dry_run.is_null() && has_key(mask, KEYSET_OPTIDX_option__dry_run) {
            *dry_run = (*opts).dry_run;
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab) && !option_has_scope(*opt_idxp, kOptScopeTab) {
            api_err_conflict(err, c"tab".as_ptr(), name);
            return FAIL;
        }

        if (*scope == kOptScopeBuf || *scope == kOptScopeWin)
            && !option_has_scope(*opt_idxp, *scope)
        {
            let tgt = if *scope == kOptScopeBuf {
                c"buf".as_ptr()
            } else {
                c"win".as_ptr()
            };
            let global = if option_has_scope(*opt_idxp, kOptScopeGlobal) {
                c"global ".as_ptr()
            } else {
                c"".as_ptr()
            };
            let req = if option_has_scope(*opt_idxp, kOptScopeBuf) {
                c"buffer-local ".as_ptr()
            } else if option_has_scope(*opt_idxp, kOptScopeWin) {
                c"window-local ".as_ptr()
            } else {
                c"".as_ptr()
            };

            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"'%s' cannot be passed for %s%soption '%s'".as_ptr(),
                tgt,
                global,
                req,
                name,
            );
            return FAIL;
        }

        if error_set(err) { FAIL } else { OK }
    }
}

unsafe fn do_ft_buf(filetype: *const c_char, aco: *mut CtxSwitch, err: *mut Error) -> *mut buf_T {
    unsafe {
        if filetype.is_null() {
            return ptr::null_mut();
        }

        let ftbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if ftbuf.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Could not create internal buffer".as_ptr(),
            );
            return ptr::null_mut();
        }

        if ml_open(ftbuf) == FAIL {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Could not load internal buffer".as_ptr(),
            );
            return ftbuf;
        }

        let mut bufref = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, ftbuf);

        ctx_switch(aco, ptr::null_mut(), ptr::null_mut(), ftbuf.cast(), 0);

        set_option_direct(
            kOptBufhidden,
            static_cstr_as_obj(c"hide"),
            OPT_LOCAL,
            SID_NONE,
        );
        set_option_direct(
            kOptBuftype,
            static_cstr_as_obj(c"nofile"),
            OPT_LOCAL,
            SID_NONE,
        );
        assert!((*(*ftbuf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fd < 0);
        *(get_option_varp_scope_from(kOptSwapfile, OPT_LOCAL, ftbuf, ptr::null_mut())
            as *mut c_int) = 0;
        *(get_option_varp_scope_from(kOptModeline, OPT_LOCAL, ftbuf, ptr::null_mut())
            as *mut c_int) = 0;
        *(get_option_varp_scope_from(kOptFiletype, OPT_LOCAL, ftbuf, ptr::null_mut())
            as *mut *mut c_char) = xstrdup(filetype);

        if !has_event(EVENT_FILETYPE) {
            return ftbuf;
        }

        let mut did_au_ft = false;
        try_wrap(err, || {
            did_au_ft = do_filetype_autocmd(ftbuf, true);
        });

        if !bufref_valid(&mut bufref) {
            if !error_set(err) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Internal buffer was deleted".as_ptr(),
                );
            }
            return ptr::null_mut();
        }

        if !did_au_ft && !error_set(err) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Could not execute FileType autocommands".as_ptr(),
            );
        }
        ftbuf
    }
}

unsafe fn wipe_ft_buf(buf: *mut buf_T) {
    unsafe {
        block_autocmds();

        let mut bufref = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, buf);

        close_windows(buf, false);
        if bufref_valid(&mut bufref) && buf != curbuf && (*buf).b_nwindows == 0 {
            wipe_buffer(buf, false);
        }
        if bufref_valid(&mut bufref) {
            (*buf).b_flags &= !BF_DUMMY;
        }

        unblock_autocmds();
    }
}

/// Gets the value of an option. The behavior of this function matches that of
/// |:set|: the local value of an option is returned if it exists; otherwise,
/// the global value is returned. Local values always correspond to the current
/// buffer or window, unless "buf" or "win" is set in {opts}.
///
/// @param name      Option name
/// @param opts      Optional parameters
///                  - buf: Buffer number. Used for getting buffer local options.
///                         Implies {scope} is "local".
///                  - filetype: |filetype|. Used to get the default option for a
///                    specific filetype. Cannot be used with any other option.
///                    Note: this will trigger |ftplugin| and all |FileType|
///                    autocommands for the corresponding filetype.
///                  - scope: One of "global" or "local". Analogous to
///                  |:setglobal| and |:setlocal|, respectively.
///                  - tab: |tab-ID| for tab-local options. Currently only
///                    supports "cmdheight". Tabpage `0` means the current tabpage.
///                  - win: |window-ID|. Used for getting window local options.
/// @param[out] err  Error details, if any
/// @return          Option value
// DLLEXPORT Object nvim_get_option_value(String name, Dict(option) *opts, Error *err)
//   FUNC_API_SINCE(9) FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_option_value(
    name: NvimString,
    opts: *mut KeyDict_option,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = kOptScopeGlobal;
        let mut from: *mut c_void = ptr::null_mut();
        let mut filetype: *mut c_char = ptr::null_mut();

        if validate_option_value_args(
            opts,
            name.data,
            true,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut from,
            &mut filetype,
            ptr::null_mut(),
            ptr::null_mut(),
            err,
        ) == 0
        {
            return OBJECT_NIL;
        }

        let mut aco: CtxSwitch = std::mem::zeroed();

        let ftbuf = do_ft_buf(filetype, &mut aco, err);
        if error_set(err) {
            ctx_restore(&mut aco);
            if !ftbuf.is_null() {
                wipe_ft_buf(ftbuf);
            }
            return OBJECT_NIL;
        }

        if !ftbuf.is_null() {
            assert!(from.is_null());
            from = ftbuf.cast();
        }

        let value = get_option_value_for(opt_idx, opt_flags, scope, from, err);

        ctx_restore(&mut aco);
        if !ftbuf.is_null() {
            wipe_ft_buf(ftbuf);
        }

        if error_set(err) {
            api_free_object(value);
            return OBJECT_NIL;
        }

        value
    }
}

/// @nodoc
// DLLEXPORT Object nvim__option_to_object(String name, Object value, Arena *arena, Error *err)
//   FUNC_API_LUA_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__option_to_object(
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let opt_idx = find_option(name.data);
        if opt_idx == kOptInvalid {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Unknown option '%s'".as_ptr(),
                name.data,
            );
            return OBJECT_NIL;
        }

        if value.obj_type != ObjectType::kObjectTypeString {
            return copy_object(value, arena);
        }

        let option = get_option(opt_idx);
        let flags = (*option).flags;
        let schema = (*option).schema;

        let str_ptr = if !value.data.string.data.is_null() {
            value.data.string.data
        } else {
            c"".as_ptr().cast_mut()
        };

        if (flags & (kOptFlagComma | kOptFlagFlagList)) == 0 {
            string_obj(arena_string(arena, cstr_as_string(str_ptr)))
        } else if (flags & kOptFlagFlagList) != 0 && (flags & kOptFlagComma) == 0 {
            let n = strlen(str_ptr);
            let mut d = arena_dict(arena, n);
            for i in 0..n {
                put_c(
                    &mut d,
                    arena_memdupz(arena, str_ptr.add(i), 1),
                    boolean_obj(true),
                );
            }
            dict_obj(d)
        } else {
            let mut nparts: usize = 1;
            let mut p = str_ptr;
            while *p != NUL {
                if *p == b',' as c_char {
                    nparts += 1;
                }
                p = p.add(1);
            }
            let as_map = (flags & kOptFlagColon) != 0
                || (flags & kOptFlagFlagList) != 0
                || !schema.is_null();
            let mut d = if as_map {
                arena_dict(arena, nparts)
            } else {
                Dict::new()
            };
            let mut a = if as_map {
                Array::new()
            } else {
                arena_array(arena, nparts)
            };
            let str_len = strlen(str_ptr);
            let item = xmalloc(str_len + 1).cast::<c_char>();
            p = str_ptr;
            while *p != NUL {
                copy_option_part(&raw mut p, item, str_len + 1, c",".as_ptr().cast_mut());
                if *item == NUL {
                    continue;
                }
                if !as_map {
                    add_c(
                        &mut a,
                        string_obj(arena_string(arena, cstr_as_string(item))),
                    );
                    continue;
                }
                let colon = strchr(item, b':' as c_int);
                let mut val: *mut c_char = ptr::null_mut();
                if !colon.is_null() {
                    *colon = NUL;
                    val = colon.add(1);
                }
                let key = arena_string(arena, cstr_as_string(item));
                let v = if (flags & kOptFlagFlagList) != 0 || val.is_null() {
                    boolean_obj(true)
                } else {
                    string_obj(arena_string(arena, cstr_as_string(val)))
                };
                put_c(&mut d, key.data, v);
            }
            xfree(item.cast());
            if as_map { dict_obj(d) } else { array_obj(a) }
        }
    }
}

/// Sets the value of an option. The behavior of this function matches that of
/// |:set|: for global-local options, both the global and local value are set
/// unless otherwise specified with {scope}.
///
/// Note that `win` and `buf` cannot be used together.
///
/// @param name      Option name
/// @param value     New option value. May be structured for dict/list options, e.g. 'formatoptions'
///                  accepts `{ 'c', 'r' }`, 'listchars' accepts `{ eol = '~' }`.
/// @param opts      Optional parameters
///                  - buf: Buffer number. Used for setting buffer local option.
///                  - dry_run: (`boolean?`, default: false) If true, validates the
///                    option value without setting it.
///                  - operation: (`string?`, default: "set") One of "set", "append", "prepend", or
///                    "remove". Corresponds to |:set=|, |:set+=|, |:set^=|, and |:set-=|.
///                    With "remove", each item in a structured `value` is removed regardless of
///                    order/adjacency.
///                  - scope: One of "global" or "local". Analogous to
///                  |:setglobal| and |:setlocal|, respectively.
///                  - tab: |tab-ID| for tab-local options (currently only 'cmdheight'). Tabpage 0
///                    means the current tabpage. If a non-current tab is given, the value will take
///                    effect when it is switched-to.
///                  - win: |window-ID|. Used for setting window local option.
/// @param[out] err  Error details, if any
/// @return          Option value
// DLLEXPORT Object nvim_set_option_value(uint64_t channel_id, String name, Object value,
//                                        Dict(option) *opts, Arena *arena, Error *err)
//   FUNC_API_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_option_value(
    channel_id: u64,
    name: NvimString,
    value: Object,
    opts: *mut KeyDict_option,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = kOptScopeGlobal;
        let mut operation: set_op_T = OP_NONE;
        let mut to: *mut c_void = ptr::null_mut();
        let mut dry_run = false;

        if validate_option_value_args(
            opts,
            name.data,
            true,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut to,
            ptr::null_mut(),
            &mut operation,
            &mut dry_run,
            err,
        ) == 0
        {
            return OBJECT_NIL;
        }

        if scope == kOptScopeWin && opt_flags == 0 && option_has_scope(opt_idx, kOptScopeGlobal) {
            opt_flags = OPT_LOCAL;
        }

        let mut merged_val = OBJECT_NIL;
        let mut errmsg: *const c_char = ptr::null();
        let option = get_option(opt_idx);
        let is_array = value.obj_type == ObjectType::kObjectTypeArray;
        let is_dict = value.obj_type == ObjectType::kObjectTypeDict;

        let buf = if scope == kOptScopeBuf {
            to as *mut buf_T
        } else {
            curbuf
        };
        let win = if scope == kOptScopeWin {
            to as *mut win_T
        } else {
            curwin
        };
        let varp = get_varp_from(option, buf, win);

        let nitems = if is_array {
            value.data.array.size
        } else if is_dict {
            value.data.dict.size
        } else {
            0
        };
        let per_item = operation == OP_REMOVING && nitems > 0;
        let count = if per_item { nitems } else { 1 };

        for i in 0..count {
            let mut item = value;
            if per_item && is_array {
                item = Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData {
                        array: Array {
                            items: value.data.array.items.add(i),
                            size: 1,
                            capacity: 0,
                        },
                    },
                };
            } else if per_item {
                item = Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: Dict {
                            items: value.data.dict.items.add(i),
                            size: 1,
                            capacity: 0,
                        },
                    },
                };
            }

            let mut error = false;
            let optval_right = optval_from_obj(opt_idx, item, operation, &mut error);

            if error {
                api_err_exp(
                    err,
                    name.data,
                    c"a valid type".as_ptr(),
                    api_typename(value.obj_type),
                );
                optval_free(merged_val);
                return OBJECT_NIL;
            }

            let mut argp: *mut c_char = ptr::null_mut();

            match optval_right.obj_type {
                ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {}
                ObjectType::kObjectTypeString => {
                    let optval_escaped = escape_option_str_cmdline(optval_right.data.string.data);
                    let escaped_bytes = CStr::from_ptr(optval_escaped).to_bytes();
                    let mut buf_bytes = Vec::with_capacity(1 + escaped_bytes.len());
                    buf_bytes.push(b'=');
                    buf_bytes.extend_from_slice(escaped_bytes);
                    argp = arena_memdupz(arena, buf_bytes.as_ptr().cast(), buf_bytes.len());
                    xfree(optval_escaped.cast());
                }
                ObjectType::kObjectTypeInteger => {
                    let s = format!("={}", optval_right.data.integer);
                    argp = arena_memdupz(arena, s.as_ptr().cast(), s.len());
                }
                ObjectType::kObjectTypeBoolean => {
                    merged_val = optval_right;
                }
                ObjectType::kObjectTypeLuaRef => {
                    merged_val = copy_object(optval_right, ptr::null_mut());
                }
                _ => {
                    std::process::abort();
                }
            }

            optval_free(optval_right);

            if optval_right.obj_type == ObjectType::kObjectTypeInteger
                || optval_right.obj_type == ObjectType::kObjectTypeString
            {
                let mut oldval = if i == 0 {
                    optval_own(opt_idx, opt_from_varp(opt_idx, varp))
                } else {
                    merged_val
                };
                merged_val = get_option_newval(
                    opt_idx,
                    opt_flags,
                    PREFIX_NONE,
                    &mut argp,
                    0,
                    operation,
                    varp,
                    &mut oldval,
                    &mut errmsg,
                );
                optval_free(oldval);
                if !errmsg.is_null() {
                    api_set_error!(err, ErrorType::kErrorTypeValidation, c"%s".as_ptr(), errmsg);
                    optval_free(merged_val);
                    return OBJECT_NIL;
                }
            }
        }

        if dry_run {
            let mut raw_buf = [0 as c_char; IOSIZE];
            let errbuf = CharBuf {
                data: raw_buf.as_mut_ptr(),
                size: IOSIZE,
            };
            let mut args = optset_T {
                os_varp: ptr::null_mut(),
                os_idx: opt_idx,
                os_flags: opt_flags,
                os_oldval: OBJECT_NIL,
                os_newval: merged_val,
                os_prep: OptSetPrep::None,
                os_prep_free: None,
                os_value_checked: false,
                os_value_changed: false,
                os_restore_chartab: false,
                os_errbuf: &errbuf,
                os_win: win,
                os_buf: buf,
            };
            errmsg = validate_and_prepare_option_value(&mut args);
            merged_val = args.os_newval;
            free_option_prepared(&mut args);
            if !errmsg.is_null() {
                api_set_error!(err, ErrorType::kErrorTypeException, c"%s".as_ptr(), errmsg);
                optval_free(merged_val);
                return OBJECT_NIL;
            }
        } else {
            with_script_context(channel_id, || {
                set_option_value_for(
                    opt_idx,
                    merged_val,
                    opt_flags,
                    scope,
                    to,
                    operation == OP_NONE,
                    err,
                );
            });
        }

        let rv = nvim__option_to_object(name, merged_val, arena, err);
        optval_free(merged_val);
        rv
    }
}

/// Gets the option information for all options.
///
/// The dict has the full option names as keys and option metadata dicts as detailed at
/// |nvim_get_option_info2()|.
///
/// @see |nvim_get_commands()|
///
/// @return dict of all options
// DLLEXPORT Dict nvim_get_all_options_info(Arena *arena, Error *err)
//   FUNC_API_SINCE(7);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_all_options_info(arena: *mut Arena, _err: *mut Error) -> Dict {
    unsafe { get_all_options_info(arena) }
}

/// Gets the option information for one option from arbitrary buffer or window
///
/// Resulting dict has keys:
/// - name: Name of the option (like 'filetype')
/// - shortname: Shortened name of the option (like 'ft')
/// - type: type of option ("string", "number" or "boolean")
/// - default: The default value for the option
/// - was_set: Whether the option was set.
///
/// - last_set_sid: Last set script id (if any)
/// - last_set_linenr: line number where option was set
/// - last_set_chan: Channel where option was set (0 for local)
///
/// - scope: one of "global", "win", "buf", or "tab"
/// - global_local: whether win or buf option has a global value
///
/// - commalist: List of comma separated values
/// - flaglist: List of single char flags
///
/// When {scope} is not provided, the last set information applies to the local
/// value in the current buffer or window if it is available, otherwise the
/// global value information is returned. This behavior can be disabled by
/// explicitly specifying {scope} in the {opts} table.
///
/// @param name      Option name
/// @param opts      Optional parameters
///                  - buf: Buffer number. Used for getting buffer local options.
///                         Implies {scope} is "local".
///                  - scope: One of "global" or "local". Analogous to
///                  |:setglobal| and |:setlocal|, respectively.
///                  - win: |window-ID|. Used for getting window local options.
/// @param[out] err Error details, if any
/// @return         Option Information
// DLLEXPORT DictAs(get_option_info) nvim_get_option_info2(String name, Dict(option) *opts,
//                                                         Arena *arena, Error *err)
//   FUNC_API_SINCE(11);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_option_info2(
    name: NvimString,
    opts: *mut KeyDict_option,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = kOptScopeGlobal;
        let mut from: *mut c_void = ptr::null_mut();

        if validate_option_value_args(
            opts,
            name.data,
            false,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut from,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            err,
        ) == 0
        {
            return Dict::new();
        }

        let buf = if scope == kOptScopeBuf {
            from as *mut buf_T
        } else {
            curbuf
        };
        let win = if scope == kOptScopeWin {
            from as *mut win_T
        } else {
            curwin
        };

        get_option_info(name, opt_flags, buf, win, arena, err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_bitmask_and_strequal_helpers() {
        let mask = (1u64 << KEYSET_OPTIDX_option__buf) | (1u64 << KEYSET_OPTIDX_option__scope);
        assert!(has_key(mask, KEYSET_OPTIDX_option__buf));
        assert!(has_key(mask, KEYSET_OPTIDX_option__scope));
        assert!(!has_key(mask, KEYSET_OPTIDX_option__win));
        unsafe {
            assert!(strequal_str(c"local".as_ptr(), "local"));
            assert!(!strequal_str(c"local".as_ptr(), "global"));
            assert!(!strequal_str(ptr::null(), "local"));
        }
    }
}
