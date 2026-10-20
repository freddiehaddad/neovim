//! Functions for creating a session file (`:mkexrc`, `:mkvimrc`, `:mkview`, `:mksession`) (`src/nvim/ex_session.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::autocmd::apply_autocmds;
#[allow(unused_imports)]
use crate::buffer::buflist_findnr;
#[allow(unused_imports)]
use crate::buffer::{bt_help, bt_nofilename, bt_normal, bt_terminal};
#[allow(unused_imports)]
use crate::charset::ascii_isdigit;
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::typval::tv_get_string;
#[allow(unused_imports)]
use crate::eval::var_flavour;
#[allow(unused_imports)]
use crate::eval::vars::get_globvar_dict;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_string;
#[allow(unused_imports)]
use crate::ex_docmd::{open_exfile, vim_mkdir_emsg};
#[allow(unused_imports)]
use crate::ex_getln::vim_strsave_fnameescape;
#[allow(unused_imports)]
use crate::file_search::vim_chdirfile;
#[allow(unused_imports)]
use crate::fileio::shorten_fnames;
#[allow(unused_imports)]
use crate::fileio::{fclose, fputc, fputs, fwrite};
#[allow(unused_imports)]
use crate::fold::put_folds;
use crate::globals::*;
#[allow(unused_imports)]
use crate::mapping::makemap;
#[allow(unused_imports)]
use crate::mbyte::utfc_ptr2len;
#[allow(unused_imports)]
use crate::message::emsg;
use crate::message::semsg;
#[allow(unused_imports)]
use crate::option::{makefoldset, makeset};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::os::env::home_replace_save;
#[allow(unused_imports)]
use crate::os::fs::{os_chdir, os_dirname, os_isdir};
#[allow(unused_imports)]
use crate::path::{add_pathsep, vim_FullName, vim_ispathsep};
#[allow(unused_imports)]
use crate::runtime::do_source;
#[allow(unused_imports)]
use crate::strings::gettext;
#[allow(unused_imports)]
use crate::strings::vim_strsave_escaped;
#[allow(unused_imports)]
use crate::window::tabpage_index;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use crate::api::command::exarg_T;
use crate::api::tabpage::{frame_T, tabpage_T, win_T};
use crate::arglist::{aentry_T, alist_T, alist_name};
use crate::clipboard::VarType;
use crate::eval::decode::dictitem_T;
use crate::eval::deprecated::{buf_T, fmark_T};
use crate::garray::garray_T;
use crate::hashtab::hashitem_empty;
use crate::memory::{xfree, xmalloc, xmemcpyz};

pub use crate::fileio::FILE;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const MAXCOL: c_int = 0x7fffffff;
const MAXPATHL: usize = 4096;

const FR_LEAF: c_char = 0;
const FR_COL: c_char = 2;

const VSE_NONE: c_int = 0;
const OPT_GLOBAL: c_int = 0x01;
const OPT_LOCAL: c_int = 0x02;
const OPT_SKIPRTP: c_int = 0x80;
const VAR_FLAVOUR_SESSION: c_int = 2;
const DOSO_NONE: c_int = 0;
const kCdCauseOther: c_int = -1;
const VV_THIS_SESSION: c_int = 7;

const CMD_mksession: c_int = 287;
const CMD_mkvimrc: c_int = 289;
const CMD_mkview: c_int = 290;

const EVENT_SESSIONWRITEPOST: c_int = 100;
const EVENT_SESSIONWRITEPRE: c_int = 101;

const kOptSsopFlagBuffers: c_uint = 0x01;
const kOptSsopFlagResize: c_uint = 0x04;
const kOptSsopFlagWinsize: c_uint = 0x08;
const kOptSsopFlagLocaloptions: c_uint = 0x10;
const kOptSsopFlagOptions: c_uint = 0x20;
const kOptSsopFlagHelp: c_uint = 0x40;
const kOptSsopFlagBlank: c_uint = 0x80;
const kOptSsopFlagGlobals: c_uint = 0x100;
const kOptSsopFlagSesdir: c_uint = 0x800;
const kOptSsopFlagCurdir: c_uint = 0x1000;
const kOptSsopFlagFolds: c_uint = 0x2000;
const kOptSsopFlagCursor: c_uint = 0x4000;
const kOptSsopFlagTabpages: c_uint = 0x8000;
const kOptSsopFlagTerminal: c_uint = 0x10000;
const kOptSsopFlagSkiprtp: c_uint = 0x20000;

#[repr(C)]
pub struct wininfo_Prefix {
    pub wi_win: *mut win_T,
    pub wi_mark: fmark_T,
}

static mut did_lcd: bool = false;

#[inline]
unsafe fn cstr_to_str<'a>(p: *const c_char) -> &'a str {
    unsafe { std::str::from_utf8_unchecked(CStr::from_ptr(p).to_bytes()) }
}

#[inline]
unsafe fn write_bytes(fd: *mut FILE, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    unsafe { fwrite(bytes.as_ptr().cast(), 1, bytes.len(), fd) == bytes.len() }
}

#[inline]
unsafe fn write_fmt(fd: *mut FILE, s: String) -> bool {
    unsafe { write_bytes(fd, s.as_bytes()) }
}

unsafe fn put_view_curpos(fd: *mut FILE, wp: *const win_T, spaces: &str) -> c_int {
    unsafe {
        let s = if (*wp).w_curswant == MAXCOL {
            format!("{spaces}normal! $\n")
        } else {
            format!("{spaces}normal! 0{}|\n", (*wp).w_virtcol + 1)
        };
        c_int::from(write_fmt(fd, s))
    }
}

unsafe fn ses_winsizes(fd: *mut FILE, restore_size: bool, tab_firstwin: *mut win_T) -> c_int {
    unsafe {
        if restore_size && (ssop_flags & kOptSsopFlagWinsize) != 0 {
            let mut n: c_int = 0;
            let mut wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(wp) == 0 {
                    wp = (*wp).w_next;
                    continue;
                }
                n += 1;

                if (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height
                    < (*topframe).fr_height
                    && !write_fmt(
                        fd,
                        format!(
                            "exe '{n}resize ' . ((&lines * {} + {}) / {})\n",
                            i64::from((*wp).w_height),
                            i64::from(Rows) / 2,
                            i64::from(Rows)
                        ),
                    )
                {
                    return FAIL;
                }

                if (*wp).w_width < Columns
                    && !write_fmt(
                        fd,
                        format!(
                            "exe 'vert {n}resize ' . ((&columns * {} + {}) / {})\n",
                            i64::from((*wp).w_width),
                            i64::from(Columns) / 2,
                            i64::from(Columns)
                        ),
                    )
                {
                    return FAIL;
                }
                wp = (*wp).w_next;
            }
        } else if put_line(fd, c"wincmd =".as_ptr().cast_mut()) == FAIL {
            return FAIL;
        }
        OK
    }
}

unsafe fn ses_win_rec(fd: *mut FILE, fr: *mut frame_T) -> c_int {
    unsafe {
        let mut count: c_int = 0;

        if (*fr).fr_layout == FR_LEAF {
            return OK;
        }

        let mut frc = ses_skipframe((*fr).fr_child);
        if !frc.is_null() {
            loop {
                frc = ses_skipframe((*frc).fr_next);
                if frc.is_null() {
                    break;
                }
                let split_cmd = if (*fr).fr_layout == FR_COL {
                    b"wincmd _ | wincmd |\nsplit\n" as &[u8]
                } else {
                    b"wincmd _ | wincmd |\nvsplit\n" as &[u8]
                };
                if !write_bytes(fd, split_cmd) {
                    return FAIL;
                }
                count += 1;
            }
        }

        if count > 0 {
            let back_cmd = if (*fr).fr_layout == FR_COL {
                format!("{count}wincmd k\n")
            } else {
                format!("{count}wincmd h\n")
            };
            if !write_fmt(fd, back_cmd) {
                return FAIL;
            }
        }

        frc = ses_skipframe((*fr).fr_child);
        while !frc.is_null() {
            ses_win_rec(fd, frc);
            frc = ses_skipframe((*frc).fr_next);
            if !frc.is_null() && put_line(fd, c"wincmd w".as_ptr().cast_mut()) == FAIL {
                return FAIL;
            }
        }

        OK
    }
}

unsafe fn ses_skipframe(fr: *mut frame_T) -> *mut frame_T {
    unsafe {
        let mut frc = fr;
        while !frc.is_null() {
            if ses_do_frame(frc) {
                break;
            }
            frc = (*frc).fr_next;
        }
        frc
    }
}

unsafe fn ses_do_frame(fr: *const frame_T) -> bool {
    unsafe {
        if (*fr).fr_layout == FR_LEAF {
            return ses_do_win((*fr).fr_win) != 0;
        }
        let mut frc: *const frame_T = (*fr).fr_child;
        while !frc.is_null() {
            if ses_do_frame(frc) {
                return true;
            }
            frc = (*frc).fr_next;
        }
        false
    }
}

unsafe fn ses_do_win(wp: *mut win_T) -> c_int {
    unsafe {
        if (*wp).w_floating {
            return 0;
        }
        let buf = (*wp).w_buffer;
        if (*buf).b_fname.is_null() || ((*buf).terminal.is_null() && bt_nofilename(buf)) {
            return (ssop_flags & kOptSsopFlagBlank) as c_int;
        }
        if bt_help(buf) {
            return (ssop_flags & kOptSsopFlagHelp) as c_int;
        }
        if bt_terminal(buf) {
            return (ssop_flags & kOptSsopFlagTerminal) as c_int;
        }
        1
    }
}

unsafe fn ses_arglist(
    fd: *mut FILE,
    cmd: *const c_char,
    gap: *mut garray_T,
    fullname: bool,
    flagp: *mut c_uint,
) -> c_int {
    unsafe {
        let mut buf: *mut c_char = ptr::null_mut();

        if !write_fmt(fd, format!("{}\n%argdel\n", cstr_to_str(cmd))) {
            return FAIL;
        }
        for i in 0..(*gap).ga_len {
            let entry = (*gap).ga_data.cast::<aentry_T>().add(i as usize);
            let mut s = alist_name(entry);
            if !s.is_null() {
                if fullname {
                    buf = xmalloc(MAXPATHL).cast();
                    vim_FullName(s, buf, MAXPATHL, false);
                    s = buf;
                }
                let fname_esc = ses_escape_fname(s, flagp);
                if !write_fmt(fd, format!("$argadd {}\n", cstr_to_str(fname_esc))) {
                    xfree(fname_esc.cast());
                    xfree(buf.cast());
                    return FAIL;
                }
                xfree(fname_esc.cast());
                xfree(buf.cast());
                buf = ptr::null_mut();
            }
        }
        OK
    }
}

unsafe fn ses_get_fname(buf: *mut buf_T, flagp: *const c_uint) -> *mut c_char {
    unsafe {
        if !(*buf).b_sfname.is_null()
            && ptr::eq(flagp, &raw const ssop_flags)
            && (ssop_flags & (kOptSsopFlagCurdir | kOptSsopFlagSesdir)) != 0
            && p_acd == 0
            && !did_lcd
        {
            return (*buf).b_sfname;
        }
        (*buf).b_ffname
    }
}

unsafe fn ses_fname(fd: *mut FILE, buf: *mut buf_T, flagp: *mut c_uint, add_eol: bool) -> c_int {
    unsafe {
        let name = ses_get_fname(buf, flagp);
        if ses_put_fname(fd, name, flagp) == FAIL || (add_eol && !write_bytes(fd, b"\n")) {
            return FAIL;
        }
        OK
    }
}

unsafe fn ses_escape_fname(name: *mut c_char, _flagp: *mut c_uint) -> *mut c_char {
    unsafe {
        let sname = home_replace_save(ptr::null_mut(), name);

        let mut p = sname;
        while *p != NUL {
            if *p == b'\\' as c_char {
                *p = b'/' as c_char;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        let res = vim_strsave_fnameescape(sname, VSE_NONE);
        xfree(sname.cast());
        res
    }
}

unsafe fn ses_put_fname(fd: *mut FILE, name: *mut c_char, flagp: *mut c_uint) -> c_int {
    unsafe {
        let p = ses_escape_fname(name, flagp);
        let retval = if fputs(p, fd) < 0 { FAIL } else { OK };
        xfree(p.cast());
        retval
    }
}

unsafe fn put_view(
    fd: *mut FILE,
    wp: *mut win_T,
    tp: *mut tabpage_T,
    add_edit: bool,
    flagp: *mut c_uint,
    current_arg_idx: c_int,
) -> c_int {
    unsafe {
        let mut did_next = false;
        let mut do_cursor =
            ptr::eq(flagp, &raw mut ssop_flags) || (*flagp & kOptSsopFlagCursor) != 0;

        if ptr::eq((*wp).w_alist.cast::<alist_T>(), &raw const global_alist) {
            if put_line(fd, c"argglobal".as_ptr().cast_mut()) == FAIL {
                return FAIL;
            }
        } else {
            let alist = (*wp).w_alist.cast::<alist_T>();
            if ses_arglist(
                fd,
                c"arglocal".as_ptr(),
                &raw mut (*alist).al_ga,
                ptr::eq(flagp, &raw mut vop_flags)
                    || (*flagp & kOptSsopFlagCurdir) == 0
                    || !(*tp).tp_localdir.is_null()
                    || !(*wp).w_localdir.is_null(),
                flagp,
            ) == FAIL
            {
                return FAIL;
            }
        }

        let wargcount = (*(*wp).w_alist.cast::<alist_T>()).al_ga.ga_len;
        if (*wp).w_arg_idx != current_arg_idx
            && (*wp).w_arg_idx < wargcount
            && ptr::eq(flagp, &raw mut ssop_flags)
        {
            if !write_fmt(fd, format!("{}argu\n", i64::from((*wp).w_arg_idx) + 1)) {
                return FAIL;
            }
            did_next = true;
        }

        if add_edit && (!did_next || (*wp).w_arg_idx_invalid) {
            let fname_esc = ses_escape_fname(ses_get_fname((*wp).w_buffer, flagp), flagp);
            if bt_help((*wp).w_buffer) {
                let mut curtag: *const c_char = c"".as_ptr();
                if 0 < (*wp).w_tagstackidx && (*wp).w_tagstackidx <= (*wp).w_tagstacklen {
                    curtag = (*wp).w_tagstack[((*wp).w_tagstackidx - 1) as usize].tagname;
                }

                if put_line(fd, c"enew | setl bt=help".as_ptr().cast_mut()) == FAIL
                    || !write_fmt(fd, format!("help {}", cstr_to_str(curtag)))
                    || put_eol(fd) == FAIL
                {
                    xfree(fname_esc.cast());
                    return FAIL;
                }
            } else if !(*(*wp).w_buffer).b_ffname.is_null()
                && (!bt_nofilename((*wp).w_buffer) || !(*(*wp).w_buffer).terminal.is_null())
            {
                let fesc = cstr_to_str(fname_esc);
                if !write_fmt(
                    fd,
                    format!(
                        "if bufexists(fnamemodify(\"{fesc}\", \":p\")) | buffer {fesc} | else | edit {fesc} | endif\nif &buftype ==# 'terminal'\n  silent file {fesc}\nendif\n"
                    ),
                ) {
                    xfree(fname_esc.cast());
                    return FAIL;
                }
            } else {
                if put_line(fd, c"enew".as_ptr().cast_mut()) == FAIL {
                    xfree(fname_esc.cast());
                    return FAIL;
                }
                if !(*(*wp).w_buffer).b_ffname.is_null()
                    && !write_fmt(fd, format!("file {}\n", cstr_to_str(fname_esc)))
                {
                    xfree(fname_esc.cast());
                    return FAIL;
                }
                do_cursor = false;
            }
            xfree(fname_esc.cast());
        }

        if (*wp).w_alt_fnum != 0 {
            let alt = buflist_findnr((*wp).w_alt_fnum);
            if ptr::eq(flagp, &raw mut ssop_flags)
                && !alt.is_null()
                && !(*alt).b_fname.is_null()
                && *(*alt).b_fname != NUL
                && (*alt).b_p_bl != 0
                && !(bt_terminal(alt) && (ssop_flags & kOptSsopFlagTerminal) == 0)
                && (fputs(c"balt ".as_ptr(), fd) < 0 || ses_fname(fd, alt, flagp, true) == FAIL)
            {
                return FAIL;
            }
        }

        if (*flagp & (kOptSsopFlagOptions | kOptSsopFlagLocaloptions)) != 0
            && makemap(fd, (*wp).w_buffer) == FAIL
        {
            return FAIL;
        }

        let save_curwin = curwin;
        curwin = wp;
        curbuf = (*curwin).w_buffer;
        let f = if (*flagp & (kOptSsopFlagOptions | kOptSsopFlagLocaloptions)) != 0 {
            makeset(
                fd,
                OPT_LOCAL,
                c_int::from(
                    ptr::eq(flagp, &raw mut vop_flags) || (*flagp & kOptSsopFlagOptions) == 0,
                ),
            )
        } else if (*flagp & kOptSsopFlagFolds) != 0 {
            makefoldset(fd)
        } else {
            OK
        };
        curwin = save_curwin;
        curbuf = (*curwin).w_buffer;
        if f == FAIL {
            return FAIL;
        }

        if (*flagp & kOptSsopFlagFolds) != 0
            && !(*(*wp).w_buffer).b_ffname.is_null()
            && (bt_normal((*wp).w_buffer) || bt_help((*wp).w_buffer))
            && put_folds(fd, wp) == FAIL
        {
            return FAIL;
        }

        if do_cursor {
            if (*wp).w_view_height <= 0 {
                if !write_fmt(fd, format!("let s:l = {}\n", (*wp).w_cursor.lnum)) {
                    return FAIL;
                }
            } else if !write_fmt(
                fd,
                format!(
                    "let s:l = {} - (({} * winheight(0) + {}) / {})\n",
                    (*wp).w_cursor.lnum,
                    (*wp).w_cursor.lnum - (*wp).w_topline,
                    (*wp).w_view_height / 2,
                    (*wp).w_view_height
                ),
            ) {
                return FAIL;
            }
            if !write_fmt(
                fd,
                format!(
                    "if s:l < 1 | let s:l = 1 | endif\nkeepjumps exe s:l\nnormal! zt\nkeepjumps {}\n",
                    (*wp).w_cursor.lnum
                ),
            ) {
                return FAIL;
            }
            if (*wp).w_cursor.col == 0 {
                if put_line(fd, c"normal! 0".as_ptr().cast_mut()) == FAIL {
                    return FAIL;
                }
            } else if (*wp).w_onebuf_opt.wo_wrap == 0 && (*wp).w_leftcol > 0 && (*wp).w_width > 0 {
                let vcol1 = i64::from((*wp).w_virtcol) + 1;
                if !write_fmt(
                    fd,
                    format!(
                        "let s:c = {vcol1} - (({} * winwidth(0) + {}) / {})\nif s:c > 0\n  exe 'normal! ' . s:c . '|zs' . {vcol1} . '|'\nelse\n",
                        i64::from((*wp).w_virtcol - (*wp).w_leftcol),
                        i64::from((*wp).w_width / 2),
                        i64::from((*wp).w_width),
                    ),
                ) || put_view_curpos(fd, wp, "  ") == FAIL
                    || put_line(fd, c"endif".as_ptr().cast_mut()) == FAIL
                {
                    return FAIL;
                }
            } else if put_view_curpos(fd, wp, "") == FAIL {
                return FAIL;
            }
        }

        if !(*wp).w_localdir.is_null()
            && (!ptr::eq(flagp, &raw mut vop_flags) || (*flagp & kOptSsopFlagCurdir) != 0)
        {
            if fputs(c"lcd ".as_ptr(), fd) < 0
                || ses_put_fname(fd, (*wp).w_localdir, flagp) == FAIL
                || !write_bytes(fd, b"\n")
            {
                return FAIL;
            }
            did_lcd = true;
        }

        OK
    }
}

unsafe fn store_session_globals(fd: *mut FILE) -> c_int {
    unsafe {
        let d = get_globvar_dict();
        let ht = &raw mut (*d).dv_hashtab;
        let mut todo = (*ht).ht_used;
        let mut hi = (*ht).ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let this_var = ((*hi).hi_key as *mut u8)
                    .sub(std::mem::offset_of!(dictitem_T, di_key))
                    .cast::<dictitem_T>();
                let key_ptr = (*this_var).di_key.as_mut_ptr();
                if ((*this_var).di_tv.v_type == VarType::VAR_NUMBER
                    || (*this_var).di_tv.v_type == VarType::VAR_STRING)
                    && var_flavour(key_ptr) == VAR_FLAVOUR_SESSION
                {
                    let p = vim_strsave_escaped(
                        tv_get_string(&raw const (*this_var).di_tv),
                        c"\\\"\n\r".as_ptr(),
                    );
                    let mut t = p;
                    while *t != NUL {
                        if *t == b'\n' as c_char {
                            *t = b'n' as c_char;
                        } else if *t == b'\r' as c_char {
                            *t = b'r' as c_char;
                        }
                        t = t.add(1);
                    }
                    let quote = if (*this_var).di_tv.v_type == VarType::VAR_STRING {
                        '"'
                    } else {
                        ' '
                    };
                    if !write_fmt(
                        fd,
                        format!(
                            "let {} = {quote}{}{quote}",
                            cstr_to_str(key_ptr),
                            cstr_to_str(p)
                        ),
                    ) || put_eol(fd) == FAIL
                    {
                        xfree(p.cast());
                        return FAIL;
                    }
                    xfree(p.cast());
                } else if (*this_var).di_tv.v_type == VarType::VAR_FLOAT
                    && var_flavour(key_ptr) == VAR_FLAVOUR_SESSION
                {
                    let mut f = (*this_var).di_tv.vval.v_float;
                    let sign = if f < 0.0 {
                        f = -f;
                        '-'
                    } else {
                        ' '
                    };
                    if !write_fmt(fd, format!("let {} = {sign}{f:.6}", cstr_to_str(key_ptr)))
                        || put_eol(fd) == FAIL
                    {
                        return FAIL;
                    }
                }
            }
            hi = hi.add(1);
        }
        OK
    }
}

unsafe fn makeopens(fd: *mut FILE, dirnow: *mut c_char) -> c_int {
    unsafe {
        let mut only_save_windows = true;
        let mut edited_win: *mut win_T = ptr::null_mut();
        let mut cur_arg_idx: c_int = 0;
        let mut next_arg_idx: c_int = 0;

        if (ssop_flags & kOptSsopFlagBuffers) != 0 {
            only_save_windows = false;
        }

        if put_line(
            fd,
            c"let v:this_session=expand(\"<sfile>:p\")"
                .as_ptr()
                .cast_mut(),
        ) == FAIL
        {
            return FAIL;
        }
        if put_line(fd, c"doautoall SessionLoadPre".as_ptr().cast_mut()) == FAIL {
            return FAIL;
        }

        if (ssop_flags & kOptSsopFlagGlobals) != 0 && store_session_globals(fd) == FAIL {
            return FAIL;
        }

        if put_line(fd, c"silent only".as_ptr().cast_mut()) == FAIL {
            return FAIL;
        }
        if (ssop_flags & kOptSsopFlagTabpages) != 0
            && put_line(fd, c"silent tabonly".as_ptr().cast_mut()) == FAIL
        {
            return FAIL;
        }

        if (ssop_flags & kOptSsopFlagSesdir) != 0 {
            if put_line(
                fd,
                c"exe \"cd \" . escape(expand(\"<sfile>:p:h\"), ' ')"
                    .as_ptr()
                    .cast_mut(),
            ) == FAIL
            {
                return FAIL;
            }
        } else if (ssop_flags & kOptSsopFlagCurdir) != 0 {
            let sname = home_replace_save(
                ptr::null_mut(),
                if !globaldir.is_null() {
                    globaldir
                } else {
                    dirnow
                },
            );
            let fname_esc = ses_escape_fname(sname, &raw mut ssop_flags);
            if !write_fmt(fd, format!("cd {}\n", cstr_to_str(fname_esc))) {
                xfree(fname_esc.cast());
                xfree(sname.cast());
                return FAIL;
            }
            xfree(fname_esc.cast());
            xfree(sname.cast());
        }

        if !write_bytes(
            fd,
            b"if expand('%') == '' && !&modified && line('$') <= 1 && getline(1) == ''\n  let s:wipebuf = bufnr('%')\nendif\n",
        ) {
            return FAIL;
        }

        if (ssop_flags & kOptSsopFlagOptions) == 0
            && put_line(fd, c"let s:shortmess_save = &shortmess".as_ptr().cast_mut()) == FAIL
        {
            return FAIL;
        }

        if put_line(fd, c"set shortmess+=aoO".as_ptr().cast_mut()) == FAIL {
            return FAIL;
        }

        let mut buf = firstbuf;
        while !buf.is_null() {
            if !(only_save_windows && (*buf).b_nwindows == 0)
                && !((*buf).b_help && (ssop_flags & kOptSsopFlagHelp) == 0)
                && !(bt_terminal(buf) && (ssop_flags & kOptSsopFlagTerminal) == 0)
                && !(*buf).b_fname.is_null()
                && (*buf).b_p_bl != 0
            {
                let lnum = if (*buf).b_wininfo.size == 0 {
                    1i64
                } else {
                    let wi = (*(*buf).b_wininfo.items).cast::<wininfo_Prefix>();
                    i64::from((*wi).wi_mark.mark.lnum)
                };
                if !write_fmt(fd, format!("badd +{lnum} "))
                    || ses_fname(fd, buf, &raw mut ssop_flags, true) == FAIL
                {
                    return FAIL;
                }
            }
            buf = (*buf).b_next;
        }

        if ses_arglist(
            fd,
            c"argglobal".as_ptr(),
            &raw mut global_alist.al_ga,
            (ssop_flags & kOptSsopFlagCurdir) == 0,
            &raw mut ssop_flags,
        ) == FAIL
        {
            return FAIL;
        }

        if (ssop_flags & kOptSsopFlagCurdir) != 0 {
            let mut b = firstbuf;
            while !b.is_null() {
                if (*b).b_localdir.is_null()
                    || (*b).b_fname.is_null()
                    || (*b).b_p_bl == 0
                    || (only_save_windows && (*b).b_nwindows == 0)
                    || ((*b).b_help && (ssop_flags & kOptSsopFlagHelp) == 0)
                    || (bt_terminal(b) && (ssop_flags & kOptSsopFlagTerminal) == 0)
                {
                    b = (*b).b_next;
                    continue;
                }
                if fputs(
                    c"lua vim.api.nvim_create_autocmd('BufEnter', { once = true, buffer = vim.fn.bufadd([==["
                        .as_ptr(),
                    fd,
                ) < 0
                    || fputs(ses_get_fname(b, &raw const ssop_flags), fd) < 0
                    || fputs(
                        c"]==]), callback = function() vim.cmd.bcd({ [==[".as_ptr(),
                        fd,
                    ) < 0
                    || fputs((*b).b_localdir, fd) < 0
                    || fputs(
                        c"]==], magic = { file = false, bar = false } }) end })\n".as_ptr(),
                        fd,
                    ) < 0
                {
                    return FAIL;
                }
                did_lcd = true;
                b = (*b).b_next;
            }
        }

        if (ssop_flags & kOptSsopFlagResize) != 0
            && !write_fmt(
                fd,
                format!(
                    "set lines={} columns={}\n",
                    i64::from(Rows),
                    i64::from(Columns)
                ),
            )
        {
            return FAIL;
        }

        let mut restore_stal = false;
        if p_stal == 1 && !(*first_tabpage).tp_next.is_null() {
            if put_line(fd, c"set stal=2".as_ptr().cast_mut()) == FAIL {
                return FAIL;
            }
            restore_stal = true;
        }

        if (ssop_flags & kOptSsopFlagTabpages) != 0 {
            let mut tp = first_tabpage;
            while !tp.is_null() {
                if !(*tp).tp_next.is_null()
                    && put_line(fd, c"tabnew +setlocal\\ bufhidden=wipe".as_ptr().cast_mut())
                        == FAIL
                {
                    return FAIL;
                }
                tp = (*tp).tp_next;
            }

            if !(*first_tabpage).tp_next.is_null()
                && put_line(fd, c"tabrewind".as_ptr().cast_mut()) == FAIL
            {
                return FAIL;
            }
        }

        let mut restore_height_width = false;
        let mut tp_iter = first_tabpage;
        while !tp_iter.is_null() {
            let mut tp = tp_iter;
            let mut need_tabnext = false;
            let mut cnr: c_int = 1;
            let tab_firstwin: *mut win_T;
            let tab_topframe: *mut frame_T;

            if (ssop_flags & kOptSsopFlagTabpages) != 0 {
                if ptr::eq(tp, curtab) {
                    tab_firstwin = firstwin;
                    tab_topframe = topframe;
                } else {
                    tab_firstwin = (*tp).tp_firstwin;
                    tab_topframe = (*tp).tp_topframe.cast::<frame_T>();
                }
                if !ptr::eq(tp, first_tabpage) {
                    need_tabnext = true;
                }
            } else {
                tp = curtab;
                tab_firstwin = firstwin;
                tab_topframe = topframe;
            }

            if (ssop_flags & kOptSsopFlagCurdir) != 0 && !(*tp).tp_localdir.is_null() {
                if need_tabnext && put_line(fd, c"tabnext".as_ptr().cast_mut()) == FAIL {
                    return FAIL;
                }
                need_tabnext = false;
                if fputs(c"tcd ".as_ptr(), fd) < 0
                    || ses_put_fname(fd, (*tp).tp_localdir, &raw mut ssop_flags) == FAIL
                    || put_eol(fd) == FAIL
                {
                    return FAIL;
                }
                did_lcd = true;
            }

            let mut wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(wp) != 0
                    && !(*(*wp).w_buffer).b_ffname.is_null()
                    && !bt_help((*wp).w_buffer)
                    && !bt_nofilename((*wp).w_buffer)
                {
                    if need_tabnext && put_line(fd, c"tabnext".as_ptr().cast_mut()) == FAIL {
                        return FAIL;
                    }
                    need_tabnext = false;

                    if fputs(c"edit ".as_ptr(), fd) < 0
                        || ses_fname(fd, (*wp).w_buffer, &raw mut ssop_flags, true) == FAIL
                    {
                        return FAIL;
                    }
                    if !(*wp).w_arg_idx_invalid {
                        edited_win = wp;
                    }
                    break;
                }
                wp = (*wp).w_next;
            }

            if need_tabnext && put_line(fd, c"tabnext".as_ptr().cast_mut()) == FAIL {
                return FAIL;
            }

            if (*tab_topframe).fr_layout != FR_LEAF
                && (put_line(
                    fd,
                    c"let s:save_splitbelow = &splitbelow".as_ptr().cast_mut(),
                ) == FAIL
                    || put_line(
                        fd,
                        c"let s:save_splitright = &splitright".as_ptr().cast_mut(),
                    ) == FAIL
                    || put_line(fd, c"set splitbelow splitright".as_ptr().cast_mut()) == FAIL
                    || ses_win_rec(fd, tab_topframe) == FAIL
                    || put_line(
                        fd,
                        c"let &splitbelow = s:save_splitbelow".as_ptr().cast_mut(),
                    ) == FAIL
                    || put_line(
                        fd,
                        c"let &splitright = s:save_splitright".as_ptr().cast_mut(),
                    ) == FAIL)
            {
                return FAIL;
            }

            let mut nr: c_int = 0;
            let mut restore_size = true;
            wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(wp) != 0 {
                    nr += 1;
                } else if !(*wp).w_floating {
                    restore_size = false;
                }
                if ptr::eq(curwin, wp) {
                    cnr = nr;
                }
                wp = (*wp).w_next;
            }

            if !tab_firstwin.is_null() && !(*tab_firstwin).w_next.is_null() {
                if put_line(fd, c"wincmd t".as_ptr().cast_mut()) == FAIL {
                    return FAIL;
                }
                if !restore_height_width
                    && (put_line(
                        fd,
                        c"let s:save_winminheight = &winminheight"
                            .as_ptr()
                            .cast_mut(),
                    ) == FAIL
                        || put_line(
                            fd,
                            c"let s:save_winminwidth = &winminwidth".as_ptr().cast_mut(),
                        ) == FAIL)
                {
                    return FAIL;
                }
                if !write_bytes(
                    fd,
                    b"set winminheight=0\nset winheight=1\nset winminwidth=0\nset winwidth=1\n",
                ) {
                    return FAIL;
                }
                restore_height_width = true;
            }
            if nr > 1 && ses_winsizes(fd, restore_size, tab_firstwin) == FAIL {
                return FAIL;
            }

            wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(wp) == 0 {
                    wp = (*wp).w_next;
                    continue;
                }
                if put_view(
                    fd,
                    wp,
                    tp,
                    !ptr::eq(wp, edited_win),
                    &raw mut ssop_flags,
                    cur_arg_idx,
                ) == FAIL
                {
                    return FAIL;
                }
                if nr > 1 && put_line(fd, c"wincmd w".as_ptr().cast_mut()) == FAIL {
                    return FAIL;
                }
                next_arg_idx = (*wp).w_arg_idx;
                wp = (*wp).w_next;
            }

            cur_arg_idx = next_arg_idx;

            if cnr > 1 && !write_fmt(fd, format!("{cnr}wincmd w\n")) {
                return FAIL;
            }

            if nr > 1 && ses_winsizes(fd, restore_size, tab_firstwin) == FAIL {
                return FAIL;
            }

            if (ssop_flags & kOptSsopFlagTabpages) == 0 {
                break;
            }
            tp_iter = (*tp_iter).tp_next;
        }

        if (ssop_flags & kOptSsopFlagTabpages) != 0
            && !write_fmt(fd, format!("tabnext {}\n", tabpage_index(curtab)))
        {
            return FAIL;
        }
        if restore_stal && put_line(fd, c"set stal=1".as_ptr().cast_mut()) == FAIL {
            return FAIL;
        }

        if !write_bytes(
            fd,
            b"if exists('s:wipebuf') && len(win_findbuf(s:wipebuf)) == 0 && getbufvar(s:wipebuf, '&buftype') isnot# 'terminal'\n  silent exe 'bwipe ' . s:wipebuf\nendif\nunlet! s:wipebuf\n",
        ) {
            return FAIL;
        }

        let wh = p_wh;
        let wiw = p_wiw;
        if !write_fmt(fd, format!("set winheight={wh} winwidth={wiw}\n")) {
            return FAIL;
        }

        if (ssop_flags & kOptSsopFlagOptions) != 0 {
            if !write_fmt(fd, format!("set shortmess={}\n", cstr_to_str(p_shm))) {
                return FAIL;
            }
        } else if put_line(fd, c"let &shortmess = s:shortmess_save".as_ptr().cast_mut()) == FAIL {
            return FAIL;
        }

        if restore_height_width
            && (put_line(
                fd,
                c"let &winminheight = s:save_winminheight"
                    .as_ptr()
                    .cast_mut(),
            ) == FAIL
                || put_line(
                    fd,
                    c"let &winminwidth = s:save_winminwidth".as_ptr().cast_mut(),
                ) == FAIL)
        {
            return FAIL;
        }

        if !write_bytes(
            fd,
            b"let s:sx = expand(\"<sfile>:p:r\").\"x.vim\"\nif filereadable(s:sx)\n  exe \"source \" . fnameescape(s:sx)\nendif\n",
        ) {
            return FAIL;
        }

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_loadview(eap: *mut exarg_T) {
    unsafe {
        let fname = get_view_file(*(*eap).arg);
        if fname.is_null() {
            return;
        }

        if do_source(fname, false, DOSO_NONE, ptr::null_mut()) == FAIL {
            semsg!(gettext(e_notopen.as_ptr()), fname);
        }
        xfree(fname.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_mkrc(eap: *mut exarg_T) {
    unsafe {
        let mut view_session = false;
        let mut using_vdir = false;
        let mut view_file: *mut c_char = ptr::null_mut();

        if (*eap).cmdidx == CMD_mksession || (*eap).cmdidx == CMD_mkview {
            view_session = true;
        }

        did_lcd = false;

        let fname: *mut c_char;
        if (*eap).cmdidx == CMD_mkview
            && (*(*eap).arg == NUL
                || (ascii_isdigit(*(*eap).arg as c_int) && *(*eap).arg.add(1) == NUL))
        {
            (*eap).forceit = 1;
            fname = get_view_file(*(*eap).arg);
            if fname.is_null() {
                return;
            }
            view_file = fname;
            using_vdir = true;
        } else if *(*eap).arg != NUL {
            fname = (*eap).arg;
        } else if (*eap).cmdidx == CMD_mkvimrc {
            fname = c".nvimrc".as_ptr().cast_mut();
        } else if (*eap).cmdidx == CMD_mksession {
            fname = c"Session.vim".as_ptr().cast_mut();
        } else {
            fname = c".exrc".as_ptr().cast_mut();
        }

        if using_vdir && !os_isdir(p_vdir) {
            vim_mkdir_emsg(p_vdir, 0o755);
        }

        let fd = open_exfile(fname, (*eap).forceit, c"wb".as_ptr().cast_mut());
        if !fd.is_null() {
            let mut failed = false;
            let flagp = if (*eap).cmdidx == CMD_mkview {
                &raw mut vop_flags
            } else {
                &raw mut ssop_flags
            };

            if (*eap).cmdidx == CMD_mksession {
                apply_autocmds(
                    EVENT_SESSIONWRITEPRE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    curbuf,
                );
            }

            if (*eap).cmdidx == CMD_mkvimrc {
                put_line(fd, c"version 6.0".as_ptr().cast_mut());
            }

            if (*eap).cmdidx == CMD_mksession
                && put_line(fd, c"let SessionLoad = 1".as_ptr().cast_mut()) == FAIL
            {
                failed = true;
            }

            if !view_session
                || ((*eap).cmdidx == CMD_mksession && (*flagp & kOptSsopFlagOptions) != 0)
            {
                let mut flags = OPT_GLOBAL;
                if (*eap).cmdidx == CMD_mksession && (*flagp & kOptSsopFlagSkiprtp) != 0 {
                    flags |= OPT_SKIPRTP;
                }
                failed |= makemap(fd, ptr::null_mut()) == FAIL || makeset(fd, flags, 0) == FAIL;
            }

            if !failed && view_session {
                if put_line(
                    fd,
                    c"let s:so_save = &g:so | let s:siso_save = &g:siso | setg so=0 siso=0 | setl so=-1 siso=-1"
                        .as_ptr()
                        .cast_mut(),
                ) == FAIL
                {
                    failed = true;
                }
                if (*eap).cmdidx == CMD_mksession {
                    let dirnow: *mut c_char = xmalloc(MAXPATHL).cast();

                    if os_dirname(dirnow, MAXPATHL) == FAIL || os_chdir(dirnow) != 0 {
                        *dirnow = NUL;
                    }
                    if *dirnow != NUL && (ssop_flags & kOptSsopFlagSesdir) != 0 {
                        if vim_chdirfile(fname, kCdCauseOther) == OK {
                            shorten_fnames(1);
                        }
                    } else if *dirnow != NUL
                        && (ssop_flags & kOptSsopFlagCurdir) != 0
                        && !globaldir.is_null()
                        && os_chdir(globaldir) == 0
                    {
                        shorten_fnames(1);
                    }

                    failed |= makeopens(fd, dirnow) == FAIL;

                    if *dirnow != NUL
                        && ((ssop_flags & kOptSsopFlagSesdir) != 0
                            || ((ssop_flags & kOptSsopFlagCurdir) != 0 && !globaldir.is_null()))
                    {
                        if os_chdir(dirnow) != 0 {
                            emsg(gettext(e_prev_dir.as_ptr()));
                        }
                        shorten_fnames(1);
                    }
                    xfree(dirnow.cast());
                } else {
                    failed |= put_view(fd, curwin, curtab, !using_vdir, flagp, -1) == FAIL;
                }
                if !write_bytes(fd, b"let &g:so = s:so_save | let &g:siso = s:siso_save\n") {
                    failed = true;
                }
                if p_hls != 0 && !write_bytes(fd, b"set hlsearch\n") {
                    failed = true;
                }
                if Search.no_hlsearch && !write_bytes(fd, b"nohlsearch\n") {
                    failed = true;
                }
                if (*eap).cmdidx == CMD_mksession
                    && !write_bytes(fd, b"doautoall SessionLoadPost\nunlet SessionLoad\n")
                {
                    failed = true;
                }
            }
            if put_line(fd, c"\" vim: set ft=vim :".as_ptr().cast_mut()) == FAIL {
                failed = true;
            }

            failed |= fclose(fd) != 0;

            if failed {
                emsg(gettext(e_write.as_ptr()));
            } else if (*eap).cmdidx == CMD_mksession {
                let tbuf: *mut c_char = xmalloc(MAXPATHL).cast();
                if vim_FullName(fname, tbuf, MAXPATHL, false) == OK {
                    set_vim_var_string(VV_THIS_SESSION, tbuf, -1);
                }
                xfree(tbuf.cast());
            }
        }

        xfree(view_file.cast());

        if (*eap).cmdidx == CMD_mksession {
            apply_autocmds(
                EVENT_SESSIONWRITEPOST,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
    }
}

unsafe fn get_view_file(c: c_char) -> *mut c_char {
    unsafe {
        if (*curbuf).b_ffname.is_null() {
            emsg(gettext(e_noname.as_ptr()));
            return ptr::null_mut();
        }
        let sname = home_replace_save(ptr::null_mut(), (*curbuf).b_ffname);

        let mut len: usize = 0;
        let mut p = sname;
        while *p != NUL {
            if *p == b'=' as c_char || vim_ispathsep(c_int::from(*p as u8)) {
                len += 1;
            }
            p = p.add(1);
        }
        let sname_len = CStr::from_ptr(sname).to_bytes().len();
        let vdir_len = CStr::from_ptr(p_vdir).to_bytes().len();
        let retval: *mut c_char = xmalloc(sname_len + len + vdir_len + 9).cast();
        ptr::copy_nonoverlapping(p_vdir, retval, vdir_len + 1);
        add_pathsep(retval);
        let mut s = retval.add(CStr::from_ptr(retval).to_bytes().len());
        p = sname;
        while *p != NUL {
            if *p == b'=' as c_char {
                *s = b'=' as c_char;
                *s.add(1) = b'=' as c_char;
                s = s.add(2);
            } else if vim_ispathsep(c_int::from(*p as u8)) {
                *s = b'=' as c_char;
                #[cfg(windows)]
                {
                    *s.add(1) = if *p == b':' as c_char {
                        b'-' as c_char
                    } else {
                        b'+' as c_char
                    };
                }
                #[cfg(not(windows))]
                {
                    *s.add(1) = b'+' as c_char;
                }
                s = s.add(2);
            } else {
                *s = *p;
                s = s.add(1);
            }
            p = p.add(1);
        }
        *s = b'=' as c_char;
        *s.add(1) = c;
        s = s.add(2);
        xmemcpyz(s.cast(), b".vim".as_ptr().cast(), 4);

        xfree(sname.cast());
        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn put_eol(fd: *mut FILE) -> c_int {
    unsafe {
        if fputc(b'\n' as c_int, fd) < 0 {
            return FAIL;
        }
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn put_line(fd: *mut FILE, s: *mut c_char) -> c_int {
    unsafe {
        if fputs(s, fd) < 0 || fputc(b'\n' as c_int, fd) < 0 {
            return FAIL;
        }
        OK
    }
}
