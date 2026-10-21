//! Private API helper functions (`src/nvim/api/private/helpers.c`).

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
    api_err_exp,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::buffer::buf_T;
use crate::cursor::{colnr_T, linenr_T, pos_T};
use crate::eval::typval::{
    dict_T, dictitem_T, tv_clear, tv_copy, tv_dict_add, tv_dict_find, tv_dict_item_alloc_len,
    tv_dict_item_remove, tv_dict_watcher_notify, typval_T,
};
use crate::eval::vars::MAXCOL;
use crate::eval::vars::{before_set_vvar, get_vimvar_dict, scid_T};
use crate::ex_docmd::CAR;
use crate::ex_eval::{
    ET_ERROR, discard_current_exception, except_T, free_global_msglist, get_exception_string,
    msglist_T,
};
use crate::garray::garray_T;
use crate::globals::*;
use crate::highlight_group::{highlight_num_groups, syn_check_group, syn_id2name};
use crate::lua::converter::{Boolean, FieldHashfn, Float, KeySetLink, LuaRef, OptKeySet};
use crate::lua::executor::{api_free_luaref, api_new_luaref};

use crate::mark::setmark_pos;
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{
    Arena, ArenaMem, arena_alloc, arena_finish, arena_memdupz, memchrsub, xfree, xmemdupz, xstrdup,
    xstrndup, xstrnlen,
};
use crate::message::hl_msg_free;
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::unpacker::unpack;
use crate::runtime::{script_is_lua, sctx_T};
use crate::window::{tabpage_T, win_T};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

pub(crate) const MAXLNUM: i64 = 0x7fffffff;

pub(crate) const DI_FLAGS_RO: u8 = 1;
pub(crate) const DI_FLAGS_FIX: u8 = 4;
pub(crate) const DI_FLAGS_LOCK: u8 = 8;

pub(crate) const HLF_E: c_int = 6;

pub(crate) const SID_API_CLIENT: scid_T = -9;
pub(crate) const SID_LUA: scid_T = -8;

pub const OBJECT_NIL: Object = Object {
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
pub const fn cbuf_as_string(data: *mut c_char, size: usize) -> NvimString {
    NvimString { data, size }
}

#[inline]
pub const fn static_cstr_as_string(s: &'static CStr) -> NvimString {
    NvimString {
        data: s.as_ptr().cast_mut(),
        size: s.to_bytes().len(),
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

#[inline]
pub unsafe fn array_add(a: &mut Array, item: Object) {
    unsafe {
        a.push(item);
    }
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

use crate::api::vim::HlMessage;

pub unsafe fn api_set_error_str(err: *mut Error, err_type: ErrorType, msg: *const c_char) {
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
        {
            let len = $crate::strings::vim_snprintf!(std::ptr::null_mut(), 0, fmt_ptr, $($arg),+);
            let bufsize = ((len.max(0) as usize) + 1).min(1024 * 1024);
            let buf = $crate::memory::xmalloc(bufsize).cast::<std::ffi::c_char>();
            $crate::strings::vim_snprintf!(buf, bufsize, fmt_ptr, $($arg),+);
            (*err_ptr).msg = buf;
            (*err_ptr).err_type = et;
        }
    }};
}
pub use api_set_error;

#[inline]
pub const fn has_key(mask: u64, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
pub const fn static_cstr_as_obj(s: &'static CStr) -> Object {
    string_obj(static_cstr_as_string(s))
}

#[inline]
pub unsafe fn cstr_to_obj(s: *const c_char) -> Object {
    unsafe { string_obj(cstr_to_string(s)) }
}

#[inline]
pub unsafe fn cstr_to_arena_obj(arena: *mut Arena, s: *const c_char) -> Object {
    unsafe { string_obj(arena_string(arena, cstr_as_string(s))) }
}

#[inline]
pub unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = zeroed();
        try_enter(&raw mut tstate);
        f();
        try_leave(&raw const tstate, err);
    }
}

#[inline]
pub unsafe fn with_script_context<F: FnOnce()>(channel_id: u64, f: F) {
    unsafe {
        let save_current_sctx = api_set_sctx(channel_id);
        f();
        current_sctx = save_current_sctx;
    }
}

#[inline]
pub const fn error_init() -> Error {
    Error {
        err_type: ErrorType::kErrorTypeNone,
        msg: ptr::null_mut(),
    }
}

#[inline]
pub unsafe fn api_clear_string(s: *mut NvimString) {
    unsafe {
        xfree((*s).data.cast());
        (*s).data = ptr::null_mut();
        (*s).size = 0;
    }
}

#[inline]
pub unsafe fn tv_dict_is_watched(d: *const dict_T) -> bool {
    unsafe { !d.is_null() && (*d).watchers[0].cast_const() != (&raw const (*d).watchers).cast() }
}

use crate::map::pmap_get_int;

pub trait IntoDictKey {
    unsafe fn into_dict_key(self) -> NvimString;
}

impl IntoDictKey for *const c_char {
    #[inline]
    unsafe fn into_dict_key(self) -> NvimString {
        unsafe { cstr_as_string(self) }
    }
}

impl IntoDictKey for *mut c_char {
    #[inline]
    unsafe fn into_dict_key(self) -> NvimString {
        unsafe { cstr_as_string(self) }
    }
}

impl IntoDictKey for &'static CStr {
    #[inline]
    unsafe fn into_dict_key(self) -> NvimString {
        static_cstr_as_string(self)
    }
}

#[inline]
pub unsafe fn put_c<K: IntoDictKey>(dict: &mut Dict, key: K, val: Object) {
    debug_assert!(!dict.items.is_null() && dict.size < dict.capacity);
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: key.into_dict_key(),
            value: val,
        };
    }
    dict.size += 1;
}

#[inline]
pub unsafe fn cbuf_to_arena_str(arena: *mut Arena, buf: *const c_char, len: usize) -> NvimString {
    let ptr = if buf.is_null() { c"".as_ptr() } else { buf };
    unsafe {
        NvimString {
            data: arena_memdupz(arena, ptr, len),
            size: len,
        }
    }
}

#[inline]
pub unsafe fn error_set(e: *const Error) -> bool {
    unsafe { !matches!((*e).err_type, ErrorType::kErrorTypeNone) }
}

/// Start block that may cause Vimscript exceptions while evaluating another code (`try_enter`).
pub unsafe fn try_enter(tstate: *mut TryState) {
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
pub unsafe fn try_leave(tstate: *const TryState, err: *mut Error) {
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
pub unsafe fn dict_get_value(
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

pub unsafe fn dict_check_writable(
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
pub unsafe fn dict_set_var(
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

pub unsafe fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T {
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

#[unsafe(no_mangle)]
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

pub unsafe fn find_tab_by_handle(tabpage: Tabpage, err: *mut Error) -> *mut tabpage_T {
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

pub unsafe fn cstr_to_string(str_: *const c_char) -> NvimString {
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

pub unsafe fn string_to_cstr(str_: NvimString) -> *mut c_char {
    unsafe { xstrndup(str_.data, str_.size) }
}

pub unsafe fn cbuf_to_string(buf: *const c_char, size: usize) -> NvimString {
    unsafe {
        NvimString {
            data: xmemdupz(buf.cast(), size).cast(),
            size,
        }
    }
}

pub unsafe fn cstrn_to_string(str_: *const c_char, maxsize: usize) -> NvimString {
    unsafe { cbuf_to_string(str_, xstrnlen(str_, maxsize)) }
}

pub unsafe fn cstrn_as_string(str_: *const c_char, maxsize: usize) -> NvimString {
    unsafe {
        if str_.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        NvimString {
            data: str_.cast_mut(),
            size: xstrnlen(str_, maxsize),
        }
    }
}

pub unsafe fn cstr_as_string(str_: *const c_char) -> NvimString {
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

pub unsafe fn ga_take_string(ga: *mut garray_T) -> NvimString {
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

pub unsafe fn string_to_array(input: NvimString, crlf: bool, arena: *mut Arena) -> Array {
    unsafe {
        let mut ret: ArrayBuilder = zeroed();
        ret.init();

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
            ret.push(Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData { string: s },
            });
            if i + 1 == input.size && (*end == NL || (crlf && *end == CAR)) {
                let empty_s = arena_string(
                    arena,
                    NvimString {
                        data: c"".as_ptr().cast_mut(),
                        size: 0,
                    },
                );
                ret.push(Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData { string: empty_s },
                });
            }
            i += 1;
        }

        arena_take_arraybuilder(arena, &raw mut ret)
    }
}

pub unsafe fn normalize_index(
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

pub unsafe fn buf_get_text(
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

pub unsafe fn api_free_string(value: NvimString) {
    unsafe {
        xfree(value.data.cast());
    }
}

pub unsafe fn arena_array(arena: *mut Arena, max_size: usize) -> Array {
    unsafe {
        Array {
            size: 0,
            capacity: max_size,
            items: arena_alloc(arena, size_of::<Object>() * max_size, true).cast(),
        }
    }
}

pub unsafe fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict {
    unsafe {
        Dict {
            size: 0,
            capacity: max_size,
            items: arena_alloc(arena, size_of::<KeyValuePair>() * max_size, true).cast(),
        }
    }
}

pub unsafe fn arena_string(arena: *mut Arena, str_: NvimString) -> NvimString {
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

pub unsafe fn arena_take_arraybuilder(arena: *mut Arena, arr: *mut ArrayBuilder) -> Array {
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

#[unsafe(no_mangle)]
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

pub unsafe fn api_free_array(value: Array) {
    unsafe {
        for i in 0..value.size {
            api_free_object(*value.items.add(i));
        }
        xfree(value.items.cast());
    }
}

pub unsafe fn api_free_dict(value: Dict) {
    unsafe {
        for i in 0..value.size {
            let item = *value.items.add(i);
            api_free_string(item.key);
            api_free_object(item.value);
        }
        xfree(value.items.cast());
    }
}

pub unsafe fn api_clear_error(value: *mut Error) {
    unsafe {
        if !error_set(value) {
            return;
        }
        xfree((*value).msg.cast());
        (*value).msg = ptr::null_mut();
        (*value).err_type = ErrorType::kErrorTypeNone;
    }
}

pub unsafe fn copy_string(str_: NvimString, arena: *mut Arena) -> NvimString {
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

pub unsafe fn copy_array(array: Array, arena: *mut Arena) -> Array {
    unsafe {
        let mut rv = arena_array(arena, array.size);
        for i in 0..array.size {
            *rv.items.add(i) = copy_object(*array.items.add(i), arena);
        }
        rv.size = array.size;
        rv
    }
}

pub unsafe fn copy_dict(dict: Dict, arena: *mut Arena) -> Dict {
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

pub unsafe fn copy_object(obj: Object, arena: *mut Arena) -> Object {
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

pub unsafe fn api_object_to_bool(
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

pub unsafe fn object_to_hl_id(obj: Object, what: *const c_char, err: *mut Error) -> c_int {
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

pub fn api_typename(t: ObjectType) -> *const c_char {
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

pub unsafe fn parse_hl_msg(chunks: Array, is_err: bool, err: *mut Error) -> HlMessage {
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

pub unsafe fn api_dict_to_keydict(
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

pub unsafe fn api_keydict_to_dict(
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

pub unsafe fn api_luarefs_free_object(value: Object) {
    unsafe {
        match value.obj_type {
            ObjectType::kObjectTypeLuaRef => api_free_luaref(value.data.luaref),
            ObjectType::kObjectTypeArray => api_luarefs_free_array(value.data.array),
            ObjectType::kObjectTypeDict => api_luarefs_free_dict(value.data.dict),
            _ => {}
        }
    }
}

pub unsafe fn api_luarefs_free_keydict(dict: *mut c_void, table: *mut KeySetLink) {
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

pub unsafe fn api_luarefs_free_array(value: Array) {
    unsafe {
        for i in 0..value.size {
            api_luarefs_free_object(*value.items.add(i));
        }
    }
}

pub unsafe fn api_luarefs_free_dict(value: Dict) {
    unsafe {
        for i in 0..value.size {
            api_luarefs_free_object((*value.items.add(i)).value);
        }
    }
}

pub unsafe fn set_mark(
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

pub unsafe fn get_default_stl_hl(
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

pub unsafe fn api_set_sctx(channel_id: u64) -> sctx_T {
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

struct ApiFunctionMeta {
    name: &'static str,
    method: bool,
    since: u32,
    deprecated_since: Option<u32>,
    return_type: &'static str,
    parameters: &'static [(&'static str, &'static str, bool)],
}

const fn api_fn(
    name: &'static str,
    method: bool,
    since: u32,
    deprecated_since: Option<u32>,
    return_type: &'static str,
    parameters: &'static [(&'static str, &'static str, bool)],
) -> ApiFunctionMeta {
    ApiFunctionMeta {
        name,
        method,
        since,
        deprecated_since,
        return_type,
        parameters,
    }
}

struct ApiUiEventMeta {
    name: &'static str,
    since: u32,
    parameters: &'static [(&'static str, &'static str)],
}

const fn api_ui_ev(
    name: &'static str,
    since: u32,
    parameters: &'static [(&'static str, &'static str)],
) -> ApiUiEventMeta {
    ApiUiEventMeta {
        name,
        since,
        parameters,
    }
}

static API_FUNCTIONS: [ApiFunctionMeta; 263] = [
    api_fn(
        "nvim_get_autocmds",
        false,
        9,
        None,
        "ArrayOf(Dict)",
        &[("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_create_autocmd",
        false,
        9,
        None,
        "Integer",
        &[("Object", "event", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_del_autocmd",
        false,
        9,
        None,
        "void",
        &[("Integer", "id", false)],
    ),
    api_fn(
        "nvim_clear_autocmds",
        false,
        9,
        None,
        "void",
        &[("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_create_augroup",
        false,
        9,
        None,
        "Integer",
        &[("String", "name", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_del_augroup_by_id",
        false,
        9,
        None,
        "void",
        &[("Integer", "id", false)],
    ),
    api_fn(
        "nvim_del_augroup_by_name",
        false,
        9,
        None,
        "void",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_exec_autocmds",
        false,
        9,
        None,
        "void",
        &[("Object", "event", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_buf_line_count",
        true,
        1,
        None,
        "Integer",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_buf_attach",
        true,
        4,
        None,
        "Boolean",
        &[
            ("Buffer", "buf", false),
            ("Boolean", "send_buffer", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_detach",
        true,
        4,
        None,
        "Boolean",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_buf_get_lines",
        true,
        1,
        None,
        "ArrayOf(String)",
        &[
            ("Buffer", "buf", false),
            ("Integer", "start", false),
            ("Integer", "end", false),
            ("Boolean", "strict_indexing", false),
        ],
    ),
    api_fn(
        "nvim_buf_set_lines",
        true,
        1,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("Integer", "start", false),
            ("Integer", "end", false),
            ("Boolean", "strict_indexing", false),
            ("ArrayOf(String)", "replacement", false),
        ],
    ),
    api_fn(
        "nvim_buf_set_text",
        true,
        7,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("Integer", "start_row", false),
            ("Integer", "start_col", false),
            ("Integer", "end_row", false),
            ("Integer", "end_col", false),
            ("ArrayOf(String)", "replacement", false),
        ],
    ),
    api_fn(
        "nvim_buf_get_text",
        true,
        9,
        None,
        "ArrayOf(String)",
        &[
            ("Buffer", "buf", false),
            ("Integer", "start_row", false),
            ("Integer", "start_col", false),
            ("Integer", "end_row", false),
            ("Integer", "end_col", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_get_offset",
        true,
        5,
        None,
        "Integer",
        &[("Buffer", "buf", false), ("Integer", "index", false)],
    ),
    api_fn(
        "nvim_buf_get_var",
        true,
        1,
        None,
        "Object",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_get_changedtick",
        true,
        2,
        None,
        "Integer",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_buf_get_keymap",
        true,
        3,
        None,
        "ArrayOf(Dict)",
        &[("Buffer", "buf", false), ("String", "mode", false)],
    ),
    api_fn(
        "nvim_buf_set_keymap",
        true,
        6,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("String", "mode", false),
            ("String", "lhs", false),
            ("String", "rhs", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_del_keymap",
        true,
        6,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("String", "mode", false),
            ("String", "lhs", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_set_var",
        true,
        1,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "nvim_buf_del_var",
        true,
        1,
        None,
        "void",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_get_name",
        true,
        1,
        None,
        "String",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_buf_set_name",
        true,
        1,
        None,
        "void",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_is_loaded",
        true,
        5,
        None,
        "Boolean",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_buf_delete",
        true,
        7,
        None,
        "void",
        &[("Buffer", "buf", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_buf_is_valid",
        true,
        1,
        None,
        "Boolean",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_buf_del_mark",
        true,
        8,
        None,
        "Boolean",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_set_mark",
        true,
        8,
        None,
        "Boolean",
        &[
            ("Buffer", "buf", false),
            ("String", "name", false),
            ("Integer", "line", false),
            ("Integer", "col", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_get_mark",
        true,
        1,
        None,
        "ArrayOf(Integer, 2)",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_call",
        true,
        7,
        None,
        "Object",
        &[("Buffer", "buf", false), ("LuaRef", "fn", false)],
    ),
    api_fn(
        "nvim_parse_cmd",
        false,
        10,
        None,
        "Dict",
        &[("String", "str", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_cmd",
        false,
        10,
        None,
        "String",
        &[("Dict", "cmd", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_create_user_command",
        false,
        9,
        None,
        "void",
        &[
            ("String", "name", false),
            ("Object", "cmd", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_del_user_command",
        false,
        9,
        None,
        "void",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_create_user_command",
        true,
        9,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("String", "name", false),
            ("Object", "cmd", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_del_user_command",
        true,
        9,
        None,
        "void",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_get_commands",
        false,
        4,
        None,
        "Dict",
        &[("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_buf_get_commands",
        true,
        4,
        None,
        "Dict",
        &[("Buffer", "buf", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_exec",
        false,
        7,
        Some(11),
        "String",
        &[("String", "src", false), ("Boolean", "output", false)],
    ),
    api_fn(
        "nvim_command_output",
        false,
        1,
        Some(7),
        "String",
        &[("String", "command", false)],
    ),
    api_fn(
        "nvim_execute_lua",
        false,
        3,
        Some(7),
        "Object",
        &[("String", "code", false), ("Array", "args", false)],
    ),
    api_fn(
        "nvim_buf_get_number",
        true,
        1,
        Some(2),
        "Integer",
        &[("Buffer", "buffer", false)],
    ),
    api_fn(
        "nvim_buf_clear_highlight",
        true,
        1,
        Some(7),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "ns_id", false),
            ("Integer", "line_start", false),
            ("Integer", "line_end", false),
        ],
    ),
    api_fn(
        "nvim_buf_add_highlight",
        true,
        1,
        Some(13),
        "Integer",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "ns_id", false),
            ("String", "hl_group", false),
            ("Integer", "line", false),
            ("Integer", "col_start", false),
            ("Integer", "col_end", false),
        ],
    ),
    api_fn(
        "nvim_buf_set_virtual_text",
        true,
        5,
        Some(8),
        "Integer",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "src_id", false),
            ("Integer", "line", false),
            ("Array", "chunks", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_get_hl_by_id",
        false,
        3,
        Some(9),
        "Dict",
        &[("Integer", "hl_id", false), ("Boolean", "rgb", false)],
    ),
    api_fn(
        "nvim_get_hl_by_name",
        false,
        3,
        Some(9),
        "Dict",
        &[("String", "name", false), ("Boolean", "rgb", false)],
    ),
    api_fn(
        "buffer_insert",
        false,
        0,
        Some(1),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "lnum", false),
            ("ArrayOf(String)", "lines", false),
        ],
    ),
    api_fn(
        "buffer_get_line",
        false,
        0,
        Some(1),
        "String",
        &[("Buffer", "buffer", false), ("Integer", "index", false)],
    ),
    api_fn(
        "buffer_set_line",
        false,
        0,
        Some(1),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "index", false),
            ("String", "line", false),
        ],
    ),
    api_fn(
        "buffer_del_line",
        false,
        0,
        Some(1),
        "void",
        &[("Buffer", "buffer", false), ("Integer", "index", false)],
    ),
    api_fn(
        "buffer_get_line_slice",
        false,
        0,
        Some(1),
        "ArrayOf(String)",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "start", false),
            ("Integer", "end", false),
            ("Boolean", "include_start", false),
            ("Boolean", "include_end", false),
        ],
    ),
    api_fn(
        "buffer_set_line_slice",
        false,
        0,
        Some(1),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "start", false),
            ("Integer", "end", false),
            ("Boolean", "include_start", false),
            ("Boolean", "include_end", false),
            ("ArrayOf(String)", "replacement", false),
        ],
    ),
    api_fn(
        "buffer_set_var",
        false,
        0,
        Some(1),
        "Object",
        &[
            ("Buffer", "buffer", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "buffer_del_var",
        false,
        0,
        Some(1),
        "Object",
        &[("Buffer", "buffer", false), ("String", "name", false)],
    ),
    api_fn(
        "window_set_var",
        false,
        0,
        Some(1),
        "Object",
        &[
            ("Window", "window", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "window_del_var",
        false,
        0,
        Some(1),
        "Object",
        &[("Window", "window", false), ("String", "name", false)],
    ),
    api_fn(
        "tabpage_set_var",
        false,
        0,
        Some(1),
        "Object",
        &[
            ("Tabpage", "tabpage", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "tabpage_del_var",
        false,
        0,
        Some(1),
        "Object",
        &[("Tabpage", "tabpage", false), ("String", "name", false)],
    ),
    api_fn(
        "vim_set_var",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "name", false), ("Object", "value", false)],
    ),
    api_fn(
        "vim_del_var",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_get_option_info",
        false,
        7,
        Some(11),
        "Dict",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_set_option",
        false,
        1,
        Some(11),
        "void",
        &[("String", "name", false), ("Object", "value", false)],
    ),
    api_fn(
        "nvim_get_option",
        false,
        1,
        Some(11),
        "Object",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_get_option",
        true,
        1,
        Some(11),
        "Object",
        &[("Buffer", "buffer", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_buf_set_option",
        true,
        1,
        Some(11),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "nvim_win_get_option",
        true,
        1,
        Some(11),
        "Object",
        &[("Window", "window", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_win_set_option",
        true,
        1,
        Some(11),
        "void",
        &[
            ("Window", "window", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "nvim_call_atomic",
        false,
        1,
        Some(12),
        "Array",
        &[("Array", "calls", false)],
    ),
    api_fn(
        "nvim_subscribe",
        false,
        1,
        Some(13),
        "void",
        &[("String", "event", false)],
    ),
    api_fn(
        "nvim_unsubscribe",
        false,
        1,
        Some(13),
        "void",
        &[("String", "event", false)],
    ),
    api_fn(
        "nvim_out_write",
        false,
        1,
        Some(13),
        "void",
        &[("String", "str", false)],
    ),
    api_fn(
        "nvim_err_write",
        false,
        1,
        Some(13),
        "void",
        &[("String", "str", false)],
    ),
    api_fn(
        "nvim_err_writeln",
        false,
        1,
        Some(13),
        "void",
        &[("String", "str", false)],
    ),
    api_fn(
        "nvim_notify",
        false,
        7,
        Some(13),
        "Object",
        &[
            ("String", "msg", false),
            ("Integer", "log_level", false),
            ("Dict", "opts", false),
        ],
    ),
    api_fn(
        "nvim_win_set_height",
        true,
        1,
        Some(15),
        "void",
        &[("Window", "win", false), ("Integer", "height", false)],
    ),
    api_fn(
        "nvim_win_set_width",
        true,
        1,
        Some(15),
        "void",
        &[("Window", "win", false), ("Integer", "width", false)],
    ),
    api_fn(
        "nvim_ui_term_event",
        false,
        12,
        None,
        "void",
        &[("String", "event", false), ("Object", "value", false)],
    ),
    api_fn(
        "nvim_create_namespace",
        false,
        5,
        None,
        "Integer",
        &[("String", "name", false)],
    ),
    api_fn("nvim_get_namespaces", false, 5, None, "Dict", &[]),
    api_fn(
        "nvim_buf_get_extmark_by_id",
        true,
        7,
        None,
        "Array",
        &[
            ("Buffer", "buf", false),
            ("Integer", "ns_id", false),
            ("Integer", "id", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_get_extmarks",
        true,
        7,
        None,
        "ArrayOf(Dict)",
        &[
            ("Buffer", "buf", false),
            ("Integer", "ns_id", false),
            ("Object", "start", false),
            ("Object", "end", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_set_extmark",
        true,
        7,
        None,
        "Integer",
        &[
            ("Buffer", "buf", false),
            ("Integer", "ns_id", false),
            ("Integer", "line", false),
            ("Integer", "col", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_buf_del_extmark",
        true,
        7,
        None,
        "Boolean",
        &[
            ("Buffer", "buf", false),
            ("Integer", "ns_id", false),
            ("Integer", "id", false),
        ],
    ),
    api_fn(
        "nvim_buf_clear_namespace",
        true,
        5,
        None,
        "void",
        &[
            ("Buffer", "buf", false),
            ("Integer", "ns_id", false),
            ("Integer", "line_start", false),
            ("Integer", "line_end", false),
        ],
    ),
    api_fn(
        "nvim_set_decoration_provider",
        false,
        7,
        None,
        "void",
        &[("Integer", "ns_id", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_get_option_value",
        false,
        9,
        None,
        "Object",
        &[("String", "name", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_set_option_value",
        false,
        9,
        None,
        "Object",
        &[
            ("String", "name", false),
            ("Object", "value", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn("nvim_get_all_options_info", false, 7, None, "Dict", &[]),
    api_fn(
        "nvim_get_option_info2",
        false,
        11,
        None,
        "Dict",
        &[("String", "name", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_tabpage_list_wins",
        true,
        1,
        None,
        "ArrayOf(Window)",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "nvim_tabpage_get_var",
        true,
        1,
        None,
        "Object",
        &[("Tabpage", "tabpage", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_tabpage_set_var",
        true,
        1,
        None,
        "void",
        &[
            ("Tabpage", "tabpage", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "nvim_tabpage_del_var",
        true,
        1,
        None,
        "void",
        &[("Tabpage", "tabpage", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_tabpage_get_win",
        true,
        1,
        None,
        "Window",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "nvim_tabpage_set_win",
        true,
        12,
        None,
        "void",
        &[("Tabpage", "tabpage", false), ("Window", "win", false)],
    ),
    api_fn(
        "nvim_tabpage_get_number",
        true,
        1,
        None,
        "Integer",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "nvim_tabpage_is_valid",
        true,
        1,
        None,
        "Boolean",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "nvim_open_tabpage",
        false,
        14,
        None,
        "Tabpage",
        &[
            ("Buffer", "buf", false),
            ("Boolean", "enter", false),
            ("Dict", "config", false),
        ],
    ),
    api_fn(
        "nvim_ui_attach",
        false,
        1,
        None,
        "void",
        &[
            ("Integer", "width", false),
            ("Integer", "height", false),
            ("Dict", "options", false),
        ],
    ),
    api_fn(
        "ui_attach",
        false,
        0,
        Some(1),
        "void",
        &[
            ("Integer", "width", false),
            ("Integer", "height", false),
            ("Boolean", "enable_rgb", false),
        ],
    ),
    api_fn(
        "nvim_ui_set_focus",
        false,
        11,
        None,
        "void",
        &[("Boolean", "gained", false)],
    ),
    api_fn("nvim_ui_detach", false, 1, None, "void", &[]),
    api_fn(
        "nvim_ui_try_resize",
        false,
        1,
        None,
        "void",
        &[("Integer", "width", false), ("Integer", "height", false)],
    ),
    api_fn(
        "nvim_ui_set_option",
        false,
        1,
        None,
        "void",
        &[("String", "name", false), ("Object", "value", false)],
    ),
    api_fn(
        "nvim_ui_try_resize_grid",
        false,
        6,
        None,
        "void",
        &[
            ("Integer", "grid", false),
            ("Integer", "width", false),
            ("Integer", "height", false),
        ],
    ),
    api_fn(
        "nvim_ui_pum_set_height",
        false,
        6,
        None,
        "void",
        &[("Integer", "height", false)],
    ),
    api_fn(
        "nvim_ui_pum_set_bounds",
        false,
        7,
        None,
        "void",
        &[
            ("Float", "width", false),
            ("Float", "height", false),
            ("Float", "row", false),
            ("Float", "col", false),
        ],
    ),
    api_fn(
        "nvim_ui_send",
        false,
        14,
        None,
        "void",
        &[("String", "content", false)],
    ),
    api_fn(
        "nvim_get_hl_id_by_name",
        false,
        7,
        None,
        "Integer",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_get_hl",
        false,
        11,
        None,
        "Dict",
        &[("Integer", "ns_id", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_set_hl",
        false,
        7,
        None,
        "void",
        &[
            ("Integer", "ns_id", false),
            ("String", "name", false),
            ("Dict", "val", false),
        ],
    ),
    api_fn(
        "nvim_get_hl_ns",
        false,
        12,
        None,
        "Integer",
        &[("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_set_hl_ns",
        false,
        10,
        None,
        "void",
        &[("Integer", "ns_id", false)],
    ),
    api_fn(
        "nvim_set_hl_ns_fast",
        false,
        10,
        None,
        "void",
        &[("Integer", "ns_id", false)],
    ),
    api_fn(
        "nvim_feedkeys",
        false,
        1,
        None,
        "void",
        &[
            ("String", "keys", false),
            ("String", "mode", false),
            ("Boolean", "escape_ks", false),
        ],
    ),
    api_fn(
        "nvim_input",
        false,
        1,
        None,
        "Integer",
        &[("String", "keys", false)],
    ),
    api_fn(
        "nvim_input_mouse",
        false,
        6,
        None,
        "void",
        &[
            ("String", "button", false),
            ("String", "action", false),
            ("String", "modifier", false),
            ("Integer", "grid", false),
            ("Integer", "row", false),
            ("Integer", "col", false),
        ],
    ),
    api_fn(
        "nvim_replace_termcodes",
        false,
        1,
        None,
        "String",
        &[
            ("String", "str", false),
            ("Boolean", "from_part", false),
            ("Boolean", "do_lt", false),
            ("Boolean", "special", false),
        ],
    ),
    api_fn(
        "nvim_exec_lua",
        false,
        7,
        None,
        "Object",
        &[("String", "code", false), ("Array", "args", false)],
    ),
    api_fn(
        "nvim_strwidth",
        false,
        1,
        None,
        "Integer",
        &[("String", "text", false)],
    ),
    api_fn(
        "nvim_list_runtime_paths",
        false,
        1,
        None,
        "ArrayOf(String)",
        &[],
    ),
    api_fn(
        "nvim_get_runtime_file",
        false,
        7,
        None,
        "ArrayOf(String)",
        &[("String", "name", false), ("Boolean", "all", false)],
    ),
    api_fn(
        "nvim_set_current_dir",
        false,
        1,
        None,
        "void",
        &[("String", "dir", false)],
    ),
    api_fn("nvim_get_current_line", false, 1, None, "String", &[]),
    api_fn(
        "nvim_set_current_line",
        false,
        1,
        None,
        "void",
        &[("String", "line", false)],
    ),
    api_fn("nvim_del_current_line", false, 1, None, "void", &[]),
    api_fn(
        "nvim_get_var",
        false,
        1,
        None,
        "Object",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_set_var",
        false,
        1,
        None,
        "void",
        &[("String", "name", false), ("Object", "value", false)],
    ),
    api_fn(
        "nvim_del_var",
        false,
        1,
        None,
        "void",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_get_vvar",
        false,
        1,
        None,
        "Object",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_set_vvar",
        false,
        6,
        None,
        "void",
        &[("String", "name", false), ("Object", "value", false)],
    ),
    api_fn(
        "nvim_echo",
        false,
        7,
        None,
        "Object",
        &[
            ("ArrayOf(Array)", "chunks", false),
            ("Boolean", "history", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn("nvim_list_bufs", false, 1, None, "ArrayOf(Buffer)", &[]),
    api_fn("nvim_get_current_buf", false, 1, None, "Buffer", &[]),
    api_fn(
        "nvim_set_current_buf",
        false,
        1,
        None,
        "void",
        &[("Buffer", "buf", false)],
    ),
    api_fn("nvim_list_wins", false, 1, None, "ArrayOf(Window)", &[]),
    api_fn("nvim_get_current_win", false, 1, None, "Window", &[]),
    api_fn(
        "nvim_set_current_win",
        false,
        1,
        None,
        "void",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_create_buf",
        false,
        6,
        None,
        "Buffer",
        &[("Boolean", "listed", false), ("Boolean", "scratch", false)],
    ),
    api_fn(
        "nvim_open_term",
        false,
        7,
        None,
        "Integer",
        &[("Buffer", "buf", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_chan_send",
        false,
        7,
        None,
        "void",
        &[("Integer", "chan", false), ("String", "data", false)],
    ),
    api_fn(
        "nvim_list_tabpages",
        false,
        1,
        None,
        "ArrayOf(Tabpage)",
        &[],
    ),
    api_fn("nvim_get_current_tabpage", false, 1, None, "Tabpage", &[]),
    api_fn(
        "nvim_set_current_tabpage",
        false,
        1,
        None,
        "void",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "nvim_paste",
        false,
        6,
        None,
        "Boolean",
        &[
            ("String", "data", false),
            ("Boolean", "crlf", false),
            ("Integer", "phase", false),
        ],
    ),
    api_fn(
        "nvim_put",
        false,
        6,
        None,
        "void",
        &[
            ("ArrayOf(String)", "lines", false),
            ("String", "type", false),
            ("Boolean", "after", false),
            ("Boolean", "follow", false),
        ],
    ),
    api_fn(
        "nvim_get_color_by_name",
        false,
        1,
        None,
        "Integer",
        &[("String", "name", false)],
    ),
    api_fn("nvim_get_color_map", false, 1, None, "Dict", &[]),
    api_fn(
        "nvim_get_context",
        false,
        6,
        None,
        "Dict",
        &[("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_load_context",
        false,
        6,
        None,
        "Object",
        &[("Dict", "dict", false)],
    ),
    api_fn("nvim_get_mode", false, 2, None, "Dict", &[]),
    api_fn(
        "nvim_get_keymap",
        false,
        3,
        None,
        "ArrayOf(Dict)",
        &[("String", "mode", false)],
    ),
    api_fn(
        "nvim_set_keymap",
        false,
        6,
        None,
        "void",
        &[
            ("String", "mode", false),
            ("String", "lhs", false),
            ("String", "rhs", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_del_keymap",
        false,
        6,
        None,
        "void",
        &[
            ("String", "mode", false),
            ("String", "lhs", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_get_api_info",
        false,
        1,
        None,
        "ArrayOf(Object, 2)",
        &[],
    ),
    api_fn(
        "nvim_set_client_info",
        false,
        4,
        None,
        "void",
        &[
            ("String", "name", false),
            ("Dict", "version", false),
            ("String", "type", false),
            ("Dict", "methods", false),
            ("Dict", "attributes", false),
        ],
    ),
    api_fn(
        "nvim_chan_set",
        false,
        15,
        None,
        "void",
        &[("Integer", "chan", false), ("Dict", "config", false)],
    ),
    api_fn(
        "nvim_get_chan_info",
        false,
        4,
        None,
        "Dict",
        &[("Integer", "chan", false)],
    ),
    api_fn("nvim_list_chans", false, 4, None, "ArrayOf(Dict)", &[]),
    api_fn("nvim_list_uis", false, 4, None, "ArrayOf(Dict)", &[]),
    api_fn(
        "nvim_get_proc_children",
        false,
        4,
        None,
        "Array",
        &[("Integer", "pid", false)],
    ),
    api_fn(
        "nvim_get_proc",
        false,
        4,
        None,
        "Object",
        &[("Integer", "pid", false)],
    ),
    api_fn(
        "nvim_select_popupmenu_item",
        false,
        6,
        None,
        "void",
        &[
            ("Integer", "item", false),
            ("Boolean", "insert", false),
            ("Boolean", "finish", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "nvim_del_mark",
        false,
        8,
        None,
        "Boolean",
        &[("String", "name", false)],
    ),
    api_fn(
        "nvim_get_mark",
        false,
        8,
        None,
        "Array",
        &[("String", "name", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_eval_statusline",
        false,
        8,
        None,
        "Dict",
        &[("String", "str", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_exec2",
        false,
        11,
        None,
        "Dict",
        &[("String", "src", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_command",
        false,
        1,
        None,
        "void",
        &[("String", "cmd", false)],
    ),
    api_fn(
        "nvim_eval",
        false,
        1,
        None,
        "Object",
        &[("String", "expr", false)],
    ),
    api_fn(
        "nvim_call_function",
        false,
        1,
        None,
        "Object",
        &[("String", "fn", false), ("Array", "args", false)],
    ),
    api_fn(
        "nvim_call_dict_function",
        false,
        4,
        None,
        "Object",
        &[
            ("Object", "dict", false),
            ("String", "fn", false),
            ("Array", "args", false),
        ],
    ),
    api_fn(
        "nvim_parse_expression",
        false,
        4,
        None,
        "Dict",
        &[
            ("String", "expr", false),
            ("String", "flags", false),
            ("Boolean", "hl", false),
        ],
    ),
    api_fn(
        "nvim_open_win",
        false,
        6,
        None,
        "Window",
        &[
            ("Buffer", "buf", false),
            ("Boolean", "enter", false),
            ("Dict", "config", false),
        ],
    ),
    api_fn(
        "nvim_win_set_config",
        true,
        6,
        None,
        "void",
        &[("Window", "win", false), ("Dict", "config", false)],
    ),
    api_fn(
        "nvim_win_get_config",
        true,
        6,
        None,
        "Dict",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_get_buf",
        true,
        1,
        None,
        "Buffer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_set_buf",
        true,
        5,
        None,
        "void",
        &[("Window", "win", false), ("Buffer", "buf", false)],
    ),
    api_fn(
        "nvim_win_get_cursor",
        true,
        1,
        None,
        "ArrayOf(Integer, 2)",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_set_cursor",
        true,
        1,
        None,
        "void",
        &[
            ("Window", "win", false),
            ("ArrayOf(Integer, 2)", "pos", false),
        ],
    ),
    api_fn(
        "nvim_win_get_height",
        true,
        1,
        None,
        "Integer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_get_width",
        true,
        1,
        None,
        "Integer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_get_var",
        true,
        1,
        None,
        "Object",
        &[("Window", "win", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_win_set_var",
        true,
        1,
        None,
        "void",
        &[
            ("Window", "win", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "nvim_win_del_var",
        true,
        1,
        None,
        "void",
        &[("Window", "win", false), ("String", "name", false)],
    ),
    api_fn(
        "nvim_win_get_position",
        true,
        1,
        None,
        "ArrayOf(Integer, 2)",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_get_tabpage",
        true,
        1,
        None,
        "Tabpage",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_get_number",
        true,
        1,
        None,
        "Integer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_is_valid",
        true,
        1,
        None,
        "Boolean",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_hide",
        true,
        7,
        None,
        "void",
        &[("Window", "win", false)],
    ),
    api_fn(
        "nvim_win_close",
        true,
        6,
        None,
        "void",
        &[("Window", "win", false), ("Boolean", "force", false)],
    ),
    api_fn(
        "nvim_win_call",
        true,
        7,
        None,
        "Object",
        &[("Window", "win", false), ("LuaRef", "fn", false)],
    ),
    api_fn(
        "nvim_win_set_hl_ns",
        true,
        10,
        None,
        "void",
        &[("Window", "win", false), ("Integer", "ns_id", false)],
    ),
    api_fn(
        "nvim_win_text_height",
        true,
        12,
        None,
        "Dict",
        &[("Window", "win", false), ("Dict", "opts", true)],
    ),
    api_fn(
        "nvim_win_resize",
        true,
        15,
        None,
        "void",
        &[
            ("Window", "win", false),
            ("Integer", "width", false),
            ("Integer", "height", false),
            ("Dict", "opts", true),
        ],
    ),
    api_fn(
        "buffer_line_count",
        true,
        0,
        Some(1),
        "Integer",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "buffer_get_lines",
        true,
        0,
        Some(1),
        "ArrayOf(String)",
        &[
            ("Buffer", "buf", false),
            ("Integer", "start", false),
            ("Integer", "end", false),
            ("Boolean", "strict_indexing", false),
        ],
    ),
    api_fn(
        "buffer_set_lines",
        true,
        0,
        Some(1),
        "void",
        &[
            ("Buffer", "buf", false),
            ("Integer", "start", false),
            ("Integer", "end", false),
            ("Boolean", "strict_indexing", false),
            ("ArrayOf(String)", "replacement", false),
        ],
    ),
    api_fn(
        "buffer_get_var",
        true,
        0,
        Some(1),
        "Object",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "buffer_get_name",
        true,
        0,
        Some(1),
        "String",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "buffer_set_name",
        true,
        0,
        Some(1),
        "void",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "buffer_is_valid",
        true,
        0,
        Some(1),
        "Boolean",
        &[("Buffer", "buf", false)],
    ),
    api_fn(
        "buffer_get_mark",
        true,
        0,
        Some(1),
        "ArrayOf(Integer, 2)",
        &[("Buffer", "buf", false), ("String", "name", false)],
    ),
    api_fn(
        "vim_command_output",
        false,
        0,
        Some(1),
        "String",
        &[("String", "command", false)],
    ),
    api_fn(
        "buffer_get_number",
        true,
        0,
        Some(1),
        "Integer",
        &[("Buffer", "buffer", false)],
    ),
    api_fn(
        "buffer_clear_highlight",
        true,
        0,
        Some(1),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "ns_id", false),
            ("Integer", "line_start", false),
            ("Integer", "line_end", false),
        ],
    ),
    api_fn(
        "buffer_add_highlight",
        true,
        0,
        Some(1),
        "Integer",
        &[
            ("Buffer", "buffer", false),
            ("Integer", "ns_id", false),
            ("String", "hl_group", false),
            ("Integer", "line", false),
            ("Integer", "col_start", false),
            ("Integer", "col_end", false),
        ],
    ),
    api_fn(
        "vim_set_option",
        false,
        0,
        Some(1),
        "void",
        &[("String", "name", false), ("Object", "value", false)],
    ),
    api_fn(
        "vim_get_option",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "name", false)],
    ),
    api_fn(
        "buffer_get_option",
        true,
        0,
        Some(1),
        "Object",
        &[("Buffer", "buffer", false), ("String", "name", false)],
    ),
    api_fn(
        "buffer_set_option",
        true,
        0,
        Some(1),
        "void",
        &[
            ("Buffer", "buffer", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "window_get_option",
        true,
        0,
        Some(1),
        "Object",
        &[("Window", "window", false), ("String", "name", false)],
    ),
    api_fn(
        "window_set_option",
        true,
        0,
        Some(1),
        "void",
        &[
            ("Window", "window", false),
            ("String", "name", false),
            ("Object", "value", false),
        ],
    ),
    api_fn(
        "vim_subscribe",
        false,
        0,
        Some(1),
        "void",
        &[("String", "event", false)],
    ),
    api_fn(
        "vim_unsubscribe",
        false,
        0,
        Some(1),
        "void",
        &[("String", "event", false)],
    ),
    api_fn(
        "vim_out_write",
        false,
        0,
        Some(1),
        "void",
        &[("String", "str", false)],
    ),
    api_fn(
        "vim_err_write",
        false,
        0,
        Some(1),
        "void",
        &[("String", "str", false)],
    ),
    api_fn(
        "vim_report_error",
        false,
        0,
        Some(1),
        "void",
        &[("String", "str", false)],
    ),
    api_fn(
        "window_set_height",
        true,
        0,
        Some(1),
        "void",
        &[("Window", "win", false), ("Integer", "height", false)],
    ),
    api_fn(
        "window_set_width",
        true,
        0,
        Some(1),
        "void",
        &[("Window", "win", false), ("Integer", "width", false)],
    ),
    api_fn(
        "tabpage_get_windows",
        true,
        0,
        Some(1),
        "ArrayOf(Window)",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "tabpage_get_var",
        true,
        0,
        Some(1),
        "Object",
        &[("Tabpage", "tabpage", false), ("String", "name", false)],
    ),
    api_fn(
        "tabpage_get_window",
        true,
        0,
        Some(1),
        "Window",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "tabpage_is_valid",
        true,
        0,
        Some(1),
        "Boolean",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn("ui_detach", false, 0, Some(1), "void", &[]),
    api_fn(
        "ui_try_resize",
        false,
        0,
        Some(1),
        "Object",
        &[("Integer", "width", false), ("Integer", "height", false)],
    ),
    api_fn(
        "vim_feedkeys",
        false,
        0,
        Some(1),
        "void",
        &[
            ("String", "keys", false),
            ("String", "mode", false),
            ("Boolean", "escape_ks", false),
        ],
    ),
    api_fn(
        "vim_input",
        false,
        0,
        Some(1),
        "Integer",
        &[("String", "keys", false)],
    ),
    api_fn(
        "vim_replace_termcodes",
        false,
        0,
        Some(1),
        "String",
        &[
            ("String", "str", false),
            ("Boolean", "from_part", false),
            ("Boolean", "do_lt", false),
            ("Boolean", "special", false),
        ],
    ),
    api_fn(
        "vim_strwidth",
        false,
        0,
        Some(1),
        "Integer",
        &[("String", "text", false)],
    ),
    api_fn(
        "vim_list_runtime_paths",
        false,
        0,
        Some(1),
        "ArrayOf(String)",
        &[],
    ),
    api_fn(
        "vim_change_directory",
        false,
        0,
        Some(1),
        "void",
        &[("String", "dir", false)],
    ),
    api_fn("vim_get_current_line", false, 0, Some(1), "String", &[]),
    api_fn(
        "vim_set_current_line",
        false,
        0,
        Some(1),
        "void",
        &[("String", "line", false)],
    ),
    api_fn("vim_del_current_line", false, 0, Some(1), "void", &[]),
    api_fn(
        "vim_get_var",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "name", false)],
    ),
    api_fn(
        "vim_get_vvar",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "name", false)],
    ),
    api_fn("vim_get_buffers", false, 0, Some(1), "ArrayOf(Buffer)", &[]),
    api_fn("vim_get_current_buffer", false, 0, Some(1), "Buffer", &[]),
    api_fn(
        "vim_set_current_buffer",
        false,
        0,
        Some(1),
        "void",
        &[("Buffer", "buf", false)],
    ),
    api_fn("vim_get_windows", false, 0, Some(1), "ArrayOf(Window)", &[]),
    api_fn("vim_get_current_window", false, 0, Some(1), "Window", &[]),
    api_fn(
        "vim_set_current_window",
        false,
        0,
        Some(1),
        "void",
        &[("Window", "win", false)],
    ),
    api_fn(
        "vim_get_tabpages",
        false,
        0,
        Some(1),
        "ArrayOf(Tabpage)",
        &[],
    ),
    api_fn("vim_get_current_tabpage", false, 0, Some(1), "Tabpage", &[]),
    api_fn(
        "vim_set_current_tabpage",
        false,
        0,
        Some(1),
        "void",
        &[("Tabpage", "tabpage", false)],
    ),
    api_fn(
        "vim_name_to_color",
        false,
        0,
        Some(1),
        "Integer",
        &[("String", "name", false)],
    ),
    api_fn("vim_get_color_map", false, 0, Some(1), "Dict", &[]),
    api_fn(
        "vim_get_api_info",
        false,
        0,
        Some(1),
        "ArrayOf(Object, 2)",
        &[],
    ),
    api_fn(
        "vim_command",
        false,
        0,
        Some(1),
        "void",
        &[("String", "cmd", false)],
    ),
    api_fn(
        "vim_eval",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "expr", false)],
    ),
    api_fn(
        "vim_call_function",
        false,
        0,
        Some(1),
        "Object",
        &[("String", "fn", false), ("Array", "args", false)],
    ),
    api_fn(
        "window_get_buffer",
        true,
        0,
        Some(1),
        "Buffer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "window_get_cursor",
        true,
        0,
        Some(1),
        "ArrayOf(Integer, 2)",
        &[("Window", "win", false)],
    ),
    api_fn(
        "window_set_cursor",
        true,
        0,
        Some(1),
        "void",
        &[
            ("Window", "win", false),
            ("ArrayOf(Integer, 2)", "pos", false),
        ],
    ),
    api_fn(
        "window_get_height",
        true,
        0,
        Some(1),
        "Integer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "window_get_width",
        true,
        0,
        Some(1),
        "Integer",
        &[("Window", "win", false)],
    ),
    api_fn(
        "window_get_var",
        true,
        0,
        Some(1),
        "Object",
        &[("Window", "win", false), ("String", "name", false)],
    ),
    api_fn(
        "window_get_position",
        true,
        0,
        Some(1),
        "ArrayOf(Integer, 2)",
        &[("Window", "win", false)],
    ),
    api_fn(
        "window_get_tabpage",
        true,
        0,
        Some(1),
        "Tabpage",
        &[("Window", "win", false)],
    ),
    api_fn(
        "window_is_valid",
        true,
        0,
        Some(1),
        "Boolean",
        &[("Window", "win", false)],
    ),
];

static API_UI_EVENTS: [ApiUiEventMeta; 69] = [
    api_ui_ev(
        "mode_info_set",
        3,
        &[("Boolean", "enabled"), ("Array", "cursor_styles")],
    ),
    api_ui_ev("update_menu", 3, &[]),
    api_ui_ev("busy_start", 3, &[]),
    api_ui_ev("busy_stop", 3, &[]),
    api_ui_ev("mouse_on", 3, &[]),
    api_ui_ev("mouse_off", 3, &[]),
    api_ui_ev(
        "mode_change",
        3,
        &[("String", "mode"), ("Integer", "mode_idx")],
    ),
    api_ui_ev("bell", 3, &[]),
    api_ui_ev("visual_bell", 3, &[]),
    api_ui_ev("flush", 3, &[]),
    api_ui_ev("connect", 14, &[("String", "server_addr")]),
    api_ui_ev(
        "restart",
        14,
        &[("String", "listen_addr"), ("Dictionary", "chan_config")],
    ),
    api_ui_ev("suspend", 3, &[]),
    api_ui_ev("set_title", 3, &[("String", "title")]),
    api_ui_ev("set_icon", 3, &[("String", "icon")]),
    api_ui_ev("screenshot", 7, &[("String", "path")]),
    api_ui_ev("option_set", 4, &[("String", "name"), ("Object", "value")]),
    api_ui_ev("chdir", 12, &[("String", "path")]),
    api_ui_ev("ui_send", 14, &[("String", "content")]),
    api_ui_ev("update_fg", 3, &[("Integer", "fg")]),
    api_ui_ev("update_bg", 3, &[("Integer", "bg")]),
    api_ui_ev("update_sp", 3, &[("Integer", "sp")]),
    api_ui_ev("resize", 3, &[("Integer", "width"), ("Integer", "height")]),
    api_ui_ev("clear", 3, &[]),
    api_ui_ev("eol_clear", 3, &[]),
    api_ui_ev("cursor_goto", 3, &[("Integer", "row"), ("Integer", "col")]),
    api_ui_ev("highlight_set", 3, &[("Dictionary", "attrs")]),
    api_ui_ev("put", 3, &[("String", "str")]),
    api_ui_ev(
        "set_scroll_region",
        3,
        &[
            ("Integer", "top"),
            ("Integer", "bot"),
            ("Integer", "left"),
            ("Integer", "right"),
        ],
    ),
    api_ui_ev("scroll", 3, &[("Integer", "count")]),
    api_ui_ev(
        "default_colors_set",
        4,
        &[
            ("Integer", "rgb_fg"),
            ("Integer", "rgb_bg"),
            ("Integer", "rgb_sp"),
            ("Integer", "cterm_fg"),
            ("Integer", "cterm_bg"),
        ],
    ),
    api_ui_ev(
        "hl_attr_define",
        5,
        &[
            ("Integer", "id"),
            ("Dictionary", "rgb_attrs"),
            ("Dictionary", "cterm_attrs"),
            ("Array", "info"),
        ],
    ),
    api_ui_ev("hl_group_set", 6, &[("String", "name"), ("Integer", "id")]),
    api_ui_ev(
        "grid_resize",
        5,
        &[
            ("Integer", "grid"),
            ("Integer", "width"),
            ("Integer", "height"),
        ],
    ),
    api_ui_ev("grid_clear", 5, &[("Integer", "grid")]),
    api_ui_ev(
        "grid_cursor_goto",
        5,
        &[("Integer", "grid"), ("Integer", "row"), ("Integer", "col")],
    ),
    api_ui_ev(
        "grid_line",
        5,
        &[
            ("Integer", "grid"),
            ("Integer", "row"),
            ("Integer", "col_start"),
            ("Array", "data"),
            ("Boolean", "wrap"),
        ],
    ),
    api_ui_ev(
        "grid_scroll",
        5,
        &[
            ("Integer", "grid"),
            ("Integer", "top"),
            ("Integer", "bot"),
            ("Integer", "left"),
            ("Integer", "right"),
            ("Integer", "rows"),
            ("Integer", "cols"),
        ],
    ),
    api_ui_ev("grid_destroy", 6, &[("Integer", "grid")]),
    api_ui_ev(
        "win_pos",
        6,
        &[
            ("Integer", "grid"),
            ("Window", "win"),
            ("Integer", "startrow"),
            ("Integer", "startcol"),
            ("Integer", "width"),
            ("Integer", "height"),
        ],
    ),
    api_ui_ev(
        "win_float_pos",
        6,
        &[
            ("Integer", "grid"),
            ("Window", "win"),
            ("String", "anchor"),
            ("Integer", "anchor_grid"),
            ("Float", "anchor_row"),
            ("Float", "anchor_col"),
            ("Boolean", "mouse_enabled"),
            ("Integer", "zindex"),
            ("Integer", "compindex"),
            ("Integer", "screen_row"),
            ("Integer", "screen_col"),
        ],
    ),
    api_ui_ev(
        "win_external_pos",
        6,
        &[("Integer", "grid"), ("Window", "win")],
    ),
    api_ui_ev("win_hide", 6, &[("Integer", "grid")]),
    api_ui_ev("win_close", 6, &[("Integer", "grid")]),
    api_ui_ev(
        "msg_set_pos",
        6,
        &[
            ("Integer", "grid"),
            ("Integer", "row"),
            ("Boolean", "scrolled"),
            ("String", "sep_char"),
            ("Integer", "zindex"),
            ("Integer", "compindex"),
        ],
    ),
    api_ui_ev(
        "win_viewport",
        7,
        &[
            ("Integer", "grid"),
            ("Window", "win"),
            ("Integer", "topline"),
            ("Integer", "botline"),
            ("Integer", "curline"),
            ("Integer", "curcol"),
            ("Integer", "line_count"),
            ("Integer", "scroll_delta"),
        ],
    ),
    api_ui_ev(
        "win_viewport_margins",
        12,
        &[
            ("Integer", "grid"),
            ("Window", "win"),
            ("Integer", "top"),
            ("Integer", "bottom"),
            ("Integer", "left"),
            ("Integer", "right"),
        ],
    ),
    api_ui_ev(
        "win_extmark",
        10,
        &[
            ("Integer", "grid"),
            ("Window", "win"),
            ("Integer", "ns_id"),
            ("Integer", "mark_id"),
            ("Integer", "row"),
            ("Integer", "col"),
        ],
    ),
    api_ui_ev(
        "popupmenu_show",
        3,
        &[
            ("Array", "items"),
            ("Integer", "selected"),
            ("Integer", "row"),
            ("Integer", "col"),
            ("Integer", "grid"),
        ],
    ),
    api_ui_ev("popupmenu_hide", 3, &[]),
    api_ui_ev("popupmenu_select", 3, &[("Integer", "selected")]),
    api_ui_ev(
        "tabline_update",
        3,
        &[
            ("Tabpage", "current"),
            ("Array", "tabs"),
            ("Buffer", "current_buffer"),
            ("Array", "buffers"),
        ],
    ),
    api_ui_ev(
        "cmdline_show",
        3,
        &[
            ("Array", "content"),
            ("Integer", "pos"),
            ("String", "firstc"),
            ("String", "prompt"),
            ("Integer", "indent"),
            ("Integer", "level"),
            ("Integer", "hl_id"),
        ],
    ),
    api_ui_ev(
        "cmdline_pos",
        3,
        &[("Integer", "pos"), ("Integer", "level")],
    ),
    api_ui_ev(
        "cmdline_special_char",
        3,
        &[("String", "c"), ("Boolean", "shift"), ("Integer", "level")],
    ),
    api_ui_ev(
        "cmdline_hide",
        3,
        &[("Integer", "level"), ("Boolean", "abort")],
    ),
    api_ui_ev("cmdline_block_show", 3, &[("Array", "lines")]),
    api_ui_ev("cmdline_block_append", 3, &[("Array", "lines")]),
    api_ui_ev("cmdline_block_hide", 3, &[]),
    api_ui_ev("wildmenu_show", 3, &[("Array", "items")]),
    api_ui_ev("wildmenu_select", 3, &[("Integer", "selected")]),
    api_ui_ev("wildmenu_hide", 3, &[]),
    api_ui_ev(
        "msg_show",
        6,
        &[
            ("String", "kind"),
            ("Array", "content"),
            ("Boolean", "replace_last"),
            ("Boolean", "history"),
            ("Boolean", "append"),
            ("Object", "id"),
            ("String", "trigger"),
        ],
    ),
    api_ui_ev("msg_clear", 6, &[]),
    api_ui_ev("msg_showcmd", 6, &[("Array", "content")]),
    api_ui_ev("msg_showmode", 6, &[("Array", "content")]),
    api_ui_ev("msg_ruler", 6, &[("Array", "content")]),
    api_ui_ev(
        "msg_history_show",
        6,
        &[("Array", "entries"), ("Boolean", "prev_cmd")],
    ),
    api_ui_ev("error_exit", 12, &[("Integer", "status")]),
];

static API_UI_OPTIONS: [&str; 10] = [
    "rgb",
    "ext_cmdline",
    "ext_popupmenu",
    "ext_tabline",
    "ext_wildmenu",
    "ext_messages",
    "ext_linegrid",
    "ext_multigrid",
    "ext_hlstate",
    "ext_termcolors",
];

static API_ERROR_TYPES: [(&str, u32); 2] = [("Exception", 0), ("Validation", 1)];

static API_TYPES: [(&str, u32, &str); 3] = [
    ("Buffer", 0, "nvim_buf_"),
    ("Window", 1, "nvim_win_"),
    ("Tabpage", 2, "nvim_tabpage_"),
];

use crate::api::buffer::{LUA_INTERNAL_CALL, VIML_INTERNAL_CALL};
use crate::charset::{NL, NUL};
use crate::version::{
    nvim_api_level, nvim_api_level_compat, nvim_api_prerelease, nvim_version_build,
    nvim_version_major, nvim_version_minor, nvim_version_patch, nvim_version_prerelease,
};

static PACKED_API_METADATA: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();

#[inline]
fn mpack_push_str(out: &mut Vec<u8>, s: &[u8]) {
    if s.len() <= 31 {
        out.push(0xa0 | (s.len() as u8));
    } else if s.len() <= u8::MAX as usize {
        out.push(0xd9);
        out.push(s.len() as u8);
    } else {
        out.push(0xda);
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(s);
}

#[inline]
fn mpack_push_uint(out: &mut Vec<u8>, val: u32) {
    if val <= 0x7f {
        out.push(val as u8);
    } else if val <= u8::MAX as u32 {
        out.push(0xcc);
        out.push(val as u8);
    } else if val <= u16::MAX as u32 {
        out.push(0xcd);
        out.extend_from_slice(&(val as u16).to_be_bytes());
    } else {
        out.push(0xce);
        out.extend_from_slice(&val.to_be_bytes());
    }
}

#[inline]
fn mpack_push_bool(out: &mut Vec<u8>, val: bool) {
    out.push(if val { 0xc3 } else { 0xc2 });
}

#[inline]
fn mpack_push_array(out: &mut Vec<u8>, len: usize) {
    if len <= 15 {
        out.push(0x90 | (len as u8));
    } else if len <= u16::MAX as usize {
        out.push(0xdc);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0xdd);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

pub unsafe fn api_get_packed_metadata(len: *mut usize) -> *const u8 {
    let bytes = PACKED_API_METADATA.get_or_init(|| {
        let mut out = Vec::with_capacity(32768);
        out.push(0x86); // fixmap(6)
        mpack_push_str(&mut out, b"version");
        out.push(0x88); // fixmap(8)
        mpack_push_str(&mut out, b"major");
        mpack_push_uint(&mut out, nvim_version_major as u32);
        mpack_push_str(&mut out, b"minor");
        mpack_push_uint(&mut out, nvim_version_minor as u32);
        mpack_push_str(&mut out, b"patch");
        mpack_push_uint(&mut out, nvim_version_patch as u32);
        mpack_push_str(&mut out, b"prerelease");
        mpack_push_bool(&mut out, nvim_version_prerelease);
        mpack_push_str(&mut out, b"api_level");
        mpack_push_uint(&mut out, nvim_api_level as u32);
        mpack_push_str(&mut out, b"api_compatible");
        mpack_push_uint(&mut out, nvim_api_level_compat as u32);
        mpack_push_str(&mut out, b"api_prerelease");
        mpack_push_bool(&mut out, nvim_api_prerelease);
        mpack_push_str(&mut out, b"build");
        let build_ptr = unsafe { nvim_version_build };
        if build_ptr.is_null() {
            out.push(0xc0);
        } else {
            let b = unsafe { CStr::from_ptr(build_ptr) }.to_bytes();
            mpack_push_str(&mut out, b);
        }

        mpack_push_str(&mut out, b"functions");
        mpack_push_array(&mut out, API_FUNCTIONS.len());
        for f in &API_FUNCTIONS {
            out.push(if f.deprecated_since.is_some() {
                0x86
            } else {
                0x85
            });
            mpack_push_str(&mut out, b"method");
            mpack_push_bool(&mut out, f.method);
            if let Some(dep) = f.deprecated_since {
                mpack_push_str(&mut out, b"deprecated_since");
                mpack_push_uint(&mut out, dep);
            }
            mpack_push_str(&mut out, b"parameters");
            mpack_push_array(&mut out, f.parameters.len());
            for &(pt, pn, po) in f.parameters {
                out.push(0x93);
                mpack_push_str(&mut out, pt.as_bytes());
                mpack_push_str(&mut out, pn.as_bytes());
                mpack_push_bool(&mut out, po);
            }
            mpack_push_str(&mut out, b"since");
            mpack_push_uint(&mut out, f.since);
            mpack_push_str(&mut out, b"return_type");
            mpack_push_str(&mut out, f.return_type.as_bytes());
            mpack_push_str(&mut out, b"name");
            mpack_push_str(&mut out, f.name.as_bytes());
        }

        mpack_push_str(&mut out, b"ui_events");
        mpack_push_array(&mut out, API_UI_EVENTS.len());
        for ev in &API_UI_EVENTS {
            out.push(0x83);
            mpack_push_str(&mut out, b"since");
            mpack_push_uint(&mut out, ev.since);
            mpack_push_str(&mut out, b"name");
            mpack_push_str(&mut out, ev.name.as_bytes());
            mpack_push_str(&mut out, b"parameters");
            mpack_push_array(&mut out, ev.parameters.len());
            for &(pt, pn) in ev.parameters {
                out.push(0x92);
                mpack_push_str(&mut out, pt.as_bytes());
                mpack_push_str(&mut out, pn.as_bytes());
            }
        }

        mpack_push_str(&mut out, b"ui_options");
        mpack_push_array(&mut out, API_UI_OPTIONS.len());
        for &opt in &API_UI_OPTIONS {
            mpack_push_str(&mut out, opt.as_bytes());
        }

        mpack_push_str(&mut out, b"error_types");
        out.push(0x80 | (API_ERROR_TYPES.len() as u8));
        for &(ename, eid) in &API_ERROR_TYPES {
            mpack_push_str(&mut out, ename.as_bytes());
            out.push(0x81);
            mpack_push_str(&mut out, b"id");
            mpack_push_uint(&mut out, eid);
        }

        mpack_push_str(&mut out, b"types");
        out.push(0x80 | (API_TYPES.len() as u8));
        for &(tname, tid, tprefix) in &API_TYPES {
            mpack_push_str(&mut out, tname.as_bytes());
            out.push(0x82);
            mpack_push_str(&mut out, b"id");
            mpack_push_uint(&mut out, tid);
            mpack_push_str(&mut out, b"prefix");
            mpack_push_str(&mut out, tprefix.as_bytes());
        }

        out.into_boxed_slice()
    });
    if !len.is_null() {
        unsafe {
            *len = bytes.len();
        }
    }
    bytes.as_ptr()
}

pub unsafe fn api_metadata() -> Object {
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

pub unsafe fn api_metadata_raw() -> NvimString {
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

            let raw = api_metadata_raw();
            assert!(!raw.data.is_null());
            assert!(raw.size > 30_000);
            let meta = api_metadata();
            assert_eq!(meta.obj_type, ObjectType::kObjectTypeDict);
            assert_eq!(meta.data.dict.size, 6);
        }
    }
}
