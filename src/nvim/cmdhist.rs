//! Command-line history (`src/nvim/cmdhist.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::charset::vim_strsize;
#[allow(unused_imports)]
use crate::eval::typval::{tv_get_number, tv_get_number_chk};
#[allow(unused_imports)]
use crate::eval::typval::{tv_get_string_buf, tv_get_string_chk};
#[allow(unused_imports)]
use crate::ex_cmds::check_secure;
#[allow(unused_imports)]
use crate::ex_getln::{get_cmdline_firstc, get_list_range};
use crate::globals::*;
use crate::message::semsg;
#[allow(unused_imports)]
use crate::message::{
    message_filtered, msg, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts_title,
    trunc_string,
};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::regexp::{vim_regcomp, vim_regexec, vim_regfree};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicI32, Ordering};

use crate::clipboard::{VarType, typval_T};
use crate::eval::deprecated::EvalFuncData;
use crate::lua::secure::exarg_T;
use crate::math::{FAIL, OK};
use crate::memory::{xfree, xmalloc, xmallocz, xstrlcpy};
use crate::os::time::os_time;
use crate::profile::expand_T;

pub const HIST_DEFAULT: c_int = -2;
pub const HIST_INVALID: c_int = -1;
pub const HIST_CMD: c_int = 0;
pub const HIST_SEARCH: c_int = 1;
pub const HIST_EXPR: c_int = 2;
pub const HIST_INPUT: c_int = 3;
pub const HIST_DEBUG: c_int = 4;
pub const HIST_COUNT: usize = 5;

const NUMBUFLEN: usize = 65;
const IOSIZE: usize = 1025;
const NSUBEXP: usize = 10;

const CMOD_KEEPPATTERNS: c_int = 0x1000;
const RE_MAGIC: c_int = 1;
const RE_STRING: c_int = 2;

static mut HISTORY: [*mut histentry_T; HIST_COUNT] = [std::ptr::null_mut(); HIST_COUNT];
static mut HISIDX: [c_int; HIST_COUNT] = [-1; HIST_COUNT];
static mut HISNUM: [c_int; HIST_COUNT] = [0; HIST_COUNT];
static HISLEN: AtomicI32 = AtomicI32::new(0);
static LAST_MAPTICK: AtomicI32 = AtomicI32::new(-1);

const HISTORY_NAMES: [&CStr; HIST_COUNT] = [c"cmd", c"search", c"expr", c"input", c"debug"];
const HISTORY_EXPAND_ARGS: [&CStr; 12] = [
    c":", c"=", c"@", c">", c"?", c"/", c"cmd", c"search", c"expr", c"input", c"debug", c"all",
];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct histentry_T {
    pub hisnum: c_int,
    pub hisstr: *mut c_char,
    pub hisstrlen: usize,
    pub timestamp: u64,
    pub additional_data: *mut c_void,
}

pub use crate::api::command::cmdmod_T;
pub use crate::api::command::regmatch_T;

#[inline]
unsafe fn strnicmp_asc(mut s1: *const c_char, mut s2: *const c_char, mut len: usize) -> bool {
    unsafe {
        while len > 0 {
            let c1 = (*s1 as u8).to_ascii_lowercase();
            let c2 = (*s2 as u8).to_ascii_lowercase();
            if c1 != c2 {
                return false;
            }
            if c1 == 0 {
                break;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            len -= 1;
        }
        true
    }
}

#[inline]
unsafe fn clear_hist_entry(hisptr: *mut histentry_T) {
    unsafe {
        *hisptr = histentry_T {
            hisnum: 0,
            hisstr: std::ptr::null_mut(),
            hisstrlen: 0,
            timestamp: 0,
            additional_data: std::ptr::null_mut(),
        };
    }
}

#[inline]
unsafe fn hist_free_entry(hisptr: *mut histentry_T) {
    unsafe {
        xfree((*hisptr).hisstr.cast::<c_void>());
        xfree((*hisptr).additional_data);
        clear_hist_entry(hisptr);
    }
}

/// Return the length of the history tables (`get_hislen`).
#[unsafe(no_mangle)]
pub extern "C" fn get_hislen() -> c_int {
    HISLEN.load(Ordering::Relaxed)
}

/// Return a pointer to a specified history table (`get_histentry`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_histentry(hist_type: c_int) -> *mut histentry_T {
    unsafe { HISTORY[hist_type as usize] }
}

/// Set the pointer to a specified history table (`set_histentry`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_histentry(hist_type: c_int, entry: *mut histentry_T) {
    unsafe {
        HISTORY[hist_type as usize] = entry;
    }
}

/// Return a pointer to the current index for `hist_type` (`get_hisidx`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_hisidx(hist_type: c_int) -> *mut c_int {
    unsafe { std::ptr::addr_of_mut!(HISIDX[hist_type as usize]) }
}

/// Return a pointer to the current counter for `hist_type` (`get_hisnum`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_hisnum(hist_type: c_int) -> *mut c_int {
    unsafe { std::ptr::addr_of_mut!(HISNUM[hist_type as usize]) }
}

/// Translate a history character to the associated `HistoryType` (`hist_char2type`).
#[unsafe(no_mangle)]
pub const extern "C" fn hist_char2type(c: c_int) -> c_int {
    match c as u8 {
        b':' => HIST_CMD,
        b'=' => HIST_EXPR,
        b'@' => HIST_INPUT,
        b'>' => HIST_DEBUG,
        0 | b'/' | b'?' => HIST_SEARCH,
        _ => HIST_INVALID,
    }
}

/// Obtain possible first arguments of `:history` for completion (`get_history_arg`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_history_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx >= 0 && (idx as usize) < HISTORY_EXPAND_ARGS.len() {
        HISTORY_EXPAND_ARGS[idx as usize].as_ptr().cast_mut()
    } else {
        std::ptr::null_mut()
    }
}

/// Initialize or resize command-line history tables (`init_history`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_history() {
    #[cfg(not(test))]
    unsafe {
        assert!(p_hi >= 0 && p_hi <= (c_int::MAX as i64));
        let newlen = p_hi as c_int;
        let oldlen = HISLEN.load(Ordering::Relaxed);

        if newlen == oldlen {
            return;
        }

        let elem_size = std::mem::size_of::<histentry_T>();
        for hist_type in 0..HIST_COUNT {
            let temp = if newlen > 0 {
                xmalloc((newlen as usize) * elem_size).cast::<histentry_T>()
            } else {
                std::ptr::null_mut()
            };

            let j = HISIDX[hist_type];
            if j >= 0 {
                let l1 = (j + 1).min(newlen);
                let l2 = newlen.min(oldlen) - l1;
                let i1 = j + 1 - l1;
                let i2 = l1.max(oldlen - newlen + l1);

                if newlen > 0 {
                    if l2 > 0 {
                        std::ptr::copy_nonoverlapping(
                            HISTORY[hist_type].add(i2 as usize),
                            temp,
                            l2 as usize,
                        );
                    }
                    if l1 > 0 {
                        std::ptr::copy_nonoverlapping(
                            HISTORY[hist_type].add(i1 as usize),
                            temp.add(l2 as usize),
                            l1 as usize,
                        );
                    }
                }

                for i in 0..i1 {
                    hist_free_entry(HISTORY[hist_type].add(i as usize));
                }
                for i in (i1 + l1)..i2 {
                    hist_free_entry(HISTORY[hist_type].add(i as usize));
                }
            }

            let l3 = if j < 0 { 0 } else { newlen.min(oldlen) };
            if newlen > l3 {
                std::ptr::write_bytes(
                    temp.add(l3 as usize).cast::<u8>(),
                    0,
                    ((newlen - l3) as usize) * elem_size,
                );
            }

            HISIDX[hist_type] = l3 - 1;
            xfree(HISTORY[hist_type].cast::<c_void>());
            HISTORY[hist_type] = temp;
        }
        HISLEN.store(newlen, Ordering::Relaxed);
    }
}

unsafe fn in_history(
    hist_type: usize,
    str_ptr: *const c_char,
    move_to_front: bool,
    sep: c_int,
) -> bool {
    unsafe {
        let cur_idx = HISIDX[hist_type];
        if cur_idx < 0 {
            return false;
        }
        let hislen = HISLEN.load(Ordering::Relaxed);
        let str_bytes = CStr::from_ptr(str_ptr).to_bytes();
        let mut last_i: c_int = -1;
        let mut i = cur_idx;

        loop {
            let entry = &*HISTORY[hist_type].add(i as usize);
            if entry.hisstr.is_null() {
                return false;
            }
            let p_bytes = CStr::from_ptr(entry.hisstr).to_bytes();
            let sep_matches = (hist_type as c_int) != HIST_SEARCH
                || (sep as c_char) == *entry.hisstr.add(entry.hisstrlen + 1);
            if str_bytes == p_bytes && sep_matches {
                if !move_to_front {
                    return true;
                }
                last_i = i;
                break;
            }
            i -= 1;
            if i < 0 {
                i = hislen - 1;
            }
            if i == cur_idx {
                break;
            }
        }

        if last_i < 0 {
            return false;
        }

        let matched = *HISTORY[hist_type].add(i as usize);
        while i != HISIDX[hist_type] {
            i += 1;
            if i >= hislen {
                i = 0;
            }
            *HISTORY[hist_type].add(last_i as usize) = *HISTORY[hist_type].add(i as usize);
            last_i = i;
        }
        xfree(matched.additional_data);
        HISNUM[hist_type] += 1;
        let dst = &mut *HISTORY[hist_type].add(i as usize);
        dst.hisnum = HISNUM[hist_type];
        dst.hisstr = matched.hisstr;
        dst.hisstrlen = matched.hisstrlen;
        dst.timestamp = os_time();
        dst.additional_data = std::ptr::null_mut();
        true
    }
}

#[cfg(not(test))]
unsafe fn get_histtype(name: *const c_char, len: usize, return_default: bool) -> c_int {
    unsafe {
        if len == 0 {
            return if return_default {
                HIST_DEFAULT
            } else {
                hist_char2type(get_cmdline_firstc())
            };
        }

        for (i, &hname) in HISTORY_NAMES.iter().enumerate() {
            if strnicmp_asc(name, hname.as_ptr(), len) {
                return i as c_int;
            }
        }

        let b0 = *name as u8;
        if len == 1 && matches!(b0, b':' | b'=' | b'@' | b'>' | b'?' | b'/') {
            return hist_char2type(b0 as c_int);
        }

        HIST_INVALID
    }
}

/// Add the given string to the given history (`add_to_history`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn add_to_history(
    histype: c_int,
    new_entry: *const c_char,
    new_entrylen: usize,
    in_map: bool,
    sep: c_int,
) {
    #[cfg(test)]
    let _ = in_map;

    let hislen = HISLEN.load(Ordering::Relaxed);
    if hislen == 0 || histype == HIST_INVALID {
        return;
    }
    assert_ne!(histype, HIST_DEFAULT);

    #[cfg(not(test))]
    unsafe {
        if (cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0 && histype == HIST_SEARCH {
            return;
        }

        if histype == HIST_SEARCH && in_map {
            if maptick == LAST_MAPTICK.load(Ordering::Relaxed) && HISIDX[HIST_SEARCH as usize] >= 0
            {
                let sidx = HISIDX[HIST_SEARCH as usize] as usize;
                hist_free_entry(HISTORY[HIST_SEARCH as usize].add(sidx));
                HISNUM[histype as usize] -= 1;
                HISIDX[HIST_SEARCH as usize] -= 1;
                if HISIDX[HIST_SEARCH as usize] < 0 {
                    HISIDX[HIST_SEARCH as usize] = hislen - 1;
                }
            }
            LAST_MAPTICK.store(-1, Ordering::Relaxed);
        }
    }

    let htype = histype as usize;
    unsafe {
        if in_history(htype, new_entry, true, sep) {
            return;
        }

        HISIDX[htype] += 1;
        if HISIDX[htype] == hislen {
            HISIDX[htype] = 0;
        }
        let hisptr = &mut *HISTORY[htype].add(HISIDX[htype] as usize);
        hist_free_entry(hisptr);

        let buf = xmallocz(new_entrylen + 2).cast::<c_char>();
        let actual_len = CStr::from_ptr(new_entry).to_bytes().len().min(new_entrylen);
        if actual_len > 0 {
            std::ptr::copy_nonoverlapping(new_entry.cast::<u8>(), buf.cast::<u8>(), actual_len);
        }
        std::ptr::write_bytes(
            buf.add(actual_len).cast::<u8>(),
            0,
            new_entrylen + 1 - actual_len,
        );
        *buf.add(new_entrylen + 1) = sep as c_char;

        hisptr.hisstr = buf;
        hisptr.timestamp = os_time();
        hisptr.additional_data = std::ptr::null_mut();
        hisptr.hisstrlen = new_entrylen;
        HISNUM[htype] += 1;
        hisptr.hisnum = HISNUM[htype];

        #[cfg(not(test))]
        if histype == HIST_SEARCH && in_map {
            LAST_MAPTICK.store(maptick, Ordering::Relaxed);
        }
    }
}

unsafe fn get_history_idx(histype: c_int) -> c_int {
    let hislen = HISLEN.load(Ordering::Relaxed);
    if hislen == 0 || histype < 0 || (histype as usize) >= HIST_COUNT {
        return -1;
    }
    let idx = unsafe { HISIDX[histype as usize] };
    if idx < 0 {
        return -1;
    }
    unsafe { (*HISTORY[histype as usize].add(idx as usize)).hisnum }
}

unsafe fn calc_hist_idx(histype: c_int, num: c_int) -> c_int {
    let hislen = HISLEN.load(Ordering::Relaxed);
    if hislen == 0 || histype < 0 || (histype as usize) >= HIST_COUNT || num == 0 {
        return -1;
    }
    let mut i = unsafe { HISIDX[histype as usize] };
    if i < 0 {
        return -1;
    }

    let hist = unsafe { HISTORY[histype as usize] };
    if num > 0 {
        let mut wrapped = false;
        while unsafe { (*hist.add(i as usize)).hisnum } > num {
            i -= 1;
            if i < 0 {
                if wrapped {
                    break;
                }
                i += hislen;
                wrapped = true;
            }
        }
        if i >= 0
            && unsafe { (*hist.add(i as usize)).hisnum } == num
            && unsafe { !(*hist.add(i as usize)).hisstr.is_null() }
        {
            return i;
        }
    } else if -num <= hislen {
        i += num + 1;
        if i < 0 {
            i += hislen;
        }
        if unsafe { !(*hist.add(i as usize)).hisstr.is_null() } {
            return i;
        }
    }
    -1
}

/// Clear all entries in a history (`clr_history`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clr_history(histype: c_int) -> c_int {
    let hislen = HISLEN.load(Ordering::Relaxed);
    if hislen != 0 && histype >= 0 && (histype as usize) < HIST_COUNT {
        let htype = histype as usize;
        unsafe {
            let hisptr = HISTORY[htype];
            for i in 0..(hislen as usize) {
                hist_free_entry(hisptr.add(i));
            }
            HISIDX[htype] = -1;
            HISNUM[htype] = 0;
        }
        OK
    } else {
        FAIL
    }
}

#[cfg(not(test))]
unsafe fn del_history_entry(histype: c_int, str_ptr: *mut c_char) -> c_int {
    unsafe {
        let hislen = HISLEN.load(Ordering::Relaxed);
        if hislen == 0
            || histype < 0
            || (histype as usize) >= HIST_COUNT
            || *str_ptr == 0
            || HISIDX[histype as usize] < 0
        {
            return 0;
        }

        let htype = histype as usize;
        let idx = HISIDX[htype];
        let mut regmatch = regmatch_T {
            regprog: vim_regcomp(str_ptr, RE_MAGIC + RE_STRING),
            startp: [std::ptr::null_mut(); NSUBEXP],
            endp: [std::ptr::null_mut(); NSUBEXP],
            rm_matchcol: 0,
            rm_ic: false,
        };
        if regmatch.regprog.is_null() {
            return 0;
        }

        let mut found = false;
        let mut i = idx;
        let mut last = idx;
        loop {
            let hisptr = HISTORY[htype].add(i as usize);
            if (*hisptr).hisstr.is_null() {
                break;
            }
            if vim_regexec(&mut regmatch, (*hisptr).hisstr, 0) {
                found = true;
                hist_free_entry(hisptr);
            } else {
                if i != last {
                    *HISTORY[htype].add(last as usize) = *hisptr;
                    clear_hist_entry(hisptr);
                }
                last -= 1;
                if last < 0 {
                    last += hislen;
                }
            }
            i -= 1;
            if i < 0 {
                i += hislen;
            }
            if i == idx {
                break;
            }
        }

        if (*HISTORY[htype].add(idx as usize)).hisstr.is_null() {
            HISIDX[htype] = -1;
        }

        vim_regfree(regmatch.regprog);
        c_int::from(found)
    }
}

unsafe fn del_history_idx(histype: c_int, idx_arg: c_int) -> c_int {
    unsafe {
        let mut i = calc_hist_idx(histype, idx_arg);
        if i < 0 {
            return 0;
        }
        let htype = histype as usize;
        let idx = HISIDX[htype];
        let hislen = HISLEN.load(Ordering::Relaxed);
        hist_free_entry(HISTORY[htype].add(i as usize));

        #[cfg(not(test))]
        if histype == HIST_SEARCH && maptick == LAST_MAPTICK.load(Ordering::Relaxed) && i == idx {
            LAST_MAPTICK.store(-1, Ordering::Relaxed);
        }

        while i != idx {
            let j = (i + 1) % hislen;
            *HISTORY[htype].add(i as usize) = *HISTORY[htype].add(j as usize);
            i = j;
        }
        clear_hist_entry(HISTORY[htype].add(idx as usize));
        i -= 1;
        if i < 0 {
            i += hislen;
        }
        HISIDX[htype] = i;
        1
    }
}

/// `"histadd()"` function (`f_histadd`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_histadd(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = 0;
        if check_secure() {
            return;
        }
        let str_ptr = tv_get_string_chk(argvars);
        let histype = if !str_ptr.is_null() {
            get_histtype(str_ptr, CStr::from_ptr(str_ptr).to_bytes().len(), false)
        } else {
            HIST_INVALID
        };
        if histype == HIST_INVALID {
            return;
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let entry_str = tv_get_string_buf(argvars.add(1), buf.as_mut_ptr());
        if *entry_str == 0 {
            return;
        }

        init_history();
        add_to_history(
            histype,
            entry_str,
            CStr::from_ptr(entry_str).to_bytes().len(),
            false,
            0,
        );
        (*rettv).vval.v_number = 1;
    }
}

/// `"histdel()"` function (`f_histdel`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_histdel(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        let str_ptr = tv_get_string_chk(argvars);
        let n = if str_ptr.is_null() {
            0
        } else {
            let htype = get_histtype(str_ptr, CStr::from_ptr(str_ptr).to_bytes().len(), false);
            let arg1 = &*argvars.add(1);
            if arg1.v_type == VarType::VAR_UNKNOWN {
                clr_history(htype)
            } else if arg1.v_type == VarType::VAR_NUMBER {
                del_history_idx(htype, tv_get_number(arg1) as c_int)
            } else {
                let mut buf = [0 as c_char; NUMBUFLEN];
                let pat = tv_get_string_buf(arg1, buf.as_mut_ptr());
                del_history_entry(htype, pat.cast_mut())
            }
        };
        (*rettv).vval.v_number = n as i64;
    }
}

/// `"histget()"` function (`f_histget`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_histget(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        let str_ptr = tv_get_string_chk(argvars);
        if str_ptr.is_null() {
            (*rettv).vval.v_string = std::ptr::null_mut();
        } else {
            let htype = get_histtype(str_ptr, CStr::from_ptr(str_ptr).to_bytes().len(), false);
            let arg1 = &*argvars.add(1);
            let req_idx = if arg1.v_type == VarType::VAR_UNKNOWN {
                get_history_idx(htype)
            } else {
                tv_get_number_chk(arg1, std::ptr::null_mut()) as c_int
            };
            let idx = calc_hist_idx(htype, req_idx);
            if idx < 0 {
                (*rettv).vval.v_string = xmallocz(0).cast::<c_char>();
            } else {
                let entry = &*HISTORY[htype as usize].add(idx as usize);
                let out = xmallocz(entry.hisstrlen).cast::<c_char>();
                let actual = CStr::from_ptr(entry.hisstr)
                    .to_bytes()
                    .len()
                    .min(entry.hisstrlen);
                if actual > 0 {
                    std::ptr::copy_nonoverlapping(
                        entry.hisstr.cast::<u8>(),
                        out.cast::<u8>(),
                        actual,
                    );
                }
                *out.add(actual) = 0;
                (*rettv).vval.v_string = out;
            }
        }
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// `"histnr()"` function (`f_histnr`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_histnr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        let histname = tv_get_string_chk(argvars);
        let htype = if histname.is_null() {
            HIST_INVALID
        } else {
            get_histtype(histname, CStr::from_ptr(histname).to_bytes().len(), false)
        };
        (*rettv).vval.v_number = if htype != HIST_INVALID {
            get_history_idx(htype) as i64
        } else {
            HIST_INVALID as i64
        };
    }
}

/// `:history` command (`ex_history`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_history(eap: *mut exarg_T) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
    unsafe {
        let mut histype1 = HIST_CMD;
        let mut histype2 = HIST_CMD;
        let mut hisidx1: c_int = 1;
        let mut hisidx2: c_int = -1;
        let arg = (*eap).arg;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        let hislen = HISLEN.load(Ordering::Relaxed);
        if hislen == 0 {
            msg(c"'history' option is zero".as_ptr(), 0);
            return;
        }

        let b0 = *arg as u8;
        let mut end = arg;
        if !(b0.is_ascii_digit() || b0 == b'-' || b0 == b',') {
            while (*end as u8).is_ascii_alphabetic()
                || matches!(*end as u8, b':' | b'=' | b'@' | b'>' | b'/' | b'?')
            {
                end = end.add(1);
            }
            let name_len = (end as usize) - (arg as usize);
            histype1 = get_histtype(arg, name_len, false);
            if histype1 == HIST_INVALID {
                if strnicmp_asc(arg, c"all".as_ptr(), name_len) {
                    histype1 = 0;
                    histype2 = (HIST_COUNT as c_int) - 1;
                } else {
                    semsg!(c"E488: Trailing characters: %s".as_ptr(), arg);
                    return;
                }
            } else {
                histype2 = histype1;
            }
        }

        if get_list_range(&mut end, &mut hisidx1, &mut hisidx2) == 0 || *end != 0 {
            if *end != 0 {
                semsg!(c"E488: Trailing characters: %s".as_ptr(), end);
            } else {
                semsg!(c"E1510: Value too large: %s".as_ptr(), arg);
            }
            return;
        }

        let mut iobuf = [0 as c_char; IOSIZE];
        while !got_int && histype1 <= histype2 {
            let hname = HISTORY_NAMES[histype1 as usize].to_str().unwrap();
            let title = format!("\n      #  {hname} history\0");
            msg_puts_title(title.as_ptr().cast::<c_char>());

            let idx = HISIDX[histype1 as usize];
            let hist = HISTORY[histype1 as usize];
            let mut j = hisidx1;
            let mut k = hisidx2;
            if j < 0 {
                j = if -j > hislen {
                    0
                } else {
                    (*hist.add(((hislen + j + idx + 1) % hislen) as usize)).hisnum
                };
            }
            if k < 0 {
                k = if -k > hislen {
                    0
                } else {
                    (*hist.add(((hislen + k + idx + 1) % hislen) as usize)).hisnum
                };
            }
            if idx >= 0 && j <= k {
                let mut i = idx + 1;
                while !got_int {
                    if i == hislen {
                        i = 0;
                    }
                    let entry = &*hist.add(i as usize);
                    if !entry.hisstr.is_null()
                        && entry.hisnum >= j
                        && entry.hisnum <= k
                        && !message_filtered(entry.hisstr)
                    {
                        msg_putchar(b'\n' as c_int);
                        let marker = if i == idx { '>' } else { ' ' };
                        let prefix = format!("{marker}{:6}  ", entry.hisnum);
                        let prefix_bytes = prefix.as_bytes();
                        std::ptr::copy_nonoverlapping(
                            prefix_bytes.as_ptr(),
                            iobuf.as_mut_ptr().cast::<u8>(),
                            prefix_bytes.len(),
                        );
                        let len = prefix_bytes.len();
                        if vim_strsize(entry.hisstr) > Columns - 10 {
                            trunc_string(
                                entry.hisstr,
                                iobuf.as_mut_ptr().add(len),
                                Columns - 10,
                                (IOSIZE - len) as c_int,
                            );
                        } else {
                            xstrlcpy(iobuf.as_mut_ptr().add(len), entry.hisstr, IOSIZE - len);
                        }
                        msg_outtrans(iobuf.as_ptr(), 0, false);
                    }
                    if i == idx {
                        break;
                    }
                    i += 1;
                }
            }
            histype1 += 1;
        }
    }
}

/// Iterate over history items (`hist_iter`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hist_iter(
    iter: *const c_void,
    history_type: u8,
    zero: bool,
    hist: *mut histentry_T,
) -> *const c_void {
    unsafe {
        *hist = histentry_T {
            hisnum: 0,
            hisstr: std::ptr::null_mut(),
            hisstrlen: 0,
            timestamp: 0,
            additional_data: std::ptr::null_mut(),
        };
        let htype = history_type as usize;
        if HISIDX[htype] == -1 {
            return std::ptr::null();
        }
        let hislen = HISLEN.load(Ordering::Relaxed);
        let hstart = HISTORY[htype];
        let hlast = HISTORY[htype].add(HISIDX[htype] as usize);
        let hend = HISTORY[htype].add((hislen - 1) as usize);

        let mut hiter = if iter.is_null() {
            let mut hfirst = hlast;
            loop {
                hfirst = hfirst.add(1);
                if (hfirst as usize) > (hend as usize) {
                    hfirst = hstart;
                }
                if !(*hfirst).hisstr.is_null() || hfirst == hlast {
                    break;
                }
            }
            hfirst
        } else {
            iter.cast_mut().cast::<histentry_T>()
        };

        if hiter.is_null() {
            return std::ptr::null();
        }
        *hist = *hiter;
        if zero {
            clear_hist_entry(hiter);
        }
        if hiter == hlast {
            return std::ptr::null();
        }
        hiter = hiter.add(1);
        if (hiter as usize) > (hend as usize) {
            hstart.cast::<c_void>()
        } else {
            hiter.cast::<c_void>()
        }
    }
}

/// Get array of history items (`hist_get_array`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hist_get_array(
    history_type: u8,
    new_hisidx: *mut *mut c_int,
    new_hisnum: *mut *mut c_int,
) -> *mut histentry_T {
    unsafe {
        init_history();
        let htype = history_type as usize;
        *new_hisidx = std::ptr::addr_of_mut!(HISIDX[htype]);
        *new_hisnum = std::ptr::addr_of_mut!(HISNUM[htype]);
        HISTORY[htype]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hist_char2type_maps_all_prefixes() {
        assert_eq!(hist_char2type(b':' as c_int), HIST_CMD);
        assert_eq!(hist_char2type(b'=' as c_int), HIST_EXPR);
        assert_eq!(hist_char2type(b'@' as c_int), HIST_INPUT);
        assert_eq!(hist_char2type(b'>' as c_int), HIST_DEBUG);
        assert_eq!(hist_char2type(b'/' as c_int), HIST_SEARCH);
        assert_eq!(hist_char2type(b'?' as c_int), HIST_SEARCH);
        assert_eq!(hist_char2type(0), HIST_SEARCH);
        assert_eq!(hist_char2type(b'x' as c_int), HIST_INVALID);
    }
}
