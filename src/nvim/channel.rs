//! Async I/O and process/socket/stdio/terminal channel management (`src/nvim/channel.c`).

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{arena_array, arena_dict, cstr_as_string, put_c};
use crate::api::private::validate::{Array, Dict, NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{apply_autocmds, event_T, has_event};
use crate::buffer::buf_T;
use crate::errors::*;
use crate::eval::encode::encode_list_write;
use crate::eval::typval::{
    Callback, TypvalVval, VAR_UNLOCKED, VarType, callback_free, dict_T, kCallbackNone, list_T,
    tv_clear, tv_dict_add_dict, tv_dict_add_list, tv_dict_find, tv_dict_free,
    tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_string, tv_list_ref, tv_list_unref,
    typval_T, varnumber_T,
};
use crate::eval::{callback_call, get_v_event, kListLenMayKnow, restore_v_event};
use crate::event::r#loop::{loop_queues, uv_strerror};
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_free, multiqueue_new_child,
    multiqueue_put_event,
};
use crate::event::proc::{
    Proc, exit_on_closed_chan, proc_err, proc_free, proc_header, proc_in, proc_out, proc_size,
    proc_spawn, proc_stop, proc_tail,
};
use crate::event::rstream::{
    RStream, rstream_init, rstream_init_fd, rstream_may_close, rstream_size, rstream_start,
    rstream_start_inner, rstream_stop_inner,
};
use crate::event::socket::{SocketWatcher, socket_connect, socket_watcher_accept};
use crate::event::stream::{Stream, stream_closed_ptr, stream_fields, stream_may_close};
use crate::event::wstream::{wstream_init, wstream_init_fd, wstream_new_buffer, wstream_write};
#[cfg(not(windows))]
use crate::fileio::fcntl;
use crate::garray::{ga_clear, ga_concat_len, ga_init};
use crate::globals::*;
use crate::lua::converter::LuaRef;
use crate::lua::executor::api_free_luaref;
use crate::map::{Map, MapHash, Set, map_del_uint64_t_ptr_t, map_put_ref_uint64_t_ptr_t};
use crate::memory::{
    Arena, arena_alloc, arena_finish, arena_mem_free, arena_memdupz, xcalloc, xfree, xmemdup,
    xstrdup,
};
use crate::message::semsg;
use crate::msgpack_rpc::channel::{
    ChannelPart, ChannelStreamType, channel_instream, channel_prefix, channel_size,
    channel_stream_ptr, channel_tail, find_channel, rpc_close, rpc_free, rpc_init, rpc_start,
};
use crate::msgpack_rpc::packer::Integer;
use crate::msgpack_rpc::server::server_owns_pipe_address;
#[cfg(windows)]
use crate::os::fs::os_dup_cloexec;
use crate::os::fs::os_write;
#[cfg(windows)]
use crate::os::os_win_console::os_attach_parent_console;
#[cfg(windows)]
use crate::os::pty_conpty_win::os_has_conpty_working;
use crate::os::pty_proc::{
    pty_proc_close_master, pty_proc_resize, pty_proc_resume, pty_proc_tty_name,
};
use crate::os::shell::shell_free_argv;
use crate::state::save_v_event_T;
use crate::strings::gettext;
use crate::terminal::{
    terminal_alloc, terminal_buf, terminal_close, terminal_destroy, terminal_receive,
    terminal_set_state,
};
use crate::ui_client::ui_client_attach_to_restarted_server;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

pub type ChannelStdinMode = c_int;
pub type Channel = c_void;

#[repr(C)]
pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: *mut dict_T,
    pub buffer: crate::garray::garray_T,
    pub eof: bool,
    pub buffered: bool,
    pub fwd_err: bool,
    pub reader_type: *const c_char,
}

use crate::option::OptInt;

pub(crate) const CHAN_STDERR: u64 = 2;

pub(crate) const STDIN_FILENO: c_int = 0;
pub(crate) const STDOUT_FILENO: c_int = 1;
pub(crate) const STDERR_FILENO: c_int = 2;

pub(crate) const kProcTypeUv: c_int = 0;
pub(crate) const kProcTypePty: c_int = 1;

pub(crate) const kChannelStreamProc: ChannelStreamType = 0;
pub(crate) const kChannelStreamSocket: ChannelStreamType = 1;
pub(crate) const kChannelStreamStdio: ChannelStreamType = 2;
const kChannelStreamStderr: ChannelStreamType = 3;

pub(crate) const kChannelPartStdin: ChannelPart = 0;
pub(crate) const kChannelPartStdout: ChannelPart = 1;
pub(crate) const kChannelPartStderr: ChannelPart = 2;
pub(crate) const kChannelPartRpc: ChannelPart = 3;

pub(crate) const kChannelStdinPipe: ChannelStdinMode = 0;

#[cfg(target_os = "linux")]
const F_DUPFD_CLOEXEC: c_int = 1030;
#[cfg(target_os = "macos")]
const F_DUPFD_CLOEXEC: c_int = 67;
#[cfg(target_os = "freebsd")]
const F_DUPFD_CLOEXEC: c_int = 17;
#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))
))]
const F_DUPFD_CLOEXEC: c_int = 12;

static mut DID_STDIO: bool = false;
static mut NEXT_CHAN_ID: u64 = CHAN_STDERR + 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InternalState {
    pub cb: LuaRef,
    pub closed: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TerminalOptions {
    pub data: *mut c_void,
    pub width: u16,
    pub height: u16,
    pub read_pause_cb: Option<unsafe extern "C" fn(pause: bool, data: *mut c_void)>,
    pub write_cb:
        Option<unsafe extern "C" fn(buffer: *const c_char, size: usize, data: *mut c_void)>,
    pub resize_cb: Option<unsafe extern "C" fn(width: u16, height: u16, data: *mut c_void)>,
    pub resume_cb: Option<unsafe extern "C" fn(data: *mut c_void)>,
    pub close_cb: Option<unsafe extern "C" fn(data: *mut c_void)>,
    pub force_crlf: bool,
}

use crate::api::ui::{CHAN_STDIO, kChannelPartAll};
use crate::api::vim::kChannelStreamInternal;
use crate::autocmd::{EVENT_CHANCLOSE, EVENT_CHANOPEN};
#[cfg(not(windows))]
use crate::fileio::dup2;
use crate::lua::converter::LUA_NOREF;

use crate::fileio::freopen;

#[inline]
unsafe fn redirect_stderr_to_null() {
    #[cfg(windows)]
    unsafe {
        let _ = freopen(
            c"NUL:".as_ptr(),
            c"w".as_ptr(),
            crate::log::log_get_stderr().cast(),
        );
    }
    #[cfg(not(windows))]
    unsafe {
        let _ = freopen(
            c"/dev/null".as_ptr(),
            c"w".as_ptr(),
            crate::log::log_get_stderr().cast(),
        );
    }
}

#[inline]
fn callback_reader_set(reader: &CallbackReader) -> bool {
    reader.cb.cb_type != kCallbackNone || !reader.self_dict.is_null()
}

#[inline]
unsafe fn pty_width_ptr(chan: *mut Channel) -> *mut u16 {
    unsafe { channel_stream_ptr(chan).add(proc_size()).cast::<u16>() }
}

#[inline]
unsafe fn pty_height_ptr(chan: *mut Channel) -> *mut u16 {
    unsafe { channel_stream_ptr(chan).add(proc_size() + 2).cast::<u16>() }
}

#[inline]
pub unsafe fn put_event_1(
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
fn put_event_2(
    queue: *mut MultiQueue,
    handler: unsafe extern "C" fn(*mut *mut c_void),
    a0: *mut c_void,
    a1: *mut c_void,
) {
    let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
    argv[0] = a0;
    argv[1] = a1;
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

use crate::api::private::helpers::add_c;

/// Teardown the module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_teardown() {
    unsafe {
        for i in 0..(channels.set.h.n_keys as usize) {
            let chan = (*channels.values.add(i)).cast::<Channel>();
            channel_close((*channel_prefix(chan)).id, kChannelPartAll, ptr::null_mut());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_free_all_mem() {
    unsafe {
        for i in 0..(channels.set.h.n_keys as usize) {
            let chan = (*channels.values.add(i)).cast::<Channel>();
            channel_destroy(chan);
        }
        xfree(channels.set.keys.cast());
        xfree(channels.set.h.hash.cast());
        xfree(channels.values.cast());
        channels = Map {
            set: Set {
                h: MapHash {
                    n_buckets: 0,
                    size: 0,
                    n_occupied: 0,
                    upper_bound: 0,
                    n_keys: 0,
                    keys_capacity: 0,
                    hash: ptr::null_mut(),
                },
                keys: ptr::null_mut(),
            },
            values: ptr::null_mut(),
        };

        callback_free(&raw mut on_print);
    }
}

/// Closes a channel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_close(
    id: u64,
    part: ChannelPart,
    mut error: *mut *const c_char,
) -> bool {
    unsafe {
        let mut dummy: *const c_char = ptr::null();
        if error.is_null() {
            error = &raw mut dummy;
        }

        let chan = find_channel(id);
        if chan.is_null() {
            if id < NEXT_CHAN_ID {
                return true;
            }
            {
                *error = e_invchan.as_ptr();
            }
            return false;
        }

        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        let mut close_main = false;
        if part == kChannelPartRpc || part == kChannelPartAll {
            close_main = true;
            if (*ct).is_rpc {
                rpc_close(chan);
            } else if part == kChannelPartRpc {
                {
                    *error = e_invstream.as_ptr();
                }
                return false;
            }
        } else if (part == kChannelPartStdin || part == kChannelPartStdout) && (*ct).is_rpc {
            {
                *error = e_invstreamrpc.as_ptr();
            }
            return false;
        }

        match (*cp).streamtype {
            kChannelStreamSocket => {
                if !close_main {
                    {
                        *error = e_invstream.as_ptr();
                    }
                    return false;
                }
                rstream_may_close(channel_stream_ptr(chan).cast());
            }
            kChannelStreamProc => {
                let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
                if part == kChannelPartStdin || close_main {
                    stream_may_close(proc_in(proc_ptr));
                }
                if part == kChannelPartStdout || close_main {
                    rstream_may_close(proc_out(proc_ptr));
                }
                if part == kChannelPartStderr || part == kChannelPartAll {
                    rstream_may_close(proc_err(proc_ptr));
                }
                if (*proc_header(proc_ptr)).r#type == kProcTypePty && part == kChannelPartAll {
                    pty_proc_close_master(proc_ptr.cast());
                }
            }
            kChannelStreamStdio => {
                if part == kChannelPartStdin || close_main {
                    rstream_may_close(channel_stream_ptr(chan).cast());
                }
                if part == kChannelPartStdout || close_main {
                    stream_may_close(channel_stream_ptr(chan).add(rstream_size()).cast());
                }
                if part == kChannelPartStderr {
                    {
                        *error = e_invstream.as_ptr();
                    }
                    return false;
                }
            }
            kChannelStreamStderr => {
                if part != kChannelPartAll && part != kChannelPartStderr {
                    {
                        *error = e_invstream.as_ptr();
                    }
                    return false;
                }
                let err_closed = channel_stream_ptr(chan).cast::<bool>();
                if !*err_closed {
                    *err_closed = true;
                    if !exiting {
                        redirect_stderr_to_null();
                    }
                    channel_decref(chan);
                }
            }
            kChannelStreamInternal => {
                if !close_main {
                    {
                        *error = e_invstream.as_ptr();
                    }
                    return false;
                }
                if !(*ct).term.is_null() {
                    let internal = channel_stream_ptr(chan).cast::<InternalState>();
                    api_free_luaref((*internal).cb);
                    (*internal).cb = LUA_NOREF;
                    (*internal).closed = true;
                    terminal_close(&raw mut (*ct).term, 0);
                    (*ct).exit_status = 0;
                } else {
                    channel_decref(chan);
                }
            }
            _ => {}
        }

        true
    }
}

/// Initializes the module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_init() {
    unsafe {
        let _ = channel_alloc(kChannelStreamStderr);
        rpc_init();
    }
}

/// Allocates a channel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_alloc(r#type: ChannelStreamType) -> *mut Channel {
    unsafe {
        let chan = xcalloc(1, channel_size()).cast::<Channel>();
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        if r#type == kChannelStreamStdio {
            (*cp).id = CHAN_STDIO;
        } else if r#type == kChannelStreamStderr {
            (*cp).id = CHAN_STDERR;
        } else {
            (*cp).id = NEXT_CHAN_ID;
            NEXT_CHAN_ID += 1;
        }

        {
            let qc = loop_queues((&raw mut main_loop).cast());
            (*cp).events = multiqueue_new_child((*qc).events);
        }
        (*cp).refcount = 1;
        (*ct).exit_status = -1;
        (*cp).streamtype = r#type;
        (*ct).detach = r#type != kChannelStreamStdio;
        assert!((*cp).id <= (i64::MAX as u64));

        {
            let slot = map_put_ref_uint64_t_ptr_t(
                &raw mut channels,
                (*cp).id,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            *slot = chan.cast();
        }

        chan
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_create_event(chan: *mut Channel, _ext_source: *const c_char) {
    unsafe {
        channel_event(chan, EVENT_CHANOPEN);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_incref(chan: *mut Channel) {
    unsafe {
        (*channel_prefix(chan)).refcount += 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_decref(chan: *mut Channel) {
    unsafe {
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);
        if (*cp).refcount == 1 && !(*ct).did_close_event {
            (*ct).did_close_event = true;
            channel_event(chan, EVENT_CHANCLOSE);
        }

        (*cp).refcount -= 1;
        if (*cp).refcount == 0 {
            {
                let qc = loop_queues((&raw mut main_loop).cast());
                put_event_1((*qc).events, free_channel_event, chan.cast());
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn callback_reader_free(reader: *mut CallbackReader) {
    unsafe {
        callback_free(&raw mut (*reader).cb);
        ga_clear(&raw mut (*reader).buffer);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn callback_reader_start(reader: *mut CallbackReader, r#type: *const c_char) {
    unsafe {
        ga_init(
            &raw mut (*reader).buffer,
            size_of::<*mut c_char>() as c_int,
            32,
        );
        (*reader).reader_type = r#type;
    }
}

unsafe fn channel_destroy(chan: *mut Channel) {
    unsafe {
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        if (*ct).is_rpc {
            rpc_free(chan);
        }

        if (*cp).streamtype == kChannelStreamProc {
            proc_free(channel_stream_ptr(chan).cast());
        }

        callback_reader_free(&raw mut (*ct).on_data);
        callback_reader_free(&raw mut (*ct).on_stderr);
        callback_free(&raw mut (*ct).on_exit);

        multiqueue_free((*cp).events);
        xfree(chan.cast());
    }
}

unsafe extern "C" fn free_channel_event(argv: *mut *mut c_void) {
    unsafe {
        let chan = (*argv).cast::<Channel>();
        let _ = map_del_uint64_t_ptr_t(
            &raw mut channels,
            (*channel_prefix(chan)).id,
            ptr::null_mut(),
        );
        channel_destroy(chan);
    }
}

unsafe fn channel_destroy_early(chan: *mut Channel) {
    unsafe {
        let cp = channel_prefix(chan);
        NEXT_CHAN_ID -= 1;
        if (*cp).id != NEXT_CHAN_ID {
            std::process::abort();
        }
        let _ = map_del_uint64_t_ptr_t(&raw mut channels, (*cp).id, ptr::null_mut());
        (*cp).id = 0;

        (*cp).refcount -= 1;
        if (*cp).refcount != 0 {
            std::process::abort();
        }

        {
            let qc = loop_queues((&raw mut main_loop).cast());
            put_event_1((*qc).events, free_channel_event, chan.cast());
        }
    }
}

unsafe extern "C" fn close_cb(_stream: *mut Stream, data: *mut c_void) {
    unsafe {
        channel_decref(data.cast());
    }
}

/// Starts a job and returns the associated channel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_job_start(
    argv: *mut *mut c_char,
    exepath: *const c_char,
    on_stdout: CallbackReader,
    on_stderr: CallbackReader,
    on_exit: Callback,
    pty: bool,
    rpc: bool,
    overlapped: bool,
    detach: bool,
    stdin_mode: ChannelStdinMode,
    cwd: *const c_char,
    pty_width: u16,
    pty_height: u16,
    env: *mut dict_T,
    status_out: *mut varnumber_T,
) -> *mut Channel {
    unsafe {
        let chan = channel_alloc(kChannelStreamProc);
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);
        (*ct).on_data = on_stdout;
        (*ct).on_stderr = on_stderr;
        (*ct).on_exit = on_exit;

        let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
        if pty {
            if detach {
                {
                    semsg!(
                        gettext(e_invarg2.as_ptr()),
                        c"terminal/pty job cannot be detached".as_ptr(),
                    );
                    if !env.is_null() {
                        tv_dict_free(env);
                    }
                }
                shell_free_argv(argv);
                channel_destroy_early(chan);
                *status_out = 0;
                return ptr::null_mut();
            }
            let ph = proc_header(proc_ptr);
            (*ph).r#type = kProcTypePty;
            (*ph).data = chan.cast();
            {
                (*ph).loop_ = (&raw mut main_loop).cast();
            }
            (*ph).status = -1;
            (*stream_fields(proc_out(proc_ptr).cast())).fd = STDOUT_FILENO;
            (*stream_fields(proc_err(proc_ptr).cast())).fd = STDERR_FILENO;
            *pty_width_ptr(chan) = if pty_width > 0 { pty_width } else { 80 };
            *pty_height_ptr(chan) = if pty_height > 0 { pty_height } else { 24 };
            #[cfg(not(windows))]
            {
                *channel_stream_ptr(chan)
                    .add(proc_size() + 12)
                    .cast::<c_int>() = -1;
            }
        } else {
            let ph = proc_header(proc_ptr);
            (*ph).r#type = kProcTypeUv;
            (*ph).data = chan.cast();
            {
                (*ph).loop_ = (&raw mut main_loop).cast();
            }
            (*ph).status = -1;
            (*stream_fields(proc_out(proc_ptr).cast())).fd = STDOUT_FILENO;
            (*stream_fields(proc_err(proc_ptr).cast())).fd = STDERR_FILENO;
        }

        let ph = proc_header(proc_ptr);
        let pt = proc_tail(proc_ptr);
        (*ph).argv = argv;
        (*ph).exepath = exepath;
        (*pt).cb = Some(channel_proc_exit_cb);
        (*pt).state_cb = Some(channel_proc_state_cb);
        (*pt).events = (*cp).events;
        (*pt).detach = detach;
        (*ph).cwd = cwd;
        (*ph).env = env;
        (*pt).overlapped = overlapped;
        #[cfg(windows)]
        {
            (*pt).stdio_noinherit = true;
        }

        let exe_str = if !(*ph).exepath.is_null() {
            (*ph).exepath
        } else {
            *(*ph).argv
        };
        let cmd = xstrdup(exe_str);

        let (has_out, has_err) = if (*ph).r#type == kProcTypePty {
            (true, false)
        } else {
            let out = rpc || callback_reader_set(&(*ct).on_data);
            #[cfg(windows)]
            let mut err = callback_reader_set(&(*ct).on_stderr);
            #[cfg(not(windows))]
            let err = callback_reader_set(&(*ct).on_stderr);
            (*pt).fwd_err = (*ct).on_stderr.fwd_err;
            #[cfg(windows)]
            if !err && (*pt).fwd_err && (*pt).detach {
                err = true;
                (*pt).fwd_err = false;
            }
            (out, err)
        };

        let has_in = stdin_mode == kChannelStdinPipe;

        let status = proc_spawn(proc_ptr, has_in, has_out, has_err);
        if status != 0 {
            {
                semsg!(gettext(e_jobspawn.as_ptr()), uv_strerror(status), cmd);
                if !(*ph).env.is_null() {
                    tv_dict_free((*ph).env);
                }
            }
            xfree(cmd.cast());
            channel_destroy_early(chan);
            *status_out = varnumber_T::from((*ph).status);
            return ptr::null_mut();
        }

        xfree(cmd.cast());
        if !(*ph).env.is_null() {
            tv_dict_free((*ph).env);
        }

        if has_in {
            wstream_init(proc_in(proc_ptr), 0);
        }
        if has_out {
            rstream_init(proc_out(proc_ptr));
        }

        if rpc {
            rpc_start(chan);
        } else if has_out {
            callback_reader_start(&raw mut (*ct).on_data, c"stdout".as_ptr());
            rstream_start(proc_out(proc_ptr), Some(on_channel_data), chan.cast());
        }

        if has_err {
            callback_reader_start(&raw mut (*ct).on_stderr, c"stderr".as_ptr());
            rstream_init(proc_err(proc_ptr));
            rstream_start(proc_err(proc_ptr), Some(on_job_stderr), chan.cast());
        }

        *status_out = (*cp).id as varnumber_T;
        chan
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_connect(
    tcp: bool,
    address: *const c_char,
    rpc: bool,
    on_output: CallbackReader,
    timeout: c_int,
    error: *mut *const c_char,
) -> u64 {
    unsafe {
        if !tcp && rpc && server_owns_pipe_address(address) {
            let channel = channel_alloc(kChannelStreamInternal);
            let internal = channel_stream_ptr(channel).cast::<InternalState>();
            (*internal).cb = LUA_NOREF;
            rpc_start(channel);
            channel_create_event(channel, address);
            return (*channel_prefix(channel)).id;
        }

        let channel = channel_alloc(kChannelStreamSocket);
        let socket_stream = channel_stream_ptr(channel).cast::<RStream>();
        let connected = socket_connect(
            (&raw mut main_loop).cast(),
            socket_stream,
            tcp,
            address,
            timeout,
            error,
        );

        if !connected {
            channel_decref(channel);
            return 0;
        }

        let sf = stream_fields(socket_stream.cast());
        (*sf).internal_close_cb = Some(close_cb);
        (*sf).internal_data = channel.cast();
        wstream_init(socket_stream.cast(), 0);
        rstream_init(socket_stream);

        if rpc {
            rpc_start(channel);
        } else {
            let ct = channel_tail(channel);
            (*ct).on_data = on_output;
            callback_reader_start(&raw mut (*ct).on_data, c"data".as_ptr());
            rstream_start(socket_stream, Some(on_channel_data), channel.cast());
        }

        channel_create_event(channel, address);
        (*channel_prefix(channel)).id
    }
}

/// Creates an RPC channel from a tcp/pipe socket connection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_from_connection(watcher: *mut SocketWatcher) {
    unsafe {
        let channel = channel_alloc(kChannelStreamSocket);
        let socket_stream = channel_stream_ptr(channel).cast::<RStream>();
        let result = socket_watcher_accept(watcher, socket_stream);
        if result != 0 {
            channel_destroy_early(channel);
            return;
        }
        let sf = stream_fields(socket_stream.cast());
        (*sf).internal_close_cb = Some(close_cb);
        (*sf).internal_data = channel.cast();
        wstream_init(socket_stream.cast(), 0);
        rstream_init(socket_stream);
        rpc_start(channel);
        channel_create_event(channel, watcher.cast::<c_char>());
    }
}

/// Creates an API channel from stdin/stdout. Used when embedding Nvim.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_from_stdio(
    rpc: bool,
    on_output: CallbackReader,
    error: *mut *const c_char,
) -> u64 {
    unsafe {
        if !headless_mode && !embedded_mode {
            *error = gettext(c"can only be opened in headless mode".as_ptr());
            return 0;
        }

        if DID_STDIO {
            {
                *error = gettext(c"channel was already open".as_ptr());
            }
            return 0;
        }
        DID_STDIO = true;

        let channel = channel_alloc(kChannelStreamStdio);
        let mut stdin_dup_fd = STDIN_FILENO;
        let mut stdout_dup_fd = STDOUT_FILENO;

        #[cfg(windows)]
        if embedded_mode && os_has_conpty_working() {
            stdin_dup_fd = os_dup_cloexec(STDIN_FILENO);
            stdout_dup_fd = os_dup_cloexec(STDOUT_FILENO);
            os_attach_parent_console();
        }

        #[cfg(not(windows))]
        if embedded_mode {
            stdin_dup_fd = fcntl(STDIN_FILENO, F_DUPFD_CLOEXEC, STDERR_FILENO + 1);
            stdout_dup_fd = fcntl(STDOUT_FILENO, F_DUPFD_CLOEXEC, STDERR_FILENO + 1);
            let _ = dup2(STDERR_FILENO, STDOUT_FILENO);
            let _ = dup2(STDERR_FILENO, STDIN_FILENO);
        }

        let stdio_in = channel_stream_ptr(channel).cast::<RStream>();
        let stdio_out = channel_stream_ptr(channel)
            .add(rstream_size())
            .cast::<Stream>();
        {
            rstream_init_fd((&raw mut main_loop).cast(), stdio_in, stdin_dup_fd);
            wstream_init_fd((&raw mut main_loop).cast(), stdio_out, stdout_dup_fd, 0);
        }

        if rpc {
            rpc_start(channel);
        } else {
            let ct = channel_tail(channel);
            (*ct).on_data = on_output;
            callback_reader_start(&raw mut (*ct).on_data, c"stdin".as_ptr());
            rstream_start(stdio_in, Some(on_channel_data), channel.cast());
        }

        (*channel_prefix(channel)).id
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_send(
    id: u64,
    data: *mut c_char,
    len: usize,
    data_owned: bool,
    error: *mut *const c_char,
) -> usize {
    unsafe {
        let chan = find_channel(id);
        let mut written: usize = 0;

        'retfree: {
            if chan.is_null() {
                {
                    *error = gettext(e_invchan.as_ptr());
                }
                break 'retfree;
            }

            let cp = channel_prefix(chan);
            let ct = channel_tail(chan);

            if (*cp).streamtype == kChannelStreamStderr {
                if *channel_stream_ptr(chan).cast::<bool>() {
                    {
                        *error = gettext(c"Can't send data to closed stream".as_ptr());
                    }
                    break 'retfree;
                }
                {
                    let wres = os_write(STDERR_FILENO, data, len, false);
                    if wres >= 0 {
                        written = wres as usize;
                    }
                }
                break 'retfree;
            }

            if (*cp).streamtype == kChannelStreamInternal {
                if (*ct).is_rpc {
                    {
                        *error = gettext(c"Can't send raw data to rpc channel".as_ptr());
                    }
                    break 'retfree;
                }
                let internal = channel_stream_ptr(chan).cast::<InternalState>();
                if (*ct).term.is_null() || (*internal).closed {
                    {
                        *error = gettext(c"Can't send data to closed stream".as_ptr());
                    }
                    break 'retfree;
                }
                terminal_receive((*ct).term, data, len);
                written = len;
                break 'retfree;
            }

            let in_stream = channel_instream(chan);
            if *stream_closed_ptr(in_stream) {
                {
                    *error = gettext(c"Can't send data to closed stream".as_ptr());
                }
                break 'retfree;
            }

            if (*ct).is_rpc {
                {
                    *error = gettext(c"Can't send raw data to rpc channel".as_ptr());
                }
                break 'retfree;
            }

            let buf_data = if data_owned {
                data
            } else {
                xmemdup(data.cast(), len).cast::<c_char>()
            };
            let buf = wstream_new_buffer(buf_data, len, 1, Some(xfree));
            return if wstream_write(in_stream, buf) == 0 {
                len
            } else {
                0
            };
        }

        if data_owned {
            xfree(data.cast());
        }
        written
    }
}

#[inline]
unsafe fn buffer_to_tv_list(buf: *const c_char, len: usize) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        tv_list_append_string(l, c"".as_ptr(), 0);
        if len > 0 {
            encode_list_write(l.cast(), buf, len);
        }
        l
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn on_channel_data(
    stream: *mut RStream,
    buf: *const c_char,
    count: usize,
    data: *mut c_void,
    eof: bool,
) -> usize {
    unsafe {
        let chan = data.cast::<Channel>();
        let ct = channel_tail(chan);
        on_channel_output(stream, chan, buf, count, eof, &raw mut (*ct).on_data)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn on_job_stderr(
    stream: *mut RStream,
    buf: *const c_char,
    count: usize,
    data: *mut c_void,
    eof: bool,
) -> usize {
    unsafe {
        let chan = data.cast::<Channel>();
        let ct = channel_tail(chan);
        on_channel_output(stream, chan, buf, count, eof, &raw mut (*ct).on_stderr)
    }
}

unsafe fn on_channel_output(
    _stream: *mut RStream,
    chan: *mut Channel,
    buf: *const c_char,
    count: usize,
    eof: bool,
    reader: *mut CallbackReader,
) -> usize {
    unsafe {
        let ct = channel_tail(chan);
        if !(*ct).term.is_null() {
            terminal_receive((*ct).term, buf, count);
        }

        if eof {
            (*reader).eof = true;
        }

        #[cfg(windows)]
        if (*reader).fwd_err && count > 0 {
            let wres = os_write(STDOUT_FILENO, buf, count, false);
            return wres.max(0) as usize;
        }

        if callback_reader_set(&*reader) {
            ga_concat_len(&raw mut (*reader).buffer, buf, count);
            schedule_channel_event(chan);
        }

        count
    }
}

unsafe fn schedule_channel_event(chan: *mut Channel) {
    unsafe {
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);
        if !(*ct).callback_scheduled {
            if !(*ct).callback_busy {
                put_event_1((*cp).events, on_channel_event, chan.cast());
                channel_incref(chan);
            }
            (*ct).callback_scheduled = true;
        }
    }
}

unsafe extern "C" fn on_channel_event(args: *mut *mut c_void) {
    unsafe {
        let chan = (*args).cast::<Channel>();
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        (*ct).callback_busy = true;
        (*ct).callback_scheduled = false;

        let exit_status = (*ct).exit_status;
        channel_reader_callbacks(chan, &raw mut (*ct).on_data);
        channel_reader_callbacks(chan, &raw mut (*ct).on_stderr);
        if exit_status > -1 {
            channel_callback_call(chan, ptr::null_mut());
            (*ct).exit_status = -1;
        }

        (*ct).callback_busy = false;
        if (*ct).callback_scheduled {
            put_event_1((*cp).events, on_channel_event, chan.cast());
            channel_incref(chan);
        }

        channel_decref(chan);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_reader_callbacks(chan: *mut Channel, reader: *mut CallbackReader) {
    unsafe {
        if (*reader).buffered {
            if (*reader).eof {
                if !(*reader).self_dict.is_null() {
                    if tv_dict_find((*reader).self_dict, (*reader).reader_type, -1).is_null() {
                        let data = buffer_to_tv_list(
                            (*reader).buffer.ga_data.cast(),
                            (*reader).buffer.ga_len as usize,
                        );
                        let type_len = CStr::from_ptr((*reader).reader_type).to_bytes().len();
                        let _ = tv_dict_add_list(
                            (*reader).self_dict,
                            (*reader).reader_type,
                            type_len,
                            data,
                        );
                    } else {
                        semsg!(
                            gettext(e_streamkey.as_ptr()),
                            (*reader).reader_type,
                            (*channel_prefix(chan)).id,
                        );
                    }
                } else {
                    channel_callback_call(chan, reader);
                }
                (*reader).eof = false;
            }
        } else {
            let is_eof = (*reader).eof;
            if (*reader).buffer.ga_len > 0 {
                channel_callback_call(chan, reader);
            }
            if is_eof {
                channel_callback_call(chan, reader);
                (*reader).eof = false;
            }
        }
    }
}

unsafe extern "C" fn channel_proc_exit_cb(proc_: *mut Proc, status: c_int, data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        if !(*ct).term.is_null() {
            terminal_close(&raw mut (*ct).term, status);
        }

        if !exiting && ui_client_channel_id == (*cp).id {
            ui_client_attach_to_restarted_server((*proc_header(proc_)).status != 0);
            if ui_client_channel_id == (*cp).id {
                exit_on_closed_chan(status);
            }
        }

        let exited = status >= 0;
        if exited && (*ct).on_exit.cb_type != kCallbackNone {
            schedule_channel_event(chan);
        }
        if exited {
            (*ct).exit_status = status;
        }

        channel_decref(chan);
    }
}

unsafe extern "C" fn channel_proc_state_cb(_proc: *mut Proc, suspended: bool, data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let ct = channel_tail(chan);
        if !(*ct).term.is_null() {
            terminal_set_state((*ct).term, suspended);
        }
    }
}

unsafe fn channel_callback_call(chan: *mut Channel, reader: *mut CallbackReader) {
    unsafe {
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        let mut argv: [typval_T; 4] = std::mem::zeroed();
        argv[0].v_type = VarType::VAR_NUMBER;
        argv[0].v_lock = VAR_UNLOCKED;
        argv[0].vval = TypvalVval {
            v_number: (*cp).id as varnumber_T,
        };

        let cb: *mut Callback = if !reader.is_null() {
            let l = buffer_to_tv_list(
                (*reader).buffer.ga_data.cast(),
                (*reader).buffer.ga_len as usize,
            );
            argv[1].v_type = VarType::VAR_LIST;
            argv[1].v_lock = VAR_UNLOCKED;
            argv[1].vval = TypvalVval { v_list: l };
            tv_list_ref(l);
            ga_clear(&raw mut (*reader).buffer);
            argv[2].vval = TypvalVval {
                v_string: (*reader).reader_type.cast_mut(),
            };
            &raw mut (*reader).cb
        } else {
            argv[1].v_type = VarType::VAR_NUMBER;
            argv[1].v_lock = VAR_UNLOCKED;
            argv[1].vval = TypvalVval {
                v_number: varnumber_T::from((*ct).exit_status),
            };
            argv[2].vval = TypvalVval {
                v_string: c"exit".as_ptr().cast_mut(),
            };
            &raw mut (*ct).on_exit
        };

        argv[2].v_type = VarType::VAR_STRING;
        argv[2].v_lock = VAR_UNLOCKED;

        {
            let mut rettv: typval_T = std::mem::zeroed();
            let _ = callback_call(cb, 3, argv.as_mut_ptr(), &raw mut rettv);
            tv_clear(&raw mut rettv);
        }

        if !reader.is_null() {
            tv_list_unref(argv[1].vval.v_list);
        }
    }
}

/// Allocate terminal for channel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_terminal_alloc(buf: *mut buf_T, chan: *mut Channel) {
    unsafe {
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);
        let topts = TerminalOptions {
            data: chan.cast(),
            width: *pty_width_ptr(chan),
            height: *pty_height_ptr(chan),
            read_pause_cb: Some(term_read_pause),
            write_cb: Some(term_write),
            resize_cb: Some(term_resize),
            resume_cb: Some(term_resume),
            close_cb: Some(term_close),
            force_crlf: false,
        };
        (*buf).b_p_channel = (*cp).id as OptInt;
        channel_incref(chan);
        {
            (*ct).term = terminal_alloc(buf, topts);
        }
    }
}

unsafe extern "C" fn term_read_pause(pause: bool, data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
        let out_stream = proc_out(proc_ptr);
        if *stream_closed_ptr(out_stream.cast()) {
            return;
        }
        if pause {
            rstream_stop_inner(out_stream);
        } else {
            rstream_start_inner(out_stream);
        }
    }
}

unsafe extern "C" fn term_write(buf: *const c_char, size: usize, data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
        let in_stream = proc_in(proc_ptr);
        if *stream_closed_ptr(in_stream) {
            return;
        }
        let wbuf = wstream_new_buffer(
            xmemdup(buf.cast(), size).cast::<c_char>(),
            size,
            1,
            Some(xfree),
        );
        let _ = wstream_write(in_stream, wbuf);
    }
}

unsafe extern "C" fn term_resize(width: u16, height: u16, data: *mut c_void) {
    let chan = data.cast::<Channel>();
    unsafe {
        pty_proc_resize(channel_stream_ptr(chan).cast(), width, height);
    }
}

unsafe extern "C" fn term_resume(data: *mut c_void) {
    let chan = data.cast::<Channel>();
    unsafe {
        pty_proc_resume(channel_stream_ptr(chan).cast());
    }
}

unsafe extern "C" fn term_delayed_free(argv: *mut *mut c_void) {
    unsafe {
        let chan = (*argv).cast::<Channel>();
        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);
        let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
        if (*stream_fields(proc_in(proc_ptr))).pending_reqs != 0
            || (*stream_fields(proc_out(proc_ptr).cast())).pending_reqs != 0
        {
            put_event_1((*cp).events, term_delayed_free, chan.cast());
            return;
        }

        if !(*ct).term.is_null() {
            terminal_destroy(&raw mut (*ct).term);
        }
        channel_decref(chan);
    }
}

unsafe extern "C" fn term_close(data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let cp = channel_prefix(chan);
        proc_stop(channel_stream_ptr(chan).cast());
        put_event_1((*cp).events, term_delayed_free, data);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_event(chan: *mut Channel, event: event_T) {
    unsafe {
        if has_event(event) {
            channel_incref(chan);
            let qc = loop_queues((&raw mut main_loop).cast());
            put_event_2(
                (*qc).events,
                set_info_event,
                chan.cast(),
                (event as isize) as *mut c_void,
            );
        }
    }
}

unsafe extern "C" fn set_info_event(argv: *mut *mut c_void) {
    unsafe {
        let chan = (*argv).cast::<Channel>();
        let event = (*argv.add(1) as isize) as event_T;

        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let dict = get_v_event(&raw mut save_v_event);
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let info = channel_info((*channel_prefix(chan)).id, &raw mut arena);
        let mut retval: typval_T = std::mem::zeroed();
        object_to_vim(
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData { dict: info },
            },
            &raw mut retval,
            ptr::null_mut(),
        );
        assert_eq!(retval.v_type, VarType::VAR_DICT);
        let _ = tv_dict_add_dict(dict, c"info".as_ptr(), 4, retval.vval.v_dict);
        tv_dict_set_keys_readonly(dict);

        let _ = apply_autocmds(event, ptr::null_mut(), ptr::null_mut(), true, curbuf);

        restore_v_event(dict, &raw mut save_v_event);
        arena_mem_free(arena_finish(&raw mut arena));
        channel_decref(chan);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_job_running(id: u64) -> bool {
    unsafe {
        let chan = find_channel(id);
        if chan.is_null() {
            return false;
        }
        let cp = channel_prefix(chan);
        if (*cp).streamtype != kChannelStreamProc {
            return false;
        }
        let ph = proc_header(channel_stream_ptr(chan).cast());
        !((*ph).status >= 0 || (*ph).stopped_time != 0)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_info(id: u64, arena: *mut Arena) -> Dict {
    unsafe {
        let chan = find_channel(id);
        if chan.is_null() {
            return Dict {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            };
        }

        let mut info = arena_dict(arena, 9);

        let cp = channel_prefix(chan);
        let ct = channel_tail(chan);

        put_c(
            &mut info,
            c"id",
            Object {
                obj_type: ObjectType::kObjectTypeInteger,
                data: ObjectData {
                    integer: (*cp).id as Integer,
                },
            },
        );

        let stream_desc: &'static CStr = match (*cp).streamtype {
            kChannelStreamProc => {
                let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
                let ph = proc_header(proc_ptr);
                if (*ph).r#type == kProcTypePty {
                    let name = pty_proc_tty_name(proc_ptr.cast());
                    let s = if name.is_null() {
                        NvimString {
                            data: c"".as_ptr().cast_mut(),
                            size: 0,
                        }
                    } else {
                        let len = CStr::from_ptr(name).to_bytes().len();
                        NvimString {
                            data: arena_memdupz(arena, name, len),
                            size: len,
                        }
                    };
                    put_c(
                        &mut info,
                        c"pty",
                        Object {
                            obj_type: ObjectType::kObjectTypeString,
                            data: ObjectData { string: s },
                        },
                    );
                }

                let args = (*ph).argv;
                let mut argv = Array {
                    items: ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                };
                if !args.is_null() {
                    let mut n: usize = 0;
                    while !(*args.add(n)).is_null() {
                        n += 1;
                    }
                    {
                        argv = arena_array(arena, n);
                    }
                    for i in 0..n {
                        add_c(
                            &mut argv,
                            Object {
                                obj_type: ObjectType::kObjectTypeString,
                                data: ObjectData {
                                    string: cstr_as_string(*args.add(i)),
                                },
                            },
                        );
                    }
                }
                put_c(
                    &mut info,
                    c"argv",
                    Object {
                        obj_type: ObjectType::kObjectTypeArray,
                        data: ObjectData { array: argv },
                    },
                );
                c"job"
            }
            kChannelStreamStdio => c"stdio",
            kChannelStreamStderr => c"stderr",
            kChannelStreamInternal => {
                put_c(
                    &mut info,
                    c"internal",
                    Object {
                        obj_type: ObjectType::kObjectTypeBoolean,
                        data: ObjectData { boolean: true },
                    },
                );
                c"socket"
            }
            _ => c"socket",
        };

        put_c(
            &mut info,
            c"stream",
            Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData {
                    string: cstr_as_string(stream_desc.as_ptr()),
                },
            },
        );

        let mode_desc: &'static CStr = if (*ct).is_rpc {
            put_c(
                &mut info,
                c"client",
                Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: (*ct).rpc.info,
                    },
                },
            );
            put_c(
                &mut info,
                c"detach",
                Object {
                    obj_type: ObjectType::kObjectTypeBoolean,
                    data: ObjectData {
                        boolean: (*ct).detach,
                    },
                },
            );
            c"rpc"
        } else if !(*ct).term.is_null() {
            let tbuf = terminal_buf((*ct).term);
            put_c(
                &mut info,
                c"buf",
                Object {
                    obj_type: ObjectType::kObjectTypeBuffer,
                    data: ObjectData {
                        integer: i64::from(tbuf),
                    },
                },
            );
            put_c(
                &mut info,
                c"buffer",
                Object {
                    obj_type: ObjectType::kObjectTypeBuffer,
                    data: ObjectData {
                        integer: i64::from(tbuf),
                    },
                },
            );
            put_c(
                &mut info,
                c"exitcode",
                Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: i64::from((*ct).exit_status),
                    },
                },
            );
            c"terminal"
        } else {
            c"bytes"
        };

        put_c(
            &mut info,
            c"mode",
            Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData {
                    string: cstr_as_string(mode_desc.as_ptr()),
                },
            },
        );

        info
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn channel_all_info(arena: *mut Arena) -> Array {
    unsafe {
        let count = channels.set.h.size as usize;

        let items = arena_alloc(arena, count * size_of::<i64>(), true).cast::<i64>();
        let mut size: usize = 0;
        for i in 0..(channels.set.h.n_keys as usize) {
            let id = *channels.set.keys.add(i);
            *items.add(size) = id as i64;
            size += 1;
        }
        if size > 1 {
            let slice = std::slice::from_raw_parts_mut(items, size);
            slice.sort_unstable();
        }

        let mut ret = arena_array(arena, size);

        for i in 0..size {
            let info = channel_info((*items.add(i)) as u64, arena);
            add_c(
                &mut ret,
                Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData { dict: info },
                },
            );
        }

        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_reader_set_checks_callback_type_or_self_dict() {
        let mut reader = CallbackReader {
            cb: Callback {
                data: ptr::null_mut(),
                cb_type: kCallbackNone,
            },
            self_dict: ptr::null_mut(),
            buffer: crate::garray::garray_T {
                ga_len: 0,
                ga_maxlen: 0,
                ga_itemsize: 0,
                ga_growsize: 0,
                ga_data: ptr::null_mut(),
            },
            eof: false,
            buffered: false,
            fwd_err: false,
            reader_type: ptr::null(),
        };
        assert!(!callback_reader_set(&reader));
        reader.cb.cb_type = 1;
        assert!(callback_reader_set(&reader));
    }
}
