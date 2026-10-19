//! Windows PTY process spawning (`src/nvim/os/pty_proc_win.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::upper_case_acronyms
)]

#[allow(unused_imports)]
use crate::eval::typval::tv_dict_to_env;
#[allow(unused_imports)]
use crate::event::r#loop::{uv_req_size, uv_run, uv_translate_sys_error};
use crate::log::logmsg;
#[cfg(windows)]
#[allow(unused_imports)]
use crate::mbyte::utf8_to_utf16;
#[allow(unused_imports)]
use crate::os::env::os_shell_is_cmdexe;
#[allow(unused_imports)]
use crate::os::fs::os_can_exe;
#[allow(unused_imports)]
use crate::strings::wcslen;
use std::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use std::ptr;

#[cfg(windows)]
use crate::eval::decode::dict_T;
#[cfg(windows)]
use crate::event::r#loop::{Loop, loop_schedule_fast};
#[cfg(windows)]
use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event};
#[cfg(windows)]
use crate::event::proc::{
    Proc, PtyProc, proc_err, proc_header, proc_in, proc_out, proc_size, proc_tail,
};
#[cfg(windows)]
use crate::event::rstream::rstream_fields;
#[cfg(windows)]
use crate::event::stream::{
    Stream, stream_closed_ptr, stream_fields, stream_uv_ptr, uv_loop_t, uv_pipe_t, uv_stream_t,
};
use crate::memory::{xfree, xmalloc, xstrlcat, xstrlcpy};
#[cfg(windows)]
use crate::os::pty_conpty_win::{
    conpty_t, os_conpty_free, os_conpty_init, os_conpty_set_size, os_conpty_spawn,
    os_has_conpty_working,
};

const NUL: c_char = 0;

#[inline]
unsafe fn c_strlen(s: *const c_char) -> usize {
    unsafe { CStr::from_ptr(s).to_bytes().len() }
}

/// Emulate quote_cmd_arg of libuv and quotes command line argument.
pub(crate) unsafe fn quote_cmd_arg(
    dest: *mut c_char,
    mut dest_remaining: usize,
    src: *const c_char,
) {
    unsafe {
        let src_bytes = CStr::from_ptr(src).to_bytes();
        let src_len = src_bytes.len();
        let mut quote_hit = true;
        let mut start = dest;
        let mut cur = dest;

        if src_len == 0 {
            xstrlcpy(dest, c"\"\"".as_ptr(), dest_remaining);
            return;
        }

        if !src_bytes
            .iter()
            .any(|&b| b == b' ' || b == b'\t' || b == b'"')
        {
            xstrlcpy(dest, src, dest_remaining);
            return;
        }

        if !src_bytes.iter().any(|&b| b == b'"' || b == b'\\') {
            xstrlcpy(dest, c"\"".as_ptr(), dest_remaining);
            xstrlcat(dest, src, dest_remaining);
            xstrlcat(dest, c"\"".as_ptr(), dest_remaining);
            return;
        }

        debug_assert!(dest_remaining > 0);
        dest_remaining -= 1;
        *cur = NUL;
        cur = cur.add(1);
        debug_assert!(dest_remaining > 0);
        dest_remaining -= 1;
        *cur = b'"' as c_char;
        cur = cur.add(1);
        for i in (1..=src_len).rev() {
            debug_assert!(dest_remaining > 0);
            dest_remaining -= 1;
            let ch = src_bytes[i - 1];
            *cur = ch as c_char;
            cur = cur.add(1);
            if quote_hit && ch == b'\\' {
                debug_assert!(dest_remaining > 0);
                dest_remaining -= 1;
                *cur = b'\\' as c_char;
                cur = cur.add(1);
            } else if ch == b'"' {
                quote_hit = true;
                debug_assert!(dest_remaining > 0);
                dest_remaining -= 1;
                *cur = b'\\' as c_char;
                cur = cur.add(1);
            } else {
                quote_hit = false;
            }
        }
        debug_assert!(dest_remaining > 0);
        *cur = b'"' as c_char;

        while (start as usize) < (cur as usize) {
            ptr::swap(start, cur);
            start = start.add(1);
            cur = cur.sub(1);
        }
    }
}

#[cfg(windows)]
mod win_impl {
    use super::*;

    type HANDLE = *mut c_void;
    type DWORD = c_ulong;
    type BOOL = c_int;
    type BOOLEAN = u8;
    type uv_connect_t = c_void;
    type uv_thread_t = *mut c_void;
    type uv_connect_cb = Option<unsafe extern "C" fn(req: *mut uv_connect_t, status: c_int)>;
    type uv_thread_cb = Option<unsafe extern "C" fn(arg: *mut c_void)>;
    type WaitOrTimerCallback =
        Option<unsafe extern "system" fn(lpParameter: *mut c_void, TimerOrWaitFired: BOOLEAN)>;

    const UV_REQ: c_int = 1;
    const UV_CONNECT: c_int = 2;
    const UV_RUN_ONCE: c_int = 1;
    const UV_ENOENT: c_int = -4058;
    const UV_ENOSYS: c_int = -4054;

    const WAIT_TIMEOUT: DWORD = 258;
    const INFINITE: DWORD = 0xFFFF_FFFF;
    const WT_EXECUTEDEFAULT: c_ulong = 0x0000_0000;
    const WT_EXECUTEONLYONCE: c_ulong = 0x0000_0008;
    const LOGLVL_ERR: c_int = 4;

    #[repr(C)]
    struct PtyProcWinTail {
        width: u16,
        height: u16,
        conpty: *mut conpty_t,
        finish_wait: HANDLE,
        proc_handle: HANDLE,
    }

    unsafe extern "C" {
        fn uv_pipe_connect(
            req: *mut uv_connect_t,
            handle: *mut uv_pipe_t,
            name: *const c_char,
            cb: uv_connect_cb,
        );
        fn uv_thread_create(tid: *mut uv_thread_t, entry: uv_thread_cb, arg: *mut c_void) -> c_int;
        fn uv_thread_detach(tid: *mut uv_thread_t) -> c_int;

    }

    unsafe extern "system" {
        fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
        fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;
        fn GetLastError() -> DWORD;
        fn GetProcessId(Process: HANDLE) -> DWORD;
        fn RegisterWaitForSingleObject(
            phNewWaitObject: *mut HANDLE,
            hObject: HANDLE,
            Callback: WaitOrTimerCallback,
            Context: *mut c_void,
            dwMilliseconds: c_ulong,
            dwFlags: c_ulong,
        ) -> BOOL;
        fn UnregisterWaitEx(WaitHandle: HANDLE, CompletionEvent: HANDLE) -> BOOL;
        fn CloseHandle(hObject: HANDLE) -> BOOL;
        fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut DWORD) -> BOOL;
    }

    #[inline]
    unsafe fn pty_proc_win_tail(ptyproc: *mut PtyProc) -> *mut PtyProcWinTail {
        unsafe {
            ptyproc
                .cast::<u8>()
                .add(proc_size())
                .cast::<PtyProcWinTail>()
        }
    }

    #[inline]
    unsafe fn connect_req_handle_ptr(req: *mut uv_connect_t) -> *mut *mut uv_stream_t {
        unsafe {
            req.cast::<u8>()
                .add(uv_req_size(UV_REQ) + size_of::<*mut c_void>())
                .cast::<*mut uv_stream_t>()
        }
    }

    unsafe extern "C" fn os_conpty_free_thread(arg: *mut c_void) {
        unsafe {
            os_conpty_free(arg.cast());
        }
    }

    unsafe extern "system" fn pty_proc_terminate_cb(context: *mut c_void, _unused: BOOLEAN) {
        unsafe {
            let proc_: *mut Proc = context;
            let hdr = proc_header(proc_);
            let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
            argv[0] = context;
            loop_schedule_fast(
                (*hdr).loop_,
                Event {
                    handler: Some(pty_proc_finish),
                    argv,
                },
            );
        }
    }

    unsafe extern "C" fn pty_proc_close_console(stream: *mut Stream, _data: *mut c_void) {
        unsafe {
            let s_fields = stream_fields(stream);
            let ptyproc: *mut PtyProc = (*s_fields).internal_data;
            let proc_: *mut Proc = ptyproc.cast();
            let hdr = proc_header(proc_);
            let r_out = proc_out(proc_);
            let r_fields = rstream_fields(r_out);
            let ptail = pty_proc_win_tail(ptyproc);
            if (*ptail).conpty.is_null() {
                return;
            }
            if (*r_fields).num_bytes == 0 {
                TerminateProcess((*ptail).proc_handle, 0);
            }
            let mut tid: uv_thread_t = ptr::null_mut();
            uv_thread_create(
                &raw mut tid,
                Some(os_conpty_free_thread),
                (*ptail).conpty.cast(),
            );
            while !(*r_fields).did_eof && WaitForSingleObject(tid, 0) == WAIT_TIMEOUT {
                uv_run((*hdr).loop_.cast(), UV_RUN_ONCE);
            }
            uv_thread_detach(&raw mut tid);
            (*ptail).conpty = ptr::null_mut();
        }
    }

    /// @returns zero on success, or negative error code.
    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_spawn(ptyproc: *mut PtyProc) -> c_int {
        unsafe {
            let proc_: *mut Proc = ptyproc.cast();
            let hdr = proc_header(proc_);
            let ptail = pty_proc_win_tail(ptyproc);
            let s_in = proc_in(proc_);
            let r_out = proc_out(proc_);
            let r_err = proc_err(proc_);

            let mut status: c_int;
            let mut conpty_object: *mut conpty_t = ptr::null_mut();
            let mut in_name: *mut c_char = ptr::null_mut();
            let mut out_name: *mut c_char = ptr::null_mut();
            let mut proc_handle: HANDLE = ptr::null_mut();
            let mut in_req: *mut uv_connect_t = ptr::null_mut();
            let mut out_req: *mut uv_connect_t = ptr::null_mut();
            let mut file: *mut u16 = ptr::null_mut();
            let mut cmd_line: *mut u16 = ptr::null_mut();
            let mut cwd: *mut u16 = ptr::null_mut();
            let mut env: *mut u16 = ptr::null_mut();
            let mut emsg: *const c_char = ptr::null();

            debug_assert!(*stream_closed_ptr(r_err.cast()));

            'cleanup: {
                if !os_has_conpty_working() {
                    status = UV_ENOSYS;
                    break 'cleanup;
                }
                conpty_object = os_conpty_init(
                    &raw mut in_name,
                    &raw mut out_name,
                    (*ptail).width,
                    (*ptail).height,
                );
                if conpty_object.is_null() {
                    status = UV_ENOSYS;
                    break 'cleanup;
                }

                if !*stream_closed_ptr(s_in) {
                    in_req = xmalloc(uv_req_size(UV_CONNECT)).cast();
                    uv_pipe_connect(
                        in_req,
                        stream_uv_ptr(s_in).cast(),
                        in_name,
                        Some(pty_proc_connect_cb),
                    );
                }

                if !*stream_closed_ptr(r_out.cast()) {
                    out_req = xmalloc(uv_req_size(UV_CONNECT)).cast();
                    uv_pipe_connect(
                        out_req,
                        stream_uv_ptr(r_out.cast()).cast(),
                        out_name,
                        Some(pty_proc_connect_cb),
                    );
                }

                if !(*hdr).cwd.is_null() {
                    status = utf8_to_utf16((*hdr).cwd, -1, &raw mut cwd);
                    if status != 0 {
                        emsg = c"utf8_to_utf16(proc->cwd) failed".as_ptr();
                        break 'cleanup;
                    }
                }

                let is_cmdexe = os_shell_is_cmdexe(*(*hdr).argv);
                if is_cmdexe {
                    let mut path: *mut c_char = ptr::null_mut();
                    if !os_can_exe(*(*hdr).argv, &raw mut path, true) {
                        status = UV_ENOENT;
                        emsg = c"executable not found".as_ptr();
                        break 'cleanup;
                    }
                    status = utf8_to_utf16(path, -1, &raw mut file);
                    if status != 0 {
                        emsg = c"utf8_to_utf16(proc->argv[0]) failed".as_ptr();
                        break 'cleanup;
                    }
                    xfree(path.cast());
                }
                status = build_cmd_line((*hdr).argv, &raw mut cmd_line, is_cmdexe);
                if status != 0 {
                    emsg = c"build_cmd_line failed".as_ptr();
                    break 'cleanup;
                }

                if !(*hdr).env.is_null() {
                    status = build_env_block((*hdr).env, &raw mut env);
                }

                if status != 0 {
                    emsg = c"build_env_block failed".as_ptr();
                    break 'cleanup;
                }

                if !os_conpty_spawn(
                    conpty_object,
                    &raw mut proc_handle,
                    file,
                    cmd_line,
                    cwd,
                    env,
                ) {
                    emsg = c"os_conpty_spawn failed".as_ptr();
                    status = GetLastError() as c_int;
                    break 'cleanup;
                }
                (*hdr).pid = GetProcessId(proc_handle) as c_int;

                if RegisterWaitForSingleObject(
                    &raw mut (*ptail).finish_wait,
                    proc_handle,
                    Some(pty_proc_terminate_cb),
                    ptyproc,
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
                    uv_run((*hdr).loop_.cast(), UV_RUN_ONCE);
                }

                let out_s_fields = stream_fields(r_out.cast());
                (*out_s_fields).before_close_cb = Some(pty_proc_close_console);
                (*ptail).conpty = conpty_object;
                (*ptail).proc_handle = proc_handle;
                conpty_object = ptr::null_mut();
                proc_handle = ptr::null_mut();
            }

            if status != 0 {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"pty_proc_spawn".as_ptr(),
                    line!() as c_int,
                    true,
                    c"pty_proc_spawn(%s): %s: error code: %d".as_ptr(),
                    *(*hdr).argv,
                    emsg,
                    status,
                );
                status = uv_translate_sys_error(status);
            }
            os_conpty_free(conpty_object.cast());
            xfree(in_name.cast());
            xfree(out_name.cast());
            if !proc_handle.is_null() {
                CloseHandle(proc_handle);
            }
            xfree(in_req.cast());
            xfree(out_req.cast());
            xfree(file.cast());
            xfree(cmd_line.cast());
            xfree(env.cast());
            xfree(cwd.cast());
            status
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_tty_name(_ptyproc: *mut PtyProc) -> *const c_char {
        c"?".as_ptr()
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_resize(ptyproc: *mut PtyProc, width: u16, height: u16) {
        unsafe {
            let ptail = pty_proc_win_tail(ptyproc);
            os_conpty_set_size((*ptail).conpty, width, height);
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_resume(_ptyproc: *mut PtyProc) {}

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_flush_master(_ptyproc: *mut PtyProc) {}

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_close(ptyproc: *mut PtyProc) {
        unsafe {
            let proc_: *mut Proc = ptyproc.cast();
            let proc_t = proc_tail(proc_);
            let ptail = pty_proc_win_tail(ptyproc);

            pty_proc_close_master(ptyproc);

            if !(*ptail).finish_wait.is_null() {
                UnregisterWaitEx((*ptail).finish_wait, ptr::null_mut());
                (*ptail).finish_wait = ptr::null_mut();
            }
            if !(*ptail).proc_handle.is_null() {
                CloseHandle((*ptail).proc_handle);
                (*ptail).proc_handle = ptr::null_mut();
            }

            if let Some(cb) = (*proc_t).internal_close_cb {
                cb(proc_);
            }
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_close_master(_ptyproc: *mut PtyProc) {}

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_teardown(_loop_: *mut Loop) {}

    unsafe extern "C" fn pty_proc_connect_cb(req: *mut uv_connect_t, status: c_int) {
        debug_assert_eq!(status, 0);
        unsafe {
            ptr::write_volatile(connect_req_handle_ptr(req), ptr::null_mut());
        }
    }

    unsafe extern "C" fn pty_proc_finish(argv: *mut *mut c_void) {
        unsafe {
            let ptyproc = *argv as *mut PtyProc;
            let proc_: *mut Proc = ptyproc.cast();
            let hdr = proc_header(proc_);
            let proc_t = proc_tail(proc_);
            let ptail = pty_proc_win_tail(ptyproc);

            let mut exit_code: DWORD = 0;
            GetExitCodeProcess((*ptail).proc_handle, &raw mut exit_code);
            (*hdr).status = if (*hdr).exit_signal != 0 {
                128 + c_int::from((*hdr).exit_signal)
            } else {
                exit_code as c_int
            };

            if let Some(cb) = (*proc_t).internal_exit_cb {
                cb(proc_);
            }
        }
    }

    unsafe fn build_cmd_line(
        mut argv: *mut *mut c_char,
        cmd_line: *mut *mut u16,
        is_cmdexe: bool,
    ) -> c_int {
        unsafe {
            let mut args: Vec<*mut c_char> = Vec::new();
            let mut utf8_cmd_line_len: usize = 0;
            let mut argc: usize = 0;

            while !(*argv).is_null() {
                let arg_str = *argv;
                let buf_len = if is_cmdexe {
                    c_strlen(arg_str) + 1
                } else {
                    c_strlen(arg_str) * 2 + 3
                };
                let arg_buf: *mut c_char = xmalloc(buf_len).cast();
                if is_cmdexe {
                    xstrlcpy(
                        arg_buf,
                        if argc == 0 {
                            c"cmd.exe".as_ptr()
                        } else {
                            arg_str
                        },
                        buf_len,
                    );
                } else {
                    quote_cmd_arg(arg_buf, buf_len, arg_str);
                }
                utf8_cmd_line_len += c_strlen(arg_buf);
                args.push(arg_buf);
                argc += 1;
                argv = argv.add(1);
            }

            utf8_cmd_line_len += argc;
            let utf8_cmd_line: *mut c_char = xmalloc(utf8_cmd_line_len).cast();
            *utf8_cmd_line = NUL;
            for (idx, arg_buf) in args.iter().copied().enumerate() {
                xstrlcat(utf8_cmd_line, arg_buf, utf8_cmd_line_len);
                xfree(arg_buf.cast());
                if idx + 1 < args.len() {
                    xstrlcat(utf8_cmd_line, c" ".as_ptr(), utf8_cmd_line_len);
                }
            }

            let result = utf8_to_utf16(utf8_cmd_line, -1, cmd_line);
            xfree(utf8_cmd_line.cast());
            result
        }
    }

    unsafe fn build_env_block(denv: *mut dict_T, env_block: *mut *mut u16) -> c_int {
        unsafe {
            let denv_size = if denv.is_null() {
                0
            } else {
                (*denv).dv_hashtab.ht_used
            };
            let mut env_block_len: usize = 0;
            let mut rc: c_int = 0;
            let env = tv_dict_to_env(denv);
            let mut nodes: Vec<(*mut u16, usize)> = Vec::with_capacity(denv_size);

            'cleanup: {
                for i in 0..denv_size {
                    let mut str_utf16: *mut u16 = ptr::null_mut();
                    rc = utf8_to_utf16(*env.add(i), -1, &raw mut str_utf16);
                    if rc != 0 {
                        xfree(str_utf16.cast());
                        break 'cleanup;
                    }
                    let len = wcslen(str_utf16) + 1;
                    env_block_len += len;
                    nodes.push((str_utf16, len));
                }

                env_block_len += 1;
                let block: *mut u16 = xmalloc(size_of::<u16>() * env_block_len).cast();
                *env_block = block;
                let mut pos = block;

                for &(s, len) in &nodes {
                    ptr::copy_nonoverlapping(s, pos, len);
                    pos = pos.add(len);
                }
                *pos = 0;
            }

            for (s, _) in nodes {
                xfree(s.cast());
            }
            rc
        }
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
            let mut c_in = Vec::with_capacity(input.len() + 1);
            c_in.extend_from_slice(input.as_bytes());
            c_in.push(0);
            let mut out = vec![0 as c_char; input.len() * 2 + 4];
            unsafe {
                quote_cmd_arg(out.as_mut_ptr(), out.len(), c_in.as_ptr().cast());
                let got = CStr::from_ptr(out.as_ptr()).to_str().unwrap();
                assert_eq!(got, expected, "failed for input {input:?}");
            }
        }
    }
}
