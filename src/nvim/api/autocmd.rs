//! Autocommand API functions (`src/nvim/api/autocmd.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clippy::missing_safety_doc,
    clippy::needless_range_loop
)]

use std::ffi::{CStr, c_char, c_int, c_void};

use crate::api::private::converter::sctx_T;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_conflict,
    api_err_exp, api_err_invalid, api_err_required, check_string_array,
};
use crate::eval::deprecated::{Callback, buf_T};
use crate::kvec::KVecWithInit;
use crate::lua::converter::LuaRef;
use crate::map_glyph_cache::NvimString;
use crate::memory::{Arena, arena_memdupz, xfree, xstrndup};
use crate::msgpack_rpc::packer::{Integer, handle_T};

pub type Buffer = handle_T;
pub type event_T = c_int;
pub type OptionalKeys = u64;
pub type ArrayBuilder = KVecWithInit<Object, 16>;

const AUCMD_MAX_PATTERNS: usize = 256;
const NUM_EVENTS: usize = 150;
const BUFLOCAL_PAT_LEN: usize = 25;
const AUGROUP_DEFAULT: c_int = -1;
const AUGROUP_ERROR: c_int = -2;
const AUGROUP_ALL: c_int = -3;
const LUA_NOREF: LuaRef = -2;
const FAIL: c_int = 0;

pub const kCallbackNone: c_int = 0;
pub const kCallbackFuncref: c_int = 1;
pub const kCallbackPartial: c_int = 2;
pub const kCallbackLua: c_int = 3;
pub const kCallbackExpr: c_int = 4;

const KEYSET_OPTIDX_get_autocmds__id: u64 = 1;
const KEYSET_OPTIDX_get_autocmds__buf: u64 = 2;
const KEYSET_OPTIDX_get_autocmds__event: u64 = 3;
const KEYSET_OPTIDX_get_autocmds__group: u64 = 4;
const KEYSET_OPTIDX_get_autocmds__buffer: u64 = 5;
const KEYSET_OPTIDX_get_autocmds__pattern: u64 = 6;

const KEYSET_OPTIDX_create_autocmd__buf: u64 = 1;
const KEYSET_OPTIDX_create_autocmd__desc: u64 = 2;
const KEYSET_OPTIDX_create_autocmd__once: u64 = 3;
const KEYSET_OPTIDX_create_autocmd__group: u64 = 4;
const KEYSET_OPTIDX_create_autocmd__buffer: u64 = 5;
const KEYSET_OPTIDX_create_autocmd__nested: u64 = 6;
const KEYSET_OPTIDX_create_autocmd__command: u64 = 7;
const KEYSET_OPTIDX_create_autocmd__pattern: u64 = 8;
const KEYSET_OPTIDX_create_autocmd__callback: u64 = 9;

const KEYSET_OPTIDX_clear_autocmds__buf: u64 = 1;
const KEYSET_OPTIDX_clear_autocmds__event: u64 = 2;
const KEYSET_OPTIDX_clear_autocmds__group: u64 = 3;
const KEYSET_OPTIDX_clear_autocmds__buffer: u64 = 4;
const KEYSET_OPTIDX_clear_autocmds__pattern: u64 = 5;

const KEYSET_OPTIDX_create_augroup__clear: u64 = 1;

const KEYSET_OPTIDX_exec_autocmds__buf: u64 = 1;
const KEYSET_OPTIDX_exec_autocmds__data: u64 = 2;
const KEYSET_OPTIDX_exec_autocmds__group: u64 = 3;
const KEYSET_OPTIDX_exec_autocmds__buffer: u64 = 4;
const KEYSET_OPTIDX_exec_autocmds__pattern: u64 = 5;
const KEYSET_OPTIDX_exec_autocmds__modeline: u64 = 6;

#[repr(C)]
pub struct AutoPat {
    pub refcount: usize,
    pub pat: *mut c_char,
    pub reg_prog: *mut c_void,
    pub group: c_int,
    pub patlen: c_int,
    pub buflocal_nr: c_int,
    pub allow_dirs: c_char,
}

#[repr(C)]
pub struct AutoCmd {
    pub pat: *mut AutoPat,
    pub id: i64,
    pub desc: *mut c_char,
    pub handler_cmd: *mut c_char,
    pub handler_fn: Callback,
    pub script_ctx: sctx_T,
    pub once: bool,
    pub nested: bool,
}

#[repr(C)]
pub struct AutoCmdVec {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut AutoCmd,
}

#[repr(C)]
pub struct TryState {
    pub current_exception: *mut c_void,
    pub private_msg_list: *mut c_void,
    pub msg_list: *const *const c_void,
    pub got_int: c_int,
    pub did_throw: bool,
    pub need_rethrow: c_int,
    pub did_emsg: c_int,
}

#[repr(C)]
pub struct KeyDict_get_autocmds {
    pub is_set__get_autocmds_: OptionalKeys,
    pub event: Object,
    pub group: Object,
    pub pattern: Object,
    pub buffer: Object,
    pub buf: Object,
    pub id: Integer,
}

#[repr(C)]
pub struct KeyDict_create_autocmd {
    pub is_set__create_autocmd_: OptionalKeys,
    pub buffer: Buffer,
    pub buf: Buffer,
    pub callback: Object,
    pub command: NvimString,
    pub desc: NvimString,
    pub group: Object,
    pub nested: bool,
    pub once: bool,
    pub pattern: Object,
}

#[repr(C)]
pub struct KeyDict_clear_autocmds {
    pub is_set__clear_autocmds_: OptionalKeys,
    pub buffer: Buffer,
    pub buf: Buffer,
    pub event: Object,
    pub group: Object,
    pub pattern: Object,
}

#[repr(C)]
pub struct KeyDict_create_augroup {
    pub is_set__create_augroup_: OptionalKeys,
    pub clear: bool,
}

#[repr(C)]
pub struct KeyDict_exec_autocmds {
    pub is_set__exec_autocmds_: OptionalKeys,
    pub buffer: Buffer,
    pub buf: Buffer,
    pub group: Object,
    pub modeline: bool,
    pub pattern: Object,
    pub data: Object,
}

static mut NEXT_AUTOCMD_ID: i64 = 1;

#[cfg(not(test))]
unsafe extern "C" {
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut c_void;
    static mut current_sctx: sctx_T;

    fn event_name2nr_str(str: NvimString) -> event_T;
    fn event_nr2name(event: event_T) -> *const c_char;
    fn augroup_find(name: *const c_char) -> c_int;
    fn augroup_name(group: c_int) -> *mut c_char;
    fn augroup_exists(name: *const c_char) -> bool;
    fn augroup_add(name: *const c_char) -> c_int;
    fn augroup_del_by_id(group: c_int);
    fn augroup_del(name: *mut c_char, id: c_int, stupid_legacy_mode: bool);
    fn aucmd_del_for_event_and_group(event: event_T, group: c_int);
    fn au_get_autocmds_for_event(event: event_T) -> *mut AutoCmdVec;
    fn aupat_is_buflocal(pat: *const c_char, patlen: c_int) -> bool;
    fn aupat_get_buflocal_nr(pat: *const c_char, patlen: c_int) -> c_int;
    fn aupat_normalize_buflocal_pat(
        dest: *mut c_char,
        pat: *const c_char,
        patlen: c_int,
        buflocal_nr: c_int,
    );
    fn aucmd_span_pattern(pat: *const c_char, start: *mut *const c_char) -> usize;
    fn autocmd_register(
        id: i64,
        event: event_T,
        pat: *const c_char,
        patlen: c_int,
        group: c_int,
        once: bool,
        nested: bool,
        desc: *const c_char,
        handler_cmd: *const c_char,
        handler_fn: *mut Callback,
    ) -> c_int;
    fn autocmd_delete_id(id: i64) -> bool;
    fn do_autocmd_event(
        event: event_T,
        pat: *const c_char,
        once: bool,
        nested: c_int,
        cmd: *const c_char,
        forceit: bool,
        group: c_int,
    ) -> c_int;
    fn apply_autocmds_group(
        event: event_T,
        fname: *mut c_char,
        fname_io: *mut c_char,
        force: bool,
        group: c_int,
        buf: *mut buf_T,
        win: *mut c_void,
        eap: *mut c_void,
        data: *mut Object,
        buflocal_only: bool,
    ) -> bool;
    fn do_modelines(flags: c_int);

    fn nlua_ref_is_function(ref_id: LuaRef) -> bool;
    fn api_new_luaref(ref_id: LuaRef) -> LuaRef;
    fn callback_to_string(cb: *mut Callback, arena: *mut Arena) -> *mut c_char;
    fn callback_free(cb: *mut Callback);

    fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T;
    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict;
    fn arena_string(arena: *mut Arena, str: NvimString) -> NvimString;
    fn arena_take_arraybuilder(arena: *mut Arena, arr: *mut ArrayBuilder) -> Array;
    fn api_typename(t: ObjectType) -> *const c_char;
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn api_set_sctx(channel_id: u64) -> sctx_T;
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
}

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
unsafe fn api_err_con(err: *mut Error, name: *const c_char, name2: *const c_char) {
    unsafe {
        api_err_conflict(err, name, name2);
    }
}

#[inline]
unsafe fn api_err_inv(err: *mut Error, name: *const c_char, val_s: *const c_char) {
    unsafe {
        api_err_invalid(err, name, val_s, 0, true);
    }
}

#[inline]
unsafe fn api_err_int(err: *mut Error, name: *const c_char, val_n: i64) {
    unsafe {
        api_err_invalid(err, name, std::ptr::null(), val_n, false);
    }
}

#[inline]
unsafe fn api_err_req(err: *mut Error, name: *const c_char) {
    unsafe {
        api_err_required(err, name);
    }
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        }
    } else {
        NvimString {
            data: s.cast_mut(),
            size: unsafe { CStr::from_ptr(s).to_bytes().len() },
        }
    }
}

#[inline]
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
const fn boolean_obj(b: bool) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

#[inline]
const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
const fn luaref_obj(r: LuaRef) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeLuaRef,
        data: ObjectData { luaref: r },
    }
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: NvimString {
                data: key.as_ptr().cast_mut(),
                size: key.to_bytes().len(),
            },
            value,
        };
        dict.size += 1;
    }
}

#[inline]
unsafe fn add_c(arr: &mut Array, item: Object) {
    unsafe {
        *arr.items.add(arr.size) = item;
        arr.size += 1;
    }
}

#[inline]
unsafe fn format_buflocal_arena(arena: *mut Arena, handle: c_int) -> NvimString {
    let s = format!("<buffer={handle}>");
    unsafe {
        let data = arena_memdupz(arena, s.as_ptr().cast(), s.len());
        NvimString {
            data,
            size: s.len(),
        }
    }
}

#[cfg(not(test))]
unsafe fn unpack_string_or_array(
    v: Object,
    k: *const c_char,
    required: bool,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        if v.obj_type == ObjectType::kObjectTypeString {
            let mut arr = arena_array(arena, 1);
            add_c(&mut arr, v);
            arr
        } else if v.obj_type == ObjectType::kObjectTypeArray {
            if !check_string_array(v.data.array, k.cast_mut(), true, err) {
                return Array {
                    items: std::ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                };
            }
            v.data.array
        } else {
            if !(!required && v.obj_type == ObjectType::kObjectTypeNil) {
                api_err_exp(
                    err,
                    k,
                    c"Array or String".as_ptr(),
                    api_typename(v.obj_type),
                );
            }
            Array {
                items: std::ptr::null_mut(),
                size: 0,
                capacity: 0,
            }
        }
    }
}

#[cfg(not(test))]
unsafe fn get_augroup_from_object(group: Object, err: *mut Error) -> c_int {
    unsafe {
        match group.obj_type {
            ObjectType::kObjectTypeNil => AUGROUP_DEFAULT,
            ObjectType::kObjectTypeString => {
                let au_group = augroup_find(group.data.string.data);
                if au_group == AUGROUP_ERROR {
                    api_err_inv(err, c"group".as_ptr(), group.data.string.data);
                    return AUGROUP_ERROR;
                }
                au_group
            }
            ObjectType::kObjectTypeInteger => {
                let au_group = group.data.integer as c_int;
                let name = if au_group == 0 {
                    std::ptr::null()
                } else {
                    augroup_name(au_group)
                };
                if !augroup_exists(name) {
                    api_err_int(err, c"group".as_ptr(), i64::from(au_group));
                    return AUGROUP_ERROR;
                }
                au_group
            }
            _ => {
                api_err_exp(
                    err,
                    c"group".as_ptr(),
                    c"String or Integer".as_ptr(),
                    api_typename(group.obj_type),
                );
                AUGROUP_ERROR
            }
        }
    }
}

#[cfg(not(test))]
unsafe fn get_patterns_from_pattern_or_buf(
    pattern: Object,
    has_buf: bool,
    buf: Buffer,
    fallback: *const c_char,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut patterns: ArrayBuilder = std::mem::zeroed();
        patterns.init();

        if pattern.obj_type != ObjectType::kObjectTypeNil {
            if pattern.obj_type == ObjectType::kObjectTypeString {
                let mut pat: *const c_char = pattern.data.string.data;
                let mut patlen = aucmd_span_pattern(pat, &mut pat);
                while patlen != 0 {
                    let s = arena_string(
                        arena,
                        NvimString {
                            data: pat.cast_mut(),
                            size: patlen,
                        },
                    );
                    patterns.push(string_obj(s));
                    patlen = aucmd_span_pattern(pat.add(patlen), &mut pat);
                }
            } else if pattern.obj_type == ObjectType::kObjectTypeArray {
                if !check_string_array(
                    pattern.data.array,
                    c"pattern".as_ptr().cast_mut(),
                    true,
                    err,
                ) {
                    patterns.destroy();
                    return Array {
                        items: std::ptr::null_mut(),
                        size: 0,
                        capacity: 0,
                    };
                }
                let array = pattern.data.array;
                for i in 0..array.size {
                    let entry = *array.items.add(i);
                    let mut pat: *const c_char = entry.data.string.data;
                    let mut patlen = aucmd_span_pattern(pat, &mut pat);
                    while patlen != 0 {
                        let s = arena_string(
                            arena,
                            NvimString {
                                data: pat.cast_mut(),
                                size: patlen,
                            },
                        );
                        patterns.push(string_obj(s));
                        patlen = aucmd_span_pattern(pat.add(patlen), &mut pat);
                    }
                }
            } else {
                api_err_exp(
                    err,
                    c"pattern".as_ptr(),
                    c"String or Table".as_ptr(),
                    api_typename(pattern.obj_type),
                );
                patterns.destroy();
                return Array {
                    items: std::ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                };
            }
        } else if has_buf {
            let b = find_buffer_by_handle(buf, err);
            if error_set(err) {
                patterns.destroy();
                return Array {
                    items: std::ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                };
            }
            patterns.push(string_obj(format_buflocal_arena(arena, (*b).b_fnum)));
        } else if !fallback.is_null() {
            patterns.push(string_obj(cstr_as_string(fallback)));
        }

        arena_take_arraybuilder(arena, &mut patterns)
    }
}

#[cfg(not(test))]
unsafe fn clear_autocmd(
    event: event_T,
    pat: *const c_char,
    au_group: c_int,
    err: *mut Error,
) -> bool {
    unsafe {
        if do_autocmd_event(event, pat, false, 0, c"".as_ptr(), true, au_group) == FAIL {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to clear autocmd".as_ptr(),
            );
            return false;
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_autocmds(
    opts: *mut KeyDict_get_autocmds,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    #[cfg(test)]
    {
        let _ = (opts, arena, err);
        Array {
            items: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        }
    }

    #[cfg(not(test))]
    unsafe {
        let mut autocmd_list: ArrayBuilder = std::mem::zeroed();
        autocmd_list.init();
        let mut pattern_filters = [std::ptr::null::<c_char>(); AUCMD_MAX_PATTERNS];
        let mut buffers = Array {
            items: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let mut event_set = [false; NUM_EVENTS];
        let mut check_event = false;
        let mut group: c_int = 0;

        'body: {
            match (*opts).group.obj_type {
                ObjectType::kObjectTypeNil => {}
                ObjectType::kObjectTypeString => {
                    group = augroup_find((*opts).group.data.string.data);
                    if group < 0 {
                        api_err_inv(err, c"group".as_ptr(), (*opts).group.data.string.data);
                        break 'body;
                    }
                }
                ObjectType::kObjectTypeInteger => {
                    group = (*opts).group.data.integer as c_int;
                    let name = if group == 0 {
                        std::ptr::null()
                    } else {
                        augroup_name(group)
                    };
                    if !augroup_exists(name) {
                        api_err_int(err, c"group".as_ptr(), (*opts).group.data.integer);
                        break 'body;
                    }
                }
                _ => {
                    api_err_exp(
                        err,
                        c"group".as_ptr(),
                        c"String or Integer".as_ptr(),
                        api_typename((*opts).group.obj_type),
                    );
                    break 'body;
                }
            }

            let mask = (*opts).is_set__get_autocmds_;
            let id: c_int = if has_key(mask, KEYSET_OPTIDX_get_autocmds__id) {
                (*opts).id as c_int
            } else {
                -1
            };

            if has_key(mask, KEYSET_OPTIDX_get_autocmds__event) {
                check_event = true;
                let v = (*opts).event;
                if v.obj_type == ObjectType::kObjectTypeString {
                    let event_nr = event_name2nr_str(v.data.string);
                    if (event_nr as usize) >= NUM_EVENTS {
                        api_err_inv(err, c"event".as_ptr(), v.data.string.data);
                        break 'body;
                    }
                    event_set[event_nr as usize] = true;
                } else if v.obj_type == ObjectType::kObjectTypeArray {
                    for i in 0..v.data.array.size {
                        let event_v = *v.data.array.items.add(i);
                        if event_v.obj_type != ObjectType::kObjectTypeString {
                            api_err_exp(
                                err,
                                c"event item".as_ptr(),
                                api_typename(ObjectType::kObjectTypeString),
                                api_typename(event_v.obj_type),
                            );
                            break 'body;
                        }
                        let event_nr = event_name2nr_str(event_v.data.string);
                        if (event_nr as usize) >= NUM_EVENTS {
                            api_err_inv(err, c"event".as_ptr(), event_v.data.string.data);
                            break 'body;
                        }
                        event_set[event_nr as usize] = true;
                    }
                } else {
                    api_err_exp(
                        err,
                        c"event".as_ptr(),
                        c"String or Array".as_ptr(),
                        std::ptr::null(),
                    );
                    break 'body;
                }
            }

            let has_buf_key = has_key(mask, KEYSET_OPTIDX_get_autocmds__buf);
            let has_buffer_key = has_key(mask, KEYSET_OPTIDX_get_autocmds__buffer);
            let has_buf = has_buf_key || has_buffer_key;
            let buf = if has_buf_key {
                (*opts).buf
            } else {
                (*opts).buffer
            };

            if has_buf_key && has_buffer_key {
                api_err_con(err, c"buf".as_ptr(), c"buffer".as_ptr());
                break 'body;
            }
            if has_key(mask, KEYSET_OPTIDX_get_autocmds__pattern) && has_buf {
                api_err_con(err, c"pattern".as_ptr(), c"buf".as_ptr());
                break 'body;
            }

            let mut pattern_filter_count = 0usize;
            if has_key(mask, KEYSET_OPTIDX_get_autocmds__pattern) {
                let v = (*opts).pattern;
                if v.obj_type == ObjectType::kObjectTypeString {
                    pattern_filters[pattern_filter_count] = v.data.string.data;
                    pattern_filter_count += 1;
                } else if v.obj_type == ObjectType::kObjectTypeArray {
                    if v.data.array.size > AUCMD_MAX_PATTERNS {
                        api_set_error(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"Too many patterns (maximum of %d)".as_ptr(),
                            AUCMD_MAX_PATTERNS as c_int,
                        );
                        break 'body;
                    }
                    for i in 0..v.data.array.size {
                        let item = *v.data.array.items.add(i);
                        if item.obj_type != ObjectType::kObjectTypeString {
                            api_err_exp(
                                err,
                                c"pattern".as_ptr(),
                                api_typename(ObjectType::kObjectTypeString),
                                api_typename(item.obj_type),
                            );
                            break 'body;
                        }
                        pattern_filters[pattern_filter_count] = item.data.string.data;
                        pattern_filter_count += 1;
                    }
                } else {
                    api_err_exp(
                        err,
                        c"pattern".as_ptr(),
                        c"String or Array".as_ptr(),
                        api_typename(v.obj_type),
                    );
                    break 'body;
                }
            }

            if buf.obj_type == ObjectType::kObjectTypeInteger
                || buf.obj_type == ObjectType::kObjectTypeBuffer
            {
                let b = find_buffer_by_handle(buf.data.integer as Buffer, err);
                if error_set(err) {
                    break 'body;
                }
                let pat = format_buflocal_arena(arena, (*b).b_fnum);
                buffers = arena_array(arena, 1);
                add_c(&mut buffers, string_obj(pat));
            } else if buf.obj_type == ObjectType::kObjectTypeArray {
                if buf.data.array.size > AUCMD_MAX_PATTERNS {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Too many buffers (maximum of %d)".as_ptr(),
                        AUCMD_MAX_PATTERNS as c_int,
                    );
                    break 'body;
                }
                buffers = arena_array(arena, buf.data.array.size);
                for i in 0..buf.data.array.size {
                    let bufnr = *buf.data.array.items.add(i);
                    if bufnr.obj_type != ObjectType::kObjectTypeInteger
                        && bufnr.obj_type != ObjectType::kObjectTypeBuffer
                    {
                        api_err_exp(
                            err,
                            c"buffer".as_ptr(),
                            c"Integer".as_ptr(),
                            api_typename(bufnr.obj_type),
                        );
                        break 'body;
                    }
                    let b = find_buffer_by_handle(bufnr.data.integer as Buffer, err);
                    if error_set(err) {
                        break 'body;
                    }
                    add_c(
                        &mut buffers,
                        string_obj(format_buflocal_arena(arena, (*b).b_fnum)),
                    );
                }
            } else if has_buf {
                api_err_exp(
                    err,
                    c"buffer".as_ptr(),
                    c"Integer or Array".as_ptr(),
                    api_typename(buf.obj_type),
                );
                break 'body;
            }

            for i in 0..buffers.size {
                let bufnr = *buffers.items.add(i);
                pattern_filters[pattern_filter_count] = bufnr.data.string.data;
                pattern_filter_count += 1;
            }

            for event in 0..NUM_EVENTS {
                if check_event && !event_set[event] {
                    continue;
                }

                let acs = au_get_autocmds_for_event(event as event_T);
                for i in 0..(*acs).size {
                    let ac = &mut *(*acs).items.add(i);
                    let ap = ac.pat;
                    if ap.is_null() {
                        continue;
                    }
                    if id != -1 && ac.id != i64::from(id) {
                        continue;
                    }
                    if group != 0 && (*ap).group != group {
                        continue;
                    }

                    if pattern_filter_count > 0 {
                        let mut passed = false;
                        for &raw_pat in &pattern_filters[..pattern_filter_count] {
                            let mut pat = raw_pat;
                            let patlen = CStr::from_ptr(pat).to_bytes().len() as c_int;
                            let mut pattern_buflocal = [0 as c_char; BUFLOCAL_PAT_LEN];
                            if aupat_is_buflocal(pat, patlen) {
                                aupat_normalize_buflocal_pat(
                                    pattern_buflocal.as_mut_ptr(),
                                    pat,
                                    patlen,
                                    aupat_get_buflocal_nr(pat, patlen),
                                );
                                pat = pattern_buflocal.as_ptr();
                            }
                            if !(*ap).pat.is_null()
                                && !pat.is_null()
                                && CStr::from_ptr((*ap).pat).to_bytes()
                                    == CStr::from_ptr(pat).to_bytes()
                            {
                                passed = true;
                                break;
                            }
                        }
                        if !passed {
                            continue;
                        }
                    }

                    let mut autocmd_info = arena_dict(arena, 12);

                    if (*ap).group != AUGROUP_DEFAULT {
                        put_c(
                            &mut autocmd_info,
                            c"group",
                            integer_obj(i64::from((*ap).group)),
                        );
                        put_c(
                            &mut autocmd_info,
                            c"group_name",
                            string_obj(cstr_as_string(augroup_name((*ap).group))),
                        );
                    }

                    if ac.id > 0 {
                        put_c(&mut autocmd_info, c"id", integer_obj(ac.id));
                    }

                    if !ac.desc.is_null() {
                        put_c(
                            &mut autocmd_info,
                            c"desc",
                            string_obj(cstr_as_string(ac.desc)),
                        );
                    }

                    if !ac.handler_cmd.is_null() {
                        put_c(
                            &mut autocmd_info,
                            c"command",
                            string_obj(cstr_as_string(ac.handler_cmd)),
                        );
                    } else {
                        put_c(
                            &mut autocmd_info,
                            c"command",
                            string_obj(NvimString {
                                data: std::ptr::null_mut(),
                                size: 0,
                            }),
                        );

                        let cb = &mut ac.handler_fn;
                        match cb.cb_type {
                            kCallbackLua => {
                                let luaref = cb.data as isize as LuaRef;
                                if nlua_ref_is_function(luaref) {
                                    put_c(
                                        &mut autocmd_info,
                                        c"callback",
                                        luaref_obj(api_new_luaref(luaref)),
                                    );
                                }
                            }
                            kCallbackFuncref | kCallbackPartial => {
                                put_c(
                                    &mut autocmd_info,
                                    c"callback",
                                    string_obj(cstr_as_string(callback_to_string(cb, arena))),
                                );
                            }
                            _ => {
                                std::process::abort();
                            }
                        }
                    }

                    put_c(
                        &mut autocmd_info,
                        c"pattern",
                        string_obj(cstr_as_string((*ap).pat)),
                    );
                    put_c(
                        &mut autocmd_info,
                        c"event",
                        string_obj(cstr_as_string(event_nr2name(event as event_T))),
                    );
                    put_c(&mut autocmd_info, c"once", boolean_obj(ac.once));

                    if (*ap).buflocal_nr != 0 {
                        put_c(&mut autocmd_info, c"buflocal", boolean_obj(true));
                        put_c(
                            &mut autocmd_info,
                            c"buf",
                            integer_obj(i64::from((*ap).buflocal_nr)),
                        );
                        put_c(
                            &mut autocmd_info,
                            c"buffer",
                            integer_obj(i64::from((*ap).buflocal_nr)),
                        );
                    } else {
                        put_c(&mut autocmd_info, c"buflocal", boolean_obj(false));
                    }

                    autocmd_list.push(dict_obj(autocmd_info));
                }
            }
        }

        arena_take_arraybuilder(arena, &mut autocmd_list)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_create_autocmd(
    channel_id: u64,
    event: Object,
    opts: *mut KeyDict_create_autocmd,
    arena: *mut Arena,
    err: *mut Error,
) -> Integer {
    #[cfg(test)]
    {
        let _ = (channel_id, event, opts, arena, err);
        -1
    }

    #[cfg(not(test))]
    unsafe {
        let mut autocmd_id: i64 = -1;
        let mut desc: *const c_char = std::ptr::null();
        let mut handler_cmd: *mut c_char = std::ptr::null_mut();
        let mut handler_fn = Callback {
            data: std::ptr::null_mut(),
            cb_type: kCallbackNone,
        };

        'body: {
            let event_array = unpack_string_or_array(event, c"event".as_ptr(), true, arena, err);
            if error_set(err) {
                break 'body;
            }

            let mask = (*opts).is_set__create_autocmd_;
            let has_callback = has_key(mask, KEYSET_OPTIDX_create_autocmd__callback);
            let has_command = has_key(mask, KEYSET_OPTIDX_create_autocmd__command);

            if has_callback && has_command {
                api_err_con(err, c"callback".as_ptr(), c"command".as_ptr());
                break 'body;
            }

            if has_callback {
                let callback = &mut (*opts).callback;
                match callback.obj_type {
                    ObjectType::kObjectTypeLuaRef => {
                        if callback.data.luaref == LUA_NOREF {
                            api_err_inv(err, c"callback".as_ptr(), c"<no value>".as_ptr());
                            break 'body;
                        }
                        if !nlua_ref_is_function(callback.data.luaref) {
                            api_err_inv(err, c"callback".as_ptr(), c"<not a function>".as_ptr());
                            break 'body;
                        }
                        handler_fn.cb_type = kCallbackLua;
                        handler_fn.data = (callback.data.luaref as isize) as *mut c_void;
                        callback.data.luaref = LUA_NOREF;
                    }
                    ObjectType::kObjectTypeString => {
                        handler_fn.cb_type = kCallbackFuncref;
                        handler_fn.data =
                            xstrndup(callback.data.string.data, callback.data.string.size).cast();
                    }
                    _ => {
                        api_err_exp(
                            err,
                            c"callback".as_ptr(),
                            c"Lua function or Vim function name".as_ptr(),
                            api_typename(callback.obj_type),
                        );
                        break 'body;
                    }
                }
            } else if has_command {
                handler_cmd = xstrndup((*opts).command.data, (*opts).command.size);
            } else {
                api_err_req(err, c"'command' or 'callback'".as_ptr());
                break 'body;
            }

            let au_group = get_augroup_from_object((*opts).group, err);
            if au_group == AUGROUP_ERROR {
                break 'body;
            }

            let has_buf_key = has_key(mask, KEYSET_OPTIDX_create_autocmd__buf);
            let has_buffer_key = has_key(mask, KEYSET_OPTIDX_create_autocmd__buffer);
            let has_buf = has_buf_key || has_buffer_key;
            let buf = if has_buf_key {
                (*opts).buf
            } else {
                (*opts).buffer
            };

            if has_buf_key && has_buffer_key {
                api_err_con(err, c"buf".as_ptr(), c"buffer".as_ptr());
                break 'body;
            }
            if has_key(mask, KEYSET_OPTIDX_create_autocmd__pattern) && has_buf {
                api_err_con(err, c"pattern".as_ptr(), c"buf".as_ptr());
                break 'body;
            }

            let patterns = get_patterns_from_pattern_or_buf(
                (*opts).pattern,
                has_buf,
                buf,
                c"*".as_ptr(),
                arena,
                err,
            );
            if error_set(err) {
                break 'body;
            }
            if patterns.size == 0 {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"No non-empty patterns specified".as_ptr(),
                );
                break 'body;
            }

            if has_key(mask, KEYSET_OPTIDX_create_autocmd__desc) {
                desc = (*opts).desc.data;
            }

            if event_array.size == 0 {
                api_err_req(err, c"event".as_ptr());
                break 'body;
            }

            autocmd_id = NEXT_AUTOCMD_ID;
            NEXT_AUTOCMD_ID += 1;

            for i in 0..event_array.size {
                let event_str = *event_array.items.add(i);
                let event_nr = event_name2nr_str(event_str.data.string);
                if (event_nr as usize) >= NUM_EVENTS {
                    api_err_inv(err, c"event".as_ptr(), event_str.data.string.data);
                    break 'body;
                }

                for j in 0..patterns.size {
                    let pat = *patterns.items.add(j);
                    let save_current_sctx = api_set_sctx(channel_id);
                    let retval = autocmd_register(
                        autocmd_id,
                        event_nr,
                        pat.data.string.data,
                        pat.data.string.size as c_int,
                        au_group,
                        (*opts).once,
                        (*opts).nested,
                        desc,
                        handler_cmd,
                        &mut handler_fn,
                    );
                    current_sctx = save_current_sctx;

                    if retval == FAIL {
                        api_set_error(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Failed to set autocmd".as_ptr(),
                        );
                        break 'body;
                    }
                }
            }
        }

        if !handler_cmd.is_null() {
            xfree(handler_cmd.cast());
        } else {
            callback_free(&mut handler_fn);
        }

        autocmd_id
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_del_autocmd(id: Integer, err: *mut Error) {
    #[cfg(test)]
    let _ = (id, err);

    #[cfg(not(test))]
    unsafe {
        if id <= 0 {
            api_err_int(err, c"autocmd id".as_ptr(), id);
            return;
        }
        if !autocmd_delete_id(id) {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to delete autocmd".as_ptr(),
            );
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_clear_autocmds(
    opts: *mut KeyDict_clear_autocmds,
    arena: *mut Arena,
    err: *mut Error,
) {
    #[cfg(test)]
    let _ = (opts, arena, err);

    #[cfg(not(test))]
    unsafe {
        let event_array =
            unpack_string_or_array((*opts).event, c"event".as_ptr(), false, arena, err);
        if error_set(err) {
            return;
        }

        let mask = (*opts).is_set__clear_autocmds_;
        let has_buf_key = has_key(mask, KEYSET_OPTIDX_clear_autocmds__buf);
        let has_buffer_key = has_key(mask, KEYSET_OPTIDX_clear_autocmds__buffer);
        let has_buf = has_buf_key || has_buffer_key;
        let buf = if has_buf_key {
            (*opts).buf
        } else {
            (*opts).buffer
        };

        if has_buf_key && has_buffer_key {
            api_err_con(err, c"buf".as_ptr(), c"buffer".as_ptr());
            return;
        }
        if has_key(mask, KEYSET_OPTIDX_clear_autocmds__pattern) && has_buf {
            api_err_con(err, c"pattern".as_ptr(), c"buf".as_ptr());
            return;
        }

        let au_group = get_augroup_from_object((*opts).group, err);
        if au_group == AUGROUP_ERROR {
            return;
        }

        let patterns = get_patterns_from_pattern_or_buf(
            (*opts).pattern,
            has_buf,
            buf,
            c"".as_ptr(),
            arena,
            err,
        );
        if error_set(err) {
            return;
        }

        if (*opts).event.obj_type == ObjectType::kObjectTypeNil {
            for event in 0..NUM_EVENTS {
                for j in 0..patterns.size {
                    let pat_object = *patterns.items.add(j);
                    if !clear_autocmd(event as event_T, pat_object.data.string.data, au_group, err)
                    {
                        return;
                    }
                }
            }
        } else {
            for i in 0..event_array.size {
                let event_str = *event_array.items.add(i);
                let event_nr = event_name2nr_str(event_str.data.string);
                if (event_nr as usize) >= NUM_EVENTS {
                    api_err_inv(err, c"event".as_ptr(), event_str.data.string.data);
                    return;
                }
                for j in 0..patterns.size {
                    let pat_object = *patterns.items.add(j);
                    if !clear_autocmd(event_nr, pat_object.data.string.data, au_group, err) {
                        return;
                    }
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_create_augroup(
    channel_id: u64,
    name: NvimString,
    opts: *mut KeyDict_create_augroup,
    err: *mut Error,
) -> Integer {
    #[cfg(test)]
    {
        let _ = (channel_id, name, opts, err);
        -1
    }

    #[cfg(not(test))]
    unsafe {
        let augroup_name = name.data;
        let clear_autocmds = if has_key(
            (*opts).is_set__create_augroup_,
            KEYSET_OPTIDX_create_augroup__clear,
        ) {
            (*opts).clear
        } else {
            true
        };

        let save_current_sctx = api_set_sctx(channel_id);
        let augroup = augroup_add(augroup_name);
        if augroup == AUGROUP_ERROR {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to set augroup".as_ptr(),
            );
            current_sctx = save_current_sctx;
            return -1;
        }

        if clear_autocmds {
            for event in 0..NUM_EVENTS {
                aucmd_del_for_event_and_group(event as event_T, augroup);
            }
        }
        current_sctx = save_current_sctx;

        i64::from(augroup)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_del_augroup_by_id(id: Integer, err: *mut Error) {
    #[cfg(test)]
    let _ = (id, err);

    #[cfg(not(test))]
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        augroup_del_by_id(id as c_int);
        try_leave(&tstate, err);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_del_augroup_by_name(name: NvimString, err: *mut Error) {
    #[cfg(test)]
    let _ = (name, err);

    #[cfg(not(test))]
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        augroup_del(name.data, -1, false);
        try_leave(&tstate, err);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_exec_autocmds(
    event: Object,
    opts: *mut KeyDict_exec_autocmds,
    arena: *mut Arena,
    err: *mut Error,
) {
    #[cfg(test)]
    let _ = (event, opts, arena, err);

    #[cfg(not(test))]
    unsafe {
        let mut au_group = AUGROUP_ALL;
        let mut b = curbuf;
        let mut data: *mut Object = std::ptr::null_mut();

        let event_array = unpack_string_or_array(event, c"event".as_ptr(), true, arena, err);
        if error_set(err) {
            return;
        }

        match (*opts).group.obj_type {
            ObjectType::kObjectTypeNil => {}
            ObjectType::kObjectTypeString => {
                au_group = augroup_find((*opts).group.data.string.data);
                if au_group == AUGROUP_ERROR {
                    api_err_inv(err, c"group".as_ptr(), (*opts).group.data.string.data);
                    return;
                }
            }
            ObjectType::kObjectTypeInteger => {
                au_group = (*opts).group.data.integer as c_int;
                let name = if au_group == 0 {
                    std::ptr::null()
                } else {
                    augroup_name(au_group)
                };
                if !augroup_exists(name) {
                    api_err_int(err, c"group".as_ptr(), i64::from(au_group));
                    return;
                }
            }
            _ => {
                api_err_exp(
                    err,
                    c"group".as_ptr(),
                    c"String or Integer".as_ptr(),
                    api_typename((*opts).group.obj_type),
                );
                return;
            }
        }

        let mask = (*opts).is_set__exec_autocmds_;
        let has_buf_key = has_key(mask, KEYSET_OPTIDX_exec_autocmds__buf);
        let has_buffer_key = has_key(mask, KEYSET_OPTIDX_exec_autocmds__buffer);
        let has_buf = has_buf_key || has_buffer_key;
        let buf = if has_buf_key {
            (*opts).buf
        } else {
            (*opts).buffer
        };

        if has_buf_key && has_buffer_key {
            api_err_con(err, c"buf".as_ptr(), c"buffer".as_ptr());
            return;
        }

        if has_buf {
            if has_key(mask, KEYSET_OPTIDX_exec_autocmds__pattern) {
                api_err_con(err, c"pattern".as_ptr(), c"buf".as_ptr());
                return;
            }
            b = find_buffer_by_handle(buf, err);
            if error_set(err) {
                return;
            }
        }

        let patterns = get_patterns_from_pattern_or_buf(
            (*opts).pattern,
            has_buf,
            buf,
            c"".as_ptr(),
            arena,
            err,
        );
        if error_set(err) {
            return;
        }

        if has_key(mask, KEYSET_OPTIDX_exec_autocmds__data) {
            data = &mut (*opts).data;
        }

        let modeline = if has_key(mask, KEYSET_OPTIDX_exec_autocmds__modeline) {
            (*opts).modeline
        } else {
            true
        };

        let mut did_aucmd = false;
        for i in 0..event_array.size {
            let event_str = *event_array.items.add(i);
            let event_nr = event_name2nr_str(event_str.data.string);
            if (event_nr as usize) >= NUM_EVENTS {
                api_err_inv(err, c"event".as_ptr(), event_str.data.string.data);
                return;
            }
            for j in 0..patterns.size {
                let pat = *patterns.items.add(j);
                let fname = if !has_buf {
                    pat.data.string.data
                } else {
                    std::ptr::null_mut()
                };
                did_aucmd |= apply_autocmds_group(
                    event_nr,
                    fname,
                    std::ptr::null_mut(),
                    true,
                    au_group,
                    b,
                    curwin,
                    std::ptr::null_mut(),
                    data,
                    has_buf,
                );
            }
        }

        if did_aucmd && modeline && !has_buf {
            do_modelines(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_key_checks_bitmask() {
        let mask = (1u64 << KEYSET_OPTIDX_create_autocmd__once)
            | (1u64 << KEYSET_OPTIDX_create_autocmd__callback);
        assert!(has_key(mask, KEYSET_OPTIDX_create_autocmd__once));
        assert!(has_key(mask, KEYSET_OPTIDX_create_autocmd__callback));
        assert!(!has_key(mask, KEYSET_OPTIDX_create_autocmd__command));
    }
}
