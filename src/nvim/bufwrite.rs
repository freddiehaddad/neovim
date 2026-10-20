//! Buffer file writing, encoding conversion, and backup management (`src/nvim/bufwrite.c`).

#![cfg_attr(test, allow(unused_mut, unused_variables))]
#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::command::exarg_T;
#[cfg(not(test))]
use crate::api::options::{CtxSwitch, IOSIZE, bufref_T};
#[cfg(not(test))]
use crate::api::tabpage::pos_T;
#[cfg(not(test))]
use crate::autocmd::{
    EVENT_BUFWRITECMD, EVENT_BUFWRITEPOST, EVENT_BUFWRITEPRE, EVENT_FILEAPPENDCMD,
    EVENT_FILEAPPENDPOST, EVENT_FILEAPPENDPRE, EVENT_FILEWRITECMD, EVENT_FILEWRITEPOST,
    EVENT_FILEWRITEPRE, EVENT_FILTERWRITEPOST, EVENT_FILTERWRITEPRE, apply_autocmds_exarg,
};
#[cfg(all(not(test), unix))]
use crate::buffer::buf_set_file_id;
#[cfg(not(test))]
use crate::buffer::{
    BF_NEW, BF_WRITE_MASK, NOTDONE, bt_nofilename, buf_get_changedtick, bufref_valid, set_bufref,
};
#[cfg(not(test))]
use crate::change::unchanged;
#[cfg(not(test))]
use crate::context::{ctx_restore, ctx_switch};
use crate::cursor::linenr_T;
#[cfg(not(test))]
use crate::dialog::ask_yesno;
#[cfg(not(test))]
use crate::drawscreen::status_redraw_all;
#[cfg(not(test))]
use crate::errors::*;
use crate::eval::deprecated::buf_T;
#[cfg(not(test))]
use crate::eval::fs::FileInfo;
#[cfg(not(test))]
use crate::eval::vars::{FORCE_BIN, eval_charconvert};
#[cfg(not(test))]
use crate::event::r#loop::uv_strerror;
#[cfg(not(test))]
use crate::ex_cmds::check_secure;
#[cfg(not(test))]
use crate::ex_eval::{aborting, should_abort};
#[cfg(all(not(test), windows))]
use crate::fileio::{_close, libiconv, libiconv_close};
#[cfg(not(test))]
use crate::fileio::{
    EOL_DOS, EOL_UNIX, FIO_NOCONVERT, ICONV_EINVAL, WRITEBUFSIZE, add_quoted_fname,
    buf_store_file_info, filemess_bufwrite, fname_msg_id, get_fio_flags, kShmOver, match_file_list,
    modname, msg_add_fileformat, msg_add_lines, need_conversion, set_rw_fname, slow_io_ns,
    time_differs, vim_rename, vim_tempname, write_eintr,
};
#[cfg(not(windows))]
#[cfg(not(test))]
use crate::fileio::{close, getgid, getuid, iconv, iconv_close};
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::mbyte::{enc_canonize, my_iconv_open, utf_ptr2char, utf_ptr2len_len};
#[cfg(not(test))]
use crate::memline::{get_file_in_dir, make_percent_swname, ml_get_buf, ml_preserve, ml_timestamp};
#[cfg(not(test))]
use crate::memory::{verbose_try_malloc, xfree, xmalloc, xmemcpyz, xstrdup, xstrlcat, xstrlcpy};
#[cfg(not(test))]
use crate::message::{emsg, msg, msg_progress, msg_puts_hl, msg_with_id, semsg, set_keep_msg};
#[cfg(not(test))]
use crate::option::{copy_option_part, get_bkc_flags, get_fileformat_force, shortmess};
#[cfg(not(test))]
use crate::option_vars::*;
#[cfg(not(test))]
use crate::os::fs::{
    O_WRONLY, os_close, os_copy, os_copy_xattr, os_errno as iconv_errno, os_file_is_writable,
    os_fileinfo, os_fileinfo_hardlinks, os_fileinfo_id_equal, os_fileinfo_link, os_free_acl,
    os_fsync, os_get_acl, os_getperm, os_isdir, os_mkdir_recurse, os_nodetype, os_open,
    os_path_exists, os_remove, os_set_acl, os_setperm, vim_acl_T,
};
#[cfg(all(not(test), unix))]
use crate::os::fs::{os_chown, os_fchown, os_file_settime, os_fsetperm, uv_gid_t, uv_uid_t};
#[cfg(not(test))]
use crate::os::input::os_breakcheck;
#[cfg(not(test))]
use crate::os::time::os_hrtime;
#[cfg(not(test))]
use crate::path::{after_pathsep, kPathCmpLiteral, path_equal, path_tail};
#[cfg(not(test))]
use crate::sha256::{context_sha256_T, sha256_finish, sha256_start, sha256_update};
#[cfg(not(test))]
use crate::strings::{gettext, strlen as c_strlen, vim_snprintf, vim_snprintf_add, vim_strchr};
#[cfg(not(test))]
use crate::ui::{ui_busy_start, ui_busy_stop, ui_flush};
#[cfg(not(test))]
use crate::undo::{curbufIsChanged, u_unchanged, u_update_save_nr, u_write_undo};
#[cfg(not(test))]
use std::ffi::c_uint;
use std::ffi::{c_char, c_int};
use std::ptr;

#[cfg(not(test))]
use crate::api::autocmd::kCallbackNone;
#[cfg(not(test))]
use crate::api::buffer::NUL;
#[cfg(not(test))]
use crate::api::command::CMOD_LOCKMARKS;
#[cfg(not(test))]
use crate::api::private::helpers::HLF_E;
#[cfg(not(test))]
use crate::api::vim::MAXPATHL;
#[cfg(not(test))]
use crate::arglist::ML_EMPTY;
#[cfg(not(test))]
use crate::charset::EOL_MAC;
pub use crate::eval::buffer::varnumber_T;
#[cfg(not(test))]
use crate::ex_docmd::UNDO_HASH_SIZE;
pub use crate::fileio::iconv_t;
use crate::fileio::{FIO_ENDIAN_L, FIO_LATIN1, FIO_UCS2, FIO_UCS4, FIO_UTF8, FIO_UTF16};
#[cfg(all(not(test), unix))]
use crate::fileio::{S_IFDIR, S_IFMT, S_IFREG};
use crate::math::FAIL;
#[cfg(not(test))]
use crate::math::OK;
#[cfg(not(test))]
use crate::optionstr::{kOptBkcFlagAuto, kOptBkcFlagYes};
#[cfg(all(not(test), windows))]
use crate::os::fs::NODE_OTHER;
pub use crate::path::PathCmpFlags;

#[cfg(not(test))]
const NL: u32 = b'\n' as u32;
#[cfg(not(test))]
const NL_CHAR: c_char = b'\n' as c_char;
#[cfg(not(test))]
const CAR_CHAR: c_char = b'\r' as c_char;

#[cfg(not(test))]
const SMALLBUFSIZE: usize = 256;
#[cfg(not(test))]
const ICONV_MULT: usize = 8;

#[cfg(not(test))]
pub(crate) const NODE_WRITABLE: c_int = 1;

#[cfg(all(not(test), unix))]
const kOptBkcFlagBreaksymlink: c_uint = 0x08;
#[cfg(all(not(test), unix))]
const kOptBkcFlagBreakhardlink: c_uint = 0x10;

#[cfg(not(test))]
const kCpoFnameapp: c_int = b'P' as c_int;
#[cfg(not(test))]
const kCpoFnamew: c_int = b'F' as c_int;
#[cfg(not(test))]
const kCpoFwrite: c_int = b'W' as c_int;
#[cfg(not(test))]
const kCpoKeepro: c_int = b'Z' as c_int;
#[cfg(not(test))]
const kCpoPlus: c_int = b'+' as c_int;

#[cfg(not(test))]
const kShmWri: c_int = b'w' as c_int;
#[cfg(not(test))]
const kShmWrite: c_int = b'W' as c_int;

#[cfg(not(test))]
const UV_FS_COPYFILE_FICLONE: c_int = 0x0002;

#[cfg(all(not(test), windows))]
const UV_ENOTSUP: c_int = -4049;
#[cfg(all(not(test), target_os = "linux"))]
const UV_ENOTSUP: c_int = -95;
#[cfg(not(any(windows, target_os = "linux")))]
const UV_ENOTSUP: c_int = -45;

#[cfg(all(not(test), windows))]
const O_APPEND: c_int = 0x0008;
#[cfg(all(not(test), windows))]
const O_CREAT: c_int = 0x0100;
#[cfg(all(not(test), windows))]
const O_TRUNC: c_int = 0x0200;
#[cfg(all(not(test), windows))]
const O_EXCL: c_int = 0x0400;
#[cfg(all(not(test), windows))]
const O_NOFOLLOW: c_int = 0;

#[cfg(all(not(test), target_os = "linux"))]
const O_CREAT: c_int = 0o100;
#[cfg(all(not(test), target_os = "linux"))]
const O_EXCL: c_int = 0o200;
#[cfg(all(not(test), target_os = "linux"))]
const O_TRUNC: c_int = 0o1000;
#[cfg(all(not(test), target_os = "linux"))]
const O_APPEND: c_int = 0o2000;
#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "arm", target_arch = "aarch64"))
))]
#[cfg(not(test))]
const O_NOFOLLOW: c_int = 0o400000;
#[cfg(all(target_os = "linux", any(target_arch = "arm", target_arch = "aarch64")))]
const O_NOFOLLOW: c_int = 0o100000;

#[cfg(not(any(windows, target_os = "linux")))]
const O_APPEND: c_int = 0x0008;
#[cfg(not(any(windows, target_os = "linux")))]
const O_NOFOLLOW: c_int = 0x0100;
#[cfg(not(any(windows, target_os = "linux")))]
const O_CREAT: c_int = 0x0200;
#[cfg(not(any(windows, target_os = "linux")))]
const O_TRUNC: c_int = 0x0400;
#[cfg(not(any(windows, target_os = "linux")))]
const O_EXCL: c_int = 0x0800;

#[cfg(not(test))]
#[repr(C)]
struct Error_T {
    num: *const c_char,
    msg: *mut c_char,
    arg: c_int,
    alloc: bool,
}

#[cfg(not(test))]
#[repr(C)]
struct bw_info {
    bw_fd: c_int,
    bw_buf: *mut c_char,
    bw_len: c_int,
    bw_flags: c_int,
    bw_first: c_int,
    bw_conv_buf: *mut c_char,
    bw_conv_buflen: usize,
    bw_conv_error: c_int,
    bw_conv_error_lnum: linenr_T,
    bw_start_lnum: linenr_T,
    bw_iconv_fd: iconv_t,
}

#[cfg(all(not(test), unix))]
#[inline]
const fn s_isdir(m: u64) -> bool {
    (m & S_IFMT) == S_IFDIR
}

#[cfg(all(not(test), unix))]
#[inline]
const fn s_isreg(m: u64) -> bool {
    (m & S_IFMT) == S_IFREG
}

#[cfg(not(test))]
#[inline]
unsafe fn close_fd(fd: c_int) -> c_int {
    #[cfg(all(not(test), windows))]
    unsafe {
        _close(fd)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        close(fd)
    }
    #[cfg(test)]
    {
        0
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn call_iconv(
    cd: iconv_t,
    inbuf: *mut *const c_char,
    inbytesleft: *mut usize,
    outbuf: *mut *mut c_char,
    outbytesleft: *mut usize,
) -> usize {
    #[cfg(all(not(test), windows))]
    unsafe {
        libiconv(cd, inbuf, inbytesleft, outbuf, outbytesleft)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        iconv(cd, inbuf, inbytesleft, outbuf, outbytesleft)
    }
    #[cfg(test)]
    {
        0
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn call_iconv_close(cd: iconv_t) -> c_int {
    #[cfg(all(not(test), windows))]
    unsafe {
        libiconv_close(cd)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        iconv_close(cd)
    }
    #[cfg(test)]
    {
        0
    }
}

/// Convert a Unicode character to bytes.
unsafe fn ucs2bytes(mut c: u32, pp: *mut *mut c_char, flags: c_int) -> bool {
    unsafe {
        let mut p = (*pp).cast::<u8>();
        let mut error = false;

        if (flags & FIO_UCS4) != 0 {
            if (flags & FIO_ENDIAN_L) != 0 {
                *p = c as u8;
                *p.add(1) = (c >> 8) as u8;
                *p.add(2) = (c >> 16) as u8;
                *p.add(3) = (c >> 24) as u8;
            } else {
                *p = (c >> 24) as u8;
                *p.add(1) = (c >> 16) as u8;
                *p.add(2) = (c >> 8) as u8;
                *p.add(3) = c as u8;
            }
            p = p.add(4);
        } else if (flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
            if c >= 0x10000 {
                if (flags & FIO_UTF16) != 0 {
                    c -= 0x10000;
                    if c >= 0x100000 {
                        error = true;
                    }
                    let cc = ((c >> 10) & 0x3ff) + 0xd800;
                    if (flags & FIO_ENDIAN_L) != 0 {
                        *p = cc as u8;
                        *p.add(1) = (cc >> 8) as u8;
                    } else {
                        *p = (cc >> 8) as u8;
                        *p.add(1) = cc as u8;
                    }
                    p = p.add(2);
                    c = (c & 0x3ff) + 0xdc00;
                } else {
                    error = true;
                }
            }
            if (flags & FIO_ENDIAN_L) != 0 {
                *p = c as u8;
                *p.add(1) = (c >> 8) as u8;
            } else {
                *p = (c >> 8) as u8;
                *p.add(1) = c as u8;
            }
            p = p.add(2);
        } else if c >= 0x100 {
            error = true;
            *p = 0xbf;
            p = p.add(1);
        } else {
            *p = c as u8;
            p = p.add(1);
        }

        *pp = p.cast::<c_char>();
        error
    }
}

#[cfg(not(test))]
unsafe fn buf_write_convert_with_iconv(
    ip: *mut bw_info,
    bufp: *mut *mut c_char,
    lenp: *mut c_int,
) -> c_int {
    unsafe {
        let len = *lenp;
        let mut from: *const c_char = *bufp;
        let mut fromlen = len as usize;
        let mut tolen = (*ip).bw_conv_buflen;
        let mut to = (*ip).bw_conv_buf;

        if (*ip).bw_first != 0 {
            let save_len = tolen;
            let _ = call_iconv(
                (*ip).bw_iconv_fd,
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut to,
                &raw mut tolen,
            );
            if to.is_null() {
                to = (*ip).bw_conv_buf;
                tolen = save_len;
            }
            (*ip).bw_first = 0;
        }

        if call_iconv(
            (*ip).bw_iconv_fd,
            &raw mut from,
            &raw mut fromlen,
            &raw mut to,
            &raw mut tolen,
        ) == usize::MAX
            && iconv_errno() != ICONV_EINVAL
        {
            (*ip).bw_conv_error = 1;
            return -1;
        }

        *bufp = (*ip).bw_conv_buf;
        *lenp = ((to as usize) - ((*ip).bw_conv_buf as usize)) as c_int;

        len - (fromlen as c_int)
    }
}

#[cfg(not(test))]
unsafe fn buf_write_convert(ip: *mut bw_info, bufp: *mut *mut c_char, lenp: *mut c_int) -> c_int {
    unsafe {
        let flags = (*ip).bw_flags;
        let mut wlen = *lenp;

        if (flags & (FIO_UCS4 | FIO_UTF16 | FIO_UCS2 | FIO_LATIN1)) != 0 {
            let mut p = if (flags & FIO_LATIN1) != 0 {
                *bufp
            } else {
                (*ip).bw_conv_buf
            };
            wlen = 0;
            while wlen < *lenp {
                #[cfg(not(test))]
                let n = utf_ptr2len_len((*bufp).add(wlen as usize), *lenp - wlen);
                #[cfg(test)]
                let n = 1;
                if n > *lenp - wlen {
                    break;
                }
                let c: u32 = if n > 1 {
                    #[cfg(not(test))]
                    {
                        utf_ptr2char((*bufp).add(wlen as usize)) as u32
                    }
                    #[cfg(test)]
                    {
                        0
                    }
                } else {
                    u32::from(*(*bufp).add(wlen as usize) as u8)
                };

                if (flags & FIO_LATIN1) == 0 {
                    let mut need: usize = if (flags & FIO_UCS4) != 0 { 4 } else { 2 };
                    if (flags & FIO_UTF16) != 0 && c >= 0x10000 {
                        need = 4;
                    }
                    if ((p as usize) - ((*ip).bw_conv_buf as usize)) + need > (*ip).bw_conv_buflen {
                        return FAIL;
                    }
                }

                if ucs2bytes(c, &raw mut p, flags) && (*ip).bw_conv_error == 0 {
                    (*ip).bw_conv_error = 1;
                    (*ip).bw_conv_error_lnum = (*ip).bw_start_lnum;
                }
                if c == NL {
                    (*ip).bw_start_lnum += 1;
                }
                wlen += n;
            }
            if (flags & FIO_LATIN1) != 0 {
                *lenp = ((p as usize) - (*bufp as usize)) as c_int;
            } else {
                *bufp = (*ip).bw_conv_buf;
                *lenp = ((p as usize) - ((*ip).bw_conv_buf as usize)) as c_int;
            }
        }

        if (*ip).bw_iconv_fd != -1 {
            return buf_write_convert_with_iconv(ip, bufp, lenp);
        }

        wlen
    }
}

#[cfg(not(test))]
unsafe fn buf_write_bytes(ip: *mut bw_info) -> c_int {
    unsafe {
        let mut buf = (*ip).bw_buf;
        let mut len = (*ip).bw_len;
        let flags = (*ip).bw_flags;

        if (flags & FIO_NOCONVERT) == 0 {
            let converted = buf_write_convert(ip, &raw mut buf, &raw mut len);
            if converted < 0 {
                return FAIL;
            }
            let remaining = (*ip).bw_len - converted;
            (*ip).bw_len = remaining;

            if (*ip).bw_fd >= 0 {
                #[cfg(not(test))]
                let wlen = write_eintr((*ip).bw_fd, buf.cast(), len as usize) as c_int;
                #[cfg(test)]
                let wlen = len;
                if wlen < len {
                    return FAIL;
                }
            }

            if remaining > 0 {
                ptr::copy(
                    (*ip).bw_buf.add(converted as usize),
                    (*ip).bw_buf,
                    remaining as usize,
                );
            }
        } else {
            (*ip).bw_len = 0;
            if (*ip).bw_fd >= 0 {
                #[cfg(not(test))]
                let wlen = write_eintr((*ip).bw_fd, buf.cast(), len as usize) as c_int;
                #[cfg(test)]
                let wlen = len;
                if wlen < len {
                    return FAIL;
                }
            }
        }

        OK
    }
}

#[cfg(not(test))]
unsafe fn check_mtime(buf: *mut buf_T, file_info: *const FileInfo) -> c_int {
    #[cfg(not(test))]
    unsafe {
        if (*buf).b_mtime_read != 0
            && time_differs(file_info, (*buf).b_mtime_read, (*buf).b_mtime_read_ns)
        {
            msg_scroll = 1;
            msg_silent = 0;
            msg(
                gettext(c"WARNING: The file has been changed since reading it!!!".as_ptr()),
                HLF_E,
            );
            if ask_yesno(gettext(c"Do you really want to write to it".as_ptr())) == b'n' as c_int {
                return FAIL;
            }
            msg_scroll = 0;
        }
    }
    OK
}

unsafe fn make_bom(buf_in: *mut c_char, name: *mut c_char) -> c_int {
    unsafe {
        let buf = buf_in.cast::<u8>();
        #[cfg(not(test))]
        let flags = get_fio_flags(name);
        #[cfg(test)]
        let flags = FIO_UTF8;

        if flags == FIO_LATIN1 || flags == 0 {
            return 0;
        }
        if flags == FIO_UTF8 {
            *buf = 0xef;
            *buf.add(1) = 0xbb;
            *buf.add(2) = 0xbf;
            return 3;
        }
        let mut p = buf_in;
        let _ = ucs2bytes(0xfeff, &raw mut p, flags);
        ((p as usize) - (buf_in as usize)) as c_int
    }
}

#[cfg(not(test))]
unsafe fn buf_write_do_autocmds(
    mut buf: *mut buf_T,
    fnamep: *mut *mut c_char,
    sfnamep: *mut *mut c_char,
    ffnamep: *mut *mut c_char,
    start: linenr_T,
    endp: *mut linenr_T,
    eap: *mut exarg_T,
    append: bool,
    filtering: bool,
    reset_changed: bool,
    overwriting: bool,
    whole: bool,
    orig_start: pos_T,
    orig_end: pos_T,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let old_line_count = (*buf).b_ml.ml_line_count;
        let msg_save = msg_scroll;

        let mut aco: CtxSwitch = std::mem::zeroed();
        let mut did_cmd = false;
        let mut nofile_err = false;
        let empty_memline = (*buf).b_ml.ml_mfp.is_null();
        let mut bufref = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };

        let sfname = *sfnamep;
        let buf_ffname = *ffnamep == (*buf).b_ffname;
        let buf_sfname = sfname == (*buf).b_sfname;
        let buf_fname_f = *fnamep == (*buf).b_ffname;
        let buf_fname_s = *fnamep == (*buf).b_sfname;

        let _ = ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        set_bufref(&raw mut bufref, buf);

        if append {
            did_cmd = apply_autocmds_exarg(EVENT_FILEAPPENDCMD, sfname, sfname, false, curbuf, eap);
            if !did_cmd {
                if overwriting && bt_nofilename(curbuf) {
                    nofile_err = true;
                } else {
                    let _ = apply_autocmds_exarg(
                        EVENT_FILEAPPENDPRE,
                        sfname,
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );
                }
            }
        } else if filtering {
            let _ = apply_autocmds_exarg(
                EVENT_FILTERWRITEPRE,
                ptr::null_mut(),
                sfname,
                false,
                curbuf,
                eap,
            );
        } else if reset_changed && whole {
            let was_changed = curbufIsChanged();
            did_cmd = apply_autocmds_exarg(EVENT_BUFWRITECMD, sfname, sfname, false, curbuf, eap);
            if did_cmd {
                if was_changed && !curbufIsChanged() {
                    u_unchanged(curbuf);
                    u_update_save_nr(curbuf);
                }
            } else if overwriting && bt_nofilename(curbuf) {
                nofile_err = true;
            } else {
                let _ = apply_autocmds_exarg(EVENT_BUFWRITEPRE, sfname, sfname, false, curbuf, eap);
            }
        } else {
            did_cmd = apply_autocmds_exarg(EVENT_FILEWRITECMD, sfname, sfname, false, curbuf, eap);
            if !did_cmd {
                if overwriting && bt_nofilename(curbuf) {
                    nofile_err = true;
                } else {
                    let _ = apply_autocmds_exarg(
                        EVENT_FILEWRITEPRE,
                        sfname,
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );
                }
            }
        }

        ctx_restore(&raw mut aco);

        if !bufref_valid(&raw mut bufref) {
            buf = ptr::null_mut();
        }
        if buf.is_null()
            || ((*buf).b_ml.ml_mfp.is_null() && !empty_memline)
            || did_cmd
            || nofile_err
            || aborting()
        {
            if !buf.is_null() && (cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
                (*buf).b_op_start = orig_start;
                (*buf).b_op_end = orig_end;
            }

            no_wait_return -= 1;
            msg_scroll = msg_save;
            if nofile_err {
                let _ = semsg!(
                    gettext(c"E676: No matching autocommands for buftype=%s buffer".as_ptr()),
                    (*curbuf).b_p_bt,
                );
            }

            if nofile_err || aborting() {
                return FAIL;
            }
            if did_cmd {
                if buf.is_null() {
                    return OK;
                }
                if overwriting {
                    ml_timestamp(buf);
                    if append {
                        (*buf).b_flags &= !BF_NEW;
                    } else {
                        (*buf).b_flags &= !BF_WRITE_MASK;
                    }
                }
                if reset_changed
                    && (*buf).b_changed != 0
                    && !append
                    && (overwriting || !vim_strchr(p_cpo, kCpoPlus).is_null())
                {
                    return FAIL;
                }
                return OK;
            }
            if !aborting() {
                emsg(gettext(
                    c"E203: Autocommands deleted or unloaded buffer to be written".as_ptr(),
                ));
            }
            return FAIL;
        }

        if (*buf).b_ml.ml_line_count != old_line_count {
            if whole {
                *endp = (*buf).b_ml.ml_line_count;
            } else if (*buf).b_ml.ml_line_count > old_line_count {
                *endp += (*buf).b_ml.ml_line_count - old_line_count;
            } else {
                *endp -= old_line_count - (*buf).b_ml.ml_line_count;
                if *endp < start {
                    no_wait_return -= 1;
                    msg_scroll = msg_save;
                    emsg(gettext(
                        c"E204: Autocommand changed number of lines in unexpected way".as_ptr(),
                    ));
                    return FAIL;
                }
            }
        }

        if buf_ffname {
            *ffnamep = (*buf).b_ffname;
        }
        if buf_sfname {
            *sfnamep = (*buf).b_fname;
        }
        if buf_fname_f {
            *fnamep = (*buf).b_ffname;
        }
        if buf_fname_s {
            *fnamep = (*buf).b_fname;
        }
    }
    NOTDONE
}

#[cfg(not(test))]
unsafe fn buf_write_do_post_autocmds(
    buf: *mut buf_T,
    fname: *mut c_char,
    eap: *mut exarg_T,
    append: bool,
    filtering: bool,
    reset_changed: bool,
    whole: bool,
) {
    #[cfg(not(test))]
    unsafe {
        let mut aco: CtxSwitch = std::mem::zeroed();
        (*curbuf).b_no_eol_lnum = 0;

        let _ = ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);

        if append {
            let _ = apply_autocmds_exarg(EVENT_FILEAPPENDPOST, fname, fname, false, curbuf, eap);
        } else if filtering {
            let _ = apply_autocmds_exarg(
                EVENT_FILTERWRITEPOST,
                ptr::null_mut(),
                fname,
                false,
                curbuf,
                eap,
            );
        } else if reset_changed && whole {
            let _ = apply_autocmds_exarg(EVENT_BUFWRITEPOST, fname, fname, false, curbuf, eap);
        } else {
            let _ = apply_autocmds_exarg(EVENT_FILEWRITEPOST, fname, fname, false, curbuf, eap);
        }

        ctx_restore(&raw mut aco);
    }
}

#[cfg(not(test))]
#[inline]
const fn set_err_num(num: *const c_char, msg: *const c_char) -> Error_T {
    Error_T {
        num,
        msg: msg.cast_mut(),
        arg: 0,
        alloc: false,
    }
}

#[cfg(not(test))]
#[inline]
const fn set_err(msg: *const c_char) -> Error_T {
    Error_T {
        num: ptr::null(),
        msg: msg.cast_mut(),
        arg: 0,
        alloc: false,
    }
}

#[cfg(not(test))]
#[inline]
const fn set_err_arg(msg: *const c_char, arg: c_int) -> Error_T {
    Error_T {
        num: ptr::null(),
        msg: msg.cast_mut(),
        arg,
        alloc: false,
    }
}

#[cfg(not(test))]
unsafe fn emit_err(e: *mut Error_T, msg_id: *const c_char) {
    #[cfg(not(test))]
    unsafe {
        let mut errmsg = [0 as c_char; IOSIZE];
        if !(*e).num.is_null() && (*e).arg != 0 {
            let _ = vim_snprintf!(
                errmsg.as_mut_ptr(),
                IOSIZE,
                c"%s: %s%s: %s".as_ptr(),
                (*e).num,
                (&raw const IObuff).cast::<c_char>(),
                (*e).msg,
                uv_strerror((*e).arg),
            );
        } else if !(*e).num.is_null() {
            let _ = vim_snprintf!(
                errmsg.as_mut_ptr(),
                IOSIZE,
                c"%s: %s%s".as_ptr(),
                (*e).num,
                (&raw const IObuff).cast::<c_char>(),
                (*e).msg,
            );
        } else if (*e).arg != 0 {
            let _ = vim_snprintf!(errmsg.as_mut_ptr(), IOSIZE, (*e).msg, uv_strerror((*e).arg));
        } else {
            xstrlcpy(errmsg.as_mut_ptr(), (*e).msg, IOSIZE);
        }

        if !msg_id.is_null() {
            let _ = msg_progress(
                errmsg.as_mut_ptr(),
                msg_id.cast_mut(),
                c"failed".as_ptr().cast_mut(),
                -1,
                HLF_E,
                false,
                false,
                true,
            );
        } else {
            emsg(errmsg.as_ptr());
        }
        if (*e).alloc {
            xfree((*e).msg.cast());
        }
    }
}

#[cfg(all(not(test), not(windows)))]
unsafe fn get_fileinfo_os(
    fname: *mut c_char,
    file_info_old: *mut FileInfo,
    _overwriting: bool,
    perm: *mut c_int,
    device: *mut bool,
    newfile: *mut bool,
    err: *mut Error_T,
) -> c_int {
    unsafe {
        *perm = -1;
        if !os_fileinfo(fname, file_info_old) {
            *newfile = true;
        } else {
            *perm = (*file_info_old).stat.st_mode as c_int;
            if !s_isreg((*file_info_old).stat.st_mode) {
                if s_isdir((*file_info_old).stat.st_mode) {
                    #[cfg(not(test))]
                    {
                        *err = set_err_num(c"E502".as_ptr(), gettext(c"is a directory".as_ptr()));
                    }
                    return FAIL;
                }
                if os_nodetype(fname) != NODE_WRITABLE {
                    #[cfg(not(test))]
                    {
                        *err = set_err_num(
                            c"E503".as_ptr(),
                            gettext(c"is not a file or writable device".as_ptr()),
                        );
                    }
                    return FAIL;
                }
                *device = true;
                *newfile = true;
                *perm = -1;
            }
        }
        OK
    }
}

#[cfg(all(not(test), windows))]
unsafe fn get_fileinfo_os(
    fname: *mut c_char,
    file_info_old: *mut FileInfo,
    overwriting: bool,
    perm: *mut c_int,
    device: *mut bool,
    newfile: *mut bool,
    err: *mut Error_T,
) -> c_int {
    unsafe {
        let nodetype = if fname.is_null() {
            NODE_OTHER
        } else {
            os_nodetype(fname)
        };
        if nodetype == NODE_OTHER {
            #[cfg(not(test))]
            {
                *err = set_err_num(
                    c"E503".as_ptr(),
                    gettext(c"is not a file or writable device".as_ptr()),
                );
            }
            return FAIL;
        }
        if nodetype == NODE_WRITABLE {
            *device = true;
            *newfile = true;
            *perm = -1;
        } else {
            *perm = os_getperm(fname);
            if *perm < 0 {
                *newfile = true;
            } else if os_isdir(fname) {
                #[cfg(not(test))]
                {
                    *err = set_err_num(c"E502".as_ptr(), gettext(c"is a directory".as_ptr()));
                }
                return FAIL;
            }
            if overwriting {
                let _ = os_fileinfo(fname, file_info_old);
            }
        }
        OK
    }
}

#[cfg(not(test))]
unsafe fn get_fileinfo(
    buf: *mut buf_T,
    fname: *mut c_char,
    overwriting: bool,
    forceit: bool,
    file_info_old: *mut FileInfo,
    perm: *mut c_int,
    device: *mut bool,
    newfile: *mut bool,
    readonly: *mut bool,
    err: *mut Error_T,
) -> c_int {
    unsafe {
        if get_fileinfo_os(
            fname,
            file_info_old,
            overwriting,
            perm,
            device,
            newfile,
            err,
        ) == FAIL
        {
            return FAIL;
        }

        *readonly = false;

        if !*device && !*newfile {
            *readonly = os_file_is_writable(fname) == 0;

            if !forceit && *readonly {
                #[cfg(not(test))]
                if !vim_strchr(p_cpo, kCpoFwrite).is_null() {
                    *err = set_err_num(
                        c"E504".as_ptr(),
                        gettext(c"is read-only (cannot override: \"W\" in 'cpoptions')".as_ptr()),
                    );
                } else {
                    *err = set_err_num(
                        c"E505".as_ptr(),
                        gettext(c"is read-only (add ! to override)".as_ptr()),
                    );
                }
                return FAIL;
            }

            if overwriting && !forceit && check_mtime(buf, file_info_old) == FAIL {
                return FAIL;
            }
        }
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_get_backup_name(
    fname: *mut c_char,
    dirp: *mut *mut c_char,
    no_prepend_dot: bool,
    backup_ext: *mut c_char,
) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        let mut backup: *mut c_char = ptr::null_mut();
        let iobuf = (&raw mut IObuff).cast::<c_char>();
        let dir_len = copy_option_part(dirp, iobuf, IOSIZE, c",".as_ptr());
        let p = iobuf.add(dir_len);
        if **dirp == NUL && !os_isdir(iobuf) {
            let mut failed_dir: *mut c_char = ptr::null_mut();
            let ret = os_mkdir_recurse(iobuf, 0o755, &raw mut failed_dir, ptr::null_mut());
            if ret != 0 {
                let _ = semsg!(
                    gettext(
                        c"E303: Unable to create directory \"%s\" for backup file: %s".as_ptr(),
                    ),
                    failed_dir,
                    uv_strerror(ret),
                );
                xfree(failed_dir.cast());
            }
        }
        if dir_len > 1 && after_pathsep(iobuf, p) != 0 && *p.sub(1) == *p.sub(2) {
            let swname = make_percent_swname(iobuf, p, fname);
            if !swname.is_null() {
                backup = modname(swname, backup_ext, no_prepend_dot);
                xfree(swname.cast());
            }
        }
        if backup.is_null() {
            let rootname = get_file_in_dir(fname, iobuf);
            if !rootname.is_null() {
                backup = modname(rootname, backup_ext, no_prepend_dot);
                xfree(rootname.cast());
            }
        }
        backup
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

#[cfg(not(test))]
unsafe fn buf_write_make_backup(
    fname: *mut c_char,
    append: bool,
    file_info_old: *mut FileInfo,
    acl: vim_acl_T,
    perm: c_int,
    bkc: c_uint,
    file_readonly: bool,
    forceit: bool,
    backup_copyp: *mut bool,
    backupp: *mut *mut c_char,
    err: *mut Error_T,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut file_info = FileInfo::default();
        let no_prepend_dot = false;

        if (bkc & kOptBkcFlagYes) != 0 || append {
            *backup_copyp = true;
        } else if (bkc & kOptBkcFlagAuto) != 0 {
            if os_fileinfo_hardlinks(file_info_old) > 1
                || !os_fileinfo_link(fname, &raw mut file_info)
                || !os_fileinfo_id_equal(&raw const file_info, file_info_old)
            {
                *backup_copyp = true;
            } else {
                let dirlen = (path_tail(fname) as usize) - (fname as usize);
                assert!(dirlen < MAXPATHL);
                let mut tmp_fname = [0 as c_char; MAXPATHL];
                let _ = xmemcpyz(tmp_fname.as_mut_ptr().cast(), fname.cast(), dirlen);
                let mut i: c_int = 4913;
                loop {
                    let suffix = format!(".{i}.swp\0");
                    ptr::copy_nonoverlapping(
                        suffix.as_ptr().cast::<c_char>(),
                        tmp_fname.as_mut_ptr().add(dirlen),
                        suffix.len(),
                    );
                    if !os_fileinfo_link(tmp_fname.as_ptr(), &raw mut file_info) {
                        break;
                    }
                    i += 123;
                }
                let fd = os_open(
                    tmp_fname.as_ptr(),
                    O_CREAT | O_WRONLY | O_EXCL | O_NOFOLLOW,
                    perm,
                );
                if fd < 0 {
                    *backup_copyp = true;
                } else {
                    #[cfg(not(windows))]
                    {
                        let _ = os_fchown(
                            fd,
                            (*file_info_old).stat.st_uid as uv_uid_t,
                            (*file_info_old).stat.st_gid as uv_gid_t,
                        );
                        let _ = os_fsetperm(fd, perm);
                        if !os_fileinfo(tmp_fname.as_ptr(), &raw mut file_info)
                            || file_info.stat.st_uid != (*file_info_old).stat.st_uid
                            || file_info.stat.st_gid != (*file_info_old).stat.st_gid
                            || (file_info.stat.st_mode as c_int) != perm
                        {
                            *backup_copyp = true;
                        }
                    }
                    let _ = close_fd(fd);
                    let _ = os_remove(tmp_fname.as_ptr());
                }
            }
        }

        #[cfg(not(windows))]
        if (bkc & (kOptBkcFlagBreaksymlink | kOptBkcFlagBreakhardlink)) != 0 {
            let file_info_link_ok = os_fileinfo_link(fname, &raw mut file_info);
            if (bkc & kOptBkcFlagBreaksymlink) != 0
                && file_info_link_ok
                && !os_fileinfo_id_equal(&raw const file_info, file_info_old)
            {
                *backup_copyp = false;
            }
            if (bkc & kOptBkcFlagBreakhardlink) != 0
                && os_fileinfo_hardlinks(file_info_old) > 1
                && (!file_info_link_ok || os_fileinfo_id_equal(&raw const file_info, file_info_old))
            {
                *backup_copyp = false;
            }
        }

        let backup_ext = if *p_bex == NUL {
            c".bak".as_ptr().cast_mut()
        } else {
            p_bex
        };

        if *backup_copyp {
            let mut some_error = false;
            let mut dirp = p_bdir;
            'nobackup: {
                while *dirp != 0 {
                    *backupp =
                        buf_get_backup_name(fname, &raw mut dirp, no_prepend_dot, backup_ext);
                    if (*backupp).is_null() {
                        some_error = true;
                        break 'nobackup;
                    }

                    let mut file_info_new = FileInfo::default();
                    if os_fileinfo(*backupp, &raw mut file_info_new) {
                        if os_fileinfo_id_equal(&raw const file_info_new, file_info_old) {
                            xfree((*backupp).cast());
                            *backupp = ptr::null_mut();
                        } else if p_bk == 0 {
                            let blen = c_strlen(*backupp);
                            let extlen = c_strlen(backup_ext);
                            let mut wp = if blen > extlen + 1 {
                                (*backupp).add(blen - 1 - extlen)
                            } else {
                                *backupp
                            };
                            if (wp as usize) < (*backupp as usize) {
                                wp = *backupp;
                            }
                            *wp = b'z' as c_char;
                            while (*wp as u8) > b'a'
                                && os_fileinfo(*backupp, &raw mut file_info_new)
                            {
                                *wp -= 1;
                            }
                            if (*wp as u8) == b'a' {
                                xfree((*backupp).cast());
                                *backupp = ptr::null_mut();
                            }
                        }
                    }

                    if !(*backupp).is_null() {
                        let _ = os_remove(*backupp);
                        if os_copy(fname, *backupp, UV_FS_COPYFILE_FICLONE) != 0 {
                            *err = set_err(gettext(
                                c"E509: Cannot create backup file (add ! to override)".as_ptr(),
                            ));
                            xfree((*backupp).cast());
                            *backupp = ptr::null_mut();
                            continue;
                        }

                        let _ = os_setperm(*backupp, perm & 0o777);

                        #[cfg(not(windows))]
                        {
                            if file_info_new.stat.st_gid != (*file_info_old).stat.st_gid
                                && os_chown(
                                    *backupp,
                                    u32::MAX as uv_uid_t,
                                    (*file_info_old).stat.st_gid as uv_gid_t,
                                ) != 0
                            {
                                let _ = os_setperm(*backupp, (perm & 0o707) | ((perm & 0o7) << 3));
                            }
                            let _ = os_file_settime(
                                *backupp,
                                (*file_info_old).stat.st_atim.tv_sec as f64,
                                (*file_info_old).stat.st_mtim.tv_sec as f64,
                            );
                        }

                        os_set_acl(*backupp, acl);
                        os_copy_xattr(fname, *backupp);
                        *err = set_err(ptr::null());
                        break;
                    }
                }
            }

            if (*backupp).is_null() && (*err).msg.is_null() {
                *err = set_err(gettext(
                    c"E509: Cannot create backup file (add ! to override)".as_ptr(),
                ));
            }
            if (some_error || !(*err).msg.is_null()) && !forceit {
                return FAIL;
            }
            *err = set_err(ptr::null());
        } else {
            if file_readonly && !vim_strchr(p_cpo, kCpoFwrite).is_null() {
                *err = set_err_num(
                    c"E504".as_ptr(),
                    gettext(c"is read-only (cannot override: \"W\" in 'cpoptions')".as_ptr()),
                );
                return FAIL;
            }

            let mut dirp = p_bdir;
            while *dirp != 0 {
                *backupp = buf_get_backup_name(fname, &raw mut dirp, no_prepend_dot, backup_ext);
                if !(*backupp).is_null() && p_bk == 0 && os_path_exists(*backupp) {
                    let blen = c_strlen(*backupp);
                    let extlen = c_strlen(backup_ext);
                    let mut p = if blen > extlen + 1 {
                        (*backupp).add(blen - 1 - extlen)
                    } else {
                        *backupp
                    };
                    if (p as usize) < (*backupp as usize) {
                        p = *backupp;
                    }
                    *p = b'z' as c_char;
                    while (*p as u8) > b'a' && os_path_exists(*backupp) {
                        *p -= 1;
                    }
                    if (*p as u8) == b'a' {
                        xfree((*backupp).cast());
                        *backupp = ptr::null_mut();
                    }
                }
                if !(*backupp).is_null() {
                    if vim_rename(fname, *backupp) == 0 {
                        break;
                    }
                    xfree((*backupp).cast());
                    *backupp = ptr::null_mut();
                }
            }
            if (*backupp).is_null() && !forceit {
                *err = set_err(gettext(
                    c"E510: Can't make backup file (add ! to override)".as_ptr(),
                ));
                return FAIL;
            }
        }
    }
    OK
}

/// Write to file `fname` lines `start` through `end`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_write(
    mut buf: *mut buf_T,
    mut fname: *mut c_char,
    mut sfname: *mut c_char,
    mut start: linenr_T,
    mut end: linenr_T,
    eap: *mut exarg_T,
    append: bool,
    forceit: bool,
    reset_changed: bool,
    filtering: bool,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut retval = OK;
        let msg_save = msg_scroll;
        let mut prev_got_int = got_int;
        let whole = start == 1 && end == (*buf).b_ml.ml_line_count;
        let mut write_undo_file = false;
        let mut sha_ctx: context_sha256_T = std::mem::zeroed();
        let bkc = get_bkc_flags(buf);

        if fname.is_null() || *fname == NUL {
            return FAIL;
        }
        if (*buf).b_ml.ml_mfp.is_null() {
            emsg(gettext(e_empty_buffer.as_ptr()));
            return FAIL;
        }
        if check_secure() {
            return FAIL;
        }
        if c_strlen(fname) >= MAXPATHL {
            emsg(gettext(e_longname.as_ptr()));
            return FAIL;
        }

        let mut write_info = bw_info {
            bw_fd: 0,
            bw_buf: ptr::null_mut(),
            bw_len: 0,
            bw_flags: 0,
            bw_first: 0,
            bw_conv_buf: ptr::null_mut(),
            bw_conv_buflen: 0,
            bw_conv_error: 0,
            bw_conv_error_lnum: 0,
            bw_start_lnum: 0,
            bw_iconv_fd: -1,
        };

        if (*buf).b_ffname.is_null()
            && reset_changed
            && whole
            && buf == curbuf
            && !bt_nofilename(buf)
            && !filtering
            && (!append || !vim_strchr(p_cpo, kCpoFnameapp).is_null())
            && !vim_strchr(p_cpo, kCpoFnamew).is_null()
        {
            if set_rw_fname(fname, sfname) == FAIL {
                return FAIL;
            }
            buf = curbuf;
        }

        if sfname.is_null() {
            sfname = fname;
        }

        let mut ffname = fname;
        #[cfg(not(windows))]
        {
            fname = sfname;
        }

        let overwriting =
            !(*buf).b_ffname.is_null() && path_equal(ffname, (*buf).b_ffname, kPathCmpLiteral);

        no_wait_return += 1;

        let orig_start = (*buf).b_op_start;
        let orig_end = (*buf).b_op_end;

        (*buf).b_op_start.lnum = start;
        (*buf).b_op_start.col = 0;
        (*buf).b_op_end.lnum = end;
        (*buf).b_op_end.col = 0;

        let res = buf_write_do_autocmds(
            buf,
            &raw mut fname,
            &raw mut sfname,
            &raw mut ffname,
            start,
            &raw mut end,
            eap,
            append,
            filtering,
            reset_changed,
            overwriting,
            whole,
            orig_start,
            orig_end,
        );
        if res != NOTDONE {
            return res;
        }

        let fname_copy = xstrdup(fname);
        let sfname_copy = xstrdup(sfname);
        let ffname_copy = xstrdup(ffname);
        fname = fname_copy;
        sfname = sfname_copy;
        ffname = ffname_copy;

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            (*buf).b_op_start = orig_start;
            (*buf).b_op_end = orig_end;
        }

        msg_scroll = c_int::from(!(shortmess(kShmOver) && !exiting));

        let mut msg_id = [0 as c_char; MAXPATHL + 32];
        #[cfg(windows)]
        let msgname = sfname;
        #[cfg(not(windows))]
        let msgname = fname;

        fname_msg_id(
            msg_id.as_mut_ptr(),
            msg_id.len(),
            c"nvim.bufwrite".as_ptr(),
            buf,
            msgname,
        );

        let show_msg = !filtering && msg_silent == 0;
        if show_msg {
            ui_busy_start();
        }
        if !filtering {
            filemess_bufwrite(buf, msgname, msg_id.as_ptr());
        }

        let mut progress: c_int = -1;
        let mut progress_nchars: i64 = 0;
        let write_start = os_hrtime();
        let write_tick = buf_get_changedtick(buf);
        msg_scroll = 0;

        let mut smallbuf = [0 as c_char; SMALLBUFSIZE];
        let mut buffer = verbose_try_malloc(WRITEBUFSIZE).cast::<c_char>();
        let bufsize: c_int = if buffer.is_null() {
            buffer = smallbuf.as_mut_ptr();
            SMALLBUFSIZE as c_int
        } else {
            WRITEBUFSIZE as c_int
        };

        let mut err = set_err(ptr::null());
        let mut perm: c_int = -1;
        let mut newfile = false;
        let mut device = false;
        let mut file_readonly = false;
        let mut backup: *mut c_char = ptr::null_mut();
        let mut fenc_tofree: *mut c_char = ptr::null_mut();
        let mut file_info_old = FileInfo::default();
        let mut acl: vim_acl_T = ptr::null_mut();

        'nofail: {
            'fail: {
                if get_fileinfo(
                    buf,
                    fname,
                    overwriting,
                    forceit,
                    &raw mut file_info_old,
                    &raw mut perm,
                    &raw mut device,
                    &raw mut newfile,
                    &raw mut file_readonly,
                    &raw mut err,
                ) == FAIL
                {
                    break 'fail;
                }

                if !newfile {
                    acl = os_get_acl(fname);
                }

                let mut dobackup = p_wb != 0 || p_bk != 0 || *p_pm != NUL;
                if dobackup && *p_bsk != NUL && match_file_list(p_bsk, sfname, ffname) {
                    dobackup = false;
                }

                let mut backup_copy = false;
                prev_got_int = got_int;
                got_int = false;
                (*buf).b_saving = true;

                if !(append && *p_pm == NUL)
                    && !filtering
                    && perm >= 0
                    && dobackup
                    && buf_write_make_backup(
                        fname,
                        append,
                        &raw mut file_info_old,
                        acl,
                        perm,
                        bkc,
                        file_readonly,
                        forceit,
                        &raw mut backup_copy,
                        &raw mut backup,
                        &raw mut err,
                    ) == FAIL
                {
                    retval = FAIL;
                    break 'fail;
                }

                #[cfg(not(windows))]
                let mut made_writable = false;
                #[cfg(not(windows))]
                if forceit
                    && perm >= 0
                    && (perm & 0o200) == 0
                    && file_info_old.stat.st_uid == u64::from(getuid())
                    && vim_strchr(p_cpo, kCpoFwrite).is_null()
                {
                    perm |= 0o200;
                    let _ = os_setperm(fname, perm);
                    made_writable = true;
                }

                if forceit && overwriting && vim_strchr(p_cpo, kCpoKeepro).is_null() {
                    (*buf).b_p_ro = 0;
                    need_maketitle = true;
                    status_redraw_all();
                }

                end = end.min((*buf).b_ml.ml_line_count);
                if ((*buf).b_ml.ml_flags & ML_EMPTY) != 0 {
                    start = end + 1;
                }

                let mut wfname: *mut c_char = fname;

                'restore_backup: {
                    if reset_changed && !newfile && overwriting && !(exiting && !backup.is_null()) {
                        let fsync_opt = if (*buf).b_p_fs >= 0 {
                            (*buf).b_p_fs != 0
                        } else {
                            p_fs != 0
                        };
                        ml_preserve(buf, false, fsync_opt);
                        if got_int {
                            err = set_err(gettext(e_interr.as_ptr()));
                            break 'restore_backup;
                        }
                    }

                    let fenc: *mut c_char = if !eap.is_null() && (*eap).force_enc != 0 {
                        let e = enc_canonize((*eap).cmd.add((*eap).force_enc as usize));
                        fenc_tofree = e;
                        e
                    } else {
                        (*buf).b_p_fenc
                    };

                    let converted = need_conversion(fenc);
                    let mut wb_flags: c_int = 0;

                    if converted {
                        wb_flags = get_fio_flags(fenc);
                        if (wb_flags & (FIO_UCS2 | FIO_UCS4 | FIO_UTF16 | FIO_UTF8)) != 0 {
                            write_info.bw_conv_buflen =
                                if (wb_flags & (FIO_UCS2 | FIO_UTF16 | FIO_UTF8)) != 0 {
                                    (bufsize as usize) * 2
                                } else {
                                    (bufsize as usize) * 4
                                };
                            write_info.bw_conv_buf =
                                verbose_try_malloc(write_info.bw_conv_buflen).cast::<c_char>();
                            if write_info.bw_conv_buf.is_null() {
                                end = 0;
                            }
                        }
                    }

                    if converted && wb_flags == 0 {
                        write_info.bw_iconv_fd =
                            my_iconv_open(fenc, c"utf-8".as_ptr().cast_mut()) as iconv_t;
                        if write_info.bw_iconv_fd != -1 {
                            write_info.bw_conv_buflen = (bufsize as usize) * ICONV_MULT;
                            write_info.bw_conv_buf =
                                verbose_try_malloc(write_info.bw_conv_buflen).cast::<c_char>();
                            if write_info.bw_conv_buf.is_null() {
                                end = 0;
                            }
                            write_info.bw_first = 1;
                        } else if p_ccv.cb_type != kCallbackNone {
                            wfname = vim_tempname();
                            if wfname.is_null() {
                                err = set_err(gettext(
                                    c"E214: Can't find temp file for writing".as_ptr(),
                                ));
                                break 'restore_backup;
                            }
                        }
                    }

                    let mut notconverted = false;
                    if converted && wb_flags == 0 && write_info.bw_iconv_fd == -1 && wfname == fname
                    {
                        if !forceit {
                            err = set_err(gettext(
                                c"E213: Cannot convert (add ! to write without conversion)"
                                    .as_ptr(),
                            ));
                            break 'restore_backup;
                        }
                        notconverted = true;
                    }

                    let mut no_eol = false;
                    let mut nchars: i64;
                    let mut lnum: linenr_T;
                    let mut fileformat: c_int;
                    let mut checking_conversion = true;
                    let mut fd: c_int;

                    loop {
                        if !converted || dobackup {
                            checking_conversion = false;
                        }

                        if checking_conversion {
                            fd = -1;
                            write_info.bw_fd = fd;
                        } else {
                            let fflags = O_WRONLY
                                | if append {
                                    if forceit {
                                        O_APPEND | O_CREAT
                                    } else {
                                        O_APPEND
                                    }
                                } else {
                                    O_CREAT | O_TRUNC
                                };
                            let mode = if perm < 0 { 0o666 } else { perm & 0o777 };

                            loop {
                                fd = os_open(wfname, fflags, mode);
                                if fd >= 0 {
                                    break;
                                }
                                if err.msg.is_null() {
                                    #[cfg(not(windows))]
                                    {
                                        let mut file_info = FileInfo::default();
                                        if (!newfile
                                            && os_fileinfo_hardlinks(&raw const file_info_old) > 1)
                                            || (os_fileinfo_link(fname, &raw mut file_info)
                                                && !os_fileinfo_id_equal(
                                                    &raw const file_info,
                                                    &raw const file_info_old,
                                                ))
                                        {
                                            err = set_err(gettext(
                                                c"E166: Can't open linked file for writing"
                                                    .as_ptr(),
                                            ));
                                        } else {
                                            err = set_err_arg(
                                                gettext(
                                                    c"E212: Can't open file for writing: %s"
                                                        .as_ptr(),
                                                ),
                                                fd,
                                            );
                                            if forceit
                                                && vim_strchr(p_cpo, kCpoFwrite).is_null()
                                                && perm >= 0
                                            {
                                                if (perm & 0o200) == 0 {
                                                    made_writable = true;
                                                }
                                                perm |= 0o200;
                                                if file_info_old.stat.st_uid != u64::from(getuid())
                                                    || file_info_old.stat.st_gid
                                                        != u64::from(getgid())
                                                {
                                                    perm &= 0o777;
                                                }
                                                if !append {
                                                    let _ = os_remove(wfname);
                                                }
                                                continue;
                                            }
                                        }
                                    }
                                    #[cfg(windows)]
                                    {
                                        err = set_err_arg(
                                            gettext(
                                                c"E212: Can't open file for writing: %s".as_ptr(),
                                            ),
                                            fd,
                                        );
                                        if forceit
                                            && vim_strchr(p_cpo, kCpoFwrite).is_null()
                                            && perm >= 0
                                        {
                                            if !append {
                                                let _ = os_remove(wfname);
                                            }
                                            continue;
                                        }
                                    }
                                }
                                break 'restore_backup;
                            }
                            write_info.bw_fd = fd;
                        }
                        err = set_err(ptr::null());

                        write_info.bw_buf = buffer;
                        nchars = 0;

                        let write_bin = if !eap.is_null() && (*eap).force_bin != 0 {
                            (*eap).force_bin == FORCE_BIN
                        } else {
                            (*buf).b_p_bin != 0
                        };

                        if (*buf).b_p_bomb != 0 && !write_bin && (!append || perm < 0) {
                            write_info.bw_len = make_bom(buffer, fenc);
                            if write_info.bw_len > 0 {
                                write_info.bw_flags = FIO_NOCONVERT | wb_flags;
                                if buf_write_bytes(&raw mut write_info) == FAIL {
                                    end = 0;
                                } else {
                                    nchars += i64::from(write_info.bw_len);
                                }
                            }
                        }
                        write_info.bw_start_lnum = start;

                        write_undo_file = (*buf).b_p_udf != 0
                            && overwriting
                            && !append
                            && !filtering
                            && reset_changed
                            && !checking_conversion;
                        if write_undo_file {
                            sha256_start(&raw mut sha_ctx);
                        }

                        write_info.bw_len = 0;
                        write_info.bw_flags = wb_flags;
                        fileformat = get_fileformat_force(buf, eap);
                        let mut s = buffer;
                        lnum = start;
                        while lnum <= end {
                            let mut ptr = ml_get_buf(buf, lnum).sub(1);
                            if write_undo_file {
                                let line_ptr = ptr.add(1);
                                sha256_update(
                                    &raw mut sha_ctx,
                                    line_ptr.cast(),
                                    c_strlen(line_ptr) + 1,
                                );
                            }
                            loop {
                                ptr = ptr.add(1);
                                let c = *ptr;
                                if c == NUL {
                                    break;
                                }
                                if c == NL_CHAR {
                                    *s = NUL;
                                } else if c == CAR_CHAR && fileformat == EOL_MAC {
                                    *s = NL_CHAR;
                                } else {
                                    *s = c;
                                }
                                s = s.add(1);
                                write_info.bw_len += 1;
                                if write_info.bw_len != bufsize {
                                    continue;
                                }
                                if buf_write_bytes(&raw mut write_info) == FAIL {
                                    end = 0;
                                    break;
                                }
                                nchars += i64::from(bufsize - write_info.bw_len);
                                s = buffer.add(write_info.bw_len as usize);
                                write_info.bw_start_lnum = lnum;
                            }
                            if end == 0
                                || (lnum == end
                                    && (write_bin || (*buf).b_p_fixeol == 0)
                                    && ((write_bin && lnum == (*buf).b_no_eol_lnum)
                                        || (lnum == (*buf).b_ml.ml_line_count
                                            && (*buf).b_p_eol == 0)))
                            {
                                lnum += 1;
                                no_eol = true;
                                break;
                            }
                            if fileformat == EOL_UNIX {
                                *s = NL_CHAR;
                                s = s.add(1);
                            } else {
                                *s = CAR_CHAR;
                                s = s.add(1);
                                if fileformat == EOL_DOS {
                                    write_info.bw_len += 1;
                                    if write_info.bw_len == bufsize {
                                        if buf_write_bytes(&raw mut write_info) == FAIL {
                                            end = 0;
                                            break;
                                        }
                                        nchars += i64::from(bufsize - write_info.bw_len);
                                        s = buffer.add(write_info.bw_len as usize);
                                    }
                                    *s = NL_CHAR;
                                    s = s.add(1);
                                }
                            }
                            write_info.bw_len += 1;
                            if write_info.bw_len == bufsize {
                                if buf_write_bytes(&raw mut write_info) == FAIL {
                                    end = 0;
                                    break;
                                }
                                nchars += i64::from(bufsize - write_info.bw_len);
                                s = buffer.add(write_info.bw_len as usize);

                                os_breakcheck();
                                if got_int {
                                    end = 0;
                                    break;
                                }
                            }

                            if show_msg && nchars != progress_nchars {
                                progress_nchars = nchars;
                                let percent = (i64::from(lnum - start) * 100
                                    / i64::from(end - start + 1))
                                    as c_int;
                                if percent != progress
                                    && (progress >= 0 || os_hrtime() - write_start > slow_io_ns())
                                {
                                    progress = percent;
                                    let iobuf = (&raw mut IObuff).cast::<c_char>();
                                    add_quoted_fname(iobuf, IOSIZE, buf, msgname);
                                    let _ = msg_progress(
                                        iobuf,
                                        msg_id.as_mut_ptr(),
                                        c"running".as_ptr().cast_mut(),
                                        progress,
                                        0,
                                        false,
                                        true,
                                        false,
                                    );
                                    if buf_get_changedtick(buf) != write_tick {
                                        err = set_err(gettext(
                                            c"E787: Buffer changed unexpectedly".as_ptr(),
                                        ));
                                        end = 0;
                                        break;
                                    }
                                }
                            }

                            lnum += 1;
                        }
                        if write_info.bw_len > 0 && end > 0 {
                            let remaining = write_info.bw_len;
                            if buf_write_bytes(&raw mut write_info) == FAIL {
                                end = 0;
                            }
                            nchars += i64::from(remaining - write_info.bw_len);
                        }

                        if end != 0 && write_info.bw_len > 0 {
                            write_info.bw_conv_error = 1;
                            write_info.bw_conv_error_lnum = end;
                            end = 0;
                        }

                        if (*buf).b_p_fixeol == 0 && (*buf).b_p_eof != 0 {
                            let _ = write_eintr(write_info.bw_fd, c"\x1a".as_ptr().cast(), 1);
                        }

                        if !checking_conversion || end == 0 {
                            break;
                        }
                        checking_conversion = false;
                    }

                    if !checking_conversion {
                        let do_fsync = if (*buf).b_p_fs >= 0 {
                            (*buf).b_p_fs != 0
                        } else {
                            p_fs != 0
                        };
                        if do_fsync {
                            let error = os_fsync(fd);
                            if error != 0 && error != UV_ENOTSUP && !device {
                                err = set_err_arg(e_fsync.as_ptr(), error);
                                end = 0;
                            }
                        }

                        if !backup_copy {
                            os_copy_xattr(backup, wfname);
                        }

                        #[cfg(not(windows))]
                        if !backup.is_null() && !backup_copy {
                            let mut file_info = FileInfo::default();
                            if !os_fileinfo(wfname, &raw mut file_info)
                                || file_info.stat.st_uid != file_info_old.stat.st_uid
                                || file_info.stat.st_gid != file_info_old.stat.st_gid
                            {
                                let _ = os_fchown(
                                    fd,
                                    file_info_old.stat.st_uid as uv_uid_t,
                                    file_info_old.stat.st_gid as uv_gid_t,
                                );
                                if perm >= 0 {
                                    let _ = os_setperm(wfname, perm);
                                }
                            }
                            buf_set_file_id(buf);
                        } else if !(*buf).file_id_valid {
                            buf_set_file_id(buf);
                        }

                        let close_err = os_close(fd);
                        if close_err != 0 {
                            err =
                                set_err_arg(gettext(c"E512: Close failed: %s".as_ptr()), close_err);
                            end = 0;
                        }

                        #[cfg(not(windows))]
                        if made_writable {
                            perm &= !0o200;
                        }
                        if perm >= 0 {
                            let _ = os_setperm(wfname, perm);
                        }
                        if !backup_copy {
                            os_set_acl(wfname, acl);
                        }

                        if wfname != fname {
                            if end != 0
                                && eval_charconvert(c"utf-8".as_ptr(), fenc, wfname, fname) == FAIL
                            {
                                write_info.bw_conv_error = 1;
                                end = 0;
                            }
                            let _ = os_remove(wfname);
                            xfree(wfname.cast());
                        }
                    }

                    if end == 0 {
                        if err.msg.is_null() {
                            if write_info.bw_conv_error != 0 {
                                if write_info.bw_conv_error_lnum == 0 {
                                    err = set_err(gettext(
                                        c"E513: Write error, conversion failed (make 'fenc' empty to override)"
                                            .as_ptr(),
                                    ));
                                } else {
                                    let alloc_msg = xmalloc(300).cast::<c_char>();
                                    let _ = vim_snprintf!(
                                        alloc_msg,
                                        300,
                                        gettext(
                                            c"E513: Write error, conversion failed in line %d (make 'fenc' empty to override)"
                                                .as_ptr(),
                                        ),
                                        write_info.bw_conv_error_lnum,
                                    );
                                    err = Error_T {
                                        num: ptr::null(),
                                        msg: alloc_msg,
                                        arg: 0,
                                        alloc: true,
                                    };
                                }
                            } else if got_int {
                                err = set_err(gettext(e_interr.as_ptr()));
                            } else {
                                err = set_err(gettext(
                                    c"E514: Write error (file system full?)".as_ptr(),
                                ));
                            }
                        }

                        if !backup.is_null() {
                            if backup_copy {
                                if got_int {
                                    msg(gettext(e_interr.as_ptr()), 0);
                                    ui_flush();
                                }
                                if os_copy(backup, fname, UV_FS_COPYFILE_FICLONE) == 0 {
                                    end = 1;
                                }
                            } else if vim_rename(backup, fname) == 0 {
                                end = 1;
                            }
                        }
                        break 'fail;
                    }

                    lnum -= start;
                    no_wait_return -= 1;

                    #[cfg(windows)]
                    {
                        fname = sfname;
                    }
                    if !filtering {
                        let iobuf = (&raw mut IObuff).cast::<c_char>();
                        add_quoted_fname(iobuf, IOSIZE, buf, fname);
                        let mut insert_space = false;
                        if write_info.bw_conv_error != 0 {
                            xstrlcat(iobuf, gettext(c" CONVERSION ERROR".as_ptr()), IOSIZE);
                            insert_space = true;
                            if write_info.bw_conv_error_lnum != 0 {
                                let _ = vim_snprintf_add!(
                                    iobuf,
                                    IOSIZE,
                                    gettext(c" in line %lld;".as_ptr()),
                                    i64::from(write_info.bw_conv_error_lnum),
                                );
                            }
                        } else if notconverted {
                            xstrlcat(iobuf, gettext(c"[NOT converted]".as_ptr()), IOSIZE);
                            insert_space = true;
                        } else if converted {
                            xstrlcat(iobuf, gettext(c"[converted]".as_ptr()), IOSIZE);
                            insert_space = true;
                        }
                        if device {
                            xstrlcat(iobuf, gettext(c"[Device]".as_ptr()), IOSIZE);
                            insert_space = true;
                        } else if newfile {
                            xstrlcat(iobuf, gettext(c"[New]".as_ptr()), IOSIZE);
                            insert_space = true;
                        }
                        if no_eol {
                            xstrlcat(iobuf, gettext(c"[noeol]".as_ptr()), IOSIZE);
                            insert_space = true;
                        }
                        if msg_add_fileformat(fileformat) {
                            insert_space = true;
                        }
                        msg_add_lines(c_int::from(insert_space), lnum, nchars);
                        if !shortmess(kShmWrite) {
                            if append {
                                xstrlcat(
                                    iobuf,
                                    if shortmess(kShmWri) {
                                        gettext(c" [a]".as_ptr())
                                    } else {
                                        gettext(c" appended".as_ptr())
                                    },
                                    IOSIZE,
                                );
                            } else {
                                xstrlcat(
                                    iobuf,
                                    if shortmess(kShmWri) {
                                        gettext(c" [w]".as_ptr())
                                    } else {
                                        gettext(c" written".as_ptr())
                                    },
                                    IOSIZE,
                                );
                            }
                        }
                        set_keep_msg(
                            if progress >= 0 {
                                msg_progress(
                                    iobuf,
                                    msg_id.as_mut_ptr(),
                                    c"success".as_ptr().cast_mut(),
                                    -1,
                                    0,
                                    true,
                                    true,
                                    false,
                                )
                            } else {
                                msg_with_id(
                                    iobuf,
                                    msg_id.as_mut_ptr(),
                                    c"bufwrite".as_ptr(),
                                    ptr::null_mut(),
                                    0,
                                    true,
                                    true,
                                    false,
                                )
                            },
                            0,
                        );
                    }

                    if reset_changed
                        && whole
                        && !append
                        && write_info.bw_conv_error == 0
                        && (overwriting || !vim_strchr(p_cpo, kCpoPlus).is_null())
                        && buf_get_changedtick(buf) == write_tick
                    {
                        unchanged(buf, true, false);
                        let changedtick = buf_get_changedtick(buf);
                        if (*buf).b_last_changedtick + 1 == changedtick {
                            (*buf).b_last_changedtick = changedtick;
                        }
                        u_unchanged(buf);
                        u_update_save_nr(buf);
                    }

                    if overwriting {
                        ml_timestamp(buf);
                        if append {
                            (*buf).b_flags &= !BF_NEW;
                        } else {
                            (*buf).b_flags &= !BF_WRITE_MASK;
                        }
                    }

                    if *p_pm != 0 && dobackup {
                        let org = modname(fname, p_pm, false);
                        if !backup.is_null() {
                            if org.is_null() {
                                emsg(gettext(
                                    c"E205: Patchmode: can't save original file".as_ptr(),
                                ));
                            } else if !os_path_exists(org) {
                                let _ = vim_rename(backup, org);
                                xfree(backup.cast());
                                backup = ptr::null_mut();
                                #[cfg(not(windows))]
                                {
                                    let _ = os_file_settime(
                                        org,
                                        file_info_old.stat.st_atim.tv_sec as f64,
                                        file_info_old.stat.st_mtim.tv_sec as f64,
                                    );
                                }
                            }
                        } else {
                            let empty_fd = if org.is_null() {
                                -1
                            } else {
                                os_open(
                                    org,
                                    O_CREAT | O_EXCL | O_NOFOLLOW,
                                    if perm < 0 { 0o666 } else { perm & 0o777 },
                                )
                            };
                            if empty_fd < 0 {
                                emsg(gettext(
                                    c"E206: Patchmode: can't touch empty original file".as_ptr(),
                                ));
                            } else {
                                let _ = close_fd(empty_fd);
                            }
                        }
                        if !org.is_null() {
                            let _ = os_setperm(org, os_getperm(fname) & 0o777);
                            xfree(org.cast());
                        }
                    }

                    if p_bk == 0
                        && !backup.is_null()
                        && write_info.bw_conv_error == 0
                        && os_remove(backup) != 0
                    {
                        emsg(gettext(c"E207: Can't delete backup file".as_ptr()));
                    }

                    break 'nofail;
                } // 'restore_backup

                if !backup.is_null() && wfname == fname {
                    if backup_copy {
                        if !os_path_exists(fname) {
                            let _ = vim_rename(backup, fname);
                        }
                        if os_path_exists(fname) {
                            let _ = os_remove(backup);
                        }
                    } else {
                        let _ = vim_rename(backup, fname);
                    }
                }
                if !newfile && !os_path_exists(fname) {
                    end = 0;
                }
                if wfname != fname {
                    xfree(wfname.cast());
                }
            } // 'fail

            no_wait_return -= 1;
        } // 'nofail

        if show_msg {
            ui_busy_stop();
        }

        xfree(backup.cast());
        if !ptr::eq(buffer, smallbuf.as_ptr()) {
            xfree(buffer.cast());
        }
        xfree(fenc_tofree.cast());
        xfree(write_info.bw_conv_buf.cast());
        if write_info.bw_iconv_fd != -1 {
            let _ = call_iconv_close(write_info.bw_iconv_fd);
        }
        os_free_acl(acl);

        if !err.msg.is_null() {
            add_quoted_fname((&raw mut IObuff).cast(), IOSIZE - 100, buf, msgname);
            emit_err(
                &raw mut err,
                if progress >= 0 {
                    msg_id.as_ptr()
                } else {
                    ptr::null()
                },
            );

            retval = FAIL;
            if end == 0 {
                msg_puts_hl(
                    gettext(c"\nWARNING: Original file may be lost or damaged\n".as_ptr()),
                    HLF_E,
                    true,
                );
                msg_puts_hl(
                    gettext(
                        c"don't quit the editor until the file is successfully written!".as_ptr(),
                    ),
                    HLF_E,
                    true,
                );

                if os_fileinfo(fname, &raw mut file_info_old) {
                    buf_store_file_info(buf, &raw mut file_info_old);
                    (*buf).b_mtime_read = (*buf).b_mtime;
                    (*buf).b_mtime_read_ns = (*buf).b_mtime_ns;
                }
            }
        }

        (*buf).b_saving = false;
        msg_scroll = msg_save;

        if retval == OK && write_undo_file {
            let mut hash = [0u8; UNDO_HASH_SIZE];
            sha256_finish(&raw mut sha_ctx, hash.as_mut_ptr());
            u_write_undo(ptr::null(), false, buf, hash.as_mut_ptr());
        }

        if !should_abort(retval) {
            buf_write_do_post_autocmds(buf, fname, eap, append, filtering, reset_changed, whole);
            if aborting() {
                retval = FAIL;
            }
        }

        got_int |= prev_got_int;

        xfree(fname_copy.cast());
        xfree(sfname_copy.cast());
        xfree(ffname_copy.cast());

        retval
    }
    #[cfg(test)]
    {
        FAIL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ucs2bytes_and_make_bom_encode_utf8_and_utf16() {
        unsafe {
            let mut buf = [0 as c_char; 8];
            assert_eq!(make_bom(buf.as_mut_ptr(), ptr::null_mut()), 3);
            assert_eq!(
                [buf[0] as u8, buf[1] as u8, buf[2] as u8],
                [0xef, 0xbb, 0xbf]
            );

            let mut p = buf.as_mut_ptr();
            assert!(!ucs2bytes(0xfeff, &raw mut p, FIO_UCS2 | FIO_ENDIAN_L));
            assert_eq!([buf[0] as u8, buf[1] as u8], [0xff, 0xfe]);
        }
    }
}
