//! Msgpack-RPC channel transport and request/response dispatch (`src/nvim/msgpack_rpc/channel.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_variables,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::private::helpers::{api_clear_error, api_free_dict, api_free_object, copy_dict};
#[allow(unused_imports)]
use crate::api::ui::remote_ui_disconnect;
#[allow(unused_imports)]
use crate::api::ui::remote_ui_flush_pending_data;
#[allow(unused_imports)]
use crate::channel::{channel_close, channel_decref, channel_event, channel_incref};
#[allow(unused_imports)]
use crate::event::r#loop::{uv_handle_size, uv_strerror};
#[allow(unused_imports)]
use crate::event::proc::exit_on_closed_chan;
use crate::globals::*;
use crate::log::logmsg;
use crate::message::semsg;
#[cfg(windows)]
#[allow(unused_imports)]
use crate::os::os_win_console::os_detach_ui_console;
#[allow(unused_imports)]
use crate::ui_client::{ui_client_attach_to_restarted_server, ui_client_event_raw_line};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, Object, ObjectData, ObjectType,
};
use crate::eval::deprecated::{Callback, CallbackReader};
use crate::event::r#loop::{Loop, loop_poll_events, loop_queues};
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, event_create_oneshot, multiqueue_empty,
    multiqueue_free, multiqueue_new_child, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::proc::{proc_in, proc_out, proc_size};
use crate::event::rstream::{RStream, rstream_size, rstream_start};
use crate::event::stream::Stream;
use crate::event::wstream::{WBuffer, wstream_new_buffer, wstream_release_wbuffer, wstream_write};
use crate::kvec::KVec;
use crate::map::{Map, mh_get_uint64_t, ptr_t};
use crate::map_glyph_cache::{MH_TOMBSTONE, NvimString};
use crate::memory::{
    ARENA_BLOCK_SIZE, Arena, ArenaMem, alloc_block, arena_finish, arena_mem_free, arena_memdupz,
    free_block, xcalloc, xfree, xmalloc,
};
use crate::msgpack_rpc::packer::{
    Integer, PackerBuffer, mpack_array, mpack_integer, mpack_nil, mpack_object, mpack_object_array,
    mpack_str, mpack_uint, mpack_w,
};
use crate::msgpack_rpc::unpacker::{
    GridLineEvent, MessageType, MsgpackRpcRequestHandler, Unpacker, unpacker_advance,
    unpacker_init, unpacker_teardown,
};
use crate::os::input::input_blocking;

pub use crate::eval::deprecated::Channel;
pub type ClientType = c_int;
pub type ChannelStreamType = c_int;
pub type ChannelPart = c_int;
pub use crate::api::autocmd::event_T;

const UV_PROCESS: c_int = 10;

#[cfg(windows)]
const UV_EPIPE: c_int = -4047;
#[cfg(not(windows))]
const UV_EPIPE: c_int = -32;

const LOGLVL_DBG: c_int = 0;
const LOGLVL_INF: c_int = 1;
const LOGLVL_ERR: c_int = 3;

const EVENT_CHANINFO: event_T = 23;

const kClientTypeUnknown: ClientType = -1;
const kClientTypeRemote: ClientType = 0;
const kClientTypeUi: ClientType = 1;
const kClientTypeEmbedder: ClientType = 2;
const kClientTypeHost: ClientType = 3;
const kClientTypePlugin: ClientType = 4;
const kClientTypeMsgpackRpc: ClientType = 5;

const kChannelStreamProc: ChannelStreamType = 0;
const kChannelStreamSocket: ChannelStreamType = 1;
const kChannelStreamStdio: ChannelStreamType = 2;
const kChannelStreamStderr: ChannelStreamType = 3;
const kChannelStreamInternal: ChannelStreamType = 4;

const kChannelPartStdin: ChannelPart = 0;
const kChannelPartStdout: ChannelPart = 1;
const kChannelPartStderr: ChannelPart = 2;
const kChannelPartRpc: ChannelPart = 3;
const kChannelPartAll: ChannelPart = 4;

const kUIExtCount: usize = 10;

const NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { boolean: false },
};

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        unsafe {
            NvimString {
                data: s.cast_mut(),
                size: CStr::from_ptr(s).to_bytes().len(),
            }
        }
    }
}

#[inline]
unsafe fn arena_string(arena: *mut Arena, str_: NvimString) -> NvimString {
    if str_.size == 0 || str_.data.is_null() {
        NvimString {
            data: c"".as_ptr().cast_mut(),
            size: 0,
        }
    } else {
        unsafe {
            NvimString {
                data: arena_memdupz(arena, str_.data, str_.size),
                size: str_.size,
            }
        }
    }
}

unsafe extern "C" fn free_block_void(block: *mut c_void) {
    unsafe {
        free_block(block.cast());
    }
}

#[cfg(windows)]
type uv_uid_t = u8;
#[cfg(not(windows))]
type uv_uid_t = u32;

#[cfg(windows)]
type uv_gid_t = u8;
#[cfg(not(windows))]
type uv_gid_t = u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct UvStdioContainer {
    flags: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct UvProcessOptions {
    exit_cb: *const c_void,
    file: *const c_char,
    args: *mut *mut c_char,
    env: *mut *mut c_char,
    cwd: *const c_char,
    flags: c_uint,
    stdio_count: c_int,
    stdio: *mut UvStdioContainer,
    uid: uv_uid_t,
    gid: uv_gid_t,
}

#[repr(C)]
struct LibuvProcTail {
    uvopts: UvProcessOptions,
    uvstdio: [UvStdioContainer; 4],
}

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
    pub term_name: *mut c_char,
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

#[repr(C)]
pub struct RpcState {
    pub closed: bool,
    pub unpacker: *mut Unpacker,
    pub ui: *mut RemoteUI,
    pub next_request_id: u32,
    pub call_stack: KVec<*mut ChannelCallFrame>,
    pub info: Dict,
    pub client_type: ClientType,
}

#[repr(C)]
pub struct ChannelPrefix {
    pub id: u64,
    pub refcount: usize,
    pub events: *mut MultiQueue,
    pub streamtype: ChannelStreamType,
}

#[repr(C)]
pub struct ChannelTail {
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

type RequestHandlerFn = unsafe extern "C" fn(
    channel_id: u64,
    args: Array,
    arena: *mut Arena,
    error: *mut Error,
) -> Object;
type UIEventHandlerFn = unsafe extern "C" fn(args: Array);

#[cfg(not(test))]
use crate::api::private::dispatch::{
    handle_nvim_get_mode, handle_nvim_paste, handle_nvim_ui_try_resize,
};

#[inline]
pub unsafe fn channel_stream_union_size() -> usize {
    unsafe {
        #[cfg(not(test))]
        let proc_uv_sz = uv_handle_size(UV_PROCESS);
        #[cfg(test)]
        let proc_uv_sz = 136;
        proc_size() + proc_uv_sz + size_of::<LibuvProcTail>()
    }
}

#[inline]
pub unsafe fn channel_prefix(chan: *mut Channel) -> *mut ChannelPrefix {
    chan.cast::<ChannelPrefix>()
}

#[inline]
pub unsafe fn channel_stream_ptr(chan: *mut Channel) -> *mut u8 {
    unsafe { chan.cast::<u8>().add(size_of::<ChannelPrefix>()) }
}

#[inline]
pub unsafe fn channel_tail(chan: *mut Channel) -> *mut ChannelTail {
    unsafe {
        chan.cast::<u8>()
            .add(size_of::<ChannelPrefix>() + channel_stream_union_size())
            .cast::<ChannelTail>()
    }
}

#[inline]
pub unsafe fn channel_size() -> usize {
    unsafe { size_of::<ChannelPrefix>() + channel_stream_union_size() + size_of::<ChannelTail>() }
}

#[inline]
pub unsafe fn channel_instream(chan: *mut Channel) -> *mut Stream {
    unsafe {
        match (*channel_prefix(chan)).streamtype {
            kChannelStreamProc => proc_in(channel_stream_ptr(chan).cast()),
            kChannelStreamSocket => channel_stream_ptr(chan).cast::<Stream>(),
            kChannelStreamStdio => channel_stream_ptr(chan)
                .add(rstream_size())
                .cast::<Stream>(),
            _ => std::process::abort(),
        }
    }
}

#[inline]
pub unsafe fn channel_outstream(chan: *mut Channel) -> *mut RStream {
    unsafe {
        match (*channel_prefix(chan)).streamtype {
            kChannelStreamProc => proc_out(channel_stream_ptr(chan).cast()),
            kChannelStreamSocket | kChannelStreamStdio => {
                channel_stream_ptr(chan).cast::<RStream>()
            }
            _ => std::process::abort(),
        }
    }
}

#[inline]
pub unsafe fn find_channel(id: u64) -> *mut Channel {
    #[cfg(not(test))]
    unsafe {
        let k = mh_get_uint64_t(&raw mut channels.set, id);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*channels.values.add(k as usize)).cast::<Channel>()
        }
    }
    #[cfg(test)]
    {
        let _ = id;
        ptr::null_mut()
    }
}

#[inline]
fn unpacker_closed(p: *const Unpacker) -> bool {
    unsafe { (*p).state < 0 }
}

#[inline]
fn put_event_1(
    queue: *mut MultiQueue,
    handler: unsafe extern "C" fn(*mut *mut c_void),
    a0: *mut c_void,
) {
    let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
    argv[0] = a0;
    unsafe {
        multiqueue_put_event(
            queue,
            Event {
                handler: Some(handler),
                argv,
            },
        );
    }
}

#[inline]
fn create_event_2(
    queue: *mut MultiQueue,
    handler: unsafe extern "C" fn(*mut *mut c_void),
    a0: *mut c_void,
    a1: *mut c_void,
) {
    let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
    argv[0] = a0;
    argv[1] = a1;
    unsafe {
        if !queue.is_null() {
            multiqueue_put_event(
                queue,
                Event {
                    handler: Some(handler),
                    argv,
                },
            );
        } else {
            handler(argv.as_mut_ptr());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_init() {
    #[cfg(not(test))]
    unsafe {
        let qc = loop_queues((&raw mut main_loop).cast());
        ch_before_blocking_events = multiqueue_new_child((*qc).events);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_start(channel: *mut Channel) {
    unsafe {
        #[cfg(not(test))]
        channel_incref(channel);

        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);
        (*ct).is_rpc = true;
        let rpc = &raw mut (*ct).rpc;
        (*rpc).closed = false;
        (*rpc).unpacker = xcalloc(1, size_of::<Unpacker>()).cast::<Unpacker>();
        unpacker_init((*rpc).unpacker);
        (*rpc).next_request_id = 1;
        (*rpc).info = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        (*rpc).call_stack = KVec::new();

        if (*cp).streamtype != kChannelStreamInternal {
            let out = channel_outstream(channel);
            rstream_start(out, Some(receive_msgpack), channel.cast());
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
        let ct = channel_tail(chan);
        if !(*ct).is_rpc || (*ct).rpc.closed {
            return ptr::null_mut();
        }
        chan
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_send_event(id: u64, name: *const c_char, args: Array) -> bool {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_send_call(
    id: u64,
    method_name: *const c_char,
    args: Array,
    result_mem: *mut ArenaMem,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut channel = find_rpc_channel(id);
        if channel.is_null() {
            #[cfg(not(test))]
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Invalid channel: %llu".as_ptr(),
                id,
            );
            return NIL;
        }

        #[cfg(not(test))]
        channel_incref(channel);

        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);
        let rpc = &raw mut (*ct).rpc;
        let request_id = (*rpc).next_request_id;
        (*rpc).next_request_id += 1;

        serialize_request(&raw mut channel, 1, request_id, method_name, args);

        let mut frame = ChannelCallFrame {
            request_id,
            returned: false,
            errored: false,
            result: NIL,
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
                #[cfg(not(test))]
                loop_poll_events((&raw mut main_loop).cast(), -1);
                #[cfg(test)]
                break;
            }
        }
        (*rpc).call_stack.size -= 1;

        if !(*frame_ptr).returned {
            #[cfg(not(test))]
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Invalid channel: %llu".as_ptr(),
                    id,
                );
                channel_decref(channel);
            }
            return NIL;
        }

        if (*frame_ptr).errored {
            let res = (*frame_ptr).result;
            #[cfg(not(test))]
            if res.obj_type == ObjectType::kObjectTypeString {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    res.data.string.data,
                );
            } else if res.obj_type == ObjectType::kObjectTypeArray {
                let array = res.data.array;
                if array.size == 2
                    && (*array.items).obj_type == ObjectType::kObjectTypeInteger
                    && ((*array.items).data.integer == (ErrorType::kErrorTypeException as i64)
                        || (*array.items).data.integer == (ErrorType::kErrorTypeValidation as i64))
                    && (*array.items.add(1)).obj_type == ObjectType::kObjectTypeString
                {
                    let et: ErrorType = std::mem::transmute((*array.items).data.integer as c_int);
                    api_set_error!(
                        err,
                        et,
                        c"%s".as_ptr(),
                        (*array.items.add(1)).data.string.data,
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

        #[cfg(not(test))]
        channel_decref(channel);

        *result_mem = (*frame_ptr).result_mem;

        if (*frame_ptr).errored {
            NIL
        } else {
            (*frame_ptr).result
        }
    }
}

unsafe extern "C" fn receive_msgpack(
    _stream: *mut RStream,
    rbuf: *const c_char,
    c: usize,
    data: *mut c_void,
    eof: bool,
) -> usize {
    unsafe {
        let channel = data.cast::<Channel>();
        #[cfg(not(test))]
        channel_incref(channel);

        let mut consumed: usize = 0;
        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);

        if c > 0 {
            let p = (*ct).rpc.unpacker;
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

        #[cfg(not(test))]
        channel_decref(channel);

        consumed
    }
}

unsafe fn find_call_frame(rpc: *mut RpcState, request_id: u32) -> *mut ChannelCallFrame {
    unsafe {
        for i in 0..(*rpc).call_stack.size {
            let frame = *(*rpc).call_stack.items.add((*rpc).call_stack.size - 1 - i);
            if (*frame).request_id == request_id {
                return frame;
            }
        }
        ptr::null_mut()
    }
}

unsafe fn parse_msgpack(channel: *mut Channel) {
    unsafe {
        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);
        let p = (*ct).rpc.unpacker;

        while unpacker_advance(p) {
            if (*p).msg_type == MessageType::kMessageTypeRedrawEvent {
                #[cfg(not(test))]
                if ui_client_attached {
                    if (*p).has_grid_line_event {
                        ui_client_event_raw_line(&raw mut (*p).grid_line_event);
                        (*p).has_grid_line_event = false;
                    } else if let Some(ui_fn) = (*p).ui_handler.r#fn
                        && (*p).result.obj_type == ObjectType::kObjectTypeArray
                    {
                        ui_fn((*p).result.data.array);
                    }
                }
                arena_mem_free(arena_finish(&raw mut (*p).arena));
            } else if (*p).msg_type == MessageType::kMessageTypeResponse {
                let frame = if (*ct).rpc.client_type == kClientTypeMsgpackRpc {
                    find_call_frame(&raw mut (*ct).rpc, (*p).request_id)
                } else if (*ct).rpc.call_stack.size > 0 {
                    *(*ct)
                        .rpc
                        .call_stack
                        .items
                        .add((*ct).rpc.call_stack.size - 1)
                } else {
                    ptr::null_mut()
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
                handle_request(channel, p, res.data.array);
            }
        }

        if unpacker_closed(p) {
            chan_close_on_err(channel, (*p).unpack_error.msg, LOGLVL_INF);
            #[cfg(not(test))]
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
                (*p).unpack_error.msg,
            );
            #[cfg(not(test))]
            api_clear_error(&raw mut (*p).unpack_error);
            arena_mem_free(arena_finish(&raw mut (*p).arena));
            return;
        }

        let cp = channel_prefix(channel);
        let evdata = xmalloc(size_of::<RequestEvent>()).cast::<RequestEvent>();
        (*evdata).r#type = (*p).msg_type;
        (*evdata).channel = channel;
        (*evdata).handler = (*p).handler;
        (*evdata).args = args;
        (*evdata).used_mem = (*p).arena;
        (*p).arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        (*evdata).request_id = (*p).request_id;
        #[cfg(not(test))]
        channel_incref(channel);

        if (*p).handler.fast {
            #[cfg(not(test))]
            let is_get_mode = (*p).handler.r#fn.map(|f| f as *const c_void)
                == Some(handle_nvim_get_mode as *const c_void);
            #[cfg(test)]
            let is_get_mode = false;

            if is_get_mode && !input_blocking() {
                #[cfg(not(test))]
                put_event_1(ch_before_blocking_events, request_event, evdata.cast());
            } else {
                let mut argv_ptr = evdata.cast::<c_void>();
                request_event(&raw mut argv_ptr);
            }
        } else {
            #[cfg(not(test))]
            let is_resize = (*p).handler.r#fn.map(|f| f as *const c_void)
                == Some(handle_nvim_ui_try_resize as *const c_void);
            #[cfg(test)]
            let is_resize = false;

            if is_resize {
                let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
                argv[0] = evdata.cast();
                let ev = event_create_oneshot(
                    Event {
                        handler: Some(request_event),
                        argv,
                    },
                    2,
                );
                multiqueue_put_event((*cp).events, ev);
                #[cfg(not(test))]
                multiqueue_put_event(resize_events, ev);
            } else {
                put_event_1((*cp).events, request_event, evdata.cast());
            }
        }
    }
}

unsafe extern "C" fn request_event(argv: *mut *mut c_void) {
    unsafe {
        let e = (*argv).cast::<RequestEvent>();
        let channel = (*e).channel;
        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);
        let handler = (*e).handler;
        let mut error = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };

        if !(*ct).rpc.closed {
            let fn_call = handler.r#fn.unwrap();
            let mut result = fn_call((*cp).id, (*e).args, &raw mut (*e).used_mem, &raw mut error);
            if (*e).r#type == MessageType::kMessageTypeRequest
                || error.err_type != ErrorType::kErrorTypeNone
            {
                serialize_response(
                    channel,
                    (*e).handler,
                    (*e).r#type,
                    (*e).request_id,
                    &raw mut error,
                    &raw mut result,
                );
            }
            if handler.ret_alloc {
                #[cfg(not(test))]
                api_free_object(result);
            }
        }

        arena_mem_free(arena_finish(&raw mut (*e).used_mem));
        #[cfg(not(test))]
        channel_decref(channel);
        xfree(e.cast());
        #[cfg(not(test))]
        api_clear_error(&raw mut error);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_write_raw(id: u64, buffer: *mut WBuffer) -> bool {
    unsafe {
        let channel = find_rpc_channel(id);
        if channel.is_null() {
            wstream_release_wbuffer(buffer);
            return false;
        }

        channel_write(channel, buffer)
    }
}

unsafe fn channel_write(channel: *mut Channel, buffer: *mut WBuffer) -> bool {
    unsafe {
        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);
        let mut err: c_int = 0;

        if (*ct).rpc.closed {
            wstream_release_wbuffer(buffer);
            return false;
        }

        if (*cp).streamtype == kChannelStreamInternal {
            #[cfg(not(test))]
            channel_incref(channel);
            create_event_2(
                (*cp).events,
                internal_read_event,
                channel.cast(),
                buffer.cast(),
            );
        } else {
            let in_stream = channel_instream(channel);
            err = wstream_write(in_stream, buffer);
        }

        if err != 0 {
            #[cfg(not(test))]
            let err_str = CStr::from_ptr(uv_strerror(err)).to_string_lossy();
            #[cfg(test)]
            let err_str = "error";
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

unsafe extern "C" fn internal_read_event(argv: *mut *mut c_void) {
    unsafe {
        let channel = (*argv).cast::<Channel>();
        let buffer = (*argv.add(1)).cast::<WBuffer>();
        let ct = channel_tail(channel);
        let p = (*ct).rpc.unpacker;

        (*p).read_ptr = (*buffer).data;
        (*p).read_size = (*buffer).size;
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

        #[cfg(not(test))]
        channel_decref(channel);
        wstream_release_wbuffer(buffer);
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
        let mut e = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        #[cfg(not(test))]
        api_set_error!(
            &raw mut e,
            ErrorType::kErrorTypeException,
            c"%s".as_ptr(),
            err,
        );
        #[cfg(test)]
        let _ = err;
        let mut nil_obj = NIL;
        serialize_response(chan, handler, r#type, id, &raw mut e, &raw mut nil_obj);
        #[cfg(not(test))]
        api_clear_error(&raw mut e);
    }
}

unsafe fn broadcast_event(name: *const c_char, args: Array) {
    unsafe {
        let mut chans = KVec::<*mut Channel>::new();

        #[cfg(not(test))]
        for i in 0..(channels.set.h.n_keys as usize) {
            let channel = (*channels.values.add(i)).cast::<Channel>();
            if (*channel_tail(channel)).is_rpc {
                chans.push(channel);
            }
        }

        if chans.size > 0 {
            serialize_request(chans.items, chans.size, 0, name, args);
        }

        chans.destroy();
    }
}

unsafe fn has_undetached_rpc_chan() -> bool {
    #[cfg(not(test))]
    unsafe {
        for i in 0..(channels.set.h.n_keys as usize) {
            let chan = (*channels.values.add(i)).cast::<Channel>();
            let ct = channel_tail(chan);
            if (*ct).is_rpc && !(*ct).rpc.closed && !(*ct).detach {
                return true;
            }
        }
    }
    false
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_close(channel: *mut Channel) {
    unsafe {
        let ct = channel_tail(channel);
        if (*ct).rpc.closed {
            return;
        }

        (*ct).rpc.closed = true;

        #[cfg(not(test))]
        {
            let qc = loop_queues((&raw mut main_loop).cast());
            put_event_1((*qc).fast_events, rpc_close_event, channel.cast());
        }
    }
}

unsafe extern "C" fn rpc_close_event(argv: *mut *mut c_void) {
    unsafe {
        let channel = (*argv).cast::<Channel>();
        assert!(!channel.is_null());

        #[cfg(not(test))]
        channel_decref(channel);

        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);

        #[cfg(all(not(test), windows))]
        let ui_closed = !(*ct).rpc.ui.is_null();

        #[cfg(not(test))]
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
        #[cfg(test)]
        let _ = (cp, ct);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_free(channel: *mut Channel) {
    unsafe {
        let ct = channel_tail(channel);
        unpacker_teardown((*ct).rpc.unpacker);
        xfree((*ct).rpc.unpacker.cast());

        (*ct).rpc.call_stack.destroy();
        #[cfg(not(test))]
        api_free_dict((*ct).rpc.info);
    }
}

unsafe fn chan_close_on_err(channel: *mut Channel, msg: *const c_char, loglevel: c_int) {
    unsafe {
        #[cfg(not(test))]
        logmsg!(
            loglevel,
            c"RPC: ".as_ptr(),
            ptr::null(),
            -1,
            true,
            c"%s".as_ptr(),
            msg,
        );

        let cp = channel_prefix(channel);
        let ct = channel_tail(channel);
        let p = (*ct).rpc.unpacker;

        for i in 0..(*ct).rpc.call_stack.size {
            let frame = *(*ct).rpc.call_stack.items.add(i);
            if (*frame).returned {
                continue;
            }
            (*frame).returned = true;
            (*frame).errored = true;
            let s = arena_string(&raw mut (*p).arena, cstr_as_string(msg));
            (*frame).result = Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData { string: s },
            };
            (*frame).result_mem = arena_finish(&raw mut (*p).arena);
        }

        #[cfg(not(test))]
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
        #[cfg(test)]
        let _ = (cp, loglevel);
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
        let mut packer = PackerBuffer {
            startptr: ptr::null_mut(),
            ptr: ptr::null_mut(),
            endptr: ptr::null_mut(),
            anydata: ptr::null_mut(),
            anyint: 0,
            packer_flush: None,
        };
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn serialize_response(
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
            #[cfg(not(test))]
            let is_paste = handler.r#fn.map(|f| f as *const c_void)
                == Some(handle_nvim_paste as *const c_void);
            #[cfg(test)]
            let is_paste = false;

            if is_paste {
                #[cfg(not(test))]
                {
                    semsg!(c"paste: %s".as_ptr(), (*err).msg);
                    api_clear_error(err);
                }
            } else {
                let mut items = [
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: (*err).err_type as i64,
                        },
                    },
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: cstr_as_string((*err).msg),
                        },
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

        let mut packer = PackerBuffer {
            startptr: ptr::null_mut(),
            ptr: ptr::null_mut(),
            endptr: ptr::null_mut(),
            anydata: ptr::null_mut(),
            anyint: 0,
            packer_flush: None,
        };
        packer_buffer_init_channels(&raw mut channel, 1, &raw mut packer);

        mpack_array(&raw mut packer.ptr, 4);
        mpack_w(&raw mut packer.ptr, 1);
        mpack_uint(&raw mut packer.ptr, response_id);

        if (*err).err_type != ErrorType::kErrorTypeNone {
            mpack_array(&raw mut packer.ptr, 2);
            mpack_integer(&raw mut packer.ptr, (*err).err_type as Integer);
            mpack_str(cstr_as_string((*err).msg), &raw mut packer);
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
            let ct = channel_tail(chan);
            if !(*ct).rpc.ui.is_null() && (*(*ct).rpc.ui).incomplete_event {
                #[cfg(not(test))]
                remote_ui_flush_pending_data((*ct).rpc.ui);
            }
        }
        (*packer).startptr = alloc_block();
        (*packer).ptr = (*packer).startptr;
        (*packer).endptr = (*packer).startptr.add(ARENA_BLOCK_SIZE);
        (*packer).packer_flush = Some(channel_flush_callback);
        (*packer).anydata = chans.cast();
        (*packer).anyint = nchans as i64;
    }
}

unsafe fn packer_buffer_finish_channels(packer: *mut PackerBuffer) {
    unsafe {
        let len = ((*packer).ptr as usize) - ((*packer).startptr as usize);
        if len > 0 {
            let buf = wstream_new_buffer(
                (*packer).startptr,
                len,
                (*packer).anyint as usize,
                Some(free_block_void),
            );
            let chans = (*packer).anydata.cast::<*mut Channel>();
            for i in 0..((*packer).anyint as usize) {
                let _ = channel_write(*chans.add(i), buf);
            }
        } else {
            free_block((*packer).startptr);
        }
    }
}

unsafe extern "C" fn channel_flush_callback(packer: *mut PackerBuffer) {
    unsafe {
        packer_buffer_finish_channels(packer);
        packer_buffer_init_channels(
            (*packer).anydata.cast::<*mut Channel>(),
            (*packer).anyint as usize,
            packer,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_set_client_info(id: u64, info: Dict) {
    unsafe {
        let chan = find_rpc_channel(id);
        if chan.is_null() {
            std::process::abort();
        }

        let ct = channel_tail(chan);
        #[cfg(not(test))]
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

        #[cfg(not(test))]
        channel_event(chan, EVENT_CHANINFO);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_client_info(chan: *mut Channel) -> Dict {
    unsafe {
        #[cfg(not(test))]
        {
            copy_dict((*channel_tail(chan)).rpc.info, ptr::null_mut())
        }
        #[cfg(test)]
        {
            (*channel_tail(chan)).rpc.info
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_client_info(chan: *mut Channel, key: *const c_char) -> *const c_char {
    unsafe {
        let ct = channel_tail(chan);
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
                return item.value.data.string.data;
            }
        }

        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rpc_free_all_mem() {
    #[cfg(not(test))]
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
                unpacker: ptr::null_mut(),
                ui: ptr::null_mut(),
                next_request_id: 1,
                call_stack: KVec::new(),
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
                result: NIL,
                result_mem: ptr::null_mut(),
            };
            let mut f2 = ChannelCallFrame {
                request_id: 20,
                returned: false,
                errored: false,
                result: NIL,
                result_mem: ptr::null_mut(),
            };
            rpc.call_stack.push(&raw mut f1);
            rpc.call_stack.push(&raw mut f2);

            assert_eq!(find_call_frame(&raw mut rpc, 20), &raw mut f2);
            assert_eq!(find_call_frame(&raw mut rpc, 10), &raw mut f1);
            assert!(find_call_frame(&raw mut rpc, 99).is_null());

            rpc.call_stack.destroy();
        }
    }
}
