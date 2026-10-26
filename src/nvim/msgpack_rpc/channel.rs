//! Msgpack-RPC channel transport and request/response dispatch (`src/nvim/msgpack_rpc/channel.c`).

use crate::api::private::dispatch::MsgpackRpcRequestHandler;
use crate::api::private::helpers::{
    OBJECT_NIL, api_clear_error, api_free_dict, api_free_object, api_set_error, cstr_as_string,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, Object, ObjectData, ObjectType,
};
use crate::api::ui::{remote_ui_disconnect, remote_ui_flush_pending_data};
use crate::channel::{
    CallbackReader, InternalState, channel_close, channel_decref, channel_event, channel_incref,
    kChannelPartRpc, kChannelStreamProc, kChannelStreamSocket, kChannelStreamStdio,
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
use crate::event::wstream::{WBuffer, WBufferFinalizer, wstream_new_buffer, wstream_write};
use crate::globals::*;
use crate::log::logmsg;
use crate::memory::{
    ARENA_BLOCK_SIZE, Arena, ArenaMem, alloc_block, arena_finish, arena_mem_free, free_block,
};
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
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type ClientType = c_int;
pub type ChannelStreamType = c_int;
pub type ChannelPart = c_int;

#[cfg(windows)]
const UV_EPIPE: c_int = -4047;
#[cfg(not(windows))]
const UV_EPIPE: c_int = -32;

const kClientTypeUnknown: ClientType = -1;
const kClientTypeRemote: ClientType = 0;
const kClientTypeUi: ClientType = 1;
const kClientTypeEmbedder: ClientType = 2;
const kClientTypeHost: ClientType = 3;
const kClientTypePlugin: ClientType = 4;
const kClientTypeMsgpackRpc: ClientType = 5;

use crate::api::private::helpers::arena_string;

#[repr(C)]
pub struct RemoteUI {
    pub rgb: bool,
    pub r#override: bool,
    pub composed: bool,
    pub ui_ext: [bool; kUIExtCount],
    pub width: c_int,
    pub height: c_int,
    pub pum_nlines: c_int,
    pub pum_pos: bool,
    pub pum_row: f64,
    pub pum_col: f64,
    pub pum_height: f64,
    pub pum_width: f64,
    pub term_name: Option<std::ffi::CString>,
    pub term_background: *mut c_char,
    pub term_colors: c_int,
    pub stdin_tty: bool,
    pub stdout_tty: bool,
    pub channel_id: u64,
    pub packer: PackerBuffer,
    pub cur_event: *const c_char,
    pub nevents_pos: *mut c_char,
    pub ncalls_pos: *mut c_char,
    pub nevents: u32,
    pub ncalls: u32,
    pub flushed_events: bool,
    pub incomplete_event: bool,
    pub ncells_pending: usize,
    pub hl_id: c_int,
    pub cursor_row: Integer,
    pub cursor_col: Integer,
    pub client_row: Integer,
    pub client_col: Integer,
    pub wildmenu_active: bool,
}

impl Drop for RemoteUI {
    fn drop(&mut self) {
        unsafe {
            if !self.packer.startptr.is_null() {
                free_block(self.packer.startptr);
            }
        }
    }
}

#[repr(C)]
pub struct ChannelCallFrame {
    pub request_id: u32,
    pub returned: bool,
    pub errored: bool,
    pub result: Object,
    pub result_mem: ArenaMem,
}

#[repr(C)]
pub struct RequestEvent {
    pub r#type: MessageType,
    pub channel: *mut Channel,
    pub handler: MsgpackRpcRequestHandler,
    pub args: Array,
    pub request_id: u32,
    pub used_mem: Arena,
}

impl Drop for RequestEvent {
    fn drop(&mut self) {
        unsafe {
            arena_mem_free(arena_finish(&mut self.used_mem));
            channel_decref(self.channel);
        }
    }
}

#[repr(C)]
pub struct RpcState {
    pub closed: bool,
    pub unpacker: Option<Box<Unpacker>>,
    pub ui: *mut RemoteUI,
    pub next_request_id: u32,
    pub call_stack: Vec<*mut ChannelCallFrame>,
    pub info: Dict,
    pub client_type: ClientType,
}

impl RpcState {
    pub const fn new() -> Self {
        Self {
            closed: false,
            unpacker: None,
            ui: ptr::null_mut(),
            next_request_id: 0,
            call_stack: Vec::new(),
            info: Dict {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            },
            client_type: 0,
        }
    }
}

impl Drop for RpcState {
    fn drop(&mut self) {
        unsafe {
            api_free_dict(self.info);
        }
    }
}

#[repr(C)]
pub struct Channel {
    pub id: u64,
    pub refcount: usize,
    pub events: *mut MultiQueue,
    pub streamtype: ChannelStreamType,
    pub uv: LibuvProc,
    pub socket: RStream,
    pub stdio_in: RStream,
    pub stdio_out: Stream,
    pub err_closed: bool,
    pub internal: InternalState,
    pub is_rpc: bool,
    pub detach: bool,
    pub rpc: RpcState,
    pub term: *mut crate::terminal::Terminal,
    pub on_data: CallbackReader,
    pub on_stderr: CallbackReader,
    pub on_exit: Callback,
    pub exit_status: c_int,
    pub callback_busy: bool,
    pub callback_scheduled: bool,
    pub did_close_event: bool,
}

impl Drop for Channel {
    fn drop(&mut self) {
        unsafe {
            crate::eval::typval::callback_free(&mut self.on_exit);
            if !self.events.is_null() {
                multiqueue_free(self.events);
            }
        }
    }
}

use crate::api::private::dispatch::{
    handle_nvim_get_mode, handle_nvim_paste, handle_nvim_ui_try_resize,
};
use crate::api::ui::kUIExtCount;
use crate::api::vim::kChannelStreamInternal;
use crate::autocmd::EVENT_CHANINFO;
use crate::log::{LOGLVL_ERR, LOGLVL_INF};

#[inline]
pub unsafe fn channel_instream(chan: *mut Channel) -> *mut Stream {
    unsafe {
        match (*chan).streamtype {
            kChannelStreamProc => &raw mut (*chan).uv.proc_.r#in,
            kChannelStreamSocket => &raw mut (*chan).socket.s,
            kChannelStreamStdio => &raw mut (*chan).stdio_out,
            _ => std::process::abort(),
        }
    }
}

#[inline]
pub unsafe fn channel_outstream(chan: *mut Channel) -> *mut RStream {
    unsafe {
        match (*chan).streamtype {
            kChannelStreamProc => &raw mut (*chan).uv.proc_.out,
            kChannelStreamSocket => &raw mut (*chan).socket,
            kChannelStreamStdio => &raw mut (*chan).stdio_in,
            _ => std::process::abort(),
        }
    }
}

#[inline]
pub unsafe fn find_channel(id: u64) -> *mut Channel {
    unsafe {
        let chans_ptr = &raw const channels;
        (*chans_ptr).get(&id).copied().unwrap_or(ptr::null_mut())
    }
}

#[inline]
fn unpacker_closed(p: *const Unpacker) -> bool {
    unsafe { (*p).state < 0 }
}

pub unsafe fn rpc_init() {
    unsafe {
        let qc = loop_queues((&raw mut main_loop).cast());
        ch_before_blocking_events = multiqueue_new_child((*qc).events);
    }
}

pub unsafe fn rpc_start(channel: *mut Channel) {
    unsafe {
        channel_incref(channel);

        let cp = channel;
        let ct = channel;
        (*ct).is_rpc = true;
        let rpc = &raw mut (*ct).rpc;
        (*rpc).closed = false;
        let mut unpacker = Box::new(Unpacker::new());
        unpacker_init(&raw mut *unpacker);
        (*rpc).unpacker = Some(unpacker);
        (*rpc).next_request_id = 1;
        (*rpc).info = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        (*rpc).call_stack.clear();

        if (*cp).streamtype != kChannelStreamInternal {
            let out = channel_outstream(channel);
            rstream_start(out, Some(receive_msgpack), StreamCbData::Channel(channel));
        }
    }
}

#[inline]
unsafe fn find_rpc_channel(id: u64) -> *mut Channel {
    unsafe {
        let chan = find_channel(id);
        if chan.is_null() {
            return ptr::null_mut();
        }
        let ct = chan;
        if !(*ct).is_rpc || (*ct).rpc.closed {
            return ptr::null_mut();
        }
        chan
    }
}

pub unsafe fn rpc_send_event(id: u64, name: *const c_char, args: Array) -> bool {
    unsafe {
        let mut channel: *mut Channel = ptr::null_mut();
        if id != 0 {
            channel = find_rpc_channel(id);
            if channel.is_null() {
                return false;
            }
        }

        if !channel.is_null() {
            serialize_request(&raw mut channel, 1, 0, name, args);
        } else {
            broadcast_event(name, args);
        }

        true
    }
}

pub unsafe fn rpc_send_call(
    id: u64,
    method_name: *const c_char,
    args: Array,
    result_mem: *mut ArenaMem,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut channel = find_rpc_channel(id);
        if channel.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Invalid channel: %llu".as_ptr(),
                id,
            );
            return OBJECT_NIL;
        }

        channel_incref(channel);

        let cp = channel;
        let ct = channel;
        let rpc = &raw mut (*ct).rpc;
        let request_id = (*rpc).next_request_id;
        (*rpc).next_request_id += 1;

        serialize_request(&raw mut channel, 1, request_id, method_name, args);

        let mut frame = ChannelCallFrame {
            request_id,
            returned: false,
            errored: false,
            result: OBJECT_NIL,
            result_mem: ptr::null_mut(),
        };
        let frame_ptr: *mut ChannelCallFrame = &raw mut frame;
        (*rpc).call_stack.push(frame_ptr);

        while !ptr::read_volatile(&raw const (*frame_ptr).returned)
            && !ptr::read_volatile(&raw const (*rpc).closed)
        {
            if !(*cp).events.is_null() && !multiqueue_empty((*cp).events) {
                multiqueue_process_events((*cp).events);
            } else {
                loop_poll_events((&raw mut main_loop).cast(), -1);
            }
        }
        (*rpc).call_stack.pop();

        if !(*frame_ptr).returned {
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Invalid channel: %llu".as_ptr(),
                    id,
                );
                channel_decref(channel);
            }
            return OBJECT_NIL;
        }

        if (*frame_ptr).errored {
            let res = (*frame_ptr).result;
            if res.obj_type == ObjectType::kObjectTypeString {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    res.data.string().data,
                );
            } else if res.obj_type == ObjectType::kObjectTypeArray {
                let array = res.data.array();
                if array.size == 2
                    && (*array.items).obj_type == ObjectType::kObjectTypeInteger
                    && let Some(et) = ErrorType::from_raw((*array.items).data.integer() as c_int)
                    && matches!(
                        et,
                        ErrorType::kErrorTypeException | ErrorType::kErrorTypeValidation
                    )
                    && array[1].obj_type == ObjectType::kObjectTypeString
                {
                    api_set_error!(
                        err,
                        et,
                        c"%s".as_ptr(),
                        array[1].data.string().data,
                    );
                } else {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"%s".as_ptr(),
                        c"unknown error".as_ptr(),
                    );
                }
            } else {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    c"unknown error".as_ptr(),
                );
            }

            arena_mem_free((*frame_ptr).result_mem);
            (*frame_ptr).result_mem = ptr::null_mut();
        }

        channel_decref(channel);

        *result_mem = (*frame_ptr).result_mem;

        if (*frame_ptr).errored {
            OBJECT_NIL
        } else {
            (*frame_ptr).result
        }
    }
}

unsafe fn receive_msgpack(
    _stream: *mut RStream,
    rbuf: *const c_char,
    c: usize,
    data: StreamCbData,
    eof: bool,
) -> usize {
    unsafe {
        let channel = data.as_channel();
        channel_incref(channel);

        let mut consumed: usize = 0;
        let cp = channel;
        let ct = channel;

        if c > 0 {
            let p: *mut Unpacker = (*ct).rpc.unpacker.as_deref_mut().unwrap();
            (*p).read_ptr = rbuf;
            (*p).read_size = c;
            parse_msgpack(channel);

            if !unpacker_closed(p) {
                consumed = c - (*p).read_size;
            }
        }

        if eof {
            let msg = format!("ch {} was closed by the peer\0", (*cp).id);
            chan_close_on_err(channel, msg.as_ptr().cast::<c_char>(), LOGLVL_INF);
        }

        channel_decref(channel);

        consumed
    }
}

unsafe fn find_call_frame(rpc: *mut RpcState, request_id: u32) -> *mut ChannelCallFrame {
    unsafe {
        for &frame in (*rpc).call_stack.iter().rev() {
            if (*frame).request_id == request_id {
                return frame;
            }
        }
        ptr::null_mut()
    }
}

unsafe fn parse_msgpack(channel: *mut Channel) {
    unsafe {
        let cp = channel;
        let ct = channel;
        let p: *mut Unpacker = (*ct).rpc.unpacker.as_deref_mut().unwrap();

        while unpacker_advance(p) {
            if (*p).msg_type == MessageType::kMessageTypeRedrawEvent {
                if ui_client_attached {
                    if (*p).has_grid_line_event {
                        ui_client_event_raw_line(&raw mut (*p).grid_line_event);
                        (*p).has_grid_line_event = false;
                    } else if let Some(ui_fn) = (*p).ui_handler.r#fn
                        && (*p).result.obj_type == ObjectType::kObjectTypeArray
                    {
                        ui_fn((*p).result.data.array());
                    }
                }
                arena_mem_free(arena_finish(&raw mut (*p).arena));
            } else if (*p).msg_type == MessageType::kMessageTypeResponse {
                let frame = if (*ct).rpc.client_type == kClientTypeMsgpackRpc {
                    find_call_frame(&raw mut (*ct).rpc, (*p).request_id)
                } else {
                    (*ct)
                        .rpc
                        .call_stack
                        .last()
                        .copied()
                        .unwrap_or(ptr::null_mut())
                };

                if frame.is_null() || (*p).request_id != (*frame).request_id {
                    let msg = format!(
                        "ch {} (type={}) returned a response with an unknown request id {}. Ensure the client is properly synchronized\0",
                        (*cp).id,
                        (*ct).rpc.client_type as u32,
                        (*p).request_id
                    );
                    chan_close_on_err(channel, msg.as_ptr().cast::<c_char>(), LOGLVL_ERR);
                    return;
                }

                (*frame).returned = true;
                (*frame).errored = (*p).error.obj_type != ObjectType::kObjectTypeNil;

                if (*frame).errored {
                    (*frame).result = (*p).error;
                } else {
                    (*frame).result = (*p).result;
                }
                (*frame).result_mem = arena_finish(&raw mut (*p).arena);
            } else {
                let res = (*p).result;
                if (*p).result.obj_type != ObjectType::kObjectTypeArray {
                    chan_close_on_err(
                        channel,
                        c"msgpack-rpc request args must be an array".as_ptr(),
                        LOGLVL_ERR,
                    );
                    return;
                }
                handle_request(channel, p, res.data.array());
            }
        }

        if unpacker_closed(p) {
            chan_close_on_err(channel, (*p).unpack_error.msg_ptr(), LOGLVL_INF);
            api_clear_error(&raw mut (*p).unpack_error);
        }
    }
}

unsafe fn handle_request(channel: *mut Channel, p: *mut Unpacker, args: Array) {
    unsafe {
        assert!(
            (*p).msg_type == MessageType::kMessageTypeRequest
                || (*p).msg_type == MessageType::kMessageTypeNotification
        );

        if (*p).handler.r#fn.is_none() {
            send_error(
                channel,
                (*p).handler,
                (*p).msg_type,
                (*p).request_id,
                (*p).unpack_error.msg_ptr(),
            );
            api_clear_error(&raw mut (*p).unpack_error);
            arena_mem_free(arena_finish(&raw mut (*p).arena));
            return;
        }

        let cp = channel;
        let evdata = RequestEvent {
            r#type: (*p).msg_type,
            channel,
            handler: (*p).handler,
            args,
            used_mem: (*p).arena,
            request_id: (*p).request_id,
        };
        (*p).arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        channel_incref(channel);

        if (*p).handler.fast {
            let is_get_mode = (*p)
                .handler
                .r#fn
                .is_some_and(|f| f as *const () == handle_nvim_get_mode as *const ());

            if is_get_mode && !input_blocking() {
                multiqueue_put_event(
                    ch_before_blocking_events,
                    Event::RpcRequest(Box::new(evdata)),
                );
            } else {
                request_event(evdata);
            }
        } else {
            let is_resize = (*p)
                .handler
                .r#fn
                .is_some_and(|f| f as *const () == handle_nvim_ui_try_resize as *const ());

            if is_resize {
                let oneshot = event_create_oneshot(Event::RpcRequest(Box::new(evdata)));
                multiqueue_put_event((*cp).events, Event::Oneshot(std::rc::Rc::clone(&oneshot)));
                multiqueue_put_event(resize_events, Event::Oneshot(oneshot));
            } else {
                multiqueue_put_event((*cp).events, Event::RpcRequest(Box::new(evdata)));
            }
        }
    }
}

pub unsafe fn request_event(mut e: RequestEvent) {
    unsafe {
        let channel = e.channel;
        let cp = channel;
        let ct = channel;
        let handler = e.handler;
        let mut error = Error::new();

        if !(*ct).rpc.closed {
            let fn_call = handler.r#fn.unwrap();
            let mut result = fn_call((*cp).id, e.args, &raw mut e.used_mem, &raw mut error);
            if e.r#type == MessageType::kMessageTypeRequest
                || error.err_type != ErrorType::kErrorTypeNone
            {
                serialize_response(
                    channel,
                    e.handler,
                    e.r#type,
                    e.request_id,
                    &raw mut error,
                    &raw mut result,
                );
            }
            if handler.ret_alloc {
                api_free_object(result);
            }
        }

        drop(e);
        api_clear_error(&raw mut error);
    }
}

pub unsafe fn rpc_write_raw(id: u64, buffer: std::rc::Rc<WBuffer>) -> bool {
    unsafe {
        let channel = find_rpc_channel(id);
        if channel.is_null() {
            return false;
        }

        channel_write(channel, buffer)
    }
}

unsafe fn channel_write(channel: *mut Channel, buffer: std::rc::Rc<WBuffer>) -> bool {
    unsafe {
        let cp = channel;
        let ct = channel;
        let mut err: c_int = 0;

        if (*ct).rpc.closed {
            return false;
        }

        if (*cp).streamtype == kChannelStreamInternal {
            channel_incref(channel);
            if !(*cp).events.is_null() {
                multiqueue_put_event((*cp).events, Event::RpcInternalRead { channel, buffer });
            } else {
                internal_read_event(channel, buffer);
            }
        } else {
            let in_stream = channel_instream(channel);
            err = wstream_write(in_stream, buffer);
        }

        if err != 0 {
            let err_str = CStr::from_ptr(uv_strerror(err)).to_string_lossy();
            let msg = format!(
                "ch {}: stream write failed: {err_str}. RPC canceled; closing channel\0",
                (*cp).id
            );
            chan_close_on_err(
                channel,
                msg.as_ptr().cast::<c_char>(),
                if err == UV_EPIPE {
                    LOGLVL_INF
                } else {
                    LOGLVL_ERR
                },
            );
        }

        err == 0
    }
}

pub unsafe fn internal_read_event(channel: *mut Channel, buffer: std::rc::Rc<WBuffer>) {
    unsafe {
        let ct = channel;
        let p: *mut Unpacker = (*ct).rpc.unpacker.as_deref_mut().unwrap();

        (*p).read_ptr = buffer.data;
        (*p).read_size = buffer.size;
        if !unpacker_closed(p) {
            parse_msgpack(channel);

            if (*p).read_size != 0 && !(*ct).rpc.closed {
                chan_close_on_err(
                    channel,
                    c"internal channel: internal error".as_ptr(),
                    LOGLVL_ERR,
                );
            }
        }

        channel_decref(channel);
    }
}

unsafe fn send_error(
    chan: *mut Channel,
    handler: MsgpackRpcRequestHandler,
    r#type: MessageType,
    id: u32,
    err: *const c_char,
) {
    unsafe {
        let mut e = Error::new();
        api_set_error!(
            &raw mut e,
            ErrorType::kErrorTypeException,
            c"%s".as_ptr(),
            err,
        );
        let mut nil_obj = OBJECT_NIL;
        serialize_response(chan, handler, r#type, id, &raw mut e, &raw mut nil_obj);
        api_clear_error(&raw mut e);
    }
}

unsafe fn broadcast_event(name: *const c_char, args: Array) {
    unsafe {
        let mut chans = Vec::<*mut Channel>::new();

        let chans_ptr = &raw const channels;
        for &channel in (*chans_ptr).values() {
            if (*channel).is_rpc {
                chans.push(channel);
            }
        }

        if !chans.is_empty() {
            serialize_request(chans.as_mut_ptr(), chans.len(), 0, name, args);
        }
    }
}

unsafe fn has_undetached_rpc_chan() -> bool {
    unsafe {
        let chans_ptr = &raw const channels;
        for &chan in (*chans_ptr).values() {
            let ct = chan;
            if (*ct).is_rpc && !(*ct).rpc.closed && !(*ct).detach {
                return true;
            }
        }
    }
    false
}

pub unsafe fn rpc_close(channel: *mut Channel) {
    unsafe {
        let ct = channel;
        if (*ct).rpc.closed {
            return;
        }

        (*ct).rpc.closed = true;

        {
            let qc = loop_queues((&raw mut main_loop).cast());
            multiqueue_put_event((*qc).fast_events, Event::RpcClose(channel));
        }
    }
}

pub unsafe fn rpc_close_event(channel: *mut Channel) {
    unsafe {
        assert!(!channel.is_null());

        channel_decref(channel);

        let cp = channel;
        let ct = channel;

        #[cfg(windows)]
        let ui_closed = !(*ct).rpc.ui.is_null();

        {
            remote_ui_disconnect((*cp).id, ptr::null_mut(), false);

            let is_ui_client = ui_client_channel_id != 0 && (*cp).id == ui_client_channel_id;
            if is_ui_client {
                if !exiting && (*cp).streamtype != kChannelStreamProc {
                    ui_client_attach_to_restarted_server(false);
                }
                if ui_client_channel_id != (*cp).id {
                    return;
                }
                if (*cp).streamtype == kChannelStreamProc && ui_client_error_exit < 0 {
                    return;
                }
                exit_on_closed_chan(0);
            } else if !(*ct).detach && !has_undetached_rpc_chan() {
                exit_on_closed_chan(0);
            } else {
                #[cfg(windows)]
                if ui_closed {
                    os_detach_ui_console((*cp).streamtype == kChannelStreamStdio);
                }
            }
        }
    }
}

unsafe fn chan_close_on_err(channel: *mut Channel, msg: *const c_char, loglevel: c_int) {
    unsafe {
        logmsg!(
            loglevel,
            c"RPC: ".as_ptr(),
            ptr::null(),
            -1,
            true,
            c"%s".as_ptr(),
            msg,
        );

        let cp = channel;
        let ct = channel;
        let p: *mut Unpacker = (*ct).rpc.unpacker.as_deref_mut().unwrap();

        for &frame in &(*ct).rpc.call_stack {
            if (*frame).returned {
                continue;
            }
            (*frame).returned = true;
            (*frame).errored = true;
            let s = arena_string(&raw mut (*p).arena, cstr_as_string(msg));
            (*frame).result = Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData::String(s),
            };
            (*frame).result_mem = arena_finish(&raw mut (*p).arena);
        }

        {
            channel_close((*cp).id, kChannelPartRpc, ptr::null_mut());
            logmsg!(
                loglevel,
                ptr::null(),
                c"chan_close_on_err".as_ptr(),
                492,
                true,
                c"RPC: %s".as_ptr(),
                msg,
            );
        }
    }
}

unsafe fn serialize_request(
    chans: *mut *mut Channel,
    nchans: usize,
    request_id: u32,
    method: *const c_char,
    args: Array,
) {
    unsafe {
        let mut packer = PackerBuffer::new();
        packer_buffer_init_channels(chans, nchans, &raw mut packer);

        mpack_array(&raw mut packer.ptr, if request_id != 0 { 4 } else { 3 });
        mpack_w(&raw mut packer.ptr, if request_id != 0 { 0 } else { 2 });

        if request_id != 0 {
            mpack_uint(&raw mut packer.ptr, request_id);
        }

        mpack_str(cstr_as_string(method), &raw mut packer);
        mpack_object_array(args, &raw mut packer);

        packer_buffer_finish_channels(&raw mut packer);
    }
}

pub unsafe fn serialize_response(
    mut channel: *mut Channel,
    handler: MsgpackRpcRequestHandler,
    r#type: MessageType,
    response_id: u32,
    err: *mut Error,
    arg: *mut Object,
) {
    unsafe {
        if (*err).err_type != ErrorType::kErrorTypeNone
            && r#type == MessageType::kMessageTypeNotification
        {
            let is_paste = handler
                .r#fn
                .is_some_and(|f| f as *const () == handle_nvim_paste as *const ());

            if is_paste {
                {
                    semsg!(c"paste: %s".as_ptr(), (*err).msg_ptr());
                    api_clear_error(err);
                }
            } else {
                let mut items = [
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer((*err).err_type as i64),
                    },
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData::String(cstr_as_string((*err).msg_ptr())),
                    },
                ];
                let args = Array {
                    items: items.as_mut_ptr(),
                    size: 2,
                    capacity: 2,
                };
                serialize_request(&raw mut channel, 1, 0, c"nvim_error_event".as_ptr(), args);
            }
            return;
        }

        let mut packer = PackerBuffer::new();
        packer_buffer_init_channels(&raw mut channel, 1, &raw mut packer);

        mpack_array(&raw mut packer.ptr, 4);
        mpack_w(&raw mut packer.ptr, 1);
        mpack_uint(&raw mut packer.ptr, response_id);

        if (*err).err_type != ErrorType::kErrorTypeNone {
            mpack_array(&raw mut packer.ptr, 2);
            mpack_integer(&raw mut packer.ptr, (*err).err_type as Integer);
            mpack_str(cstr_as_string((*err).msg_ptr()), &raw mut packer);
            mpack_nil(&raw mut packer.ptr);
        } else {
            mpack_nil(&raw mut packer.ptr);
            mpack_object(arg, &raw mut packer);
        }

        packer_buffer_finish_channels(&raw mut packer);
    }
}

unsafe fn packer_buffer_init_channels(
    chans: *mut *mut Channel,
    nchans: usize,
    packer: *mut PackerBuffer,
) {
    unsafe {
        for i in 0..nchans {
            let chan = *chans.add(i);
            let ct = chan;
            if !(*ct).rpc.ui.is_null() && (*(*ct).rpc.ui).incomplete_event {
                remote_ui_flush_pending_data((*ct).rpc.ui);
            }
        }
        (*packer).startptr = alloc_block();
        (*packer).ptr = (*packer).startptr;
        (*packer).endptr = (*packer).startptr.add(ARENA_BLOCK_SIZE);
        (*packer).packer_flush = Some(channel_flush_callback);
        (*packer).anydata = crate::msgpack_rpc::packer::PackerData::Chans(chans);
        (*packer).anyint = nchans as i64;
    }
}

unsafe fn packer_buffer_finish_channels(packer: *mut PackerBuffer) {
    unsafe {
        let len = ((*packer).ptr as usize) - ((*packer).startptr as usize);
        if len > 0 {
            let buf = wstream_new_buffer((*packer).startptr, len, WBufferFinalizer::FreeBlock);
            let chans = (*packer).anydata.as_chans();
            for i in 0..((*packer).anyint as usize) {
                let _ = channel_write(*chans.add(i), std::rc::Rc::clone(&buf));
            }
        } else {
            free_block((*packer).startptr);
        }
    }
}

unsafe fn channel_flush_callback(packer: *mut PackerBuffer) {
    unsafe {
        packer_buffer_finish_channels(packer);
        packer_buffer_init_channels(
            (*packer).anydata.as_chans(),
            (*packer).anyint as usize,
            packer,
        );
    }
}

pub unsafe fn rpc_set_client_info(id: u64, info: Dict) {
    unsafe {
        let chan = find_rpc_channel(id);
        if chan.is_null() {
            std::process::abort();
        }

        let ct = chan;
        api_free_dict((*ct).rpc.info);
        (*ct).rpc.info = info;

        let r#type = get_client_info(chan, c"type".as_ptr());
        (*ct).rpc.client_type = if r#type.is_null() {
            kClientTypeRemote
        } else {
            match CStr::from_ptr(r#type).to_bytes() {
                b"remote" => kClientTypeRemote,
                b"msgpack-rpc" => kClientTypeMsgpackRpc,
                b"ui" => kClientTypeUi,
                b"embedder" => kClientTypeEmbedder,
                b"host" => kClientTypeHost,
                b"plugin" => kClientTypePlugin,
                _ => kClientTypeUnknown,
            }
        };

        channel_event(chan, EVENT_CHANINFO);
    }
}

pub unsafe fn get_client_info(chan: *mut Channel, key: *const c_char) -> *const c_char {
    unsafe {
        let ct = chan;
        if !(*ct).is_rpc {
            return ptr::null();
        }
        let key_bytes = CStr::from_ptr(key).to_bytes();
        let info = (*ct).rpc.info;
        for i in 0..info.size {
            let item = &*info.items.add(i);
            if !item.key.data.is_null()
                && CStr::from_ptr(item.key.data).to_bytes() == key_bytes
                && item.value.obj_type == ObjectType::kObjectTypeString
            {
                return item.value.data.string().data;
            }
        }

        ptr::null()
    }
}

pub unsafe fn rpc_free_all_mem() {
    unsafe {
        multiqueue_free(ch_before_blocking_events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_call_frame_searches_stack_from_top() {
        unsafe {
            let mut rpc = RpcState {
                closed: false,
                unpacker: None,
                ui: ptr::null_mut(),
                next_request_id: 1,
                call_stack: Vec::new(),
                info: Dict {
                    items: ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                },
                client_type: kClientTypeMsgpackRpc,
            };
            let mut f1 = ChannelCallFrame {
                request_id: 10,
                returned: false,
                errored: false,
                result: OBJECT_NIL,
                result_mem: ptr::null_mut(),
            };
            let mut f2 = ChannelCallFrame {
                request_id: 20,
                returned: false,
                errored: false,
                result: OBJECT_NIL,
                result_mem: ptr::null_mut(),
            };
            rpc.call_stack.push(&raw mut f1);
            rpc.call_stack.push(&raw mut f2);

            assert_eq!(find_call_frame(&raw mut rpc, 20), &raw mut f2);
            assert_eq!(find_call_frame(&raw mut rpc, 10), &raw mut f1);
            assert!(find_call_frame(&raw mut rpc, 99).is_null());
        }
    }
}
