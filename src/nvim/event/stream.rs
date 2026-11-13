//! Low-level libuv stream wrapper (`src/nvim/event/stream.c`).

use crate::event::r#loop::{
    Loop, UV_RUN_NOWAIT, uv_close, uv_guess_handle, uv_handle_t, uv_is_closing, uv_loop_close,
    uv_loop_init, uv_loop_t, uv_pipe_init, uv_pipe_open, uv_run,
};
#[cfg(windows)]
use crate::event::r#loop::{uv_handle_size, uv_tty_init, uv_tty_set_mode};
use crate::event::multiqueue::MultiQueue;
use crate::globals::g_st;
use crate::log::{LOGLVL_DBG, LOGLVL_INF, logmsg, min_log_level};
use std::ffi::c_void;

pub const UV_NAMED_PIPE: i32 = 7;
#[cfg(windows)]
pub const UV_STREAM: i32 = 11;
pub const UV_TCP: i32 = 12;
pub const UV_TTY: i32 = 14;
pub const UV_FILE: i32 = 17;

#[cfg(windows)]
use crate::event::r#loop::UV_TTY_MODE_NORMAL;
#[cfg(windows)]
const UV_TTY_MODE_RAW: i32 = 1;

#[cfg(windows)]
pub(crate) const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

#[allow(non_camel_case_types)]
pub type uv_stream_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_pipe_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_idle_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_tty_t = c_void;

#[derive(Clone, Copy)]
pub enum StreamCbData {
    None,
    ChannelData(u64),
    ChannelStderr(u64),
    ChannelRpc(u64),
    ShellBuf,
    ShellOut,
    Input,
}

impl StreamCbData {
    pub const NONE: Self = Self::None;

    #[inline]
    pub const fn has_read_cb(self) -> bool {
        !matches!(self, Self::None)
    }

    pub fn call_read_cb(self, stream_fd: i32, buf: &[u8], eof: bool) -> Option<usize> {
        match self {
            Self::None => None,
            Self::ChannelData(chan_id) => {
                let chan = crate::msgpack_rpc::channel::find_channel_opt(chan_id)?;
                Some(crate::channel::on_channel_data(buf, chan, eof))
            }
            Self::ChannelStderr(chan_id) => {
                let chan = crate::msgpack_rpc::channel::find_channel_opt(chan_id)?;
                Some(crate::channel::on_job_stderr(buf, chan, eof))
            }
            Self::ChannelRpc(chan_id) => {
                let chan = crate::msgpack_rpc::channel::find_channel_opt(chan_id)?;
                Some(crate::msgpack_rpc::channel::receive_msgpack(buf, chan, eof))
            }
            Self::ShellBuf => Some(crate::os::shell::system_data_cb(buf, eof)),
            Self::ShellOut => Some(crate::os::shell::out_data_cb(stream_fd, buf, eof)),
            Self::Input => Some(crate::ui_client::with_tui_mut(|tui| {
                crate::tui::input::tinput_read_cb(buf, tui, eof)
            })),
        }
    }
}

#[derive(Clone, Copy)]
pub enum StreamInternalData {
    None,
    Channel(u64),
    Proc(*mut crate::event::proc::Proc),
    Closed(*mut bool),
}

impl StreamInternalData {
    pub const NONE: Self = Self::None;

    #[cfg(windows)]
    #[inline]
    pub fn as_proc(self) -> &'static mut crate::event::proc::Proc {
        match self {
            Self::Proc(p) => unsafe { &mut *p },
            _ => unreachable!(),
        }
    }
}

#[repr(C, align(8))]
pub struct UvStreamHandle(pub [u8; 640]);

#[repr(C)]
pub struct Stream {
    pub closed: bool,
    pub uv: UvStreamHandle,
    pub has_uvstream: bool,
    pub fd: i32,
    pub fpos: i64,
    pub cb_data: StreamCbData,
    pub is_rstream: bool,
    #[cfg(windows)]
    pub close_console: bool,
    pub internal_data: StreamInternalData,
    pub pending_reqs: usize,
    pub events: MultiQueue,
    pub notify_write: bool,
    pub curmem: usize,
    pub maxmem: usize,
}

impl Stream {
    pub const fn new() -> Self {
        Self {
            closed: false,
            uv: UvStreamHandle([0; 640]),
            has_uvstream: false,
            fd: -1,
            fpos: 0,
            cb_data: StreamCbData::NONE,
            is_rstream: false,
            #[cfg(windows)]
            close_console: false,
            internal_data: StreamInternalData::NONE,
            pending_reqs: 0,
            events: MultiQueue::NULL,
            notify_write: false,
            curmem: 0,
            maxmem: 0,
        }
    }

    #[inline]
    pub fn uvstream_mut(&mut self) -> Option<&mut uv_stream_t> {
        if self.has_uvstream {
            Some(unsafe { &mut *(&raw mut self.uv).cast() })
        } else {
            None
        }
    }
}

use crate::event::r#loop::{uv_idle_init, uv_stream_get_write_queue_size, uv_stream_set_blocking};

#[cfg(windows)]
use crate::os::os_win_console::{GetConsoleMode, SetConsoleMode};

pub fn stream_set_blocking(fd: i32, blocking: bool) -> i32 {
    unsafe {
        let mut loop_buf = [0u64; 256];
        let mut pipe_buf = [0u64; 128];
        let loop_ptr = loop_buf.as_mut_ptr().cast::<uv_loop_t>();
        let pipe_ptr = pipe_buf.as_mut_ptr().cast::<uv_pipe_t>();
        uv_loop_init(loop_ptr);
        uv_pipe_init(loop_ptr, pipe_ptr, 0);
        uv_pipe_open(pipe_ptr, fd);
        let retval = uv_stream_set_blocking(pipe_ptr.cast(), i32::from(blocking));
        uv_close(pipe_ptr.cast(), None);
        uv_run(loop_ptr, UV_RUN_NOWAIT);
        uv_loop_close(loop_ptr);
        retval
    }
}

pub fn stream_init(loop_: Option<&mut Loop>, stream: &mut Stream, fd: i32, has_uvstream: bool) {
    assert!(if !has_uvstream {
        fd >= 0 && loop_.is_some()
    } else {
        fd < 0 && loop_.is_none()
    });
    let uv_ptr: *mut c_void = (&raw mut stream.uv).cast();
    stream.has_uvstream = has_uvstream;

    if fd >= 0 {
        let loop_ptr: *mut uv_loop_t = (&raw mut loop_.unwrap().uv).cast();
        let h_type = unsafe { uv_guess_handle(fd) };
        stream.fd = fd;

        if h_type == UV_FILE {
            unsafe { uv_idle_init(loop_ptr, uv_ptr.cast()) };
            unsafe {
                *uv_ptr.cast::<*mut Stream>() = stream;
            }
        } else {
            #[cfg(windows)]
            if h_type == UV_TTY {
                unsafe { uv_tty_init(loop_ptr, uv_ptr.cast(), fd, 0) };
                unsafe { uv_tty_set_mode(uv_ptr.cast(), UV_TTY_MODE_RAW) };
                let tty_handle = unsafe {
                    *uv_ptr
                        .cast::<u8>()
                        .wrapping_add(uv_handle_size(UV_STREAM))
                        .cast::<*mut c_void>()
                };
                let mut dw_mode: u32 = 0;
                if unsafe { GetConsoleMode(tty_handle, &mut dw_mode) } != 0 {
                    dw_mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
                    unsafe { SetConsoleMode(tty_handle, dw_mode) };
                }
                stream.has_uvstream = true;
            } else {
                assert!(h_type == UV_NAMED_PIPE || h_type == UV_TTY);
                unsafe { uv_pipe_init(loop_ptr, uv_ptr.cast(), 0) };
                unsafe { uv_pipe_open(uv_ptr.cast(), fd) };
                stream.has_uvstream = true;
            }

            #[cfg(not(windows))]
            {
                assert!(h_type == UV_NAMED_PIPE || h_type == UV_TTY);
                unsafe { uv_pipe_init(loop_ptr, uv_ptr.cast(), 0) };
                unsafe { uv_pipe_open(uv_ptr.cast(), fd) };
                stream.has_uvstream = true;
            }
        }
    }

    if stream.has_uvstream {
        unsafe {
            *(&raw mut stream.uv).cast::<*mut Stream>() = stream;
        }
    }

    stream.fpos = 0;
    stream.internal_data = StreamInternalData::NONE;
    stream.curmem = 0;
    stream.maxmem = 0;
    stream.pending_reqs = 0;
    stream.notify_write = false;
    stream.is_rstream = false;
    stream.closed = false;
    stream.events = MultiQueue::NULL;
}

pub fn stream_may_close(stream: &mut Stream) {
    if stream.closed {
        return;
    }
    if min_log_level() <= LOGLVL_DBG {
        logmsg!(
            LOGLVL_DBG,
            None,
            Some(c"stream_may_close"),
            94,
            true,
            c"closing Stream: %p",
            std::ptr::from_ref(stream).addr(),
        );
    }
    stream.closed = true;

    #[cfg(windows)]
    unsafe {
        if uv_guess_handle(stream.fd) == UV_TTY {
            uv_tty_set_mode((&raw mut stream.uv).cast(), UV_TTY_MODE_NORMAL);
        }
    }

    if stream.pending_reqs == 0 {
        stream_close_handle(stream);
    }
}

pub fn stream_close_handle(stream: &mut Stream) {
    let handle: *mut uv_handle_t = if stream.has_uvstream {
        let uv_s: *mut uv_stream_t = (&raw mut stream.uv).cast();
        let queued = unsafe { uv_stream_get_write_queue_size(uv_s) };
        if !g_st().exiting.get() && queued > 0 {
            logmsg!(
                LOGLVL_INF,
                None,
                Some(c"stream_close_handle"),
                116,
                true,
                c"closed Stream (%p) with %zu unwritten bytes",
                std::ptr::from_ref(stream).addr(),
                queued,
            );
        }
        uv_s.cast()
    } else {
        (&raw mut stream.uv).cast()
    };

    assert!(!handle.is_null());

    #[cfg(windows)]
    if stream.close_console {
        stream.pending_reqs += 1;
        crate::os::pty_proc_win::pty_proc_close_console(stream);
        stream.pending_reqs -= 1;
    }
    if unsafe { uv_is_closing(handle) } == 0 {
        unsafe { uv_close(handle, Some(close_cb)) };
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let stream = *handle.cast::<*mut Stream>();
        if !stream.is_null() {
            if (*stream).is_rstream {
                crate::event::rstream::rstream_close_cb(&mut *stream);
            }
            let internal_data = (*stream).internal_data;
            match internal_data {
                StreamInternalData::None => {}
                StreamInternalData::Channel(chan_id) => {
                    if let Some(chan) = crate::msgpack_rpc::channel::find_channel_opt(chan_id) {
                        crate::channel::channel_decref(chan);
                    }
                }
                StreamInternalData::Proc(proc_) => {
                    crate::event::proc::proc_decref(&mut *proc_);
                }
                StreamInternalData::Closed(closed_ptr) => *closed_ptr = true,
            }
        }
    }
}

#[repr(C)]
pub struct UvSyncOutput {
    write_loop: [u64; 256],
    output_handle: [u64; 80],
}

impl UvSyncOutput {
    pub const fn new() -> Self {
        Self {
            write_loop: [0; 256],
            output_handle: [0; 80],
        }
    }

    #[inline]
    fn loop_ptr(&mut self) -> *mut uv_loop_t {
        self.write_loop.as_mut_ptr().cast()
    }

    #[inline]
    fn handle_ptr(&mut self) -> *mut uv_handle_t {
        self.output_handle.as_mut_ptr().cast()
    }

    pub fn init_loop(&mut self) {
        debug_assert!(unsafe { crate::event::r#loop::uv_loop_size() } <= size_of::<[u64; 256]>());
        debug_assert!(
            unsafe { crate::event::r#loop::uv_handle_size(UV_NAMED_PIPE) }
                .max(unsafe { crate::event::r#loop::uv_handle_size(UV_TTY) })
                <= size_of::<[u64; 80]>()
        );
        unsafe { uv_loop_init(self.loop_ptr()) };
    }

    pub fn init_tty(&mut self, fd: i32) -> i32 {
        unsafe { crate::event::r#loop::uv_tty_init(self.loop_ptr(), self.handle_ptr(), fd, 0) }
    }

    #[cfg(not(windows))]
    pub fn set_tty_mode(&mut self, mode: i32) -> i32 {
        unsafe { crate::event::r#loop::uv_tty_set_mode(self.handle_ptr(), mode) }
    }

    pub fn init_pipe(&mut self) -> i32 {
        unsafe { uv_pipe_init(self.loop_ptr(), self.handle_ptr(), 0) }
    }

    pub fn open_pipe(&mut self, fd: i32) -> i32 {
        unsafe { uv_pipe_open(self.handle_ptr(), fd) }
    }

    pub fn stop_and_close(&mut self) {
        unsafe {
            crate::event::r#loop::uv_tty_reset_mode();
            uv_close(self.handle_ptr(), None);
            uv_run(self.loop_ptr(), crate::event::r#loop::UV_RUN_DEFAULT);
            if uv_loop_close(self.loop_ptr()) != 0 {
                std::process::abort();
            }
        }
    }

    pub fn is_closing(&mut self) -> bool {
        unsafe { uv_is_closing(self.handle_ptr()) != 0 }
    }

    pub fn get_winsize(&mut self, width: &mut i32, height: &mut i32) -> bool {
        unsafe { crate::event::r#loop::uv_tty_get_winsize(self.handle_ptr(), width, height) == 0 }
    }

    pub fn write_bufs(&mut self, bufs: &[crate::event::wstream::uv_buf_t]) -> i32 {
        let mut req = [0u64; 64];
        let ret = unsafe {
            crate::event::r#loop::uv_write(
                req.as_mut_ptr().cast(),
                self.handle_ptr(),
                bufs.as_ptr(),
                bufs.len() as u32,
                None,
            )
        };
        unsafe { uv_run(self.loop_ptr(), crate::event::r#loop::UV_RUN_DEFAULT) };
        ret
    }
}

impl Default for UvSyncOutput {
    fn default() -> Self {
        Self::new()
    }
}

pub fn stream_get_stty_erase(in_fd: i32, stream: &mut Stream) -> Option<u8> {
    #[cfg(unix)]
    {
        let _ = stream;
        let mut t = crate::os::pty_proc_unix::termios::new();
        if unsafe { crate::os::pty_proc_unix::tcgetattr(in_fd, &raw mut t) } != -1 {
            Some(t.c_cc[crate::os::pty_proc_unix::VERASE])
        } else {
            None
        }
    }
    #[cfg(windows)]
    {
        let _ = in_fd;
        let uv_ptr = &raw mut stream.uv;
        let mut dw_mode: u32 = 0;
        if unsafe { crate::event::r#loop::uv_handle_get_type(uv_ptr.cast()) } == UV_TTY {
            let tty_handle = unsafe {
                *uv_ptr
                    .cast::<u8>()
                    .wrapping_add(uv_handle_size(UV_STREAM))
                    .cast::<crate::os::os_win_console::HANDLE>()
            };
            if unsafe { GetConsoleMode(tty_handle, &raw mut dw_mode) } != 0
                && (dw_mode & ENABLE_VIRTUAL_TERMINAL_INPUT) != 0
            {
                return Some(0x7f);
            }
        }
        None
    }
}
