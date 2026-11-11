//! Functions for creating a session file (`:mkexrc`, `:mkvimrc`, `:mkview`, `:mksession`) (`src/nvim/ex_session.c`).
#![forbid(unsafe_code)]

use crate::arglist::{Aentry, alist_name};
use crate::autocmd::{EVENT_SESSIONWRITEPOST, EVENT_SESSIONWRITEPRE, apply_autocmds};
use crate::buffer::{FileBuf, bt_help, bt_nofilename, bt_normal, bt_terminal, buflist_findnr};
use crate::cmdexpand::Cmdidx;
use crate::errors::*;
use crate::eval::typval::{VarType, tv_get_string};
use crate::eval::vars::{VV_THIS_SESSION, get_globvar_dict, set_vim_var_string};
use crate::eval::{VAR_FLAVOUR_SESSION, var_flavour};
use crate::ex_docmd::{Exarg, open_exfile, vim_mkdir_emsg};
use crate::ex_getln::{VSE_NONE, vim_strsave_fnameescape};
use crate::file_search::vim_chdirfile;
use crate::fileio::shorten_fnames;
use crate::fold::put_folds;
use crate::globals::*;
use crate::mapping::makemap;

use crate::message::{emsg, semsg};
use crate::option::{OPT_GLOBAL, OPT_LOCAL, makefoldset, makeset};
use crate::option_vars::*;
use crate::os::env::home_replace_save;
use crate::os::fs::{CFile, MAXPATHL, os_chdir, os_dirname, os_isdir};
use crate::path::{add_pathsep, vim_full_name, vim_ispathsep};
use crate::runtime::{DOSO_NONE, do_source};
use crate::strings::{gettext, vim_strsave_escaped};
use crate::window::{Frame, TabPageInfo, WinInfoInfo, tabpage_index};
use std::ffi::{CStr, CString};
use std::ptr;

use crate::api::win_config::FR_COL;
use crate::drawscreen::FR_LEAF;
use crate::regexp::MAXCOL;

pub(crate) const OPT_SKIPRTP: i32 = 0x80;
pub(crate) const K_CD_CAUSE_OTHER: i32 = -1;

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

std::thread_local! {
    static DID_LCD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[inline]
fn cstr_to_str(p: &CStr) -> &str {
    std::str::from_utf8(p.to_bytes()).unwrap_or("")
}

#[inline]
fn write_bytes(fd: &CFile, bytes: &[u8]) -> bool {
    fd.write_all(bytes)
}

#[inline]
fn write_fmt(fd: &CFile, s: String) -> bool {
    write_bytes(fd, s.as_bytes())
}

fn put_view_curpos(fd: &CFile, wp: &WinInfoInfo, spaces: &str) -> bool {
    let s = if wp.w_curswant.get() == MAXCOL {
        format!("{spaces}normal! $\n")
    } else {
        format!("{spaces}normal! 0{}|\n", wp.w_virtcol.get() + 1)
    };
    write_fmt(fd, s)
}

fn ses_winsizes(fd: &CFile, restore_size: bool, tab_firstwin: Option<&mut WinInfoInfo>) -> bool {
    if restore_size && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_WINSIZE) != 0 {
        let mut n: i32 = 0;
        for wp in win_iter_mut(tab_firstwin.map_or(ptr::null_mut(), ptr::from_mut)) {
            if ses_do_win(wp) == 0 {
                continue;
            }
            n += 1;

            if wp.w_height.get() + wp.w_hsep_height.get() + wp.w_status_height.get()
                < crate::globals::topframe_mut().fr_height.get()
                && !write_fmt(
                    fd,
                    format!(
                        "exe '{n}resize ' . ((&lines * {} + {}) / {})\n",
                        i64::from(wp.w_height.get()),
                        i64::from(g_st().rows_global.get()) / 2,
                        i64::from(g_st().rows_global.get())
                    ),
                )
            {
                return false;
            }

            if wp.w_width.get() < g_st().columns_global.get()
                && !write_fmt(
                    fd,
                    format!(
                        "exe 'vert {n}resize ' . ((&columns * {} + {}) / {})\n",
                        i64::from(wp.w_width.get()),
                        i64::from(g_st().columns_global.get()) / 2,
                        i64::from(g_st().columns_global.get())
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

fn ses_win_rec(fd: &CFile, fr: &Frame) -> bool {
    let mut count: i32 = 0;

    if fr.fr_layout.get() == FR_LEAF {
        return true;
    }

    let mut frc = ses_skipframe(fr.child_opt());
    if let Some(first_frc) = frc {
        frc = ses_skipframe(first_frc.next_opt());
        while let Some(cur_frc) = frc {
            let split_cmd = if fr.fr_layout.get() == FR_COL {
                b"wincmd _ | wincmd |\nsplit\n" as &[u8]
            } else {
                b"wincmd _ | wincmd |\nvsplit\n" as &[u8]
            };
            if !write_bytes(fd, split_cmd) {
                return false;
            }
            count += 1;
            frc = ses_skipframe(cur_frc.next_opt());
        }
    }

    if count > 0 {
        let back_cmd = if fr.fr_layout.get() == FR_COL {
            format!("{count}wincmd k\n")
        } else {
            format!("{count}wincmd h\n")
        };
        if !write_fmt(fd, back_cmd) {
            return false;
        }
    }

    frc = ses_skipframe(fr.child_opt());
    while let Some(cur_frc) = frc {
        let next_opt = cur_frc.next_opt();
        ses_win_rec(fd, &cur_frc);
        frc = ses_skipframe(next_opt);
        if frc.is_some() && !put_line(fd, c"wincmd w") {
            return false;
        }
    }

    true
}

fn ses_skipframe(fr: Option<crate::globals::FramePtr>) -> Option<crate::globals::FramePtr> {
    frame_iter_mut(crate::globals::FramePtr::from(fr)).find(|frc| ses_do_frame(frc))
}

fn ses_do_frame(fr: &Frame) -> bool {
    if fr.fr_layout.get() == FR_LEAF {
        return ses_do_win(fr.win_mut()) != 0;
    }
    for frc in frame_iter_mut(fr.fr_child.get()) {
        if ses_do_frame(&frc) {
            return true;
        }
    }
    false
}

fn ses_do_win(wp: &mut WinInfoInfo) -> i32 {
    if wp.w_floating.get() {
        return 0;
    }
    let buf = wp.buf_mut();
    if buf.fname_opt().is_none() || (buf.terminal.get().is_null() && bt_nofilename(Some(buf))) {
        return (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_BLANK) as i32;
    }
    if bt_help(Some(buf)) {
        return (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_HELP) as i32;
    }
    if bt_terminal(Some(buf)) {
        return (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TERMINAL) as i32;
    }
    1
}

fn ses_arglist(
    fd: &CFile,
    cmd: &std::ffi::CStr,
    gap: &[Aentry],
    fullname: bool,
    _flagp: &std::cell::Cell<u32>,
) -> bool {
    if !write_fmt(fd, format!("{}\n%argdel\n", cstr_to_str(cmd))) {
        return false;
    }
    let mut buf: Vec<u8> = if fullname {
        vec![0; MAXPATHL]
    } else {
        Vec::new()
    };
    for entry in gap {
        if let Some(s_cstr) = alist_name(entry) {
            let s = if fullname {
                vim_full_name(Some(s_cstr), &mut buf, false);
                crate::memory::bytes_as_cstr(&buf)
            } else {
                s_cstr
            };
            let fname_esc = ses_escape_fname(Some(s));
            if !write_fmt(fd, format!("$argadd {}\n", cstr_to_str(&fname_esc))) {
                return false;
            }
        }
    }
    true
}

fn ses_get_fname<'a>(buf: &'a mut FileBuf, flagp: &std::cell::Cell<u32>) -> Option<&'a CStr> {
    if buf.b_sfname.get().is_some()
        && ptr::eq(flagp, &opt_vars().ssop_flags)
        && (opt_vars().ssop_flags.get() & (K_OPT_SSOP_FLAG_CURDIR | K_OPT_SSOP_FLAG_SESDIR)) != 0
        && !opt_vars().p_acd.get()
        && !DID_LCD.get()
    {
        return buf.sfname_opt();
    }
    buf.ffname_opt()
}

fn ses_fname(fd: &CFile, buf: &mut FileBuf, flagp: &std::cell::Cell<u32>, add_eol: bool) -> bool {
    let name = ses_get_fname(buf, flagp);
    if !ses_put_fname(fd, name, flagp) || (add_eol && !write_bytes(fd, b"\n")) {
        return false;
    }
    true
}

fn ses_escape_fname(name: Option<&CStr>) -> CString {
    let mut sname_vec: Vec<u8> = home_replace_save(None, name).into_bytes_with_nul();

    let mut idx = 0usize;
    while idx < sname_vec.len() && sname_vec[idx] != 0 {
        if sname_vec[idx] == b'\\' {
            sname_vec[idx] = b'/';
        }
        idx += crate::mbyte::utfc_slice2len(&sname_vec[idx..]).max(1);
    }

    vim_strsave_fnameescape(&sname_vec, VSE_NONE)
}

fn ses_put_fname(fd: &CFile, name: Option<&CStr>, _flagp: &std::cell::Cell<u32>) -> bool {
    let p = ses_escape_fname(name);
    fd.puts(&p) >= 0
}

fn put_view(
    fd: &CFile,
    wp: &mut WinInfoInfo,
    tp: &TabPageInfo,
    add_edit: bool,
    flagp: &std::cell::Cell<u32>,
    current_arg_idx: i32,
) -> bool {
    let mut did_next = false;
    let mut do_cursor =
        ptr::eq(flagp, &opt_vars().ssop_flags) || (flagp.get() & K_OPT_SSOP_FLAG_CURSOR) != 0;

    if wp.w_alist.get().is_none() {
        if !put_line(fd, c"argglobal") {
            return false;
        }
    } else {
        let alist = crate::arglist::win_alist_rc(&*wp);
        let entries = alist.al_ga.borrow().clone();
        if !ses_arglist(
            fd,
            c"arglocal",
            &entries,
            ptr::eq(flagp, &opt_vars().vop_flags)
                || (flagp.get() & K_OPT_SSOP_FLAG_CURDIR) == 0
                || tp.tp_localdir.get().is_some()
                || wp.w_localdir.get().is_some(),
            flagp,
        ) {
            return false;
        }
    }

    let wargcount = crate::arglist::wargcount(&*wp);
    if wp.w_arg_idx.get() != current_arg_idx
        && wp.w_arg_idx.get() < wargcount
        && ptr::eq(flagp, &opt_vars().ssop_flags)
    {
        if !write_fmt(fd, format!("{}argu\n", i64::from(wp.w_arg_idx.get()) + 1)) {
            return false;
        }
        did_next = true;
    }

    if add_edit && (!did_next || wp.w_arg_idx_invalid.get()) {
        let fname_esc = ses_escape_fname(ses_get_fname((*wp).buf_mut(), flagp));
        if bt_help(wp.buf_opt()) {
            let mut curtag: &CStr = c"";
            if 0 < wp.w_tagstackidx.get() && wp.w_tagstackidx.get() <= wp.w_tagstacklen.get() {
                curtag = wp.w_tagstack[(wp.w_tagstackidx.get() - 1) as usize]
                    .tagname
                    .as_deref()
                    .unwrap_or(c"");
            }

            if !put_line(fd, c"enew | setl bt=help")
                || !write_fmt(fd, format!("help {}", cstr_to_str(curtag)))
                || !put_eol(fd)
            {
                return false;
            }
        } else if (*wp).buf_mut().b_ffname.get().is_some()
            && (!bt_nofilename(wp.buf_opt()) || !(*wp).buf_mut().terminal.get().is_null())
        {
            let fesc = cstr_to_str(&fname_esc);
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
            if (*wp).buf_mut().b_ffname.get().is_some()
                && !write_fmt(fd, format!("file {}\n", cstr_to_str(&fname_esc)))
            {
                return false;
            }
            do_cursor = false;
        }
    }

    if wp.w_alt_fnum.get() != 0
        && let Some(alt) = buflist_findnr(wp.w_alt_fnum.get())
        && ptr::eq(flagp, &opt_vars().ssop_flags)
        && !alt.fname_bytes().is_empty()
        && alt.b_p_bl.get()
        && !(bt_terminal(Some(alt))
            && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TERMINAL) == 0)
        && (fd.puts(c"balt ") < 0 || !ses_fname(fd, alt, flagp, true))
    {
        return false;
    }

    if (flagp.get() & (K_OPT_SSOP_FLAG_OPTIONS | K_OPT_SSOP_FLAG_LOCALOPTIONS)) != 0
        && !makemap(fd, wp.buf_opt())
    {
        return false;
    }

    let save_curwin = curwin_ptr();
    set_curwin(&mut *wp);
    g_st().curbuf.set(curwin_mut().w_buffer.get());
    let f = if (flagp.get() & (K_OPT_SSOP_FLAG_OPTIONS | K_OPT_SSOP_FLAG_LOCALOPTIONS)) != 0 {
        makeset(
            fd,
            OPT_LOCAL,
            i32::from(
                ptr::eq(flagp, &opt_vars().vop_flags)
                    || (flagp.get() & K_OPT_SSOP_FLAG_OPTIONS) == 0,
            ),
        )
    } else if (flagp.get() & K_OPT_SSOP_FLAG_FOLDS) != 0 {
        makefoldset(fd)
    } else {
        true
    };
    set_curwin(save_curwin);
    g_st().curbuf.set(curwin_mut().w_buffer.get());
    if !f {
        return false;
    }

    if (flagp.get() & K_OPT_SSOP_FLAG_FOLDS) != 0
        && (*wp).buf_mut().b_ffname.get().is_some()
        && (bt_normal(wp.buf_opt()) || bt_help(wp.buf_opt()))
        && !put_folds(fd, &mut *wp)
    {
        return false;
    }

    if do_cursor {
        if wp.w_view_height.get() <= 0 {
            if !write_fmt(fd, format!("let s:l = {}\n", wp.w_cursor.lnum)) {
                return false;
            }
        } else if !write_fmt(
            fd,
            format!(
                "let s:l = {} - (({} * winheight(0) + {}) / {})\n",
                wp.w_cursor.lnum,
                wp.w_cursor.lnum - wp.w_topline.get(),
                wp.w_view_height.get() / 2,
                wp.w_view_height.get()
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
        } else if !wp.w_onebuf_opt.wo_wrap.get() && wp.w_leftcol.get() > 0 && wp.w_width.get() > 0 {
            let vcol1 = i64::from(wp.w_virtcol.get()) + 1;
            if !write_fmt(
                fd,
                format!(
                    "let s:c = {vcol1} - (({} * winwidth(0) + {}) / {})\nif s:c > 0\n  exe 'normal! ' . s:c . '|zs' . {vcol1} . '|'\nelse\n",
                    i64::from(wp.w_virtcol.get() - wp.w_leftcol.get()),
                    i64::from(wp.w_width.get() / 2),
                    i64::from(wp.w_width.get()),
                ),
            ) || !put_view_curpos(fd, &*wp, "  ")
                || !put_line(fd, c"endif")
            {
                return false;
            }
        } else if !put_view_curpos(fd, &*wp, "") {
            return false;
        }
    }

    if let Some(ref w_localdir) = wp.w_localdir.get()
        && (!ptr::eq(flagp, &opt_vars().vop_flags) || (flagp.get() & K_OPT_SSOP_FLAG_CURDIR) != 0)
    {
        if fd.puts(c"lcd ") < 0
            || !ses_put_fname(fd, Some(w_localdir.as_c_str()), flagp)
            || !write_bytes(fd, b"\n")
        {
            return false;
        }
        DID_LCD.set(true);
    }

    true
}

fn store_session_globals(fd: &CFile) -> bool {
    let d = get_globvar_dict();
    for (_, this_var) in d.dv_hashtab.iter_items() {
        let key_cstr = this_var.key_cstr();
        let this_tv = this_var.tv();
        if (this_tv.v_type == VarType::VarNumber || this_tv.v_type == VarType::VarString)
            && var_flavour(key_cstr.to_bytes()) == VAR_FLAVOUR_SESSION
        {
            let p = vim_strsave_escaped(tv_get_string(&this_tv).to_bytes(), b"\\\"\n\r");
            let mut p_vec: Vec<u8> = p.into_bytes_with_nul();
            for b in &mut p_vec {
                if *b == 0 {
                    break;
                }
                if *b == b'\n' {
                    *b = b'n';
                } else if *b == b'\r' {
                    *b = b'r';
                }
            }
            let quote = if this_tv.v_type == VarType::VarString {
                '"'
            } else {
                ' '
            };
            if !write_fmt(
                fd,
                format!(
                    "let {} = {quote}{}{quote}",
                    cstr_to_str(key_cstr),
                    cstr_to_str(CStr::from_bytes_until_nul(&p_vec).unwrap_or(c""))
                ),
            ) || !put_eol(fd)
            {
                return false;
            }
        } else if this_tv.v_type == VarType::VarFloat
            && var_flavour(key_cstr.to_bytes()) == VAR_FLAVOUR_SESSION
        {
            let mut f = this_tv.vval.v_float();
            let sign = if f < 0.0 {
                f = -f;
                '-'
            } else {
                ' '
            };
            if !write_fmt(fd, format!("let {} = {sign}{f:.6}", cstr_to_str(key_cstr)))
                || !put_eol(fd)
            {
                return false;
            }
        }
    }
    true
}

fn makeopens(fd: &CFile, dirnow: &CStr) -> bool {
    {
        let mut only_save_windows = true;
        let mut edited_win = crate::globals::WinPtr::NULL;
        let mut cur_arg_idx: i32 = 0;
        let mut next_arg_idx: i32 = 0;

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_BUFFERS) != 0 {
            only_save_windows = false;
        }

        if !put_line(fd, c"let v:this_session=expand(\"<sfile>:p\")") {
            return false;
        }
        if !put_line(fd, c"doautoall SessionLoadPre") {
            return false;
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_GLOBALS) != 0
            && !store_session_globals(fd)
        {
            return false;
        }

        if !put_line(fd, c"silent only") {
            return false;
        }
        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TABPAGES) != 0
            && !put_line(fd, c"silent tabonly")
        {
            return false;
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_SESDIR) != 0 {
            if !put_line(fd, c"exe \"cd \" . escape(expand(\"<sfile>:p:h\"), ' ')") {
                return false;
            }
        } else if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_CURDIR) != 0 {
            let sname = home_replace_save(None, g_st().globaldir.get().as_deref().or(Some(dirnow)));
            let fname_esc = ses_escape_fname(Some(sname.as_c_str()));
            if !write_fmt(fd, format!("cd {}\n", cstr_to_str(&fname_esc))) {
                return false;
            }
        }

        if !write_bytes(
            fd,
            b"if expand('%') == '' && !&modified && line('$') <= 1 && getline(1) == ''\n  let s:wipebuf = bufnr('%')\nendif\n",
        ) {
            return false;
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_OPTIONS) == 0
            && !put_line(fd, c"let s:shortmess_save = &shortmess")
        {
            return false;
        }

        if !put_line(fd, c"set shortmess+=aoO") {
            return false;
        }

        for buf in for_all_buffers_ptr() {
            if !(only_save_windows && buf.b_nwindows.get() == 0)
                && !(buf.b_help.get() && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_HELP) == 0)
                && !(bt_terminal(Some(&*buf))
                    && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TERMINAL) == 0)
                && buf.fname_opt().is_some()
                && buf.b_p_bl.get()
            {
                let lnum = buf
                    .b_wininfo
                    .first()
                    .map_or(1i64, |wi| i64::from(wi.wi_mark.mark.lnum));
                if !write_fmt(fd, format!("badd +{lnum} "))
                    || !ses_fname(fd, &mut *buf, &opt_vars().ssop_flags, true)
                {
                    return false;
                }
            }
        }

        let g_entries = crate::arglist::global_alist_rc().al_ga.borrow().clone();
        if !ses_arglist(
            fd,
            c"argglobal",
            &g_entries,
            (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_CURDIR) == 0,
            &opt_vars().ssop_flags,
        ) {
            return false;
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_CURDIR) != 0 {
            for b in for_all_buffers_ptr() {
                if b.b_localdir.get().is_none()
                    || b.fname_opt().is_none()
                    || !b.b_p_bl.get()
                    || (only_save_windows && b.b_nwindows.get() == 0)
                    || (b.b_help.get() && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_HELP) == 0)
                    || (bt_terminal(Some(&*b))
                        && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TERMINAL) == 0)
                {
                    continue;
                }
                let b_localdir = b.b_localdir.get().clone().unwrap();
                if fd.puts(
                    c"lua vim.api.nvim_create_autocmd('BufEnter', { once = true, buffer = vim.fn.bufadd([==[",
                ) < 0
                    || fd.puts(ses_get_fname(&mut *b, &opt_vars().ssop_flags).unwrap_or(c"")) < 0
                    || fd.puts(c"]==]), callback = function() vim.cmd.bcd({ [==[") < 0
                    || fd.puts(&b_localdir) < 0
                    || fd.puts(c"]==], magic = { file = false, bar = false } }) end })\n") < 0
                {
                    return false;
                }
                DID_LCD.set(true);
            }
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_RESIZE) != 0
            && !write_fmt(
                fd,
                format!(
                    "set lines={} columns={}\n",
                    i64::from(g_st().rows_global.get()),
                    i64::from(g_st().columns_global.get())
                ),
            )
        {
            return false;
        }

        let mut restore_stal = false;
        if opt_vars().p_stal.get() == 1
            && !crate::globals::first_tabpage_mut().tp_next.get().is_null()
        {
            if !put_line(fd, c"set stal=2") {
                return false;
            }
            restore_stal = true;
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TABPAGES) != 0 {
            for tp in for_all_tabpages_mut() {
                if !tp.tp_next.get().is_null()
                    && !put_line(fd, c"tabnew +setlocal\\ bufhidden=wipe")
                {
                    return false;
                }
            }

            if !crate::globals::first_tabpage_mut().tp_next.get().is_null()
                && !put_line(fd, c"tabrewind")
            {
                return false;
            }
        }

        let mut restore_height_width = false;
        for tp_iter in for_all_tabpages_ptr() {
            let mut tp = tp_iter;
            let mut need_tabnext = false;
            let mut cnr: i32 = 1;
            let tab_firstwin;
            let tab_topframe;

            if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TABPAGES) != 0 {
                if g_st().curtab.get() == tp {
                    tab_firstwin = g_st().firstwin.get();
                    tab_topframe = g_st().topframe.get().clone();
                } else {
                    tab_firstwin = tp.tp_firstwin.get();
                    tab_topframe = tp.tp_topframe.get();
                }
                if g_st().first_tabpage.get() != tp {
                    need_tabnext = true;
                }
            } else {
                tp = g_st().curtab.get().clone();
                tab_firstwin = g_st().firstwin.get();
                tab_topframe = g_st().topframe.get().clone();
            }

            if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_CURDIR) != 0
                && let Some(tp_localdir) = tp.tp_localdir.get()
            {
                if need_tabnext && !put_line(fd, c"tabnext") {
                    return false;
                }
                need_tabnext = false;
                if fd.puts(c"tcd ") < 0
                    || !ses_put_fname(fd, Some(tp_localdir.as_c_str()), &opt_vars().ssop_flags)
                    || !put_eol(fd)
                {
                    return false;
                }
                DID_LCD.set(true);
            }

            let mut wp = tab_firstwin;
            while let Some(wp_ref) = wp.opt_mut() {
                if ses_do_win(wp_ref) != 0
                    && wp_ref.buf_mut().b_ffname.get().is_some()
                    && !bt_help(wp_ref.buf_opt())
                    && !bt_nofilename(wp_ref.buf_opt())
                {
                    if need_tabnext && !put_line(fd, c"tabnext") {
                        return false;
                    }
                    need_tabnext = false;

                    if fd.puts(c"edit ") < 0
                        || !ses_fname(fd, wp_ref.buf_mut(), &opt_vars().ssop_flags, true)
                    {
                        return false;
                    }
                    if !wp_ref.w_arg_idx_invalid.get() {
                        edited_win = wp;
                    }
                    break;
                }
                wp = wp_ref.w_next.get();
            }

            if need_tabnext && !put_line(fd, c"tabnext") {
                return false;
            }

            if tab_topframe.fr_layout.get() != FR_LEAF
                && (!put_line(fd, c"let s:save_splitbelow = &splitbelow")
                    || !put_line(fd, c"let s:save_splitright = &splitright")
                    || !put_line(fd, c"set splitbelow splitright")
                    || !ses_win_rec(fd, tab_topframe.borrow())
                    || !put_line(fd, c"let &splitbelow = s:save_splitbelow")
                    || !put_line(fd, c"let &splitright = s:save_splitright"))
            {
                return false;
            }

            let mut nr: i32 = 0;
            let mut restore_size = true;
            wp = tab_firstwin;
            while let Some(wp_ref) = wp.opt_mut() {
                if ses_do_win(wp_ref) != 0 {
                    nr += 1;
                } else if !wp_ref.w_floating.get() {
                    restore_size = false;
                }
                if curwin_ptr() == wp {
                    cnr = nr;
                }
                wp = wp_ref.w_next.get();
            }

            if let Some(tfw) = tab_firstwin.opt()
                && !tfw.w_next.get().is_null()
            {
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
            if nr > 1 && !ses_winsizes(fd, restore_size, tab_firstwin.opt_mut()) {
                return false;
            }

            wp = tab_firstwin;
            while let Some(wp_ref) = wp.opt_mut() {
                if ses_do_win(wp_ref) == 0 {
                    wp = wp_ref.w_next.get();
                    continue;
                }
                if !put_view(
                    fd,
                    wp_ref,
                    tp.borrow(),
                    edited_win != wp,
                    &opt_vars().ssop_flags,
                    cur_arg_idx,
                ) {
                    return false;
                }
                if nr > 1 && !put_line(fd, c"wincmd w") {
                    return false;
                }
                next_arg_idx = wp_ref.w_arg_idx.get();
                wp = wp_ref.w_next.get();
            }

            cur_arg_idx = next_arg_idx;

            if cnr > 1 && !write_fmt(fd, format!("{cnr}wincmd w\n")) {
                return false;
            }

            if nr > 1 && !ses_winsizes(fd, restore_size, tab_firstwin.opt_mut()) {
                return false;
            }

            if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TABPAGES) == 0 {
                break;
            }
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_TABPAGES) != 0
            && !write_fmt(
                fd,
                format!("tabnext {}\n", tabpage_index(g_st().curtab.get().opt())),
            )
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

        let wh = opt_vars().p_wh.get();
        let wiw = opt_vars().p_wiw.get();
        if !write_fmt(fd, format!("set winheight={wh} winwidth={wiw}\n")) {
            return false;
        }

        if (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_OPTIONS) != 0 {
            if !write_fmt(
                fd,
                format!(
                    "set shortmess={}\n",
                    cstr_to_str(opt_vars().p_shm.as_cstr())
                ),
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
    let Some(fname) = get_view_file(eap.arg.first_byte()) else {
        return;
    };

    if !do_source(fname.as_c_str(), false, DOSO_NONE, None) {
        semsg!(gettext(E_NOTOPEN), &fname);
    }
}

pub fn ex_mkrc(eap: &mut Exarg) {
    let mut view_session = false;
    let mut using_vdir = false;
    let mut view_file: Option<CString> = None;

    if eap.cmdidx == Cmdidx::CmdMksession || eap.cmdidx == Cmdidx::CmdMkview {
        view_session = true;
    }

    DID_LCD.set(false);

    let fname: &CStr;
    if eap.cmdidx == Cmdidx::CmdMkview && {
        let arg_bytes = eap.arg.as_bytes();
        arg_bytes.is_empty() || (arg_bytes.len() == 1 && arg_bytes[0].is_ascii_digit())
    } {
        eap.forceit = true;
        let Some(vf) = get_view_file(eap.arg.first_byte()) else {
            return;
        };
        view_file = Some(vf);
        fname = view_file.as_deref().unwrap();
        using_vdir = true;
    } else if !eap.arg.is_empty() {
        fname = eap.arg.as_cstr();
    } else if eap.cmdidx == Cmdidx::CmdMkvimrc {
        fname = c".nvimrc";
    } else if eap.cmdidx == Cmdidx::CmdMksession {
        fname = c"Session.vim";
    } else {
        fname = c".exrc";
    }

    if using_vdir && !os_isdir(opt_vars().p_vdir.as_cstr()) {
        vim_mkdir_emsg(opt_vars().p_vdir.as_cstr(), 0o755);
    }

    if let Some(fd) = open_exfile(fname, eap.forceit, c"wb") {
        let mut failed = false;

        if eap.cmdidx == Cmdidx::CmdMksession {
            apply_autocmds(EVENT_SESSIONWRITEPRE, None, None, false, curbuf_opt_mut());
        }

        if eap.cmdidx == Cmdidx::CmdMkvimrc {
            put_line(&fd, c"version 6.0");
        }

        if eap.cmdidx == Cmdidx::CmdMksession && !put_line(&fd, c"let SessionLoad = 1") {
            failed = true;
        }

        if !view_session
            || (eap.cmdidx == Cmdidx::CmdMksession
                && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_OPTIONS) != 0)
        {
            let mut flags = OPT_GLOBAL;
            if eap.cmdidx == Cmdidx::CmdMksession
                && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_SKIPRTP) != 0
            {
                flags |= OPT_SKIPRTP;
            }
            failed |= !makemap(&fd, None) || !makeset(&fd, flags, 0);
        }

        if !failed && view_session {
            if !put_line(
                    &fd,
                    c"let s:so_save = &g:so | let s:siso_save = &g:siso | setg so=0 siso=0 | setl so=-1 siso=-1",
                )
                {
                    failed = true;
                }
            if eap.cmdidx == Cmdidx::CmdMksession {
                let mut dirnow_vec: Vec<u8> = vec![0; MAXPATHL];

                if !os_dirname(&mut dirnow_vec)
                    || os_chdir(crate::memory::bytes_as_cstr(&dirnow_vec)) != 0
                {
                    dirnow_vec[0] = 0;
                }
                let dirnow_cstr = crate::memory::bytes_as_cstr(&dirnow_vec);
                let has_dirnow = !dirnow_cstr.to_bytes().is_empty();
                if has_dirnow && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_SESDIR) != 0 {
                    if vim_chdirfile(fname, K_CD_CAUSE_OTHER) {
                        shorten_fnames(true);
                    }
                } else if has_dirnow
                    && (opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_CURDIR) != 0
                    && let Some(ref gdir) = g_st().globaldir.get()
                    && os_chdir(gdir) == 0
                {
                    shorten_fnames(true);
                }

                failed |= !makeopens(&fd, dirnow_cstr);

                if has_dirnow
                    && ((opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_SESDIR) != 0
                        || ((opt_vars().ssop_flags.get() & K_OPT_SSOP_FLAG_CURDIR) != 0
                            && g_st().globaldir.get().is_some()))
                {
                    if os_chdir(dirnow_cstr) != 0 {
                        emsg(gettext(E_PREV_DIR));
                    }
                    shorten_fnames(true);
                }
            } else {
                let flagp = if eap.cmdidx == Cmdidx::CmdMkview {
                    &opt_vars().vop_flags
                } else {
                    &opt_vars().ssop_flags
                };
                failed |= !put_view(&fd, curwin_mut(), &curtab_mut(), !using_vdir, flagp, -1);
            }
            if !write_bytes(&fd, b"let &g:so = s:so_save | let &g:siso = s:siso_save\n") {
                failed = true;
            }
            if opt_vars().p_hls.get() && !write_bytes(&fd, b"set hlsearch\n") {
                failed = true;
            }
            if g_st().search_global.no_hlsearch.get() && !write_bytes(&fd, b"nohlsearch\n") {
                failed = true;
            }
            if eap.cmdidx == Cmdidx::CmdMksession
                && !write_bytes(&fd, b"doautoall SessionLoadPost\nunlet SessionLoad\n")
            {
                failed = true;
            }
        }
        if !put_line(&fd, c"\" vim: set ft=vim :") {
            failed = true;
        }

        failed |= fd.close() != 0;

        if failed {
            emsg(gettext(E_WRITE));
        } else if eap.cmdidx == Cmdidx::CmdMksession {
            let mut tbuf: Vec<u8> = vec![0; MAXPATHL];
            if vim_full_name(Some(fname), &mut tbuf, false) {
                set_vim_var_string(
                    VV_THIS_SESSION,
                    Some(crate::memory::bytes_as_cstr(&tbuf)),
                    -1,
                );
            }
        }
    }

    drop(view_file);

    if eap.cmdidx == Cmdidx::CmdMksession {
        apply_autocmds(EVENT_SESSIONWRITEPOST, None, None, false, curbuf_opt_mut());
    }
}

fn get_view_file(c: u8) -> Option<CString> {
    if curbuf_mut().b_ffname.get().is_none() {
        emsg(gettext(E_NONAME));
        return None;
    }
    let sname_cstr = home_replace_save(None, curbuf_mut().ffname_opt());
    let sname_bytes = sname_cstr.as_bytes();

    let mut len: usize = 0;
    for &b in sname_bytes {
        if b == b'=' || vim_ispathsep(i32::from(b)) {
            len += 1;
        }
    }
    let sname_len = sname_bytes.len();
    let p_vdir = opt_vars().p_vdir.to_owned();
    let vdir_bytes = p_vdir.as_bytes_with_nul();
    let vdir_len = vdir_bytes.len() - 1;
    let mut retval_vec: Vec<u8> = vec![0; sname_len + len + vdir_len + 9];
    retval_vec[..vdir_len + 1].copy_from_slice(vdir_bytes);
    add_pathsep(&mut retval_vec);
    let mut s_idx = crate::memory::bytes_until_nul(&retval_vec).len();
    for &b in sname_bytes {
        if b == b'=' {
            retval_vec[s_idx] = b'=';
            retval_vec[s_idx + 1] = b'=';
            s_idx += 2;
        } else if vim_ispathsep(i32::from(b)) {
            retval_vec[s_idx] = b'=';
            #[cfg(windows)]
            {
                retval_vec[s_idx + 1] = if b == b':' { b'-' } else { b'+' };
            }
            #[cfg(not(windows))]
            {
                retval_vec[s_idx + 1] = b'+';
            }
            s_idx += 2;
        } else {
            retval_vec[s_idx] = b;
            s_idx += 1;
        }
    }
    retval_vec[s_idx] = b'=';
    retval_vec[s_idx + 1] = c;
    s_idx += 2;
    retval_vec[s_idx..s_idx + 5].copy_from_slice(b".vim\0");

    Some(crate::memory::cstring_until_nul(&retval_vec))
}

pub fn put_eol(fd: &CFile) -> bool {
    fd.putc(b'\n' as i32) >= 0
}

pub fn put_line(fd: &CFile, s: &std::ffi::CStr) -> bool {
    fd.puts(s) >= 0 && fd.putc(b'\n' as i32) >= 0
}
