//! Log module (`src/nvim/log.c`).
//!
//! How Linux `printk()` handles recursion, buffering, etc.:
//! <https://lwn.net/Articles/780556/>

use crate::eval::vars::{VV_SEND_SERVER, get_vim_var_str};
use crate::event::r#loop::{uv_mutex_lock, uv_mutex_unlock, uv_strerror};
use crate::fileio::{EOF, FILE, fclose, fflush, fopen, fprintf, fputc};
use crate::globals::*;
use crate::memory::{mem_st, xstrlcpy};
use crate::message::msg_schedule_semsg;
use crate::os::env::{expand_env, os_get_pid, os_getenv_buf, os_setenv};
use crate::os::fs::{os_isdir, os_mkdir_recurse};
use crate::os::stdpaths::{get_xdg_home, stdpaths_user_state_subpath};
use crate::os::time::{Tm, os_localtime, strftime};
use crate::path::{concat_fnames_cstr, path_tail};
use crate::strings::strerror;
use std::ffi::{CStr, c_char, c_int};
use std::io::Write;
use std::ptr;

pub const LOGLVL_DBG: c_int = 1;
pub const LOGLVL_INF: c_int = 2;
pub const LOGLVL_WRN: c_int = 3;
pub const LOGLVL_ERR: c_int = 4;

#[repr(C, align(8))]
struct UvMutex([u8; 128]);

pub(crate) struct LogModuleState {
    pub(crate) g_min_log_level: c_int,
    log_file_path: [c_char; MAXPATHL + 1],
    did_log_init: bool,
    mutex: UvMutex,
    recursive: bool,
    did_msg: bool,
    instance_name: [c_char; 32],
}

impl LogModuleState {
    const fn new() -> Self {
        Self {
            g_min_log_level: LOGLVL_WRN,
            log_file_path: [0; MAXPATHL + 1],
            did_log_init: false,
            mutex: UvMutex([0; 128]),
            recursive: false,
            did_msg: false,
            instance_name: [0; 32],
        }
    }
}

static LOG_STATE: crate::globals::GlobalState<LogModuleState> = crate::globals::GlobalState::new(LogModuleState::new());

#[inline]
pub(crate) fn log_st() -> &'static mut LogModuleState {
    unsafe { &mut *LOG_STATE.as_ptr() }
}

use crate::event::r#loop::{
    UvTimeval64, uv_gettimeofday, uv_mutex_init_recursive, uv_print_all_handles,
};

unsafe extern "C" {
    #[cfg(windows)]
    fn __acrt_iob_func(ix: u32) -> *mut FILE;
    #[cfg(not(windows))]
    #[cfg_attr(target_os = "linux", link_name = "stderr")]
    #[cfg_attr(not(target_os = "linux"), link_name = "__stderrp")]
    static C_STDERR: *mut FILE;
    #[cfg(not(windows))]
    #[cfg_attr(target_os = "linux", link_name = "stdout")]
    #[cfg_attr(not(target_os = "linux"), link_name = "__stdoutp")]
    static C_STDOUT: *mut FILE;
}

pub fn log_get_stdout() -> *mut FILE {
    #[cfg(windows)]
    unsafe {
        __acrt_iob_func(1)
    }
    #[cfg(not(windows))]
    unsafe {
        C_STDOUT
    }
}

pub fn log_get_stderr() -> *mut FILE {
    #[cfg(windows)]
    unsafe {
        __acrt_iob_func(2)
    }
    #[cfg(not(windows))]
    unsafe {
        C_STDERR
    }
}

pub fn log_is_std_stream(fp: *mut FILE) -> bool {
    fp == log_get_stderr() || fp == log_get_stdout()
}

#[macro_export]
macro_rules! logmsg {
    ($log_level:expr, $context:expr, $func_name:expr, $line_num:expr, $eol:expr, $fmt:expr $(,)?) => {{
        $crate::log::logmsg_str(
            $log_level,
            $crate::strings::IntoCCharPtr::into_c_char_ptr($context),
            $crate::strings::IntoCCharPtr::into_c_char_ptr($func_name),
            $line_num,
            $eol,
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt),
        )
    }};
    ($log_level:expr, $context:expr, $func_name:expr, $line_num:expr, $eol:expr, $fmt:expr, $($arg:expr),+ $(,)?) => {{
        let lvl = $log_level;
        if !$crate::log::log_should_log(lvl) {
            false
        } else {
            let fmt_ptr: *const std::ffi::c_char =
                $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
            let ctx_ptr: *const std::ffi::c_char =
                $crate::strings::IntoCCharPtr::into_c_char_ptr($context);
            let fn_ptr: *const std::ffi::c_char =
                $crate::strings::IntoCCharPtr::into_c_char_ptr($func_name);
            let mut stack_buf = [0 as std::ffi::c_char; 1024];
            let len = $crate::strings::vim_snprintf!(
                (&raw mut stack_buf).cast::<std::ffi::c_char>(),
                stack_buf.len(),
                fmt_ptr,
                $($arg),+
            );
            if len >= stack_buf.len() as std::ffi::c_int {
                let cap = (len as usize) + 1;
                let mut heap_buf = vec![0 as std::ffi::c_char; cap];
                $crate::strings::vim_snprintf!(heap_buf.as_mut_ptr(), cap, fmt_ptr, $($arg),+);
                $crate::log::logmsg_str(
                    lvl,
                    ctx_ptr,
                    fn_ptr,
                    $line_num,
                    $eol,
                    heap_buf.as_ptr(),
                )
            } else {
                $crate::log::logmsg_str(
                    lvl,
                    ctx_ptr,
                    fn_ptr,
                    $line_num,
                    $eol,
                    stack_buf.as_ptr(),
                )
            }
        }
    }};
}
use crate::charset::NUL;
use crate::os::fs::MAXPATHL;
pub use logmsg;

fn log_try_create(fname: impl crate::strings::IntoCCharPtr) -> bool {
    let fname = fname.into_c_char_ptr();
    unsafe {
        if crate::memory::cstr_is_empty(fname) {
            return false;
        }
        {
            let log_file = fopen(fname, c"a".as_ptr());
            if log_file.is_null() {
                return false;
            }
            fclose(log_file);
            true
        }
    }
}

/// Initializes the log file path and sets `$NVIM_LOG_FILE` if empty.
fn log_path_init() {
    unsafe {
        let size = MAXPATHL + 1;
        let path_ptr = (&raw mut log_st().log_file_path).cast::<c_char>();
        expand_env(
            c"$NVIM_LOG_FILE",
            path_ptr,
            (size - 1) as c_int,
        );
        let user_set = crate::memory::cstr_as_bytes(path_ptr) != b"$NVIM_LOG_FILE";

        if !user_set || crate::memory::cstr_is_empty(path_ptr) || os_isdir(path_ptr) || !log_try_create(path_ptr) {
            if user_set {
                os_setenv(c"__NVIM_LOG_FILE_WANT", path_ptr, 1);
            }
            let state_home = get_xdg_home(crate::os::stdpaths::XDGVarType::XDGStateHome)
                .unwrap_or_default();
            let loghome = concat_fnames_cstr(
                state_home.as_ptr(),
                c"logs",
                true,
            );
            let mut failed_dir: Option<std::ffi::CString> = None;
            let mut log_dir_failure: c_int = 0;
            if !os_isdir(loghome.as_ptr()) {
                log_dir_failure =
                    os_mkdir_recurse(loghome.as_ptr(), 0o700, &mut failed_dir, None);
            }
            drop(loghome);

            let defaultpath = stdpaths_user_state_subpath(
                c"logs/nvim.log",
                0,
                true,
            );
            let mut len = xstrlcpy(path_ptr, defaultpath.as_ptr(), size);
            drop(defaultpath);

            if len >= size || !log_try_create(path_ptr) {
                if !user_set {
                    os_setenv(c"__NVIM_LOG_FILE_WANT", path_ptr, 1);
                }
                len = xstrlcpy(path_ptr, c"nvim.log", size);
            }
            if len >= size || !log_try_create(path_ptr) {
                *path_ptr = NUL;
                return;
            }
            os_setenv(c"NVIM_LOG_FILE", path_ptr, 1);
            if log_dir_failure != 0 {
                let err_str = uv_strerror(log_dir_failure);
                let failed_s = match failed_dir {
                    None => "(null)",
                    Some(ref d) => d.to_str().unwrap_or(""),
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
                    c"log_path_init",
                    line!() as c_int,
                    true,
                    msg.as_ptr().cast(),
                );
            }
        }
    }
}

pub fn log_init() {
    unsafe {
        uv_mutex_init_recursive((&raw mut log_st().mutex).cast());
        log_path_init();
        log_st().did_log_init = true;
    }
}

pub fn log_lock() {
    unsafe {
        uv_mutex_lock((&raw mut log_st().mutex).cast());
    }
}

pub fn log_unlock() {
    unsafe {
        uv_mutex_unlock((&raw mut log_st().mutex).cast());
    }
}

pub fn log_should_log(log_level: c_int) -> bool {
    if !log_st().did_log_init {
            g_st().g_stats.log_skip += 1;
            return false;
        }
        log_level >= log_st().g_min_log_level
}

pub fn logmsg_str(
    log_level: c_int,
    context: impl crate::strings::IntoCCharPtr,
    func_name: impl crate::strings::IntoCCharPtr,
    line_num: c_int,
    eol: bool,
    msg: impl crate::strings::IntoCCharPtr,
) -> bool { let context = context.into_c_char_ptr();
    let func_name = func_name.into_c_char_ptr();
    let msg = msg.into_c_char_ptr();
    unsafe {
        if !log_st().did_log_init {
            g_st().g_stats.log_skip += 1;
            return false;
        }
        if log_level < log_st().g_min_log_level {
            return false;
        }

        if mem_st().entered_free_all_mem {
            let stderr = log_get_stderr();
            fprintf!(
                stderr,
                c"FATAL: error in free_all_mem\n %s %s %d: ",
                context,
                func_name,
                line_num,
            );
            fprintf!(stderr, c"%s", msg);
            if eol {
                fprintf!(stderr, c"\n");
            }
            std::process::abort();
        }

        log_lock();
        if log_st().recursive {
            if !log_st().did_msg {
                log_st().did_msg = true;
                msg_schedule_semsg!(
                    c"E5430: %s:%d: recursive log!",
                    if !func_name.is_null() {
                        func_name
                    } else {
                        context
                    },
                    line_num,
                );
            }
            g_st().g_stats.log_skip += 1;
            log_unlock();
            return false;
        }
        log_st().recursive = true;
        let log_file = open_log_file();
        let ret = do_log_to_file_str(log_file, log_level, context, func_name, line_num, eol, msg);
        if !log_is_std_stream(log_file) {
            fclose(log_file);
        }
        log_st().recursive = false;
        log_unlock();
        ret
    }
}

pub fn log_uv_handles(loop_: *mut crate::event::r#loop::uv_loop_t) {
    unsafe {
        log_lock();
        let log_file = open_log_file();
        uv_print_all_handles(loop_, log_file);
        if !log_is_std_stream(log_file) {
            fclose(log_file);
        }
        log_unlock();
    }
}

/// Open the log file for appending.
///
/// @return Log file, or `stderr` on failure
pub fn open_log_file() -> *mut FILE {
    unsafe {
        let path_ptr = (&raw const log_st().log_file_path).cast::<c_char>();
        let mut errnum: c_int = 0;
        if !crate::memory::cstr_is_empty(path_ptr) {
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
            c"open_log_file",
            line!() as c_int,
            true,
            msg.as_ptr().cast(),
        );
        stderr
    }
}

fn do_log_to_file_str(
    log_file: *mut FILE,
    log_level: c_int,
    context: impl crate::strings::IntoCCharPtr,
    func_name: impl crate::strings::IntoCCharPtr,
    line_num: c_int,
    eol: bool,
    msg: impl crate::strings::IntoCCharPtr,
) -> bool { let context = context.into_c_char_ptr(); let msg = msg.into_c_char_ptr();
    let func_name = func_name.into_c_char_ptr();
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
        if uv_gettimeofday(&mut curtime) == 0 {
            millis = curtime.tv_usec / 1000;
        }

        let ui = g_st().ui_client_channel_id != 0;
        let name_ptr = (&raw mut log_st().instance_name).cast::<u8>();
        let first_byte = *name_ptr;
        let regen = ui || first_byte == 0 || first_byte == b'?';

        if regen {
            let mut parent_buf = [0 as c_char; MAXPATHL];
            let parent = path_tail(os_getenv_buf(
                c"NVIM",
                parent_buf.as_mut_ptr(),
                parent_buf.len(),
            ));
            let serv = path_tail(get_vim_var_str(VV_SEND_SERVER));
            let mut formatted = Vec::with_capacity(32);
            if !crate::memory::cstr_is_empty(parent) {
                let p = crate::memory::cstr_as_bytes(parent);
                formatted.extend_from_slice(if ui { b"ui/c/" } else { b"c/" });
                formatted.extend_from_slice(p);
            } else if !crate::memory::cstr_is_empty(serv) {
                let s = crate::memory::cstr_as_bytes(serv);
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
        let name_cptr = (&raw const log_st().instance_name).cast::<c_char>();
        let rv = if line_num == -1 || func_name.is_null() {
            fprintf!(
                log_file,
                c"%s %s.%03d %-10s %s",
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
            fprintf!(
                log_file,
                c"%s %s.%03d %-10s %s%s:%d: ",
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
        if fprintf!(log_file, c"%s", msg) < 0 {
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
        assert!(!log_should_log(LOGLVL_ERR));
            assert!(!log_try_create(ptr::null()));
            assert!(!log_try_create(c""));
    }
}
