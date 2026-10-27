//! Low-level libuv stream wrapper (`src/nvim/event/stream.c`).

use crate::event::r#loop::{
    Loop, UV_RUN_NOWAIT, uv_close, uv_guess_handle, uv_handle_t, uv_is_closing, uv_loop_close,
    uv_loop_init, uv_loop_t, uv_pipe_init, uv_pipe_open, uv_run,
};
#[cfg(windows)]
use crate::event::r#loop::{uv_handle_size, uv_tty_init, uv_tty_set_mode};
use crate::event::multiqueue::MultiQueue;
use crate::log::{LOGLVL_DBG, LOGLVL_WRN, g_min_log_level, logmsg};
use std::ffi::{c_int, c_void};
use std::ptr;

pub const UV_NAMED_PIPE: c_int = 7;
#[cfg(windows)]
pub const UV_STREAM: c_int = 11;
pub const UV_TCP: c_int = 12;
pub const UV_TTY: c_int = 14;
pub const UV_FILE: c_int = 17;

#[cfg(windows)]
use crate::event::r#loop::UV_TTY_MODE_NORMAL;
#[cfg(windows)]
const UV_TTY_MODE_RAW: c_int = 1;

#[cfg(windows)]
pub(crate) const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

pub type uv_stream_t = c_void;
pub type uv_pipe_t = c_void;
pub type uv_idle_t = c_void;
pub type uv_tty_t = c_void;

#[derive(Clone, Copy)]
pub enum StreamCbData {
    None,
    ChannelData(*mut crate::channel::Channel),
    ChannelStderr(*mut crate::channel::Channel),
    ChannelRpc(*mut crate::channel::Channel),
    ShellBuf(*mut Vec<std::ffi::c_char>),
    ShellOut,
    Input(*mut crate::tui::input::TermInput),
}

impl StreamCbData {
    pub const NONE: Self = Self::None;

    #[inline]
    pub const fn has_read_cb(self) -> bool {
        !matches!(self, Self::None)
    }

    pub unsafe fn call_read_cb(
        self,
        stream: *mut crate::event::rstream::RStream,
        buf: *const std::ffi::c_char,
        count: usize,
        eof: bool,
    ) -> Option<usize> {
        unsafe {
            match self {
                Self::None => None,
                Self::ChannelData(chan) => {
                    Some(crate::channel::on_channel_data(stream, buf, count, chan, eof))
                }
                Self::ChannelStderr(chan) => {
                    Some(crate::channel::on_job_stderr(stream, buf, count, chan, eof))
                }
                Self::ChannelRpc(chan) => Some(crate::msgpack_rpc::channel::receive_msgpack(
                    stream, buf, count, chan, eof,
                )),
                Self::ShellBuf(dbuf) => Some(crate::os::shell::system_data_cb(
                    stream, buf, count, dbuf, eof,
                )),
                Self::ShellOut => Some(crate::os::shell::out_data_cb(stream, buf, count, eof)),
                Self::Input(input) => {
                    Some(crate::tui::input::tinput_read_cb(stream, buf, count, input, eof))
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum StreamInternalData {
    None,
    Channel(*mut crate::channel::Channel),
    Proc(*mut crate::event::proc::Proc),
    Closed(*mut bool),
}

impl StreamInternalData {
    pub const NONE: Self = Self::None;

    #[cfg(windows)]
    #[inline]
    pub const fn as_proc(self) -> *mut crate::event::proc::Proc {
        match self {
            Self::Proc(p) => p,
            _ => ptr::null_mut(),
        }
    }
}

#[repr(C, align(8))]
pub struct UvStreamHandle(pub [u8; 640]);

#[repr(C)]
pub struct Stream {
    pub closed: bool,
    pub uv: UvStreamHandle,
    pub uvstream: *mut uv_stream_t,
    pub fd: c_int,
    pub fpos: i64,
    pub cb_data: StreamCbData,
    pub is_rstream: bool,
    #[cfg(windows)]
    pub close_console: bool,
    pub internal_data: StreamInternalData,
    pub pending_reqs: usize,
    pub events: *mut MultiQueue,
    pub notify_write: bool,
    pub curmem: usize,
    pub maxmem: usize,
}

impl Stream {
    pub const fn new() -> Self {
        Self {
            closed: false,
            uv: UvStreamHandle([0; 640]),
            uvstream: ptr::null_mut(),
            fd: -1,
            fpos: 0,
            cb_data: StreamCbData::NONE,
            is_rstream: false,
            #[cfg(windows)]
            close_console: false,
            internal_data: StreamInternalData::NONE,
            pending_reqs: 0,
            events: ptr::null_mut(),
            notify_write: false,
            curmem: 0,
            maxmem: 0,
        }
    }
}

use crate::event::r#loop::{uv_idle_init, uv_stream_get_write_queue_size, uv_stream_set_blocking};

#[cfg(windows)]
use crate::os::os_win_console::{GetConsoleMode, SetConsoleMode};

pub unsafe fn stream_set_blocking(fd: c_int, blocking: bool) -> c_int {
    unsafe {
        let mut loop_buf = [0u64; 256];
        let mut pipe_buf = [0u64; 128];
        let loop_ptr = loop_buf.as_mut_ptr().cast::<uv_loop_t>();
        let pipe_ptr = pipe_buf.as_mut_ptr().cast::<uv_pipe_t>();
        uv_loop_init(loop_ptr);
        uv_pipe_init(loop_ptr, pipe_ptr, 0);
        uv_pipe_open(pipe_ptr, fd);
        let retval = uv_stream_set_blocking(pipe_ptr.cast(), c_int::from(blocking));
        uv_close(pipe_ptr.cast(), None);
        uv_run(loop_ptr, UV_RUN_NOWAIT);
        uv_loop_close(loop_ptr);
        retval
    }
}

pub unsafe fn stream_init(
    loop_: *mut Loop,
    stream: *mut Stream,
    fd: c_int,
    uvstream: *mut uv_stream_t,
) {
    unsafe {
        assert!(if uvstream.is_null() {
            fd >= 0 && !loop_.is_null()
        } else {
            fd < 0 && loop_.is_null()
        });
        let uv_ptr: *mut c_void = (&raw mut (*stream).uv).cast();
        (*stream).uvstream = uvstream;

        if fd >= 0 {
            let h_type = uv_guess_handle(fd);
            (*stream).fd = fd;

            if h_type == UV_FILE {
                uv_idle_init(loop_.cast(), uv_ptr.cast());
                *uv_ptr.cast::<*mut Stream>() = stream;
            } else {
                #[cfg(windows)]
                if h_type == UV_TTY {
                    uv_tty_init(loop_.cast(), uv_ptr.cast(), fd, 0);
                    uv_tty_set_mode(uv_ptr.cast(), UV_TTY_MODE_RAW);
                    let tty_handle = *uv_ptr
                        .cast::<u8>()
                        .add(uv_handle_size(UV_STREAM))
                        .cast::<*mut c_void>();
                    let mut dw_mode: u32 = 0;
                    if GetConsoleMode(tty_handle, &mut dw_mode) != 0 {
                        dw_mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
                        SetConsoleMode(tty_handle, dw_mode);
                    }
                    (*stream).uvstream = uv_ptr.cast();
                } else {
                    assert!(h_type == UV_NAMED_PIPE || h_type == UV_TTY);
                    uv_pipe_init(loop_.cast(), uv_ptr.cast(), 0);
                    uv_pipe_open(uv_ptr.cast(), fd);
                    (*stream).uvstream = uv_ptr.cast();
                }

                #[cfg(not(windows))]
                {
                    assert!(h_type == UV_NAMED_PIPE || h_type == UV_TTY);
                    uv_pipe_init(loop_.cast(), uv_ptr.cast(), 0);
                    uv_pipe_open(uv_ptr.cast(), fd);
                    (*stream).uvstream = uv_ptr.cast();
                }
            }
        }

        if !(*stream).uvstream.is_null() {
            *(*stream).uvstream.cast::<*mut Stream>() = stream;
        }

        (*stream).fpos = 0;
        (*stream).internal_data = StreamInternalData::NONE;
        (*stream).curmem = 0;
        (*stream).maxmem = 0;
        (*stream).pending_reqs = 0;
        (*stream).notify_write = false;
        (*stream).is_rstream = false;
        #[cfg(windows)]
        {
            (*stream).close_console = false;
        }
        (*stream).closed = false;
        (*stream).events = ptr::null_mut();
    }
}

pub unsafe fn stream_may_close(stream: *mut Stream) {
    unsafe {
        if (*stream).closed {
            return;
        }
        if g_min_log_level <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"stream_may_close".as_ptr(),
                94,
                true,
                c"closing Stream: %p".as_ptr(),
                stream,
            );
        }
        (*stream).closed = true;

        #[cfg(windows)]
        {
            if uv_guess_handle((*stream).fd) == UV_TTY {
                uv_tty_set_mode((&raw mut (*stream).uv).cast(), UV_TTY_MODE_NORMAL);
            }
        }

        if (*stream).pending_reqs == 0 {
            stream_close_handle(stream);
        }
    }
}

pub unsafe fn stream_close_handle(stream: *mut Stream) {
    unsafe {
        let handle: *mut uv_handle_t = if !(*stream).uvstream.is_null() {
            let queued = uv_stream_get_write_queue_size((*stream).uvstream);
            if queued > 0 {
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"stream_close_handle".as_ptr(),
                    116,
                    true,
                    c"closed Stream (%p) with %zu unwritten bytes".as_ptr(),
                    stream,
                    queued,
                );
            }
            (*stream).uvstream.cast()
        } else {
            (&raw mut (*stream).uv).cast()
        };

        assert!(!handle.is_null());

        #[cfg(windows)]
        if (*stream).close_console {
            (*stream).pending_reqs += 1;
            crate::os::pty_proc_win::pty_proc_close_console(stream);
            (*stream).pending_reqs -= 1;
        }
        if uv_is_closing(handle) == 0 {
            uv_close(handle, Some(close_cb));
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let stream = *handle.cast::<*mut Stream>();
        if !stream.is_null() {
            if (*stream).is_rstream {
                crate::event::rstream::rstream_close_cb(stream);
            }
            match (*stream).internal_data {
                StreamInternalData::None => {}
                StreamInternalData::Channel(chan) => crate::channel::channel_decref(chan),
                StreamInternalData::Proc(proc_) => crate::event::proc::proc_decref(proc_),
                StreamInternalData::Closed(closed_ptr) => *closed_ptr = true,
            }
        }
    }
}
