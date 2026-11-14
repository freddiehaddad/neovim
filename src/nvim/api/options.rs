//! Options API functions (`src/nvim/api/options.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    OBJECT_NIL, add_c, api_free_object, api_set_error, api_typename, array_obj, boolean_obj,
    copy_object, dict_obj, error_set, find_buffer_by_handle, find_tab_by_handle,
    find_window_by_handle, has_key, integer_obj, put_c, static_cstr_as_obj, string_obj, try_wrap,
    with_script_context,
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
    BF_DUMMY, BLN_DUMMY, Bufref, FileBuf, buflist_new, bufref_valid, set_bufref, wipe_buffer,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::eval::vars::K_OPT_FLAG_FUNC;
use crate::globals::*;
use crate::lua::converter::Boolean;
use crate::memline::ml_open;
use crate::option::{
    K_OPT_BUFHIDDEN, K_OPT_BUFTYPE, K_OPT_FLAG_COLON, K_OPT_FLAG_COMMA, K_OPT_FLAG_EXPAND,
    K_OPT_FLAG_EXPR, K_OPT_FLAG_FLAG_LIST, K_OPT_INVALID, K_OPT_KEYWORDPRG, K_OPT_SCOPE_BUF,
    K_OPT_SCOPE_GLOBAL, K_OPT_SCOPE_TAB, K_OPT_SCOPE_WIN, OP_ADDING, OP_NONE, OP_PREPENDING,
    OP_REMOVING, OPT_GLOBAL, OPT_LOCAL, OptScope, OptTarget, PREFIX_NONE, SetOp, copy_option_part,
    escape_option_str_cmdline, find_option, get_all_options_info, get_option, get_option_info,
    get_option_newval, get_option_value_for, get_varp_from, opt_from_varp, option_has_scope,
    option_has_type, optval_free, optval_from_obj, optval_own, set_option_direct,
    set_option_value_for, validate_and_prepare_option_value,
};
use crate::window::close_windows;
use std::ffi::CStr;

use crate::highlight::OptionalKeys;
use crate::optionstr::{OptIndex, OptSetPrep, OptVar, Optset};
use crate::runtime::SID_NONE;

const KEYSET_OPTIDX_OPTION_BUF: u64 = 1;
const KEYSET_OPTIDX_OPTION_TAB: u64 = 2;
const KEYSET_OPTIDX_OPTION_WIN: u64 = 3;
const KEYSET_OPTIDX_OPTION_SCOPE: u64 = 4;
const KEYSET_OPTIDX_OPTION_DRY_RUN: u64 = 5;
const KEYSET_OPTIDX_OPTION_FILETYPE: u64 = 6;
const KEYSET_OPTIDX_OPTION_OPERATION: u64 = 7;

#[derive(Default)]
pub struct KeyDictOption {
    pub is_set_option: OptionalKeys,
    pub scope: NvimString,
    pub win: Window,
    pub buf: Buffer,
    pub tab: Tabpage,
    pub filetype: NvimString,
    pub operation: NvimString,
    pub dry_run: Boolean,
}

fn validate_option_value_args<'a>(
    opts: &'a mut KeyDictOption,
    name: &CStr,
    allow_tab: bool,
    opt_idxp: &mut OptIndex,
    opt_flags: &mut i32,
    scope: &mut OptScope,
    from: &mut OptTarget,
    filetype: Option<&mut Option<&'a CStr>>,
    operation: Option<&mut SetOp>,
    dry_run: Option<&mut bool>,
    err: &mut Error,
) -> bool {
    {
        let mask = opts.is_set_option;
        if has_key(mask, KEYSET_OPTIDX_OPTION_FILETYPE)
            && (has_key(mask, KEYSET_OPTIDX_OPTION_SCOPE)
                || has_key(mask, KEYSET_OPTIDX_OPTION_BUF)
                || has_key(mask, KEYSET_OPTIDX_OPTION_WIN)
                || has_key(mask, KEYSET_OPTIDX_OPTION_TAB))
        {
            api_err_conflict(err, c"filetype", c"'scope', 'buf', 'win' or 'tab'");
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_TAB) && !allow_tab {
            api_err_conflict(err, c"tab", c"this function");
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_TAB)
            && (has_key(mask, KEYSET_OPTIDX_OPTION_WIN)
                || has_key(mask, KEYSET_OPTIDX_OPTION_BUF)
                || has_key(mask, KEYSET_OPTIDX_OPTION_FILETYPE)
                || has_key(mask, KEYSET_OPTIDX_OPTION_SCOPE))
        {
            api_err_conflict(err, c"tab", c"'win', 'buf', 'filetype' or 'scope'");
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_WIN) && has_key(mask, KEYSET_OPTIDX_OPTION_BUF) {
            api_err_conflict(err, c"buf", c"win");
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_SCOPE) {
            match opts.scope.cstr_bytes() {
                b"local" => *opt_flags = OPT_LOCAL,
                b"global" => *opt_flags = OPT_GLOBAL,
                _ => {
                    api_err_exp(err, c"scope", c"'local' or 'global'", None);
                    return false;
                }
            }
        }

        *scope = K_OPT_SCOPE_GLOBAL;

        if let Some(ft_out) = filetype
            && has_key(mask, KEYSET_OPTIDX_OPTION_FILETYPE)
        {
            *ft_out = opts.filetype.as_opt_cstr();
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_WIN) {
            *scope = K_OPT_SCOPE_WIN;
            *from = OptTarget::Win(find_window_by_handle(opts.win, err).into());
            if error_set(err) {
                return false;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_BUF) {
            if has_key(mask, KEYSET_OPTIDX_OPTION_SCOPE) && *opt_flags == OPT_GLOBAL {
                api_err_conflict(err, c"buf", c"global scope");
                return false;
            }
            *opt_flags = OPT_LOCAL;
            *scope = K_OPT_SCOPE_BUF;
            *from = OptTarget::Buf(find_buffer_by_handle(opts.buf, err).into());
            if error_set(err) {
                return false;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_TAB) {
            *scope = K_OPT_SCOPE_TAB;
            *from = OptTarget::Tab(find_tab_by_handle(opts.tab, err).into());
            if error_set(err) {
                return false;
            }
        }

        *opt_idxp = find_option(name.to_bytes());
        if *opt_idxp == K_OPT_INVALID {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Unknown option '%s'",
                name,
            );
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_OPERATION)
            && let Some(operation) = operation
        {
            match opts.operation.cstr_bytes() {
                b"set" => *operation = OP_NONE,
                b"append" => *operation = OP_ADDING,
                b"prepend" => *operation = OP_PREPENDING,
                b"remove" => *operation = OP_REMOVING,
                _ => {
                    api_err_exp(
                        err,
                        c"operation",
                        c"'set', 'append', 'prepend', or 'remove'",
                        None,
                    );
                    return false;
                }
            }

            if !(*operation == OP_NONE
                || option_has_type(*opt_idxp, ObjectType::ObjectTypeString)
                || option_has_type(*opt_idxp, ObjectType::ObjectTypeInteger))
            {
                api_err_conflict(err, opts.operation.as_cstr(), c"boolean options");
                return false;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_DRY_RUN)
            && let Some(dry_run) = dry_run
        {
            *dry_run = opts.dry_run;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_TAB) && !option_has_scope(*opt_idxp, K_OPT_SCOPE_TAB)
        {
            api_err_conflict(err, c"tab", name);
            return false;
        }

        if (*scope == K_OPT_SCOPE_BUF || *scope == K_OPT_SCOPE_WIN)
            && !option_has_scope(*opt_idxp, *scope)
        {
            let tgt = if *scope == K_OPT_SCOPE_BUF {
                c"buf"
            } else {
                c"win"
            };
            let global = if option_has_scope(*opt_idxp, K_OPT_SCOPE_GLOBAL) {
                c"global "
            } else {
                c""
            };
            let req = if option_has_scope(*opt_idxp, K_OPT_SCOPE_BUF) {
                c"buffer-local "
            } else if option_has_scope(*opt_idxp, K_OPT_SCOPE_WIN) {
                c"window-local "
            } else {
                c""
            };

            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"'%s' cannot be passed for %s%soption '%s'",
                tgt,
                global,
                req,
                name,
            );
            return false;
        }

        !error_set(err)
    }
}

fn do_ft_buf(
    filetype: Option<&CStr>,
    aco: &mut CtxSwitch,
    err: &mut Error,
) -> Option<crate::globals::BufPtr> {
    let filetype = filetype?;

    let Some(ftbuf) = buflist_new(None, None, 1, BLN_DUMMY) else {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Could not create internal buffer",
        );
        return None;
    };

    if !ml_open(&ftbuf) {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Could not load internal buffer",
        );
        return Some(ftbuf);
    }

    let mut bufref = Bufref {
        br_buf: crate::globals::BufPtr::NULL,
        br_fnum: 0,
        br_buf_free_count: 0,
    };
    set_bufref(&mut bufref, ftbuf.opt());

    ctx_switch(&mut *aco, None, None, ftbuf.opt(), 0);

    set_option_direct(
        K_OPT_BUFHIDDEN,
        static_cstr_as_obj(c"hide"),
        OPT_LOCAL,
        SID_NONE,
    );
    set_option_direct(
        K_OPT_BUFTYPE,
        static_cstr_as_obj(c"nofile"),
        OPT_LOCAL,
        SID_NONE,
    );

    assert!(ftbuf.b_ml.ml_mfp.borrow().as_ref().unwrap().mf_fd < 0);

    ftbuf.b_p_swf.set(false);
    ftbuf.b_p_ml.set(false);
    ftbuf.b_p_ft.set_option_cstr(filetype);

    if !has_event(EVENT_FILETYPE) {
        return (!ftbuf.is_null()).then_some(ftbuf);
    }

    let mut did_au_ft = false;
    try_wrap(err, || {
        did_au_ft = do_filetype_autocmd(ftbuf.borrow(), true);
    });

    if !bufref_valid(Some(&bufref)) {
        if !error_set(err) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Internal buffer was deleted",
            );
        }
        return None;
    }

    if !did_au_ft && !error_set(err) {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Could not execute FileType autocommands",
        );
    }
    (!ftbuf.is_null()).then_some(ftbuf)
}

fn wipe_ft_buf(buf: &FileBuf) {
    let buf_ptr = crate::globals::BufPtr::from(buf);
    block_autocmds();

    let mut bufref = Bufref {
        br_buf: crate::globals::BufPtr::NULL,
        br_fnum: 0,
        br_buf_free_count: 0,
    };
    set_bufref(&mut bufref, buf_ptr.opt());

    close_windows(buf_ptr.opt(), false);
    if bufref_valid(Some(&bufref))
        && g_st().curbuf.get() != buf_ptr
        && buf_ptr.b_nwindows.get() == 0
    {
        wipe_buffer(buf_ptr.borrow(), false);
    }
    if bufref_valid(Some(&bufref)) {
        buf_ptr.b_flags.set(buf_ptr.b_flags.get() & (!BF_DUMMY));
    }

    unblock_autocmds();
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
pub fn nvim_get_option_value(
    name: NvimString,
    opts: &mut KeyDictOption,
    err: &mut Error,
) -> Object {
    let mut opt_idx: OptIndex = 0;
    let mut opt_flags: i32 = 0;
    let mut scope: OptScope = K_OPT_SCOPE_GLOBAL;
    let mut from = OptTarget::Global;
    let mut filetype: Option<&CStr> = None;

    if !validate_option_value_args(
        opts,
        name.as_cstr(),
        true,
        &mut opt_idx,
        &mut opt_flags,
        &mut scope,
        &mut from,
        Some(&mut filetype),
        None,
        None,
        err,
    ) {
        return OBJECT_NIL;
    }

    let mut aco: CtxSwitch = CtxSwitch::new();

    let ftbuf = crate::globals::BufPtr::from(do_ft_buf(filetype, &mut aco, err));
    if error_set(err) {
        ctx_restore(&mut aco);
        if !ftbuf.is_null() {
            wipe_ft_buf(ftbuf.borrow());
        }
        return OBJECT_NIL;
    }

    if !ftbuf.is_null() {
        assert!(matches!(from, OptTarget::Global));
        from = OptTarget::Buf(ftbuf.clone());
    }

    let value = get_option_value_for(opt_idx, opt_flags, scope, from, err);

    ctx_restore(&mut aco);
    if !ftbuf.is_null() {
        wipe_ft_buf(ftbuf.borrow());
    }

    if error_set(err) {
        api_free_object(value);
        return OBJECT_NIL;
    }

    value
}

fn option_to_object_idx(opt_idx: OptIndex, value: Object) -> Object {
    if value.obj_type != ObjectType::ObjectTypeString {
        return copy_object(value);
    }

    let option = get_option(opt_idx);
    let flags = option.flags();
    let schema = option.schema;
    let bytes = value.data.cstr_bytes();

    if (flags & (K_OPT_FLAG_COMMA | K_OPT_FLAG_FLAG_LIST)) == 0 {
        string_obj(crate::api::private::helpers::ensure_nul_string(
            value.data.string(),
        ))
    } else if (flags & K_OPT_FLAG_FLAG_LIST) != 0 && (flags & K_OPT_FLAG_COMMA) == 0 {
        let mut d = Dict::with_capacity(bytes.len());
        for &b in bytes {
            put_c(&mut d, NvimString::from_bytes(&[b]), boolean_obj(true));
        }
        dict_obj(d)
    } else {
        let mut nparts: usize = 1;
        for &p_byte in bytes {
            if p_byte == b',' {
                nparts += 1;
            }
        }
        let as_map = (flags & K_OPT_FLAG_COLON) != 0
            || (flags & K_OPT_FLAG_FLAG_LIST) != 0
            || !schema.is_empty();
        let mut d = if as_map {
            Dict::with_capacity(nparts)
        } else {
            Dict::new()
        };
        let mut a = if as_map {
            Array::new()
        } else {
            Array::with_capacity(nparts)
        };
        let mut p_bytes = bytes;
        let str_len = p_bytes.len();
        let mut stack_buf = [0u8; 256];
        let mut heap_buf: Vec<u8> = Vec::new();
        let item_buf: &mut [u8] = if str_len < stack_buf.len() {
            &mut stack_buf[..str_len + 1]
        } else {
            heap_buf.resize(str_len + 1, 0);
            &mut heap_buf
        };
        while !p_bytes.is_empty() {
            let item_len = copy_option_part(&mut p_bytes, item_buf, c",");
            let item_bytes = &item_buf[..item_len];
            if item_bytes.is_empty() {
                continue;
            }
            if !as_map {
                add_c(&mut a, string_obj(NvimString::from_bytes(item_bytes)));
                continue;
            }
            let (key_bytes, val_bytes) = match item_bytes.iter().position(|&b| b == b':') {
                Some(idx) => (&item_bytes[..idx], Some(&item_bytes[idx + 1..])),
                None => (item_bytes, None),
            };
            let key = NvimString::from_bytes(key_bytes);
            let v = match val_bytes {
                Some(vb) if (flags & K_OPT_FLAG_FLAG_LIST) == 0 => {
                    string_obj(NvimString::from_bytes(vb))
                }
                _ => boolean_obj(true),
            };
            put_c(&mut d, key, v);
        }
        if as_map { dict_obj(d) } else { array_obj(a) }
    }
}

/// @nodoc
// DLLEXPORT Object nvim__option_to_object(String name, Object value, Arena *arena, Error *err)
//   FUNC_API_LUA_ONLY;
pub fn nvim_option_to_object(name: NvimString, value: Object, err: &mut Error) -> Object {
    let opt_idx = find_option(name.as_bytes());
    if opt_idx == K_OPT_INVALID {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Unknown option '%s'",
            name,
        );
        return OBJECT_NIL;
    }

    option_to_object_idx(opt_idx, value)
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
pub fn nvim_set_option_value(
    channel_id: u64,
    name: NvimString,
    value: Object,
    opts: &mut KeyDictOption,
    err: &mut Error,
) -> Object {
    let mut opt_idx: OptIndex = 0;
    let mut opt_flags: i32 = 0;
    let mut scope: OptScope = K_OPT_SCOPE_GLOBAL;
    let mut operation: SetOp = OP_NONE;
    let mut to = OptTarget::Global;
    let mut dry_run = false;

    if !validate_option_value_args(
        opts,
        name.as_cstr(),
        true,
        &mut opt_idx,
        &mut opt_flags,
        &mut scope,
        &mut to,
        None,
        Some(&mut operation),
        Some(&mut dry_run),
        err,
    ) {
        return OBJECT_NIL;
    }

    if scope == K_OPT_SCOPE_WIN && opt_flags == 0 && option_has_scope(opt_idx, K_OPT_SCOPE_GLOBAL) {
        opt_flags = OPT_LOCAL;
    }

    let mut merged_val = OBJECT_NIL;
    let mut errmsg = None;
    let option = get_option(opt_idx);
    let is_array = value.obj_type == ObjectType::ObjectTypeArray;
    let is_dict = value.obj_type == ObjectType::ObjectTypeDict;

    let buf = if let (K_OPT_SCOPE_BUF, OptTarget::Buf(b)) = (scope, &to) {
        b
    } else {
        &g_st().curbuf.get()
    };
    let win = if let (K_OPT_SCOPE_WIN, OptTarget::Win(w)) = (scope, &to) {
        w
    } else {
        &crate::globals::curwin()
    };
    let varp = get_varp_from(option, buf.borrow(), win.opt());

    let nitems = if is_array {
        value.data.array().size
    } else if is_dict {
        value.data.dict().size
    } else {
        0
    };
    let per_item = operation == OP_REMOVING && nitems > 0;
    let count = if per_item { nitems } else { 1 };

    for i in 0..count {
        let mut single_arr_item;
        let mut single_dict_item;
        let mut item = value.clone();
        if per_item && is_array {
            single_arr_item = [value.data.array()[i].clone()];
            item = Object {
                obj_type: ObjectType::ObjectTypeArray,
                data: ObjectData::Array(Array::from_slice(&mut single_arr_item)),
            };
        } else if per_item {
            single_dict_item = [value.data.dict()[i].clone()];
            item = Object {
                obj_type: ObjectType::ObjectTypeDict,
                data: ObjectData::Dict(Dict::from_slice(&mut single_dict_item)),
            };
        }

        let mut error = false;
        let optval_right = optval_from_obj(opt_idx, item, operation, &mut error);
        let right_type = optval_right.obj_type;

        if error {
            api_err_exp(
                err,
                name.as_cstr(),
                c"a valid type",
                Some(api_typename(value.obj_type)),
            );
            optval_free(merged_val);
            return OBJECT_NIL;
        }

        if right_type == ObjectType::ObjectTypeInteger {
            if operation == OP_NONE {
                merged_val = optval_right;
            } else {
                let oldval_num = if i == 0 {
                    varp.int_val()
                } else {
                    merged_val.data.integer()
                };
                let rhs = optval_right.data.integer();
                let res = match operation {
                    OP_ADDING => oldval_num.wrapping_add(rhs),
                    OP_PREPENDING => oldval_num.wrapping_mul(rhs),
                    OP_REMOVING => oldval_num.wrapping_sub(rhs),
                    _ => rhs,
                };
                merged_val = integer_obj(res);
            }
            continue;
        }

        if right_type == ObjectType::ObjectTypeString
            && operation == OP_NONE
            && option_has_type(opt_idx, ObjectType::ObjectTypeString)
            && (option.flags() & (K_OPT_FLAG_FUNC | K_OPT_FLAG_EXPR | K_OPT_FLAG_FLAG_LIST)) == 0
            && opt_idx != K_OPT_KEYWORDPRG
            && {
                let s = optval_right.data.cstr_bytes();
                optval_right.data.string().size == s.len()
                    && ((option.flags() & K_OPT_FLAG_EXPAND) == 0
                        || (!s
                            .first()
                            .is_some_and(|&b| crate::charset::ascii_iswhite(i32::from(b)))
                            && !s.iter().any(|&b| b == b'$' || b == b'~' || b == b'\\')))
            }
        {
            merged_val = optval_right;
            continue;
        }

        let mut arg_buf: Vec<u8> = Vec::new();

        match right_type {
            ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {}
            ObjectType::ObjectTypeString => {
                let optval_escaped = escape_option_str_cmdline(optval_right.data.as_bytes());
                let escaped_bytes = optval_escaped.as_bytes();
                arg_buf.reserve(1 + escaped_bytes.len());
                arg_buf.push(b'=');
                arg_buf.extend_from_slice(escaped_bytes);
            }
            ObjectType::ObjectTypeBoolean => {
                merged_val = optval_right.clone();
            }
            ObjectType::ObjectTypeLuaRef => {
                merged_val = copy_object(optval_right.clone());
            }
            _ => {
                std::process::abort();
            }
        }

        optval_free(optval_right);

        if right_type == ObjectType::ObjectTypeString {
            let mut oldval = if i == 0 {
                optval_own(opt_idx, opt_from_varp(opt_idx, varp.clone()))
            } else {
                merged_val
            };
            let mut argp: &[u8] = &arg_buf;
            merged_val = get_option_newval(
                opt_idx,
                opt_flags,
                PREFIX_NONE,
                &mut argp,
                0,
                operation,
                varp.clone(),
                Some(&mut oldval),
                &mut errmsg,
            );
            optval_free(oldval);
            if let Some(errmsg) = errmsg {
                api_set_error!(err, ErrorType::ErrorTypeValidation, c"%s", errmsg);
                optval_free(merged_val);
                return OBJECT_NIL;
            }
        }
    }

    if dry_run {
        let mut raw_buf = [0u8; IOSIZE];
        let mut args = Optset {
            os_varp: OptVar::None,
            os_idx: opt_idx,
            os_flags: opt_flags,
            os_oldval: OBJECT_NIL,
            os_newval: merged_val,
            os_prep: OptSetPrep::None,
            os_value_checked: false,
            os_value_changed: false,
            os_errbuf: Some(&mut raw_buf),
            os_win: win.clone(),
            os_buf: buf.clone(),
        };
        let errmsg = validate_and_prepare_option_value(&mut args);
        merged_val = args.os_newval;
        if let Some(errmsg) = errmsg {
            api_set_error!(err, ErrorType::ErrorTypeException, c"%s", errmsg);
            optval_free(merged_val);
            return OBJECT_NIL;
        }
    } else {
        with_script_context(channel_id, || {
            set_option_value_for(
                opt_idx,
                merged_val.clone(),
                opt_flags,
                scope,
                to,
                operation == OP_NONE,
                err,
            );
        });
    }

    let rv = option_to_object_idx(opt_idx, merged_val.clone());
    optval_free(merged_val);
    rv
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
pub fn nvim_get_all_options_info(_err: &mut Error) -> Dict {
    get_all_options_info()
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
pub fn nvim_get_option_info2(name: NvimString, opts: &mut KeyDictOption, err: &mut Error) -> Dict {
    let mut opt_idx: OptIndex = 0;
    let mut opt_flags: i32 = 0;
    let mut scope: OptScope = K_OPT_SCOPE_GLOBAL;
    let mut from = OptTarget::Global;

    if !validate_option_value_args(
        opts,
        name.as_cstr(),
        false,
        &mut opt_idx,
        &mut opt_flags,
        &mut scope,
        &mut from,
        None,
        None,
        None,
        err,
    ) {
        return Dict::new();
    }

    let buf = if let (K_OPT_SCOPE_BUF, OptTarget::Buf(b)) = (scope, &from) {
        b
    } else {
        &g_st().curbuf.get()
    };
    let win = if let (K_OPT_SCOPE_WIN, OptTarget::Win(w)) = (scope, &from) {
        w
    } else {
        &crate::globals::curwin()
    };

    get_option_info(name, opt_flags, buf.borrow(), win.borrow(), err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_bitmask_and_strequal_helpers() {
        let mask = (1u64 << KEYSET_OPTIDX_OPTION_BUF) | (1u64 << KEYSET_OPTIDX_OPTION_SCOPE);
        assert!(has_key(mask, KEYSET_OPTIDX_OPTION_BUF));
        assert!(has_key(mask, KEYSET_OPTIDX_OPTION_SCOPE));
        assert!(!has_key(mask, KEYSET_OPTIDX_OPTION_WIN));
    }
}
