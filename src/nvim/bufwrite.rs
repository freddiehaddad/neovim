//! Buffer file writing, encoding conversion, and backup management (`src/nvim/bufwrite.c`).

use crate::autocmd::{
    EVENT_BUFWRITECMD, EVENT_BUFWRITEPOST, EVENT_BUFWRITEPRE, EVENT_FILEAPPENDCMD,
    EVENT_FILEAPPENDPOST, EVENT_FILEAPPENDPRE, EVENT_FILEWRITECMD, EVENT_FILEWRITEPOST,
    EVENT_FILEWRITEPRE, EVENT_FILTERWRITEPOST, EVENT_FILTERWRITEPRE, apply_autocmds_exarg,
};
#[cfg(unix)]
use crate::buffer::buf_set_file_id;
use crate::buffer::{
    BF_NEW, BF_WRITE_MASK, Bufref, FileBuf, NOTDONE, bt_nofilename, buf_get_changedtick,
    bufref_valid, set_bufref,
};
use crate::change::unchanged;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{CursorPos, Linenr};
use crate::dialog::ask_yesno;
use crate::drawscreen::status_redraw_all;
use crate::errors::*;
use crate::eval::vars::{FORCE_BIN, eval_charconvert};
use crate::event::r#loop::uv_strerror;
#[cfg(unix)]
use crate::event::r#loop::{uv_gid_t, uv_uid_t};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{CMOD_LOCKMARKS, Exarg, UNDO_HASH_SIZE};
use crate::ex_eval::{aborting, should_abort};
use crate::fileio::{
    EOL_DOS, EOL_UNIX, FIO_ENDIAN_L, FIO_LATIN1, FIO_NOCONVERT, FIO_UCS2, FIO_UCS4, FIO_UTF8,
    FIO_UTF16, ICONV_EINVAL, K_SHM_OVER, WRITEBUFSIZE, add_quoted_fname, buf_store_file_info,
    call_iconv, call_iconv_close, filemess_bufwrite, fname_msg_id, get_fio_flags, iconv_t,
    match_file_list, modname, msg_add_fileformat, msg_add_lines, need_conversion, set_rw_fname,
    slow_io_ns, time_differs, vim_rename, vim_tempname, write_eintr,
};
#[cfg(not(windows))]
use crate::fileio::{getgid, getuid};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::mbyte::{enc_canonize, my_iconv_open, utf_ptr2char, utf_ptr2len_len};
use crate::memline::{get_file_in_dir, make_percent_swname, ml_get_buf, ml_preserve, ml_timestamp};
use crate::memory::xstrlcpy;
use crate::message::{emsg, msg, msg_progress, msg_puts_hl, msg_with_id, semsg, set_keep_msg};
use crate::option::{copy_option_part, get_bkc_flags, get_fileformat_force, shortmess};
use crate::option_vars::*;
use crate::os::fs::{
    FileInfo, MAXPATHL, O_APPEND, O_CREAT, O_EXCL, O_NOFOLLOW, O_TRUNC, O_WRONLY, VimAcl, os_close,
    os_copy, os_copy_xattr, os_errno, os_file_is_writable, os_fileinfo, os_fileinfo_hardlinks,
    os_fileinfo_id_equal, os_fileinfo_link, os_free_acl, os_fsync, os_get_acl, os_getperm,
    os_isdir, os_mkdir_recurse, os_nodetype, os_open, os_path_exists, os_remove, os_set_acl,
    os_setperm,
};
#[cfg(unix)]
use crate::os::fs::{os_chown, os_fchown, os_file_settime, os_fsetperm, s_isdir, s_isreg};
use crate::os::input::os_breakcheck;
use crate::os::time::os_hrtime;
use crate::path::{K_PATH_CMP_LITERAL, after_pathsep, path_equal, path_tail};
use crate::sha256::ContextSha256;
use crate::strings::{gettext, vim_snprintf, vim_snprintf_add};
use crate::ui::{ui_busy_start, ui_busy_stop, ui_flush};
use crate::undo::{curbuf_is_changed_fn, u_unchanged, u_update_save_nr, u_write_undo};
use std::ffi::CStr;
use std::ptr;

use crate::api::private::helpers::HLF_E;
use crate::arglist::ML_EMPTY;
use crate::charset::EOL_MAC;
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::math::{FAIL, OK};
use crate::optionstr::{K_OPT_BKC_FLAG_AUTO, K_OPT_BKC_FLAG_YES};
#[cfg(windows)]
use crate::os::fs::NODE_OTHER;

const NL: u32 = b'\n' as u32;
const NL_CHAR: u8 = b'\n';
const CAR_CHAR: u8 = b'\r';

const ICONV_MULT: usize = 8;

pub(crate) const NODE_WRITABLE: i32 = 1;

#[cfg(unix)]
const K_OPT_BKC_FLAG_BREAKSYMLINK: u32 = 0x08;
#[cfg(unix)]
const K_OPT_BKC_FLAG_BREAKHARDLINK: u32 = 0x10;

const K_CPO_FNAMEAPP: i32 = b'P' as i32;
const K_CPO_FNAMEW: i32 = b'F' as i32;
const K_CPO_FWRITE: i32 = b'W' as i32;
const K_CPO_KEEPRO: i32 = b'Z' as i32;
const K_CPO_PLUS: i32 = b'+' as i32;

const K_SHM_WRI: i32 = b'w' as i32;
const K_SHM_WRITE: i32 = b'W' as i32;

const UV_FS_COPYFILE_FICLONE: i32 = 0x0002;

use crate::os::fs::UV_ENOTSUP;

#[repr(C)]
struct BwError {
    num: Option<&'static CStr>,
    msg: Option<std::ffi::CString>,
    arg: i32,
}

#[repr(C)]
struct BwInfo {
    bw_fd: i32,
    bw_buf: Vec<u8>,
    bw_len: i32,
    bw_flags: i32,
    bw_first: i32,
    bw_conv_buf: Vec<u8>,
    bw_conv_error: i32,
    bw_conv_error_lnum: Linenr,
    bw_start_lnum: Linenr,
    bw_iconv_fd: iconv_t,
}

/// Convert a Unicode character to bytes.
fn ucs2bytes(mut c: u32, pp: &mut *mut u8, flags: i32) -> bool {
    unsafe {
        let mut p = *pp;
        let mut error = false;

        if (flags & FIO_UCS4) != 0 {
            if (flags & FIO_ENDIAN_L) != 0 {
                *p = c as u8;
                *p.wrapping_add(1) = (c >> 8) as u8;
                *p.wrapping_add(2) = (c >> 16) as u8;
                *p.wrapping_add(3) = (c >> 24) as u8;
            } else {
                *p = (c >> 24) as u8;
                *p.wrapping_add(1) = (c >> 16) as u8;
                *p.wrapping_add(2) = (c >> 8) as u8;
                *p.wrapping_add(3) = c as u8;
            }
            p = p.wrapping_add(4);
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
                        *p.wrapping_add(1) = (cc >> 8) as u8;
                    } else {
                        *p = (cc >> 8) as u8;
                        *p.wrapping_add(1) = cc as u8;
                    }
                    p = p.wrapping_add(2);
                    c = (c & 0x3ff) + 0xdc00;
                } else {
                    error = true;
                }
            }
            if (flags & FIO_ENDIAN_L) != 0 {
                *p = c as u8;
                *p.wrapping_add(1) = (c >> 8) as u8;
            } else {
                *p = (c >> 8) as u8;
                *p.wrapping_add(1) = c as u8;
            }
            p = p.wrapping_add(2);
        } else if c >= 0x100 {
            error = true;
            *p = 0xbf;
            p = p.wrapping_add(1);
        } else {
            *p = c as u8;
            p = p.wrapping_add(1);
        }

        *pp = p;
        error
    }
}

fn buf_write_convert_with_iconv(ip: &mut BwInfo, bufp: &mut *mut u8, lenp: &mut i32) -> i32 {
    let len = *lenp;
    let mut from = (*bufp).cast_const();
    let mut fromlen = len as usize;
    let mut tolen = ip.bw_conv_buf.len();
    let conv_buf = ip.bw_conv_buf.as_mut_ptr();
    let mut to = conv_buf;

    if ip.bw_first != 0 {
        let save_len = tolen;
        let _ = call_iconv(
            ip.bw_iconv_fd,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut to,
            &raw mut tolen,
        );
        if to.is_null() {
            to = conv_buf;
            tolen = save_len;
        }
        ip.bw_first = 0;
    }

    if call_iconv(
        ip.bw_iconv_fd,
        &raw mut from,
        &raw mut fromlen,
        &raw mut to,
        &raw mut tolen,
    ) == usize::MAX
        && os_errno() != ICONV_EINVAL
    {
        ip.bw_conv_error = 1;
        return -1;
    }

    *bufp = conv_buf;
    *lenp = ((to as usize) - (conv_buf as usize)) as i32;

    len - (fromlen as i32)
}

fn buf_write_convert(ip: &mut BwInfo, bufp: &mut *mut u8, lenp: &mut i32) -> i32 {
    unsafe {
        let flags = ip.bw_flags;
        let mut wlen = *lenp;
        let conv_buflen = ip.bw_conv_buf.len();
        let conv_buf = ip.bw_conv_buf.as_mut_ptr();

        if (flags & (FIO_UCS4 | FIO_UTF16 | FIO_UCS2 | FIO_LATIN1)) != 0 {
            let mut p = if (flags & FIO_LATIN1) != 0 {
                *bufp
            } else {
                conv_buf
            };
            wlen = 0;
            while wlen < *lenp {
                let n = utf_ptr2len_len((*bufp).wrapping_add(wlen as usize).cast(), *lenp - wlen);
                if n > *lenp - wlen {
                    break;
                }
                let c: u32 = if n > 1 {
                    utf_ptr2char((*bufp).wrapping_add(wlen as usize).cast()) as u32
                } else {
                    u32::from(*(*bufp).wrapping_add(wlen as usize))
                };

                if (flags & FIO_LATIN1) == 0 {
                    let mut need: usize = if (flags & FIO_UCS4) != 0 { 4 } else { 2 };
                    if (flags & FIO_UTF16) != 0 && c >= 0x10000 {
                        need = 4;
                    }
                    if ((p as usize) - (conv_buf as usize)) + need > conv_buflen {
                        return FAIL;
                    }
                }

                if ucs2bytes(c, &mut p, flags) && ip.bw_conv_error == 0 {
                    ip.bw_conv_error = 1;
                    ip.bw_conv_error_lnum = ip.bw_start_lnum;
                }
                if c == NL {
                    ip.bw_start_lnum += 1;
                }
                wlen += n;
            }
            if (flags & FIO_LATIN1) != 0 {
                *lenp = ((p as usize) - (*bufp as usize)) as i32;
            } else {
                *bufp = conv_buf;
                *lenp = ((p as usize) - (conv_buf as usize)) as i32;
            }
        }

        if ip.bw_iconv_fd != -1 {
            return buf_write_convert_with_iconv(ip, bufp, lenp);
        }

        wlen
    }
}

fn buf_write_bytes(ip: &mut BwInfo) -> bool {
    let mut buf = ip.bw_buf.as_mut_ptr();
    let mut len = ip.bw_len;
    let flags = ip.bw_flags;

    if (flags & FIO_NOCONVERT) == 0 {
        let converted = buf_write_convert(ip, &mut buf, &mut len);
        if converted < 0 {
            return false;
        }
        let remaining = ip.bw_len - converted;
        ip.bw_len = remaining;

        if ip.bw_fd >= 0 {
            let wlen = write_eintr(ip.bw_fd, buf.cast(), len as usize) as i32;
            if wlen < len {
                return false;
            }
        }

        if remaining > 0 {
            let c = converted as usize;
            let r = remaining as usize;
            ip.bw_buf.as_mut_slice().copy_within(c..c + r, 0);
        }
    } else {
        ip.bw_len = 0;
        if ip.bw_fd >= 0 {
            let wlen = write_eintr(ip.bw_fd, buf.cast(), len as usize) as i32;
            if wlen < len {
                return false;
            }
        }
    }

    true
}

fn check_mtime(buf: &mut FileBuf, file_info: &FileInfo) -> bool {
    {
        if buf.b_mtime_read != 0 && time_differs(file_info, buf.b_mtime_read, buf.b_mtime_read_ns) {
            g_st().msg_scroll = 1;
            g_st().msg_silent = 0;
            msg(
                gettext(c"WARNING: The file has been changed since reading it!!!"),
                HLF_E,
            );
            if ask_yesno(gettext(c"Do you really want to write to it")) == b'n' as i32 {
                return false;
            }
            g_st().msg_scroll = 0;
        }
    }
    true
}

fn make_bom(buf_in: *mut u8, name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr().cast_mut();
    {
        let buf = buf_in;
        let flags = get_fio_flags(name);

        if flags == FIO_LATIN1 || flags == 0 {
            return 0;
        }
        if flags == FIO_UTF8 {
            unsafe {
                *buf = 0xef;
                *buf.wrapping_add(1) = 0xbb;
                *buf.wrapping_add(2) = 0xbf;
            }
            return 3;
        }
        let mut p = buf_in;
        let _ = ucs2bytes(0xfeff, &mut p, flags);
        ((p as usize) - (buf_in as usize)) as i32
    }
}

fn buf_write_do_autocmds<T>(
    mut buf: *mut FileBuf,
    fnamep: *mut *mut T,
    sfnamep: *mut *mut T,
    ffnamep: *mut *mut T,
    start: Linenr,
    endp: &mut Linenr,
    eap: Option<&Exarg>,
    append: bool,
    filtering: bool,
    reset_changed: bool,
    overwriting: bool,
    whole: bool,
    orig_start: CursorPos,
    orig_end: CursorPos,
) -> i32 {
    let fnamep = fnamep.cast();
    let sfnamep = sfnamep.cast();
    let ffnamep = ffnamep.cast();
    unsafe {
        let old_line_count = (*buf).b_ml.ml_line_count;
        let msg_save = g_st().msg_scroll;

        let mut aco: CtxSwitch = CtxSwitch::new();
        let mut did_cmd = false;
        let mut nofile_err = false;
        let empty_memline = (*buf).b_ml.ml_mfp.is_none();
        let mut bufref = Bufref {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };

        let sfname = *sfnamep;
        let buf_ffname = *ffnamep == (*buf).b_ffname;
        let buf_sfname = sfname == (*buf).b_sfname;
        let buf_fname_f = *fnamep == (*buf).b_ffname;
        let buf_fname_s = *fnamep == (*buf).b_sfname;

        let _ = ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        set_bufref(&mut bufref, buf);

        if append {
            did_cmd = apply_autocmds_exarg(
                EVENT_FILEAPPENDCMD,
                sfname,
                sfname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
            if !did_cmd {
                if overwriting && bt_nofilename(Some(crate::globals::curbuf_mut())) {
                    nofile_err = true;
                } else {
                    let _ = apply_autocmds_exarg(
                        EVENT_FILEAPPENDPRE,
                        sfname,
                        sfname,
                        false,
                        g_st().curbuf,
                        (eap.is_some()).then(|| eap.unwrap()),
                    );
                }
            }
        } else if filtering {
            let _ = apply_autocmds_exarg(
                EVENT_FILTERWRITEPRE,
                ptr::null_mut(),
                sfname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
        } else if reset_changed && whole {
            let was_changed = curbuf_is_changed_fn();
            did_cmd = apply_autocmds_exarg(
                EVENT_BUFWRITECMD,
                sfname,
                sfname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
            if did_cmd {
                if was_changed && !curbuf_is_changed_fn() {
                    u_unchanged(curbuf_mut());
                    u_update_save_nr(curbuf_mut());
                }
            } else if overwriting && bt_nofilename(Some(crate::globals::curbuf_mut())) {
                nofile_err = true;
            } else {
                let _ = apply_autocmds_exarg(
                    EVENT_BUFWRITEPRE,
                    sfname,
                    sfname,
                    false,
                    g_st().curbuf,
                    (eap.is_some()).then(|| eap.unwrap()),
                );
            }
        } else {
            did_cmd = apply_autocmds_exarg(
                EVENT_FILEWRITECMD,
                sfname,
                sfname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
            if !did_cmd {
                if overwriting && bt_nofilename(Some(crate::globals::curbuf_mut())) {
                    nofile_err = true;
                } else {
                    let _ = apply_autocmds_exarg(
                        EVENT_FILEWRITEPRE,
                        sfname,
                        sfname,
                        false,
                        g_st().curbuf,
                        (eap.is_some()).then(|| eap.unwrap()),
                    );
                }
            }
        }

        ctx_restore(&mut aco);

        if !bufref_valid(Some(&bufref)) {
            buf = ptr::null_mut();
        }
        if buf.is_null()
            || ((*buf).b_ml.ml_mfp.is_none() && !empty_memline)
            || did_cmd
            || nofile_err
            || aborting()
        {
            if !buf.is_null() && (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
                (*buf).b_op_start = orig_start;
                (*buf).b_op_end = orig_end;
            }

            g_st().no_wait_return -= 1;
            g_st().msg_scroll = msg_save;
            if nofile_err {
                let _ = semsg!(
                    gettext(c"E676: No matching autocommands for buftype=%s buffer"),
                    curbuf_mut().b_p_bt,
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
                    ml_timestamp(&mut *buf);
                    if append {
                        (*buf).b_flags &= !BF_NEW;
                    } else {
                        (*buf).b_flags &= !BF_WRITE_MASK;
                    }
                }
                if reset_changed
                    && (*buf).b_changed != 0
                    && !append
                    && (overwriting || crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_PLUS))
                {
                    return FAIL;
                }
                return OK;
            }
            if !aborting() {
                emsg(gettext(
                    c"E203: Autocommands deleted or unloaded buffer to be written",
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
                    g_st().no_wait_return -= 1;
                    g_st().msg_scroll = msg_save;
                    emsg(gettext(
                        c"E204: Autocommand changed number of lines in unexpected way",
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

fn buf_write_do_post_autocmds(
    buf: *mut FileBuf,
    fname: impl crate::strings::IntoCCharPtr,
    eap: Option<&Exarg>,
    append: bool,
    filtering: bool,
    reset_changed: bool,
    whole: bool,
) {
    let fname = fname.into_c_char_ptr();
    {
        let mut aco: CtxSwitch = CtxSwitch::new();
        curbuf_mut().b_no_eol_lnum = 0;

        let _ = ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);

        if append {
            let _ = apply_autocmds_exarg(
                EVENT_FILEAPPENDPOST,
                fname,
                fname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
        } else if filtering {
            let _ = apply_autocmds_exarg(
                EVENT_FILTERWRITEPOST,
                ptr::null_mut(),
                fname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
        } else if reset_changed && whole {
            let _ = apply_autocmds_exarg(
                EVENT_BUFWRITEPOST,
                fname,
                fname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
        } else {
            let _ = apply_autocmds_exarg(
                EVENT_FILEWRITEPOST,
                fname,
                fname,
                false,
                g_st().curbuf,
                (eap.is_some()).then(|| eap.unwrap()),
            );
        }

        ctx_restore(&mut aco);
    }
}

#[inline]
fn set_err_num(num: &'static CStr, msg: impl crate::strings::IntoCCharPtr) -> BwError {
    let msg = msg.into_c_char_ptr();
    {
        BwError {
            num: Some(num),
            msg: if msg.is_null() {
                None
            } else {
                Some(crate::memory::cstr_to_cstring(msg))
            },
            arg: 0,
        }
    }
}

#[inline]
fn set_err(msg: impl crate::strings::IntoCCharPtr) -> BwError {
    let msg = msg.into_c_char_ptr();
    {
        BwError {
            num: None,
            msg: if msg.is_null() {
                None
            } else {
                Some(crate::memory::cstr_to_cstring(msg))
            },
            arg: 0,
        }
    }
}

#[inline]
fn set_err_arg(msg: impl crate::strings::IntoCCharPtr, arg: i32) -> BwError {
    let msg = msg.into_c_char_ptr();
    {
        BwError {
            num: None,
            msg: if msg.is_null() {
                None
            } else {
                Some(crate::memory::cstr_to_cstring(msg))
            },
            arg,
        }
    }
}

fn emit_err(e: &mut BwError, msg_id: impl crate::strings::IntoCCharPtr) {
    let msg_id = msg_id.into_c_char_ptr();
    {
        let mut errmsg = [0u8; IOSIZE];
        let errmsg_ptr = errmsg.as_mut_ptr().cast();
        let msg_ptr = e.msg.as_ref().map_or(c"".as_ptr(), |m| m.as_ptr());
        if let Some(num) = e.num {
            if e.arg != 0 {
                let _ = vim_snprintf!(
                    errmsg_ptr,
                    IOSIZE,
                    c"%s: %s%s: %s",
                    num.as_ptr(),
                    g_st().iobuff.as_ptr(),
                    msg_ptr,
                    unsafe { uv_strerror(e.arg) },
                );
            } else {
                let _ = vim_snprintf!(
                    errmsg_ptr,
                    IOSIZE,
                    c"%s: %s%s",
                    num.as_ptr(),
                    g_st().iobuff.as_ptr(),
                    msg_ptr,
                );
            }
        } else if e.arg != 0 {
            let _ = vim_snprintf!(errmsg_ptr, IOSIZE, msg_ptr, unsafe { uv_strerror(e.arg) });
        } else {
            xstrlcpy(errmsg_ptr, msg_ptr, IOSIZE);
        }

        if !msg_id.is_null() {
            let _ = msg_progress(
                errmsg_ptr,
                msg_id.cast_mut(),
                c"failed",
                -1,
                HLF_E,
                false,
                false,
                true,
            );
        } else {
            emsg(errmsg_ptr);
        }
        e.msg = None;
    }
}

#[cfg(not(windows))]
fn get_fileinfo_os(
    fname: impl crate::strings::IntoCCharPtr,
    file_info_old: &mut FileInfo,
    _overwriting: bool,
    perm: &mut i32,
    device: &mut bool,
    newfile: &mut bool,
    err: &mut BwError,
) -> bool {
    let fname = fname.into_c_char_ptr().cast_mut();
    *perm = -1;
    if !os_fileinfo(fname, file_info_old) {
        *newfile = true;
    } else {
        *perm = file_info_old.stat.st_mode as i32;
        if !s_isreg(file_info_old.stat.st_mode) {
            if s_isdir(file_info_old.stat.st_mode) {
                *err = set_err_num(c"E502", gettext(c"is a directory"));
                return false;
            }
            if os_nodetype(fname) != NODE_WRITABLE {
                *err = set_err_num(c"E503", gettext(c"is not a file or writable device"));
                return false;
            }
            *device = true;
            *newfile = true;
            *perm = -1;
        }
    }
    true
}

#[cfg(windows)]
fn get_fileinfo_os(
    fname: impl crate::strings::IntoCCharPtr,
    file_info_old: &mut FileInfo,
    overwriting: bool,
    perm: &mut i32,
    device: &mut bool,
    newfile: &mut bool,
    err: &mut BwError,
) -> bool {
    let fname = fname.into_c_char_ptr().cast_mut();
    let nodetype = if fname.is_null() {
        NODE_OTHER
    } else {
        os_nodetype(fname)
    };
    if nodetype == NODE_OTHER {
        *err = set_err_num(c"E503", gettext(c"is not a file or writable device"));
        return false;
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
            *err = set_err_num(c"E502", gettext(c"is a directory"));
            return false;
        }
        if overwriting {
            let _ = os_fileinfo(fname, file_info_old);
        }
    }
    true
}

fn get_fileinfo(
    buf: &mut FileBuf,
    fname: impl crate::strings::IntoCCharPtr,
    overwriting: bool,
    forceit: bool,
    file_info_old: &mut FileInfo,
    perm: &mut i32,
    device: &mut bool,
    newfile: &mut bool,
    readonly: &mut bool,
    err: &mut BwError,
) -> bool {
    let fname = fname.into_c_char_ptr().cast_mut();
    {
        if !get_fileinfo_os(
            fname,
            file_info_old,
            overwriting,
            perm,
            device,
            newfile,
            err,
        ) {
            return false;
        }

        *readonly = false;

        if !*device && !*newfile {
            *readonly = os_file_is_writable(fname) == 0;

            if !forceit && *readonly {
                if crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_FWRITE) {
                    *err = set_err_num(
                        c"E504",
                        gettext(c"is read-only (cannot override: \"W\" in 'cpoptions')"),
                    );
                } else {
                    *err = set_err_num(c"E505", gettext(c"is read-only (add ! to override)"));
                }
                return false;
            }

            if overwriting && !forceit && !check_mtime(buf, file_info_old) {
                return false;
            }
        }
        true
    }
}

pub fn buf_get_backup_name(
    fname: impl crate::strings::IntoCCharPtr,
    dirp: &mut crate::strings::RawCCharPtrMut,
    no_prepend_dot: bool,
    backup_ext: impl crate::strings::IntoCCharPtr,
) -> Option<std::ffi::CString> {
    let fname = fname.into_c_char_ptr().cast_mut();
    let backup_ext = backup_ext.into_c_char_ptr();
    {
        let mut backup: Option<std::ffi::CString> = None;
        let iobuf = g_st().iobuff.as_mut_ptr();
        let dir_len = copy_option_part(dirp, iobuf.cast(), IOSIZE, c",");
        let p = iobuf.wrapping_add(dir_len);
        if crate::memory::cstr_is_empty(*dirp) && !os_isdir(iobuf) {
            let mut failed_dir: Option<std::ffi::CString> = None;
            let ret = os_mkdir_recurse(iobuf, 0o755, &mut failed_dir, None);
            if ret != 0 {
                let _ = semsg!(
                    gettext(c"E303: Unable to create directory \"%s\" for backup file: %s",),
                    failed_dir.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                    unsafe { uv_strerror(ret) },
                );
            }
        }
        if dir_len > 1
            && after_pathsep(iobuf, p) != 0
            && g_st().iobuff[dir_len - 1] == g_st().iobuff[dir_len - 2]
            && let Some(swname) = make_percent_swname(iobuf, p, fname)
        {
            backup = modname(swname.as_ptr(), backup_ext, no_prepend_dot);
        }
        if backup.is_none()
            && let Some(rootname) = get_file_in_dir(fname, iobuf)
        {
            backup = modname(rootname.as_ptr(), backup_ext, no_prepend_dot);
        }
        backup
    }
}

fn buf_write_make_backup(
    fname: impl crate::strings::IntoCCharPtr,
    append: bool,
    file_info_old: &mut FileInfo,
    acl: VimAcl,
    perm: i32,
    bkc: u32,
    file_readonly: bool,
    forceit: bool,
    backup_copyp: &mut bool,
    backupp: &mut Option<std::ffi::CString>,
    err: &mut BwError,
) -> bool {
    let fname = fname.into_c_char_ptr().cast_mut();
    {
        let mut file_info = FileInfo::new();
        let no_prepend_dot = false;

        if (bkc & K_OPT_BKC_FLAG_YES) != 0 || append {
            *backup_copyp = true;
        } else if (bkc & K_OPT_BKC_FLAG_AUTO) != 0 {
            if os_fileinfo_hardlinks(file_info_old) > 1
                || !os_fileinfo_link(fname, &mut file_info)
                || !os_fileinfo_id_equal(&file_info, file_info_old)
            {
                *backup_copyp = true;
            } else {
                let dirlen = (path_tail(fname) as usize) - (fname as usize);
                assert!(dirlen < MAXPATHL);
                let mut tmp_fname = [0u8; MAXPATHL];
                crate::memory::cstr_ncpy(tmp_fname.as_mut_ptr().cast(), fname, dirlen);
                let mut i: i32 = 4913;
                loop {
                    let suffix = format!(".{i}.swp\0");
                    tmp_fname[dirlen..dirlen + suffix.len()].copy_from_slice(suffix.as_bytes());
                    if !os_fileinfo_link(tmp_fname.as_ptr().cast(), &mut file_info) {
                        break;
                    }
                    i += 123;
                }
                let fd = os_open(
                    tmp_fname.as_ptr().cast(),
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
                            file_info_old.stat.st_uid as uv_uid_t,
                            file_info_old.stat.st_gid as uv_gid_t,
                        );
                        let _ = os_fsetperm(fd, perm);
                        if !os_fileinfo(tmp_fname.as_ptr().cast(), &mut file_info)
                            || file_info.stat.st_uid != file_info_old.stat.st_uid
                            || file_info.stat.st_gid != file_info_old.stat.st_gid
                            || (file_info.stat.st_mode as i32) != perm
                        {
                            *backup_copyp = true;
                        }
                    }
                    os_close(fd);
                    let _ = os_remove(tmp_fname.as_ptr().cast());
                }
            }
        }

        #[cfg(not(windows))]
        if (bkc & (K_OPT_BKC_FLAG_BREAKSYMLINK | K_OPT_BKC_FLAG_BREAKHARDLINK)) != 0 {
            let file_info_link_ok = os_fileinfo_link(fname, &mut file_info);
            if (bkc & K_OPT_BKC_FLAG_BREAKSYMLINK) != 0
                && file_info_link_ok
                && !os_fileinfo_id_equal(&file_info, file_info_old)
            {
                *backup_copyp = false;
            }
            if (bkc & K_OPT_BKC_FLAG_BREAKHARDLINK) != 0
                && os_fileinfo_hardlinks(file_info_old) > 1
                && (!file_info_link_ok || os_fileinfo_id_equal(&file_info, file_info_old))
            {
                *backup_copyp = false;
            }
        }

        let backup_ext = if crate::memory::cstr_is_empty(opt_vars().p_bex) {
            c".bak".as_ptr().cast_mut()
        } else {
            opt_vars().p_bex
        };

        if *backup_copyp {
            let mut some_error = false;
            let mut dirp = opt_vars().p_bdir;
            'nobackup: {
                while !crate::memory::cstr_is_empty(dirp) {
                    *backupp = buf_get_backup_name(fname, &mut dirp, no_prepend_dot, backup_ext);
                    let Some(bk_cstr) = backupp.take() else {
                        some_error = true;
                        break 'nobackup;
                    };
                    let mut bk_vec: Vec<u8> = bk_cstr.into_bytes_with_nul();
                    let mut keep_bk = true;

                    let mut file_info_new = FileInfo::new();
                    if os_fileinfo(&bk_vec, &mut file_info_new) {
                        if os_fileinfo_id_equal(&file_info_new, file_info_old) {
                            keep_bk = false;
                        } else if opt_vars().p_bk == 0 {
                            let blen = crate::memory::cstr_len(&bk_vec);
                            let extlen = crate::memory::cstr_len(backup_ext);
                            let wp_idx = if blen > extlen + 1 {
                                blen - 1 - extlen
                            } else {
                                0
                            };
                            bk_vec[wp_idx] = b'z';
                            while bk_vec[wp_idx] > b'a' && os_fileinfo(&bk_vec, &mut file_info_new)
                            {
                                bk_vec[wp_idx] -= 1;
                            }
                            if bk_vec[wp_idx] == b'a' {
                                keep_bk = false;
                            }
                        }
                    }

                    if keep_bk {
                        let bk_ptr = bk_vec.as_mut_ptr();
                        let _ = os_remove(bk_ptr.cast());
                        if os_copy(fname, bk_ptr.cast(), UV_FS_COPYFILE_FICLONE) != 0 {
                            *err = set_err(gettext(
                                c"E509: Cannot create backup file (add ! to override)",
                            ));
                            continue;
                        }

                        let _ = os_setperm(bk_ptr.cast(), perm & 0o777);

                        #[cfg(not(windows))]
                        {
                            if file_info_new.stat.st_gid != file_info_old.stat.st_gid
                                && os_chown(
                                    bk_ptr.cast(),
                                    u32::MAX as uv_uid_t,
                                    file_info_old.stat.st_gid as uv_gid_t,
                                ) != 0
                            {
                                let _ =
                                    os_setperm(bk_ptr.cast(), (perm & 0o707) | ((perm & 0o7) << 3));
                            }
                            let _ = os_file_settime(
                                bk_ptr.cast(),
                                file_info_old.stat.st_atim.tv_sec as f64,
                                file_info_old.stat.st_mtim.tv_sec as f64,
                            );
                        }

                        os_set_acl(bk_ptr.cast(), acl);
                        os_copy_xattr(fname, bk_ptr.cast());
                        *backupp = Some(crate::memory::cstr_to_cstring(bk_ptr.cast()));
                        *err = set_err(ptr::null());
                        break;
                    }
                }
            }

            if backupp.is_none() && err.msg.is_none() {
                *err = set_err(gettext(
                    c"E509: Cannot create backup file (add ! to override)",
                ));
            }
            if (some_error || err.msg.is_some()) && !forceit {
                return false;
            }
            *err = set_err(ptr::null());
        } else {
            if file_readonly && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_FWRITE) {
                *err = set_err_num(
                    c"E504",
                    gettext(c"is read-only (cannot override: \"W\" in 'cpoptions')"),
                );
                return false;
            }

            let mut dirp = opt_vars().p_bdir;
            while !crate::memory::cstr_is_empty(dirp) {
                *backupp = buf_get_backup_name(fname, &mut dirp, no_prepend_dot, backup_ext);
                if let Some(bk_cstr) = backupp.take() {
                    let mut bk_vec: Vec<u8> = bk_cstr.into_bytes_with_nul();
                    let mut keep_bk = true;
                    if opt_vars().p_bk == 0 && os_path_exists(&bk_vec) {
                        let blen = crate::memory::cstr_len(&bk_vec);
                        let extlen = crate::memory::cstr_len(backup_ext);
                        let p_idx = if blen > extlen + 1 {
                            blen - 1 - extlen
                        } else {
                            0
                        };
                        bk_vec[p_idx] = b'z';
                        while bk_vec[p_idx] > b'a' && os_path_exists(&bk_vec) {
                            bk_vec[p_idx] -= 1;
                        }
                        if bk_vec[p_idx] == b'a' {
                            keep_bk = false;
                        }
                    }
                    if keep_bk && vim_rename(fname, &bk_vec) == 0 {
                        *backupp = Some(crate::memory::cstr_to_cstring(&bk_vec));
                        break;
                    }
                }
            }
            if backupp.is_none() && !forceit {
                *err = set_err(gettext(c"E510: Can't make backup file (add ! to override)"));
                return false;
            }
        }
    }
    true
}

/// Write to file `fname` lines `start` through `end`.
pub fn buf_write(
    mut buf: *mut FileBuf,
    fname: impl crate::strings::IntoCCharPtr,
    sfname: impl crate::strings::IntoCCharPtr,
    mut start: Linenr,
    mut end: Linenr,
    eap: Option<&Exarg>,
    append: bool,
    forceit: bool,
    reset_changed: bool,
    filtering: bool,
) -> i32 {
    let mut fname = fname.into_c_char_ptr().cast_mut();
    let mut sfname = sfname.into_c_char_ptr().cast_mut();
    unsafe {
        let mut retval = OK;
        let msg_save = g_st().msg_scroll;
        let mut prev_got_int = g_st().got_int;
        let whole = start == 1 && end == (*buf).b_ml.ml_line_count;
        let mut write_undo_file = false;
        let mut sha_ctx = ContextSha256::new();
        let bkc = get_bkc_flags(&mut *buf);

        if crate::memory::cstr_is_empty(fname) {
            return FAIL;
        }
        if (*buf).b_ml.ml_mfp.is_none() {
            emsg(gettext(E_EMPTY_BUFFER));
            return FAIL;
        }
        if check_secure() {
            return FAIL;
        }
        if crate::memory::cstr_len(fname) >= MAXPATHL {
            emsg(gettext(E_LONGNAME));
            return FAIL;
        }

        let mut write_info = BwInfo {
            bw_fd: 0,
            bw_buf: Vec::new(),
            bw_len: 0,
            bw_flags: 0,
            bw_first: 0,
            bw_conv_buf: Vec::new(),
            bw_conv_error: 0,
            bw_conv_error_lnum: 0,
            bw_start_lnum: 0,
            bw_iconv_fd: -1,
        };

        if (*buf).b_ffname.is_null()
            && reset_changed
            && whole
            && buf == g_st().curbuf
            && !bt_nofilename(buf.as_ref())
            && !filtering
            && (!append || crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_FNAMEAPP))
            && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_FNAMEW)
        {
            if !set_rw_fname(fname, sfname) {
                return FAIL;
            }
            buf = g_st().curbuf;
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
            !(*buf).b_ffname.is_null() && path_equal(ffname, (*buf).b_ffname, K_PATH_CMP_LITERAL);

        g_st().no_wait_return += 1;

        let orig_start = (*buf).b_op_start;
        let orig_end = (*buf).b_op_end;

        (*buf).b_op_start.lnum = start;
        (*buf).b_op_start.col = 0;
        (*buf).b_op_end.lnum = end;
        (*buf).b_op_end.col = 0;

        let res = buf_write_do_autocmds(
            &mut *buf,
            &raw mut fname,
            &raw mut sfname,
            &raw mut ffname,
            start,
            &mut end,
            (eap.is_some()).then(|| eap.unwrap()),
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
        let buf = &mut *buf;

        let fname_copy = (!fname.is_null()).then(|| crate::memory::cstr_to_cstring(fname));
        let sfname_copy = (!sfname.is_null()).then(|| crate::memory::cstr_to_cstring(sfname));
        let ffname_copy = (!ffname.is_null()).then(|| crate::memory::cstr_to_cstring(ffname));
        fname = fname_copy
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        sfname = sfname_copy
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        ffname = ffname_copy
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            buf.b_op_start = orig_start;
            buf.b_op_end = orig_end;
        }

        g_st().msg_scroll = i32::from(!(shortmess(K_SHM_OVER) && !g_st().exiting));

        let mut msg_id = [0u8; MAXPATHL + 32];
        #[cfg(windows)]
        let msgname = sfname;
        #[cfg(not(windows))]
        let msgname = fname;

        fname_msg_id(
            msg_id.as_mut_ptr(),
            msg_id.len(),
            c"nvim.bufwrite",
            buf,
            msgname,
        );

        let show_msg = !filtering && g_st().msg_silent == 0;
        if show_msg {
            ui_busy_start();
        }
        if !filtering {
            filemess_bufwrite(buf, msgname, msg_id.as_ptr().cast());
        }

        let mut progress: i32 = -1;
        let mut progress_nchars: i64 = 0;
        let write_start = os_hrtime();
        let write_tick = buf_get_changedtick(&*buf);
        g_st().msg_scroll = 0;

        write_info.bw_buf = vec![0; WRITEBUFSIZE];
        let bufsize: i32 = WRITEBUFSIZE as i32;

        let mut err = set_err(ptr::null());
        let mut perm: i32 = -1;
        let mut newfile = false;
        let mut device = false;
        let mut file_readonly = false;
        let mut backup: Option<std::ffi::CString> = None;
        let mut fenc_tofree: Option<std::ffi::CString> = None;
        let mut file_info_old = FileInfo::new();
        let mut acl: VimAcl = ptr::null_mut();

        'nofail: {
            'fail: {
                if !get_fileinfo(
                    &mut *buf,
                    fname,
                    overwriting,
                    forceit,
                    &mut file_info_old,
                    &mut perm,
                    &mut device,
                    &mut newfile,
                    &mut file_readonly,
                    &mut err,
                ) {
                    break 'fail;
                }

                if !newfile {
                    acl = os_get_acl(fname);
                }

                let mut dobackup = opt_vars().p_wb != 0
                    || opt_vars().p_bk != 0
                    || !crate::memory::cstr_is_empty(opt_vars().p_pm);
                if dobackup
                    && !crate::memory::cstr_is_empty(opt_vars().p_bsk)
                    && match_file_list(opt_vars().p_bsk, sfname, ffname)
                {
                    dobackup = false;
                }

                let mut backup_copy = false;
                prev_got_int = g_st().got_int;
                g_st().got_int = false;
                buf.b_saving = true;

                if !(append && crate::memory::cstr_is_empty(opt_vars().p_pm))
                    && !filtering
                    && perm >= 0
                    && dobackup
                    && !buf_write_make_backup(
                        fname,
                        append,
                        &mut file_info_old,
                        acl,
                        perm,
                        bkc,
                        file_readonly,
                        forceit,
                        &mut backup_copy,
                        &mut backup,
                        &mut err,
                    )
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
                    && !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_FWRITE)
                {
                    perm |= 0o200;
                    let _ = os_setperm(fname, perm);
                    made_writable = true;
                }

                if forceit
                    && overwriting
                    && !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_KEEPRO)
                {
                    buf.b_p_ro = 0;
                    g_st().need_maketitle = true;
                    status_redraw_all();
                }

                end = end.min(buf.b_ml.ml_line_count);
                if (buf.b_ml.ml_flags & ML_EMPTY) != 0 {
                    start = end + 1;
                }

                let mut wfname = fname;
                let mut wfname_owned: Option<std::ffi::CString> = None;

                'restore_backup: {
                    if reset_changed
                        && !newfile
                        && overwriting
                        && !(g_st().exiting && backup.is_some())
                    {
                        let fsync_opt = if buf.b_p_fs >= 0 {
                            buf.b_p_fs != 0
                        } else {
                            opt_vars().p_fs != 0
                        };
                        ml_preserve(&mut *buf, false, fsync_opt);
                        if g_st().got_int {
                            err = set_err(gettext(E_INTERR));
                            break 'restore_backup;
                        }
                    }

                    let fenc = if let Some(eap) = eap
                        && eap.force_enc != 0
                    {
                        let e = enc_canonize(eap.cmd.wrapping_add(eap.force_enc as usize));
                        let ptr = e.as_ptr().cast_mut();
                        fenc_tofree = Some(e);
                        ptr
                    } else {
                        buf.b_p_fenc
                    };

                    let converted = need_conversion(fenc);
                    let mut wb_flags: i32 = 0;

                    if converted {
                        wb_flags = get_fio_flags(fenc);
                        if (wb_flags & (FIO_UCS2 | FIO_UCS4 | FIO_UTF16 | FIO_UTF8)) != 0 {
                            let conv_buflen = if (wb_flags & (FIO_UCS2 | FIO_UTF16 | FIO_UTF8)) != 0
                            {
                                (bufsize as usize) * 2
                            } else {
                                (bufsize as usize) * 4
                            };
                            write_info.bw_conv_buf = vec![0; conv_buflen];
                        }
                    }

                    if converted && wb_flags == 0 {
                        write_info.bw_iconv_fd =
                            my_iconv_open(fenc, c"utf-8").map_or(-1, |fd| fd as iconv_t);
                        if write_info.bw_iconv_fd != -1 {
                            write_info.bw_conv_buf = vec![0; (bufsize as usize) * ICONV_MULT];
                            write_info.bw_first = 1;
                        } else if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv)
                            != K_CALLBACK_NONE_CONST
                        {
                            wfname_owned = vim_tempname();
                            match wfname_owned {
                                Some(ref wf) => wfname = wf.as_ptr().cast_mut(),
                                None => {
                                    err =
                                        set_err(gettext(c"E214: Can't find temp file for writing"));
                                    break 'restore_backup;
                                }
                            }
                        }
                    }

                    let mut notconverted = false;
                    if converted && wb_flags == 0 && write_info.bw_iconv_fd == -1 && wfname == fname
                    {
                        if !forceit {
                            err = set_err(gettext(
                                c"E213: Cannot convert (add ! to write without conversion)",
                            ));
                            break 'restore_backup;
                        }
                        notconverted = true;
                    }

                    let mut no_eol = false;
                    let mut nchars: i64;
                    let mut lnum: Linenr;
                    let mut fileformat: i32;
                    let mut checking_conversion = true;
                    let mut fd: i32;

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
                                if err.msg.is_none() {
                                    #[cfg(not(windows))]
                                    {
                                        let mut file_info = FileInfo::new();
                                        if (!newfile && os_fileinfo_hardlinks(&file_info_old) > 1)
                                            || (os_fileinfo_link(fname, &mut file_info)
                                                && !os_fileinfo_id_equal(
                                                    &file_info,
                                                    &file_info_old,
                                                ))
                                        {
                                            err = set_err(gettext(
                                                c"E166: Can't open linked file for writing",
                                            ));
                                        } else {
                                            err = set_err_arg(
                                                gettext(c"E212: Can't open file for writing: %s"),
                                                fd,
                                            );
                                            if forceit
                                                && !crate::strings::vim_haschr(
                                                    opt_vars().p_cpo,
                                                    K_CPO_FWRITE,
                                                )
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
                                            gettext(c"E212: Can't open file for writing: %s"),
                                            fd,
                                        );
                                        if forceit
                                            && !crate::strings::vim_haschr(
                                                opt_vars().p_cpo,
                                                K_CPO_FWRITE,
                                            )
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

                        let buf = &mut *buf;
                        let buffer = write_info.bw_buf.as_mut_ptr();
                        nchars = 0;

                        let write_bin = if let Some(eap) = eap
                            && eap.force_bin != 0
                        {
                            eap.force_bin == FORCE_BIN
                        } else {
                            buf.b_p_bin != 0
                        };

                        if buf.b_p_bomb != 0 && !write_bin && (!append || perm < 0) {
                            write_info.bw_len = make_bom(buffer, fenc);
                            if write_info.bw_len > 0 {
                                write_info.bw_flags = FIO_NOCONVERT | wb_flags;
                                if !buf_write_bytes(&mut write_info) {
                                    end = 0;
                                } else {
                                    nchars += i64::from(write_info.bw_len);
                                }
                            }
                        }
                        write_info.bw_start_lnum = start;

                        write_undo_file = buf.b_p_udf != 0
                            && overwriting
                            && !append
                            && !filtering
                            && reset_changed
                            && !checking_conversion;
                        if write_undo_file {
                            sha_ctx = ContextSha256::new();
                        }

                        write_info.bw_len = 0;
                        write_info.bw_flags = wb_flags;
                        fileformat =
                            get_fileformat_force(&*buf, (eap.is_some()).then(|| eap.unwrap()));
                        let mut s = buffer;
                        lnum = start;
                        while lnum <= end {
                            let mut ptr = ml_get_buf(&mut *buf, lnum).wrapping_sub(1);
                            if write_undo_file {
                                let line_ptr = ptr.wrapping_add(1);
                                sha_ctx.update(crate::memory::cstr_slice(
                                    line_ptr,
                                    crate::memory::cstr_len(line_ptr) + 1,
                                ));
                            }
                            loop {
                                ptr = ptr.wrapping_add(1);
                                let c = *ptr as u8;
                                if c == 0 {
                                    break;
                                }
                                if c == NL_CHAR {
                                    *s = 0;
                                } else if c == CAR_CHAR && fileformat == EOL_MAC {
                                    *s = NL_CHAR;
                                } else {
                                    *s = c;
                                }
                                s = s.wrapping_add(1);
                                write_info.bw_len += 1;
                                if write_info.bw_len != bufsize {
                                    continue;
                                }
                                if !buf_write_bytes(&mut write_info) {
                                    end = 0;
                                    break;
                                }
                                nchars += i64::from(bufsize - write_info.bw_len);
                                s = buffer.wrapping_add(write_info.bw_len as usize);
                                write_info.bw_start_lnum = lnum;
                            }
                            if end == 0
                                || (lnum == end
                                    && (write_bin || buf.b_p_fixeol == 0)
                                    && ((write_bin && lnum == buf.b_no_eol_lnum)
                                        || (lnum == buf.b_ml.ml_line_count && buf.b_p_eol == 0)))
                            {
                                lnum += 1;
                                no_eol = true;
                                break;
                            }
                            if fileformat == EOL_UNIX {
                                *s = NL_CHAR;
                                s = s.wrapping_add(1);
                            } else {
                                *s = CAR_CHAR;
                                s = s.wrapping_add(1);
                                if fileformat == EOL_DOS {
                                    write_info.bw_len += 1;
                                    if write_info.bw_len == bufsize {
                                        if !buf_write_bytes(&mut write_info) {
                                            end = 0;
                                            break;
                                        }
                                        nchars += i64::from(bufsize - write_info.bw_len);
                                        s = buffer.wrapping_add(write_info.bw_len as usize);
                                    }
                                    *s = NL_CHAR;
                                    s = s.wrapping_add(1);
                                }
                            }
                            write_info.bw_len += 1;
                            if write_info.bw_len == bufsize {
                                if !buf_write_bytes(&mut write_info) {
                                    end = 0;
                                    break;
                                }
                                nchars += i64::from(bufsize - write_info.bw_len);
                                s = buffer.wrapping_add(write_info.bw_len as usize);

                                os_breakcheck();
                                if g_st().got_int {
                                    end = 0;
                                    break;
                                }
                            }

                            if show_msg && nchars != progress_nchars {
                                progress_nchars = nchars;
                                let percent = (i64::from(lnum - start) * 100
                                    / i64::from(end - start + 1))
                                    as i32;
                                if percent != progress
                                    && (progress >= 0 || os_hrtime() - write_start > slow_io_ns())
                                {
                                    progress = percent;
                                    let iobuf = g_st().iobuff.as_mut_ptr();
                                    add_quoted_fname(iobuf, IOSIZE, buf, msgname);
                                    let _ = msg_progress(
                                        iobuf,
                                        msg_id.as_mut_ptr().cast(),
                                        c"running",
                                        progress,
                                        0,
                                        false,
                                        true,
                                        false,
                                    );
                                    if buf_get_changedtick(&*buf) != write_tick {
                                        err = set_err(gettext(E_BUFFER_CHANGED_UNEXPECTEDLY));
                                        end = 0;
                                        break;
                                    }
                                }
                            }

                            lnum += 1;
                        }
                        if write_info.bw_len > 0 && end > 0 {
                            let remaining = write_info.bw_len;
                            if !buf_write_bytes(&mut write_info) {
                                end = 0;
                            }
                            nchars += i64::from(remaining - write_info.bw_len);
                        }

                        if end != 0 && write_info.bw_len > 0 {
                            write_info.bw_conv_error = 1;
                            write_info.bw_conv_error_lnum = end;
                            end = 0;
                        }

                        if buf.b_p_fixeol == 0 && buf.b_p_eof != 0 {
                            let _ = write_eintr(write_info.bw_fd, c"\x1a".as_ptr().cast(), 1);
                        }

                        if !checking_conversion || end == 0 {
                            break;
                        }
                        checking_conversion = false;
                    }

                    if !checking_conversion {
                        let do_fsync = if buf.b_p_fs >= 0 {
                            buf.b_p_fs != 0
                        } else {
                            opt_vars().p_fs != 0
                        };
                        if do_fsync {
                            let error = os_fsync(fd);
                            if error != 0 && error != UV_ENOTSUP && !device {
                                err = set_err_arg(E_FSYNC, error);
                                end = 0;
                            }
                        }

                        if !backup_copy {
                            os_copy_xattr(
                                backup.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                                wfname,
                            );
                        }

                        #[cfg(not(windows))]
                        if backup.is_some() && !backup_copy {
                            let mut file_info = FileInfo::new();
                            if !os_fileinfo(wfname, &mut file_info)
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
                            buf_set_file_id(&mut *buf);
                        } else if !buf.file_id_valid {
                            buf_set_file_id(&mut *buf);
                        }

                        let close_err = os_close(fd);
                        if close_err != 0 {
                            err = set_err_arg(gettext(c"E512: Close failed: %s"), close_err);
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
                            if end != 0 && !eval_charconvert(c"utf-8", fenc, wfname, fname) {
                                write_info.bw_conv_error = 1;
                                end = 0;
                            }
                            let _ = os_remove(wfname);
                            drop(wfname_owned.take());
                        }
                    }

                    if end == 0 {
                        if err.msg.is_none() {
                            if write_info.bw_conv_error != 0 {
                                if write_info.bw_conv_error_lnum == 0 {
                                    err = set_err(gettext(
                                        c"E513: Write error, conversion failed (make 'fenc' empty to override)",
                                    ));
                                } else {
                                    let mut alloc_msg: Vec<u8> = vec![0; 300];
                                    let _ = vim_snprintf!(
                                        alloc_msg.as_mut_ptr().cast(),
                                        300,
                                        gettext(
                                            c"E513: Write error, conversion failed in line %d (make 'fenc' empty to override)",
                                        ),
                                        write_info.bw_conv_error_lnum,
                                    );
                                    err = BwError {
                                        num: None,
                                        msg: Some(crate::memory::cstr_to_cstring(
                                            alloc_msg.as_ptr().cast(),
                                        )),
                                        arg: 0,
                                    };
                                }
                            } else if g_st().got_int {
                                err = set_err(gettext(E_INTERR));
                            } else {
                                err = set_err(gettext(c"E514: Write error (file system full?)"));
                            }
                        }

                        if let Some(ref bk) = backup {
                            if backup_copy {
                                if g_st().got_int {
                                    msg(gettext(E_INTERR), 0);
                                    ui_flush();
                                }
                                if os_copy(bk.as_ptr(), fname, UV_FS_COPYFILE_FICLONE) == 0 {
                                    end = 1;
                                }
                            } else if vim_rename(bk.as_ptr(), fname) == 0 {
                                end = 1;
                            }
                        }
                        break 'fail;
                    }

                    lnum -= start;
                    g_st().no_wait_return -= 1;

                    #[cfg(windows)]
                    {
                        fname = sfname;
                    }
                    if !filtering {
                        let iobuf = g_st().iobuff.as_mut_ptr();
                        let iobuf_append = |src: &std::ffi::CStr| {
                            let dlen = crate::memory::cstr_len(iobuf);
                            crate::memory::xstrlcpy(iobuf.wrapping_add(dlen), src, IOSIZE - dlen);
                        };
                        add_quoted_fname(iobuf, IOSIZE, buf, fname);
                        let mut insert_space = false;
                        if write_info.bw_conv_error != 0 {
                            iobuf_append(gettext(c" CONVERSION ERROR"));
                            insert_space = true;
                            if write_info.bw_conv_error_lnum != 0 {
                                let _ = vim_snprintf_add!(
                                    iobuf,
                                    IOSIZE,
                                    gettext(c" in line %lld;"),
                                    i64::from(write_info.bw_conv_error_lnum),
                                );
                            }
                        } else if notconverted {
                            iobuf_append(gettext(c"[NOT converted]"));
                            insert_space = true;
                        } else if converted {
                            iobuf_append(gettext(c"[converted]"));
                            insert_space = true;
                        }
                        if device {
                            iobuf_append(gettext(c"[Device]"));
                            insert_space = true;
                        } else if newfile {
                            iobuf_append(gettext(c"[New]"));
                            insert_space = true;
                        }
                        if no_eol {
                            iobuf_append(gettext(c"[noeol]"));
                            insert_space = true;
                        }
                        if msg_add_fileformat(fileformat) {
                            insert_space = true;
                        }
                        msg_add_lines(i32::from(insert_space), lnum, nchars);
                        if !shortmess(K_SHM_WRITE) {
                            if append {
                                iobuf_append(if shortmess(K_SHM_WRI) {
                                    gettext(c" [a]")
                                } else {
                                    gettext(c" appended")
                                });
                            } else {
                                iobuf_append(if shortmess(K_SHM_WRI) {
                                    gettext(c" [w]")
                                } else {
                                    gettext(c" written")
                                });
                            }
                        }
                        set_keep_msg(
                            if progress >= 0 {
                                msg_progress(
                                    iobuf,
                                    msg_id.as_mut_ptr().cast(),
                                    c"success",
                                    -1,
                                    0,
                                    true,
                                    true,
                                    false,
                                )
                            } else {
                                msg_with_id(
                                    iobuf,
                                    msg_id.as_mut_ptr().cast(),
                                    c"bufwrite",
                                    None,
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
                        && (overwriting || crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_PLUS))
                        && buf_get_changedtick(&*buf) == write_tick
                    {
                        unchanged(&mut *buf, true, false);
                        let changedtick = buf_get_changedtick(&*buf);
                        if buf.b_last_changedtick + 1 == changedtick {
                            buf.b_last_changedtick = changedtick;
                        }
                        u_unchanged(&mut *buf);
                        u_update_save_nr(&mut *buf);
                    }

                    if overwriting {
                        ml_timestamp(&mut *buf);
                        if append {
                            buf.b_flags &= !BF_NEW;
                        } else {
                            buf.b_flags &= !BF_WRITE_MASK;
                        }
                    }

                    if !crate::memory::cstr_is_empty(opt_vars().p_pm) && dobackup {
                        let org = modname(fname, opt_vars().p_pm, false);
                        if let Some(ref bk) = backup {
                            match org {
                                None => {
                                    emsg(gettext(c"E205: Patchmode: can't save original file"));
                                }
                                Some(ref org) if !os_path_exists(org.as_ptr()) => {
                                    let _ = vim_rename(bk.as_ptr(), org.as_ptr());
                                    backup = None;
                                    #[cfg(not(windows))]
                                    {
                                        let _ = os_file_settime(
                                            org.as_ptr(),
                                            file_info_old.stat.st_atim.tv_sec as f64,
                                            file_info_old.stat.st_mtim.tv_sec as f64,
                                        );
                                    }
                                }
                                Some(_) => {}
                            }
                        } else {
                            let empty_fd = match org {
                                None => -1,
                                Some(ref org) => os_open(
                                    org.as_ptr(),
                                    O_CREAT | O_EXCL | O_NOFOLLOW,
                                    if perm < 0 { 0o666 } else { perm & 0o777 },
                                ),
                            };
                            if empty_fd < 0 {
                                emsg(gettext(c"E206: Patchmode: can't touch empty original file"));
                            } else {
                                os_close(empty_fd);
                            }
                        }
                        if let Some(ref org) = org {
                            let _ = os_setperm(org.as_ptr(), os_getperm(fname) & 0o777);
                        }
                    }

                    if opt_vars().p_bk == 0
                        && let Some(ref bk) = backup
                        && write_info.bw_conv_error == 0
                        && os_remove(bk.as_ptr()) != 0
                    {
                        emsg(gettext(c"E207: Can't delete backup file"));
                    }

                    break 'nofail;
                } // 'restore_backup

                if let Some(ref bk) = backup
                    && wfname == fname
                {
                    if backup_copy {
                        if !os_path_exists(fname) {
                            let _ = vim_rename(bk.as_ptr(), fname);
                        }
                        if os_path_exists(fname) {
                            let _ = os_remove(bk.as_ptr());
                        }
                    } else {
                        let _ = vim_rename(bk.as_ptr(), fname);
                    }
                }
                if !newfile && !os_path_exists(fname) {
                    end = 0;
                }
                drop(wfname_owned);
            } // 'fail

            g_st().no_wait_return -= 1;
        } // 'nofail

        if show_msg {
            ui_busy_stop();
        }

        drop(backup);
        drop(fenc_tofree);
        if write_info.bw_iconv_fd != -1 {
            let _ = call_iconv_close(write_info.bw_iconv_fd);
        }
        os_free_acl(acl);

        if err.msg.is_some() {
            add_quoted_fname(g_st().iobuff.as_mut_ptr(), IOSIZE - 100, buf, msgname);
            emit_err(
                &mut err,
                if progress >= 0 {
                    msg_id.as_ptr().cast()
                } else {
                    ptr::null()
                },
            );

            retval = FAIL;
            if end == 0 {
                msg_puts_hl(
                    gettext(c"\nWARNING: Original file may be lost or damaged\n"),
                    HLF_E,
                    true,
                );
                msg_puts_hl(
                    gettext(c"don't quit the editor until the file is successfully written!"),
                    HLF_E,
                    true,
                );

                if os_fileinfo(fname, &mut file_info_old) {
                    buf_store_file_info(&mut *buf, &mut file_info_old);
                    buf.b_mtime_read = buf.b_mtime;
                    buf.b_mtime_read_ns = buf.b_mtime_ns;
                }
            }
        }

        buf.b_saving = false;
        g_st().msg_scroll = msg_save;

        if retval == OK && write_undo_file {
            let mut hash = [0u8; UNDO_HASH_SIZE];
            sha_ctx.finish_into(&mut hash);
            u_write_undo(ptr::null(), false, &mut *buf, hash.as_mut_ptr());
        }

        if !should_abort(retval) {
            buf_write_do_post_autocmds(
                buf,
                fname,
                (eap.is_some()).then(|| eap.unwrap()),
                append,
                filtering,
                reset_changed,
                whole,
            );
            if aborting() {
                retval = FAIL;
            }
        }

        g_st().got_int |= prev_got_int;

        drop(fname_copy);
        drop(sfname_copy);
        drop(ffname_copy);

        retval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ucs2bytes_and_make_bom_encode_utf8_and_utf16() {
        let mut buf = [0u8; 8];
        assert_eq!(make_bom(buf.as_mut_ptr(), c"utf-8"), 3);
        assert_eq!([buf[0], buf[1], buf[2]], [0xef, 0xbb, 0xbf]);

        let mut p = buf.as_mut_ptr();
        assert!(!ucs2bytes(0xfeff, &mut p, FIO_UCS2 | FIO_ENDIAN_L));
        assert_eq!([buf[0], buf[1]], [0xff, 0xfe]);
    }
}
