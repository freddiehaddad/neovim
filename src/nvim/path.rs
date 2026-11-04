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
use crate::memory::{cstr_from_bytes, cstr_to_cstring};
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
use crate::strings::{RawCCharPtr, RawCCharPtrMut, sort_strings, vim_snprintf};
use std::cmp::Ordering;
use std::ffi::CStr;
use std::ptr;

pub fn remove_duplicate_strings(fnames: &mut Vec<std::ffi::CString>) {
    if fnames.is_empty() {
        return;
    }
    sort_strings(fnames);
    let mut i = fnames.len() - 1;
    while i > 0 {
        if path_equal(
            fnames[i - 1].as_ptr(),
            fnames[i].as_ptr(),
            K_PATH_CMP_LITERAL,
        ) {
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
    scandir_count: i32,
    stardepth: i32,
    expand_recursive: bool,
}

impl PathModuleState {
    const fn new() -> Self {
        Self {
            scandir_count: 0,
            stardepth: 0,
            expand_recursive: false,
        }
    }
}

static PATH_STATE: crate::globals::GlobalState<PathModuleState> =
    crate::globals::GlobalState::new(PathModuleState::new());

#[inline]
fn path_st() -> &'static mut PathModuleState {
    PATH_STATE.as_mut()
}

#[cfg(windows)]
use crate::fileio::_getdrive;

#[inline]
pub fn to_slash(p: *mut u8) {
    #[cfg(windows)]
    {
        path_to_slash(p);
    }
    #[cfg(not(windows))]
    let _ = p;
}

#[inline]
pub fn to_slash_save(p: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let p = p.into_c_char_ptr();
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

pub fn path_equal<S1: crate::strings::IntoCCharPtr, S2: crate::strings::IntoCCharPtr>(
    s1: S1,
    s2: S2,
    flags: PathCmpFlags,
) -> bool {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    {
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
            let is_url = path_with_url(s1) || path_with_url(s2);
            return (!is_url || crate::memory::cstr_len(s1) == crate::memory::cstr_len(s2))
                && path_cmp(opt_vars().p_fic != 0, s1, s2, MAXPATHL) == 0;
        }

        if (flags & K_PATH_CMP_EXPAND) != 0 {
            expand_env_esc(
                crate::memory::cstr_from_ptr(s1),
                &mut expanded_s1,
                None,
                false,
                None,
            );
        } else if flags != 0 {
            crate::memory::copy_bytes_to_c_buf(&mut expanded_s1, crate::memory::cstr_as_bytes(s1));
        }

        let id_ok1 = os_fileid(crate::memory::cstr_until_nul(&expanded_s1), &mut file_id1);
        let id_ok2 = os_fileid(crate::memory::cstr_from_ptr((s2).cast()), &mut file_id2);
        if id_ok1 && id_ok2 && os_fileid_equal(&file_id1, &file_id2) {
            return true;
        }
        if !id_ok1 && !id_ok2 && (flags & K_PATH_CMP_FULL) != 0 {
            vim_full_name(
                crate::memory::cstr_until_nul(&expanded_s1),
                full1.as_mut_ptr(),
                MAXPATHL,
                false,
            );
            vim_full_name(s2, full2.as_mut_ptr(), MAXPATHL, false);
            return path_cmp(
                opt_vars().p_fic != 0,
                crate::memory::cstr_until_nul(&full1),
                crate::memory::cstr_until_nul(&full2),
                MAXPATHL,
            ) == 0;
        }
        false
    }
}

pub fn path_tail(fname: impl crate::strings::IntoCCharPtr) -> RawCCharPtrMut {
    let fname = fname.into_c_char_ptr();
    if fname.is_null() {
        return c"".as_ptr().cast_mut();
    }

    let past_head = get_past_head(fname);
    let rem = crate::memory::cstr_as_bytes(past_head);
    let mut tail_off = 0usize;
    for (i, &b) in rem.iter().enumerate() {
        if vim_ispathsep_nocolon(b as i32) {
            tail_off = i + 1;
        }
    }
    past_head.wrapping_add(tail_off)
}

pub fn path_tail_with_sep(fname: impl crate::strings::IntoCCharPtr) -> RawCCharPtrMut {
    let fname = fname.into_c_char_ptr();
    {
        let past_head = get_past_head(fname);
        let mut tail = path_tail(fname);
        while tail > past_head && after_pathsep(fname, tail) != 0 {
            tail = tail.wrapping_sub(1);
        }
        tail
    }
}

pub fn invocation_path_tail(
    invocation: impl crate::strings::IntoCCharPtr,
    len: Option<&mut usize>,
) -> RawCCharPtr {
    let invocation = invocation.into_c_char_ptr();
    let base: RawCCharPtr = get_past_head(invocation);
    let bytes = crate::memory::cstr_as_bytes(base);
    let mut tail_idx = 0usize;
    let mut tail_end_idx = 0usize;
    let mut idx = 0usize;
    let mut inquote = false;
    while idx < bytes.len() && (inquote || bytes[idx] != b' ') {
        let l = utfc_ptr2len(crate::memory::cstr_cluster_bytes(
            (base.wrapping_add(idx)).cast(),
        )) as usize;
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

    base.wrapping_add(tail_idx)
}

pub fn path_next_component(fname: impl crate::strings::IntoCCharPtr) -> RawCCharPtr {
    let fname = fname.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(fname);
    let mut i = bytes
        .iter()
        .take_while(|&&b| !vim_ispathsep(b as i32))
        .count();
    if i < bytes.len() {
        i += 1;
    }
    fname.wrapping_add(i)
}

pub fn path_skip_sep(path: impl crate::strings::IntoCCharPtr, colon: bool) -> RawCCharPtrMut {
    let path = path.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(path);
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
    path.wrapping_add(skip).cast_mut()
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

pub fn get_past_head(path: impl crate::strings::IntoCCharPtr) -> RawCCharPtrMut {
    let path = path.into_c_char_ptr();
    #[cfg(windows)]
    let mut retval = path;
    #[cfg(not(windows))]
    let retval = path;
    #[cfg(windows)]
    if is_path_head(crate::memory::cstr_as_bytes(path)) {
        retval = path.wrapping_add(2);
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

pub fn shorten_dir_len(str_: *mut u8, trim_len: i32) {
    let tail = path_tail(str_.cast()).cast::<u8>();
    let mut d = str_;
    let mut skip = false;
    let mut dirchunk_len: i32 = 0;
    let mut s = str_;
    loop {
        if s >= tail {
            *crate::globals::gptr(d) = *crate::globals::gptr(s);

            d = d.wrapping_add(1);
            if crate::memory::cstr_is_empty(s.cast()) {
                break;
            }
        } else if vim_ispathsep(crate::memory::cstr_first_byte(s.cast())) {
            *crate::globals::gptr(d) = *crate::globals::gptr(s);

            d = d.wrapping_add(1);
            skip = false;
            dirchunk_len = 0;
        } else if !skip {
            *crate::globals::gptr(d) = *crate::globals::gptr(s);

            d = d.wrapping_add(1);
            if crate::memory::cstr_first_byte(s.cast()) != b'~'
                && crate::memory::cstr_first_byte(s.cast()) != b'.'
            {
                dirchunk_len += 1;
                if dirchunk_len >= trim_len {
                    skip = true;
                }
            }
            let mut l = utfc_ptr2len(crate::memory::cstr_cluster_bytes(s.cast()));
            while {
                l -= 1;
                l > 0
            } {
                s = s.wrapping_add(1);

                *crate::globals::gptr(d) = *crate::globals::gptr(s);

                d = d.wrapping_add(1);
            }
        }
        s = s.wrapping_add(1);
    }
}

pub fn shorten_dir<T>(str_: *mut T) {
    shorten_dir_len(str_.cast(), 1);
}

pub fn dir_of_file_exists(fname: *mut u8) -> bool {
    let p = path_tail_with_sep(fname.cast()).cast::<u8>();
    if p == fname {
        return true;
    }
    let c = *crate::globals::gptr(p);

    *crate::globals::gptr(p) = 0;

    let retval = os_isdir(crate::memory::cstr_from_ptr(fname.cast()));

    *crate::globals::gptr(p) = c;

    retval
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
    cstr_from_bytes(&buf)
}

pub fn concat_fnames(fname1: NvimString, fname2: NvimString, sep: bool) -> std::ffi::CString {
    do_concat_fnames(fname1.as_bytes(), fname2.as_bytes(), sep)
}

pub fn concat_fnames_cstr(
    fname1: impl crate::strings::IntoCCharPtr,
    fname2: impl crate::strings::IntoCCharPtr,
    sep: bool,
) -> std::ffi::CString {
    do_concat_fnames(
        crate::memory::cstr_as_bytes(fname1.into_c_char_ptr()),
        crate::memory::cstr_as_bytes(fname2.into_c_char_ptr()),
        sep,
    )
}

pub fn add_pathsep(p: *mut u8) -> bool {
    if crate::memory::cstr_is_empty(p.cast()) {
        return true;
    }

    let plen = crate::memory::cstr_len(p.cast());
    if after_pathsep(p.cast(), p.wrapping_add(plen).cast()) == 0 {
        let pathsep_len = PATHSEPSTR.len();
        if plen > MAXPATHL - pathsep_len {
            return false;
        }
        unsafe { ptr::copy_nonoverlapping(PATHSEPSTR.as_ptr(), p.wrapping_add(plen), pathsep_len) };
    }
    true
}

#[allow(non_snake_case)]
pub fn FullName_save(
    fname: impl crate::strings::IntoCCharPtr,
    force: bool,
) -> Option<std::ffi::CString> {
    let fname = fname.into_c_char_ptr();
    if fname.is_null() {
        return None;
    }

    let mut buf: Vec<u8> = vec![0; MAXPATHL];
    if !vim_full_name(fname, buf.as_mut_ptr(), MAXPATHL, force) {
        return to_slash_save(fname);
    }
    Some(crate::memory::cstring_until_nul(&buf))
}

pub fn save_abs_path(name: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let name = name.into_c_char_ptr();
    if !path_is_absolute(name) {
        return FullName_save(name, true);
    }
    to_slash_save(name)
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
        path_st().scandir_count = 0;
        return None;
    };

    path_st().scandir_count += 1;
    if path_st().scandir_count == 1 || path_st().scandir_count == 2 {
        return Some(if path_st().scandir_count == 1 {
            c"."
        } else {
            c".."
        });
    }
    os_scandir_next(dir)
}

fn path_expand(
    gap: &mut Vec<std::ffi::CString>,
    path: impl crate::strings::IntoCCharPtr,
    flags: i32,
) -> usize {
    let path = path.into_c_char_ptr();
    do_path_expand(gap, path, 0, flags, false)
}

fn do_path_expand(
    gap: &mut Vec<std::ffi::CString>,
    path: impl crate::strings::IntoCCharPtr,
    wildoff: usize,
    flags: i32,
    didstar: bool,
) -> usize {
    let path = path.into_c_char_ptr();
    {
        let start_len = gap.len();
        let mut starstar = false;

        if path_st().stardepth > 0 && (flags & EW_NOBREAK) == 0 {
            os_breakcheck();
            if g_st().got_int {
                return 0;
            }
        }

        let buflen = crate::memory::cstr_len(path) + MAXPATHL;
        let mut buf_vec = vec![0 as _; buflen];
        let buf = buf_vec.as_mut_ptr();

        let mut p = buf;
        let mut s = buf;
        let mut e: RawCCharPtrMut = ptr::null_mut();
        let mut path_end = path;
        let wild_start = path.wrapping_add(wildoff);

        while !crate::memory::cstr_is_empty(path_end) {
            if path_end >= wild_start && rem_backslash(crate::memory::cstr_as_bytes(path_end)) {
                *crate::globals::gptr(p) = *crate::globals::gptr(path_end);

                p = p.wrapping_add(1);
                path_end = path_end.wrapping_add(1);
            } else if vim_ispathsep_nocolon(crate::memory::cstr_first_byte(path_end)) {
                if !e.is_null() {
                    break;
                }
                s = p.wrapping_add(1);
            } else if path_end >= wild_start && {
                let is_esc_wild = crate::strings::vim_haschr(
                    PATH_ESC_WILDCARDS,
                    crate::memory::cstr_first_byte(path_end) as i32,
                );
                #[cfg(not(windows))]
                let is_icase_alpha = opt_vars().p_fic == 0
                    && (flags & EW_ICASE) != 0
                    && mb_isalpha(crate::mbyte::utf_ptr2char(crate::memory::cstr_char_bytes(
                        (path_end).cast(),
                    )));
                #[cfg(windows)]
                let is_icase_alpha = false;
                is_esc_wild || is_icase_alpha
            } {
                e = p;
            }
            let charlen =
                utfc_ptr2len(crate::memory::cstr_cluster_bytes((path_end).cast())) as usize;
            unsafe { ptr::copy_nonoverlapping(path_end, p, charlen) };
            p = p.wrapping_add(charlen);
            path_end = path_end.wrapping_add(charlen);
        }
        e = p;

        *crate::globals::gptr(e) = NUL;

        p = buf.wrapping_add(wildoff);
        while p < s {
            if rem_backslash(crate::memory::cstr_as_bytes(p)) {
                let move_len = crate::memory::cstr_len(p.wrapping_add(1)) + 1;
                unsafe { ptr::copy(p.wrapping_add(1), p, move_len) };
                e = e.wrapping_sub(1);
                s = s.wrapping_sub(1);
            }
            p = p.wrapping_add(1);
        }

        p = s;
        while p < e {
            if crate::memory::cstr_as_bytes(p).starts_with(b"**") {
                starstar = true;
            }
            p = p.wrapping_add(1);
        }

        let starts_with_dot = crate::memory::cstr_first_byte(s) == b'.';
        let Some(pat) = file_pat_to_reg_pat(
            crate::memory::cstr_slice(s, crate::memory::ptr_offset_from(e, s).max(0) as usize),
            None,
            0,
        ) else {
            return 0;
        };

        let mut regmatch = Regmatch {
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

        let mut len = crate::memory::ptr_offset_from(s, buf) as usize;
        if !didstar
            && path_st().stardepth < 100
            && starstar
            && crate::memory::ptr_offset_from(e, s) == 2
            && crate::memory::cstr_first_byte(path_end) == b'/'
        {
            vim_snprintf!(s, buflen - len, c"%s", path_end.wrapping_add(1));
            path_st().stardepth += 1;
            do_path_expand(gap, buf, len, flags, true);
            path_st().stardepth -= 1;
        }

        *crate::globals::gptr(s) = NUL;

        let mut dir = Directory::new();
        let dirpath = if crate::memory::cstr_is_empty(buf) {
            c".".as_ptr()
        } else {
            buf
        };
        if os_file_is_readable(crate::memory::cstr_from_ptr((dirpath).cast()))
            && os_scandir(&mut dir, crate::memory::cstr_from_ptr((dirpath).cast()))
        {
            scandir_next_with_dots(None);
            while !g_st().got_int {
                let Some(name) = scandir_next_with_dots(Some(&mut dir)) else {
                    break;
                };
                let name_bytes = name.to_bytes();
                len = crate::memory::ptr_offset_from(s, buf) as usize;
                let dot_ok = name_bytes.first() != Some(&b'.')
                    || starts_with_dot
                    || ((flags & EW_DODOT) != 0 && name_bytes != b"." && name_bytes != b"..");
                let reg_ok = (!regmatch.regprog.is_null() && vim_regexec(&mut regmatch, name, 0))
                    || ((flags & EW_NOTWILD) != 0
                        && path_cmp(
                            opt_vars().p_fic != 0,
                            path.wrapping_add(len),
                            name,
                            crate::memory::ptr_offset_from(e, s) as usize,
                        ) == 0);
                if dot_ok && reg_ok {
                    len += vim_snprintf!(s, buflen - len, c"%s", name) as usize;
                    if len + 1 >= buflen {
                        continue;
                    }

                    if starstar && path_st().stardepth < 100 {
                        vim_snprintf!(buf.wrapping_add(len), buflen - len, c"/**%s", path_end);
                        path_st().stardepth += 1;
                        do_path_expand(gap, buf, len + 1, flags, true);
                        path_st().stardepth -= 1;
                    }

                    vim_snprintf!(buf.wrapping_add(len), buflen - len, c"%s", path_end);
                    if path_has_wildcard(crate::memory::cstr_as_bytes(path_end), false) {
                        if path_st().stardepth < 100 {
                            path_st().stardepth += 1;
                            do_path_expand(gap, buf, len + 1, flags, false);
                            path_st().stardepth -= 1;
                        }
                    } else {
                        let mut file_info: FileInfo = FileInfo::new();
                        if !crate::memory::cstr_is_empty(path_end) {
                            let tail_ptr = buf.wrapping_add(len + 1);
                            let tail_len = crate::memory::cstr_len(tail_ptr.cast());
                            backslash_halve(unsafe {
                                std::slice::from_raw_parts_mut(tail_ptr.cast::<u8>(), tail_len + 1)
                            });
                        }
                        if if (flags & EW_ALLLINKS) != 0 {
                            os_fileinfo_link(
                                crate::memory::cstr_from_ptr((buf).cast()),
                                &mut file_info,
                            )
                        } else {
                            os_path_exists(crate::memory::cstr_from_ptr((buf).cast()))
                        } {
                            addfile(gap, buf, flags);
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
    let candidate_cstr = cstr_from_bytes(maybe_unique);

    for (j, other_cstr) in gap.iter().enumerate() {
        if j == i {
            continue;
        }
        let other = other_cstr.as_ptr();
        let other_bytes = other_cstr.as_bytes();
        if other_bytes.len() < candidate_len {
            continue;
        }
        let offset = other_bytes.len() - candidate_len;
        let rival = other.wrapping_add(offset);
        if path_equal(candidate_cstr.as_ptr(), rival, K_PATH_CMP_LITERAL)
            && (offset == 0 || vim_ispathsep(i32::from(other_bytes[offset - 1])))
        {
            return false;
        }
    }
    true
}

fn expand_path_option(
    curdir: impl crate::strings::IntoCCharPtr,
    path_option: impl crate::strings::IntoCCharPtr,
    gap: &mut Vec<std::ffi::CString>,
) {
    let path_option = path_option.into_c_char_ptr().cast_mut();
    let curdir = curdir.into_c_char_ptr();
    {
        let mut buf_arr = [0u8; MAXPATHL];
        let mut curdirlen: usize = 0;
        let mut path_bytes = crate::memory::cstr_as_bytes(path_option);

        while !path_bytes.is_empty() {
            let mut buflen = copy_option_part(&mut path_bytes, &mut buf_arr[..MAXPATHL], c" ,");
            let buf = buf_arr.as_mut_ptr();

            if crate::strings::vim_haschr(buf.cast(), b'`' as i32) {
                continue;
            }

            let buf_bytes = crate::memory::cstr_as_bytes(buf.cast());
            if buf_bytes.first() == Some(&b'.')
                && buf_bytes
                    .get(1)
                    .is_none_or(|&b| vim_ispathsep(i32::from(b)))
            {
                if curbuf_mut().b_ffname.is_null() {
                    continue;
                }
                let p = path_tail(curbuf_mut().b_ffname);
                let plen = crate::memory::ptr_offset_from(p, curbuf_mut().b_ffname) as usize;
                if plen + crate::memory::cstr_len(buf.cast()) >= MAXPATHL {
                    continue;
                }
                if buf_bytes.len() == 1 {
                    *crate::globals::gptr(buf.wrapping_add(plen)) = 0;
                } else {
                    unsafe {
                        ptr::copy(
                            buf.wrapping_add(2),
                            buf.wrapping_add(plen),
                            (buflen - 2) + 1,
                        )
                    };
                }
                unsafe { ptr::copy(curbuf_mut().b_ffname.cast::<u8>(), buf, plen) };
                buflen = simplify_filename(buf);
            } else if crate::memory::cstr_is_empty(buf.cast()) {
                let c_len = crate::memory::cstr_len(curdir);
                unsafe { ptr::copy_nonoverlapping(curdir.cast::<u8>(), buf, c_len + 1) };
                if curdirlen == 0 {
                    curdirlen = c_len;
                }
                buflen = curdirlen;
            } else if path_with_url(buf.cast()) {
                continue;
            } else if !path_is_absolute(buf.cast()) {
                if curdirlen == 0 {
                    curdirlen = crate::memory::cstr_len(curdir);
                }
                if curdirlen + buflen + 3 > MAXPATHL {
                    continue;
                }
                unsafe { ptr::copy(buf, buf.wrapping_add(curdirlen + 1), buflen + 1) };
                unsafe { ptr::copy_nonoverlapping(curdir.cast::<u8>(), buf, curdirlen) };

                *crate::globals::gptr(buf.wrapping_add(curdirlen)) = PATHSEP;

                buflen = simplify_filename(buf);
            }

            gap.push(std::ffi::CString::new(&buf_arr[..buflen]).unwrap_or_default());
        }
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

fn uniquefy_paths(
    gap: &mut Vec<std::ffi::CString>,
    pattern: impl crate::strings::IntoCCharPtr,
    path_option: impl crate::strings::IntoCCharPtr,
) {
    let path_option = path_option.into_c_char_ptr().cast_mut();
    let pattern = pattern.into_c_char_ptr();
    {
        let mut sort_again = false;
        let mut path_ga: Vec<std::ffi::CString> = Vec::new();

        remove_duplicate_strings(gap);

        let pat_slice = crate::memory::cstr_as_bytes(pattern);
        let mut file_pattern = Vec::with_capacity(pat_slice.len() + 1);
        file_pattern.push(b'*');
        file_pattern.extend_from_slice(pat_slice);
        let Some(pat) = file_pat_to_reg_pat(&file_pattern, None, 0) else {
            return;
        };
        drop(file_pattern);

        let mut regmatch = Regmatch {
            regprog: ptr::null_mut(),
            startp: [ptr::null_mut(); 10],
            endp: [ptr::null_mut(); 10],
            rm_matchcol: 0,
            rm_ic: true,
        };
        regmatch.set_prog(vim_regcomp(&pat, RE_MAGIC + RE_STRING));
        drop(pat);
        if regmatch.regprog.is_null() {
            return;
        }

        let mut curdir_arr = [0u8; MAXPATHL];
        os_dirname(&mut curdir_arr);
        let curdir = curdir_arr.as_mut_ptr();
        let curdir_cstr = crate::memory::cstr_until_nul(&curdir_arr);
        expand_path_option(curdir_cstr, path_option, &mut path_ga);

        let mut in_curdir: Vec<Option<std::ffi::CString>> = vec![None; gap.len()];

        for i in 0..gap.len() {
            if g_st().got_int {
                break;
            }
            let path = gap[i].as_ptr().cast_mut();
            let path_bytes = gap[i].as_bytes();
            let dir_end = gettail_dir(path);

            let dir_len = crate::memory::ptr_offset_from(dir_end, path) as usize;
            let is_in_curdir = path_cmp(opt_vars().p_fic != 0, curdir_cstr, path, dir_len) == 0
                && curdir_arr[dir_len] == 0;
            if is_in_curdir {
                in_curdir[i] = Some(gap[i].clone());
            }

            let cutoff_idx = get_path_cutoff(path_bytes, &path_ga);

            let pat_bytes = crate::memory::cstr_as_bytes(pattern);
            if pat_bytes.starts_with(b"**")
                && pat_bytes
                    .get(2)
                    .is_some_and(|&b| vim_ispathsep_nocolon(i32::from(b)))
                && let Some(c_idx) = cutoff_idx
                && vim_regexec(
                    &mut regmatch,
                    unsafe { CStr::from_ptr(path.wrapping_add(c_idx)) },
                    0,
                )
                && is_unique(&path_bytes[c_idx..], gap, i)
            {
                sort_again = true;
                gap[i] = cstr_from_bytes(&path_bytes[c_idx..]);
            } else {
                let mut pathsep_idx = path_bytes.len().saturating_sub(1);
                while find_previous_pathsep(path_bytes, &mut pathsep_idx) {
                    let cand_idx = pathsep_idx + 1;
                    let candidate = unsafe { CStr::from_ptr(path.wrapping_add(cand_idx)) };
                    if vim_regexec(&mut regmatch, candidate, 0)
                        && is_unique(&path_bytes[cand_idx..], gap, i)
                        && cutoff_idx.is_some_and(|c_idx| cand_idx >= c_idx)
                    {
                        sort_again = true;
                        gap[i] = cstr_from_bytes(&path_bytes[cand_idx..]);
                        break;
                    }
                }
            }

            let path = gap[i].as_ptr().cast_mut();
            if path_is_absolute(path) {
                let short_name = path_shorten_fname(path, curdir);
                if !short_name.is_null() && short_name > path.wrapping_add(1) {
                    let rel_pathsize =
                        1 + (PATHSEPSTR.len() - 1) + crate::memory::cstr_len(short_name) + 1;
                    let mut rel_vec = vec![0u8; rel_pathsize];
                    vim_snprintf!(
                        rel_vec.as_mut_ptr().cast(),
                        rel_pathsize,
                        c".%s%s",
                        PATHSEPSTR.as_ptr().cast::<RawCCharPtr>(),
                        short_name,
                    );
                    gap[i] = crate::memory::cstring_until_nul(&rel_vec);
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
            let short_bytes = crate::memory::cstr_as_bytes(short_name);
            if is_unique(short_bytes, gap, i) {
                gap[i] = cstr_from_bytes(short_bytes);
                continue;
            }

            let rel_pathsize = 1 + (PATHSEPSTR.len() - 1) + crate::memory::cstr_len(short_name) + 1;
            let mut rel_vec = vec![0u8; rel_pathsize];
            vim_snprintf!(
                rel_vec.as_mut_ptr().cast(),
                rel_pathsize,
                c".%s%s",
                PATHSEPSTR.as_ptr().cast::<RawCCharPtr>(),
                short_name,
            );

            gap[i] = crate::memory::cstring_until_nul(&rel_vec);
            sort_again = true;
            os_breakcheck();
        }

        regmatch.free_prog();

        if sort_again {
            remove_duplicate_strings(gap);
        }
    }
}

pub fn gettail_dir(fname: impl crate::strings::IntoCCharPtr) -> RawCCharPtr {
    let fname = fname.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(fname);
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
    fname.wrapping_add(dir_end)
}

fn expand_in_path(
    gap: &mut Vec<std::ffi::CString>,
    pattern: impl crate::strings::IntoCCharPtr,
    flags: i32,
) -> i32 {
    let pattern = pattern.into_c_char_ptr();
    {
        let mut path_ga: Vec<std::ffi::CString> = Vec::new();
        let path_option = if crate::memory::cstr_is_empty(curbuf_mut().b_p_path) {
            opt_vars().p_path
        } else {
            curbuf_mut().b_p_path
        };

        let mut curdir_arr = [0u8; MAXPATHL];
        os_dirname(&mut curdir_arr);
        let curdir_cstr = crate::memory::cstr_until_nul(&curdir_arr);

        if (flags & EW_CDPATH) != 0 {
            expand_path_option(curdir_cstr, opt_vars().p_cdpath, &mut path_ga);
        } else {
            expand_path_option(curdir_cstr, path_option, &mut path_ga);
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
            crate::memory::cstr_from_ptr(pattern),
            gap,
            glob_flags,
            (flags & EW_CDPATH) != 0,
        );

        gap.len() as i32
    }
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
    num_pat: i32,
    pat: *mut *mut u8,
    file: &mut Vec<std::ffi::CString>,
    flags: i32,
) -> bool {
    file.clear();
    let mut did_expand_in_path = false;
    let path_option = if crate::memory::cstr_is_empty(curbuf_mut().b_p_path) {
        opt_vars().p_path
    } else {
        curbuf_mut().b_p_path
    };

    if path_st().expand_recursive {
        #[cfg(unix)]
        {
            let pat_slice: Vec<&CStr> = (0..num_pat as usize)
                .map(|i| crate::memory::cstr_from_ptr(unsafe { (*pat.add(i)).cast() }))
                .collect();
            return os_expand_wildcards(&pat_slice, file, flags);
        }
        #[cfg(not(unix))]
        {
            return false;
        }
    }

    #[cfg(unix)]
    for i in 0..num_pat {
        let pi = (*crate::globals::gptr(pat.wrapping_add(i as usize)))
            .cast_const()
            .cast();
        let pi_bytes = crate::memory::cstr_as_bytes(pi);
        if has_special_wildchar(pi_bytes, flags)
            && !(vim_backtick(pi_bytes) && pi_bytes.get(1) == Some(&b'='))
        {
            let pat_slice: Vec<&CStr> = (0..num_pat as usize)
                .map(|j| crate::memory::cstr_from_ptr(unsafe { (*pat.add(j)).cast() }))
                .collect();
            return os_expand_wildcards(&pat_slice, file, flags);
        }
    }

    path_st().expand_recursive = true;

    for i in 0..num_pat {
        if g_st().got_int {
            break;
        }
        let mut add_pat: i32 = -1;
        let mut p: RawCCharPtrMut = crate::globals::gptr(pat.wrapping_add(i as usize)).cast();
        let mut p_owned: Option<std::ffi::CString> = None;
        let p_bytes = crate::memory::cstr_as_bytes(p);

        if vim_backtick(p_bytes) {
            add_pat = expand_backtick(file, p_bytes, flags);
            if add_pat == -1 {
                path_st().expand_recursive = false;
                file.clear();
                return false;
            }
        } else {
            if (has_env_var(p_bytes) && (flags & EW_NOTENV) == 0) || p_bytes.first() == Some(&b'~')
            {
                let po = expand_env_save_opt(
                    crate::memory::cstr_from_ptr(p),
                    true,
                    Some(PATH_ESC_WILDCARDS),
                );
                p = po.as_ptr().cast_mut();
                p_owned = Some(po);
                #[cfg(unix)]
                {
                    let po_bytes = crate::memory::cstr_as_bytes(p);
                    if has_env_var(po_bytes) || po_bytes.first() == Some(&b'~') {
                        drop(p_owned);
                        file.clear();
                        let pat_slice: Vec<&CStr> = (0..num_pat as usize)
                            .map(|j| crate::memory::cstr_from_ptr(unsafe { (*pat.add(j)).cast() }))
                            .collect();
                        let res = os_expand_wildcards(&pat_slice, file, flags | EW_KEEPDOLLAR);
                        path_st().expand_recursive = false;
                        return res;
                    }
                }
            }

            if path_has_wildcard(crate::memory::cstr_as_bytes(p), false) || (flags & EW_ICASE) != 0
            {
                if (flags & (EW_PATH | EW_CDPATH)) != 0
                    && !path_is_absolute(p)
                    && !{
                        let p_bytes = crate::memory::cstr_as_bytes(p);
                        p_bytes.first() == Some(&b'.')
                            && (p_bytes.get(1).is_some_and(|&b| vim_ispathsep(i32::from(b)))
                                || (p_bytes.get(1) == Some(&b'.')
                                    && p_bytes
                                        .get(2)
                                        .is_some_and(|&b| vim_ispathsep(i32::from(b)))))
                    }
                {
                    path_st().expand_recursive = false;
                    add_pat = expand_in_path(file, p, flags);
                    path_st().expand_recursive = true;
                    did_expand_in_path = true;
                } else {
                    path_st().expand_recursive = false;
                    let tmp_add_pat = path_expand(file, p, flags);
                    path_st().expand_recursive = true;
                    debug_assert!(tmp_add_pat <= i32::MAX as usize);
                    add_pat = tmp_add_pat as i32;
                }
            }
        }

        if add_pat == -1 || (add_pat == 0 && (flags & EW_NOTFOUND) != 0) {
            let t_owned = backslash_halve_save(crate::memory::cstr_as_bytes(p));
            let t = t_owned.as_ptr().cast_mut();
            if (flags & EW_NOTFOUND) != 0 {
                addfile(file, t, flags | EW_DIR | EW_FILE);
            } else {
                addfile(file, t, flags);
            }
        }

        if did_expand_in_path && !file.is_empty() && (flags & (EW_PATH | EW_CDPATH)) != 0 {
            path_st().expand_recursive = false;
            uniquefy_paths(file, p, path_option);
            path_st().expand_recursive = true;
        }
        drop(p_owned);
    }

    path_st().expand_recursive = false;

    (flags & EW_EMPTYOK) != 0 || !file.is_empty()
}

pub fn expand_wildcards_vec(
    num_pat: i32,
    pat: *mut *mut u8,
    files: &mut Vec<std::ffi::CString>,
    flags: i32,
) -> bool {
    let retval = gen_expand_wildcards_vec(num_pat, pat, files, flags);

    if (flags & EW_KEEPALL) != 0 || !retval {
        return retval;
    }

    if !crate::memory::cstr_is_empty(opt_vars().p_wig) {
        files.retain(|fi_cstr| {
            let ffname = FullName_save(fi_cstr.as_ptr(), false).unwrap_or_default();
            !match_file_list(
                crate::memory::cstr_from_ptr(opt_vars().p_wig),
                Some(fi_cstr),
                &ffname,
            )
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

fn vim_backtick(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes.first() == Some(&b'`') && bytes.last() == Some(&b'`')
}

fn expand_backtick(gap: &mut Vec<std::ffi::CString>, pat_bytes: &[u8], flags: i32) -> i32 {
    let mut cnt: i32 = 0;
    let mut cmd_vec: Vec<u8> = pat_bytes[1..pat_bytes.len() - 1].to_vec();
    cmd_vec.push(0);

    let cmd_out: Option<std::ffi::CString> = if cmd_vec[0] == b'=' {
        eval_to_string(crate::memory::cstr_until_nul(&cmd_vec[1..]), true, false)
    } else {
        get_cmd_output(
            Some(crate::memory::cstr_until_nul(&cmd_vec)),
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
            addfile(gap, cmd_buf.as_mut_ptr().wrapping_add(idx).cast(), flags);
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
pub fn slash_adjust(p: *mut u8) {
    if path_with_url(p.cast()) {
        return;
    }

    let p_bytes = crate::memory::cstr_as_bytes(p.cast());
    if p_bytes.first() == Some(&b'`') && p_bytes.len() > 2 && p_bytes.last() == Some(&b'`') {
        return;
    }

    let from = if opt_vars().p_ssl != 0 {
        b'\\'
    } else {
        PATHSEP
    };
    let to = if opt_vars().p_ssl != 0 {
        PATHSEP
    } else {
        b'\\'
    };
    {
        let _len = crate::memory::cstr_len(p.cast());
        for _b in unsafe { std::slice::from_raw_parts_mut(p, _len) } {
            if *_b == from {
                *_b = to;
            }
        }
    };
}

#[cfg(windows)]
pub fn path_to_backslash<T>(p: *mut T) -> *mut T {
    if !p.is_null() {
        let p_u8 = p.cast::<u8>();
        let _len = crate::memory::cstr_len(p_u8.cast());
        for _b in unsafe { std::slice::from_raw_parts_mut(p_u8, _len) } {
            if *_b == PATHSEP {
                *_b = b'\\';
            }
        }
    }
    p
}

#[cfg(windows)]
pub fn path_to_slash(p: *mut u8) -> *mut u8 {
    if !p.is_null() && !path_with_url(p.cast_const().cast()) {
        let _len = crate::memory::cstr_len(p.cast());
        for _b in unsafe { std::slice::from_raw_parts_mut(p, _len) } {
            if *_b == b'\\' {
                *_b = PATHSEP;
            }
        }
    }
    p
}

#[cfg(windows)]
pub fn path_to_slash_save(p: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let p = p.into_c_char_ptr();
    if p.is_null() {
        None
    } else {
        let mut bytes = cstr_to_cstring(p).into_bytes_with_nul();
        path_to_slash(bytes.as_mut_ptr());
        Some(std::ffi::CString::from_vec_with_nul(bytes).unwrap())
    }
}

pub fn addfile(gap: &mut Vec<std::ffi::CString>, f: impl crate::strings::IntoCCharPtr, flags: i32) {
    let f = f.into_c_char_ptr();
    {
        let mut file_info: FileInfo = FileInfo::new();

        if (flags & EW_NOTFOUND) == 0
            && if (flags & EW_ALLLINKS) != 0 {
                !os_fileinfo_link(crate::memory::cstr_from_ptr((f).cast()), &mut file_info)
            } else {
                !os_path_exists(crate::memory::cstr_from_ptr((f).cast()))
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

        let isdir = os_isdir(crate::memory::cstr_from_ptr((f).cast()));
        if (isdir && (flags & EW_DIR) == 0) || (!isdir && (flags & EW_FILE) == 0) {
            return;
        }

        if !isdir
            && (flags & EW_EXEC) != 0
            && !os_can_exe(
                crate::memory::cstr_from_ptr((f).cast()),
                None,
                (flags & EW_SHELLCMD) == 0,
            )
        {
            return;
        }

        let f_bytes = crate::memory::cstr_from_ptr(f).to_bytes_with_nul();
        let flen = f_bytes.len() - 1;
        let mut buf = vec![0u8; flen + 1 + usize::from(isdir)];
        buf[..flen + 1].copy_from_slice(f_bytes);
        to_slash(buf.as_mut_ptr());
        if isdir && (flags & EW_ADDSLASH) != 0 {
            add_pathsep(buf.as_mut_ptr());
        }
        gap.push(crate::memory::cstring_until_nul(&buf));
    }
}

pub fn simplify_filename(filename: *mut u8) -> usize {
    let mut components: i32 = 0;
    let mut stripping_disabled = false;
    let mut relative = true;

    let mut p = filename;
    #[cfg(windows)]
    if crate::memory::cstr_as_bytes(p.cast()).get(1) == Some(&b':') {
        p = p.wrapping_add(2);
    }

    if vim_ispathsep(crate::memory::cstr_first_byte(p.cast())) {
        relative = false;
        p = path_skip_sep(p.cast(), true).cast();
    }
    #[cfg(unix)]
    let mut start = p;
    #[cfg(not(unix))]
    let start = p;
    let mut p_end = p.wrapping_add(crate::memory::cstr_len(p.cast()));

    #[cfg(unix)]
    if start > filename.wrapping_add(2) {
        let move_len = crate::memory::ptr_offset_from(p_end, p) as usize + 1;

        unsafe { ptr::copy(p, filename.wrapping_add(1), move_len) };

        p_end = p_end
            .wrapping_sub(crate::memory::ptr_offset_from(p, filename.wrapping_add(1)) as usize);
        p = filename.wrapping_add(1);
        start = p;
    }

    loop {
        if vim_ispathsep(crate::memory::cstr_first_byte(p.cast())) {
            let move_len = crate::memory::ptr_offset_from(p_end, p.wrapping_add(1)) as usize + 1;

            unsafe { ptr::copy(p.wrapping_add(1), p, move_len) };

            p_end = p_end.wrapping_sub(1);
        } else if crate::memory::cstr_first_byte(p.cast()) == b'.'
            && crate::memory::cstr_as_bytes(p.cast())
                .get(1)
                .is_none_or(|&b| vim_ispathsep(i32::from(b)))
        {
            let has_b1 = crate::memory::cstr_len(p.cast()) > 1;
            if p == start && relative {
                p = p.wrapping_add(1 + usize::from(has_b1));
            } else {
                let mut tail = p.wrapping_add(1);
                if has_b1 {
                    tail = path_skip_sep(tail.cast(), true).cast();
                } else if p > start {
                    p = p.wrapping_sub(1);
                }
                let move_len = crate::memory::ptr_offset_from(p_end, tail) as usize + 1;

                unsafe { ptr::copy(tail, p, move_len) };

                p_end = p_end.wrapping_sub(crate::memory::ptr_offset_from(tail, p) as usize);
            }
        } else if crate::memory::cstr_as_bytes(p.cast()).starts_with(b"..")
            && crate::memory::cstr_as_bytes(p.cast())
                .get(2)
                .is_none_or(|&b| vim_ispathsep(i32::from(b)))
        {
            let mut tail = path_skip_sep(p.wrapping_add(2).cast(), true).cast::<u8>();

            if components > 0 {
                let mut do_strip = false;

                if !stripping_disabled {
                    let saved_char = *crate::globals::gptr(p.wrapping_sub(1));

                    *crate::globals::gptr(p.wrapping_sub(1)) = 0;

                    let mut file_info: FileInfo = FileInfo::new();
                    if !os_fileinfo_link(
                        crate::memory::cstr_from_ptr(filename.cast()),
                        &mut file_info,
                    ) {
                        do_strip = true;
                    }

                    *crate::globals::gptr(p.wrapping_sub(1)) = saved_char;

                    p = p.wrapping_sub(1);
                    while p > start && after_pathsep(start.cast(), p.cast()) == 0 {
                        let off = {
                            let (_b, _i) = crate::memory::cstr_span_to_char(
                                start.cast(),
                                p.wrapping_sub(1).cast(),
                            );
                            crate::mbyte::utf_head_off(_b, _i)
                        } + 1;
                        p = p.wrapping_sub(off as usize);
                    }

                    if !do_strip {
                        let saved_char = *crate::globals::gptr(tail);

                        *crate::globals::gptr(tail) = 0;

                        if os_fileinfo(
                            crate::memory::cstr_from_ptr(filename.cast()),
                            &mut file_info,
                        ) {
                            do_strip = true;
                        } else {
                            stripping_disabled = true;
                        }

                        *crate::globals::gptr(tail) = saved_char;

                        if do_strip {
                            let mut new_file_info: FileInfo = FileInfo::new();
                            if p == start && relative {
                                os_fileinfo(c".", &mut new_file_info);
                            } else {
                                let saved_char = *crate::globals::gptr(p);

                                *crate::globals::gptr(p) = 0;

                                os_fileinfo(
                                    crate::memory::cstr_from_ptr(filename.cast()),
                                    &mut new_file_info,
                                );

                                *crate::globals::gptr(p) = saved_char;
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
                    if p == start && relative && *crate::globals::gptr(tail.wrapping_sub(1)) == b'.'
                    {
                        *crate::globals::gptr(p) = b'.';

                        p = p.wrapping_add(1);

                        *crate::globals::gptr(p) = 0;
                    } else {
                        if p > start && *crate::globals::gptr(tail.wrapping_sub(1)) == b'.' {
                            p = p.wrapping_sub(1);
                        }
                        let move_len = crate::memory::ptr_offset_from(p_end, tail) as usize + 1;

                        unsafe { ptr::copy(tail, p, move_len) };

                        p_end =
                            p_end.wrapping_sub(crate::memory::ptr_offset_from(tail, p) as usize);
                    }
                    components -= 1;
                }
            } else if p == start && !relative {
                let move_len = crate::memory::ptr_offset_from(p_end, tail) as usize + 1;

                unsafe { ptr::copy(tail, p, move_len) };

                p_end = p_end.wrapping_sub(crate::memory::ptr_offset_from(tail, p) as usize);
            } else {
                if p == start.wrapping_add(2) && *crate::globals::gptr(p.wrapping_sub(2)) == b'.' {
                    let move_len = crate::memory::ptr_offset_from(p_end, p) as usize + 1;

                    unsafe { ptr::copy(p, p.wrapping_sub(2), move_len) };

                    p_end = p_end.wrapping_sub(2);
                    tail = tail.wrapping_sub(2);
                }
                p = tail;
            }
        } else {
            components += 1;
            p = path_next_component(p.cast()).cast_mut().cast();
        }

        if crate::memory::cstr_is_empty(p.cast()) {
            break;
        }
    }

    crate::memory::ptr_offset_from(p_end, filename) as usize
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

pub fn path_with_url(fname: impl crate::strings::IntoCCharPtr) -> bool {
    let fname = fname.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(fname);
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

pub fn path_with_extension(
    path: impl crate::strings::IntoCCharPtr,
    extension: &std::ffi::CStr,
) -> bool {
    let path = path.into_c_char_ptr();
    {
        let bytes = crate::memory::cstr_as_bytes(path);
        let Some(pos) = bytes.iter().rposition(|&b| b == b'.') else {
            return false;
        };
        let last_dot = path.wrapping_add(pos);
        mb_strcmp_ic(
            opt_vars().p_fic != 0,
            crate::memory::cstr_from_ptr((last_dot.wrapping_add(1)).cast()),
            crate::memory::cstr_from_ptr((extension.as_ptr()).cast()),
        ) == 0
    }
}

#[allow(non_snake_case)]
pub fn vim_isAbsName(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    path_with_url(name) || path_is_absolute(name)
}

pub fn vim_full_name(
    fname: impl crate::strings::IntoCCharPtr,
    buf: *mut u8,
    len: usize,
    force: bool,
) -> bool {
    let fname = fname.into_c_char_ptr();
    {
        *crate::globals::gptr(buf) = 0;

        if fname.is_null() {
            return false;
        }

        let buf_slice = unsafe { std::slice::from_raw_parts_mut(buf, len) };
        let fname_bytes = crate::memory::cstr_as_bytes(fname);
        if fname_bytes.len() > len.saturating_sub(1) {
            crate::memory::copy_bytes_to_c_buf(buf_slice, fname_bytes);
            to_slash(buf);
            return false;
        }

        if path_with_url(fname) {
            crate::memory::copy_bytes_to_c_buf(buf_slice, fname_bytes);
            return true;
        }

        let rv = path_to_absolute(fname, buf, len, i32::from(force));
        if !rv {
            let buf_slice = unsafe { std::slice::from_raw_parts_mut(buf, len) };
            crate::memory::copy_bytes_to_c_buf(buf_slice, fname_bytes);
        }
        to_slash(buf);
        rv
    }
}

pub fn fix_fname(fname: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let fname = fname.into_c_char_ptr();
    #[cfg(unix)]
    {
        FullName_save(fname, true)
    }
    #[cfg(not(unix))]
    {
        if fname.is_null() {
            return None;
        }
        let fname_bytes = crate::memory::cstr_as_bytes(fname);
        if !vim_isAbsName(fname)
            || fname_bytes
                .windows(2)
                .any(|w| matches!(w, b".." | b"//" | b"\\\\"))
            || matches!(fname_bytes.first(), Some(&b'/' | &b'\\'))
        {
            return FullName_save(fname, false);
        }

        let mut bytes = to_slash_save(fname)?.into_bytes_with_nul();
        path_fix_case(bytes.as_mut_ptr());
        Some(crate::memory::cstring_until_nul(&bytes))
    }
}

pub fn path_fix_case(name: *mut u8) {
    let mut file_info: FileInfo = FileInfo::new();
    if !os_fileinfo_link(crate::memory::cstr_from_ptr(name.cast()), &mut file_info) {
        return;
    }

    let name_bytes = crate::memory::cstr_as_bytes(name.cast());
    let slash_pos = name_bytes.iter().rposition(|&b| b == b'/');
    let mut dir = Directory::new();
    let (ok, tail) = if let Some(pos) = slash_pos {
        let slash = name.wrapping_add(pos);

        *crate::globals::gptr(slash) = 0;

        let ok = os_scandir(&mut dir, crate::memory::cstr_from_ptr(name.cast()));

        *crate::globals::gptr(slash) = b'/';

        (ok, slash.wrapping_add(1))
    } else {
        let ok = os_scandir(&mut dir, c".");
        (ok, name)
    };

    if !ok {
        return;
    }

    let taillen = crate::memory::cstr_len(tail.cast());
    while let Some(entry) = os_scandir_next(&mut dir) {
        if crate::memory::cstr_as_bytes(tail.cast()).eq_ignore_ascii_case(entry.to_bytes())
            && taillen == entry.to_bytes().len()
        {
            let mut newname = [0u8; MAXPATHL + 1];
            let tail_off = crate::memory::ptr_offset_from(tail, name) as usize;
            crate::memory::copy_bytes_to_c_buf(
                &mut newname,
                crate::memory::cstr_as_bytes(name.cast()),
            );
            crate::memory::copy_bytes_to_c_buf(&mut newname[tail_off..], entry.to_bytes());
            let mut file_info_new: FileInfo = FileInfo::new();
            if os_fileinfo_link(crate::memory::cstr_until_nul(&newname), &mut file_info_new)
                && os_fileinfo_id_equal(&file_info, &file_info_new)
            {
                unsafe { ptr::copy_nonoverlapping(entry.as_ptr().cast::<u8>(), tail, taillen + 1) };
                break;
            }
        }
    }

    os_closedir(&mut dir);
}

pub fn after_pathsep(
    b: impl crate::strings::IntoCCharPtr,
    p: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let b = b.into_c_char_ptr();
    let p = p.into_c_char_ptr();
    {
        i32::from(
            p > b && vim_ispathsep((*crate::globals::gptr(p.wrapping_sub(1))) as i32) && {
                let (_b, _i) =
                    crate::memory::cstr_span_to_char((b).cast(), (p.wrapping_sub(1)).cast());
                crate::mbyte::utf_head_off(_b, _i)
            } == 0,
        )
    }
}

pub fn same_directory(
    f1: impl crate::strings::IntoCCharPtr,
    f2: impl crate::strings::IntoCCharPtr,
) -> bool {
    let f1 = f1.into_c_char_ptr();
    let f2 = f2.into_c_char_ptr();
    {
        if f1.is_null() || f2.is_null() {
            return false;
        }

        let mut ffname = [0u8; MAXPATHL];
        vim_full_name(f1, ffname.as_mut_ptr(), MAXPATHL, false);
        let ffname_cstr = crate::memory::cstr_until_nul(&ffname);
        let t1 = path_tail_with_sep(ffname_cstr);
        let t2 = path_tail_with_sep(f2);
        let d1 = crate::memory::ptr_offset_from(t1, ffname.as_ptr().cast());
        let d2 = crate::memory::ptr_offset_from(t2, f2);
        d1 == d2 && path_cmp(opt_vars().p_fic != 0, ffname_cstr, f2, d1 as usize) == 0
    }
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

pub fn path_cmp<S1: crate::strings::IntoCCharPtr, S2: crate::strings::IntoCCharPtr>(
    ic: bool,
    p: S1,
    q: S2,
    maxlen: usize,
) -> i32 {
    let p = p.into_c_char_ptr();
    let q = q.into_c_char_ptr();
    {
        let mut p_bytes = crate::memory::cstr_as_bytes(p);
        let mut q_bytes = crate::memory::cstr_as_bytes(q);
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
                if q_bytes[0].to_ascii_lowercase() as i32
                    == unsafe { _getdrive() } + (b'a' as i32) - 1
                {
                    q_bytes = &q_bytes[2..];
                }
            } else if q_bytes
                .first()
                .is_some_and(|&b| vim_ispathsep_nocolon(b as i32))
                && p_bytes.first().is_some_and(u8::is_ascii_alphabetic)
                && p_bytes.get(1) == Some(&b':')
                && p_bytes[0].to_ascii_lowercase() as i32
                    == unsafe { _getdrive() } + (b'a' as i32) - 1
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
}

pub fn path_try_shorten_fname<T>(full_path: *mut T) -> *mut T {
    let mut dirname = [0u8; MAXPATHL];
    let mut p = full_path;

    if os_dirname(&mut dirname) {
        p = path_shorten_fname(full_path, dirname.as_mut_ptr());
        if crate::memory::cstr_is_empty(p.cast_const().cast()) {
            p = full_path;
        }
    }
    p
}

pub fn path_shorten_fname<T>(full_path: *mut T, dir_name: *mut u8) -> *mut T {
    if full_path.is_null() {
        return ptr::null_mut();
    }

    {
        debug_assert!(!dir_name.is_null());
        let dir_name = dir_name.cast_const().cast();
        let full_u8 = full_path.cast::<u8>();
        let len = crate::memory::cstr_len(dir_name);

        let ic = opt_vars().p_fic != 0;

        if path_cmp(ic, dir_name, full_u8.cast(), len) != 0 {
            return ptr::null_mut();
        }

        if len == path_head_length() as usize
            && get_past_head(dir_name) == dir_name.wrapping_add(len).cast_mut()
        {
            return full_u8.wrapping_add(len).cast();
        }

        let p = full_u8.wrapping_add(len);
        if !vim_ispathsep(crate::memory::cstr_first_byte(p.cast())) {
            return ptr::null_mut();
        }

        path_skip_sep(p.wrapping_add(1).cast(), false).cast()
    }
}

pub fn expand_wildcards_eval(
    pat: *mut *mut u8,
    file: &mut Vec<std::ffi::CString>,
    flags: i32,
) -> bool {
    let mut ret = false;
    let mut eval_pat: Option<std::ffi::CString> = None;
    let mut exp_pat_owned: Option<std::ffi::CString> = None;
    let mut exp_pat = *crate::globals::gptr(pat);
    let mut ignored_msg = None;
    let mut usedlen: usize = 0;
    let is_cur_alt_file = crate::memory::cstr_first_byte(exp_pat.cast()) == b'%'
        || crate::memory::cstr_first_byte(exp_pat.cast()) == b'#';
    let mut star_follows = false;

    file.clear();

    if is_cur_alt_file || crate::memory::cstr_first_byte(exp_pat.cast()) == b'<' {
        g_st().emsg_off += 1;
        let exp_cstr = crate::memory::cstr_from_ptr(exp_pat.cast());
        eval_pat = eval_vars(
            exp_cstr,
            exp_cstr,
            &mut usedlen,
            None,
            &mut ignored_msg,
            None,
            true,
        );
        g_st().emsg_off -= 1;
        if let Some(ref ep) = eval_pat {
            let tail_bytes = crate::memory::cstr_as_bytes(exp_pat.wrapping_add(usedlen).cast());
            star_follows = tail_bytes == b"*";
            let mut joined = Vec::with_capacity(ep.as_bytes().len() + tail_bytes.len());
            joined.extend_from_slice(ep.as_bytes());
            joined.extend_from_slice(tail_bytes);
            let cstr = crate::memory::cstr_from_bytes(&joined);
            exp_pat = cstr.as_ptr().cast_mut().cast();
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

pub fn match_suffix(fname: impl crate::strings::IntoCCharPtr) -> bool {
    let fname = fname.into_c_char_ptr();
    {
        let mut suf_buf = [0u8; MAXSUFLEN];
        let fnamelen = crate::memory::cstr_len(fname);
        let mut setsuflen: usize = 0;
        let mut setsuf = crate::memory::cstr_as_bytes(opt_vars().p_su);
        while !setsuf.is_empty() {
            setsuflen = copy_option_part(&mut setsuf, &mut suf_buf[..MAXSUFLEN], c".,");
            if setsuflen == 0 {
                let tail = path_tail(fname);
                if !crate::strings::vim_haschr(tail, b'.' as i32) {
                    setsuflen = 1;
                    break;
                }
            } else {
                if fnamelen >= setsuflen
                    && path_cmp(
                        opt_vars().p_fic != 0,
                        crate::memory::cstr_until_nul(&suf_buf),
                        fname.wrapping_add(fnamelen - setsuflen),
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

pub fn path_full_dir_name(
    directory: impl crate::strings::IntoCCharPtr,
    buffer: *mut u8,
    len: usize,
) -> bool {
    let directory = directory.into_c_char_ptr();
    {
        if crate::memory::cstr_is_empty(directory) {
            return os_dirname(unsafe { std::slice::from_raw_parts_mut(buffer, len) });
        }

        if os_realpath(crate::memory::cstr_from_ptr((directory).cast()), unsafe {
            std::slice::from_raw_parts_mut(buffer, len)
        })
        .is_some()
        {
            return true;
        }

        if path_is_absolute(directory) {
            return false;
        }

        let mut old_dir = [0u8; MAXPATHL];
        if !os_dirname(&mut old_dir) {
            return false;
        }

        crate::memory::copy_bytes_to_c_buf(
            unsafe { std::slice::from_raw_parts_mut(buffer, len) },
            &old_dir,
        );
        if !append_path(buffer, directory, len) {
            return false;
        }

        true
    }
}

pub fn append_path(
    path: *mut u8,
    to_append: impl crate::strings::IntoCCharPtr,
    max_len: usize,
) -> bool {
    let to_append = to_append.into_c_char_ptr();
    {
        let path_slice = unsafe { std::slice::from_raw_parts_mut(path, max_len) };
        let mut current_length = crate::memory::bytes_until_nul(path_slice).len();
        let to_append_bytes = crate::memory::cstr_as_bytes(to_append);
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
}

fn path_to_absolute(
    fname: impl crate::strings::IntoCCharPtr,
    buf: *mut u8,
    len: usize,
    force: i32,
) -> bool {
    let fname = fname.into_c_char_ptr();

    *crate::globals::gptr(buf) = 0;

    #[cfg(windows)]
    let is_abs =
        path_is_absolute(fname) && !vim_ispathsep_nocolon(crate::memory::cstr_first_byte(fname));
    #[cfg(not(windows))]
    let is_abs = path_is_absolute(fname);

    if force == 0 && is_abs {
        return append_path(buf, fname, len);
    }

    let mut tail = path_tail(fname);
    if crate::memory::cstr_as_bytes(tail) == b".." {
        tail = tail.wrapping_add(2);
    }

    let rel_len = (tail as usize) - (fname as usize);
    let dir = cstr_from_bytes(crate::memory::cstr_slice(fname, rel_len));
    if !path_full_dir_name(dir.as_ptr(), buf, len) {
        return false;
    }
    append_path(buf, tail, len)
}

pub fn path_is_absolute(fname: impl crate::strings::IntoCCharPtr) -> bool {
    let fname = fname.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(fname);
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

pub fn path_guess_exepath(argv0: impl crate::strings::IntoCCharPtr, buf_slice: &mut [u8]) {
    let argv0 = argv0.into_c_char_ptr();
    {
        let buf = buf_slice.as_mut_ptr();
        let path_cstr = os_getenv_cstr(c"PATH");
        let argv0_bytes = crate::memory::cstr_as_bytes(argv0);

        if path_is_absolute(argv0) {
            crate::memory::copy_bytes_to_c_buf(buf_slice, argv0_bytes);
        } else if let Some(ref path_cstr) = path_cstr {
            if crate::memory::cstr_first_byte(argv0) == b'.' || argv0_bytes.contains(&PATHSEP) {
                if !os_dirname(buf_slice) {
                    buf_slice[0] = 0;
                }
                crate::memory::append_bytes_to_c_buf(buf_slice, PATHSEPSTR);
                crate::memory::append_bytes_to_c_buf(buf_slice, argv0_bytes);
            } else {
                let path_bytes = path_cstr.as_bytes();
                let namebuff = g_st().name_buff.as_mut_ptr();
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
                    if os_can_exe(crate::memory::cstr_from_ptr((namebuff).cast()), None, false) {
                        crate::memory::copy_bytes_to_c_buf(
                            buf_slice,
                            crate::memory::cstr_as_bytes(namebuff),
                        );
                        to_slash(buf);
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
    }
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
        assert_eq!(
            crate::memory::cstr_as_bytes(path_tail(c"directory/file.txt")),
            b"file.txt"
        );
        assert_eq!(crate::memory::cstr_as_bytes(path_tail(c"directory/")), b"");

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
            crate::memory::cstr_as_bytes(path_next_component(c"some/directory/file.txt")),
            b"directory/file.txt"
        );
        assert_eq!(
            crate::memory::cstr_as_bytes(path_next_component(c"file.txt")),
            b""
        );

        let mut len = 0usize;
        let inv = invocation_path_tail(c"directory/exe a b c", Some(&mut len));
        assert_eq!(len, 3);
        assert_eq!(crate::memory::cstr_as_bytes(inv), b"exe a b c");
        assert_eq!(
            crate::memory::cstr_as_bytes(invocation_path_tail(c"directory/exe a b c", None)),
            b"exe a b c"
        );

        #[cfg(windows)]
        let abs_inv = c"C:\\Users\\anyone\\bin\\z a b";
        #[cfg(not(windows))]
        let abs_inv = c"/usr/bin/z a b";
        let inv2 = invocation_path_tail(abs_inv.as_ptr(), Some(&mut len));
        assert_eq!(len, 1);
        assert_eq!(crate::memory::cstr_as_bytes(inv2), b"z a b");

        let inv3 = invocation_path_tail(c"exe a/b\\c", Some(&mut len));
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

            let mut dir = *b"some/very/long\0";
            assert!(path_shorten_fname(ptr::null_mut::<u8>(), dir.as_mut_ptr()).is_null());
            let mut other = *b"as/this.txt\0";
            assert!(path_shorten_fname(other.as_mut_ptr(), dir.as_mut_ptr()).is_null());
            let mut trailing_dir = *b"some/very/long/\0";
            let mut full = *b"some/very/long/directory/file.txt\0";
            assert!(path_shorten_fname(full.as_mut_ptr(), trailing_dir.as_mut_ptr()).is_null());
            let shortened = path_shorten_fname(full.as_mut_ptr(), dir.as_mut_ptr());
            assert_eq!(
                crate::memory::cstr_as_bytes(shortened.cast()),
                b"directory/file.txt"
            );

            let mut full_dup = *b"some/very/long///directory/file.txt\0";
            let shortened_dup = path_shorten_fname(full_dup.as_mut_ptr(), dir.as_mut_ptr());
            assert_eq!(
                crate::memory::cstr_as_bytes(shortened_dup.cast()),
                b"directory/file.txt"
            );
            #[cfg(windows)]
            {
                let mut drive_nonroot = *b"c:x\0";
                let mut drive_rel = *b"c:xyz\\foo\0";
                assert!(
                    path_shorten_fname(drive_rel.as_mut_ptr(), drive_nonroot.as_mut_ptr())
                        .is_null()
                );
                let mut drive_root = *b"c:\\\0";
                let mut drive_abs = *b"c:\\xyz\\foo\0";
                let shortened_root =
                    path_shorten_fname(drive_abs.as_mut_ptr(), drive_root.as_mut_ptr());
                assert_eq!(
                    crate::memory::cstr_as_bytes(shortened_root.cast()),
                    b"xyz\\foo"
                );
            }

            assert!(path_try_shorten_fname(ptr::null_mut::<u8>()).is_null());
            let mut cwd_buf = [0u8; MAXPATHL];
            assert!(os_dirname(&mut cwd_buf));
            let cwd_str = crate::memory::cstr_until_nul(&cwd_buf).to_str().unwrap();
            let mut in_cwd = CString::new(format!("{cwd_str}/subdir/file.txt"))
                .unwrap()
                .into_bytes_with_nul();
            assert_eq!(
                crate::memory::cstr_as_bytes(path_try_shorten_fname(in_cwd.as_mut_ptr()).cast()),
                b"subdir/file.txt"
            );

            let mut buf = [0u8; 100];
            crate::memory::copy_bytes_to_c_buf(&mut buf, b"path1");
            assert!(append_path(buf.as_mut_ptr(), c"path2", 100));
            assert_eq!(
                crate::memory::cstr_until_nul(&buf).to_bytes(),
                b"path1/path2"
            );

            crate::memory::copy_bytes_to_c_buf(&mut buf, b"path1/");
            assert!(append_path(buf.as_mut_ptr(), c"path2", 100));
            assert_eq!(
                crate::memory::cstr_until_nul(&buf).to_bytes(),
                b"path1/path2"
            );
            assert!(!append_path(buf.as_mut_ptr(), c"path2", 11));

            crate::memory::copy_bytes_to_c_buf(&mut buf, b"path1");
            assert!(append_path(buf.as_mut_ptr(), c"", 6));
            assert_eq!(crate::memory::cstr_until_nul(&buf).to_bytes(), b"path1");
            assert!(append_path(buf.as_mut_ptr(), c".", 6));
            assert_eq!(crate::memory::cstr_until_nul(&buf).to_bytes(), b"path1");

            buf[0] = 0;
            assert!(append_path(buf.as_mut_ptr(), c"/path2", 7));
            assert_eq!(crate::memory::cstr_until_nul(&buf).to_bytes(), b"/path2");
        }
    }

    #[test]
    fn path_url_extension_and_is_absolute() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(path_is_absolute(c"/some/directory/"));
        #[cfg(windows)]
        assert!(path_is_absolute(c"C:/some/directory/"));
        #[cfg(not(windows))]
        assert!(path_is_absolute(c"~/in/my/home~/directory"));
        assert!(!path_is_absolute(c"not/in/my/home~/directory"));

        let saved_fic = opt_vars().p_fic;
        opt_vars().p_fic = 0;
        assert!(path_with_extension(c"/some/path/file.lua", c"lua"));
        assert!(!path_with_extension(c"/some/path/file.vim", c"lua"));
        assert!(!path_with_extension(c"/some/path/file", c"lua"));
        assert!(!path_with_extension(c"/some/path/file.LUA", c"lua"));
        opt_vars().p_fic = 1;
        assert!(path_with_extension(c"/some/path/file.LUA", c"lua"));
        opt_vars().p_fic = saved_fic;

        assert!(path_with_url(c"test://xyz/foo/b0"));
        assert!(!path_with_url(c"test:\\\\xyz\\foo\\b0"));
        assert!(path_with_url(c"test123://xyz/foo/b0"));
        assert!(!path_with_url(c"test123:\\\\xyz\\foo\\b0"));
        assert!(!path_with_url(c"test_abc://xyz/foo/b2"));
        assert!(path_with_url(c"test+abc-123.ghi://xyz/foo/b1"));
        assert!(!path_with_url(c"test+abc-123.ghi:\\\\xyz\\foo\\b1"));
        assert!(!path_with_url(c"-test://xyz/foo/b4"));
        assert!(!path_with_url(c"test-://xyz/foo/b5"));
        assert!(path_with_url(c"test-C:/xyz/foo/b5"));
        assert!(!path_with_url(c"c:/xyz/foo/b5"));
        assert!(!path_with_url(c"C:/xyz/foo/b5"));
        assert!(!path_with_url(c"C:\\xyz\\foo\\b5"));
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
            let cwd = crate::memory::cstr_until_nul(&cwd_buf)
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

            assert!(path_equal(f1_c.as_ptr(), f1_c.as_ptr(), K_PATH_CMP_EXPAND));
            assert!(path_equal(
                f1_c.as_ptr(),
                f1_abs_c.as_ptr(),
                K_PATH_CMP_EXPAND
            ));
            assert!(!path_equal(f1_c.as_ptr(), f2_c.as_ptr(), K_PATH_CMP_EXPAND));
            assert!(!path_equal(
                f1_c.as_ptr(),
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

            let saved_fic = opt_vars().p_fic;
            opt_vars().p_fic = 0;
            assert!(!path_equal(c"Foo", c"foo", K_PATH_CMP_LITERAL));
            opt_vars().p_fic = 1;
            assert!(path_equal(c"Foo", c"foo", K_PATH_CMP_LITERAL));
            opt_vars().p_fic = saved_fic;

            // path_full_dir_name
            let mut out = [0u8; MAXPATHL];
            let mut empty = *b"\0";
            assert!(path_full_dir_name(
                empty.as_mut_ptr().cast(),
                out.as_mut_ptr(),
                MAXPATHL
            ));
            assert_eq!(crate::memory::cstr_until_nul(&out).to_str().unwrap(), cwd);

            let mut rel_c = CString::new(rel_dir.clone()).unwrap().into_bytes_with_nul();
            assert!(path_full_dir_name(
                rel_c.as_mut_ptr().cast(),
                out.as_mut_ptr(),
                MAXPATHL
            ));
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                abs_dir
            );

            let mut non_exist_abs = CString::new(format!("{abs_dir}/does_not_exist"))
                .unwrap()
                .into_bytes_with_nul();
            assert!(!path_full_dir_name(
                non_exist_abs.as_mut_ptr().cast(),
                out.as_mut_ptr(),
                MAXPATHL
            ));

            // vim_FullName
            assert!(!vim_full_name(ptr::null(), out.as_mut_ptr(), 10, true));
            assert!(vim_full_name(
                c"http://www.neovim.org",
                out.as_mut_ptr(),
                MAXPATHL,
                true
            ));
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                "http://www.neovim.org"
            );

            let rel_dot = CString::new(format!("./{f1_rel}")).unwrap();
            assert!(vim_full_name(
                rel_dot.as_ptr(),
                out.as_mut_ptr(),
                MAXPATHL,
                true
            ));
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                format!("{abs_dir}/f1.o")
            );

            let dir_dotdot = CString::new(format!("{rel_dir}/..")).unwrap();
            assert!(vim_full_name(
                dir_dotdot.as_ptr(),
                out.as_mut_ptr(),
                MAXPATHL,
                true
            ));
            let expected_parent = std::path::Path::new(&abs_dir)
                .parent()
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                expected_parent
            );

            let dir_slash = CString::new(format!("{rel_dir}/")).unwrap();
            assert!(vim_full_name(
                dir_slash.as_ptr(),
                out.as_mut_ptr(),
                MAXPATHL,
                true
            ));
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                abs_dir
            );

            // path_fix_case
            let mut case_buf = CString::new(format!("{rel_dir}/camelcase"))
                .unwrap()
                .into_bytes_with_nul();
            path_fix_case(case_buf.as_mut_ptr());
            let expected_case = if std::path::Path::new(&format!("{abs_dir}/camelcase")).exists() {
                format!("{rel_dir}/CamelCase")
            } else {
                format!("{rel_dir}/camelcase")
            };
            assert_eq!(
                crate::memory::cstr_until_nul(&case_buf).to_str().unwrap(),
                expected_case
            );

            // path_guess_exepath
            path_guess_exepath(c"./nvim", &mut out);
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                format!("{cwd}/./nvim")
            );

            path_guess_exepath(c"23u0293_not_in_path", &mut out);
            assert_eq!(
                crate::memory::cstr_until_nul(&out).to_str().unwrap(),
                "23u0293_not_in_path"
            );

            let _ = std::fs::remove_dir_all(&abs_dir);
        }
    }
}
