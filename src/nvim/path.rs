#![forbid(unsafe_code)]

//! Path manipulation, comparison, canonicalization, and wildcard expansion (`src/nvim/path.c`).

use crate::api::private::validate::NvimString;
use crate::charset::{NUL, backslash_halve, backslash_halve_save, rem_backslash};
use crate::cmdexpand::{WILD_ADD_SLASH, WILD_ICASE, globpath};
use crate::diff::K_SHELL_OPT_SILENT;
use crate::eval::eval_to_string;
use crate::ex_docmd::eval_vars;
use crate::fileio::{file_pat_to_reg_pat, match_file_list};
use crate::globals::*;
#[cfg(unix)]
use crate::mbyte::mb_isalpha;
use crate::mbyte::{mb_strcmp_ic, utf_fold, utfc_ptr2len};
use crate::memory::bytes_to_cstring;
use crate::option::copy_option_part;
use crate::option_vars::*;
use crate::os::env::{expand_env_esc, expand_env_save_opt, os_getenv_cstr, vim_env_iter};
use crate::os::fs::{
    Directory, FileID, FileInfo, MAXPATHL, os_can_exe, os_closedir, os_dirname,
    os_file_is_readable, os_fileid, os_fileid_equal, os_fileinfo, os_fileinfo_id_equal,
    os_fileinfo_link, os_isdir, os_path_exists, os_realpath, os_scandir, os_scandir_next,
};
use crate::os::input::os_breakcheck;
use crate::os::shell::get_cmd_output;
#[cfg(unix)]
use crate::os::shell::os_expand_wildcards;
use crate::regexp::{RE_MAGIC, RE_NOBREAK, RE_STRING, Regmatch, vim_regcomp, vim_regexec};
use crate::strings::{format_to_buf, sort_strings};
use std::cmp::Ordering;
use std::ffi::CStr;

pub fn remove_duplicate_strings(fnames: &mut Vec<std::ffi::CString>) {
    if fnames.is_empty() {
        return;
    }
    sort_strings(fnames);
    let mut i = fnames.len() - 1;
    while i > 0 {
        if path_equal(&fnames[i - 1], &fnames[i], K_PATH_CMP_LITERAL) {
            fnames.remove(i);
        }
        i -= 1;
    }
}

pub fn concat_cstrings(strings: &[std::ffi::CString], sep: &std::ffi::CStr) -> std::ffi::CString {
    let nelem = strings.len();
    if nelem == 0 {
        return std::ffi::CString::default();
    }
    {
        let sep_bytes = sep.to_bytes();
        let mut len = (nelem - 1) * sep_bytes.len();
        for s in strings {
            len += s.as_bytes().len();
        }
        let mut out: Vec<u8> = Vec::with_capacity(len);
        for (i, s) in strings.iter().enumerate() {
            let bytes = s.as_bytes();
            if !bytes.is_empty() {
                out.extend_from_slice(bytes);
            }
            if i + 1 < nelem && !sep_bytes.is_empty() {
                out.extend_from_slice(sep_bytes);
            }
        }
        std::ffi::CString::new(out).unwrap_or_default()
    }
}

pub type PathCmpFlags = i32;

const MAXSUFLEN: usize = 30;

pub(crate) const PATHSEP: u8 = b'/';
pub(crate) const PATHSEPSTR: &[u8] = b"/\0";

pub(crate) const ENV_SEPCHAR: u8 = if cfg!(windows) { b';' } else { b':' };

#[cfg(windows)]
const PATH_ESC_WILDCARDS: &CStr = c"*?[";
#[cfg(not(windows))]
const PATH_ESC_WILDCARDS: &CStr = c"*?[{";

#[cfg(windows)]
const PATH_ALL_WILDCARDS: &CStr = c"*?[`$";
#[cfg(not(windows))]
const PATH_ALL_WILDCARDS: &CStr = c"*?[{`'$";

#[cfg(unix)]
const SPECIAL_WILDCHAR: &CStr = c"`'{";

#[cfg(windows)]
const FNAME_ILLEGAL: &[u8] = b"\"*?><|";

pub const EW_DIR: i32 = 0x01;
pub const EW_FILE: i32 = 0x02;
pub const EW_NOTFOUND: i32 = 0x04;
pub const EW_ADDSLASH: i32 = 0x08;
pub const EW_KEEPALL: i32 = 0x10;
pub const EW_SILENT: i32 = 0x20;
pub const EW_EXEC: i32 = 0x40;
pub const EW_PATH: i32 = 0x80;
pub const EW_ICASE: i32 = 0x100;
pub const EW_NOERROR: i32 = 0x200;
pub const EW_NOTWILD: i32 = 0x400;
#[cfg(unix)]
pub const EW_KEEPDOLLAR: i32 = 0x800;
pub const EW_ALLLINKS: i32 = 0x1000;
pub const EW_SHELLCMD: i32 = 0x2000;
pub const EW_DODOT: i32 = 0x4000;
pub const EW_EMPTYOK: i32 = 0x8000;
pub const EW_NOTENV: i32 = 0x10000;
pub const EW_CDPATH: i32 = 0x20000;
pub const EW_NOBREAK: i32 = 0x40000;

pub const K_PATH_CMP_LITERAL: PathCmpFlags = 1 << 0;
pub const K_PATH_CMP_EXPAND: PathCmpFlags = 1 << 1;
pub const K_PATH_CMP_FULL: PathCmpFlags = 1 << 2;

struct PathModuleState {
    scandir_count: std::cell::Cell<i32>,
    stardepth: std::cell::Cell<i32>,
    expand_recursive: std::cell::Cell<bool>,
}

impl PathModuleState {
    const fn new() -> Self {
        Self {
            scandir_count: std::cell::Cell::new(0),
            stardepth: std::cell::Cell::new(0),
            expand_recursive: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static PATH_STATE: PathModuleState = const { PathModuleState::new() };
}

#[cfg(windows)]
use crate::fileio::_getdrive;

#[inline]
pub fn to_slash(p: &mut [u8]) {
    #[cfg(windows)]
    {
        path_to_slash(p);
    }
    #[cfg(not(windows))]
    let _ = p;
}

#[inline]
pub fn to_slash_save(p: Option<&CStr>) -> Option<std::ffi::CString> {
    #[cfg(windows)]
    {
        path_to_slash_save(p)
    }
    #[cfg(not(windows))]
    {
        p.map(CStr::to_owned)
    }
}

pub fn path_equal(s1: &CStr, s2: &CStr, flags: PathCmpFlags) -> bool {
    let mut expanded_s1 = [0u8; MAXPATHL];
    let mut full1 = [0u8; MAXPATHL];
    let mut full2 = [0u8; MAXPATHL];
    let mut file_id1 = FileID {
        inode: 0,
        device_id: 0,
    };
    let mut file_id2 = FileID {
        inode: 0,
        device_id: 0,
    };

    debug_assert!((flags & K_PATH_CMP_LITERAL) == 0 || flags == K_PATH_CMP_LITERAL);

    if flags == K_PATH_CMP_LITERAL {
        let is_url = path_with_url(s1.to_bytes()) || path_with_url(s2.to_bytes());
        return (!is_url || s1.to_bytes().len() == s2.to_bytes().len())
            && path_cmp(opt_vars().p_fic.get(), s1, s2, MAXPATHL) == 0;
    }

    if (flags & K_PATH_CMP_EXPAND) != 0 {
        expand_env_esc(s1, &mut expanded_s1, None, false, None);
    } else if flags != 0 {
        crate::memory::copy_bytes_to_c_buf(&mut expanded_s1, s1.to_bytes());
    }

    let id_ok1 = os_fileid(crate::memory::bytes_as_cstr(&expanded_s1), &mut file_id1);
    let id_ok2 = os_fileid(s2, &mut file_id2);
    if id_ok1 && id_ok2 && os_fileid_equal(&file_id1, &file_id2) {
        return true;
    }
    if !id_ok1 && !id_ok2 && (flags & K_PATH_CMP_FULL) != 0 {
        vim_full_name(
            Some(crate::memory::bytes_as_cstr(&expanded_s1)),
            &mut full1,
            false,
        );
        vim_full_name(Some(s2), &mut full2, false);
        return path_cmp(
            opt_vars().p_fic.get(),
            crate::memory::bytes_as_cstr(&full1),
            crate::memory::bytes_as_cstr(&full2),
            MAXPATHL,
        ) == 0;
    }
    false
}

pub fn path_tail(fname: &CStr) -> &CStr {
    let past_head = get_past_head(fname);
    let rem = past_head.to_bytes();
    let mut tail_off = 0usize;
    for (i, &b) in rem.iter().enumerate() {
        if vim_ispathsep_nocolon(b as i32) {
            tail_off = i + 1;
        }
    }
    &past_head[tail_off..]
}

pub fn path_tail_with_sep(fname: &CStr) -> &CStr {
    let fname_bytes = fname.to_bytes();
    let past_head = get_past_head(fname);
    let past_head_off = fname_bytes.len() - past_head.to_bytes().len();
    let tail = path_tail(fname);
    let mut tail_off = fname_bytes.len() - tail.to_bytes().len();
    while tail_off > past_head_off && after_pathsep(&fname_bytes[..tail_off]) != 0 {
        tail_off -= 1;
    }
    &fname[tail_off..]
}

pub fn invocation_path_tail<'a>(invocation: &'a CStr, len: Option<&mut usize>) -> &'a CStr {
    let base = get_past_head(invocation);
    let bytes = base.to_bytes();
    let mut tail_idx = 0usize;
    let mut tail_end_idx = 0usize;
    let mut idx = 0usize;
    let mut inquote = false;
    while idx < bytes.len() && (inquote || bytes[idx] != b' ') {
        let l = utfc_ptr2len(&bytes[idx..]) as usize;
        if vim_ispathsep_nocolon(bytes[idx] as i32) {
            tail_idx = idx + 1;
            tail_end_idx = tail_idx;
        } else if bytes[idx] == b'\\' && inquote {
            idx += 1;
        } else if bytes[idx] == b'"' {
            inquote = !inquote;
        } else {
            tail_end_idx = idx + l;
        }
        idx += l;
    }

    if let Some(len) = len {
        *len = tail_end_idx.saturating_sub(tail_idx);
    }

    &base[tail_idx.min(bytes.len())..]
}

pub fn path_next_component(fname: &CStr) -> &CStr {
    let bytes = fname.to_bytes();
    let mut i = bytes
        .iter()
        .take_while(|&&b| !vim_ispathsep(b as i32))
        .count();
    if i < bytes.len() {
        i += 1;
    }
    &fname[i..]
}

pub fn path_skip_sep(path: &CStr, colon: bool) -> &CStr {
    let bytes = path.to_bytes();
    let skip = bytes
        .iter()
        .take_while(|&&b| {
            if colon {
                vim_ispathsep(b as i32)
            } else {
                vim_ispathsep_nocolon(b as i32)
            }
        })
        .count();
    &path[skip..]
}

pub fn path_head_length() -> i32 {
    #[cfg(windows)]
    {
        3
    }
    #[cfg(not(windows))]
    {
        1
    }
}

#[cfg(windows)]
fn is_path_head(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':')
}

pub fn get_past_head(path: &CStr) -> &CStr {
    #[cfg(windows)]
    let mut retval = path;
    #[cfg(not(windows))]
    let retval = path;
    #[cfg(windows)]
    if is_path_head(path.to_bytes()) {
        retval = &path[2..];
    }
    path_skip_sep(retval, false)
}

pub fn vim_ispathsep<C: crate::charset::AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    #[cfg(unix)]
    {
        ch == b'/' as i32
    }
    #[cfg(windows)]
    {
        ch == b':' as i32 || ch == b'/' as i32 || ch == b'\\' as i32
    }
}

pub fn vim_ispathsep_nocolon<C: crate::charset::AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    #[cfg(windows)]
    {
        vim_ispathsep(ch) && ch != b':' as i32
    }
    #[cfg(not(windows))]
    {
        vim_ispathsep(ch)
    }
}

pub fn vim_ispathlistsep<C: crate::charset::AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    #[cfg(unix)]
    {
        ch == b':' as i32
    }
    #[cfg(not(unix))]
    {
        ch == b';' as i32
    }
}

pub fn shorten_dir_len(str_: &mut [u8], trim_len: i32) {
    let str_cstr = crate::memory::bytes_as_cstr(str_);
    let str_len = str_cstr.to_bytes().len();
    let tail_off = str_len - path_tail(str_cstr).to_bytes().len();
    let mut d = 0usize;
    let mut skip = false;
    let mut dirchunk_len: i32 = 0;
    let mut s = 0usize;
    while s <= str_len && s < str_.len() {
        let b = str_[s];
        if s >= tail_off {
            str_[d] = b;
            d += 1;
            if b == 0 {
                break;
            }
        } else if vim_ispathsep(b) {
            str_[d] = b;
            d += 1;
            skip = false;
            dirchunk_len = 0;
        } else if !skip {
            str_[d] = b;
            d += 1;
            if b != b'~' && b != b'.' {
                dirchunk_len += 1;
                if dirchunk_len >= trim_len {
                    skip = true;
                }
            }
            let mut l = utfc_ptr2len(&str_[s..=str_len]);
            while {
                l -= 1;
                l > 0 && s < str_len
            } {
                s += 1;
                str_[d] = str_[s];
                d += 1;
            }
        }
        s += 1;
    }
}

pub fn shorten_dir(str_: &mut [u8]) {
    shorten_dir_len(str_, 1);
}

pub fn dir_of_file_exists(fname: &CStr) -> bool {
    let fname_bytes = fname.to_bytes();
    let tail = path_tail_with_sep(fname);
    let dir_len = fname_bytes.len() - tail.to_bytes().len();
    if dir_len == 0 {
        return true;
    }
    let dir_cstr = bytes_to_cstring(&fname_bytes[..dir_len]);
    os_isdir(&dir_cstr)
}

#[inline]
fn do_concat_fnames(fname1: &[u8], fname2: &[u8], sep: bool) -> std::ffi::CString {
    let has_trailing_sep = !fname1.is_empty()
        && vim_ispathsep(i32::from(fname1[fname1.len() - 1]))
        && crate::mbyte::utf_head_off_slice(fname1, fname1.len() - 1) == 0;
    let need_sep = sep && !fname1.is_empty() && !has_trailing_sep;
    let mut buf = Vec::with_capacity(fname1.len() + usize::from(need_sep) + fname2.len());
    buf.extend_from_slice(fname1);
    if need_sep {
        buf.push(PATHSEP);
    }
    buf.extend_from_slice(fname2);
    bytes_to_cstring(&buf)
}

pub fn concat_fnames(fname1: NvimString, fname2: NvimString, sep: bool) -> std::ffi::CString {
    do_concat_fnames(fname1.as_bytes(), fname2.as_bytes(), sep)
}

pub fn concat_fnames_cstr(fname1: &CStr, fname2: &CStr, sep: bool) -> std::ffi::CString {
    do_concat_fnames(fname1.to_bytes(), fname2.to_bytes(), sep)
}

pub fn add_pathsep(p: &mut [u8]) -> bool {
    let plen = crate::memory::bytes_until_nul(p).len();
    if plen == 0 {
        return true;
    }

    if after_pathsep(&p[..plen]) == 0 {
        let pathsep_len = PATHSEPSTR.len();
        if plen > MAXPATHL - pathsep_len || plen + pathsep_len > p.len() {
            return false;
        }
        p[plen..plen + pathsep_len].copy_from_slice(PATHSEPSTR);
    }
    true
}

#[allow(non_snake_case)]
pub fn FullName_save(fname: Option<&CStr>, force: bool) -> Option<std::ffi::CString> {
    let fname = fname?;
    let mut buf: Vec<u8> = vec![0; MAXPATHL];
    if !vim_full_name(Some(fname), &mut buf, force) {
        return to_slash_save(Some(fname));
    }
    Some(crate::memory::cstring_until_nul(&buf))
}

pub fn save_abs_path(name: &CStr) -> Option<std::ffi::CString> {
    if !path_is_absolute(name.to_bytes()) {
        return FullName_save(Some(name), true);
    }
    to_slash_save(Some(name))
}

pub fn path_has_wildcard(bytes: &[u8], all: bool) -> bool {
    let wildcards = if all {
        PATH_ALL_WILDCARDS.to_bytes()
    } else {
        PATH_ESC_WILDCARDS.to_bytes()
    };
    let mut i = 0usize;
    while i < bytes.len() {
        #[cfg(unix)]
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if wildcards.contains(&bytes[i]) || (all && bytes[i] == b'~' && i + 1 < bytes.len()) {
            return true;
        }
        i += 1;
    }
    false
}

fn scandir_next_with_dots(dir: Option<&mut Directory>) -> Option<&std::ffi::CStr> {
    let Some(dir) = dir else {
        PATH_STATE.with(|st| st.scandir_count.set(0));
        return None;
    };

    let c = PATH_STATE.with(|st| {
        let next = st.scandir_count.get() + 1;
        st.scandir_count.set(next);
        next
    });
    if c == 1 || c == 2 {
        return Some(if c == 1 { c"." } else { c".." });
    }
    os_scandir_next(dir)
}

fn path_expand(gap: &mut Vec<std::ffi::CString>, path: &CStr, flags: i32) -> usize {
    do_path_expand(gap, path, 0, flags, false)
}

fn do_path_expand(
    gap: &mut Vec<std::ffi::CString>,
    path: &CStr,
    wildoff: usize,
    flags: i32,
    didstar: bool,
) -> usize {
    let start_len = gap.len();
    let mut starstar = false;

    if PATH_STATE.with(|st| st.stardepth.get()) > 0 && (flags & EW_NOBREAK) == 0 {
        os_breakcheck();
        if g_st().got_int {
            return 0;
        }
    }

    let path_bytes = path.to_bytes();
    let buflen = path_bytes.len() + MAXPATHL;
    let mut buf_vec = vec![0u8; buflen];

    let mut p_idx = 0usize;
    let mut s_idx = 0usize;
    let mut e_idx: Option<usize> = None;
    let mut path_idx = 0usize;

    while path_idx < path_bytes.len() {
        let rem = &path_bytes[path_idx..];
        if path_idx >= wildoff && rem_backslash(rem) {
            buf_vec[p_idx] = rem[0];
            p_idx += 1;
            path_idx += 1;
        } else if vim_ispathsep_nocolon(path_bytes[path_idx]) {
            if e_idx.is_some() {
                break;
            }
            s_idx = p_idx + 1;
        } else if path_idx >= wildoff && {
            let is_esc_wild =
                crate::strings::vim_haschr(PATH_ESC_WILDCARDS, i32::from(path_bytes[path_idx]));
            #[cfg(not(windows))]
            let is_icase_alpha = !opt_vars().p_fic.get()
                && (flags & EW_ICASE) != 0
                && mb_isalpha(crate::mbyte::utf_ptr2char(&path_bytes[path_idx..]));
            #[cfg(windows)]
            let is_icase_alpha = false;
            is_esc_wild || is_icase_alpha
        } {
            e_idx = Some(p_idx);
        }
        let rem = &path_bytes[path_idx..];
        let charlen = utfc_ptr2len(rem) as usize;
        buf_vec[p_idx..p_idx + charlen].copy_from_slice(&rem[..charlen]);
        p_idx += charlen;
        path_idx += charlen;
    }
    let mut e_pos = p_idx;
    buf_vec[e_pos] = 0;

    let mut p_pos = wildoff;
    while p_pos < s_idx {
        if rem_backslash(&buf_vec[p_pos..e_pos]) {
            buf_vec.copy_within(p_pos + 1..=e_pos, p_pos);
            e_pos -= 1;
            s_idx -= 1;
        }
        p_pos += 1;
    }

    p_pos = s_idx;
    while p_pos < e_pos {
        if buf_vec[p_pos..e_pos].starts_with(b"**") {
            starstar = true;
        }
        p_pos += 1;
    }

    let starts_with_dot = buf_vec[s_idx] == b'.';
    let Some(pat) = file_pat_to_reg_pat(&buf_vec[s_idx..e_pos], None, 0) else {
        return 0;
    };

    let mut regmatch = Regmatch {
        #[cfg(unix)]
        rm_ic: (flags & EW_ICASE) != 0 || opt_vars().p_fic.get(),
        #[cfg(not(unix))]
        rm_ic: true,
        ..Regmatch::new()
    };

    if (flags & (EW_NOERROR | EW_NOTWILD)) != 0 {
        g_st().emsg_silent += 1;
    }
    let nobreak = (flags & EW_NOBREAK) != 0;
    regmatch.set_prog(vim_regcomp(
        &pat,
        RE_MAGIC | if nobreak { RE_NOBREAK } else { 0 },
    ));
    if (flags & (EW_NOERROR | EW_NOTWILD)) != 0 {
        g_st().emsg_silent -= 1;
    }
    drop(pat);

    if regmatch.regprog.is_null() && (flags & EW_NOTWILD) == 0 {
        return 0;
    }

    let mut len = s_idx;
    let path_end_cstr = &path[path_idx..];
    if !didstar
        && PATH_STATE.with(|st| st.stardepth.get()) < 100
        && starstar
        && e_pos - s_idx == 2
        && path_bytes.get(path_idx) == Some(&b'/')
    {
        format_to_buf!(&mut buf_vec[s_idx..], c"%s", &path_end_cstr[1..]);
        PATH_STATE.with(|st| st.stardepth.set(st.stardepth.get() + 1));
        let sub_path = crate::memory::cstring_until_nul(&buf_vec);
        do_path_expand(gap, &sub_path, len, flags, true);
        PATH_STATE.with(|st| st.stardepth.set(st.stardepth.get() - 1));
    }

    buf_vec[s_idx] = 0;

    let mut dir = Directory::new();
    let dirpath = if buf_vec[0] == 0 {
        c"."
    } else {
        crate::memory::bytes_as_cstr(&buf_vec)
    };
    if os_file_is_readable(dirpath) && os_scandir(&mut dir, dirpath) {
        scandir_next_with_dots(None);
        while !g_st().got_int {
            let Some(name) = scandir_next_with_dots(Some(&mut dir)) else {
                break;
            };
            let name_bytes = name.to_bytes();
            len = s_idx;
            let dot_ok = name_bytes.first() != Some(&b'.')
                || starts_with_dot
                || ((flags & EW_DODOT) != 0 && name_bytes != b"." && name_bytes != b"..");
            let reg_ok = (!regmatch.regprog.is_null() && vim_regexec(&mut regmatch, name, 0))
                || ((flags & EW_NOTWILD) != 0
                    && path_cmp(
                        opt_vars().p_fic.get(),
                        &path[len.min(path_bytes.len())..],
                        name,
                        e_pos - s_idx,
                    ) == 0);
            if dot_ok && reg_ok {
                len += format_to_buf!(&mut buf_vec[s_idx..], c"%s", name) as usize;
                if len + 1 >= buflen {
                    continue;
                }

                if starstar && PATH_STATE.with(|st| st.stardepth.get()) < 100 {
                    format_to_buf!(&mut buf_vec[len..], c"/**%s", path_end_cstr);
                    PATH_STATE.with(|st| st.stardepth.set(st.stardepth.get() + 1));
                    let sub_path = crate::memory::cstring_until_nul(&buf_vec);
                    do_path_expand(gap, &sub_path, len + 1, flags, true);
                    PATH_STATE.with(|st| st.stardepth.set(st.stardepth.get() - 1));
                }

                format_to_buf!(&mut buf_vec[len..], c"%s", path_end_cstr);
                if path_has_wildcard(path_end_cstr.to_bytes(), false) {
                    if PATH_STATE.with(|st| st.stardepth.get()) < 100 {
                        PATH_STATE.with(|st| st.stardepth.set(st.stardepth.get() + 1));
                        let sub_path = crate::memory::cstring_until_nul(&buf_vec);
                        do_path_expand(gap, &sub_path, len + 1, flags, false);
                        PATH_STATE.with(|st| st.stardepth.set(st.stardepth.get() - 1));
                    }
                } else {
                    let mut file_info: FileInfo = FileInfo::new();
                    if !path_end_cstr.to_bytes().is_empty() {
                        backslash_halve(&mut buf_vec[len + 1..]);
                    }
                    let buf_cstr = crate::memory::bytes_as_cstr(&buf_vec);
                    if if (flags & EW_ALLLINKS) != 0 {
                        os_fileinfo_link(buf_cstr, &mut file_info)
                    } else {
                        os_path_exists(buf_cstr)
                    } {
                        addfile(gap, buf_cstr, flags);
                    }
                }
            }
        }
        os_closedir(&mut dir);
    }

    drop(buf_vec);
    regmatch.free_prog();

    let matches = gap.len() - start_len;
    if matches > 0 && !g_st().got_int {
        gap[start_len..].sort_by(|a, b| {
            let cmp = path_cmp(opt_vars().p_fic.get(), a, b, MAXPATHL);
            if cmp < 0 {
                Ordering::Less
            } else if cmp > 0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });
    }
    matches
}

fn find_previous_pathsep(path: &[u8], psep: &mut usize) -> bool {
    let mut cur = (*psep).min(path.len().saturating_sub(1));
    if cur > 0 && vim_ispathsep(i32::from(path[cur])) {
        cur -= 1;
        *psep = cur;
    }

    while cur > 0 {
        if vim_ispathsep(i32::from(path[cur])) {
            return true;
        }
        let off = crate::mbyte::utf_head_off_slice(path, cur - 1) as usize + 1;
        cur = cur.saturating_sub(off);
        *psep = cur;
    }

    false
}

fn is_unique(maybe_unique: &[u8], gap: &[std::ffi::CString], i: usize) -> bool {
    let candidate_len = maybe_unique.len();
    let candidate_cstr = bytes_to_cstring(maybe_unique);

    for (j, other_cstr) in gap.iter().enumerate() {
        if j == i {
            continue;
        }
        let other_bytes = other_cstr.as_bytes();
        if other_bytes.len() < candidate_len {
            continue;
        }
        let offset = other_bytes.len() - candidate_len;
        let rival = &other_cstr.as_c_str()[offset..];
        if path_equal(&candidate_cstr, rival, K_PATH_CMP_LITERAL)
            && (offset == 0 || vim_ispathsep(i32::from(other_bytes[offset - 1])))
        {
            return false;
        }
    }
    true
}

fn expand_path_option(curdir: &CStr, path_option: &CStr, gap: &mut Vec<std::ffi::CString>) {
    let mut buf_arr = [0u8; MAXPATHL];
    let mut curdirlen: usize = 0;
    let mut path_bytes = path_option.to_bytes();
    let curdir_bytes = curdir.to_bytes();

    while !path_bytes.is_empty() {
        let mut buflen = copy_option_part(&mut path_bytes, &mut buf_arr[..MAXPATHL], c" ,");

        if buf_arr[..buflen].contains(&b'`') {
            continue;
        }

        if buf_arr.first() == Some(&b'.')
            && (buflen == 1 || buf_arr.get(1).is_some_and(|&b| vim_ispathsep(i32::from(b))))
        {
            if curbuf_mut().b_ffname.is_none() {
                continue;
            }
            let ffname_cstr = curbuf_mut().ffname_cstr();
            let ffname_bytes = ffname_cstr.to_bytes();
            let p = path_tail(ffname_cstr);
            let plen = ffname_bytes.len() - p.to_bytes().len();
            if plen + buflen >= MAXPATHL {
                continue;
            }
            if buflen == 1 {
                buf_arr[plen] = 0;
            } else {
                buf_arr.copy_within(2..=buflen, plen);
            }
            buf_arr[..plen].copy_from_slice(&ffname_bytes[..plen]);
            buflen = simplify_filename(&mut buf_arr);
        } else if buflen == 0 {
            let c_len = curdir_bytes.len();
            buf_arr[..c_len + 1].copy_from_slice(curdir.to_bytes_with_nul());
            if curdirlen == 0 {
                curdirlen = c_len;
            }
            buflen = curdirlen;
        } else if path_with_url(&buf_arr[..buflen]) {
            continue;
        } else if !path_is_absolute(&buf_arr[..buflen]) {
            if curdirlen == 0 {
                curdirlen = curdir_bytes.len();
            }
            if curdirlen + buflen + 3 > MAXPATHL {
                continue;
            }
            buf_arr.copy_within(0..=buflen, curdirlen + 1);
            buf_arr[..curdirlen].copy_from_slice(&curdir_bytes[..curdirlen]);
            buf_arr[curdirlen] = PATHSEP;
            buflen = simplify_filename(&mut buf_arr);
        }

        gap.push(std::ffi::CString::new(&buf_arr[..buflen]).unwrap_or_default());
    }
}

fn get_path_cutoff(fname_bytes: &[u8], gap: &[std::ffi::CString]) -> Option<usize> {
    let mut maxlen: usize = 0;

    for item in gap {
        let part_bytes = item.as_bytes();
        let mut j: usize = 0;

        while j < fname_bytes.len() && j < part_bytes.len() {
            let fc = fname_bytes[j];
            let pc = part_bytes[j];
            #[cfg(windows)]
            let eq = fc == pc || (vim_ispathsep(fc as i32) && vim_ispathsep(pc as i32));
            #[cfg(not(windows))]
            let eq = fc == pc;
            if !eq {
                break;
            }
            j += 1;
        }
        if j > maxlen {
            maxlen = j;
        }
    }

    if maxlen == 0 {
        return None;
    }

    let mut idx = maxlen.min(fname_bytes.len());
    while idx < fname_bytes.len() && vim_ispathsep(i32::from(fname_bytes[idx])) {
        idx += 1;
    }
    Some(idx)
}

fn uniquefy_paths(gap: &mut Vec<std::ffi::CString>, pattern: &CStr, path_option: &CStr) {
    let mut sort_again = false;
    let mut path_ga: Vec<std::ffi::CString> = Vec::new();

    remove_duplicate_strings(gap);

    let pat_slice = pattern.to_bytes();
    let mut file_pattern = Vec::with_capacity(pat_slice.len() + 1);
    file_pattern.push(b'*');
    file_pattern.extend_from_slice(pat_slice);
    let Some(pat) = file_pat_to_reg_pat(&file_pattern, None, 0) else {
        return;
    };
    drop(file_pattern);

    let mut regmatch = Regmatch {
        rm_ic: true,
        ..Regmatch::new()
    };
    regmatch.set_prog(vim_regcomp(&pat, RE_MAGIC + RE_STRING));
    drop(pat);
    if regmatch.regprog.is_null() {
        return;
    }

    let mut curdir_arr = [0u8; MAXPATHL];
    os_dirname(&mut curdir_arr);
    let curdir_cstr = crate::memory::bytes_as_cstr(&curdir_arr);
    expand_path_option(curdir_cstr, path_option, &mut path_ga);

    let mut in_curdir: Vec<Option<std::ffi::CString>> = vec![None; gap.len()];

    for i in 0..gap.len() {
        if g_st().got_int {
            break;
        }
        let path_cstr = gap[i].as_c_str();
        let path_bytes = path_cstr.to_bytes();
        let dir_end = gettail_dir(path_cstr);

        let dir_len = path_bytes.len() - dir_end.to_bytes().len();
        let is_in_curdir = path_cmp(opt_vars().p_fic.get(), curdir_cstr, path_cstr, dir_len) == 0
            && curdir_arr[dir_len] == 0;
        if is_in_curdir {
            in_curdir[i] = Some(gap[i].clone());
        }

        let cutoff_idx = get_path_cutoff(path_bytes, &path_ga);

        let pat_bytes = pattern.to_bytes();
        if pat_bytes.starts_with(b"**")
            && pat_bytes
                .get(2)
                .is_some_and(|&b| vim_ispathsep_nocolon(i32::from(b)))
            && let Some(c_idx) = cutoff_idx
            && vim_regexec(&mut regmatch, &path_cstr[c_idx..], 0)
            && is_unique(&path_bytes[c_idx..], gap, i)
        {
            sort_again = true;
            gap[i] = bytes_to_cstring(&path_bytes[c_idx..]);
        } else {
            let mut pathsep_idx = path_bytes.len().saturating_sub(1);
            while find_previous_pathsep(path_bytes, &mut pathsep_idx) {
                let cand_idx = pathsep_idx + 1;
                let candidate = &path_cstr[cand_idx..];
                if vim_regexec(&mut regmatch, candidate, 0)
                    && is_unique(&path_bytes[cand_idx..], gap, i)
                    && cutoff_idx.is_some_and(|c_idx| cand_idx >= c_idx)
                {
                    sort_again = true;
                    gap[i] = bytes_to_cstring(&path_bytes[cand_idx..]);
                    break;
                }
            }
        }

        let path_cstr = gap[i].as_c_str();
        if path_is_absolute(path_cstr.to_bytes())
            && let Some(short_name) = path_shorten_fname(Some(path_cstr), curdir_cstr)
            && short_name.to_bytes().len() + 1 < path_cstr.to_bytes().len()
        {
            let short_bytes = short_name.to_bytes();
            let sep_bytes = &PATHSEPSTR[..PATHSEPSTR.len() - 1];
            let mut rel_vec = Vec::with_capacity(1 + sep_bytes.len() + short_bytes.len());
            rel_vec.push(b'.');
            rel_vec.extend_from_slice(sep_bytes);
            rel_vec.extend_from_slice(short_bytes);
            gap[i] = bytes_to_cstring(&rel_vec);
        }
        os_breakcheck();
    }

    for (i, path_opt) in in_curdir.iter().enumerate() {
        if g_st().got_int {
            break;
        }
        let Some(path_cstr) = path_opt else {
            continue;
        };
        let short_name =
            path_shorten_fname(Some(path_cstr.as_c_str()), curdir_cstr).unwrap_or(path_cstr);
        let short_bytes = short_name.to_bytes();
        if is_unique(short_bytes, gap, i) {
            gap[i] = bytes_to_cstring(short_bytes);
            continue;
        }

        let sep_bytes = &PATHSEPSTR[..PATHSEPSTR.len() - 1];
        let mut rel_vec = Vec::with_capacity(1 + sep_bytes.len() + short_bytes.len());
        rel_vec.push(b'.');
        rel_vec.extend_from_slice(sep_bytes);
        rel_vec.extend_from_slice(short_bytes);
        gap[i] = bytes_to_cstring(&rel_vec);
        sort_again = true;
        os_breakcheck();
    }

    regmatch.free_prog();

    if sort_again {
        remove_duplicate_strings(gap);
    }
}

pub fn gettail_dir(fname: &CStr) -> &CStr {
    let bytes = fname.to_bytes();
    let mut dir_end = 0usize;
    let mut next_dir_end = 0usize;
    let mut look_for_sep = true;

    for (i, &b) in bytes.iter().enumerate() {
        if vim_ispathsep(b as i32) {
            if look_for_sep {
                next_dir_end = i;
                look_for_sep = false;
            }
        } else {
            if !look_for_sep {
                dir_end = next_dir_end;
            }
            look_for_sep = true;
        }
    }
    &fname[dir_end..]
}

fn expand_in_path(gap: &mut Vec<std::ffi::CString>, pattern: &CStr, flags: i32) -> i32 {
    let mut path_ga: Vec<std::ffi::CString> = Vec::new();
    let path_option = if curbuf_mut().b_p_path.is_empty() {
        opt_vars().p_path.to_owned()
    } else {
        curbuf_mut().b_p_path.to_owned()
    };

    let mut curdir_arr = [0u8; MAXPATHL];
    os_dirname(&mut curdir_arr);
    let curdir_cstr = crate::memory::bytes_as_cstr(&curdir_arr);

    if (flags & EW_CDPATH) != 0 {
        let p_cdpath = opt_vars().p_cdpath.to_owned();
        expand_path_option(curdir_cstr, &p_cdpath, &mut path_ga);
    } else {
        expand_path_option(curdir_cstr, &path_option, &mut path_ga);
    }
    if path_ga.is_empty() {
        return 0;
    }

    let mut paths_bytes: Vec<u8> = Vec::new();
    for (i, item) in path_ga.iter().enumerate() {
        if i > 0 {
            paths_bytes.push(b',');
        }
        paths_bytes.extend_from_slice(item.as_bytes());
    }
    let paths = std::ffi::CString::new(paths_bytes).unwrap_or_default();

    let mut glob_flags = 0;
    if (flags & EW_ICASE) != 0 {
        glob_flags |= WILD_ICASE;
    }
    if (flags & EW_ADDSLASH) != 0 {
        glob_flags |= WILD_ADD_SLASH;
    }
    globpath(
        paths.as_c_str(),
        pattern,
        gap,
        glob_flags,
        (flags & EW_CDPATH) != 0,
    );

    gap.len() as i32
}

fn has_env_var(bytes: &[u8]) -> bool {
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(unix)]
fn has_special_wildchar(bytes: &[u8], flags: i32) -> bool {
    let special = SPECIAL_WILDCHAR.to_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if matches!(b, b'\r' | b'\n') {
            break;
        }
        if b == b'\\' && i + 1 < bytes.len() && !matches!(bytes[i + 1], b'\r' | b'\n') {
            i += 2;
            continue;
        }
        if special.contains(&b) {
            if b == b'{' && (flags & EW_NOTFOUND) == 0 {
                i += 1;
                continue;
            }
            if b == b'{' && !bytes[i..].contains(&b'}') {
                i += 1;
                continue;
            }
            if matches!(b, b'`' | b'\'') && !bytes[i + 1..].contains(&b) {
                i += 1;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

pub fn gen_expand_wildcards_vec(
    pat: &[&CStr],
    file: &mut Vec<std::ffi::CString>,
    flags: i32,
) -> bool {
    file.clear();
    let mut did_expand_in_path = false;
    let path_option_owned = if curbuf_mut().b_p_path.is_empty() {
        opt_vars().p_path.to_owned()
    } else {
        curbuf_mut().b_p_path.to_owned()
    };
    let path_option = path_option_owned.as_c_str();

    if PATH_STATE.with(|st| st.expand_recursive.get()) {
        #[cfg(unix)]
        {
            return os_expand_wildcards(pat, file, flags);
        }
        #[cfg(not(unix))]
        {
            return false;
        }
    }

    #[cfg(unix)]
    for &pi in pat {
        let pi_bytes = pi.to_bytes();
        if has_special_wildchar(pi_bytes, flags)
            && !(vim_backtick(pi_bytes) && pi_bytes.get(1) == Some(&b'='))
        {
            return os_expand_wildcards(pat, file, flags);
        }
    }

    PATH_STATE.with(|st| st.expand_recursive.set(true));

    for &pat_item in pat {
        if g_st().got_int {
            break;
        }
        let mut add_pat: i32 = -1;
        let mut p_cstr: &CStr = pat_item;
        let mut p_owned: Option<std::ffi::CString> = None;
        let p_bytes = p_cstr.to_bytes();

        if vim_backtick(p_bytes) {
            add_pat = expand_backtick(file, p_bytes, flags);
            if add_pat == -1 {
                PATH_STATE.with(|st| st.expand_recursive.set(false));
                file.clear();
                return false;
            }
        } else {
            if (has_env_var(p_bytes) && (flags & EW_NOTENV) == 0) || p_bytes.first() == Some(&b'~')
            {
                let po = expand_env_save_opt(p_cstr, true, Some(PATH_ESC_WILDCARDS));
                p_owned = Some(po);
                p_cstr = p_owned.as_deref().unwrap();
                #[cfg(unix)]
                {
                    let po_bytes = p_cstr.to_bytes();
                    if has_env_var(po_bytes) || po_bytes.first() == Some(&b'~') {
                        drop(p_owned);
                        file.clear();
                        let res = os_expand_wildcards(pat, file, flags | EW_KEEPDOLLAR);
                        PATH_STATE.with(|st| st.expand_recursive.set(false));
                        return res;
                    }
                }
            }

            let cur_bytes = p_cstr.to_bytes();
            if path_has_wildcard(cur_bytes, false) || (flags & EW_ICASE) != 0 {
                if (flags & (EW_PATH | EW_CDPATH)) != 0
                    && !path_is_absolute(cur_bytes)
                    && !(cur_bytes.first() == Some(&b'.')
                        && (cur_bytes
                            .get(1)
                            .is_some_and(|&b| vim_ispathsep(i32::from(b)))
                            || (cur_bytes.get(1) == Some(&b'.')
                                && cur_bytes
                                    .get(2)
                                    .is_some_and(|&b| vim_ispathsep(i32::from(b))))))
                {
                    PATH_STATE.with(|st| st.expand_recursive.set(false));
                    add_pat = expand_in_path(file, p_cstr, flags);
                    PATH_STATE.with(|st| st.expand_recursive.set(true));
                    did_expand_in_path = true;
                } else {
                    PATH_STATE.with(|st| st.expand_recursive.set(false));
                    let tmp_add_pat = path_expand(file, p_cstr, flags);
                    PATH_STATE.with(|st| st.expand_recursive.set(true));
                    debug_assert!(tmp_add_pat <= i32::MAX as usize);
                    add_pat = tmp_add_pat as i32;
                }
            }
        }

        if add_pat == -1 || (add_pat == 0 && (flags & EW_NOTFOUND) != 0) {
            let t_owned = backslash_halve_save(p_cstr.to_bytes());
            if (flags & EW_NOTFOUND) != 0 {
                addfile(file, &t_owned, flags | EW_DIR | EW_FILE);
            } else {
                addfile(file, &t_owned, flags);
            }
        }

        if did_expand_in_path && !file.is_empty() && (flags & (EW_PATH | EW_CDPATH)) != 0 {
            PATH_STATE.with(|st| st.expand_recursive.set(false));
            uniquefy_paths(file, p_cstr, path_option);
            PATH_STATE.with(|st| st.expand_recursive.set(true));
        }
        drop(p_owned);
    }

    PATH_STATE.with(|st| st.expand_recursive.set(false));

    (flags & EW_EMPTYOK) != 0 || !file.is_empty()
}

pub fn expand_wildcards_vec(pat: &[&CStr], files: &mut Vec<std::ffi::CString>, flags: i32) -> bool {
    let retval = gen_expand_wildcards_vec(pat, files, flags);

    if (flags & EW_KEEPALL) != 0 || !retval {
        return retval;
    }

    if !opt_vars().p_wig.is_empty() {
        files.retain(|fi_cstr| {
            let ffname = FullName_save(Some(fi_cstr.as_c_str()), false).unwrap_or_default();
            !match_file_list(opt_vars().p_wig.as_cstr(), Some(fi_cstr), &ffname)
        });
    }

    if files.len() > 1 && !g_st().got_int {
        let mut non_suf_match = 0;
        for i in 0..files.len() {
            if !match_suffix(&files[i]) {
                files[non_suf_match..=i].rotate_right(1);
                non_suf_match += 1;
            }
        }
    }

    if files.is_empty() {
        return false;
    }

    retval
}

fn vim_backtick(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes.first() == Some(&b'`') && bytes.last() == Some(&b'`')
}

fn expand_backtick(gap: &mut Vec<std::ffi::CString>, pat_bytes: &[u8], flags: i32) -> i32 {
    let mut cnt: i32 = 0;
    let mut cmd_vec: Vec<u8> = pat_bytes[1..pat_bytes.len() - 1].to_vec();
    cmd_vec.push(0);

    let cmd_out: Option<std::ffi::CString> = if cmd_vec[0] == b'=' {
        eval_to_string(crate::memory::bytes_as_cstr(&cmd_vec[1..]), true, false)
    } else {
        get_cmd_output(
            Some(crate::memory::bytes_as_cstr(&cmd_vec)),
            None,
            if (flags & EW_SILENT) != 0 {
                K_SHELL_OPT_SILENT
            } else {
                0
            },
        )
    };
    drop(cmd_vec);
    let Some(cmd_cstr) = cmd_out else {
        return -1;
    };
    let mut cmd_buf = cmd_cstr.into_bytes_with_nul();
    let cmd_len = cmd_buf.len() - 1;
    let mut idx = 0usize;
    while idx < cmd_len {
        while idx < cmd_len && matches!(cmd_buf[idx], b' ' | b'\t') {
            idx += 1;
        }
        let mut end = idx;
        while end < cmd_len && !matches!(cmd_buf[end], b'\r' | b'\n') {
            end += 1;
        }
        if end > idx {
            let saved = cmd_buf[end];
            cmd_buf[end] = NUL as u8;
            addfile(
                gap,
                crate::memory::bytes_as_cstr(&cmd_buf[idx..=end]),
                flags,
            );
            cmd_buf[end] = saved;
            cnt += 1;
        }
        idx = end;
        while idx < cmd_len && matches!(cmd_buf[idx], b'\r' | b'\n') {
            idx += 1;
        }
    }
    cnt
}

#[cfg(windows)]
pub fn slash_adjust(p: &mut [u8]) {
    let p_bytes = crate::memory::bytes_until_nul(p);
    if path_with_url(p_bytes) {
        return;
    }

    if p_bytes.first() == Some(&b'`') && p_bytes.len() > 2 && p_bytes.last() == Some(&b'`') {
        return;
    }

    let from = if opt_vars().p_ssl.get() {
        b'\\'
    } else {
        PATHSEP
    };
    let to = if opt_vars().p_ssl.get() {
        PATHSEP
    } else {
        b'\\'
    };
    let len = p_bytes.len();
    for b in &mut p[..len] {
        if *b == from {
            *b = to;
        }
    }
}

#[cfg(windows)]
pub fn path_to_backslash(p: &mut [u8]) -> &mut [u8] {
    let len = crate::memory::bytes_until_nul(p).len();
    for b in &mut p[..len] {
        if *b == PATHSEP {
            *b = b'\\';
        }
    }
    p
}

#[cfg(windows)]
pub fn path_to_slash(p: &mut [u8]) -> &mut [u8] {
    let p_bytes = crate::memory::bytes_until_nul(p);
    if !p_bytes.is_empty() && !path_with_url(p_bytes) {
        let len = p_bytes.len();
        for b in &mut p[..len] {
            if *b == b'\\' {
                *b = PATHSEP;
            }
        }
    }
    p
}

#[cfg(windows)]
pub fn path_to_slash_save(p: Option<&CStr>) -> Option<std::ffi::CString> {
    let p = p?;
    let mut bytes = p.to_bytes_with_nul().to_vec();
    path_to_slash(&mut bytes);
    Some(std::ffi::CString::from_vec_with_nul(bytes).unwrap())
}

pub fn addfile(gap: &mut Vec<std::ffi::CString>, f: &CStr, flags: i32) {
    let mut file_info: FileInfo = FileInfo::new();

    if (flags & EW_NOTFOUND) == 0
        && if (flags & EW_ALLLINKS) != 0 {
            !os_fileinfo_link(f, &mut file_info)
        } else {
            !os_path_exists(f)
        }
    {
        return;
    }

    #[cfg(windows)]
    {
        let bytes = f.to_bytes();
        if bytes.iter().any(|b| FNAME_ILLEGAL.contains(b)) {
            return;
        }
    }

    let isdir = os_isdir(f);
    if (isdir && (flags & EW_DIR) == 0) || (!isdir && (flags & EW_FILE) == 0) {
        return;
    }

    if !isdir && (flags & EW_EXEC) != 0 && !os_can_exe(f, None, (flags & EW_SHELLCMD) == 0) {
        return;
    }

    let f_bytes = f.to_bytes_with_nul();
    let flen = f_bytes.len() - 1;
    let mut buf = vec![0u8; flen + 1 + usize::from(isdir) + PATHSEPSTR.len()];
    buf[..flen + 1].copy_from_slice(f_bytes);
    to_slash(&mut buf);
    if isdir && (flags & EW_ADDSLASH) != 0 {
        add_pathsep(&mut buf);
    }
    gap.push(crate::memory::cstring_until_nul(&buf));
}

pub fn simplify_filename(filename: &mut [u8]) -> usize {
    let mut components: i32 = 0;
    let mut stripping_disabled = false;
    let mut relative = true;

    let mut p = 0usize;
    let mut p_end = crate::memory::bytes_until_nul(filename).len();
    #[cfg(windows)]
    if p_end >= 2 && filename.get(1) == Some(&b':') {
        p = 2;
    }

    if p < p_end && vim_ispathsep(filename[p]) {
        relative = false;
        let rem = crate::memory::bytes_as_cstr(&filename[p..=p_end]);
        let skipped = path_skip_sep(rem, true);
        p = p_end - skipped.to_bytes().len();
    }
    #[cfg(unix)]
    let mut start = p;
    #[cfg(not(unix))]
    let start = p;

    #[cfg(unix)]
    if start > 2 {
        filename.copy_within(p..=p_end, 1);
        p_end -= p - 1;
        p = 1;
        start = 1;
    }

    loop {
        if p < p_end && vim_ispathsep(filename[p]) {
            filename.copy_within(p + 1..=p_end, p);
            p_end -= 1;
        } else if p < p_end
            && filename[p] == b'.'
            && (p + 1 == p_end || vim_ispathsep(i32::from(filename[p + 1])))
        {
            let has_b1 = p + 1 < p_end;
            if p == start && relative {
                p += 1 + usize::from(has_b1);
            } else {
                let mut tail = p + 1;
                if has_b1 {
                    let rem = crate::memory::bytes_as_cstr(&filename[tail..=p_end]);
                    let skipped = path_skip_sep(rem, true);
                    tail = p_end - skipped.to_bytes().len();
                } else if p > start {
                    p -= 1;
                }
                filename.copy_within(tail..=p_end, p);
                p_end -= tail - p;
            }
        } else if filename[p..p_end].starts_with(b"..")
            && (p + 2 == p_end || vim_ispathsep(i32::from(filename[p + 2])))
        {
            let rem = crate::memory::bytes_as_cstr(&filename[p + 2..=p_end]);
            let skipped = path_skip_sep(rem, true);
            let mut tail = p_end - skipped.to_bytes().len();

            if components > 0 {
                let mut do_strip = false;

                if !stripping_disabled {
                    let saved_char = filename[p - 1];
                    filename[p - 1] = 0;

                    let mut file_info: FileInfo = FileInfo::new();
                    if !os_fileinfo_link(
                        crate::memory::bytes_as_cstr(&filename[..p]),
                        &mut file_info,
                    ) {
                        do_strip = true;
                    }

                    filename[p - 1] = saved_char;

                    p -= 1;
                    while p > start && after_pathsep(&filename[start..p]) == 0 {
                        let off =
                            crate::mbyte::utf_head_off(&filename[start..p], p - start - 1) + 1;
                        p -= off as usize;
                    }

                    if !do_strip {
                        let saved_char = filename[tail];
                        filename[tail] = 0;

                        if os_fileinfo(
                            crate::memory::bytes_as_cstr(&filename[..=tail]),
                            &mut file_info,
                        ) {
                            do_strip = true;
                        } else {
                            stripping_disabled = true;
                        }

                        filename[tail] = saved_char;

                        if do_strip {
                            let mut new_file_info: FileInfo = FileInfo::new();
                            if p == start && relative {
                                os_fileinfo(c".", &mut new_file_info);
                            } else {
                                let saved_char = filename[p];
                                filename[p] = 0;

                                os_fileinfo(
                                    crate::memory::bytes_as_cstr(&filename[..=p]),
                                    &mut new_file_info,
                                );

                                filename[p] = saved_char;
                            }

                            if !os_fileinfo_id_equal(&file_info, &new_file_info) {
                                do_strip = false;
                            }
                        }
                    }
                }

                if !do_strip {
                    p = tail;
                    components = 0;
                } else {
                    if p == start && relative && filename[tail - 1] == b'.' {
                        filename[p] = b'.';
                        p += 1;
                        filename[p] = 0;
                        p_end = p;
                    } else {
                        if p > start && filename[tail - 1] == b'.' {
                            p -= 1;
                        }
                        filename.copy_within(tail..=p_end, p);
                        p_end -= tail - p;
                    }
                    components -= 1;
                }
            } else if p == start && !relative {
                filename.copy_within(tail..=p_end, p);
                p_end -= tail - p;
            } else {
                if p == start + 2 && filename[p - 2] == b'.' {
                    filename.copy_within(p..=p_end, p - 2);
                    p_end -= 2;
                    tail -= 2;
                }
                p = tail;
            }
        } else {
            components += 1;
            let rem = crate::memory::bytes_as_cstr(&filename[p..=p_end]);
            let next_comp = path_next_component(rem);
            p = p_end - next_comp.to_bytes().len();
        }

        if p >= p_end || filename[p] == 0 {
            break;
        }
    }

    p_end
}

pub fn path_has_drive_letter(bytes: &[u8]) -> bool {
    let path_len = bytes.len();
    path_len >= 2
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && matches!(bytes.get(1), Some(b':' | b'|'))
        && (path_len == 2
            || matches!(
                bytes.get(2).copied().unwrap_or(0),
                b'/' | b'\\' | b'?' | b'#'
            ))
}

pub fn path_is_url(bytes: &[u8]) -> bool {
    bytes.starts_with(b":/")
}

pub fn path_with_url(fname: &[u8]) -> bool {
    let bytes = crate::memory::bytes_until_nul(fname);
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }

    if path_has_drive_letter(bytes) {
        return false;
    }

    let mut i: usize = 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'+' | b'-' | b'.'))
    {
        i += 1;
    }

    if matches!(bytes[i - 1], b'+' | b'-' | b'.') {
        return false;
    }

    bytes[i..].starts_with(b":/")
}

pub fn path_with_extension(path: &CStr, extension: &CStr) -> bool {
    let bytes = path.to_bytes();
    let Some(pos) = bytes.iter().rposition(|&b| b == b'.') else {
        return false;
    };
    mb_strcmp_ic(opt_vars().p_fic.get(), &path[pos + 1..], extension) == 0
}

#[allow(non_snake_case)]
pub fn vim_isAbsName(name: &[u8]) -> bool {
    path_with_url(name) || path_is_absolute(name)
}

pub fn vim_full_name(fname: Option<&CStr>, buf: &mut [u8], force: bool) -> bool {
    if buf.is_empty() {
        return false;
    }
    buf[0] = 0;

    let Some(fname) = fname else {
        return false;
    };

    let len = buf.len();
    let fname_bytes = fname.to_bytes();
    if fname_bytes.len() > len.saturating_sub(1) {
        crate::memory::copy_bytes_to_c_buf(buf, fname_bytes);
        to_slash(buf);
        return false;
    }

    if path_with_url(fname_bytes) {
        crate::memory::copy_bytes_to_c_buf(buf, fname_bytes);
        return true;
    }

    let rv = path_to_absolute(fname, buf, i32::from(force));
    if !rv {
        crate::memory::copy_bytes_to_c_buf(buf, fname_bytes);
    }
    to_slash(buf);
    rv
}

pub fn fix_fname(fname: Option<&CStr>) -> Option<std::ffi::CString> {
    #[cfg(unix)]
    {
        FullName_save(fname, true)
    }
    #[cfg(not(unix))]
    {
        let fname = fname?;
        let fname_bytes = fname.to_bytes();
        if !vim_isAbsName(fname_bytes)
            || fname_bytes
                .windows(2)
                .any(|w| matches!(w, b".." | b"//" | b"\\\\"))
            || matches!(fname_bytes.first(), Some(&b'/' | &b'\\'))
        {
            return FullName_save(Some(fname), false);
        }

        let mut bytes = to_slash_save(Some(fname))?.into_bytes_with_nul();
        path_fix_case(&mut bytes);
        Some(crate::memory::cstring_until_nul(&bytes))
    }
}

pub fn path_fix_case(name: &mut [u8]) {
    let mut file_info: FileInfo = FileInfo::new();
    let name_cstr = crate::memory::bytes_as_cstr(name);
    if !os_fileinfo_link(name_cstr, &mut file_info) {
        return;
    }

    let name_bytes = name_cstr.to_bytes();
    let name_len = name_bytes.len();
    let slash_pos = name_bytes.iter().rposition(|&b| b == b'/');
    let mut dir = Directory::new();
    let (ok, tail_off) = if let Some(pos) = slash_pos {
        name[pos] = 0;
        let ok = os_scandir(&mut dir, crate::memory::bytes_as_cstr(&name[..=pos]));
        name[pos] = b'/';
        (ok, pos + 1)
    } else {
        let ok = os_scandir(&mut dir, c".");
        (ok, 0)
    };

    if !ok {
        return;
    }

    let taillen = name_len - tail_off;
    while let Some(entry) = os_scandir_next(&mut dir) {
        let entry_bytes = entry.to_bytes();
        if name[tail_off..name_len].eq_ignore_ascii_case(entry_bytes)
            && taillen == entry_bytes.len()
        {
            let mut newname = [0u8; MAXPATHL + 1];
            crate::memory::copy_bytes_to_c_buf(&mut newname, &name[..name_len]);
            crate::memory::copy_bytes_to_c_buf(&mut newname[tail_off..], entry_bytes);
            let mut file_info_new: FileInfo = FileInfo::new();
            if os_fileinfo_link(crate::memory::bytes_as_cstr(&newname), &mut file_info_new)
                && os_fileinfo_id_equal(&file_info, &file_info_new)
            {
                name[tail_off..tail_off + taillen].copy_from_slice(entry_bytes);
                name[tail_off + taillen] = 0;
                break;
            }
        }
    }

    os_closedir(&mut dir);
}

pub fn after_pathsep(prefix: &[u8]) -> i32 {
    i32::from(
        !prefix.is_empty()
            && vim_ispathsep(i32::from(prefix[prefix.len() - 1]))
            && crate::mbyte::utf_head_off(prefix, prefix.len() - 1) == 0,
    )
}

pub fn same_directory(f1: Option<&CStr>, f2: Option<&CStr>) -> bool {
    let (Some(f1), Some(f2)) = (f1, f2) else {
        return false;
    };

    let mut ffname = [0u8; MAXPATHL];
    vim_full_name(Some(f1), &mut ffname, false);
    let ffname_cstr = crate::memory::bytes_as_cstr(&ffname);
    let t1 = path_tail_with_sep(ffname_cstr);
    let t2 = path_tail_with_sep(f2);
    let d1 = ffname_cstr.to_bytes().len() - t1.to_bytes().len();
    let d2 = f2.to_bytes().len() - t2.to_bytes().len();
    d1 == d2 && path_cmp(opt_vars().p_fic.get(), ffname_cstr, f2, d1) == 0
}

pub fn path_fold_char(ic: bool, p: &[u8], len: &mut i32) -> i32 {
    let b0 = p.first().copied().unwrap_or(0);
    if vim_ispathsep_nocolon(i32::from(b0)) {
        *len = 1;
        return i32::from(PATHSEP);
    }
    *len = if ic {
        crate::mbyte::utfc_slice2len(p) as i32
    } else {
        1
    };
    if ic {
        utf_fold(crate::mbyte::utf_slice2char(p))
    } else {
        i32::from(b0)
    }
}

pub fn path_cmp(ic: bool, p: &CStr, q: &CStr, maxlen: usize) -> i32 {
    let mut p_bytes = p.to_bytes();
    let mut q_bytes = q.to_bytes();
    let mut c1: i32 = NUL as i32;
    let mut c2: i32 = NUL as i32;
    let mut len: usize = 0;

    #[cfg(windows)]
    {
        if p_bytes
            .first()
            .is_some_and(|&b| vim_ispathsep_nocolon(b as i32))
            && q_bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && q_bytes.get(1) == Some(&b':')
        {
            if q_bytes[0].to_ascii_lowercase() as i32 == _getdrive() + (b'a' as i32) - 1 {
                q_bytes = &q_bytes[2..];
            }
        } else if q_bytes
            .first()
            .is_some_and(|&b| vim_ispathsep_nocolon(b as i32))
            && p_bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && p_bytes.get(1) == Some(&b':')
            && p_bytes[0].to_ascii_lowercase() as i32 == _getdrive() + (b'a' as i32) - 1
        {
            p_bytes = &p_bytes[2..];
        }
    }

    let mut i: i32 = 0;
    let mut j: i32 = 0;
    let mut prev_c: i32 = NUL as i32;
    while len < maxlen {
        c1 = path_fold_char(ic, p_bytes, &mut i);
        c2 = path_fold_char(ic, q_bytes, &mut j);
        if c1 == NUL as i32 || c2 == NUL as i32 || c1 != c2 {
            break;
        }
        prev_c = c1;
        p_bytes = &p_bytes[(i as usize).min(p_bytes.len())..];
        q_bytes = &q_bytes[(j as usize).min(q_bytes.len())..];
        len += i as usize;
    }

    if (c1 == NUL as i32 && c2 == NUL as i32) || len >= maxlen {
        return 0;
    }

    if c1 != NUL as i32 && c2 != NUL as i32 {
        if vim_ispathsep(c1) {
            return -1;
        }
        if vim_ispathsep(c2) {
            return 1;
        }
        return c1 - c2;
    }

    let s_bytes = if c1 == NUL as i32 { q_bytes } else { p_bytes };
    if s_bytes.len() == 1
        && vim_ispathsep_nocolon(s_bytes[0] as i32)
        && len > 0
        && !vim_ispathsep(prev_c)
    {
        return 0;
    }
    if c1 == NUL as i32 { -1 } else { 1 }
}

pub fn path_try_shorten_fname(full_path: Option<&CStr>) -> Option<&CStr> {
    let full_path = full_path?;
    let mut dirname = [0u8; MAXPATHL];
    if os_dirname(&mut dirname)
        && let Some(p) = path_shorten_fname(Some(full_path), crate::memory::bytes_as_cstr(&dirname))
        && !p.to_bytes().is_empty()
    {
        return Some(p);
    }
    Some(full_path)
}

pub fn path_shorten_fname<'a>(full_path: Option<&'a CStr>, dir_name: &CStr) -> Option<&'a CStr> {
    let full_path = full_path?;
    let full_bytes = full_path.to_bytes();
    let dir_bytes = dir_name.to_bytes();
    let len = dir_bytes.len();

    let ic = opt_vars().p_fic.get();

    if path_cmp(ic, dir_name, full_path, len) != 0 {
        return None;
    }

    if len == path_head_length() as usize && get_past_head(dir_name).to_bytes().is_empty() {
        return Some(&full_path[len.min(full_bytes.len())..]);
    }

    if full_bytes
        .get(len)
        .is_none_or(|&b| !vim_ispathsep(b as i32))
    {
        return None;
    }

    Some(path_skip_sep(&full_path[len + 1..], false))
}

pub fn expand_wildcards_eval(pat: &CStr, file: &mut Vec<std::ffi::CString>, flags: i32) -> bool {
    let mut ret = false;
    let mut eval_pat: Option<std::ffi::CString> = None;
    let mut exp_pat_owned: Option<std::ffi::CString> = None;
    let mut exp_pat: &CStr = pat;
    let mut ignored_msg = None;
    let mut usedlen: usize = 0;
    let first_byte = exp_pat.to_bytes().first().copied().unwrap_or(0);
    let is_cur_alt_file = first_byte == b'%' || first_byte == b'#';
    let mut star_follows = false;

    file.clear();

    if is_cur_alt_file || first_byte == b'<' {
        g_st().emsg_off += 1;
        let mut pat_buf = exp_pat.to_bytes_with_nul().to_vec();
        eval_pat = eval_vars(
            &mut pat_buf,
            0,
            &mut usedlen,
            None,
            &mut ignored_msg,
            None,
            true,
        );
        g_st().emsg_off -= 1;
        if let Some(ref ep) = eval_pat {
            let exp_bytes = exp_pat.to_bytes();
            let tail_bytes = &exp_bytes[usedlen.min(exp_bytes.len())..];
            star_follows = tail_bytes == b"*";
            let mut joined = Vec::with_capacity(ep.as_bytes().len() + tail_bytes.len());
            joined.extend_from_slice(ep.as_bytes());
            joined.extend_from_slice(tail_bytes);
            let cstr = crate::memory::bytes_to_cstring(&joined);
            exp_pat_owned = Some(cstr);
            exp_pat = exp_pat_owned.as_deref().unwrap();
        }
    }

    if !exp_pat.to_bytes().is_empty() || eval_pat.is_none() {
        ret = expand_wildcards_vec(&[exp_pat], file, flags);
    }

    if let Some(ep) = eval_pat
        && file.is_empty()
        && is_cur_alt_file
        && star_follows
    {
        file.push(ep);
        ret = true;
    }
    drop(exp_pat_owned);

    ret
}

pub fn match_suffix(fname: &CStr) -> bool {
    let mut suf_buf = [0u8; MAXSUFLEN];
    let fname_bytes = fname.to_bytes();
    let fnamelen = fname_bytes.len();
    let mut setsuflen: usize = 0;
    let p_su = opt_vars().p_su.to_owned();
    let mut setsuf = p_su.as_bytes();
    while !setsuf.is_empty() {
        setsuflen = copy_option_part(&mut setsuf, &mut suf_buf[..MAXSUFLEN], c".,");
        if setsuflen == 0 {
            let tail = path_tail(fname);
            if !tail.to_bytes().contains(&b'.') {
                setsuflen = 1;
                break;
            }
        } else {
            if fnamelen >= setsuflen
                && path_cmp(
                    opt_vars().p_fic.get(),
                    crate::memory::bytes_as_cstr(&suf_buf),
                    &fname[fnamelen - setsuflen..],
                    setsuflen,
                ) == 0
            {
                break;
            }
            setsuflen = 0;
        }
    }
    setsuflen != 0
}

pub fn path_full_dir_name(directory: &CStr, buffer: &mut [u8]) -> bool {
    if directory.to_bytes().is_empty() {
        return os_dirname(buffer);
    }

    if os_realpath(directory, buffer).is_some() {
        return true;
    }

    if path_is_absolute(directory.to_bytes()) {
        return false;
    }

    let mut old_dir = [0u8; MAXPATHL];
    if !os_dirname(&mut old_dir) {
        return false;
    }

    crate::memory::copy_bytes_to_c_buf(buffer, crate::memory::bytes_until_nul(&old_dir));
    append_path(buffer, directory)
}

pub fn append_path(path_slice: &mut [u8], to_append: &CStr) -> bool {
    let max_len = path_slice.len();
    let mut current_length = crate::memory::bytes_until_nul(path_slice).len();
    let to_append_bytes = to_append.to_bytes();
    let to_append_length = to_append_bytes.len();

    if to_append_length == 0 || to_append_bytes == b"." {
        return true;
    }

    if current_length > 0 && !vim_ispathsep_nocolon(i32::from(path_slice[current_length - 1])) {
        let pathsep_len = PATHSEPSTR.len() - 1;
        if current_length + pathsep_len + 1 > max_len {
            return false;
        }
        crate::memory::copy_bytes_to_c_buf(&mut path_slice[current_length..], PATHSEPSTR);
        current_length += pathsep_len;
    }

    if current_length + to_append_length + 1 > max_len {
        return false;
    }

    crate::memory::copy_bytes_to_c_buf(&mut path_slice[current_length..], to_append_bytes);
    true
}

fn path_to_absolute(fname: &CStr, buf: &mut [u8], force: i32) -> bool {
    if buf.is_empty() {
        return false;
    }
    buf[0] = 0;

    let fname_bytes = fname.to_bytes();
    #[cfg(windows)]
    let is_abs = path_is_absolute(fname_bytes)
        && !vim_ispathsep_nocolon(fname_bytes.first().copied().unwrap_or(0));
    #[cfg(not(windows))]
    let is_abs = path_is_absolute(fname_bytes);

    if force == 0 && is_abs {
        return append_path(buf, fname);
    }

    let mut tail = path_tail(fname);
    if tail.to_bytes() == b".." {
        tail = &tail[2..];
    }

    let rel_len = fname_bytes.len() - tail.to_bytes().len();
    let dir = bytes_to_cstring(&fname_bytes[..rel_len]);
    if !path_full_dir_name(&dir, buf) {
        return false;
    }
    append_path(buf, tail)
}

pub fn path_is_absolute(fname: &[u8]) -> bool {
    let bytes = crate::memory::bytes_until_nul(fname);
    #[cfg(windows)]
    {
        (bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && bytes.get(1) == Some(&b':')
            && bytes
                .get(2)
                .is_some_and(|&b| vim_ispathsep_nocolon(b as i32)))
            || bytes
                .first()
                .is_some_and(|&b| vim_ispathsep_nocolon(b as i32))
    }
    #[cfg(not(windows))]
    {
        matches!(bytes.first(), Some(b'/' | b'~'))
    }
}

pub fn path_guess_exepath(argv0: &CStr, buf_slice: &mut [u8]) {
    let path_cstr = os_getenv_cstr(c"PATH");
    let argv0_bytes = argv0.to_bytes();

    if path_is_absolute(argv0_bytes) {
        crate::memory::copy_bytes_to_c_buf(buf_slice, argv0_bytes);
    } else if let Some(ref path_cstr) = path_cstr {
        if argv0_bytes.first() == Some(&b'.') || argv0_bytes.contains(&PATHSEP) {
            if !os_dirname(buf_slice) {
                buf_slice[0] = 0;
            }
            crate::memory::append_bytes_to_c_buf(buf_slice, PATHSEPSTR);
            crate::memory::append_bytes_to_c_buf(buf_slice, argv0_bytes);
        } else {
            let path_bytes = path_cstr.as_bytes();
            let mut iter = None;
            loop {
                let mut dir: &[u8] = &[];
                iter = vim_env_iter(ENV_SEPCHAR, path_bytes, iter, &mut dir);
                if dir.is_empty() {
                    break;
                }
                if dir.len() + 1 > MAXPATHL {
                    continue;
                }
                crate::memory::copy_bytes_to_c_buf(g_st().name_buff_bytes_mut(), dir);
                crate::memory::append_bytes_to_c_buf(g_st().name_buff_bytes_mut(), PATHSEPSTR);
                crate::memory::append_bytes_to_c_buf(g_st().name_buff_bytes_mut(), argv0_bytes);
                let nb_cstr =
                    crate::memory::bytes_as_cstr(g_st().borrow_mut().name_buff_bytes_mut());
                if os_can_exe(nb_cstr, None, false) {
                    crate::memory::copy_bytes_to_c_buf(buf_slice, nb_cstr.to_bytes());
                    to_slash(buf_slice);
                    return;
                }
                if iter.is_none() {
                    break;
                }
            }
            crate::memory::copy_bytes_to_c_buf(buf_slice, argv0_bytes);
        }
    } else {
        crate::memory::copy_bytes_to_c_buf(buf_slice, argv0_bytes);
    }
    to_slash(buf_slice);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::vars::{VV_PROGPATH, set_vim_var_string};
    use std::ffi::CString;

    fn init_progpath() {
        if crate::eval::vars::get_vim_var_str(VV_PROGPATH)
            .to_bytes()
            .is_empty()
        {
            set_vim_var_string(VV_PROGPATH, Some(c"nvim"), -1);
        }
    }

    #[test]
    fn path_tail_next_component_and_invocation_tail() {
        assert!(vim_ispathsep(b'/' as i32));
        assert_eq!(path_tail(c"directory/file.txt").to_bytes(), b"file.txt");
        assert_eq!(path_tail(c"directory/").to_bytes(), b"");

        assert_eq!(
            path_tail_with_sep(c"directory///file.txt").to_bytes(),
            b"///file.txt"
        );
        assert_eq!(path_tail_with_sep(c"").to_bytes(), b"");
        assert_eq!(path_tail_with_sep(c"some/directory/").to_bytes(), b"/");
        assert_eq!(path_tail_with_sep(c"/file.txt").to_bytes(), b"file.txt");
        assert_eq!(path_tail_with_sep(c"/").to_bytes(), b"");

        assert_eq!(
            path_next_component(c"some/directory/file.txt").to_bytes(),
            b"directory/file.txt"
        );
        assert_eq!(path_next_component(c"file.txt").to_bytes(), b"");

        let mut len = 0usize;
        let inv = invocation_path_tail(c"directory/exe a b c", Some(&mut len));
        assert_eq!(len, 3);
        assert_eq!(inv.to_bytes(), b"exe a b c");
        assert_eq!(
            invocation_path_tail(c"directory/exe a b c", None).to_bytes(),
            b"exe a b c"
        );

        #[cfg(windows)]
        let abs_inv = c"C:\\Users\\anyone\\bin\\z a b";
        #[cfg(not(windows))]
        let abs_inv = c"/usr/bin/z a b";
        let inv2 = invocation_path_tail(abs_inv, Some(&mut len));
        assert_eq!(len, 1);
        assert_eq!(inv2.to_bytes(), b"z a b");

        let inv3 = invocation_path_tail(c"exe a/b\\c", Some(&mut len));
        assert_eq!(len, 3);
        assert_eq!(inv3.to_bytes(), b"exe a/b\\c");

        let mut str_vec: Vec<std::ffi::CString> = Vec::new();
        let empty_concat = concat_cstrings(&str_vec, c",");
        assert_eq!(empty_concat.to_str().unwrap(), "");

        for w in [c"ccc", c"aaa", c"bbb", c"aaa", c"ccc"] {
            str_vec.push(w.to_owned());
        }
        remove_duplicate_strings(&mut str_vec);
        assert_eq!(str_vec.len(), 3);
        let joined = concat_cstrings(&str_vec, c",");
        assert_eq!(joined.to_str().unwrap(), "aaa,bbb,ccc");
    }

    #[test]
    fn path_cmp_shorten_and_append_path() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            assert_eq!(path_cmp(false, c"foo/bar", c"foo/bar", 7), 0);
            assert_eq!(path_cmp(false, c"foobar", c"foo/bar", 7), 1);
            assert_eq!(path_cmp(false, c"foo/bar", c"foobar", 7), -1);
            assert_ne!(path_cmp(false, c"foo/bar", c"foo/baz", 7), 0);
            assert_eq!(path_cmp(false, c"foo/bar", c"foo/baz", 6), 0);
            assert_eq!(path_cmp(false, c"foo", c"foo/", 4), 0);
            assert_ne!(path_cmp(false, c"/", c"//", 2), 0);
            assert_ne!(path_cmp(false, c"foo", c"foo//", 5), 0);
            assert_ne!(path_cmp(false, c"foo/", c"foo//", 5), 0);

            let dir = c"some/very/long";
            assert!(path_shorten_fname(None, dir).is_none());
            let other = c"as/this.txt";
            assert!(path_shorten_fname(Some(other), dir).is_none());
            let trailing_dir = c"some/very/long/";
            let full = c"some/very/long/directory/file.txt";
            assert!(path_shorten_fname(Some(full), trailing_dir).is_none());
            let shortened = path_shorten_fname(Some(full), dir).unwrap();
            assert_eq!(shortened.to_bytes(), b"directory/file.txt");

            let full_dup = c"some/very/long///directory/file.txt";
            let shortened_dup = path_shorten_fname(Some(full_dup), dir).unwrap();
            assert_eq!(shortened_dup.to_bytes(), b"directory/file.txt");
            #[cfg(windows)]
            {
                let drive_nonroot = c"c:x";
                let drive_rel = c"c:xyz\\foo";
                assert!(path_shorten_fname(Some(drive_rel), drive_nonroot).is_none());
                let drive_root = c"c:\\";
                let drive_abs = c"c:\\xyz\\foo";
                let shortened_root = path_shorten_fname(Some(drive_abs), drive_root).unwrap();
                assert_eq!(shortened_root.to_bytes(), b"xyz\\foo");
            }

            assert!(path_try_shorten_fname(None).is_none());
            let mut cwd_buf = [0u8; MAXPATHL];
            assert!(os_dirname(&mut cwd_buf));
            let cwd_str = crate::memory::bytes_as_cstr(&cwd_buf).to_str().unwrap();
            let in_cwd = CString::new(format!("{cwd_str}/subdir/file.txt")).unwrap();
            assert_eq!(
                path_try_shorten_fname(Some(&in_cwd)).unwrap().to_bytes(),
                b"subdir/file.txt"
            );

            let mut buf = [0u8; 100];
            crate::memory::copy_bytes_to_c_buf(&mut buf, b"path1");
            assert!(append_path(&mut buf, c"path2"));
            assert_eq!(crate::memory::bytes_until_nul(&buf), b"path1/path2");

            crate::memory::copy_bytes_to_c_buf(&mut buf, b"path1/");
            assert!(append_path(&mut buf, c"path2"));
            assert_eq!(crate::memory::bytes_until_nul(&buf), b"path1/path2");
            assert!(!append_path(&mut buf[..11], c"path2"));

            crate::memory::copy_bytes_to_c_buf(&mut buf, b"path1");
            assert!(append_path(&mut buf[..6], c""));
            assert_eq!(crate::memory::bytes_until_nul(&buf), b"path1");
            assert!(append_path(&mut buf[..6], c"."));
            assert_eq!(crate::memory::bytes_until_nul(&buf), b"path1");

            buf[0] = 0;
            assert!(append_path(&mut buf[..7], c"/path2"));
            assert_eq!(crate::memory::bytes_until_nul(&buf), b"/path2");
        }
    }

    #[test]
    fn path_url_extension_and_is_absolute() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(path_is_absolute(b"/some/directory/"));
        #[cfg(windows)]
        assert!(path_is_absolute(b"C:/some/directory/"));
        #[cfg(not(windows))]
        assert!(path_is_absolute(b"~/in/my/home~/directory"));
        assert!(!path_is_absolute(b"not/in/my/home~/directory"));

        let saved_fic = opt_vars().p_fic.get();
        opt_vars().p_fic.set(false);
        assert!(path_with_extension(c"/some/path/file.lua", c"lua"));
        assert!(!path_with_extension(c"/some/path/file.vim", c"lua"));
        assert!(!path_with_extension(c"/some/path/file", c"lua"));
        assert!(!path_with_extension(c"/some/path/file.LUA", c"lua"));
        opt_vars().p_fic.set(true);
        assert!(path_with_extension(c"/some/path/file.LUA", c"lua"));
        opt_vars().p_fic.set(saved_fic);

        assert!(path_with_url(b"test://xyz/foo/b0"));
        assert!(!path_with_url(b"test:\\\\xyz\\foo\\b0"));
        assert!(path_with_url(b"test123://xyz/foo/b0"));
        assert!(!path_with_url(b"test123:\\\\xyz\\foo\\b0"));
        assert!(!path_with_url(b"test_abc://xyz/foo/b2"));
        assert!(path_with_url(b"test+abc-123.ghi://xyz/foo/b1"));
        assert!(!path_with_url(b"test+abc-123.ghi:\\\\xyz\\foo\\b1"));
        assert!(!path_with_url(b"-test://xyz/foo/b4"));
        assert!(!path_with_url(b"test-://xyz/foo/b5"));
        assert!(path_with_url(b"test-C:/xyz/foo/b5"));
        assert!(!path_with_url(b"c:/xyz/foo/b5"));
        assert!(!path_with_url(b"C:/xyz/foo/b5"));
        assert!(!path_with_url(b"C:\\xyz\\foo\\b5"));
    }

    #[test]
    fn path_equal_fullname_full_dir_name_and_guess_exepath() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            init_progpath();

            let mut cwd_buf = [0u8; MAXPATHL];
            assert!(os_dirname(&mut cwd_buf));
            let cwd = crate::memory::bytes_as_cstr(&cwd_buf)
                .to_str()
                .unwrap()
                .to_owned();

            // Create a temporary directory inside target/ (within cwd)
            let rel_dir = format!("target/ut_path_{}", std::process::id());
            let abs_dir = format!("{cwd}/{rel_dir}");
            let _ = std::fs::remove_dir_all(&abs_dir);
            std::fs::create_dir_all(format!("{abs_dir}/CamelCase")).unwrap();
            let f1_rel = format!("{rel_dir}/f1.o");
            let f2_rel = format!("{rel_dir}/f2.o");
            std::fs::write(format!("{abs_dir}/f1.o"), b"").unwrap();
            std::fs::write(format!("{abs_dir}/f2.o"), b"").unwrap();

            let f1_c = CString::new(f1_rel.clone()).unwrap();
            let f2_c = CString::new(f2_rel.clone()).unwrap();
            let f1_abs_c = CString::new(format!("{abs_dir}/f1.o")).unwrap();

            assert!(path_equal(&f1_c, &f1_c, K_PATH_CMP_EXPAND));
            assert!(path_equal(&f1_c, &f1_abs_c, K_PATH_CMP_EXPAND));
            assert!(!path_equal(&f1_c, &f2_c, K_PATH_CMP_EXPAND));
            assert!(!path_equal(
                &f1_c,
                c"nonexistent_ut_file.txt",
                K_PATH_CMP_EXPAND
            ));
            assert!(!path_equal(
                c"nonexistent_ut_file.txt",
                c"nonexistent_ut_file.txt",
                K_PATH_CMP_EXPAND
            ));
            assert!(path_equal(
                c"nonexistent_ut_file.txt",
                c"nonexistent_ut_file.txt",
                K_PATH_CMP_EXPAND | K_PATH_CMP_FULL
            ));

            let saved_fic = opt_vars().p_fic.get();
            opt_vars().p_fic.set(false);
            assert!(!path_equal(c"Foo", c"foo", K_PATH_CMP_LITERAL));
            opt_vars().p_fic.set(true);
            assert!(path_equal(c"Foo", c"foo", K_PATH_CMP_LITERAL));
            opt_vars().p_fic.set(saved_fic);

            // path_full_dir_name
            let mut out = [0u8; MAXPATHL];
            assert!(path_full_dir_name(c"", &mut out));
            assert_eq!(crate::memory::bytes_as_cstr(&out).to_str().unwrap(), cwd);

            let rel_c = CString::new(rel_dir.clone()).unwrap();
            assert!(path_full_dir_name(&rel_c, &mut out));
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                abs_dir
            );

            let non_exist_abs = CString::new(format!("{abs_dir}/does_not_exist")).unwrap();
            assert!(!path_full_dir_name(&non_exist_abs, &mut out));

            // vim_FullName
            assert!(!vim_full_name(None, &mut out[..10], true));
            assert!(vim_full_name(
                Some(c"http://www.neovim.org"),
                &mut out,
                true
            ));
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                "http://www.neovim.org"
            );

            let rel_dot = CString::new(format!("./{f1_rel}")).unwrap();
            assert!(vim_full_name(Some(&rel_dot), &mut out, true));
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                format!("{abs_dir}/f1.o")
            );

            let dir_dotdot = CString::new(format!("{rel_dir}/..")).unwrap();
            assert!(vim_full_name(Some(&dir_dotdot), &mut out, true));
            let expected_parent = std::path::Path::new(&abs_dir)
                .parent()
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                expected_parent
            );

            let dir_slash = CString::new(format!("{rel_dir}/")).unwrap();
            assert!(vim_full_name(Some(&dir_slash), &mut out, true));
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                abs_dir
            );

            // path_fix_case
            let mut case_buf = CString::new(format!("{rel_dir}/camelcase"))
                .unwrap()
                .into_bytes_with_nul();
            path_fix_case(&mut case_buf);
            let expected_case = if std::path::Path::new(&format!("{abs_dir}/camelcase")).exists() {
                format!("{rel_dir}/CamelCase")
            } else {
                format!("{rel_dir}/camelcase")
            };
            assert_eq!(
                crate::memory::bytes_as_cstr(&case_buf).to_str().unwrap(),
                expected_case
            );

            // path_guess_exepath
            path_guess_exepath(c"./nvim", &mut out);
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                format!("{cwd}/./nvim")
            );

            path_guess_exepath(c"23u0293_not_in_path", &mut out);
            assert_eq!(
                crate::memory::bytes_as_cstr(&out).to_str().unwrap(),
                "23u0293_not_in_path"
            );

            let _ = std::fs::remove_dir_all(&abs_dir);
        }
    }
}
