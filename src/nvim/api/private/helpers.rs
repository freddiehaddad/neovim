//! Private API helper functions (`src/nvim/api/private/helpers.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::eval::typval::tv_dict_watcher_notify;
use crate::globals::*;
#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, api_new_luaref};
#[allow(unused_imports)]
use crate::message::hl_msg_free;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::api::private::converter::{object_to_vim, sctx_T, vim_to_object};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_exp,
};
use crate::api::tabpage::{Buffer, Tabpage, Window, pos_T, tabpage_T, win_T};
use crate::clipboard::typval_T;
use crate::cursor::{colnr_T, linenr_T};
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::buf_T;
use crate::eval::typval::{
    tv_clear, tv_copy, tv_dict_add, tv_dict_find, tv_dict_item_alloc_len, tv_dict_item_remove,
};
use crate::eval::vars::{LuaRef, before_set_vvar, get_vimvar_dict, scid_T};
use crate::ex_eval::{
    ET_ERROR, discard_current_exception, except_T, free_global_msglist, get_exception_string,
    msglist_T,
};
use crate::garray::garray_T;
use crate::highlight_group::{highlight_num_groups, syn_check_group, syn_id2name};
use crate::lua::converter::{Boolean, FieldHashfn, Float, KeySetLink, OptKeySet};
use crate::map::{Map, mh_get_int};
use crate::map_glyph_cache::{MH_TOMBSTONE, NvimString};
use crate::mark::setmark_pos;
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{
    Arena, ArenaMem, arena_alloc, arena_finish, arena_memdupz, memchrsub, xfree, xmalloc, xmemdupz,
    xrealloc, xstrdup, xstrndup, xstrnlen,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::unpacker::unpack;
use crate::runtime::script_is_lua;

const NUL: c_char = 0;
const NL: c_char = b'\n' as c_char;
const CAR: c_char = b'\r' as c_char;
const MAXLNUM: i64 = 0x7fffffff;
const MAXCOL: i64 = 0x7fffffff;

const DI_FLAGS_RO: u8 = 1;
const DI_FLAGS_FIX: u8 = 4;
const DI_FLAGS_LOCK: u8 = 8;

const HLF_E: c_int = 6;

const SID_API_CLIENT: scid_T = -9;
const SID_LUA: scid_T = -8;
const INTERNAL_CALL_MASK: u64 = 1u64 << 63;
const VIML_INTERNAL_CALL: u64 = INTERNAL_CALL_MASK;
const LUA_INTERNAL_CALL: u64 = VIML_INTERNAL_CALL + 1;

const OBJECT_NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[inline]
pub const fn nil_obj() -> Object {
    OBJECT_NIL
}

#[inline]
pub const fn boolean_obj(b: Boolean) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

#[inline]
pub const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
pub const fn float_obj(f: Float) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeFloat,
        data: ObjectData { floating: f },
    }
}

#[inline]
pub const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
pub const fn buffer_obj(b: Buffer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBuffer,
        data: ObjectData {
            integer: b as Integer,
        },
    }
}

#[inline]
pub const fn window_obj(w: Window) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeWindow,
        data: ObjectData {
            integer: w as Integer,
        },
    }
}

#[inline]
pub const fn tabpage_obj(t: Tabpage) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeTabpage,
        data: ObjectData {
            integer: t as Integer,
        },
    }
}

#[inline]
pub const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
pub const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
pub const fn luaref_obj(r: LuaRef) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeLuaRef,
        data: ObjectData { luaref: r },
    }
}

#[inline]
pub unsafe fn cstr_as_obj(s: *const c_char) -> Object {
    unsafe { string_obj(cstr_as_string(s)) }
}

#[inline]
pub unsafe fn add_c(array: &mut Array, item: Object) {
    debug_assert!(!array.items.is_null() && array.size < array.capacity);
    unsafe {
        *array.items.add(array.size) = item;
    }
    array.size += 1;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TryState {
    pub current_exception: *mut except_T,
    pub private_msg_list: *mut msglist_T,
    pub msg_list: *const *const msglist_T,
    pub got_int: c_int,
    pub did_throw: bool,
    pub need_rethrow: c_int,
    pub did_emsg: c_int,
}

pub type ArrayBuilder = crate::kvec::KVecWithInit<Object, 16>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HlMessageChunk {
    pub text: NvimString,
    pub hl_id: c_int,
}

pub use crate::api::vim::HlMessage;

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_set_error_str(
    err: *mut Error,
    err_type: ErrorType,
    msg: *const c_char,
) {
    unsafe {
        debug_assert!(!matches!(err_type, ErrorType::kErrorTypeNone));
        (*err).msg = xstrdup(msg);
        (*err).err_type = err_type;
    }
}

#[macro_export]
macro_rules! api_set_error {
    ($err:expr, $err_type:expr, $fmt:expr $(,)?) => {{
        $crate::api::private::helpers::api_set_error_str($err, $err_type, $fmt);
    }};
    ($err:expr, $err_type:expr, $fmt:expr, $($arg:expr),+ $(,)?) => {{
        let err_ptr: *mut $crate::api::private::validate::Error = $err;
        let et: $crate::api::private::validate::ErrorType = $err_type;
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        #[cfg(not(test))]
        {
            let len = $crate::strings::vim_snprintf!(std::ptr::null_mut(), 0, fmt_ptr, $($arg),+);
            let bufsize = ((len.max(0) as usize) + 1).min(1024 * 1024);
            let buf = $crate::memory::xmalloc(bufsize).cast::<std::ffi::c_char>();
            $crate::strings::vim_snprintf!(buf, bufsize, fmt_ptr, $($arg),+);
            (*err_ptr).msg = buf;
            (*err_ptr).err_type = et;
        }
        #[cfg(test)]
        {
            let _ = ($($crate::strings::IntoFmtVal::into_fmt_val($arg)),+);
            $crate::api::private::helpers::api_set_error_str(err_ptr, et, fmt_ptr);
        }
    }};
}
pub use api_set_error;

#[inline]
unsafe fn tv_dict_is_watched(d: *const dict_T) -> bool {
    unsafe { !d.is_null() && (*d).watchers[0].cast_const() != (&raw const (*d).watchers).cast() }
}

#[inline]
unsafe fn pmap_get_int(map: *mut Map<c_int, *mut c_void>, key: c_int) -> *mut c_void {
    unsafe {
        let k = mh_get_int(&raw mut (*map).set, key);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            *(*map).values.add(k as usize)
        }
    }
}

#[inline]
const fn error_set(e: *const Error) -> bool {
    unsafe { !matches!((*e).err_type, ErrorType::kErrorTypeNone) }
}

/// Start block that may cause Vimscript exceptions while evaluating another code (`try_enter`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn try_enter(tstate: *mut TryState) {
    unsafe {
        *tstate = TryState {
            current_exception,
            msg_list: msg_list.cast(),
            private_msg_list: ptr::null_mut(),
            got_int: c_int::from(got_int),
            did_throw,
            need_rethrow: c_int::from(need_rethrow),
            did_emsg,
        };
        msg_list = &raw mut (*tstate).private_msg_list;
        current_exception = ptr::null_mut();
        got_int = false;
        did_throw = false;
        need_rethrow = false;
        did_emsg = 0;
        trylevel += 1;
    }
}

/// Ends a `try_enter` block; sets error message if any (`try_leave`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn try_leave(tstate: *const TryState, err: *mut Error) {
    unsafe {
        debug_assert!(trylevel > 0);
        trylevel -= 1;

        did_emsg = 0;
        force_abort = false;

        if got_int {
            if did_throw {
                discard_current_exception();
            }
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Keyboard interrupt".as_ptr(),
            );
            got_int = false;
        } else if !msg_list.is_null() && !(*msg_list).is_null() {
            let mut should_free = false;
            let msg = get_exception_string(
                (*msg_list).cast(),
                ET_ERROR,
                ptr::null_mut(),
                &raw mut should_free,
            );
            api_set_error!(err, ErrorType::kErrorTypeException, c"%s".as_ptr(), msg);
            free_global_msglist();
            if should_free {
                xfree(msg.cast());
            }
        } else if did_throw || need_rethrow {
            if *(*current_exception).throw_name != NUL {
                if (*current_exception).throw_lnum != 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"%s, line %d: %s".as_ptr(),
                        (*current_exception).throw_name,
                        (*current_exception).throw_lnum,
                        (*current_exception).value,
                    );
                } else {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"%s: %s".as_ptr(),
                        (*current_exception).throw_name,
                        (*current_exception).value,
                    );
                }
            } else {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    (*current_exception).value,
                );
            }
            discard_current_exception();
        }

        msg_list = (*tstate).msg_list.cast_mut().cast();
        current_exception = (*tstate).current_exception;
        got_int = (*tstate).got_int != 0;
        did_throw = (*tstate).did_throw;
        need_rethrow = (*tstate).need_rethrow != 0;
        did_emsg = (*tstate).did_emsg;
    }
}

/// Recursively expands a vimscript value in a dict (`dict_get_value`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dict_get_value(
    dict: *mut dict_T,
    key: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let di = tv_dict_find(dict, key.data, key.size as isize);
        if di.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Key not found: %s".as_ptr(),
                key.data,
            );
            return OBJECT_NIL;
        }
        vim_to_object(&raw mut (*di).di_tv, arena, true)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dict_check_writable(
    dict: *mut dict_T,
    key: NvimString,
    del: bool,
    err: *mut Error,
) -> *mut dictitem_T {
    unsafe {
        let di = tv_dict_find(dict, key.data, key.size as isize);
        if !di.is_null() {
            if ((*di).di_flags & DI_FLAGS_RO) != 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Key is read-only: %s".as_ptr(),
                    key.data,
                );
            } else if ((*di).di_flags & DI_FLAGS_LOCK) != 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Key is locked: %s".as_ptr(),
                    key.data,
                );
            } else if del && ((*di).di_flags & DI_FLAGS_FIX) != 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Key is fixed: %s".as_ptr(),
                    key.data,
                );
            }
        } else if (*dict).dv_lock != 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Dict is locked".as_ptr(),
            );
        } else if key.size == 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Key name is empty".as_ptr(),
            );
        } else if key.size > c_int::MAX as usize {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Key name is too long".as_ptr(),
            );
        }
        di
    }
}

/// Set a value in a scope dict (`dict_set_var`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dict_set_var(
    dict: *mut dict_T,
    key: NvimString,
    value: Object,
    del: bool,
    retval: bool,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut rv = OBJECT_NIL;
        let mut di = dict_check_writable(dict, key, del, err);
        if error_set(err) {
            return rv;
        }

        let watched = tv_dict_is_watched(dict);
        if del {
            if di.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Key not found: %s".as_ptr(),
                    key.data,
                );
            } else {
                if watched {
                    tv_dict_watcher_notify(dict, key.data, ptr::null_mut(), &raw mut (*di).di_tv);
                }
                if retval {
                    rv = vim_to_object(&raw mut (*di).di_tv, arena, false);
                }
                tv_dict_item_remove(dict, di);
            }
        } else {
            let mut tv: typval_T = zeroed();
            object_to_vim(value, &raw mut tv, err);
            let mut oldtv: typval_T = zeroed();

            if di.is_null() {
                di = tv_dict_item_alloc_len(key.data, key.size);
                tv_dict_add(dict, di);
            } else {
                if retval {
                    rv = vim_to_object(&raw mut (*di).di_tv, arena, false);
                }
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
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"Setting v:%s to value with wrong type".as_ptr(),
                            key.data,
                        );
                    }
                    return rv;
                }
                if watched {
                    tv_copy(&raw mut (*di).di_tv, &raw mut oldtv);
                }
                tv_clear(&raw mut (*di).di_tv);
            }

            tv_copy(&raw mut tv, &raw mut (*di).di_tv);
            if watched {
                tv_dict_watcher_notify(dict, key.data, &raw mut tv, &raw mut oldtv);
                tv_clear(&raw mut oldtv);
            }
            tv_clear(&raw mut tv);
        }

        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T {
    unsafe {
        if buffer == 0 {
            return curbuf;
        }
        let rv = pmap_get_int(&raw mut buffer_handles, buffer).cast::<buf_T>();
        if rv.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Invalid buffer id: %lld".as_ptr(),
                i64::from(buffer),
            );
            return ptr::null_mut();
        }
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_window_by_handle(window: Window, err: *mut Error) -> *mut win_T {
    unsafe {
        if window == 0 {
            return curwin;
        }
        let rv = pmap_get_int(&raw mut window_handles, window).cast::<win_T>();
        if rv.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Invalid window id: %lld".as_ptr(),
                i64::from(window),
            );
            return ptr::null_mut();
        }
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_tab_by_handle(tabpage: Tabpage, err: *mut Error) -> *mut tabpage_T {
    unsafe {
        if tabpage == 0 {
            return curtab;
        }
        let rv = pmap_get_int(&raw mut tabpage_handles, tabpage).cast::<tabpage_T>();
        if rv.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Invalid tabpage id: %lld".as_ptr(),
                i64::from(tabpage),
            );
            return ptr::null_mut();
        }
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cchar_to_string(c: c_char) -> NvimString {
    unsafe {
        let buf: [c_char; 2] = [c, NUL];
        NvimString {
            data: xmemdupz(buf.as_ptr().cast(), 1).cast(),
            size: usize::from(c != NUL),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cstr_to_string(str_: *const c_char) -> NvimString {
    unsafe {
        if str_.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        let len = CStr::from_ptr(str_).to_bytes().len();
        NvimString {
            data: xmemdupz(str_.cast(), len).cast(),
            size: len,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn string_to_cstr(str_: NvimString) -> *mut c_char {
    unsafe { xstrndup(str_.data, str_.size) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cbuf_to_string(buf: *const c_char, size: usize) -> NvimString {
    unsafe {
        NvimString {
            data: xmemdupz(buf.cast(), size).cast(),
            size,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cstrn_to_string(str_: *const c_char, maxsize: usize) -> NvimString {
    unsafe { cbuf_to_string(str_, xstrnlen(str_, maxsize)) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cstrn_as_string(str_: *mut c_char, maxsize: usize) -> NvimString {
    unsafe {
        NvimString {
            data: str_,
            size: xstrnlen(str_, maxsize),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cstr_as_string(str_: *const c_char) -> NvimString {
    unsafe {
        if str_.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        NvimString {
            data: str_.cast_mut(),
            size: CStr::from_ptr(str_).to_bytes().len(),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ga_take_string(ga: *mut garray_T) -> NvimString {
    unsafe {
        let str_ = NvimString {
            data: (*ga).ga_data.cast(),
            size: (*ga).ga_len as usize,
        };
        (*ga).ga_data = ptr::null_mut();
        (*ga).ga_len = 0;
        (*ga).ga_maxlen = 0;
        str_
    }
}

unsafe fn kvi_push_obj(v: *mut ArrayBuilder, item: Object) {
    unsafe {
        if (*v).size == (*v).capacity {
            (*v).capacity <<= 1;
            let init_ptr = (&raw mut (*v).init_array).cast::<Object>();
            if (*v).items == init_ptr {
                let new_items = xmalloc((*v).capacity * size_of::<Object>()).cast::<Object>();
                ptr::copy_nonoverlapping(init_ptr, new_items, (*v).size);
                (*v).items = new_items;
            } else {
                (*v).items =
                    xrealloc((*v).items.cast(), (*v).capacity * size_of::<Object>()).cast();
            }
        }
        *(*v).items.add((*v).size) = item;
        (*v).size += 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn string_to_array(
    input: NvimString,
    crlf: bool,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let mut ret: ArrayBuilder = zeroed();
        ret.capacity = 16;
        ret.size = 0;
        ret.items = (&raw mut ret.init_array).cast::<Object>();

        let mut i: usize = 0;
        while i < input.size {
            let start = input.data.add(i);
            let mut end = start;
            let mut line_len: usize = 0;
            while line_len < input.size - i {
                end = start.add(line_len);
                if *end == NL || (crlf && *end == CAR) {
                    break;
                }
                line_len += 1;
            }
            i += line_len;
            if crlf && *end == CAR && i + 1 < input.size && *end.add(1) == NL {
                i += 1;
            }
            let s = arena_string(
                arena,
                NvimString {
                    data: start,
                    size: line_len,
                },
            );
            memchrsub(s.data.cast(), NUL, NL, line_len);
            kvi_push_obj(
                &raw mut ret,
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData { string: s },
                },
            );
            if i + 1 == input.size && (*end == NL || (crlf && *end == CAR)) {
                let empty_s = arena_string(
                    arena,
                    NvimString {
                        data: c"".as_ptr().cast_mut(),
                        size: 0,
                    },
                );
                kvi_push_obj(
                    &raw mut ret,
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData { string: empty_s },
                    },
                );
            }
            i += 1;
        }

        arena_take_arraybuilder(arena, &raw mut ret)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn normalize_index(
    buf: *mut buf_T,
    mut index: i64,
    end_exclusive: bool,
    oob: *mut bool,
) -> i64 {
    unsafe {
        debug_assert!((*buf).b_ml.ml_line_count > 0);
        let max_index = i64::from((*buf).b_ml.ml_line_count) + i64::from(end_exclusive) - 1;
        if index < 0 {
            index = max_index + index + 1;
        }
        if index > max_index {
            *oob = true;
            index = max_index;
        } else if index < 0 {
            *oob = true;
            index = 0;
        }
        index + 1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_get_text(
    buf: *mut buf_T,
    lnum: i64,
    mut start_col: i64,
    mut end_col: i64,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let rv = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        if lnum >= MAXLNUM {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"'line index' value out of range".as_ptr(),
            );
            return rv;
        }

        let bufstr = ml_get_buf(buf, lnum as linenr_T);
        let line_length = i64::from(ml_get_buf_len(buf, lnum as linenr_T));

        if start_col < 0 {
            start_col = line_length + start_col + 1;
        }
        if end_col < 0 {
            end_col = line_length + end_col + 1;
        }

        start_col = start_col.clamp(0, line_length);
        end_col = end_col.clamp(0, line_length);

        if start_col > end_col {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"start_col must be less than or equal to end_col".as_ptr(),
            );
            return rv;
        }

        NvimString {
            data: bufstr.add(start_col as usize),
            size: (end_col - start_col) as usize,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_free_string(value: NvimString) {
    unsafe {
        xfree(value.data.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn arena_array(arena: *mut Arena, max_size: usize) -> Array {
    unsafe {
        Array {
            size: 0,
            capacity: max_size,
            items: arena_alloc(arena, size_of::<Object>() * max_size, true).cast(),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict {
    unsafe {
        Dict {
            size: 0,
            capacity: max_size,
            items: arena_alloc(arena, size_of::<KeyValuePair>() * max_size, true).cast(),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn arena_string(arena: *mut Arena, str_: NvimString) -> NvimString {
    unsafe {
        if str_.size != 0 {
            NvimString {
                data: arena_memdupz(arena, str_.data, str_.size),
                size: str_.size,
            }
        } else {
            NvimString {
                data: if !arena.is_null() {
                    c"".as_ptr().cast_mut()
                } else {
                    xstrdup(c"".as_ptr())
                },
                size: 0,
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn arena_take_arraybuilder(
    arena: *mut Arena,
    arr: *mut ArrayBuilder,
) -> Array {
    unsafe {
        let mut ret = arena_array(arena, (*arr).size);
        ret.size = (*arr).size;
        if ret.size > 0 {
            ptr::copy_nonoverlapping((*arr).items, ret.items, ret.size);
        }
        let init_ptr = (&raw mut (*arr).init_array).cast::<Object>();
        if (*arr).items != init_ptr {
            xfree((*arr).items.cast());
        }
        ret
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_free_object(value: Object) {
    unsafe {
        match value.obj_type {
            ObjectType::kObjectTypeUnset
            | ObjectType::kObjectTypeNil
            | ObjectType::kObjectTypeBoolean
            | ObjectType::kObjectTypeInteger
            | ObjectType::kObjectTypeFloat
            | ObjectType::kObjectTypeBuffer
            | ObjectType::kObjectTypeWindow
            | ObjectType::kObjectTypeTabpage => {}
            ObjectType::kObjectTypeString => api_free_string(value.data.string),
            ObjectType::kObjectTypeArray => api_free_array(value.data.array),
            ObjectType::kObjectTypeDict => api_free_dict(value.data.dict),
            ObjectType::kObjectTypeLuaRef => api_free_luaref(value.data.luaref),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_free_array(value: Array) {
    unsafe {
        for i in 0..value.size {
            api_free_object(*value.items.add(i));
        }
        xfree(value.items.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_free_dict(value: Dict) {
    unsafe {
        for i in 0..value.size {
            let item = *value.items.add(i);
            api_free_string(item.key);
            api_free_object(item.value);
        }
        xfree(value.items.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_clear_error(value: *mut Error) {
    unsafe {
        if !error_set(value) {
            return;
        }
        xfree((*value).msg.cast());
        (*value).msg = ptr::null_mut();
        (*value).err_type = ErrorType::kErrorTypeNone;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_string(str_: NvimString, arena: *mut Arena) -> NvimString {
    unsafe {
        if !str_.data.is_null() {
            NvimString {
                data: arena_memdupz(arena, str_.data, str_.size),
                size: str_.size,
            }
        } else {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_array(array: Array, arena: *mut Arena) -> Array {
    unsafe {
        let mut rv = arena_array(arena, array.size);
        for i in 0..array.size {
            *rv.items.add(i) = copy_object(*array.items.add(i), arena);
        }
        rv.size = array.size;
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_dict(dict: Dict, arena: *mut Arena) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, dict.size);
        for i in 0..dict.size {
            let item = *dict.items.add(i);
            let k_copy = copy_string(item.key, arena);
            *rv.items.add(i) = KeyValuePair {
                key: cstr_as_string(k_copy.data),
                value: copy_object(item.value, arena),
            };
        }
        rv.size = dict.size;
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_object(obj: Object, arena: *mut Arena) -> Object {
    unsafe {
        match obj.obj_type {
            ObjectType::kObjectTypeUnset
            | ObjectType::kObjectTypeBuffer
            | ObjectType::kObjectTypeTabpage
            | ObjectType::kObjectTypeWindow
            | ObjectType::kObjectTypeNil
            | ObjectType::kObjectTypeBoolean
            | ObjectType::kObjectTypeInteger
            | ObjectType::kObjectTypeFloat => obj,
            ObjectType::kObjectTypeString => Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData {
                    string: copy_string(obj.data.string, arena),
                },
            },
            ObjectType::kObjectTypeArray => Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData {
                    array: copy_array(obj.data.array, arena),
                },
            },
            ObjectType::kObjectTypeDict => Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData {
                    dict: copy_dict(obj.data.dict, arena),
                },
            },
            ObjectType::kObjectTypeLuaRef => Object {
                obj_type: ObjectType::kObjectTypeLuaRef,
                data: ObjectData {
                    luaref: api_new_luaref(obj.data.luaref),
                },
            },
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_object_to_bool(
    obj: Object,
    what: *const c_char,
    nil_value: bool,
    err: *mut Error,
) -> bool {
    unsafe {
        match obj.obj_type {
            ObjectType::kObjectTypeBoolean => obj.data.boolean,
            ObjectType::kObjectTypeInteger => obj.data.integer != 0,
            ObjectType::kObjectTypeNil => nil_value,
            _ => {
                api_err_exp(err, what, c"boolean".as_ptr(), ptr::null());
                false
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn object_to_hl_id(
    obj: Object,
    what: *const c_char,
    err: *mut Error,
) -> c_int {
    unsafe {
        match obj.obj_type {
            ObjectType::kObjectTypeString => {
                let str_ = obj.data.string;
                if str_.size != 0 {
                    syn_check_group(str_.data, str_.size)
                } else {
                    0
                }
            }
            ObjectType::kObjectTypeInteger => {
                let id = obj.data.integer as c_int;
                if (1..=highlight_num_groups()).contains(&id) {
                    id
                } else {
                    0
                }
            }
            _ => {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid 'hl_group': '%s'".as_ptr(),
                    what,
                );
                0
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn api_typename(t: ObjectType) -> *const c_char {
    match t {
        ObjectType::kObjectTypeUnset => c"unset".as_ptr(),
        ObjectType::kObjectTypeNil => c"nil".as_ptr(),
        ObjectType::kObjectTypeBoolean => c"Boolean".as_ptr(),
        ObjectType::kObjectTypeInteger => c"Integer".as_ptr(),
        ObjectType::kObjectTypeFloat => c"Float".as_ptr(),
        ObjectType::kObjectTypeString => c"String".as_ptr(),
        ObjectType::kObjectTypeArray => c"Array".as_ptr(),
        ObjectType::kObjectTypeDict => c"Dict".as_ptr(),
        ObjectType::kObjectTypeLuaRef => c"Function".as_ptr(),
        ObjectType::kObjectTypeBuffer => c"Buffer".as_ptr(),
        ObjectType::kObjectTypeWindow => c"Window".as_ptr(),
        ObjectType::kObjectTypeTabpage => c"Tabpage".as_ptr(),
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_hl_msg(chunks: Array, is_err: bool, err: *mut Error) -> HlMessage {
    unsafe {
        let mut hl_msg = HlMessage::new();
        for i in 0..chunks.size {
            let item = *chunks.items.add(i);
            if item.obj_type != ObjectType::kObjectTypeArray {
                api_err_exp(
                    err,
                    c"chunk".as_ptr(),
                    api_typename(ObjectType::kObjectTypeArray),
                    api_typename(item.obj_type),
                );
                hl_msg_free(hl_msg);
                return HlMessage::new();
            }
            let chunk = item.data.array;
            if chunk.size == 0
                || chunk.size > 2
                || (*chunk.items).obj_type != ObjectType::kObjectTypeString
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"Invalid chunk: expected Array with 1 or 2 Strings".as_ptr(),
                );
                hl_msg_free(hl_msg);
                return HlMessage::new();
            }

            let str_ = copy_string((*chunk.items).data.string, ptr::null_mut());
            let hl_id = if chunk.size == 2 {
                object_to_hl_id(*chunk.items.add(1), c"text highlight".as_ptr(), err)
            } else if is_err {
                HLF_E
            } else {
                0
            };
            hl_msg.push(HlMessageChunk { text: str_, hl_id });
        }
        hl_msg
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_dict_to_keydict(
    retval: *mut c_void,
    hashy: FieldHashfn,
    dict: Dict,
    err: *mut Error,
) -> bool {
    unsafe {
        let Some(hashy_fn) = hashy else {
            return false;
        };
        for i in 0..dict.size {
            let k = (*dict.items.add(i)).key;
            let field = hashy_fn(k.data, k.size);
            if field.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid key: '%.*s'".as_ptr(),
                    k.size as c_int,
                    k.data,
                );
                return false;
            }

            if (*field).opt_index >= 0 {
                let ks = retval.cast::<OptKeySet>();
                (*ks).is_set_ |= 1u64 << (*field).opt_index;
            }

            let mem = retval.cast::<u8>().add((*field).ptr_off);
            let value = &raw mut (*dict.items.add(i)).value;
            let field_type: ObjectType = std::mem::transmute((*field).key_type);

            match field_type {
                ObjectType::kObjectTypeNil => {
                    *mem.cast::<Object>() = *value;
                }
                ObjectType::kObjectTypeInteger => {
                    if (*field).is_hlgroup {
                        let mut hl_id: c_int = 0;
                        if (*value).obj_type != ObjectType::kObjectTypeNil {
                            hl_id = object_to_hl_id(*value, k.data, err);
                            if error_set(err) {
                                return false;
                            }
                        }
                        *mem.cast::<Integer>() = Integer::from(hl_id);
                    } else {
                        if (*value).obj_type != ObjectType::kObjectTypeInteger {
                            api_err_exp(
                                err,
                                (*field).str,
                                api_typename(ObjectType::kObjectTypeInteger),
                                api_typename((*value).obj_type),
                            );
                            return false;
                        }
                        *mem.cast::<Integer>() = (*value).data.integer;
                    }
                }
                ObjectType::kObjectTypeFloat => {
                    let val = mem.cast::<Float>();
                    if (*value).obj_type == ObjectType::kObjectTypeInteger {
                        *val = (*value).data.integer as Float;
                    } else if (*value).obj_type == ObjectType::kObjectTypeFloat {
                        *val = (*value).data.floating;
                    } else {
                        api_err_exp(
                            err,
                            (*field).str,
                            api_typename(ObjectType::kObjectTypeFloat),
                            api_typename((*value).obj_type),
                        );
                        return false;
                    }
                }
                ObjectType::kObjectTypeBoolean => {
                    *mem.cast::<Boolean>() = api_object_to_bool(*value, (*field).str, false, err);
                    if error_set(err) {
                        return false;
                    }
                }
                ObjectType::kObjectTypeString => {
                    if (*value).obj_type != ObjectType::kObjectTypeString {
                        api_err_exp(
                            err,
                            (*field).str,
                            api_typename(ObjectType::kObjectTypeString),
                            api_typename((*value).obj_type),
                        );
                        return false;
                    }
                    *mem.cast::<NvimString>() = (*value).data.string;
                }
                ObjectType::kObjectTypeArray => {
                    if (*value).obj_type != ObjectType::kObjectTypeArray {
                        api_err_exp(
                            err,
                            (*field).str,
                            api_typename(ObjectType::kObjectTypeArray),
                            api_typename((*value).obj_type),
                        );
                        return false;
                    }
                    *mem.cast::<Array>() = (*value).data.array;
                }
                ObjectType::kObjectTypeDict => {
                    let val = mem.cast::<Dict>();
                    if (*value).obj_type == ObjectType::kObjectTypeArray
                        && (*value).data.array.size == 0
                    {
                        *val = Dict {
                            items: ptr::null_mut(),
                            size: 0,
                            capacity: 0,
                        };
                    } else if (*value).obj_type == ObjectType::kObjectTypeDict {
                        *val = (*value).data.dict;
                    } else {
                        api_err_exp(
                            err,
                            (*field).str,
                            api_typename(field_type),
                            api_typename((*value).obj_type),
                        );
                        return false;
                    }
                }
                ObjectType::kObjectTypeBuffer
                | ObjectType::kObjectTypeWindow
                | ObjectType::kObjectTypeTabpage => {
                    if (*value).obj_type == ObjectType::kObjectTypeInteger
                        || (*value).obj_type == field_type
                    {
                        *mem.cast::<handle_T>() = (*value).data.integer as handle_T;
                    } else {
                        api_err_exp(
                            err,
                            (*field).str,
                            api_typename(field_type),
                            api_typename((*value).obj_type),
                        );
                        return false;
                    }
                }
                ObjectType::kObjectTypeLuaRef => {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Invalid key: '%.*s' is only allowed from Lua".as_ptr(),
                        k.size as c_int,
                        k.data,
                    );
                    return false;
                }
                ObjectType::kObjectTypeUnset => std::process::abort(),
            }
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_keydict_to_dict(
    value: *mut c_void,
    table: *mut KeySetLink,
    max_size: usize,
    arena: *mut Arena,
) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, max_size);
        let mut i: usize = 0;
        while !(*table.add(i)).str.is_null() {
            let field = table.add(i);
            let mut is_set = true;
            if (*field).opt_index >= 0 {
                let ks = value.cast::<OptKeySet>();
                is_set = ((*ks).is_set_ & (1u64 << (*field).opt_index)) != 0;
            }
            if !is_set {
                i += 1;
                continue;
            }

            let mem = value.cast::<u8>().add((*field).ptr_off);
            let field_type: ObjectType = std::mem::transmute((*field).key_type);
            let val = match field_type {
                ObjectType::kObjectTypeNil => *mem.cast::<Object>(),
                ObjectType::kObjectTypeInteger => Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: *mem.cast::<Integer>(),
                    },
                },
                ObjectType::kObjectTypeFloat => Object {
                    obj_type: ObjectType::kObjectTypeFloat,
                    data: ObjectData {
                        floating: *mem.cast::<Float>(),
                    },
                },
                ObjectType::kObjectTypeBoolean => Object {
                    obj_type: ObjectType::kObjectTypeBoolean,
                    data: ObjectData {
                        boolean: *mem.cast::<Boolean>(),
                    },
                },
                ObjectType::kObjectTypeString => Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: *mem.cast::<NvimString>(),
                    },
                },
                ObjectType::kObjectTypeArray => Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData {
                        array: *mem.cast::<Array>(),
                    },
                },
                ObjectType::kObjectTypeDict => Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: *mem.cast::<Dict>(),
                    },
                },
                ObjectType::kObjectTypeBuffer
                | ObjectType::kObjectTypeWindow
                | ObjectType::kObjectTypeTabpage => Object {
                    obj_type: field_type,
                    data: ObjectData {
                        integer: Integer::from(*mem.cast::<handle_T>()),
                    },
                },
                ObjectType::kObjectTypeLuaRef => OBJECT_NIL,
                ObjectType::kObjectTypeUnset => std::process::abort(),
            };

            *rv.items.add(rv.size) = KeyValuePair {
                key: cstr_as_string((*field).str),
                value: val,
            };
            rv.size += 1;
            i += 1;
        }
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_luarefs_free_object(value: Object) {
    unsafe {
        match value.obj_type {
            ObjectType::kObjectTypeLuaRef => api_free_luaref(value.data.luaref),
            ObjectType::kObjectTypeArray => api_luarefs_free_array(value.data.array),
            ObjectType::kObjectTypeDict => api_luarefs_free_dict(value.data.dict),
            _ => {}
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_luarefs_free_keydict(dict: *mut c_void, table: *mut KeySetLink) {
    unsafe {
        let mut i: usize = 0;
        while !(*table.add(i)).str.is_null() {
            let field = table.add(i);
            let mem = dict.cast::<u8>().add((*field).ptr_off);
            let field_type: ObjectType = std::mem::transmute((*field).key_type);
            match field_type {
                ObjectType::kObjectTypeNil => api_luarefs_free_object(*mem.cast::<Object>()),
                ObjectType::kObjectTypeLuaRef => api_free_luaref(*mem.cast::<LuaRef>()),
                ObjectType::kObjectTypeDict => api_luarefs_free_dict(*mem.cast::<Dict>()),
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_luarefs_free_array(value: Array) {
    unsafe {
        for i in 0..value.size {
            api_luarefs_free_object(*value.items.add(i));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_luarefs_free_dict(value: Dict) {
    unsafe {
        for i in 0..value.size {
            api_luarefs_free_object((*value.items.add(i)).value);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_mark(
    mut buf: *mut buf_T,
    name: NvimString,
    line: Integer,
    mut col: Integer,
    err: *mut Error,
) -> bool {
    unsafe {
        if buf.is_null() {
            buf = curbuf;
        }
        let mut deleting = false;
        if line == 0 {
            col = 0;
            deleting = true;
        } else {
            if col > MAXCOL {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"'column' value out of range".as_ptr(),
                );
                return false;
            }
            if line < 1 || line > i64::from((*buf).b_ml.ml_line_count) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"'line' value out of range".as_ptr(),
                );
                return false;
            }
        }
        let mut pos = pos_T {
            lnum: line as linenr_T,
            col: col as colnr_T,
            coladd: 0,
        };
        let res = setmark_pos(
            *name.data as c_int,
            &raw mut pos,
            (*buf).b_fnum,
            ptr::null_mut(),
        ) != 0;
        if !res {
            if deleting {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to delete named mark: %c".as_ptr(),
                    *name.data as c_int,
                );
            } else {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to set named mark: %c".as_ptr(),
                    *name.data as c_int,
                );
            }
        }
        res
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_default_stl_hl(
    wp: *mut win_T,
    use_winbar: bool,
    stc_hl_id: c_int,
) -> *const c_char {
    unsafe {
        if wp.is_null() {
            c"TabLineFill".as_ptr()
        } else if use_winbar {
            if wp == curwin {
                c"WinBar".as_ptr()
            } else {
                c"WinBarNC".as_ptr()
            }
        } else if stc_hl_id > 0 {
            syn_id2name(stc_hl_id)
        } else if wp == curwin {
            c"StatusLine".as_ptr()
        } else {
            c"StatusLineNC".as_ptr()
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_set_sctx(channel_id: u64) -> sctx_T {
    unsafe {
        let old_current_sctx = current_sctx;
        if channel_id != VIML_INTERNAL_CALL {
            current_sctx.sc_lnum = 0;
            if channel_id == LUA_INTERNAL_CALL {
                if !script_is_lua(current_sctx.sc_sid) {
                    current_sctx.sc_sid = SID_LUA;
                }
            } else {
                current_sctx.sc_sid = SID_API_CLIENT;
                current_sctx.sc_chan = channel_id;
            }
        }
        old_current_sctx
    }
}

static mut MEM_FOR_METADATA: ArenaMem = ptr::null_mut();
static mut METADATA_OBJ: Object = OBJECT_NIL;

unsafe extern "C" {
    fn api_get_packed_metadata(len: *mut usize) -> *const u8;
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_metadata() -> Object {
    unsafe {
        if METADATA_OBJ.obj_type == ObjectType::kObjectTypeNil {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            let mut len: usize = 0;
            let ptr = api_get_packed_metadata(&mut len).cast::<c_char>();
            METADATA_OBJ = unpack(ptr, len, &mut arena, &mut err);
            if error_set(&err) || METADATA_OBJ.obj_type != ObjectType::kObjectTypeDict {
                std::process::abort();
            }
            MEM_FOR_METADATA = arena_finish(&mut arena);
        }
        METADATA_OBJ
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_metadata_raw() -> NvimString {
    unsafe {
        let mut len: usize = 0;
        let ptr = api_get_packed_metadata(&mut len)
            .cast::<c_char>()
            .cast_mut();
        NvimString {
            data: ptr,
            size: len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_typename_and_cstr_helpers_work() {
        unsafe {
            assert_eq!(
                CStr::from_ptr(api_typename(ObjectType::kObjectTypeString))
                    .to_str()
                    .unwrap(),
                "String"
            );
            assert_eq!(
                CStr::from_ptr(api_typename(ObjectType::kObjectTypeLuaRef))
                    .to_str()
                    .unwrap(),
                "Function"
            );
            let s = cstr_to_string(c"hello".as_ptr());
            assert_eq!(s.size, 5);
            let cstr = string_to_cstr(s);
            assert_eq!(CStr::from_ptr(cstr).to_str().unwrap(), "hello");
            xfree(cstr.cast());
            api_free_string(s);
        }
    }
}
