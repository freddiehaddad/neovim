#![forbid(unsafe_code)]

//! Async I/O and process/socket/stdio/terminal channel management (`src/nvim/channel.c`).

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{cstr_as_string, put_c};
use crate::api::private::validate::{Array, Dict, NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{AuEvent, apply_autocmds, has_event};
use crate::buffer::FileBuf;
use crate::errors::*;
use crate::eval::encode::encode_list_write;
use crate::eval::typval::{
    Callback, K_CALLBACK_NONE_CONST, ListId, TypVal, TypvalVval, VAR_UNLOCKED, VarType, Varnumber,
    callback_free, tv_clear, tv_dict_add_dict, tv_dict_add_list, tv_dict_find, tv_dict_free,
    tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_string, tv_list_ref, tv_list_unref,
};
use crate::eval::{K_LIST_LEN_MAY_KNOW_CONST, callback_call, get_v_event, restore_v_event};
use crate::event::r#loop::{loop_queues, uv_strerror};
use crate::event::multiqueue::{Event, multiqueue_new_child, multiqueue_put_event};
use crate::event::proc::{exit_on_closed_chan, proc_spawn, proc_stop};
use crate::event::rstream::{
    rstream_init, rstream_init_fd, rstream_may_close, rstream_start, rstream_start_inner,
    rstream_stop_inner,
};
use crate::event::socket::{SocketHandle, socket_connect, socket_watcher_accept};
use crate::event::stream::{StreamCbData, StreamInternalData, stream_may_close};
use crate::event::wstream::{wstream_init, wstream_init_fd, wstream_new_vec_buffer, wstream_write};
#[cfg(not(windows))]
use crate::fileio::{dup2, os_dupfd_cloexec};
use crate::globals::*;
use crate::lua::converter::LuaRef;
use crate::lua::executor::api_free_luaref;
use crate::message::semsg;
pub use crate::msgpack_rpc::channel::Channel;
use crate::msgpack_rpc::channel::{
    ChannelPart, ChannelStreamType, channel_instream, find_channel, find_channel_opt, rpc_close,
    rpc_init, rpc_start,
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

pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: crate::eval::typval::DictId,
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
            self_dict: crate::eval::typval::DictId::NULL,
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
        callback_free(&self.cb);
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

struct ChannelModuleState {
    did_stdio: std::cell::Cell<bool>,
    next_chan_id: std::cell::Cell<u64>,
}

impl ChannelModuleState {
    const fn new() -> Self {
        Self {
            did_stdio: std::cell::Cell::new(false),
            next_chan_id: std::cell::Cell::new(CHAN_STDERR + 1),
        }
    }
}

std::thread_local! {
    static CHAN_STATE: ChannelModuleState = const { ChannelModuleState::new() };
}

#[inline]
fn chan_st<R>(f: impl FnOnce(&ChannelModuleState) -> R) -> R {
    CHAN_STATE.with(f)
}

#[derive(Clone, Copy)]
pub struct InternalState {
    pub cb: LuaRef,
    pub closed: bool,
}

#[derive(Clone, Copy)]
pub struct TerminalOptions {
    pub chan_id: u64,
    pub width: u16,
    pub height: u16,
    pub force_crlf: bool,
}

use crate::api::ui::{CHAN_STDIO, K_CHANNEL_PART_ALL};
use crate::api::vim::K_CHANNEL_STREAM_INTERNAL;
use crate::autocmd::{EVENT_CHANCLOSE, EVENT_CHANOPEN};
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
    let all_chans: Vec<u64> = crate::event::r#loop::main_loop()
        .channels()
        .iter()
        .map(|channel| channel.id)
        .collect();
    for id in all_chans {
        channel_close(id, K_CHANNEL_PART_ALL, None);
    }
}

pub fn channel_free_all_mem() {
    crate::event::r#loop::main_loop().clear_channels();

    callback_free(&msg_state().on_print);
}

/// Closes a channel.
pub fn channel_close(
    id: u64,
    part: ChannelPart,
    error: Option<&mut Option<&'static CStr>>,
) -> bool {
    let Some(chan) = find_channel_opt(id) else {
        if id < chan_st(|st| st.next_chan_id.get()) {
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
        if chan.is_rpc.get() {
            rpc_close(&chan);
        } else if part == K_CHANNEL_PART_RPC_CONST {
            if let Some(err) = error {
                *err = Some(E_INVSTREAM);
            }
            return false;
        }
    } else if (part == K_CHANNEL_PART_STDIN || part == K_CHANNEL_PART_STDOUT) && chan.is_rpc.get() {
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
            rstream_may_close(&chan.socket);
        }
        K_CHANNEL_STREAM_PROC => {
            let proc_ptr = &chan.uv.proc_;
            if part == K_CHANNEL_PART_STDIN || close_main {
                stream_may_close(&proc_ptr.r#in);
            }
            if part == K_CHANNEL_PART_STDOUT || close_main {
                rstream_may_close(&proc_ptr.out);
            }
            if part == K_CHANNEL_PART_STDERR || part == K_CHANNEL_PART_ALL {
                rstream_may_close(&proc_ptr.err);
            }
            if proc_ptr.r#type.get() == K_PROC_TYPE_PTY && part == K_CHANNEL_PART_ALL {
                pty_proc_close_master(proc_ptr);
            }
        }
        K_CHANNEL_STREAM_STDIO => {
            if part == K_CHANNEL_PART_STDIN || close_main {
                rstream_may_close(&chan.stdio_in);
            }
            if part == K_CHANNEL_PART_STDOUT || close_main {
                stream_may_close(&chan.stdio_out);
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
            if !chan.err_closed.get() {
                chan.err_closed.set(true);
                if !exc_state().exiting.get() {
                    redirect_stderr_to_null();
                }
                channel_decref(&chan);
            }
        }
        K_CHANNEL_STREAM_INTERNAL => {
            if !close_main {
                if let Some(err) = error {
                    *err = Some(E_INVSTREAM);
                }
                return false;
            }
            if !chan.term.get().is_null() {
                let mut internal = chan.internal.get();
                api_free_luaref(internal.cb);
                internal.cb = LUA_NOREF;
                internal.closed = true;
                chan.internal.set(internal);
                terminal_close(chan.term.get(), 0);
                chan.exit_status.set(0);
            } else {
                channel_decref(&chan);
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
pub fn channel_alloc(r#type: ChannelStreamType) -> std::rc::Rc<Channel> {
    let id = if r#type == K_CHANNEL_STREAM_STDIO {
        CHAN_STDIO
    } else if r#type == K_CHANNEL_STREAM_STDERR {
        CHAN_STDERR
    } else {
        chan_st(|st| {
            let id = st.next_chan_id.get();
            st.next_chan_id.set(id + 1);
            id
        })
    };
    assert!(id <= (i64::MAX as u64));

    let qc = loop_queues(crate::event::r#loop::main_loop());
    let chan = std::rc::Rc::new(Channel::new(
        id,
        r#type,
        multiqueue_new_child(qc.events.get()),
    ));
    qc.insert_channel(chan.clone());
    chan
}

pub fn channel_create_event(chan: &Channel) {
    channel_event(chan, EVENT_CHANOPEN);
}

pub fn channel_incref(chan: &Channel) {
    chan.refcount.set(chan.refcount.get() + (1));
}

pub fn channel_decref(chan: &Channel) {
    if chan.refcount.get() == 1 && !chan.did_close_event.get() {
        chan.did_close_event.set(true);
        channel_event(chan, EVENT_CHANCLOSE);
    }

    chan.refcount.set(chan.refcount.get() - (1));
    if chan.refcount.get() == 0 {
        let qc = loop_queues(crate::event::r#loop::main_loop());
        multiqueue_put_event(qc.events.get(), Event::ChannelFree(chan.id));
    }
}

pub fn callback_reader_start(reader: &mut CallbackReader, r#type: &'static CStr) {
    reader.reader_type = Some(r#type);
}

pub fn free_channel_event(chan_id: u64) {
    drop(crate::event::r#loop::main_loop().remove_channel(chan_id));
}

fn channel_destroy_early(chan: &Channel) {
    let next_id = chan_st(|st| {
        let id = st.next_chan_id.get() - 1;
        st.next_chan_id.set(id);
        id
    });
    if chan.id != next_id {
        std::process::abort();
    }
    let chan_id = chan.id;
    let owned = crate::event::r#loop::main_loop()
        .remove_channel(chan_id)
        .unwrap();
    owned.refcount.set(owned.refcount.get() - 1);
    if owned.refcount.get() != 0 {
        std::process::abort();
    }

    let qc = loop_queues(crate::event::r#loop::main_loop());
    multiqueue_put_event(qc.events.get(), Event::ChannelFreeOwned(owned));
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
    env: Option<crate::eval::typval::DictId>,
    status_out: &mut Varnumber,
) -> Option<std::rc::Rc<Channel>> {
    let env_ptr = env.unwrap_or(crate::eval::typval::DictId::NULL);
    let chan = channel_alloc(K_CHANNEL_STREAM_PROC);
    let chan_id = chan.id;
    chan.set_reader(false, on_stdout);
    chan.set_reader(true, on_stderr);
    chan.set_exit_callback(on_exit);

    if pty {
        if detach {
            semsg!(gettext(E_INVARG2), c"terminal/pty job cannot be detached",);
            if let Some(env_ref) = env_ptr.opt() {
                tv_dict_free(env_ref);
            }
            drop(argv);
            channel_destroy_early(&chan);

            *status_out = 0;

            return None;
        }
        chan.uv.proc_.r#type.set(K_PROC_TYPE_PTY);
        chan.uv.proc_.chan_id.set(chan_id);
        chan.uv.proc_.status.set(-1);
        chan.uv.proc_.out.s.fd.set(STDOUT_FILENO);
        chan.uv.proc_.err.s.fd.set(STDERR_FILENO);
        chan.uv
            .proc_
            .width
            .set(if pty_width > 0 { pty_width } else { 80 });
        chan.uv
            .proc_
            .height
            .set(if pty_height > 0 { pty_height } else { 24 });
        #[cfg(not(windows))]
        {
            chan.uv.proc_.tty_fd.set(-1);
        }
    } else {
        chan.uv.proc_.r#type.set(K_PROC_TYPE_UV);
        chan.uv.proc_.chan_id.set(chan_id);
        chan.uv.proc_.status.set(-1);
        chan.uv.proc_.out.s.fd.set(STDOUT_FILENO);
        chan.uv.proc_.err.s.fd.set(STDERR_FILENO);
    }

    chan.uv.proc_.argv.set(argv);
    chan.uv.proc_.exepath.set(exepath.map(CStr::to_owned));
    chan.uv.proc_.events.set(chan.events);
    chan.uv.proc_.detach.set(detach);
    chan.uv.proc_.cwd.set(cwd.map(CStr::to_owned));
    chan.uv.proc_.env.set(env_ptr);
    chan.uv.proc_.overlapped.set(overlapped);

    #[cfg(windows)]
    {
        chan.uv.proc_.stdio_noinherit.set(true);
    }

    let cmd = chan
        .uv
        .proc_
        .exepath
        .get()
        .clone()
        .unwrap_or_else(|| chan.uv.proc_.argv.get().as_slice()[0].clone());

    let (has_out, has_err) = if chan.uv.proc_.r#type.get() == K_PROC_TYPE_PTY {
        (true, false)
    } else {
        let out = rpc || chan.with_reader(false, callback_reader_set);
        #[cfg(windows)]
        let mut err = chan.with_reader(true, callback_reader_set);
        #[cfg(not(windows))]
        let err = chan.with_reader(true, callback_reader_set);

        chan.uv
            .proc_
            .fwd_err
            .set(chan.with_reader(true, |reader| reader.fwd_err));

        #[cfg(windows)]
        if !err && chan.uv.proc_.fwd_err.get() && chan.uv.proc_.detach.get() {
            err = true;
            chan.uv.proc_.fwd_err.set(false);
        }
        (out, err)
    };

    let has_in = stdin_mode == K_CHANNEL_STDIN_PIPE_CONST;

    let status = proc_spawn(&chan.uv.proc_, has_in, has_out, has_err);
    if status != 0 {
        semsg!(gettext(E_JOBSPAWN), uv_strerror(status), &cmd,);

        if let Some(env_ref) = chan.uv.proc_.env.get().opt() {
            tv_dict_free(env_ref);
        }
        let proc_status = chan.uv.proc_.status.get();
        channel_destroy_early(&chan);

        *status_out = Varnumber::from(proc_status);

        return None;
    }

    drop(cmd);
    if let Some(env_ref) = chan.uv.proc_.env.get().opt() {
        tv_dict_free(env_ref);
    }

    if has_in {
        wstream_init(&chan.uv.proc_.r#in, 0);
    }
    if has_out {
        rstream_init(&chan.uv.proc_.out);
    }

    if rpc {
        rpc_start(&chan);
    } else if has_out {
        chan.with_reader_mut(false, |reader| callback_reader_start(reader, c"stdout"));

        rstream_start(&chan.uv.proc_.out, StreamCbData::ChannelData(chan_id));
    }

    if has_err {
        chan.with_reader_mut(true, |reader| callback_reader_start(reader, c"stderr"));
        rstream_init(&chan.uv.proc_.err);

        rstream_start(&chan.uv.proc_.err, StreamCbData::ChannelStderr(chan_id));
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
        channel.internal.set(InternalState {
            cb: LUA_NOREF,
            closed: false,
        });
        rpc_start(&channel);
        channel.trust.set(trust);
        channel_create_event(&channel);
        return channel.id;
    }

    let channel = channel_alloc(K_CHANNEL_STREAM_SOCKET);
    let chan_id = channel.id;
    let connected = socket_connect(
        crate::event::r#loop::main_loop(),
        &channel.socket,
        tcp,
        address,
        timeout,
        error,
    );

    if !connected {
        channel_decref(&channel);
        return 0;
    }

    channel
        .socket
        .s
        .internal_data
        .set(StreamInternalData::Channel(chan_id));
    wstream_init(&channel.socket.s, 0);
    rstream_init(&channel.socket);

    if rpc {
        rpc_start(&channel);
    } else {
        channel.set_reader(false, on_output);
        channel.with_reader_mut(false, |reader| callback_reader_start(reader, c"data"));
        rstream_start(&channel.socket, StreamCbData::ChannelData(chan_id));
    }

    channel.trust.set(trust);
    channel_create_event(&channel);
    channel.id
}

/// Creates an RPC channel from a tcp/pipe socket connection.
pub fn channel_from_connection(watcher: &SocketHandle) {
    let channel = channel_alloc(K_CHANNEL_STREAM_SOCKET);
    let chan_id = channel.id;
    let result = socket_watcher_accept(watcher, &channel.socket);
    if result != 0 {
        channel_destroy_early(&channel);
        return;
    }
    channel
        .socket
        .s
        .internal_data
        .set(StreamInternalData::Channel(chan_id));
    wstream_init(&channel.socket.s, 0);
    rstream_init(&channel.socket);
    rpc_start(&channel);
    channel_create_event(&channel);
}

/// Creates an API channel from stdin/stdout. Used when embedding Nvim.
pub fn channel_from_stdio(
    rpc: bool,
    on_output: CallbackReader,
    error: &mut Option<&'static CStr>,
) -> u64 {
    if !exc_state().headless_mode.get() && !exc_state().embedded_mode.get() {
        *error = Some(gettext(c"can only be opened in headless mode"));
        return 0;
    }

    if chan_st(|st| st.did_stdio.replace(true)) {
        *error = Some(gettext(c"channel was already open"));
        return 0;
    }

    let channel = channel_alloc(K_CHANNEL_STREAM_STDIO);
    let chan_id = channel.id;
    let mut stdin_dup_fd = STDIN_FILENO;
    let mut stdout_dup_fd = STDOUT_FILENO;

    #[cfg(windows)]
    if exc_state().embedded_mode.get() && os_has_conpty_working() {
        stdin_dup_fd = os_dup_cloexec(STDIN_FILENO);
        stdout_dup_fd = os_dup_cloexec(STDOUT_FILENO);
        os_attach_parent_console();
    }

    #[cfg(not(windows))]
    if exc_state().embedded_mode.get() {
        stdin_dup_fd = os_dupfd_cloexec(STDIN_FILENO, STDERR_FILENO + 1);
        stdout_dup_fd = os_dupfd_cloexec(STDOUT_FILENO, STDERR_FILENO + 1);
        let _ = dup2(STDERR_FILENO, STDOUT_FILENO);
        let _ = dup2(STDERR_FILENO, STDIN_FILENO);
    }

    rstream_init_fd(
        crate::event::r#loop::main_loop(),
        &channel.stdio_in,
        stdin_dup_fd,
    );
    wstream_init_fd(
        crate::event::r#loop::main_loop(),
        &channel.stdio_out,
        stdout_dup_fd,
        0,
    );

    if rpc {
        rpc_start(&channel);
    } else {
        channel.set_reader(false, on_output);
        channel.with_reader_mut(false, |reader| callback_reader_start(reader, c"stdin"));
        rstream_start(&channel.stdio_in, StreamCbData::ChannelData(chan_id));
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
        if chan.err_closed.get() {
            *error = Some(gettext(c"Can't send data to closed stream"));
            return 0;
        }
        let wres = os_write(STDERR_FILENO, &data, false);
        return if wres >= 0 { wres as usize } else { 0 };
    }

    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        if chan.is_rpc.get() {
            *error = Some(gettext(c"Can't send raw data to rpc channel"));
            return 0;
        }
        if chan.term.get().is_null() || chan.internal.get().closed {
            *error = Some(gettext(c"Can't send data to closed stream"));
            return 0;
        }
        terminal_receive(chan.term(), &data);
        return len;
    }

    let is_rpc = chan.is_rpc.get();
    let in_stream = channel_instream(&chan);
    if in_stream.closed.get() {
        *error = Some(gettext(c"Can't send data to closed stream"));
        return 0;
    }

    if is_rpc {
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
fn buffer_to_tv_list(buf: &[u8]) -> ListId {
    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    tv_list_append_string(&l, Some(c""), 0);
    if !buf.is_empty() {
        encode_list_write(Some(&l), buf);
    }
    l
}

pub fn on_channel_data(buf: &[u8], chan: &Channel, eof: bool) -> usize {
    let consumed = on_channel_output(chan, buf, eof, false);
    if eof && chan.streamtype == K_CHANNEL_STREAM_SOCKET {
        // Peer stopped sending (half-close not supported), close the channel (like receive_msgpack()).
        // Other streams stay open on read-side EOF: a job may read stdin, stdio stdout is writable.
        channel_close(chan.id, K_CHANNEL_PART_ALL, None);
    }
    consumed
}

pub fn on_job_stderr(buf: &[u8], chan: &Channel, eof: bool) -> usize {
    on_channel_output(chan, buf, eof, true)
}

fn on_channel_output(chan: &Channel, buf: &[u8], eof: bool, is_stderr: bool) -> usize {
    let count = buf.len();
    if !chan.term.get().is_null() {
        terminal_receive(chan.term(), buf);
    }
    #[cfg(windows)]
    if chan.with_reader(is_stderr, |reader| reader.fwd_err) && count > 0 {
        let wres = os_write(STDOUT_FILENO, &buf[..count], false);
        return wres.max(0) as usize;
    }
    let schedule = chan.with_reader_mut(is_stderr, |reader| {
        reader.eof |= eof;
        let set = callback_reader_set(reader);
        if set {
            reader.buffer.extend_from_slice(buf);
        }
        set
    });
    if schedule {
        schedule_channel_event(chan);
    }

    count
}

fn schedule_channel_event(chan: &Channel) {
    if !chan.callback_scheduled.get() {
        if !chan.callback_busy.get() {
            multiqueue_put_event(chan.events(), Event::ChannelCallback(chan.id));
            channel_incref(chan);
        }
        chan.callback_scheduled.set(true);
    }
}

pub fn on_channel_event(chan: &Channel) {
    chan.callback_busy.set(true);
    chan.callback_scheduled.set(false);

    let exit_status = chan.exit_status.get();
    channel_reader_callbacks(chan, false);
    channel_reader_callbacks(chan, true);
    if exit_status > -1 {
        channel_callback_call(chan, None);
        chan.exit_status.set(-1);
    }

    chan.callback_busy.set(false);
    if chan.callback_scheduled.get() {
        multiqueue_put_event(chan.events, Event::ChannelCallback(chan.id));
        channel_incref(chan);
    }

    channel_decref(chan);
}

pub fn channel_reader_callbacks(chan: &Channel, is_stderr: bool) {
    let (buffered, eof, not_empty, dict, reader_type) = chan.with_reader(is_stderr, |reader| {
        (
            reader.buffered,
            reader.eof,
            !reader.buffer.is_empty(),
            reader.self_dict.clone(),
            reader.reader_type,
        )
    });
    if buffered {
        if eof {
            if !dict.is_null() {
                let reader_type = reader_type.unwrap();
                if tv_dict_find(dict.opt(), reader_type, -1).is_none() {
                    let bytes = chan.with_reader(is_stderr, |reader| reader.buffer.clone());
                    let data = buffer_to_tv_list(&bytes);
                    let _ = tv_dict_add_list(&dict, reader_type, Some(&data));
                } else {
                    semsg!(gettext(E_STREAMKEY), reader_type, chan.id,);
                }
                chan.with_reader_mut(is_stderr, |reader| reader.eof = false);
            } else {
                chan.with_reader_mut(is_stderr, |reader| reader.eof = false);
                channel_callback_call(chan, Some(is_stderr));
            }
        }
    } else {
        if not_empty {
            channel_callback_call(chan, Some(is_stderr));
        }
        if eof {
            channel_callback_call(chan, Some(is_stderr));
            chan.with_reader_mut(is_stderr, |reader| reader.eof = false);
        }
    }
}

pub(crate) fn channel_proc_exit_cb(status: i32, chan: &Channel) {
    if !chan.term.get().is_null() {
        terminal_close(chan.term.get(), status);
    }

    if !exc_state().exiting.get() && screen_state().ui_client_channel_id.get() == chan.id {
        ui_client_attach_to_restarted_server(status != 0);
        if screen_state().ui_client_channel_id.get() == chan.id {
            exit_on_closed_chan(status);
        }
    }

    let exited = status >= 0;
    if exited && chan.on_exit.cb_type() != K_CALLBACK_NONE_CONST {
        schedule_channel_event(chan);
    }
    if exited {
        chan.exit_status.set(status);
    }

    channel_decref(chan);
}

#[cfg(unix)]
pub(crate) fn channel_proc_state_cb(
    _proc: &crate::event::proc::Proc,
    suspended: bool,
    chan: &Channel,
) {
    if let Some(term) = chan.term.get().opt() {
        terminal_set_state(term, suspended);
    }
}

fn channel_callback_call(chan: &Channel, is_stderr: Option<bool>) {
    let mut argv: [TypVal; 4] = [const { TypVal::new() }; 4];
    argv[0].v_type = VarType::VarNumber;
    argv[0].v_lock = VAR_UNLOCKED;
    argv[0].vval = TypvalVval::Number(chan.id as Varnumber);

    let cb = if let Some(is_stderr) = is_stderr {
        let (bytes, reader_type, callback) = chan.with_reader_mut(is_stderr, |reader| {
            (
                std::mem::take(&mut reader.buffer),
                reader.reader_type,
                reader.cb.clone(),
            )
        });
        let l = buffer_to_tv_list(&bytes);
        tv_list_ref(Some(&l));
        argv[1].v_type = VarType::VarList;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval = TypvalVval::List(l);
        argv[2].v_type = VarType::VarString;
        argv[2].v_lock = VAR_UNLOCKED;
        argv[2].vval = TypvalVval::from_opt_cstr(reader_type);
        callback
    } else {
        argv[1].v_type = VarType::VarNumber;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval = TypvalVval::Number(Varnumber::from(chan.exit_status.get()));
        argv[2].v_type = VarType::VarString;
        argv[2].v_lock = VAR_UNLOCKED;
        argv[2].vval = TypvalVval::from_cstr(c"exit");
        chan.on_exit.clone()
    };

    let mut rettv: TypVal = TypVal::new();
    let _ = callback_call(&cb, 3, &mut argv, &mut rettv);
    tv_clear(&mut rettv);

    if is_stderr.is_some() {
        tv_list_unref(argv[1].vval.v_list_opt());
    }
}

/// Allocate terminal for channel.
pub fn channel_terminal_alloc(buf: &FileBuf, chan: &Channel) {
    let topts = TerminalOptions {
        chan_id: chan.id,
        width: chan.uv.proc_.width.get(),
        height: chan.uv.proc_.height.get(),
        force_crlf: false,
    };
    buf.b_p_channel.set(chan.id as OptInt);
    channel_incref(chan);
    {
        chan.term.set(terminal_alloc(buf, topts));
    }
}

pub fn channel_term_read_pause(pause: bool, chan: &Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        return;
    }
    let out_stream = &chan.uv.proc_.out;
    if out_stream.s.closed.get() {
        return;
    }
    if pause {
        rstream_stop_inner(out_stream);
    } else {
        rstream_start_inner(out_stream);
    }
}

pub fn channel_term_write(buf: &[u8], chan: &Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        let cb = chan.internal.get().cb;
        if cb == LUA_NOREF {
            return;
        }
        let mut args_items = [
            crate::api::private::helpers::integer_obj(chan.id as Integer),
            crate::api::private::helpers::buffer_obj(crate::terminal::terminal_buf(
                chan.term.get().borrow(),
            )),
            crate::api::private::helpers::string_obj(NvimString::from_bytes(buf)),
        ];
        let args = Array::from_slice(&mut args_items[..3]);
        {
            let _textlock = input_state().lock_text();
            crate::lua::executor::nlua_call_ref_log_err(
                cb,
                Some(c"input"),
                args,
                crate::lua::executor::K_RET_NIL_BOOL,
            );
        }
        return;
    }
    let in_stream = &chan.uv.proc_.r#in;
    if in_stream.closed.get() {
        return;
    }
    let wbuf = wstream_new_vec_buffer(buf.to_vec());
    let _ = wstream_write(in_stream, wbuf);
}

pub fn channel_term_resize(width: u16, height: u16, chan: &Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        return;
    }
    pty_proc_resize(&chan.uv.proc_, width, height);
}

pub fn channel_term_resume(chan: &Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        return;
    }
    pty_proc_resume(&chan.uv.proc_);
}

pub fn term_delayed_free(chan: &Channel) {
    let proc_ptr = &chan.uv.proc_;
    if proc_ptr.r#in.pending_reqs.get() != 0 || proc_ptr.out.s.pending_reqs.get() != 0 {
        multiqueue_put_event(chan.events, Event::ChannelTermDelayedFree(chan.id));
        return;
    }

    if !chan.term.get().is_null() {
        terminal_destroy(chan);
    }
    channel_decref(chan);
}

pub fn channel_term_close(chan: &Channel) {
    if chan.streamtype == K_CHANNEL_STREAM_INTERNAL {
        let mut internal = chan.internal.get();
        let cb = internal.cb;
        internal.cb = LUA_NOREF;
        chan.internal.set(internal);
        terminal_destroy(chan);
        crate::lua::executor::api_free_luaref(cb);
        channel_decref(chan);
        return;
    }
    proc_stop(&chan.uv.proc_);
    multiqueue_put_event(chan.events, Event::ChannelTermDelayedFree(chan.id));
}

pub fn channel_event(chan: &Channel, event: AuEvent) {
    if has_event(event) {
        channel_incref(chan);
        let qc = loop_queues(crate::event::r#loop::main_loop());
        multiqueue_put_event(
            qc.events.get(),
            Event::ChannelSetInfo {
                chan_id: chan.id,
                event,
            },
        );
    }
}

pub fn set_info_event(chan: &Channel, event: AuEvent) {
    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let dict = get_v_event(&mut save_v_event);
    let info = channel_info(chan.id);
    let mut retval: TypVal = TypVal::new();
    object_to_vim(
        Object {
            obj_type: ObjectType::ObjectTypeDict,
            data: ObjectData::Dict(info),
        },
        &mut retval,
    );
    assert_eq!(retval.v_type, VarType::VarDict);
    let _ = tv_dict_add_dict(&dict, c"info", retval.vval.v_dict_opt());
    tv_dict_set_keys_readonly(&dict);

    let _ = apply_autocmds(event, None, None, true, curbuf_opt().as_deref());

    restore_v_event(&dict, &mut save_v_event);
    channel_decref(chan);
}

pub fn channel_job_running(id: u64) -> bool {
    let Some(chan) = find_channel(id) else {
        return false;
    };
    if chan.streamtype != K_CHANNEL_STREAM_PROC {
        return false;
    }
    let ph = &chan.uv.proc_;
    !(ph.status.get() >= 0 || ph.stopped_time.get() != 0)
}

pub fn channel_info(id: u64) -> Dict {
    let Some(chan) = find_channel(id) else {
        return Dict::new();
    };

    let mut info = Dict::with_capacity(10);

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
            let ph = &chan.uv.proc_;
            if ph.r#type.get() == K_PROC_TYPE_PTY {
                let s = match pty_proc_tty_name(ph) {
                    Some(name) => NvimString::from_bytes(name.to_bytes()),
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

            let args = ph.argv.get();
            let mut argv = Array::new();
            if !args.is_empty() {
                let n = args.len();
                argv = Array::with_capacity(n);
                for arg in &args {
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
    if let Some(ref addr) = chan.addr.get() {
        put_c(
            &mut info,
            c"addr",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(Some(addr.as_c_str()))),
            },
        );
    }

    let mode_desc: &'static CStr = if chan.is_rpc.get() {
        put_c(
            &mut info,
            c"client",
            Object {
                obj_type: ObjectType::ObjectTypeDict,
                data: ObjectData::Dict(chan.rpc.info.get().clone()),
            },
        );

        put_c(
            &mut info,
            c"detach",
            Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(chan.detach.get()),
            },
        );

        put_c(
            &mut info,
            c"trust",
            Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(chan.trust.get()),
            },
        );

        c"rpc"
    } else if !chan.term.get().is_null() {
        let tbuf = terminal_buf(chan.term.get().borrow());
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
                data: ObjectData::Integer(i64::from(chan.exit_status.get())),
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

pub fn channel_all_info() -> Array {
    let channels = crate::event::r#loop::main_loop().channels();
    let mut ret = Array::with_capacity(channels.len());
    for channel in channels {
        let info = channel_info(channel.id);
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
        let reader = CallbackReader::none();
        assert!(!callback_reader_set(&reader));
        reader
            .cb
            .data
            .set(crate::eval::typval::CallbackData::lua(1));
        assert!(callback_reader_set(&reader));
        reader.cb.data.set(crate::eval::typval::CallbackData::None);
    }
}
