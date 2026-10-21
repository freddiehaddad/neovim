//! Libuv process spawning (`src/nvim/event/libuv_proc.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::channel::STDERR_FILENO;
use crate::eval::typval::tv_dict_to_env;
use crate::event::r#loop::{
    uv_close, uv_gid_t, uv_handle_size, uv_handle_t, uv_loop_t, uv_pipe_open, uv_uid_t,
};
use crate::event::proc::{
    LibuvProc, Proc, libuv_proc_uv, proc_err, proc_header, proc_in, proc_out, proc_size, proc_tail,
};
use crate::event::stream::{stream_closed_ptr, stream_uv_ptr, uv_pipe_t, uv_stream_t};
use crate::globals::*;
use crate::os::env::os_free_fullenv;
#[cfg(windows)]
use crate::os::env::os_shell_is_cmdexe;
use crate::os::fs::os_close;
#[cfg(windows)]
use crate::path::path_to_backslash;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

pub(crate) const UV_PROCESS: c_int = 10;

const UV_IGNORE: c_int = 0x00;
#[cfg(windows)]
const UV_CREATE_PIPE: c_int = 0x01;
const UV_INHERIT_FD: c_int = 0x02;
#[cfg(windows)]
const UV_READABLE_PIPE: c_int = 0x10;
#[cfg(windows)]
const UV_WRITABLE_PIPE: c_int = 0x20;
const UV_NONBLOCK_PIPE: c_int = 0x40;
#[cfg(windows)]
const UV_OVERLAPPED_PIPE: c_int = 0x40;

#[cfg(windows)]
const UV_PROCESS_WINDOWS_VERBATIM_ARGUMENTS: c_uint = 1 << 2;
const UV_PROCESS_DETACHED: c_uint = 1 << 3;
const UV_PROCESS_WINDOWS_HIDE: c_uint = 1 << 4;

const UI_CLIENT_STDIN_FD: c_int = 3;

pub type uv_process_t = c_void;
type uv_exit_cb =
    Option<unsafe extern "C" fn(handle: *mut uv_process_t, exit_status: i64, term_signal: c_int)>;

#[repr(C)]
#[derive(Clone, Copy)]
pub union UvStdioData {
    pub stream: *mut uv_stream_t,
    pub fd: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct uv_stdio_container_t {
    pub flags: c_int,
    pub data: UvStdioData,
}

#[repr(C)]
pub struct uv_process_options_t {
    pub exit_cb: uv_exit_cb,
    pub file: *const c_char,
    pub args: *mut *mut c_char,
    pub env: *mut *mut c_char,
    pub cwd: *const c_char,
    pub flags: c_uint,
    pub stdio_count: c_int,
    pub stdio: *mut uv_stdio_container_t,
    pub uid: uv_uid_t,
    pub gid: uv_gid_t,
}

#[repr(C)]
pub(crate) struct LibuvProcTail {
    pub(crate) uvopts: uv_process_options_t,
    pub(crate) uvstdio: [uv_stdio_container_t; 4],
}

unsafe extern "C" {

    fn uv_pipe(fds: *mut c_int, read_flags: c_int, write_flags: c_int) -> c_int;
    fn uv_spawn(
        loop_: *mut uv_loop_t,
        handle: *mut uv_process_t,
        options: *const uv_process_options_t,
    ) -> c_int;
    fn uv_process_get_pid(handle: *const uv_process_t) -> c_int;

}

#[inline]
unsafe fn libuv_proc_tail(uvproc: *mut LibuvProc) -> *mut LibuvProcTail {
    unsafe {
        uvproc
            .cast::<u8>()
            .add(proc_size() + uv_handle_size(UV_PROCESS))
            .cast::<LibuvProcTail>()
    }
}

#[inline]
pub unsafe fn proc_get_exepath(proc_: *mut Proc) -> *const c_char {
    unsafe {
        let hdr = proc_header(proc_);
        if !(*hdr).exepath.is_null() {
            (*hdr).exepath
        } else {
            *(*hdr).argv
        }
    }
}

unsafe fn libuv_proc_stdio(
    uvproc: *mut LibuvProc,
    idx: usize,
    parent_pipe: *mut uv_pipe_t,
    child_readable: bool,
    overlapped: bool,
    win_create_pipe: bool,
    to_close: &mut [c_int; 3],
) {
    unsafe {
        let ltail = libuv_proc_tail(uvproc);
        #[cfg(windows)]
        if win_create_pipe {
            (*ltail).uvstdio[idx].flags = UV_CREATE_PIPE
                | if child_readable {
                    UV_READABLE_PIPE
                } else {
                    UV_WRITABLE_PIPE
                };
            if overlapped {
                (*ltail).uvstdio[idx].flags |= UV_OVERLAPPED_PIPE | UV_READABLE_PIPE;
            }
            (*ltail).uvstdio[idx].data.stream = parent_pipe.cast();
            return;
        }
        #[cfg(not(windows))]
        let _ = (overlapped, win_create_pipe);

        #[cfg(windows)]
        let child_flags = if child_readable && overlapped {
            UV_NONBLOCK_PIPE
        } else {
            0
        };
        #[cfg(not(windows))]
        let child_flags = 0;

        let mut pipe_pair = [-1 as c_int; 2];
        uv_pipe(
            pipe_pair.as_mut_ptr(),
            if child_readable {
                child_flags
            } else {
                UV_NONBLOCK_PIPE
            },
            if child_readable {
                UV_NONBLOCK_PIPE
            } else {
                child_flags
            },
        );

        let child_fd = if child_readable {
            pipe_pair[0]
        } else {
            pipe_pair[1]
        };
        let parent_fd = if child_readable {
            pipe_pair[1]
        } else {
            pipe_pair[0]
        };
        (*ltail).uvstdio[idx].flags = UV_INHERIT_FD;
        (*ltail).uvstdio[idx].data.fd = child_fd;
        to_close[idx] = child_fd;
        uv_pipe_open(parent_pipe, parent_fd);
    }
}

/// @returns zero on success, or negative error code
#[unsafe(no_mangle)]
pub unsafe extern "C" fn libuv_proc_spawn(uvproc: *mut LibuvProc) -> c_int {
    unsafe {
        let proc_: *mut Proc = uvproc.cast();
        let hdr = proc_header(proc_);
        let ptail = proc_tail(proc_);
        let uv = libuv_proc_uv(proc_);
        let ltail = libuv_proc_tail(uvproc);

        (*ltail).uvopts.file = proc_get_exepath(proc_);
        (*ltail).uvopts.args = (*hdr).argv;
        (*ltail).uvopts.flags = UV_PROCESS_WINDOWS_HIDE;

        #[cfg(windows)]
        {
            if os_shell_is_cmdexe(*(*hdr).argv) {
                (*ltail).uvopts.flags |= UV_PROCESS_WINDOWS_VERBATIM_ARGUMENTS;
                path_to_backslash(*(*hdr).argv);
            }
            if (*ptail).detach {
                (*ltail).uvopts.flags |= UV_PROCESS_DETACHED;
            }
        }
        #[cfg(not(windows))]
        {
            (*ltail).uvopts.flags |= UV_PROCESS_DETACHED;
        }

        (*ltail).uvopts.exit_cb = Some(exit_cb);
        (*ltail).uvopts.cwd = (*hdr).cwd;

        (*ltail).uvopts.stdio = (*ltail).uvstdio.as_mut_ptr();
        (*ltail).uvopts.stdio_count = 3;
        (*ltail).uvstdio[0].flags = UV_IGNORE;
        (*ltail).uvstdio[1].flags = UV_IGNORE;
        (*ltail).uvstdio[2].flags = UV_IGNORE;

        if ui_client_forward_stdin {
            debug_assert_eq!(UI_CLIENT_STDIN_FD, 3);
            (*ltail).uvopts.stdio_count = 4;
            (*ltail).uvstdio[3].data.fd = 0;
            (*ltail).uvstdio[3].flags = UV_INHERIT_FD;
        }
        *uv.cast::<*mut Proc>() = proc_;

        if !(*hdr).env.is_null() {
            (*ltail).uvopts.env = tv_dict_to_env((*hdr).env);
        } else {
            (*ltail).uvopts.env = ptr::null_mut();
        }

        let mut to_close = [-1 as c_int; 3];

        let s_in = proc_in(proc_);
        let r_out = proc_out(proc_);
        let r_err = proc_err(proc_);

        if !*stream_closed_ptr(s_in) {
            libuv_proc_stdio(
                uvproc,
                0,
                stream_uv_ptr(s_in).cast(),
                true,
                (*ptail).overlapped,
                (*ptail).stdio_noinherit,
                &mut to_close,
            );
        }

        if !*stream_closed_ptr(r_out.cast()) {
            libuv_proc_stdio(
                uvproc,
                1,
                stream_uv_ptr(r_out.cast()).cast(),
                false,
                (*ptail).overlapped,
                true,
                &mut to_close,
            );
        }

        if !*stream_closed_ptr(r_err.cast()) {
            libuv_proc_stdio(
                uvproc,
                2,
                stream_uv_ptr(r_err.cast()).cast(),
                false,
                (*ptail).overlapped,
                (*ptail).stdio_noinherit,
                &mut to_close,
            );
        } else if (*ptail).fwd_err {
            (*ltail).uvstdio[2].flags = UV_INHERIT_FD;
            (*ltail).uvstdio[2].data.fd = STDERR_FILENO;
        }

        let status = uv_spawn((*hdr).loop_.cast(), uv.cast(), &raw const (*ltail).uvopts);
        if status != 0 {
            if !(*ltail).uvopts.env.is_null() {
                os_free_fullenv((*ltail).uvopts.env);
            }
        } else {
            (*hdr).pid = uv_process_get_pid(uv.cast());
        }

        for fd in to_close {
            if fd > -1 {
                os_close(fd);
            }
        }
        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn libuv_proc_close(uvproc: *mut LibuvProc) {
    unsafe {
        uv_close(libuv_proc_uv(uvproc.cast()), Some(close_cb));
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let proc_ = *handle.cast::<*mut Proc>();
        let ptail = proc_tail(proc_);
        if let Some(cb) = (*ptail).internal_close_cb {
            cb(proc_);
        }
        let ltail = libuv_proc_tail(proc_.cast());
        if !(*ltail).uvopts.env.is_null() {
            os_free_fullenv((*ltail).uvopts.env);
        }
    }
}

unsafe extern "C" fn exit_cb(handle: *mut uv_process_t, status: i64, term_signal: c_int) {
    unsafe {
        let proc_ = *handle.cast::<*mut Proc>();
        let hdr = proc_header(proc_);
        let ptail = proc_tail(proc_);
        #[cfg(windows)]
        let sig = {
            let _ = term_signal;
            c_int::from((*hdr).exit_signal)
        };
        #[cfg(not(windows))]
        let sig = term_signal;
        (*hdr).status = if sig != 0 { 128 + sig } else { status as c_int };
        if let Some(cb) = (*ptail).internal_exit_cb {
            cb(proc_);
        }
    }
}
