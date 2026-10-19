//! Path manipulation, comparison, canonicalization, and wildcard expansion (`src/nvim/path.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_mut,
    unused_variables,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::globals::*;
use crate::option_vars::*;
use std::cmp::Ordering;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::charset::{backslash_halve, backslash_halve_save, rem_backslash, skipwhite};
use crate::cursor::{colnr_T, linenr_T};
use crate::eval::deprecated::buf_T;
use crate::eval::fs::FileInfo;
use crate::file_search::FileID;
use crate::garray::{
    ga_append_via_ptr, ga_clear_strings, ga_concat_strings, ga_init, ga_remove_duplicate_strings,
    garray_T,
};
use crate::lua::stdlib::{regmatch_T, regprog_T};
use crate::map_glyph_cache::NvimString;
use crate::memory::{
    xcalloc, xfree, xmalloc, xmemcpyz, xmemdupz, xrealloc, xstrdup, xstrlcat, xstrlcpy,
};
use crate::os::env::{expand_env_esc, expand_env_save_opt, os_getenv, vim_env_iter};
use crate::os::fs::{
    Directory, os_can_exe, os_closedir, os_dirname, os_file_is_readable, os_fileid,
    os_fileid_equal, os_fileinfo, os_fileinfo_id_equal, os_fileinfo_link, os_isdir, os_path_exists,
    os_realpath, os_scandir, os_scandir_next,
};
use crate::os::input::os_breakcheck;
use crate::os::shell::{get_cmd_output, os_expand_wildcards};

pub type PathCmpFlags = c_int;

const NUL: c_char = 0;
const OK: c_int = 1;
const FAIL: c_int = 0;

const MAXPATHL: usize = 4096;
const MAXSUFLEN: usize = 30;

const PATHSEP: c_char = b'/' as c_char;
const PATHSEPSTR: &[u8] = b"/\0";

#[cfg(windows)]
const ENV_SEPCHAR: c_char = b';' as c_char;
#[cfg(not(windows))]
const ENV_SEPCHAR: c_char = b':' as c_char;

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

const WILD_ADD_SLASH: c_int = 0x10;
const WILD_ICASE: c_int = 0x100;

const RE_MAGIC: c_int = 1;
const RE_STRING: c_int = 2;
const RE_NOBREAK: c_int = 16;

const kShellOptSilent: c_int = 0x08;

static mut SCANDIR_COUNT: c_int = 0;
static mut STARDEPTH: c_int = 0;
static mut EXPAND_RECURSIVE: bool = false;

#[cfg(not(test))]
unsafe extern "C" {

    #[cfg(windows)]
    fn _getdrive() -> c_int;
    fn tolower(c: c_int) -> c_int;

    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;
    fn utfc_ptr2len(p: *const c_char) -> c_int;
    fn utf_ptr2char(p: *const c_char) -> c_int;
    fn utf_head_off(base: *const c_char, p: *const c_char) -> c_int;
    fn utf_fold(a: c_int) -> c_int;
    fn mb_isalpha(a: c_int) -> bool;
    fn mb_strcmp_ic(ic: bool, s1: *const c_char, s2: *const c_char) -> c_int;

    fn file_pat_to_reg_pat(
        pat: *const c_char,
        pat_end: *const c_char,
        allow_dirs: *mut c_char,
        no_bslash: c_int,
    ) -> *mut c_char;
    fn vim_regcomp(expr_arg: *const c_char, re_flags: c_int) -> *mut regprog_T;
    fn vim_regexec(rmp: *mut regmatch_T, line: *const c_char, col: colnr_T) -> bool;
    fn vim_regfree(prog: *mut regprog_T);

    fn copy_option_part(
        option: *mut *mut c_char,
        buf: *mut c_char,
        maxlen: usize,
        sep_chars: *const c_char,
    ) -> usize;
    fn globpath(
        path: *mut c_char,
        file: *mut c_char,
        ga: *mut garray_T,
        expand_options: c_int,
        dirs: bool,
    );
    fn eval_to_string(arg: *const c_char, convert: bool, use_sandbox: bool) -> *mut c_char;
    fn eval_vars(
        src: *mut c_char,
        srcstart: *const c_char,
        usedlen: *mut usize,
        lnump: *mut linenr_T,
        errormsg: *mut *const c_char,
        escaped: *mut c_int,
        empty_is_error: bool,
    ) -> *mut c_char;
    fn concat_str(str1: *const c_char, str2: *const c_char) -> *mut c_char;
    fn match_file_list(list: *mut c_char, sfname: *mut c_char, ffname: *mut c_char) -> bool;
    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
}

#[cfg(test)]
unsafe fn vim_strchr(mut string: *const c_char, c: c_int) -> *mut c_char {
    unsafe {
        while *string != NUL {
            if *string as u8 as c_int == c {
                return string.cast_mut();
            }
            string = string.add(1);
        }
        ptr::null_mut()
    }
}

#[inline]
unsafe fn to_slash(p: *mut c_char) {
    #[cfg(windows)]
    unsafe {
        path_to_slash(p);
    }
    #[cfg(not(windows))]
    let _ = p;
}

#[inline]
unsafe fn to_slash_save(p: *const c_char) -> *mut c_char {
    #[cfg(windows)]
    unsafe {
        path_to_slash_save(p)
    }
    #[cfg(not(windows))]
    unsafe {
        xstrdup(p)
    }
}

#[inline]
unsafe fn strchrsub_in_place(mut buf: *mut c_char, c: c_char, x: c_char) {
    unsafe {
        while *buf != NUL {
            if *buf == c {
                *buf = x;
            }
            buf = buf.add(1);
        }
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn stricmp(mut s1: *const c_char, mut s2: *const c_char) -> c_int {
    unsafe {
        loop {
            let i = tolower(*s1 as u8 as c_int) - tolower(*s2 as u8 as c_int);
            if i != 0 {
                return i;
            }
            if *s1 == NUL {
                return 0;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
        }
    }
}

#[inline]
unsafe fn str_contains(haystack: *const c_char, needle: &[u8]) -> bool {
    unsafe {
        let bytes = CStr::from_ptr(haystack).to_bytes();
        bytes.windows(needle.len()).any(|w| w == needle)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_equal(
    s1: *const c_char,
    s2: *const c_char,
    flags: PathCmpFlags,
) -> bool {
    #[cfg(test)]
    {
        let _ = (s1, s2, flags);
        false
    }
    #[cfg(not(test))]
    unsafe {
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
                || CStr::from_ptr(s1).to_bytes().len() == CStr::from_ptr(s2).to_bytes().len())
                && path_cmp(p_fic != 0, s1, s2, MAXPATHL) == 0;
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
            return path_cmp(p_fic != 0, full1.as_ptr(), full2.as_ptr(), MAXPATHL) == 0;
        }
        false
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_tail(fname: *const c_char) -> *mut c_char {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_tail_with_sep(fname: *mut c_char) -> *mut c_char {
    unsafe {
        let past_head = get_past_head(fname);
        let mut tail = path_tail(fname);
        while tail > past_head && after_pathsep(fname, tail) != 0 {
            tail = tail.sub(1);
        }
        tail
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn invocation_path_tail(
    invocation: *const c_char,
    len: *mut usize,
) -> *const c_char {
    #[cfg(test)]
    {
        let _ = len;
        invocation
    }
    #[cfg(not(test))]
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

        if !len.is_null() {
            *len = tail_end.offset_from(tail) as usize;
        }

        tail
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_next_component(mut fname: *const c_char) -> *const c_char {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_skip_sep(mut path: *const c_char, colon: bool) -> *mut c_char {
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

#[unsafe(no_mangle)]
pub extern "C" fn path_head_length() -> c_int {
    #[cfg(windows)]
    {
        3
    }
    #[cfg(not(windows))]
    {
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn is_path_head(path: *const c_char) -> bool {
    #[cfg(windows)]
    unsafe {
        (*path as u8).is_ascii_alphabetic() && *path.add(1) == b':' as c_char
    }
    #[cfg(not(windows))]
    unsafe {
        vim_ispathsep(*path as c_int)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_past_head(path: *const c_char) -> *mut c_char {
    unsafe {
        let mut retval = path;
        #[cfg(windows)]
        if is_path_head(path) {
            retval = path.add(2);
        }
        path_skip_sep(retval, true)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn vim_ispathsep(c: c_int) -> bool {
    #[cfg(unix)]
    {
        c == b'/' as c_int
    }
    #[cfg(windows)]
    {
        c == b':' as c_int || c == b'/' as c_int || c == b'\\' as c_int
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn vim_ispathsep_nocolon(c: c_int) -> bool {
    #[cfg(windows)]
    {
        vim_ispathsep(c) && c != b':' as c_int
    }
    #[cfg(not(windows))]
    {
        vim_ispathsep(c)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn vim_ispathlistsep(c: c_int) -> bool {
    #[cfg(unix)]
    {
        c == b':' as c_int
    }
    #[cfg(not(unix))]
    {
        c == b';' as c_int
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shorten_dir_len(str_: *mut c_char, trim_len: c_int) {
    #[cfg(test)]
    {
        let _ = (str_, trim_len);
    }
    #[cfg(not(test))]
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shorten_dir(str_: *mut c_char) {
    unsafe {
        shorten_dir_len(str_, 1);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dir_of_file_exists(fname: *mut c_char) -> bool {
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
unsafe fn do_concat_fnames(
    fname1: *mut c_char,
    len1: usize,
    fname2: *const c_char,
    len2: usize,
    sep: bool,
    len: *mut usize,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn concat_fnames(
    fname1: NvimString,
    fname2: NvimString,
    sep: bool,
) -> NvimString {
    unsafe {
        let mut ret = NvimString {
            data: xmalloc(fname1.size + usize::from(sep) + fname2.size + 1).cast::<c_char>(),
            size: 0,
        };
        ptr::copy(fname1.data, ret.data, fname1.size + 1);
        do_concat_fnames(
            ret.data,
            fname1.size,
            fname2.data,
            fname2.size,
            sep,
            &mut ret.size,
        );
        ret
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn concat_fnames_realloc(
    fname1: *mut c_char,
    fname2: *const c_char,
    sep: bool,
) -> *mut c_char {
    unsafe {
        let len1 = CStr::from_ptr(fname1).to_bytes().len();
        let len2 = CStr::from_ptr(fname2).to_bytes().len();
        let mut len: usize = 0;
        let reallocated = xrealloc(fname1.cast::<c_void>(), len1 + len2 + 3).cast::<c_char>();
        do_concat_fnames(reallocated, len1, fname2, len2, sep, &mut len)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn add_pathsep(p: *mut c_char) -> bool {
    unsafe {
        if *p == NUL {
            return true;
        }

        let plen = CStr::from_ptr(p).to_bytes().len();
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FullName_save(fname: *const c_char, force: bool) -> *mut c_char {
    if fname.is_null() {
        return ptr::null_mut();
    }

    unsafe {
        let buf = xmalloc(MAXPATHL).cast::<c_char>();
        if vim_FullName(fname, buf, MAXPATHL, force) == FAIL {
            xfree(buf.cast::<c_void>());
            return to_slash_save(fname);
        }
        buf
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn save_abs_path(name: *const c_char) -> *mut c_char {
    unsafe {
        if !path_is_absolute(name) {
            return FullName_save(name, true);
        }
        to_slash_save(name)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_has_wildcard(mut p: *const c_char, all: bool) -> bool {
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

unsafe fn scandir_next_with_dots(dir: *mut Directory) -> *const c_char {
    unsafe {
        if dir.is_null() {
            SCANDIR_COUNT = 0;
            return ptr::null();
        }

        SCANDIR_COUNT += 1;
        if SCANDIR_COUNT == 1 || SCANDIR_COUNT == 2 {
            return if SCANDIR_COUNT == 1 {
                c".".as_ptr()
            } else {
                c"..".as_ptr()
            };
        }
        os_scandir_next(dir)
    }
}

#[cfg(not(test))]
unsafe fn path_expand(gap: *mut garray_T, path: *const c_char, flags: c_int) -> usize {
    unsafe { do_path_expand(gap, path, 0, flags, false) }
}

#[cfg(not(test))]
unsafe fn do_path_expand(
    gap: *mut garray_T,
    path: *const c_char,
    wildoff: usize,
    flags: c_int,
    didstar: bool,
) -> usize {
    unsafe {
        let start_len = (*gap).ga_len;
        let mut starstar = false;

        if STARDEPTH > 0 && (flags & EW_NOBREAK) == 0 {
            os_breakcheck();
            if got_int {
                return 0;
            }
        }

        let buflen = CStr::from_ptr(path).to_bytes().len() + MAXPATHL;
        let buf = xmalloc(buflen).cast::<c_char>();

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
                    p_fic == 0 && (flags & EW_ICASE) != 0 && mb_isalpha(utf_ptr2char(path_end));
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
                let move_len = CStr::from_ptr(p.add(1)).to_bytes().len() + 1;
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
        let pat = file_pat_to_reg_pat(s, e, ptr::null_mut(), 0);
        if pat.is_null() {
            xfree(buf.cast::<c_void>());
            return 0;
        }

        let mut regmatch = regmatch_T {
            regprog: ptr::null_mut(),
            startp: [ptr::null_mut(); 10],
            endp: [ptr::null_mut(); 10],
            rm_matchcol: 0,
            #[cfg(unix)]
            rm_ic: (flags & EW_ICASE) != 0 || p_fic != 0,
            #[cfg(not(unix))]
            rm_ic: true,
        };

        if (flags & (EW_NOERROR | EW_NOTWILD)) != 0 {
            emsg_silent += 1;
        }
        let nobreak = (flags & EW_NOBREAK) != 0;
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC | if nobreak { RE_NOBREAK } else { 0 });
        if (flags & (EW_NOERROR | EW_NOTWILD)) != 0 {
            emsg_silent -= 1;
        }
        xfree(pat.cast::<c_void>());

        if regmatch.regprog.is_null() && (flags & EW_NOTWILD) == 0 {
            xfree(buf.cast::<c_void>());
            return 0;
        }

        let mut len = s.offset_from(buf) as usize;
        if !didstar
            && STARDEPTH < 100
            && starstar
            && e.offset_from(s) == 2
            && *path_end == b'/' as c_char
        {
            vim_snprintf(s, buflen - len, c"%s".as_ptr(), path_end.add(1));
            STARDEPTH += 1;
            do_path_expand(gap, buf, len, flags, true);
            STARDEPTH -= 1;
        }
        *s = NUL;

        let mut dir: Directory = std::mem::zeroed();
        let dirpath = if *buf == NUL { c".".as_ptr() } else { buf };
        if os_file_is_readable(dirpath) && os_scandir(&mut dir, dirpath) {
            scandir_next_with_dots(ptr::null_mut());
            while !got_int {
                let name = scandir_next_with_dots(&mut dir);
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
                        && path_cmp(p_fic != 0, path.add(len), name, e.offset_from(s) as usize)
                            == 0);
                if dot_ok && reg_ok {
                    len += vim_snprintf(s, buflen - len, c"%s".as_ptr(), name) as usize;
                    if len + 1 >= buflen {
                        continue;
                    }

                    if starstar && STARDEPTH < 100 {
                        vim_snprintf(buf.add(len), buflen - len, c"/**%s".as_ptr(), path_end);
                        STARDEPTH += 1;
                        do_path_expand(gap, buf, len + 1, flags, true);
                        STARDEPTH -= 1;
                    }

                    vim_snprintf(buf.add(len), buflen - len, c"%s".as_ptr(), path_end);
                    if path_has_wildcard(path_end, false) {
                        if STARDEPTH < 100 {
                            STARDEPTH += 1;
                            do_path_expand(gap, buf, len + 1, flags, false);
                            STARDEPTH -= 1;
                        }
                    } else {
                        let mut file_info: FileInfo = std::mem::zeroed();
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

        xfree(buf.cast::<c_void>());
        vim_regfree(regmatch.regprog);

        let matches = ((*gap).ga_len - start_len) as usize;
        if matches > 0 && !got_int {
            let slice = std::slice::from_raw_parts_mut(
                ((*gap).ga_data.cast::<*mut c_char>()).add(start_len as usize),
                matches,
            );
            slice.sort_by(|&a, &b| {
                let cmp = path_cmp(p_fic != 0, a, b, MAXPATHL);
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

#[cfg(not(test))]
unsafe fn find_previous_pathsep(path: *mut c_char, psep: *mut *mut c_char) -> c_int {
    unsafe {
        if *psep > path && vim_ispathsep(**psep as c_int) {
            *psep = (*psep).sub(1);
        }

        while *psep > path {
            if vim_ispathsep(**psep as c_int) {
                return OK;
            }
            let off = utf_head_off(path, (*psep).sub(1)) + 1;
            *psep = (*psep).sub(off as usize);
        }

        FAIL
    }
}

#[cfg(not(test))]
unsafe fn is_unique(maybe_unique: *mut c_char, gap: *mut garray_T, i: c_int) -> bool {
    unsafe {
        let candidate_len = CStr::from_ptr(maybe_unique).to_bytes().len();
        let other_paths = (*gap).ga_data.cast::<*mut c_char>();

        for j in 0..(*gap).ga_len {
            if j == i {
                continue;
            }
            let other = *other_paths.add(j as usize);
            let other_path_len = CStr::from_ptr(other).to_bytes().len();
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

#[cfg(not(test))]
unsafe fn expand_path_option(
    curdir: *mut c_char,
    mut path_option: *mut c_char,
    gap: *mut garray_T,
) {
    unsafe {
        let buf = xmalloc(MAXPATHL).cast::<c_char>();
        let mut curdirlen: usize = 0;

        while *path_option != NUL {
            let mut buflen = copy_option_part(&mut path_option, buf, MAXPATHL, c" ,".as_ptr());

            if !vim_strchr(buf, b'`' as c_int).is_null() {
                continue;
            }

            if *buf == b'.' as c_char && (*buf.add(1) == NUL || vim_ispathsep(*buf.add(1) as c_int))
            {
                if (*curbuf).b_ffname.is_null() {
                    continue;
                }
                let p = path_tail((*curbuf).b_ffname);
                let plen = p.offset_from((*curbuf).b_ffname) as usize;
                if plen + CStr::from_ptr(buf).to_bytes().len() >= MAXPATHL {
                    continue;
                }
                if *buf.add(1) == NUL {
                    *buf.add(plen) = NUL;
                } else {
                    ptr::copy(buf.add(2), buf.add(plen), (buflen - 2) + 1);
                }
                ptr::copy((*curbuf).b_ffname, buf, plen);
                buflen = simplify_filename(buf);
            } else if *buf == NUL {
                let c_len = CStr::from_ptr(curdir).to_bytes().len();
                ptr::copy_nonoverlapping(curdir, buf, c_len + 1);
                if curdirlen == 0 {
                    curdirlen = c_len;
                }
                buflen = curdirlen;
            } else if path_with_url(buf) != 0 {
                continue;
            } else if !path_is_absolute(buf) {
                if curdirlen == 0 {
                    curdirlen = CStr::from_ptr(curdir).to_bytes().len();
                }
                if curdirlen + buflen + 3 > MAXPATHL {
                    continue;
                }
                ptr::copy(buf, buf.add(curdirlen + 1), buflen + 1);
                ptr::copy_nonoverlapping(curdir, buf, curdirlen);
                *buf.add(curdirlen) = PATHSEP;
                buflen = simplify_filename(buf);
            }

            let slot = ga_append_via_ptr(gap, size_of::<*mut c_char>()).cast::<*mut c_char>();
            *slot = xmemdupz(buf.cast::<c_void>(), buflen).cast::<c_char>();
        }

        xfree(buf.cast::<c_void>());
    }
}

#[cfg(not(test))]
unsafe fn get_path_cutoff(fname: *mut c_char, gap: *mut garray_T) -> *mut c_char {
    unsafe {
        let mut maxlen: c_int = 0;
        let path_part = (*gap).ga_data.cast::<*mut c_char>();
        let mut cutoff: *mut c_char = ptr::null_mut();

        for i in 0..(*gap).ga_len {
            let part = *path_part.add(i as usize);
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

#[cfg(not(test))]
unsafe fn uniquefy_paths(gap: *mut garray_T, pattern: *mut c_char, path_option: *mut c_char) {
    unsafe {
        let mut sort_again = false;
        let mut path_ga: garray_T = std::mem::zeroed();

        ga_remove_duplicate_strings(gap);
        ga_init(&mut path_ga, size_of::<*mut c_char>() as c_int, 1);

        let mut len = CStr::from_ptr(pattern).to_bytes().len();
        let file_pattern = xmalloc(len + 2).cast::<c_char>();
        *file_pattern = b'*' as c_char;
        ptr::copy_nonoverlapping(pattern, file_pattern.add(1), len + 1);
        let pat = file_pat_to_reg_pat(file_pattern, ptr::null(), ptr::null_mut(), 0);
        xfree(file_pattern.cast::<c_void>());
        if pat.is_null() {
            return;
        }

        let mut regmatch = regmatch_T {
            regprog: vim_regcomp(pat, RE_MAGIC + RE_STRING),
            startp: [ptr::null_mut(); 10],
            endp: [ptr::null_mut(); 10],
            rm_matchcol: 0,
            rm_ic: true,
        };
        xfree(pat.cast::<c_void>());
        if regmatch.regprog.is_null() {
            return;
        }

        let curdir = xmalloc(MAXPATHL).cast::<c_char>();
        os_dirname(curdir, MAXPATHL);
        expand_path_option(curdir, path_option, &mut path_ga);

        let fnames = (*gap).ga_data.cast::<*mut c_char>();
        let in_curdir =
            xcalloc((*gap).ga_len as usize, size_of::<*mut c_char>()).cast::<*mut c_char>();

        for i in 0..(*gap).ga_len {
            if got_int {
                break;
            }
            let path = *fnames.add(i as usize);
            let dir_end = gettail_dir(path);

            len = CStr::from_ptr(path).to_bytes().len();
            let dir_len = dir_end.offset_from(path) as usize;
            let is_in_curdir =
                path_cmp(p_fic != 0, curdir, path, dir_len) == 0 && *curdir.add(dir_len) == NUL;
            if is_in_curdir {
                *in_curdir.add(i as usize) = xmemdupz(path.cast::<c_void>(), len).cast::<c_char>();
            }

            let path_cutoff = get_path_cutoff(path, &mut path_ga);

            if *pattern == b'*' as c_char
                && *pattern.add(1) == b'*' as c_char
                && vim_ispathsep_nocolon(*pattern.add(2) as c_int)
                && !path_cutoff.is_null()
                && vim_regexec(&mut regmatch, path_cutoff, 0)
                && is_unique(path_cutoff, gap, i)
            {
                sort_again = true;
                let cutoff_len = CStr::from_ptr(path_cutoff).to_bytes().len();
                ptr::copy(path_cutoff, path, cutoff_len + 1);
            } else {
                let mut pathsep_p = path.add(len.saturating_sub(1));
                while find_previous_pathsep(path, &mut pathsep_p) != 0 {
                    let candidate = pathsep_p.add(1);
                    if vim_regexec(&mut regmatch, candidate, 0)
                        && is_unique(candidate, gap, i)
                        && !path_cutoff.is_null()
                        && candidate >= path_cutoff
                    {
                        sort_again = true;
                        let move_len = path.add(len).offset_from(candidate) as usize + 1;
                        ptr::copy(candidate, path, move_len);
                        break;
                    }
                }
            }

            if path_is_absolute(path) {
                let short_name = path_shorten_fname(path, curdir);
                if !short_name.is_null() && short_name > path.add(1) {
                    vim_snprintf(
                        path,
                        MAXPATHL,
                        c".%s%s".as_ptr(),
                        PATHSEPSTR.as_ptr().cast::<c_char>(),
                        short_name,
                    );
                }
            }
            os_breakcheck();
        }

        for i in 0..(*gap).ga_len {
            if got_int {
                break;
            }
            let path = *in_curdir.add(i as usize);
            if path.is_null() {
                continue;
            }

            let mut short_name = path_shorten_fname(path, curdir);
            if short_name.is_null() {
                short_name = path;
            }
            if is_unique(short_name, gap, i) {
                let slen = CStr::from_ptr(short_name).to_bytes().len();
                ptr::copy_nonoverlapping(short_name, *fnames.add(i as usize), slen + 1);
                continue;
            }

            let rel_pathsize =
                1 + (PATHSEPSTR.len() - 1) + CStr::from_ptr(short_name).to_bytes().len() + 1;
            let rel_path = xmalloc(rel_pathsize).cast::<c_char>();
            vim_snprintf(
                rel_path,
                rel_pathsize,
                c".%s%s".as_ptr(),
                PATHSEPSTR.as_ptr().cast::<c_char>(),
                short_name,
            );

            xfree((*fnames.add(i as usize)).cast::<c_void>());
            *fnames.add(i as usize) = rel_path;
            sort_again = true;
            os_breakcheck();
        }

        xfree(curdir.cast::<c_void>());
        for i in 0..(*gap).ga_len {
            xfree((*in_curdir.add(i as usize)).cast::<c_void>());
        }
        xfree(in_curdir.cast::<c_void>());
        ga_clear_strings(&mut path_ga);
        vim_regfree(regmatch.regprog);

        if sort_again {
            ga_remove_duplicate_strings(gap);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gettail_dir(fname: *const c_char) -> *const c_char {
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

#[cfg(not(test))]
unsafe fn expand_in_path(gap: *mut garray_T, pattern: *mut c_char, flags: c_int) -> c_int {
    unsafe {
        let mut path_ga: garray_T = std::mem::zeroed();
        let path_option = if *(*curbuf).b_p_path == NUL {
            p_path
        } else {
            (*curbuf).b_p_path
        };

        let curdir = xmalloc(MAXPATHL).cast::<c_char>();
        os_dirname(curdir, MAXPATHL);

        ga_init(&mut path_ga, size_of::<*mut c_char>() as c_int, 1);
        if (flags & EW_CDPATH) != 0 {
            expand_path_option(curdir, p_cdpath, &mut path_ga);
        } else {
            expand_path_option(curdir, path_option, &mut path_ga);
        }
        xfree(curdir.cast::<c_void>());
        if path_ga.ga_len == 0 {
            ga_clear_strings(&mut path_ga);
            return 0;
        }

        let paths = ga_concat_strings(&path_ga, c",".as_ptr());
        ga_clear_strings(&mut path_ga);

        let mut glob_flags = 0;
        if (flags & EW_ICASE) != 0 {
            glob_flags |= WILD_ICASE;
        }
        if (flags & EW_ADDSLASH) != 0 {
            glob_flags |= WILD_ADD_SLASH;
        }
        globpath(paths, pattern, gap, glob_flags, (flags & EW_CDPATH) != 0);
        xfree(paths.cast::<c_void>());

        (*gap).ga_len
    }
}

unsafe fn has_env_var(mut p: *const c_char) -> bool {
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
unsafe fn has_special_wildchar(mut p: *const c_char, flags: c_int) -> bool {
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
                    && vim_strchr(p, *p as u8 as c_int).is_null()
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gen_expand_wildcards(
    num_pat: c_int,
    pat: *mut *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
    flags: c_int,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (num_pat, pat, num_file, file, flags);
        FAIL
    }
    #[cfg(not(test))]
    unsafe {
        let mut ga: garray_T = std::mem::zeroed();
        let mut did_expand_in_path = false;
        let path_option = if *(*curbuf).b_p_path == NUL {
            p_path
        } else {
            (*curbuf).b_p_path
        };

        if EXPAND_RECURSIVE {
            #[cfg(unix)]
            {
                return os_expand_wildcards(num_pat, pat, num_file, file, flags);
            }
            #[cfg(not(unix))]
            {
                return FAIL;
            }
        }

        #[cfg(unix)]
        for i in 0..num_pat {
            let pi = *pat.add(i as usize);
            if has_special_wildchar(pi, flags)
                && !(vim_backtick(pi) && *pi.add(1) == b'=' as c_char)
            {
                return os_expand_wildcards(num_pat, pat, num_file, file, flags);
            }
        }

        EXPAND_RECURSIVE = true;
        ga_init(&mut ga, size_of::<*mut c_char>() as c_int, 30);

        for i in 0..num_pat {
            if got_int {
                break;
            }
            let mut add_pat: c_int = -1;
            let mut p = *pat.add(i as usize);

            if vim_backtick(p) {
                add_pat = expand_backtick(&mut ga, p, flags);
                if add_pat == -1 {
                    EXPAND_RECURSIVE = false;
                    ga_clear_strings(&mut ga);
                    *num_file = 0;
                    *file = ptr::null_mut();
                    return FAIL;
                }
            } else {
                if (has_env_var(p) && (flags & EW_NOTENV) == 0) || *p == b'~' as c_char {
                    p = expand_env_save_opt(p, true, PATH_ESC_WILDCARDS.cast_mut());
                    if p.is_null() {
                        p = *pat.add(i as usize);
                    } else {
                        #[cfg(unix)]
                        if has_env_var(p) || *p == b'~' as c_char {
                            xfree(p.cast::<c_void>());
                            ga_clear_strings(&mut ga);
                            let res = os_expand_wildcards(
                                num_pat,
                                pat,
                                num_file,
                                file,
                                flags | EW_KEEPDOLLAR,
                            );
                            EXPAND_RECURSIVE = false;
                            return res;
                        }
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
                        EXPAND_RECURSIVE = false;
                        add_pat = expand_in_path(&mut ga, p, flags);
                        EXPAND_RECURSIVE = true;
                        did_expand_in_path = true;
                    } else {
                        EXPAND_RECURSIVE = false;
                        let tmp_add_pat = path_expand(&mut ga, p, flags);
                        EXPAND_RECURSIVE = true;
                        debug_assert!(tmp_add_pat <= c_int::MAX as usize);
                        add_pat = tmp_add_pat as c_int;
                    }
                }
            }

            if add_pat == -1 || (add_pat == 0 && (flags & EW_NOTFOUND) != 0) {
                let t = backslash_halve_save(p);
                if (flags & EW_NOTFOUND) != 0 {
                    addfile(&mut ga, t, flags | EW_DIR | EW_FILE);
                } else {
                    addfile(&mut ga, t, flags);
                }
                if t != p {
                    xfree(t.cast::<c_void>());
                }
            }

            if did_expand_in_path && ga.ga_len > 0 && (flags & (EW_PATH | EW_CDPATH)) != 0 {
                EXPAND_RECURSIVE = false;
                uniquefy_paths(&mut ga, p, path_option);
                EXPAND_RECURSIVE = true;
            }
            if p != *pat.add(i as usize) {
                xfree(p.cast::<c_void>());
            }
        }

        *num_file = ga.ga_len;
        *file = if !ga.ga_data.is_null() {
            ga.ga_data.cast::<*mut c_char>()
        } else {
            ptr::null_mut()
        };

        EXPAND_RECURSIVE = false;

        if (flags & EW_EMPTYOK) != 0 || !ga.ga_data.is_null() {
            OK
        } else {
            FAIL
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FreeWild(mut count: c_int, files: *mut *mut c_char) {
    if count <= 0 || files.is_null() {
        return;
    }
    unsafe {
        while count > 0 {
            count -= 1;
            xfree((*files.add(count as usize)).cast::<c_void>());
        }
        xfree(files.cast::<c_void>());
    }
}

unsafe fn vim_backtick(p: *const c_char) -> bool {
    unsafe {
        if *p != b'`' as c_char || *p.add(1) == NUL {
            return false;
        }
        let len = CStr::from_ptr(p).to_bytes().len();
        *p.add(len - 1) == b'`' as c_char
    }
}

#[cfg(not(test))]
unsafe fn expand_backtick(gap: *mut garray_T, pat: *mut c_char, flags: c_int) -> c_int {
    unsafe {
        let mut cnt: c_int = 0;
        let pat_len = CStr::from_ptr(pat).to_bytes().len();
        let mut cmd = xmemdupz(pat.add(1).cast::<c_void>(), pat_len - 2).cast::<c_char>();

        let buffer = if *cmd == b'=' as c_char {
            eval_to_string(cmd.add(1), true, false)
        } else {
            get_cmd_output(
                cmd,
                ptr::null_mut(),
                if (flags & EW_SILENT) != 0 {
                    kShellOptSilent
                } else {
                    0
                },
                ptr::null_mut(),
            )
        };
        xfree(cmd.cast::<c_void>());
        if buffer.is_null() {
            return -1;
        }

        cmd = buffer;
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

        xfree(buffer.cast::<c_void>());
        cnt
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn slash_adjust(p: *mut c_char) {
    #[cfg(test)]
    {
        let _ = p;
    }
    #[cfg(not(test))]
    unsafe {
        if path_with_url(p) != 0 {
            return;
        }

        if *p == b'`' as c_char {
            let len = CStr::from_ptr(p).to_bytes().len();
            if len > 2 && *p.add(len - 1) == b'`' as c_char {
                return;
            }
        }

        let from = if p_ssl != 0 { b'\\' as c_char } else { PATHSEP };
        let to = if p_ssl != 0 { PATHSEP } else { b'\\' as c_char };
        strchrsub_in_place(p, from, to);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_to_backslash(p: *mut c_char) -> *mut c_char {
    if !p.is_null() {
        unsafe {
            strchrsub_in_place(p, PATHSEP, b'\\' as c_char);
        }
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_to_slash(p: *mut c_char) -> *mut c_char {
    if !p.is_null() {
        unsafe {
            if path_with_url(p) == 0 {
                strchrsub_in_place(p, b'\\' as c_char, PATHSEP);
            }
        }
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_to_slash_save(p: *const c_char) -> *mut c_char {
    if p.is_null() {
        ptr::null_mut()
    } else {
        unsafe { path_to_slash(xstrdup(p)) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn addfile(gap: *mut garray_T, f: *mut c_char, flags: c_int) {
    unsafe {
        let mut file_info: FileInfo = std::mem::zeroed();

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
            let bytes = CStr::from_ptr(f).to_bytes();
            if bytes.iter().any(|b| FNAME_ILLEGAL.contains(b)) {
                return;
            }
        }

        let isdir = os_isdir(f);
        if (isdir && (flags & EW_DIR) == 0) || (!isdir && (flags & EW_FILE) == 0) {
            return;
        }

        if !isdir
            && (flags & EW_EXEC) != 0
            && !os_can_exe(f, ptr::null_mut(), (flags & EW_SHELLCMD) == 0)
        {
            return;
        }

        let flen = CStr::from_ptr(f).to_bytes().len();
        let p = xmalloc(flen + 1 + usize::from(isdir)).cast::<c_char>();
        ptr::copy_nonoverlapping(f, p, flen + 1);
        to_slash(p);
        if isdir && (flags & EW_ADDSLASH) != 0 {
            add_pathsep(p);
        }
        let slot = ga_append_via_ptr(gap, size_of::<*mut c_char>()).cast::<*mut c_char>();
        *slot = p;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn simplify_filename(filename: *mut c_char) -> usize {
    #[cfg(test)]
    {
        unsafe { CStr::from_ptr(filename).to_bytes().len() }
    }
    #[cfg(not(test))]
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
        let mut start = p;
        let mut p_end = p.add(CStr::from_ptr(p).to_bytes().len());

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
                        let mut file_info: FileInfo = std::mem::zeroed();
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
                                let mut new_file_info: FileInfo = std::mem::zeroed();
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_has_drive_letter(p: *const c_char, path_len: usize) -> bool {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_is_url(p: *const c_char) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_with_url(fname: *const c_char) -> c_int {
    unsafe {
        if !(*fname as u8).is_ascii_alphabetic() {
            return 0;
        }

        if path_has_drive_letter(fname, CStr::from_ptr(fname).to_bytes().len()) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_with_extension(
    path: *const c_char,
    extension: *const c_char,
) -> bool {
    #[cfg(test)]
    {
        let _ = (path, extension);
        false
    }
    #[cfg(not(test))]
    unsafe {
        let bytes = CStr::from_ptr(path).to_bytes();
        let Some(pos) = bytes.iter().rposition(|&b| b == b'.') else {
            return false;
        };
        let last_dot = path.add(pos);
        mb_strcmp_ic(p_fic != 0, last_dot.add(1), extension) == 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_isAbsName(name: *const c_char) -> bool {
    unsafe { path_with_url(name) != 0 || path_is_absolute(name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_FullName(
    fname: *const c_char,
    buf: *mut c_char,
    len: usize,
    force: bool,
) -> c_int {
    unsafe {
        *buf = NUL;
        if fname.is_null() {
            return FAIL;
        }

        if CStr::from_ptr(fname).to_bytes().len() > len.saturating_sub(1) {
            xstrlcpy(buf, fname, len);
            to_slash(buf);
            return FAIL;
        }

        if path_with_url(fname) != 0 {
            xstrlcpy(buf, fname, len);
            return OK;
        }

        let rv = path_to_absolute(fname, buf, len, c_int::from(force));
        if rv == FAIL {
            xstrlcpy(buf, fname, len);
        }
        to_slash(buf);
        rv
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fix_fname(fname: *const c_char) -> *mut c_char {
    #[cfg(unix)]
    unsafe {
        FullName_save(fname, true)
    }
    #[cfg(not(unix))]
    unsafe {
        if !vim_isAbsName(fname)
            || str_contains(fname, b"..")
            || str_contains(fname, b"//")
            || str_contains(fname, b"\\\\")
            || *fname == b'/' as c_char
            || *fname == b'\\' as c_char
        {
            return FullName_save(fname, false);
        }

        let res = to_slash_save(fname);
        path_fix_case(res);
        res
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_fix_case(name: *mut c_char) {
    #[cfg(test)]
    {
        let _ = name;
    }
    #[cfg(not(test))]
    unsafe {
        let mut file_info: FileInfo = std::mem::zeroed();
        if !os_fileinfo_link(name, &mut file_info) {
            return;
        }

        let name_bytes = CStr::from_ptr(name).to_bytes();
        let slash_pos = name_bytes.iter().rposition(|&b| b == b'/');
        let mut dir: Directory = std::mem::zeroed();
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

        let taillen = CStr::from_ptr(tail).to_bytes().len();
        loop {
            let entry = os_scandir_next(&mut dir);
            if entry.is_null() {
                break;
            }
            if stricmp(tail, entry) == 0 && taillen == CStr::from_ptr(entry).to_bytes().len() {
                let mut newname = [0 as c_char; MAXPATHL + 1];
                let tail_off = tail.offset_from(name) as usize;
                xstrlcpy(newname.as_mut_ptr(), name, MAXPATHL + 1);
                xstrlcpy(
                    newname.as_mut_ptr().add(tail_off),
                    entry,
                    MAXPATHL - tail_off + 1,
                );
                let mut file_info_new: FileInfo = std::mem::zeroed();
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn after_pathsep(b: *const c_char, p: *const c_char) -> c_int {
    #[cfg(test)]
    {
        c_int::from(p > b && unsafe { vim_ispathsep(*p.sub(1) as c_int) })
    }
    #[cfg(not(test))]
    unsafe {
        c_int::from(p > b && vim_ispathsep(*p.sub(1) as c_int) && utf_head_off(b, p.sub(1)) == 0)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn same_directory(f1: *mut c_char, f2: *mut c_char) -> bool {
    #[cfg(test)]
    {
        let _ = (f1, f2);
        false
    }
    #[cfg(not(test))]
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
        d1 == d2 && path_cmp(p_fic != 0, ffname.as_ptr(), f2, d1 as usize) == 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_fold_char(ic: bool, p: *const c_char, len: *mut c_int) -> c_int {
    #[cfg(test)]
    {
        unsafe {
            *len = 1;
            *p as u8 as c_int
        }
    }
    #[cfg(not(test))]
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_cmp(
    ic: bool,
    mut p: *const c_char,
    mut q: *const c_char,
    maxlen: usize,
) -> c_int {
    unsafe {
        let mut c1: c_int = NUL as c_int;
        let mut c2: c_int = NUL as c_int;
        let mut len: usize = 0;

        #[cfg(all(windows, not(test)))]
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_try_shorten_fname(full_path: *mut c_char) -> *mut c_char {
    unsafe {
        let dirname = xmalloc(MAXPATHL).cast::<c_char>();
        let mut p = full_path;

        if os_dirname(dirname, MAXPATHL) == OK {
            p = path_shorten_fname(full_path, dirname);
            if p.is_null() || *p == NUL {
                p = full_path;
            }
        }
        xfree(dirname.cast::<c_void>());
        p
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_shorten_fname(
    full_path: *mut c_char,
    dir_name: *mut c_char,
) -> *mut c_char {
    if full_path.is_null() {
        return ptr::null_mut();
    }

    unsafe {
        debug_assert!(!dir_name.is_null());
        let len = CStr::from_ptr(dir_name).to_bytes().len();

        #[cfg(not(test))]
        let ic = p_fic != 0;
        #[cfg(test)]
        let ic = false;

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_wildcards_eval(
    pat: *mut *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
    flags: c_int,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (pat, num_file, file, flags);
        FAIL
    }
    #[cfg(not(test))]
    unsafe {
        let mut ret = FAIL;
        let mut eval_pat: *mut c_char = ptr::null_mut();
        let mut exp_pat = *pat;
        let mut ignored_msg: *const c_char = ptr::null();
        let mut usedlen: usize = 0;
        let is_cur_alt_file = *exp_pat == b'%' as c_char || *exp_pat == b'#' as c_char;
        let mut star_follows = false;

        if is_cur_alt_file || *exp_pat == b'<' as c_char {
            emsg_off += 1;
            eval_pat = eval_vars(
                exp_pat,
                exp_pat,
                &mut usedlen,
                ptr::null_mut(),
                &mut ignored_msg,
                ptr::null_mut(),
                true,
            );
            emsg_off -= 1;
            if !eval_pat.is_null() {
                star_follows = CStr::from_ptr(exp_pat.add(usedlen)).to_bytes() == b"*";
                exp_pat = concat_str(eval_pat, exp_pat.add(usedlen));
            }
        }

        if !exp_pat.is_null() {
            ret = expand_wildcards(1, &mut exp_pat, num_file, file, flags);
        }

        if !eval_pat.is_null() {
            if *num_file == 0 && is_cur_alt_file && star_follows {
                *file = xmalloc(size_of::<*mut c_char>()).cast::<*mut c_char>();
                **file = eval_pat;
                eval_pat = ptr::null_mut();
                *num_file = 1;
                ret = OK;
            }
            xfree(exp_pat.cast::<c_void>());
            xfree(eval_pat.cast::<c_void>());
        }

        ret
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_wildcards(
    num_pat: c_int,
    pat: *mut *mut c_char,
    num_files: *mut c_int,
    files: *mut *mut *mut c_char,
    flags: c_int,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (num_pat, pat, num_files, files, flags);
        FAIL
    }
    #[cfg(not(test))]
    unsafe {
        let retval = gen_expand_wildcards(num_pat, pat, num_files, files, flags);

        if (flags & EW_KEEPALL) != 0 || retval == FAIL {
            return retval;
        }

        if *p_wig != NUL {
            debug_assert!(*num_files == 0 || !(*files).is_null());
            let mut i: c_int = 0;
            while i < *num_files {
                let fi = *(*files).add(i as usize);
                let ffname = FullName_save(fi, false);
                debug_assert!(!fi.is_null());
                debug_assert!(!ffname.is_null());
                if match_file_list(p_wig, fi, ffname) {
                    xfree(fi.cast::<c_void>());
                    let mut j = i;
                    while j + 1 < *num_files {
                        *(*files).add(j as usize) = *(*files).add((j + 1) as usize);
                        j += 1;
                    }
                    *num_files -= 1;
                    i -= 1;
                }
                xfree(ffname.cast::<c_void>());
                i += 1;
            }
        }

        debug_assert!(*num_files == 0 || !(*files).is_null());
        if *num_files > 1 && !got_int {
            let mut non_suf_match: c_int = 0;
            for i in 0..*num_files {
                let fi = *(*files).add(i as usize);
                if !match_suffix(fi) {
                    let p = fi;
                    let mut j = i;
                    while j > non_suf_match {
                        *(*files).add(j as usize) = *(*files).add((j - 1) as usize);
                        j -= 1;
                    }
                    *(*files).add(non_suf_match as usize) = p;
                    non_suf_match += 1;
                }
            }
        }

        if *num_files == 0 {
            xfree((*files).cast::<c_void>());
            *files = ptr::null_mut();
            return FAIL;
        }

        retval
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn match_suffix(fname: *mut c_char) -> bool {
    #[cfg(test)]
    {
        let _ = fname;
        false
    }
    #[cfg(not(test))]
    unsafe {
        let mut suf_buf = [0 as c_char; MAXSUFLEN];
        let fnamelen = CStr::from_ptr(fname).to_bytes().len();
        let mut setsuflen: usize = 0;
        let mut setsuf = p_su;
        while *setsuf != NUL {
            setsuflen =
                copy_option_part(&mut setsuf, suf_buf.as_mut_ptr(), MAXSUFLEN, c".,".as_ptr());
            if setsuflen == 0 {
                let tail = path_tail(fname);
                if vim_strchr(tail, b'.' as c_int).is_null() {
                    setsuflen = 1;
                    break;
                }
            } else {
                if fnamelen >= setsuflen
                    && path_cmp(
                        p_fic != 0,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_full_dir_name(
    directory: *mut c_char,
    buffer: *mut c_char,
    len: usize,
) -> c_int {
    unsafe {
        if *directory == NUL {
            return os_dirname(buffer, len);
        }

        if !os_realpath(directory, buffer, len).is_null() {
            return OK;
        }

        if path_is_absolute(directory) {
            return FAIL;
        }

        let mut old_dir = [0 as c_char; MAXPATHL];
        if os_dirname(old_dir.as_mut_ptr(), MAXPATHL) == FAIL {
            return FAIL;
        }

        xstrlcpy(buffer, old_dir.as_ptr(), len);
        if append_path(buffer, directory, len) == FAIL {
            return FAIL;
        }

        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn append_path(
    path: *mut c_char,
    to_append: *const c_char,
    max_len: usize,
) -> c_int {
    unsafe {
        let mut current_length = CStr::from_ptr(path).to_bytes().len();
        let to_append_bytes = CStr::from_ptr(to_append).to_bytes();
        let to_append_length = to_append_bytes.len();

        if to_append_length == 0 || to_append_bytes == b"." {
            return OK;
        }

        if current_length > 0 && !vim_ispathsep_nocolon(*path.add(current_length - 1) as c_int) {
            let pathsep_len = PATHSEPSTR.len() - 1;
            if current_length + pathsep_len + 1 > max_len {
                return FAIL;
            }
            xstrlcpy(
                path.add(current_length),
                PATHSEPSTR.as_ptr().cast::<c_char>(),
                max_len - current_length,
            );
            current_length += pathsep_len;
        }

        if current_length + to_append_length + 1 > max_len {
            return FAIL;
        }

        xstrlcpy(
            path.add(current_length),
            to_append,
            max_len - current_length,
        );
        OK
    }
}

unsafe fn path_to_absolute(
    fname: *const c_char,
    buf: *mut c_char,
    len: usize,
    force: c_int,
) -> c_int {
    unsafe {
        *buf = NUL;
        let relative_directory = xmalloc(len).cast::<c_char>();
        let mut end_of_path = fname;

        #[cfg(windows)]
        let need_expand = force != 0
            || !path_is_absolute(fname)
            || *fname == b'/' as c_char
            || *fname == b'\\' as c_char;
        #[cfg(not(windows))]
        let need_expand = force != 0 || !path_is_absolute(fname);

        if need_expand {
            let fname_bytes = CStr::from_ptr(fname).to_bytes();
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
                if vim_ispathsep(*p as c_int) && CStr::from_ptr(p.add(1)).to_bytes() == b".." {
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

            if path_full_dir_name(relative_directory, buf, len) == FAIL {
                xfree(relative_directory.cast::<c_void>());
                return FAIL;
            }
        }
        xfree(relative_directory.cast::<c_void>());
        append_path(buf, end_of_path, len)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_is_absolute(fname: *const c_char) -> bool {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn path_guess_exepath(
    argv0: *const c_char,
    buf: *mut c_char,
    bufsize: usize,
) {
    #[cfg(test)]
    {
        let _ = (argv0, buf, bufsize);
    }
    #[cfg(not(test))]
    unsafe {
        let path = os_getenv(c"PATH".as_ptr());
        let argv0_bytes = CStr::from_ptr(argv0).to_bytes();

        if path.is_null() || path_is_absolute(argv0) {
            xstrlcpy(buf, argv0, bufsize);
        } else if *argv0 == b'.' as c_char || argv0_bytes.contains(&(PATHSEP as u8)) {
            if os_dirname(buf, MAXPATHL) != OK {
                *buf = NUL;
            }
            xstrlcat(buf, PATHSEPSTR.as_ptr().cast::<c_char>(), bufsize);
            xstrlcat(buf, argv0, bufsize);
        } else {
            let namebuff = (&raw mut NameBuff).cast::<c_char>();
            let mut iter: *const c_void = ptr::null();
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
                xmemcpyz(namebuff.cast::<c_void>(), dir.cast::<c_void>(), dir_len);
                xstrlcat(namebuff, PATHSEPSTR.as_ptr().cast::<c_char>(), MAXPATHL);
                xstrlcat(namebuff, argv0, MAXPATHL);
                if os_can_exe(namebuff, ptr::null_mut(), false) {
                    xstrlcpy(buf, namebuff, bufsize);
                    to_slash(buf);
                    xfree(path.cast::<c_void>());
                    return;
                }
                if iter.is_null() {
                    break;
                }
            }
            xstrlcpy(buf, argv0, bufsize);
        }
        xfree(path.cast::<c_void>());
    }
}
