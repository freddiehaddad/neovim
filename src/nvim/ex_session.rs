//! Functions for creating a session file (`:mkexrc`, `:mkvimrc`, `:mkview`, `:mksession`) (`src/nvim/ex_session.c`).

use crate::arglist::{Aentry, alist_name};
use crate::autocmd::{EVENT_SESSIONWRITEPOST, EVENT_SESSIONWRITEPRE, apply_autocmds};
use crate::buffer::{FileBuf, bt_help, bt_nofilename, bt_normal, bt_terminal, buflist_findnr};
use crate::charset::{NUL, ascii_isdigit};
use crate::errors::*;
use crate::eval::typval::{VarType, tv_get_string};
use crate::eval::vars::{VV_THIS_SESSION, get_globvar_dict, set_vim_var_string};
use crate::eval::{VAR_FLAVOUR_SESSION, var_flavour};
use crate::ex_docmd::{Exarg, open_exfile, vim_mkdir_emsg};
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
use crate::path::{add_pathsep, vim_full_name, vim_ispathsep};
use crate::runtime::{DOSO_NONE, do_source};
use crate::strings::{gettext, vim_strsave_escaped};
use crate::window::{Frame, TabPageInfo, WinInfoInfo, tabpage_index};
use std::ffi::{CString, c_char};
use std::ptr;

use crate::api::win_config::FR_COL;
use crate::drawscreen::FR_LEAF;
use crate::math::FAIL;
use crate::regexp::MAXCOL;

pub(crate) const OPT_SKIPRTP: i32 = 0x80;
pub(crate) const K_CD_CAUSE_OTHER: i32 = -1;

const CMD_MKSESSION: i32 = 287;
const CMD_MKVIMRC: i32 = 289;
const CMD_MKVIEW: i32 = 290;

const K_OPT_SSOP_FLAG_BUFFERS: u32 = 0x01;
const K_OPT_SSOP_FLAG_RESIZE: u32 = 0x04;
const K_OPT_SSOP_FLAG_WINSIZE: u32 = 0x08;
const K_OPT_SSOP_FLAG_LOCALOPTIONS: u32 = 0x10;
const K_OPT_SSOP_FLAG_OPTIONS: u32 = 0x20;
const K_OPT_SSOP_FLAG_HELP: u32 = 0x40;
const K_OPT_SSOP_FLAG_BLANK: u32 = 0x80;
const K_OPT_SSOP_FLAG_GLOBALS: u32 = 0x100;
pub(crate) const K_OPT_SSOP_FLAG_SESDIR: u32 = 0x800;
pub(crate) const K_OPT_SSOP_FLAG_CURDIR: u32 = 0x1000;
const K_OPT_SSOP_FLAG_FOLDS: u32 = 0x2000;
const K_OPT_SSOP_FLAG_CURSOR: u32 = 0x4000;
const K_OPT_SSOP_FLAG_TABPAGES: u32 = 0x8000;
const K_OPT_SSOP_FLAG_TERMINAL: u32 = 0x10000;
const K_OPT_SSOP_FLAG_SKIPRTP: u32 = 0x20000;

struct ExSessionState {
    did_lcd: bool,
}

impl ExSessionState {
    const fn new() -> Self {
        Self { did_lcd: false }
    }
}

static EXSES_STATE: crate::globals::GlobalState<ExSessionState> =
    crate::globals::GlobalState::new(ExSessionState::new());

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

fn put_view_curpos(fd: *mut FILE, wp: &WinInfoInfo, spaces: &str) -> i32 {
    let s = if wp.w_curswant == MAXCOL {
        format!("{spaces}normal! $\n")
    } else {
        format!("{spaces}normal! 0{}|\n", wp.w_virtcol + 1)
    };
    i32::from(write_fmt(fd, s))
}

fn ses_winsizes(fd: *mut FILE, restore_size: bool, tab_firstwin: *mut WinInfoInfo) -> bool {
    if restore_size && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_WINSIZE) != 0 {
        let mut n: i32 = 0;
        for wp in win_iter_mut(tab_firstwin) {
            if ses_do_win(wp) == 0 {
                continue;
            }
            n += 1;

            if wp.w_height + wp.w_hsep_height + wp.w_status_height
                < crate::globals::topframe_mut().fr_height
                && !write_fmt(
                    fd,
                    format!(
                        "exe '{n}resize ' . ((&lines * {} + {}) / {})\n",
                        i64::from(wp.w_height),
                        i64::from(g_st().rows_global) / 2,
                        i64::from(g_st().rows_global)
                    ),
                )
            {
                return false;
            }

            if wp.w_width < g_st().columns_global
                && !write_fmt(
                    fd,
                    format!(
                        "exe 'vert {n}resize ' . ((&columns * {} + {}) / {})\n",
                        i64::from(wp.w_width),
                        i64::from(g_st().columns_global) / 2,
                        i64::from(g_st().columns_global)
                    ),
                )
            {
                return false;
            }
        }
    } else if !put_line(fd, c"wincmd =") {
        return false;
    }
    true
}

fn ses_win_rec(fd: *mut FILE, fr: &mut Frame) -> bool {
    let mut count: i32 = 0;

    if fr.fr_layout == FR_LEAF {
        return true;
    }

    let mut frc = ses_skipframe(fr.fr_child);
    if !frc.is_null() {
        loop {
            frc = ses_skipframe(unsafe { (*frc).fr_next });
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
        ses_win_rec(fd, unsafe { &mut *frc });
        frc = ses_skipframe(unsafe { (*frc).fr_next });
        if !frc.is_null() && !put_line(fd, c"wincmd w") {
            return false;
        }
    }

    true
}

fn ses_skipframe(fr: *mut Frame) -> *mut Frame {
    let mut frc = fr;
    while !frc.is_null() {
        if ses_do_frame(unsafe { &*frc }) {
            break;
        }
        frc = unsafe { (*frc).fr_next };
    }
    frc
}

fn ses_do_frame(fr: &Frame) -> bool {
    if fr.fr_layout == FR_LEAF {
        return ses_do_win(unsafe { &mut *fr.fr_win }) != 0;
    }
    for frc in frame_iter_mut(fr.fr_child) {
        if ses_do_frame(frc) {
            return true;
        }
    }
    false
}

fn ses_do_win(wp: &mut WinInfoInfo) -> i32 {
    unsafe {
        if wp.w_floating {
            return 0;
        }
        let buf = wp.w_buffer;
        if (*buf).b_fname.is_null() || ((*buf).terminal.is_null() && bt_nofilename(buf.as_ref())) {
            return (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_BLANK) as i32;
        }
        if bt_help(buf.as_ref()) {
            return (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_HELP) as i32;
        }
        if bt_terminal(buf.as_ref()) {
            return (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TERMINAL) as i32;
        }
        1
    }
}

fn ses_arglist(
    fd: *mut FILE,
    cmd: &std::ffi::CStr,
    gap: &[Aentry],
    fullname: bool,
    _flagp: &mut u32,
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
                vim_full_name(s, buf.as_mut_ptr(), MAXPATHL, false);
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

fn ses_get_fname(buf: &mut FileBuf, flagp: &u32) -> *mut c_char {
    if !buf.b_sfname.is_null()
        && ptr::eq(flagp, &raw const opt_vars().ssop_flags)
        && (opt_vars().ssop_flags & (K_OPT_SSOP_FLAG_CURDIR | K_OPT_SSOP_FLAG_SESDIR)) != 0
        && opt_vars().p_acd == 0
        && !exses_st().did_lcd
    {
        return buf.b_sfname;
    }
    buf.b_ffname
}

fn ses_fname(fd: *mut FILE, buf: &mut FileBuf, flagp: &mut u32, add_eol: bool) -> bool {
    let name = ses_get_fname(buf, flagp);
    if !ses_put_fname(fd, name, flagp) || (add_eol && !write_bytes(fd, b"\n")) {
        return false;
    }
    true
}

fn ses_escape_fname(name: impl crate::strings::IntoCCharPtr) -> CString {
    let name = name.into_c_char_ptr();
    {
        let mut sname_vec: Vec<c_char> = home_replace_save(None, name)
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();

        let mut p = sname_vec.as_mut_ptr();
        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) == b'\\' {
                unsafe {
                    *p = b'/' as c_char;
                }
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
        }

        vim_strsave_fnameescape(sname_vec.as_ptr(), VSE_NONE)
    }
}

fn ses_put_fname(fd: *mut FILE, name: impl crate::strings::IntoCCharPtr, _flagp: &mut u32) -> bool {
    let name = name.into_c_char_ptr();
    {
        let p = ses_escape_fname(name);
        fputs(p.as_ptr(), fd) >= 0
    }
}

fn put_view(
    fd: *mut FILE,
    wp: &mut WinInfoInfo,
    tp: &mut TabPageInfo,
    add_edit: bool,
    flagp: &mut u32,
    current_arg_idx: i32,
) -> bool {
    let mut did_next = false;
    let mut do_cursor =
        ptr::eq(flagp, &raw mut opt_vars().ssop_flags) || (*flagp & K_OPT_SSOP_FLAG_CURSOR) != 0;

    if wp.w_alist.is_none() {
        if !put_line(fd, c"argglobal") {
            return false;
        }
    } else {
        let alist = crate::arglist::win_alist(&*wp);
        if !ses_arglist(
            fd,
            c"arglocal",
            &alist.al_ga,
            ptr::eq(&raw mut *flagp, &raw mut opt_vars().vop_flags)
                || (*flagp & K_OPT_SSOP_FLAG_CURDIR) == 0
                || tp.tp_localdir.is_some()
                || wp.w_localdir.is_some(),
            &mut *flagp,
        ) {
            return false;
        }
    }

    let wargcount = crate::arglist::win_alist(&*wp).al_ga.len() as i32;
    if wp.w_arg_idx != current_arg_idx
        && wp.w_arg_idx < wargcount
        && ptr::eq(flagp, &raw mut opt_vars().ssop_flags)
    {
        if !write_fmt(fd, format!("{}argu\n", i64::from(wp.w_arg_idx) + 1)) {
            return false;
        }
        did_next = true;
    }

    if add_edit && (!did_next || wp.w_arg_idx_invalid) {
        let fname_esc = ses_escape_fname(ses_get_fname((*wp).buf_mut(), flagp));
        if bt_help(unsafe { (wp.w_buffer).as_ref() }) {
            let mut curtag: *const c_char = c"".as_ptr();
            if 0 < wp.w_tagstackidx && wp.w_tagstackidx <= wp.w_tagstacklen {
                curtag = wp.w_tagstack[(wp.w_tagstackidx - 1) as usize]
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
        } else if !(*wp).buf_mut().b_ffname.is_null()
            && (!bt_nofilename(unsafe { (wp.w_buffer).as_ref() })
                || !(*wp).buf_mut().terminal.is_null())
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
            if !(*wp).buf_mut().b_ffname.is_null()
                && !write_fmt(fd, format!("file {}\n", cstr_to_str(fname_esc.as_ptr())))
            {
                return false;
            }
            do_cursor = false;
        }
    }

    if wp.w_alt_fnum != 0 {
        let alt = buflist_findnr(wp.w_alt_fnum);
        if ptr::eq(flagp, &raw mut opt_vars().ssop_flags)
            && !alt.is_null()
            && !crate::memory::cstr_is_empty(unsafe { (*alt).b_fname })
            && unsafe { (*alt).b_p_bl } != 0
            && !(bt_terminal(unsafe { alt.as_ref() })
                && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TERMINAL) == 0)
            && (fputs(c"balt ", fd) < 0 || !ses_fname(fd, unsafe { &mut *alt }, &mut *flagp, true))
        {
            return false;
        }
    }

    if (*flagp & (K_OPT_SSOP_FLAG_OPTIONS | K_OPT_SSOP_FLAG_LOCALOPTIONS)) != 0
        && !makemap(fd, wp.w_buffer)
    {
        return false;
    }

    let save_curwin = curwin_ptr();
    set_curwin(wp);
    g_st().curbuf = curwin_mut().w_buffer;
    let f = if (*flagp & (K_OPT_SSOP_FLAG_OPTIONS | K_OPT_SSOP_FLAG_LOCALOPTIONS)) != 0 {
        makeset(
            fd,
            OPT_LOCAL,
            i32::from(
                ptr::eq(flagp, &raw mut opt_vars().vop_flags)
                    || (*flagp & K_OPT_SSOP_FLAG_OPTIONS) == 0,
            ),
        )
    } else if (*flagp & K_OPT_SSOP_FLAG_FOLDS) != 0 {
        makefoldset(fd)
    } else {
        true
    };
    set_curwin(save_curwin);
    g_st().curbuf = curwin_mut().w_buffer;
    if !f {
        return false;
    }

    if (*flagp & K_OPT_SSOP_FLAG_FOLDS) != 0
        && !(*wp).buf_mut().b_ffname.is_null()
        && (bt_normal(unsafe { (wp.w_buffer).as_ref() })
            || bt_help(unsafe { (wp.w_buffer).as_ref() }))
        && !put_folds(fd, &mut *wp)
    {
        return false;
    }

    if do_cursor {
        if wp.w_view_height <= 0 {
            if !write_fmt(fd, format!("let s:l = {}\n", wp.w_cursor.lnum)) {
                return false;
            }
        } else if !write_fmt(
            fd,
            format!(
                "let s:l = {} - (({} * winheight(0) + {}) / {})\n",
                wp.w_cursor.lnum,
                wp.w_cursor.lnum - wp.w_topline,
                wp.w_view_height / 2,
                wp.w_view_height
            ),
        ) {
            return false;
        }
        if !write_fmt(
            fd,
            format!(
                "if s:l < 1 | let s:l = 1 | endif\nkeepjumps exe s:l\nnormal! zt\nkeepjumps {}\n",
                wp.w_cursor.lnum
            ),
        ) {
            return false;
        }
        if wp.w_cursor.col == 0 {
            if !put_line(fd, c"normal! 0") {
                return false;
            }
        } else if wp.w_onebuf_opt.wo_wrap == 0 && wp.w_leftcol > 0 && wp.w_width > 0 {
            let vcol1 = i64::from(wp.w_virtcol) + 1;
            if !write_fmt(
                fd,
                format!(
                    "let s:c = {vcol1} - (({} * winwidth(0) + {}) / {})\nif s:c > 0\n  exe 'normal! ' . s:c . '|zs' . {vcol1} . '|'\nelse\n",
                    i64::from(wp.w_virtcol - wp.w_leftcol),
                    i64::from(wp.w_width / 2),
                    i64::from(wp.w_width),
                ),
            ) || put_view_curpos(fd, &*wp, "  ") == FAIL
                || !put_line(fd, c"endif")
            {
                return false;
            }
        } else if put_view_curpos(fd, &*wp, "") == FAIL {
            return false;
        }
    }

    if let Some(ref w_localdir) = wp.w_localdir
        && (!ptr::eq(flagp, &raw mut opt_vars().vop_flags)
            || (*flagp & K_OPT_SSOP_FLAG_CURDIR) != 0)
    {
        if fputs(c"lcd ", fd) < 0
            || !ses_put_fname(fd, w_localdir.as_ptr().cast_mut(), &mut *flagp)
            || !write_bytes(fd, b"\n")
        {
            return false;
        }
        exses_st().did_lcd = true;
    }

    true
}

fn store_session_globals(fd: *mut FILE) -> bool {
    let d = get_globvar_dict();
    let ht = &mut d.dv_hashtab;
    let mut todo = ht.ht_used;
    let mut hi = (ht).array_ptr();
    while todo > 0 {
        if !hashitem_empty(unsafe { &*hi }) {
            todo -= 1;
            let this_var = crate::eval::typval::tv_dict_hi2di(hi);
            let key_ptr = this_var.di_key.as_mut_ptr();
            if (this_var.di_tv.v_type == VarType::VarNumber
                || this_var.di_tv.v_type == VarType::VarString)
                && var_flavour(key_ptr) == VAR_FLAVOUR_SESSION
            {
                let p = vim_strsave_escaped(tv_get_string(&this_var.di_tv), c"\\\"\n\r");
                let mut p_vec: Vec<c_char> = p
                    .into_bytes_with_nul()
                    .into_iter()
                    .map(|b| b as c_char)
                    .collect();
                let mut t = p_vec.as_mut_ptr();
                while !crate::memory::cstr_is_empty(t) {
                    if crate::memory::cstr_first_byte(t) == b'\n' {
                        unsafe {
                            *t = b'n' as c_char;
                        }
                    } else if crate::memory::cstr_first_byte(t) == b'\r' {
                        unsafe {
                            *t = b'r' as c_char;
                        }
                    }
                    t = t.wrapping_add(1);
                }
                let quote = if this_var.di_tv.v_type == VarType::VarString {
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
            } else if this_var.di_tv.v_type == VarType::VarFloat
                && var_flavour(key_ptr) == VAR_FLAVOUR_SESSION
            {
                let mut f = this_var.di_tv.vval.v_float();
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
        hi = hi.wrapping_add(1);
    }
    true
}

fn makeopens(fd: *mut FILE, dirnow: impl crate::strings::IntoCCharPtr) -> bool {
    let dirnow = dirnow.into_c_char_ptr();
    unsafe {
        let mut only_save_windows = true;
        let mut edited_win: *mut WinInfoInfo = ptr::null_mut();
        let mut cur_arg_idx: i32 = 0;
        let mut next_arg_idx: i32 = 0;

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_BUFFERS) != 0 {
            only_save_windows = false;
        }

        if !put_line(fd, c"let v:this_session=expand(\"<sfile>:p\")") {
            return false;
        }
        if !put_line(fd, c"doautoall SessionLoadPre") {
            return false;
        }

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_GLOBALS) != 0 && !store_session_globals(fd) {
            return false;
        }

        if !put_line(fd, c"silent only") {
            return false;
        }
        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TABPAGES) != 0
            && !put_line(fd, c"silent tabonly")
        {
            return false;
        }

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_SESDIR) != 0 {
            if !put_line(fd, c"exe \"cd \" . escape(expand(\"<sfile>:p:h\"), ' ')") {
                return false;
            }
        } else if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_CURDIR) != 0 {
            let sname = home_replace_save(
                None,
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

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_OPTIONS) == 0
            && !put_line(fd, c"let s:shortmess_save = &shortmess")
        {
            return false;
        }

        if !put_line(fd, c"set shortmess+=aoO") {
            return false;
        }

        for buf in for_all_buffers_ptr() {
            if !(only_save_windows && buf.b_nwindows == 0)
                && !(buf.b_help && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_HELP) == 0)
                && !(bt_terminal(Some(&*buf))
                    && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TERMINAL) == 0)
                && !buf.b_fname.is_null()
                && buf.b_p_bl != 0
            {
                let lnum = buf
                    .b_wininfo
                    .first()
                    .map_or(1i64, |wi| i64::from(wi.wi_mark.mark.lnum));
                if !write_fmt(fd, format!("badd +{lnum} "))
                    || !ses_fname(fd, &mut *buf, &mut opt_vars().ssop_flags, true)
                {
                    return false;
                }
            }
        }

        if !ses_arglist(
            fd,
            c"argglobal",
            &g_st().global_alist.al_ga,
            (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_CURDIR) == 0,
            &mut opt_vars().ssop_flags,
        ) {
            return false;
        }

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_CURDIR) != 0 {
            for b in for_all_buffers_ptr() {
                let b: *mut FileBuf = b;
                let Some(ref b_localdir) = (*b).b_localdir else {
                    continue;
                };
                if (*b).b_fname.is_null()
                    || (*b).b_p_bl == 0
                    || (only_save_windows && (*b).b_nwindows == 0)
                    || ((*b).b_help && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_HELP) == 0)
                    || (bt_terminal(Some(&*b))
                        && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TERMINAL) == 0)
                {
                    continue;
                }
                if fputs(
                    c"lua vim.api.nvim_create_autocmd('BufEnter', { once = true, buffer = vim.fn.bufadd([==[",
                    fd,
                ) < 0
                    || fputs(ses_get_fname( &mut *b, &opt_vars().ssop_flags), fd) < 0
                    || fputs(
                        c"]==]), callback = function() vim.cmd.bcd({ [==[",
                        fd,
                    ) < 0
                    || fputs(b_localdir.as_ptr(), fd) < 0
                    || fputs(
                        c"]==], magic = { file = false, bar = false } }) end })\n",
                        fd,
                    ) < 0
                {
                    return false;
                }
                exses_st().did_lcd = true;
            }
        }

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_RESIZE) != 0
            && !write_fmt(
                fd,
                format!(
                    "set lines={} columns={}\n",
                    i64::from(g_st().rows_global),
                    i64::from(g_st().columns_global)
                ),
            )
        {
            return false;
        }

        let mut restore_stal = false;
        if opt_vars().p_stal == 1 && !crate::globals::first_tabpage_mut().tp_next.is_null() {
            if !put_line(fd, c"set stal=2") {
                return false;
            }
            restore_stal = true;
        }

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TABPAGES) != 0 {
            for tp in for_all_tabpages_mut() {
                if !tp.tp_next.is_null() && !put_line(fd, c"tabnew +setlocal\\ bufhidden=wipe") {
                    return false;
                }
            }

            if !crate::globals::first_tabpage_mut().tp_next.is_null() && !put_line(fd, c"tabrewind")
            {
                return false;
            }
        }

        let mut restore_height_width = false;
        for tp_iter in for_all_tabpages_ptr() {
            let mut tp: *mut TabPageInfo = tp_iter;
            let mut need_tabnext = false;
            let mut cnr: i32 = 1;
            let tab_firstwin: *mut WinInfoInfo;
            let tab_topframe: *mut Frame;

            if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TABPAGES) != 0 {
                if ptr::eq(tp, g_st().curtab) {
                    tab_firstwin = g_st().firstwin;
                    tab_topframe = g_st().topframe;
                } else {
                    tab_firstwin = (*tp).tp_firstwin;
                    tab_topframe = (*tp).tp_topframe.cast::<Frame>();
                }
                if !ptr::eq(tp, g_st().first_tabpage) {
                    need_tabnext = true;
                }
            } else {
                tp = g_st().curtab;
                tab_firstwin = g_st().firstwin;
                tab_topframe = g_st().topframe;
            }

            if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_CURDIR) != 0
                && let Some(ref tp_localdir) = (*tp).tp_localdir
            {
                if need_tabnext && !put_line(fd, c"tabnext") {
                    return false;
                }
                need_tabnext = false;
                if fputs(c"tcd ", fd) < 0
                    || !ses_put_fname(
                        fd,
                        tp_localdir.as_ptr().cast_mut(),
                        &mut opt_vars().ssop_flags,
                    )
                    || !put_eol(fd)
                {
                    return false;
                }
                exses_st().did_lcd = true;
            }

            let mut wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(&mut *wp) != 0
                    && !(*wp).buf_mut().b_ffname.is_null()
                    && !bt_help(((*wp).w_buffer).as_ref())
                    && !bt_nofilename(((*wp).w_buffer).as_ref())
                {
                    if need_tabnext && !put_line(fd, c"tabnext") {
                        return false;
                    }
                    need_tabnext = false;

                    if fputs(c"edit ", fd) < 0
                        || !ses_fname(fd, (*wp).buf_mut(), &mut opt_vars().ssop_flags, true)
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
                && (!put_line(fd, c"let s:save_splitbelow = &splitbelow")
                    || !put_line(fd, c"let s:save_splitright = &splitright")
                    || !put_line(fd, c"set splitbelow splitright")
                    || !ses_win_rec(fd, &mut *tab_topframe)
                    || !put_line(fd, c"let &splitbelow = s:save_splitbelow")
                    || !put_line(fd, c"let &splitright = s:save_splitright"))
            {
                return false;
            }

            let mut nr: i32 = 0;
            let mut restore_size = true;
            wp = tab_firstwin;
            while !wp.is_null() {
                if ses_do_win(&mut *wp) != 0 {
                    nr += 1;
                } else if !(*wp).w_floating {
                    restore_size = false;
                }
                if ptr::eq(curwin_ptr(), wp) {
                    cnr = nr;
                }
                wp = (*wp).w_next;
            }

            if !tab_firstwin.is_null() && !(*tab_firstwin).w_next.is_null() {
                if !put_line(fd, c"wincmd t") {
                    return false;
                }
                if !restore_height_width
                    && (!put_line(fd, c"let s:save_winminheight = &winminheight")
                        || !put_line(fd, c"let s:save_winminwidth = &winminwidth"))
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
                    &mut *(wp),
                    &mut *tp,
                    !ptr::eq(wp, edited_win),
                    &mut opt_vars().ssop_flags,
                    cur_arg_idx,
                ) {
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

            if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TABPAGES) == 0 {
                break;
            }
        }

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_TABPAGES) != 0
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

        if (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_OPTIONS) != 0 {
            if !write_fmt(
                fd,
                format!("set shortmess={}\n", cstr_to_str(opt_vars().p_shm)),
            ) {
                return false;
            }
        } else if !put_line(fd, c"let &shortmess = s:shortmess_save") {
            return false;
        }

        if restore_height_width
            && (!put_line(fd, c"let &winminheight = s:save_winminheight")
                || !put_line(fd, c"let &winminwidth = s:save_winminwidth"))
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

pub fn ex_loadview(eap: &mut Exarg) {
    let Some(fname) = get_view_file(unsafe { *eap.arg }) else {
        return;
    };

    if !do_source(fname.as_ptr().cast_mut(), false, DOSO_NONE, ptr::null_mut()) {
        semsg!(gettext(E_NOTOPEN), fname.as_ptr());
    }
}

pub fn ex_mkrc(eap: &mut Exarg) {
    let mut view_session = false;
    let mut using_vdir = false;
    let mut view_file: Option<CString> = None;

    if eap.cmdidx == CMD_MKSESSION || eap.cmdidx == CMD_MKVIEW {
        view_session = true;
    }

    exses_st().did_lcd = false;

    let fname: *mut c_char;
    if eap.cmdidx == CMD_MKVIEW
        && (crate::memory::cstr_is_empty(eap.arg)
            || (ascii_isdigit(unsafe { *eap.arg } as i32)
                && crate::memory::cstr_byte_at(eap.arg, 1) == 0))
    {
        eap.forceit = 1;
        let Some(vf) = get_view_file(unsafe { *eap.arg }) else {
            return;
        };
        fname = vf.as_ptr().cast_mut();
        view_file = Some(vf);
        using_vdir = true;
    } else if !crate::memory::cstr_is_empty(eap.arg) {
        fname = eap.arg;
    } else if eap.cmdidx == CMD_MKVIMRC {
        fname = c".nvimrc".as_ptr().cast_mut();
    } else if eap.cmdidx == CMD_MKSESSION {
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
        let flagp = if eap.cmdidx == CMD_MKVIEW {
            &raw mut opt_vars().vop_flags
        } else {
            &raw mut opt_vars().ssop_flags
        };

        if eap.cmdidx == CMD_MKSESSION {
            apply_autocmds(
                EVENT_SESSIONWRITEPRE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }

        if eap.cmdidx == CMD_MKVIMRC {
            put_line(fd, c"version 6.0");
        }

        if eap.cmdidx == CMD_MKSESSION && !put_line(fd, c"let SessionLoad = 1") {
            failed = true;
        }

        if !view_session
            || (eap.cmdidx == CMD_MKSESSION && (unsafe { *flagp } & K_OPT_SSOP_FLAG_OPTIONS) != 0)
        {
            let mut flags = OPT_GLOBAL;
            if eap.cmdidx == CMD_MKSESSION && (unsafe { *flagp } & K_OPT_SSOP_FLAG_SKIPRTP) != 0 {
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
            if eap.cmdidx == CMD_MKSESSION {
                let mut dirnow_vec: Vec<c_char> = vec![0; MAXPATHL];
                let dirnow = dirnow_vec.as_mut_ptr();

                if !os_dirname(dirnow, MAXPATHL) || os_chdir(dirnow) != 0 {
                    unsafe {
                        *dirnow = NUL;
                    }
                }
                if !crate::memory::cstr_is_empty(dirnow)
                    && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_SESDIR) != 0
                {
                    if vim_chdirfile(fname, K_CD_CAUSE_OTHER) {
                        shorten_fnames(1);
                    }
                } else if !crate::memory::cstr_is_empty(dirnow)
                    && (opt_vars().ssop_flags & K_OPT_SSOP_FLAG_CURDIR) != 0
                    && let Some(ref gdir) = g_st().globaldir
                    && os_chdir(gdir.as_ptr()) == 0
                {
                    shorten_fnames(1);
                }

                failed |= !makeopens(fd, dirnow);

                if !crate::memory::cstr_is_empty(dirnow)
                    && ((opt_vars().ssop_flags & K_OPT_SSOP_FLAG_SESDIR) != 0
                        || ((opt_vars().ssop_flags & K_OPT_SSOP_FLAG_CURDIR) != 0
                            && g_st().globaldir.is_some()))
                {
                    if os_chdir(dirnow) != 0 {
                        emsg(gettext(E_PREV_DIR));
                    }
                    shorten_fnames(1);
                }
            } else {
                failed |= !put_view(
                    fd,
                    curwin_mut(),
                    curtab_mut(),
                    !using_vdir,
                    unsafe { &mut *flagp },
                    -1,
                );
            }
            if !write_bytes(fd, b"let &g:so = s:so_save | let &g:siso = s:siso_save\n") {
                failed = true;
            }
            if opt_vars().p_hls != 0 && !write_bytes(fd, b"set hlsearch\n") {
                failed = true;
            }
            if g_st().search_global.no_hlsearch && !write_bytes(fd, b"nohlsearch\n") {
                failed = true;
            }
            if eap.cmdidx == CMD_MKSESSION
                && !write_bytes(fd, b"doautoall SessionLoadPost\nunlet SessionLoad\n")
            {
                failed = true;
            }
        }
        if !put_line(fd, c"\" vim: set ft=vim :") {
            failed = true;
        }

        failed |= unsafe { fclose(fd) } != 0;

        if failed {
            emsg(gettext(E_WRITE));
        } else if eap.cmdidx == CMD_MKSESSION {
            let mut tbuf: Vec<c_char> = vec![0; MAXPATHL];
            if vim_full_name(fname, tbuf.as_mut_ptr(), MAXPATHL, false) {
                set_vim_var_string(VV_THIS_SESSION, tbuf.as_ptr(), -1);
            }
        }
    }

    drop(view_file);

    if eap.cmdidx == CMD_MKSESSION {
        apply_autocmds(
            EVENT_SESSIONWRITEPOST,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
    }
}

fn get_view_file(c: c_char) -> Option<CString> {
    unsafe {
        if curbuf_mut().b_ffname.is_null() {
            emsg(gettext(E_NONAME));
            return None;
        }
        let sname_cstr = home_replace_save(None, curbuf_mut().b_ffname);
        let sname = sname_cstr.as_ptr();

        let mut len: usize = 0;
        let mut p = sname;
        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) == b'='
                || vim_ispathsep(i32::from(crate::memory::cstr_first_byte(p)))
            {
                len += 1;
            }
            p = p.wrapping_add(1);
        }
        let sname_len = sname_cstr.as_bytes().len();
        let vdir_len = crate::memory::cstr_len(opt_vars().p_vdir);
        let mut retval_vec: Vec<c_char> = vec![0; sname_len + len + vdir_len + 9];
        let retval = retval_vec.as_mut_ptr();
        ptr::copy_nonoverlapping(opt_vars().p_vdir, retval, vdir_len + 1);
        add_pathsep(retval);
        let mut s = retval.wrapping_add(crate::memory::cstr_len(retval));
        p = sname;
        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) == b'=' {
                *s = b'=' as c_char;
                *s.wrapping_add(1) = b'=' as c_char;
                s = s.wrapping_add(2);
            } else if vim_ispathsep(i32::from(crate::memory::cstr_first_byte(p))) {
                *s = b'=' as c_char;
                #[cfg(windows)]
                {
                    *s.wrapping_add(1) = if crate::memory::cstr_first_byte(p) == b':' {
                        b'-' as c_char
                    } else {
                        b'+' as c_char
                    };
                }
                #[cfg(not(windows))]
                {
                    *s.add(1) = b'+' as c_char;
                }
                s = s.wrapping_add(2);
            } else {
                *s = *p;
                s = s.wrapping_add(1);
            }
            p = p.wrapping_add(1);
        }
        *s = b'=' as c_char;
        *s.wrapping_add(1) = c;
        s = s.wrapping_add(2);
        crate::memory::cstr_ncpy(s, c".vim", 4);

        Some(crate::memory::cstr_to_cstring(retval))
    }
}

pub fn put_eol(fd: *mut FILE) -> bool {
    if unsafe { fputc(b'\n' as i32, fd) } < 0 {
        return false;
    }
    true
}

pub fn put_line(fd: *mut FILE, s: &std::ffi::CStr) -> bool {
    if fputs(s.as_ptr().cast_mut(), fd) < 0 || unsafe { fputc(b'\n' as i32, fd) } < 0 {
        return false;
    }
    true
}
