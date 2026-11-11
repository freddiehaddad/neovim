//! Buffer update event notifications and Lua callbacks (`src/nvim/buffer_updates.c`).
#![forbid(unsafe_code)]

use crate::api::buffer::{LUA_INTERNAL_CALL, buf_collect_lines};
use crate::api::private::helpers::{
    OBJECT_NIL, arena_array, array_obj, boolean_obj, buffer_obj, integer_obj,
};
use crate::api::private::validate::{Array, Object, ObjectType};
use crate::buffer::{BufUpdateCallbacks, FileBuf, buf_get_changedtick};
use crate::extmark::Bcount;
use crate::globals::*;
use crate::log::logmsg;
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::{K_RET_NIL_BOOL, nlua_call_ref};
use crate::memline::ml_flush_deleted_bytes;
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::msgpack_rpc::packer::Integer;

pub(crate) const ARENA_EMPTY: Arena = Arena::new();

use crate::cursor::{Colnr, Linenr};

const LOG_ERROR: i32 = 3;

#[inline]
pub fn luaret_truthy(res: Object) -> bool {
    res.obj_type == ObjectType::ObjectTypeBoolean && res.data.boolean()
}

fn textlock_wrap<F: FnOnce()>(f: F) {
    let save_cursor = curwin_mut().w_cursor.get();
    g_st().textlock.set(g_st().textlock.get() + (1));
    f();
    g_st().textlock.set(g_st().textlock.get() - (1));
    curwin_mut().w_cursor.set(save_cursor);
}

pub fn buf_updates_register(
    buf: &mut FileBuf,
    channel_id: u64,
    cb: BufUpdateCallbacks,
    send_buffer: bool,
) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_none() {
        return false;
    }

    if channel_id == LUA_INTERNAL_CALL {
        let utf_sizes = cb.utf_sizes;
        buf.update_callbacks.borrow_mut().push(cb);
        if utf_sizes {
            buf.update_need_codepoints.set(true);
        }
        return true;
    }

    if buf.update_channels.borrow().contains(&channel_id) {
        return true;
    }

    buf.update_channels.borrow_mut().push(channel_id);

    if send_buffer {
        let line_count = buf.b_ml.ml_line_count.get() as usize;
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
                std::ptr::null_mut(),
                &mut arena,
            );
        }

        let mut items = [
            buffer_obj(buf.b_fnum.get()),
            integer_obj(buf_get_changedtick(&*buf)),
            integer_obj(0),
            integer_obj(-1),
            array_obj(linedata),
            boolean_obj(false),
        ];
        let args = Array::from_slice(&mut items[..6]);

        rpc_send_event(channel_id, c"nvim_buf_lines_event", args);
        arena_mem_free(arena_finish(&mut arena));
    } else {
        buf_updates_changedtick_single(buf, channel_id);
    }

    true
}

pub fn buf_updates_active(buf: &mut FileBuf) -> bool {
    !buf.update_channels.borrow().is_empty() || !buf.update_callbacks.borrow().is_empty()
}

pub fn buf_updates_send_end(buf: &mut FileBuf, channelid: u64) {
    let mut items = [buffer_obj(buf.b_fnum.get())];
    let args = Array::from_slice(&mut items[..1]);
    rpc_send_event(channelid, c"nvim_buf_detach_event", args);
}

pub fn buf_updates_unregister(buf: &mut FileBuf, channelid: u64) {
    let size = buf.update_channels.borrow().len();
    if size == 0 {
        return;
    }

    buf.update_channels
        .borrow_mut()
        .retain(|&id| id != channelid);
    if buf.update_channels.borrow().len() < size {
        buf_updates_send_end(buf, channelid);
        if buf.update_channels.borrow().is_empty() {
            *buf.update_channels.borrow_mut() = Vec::new();
        }
    }
}

pub fn buf_free_callbacks(buf: &mut FileBuf) {
    *buf.update_channels.borrow_mut() = Vec::new();
    *buf.update_callbacks.borrow_mut() = Vec::new();
}

pub fn buf_updates_unload(buf: &mut FileBuf, can_reload: bool) {
    let channels = std::mem::take(&mut *buf.update_channels.borrow_mut());
    for channelid in channels {
        buf_updates_send_end(buf, channelid);
    }

    let mut i = 0usize;
    while i < buf.update_callbacks.borrow().len() {
        let on_reload = buf.update_callbacks.borrow().as_slice()[i].on_reload;
        let on_detach = buf.update_callbacks.borrow().as_slice()[i].on_detach;
        let mut thecb = LUA_NOREF;
        let mut keep = false;

        if can_reload && on_reload != LUA_NOREF {
            keep = true;
            thecb = on_reload;
        } else if on_detach != LUA_NOREF {
            thecb = on_detach;
        }

        if thecb != LUA_NOREF {
            let mut items = [buffer_obj(buf.b_fnum.get())];
            let args = Array::from_slice(&mut items[..1]);

            textlock_wrap(|| {
                nlua_call_ref(
                    thecb,
                    Some(if keep { c"reload" } else { c"detach" }),
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
            buf.update_callbacks.borrow_mut().remove(i);
        }
    }
    if buf.update_callbacks.borrow().is_empty() {
        *buf.update_callbacks.borrow_mut() = Vec::new();
    }
}

pub fn buf_updates_send_changes(
    buf: &mut FileBuf,
    firstline: Linenr,
    num_added: i64,
    num_removed: i64,
) {
    let mut deleted_codepoints: usize = 0;
    let mut deleted_codeunits: usize = 0;
    let deleted_bytes =
        ml_flush_deleted_bytes(buf, &mut deleted_codepoints, &mut deleted_codeunits);

    if !buf_updates_active(buf) {
        return;
    }

    let send_tick = !(g_st().cmdpreview.get() && (&raw mut *buf) == g_st().curbuf.get().as_ptr());
    let mut badchannelid: u64 = 0;

    let mut arena = ARENA_EMPTY;
    let mut linedata = Array::new();
    if num_added > 0 && !buf.update_channels.borrow().is_empty() {
        linedata = arena_array(&mut arena, num_added as usize);
        buf_collect_lines(
            buf,
            num_added as usize,
            firstline,
            0,
            true,
            &mut linedata,
            std::ptr::null_mut(),
            &mut arena,
        );
    }

    let channels = buf.update_channels.borrow().clone();
    for &channelid in &channels {
        let mut items = [
            buffer_obj(buf.b_fnum.get()),
            if send_tick {
                integer_obj(buf_get_changedtick(buf))
            } else {
                OBJECT_NIL
            },
            integer_obj(i64::from(firstline - 1)),
            integer_obj(i64::from(firstline - 1) + num_removed),
            array_obj(linedata.clone()),
            boolean_obj(false),
        ];
        let args = Array::from_slice(&mut items[..6]);
        if !rpc_send_event(channelid, c"nvim_buf_lines_event", args) {
            badchannelid = channelid;
        }
    }

    if badchannelid != 0 {
        logmsg!(
            LOG_ERROR,
            None,
            Some(c"buf_updates_send_changes"),
            268,
            true,
            c"Disabling buffer updates for dead channel %llu",
            badchannelid,
        );
        buf_updates_unregister(buf, badchannelid);
    }

    arena_mem_free(arena_finish(&mut arena));

    let mut i = 0usize;
    while i < buf.update_callbacks.borrow().len() {
        let on_lines = buf.update_callbacks.borrow().as_slice()[i].on_lines;
        let preview = buf.update_callbacks.borrow().as_slice()[i].preview;
        let utf_sizes = buf.update_callbacks.borrow().as_slice()[i].utf_sizes;
        let mut keep = true;
        if on_lines != LUA_NOREF && (preview || !g_st().cmdpreview.get()) {
            let mut items = [
                buffer_obj(buf.b_fnum.get()),
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
            let args = Array::from_slice(&mut items[..if utf_sizes { 8 } else { 6 }]);

            let mut res = OBJECT_NIL;
            textlock_wrap(|| {
                res = nlua_call_ref(on_lines, Some(c"lines"), args, K_RET_NIL_BOOL, None, None);
            });

            if luaret_truthy(res) {
                keep = false;
            }
        }
        if keep {
            i += 1;
        } else {
            buf.update_callbacks.borrow_mut().remove(i);
        }
    }
}

pub fn buf_updates_send_splice(
    buf: &mut FileBuf,
    start_row: i32,
    start_col: Colnr,
    start_byte: Bcount,
    old_row: i32,
    old_col: Colnr,
    old_byte: Bcount,
    new_row: i32,
    new_col: Colnr,
    new_byte: Bcount,
) {
    if !buf_updates_active(buf) || (old_byte == 0 && new_byte == 0) {
        return;
    }

    let mut i = 0usize;
    while i < buf.update_callbacks.borrow().len() {
        let on_bytes = buf.update_callbacks.borrow().as_slice()[i].on_bytes;
        let preview = buf.update_callbacks.borrow().as_slice()[i].preview;
        let mut keep = true;
        if on_bytes != LUA_NOREF && (preview || !g_st().cmdpreview.get()) {
            let mut items = [
                buffer_obj(buf.b_fnum.get()),
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
            let args = Array::from_slice(&mut items[..11]);

            let mut res = OBJECT_NIL;
            textlock_wrap(|| {
                res = nlua_call_ref(on_bytes, Some(c"bytes"), args, K_RET_NIL_BOOL, None, None);
            });

            if luaret_truthy(res) {
                keep = false;
            }
        }
        if keep {
            i += 1;
        } else {
            buf.update_callbacks.borrow_mut().remove(i);
        }
    }
}

pub fn buf_updates_changedtick(buf: &mut FileBuf) {
    let len = buf.update_channels.borrow().len();
    for idx in 0..len {
        let channel_id = buf.update_channels.borrow()[idx];
        buf_updates_changedtick_single(buf, channel_id);
    }

    let mut i = 0usize;
    while i < buf.update_callbacks.borrow().len() {
        let on_changedtick = buf.update_callbacks.borrow().as_slice()[i].on_changedtick;
        let mut keep = true;
        if on_changedtick != LUA_NOREF {
            let mut items = [
                buffer_obj(buf.b_fnum.get()),
                integer_obj(buf_get_changedtick(&*buf)),
            ];
            let args = Array::from_slice(&mut items[..2]);

            let mut res = OBJECT_NIL;
            textlock_wrap(|| {
                res = nlua_call_ref(
                    on_changedtick,
                    Some(c"changedtick"),
                    args,
                    K_RET_NIL_BOOL,
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
            buf.update_callbacks.borrow_mut().remove(i);
        }
    }
}

pub fn buf_updates_changedtick_single(buf: &mut FileBuf, channel_id: u64) {
    let mut items = [
        buffer_obj(buf.b_fnum.get()),
        integer_obj(buf_get_changedtick(&*buf)),
    ];
    let args = Array::from_slice(&mut items[..2]);
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
