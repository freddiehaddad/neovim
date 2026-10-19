//! Log module (`src/nvim/log.c`).
//!
//! How Linux `printk()` handles recursion, buffering, etc.:
//! <https://lwn.net/Articles/780556/>

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::eval::vars::get_vim_var_str;
use crate::globals::*;
use crate::message::msg_schedule_semsg;
#[allow(unused_imports)]
use crate::os::env::os_getenv_buf;
#[allow(unused_imports)]
use crate::os::env::{expand_env, os_get_pid, os_setenv};
#[allow(unused_imports)]
use crate::os::fs::{os_exepath, os_isdir, os_mkdir_recurse};
#[allow(unused_imports)]
use crate::os::stdpaths::get_xdg_home;
#[allow(unused_imports)]
use crate::os::stdpaths::stdpaths_user_state_subpath;
#[allow(unused_imports)]
use crate::path::{concat_fnames_realloc, path_tail};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::Write;
use std::ptr;

use crate::memory::{entered_free_all_mem, xfree, xstrlcat, xstrlcpy};
use crate::os::time::{Tm, os_localtime};
use crate::profile::FILE;

const NUL: c_char = 0;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const EOF: c_int = -1;

pub const LOGLVL_DBG: c_int = 1;
pub const LOGLVL_INF: c_int = 2;
pub const LOGLVL_WRN: c_int = 3;
pub const LOGLVL_ERR: c_int = 4;

const KXDG_STATE_HOME: c_int = 3;
const VV_SEND_SERVER: c_int = 28;

use crate::os::fs::NvimStats;

#[repr(C)]
struct UvTimeval64 {
    tv_sec: i64,
    tv_usec: i32,
}

#[repr(C, align(8))]
struct UvMutex([u8; 128]);

static mut LOG_FILE_PATH: [c_char; MAXPATHL + 1] = [0; MAXPATHL + 1];
static mut DID_LOG_INIT: bool = false;
static mut MUTEX: UvMutex = UvMutex([0; 128]);
static mut RECURSIVE: bool = false;
static mut DID_MSG: bool = false;
static mut INSTANCE_NAME: [c_char; 32] = [0; 32];

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut g_min_log_level: c_int = LOGLVL_WRN;

#[cfg(not(test))]
unsafe extern "C" {
    fn uv_strerror(err: c_int) -> *const c_char;
    fn uv_mutex_init_recursive(handle: *mut c_void) -> c_int;
    fn uv_mutex_lock(handle: *mut c_void);
    fn uv_mutex_unlock(handle: *mut c_void);
    fn uv_print_all_handles(loop_: *mut c_void, stream: *mut FILE);
    fn uv_gettimeofday(tv: *mut UvTimeval64) -> c_int;
    fn strftime(s: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;

    fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
    fn fclose(stream: *mut FILE) -> c_int;
    fn fprintf(stream: *mut FILE, format: *const c_char, ...) -> c_int;
    fn fputc(c: c_int, stream: *mut FILE) -> c_int;
    fn fflush(stream: *mut FILE) -> c_int;
    fn strerror(errnum: c_int) -> *const c_char;

    #[cfg(unix)]
    fn fgets(s: *mut c_char, n: c_int, stream: *mut FILE) -> *mut c_char;
    #[cfg(unix)]
    fn popen(command: *const c_char, mode: *const c_char) -> *mut FILE;
    #[cfg(unix)]
    fn pclose(stream: *mut FILE) -> c_int;
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    fn backtrace(buffer: *mut *mut c_void, size: c_int) -> c_int;

    #[cfg(windows)]
    fn __acrt_iob_func(ix: u32) -> *mut FILE;
    #[cfg(target_os = "linux")]
    #[link_name = "stderr"]
    static mut c_stderr: *mut FILE;
    #[cfg(target_os = "linux")]
    #[link_name = "stdout"]
    static mut c_stdout: *mut FILE;
    #[cfg(not(any(windows, target_os = "linux")))]
    static mut __stderrp: *mut FILE;
    #[cfg(not(any(windows, target_os = "linux")))]
    static mut __stdoutp: *mut FILE;

    #[doc(hidden)]
    pub fn snprintf(s: *mut c_char, n: usize, format: *const c_char, ...) -> c_int;
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_get_stdout() -> *mut FILE {
    #[cfg(all(not(test), windows))]
    unsafe {
        __acrt_iob_func(1)
    }
    #[cfg(all(not(test), target_os = "linux"))]
    unsafe {
        c_stdout
    }
    #[cfg(all(not(test), not(any(windows, target_os = "linux"))))]
    unsafe {
        __stdoutp
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_get_stderr() -> *mut FILE {
    #[cfg(all(not(test), windows))]
    unsafe {
        __acrt_iob_func(2)
    }
    #[cfg(all(not(test), target_os = "linux"))]
    unsafe {
        c_stderr
    }
    #[cfg(all(not(test), not(any(windows, target_os = "linux"))))]
    unsafe {
        __stderrp
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_is_std_stream(fp: *mut FILE) -> bool {
    #[cfg(all(not(test), windows))]
    unsafe {
        fp == __acrt_iob_func(2) || fp == __acrt_iob_func(1)
    }
    #[cfg(all(not(test), target_os = "linux"))]
    unsafe {
        fp == c_stderr || fp == c_stdout
    }
    #[cfg(all(not(test), not(any(windows, target_os = "linux"))))]
    unsafe {
        fp == __stderrp || fp == __stdoutp
    }
    #[cfg(test)]
    {
        let _ = fp;
        false
    }
}

#[macro_export]
macro_rules! logmsg {
    ($log_level:expr, $context:expr, $func_name:expr, $line_num:expr, $eol:expr, $fmt:expr $(,)?) => {{
        $crate::log::logmsg_str($log_level, $context, $func_name, $line_num, $eol, $fmt)
    }};
    ($log_level:expr, $context:expr, $func_name:expr, $line_num:expr, $eol:expr, $fmt:expr, $($arg:expr),+ $(,)?) => {{
        let lvl = $log_level;
        if !$crate::log::log_should_log(lvl) {
            false
        } else {
            let fmt_ptr: *const std::ffi::c_char = $fmt;
            let mut stack_buf = [0 as std::ffi::c_char; 1024];
            #[cfg(not(test))]
            let len = $crate::log::snprintf(
                (&raw mut stack_buf).cast::<std::ffi::c_char>(),
                stack_buf.len(),
                fmt_ptr,
                $($arg),+
            );
            #[cfg(test)]
            let len = {
                let _ = (&raw mut stack_buf, fmt_ptr, $($arg),+);
                0
            };
            if len >= stack_buf.len() as std::ffi::c_int {
                let cap = (len as usize) + 1;
                let heap_buf = $crate::memory::xmalloc(cap).cast::<std::ffi::c_char>();
                #[cfg(not(test))]
                $crate::log::snprintf(heap_buf, cap, fmt_ptr, $($arg),+);
                let ret = $crate::log::logmsg_str(
                    lvl,
                    $context,
                    $func_name,
                    $line_num,
                    $eol,
                    heap_buf,
                );
                $crate::memory::xfree(heap_buf.cast());
                ret
            } else {
                $crate::log::logmsg_str(
                    lvl,
                    $context,
                    $func_name,
                    $line_num,
                    $eol,
                    stack_buf.as_ptr(),
                )
            }
        }
    }};
}
pub use logmsg;

unsafe fn log_try_create(fname: *const c_char) -> bool {
    unsafe {
        if fname.is_null() || *fname == NUL {
            return false;
        }
        #[cfg(not(test))]
        {
            let log_file = fopen(fname, c"a".as_ptr());
            if log_file.is_null() {
                return false;
            }
            fclose(log_file);
            true
        }
        #[cfg(test)]
        {
            false
        }
    }
}

/// Initializes the log file path and sets `$NVIM_LOG_FILE` if empty.
unsafe fn log_path_init() {
    #[cfg(not(test))]
    unsafe {
        let size = MAXPATHL + 1;
        let path_ptr = (&raw mut LOG_FILE_PATH).cast::<c_char>();
        expand_env(
            c"$NVIM_LOG_FILE".as_ptr().cast_mut(),
            path_ptr,
            (size - 1) as c_int,
        );
        let user_set = CStr::from_ptr(path_ptr) != c"$NVIM_LOG_FILE";

        if !user_set || *path_ptr == NUL || os_isdir(path_ptr) || !log_try_create(path_ptr) {
            if user_set {
                os_setenv(c"__NVIM_LOG_FILE_WANT".as_ptr(), path_ptr, 1);
            }
            let loghome = concat_fnames_realloc(
                get_xdg_home(crate::os::stdpaths::XDGVarType::kXDGStateHome),
                c"logs".as_ptr(),
                true,
            );
            let mut failed_dir: *mut c_char = ptr::null_mut();
            let mut log_dir_failure: c_int = 0;
            if !os_isdir(loghome) {
                log_dir_failure =
                    os_mkdir_recurse(loghome, 0o700, &raw mut failed_dir, ptr::null_mut());
            }
            xfree(loghome.cast());

            let defaultpath = stdpaths_user_state_subpath(c"logs/nvim.log".as_ptr(), 0, true);
            let mut len = xstrlcpy(path_ptr, defaultpath, size);
            xfree(defaultpath.cast());

            if len >= size || !log_try_create(path_ptr) {
                if !user_set {
                    os_setenv(c"__NVIM_LOG_FILE_WANT".as_ptr(), path_ptr, 1);
                }
                len = xstrlcpy(path_ptr, c"nvim.log".as_ptr(), size);
            }
            if len >= size || !log_try_create(path_ptr) {
                *path_ptr = NUL;
                xfree(failed_dir.cast());
                return;
            }
            os_setenv(c"NVIM_LOG_FILE".as_ptr(), path_ptr, 1);
            if log_dir_failure != 0 {
                let err_str = uv_strerror(log_dir_failure);
                let failed_s = if failed_dir.is_null() {
                    "(null)"
                } else {
                    CStr::from_ptr(failed_dir).to_str().unwrap_or("")
                };
                let err_s = if err_str.is_null() {
                    ""
                } else {
                    CStr::from_ptr(err_str).to_str().unwrap_or("")
                };
                let mut msg = Vec::with_capacity(128);
                let _ = write!(
                    msg,
                    "Failed to create directory {failed_s} for writing logs: {err_s}\0"
                );
                logmsg_str(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"log_path_init".as_ptr(),
                    line!() as c_int,
                    true,
                    msg.as_ptr().cast(),
                );
            }
            xfree(failed_dir.cast());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_init() {
    #[cfg(not(test))]
    unsafe {
        uv_mutex_init_recursive((&raw mut MUTEX).cast());
        log_path_init();
        DID_LOG_INIT = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_lock() {
    #[cfg(not(test))]
    unsafe {
        uv_mutex_lock((&raw mut MUTEX).cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_unlock() {
    #[cfg(not(test))]
    unsafe {
        uv_mutex_unlock((&raw mut MUTEX).cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_should_log(log_level: c_int) -> bool {
    #[cfg(not(test))]
    unsafe {
        if !DID_LOG_INIT {
            g_stats.log_skip += 1;
            return false;
        }
        log_level >= g_min_log_level
    }
    #[cfg(test)]
    {
        let _ = log_level;
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn logmsg_str(
    log_level: c_int,
    context: *const c_char,
    func_name: *const c_char,
    line_num: c_int,
    eol: bool,
    msg: *const c_char,
) -> bool {
    #[cfg(not(test))]
    unsafe {
        if !DID_LOG_INIT {
            g_stats.log_skip += 1;
            return false;
        }
        if log_level < g_min_log_level {
            return false;
        }

        if entered_free_all_mem {
            let stderr = log_get_stderr();
            fprintf(
                stderr,
                c"FATAL: error in free_all_mem\n %s %s %d: ".as_ptr(),
                context,
                func_name,
                line_num,
            );
            fprintf(stderr, c"%s".as_ptr(), msg);
            if eol {
                fprintf(stderr, c"\n".as_ptr());
            }
            std::process::abort();
        }

        log_lock();
        if RECURSIVE {
            if !DID_MSG {
                DID_MSG = true;
                msg_schedule_semsg!(
                    c"E5430: %s:%d: recursive log!".as_ptr(),
                    if !func_name.is_null() {
                        func_name
                    } else {
                        context
                    },
                    line_num,
                );
            }
            g_stats.log_skip += 1;
            log_unlock();
            return false;
        }
        RECURSIVE = true;
        let log_file = open_log_file();
        let ret = do_log_to_file_str(log_file, log_level, context, func_name, line_num, eol, msg);
        if !log_is_std_stream(log_file) {
            fclose(log_file);
        }
        RECURSIVE = false;
        log_unlock();
        ret
    }
    #[cfg(test)]
    {
        let _ = (log_level, context, func_name, line_num, eol, msg);
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_uv_handles(loop_: *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        log_lock();
        let log_file = open_log_file();
        uv_print_all_handles(loop_, log_file);
        if !log_is_std_stream(log_file) {
            fclose(log_file);
        }
        log_unlock();
    }
    #[cfg(test)]
    {
        let _ = loop_;
    }
}

/// Open the log file for appending.
///
/// @return Log file, or `stderr` on failure
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn open_log_file() -> *mut FILE {
    #[cfg(not(test))]
    unsafe {
        let path_ptr = (&raw const LOG_FILE_PATH).cast::<c_char>();
        let mut errnum: c_int = 0;
        if *path_ptr != NUL {
            let f = fopen(path_ptr, c"a".as_ptr());
            if !f.is_null() {
                return f;
            }
            errnum = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        }

        let stderr = log_get_stderr();
        let err_ptr = strerror(errnum);
        let err_s = if err_ptr.is_null() {
            ""
        } else {
            CStr::from_ptr(err_ptr).to_str().unwrap_or("")
        };
        let path_s = CStr::from_ptr(path_ptr).to_str().unwrap_or("");
        let mut msg = Vec::with_capacity(128);
        let _ = write!(msg, "failed to open $NVIM_LOG_FILE ({err_s}): {path_s}\0");
        do_log_to_file_str(
            stderr,
            LOGLVL_ERR,
            ptr::null(),
            c"open_log_file".as_ptr(),
            line!() as c_int,
            true,
            msg.as_ptr().cast(),
        );
        stderr
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

#[cfg(unix)]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_callstack_to_file_trace(
    log_file: *mut FILE,
    func_name: *const c_char,
    line_num: c_int,
    trace: *const *mut c_void,
    trace_size: c_int,
) {
    #[cfg(not(test))]
    unsafe {
        let mut exepath = [0 as c_char; MAXPATHL];
        let mut exepathlen: usize = MAXPATHL;
        if os_exepath(exepath.as_mut_ptr(), &raw mut exepathlen) != 0 {
            std::process::abort();
        }
        assert!(24 + exepathlen < IOSIZE);

        let exepath_s = CStr::from_ptr(exepath.as_ptr()).to_str().unwrap_or("");
        let mut cmdbuf = Vec::with_capacity(IOSIZE + 2000 + MAXPATHL);
        let _ = write!(cmdbuf, "addr2line -e {exepath_s} -f -p");
        for i in 1..(trace_size as usize) {
            let p = *trace.add(i);
            let _ = write!(cmdbuf, " {p:p}");
        }
        cmdbuf.push(0);

        do_log_to_file_str(
            log_file,
            LOGLVL_DBG,
            ptr::null(),
            func_name,
            line_num,
            true,
            c"trace:".as_ptr(),
        );
        let fp = popen(cmdbuf.as_ptr().cast(), c"r".as_ptr());
        assert!(!fp.is_null());
        let mut linebuf = [0 as c_char; IOSIZE];
        while !fgets(linebuf.as_mut_ptr(), (IOSIZE - 1) as c_int, fp).is_null() {
            fprintf(log_file, c"  %s".as_ptr(), linebuf.as_ptr());
        }
        pclose(fp);

        if !log_is_std_stream(log_file) {
            fclose(log_file);
        }
    }
    #[cfg(test)]
    {
        let _ = (log_file, func_name, line_num, trace, trace_size);
    }
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_callstack_to_file(
    log_file: *mut FILE,
    func_name: *const c_char,
    line_num: c_int,
) {
    #[cfg(not(test))]
    unsafe {
        let mut trace: [*mut c_void; 100] = [ptr::null_mut(); 100];
        let trace_size = backtrace(trace.as_mut_ptr(), trace.len() as c_int);
        log_callstack_to_file_trace(log_file, func_name, line_num, trace.as_ptr(), trace_size);
    }
    #[cfg(test)]
    {
        let _ = (log_file, func_name, line_num);
    }
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn log_callstack(func_name: *const c_char, line_num: c_int) {
    #[cfg(not(test))]
    unsafe {
        log_lock();
        let log_file = open_log_file();
        log_callstack_to_file(log_file, func_name, line_num);
        log_unlock();
    }
    #[cfg(test)]
    {
        let _ = (func_name, line_num);
    }
}

unsafe fn do_log_to_file_str(
    log_file: *mut FILE,
    log_level: c_int,
    context: *const c_char,
    func_name: *const c_char,
    line_num: c_int,
    eol: bool,
    msg: *const c_char,
) -> bool {
    #[cfg(not(test))]
    unsafe {
        const LOG_LEVELS: [&CStr; 5] = [c"", c"DBG", c"INF", c"WRN", c"ERR"];
        assert!((LOGLVL_DBG..=LOGLVL_ERR).contains(&log_level));

        let mut local_time = Tm { _fields: [0; 16] };
        if os_localtime(&raw mut local_time).is_null() {
            return false;
        }
        let mut date_time = [0 as c_char; 20];
        if strftime(
            date_time.as_mut_ptr(),
            date_time.len(),
            c"%Y-%m-%dT%H:%M:%S".as_ptr(),
            &raw const local_time,
        ) == 0
        {
            return false;
        }

        let mut millis: c_int = 0;
        let mut curtime = UvTimeval64 {
            tv_sec: 0,
            tv_usec: 0,
        };
        if uv_gettimeofday(&raw mut curtime) == 0 {
            millis = curtime.tv_usec / 1000;
        }

        let ui = ui_client_channel_id != 0;
        let name_ptr = (&raw mut INSTANCE_NAME).cast::<u8>();
        let first_byte = *name_ptr;
        let regen = ui || first_byte == 0 || first_byte == b'?';

        if regen {
            let mut parent_buf = [0 as c_char; MAXPATHL];
            let parent = path_tail(os_getenv_buf(
                c"NVIM".as_ptr(),
                parent_buf.as_mut_ptr(),
                parent_buf.len(),
            ));
            let serv = path_tail(get_vim_var_str(VV_SEND_SERVER));
            let mut formatted = Vec::with_capacity(32);
            if !parent.is_null() && *parent != NUL {
                let p = CStr::from_ptr(parent).to_bytes();
                formatted.extend_from_slice(if ui { b"ui/c/" } else { b"c/" });
                formatted.extend_from_slice(p);
            } else if !serv.is_null() && *serv != NUL {
                let s = CStr::from_ptr(serv).to_bytes();
                if ui {
                    formatted.extend_from_slice(b"ui/");
                }
                formatted.extend_from_slice(s);
            } else {
                let pid = os_get_pid();
                let prefix = if ui { "ui" } else { "?" };
                let _ = write!(formatted, "{prefix}.{pid:<5}");
            }
            let copy_len = formatted.len().min(31);
            ptr::copy_nonoverlapping(formatted.as_ptr(), name_ptr, copy_len);
            *name_ptr.add(copy_len) = 0;
        }

        let level_ptr = LOG_LEVELS[log_level as usize].as_ptr();
        let name_cptr = (&raw const INSTANCE_NAME).cast::<c_char>();
        let rv = if line_num == -1 || func_name.is_null() {
            fprintf(
                log_file,
                c"%s %s.%03d %-10s %s".as_ptr(),
                level_ptr,
                date_time.as_ptr(),
                millis,
                name_cptr,
                if context.is_null() {
                    c"?:".as_ptr()
                } else {
                    context
                },
            )
        } else {
            fprintf(
                log_file,
                c"%s %s.%03d %-10s %s%s:%d: ".as_ptr(),
                level_ptr,
                date_time.as_ptr(),
                millis,
                name_cptr,
                if context.is_null() {
                    c"".as_ptr()
                } else {
                    context
                },
                func_name,
                line_num,
            )
        };

        if rv < 0 {
            return false;
        }
        if fprintf(log_file, c"%s".as_ptr(), msg) < 0 {
            return false;
        }
        if eol {
            fputc(b'\n' as c_int, log_file);
        }
        if fflush(log_file) == EOF {
            return false;
        }

        true
    }
    #[cfg(test)]
    {
        let _ = (log_file, log_level, context, func_name, line_num, eol, msg);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_levels_and_should_log_in_test_mode() {
        assert_eq!(LOGLVL_DBG, 1);
        assert_eq!(LOGLVL_INF, 2);
        assert_eq!(LOGLVL_WRN, 3);
        assert_eq!(LOGLVL_ERR, 4);
        unsafe {
            assert!(!log_should_log(LOGLVL_ERR));
            assert!(!log_try_create(ptr::null()));
            assert!(!log_try_create(c"".as_ptr()));
        }
    }
}
