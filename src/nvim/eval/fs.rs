#![forbid(unsafe_code)]

//! Filesystem related builtin functions (`src/nvim/eval/fs.c`).

use crate::api::private::helpers::{MAXLNUM, SID_API_CLIENT, find_buffer_by_handle};
use crate::api::private::validate::{Error, ErrorType};
use crate::buffer::FileBuf;
use crate::cmdexpand::{
    EXPAND_FILES, WILD_ALL, WILD_ALL_KEEP, WILD_ALLLINKS, WILD_ICASE, WILD_KEEP_ALL, WILD_SILENT,
    WILD_USE_NL, expand_one, globpath,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TvBlob, TvList, TypVal, VarType, Varnumber, tv_blob_alloc_ret, tv_blob_len,
    tv_check_for_nonempty_string_arg, tv_check_for_string_arg, tv_check_str_or_nr, tv_clear,
    tv_get_bool_chk, tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf,
    tv_get_string_buf_chk, tv_get_string_chk, tv_list_alloc_ret, tv_list_append_string,
    tv_list_first, tv_list_item_remove, tv_list_len, tv_list_set_ret,
};
use crate::eval::userfunc::{add_defer, can_add_defer};
use crate::eval::vars::{VV_VAL, prepare_vimvar, restore_vimvar, set_vim_var_string};
use crate::eval::window::find_win_by_nr;
use crate::eval::{do_string_sub, eval_expr_typval};
use crate::event::r#loop::uv_strerror;
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{do_chdir, vim_mkdir_emsg};
use crate::file_search::{FfSearchCtx, FindFileWhat, find_file_in_path_option};
#[cfg(not(windows))]
use crate::fileio::os_readlink;
use crate::fileio::{
    Off, SEEK_END, SEEK_SET, delete_recursive, file_pat_to_reg_pat, readdir_core, vim_copyfile,
    vim_rename, vim_tempname,
};
use crate::globals::*;
use crate::mbyte::utfc_slice2len;
use crate::memory::bytes_to_cstring;
use crate::message::{emsg, semsg};
use crate::option_vars::*;
use crate::os::env::{ExpandCtx, expand_env_save, home_replace};
use crate::os::fileio::{
    FileDescriptor, K_FILE_APPEND, K_FILE_CREATE, K_FILE_MK_DIR, K_FILE_TRUNCATE, file_close,
    file_flush, file_open, file_write,
};
use crate::os::fs::{
    CFile, FileInfo, MAXPATHL, os_can_exe, os_dirname, os_file_is_readable, os_file_is_writable,
    os_fileinfo, os_fileinfo_fd, os_fileinfo_link, os_fileinfo_size, os_fileinfo2, os_getperm,
    os_isdir, os_mkdir_recurse, os_remove, os_rmdir, s_isblk, s_isdir, s_isfifo, s_islnk, s_isreg,
    s_issock,
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
use crate::strings::{gettext, vim_strsave_shellescape};
use crate::window::{TabPageInfo, WinInfoInfo, find_tabpage};
use std::ffi::CStr;

use crate::api::vim::K_CD_SCOPE_GLOBAL;
use crate::charset::{NL, NUL};
use crate::regexp::NUMBUFLEN;

pub(crate) const K_LIST_LEN_UNKNOWN: isize = -1;

pub(crate) const VALID_PATH: i32 = 1;
pub(crate) const VALID_HEAD: i32 = 2;

const K_CD_SCOPE_INVALID: i32 = -1;
pub(crate) const K_CD_SCOPE_BUFFER: i32 = 0;
pub(crate) const K_CD_SCOPE_WINDOW: i32 = 1;
pub(crate) const K_CD_SCOPE_TABPAGE: i32 = 2;

const S_IFMT_MASK: u64 = 0xF000;
const S_IFCHR_VAL: u64 = 0x2000;

const E_ERROR_WHILE_WRITING_STR: &CStr = c"E80: Error while writing: %s";

#[inline]
fn s_ischr(m: u64) -> bool {
    (m & S_IFMT_MASK) == S_IFCHR_VAL
}

/// Adjust a filename, according to a string of modifiers.
/// When there is an error, returns -1 and clears `fbuf`.
pub fn modify_fname(
    src_bytes: &[u8],
    tilde_file: bool,
    usedlen: &mut usize,
    fbuf: &mut Vec<u8>,
    fname_off: &mut usize,
    fnamelen: &mut usize,
    use_shellslash: bool,
) -> i32 {
    let mut off = *fname_off;
    let mut valid: i32 = 0;
    let mut dirname = [0u8; MAXPATHL];
    let mut has_fullname = false;
    let mut has_homerelative = false;
    let mut didit = false;

    loop {
        // ":p" - full path/file_name
        if src_bytes
            .get(*usedlen..)
            .is_some_and(|r| r.starts_with(b":p"))
        {
            has_fullname = true;

            valid |= VALID_PATH;
            *usedlen += 2;

            // Expand "~/path" for all systems and "~user/path" for Unix
            let fname_bytes = crate::memory::bytes_until_nul(&fbuf[off..]);
            let tilde_cond = fname_bytes.first() == Some(&b'~')
                && (cfg!(unix)
                    || fname_bytes.get(1) == Some(&b'/')
                    || (cfg!(windows) && fname_bytes.get(1) == Some(&b'\\'))
                    || fname_bytes.len() == 1)
                && !(tilde_file && fname_bytes.len() == 1);
            if tilde_cond {
                let exp = expand_env_save(crate::memory::bytes_as_cstr(&fbuf[off..]));
                *fbuf = exp.into_bytes_with_nul();
                off = 0;
            }

            // When "/." or "/.." is used: force expansion to get rid of it.
            let fname_bytes = crate::memory::bytes_until_nul(&fbuf[off..]);
            let mut p_idx = 0usize;
            while p_idx < fname_bytes.len() {
                let rem = &fname_bytes[p_idx..];
                if vim_ispathsep(rem[0] as i32)
                    && rem.get(1) == Some(&b'.')
                    && (rem.len() == 2
                        || vim_ispathsep(rem[2] as i32)
                        || (rem[2] == b'.' && (rem.len() == 3 || vim_ispathsep(rem[3] as i32))))
                {
                    break;
                }
                p_idx += utfc_slice2len(rem).min(rem.len());
            }
            let has_dot_comp = p_idx < fname_bytes.len();

            // FullName_save() is slow, don't use it when not needed.
            if has_dot_comp
                || !vim_isAbsName(fname_bytes)
                || (cfg!(windows) && matches!(fname_bytes.first(), Some(b'/' | b'\\')))
            {
                let Some(full) = FullName_save(
                    Some(crate::memory::bytes_as_cstr(&fbuf[off..])),
                    has_dot_comp,
                ) else {
                    fbuf.clear();
                    *fname_off = 0;
                    return -1;
                };
                *fbuf = full.into_bytes_with_nul();
                off = 0;
            }

            // Append a path separator to a directory.
            if os_isdir(crate::memory::bytes_as_cstr(&fbuf[off..])) {
                // Make room for one or two extra characters.
                let cur_bytes = crate::memory::bytes_until_nul(&fbuf[off..]);
                let cur_len = cur_bytes.len();
                let mut v = vec![0u8; cur_len + 3];
                v[..cur_len].copy_from_slice(cur_bytes);
                add_pathsep(&mut v);
                *fbuf = v;
                off = 0;
            }
        }

        let mut c: i32;

        // ":." - path relative to the current directory
        // ":~" - path relative to the home directory
        // ":8" - shortname path - postponed till after
        while src_bytes.get(*usedlen) == Some(&b':') && {
            c = src_bytes.get(*usedlen + 1).copied().unwrap_or(0) as i32;
            c == b'.' as i32 || c == b'~' as i32 || c == b'8' as i32
        } {
            *usedlen += 2;
            if c == b'8' as i32 {
                continue;
            }
            let mut pbuf: Option<Vec<u8>> = None;
            let mut has_p = true;
            // Need full path first (use expand_env() to remove a "~/")
            if !has_fullname && !has_homerelative {
                if fbuf.get(off) == Some(&b'~') {
                    let exp = expand_env_save(crate::memory::bytes_as_cstr(&fbuf[off..]))
                        .into_bytes_with_nul();
                    pbuf = Some(exp);
                } else if let Some(full) =
                    FullName_save(Some(crate::memory::bytes_as_cstr(&fbuf[off..])), false)
                {
                    pbuf = Some(full.into_bytes_with_nul());
                } else {
                    has_p = false;
                }
            }

            has_fullname = false;

            if has_p {
                let mut dirnamelen: usize = 0;
                let p_cstr = match pbuf {
                    Some(ref pb) => crate::memory::bytes_as_cstr(pb),
                    None => crate::memory::bytes_as_cstr(&fbuf[off..]),
                };

                if c == b'.' as i32 {
                    os_dirname(&mut dirname);
                    if has_homerelative {
                        let s_cstr = crate::memory::cstring_until_nul(&dirname);
                        dirnamelen =
                            home_replace(None, Some(s_cstr.as_c_str()), &mut dirname, true);
                    }

                    if dirnamelen == 0 {
                        dirnamelen = crate::memory::bytes_until_nul(&dirname).len();
                    }

                    // Do not call shorten_fname() here since it removes the prefix
                    // even though the path does not have a prefix.
                    if path_cmp(
                        opt_vars().p_fic,
                        p_cstr,
                        crate::memory::bytes_as_cstr(&dirname),
                        dirnamelen,
                    ) == 0
                    {
                        let p_bytes = p_cstr.to_bytes();
                        let p_rem = p_bytes.get(dirnamelen..).unwrap_or(&[]);
                        if p_rem.first().is_some_and(|&b| vim_ispathsep(b as i32)) {
                            let skip_seps = p_rem
                                .iter()
                                .take_while(|&&b| vim_ispathsep(b as i32))
                                .count();
                            let advance = dirnamelen + skip_seps;
                            if let Some(pb) = pbuf.take() {
                                *fbuf = pb;
                                off = advance;
                            } else {
                                off += advance;
                            }
                        }
                    }
                } else {
                    dirnamelen = home_replace(None, Some(p_cstr), &mut dirname, true);
                    // Only replace it when it starts with '~'
                    if dirname[0] == b'~' {
                        let mut v = vec![0u8; dirnamelen + 1];
                        v[..dirnamelen].copy_from_slice(&dirname[..dirnamelen]);
                        *fbuf = v;
                        off = 0;
                        has_homerelative = true;
                    }
                }
            }
        }

        let mut file_info = FileInfo::new();
        os_fileinfo2(crate::memory::bytes_as_cstr(&fbuf[off..]), &mut file_info);
        let mut s_off = 0usize;
        if src_bytes
            .get(*usedlen..)
            .is_some_and(|r| r.starts_with(b":h"))
        {
            s_off = off + file_info.rest_off;
            off += file_info.prefix_off;
        }

        let fname_cstr = crate::memory::bytes_as_cstr(&fbuf[off..]);
        let mut tail_off =
            off + (path_tail(fname_cstr).as_ptr() as usize - fname_cstr.as_ptr() as usize);
        *fnamelen = fname_cstr.to_bytes().len();

        // ":h" - head, remove "/file_name", can be repeated
        // Don't remove the logical root, see `FileInfo`.
        while src_bytes
            .get(*usedlen..)
            .is_some_and(|r| r.starts_with(b":h"))
        {
            valid |= VALID_HEAD;
            *usedlen += 2;
            while tail_off > s_off && after_pathsep(&fbuf[s_off..tail_off]) != 0 {
                let back =
                    crate::mbyte::utf_head_off(&fbuf[off..], tail_off - off - 1) as usize + 1;
                tail_off -= back;
            }
            *fnamelen = if tail_off <= s_off {
                s_off.saturating_sub(off)
            } else {
                tail_off.saturating_sub(off)
            };
            if *fnamelen == 0 {
                // Result is empty.  Turn it into "." to make ":cd %:h" work.
                *fbuf = vec![b'.', 0];
                off = 0;
                tail_off = 0;
                s_off = 0;
                *fnamelen = 1;
            } else {
                while tail_off > s_off && after_pathsep(&fbuf[s_off..tail_off]) == 0 {
                    let back =
                        crate::mbyte::utf_head_off(&fbuf[off..], tail_off - off - 1) as usize + 1;
                    tail_off -= back;
                }
            }
        }

        // ":8" - shortname
        if src_bytes
            .get(*usedlen..)
            .is_some_and(|r| r.starts_with(b":8"))
        {
            *usedlen += 2;
        }

        // ":t" - tail, just the basename
        if src_bytes
            .get(*usedlen..)
            .is_some_and(|r| r.starts_with(b":t"))
        {
            *usedlen += 2;
            *fnamelen -= tail_off.wrapping_sub(off);
            off = tail_off;
        }

        // ":e" - extension, can be repeated
        // ":r" - root, without extension, can be repeated
        while src_bytes.get(*usedlen) == Some(&b':')
            && matches!(src_bytes.get(*usedlen + 1), Some(b'e' | b'r'))
        {
            let mod_ch = src_bytes[*usedlen + 1];
            // find a '.' in the tail:
            // - for second :e: before the current fname
            // - otherwise: The last '.'
            let is_second_e = off > tail_off;
            let mut s_idx: isize = if mod_ch == b'e' && is_second_e {
                off as isize - 2
            } else {
                (off + *fnamelen) as isize - 1
            };

            while s_idx > tail_off as isize {
                if fbuf.get(s_idx as usize) == Some(&b'.') {
                    break;
                }
                s_idx -= 1;
            }
            if mod_ch == b'e' {
                if s_idx > tail_off as isize {
                    // we stopped at a '.' (so anchor to &'.' + 1)
                    let newstart = (s_idx + 1) as usize;
                    let distance_stepped_back = off.wrapping_sub(newstart);
                    *fnamelen = (*fnamelen).wrapping_add(distance_stepped_back);
                    off = newstart;
                } else if off <= tail_off {
                    *fnamelen = 0;
                }
            } else {
                // :r - Remove one extension
                let max_off = tail_off.max(off) as isize;
                if s_idx > max_off {
                    *fnamelen = (s_idx as usize) - off;
                }
            }
            *usedlen += 2;
        }

        #[cfg(windows)]
        if !didit && use_shellslash && !fbuf.is_empty() {
            slash_adjust(&mut fbuf[off..]);
        }
        #[cfg(not(windows))]
        let _ = use_shellslash;

        // ":s?pat?foo?" - substitute
        // ":gs?pat?foo?" - global substitute
        let mut repeat_loop = false;
        if src_bytes.get(*usedlen) == Some(&b':')
            && (src_bytes.get(*usedlen + 1) == Some(&b's')
                || src_bytes
                    .get(*usedlen + 1..)
                    .is_some_and(|r| r.starts_with(b"gs")))
        {
            let mut flags = c"";
            let mut s_idx = *usedlen + 2;
            if src_bytes.get(*usedlen + 1) == Some(&b'g') {
                flags = c"g";
                s_idx += 1;
            }

            let sep = src_bytes.get(s_idx).copied().unwrap_or(0);
            s_idx += 1;
            if sep != 0 {
                // find end of pattern
                if let Some(pat_len) = src_bytes
                    .get(s_idx..)
                    .and_then(|r| r.iter().position(|&b| b == sep))
                {
                    let pat = bytes_to_cstring(&src_bytes[s_idx..s_idx + pat_len]);
                    s_idx += pat_len + 1;
                    // find end of substitution
                    if let Some(sub_len) = src_bytes
                        .get(s_idx..)
                        .and_then(|r| r.iter().position(|&b| b == sep))
                    {
                        let sub = bytes_to_cstring(&src_bytes[s_idx..s_idx + sub_len]);
                        let mut str_copy = vec![0u8; *fnamelen + 1];
                        if *fnamelen > 0 {
                            str_copy[..*fnamelen].copy_from_slice(&fbuf[off..off + *fnamelen]);
                        }
                        *usedlen = s_idx + sub_len + 1;
                        let sub_res = do_string_sub(
                            crate::memory::bytes_as_cstr(&str_copy),
                            *fnamelen,
                            &pat,
                            &sub,
                            None,
                            flags,
                        );
                        *fnamelen = sub_res.as_bytes().len();
                        *fbuf = sub_res.into_bytes_with_nul();
                        off = 0;
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

        if src_bytes
            .get(*usedlen..)
            .is_some_and(|r| r.starts_with(b":S"))
        {
            let esc = vim_strsave_shellescape(&fbuf[off..off + *fnamelen], false, false);
            *fnamelen = esc.as_bytes().len();
            *fbuf = esc.into_bytes_with_nul();
            off = 0;
            *usedlen += 2;
        }

        break;
    }

    *fname_off = off;
    valid
}

/// "chdir(dir)" function
pub fn f_chdir(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    if check_secure() {
        return;
    }

    if argvars[0].v_type != VarType::VarString {
        // Returning an empty string means it failed.
        // No error message, for historic reasons.
        return;
    }

    // Return the current directory
    let mut cwd = vec![0u8; MAXPATHL];
    if os_dirname(&mut cwd) {
        #[cfg(windows)]
        slash_adjust(&mut cwd);
        crate::eval::typval::tv_set_string(
            rettv,
            Some(CStr::from_bytes_until_nul(&cwd).unwrap_or_default()),
        );
    }

    let mut scope: i32 = K_CD_SCOPE_GLOBAL;
    if argvars[1].v_type != VarType::VarUnknown {
        let s = tv_get_string(&argvars[1]);
        let s_bytes = s.to_bytes();
        if s_bytes == b"global" {
            scope = K_CD_SCOPE_GLOBAL;
        } else if s_bytes == b"tabpage" {
            scope = K_CD_SCOPE_TABPAGE;
        } else if s_bytes == b"window" {
            scope = K_CD_SCOPE_WINDOW;
        } else if s_bytes == b"buffer" {
            scope = K_CD_SCOPE_BUFFER;
        } else {
            semsg!(gettext(E_INVARG_NVAL), c"scope", s);
            return;
        }
    } else if curbuf_mut().b_localdir.is_some() {
        scope = K_CD_SCOPE_BUFFER;
    } else if curwin_mut().w_localdir.is_some() {
        scope = K_CD_SCOPE_WINDOW;
    } else if curtab_mut().tp_localdir.is_some() {
        scope = K_CD_SCOPE_TABPAGE;
    }

    if !do_chdir(argvars[0].vval.v_string_cstr(), scope) {
        // Directory change failed
        tv_clear(rettv);
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "delete()" function
pub fn f_delete(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    if check_secure() {
        return;
    }

    let name = tv_get_string(&argvars[0]);
    if name.to_bytes().is_empty() {
        emsg(gettext(E_INVARG));
        return;
    }

    let mut nbuf = [0u8; NUMBUFLEN];
    let flags = if argvars[1].v_type != VarType::VarUnknown {
        tv_get_string_buf(&argvars[1], &mut nbuf)
    } else {
        c""
    };

    let flags_bytes = flags.to_bytes();
    if flags_bytes.is_empty() {
        // delete a file
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(if os_remove(name) == 0 { 0 } else { -1 });
    } else if flags_bytes == b"d" {
        // delete an empty directory
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(if os_rmdir(name) == 0 { 0 } else { -1 });
    } else if flags_bytes == b"rf" {
        // delete a directory recursively
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(Varnumber::from(delete_recursive(name)));
    } else {
        semsg!(gettext(E_INVEXPR2), flags);
    }
}

/// "executable()" function
pub fn f_executable(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_string_arg(argvars, 0) {
        return;
    }

    // Check in $PATH and also check directly if there is a directory name
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(os_can_exe(
        tv_get_string(&argvars[0]),
        None,
        true,
    )));
}

/// "exepath()" function
pub fn f_exepath(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_nonempty_string_arg(argvars, 0) {
        return;
    }

    let mut path: Option<std::ffi::CString> = None;

    os_can_exe(tv_get_string(&argvars[0]), Some(&mut path), true);

    rettv.v_type = VarType::VarString;
    if let Some(p) = path {
        #[cfg(windows)]
        {
            let mut v = p.into_bytes_with_nul();
            slash_adjust(&mut v);
            crate::eval::typval::tv_set_string(
                rettv,
                Some(CStr::from_bytes_until_nul(&v).unwrap_or_default()),
            );
        }
        #[cfg(not(windows))]
        {
            crate::eval::typval::tv_set_string(rettv, Some(p.as_c_str()));
        }
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "filecopy()" function
pub fn f_filecopy(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
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
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(vim_copyfile(
            tv_get_string(&argvars[0]),
            tv_get_string(&argvars[1]),
        )));
    }
}

/// "filereadable()" function
pub fn f_filereadable(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let p = tv_get_string(&argvars[0]);
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
        !p.to_bytes().is_empty() && !os_isdir(p) && os_file_is_readable(p),
    ));
}

/// @return  0 for not writable
///          1 for writable file
///          2 for a dir which we have rights to write into.
pub fn f_filewritable(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let filename = tv_get_string(&argvars[0]);
    rettv.vval =
        crate::eval::typval::TypvalVval::Number(Varnumber::from(os_file_is_writable(filename)));
}

fn findfilendir(argvars: &mut [TypVal], rettv: &mut TypVal, find_what: FindFileWhat) {
    let mut fresult: Option<std::ffi::CString> = None;
    let path_owned = if curbuf_mut().b_p_path.is_empty() {
        opt_vars().p_path.to_owned()
    } else {
        curbuf_mut().b_p_path.to_owned()
    };
    let mut path = path_owned.as_c_str();
    let mut count: i32 = 1;
    let mut first = true;
    let mut error = false;

    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    rettv.v_type = VarType::VarString;

    let fname = tv_get_string(&argvars[0]);

    let mut pathbuf = [0u8; NUMBUFLEN];
    if argvars[1].v_type != VarType::VarUnknown {
        if let Some(p) = tv_get_string_buf_chk(&argvars[1], &mut pathbuf) {
            if !p.to_bytes().is_empty() {
                path = p;
            }

            if argvars[2].v_type != VarType::VarUnknown {
                count = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
            }
        } else {
            error = true;
        }
    }

    if count < 0 {
        tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_UNKNOWN);
    }

    if !fname.is_empty() && !error {
        let mut file_to_find: Option<std::ffi::CString> = None;
        let mut search_ctx: Option<FfSearchCtx> = None;

        loop {
            fresult = find_file_in_path_option(
                if first { fname.to_bytes() } else { &[] },
                0,
                first,
                path,
                find_what,
                curbuf_mut().ffname_opt(),
                if find_what == FindFileWhat::Dir {
                    c""
                } else {
                    curbuf_mut().b_p_sua.as_cstr()
                },
                &mut file_to_find,
                &mut search_ctx,
            );
            first = false;

            if let Some(ref res) = fresult
                && rettv.v_type == VarType::VarList
            {
                tv_list_append_string(rettv.vval.v_list_ref(), Some(res.as_c_str()), -1);
            }

            let keep_going = rettv.v_type == VarType::VarList || {
                count -= 1;
                count > 0
            };
            if !keep_going || fresult.is_none() {
                break;
            }
        }
    }

    if rettv.v_type == VarType::VarString {
        crate::eval::typval::tv_set_string(&mut *rettv, fresult.as_deref());
    }
}

/// "finddir({fname}[, {path}[, {count}]])" function
pub fn f_finddir(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    findfilendir(argvars, rettv, FindFileWhat::Dir);
}

/// "findfile({fname}[, {path}[, {count}]])" function
pub fn f_findfile(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    findfilendir(argvars, rettv, FindFileWhat::File);
}

/// "fnamemodify({fname}, {mods})" function
pub fn f_fnamemodify(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut len: usize;
    let mut buf = [0u8; NUMBUFLEN];
    let fname_cstr = tv_get_string_chk(&argvars[0]);
    let mods_cstr = tv_get_string_buf_chk(&argvars[1], &mut buf);
    let (Some(fname_cstr), Some(mods_cstr)) = (fname_cstr, mods_cstr) else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
        return;
    };

    #[cfg(windows)]
    let initial_cstr = path_to_slash_save(Some(fname_cstr)).unwrap_or_default();
    #[cfg(not(windows))]
    let initial_cstr = fname_cstr.to_owned();
    len = initial_cstr.as_bytes().len();
    let mut fbuf = initial_cstr.into_bytes_with_nul();
    let mut fname_off = 0usize;
    let mods_bytes = mods_cstr.to_bytes();
    if !mods_bytes.is_empty() {
        let mut usedlen: usize = 0;
        if modify_fname(
            mods_bytes,
            false,
            &mut usedlen,
            &mut fbuf,
            &mut fname_off,
            &mut len,
            false,
        ) == -1
        {
            rettv.v_type = VarType::VarString;
            rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
            return;
        }
    }

    crate::eval::typval::tv_set_bytes(rettv, &fbuf[fname_off..fname_off + len]);
}

/// Resolves the `[{winnr}[, {tabnr}[, {bufnr}]]]` arguments of `getcwd()`/`haslocaldir()` to the
/// requested scope and the scope objects to inspect.
fn getcwd_scope_args(
    argvars: &mut [TypVal],
    default_scope: i32,
    scope: &mut i32,
    win: &mut Option<&'static mut WinInfoInfo>,
    tp: &mut Option<&'static mut TabPageInfo>,
    buf: &mut Option<&'static mut FileBuf>,
) -> bool {
    const K_WIN_ARG: usize = 0;
    const K_TAB_ARG: usize = 1;
    const K_BUF_ARG: usize = 2;
    let mut argv = [0_i32; 3];
    let mut argc: usize = 0;

    *scope = default_scope;
    *win = Some(curwin_mut());
    *tp = Some(curtab_mut());
    *buf = Some(curbuf_mut());

    while argc < 3 {
        // If there is no argument there are no more scopes after it, break out.
        if argvars[argc].v_type == VarType::VarUnknown {
            break;
        }
        if argvars[argc].v_type != VarType::VarNumber {
            emsg(gettext(E_INVARG));
            return false;
        }
        argv[argc] = argvars[argc].vval.v_number() as i32;
        // It is an error for the scope number to be less than `-1`.
        if argv[argc] < -1 {
            emsg(gettext(E_INVARG));
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
            if argv[K_WIN_ARG] >= 0 {
                *scope = K_CD_SCOPE_WINDOW; // (X)
            } else {
                *scope = K_CD_SCOPE_TABPAGE; // (-1)
            }
        }
        2 => {
            if argv[K_WIN_ARG] >= 0 {
                *scope = K_CD_SCOPE_WINDOW; // (X, ...)
            } else if argv[K_TAB_ARG] >= 0 {
                *scope = K_CD_SCOPE_TABPAGE; // (-1, X)
            } else {
                *scope = K_CD_SCOPE_GLOBAL; // (-1, -1)
            }
        }
        3 => {
            if argv[K_BUF_ARG] >= 0 {
                *scope = K_CD_SCOPE_BUFFER; // (-1, -1, X)
            } else {
                *scope = K_CD_SCOPE_GLOBAL; // (-1, -1, -1)
            }
        }
        _ => unreachable!(),
    }

    if *scope == K_CD_SCOPE_BUFFER {
        if argv[K_WIN_ARG] >= 0 || argv[K_TAB_ARG] >= 0 {
            emsg(gettext(
                c"E5006: Window and tab scope must be -1 when using buffer scope",
            ));
            return false;
        }
        if argv[K_BUF_ARG] > 0 {
            let mut err = Error::new();
            *buf = find_buffer_by_handle(argv[K_BUF_ARG], &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                emsg(gettext(c"E5007: Cannot find buffer number."));
                crate::api::private::helpers::api_clear_error(&mut err);
                return false;
            }
        }
    }

    // Find the tabpage by number.
    if argv[K_TAB_ARG] > 0 {
        *tp = find_tabpage(argv[K_TAB_ARG]);

        if tp.is_none() {
            emsg(gettext(c"E5000: Cannot find tab number."));
            return false;
        }
    }

    // Find the window in `tp` by number.
    if argv[K_WIN_ARG] > 0 {
        *win = find_win_by_nr(&argvars[0], tp.as_deref_mut());

        if win.is_none() {
            emsg(gettext(c"E5002: Cannot find window number."));
            return false;
        }
    }

    true
}

/// `getcwd([{win}[, {tab}[, {buf}]]])` function
pub fn f_getcwd(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut scope: i32 = K_CD_SCOPE_INVALID;
    let mut win: Option<&'static mut WinInfoInfo> = None;
    let mut tp: Option<&'static mut TabPageInfo> = None;
    let mut buf: Option<&'static mut FileBuf> = None;

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    // Without arguments (K_CD_SCOPE_INVALID), the effective working directory is returned.
    if !getcwd_scope_args(
        argvars,
        K_CD_SCOPE_INVALID,
        &mut scope,
        &mut win,
        &mut tp,
        &mut buf,
    ) {
        return;
    }

    let mut from: Option<&CStr> = None; // The local directory to copy.
    let mut cwd = vec![0u8; MAXPATHL];

    let mut cur_scope = scope;
    if cur_scope == K_CD_SCOPE_WINDOW {
        from = win.as_ref().unwrap().w_localdir.as_deref();

        if from.is_none() {
            cur_scope = K_CD_SCOPE_TABPAGE;
        }
    }
    if cur_scope == K_CD_SCOPE_TABPAGE {
        from = tp.as_ref().unwrap().tp_localdir.as_deref();

        if from.is_none() {
            cur_scope = K_CD_SCOPE_BUFFER;
        }
    }
    if cur_scope == K_CD_SCOPE_BUFFER {
        // Only a buffer target reaches this with `scope` unchanged: a buffer belongs to no
        // particular window or tabpage, so it skips those and falls back to global dir.
        from = if scope == K_CD_SCOPE_BUFFER {
            buf.as_ref().unwrap().b_localdir.as_deref()
        } else {
            None
        };
        if from.is_none() {
            cur_scope = K_CD_SCOPE_GLOBAL;
        }
    }
    if cur_scope == K_CD_SCOPE_GLOBAL {
        if let Some(ref gdir) = g_st().borrow_mut().globaldir {
            from = Some(gdir.as_c_str());
        } else {
            cur_scope = K_CD_SCOPE_INVALID;
        }
    }
    if cur_scope == K_CD_SCOPE_INVALID && !os_dirname(&mut cwd) {
        from = Some(c""); // Return empty string on failure.
    }

    if let Some(from_cstr) = from {
        crate::memory::copy_bytes_to_c_buf(&mut cwd, from_cstr.to_bytes());
    }

    #[cfg(windows)]
    slash_adjust(&mut cwd);
    crate::eval::typval::tv_set_string(
        rettv,
        Some(CStr::from_bytes_until_nul(&cwd).unwrap_or_default()),
    );
}

/// "getfperm({fname})" function
pub fn f_getfperm(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let flags = *b"rwx";

    let filename = tv_get_string(&argvars[0]);
    let file_perm = os_getperm(filename);
    if file_perm >= 0 {
        let mut perm = [b'-'; 9];
        for i in 0..9 {
            if (file_perm & (1 << (8 - i))) != 0 {
                perm[i] = flags[i % 3];
            }
        }
        crate::eval::typval::tv_set_string_len(rettv, Some(&perm));
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "getfsize({fname})" function
pub fn f_getfsize(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let fname = tv_get_string(&argvars[0]);

    let mut file_info = FileInfo::new();
    if os_fileinfo(fname, &mut file_info) {
        let filesize = os_fileinfo_size(&file_info);
        if s_isdir(file_info.stat.st_mode) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(filesize as Varnumber);

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
pub fn f_getftime(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let fname = tv_get_string(&argvars[0]);

    let mut file_info = FileInfo::new();
    if os_fileinfo(fname, &mut file_info) {
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(file_info.stat.st_mtim.tv_sec as Varnumber);
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    }
}

/// "getftype({fname})" function
pub fn f_getftype(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let fname = tv_get_string(&argvars[0]);

    let mut file_info = FileInfo::new();
    if os_fileinfo_link(fname, &mut file_info) {
        let mode = file_info.stat.st_mode;
        let t = if s_isreg(mode) {
            c"file"
        } else if s_isdir(mode) {
            c"dir"
        } else if s_islnk(mode) {
            c"link"
        } else if s_isblk(mode) {
            c"bdev"
        } else if s_ischr(mode) {
            c"cdev"
        } else if s_isfifo(mode) {
            c"fifo"
        } else if s_issock(mode) {
            c"socket"
        } else {
            c"other"
        };
        crate::eval::typval::tv_set_string(rettv, Some(t));
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "glob()" function
pub fn f_glob(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut options: i32 = WILD_SILENT | WILD_USE_NL;
    let mut xpc = ExpandCtx::new();
    let mut error = false;

    // When the optional second argument is non-zero, don't remove matches
    // for 'wildignore' and don't put matches for 'suffixes' at the end.
    rettv.v_type = VarType::VarString;
    if argvars[1].v_type != VarType::VarUnknown {
        if tv_get_bool_chk(&argvars[1], Some(&mut error)) != 0 {
            options |= WILD_KEEP_ALL;
        }
        if argvars[2].v_type != VarType::VarUnknown {
            if tv_get_bool_chk(&argvars[2], Some(&mut error)) != 0 {
                tv_list_set_ret(rettv, None);
            }
            if argvars[3].v_type != VarType::VarUnknown
                && tv_get_bool_chk(&argvars[3], Some(&mut error)) != 0
            {
                options |= WILD_ALLLINKS;
            }
        }
    }
    if !error {
        xpc.xp_context = EXPAND_FILES;
        if opt_vars().p_wic {
            options += WILD_ICASE;
        }
        if rettv.v_type == VarType::VarString {
            if let Some(exp) = expand_one(
                &mut xpc,
                Some(tv_get_string(&argvars[0])),
                None,
                options,
                WILD_ALL,
            ) {
                crate::eval::typval::tv_set_string(rettv, Some(exp.as_c_str()));
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
            }
        } else {
            expand_one(
                &mut xpc,
                Some(tv_get_string(&argvars[0])),
                None,
                options,
                WILD_ALL_KEEP,
            );
            tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);
            for f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
                tv_list_append_string(
                    rettv.vval.v_list_ref(),
                    Some(f.as_c_str()),
                    f.as_bytes().len() as isize,
                );
            }
        }
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "globpath()" function
pub fn f_globpath(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut flags: i32 = 0; // Flags for globpath.
    let mut error = false;

    // Return a string, or a list if the optional third argument is non-zero.
    rettv.v_type = VarType::VarString;

    if argvars[2].v_type != VarType::VarUnknown {
        // When the optional second argument is non-zero, don't remove matches
        // for 'wildignore' and don't put matches for 'suffixes' at the end.
        if tv_get_bool_chk(&argvars[2], Some(&mut error)) != 0 {
            flags |= WILD_KEEP_ALL;
        }

        if argvars[3].v_type != VarType::VarUnknown {
            if tv_get_bool_chk(&argvars[3], Some(&mut error)) != 0 {
                tv_list_set_ret(rettv, None);
            }
            if argvars[4].v_type != VarType::VarUnknown
                && tv_get_bool_chk(&argvars[4], Some(&mut error)) != 0
            {
                flags |= WILD_ALLLINKS;
            }
        }
    }

    let mut buf1 = [0u8; NUMBUFLEN];
    if let Some(file) = tv_get_string_buf_chk(&argvars[1], &mut buf1)
        && !error
    {
        let mut ga: Vec<std::ffi::CString> = Vec::new();
        globpath(tv_get_string(&argvars[0]), file, &mut ga, flags, false);

        if rettv.v_type == VarType::VarString {
            let joined = crate::path::concat_cstrings(&ga, c"\n");
            crate::eval::typval::tv_set_string(rettv, Some(joined.as_c_str()));
        } else {
            tv_list_alloc_ret(rettv, ga.len() as isize);
            for s in &ga {
                tv_list_append_string(
                    rettv.vval.v_list_ref(),
                    Some(s.as_c_str()),
                    s.as_bytes().len() as isize,
                );
            }
        }
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "glob2regpat()" function
pub fn f_glob2regpat(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let regpat =
        tv_get_string_chk(&argvars[0]).and_then(|pat| file_pat_to_reg_pat(pat.to_bytes(), None, 0));
    crate::eval::typval::tv_set_string(rettv, regpat.as_deref());
}

/// `haslocaldir([{win}[, {tab}[, {buf}]]])` function
pub fn f_haslocaldir(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut scope: i32 = K_CD_SCOPE_WINDOW;
    let mut win: Option<&'static mut WinInfoInfo> = None;
    let mut tp: Option<&'static mut TabPageInfo> = None;
    let mut buf: Option<&'static mut FileBuf> = None;

    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    // Without arguments, default to window scope.
    if !getcwd_scope_args(
        argvars,
        K_CD_SCOPE_WINDOW,
        &mut scope,
        &mut win,
        &mut tp,
        &mut buf,
    ) {
        return;
    }

    match scope {
        K_CD_SCOPE_WINDOW => {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
                win.unwrap().w_localdir.is_some(),
            ))
        }
        K_CD_SCOPE_BUFFER => {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
                buf.unwrap().b_localdir.is_some(),
            ))
        }
        K_CD_SCOPE_TABPAGE => {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
                tp.unwrap().tp_localdir.is_some(),
            ))
        }
        K_CD_SCOPE_GLOBAL => {
            // The global scope never has a local directory
        }
        _ => std::process::abort(),
    }
}

/// "isabsolutepath()" function
pub fn f_isabsolutepath(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(path_is_absolute(
        (tv_get_string(&argvars[0])).to_bytes(),
    )));
}

/// "isdirectory()" function
pub fn f_isdirectory(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(os_isdir(tv_get_string(
        &argvars[0],
    ))));
}

/// "mkdir()" function
pub fn f_mkdir(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut prot: i32 = 0o755;

    rettv.vval = crate::eval::typval::TypvalVval::Number(0);
    if check_secure() {
        return;
    }

    let mut buf = [0u8; NUMBUFLEN];
    let dir_in = tv_get_string_buf(&argvars[0], &mut buf);
    if dir_in.to_bytes().is_empty() {
        return;
    }
    let mut dir_vec = dir_in.to_bytes_with_nul().to_vec();
    let dir_cstr = crate::memory::bytes_as_cstr(&dir_vec);
    if path_tail(dir_cstr).is_empty() {
        // Remove trailing slashes.
        let sep_off =
            (path_tail_with_sep(dir_cstr).as_ptr() as usize) - (dir_cstr.as_ptr() as usize);
        dir_vec[sep_off] = NUL as u8;
    }
    let dir_cstr = crate::memory::bytes_as_cstr(&dir_vec);

    let mut defer = false;
    let mut defer_recurse = false;
    let mut created: Option<std::ffi::CString> = None;
    if argvars[1].v_type != VarType::VarUnknown {
        if argvars[2].v_type != VarType::VarUnknown {
            prot = tv_get_number_chk(&argvars[2], None) as i32;
            if prot == -1 {
                return;
            }
        }
        let arg2 = tv_get_string(&argvars[1]);
        defer = crate::strings::vim_haschr(arg2, b'D' as i32);
        defer_recurse = crate::strings::vim_haschr(arg2, b'R' as i32);
        if (defer || defer_recurse) && !can_add_defer() {
            return;
        }

        if crate::strings::vim_haschr(arg2, b'p' as i32) {
            let mut failed_dir: Option<std::ffi::CString> = None;
            let ret = os_mkdir_recurse(
                dir_cstr,
                prot,
                &mut failed_dir,
                (defer || defer_recurse).then_some(&mut created),
            );
            if ret != 0 {
                semsg!(gettext(E_MKDIR), failed_dir, uv_strerror(ret));
                rettv.vval = crate::eval::typval::TypvalVval::Number(0);
                return;
            }
            rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        }
    }
    if rettv.vval.v_number() == 0 {
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(vim_mkdir_emsg(
            dir_cstr, prot,
        )));
    }

    // Handle "D" and "R": deferred deletion of the created directory.
    if rettv.vval.v_number() == 1 && created.is_none() && (defer || defer_recurse) {
        created = FullName_save(Some(dir_cstr), false);
    }
    if let Some(created_cstr) = created {
        let mut tv = [TypVal::new(), TypVal::new()];
        crate::eval::typval::tv_set_string(&mut tv[0], Some(created_cstr.as_c_str()));
        crate::eval::typval::tv_set_string(
            &mut tv[1],
            Some(if defer_recurse { c"rf" } else { c"d" }),
        );
        add_defer(c"delete", &tv);
    }
}

/// "pathshorten()" function
pub fn f_pathshorten(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut trim_len: i32 = 1;

    if argvars[1].v_type != VarType::VarUnknown {
        trim_len = tv_get_number(&argvars[1]) as i32;
        if trim_len < 1 {
            trim_len = 1;
        }
    }

    if let Some(p) = tv_get_string_chk(&argvars[0]) {
        let mut v = p.to_owned().into_bytes_with_nul();
        shorten_dir_len(&mut v, trim_len);
        crate::eval::typval::tv_set_string(
            rettv,
            Some(CStr::from_bytes_until_nul(&v).unwrap_or_default()),
        );
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// Evaluate "expr" (= "context") for readdir().
pub(crate) fn readdir_checkitem(expr: &mut TypVal, name: &CStr) -> Varnumber {
    let mut argv = [const { TypVal::new() }; 2];
    let mut retval: Varnumber = 0;
    let mut error = false;

    if expr.v_type == VarType::VarUnknown {
        return 1;
    }

    let mut save_val = TypVal::new();
    prepare_vimvar(VV_VAL, &mut save_val);
    set_vim_var_string(VV_VAL, Some(name), -1);
    argv[0].v_type = VarType::VarString;
    argv[0].vval = crate::eval::typval::TypvalVval::from_cstr(name);

    let mut rettv = TypVal::new();
    if eval_expr_typval(&*expr, false, &mut argv, 1, &mut rettv) {
        retval = tv_get_number_chk(&rettv, Some(&mut error));
        if error {
            retval = -1;
        }

        tv_clear(&mut rettv);
    }

    set_vim_var_string(VV_VAL, None, 0);
    restore_vimvar(VV_VAL, &mut save_val);
    retval
}

/// "readdir()" function
pub fn f_readdir(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN);
    if check_secure() {
        return;
    }

    let [arg0, arg1, ..] = argvars else {
        unreachable!()
    };
    let path = tv_get_string(arg0);
    let mut entries: Vec<std::ffi::CString> = Vec::new();
    let ret = readdir_core(&mut entries, path, Some(arg1));
    if ret {
        for entry in &entries {
            tv_list_append_string(rettv.vval.v_list_ref(), Some(entry.as_c_str()), -1);
        }
    }
}

/// Read blob from file "fd".
/// Caller has allocated a blob in "rettv".
fn read_blob(fd: &CFile, rettv: &mut TypVal, mut offset: Off, size_arg: Off) -> bool {
    let mut file_info = FileInfo::new();
    if !os_fileinfo_fd(fd.fileno(), &mut file_info) {
        return false; // can't read the file, error
    }

    let mut size = size_arg;
    let file_size = os_fileinfo_size(&file_info) as Off;
    let whence = if offset >= 0 {
        // The size defaults to the whole file.  If a size is given it is
        // limited to not go past the end of the file.
        if size == -1 || (size > file_size - offset && !s_ischr(file_info.stat.st_mode)) {
            // size may become negative, checked below
            size = os_fileinfo_size(&file_info) as Off - offset;
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
    if offset != 0 && fd.seek(offset, whence) != 0 {
        return true;
    }

    let bv = &mut rettv.vval.v_blob_ref().bv_ga;
    bv.resize(size as usize, 0);
    if fd.read_bytes(bv) < bv.len() {
        // An empty blob is returned on error.
        crate::eval::typval::tv_blob_unref(rettv.vval.v_blob_opt());
        rettv.vval = crate::eval::typval::TypvalVval::Blob(crate::eval::typval::BlobId::NULL);
        return false;
    }
    true
}

/// "readfile()" or "readblob()" function
fn read_file_or_blob(argvars: &mut [TypVal], rettv: &mut TypVal, always_blob: bool) {
    let mut binary = false;
    let mut blob = always_blob;
    let mut buf = [0u8; (IOSIZE / 256) * 256]; // rounded to avoid odd + 1
    let io_size = buf.len();
    let mut prev: Vec<u8> = Vec::new(); // previously read bytes, if any
    let mut maxline: i64 = MAXLNUM;
    let mut offset: Off = 0;
    let mut size: Off = -1;

    if argvars[1].v_type != VarType::VarUnknown {
        if always_blob {
            offset = tv_get_number(&argvars[1]);
            if argvars[2].v_type != VarType::VarUnknown {
                size = tv_get_number(&argvars[2]);
            }
        } else {
            let arg1 = tv_get_string(&argvars[1]).to_bytes();
            if arg1 == b"b" {
                binary = true;
            } else if arg1 == b"B" {
                blob = true;
            }
            if argvars[2].v_type != VarType::VarUnknown {
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
        semsg!(gettext(E_ISADIR2), fname);
        return;
    }
    if fname.to_bytes().is_empty() {
        semsg!(gettext(E_NOTOPEN), gettext(c"<empty>"));
        return;
    }
    let Some(fd) = CFile::open(fname, c"rb") else {
        semsg!(gettext(E_NOTOPEN), fname);
        return;
    };

    if blob {
        if !read_blob(&fd, rettv, offset, size) {
            semsg!(gettext(E_CANT_READ_FILE_STR), fname);
        }
        return;
    }

    let l = rettv.vval.v_list_ref();
    while maxline < 0 || i64::from(tv_list_len(Some(&*l))) < maxline {
        let mut readlen = fd.read_bytes(&mut buf[..io_size]) as isize;

        let mut p_idx: usize = 0; // Position in buf.
        let mut start_idx: usize = 0; // Start of current line.
        while (readlen > 0 && p_idx < readlen as usize)
            || (readlen <= 0 && (!prev.is_empty() || binary))
        {
            if readlen <= 0 || buf[p_idx] == b'\n' {
                let mut len = p_idx.saturating_sub(start_idx);

                // Finished a line.  Remove CRs before NL.
                if readlen > 0 && !binary {
                    let line_bytes = &buf[start_idx..start_idx + len];
                    while len > 0 && line_bytes[len - 1] == b'\r' {
                        len -= 1;
                    }
                    // removal may cross back to the "prev" string
                    if len == 0 {
                        while prev.last() == Some(&b'\r') {
                            prev.pop();
                        }
                    }
                }
                if prev.is_empty() {
                    debug_assert!(len < i32::MAX as usize);

                    crate::eval::typval::tv_list_append_bytes(l, &buf[start_idx..start_idx + len]);
                } else {
                    prev.extend_from_slice(&buf[start_idx..start_idx + len]);

                    crate::eval::typval::tv_list_append_bytes(l, &prev);

                    prev.clear();
                }

                start_idx = p_idx + 1; // Step over newline.
                if maxline < 0 {
                    if i64::from(tv_list_len(Some(&*l))) > -maxline {
                        debug_assert_eq!(i64::from(tv_list_len(Some(&*l))), 1 + (-maxline));

                        let first_li = tv_list_first(Some(&*l)).unwrap();
                        tv_list_item_remove(l, first_li);
                    }
                } else if i64::from(tv_list_len(Some(&*l))) >= maxline {
                    debug_assert_eq!(i64::from(tv_list_len(Some(&*l))), maxline);

                    break;
                }
                if readlen <= 0 {
                    break;
                }
            } else if buf[p_idx] == 0 {
                buf[p_idx] = b'\n';
            } else if buf[p_idx] == 0xbf && !binary {
                // Check for utf8 "bom"; U+FEFF is encoded as EF BB BF.  Do this
                // when finding the BF and check the previous two bytes.
                let back1 = if p_idx >= 1 {
                    buf[p_idx - 1]
                } else {
                    prev.last().copied().unwrap_or(0)
                };
                let back2 = if p_idx >= 2 {
                    buf[p_idx - 2]
                } else if p_idx == 1 {
                    prev.last().copied().unwrap_or(0)
                } else if prev.len() >= 2 {
                    prev[prev.len() - 2]
                } else {
                    0
                };

                if back2 == 0xef && back1 == 0xbb {
                    // Usually a BOM is at the beginning of a file, and so at
                    // the beginning of a line; then we can just step over it.
                    if p_idx >= 2 && start_idx == p_idx - 2 {
                        start_idx = p_idx + 1;
                    } else {
                        // have to shuffle buf to close gap
                        let (dest_idx, adjust_prevlen) = if p_idx >= 2 {
                            (p_idx - 2, 0usize)
                        } else {
                            (0usize, 2 - p_idx)
                        };
                        let readlen_u = readlen.max(0) as usize;
                        if readlen_u > p_idx + 1 {
                            buf.copy_within(p_idx + 1..readlen_u, dest_idx);
                        }
                        readlen -= 3 - adjust_prevlen as isize;
                        prev.truncate(prev.len() - adjust_prevlen);
                        p_idx = dest_idx;
                        continue;
                    }
                }
            }
            p_idx += 1;
        }

        if (maxline >= 0 && i64::from(tv_list_len(Some(&*l))) >= maxline) || readlen <= 0 {
            break;
        }
        if start_idx < p_idx {
            prev.extend_from_slice(&buf[start_idx..p_idx]);
        }
    }
}

/// "readblob()" function
pub fn f_readblob(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if check_secure() {
        return;
    }

    read_file_or_blob(argvars, rettv, true);
}

/// "readfile()" function
pub fn f_readfile(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if check_secure() {
        return;
    }

    read_file_or_blob(argvars, rettv, false);
}

/// "rename({from}, {to})" function
pub fn f_rename(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if check_secure() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    } else {
        let mut buf = [0u8; NUMBUFLEN];
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(vim_rename(
            tv_get_string(&argvars[0]),
            tv_get_string_buf(&argvars[1], &mut buf),
        )));
    }
}

/// "resolve()" function
pub fn f_resolve(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    let fname = tv_get_string(&argvars[0]);
    #[cfg(windows)]
    {
        let mut v = os_resolve_shortcut(Some(fname));
        if v.is_none() && os_is_reparse_point_include(fname) {
            v = os_realpath_cstr(fname, MAXPATHL + 1);
        }
        if let Some(resolved) = v {
            crate::eval::typval::tv_set_string(rettv, Some(resolved.as_c_str()));
        } else {
            crate::eval::typval::tv_set_string(rettv, Some(fname));
        }
    }
    #[cfg(not(windows))]
    {
        let mut is_relative_to_current = false;
        let mut has_trailing_pathsep = false;
        let mut limit: i32 = 100;

        let mut p_vec = fname.to_bytes_with_nul().to_vec();
        let mut p_len = p_vec.len() - 1;

        if p_vec.first() == Some(&b'.')
            && (p_vec.get(1).is_some_and(|&b| vim_ispathsep(b as i32))
                || (p_vec.get(1) == Some(&b'.')
                    && p_vec.get(2).is_some_and(|&b| vim_ispathsep(b as i32))))
        {
            is_relative_to_current = true;
        }

        if p_len > 1 && after_pathsep(&p_vec[..p_len]) != 0 {
            has_trailing_pathsep = true;
            p_len -= 1;
            p_vec[p_len] = 0; // The trailing slash breaks readlink().
        }

        let p_cstr = crate::memory::bytes_as_cstr(&p_vec);
        let q = path_next_component(p_cstr);
        let mut remain_vec: Option<Vec<u8>> = None;
        if !q.is_empty() {
            // Separate the first path component in "p", and keep the
            // remainder (beginning with the path separator).
            let q_off = (q.as_ptr() as usize) - (p_cstr.as_ptr() as usize);
            let mut rem = Vec::with_capacity(p_len - (q_off - 1) + 1);
            rem.extend_from_slice(&p_vec[q_off - 1..p_len]);
            rem.push(0);
            remain_vec = Some(rem);
            p_len = q_off - 1;
            p_vec[p_len] = 0;
        }

        let mut buf_vec = vec![0u8; MAXPATHL + 1];

        loop {
            loop {
                let len = os_readlink(
                    crate::memory::bytes_as_cstr(&p_vec),
                    &mut buf_vec[..MAXPATHL],
                );
                if len <= 0 {
                    break;
                }
                let mut buf_len = len as usize;
                buf_vec[buf_len] = 0;

                let old_limit = limit;
                limit -= 1;
                if old_limit == 0 {
                    emsg(gettext(c"E655: Too many symbolic links (cycle?)"));
                    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
                    return;
                }

                // Ensure that the result will have a trailing path separator
                // if the argument has one.
                if remain_vec.is_none() && has_trailing_pathsep {
                    add_pathsep(&mut buf_vec);
                    buf_len = crate::memory::bytes_until_nul(&buf_vec).len();
                }

                // Separate the first path component in the link value and
                // concatenate the remainders.
                let start_off = usize::from(vim_ispathsep(buf_vec[0] as i32));
                let start_cstr = crate::memory::bytes_as_cstr(&buf_vec[start_off..]);
                let q = path_next_component(start_cstr);
                if !q.is_empty() {
                    let q_off =
                        start_off + ((q.as_ptr() as usize) - (start_cstr.as_ptr() as usize));
                    let head_bytes = &buf_vec[q_off - 1..buf_len];
                    let rem_old_len = remain_vec
                        .as_ref()
                        .map_or(0, |r| crate::memory::bytes_until_nul(r).len());
                    let mut new_rem = Vec::with_capacity(head_bytes.len() + rem_old_len + 1);
                    new_rem.extend_from_slice(head_bytes);
                    if let Some(ref rem) = remain_vec {
                        new_rem.extend_from_slice(&rem[..rem_old_len]);
                    }
                    new_rem.push(0);
                    remain_vec = Some(new_rem);
                    buf_len = q_off - 1;
                    buf_vec[buf_len] = 0;
                }

                let p_cstr = crate::memory::bytes_as_cstr(&p_vec);
                let mut q_off = (path_tail(p_cstr).as_ptr() as usize) - (p_cstr.as_ptr() as usize);
                if q_off > 0 && p_vec[q_off] == 0 {
                    // Ignore trailing path separator.
                    p_vec[q_off - 1] = 0;
                    let p_cstr = crate::memory::bytes_as_cstr(&p_vec);
                    q_off = (path_tail(p_cstr).as_ptr() as usize) - (p_cstr.as_ptr() as usize);
                }
                if q_off > 0 && !path_is_absolute(&buf_vec[..buf_len]) {
                    // Symlink is relative to directory of argument. Replace the
                    // symlink with the resolved name in the same directory.
                    let prefix_len = q_off;
                    let mut new_p = vec![0u8; prefix_len + buf_len + 1];
                    new_p[..prefix_len].copy_from_slice(&p_vec[..prefix_len]);
                    new_p[prefix_len..prefix_len + buf_len].copy_from_slice(&buf_vec[..buf_len]);
                    p_len = prefix_len + buf_len;
                    p_vec = new_p;
                } else {
                    let mut new_p = Vec::with_capacity(buf_len + 1);
                    new_p.extend_from_slice(&buf_vec[..buf_len]);
                    new_p.push(0);
                    p_len = buf_len;
                    p_vec = new_p;
                }
            }

            let Some(ref mut rem_v) = remain_vec else {
                break;
            };
            let rem_cstr = crate::memory::bytes_as_cstr(&rem_v[1..]);
            let q = path_next_component(rem_cstr);
            let q_off = 1 + ((q.as_ptr() as usize) - (rem_cstr.as_ptr() as usize));
            let comp_len = q_off - usize::from(!q.is_empty());
            let mut new_p = vec![0u8; p_len + comp_len + 1];
            new_p[..p_len].copy_from_slice(&p_vec[..p_len]);
            new_p[p_len..p_len + comp_len].copy_from_slice(&rem_v[..comp_len]);
            p_len += comp_len;
            p_vec = new_p;

            // Shorten "remain".
            if !q.is_empty() {
                let rem_tail_off = q_off - 1;
                let rem_len = crate::memory::bytes_until_nul(&rem_v[rem_tail_off..]).len() + 1;
                rem_v.copy_within(rem_tail_off..rem_tail_off + rem_len, 0);
            } else {
                remain_vec = None;
            }
        }

        // If the result is a relative path name, make it explicitly relative to
        // the current directory if and only if the argument had this form.
        if !vim_ispathsep(p_vec[0] as i32) {
            if is_relative_to_current
                && p_len > 0
                && !(p_vec[0] == b'.'
                    && (p_len == 1
                        || vim_ispathsep(p_vec[1] as i32)
                        || (p_vec[1] == b'.' && (p_len == 2 || vim_ispathsep(p_vec[2] as i32)))))
            {
                // Prepend "./".
                let mut new_p = vec![0u8; p_len + 3];
                new_p[0] = b'.';
                new_p[1] = b'/';
                new_p[2..2 + p_len].copy_from_slice(&p_vec[..p_len]);
                p_len += 2;
                p_vec = new_p;
            } else if !is_relative_to_current {
                // Strip leading "./".
                let mut q_idx = 0usize;
                while q_idx + 1 < p_len
                    && p_vec[q_idx] == b'.'
                    && vim_ispathsep(p_vec[q_idx + 1] as i32)
                {
                    q_idx += 2;
                }
                if q_idx > 0 {
                    p_vec.copy_within(2..=p_len, 0);
                    p_len -= 2;
                }
            }
        }

        // Ensure that the result will have no trailing path separator
        // if the argument had none.  But keep "/" or "//".
        if !has_trailing_pathsep && after_pathsep(&p_vec[..p_len]) != 0 {
            let p_cstr = crate::memory::bytes_as_cstr(&p_vec);
            let sep_off =
                (path_tail_with_sep(p_cstr).as_ptr() as usize) - (p_cstr.as_ptr() as usize);
            p_vec[sep_off] = 0;
        }

        crate::eval::typval::tv_set_string(
            rettv,
            Some(CStr::from_bytes_until_nul(&p_vec).unwrap_or_default()),
        );
    }

    if let Some(mut s) = crate::eval::typval::tv_take_string(rettv) {
        crate::memory::cstring_modify(&mut s, |b| {
            simplify_filename(b);
        });
        rettv.vval = crate::eval::typval::TypvalVval::from_cstring(s);
    }
}

/// "simplify()" function
pub fn f_simplify(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let p = tv_get_string(&argvars[0]);
    let mut v = (*p).to_owned().into_bytes_with_nul();
    simplify_filename(&mut v); // Simplify in place.
    crate::eval::typval::tv_set_string(
        rettv,
        Some(CStr::from_bytes_until_nul(&v).unwrap_or_default()),
    );
}

/// "tempname()" function
pub fn f_tempname(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if let Some(tn) = vim_tempname() {
        crate::eval::typval::tv_set_string(rettv, Some(tn.as_c_str()));
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// Write "list" of strings to file "fp".
fn write_list(fp: &mut FileDescriptor, list: Option<&TvList>, binary: bool) -> bool {
    let mut error: i32 = 0;
    let mut li = tv_list_first(list);
    'outer: while let Some(cur_li) = li {
        let Some(s_cstr) = tv_get_string_chk(&cur_li.li_tv) else {
            return false;
        };
        let s_bytes = s_cstr.to_bytes();
        let mut hunk_start = 0usize;
        for i in 0..=s_bytes.len() {
            let b = s_bytes.get(i).copied().unwrap_or(0);
            if b == 0 || b == NL as u8 {
                if i != hunk_start {
                    let hunk = &s_bytes[hunk_start..i];
                    let written = file_write(fp, hunk);
                    if written < 0 {
                        error = written as i32;
                        break 'outer;
                    }
                }
                if b == 0 {
                    break;
                } else {
                    hunk_start = i + 1;
                    let written = file_write(fp, &[0]);
                    if written < 0 {
                        error = written as i32;
                        break;
                    }
                }
            }
        }
        if !binary || !cur_li.li_next.is_null() {
            let written = file_write(fp, b"\n");
            if written < 0 {
                error = written as i32;
                break 'outer;
            }
        }

        li = cur_li.next_opt();
    }
    if error == 0 {
        error = file_flush(fp);
        if error == 0 {
            return true;
        }
    }

    semsg!(gettext(E_ERROR_WHILE_WRITING_STR), uv_strerror(error),);

    false
}

/// Write a byte slice to file with descriptor `fp`.
fn write_data(fp: &mut FileDescriptor, data: &[u8]) -> bool {
    if !data.is_empty() {
        let written = file_write(fp, data);
        if written < data.len() as isize {
            let error = written as i32;
            semsg!(gettext(E_ERROR_WHILE_WRITING_STR), uv_strerror(error),);
            return false;
        }
    }
    let error = file_flush(fp);
    if error != 0 {
        semsg!(gettext(E_ERROR_WHILE_WRITING_STR), uv_strerror(error),);
        return false;
    }
    true
}

fn write_blob(fp: &mut FileDescriptor, blob: &TvBlob) -> bool {
    let len = tv_blob_len(Some(blob)) as usize;
    write_data(fp, &blob.bv_ga[..len])
}

fn write_string(fp: &mut FileDescriptor, data: &[u8]) -> bool {
    write_data(fp, data)
}

/// "writefile()" function
pub fn f_writefile(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if check_secure() {
        return;
    }

    if argvars[0].v_type == VarType::VarList {
        for li in crate::eval::typval::list_iter_mut(tv_list_first(
            argvars[0].vval.v_list_opt().as_deref(),
        )) {
            if !tv_check_str_or_nr(&li.li_tv) {
                return;
            }
        }
    } else if argvars[0].v_type != VarType::VarBlob
        && !(argvars[0].v_type == VarType::VarString
            && (script_is_lua(g_st().current_sctx.sc_sid)
                || g_st().current_sctx.sc_sid == SID_API_CLIENT))
    {
        semsg!(
            gettext(E_INVARG2),
            gettext(c"writefile() first argument must be a List or a Blob"),
        );
        return;
    }

    let mut binary = false;
    let mut append = false;
    let mut defer = false;
    let mut do_fsync = opt_vars().p_fs;
    let mut mkdir_p = false;
    if argvars[2].v_type != VarType::VarUnknown {
        let Some(flags_cstr) = tv_get_string_chk(&argvars[2]) else {
            return;
        };
        let flags_bytes = flags_cstr.to_bytes();
        for (i, &b) in flags_bytes.iter().enumerate() {
            match b {
                b'b' => binary = true,
                b'a' => append = true,
                b'D' => defer = true,
                b's' => do_fsync = true,
                b'S' => do_fsync = false,
                b'p' => mkdir_p = true,
                _ => {
                    semsg!(gettext(c"E5060: Unknown flag: %s"), &flags_cstr[i..]);
                    return;
                }
            }
        }
    }

    let mut buf = [0u8; NUMBUFLEN];
    let Some(fname) = tv_get_string_buf_chk(&argvars[1], &mut buf) else {
        return;
    };

    if defer && !can_add_defer() {
        return;
    }

    let mut fp = FileDescriptor::new();
    if fname.to_bytes().is_empty() {
        emsg(gettext(c"E482: Can't open file with an empty name"));
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
        let mut error = file_open(&mut fp, Some(fname), open_flags, 0o666);
        if error != 0 {
            semsg!(
                gettext(c"E482: Can't open file %s for writing: %s"),
                fname,
                uv_strerror(error),
            );
        } else {
            if defer {
                let mut tv = TypVal::new();
                if let Some(full) = FullName_save(Some(fname), false) {
                    crate::eval::typval::tv_set_string(&mut tv, Some(full.as_c_str()));
                } else {
                    tv.v_type = VarType::VarString;
                }
                add_defer(c"delete", std::slice::from_ref(&tv));
            }

            let write_ok = if argvars[0].v_type == VarType::VarBlob {
                argvars[0]
                    .vval
                    .v_blob_opt()
                    .is_none_or(|b| write_blob(&mut fp, b))
            } else if argvars[0].v_type == VarType::VarString {
                write_string(&mut fp, argvars[0].vval.v_string_bytes())
            } else {
                write_list(&mut fp, argvars[0].vval.v_list_opt().as_deref(), binary)
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

/// "browse(save, title, initdir, default)" function
pub fn f_browse(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    rettv.v_type = VarType::VarString;
}

/// "browsedir(title, initdir)" function
pub fn f_browsedir(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
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
