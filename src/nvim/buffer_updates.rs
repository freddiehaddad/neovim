//! Buffer update event notifications and Lua callbacks (`src/nvim/buffer_updates.c`).

use crate::api::buffer::{LUA_INTERNAL_CALL, buf_collect_lines};
use crate::api::private::helpers::{
    OBJECT_NIL, arena_array, array_obj, boolean_obj, buffer_obj, integer_obj,
};
use crate::api::private::validate::{Array, Object, ObjectType};
use crate::buffer::{BufUpdateCallbacks, buf_T, buf_get_changedtick};
use crate::extmark::bcount_t;
use crate::globals::*;
use crate::log::logmsg;
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::{kRetNilBool, nlua_call_ref};
use crate::memline::ml_flush_deleted_bytes;
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::msgpack_rpc::packer::Integer;
use std::ffi::c_int;
use std::ptr;

pub(crate) const ARENA_EMPTY: Arena = Arena {
    cur_blk: ptr::null_mut(),
    pos: 0,
    size: 0,
};

use crate::cursor::{colnr_T, linenr_T};

const LOG_ERROR: c_int = 3;

#[inline]
pub fn luaret_truthy(res: Object) -> bool {
    res.obj_type == ObjectType::kObjectTypeBoolean && res.data.boolean()
}

fn textlock_wrap<F: FnOnce()>(f: F) {
     {
        let save_cursor = curwin_mut().w_cursor;
        g_st().textlock += 1;
        f();
        g_st().textlock -= 1;
        curwin_mut().w_cursor = save_cursor;
    }
}

pub fn buf_updates_register(
    buf: &mut buf_T,
    channel_id: u64,
    cb: BufUpdateCallbacks,
    send_buffer: bool,
) -> bool {
    if buf.b_ml.ml_mfp.is_none() {
            return false;
        }

        if channel_id == LUA_INTERNAL_CALL {
            let utf_sizes = cb.utf_sizes;
            buf.update_callbacks.push(cb);
            if utf_sizes {
                buf.update_need_codepoints = true;
            }
            return true;
        }

        if buf.update_channels.contains(&channel_id) {
            return true;
        }

        buf.update_channels.push(channel_id);

        if send_buffer {
            let line_count = buf.b_ml.ml_line_count as usize;
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
                buffer_obj(buf.b_fnum),
                integer_obj(buf_get_changedtick(&*buf)),
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

            rpc_send_event(channel_id, c"nvim_buf_lines_event", args);
            arena_mem_free(arena_finish(&mut arena));
        } else {
            buf_updates_changedtick_single(buf, channel_id);
        }

        true
}

pub fn buf_updates_active(buf: &mut buf_T) -> bool {
    !buf.update_channels.is_empty() || !buf.update_callbacks.is_empty()
}

pub fn buf_updates_send_end(buf: &mut buf_T, channelid: u64) {
    let mut items = [buffer_obj(buf.b_fnum)];
        let args = Array {
            size: 1,
            capacity: 1,
            items: items.as_mut_ptr(),
        };
        rpc_send_event(channelid, c"nvim_buf_detach_event", args);
}

pub fn buf_updates_unregister(buf: &mut buf_T, channelid: u64) {
    let size = buf.update_channels.len();
        if size == 0 {
            return;
        }

        buf.update_channels.retain(|&id| id != channelid);
        if buf.update_channels.len() < size {
            buf_updates_send_end(buf, channelid);
            if buf.update_channels.is_empty() {
                buf.update_channels = Vec::new();
            }
        }
}

pub fn buf_free_callbacks(buf: &mut buf_T) {
    buf.update_channels = Vec::new();
        buf.update_callbacks = Vec::new();
}

pub fn buf_updates_unload(buf: &mut buf_T, can_reload: bool) {
    for channelid in std::mem::take(&mut buf.update_channels) {
            buf_updates_send_end(buf, channelid);
        }

        let mut i = 0usize;
        while i < buf.update_callbacks.len() {
            let on_reload = buf.update_callbacks.as_slice()[i].on_reload;
            let on_detach = buf.update_callbacks.as_slice()[i].on_detach;
            let mut thecb = LUA_NOREF;
            let mut keep = false;

            if can_reload && on_reload != LUA_NOREF {
                keep = true;
                thecb = on_reload;
            } else if on_detach != LUA_NOREF {
                thecb = on_detach;
            }

            if thecb != LUA_NOREF {
                let mut items = [buffer_obj(buf.b_fnum)];
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
                        None,
                        None,
                    );
                });
            }

            if keep {
                i += 1;
            } else {
                buf.update_callbacks.remove(i);
            }
        }
        if buf.update_callbacks.is_empty() {
            buf.update_callbacks = Vec::new();
        }
}

pub fn buf_updates_send_changes(
    buf: &mut buf_T,
    firstline: linenr_T,
    num_added: i64,
    num_removed: i64,
) {
     {
        let mut deleted_codepoints: usize = 0;
        let mut deleted_codeunits: usize = 0;
        let deleted_bytes =
            ml_flush_deleted_bytes(buf, &mut deleted_codepoints, &mut deleted_codeunits);

        if !buf_updates_active(buf) {
            return;
        }

        let send_tick = !(g_st().cmdpreview && (&raw mut *buf) == g_st().curbuf);
        let mut badchannelid: u64 = 0;

        let mut arena = ARENA_EMPTY;
        let mut linedata = Array::new();
        if num_added > 0 && !buf.update_channels.is_empty() {
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

        for &channelid in &buf.update_channels {
            let mut items = [
                buffer_obj(buf.b_fnum),
                if send_tick {
                    integer_obj(buf_get_changedtick(buf))
                } else {
                    OBJECT_NIL
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
            if !rpc_send_event(channelid, c"nvim_buf_lines_event", args) {
                badchannelid = channelid;
            }
        }

        if badchannelid != 0 {
            logmsg!(
                LOG_ERROR,
                ptr::null(),
                c"buf_updates_send_changes",
                268,
                true,
                c"Disabling buffer updates for dead channel %llu",
                badchannelid,
            );
            buf_updates_unregister(buf, badchannelid);
        }

        arena_mem_free(arena_finish(&mut arena));

        let mut i = 0usize;
        while i < buf.update_callbacks.len() {
            let on_lines = buf.update_callbacks.as_slice()[i].on_lines;
            let preview = buf.update_callbacks.as_slice()[i].preview;
            let utf_sizes = buf.update_callbacks.as_slice()[i].utf_sizes;
            let mut keep = true;
            if on_lines != LUA_NOREF && (preview || !g_st().cmdpreview) {
                let mut items = [
                    buffer_obj(buf.b_fnum),
                    if send_tick {
                        integer_obj(buf_get_changedtick(buf))
                    } else {
                        OBJECT_NIL
                    },
                    integer_obj(i64::from(firstline - 1)),
                    integer_obj(i64::from(firstline - 1) + num_removed),
                    integer_obj(i64::from(firstline - 1) + num_added),
                    integer_obj(deleted_bytes as Integer),
                    integer_obj(deleted_codepoints as Integer),
                    integer_obj(deleted_codeunits as Integer),
                ];
                let args = Array {
                    size: if utf_sizes { 8 } else { 6 },
                    capacity: 8,
                    items: items.as_mut_ptr(),
                };

                let mut res = OBJECT_NIL;
                textlock_wrap(|| {
                    res = nlua_call_ref(
                        on_lines,
                        c"lines",
                        args,
                        kRetNilBool,
                        None,
                        None,
                    );
                });

                if luaret_truthy(res) {
                    keep = false;
                }
            }
            if keep {
                i += 1;
            } else {
                buf.update_callbacks.remove(i);
            }
        }
    }
}

pub fn buf_updates_send_splice(buf: &mut buf_T,
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
     {
        if !buf_updates_active(buf) || (old_byte == 0 && new_byte == 0) {
            return;
        }

        let mut i = 0usize;
        while i < buf.update_callbacks.len() {
            let on_bytes = buf.update_callbacks.as_slice()[i].on_bytes;
            let preview = buf.update_callbacks.as_slice()[i].preview;
            let mut keep = true;
            if on_bytes != LUA_NOREF && (preview || !g_st().cmdpreview) {
                let mut items = [
                    buffer_obj(buf.b_fnum),
                    integer_obj(buf_get_changedtick(&*buf)),
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

                let mut res = OBJECT_NIL;
                textlock_wrap(|| {
                    res = nlua_call_ref(
                        on_bytes,
                        c"bytes",
                        args,
                        kRetNilBool,
                        None,
                        None,
                    );
                });

                if luaret_truthy(res) {
                    keep = false;
                }
            }
            if keep {
                i += 1;
            } else {
                buf.update_callbacks.remove(i);
            }
        }
    }
}

pub fn buf_updates_changedtick(buf: &mut buf_T) {
    for idx in 0..buf.update_channels.len() {
            let channel_id = buf.update_channels[idx];
            buf_updates_changedtick_single(buf, channel_id);
        }

        let mut i = 0usize;
        while i < buf.update_callbacks.len() {
            let on_changedtick = buf.update_callbacks.as_slice()[i].on_changedtick;
            let mut keep = true;
            if on_changedtick != LUA_NOREF {
                let mut items = [
                    buffer_obj(buf.b_fnum),
                    integer_obj(buf_get_changedtick(&*buf)),
                ];
                let args = Array {
                    size: 2,
                    capacity: 2,
                    items: items.as_mut_ptr(),
                };

                let mut res = OBJECT_NIL;
                textlock_wrap(|| {
                    res = nlua_call_ref(
                        on_changedtick,
                        c"changedtick",
                        args,
                        kRetNilBool,
                        None,
                        None,
                    );
                });

                if luaret_truthy(res) {
                    keep = false;
                }
            }
            if keep {
                i += 1;
            } else {
                buf.update_callbacks.remove(i);
            }
        }
}

pub fn buf_updates_changedtick_single(buf: &mut buf_T, channel_id: u64) {
    let mut items = [
            buffer_obj(buf.b_fnum),
            integer_obj(buf_get_changedtick(&*buf)),
        ];
        let args = Array {
            size: 2,
            capacity: 2,
            items: items.as_mut_ptr(),
        };
        rpc_send_event(channel_id, c"nvim_buf_changedtick_event", args);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luaret_truthy_checks_boolean_true() {
        assert!(luaret_truthy(boolean_obj(true)));
        assert!(!luaret_truthy(boolean_obj(false)));
        assert!(!luaret_truthy(OBJECT_NIL));
        assert!(!luaret_truthy(integer_obj(1)));
    }
}
