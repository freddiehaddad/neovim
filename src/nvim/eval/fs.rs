//! Filesystem related builtin functions (`src/nvim/eval/fs.c`).

#![allow(
    dead_code,
    unused_imports,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::find_buffer_by_handle;
#[allow(unused_imports)]
use crate::cmdexpand::{ExpandCleanup, ExpandInit, ExpandOne, globpath};
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::typval::tv_list_set_ret;
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_blob_alloc_ret, tv_blob_free, tv_clear, tv_get_bool_chk, tv_get_number, tv_get_number_chk,
    tv_list_alloc_ret, tv_list_append_owned_tv, tv_list_append_string, tv_list_item_remove,
};
#[allow(unused_imports)]
use crate::eval::typval::{tv_blob_len, tv_list_ref};
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_check_for_nonempty_string_arg, tv_check_for_string_arg, tv_check_str_or_nr, tv_get_string,
    tv_get_string_buf, tv_get_string_buf_chk, tv_get_string_chk,
};
#[allow(unused_imports)]
use crate::eval::typval::{tv_list_first, tv_list_len};
#[allow(unused_imports)]
use crate::eval::userfunc::add_defer;
#[allow(unused_imports)]
use crate::eval::userfunc::can_add_defer;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_string;
#[allow(unused_imports)]
use crate::eval::vars::{prepare_vimvar, restore_vimvar};
#[allow(unused_imports)]
use crate::eval::{do_string_sub, eval_expr_typval};
#[allow(unused_imports)]
use crate::event::r#loop::uv_strerror;
#[allow(unused_imports)]
use crate::ex_cmds::check_secure;
#[allow(unused_imports)]
use crate::ex_docmd::do_chdir;
#[allow(unused_imports)]
use crate::ex_docmd::vim_mkdir_emsg;
#[allow(unused_imports)]
use crate::fileio::file_pat_to_reg_pat;
#[cfg(windows)]
#[allow(unused_imports)]
use crate::fileio::{_fileno, _fseeki64};
#[allow(unused_imports)]
use crate::fileio::{delete_recursive, readdir_core, vim_copyfile, vim_rename, vim_tempname};
#[allow(unused_imports)]
use crate::fileio::{fclose, fread};
#[cfg(not(windows))]
#[allow(unused_imports)]
use crate::fileio::{fileno, fseeko, readlink};
use crate::globals::*;
#[allow(unused_imports)]
use crate::mbyte::{utf_head_off, utfc_ptr2len};
#[allow(unused_imports)]
use crate::message::emsg;
use crate::message::semsg;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::os::env::{expand_env_save, home_replace};
#[allow(unused_imports)]
use crate::os::fs::os_fopen;
#[allow(unused_imports)]
use crate::os::fs::{
    os_can_exe, os_dirname, os_file_is_readable, os_file_is_writable, os_fileinfo, os_fileinfo_fd,
    os_fileinfo_link, os_fileinfo_size, os_fileinfo2, os_getperm, os_isdir, os_mkdir_recurse,
    os_realpath, os_remove, os_rmdir,
};
#[allow(unused_imports)]
use crate::os::fs::{os_is_reparse_point_include, os_resolve_shortcut};
#[cfg(windows)]
#[allow(unused_imports)]
use crate::path::slash_adjust;
#[allow(unused_imports)]
use crate::path::{
    FullName_save, add_pathsep, after_pathsep, path_cmp, path_is_absolute, path_next_component,
    path_tail, path_tail_with_sep, path_to_slash_save, shorten_dir_len, simplify_filename,
    vim_isAbsName, vim_ispathsep,
};
#[allow(unused_imports)]
use crate::runtime::script_is_lua;
#[allow(unused_imports)]
use crate::strings::gettext;
#[allow(unused_imports)]
use crate::strings::strlen as c_strlen;
#[allow(unused_imports)]
use crate::strings::{concat_str, vim_strchr, vim_strsave_shellescape, xstrnsave};
#[allow(unused_imports)]
use crate::window::find_tabpage;
use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::ptr;

use crate::api::private::converter::sctx_T;
use crate::api::private::validate::{Error, ErrorType};
use crate::api::tabpage::{tabpage_T, win_T};
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::eval::deprecated::{EvalFuncData, buf_T};
use crate::eval::executor::blob_T;
use crate::eval::window::find_win_by_nr;
use crate::file_search::{
    FINDFILE_DIR, FINDFILE_FILE, find_file_in_path_option, vim_findfile_cleanup,
};
use crate::garray::{ga_clear_strings, ga_concat_strings, ga_grow, ga_init, garray_T};
use crate::math::{FAIL, OK};
use crate::memory::{xfree, xmalloc, xmallocz, xmemdupz, xrealloc, xstrdup, xstrlcat, xstrlcpy};
use crate::os::fileio::{
    FileDescriptor, K_FILE_APPEND, K_FILE_CREATE, K_FILE_MK_DIR, K_FILE_TRUNCATE, file_close,
    file_flush, file_open, file_write,
};
use crate::sign::expand_T;

pub use crate::eval::buffer::varnumber_T;
pub use crate::fileio::off_T;
pub type CheckItem =
    Option<unsafe extern "C" fn(expr: *mut c_void, name: *const c_char) -> varnumber_T>;

const NUL: c_char = 0;
const NL: c_char = b'\n' as c_char;
const NUMBUFLEN: usize = 65;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const MAXLNUM: i64 = 0x7fffffff;

const VAR_UNLOCKED: c_int = 0;
const VAR_FIXED: c_int = 2;
const K_LIST_LEN_UNKNOWN: isize = -1;

const VALID_PATH: c_int = 1;
const VALID_HEAD: c_int = 2;

const VV_VAL: c_int = 35;
const SID_API_CLIENT: c_int = -9;

const EXPAND_FILES: c_int = 2;
const WILD_ALL: c_int = 6;
const WILD_ALL_KEEP: c_int = 8;
const WILD_USE_NL: c_int = 0x04;
const WILD_KEEP_ALL: c_int = 0x20;
const WILD_SILENT: c_int = 0x40;
const WILD_ICASE: c_int = 0x100;
const WILD_ALLLINKS: c_int = 0x200;

const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;

const kCdScopeInvalid: c_int = -1;
const kCdScopeBuffer: c_int = 0;
const kCdScopeWindow: c_int = 1;
const kCdScopeTabpage: c_int = 2;
const kCdScopeGlobal: c_int = 3;

const S_IFMT_MASK: u64 = 0xF000;
const S_IFIFO_VAL: u64 = 0x1000;
const S_IFCHR_VAL: u64 = 0x2000;
const S_IFDIR_VAL: u64 = 0x4000;
const S_IFBLK_VAL: u64 = 0x6000;
const S_IFREG_VAL: u64 = 0x8000;
const S_IFLNK_VAL: u64 = 0xA000;
const S_IFSOCK_VAL: u64 = 0xC000;

const E_ERROR_WHILE_WRITING_STR: &CStr = c"E80: Error while writing: %s";

pub use crate::fileio::FILE;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct uv_timespec_t {
    pub tv_sec: c_long,
    pub tv_nsec: c_long,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct uv_stat_t {
    pub st_dev: u64,
    pub st_mode: u64,
    pub st_nlink: u64,
    pub st_uid: u64,
    pub st_gid: u64,
    pub st_rdev: u64,
    pub st_ino: u64,
    pub st_size: u64,
    pub st_blksize: u64,
    pub st_blocks: u64,
    pub st_flags: u64,
    pub st_gen: u64,
    pub st_atim: uv_timespec_t,
    pub st_mtim: uv_timespec_t,
    pub st_ctim: uv_timespec_t,
    pub st_birthtim: uv_timespec_t,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileInfo {
    pub stat: uv_stat_t,
    pub prefix_off: usize,
    pub root_off: usize,
    pub rest_off: usize,
    pub r#type: c_int,
}

#[inline]
fn s_isreg(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFREG_VAL
}

#[inline]
fn s_isdir(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFDIR_VAL
}

#[inline]
fn s_islnk(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFLNK_VAL
}

#[inline]
fn s_ischr(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFCHR_VAL
}

#[inline]
fn s_isfifo(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFIFO_VAL
}

#[inline]
#[cfg(unix)]
fn s_isblk(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFBLK_VAL
}

#[inline]
#[cfg(not(unix))]
fn s_isblk(_m: u64) -> bool {
    false
}

#[inline]
#[cfg(unix)]
fn s_issock(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFSOCK_VAL
}

#[inline]
#[cfg(not(unix))]
fn s_issock(_m: u64) -> bool {
    false
}

#[inline]
unsafe fn c_fileno(stream: *mut FILE) -> c_int {
    #[cfg(windows)]
    {
        unsafe { _fileno(stream) }
    }
    #[cfg(not(windows))]
    {
        unsafe { fileno(stream) }
    }
}

#[inline]
unsafe fn vim_fseek(stream: *mut FILE, offset: off_T, whence: c_int) -> c_int {
    #[cfg(windows)]
    {
        unsafe { _fseeki64(stream, offset, whence) }
    }
    #[cfg(not(windows))]
    {
        unsafe { fseeko(stream, offset, whence) }
    }
}

/// Adjust a filename, according to a string of modifiers.
/// *fnamep must be NUL terminated when called.  When returning, the length is
/// determined by *fnamelen.
/// Returns VALID_ flags or -1 for failure.
/// When there is an error, *fnamep is set to NULL.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn modify_fname(
    src: *mut c_char,
    tilde_file: bool,
    usedlen: *mut usize,
    fnamep: *mut *mut c_char,
    bufp: *mut *mut c_char,
    fnamelen: *mut usize,
    use_shellslash: bool,
) -> c_int {
    unsafe {
        let mut valid: c_int = 0;
        let mut s: *mut c_char = ptr::null_mut();
        let mut p: *mut c_char;
        let mut pbuf: *mut c_char;
        let mut dirname = [0 as c_char; MAXPATHL];
        let mut has_fullname = false;
        let mut has_homerelative = false;
        let mut didit = false;

        loop {
            // ":p" - full path/file_name
            if *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b'p' as c_char {
                has_fullname = true;

                valid |= VALID_PATH;
                *usedlen += 2;

                // Expand "~/path" for all systems and "~user/path" for Unix
                let tilde_cond = *(*fnamep) == b'~' as c_char
                    && (cfg!(unix)
                        || *(*fnamep).add(1) == b'/' as c_char
                        || (cfg!(windows) && *(*fnamep).add(1) == b'\\' as c_char)
                        || *(*fnamep).add(1) == NUL)
                    && !(tilde_file && *(*fnamep).add(1) == NUL);
                if tilde_cond {
                    *fnamep = expand_env_save(*fnamep);
                    xfree((*bufp).cast()); // free any allocated file name
                    *bufp = *fnamep;
                    if (*fnamep).is_null() {
                        return -1;
                    }
                }

                // When "/." or "/.." is used: force expansion to get rid of it.
                p = *fnamep;
                while *p != NUL {
                    if vim_ispathsep(*p as u8 as c_int)
                        && *p.add(1) == b'.' as c_char
                        && (*p.add(2) == NUL
                            || vim_ispathsep(*p.add(2) as u8 as c_int)
                            || (*p.add(2) == b'.' as c_char
                                && (*p.add(3) == NUL || vim_ispathsep(*p.add(3) as u8 as c_int))))
                    {
                        break;
                    }
                    p = p.add(utfc_ptr2len(p) as usize);
                }

                // FullName_save() is slow, don't use it when not needed.
                if *p != NUL
                    || !vim_isAbsName(*fnamep)
                    || (cfg!(windows)
                        && (*(*fnamep) == b'/' as c_char || *(*fnamep) == b'\\' as c_char))
                {
                    *fnamep = FullName_save(*fnamep, *p != NUL);
                    xfree((*bufp).cast()); // free any allocated file name
                    *bufp = *fnamep;
                    if (*fnamep).is_null() {
                        return -1;
                    }
                }

                // Append a path separator to a directory.
                if os_isdir(*fnamep) {
                    // Make room for one or two extra characters.
                    *fnamep = xstrnsave(*fnamep, c_strlen(*fnamep) + 2);
                    xfree((*bufp).cast()); // free any allocated file name
                    *bufp = *fnamep;
                    add_pathsep(*fnamep);
                }
            }

            let mut c: c_int;

            // ":." - path relative to the current directory
            // ":~" - path relative to the home directory
            // ":8" - shortname path - postponed till after
            while *src.add(*usedlen) == b':' as c_char && {
                c = *src.add(*usedlen + 1) as u8 as c_int;
                c == b'.' as c_int || c == b'~' as c_int || c == b'8' as c_int
            } {
                *usedlen += 2;
                if c == b'8' as c_int {
                    continue;
                }
                pbuf = ptr::null_mut();
                // Need full path first (use expand_env() to remove a "~/")
                if !has_fullname && !has_homerelative {
                    if *(*fnamep) == b'~' as c_char {
                        pbuf = expand_env_save(*fnamep);
                        p = pbuf;
                    } else {
                        pbuf = FullName_save(*fnamep, false);
                        p = pbuf;
                    }
                } else {
                    p = *fnamep;
                }

                has_fullname = false;

                if !p.is_null() {
                    let mut dirnamelen: usize = 0;

                    if c == b'.' as c_int {
                        os_dirname(dirname.as_mut_ptr(), MAXPATHL);
                        if has_homerelative {
                            s = xstrdup(dirname.as_ptr());
                            dirnamelen =
                                home_replace(ptr::null(), s, dirname.as_mut_ptr(), MAXPATHL, true);
                            xfree(s.cast());
                        }

                        if dirnamelen == 0 {
                            dirnamelen = c_strlen(dirname.as_ptr());
                        }

                        // Do not call shorten_fname() here since it removes the prefix
                        // even though the path does not have a prefix.
                        if path_cmp(p_fic != 0, p, dirname.as_ptr(), dirnamelen) == 0 {
                            p = p.add(dirnamelen);
                            if vim_ispathsep(*p as u8 as c_int) {
                                while *p != NUL && vim_ispathsep(*p as u8 as c_int) {
                                    p = p.add(1);
                                }
                                *fnamep = p;
                                if !pbuf.is_null() {
                                    // free any allocated file name
                                    xfree((*bufp).cast());
                                    *bufp = pbuf;
                                    pbuf = ptr::null_mut();
                                }
                            }
                        }
                    } else {
                        dirnamelen =
                            home_replace(ptr::null(), p, dirname.as_mut_ptr(), MAXPATHL, true);
                        // Only replace it when it starts with '~'
                        if dirname[0] == b'~' as c_char {
                            s = xmemdupz(dirname.as_ptr().cast(), dirnamelen).cast();
                            debug_assert!(!s.is_null());
                            *fnamep = s;
                            xfree((*bufp).cast());
                            *bufp = s;
                            has_homerelative = true;
                        }
                    }
                    xfree(pbuf.cast());
                }
            }

            let mut file_info = FileInfo::default();
            os_fileinfo2(*fnamep, &raw mut file_info);
            if *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b'h' as c_char {
                s = (*fnamep).add(file_info.rest_off);
                *fnamep = (*fnamep).add(file_info.prefix_off);
            }

            let mut tail = path_tail(*fnamep);
            *fnamelen = c_strlen(*fnamep);

            // ":h" - head, remove "/file_name", can be repeated
            // Don't remove the logical root, see `FileInfo`.
            while *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b'h' as c_char {
                valid |= VALID_HEAD;
                *usedlen += 2;
                while (tail as usize) > (s as usize) && after_pathsep(s, tail) != 0 {
                    tail = tail.sub(utf_head_off(*fnamep, tail.sub(1)) as usize + 1);
                }
                *fnamelen = if (tail as usize) <= (s as usize) {
                    (s as usize).wrapping_sub(*fnamep as usize)
                } else {
                    (tail as usize).wrapping_sub(*fnamep as usize)
                };
                if *fnamelen == 0 {
                    // Result is empty.  Turn it into "." to make ":cd %:h" work.
                    xfree((*bufp).cast());
                    tail = xstrdup(c".".as_ptr());
                    *fnamep = tail;
                    *bufp = tail;
                    s = *fnamep; // s pointed into the freed buffer.
                    *fnamelen = 1;
                } else {
                    while (tail as usize) > (s as usize) && after_pathsep(s, tail) == 0 {
                        tail = tail.sub(utf_head_off(*fnamep, tail.sub(1)) as usize + 1);
                    }
                }
            }

            // ":8" - shortname
            if *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b'8' as c_char {
                *usedlen += 2;
            }

            // ":t" - tail, just the basename
            if *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b't' as c_char {
                *usedlen += 2;
                *fnamelen -= (tail as usize).wrapping_sub(*fnamep as usize);
                *fnamep = tail;
            }

            // ":e" - extension, can be repeated
            // ":r" - root, without extension, can be repeated
            while *src.add(*usedlen) == b':' as c_char
                && (*src.add(*usedlen + 1) == b'e' as c_char
                    || *src.add(*usedlen + 1) == b'r' as c_char)
            {
                // find a '.' in the tail:
                // - for second :e: before the current fname
                // - otherwise: The last '.'
                let is_second_e = (*fnamep as usize) > (tail as usize);
                if *src.add(*usedlen + 1) == b'e' as c_char && is_second_e {
                    s = (*fnamep).sub(2);
                } else {
                    s = (*fnamep).add(*fnamelen).sub(1);
                }

                while (s as usize) > (tail as usize) {
                    if *s == b'.' as c_char {
                        break;
                    }
                    s = s.sub(1);
                }
                if *src.add(*usedlen + 1) == b'e' as c_char {
                    if (s as usize) > (tail as usize) {
                        // we stopped at a '.' (so anchor to &'.' + 1)
                        let newstart = s.add(1);
                        let distance_stepped_back =
                            (*fnamep as usize).wrapping_sub(newstart as usize);
                        *fnamelen += distance_stepped_back;
                        *fnamep = newstart;
                    } else if (*fnamep as usize) <= (tail as usize) {
                        *fnamelen = 0;
                    }
                } else {
                    // :r - Remove one extension
                    let max_ptr = if (tail as usize) > (*fnamep as usize) {
                        tail
                    } else {
                        *fnamep
                    };
                    if (s as usize) > (max_ptr as usize) {
                        *fnamelen = (s as usize).wrapping_sub(*fnamep as usize);
                    }
                }
                *usedlen += 2;
            }

            #[cfg(windows)]
            if !didit && use_shellslash && !(*fnamep).is_null() {
                *fnamep = xstrdup(*fnamep);
                slash_adjust(*fnamep);
                xfree((*bufp).cast());
                *bufp = *fnamep;
            }
            #[cfg(not(windows))]
            let _ = use_shellslash;

            // ":s?pat?foo?" - substitute
            // ":gs?pat?foo?" - global substitute
            let mut repeat_loop = false;
            if *src.add(*usedlen) == b':' as c_char
                && (*src.add(*usedlen + 1) == b's' as c_char
                    || (*src.add(*usedlen + 1) == b'g' as c_char
                        && *src.add(*usedlen + 2) == b's' as c_char))
            {
                let mut flags = c"".as_ptr();
                s = src.add(*usedlen + 2);
                if *src.add(*usedlen + 1) == b'g' as c_char {
                    flags = c"g".as_ptr();
                    s = s.add(1);
                }

                let sep = *s as u8 as c_int;
                s = s.add(1);
                if sep != 0 {
                    // find end of pattern
                    p = vim_strchr(s, sep);
                    if !p.is_null() {
                        let pat: *mut c_char =
                            xmemdupz(s.cast(), (p as usize).wrapping_sub(s as usize)).cast();
                        s = p.add(1);
                        // find end of substitution
                        p = vim_strchr(s, sep);
                        if !p.is_null() {
                            let sub: *mut c_char =
                                xmemdupz(s.cast(), (p as usize).wrapping_sub(s as usize)).cast();
                            let str_copy: *mut c_char =
                                xmemdupz((*fnamep).cast(), *fnamelen).cast();
                            *usedlen = (p.add(1) as usize).wrapping_sub(src as usize);
                            let mut slen: usize = 0;
                            s = do_string_sub(
                                str_copy,
                                *fnamelen,
                                pat,
                                sub,
                                ptr::null_mut(),
                                flags,
                                &raw mut slen,
                            );
                            *fnamep = s;
                            *fnamelen = slen;
                            xfree((*bufp).cast());
                            *bufp = s;
                            didit = true;
                            xfree(sub.cast());
                            xfree(str_copy.cast());
                        }
                        xfree(pat.cast());
                    }
                    // after using ":s", repeat all the modifiers
                    if didit {
                        repeat_loop = true;
                    }
                }
            }
            if repeat_loop {
                continue;
            }

            if *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b'S' as c_char {
                // vim_strsave_shellescape() needs a NUL terminated string.
                c = *(*fnamep).add(*fnamelen) as u8 as c_int;
                if c != NUL as c_int {
                    *(*fnamep).add(*fnamelen) = NUL;
                }
                p = vim_strsave_shellescape(*fnamep, false, false);
                if c != NUL as c_int {
                    *(*fnamep).add(*fnamelen) = c as c_char;
                }
                xfree((*bufp).cast());
                *fnamep = p;
                *bufp = p;
                *fnamelen = c_strlen(p);
                *usedlen += 2;
            }

            break;
        }

        valid
    }
}

/// "chdir(dir)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_chdir(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if check_secure() {
            return;
        }

        if (*argvars).v_type != VarType::VAR_STRING {
            // Returning an empty string means it failed.
            // No error message, for historic reasons.
            return;
        }

        // Return the current directory
        let cwd: *mut c_char = xmalloc(MAXPATHL).cast();
        if os_dirname(cwd, MAXPATHL) != FAIL {
            #[cfg(windows)]
            slash_adjust(cwd);
            (*rettv).vval.v_string = xstrdup(cwd);
        }
        xfree(cwd.cast());

        let mut scope: c_int = kCdScopeGlobal;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let s = tv_get_string(argvars.add(1));
            let s_bytes = CStr::from_ptr(s).to_bytes();
            if s_bytes == b"global" {
                scope = kCdScopeGlobal;
            } else if s_bytes == b"tabpage" {
                scope = kCdScopeTabpage;
            } else if s_bytes == b"window" {
                scope = kCdScopeWindow;
            } else if s_bytes == b"buffer" {
                scope = kCdScopeBuffer;
            } else {
                semsg!(gettext(e_invargNval.as_ptr()), c"scope".as_ptr(), s);
                return;
            }
        } else if !(*curbuf).b_localdir.is_null() {
            scope = kCdScopeBuffer;
        } else if !(*curwin).w_localdir.is_null() {
            scope = kCdScopeWindow;
        } else if !(*curtab).tp_localdir.is_null() {
            scope = kCdScopeTabpage;
        }

        if !do_chdir((*argvars).vval.v_string, scope) {
            // Directory change failed
            xfree((*rettv).vval.v_string.cast());
            (*rettv).vval.v_string = ptr::null_mut();
        }
    }
}

/// "delete()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_delete(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;
        if check_secure() {
            return;
        }

        let name = tv_get_string(argvars);
        if *name == NUL {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let flags = if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            tv_get_string_buf(argvars.add(1), nbuf.as_mut_ptr())
        } else {
            c"".as_ptr()
        };

        let flags_bytes = CStr::from_ptr(flags).to_bytes();
        if flags_bytes.is_empty() {
            // delete a file
            (*rettv).vval.v_number = if os_remove(name) == 0 { 0 } else { -1 };
        } else if flags_bytes == b"d" {
            // delete an empty directory
            (*rettv).vval.v_number = if os_rmdir(name) == 0 { 0 } else { -1 };
        } else if flags_bytes == b"rf" {
            // delete a directory recursively
            (*rettv).vval.v_number = varnumber_T::from(delete_recursive(name));
        } else {
            semsg!(gettext(e_invexpr2.as_ptr()), flags);
        }
    }
}

/// "executable()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_executable(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_string_arg(argvars, 0) == FAIL {
            return;
        }

        // Check in $PATH and also check directly if there is a directory name
        (*rettv).vval.v_number =
            varnumber_T::from(os_can_exe(tv_get_string(argvars), ptr::null_mut(), true));
    }
}

/// "exepath()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_exepath(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_nonempty_string_arg(argvars, 0) == FAIL {
            return;
        }

        let mut path: *mut c_char = ptr::null_mut();

        os_can_exe(tv_get_string(argvars), &raw mut path, true);

        #[cfg(windows)]
        if !path.is_null() {
            slash_adjust(path);
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = path;
    }
}

/// "filecopy()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_filecopy(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = 0;

        if check_secure()
            || tv_check_for_string_arg(argvars, 0) == FAIL
            || tv_check_for_string_arg(argvars, 1) == FAIL
        {
            return;
        }

        let from = tv_get_string(argvars);

        let mut from_info = FileInfo::default();
        if os_fileinfo_link(from, &raw mut from_info)
            && (s_isreg(from_info.stat.st_mode) || s_islnk(from_info.stat.st_mode))
        {
            (*rettv).vval.v_number = varnumber_T::from(
                vim_copyfile(tv_get_string(argvars), tv_get_string(argvars.add(1))) == OK,
            );
        }
    }
}

/// "filereadable()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_filereadable(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let p = tv_get_string(argvars);
        (*rettv).vval.v_number =
            varnumber_T::from(*p != NUL && !os_isdir(p) && os_file_is_readable(p));
    }
}

/// @return  0 for not writable
///          1 for writable file
///          2 for a dir which we have rights to write into.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_filewritable(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let filename = tv_get_string(argvars);
        (*rettv).vval.v_number = varnumber_T::from(os_file_is_writable(filename));
    }
}

unsafe fn findfilendir(argvars: *mut typval_T, rettv: *mut typval_T, find_what: c_int) {
    unsafe {
        let mut fresult: *mut c_char = ptr::null_mut();
        let mut path = if *(*curbuf).b_p_path == NUL {
            p_path
        } else {
            (*curbuf).b_p_path
        };
        let mut count: c_int = 1;
        let mut first = true;
        let mut error = false;

        (*rettv).vval.v_string = ptr::null_mut();
        (*rettv).v_type = VarType::VAR_STRING;

        let fname = tv_get_string(argvars);

        let mut pathbuf = [0 as c_char; NUMBUFLEN];
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let p = tv_get_string_buf_chk(argvars.add(1), pathbuf.as_mut_ptr());
            if p.is_null() {
                error = true;
            } else {
                if *p != NUL {
                    path = p.cast_mut();
                }

                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    count = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
                }
            }
        }

        if count < 0 {
            tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN);
        }

        if *fname != NUL && !error {
            let mut file_to_find: *mut c_char = ptr::null_mut();
            let mut search_ctx: *mut c_char = ptr::null_mut();

            loop {
                if (*rettv).v_type == VarType::VAR_STRING || (*rettv).v_type == VarType::VAR_LIST {
                    xfree(fresult.cast());
                }
                fresult = find_file_in_path_option(
                    if first {
                        fname.cast_mut()
                    } else {
                        ptr::null_mut()
                    },
                    if first { c_strlen(fname) } else { 0 },
                    0,
                    c_int::from(first),
                    path,
                    find_what,
                    (*curbuf).b_ffname,
                    if find_what == FINDFILE_DIR {
                        c"".as_ptr().cast_mut()
                    } else {
                        (*curbuf).b_p_sua
                    },
                    &raw mut file_to_find,
                    &raw mut search_ctx,
                );
                first = false;

                if !fresult.is_null() && (*rettv).v_type == VarType::VAR_LIST {
                    tv_list_append_string((*rettv).vval.v_list, fresult, -1);
                }

                let keep_going = (*rettv).v_type == VarType::VAR_LIST || {
                    count -= 1;
                    count > 0
                };
                if !keep_going || fresult.is_null() {
                    break;
                }
            }

            xfree(file_to_find.cast());
            vim_findfile_cleanup(search_ctx.cast());
        }

        if (*rettv).v_type == VarType::VAR_STRING {
            (*rettv).vval.v_string = fresult;
        }
    }
}

/// "finddir({fname}[, {path}[, {count}]])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_finddir(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        findfilendir(argvars, rettv, FINDFILE_DIR);
    }
}

/// "findfile({fname}[, {path}[, {count}]])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_findfile(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        findfilendir(argvars, rettv, FINDFILE_FILE);
    }
}

/// "fnamemodify({fname}, {mods})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_fnamemodify(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut fbuf: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut fname = tv_get_string_chk(argvars).cast_mut();
        let mods = tv_get_string_buf_chk(argvars.add(1), buf.as_mut_ptr());
        if mods.is_null() || fname.is_null() {
            fname = ptr::null_mut();
        } else {
            #[cfg(windows)]
            {
                fbuf = path_to_slash_save(fname);
            }
            #[cfg(not(windows))]
            {
                fbuf = xstrdup(fname);
            }
            fname = fbuf;
            len = c_strlen(fname);
            if *mods != NUL {
                let mut usedlen: usize = 0;
                modify_fname(
                    mods.cast_mut(),
                    false,
                    &raw mut usedlen,
                    &raw mut fname,
                    &raw mut fbuf,
                    &raw mut len,
                    false,
                );
            }
        }

        (*rettv).v_type = VarType::VAR_STRING;
        if fname.is_null() {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            (*rettv).vval.v_string = xmemdupz(fname.cast(), len).cast();
        }
        xfree(fbuf.cast());
    }
}

/// Resolves the `[{winnr}[, {tabnr}[, {bufnr}]]]` arguments of `getcwd()`/`haslocaldir()` to the
/// requested scope and the scope objects to inspect.
unsafe fn getcwd_scope_args(
    argvars: *mut typval_T,
    default_scope: c_int,
    scope: *mut c_int,
    win: *mut *mut win_T,
    tp: *mut *mut tabpage_T,
    buf: *mut *mut buf_T,
) -> bool {
    unsafe {
        const kWinArg: usize = 0;
        const kTabArg: usize = 1;
        const kBufArg: usize = 2;
        let mut argv = [0 as c_int; 3];
        let mut argc: usize = 0;

        *scope = default_scope;
        *win = curwin;
        *tp = curtab;
        *buf = curbuf;

        while argc < 3 {
            // If there is no argument there are no more scopes after it, break out.
            if (*argvars.add(argc)).v_type == VarType::VAR_UNKNOWN {
                break;
            }
            if (*argvars.add(argc)).v_type != VarType::VAR_NUMBER {
                emsg(gettext(e_invarg.as_ptr()));
                return false;
            }
            argv[argc] = (*argvars.add(argc)).vval.v_number as c_int;
            // It is an error for the scope number to be less than `-1`.
            if argv[argc] < -1 {
                emsg(gettext(e_invarg.as_ptr()));
                return false;
            }
            argc += 1;
        }

        // An argument may only be -1 if all preceding arguments are -1: `(2, 3, -1)` is an error.
        let mut explicit_scope = false;
        for &val in &argv[..argc] {
            explicit_scope = explicit_scope || val >= 0;
            if explicit_scope && val < 0 {
                emsg(gettext(
                    c"E5001: Argument cannot be -1 if preceding argument is >= 0.".as_ptr(),
                ));
                return false;
            }
        }

        // Narrowest requested scope. Imagine X >= 0:
        match argc {
            0 => {}
            1 => {
                if argv[kWinArg] >= 0 {
                    *scope = kCdScopeWindow; // (X)
                } else {
                    *scope = kCdScopeTabpage; // (-1)
                }
            }
            2 => {
                if argv[kWinArg] >= 0 {
                    *scope = kCdScopeWindow; // (X, ...)
                } else if argv[kTabArg] >= 0 {
                    *scope = kCdScopeTabpage; // (-1, X)
                } else {
                    *scope = kCdScopeGlobal; // (-1, -1)
                }
            }
            3 => {
                if argv[kBufArg] >= 0 {
                    *scope = kCdScopeBuffer; // (-1, -1, X)
                } else {
                    *scope = kCdScopeGlobal; // (-1, -1, -1)
                }
            }
            _ => unreachable!(),
        }

        if *scope == kCdScopeBuffer {
            if argv[kWinArg] >= 0 || argv[kTabArg] >= 0 {
                emsg(gettext(
                    c"E5006: Window and tab scope must be -1 when using buffer scope".as_ptr(),
                ));
                return false;
            }
            if argv[kBufArg] > 0 {
                let mut err = Error {
                    err_type: ErrorType::kErrorTypeNone,
                    msg: ptr::null_mut(),
                };
                *buf = find_buffer_by_handle(argv[kBufArg], &raw mut err);
                if err.err_type != ErrorType::kErrorTypeNone {
                    emsg(gettext(c"E5007: Cannot find buffer number.".as_ptr()));
                    xfree(err.msg.cast());
                    return false;
                }
            }
        }

        // Find the tabpage by number.
        if argv[kTabArg] > 0 {
            *tp = find_tabpage(argv[kTabArg]);
            if (*tp).is_null() {
                emsg(gettext(c"E5000: Cannot find tab number.".as_ptr()));
                return false;
            }
        }

        // Find the window in `tp` by number.
        if argv[kWinArg] > 0 {
            *win = find_win_by_nr(argvars, *tp);
            if (*win).is_null() {
                emsg(gettext(c"E5002: Cannot find window number.".as_ptr()));
                return false;
            }
        }

        true
    }
}

/// `getcwd([{win}[, {tab}[, {buf}]]])` function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcwd(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut scope: c_int = kCdScopeInvalid;
        let mut win: *mut win_T = ptr::null_mut();
        let mut tp: *mut tabpage_T = ptr::null_mut();
        let mut buf: *mut buf_T = ptr::null_mut();

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        // Without arguments (kCdScopeInvalid), the effective working directory is returned.
        if !getcwd_scope_args(
            argvars,
            kCdScopeInvalid,
            &raw mut scope,
            &raw mut win,
            &raw mut tp,
            &raw mut buf,
        ) {
            return;
        }

        let mut from: *const c_char = ptr::null(); // The local directory to copy.
        let cwd: *mut c_char = xmalloc(MAXPATHL).cast();

        let mut cur_scope = scope;
        if cur_scope == kCdScopeWindow {
            from = (*win).w_localdir;
            if from.is_null() {
                cur_scope = kCdScopeTabpage;
            }
        }
        if cur_scope == kCdScopeTabpage {
            from = (*tp).tp_localdir;
            if from.is_null() {
                cur_scope = kCdScopeBuffer;
            }
        }
        if cur_scope == kCdScopeBuffer {
            // Only a buffer target reaches this with `scope` unchanged: a buffer belongs to no
            // particular window or tabpage, so it skips those and falls back to global dir.
            from = if scope == kCdScopeBuffer {
                (*buf).b_localdir
            } else {
                ptr::null()
            };
            if from.is_null() {
                cur_scope = kCdScopeGlobal;
            }
        }
        if cur_scope == kCdScopeGlobal {
            if !globaldir.is_null() {
                from = globaldir;
            } else {
                cur_scope = kCdScopeInvalid;
            }
        }
        if cur_scope == kCdScopeInvalid && os_dirname(cwd, MAXPATHL) == FAIL {
            from = c"".as_ptr(); // Return empty string on failure.
        }

        if !from.is_null() {
            xstrlcpy(cwd, from, MAXPATHL);
        }

        (*rettv).vval.v_string = xstrdup(cwd);
        #[cfg(windows)]
        slash_adjust((*rettv).vval.v_string);

        xfree(cwd.cast());
    }
}

/// "getfperm({fname})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getfperm(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut perm: *mut c_char = ptr::null_mut();
        let flags = [b'r' as c_char, b'w' as c_char, b'x' as c_char];

        let filename = tv_get_string(argvars);
        let file_perm = os_getperm(filename);
        if file_perm >= 0 {
            perm = xstrdup(c"---------".as_ptr());
            for i in 0..9 {
                if (file_perm & (1 << (8 - i))) != 0 {
                    *perm.add(i) = flags[i % 3];
                }
            }
        }
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = perm;
    }
}

/// "getfsize({fname})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getfsize(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let fname = tv_get_string(argvars);

        let mut file_info = FileInfo::default();
        if os_fileinfo(fname, &raw mut file_info) {
            let filesize = os_fileinfo_size(&raw const file_info);
            if s_isdir(file_info.stat.st_mode) {
                (*rettv).vval.v_number = 0;
            } else {
                (*rettv).vval.v_number = filesize as varnumber_T;

                // non-perfect check for overflow
                if (*rettv).vval.v_number as u64 != filesize {
                    (*rettv).vval.v_number = -2;
                }
            }
        } else {
            (*rettv).vval.v_number = -1;
        }
    }
}

/// "getftime({fname})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getftime(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let fname = tv_get_string(argvars);

        let mut file_info = FileInfo::default();
        if os_fileinfo(fname, &raw mut file_info) {
            (*rettv).vval.v_number = file_info.stat.st_mtim.tv_sec as varnumber_T;
        } else {
            (*rettv).vval.v_number = -1;
        }
    }
}

/// "getftype({fname})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getftype(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut type_str: *mut c_char = ptr::null_mut();

        let fname = tv_get_string(argvars);

        (*rettv).v_type = VarType::VAR_STRING;
        let mut file_info = FileInfo::default();
        if os_fileinfo_link(fname, &raw mut file_info) {
            let mode = file_info.stat.st_mode;
            let t = if s_isreg(mode) {
                c"file".as_ptr()
            } else if s_isdir(mode) {
                c"dir".as_ptr()
            } else if s_islnk(mode) {
                c"link".as_ptr()
            } else if s_isblk(mode) {
                c"bdev".as_ptr()
            } else if s_ischr(mode) {
                c"cdev".as_ptr()
            } else if s_isfifo(mode) {
                c"fifo".as_ptr()
            } else if s_issock(mode) {
                c"socket".as_ptr()
            } else {
                c"other".as_ptr()
            };
            type_str = xstrdup(t);
        }
        (*rettv).vval.v_string = type_str;
    }
}

/// "glob()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_glob(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut options: c_int = WILD_SILENT | WILD_USE_NL;
        let mut xpc = std::mem::MaybeUninit::<expand_T>::zeroed().assume_init();
        let mut error = false;

        // When the optional second argument is non-zero, don't remove matches
        // for 'wildignore' and don't put matches for 'suffixes' at the end.
        (*rettv).v_type = VarType::VAR_STRING;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            if tv_get_bool_chk(argvars.add(1), &raw mut error) != 0 {
                options |= WILD_KEEP_ALL;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                if tv_get_bool_chk(argvars.add(2), &raw mut error) != 0 {
                    tv_list_set_ret(rettv, ptr::null_mut());
                }
                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN
                    && tv_get_bool_chk(argvars.add(3), &raw mut error) != 0
                {
                    options |= WILD_ALLLINKS;
                }
            }
        }
        if !error {
            ExpandInit(&raw mut xpc);
            xpc.xp_context = EXPAND_FILES;
            if p_wic != 0 {
                options += WILD_ICASE;
            }
            if (*rettv).v_type == VarType::VAR_STRING {
                (*rettv).vval.v_string = ExpandOne(
                    &raw mut xpc,
                    tv_get_string(argvars).cast_mut(),
                    ptr::null_mut(),
                    options,
                    WILD_ALL,
                );
            } else {
                ExpandOne(
                    &raw mut xpc,
                    tv_get_string(argvars).cast_mut(),
                    ptr::null_mut(),
                    options,
                    WILD_ALL_KEEP,
                );
                tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);
                for i in 0..xpc.xp_numfiles {
                    tv_list_append_string((*rettv).vval.v_list, *xpc.xp_files.add(i as usize), -1);
                }
                ExpandCleanup(&raw mut xpc);
            }
        } else {
            (*rettv).vval.v_string = ptr::null_mut();
        }
    }
}

/// "globpath()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_globpath(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut flags: c_int = 0; // Flags for globpath.
        let mut error = false;

        // Return a string, or a list if the optional third argument is non-zero.
        (*rettv).v_type = VarType::VAR_STRING;

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            // When the optional second argument is non-zero, don't remove matches
            // for 'wildignore' and don't put matches for 'suffixes' at the end.
            if tv_get_bool_chk(argvars.add(2), &raw mut error) != 0 {
                flags |= WILD_KEEP_ALL;
            }

            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                if tv_get_bool_chk(argvars.add(3), &raw mut error) != 0 {
                    tv_list_set_ret(rettv, ptr::null_mut());
                }
                if (*argvars.add(4)).v_type != VarType::VAR_UNKNOWN
                    && tv_get_bool_chk(argvars.add(4), &raw mut error) != 0
                {
                    flags |= WILD_ALLLINKS;
                }
            }
        }

        let mut buf1 = [0 as c_char; NUMBUFLEN];
        let file = tv_get_string_buf_chk(argvars.add(1), buf1.as_mut_ptr());
        if !file.is_null() && !error {
            let mut ga = garray_T {
                ga_len: 0,
                ga_maxlen: 0,
                ga_itemsize: 0,
                ga_growsize: 0,
                ga_data: ptr::null_mut(),
            };
            ga_init(&raw mut ga, size_of::<*mut c_char>() as c_int, 10);
            globpath(
                tv_get_string(argvars).cast_mut(),
                file.cast_mut(),
                &raw mut ga,
                flags,
                false,
            );

            if (*rettv).v_type == VarType::VAR_STRING {
                (*rettv).vval.v_string = ga_concat_strings(&raw const ga, c"\n".as_ptr());
            } else {
                tv_list_alloc_ret(rettv, ga.ga_len as isize);
                for i in 0..ga.ga_len {
                    let s = *(ga.ga_data.cast::<*const c_char>()).add(i as usize);
                    tv_list_append_string((*rettv).vval.v_list, s, -1);
                }
            }

            ga_clear_strings(&raw mut ga);
        } else {
            (*rettv).vval.v_string = ptr::null_mut();
        }
    }
}

/// "glob2regpat()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_glob2regpat(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let pat = tv_get_string_chk(argvars); // NULL on type error

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = if pat.is_null() {
            ptr::null_mut()
        } else {
            file_pat_to_reg_pat(pat, ptr::null(), ptr::null_mut(), 0)
        };
    }
}

/// `haslocaldir([{win}[, {tab}[, {buf}]]])` function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_haslocaldir(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut scope: c_int = kCdScopeWindow;
        let mut win: *mut win_T = ptr::null_mut();
        let mut tp: *mut tabpage_T = ptr::null_mut();
        let mut buf: *mut buf_T = ptr::null_mut();

        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        // Without arguments, default to window scope.
        if !getcwd_scope_args(
            argvars,
            kCdScopeWindow,
            &raw mut scope,
            &raw mut win,
            &raw mut tp,
            &raw mut buf,
        ) {
            return;
        }

        match scope {
            kCdScopeWindow => {
                (*rettv).vval.v_number = varnumber_T::from(!(*win).w_localdir.is_null());
            }
            kCdScopeBuffer => {
                (*rettv).vval.v_number = varnumber_T::from(!(*buf).b_localdir.is_null());
            }
            kCdScopeTabpage => {
                (*rettv).vval.v_number = varnumber_T::from(!(*tp).tp_localdir.is_null());
            }
            kCdScopeGlobal => {
                // The global scope never has a local directory
            }
            _ => std::process::abort(),
        }
    }
}

/// "isabsolutepath()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_isabsolutepath(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(path_is_absolute(tv_get_string(argvars)));
    }
}

/// "isdirectory()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_isdirectory(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(os_isdir(tv_get_string(argvars)));
    }
}

/// "mkdir()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_mkdir(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut prot: c_int = 0o755;

        (*rettv).vval.v_number = varnumber_T::from(FAIL);
        if check_secure() {
            return;
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let dir = tv_get_string_buf(argvars, buf.as_mut_ptr());
        if *dir == NUL {
            return;
        }

        if *path_tail(dir) == NUL {
            // Remove trailing slashes.
            *path_tail_with_sep(dir.cast_mut()) = NUL;
        }

        let mut defer = false;
        let mut defer_recurse = false;
        let mut created: *mut c_char = ptr::null_mut();
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                prot = tv_get_number_chk(argvars.add(2), ptr::null_mut()) as c_int;
                if prot == -1 {
                    return;
                }
            }
            let arg2 = tv_get_string(argvars.add(1));
            defer = !vim_strchr(arg2, b'D' as c_int).is_null();
            defer_recurse = !vim_strchr(arg2, b'R' as c_int).is_null();
            if (defer || defer_recurse) && !can_add_defer() {
                return;
            }

            if !vim_strchr(arg2, b'p' as c_int).is_null() {
                let mut failed_dir: *mut c_char = ptr::null_mut();
                let ret = os_mkdir_recurse(
                    dir,
                    prot,
                    &raw mut failed_dir,
                    if defer || defer_recurse {
                        &raw mut created
                    } else {
                        ptr::null_mut()
                    },
                );
                if ret != 0 {
                    semsg!(gettext(e_mkdir.as_ptr()), failed_dir, uv_strerror(ret));
                    xfree(failed_dir.cast());
                    (*rettv).vval.v_number = varnumber_T::from(FAIL);
                    return;
                }
                (*rettv).vval.v_number = varnumber_T::from(OK);
            }
        }
        if (*rettv).vval.v_number == varnumber_T::from(FAIL) {
            (*rettv).vval.v_number = varnumber_T::from(vim_mkdir_emsg(dir, prot));
        }

        // Handle "D" and "R": deferred deletion of the created directory.
        if (*rettv).vval.v_number == varnumber_T::from(OK)
            && created.is_null()
            && (defer || defer_recurse)
        {
            created = FullName_save(dir, false);
        }
        if !created.is_null() {
            let mut tv = [
                typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval { v_string: created },
                },
                typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval {
                        v_string: xstrdup(if defer_recurse {
                            c"rf".as_ptr()
                        } else {
                            c"d".as_ptr()
                        }),
                    },
                },
            ];
            add_defer(c"delete".as_ptr().cast_mut(), 2, tv.as_mut_ptr());
        }
    }
}

/// "pathshorten()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_pathshorten(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut trim_len: c_int = 1;

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            trim_len = tv_get_number(argvars.add(1)) as c_int;
            if trim_len < 1 {
                trim_len = 1;
            }
        }

        (*rettv).v_type = VarType::VAR_STRING;
        let p = tv_get_string_chk(argvars);
        if p.is_null() {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            (*rettv).vval.v_string = xstrdup(p);
            shorten_dir_len((*rettv).vval.v_string, trim_len);
        }
    }
}

/// Evaluate "expr" (= "context") for readdir().
unsafe extern "C" fn readdir_checkitem(context: *mut c_void, name: *const c_char) -> varnumber_T {
    unsafe {
        let expr: *mut typval_T = context.cast();
        let tv_init = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_number: 0 },
        };
        let mut argv = [tv_init; 2];
        let mut retval: varnumber_T = 0;
        let mut error = false;

        if (*expr).v_type == VarType::VAR_UNKNOWN {
            return 1;
        }

        let mut save_val = tv_init;
        prepare_vimvar(VV_VAL, &raw mut save_val);
        set_vim_var_string(VV_VAL, name, -1);
        argv[0].v_type = VarType::VAR_STRING;
        argv[0].vval.v_string = name.cast_mut();

        let mut rettv = tv_init;
        if eval_expr_typval(expr, false, argv.as_mut_ptr(), 1, &raw mut rettv) != FAIL {
            retval = tv_get_number_chk(&raw const rettv, &raw mut error);
            if error {
                retval = -1;
            }

            tv_clear(&raw mut rettv);
        }

        set_vim_var_string(VV_VAL, ptr::null(), 0);
        restore_vimvar(VV_VAL, &raw mut save_val);
        retval
    }
}

/// "readdir()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_readdir(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN);
        if check_secure() {
            return;
        }

        let path = tv_get_string(argvars);
        let expr = argvars.add(1);
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        let ret = readdir_core(&raw mut ga, path, expr.cast(), Some(readdir_checkitem));
        if ret == OK && ga.ga_len > 0 {
            for i in 0..ga.ga_len {
                let p = *(ga.ga_data.cast::<*const c_char>()).add(i as usize);
                tv_list_append_string((*rettv).vval.v_list, p, -1);
            }
        }
        ga_clear_strings(&raw mut ga);
    }
}

/// Read blob from file "fd".
/// Caller has allocated a blob in "rettv".
unsafe fn read_blob(
    fd: *mut FILE,
    rettv: *mut typval_T,
    mut offset: off_T,
    size_arg: off_T,
) -> c_int {
    unsafe {
        let blob = (*rettv).vval.v_blob;
        let mut file_info = FileInfo::default();
        if !os_fileinfo_fd(c_fileno(fd), &raw mut file_info) {
            return FAIL; // can't read the file, error
        }

        let mut size = size_arg;
        let file_size = os_fileinfo_size(&raw const file_info) as off_T;
        let whence = if offset >= 0 {
            // The size defaults to the whole file.  If a size is given it is
            // limited to not go past the end of the file.
            if size == -1 || (size > file_size - offset && !s_ischr(file_info.stat.st_mode)) {
                // size may become negative, checked below
                size = os_fileinfo_size(&raw const file_info) as off_T - offset;
            }
            SEEK_SET
        } else {
            // limit the offset to not go before the start of the file
            if -offset > file_size && !s_ischr(file_info.stat.st_mode) {
                offset = -file_size;
            }
            // Size defaults to reading until the end of the file.
            if size == -1 || size > -offset {
                size = -offset;
            }
            SEEK_END
        };
        if size <= 0 {
            return OK;
        }
        if offset != 0 && vim_fseek(fd, offset, whence) != 0 {
            return OK;
        }

        ga_grow(&raw mut (*blob).bv_ga, size as c_int);
        (*blob).bv_ga.ga_len = size as c_int;
        if fread((*blob).bv_ga.ga_data, 1, (*blob).bv_ga.ga_len as usize, fd)
            < (*blob).bv_ga.ga_len as usize
        {
            // An empty blob is returned on error.
            tv_blob_free((*rettv).vval.v_blob);
            (*rettv).vval.v_blob = ptr::null_mut();
            return FAIL;
        }
        OK
    }
}

/// "readfile()" or "readblob()" function
unsafe fn read_file_or_blob(argvars: *mut typval_T, rettv: *mut typval_T, always_blob: bool) {
    unsafe {
        let mut binary = false;
        let mut blob = always_blob;
        let mut buf = [0 as c_char; (IOSIZE / 256) * 256]; // rounded to avoid odd + 1
        let io_size = buf.len();
        let mut prev: *mut c_char = ptr::null_mut(); // previously read bytes, if any
        let mut prevlen: isize = 0; // length of data in prev
        let mut prevsize: isize = 0; // size of prev buffer
        let mut maxline: i64 = MAXLNUM;
        let mut offset: off_T = 0;
        let mut size: off_T = -1;

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            if always_blob {
                offset = tv_get_number(argvars.add(1));
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    size = tv_get_number(argvars.add(2));
                }
            } else {
                let arg1 = CStr::from_ptr(tv_get_string(argvars.add(1))).to_bytes();
                if arg1 == b"b" {
                    binary = true;
                } else if arg1 == b"B" {
                    blob = true;
                }
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    maxline = tv_get_number(argvars.add(2));
                }
            }
        }

        if blob {
            tv_blob_alloc_ret(rettv);
        } else {
            tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN);
        }

        // Always open the file in binary mode, library functions have a mind of
        // their own about CR-LF conversion.
        let fname = tv_get_string(argvars);

        if os_isdir(fname) {
            semsg!(gettext(e_isadir2.as_ptr()), fname);
            return;
        }
        if *fname == NUL {
            semsg!(gettext(e_notopen.as_ptr()), gettext(c"<empty>".as_ptr()));
            return;
        }
        let fd = os_fopen(fname, c"rb".as_ptr());
        if fd.is_null() {
            semsg!(gettext(e_notopen.as_ptr()), fname);
            return;
        }

        if blob {
            if read_blob(fd, rettv, offset, size) == FAIL {
                semsg!(gettext(e_cant_read_file_str.as_ptr()), fname);
            }
            fclose(fd);
            return;
        }

        let l = (*rettv).vval.v_list;
        let buf_ptr = buf.as_mut_ptr();

        while maxline < 0 || i64::from(tv_list_len(l)) < maxline {
            let mut readlen = fread(buf_ptr.cast(), 1, io_size, fd) as isize;

            let mut p = buf_ptr; // Position in buf.
            let mut start = buf_ptr; // Start of current line.
            while (p as usize) < (buf_ptr.offset(readlen) as usize)
                || (readlen <= 0 && (prevlen > 0 || binary))
            {
                if readlen <= 0 || *p == b'\n' as c_char {
                    let mut len = (p as usize).wrapping_sub(start as usize);

                    // Finished a line.  Remove CRs before NL.
                    if readlen > 0 && !binary {
                        while len > 0 && *start.add(len - 1) == b'\r' as c_char {
                            len -= 1;
                        }
                        // removal may cross back to the "prev" string
                        if len == 0 {
                            while prevlen > 0 && *prev.offset(prevlen - 1) == b'\r' as c_char {
                                prevlen -= 1;
                            }
                        }
                    }
                    let s: *mut c_char = if prevlen == 0 {
                        debug_assert!(len < c_int::MAX as usize);
                        xmemdupz(start.cast(), len).cast()
                    } else {
                        // Change "prev" buffer to be the right size.  This way
                        // the bytes are only copied once, and very long lines are
                        // allocated only once.
                        let new_s: *mut c_char =
                            xrealloc(prev.cast(), prevlen as usize + len + 1).cast();
                        ptr::copy_nonoverlapping(start, new_s.offset(prevlen), len);
                        *new_s.add(prevlen as usize + len) = NUL;
                        prev = ptr::null_mut(); // the list will own the string
                        prevlen = 0;
                        prevsize = 0;
                        new_s
                    };

                    tv_list_append_owned_tv(
                        l,
                        typval_T {
                            v_type: VarType::VAR_STRING,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval { v_string: s },
                        },
                    );

                    start = p.add(1); // Step over newline.
                    if maxline < 0 {
                        if i64::from(tv_list_len(l)) > -maxline {
                            debug_assert_eq!(i64::from(tv_list_len(l)), 1 + (-maxline));
                            tv_list_item_remove(l, tv_list_first(l));
                        }
                    } else if i64::from(tv_list_len(l)) >= maxline {
                        debug_assert_eq!(i64::from(tv_list_len(l)), maxline);
                        break;
                    }
                    if readlen <= 0 {
                        break;
                    }
                } else if *p == NUL {
                    *p = b'\n' as c_char;
                } else if *p as u8 == 0xbf && !binary {
                    // Check for utf8 "bom"; U+FEFF is encoded as EF BB BF.  Do this
                    // when finding the BF and check the previous two bytes.
                    let back1 = if (p as usize) >= (buf_ptr.add(1) as usize) {
                        *p.sub(1)
                    } else if prevlen >= 1 {
                        *prev.offset(prevlen - 1)
                    } else {
                        NUL
                    };
                    let back2 = if (p as usize) >= (buf_ptr.add(2) as usize) {
                        *p.sub(2)
                    } else if p == buf_ptr.add(1) && prevlen >= 1 {
                        *prev.offset(prevlen - 1)
                    } else if prevlen >= 2 {
                        *prev.offset(prevlen - 2)
                    } else {
                        NUL
                    };

                    if back2 as u8 == 0xef && back1 as u8 == 0xbb {
                        let mut dest = p.sub(2);

                        // Usually a BOM is at the beginning of a file, and so at
                        // the beginning of a line; then we can just step over it.
                        if start == dest {
                            start = p.add(1);
                        } else {
                            // have to shuffle buf to close gap
                            let mut adjust_prevlen: isize = 0;

                            if (dest as usize) < (buf_ptr as usize) {
                                // adjust_prevlen must be 1 or 2.
                                adjust_prevlen =
                                    (buf_ptr as usize).wrapping_sub(dest as usize) as isize;
                                dest = buf_ptr;
                            }
                            let p_off = (p as usize).wrapping_sub(buf_ptr as usize) as isize;
                            if readlen > p_off + 1 {
                                ptr::copy(p.add(1), dest, (readlen - p_off - 1) as usize);
                            }
                            readlen -= 3 - adjust_prevlen;
                            prevlen -= adjust_prevlen;
                            p = dest.sub(1);
                        }
                    }
                }
                p = p.add(1);
            }

            if (maxline >= 0 && i64::from(tv_list_len(l)) >= maxline) || readlen <= 0 {
                break;
            }
            if (start as usize) < (p as usize) {
                let part_len = (p as usize).wrapping_sub(start as usize) as isize;
                // There's part of a line in buf, store it in "prev".
                if part_len + prevlen >= prevsize {
                    if prevsize == 0 {
                        prevsize = part_len;
                    } else {
                        let grow50pc = (prevsize * 3) / 2;
                        let growmin = part_len * 2 + prevlen;
                        prevsize = if grow50pc > growmin {
                            grow50pc
                        } else {
                            growmin
                        };
                    }
                    prev = xrealloc(prev.cast(), prevsize as usize).cast();
                }
                // Add the line part to end of "prev".
                ptr::copy(start, prev.offset(prevlen), part_len as usize);
                prevlen += part_len;
            }
        }

        xfree(prev.cast());
        fclose(fd);
    }
}

/// "readblob()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_readblob(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }

        read_file_or_blob(argvars, rettv, true);
    }
}

/// "readfile()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_readfile(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }

        read_file_or_blob(argvars, rettv, false);
    }
}

/// "rename({from}, {to})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_rename(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            (*rettv).vval.v_number = -1;
        } else {
            let mut buf = [0 as c_char; NUMBUFLEN];
            (*rettv).vval.v_number = varnumber_T::from(vim_rename(
                tv_get_string(argvars),
                tv_get_string_buf(argvars.add(1), buf.as_mut_ptr()),
            ));
        }
    }
}

/// "resolve()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_resolve(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let fname = tv_get_string(argvars);
        #[cfg(windows)]
        {
            let mut v = os_resolve_shortcut(fname);
            if v.is_null() && os_is_reparse_point_include(fname) {
                v = os_realpath(fname, ptr::null_mut(), MAXPATHL + 1);
            }
            (*rettv).vval.v_string = if v.is_null() { xstrdup(fname) } else { v };
        }
        #[cfg(not(windows))]
        {
            let mut is_relative_to_current = false;
            let mut has_trailing_pathsep = false;
            let mut limit: c_int = 100;

            let mut p = xstrdup(fname);

            if *p == b'.' as c_char
                && (vim_ispathsep(*p.add(1) as u8 as c_int)
                    || (*p.add(1) == b'.' as c_char && vim_ispathsep(*p.add(2) as u8 as c_int)))
            {
                is_relative_to_current = true;
            }

            let mut len = c_strlen(p) as isize;
            if len > 1 && after_pathsep(p, p.offset(len)) != 0 {
                has_trailing_pathsep = true;
                *p.offset(len - 1) = NUL; // The trailing slash breaks readlink().
            }

            let mut q = path_next_component(p).cast_mut();
            let mut remain: *mut c_char = ptr::null_mut();
            if *q != NUL {
                // Separate the first path component in "p", and keep the
                // remainder (beginning with the path separator).
                remain = xstrdup(q.sub(1));
                *q.sub(1) = NUL;
            }

            let buf: *mut c_char = xmallocz(MAXPATHL).cast();

            let mut cpy: *mut c_char;
            loop {
                loop {
                    len = readlink(p, buf, MAXPATHL);
                    if len <= 0 {
                        break;
                    }
                    *buf.offset(len) = NUL;

                    let old_limit = limit;
                    limit -= 1;
                    if old_limit == 0 {
                        xfree(p.cast());
                        xfree(remain.cast());
                        emsg(gettext(c"E655: Too many symbolic links (cycle?)".as_ptr()));
                        (*rettv).vval.v_string = ptr::null_mut();
                        xfree(buf.cast());
                        return;
                    }

                    // Ensure that the result will have a trailing path separator
                    // if the argument has one.
                    if remain.is_null() && has_trailing_pathsep {
                        add_pathsep(buf);
                    }

                    // Separate the first path component in the link value and
                    // concatenate the remainders.
                    let start_comp = if vim_ispathsep(*buf as u8 as c_int) {
                        buf.add(1)
                    } else {
                        buf
                    };
                    q = path_next_component(start_comp).cast_mut();
                    if *q != NUL {
                        cpy = remain;
                        remain = if !remain.is_null() {
                            concat_str(q.sub(1), remain)
                        } else {
                            xstrdup(q.sub(1))
                        };
                        xfree(cpy.cast());
                        *q.sub(1) = NUL;
                    }

                    q = path_tail(p);
                    if (q as usize) > (p as usize) && *q == NUL {
                        // Ignore trailing path separator.
                        *q.sub(1) = NUL;
                        q = path_tail(p);
                    }
                    if (q as usize) > (p as usize) && !path_is_absolute(buf) {
                        // Symlink is relative to directory of argument. Replace the
                        // symlink with the resolved name in the same directory.
                        let p_len = c_strlen(p);
                        let buf_len = c_strlen(buf);
                        p = xrealloc(p.cast(), p_len + buf_len + 1).cast();
                        ptr::copy_nonoverlapping(buf, path_tail(p), buf_len + 1);
                    } else {
                        xfree(p.cast());
                        p = xstrdup(buf);
                    }
                }

                if remain.is_null() {
                    break;
                }

                // Append the first path component of "remain" to "p".
                q = path_next_component(remain.add(1)).cast_mut();
                len = (q as usize).wrapping_sub(remain as usize) as isize - isize::from(*q != NUL);
                let p_len = c_strlen(p);
                cpy = xmallocz(p_len + len as usize).cast();
                ptr::copy_nonoverlapping(p, cpy, p_len + 1);
                xstrlcat(cpy.add(p_len), remain, len as usize + 1);
                xfree(p.cast());
                p = cpy;

                // Shorten "remain".
                if *q != NUL {
                    let rem_tail = q.sub(1);
                    let rem_len = c_strlen(rem_tail) + 1;
                    ptr::copy(rem_tail, remain, rem_len);
                } else {
                    xfree(remain.cast());
                    remain = ptr::null_mut();
                }
            }

            // If the result is a relative path name, make it explicitly relative to
            // the current directory if and only if the argument had this form.
            if !vim_ispathsep(*p as u8 as c_int) {
                if is_relative_to_current
                    && *p != NUL
                    && !(*p == b'.' as c_char
                        && (*p.add(1) == NUL
                            || vim_ispathsep(*p.add(1) as u8 as c_int)
                            || (*p.add(1) == b'.' as c_char
                                && (*p.add(2) == NUL || vim_ispathsep(*p.add(2) as u8 as c_int)))))
                {
                    // Prepend "./".
                    cpy = concat_str(c"./".as_ptr(), p);
                    xfree(p.cast());
                    p = cpy;
                } else if !is_relative_to_current {
                    // Strip leading "./".
                    q = p;
                    while *q == b'.' as c_char && vim_ispathsep(*q.add(1) as u8 as c_int) {
                        q = q.add(2);
                    }
                    if (q as usize) > (p as usize) {
                        let src_ptr = p.add(2);
                        let move_len = c_strlen(src_ptr) + 1;
                        ptr::copy(src_ptr, p, move_len);
                    }
                }
            }

            // Ensure that the result will have no trailing path separator
            // if the argument had none.  But keep "/" or "//".
            if !has_trailing_pathsep {
                q = p.add(c_strlen(p));
                if after_pathsep(p, q) != 0 {
                    *path_tail_with_sep(p) = NUL;
                }
            }

            (*rettv).vval.v_string = p;
            xfree(buf.cast());
        }

        simplify_filename((*rettv).vval.v_string);
    }
}

/// "simplify()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_simplify(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let p = tv_get_string(argvars);
        (*rettv).vval.v_string = xstrdup(p);
        simplify_filename((*rettv).vval.v_string); // Simplify in place.
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "tempname()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tempname(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = vim_tempname();
    }
}

/// Write "list" of strings to file "fp".
unsafe fn write_list(fp: *mut FileDescriptor, list: *const list_T, binary: bool) -> bool {
    unsafe {
        let mut error: c_int = 0;
        let mut li = tv_list_first(list);
        'outer: while !li.is_null() {
            let s = tv_get_string_chk(&raw const (*li).li_tv);
            if s.is_null() {
                return false;
            }
            let mut hunk_start = s;
            let mut p = hunk_start;
            loop {
                if *p == NUL || *p == NL {
                    if p != hunk_start {
                        let written = file_write(
                            fp,
                            hunk_start,
                            (p as usize).wrapping_sub(hunk_start as usize),
                        );
                        if written < 0 {
                            error = written as c_int;
                            break 'outer;
                        }
                    }
                    if *p == NUL {
                        break;
                    } else {
                        hunk_start = p.add(1);
                        let nul_byte: c_char = NUL;
                        let written = file_write(fp, &raw const nul_byte, 1);
                        if written < 0 {
                            error = written as c_int;
                            break;
                        }
                    }
                }
                p = p.add(1);
            }
            if !binary || !(*li).li_next.is_null() {
                let nl_byte: c_char = NL;
                let written = file_write(fp, &raw const nl_byte, 1);
                if written < 0 {
                    error = written as c_int;
                    break 'outer;
                }
            }
            li = (*li).li_next;
        }
        if error == 0 {
            error = file_flush(fp);
            if error == 0 {
                return true;
            }
        }
        semsg!(
            gettext(E_ERROR_WHILE_WRITING_STR.as_ptr()),
            uv_strerror(error),
        );
        false
    }
}

/// Write a blob to file with descriptor `fp`.
unsafe fn write_data(fp: *mut FileDescriptor, data: *const c_char, len: usize) -> bool {
    unsafe {
        if len > 0 {
            let written = file_write(fp, data, len);
            if written < len as isize {
                let error = written as c_int;
                semsg!(
                    gettext(E_ERROR_WHILE_WRITING_STR.as_ptr()),
                    uv_strerror(error),
                );
                return false;
            }
        }
        let error = file_flush(fp);
        if error != 0 {
            semsg!(
                gettext(E_ERROR_WHILE_WRITING_STR.as_ptr()),
                uv_strerror(error),
            );
            return false;
        }
        true
    }
}

unsafe fn write_blob(fp: *mut FileDescriptor, blob: *const blob_T) -> bool {
    unsafe { write_data(fp, (*blob).bv_ga.ga_data.cast(), tv_blob_len(blob) as usize) }
}

unsafe fn write_string(fp: *mut FileDescriptor, data: *const c_char) -> bool {
    unsafe { write_data(fp, data, c_strlen(data)) }
}

/// "writefile()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_writefile(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;

        if check_secure() {
            return;
        }

        if (*argvars).v_type == VarType::VAR_LIST {
            let mut li = tv_list_first((*argvars).vval.v_list);
            while !li.is_null() {
                if !tv_check_str_or_nr(&raw const (*li).li_tv) {
                    return;
                }
                li = (*li).li_next;
            }
        } else if (*argvars).v_type != VarType::VAR_BLOB
            && !((*argvars).v_type == VarType::VAR_STRING
                && (script_is_lua(current_sctx.sc_sid) || current_sctx.sc_sid == SID_API_CLIENT))
        {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                gettext(c"writefile() first argument must be a List or a Blob".as_ptr()),
            );
            return;
        }

        let mut binary = false;
        let mut append = false;
        let mut defer = false;
        let mut do_fsync = p_fs != 0;
        let mut mkdir_p = false;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let flags = tv_get_string_chk(argvars.add(2));
            if flags.is_null() {
                return;
            }
            let mut p = flags;
            while *p != NUL {
                match *p as u8 {
                    b'b' => binary = true,
                    b'a' => append = true,
                    b'D' => defer = true,
                    b's' => do_fsync = true,
                    b'S' => do_fsync = false,
                    b'p' => mkdir_p = true,
                    _ => {
                        semsg!(gettext(c"E5060: Unknown flag: %s".as_ptr()), p);
                        return;
                    }
                }
                p = p.add(1);
            }
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let fname = tv_get_string_buf_chk(argvars.add(1), buf.as_mut_ptr());
        if fname.is_null() {
            return;
        }

        if defer && !can_add_defer() {
            return;
        }

        let mut fp = std::mem::MaybeUninit::<FileDescriptor>::zeroed().assume_init();
        if *fname == NUL {
            emsg(gettext(
                c"E482: Can't open file with an empty name".as_ptr(),
            ));
        } else {
            let open_flags = (if append {
                K_FILE_APPEND
            } else {
                K_FILE_TRUNCATE
            }) | (if mkdir_p {
                K_FILE_MK_DIR
            } else {
                K_FILE_CREATE
            }) | K_FILE_CREATE;
            let mut error = file_open(&raw mut fp, fname, open_flags, 0o666);
            if error != 0 {
                semsg!(
                    gettext(c"E482: Can't open file %s for writing: %s".as_ptr()),
                    fname,
                    uv_strerror(error),
                );
            } else {
                if defer {
                    let mut tv = typval_T {
                        v_type: VarType::VAR_STRING,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval {
                            v_string: FullName_save(fname, false),
                        },
                    };
                    add_defer(c"delete".as_ptr().cast_mut(), 1, &raw mut tv);
                }

                let write_ok = if (*argvars).v_type == VarType::VAR_BLOB {
                    (*argvars).vval.v_blob.is_null()
                        || write_blob(&raw mut fp, (*argvars).vval.v_blob)
                } else if (*argvars).v_type == VarType::VAR_STRING {
                    write_string(&raw mut fp, (*argvars).vval.v_string)
                } else {
                    write_list(&raw mut fp, (*argvars).vval.v_list, binary)
                };
                if write_ok {
                    (*rettv).vval.v_number = 0;
                }
                error = file_close(&raw mut fp, do_fsync);
                if error != 0 {
                    semsg!(
                        gettext(c"E80: Error when closing file %s: %s".as_ptr()),
                        fname,
                        uv_strerror(error),
                    );
                }
            }
        }
    }
}

/// "browse(save, title, initdir, default)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_browse(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_string = ptr::null_mut();
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "browsedir(title, initdir)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_browsedir(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    unsafe {
        f_browse(argvars, rettv, fptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_mode_predicates_identify_file_types() {
        assert!(s_isreg(S_IFREG_VAL | 0o644));
        assert!(!s_isdir(S_IFREG_VAL | 0o644));
        assert!(s_isdir(S_IFDIR_VAL | 0o755));
        assert!(s_islnk(S_IFLNK_VAL | 0o777));
        assert!(s_ischr(S_IFCHR_VAL | 0o600));
        assert!(s_isfifo(S_IFIFO_VAL | 0o600));
    }
}
