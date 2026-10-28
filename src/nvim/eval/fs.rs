//! Filesystem related builtin functions (`src/nvim/eval/fs.c`).

use crate::api::private::helpers::{MAXLNUM, SID_API_CLIENT, find_buffer_by_handle};
use crate::api::private::validate::{Error, ErrorType};
use crate::buffer::buf_T;
use crate::cmdexpand::{
    EXPAND_FILES, ExpandOne, WILD_ALL, WILD_ALL_KEEP, WILD_ALLLINKS, WILD_ICASE, WILD_KEEP_ALL,
    WILD_SILENT, WILD_USE_NL, globpath,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypvalVval, VAR_UNLOCKED, VarType, blob_T, list_T, tv_blob_alloc_ret,
    tv_blob_len, tv_check_for_nonempty_string_arg, tv_check_for_string_arg, tv_check_str_or_nr,
    tv_clear, tv_get_bool_chk, tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf,
    tv_get_string_buf_chk, tv_get_string_chk, tv_list_alloc_ret, tv_list_append_string,
    tv_list_first, tv_list_item_remove, tv_list_len, tv_list_set_ret, typval_T, varnumber_T,
};
use crate::eval::userfunc::{add_defer, can_add_defer};
use crate::eval::vars::{VV_VAL, prepare_vimvar, restore_vimvar, set_vim_var_string};
use crate::eval::window::find_win_by_nr;
use crate::eval::{do_string_sub, eval_expr_typval};
use crate::event::r#loop::uv_strerror;
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{do_chdir, vim_mkdir_emsg};
use crate::file_search::{FINDFILE_DIR, FINDFILE_FILE, ff_search_ctx_T, find_file_in_path_option};
#[cfg(not(windows))]
use crate::fileio::readlink;
use crate::fileio::{
    FILE, SEEK_END, SEEK_SET, delete_recursive, fclose, file_pat_to_reg_pat, fileno, fread, off_T,
    readdir_core, vim_copyfile, vim_fseek, vim_rename, vim_tempname,
};
use crate::globals::*;
use crate::math::{FAIL, OK};
use crate::mbyte::{utf_head_off, utfc_ptr2len};
use crate::memory::{cstr_from_bytes, cstr_to_cstring, xstrlcpy};
use crate::message::{emsg, semsg};
use crate::option_vars::*;
use crate::os::env::{expand_T, expand_env_save, home_replace};
use crate::os::fileio::{
    FileDescriptor, K_FILE_APPEND, K_FILE_CREATE, K_FILE_MK_DIR, K_FILE_TRUNCATE, file_close,
    file_flush, file_open, file_write,
};
use crate::os::fs::{
    FileInfo, MAXPATHL, os_can_exe, os_dirname, os_file_is_readable, os_file_is_writable,
    os_fileinfo, os_fileinfo_fd, os_fileinfo_link, os_fileinfo_size, os_fileinfo2, os_fopen,
    os_getperm, os_isdir, os_mkdir_recurse, os_remove, os_rmdir, s_isblk, s_isdir, s_isfifo,
    s_islnk, s_isreg, s_issock,
};
#[cfg(windows)]
use crate::os::fs::{os_is_reparse_point_include, os_realpath_cstr, os_resolve_shortcut};
#[cfg(unix)]
use crate::path::path_next_component;
use crate::path::{
    FullName_save, add_pathsep, after_pathsep, path_cmp, path_is_absolute, path_tail,
    path_tail_with_sep, shorten_dir_len, simplify_filename, vim_isAbsName, vim_ispathsep,
};
#[cfg(windows)]
use crate::path::{path_to_slash_save, slash_adjust};
use crate::runtime::script_is_lua;
use crate::strings::{gettext, vim_strchr, vim_strsave_shellescape};
use crate::window::{find_tabpage, tabpage_T, win_T};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::api::vim::kCdScopeGlobal;
use crate::charset::{NL, NUL};
use crate::regexp::NUMBUFLEN;

pub(crate) const K_LIST_LEN_UNKNOWN: isize = -1;

pub(crate) const VALID_PATH: c_int = 1;
pub(crate) const VALID_HEAD: c_int = 2;

const kCdScopeInvalid: c_int = -1;
pub(crate) const kCdScopeBuffer: c_int = 0;
pub(crate) const kCdScopeWindow: c_int = 1;
pub(crate) const kCdScopeTabpage: c_int = 2;

const S_IFMT_MASK: u64 = 0xF000;
const S_IFCHR_VAL: u64 = 0x2000;

const E_ERROR_WHILE_WRITING_STR: &CStr = c"E80: Error while writing: %s";

#[inline]
fn s_ischr(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFCHR_VAL
}

/// Adjust a filename, according to a string of modifiers.
/// *fnamep must be NUL terminated when called.  When returning, the length is
/// determined by *fnamelen.
/// Returns VALID_ flags or -1 for failure.
#[inline]
fn set_fname_buf(
    fnamep: *mut *mut c_char,
    bufp: &mut Option<Vec<c_char>>,
    new_vec: Vec<c_char>,
) {
    unsafe {
        *bufp = Some(new_vec);
        *fnamep = bufp.as_mut().unwrap().as_mut_ptr();
    }
}

#[inline]
fn cstr_into_char_vec(s: std::ffi::CString) -> Vec<c_char> {
    s.into_bytes_with_nul()
        .into_iter()
        .map(|b| b as c_char)
        .collect()
}

/// When there is an error, *fnamep is set to NULL.
pub fn modify_fname(
    src: impl crate::strings::IntoCCharPtr,
    tilde_file: bool,
    usedlen: &mut usize,
    fnamep: *mut *mut c_char,
    bufp: &mut Option<Vec<c_char>>,
    fnamelen: &mut usize,
    use_shellslash: bool,
) -> c_int {
    let src = src.into_c_char_ptr().cast_mut();
    unsafe {
        let mut valid: c_int = 0;
        let mut s: *mut c_char = ptr::null_mut();
        let mut p: *mut c_char;
        let mut pbuf: Option<Vec<c_char>>;
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
                    let exp = expand_env_save(*fnamep);
                    set_fname_buf(fnamep, bufp, cstr_into_char_vec(exp));
                }

                // When "/." or "/.." is used: force expansion to get rid of it.
                p = *fnamep;
                while !crate::memory::cstr_is_empty(p) {
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
                if !crate::memory::cstr_is_empty(p)
                    || !vim_isAbsName(*fnamep)
                    || (cfg!(windows)
                        && (*(*fnamep) == b'/' as c_char || *(*fnamep) == b'\\' as c_char))
                {
                    let Some(full) = FullName_save(*fnamep, !crate::memory::cstr_is_empty(p)) else {
                        *fnamep = ptr::null_mut();
                        *bufp = None;
                        return -1;
                    };
                    set_fname_buf(fnamep, bufp, cstr_into_char_vec(full));
                }

                // Append a path separator to a directory.
                if os_isdir(*fnamep) {
                    // Make room for one or two extra characters.
                    let cur_len = crate::memory::cstr_len(*fnamep);
                    let mut v = vec![0 as c_char; cur_len + 3];
                    ptr::copy_nonoverlapping(*fnamep, v.as_mut_ptr(), cur_len + 1);
                    set_fname_buf(fnamep, bufp, v);
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
                pbuf = None;
                // Need full path first (use expand_env() to remove a "~/")
                if !has_fullname && !has_homerelative {
                    if *(*fnamep) == b'~' as c_char {
                        let exp = cstr_into_char_vec(expand_env_save(*fnamep));
                        p = exp.as_ptr().cast_mut();
                        pbuf = Some(exp);
                    } else if let Some(full) = FullName_save(*fnamep, false) {
                        let full_vec = cstr_into_char_vec(full);
                        p = full_vec.as_ptr().cast_mut();
                        pbuf = Some(full_vec);
                    } else {
                        p = ptr::null_mut();
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
                            let s_cstr = cstr_to_cstring(dirname.as_ptr());
                            dirnamelen = home_replace(
                                None,
                                s_cstr.as_ptr(),
                                dirname.as_mut_ptr(),
                                MAXPATHL,
                                true,
                            );
                        }

                        if dirnamelen == 0 {
                            dirnamelen = crate::memory::cstr_len(dirname.as_ptr());
                        }

                        // Do not call shorten_fname() here since it removes the prefix
                        // even though the path does not have a prefix.
                        if path_cmp(opt_vars().p_fic != 0, p, dirname.as_ptr(), dirnamelen) == 0 {
                            p = p.add(dirnamelen);
                            if vim_ispathsep(*p as u8 as c_int) {
                                while !crate::memory::cstr_is_empty(p) && vim_ispathsep(*p as u8 as c_int) {
                                    p = p.add(1);
                                }
                                *fnamep = p;
                                if let Some(pb) = pbuf.take() {
                                    *bufp = Some(pb);
                                }
                            }
                        }
                    } else {
                        dirnamelen =
                            home_replace(None, p, dirname.as_mut_ptr(), MAXPATHL, true);
                        // Only replace it when it starts with '~'
                        if dirname[0] == b'~' as c_char {
                            let mut v = vec![0 as c_char; dirnamelen + 1];
                            ptr::copy_nonoverlapping(dirname.as_ptr(), v.as_mut_ptr(), dirnamelen);
                            set_fname_buf(fnamep, bufp, v);
                            has_homerelative = true;
                        }
                    }
                    drop(pbuf);
                }
            }

            let mut file_info = FileInfo::new();
            os_fileinfo2(*fnamep, &mut file_info);
            if *src.add(*usedlen) == b':' as c_char && *src.add(*usedlen + 1) == b'h' as c_char {
                s = (*fnamep).add(file_info.rest_off);
                *fnamep = (*fnamep).add(file_info.prefix_off);
            }

            let mut tail = path_tail(*fnamep);
            *fnamelen = crate::memory::cstr_len(*fnamep);

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
                    set_fname_buf(fnamep, bufp, vec![b'.' as c_char, NUL]);
                    tail = *fnamep;
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
                    s = (*fnamep).wrapping_sub(2);
                } else {
                    s = (*fnamep).wrapping_add(*fnamelen).wrapping_sub(1);
                }

                while (s as usize) > (tail as usize) {
                    if crate::memory::cstr_first_byte(s) == b'.' {
                        break;
                    }
                    s = s.wrapping_sub(1);
                }
                if *src.add(*usedlen + 1) == b'e' as c_char {
                    if (s as usize) > (tail as usize) {
                        // we stopped at a '.' (so anchor to &'.' + 1)
                        let newstart = s.add(1);
                        let distance_stepped_back =
                            (*fnamep as usize).wrapping_sub(newstart as usize);
                        *fnamelen = (*fnamelen).wrapping_add(distance_stepped_back);
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
                let mut v = cstr_into_char_vec(cstr_to_cstring(*fnamep));
                slash_adjust(v.as_mut_ptr());
                set_fname_buf(fnamep, bufp, v);
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
                        let pat = cstr_from_bytes(std::slice::from_raw_parts(
                            s.cast::<u8>(),
                            (p as usize).wrapping_sub(s as usize),
                        ));
                        s = p.add(1);
                        // find end of substitution
                        p = vim_strchr(s, sep);
                        if !p.is_null() {
                            let sub = cstr_from_bytes(std::slice::from_raw_parts(
                                s.cast::<u8>(),
                                (p as usize).wrapping_sub(s as usize),
                            ));
                            let mut str_copy = vec![0 as c_char; *fnamelen + 1];
                            if *fnamelen > 0 {
                                ptr::copy_nonoverlapping(*fnamep, str_copy.as_mut_ptr(), *fnamelen);
                            }
                            *usedlen = (p.add(1) as usize).wrapping_sub(src as usize);
                            let sub_res = do_string_sub(
                                str_copy.as_mut_ptr(),
                                *fnamelen,
                                pat.as_ptr().cast_mut(),
                                sub.as_ptr().cast_mut(),
                                None,
                                flags.cast_mut(),
                            );
                            *fnamelen = sub_res.as_bytes().len();
                            set_fname_buf(fnamep, bufp, cstr_into_char_vec(sub_res));
                            didit = true;
                        }
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
                let esc = vim_strsave_shellescape(*fnamep, false, false);
                if c != NUL as c_int {
                    *(*fnamep).add(*fnamelen) = c as c_char;
                }
                *fnamelen = esc.as_bytes().len();
                set_fname_buf(fnamep, bufp, cstr_into_char_vec(esc));
                *usedlen += 2;
            }

            break;
        }

        valid
    }
}

/// "chdir(dir)" function
pub fn f_chdir(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if check_secure() {
            return;
        }

        if argvars[0].v_type != VarType::VAR_STRING {
            // Returning an empty string means it failed.
            // No error message, for historic reasons.
            return;
        }

        // Return the current directory
        let mut cwd = vec![0 as c_char; MAXPATHL];
        if os_dirname(cwd.as_mut_ptr(), MAXPATHL) {
            #[cfg(windows)]
            slash_adjust(cwd.as_mut_ptr());
            crate::eval::typval::tv_set_string(rettv, cwd.as_ptr());
        }

        let mut scope: c_int = kCdScopeGlobal;
        if argvars[1].v_type != VarType::VAR_UNKNOWN {
            let s = tv_get_string(&argvars[1]);
            let s_bytes = crate::memory::cstr_as_bytes(s);
            if s_bytes == b"global" {
                scope = kCdScopeGlobal;
            } else if s_bytes == b"tabpage" {
                scope = kCdScopeTabpage;
            } else if s_bytes == b"window" {
                scope = kCdScopeWindow;
            } else if s_bytes == b"buffer" {
                scope = kCdScopeBuffer;
            } else {
                semsg!(gettext(e_invargNval), c"scope", s);
                return;
            }
        } else if curbuf_mut().b_localdir.is_some() {
            scope = kCdScopeBuffer;
        } else if curwin_mut().w_localdir.is_some() {
            scope = kCdScopeWindow;
        } else if curtab_mut().tp_localdir.is_some() {
            scope = kCdScopeTabpage;
        }

        if !do_chdir(argvars[0].vval.v_string(), scope) {
            // Directory change failed
            tv_clear(rettv);
            rettv.v_type = VarType::VAR_STRING;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

/// "delete()" function
pub fn f_delete(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        if check_secure() {
            return;
        }

        let name = tv_get_string(&argvars[0]);
        if crate::memory::cstr_is_empty(name) {
            emsg(gettext(e_invarg));
            return;
        }

        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let flags = if argvars[1].v_type != VarType::VAR_UNKNOWN {
            tv_get_string_buf(&argvars[1], nbuf.as_mut_ptr())
        } else {
            c"".as_ptr()
        };

        let flags_bytes = crate::memory::cstr_as_bytes(flags);
        if flags_bytes.is_empty() {
            // delete a file
            rettv.vval = crate::eval::typval::TypvalVval::Number(if os_remove(name) == 0 { 0 } else { -1 });
        } else if flags_bytes == b"d" {
            // delete an empty directory
            rettv.vval = crate::eval::typval::TypvalVval::Number(if os_rmdir(name) == 0 { 0 } else { -1 });
        } else if flags_bytes == b"rf" {
            // delete a directory recursively
            rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(delete_recursive(name)));
        } else {
            semsg!(gettext(e_invexpr2), flags);
        }
    }
}

/// "executable()" function
pub fn f_executable(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    if !tv_check_for_string_arg(argvars, 0) {
            return;
        }

        // Check in $PATH and also check directly if there is a directory name
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(os_can_exe(tv_get_string(&argvars[0]), None, true)));
}

/// "exepath()" function
pub fn f_exepath(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    if !tv_check_for_nonempty_string_arg(argvars, 0) {
        return;
    }

    let mut path: Option<std::ffi::CString> = None;

    os_can_exe(tv_get_string(&argvars[0]), Some(&mut path), true);

    rettv.v_type = VarType::VAR_STRING;
    if let Some(p) = path {
        #[cfg(windows)]
        {
            let mut v = p.into_bytes_with_nul();
            slash_adjust(v.as_mut_ptr().cast::<c_char>());
            crate::eval::typval::tv_set_string(rettv, v.as_ptr().cast::<c_char>());
        }
        #[cfg(not(windows))]
        {
            crate::eval::typval::tv_set_string(rettv, p.as_ptr());
        }
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
    }
}

/// "filecopy()" function
pub fn f_filecopy(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    if check_secure()
        || !tv_check_for_string_arg(argvars, 0)
        || !tv_check_for_string_arg(argvars, 1)
    {
        return;
    }

    let from = tv_get_string(&argvars[0]);

    let mut from_info = FileInfo::new();
    if os_fileinfo_link(from, &mut from_info)
        && (s_isreg(from_info.stat.st_mode) || s_islnk(from_info.stat.st_mode))
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(
            vim_copyfile(tv_get_string(&argvars[0]), tv_get_string(&argvars[1])),
        ));
    }
}

/// "filereadable()" function
pub fn f_filereadable(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let p = tv_get_string(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(!crate::memory::cstr_is_empty(p) && !os_isdir(p) && os_file_is_readable(p)));
    }
}

/// @return  0 for not writable
///          1 for writable file
///          2 for a dir which we have rights to write into.
pub fn f_filewritable(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let filename = tv_get_string(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(os_file_is_writable(filename)));
}

fn findfilendir(argvars: &mut [typval_T], rettv: &mut typval_T, find_what: c_int) {
    unsafe {
        let mut fresult: Option<std::ffi::CString> = None;
        let mut path = if crate::memory::cstr_is_empty(curbuf_mut().b_p_path) {
            opt_vars().p_path
        } else {
            curbuf_mut().b_p_path
        };
        let mut count: c_int = 1;
        let mut first = true;
        let mut error = false;

        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        rettv.v_type = VarType::VAR_STRING;

        let fname = tv_get_string(&argvars[0]);

        let mut pathbuf = [0 as c_char; NUMBUFLEN];
        if argvars[1].v_type != VarType::VAR_UNKNOWN {
            let p = tv_get_string_buf_chk(&argvars[1], pathbuf.as_mut_ptr());
            if p.is_null() {
                error = true;
            } else {
                if !crate::memory::cstr_is_empty(p) {
                    path = p.cast_mut();
                }

                if argvars[2].v_type != VarType::VAR_UNKNOWN {
                    count = tv_get_number_chk(&argvars[2], Some(&mut error)) as c_int;
                }
            }
        }

        if count < 0 {
            tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_UNKNOWN);
        }

        if !crate::memory::cstr_is_empty(fname) && !error {
            let mut file_to_find: Option<std::ffi::CString> = None;
            let mut search_ctx: Option<ff_search_ctx_T> = None;

            loop {
                fresult = find_file_in_path_option(
                    if first {
                        fname.cast_mut()
                    } else {
                        ptr::null_mut()
                    },
                    if first { crate::memory::cstr_len(fname) } else { 0 },
                    0,
                    c_int::from(first),
                    path,
                    find_what,
                    curbuf_mut().b_ffname,
                    if find_what == FINDFILE_DIR {
                        c"".as_ptr().cast_mut()
                    } else {
                        curbuf_mut().b_p_sua
                    },
                    &mut file_to_find,
                    &mut search_ctx,
                );
                first = false;

                if let Some(ref res) = fresult
                    && rettv.v_type == VarType::VAR_LIST
                {
                    tv_list_append_string( &mut *rettv.vval.v_list(), res.as_ptr(), -1);
                }

                let keep_going = rettv.v_type == VarType::VAR_LIST || {
                    count -= 1;
                    count > 0
                };
                if !keep_going || fresult.is_none() {
                    break;
                }
            }
        }

        if rettv.v_type == VarType::VAR_STRING {
            crate::eval::typval::tv_set_string(
                &mut *rettv,
                fresult.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            );
        }
    }
}

/// "finddir({fname}[, {path}[, {count}]])" function
pub fn f_finddir(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    findfilendir(argvars, rettv, FINDFILE_DIR);
}

/// "findfile({fname}[, {path}[, {count}]])" function
pub fn f_findfile(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    findfilendir(argvars, rettv, FINDFILE_FILE);
}

/// "fnamemodify({fname}, {mods})" function
pub fn f_fnamemodify(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let mut fbuf: Option<Vec<c_char>> = None;
        let mut len: usize = 0;
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut fname = tv_get_string_chk(&argvars[0]).cast_mut();
        let mods = tv_get_string_buf_chk(&argvars[1], buf.as_mut_ptr());
        if mods.is_null() || fname.is_null() {
            fname = ptr::null_mut();
        } else {
            #[cfg(windows)]
            let initial_cstr = path_to_slash_save(fname).unwrap_or_default();
            #[cfg(not(windows))]
            let initial_cstr = cstr_to_cstring(fname);
            len = initial_cstr.as_bytes().len();
            set_fname_buf(&raw mut fname, &mut fbuf, cstr_into_char_vec(initial_cstr));
            if !crate::memory::cstr_is_empty(mods) {
                let mut usedlen: usize = 0;
                modify_fname(
                    mods.cast_mut(),
                    false,
                    &mut usedlen,
                    &raw mut fname,
                    &mut fbuf,
                    &mut len,
                    false,
                );
            }
        }

        if fname.is_null() {
            rettv.v_type = VarType::VAR_STRING;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        } else {
            crate::eval::typval::tv_set_string_len(rettv, fname, len);
        }
        drop(fbuf);
    }
}

/// Resolves the `[{winnr}[, {tabnr}[, {bufnr}]]]` arguments of `getcwd()`/`haslocaldir()` to the
/// requested scope and the scope objects to inspect.
fn getcwd_scope_args(argvars: &mut [typval_T],
    default_scope: c_int,
    scope: &mut c_int,
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
        *win = curwin_ptr();
        *tp = g_st().curtab;
        *buf = g_st().curbuf;

        while argc < 3 {
            // If there is no argument there are no more scopes after it, break out.
            if argvars[argc].v_type == VarType::VAR_UNKNOWN {
                break;
            }
            if argvars[argc].v_type != VarType::VAR_NUMBER {
                emsg(gettext(e_invarg));
                return false;
            }
            argv[argc] = argvars[argc].vval.v_number() as c_int;
            // It is an error for the scope number to be less than `-1`.
            if argv[argc] < -1 {
                emsg(gettext(e_invarg));
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
                    c"E5001: Argument cannot be -1 if preceding argument is >= 0.",
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
                    c"E5006: Window and tab scope must be -1 when using buffer scope",
                ));
                return false;
            }
            if argv[kBufArg] > 0 {
                let mut err = Error::new();
                *buf = find_buffer_by_handle(argv[kBufArg], &mut err);
                if err.err_type != ErrorType::kErrorTypeNone {
                    emsg(gettext(c"E5007: Cannot find buffer number."));
                    crate::api::private::helpers::api_clear_error(&mut err);
                    return false;
                }
            }
        }

        // Find the tabpage by number.
        if argv[kTabArg] > 0 {
            *tp = find_tabpage(argv[kTabArg]);
            if (*tp).is_null() {
                emsg(gettext(c"E5000: Cannot find tab number."));
                return false;
            }
        }

        // Find the window in `tp` by number.
        if argv[kWinArg] > 0 {
            *win = find_win_by_nr(&argvars[0], *tp);
            if (*win).is_null() {
                emsg(gettext(c"E5002: Cannot find window number."));
                return false;
            }
        }

        true
    }
}

/// `getcwd([{win}[, {tab}[, {buf}]]])` function
pub fn f_getcwd(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    unsafe {
        let mut scope: c_int = kCdScopeInvalid;
        let mut win: *mut win_T = ptr::null_mut();
        let mut tp: *mut tabpage_T = ptr::null_mut();
        let mut buf: *mut buf_T = ptr::null_mut();

        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        // Without arguments (kCdScopeInvalid), the effective working directory is returned.
        if !getcwd_scope_args(
            argvars,
            kCdScopeInvalid,
            &mut scope,
            &raw mut win,
            &raw mut tp,
            &raw mut buf,
        ) {
            return;
        }

        let mut from: *const c_char = ptr::null(); // The local directory to copy.
        let mut cwd = vec![0 as c_char; MAXPATHL];

        let mut cur_scope = scope;
        if cur_scope == kCdScopeWindow {
            from = (*win)
                .w_localdir
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr());
            if from.is_null() {
                cur_scope = kCdScopeTabpage;
            }
        }
        if cur_scope == kCdScopeTabpage {
            from = (*tp)
                .tp_localdir
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr());
            if from.is_null() {
                cur_scope = kCdScopeBuffer;
            }
        }
        if cur_scope == kCdScopeBuffer {
            // Only a buffer target reaches this with `scope` unchanged: a buffer belongs to no
            // particular window or tabpage, so it skips those and falls back to global dir.
            from = if scope == kCdScopeBuffer {
                (*buf)
                    .b_localdir
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr())
            } else {
                ptr::null()
            };
            if from.is_null() {
                cur_scope = kCdScopeGlobal;
            }
        }
        if cur_scope == kCdScopeGlobal {
            if let Some(ref gdir) = g_st().globaldir {
                from = gdir.as_ptr();
            } else {
                cur_scope = kCdScopeInvalid;
            }
        }
        if cur_scope == kCdScopeInvalid && !os_dirname(cwd.as_mut_ptr(), MAXPATHL) {
            from = c"".as_ptr(); // Return empty string on failure.
        }

        if !from.is_null() {
            xstrlcpy(cwd.as_mut_ptr(), from, MAXPATHL);
        }

        #[cfg(windows)]
        slash_adjust(cwd.as_mut_ptr());
        crate::eval::typval::tv_set_string(rettv, cwd.as_ptr());
    }
}

/// "getfperm({fname})" function
pub fn f_getfperm(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let flags = [b'r' as c_char, b'w' as c_char, b'x' as c_char];

        let filename = tv_get_string(&argvars[0]);
        let file_perm = os_getperm(filename);
        if file_perm >= 0 {
            let mut perm = [b'-' as c_char; 9];
            for i in 0..9 {
                if (file_perm & (1 << (8 - i))) != 0 {
                    perm[i] = flags[i % 3];
                }
            }
            crate::eval::typval::tv_set_string_len(rettv, perm.as_ptr(), 9);
        } else {
            rettv.v_type = VarType::VAR_STRING;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
}

/// "getfsize({fname})" function
pub fn f_getfsize(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let fname = tv_get_string(&argvars[0]);

        let mut file_info = FileInfo::new();
        if os_fileinfo(fname, &mut file_info) {
            let filesize = os_fileinfo_size(&file_info);
            if s_isdir(file_info.stat.st_mode) {
                rettv.vval = crate::eval::typval::TypvalVval::Number(0);
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::Number(filesize as varnumber_T);

                // non-perfect check for overflow
                if rettv.vval.v_number() as u64 != filesize {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(-2);
                }
            }
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        }
}

/// "getftime({fname})" function
pub fn f_getftime(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let fname = tv_get_string(&argvars[0]);

        let mut file_info = FileInfo::new();
        if os_fileinfo(fname, &mut file_info) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(file_info.stat.st_mtim.tv_sec as varnumber_T);
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        }
}

/// "getftype({fname})" function
pub fn f_getftype(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let fname = tv_get_string(&argvars[0]);

        let mut file_info = FileInfo::new();
        if os_fileinfo_link(fname, &mut file_info) {
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
            crate::eval::typval::tv_set_string(rettv, t);
        } else {
            rettv.v_type = VarType::VAR_STRING;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
}

/// "glob()" function
pub fn f_glob(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let mut options: c_int = WILD_SILENT | WILD_USE_NL;
        let mut xpc = expand_T::new();
        let mut error = false;

        // When the optional second argument is non-zero, don't remove matches
        // for 'wildignore' and don't put matches for 'suffixes' at the end.
        rettv.v_type = VarType::VAR_STRING;
        if argvars[1].v_type != VarType::VAR_UNKNOWN {
            if tv_get_bool_chk(&argvars[1], Some(&mut error)) != 0 {
                options |= WILD_KEEP_ALL;
            }
            if argvars[2].v_type != VarType::VAR_UNKNOWN {
                if tv_get_bool_chk(&argvars[2], Some(&mut error)) != 0 {
                    tv_list_set_ret(rettv, ptr::null_mut());
                }
                if argvars[3].v_type != VarType::VAR_UNKNOWN
                    && tv_get_bool_chk(&argvars[3], Some(&mut error)) != 0
                {
                    options |= WILD_ALLLINKS;
                }
            }
        }
        if !error {
            xpc.xp_context = EXPAND_FILES;
            if opt_vars().p_wic != 0 {
                options += WILD_ICASE;
            }
            if rettv.v_type == VarType::VAR_STRING {
                if let Some(exp) = ExpandOne(
                    &mut xpc,
                    tv_get_string(&argvars[0]).cast_mut(),
                    None,
                    options,
                    WILD_ALL,
                ) {
                    crate::eval::typval::tv_set_string(rettv, exp.as_ptr());
                } else {
                    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                }
            } else {
                ExpandOne(
                    &mut xpc,
                    tv_get_string(&argvars[0]).cast_mut(),
                    None,
                    options,
                    WILD_ALL_KEEP,
                );
                tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);
                for f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
                    tv_list_append_string(
                        unsafe { &mut *rettv.vval.v_list() },
                        f.as_ptr(),
                        f.as_bytes().len() as isize,
                    );
                }
            }
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

/// "globpath()" function
pub fn f_globpath(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let mut flags: c_int = 0; // Flags for globpath.
        let mut error = false;

        // Return a string, or a list if the optional third argument is non-zero.
        rettv.v_type = VarType::VAR_STRING;

        if argvars[2].v_type != VarType::VAR_UNKNOWN {
            // When the optional second argument is non-zero, don't remove matches
            // for 'wildignore' and don't put matches for 'suffixes' at the end.
            if tv_get_bool_chk(&argvars[2], Some(&mut error)) != 0 {
                flags |= WILD_KEEP_ALL;
            }

            if argvars[3].v_type != VarType::VAR_UNKNOWN {
                if tv_get_bool_chk(&argvars[3], Some(&mut error)) != 0 {
                    tv_list_set_ret(rettv, ptr::null_mut());
                }
                if argvars[4].v_type != VarType::VAR_UNKNOWN
                    && tv_get_bool_chk(&argvars[4], Some(&mut error)) != 0
                {
                    flags |= WILD_ALLLINKS;
                }
            }
        }

        let mut buf1 = [0 as c_char; NUMBUFLEN];
        let file = tv_get_string_buf_chk(&argvars[1], buf1.as_mut_ptr());
        if !file.is_null() && !error {
            let mut ga: Vec<std::ffi::CString> = Vec::new();
            globpath(
                tv_get_string(&argvars[0]).cast_mut(),
                file.cast_mut(),
                &mut ga,
                flags,
                false,
            );

            if rettv.v_type == VarType::VAR_STRING {
                let joined = crate::path::concat_cstrings(&ga, c"\n");
                crate::eval::typval::tv_set_string(rettv, joined.as_ptr());
            } else {
                tv_list_alloc_ret(rettv, ga.len() as isize);
                for s in &ga {
                    tv_list_append_string(
                        unsafe { &mut *rettv.vval.v_list() },
                        s.as_ptr(),
                        s.as_bytes().len() as isize,
                    );
                }
            }
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

/// "glob2regpat()" function
pub fn f_glob2regpat(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let pat = tv_get_string_chk(&argvars[0]); // NULL on type error
        let regpat = if pat.is_null() {
            None
        } else {
            file_pat_to_reg_pat(pat, ptr::null(), ptr::null_mut(), 0)
        };
        crate::eval::typval::tv_set_string(
            rettv,
            regpat.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        );
}

/// `haslocaldir([{win}[, {tab}[, {buf}]]])` function
pub fn f_haslocaldir(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    unsafe {
        let mut scope: c_int = kCdScopeWindow;
        let mut win: *mut win_T = ptr::null_mut();
        let mut tp: *mut tabpage_T = ptr::null_mut();
        let mut buf: *mut buf_T = ptr::null_mut();

        rettv.v_type = VarType::VAR_NUMBER;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        // Without arguments, default to window scope.
        if !getcwd_scope_args(
            argvars,
            kCdScopeWindow,
            &mut scope,
            &raw mut win,
            &raw mut tp,
            &raw mut buf,
        ) {
            return;
        }

        match scope {
            kCdScopeWindow => {
                rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from((*win).w_localdir.is_some()));
            }
            kCdScopeBuffer => {
                rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from((*buf).b_localdir.is_some()));
            }
            kCdScopeTabpage => {
                rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from((*tp).tp_localdir.is_some()));
            }
            kCdScopeGlobal => {
                // The global scope never has a local directory
            }
            _ => std::process::abort(),
        }
    }
}

/// "isabsolutepath()" function
pub fn f_isabsolutepath(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(path_is_absolute(tv_get_string(&argvars[0]))));
}

/// "isdirectory()" function
pub fn f_isdirectory(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(os_isdir(tv_get_string(&argvars[0]))));
}

/// "mkdir()" function
pub fn f_mkdir(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let mut prot: c_int = 0o755;

        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(FAIL));
        if check_secure() {
            return;
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let dir = tv_get_string_buf(&argvars[0], buf.as_mut_ptr());
        if crate::memory::cstr_is_empty(dir) {
            return;
        }

        if *path_tail(dir) == NUL {
            // Remove trailing slashes.
            *path_tail_with_sep(dir.cast_mut()) = NUL;
        }

        let mut defer = false;
        let mut defer_recurse = false;
        let mut created: Option<std::ffi::CString> = None;
        if argvars[1].v_type != VarType::VAR_UNKNOWN {
            if argvars[2].v_type != VarType::VAR_UNKNOWN {
                prot = tv_get_number_chk(&argvars[2], None) as c_int;
                if prot == -1 {
                    return;
                }
            }
            let arg2 = tv_get_string(&argvars[1]);
            defer = !vim_strchr(arg2, b'D' as c_int).is_null();
            defer_recurse = !vim_strchr(arg2, b'R' as c_int).is_null();
            if (defer || defer_recurse) && !can_add_defer() {
                return;
            }

            if !vim_strchr(arg2, b'p' as c_int).is_null() {
                let mut failed_dir: Option<std::ffi::CString> = None;
                let ret = os_mkdir_recurse(
                    dir,
                    prot,
                    &mut failed_dir,
                    (defer || defer_recurse).then_some(&mut created),
                );
                if ret != 0 {
                    semsg!(
                        gettext(e_mkdir),
                        failed_dir.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                        uv_strerror(ret),
                    );
                    rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(FAIL));
                    return;
                }
                rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(OK));
            }
        }
        if rettv.vval.v_number() == varnumber_T::from(FAIL) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(vim_mkdir_emsg(dir, prot)));
        }

        // Handle "D" and "R": deferred deletion of the created directory.
        if rettv.vval.v_number() == varnumber_T::from(OK)
            && created.is_none()
            && (defer || defer_recurse)
        {
            created = FullName_save(dir, false);
        }
        if let Some(created_cstr) = created {
            let mut tv = [typval_T::new(), typval_T::new()];
            crate::eval::typval::tv_set_string(&mut tv[0], created_cstr.as_ptr());
            crate::eval::typval::tv_set_string(
                &mut tv[1],
                if defer_recurse {
                    c"rf".as_ptr()
                } else {
                    c"d".as_ptr()
                },
            );
            add_defer(c"delete", 2, tv.as_mut_ptr());
        }
    }
}

/// "pathshorten()" function
pub fn f_pathshorten(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let mut trim_len: c_int = 1;

        if argvars[1].v_type != VarType::VAR_UNKNOWN {
            trim_len = tv_get_number(&argvars[1]) as c_int;
            if trim_len < 1 {
                trim_len = 1;
            }
        }

        let p = tv_get_string_chk(&argvars[0]);
        if p.is_null() {
            rettv.v_type = VarType::VAR_STRING;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        } else {
            let mut v = cstr_into_char_vec(cstr_to_cstring(p));
            shorten_dir_len(v.as_mut_ptr(), trim_len);
            crate::eval::typval::tv_set_string(rettv, v.as_ptr());
        }
    }
}

/// Evaluate "expr" (= "context") for readdir().
pub(crate) fn readdir_checkitem(expr: &mut typval_T, name: *const c_char) -> varnumber_T {
    let tv_init = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        let mut argv = [tv_init; 2];
        let mut retval: varnumber_T = 0;
        let mut error = false;

        if expr.v_type == VarType::VAR_UNKNOWN {
            return 1;
        }

        let mut save_val = tv_init;
        prepare_vimvar(VV_VAL, &mut save_val);
        set_vim_var_string(VV_VAL, name, -1);
        argv[0].v_type = VarType::VAR_STRING;
        argv[0].vval = crate::eval::typval::TypvalVval::String(name.cast_mut());

        let mut rettv = tv_init;
        if eval_expr_typval(&*expr, false, argv.as_mut_ptr(), 1, &mut rettv) {
            retval = tv_get_number_chk(&rettv, Some(&mut error));
            if error {
                retval = -1;
            }

            tv_clear(&mut rettv);
        }

        set_vim_var_string(VV_VAL, ptr::null(), 0);
        restore_vimvar(VV_VAL, &mut save_val);
        retval
}

/// "readdir()" function
pub fn f_readdir(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN);
        if check_secure() {
            return;
        }

        let path = tv_get_string(&argvars[0]);
        let expr = (&raw mut argvars[1]);
        let mut entries: Vec<std::ffi::CString> = Vec::new();
        let ret = readdir_core(&mut entries, path, unsafe { expr.as_mut() });
        if ret {
            for entry in &entries {
                tv_list_append_string(unsafe { &mut *rettv.vval.v_list() }, entry.as_ptr(), -1);
            }
        }
    }
}

/// Read blob from file "fd".
/// Caller has allocated a blob in "rettv".
fn read_blob(
    fd: *mut FILE,
    rettv: &mut typval_T,
    mut offset: off_T,
    size_arg: off_T,
) -> bool {
    unsafe {
        let blob = rettv.vval.v_blob();
        let mut file_info = FileInfo::new();
        if !os_fileinfo_fd(fileno(fd), &mut file_info) {
            return false; // can't read the file, error
        }

        let mut size = size_arg;
        let file_size = os_fileinfo_size(&file_info) as off_T;
        let whence = if offset >= 0 {
            // The size defaults to the whole file.  If a size is given it is
            // limited to not go past the end of the file.
            if size == -1 || (size > file_size - offset && !s_ischr(file_info.stat.st_mode)) {
                // size may become negative, checked below
                size = os_fileinfo_size(&file_info) as off_T - offset;
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
            return true;
        }
        if offset != 0 && vim_fseek(fd, offset, whence) != 0 {
            return true;
        }

        let bv = &mut (*blob).bv_ga;
        bv.resize(size as usize, 0);
        if fread(bv.as_mut_ptr().cast(), 1, bv.len(), fd) < bv.len() {
            // An empty blob is returned on error.
            crate::eval::typval::tv_blob_unref(rettv.vval.v_blob());
            rettv.vval = crate::eval::typval::TypvalVval::Blob(ptr::null_mut());
            return false;
        }
        true
    }
}

/// "readfile()" or "readblob()" function
fn read_file_or_blob(argvars: &mut [typval_T], rettv: &mut typval_T, always_blob: bool) {
    unsafe {
        let mut binary = false;
        let mut blob = always_blob;
        let mut buf = [0 as c_char; (IOSIZE / 256) * 256]; // rounded to avoid odd + 1
        let io_size = buf.len();
        let mut prev: Vec<c_char> = Vec::new(); // previously read bytes, if any
        let mut maxline: i64 = MAXLNUM;
        let mut offset: off_T = 0;
        let mut size: off_T = -1;

        if argvars[1].v_type != VarType::VAR_UNKNOWN {
            if always_blob {
                offset = tv_get_number(&argvars[1]);
                if argvars[2].v_type != VarType::VAR_UNKNOWN {
                    size = tv_get_number(&argvars[2]);
                }
            } else {
                let arg1 = crate::memory::cstr_as_bytes(tv_get_string(&argvars[1]));
                if arg1 == b"b" {
                    binary = true;
                } else if arg1 == b"B" {
                    blob = true;
                }
                if argvars[2].v_type != VarType::VAR_UNKNOWN {
                    maxline = tv_get_number(&argvars[2]);
                }
            }
        }

        if blob {
            tv_blob_alloc_ret(rettv);
        } else {
            tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_UNKNOWN);
        }

        // Always open the file in binary mode, library functions have a mind of
        // their own about CR-LF conversion.
        let fname = tv_get_string(&argvars[0]);

        if os_isdir(fname) {
            semsg!(gettext(e_isadir2), fname);
            return;
        }
        if crate::memory::cstr_is_empty(fname) {
            semsg!(gettext(e_notopen), gettext(c"<empty>"));
            return;
        }
        let fd = os_fopen(fname, c"rb");
        if fd.is_null() {
            semsg!(gettext(e_notopen), fname);
            return;
        }

        if blob {
            if !read_blob(fd, rettv, offset, size) {
                semsg!(gettext(e_cant_read_file_str), fname);
            }
            fclose(fd);
            return;
        }

        let l = rettv.vval.v_list();
        let buf_ptr = buf.as_mut_ptr();

        while maxline < 0 || i64::from(tv_list_len(l.as_ref())) < maxline {
            let mut readlen = fread(buf_ptr.cast(), 1, io_size, fd) as isize;

            let mut p = buf_ptr; // Position in buf.
            let mut start = buf_ptr; // Start of current line.
            while (p as usize) < (buf_ptr.offset(readlen) as usize)
                || (readlen <= 0 && (!prev.is_empty() || binary))
            {
                if readlen <= 0 || crate::memory::cstr_first_byte(p) == b'\n' {
                    let mut len = (p as usize).wrapping_sub(start as usize);

                    // Finished a line.  Remove CRs before NL.
                    if readlen > 0 && !binary {
                        while len > 0 && *start.add(len - 1) == b'\r' as c_char {
                            len -= 1;
                        }
                        // removal may cross back to the "prev" string
                        if len == 0 {
                            while prev.last() == Some(&(b'\r' as c_char)) {
                                prev.pop();
                            }
                        }
                    }
                    if prev.is_empty() {
                        debug_assert!(len < c_int::MAX as usize);
                        tv_list_append_string( &mut *l, start, len as isize);
                    } else {
                        prev.extend_from_slice(std::slice::from_raw_parts(start, len));
                        tv_list_append_string( &mut *l, prev.as_ptr(), prev.len() as isize);
                        prev.clear();
                    }

                    start = p.add(1); // Step over newline.
                    if maxline < 0 {
                        if i64::from(tv_list_len(l.as_ref())) > -maxline {
                            debug_assert_eq!(i64::from(tv_list_len(l.as_ref())), 1 + (-maxline));
                            tv_list_item_remove(&mut *l, tv_list_first(l.as_ref()));
                        }
                    } else if i64::from(tv_list_len(l.as_ref())) >= maxline {
                        debug_assert_eq!(i64::from(tv_list_len(l.as_ref())), maxline);
                        break;
                    }
                    if readlen <= 0 {
                        break;
                    }
                } else if crate::memory::cstr_is_empty(p) {
                    *p = b'\n' as c_char;
                } else if *p as u8 == 0xbf && !binary {
                    // Check for utf8 "bom"; U+FEFF is encoded as EF BB BF.  Do this
                    // when finding the BF and check the previous two bytes.
                    let back1 = if (p as usize) >= (buf_ptr.add(1) as usize) {
                        *p.sub(1)
                    } else {
                        prev.last().copied().unwrap_or(NUL)
                    };
                    let back2 = if (p as usize) >= (buf_ptr.add(2) as usize) {
                        *p.sub(2)
                    } else if p == buf_ptr.add(1) {
                        prev.last().copied().unwrap_or(NUL)
                    } else if prev.len() >= 2 {
                        prev[prev.len() - 2]
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
                            let mut adjust_prevlen: usize = 0;

                            if (dest as usize) < (buf_ptr as usize) {
                                // adjust_prevlen must be 1 or 2.
                                adjust_prevlen = (buf_ptr as usize).wrapping_sub(dest as usize);
                                dest = buf_ptr;
                            }
                            let p_off = (p as usize).wrapping_sub(buf_ptr as usize) as isize;
                            if readlen > p_off + 1 {
                                ptr::copy(p.add(1), dest, (readlen - p_off - 1) as usize);
                            }
                            readlen -= 3 - adjust_prevlen as isize;
                            prev.truncate(prev.len() - adjust_prevlen);
                            p = dest.sub(1);
                        }
                    }
                }
                p = p.add(1);
            }

            if (maxline >= 0 && i64::from(tv_list_len(l.as_ref())) >= maxline) || readlen <= 0 {
                break;
            }
            if (start as usize) < (p as usize) {
                let part_len = (p as usize).wrapping_sub(start as usize);
                prev.extend_from_slice(std::slice::from_raw_parts(start, part_len));
            }
        }

        fclose(fd);
    }
}

/// "readblob()" function
pub fn f_readblob(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    if check_secure() {
            return;
        }

        read_file_or_blob(argvars, rettv, true);
}

/// "readfile()" function
pub fn f_readfile(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    if check_secure() {
            return;
        }

        read_file_or_blob(argvars, rettv, false);
}

/// "rename({from}, {to})" function
pub fn f_rename(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    if check_secure() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    } else {
        let mut buf = [0 as c_char; NUMBUFLEN];
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(vim_rename(
            tv_get_string(&argvars[0]),
            tv_get_string_buf(&argvars[1], buf.as_mut_ptr()),
        )));
    }
}

/// "resolve()" function
pub fn f_resolve(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    rettv.v_type = VarType::VAR_STRING;
    let fname = tv_get_string(&argvars[0]);
    #[cfg(windows)]
    {
        let mut v = os_resolve_shortcut(fname);
        if v.is_none() && os_is_reparse_point_include(fname) {
            v = os_realpath_cstr(fname, MAXPATHL + 1);
        }
        if let Some(resolved) = v {
            crate::eval::typval::tv_set_string(rettv, resolved.as_ptr());
        } else {
            crate::eval::typval::tv_set_string(rettv, fname);
        }
    }
    #[cfg(not(windows))]
    unsafe {
            let mut is_relative_to_current = false;
            let mut has_trailing_pathsep = false;
            let mut limit: c_int = 100;

            let mut p_vec = cstr_into_char_vec(cstr_to_cstring(fname));
            let mut p = p_vec.as_mut_ptr();

            if crate::memory::cstr_first_byte(p) == b'.'
                && (vim_ispathsep(*p.add(1) as u8 as c_int)
                    || (*p.add(1) == b'.' as c_char && vim_ispathsep(*p.add(2) as u8 as c_int)))
            {
                is_relative_to_current = true;
            }

            let mut len = crate::memory::cstr_len(p) as isize;
            if len > 1 && after_pathsep(p, p.offset(len)) != 0 {
                has_trailing_pathsep = true;
                *p.offset(len - 1) = NUL; // The trailing slash breaks readlink().
            }

            let mut q = path_next_component(p).cast_mut();
            let mut remain_vec: Option<Vec<c_char>> = None;
            if !crate::memory::cstr_is_empty(q) {
                // Separate the first path component in "p", and keep the
                // remainder (beginning with the path separator).
                remain_vec = Some(cstr_into_char_vec(cstr_to_cstring(q.sub(1))));
                *q.sub(1) = NUL;
            }

            let mut buf_vec = vec![0 as c_char; MAXPATHL + 1];
            let buf = buf_vec.as_mut_ptr();

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
                        emsg(gettext(c"E655: Too many symbolic links (cycle?)"));
                        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                        return;
                    }

                    // Ensure that the result will have a trailing path separator
                    // if the argument has one.
                    if remain_vec.is_none() && has_trailing_pathsep {
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
                    if !crate::memory::cstr_is_empty(q) {
                        let head_bytes = crate::memory::cstr_as_bytes(q.sub(1));
                        let mut new_rem = Vec::with_capacity(
                            head_bytes.len()
                                + remain_vec.as_ref().map_or(0, |r| crate::memory::cstr_len(r.as_ptr()))
                                + 1,
                        );
                        new_rem.extend(head_bytes.iter().map(|&b| b as c_char));
                        if let Some(ref rem) = remain_vec {
                            let rem_len = crate::memory::cstr_len(rem.as_ptr());
                            new_rem.extend_from_slice(&rem[..rem_len]);
                        }
                        new_rem.push(NUL);
                        remain_vec = Some(new_rem);
                        *q.sub(1) = NUL;
                    }

                    q = path_tail(p);
                    if (q as usize) > (p as usize) && crate::memory::cstr_is_empty(q) {
                        // Ignore trailing path separator.
                        *q.sub(1) = NUL;
                        q = path_tail(p);
                    }
                    if (q as usize) > (p as usize) && !path_is_absolute(buf) {
                        // Symlink is relative to directory of argument. Replace the
                        // symlink with the resolved name in the same directory.
                        let prefix_len = (q as usize) - (p as usize);
                        let buf_len = crate::memory::cstr_len(buf);
                        let mut new_p = vec![0 as c_char; prefix_len + buf_len + 1];
                        ptr::copy_nonoverlapping(p, new_p.as_mut_ptr(), prefix_len);
                        ptr::copy_nonoverlapping(buf, new_p.as_mut_ptr().add(prefix_len), buf_len);
                        p_vec = new_p;
                        p = p_vec.as_mut_ptr();
                    } else {
                        p_vec = cstr_into_char_vec(cstr_to_cstring(buf));
                        p = p_vec.as_mut_ptr();
                    }
                }

                let Some(ref mut rem_v) = remain_vec else {
                    break;
                };
                let remain = rem_v.as_mut_ptr();

                // Append the first path component of "remain" to "p".
                q = path_next_component(remain.add(1)).cast_mut();
                len = (q as usize).wrapping_sub(remain as usize) as isize - isize::from(!crate::memory::cstr_is_empty(q));
                let p_len = crate::memory::cstr_len(p);
                let mut new_p = vec![0 as c_char; p_len + len as usize + 1];
                ptr::copy_nonoverlapping(p, new_p.as_mut_ptr(), p_len);
                ptr::copy_nonoverlapping(remain, new_p.as_mut_ptr().add(p_len), len as usize);
                p_vec = new_p;
                p = p_vec.as_mut_ptr();

                // Shorten "remain".
                if !crate::memory::cstr_is_empty(q) {
                    let rem_tail = q.sub(1);
                    let rem_len = crate::memory::cstr_len(rem_tail) + 1;
                    ptr::copy(rem_tail, remain, rem_len);
                } else {
                    remain_vec = None;
                }
            }

            // If the result is a relative path name, make it explicitly relative to
            // the current directory if and only if the argument had this form.
            if !vim_ispathsep(*p as u8 as c_int) {
                if is_relative_to_current
                    && !crate::memory::cstr_is_empty(p)
                    && !(crate::memory::cstr_first_byte(p) == b'.'
                        && (*p.add(1) == NUL
                            || vim_ispathsep(*p.add(1) as u8 as c_int)
                            || (*p.add(1) == b'.' as c_char
                                && (*p.add(2) == NUL || vim_ispathsep(*p.add(2) as u8 as c_int)))))
                {
                    // Prepend "./".
                    let p_len = crate::memory::cstr_len(p);
                    let mut new_p = vec![0 as c_char; p_len + 3];
                    new_p[0] = b'.' as c_char;
                    new_p[1] = b'/' as c_char;
                    ptr::copy_nonoverlapping(p, new_p.as_mut_ptr().add(2), p_len);
                    p_vec = new_p;
                    p = p_vec.as_mut_ptr();
                } else if !is_relative_to_current {
                    // Strip leading "./".
                    q = p;
                    while crate::memory::cstr_first_byte(q) == b'.' && vim_ispathsep(*q.add(1) as u8 as c_int) {
                        q = q.add(2);
                    }
                    if (q as usize) > (p as usize) {
                        let src_ptr = p.add(2);
                        let move_len = crate::memory::cstr_len(src_ptr) + 1;
                        ptr::copy(src_ptr, p, move_len);
                    }
                }
            }

            // Ensure that the result will have no trailing path separator
            // if the argument had none.  But keep "/" or "//".
            if !has_trailing_pathsep {
                q = p.add(crate::memory::cstr_len(p));
                if after_pathsep(p, q) != 0 {
                    *path_tail_with_sep(p) = NUL;
                }
            }

            crate::eval::typval::tv_set_string(rettv, p);
        }

        simplify_filename(rettv.vval.v_string());
}

/// "simplify()" function
pub fn f_simplify(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let p = tv_get_string(&argvars[0]);
        let mut v = cstr_into_char_vec(cstr_to_cstring(p));
        simplify_filename(v.as_mut_ptr()); // Simplify in place.
        crate::eval::typval::tv_set_string(rettv, v.as_ptr());
}

/// "tempname()" function
pub fn f_tempname(_argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    if let Some(tn) = vim_tempname() {
            crate::eval::typval::tv_set_string(rettv, tn.as_ptr());
        } else {
            rettv.v_type = VarType::VAR_STRING;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
}

/// Write "list" of strings to file "fp".
fn write_list(fp: &mut FileDescriptor, list: Option<&list_T>, binary: bool) -> bool {
    unsafe {
        let mut error: c_int = 0;
        let mut li = tv_list_first(list);
        'outer: while !li.is_null() {
            let s = tv_get_string_chk(&(*li).li_tv);
            if s.is_null() {
                return false;
            }
            let mut hunk_start = s;
            let mut p = hunk_start;
            loop {
                if crate::memory::cstr_is_empty(p) || *p == NL {
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
                    if crate::memory::cstr_is_empty(p) {
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
            error = file_flush(&raw mut *fp);
            if error == 0 {
                return true;
            }
        }
        semsg!(
            gettext(E_ERROR_WHILE_WRITING_STR),
            uv_strerror(error),
        );
        false
    }
}

/// Write a blob to file with descriptor `fp`.
fn write_data(fp: &mut FileDescriptor, data: *const c_char, len: usize) -> bool {
    unsafe {
        if len > 0 {
            let written = file_write(fp, data, len);
            if written < len as isize {
                let error = written as c_int;
                semsg!(
                    gettext(E_ERROR_WHILE_WRITING_STR),
                    uv_strerror(error),
                );
                return false;
            }
        }
        let error = file_flush(&raw mut *fp);
        if error != 0 {
            semsg!(
                gettext(E_ERROR_WHILE_WRITING_STR),
                uv_strerror(error),
            );
            return false;
        }
        true
    }
}

fn write_blob(fp: &mut FileDescriptor, blob: &blob_T) -> bool {
    write_data(
            fp,
            blob.bv_ga.as_ptr().cast(),
            tv_blob_len(Some(blob)) as usize,
        )
}

fn write_string(fp: &mut FileDescriptor, data: *const c_char) -> bool {
     write_data(fp, data, crate::memory::cstr_len(data))
}

/// "writefile()" function
pub fn f_writefile(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if check_secure() {
            return;
        }

        if argvars[0].v_type == VarType::VAR_LIST {
            for li in crate::eval::typval::list_iter_mut(tv_list_first(argvars[0].vval.v_list().as_ref())) {
                if !tv_check_str_or_nr(&li.li_tv) {
                    return;
                }
            }
        } else if argvars[0].v_type != VarType::VAR_BLOB
            && !(argvars[0].v_type == VarType::VAR_STRING
                && (script_is_lua(g_st().current_sctx.sc_sid) || g_st().current_sctx.sc_sid == SID_API_CLIENT))
        {
            semsg!(
                gettext(e_invarg2),
                gettext(c"writefile() first argument must be a List or a Blob"),
            );
            return;
        }

        let mut binary = false;
        let mut append = false;
        let mut defer = false;
        let mut do_fsync = opt_vars().p_fs != 0;
        let mut mkdir_p = false;
        if argvars[2].v_type != VarType::VAR_UNKNOWN {
            let flags = tv_get_string_chk(&argvars[2]);
            if flags.is_null() {
                return;
            }
            let mut p = flags;
            while !crate::memory::cstr_is_empty(p) {
                match *p as u8 {
                    b'b' => binary = true,
                    b'a' => append = true,
                    b'D' => defer = true,
                    b's' => do_fsync = true,
                    b'S' => do_fsync = false,
                    b'p' => mkdir_p = true,
                    _ => {
                        semsg!(gettext(c"E5060: Unknown flag: %s"), p);
                        return;
                    }
                }
                p = p.add(1);
            }
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let fname = tv_get_string_buf_chk(&argvars[1], buf.as_mut_ptr());
        if fname.is_null() {
            return;
        }

        if defer && !can_add_defer() {
            return;
        }

        let mut fp = FileDescriptor::new();
        if crate::memory::cstr_is_empty(fname) {
            emsg(gettext(
                c"E482: Can't open file with an empty name",
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
            let mut error = file_open(&mut fp, fname, open_flags, 0o666);
            if error != 0 {
                semsg!(
                    gettext(c"E482: Can't open file %s for writing: %s"),
                    fname,
                    uv_strerror(error),
                );
            } else {
                if defer {
                    let mut tv = typval_T::new();
                    if let Some(full) = FullName_save(fname, false) {
                        crate::eval::typval::tv_set_string(&mut tv, full.as_ptr());
                    } else {
                        tv.v_type = VarType::VAR_STRING;
                    }
                    add_defer(c"delete", 1, &raw mut tv);
                }

                let write_ok = if argvars[0].v_type == VarType::VAR_BLOB {
                    argvars[0].vval.v_blob().is_null()
                        || write_blob(&mut fp, &*argvars[0].vval.v_blob())
                } else if argvars[0].v_type == VarType::VAR_STRING {
                    write_string(&mut fp, argvars[0].vval.v_string())
                } else {
                    write_list(&mut fp, argvars[0].vval.v_list().as_ref(), binary)
                };
                if write_ok {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(0);
                }
                error = file_close(&mut fp, do_fsync);
                if error != 0 {
                    semsg!(
                        gettext(c"E80: Error when closing file %s: %s"),
                        fname,
                        uv_strerror(error),
                    );
                }
            }
        }
    }
}

/// "browse(save, title, initdir, default)" function
pub fn f_browse(_argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
    rettv.v_type = VarType::VAR_STRING;
}

/// "browsedir(title, initdir)" function
pub fn f_browsedir(argvars: &mut [typval_T], rettv: &mut typval_T, fptr: EvalFuncData) {
    f_browse(argvars, rettv, fptr);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fileio::{S_IFDIR, S_IFIFO, S_IFREG};
    use crate::os::fs::S_IFLNK;

    #[test]
    fn stat_mode_predicates_identify_file_types() {
        assert!(s_isreg(S_IFREG | 0o644));
        assert!(!s_isdir(S_IFREG | 0o644));
        assert!(s_isdir(S_IFDIR | 0o755));
        assert!(s_islnk(S_IFLNK | 0o777));
        assert!(s_ischr(S_IFCHR_VAL | 0o600));
        assert!(s_isfifo(S_IFIFO | 0o600));
    }
}
