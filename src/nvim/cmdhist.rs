//! Command-line history (`src/nvim/cmdhist.c`).

use crate::charset::vim_strsize;
use crate::errors::{E_TRAILING_ARG, E_VAL_TOO_LARGE};
use crate::eval::typval::{
    EvalFuncData, TypVal, VarType, tv_get_number, tv_get_number_chk, tv_get_string_buf,
    tv_get_string_chk, tv_set_string,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{CMOD_KEEPPATTERNS, Exarg};
use crate::ex_getln::{get_cmdline_firstc, get_list_range};
use crate::globals::*;
use crate::memory::xstrlcpy;
use crate::message::{
    message_filtered, msg, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts_title, semsg,
    trunc_string,
};
use crate::option_vars::*;
use crate::os::env::ExpandCtx;
use crate::os::time::os_time;
use crate::regexp::{
    NSUBEXP, NUMBUFLEN, RE_MAGIC, RE_STRING, Regmatch, vim_regcomp, vim_regexec, vim_regfree,
};
use std::ffi::CStr;
use std::ptr;

pub const HIST_DEFAULT: i32 = -2;
pub const HIST_INVALID: i32 = -1;
pub const HIST_CMD: i32 = 0;
pub const HIST_SEARCH: i32 = 1;
pub const HIST_EXPR: i32 = 2;
pub const HIST_INPUT: i32 = 3;
pub const HIST_DEBUG: i32 = 4;
pub const HIST_COUNT: usize = 5;

struct CmdhistState {
    history: [Vec<Histentry>; HIST_COUNT],
    hisidx: [i32; HIST_COUNT],
    hisnum: [i32; HIST_COUNT],
    hislen: i32,
    last_maptick: i32,
}

impl CmdhistState {
    const fn new() -> Self {
        Self {
            history: [const { Vec::new() }; HIST_COUNT],
            hisidx: [-1; HIST_COUNT],
            hisnum: [0; HIST_COUNT],
            hislen: 0,
            last_maptick: -1,
        }
    }
}

static HIST_STATE: crate::globals::GlobalState<CmdhistState> =
    crate::globals::GlobalState::new(CmdhistState::new());

#[inline]
fn hist_st() -> &'static mut CmdhistState {
    unsafe { &mut *HIST_STATE.as_ptr() }
}

const HISTORY_NAMES: [&CStr; HIST_COUNT] = [c"cmd", c"search", c"expr", c"input", c"debug"];
const HISTORY_EXPAND_ARGS: [&CStr; 12] = [
    c":", c"=", c"@", c">", c"?", c"/", c"cmd", c"search", c"expr", c"input", c"debug", c"all",
];

#[derive(Clone)]
pub struct Histentry {
    pub hisnum: i32,
    pub hisstr: Option<std::ffi::CString>,
    pub sep: u8,
    pub timestamp: u64,
    pub additional_data: Option<crate::search::AdditionalData>,
}

impl Histentry {
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
    pub fn hisstr_cstr(&self) -> Option<&CStr> {
        self.hisstr.as_deref()
    }

    #[inline]
    pub fn hisstrlen(&self) -> usize {
        self.hisstr.as_ref().map_or(0, |s| s.to_bytes().len())
    }
}

impl Default for Histentry {
    fn default() -> Self {
        Self::new()
    }
}

use crate::globals::IOSIZE;

#[inline]
fn clear_hist_entry(hisptr: *mut Histentry) {
    unsafe {
        *hisptr = Histentry::new();
    }
}

#[inline]
fn hist_free_entry(hisptr: *mut Histentry) {
    clear_hist_entry(hisptr);
}

/// Return the length of the history tables (`get_hislen`).
pub fn get_hislen() -> i32 {
    hist_st().hislen
}

/// Return a slice of a specified history table (`get_histentry`).
pub fn get_histentry(hist_type: i32) -> &'static [Histentry] {
    unsafe { (&raw const hist_st().history[hist_type as usize]).as_ref() }
        .unwrap()
        .as_slice()
}

/// Return the current index for `hist_type` (`get_hisidx`).
pub fn get_hisidx(hist_type: i32) -> i32 {
    hist_st().hisidx[hist_type as usize]
}

/// Translate a history character to the associated `HistoryType` (`hist_char2type`).
pub const fn hist_char2type(c: i32) -> i32 {
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
pub fn get_history_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    usize::try_from(idx)
        .ok()
        .and_then(|i| HISTORY_EXPAND_ARGS.get(i).copied())
}

/// Initialize or resize command-line history tables (`init_history`).
pub fn init_history() {
    assert!(opt_vars().p_hi >= 0 && opt_vars().p_hi <= (i32::MAX as i64));
    let newlen = opt_vars().p_hi as i32;
    let oldlen = hist_st().hislen;

    if newlen == oldlen {
        return;
    }

    for hist_type in 0..HIST_COUNT {
        let mut temp = vec![Histentry::new(); newlen.max(0) as usize];

        let j = hist_st().hisidx[hist_type];
        if j >= 0 {
            let l1 = (j + 1).min(newlen);
            let l2 = newlen.min(oldlen) - l1;
            let i1 = j + 1 - l1;
            let i2 = l1.max(oldlen - newlen + l1);

            if newlen > 0 {
                if l2 > 0 {
                    temp[..l2 as usize].clone_from_slice(
                        &hist_st().history[hist_type][i2 as usize..(i2 + l2) as usize],
                    );
                }
                if l1 > 0 {
                    temp[l2 as usize..(l2 + l1) as usize].clone_from_slice(
                        &hist_st().history[hist_type][i1 as usize..(i1 + l1) as usize],
                    );
                }
            }

            for i in 0..i1 {
                hist_free_entry(&raw mut hist_st().history[hist_type][i as usize]);
            }
            for i in (i1 + l1)..i2 {
                hist_free_entry(&raw mut hist_st().history[hist_type][i as usize]);
            }
        }

        let l3 = if j < 0 { 0 } else { newlen.min(oldlen) };
        hist_st().hisidx[hist_type] = l3 - 1;
        hist_st().history[hist_type] = temp;
    }
    hist_st().hislen = newlen;
}

fn in_history(hist_type: usize, str_bytes: &[u8], move_to_front: bool, sep: i32) -> bool {
    let cur_idx = hist_st().hisidx[hist_type];
    if cur_idx < 0 {
        return false;
    }
    let hislen = hist_st().hislen;
    let mut last_i: i32 = -1;
    let mut i = cur_idx;

    loop {
        let entry = &hist_st().history[hist_type][i as usize];
        let Some(ref hisstr) = entry.hisstr else {
            return false;
        };
        let p_bytes = hisstr.to_bytes();
        let sep_matches = (hist_type as i32) != HIST_SEARCH || (sep as u8) == entry.sep;
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

    let matched = hist_st().history[hist_type][i as usize].clone();
    while i != hist_st().hisidx[hist_type] {
        i += 1;
        if i >= hislen {
            i = 0;
        }
        hist_st().history[hist_type][last_i as usize] =
            hist_st().history[hist_type][i as usize].clone();
        last_i = i;
    }
    hist_st().hisnum[hist_type] += 1;
    let dst = &mut hist_st().history[hist_type][i as usize];
    dst.hisnum = hist_st().hisnum[hist_type];
    dst.hisstr = matched.hisstr;
    dst.sep = matched.sep;
    dst.timestamp = os_time();
    dst.additional_data = None;
    true
}

fn get_histtype(name: &[u8], return_default: bool) -> i32 {
    if name.is_empty() {
        return if return_default {
            HIST_DEFAULT
        } else {
            hist_char2type(get_cmdline_firstc())
        };
    }

    for (i, &hname) in HISTORY_NAMES.iter().enumerate() {
        let hb = hname.to_bytes();
        if name.len() <= hb.len() && name.eq_ignore_ascii_case(&hb[..name.len()]) {
            return i as i32;
        }
    }

    let b0 = name[0];
    if name.len() == 1 && matches!(b0, b':' | b'=' | b'@' | b'>' | b'?' | b'/') {
        return hist_char2type(b0 as i32);
    }

    HIST_INVALID
}

/// Add the given string to the given history (`add_to_history`).
pub fn add_to_history(
    histype: i32,
    new_entry: impl crate::strings::IntoCCharPtr,
    new_entrylen: usize,
    in_map: bool,
    sep: i32,
) {
    let new_entry = new_entry.into_c_char_ptr();
    let hislen = hist_st().hislen;
    if hislen == 0 || histype == HIST_INVALID {
        return;
    }
    assert_ne!(histype, HIST_DEFAULT);

    if (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0 && histype == HIST_SEARCH {
        return;
    }

    if histype == HIST_SEARCH && in_map {
        if g_st().maptick == hist_st().last_maptick && hist_st().hisidx[HIST_SEARCH as usize] >= 0 {
            let sidx = hist_st().hisidx[HIST_SEARCH as usize] as usize;
            hist_free_entry(&raw mut hist_st().history[HIST_SEARCH as usize][sidx]);
            hist_st().hisnum[histype as usize] -= 1;
            hist_st().hisidx[HIST_SEARCH as usize] -= 1;
            if hist_st().hisidx[HIST_SEARCH as usize] < 0 {
                hist_st().hisidx[HIST_SEARCH as usize] = hislen - 1;
            }
        }
        hist_st().last_maptick = -1;
    }

    let htype = histype as usize;
    {
        let entry_bytes = crate::memory::cstr_as_bytes(new_entry);
        let actual_len = entry_bytes.len().min(new_entrylen);
        let entry_slice = &entry_bytes[..actual_len];
        if in_history(htype, entry_slice, true, sep) {
            return;
        }

        hist_st().hisidx[htype] += 1;
        if hist_st().hisidx[htype] == hislen {
            hist_st().hisidx[htype] = 0;
        }
        let hisptr = &mut hist_st().history[htype][hist_st().hisidx[htype] as usize];
        hist_free_entry(hisptr);

        hisptr.hisstr = Some(std::ffi::CString::new(entry_slice).unwrap());
        hisptr.sep = sep as u8;
        hisptr.timestamp = os_time();
        hisptr.additional_data = None;
        hist_st().hisnum[htype] += 1;
        hisptr.hisnum = hist_st().hisnum[htype];

        if histype == HIST_SEARCH && in_map {
            hist_st().last_maptick = g_st().maptick;
        }
    }
}

fn get_history_idx(histype: i32) -> i32 {
    let hislen = hist_st().hislen;
    if hislen == 0 || histype < 0 || (histype as usize) >= HIST_COUNT {
        return -1;
    }
    let idx = hist_st().hisidx[histype as usize];
    if idx < 0 {
        return -1;
    }
    hist_st().history[histype as usize][idx as usize].hisnum
}

fn calc_hist_idx(histype: i32, num: i32) -> i32 {
    let hislen = hist_st().hislen;
    if hislen == 0 || histype < 0 || (histype as usize) >= HIST_COUNT || num == 0 {
        return -1;
    }
    let mut i = hist_st().hisidx[histype as usize];
    if i < 0 {
        return -1;
    }

    let hist = hist_st().history[histype as usize].as_slice();
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
pub fn clr_history(histype: i32) -> bool {
    let hislen = hist_st().hislen;
    if hislen != 0 && histype >= 0 && (histype as usize) < HIST_COUNT {
        let htype = histype as usize;
        for entry in hist_st().history[htype].iter_mut().take(hislen as usize) {
            hist_free_entry(entry);
        }
        hist_st().hisidx[htype] = -1;
        hist_st().hisnum[htype] = 0;
        true
    } else {
        false
    }
}

fn del_history_entry(histype: i32, str_ptr: &CStr) -> i32 {
    unsafe {
        let hislen = hist_st().hislen;
        if hislen == 0
            || histype < 0
            || (histype as usize) >= HIST_COUNT
            || str_ptr.to_bytes().is_empty()
            || hist_st().hisidx[histype as usize] < 0
        {
            return 0;
        }

        let htype = histype as usize;
        let idx = hist_st().hisidx[htype];
        let mut regmatch = Regmatch {
            regprog: vim_regcomp(str_ptr.as_ptr(), RE_MAGIC + RE_STRING),
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
            let hisptr = &raw mut hist_st().history[htype][i as usize];
            let Some(ref hisstr) = (*hisptr).hisstr else {
                break;
            };
            if vim_regexec(&mut regmatch, hisstr.as_ptr(), 0) {
                found = true;
                hist_free_entry(hisptr);
            } else {
                if i != last {
                    hist_st().history[htype][last as usize] = (*hisptr).clone();
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

        if hist_st().history[htype][idx as usize].hisstr.is_none() {
            hist_st().hisidx[htype] = -1;
        }

        vim_regfree(regmatch.regprog);
        i32::from(found)
    }
}

fn del_history_idx(histype: i32, idx_arg: i32) -> i32 {
    let mut i = calc_hist_idx(histype, idx_arg);
    if i < 0 {
        return 0;
    }
    let htype = histype as usize;
    let idx = hist_st().hisidx[htype];
    let hislen = hist_st().hislen;
    hist_free_entry(&raw mut hist_st().history[htype][i as usize]);

    if histype == HIST_SEARCH && g_st().maptick == hist_st().last_maptick && i == idx {
        hist_st().last_maptick = -1;
    }

    while i != idx {
        let j = (i + 1) % hislen;
        hist_st().history[htype][i as usize] = hist_st().history[htype][j as usize].clone();
        i = j;
    }
    clear_hist_entry(&raw mut hist_st().history[htype][idx as usize]);
    i -= 1;
    if i < 0 {
        i += hislen;
    }
    hist_st().hisidx[htype] = i;
    1
}

/// `"histadd()"` function (`f_histadd`).
pub fn f_histadd(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);
    if check_secure() {
        return;
    }
    let str_ptr = tv_get_string_chk(&argvars[0]);
    let histype = if !str_ptr.is_null() {
        get_histtype(crate::memory::cstr_as_bytes(str_ptr), false)
    } else {
        HIST_INVALID
    };
    if histype == HIST_INVALID {
        return;
    }

    let mut buf = [0u8; NUMBUFLEN];
    let entry_str = tv_get_string_buf(&argvars[1], &mut buf);
    if entry_str.to_bytes().is_empty() {
        return;
    }

    init_history();
    add_to_history(histype, entry_str, entry_str.to_bytes().len(), false, 0);
    rettv.vval = crate::eval::typval::TypvalVval::Number(1);
}

/// `"histdel()"` function (`f_histdel`).
pub fn f_histdel(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let str_ptr = tv_get_string_chk(&argvars[0]);
    let n = if str_ptr.is_null() {
        0
    } else {
        let htype = get_histtype(crate::memory::cstr_as_bytes(str_ptr), false);
        let arg1 = &argvars[1];
        if arg1.v_type == VarType::VarUnknown {
            i32::from(clr_history(htype))
        } else if arg1.v_type == VarType::VarNumber {
            del_history_idx(htype, tv_get_number(arg1) as i32)
        } else {
            let mut buf = [0u8; NUMBUFLEN];
            let pat = tv_get_string_buf(arg1, &mut buf);
            del_history_entry(htype, pat)
        }
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(n as i64);
}

/// `"histget()"` function (`f_histget`).
pub fn f_histget(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let str_ptr = tv_get_string_chk(&argvars[0]);
    if str_ptr.is_null() {
        tv_set_string(rettv, std::ptr::null());
    } else {
        let htype = get_histtype(crate::memory::cstr_as_bytes(str_ptr), false);
        let arg1 = &argvars[1];
        let req_idx = if arg1.v_type == VarType::VarUnknown {
            get_history_idx(htype)
        } else {
            tv_get_number_chk(arg1, None) as i32
        };
        let idx = calc_hist_idx(htype, req_idx);
        if idx < 0 {
            tv_set_string(rettv, c"");
        } else {
            let entry = &hist_st().history[htype as usize][idx as usize];
            tv_set_string(rettv, entry.hisstr_cstr().map_or(ptr::null(), CStr::as_ptr));
        }
    }
}

/// `"histnr()"` function (`f_histnr`).
pub fn f_histnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let histname = tv_get_string_chk(&argvars[0]);
    let htype = if histname.is_null() {
        HIST_INVALID
    } else {
        get_histtype(crate::memory::cstr_as_bytes(histname), false)
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(if htype != HIST_INVALID {
        get_history_idx(htype) as i64
    } else {
        HIST_INVALID as i64
    });
}

/// `:history` command (`ex_history`).
pub fn ex_history(eap: &mut Exarg) {
    let mut histype1 = HIST_CMD;
    let mut histype2 = HIST_CMD;
    let mut hisidx1: i32 = 1;
    let mut hisidx2: i32 = -1;
    let arg = eap.arg;

    msg_ext_set_kind(c"list_cmd");
    let hislen = hist_st().hislen;
    if hislen == 0 {
        msg(c"'history' option is zero", 0);
        return;
    }

    let b0 = crate::memory::cstr_first_byte(arg);
    let mut end = arg;
    if !(b0.is_ascii_digit() || b0 == b'-' || b0 == b',') {
        while (crate::memory::cstr_first_byte(end)).is_ascii_alphabetic()
            || matches!(
                crate::memory::cstr_first_byte(end),
                b':' | b'=' | b'@' | b'>' | b'/' | b'?'
            )
        {
            end = end.wrapping_add(1);
        }
        let name_len = (end as usize) - (arg as usize);
        let name_slice = crate::memory::cstr_slice(arg, name_len);
        histype1 = get_histtype(name_slice, false);
        if histype1 == HIST_INVALID {
            if name_slice.len() <= 3 && name_slice.eq_ignore_ascii_case(&b"all"[..name_slice.len()])
            {
                histype1 = 0;
                histype2 = (HIST_COUNT as i32) - 1;
            } else {
                semsg!(E_TRAILING_ARG, arg);
                return;
            }
        } else {
            histype2 = histype1;
        }
    }

    if !get_list_range(&mut end, &mut hisidx1, &mut hisidx2) || unsafe { *end } != 0 {
        if unsafe { *end } != 0 {
            semsg!(E_TRAILING_ARG, end);
        } else {
            semsg!(E_VAL_TOO_LARGE, arg);
        }
        return;
    }

    let mut iobuf = [0u8; IOSIZE];
    while !g_st().got_int && histype1 <= histype2 {
        let hname = HISTORY_NAMES[histype1 as usize].to_str().unwrap();
        let title = format!("\n      #  {hname} history\0");
        msg_puts_title(title.as_ptr().cast());

        let idx = hist_st().hisidx[histype1 as usize];
        let hist = hist_st().history[histype1 as usize].as_slice();
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
                let entry_ptr = entry.hisstr_cstr().map_or(ptr::null(), CStr::as_ptr);
                if !entry_ptr.is_null()
                    && entry.hisnum >= j
                    && entry.hisnum <= k
                    && !message_filtered(entry_ptr)
                {
                    msg_putchar(b'\n' as i32);
                    let marker = if i == idx { '>' } else { ' ' };
                    let prefix = format!("{marker}{:6}  ", entry.hisnum);
                    let prefix_bytes = prefix.as_bytes();
                    let len = prefix_bytes.len();
                    iobuf[..len].copy_from_slice(prefix_bytes);
                    if vim_strsize(entry_ptr) > g_st().columns_global - 10 {
                        trunc_string(
                            entry_ptr,
                            iobuf.as_mut_ptr().wrapping_add(len),
                            g_st().columns_global - 10,
                            (IOSIZE - len) as i32,
                        );
                    } else {
                        xstrlcpy(
                            iobuf.as_mut_ptr().wrapping_add(len).cast(),
                            entry_ptr,
                            IOSIZE - len,
                        );
                    }
                    msg_outtrans(iobuf.as_ptr().cast(), 0, false);
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

/// Iterate over history items (`hist_iter`).
pub fn hist_iter(
    iter: Option<usize>,
    history_type: u8,
    zero: bool,
    hist: &mut Histentry,
) -> Option<usize> {
    *hist = Histentry::new();
    let htype = history_type as usize;
    let last = hist_st().hisidx[htype];
    if last == -1 {
        return None;
    }
    let hislen = hist_st().hislen as usize;
    let last_idx = last as usize;
    let history = hist_st().history[htype].as_mut_slice();

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

/// Get array of history items (`hist_get_array`).
pub fn hist_get_array(
    history_type: u8,
) -> (&'static mut [Histentry], &'static mut i32, &'static mut i32) {
    init_history();
    let htype = history_type as usize;
    (
        hist_st().history[htype].as_mut_slice(),
        &mut hist_st().hisidx[htype],
        &mut hist_st().hisnum[htype],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hist_char2type_maps_all_prefixes() {
        assert_eq!(hist_char2type(b':' as i32), HIST_CMD);
        assert_eq!(hist_char2type(b'=' as i32), HIST_EXPR);
        assert_eq!(hist_char2type(b'@' as i32), HIST_INPUT);
        assert_eq!(hist_char2type(b'>' as i32), HIST_DEBUG);
        assert_eq!(hist_char2type(b'/' as i32), HIST_SEARCH);
        assert_eq!(hist_char2type(b'?' as i32), HIST_SEARCH);
        assert_eq!(hist_char2type(0), HIST_SEARCH);
        assert_eq!(hist_char2type(b'x' as i32), HIST_INVALID);
    }
}
