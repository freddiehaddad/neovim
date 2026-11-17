//! Libuv process spawning and its close-callback ownership.

use crate::channel::STDERR_FILENO;
use crate::eval::typval::tv_dict_to_env;
use crate::event::r#loop::{
    main_loop, uv_close, uv_gid_t, uv_handle_t, uv_pipe, uv_pipe_open, uv_process_get_pid,
    uv_spawn, uv_uid_t,
};
use crate::event::proc::Proc;
use crate::event::stream::{uv_pipe_t, uv_stream_t};
use crate::globals::*;
use crate::os::fs::os_close;
use std::ffi::{CString, c_void};
use std::ptr;

const UV_IGNORE: i32 = 0;
const UV_INHERIT_FD: i32 = 2;
const UV_NONBLOCK_PIPE: i32 = 0x40;
const UV_PROCESS_DETACHED: u32 = 1 << 3;
const UV_PROCESS_WINDOWS_HIDE: u32 = 1 << 4;

#[allow(non_camel_case_types)]
pub type uv_process_t = c_void;
#[allow(non_camel_case_types)]
type uv_exit_cb =
    Option<unsafe extern "C" fn(handle: *mut uv_process_t, exit_status: i64, term_signal: i32)>;

#[repr(C)]
#[derive(Clone, Copy)]
pub union UvStdioData {
    pub stream: *mut uv_stream_t,
    pub fd: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct uv_stdio_container_t {
    pub flags: i32,
    pub data: UvStdioData,
}

#[repr(C)]
pub struct uv_process_options_t {
    pub exit_cb: uv_exit_cb,
    pub file: *const u8,
    pub args: *mut *mut u8,
    pub env: *mut *mut u8,
    pub cwd: *const u8,
    pub flags: u32,
    pub stdio_count: i32,
    pub stdio: *mut uv_stdio_container_t,
    pub uid: uv_uid_t,
    pub gid: uv_gid_t,
}

#[cfg(test)]
pub type UvProcessHandle = crate::event::owned::RawHandle<40>;

pub struct LibuvProc {
    pub proc_: Proc,
}

impl LibuvProc {
    pub fn new() -> Self {
        Self { proc_: Proc::new() }
    }
}

pub fn proc_get_exepath(proc_: &Proc) -> CString {
    proc_
        .exepath
        .get()
        .unwrap_or_else(|| proc_.argv.get()[0].clone())
}

fn libuv_proc_stdio(
    proc_: &Proc,
    stdio: &mut [uv_stdio_container_t; 4],
    index: usize,
    child_readable: bool,
    overlapped: bool,
    win_create_pipe: bool,
    to_close: &mut [i32; 3],
) {
    let pipe: *mut uv_pipe_t = match index {
        0 => proc_.r#in.uv.as_ptr(),
        1 => proc_.out.uv.as_ptr(),
        2 => proc_.err.uv.as_ptr(),
        _ => unreachable!(),
    };
    #[cfg(windows)]
    if win_create_pipe {
        stdio[index].flags = 1 | if child_readable { 0x10 } else { 0x20 };
        if overlapped {
            stdio[index].flags |= 0x40 | 0x10;
        }
        stdio[index].data.stream = pipe;
        return;
    }
    #[cfg(not(windows))]
    let _ = (overlapped, win_create_pipe);
    let child_flags = if cfg!(windows) && child_readable && overlapped {
        UV_NONBLOCK_PIPE
    } else {
        0
    };
    let mut pipe_pair = [-1; 2];
    unsafe {
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
    }
    let (child, parent) = if child_readable {
        (pipe_pair[0], pipe_pair[1])
    } else {
        (pipe_pair[1], pipe_pair[0])
    };
    stdio[index].flags = UV_INHERIT_FD;
    stdio[index].data.fd = child;
    to_close[index] = child;
    unsafe { uv_pipe_open(pipe, parent) };
}

pub fn libuv_proc_spawn(proc_: &Proc) -> i32 {
    let argv = proc_.argv.get();
    #[cfg(windows)]
    let mut argv = argv;
    let mut flags = UV_PROCESS_WINDOWS_HIDE;
    #[cfg(windows)]
    {
        if crate::os::env::os_shell_is_cmdexe(&argv[0]) {
            flags |= 1 << 2;
            crate::memory::cstring_modify(&mut argv[0], |bytes| {
                crate::path::path_to_backslash(bytes);
            });
            proc_.argv.set(argv.clone());
        }
        if proc_.detach.get() {
            flags |= UV_PROCESS_DETACHED;
        }
    }
    #[cfg(not(windows))]
    {
        flags |= UV_PROCESS_DETACHED;
    }
    let executable = proc_get_exepath(proc_);
    let cwd = proc_.cwd.get();
    let env = proc_.env.get();
    let env_values = if env.is_null() {
        Vec::new()
    } else {
        tv_dict_to_env(env.opt())
    };
    let mut argv_ptrs: Vec<*mut u8> = argv
        .iter()
        .map(|s| s.as_ptr().cast_mut().cast())
        .chain(std::iter::once(ptr::null_mut()))
        .collect();
    let mut env_ptrs: Vec<*mut u8> = env_values
        .iter()
        .map(|s| s.as_ptr().cast_mut().cast())
        .chain(std::iter::once(ptr::null_mut()))
        .collect();
    let mut stdio = [uv_stdio_container_t {
        flags: UV_IGNORE,
        data: UvStdioData {
            stream: ptr::null_mut(),
        },
    }; 4];
    let mut options = uv_process_options_t {
        exit_cb: Some(exit_cb),
        file: executable.as_ptr().cast(),
        args: argv_ptrs.as_mut_ptr(),
        env: if env.is_null() {
            ptr::null_mut()
        } else {
            env_ptrs.as_mut_ptr()
        },
        cwd: cwd.as_ref().map_or(ptr::null(), |s| s.as_ptr().cast()),
        flags,
        stdio_count: 3,
        stdio: stdio.as_mut_ptr(),
        uid: 0,
        gid: 0,
    };
    if screen_state().ui_client_forward_stdin.get() {
        debug_assert_eq!(crate::ui_client::UI_CLIENT_STDIN_FD, 3);
        options.stdio_count = 4;
        stdio[3].data.fd = 0;
        stdio[3].flags = UV_INHERIT_FD;
    }
    let mut to_close = [-1; 3];
    let overlapped = proc_.overlapped.get();
    if !proc_.r#in.closed.get() {
        libuv_proc_stdio(
            proc_,
            &mut stdio,
            0,
            true,
            overlapped,
            proc_.stdio_noinherit.get(),
            &mut to_close,
        );
    }
    if !proc_.out.closed.get() {
        libuv_proc_stdio(proc_, &mut stdio, 1, false, overlapped, true, &mut to_close);
    }
    if !proc_.err.closed.get() {
        libuv_proc_stdio(
            proc_,
            &mut stdio,
            2,
            false,
            overlapped,
            proc_.stdio_noinherit.get(),
            &mut to_close,
        );
    } else if proc_.fwd_err.get() {
        stdio[2].flags = UV_INHERIT_FD;
        stdio[2].data.fd = STDERR_FILENO;
    }
    options.stdio = stdio.as_mut_ptr();
    let status = unsafe { uv_spawn(main_loop().uv_ptr(), proc_.uv.as_ptr(), &options) };
    unsafe { proc_.uv.register(proc_.owner()) };
    main_loop().register_handle(proc_.uv.as_ptr(), close_registered);
    if status == 0 {
        proc_
            .pid
            .set(unsafe { uv_process_get_pid(proc_.uv.as_ptr()) });
    }
    for fd in to_close {
        if fd >= 0 {
            os_close(fd);
        }
    }
    status
}

pub fn libuv_proc_close(proc_: &Proc) {
    if proc_.uv.begin_close() {
        unsafe { uv_close(proc_.uv.as_ptr(), Some(close_cb)) };
    }
}

unsafe fn close_registered(handle: *mut c_void) {
    libuv_proc_close(&unsafe { Proc::from_callback(handle) });
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    let proc_ = unsafe { Proc::from_callback(handle) };
    if proc_.spawned.get() {
        crate::event::proc::proc_decref(&proc_);
    }
    main_loop().unregister_handle(handle);
    drop(unsafe { proc_.uv.finish_close() });
}

unsafe extern "C" fn exit_cb(handle: *mut uv_process_t, status: i64, term_signal: i32) {
    let proc_ = unsafe { Proc::from_callback(handle) };
    let signal = if cfg!(windows) {
        i32::from(proc_.exit_signal.get())
    } else {
        term_signal
    };
    proc_.status.set(if signal != 0 {
        128 + signal
    } else {
        status as i32
    });
    crate::event::proc::on_proc_exit(&proc_);
}
