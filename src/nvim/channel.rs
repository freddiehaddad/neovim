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
use crate::event::multiqueue::{Event, multiqueue_new_child, multiqueue_put_event};
use crate::event::proc::{Proc, exit_on_closed_chan, proc_spawn, proc_stop};
use crate::event::rstream::{
    RStream, rstream_init, rstream_init_fd, rstream_may_close, rstream_start, rstream_start_inner,
    rstream_stop_inner,
};
use crate::event::socket::{SocketWatcher, socket_connect, socket_watcher_accept};
use crate::event::stream::{Stream, StreamCbData, StreamInternalData, stream_may_close};
use crate::event::wstream::{wstream_init, wstream_init_fd, wstream_new_vec_buffer, wstream_write};
#[cfg(not(windows))]
use crate::fileio::fcntl;
use crate::globals::*;
use crate::lua::converter::LuaRef;
use crate::lua::executor::api_free_luaref;
use crate::memory::{Arena, arena_finish, arena_mem_free, arena_memdupz, xfree, xstrdup};
use crate::message::semsg;
pub use crate::msgpack_rpc::channel::Channel;
use crate::msgpack_rpc::channel::{
    ChannelPart, ChannelStreamType, RpcState, channel_instream, find_channel, rpc_close, rpc_init,
    rpc_start,
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
use crate::state::save_v_event_T;
use crate::strings::gettext;
use crate::terminal::{
    terminal_alloc, terminal_buf, terminal_close, terminal_destroy, terminal_receive,
    terminal_set_state,
};
use crate::ui_client::ui_client_attach_to_restarted_server;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type ChannelStdinMode = c_int;

#[repr(C)]
pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: *mut dict_T,
    pub buffer: Vec<c_char>,
    pub eof: bool,
    pub buffered: bool,
    pub fwd_err: bool,
    pub reader_type: *const c_char,
}

impl CallbackReader {
    #[inline]
    pub const fn none() -> Self {
        Self {
            cb: Callback::NONE,
            self_dict: ptr::null_mut(),
            buffer: Vec::new(),
            eof: false,
            buffered: false,
            fwd_err: false,
            reader_type: ptr::null(),
        }
    }
}

impl Drop for CallbackReader {
    fn drop(&mut self) {
        unsafe {
            callback_free(&raw mut self.cb);
        }
    }
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
    pub data: *mut Channel,
    pub width: u16,
    pub height: u16,
    pub read_pause_cb: Option<unsafe fn(pause: bool, data: *mut Channel)>,
    pub write_cb: Option<unsafe fn(buffer: *const c_char, size: usize, data: *mut Channel)>,
    pub resize_cb: Option<unsafe fn(width: u16, height: u16, data: *mut Channel)>,
    pub resume_cb: Option<unsafe fn(data: *mut Channel)>,
    pub close_cb: Option<unsafe fn(data: *mut Channel)>,
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
    reader.cb.cb_type() != kCallbackNone || !reader.self_dict.is_null()
}

use crate::api::private::helpers::add_c;

/// Teardown the module.
pub unsafe fn channel_teardown() {
    unsafe {
        let chans_ptr = &raw const channels;
        let all_chans: Vec<*mut Channel> = (*chans_ptr).values().copied().collect();
        for chan in all_chans {
            channel_close((*chan).id, kChannelPartAll, ptr::null_mut());
        }
    }
}

pub unsafe fn channel_free_all_mem() {
    unsafe {
        let chans_ptr = &raw mut channels;
        for (_, chan) in std::mem::take(&mut *chans_ptr) {
            drop(Box::from_raw(chan));
        }

        callback_free(&raw mut on_print);
    }
}

/// Closes a channel.
pub unsafe fn channel_close(id: u64, part: ChannelPart, mut error: *mut *const c_char) -> bool {
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

        let cp = chan;
        let ct = chan;

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
                rstream_may_close(&raw mut (*chan).socket);
            }
            kChannelStreamProc => {
                let proc_ptr = &raw mut (*chan).uv.proc_;
                if part == kChannelPartStdin || close_main {
                    stream_may_close(&raw mut (*proc_ptr).r#in);
                }
                if part == kChannelPartStdout || close_main {
                    rstream_may_close(&raw mut (*proc_ptr).out);
                }
                if part == kChannelPartStderr || part == kChannelPartAll {
                    rstream_may_close(&raw mut (*proc_ptr).err);
                }
                if (*proc_ptr).r#type == kProcTypePty && part == kChannelPartAll {
                    pty_proc_close_master(proc_ptr);
                }
            }
            kChannelStreamStdio => {
                if part == kChannelPartStdin || close_main {
                    rstream_may_close(&raw mut (*chan).stdio_in);
                }
                if part == kChannelPartStdout || close_main {
                    stream_may_close(&raw mut (*chan).stdio_out);
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
                if !(*chan).err_closed {
                    (*chan).err_closed = true;
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
                    let internal = &raw mut (*chan).internal;
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
pub unsafe fn channel_init() {
    unsafe {
        let _ = channel_alloc(kChannelStreamStderr);
        rpc_init();
    }
}

/// Allocates a channel.
pub unsafe fn channel_alloc(r#type: ChannelStreamType) -> *mut Channel {
    unsafe {
        let id = if r#type == kChannelStreamStdio {
            CHAN_STDIO
        } else if r#type == kChannelStreamStderr {
            CHAN_STDERR
        } else {
            let id = NEXT_CHAN_ID;
            NEXT_CHAN_ID += 1;
            id
        };
        assert!(id <= (i64::MAX as u64));

        let qc = loop_queues((&raw mut main_loop).cast());
        let chan = Box::into_raw(Box::new(Channel {
            id,
            refcount: 1,
            events: multiqueue_new_child((*qc).events),
            streamtype: r#type,
            uv: crate::event::proc::LibuvProc::new(),
            socket: RStream::new(),
            stdio_in: RStream::new(),
            stdio_out: Stream::new(),
            err_closed: false,
            internal: InternalState {
                cb: 0,
                closed: false,
            },
            is_rpc: false,
            detach: r#type != kChannelStreamStdio,
            rpc: RpcState::new(),
            term: ptr::null_mut(),
            on_data: CallbackReader::none(),
            on_stderr: CallbackReader::none(),
            on_exit: Callback::NONE,
            exit_status: -1,
            callback_busy: false,
            callback_scheduled: false,
            did_close_event: false,
        }));

        {
            let chans_ptr = &raw mut channels;
            (*chans_ptr).insert(id, chan);
        }

        chan
    }
}

pub unsafe fn channel_create_event(chan: *mut Channel, _ext_source: *const c_char) {
    unsafe {
        channel_event(chan, EVENT_CHANOPEN);
    }
}

pub unsafe fn channel_incref(chan: *mut Channel) {
    unsafe {
        (*chan).refcount += 1;
    }
}

pub unsafe fn channel_decref(chan: *mut Channel) {
    unsafe {
        if (*chan).refcount == 1 && !(*chan).did_close_event {
            (*chan).did_close_event = true;
            channel_event(chan, EVENT_CHANCLOSE);
        }

        (*chan).refcount -= 1;
        if (*chan).refcount == 0 {
            let qc = loop_queues((&raw mut main_loop).cast());
            multiqueue_put_event((*qc).events, Event::ChannelFree(chan));
        }
    }
}

pub unsafe fn callback_reader_start(reader: *mut CallbackReader, r#type: *const c_char) {
    unsafe {
        (*reader).reader_type = r#type;
    }
}

pub unsafe fn free_channel_event(chan: *mut Channel) {
    unsafe {
        let chans_ptr = &raw mut channels;
        let _ = (*chans_ptr).remove(&(*chan).id);
        drop(Box::from_raw(chan));
    }
}

unsafe fn channel_destroy_early(chan: *mut Channel) {
    unsafe {
        let cp = chan;
        NEXT_CHAN_ID -= 1;
        if (*cp).id != NEXT_CHAN_ID {
            std::process::abort();
        }
        let chans_ptr = &raw mut channels;
        let _ = (*chans_ptr).remove(&(*cp).id);
        (*cp).id = 0;

        (*cp).refcount -= 1;
        if (*cp).refcount != 0 {
            std::process::abort();
        }

        let qc = loop_queues((&raw mut main_loop).cast());
        multiqueue_put_event((*qc).events, Event::ChannelFree(chan));
    }
}

unsafe fn close_cb(_stream: *mut Stream, data: StreamInternalData) {
    unsafe {
        channel_decref(data.as_channel());
    }
}

/// Starts a job and returns the associated channel.
pub unsafe fn channel_job_start(
    mut argv: Vec<*mut c_char>,
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
        let cp = chan;
        let ct = chan;
        (*ct).on_data = on_stdout;
        (*ct).on_stderr = on_stderr;
        (*ct).on_exit = on_exit;

        let proc_ptr = &raw mut (*chan).uv.proc_;
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
                crate::path::clear_cstrings(&mut argv);
                channel_destroy_early(chan);
                *status_out = 0;
                return ptr::null_mut();
            }
            (*proc_ptr).r#type = kProcTypePty;
            (*proc_ptr).data = chan;
            {
                (*proc_ptr).loop_ = (&raw mut main_loop).cast();
            }
            (*proc_ptr).status = -1;
            (*proc_ptr).out.s.fd = STDOUT_FILENO;
            (*proc_ptr).err.s.fd = STDERR_FILENO;
            (*proc_ptr).width = if pty_width > 0 { pty_width } else { 80 };
            (*proc_ptr).height = if pty_height > 0 { pty_height } else { 24 };
            #[cfg(not(windows))]
            {
                (*proc_ptr).tty_fd = -1;
            }
        } else {
            (*proc_ptr).r#type = kProcTypeUv;
            (*proc_ptr).data = chan;
            {
                (*proc_ptr).loop_ = (&raw mut main_loop).cast();
            }
            (*proc_ptr).status = -1;
            (*proc_ptr).out.s.fd = STDOUT_FILENO;
            (*proc_ptr).err.s.fd = STDERR_FILENO;
        }

        let ph = proc_ptr;
        let pt = proc_ptr;
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
            (*ph).argv.as_slice()[0]
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
            wstream_init(&raw mut (*proc_ptr).r#in, 0);
        }
        if has_out {
            rstream_init(&raw mut (*proc_ptr).out);
        }

        if rpc {
            rpc_start(chan);
        } else if has_out {
            callback_reader_start(&raw mut (*ct).on_data, c"stdout".as_ptr());
            rstream_start(
                &raw mut (*proc_ptr).out,
                Some(on_channel_data),
                StreamCbData::Channel(chan),
            );
        }

        if has_err {
            callback_reader_start(&raw mut (*ct).on_stderr, c"stderr".as_ptr());
            rstream_init(&raw mut (*proc_ptr).err);
            rstream_start(
                &raw mut (*proc_ptr).err,
                Some(on_job_stderr),
                StreamCbData::Channel(chan),
            );
        }

        *status_out = (*cp).id as varnumber_T;
        chan
    }
}

pub unsafe fn channel_connect(
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
            (*channel).internal.cb = LUA_NOREF;
            rpc_start(channel);
            channel_create_event(channel, address);
            return (*channel).id;
        }

        let channel = channel_alloc(kChannelStreamSocket);
        let socket_stream = &raw mut (*channel).socket;
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

        (*socket_stream).s.internal_close_cb = Some(close_cb);
        (*socket_stream).s.internal_data = StreamInternalData::Channel(channel);
        wstream_init(&raw mut (*socket_stream).s, 0);
        rstream_init(socket_stream);

        if rpc {
            rpc_start(channel);
        } else {
            (*channel).on_data = on_output;
            callback_reader_start(&raw mut (*channel).on_data, c"data".as_ptr());
            rstream_start(
                socket_stream,
                Some(on_channel_data),
                StreamCbData::Channel(channel),
            );
        }

        channel_create_event(channel, address);
        (*channel).id
    }
}

/// Creates an RPC channel from a tcp/pipe socket connection.
pub unsafe fn channel_from_connection(watcher: *mut SocketWatcher) {
    unsafe {
        let channel = channel_alloc(kChannelStreamSocket);
        let socket_stream = &raw mut (*channel).socket;
        let result = socket_watcher_accept(watcher, socket_stream);
        if result != 0 {
            channel_destroy_early(channel);
            return;
        }
        (*socket_stream).s.internal_close_cb = Some(close_cb);
        (*socket_stream).s.internal_data = StreamInternalData::Channel(channel);
        wstream_init(&raw mut (*socket_stream).s, 0);
        rstream_init(socket_stream);
        rpc_start(channel);
        channel_create_event(channel, (*watcher).addr.as_ptr());
    }
}

/// Creates an API channel from stdin/stdout. Used when embedding Nvim.
pub unsafe fn channel_from_stdio(
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

        let stdio_in = &raw mut (*channel).stdio_in;
        let stdio_out = &raw mut (*channel).stdio_out;
        {
            rstream_init_fd((&raw mut main_loop).cast(), stdio_in, stdin_dup_fd);
            wstream_init_fd((&raw mut main_loop).cast(), stdio_out, stdout_dup_fd, 0);
        }

        if rpc {
            rpc_start(channel);
        } else {
            (*channel).on_data = on_output;
            callback_reader_start(&raw mut (*channel).on_data, c"stdin".as_ptr());
            rstream_start(
                stdio_in,
                Some(on_channel_data),
                StreamCbData::Channel(channel),
            );
        }

        (*channel).id
    }
}

pub unsafe fn channel_send(id: u64, data: Vec<c_char>, error: *mut *const c_char) -> usize {
    unsafe {
        let chan = find_channel(id);
        let len = data.len();

        if chan.is_null() {
            *error = gettext(e_invchan.as_ptr());
            return 0;
        }

        let cp = chan;
        let ct = chan;

        if (*cp).streamtype == kChannelStreamStderr {
            if (*chan).err_closed {
                *error = gettext(c"Can't send data to closed stream".as_ptr());
                return 0;
            }
            let wres = os_write(STDERR_FILENO, data.as_ptr(), len, false);
            return if wres >= 0 { wres as usize } else { 0 };
        }

        if (*cp).streamtype == kChannelStreamInternal {
            if (*ct).is_rpc {
                *error = gettext(c"Can't send raw data to rpc channel".as_ptr());
                return 0;
            }
            let internal = &raw mut (*chan).internal;
            if (*ct).term.is_null() || (*internal).closed {
                *error = gettext(c"Can't send data to closed stream".as_ptr());
                return 0;
            }
            terminal_receive((*ct).term, data.as_ptr(), len);
            return len;
        }

        let in_stream = channel_instream(chan);
        if (*in_stream).closed {
            *error = gettext(c"Can't send data to closed stream".as_ptr());
            return 0;
        }

        if (*ct).is_rpc {
            *error = gettext(c"Can't send raw data to rpc channel".as_ptr());
            return 0;
        }

        let buf = wstream_new_vec_buffer(data);
        if wstream_write(in_stream, buf) == 0 {
            len
        } else {
            0
        }
    }
}

#[inline]
unsafe fn buffer_to_tv_list(buf: *const c_char, len: usize) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        tv_list_append_string(l, c"".as_ptr(), 0);
        if len > 0 {
            encode_list_write(l, buf, len);
        }
        l
    }
}

pub unsafe fn on_channel_data(
    stream: *mut RStream,
    buf: *const c_char,
    count: usize,
    data: StreamCbData,
    eof: bool,
) -> usize {
    unsafe {
        let chan = data.as_channel();
        let ct = chan;
        on_channel_output(stream, chan, buf, count, eof, &raw mut (*ct).on_data)
    }
}

pub unsafe fn on_job_stderr(
    stream: *mut RStream,
    buf: *const c_char,
    count: usize,
    data: StreamCbData,
    eof: bool,
) -> usize {
    unsafe {
        let chan = data.as_channel();
        let ct = chan;
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
        let ct = chan;
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
            if !buf.is_null() && count > 0 {
                (*reader)
                    .buffer
                    .extend_from_slice(std::slice::from_raw_parts(buf, count));
            }
            schedule_channel_event(chan);
        }

        count
    }
}

unsafe fn schedule_channel_event(chan: *mut Channel) {
    unsafe {
        let cp = chan;
        let ct = chan;
        if !(*ct).callback_scheduled {
            if !(*ct).callback_busy {
                multiqueue_put_event((*cp).events, Event::ChannelCallback(chan));
                channel_incref(chan);
            }
            (*ct).callback_scheduled = true;
        }
    }
}

pub unsafe fn on_channel_event(chan: *mut Channel) {
    unsafe {
        let cp = chan;
        let ct = chan;

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
            multiqueue_put_event((*cp).events, Event::ChannelCallback(chan));
            channel_incref(chan);
        }

        channel_decref(chan);
    }
}

pub unsafe fn channel_reader_callbacks(chan: *mut Channel, reader: *mut CallbackReader) {
    unsafe {
        if (*reader).buffered {
            if (*reader).eof {
                if !(*reader).self_dict.is_null() {
                    if tv_dict_find((*reader).self_dict, (*reader).reader_type, -1).is_null() {
                        let data =
                            buffer_to_tv_list((*reader).buffer.as_ptr(), (*reader).buffer.len());
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
                            (*chan).id,
                        );
                    }
                } else {
                    channel_callback_call(chan, reader);
                }
                (*reader).eof = false;
            }
        } else {
            let is_eof = (*reader).eof;
            if !(*reader).buffer.is_empty() {
                channel_callback_call(chan, reader);
            }
            if is_eof {
                channel_callback_call(chan, reader);
                (*reader).eof = false;
            }
        }
    }
}

unsafe fn channel_proc_exit_cb(proc_: *mut Proc, status: c_int, chan: *mut Channel) {
    unsafe {
        let cp = chan;
        let ct = chan;

        if !(*ct).term.is_null() {
            terminal_close(&raw mut (*ct).term, status);
        }

        if !exiting && ui_client_channel_id == (*cp).id {
            ui_client_attach_to_restarted_server((*proc_).status != 0);
            if ui_client_channel_id == (*cp).id {
                exit_on_closed_chan(status);
            }
        }

        let exited = status >= 0;
        if exited && (*ct).on_exit.cb_type() != kCallbackNone {
            schedule_channel_event(chan);
        }
        if exited {
            (*ct).exit_status = status;
        }

        channel_decref(chan);
    }
}

unsafe fn channel_proc_state_cb(_proc: *mut Proc, suspended: bool, chan: *mut Channel) {
    unsafe {
        let ct = chan;
        if !(*ct).term.is_null() {
            terminal_set_state((*ct).term, suspended);
        }
    }
}

unsafe fn channel_callback_call(chan: *mut Channel, reader: *mut CallbackReader) {
    unsafe {
        let cp = chan;
        let ct = chan;

        let mut argv: [typval_T; 4] = [typval_T::new(); 4];
        argv[0].v_type = VarType::VAR_NUMBER;
        argv[0].v_lock = VAR_UNLOCKED;
        argv[0].vval = TypvalVval {
            v_number: (*cp).id as varnumber_T,
        };

        let cb: *mut Callback = if !reader.is_null() {
            let l = buffer_to_tv_list((*reader).buffer.as_ptr(), (*reader).buffer.len());
            argv[1].v_type = VarType::VAR_LIST;
            argv[1].v_lock = VAR_UNLOCKED;
            argv[1].vval = TypvalVval { v_list: l };
            tv_list_ref(l);
            (*reader).buffer = Vec::new();
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
            let mut rettv: typval_T = typval_T::new();
            let _ = callback_call(cb, 3, argv.as_mut_ptr(), &raw mut rettv);
            tv_clear(&raw mut rettv);
        }

        if !reader.is_null() {
            tv_list_unref(argv[1].vval.v_list);
        }
    }
}

/// Allocate terminal for channel.
pub unsafe fn channel_terminal_alloc(buf: *mut buf_T, chan: *mut Channel) {
    unsafe {
        let cp = chan;
        let ct = chan;
        let topts = TerminalOptions {
            data: chan,
            width: (*chan).uv.proc_.width,
            height: (*chan).uv.proc_.height,
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

unsafe fn term_read_pause(pause: bool, chan: *mut Channel) {
    unsafe {
        let out_stream = &raw mut (*chan).uv.proc_.out;
        if (*out_stream).s.closed {
            return;
        }
        if pause {
            rstream_stop_inner(out_stream);
        } else {
            rstream_start_inner(out_stream);
        }
    }
}

unsafe fn term_write(buf: *const c_char, size: usize, chan: *mut Channel) {
    unsafe {
        let in_stream = &raw mut (*chan).uv.proc_.r#in;
        if (*in_stream).closed {
            return;
        }
        let data = if size == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(buf, size).to_vec()
        };
        let wbuf = wstream_new_vec_buffer(data);
        let _ = wstream_write(in_stream, wbuf);
    }
}

unsafe fn term_resize(width: u16, height: u16, chan: *mut Channel) {
    unsafe {
        pty_proc_resize(&raw mut (*chan).uv.proc_, width, height);
    }
}

unsafe fn term_resume(chan: *mut Channel) {
    unsafe {
        pty_proc_resume(&raw mut (*chan).uv.proc_);
    }
}

pub unsafe fn term_delayed_free(chan: *mut Channel) {
    unsafe {
        let cp = chan;
        let ct = chan;
        let proc_ptr = &raw mut (*chan).uv.proc_;
        if (*proc_ptr).r#in.pending_reqs != 0 || (*proc_ptr).out.s.pending_reqs != 0 {
            multiqueue_put_event((*cp).events, Event::ChannelTermDelayedFree(chan));
            return;
        }

        if !(*ct).term.is_null() {
            terminal_destroy(&raw mut (*ct).term);
        }
        channel_decref(chan);
    }
}

unsafe fn term_close(chan: *mut Channel) {
    unsafe {
        let cp = chan;
        proc_stop(&raw mut (*chan).uv.proc_);
        multiqueue_put_event((*cp).events, Event::ChannelTermDelayedFree(chan));
    }
}

pub unsafe fn channel_event(chan: *mut Channel, event: event_T) {
    unsafe {
        if has_event(event) {
            channel_incref(chan);
            let qc = loop_queues((&raw mut main_loop).cast());
            multiqueue_put_event((*qc).events, Event::ChannelSetInfo { chan, event });
        }
    }
}

pub unsafe fn set_info_event(chan: *mut Channel, event: event_T) {
    unsafe {
        let mut save_v_event: save_v_event_T = save_v_event_T::new();
        let dict = get_v_event(&raw mut save_v_event);
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let info = channel_info((*chan).id, &raw mut arena);
        let mut retval: typval_T = typval_T::new();
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

pub unsafe fn channel_job_running(id: u64) -> bool {
    unsafe {
        let chan = find_channel(id);
        if chan.is_null() {
            return false;
        }
        if (*chan).streamtype != kChannelStreamProc {
            return false;
        }
        let ph = &raw mut (*chan).uv.proc_;
        !((*ph).status >= 0 || (*ph).stopped_time != 0)
    }
}

pub unsafe fn channel_info(id: u64, arena: *mut Arena) -> Dict {
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

        let cp = chan;
        let ct = chan;

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
                let proc_ptr = &raw mut (*chan).uv.proc_;
                let ph = proc_ptr;
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

                let args = (*ph).argv.as_slice();
                let mut argv = Array {
                    items: ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                };
                if !args.is_empty() {
                    let n = args.iter().take_while(|&&p| !p.is_null()).count();
                    argv = arena_array(arena, n);
                    for &arg in &args[..n] {
                        add_c(
                            &mut argv,
                            Object {
                                obj_type: ObjectType::kObjectTypeString,
                                data: ObjectData {
                                    string: cstr_as_string(arg),
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

pub unsafe fn channel_all_info(arena: *mut Arena) -> Array {
    unsafe {
        let chans_ptr = &raw const channels;
        let mut ret = arena_array(arena, (*chans_ptr).len());

        for &id in (*chans_ptr).keys() {
            let info = channel_info(id, arena);
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
        let mut reader = CallbackReader::none();
        assert!(!callback_reader_set(&reader));
        reader.cb.data = crate::eval::typval::CallbackData::lua(1);
        assert!(callback_reader_set(&reader));
        reader.cb.data = crate::eval::typval::CallbackData::None;
    }
}
