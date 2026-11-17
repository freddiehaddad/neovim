#![forbid(unsafe_code)]

//! Msgpack-RPC channel transport and request/response dispatch (`src/nvim/msgpack_rpc/channel.c`).

use crate::api::private::dispatch::MsgpackRpcRequestHandler;
use crate::api::private::helpers::{
    OBJECT_NIL, api_error, api_free_dict, api_free_object, cstr_as_string,
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
use crate::memory::ARENA_BLOCK_SIZE;
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

use crate::api::private::helpers::ensure_nul_string;

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

pub struct ChannelCallFrame {
    pub request_id: u32,
    pub returned: std::cell::Cell<bool>,
    pub errored: std::cell::Cell<bool>,
    pub result: crate::api::private::validate::CloneCell<Object>,
}

pub struct RequestEvent {
    pub r#type: MessageType,
    pub channel: std::rc::Rc<Channel>,
    pub handler: MsgpackRpcRequestHandler,
    pub args: Array,
    pub request_id: u32,
}

impl Drop for RequestEvent {
    fn drop(&mut self) {
        channel_decref(&self.channel);
    }
}

pub struct RpcState {
    pub closed: std::cell::Cell<bool>,
    unpacker: std::cell::RefCell<Option<Box<Unpacker>>>,
    pub has_ui: std::cell::Cell<bool>,
    pub next_request_id: std::cell::Cell<u32>,
    call_stack: std::cell::RefCell<Vec<std::rc::Rc<ChannelCallFrame>>>,
    pub info: crate::globals::CloneCell<Dict>,
    pub client_type: std::cell::Cell<ClientType>,
}

impl RpcState {
    pub const fn new() -> Self {
        Self {
            closed: std::cell::Cell::new(false),
            unpacker: std::cell::RefCell::new(None),
            has_ui: std::cell::Cell::new(false),
            next_request_id: std::cell::Cell::new(0),
            call_stack: std::cell::RefCell::new(Vec::new()),
            info: crate::globals::CloneCell::new(Dict::new()),
            client_type: std::cell::Cell::new(0),
        }
    }
}

impl Drop for RpcState {
    fn drop(&mut self) {
        api_free_dict(self.info.take());
    }
}

pub struct Channel {
    pub id: u64,
    pub refcount: std::cell::Cell<usize>,
    pub events: MultiQueue,
    pub streamtype: ChannelStreamType,
    pub uv: LibuvProc,
    pub socket: RStream,
    pub stdio_in: RStream,
    pub stdio_out: Stream,
    pub err_closed: std::cell::Cell<bool>,
    pub internal: std::cell::Cell<InternalState>,
    pub is_rpc: std::cell::Cell<bool>,
    pub detach: std::cell::Cell<bool>,
    pub addr: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub trust: std::cell::Cell<bool>,
    pub rpc: RpcState,
    pub term: std::cell::Cell<crate::terminal::TermId>,
    on_data: std::cell::RefCell<CallbackReader>,
    on_stderr: std::cell::RefCell<CallbackReader>,
    pub on_exit: Callback,
    pub exit_status: std::cell::Cell<i32>,
    pub callback_busy: std::cell::Cell<bool>,
    pub callback_scheduled: std::cell::Cell<bool>,
    pub did_close_event: std::cell::Cell<bool>,
}

impl Channel {
    pub fn new(id: u64, streamtype: ChannelStreamType, events: MultiQueue) -> Self {
        Self {
            id,
            streamtype,
            events,
            refcount: std::cell::Cell::new(1),
            uv: LibuvProc::new(),
            socket: RStream::new(),
            stdio_in: RStream::new(),
            stdio_out: Stream::new(),
            err_closed: std::cell::Cell::new(false),
            internal: std::cell::Cell::new(InternalState {
                cb: 0,
                closed: false,
            }),
            is_rpc: std::cell::Cell::new(false),
            detach: std::cell::Cell::new(streamtype != K_CHANNEL_STREAM_STDIO),
            addr: crate::globals::CloneCell::new(None),
            trust: std::cell::Cell::new(true),
            rpc: RpcState::new(),
            term: std::cell::Cell::new(crate::terminal::TermId::NULL),
            on_data: std::cell::RefCell::new(CallbackReader::none()),
            on_stderr: std::cell::RefCell::new(CallbackReader::none()),
            on_exit: Callback::NONE,
            exit_status: std::cell::Cell::new(-1),
            callback_busy: std::cell::Cell::new(false),
            callback_scheduled: std::cell::Cell::new(false),
            did_close_event: std::cell::Cell::new(false),
        }
    }

    #[inline]
    pub fn events(&self) -> MultiQueue {
        self.events
    }

    #[inline]
    pub fn term(&self) -> crate::terminal::TermId {
        self.term.get()
    }

    pub fn with_reader<R>(&self, stderr: bool, f: impl FnOnce(&CallbackReader) -> R) -> R {
        let reader = if stderr {
            &self.on_stderr
        } else {
            &self.on_data
        };
        f(&reader.borrow())
    }

    pub fn with_reader_mut<R>(&self, stderr: bool, f: impl FnOnce(&mut CallbackReader) -> R) -> R {
        let reader = if stderr {
            &self.on_stderr
        } else {
            &self.on_data
        };
        f(&mut reader.borrow_mut())
    }

    pub fn set_reader(&self, stderr: bool, value: CallbackReader) {
        let old = self.with_reader_mut(stderr, |reader| std::mem::replace(reader, value));
        drop(old);
    }

    pub fn set_exit_callback(&self, value: Callback) {
        crate::eval::typval::callback_free(&self.on_exit);
        self.on_exit.data.set(value.data.take());
    }

    fn with_unpacker_mut<R>(&self, f: impl FnOnce(&mut Unpacker) -> R) -> R {
        f(self.rpc.unpacker.borrow_mut().as_deref_mut().unwrap())
    }

    fn unpacker_is_closed(&self) -> bool {
        unpacker_closed(self.rpc.unpacker.borrow().as_deref().unwrap())
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        crate::eval::typval::callback_free(&self.on_exit);
        if let Some(events) = self.events.opt() {
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
pub fn channel_instream(chan: &Channel) -> &Stream {
    match chan.streamtype {
        K_CHANNEL_STREAM_PROC => &chan.uv.proc_.r#in,
        K_CHANNEL_STREAM_SOCKET => &chan.socket.s,
        K_CHANNEL_STREAM_STDIO => &chan.stdio_out,
        _ => std::process::abort(),
    }
}

#[inline]
pub fn channel_outstream(chan: &Channel) -> &RStream {
    match chan.streamtype {
        K_CHANNEL_STREAM_PROC => &chan.uv.proc_.out,
        K_CHANNEL_STREAM_SOCKET => &chan.socket,
        K_CHANNEL_STREAM_STDIO => &chan.stdio_in,
        _ => std::process::abort(),
    }
}

#[inline]
pub fn find_channel(id: u64) -> Option<std::rc::Rc<Channel>> {
    crate::event::r#loop::main_loop().find_channel(id)
}

#[inline]
pub fn find_channel_opt(id: u64) -> Option<std::rc::Rc<Channel>> {
    find_channel(id)
}

#[inline]
fn unpacker_closed(p: &Unpacker) -> bool {
    p.state < 0
}

pub fn rpc_init() {
    let qc = loop_queues(crate::event::r#loop::main_loop());
    input_state()
        .ch_before_blocking_events
        .set(multiqueue_new_child(qc.events.get()));
}

pub fn rpc_start(channel: &Channel) {
    channel_incref(channel);
    channel.is_rpc.set(true);
    let rpc = &channel.rpc;
    rpc.closed.set(false);
    let mut unpacker = Box::new(Unpacker::new());
    unpacker_init(&mut unpacker);
    let old = rpc.unpacker.replace(Some(unpacker));
    drop(old);
    rpc.next_request_id.set(1);
    rpc.info.set(Dict::new());
    rpc.call_stack.borrow_mut().clear();

    if channel.streamtype != K_CHANNEL_STREAM_INTERNAL {
        let ch_id = channel.id;
        rstream_start(channel_outstream(channel), StreamCbData::ChannelRpc(ch_id));
    }
}

#[inline]
fn find_rpc_channel_opt(id: u64) -> Option<std::rc::Rc<Channel>> {
    let chan = find_channel_opt(id)?;
    if !chan.is_rpc.get() || chan.rpc.closed.get() {
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
) -> Result<Object, Error> {
    let Some(channel) = find_rpc_channel_opt(id) else {
        return Err(api_error!(
            ErrorType::ErrorTypeException,
            c"Invalid channel: %llu",
            id,
        ));
    };

    channel_incref(&channel);

    let rpc = &channel.rpc;
    let request_id = rpc.next_request_id.get();
    rpc.next_request_id.set(rpc.next_request_id.get() + (1));

    let frame = std::rc::Rc::new(ChannelCallFrame {
        request_id,
        returned: std::cell::Cell::new(false),
        errored: std::cell::Cell::new(false),
        result: crate::api::private::validate::CloneCell::new(OBJECT_NIL),
    });
    rpc.call_stack.borrow_mut().push(frame.clone());
    let ev_ptr = channel.events;

    serialize_request(id, request_id, method_name, args);

    let timeout_ms = i64::from(timeout);
    let start = if timeout_ms >= 0 {
        crate::os::time::os_hrtime()
    } else {
        0
    };
    let mut remaining = timeout_ms;
    let mut expired = false;
    while !frame.returned.get() && !channel.rpc.closed.get() && !expired {
        if !ev_ptr.is_null() && !multiqueue_empty(ev_ptr.borrow()) {
            multiqueue_process_events(ev_ptr);
        } else {
            loop_poll_events(crate::event::r#loop::main_loop(), remaining);
        }
        if timeout_ms >= 0 {
            remaining = timeout_ms - ((crate::os::time::os_hrtime() - start) / 1_000_000) as i64;
            expired = remaining <= 0;
        }
    }

    channel.rpc.call_stack.borrow_mut().pop();

    if !frame.returned.get() {
        let err = if channel.rpc.closed.get() {
            api_error!(ErrorType::ErrorTypeException, c"Invalid channel: %llu", id,)
        } else {
            api_error!(
                ErrorType::ErrorTypeException,
                c"Request timed out after %d ms: %s",
                timeout,
                method_name,
            )
        };

        channel_decref(&channel);
        return Err(err);
    }

    channel_decref(&channel);

    if frame.errored.get() {
        let res = frame.result.get();
        let err = if res.obj_type == ObjectType::ObjectTypeString {
            api_error!(ErrorType::ErrorTypeException, c"%s", res.data.string())
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
                api_error!(et, c"%s", array[1].data.string())
            } else {
                api_error!(ErrorType::ErrorTypeException, c"%s", c"unknown error")
            }
        } else {
            api_error!(ErrorType::ErrorTypeException, c"%s", c"unknown error")
        };
        Err(err)
    } else {
        Ok(frame.result.get())
    }
}

pub(crate) fn receive_msgpack(rbuf: &[u8], channel: &std::rc::Rc<Channel>, eof: bool) -> usize {
    let c = rbuf.len();
    channel_incref(channel);

    let mut consumed: usize = 0;

    if c > 0 {
        let mut read_buf = rbuf;
        parse_msgpack(channel, &mut read_buf);

        if !channel.unpacker_is_closed() {
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
    for frame in rpc.call_stack.borrow().iter().rev() {
        if frame.request_id == request_id {
            return Some(frame.clone());
        }
    }
    None
}

struct DecodedRequest {
    request_id: Option<u32>,
    handler: MsgpackRpcRequestHandler,
    args: Array,
    unpack_error: Option<std::ffi::CString>,
}

enum DecodedMessage {
    Request(DecodedRequest),
    Response {
        request_id: u32,
        result: Result<Object, Object>,
    },
    GridLine(crate::msgpack_rpc::unpacker::GridLineEvent),
    UiCall {
        handler: fn(Array),
        args: Array,
    },
    InvalidArguments,
    Ignore,
}

fn parse_msgpack(channel: &std::rc::Rc<Channel>, read_buf: &mut &[u8]) {
    loop {
        let message = channel.with_unpacker_mut(|p| {
            if !unpacker_advance(p, read_buf) {
                return None;
            }
            let unpack_error = p.unpack_error.msg.take();
            p.unpack_error.clear();
            let mut result = std::mem::take(&mut p.result);
            let error = std::mem::take(&mut p.error);
            Some(match p.msg_type {
                MessageType::MessageTypeRedrawEvent => {
                    if std::mem::take(&mut p.has_grid_line_event) {
                        DecodedMessage::GridLine(p.grid_line_event)
                    } else if let Some(handler) = p.ui_handler.r#fn
                        && result.obj_type == ObjectType::ObjectTypeArray
                    {
                        DecodedMessage::UiCall {
                            handler,
                            args: std::mem::take(result.data.array_mut()),
                        }
                    } else {
                        DecodedMessage::Ignore
                    }
                }
                MessageType::MessageTypeResponse => DecodedMessage::Response {
                    request_id: p.request_id,
                    result: if error.obj_type == ObjectType::ObjectTypeNil {
                        Ok(result)
                    } else {
                        Err(error)
                    },
                },
                MessageType::MessageTypeRequest | MessageType::MessageTypeNotification => {
                    if result.obj_type != ObjectType::ObjectTypeArray {
                        DecodedMessage::InvalidArguments
                    } else {
                        DecodedMessage::Request(DecodedRequest {
                            request_id: (p.msg_type == MessageType::MessageTypeRequest)
                                .then_some(p.request_id),
                            handler: p.handler,
                            args: std::mem::take(result.data.array_mut()),
                            unpack_error,
                        })
                    }
                }
            })
        });
        let Some(message) = message else { break };
        match message {
            DecodedMessage::GridLine(line) => {
                if screen_state().ui_client_attached.get() {
                    ui_client_event_raw_line(&line);
                }
            }
            DecodedMessage::UiCall { handler, args } => {
                if screen_state().ui_client_attached.get() {
                    handler(args);
                }
            }
            DecodedMessage::Response { request_id, result } => {
                let frame = find_call_frame(&channel.rpc, request_id);
                if frame.is_none() && request_id < channel.rpc.next_request_id.get() {
                    logmsg!(
                        crate::log::LOGLVL_DBG,
                        None,
                        Some(c"parse_msgpack"),
                        line!() as i32,
                        true,
                        c"ch %llu: discarding response id=%u",
                        channel.id,
                        request_id
                    );
                    continue;
                }
                let is_not_last = channel.rpc.client_type.get() != K_CLIENT_TYPE_MSGPACK_RPC
                    && frame.as_ref().is_none_or(|frame| {
                        channel
                            .rpc
                            .call_stack
                            .borrow()
                            .last()
                            .is_none_or(|last| !std::rc::Rc::ptr_eq(frame, last))
                    });
                let Some(frame) = frame.filter(|_| !is_not_last) else {
                    let message = format!(
                        "ch {} (type={}) returned a response with an unknown request id {}. Ensure the client is properly synchronized\0",
                        channel.id,
                        channel.rpc.client_type.get() as u32,
                        request_id
                    );
                    let message = CStr::from_bytes_with_nul(message.as_bytes()).unwrap();
                    chan_close_on_err(channel, message, LOGLVL_ERR);
                    return;
                };
                frame.returned.set(true);
                frame.errored.set(result.is_err());
                frame.result.set(result.unwrap_or_else(|error| error));
            }
            DecodedMessage::Request(request) => handle_request(channel, request),
            DecodedMessage::InvalidArguments => {
                chan_close_on_err(
                    channel,
                    c"msgpack-rpc request args must be an array",
                    LOGLVL_ERR,
                );
                return;
            }
            DecodedMessage::Ignore => {}
        }
    }
    let error = channel.with_unpacker_mut(|p| {
        if !unpacker_closed(p) {
            return None;
        }
        let message = p.unpack_error.msg.take().unwrap_or_default();
        p.unpack_error.clear();
        Some(message)
    });
    if let Some(message) = error {
        chan_close_on_err(channel, &message, LOGLVL_INF);
    }
}

fn handle_request(channel: &std::rc::Rc<Channel>, mut message: DecodedRequest) {
    let kind = if message.request_id.is_some() {
        MessageType::MessageTypeRequest
    } else {
        MessageType::MessageTypeNotification
    };
    let request_id = message.request_id.unwrap_or(0);
    // SECURITY: reject API calls from an untrusted peer. Else a malicious (remote) server could run
    // arbitrary code in the local process.
    if !channel.trust.get()
        && message
            .handler
            .r#fn
            .is_some_and(|f| f as *const () != handle_nvim_error_event as *const ())
    {
        message.unpack_error = api_error!(
            ErrorType::ErrorTypeException,
            c"Rejected API call from untrusted peer: %s",
            message.handler.name,
        )
        .msg;
        message.handler.r#fn = None;
    }

    if message.handler.r#fn.is_none() {
        send_error(
            channel,
            message.handler,
            kind,
            request_id,
            message.unpack_error,
        );
        return;
    }
    let handler = message.handler;
    let evdata = RequestEvent {
        r#type: kind,
        channel: channel.clone(),
        handler,
        args: message.args,
        request_id,
    };

    channel_incref(channel);

    if handler.fast {
        let is_get_mode = handler
            .r#fn
            .is_some_and(|f| f as *const () == handle_nvim_get_mode as *const ());

        if is_get_mode && !input_blocking() {
            multiqueue_put_event(
                input_state().ch_before_blocking_events.get(),
                Event::RpcRequest(Box::new(evdata)),
            );
        } else {
            request_event(evdata);
        }
    } else {
        let is_resize = handler
            .r#fn
            .is_some_and(|f| f as *const () == handle_nvim_ui_try_resize as *const ());

        if is_resize {
            let oneshot = event_create_oneshot(Event::RpcRequest(Box::new(evdata)));

            multiqueue_put_event(channel.events, Event::Oneshot(std::rc::Rc::clone(&oneshot)));

            multiqueue_put_event(screen_state().resize_events.get(), Event::Oneshot(oneshot));
        } else {
            multiqueue_put_event(channel.events, Event::RpcRequest(Box::new(evdata)));
        }
    }
}

pub fn request_event(mut e: RequestEvent) {
    let handler = e.handler;

    let is_open = !e.channel.rpc.closed.get();
    if is_open {
        let fn_call = handler.r#fn.unwrap();
        let args = std::mem::take(&mut e.args);
        let mut call_res = fn_call(e.channel.id, args);
        if (e.r#type == MessageType::MessageTypeRequest || call_res.is_err())
            && let Some(channel) = find_channel_opt(e.channel.id)
        {
            serialize_response(
                &channel,
                e.handler,
                e.r#type,
                e.request_id,
                call_res.as_mut().map_err(|err| &*err),
            );
        }
        if handler.ret_alloc
            && let Ok(result) = call_res
        {
            api_free_object(result);
        }
    }

    drop(e);
}

pub fn rpc_write_raw(id: u64, buffer: std::rc::Rc<WBuffer>) -> bool {
    let Some(channel) = find_rpc_channel_opt(id) else {
        return false;
    };

    channel_write(&channel, buffer)
}

fn channel_write(channel: &std::rc::Rc<Channel>, buffer: std::rc::Rc<WBuffer>) -> bool {
    let mut err: i32 = 0;

    if channel.rpc.closed.get() {
        return false;
    }

    if channel.streamtype == K_CHANNEL_STREAM_INTERNAL {
        channel_incref(channel);
        if !channel.events.is_null() {
            multiqueue_put_event(
                channel.events(),
                Event::RpcInternalRead {
                    chan_id: channel.id,
                    buffer,
                },
            );
        } else {
            internal_read_event(channel, buffer);
        }
    } else {
        err = wstream_write(channel_instream(channel), buffer);
    }

    if err != 0 {
        let err_str = uv_strerror(err).to_string_lossy();
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

pub fn internal_read_event(channel: &std::rc::Rc<Channel>, buffer: std::rc::Rc<WBuffer>) {
    let mut read_buf = buffer.data.as_slice();

    if !channel.unpacker_is_closed() {
        parse_msgpack(channel, &mut read_buf);

        if !read_buf.is_empty() && !channel.rpc.closed.get() {
            chan_close_on_err(channel, c"internal channel: internal error", LOGLVL_ERR);
        }
    }

    channel_decref(channel);
}

fn send_error(
    chan: &Channel,
    handler: MsgpackRpcRequestHandler,
    r#type: MessageType,
    id: u32,
    err: Option<std::ffi::CString>,
) {
    let e = Error {
        err_type: ErrorType::ErrorTypeException,
        msg: err,
    };
    serialize_response(chan, handler, r#type, id, Err(&e));
}

fn broadcast_event(name: &CStr, args: Array) {
    if crate::event::r#loop::main_loop()
        .channels()
        .iter()
        .any(|ch| ch.is_rpc.get())
    {
        serialize_request(0, 0, name, args);
    }
}

fn has_undetached_rpc_chan() -> bool {
    for chan in crate::event::r#loop::main_loop().channels() {
        if chan.is_rpc.get() && !chan.rpc.closed.get() && !chan.detach.get() {
            return true;
        }
    }
    false
}

pub fn rpc_close(channel: &Channel) {
    if channel.rpc.closed.get() {
        return;
    }

    channel.rpc.closed.set(true);

    {
        let qc = loop_queues(crate::event::r#loop::main_loop());
        multiqueue_put_event(qc.fast_events.get(), Event::RpcClose(channel.id));
    }
}

pub fn rpc_close_event(chan_id: u64) {
    let Some(channel) = find_channel_opt(chan_id) else {
        return;
    };
    channel_decref(&channel);

    #[cfg(windows)]
    let ui_closed = channel.rpc.has_ui.get();

    let _ = remote_ui_disconnect(channel.id, false);

    let is_ui_client = screen_state().ui_client_channel_id.get() != 0
        && channel.id == screen_state().ui_client_channel_id.get();
    if is_ui_client {
        if !exc_state().exiting.get() && channel.streamtype != K_CHANNEL_STREAM_PROC {
            ui_client_attach_to_restarted_server(false);
        }
        if screen_state().ui_client_channel_id.get() != channel.id {
            return;
        }
        if channel.streamtype == K_CHANNEL_STREAM_PROC
            && screen_state().ui_client_error_exit.get() < 0
        {
            return;
        }
        exit_on_closed_chan(0);
    } else if !channel.detach.get() && !has_undetached_rpc_chan() {
        exit_on_closed_chan(0);
    } else {
        #[cfg(windows)]
        if ui_closed {
            os_detach_ui_console(channel.streamtype == K_CHANNEL_STREAM_STDIO);
        }
    }
}

fn chan_close_on_err(channel: &Channel, msg: &std::ffi::CStr, loglevel: i32) {
    logmsg!(loglevel, Some(c"RPC: "), None, -1, true, c"%s", msg,);

    let frames = channel.rpc.call_stack.borrow().clone();
    for frame in frames {
        if frame.returned.get() {
            continue;
        }
        frame.returned.set(true);
        frame.errored.set(true);
        let s = ensure_nul_string(cstr_as_string(Some(msg)));
        frame.result.set(Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(s),
        });
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
    channel: &Channel,
    handler: MsgpackRpcRequestHandler,
    r#type: MessageType,
    response_id: u32,
    res: Result<&mut Object, &Error>,
) {
    if let Err(err) = res
        && r#type == MessageType::MessageTypeNotification
    {
        let is_paste = handler
            .r#fn
            .is_some_and(|f| f as *const () == handle_nvim_paste as *const ());

        if is_paste {
            semsg!(c"paste: %s", err.msg_cstr());
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
            let args = Array::from_slice(&mut items[..2]);
            serialize_request(channel.id, 0, c"nvim_error_event", args);
        }
        return;
    }

    let mut packer = PackerBuffer::new();
    packer_buffer_init_channels(channel.id, &mut packer);

    mpack_array(&mut packer, 4);
    mpack_w(&mut packer, 1);
    mpack_uint(&mut packer, response_id);

    match res {
        Err(err) => {
            mpack_array(&mut packer, 2);
            mpack_integer(&mut packer, err.err_type as Integer);
            mpack_str(cstr_as_string(err.msg_cstr()), &mut packer);
            mpack_nil(&mut packer);
        }
        Ok(arg) => {
            mpack_nil(&mut packer);
            mpack_object(arg, &mut packer);
        }
    }

    packer_buffer_finish_channels(&mut packer);
}

fn packer_buffer_init_channels(chan_id: u64, packer: &mut PackerBuffer) {
    if chan_id != 0 {
        if let Some(ct) = find_channel_opt(chan_id)
            && ct.rpc.has_ui.get()
        {
            remote_ui_flush_if_incomplete(ct.id);
        }
    } else {
        let rpc_ids: Vec<u64> = crate::event::r#loop::main_loop()
            .channels()
            .iter()
            .filter_map(|ch| ch.is_rpc.get().then_some(ch.id))
            .collect();
        for id in rpc_ids {
            if let Some(ct) = find_channel_opt(id)
                && ct.rpc.has_ui.get()
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
                let _ = channel_write(&chan, buf);
            }
        } else {
            let rpc_chan_ids: Vec<u64> = crate::event::r#loop::main_loop()
                .channels()
                .iter()
                .filter(|ch| ch.is_rpc.get())
                .map(|ch| ch.id)
                .collect();
            for id in rpc_chan_ids {
                if let Some(chan) = find_channel_opt(id) {
                    let _ = channel_write(&chan, std::rc::Rc::clone(&buf));
                }
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

    api_free_dict(chan.rpc.info.replace(info));

    let r#type = get_client_info(&chan, c"type");

    chan.rpc
        .client_type
        .set(match r#type.as_deref().map(CStr::to_bytes) {
            None => K_CLIENT_TYPE_REMOTE,
            Some(b"remote") => K_CLIENT_TYPE_REMOTE,
            Some(b"msgpack-rpc") => K_CLIENT_TYPE_MSGPACK_RPC,
            Some(b"ui") => K_CLIENT_TYPE_UI,
            Some(b"embedder") => K_CLIENT_TYPE_EMBEDDER,
            Some(b"host") => K_CLIENT_TYPE_HOST,
            Some(b"plugin") => K_CLIENT_TYPE_PLUGIN,
            Some(_) => K_CLIENT_TYPE_UNKNOWN,
        });

    channel_event(&chan, EVENT_CHANINFO);
}

pub fn get_client_info(chan: &Channel, key: &CStr) -> Option<std::ffi::CString> {
    let ct = chan;
    if !ct.is_rpc.get() {
        return None;
    }
    let key_bytes = key.to_bytes();
    let info = &ct.rpc.info.get();
    for item in info.as_slice() {
        if !item.key.data.is_null()
            && item.key.cstr_bytes() == key_bytes
            && item.value.obj_type == ObjectType::ObjectTypeString
            && let Some(cstr) = item.value.data.as_opt_cstr()
        {
            return Some(cstr.to_owned());
        }
    }

    None
}

pub fn rpc_free_all_mem() {
    multiqueue_free(
        input_state()
            .ch_before_blocking_events
            .replace(MultiQueue::NULL),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_request_releases_channel_after_registry_removal() {
        let queue = crate::event::multiqueue::multiqueue_new(None);
        let channel = std::rc::Rc::new(Channel::new(u64::MAX, K_CHANNEL_STREAM_INTERNAL, queue));
        let weak = std::rc::Rc::downgrade(&channel);
        let loop_ = crate::event::r#loop::main_loop();
        loop_.insert_channel(channel.clone());
        channel_incref(&channel);
        let request = RequestEvent {
            r#type: MessageType::MessageTypeRequest,
            channel: channel.clone(),
            handler: MsgpackRpcRequestHandler {
                name: c"test",
                r#fn: None,
                fast: false,
                ret_alloc: false,
            },
            args: Array::new(),
            request_id: 1,
        };
        channel_decref(&channel);
        drop(loop_.remove_channel(channel.id));
        drop(channel);
        assert_eq!(weak.upgrade().unwrap().refcount.get(), 1);
        drop(request);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn find_call_frame_searches_stack_from_top() {
        let rpc = RpcState::new();
        rpc.next_request_id.set(1);
        rpc.client_type.set(K_CLIENT_TYPE_MSGPACK_RPC);
        let f1 = std::rc::Rc::new(ChannelCallFrame {
            request_id: 10,
            returned: std::cell::Cell::new(false),
            errored: std::cell::Cell::new(false),
            result: crate::api::private::validate::CloneCell::new(OBJECT_NIL),
        });
        let f2 = std::rc::Rc::new(ChannelCallFrame {
            request_id: 20,
            returned: std::cell::Cell::new(false),
            errored: std::cell::Cell::new(false),
            result: crate::api::private::validate::CloneCell::new(OBJECT_NIL),
        });
        rpc.call_stack.borrow_mut().push(f1.clone());
        rpc.call_stack.borrow_mut().push(f2.clone());

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
