//! Msgpack-RPC channel transport and request/response dispatch (`src/nvim/msgpack_rpc/channel.c`).

use crate::api::private::dispatch::MsgpackRpcRequestHandler;
use crate::api::private::helpers::{
    OBJECT_NIL, api_clear_error, api_free_dict, api_free_object, api_set_error, cstr_as_string,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, Object, ObjectData, ObjectType,
};
use crate::api::ui::{remote_ui_disconnect, remote_ui_flush_if_incomplete};
use crate::channel::{
    CallbackReader, InternalState, K_CHANNEL_PART_RPC_CONST, K_CHANNEL_STREAM_PROC,
    K_CHANNEL_STREAM_SOCKET, K_CHANNEL_STREAM_STDIO, channel_close, channel_decref, channel_event,
    channel_incref,
};
use crate::eval::typval::Callback;
use crate::event::r#loop::{loop_poll_events, loop_queues, uv_strerror};
use crate::event::multiqueue::{
    Event, MultiQueue, event_create_oneshot, multiqueue_empty, multiqueue_free,
    multiqueue_new_child, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::proc::{LibuvProc, exit_on_closed_chan};
use crate::event::rstream::{RStream, rstream_start};
use crate::event::stream::{Stream, StreamCbData};
use crate::event::wstream::{WBuffer, wstream_new_vec_buffer, wstream_write};
use crate::globals::*;
use crate::log::logmsg;
use crate::memory::{ARENA_BLOCK_SIZE, Arena, ArenaMem, arena_finish, arena_mem_free};
use crate::message::semsg;
use crate::msgpack_rpc::packer::{
    Integer, PackerBuffer, mpack_array, mpack_integer, mpack_nil, mpack_object, mpack_object_array,
    mpack_str, mpack_uint, mpack_w,
};
use crate::msgpack_rpc::unpacker::{MessageType, Unpacker, unpacker_advance, unpacker_init};
use crate::os::input::input_blocking;
#[cfg(windows)]
use crate::os::os_win_console::os_detach_ui_console;
use crate::ui_client::{ui_client_attach_to_restarted_server, ui_client_event_raw_line};
use std::ffi::CStr;
use std::ptr;

pub type ClientType = i32;
pub type ChannelStreamType = i32;
pub type ChannelPart = i32;

#[cfg(windows)]
const UV_EPIPE: i32 = -4047;
#[cfg(not(windows))]
const UV_EPIPE: i32 = -32;

const K_CLIENT_TYPE_UNKNOWN: ClientType = -1;
const K_CLIENT_TYPE_REMOTE: ClientType = 0;
const K_CLIENT_TYPE_UI: ClientType = 1;
const K_CLIENT_TYPE_EMBEDDER: ClientType = 2;
const K_CLIENT_TYPE_HOST: ClientType = 3;
const K_CLIENT_TYPE_PLUGIN: ClientType = 4;
const K_CLIENT_TYPE_MSGPACK_RPC: ClientType = 5;

use crate::api::private::helpers::arena_string;

#[repr(C)]
pub struct RemoteUI {
    pub rgb: bool,
    pub r#override: bool,
    pub composed: bool,
    pub ui_ext: [bool; K_UIEXT_COUNT],
    pub width: i32,
    pub height: i32,
    pub pum_nlines: i32,
    pub pum_pos: bool,
    pub pum_row: f64,
    pub pum_col: f64,
    pub pum_height: f64,
    pub pum_width: f64,
    pub term_name: Option<std::ffi::CString>,
    pub term_colors: i32,
    pub stdin_tty: bool,
    pub stdout_tty: bool,
    pub channel_id: u64,
    pub packer: PackerBuffer,
    pub hl_id: i32,
    pub cursor_row: Integer,
    pub cursor_col: Integer,
    pub client_row: Integer,
    pub client_col: Integer,
    pub wildmenu_active: bool,
}

#[repr(C)]
pub struct ChannelCallFrame {
    pub request_id: u32,
    pub returned: std::cell::Cell<bool>,
    pub errored: std::cell::Cell<bool>,
    pub result: std::cell::Cell<Object>,
    pub result_mem: std::cell::Cell<ArenaMem>,
}

#[repr(C)]
pub struct RequestEvent {
    pub r#type: MessageType,
    pub channel: crate::globals::GraphPtr<Channel>,
    pub handler: MsgpackRpcRequestHandler,
    pub args: Array,
    pub request_id: u32,
    pub used_mem: Arena,
}

impl Drop for RequestEvent {
    fn drop(&mut self) {
        arena_mem_free(arena_finish(&mut self.used_mem));
        channel_decref(self.channel.borrow_mut());
    }
}

#[repr(C)]
pub struct RpcState {
    pub closed: bool,
    pub unpacker: Option<Box<Unpacker>>,
    pub has_ui: bool,
    pub next_request_id: u32,
    pub call_stack: Vec<std::rc::Rc<ChannelCallFrame>>,
    pub info: Dict,
    pub client_type: ClientType,
}

impl RpcState {
    pub const fn new() -> Self {
        Self {
            closed: false,
            unpacker: None,
            has_ui: false,
            next_request_id: 0,
            call_stack: Vec::new(),
            info: Dict {
                items: crate::globals::GraphPtr::NULL,
                size: 0,
                capacity: 0,
            },
            client_type: 0,
        }
    }
}

impl Drop for RpcState {
    fn drop(&mut self) {
        api_free_dict(self.info);
    }
}

#[repr(C)]
pub struct Channel {
    pub id: u64,
    pub refcount: usize,
    pub events: crate::globals::GraphPtr<MultiQueue>,
    pub streamtype: ChannelStreamType,
    pub uv: LibuvProc,
    pub socket: RStream,
    pub stdio_in: RStream,
    pub stdio_out: Stream,
    pub err_closed: bool,
    pub internal: InternalState,
    pub is_rpc: bool,
    pub detach: bool,
    pub addr: Option<std::ffi::CString>,
    pub trust: bool,
    pub rpc: RpcState,
    pub term: crate::globals::GraphPtr<crate::terminal::Terminal>,
    pub on_data: CallbackReader,
    pub on_stderr: CallbackReader,
    pub on_exit: Callback,
    pub exit_status: i32,
    pub callback_busy: bool,
    pub callback_scheduled: bool,
    pub did_close_event: bool,
}

impl Channel {
    #[inline]
    pub fn events_mut(&self) -> &'static mut MultiQueue {
        self.events.borrow_mut()
    }

    #[inline]
    pub fn term_mut(&self) -> &'static mut crate::terminal::Terminal {
        self.term.borrow_mut()
    }

    #[inline]
    pub fn unpacker_mut(&mut self) -> &'static mut Unpacker {
        let p = crate::globals::GraphPtr::<Unpacker>::of(self.rpc.unpacker.as_deref_mut().unwrap());
        p.borrow_mut()
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        crate::eval::typval::callback_free(&mut self.on_exit);
        if let Some(events) = self.events.opt_mut() {
            multiqueue_free(events);
        }
    }
}

use crate::api::private::dispatch::{
    handle_nvim_error_event, handle_nvim_get_mode, handle_nvim_paste, handle_nvim_ui_try_resize,
};
use crate::api::ui::K_UIEXT_COUNT;
use crate::api::vim::K_CHANNEL_STREAM_INTERNAL;
use crate::autocmd::EVENT_CHANINFO;
use crate::log::{LOGLVL_ERR, LOGLVL_INF};

#[inline]
pub fn channel_instream_mut(chan: &mut Channel) -> &'static mut Stream {
    let ptr = crate::globals::GraphPtr::<Stream>::of(match chan.streamtype {
        K_CHANNEL_STREAM_PROC => &raw mut chan.uv.proc_.r#in,
        K_CHANNEL_STREAM_SOCKET => &raw mut chan.socket.s,
        K_CHANNEL_STREAM_STDIO => &raw mut chan.stdio_out,
        _ => std::process::abort(),
    })
    .as_ptr();
    crate::globals::GraphPtr::of(ptr).borrow_mut()
}

#[inline]
pub fn channel_outstream_mut(chan: &mut Channel) -> &'static mut RStream {
    let ptr = crate::globals::GraphPtr::<RStream>::of(match chan.streamtype {
        K_CHANNEL_STREAM_PROC => &raw mut chan.uv.proc_.out,
        K_CHANNEL_STREAM_SOCKET => &raw mut chan.socket,
        K_CHANNEL_STREAM_STDIO => &raw mut chan.stdio_in,
        _ => std::process::abort(),
    })
    .as_ptr();
    crate::globals::GraphPtr::of(ptr).borrow_mut()
}

#[inline]
pub fn find_channel(id: u64) -> Option<&'static mut Channel> {
    g_st().channels.get(&id).copied().and_then(|p| p.opt_mut())
}

#[inline]
pub fn find_channel_opt(id: u64) -> Option<&'static mut Channel> {
    find_channel(id)
}

#[inline]
fn unpacker_closed(p: &Unpacker) -> bool {
    p.state < 0
}

pub fn rpc_init() {
    let qc = loop_queues(&mut g_st().main_loop);
    g_st().ch_before_blocking_events =
        crate::globals::GraphPtr::from(multiqueue_new_child(qc.events.borrow_mut()));
}

pub fn rpc_start(channel: &mut Channel) {
    channel_incref(&mut *channel);
    channel.is_rpc = true;
    let rpc = &mut channel.rpc;
    rpc.closed = false;
    let mut unpacker = Box::new(Unpacker::new());
    unpacker_init(&mut unpacker);
    rpc.unpacker = Some(unpacker);
    rpc.next_request_id = 1;
    rpc.info = Dict {
        items: crate::globals::GraphPtr::NULL,
        size: 0,
        capacity: 0,
    };
    rpc.call_stack.clear();

    if channel.streamtype != K_CHANNEL_STREAM_INTERNAL {
        let ch_ptr = crate::globals::GraphPtr::<Channel>::from_ref(channel);
        rstream_start(
            channel_outstream_mut(channel),
            StreamCbData::ChannelRpc(ch_ptr),
        );
    }
}

#[inline]
fn find_rpc_channel_opt(id: u64) -> Option<&'static mut Channel> {
    let chan = find_channel_opt(id)?;
    if !chan.is_rpc || chan.rpc.closed {
        return None;
    }
    Some(chan)
}

pub fn rpc_send_event(id: u64, name: &CStr, args: Array) -> bool {
    if id != 0 {
        let Some(channel) = find_rpc_channel_opt(id) else {
            return false;
        };
        serialize_request(channel.id, 0, name, args);
    } else {
        broadcast_event(name, args);
    }

    true
}

pub fn rpc_send_call(
    id: u64,
    method_name: &CStr,
    args: Array,
    timeout: i32,
    result_mem: &mut ArenaMem,
    err: &mut Error,
) -> Object {
    let Some(chan_ref) = find_rpc_channel_opt(id) else {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Invalid channel: %llu",
            id,
        );
        return OBJECT_NIL;
    };
    let channel = crate::globals::GraphPtr::<Channel>::from_ref(chan_ref);

    channel_incref(channel.borrow_mut());

    let rpc = &mut (channel.borrow_mut().rpc);
    let request_id = rpc.next_request_id;
    rpc.next_request_id += 1;

    serialize_request(id, request_id, method_name, args);

    let frame = std::rc::Rc::new(ChannelCallFrame {
        request_id,
        returned: std::cell::Cell::new(false),
        errored: std::cell::Cell::new(false),
        result: std::cell::Cell::new(OBJECT_NIL),
        result_mem: std::cell::Cell::new(ArenaMem::NULL),
    });
    rpc.call_stack.push(frame.clone());

    let timeout_ms = i64::from(timeout);
    let start = if timeout_ms >= 0 {
        crate::os::time::os_hrtime()
    } else {
        0
    };
    let mut remaining = timeout_ms;
    let mut expired = false;
    while !frame.returned.get() && !unsafe { ptr::read_volatile(&raw const rpc.closed) } && !expired
    {
        let ev_ptr = channel.events;
        if !ev_ptr.is_null() && !multiqueue_empty(ev_ptr.borrow()) {
            multiqueue_process_events(ev_ptr.borrow_mut());
        } else {
            loop_poll_events(&mut g_st().main_loop, remaining);
        }
        if timeout_ms >= 0 {
            remaining = timeout_ms - ((crate::os::time::os_hrtime() - start) / 1_000_000) as i64;
            expired = remaining <= 0;
        }
    }

    rpc.call_stack.pop();

    if !frame.returned.get() {
        if unsafe { ptr::read_volatile(&raw const rpc.closed) } {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Invalid channel: %llu",
                id,
            );
        } else {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Request timed out after %d ms: %s",
                timeout,
                method_name,
            );
        }

        channel_decref(channel.borrow_mut());
        return OBJECT_NIL;
    }

    if frame.errored.get() {
        let res = frame.result.get();
        if res.obj_type == ObjectType::ObjectTypeString {
            api_set_error!(err, ErrorType::ErrorTypeException, c"%s", res.data.string(),);
        } else if res.obj_type == ObjectType::ObjectTypeArray {
            let array = res.data.array();
            if array.size == 2
                && array[0].obj_type == ObjectType::ObjectTypeInteger
                && let Some(et) = ErrorType::from_raw(array[0].data.integer() as i32)
                && matches!(
                    et,
                    ErrorType::ErrorTypeException | ErrorType::ErrorTypeValidation
                )
                && array[1].obj_type == ObjectType::ObjectTypeString
            {
                api_set_error!(err, et, c"%s", array[1].data.string(),);
            } else {
                api_set_error!(err, ErrorType::ErrorTypeException, c"%s", c"unknown error",);
            }
        } else {
            api_set_error!(err, ErrorType::ErrorTypeException, c"%s", c"unknown error",);
        }

        arena_mem_free(frame.result_mem.get());
        frame.result_mem.set(ArenaMem::NULL);
    }

    channel_decref(channel.borrow_mut());

    *result_mem = frame.result_mem.get();

    if frame.errored.get() {
        OBJECT_NIL
    } else {
        frame.result.get()
    }
}

pub(crate) fn receive_msgpack(rbuf: &[u8], channel: &mut Channel, eof: bool) -> usize {
    let c = rbuf.len();
    channel_incref(&mut *channel);

    let mut consumed: usize = 0;

    if c > 0 {
        let p = channel.unpacker_mut();
        let mut read_buf = rbuf;
        parse_msgpack(channel, &mut read_buf);

        if !unpacker_closed(p) {
            consumed = c - read_buf.len();
        }
    }

    if eof {
        let msg = format!("ch {} was closed by the peer\0", channel.id);
        let msg_cstr = std::ffi::CStr::from_bytes_with_nul(msg.as_bytes()).unwrap();
        chan_close_on_err(channel, msg_cstr, LOGLVL_INF);
    }

    channel_decref(channel);

    consumed
}

fn find_call_frame(rpc: &RpcState, request_id: u32) -> Option<std::rc::Rc<ChannelCallFrame>> {
    for frame in rpc.call_stack.iter().rev() {
        if frame.request_id == request_id {
            return Some(frame.clone());
        }
    }
    None
}

fn parse_msgpack(channel: &mut Channel, read_buf: &mut &[u8]) {
    let p_ptr =
        crate::globals::GraphPtr::<Unpacker>::of(channel.rpc.unpacker.as_deref_mut().unwrap());

    while unpacker_advance(p_ptr.borrow_mut(), read_buf) {
        let p = p_ptr.borrow_mut();
        if p.msg_type == MessageType::MessageTypeRedrawEvent {
            if g_st().ui_client_attached {
                if p.has_grid_line_event {
                    ui_client_event_raw_line(&mut p.grid_line_event);
                    p.has_grid_line_event = false;
                } else if let Some(ui_fn) = p.ui_handler.r#fn
                    && p.result.obj_type == ObjectType::ObjectTypeArray
                {
                    ui_fn(p.result.data.array());
                }
            }

            arena_mem_free(arena_finish(&mut p.arena));
        } else if p.msg_type == MessageType::MessageTypeResponse {
            let frame_opt = find_call_frame(&channel.rpc, p.request_id);
            if frame_opt.is_none() && p.request_id < channel.rpc.next_request_id {
                logmsg!(
                    crate::log::LOGLVL_DBG,
                    None,
                    Some(c"parse_msgpack"),
                    line!() as i32,
                    true,
                    c"ch %llu: discarding response id=%u",
                    channel.id,
                    p.request_id,
                );
                arena_mem_free(arena_finish(&mut p.arena));
                continue;
            }
            let is_not_last = channel.rpc.client_type != K_CLIENT_TYPE_MSGPACK_RPC
                && frame_opt.as_ref().is_none_or(|f| {
                    channel
                        .rpc
                        .call_stack
                        .last()
                        .is_none_or(|last| !std::rc::Rc::ptr_eq(f, last))
                });
            let Some(frame) = frame_opt.filter(|_| !is_not_last) else {
                let msg = format!(
                    "ch {} (type={}) returned a response with an unknown request id {}. Ensure the client is properly synchronized\0",
                    channel.id, channel.rpc.client_type as u32, p.request_id
                );
                let msg_cstr = std::ffi::CStr::from_bytes_with_nul(msg.as_bytes()).unwrap();
                chan_close_on_err(channel, msg_cstr, LOGLVL_ERR);
                return;
            };

            let errored = p.error.obj_type != ObjectType::ObjectTypeNil;
            frame.returned.set(true);
            frame.errored.set(errored);

            if errored {
                frame.result.set(p.error);
            } else {
                frame.result.set(p.result);
            }

            frame.result_mem.set(arena_finish(&mut p.arena));
        } else {
            let res = p.result;
            if res.obj_type != ObjectType::ObjectTypeArray {
                chan_close_on_err(
                    channel,
                    c"msgpack-rpc request args must be an array",
                    LOGLVL_ERR,
                );
                return;
            }
            handle_request(channel, p, res.data.array());
        }
    }

    let p = p_ptr.borrow_mut();
    if unpacker_closed(p) {
        let msg_cstr = p.unpack_error.msg_cstr().unwrap_or(c"");
        chan_close_on_err(channel, msg_cstr, LOGLVL_INF);

        api_clear_error(&mut p.unpack_error);
    }
}

fn handle_request(channel: &mut Channel, p: &mut Unpacker, args: Array) {
    assert!(
        p.msg_type == MessageType::MessageTypeRequest
            || p.msg_type == MessageType::MessageTypeNotification
    );

    // SECURITY: reject API calls from an untrusted peer. Else a malicious (remote) server could run
    // arbitrary code in the local process.
    if !channel.trust
        && p.handler
            .r#fn
            .is_some_and(|f| f as *const () != handle_nvim_error_event as *const ())
    {
        api_set_error!(
            &mut p.unpack_error,
            ErrorType::ErrorTypeException,
            c"Rejected API call from untrusted peer: %s",
            p.handler.name,
        );
        p.handler.r#fn = None;
    }

    if p.handler.r#fn.is_none() {
        let handler = p.handler;
        let msg_type = p.msg_type;
        let request_id = p.request_id;
        let msg = p.unpack_error.msg.take();
        send_error(channel, handler, msg_type, request_id, msg);

        api_clear_error(&mut p.unpack_error);

        arena_mem_free(arena_finish(&mut p.arena));

        return;
    }
    let evdata = RequestEvent {
        r#type: p.msg_type,
        channel: crate::globals::GraphPtr::from(&mut *channel),
        handler: p.handler,
        args,
        used_mem: p.arena,
        request_id: p.request_id,
    };

    p.arena = Arena::new();

    channel_incref(&mut *channel);

    if p.handler.fast {
        let is_get_mode = p
            .handler
            .r#fn
            .is_some_and(|f| f as *const () == handle_nvim_get_mode as *const ());

        if is_get_mode && !input_blocking() {
            multiqueue_put_event(
                g_st().ch_before_blocking_events.borrow_mut(),
                Event::RpcRequest(Box::new(evdata)),
            );
        } else {
            request_event(evdata);
        }
    } else {
        let is_resize = p
            .handler
            .r#fn
            .is_some_and(|f| f as *const () == handle_nvim_ui_try_resize as *const ());

        if is_resize {
            let oneshot = event_create_oneshot(Event::RpcRequest(Box::new(evdata)));

            multiqueue_put_event(
                channel.events.borrow_mut(),
                Event::Oneshot(std::rc::Rc::clone(&oneshot)),
            );

            multiqueue_put_event(g_st().resize_events.borrow_mut(), Event::Oneshot(oneshot));
        } else {
            multiqueue_put_event(
                channel.events.borrow_mut(),
                Event::RpcRequest(Box::new(evdata)),
            );
        }
    }
}

pub fn request_event(mut e: RequestEvent) {
    let channel = e.channel.borrow_mut();
    let handler = e.handler;
    let mut error = Error::new();

    if !channel.rpc.closed {
        let fn_call = handler.r#fn.unwrap();
        let mut result = fn_call(channel.id, e.args, &mut e.used_mem, &mut error);
        if e.r#type == MessageType::MessageTypeRequest || error.err_type != ErrorType::ErrorTypeNone
        {
            serialize_response(
                channel,
                e.handler,
                e.r#type,
                e.request_id,
                &mut error,
                &mut result,
            );
        }
        if handler.ret_alloc {
            api_free_object(result);
        }
    }

    drop(e);
    api_clear_error(&mut error);
}

pub fn rpc_write_raw(id: u64, buffer: std::rc::Rc<WBuffer>) -> bool {
    let Some(channel) = find_rpc_channel_opt(id) else {
        return false;
    };

    channel_write(channel, buffer)
}

fn channel_write(channel: &mut Channel, buffer: std::rc::Rc<WBuffer>) -> bool {
    let mut err: i32 = 0;

    if channel.rpc.closed {
        return false;
    }

    if channel.streamtype == K_CHANNEL_STREAM_INTERNAL {
        channel_incref(&mut *channel);
        if !channel.events.is_null() {
            multiqueue_put_event(
                channel.events_mut(),
                Event::RpcInternalRead {
                    channel: channel.into(),
                    buffer,
                },
            );
        } else {
            internal_read_event(channel, buffer);
        }
    } else {
        err = wstream_write(channel_instream_mut(channel), buffer);
    }

    if err != 0 {
        let err_str = crate::strings::ByteSlice::from_ptr(uv_strerror(err))
            .as_cstr()
            .to_string_lossy();
        let msg = format!(
            "ch {}: stream write failed: {err_str}. RPC canceled; closing channel\0",
            channel.id
        );
        let msg_cstr = std::ffi::CStr::from_bytes_with_nul(msg.as_bytes()).unwrap();
        chan_close_on_err(
            channel,
            msg_cstr,
            if err == UV_EPIPE {
                LOGLVL_INF
            } else {
                LOGLVL_ERR
            },
        );
    }

    err == 0
}

pub fn internal_read_event(channel: &mut Channel, buffer: std::rc::Rc<WBuffer>) {
    let p = channel.unpacker_mut();
    let mut read_buf = buffer.data.as_slice();

    if !unpacker_closed(p) {
        parse_msgpack(channel, &mut read_buf);

        if !read_buf.is_empty() && !channel.rpc.closed {
            chan_close_on_err(channel, c"internal channel: internal error", LOGLVL_ERR);
        }
    }

    channel_decref(channel);
}

fn send_error(
    chan: &mut Channel,
    handler: MsgpackRpcRequestHandler,
    r#type: MessageType,
    id: u32,
    err: Option<std::ffi::CString>,
) {
    let mut e = Error {
        err_type: ErrorType::ErrorTypeException,
        msg: err,
    };
    let mut nil_obj = OBJECT_NIL;
    serialize_response(chan, handler, r#type, id, &mut e, &mut nil_obj);
    api_clear_error(&mut e);
}

fn broadcast_event(name: &CStr, args: Array) {
    if g_st().channels.values().any(|&ch| ch.is_rpc) {
        serialize_request(0, 0, name, args);
    }
}

fn has_undetached_rpc_chan() -> bool {
    for &chan in g_st().channels.values() {
        let ct = chan.borrow();
        if ct.is_rpc && !ct.rpc.closed && !ct.detach {
            return true;
        }
    }
    false
}

pub fn rpc_close(channel: &mut Channel) {
    if channel.rpc.closed {
        return;
    }

    channel.rpc.closed = true;

    {
        let qc = loop_queues(&mut g_st().main_loop);
        multiqueue_put_event(qc.fast_events.borrow_mut(), Event::RpcClose(channel.into()));
    }
}

pub fn rpc_close_event(channel: Option<&mut Channel>) {
    let channel = channel.unwrap();
    channel_decref(channel);

    #[cfg(windows)]
    let ui_closed = channel.rpc.has_ui;

    remote_ui_disconnect(channel.id, &mut Error::new(), false);

    let is_ui_client =
        g_st().ui_client_channel_id != 0 && channel.id == g_st().ui_client_channel_id;
    if is_ui_client {
        if !g_st().exiting && channel.streamtype != K_CHANNEL_STREAM_PROC {
            ui_client_attach_to_restarted_server(false);
        }
        if g_st().ui_client_channel_id != channel.id {
            return;
        }
        if channel.streamtype == K_CHANNEL_STREAM_PROC && g_st().ui_client_error_exit < 0 {
            return;
        }
        exit_on_closed_chan(0);
    } else if !channel.detach && !has_undetached_rpc_chan() {
        exit_on_closed_chan(0);
    } else {
        #[cfg(windows)]
        if ui_closed {
            os_detach_ui_console(channel.streamtype == K_CHANNEL_STREAM_STDIO);
        }
    }
}

fn chan_close_on_err(channel: &mut Channel, msg: &std::ffi::CStr, loglevel: i32) {
    let msg = crate::strings::ByteSlice::from_cstr(msg);
    logmsg!(loglevel, Some(c"RPC: "), None, -1, true, c"%s", msg,);
    let p = channel.unpacker_mut();

    for frame in &channel.rpc.call_stack {
        if frame.returned.get() {
            continue;
        }
        frame.returned.set(true);
        frame.errored.set(true);
        let s = arena_string(&mut p.arena, cstr_as_string(msg.as_opt_cstr()));
        frame.result.set(Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(s),
        });
        frame.result_mem.set(arena_finish(&mut p.arena));
    }

    channel_close(channel.id, K_CHANNEL_PART_RPC_CONST, None);
    logmsg!(
        loglevel,
        None,
        Some(c"chan_close_on_err"),
        492,
        true,
        c"RPC: %s",
        msg,
    );
}

fn serialize_request(chan_id: u64, request_id: u32, method: &CStr, args: Array) {
    let mut packer = PackerBuffer::new();
    packer_buffer_init_channels(chan_id, &mut packer);

    mpack_array(&mut packer, if request_id != 0 { 4 } else { 3 });
    mpack_w(&mut packer, if request_id != 0 { 0 } else { 2 });

    if request_id != 0 {
        mpack_uint(&mut packer, request_id);
    }

    mpack_str(cstr_as_string(Some(method)), &mut packer);
    mpack_object_array(args, &mut packer);

    packer_buffer_finish_channels(&mut packer);
}

pub fn serialize_response(
    channel: &mut Channel,
    handler: MsgpackRpcRequestHandler,
    r#type: MessageType,
    response_id: u32,
    err: &mut Error,
    arg: &mut Object,
) {
    if err.err_type != ErrorType::ErrorTypeNone && r#type == MessageType::MessageTypeNotification {
        let is_paste = handler
            .r#fn
            .is_some_and(|f| f as *const () == handle_nvim_paste as *const ());

        if is_paste {
            semsg!(c"paste: %s", err.msg_cstr());
            api_clear_error(err);
        } else {
            let mut items = [
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(err.err_type as i64),
                },
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(err.msg_cstr())),
                },
            ];
            let args = Array {
                items: crate::globals::GraphPtr::new(items.as_mut_ptr()),
                size: 2,
                capacity: 2,
            };
            serialize_request(channel.id, 0, c"nvim_error_event", args);
        }
        return;
    }

    let mut packer = PackerBuffer::new();
    packer_buffer_init_channels(channel.id, &mut packer);

    mpack_array(&mut packer, 4);
    mpack_w(&mut packer, 1);
    mpack_uint(&mut packer, response_id);

    if err.err_type != ErrorType::ErrorTypeNone {
        mpack_array(&mut packer, 2);
        mpack_integer(&mut packer, err.err_type as Integer);
        mpack_str(cstr_as_string(err.msg_cstr()), &mut packer);
        mpack_nil(&mut packer);
    } else {
        mpack_nil(&mut packer);
        mpack_object(arg, &mut packer);
    }

    packer_buffer_finish_channels(&mut packer);
}

fn packer_buffer_init_channels(chan_id: u64, packer: &mut PackerBuffer) {
    if chan_id != 0 {
        if let Some(ct) = find_channel_opt(chan_id)
            && ct.rpc.has_ui
        {
            remote_ui_flush_if_incomplete(ct.id);
        }
    } else {
        let rpc_ids: Vec<u64> = g_st()
            .channels
            .values()
            .filter_map(|&ch| ch.is_rpc.then_some(ch.id))
            .collect();
        for id in rpc_ids {
            if let Some(ct) = find_channel_opt(id)
                && ct.rpc.has_ui
            {
                remote_ui_flush_if_incomplete(ct.id);
            }
        }
    }
    packer.owned_buf = vec![0; ARENA_BLOCK_SIZE];
    packer.pos = 0;
    packer.anydata = crate::msgpack_rpc::packer::PackerData::Chans;
    packer.anyint = chan_id as i64;
}

fn packer_buffer_finish_channels(packer: &mut PackerBuffer) {
    let len = packer.pos;
    let mut vec = std::mem::take(&mut packer.owned_buf);
    packer.pos = 0;
    if len > 0 {
        vec.truncate(len);
        let buf = wstream_new_vec_buffer(vec);
        let chan_id = packer.anyint as u64;
        if chan_id != 0 {
            if let Some(chan) = find_channel_opt(chan_id) {
                let _ = channel_write(chan, buf);
            }
        } else {
            let rpc_chans: Vec<crate::globals::GraphPtr<Channel>> = g_st()
                .channels
                .values()
                .copied()
                .filter(|&ch| ch.is_rpc)
                .collect();
            for chan in rpc_chans {
                let _ = channel_write(chan.borrow_mut(), std::rc::Rc::clone(&buf));
            }
        }
    }
}

pub(crate) fn channel_flush_callback(packer: &mut PackerBuffer) {
    packer_buffer_finish_channels(packer);
    packer_buffer_init_channels(packer.anyint as u64, packer);
}

pub fn rpc_set_client_info(id: u64, info: Dict) {
    let Some(chan) = find_rpc_channel_opt(id) else {
        std::process::abort();
    };

    api_free_dict(chan.rpc.info);

    chan.rpc.info = info;

    let r#type = get_client_info(chan, c"type");

    chan.rpc.client_type = match r#type.map(CStr::to_bytes) {
        None => K_CLIENT_TYPE_REMOTE,
        Some(b"remote") => K_CLIENT_TYPE_REMOTE,
        Some(b"msgpack-rpc") => K_CLIENT_TYPE_MSGPACK_RPC,
        Some(b"ui") => K_CLIENT_TYPE_UI,
        Some(b"embedder") => K_CLIENT_TYPE_EMBEDDER,
        Some(b"host") => K_CLIENT_TYPE_HOST,
        Some(b"plugin") => K_CLIENT_TYPE_PLUGIN,
        Some(_) => K_CLIENT_TYPE_UNKNOWN,
    };

    channel_event(chan, EVENT_CHANINFO);
}

pub fn get_client_info<'a>(chan: &'a mut Channel, key: &CStr) -> Option<&'a CStr> {
    let ct = chan;
    if !ct.is_rpc {
        return None;
    }
    let key_bytes = key.to_bytes();
    let info = ct.rpc.info;
    for i in 0..info.size {
        let item = &info[i];
        if !item.key.data.is_null()
            && item.key.cstr_bytes() == key_bytes
            && item.value.obj_type == ObjectType::ObjectTypeString
        {
            let val_str = item.value.data.string();
            if !val_str.data.is_null() {
                return Some(val_str.as_cstr());
            }
        }
    }

    None
}

pub fn rpc_free_all_mem() {
    multiqueue_free(g_st().ch_before_blocking_events.borrow_mut());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_call_frame_searches_stack_from_top() {
        let mut rpc = RpcState {
            closed: false,
            unpacker: None,
            has_ui: false,
            next_request_id: 1,
            call_stack: Vec::new(),
            info: Dict {
                items: crate::globals::GraphPtr::NULL,
                size: 0,
                capacity: 0,
            },
            client_type: K_CLIENT_TYPE_MSGPACK_RPC,
        };
        let f1 = std::rc::Rc::new(ChannelCallFrame {
            request_id: 10,
            returned: std::cell::Cell::new(false),
            errored: std::cell::Cell::new(false),
            result: std::cell::Cell::new(OBJECT_NIL),
            result_mem: std::cell::Cell::new(ArenaMem::NULL),
        });
        let f2 = std::rc::Rc::new(ChannelCallFrame {
            request_id: 20,
            returned: std::cell::Cell::new(false),
            errored: std::cell::Cell::new(false),
            result: std::cell::Cell::new(OBJECT_NIL),
            result_mem: std::cell::Cell::new(ArenaMem::NULL),
        });
        rpc.call_stack.push(f1.clone());
        rpc.call_stack.push(f2.clone());

        assert!(std::rc::Rc::ptr_eq(
            &find_call_frame(&rpc, 20).unwrap(),
            &f2
        ));
        assert!(std::rc::Rc::ptr_eq(
            &find_call_frame(&rpc, 10).unwrap(),
            &f1
        ));
        assert!(find_call_frame(&rpc, 99).is_none());
    }
}
