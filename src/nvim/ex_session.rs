//! Functions for creating a session file (`:mkexrc`, `:mkvimrc`, `:mkview`, `:mksession`) (`src/nvim/ex_session.c`).

use crate::arglist::{aentry_T, alist_name};
use crate::autocmd::{EVENT_SESSIONWRITEPOST, EVENT_SESSIONWRITEPRE, apply_autocmds};
use crate::buffer::{bt_help, bt_nofilename, bt_normal, bt_terminal, buf_T, buflist_findnr};
use crate::charset::{NUL, ascii_isdigit};
use crate::errors::*;
use crate::eval::typval::{VarType, tv_get_string};
use crate::eval::vars::{VV_THIS_SESSION, get_globvar_dict, set_vim_var_string};
use crate::eval::{VAR_FLAVOUR_SESSION, var_flavour};
use crate::ex_docmd::{exarg_T, open_exfile, vim_mkdir_emsg};
use crate::ex_getln::{VSE_NONE, vim_strsave_fnameescape};
use crate::file_search::vim_chdirfile;
use crate::fileio::{FILE, fclose, fputc, fputs, fwrite, shorten_fnames};
use crate::fold::put_folds;
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::mapping::makemap;
use crate::mbyte::utfc_ptr2len;
use crate::message::{emsg, semsg};
use crate::option::{OPT_GLOBAL, OPT_LOCAL, makefoldset, makeset};
use crate::option_vars::*;
use crate::os::env::home_replace_save;
use crate::os::fs::{MAXPATHL, os_chdir, os_dirname, os_isdir};
use crate::path::{add_pathsep, vim_FullName, vim_ispathsep};
use crate::runtime::{DOSO_NONE, do_source};
use crate::strings::{gettext, vim_strsave_escaped};
use crate::window::{frame_T, tabpage_T, tabpage_index, win_T};
use std::ffi::{CString, c_char, c_int, c_uint};
use std::ptr;

use crate::api::win_config::FR_COL;
use crate::drawscreen::FR_LEAF;
use crate::math::FAIL;
use crate::regexp::MAXCOL;

pub(crate) const OPT_SKIPRTP: c_int = 0x80;
pub(crate) const kCdCauseOther: c_int = -1;

const CMD_mksession: c_int = 287;
const CMD_mkvimrc: c_int = 289;
const CMD_mkview: c_int = 290;

const kOptSsopFlagBuffers: c_uint = 0x01;
const kOptSsopFlagResize: c_uint = 0x04;
const kOptSsopFlagWinsize: c_uint = 0x08;
const kOptSsopFlagLocaloptions: c_uint = 0x10;
const kOptSsopFlagOptions: c_uint = 0x20;
const kOptSsopFlagHelp: c_uint = 0x40;
const kOptSsopFlagBlank: c_uint = 0x80;
const kOptSsopFlagGlobals: c_uint = 0x100;
pub(crate) const kOptSsopFlagSesdir: c_uint = 0x800;
pub(crate) const kOptSsopFlagCurdir: c_uint = 0x1000;
const kOptSsopFlagFolds: c_uint = 0x2000;
const kOptSsopFlagCursor: c_uint = 0x4000;
const kOptSsopFlagTabpages: c_uint = 0x8000;
const kOptSsopFlagTerminal: c_uint = 0x10000;
const kOptSsopFlagSkiprtp: c_uint = 0x20000;

struct ExSessionState {
    did_lcd: bool,
}

impl ExSessionState {
    const fn new() -> Self {
        Self {
            did_lcd: false,
        }
    }
}

static EXSES_STATE: crate::globals::GlobalState<ExSessionState> = crate::globals::GlobalState::new(ExSessionState::new());

#[inline]
fn exses_st() -> &'static mut ExSessionState {
    unsafe { &mut *EXSES_STATE.as_ptr() }
}

#[inline]
fn cstr_to_str<'a>(p: *const c_char) -> &'a str {
    unsafe { std::str::from_utf8_unchecked(crate::memory::cstr_as_bytes(p)) }
}

#[inline]
fn write_bytes(fd: *mut FILE, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    unsafe { fwrite(bytes.as_ptr().cast(), 1, bytes.len(), fd) == bytes.len() }
}

#[inline]
fn write_fmt(fd: *mut FILE, s: String) -> bool {
    write_bytes(fd, s.as_bytes())
}

fn put_view_curpos(fd: *mut FILE, wp: *const win_T, spaces: &str) -> c_int {
    unsafe {
        let s = if (*wp).w_curswant == MAXCOL {
            format!("{spaces}normal! $\n")
        } else {
            format!("{spaces}normal! 0{}|\n", (*wp).w_virtcol + 1)
        };
        c_int::from(write_fmt(fd, s))
    }
}

fn ses_winsizes(fd: *mut FILE, restore_size: bool, tab_firstwin: *mut win_T) -> bool {
    unsafe {
        if restore_size && (opt_vars().ssop_flags & kOptSsopFlagWinsize) != 0 {
            let mut n: c_int = 0;
            let mut wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(&mut *wp) == 0 {
                    wp = (*wp).w_next;
                    continue;
                }
                n += 1;

                if (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height
                    < (*g_st().topframe).fr_height
                    && !write_fmt(
                        fd,
                        format!(
                            "exe '{n}resize ' . ((&lines * {} + {}) / {})\n",
                            i64::from((*wp).w_height),
                            i64::from(g_st().Rows) / 2,
                            i64::from(g_st().Rows)
                        ),
                    )
                {
                    return false;
                }

                if (*wp).w_width < g_st().Columns
                    && !write_fmt(
                        fd,
                        format!(
                            "exe 'vert {n}resize ' . ((&columns * {} + {}) / {})\n",
                            i64::from((*wp).w_width),
                            i64::from(g_st().Columns) / 2,
                            i64::from(g_st().Columns)
                        ),
                    )
                {
                    return false;
                }
                wp = (*wp).w_next;
            }
        } else if !put_line(fd, c"wincmd =") {
            return false;
        }
        true
    }
}

fn ses_win_rec(fd: *mut FILE, fr: &mut frame_T) -> bool {
    unsafe {
        let mut count: c_int = 0;

        if fr.fr_layout == FR_LEAF {
            return true;
        }

        let mut frc = ses_skipframe(fr.fr_child);
        if !frc.is_null() {
            loop {
                frc = ses_skipframe((*frc).fr_next);
                if frc.is_null() {
                    break;
                }
                let split_cmd = if fr.fr_layout == FR_COL {
                    b"wincmd _ | wincmd |\nsplit\n" as &[u8]
                } else {
                    b"wincmd _ | wincmd |\nvsplit\n" as &[u8]
                };
                if !write_bytes(fd, split_cmd) {
                    return false;
                }
                count += 1;
            }
        }

        if count > 0 {
            let back_cmd = if fr.fr_layout == FR_COL {
                format!("{count}wincmd k\n")
            } else {
                format!("{count}wincmd h\n")
            };
            if !write_fmt(fd, back_cmd) {
                return false;
            }
        }

        frc = ses_skipframe(fr.fr_child);
        while !frc.is_null() {
            ses_win_rec(fd, &mut *frc);
            frc = ses_skipframe((*frc).fr_next);
            if !frc.is_null() && !put_line(fd, c"wincmd w") {
                return false;
            }
        }

        true
    }
}

fn ses_skipframe(fr: *mut frame_T) -> *mut frame_T {
    unsafe {
        let mut frc = fr;
        while !frc.is_null() {
            if ses_do_frame(&*frc) {
                break;
            }
            frc = (*frc).fr_next;
        }
        frc
    }
}

fn ses_do_frame(fr: &frame_T) -> bool {
    unsafe {
        if fr.fr_layout == FR_LEAF {
            return ses_do_win(&mut *fr.fr_win) != 0;
        }
        let mut frc: *const frame_T = fr.fr_child;
        while !frc.is_null() {
            if ses_do_frame(&*frc) {
                return true;
            }
            frc = (*frc).fr_next;
        }
        false
    }
}

fn ses_do_win(wp: &mut win_T) -> c_int {
    unsafe {
        if wp.w_floating {
            return 0;
        }
        let buf = wp.w_buffer;
        if (*buf).b_fname.is_null() || ((*buf).terminal.is_null() && bt_nofilename(buf)) {
            return (opt_vars().ssop_flags & kOptSsopFlagBlank) as c_int;
        }
        if bt_help(buf) {
            return (opt_vars().ssop_flags & kOptSsopFlagHelp) as c_int;
        }
        if bt_terminal(buf) {
            return (opt_vars().ssop_flags & kOptSsopFlagTerminal) as c_int;
        }
        1
    }
}

fn ses_arglist(
    fd: *mut FILE,
    cmd: &std::ffi::CStr,
    gap: &[aentry_T],
    fullname: bool,
    _flagp: &mut c_uint,
) -> bool {
    if !write_fmt(fd, format!("{}\n%argdel\n", cstr_to_str(cmd.as_ptr()))) {
            return false;
        }
        let mut buf: Vec<c_char> = if fullname {
            vec![0; MAXPATHL]
        } else {
            Vec::new()
        };
        for entry in gap {
            let mut s = alist_name(entry);
            if !s.is_null() {
                if fullname {
                    vim_FullName(s, buf.as_mut_ptr(), MAXPATHL, false);
                    s = buf.as_mut_ptr();
                }
                let fname_esc = ses_escape_fname(s);
                if !write_fmt(fd, format!("$argadd {}\n", cstr_to_str(fname_esc.as_ptr()))) {
                    return false;
                }
            }
        }
        true
}

fn ses_get_fname(buf: *mut buf_T, flagp: &c_uint) -> *mut c_char {
    unsafe {
        if !(*buf).b_sfname.is_null()
            && ptr::eq(flagp, &raw const opt_vars().ssop_flags)
            && (opt_vars().ssop_flags & (kOptSsopFlagCurdir | kOptSsopFlagSesdir)) != 0
            && opt_vars().p_acd == 0
            && !exses_st().did_lcd
        {
            return (*buf).b_sfname;
        }
        (*buf).b_ffname
    }
}

fn ses_fname(fd: *mut FILE, buf: *mut buf_T, flagp: &mut c_uint, add_eol: bool) -> bool {
    let name = ses_get_fname(buf, flagp);
        if !ses_put_fname(fd, name, flagp) || (add_eol && !write_bytes(fd, b"\n")) {
            return false;
        }
        true
}

fn ses_escape_fname(name: *mut c_char) -> CString {
    unsafe {
        let mut sname_vec: Vec<c_char> = home_replace_save(ptr::null_mut(), name)
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();

        let mut p = sname_vec.as_mut_ptr();
        while *p != NUL {
            if *p == b'\\' as c_char {
                *p = b'/' as c_char;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        vim_strsave_fnameescape(sname_vec.as_ptr(), VSE_NONE)
    }
}

fn ses_put_fname(fd: *mut FILE, name: *mut c_char, _flagp: &mut c_uint) -> bool {
    unsafe {
        let p = ses_escape_fname(name);
        fputs(p.as_ptr(), fd) >= 0
    }
}

fn put_view(
    fd: *mut FILE,
    wp: *mut win_T,
    tp: &mut tabpage_T,
    add_edit: bool,
    flagp: &mut c_uint,
    current_arg_idx: c_int,
) -> bool {
    unsafe {
        let mut did_next = false;
        let mut do_cursor =
            ptr::eq(flagp, &raw mut opt_vars().ssop_flags) || (*flagp & kOptSsopFlagCursor) != 0;

        if (*wp).w_alist.is_none() {
            if !put_line(fd, c"argglobal") {
                return false;
            }
        } else {
            let alist = crate::arglist::win_alist(&*wp);
            if !ses_arglist(
                fd,
                c"arglocal",
                &(*alist).al_ga,
                ptr::eq(&raw mut *flagp, &raw mut opt_vars().vop_flags)
                    || (*flagp & kOptSsopFlagCurdir) == 0
                    || tp.tp_localdir.is_some()
                    || (*wp).w_localdir.is_some(),
                &mut *flagp,
            )
            {
                return false;
            }
        }

        let wargcount = (*crate::arglist::win_alist(&*wp)).al_ga.len() as c_int;
        if (*wp).w_arg_idx != current_arg_idx
            && (*wp).w_arg_idx < wargcount
            && ptr::eq(flagp, &raw mut opt_vars().ssop_flags)
        {
            if !write_fmt(fd, format!("{}argu\n", i64::from((*wp).w_arg_idx) + 1)) {
                return false;
            }
            did_next = true;
        }

        if add_edit && (!did_next || (*wp).w_arg_idx_invalid) {
            let fname_esc = ses_escape_fname(ses_get_fname((*wp).w_buffer, flagp));
            if bt_help((*wp).w_buffer) {
                let mut curtag: *const c_char = c"".as_ptr();
                if 0 < (*wp).w_tagstackidx && (*wp).w_tagstackidx <= (*wp).w_tagstacklen {
                    curtag = (*wp).w_tagstack[((*wp).w_tagstackidx - 1) as usize]
                        .tagname
                        .as_ref()
                        .map_or(c"".as_ptr(), |s| s.as_ptr());
                }

                if !put_line(fd, c"enew | setl bt=help")
                    || !write_fmt(fd, format!("help {}", cstr_to_str(curtag)))
                    || !put_eol(fd)
                {
                    return false;
                }
            } else if !(*(*wp).w_buffer).b_ffname.is_null()
                && (!bt_nofilename((*wp).w_buffer) || !(*(*wp).w_buffer).terminal.is_null())
            {
                let fesc = cstr_to_str(fname_esc.as_ptr());
                if !write_fmt(
                    fd,
                    format!(
                        "if bufexists(fnamemodify(\"{fesc}\", \":p\")) | buffer {fesc} | else | edit {fesc} | endif\nif &buftype ==# 'terminal'\n  silent file {fesc}\nendif\n"
                    ),
                ) {
                    return false;
                }
            } else {
                if !put_line(fd, c"enew") {
                    return false;
                }
                if !(*(*wp).w_buffer).b_ffname.is_null()
                    && !write_fmt(fd, format!("file {}\n", cstr_to_str(fname_esc.as_ptr())))
                {
                    return false;
                }
                do_cursor = false;
            }
        }

        if (*wp).w_alt_fnum != 0 {
            let alt = buflist_findnr((*wp).w_alt_fnum);
            if ptr::eq(flagp, &raw mut opt_vars().ssop_flags)
                && !alt.is_null()
                && !(*alt).b_fname.is_null()
                && *(*alt).b_fname != NUL
                && (*alt).b_p_bl != 0
                && !(bt_terminal(alt) && (opt_vars().ssop_flags & kOptSsopFlagTerminal) == 0)
                && (fputs(c"balt ".as_ptr(), fd) < 0 || !ses_fname(fd, alt, &mut *flagp, true))
            {
                return false;
            }
        }

        if (*flagp & (kOptSsopFlagOptions | kOptSsopFlagLocaloptions)) != 0
            && !makemap(fd, (*wp).w_buffer)
        {
            return false;
        }

        let save_curwin = *curwin.as_ptr();
        *curwin.as_ptr() = wp;
        g_st().curbuf = (**curwin.as_ptr()).w_buffer;
        let f = if (*flagp & (kOptSsopFlagOptions | kOptSsopFlagLocaloptions)) != 0 {
            makeset(
                fd,
                OPT_LOCAL,
                c_int::from(
                    ptr::eq(flagp, &raw mut opt_vars().vop_flags) || (*flagp & kOptSsopFlagOptions) == 0,
                ),
            )
        } else if (*flagp & kOptSsopFlagFolds) != 0 {
            makefoldset(fd)
        } else {
            true
        };
        *curwin.as_ptr() = save_curwin;
        g_st().curbuf = (**curwin.as_ptr()).w_buffer;
        if !f {
            return false;
        }

        if (*flagp & kOptSsopFlagFolds) != 0
            && !(*(*wp).w_buffer).b_ffname.is_null()
            && (bt_normal((*wp).w_buffer) || bt_help((*wp).w_buffer))
            && !put_folds(fd, wp)
        {
            return false;
        }

        if do_cursor {
            if (*wp).w_view_height <= 0 {
                if !write_fmt(fd, format!("let s:l = {}\n", (*wp).w_cursor.lnum)) {
                    return false;
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
                return false;
            }
            if !write_fmt(
                fd,
                format!(
                    "if s:l < 1 | let s:l = 1 | endif\nkeepjumps exe s:l\nnormal! zt\nkeepjumps {}\n",
                    (*wp).w_cursor.lnum
                ),
            ) {
                return false;
            }
            if (*wp).w_cursor.col == 0 {
                if !put_line(fd, c"normal! 0") {
                    return false;
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
                    || !put_line(fd, c"endif")
                {
                    return false;
                }
            } else if put_view_curpos(fd, wp, "") == FAIL {
                return false;
            }
        }

        if let Some(ref w_localdir) = (*wp).w_localdir
            && (!ptr::eq(flagp, &raw mut opt_vars().vop_flags) || (*flagp & kOptSsopFlagCurdir) != 0)
        {
            if fputs(c"lcd ".as_ptr(), fd) < 0
                || !ses_put_fname(fd, w_localdir.as_ptr().cast_mut(), &mut *flagp)
                || !write_bytes(fd, b"\n")
            {
                return false;
            }
            exses_st().did_lcd = true;
        }

        true
    }
}

fn store_session_globals(fd: *mut FILE) -> bool {
    unsafe {
        let d = get_globvar_dict();
        let ht = &raw mut (*d).dv_hashtab;
        let mut todo = (*ht).ht_used;
        let mut hi = (&*ht).array_ptr();
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let this_var = crate::eval::typval::tv_dict_hi2di(hi);
                let key_ptr = (*this_var).di_key.as_mut_ptr();
                if ((*this_var).di_tv.v_type == VarType::VAR_NUMBER
                    || (*this_var).di_tv.v_type == VarType::VAR_STRING)
                    && var_flavour(key_ptr) == VAR_FLAVOUR_SESSION
                {
                    let p = vim_strsave_escaped(
                        std::ffi::CStr::from_ptr(tv_get_string(&(*this_var).di_tv)),
                        c"\\\"\n\r",
                    );
                    let mut p_vec: Vec<c_char> = p
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    let mut t = p_vec.as_mut_ptr();
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
                            cstr_to_str(p_vec.as_ptr())
                        ),
                    ) || !put_eol(fd)
                    {
                        return false;
                    }
                } else if (*this_var).di_tv.v_type == VarType::VAR_FLOAT
                    && var_flavour(key_ptr) == VAR_FLAVOUR_SESSION
                {
                    let mut f = (*this_var).di_tv.vval.v_float();
                    let sign = if f < 0.0 {
                        f = -f;
                        '-'
                    } else {
                        ' '
                    };
                    if !write_fmt(fd, format!("let {} = {sign}{f:.6}", cstr_to_str(key_ptr)))
                        || !put_eol(fd)
                    {
                        return false;
                    }
                }
            }
            hi = hi.add(1);
        }
        true
    }
}

fn makeopens(fd: *mut FILE, dirnow: *mut c_char) -> bool {
    unsafe {
        let mut only_save_windows = true;
        let mut edited_win: *mut win_T = ptr::null_mut();
        let mut cur_arg_idx: c_int = 0;
        let mut next_arg_idx: c_int = 0;

        if (opt_vars().ssop_flags & kOptSsopFlagBuffers) != 0 {
            only_save_windows = false;
        }

        if !put_line(
            fd,
            c"let v:this_session=expand(\"<sfile>:p\")",
        )
        {
            return false;
        }
        if !put_line(fd, c"doautoall SessionLoadPre") {
            return false;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagGlobals) != 0 && !store_session_globals(fd) {
            return false;
        }

        if !put_line(fd, c"silent only") {
            return false;
        }
        if (opt_vars().ssop_flags & kOptSsopFlagTabpages) != 0
            && !put_line(fd, c"silent tabonly")
        {
            return false;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagSesdir) != 0 {
            if !put_line(
                fd,
                c"exe \"cd \" . escape(expand(\"<sfile>:p:h\"), ' ')",
            )
            {
                return false;
            }
        } else if (opt_vars().ssop_flags & kOptSsopFlagCurdir) != 0 {
            let sname = home_replace_save(
                ptr::null_mut(),
                (&raw const g_st().globaldir)
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .map_or(dirnow, |s| s.as_ptr().cast_mut()),
            );
            let fname_esc = ses_escape_fname(sname.as_ptr().cast_mut());
            if !write_fmt(fd, format!("cd {}\n", cstr_to_str(fname_esc.as_ptr()))) {
                return false;
            }
        }

        if !write_bytes(
            fd,
            b"if expand('%') == '' && !&modified && line('$') <= 1 && getline(1) == ''\n  let s:wipebuf = bufnr('%')\nendif\n",
        ) {
            return false;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagOptions) == 0
            && !put_line(fd, c"let s:shortmess_save = &shortmess")
        {
            return false;
        }

        if !put_line(fd, c"set shortmess+=aoO") {
            return false;
        }

        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if !(only_save_windows && (*buf).b_nwindows == 0)
                && !((*buf).b_help && (opt_vars().ssop_flags & kOptSsopFlagHelp) == 0)
                && !(bt_terminal(buf) && (opt_vars().ssop_flags & kOptSsopFlagTerminal) == 0)
                && !(*buf).b_fname.is_null()
                && (*buf).b_p_bl != 0
            {
                let lnum = (*buf)
                    .b_wininfo
                    .first()
                    .map_or(1i64, |wi| i64::from(wi.wi_mark.mark.lnum));
                if !write_fmt(fd, format!("badd +{lnum} "))
                    || !ses_fname(fd, buf, &mut opt_vars().ssop_flags, true)
                {
                    return false;
                }
            }
            buf = (*buf).b_next;
        }

        let ga_ptr = &raw const g_st().global_alist;
        if !ses_arglist(
            fd,
            c"argglobal",
            &(*ga_ptr).al_ga,
            (opt_vars().ssop_flags & kOptSsopFlagCurdir) == 0,
            &mut opt_vars().ssop_flags,
        )
        {
            return false;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagCurdir) != 0 {
            let mut b = g_st().firstbuf;
            while !b.is_null() {
                let Some(ref b_localdir) = (*b).b_localdir else {
                    b = (*b).b_next;
                    continue;
                };
                if (*b).b_fname.is_null()
                    || (*b).b_p_bl == 0
                    || (only_save_windows && (*b).b_nwindows == 0)
                    || ((*b).b_help && (opt_vars().ssop_flags & kOptSsopFlagHelp) == 0)
                    || (bt_terminal(b) && (opt_vars().ssop_flags & kOptSsopFlagTerminal) == 0)
                {
                    b = (*b).b_next;
                    continue;
                }
                if fputs(
                    c"lua vim.api.nvim_create_autocmd('BufEnter', { once = true, buffer = vim.fn.bufadd([==["
                        .as_ptr(),
                    fd,
                ) < 0
                    || fputs(ses_get_fname(b, &opt_vars().ssop_flags), fd) < 0
                    || fputs(
                        c"]==]), callback = function() vim.cmd.bcd({ [==[".as_ptr(),
                        fd,
                    ) < 0
                    || fputs(b_localdir.as_ptr(), fd) < 0
                    || fputs(
                        c"]==], magic = { file = false, bar = false } }) end })\n".as_ptr(),
                        fd,
                    ) < 0
                {
                    return false;
                }
                exses_st().did_lcd = true;
                b = (*b).b_next;
            }
        }

        if (opt_vars().ssop_flags & kOptSsopFlagResize) != 0
            && !write_fmt(
                fd,
                format!(
                    "set lines={} columns={}\n",
                    i64::from(g_st().Rows),
                    i64::from(g_st().Columns)
                ),
            )
        {
            return false;
        }

        let mut restore_stal = false;
        if opt_vars().p_stal == 1 && !(*g_st().first_tabpage).tp_next.is_null() {
            if !put_line(fd, c"set stal=2") {
                return false;
            }
            restore_stal = true;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagTabpages) != 0 {
            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                if !(*tp).tp_next.is_null()
                    && !put_line(fd, c"tabnew +setlocal\\ bufhidden=wipe")
                {
                    return false;
                }
                tp = (*tp).tp_next;
            }

            if !(*g_st().first_tabpage).tp_next.is_null()
                && !put_line(fd, c"tabrewind")
            {
                return false;
            }
        }

        let mut restore_height_width = false;
        let mut tp_iter = g_st().first_tabpage;
        while !tp_iter.is_null() {
            let mut tp = tp_iter;
            let mut need_tabnext = false;
            let mut cnr: c_int = 1;
            let tab_firstwin: *mut win_T;
            let tab_topframe: *mut frame_T;

            if (opt_vars().ssop_flags & kOptSsopFlagTabpages) != 0 {
                if ptr::eq(tp, g_st().curtab) {
                    tab_firstwin = g_st().firstwin;
                    tab_topframe = g_st().topframe;
                } else {
                    tab_firstwin = (*tp).tp_firstwin;
                    tab_topframe = (*tp).tp_topframe.cast::<frame_T>();
                }
                if !ptr::eq(tp, g_st().first_tabpage) {
                    need_tabnext = true;
                }
            } else {
                tp = g_st().curtab;
                tab_firstwin = g_st().firstwin;
                tab_topframe = g_st().topframe;
            }

            if (opt_vars().ssop_flags & kOptSsopFlagCurdir) != 0
                && let Some(ref tp_localdir) = (*tp).tp_localdir
            {
                if need_tabnext && !put_line(fd, c"tabnext") {
                    return false;
                }
                need_tabnext = false;
                if fputs(c"tcd ".as_ptr(), fd) < 0
                    || !ses_put_fname(fd, tp_localdir.as_ptr().cast_mut(), &mut opt_vars().ssop_flags)
                    || !put_eol(fd)
                {
                    return false;
                }
                exses_st().did_lcd = true;
            }

            let mut wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(&mut *wp) != 0
                    && !(*(*wp).w_buffer).b_ffname.is_null()
                    && !bt_help((*wp).w_buffer)
                    && !bt_nofilename((*wp).w_buffer)
                {
                    if need_tabnext && !put_line(fd, c"tabnext") {
                        return false;
                    }
                    need_tabnext = false;

                    if fputs(c"edit ".as_ptr(), fd) < 0
                        || !ses_fname(fd, (*wp).w_buffer, &mut opt_vars().ssop_flags, true)
                    {
                        return false;
                    }
                    if !(*wp).w_arg_idx_invalid {
                        edited_win = wp;
                    }
                    break;
                }
                wp = (*wp).w_next;
            }

            if need_tabnext && !put_line(fd, c"tabnext") {
                return false;
            }

            if (*tab_topframe).fr_layout != FR_LEAF
                && (!put_line(
                    fd,
                    c"let s:save_splitbelow = &splitbelow",
                )
                    || !put_line(
                        fd,
                        c"let s:save_splitright = &splitright",
                    )
                    || !put_line(fd, c"set splitbelow splitright")
                    || !ses_win_rec(fd, &mut *tab_topframe)
                    || !put_line(
                        fd,
                        c"let &splitbelow = s:save_splitbelow",
                    )
                    || !put_line(
                        fd,
                        c"let &splitright = s:save_splitright",
                    ))
            {
                return false;
            }

            let mut nr: c_int = 0;
            let mut restore_size = true;
            wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(&mut *wp) != 0 {
                    nr += 1;
                } else if !(*wp).w_floating {
                    restore_size = false;
                }
                if ptr::eq(*curwin.as_ptr(), wp) {
                    cnr = nr;
                }
                wp = (*wp).w_next;
            }

            if !tab_firstwin.is_null() && !(*tab_firstwin).w_next.is_null() {
                if !put_line(fd, c"wincmd t") {
                    return false;
                }
                if !restore_height_width
                    && (!put_line(
                        fd,
                        c"let s:save_winminheight = &winminheight",
                    )
                        || !put_line(
                            fd,
                            c"let s:save_winminwidth = &winminwidth",
                        ))
                {
                    return false;
                }
                if !write_bytes(
                    fd,
                    b"set winminheight=0\nset winheight=1\nset winminwidth=0\nset winwidth=1\n",
                ) {
                    return false;
                }
                restore_height_width = true;
            }
            if nr > 1 && !ses_winsizes(fd, restore_size, tab_firstwin) {
                return false;
            }

            wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(&mut *wp) == 0 {
                    wp = (*wp).w_next;
                    continue;
                }
                if !put_view(
                    fd,
                    wp,
                    &mut *tp,
                    !ptr::eq(wp, edited_win),
                    &mut opt_vars().ssop_flags,
                    cur_arg_idx,
                )
                {
                    return false;
                }
                if nr > 1 && !put_line(fd, c"wincmd w") {
                    return false;
                }
                next_arg_idx = (*wp).w_arg_idx;
                wp = (*wp).w_next;
            }

            cur_arg_idx = next_arg_idx;

            if cnr > 1 && !write_fmt(fd, format!("{cnr}wincmd w\n")) {
                return false;
            }

            if nr > 1 && !ses_winsizes(fd, restore_size, tab_firstwin) {
                return false;
            }

            if (opt_vars().ssop_flags & kOptSsopFlagTabpages) == 0 {
                break;
            }
            tp_iter = (*tp_iter).tp_next;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagTabpages) != 0
            && !write_fmt(fd, format!("tabnext {}\n", tabpage_index(g_st().curtab)))
        {
            return false;
        }
        if restore_stal && !put_line(fd, c"set stal=1") {
            return false;
        }

        if !write_bytes(
            fd,
            b"if exists('s:wipebuf') && len(win_findbuf(s:wipebuf)) == 0 && getbufvar(s:wipebuf, '&buftype') isnot# 'terminal'\n  silent exe 'bwipe ' . s:wipebuf\nendif\nunlet! s:wipebuf\n",
        ) {
            return false;
        }

        let wh = opt_vars().p_wh;
        let wiw = opt_vars().p_wiw;
        if !write_fmt(fd, format!("set winheight={wh} winwidth={wiw}\n")) {
            return false;
        }

        if (opt_vars().ssop_flags & kOptSsopFlagOptions) != 0 {
            if !write_fmt(fd, format!("set shortmess={}\n", cstr_to_str(opt_vars().p_shm))) {
                return false;
            }
        } else if !put_line(fd, c"let &shortmess = s:shortmess_save") {
            return false;
        }

        if restore_height_width
            && (!put_line(
                fd,
                c"let &winminheight = s:save_winminheight",
            )
                || !put_line(
                    fd,
                    c"let &winminwidth = s:save_winminwidth",
                ))
        {
            return false;
        }

        if !write_bytes(
            fd,
            b"let s:sx = expand(\"<sfile>:p:r\").\"x.vim\"\nif filereadable(s:sx)\n  exe \"source \" . fnameescape(s:sx)\nendif\n",
        ) {
            return false;
        }

        true
    }
}

pub fn ex_loadview(eap: &mut exarg_T) {
    unsafe {
        let Some(fname) = get_view_file(*eap.arg) else {
            return;
        };

        if !do_source(fname.as_ptr().cast_mut(), false, DOSO_NONE, ptr::null_mut()) {
            semsg!(gettext(e_notopen.as_ptr()), fname.as_ptr());
        }
    }
}

pub fn ex_mkrc(eap: &mut exarg_T) {
    unsafe {
        let mut view_session = false;
        let mut using_vdir = false;
        let mut view_file: Option<CString> = None;

        if eap.cmdidx == CMD_mksession || eap.cmdidx == CMD_mkview {
            view_session = true;
        }

        exses_st().did_lcd = false;

        let fname: *mut c_char;
        if eap.cmdidx == CMD_mkview
            && (*eap.arg == NUL
                || (ascii_isdigit(*eap.arg as c_int) && *eap.arg.add(1) == NUL))
        {
            eap.forceit = 1;
            let Some(vf) = get_view_file(*eap.arg) else {
                return;
            };
            fname = vf.as_ptr().cast_mut();
            view_file = Some(vf);
            using_vdir = true;
        } else if *eap.arg != NUL {
            fname = eap.arg;
        } else if eap.cmdidx == CMD_mkvimrc {
            fname = c".nvimrc".as_ptr().cast_mut();
        } else if eap.cmdidx == CMD_mksession {
            fname = c"Session.vim".as_ptr().cast_mut();
        } else {
            fname = c".exrc".as_ptr().cast_mut();
        }

        if using_vdir && !os_isdir(opt_vars().p_vdir) {
            vim_mkdir_emsg(opt_vars().p_vdir, 0o755);
        }

        let fd = open_exfile(fname, eap.forceit, c"wb");
        if !fd.is_null() {
            let mut failed = false;
            let flagp = if eap.cmdidx == CMD_mkview {
                &raw mut opt_vars().vop_flags
            } else {
                &raw mut opt_vars().ssop_flags
            };

            if eap.cmdidx == CMD_mksession {
                apply_autocmds(
                    EVENT_SESSIONWRITEPRE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
            }

            if eap.cmdidx == CMD_mkvimrc {
                put_line(fd, c"version 6.0");
            }

            if eap.cmdidx == CMD_mksession
                && !put_line(fd, c"let SessionLoad = 1")
            {
                failed = true;
            }

            if !view_session
                || (eap.cmdidx == CMD_mksession && (*flagp & kOptSsopFlagOptions) != 0)
            {
                let mut flags = OPT_GLOBAL;
                if eap.cmdidx == CMD_mksession && (*flagp & kOptSsopFlagSkiprtp) != 0 {
                    flags |= OPT_SKIPRTP;
                }
                failed |= !makemap(fd, ptr::null_mut()) || !makeset(fd, flags, 0);
            }

            if !failed && view_session {
                if !put_line(
                    fd,
                    c"let s:so_save = &g:so | let s:siso_save = &g:siso | setg so=0 siso=0 | setl so=-1 siso=-1",
                )
                {
                    failed = true;
                }
                if eap.cmdidx == CMD_mksession {
                    let mut dirnow_vec: Vec<c_char> = vec![0; MAXPATHL];
                    let dirnow = dirnow_vec.as_mut_ptr();

                    if !os_dirname(dirnow, MAXPATHL) || os_chdir(dirnow) != 0 {
                        *dirnow = NUL;
                    }
                    if *dirnow != NUL && (opt_vars().ssop_flags & kOptSsopFlagSesdir) != 0 {
                        if vim_chdirfile(fname, kCdCauseOther) {
                            shorten_fnames(1);
                        }
                    } else if *dirnow != NUL
                        && (opt_vars().ssop_flags & kOptSsopFlagCurdir) != 0
                        && let Some(ref gdir) = g_st().globaldir
                        && os_chdir(gdir.as_ptr()) == 0
                    {
                        shorten_fnames(1);
                    }

                    failed |= !makeopens(fd, dirnow);

                    if *dirnow != NUL
                        && ((opt_vars().ssop_flags & kOptSsopFlagSesdir) != 0
                            || ((opt_vars().ssop_flags & kOptSsopFlagCurdir) != 0
                                && g_st().globaldir.is_some()))
                    {
                        if os_chdir(dirnow) != 0 {
                            emsg(gettext(e_prev_dir.as_ptr()));
                        }
                        shorten_fnames(1);
                    }
                } else {
                    failed |= !put_view(fd, *curwin.as_ptr(), &mut *g_st().curtab, !using_vdir, &mut *flagp, -1);
                }
                if !write_bytes(fd, b"let &g:so = s:so_save | let &g:siso = s:siso_save\n") {
                    failed = true;
                }
                if opt_vars().p_hls != 0 && !write_bytes(fd, b"set hlsearch\n") {
                    failed = true;
                }
                if g_st().Search.no_hlsearch && !write_bytes(fd, b"nohlsearch\n") {
                    failed = true;
                }
                if eap.cmdidx == CMD_mksession
                    && !write_bytes(fd, b"doautoall SessionLoadPost\nunlet SessionLoad\n")
                {
                    failed = true;
                }
            }
            if !put_line(fd, c"\" vim: set ft=vim :") {
                failed = true;
            }

            failed |= fclose(fd) != 0;

            if failed {
                emsg(gettext(e_write.as_ptr()));
            } else if eap.cmdidx == CMD_mksession {
                let mut tbuf: Vec<c_char> = vec![0; MAXPATHL];
                if vim_FullName(fname, tbuf.as_mut_ptr(), MAXPATHL, false) {
                    set_vim_var_string(VV_THIS_SESSION, tbuf.as_ptr(), -1);
                }
            }
        }

        drop(view_file);

        if eap.cmdidx == CMD_mksession {
            apply_autocmds(
                EVENT_SESSIONWRITEPOST,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
    }
}

fn get_view_file(c: c_char) -> Option<CString> {
    unsafe {
        if (*g_st().curbuf).b_ffname.is_null() {
            emsg(gettext(e_noname.as_ptr()));
            return None;
        }
        let sname_cstr = home_replace_save(ptr::null_mut(), (*g_st().curbuf).b_ffname);
        let sname = sname_cstr.as_ptr();

        let mut len: usize = 0;
        let mut p = sname;
        while *p != NUL {
            if *p == b'=' as c_char || vim_ispathsep(c_int::from(*p as u8)) {
                len += 1;
            }
            p = p.add(1);
        }
        let sname_len = sname_cstr.as_bytes().len();
        let vdir_len = crate::memory::cstr_len(opt_vars().p_vdir);
        let mut retval_vec: Vec<c_char> = vec![0; sname_len + len + vdir_len + 9];
        let retval = retval_vec.as_mut_ptr();
        ptr::copy_nonoverlapping(opt_vars().p_vdir, retval, vdir_len + 1);
        add_pathsep(retval);
        let mut s = retval.add(crate::memory::cstr_len(retval));
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
        crate::memory::cstr_ncpy(s, c".vim".as_ptr(), 4);

        Some(crate::memory::cstr_to_cstring(retval))
    }
}

pub fn put_eol(fd: *mut FILE) -> bool {
    unsafe {
        if fputc(b'\n' as c_int, fd) < 0 {
            return false;
        }
        true
    }
}

pub fn put_line(fd: *mut FILE, s: &std::ffi::CStr) -> bool {
    unsafe {
        if fputs(s.as_ptr().cast_mut(), fd) < 0 || fputc(b'\n' as c_int, fd) < 0 {
            return false;
        }
        true
    }
}
