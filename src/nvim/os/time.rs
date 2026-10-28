//! High-resolution timers, delays, and local-time formatting (`src/nvim/os/time.c`).

use crate::event::r#loop::loop_poll_events;
use crate::globals::*;
use crate::memory::xstrlcpy;
#[cfg(unix)]
use crate::os::env::os_getenv_buf;
use crate::os::input::os_input_ready;
use std::ffi::{CStr, c_char, c_int, c_uint};

pub type Timestamp = u64;
pub type time_t = i64;

pub const NS_PER_SEC: i64 = 1_000_000_000;
const UV_CLOCK_REALTIME: c_int = 1;

use crate::event::r#loop::{UvTimespec64, uv_clock_gettime, uv_hrtime, uv_now, uv_sleep};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Tm {
    pub _fields: [c_int; 16],
}

#[cfg(unix)]
struct OsTimeState {
    TZ_CACHE: [u8; 64],
}

#[cfg(unix)]
impl OsTimeState {
    const fn new() -> Self {
        Self {
            TZ_CACHE: [0u8; 64],
        }
    }
}

#[cfg(unix)]
static OSTIME_STATE: crate::globals::GlobalState<OsTimeState> = crate::globals::GlobalState::new(OsTimeState::new());

#[cfg(unix)]
#[inline]
fn ostime_st() -> &'static mut OsTimeState {
    unsafe { &mut *OSTIME_STATE.as_ptr() }
}

unsafe extern "C" {
    #[cfg_attr(target_os = "windows", link_name = "_mktime64")]
    pub fn mktime(tm: *mut Tm) -> time_t;
    pub fn strftime(s: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;

    #[cfg(unix)]
    fn tzset();
    #[cfg(unix)]
    fn localtime_r(clock: *const time_t, result: *mut Tm) -> *mut Tm;
    #[cfg(unix)]
    fn strptime(s: *const c_char, format: *const c_char, tm: *mut Tm) -> *mut c_char;

    #[cfg(target_os = "windows")]
    #[link_name = "_localtime64"]
    fn win_localtime64(clock: &time_t) -> *mut [c_int; 9];
}

/// High-resolution monotonic clock in nanoseconds (`os_hrtime`).
pub fn os_hrtime() -> u64 {
    unsafe { uv_hrtime() }
}

/// High-resolution real-time clock in nanoseconds since Unix epoch (`os_realtime`).
pub fn os_realtime() -> i64 {
    unsafe {
        let mut ts = UvTimespec64 {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if uv_clock_gettime(UV_CLOCK_REALTIME, &mut ts) != 0 {
            return 0;
        }
        ts.tv_sec * NS_PER_SEC + i64::from(ts.tv_nsec)
    }
}

/// Millisecond-resolution monotonic loop time (`os_now`).
pub fn os_now() -> u64 {
    unsafe { uv_now((&raw const g_st().main_loop).cast()) }
}

/// Sleep for `ms` milliseconds while polling the main event loop (`os_delay`).
pub fn os_delay(ms: u64, ignoreinput: bool) {
    unsafe {
        let clamped = ms.min(u64::from(c_int::MAX as u32)) as i64;
        let mut remaining = clamped;
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };

        loop {
            let cond = if ignoreinput {
                g_st().got_int
            } else {
                os_input_ready(std::ptr::null_mut())
            };
            if cond {
                break;
            }
            loop_poll_events(&mut *(&raw mut g_st().main_loop).cast(), remaining);
            if remaining == 0 {
                break;
            } else if remaining > 0 {
                let now = os_hrtime();
                remaining -= ((now - before) / 1_000_000) as i64;
                before = now;
                if remaining <= 0 {
                    break;
                }
            }
        }
    }
}

/// Block for `ms` milliseconds without processing events (`os_sleep`).
pub fn os_sleep(ms: u64) {
    unsafe {
        let clamped = ms.min(u64::from(c_uint::MAX)) as c_uint;
        uv_sleep(clamped);
    }
}

/// Portable `localtime_r()` (`os_localtime_r`).
///
/// # Safety
/// `clock` and `result` must be valid pointers.
pub fn os_localtime_r(clock: &time_t, result: *mut Tm) -> *mut Tm {
    #[cfg(unix)]
    unsafe {
        let mut tz_buf = [0 as c_char; 64];
        let tz_ptr = os_getenv_buf(c"TZ".as_ptr(), tz_buf.as_mut_ptr(), tz_buf.len());
        let tz_bytes = if tz_ptr.is_null() {
            &[][..]
        } else {
            CStr::from_ptr(tz_ptr).to_bytes()
        };
        let cache_ptr = &raw mut ostime_st().TZ_CACHE;
        let cache = &mut *cache_ptr;
        let cache_len = cache.iter().position(|&b| b == 0).unwrap_or(cache.len());
        let cmp_len = tz_bytes.len().min(cache.len() - 1);
        if cache_len != cmp_len || cache[..cache_len] != tz_bytes[..cmp_len] {
            tzset();
            cache.fill(0);
            cache[..cmp_len].copy_from_slice(&tz_bytes[..cmp_len]);
        }
        localtime_r(clock, result)
    }

    #[cfg(target_os = "windows")]
    unsafe {
        let local_time = win_localtime64(clock);
        if local_time.is_null() {
            return std::ptr::null_mut();
        }
        *result.cast::<[c_int; 9]>() = *local_time;
        result
    }
}

/// Get the current local time (`os_localtime`).
///
/// # Safety
/// `result` must be a valid writable pointer to `struct tm`.
pub fn os_localtime(result: *mut Tm) -> *mut Tm {
    let rawtime = os_time() as time_t;
    os_localtime_r(&rawtime, result)
}

/// Format `clock` as local time into `result[..result_len]` (`os_ctime_r`).
///
/// # Safety
/// `clock` and `result` must be valid pointers, with `result` writable for `result_len` bytes.
pub fn os_ctime_r(
    clock: &time_t,
    result: *mut c_char,
    result_len: usize,
    add_newline: bool,
) -> *mut c_char {
    let mut clock_local = Tm { _fields: [0; 16] };
    let ptr = os_localtime_r(clock, &mut clock_local);

    unsafe {
        if ptr.is_null()
            || strftime(
                result,
                result_len.saturating_sub(1),
                c"%a %b %d %H:%M:%S %Y".as_ptr(),
                ptr,
            ) == 0
        {
            xstrlcpy(result, c"(Invalid)".as_ptr(), result_len);
        }
    }

    if add_newline && result_len > 0 {
        unsafe {
            let cur_len = CStr::from_ptr(result).to_bytes().len();
            if cur_len + 1 < result_len {
                *result.add(cur_len) = b'\n' as c_char;
                *result.add(cur_len + 1) = 0;
            }
        }
    }
    result
}

/// Portable wrapper for POSIX `strptime()` (`os_strptime`).
///
/// # Safety
/// `str_ptr`, `format`, and `tm` must be valid pointers.
pub fn os_strptime(
    str_ptr: *const c_char,
    format: *const c_char,
    tm: *mut Tm,
) -> *mut c_char {
    #[cfg(unix)]
    unsafe {
        strptime(str_ptr, format, tm)
    }
    #[cfg(not(unix))]
    {
        let _ = (str_ptr, format, tm);
        std::ptr::null_mut()
    }
}

/// Obtain current Unix timestamp in seconds (`os_time`).
pub fn os_time() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_time_and_ctime_r_work() {
        assert!(os_time() > 0);
        let mut buf = [0 as c_char; 64];
        let t: time_t = 0;
        unsafe {
            let out = os_ctime_r(&t, buf.as_mut_ptr(), buf.len(), true);
            let s = CStr::from_ptr(out).to_str().unwrap();
            assert!(s.ends_with('\n'));
            assert!(s.contains("1969") || s.contains("1970"));
        }
    }
}
