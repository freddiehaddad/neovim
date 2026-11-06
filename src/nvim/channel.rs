//! Async I/O and process/socket/stdio/terminal channel management (`src/nvim/channel.c`).

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{arena_array, arena_dict, cstr_as_string, put_c};
use crate::api::private::validate::{Array, Dict, NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{AuEvent, apply_autocmds, has_event};
use crate::buffer::FileBuf;
use crate::errors::*;
use crate::eval::encode::encode_list_write;
use crate::eval::typval::{
    Callback, K_CALLBACK_NONE_CONST, TvDict, TvList, TypVal, TypvalVval, VAR_UNLOCKED, VarType,
    Varnumber, callback_free, tv_clear, tv_dict_add_dict, tv_dict_add_list, tv_dict_find,
    tv_dict_free, tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_string, tv_list_ref,
    tv_list_unref,
};
use crate::eval::{K_LIST_LEN_MAY_KNOW_CONST, callback_call, get_v_event, restore_v_event};
use crate::event::r#loop::{loop_queues, uv_strerror};
use crate::event::multiqueue::{Event, multiqueue_new_child, multiqueue_put_event};
use crate::event::proc::{exit_on_closed_chan, proc_spawn, proc_stop};
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
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::message::semsg;
pub use crate::msgpack_rpc::channel::Channel;
use crate::msgpack_rpc::channel::{
    ChannelPart, ChannelStreamType, RpcState, channel_instream_mut, find_channel, find_channel_opt,
    rpc_close, rpc_init, rpc_start,
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
#[cfg(unix)]
use crate::terminal::terminal_set_state;
use crate::terminal::{
    terminal_alloc, terminal_buf, terminal_close, terminal_destroy, terminal_receive,
};
use crate::ui_client::ui_client_attach_to_restarted_server;
use std::ffi::CStr;

pub type ChannelStdinMode = i32;

#[repr(C)]
pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: crate::globals::GraphPtr<TvDict>,
    pub buffer: Vec<u8>,
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
            self_dict: crate::globals::GraphPtr::NULL,
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

pub(crate) const STDIN_FILENO: i32 = 0;
pub(crate) const STDOUT_FILENO: i32 = 1;
pub(crate) const STDERR_FILENO: i32 = 2;

pub(crate) const K_PROC_TYPE_UV: i32 = 0;
pub(crate) const K_PROC_TYPE_PTY: i32 = 1;

pub(crate) const K_CHANNEL_STREAM_PROC: ChannelStreamType = 0;
pub(crate) const K_CHANNEL_STREAM_SOCKET: ChannelStreamType = 1;
pub(crate) const K_CHANNEL_STREAM_STDIO: ChannelStreamType = 2;
const K_CHANNEL_STREAM_STDERR: ChannelStreamType = 3;

pub(crate) const K_CHANNEL_PART_STDIN: ChannelPart = 0;
pub(crate) const K_CHANNEL_PART_STDOUT: ChannelPart = 1;
pub(crate) const K_CHANNEL_PART_STDERR: ChannelPart = 2;
pub(crate) const K_CHANNEL_PART_RPC_CONST: ChannelPart = 3;

pub(crate) const K_CHANNEL_STDIN_PIPE_CONST: ChannelStdinMode = 0;

#[cfg(target_os = "linux")]
const F_DUPFD_CLOEXEC: i32 = 1030;
#[cfg(target_os = "macos")]
const F_DUPFD_CLOEXEC: i32 = 67;
#[cfg(target_os = "freebsd")]
const F_DUPFD_CLOEXEC: i32 = 17;
#[cfg(all(
    not(windows),
    not(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))
))]
const F_DUPFD_CLOEXEC: i32 = 12;

struct ChannelModuleState {
    did_stdio: bool,
    next_chan_id: u64,
}

impl ChannelModuleState {
    const fn new() -> Self {
        Self {
            did_stdio: false,
            next_chan_id: CHAN_STDERR + 1,
        }
    }
}

static CHAN_STATE: crate::globals::GlobalState<ChannelModuleState> =
    crate::globals::GlobalState::new(ChannelModuleState::new());

#[inline]
fn chan_st() -> crate::globals::GraphPtr<ChannelModuleState> {
    CHAN_STATE.handle()
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
    pub data: crate::globals::GraphPtr<Channel>,
    pub width: u16,
    pub height: u16,
    pub force_crlf: bool,
}

use crate::api::ui::{CHAN_STDIO, K_CHANNEL_PART_ALL};
use crate::api::vim::K_CHANNEL_STREAM_INTERNAL;
use crate::autocmd::{EVENT_CHANCLOSE, EVENT_CHANOPEN};
#[cfg(not(windows))]
use crate::fileio::dup2;
use crate::lua::converter::LUA_NOREF;

use crate::fileio::freopen;

#[inline]
fn redirect_stderr_to_null() {
    #[cfg(windows)]
    {
        let _ = freopen(c"NUL:", c"w", &crate::log::log_get_stderr());
    }
    #[cfg(not(windows))]
    {
        let _ = freopen(c"/dev/null", c"w", &crate::log::log_get_stderr());
    }
}

#[inline]
fn callback_reader_set(reader: &CallbackReader) -> bool {
    reader.cb.cb_type() != K_CALLBACK_NONE_CONST || !reader.self_dict.is_null()
}

use crate::api::private::helpers::add_c;

/// Teardown the module.
pub fn channel_teardown() {
    let all_chans: Vec<crate::globals::GraphPtr<Channel>> =
        g_st().channels.values().copied().collect();
    for chan in all_chans {
        channel_close(chan.id, K_CHANNEL_PART_ALL, None);
    }
}

pub fn channel_free_all_mem() {
    for (_, chan) in std::mem::take(&mut g_st().channels) {
        crate::globals::GraphPtr::of(chan.as_ptr()).drop_box();
    }

    callback_free(&mut g_st().on_print);
}

/// Closes a channel.
pub fn channel_close(
    id: u64,
    part: ChannelPart,
    error: Option<&mut Option<&'static CStr>>,
) -> bool {
    let Some(chan) = find_channel_opt(id) else {
        if id < chan_st().next_chan_id {
            return true;
        }
        if let Some(err) = error {
            *err = Some(E_INVCHAN);
        }
        return false;
    };

    let mut close_main = false;
    if part == K_CHANNEL_PART_RPC_CONST || part == K_CHANNEL_PART_ALL {
        close_main = true;
        if chan.is_rpc {
            rpc_close(&mut *chan);
        } else if part == K_CHANNEL_PART_RPC_CONST {
            if let Some(err) = error {
                *err = Some(E_INVSTREAM);
            }
            return false;
        }
    } else if (part == K_CHANNEL_PART_STDIN || part == K_CHANNEL_PART_STDOUT) && chan.is_rpc {
        if let Some(err) = error {
            *err = Some(E_INVSTREAMRPC);
        }
        return false;
    }

    match chan.streamtype {
        K_CHANNEL_STREAM_SOCKET => {
            if !close_main {
                if let Some(err) = error {
                    *err = Some(E_INVSTREAM);
                }
                return false;
            }
            rstream_may_close(&mut chan.socket);
        }
        K_CHANNEL_STREAM_PROC => {
            let proc_ptr = &mut chan.uv.proc_;
            if part == K_CHANNEL_PART_STDIN || close_main {
                stream_may_close(&mut proc_ptr.r#in);
            }
            if part == K_CHANNEL_PART_STDOUT || close_main {
                rstream_may_close(&mut proc_ptr.out);
            }
            if part == K_CHANNEL_PART_STDERR || part == K_CHANNEL_PART_ALL {
                rstream_may_close(&mut proc_ptr.err);
            }
            if proc_ptr.r#type == K_PROC_TYPE_PTY && part == K_CHANNEL_PART_ALL {
                pty_proc_close_master(proc_ptr);
            }
        }
        K_CHANNEL_STREAM_STDIO => {
            if part == K_CHANNEL_PART_STDIN || close_main {
                rstream_may_close(&mut chan.stdio_in);
            }
            if part == K_CHANNEL_PART_STDOUT || close_main {
                stream_may_close(&mut chan.stdio_out);
            }
            if part == K_CHANNEL_PART_STDERR {
                if let Some(err) = error {
                    *err = Some(E_INVSTREAM);
                }
                return false;
            }
        }
        K_CHANNEL_STREAM_STDERR => {
            if part != K_CHANNEL_PART_ALL && part != K_CHANNEL_PART_STDERR {
                if let Some(err) = error {
                    *err = Some(E_INVSTREAM);
                }
                return false;
            }
            if !chan.err_closed {
                chan.err_closed = true;
                if !g_st().exiting {
                    redirect_stderr_to_null();
                }
                channel_decref(&mut *chan);
            }
        }
        K_CHANNEL_STREAM_INTERNAL => {
            if !close_main {
                if let Some(err) = error {
                    *err = Some(E_INVSTREAM);
                }
                return false;
            }
            if !chan.term.is_null() {
                let internal = &mut chan.internal;
                api_free_luaref(internal.cb);
                internal.cb = LUA_NOREF;
                internal.closed = true;
                terminal_close(chan.term.borrow_mut(), 0);
                chan.exit_status = 0;
            } else {
                channel_decref(&mut *chan);
            }
        }
        _ => {}
    }

    true
}

/// Initializes the module.
pub fn channel_init() {
    let _ = channel_alloc(K_CHANNEL_STREAM_STDERR);
    rpc_init();
}

/// Allocates a channel.
pub fn channel_alloc(r#type: ChannelStreamType) -> &'static mut Channel {
    let id = if r#type == K_CHANNEL_STREAM_STDIO {
        CHAN_STDIO
    } else if r#type == K_CHANNEL_STREAM_STDERR {
        CHAN_STDERR
    } else {
        let id = chan_st().next_chan_id;
        chan_st().next_chan_id += 1;
        id
    };
    assert!(id <= (i64::MAX as u64));

    let qc = loop_queues(&mut g_st().main_loop);
    let chan = Box::into_raw(Box::new(Channel {
        id,
        refcount: 1,
        events: crate::globals::GraphPtr::from(multiqueue_new_child(qc.events.borrow_mut())),
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
        detach: r#type != K_CHANNEL_STREAM_STDIO,
        rpc: RpcState::new(),
        term: crate::globals::GraphPtr::NULL,
        on_data: CallbackReader::none(),
        on_stderr: CallbackReader::none(),
        on_exit: Callback::NONE,
        exit_status: -1,
        callback_busy: false,
        callback_scheduled: false,
        did_close_event: false,
    }));

    g_st().channels.insert(id, chan.into());

    crate::globals::GraphPtr::of(chan).borrow_mut()
}

pub fn channel_create_event(chan: &mut Channel) {
    channel_event(chan, EVENT_CHANOPEN);
}

pub fn channel_incref(chan: &mut Channel) {
    chan.refcount += 1;
}

pub fn channel_decref(chan: &mut Channel) {
    if chan.refcount == 1 && !chan.did_close_event {
        chan.did_close_event = true;
        channel_event(chan, EVENT_CHANCLOSE);
    }

    chan.refcount -= 1;
    if chan.refcount == 0 {
        let qc = loop_queues(&mut g_st().main_loop);
        multiqueue_put_event(qc.events.borrow_mut(), Event::ChannelFree(chan.into()));
    }
}

pub fn callback_reader_start(reader: &mut CallbackReader, r#type: &'static CStr) {
    reader.reader_type = Some(r#type);
}

pub fn free_channel_event(chan: Option<&mut Channel>) {
    let chan = chan.unwrap();
    let _ = g_st().channels.remove(&chan.id);

    crate::globals::GraphPtr::of(chan).drop_box();
}

fn channel_destroy_early(chan: &mut Channel) {
    chan_st().next_chan_id -= 1;
    if chan.id != chan_st().next_chan_id {
        std::process::abort();
    }
    let _ = g_st().channels.remove(&chan.id);
    chan.id = 0;

    chan.refcount -= 1;
    if chan.refcount != 0 {
        std::process::abort();
    }

    let qc = loop_queues(&mut g_st().main_loop);
    multiqueue_put_event(qc.events.borrow_mut(), Event::ChannelFree(chan.into()));
}

/// Starts a job and returns the associated channel.
pub fn channel_job_start(
    argv: Vec<std::ffi::CString>,
    exepath: Option<&CStr>,
    on_stdout: CallbackReader,
    on_stderr: CallbackReader,
    on_exit: Callback,
    pty: bool,
    rpc: bool,
    overlapped: bool,
    detach: bool,
    stdin_mode: ChannelStdinMode,
    cwd: Option<&CStr>,
    pty_width: u16,
    pty_height: u16,
    env: Option<&'static mut TvDict>,
    status_out: &mut Varnumber,
) -> Option<&'static mut Channel> {
    let env_ptr = crate::globals::GraphPtr::from(env);
    let chan = channel_alloc(K_CHANNEL_STREAM_PROC);
    let chan_ptr = crate::globals::GraphPtr::<Channel>::from_ref(chan);
    chan.on_data = on_stdout;
    chan.on_stderr = on_stderr;
    chan.on_exit = on_exit;

    let proc_ptr = &raw mut chan.uv.proc_;
    if pty {
        if detach {
            semsg!(gettext(E_INVARG2), c"terminal/pty job cannot be detached",);
            if let Some(env_ref) = env_ptr.opt_mut() {
                tv_dict_free(env_ref);
            }
            drop(argv);
            channel_destroy_early(&mut *chan);

            *status_out = 0;

            return None;
        }
        chan.uv.proc_.r#type = K_PROC_TYPE_PTY;
        chan.uv.proc_.data = chan_ptr;
        chan.uv.proc_.loop_ = crate::globals::GraphPtr::new((&raw mut g_st().main_loop).cast());
        chan.uv.proc_.status = -1;
        chan.uv.proc_.out.s.fd = STDOUT_FILENO;
        chan.uv.proc_.err.s.fd = STDERR_FILENO;
        chan.uv.proc_.width = if pty_width > 0 { pty_width } else { 80 };
        chan.uv.proc_.height = if pty_height > 0 { pty_height } else { 24 };
        #[cfg(not(windows))]
        {
            chan.uv.proc_.tty_fd = -1;
        }
    } else {
        chan.uv.proc_.r#type = K_PROC_TYPE_UV;
        chan.uv.proc_.data = chan_ptr;
        chan.uv.proc_.loop_ = crate::globals::GraphPtr::new((&raw mut g_st().main_loop).cast());
        chan.uv.proc_.status = -1;
        chan.uv.proc_.out.s.fd = STDOUT_FILENO;
        chan.uv.proc_.err.s.fd = STDERR_FILENO;
    }

    let ph = crate::globals::GraphPtr::of(proc_ptr).borrow_mut();

    ph.argv = argv;
    ph.exepath = exepath.map(CStr::to_owned);
    ph.events = chan.events;
    ph.detach = detach;
    ph.cwd = cwd.map(CStr::to_owned);
    ph.env = env_ptr;
    ph.overlapped = overlapped;

    #[cfg(windows)]
    {
        ph.stdio_noinherit = true;
    }

    let cmd = ph
        .exepath
        .clone()
        .unwrap_or_else(|| ph.argv.as_slice()[0].clone());

    let (has_out, has_err) = if ph.r#type == K_PROC_TYPE_PTY {
        (true, false)
    } else {
        let out = rpc || callback_reader_set(&chan.on_data);
        #[cfg(windows)]
        let mut err = callback_reader_set(&chan.on_stderr);
        #[cfg(not(windows))]
        let err = callback_reader_set(&chan.on_stderr);

        ph.fwd_err = chan.on_stderr.fwd_err;

        #[cfg(windows)]
        if !err && ph.fwd_err && ph.detach {
            err = true;
            ph.fwd_err = false;
        }
        (out, err)
    };

    let has_in = stdin_mode == K_CHANNEL_STDIN_PIPE_CONST;

    let status = proc_spawn(
        crate::globals::GraphPtr::of(proc_ptr).borrow_mut(),
        has_in,
        has_out,
        has_err,
    );
    if status != 0 {
        semsg!(gettext(E_JOBSPAWN), uv_strerror(status), &cmd,);

        let ph = crate::globals::GraphPtr::of(proc_ptr).borrow_mut();
        if let Some(env_ref) = ph.env.opt_mut() {
            tv_dict_free(env_ref);
        }
        let proc_status = ph.status;
        channel_destroy_early(&mut *chan);

        *status_out = Varnumber::from(proc_status);

        return None;
    }

    drop(cmd);
    let ph = crate::globals::GraphPtr::of(proc_ptr).borrow_mut();
    if let Some(env_ref) = ph.env.opt_mut() {
        tv_dict_free(env_ref);
    }

    if has_in {
        wstream_init(&mut chan.uv.proc_.r#in, 0);
    }
    if has_out {
        rstream_init(&mut chan.uv.proc_.out);
    }

    if rpc {
        rpc_start(&mut *chan);
    } else if has_out {
        callback_reader_start(&mut chan.on_data, c"stdout");

        rstream_start(
            &mut crate::globals::GraphPtr::of(proc_ptr).borrow_mut().out,
            StreamCbData::ChannelData(chan_ptr),
        );
    }

    if has_err {
        callback_reader_start(&mut chan.on_stderr, c"stderr");
        rstream_init(&mut chan.uv.proc_.err);

        rstream_start(
            &mut crate::globals::GraphPtr::of(proc_ptr).borrow_mut().err,
            StreamCbData::ChannelStderr(chan_ptr),
        );
    }

    *status_out = chan.id as Varnumber;

    Some(chan)
}

pub fn channel_connect(
    tcp: bool,
    address: &CStr,
    rpc: bool,
    trust: bool,
    on_output: CallbackReader,
    timeout: i32,
    error: &mut Option<&'static CStr>,
) -> u64 {
    if !tcp && rpc && server_owns_pipe_address(address) {
        let channel = channel_alloc(K_CHANNEL_STREAM_INTERNAL);
        channel.internal.cb = LUA_NOREF;
        rpc_start(&mut *channel);
        channel.rpc.trust = trust;
        channel_create_event(&mut *channel);
        return channel.id;
    }

    let channel = channel_alloc(K_CHANNEL_STREAM_SOCKET);
    let channel_ptr = crate::globals::GraphPtr::<Channel>::from_ref(channel);
    let connected = socket_connect(
        &mut g_st().main_loop,
        &mut channel.socket,
        tcp,
        address,
        timeout,
        error,
    );

    if !connected {
        channel_decref(&mut *channel);
        return 0;
    }

    channel.socket.s.internal_data = StreamInternalData::Channel(channel_ptr);
    wstream_init(&mut channel.socket.s, 0);
    rstream_init(&mut channel.socket);

    if rpc {
        rpc_start(&mut *channel);
    } else {
        channel.on_data = on_output;
        callback_reader_start(&mut channel.on_data, c"data");
        rstream_start(&mut channel.socket, StreamCbData::ChannelData(channel_ptr));
    }

    channel.rpc.trust = trust;
    channel_create_event(&mut *channel);
    channel.id
}

/// Creates an RPC channel from a tcp/pipe socket connection.
pub fn channel_from_connection(watcher: &mut SocketWatcher) {
    let channel = channel_alloc(K_CHANNEL_STREAM_SOCKET);
    let channel_ptr = crate::globals::GraphPtr::<Channel>::from_ref(channel);
    let result = socket_watcher_accept(watcher, &mut channel.socket);
    if result != 0 {
        channel_destroy_early(&mut *channel);
        return;
    }
    channel.socket.s.internal_data = StreamInternalData::Channel(channel_ptr);
    wstream_init(&mut channel.socket.s, 0);
    rstream_init(&mut channel.socket);
    rpc_start(&mut *channel);
    channel_create_event(&mut *channel);
}

/// Creates an API channel from stdin/stdout. Used when embedding Nvim.
pub fn channel_from_stdio(
    rpc: bool,
    on_output: CallbackReader,
    error: &mut Option<&'static CStr>,
) -> u64 {
    if !g_st().headless_mode && !g_st().embedded_mode {
        *error = Some(gettext(c"can only be opened in headless mode"));
        return 0;
    }

    if chan_st().did_stdio {
        *error = Some(gettext(c"channel was already open"));
        return 0;
    }
    chan_st().did_stdio = true;

    let channel = channel_alloc(K_CHANNEL_STREAM_STDIO);
    let channel_ptr = crate::globals::GraphPtr::<Channel>::from_ref(channel);
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
        stdin_dup_fd = unsafe { fcntl(STDIN_FILENO, F_DUPFD_CLOEXEC, STDERR_FILENO + 1) };
        stdout_dup_fd = unsafe { fcntl(STDOUT_FILENO, F_DUPFD_CLOEXEC, STDERR_FILENO + 1) };
        let _ = unsafe { dup2(STDERR_FILENO, STDOUT_FILENO) };
        let _ = unsafe { dup2(STDERR_FILENO, STDIN_FILENO) };
    }

    rstream_init_fd(&mut g_st().main_loop, &mut channel.stdio_in, stdin_dup_fd);
    wstream_init_fd(
        &mut g_st().main_loop,
        &mut channel.stdio_out,
        stdout_dup_fd,
        0,
    );

    if rpc {
        rpc_start(&mut *channel);
    } else {
        channel.on_data = on_output;
        callback_reader_start(&mut channel.on_data, c"stdin");
        rstream_start(
            &mut channel.stdio_in,
            StreamCbData::ChannelData(channel_ptr),
        );
    }

    channel.id
}

pub fn channel_send(id: u64, data: Vec<u8>, error: &mut Option<&'static CStr>) -> usize {
    let len = data.len();
    let Some(chan) = find_channel_opt(id) else {
        *error = Some(gettext(E_INVCHAN));
        return 0;
    };

    if chan.streamtype == K_CHANNEL_STREAM_STDERR {
        if chan.err_closed {
            *error = Some(gettext(c"Can't send data to closed stream"));
            return 0;
        }
        let wres = os_write(STDERR_FILENO, &data, false);
        return if wres >= 0 { wres as usize } else { 0 };
    }

    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        if chan.is_rpc {
            *error = Some(gettext(c"Can't send raw data to rpc channel"));
            return 0;
        }
        if chan.term.is_null() || chan.internal.closed {
            *error = Some(gettext(c"Can't send data to closed stream"));
            return 0;
        }
        terminal_receive(chan.term_mut(), &data);
        return len;
    }

    let in_stream = channel_instream_mut(&mut *chan);
    if in_stream.closed {
        *error = Some(gettext(c"Can't send data to closed stream"));
        return 0;
    }

    if chan.is_rpc {
        *error = Some(gettext(c"Can't send raw data to rpc channel"));
        return 0;
    }

    let buf = wstream_new_vec_buffer(data);
    if wstream_write(in_stream, buf) == 0 {
        len
    } else {
        0
    }
}

#[inline]
fn buffer_to_tv_list(buf: &[u8]) -> &'static mut TvList {
    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    tv_list_append_string(&mut *l, Some(c""), 0);
    if !buf.is_empty() {
        encode_list_write(Some(&mut *l), buf);
    }
    l
}

pub fn on_channel_data(buf: &[u8], chan: &mut Channel, eof: bool) -> usize {
    on_channel_output(chan, buf, eof, false)
}

pub fn on_job_stderr(buf: &[u8], chan: &mut Channel, eof: bool) -> usize {
    on_channel_output(chan, buf, eof, true)
}

fn on_channel_output(chan: &mut Channel, buf: &[u8], eof: bool, is_stderr: bool) -> usize {
    let count = buf.len();
    if !chan.term.is_null() {
        terminal_receive(chan.term_mut(), buf);
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
        let wres = os_write(STDOUT_FILENO, &buf[..count], false);
        return wres.max(0) as usize;
    }

    if callback_reader_set(reader) {
        if count > 0 {
            reader.buffer.extend_from_slice(buf);
        }
        schedule_channel_event(chan);
    }

    count
}

fn schedule_channel_event(chan: &mut Channel) {
    if !chan.callback_scheduled {
        if !chan.callback_busy {
            multiqueue_put_event(chan.events_mut(), Event::ChannelCallback(chan.into()));
            channel_incref(chan);
        }
        chan.callback_scheduled = true;
    }
}

pub fn on_channel_event(chan: &mut Channel) {
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
        multiqueue_put_event(
            chan.events.borrow_mut(),
            Event::ChannelCallback(chan.into()),
        );
        channel_incref(chan);
    }

    channel_decref(chan);
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
                if tv_dict_find(reader.self_dict.opt(), reader_type, -1).is_none() {
                    let data = buffer_to_tv_list(&reader.buffer);
                    let _type_len = reader_type.to_bytes().len();
                    let _ =
                        tv_dict_add_list(reader.self_dict.borrow_mut(), reader_type, Some(data));
                } else {
                    semsg!(gettext(E_STREAMKEY), reader_type, chan.id,);
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

pub(crate) fn channel_proc_exit_cb(status: i32, chan: &mut Channel) {
    if !chan.term.is_null() {
        terminal_close(chan.term.borrow_mut(), status);
    }

    if !g_st().exiting && g_st().ui_client_channel_id == chan.id {
        ui_client_attach_to_restarted_server(status != 0);
        if g_st().ui_client_channel_id == chan.id {
            exit_on_closed_chan(status);
        }
    }

    let exited = status >= 0;
    if exited && chan.on_exit.cb_type() != K_CALLBACK_NONE_CONST {
        schedule_channel_event(chan);
    }
    if exited {
        chan.exit_status = status;
    }

    channel_decref(chan);
}

#[cfg(unix)]
pub(crate) fn channel_proc_state_cb(
    _proc: &mut crate::event::proc::Proc,
    suspended: bool,
    chan: &mut Channel,
) {
    if let Some(term) = chan.term.opt_mut() {
        terminal_set_state(term, suspended);
    }
}

fn channel_callback_call(chan: &mut Channel, is_stderr: Option<bool>) {
    let mut argv: [TypVal; 4] = [TypVal::new(); 4];
    argv[0].v_type = VarType::VarNumber;
    argv[0].v_lock = VAR_UNLOCKED;
    argv[0].vval = TypvalVval::Number(chan.id as Varnumber);

    let cb = crate::globals::GraphPtr::<Callback>::of(if let Some(is_stderr) = is_stderr {
        let reader = if is_stderr {
            &mut chan.on_stderr
        } else {
            &mut chan.on_data
        };
        let l = buffer_to_tv_list(&reader.buffer);
        argv[1].v_type = VarType::VarList;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval = TypvalVval::List(crate::globals::GraphPtr::new(l));
        tv_list_ref(Some(&mut *l));
        reader.buffer = Vec::new();
        argv[2].vval = TypvalVval::from_opt_cstr(reader.reader_type);
        &raw mut reader.cb
    } else {
        argv[1].v_type = VarType::VarNumber;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval = TypvalVval::Number(Varnumber::from(chan.exit_status));
        argv[2].vval = TypvalVval::from_cstr(c"exit");
        &raw mut chan.on_exit
    })
    .as_ptr();

    argv[2].v_type = VarType::VarString;
    argv[2].v_lock = VAR_UNLOCKED;

    let mut rettv: TypVal = TypVal::new();
    let _ = callback_call(
        crate::globals::GraphPtr::of(cb).borrow_mut(),
        3,
        &mut argv,
        &mut rettv,
    );
    tv_clear(&mut rettv);

    if is_stderr.is_some() {
        tv_list_unref(argv[1].vval.v_list_opt());
    }
}

/// Allocate terminal for channel.
pub fn channel_terminal_alloc(buf: &mut FileBuf, chan: &mut Channel) {
    let topts = TerminalOptions {
        data: crate::globals::GraphPtr::from(&mut *chan),
        width: chan.uv.proc_.width,
        height: chan.uv.proc_.height,
        force_crlf: false,
    };
    buf.b_p_channel = chan.id as OptInt;
    channel_incref(&mut *chan);
    {
        chan.term = crate::globals::GraphPtr::from(terminal_alloc(buf, topts));
    }
}

pub fn channel_term_read_pause(pause: bool, chan: &mut Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
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

pub fn channel_term_write(buf: &[u8], chan: &mut Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        let internal = &mut chan.internal;
        let cb = internal.cb;
        if cb == LUA_NOREF {
            return;
        }
        let mut args_items = [
            crate::api::private::helpers::integer_obj(chan.id as Integer),
            crate::api::private::helpers::buffer_obj(crate::terminal::terminal_buf(
                chan.term.borrow(),
            )),
            crate::api::private::helpers::string_obj(NvimString::from_bytes(buf)),
        ];
        let args = Array {
            items: crate::globals::GraphPtr::new(args_items.as_mut_ptr()),
            size: 3,
            capacity: 3,
        };
        g_st().textlock += 1;
        crate::lua::executor::nlua_call_ref(
            cb,
            Some(c"input"),
            args,
            crate::lua::executor::K_RET_NIL_BOOL,
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
    let wbuf = wstream_new_vec_buffer(buf.to_vec());
    let _ = wstream_write(in_stream, wbuf);
}

pub fn channel_term_resize(width: u16, height: u16, chan: &mut Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        return;
    }
    pty_proc_resize(&mut chan.uv.proc_, width, height);
}

pub fn channel_term_resume(chan: &mut Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        return;
    }
    pty_proc_resume(&mut chan.uv.proc_);
}

pub fn term_delayed_free(chan: &mut Channel) {
    let proc_ptr = &mut chan.uv.proc_;
    if proc_ptr.r#in.pending_reqs != 0 || proc_ptr.out.s.pending_reqs != 0 {
        multiqueue_put_event(
            chan.events.borrow_mut(),
            Event::ChannelTermDelayedFree(chan.into()),
        );
        return;
    }

    if !chan.term.is_null() {
        terminal_destroy(chan);
    }
    channel_decref(chan);
}

pub fn channel_term_close(chan: &mut Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        let cb = chan.internal.cb;
        chan.internal.cb = LUA_NOREF;
        terminal_destroy(chan);
        crate::lua::executor::api_free_luaref(cb);
        channel_decref(chan);
        return;
    }
    proc_stop(&mut chan.uv.proc_);
    multiqueue_put_event(
        chan.events.borrow_mut(),
        Event::ChannelTermDelayedFree(chan.into()),
    );
}

pub fn channel_event(chan: &mut Channel, event: AuEvent) {
    if has_event(event) {
        channel_incref(chan);
        let qc = loop_queues(&mut g_st().main_loop);
        multiqueue_put_event(
            qc.events.borrow_mut(),
            Event::ChannelSetInfo {
                chan: chan.into(),
                event,
            },
        );
    }
}

pub fn set_info_event(chan: &mut Channel, event: AuEvent) {
    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let dict = get_v_event(&mut save_v_event);
    let mut arena = Arena::new();
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
    let _ = tv_dict_add_dict(&mut *dict, c"info", retval.vval.v_dict_opt());
    tv_dict_set_keys_readonly(&mut *dict);

    let _ = apply_autocmds(event, None, None, true, curbuf_opt_mut());

    restore_v_event(&mut *dict, &mut save_v_event);
    arena_mem_free(arena_finish(&mut arena));
    channel_decref(chan);
}

pub fn channel_job_running(id: u64) -> bool {
    let Some(chan) = find_channel(id) else {
        return false;
    };
    if chan.streamtype != K_CHANNEL_STREAM_PROC {
        return false;
    }
    let ph = &mut chan.uv.proc_;
    !(ph.status >= 0 || ph.stopped_time != 0)
}

pub fn channel_info(id: u64, arena: &mut Arena) -> Dict {
    let Some(chan) = find_channel(id) else {
        return Dict {
            items: crate::globals::GraphPtr::NULL,
            size: 0,
            capacity: 0,
        };
    };

    let mut info = arena_dict(arena, 9);

    put_c(
        &mut info,
        c"id",
        Object {
            obj_type: ObjectType::ObjectTypeInteger,
            data: ObjectData::Integer(chan.id as Integer),
        },
    );

    let stream_desc: &'static CStr = match chan.streamtype {
        K_CHANNEL_STREAM_PROC => {
            let ph = &mut chan.uv.proc_;
            if ph.r#type == K_PROC_TYPE_PTY {
                let s = match pty_proc_tty_name(ph) {
                    Some(name) => crate::api::private::helpers::arena_bytes(arena, name.to_bytes()),
                    None => NvimString::from_bytes(b""),
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

            let args = ph.argv.as_slice();
            let mut argv = Array {
                items: crate::globals::GraphPtr::NULL,
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
                            data: ObjectData::String(cstr_as_string(Some(arg.as_c_str()))),
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
        K_CHANNEL_STREAM_STDIO => c"stdio",
        K_CHANNEL_STREAM_STDERR => c"stderr",
        K_CHANNEL_STREAM_INTERNAL => {
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
            data: ObjectData::String(cstr_as_string(Some(stream_desc))),
        },
    );

    let mode_desc: &'static CStr = if chan.is_rpc {
        put_c(
            &mut info,
            c"client",
            Object {
                obj_type: ObjectType::ObjectTypeDict,
                data: ObjectData::Dict(chan.rpc.info),
            },
        );

        put_c(
            &mut info,
            c"detach",
            Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(chan.detach),
            },
        );

        put_c(
            &mut info,
            c"trust",
            Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(chan.rpc.trust),
            },
        );

        c"rpc"
    } else if !chan.term.is_null() {
        let tbuf = terminal_buf(chan.term.borrow());
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
                data: ObjectData::Integer(i64::from(chan.exit_status)),
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
            data: ObjectData::String(cstr_as_string(Some(mode_desc))),
        },
    );

    info
}

pub fn channel_all_info(arena: &mut Arena) -> Array {
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
