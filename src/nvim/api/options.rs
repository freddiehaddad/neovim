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
    BF_DUMMY, BLN_DUMMY, Bufref, FileBuf, buflist_new, bufref_valid, set_bufref, wipe_buffer,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::globals::*;
use crate::lua::converter::Boolean;
use crate::memline::ml_open;
use crate::memory::{Arena, arena_memdupz};
use crate::option::{
    K_OPT_BUFHIDDEN, K_OPT_BUFTYPE, K_OPT_FILETYPE, K_OPT_FLAG_COLON, K_OPT_FLAG_COMMA,
    K_OPT_FLAG_FLAG_LIST, K_OPT_INVALID, K_OPT_MODELINE, K_OPT_SCOPE_BUF, K_OPT_SCOPE_GLOBAL,
    K_OPT_SCOPE_TAB, K_OPT_SCOPE_WIN, K_OPT_SWAPFILE, OP_ADDING, OP_NONE, OP_PREPENDING,
    OP_REMOVING, OPT_GLOBAL, OPT_LOCAL, OptScope, OptTarget, PREFIX_NONE, SetOp, copy_option_part,
    escape_option_str_cmdline, find_option, get_all_options_info, get_option, get_option_info,
    get_option_newval, get_option_value_for, get_option_varp_scope_from, get_varp_from,
    opt_from_varp, option_has_scope, option_has_type, optval_free, optval_from_obj, optval_own,
    set_option_direct, set_option_value_for, validate_and_prepare_option_value,
};
use crate::window::close_windows;
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::highlight::OptionalKeys;
use crate::optionstr::{CharBuf, OptIndex, OptSetPrep, Optset};
use crate::runtime::SID_NONE;

const KEYSET_OPTIDX_OPTION_BUF: u64 = 1;
const KEYSET_OPTIDX_OPTION_TAB: u64 = 2;
const KEYSET_OPTIDX_OPTION_WIN: u64 = 3;
const KEYSET_OPTIDX_OPTION_SCOPE: u64 = 4;
const KEYSET_OPTIDX_OPTION_DRY_RUN: u64 = 5;
const KEYSET_OPTIDX_OPTION_FILETYPE: u64 = 6;
const KEYSET_OPTIDX_OPTION_OPERATION: u64 = 7;

use crate::charset::NUL;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_option {
    pub is_set_option: OptionalKeys,
    pub scope: NvimString,
    pub win: Window,
    pub buf: Buffer,
    pub tab: Tabpage,
    pub filetype: NvimString,
    pub operation: NvimString,
    pub dry_run: Boolean,
}

fn validate_option_value_args(
    opts: &mut KeyDict_option,
    name: impl crate::strings::IntoCCharPtr,
    allow_tab: bool,
    opt_idxp: &mut OptIndex,
    opt_flags: &mut c_int,
    scope: &mut OptScope,
    from: &mut OptTarget,
    filetype: *mut *mut c_char,
    operation: Option<&mut SetOp>,
    dry_run: Option<&mut bool>,
    err: &mut Error,
) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
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
            if crate::memory::cstr_as_opt_bytes(opts.scope.data) == Some(b"local") {
                *opt_flags = OPT_LOCAL;
            } else if crate::memory::cstr_as_opt_bytes(opts.scope.data) == Some(b"global") {
                *opt_flags = OPT_GLOBAL;
            } else {
                api_err_exp(err, c"scope", c"'local' or 'global'", std::ptr::null());
                return false;
            }
        }

        *scope = K_OPT_SCOPE_GLOBAL;

        if !filetype.is_null() && has_key(mask, KEYSET_OPTIDX_OPTION_FILETYPE) {
            *filetype = opts.filetype.data;
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_WIN) {
            *scope = K_OPT_SCOPE_WIN;
            *from = OptTarget::Win(find_window_by_handle(opts.win, err));
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
            *from = OptTarget::Buf(find_buffer_by_handle(opts.buf, err));
            if error_set(err) {
                return false;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_OPTION_TAB) {
            *scope = K_OPT_SCOPE_TAB;
            *from = OptTarget::Tab(find_tab_by_handle(opts.tab, err));
            if error_set(err) {
                return false;
            }
        }

        *opt_idxp = find_option(name);
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
            if crate::memory::cstr_as_opt_bytes(opts.operation.data) == Some(b"set") {
                *operation = OP_NONE;
            } else if crate::memory::cstr_as_opt_bytes(opts.operation.data) == Some(b"append") {
                *operation = OP_ADDING;
            } else if crate::memory::cstr_as_opt_bytes(opts.operation.data) == Some(b"prepend") {
                *operation = OP_PREPENDING;
            } else if crate::memory::cstr_as_opt_bytes(opts.operation.data) == Some(b"remove") {
                *operation = OP_REMOVING;
            } else {
                api_err_exp(
                    err,
                    c"operation",
                    c"'set', 'append', 'prepend', or 'remove'",
                    std::ptr::null(),
                );
                return false;
            }

            if !(*operation == OP_NONE
                || option_has_type(*opt_idxp, ObjectType::ObjectTypeString)
                || option_has_type(*opt_idxp, ObjectType::ObjectTypeInteger))
            {
                api_err_conflict(err, opts.operation.data, c"boolean options");
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
                c"buf".as_ptr()
            } else {
                c"win".as_ptr()
            };
            let global = if option_has_scope(*opt_idxp, K_OPT_SCOPE_GLOBAL) {
                c"global ".as_ptr()
            } else {
                c"".as_ptr()
            };
            let req = if option_has_scope(*opt_idxp, K_OPT_SCOPE_BUF) {
                c"buffer-local ".as_ptr()
            } else if option_has_scope(*opt_idxp, K_OPT_SCOPE_WIN) {
                c"window-local ".as_ptr()
            } else {
                c"".as_ptr()
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
    filetype: impl crate::strings::IntoCCharPtr,
    aco: &mut CtxSwitch,
    err: &mut Error,
) -> *mut FileBuf {
    let filetype = filetype.into_c_char_ptr();
    unsafe {
        if filetype.is_null() {
            return ptr::null_mut();
        }

        let ftbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if ftbuf.is_null() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Could not create internal buffer",
            );
            return ptr::null_mut();
        }

        if !ml_open(&mut *ftbuf) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Could not load internal buffer",
            );
            return ftbuf;
        }

        let mut bufref = Bufref {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, ftbuf);

        ctx_switch(&mut *aco, ptr::null_mut(), ptr::null_mut(), ftbuf.cast(), 0);

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
        assert!((*(*ftbuf).b_ml.mfp_ptr()).mf_fd < 0);
        *(get_option_varp_scope_from(K_OPT_SWAPFILE, OPT_LOCAL, &mut *ftbuf, ptr::null_mut())
            as *mut c_int) = 0;
        *(get_option_varp_scope_from(K_OPT_MODELINE, OPT_LOCAL, &mut *ftbuf, ptr::null_mut())
            as *mut c_int) = 0;
        *(get_option_varp_scope_from(K_OPT_FILETYPE, OPT_LOCAL, &mut *ftbuf, ptr::null_mut())
            as *mut *mut c_char) = crate::optionstr::alloc_string_option(filetype);

        if !has_event(EVENT_FILETYPE) {
            return ftbuf;
        }

        let mut did_au_ft = false;
        try_wrap(err, || {
            did_au_ft = do_filetype_autocmd(&mut *ftbuf, true);
        });

        if !bufref_valid(Some(&bufref)) {
            if !error_set(err) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Internal buffer was deleted",
                );
            }
            return ptr::null_mut();
        }

        if !did_au_ft && !error_set(err) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Could not execute FileType autocommands",
            );
        }
        ftbuf
    }
}

fn wipe_ft_buf(buf: &mut FileBuf) {
    unsafe {
        block_autocmds();

        let mut bufref = Bufref {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, &raw mut *buf);

        close_windows(&raw mut *buf, false);
        if bufref_valid(Some(&bufref)) && !ptr::eq(buf, g_st().curbuf) && buf.b_nwindows == 0 {
            wipe_buffer(&raw mut *buf, false);
        }
        if bufref_valid(Some(&bufref)) {
            buf.b_flags &= !BF_DUMMY;
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
pub fn nvim_get_option_value(
    name: NvimString,
    opts: &mut KeyDict_option,
    err: &mut Error,
) -> Object {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = K_OPT_SCOPE_GLOBAL;
        let mut from = OptTarget::Global;
        let mut filetype: *mut c_char = ptr::null_mut();

        if !validate_option_value_args(
            opts,
            name.data,
            true,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut from,
            &mut filetype,
            None,
            None,
            err,
        ) {
            return OBJECT_NIL;
        }

        let mut aco: CtxSwitch = CtxSwitch::new();

        let ftbuf = do_ft_buf(filetype, &mut aco, err);
        if error_set(err) {
            ctx_restore(&mut aco);
            if !ftbuf.is_null() {
                wipe_ft_buf(&mut *ftbuf);
            }
            return OBJECT_NIL;
        }

        if !ftbuf.is_null() {
            assert!(matches!(from, OptTarget::Global));
            from = OptTarget::Buf(ftbuf);
        }

        let value = get_option_value_for(opt_idx, opt_flags, scope, from, err);

        ctx_restore(&mut aco);
        if !ftbuf.is_null() {
            wipe_ft_buf(&mut *ftbuf);
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
pub fn nvim_option_to_object(
    name: NvimString,
    value: Object,
    arena: &mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let opt_idx = find_option(name.data);
        if opt_idx == K_OPT_INVALID {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Unknown option '%s'",
                name.data,
            );
            return OBJECT_NIL;
        }

        if value.obj_type != ObjectType::ObjectTypeString {
            return copy_object(value, Some(arena));
        }

        let option = get_option(opt_idx);
        let flags = option.flags;
        let schema = option.schema;

        let str_ptr = if !value.data.string().data.is_null() {
            value.data.string().data
        } else {
            c"".as_ptr().cast_mut()
        };

        if (flags & (K_OPT_FLAG_COMMA | K_OPT_FLAG_FLAG_LIST)) == 0 {
            string_obj(arena_string(arena, cstr_as_string(str_ptr)))
        } else if (flags & K_OPT_FLAG_FLAG_LIST) != 0 && (flags & K_OPT_FLAG_COMMA) == 0 {
            let n = crate::memory::cstr_len(str_ptr);
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
            for &p_byte in crate::memory::cstr_as_bytes(p) {
                if p_byte == b',' {
                    nparts += 1;
                }
            }
            let as_map = (flags & K_OPT_FLAG_COLON) != 0
                || (flags & K_OPT_FLAG_FLAG_LIST) != 0
                || !schema.is_empty();
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
            let str_len = crate::memory::cstr_len(str_ptr);
            let mut item_buf = vec![0 as c_char; str_len + 1];
            let item = item_buf.as_mut_ptr();
            p = str_ptr;
            while !crate::memory::cstr_is_empty(p) {
                copy_option_part(&raw mut p, item, str_len + 1, c",");
                if crate::memory::cstr_is_empty(item) {
                    continue;
                }
                if !as_map {
                    add_c(
                        &mut a,
                        string_obj(arena_string(arena, cstr_as_string(item))),
                    );
                    continue;
                }
                let colon = crate::memory::cstr_as_bytes(item)
                    .iter()
                    .position(|&b| b == b':')
                    .map_or(ptr::null_mut(), |i| item.add(i));
                let mut val: *mut c_char = ptr::null_mut();
                if !colon.is_null() {
                    *colon = NUL;
                    val = colon.add(1);
                }
                let key = arena_string(arena, cstr_as_string(item));
                let v = if (flags & K_OPT_FLAG_FLAG_LIST) != 0 || val.is_null() {
                    boolean_obj(true)
                } else {
                    string_obj(arena_string(arena, cstr_as_string(val)))
                };
                put_c(&mut d, key.data, v);
            }
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
pub fn nvim_set_option_value(
    channel_id: u64,
    name: NvimString,
    value: Object,
    opts: &mut KeyDict_option,
    arena: &mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = K_OPT_SCOPE_GLOBAL;
        let mut operation: SetOp = OP_NONE;
        let mut to = OptTarget::Global;
        let mut dry_run = false;

        if !validate_option_value_args(
            opts,
            name.data,
            true,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut to,
            ptr::null_mut(),
            Some(&mut operation),
            Some(&mut dry_run),
            err,
        ) {
            return OBJECT_NIL;
        }

        if scope == K_OPT_SCOPE_WIN
            && opt_flags == 0
            && option_has_scope(opt_idx, K_OPT_SCOPE_GLOBAL)
        {
            opt_flags = OPT_LOCAL;
        }

        let mut merged_val = OBJECT_NIL;
        let mut errmsg: *const c_char = std::ptr::null();
        let option = get_option(opt_idx);
        let is_array = value.obj_type == ObjectType::ObjectTypeArray;
        let is_dict = value.obj_type == ObjectType::ObjectTypeDict;

        let buf = if let (K_OPT_SCOPE_BUF, OptTarget::Buf(b)) = (scope, to) {
            b
        } else {
            g_st().curbuf
        };
        let win = if let (K_OPT_SCOPE_WIN, OptTarget::Win(w)) = (scope, to) {
            w
        } else {
            curwin_ptr()
        };
        let varp = get_varp_from(&mut *option, &mut *buf, &mut *win);

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
            let mut item = value;
            if per_item && is_array {
                item = Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(Array {
                        items: value.data.array().items.add(i),
                        size: 1,
                        capacity: 0,
                    }),
                };
            } else if per_item {
                item = Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(Dict {
                        items: value.data.dict().items.add(i),
                        size: 1,
                        capacity: 0,
                    }),
                };
            }

            let mut error = false;
            let optval_right = optval_from_obj(opt_idx, item, operation, &mut error);

            if error {
                api_err_exp(
                    err,
                    name.data,
                    c"a valid type",
                    api_typename(value.obj_type),
                );
                optval_free(merged_val);
                return OBJECT_NIL;
            }

            let mut argp: *mut c_char = ptr::null_mut();

            match optval_right.obj_type {
                ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {}
                ObjectType::ObjectTypeString => {
                    let optval_escaped = escape_option_str_cmdline(optval_right.data.string().data);
                    let escaped_bytes = optval_escaped.as_bytes();
                    let mut buf_bytes = Vec::with_capacity(1 + escaped_bytes.len());
                    buf_bytes.push(b'=');
                    buf_bytes.extend_from_slice(escaped_bytes);
                    argp = arena_memdupz(arena, buf_bytes.as_ptr().cast(), buf_bytes.len());
                }
                ObjectType::ObjectTypeInteger => {
                    let s = format!("={}", optval_right.data.integer());
                    argp = arena_memdupz(arena, s.as_ptr().cast(), s.len());
                }
                ObjectType::ObjectTypeBoolean => {
                    merged_val = optval_right;
                }
                ObjectType::ObjectTypeLuaRef => {
                    merged_val = copy_object(optval_right, None);
                }
                _ => {
                    std::process::abort();
                }
            }

            optval_free(optval_right);

            if optval_right.obj_type == ObjectType::ObjectTypeInteger
                || optval_right.obj_type == ObjectType::ObjectTypeString
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
                    Some(&mut oldval),
                    &mut errmsg,
                );
                optval_free(oldval);
                if !errmsg.is_null() {
                    api_set_error!(err, ErrorType::ErrorTypeValidation, c"%s", errmsg);
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
            let mut args = Optset {
                os_varp: ptr::null_mut(),
                os_idx: opt_idx,
                os_flags: opt_flags,
                os_oldval: OBJECT_NIL,
                os_newval: merged_val,
                os_prep: OptSetPrep::None,
                os_value_checked: false,
                os_value_changed: false,
                os_restore_chartab: false,
                os_errbuf: Some(errbuf),
                os_win: win,
                os_buf: buf,
            };
            errmsg = validate_and_prepare_option_value(&mut args);
            merged_val = args.os_newval;
            if !errmsg.is_null() {
                api_set_error!(err, ErrorType::ErrorTypeException, c"%s", errmsg);
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

        let rv = nvim_option_to_object(name, merged_val, arena, err);
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
pub fn nvim_get_all_options_info(arena: &mut Arena, _err: &mut Error) -> Dict {
    get_all_options_info(arena)
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
pub fn nvim_get_option_info2(
    name: NvimString,
    opts: &mut KeyDict_option,
    arena: &mut Arena,
    err: &mut Error,
) -> Dict {
    {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = K_OPT_SCOPE_GLOBAL;
        let mut from = OptTarget::Global;

        if !validate_option_value_args(
            opts,
            name.data,
            false,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut from,
            ptr::null_mut(),
            None,
            None,
            err,
        ) {
            return Dict::new();
        }

        let buf = if let (K_OPT_SCOPE_BUF, OptTarget::Buf(b)) = (scope, from) {
            b
        } else {
            g_st().curbuf
        };
        let win = if let (K_OPT_SCOPE_WIN, OptTarget::Win(w)) = (scope, from) {
            w
        } else {
            curwin_ptr()
        };

        get_option_info(name, opt_flags, buf, unsafe { &mut *win }, arena, err)
    }
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
        {
            assert!((crate::memory::cstr_as_opt_bytes(c"local") == Some(b"local")));
            assert!(!(crate::memory::cstr_as_opt_bytes(c"local") == Some(b"global")));
            assert!(!{
                let _a: *const c_char = std::ptr::null();
                !_a.is_null() && crate::memory::cstr_as_bytes(_a) == b"local"
            });
        }
    }
}
