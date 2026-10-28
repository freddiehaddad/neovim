//! Async I/O and process/socket/stdio/terminal channel management (`src/nvim/channel.c`).

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{arena_array, arena_dict, cstr_as_string, put_c};
use crate::api::private::validate::{Array, Dict, NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{apply_autocmds, AuEvent, has_event};
use crate::buffer::FileBuf;
use crate::errors::*;
use crate::eval::encode::encode_list_write;
use crate::eval::typval::{
    Callback, TypvalVval, VAR_UNLOCKED, VarType, callback_free, TvDict, kCallbackNone, TvList,
    tv_clear, tv_dict_add_dict, tv_dict_add_list, tv_dict_find, tv_dict_free,
    tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_string, tv_list_ref, tv_list_unref,
    TypVal, Varnumber,
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
use crate::memory::{Arena, arena_finish, arena_mem_free, arena_memdupz};
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
use crate::state::SaveVEvent;
use crate::strings::gettext;
use crate::terminal::{
    terminal_alloc, terminal_buf, terminal_close, terminal_destroy, terminal_receive,
};
#[cfg(unix)]
use crate::terminal::terminal_set_state;
use crate::ui_client::ui_client_attach_to_restarted_server;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type ChannelStdinMode = c_int;

#[repr(C)]
pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: *mut TvDict,
    pub buffer: Vec<c_char>,
    pub eof: bool,
    pub buffered: bool,
    pub fwd_err: bool,
    pub reader_type: Option<&'static CStr>,
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
            reader_type: None,
        }
    }
}

impl Drop for CallbackReader {
    fn drop(&mut self) {
        callback_free(&mut self.cb);
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

struct ChannelModuleState {
    DID_STDIO: bool,
    NEXT_CHAN_ID: u64,
}

impl ChannelModuleState {
    const fn new() -> Self {
        Self {
            DID_STDIO: false,
            NEXT_CHAN_ID: CHAN_STDERR + 1,
        }
    }
}

static CHAN_STATE: crate::globals::GlobalState<ChannelModuleState> = crate::globals::GlobalState::new(ChannelModuleState::new());

#[inline]
fn chan_st() -> &'static mut ChannelModuleState {
    unsafe { &mut *CHAN_STATE.as_ptr() }
}

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
fn redirect_stderr_to_null() {
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
pub fn channel_teardown() {
    unsafe {
        let all_chans: Vec<*mut Channel> = g_st().channels.values().copied().collect();
        for chan in all_chans {
            channel_close((*chan).id, kChannelPartAll, ptr::null_mut());
        }
    }
}

pub fn channel_free_all_mem() {
    unsafe {
        for (_, chan) in std::mem::take(&mut g_st().channels) {
            drop(Box::from_raw(chan));
        }

        callback_free(&mut g_st().on_print);
    }
}

/// Closes a channel.
pub fn channel_close(id: u64, part: ChannelPart, mut error: *mut *const c_char) -> bool {
    unsafe {
        let mut dummy: *const c_char = ptr::null();
        if error.is_null() {
            error = &raw mut dummy;
        }

        let chan = find_channel(id);
        if chan.is_null() {
            if id < chan_st().NEXT_CHAN_ID {
                return true;
            }
            {
                *error = e_invchan.as_ptr();
            }
            return false;
        }

        let mut close_main = false;
        if part == kChannelPartRpc || part == kChannelPartAll {
            close_main = true;
            if (*chan).is_rpc {
                rpc_close(&mut *chan);
            } else if part == kChannelPartRpc {
                {
                    *error = e_invstream.as_ptr();
                }
                return false;
            }
        } else if (part == kChannelPartStdin || part == kChannelPartStdout) && (*chan).is_rpc {
            {
                *error = e_invstreamrpc.as_ptr();
            }
            return false;
        }

        match (*chan).streamtype {
            kChannelStreamSocket => {
                if !close_main {
                    {
                        *error = e_invstream.as_ptr();
                    }
                    return false;
                }
                rstream_may_close(&mut (*chan).socket);
            }
            kChannelStreamProc => {
                let proc_ptr = &mut (*chan).uv.proc_;
                if part == kChannelPartStdin || close_main {
                    stream_may_close(&mut proc_ptr.r#in);
                }
                if part == kChannelPartStdout || close_main {
                    rstream_may_close(&mut proc_ptr.out);
                }
                if part == kChannelPartStderr || part == kChannelPartAll {
                    rstream_may_close(&mut proc_ptr.err);
                }
                if proc_ptr.r#type == kProcTypePty && part == kChannelPartAll {
                    pty_proc_close_master(proc_ptr);
                }
            }
            kChannelStreamStdio => {
                if part == kChannelPartStdin || close_main {
                    rstream_may_close(&mut (*chan).stdio_in);
                }
                if part == kChannelPartStdout || close_main {
                    stream_may_close(&mut (*chan).stdio_out);
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
                    if !g_st().exiting {
                        redirect_stderr_to_null();
                    }
                    channel_decref( &mut *chan);
                }
            }
            kChannelStreamInternal => {
                if !close_main {
                    {
                        *error = e_invstream.as_ptr();
                    }
                    return false;
                }
                if !(*chan).term.is_null() {
                    let internal = &mut (*chan).internal;
                    api_free_luaref(internal.cb);
                    internal.cb = LUA_NOREF;
                    internal.closed = true;
                    terminal_close(&raw mut (*chan).term, 0);
                    (*chan).exit_status = 0;
                } else {
                    channel_decref( &mut *chan);
                }
            }
            _ => {}
        }

        true
    }
}

/// Initializes the module.
pub fn channel_init() {
    let _ = channel_alloc(kChannelStreamStderr);
        rpc_init();
}

/// Allocates a channel.
pub fn channel_alloc(r#type: ChannelStreamType) -> *mut Channel {
    unsafe {
        let id = if r#type == kChannelStreamStdio {
            CHAN_STDIO
        } else if r#type == kChannelStreamStderr {
            CHAN_STDERR
        } else {
            let id = chan_st().NEXT_CHAN_ID;
            chan_st().NEXT_CHAN_ID += 1;
            id
        };
        assert!(id <= (i64::MAX as u64));

        let qc = loop_queues(&mut g_st().main_loop);
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
            g_st().channels.insert(id, chan);
        }

        chan
    }
}

pub fn channel_create_event(chan: &mut Channel, _ext_source: *const c_char) {
    channel_event(chan, EVENT_CHANOPEN);
}

pub fn channel_incref(chan: &mut Channel) {
    chan.refcount += 1;
}

pub fn channel_decref(chan: &mut Channel) {
    unsafe {
        if chan.refcount == 1 && !chan.did_close_event {
            chan.did_close_event = true;
            channel_event(chan, EVENT_CHANCLOSE);
        }

        chan.refcount -= 1;
        if chan.refcount == 0 {
            let qc = loop_queues(&mut g_st().main_loop);
            multiqueue_put_event( &mut *(*qc).events, Event::ChannelFree(&raw mut *chan));
        }
    }
}

pub fn callback_reader_start(reader: &mut CallbackReader, r#type: &'static CStr) {
    reader.reader_type = Some(r#type);
}

pub fn free_channel_event(chan: &mut Channel) {
    unsafe {
        let _ = g_st().channels.remove(&chan.id);
        drop(Box::from_raw(chan));
    }
}

fn channel_destroy_early(chan: &mut Channel) {
    unsafe {
        chan_st().NEXT_CHAN_ID -= 1;
        if chan.id != chan_st().NEXT_CHAN_ID {
            std::process::abort();
        }
        let _ = g_st().channels.remove(&chan.id);
        chan.id = 0;

        chan.refcount -= 1;
        if chan.refcount != 0 {
            std::process::abort();
        }

        let qc = loop_queues(&mut g_st().main_loop);
        multiqueue_put_event( &mut *(*qc).events, Event::ChannelFree(&raw mut *chan));
    }
}

/// Starts a job and returns the associated channel.
pub fn channel_job_start(
    argv: Vec<std::ffi::CString>,
    exepath: impl crate::strings::IntoCCharPtr,
    on_stdout: CallbackReader,
    on_stderr: CallbackReader,
    on_exit: Callback,
    pty: bool,
    rpc: bool,
    overlapped: bool,
    detach: bool,
    stdin_mode: ChannelStdinMode,
    cwd: impl crate::strings::IntoCCharPtr,
    pty_width: u16,
    pty_height: u16,
    env: *mut TvDict,
    status_out: *mut Varnumber,
) -> *mut Channel { let exepath = exepath.into_c_char_ptr(); let cwd = cwd.into_c_char_ptr();
    unsafe {
        let chan = channel_alloc(kChannelStreamProc);
        (*chan).on_data = on_stdout;
        (*chan).on_stderr = on_stderr;
        (*chan).on_exit = on_exit;

        let proc_ptr = &raw mut (*chan).uv.proc_;
        if pty {
            if detach {
                {
                    semsg!(
                        gettext(e_invarg2),
                        c"terminal/pty job cannot be detached",
                    );
                    if !env.is_null() {
                        tv_dict_free(env);
                    }
                }
                drop(argv);
                channel_destroy_early(&mut *chan);
                *status_out = 0;
                return ptr::null_mut();
            }
            (*proc_ptr).r#type = kProcTypePty;
            (*proc_ptr).data = chan;
            {
                (*proc_ptr).loop_ = (&raw mut g_st().main_loop).cast();
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
                (*proc_ptr).loop_ = (&raw mut g_st().main_loop).cast();
            }
            (*proc_ptr).status = -1;
            (*proc_ptr).out.s.fd = STDOUT_FILENO;
            (*proc_ptr).err.s.fd = STDERR_FILENO;
        }

        let ph = proc_ptr;
        let pt = proc_ptr;
        (*ph).argv = argv;
        (*ph).exepath = if exepath.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(exepath))
        };
        (*pt).events = (*chan).events;
        (*pt).detach = detach;
        (*ph).cwd = if cwd.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(cwd))
        };
        (*ph).env = env;
        (*pt).overlapped = overlapped;
        #[cfg(windows)]
        {
            (*pt).stdio_noinherit = true;
        }

        let cmd = (*ph)
            .exepath
            .clone()
            .unwrap_or_else(|| (*ph).argv.as_slice()[0].clone());

        let (has_out, has_err) = if (*ph).r#type == kProcTypePty {
            (true, false)
        } else {
            let out = rpc || callback_reader_set(&(*chan).on_data);
            #[cfg(windows)]
            let mut err = callback_reader_set(&(*chan).on_stderr);
            #[cfg(not(windows))]
            let err = callback_reader_set(&(*chan).on_stderr);
            (*pt).fwd_err = (*chan).on_stderr.fwd_err;
            #[cfg(windows)]
            if !err && (*pt).fwd_err && (*pt).detach {
                err = true;
                (*pt).fwd_err = false;
            }
            (out, err)
        };

        let has_in = stdin_mode == kChannelStdinPipe;

        let status = proc_spawn(&mut *proc_ptr, has_in, has_out, has_err);
        if status != 0 {
            {
                semsg!(
                    gettext(e_jobspawn),
                    uv_strerror(status),
                    cmd.as_ptr(),
                );
                if !(*ph).env.is_null() {
                    tv_dict_free((*ph).env);
                }
            }
            channel_destroy_early(&mut *chan);
            *status_out = Varnumber::from((*ph).status);
            return ptr::null_mut();
        }

        drop(cmd);
        if !(*ph).env.is_null() {
            tv_dict_free((*ph).env);
        }

        if has_in {
            wstream_init(&mut (*proc_ptr).r#in, 0);
        }
        if has_out {
            rstream_init(&mut (*proc_ptr).out);
        }

        if rpc {
            rpc_start(&mut *chan);
        } else if has_out {
            callback_reader_start(&mut (*chan).on_data, c"stdout");
            rstream_start(&mut (*proc_ptr).out, StreamCbData::ChannelData(chan));
        }

        if has_err {
            callback_reader_start(&mut (*chan).on_stderr, c"stderr");
            rstream_init(&mut (*proc_ptr).err);
            rstream_start(&mut (*proc_ptr).err, StreamCbData::ChannelStderr(chan));
        }

        *status_out = (*chan).id as Varnumber;
        chan
    }
}

pub fn channel_connect(
    tcp: bool,
    address: impl crate::strings::IntoCCharPtr,
    rpc: bool,
    on_output: CallbackReader,
    timeout: c_int,
    error: *mut *const c_char,
) -> u64 { let address = address.into_c_char_ptr();
    unsafe {
        if !tcp && rpc && server_owns_pipe_address(address) {
            let channel = channel_alloc(kChannelStreamInternal);
            (*channel).internal.cb = LUA_NOREF;
            rpc_start(&mut *channel);
            channel_create_event(&mut *channel, address);
            return (*channel).id;
        }

        let channel = channel_alloc(kChannelStreamSocket);
        let socket_stream = &mut (*channel).socket;
        let connected = socket_connect(
            &mut g_st().main_loop,
            socket_stream,
            tcp,
            address,
            timeout,
            error,
        );

        if !connected {
            channel_decref( &mut *channel);
            return 0;
        }

        socket_stream.s.internal_data = StreamInternalData::Channel(channel);
        wstream_init(&mut socket_stream.s, 0);
        rstream_init(socket_stream);

        if rpc {
            rpc_start(&mut *channel);
        } else {
            (*channel).on_data = on_output;
            callback_reader_start(&mut (*channel).on_data, c"data");
            rstream_start(socket_stream, StreamCbData::ChannelData(channel));
        }

        channel_create_event(&mut *channel, address);
        (*channel).id
    }
}

/// Creates an RPC channel from a tcp/pipe socket connection.
pub fn channel_from_connection(watcher: &mut SocketWatcher) {
    unsafe {
        let channel = channel_alloc(kChannelStreamSocket);
        let socket_stream = &mut (*channel).socket;
        let result = socket_watcher_accept(watcher, socket_stream);
        if result != 0 {
            channel_destroy_early(&mut *channel);
            return;
        }
        socket_stream.s.internal_data = StreamInternalData::Channel(channel);
        wstream_init(&mut socket_stream.s, 0);
        rstream_init(socket_stream);
        rpc_start(&mut *channel);
        channel_create_event(&mut *channel, watcher.addr.as_ptr());
    }
}

/// Creates an API channel from stdin/stdout. Used when embedding Nvim.
pub fn channel_from_stdio(
    rpc: bool,
    on_output: CallbackReader,
    error: *mut *const c_char,
) -> u64 {
    unsafe {
        if !g_st().headless_mode && !g_st().embedded_mode {
            *error = gettext(c"can only be opened in headless mode");
            return 0;
        }

        if chan_st().DID_STDIO {
            {
                *error = gettext(c"channel was already open");
            }
            return 0;
        }
        chan_st().DID_STDIO = true;

        let channel = channel_alloc(kChannelStreamStdio);
        let mut stdin_dup_fd = STDIN_FILENO;
        let mut stdout_dup_fd = STDOUT_FILENO;

        #[cfg(windows)]
        if g_st().embedded_mode && os_has_conpty_working() {
            stdin_dup_fd = os_dup_cloexec(STDIN_FILENO);
            stdout_dup_fd = os_dup_cloexec(STDOUT_FILENO);
            os_attach_parent_console();
        }

        #[cfg(not(windows))]
        if g_st().embedded_mode {
            stdin_dup_fd = fcntl(STDIN_FILENO, F_DUPFD_CLOEXEC, STDERR_FILENO + 1);
            stdout_dup_fd = fcntl(STDOUT_FILENO, F_DUPFD_CLOEXEC, STDERR_FILENO + 1);
            let _ = dup2(STDERR_FILENO, STDOUT_FILENO);
            let _ = dup2(STDERR_FILENO, STDIN_FILENO);
        }

        let stdio_in = &mut (*channel).stdio_in;
        let stdio_out = &mut (*channel).stdio_out;
        {
            rstream_init_fd((&raw mut g_st().main_loop).cast(),  stdio_in, stdin_dup_fd);
            wstream_init_fd(&mut g_st().main_loop, stdio_out, stdout_dup_fd, 0);
        }

        if rpc {
            rpc_start(&mut *channel);
        } else {
            (*channel).on_data = on_output;
            callback_reader_start(&mut (*channel).on_data, c"stdin");
            rstream_start(stdio_in, StreamCbData::ChannelData(channel));
        }

        (*channel).id
    }
}

pub fn channel_send(id: u64, data: Vec<c_char>, error: *mut *const c_char) -> usize {
    unsafe {
        let chan = find_channel(id);
        let len = data.len();

        if chan.is_null() {
            *error = gettext(e_invchan);
            return 0;
        }

        if (*chan).streamtype == kChannelStreamStderr {
            if (*chan).err_closed {
                *error = gettext(c"Can't send data to closed stream");
                return 0;
            }
            let wres = os_write(STDERR_FILENO, data.as_ptr(), len, false);
            return if wres >= 0 { wres as usize } else { 0 };
        }

        if (*chan).streamtype == kChannelStreamInternal {
            if (*chan).is_rpc {
                *error = gettext(c"Can't send raw data to rpc channel");
                return 0;
            }
            let internal = &mut (*chan).internal;
            if (*chan).term.is_null() || internal.closed {
                *error = gettext(c"Can't send data to closed stream");
                return 0;
            }
            terminal_receive(&mut *(*chan).term, data.as_ptr(), len);
            return len;
        }

        let in_stream = channel_instream(&mut *chan);
        if (*in_stream).closed {
            *error = gettext(c"Can't send data to closed stream");
            return 0;
        }

        if (*chan).is_rpc {
            *error = gettext(c"Can't send raw data to rpc channel");
            return 0;
        }

        let buf = wstream_new_vec_buffer(data);
        if wstream_write(&mut *in_stream, buf) == 0 {
            len
        } else {
            0
        }
    }
}

#[inline]
fn buffer_to_tv_list(buf: impl crate::strings::IntoCCharPtr, len: usize) -> *mut TvList { let buf = buf.into_c_char_ptr();
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        tv_list_append_string( &mut *l, c"", 0);
        if len > 0 {
            encode_list_write(l.as_mut(), buf, len);
        }
        l
    }
}

pub fn on_channel_data(
    stream: &mut RStream,
    buf: impl crate::strings::IntoCCharPtr,
    count: usize,
    chan: &mut Channel,
    eof: bool,
) -> usize { let buf = buf.into_c_char_ptr();
    on_channel_output(stream, chan, buf, count, eof, false)
}

pub fn on_job_stderr(
    stream: &mut RStream,
    buf: impl crate::strings::IntoCCharPtr,
    count: usize,
    chan: &mut Channel,
    eof: bool,
) -> usize { let buf = buf.into_c_char_ptr();
    on_channel_output(stream, chan, buf, count, eof, true)
}

fn on_channel_output(
    _stream: &mut RStream,
    chan: &mut Channel,
    buf: impl crate::strings::IntoCCharPtr,
    count: usize,
    eof: bool,
    is_stderr: bool,
) -> usize { let buf = buf.into_c_char_ptr();
    unsafe {
        if !chan.term.is_null() {
            terminal_receive(&mut *chan.term, buf, count);
        }

        let reader = if is_stderr {
            &mut chan.on_stderr
        } else {
            &mut chan.on_data
        };

        if eof {
            reader.eof = true;
        }

        #[cfg(windows)]
        if reader.fwd_err && count > 0 {
            let wres = os_write(STDOUT_FILENO, buf, count, false);
            return wres.max(0) as usize;
        }

        if callback_reader_set(reader) {
            if !buf.is_null() && count > 0 {
                reader
                    .buffer
                    .extend_from_slice(std::slice::from_raw_parts(buf, count));
            }
            schedule_channel_event(chan);
        }

        count
    }
}

fn schedule_channel_event(chan: &mut Channel) {
    if !chan.callback_scheduled {
        if !chan.callback_busy {
            multiqueue_put_event(unsafe { &mut *chan.events }, Event::ChannelCallback(&raw mut *chan));
            channel_incref(chan);
        }
        chan.callback_scheduled = true;
    }
}

pub fn on_channel_event(chan: &mut Channel) {
     {
        chan.callback_busy = true;
        chan.callback_scheduled = false;

        let exit_status = chan.exit_status;
        channel_reader_callbacks(chan, false);
        channel_reader_callbacks(chan, true);
        if exit_status > -1 {
            channel_callback_call(chan, None);
            chan.exit_status = -1;
        }

        chan.callback_busy = false;
        if chan.callback_scheduled {
            multiqueue_put_event(unsafe { &mut *chan.events }, Event::ChannelCallback(&raw mut *chan));
            channel_incref(chan);
        }

        channel_decref(chan);
    }
}

pub fn channel_reader_callbacks(chan: &mut Channel, is_stderr: bool) {
    let reader = if is_stderr {
        &mut chan.on_stderr
    } else {
        &mut chan.on_data
    };
    if reader.buffered {
        if reader.eof {
            if !reader.self_dict.is_null() {
                let reader_type = reader.reader_type.unwrap();
                if tv_dict_find(unsafe { reader.self_dict.as_ref() }, reader_type.as_ptr(), -1).is_null() {
                    let data = buffer_to_tv_list(reader.buffer.as_ptr(), reader.buffer.len());
                    let _type_len = reader_type.to_bytes().len();
                    let _ = tv_dict_add_list(
                        unsafe { &mut *reader.self_dict },
                        reader_type,
                        data,
                    );
                } else {
                    semsg!(
                        gettext(e_streamkey),
                        reader_type.as_ptr(),
                        chan.id,
                    );
                }
                reader.eof = false;
            } else {
                reader.eof = false;
                channel_callback_call(chan, Some(is_stderr));
            }
        }
    } else {
        let is_eof = reader.eof;
        let not_empty = !reader.buffer.is_empty();
        if not_empty {
            channel_callback_call(chan, Some(is_stderr));
        }
        if is_eof {
            channel_callback_call(chan, Some(is_stderr));
            let reader = if is_stderr {
                &mut chan.on_stderr
            } else {
                &mut chan.on_data
            };
            reader.eof = false;
        }
    }
}

pub(crate) fn channel_proc_exit_cb(proc_: &mut Proc, status: c_int, chan: &mut Channel) {
     {

        if !chan.term.is_null() {
            terminal_close(&raw mut chan.term, status);
        }

        if !g_st().exiting && g_st().ui_client_channel_id == chan.id {
            ui_client_attach_to_restarted_server(proc_.status != 0);
            if g_st().ui_client_channel_id == chan.id {
                exit_on_closed_chan(status);
            }
        }

        let exited = status >= 0;
        if exited && chan.on_exit.cb_type() != kCallbackNone {
            schedule_channel_event(chan);
        }
        if exited {
            chan.exit_status = status;
        }

        channel_decref(chan);
    }
}

#[cfg(unix)]
pub(crate) fn channel_proc_state_cb(_proc: &mut Proc, suspended: bool, chan: &mut Channel) {
    unsafe {
        if !chan.term.is_null() {
            terminal_set_state(&mut *chan.term, suspended);
        }
    }
}

fn channel_callback_call(chan: &mut Channel, is_stderr: Option<bool>) {
    let mut argv: [TypVal; 4] = [TypVal::new(); 4];
    argv[0].v_type = VarType::VarNumber;
    argv[0].v_lock = VAR_UNLOCKED;
    argv[0].vval = TypvalVval::Number(chan.id as Varnumber);

    let cb: *mut Callback = if let Some(is_stderr) = is_stderr {
        let reader = if is_stderr {
            &mut chan.on_stderr
        } else {
            &mut chan.on_data
        };
        let l = buffer_to_tv_list(reader.buffer.as_ptr(), reader.buffer.len());
        argv[1].v_type = VarType::VarList;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval = TypvalVval::List(l);
        tv_list_ref(unsafe { l.as_mut() });
        reader.buffer = Vec::new();
        argv[2].vval = TypvalVval::String(
            reader
                .reader_type
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        );
        &raw mut reader.cb
    } else {
        argv[1].v_type = VarType::VarNumber;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval = TypvalVval::Number(Varnumber::from(chan.exit_status));
        argv[2].vval = TypvalVval::String(c"exit".as_ptr().cast_mut());
        &raw mut chan.on_exit
    };

    argv[2].v_type = VarType::VarString;
    argv[2].v_lock = VAR_UNLOCKED;

    let mut rettv: TypVal = TypVal::new();
    let _ = callback_call(unsafe { &mut *cb }, 3, argv.as_mut_ptr(), &mut rettv);
    tv_clear(&mut rettv);

    if is_stderr.is_some() {
        tv_list_unref(argv[1].vval.v_list());
    }
}

/// Allocate terminal for channel.
pub fn channel_terminal_alloc(buf: &mut FileBuf, chan: &mut Channel) {
     {
        let topts = TerminalOptions {
            data: chan,
            width: chan.uv.proc_.width,
            height: chan.uv.proc_.height,
            force_crlf: false,
        };
        buf.b_p_channel = chan.id as OptInt;
        channel_incref(&mut *chan);
        {
            chan.term = terminal_alloc(buf, topts);
        }
    }
}

pub fn channel_term_read_pause(pause: bool, chan: &mut Channel) {
     {
        if chan.streamtype == kChannelStreamInternal {
            return;
        }
        let out_stream = &mut chan.uv.proc_.out;
        if out_stream.s.closed {
            return;
        }
        if pause {
            rstream_stop_inner(out_stream);
        } else {
            rstream_start_inner(out_stream);
        }
    }
}

pub fn channel_term_write(buf: impl crate::strings::IntoCCharPtr, size: usize, chan: &mut Channel) { let buf = buf.into_c_char_ptr();
    unsafe {
        if chan.streamtype == kChannelStreamInternal {
            let internal = &mut chan.internal;
            let cb = internal.cb;
            if cb == LUA_NOREF {
                return;
            }
            let mut args_items = [
                crate::api::private::helpers::integer_obj(chan.id as Integer),
                crate::api::private::helpers::buffer_obj(
                    crate::terminal::terminal_buf(&*chan.term),
                ),
                crate::api::private::helpers::string_obj(NvimString {
                    data: buf.cast_mut(),
                    size,
                }),
            ];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 3,
                capacity: 3,
            };
            g_st().textlock += 1;
            crate::lua::executor::nlua_call_ref(
                cb,
                c"input",
                args,
                crate::lua::executor::kRetNilBool,
                None,
                None,
            );
            g_st().textlock -= 1;
            return;
        }
        let in_stream = &mut chan.uv.proc_.r#in;
        if in_stream.closed {
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

pub fn channel_term_resize(width: u16, height: u16, chan: &mut Channel) {
    if chan.streamtype == kChannelStreamInternal {
            return;
        }
        pty_proc_resize(&mut chan.uv.proc_, width, height);
}

pub fn channel_term_resume(chan: &mut Channel) {
    if chan.streamtype == kChannelStreamInternal {
            return;
        }
        pty_proc_resume(&mut chan.uv.proc_);
}

pub fn term_delayed_free(chan: &mut Channel) {
    unsafe {
        let proc_ptr = &mut chan.uv.proc_;
        if proc_ptr.r#in.pending_reqs != 0 || proc_ptr.out.s.pending_reqs != 0 {
            multiqueue_put_event( &mut *chan.events, Event::ChannelTermDelayedFree(&raw mut *chan));
            return;
        }

        if !chan.term.is_null() {
            terminal_destroy(&raw mut chan.term);
        }
        channel_decref(chan);
    }
}

pub fn channel_term_close(chan: &mut Channel) {
    unsafe {
        if chan.streamtype == kChannelStreamInternal {
            let internal = &mut chan.internal;
            terminal_destroy(&raw mut chan.term);
            crate::lua::executor::api_free_luaref(internal.cb);
            internal.cb = LUA_NOREF;
            channel_decref(chan);
            return;
        }
        proc_stop(&mut chan.uv.proc_);
        multiqueue_put_event( &mut *chan.events, Event::ChannelTermDelayedFree(&raw mut *chan));
    }
}

pub fn channel_event(chan: &mut Channel, event: AuEvent) {
    unsafe {
        if has_event(event) {
            channel_incref(chan);
            let qc = loop_queues(&mut g_st().main_loop);
            multiqueue_put_event( &mut *(*qc).events, Event::ChannelSetInfo { chan: &raw mut *chan, event });
        }
    }
}

pub fn set_info_event(chan: &mut Channel, event: AuEvent) {
    unsafe {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let info = channel_info(chan.id, &mut arena);
        let mut retval: TypVal = TypVal::new();
        object_to_vim(
            Object {
                obj_type: ObjectType::ObjectTypeDict,
                data: ObjectData::Dict(info),
            },
            &mut retval,
            &mut crate::api::private::validate::Error::new(),
        );
        assert_eq!(retval.v_type, VarType::VarDict);
        let _ = tv_dict_add_dict(&mut *dict, c"info", retval.vval.v_dict());
        tv_dict_set_keys_readonly(&mut *dict);

        let _ = apply_autocmds(event, ptr::null_mut(), ptr::null_mut(), true, g_st().curbuf);

        restore_v_event(&mut *dict, &mut save_v_event);
        arena_mem_free(arena_finish(&mut arena));
        channel_decref(chan);
    }
}

pub fn channel_job_running(id: u64) -> bool {
    unsafe {
        let chan = find_channel(id);
        if chan.is_null() {
            return false;
        }
        if (*chan).streamtype != kChannelStreamProc {
            return false;
        }
        let ph = &mut (*chan).uv.proc_;
        !(ph.status >= 0 || ph.stopped_time != 0)
    }
}

pub fn channel_info(id: u64, arena: &mut Arena) -> Dict {
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

        put_c(
            &mut info,
            c"id",
            Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer((*chan).id as Integer),
            },
        );

        let stream_desc: &'static CStr = match (*chan).streamtype {
            kChannelStreamProc => {
                let proc_ptr = &raw mut (*chan).uv.proc_;
                let ph = proc_ptr;
                if (*ph).r#type == kProcTypePty {
                    let name = pty_proc_tty_name(&mut *proc_ptr.cast());
                    let s = if name.is_null() {
                        NvimString {
                            data: c"".as_ptr().cast_mut(),
                            size: 0,
                        }
                    } else {
                        let len = crate::memory::cstr_len(name);
                        NvimString {
                            data: arena_memdupz(arena, name, len),
                            size: len,
                        }
                    };
                    put_c(
                        &mut info,
                        c"pty",
                        Object {
                            obj_type: ObjectType::ObjectTypeString,
                            data: ObjectData::String(s),
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
                    let n = args.len();
                    argv = arena_array(arena, n);
                    for arg in args {
                        add_c(
                            &mut argv,
                            Object {
                                obj_type: ObjectType::ObjectTypeString,
                                data: ObjectData::String(cstr_as_string(arg.as_ptr())),
                            },
                        );
                    }
                }
                put_c(
                    &mut info,
                    c"argv",
                    Object {
                        obj_type: ObjectType::ObjectTypeArray,
                        data: ObjectData::Array(argv),
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
                        obj_type: ObjectType::ObjectTypeBoolean,
                        data: ObjectData::Boolean(true),
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
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(stream_desc.as_ptr())),
            },
        );

        let mode_desc: &'static CStr = if (*chan).is_rpc {
            put_c(
                &mut info,
                c"client",
                Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict((*chan).rpc.info),
                },
            );
            put_c(
                &mut info,
                c"detach",
                Object {
                    obj_type: ObjectType::ObjectTypeBoolean,
                    data: ObjectData::Boolean((*chan).detach),
                },
            );
            c"rpc"
        } else if !(*chan).term.is_null() {
            let tbuf = terminal_buf(&*(*chan).term);
            put_c(
                &mut info,
                c"buf",
                Object {
                    obj_type: ObjectType::ObjectTypeBuffer,
                    data: ObjectData::Integer(i64::from(tbuf)),
                },
            );
            put_c(
                &mut info,
                c"buffer",
                Object {
                    obj_type: ObjectType::ObjectTypeBuffer,
                    data: ObjectData::Integer(i64::from(tbuf)),
                },
            );
            put_c(
                &mut info,
                c"exitcode",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(i64::from((*chan).exit_status)),
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
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(mode_desc.as_ptr())),
            },
        );

        info
    }
}

pub fn channel_all_info(arena: &mut Arena) -> Array {
     {
        let mut ret = arena_array(arena, g_st().channels.len());

        for &id in g_st().channels.keys() {
            let info = channel_info(id, arena);
            add_c(
                &mut ret,
                Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(info),
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
