#![forbid(unsafe_code)]

//! Log module (`src/nvim/log.c`).
//!
//! How Linux `printk()` handles recursion, buffering, etc.:
//! <https://lwn.net/Articles/780556/>

use crate::eval::vars::{VV_SEND_SERVER, get_vim_var_str};
use crate::event::r#loop::{loop_print_all_handles, uv_strerror};
use crate::fileio::{EOF, fprintf};
use crate::globals::*;
use crate::memory::entered_free_all_mem;
use crate::message::msg_schedule_semsg;
use crate::os::env::{expand_env, os_get_pid, os_getenv_buf, os_setenv};
use crate::os::fs::{CFile, MAXPATHL, os_isdir, os_mkdir_recurse};
use crate::os::stdpaths::{get_xdg_home, stdpaths_user_state_subpath};
use crate::os::time::{Tm, os_gettimeofday_ms, os_localtime, os_strftime};
use crate::path::{concat_fnames_cstr, path_tail};
use crate::strings::strerror;
use std::ffi::CStr;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub const LOGLVL_DBG: i32 = 1;
pub const LOGLVL_INF: i32 = 2;
pub const LOGLVL_WRN: i32 = 3;
pub const LOGLVL_ERR: i32 = 4;

static G_MIN_LOG_LEVEL: AtomicI32 = AtomicI32::new(LOGLVL_WRN);
static DID_LOG_INIT: AtomicBool = AtomicBool::new(false);
static DID_MSG: AtomicBool = AtomicBool::new(false);

std::thread_local! {
    static LOG_RECURSIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct LogInnerState {
    log_file_path: [u8; MAXPATHL + 1],
    instance_name: [u8; 32],
}

impl LogInnerState {
    const fn new() -> Self {
        Self {
            log_file_path: [0; MAXPATHL + 1],
            instance_name: [0; 32],
        }
    }
}

static LOG_STATE: std::sync::Mutex<LogInnerState> = std::sync::Mutex::new(LogInnerState::new());

#[inline]
pub(crate) fn min_log_level() -> i32 {
    G_MIN_LOG_LEVEL.load(Ordering::Relaxed)
}

#[inline]
pub(crate) fn set_min_log_level(lvl: i32) {
    G_MIN_LOG_LEVEL.store(lvl, Ordering::Relaxed);
}

pub fn log_get_stdout() -> CFile {
    CFile::stdout()
}

pub fn log_get_stderr() -> CFile {
    CFile::stderr()
}

#[macro_export]
macro_rules! logmsg {
    ($log_level:expr, $context:expr, $func_name:expr, $line_num:expr, $eol:expr, $fmt:expr $(,)?) => {{
        $crate::log::logmsg_str(
            $log_level,
            $context,
            $func_name,
            $line_num,
            $eol,
            $fmt,
        )
    }};
    ($log_level:expr, $context:expr, $func_name:expr, $line_num:expr, $eol:expr, $fmt:expr, $($arg:expr),+ $(,)?) => {{
        let lvl = $log_level;
        if !$crate::log::log_should_log(lvl) {
            false
        } else {
            let fmt_cstr: &::std::ffi::CStr = $fmt;
            let ctx_opt: Option<&::std::ffi::CStr> = $context;
            let fn_opt: Option<&::std::ffi::CStr> = $func_name;
            let mut stack_buf = [0u8; 1024];
            let len = $crate::strings::format_to_buf_fmt(
                &mut stack_buf,
                Some(fmt_cstr),
                &[$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),+],
            );
            if len >= stack_buf.len() as i32 {
                let cap = (len as usize) + 1;
                let mut heap_buf = vec![0u8; cap];
                $crate::strings::format_to_buf_fmt(
                    &mut heap_buf,
                    Some(fmt_cstr),
                    &[$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),+],
                );
                let msg_cstr = ::std::ffi::CStr::from_bytes_until_nul(&heap_buf).unwrap_or(c"");
                $crate::log::logmsg_str(
                    lvl,
                    ctx_opt,
                    fn_opt,
                    $line_num,
                    $eol,
                    msg_cstr,
                )
            } else {
                let msg_cstr = ::std::ffi::CStr::from_bytes_until_nul(&stack_buf).unwrap_or(c"");
                $crate::log::logmsg_str(
                    lvl,
                    ctx_opt,
                    fn_opt,
                    $line_num,
                    $eol,
                    msg_cstr,
                )
            }
        }
    }};
}
pub use logmsg;

fn log_try_create(fname: &CStr) -> bool {
    !fname.to_bytes().is_empty() && CFile::open_libc(fname, c"a").is_some()
}

/// Initializes the log file path and sets `$NVIM_LOG_FILE` if empty.
fn log_path_init() {
    let size = MAXPATHL + 1;
    let mut path_buf = [0u8; MAXPATHL + 1];
    expand_env(c"$NVIM_LOG_FILE", &mut path_buf[..size - 1]);
    let path_cstr = CStr::from_bytes_until_nul(&path_buf).unwrap_or(c"");
    let user_set = path_cstr.to_bytes() != b"$NVIM_LOG_FILE";

    if !user_set
        || path_cstr.to_bytes().is_empty()
        || os_isdir(path_cstr)
        || !log_try_create(path_cstr)
    {
        if user_set {
            os_setenv(c"__NVIM_LOG_FILE_WANT", path_cstr, 1);
        }
        let state_home =
            get_xdg_home(crate::os::stdpaths::XDGVarType::XDGStateHome).unwrap_or_default();
        let loghome = concat_fnames_cstr(&state_home, c"logs", true);
        let mut failed_dir: Option<std::ffi::CString> = None;
        let mut log_dir_failure: i32 = 0;
        if !os_isdir(&loghome) {
            log_dir_failure = os_mkdir_recurse(&loghome, 0o700, &mut failed_dir, None);
        }
        drop(loghome);

        let defaultpath = stdpaths_user_state_subpath(c"logs/nvim.log", 0, true);
        let mut len = crate::memory::copy_bytes_to_c_buf(&mut path_buf, defaultpath.as_bytes());
        drop(defaultpath);

        let cur_cstr = CStr::from_bytes_until_nul(&path_buf).unwrap_or(c"");
        if len >= size || !log_try_create(cur_cstr) {
            if !user_set {
                os_setenv(c"__NVIM_LOG_FILE_WANT", cur_cstr, 1);
            }
            len = crate::memory::copy_bytes_to_c_buf(&mut path_buf, b"nvim.log");
        }
        let cur_cstr = CStr::from_bytes_until_nul(&path_buf).unwrap_or(c"");
        if len >= size || !log_try_create(cur_cstr) {
            path_buf[0] = 0;
            LOG_STATE.lock().unwrap().log_file_path = path_buf;
            return;
        }
        os_setenv(c"NVIM_LOG_FILE", cur_cstr, 1);
        LOG_STATE.lock().unwrap().log_file_path = path_buf;
        if log_dir_failure != 0 {
            let err_str = uv_strerror(log_dir_failure);
            let failed_s = match failed_dir {
                None => "(null)",
                Some(ref d) => d.to_str().unwrap_or(""),
            };
            let err_s = err_str.to_str().unwrap_or("");
            let mut msg = Vec::with_capacity(128);
            let _ = write!(
                msg,
                "Failed to create directory {failed_s} for writing logs: {err_s}\0"
            );
            logmsg_str(
                LOGLVL_WRN,
                None,
                Some(c"log_path_init"),
                line!() as i32,
                true,
                CStr::from_bytes_until_nul(&msg).unwrap_or(c""),
            );
        }
    } else {
        LOG_STATE.lock().unwrap().log_file_path = path_buf;
    }
}

pub fn log_init() {
    log_path_init();
    DID_LOG_INIT.store(true, Ordering::Relaxed);
}

pub fn log_should_log(log_level: i32) -> bool {
    if !DID_LOG_INIT.load(Ordering::Relaxed) {
        g_st().g_stats.log_skip += 1;
        return false;
    }
    log_level >= min_log_level()
}

pub fn logmsg_str(
    log_level: i32,
    context: Option<&CStr>,
    func_name: Option<&CStr>,
    line_num: i32,
    eol: bool,
    msg: &CStr,
) -> bool {
    if !DID_LOG_INIT.load(Ordering::Relaxed) {
        g_st().g_stats.log_skip += 1;
        return false;
    }
    if log_level < min_log_level() {
        return false;
    }

    if entered_free_all_mem() {
        let stderr = log_get_stderr();
        fprintf!(
            &stderr,
            c"FATAL: error in free_all_mem\n %s %s %d: ",
            context.unwrap_or(c""),
            func_name.unwrap_or(c""),
            line_num,
        );
        fprintf!(&stderr, c"%s", msg);
        if eol {
            fprintf!(&stderr, c"\n");
        }
        std::process::abort();
    }

    if LOG_RECURSIVE.get() {
        if !DID_MSG.swap(true, Ordering::Relaxed) {
            msg_schedule_semsg!(
                c"E5430: %s:%d: recursive log!",
                func_name.or(context).unwrap_or(c""),
                line_num,
            );
        }
        g_st().g_stats.log_skip += 1;
        return false;
    }
    LOG_RECURSIVE.set(true);
    let mut st = LOG_STATE.lock().unwrap();
    let log_file = open_log_file_inner(&mut st);
    let ret = do_log_to_file_str(
        &mut st, &log_file, log_level, context, func_name, line_num, eol, msg,
    );
    drop(log_file);
    drop(st);
    LOG_RECURSIVE.set(false);
    ret
}

pub fn log_uv_handles(loop_: &mut crate::event::r#loop::uv_loop_t) {
    let mut st = LOG_STATE.lock().unwrap();
    let log_file = open_log_file_inner(&mut st);
    loop_print_all_handles(loop_, &log_file);
    drop(log_file);
}

/// Open the log file for appending.
///
/// @return Log file, or `stderr` on failure
fn open_log_file_inner(st: &mut LogInnerState) -> CFile {
    let path_buf = st.log_file_path;
    let path_cstr = CStr::from_bytes_until_nul(&path_buf).unwrap_or(c"");
    let mut errnum: i32 = 0;
    if !path_cstr.to_bytes().is_empty() {
        if let Some(f) = CFile::open_libc(path_cstr, c"a") {
            return f;
        }
        errnum = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    }

    let stderr = log_get_stderr();
    let err_s = strerror(errnum).to_str().unwrap_or("");
    let path_s = path_cstr.to_str().unwrap_or("");
    let mut msg = Vec::with_capacity(128);
    let _ = write!(msg, "failed to open $NVIM_LOG_FILE ({err_s}): {path_s}\0");
    do_log_to_file_str(
        st,
        &stderr,
        LOGLVL_ERR,
        None,
        Some(c"open_log_file"),
        line!() as i32,
        true,
        CStr::from_bytes_until_nul(&msg).unwrap_or(c""),
    );
    stderr
}

fn do_log_to_file_str(
    st: &mut LogInnerState,
    log_file: &CFile,
    log_level: i32,
    context: Option<&CStr>,
    func_name: Option<&CStr>,
    line_num: i32,
    eol: bool,
    msg: &CStr,
) -> bool {
    const LOG_LEVELS: [&CStr; 5] = [c"", c"DBG", c"INF", c"WRN", c"ERR"];
    assert!((LOGLVL_DBG..=LOGLVL_ERR).contains(&log_level));

    let mut local_time = Tm { _fields: [0; 16] };
    if os_localtime(&mut local_time).is_none() {
        return false;
    }
    let mut date_time = [0u8; 20];
    if os_strftime(&mut date_time, c"%Y-%m-%dT%H:%M:%S", &local_time) == 0 {
        return false;
    }

    let millis = os_gettimeofday_ms();

    let ui = g_st().ui_client_channel_id != 0;
    let first_byte = st.instance_name[0];
    let regen = ui || first_byte == 0 || first_byte == b'?';

    if regen {
        let mut parent_buf = [0u8; MAXPATHL];
        let parent = path_tail(os_getenv_buf(c"NVIM", &mut parent_buf).unwrap_or(c""));
        let serv = path_tail(get_vim_var_str(VV_SEND_SERVER));
        let mut formatted = Vec::with_capacity(32);
        if !parent.to_bytes().is_empty() {
            let p = parent.to_bytes();
            formatted.extend_from_slice(if ui { b"ui/c/" } else { b"c/" });
            formatted.extend_from_slice(p);
        } else if !serv.to_bytes().is_empty() {
            let s = serv.to_bytes();
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
        st.instance_name[..copy_len].copy_from_slice(&formatted[..copy_len]);
        st.instance_name[copy_len] = 0;
    }

    let level_cstr = LOG_LEVELS[log_level as usize];
    let inst_name = st.instance_name;
    let name_cstr = CStr::from_bytes_until_nul(&inst_name).unwrap_or(c"");
    let date_time_cstr = CStr::from_bytes_until_nul(&date_time).unwrap_or(c"");
    let rv = if let Some(func_name) = func_name
        && line_num != -1
    {
        fprintf!(
            log_file,
            c"%s %s.%03d %-10s %s%s:%d: ",
            level_cstr,
            date_time_cstr,
            millis,
            name_cstr,
            context.unwrap_or(c""),
            func_name,
            line_num,
        )
    } else {
        fprintf!(
            log_file,
            c"%s %s.%03d %-10s %s",
            level_cstr,
            date_time_cstr,
            millis,
            name_cstr,
            context.unwrap_or(c"?:"),
        )
    };

    if rv < 0 {
        return false;
    }
    if fprintf!(log_file, c"%s", msg) < 0 {
        return false;
    }
    if eol {
        log_file.putc(b'\n' as i32);
    }
    if log_file.flush() == EOF {
        return false;
    }

    true
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
        assert!(!log_try_create(c""));
    }
}
