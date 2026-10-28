//! Private API helper functions (`src/nvim/api/private/helpers.c`).

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
    api_err_exp,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::buffer::FileBuf;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::eval::typval::{
    Dictitem, TvDict, TypVal, tv_clear, tv_copy, tv_dict_add, tv_dict_find, tv_dict_item_alloc_len,
    tv_dict_item_remove, tv_dict_watcher_notify,
};
use crate::eval::vars::MAXCOL;
use crate::eval::vars::{Scid, before_set_vvar, get_vimvar_dict};
use crate::ex_docmd::CAR;
use crate::ex_eval::{
    Except, Msglist, discard_current_exception, free_global_msglist, get_error_exception_string,
};
use crate::globals::*;
use crate::highlight_group::{highlight_num_groups, syn_check_group, syn_id2name};
use crate::lua::converter::{Boolean, FieldHashfn, Float, KeySetLink, LuaRef, OptKeySet};
use crate::lua::executor::{api_free_luaref, api_new_luaref};

use crate::mark::setmark_pos;
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{
    Arena, ArenaMem, alloc_cstring, alloc_cstring_len, arena_alloc, arena_alloc_opt, arena_finish,
    arena_memdupz, arena_memdupz_opt, free_cstring, free_items, xstrnlen,
};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::msgpack_rpc::unpacker::unpack;
use crate::runtime::{Sctx, script_is_lua};
use crate::window::{TabPageInfo, WinInfoInfo};
use std::ffi::{CStr, c_char, c_int};
use std::mem::size_of;
use std::ptr;

pub(crate) const MAXLNUM: i64 = 0x7fffffff;

pub(crate) const DI_FLAGS_RO: u8 = 1;
pub(crate) const DI_FLAGS_FIX: u8 = 4;
pub(crate) const DI_FLAGS_LOCK: u8 = 8;

pub(crate) const HLF_E: c_int = 6;

pub(crate) const SID_API_CLIENT: Scid = -9;
pub(crate) const SID_LUA: Scid = -8;

pub const OBJECT_NIL: Object = Object {
    obj_type: ObjectType::ObjectTypeNil,
    data: ObjectData::Integer(0),
};

#[inline]
pub const fn nil_obj() -> Object {
    OBJECT_NIL
}

#[inline]
pub const fn boolean_obj(b: Boolean) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeBoolean,
        data: ObjectData::Boolean(b),
    }
}

#[inline]
pub const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeInteger,
        data: ObjectData::Integer(i),
    }
}

#[inline]
pub const fn float_obj(f: Float) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeFloat,
        data: ObjectData::Floating(f),
    }
}

#[inline]
pub const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeString,
        data: ObjectData::String(s),
    }
}

#[inline]
pub const fn buffer_obj(b: Buffer) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeBuffer,
        data: ObjectData::Integer(b as Integer),
    }
}

#[inline]
pub const fn window_obj(w: Window) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeWindow,
        data: ObjectData::Integer(w as Integer),
    }
}

#[inline]
pub const fn tabpage_obj(t: Tabpage) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeTabpage,
        data: ObjectData::Integer(t as Integer),
    }
}

#[inline]
pub const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeArray,
        data: ObjectData::Array(a),
    }
}

#[inline]
pub const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeDict,
        data: ObjectData::Dict(d),
    }
}

#[inline]
pub const fn luaref_obj(r: LuaRef) -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeLuaRef,
        data: ObjectData::LuaRef(r),
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
pub fn cstr_as_obj<S: crate::strings::IntoCCharPtr>(s: S) -> Object {
    string_obj(cstr_as_string(s.into_c_char_ptr()))
}

#[inline]
pub fn add_c(array: &mut Array, item: Object) {
    debug_assert!(!array.items.is_null() && array.size < array.capacity);
    unsafe {
        *array.items.add(array.size) = item;
    }
    array.size += 1;
}

#[inline]
pub fn array_add(a: &mut Array, item: Object) {
    a.push(item);
}

#[repr(C)]
pub struct TryState {
    pub current_exception: Option<std::rc::Rc<Except>>,
    pub msg_list: Option<Vec<Msglist>>,
    pub got_int: c_int,
    pub did_throw: bool,
    pub need_rethrow: c_int,
    pub did_emsg: c_int,
}

impl TryState {
    pub const fn new() -> Self {
        Self {
            current_exception: None,
            msg_list: None,
            got_int: 0,
            did_throw: false,
            need_rethrow: 0,
            did_emsg: 0,
        }
    }
}

pub type ArrayBuilder = Vec<Object>;

#[repr(C)]
pub struct HlMessageChunk {
    pub text: NvimString,
    pub hl_id: c_int,
}

impl Drop for HlMessageChunk {
    fn drop(&mut self) {
        api_free_string(self.text);
    }
}

use crate::api::vim::HlMessage;

pub fn api_set_error_str<S: crate::strings::IntoCCharPtr>(
    err: &mut Error,
    err_type: ErrorType,
    msg: S,
) {
    debug_assert!(!matches!(err_type, ErrorType::ErrorTypeNone));
    err.msg = Some(crate::memory::cstr_to_cstring(msg.into_c_char_ptr()));
    err.err_type = err_type;
}

#[macro_export]
macro_rules! api_set_error {
    ($err:expr, $err_type:expr, $fmt:expr $(,)?) => {{
        $crate::api::private::helpers::api_set_error_str($err, $err_type, $fmt);
    }};
    ($err:expr, $err_type:expr, $fmt:expr, $($arg:expr),+ $(,)?) => {{
        let err_ptr: *mut $crate::api::private::validate::Error = $err;
        let et: $crate::api::private::validate::ErrorType = $err_type;
        let fmt_ptr: *const std::ffi::c_char =
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
        {
            let len = $crate::strings::vim_snprintf!(std::ptr::null_mut(), 0, fmt_ptr, $($arg),+);
            let bufsize = ((len.max(0) as usize) + 1).min(1024 * 1024);
            let mut buf = vec![0 as std::ffi::c_char; bufsize];
            $crate::strings::vim_snprintf!(buf.as_mut_ptr(), bufsize, fmt_ptr, $($arg),+);
            (*err_ptr).msg = Some($crate::memory::cstr_to_cstring(buf.as_ptr()));
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
pub fn cstr_to_obj(s: impl crate::strings::IntoCCharPtr) -> Object {
    let s = s.into_c_char_ptr();
    string_obj(cstr_to_string(s))
}

#[inline]
pub fn cstr_to_arena_obj(arena: &mut Arena, s: impl crate::strings::IntoCCharPtr) -> Object {
    let s = s.into_c_char_ptr();
    string_obj(arena_string(arena, cstr_as_string(s)))
}

#[inline]
pub fn try_wrap<F: FnOnce()>(err: &mut Error, f: F) {
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    f();
    try_leave(&tstate, err);
}

#[inline]
pub fn try_wrap_err<F: FnOnce(&mut Error)>(err: &mut Error, f: F) {
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    f(err);
    try_leave(&tstate, err);
}

#[inline]
pub fn with_script_context<F: FnOnce()>(channel_id: u64, f: F) {
    let save_current_sctx = api_set_sctx(channel_id);
    f();
    g_st().current_sctx = save_current_sctx;
}

#[inline]
pub const fn error_init() -> Error {
    Error::new()
}

#[inline]
pub fn api_clear_string(s: &mut NvimString) {
    api_free_string(*s);
    s.data = ptr::null_mut();
    s.size = 0;
}

#[inline]
pub fn tv_dict_is_watched(d: Option<&TvDict>) -> bool {
    d.is_some() && !d.unwrap().watchers.is_empty()
}

pub trait IntoDictKey {
    fn into_dict_key(self) -> NvimString;
}

impl IntoDictKey for *const c_char {
    #[inline]
    fn into_dict_key(self) -> NvimString {
        cstr_as_string(self)
    }
}

impl IntoDictKey for *mut c_char {
    #[inline]
    fn into_dict_key(self) -> NvimString {
        cstr_as_string(self)
    }
}

impl IntoDictKey for &'static CStr {
    #[inline]
    fn into_dict_key(self) -> NvimString {
        static_cstr_as_string(self)
    }
}

#[inline]
pub fn put_c<K: IntoDictKey>(dict: &mut Dict, key: K, val: Object) {
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
pub fn cbuf_to_arena_str(
    arena: &mut Arena,
    buf: impl crate::strings::IntoCCharPtr,
    len: usize,
) -> NvimString {
    let buf = buf.into_c_char_ptr();
    let ptr = if buf.is_null() { c"".as_ptr() } else { buf };
    NvimString {
        data: arena_memdupz(arena, ptr, len),
        size: len,
    }
}

#[inline]
pub fn cbuf_to_arena_str_opt(
    arena: Option<&mut Arena>,
    buf: impl crate::strings::IntoCCharPtr,
    len: usize,
) -> NvimString {
    let buf = buf.into_c_char_ptr();
    let ptr = if buf.is_null() { c"".as_ptr() } else { buf };
    NvimString {
        data: arena_memdupz_opt(arena, ptr, len),
        size: len,
    }
}

#[inline]
pub fn error_set(e: &Error) -> bool {
    !matches!(e.err_type, ErrorType::ErrorTypeNone)
}

/// Start block that may cause Vimscript exceptions while evaluating another code (`try_enter`).
pub fn try_enter(tstate: &mut TryState) {
    unsafe {
        let saved_msg_list = g_st().msg_list.replace(Vec::new());
        let saved_excp = g_st().current_exception.take();
        ptr::write(
            tstate,
            TryState {
                current_exception: saved_excp,
                msg_list: saved_msg_list,
                got_int: c_int::from(g_st().got_int),
                did_throw: g_st().did_throw,
                need_rethrow: c_int::from(g_st().need_rethrow),
                did_emsg: g_st().did_emsg,
            },
        );
        g_st().got_int = false;
        g_st().did_throw = false;
        g_st().need_rethrow = false;
        g_st().did_emsg = 0;
        g_st().trylevel += 1;
    }
}

/// Ends a `try_enter` block; sets error message if any (`try_leave`).
pub fn try_leave(tstate: &TryState, err: &mut Error) {
    unsafe {
        debug_assert!(g_st().trylevel > 0);
        g_st().trylevel -= 1;

        g_st().did_emsg = 0;
        g_st().force_abort = false;

        let ml = &raw const g_st().msg_list;
        let ce = &raw const g_st().current_exception;
        if g_st().got_int {
            if g_st().did_throw {
                discard_current_exception();
            }
            api_set_error!(err, ErrorType::ErrorTypeException, c"Keyboard interrupt",);
            g_st().got_int = false;
        } else if let Some(ref msgs) = *ml
            && !msgs.is_empty()
        {
            let throw_ptr = msgs[msgs[0].throw_idx].msg.as_ptr().add(msgs[0].throw_off);
            let msg = get_error_exception_string(throw_ptr, ptr::null_mut());
            api_set_error!(err, ErrorType::ErrorTypeException, c"%s", msg.as_ptr());
            free_global_msglist();
        } else if g_st().did_throw || g_st().need_rethrow {
            if let Some(ref excp) = *ce {
                if !excp.throw_name.as_bytes().is_empty() {
                    if excp.throw_lnum != 0 {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"%s, line %d: %s",
                            excp.throw_name.as_ptr(),
                            excp.throw_lnum,
                            excp.value.as_ptr(),
                        );
                    } else {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"%s: %s",
                            excp.throw_name.as_ptr(),
                            excp.value.as_ptr(),
                        );
                    }
                } else {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"%s",
                        excp.value.as_ptr(),
                    );
                }
            }
            discard_current_exception();
        }

        g_st().msg_list = tstate.msg_list.clone();
        g_st().current_exception = tstate.current_exception.clone();
        g_st().got_int = tstate.got_int != 0;
        g_st().did_throw = tstate.did_throw;
        g_st().need_rethrow = tstate.need_rethrow != 0;
        g_st().did_emsg = tstate.did_emsg;
    }
}

/// Recursively expands a vimscript value in a dict (`dict_get_value`).
pub fn dict_get_value(
    dict: Option<&TvDict>,
    key: NvimString,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> Object {
    unsafe {
        let di = tv_dict_find(dict, key.data, key.size as isize);
        if di.is_null() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Key not found: %s",
                key.data,
            );
            return OBJECT_NIL;
        }
        let di = { &mut *di };
        vim_to_object(&raw mut di.di_tv, arena, true)
    }
}

pub fn dict_check_writable(
    dict: &mut TvDict,
    key: NvimString,
    del: bool,
    err: &mut Error,
) -> *mut Dictitem {
    unsafe {
        let di = tv_dict_find(Some(dict), key.data, key.size as isize);
        if !di.is_null() {
            if ((*di).di_flags & DI_FLAGS_RO) != 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Key is read-only: %s",
                    key.data,
                );
            } else if ((*di).di_flags & DI_FLAGS_LOCK) != 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Key is locked: %s",
                    key.data,
                );
            } else if del && ((*di).di_flags & DI_FLAGS_FIX) != 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Key is fixed: %s",
                    key.data,
                );
            }
        } else if dict.dv_lock != 0 {
            api_set_error!(err, ErrorType::ErrorTypeException, c"Dict is locked",);
        } else if key.size == 0 {
            api_set_error!(err, ErrorType::ErrorTypeValidation, c"Key name is empty",);
        } else if key.size > c_int::MAX as usize {
            api_set_error!(err, ErrorType::ErrorTypeValidation, c"Key name is too long",);
        }
        di
    }
}

/// Set a value in a scope dict (`dict_set_var`).
pub fn dict_set_var(
    dict: &mut TvDict,
    key: NvimString,
    value: Object,
    del: bool,
    retval: bool,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> Object {
    unsafe {
        let mut rv = OBJECT_NIL;
        let mut di = dict_check_writable(dict, key, del, err);
        if error_set(err) {
            return rv;
        }

        let watched = tv_dict_is_watched(Some(&*dict));
        if del {
            if di.is_null() {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Key not found: %s",
                    key.data,
                );
            } else {
                if watched {
                    tv_dict_watcher_notify(dict, key.data, None, Some(&(*di).di_tv));
                }
                if retval {
                    rv = vim_to_object(&raw mut (*di).di_tv, arena, false);
                }
                tv_dict_item_remove(&mut *dict, &mut *di);
            }
        } else {
            let mut tv: TypVal = TypVal::new();
            object_to_vim(value, &mut tv, err);
            let mut oldtv: TypVal = TypVal::new();

            if di.is_null() {
                di = tv_dict_item_alloc_len(key.data, key.size);
                tv_dict_add(dict, di);
            } else {
                if retval {
                    rv = vim_to_object(&raw mut (*di).di_tv, arena, false);
                }
                let mut type_error = false;
                if ptr::eq(dict, get_vimvar_dict())
                    && !before_set_vvar(key.data, &mut *di, &mut tv, true, watched, &mut type_error)
                {
                    tv_clear(&mut tv);
                    if type_error {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeValidation,
                            c"Setting v:%s to value with wrong type",
                            key.data,
                        );
                    }
                    return rv;
                }
                if watched {
                    tv_copy(&(*di).di_tv, &mut oldtv);
                }
                tv_clear(&mut (*di).di_tv);
            }

            tv_copy(&tv, &mut (*di).di_tv);
            if watched {
                tv_dict_watcher_notify(dict, key.data, Some(&tv), Some(&oldtv));
                tv_clear(&mut oldtv);
            }
            tv_clear(&mut tv);
        }

        rv
    }
}

pub fn find_buffer_by_handle(buffer: Buffer, err: &mut Error) -> *mut FileBuf {
    unsafe {
        if buffer == 0 {
            return g_st().curbuf;
        }
        let rv = g_st()
            .buffer_handles
            .get(&buffer)
            .copied()
            .unwrap_or(ptr::null_mut());
        if rv.is_null() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid buffer id: %lld",
                i64::from(buffer),
            );
            return ptr::null_mut();
        }
        rv
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn find_window_by_handle(window: Window, err: &mut Error) -> *mut WinInfoInfo {
    unsafe {
        if window == 0 {
            return curwin_ptr();
        }
        let map_ptr = &raw const g_st().window_handles;
        let rv = (*map_ptr).get(&window).copied().unwrap_or(ptr::null_mut());
        if rv.is_null() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid window id: %lld",
                i64::from(window),
            );
            return ptr::null_mut();
        }
        rv
    }
}

pub fn find_tab_by_handle(tabpage: Tabpage, err: &mut Error) -> *mut TabPageInfo {
    unsafe {
        if tabpage == 0 {
            return g_st().curtab;
        }
        let rv = g_st()
            .tabpage_handles
            .get(&tabpage)
            .copied()
            .unwrap_or(ptr::null_mut());
        if rv.is_null() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid tabpage id: %lld",
                i64::from(tabpage),
            );
            return ptr::null_mut();
        }
        rv
    }
}

#[inline]
pub fn find_buffer_by_handle_mut(buffer: Buffer, err: &mut Error) -> Option<&'static mut FileBuf> {
    let b = find_buffer_by_handle(buffer, err);
    (!b.is_null()).then(|| unsafe { &mut *b })
}

#[inline]
pub fn find_window_by_handle_mut(
    window: Window,
    err: &mut Error,
) -> Option<&'static mut WinInfoInfo> {
    let w = find_window_by_handle(window, err);
    (!w.is_null()).then(|| unsafe { &mut *w })
}

#[inline]
pub fn find_tab_by_handle_mut(
    tabpage: Tabpage,
    err: &mut Error,
) -> Option<&'static mut TabPageInfo> {
    let t = find_tab_by_handle(tabpage, err);
    (!t.is_null()).then(|| unsafe { &mut *t })
}

pub fn cstr_to_string(str_: impl crate::strings::IntoCCharPtr) -> NvimString {
    let str_ = str_.into_c_char_ptr();
    {
        if str_.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        let len = crate::memory::cstr_len(str_);
        NvimString {
            data: alloc_cstring_len(str_, len),
            size: len,
        }
    }
}

pub fn string_to_cstr(str_: NvimString) -> std::ffi::CString {
    {
        let slice = if str_.data.is_null() || str_.size == 0 {
            &[]
        } else {
            crate::memory::cstr_slice(str_.data, str_.size)
        };
        crate::memory::cstr_from_bytes(slice)
    }
}

pub fn cbuf_to_string(buf: impl crate::strings::IntoCCharPtr, size: usize) -> NvimString {
    let buf = buf.into_c_char_ptr();
    NvimString {
        data: alloc_cstring_len(buf, size),
        size,
    }
}

pub fn cstrn_as_string(str_: impl crate::strings::IntoCCharPtr, maxsize: usize) -> NvimString {
    let str_ = str_.into_c_char_ptr();
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

pub fn cstr_as_string<S: crate::strings::IntoCCharPtr>(str_: S) -> NvimString {
    let str_ = str_.into_c_char_ptr();
    {
        if str_.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        NvimString {
            data: str_.cast_mut(),
            size: crate::memory::cstr_len(str_),
        }
    }
}

pub fn string_to_array(input: NvimString, crlf: bool, arena: &mut Arena) -> Array {
    unsafe {
        let mut ret = ArrayBuilder::new();

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
            {
                let _d = (s.data).cast::<u8>();
                let _sz = line_len;
                if _sz > 0 {
                    let _c = (NUL) as u8;
                    let _x = (NL) as u8;
                    for _b in std::slice::from_raw_parts_mut(_d, _sz) {
                        if *_b == _c {
                            *_b = _x;
                        }
                    }
                }
            };
            ret.push(Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(s),
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
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(empty_s),
                });
            }
            i += 1;
        }

        arena_take_arraybuilder(arena, &mut ret)
    }
}

pub fn normalize_index(
    buf: &mut FileBuf,
    mut index: i64,
    end_exclusive: bool,
    oob: &mut bool,
) -> i64 {
    debug_assert!(buf.b_ml.ml_line_count > 0);
    let max_index = i64::from(buf.b_ml.ml_line_count) + i64::from(end_exclusive) - 1;
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

pub fn buf_get_text(
    buf: &mut FileBuf,
    lnum: i64,
    mut start_col: i64,
    mut end_col: i64,
    err: &mut Error,
) -> NvimString {
    unsafe {
        let rv = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        if lnum >= MAXLNUM {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"'line index' value out of range",
            );
            return rv;
        }

        let bufstr = ml_get_buf(buf, lnum as Linenr);
        let line_length = i64::from(ml_get_buf_len(buf, lnum as Linenr));

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
                ErrorType::ErrorTypeValidation,
                c"start_col must be less than or equal to end_col",
            );
            return rv;
        }

        NvimString {
            data: bufstr.add(start_col as usize),
            size: (end_col - start_col) as usize,
        }
    }
}

pub fn api_free_string(value: NvimString) {
    free_cstring(value.data);
}

pub fn arena_array(arena: &mut Arena, max_size: usize) -> Array {
    Array {
        size: 0,
        capacity: max_size,
        items: arena_alloc(arena, size_of::<Object>() * max_size, true).cast(),
    }
}

#[inline]
pub fn arena_array_opt(arena: Option<&mut Arena>, max_size: usize) -> Array {
    Array {
        size: 0,
        capacity: max_size,
        items: arena_alloc_opt(arena, size_of::<Object>() * max_size, true).cast(),
    }
}

pub fn arena_dict(arena: &mut Arena, max_size: usize) -> Dict {
    Dict {
        size: 0,
        capacity: max_size,
        items: arena_alloc(arena, size_of::<KeyValuePair>() * max_size, true).cast(),
    }
}

#[inline]
pub fn arena_dict_opt(arena: Option<&mut Arena>, max_size: usize) -> Dict {
    Dict {
        size: 0,
        capacity: max_size,
        items: arena_alloc_opt(arena, size_of::<KeyValuePair>() * max_size, true).cast(),
    }
}

pub fn arena_string(arena: &mut Arena, str_: NvimString) -> NvimString {
    if str_.size != 0 {
        NvimString {
            data: arena_memdupz(arena, str_.data, str_.size),
            size: str_.size,
        }
    } else {
        NvimString {
            data: c"".as_ptr().cast_mut(),
            size: 0,
        }
    }
}

#[inline]
pub fn arena_string_opt(arena: Option<&mut Arena>, str_: NvimString) -> NvimString {
    match arena {
        Some(a) => arena_string(a, str_),
        None => {
            if str_.size != 0 {
                NvimString {
                    data: arena_memdupz_opt(None, str_.data, str_.size),
                    size: str_.size,
                }
            } else {
                NvimString {
                    data: alloc_cstring(c""),
                    size: 0,
                }
            }
        }
    }
}

pub fn arena_take_arraybuilder(arena: &mut Arena, arr: &mut ArrayBuilder) -> Array {
    unsafe {
        let len = arr.len();
        let mut ret = arena_array(arena, len);
        ret.size = len;
        if len > 0 {
            ptr::copy_nonoverlapping(arr.as_ptr(), ret.items, len);
        }
        *arr = Vec::new();
        ret
    }
}

pub fn api_free_object(value: Object) {
    match value.obj_type {
        ObjectType::ObjectTypeUnset
        | ObjectType::ObjectTypeNil
        | ObjectType::ObjectTypeBoolean
        | ObjectType::ObjectTypeInteger
        | ObjectType::ObjectTypeFloat
        | ObjectType::ObjectTypeBuffer
        | ObjectType::ObjectTypeWindow
        | ObjectType::ObjectTypeTabpage => {}
        ObjectType::ObjectTypeString => api_free_string(value.data.string()),
        ObjectType::ObjectTypeArray => api_free_array(value.data.array()),
        ObjectType::ObjectTypeDict => api_free_dict(value.data.dict()),
        ObjectType::ObjectTypeLuaRef => api_free_luaref(value.data.luaref()),
    }
}

pub fn api_free_array(value: Array) {
    for &item in value.as_slice() {
        api_free_object(item);
    }
    free_items(value.items);
}

pub fn api_free_dict(value: Dict) {
    for &item in value.as_slice() {
        api_free_string(item.key);
        api_free_object(item.value);
    }
    free_items(value.items);
}

pub fn api_clear_error(value: &mut Error) {
    if !error_set(value) {
        return;
    }
    value.msg = None;
    value.err_type = ErrorType::ErrorTypeNone;
}

pub fn copy_string(str_: NvimString, arena: Option<&mut Arena>) -> NvimString {
    if !str_.data.is_null() {
        NvimString {
            data: arena_memdupz_opt(arena, str_.data, str_.size),
            size: str_.size,
        }
    } else {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    }
}

pub fn copy_array(array: Array, mut arena: Option<&mut Arena>) -> Array {
    unsafe {
        let mut rv = arena_array_opt(arena.as_deref_mut(), array.size);
        for i in 0..array.size {
            *rv.items.add(i) = copy_object(array[i], arena.as_deref_mut());
        }
        rv.size = array.size;
        rv
    }
}

pub fn copy_dict(dict: Dict, mut arena: Option<&mut Arena>) -> Dict {
    unsafe {
        let mut rv = arena_dict_opt(arena.as_deref_mut(), dict.size);
        for i in 0..dict.size {
            let item = dict[i];
            let k_copy = copy_string(item.key, arena.as_deref_mut());
            *rv.items.add(i) = KeyValuePair {
                key: cstr_as_string(k_copy.data),
                value: copy_object(item.value, arena.as_deref_mut()),
            };
        }
        rv.size = dict.size;
        rv
    }
}

pub fn copy_object(obj: Object, arena: Option<&mut Arena>) -> Object {
    match obj.obj_type {
        ObjectType::ObjectTypeUnset
        | ObjectType::ObjectTypeBuffer
        | ObjectType::ObjectTypeTabpage
        | ObjectType::ObjectTypeWindow
        | ObjectType::ObjectTypeNil
        | ObjectType::ObjectTypeBoolean
        | ObjectType::ObjectTypeInteger
        | ObjectType::ObjectTypeFloat => obj,
        ObjectType::ObjectTypeString => Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(copy_string(obj.data.string(), arena)),
        },
        ObjectType::ObjectTypeArray => Object {
            obj_type: ObjectType::ObjectTypeArray,
            data: ObjectData::Array(copy_array(obj.data.array(), arena)),
        },
        ObjectType::ObjectTypeDict => Object {
            obj_type: ObjectType::ObjectTypeDict,
            data: ObjectData::Dict(copy_dict(obj.data.dict(), arena)),
        },
        ObjectType::ObjectTypeLuaRef => Object {
            obj_type: ObjectType::ObjectTypeLuaRef,
            data: ObjectData::LuaRef(api_new_luaref(obj.data.luaref())),
        },
    }
}

pub fn api_object_to_bool(
    obj: Object,
    what: impl crate::strings::IntoCCharPtr,
    nil_value: bool,
    err: &mut Error,
) -> bool {
    let what = what.into_c_char_ptr();
    match obj.obj_type {
        ObjectType::ObjectTypeBoolean => obj.data.boolean(),
        ObjectType::ObjectTypeInteger => obj.data.integer() != 0,
        ObjectType::ObjectTypeNil => nil_value,
        _ => {
            api_err_exp(err, what, c"boolean", ptr::null());
            false
        }
    }
}

pub fn object_to_hl_id(
    obj: Object,
    what: impl crate::strings::IntoCCharPtr,
    err: &mut Error,
) -> c_int {
    let what = what.into_c_char_ptr();
    unsafe {
        match obj.obj_type {
            ObjectType::ObjectTypeString => {
                let str_ = obj.data.string();
                if str_.size != 0 {
                    syn_check_group(str_.data, str_.size)
                } else {
                    0
                }
            }
            ObjectType::ObjectTypeInteger => {
                let id = obj.data.integer() as c_int;
                if (1..=highlight_num_groups()).contains(&id) {
                    id
                } else {
                    0
                }
            }
            _ => {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Invalid 'hl_group': '%s'",
                    what,
                );
                0
            }
        }
    }
}

pub fn api_typename(t: ObjectType) -> *const c_char {
    match t {
        ObjectType::ObjectTypeUnset => c"unset".as_ptr(),
        ObjectType::ObjectTypeNil => c"nil".as_ptr(),
        ObjectType::ObjectTypeBoolean => c"Boolean".as_ptr(),
        ObjectType::ObjectTypeInteger => c"Integer".as_ptr(),
        ObjectType::ObjectTypeFloat => c"Float".as_ptr(),
        ObjectType::ObjectTypeString => c"String".as_ptr(),
        ObjectType::ObjectTypeArray => c"Array".as_ptr(),
        ObjectType::ObjectTypeDict => c"Dict".as_ptr(),
        ObjectType::ObjectTypeLuaRef => c"Function".as_ptr(),
        ObjectType::ObjectTypeBuffer => c"Buffer".as_ptr(),
        ObjectType::ObjectTypeWindow => c"Window".as_ptr(),
        ObjectType::ObjectTypeTabpage => c"Tabpage".as_ptr(),
    }
}

pub fn parse_hl_msg(chunks: Array, is_err: bool, err: &mut Error) -> HlMessage {
    unsafe {
        let mut hl_msg = HlMessage::new();
        for i in 0..chunks.size {
            let item = chunks[i];
            if item.obj_type != ObjectType::ObjectTypeArray {
                api_err_exp(
                    err,
                    c"chunk",
                    api_typename(ObjectType::ObjectTypeArray),
                    api_typename(item.obj_type),
                );
                return HlMessage::new();
            }
            let chunk = item.data.array();
            if chunk.size == 0
                || chunk.size > 2
                || (*chunk.items).obj_type != ObjectType::ObjectTypeString
            {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"%s",
                    c"Invalid chunk: expected Array with 1 or 2 Strings",
                );
                return HlMessage::new();
            }

            let str_ = copy_string((*chunk.items).data.string(), None);
            let hl_id = if chunk.size == 2 {
                object_to_hl_id(chunk[1], c"text highlight", err)
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

pub fn api_dict_to_keydict(
    retval: *mut OptKeySet,
    hashy: FieldHashfn,
    mut dict: Dict,
    err: &mut Error,
) -> bool {
    unsafe {
        let Some(hashy_fn) = hashy else {
            return false;
        };
        for i in 0..dict.size {
            let k = dict[i].key;
            let key_bytes = if k.data.is_null() || k.size == 0 {
                &[][..]
            } else {
                crate::memory::cstr_slice(k.data, k.size)
            };
            let Some(field) = hashy_fn(key_bytes) else {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Invalid key: '%.*s'",
                    k.size as c_int,
                    k.data,
                );
                return false;
            };

            if field.opt_index >= 0 {
                (*retval).is_set_ |= 1u64 << field.opt_index;
            }

            let field_name = field.str.map_or(ptr::null(), |s| s.as_ptr());
            let mem = retval.cast::<u8>().add(field.ptr_off);
            let value: *mut Object = &raw mut dict[i].value;
            let value = &mut *value;
            let field_type =
                ObjectType::from_raw(field.key_type).unwrap_or(ObjectType::ObjectTypeUnset);

            match field_type {
                ObjectType::ObjectTypeNil => {
                    *mem.cast::<Object>() = *value;
                }
                ObjectType::ObjectTypeInteger => {
                    if field.is_hlgroup {
                        let mut hl_id: c_int = 0;
                        if value.obj_type != ObjectType::ObjectTypeNil {
                            hl_id = object_to_hl_id(*value, k.data, err);
                            if error_set(err) {
                                return false;
                            }
                        }
                        *mem.cast::<Integer>() = Integer::from(hl_id);
                    } else {
                        if value.obj_type != ObjectType::ObjectTypeInteger {
                            api_err_exp(
                                err,
                                field_name,
                                api_typename(ObjectType::ObjectTypeInteger),
                                api_typename(value.obj_type),
                            );
                            return false;
                        }
                        *mem.cast::<Integer>() = value.data.integer();
                    }
                }
                ObjectType::ObjectTypeFloat => {
                    let val = mem.cast::<Float>();
                    if value.obj_type == ObjectType::ObjectTypeInteger {
                        *val = value.data.integer() as Float;
                    } else if value.obj_type == ObjectType::ObjectTypeFloat {
                        *val = value.data.floating();
                    } else {
                        api_err_exp(
                            err,
                            field_name,
                            api_typename(ObjectType::ObjectTypeFloat),
                            api_typename(value.obj_type),
                        );
                        return false;
                    }
                }
                ObjectType::ObjectTypeBoolean => {
                    *mem.cast::<Boolean>() = api_object_to_bool(*value, field_name, false, err);
                    if error_set(err) {
                        return false;
                    }
                }
                ObjectType::ObjectTypeString => {
                    if value.obj_type != ObjectType::ObjectTypeString {
                        api_err_exp(
                            err,
                            field_name,
                            api_typename(ObjectType::ObjectTypeString),
                            api_typename(value.obj_type),
                        );
                        return false;
                    }
                    *mem.cast::<NvimString>() = value.data.string();
                }
                ObjectType::ObjectTypeArray => {
                    if value.obj_type != ObjectType::ObjectTypeArray {
                        api_err_exp(
                            err,
                            field_name,
                            api_typename(ObjectType::ObjectTypeArray),
                            api_typename(value.obj_type),
                        );
                        return false;
                    }
                    *mem.cast::<Array>() = value.data.array();
                }
                ObjectType::ObjectTypeDict => {
                    let val = mem.cast::<Dict>();
                    if value.obj_type == ObjectType::ObjectTypeArray && value.data.array().size == 0
                    {
                        *val = Dict {
                            items: ptr::null_mut(),
                            size: 0,
                            capacity: 0,
                        };
                    } else if value.obj_type == ObjectType::ObjectTypeDict {
                        *val = value.data.dict();
                    } else {
                        api_err_exp(
                            err,
                            field_name,
                            api_typename(field_type),
                            api_typename(value.obj_type),
                        );
                        return false;
                    }
                }
                ObjectType::ObjectTypeBuffer
                | ObjectType::ObjectTypeWindow
                | ObjectType::ObjectTypeTabpage => {
                    if value.obj_type == ObjectType::ObjectTypeInteger
                        || value.obj_type == field_type
                    {
                        *mem.cast::<ObjHandle>() = value.data.integer() as ObjHandle;
                    } else {
                        api_err_exp(
                            err,
                            field_name,
                            api_typename(field_type),
                            api_typename(value.obj_type),
                        );
                        return false;
                    }
                }
                ObjectType::ObjectTypeLuaRef => {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"Invalid key: '%.*s' is only allowed from Lua",
                        k.size as c_int,
                        k.data,
                    );
                    return false;
                }
                ObjectType::ObjectTypeUnset => std::process::abort(),
            }
        }
        true
    }
}

pub fn api_keydict_to_dict(
    value: *mut OptKeySet,
    table: *mut KeySetLink,
    max_size: usize,
    arena: &mut Arena,
) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, max_size);
        let mut i: usize = 0;
        while let Some(field_str) = (*table.add(i)).str {
            let field = &*table.add(i);
            let mut is_set = true;
            if field.opt_index >= 0 {
                is_set = ((*value).is_set_ & (1u64 << field.opt_index)) != 0;
            }
            if !is_set {
                i += 1;
                continue;
            }

            let mem = value.cast::<u8>().add(field.ptr_off);
            let field_type =
                ObjectType::from_raw(field.key_type).unwrap_or(ObjectType::ObjectTypeUnset);
            let val = match field_type {
                ObjectType::ObjectTypeNil => *mem.cast::<Object>(),
                ObjectType::ObjectTypeInteger => Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(*mem.cast::<Integer>()),
                },
                ObjectType::ObjectTypeFloat => Object {
                    obj_type: ObjectType::ObjectTypeFloat,
                    data: ObjectData::Floating(*mem.cast::<Float>()),
                },
                ObjectType::ObjectTypeBoolean => Object {
                    obj_type: ObjectType::ObjectTypeBoolean,
                    data: ObjectData::Boolean(*mem.cast::<Boolean>()),
                },
                ObjectType::ObjectTypeString => Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(*mem.cast::<NvimString>()),
                },
                ObjectType::ObjectTypeArray => Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(*mem.cast::<Array>()),
                },
                ObjectType::ObjectTypeDict => Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(*mem.cast::<Dict>()),
                },
                ObjectType::ObjectTypeBuffer
                | ObjectType::ObjectTypeWindow
                | ObjectType::ObjectTypeTabpage => Object {
                    obj_type: field_type,
                    data: ObjectData::Integer(Integer::from(*mem.cast::<ObjHandle>())),
                },
                ObjectType::ObjectTypeLuaRef => OBJECT_NIL,
                ObjectType::ObjectTypeUnset => std::process::abort(),
            };

            *rv.items.add(rv.size) = KeyValuePair {
                key: cstr_as_string(field_str.as_ptr()),
                value: val,
            };
            rv.size += 1;
            i += 1;
        }
        rv
    }
}

pub fn api_luarefs_free_object(value: Object) {
    match value.obj_type {
        ObjectType::ObjectTypeLuaRef => api_free_luaref(value.data.luaref()),
        ObjectType::ObjectTypeArray => api_luarefs_free_array(value.data.array()),
        ObjectType::ObjectTypeDict => api_luarefs_free_dict(value.data.dict()),
        _ => {}
    }
}

pub fn api_luarefs_free_keydict(dict: *mut OptKeySet, table: *mut KeySetLink) {
    unsafe {
        let mut i: usize = 0;
        while (*table.add(i)).str.is_some() {
            let field = &*table.add(i);
            let mem = dict.cast::<u8>().add(field.ptr_off);
            let field_type =
                ObjectType::from_raw(field.key_type).unwrap_or(ObjectType::ObjectTypeUnset);
            match field_type {
                ObjectType::ObjectTypeNil => api_luarefs_free_object(*mem.cast::<Object>()),
                ObjectType::ObjectTypeLuaRef => api_free_luaref(*mem.cast::<LuaRef>()),
                ObjectType::ObjectTypeDict => api_luarefs_free_dict(*mem.cast::<Dict>()),
                _ => {}
            }
            i += 1;
        }
    }
}

pub fn api_luarefs_free_array(value: Array) {
    for &item in value.as_slice() {
        api_luarefs_free_object(item);
    }
}

pub fn api_luarefs_free_dict(value: Dict) {
    for &item in value.as_slice() {
        api_luarefs_free_object(item.value);
    }
}

pub fn set_mark(
    mut buf: *mut FileBuf,
    name: NvimString,
    line: Integer,
    mut col: Integer,
    err: &mut Error,
) -> bool {
    unsafe {
        if buf.is_null() {
            buf = g_st().curbuf;
        }
        let mut deleting = false;
        if line == 0 {
            col = 0;
            deleting = true;
        } else {
            if col > MAXCOL {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"'column' value out of range",
                );
                return false;
            }
            if line < 1 || line > i64::from((*buf).b_ml.ml_line_count) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"'line' value out of range",
                );
                return false;
            }
        }
        let mut pos = CursorPos {
            lnum: line as Linenr,
            col: col as Colnr,
            coladd: 0,
        };
        let res = setmark_pos(
            crate::memory::cstr_first_char(name.data),
            &mut pos,
            (*buf).b_fnum,
            None,
        );
        if !res {
            if deleting {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Failed to delete named mark: %c",
                    crate::memory::cstr_first_char(name.data),
                );
            } else {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Failed to set named mark: %c",
                    crate::memory::cstr_first_char(name.data),
                );
            }
        }
        res
    }
}

pub fn get_default_stl_hl(
    wp: *mut WinInfoInfo,
    use_winbar: bool,
    stc_hl_id: c_int,
) -> *const c_char {
    {
        if wp.is_null() {
            c"TabLineFill".as_ptr()
        } else if use_winbar {
            if wp == curwin_ptr() {
                c"WinBar".as_ptr()
            } else {
                c"WinBarNC".as_ptr()
            }
        } else if stc_hl_id > 0 {
            syn_id2name(stc_hl_id)
        } else if wp == curwin_ptr() {
            c"StatusLine".as_ptr()
        } else {
            c"StatusLineNC".as_ptr()
        }
    }
}

pub fn api_set_sctx(channel_id: u64) -> Sctx {
    let old_current_sctx = g_st().current_sctx;
    if channel_id != VIML_INTERNAL_CALL {
        g_st().current_sctx.sc_lnum = 0;
        if channel_id == LUA_INTERNAL_CALL {
            if !script_is_lua(g_st().current_sctx.sc_sid) {
                g_st().current_sctx.sc_sid = SID_LUA;
            }
        } else {
            g_st().current_sctx.sc_sid = SID_API_CLIENT;
            g_st().current_sctx.sc_chan = channel_id;
        }
    }
    old_current_sctx
}

struct ApiMetadataState {
    mem_for_metadata: ArenaMem,
    metadata_obj: Object,
}

impl ApiMetadataState {
    const fn new() -> Self {
        Self {
            mem_for_metadata: ptr::null_mut(),
            metadata_obj: OBJECT_NIL,
        }
    }
}

static API_META_STATE: crate::globals::GlobalState<ApiMetadataState> =
    crate::globals::GlobalState::new(ApiMetadataState::new());

#[inline]
fn api_meta_st() -> &'static mut ApiMetadataState {
    unsafe { &mut *API_META_STATE.as_ptr() }
}

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
    NVIM_API_LEVEL_COMPAT_CONST, NVIM_API_LEVEL_CONST, NVIM_API_PRERELEASE_CONST,
    NVIM_VERSION_MAJOR_CONST, NVIM_VERSION_MINOR_CONST, NVIM_VERSION_PATCH_CONST,
    NVIM_VERSION_PRERELEASE_CONST, nvim_version_build,
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

pub fn api_get_packed_metadata(len: &mut usize) -> *const u8 {
    let bytes = PACKED_API_METADATA.get_or_init(|| {
        let mut out = Vec::with_capacity(32768);
        out.push(0x86); // fixmap(6)
        mpack_push_str(&mut out, b"version");
        out.push(0x88); // fixmap(8)
        mpack_push_str(&mut out, b"major");
        mpack_push_uint(&mut out, NVIM_VERSION_MAJOR_CONST as u32);
        mpack_push_str(&mut out, b"minor");
        mpack_push_uint(&mut out, NVIM_VERSION_MINOR_CONST as u32);
        mpack_push_str(&mut out, b"patch");
        mpack_push_uint(&mut out, NVIM_VERSION_PATCH_CONST as u32);
        mpack_push_str(&mut out, b"prerelease");
        mpack_push_bool(&mut out, NVIM_VERSION_PRERELEASE_CONST);
        mpack_push_str(&mut out, b"api_level");
        mpack_push_uint(&mut out, NVIM_API_LEVEL_CONST as u32);
        mpack_push_str(&mut out, b"api_compatible");
        mpack_push_uint(&mut out, NVIM_API_LEVEL_COMPAT_CONST as u32);
        mpack_push_str(&mut out, b"api_prerelease");
        mpack_push_bool(&mut out, NVIM_API_PRERELEASE_CONST);
        mpack_push_str(&mut out, b"build");
        let build_ptr = nvim_version_build();
        if build_ptr.is_null() {
            out.push(0xc0);
        } else {
            let b = crate::memory::cstr_as_bytes(build_ptr);
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
    *len = bytes.len();
    bytes.as_ptr()
}

pub fn api_metadata() -> Object {
    if api_meta_st().metadata_obj.obj_type == ObjectType::ObjectTypeNil {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut err = Error::new();
        let mut len: usize = 0;
        let ptr = api_get_packed_metadata(&mut len).cast::<c_char>();
        api_meta_st().metadata_obj = unpack(ptr, len, &mut arena, &mut err);
        if error_set(&err) || api_meta_st().metadata_obj.obj_type != ObjectType::ObjectTypeDict {
            std::process::abort();
        }
        api_meta_st().mem_for_metadata = arena_finish(&mut arena);
    }
    api_meta_st().metadata_obj
}

pub fn api_metadata_raw() -> NvimString {
    let mut len: usize = 0;
    let ptr = api_get_packed_metadata(&mut len)
        .cast::<c_char>()
        .cast_mut();
    NvimString {
        data: ptr,
        size: len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_typename_and_cstr_helpers_work() {
        unsafe {
            assert_eq!(
                CStr::from_ptr(api_typename(ObjectType::ObjectTypeString))
                    .to_str()
                    .unwrap(),
                "String"
            );
            assert_eq!(
                CStr::from_ptr(api_typename(ObjectType::ObjectTypeLuaRef))
                    .to_str()
                    .unwrap(),
                "Function"
            );
            let s = cstr_to_string(c"hello");
            assert_eq!(s.size, 5);
            let cstr = string_to_cstr(s);
            assert_eq!(cstr.to_str().unwrap(), "hello");
            api_free_string(s);

            let raw = api_metadata_raw();
            assert!(!raw.data.is_null());
            assert!(raw.size > 30_000);
            let meta = api_metadata();
            assert_eq!(meta.obj_type, ObjectType::ObjectTypeDict);
            assert_eq!(meta.data.dict().size, 6);
        }
    }
}
