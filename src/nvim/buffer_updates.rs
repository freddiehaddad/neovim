//! Buffer update event notifications and Lua callbacks (`src/nvim/buffer_updates.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::globals::*;
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::api::buffer::buf_collect_lines;
use crate::api::private::validate::{Array, Error, Object, ObjectData, ObjectType};
use crate::eval::deprecated::{BufUpdateCallbacks, buf_T};
use crate::lua::converter::{Boolean, LuaRef};
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::msgpack_rpc::packer::{Integer, handle_T};

const ARENA_EMPTY: Arena = Arena {
    cur_blk: ptr::null_mut(),
    pos: 0,
    size: 0,
};

pub type linenr_T = i32;
pub type colnr_T = c_int;
pub type bcount_t = isize;
pub type LuaRetMode = c_int;

const INTERNAL_CALL_MASK: u64 = 1u64 << 63;
const VIML_INTERNAL_CALL: u64 = INTERNAL_CALL_MASK;
const LUA_INTERNAL_CALL: u64 = VIML_INTERNAL_CALL + 1;

const LUA_NOREF: LuaRef = -2;
const kRetNilBool: LuaRetMode = 1;
const LOG_ERROR: c_int = 3;

unsafe extern "C" {

    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn rpc_send_event(id: u64, name: *const c_char, args: Array) -> bool;
    fn nlua_call_ref(
        ref_id: LuaRef,
        name: *const c_char,
        args: Array,
        mode: LuaRetMode,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn ml_flush_deleted_bytes(
        buf: *mut buf_T,
        codepoints: *mut usize,
        codeunits: *mut usize,
    ) -> usize;
    fn api_free_luaref(ref_id: LuaRef);
    fn logmsg(
        log_level: c_int,
        context: *const c_char,
        func_name: *const c_char,
        line_num: c_int,
        eol: bool,
        fmt: *const c_char,
        ...
    ) -> bool;
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
const fn buffer_obj(b: handle_T) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBuffer,
        data: ObjectData {
            integer: b as Integer,
        },
    }
}

#[inline]
const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
unsafe fn buf_get_changedtick(buf: *const buf_T) -> i64 {
    unsafe { (*buf).changedtick_di.di_tv.vval.v_number }
}

#[inline]
unsafe fn luaret_truthy(res: Object) -> bool {
    res.obj_type == ObjectType::kObjectTypeBoolean && unsafe { res.data.boolean }
}

unsafe fn textlock_wrap<F: FnOnce()>(f: F) {
    unsafe {
        let save_cursor = (*curwin).w_cursor;
        textlock += 1;
        f();
        textlock -= 1;
        (*curwin).w_cursor = save_cursor;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_register(
    buf: *mut buf_T,
    channel_id: u64,
    cb: BufUpdateCallbacks,
    send_buffer: bool,
) -> bool {
    unsafe {
        if (*buf).b_ml.ml_mfp.is_null() {
            return false;
        }

        if channel_id == LUA_INTERNAL_CALL {
            (*buf).update_callbacks.push(cb);
            if cb.utf_sizes {
                (*buf).update_need_codepoints = true;
            }
            return true;
        }

        let size = (*buf).update_channels.size;
        for i in 0..size {
            if *(*buf).update_channels.items.add(i) == channel_id {
                return true;
            }
        }

        (*buf).update_channels.push(channel_id);

        if send_buffer {
            let line_count = (*buf).b_ml.ml_line_count as usize;
            let mut linedata = Array::new();
            let mut arena = ARENA_EMPTY;
            if line_count > 0 {
                linedata = arena_array(&mut arena, line_count);
                buf_collect_lines(
                    buf,
                    line_count,
                    1,
                    0,
                    true,
                    &mut linedata,
                    ptr::null_mut(),
                    &mut arena,
                );
            }

            let mut items = [
                buffer_obj((*buf).b_fnum),
                integer_obj(buf_get_changedtick(buf)),
                integer_obj(0),
                integer_obj(-1),
                array_obj(linedata),
                boolean_obj(false),
            ];
            let args = Array {
                size: 6,
                capacity: 6,
                items: items.as_mut_ptr(),
            };

            rpc_send_event(channel_id, c"nvim_buf_lines_event".as_ptr(), args);
            arena_mem_free(arena_finish(&mut arena));
        } else {
            buf_updates_changedtick_single(buf, channel_id);
        }

        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_active(buf: *mut buf_T) -> bool {
    unsafe { (*buf).update_channels.size != 0 || (*buf).update_callbacks.size != 0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_send_end(buf: *mut buf_T, channelid: u64) {
    unsafe {
        let mut items = [buffer_obj((*buf).b_fnum)];
        let args = Array {
            size: 1,
            capacity: 1,
            items: items.as_mut_ptr(),
        };
        rpc_send_event(channelid, c"nvim_buf_detach_event".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_unregister(buf: *mut buf_T, channelid: u64) {
    unsafe {
        let size = (*buf).update_channels.size;
        if size == 0 {
            return;
        }

        let mut j = 0usize;
        let mut found = 0usize;
        for i in 0..size {
            if *(*buf).update_channels.items.add(i) == channelid {
                found += 1;
            } else {
                if i != j {
                    *(*buf).update_channels.items.add(j) = *(*buf).update_channels.items.add(i);
                }
                j += 1;
            }
        }

        if found > 0 {
            (*buf).update_channels.size -= found;
            buf_updates_send_end(buf, channelid);

            if found == size {
                (*buf).update_channels.destroy();
                (*buf).update_channels.init();
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_free_callbacks(buf: *mut buf_T) {
    unsafe {
        (*buf).update_channels.destroy();
        for i in 0..(*buf).update_callbacks.size {
            buffer_update_callbacks_free(*(*buf).update_callbacks.items.add(i));
        }
        (*buf).update_callbacks.destroy();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_unload(buf: *mut buf_T, can_reload: bool) {
    unsafe {
        let size = (*buf).update_channels.size;
        if size > 0 {
            for i in 0..size {
                buf_updates_send_end(buf, *(*buf).update_channels.items.add(i));
            }
            (*buf).update_channels.destroy();
            (*buf).update_channels.init();
        }

        let mut j = 0usize;
        for i in 0..(*buf).update_callbacks.size {
            let cb = *(*buf).update_callbacks.items.add(i);
            let mut thecb = LUA_NOREF;
            let mut keep = false;

            if can_reload && cb.on_reload != LUA_NOREF {
                keep = true;
                thecb = cb.on_reload;
            } else if cb.on_detach != LUA_NOREF {
                thecb = cb.on_detach;
            }

            if thecb != LUA_NOREF {
                let mut items = [buffer_obj((*buf).b_fnum)];
                let args = Array {
                    size: 1,
                    capacity: 1,
                    items: items.as_mut_ptr(),
                };

                textlock_wrap(|| {
                    nlua_call_ref(
                        thecb,
                        if keep {
                            c"reload".as_ptr()
                        } else {
                            c"detach".as_ptr()
                        },
                        args,
                        0,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                });
            }

            if keep {
                *(*buf).update_callbacks.items.add(j) = *(*buf).update_callbacks.items.add(i);
                j += 1;
            } else {
                buffer_update_callbacks_free(cb);
            }
        }
        (*buf).update_callbacks.size = j;
        if (*buf).update_callbacks.size == 0 {
            (*buf).update_callbacks.destroy();
            (*buf).update_callbacks.init();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_send_changes(
    buf: *mut buf_T,
    firstline: linenr_T,
    num_added: i64,
    num_removed: i64,
) {
    unsafe {
        let mut deleted_codepoints: usize = 0;
        let mut deleted_codeunits: usize = 0;
        let deleted_bytes =
            ml_flush_deleted_bytes(buf, &mut deleted_codepoints, &mut deleted_codeunits);

        if !buf_updates_active(buf) {
            return;
        }

        let send_tick = !(cmdpreview && buf == curbuf);
        let mut badchannelid: u64 = 0;

        let mut arena = ARENA_EMPTY;
        let mut linedata = Array::new();
        if num_added > 0 && (*buf).update_channels.size > 0 {
            linedata = arena_array(&mut arena, num_added as usize);
            buf_collect_lines(
                buf,
                num_added as usize,
                firstline,
                0,
                true,
                &mut linedata,
                ptr::null_mut(),
                &mut arena,
            );
        }

        for i in 0..(*buf).update_channels.size {
            let channelid = *(*buf).update_channels.items.add(i);
            let mut items = [
                buffer_obj((*buf).b_fnum),
                if send_tick {
                    integer_obj(buf_get_changedtick(buf))
                } else {
                    NIL
                },
                integer_obj(i64::from(firstline - 1)),
                integer_obj(i64::from(firstline - 1) + num_removed),
                array_obj(linedata),
                boolean_obj(false),
            ];
            let args = Array {
                size: 6,
                capacity: 6,
                items: items.as_mut_ptr(),
            };
            if !rpc_send_event(channelid, c"nvim_buf_lines_event".as_ptr(), args) {
                badchannelid = channelid;
            }
        }

        if badchannelid != 0 {
            logmsg(
                LOG_ERROR,
                ptr::null(),
                c"buf_updates_send_changes".as_ptr(),
                268,
                true,
                c"Disabling buffer updates for dead channel %llu".as_ptr(),
                badchannelid,
            );
            buf_updates_unregister(buf, badchannelid);
        }

        arena_mem_free(arena_finish(&mut arena));

        let mut j = 0usize;
        for i in 0..(*buf).update_callbacks.size {
            let cb = *(*buf).update_callbacks.items.add(i);
            let mut keep = true;
            if cb.on_lines != LUA_NOREF && (cb.preview || !cmdpreview) {
                let mut items = [
                    buffer_obj((*buf).b_fnum),
                    if send_tick {
                        integer_obj(buf_get_changedtick(buf))
                    } else {
                        NIL
                    },
                    integer_obj(i64::from(firstline - 1)),
                    integer_obj(i64::from(firstline - 1) + num_removed),
                    integer_obj(i64::from(firstline - 1) + num_added),
                    integer_obj(deleted_bytes as Integer),
                    integer_obj(deleted_codepoints as Integer),
                    integer_obj(deleted_codeunits as Integer),
                ];
                let args = Array {
                    size: if cb.utf_sizes { 8 } else { 6 },
                    capacity: 8,
                    items: items.as_mut_ptr(),
                };

                let mut res = NIL;
                textlock_wrap(|| {
                    res = nlua_call_ref(
                        cb.on_lines,
                        c"lines".as_ptr(),
                        args,
                        kRetNilBool,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                });

                if luaret_truthy(res) {
                    buffer_update_callbacks_free(cb);
                    keep = false;
                }
            }
            if keep {
                *(*buf).update_callbacks.items.add(j) = *(*buf).update_callbacks.items.add(i);
                j += 1;
            }
        }
        (*buf).update_callbacks.size = j;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_send_splice(
    buf: *mut buf_T,
    start_row: c_int,
    start_col: colnr_T,
    start_byte: bcount_t,
    old_row: c_int,
    old_col: colnr_T,
    old_byte: bcount_t,
    new_row: c_int,
    new_col: colnr_T,
    new_byte: bcount_t,
) {
    unsafe {
        if !buf_updates_active(buf) || (old_byte == 0 && new_byte == 0) {
            return;
        }

        let mut j = 0usize;
        for i in 0..(*buf).update_callbacks.size {
            let cb = *(*buf).update_callbacks.items.add(i);
            let mut keep = true;
            if cb.on_bytes != LUA_NOREF && (cb.preview || !cmdpreview) {
                let mut items = [
                    buffer_obj((*buf).b_fnum),
                    integer_obj(buf_get_changedtick(buf)),
                    integer_obj(i64::from(start_row)),
                    integer_obj(i64::from(start_col)),
                    integer_obj(start_byte as Integer),
                    integer_obj(i64::from(old_row)),
                    integer_obj(i64::from(old_col)),
                    integer_obj(old_byte as Integer),
                    integer_obj(i64::from(new_row)),
                    integer_obj(i64::from(new_col)),
                    integer_obj(new_byte as Integer),
                ];
                let args = Array {
                    size: 11,
                    capacity: 11,
                    items: items.as_mut_ptr(),
                };

                let mut res = NIL;
                textlock_wrap(|| {
                    res = nlua_call_ref(
                        cb.on_bytes,
                        c"bytes".as_ptr(),
                        args,
                        kRetNilBool,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                });

                if luaret_truthy(res) {
                    buffer_update_callbacks_free(cb);
                    keep = false;
                }
            }
            if keep {
                *(*buf).update_callbacks.items.add(j) = *(*buf).update_callbacks.items.add(i);
                j += 1;
            }
        }
        (*buf).update_callbacks.size = j;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_changedtick(buf: *mut buf_T) {
    unsafe {
        for i in 0..(*buf).update_channels.size {
            let channel_id = *(*buf).update_channels.items.add(i);
            buf_updates_changedtick_single(buf, channel_id);
        }

        let mut j = 0usize;
        for i in 0..(*buf).update_callbacks.size {
            let cb = *(*buf).update_callbacks.items.add(i);
            let mut keep = true;
            if cb.on_changedtick != LUA_NOREF {
                let mut items = [
                    buffer_obj((*buf).b_fnum),
                    integer_obj(buf_get_changedtick(buf)),
                ];
                let args = Array {
                    size: 2,
                    capacity: 2,
                    items: items.as_mut_ptr(),
                };

                let mut res = NIL;
                textlock_wrap(|| {
                    res = nlua_call_ref(
                        cb.on_changedtick,
                        c"changedtick".as_ptr(),
                        args,
                        kRetNilBool,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                });

                if luaret_truthy(res) {
                    buffer_update_callbacks_free(cb);
                    keep = false;
                }
            }
            if keep {
                *(*buf).update_callbacks.items.add(j) = *(*buf).update_callbacks.items.add(i);
                j += 1;
            }
        }
        (*buf).update_callbacks.size = j;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_updates_changedtick_single(buf: *mut buf_T, channel_id: u64) {
    unsafe {
        let mut items = [
            buffer_obj((*buf).b_fnum),
            integer_obj(buf_get_changedtick(buf)),
        ];
        let args = Array {
            size: 2,
            capacity: 2,
            items: items.as_mut_ptr(),
        };
        rpc_send_event(channel_id, c"nvim_buf_changedtick_event".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_update_callbacks_free(cb: BufUpdateCallbacks) {
    unsafe {
        api_free_luaref(cb.on_lines);
        api_free_luaref(cb.on_bytes);
        api_free_luaref(cb.on_changedtick);
        api_free_luaref(cb.on_reload);
        api_free_luaref(cb.on_detach);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luaret_truthy_checks_boolean_true() {
        assert!(unsafe { luaret_truthy(boolean_obj(true)) });
        assert!(!unsafe { luaret_truthy(boolean_obj(false)) });
        assert!(!unsafe { luaret_truthy(NIL) });
        assert!(!unsafe { luaret_truthy(integer_obj(1)) });
    }
}
