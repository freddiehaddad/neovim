//! Path manipulation, comparison, canonicalization, and wildcard expansion (`src/nvim/path.c`).

use crate::api::private::validate::NvimString;
use crate::charset::{NUL, backslash_halve, backslash_halve_save, rem_backslash, skipwhite};
use crate::cmdexpand::{WILD_ADD_SLASH, WILD_ICASE, globpath};
use crate::diff::kShellOptSilent;
use crate::eval::eval_to_string;
use crate::ex_docmd::eval_vars;
use crate::fileio::{file_pat_to_reg_pat, match_file_list};
use crate::globals::*;
#[cfg(unix)]
use crate::mbyte::mb_isalpha;
use crate::mbyte::{mb_strcmp_ic, utf_fold, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memory::{cstr_from_bytes, cstr_to_cstring, xstrlcpy};
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
use crate::regexp::{
    RE_MAGIC, RE_NOBREAK, RE_STRING, regmatch_T, vim_regcomp, vim_regexec, vim_regfree,
};
use crate::strings::{sort_strings, vim_snprintf, vim_strchr};
use std::cmp::Ordering;
#[cfg(test)]
use std::ffi::CStr;
use std::ffi::{c_char, c_int};
use std::ptr;

pub fn remove_duplicate_strings(fnames: &mut Vec<std::ffi::CString>) {
    if fnames.is_empty() {
        return;
    }
    sort_strings(fnames);
        let mut i = fnames.len() - 1;
        while i > 0 {
            if path_equal(fnames[i - 1].as_ptr(), fnames[i].as_ptr(), kPathCmpLiteral) {
                fnames.remove(i);
            }
            i -= 1;
        }
}

pub fn concat_cstrings(
    strings: &[std::ffi::CString],
    sep: &std::ffi::CStr,
) -> std::ffi::CString {
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

pub type PathCmpFlags = c_int;

const MAXSUFLEN: usize = 30;

pub(crate) const PATHSEP: c_char = b'/' as c_char;
pub(crate) const PATHSEPSTR: &[u8] = b"/\0";

pub(crate) const ENV_SEPCHAR: c_char = if cfg!(windows) {
    b';' as c_char
} else {
    b':' as c_char
};

#[cfg(windows)]
const PATH_ESC_WILDCARDS: *const c_char = c"*?[".as_ptr();
#[cfg(not(windows))]
const PATH_ESC_WILDCARDS: *const c_char = c"*?[{".as_ptr();

#[cfg(windows)]
const PATH_ALL_WILDCARDS: *const c_char = c"*?[`$".as_ptr();
#[cfg(not(windows))]
const PATH_ALL_WILDCARDS: *const c_char = c"*?[{`'$".as_ptr();

#[cfg(unix)]
const SPECIAL_WILDCHAR: *const c_char = c"`'{".as_ptr();

#[cfg(windows)]
const FNAME_ILLEGAL: &[u8] = b"\"*?><|";

const URL_SLASH: c_int = 1;
const URL_BACKSLASH: c_int = 2;

pub const EW_DIR: c_int = 0x01;
pub const EW_FILE: c_int = 0x02;
pub const EW_NOTFOUND: c_int = 0x04;
pub const EW_ADDSLASH: c_int = 0x08;
pub const EW_KEEPALL: c_int = 0x10;
pub const EW_SILENT: c_int = 0x20;
pub const EW_EXEC: c_int = 0x40;
pub const EW_PATH: c_int = 0x80;
pub const EW_ICASE: c_int = 0x100;
pub const EW_NOERROR: c_int = 0x200;
pub const EW_NOTWILD: c_int = 0x400;
#[cfg(unix)]
pub const EW_KEEPDOLLAR: c_int = 0x800;
pub const EW_ALLLINKS: c_int = 0x1000;
pub const EW_SHELLCMD: c_int = 0x2000;
pub const EW_DODOT: c_int = 0x4000;
pub const EW_EMPTYOK: c_int = 0x8000;
pub const EW_NOTENV: c_int = 0x10000;
pub const EW_CDPATH: c_int = 0x20000;
pub const EW_NOBREAK: c_int = 0x40000;

pub const kPathCmpLiteral: PathCmpFlags = 1 << 0;
pub const kPathCmpExpand: PathCmpFlags = 1 << 1;
pub const kPathCmpFull: PathCmpFlags = 1 << 2;

struct PathModuleState {
    SCANDIR_COUNT: c_int,
    STARDEPTH: c_int,
    EXPAND_RECURSIVE: bool,
}

impl PathModuleState {
    const fn new() -> Self {
        Self {
            SCANDIR_COUNT: 0,
            STARDEPTH: 0,
            EXPAND_RECURSIVE: false,
        }
    }
}

static PATH_STATE: crate::globals::GlobalState<PathModuleState> = crate::globals::GlobalState::new(PathModuleState::new());

#[inline]
fn path_st() -> &'static mut PathModuleState {
    unsafe { &mut *PATH_STATE.as_ptr() }
}

#[cfg(windows)]
use crate::fileio::_getdrive;

#[inline]
pub fn to_slash(p: *mut c_char) {
    #[cfg(windows)]
    {
        path_to_slash(p);
    }
    #[cfg(not(windows))]
    let _ = p;
}

#[inline]
pub fn to_slash_save(p: *const c_char) -> Option<std::ffi::CString> {
    #[cfg(windows)]
    {
        path_to_slash_save(p)
    }
    #[cfg(not(windows))]
    {
        if p.is_null() {
            None
        } else {
            Some(cstr_to_cstring(p))
        }
    }
}

#[cfg(windows)]
#[inline]
fn str_contains(haystack: *const c_char, needle: &[u8]) -> bool {
     {
        let bytes = crate::memory::cstr_as_bytes(haystack);
        bytes.windows(needle.len()).any(|w| w == needle)
    }
}

pub fn path_equal(s1: *const c_char, s2: *const c_char, flags: PathCmpFlags) -> bool {
     {
        let mut expanded_s1 = [0 as c_char; MAXPATHL];
        let mut full1 = [0 as c_char; MAXPATHL];
        let mut full2 = [0 as c_char; MAXPATHL];
        let mut file_id1 = FileID {
            inode: 0,
            device_id: 0,
        };
        let mut file_id2 = FileID {
            inode: 0,
            device_id: 0,
        };

        debug_assert!((flags & kPathCmpLiteral) == 0 || flags == kPathCmpLiteral);

        if flags == kPathCmpLiteral {
            let is_url = path_with_url(s1) != 0 || path_with_url(s2) != 0;
            return (!is_url
                || crate::memory::cstr_len(s1) == crate::memory::cstr_len(s2))
                && path_cmp(opt_vars().p_fic != 0, s1, s2, MAXPATHL) == 0;
        }

        if (flags & kPathCmpExpand) != 0 {
            expand_env_esc(
                s1,
                expanded_s1.as_mut_ptr(),
                MAXPATHL as c_int,
                ptr::null_mut(),
                false,
                ptr::null_mut(),
            );
        } else if flags != 0 {
            xstrlcpy(expanded_s1.as_mut_ptr(), s1, MAXPATHL);
        }

        let id_ok1 = os_fileid(expanded_s1.as_ptr(), &mut file_id1);
        let id_ok2 = os_fileid(s2, &mut file_id2);
        if id_ok1 && id_ok2 && os_fileid_equal(&file_id1, &file_id2) {
            return true;
        }
        if !id_ok1 && !id_ok2 && (flags & kPathCmpFull) != 0 {
            vim_FullName(expanded_s1.as_ptr(), full1.as_mut_ptr(), MAXPATHL, false);
            vim_FullName(s2, full2.as_mut_ptr(), MAXPATHL, false);
            return path_cmp(opt_vars().p_fic != 0, full1.as_ptr(), full2.as_ptr(), MAXPATHL) == 0;
        }
        false
    }
}

pub fn path_tail(fname: *const c_char) -> *mut c_char {
    if fname.is_null() {
        return c"".as_ptr().cast_mut();
    }

    unsafe {
        let mut tail: *const c_char = get_past_head(fname);
        let mut p = tail;
        while *p != NUL {
            if vim_ispathsep_nocolon(*p as c_int) {
                tail = p.add(1);
            }
            p = p.add(1);
        }
        tail.cast_mut()
    }
}

pub fn path_tail_with_sep(fname: *mut c_char) -> *mut c_char {
    unsafe {
        let past_head = get_past_head(fname);
        let mut tail = path_tail(fname);
        while tail > past_head && after_pathsep(fname, tail) != 0 {
            tail = tail.sub(1);
        }
        tail
    }
}

pub fn invocation_path_tail(invocation: *const c_char, len: Option<&mut usize>) -> *const c_char {
    unsafe {
        let mut tail: *const c_char = get_past_head(invocation);
        let mut tail_end = tail;
        let mut p = tail;
        let mut inquote = false;
        while *p != NUL && (inquote || *p != b' ' as c_char) {
            let l = utfc_ptr2len(p) as usize;
            if vim_ispathsep_nocolon(*p as c_int) {
                tail = p.add(1);
                tail_end = tail;
            } else if *p == b'\\' as c_char && inquote {
                p = p.add(1);
            } else if *p == b'"' as c_char {
                inquote = !inquote;
            } else {
                tail_end = p.add(l);
            }
            p = p.add(l);
        }

        if let Some(len) = len {
            *len = tail_end.offset_from(tail) as usize;
        }

        tail
    }
}

pub fn path_next_component(mut fname: *const c_char) -> *const c_char {
    unsafe {
        while *fname != NUL && !vim_ispathsep(*fname as c_int) {
            fname = fname.add(1);
        }
        if *fname != NUL {
            fname = fname.add(1);
        }
        fname
    }
}

pub fn path_skip_sep(mut path: *const c_char, colon: bool) -> *mut c_char {
    unsafe {
        while if colon {
            vim_ispathsep(*path as c_int)
        } else {
            vim_ispathsep_nocolon(*path as c_int)
        } {
            path = path.add(1);
        }
        path.cast_mut()
    }
}

pub fn path_head_length() -> c_int {
    #[cfg(windows)]
    {
        3
    }
    #[cfg(not(windows))]
    {
        1
    }
}

pub fn is_path_head(path: *const c_char) -> bool {
    #[cfg(windows)]
    unsafe {
        (*path as u8).is_ascii_alphabetic() && *path.add(1) == b':' as c_char
    }
    #[cfg(not(windows))]
    unsafe {
        vim_ispathsep(*path as c_int)
    }
}

pub fn get_past_head(path: *const c_char) -> *mut c_char {
    #[cfg(windows)]
    let mut retval = path;
    #[cfg(not(windows))]
    let retval = path;
    #[cfg(windows)]
    if is_path_head(path) {
        retval = unsafe { path.add(2) };
    }
    path_skip_sep(retval, true)
}

pub fn vim_ispathsep(c: c_int) -> bool {
    #[cfg(unix)]
    {
        c == b'/' as c_int
    }
    #[cfg(windows)]
    {
        c == b':' as c_int || c == b'/' as c_int || c == b'\\' as c_int
    }
}

pub fn vim_ispathsep_nocolon(c: c_int) -> bool {
    #[cfg(windows)]
    {
        vim_ispathsep(c) && c != b':' as c_int
    }
    #[cfg(not(windows))]
    {
        vim_ispathsep(c)
    }
}

pub fn vim_ispathlistsep(c: c_int) -> bool {
    #[cfg(unix)]
    {
        c == b':' as c_int
    }
    #[cfg(not(unix))]
    {
        c == b';' as c_int
    }
}

pub fn shorten_dir_len(str_: *mut c_char, trim_len: c_int) {
    unsafe {
        let tail = path_tail(str_);
        let mut d = str_;
        let mut skip = false;
        let mut dirchunk_len: c_int = 0;
        let mut s = str_;
        loop {
            if s >= tail {
                *d = *s;
                d = d.add(1);
                if *s == NUL {
                    break;
                }
            } else if vim_ispathsep(*s as c_int) {
                *d = *s;
                d = d.add(1);
                skip = false;
                dirchunk_len = 0;
            } else if !skip {
                *d = *s;
                d = d.add(1);
                if *s != b'~' as c_char && *s != b'.' as c_char {
                    dirchunk_len += 1;
                    if dirchunk_len >= trim_len {
                        skip = true;
                    }
                }
                let mut l = utfc_ptr2len(s);
                while {
                    l -= 1;
                    l > 0
                } {
                    s = s.add(1);
                    *d = *s;
                    d = d.add(1);
                }
            }
            s = s.add(1);
        }
    }
}

pub fn shorten_dir(str_: *mut c_char) {
    shorten_dir_len(str_, 1);
}

pub fn dir_of_file_exists(fname: *mut c_char) -> bool {
    unsafe {
        let p = path_tail_with_sep(fname);
        if p == fname {
            return true;
        }
        let c = *p;
        *p = NUL;
        let retval = os_isdir(fname);
        *p = c;
        retval
    }
}

#[inline]
fn do_concat_fnames(
    fname1: *mut c_char,
    len1: usize,
    fname2: *const c_char,
    len2: usize,
    sep: bool,
    len: &mut usize,
) -> *mut c_char {
    unsafe {
        if sep && *fname1 != NUL && after_pathsep(fname1, fname1.add(len1)) == 0 {
            *fname1.add(len1) = PATHSEP;
            ptr::copy(fname2, fname1.add(len1 + 1), len2 + 1);
            *len = len1 + 1 + len2;
        } else {
            ptr::copy(fname2, fname1.add(len1), len2 + 1);
            *len = len1 + len2;
        }
        fname1
    }
}

pub fn concat_fnames(
    fname1: NvimString,
    fname2: NvimString,
    sep: bool,
) -> std::ffi::CString {
    unsafe {
        let mut buf = vec![0 as c_char; fname1.size + usize::from(sep) + fname2.size + 1];
        if fname1.size > 0 {
            ptr::copy(fname1.data, buf.as_mut_ptr(), fname1.size);
        }
        *buf.as_mut_ptr().add(fname1.size) = NUL;
        let mut size: usize = 0;
        do_concat_fnames(
            buf.as_mut_ptr(),
            fname1.size,
            if fname2.data.is_null() {
                c"".as_ptr()
            } else {
                fname2.data
            },
            fname2.size,
            sep,
            &mut size,
        );
        cstr_from_bytes(std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), size))
    }
}

pub fn concat_fnames_cstr(
    fname1: *const c_char,
    fname2: *const c_char,
    sep: bool,
) -> std::ffi::CString {
    unsafe {
        let len1 = if fname1.is_null() {
            0
        } else {
            crate::memory::cstr_len(fname1)
        };
        let len2 = if fname2.is_null() {
            0
        } else {
            crate::memory::cstr_len(fname2)
        };
        let mut buf = vec![0 as c_char; len1 + len2 + 3];
        if len1 > 0 {
            ptr::copy_nonoverlapping(fname1, buf.as_mut_ptr(), len1);
        }
        let mut len: usize = 0;
        do_concat_fnames(
            buf.as_mut_ptr(),
            len1,
            if fname2.is_null() {
                c"".as_ptr()
            } else {
                fname2
            },
            len2,
            sep,
            &mut len,
        );
        crate::memory::cstr_to_cstring(buf.as_ptr())
    }
}

pub fn add_pathsep(p: *mut c_char) -> bool {
    unsafe {
        if *p == NUL {
            return true;
        }

        let plen = crate::memory::cstr_len(p);
        if after_pathsep(p, p.add(plen)) == 0 {
            let pathsep_len = PATHSEPSTR.len();
            if plen > MAXPATHL - pathsep_len {
                return false;
            }
            ptr::copy_nonoverlapping(
                PATHSEPSTR.as_ptr().cast::<c_char>(),
                p.add(plen),
                pathsep_len,
            );
        }
        true
    }
}

pub fn FullName_save(fname: *const c_char, force: bool) -> Option<std::ffi::CString> {
    if fname.is_null() {
        return None;
    }

    let mut buf: Vec<c_char> = vec![0; MAXPATHL];
        if !vim_FullName(fname, buf.as_mut_ptr(), MAXPATHL, force) {
            return to_slash_save(fname);
        }
        Some(cstr_to_cstring(buf.as_ptr()))
}

pub fn save_abs_path(name: *const c_char) -> Option<std::ffi::CString> {
    if !path_is_absolute(name) {
            return FullName_save(name, true);
        }
        to_slash_save(name)
}

pub fn path_has_wildcard(mut p: *const c_char, all: bool) -> bool {
    unsafe {
        let wildcards = if all {
            PATH_ALL_WILDCARDS
        } else {
            PATH_ESC_WILDCARDS
        };
        while *p != NUL {
            #[cfg(unix)]
            if *p == b'\\' as c_char && *p.add(1) != NUL {
                p = p.add(2);
                continue;
            }
            if !vim_strchr(wildcards, *p as u8 as c_int).is_null()
                || (all && *p == b'~' as c_char && *p.add(1) != NUL)
            {
                return true;
            }
            p = p.add(1);
        }
        false
    }
}

fn scandir_next_with_dots(dir: Option<&mut Directory>) -> *const c_char {
    let Some(dir) = dir else {
        path_st().SCANDIR_COUNT = 0;
        return ptr::null();
    };

    path_st().SCANDIR_COUNT += 1;
    if path_st().SCANDIR_COUNT == 1 || path_st().SCANDIR_COUNT == 2 {
        return if path_st().SCANDIR_COUNT == 1 {
            c".".as_ptr()
        } else {
            c"..".as_ptr()
        };
    }
    os_scandir_next(dir)
}

fn path_expand(
    gap: &mut Vec<std::ffi::CString>,
    path: *const c_char,
    flags: c_int,
) -> usize {
    do_path_expand(gap, path, 0, flags, false)
}

fn do_path_expand(
    gap: &mut Vec<std::ffi::CString>,
    path: *const c_char,
    wildoff: usize,
    flags: c_int,
    didstar: bool,
) -> usize {
    unsafe {
        let start_len = gap.len();
        let mut starstar = false;

        if path_st().STARDEPTH > 0 && (flags & EW_NOBREAK) == 0 {
            os_breakcheck();
            if g_st().got_int {
                return 0;
            }
        }

        let buflen = crate::memory::cstr_len(path) + MAXPATHL;
        let mut buf_vec = vec![0 as c_char; buflen];
        let buf = buf_vec.as_mut_ptr();

        let mut p = buf;
        let mut s = buf;
        let mut e: *mut c_char = ptr::null_mut();
        let mut path_end = path;
        let wild_start = path.add(wildoff);

        while *path_end != NUL {
            if path_end >= wild_start && rem_backslash(path_end) {
                *p = *path_end;
                p = p.add(1);
                path_end = path_end.add(1);
            } else if vim_ispathsep_nocolon(*path_end as c_int) {
                if !e.is_null() {
                    break;
                }
                s = p.add(1);
            } else if path_end >= wild_start && {
                let is_esc_wild =
                    !vim_strchr(PATH_ESC_WILDCARDS, *path_end as u8 as c_int).is_null();
                #[cfg(not(windows))]
                let is_icase_alpha =
                    opt_vars().p_fic == 0 && (flags & EW_ICASE) != 0 && mb_isalpha(utf_ptr2char(path_end));
                #[cfg(windows)]
                let is_icase_alpha = false;
                is_esc_wild || is_icase_alpha
            } {
                e = p;
            }
            let charlen = utfc_ptr2len(path_end) as usize;
            ptr::copy_nonoverlapping(path_end, p, charlen);
            p = p.add(charlen);
            path_end = path_end.add(charlen);
        }
        e = p;
        *e = NUL;

        p = buf.add(wildoff);
        while p < s {
            if rem_backslash(p) {
                let move_len = crate::memory::cstr_len(p.add(1)) + 1;
                ptr::copy(p.add(1), p, move_len);
                e = e.sub(1);
                s = s.sub(1);
            }
            p = p.add(1);
        }

        p = s;
        while p < e {
            if *p == b'*' as c_char && *p.add(1) == b'*' as c_char {
                starstar = true;
            }
            p = p.add(1);
        }

        let starts_with_dot = *s == b'.' as c_char;
        let Some(pat) = file_pat_to_reg_pat(s, e, ptr::null_mut(), 0) else {
            return 0;
        };

        let mut regmatch = regmatch_T {
            regprog: ptr::null_mut(),
            startp: [ptr::null_mut(); 10],
            endp: [ptr::null_mut(); 10],
            rm_matchcol: 0,
            #[cfg(unix)]
            rm_ic: (flags & EW_ICASE) != 0 || opt_vars().p_fic != 0,
            #[cfg(not(unix))]
            rm_ic: true,
        };

        if (flags & (EW_NOERROR | EW_NOTWILD)) != 0 {
            g_st().emsg_silent += 1;
        }
        let nobreak = (flags & EW_NOBREAK) != 0;
        regmatch.regprog =
            vim_regcomp(pat.as_ptr().cast_mut(), RE_MAGIC | if nobreak { RE_NOBREAK } else { 0 });
        if (flags & (EW_NOERROR | EW_NOTWILD)) != 0 {
            g_st().emsg_silent -= 1;
        }
        drop(pat);

        if regmatch.regprog.is_null() && (flags & EW_NOTWILD) == 0 {
            return 0;
        }

        let mut len = s.offset_from(buf) as usize;
        if !didstar
            && path_st().STARDEPTH < 100
            && starstar
            && e.offset_from(s) == 2
            && *path_end == b'/' as c_char
        {
            vim_snprintf!(s, buflen - len, c"%s", path_end.add(1));
            path_st().STARDEPTH += 1;
            do_path_expand(gap, buf, len, flags, true);
            path_st().STARDEPTH -= 1;
        }
        *s = NUL;

        let mut dir = Directory::new();
        let dirpath = if *buf == NUL { c".".as_ptr() } else { buf };
        if os_file_is_readable(dirpath) && os_scandir(&mut dir, dirpath) {
            scandir_next_with_dots(None);
            while !g_st().got_int {
                let name = scandir_next_with_dots(Some(&mut dir));
                if name.is_null() {
                    break;
                }
                len = s.offset_from(buf) as usize;
                let dot_ok = *name != b'.' as c_char
                    || starts_with_dot
                    || ((flags & EW_DODOT) != 0
                        && *name.add(1) != NUL
                        && (*name.add(1) != b'.' as c_char || *name.add(2) != NUL));
                let reg_ok = (!regmatch.regprog.is_null() && vim_regexec(&mut regmatch, name, 0))
                    || ((flags & EW_NOTWILD) != 0
                        && path_cmp(opt_vars().p_fic != 0, path.add(len), name, e.offset_from(s) as usize)
                            == 0);
                if dot_ok && reg_ok {
                    len += vim_snprintf!(s, buflen - len, c"%s", name) as usize;
                    if len + 1 >= buflen {
                        continue;
                    }

                    if starstar && path_st().STARDEPTH < 100 {
                        vim_snprintf!(buf.add(len), buflen - len, c"/**%s", path_end);
                        path_st().STARDEPTH += 1;
                        do_path_expand(gap, buf, len + 1, flags, true);
                        path_st().STARDEPTH -= 1;
                    }

                    vim_snprintf!(buf.add(len), buflen - len, c"%s", path_end);
                    if path_has_wildcard(path_end, false) {
                        if path_st().STARDEPTH < 100 {
                            path_st().STARDEPTH += 1;
                            do_path_expand(gap, buf, len + 1, flags, false);
                            path_st().STARDEPTH -= 1;
                        }
                    } else {
                        let mut file_info: FileInfo = FileInfo::new();
                        if *path_end != NUL {
                            backslash_halve(buf.add(len + 1));
                        }
                        if if (flags & EW_ALLLINKS) != 0 {
                            os_fileinfo_link(buf, &mut file_info)
                        } else {
                            os_path_exists(buf)
                        } {
                            addfile(gap, buf, flags);
                        }
                    }
                }
            }
            os_closedir(&mut dir);
        }

        drop(buf_vec);
        vim_regfree(regmatch.regprog);

        let matches = gap.len() - start_len;
        if matches > 0 && !g_st().got_int {
            gap[start_len..].sort_by(|a, b| {
                let cmp = path_cmp(opt_vars().p_fic != 0, a.as_ptr(), b.as_ptr(), MAXPATHL);
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
}

fn find_previous_pathsep(path: *mut c_char, psep: *mut *mut c_char) -> bool {
    unsafe {
        if *psep > path && vim_ispathsep(**psep as c_int) {
            *psep = (*psep).sub(1);
        }

        while *psep > path {
            if vim_ispathsep(**psep as c_int) {
                return true;
            }
            let off = utf_head_off(path, (*psep).sub(1)) + 1;
            *psep = (*psep).sub(off as usize);
        }

        false
    }
}

fn is_unique(maybe_unique: *mut c_char, gap: &[std::ffi::CString], i: usize) -> bool {
    unsafe {
        let candidate_len = crate::memory::cstr_len(maybe_unique);

        for (j, other_cstr) in gap.iter().enumerate() {
            if j == i {
                continue;
            }
            let other = other_cstr.as_ptr();
            let other_path_len = other_cstr.as_bytes().len();
            if other_path_len < candidate_len {
                continue;
            }
            let rival = other.add(other_path_len - candidate_len);
            if path_equal(maybe_unique, rival, kPathCmpLiteral)
                && (rival == other || vim_ispathsep(*rival.sub(1) as c_int))
            {
                return false;
            }
        }
        true
    }
}

fn expand_path_option(
    curdir: *mut c_char,
    mut path_option: *mut c_char,
    gap: &mut Vec<std::ffi::CString>,
) {
    unsafe {
        let mut buf_arr = [0 as c_char; MAXPATHL];
        let buf = buf_arr.as_mut_ptr();
        let mut curdirlen: usize = 0;

        while *path_option != NUL {
            let mut buflen = copy_option_part(&mut path_option, buf, MAXPATHL, c" ,");

            if !vim_strchr(buf, b'`' as c_int).is_null() {
                continue;
            }

            if *buf == b'.' as c_char && (*buf.add(1) == NUL || vim_ispathsep(*buf.add(1) as c_int))
            {
                if (*g_st().curbuf).b_ffname.is_null() {
                    continue;
                }
                let p = path_tail((*g_st().curbuf).b_ffname);
                let plen = p.offset_from((*g_st().curbuf).b_ffname) as usize;
                if plen + crate::memory::cstr_len(buf) >= MAXPATHL {
                    continue;
                }
                if *buf.add(1) == NUL {
                    *buf.add(plen) = NUL;
                } else {
                    ptr::copy(buf.add(2), buf.add(plen), (buflen - 2) + 1);
                }
                ptr::copy((*g_st().curbuf).b_ffname, buf, plen);
                buflen = simplify_filename(buf);
            } else if *buf == NUL {
                let c_len = crate::memory::cstr_len(curdir);
                ptr::copy_nonoverlapping(curdir, buf, c_len + 1);
                if curdirlen == 0 {
                    curdirlen = c_len;
                }
                buflen = curdirlen;
            } else if path_with_url(buf) != 0 {
                continue;
            } else if !path_is_absolute(buf) {
                if curdirlen == 0 {
                    curdirlen = crate::memory::cstr_len(curdir);
                }
                if curdirlen + buflen + 3 > MAXPATHL {
                    continue;
                }
                ptr::copy(buf, buf.add(curdirlen + 1), buflen + 1);
                ptr::copy_nonoverlapping(curdir, buf, curdirlen);
                *buf.add(curdirlen) = PATHSEP;
                buflen = simplify_filename(buf);
            }

            gap.push(
                std::ffi::CString::new(std::slice::from_raw_parts(buf.cast::<u8>(), buflen))
                    .unwrap_or_default(),
            );
        }
    }
}

fn get_path_cutoff(fname: *mut c_char, gap: &[std::ffi::CString]) -> *mut c_char {
    unsafe {
        let mut maxlen: c_int = 0;
        let mut cutoff: *mut c_char = ptr::null_mut();

        for item in gap {
            let part = item.as_ptr();
            let mut j: usize = 0;

            while {
                let fc = *fname.add(j);
                let pc = *part.add(j);
                #[cfg(windows)]
                let eq = fc == pc || (vim_ispathsep(fc as c_int) && vim_ispathsep(pc as c_int));
                #[cfg(not(windows))]
                let eq = fc == pc;
                eq && fc != NUL && pc != NUL
            } {
                j += 1;
            }
            if (j as c_int) > maxlen {
                maxlen = j as c_int;
                cutoff = fname.add(j);
            }
        }

        if !cutoff.is_null() {
            cutoff = path_skip_sep(cutoff, true);
        }

        cutoff
    }
}

fn uniquefy_paths(
    gap: &mut Vec<std::ffi::CString>,
    pattern: *mut c_char,
    path_option: *mut c_char,
) {
    unsafe {
        let mut sort_again = false;
        let mut path_ga: Vec<std::ffi::CString> = Vec::new();

        remove_duplicate_strings(gap);

        let mut len = crate::memory::cstr_len(pattern);
        let mut file_pattern = Vec::with_capacity(len + 2);
        file_pattern.push(b'*' as c_char);
        file_pattern.extend_from_slice(std::slice::from_raw_parts(pattern, len + 1));
        let Some(pat) = file_pat_to_reg_pat(
            file_pattern.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            0,
        ) else {
            return;
        };
        drop(file_pattern);

        let mut regmatch = regmatch_T {
            regprog: vim_regcomp(pat.as_ptr().cast_mut(), RE_MAGIC + RE_STRING),
            startp: [ptr::null_mut(); 10],
            endp: [ptr::null_mut(); 10],
            rm_matchcol: 0,
            rm_ic: true,
        };
        drop(pat);
        if regmatch.regprog.is_null() {
            return;
        }

        let mut curdir_arr = [0 as c_char; MAXPATHL];
        let curdir = curdir_arr.as_mut_ptr();
        os_dirname(curdir, MAXPATHL);
        expand_path_option(curdir, path_option, &mut path_ga);

        let mut in_curdir: Vec<Option<std::ffi::CString>> = vec![None; gap.len()];

        for i in 0..gap.len() {
            if g_st().got_int {
                break;
            }
            let path = gap[i].as_ptr().cast_mut();
            let dir_end = gettail_dir(path);

            len = gap[i].as_bytes().len();
            let dir_len = dir_end.offset_from(path) as usize;
            let is_in_curdir =
                path_cmp(opt_vars().p_fic != 0, curdir, path, dir_len) == 0 && *curdir.add(dir_len) == NUL;
            if is_in_curdir {
                in_curdir[i] = Some(gap[i].clone());
            }

            let path_cutoff = get_path_cutoff(path, &path_ga);

            if *pattern == b'*' as c_char
                && *pattern.add(1) == b'*' as c_char
                && vim_ispathsep_nocolon(*pattern.add(2) as c_int)
                && !path_cutoff.is_null()
                && vim_regexec(&mut regmatch, path_cutoff, 0)
                && is_unique(path_cutoff, gap, i)
            {
                sort_again = true;
                gap[i] = crate::memory::cstr_to_cstring(path_cutoff);
            } else {
                let mut pathsep_p = path.add(len.saturating_sub(1));
                while find_previous_pathsep(path, &mut pathsep_p) {
                    let candidate = pathsep_p.add(1);
                    if vim_regexec(&mut regmatch, candidate, 0)
                        && is_unique(candidate, gap, i)
                        && !path_cutoff.is_null()
                        && candidate >= path_cutoff
                    {
                        sort_again = true;
                        gap[i] = crate::memory::cstr_to_cstring(candidate);
                        break;
                    }
                }
            }

            let path = gap[i].as_ptr().cast_mut();
            if path_is_absolute(path) {
                let short_name = path_shorten_fname(path, curdir);
                if !short_name.is_null() && short_name > path.add(1) {
                    let rel_pathsize =
                        1 + (PATHSEPSTR.len() - 1) + crate::memory::cstr_len(short_name) + 1;
                    let mut rel_vec = vec![0 as c_char; rel_pathsize];
                    vim_snprintf!(
                        rel_vec.as_mut_ptr(),
                        rel_pathsize,
                        c".%s%s",
                        PATHSEPSTR.as_ptr().cast::<c_char>(),
                        short_name,
                    );
                    gap[i] = crate::memory::cstr_to_cstring(rel_vec.as_ptr());
                }
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
            let path = path_cstr.as_ptr().cast_mut();

            let mut short_name = path_shorten_fname(path, curdir);
            if short_name.is_null() {
                short_name = path;
            }
            if is_unique(short_name, gap, i) {
                gap[i] = crate::memory::cstr_to_cstring(short_name);
                continue;
            }

            let rel_pathsize =
                1 + (PATHSEPSTR.len() - 1) + crate::memory::cstr_len(short_name) + 1;
            let mut rel_vec = vec![0 as c_char; rel_pathsize];
            vim_snprintf!(
                rel_vec.as_mut_ptr(),
                rel_pathsize,
                c".%s%s",
                PATHSEPSTR.as_ptr().cast::<c_char>(),
                short_name,
            );

            gap[i] = crate::memory::cstr_to_cstring(rel_vec.as_ptr());
            sort_again = true;
            os_breakcheck();
        }

        vim_regfree(regmatch.regprog);

        if sort_again {
            remove_duplicate_strings(gap);
        }
    }
}

pub fn gettail_dir(fname: *const c_char) -> *const c_char {
    unsafe {
        let mut dir_end = fname;
        let mut next_dir_end = fname;
        let mut look_for_sep = true;

        let mut p = fname;
        while *p != NUL {
            if vim_ispathsep(*p as c_int) {
                if look_for_sep {
                    next_dir_end = p;
                    look_for_sep = false;
                }
            } else {
                if !look_for_sep {
                    dir_end = next_dir_end;
                }
                look_for_sep = true;
            }
            p = p.add(1);
        }
        dir_end
    }
}

fn expand_in_path(
    gap: &mut Vec<std::ffi::CString>,
    pattern: *mut c_char,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut path_ga: Vec<std::ffi::CString> = Vec::new();
        let path_option = if *(*g_st().curbuf).b_p_path == NUL {
            opt_vars().p_path
        } else {
            (*g_st().curbuf).b_p_path
        };

        let mut curdir_arr = [0 as c_char; MAXPATHL];
        let curdir = curdir_arr.as_mut_ptr();
        os_dirname(curdir, MAXPATHL);

        if (flags & EW_CDPATH) != 0 {
            expand_path_option(curdir, opt_vars().p_cdpath, &mut path_ga);
        } else {
            expand_path_option(curdir, path_option, &mut path_ga);
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
            paths.as_ptr().cast_mut(),
            pattern,
            gap,
            glob_flags,
            (flags & EW_CDPATH) != 0,
        );

        gap.len() as c_int
    }
}

fn has_env_var(mut p: *const c_char) -> bool {
    unsafe {
        while *p != NUL {
            if *p == b'\\' as c_char && *p.add(1) != NUL {
                p = p.add(2);
                continue;
            }
            if !vim_strchr(c"$".as_ptr(), *p as u8 as c_int).is_null() {
                return true;
            }
            p = p.add(1);
        }
        false
    }
}

#[cfg(unix)]
fn has_special_wildchar(mut p: *const c_char, flags: c_int) -> bool {
    unsafe {
        while *p != NUL {
            if *p == b'\r' as c_char || *p == b'\n' as c_char {
                break;
            }
            if *p == b'\\' as c_char
                && *p.add(1) != NUL
                && *p.add(1) != b'\r' as c_char
                && *p.add(1) != b'\n' as c_char
            {
                p = p.add(2);
                continue;
            }
            if !vim_strchr(SPECIAL_WILDCHAR, *p as u8 as c_int).is_null() {
                if *p == b'{' as c_char && (flags & EW_NOTFOUND) == 0 {
                    p = p.add(1);
                    continue;
                }
                if *p == b'{' as c_char && vim_strchr(p, b'}' as c_int).is_null() {
                    p = p.add(1);
                    continue;
                }
                if (*p == b'`' as c_char || *p == b'\'' as c_char)
                    && vim_strchr(p.add(1), *p as u8 as c_int).is_null()
                {
                    p = p.add(1);
                    continue;
                }
                return true;
            }
            p = p.add(1);
        }
        false
    }
}

pub fn gen_expand_wildcards_vec(
    num_pat: c_int,
    pat: *mut *mut c_char,
    file: &mut Vec<std::ffi::CString>,
    flags: c_int,
) -> bool {
    unsafe {
        file.clear();
        let mut did_expand_in_path = false;
        let path_option = if *(*g_st().curbuf).b_p_path == NUL {
            opt_vars().p_path
        } else {
            (*g_st().curbuf).b_p_path
        };

        if path_st().EXPAND_RECURSIVE {
            #[cfg(unix)]
            {
                return os_expand_wildcards(num_pat, pat, file, flags);
            }
            #[cfg(not(unix))]
            {
                return false;
            }
        }

        #[cfg(unix)]
        for i in 0..num_pat {
            let pi = *pat.add(i as usize);
            if has_special_wildchar(pi, flags)
                && !(vim_backtick(pi) && *pi.add(1) == b'=' as c_char)
            {
                return os_expand_wildcards(num_pat, pat, file, flags);
            }
        }

        path_st().EXPAND_RECURSIVE = true;

        for i in 0..num_pat {
            if g_st().got_int {
                break;
            }
            let mut add_pat: c_int = -1;
            let mut p = *pat.add(i as usize);
            let mut p_owned: Option<std::ffi::CString> = None;

            if vim_backtick(p) {
                add_pat = expand_backtick(file, p, flags);
                if add_pat == -1 {
                    path_st().EXPAND_RECURSIVE = false;
                    file.clear();
                    return false;
                }
            } else {
                if (has_env_var(p) && (flags & EW_NOTENV) == 0) || *p == b'~' as c_char {
                    let po = expand_env_save_opt(p, true, PATH_ESC_WILDCARDS.cast_mut());
                    p = po.as_ptr().cast_mut();
                    p_owned = Some(po);
                    #[cfg(unix)]
                    if has_env_var(p) || *p == b'~' as c_char {
                        drop(p_owned);
                        file.clear();
                        let res =
                            os_expand_wildcards(num_pat, pat, file, flags | EW_KEEPDOLLAR);
                        path_st().EXPAND_RECURSIVE = false;
                        return res;
                    }
                }

                if path_has_wildcard(p, false) || (flags & EW_ICASE) != 0 {
                    if (flags & (EW_PATH | EW_CDPATH)) != 0
                        && !path_is_absolute(p)
                        && !(*p == b'.' as c_char
                            && (vim_ispathsep(*p.add(1) as c_int)
                                || (*p.add(1) == b'.' as c_char
                                    && vim_ispathsep(*p.add(2) as c_int))))
                    {
                        path_st().EXPAND_RECURSIVE = false;
                        add_pat = expand_in_path(file, p, flags);
                        path_st().EXPAND_RECURSIVE = true;
                        did_expand_in_path = true;
                    } else {
                        path_st().EXPAND_RECURSIVE = false;
                        let tmp_add_pat = path_expand(file, p, flags);
                        path_st().EXPAND_RECURSIVE = true;
                        debug_assert!(tmp_add_pat <= c_int::MAX as usize);
                        add_pat = tmp_add_pat as c_int;
                    }
                }
            }

            if add_pat == -1 || (add_pat == 0 && (flags & EW_NOTFOUND) != 0) {
                let t_owned = backslash_halve_save(p);
                let t = t_owned.as_ptr().cast_mut();
                if (flags & EW_NOTFOUND) != 0 {
                    addfile(file, t, flags | EW_DIR | EW_FILE);
                } else {
                    addfile(file, t, flags);
                }
            }

            if did_expand_in_path && !file.is_empty() && (flags & (EW_PATH | EW_CDPATH)) != 0 {
                path_st().EXPAND_RECURSIVE = false;
                uniquefy_paths(file, p, path_option);
                path_st().EXPAND_RECURSIVE = true;
            }
            drop(p_owned);
        }

        path_st().EXPAND_RECURSIVE = false;

        (flags & EW_EMPTYOK) != 0 || !file.is_empty()
    }
}

pub fn expand_wildcards_vec(
    num_pat: c_int,
    pat: *mut *mut c_char,
    files: &mut Vec<std::ffi::CString>,
    flags: c_int,
) -> bool {
    unsafe {
        let retval = gen_expand_wildcards_vec(num_pat, pat, files, flags);

        if (flags & EW_KEEPALL) != 0 || !retval {
            return retval;
        }

        if *opt_vars().p_wig != NUL {
            files.retain(|fi_cstr| {
                let fi = fi_cstr.as_ptr().cast_mut();
                let ffname = FullName_save(fi, false).unwrap_or_default();
                !match_file_list(opt_vars().p_wig, fi, ffname.as_ptr().cast_mut())
            });
        }

        if files.len() > 1 && !g_st().got_int {
            let mut non_suf_match = 0;
            for i in 0..files.len() {
                let fi = files[i].as_ptr().cast_mut();
                if !match_suffix(fi) {
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
}

fn vim_backtick(p: *const c_char) -> bool {
    unsafe {
        if *p != b'`' as c_char || *p.add(1) == NUL {
            return false;
        }
        let len = crate::memory::cstr_len(p);
        *p.add(len - 1) == b'`' as c_char
    }
}

fn expand_backtick(
    gap: &mut Vec<std::ffi::CString>,
    pat: *mut c_char,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut cnt: c_int = 0;
        let pat_len = crate::memory::cstr_len(pat);
        let mut cmd_vec: Vec<c_char> = std::slice::from_raw_parts(pat.add(1), pat_len - 2).to_vec();
        cmd_vec.push(NUL);
        let cmd_ptr = cmd_vec.as_mut_ptr();

        let cmd_out: Option<std::ffi::CString> = if *cmd_ptr == b'=' as c_char {
            eval_to_string(cmd_ptr.add(1), true, false)
        } else {
            get_cmd_output(
                cmd_ptr,
                ptr::null_mut(),
                if (flags & EW_SILENT) != 0 {
                    kShellOptSilent
                } else {
                    0
                },
            )
        };
        drop(cmd_vec);
        let Some(cmd_cstr) = cmd_out else {
            return -1;
        };
        let mut cmd_buf: Vec<c_char> = cmd_cstr
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();

        let mut cmd = cmd_buf.as_mut_ptr();
        while *cmd != NUL {
            cmd = skipwhite(cmd);
            let mut p = cmd;
            while *p != NUL && *p != b'\r' as c_char && *p != b'\n' as c_char {
                p = p.add(1);
            }
            if p > cmd {
                let saved = *p;
                *p = NUL;
                addfile(gap, cmd, flags);
                *p = saved;
                cnt += 1;
            }
            cmd = p;
            while *cmd != NUL && (*cmd == b'\r' as c_char || *cmd == b'\n' as c_char) {
                cmd = cmd.add(1);
            }
        }

        cnt
    }
}

#[cfg(windows)]
pub fn slash_adjust(p: *mut c_char) {
    unsafe {
        if path_with_url(p) != 0 {
            return;
        }

        if *p == b'`' as c_char {
            let len = crate::memory::cstr_len(p);
            if len > 2 && *p.add(len - 1) == b'`' as c_char {
                return;
            }
        }

        let from = if opt_vars().p_ssl != 0 { b'\\' as c_char } else { PATHSEP };
        let to = if opt_vars().p_ssl != 0 { PATHSEP } else { b'\\' as c_char };
        { let _p = p; let _len = crate::memory::cstr_len(_p); for _b in std::slice::from_raw_parts_mut(_p.cast::<u8>(), _len) { if *_b == (from) as u8 { *_b = (to) as u8; } } };
    }
}

#[cfg(windows)]
pub fn path_to_backslash(p: *mut c_char) -> *mut c_char {
    if !p.is_null() {
        unsafe {
            { let _p = p; let _len = crate::memory::cstr_len(_p); for _b in std::slice::from_raw_parts_mut(_p.cast::<u8>(), _len) { if *_b == (PATHSEP) as u8 { *_b = (b'\\' as c_char) as u8; } } };
        }
    }
    p
}

#[cfg(windows)]
pub fn path_to_slash(p: *mut c_char) -> *mut c_char {
    if !p.is_null() {
        unsafe {
            if path_with_url(p) == 0 {
                { let _p = p; let _len = crate::memory::cstr_len(_p); for _b in std::slice::from_raw_parts_mut(_p.cast::<u8>(), _len) { if *_b == (b'\\' as c_char) as u8 { *_b = (PATHSEP) as u8; } } };
            }
        }
    }
    p
}

#[cfg(windows)]
pub fn path_to_slash_save(p: *const c_char) -> Option<std::ffi::CString> {
    if p.is_null() {
        None
    } else {
        let mut bytes = cstr_to_cstring(p).into_bytes_with_nul();
            path_to_slash(bytes.as_mut_ptr().cast::<c_char>());
            Some(std::ffi::CString::from_vec_with_nul(bytes).unwrap())
    }
}

pub fn addfile(gap: &mut Vec<std::ffi::CString>, f: *mut c_char, flags: c_int) {
    unsafe {
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
            let bytes = crate::memory::cstr_as_bytes(f);
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

        let flen = crate::memory::cstr_len(f);
        let mut buf = vec![0 as c_char; flen + 1 + usize::from(isdir)];
        ptr::copy_nonoverlapping(f, buf.as_mut_ptr(), flen + 1);
        to_slash(buf.as_mut_ptr());
        if isdir && (flags & EW_ADDSLASH) != 0 {
            add_pathsep(buf.as_mut_ptr());
        }
        gap.push(crate::memory::cstr_to_cstring(buf.as_ptr()));
    }
}

pub fn simplify_filename(filename: *mut c_char) -> usize {
    unsafe {
        let mut components: c_int = 0;
        let mut stripping_disabled = false;
        let mut relative = true;

        let mut p = filename;
        #[cfg(windows)]
        if *p != NUL && *p.add(1) == b':' as c_char {
            p = p.add(2);
        }

        if vim_ispathsep(*p as c_int) {
            relative = false;
            p = path_skip_sep(p, true);
        }
        #[cfg(unix)]
        let mut start = p;
        #[cfg(not(unix))]
        let start = p;
        let mut p_end = p.add(crate::memory::cstr_len(p));

        #[cfg(unix)]
        if start > filename.add(2) {
            let move_len = p_end.offset_from(p) as usize + 1;
            ptr::copy(p, filename.add(1), move_len);
            p_end = p_end.sub(p.offset_from(filename.add(1)) as usize);
            p = filename.add(1);
            start = p;
        }

        loop {
            if vim_ispathsep(*p as c_int) {
                let move_len = p_end.offset_from(p.add(1)) as usize + 1;
                ptr::copy(p.add(1), p, move_len);
                p_end = p_end.sub(1);
            } else if *p == b'.' as c_char
                && (vim_ispathsep(*p.add(1) as c_int) || *p.add(1) == NUL)
            {
                if p == start && relative {
                    p = p.add(1 + usize::from(*p.add(1) != NUL));
                } else {
                    let mut tail = p.add(1);
                    if *p.add(1) != NUL {
                        tail = path_skip_sep(tail, true);
                    } else if p > start {
                        p = p.sub(1);
                    }
                    let move_len = p_end.offset_from(tail) as usize + 1;
                    ptr::copy(tail, p, move_len);
                    p_end = p_end.sub(tail.offset_from(p) as usize);
                }
            } else if *p == b'.' as c_char
                && *p.add(1) == b'.' as c_char
                && (vim_ispathsep(*p.add(2) as c_int) || *p.add(2) == NUL)
            {
                let mut tail = path_skip_sep(p.add(2), true);

                if components > 0 {
                    let mut do_strip = false;

                    if !stripping_disabled {
                        let saved_char = *p.sub(1);
                        *p.sub(1) = NUL;
                        let mut file_info: FileInfo = FileInfo::new();
                        if !os_fileinfo_link(filename, &mut file_info) {
                            do_strip = true;
                        }
                        *p.sub(1) = saved_char;

                        p = p.sub(1);
                        while p > start && after_pathsep(start, p) == 0 {
                            let off = utf_head_off(start, p.sub(1)) + 1;
                            p = p.sub(off as usize);
                        }

                        if !do_strip {
                            let saved_char = *tail;
                            *tail = NUL;
                            if os_fileinfo(filename, &mut file_info) {
                                do_strip = true;
                            } else {
                                stripping_disabled = true;
                            }
                            *tail = saved_char;
                            if do_strip {
                                let mut new_file_info: FileInfo = FileInfo::new();
                                if p == start && relative {
                                    os_fileinfo(c".".as_ptr(), &mut new_file_info);
                                } else {
                                    let saved_char = *p;
                                    *p = NUL;
                                    os_fileinfo(filename, &mut new_file_info);
                                    *p = saved_char;
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
                        if p == start && relative && *tail.sub(1) == b'.' as c_char {
                            *p = b'.' as c_char;
                            p = p.add(1);
                            *p = NUL;
                        } else {
                            if p > start && *tail.sub(1) == b'.' as c_char {
                                p = p.sub(1);
                            }
                            let move_len = p_end.offset_from(tail) as usize + 1;
                            ptr::copy(tail, p, move_len);
                            p_end = p_end.sub(tail.offset_from(p) as usize);
                        }
                        components -= 1;
                    }
                } else if p == start && !relative {
                    let move_len = p_end.offset_from(tail) as usize + 1;
                    ptr::copy(tail, p, move_len);
                    p_end = p_end.sub(tail.offset_from(p) as usize);
                } else {
                    if p == start.add(2) && *p.sub(2) == b'.' as c_char {
                        let move_len = p_end.offset_from(p) as usize + 1;
                        ptr::copy(p, p.sub(2), move_len);
                        p_end = p_end.sub(2);
                        tail = tail.sub(2);
                    }
                    p = tail;
                }
            } else {
                components += 1;
                p = path_next_component(p).cast_mut();
            }

            if *p == NUL {
                break;
            }
        }

        p_end.offset_from(filename) as usize
    }
}

pub fn path_has_drive_letter(p: *const c_char, path_len: usize) -> bool {
    unsafe {
        path_len >= 2
            && (*p as u8).is_ascii_alphabetic()
            && (*p.add(1) == b':' as c_char || *p.add(1) == b'|' as c_char)
            && (path_len == 2 || {
                let c2 = *p.add(2) as u8;
                c2 == b'/' || c2 == b'\\' || c2 == b'?' || c2 == b'#'
            })
    }
}

pub fn path_is_url(p: *const c_char) -> c_int {
    unsafe {
        if *p == b':' as c_char && *p.add(1) == b'/' as c_char {
            URL_SLASH
        } else if *p == b':' as c_char
            && *p.add(1) == b'\\' as c_char
            && *p.add(2) == b'\\' as c_char
        {
            URL_BACKSLASH
        } else {
            0
        }
    }
}

pub fn path_with_url(fname: *const c_char) -> c_int {
    unsafe {
        if !(*fname as u8).is_ascii_alphabetic() {
            return 0;
        }

        if path_has_drive_letter(fname, crate::memory::cstr_len(fname)) {
            return 0;
        }

        let mut p = fname.add(1);
        while {
            let b = *p as u8;
            b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.'
        } {
            p = p.add(1);
        }

        let prev = *p.sub(1) as u8;
        if prev == b'+' || prev == b'-' || prev == b'.' {
            return 0;
        }

        path_is_url(p)
    }
}

pub fn path_with_extension(path: *const c_char, extension: &std::ffi::CStr) -> bool {
    unsafe {
        let bytes = crate::memory::cstr_as_bytes(path);
        let Some(pos) = bytes.iter().rposition(|&b| b == b'.') else {
            return false;
        };
        let last_dot = path.add(pos);
        mb_strcmp_ic(opt_vars().p_fic != 0, last_dot.add(1), extension.as_ptr()) == 0
    }
}

pub fn vim_isAbsName(name: *const c_char) -> bool {
    path_with_url(name) != 0 || path_is_absolute(name)
}

pub fn vim_FullName(
    fname: *const c_char,
    buf: *mut c_char,
    len: usize,
    force: bool,
) -> bool {
    unsafe {
        *buf = NUL;
        if fname.is_null() {
            return false;
        }

        if crate::memory::cstr_len(fname) > len.saturating_sub(1) {
            xstrlcpy(buf, fname, len);
            to_slash(buf);
            return false;
        }

        if path_with_url(fname) != 0 {
            xstrlcpy(buf, fname, len);
            return true;
        }

        let rv = path_to_absolute(fname, buf, len, c_int::from(force));
        if !rv {
            xstrlcpy(buf, fname, len);
        }
        to_slash(buf);
        rv
    }
}

pub fn fix_fname(fname: *const c_char) -> Option<std::ffi::CString> {
    #[cfg(unix)]
    {
        FullName_save(fname, true)
    }
    #[cfg(not(unix))]
    unsafe {
        if fname.is_null() {
            return None;
        }
        if !vim_isAbsName(fname)
            || str_contains(fname, b"..")
            || str_contains(fname, b"//")
            || str_contains(fname, b"\\\\")
            || *fname == b'/' as c_char
            || *fname == b'\\' as c_char
        {
            return FullName_save(fname, false);
        }

        let mut bytes = to_slash_save(fname)?.into_bytes_with_nul();
        path_fix_case(bytes.as_mut_ptr().cast::<c_char>());
        let len = crate::memory::cstr_len(bytes.as_ptr().cast::<c_char>());
        Some(cstr_from_bytes(&bytes[..len]))
    }
}

pub fn path_fix_case(name: *mut c_char) {
    unsafe {
        let mut file_info: FileInfo = FileInfo::new();
        if !os_fileinfo_link(name, &mut file_info) {
            return;
        }

        let name_bytes = crate::memory::cstr_as_bytes(name);
        let slash_pos = name_bytes.iter().rposition(|&b| b == b'/');
        let mut dir = Directory::new();
        let (ok, tail) = if let Some(pos) = slash_pos {
            let slash = name.add(pos);
            *slash = NUL;
            let ok = os_scandir(&mut dir, name);
            *slash = b'/' as c_char;
            (ok, slash.add(1))
        } else {
            let ok = os_scandir(&mut dir, c".".as_ptr());
            (ok, name)
        };

        if !ok {
            return;
        }

        let taillen = crate::memory::cstr_len(tail);
        loop {
            let entry = os_scandir_next(&mut dir);
            if entry.is_null() {
                break;
            }
            if crate::memory::cstr_as_bytes(tail).eq_ignore_ascii_case(crate::memory::cstr_as_bytes(entry)) && taillen == crate::memory::cstr_len(entry) {
                let mut newname = [0 as c_char; MAXPATHL + 1];
                let tail_off = tail.offset_from(name) as usize;
                xstrlcpy(newname.as_mut_ptr(), name, MAXPATHL + 1);
                xstrlcpy(
                    newname.as_mut_ptr().add(tail_off),
                    entry,
                    MAXPATHL - tail_off + 1,
                );
                let mut file_info_new: FileInfo = FileInfo::new();
                if os_fileinfo_link(newname.as_ptr(), &mut file_info_new)
                    && os_fileinfo_id_equal(&file_info, &file_info_new)
                {
                    ptr::copy_nonoverlapping(entry, tail, taillen + 1);
                    break;
                }
            }
        }

        os_closedir(&mut dir);
    }
}

pub fn after_pathsep(b: *const c_char, p: *const c_char) -> c_int {
    unsafe {
        c_int::from(p > b && vim_ispathsep(*p.sub(1) as c_int) && utf_head_off(b, p.sub(1)) == 0)
    }
}

pub fn same_directory(f1: *mut c_char, f2: *mut c_char) -> bool {
    unsafe {
        if f1.is_null() || f2.is_null() {
            return false;
        }

        let mut ffname = [0 as c_char; MAXPATHL];
        vim_FullName(f1, ffname.as_mut_ptr(), MAXPATHL, false);
        let t1 = path_tail_with_sep(ffname.as_mut_ptr());
        let t2 = path_tail_with_sep(f2);
        let d1 = t1.offset_from(ffname.as_mut_ptr());
        let d2 = t2.offset_from(f2);
        d1 == d2 && path_cmp(opt_vars().p_fic != 0, ffname.as_ptr(), f2, d1 as usize) == 0
    }
}

pub fn path_fold_char(ic: bool, p: *const c_char, len: &mut c_int) -> c_int {
    unsafe {
        if vim_ispathsep_nocolon(*p as c_int) {
            *len = 1;
            return PATHSEP as c_int;
        }
        *len = if ic { utfc_ptr2len(p) } else { 1 };
        if ic {
            utf_fold(utf_ptr2char(p))
        } else {
            *p as u8 as c_int
        }
    }
}

pub fn path_cmp(
    ic: bool,
    mut p: *const c_char,
    mut q: *const c_char,
    maxlen: usize,
) -> c_int {
    unsafe {
        let mut c1: c_int = NUL as c_int;
        let mut c2: c_int = NUL as c_int;
        let mut len: usize = 0;

        #[cfg(windows)]
        {
            if vim_ispathsep_nocolon(*p as c_int)
                && (*q as u8).is_ascii_alphabetic()
                && *q.add(1) == b':' as c_char
            {
                if (*q as u8).to_ascii_lowercase() as c_int == _getdrive() + (b'a' as c_int) - 1 {
                    q = q.add(2);
                }
            } else if vim_ispathsep_nocolon(*q as c_int)
                && (*p as u8).is_ascii_alphabetic()
                && *p.add(1) == b':' as c_char
                && (*p as u8).to_ascii_lowercase() as c_int == _getdrive() + (b'a' as c_int) - 1
            {
                p = p.add(2);
            }
        }

        let mut i: c_int = 0;
        let mut j: c_int = 0;
        while len < maxlen {
            c1 = path_fold_char(ic, p, &mut i);
            c2 = path_fold_char(ic, q, &mut j);
            if c1 == NUL as c_int || c2 == NUL as c_int || c1 != c2 {
                break;
            }
            p = p.add(i as usize);
            q = q.add(j as usize);
            len += i as usize;
        }

        if (c1 == NUL as c_int && c2 == NUL as c_int) || len >= maxlen {
            return 0;
        }

        if c1 != NUL as c_int && c2 != NUL as c_int {
            if vim_ispathsep(c1) {
                return -1;
            }
            if vim_ispathsep(c2) {
                return 1;
            }
            return c1 - c2;
        }

        let s = if c1 == NUL as c_int { q } else { p };
        if vim_ispathsep_nocolon(*s as c_int)
            && *s.add(1) == NUL
            && len > 0
            && !vim_ispathsep(*s.sub(1) as c_int)
        {
            return 0;
        }
        if ptr::eq(s, q) { -1 } else { 1 }
    }
}

pub fn path_try_shorten_fname(full_path: *mut c_char) -> *mut c_char {
    unsafe {
        let mut dirname = [0 as c_char; MAXPATHL];
        let mut p = full_path;

        if os_dirname(dirname.as_mut_ptr(), MAXPATHL) {
            p = path_shorten_fname(full_path, dirname.as_mut_ptr());
            if p.is_null() || *p == NUL {
                p = full_path;
            }
        }
        p
    }
}

pub fn path_shorten_fname(full_path: *mut c_char, dir_name: *mut c_char) -> *mut c_char {
    if full_path.is_null() {
        return ptr::null_mut();
    }

    unsafe {
        debug_assert!(!dir_name.is_null());
        let len = crate::memory::cstr_len(dir_name);

        let ic = opt_vars().p_fic != 0;

        if path_cmp(ic, dir_name, full_path, len) != 0 {
            return ptr::null_mut();
        }

        if len == path_head_length() as usize && is_path_head(dir_name) {
            return full_path.add(len);
        }

        let p = full_path.add(len);
        if !vim_ispathsep(*p as c_int) {
            return ptr::null_mut();
        }

        path_skip_sep(p.add(1), false)
    }
}

pub fn expand_wildcards_eval(
    pat: *mut *mut c_char,
    file: &mut Vec<std::ffi::CString>,
    flags: c_int,
) -> bool {
    unsafe {
        let mut ret = false;
        let mut eval_pat: Option<std::ffi::CString> = None;
        let mut exp_pat_owned: Option<std::ffi::CString> = None;
        let mut exp_pat = *pat;
        let mut ignored_msg: *const c_char = ptr::null();
        let mut usedlen: usize = 0;
        let is_cur_alt_file = *exp_pat == b'%' as c_char || *exp_pat == b'#' as c_char;
        let mut star_follows = false;

        file.clear();

        if is_cur_alt_file || *exp_pat == b'<' as c_char {
            g_st().emsg_off += 1;
            eval_pat = eval_vars(
                exp_pat,
                exp_pat,
                &mut usedlen,
                None,
                &mut ignored_msg,
                None,
                true,
            );
            g_st().emsg_off -= 1;
            if let Some(ref ep) = eval_pat {
                let tail_bytes = crate::memory::cstr_as_bytes(exp_pat.add(usedlen));
                star_follows = tail_bytes == b"*";
                let mut joined = Vec::with_capacity(ep.as_bytes().len() + tail_bytes.len());
                joined.extend_from_slice(ep.as_bytes());
                joined.extend_from_slice(tail_bytes);
                let cstr = crate::memory::cstr_from_bytes(&joined);
                exp_pat = cstr.as_ptr().cast_mut();
                exp_pat_owned = Some(cstr);
            }
        }

        if !exp_pat.is_null() {
            ret = expand_wildcards_vec(1, &mut exp_pat, file, flags);
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
}

pub fn match_suffix(fname: *mut c_char) -> bool {
    unsafe {
        let mut suf_buf = [0 as c_char; MAXSUFLEN];
        let fnamelen = crate::memory::cstr_len(fname);
        let mut setsuflen: usize = 0;
        let mut setsuf = opt_vars().p_su;
        while *setsuf != NUL {
            setsuflen =
                copy_option_part(&mut setsuf, suf_buf.as_mut_ptr(), MAXSUFLEN, c".,");
            if setsuflen == 0 {
                let tail = path_tail(fname);
                if vim_strchr(tail, b'.' as c_int).is_null() {
                    setsuflen = 1;
                    break;
                }
            } else {
                if fnamelen >= setsuflen
                    && path_cmp(
                        opt_vars().p_fic != 0,
                        suf_buf.as_ptr(),
                        fname.add(fnamelen - setsuflen),
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
}

pub fn path_full_dir_name(directory: *mut c_char, buffer: *mut c_char, len: usize) -> bool {
    unsafe {
        if *directory == NUL {
            return os_dirname(buffer, len);
        }

        if !os_realpath(directory, buffer, len).is_null() {
            return true;
        }

        if path_is_absolute(directory) {
            return false;
        }

        let mut old_dir = [0 as c_char; MAXPATHL];
        if !os_dirname(old_dir.as_mut_ptr(), MAXPATHL) {
            return false;
        }

        xstrlcpy(buffer, old_dir.as_ptr(), len);
        if !append_path(buffer, directory, len) {
            return false;
        }

        true
    }
}

pub fn append_path(path: *mut c_char, to_append: *const c_char, max_len: usize) -> bool {
    unsafe {
        let mut current_length = crate::memory::cstr_len(path);
        let to_append_bytes = crate::memory::cstr_as_bytes(to_append);
        let to_append_length = to_append_bytes.len();

        if to_append_length == 0 || to_append_bytes == b"." {
            return true;
        }

        if current_length > 0 && !vim_ispathsep_nocolon(*path.add(current_length - 1) as c_int) {
            let pathsep_len = PATHSEPSTR.len() - 1;
            if current_length + pathsep_len + 1 > max_len {
                return false;
            }
            xstrlcpy(
                path.add(current_length),
                PATHSEPSTR.as_ptr().cast::<c_char>(),
                max_len - current_length,
            );
            current_length += pathsep_len;
        }

        if current_length + to_append_length + 1 > max_len {
            return false;
        }

        xstrlcpy(
            path.add(current_length),
            to_append,
            max_len - current_length,
        );
        true
    }
}

fn path_to_absolute(
    fname: *const c_char,
    buf: *mut c_char,
    len: usize,
    force: c_int,
) -> bool {
    unsafe {
        *buf = NUL;
        let mut rel_dir_vec = vec![0 as c_char; len];
        let relative_directory = rel_dir_vec.as_mut_ptr();
        let mut end_of_path = fname;

        #[cfg(windows)]
        let need_expand = force != 0
            || !path_is_absolute(fname)
            || *fname == b'/' as c_char
            || *fname == b'\\' as c_char;
        #[cfg(not(windows))]
        let need_expand = force != 0 || !path_is_absolute(fname);

        if need_expand {
            let fname_bytes = crate::memory::cstr_as_bytes(fname);
            let mut p: *const c_char = fname_bytes
                .iter()
                .rposition(|&b| b == b'/')
                .map_or(ptr::null(), |pos| fname.add(pos));

            #[cfg(windows)]
            {
                if p.is_null() {
                    p = fname_bytes
                        .iter()
                        .rposition(|&b| b == b'\\')
                        .map_or(ptr::null(), |pos| fname.add(pos));
                }
                if p.is_null()
                    && (*fname as u8).is_ascii_alphabetic()
                    && *fname.add(1) == b':' as c_char
                {
                    p = fname.add(1);
                }
            }

            if p.is_null() && fname_bytes == b".." {
                p = fname.add(2);
            }
            if !p.is_null() {
                if vim_ispathsep(*p as c_int) && crate::memory::cstr_as_bytes(p.add(1)) == b".." {
                    p = p.add(3);
                }
                debug_assert!(p >= fname);
                let copy_len = p.offset_from(fname) as usize + 1;
                ptr::copy_nonoverlapping(fname, relative_directory, copy_len);
                *relative_directory.add(copy_len) = NUL;
                end_of_path = if vim_ispathsep(*p as c_int) {
                    p.add(1)
                } else {
                    p
                };
            } else {
                *relative_directory = NUL;
            }

            if !path_full_dir_name(relative_directory, buf, len) {
                return false;
            }
        }
        drop(rel_dir_vec);
        append_path(buf, end_of_path, len)
    }
}

pub fn path_is_absolute(fname: *const c_char) -> bool {
    #[cfg(windows)]
    unsafe {
        if *fname == NUL {
            return false;
        }
        ((*fname as u8).is_ascii_alphabetic()
            && *fname.add(1) == b':' as c_char
            && vim_ispathsep_nocolon(*fname.add(2) as c_int))
            || vim_ispathsep_nocolon(*fname as c_int)
    }
    #[cfg(not(windows))]
    unsafe {
        *fname == b'/' as c_char || *fname == b'~' as c_char
    }
}

pub fn path_guess_exepath(argv0: *const c_char, buf: *mut c_char, bufsize: usize) {
    unsafe {
        let path_cstr = os_getenv_cstr(c"PATH".as_ptr());
        let argv0_bytes = crate::memory::cstr_as_bytes(argv0);

        if path_is_absolute(argv0) {
            xstrlcpy(buf, argv0, bufsize);
        } else if let Some(ref path_cstr) = path_cstr {
            if *argv0 == b'.' as c_char || argv0_bytes.contains(&(PATHSEP as u8)) {
                if !os_dirname(buf, MAXPATHL) {
                    *buf = NUL;
                }
                { let _dst: *mut c_char = buf; let _dsize: usize = bufsize; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), PATHSEPSTR.as_ptr().cast::<c_char>(), _dsize - _dlen); };
                { let _dst: *mut c_char = buf; let _dsize: usize = bufsize; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), argv0, _dsize - _dlen); };
            } else {
                let path = path_cstr.as_ptr();
                let namebuff = (&raw mut g_st().NameBuff).cast::<c_char>();
                let mut iter: *const c_char = ptr::null();
                loop {
                    let mut dir: *const c_char = ptr::null();
                    let mut dir_len: usize = 0;
                    iter = vim_env_iter(ENV_SEPCHAR, path, iter, &mut dir, &mut dir_len);
                    if dir.is_null() || dir_len == 0 {
                        break;
                    }
                    if dir_len + 1 > MAXPATHL {
                        continue;
                    }
                    crate::memory::cstr_ncpy(namebuff, dir, dir_len);
                    { let _dst: *mut c_char = namebuff; let _dsize: usize = MAXPATHL; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), PATHSEPSTR.as_ptr().cast::<c_char>(), _dsize - _dlen); };
                    { let _dst: *mut c_char = namebuff; let _dsize: usize = MAXPATHL; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), argv0, _dsize - _dlen); };
                    if os_can_exe(namebuff, None, false) {
                        xstrlcpy(buf, namebuff, bufsize);
                        to_slash(buf);
                        return;
                    }
                    if iter.is_null() {
                        break;
                    }
                }
                xstrlcpy(buf, argv0, bufsize);
            }
        } else {
            xstrlcpy(buf, argv0, bufsize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::vars::{VV_PROGPATH, set_vim_var_string};
    use std::ffi::CString;

    fn init_progpath() {
        unsafe {
            if *crate::eval::vars::get_vim_var_str(VV_PROGPATH) == NUL {
                set_vim_var_string(VV_PROGPATH, c"nvim".as_ptr(), -1);
            }
        }
    }

    #[test]
    fn path_tail_next_component_and_invocation_tail() {
         {
            assert!(vim_ispathsep(b'/' as c_int));
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail(c"directory/file.txt".as_ptr())),
                b"file.txt"
            );
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail(c"directory/".as_ptr())),
                b""
            );

            let mut s1 = *b"directory///file.txt\0";
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail_with_sep(s1.as_mut_ptr().cast())),
                b"///file.txt"
            );
            let mut s2 = *b"\0";
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail_with_sep(s2.as_mut_ptr().cast())),
                b""
            );
            let mut s3 = *b"some/directory/\0";
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail_with_sep(s3.as_mut_ptr().cast())),
                b"/"
            );
            let mut s4 = *b"/file.txt\0";
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail_with_sep(s4.as_mut_ptr().cast())),
                b"file.txt"
            );
            let mut s5 = *b"/\0";
            assert_eq!(
                crate::memory::cstr_as_bytes(path_tail_with_sep(s5.as_mut_ptr().cast())),
                b""
            );

            assert_eq!(
                crate::memory::cstr_as_bytes(path_next_component(c"some/directory/file.txt".as_ptr())),
                b"directory/file.txt"
            );
            assert_eq!(
                crate::memory::cstr_as_bytes(path_next_component(c"file.txt".as_ptr())),
                b""
            );

            let mut len = 0usize;
            let inv = invocation_path_tail(c"directory/exe a b c".as_ptr(), Some(&mut len));
            assert_eq!(len, 3);
            assert_eq!(crate::memory::cstr_as_bytes(inv), b"exe a b c");
            assert_eq!(
                crate::memory::cstr_as_bytes(invocation_path_tail(
                    c"directory/exe a b c".as_ptr(),
                    None
                )),
                b"exe a b c"
            );

            #[cfg(windows)]
            let abs_inv = c"C:\\Users\\anyone\\bin\\z a b";
            #[cfg(not(windows))]
            let abs_inv = c"/usr/bin/z a b";
            let inv2 = invocation_path_tail(abs_inv.as_ptr(), Some(&mut len));
            assert_eq!(len, 1);
            assert_eq!(crate::memory::cstr_as_bytes(inv2), b"z a b");

            let inv3 = invocation_path_tail(c"exe a/b\\c".as_ptr(), Some(&mut len));
            assert_eq!(len, 3);
            assert_eq!(crate::memory::cstr_as_bytes(inv3), b"exe a/b\\c");

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
    }

    #[test]
    fn path_cmp_shorten_and_append_path() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            assert_eq!(
                path_cmp(false, c"foo/bar".as_ptr(), c"foo/bar".as_ptr(), 7),
                0
            );
            assert_eq!(
                path_cmp(false, c"foobar".as_ptr(), c"foo/bar".as_ptr(), 7),
                1
            );
            assert_eq!(
                path_cmp(false, c"foo/bar".as_ptr(), c"foobar".as_ptr(), 7),
                -1
            );
            assert_ne!(
                path_cmp(false, c"foo/bar".as_ptr(), c"foo/baz".as_ptr(), 7),
                0
            );
            assert_eq!(
                path_cmp(false, c"foo/bar".as_ptr(), c"foo/baz".as_ptr(), 6),
                0
            );
            assert_eq!(path_cmp(false, c"foo".as_ptr(), c"foo/".as_ptr(), 4), 0);
            assert_ne!(path_cmp(false, c"/".as_ptr(), c"//".as_ptr(), 2), 0);
            assert_ne!(path_cmp(false, c"foo".as_ptr(), c"foo//".as_ptr(), 5), 0);
            assert_ne!(path_cmp(false, c"foo/".as_ptr(), c"foo//".as_ptr(), 5), 0);

            let mut dir = *b"some/very/long\0";
            assert!(path_shorten_fname(ptr::null_mut(), dir.as_mut_ptr().cast()).is_null());
            let mut other = *b"as/this.txt\0";
            assert!(
                path_shorten_fname(other.as_mut_ptr().cast(), dir.as_mut_ptr().cast()).is_null()
            );
            let mut trailing_dir = *b"some/very/long/\0";
            let mut full = *b"some/very/long/directory/file.txt\0";
            assert!(
                path_shorten_fname(full.as_mut_ptr().cast(), trailing_dir.as_mut_ptr().cast())
                    .is_null()
            );
            let shortened = path_shorten_fname(full.as_mut_ptr().cast(), dir.as_mut_ptr().cast());
            assert_eq!(crate::memory::cstr_as_bytes(shortened), b"directory/file.txt");

            let mut full_dup = *b"some/very/long///directory/file.txt\0";
            let shortened_dup =
                path_shorten_fname(full_dup.as_mut_ptr().cast(), dir.as_mut_ptr().cast());
            assert_eq!(
                crate::memory::cstr_as_bytes(shortened_dup),
                b"directory/file.txt"
            );

            assert!(path_try_shorten_fname(ptr::null_mut()).is_null());
            let mut cwd_buf = [0 as c_char; MAXPATHL];
            assert!(os_dirname(cwd_buf.as_mut_ptr(), MAXPATHL));
            let cwd_str = CStr::from_ptr(cwd_buf.as_ptr()).to_str().unwrap();
            let mut in_cwd = CString::new(format!("{cwd_str}/subdir/file.txt"))
                .unwrap()
                .into_bytes_with_nul();
            assert_eq!(
                crate::memory::cstr_as_bytes(path_try_shorten_fname(in_cwd.as_mut_ptr().cast())),
                b"subdir/file.txt"
            );

            let mut buf = [0 as c_char; 100];
            xstrlcpy(buf.as_mut_ptr(), c"path1".as_ptr(), buf.len());
            assert!(append_path(buf.as_mut_ptr(), c"path2".as_ptr(), 100));
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"path1/path2");

            xstrlcpy(buf.as_mut_ptr(), c"path1/".as_ptr(), buf.len());
            assert!(append_path(buf.as_mut_ptr(), c"path2".as_ptr(), 100));
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"path1/path2");
            assert!(!append_path(buf.as_mut_ptr(), c"path2".as_ptr(), 11));

            xstrlcpy(buf.as_mut_ptr(), c"path1".as_ptr(), buf.len());
            assert!(append_path(buf.as_mut_ptr(), c"".as_ptr(), 6));
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"path1");
            assert!(append_path(buf.as_mut_ptr(), c".".as_ptr(), 6));
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"path1");

            buf[0] = NUL;
            assert!(append_path(buf.as_mut_ptr(), c"/path2".as_ptr(), 7));
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"/path2");
        }
    }

    #[test]
    fn path_url_extension_and_is_absolute() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(path_is_absolute(c"/some/directory/".as_ptr()));
            #[cfg(windows)]
            assert!(path_is_absolute(c"C:/some/directory/".as_ptr()));
            #[cfg(not(windows))]
            assert!(path_is_absolute(c"~/in/my/home~/directory".as_ptr()));
            assert!(!path_is_absolute(c"not/in/my/home~/directory".as_ptr()));

            let saved_fic = opt_vars().p_fic;
            opt_vars().p_fic = 0;
            assert!(path_with_extension(
                c"/some/path/file.lua".as_ptr(),
                c"lua"
            ));
            assert!(!path_with_extension(
                c"/some/path/file.vim".as_ptr(),
                c"lua"
            ));
            assert!(!path_with_extension(
                c"/some/path/file".as_ptr(),
                c"lua"
            ));
            assert!(!path_with_extension(
                c"/some/path/file.LUA".as_ptr(),
                c"lua"
            ));
            opt_vars().p_fic = 1;
            assert!(path_with_extension(
                c"/some/path/file.LUA".as_ptr(),
                c"lua"
            ));
            opt_vars().p_fic = saved_fic;

            assert_eq!(path_with_url(c"test://xyz/foo/b0".as_ptr()), 1);
            assert_eq!(path_with_url(c"test:\\\\xyz\\foo\\b0".as_ptr()), 2);
            assert_eq!(path_with_url(c"test123://xyz/foo/b0".as_ptr()), 1);
            assert_eq!(path_with_url(c"test_abc://xyz/foo/b2".as_ptr()), 0);
            assert_eq!(path_with_url(c"test+abc-123.ghi://xyz/foo/b1".as_ptr()), 1);
            assert_eq!(path_with_url(c"-test://xyz/foo/b4".as_ptr()), 0);
            assert_eq!(path_with_url(c"test-://xyz/foo/b5".as_ptr()), 0);
            assert_eq!(path_with_url(c"test-C:/xyz/foo/b5".as_ptr()), 1);
            assert_eq!(path_with_url(c"c:/xyz/foo/b5".as_ptr()), 0);
            assert_eq!(path_with_url(c"C:/xyz/foo/b5".as_ptr()), 0);
    }

    #[test]
    fn path_equal_fullname_full_dir_name_and_guess_exepath() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            init_progpath();

            let mut cwd_buf = [0 as c_char; MAXPATHL];
            assert!(os_dirname(cwd_buf.as_mut_ptr(), MAXPATHL));
            let cwd = CStr::from_ptr(cwd_buf.as_ptr())
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

            assert!(path_equal(f1_c.as_ptr(), f1_c.as_ptr(), kPathCmpExpand));
            assert!(path_equal(f1_c.as_ptr(), f1_abs_c.as_ptr(), kPathCmpExpand));
            assert!(!path_equal(f1_c.as_ptr(), f2_c.as_ptr(), kPathCmpExpand));
            assert!(!path_equal(
                f1_c.as_ptr(),
                c"nonexistent_ut_file.txt".as_ptr(),
                kPathCmpExpand
            ));
            assert!(!path_equal(
                c"nonexistent_ut_file.txt".as_ptr(),
                c"nonexistent_ut_file.txt".as_ptr(),
                kPathCmpExpand
            ));
            assert!(path_equal(
                c"nonexistent_ut_file.txt".as_ptr(),
                c"nonexistent_ut_file.txt".as_ptr(),
                kPathCmpExpand | kPathCmpFull
            ));

            let saved_fic = opt_vars().p_fic;
            opt_vars().p_fic = 0;
            assert!(!path_equal(
                c"Foo".as_ptr(),
                c"foo".as_ptr(),
                kPathCmpLiteral
            ));
            opt_vars().p_fic = 1;
            assert!(path_equal(
                c"Foo".as_ptr(),
                c"foo".as_ptr(),
                kPathCmpLiteral
            ));
            opt_vars().p_fic = saved_fic;

            // path_full_dir_name
            let mut out = [0 as c_char; MAXPATHL];
            let mut empty = *b"\0";
            assert!(path_full_dir_name(empty.as_mut_ptr().cast(), out.as_mut_ptr(), MAXPATHL));
            assert_eq!(CStr::from_ptr(out.as_ptr()).to_str().unwrap(), cwd);

            let mut rel_c = CString::new(rel_dir.clone()).unwrap().into_bytes_with_nul();
            assert!(path_full_dir_name(rel_c.as_mut_ptr().cast(), out.as_mut_ptr(), MAXPATHL));
            assert_eq!(CStr::from_ptr(out.as_ptr()).to_str().unwrap(), abs_dir);

            let mut non_exist_abs = CString::new(format!("{abs_dir}/does_not_exist"))
                .unwrap()
                .into_bytes_with_nul();
            assert!(!path_full_dir_name(
                    non_exist_abs.as_mut_ptr().cast(),
                    out.as_mut_ptr(),
                    MAXPATHL
                ));

            // vim_FullName
            assert!(!vim_FullName(ptr::null(), out.as_mut_ptr(), 10, true));
            assert!(vim_FullName(
                    c"http://www.neovim.org".as_ptr(),
                    out.as_mut_ptr(),
                    MAXPATHL,
                    true
                ));
            assert_eq!(
                CStr::from_ptr(out.as_ptr()).to_str().unwrap(),
                "http://www.neovim.org"
            );

            let rel_dot = CString::new(format!("./{f1_rel}")).unwrap();
            assert!(vim_FullName(rel_dot.as_ptr(), out.as_mut_ptr(), MAXPATHL, true));
            assert_eq!(
                CStr::from_ptr(out.as_ptr()).to_str().unwrap(),
                format!("{abs_dir}/f1.o")
            );

            // path_fix_case
            let mut case_buf = CString::new(format!("{rel_dir}/camelcase"))
                .unwrap()
                .into_bytes_with_nul();
            path_fix_case(case_buf.as_mut_ptr().cast());
            #[cfg(any(windows, target_os = "macos"))]
            assert_eq!(
                CStr::from_ptr(case_buf.as_ptr().cast()).to_str().unwrap(),
                format!("{rel_dir}/CamelCase")
            );
            #[cfg(not(any(windows, target_os = "macos")))]
            assert_eq!(
                CStr::from_ptr(case_buf.as_ptr().cast()).to_str().unwrap(),
                format!("{rel_dir}/camelcase")
            );

            // path_guess_exepath
            path_guess_exepath(c"./nvim".as_ptr(), out.as_mut_ptr(), MAXPATHL);
            assert_eq!(
                CStr::from_ptr(out.as_ptr()).to_str().unwrap(),
                format!("{cwd}/./nvim")
            );

            path_guess_exepath(c"23u0293_not_in_path".as_ptr(), out.as_mut_ptr(), MAXPATHL);
            assert_eq!(
                CStr::from_ptr(out.as_ptr()).to_str().unwrap(),
                "23u0293_not_in_path"
            );

            let _ = std::fs::remove_dir_all(&abs_dir);
        }
    }
}
