//! Command-line history (`src/nvim/cmdhist.c`).

use crate::charset::vim_strsize;
use crate::errors::{e_trailing_arg, e_val_too_large};
use crate::eval::typval::{
    EvalFuncData, VarType, tv_get_number, tv_get_number_chk, tv_get_string_buf, tv_get_string_chk,
    tv_set_string, typval_T,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{CMOD_KEEPPATTERNS, exarg_T};
use crate::ex_getln::{get_cmdline_firstc, get_list_range};
use crate::globals::*;
use crate::math::{FAIL, OK};
use crate::memory::xstrlcpy;
use crate::message::{
    message_filtered, msg, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts_title, semsg,
    trunc_string,
};
use crate::option_vars::*;
use crate::os::env::expand_T;
use crate::os::time::os_time;
use crate::regexp::{
    NSUBEXP, NUMBUFLEN, RE_MAGIC, RE_STRING, regmatch_T, vim_regcomp, vim_regexec, vim_regfree,
};
use std::ffi::{CStr, c_char, c_int};

pub const HIST_DEFAULT: c_int = -2;
pub const HIST_INVALID: c_int = -1;
pub const HIST_CMD: c_int = 0;
pub const HIST_SEARCH: c_int = 1;
pub const HIST_EXPR: c_int = 2;
pub const HIST_INPUT: c_int = 3;
pub const HIST_DEBUG: c_int = 4;
pub const HIST_COUNT: usize = 5;

struct CmdhistState {
    HISTORY: [Vec<histentry_T>; HIST_COUNT],
    HISIDX: [c_int; HIST_COUNT],
    HISNUM: [c_int; HIST_COUNT],
    HISLEN: c_int,
    LAST_MAPTICK: c_int,
}

impl CmdhistState {
    const fn new() -> Self {
        Self {
            HISTORY: [const { Vec::new() }; HIST_COUNT],
            HISIDX: [-1; HIST_COUNT],
            HISNUM: [0; HIST_COUNT],
            HISLEN: 0,
            LAST_MAPTICK: -1,
        }
    }
}

static HIST_STATE: crate::globals::GlobalState<CmdhistState> = crate::globals::GlobalState::new(CmdhistState::new());

#[inline]
fn hist_st() -> &'static mut CmdhistState {
    unsafe { &mut *HIST_STATE.as_ptr() }
}

const HISTORY_NAMES: [&CStr; HIST_COUNT] = [c"cmd", c"search", c"expr", c"input", c"debug"];
const HISTORY_EXPAND_ARGS: [&CStr; 12] = [
    c":", c"=", c"@", c">", c"?", c"/", c"cmd", c"search", c"expr", c"input", c"debug", c"all",
];

#[derive(Clone)]
pub struct histentry_T {
    pub hisnum: c_int,
    pub hisstr: Option<std::ffi::CString>,
    pub sep: c_char,
    pub timestamp: u64,
    pub additional_data: Option<crate::search::AdditionalData>,
}

impl histentry_T {
    pub const fn new() -> Self {
        Self {
            hisnum: 0,
            hisstr: None,
            sep: 0,
            timestamp: 0,
            additional_data: None,
        }
    }

    #[inline]
    pub fn hisstr_ptr(&self) -> *mut c_char {
        self.hisstr
            .as_ref()
            .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn hisstrlen(&self) -> usize {
        self.hisstr.as_ref().map_or(0, |s| s.to_bytes().len())
    }
}

impl Default for histentry_T {
    fn default() -> Self {
        Self::new()
    }
}

use crate::globals::IOSIZE;

#[inline]
unsafe fn clear_hist_entry(hisptr: *mut histentry_T) {
    unsafe {
        *hisptr = histentry_T::new();
    }
}

#[inline]
unsafe fn hist_free_entry(hisptr: *mut histentry_T) {
    unsafe {
        clear_hist_entry(hisptr);
    }
}

/// Return the length of the history tables (`get_hislen`).
pub fn get_hislen() -> c_int {
    hist_st().HISLEN
}

/// Return a slice of a specified history table (`get_histentry`).
pub fn get_histentry(hist_type: c_int) -> &'static [histentry_T] {
    unsafe {
        (&raw const hist_st().HISTORY[hist_type as usize])
            .as_ref()
            .unwrap()
            .as_slice()
    }
}

/// Return the current index for `hist_type` (`get_hisidx`).
pub fn get_hisidx(hist_type: c_int) -> c_int {
    hist_st().HISIDX[hist_type as usize]
}

/// Translate a history character to the associated `HistoryType` (`hist_char2type`).
pub const fn hist_char2type(c: c_int) -> c_int {
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
pub fn get_history_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx >= 0 && (idx as usize) < HISTORY_EXPAND_ARGS.len() {
        HISTORY_EXPAND_ARGS[idx as usize].as_ptr().cast_mut()
    } else {
        std::ptr::null_mut()
    }
}

/// Initialize or resize command-line history tables (`init_history`).
pub fn init_history() {
    unsafe {
        assert!(opt_vars().p_hi >= 0 && opt_vars().p_hi <= (c_int::MAX as i64));
        let newlen = opt_vars().p_hi as c_int;
        let oldlen = hist_st().HISLEN;

        if newlen == oldlen {
            return;
        }

        for hist_type in 0..HIST_COUNT {
            let mut temp = vec![histentry_T::new(); newlen.max(0) as usize];

            let j = hist_st().HISIDX[hist_type];
            if j >= 0 {
                let l1 = (j + 1).min(newlen);
                let l2 = newlen.min(oldlen) - l1;
                let i1 = j + 1 - l1;
                let i2 = l1.max(oldlen - newlen + l1);

                if newlen > 0 {
                    if l2 > 0 {
                        temp[..l2 as usize]
                            .clone_from_slice(&hist_st().HISTORY[hist_type][i2 as usize..(i2 + l2) as usize]);
                    }
                    if l1 > 0 {
                        temp[l2 as usize..(l2 + l1) as usize]
                            .clone_from_slice(&hist_st().HISTORY[hist_type][i1 as usize..(i1 + l1) as usize]);
                    }
                }

                for i in 0..i1 {
                    hist_free_entry(&raw mut hist_st().HISTORY[hist_type][i as usize]);
                }
                for i in (i1 + l1)..i2 {
                    hist_free_entry(&raw mut hist_st().HISTORY[hist_type][i as usize]);
                }
            }

            let l3 = if j < 0 { 0 } else { newlen.min(oldlen) };
            hist_st().HISIDX[hist_type] = l3 - 1;
            hist_st().HISTORY[hist_type] = temp;
        }
        hist_st().HISLEN = newlen;
    }
}

unsafe fn in_history(
    hist_type: usize,
    str_ptr: *const c_char,
    move_to_front: bool,
    sep: c_int,
) -> bool {
    unsafe {
        let cur_idx = hist_st().HISIDX[hist_type];
        if cur_idx < 0 {
            return false;
        }
        let hislen = hist_st().HISLEN;
        let str_bytes = CStr::from_ptr(str_ptr).to_bytes();
        let mut last_i: c_int = -1;
        let mut i = cur_idx;

        loop {
            let entry = &hist_st().HISTORY[hist_type][i as usize];
            let Some(ref hisstr) = entry.hisstr else {
                return false;
            };
            let p_bytes = hisstr.to_bytes();
            let sep_matches = (hist_type as c_int) != HIST_SEARCH || (sep as c_char) == entry.sep;
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

        let matched = hist_st().HISTORY[hist_type][i as usize].clone();
        while i != hist_st().HISIDX[hist_type] {
            i += 1;
            if i >= hislen {
                i = 0;
            }
            hist_st().HISTORY[hist_type][last_i as usize] = hist_st().HISTORY[hist_type][i as usize].clone();
            last_i = i;
        }
        hist_st().HISNUM[hist_type] += 1;
        let dst = &mut hist_st().HISTORY[hist_type][i as usize];
        dst.hisnum = hist_st().HISNUM[hist_type];
        dst.hisstr = matched.hisstr;
        dst.sep = matched.sep;
        dst.timestamp = os_time();
        dst.additional_data = None;
        true
    }
}

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
            if crate::memory::cstr_prefix_bytes(name, len).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(hname.as_ptr(), len)) {
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
pub unsafe fn add_to_history(
    histype: c_int,
    new_entry: *const c_char,
    new_entrylen: usize,
    in_map: bool,
    sep: c_int,
) {
    let hislen = hist_st().HISLEN;
    if hislen == 0 || histype == HIST_INVALID {
        return;
    }
    assert_ne!(histype, HIST_DEFAULT);

    unsafe {
        if (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0 && histype == HIST_SEARCH {
            return;
        }

        if histype == HIST_SEARCH && in_map {
            if g_st().maptick == hist_st().LAST_MAPTICK && hist_st().HISIDX[HIST_SEARCH as usize] >= 0 {
                let sidx = hist_st().HISIDX[HIST_SEARCH as usize] as usize;
                hist_free_entry(&raw mut hist_st().HISTORY[HIST_SEARCH as usize][sidx]);
                hist_st().HISNUM[histype as usize] -= 1;
                hist_st().HISIDX[HIST_SEARCH as usize] -= 1;
                if hist_st().HISIDX[HIST_SEARCH as usize] < 0 {
                    hist_st().HISIDX[HIST_SEARCH as usize] = hislen - 1;
                }
            }
            hist_st().LAST_MAPTICK = -1;
        }
    }

    let htype = histype as usize;
    unsafe {
        if in_history(htype, new_entry, true, sep) {
            return;
        }

        hist_st().HISIDX[htype] += 1;
        if hist_st().HISIDX[htype] == hislen {
            hist_st().HISIDX[htype] = 0;
        }
        let hisptr = &mut hist_st().HISTORY[htype][hist_st().HISIDX[htype] as usize];
        hist_free_entry(hisptr);

        let entry_bytes = CStr::from_ptr(new_entry).to_bytes();
        let actual_len = entry_bytes.len().min(new_entrylen);
        hisptr.hisstr = Some(std::ffi::CString::new(&entry_bytes[..actual_len]).unwrap());
        hisptr.sep = sep as c_char;
        hisptr.timestamp = os_time();
        hisptr.additional_data = None;
        hist_st().HISNUM[htype] += 1;
        hisptr.hisnum = hist_st().HISNUM[htype];

        if histype == HIST_SEARCH && in_map {
            hist_st().LAST_MAPTICK = g_st().maptick;
        }
    }
}

fn get_history_idx(histype: c_int) -> c_int {
    let hislen = hist_st().HISLEN;
    if hislen == 0 || histype < 0 || (histype as usize) >= HIST_COUNT {
        return -1;
    }
    let idx = hist_st().HISIDX[histype as usize];
    if idx < 0 {
        return -1;
    }
    hist_st().HISTORY[histype as usize][idx as usize].hisnum
}

fn calc_hist_idx(histype: c_int, num: c_int) -> c_int {
    let hislen = hist_st().HISLEN;
    if hislen == 0 || histype < 0 || (histype as usize) >= HIST_COUNT || num == 0 {
        return -1;
    }
    let mut i = hist_st().HISIDX[histype as usize];
    if i < 0 {
        return -1;
    }

    let hist = hist_st().HISTORY[histype as usize].as_slice();
    if num > 0 {
        let mut wrapped = false;
        while hist[i as usize].hisnum > num {
            i -= 1;
            if i < 0 {
                if wrapped {
                    break;
                }
                i += hislen;
                wrapped = true;
            }
        }
        if i >= 0 && hist[i as usize].hisnum == num && hist[i as usize].hisstr.is_some() {
            return i;
        }
    } else if -num <= hislen {
        i += num + 1;
        if i < 0 {
            i += hislen;
        }
        if hist[i as usize].hisstr.is_some() {
            return i;
        }
    }
    -1
}

/// Clear all entries in a history (`clr_history`).
pub fn clr_history(histype: c_int) -> c_int {
    let hislen = hist_st().HISLEN;
    if hislen != 0 && histype >= 0 && (histype as usize) < HIST_COUNT {
        let htype = histype as usize;
        unsafe {
            for entry in hist_st().HISTORY[htype].iter_mut().take(hislen as usize) {
                hist_free_entry(entry);
            }
            hist_st().HISIDX[htype] = -1;
            hist_st().HISNUM[htype] = 0;
        }
        OK
    } else {
        FAIL
    }
}

unsafe fn del_history_entry(histype: c_int, str_ptr: *mut c_char) -> c_int {
    unsafe {
        let hislen = hist_st().HISLEN;
        if hislen == 0
            || histype < 0
            || (histype as usize) >= HIST_COUNT
            || *str_ptr == 0
            || hist_st().HISIDX[histype as usize] < 0
        {
            return 0;
        }

        let htype = histype as usize;
        let idx = hist_st().HISIDX[htype];
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
            let hisptr = &raw mut hist_st().HISTORY[htype][i as usize];
            let Some(ref hisstr) = (*hisptr).hisstr else {
                break;
            };
            if vim_regexec(&mut regmatch, hisstr.as_ptr(), 0) {
                found = true;
                hist_free_entry(hisptr);
            } else {
                if i != last {
                    hist_st().HISTORY[htype][last as usize] = (*hisptr).clone();
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

        if hist_st().HISTORY[htype][idx as usize].hisstr.is_none() {
            hist_st().HISIDX[htype] = -1;
        }

        vim_regfree(regmatch.regprog);
        c_int::from(found)
    }
}

fn del_history_idx(histype: c_int, idx_arg: c_int) -> c_int {
    unsafe {
        let mut i = calc_hist_idx(histype, idx_arg);
        if i < 0 {
            return 0;
        }
        let htype = histype as usize;
        let idx = hist_st().HISIDX[htype];
        let hislen = hist_st().HISLEN;
        hist_free_entry(&raw mut hist_st().HISTORY[htype][i as usize]);

        if histype == HIST_SEARCH && g_st().maptick == hist_st().LAST_MAPTICK && i == idx {
            hist_st().LAST_MAPTICK = -1;
        }

        while i != idx {
            let j = (i + 1) % hislen;
            hist_st().HISTORY[htype][i as usize] = hist_st().HISTORY[htype][j as usize].clone();
            i = j;
        }
        clear_hist_entry(&raw mut hist_st().HISTORY[htype][idx as usize]);
        i -= 1;
        if i < 0 {
            i += hislen;
        }
        hist_st().HISIDX[htype] = i;
        1
    }
}

/// `"histadd()"` function (`f_histadd`).
pub unsafe fn f_histadd(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
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
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    }
}

/// `"histdel()"` function (`f_histdel`).
pub unsafe fn f_histdel(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
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
        rettv.vval = crate::eval::typval::TypvalVval::Number(n as i64);
    }
}

/// `"histget()"` function (`f_histget`).
pub unsafe fn f_histget(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let str_ptr = tv_get_string_chk(argvars);
        if str_ptr.is_null() {
            tv_set_string(rettv, std::ptr::null());
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
                tv_set_string(rettv, c"".as_ptr());
            } else {
                let entry = &hist_st().HISTORY[htype as usize][idx as usize];
                tv_set_string(rettv, entry.hisstr_ptr());
            }
        }
    }
}

/// `"histnr()"` function (`f_histnr`).
pub unsafe fn f_histnr(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let histname = tv_get_string_chk(argvars);
        let htype = if histname.is_null() {
            HIST_INVALID
        } else {
            get_histtype(histname, CStr::from_ptr(histname).to_bytes().len(), false)
        };
        rettv.vval = crate::eval::typval::TypvalVval::Number(if htype != HIST_INVALID {
            get_history_idx(htype) as i64
        } else {
            HIST_INVALID as i64
        });
    }
}

/// `:history` command (`ex_history`).
pub fn ex_history(eap: &mut exarg_T) {
    unsafe {
        let mut histype1 = HIST_CMD;
        let mut histype2 = HIST_CMD;
        let mut hisidx1: c_int = 1;
        let mut hisidx2: c_int = -1;
        let arg = eap.arg;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        let hislen = hist_st().HISLEN;
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
                if crate::memory::cstr_prefix_bytes(arg, name_len).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"all".as_ptr(), name_len)) {
                    histype1 = 0;
                    histype2 = (HIST_COUNT as c_int) - 1;
                } else {
                    semsg!(e_trailing_arg.as_ptr(), arg);
                    return;
                }
            } else {
                histype2 = histype1;
            }
        }

        if get_list_range(&mut end, &mut hisidx1, &mut hisidx2) == 0 || *end != 0 {
            if *end != 0 {
                semsg!(e_trailing_arg.as_ptr(), end);
            } else {
                semsg!(e_val_too_large.as_ptr(), arg);
            }
            return;
        }

        let mut iobuf = [0 as c_char; IOSIZE];
        while !g_st().got_int && histype1 <= histype2 {
            let hname = HISTORY_NAMES[histype1 as usize].to_str().unwrap();
            let title = format!("\n      #  {hname} history\0");
            msg_puts_title(title.as_ptr().cast::<c_char>());

            let idx = hist_st().HISIDX[histype1 as usize];
            let hist = hist_st().HISTORY[histype1 as usize].as_slice();
            let mut j = hisidx1;
            let mut k = hisidx2;
            if j < 0 {
                j = if -j > hislen {
                    0
                } else {
                    hist[((hislen + j + idx + 1) % hislen) as usize].hisnum
                };
            }
            if k < 0 {
                k = if -k > hislen {
                    0
                } else {
                    hist[((hislen + k + idx + 1) % hislen) as usize].hisnum
                };
            }
            if idx >= 0 && j <= k {
                let mut i = idx + 1;
                while !g_st().got_int {
                    if i == hislen {
                        i = 0;
                    }
                    let entry = &hist[i as usize];
                    let entry_ptr = entry.hisstr_ptr();
                    if !entry_ptr.is_null()
                        && entry.hisnum >= j
                        && entry.hisnum <= k
                        && !message_filtered(entry_ptr)
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
                        if vim_strsize(entry_ptr) > g_st().Columns - 10 {
                            trunc_string(
                                entry_ptr,
                                iobuf.as_mut_ptr().add(len),
                                g_st().Columns - 10,
                                (IOSIZE - len) as c_int,
                            );
                        } else {
                            xstrlcpy(iobuf.as_mut_ptr().add(len), entry_ptr, IOSIZE - len);
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
pub fn hist_iter(
    iter: Option<usize>,
    history_type: u8,
    zero: bool,
    hist: &mut histentry_T,
) -> Option<usize> {
    unsafe {
        *hist = histentry_T::new();
        let htype = history_type as usize;
        let last = hist_st().HISIDX[htype];
        if last == -1 {
            return None;
        }
        let hislen = hist_st().HISLEN as usize;
        let last_idx = last as usize;
        let history = hist_st().HISTORY[htype].as_mut_slice();

        let hiter = if let Some(idx) = iter {
            idx
        } else {
            let mut first = last_idx;
            loop {
                first += 1;
                if first >= hislen {
                    first = 0;
                }
                if history[first].hisstr.is_some() || first == last_idx {
                    break;
                }
            }
            first
        };

        *hist = history[hiter].clone();
        if zero {
            clear_hist_entry(&raw mut history[hiter]);
        }
        if hiter == last_idx {
            return None;
        }
        let next = hiter + 1;
        if next >= hislen { Some(0) } else { Some(next) }
    }
}

/// Get array of history items (`hist_get_array`).
pub fn hist_get_array(
    history_type: u8,
) -> (
    &'static mut [histentry_T],
    &'static mut c_int,
    &'static mut c_int,
) {
    init_history();
        let htype = history_type as usize;
        (
            hist_st().HISTORY[htype].as_mut_slice(),
            &mut hist_st().HISIDX[htype],
            &mut hist_st().HISNUM[htype],
        )
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
