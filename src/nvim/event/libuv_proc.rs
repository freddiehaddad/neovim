//! Libuv process spawning (`src/nvim/event/libuv_proc.c`).

use crate::channel::STDERR_FILENO;
use crate::eval::typval::tv_dict_to_env;
use crate::event::r#loop::{uv_close, uv_gid_t, uv_handle_t, uv_pipe_open, uv_uid_t};
use crate::event::proc::Proc;
use crate::event::stream::{uv_pipe_t, uv_stream_t};
use crate::globals::*;
#[cfg(windows)]
use crate::os::env::os_shell_is_cmdexe;
use crate::os::fs::os_close;
#[cfg(windows)]
use crate::path::path_to_backslash;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

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

use crate::ui_client::UI_CLIENT_STDIN_FD;

#[allow(non_camel_case_types)]
pub type uv_process_t = c_void;
#[allow(non_camel_case_types)]
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

#[repr(C, align(8))]
pub struct UvProcessHandle(pub [u8; 320]);

#[repr(C)]
pub struct LibuvProc {
    pub proc_: Proc,
    pub uv: UvProcessHandle,
    pub uvopts: uv_process_options_t,
    pub uvstdio: [uv_stdio_container_t; 4],
    pub args_ptrs: Vec<*mut c_char>,
    pub env_vec: Vec<std::ffi::CString>,
    pub env_ptrs: Vec<*mut c_char>,
}

impl LibuvProc {
    pub const fn new() -> Self {
        Self {
            proc_: Proc::new(),
            uv: UvProcessHandle([0; 320]),
            uvopts: uv_process_options_t {
                exit_cb: None,
                file: ptr::null(),
                args: ptr::null_mut(),
                env: ptr::null_mut(),
                cwd: ptr::null(),
                flags: 0,
                stdio_count: 0,
                stdio: ptr::null_mut(),
                uid: 0,
                gid: 0,
            },
            uvstdio: [uv_stdio_container_t {
                flags: 0,
                data: UvStdioData {
                    stream: ptr::null_mut(),
                },
            }; 4],
            args_ptrs: Vec::new(),
            env_vec: Vec::new(),
            env_ptrs: Vec::new(),
        }
    }
}

use crate::event::r#loop::{uv_pipe, uv_process_get_pid, uv_spawn};

#[inline]
pub fn proc_get_exepath(proc_: &mut Proc) -> *const c_char {
    proc_
        .exepath
        .as_ref()
        .map_or_else(|| proc_.argv.as_slice()[0].as_ptr(), |s| s.as_ptr())
}

fn libuv_proc_stdio(
    uvproc: &mut LibuvProc,
    idx: usize,
    parent_pipe: *mut uv_pipe_t,
    child_readable: bool,
    overlapped: bool,
    win_create_pipe: bool,
    to_close: &mut [c_int; 3],
) {
    unsafe {
        let ltail = uvproc;
        #[cfg(windows)]
        if win_create_pipe {
            ltail.uvstdio[idx].flags = UV_CREATE_PIPE
                | if child_readable {
                    UV_READABLE_PIPE
                } else {
                    UV_WRITABLE_PIPE
                };
            if overlapped {
                ltail.uvstdio[idx].flags |= UV_OVERLAPPED_PIPE | UV_READABLE_PIPE;
            }
            ltail.uvstdio[idx].data.stream = parent_pipe.cast();
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
        ltail.uvstdio[idx].flags = UV_INHERIT_FD;
        ltail.uvstdio[idx].data.fd = child_fd;
        to_close[idx] = child_fd;
        uv_pipe_open(parent_pipe, parent_fd);
    }
}

/// @returns zero on success, or negative error code
pub fn libuv_proc_spawn(uvproc: &mut LibuvProc) -> c_int {
    unsafe {
        let proc_: *mut Proc = &raw mut uvproc.proc_;
        let proc_ = &mut *proc_;
        let uv: *mut uv_handle_t = (&raw mut uvproc.uv).cast();

        #[cfg(windows)]
        {
            if os_shell_is_cmdexe(proc_.argv.as_slice()[0].as_ptr()) {
                uvproc.uvopts.flags |= UV_PROCESS_WINDOWS_VERBATIM_ARGUMENTS;
                path_to_backslash(proc_.argv.as_slice()[0].as_ptr().cast_mut());
            }
            if proc_.detach {
                uvproc.uvopts.flags |= UV_PROCESS_DETACHED;
            }
        }
        #[cfg(not(windows))]
        {
            uvproc.uvopts.flags |= UV_PROCESS_DETACHED;
        }

        uvproc.args_ptrs = proc_
            .argv
            .iter()
            .map(|s| s.as_ptr().cast_mut())
            .chain(std::iter::once(ptr::null_mut()))
            .collect();
        uvproc.uvopts.file = proc_get_exepath(&mut *proc_);
        uvproc.uvopts.args = uvproc.args_ptrs.as_mut_ptr();
        uvproc.uvopts.flags |= UV_PROCESS_WINDOWS_HIDE;

        uvproc.uvopts.exit_cb = Some(exit_cb);
        uvproc.uvopts.cwd = proc_.cwd.as_ref().map_or(ptr::null(), |s| s.as_ptr());

        uvproc.uvopts.stdio = uvproc.uvstdio.as_mut_ptr();
        uvproc.uvopts.stdio_count = 3;
        uvproc.uvstdio[0].flags = UV_IGNORE;
        uvproc.uvstdio[1].flags = UV_IGNORE;
        uvproc.uvstdio[2].flags = UV_IGNORE;

        if g_st().ui_client_forward_stdin {
            debug_assert_eq!(UI_CLIENT_STDIN_FD, 3);
            uvproc.uvopts.stdio_count = 4;
            uvproc.uvstdio[3].data.fd = 0;
            uvproc.uvstdio[3].flags = UV_INHERIT_FD;
        }
        *uv.cast::<*mut Proc>() = proc_;

        if !proc_.env.is_null() {
            uvproc.env_vec = tv_dict_to_env((proc_.env).as_ref());
            uvproc.env_ptrs = uvproc
                .env_vec
                .iter()
                .map(|s| s.as_ptr().cast_mut())
                .chain(std::iter::once(ptr::null_mut()))
                .collect();
            uvproc.uvopts.env = uvproc.env_ptrs.as_mut_ptr();
        } else {
            uvproc.uvopts.env = ptr::null_mut();
        }

        let mut to_close = [-1 as c_int; 3];

        if !proc_.r#in.closed {
            libuv_proc_stdio(
                &mut *uvproc,
                0,
                (&raw mut proc_.r#in.uv).cast(),
                true,
                proc_.overlapped,
                proc_.stdio_noinherit,
                &mut to_close,
            );
        }

        if !proc_.out.s.closed {
            libuv_proc_stdio(
                &mut *uvproc,
                1,
                (&raw mut proc_.out.s.uv).cast(),
                false,
                proc_.overlapped,
                true,
                &mut to_close,
            );
        }

        if !proc_.err.s.closed {
            libuv_proc_stdio(
                &mut *uvproc,
                2,
                (&raw mut proc_.err.s.uv).cast(),
                false,
                proc_.overlapped,
                proc_.stdio_noinherit,
                &mut to_close,
            );
        } else if proc_.fwd_err {
            uvproc.uvstdio[2].flags = UV_INHERIT_FD;
            uvproc.uvstdio[2].data.fd = STDERR_FILENO;
        }

        let status = uv_spawn(proc_.loop_.cast(), uv.cast(), &raw const uvproc.uvopts);
        if status != 0 {
            uvproc.env_ptrs.clear();
            uvproc.env_vec.clear();
            uvproc.uvopts.env = ptr::null_mut();
        } else {
            proc_.pid = uv_process_get_pid(uv.cast());
        }

        for fd in to_close {
            if fd > -1 {
                os_close(fd);
            }
        }
        status
    }
}

pub fn libuv_proc_close(uvproc: &mut LibuvProc) {
    unsafe {
        uv_close((&raw mut uvproc.uv).cast(), Some(close_cb));
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let proc_ = *handle.cast::<*mut Proc>();
        crate::event::proc::proc_decref(&mut *proc_);
        let uvproc = proc_.cast::<LibuvProc>();
        (*uvproc).env_ptrs.clear();
        (*uvproc).env_vec.clear();
        (*uvproc).uvopts.env = ptr::null_mut();
    }
}

unsafe extern "C" fn exit_cb(handle: *mut uv_process_t, status: i64, term_signal: c_int) {
    unsafe {
        let proc_ = *handle.cast::<*mut Proc>();
        #[cfg(windows)]
        let sig = {
            let _ = term_signal;
            c_int::from((*proc_).exit_signal)
        };
        #[cfg(not(windows))]
        let sig = term_signal;
        (*proc_).status = if sig != 0 { 128 + sig } else { status as c_int };
        crate::event::proc::on_proc_exit(&mut *proc_);
    }
}
