//! Windows PTY process spawning (`src/nvim/os/pty_proc_win.c`).

#[cfg(windows)]
use crate::eval::typval::{TvDict, tv_dict_to_env};
#[cfg(windows)]
use crate::event::r#loop::{Loop, loop_schedule_fast, uv_req_size, uv_run, uv_translate_sys_error};
#[cfg(windows)]
use crate::log::logmsg;
#[cfg(windows)]
use crate::mbyte::utf8_to_utf16;
#[cfg(windows)]
use crate::os::env::os_shell_is_cmdexe;
#[cfg(windows)]
use crate::os::fs::os_can_exe;
#[cfg(windows)]
use std::ffi::{CStr, c_void};
#[cfg(windows)]
use std::ptr;

#[cfg(windows)]
use crate::event::multiqueue::Event;
#[cfg(windows)]
use crate::event::proc::{Proc, PtyProc};
#[cfg(windows)]
use crate::event::stream::{Stream, uv_stream_t};
#[cfg(windows)]
use crate::os::pty_conpty_win::{
    Conpty, os_conpty_init, os_conpty_set_size, os_conpty_spawn, os_has_conpty_working,
};

#[cfg(any(test, windows))]
/// Emulate quote_cmd_arg of libuv and quotes command line argument.
pub(crate) fn quote_cmd_arg(dest: &mut Vec<u8>, src_bytes: &[u8]) {
    if src_bytes.is_empty() {
        dest.extend_from_slice(b"\"\"");
        return;
    }

    if !src_bytes
        .iter()
        .any(|&b| b == b' ' || b == b'\t' || b == b'"')
    {
        dest.extend_from_slice(src_bytes);
        return;
    }

    if !src_bytes.iter().any(|&b| b == b'"' || b == b'\\') {
        dest.push(b'"');
        dest.extend_from_slice(src_bytes);
        dest.push(b'"');
        return;
    }

    let start = dest.len();
    let mut quote_hit = true;
    dest.push(b'"');
    for &ch in src_bytes.iter().rev() {
        dest.push(ch);
        if quote_hit && ch == b'\\' {
            dest.push(b'\\');
        } else if ch == b'"' {
            quote_hit = true;
            dest.push(b'\\');
        } else {
            quote_hit = false;
        }
    }
    dest.push(b'"');
    dest[start..].reverse();
}

#[cfg(windows)]
mod win_impl {
    use super::*;
    use crate::event::r#loop::{uv_connect_t, uv_pipe_connect, uv_thread_t};
    use crate::os::os_win_console::{
        BOOL, CloseHandle, DWORD, GetLastError, GetProcessId, HANDLE, TerminateProcess,
    };

    type BOOLEAN = u8;
    use crate::event::r#loop::{uv_thread_create, uv_thread_detach};
    type WaitOrTimerCallback =
        Option<unsafe extern "system" fn(lp_parameter: *mut c_void, timer_or_wait_fired: BOOLEAN)>;

    use crate::event::r#loop::{UV_REQ, UV_RUN_ONCE};
    use crate::log::LOGLVL_ERR;
    use crate::os::fs::UV_ENOENT;

    const UV_CONNECT: i32 = 2;
    const UV_ENOSYS: i32 = -4054;

    const WAIT_TIMEOUT: DWORD = 258;
    const INFINITE: DWORD = 0xFFFF_FFFF;
    const WT_EXECUTEDEFAULT: DWORD = 0x0000_0000;
    const WT_EXECUTEONLYONCE: DWORD = 0x0000_0008;

    unsafe extern "system" {
        fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;
        fn RegisterWaitForSingleObject(
            phNewWaitObject: *mut HANDLE,
            hObject: HANDLE,
            Callback: WaitOrTimerCallback,
            Context: *mut c_void,
            dwMilliseconds: DWORD,
            dwFlags: DWORD,
        ) -> BOOL;
        fn UnregisterWaitEx(WaitHandle: HANDLE, CompletionEvent: HANDLE) -> BOOL;
        fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: &mut DWORD) -> BOOL;
    }

    #[inline]
    fn pty_proc_win_tail(ptyproc: &mut PtyProc) -> *mut PtyProc {
        ptyproc
    }

    #[inline]
    fn connect_req_handle_ptr(req: *mut uv_connect_t) -> *mut *mut uv_stream_t {
        req.cast::<u8>()
            .wrapping_add(unsafe { uv_req_size(UV_REQ) } + size_of::<*mut c_void>())
            .cast::<*mut uv_stream_t>()
    }

    unsafe extern "C" fn os_conpty_free_thread(arg: *mut c_void) {
        unsafe {
            if !arg.is_null() {
                drop(Box::from_raw(arg.cast::<Conpty>()));
            }
        }
    }

    unsafe extern "system" fn pty_proc_terminate_cb(context: *mut c_void, _unused: BOOLEAN) {
        unsafe {
            let proc_: *mut Proc = context.cast();
            loop_schedule_fast(&mut *(*proc_).loop_, Event::PtyProcFinish(proc_));
        }
    }

    pub(crate) fn pty_proc_close_console(stream: *mut Stream) {
        unsafe {
            let ptyproc = (*stream).internal_data.as_proc();
            let proc_: *mut Proc = ptyproc;
            let hdr = proc_;
            let r_out = &raw mut (*proc_).out;
            let r_fields = r_out;
            let ptail = pty_proc_win_tail(&mut *ptyproc);
            let Some(conpty) = (*ptail).conpty.take() else {
                return;
            };
            if (*r_fields).num_bytes == 0 {
                TerminateProcess((*ptail).proc_handle, 0);
            }
            let mut tid: uv_thread_t = 0;
            uv_thread_create(
                &raw mut tid,
                Some(os_conpty_free_thread),
                Box::into_raw(conpty).cast(),
            );
            while !ptr::read_volatile(&raw const (*r_fields).did_eof)
                && WaitForSingleObject(tid as HANDLE, 0) == WAIT_TIMEOUT
            {
                uv_run((*hdr).loop_.cast(), UV_RUN_ONCE);
            }
            uv_thread_detach(&raw mut tid);
        }
    }

    /// @returns zero on success, or negative error code.
    pub fn pty_proc_spawn(ptyproc: &mut PtyProc) -> i32 {
        unsafe {
            let proc_: *mut Proc = ptyproc;
            let hdr = proc_;
            let hdr = &mut *hdr;
            let ptail = pty_proc_win_tail(ptyproc);
            let ptail = &mut *ptail;
            let s_in = &mut (*proc_).r#in;
            let r_out = &mut (*proc_).out;
            let r_err = &mut (*proc_).err;
            let mut status: i32;
            let mut conpty_object: Option<Box<Conpty>> = None;
            let mut in_name: Option<std::ffi::CString> = None;
            let mut out_name: Option<std::ffi::CString> = None;
            let mut proc_handle: HANDLE = ptr::null_mut();
            let mut in_req_buf: Option<Vec<u64>> = None;
            let mut out_req_buf: Option<Vec<u64>> = None;
            let mut in_req: *mut uv_connect_t = ptr::null_mut();
            let mut out_req: *mut uv_connect_t = ptr::null_mut();
            let mut file: Vec<u16> = Vec::new();
            let mut cmd_line: Vec<u16> = Vec::new();
            let mut cwd: Vec<u16> = Vec::new();
            let mut env: Vec<u16> = Vec::new();
            let mut emsg: Option<&CStr> = None;

            debug_assert!(r_err.s.closed);

            'cleanup: {
                if !os_has_conpty_working() {
                    status = UV_ENOSYS;
                    break 'cleanup;
                }
                conpty_object =
                    os_conpty_init(&mut in_name, &mut out_name, ptail.width, ptail.height);
                if conpty_object.is_none() {
                    status = UV_ENOSYS;
                    break 'cleanup;
                }

                if !s_in.closed {
                    let mut buf = vec![0u64; uv_req_size(UV_CONNECT).div_ceil(8)];
                    in_req = buf.as_mut_ptr().cast();
                    in_req_buf = Some(buf);
                    uv_pipe_connect(
                        in_req,
                        (&raw mut s_in.uv).cast(),
                        in_name.as_ref().unwrap().as_ptr(),
                        Some(pty_proc_connect_cb),
                    );
                }

                if !r_out.s.closed {
                    let mut buf = vec![0u64; uv_req_size(UV_CONNECT).div_ceil(8)];
                    out_req = buf.as_mut_ptr().cast();
                    out_req_buf = Some(buf);
                    uv_pipe_connect(
                        out_req,
                        (&raw mut r_out.s.uv).cast(),
                        out_name.as_ref().unwrap().as_ptr(),
                        Some(pty_proc_connect_cb),
                    );
                }

                if let Some(ref hdr_cwd) = hdr.cwd {
                    status = utf8_to_utf16(hdr_cwd.as_ptr(), -1, &mut cwd);
                    if status != 0 {
                        emsg = Some(c"utf8_to_utf16(proc->cwd) failed");
                        break 'cleanup;
                    }
                }

                let arg0 = hdr.argv.as_slice()[0].as_ptr();
                let is_cmdexe = os_shell_is_cmdexe(arg0);
                if is_cmdexe {
                    let mut path: Option<std::ffi::CString> = None;
                    if !os_can_exe(arg0, Some(&mut path), true) {
                        status = UV_ENOENT;
                        emsg = Some(c"executable not found");
                        break 'cleanup;
                    }
                    status = utf8_to_utf16(
                        path.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                        -1,
                        &mut file,
                    );
                    if status != 0 {
                        emsg = Some(c"utf8_to_utf16(proc->argv[0]) failed");
                        break 'cleanup;
                    }
                }
                status = build_cmd_line(hdr.argv.as_slice(), &mut cmd_line, is_cmdexe);
                if status != 0 {
                    emsg = Some(c"build_cmd_line failed");
                    break 'cleanup;
                }

                if !hdr.env.is_null() {
                    status = build_env_block((hdr.env).as_ref(), &mut env);
                }

                if status != 0 {
                    emsg = Some(c"build_env_block failed");
                    break 'cleanup;
                }

                if !os_conpty_spawn(
                    conpty_object.as_deref_mut().unwrap(),
                    &mut proc_handle,
                    if file.is_empty() {
                        ptr::null_mut()
                    } else {
                        file.as_mut_ptr()
                    },
                    cmd_line.as_mut_ptr(),
                    if cwd.is_empty() {
                        ptr::null_mut()
                    } else {
                        cwd.as_mut_ptr()
                    },
                    if env.is_empty() {
                        ptr::null_mut()
                    } else {
                        env.as_mut_ptr()
                    },
                ) {
                    emsg = Some(c"os_conpty_spawn failed");
                    status = GetLastError() as i32;
                    break 'cleanup;
                }
                hdr.pid = GetProcessId(proc_handle) as i32;

                if RegisterWaitForSingleObject(
                    &raw mut ptail.finish_wait,
                    proc_handle,
                    Some(pty_proc_terminate_cb),
                    (&raw mut *ptyproc).cast(),
                    INFINITE,
                    WT_EXECUTEDEFAULT | WT_EXECUTEONLYONCE,
                ) == 0
                {
                    std::process::abort();
                }

                while (!in_req.is_null()
                    && !ptr::read_volatile(connect_req_handle_ptr(in_req)).is_null())
                    || (!out_req.is_null()
                        && !ptr::read_volatile(connect_req_handle_ptr(out_req)).is_null())
                {
                    uv_run(hdr.loop_.cast(), UV_RUN_ONCE);
                }

                r_out.s.close_console = true;
                ptail.conpty = conpty_object.take();
                ptail.proc_handle = proc_handle;
                proc_handle = ptr::null_mut();
            }

            if status != 0 {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"pty_proc_spawn",
                    line!() as i32,
                    true,
                    c"pty_proc_spawn(%s): %s: error code: %d",
                    hdr.argv.as_slice()[0].as_ptr(),
                    emsg.map_or(ptr::null(), CStr::as_ptr),
                    status,
                );
                status = uv_translate_sys_error(status);
            }
            drop(conpty_object);
            drop(in_name);
            drop(out_name);
            if !proc_handle.is_null() {
                CloseHandle(proc_handle);
            }
            drop(in_req_buf);
            drop(out_req_buf);
            status
        }
    }

    pub fn pty_proc_tty_name(_ptyproc: &mut PtyProc) -> Option<&CStr> {
        Some(c"?")
    }

    pub fn pty_proc_resize(ptyproc: &mut PtyProc, width: u16, height: u16) {
        unsafe {
            let ptail = pty_proc_win_tail(ptyproc);
            if let Some(ref conpty) = (*ptail).conpty {
                os_conpty_set_size(conpty, width, height);
            }
        }
    }

    pub fn pty_proc_resume(_ptyproc: &mut PtyProc) {}

    pub fn pty_proc_flush_master(_ptyproc: &mut PtyProc) {}

    pub fn pty_proc_close(ptyproc: &mut PtyProc) {
        let proc_: *mut Proc = ptyproc;
        let proc_t = proc_;
        let ptail = pty_proc_win_tail(ptyproc);
        let ptail = unsafe { &mut *ptail };

        pty_proc_close_master(ptyproc);

        if !ptail.finish_wait.is_null() {
            unsafe { UnregisterWaitEx(ptail.finish_wait, ptr::null_mut()) };
            ptail.finish_wait = ptr::null_mut();
        }
        if !ptail.proc_handle.is_null() {
            unsafe { CloseHandle(ptail.proc_handle) };
            ptail.proc_handle = ptr::null_mut();
        }

        let _ = proc_t;
        crate::event::proc::proc_decref(unsafe { &mut *proc_ });
    }

    pub fn pty_proc_close_master(_ptyproc: &mut PtyProc) {}

    pub fn pty_proc_teardown(_loop_: &mut Loop) {}

    unsafe extern "C" fn pty_proc_connect_cb(req: *mut uv_connect_t, status: i32) {
        unsafe {
            debug_assert_eq!(status, 0);
            {
                ptr::write_volatile(connect_req_handle_ptr(req), ptr::null_mut());
            }
        }
    }

    pub fn pty_proc_finish(ptyproc: &mut PtyProc) {
        unsafe {
            let proc_: *mut Proc = ptyproc;
            let hdr = proc_;
            let hdr = &mut *hdr;
            let ptail = pty_proc_win_tail(ptyproc);

            let mut exit_code: DWORD = 0;
            GetExitCodeProcess((*ptail).proc_handle, &mut exit_code);
            hdr.status = if hdr.exit_signal != 0 {
                128 + i32::from(hdr.exit_signal)
            } else {
                exit_code as i32
            };

            crate::event::proc::on_proc_exit(&mut *proc_);
        }
    }

    fn build_cmd_line(argv: &[std::ffi::CString], cmd_line: &mut Vec<u16>, is_cmdexe: bool) -> i32 {
        let mut utf8_cmd_line: Vec<u8> = Vec::new();

        for (argc, arg_cstr) in argv.iter().enumerate() {
            if argc > 0 {
                utf8_cmd_line.push(b' ');
            }
            if is_cmdexe {
                let src = if argc == 0 {
                    b"cmd.exe" as &[u8]
                } else {
                    arg_cstr.as_bytes()
                };
                utf8_cmd_line.extend_from_slice(src);
            } else {
                quote_cmd_arg(&mut utf8_cmd_line, arg_cstr.as_bytes());
            }
        }
        utf8_cmd_line.push(0);

        utf8_to_utf16(utf8_cmd_line.as_ptr().cast(), -1, cmd_line)
    }

    fn build_env_block(denv: Option<&TvDict>, env_block: &mut Vec<u16>) -> i32 {
        env_block.clear();
        let mut rc: i32 = 0;
        let env = tv_dict_to_env(denv);
        let mut str_utf16: Vec<u16> = Vec::new();

        for entry in &env {
            rc = utf8_to_utf16(entry.as_ptr(), -1, &mut str_utf16);
            if rc != 0 {
                env_block.clear();
                break;
            }
            let len = str_utf16
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(str_utf16.len())
                + 1;
            env_block.extend_from_slice(&str_utf16[..len]);
        }
        if rc == 0 {
            env_block.push(0);
        }

        rc
    }
}

#[cfg(windows)]
pub use win_impl::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_cmd_arg_matches_expected_windows_escaping() {
        let cases = [
            ("", "\"\""),
            ("plain", "plain"),
            ("hello world", "\"hello world\""),
            ("hello\"world", "\"hello\\\"world\""),
            ("hello\"\"world", "\"hello\\\"\\\"world\""),
            ("hello\\world", "hello\\world"),
            ("hello\\\\world", "hello\\\\world"),
            ("hello\\\"world", "\"hello\\\\\\\"world\""),
            ("hello\\\\\"world", "\"hello\\\\\\\\\\\"world\""),
            ("hello world\\", "\"hello world\\\\\""),
        ];
        for (input, expected) in cases {
            let mut out = Vec::new();
            quote_cmd_arg(&mut out, input.as_bytes());
            let got = std::str::from_utf8(&out).unwrap();
            assert_eq!(got, expected, "failed for input {input:?}");
        }
    }
}
