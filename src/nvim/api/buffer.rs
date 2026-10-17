//! Buffer API functions (`src/nvim/api/buffer.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::autocmd::{OptionalKeys, TryState};
use crate::api::deprecated::KeyDict_empty;
use crate::api::options::CtxSwitch;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_invalid,
    check_string_array,
};
use crate::api::tabpage::{Buffer, pos_T, tabpage_T, win_T};
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{BufUpdateCallbacks, buf_T, fmark_T};
use crate::kvec::KVec;
use crate::lua::base64::lua_State;
use crate::lua::converter::{Boolean, LuaRef};
use crate::map_glyph_cache::NvimString;
use crate::marktree::kMTMetaLines;
use crate::memory::{Arena, arena_alloc, arena_allocz, arena_memdupz, xfree, xmemdupz};
use crate::msgpack_rpc::packer::{Integer, handle_T};

pub type linenr_T = i32;
pub type colnr_T = c_int;
pub type bcount_t = isize;
pub type ExtmarkOp = c_int;
pub type MarkAdjustMode = c_int;
pub type MarkGet = c_int;
pub type Direction = c_int;
pub type LuaRetMode = c_int;

const OK: c_int = 1;
const FAIL: c_int = 0;
const NUL: c_char = 0;
const NL: c_char = b'\n' as c_char;
const Ctrl_V: c_int = 0x16;

const MAXLNUM: i64 = 0x7fffffff;
const MAXCOL: i64 = 0x7fffffff;

const INTERNAL_CALL_MASK: u64 = 1u64 << 63;
const VIML_INTERNAL_CALL: u64 = INTERNAL_CALL_MASK;
const LUA_INTERNAL_CALL: u64 = VIML_INTERNAL_CALL + 1;

const LUA_NOREF: LuaRef = -2;

const kExtmarkNOOP: ExtmarkOp = 0;
const kExtmarkUndo: ExtmarkOp = 1;

const kMarkAdjustApi: MarkAdjustMode = 1;
const kMarkAllNoResolve: MarkGet = 2;

const MAPTYPE_MAP: c_int = 0;
const MAPTYPE_UNMAP: c_int = 1;
const MAPTYPE_UNMAP_LHS: c_int = 3;

const DOBUF_UNLOAD: c_int = 2;
const DOBUF_WIPE: c_int = 4;
const DOBUF_FIRST: c_int = 1;
const FORWARD: Direction = 1;

const ML_LINE_DIRTY: c_int = 0x02;
const VALID_BOTLINE_AP: c_int = 0x40;
const MODE_INSERT: c_int = 0x10;

const kRetMultiStack: LuaRetMode = 4;

const KEYSET_OPTIDX_buf_attach__preview: u64 = 1;
const KEYSET_OPTIDX_buf_attach__on_bytes: u64 = 2;
const KEYSET_OPTIDX_buf_attach__on_lines: u64 = 3;
const KEYSET_OPTIDX_buf_attach__utf_sizes: u64 = 4;
const KEYSET_OPTIDX_buf_attach__on_detach: u64 = 5;
const KEYSET_OPTIDX_buf_attach__on_reload: u64 = 6;
const KEYSET_OPTIDX_buf_attach__on_changedtick: u64 = 7;

#[repr(C)]
pub struct KeyDict_buf_attach {
    pub is_set__buf_attach_: OptionalKeys,
    pub on_lines: LuaRef,
    pub on_bytes: LuaRef,
    pub on_changedtick: LuaRef,
    pub on_detach: LuaRef,
    pub on_reload: LuaRef,
    pub utf_sizes: Boolean,
    pub preview: Boolean,
}

#[repr(C)]
pub struct KeyDict_keymap {
    pub is_set__keymap_: OptionalKeys,
    pub noremap: Boolean,
    pub nowait: Boolean,
    pub silent: Boolean,
    pub script: Boolean,
    pub expr: Boolean,
    pub unique: Boolean,
    pub callback: LuaRef,
    pub desc: NvimString,
    pub replace_keycodes: Boolean,
}

#[repr(C)]
pub struct KeyDict_keymap_del {
    pub is_set__keymap_del_: OptionalKeys,
    pub lhs: Boolean,
}

#[repr(C)]
pub struct KeyDict_buf_delete {
    pub is_set__buf_delete_: OptionalKeys,
    pub force: Boolean,
    pub unload: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisualExtent {
    pub mode: c_int,
    pub line_count: linenr_T,
    pub vcol: colnr_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisualState {
    pub start: pos_T,
    pub active: bool,
    pub select: bool,
    pub select_reg: c_int,
    pub select_exclu_adj: bool,
    pub restart_select: c_int,
    pub need_end: bool,
    pub reselect: c_int,
    pub mode: c_int,
    pub resel: VisualExtent,
}

#[repr(C)]
pub struct u_header_T {
    pub uh_next: *mut u_header_T,
    pub uh_prev: *mut u_header_T,
    pub uh_alt_next: *mut u_header_T,
    pub uh_alt_prev: *mut u_header_T,
    pub uh_seq: c_int,
    pub uh_walk: c_int,
    pub uh_entry: *mut c_void,
    pub uh_getbot_entry: *mut c_void,
    pub uh_cursor: pos_T,
    pub uh_cursor_after: pos_T,
    pub uh_cursor_vcol: colnr_T,
    pub uh_flags: c_int,
    pub uh_namedm: [fmark_T; 26],
    pub uh_extmark: KVec<*mut c_void>,
}

unsafe extern "C" {
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut curtab: *mut tabpage_T;
    static mut firstwin: *mut win_T;
    static mut first_tabpage: *mut tabpage_T;
    static mut Visual: VisualState;
    static mut State: c_int;
    static mut RedrawingDisabled: c_int;
    static mut p_acd: c_int;

    fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T;
    fn buf_ensure_loaded(buf: *mut buf_T) -> bool;
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn api_clear_error(value: *mut Error);
    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict;
    fn buf_updates_register(
        buf: *mut buf_T,
        channel_id: u64,
        cb: BufUpdateCallbacks,
        send_buffer: bool,
    ) -> bool;
    fn buf_updates_unregister(buf: *mut buf_T, channel_id: u64);
    fn normalize_index(buf: *mut buf_T, index: i64, end_exclusive: bool, oob: *mut bool) -> i64;
    fn memchrsub(data: *mut c_char, c: c_char, x: c_char, len: usize);
    fn strchrsub(buf: *mut c_char, c: c_char, x: c_char);
    fn u_save_buf(buf: *mut buf_T, top: linenr_T, bot: linenr_T) -> c_int;
    fn get_region_bytecount(
        buf: *mut buf_T,
        start_lnum: linenr_T,
        end_lnum: linenr_T,
        start_col: colnr_T,
        end_col: colnr_T,
    ) -> bcount_t;
    fn ml_delete_buf(buf: *mut buf_T, lnum: linenr_T, message: bool) -> c_int;
    fn ml_replace_buf(
        buf: *mut buf_T,
        lnum: linenr_T,
        line: *mut c_char,
        copy: bool,
        noalloc: bool,
    ) -> c_int;
    fn ml_append_buf(
        buf: *mut buf_T,
        lnum: linenr_T,
        line: *mut c_char,
        len: colnr_T,
        newfile: bool,
    ) -> c_int;
    fn mark_adjust_buf(
        buf: *mut buf_T,
        line1: linenr_T,
        line2: linenr_T,
        amount: linenr_T,
        amount_after: linenr_T,
        adjust_cursor: bool,
        mode: MarkAdjustMode,
        op: ExtmarkOp,
    );
    fn check_visual_pos();
    fn extmark_splice(
        buf: *mut buf_T,
        start_row: c_int,
        start_col: colnr_T,
        old_row: c_int,
        old_col: colnr_T,
        old_byte: bcount_t,
        new_row: c_int,
        new_col: colnr_T,
        new_byte: bcount_t,
        undo: ExtmarkOp,
    );
    fn changed_lines(
        buf: *mut buf_T,
        lnum: linenr_T,
        col: colnr_T,
        lnume: linenr_T,
        xtra: linenr_T,
        do_buf_event: bool,
    );
    fn ml_get_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char;
    fn ml_get_buf_len(buf: *mut buf_T, lnum: linenr_T) -> colnr_T;
    fn buf_get_text(
        buf: *mut buf_T,
        lnum: i64,
        start_col: i64,
        end_col: i64,
        err: *mut Error,
    ) -> NvimString;
    fn ml_find_line_or_offset(
        buf: *mut buf_T,
        lnum: linenr_T,
        offp: *mut c_int,
        no_ff: bool,
    ) -> c_int;
    fn dict_get_value(
        d: *mut dict_T,
        name: NvimString,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn dict_set_var(
        dict: *mut dict_T,
        name: NvimString,
        value: Object,
        del: bool,
        retval: bool,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn keymap_array(mode: NvimString, buf: *mut buf_T, arena: *mut Arena) -> Array;
    fn modify_keymap(
        channel_id: u64,
        buffer: Buffer,
        maptype: c_int,
        mode: NvimString,
        lhs: NvimString,
        rhs: NvimString,
        opts: *mut KeyDict_keymap,
        err: *mut Error,
    );
    fn ctx_switch(
        cs: *mut CtxSwitch,
        win: *mut win_T,
        tabpage: *mut tabpage_T,
        buf: *mut buf_T,
        flags: c_int,
    ) -> bool;
    fn ctx_restore(cs: *mut CtxSwitch);
    fn rename_buffer(new_fname: *mut c_char) -> c_int;
    fn do_buffer(
        action: c_int,
        start: c_int,
        dir: Direction,
        count: handle_T,
        forceit: bool,
    ) -> c_int;
    fn mark_get(
        buf: *mut buf_T,
        win: *mut win_T,
        fmp: *mut fmark_T,
        flag: MarkGet,
        name: c_int,
    ) -> *mut fmark_T;
    fn set_mark(
        buf: *mut buf_T,
        name: NvimString,
        line: Integer,
        col: Integer,
        err: *mut Error,
    ) -> bool;
    fn nlua_call_ref(
        ref_id: LuaRef,
        name: *const c_char,
        args: Array,
        mode: LuaRetMode,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn check_cursor_lnum(win: *mut win_T);
    fn check_cursor_col(win: *mut win_T);
    fn changed_cline_bef_curs(wp: *mut win_T);
    fn update_topline(wp: *mut win_T);
    fn invalidate_botline_win(wp: *mut win_T);
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
}

unsafe extern "C-unwind" {
    fn lua_createtable(L: *mut lua_State, narr: c_int, nrec: c_int);
    fn lua_pushlstring(L: *mut lua_State, s: *const c_char, l: usize);
    fn lua_rawseti(L: *mut lua_State, idx: c_int, n: c_int);
}

const NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[inline]
const fn boolean_obj(b: Boolean) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
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
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn static_cstr_as_obj(s: &'static CStr) -> Object {
    let bytes = s.to_bytes();
    string_obj(NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    })
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        let len = unsafe { CStr::from_ptr(s).to_bytes().len() };
        NvimString {
            data: s as *mut c_char,
            size: len,
        }
    }
}

#[inline]
const fn cstr_literal(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
}

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
unsafe fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
unsafe fn api_err_range(err: *mut Error, name: *const c_char) {
    unsafe {
        api_err_invalid(err, name, c"out of range".as_ptr(), 0, false);
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
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_literal(key),
            value,
        };
        dict.size += 1;
    }
}

unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        f();
        try_leave(&tstate, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_buf_ensure_loaded(buf: Buffer, err: *mut Error) -> *mut buf_T {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return ptr::null_mut();
        }

        if (*b).b_ml.ml_mfp.is_null() && !buf_ensure_loaded(b) {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to load buffer".as_ptr(),
            );
            return ptr::null_mut();
        }

        b
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_line_count(buf: Buffer, err: *mut Error) -> Integer {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() || (*b).b_ml.ml_mfp.is_null() {
            return 0;
        }
        i64::from((*b).b_ml.ml_line_count)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_attach(
    channel_id: u64,
    buf: Buffer,
    send_buffer: Boolean,
    opts: *mut KeyDict_buf_attach,
    err: *mut Error,
) -> Boolean {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return false;
        }

        let mut cb = BufUpdateCallbacks {
            on_lines: LUA_NOREF,
            on_bytes: LUA_NOREF,
            on_changedtick: LUA_NOREF,
            on_detach: LUA_NOREF,
            on_reload: LUA_NOREF,
            utf_sizes: false,
            preview: false,
        };

        if channel_id == LUA_INTERNAL_CALL {
            let mask = (*opts).is_set__buf_attach_;
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_lines) {
                cb.on_lines = (*opts).on_lines;
                (*opts).on_lines = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_bytes) {
                cb.on_bytes = (*opts).on_bytes;
                (*opts).on_bytes = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_changedtick) {
                cb.on_changedtick = (*opts).on_changedtick;
                (*opts).on_changedtick = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_detach) {
                cb.on_detach = (*opts).on_detach;
                (*opts).on_detach = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_reload) {
                cb.on_reload = (*opts).on_reload;
                (*opts).on_reload = LUA_NOREF;
            }
            cb.utf_sizes = (*opts).utf_sizes;
            cb.preview = (*opts).preview;
        }

        buf_updates_register(b, channel_id, cb, send_buffer)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_detach(channel_id: u64, buf: Buffer, err: *mut Error) -> Boolean {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return false;
        }
        buf_updates_unregister(b, channel_id);
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_lines(
    channel_id: u64,
    buf: Buffer,
    mut start: Integer,
    mut end: Integer,
    strict_indexing: Boolean,
    arena: *mut Arena,
    lstate: *mut lua_State,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if (*b).b_ml.ml_mfp.is_null() {
            init_line_array(lstate, &mut rv, 0, arena);
            return rv;
        }

        let mut oob = false;
        start = normalize_index(b, start, true, &mut oob);
        end = normalize_index(b, end, true, &mut oob);

        if strict_indexing && oob {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return rv;
        }

        let size = if end >= start {
            (end - start) as usize
        } else {
            0
        };

        init_line_array(lstate, &mut rv, size, arena);

        buf_collect_lines(
            b,
            size,
            start as linenr_T,
            0,
            channel_id != VIML_INTERNAL_CALL,
            &mut rv,
            lstate,
            arena,
        );

        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_lines(
    channel_id: u64,
    buf: Buffer,
    mut start: Integer,
    mut end: Integer,
    strict_indexing: Boolean,
    replacement: Array,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return;
        }

        let mut oob = false;
        start = normalize_index(b, start, true, &mut oob);
        end = normalize_index(b, end, true, &mut oob);

        if strict_indexing && oob {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return;
        }
        if start > end {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start' is higher than 'end'".as_ptr(),
            );
            return;
        }

        let disallow_nl = channel_id != VIML_INTERNAL_CALL;
        if !check_string_array(
            replacement,
            c"replacement string".as_ptr().cast_mut(),
            disallow_nl,
            err,
        ) {
            return;
        }

        let new_len = replacement.size;
        let old_len = (end - start) as usize;
        let mut extra: isize = 0;
        let lines = if new_len != 0 {
            arena_alloc(arena, new_len * std::mem::size_of::<*mut c_char>(), true)
                .cast::<*mut c_char>()
        } else {
            ptr::null_mut()
        };

        for i in 0..new_len {
            let l = (*replacement.items.add(i)).data.string;
            let p = arena_memdupz(arena, l.data, l.size);
            memchrsub(p, NUL, NL, l.size);
            *lines.add(i) = p;
        }

        try_wrap(err, || 'end: {
            if (*b).b_p_ma == 0 {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Buffer is not 'modifiable'".as_ptr(),
                );
                break 'end;
            }

            if u_save_buf(b, (start - 1) as linenr_T, end as linenr_T) == FAIL {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to save undo information".as_ptr(),
                );
                break 'end;
            }

            let deleted_bytes = get_region_bytecount(b, start as linenr_T, end as linenr_T, 0, 0);

            let to_delete = old_len.saturating_sub(new_len);
            for _ in 0..to_delete {
                if ml_delete_buf(b, start as linenr_T, false) == FAIL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to delete line".as_ptr(),
                    );
                    break 'end;
                }
            }

            if to_delete > 0 {
                extra -= to_delete as isize;
            }

            let to_replace = old_len.min(new_len);
            let mut inserted_bytes: bcount_t = 0;
            for i in 0..to_replace {
                let lnum = start + (i as i64);
                if lnum >= MAXLNUM {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                let line_ptr = *lines.add(i);
                if ml_replace_buf(b, lnum as linenr_T, line_ptr, false, true) == FAIL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to replace line".as_ptr(),
                    );
                    break 'end;
                }

                inserted_bytes += CStr::from_ptr(line_ptr).to_bytes().len() as bcount_t + 1;
            }

            for i in to_replace..new_len {
                let lnum = start + (i as i64) - 1;
                if lnum >= MAXLNUM {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                let line_ptr = *lines.add(i);
                if ml_append_buf(b, lnum as linenr_T, line_ptr, 0, false) == FAIL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to insert line".as_ptr(),
                    );
                    break 'end;
                }

                inserted_bytes += CStr::from_ptr(line_ptr).to_bytes().len() as bcount_t + 1;
                extra += 1;
            }

            let adjust = if end > start { MAXLNUM as linenr_T } else { 0 };
            mark_adjust_buf(
                b,
                start as linenr_T,
                (end - 1) as linenr_T,
                adjust,
                extra as linenr_T,
                true,
                kMarkAdjustApi,
                kExtmarkNOOP,
            );

            if Visual.active && b == curbuf && Visual.start.lnum >= (start as linenr_T) {
                if Visual.start.lnum >= (end as linenr_T) {
                    Visual.start.lnum += extra as linenr_T;
                }
                check_visual_pos();
            }

            extmark_splice(
                b,
                (start as c_int) - 1,
                0,
                (end - start) as c_int,
                0,
                deleted_bytes,
                new_len as c_int,
                0,
                inserted_bytes,
                kExtmarkUndo,
            );

            changed_lines(
                b,
                start as linenr_T,
                0,
                end as linenr_T,
                extra as linenr_T,
                true,
            );

            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut win = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !win.is_null() {
                    if (*win).w_buffer == b {
                        fix_cursor(win, start as linenr_T, end as linenr_T, extra as linenr_T);
                    }
                    win = (*win).w_next;
                }
                tp = (*tp).tp_next;
            }
        });
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_text(
    channel_id: u64,
    buf: Buffer,
    mut start_row: Integer,
    mut start_col: Integer,
    mut end_row: Integer,
    mut end_col: Integer,
    mut replacement: Array,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        let mut scratch_items = [static_cstr_as_obj(c"")];
        if replacement.size == 0 {
            replacement = Array {
                size: 1,
                capacity: 1,
                items: scratch_items.as_mut_ptr(),
            };
        }

        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return;
        }

        let mut oob = false;
        start_row = normalize_index(b, start_row, false, &mut oob);
        if oob {
            api_err_range(err, c"start_row".as_ptr());
            return;
        }

        end_row = normalize_index(b, end_row, false, &mut oob);
        if oob {
            api_err_range(err, c"end_row".as_ptr());
            return;
        }

        let str_at_start_raw = ml_get_buf(b, start_row as linenr_T);
        let len_at_start = ml_get_buf_len(b, start_row as linenr_T);
        let str_at_start = arena_memdupz(arena, str_at_start_raw, len_at_start as usize);
        if start_col < 0 {
            start_col = i64::from(len_at_start) + start_col + 1;
        }
        if start_col < 0 || start_col > i64::from(len_at_start) {
            api_err_range(err, c"start_col".as_ptr());
            return;
        }

        let str_at_end_raw = ml_get_buf(b, end_row as linenr_T);
        let len_at_end = ml_get_buf_len(b, end_row as linenr_T);
        let str_at_end = arena_memdupz(arena, str_at_end_raw, len_at_end as usize);
        if end_col < 0 {
            end_col = i64::from(len_at_end) + end_col + 1;
        }
        if end_col < 0 || end_col > i64::from(len_at_end) {
            api_err_range(err, c"end_col".as_ptr());
            return;
        }

        if start_row > end_row || (end_row == start_row && start_col > end_col) {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start' is higher than 'end'".as_ptr(),
            );
            return;
        }

        let disallow_nl = channel_id != VIML_INTERNAL_CALL;
        if !check_string_array(
            replacement,
            c"replacement string".as_ptr().cast_mut(),
            disallow_nl,
            err,
        ) {
            return;
        }

        let new_len = replacement.size;
        let mut new_byte: bcount_t = 0;
        let mut old_byte: bcount_t = 0;

        if start_row == end_row {
            old_byte = (end_col - start_col) as bcount_t;
        } else {
            old_byte += (i64::from(len_at_start) - start_col) as bcount_t;
            for i in 1..(end_row - start_row) {
                let lnum = start_row + i;
                old_byte += (ml_get_buf_len(b, lnum as linenr_T) as bcount_t) + 1;
            }
            old_byte += (end_col as bcount_t) + 1;
        }

        let first_item = (*replacement.items.add(0)).data.string;
        let last_item = (*replacement.items.add(replacement.size - 1)).data.string;

        let mut firstlen = (start_col as usize) + first_item.size;
        let last_part_len = (len_at_end as usize) - (end_col as usize);
        if replacement.size == 1 {
            firstlen += last_part_len;
        }
        let first = arena_allocz(arena, firstlen).cast::<c_char>();
        let mut last: *mut c_char = ptr::null_mut();
        ptr::copy_nonoverlapping(str_at_start, first, start_col as usize);
        if first_item.size > 0 {
            ptr::copy_nonoverlapping(
                first_item.data,
                first.add(start_col as usize),
                first_item.size,
            );
        }
        memchrsub(first.add(start_col as usize), NUL, NL, first_item.size);
        if replacement.size == 1 {
            ptr::copy_nonoverlapping(
                str_at_end.add(end_col as usize),
                first.add((start_col as usize) + first_item.size),
                last_part_len,
            );
        } else {
            last = arena_allocz(arena, last_item.size + last_part_len).cast::<c_char>();
            if last_item.size > 0 {
                ptr::copy_nonoverlapping(last_item.data, last, last_item.size);
            }
            memchrsub(last, NUL, NL, last_item.size);
            ptr::copy_nonoverlapping(
                str_at_end.add(end_col as usize),
                last.add(last_item.size),
                last_part_len,
            );
        }

        let lines = arena_alloc(arena, new_len * std::mem::size_of::<*mut c_char>(), true)
            .cast::<*mut c_char>();
        *lines.add(0) = first;
        new_byte += first_item.size as bcount_t;
        for i in 1..new_len.saturating_sub(1) {
            let l = (*replacement.items.add(i)).data.string;
            let p = arena_memdupz(arena, l.data, l.size);
            memchrsub(p, NUL, NL, l.size);
            *lines.add(i) = p;
            new_byte += (l.size as bcount_t) + 1;
        }
        if replacement.size > 1 {
            *lines.add(replacement.size - 1) = last;
            new_byte += (last_item.size as bcount_t) + 1;
        }

        try_wrap(err, || 'end: {
            if (*b).b_p_ma == 0 {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Buffer is not 'modifiable'".as_ptr(),
                );
                break 'end;
            }

            if u_save_buf(b, (start_row as linenr_T) - 1, (end_row as linenr_T) + 1) == FAIL {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to save undo information".as_ptr(),
                );
                break 'end;
            }

            let mut extra: isize = 0;
            let old_len = (end_row - start_row + 1) as usize;

            let to_delete = old_len.saturating_sub(new_len);
            for _ in 0..to_delete {
                if ml_delete_buf(b, start_row as linenr_T, false) == FAIL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to delete line".as_ptr(),
                    );
                    break 'end;
                }
            }

            if to_delete > 0 {
                extra -= to_delete as isize;
            }

            let to_replace = old_len.min(new_len);
            for i in 0..to_replace {
                let lnum = start_row + (i as i64);
                if lnum >= MAXLNUM {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                if ml_replace_buf(b, lnum as linenr_T, *lines.add(i), false, true) == FAIL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to replace line".as_ptr(),
                    );
                    break 'end;
                }
            }

            for i in to_replace..new_len {
                let lnum = start_row + (i as i64) - 1;
                if lnum >= MAXLNUM {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                if ml_append_buf(b, lnum as linenr_T, *lines.add(i), 0, false) == FAIL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to insert line".as_ptr(),
                    );
                    break 'end;
                }

                extra += 1;
            }

            let col_extent =
                (end_col - if end_row == start_row { start_col } else { 0 }) as colnr_T;

            let adjust = if end_row >= start_row {
                MAXLNUM as linenr_T
            } else {
                0
            };
            mark_adjust_buf(
                b,
                start_row as linenr_T,
                (end_row as linenr_T) - 1,
                adjust,
                extra as linenr_T,
                true,
                kMarkAdjustApi,
                kExtmarkNOOP,
            );

            if Visual.active && b == curbuf && Visual.mode != Ctrl_V {
                fix_pos_col(
                    b,
                    &raw mut Visual.start,
                    start_row as linenr_T,
                    start_col as colnr_T,
                    end_row as linenr_T,
                    end_col as colnr_T,
                    new_len as linenr_T,
                    last_item.size as colnr_T,
                    1,
                );
                check_visual_pos();
            }

            extmark_splice(
                b,
                (start_row as c_int) - 1,
                start_col as colnr_T,
                (end_row - start_row) as c_int,
                col_extent,
                old_byte,
                (new_len as c_int) - 1,
                last_item.size as colnr_T,
                new_byte,
                kExtmarkUndo,
            );

            changed_lines(
                b,
                start_row as linenr_T,
                start_col as colnr_T,
                (end_row as linenr_T) + 1,
                extra as linenr_T,
                true,
            );

            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut win = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !win.is_null() {
                    if (*win).w_buffer == b {
                        if i64::from((*win).w_cursor.lnum) >= start_row
                            && i64::from((*win).w_cursor.lnum) <= end_row
                        {
                            fix_cursor_cols(
                                win,
                                start_row as linenr_T,
                                start_col as colnr_T,
                                end_row as linenr_T,
                                end_col as colnr_T,
                                new_len as linenr_T,
                                last_item.size as colnr_T,
                            );
                        } else {
                            fix_cursor(
                                win,
                                start_row as linenr_T,
                                end_row as linenr_T,
                                extra as linenr_T,
                            );
                        }
                    }
                    win = (*win).w_next;
                }
                tp = (*tp).tp_next;
            }
        });
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_text(
    channel_id: u64,
    buf: Buffer,
    mut start_row: Integer,
    start_col: Integer,
    mut end_row: Integer,
    end_col: Integer,
    _opts: *mut KeyDict_empty,
    arena: *mut Arena,
    lstate: *mut lua_State,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if (*b).b_ml.ml_mfp.is_null() {
            init_line_array(lstate, &mut rv, 0, arena);
            return rv;
        }

        let mut oob = false;
        start_row = normalize_index(b, start_row, false, &mut oob);
        end_row = normalize_index(b, end_row, false, &mut oob);

        if oob {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return rv;
        }

        if start_row > end_row {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start' is higher than 'end'".as_ptr(),
            );
            return rv;
        }

        let replace_nl = channel_id != VIML_INTERNAL_CALL;
        let size = ((end_row - start_row) as usize) + 1;

        init_line_array(lstate, &mut rv, size, arena);

        if start_row == end_row {
            let line = buf_get_text(b, start_row, start_col, end_col, err);
            if error_set(err) {
                return Array::new();
            }
            push_linestr(lstate, &mut rv, line.data, line.size, 0, replace_nl, arena);
            return rv;
        }

        let mut str_val = buf_get_text(b, start_row, start_col, MAXCOL - 1, err);
        if error_set(err) {
            return Array::new();
        }

        push_linestr(
            lstate,
            &mut rv,
            str_val.data,
            str_val.size,
            0,
            replace_nl,
            arena,
        );

        if size > 2 {
            buf_collect_lines(
                b,
                size - 2,
                (start_row as linenr_T) + 1,
                1,
                replace_nl,
                &mut rv,
                lstate,
                arena,
            );
        }

        str_val = buf_get_text(b, end_row, 0, end_col, err);
        if error_set(err) {
            return Array::new();
        }

        push_linestr(
            lstate,
            &mut rv,
            str_val.data,
            str_val.size,
            (size - 1) as c_int,
            replace_nl,
            arena,
        );

        if error_set(err) {
            return Array::new();
        }

        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_offset(
    buf: Buffer,
    index: Integer,
    err: *mut Error,
) -> Integer {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return 0;
        }

        if (*b).b_ml.ml_mfp.is_null() {
            return -1;
        }

        if index < 0 || index > i64::from((*b).b_ml.ml_line_count) {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return 0;
        }

        i64::from(ml_find_line_or_offset(
            b,
            (index as linenr_T) + 1,
            ptr::null_mut(),
            true,
        ))
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_var(
    buf: Buffer,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return NIL;
        }
        dict_get_value((*b).b_vars, name, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_changedtick(buf: Buffer, err: *mut Error) -> Integer {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return -1;
        }
        (*b).changedtick_di.di_tv.vval.v_number
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_keymap(
    buf: Buffer,
    mode: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return Array::new();
        }
        keymap_array(mode, b, arena)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_keymap(
    channel_id: u64,
    buf: Buffer,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: *mut KeyDict_keymap,
    err: *mut Error,
) {
    unsafe {
        modify_keymap(channel_id, buf, MAPTYPE_MAP, mode, lhs, rhs, opts, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_del_keymap(
    channel_id: u64,
    buf: Buffer,
    mode: NvimString,
    lhs: NvimString,
    opts: *mut KeyDict_keymap_del,
    err: *mut Error,
) {
    unsafe {
        let rhs = NvimString {
            data: c"".as_ptr() as *mut c_char,
            size: 0,
        };
        let maptype = if !opts.is_null() && (*opts).lhs {
            MAPTYPE_UNMAP_LHS
        } else {
            MAPTYPE_UNMAP
        };
        modify_keymap(
            channel_id,
            buf,
            maptype,
            mode,
            lhs,
            rhs,
            ptr::null_mut(),
            err,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_var(
    buf: Buffer,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        dict_set_var((*b).b_vars, name, value, false, false, ptr::null_mut(), err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_del_var(buf: Buffer, name: NvimString, err: *mut Error) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        dict_set_var((*b).b_vars, name, NIL, true, false, ptr::null_mut(), err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_name(buf: Buffer, err: *mut Error) -> NvimString {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() || (*b).b_ffname.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        cstr_as_string((*b).b_ffname)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_name(buf: Buffer, name: NvimString, err: *mut Error) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }

        let mut ren_ret = OK;
        try_wrap(err, || {
            let is_curbuf = b == curbuf;
            let save_acd = p_acd;
            if !is_curbuf {
                RedrawingDisabled += 1;
                p_acd = 0;
            }

            let mut aco: CtxSwitch = std::mem::zeroed();
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), b.cast(), 0);
            ren_ret = rename_buffer(name.data);
            ctx_restore(&mut aco);

            if !is_curbuf {
                RedrawingDisabled -= 1;
                p_acd = save_acd;
            }
        });

        if error_set(err) {
            return;
        }

        if ren_ret == FAIL {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to rename buffer".as_ptr(),
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_is_loaded(buf: Buffer) -> Boolean {
    unsafe {
        let mut stub = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let b = find_buffer_by_handle(buf, &mut stub);
        api_clear_error(&mut stub);
        !b.is_null() && !(*b).b_ml.ml_mfp.is_null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_delete(
    buf: Buffer,
    opts: *mut KeyDict_buf_delete,
    err: *mut Error,
) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if error_set(err) {
            return;
        }

        let force = (*opts).force;
        let unload = (*opts).unload;

        let result = do_buffer(
            if unload { DOBUF_UNLOAD } else { DOBUF_WIPE },
            DOBUF_FIRST,
            FORWARD,
            (*b).b_fnum,
            force,
        );

        if result == FAIL {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to unload buffer.".as_ptr(),
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_is_valid(buf: Buffer) -> Boolean {
    unsafe {
        let mut stub = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let ret = !find_buffer_by_handle(buf, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_del_mark(
    buf: Buffer,
    name: NvimString,
    err: *mut Error,
) -> Boolean {
    unsafe {
        let mut res = false;
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return res;
        }

        if name.size != 1 {
            api_err_invalid(
                err,
                c"mark name (must be a single char)".as_ptr(),
                name.data,
                0,
                true,
            );
            return res;
        }

        let fm = mark_get(
            b,
            curwin,
            ptr::null_mut(),
            kMarkAllNoResolve,
            c_int::from(*name.data.cast::<u8>()),
        );
        if fm.is_null() {
            api_err_invalid(err, c"mark name".as_ptr(), name.data, 0, true);
            return res;
        }

        if (*fm).mark.lnum != 0 && (*fm).fnum == (*b).b_fnum {
            res = set_mark(b, name, 0, 0, err);
        }

        res
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_mark(
    buf: Buffer,
    name: NvimString,
    line: Integer,
    col: Integer,
    _opts: *mut KeyDict_empty,
    err: *mut Error,
) -> Boolean {
    unsafe {
        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return false;
        }

        if name.size != 1 {
            api_err_invalid(
                err,
                c"mark name (must be a single char)".as_ptr(),
                name.data,
                0,
                true,
            );
            return false;
        }

        set_mark(b, name, line, col, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_mark(
    buf: Buffer,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if name.size != 1 {
            api_err_invalid(
                err,
                c"mark name (must be a single char)".as_ptr(),
                name.data,
                0,
                true,
            );
            return rv;
        }

        let mark = c_int::from(*name.data.cast::<u8>());
        let fm = mark_get(b, curwin, ptr::null_mut(), kMarkAllNoResolve, mark);
        if fm.is_null() {
            api_err_invalid(err, c"mark name".as_ptr(), name.data, 0, true);
            return rv;
        }

        let (lnum, col) = if (*fm).fnum != (*b).b_fnum {
            (0, 0)
        } else {
            ((*fm).mark.lnum, (*fm).mark.col)
        };

        rv = arena_array(arena, 2);
        add_c(&mut rv, integer_obj(i64::from(lnum)));
        add_c(&mut rv, integer_obj(i64::from(col)));
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_call(
    buf: Buffer,
    fn_ref: LuaRef,
    _lstate: *mut lua_State,
    err: *mut Error,
) -> Object {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return NIL;
        }

        try_wrap(err, || {
            let mut cs: CtxSwitch = std::mem::zeroed();
            ctx_switch(&mut cs, ptr::null_mut(), ptr::null_mut(), b.cast(), 0);

            let args = Array::new();
            nlua_call_ref(
                fn_ref,
                ptr::null(),
                args,
                kRetMultiStack,
                ptr::null_mut(),
                err,
            );

            ctx_restore(&mut cs);
        });

        NIL
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__buf_stats(buf: Buffer, arena: *mut Arena, err: *mut Error) -> Dict {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return Dict::new();
        }

        let mut rv = arena_dict(arena, 7);
        put_c(
            &mut rv,
            c"flush_count",
            integer_obj(i64::from((*b).flush_count)),
        );
        put_c(
            &mut rv,
            c"current_lnum",
            integer_obj(i64::from((*b).b_ml.ml_line_lnum)),
        );
        put_c(
            &mut rv,
            c"line_dirty",
            boolean_obj(((*b).b_ml.ml_flags & ML_LINE_DIRTY) != 0),
        );
        put_c(
            &mut rv,
            c"dirty_bytes",
            integer_obj((*b).deleted_bytes as Integer),
        );
        put_c(
            &mut rv,
            c"dirty_bytes2",
            integer_obj((*b).deleted_bytes2 as Integer),
        );
        put_c(
            &mut rv,
            c"virt_blocks",
            integer_obj((*b).b_marktree[0].meta_root[kMTMetaLines] as Integer),
        );

        let uhp = if !(*b).b_u_curhead.is_null() {
            (*b).b_u_curhead.cast::<u_header_T>()
        } else {
            (*b).b_u_newhead.cast::<u_header_T>()
        };
        if !uhp.is_null() {
            put_c(
                &mut rv,
                c"uhp_extmark_size",
                integer_obj((*uhp).uh_extmark.size as Integer),
            );
        }

        rv
    }
}

unsafe fn fix_cursor(win: *mut win_T, lo: linenr_T, hi: linenr_T, extra: linenr_T) {
    unsafe {
        if (*win).w_cursor.lnum >= lo {
            if (*win).w_cursor.lnum >= hi {
                (*win).w_cursor.lnum += extra;
            } else if extra < 0 {
                check_cursor_lnum(win);
            }
            check_cursor_col(win);
            changed_cline_bef_curs(win);
            (*win).w_valid &= !VALID_BOTLINE_AP;
            update_topline(win);
        } else {
            invalidate_botline_win(win);
        }
    }
}

unsafe fn fix_pos_col(
    buf: *mut buf_T,
    pos: *mut pos_T,
    start_row: linenr_T,
    start_col: colnr_T,
    end_row: linenr_T,
    end_col: colnr_T,
    new_rows: linenr_T,
    new_cols_at_end_row: colnr_T,
    mode_col_adj: colnr_T,
) {
    unsafe {
        if (*pos).lnum < start_row {
            return;
        }

        let old_rows = end_row - start_row + 1;
        let lnum_shift = new_rows - old_rows;

        if (*pos).lnum > end_row {
            (*pos).lnum += lnum_shift;
            return;
        }

        let end_row_change_start = if new_rows == 1 { start_col } else { 0 };
        let end_row_change_end = end_row_change_start + new_cols_at_end_row;

        if (*pos).lnum == end_row && (*pos).col + mode_col_adj > end_col {
            (*pos).lnum += lnum_shift;
            (*pos).col += end_row_change_end - end_col;
            return;
        }

        let old_coladd = (*pos).coladd;
        (*pos).col += (*pos).coladd;
        (*pos).coladd = 0;

        let new_end_row = start_row + new_rows - 1;

        if (*pos).lnum > new_end_row {
            (*pos).lnum = new_end_row;
            let len = ml_get_buf_len(buf, new_end_row);
            if (*pos).col < len {
                (*pos).col = len;
            }
        }

        if (*pos).lnum == new_end_row && (*pos).col > end_row_change_end && old_coladd == 0 {
            (*pos).col = end_row_change_end;
            if (*pos).col - mode_col_adj >= end_row_change_start {
                (*pos).col -= mode_col_adj;
            }
        }
    }
}

unsafe fn fix_cursor_cols(
    win: *mut win_T,
    start_row: linenr_T,
    start_col: colnr_T,
    end_row: linenr_T,
    end_col: colnr_T,
    new_rows: linenr_T,
    new_cols_at_end_row: colnr_T,
) {
    unsafe {
        let mode_col_adj = if win == curwin && (State & MODE_INSERT) != 0 {
            0
        } else {
            1
        };
        fix_pos_col(
            (*win).w_buffer,
            &mut (*win).w_cursor,
            start_row,
            start_col,
            end_row,
            end_col,
            new_rows,
            new_cols_at_end_row,
            mode_col_adj,
        );

        check_cursor_col(win);
        changed_cline_bef_curs(win);
        invalidate_botline_win(win);
    }
}

#[inline]
unsafe fn init_line_array(lstate: *mut lua_State, a: *mut Array, size: usize, arena: *mut Arena) {
    unsafe {
        if !lstate.is_null() {
            lua_createtable(lstate, size as c_int, 0);
        } else {
            *a = arena_array(arena, size);
        }
    }
}

unsafe fn push_linestr(
    lstate: *mut lua_State,
    a: *mut Array,
    s: *const c_char,
    len: usize,
    idx: c_int,
    replace_nl: bool,
    arena: *mut Arena,
) {
    unsafe {
        if !lstate.is_null() {
            if !s.is_null() && replace_nl && CStr::from_ptr(s).to_bytes().contains(&b'\n') {
                let tmp = xmemdupz(s.cast(), len).cast::<c_char>();
                strchrsub(tmp, NL, NUL);
                lua_pushlstring(lstate, tmp, len);
                xfree(tmp.cast());
            } else {
                lua_pushlstring(lstate, s, len);
            }
            lua_rawseti(lstate, -2, idx + 1);
        } else {
            let mut str_val = NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
            if len > 0 {
                let data = arena_memdupz(arena, s, len);
                if replace_nl {
                    strchrsub(data, NL, NUL);
                }
                str_val = NvimString { data, size: len };
            }
            add_c(&mut *a, string_obj(str_val));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_collect_lines(
    buf: *mut buf_T,
    n: usize,
    start: linenr_T,
    start_idx: c_int,
    replace_nl: bool,
    l: *mut Array,
    lstate: *mut lua_State,
    arena: *mut Arena,
) {
    unsafe {
        for i in 0..n {
            let lnum = start + (i as linenr_T);
            let bufstr = ml_get_buf(buf, lnum);
            let bufstrlen = ml_get_buf_len(buf, lnum) as usize;
            push_linestr(
                lstate,
                l,
                bufstr,
                bufstrlen,
                start_idx + (i as c_int),
                replace_nl,
                arena,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_call_constants_and_boolean_obj() {
        assert_eq!(VIML_INTERNAL_CALL, 1u64 << 63);
        assert_eq!(LUA_INTERNAL_CALL, (1u64 << 63) + 1);
        let obj = boolean_obj(true);
        assert_eq!(obj.obj_type, ObjectType::kObjectTypeBoolean);
        assert!(unsafe { obj.data.boolean });
    }
}
