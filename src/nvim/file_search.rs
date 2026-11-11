#![forbid(unsafe_code)]

//! File searching functions for `'path'`, `'tags'`, and `'cdpath'` options (`src/nvim/file_search.c`).

use crate::api::private::helpers::{api_clear_string, cbuf_to_string, copy_string, cstr_to_string};
use crate::api::private::validate::NvimString;
use crate::autocmd::{EVENT_DIRCHANGED, EVENT_DIRCHANGEDPRE, apply_autocmds, has_event};
use crate::charset::{
    NUL, getdigits, getdigits_int32, getdigits_long, skipwhite_slice, vim_isfilec,
};
use crate::context::{CdCause, K_CD_CAUSE_WINDOW};
use crate::cursor::Linenr;
use crate::errors::*;
use crate::eval::typval::{
    K_CALLBACK_NONE_CONST, TypVal, tv_clear, tv_dict_add_bool, tv_dict_add_str,
    tv_dict_set_keys_readonly, tv_get_string,
};
use crate::eval::vars::{VV_FNAME, set_vim_var_string};
use crate::eval::{eval_expr_option_tv, get_v_event, restore_v_event};
use crate::globals::*;
use crate::mbyte::{mb_tolower, utf_slice2char, utfc_slice2len};
use crate::message::{emsg, semsg};
use crate::normal::get_visual_text;
use crate::option::{OPT_LOCAL, copy_option_part, was_set_insecurely};
use crate::option_vars::*;
use crate::os::env::expand_env_esc;
use crate::os::fs::{
    FileID, MAXPATHL, os_chdir, os_dirname, os_fileid, os_fileid_equal, os_isdir, os_path_exists,
};
use crate::os::input::os_breakcheck;
use crate::path::{
    EW_ADDSLASH, EW_DIR, EW_NOTWILD, EW_SILENT, FullName_save, K_PATH_CMP_LITERAL, PATHSEP,
    after_pathsep, expand_wildcards_vec, path_cmp, path_equal, path_has_drive_letter, path_is_url,
    path_shorten_fname, path_tail, path_tail_with_sep, path_with_url, simplify_filename,
    vim_isAbsName, vim_ispathsep,
};
#[cfg(windows)]
use crate::path::{path_to_slash, vim_full_name};
use crate::state::SaveVEvent;
use crate::strings::{format_to_buf, tr, vim_strchr};
use std::ffi::CStr;

use crate::ex_docmd::{CdScope, K_CD_CAUSE_MANUAL};
use crate::optionstr::OptIndex;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum FindFileWhat {
    #[default]
    File,
    Dir,
    Both,
}

pub const FNAME_MESS: i32 = 1;
pub const FNAME_EXP: i32 = 2;
pub const FNAME_HYP: i32 = 4;
pub const FNAME_INCL: i32 = 8;
pub const FNAME_REL: i32 = 16;
pub const FNAME_UNESC: i32 = 32;

const PATHSEPSTR_C: &CStr = c"/";
const EMPTY_C: &CStr = c"";

const FF_MAX_STAR_STAR_EXPAND: u8 = 30;

const K_CPO_DOTTAG: i32 = b'd' as i32;

pub(crate) const K_OPT_INCLUDEEXPR: OptIndex = 147;
pub(crate) const K_BUF_OPT_INCLUDEEXPR: usize = 47;

pub(crate) const NULL_STRING: NvimString = NvimString::new();

const E_PATH_TOO_LONG_FOR_COMPLETION: &CStr = c"E854: Path too long for completion";

use crate::api::vim::K_CD_SCOPE_GLOBAL;
use crate::buffer::{K_CD_CAUSE_AUTO, K_CD_CAUSE_BUFFER};
use crate::eval::fs::{K_CD_SCOPE_BUFFER, K_CD_SCOPE_TABPAGE, K_CD_SCOPE_WINDOW};
use crate::ex_session::K_CD_CAUSE_OTHER;

#[repr(C)]
pub struct FfStack {
    pub ffs_fix_path: NvimString,
    pub ffs_wc_path: NvimString,
    pub ffs_filearray: Vec<std::ffi::CString>,
    pub ffs_filearray_cur: usize,
    pub ffs_stage: i32,
    pub ffs_level: i32,
    pub ffs_star_star_empty: bool,
}

impl Drop for FfStack {
    fn drop(&mut self) {
        api_clear_string(&mut self.ffs_fix_path);
        api_clear_string(&mut self.ffs_wc_path);
    }
}

#[repr(C)]
pub struct FfVisited {
    pub ffv_wc_path: Option<std::ffi::CString>,
    pub file_id_valid: bool,
    pub file_id: FileID,
    pub ffv_fname: Vec<u8>,
}

#[repr(C)]
pub struct FfVisitedListHdr {
    pub ffvl_filename: std::ffi::CString,
    pub ffvl_visited_list: Vec<FfVisited>,
}

#[repr(C)]
pub struct FfSearchCtx {
    pub ffsc_stack: Vec<FfStack>,
    pub ffsc_visited_list: Option<usize>,
    pub ffsc_dir_visited_list: Option<usize>,
    pub ffsc_visited_lists_list: Vec<FfVisitedListHdr>,
    pub ffsc_dir_visited_lists_list: Vec<FfVisitedListHdr>,
    pub ffsc_file_to_search: NvimString,
    pub ffsc_start_dir: NvimString,
    pub ffsc_fix_path: NvimString,
    pub ffsc_wc_path: NvimString,
    pub ffsc_level: i32,
    pub ffsc_stopdirs_v: Vec<std::ffi::CString>,
    pub ffsc_find_what: FindFileWhat,
    pub ffsc_tagfile: bool,
}

impl FfSearchCtx {
    pub const fn new() -> Self {
        Self {
            ffsc_stack: Vec::new(),
            ffsc_visited_list: None,
            ffsc_dir_visited_list: None,
            ffsc_visited_lists_list: Vec::new(),
            ffsc_dir_visited_lists_list: Vec::new(),
            ffsc_file_to_search: NULL_STRING,
            ffsc_start_dir: NULL_STRING,
            ffsc_fix_path: NULL_STRING,
            ffsc_wc_path: NULL_STRING,
            ffsc_level: 0,
            ffsc_stopdirs_v: Vec::new(),
            ffsc_find_what: FindFileWhat::File,
            ffsc_tagfile: false,
        }
    }
}

impl Default for FfSearchCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FfSearchCtx {
    fn drop(&mut self) {
        ff_clear(self);
    }
}

/// Initialization routine for `vim_findfile()`.
pub fn vim_findfile_init(
    path_cstr: &std::ffi::CStr,
    fn_slice: &[u8],
    stopdirs: Option<&std::ffi::CStr>,
    level: i32,
    free_visited: bool,
    find_what: FindFileWhat,
    search_ctx_arg: Option<FfSearchCtx>,
    tagfile: bool,
    rel_fname: Option<&std::ffi::CStr>,
) -> Option<FfSearchCtx> {
    let mut path = path_cstr;
    let fn_slice = crate::memory::bytes_until_nul(fn_slice);
    {
        let mut search_ctx = search_ctx_arg.unwrap_or_default();
        search_ctx.ffsc_find_what = find_what;
        search_ctx.ffsc_tagfile = tagfile;

        ff_clear(&mut search_ctx);

        if free_visited {
            vim_findfile_free_visited(Some(&mut search_ctx));
        } else {
            search_ctx.ffsc_visited_list = Some(ff_get_visited_list(
                fn_slice,
                &mut search_ctx.ffsc_visited_lists_list,
            ));
            search_ctx.ffsc_dir_visited_list = Some(ff_get_visited_list(
                fn_slice,
                &mut search_ctx.ffsc_dir_visited_lists_list,
            ));
        }

        let mut expand_buf: [u8; MAXPATHL] = [0; MAXPATHL];
        let path_bytes = path.to_bytes();

        if path_bytes.first() == Some(&b'.')
            && (path_bytes
                .get(1)
                .is_some_and(|&b| vim_ispathsep(i32::from(b)))
                || path_bytes.len() == 1)
            && (!tagfile || !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_DOTTAG))
            && let Some(rel_fname_cstr) = rel_fname
        {
            let len =
                path_tail(rel_fname_cstr).as_ptr() as usize - rel_fname_cstr.as_ptr() as usize;
            let rel_bytes = rel_fname_cstr.to_bytes();

            if !vim_isAbsName(rel_bytes) && len + 1 < MAXPATHL {
                expand_buf[..len].copy_from_slice(&rel_bytes[..len]);
                expand_buf[len] = 0;
                let expand_cstr = crate::memory::bytes_as_cstr(&expand_buf);
                let full = FullName_save(Some(expand_cstr), false).unwrap_or_default();
                search_ctx.ffsc_start_dir = cbuf_to_string(full.as_bytes());
            } else {
                search_ctx.ffsc_start_dir = cbuf_to_string(&rel_bytes[..len]);
            }
            path = &path[1 + usize::from(path_bytes.len() > 1)..];
        } else if path_bytes.is_empty() || !vim_isAbsName(path.to_bytes()) {
            #[cfg(windows)]
            let handled_drive = if path_bytes.get(1) == Some(&b':') {
                let drive: [u8; 3] = [path_bytes[0], b':', 0];
                if !vim_full_name(
                    Some(crate::memory::bytes_as_cstr(&drive)),
                    &mut expand_buf,
                    true,
                ) {
                    return None;
                }
                path = &path[2..];
                true
            } else {
                false
            };
            #[cfg(not(windows))]
            let handled_drive = false;

            if !handled_drive && !os_dirname(&mut expand_buf) {
                return None;
            }
            let expand_len = crate::memory::bytes_until_nul(&expand_buf).len();

            search_ctx.ffsc_start_dir =
                copy_string(NvimString::from_bytes_len(&expand_buf, expand_len), None);
        }

        if let Some(stopdirs_cstr) = stopdirs {
            let mut walker = stopdirs_cstr;
            while walker.to_bytes().first() == Some(&b';') {
                walker = &walker[1..];
            }

            loop {
                let helper = walker;
                let next_semi = vim_strchr(walker, b';' as i32);
                let helper_bytes = helper.to_bytes();
                let len = if let Some(semi) = next_semi {
                    helper_bytes.len() - semi.to_bytes().len()
                } else {
                    helper_bytes.len()
                };
                let dir_str = if !helper_bytes.is_empty()
                    && !vim_isAbsName(helper_bytes)
                    && len + 1 < MAXPATHL
                {
                    expand_buf[..len].copy_from_slice(&helper_bytes[..len]);
                    expand_buf[len] = 0;
                    FullName_save(Some(helper), len != 0).unwrap_or_default()
                } else {
                    std::ffi::CString::new(&helper_bytes[..len]).unwrap_or_default()
                };
                search_ctx.ffsc_stopdirs_v.push(dir_str);
                let Some(semi) = next_semi else {
                    break;
                };
                walker = &semi[1..];
            }
        }

        search_ctx.ffsc_level = level;

        let path_bytes = path.to_bytes();
        let mut expand_size: usize;
        if let Some(star_pos) = path_bytes.iter().position(|&b| b == b'*') {
            search_ctx.ffsc_fix_path = cbuf_to_string(&path_bytes[..star_pos]);

            expand_size = 0;
            let mut wc_rem = &path_bytes[star_pos..];
            while !wc_rem.is_empty() {
                if expand_size + 5 >= MAXPATHL {
                    emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION));
                    break;
                }
                if wc_rem.starts_with(b"**") {
                    expand_buf[expand_size] = b'*';
                    expand_size += 1;
                    expand_buf[expand_size] = b'*';
                    expand_size += 1;
                    wc_rem = &wc_rem[2..];

                    let before_len = wc_rem.len();
                    let llevel = getdigits(&mut wc_rem, false, 255);
                    let parsed_digits = wc_rem.len() != before_len;
                    if parsed_digits && llevel > 0 && llevel < 255 {
                        expand_buf[expand_size] = llevel as u8;
                        expand_size += 1;
                    } else if parsed_digits && llevel == 0 {
                        expand_size -= 2;
                    } else {
                        expand_buf[expand_size] = FF_MAX_STAR_STAR_EXPAND;
                        expand_size += 1;
                    }
                    if !wc_rem.is_empty() && !vim_ispathsep(i32::from(wc_rem[0])) {
                        semsg!(
                            tr(c"E343: Invalid path: '**[number]' must be at the end of the path or be followed by '%s'."),
                            PATHSEPSTR_C,
                        );
                        return None;
                    }
                } else {
                    expand_buf[expand_size] = wc_rem[0];
                    expand_size += 1;
                    wc_rem = &wc_rem[1..];
                }
            }
            expand_buf[expand_size] = NUL as u8;
            search_ctx.ffsc_wc_path =
                copy_string(NvimString::from_bytes_len(&expand_buf, expand_size), None);
        } else {
            search_ctx.ffsc_fix_path = cstr_to_string(Some(path));
        }

        if search_ctx.ffsc_start_dir.data.is_null() {
            search_ctx.ffsc_start_dir = copy_string(search_ctx.ffsc_fix_path.clone(), None);
            if let Some(first) = search_ctx.ffsc_fix_path.as_bytes_with_nul_mut().first_mut() {
                *first = 0;
            }
            search_ctx.ffsc_fix_path.size = 0;
        }

        if search_ctx.ffsc_start_dir.size + search_ctx.ffsc_fix_path.size + 3 >= MAXPATHL {
            emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION));
            return None;
        }

        let mut add_sep = after_pathsep(search_ctx.ffsc_start_dir.cstr_bytes()) == 0;
        expand_size = format_to_buf!(
            &mut expand_buf,
            c"%s%s",
            search_ctx.ffsc_start_dir,
            if add_sep { PATHSEPSTR_C } else { EMPTY_C },
        ) as usize;

        {
            let bufsize = expand_size + search_ctx.ffsc_fix_path.size + 1;
            let mut buf_vec = vec![0u8; bufsize];

            format_to_buf!(
                &mut buf_vec,
                c"%s%s",
                crate::memory::bytes_as_cstr(&expand_buf),
                search_ctx.ffsc_fix_path,
            );
            if os_isdir(crate::memory::bytes_as_cstr(&buf_vec)) {
                if search_ctx.ffsc_fix_path.size > 0 {
                    add_sep = after_pathsep(search_ctx.ffsc_fix_path.cstr_bytes()) == 0;
                    expand_size += format_to_buf!(
                        &mut expand_buf[expand_size..],
                        c"%s%s",
                        search_ctx.ffsc_fix_path,
                        if add_sep { PATHSEPSTR_C } else { EMPTY_C },
                    ) as usize;
                }
            } else {
                let fix_cstr = search_ctx.ffsc_fix_path.as_cstr();
                let tail_off = fix_cstr.to_bytes().len() - path_tail(fix_cstr).to_bytes().len();
                let mut len = search_ctx.ffsc_fix_path.size as i32;

                if tail_off > 0 {
                    len = (tail_off - 1) as i32;
                    let fix_bytes = search_ctx.ffsc_fix_path.as_bytes();
                    if len >= 2
                        && fix_bytes.starts_with(b"..")
                        && (len == 2 || fix_bytes.get(2) == Some(&PATHSEP))
                    {
                        return None;
                    }

                    add_sep = after_pathsep(search_ctx.ffsc_fix_path.cstr_bytes()) == 0;
                    expand_size += format_to_buf!(
                        &mut expand_buf[expand_size..],
                        c"%.*s%s",
                        len,
                        search_ctx.ffsc_fix_path,
                        if add_sep { PATHSEPSTR_C } else { EMPTY_C },
                    ) as usize;
                }

                if !search_ctx.ffsc_wc_path.data.is_null() {
                    let tempsize = (search_ctx.ffsc_fix_path.size - len as usize)
                        + search_ctx.ffsc_wc_path.size
                        + 1;
                    let mut temp_vec: Vec<u8> = vec![0; tempsize];
                    let temp_len = format_to_buf!(
                        &mut temp_vec,
                        c"%s%s",
                        &search_ctx.ffsc_fix_path.as_cstr()[len as usize..],
                        search_ctx.ffsc_wc_path,
                    ) as usize;
                    api_clear_string(&mut search_ctx.ffsc_wc_path);
                    search_ctx.ffsc_wc_path = cbuf_to_string(&temp_vec[..temp_len]);
                }
            }
        }

        let sptr = ff_create_stack_element(
            &expand_buf[..expand_size],
            search_ctx.ffsc_wc_path.as_bytes(),
            level,
            false,
        );

        ff_push(&mut search_ctx, sptr);
        search_ctx.ffsc_file_to_search = cbuf_to_string(fn_slice);
        Some(search_ctx)
    }
}

/// Return the stopdir byte offset in `s`. Check that `;` is not escaped.
pub fn vim_findfile_stopdir(s: &mut [u8]) -> Option<usize> {
    let len = crate::memory::bytes_until_nul(s).len();
    let max_idx = len.min(s.len().saturating_sub(1));
    let s = &mut s[..=max_idx];
    let mut i = 0;
    while s[i] != 0 && s[i] != b';' && (s[i] != b'\\' || s[i + 1] != b';') {
        i += 1;
    }
    let mut dst = i;
    if s[i] == b';' {
        s[i] = 0;
        return Some(i + 1);
    }
    if s[i] == 0 {
        return None;
    }
    s[dst] = b';';
    dst += 1;
    i += 2;
    while s[i] != 0 && s[i] != b';' {
        if s[i] == b'\\' && s[i + 1] == b';' {
            s[dst] = b';';
            dst += 1;
            i += 2;
        } else {
            s[dst] = s[i];
            dst += 1;
            i += 1;
        }
    }
    debug_assert!(dst < i);
    s[dst] = 0;
    if s[i] == b';' {
        s[i] = 0;
        Some(i + 1)
    } else {
        None
    }
}

/// Find a file in a search context.
pub fn vim_findfile(search_ctx: Option<&mut FfSearchCtx>) -> Option<std::ffi::CString> {
    let search_ctx = search_ctx?;

    let mut file_path_vec: Vec<u8> = vec![0; MAXPATHL];
    let mut file_path_len: usize;
    let mut path_end_off = search_ctx.ffsc_start_dir.size;

    'upward: loop {
        loop {
            os_breakcheck();
            if g_st().got_int.get() {
                break;
            }

            let Some(mut stack_elem) = ff_pop(search_ctx) else {
                break;
            };
            let stackp = &mut stack_elem;
            if stackp.ffs_filearray.is_empty() {
                let dir_vis_idx = search_ctx.ffsc_dir_visited_list.unwrap();
                let wc_opt =
                    (!stackp.ffs_wc_path.data.is_null()).then(|| stackp.ffs_wc_path.as_bytes());
                if !ff_check_visited(
                    &mut search_ctx.ffsc_dir_visited_lists_list[dir_vis_idx].ffvl_visited_list,
                    stackp.ffs_fix_path.as_bytes(),
                    wc_opt,
                ) {
                    continue;
                }
            }

            if stackp.ffs_level <= 0 {
                continue;
            }

            file_path_vec[0] = NUL as u8;
            file_path_len = 0;

            let mut rest_off: usize = 0;
            if stackp.ffs_filearray.is_empty() {
                let mut has_dir1 = false;

                if !vim_isAbsName(stackp.ffs_fix_path.cstr_bytes())
                    && !search_ctx.ffsc_start_dir.data.is_null()
                {
                    if search_ctx.ffsc_start_dir.size + 1 >= MAXPATHL {
                        break 'upward;
                    }
                    let add_sep = after_pathsep(search_ctx.ffsc_start_dir.cstr_bytes()) == 0;
                    file_path_len = format_to_buf!(
                        &mut file_path_vec,
                        c"%s%s",
                        search_ctx.ffsc_start_dir,
                        if add_sep { PATHSEPSTR_C } else { EMPTY_C },
                    ) as usize;
                    if file_path_len >= MAXPATHL {
                        break 'upward;
                    }
                }

                if file_path_len + stackp.ffs_fix_path.size + 1 >= MAXPATHL {
                    break 'upward;
                }
                let add_sep = after_pathsep(stackp.ffs_fix_path.cstr_bytes()) == 0;
                file_path_len += format_to_buf!(
                    &mut file_path_vec[file_path_len..],
                    c"%s%s",
                    stackp.ffs_fix_path,
                    if add_sep { PATHSEPSTR_C } else { EMPTY_C },
                ) as usize;
                if file_path_len >= MAXPATHL {
                    break 'upward;
                }

                if !stackp.ffs_wc_path.cstr_bytes().is_empty() {
                    if stackp.ffs_wc_path.as_bytes().starts_with(b"**") {
                        let wc_size = stackp.ffs_wc_path.size;
                        let wc_slice = stackp.ffs_wc_path.as_bytes_with_nul_mut();

                        if wc_slice[2] > 0 {
                            wc_slice[2] -= 1;
                            if file_path_len + 1 >= MAXPATHL {
                                break 'upward;
                            }
                            file_path_vec[file_path_len] = b'*';
                            file_path_len += 1;
                        }

                        if wc_slice[2] == 0 {
                            wc_slice.copy_within(3..=wc_size, 0);
                            stackp.ffs_wc_path.size -= 3;
                        } else {
                            rest_off = 3;
                        }

                        if !stackp.ffs_star_star_empty {
                            stackp.ffs_star_star_empty = true;
                            has_dir1 = true;
                        }
                    }

                    let wc_bytes = stackp.ffs_wc_path.cstr_bytes();
                    while rest_off < wc_bytes.len() && !vim_ispathsep(i32::from(wc_bytes[rest_off]))
                    {
                        if file_path_len + 1 >= MAXPATHL {
                            break 'upward;
                        }
                        file_path_vec[file_path_len] = wc_bytes[rest_off];
                        file_path_len += 1;
                        rest_off += 1;
                    }

                    file_path_vec[file_path_len] = NUL as u8;
                    if rest_off < wc_bytes.len() && vim_ispathsep(i32::from(wc_bytes[rest_off])) {
                        rest_off += 1;
                    }
                }

                let dir0_cstr = crate::memory::bytes_as_cstr(&file_path_vec);
                let dir0_bytes = dir0_cstr.to_bytes();
                if path_with_url(dir0_bytes) {
                    let dir_len = file_path_len.min(dir0_bytes.len());
                    stackp.ffs_filearray =
                        vec![crate::memory::bytes_to_cstring(&dir0_bytes[..dir_len])];
                } else {
                    let mut dir_cstrs: Vec<&std::ffi::CStr> = vec![dir0_cstr];
                    if has_dir1 {
                        dir_cstrs.push(stackp.ffs_fix_path.as_cstr());
                    }
                    expand_wildcards_vec(
                        &dir_cstrs,
                        &mut stackp.ffs_filearray,
                        EW_DIR | EW_ADDSLASH | EW_SILENT | EW_NOTWILD,
                    );
                }

                stackp.ffs_filearray_cur = 0;
                stackp.ffs_stage = 0;
            } else {
                rest_off = stackp.ffs_wc_path.cstr_bytes().len();
            }

            if stackp.ffs_stage == 0 {
                let rest_wc_bytes = &stackp.ffs_wc_path.cstr_bytes()[rest_off..];
                if rest_wc_bytes.is_empty() {
                    for i in stackp.ffs_filearray_cur..stackp.ffs_filearray.len() {
                        let entry_cstr = stackp.ffs_filearray.as_slice()[i].as_c_str();
                        let entry_bytes = entry_cstr.to_bytes();
                        if !path_with_url(entry_bytes) && !os_isdir(entry_cstr) {
                            continue;
                        }
                        let mut len = entry_bytes.len();
                        if len + 1 + search_ctx.ffsc_file_to_search.size >= MAXPATHL {
                            break 'upward;
                        }
                        let add_sep = after_pathsep(entry_bytes) == 0;
                        file_path_len = format_to_buf!(
                            &mut file_path_vec,
                            c"%s%s%s",
                            entry_cstr,
                            if add_sep { PATHSEPSTR_C } else { EMPTY_C },
                            search_ctx.ffsc_file_to_search,
                        ) as usize;
                        if file_path_len >= MAXPATHL {
                            break 'upward;
                        }

                        len = file_path_len;
                        let mut suf_bytes = if search_ctx.ffsc_tagfile {
                            EMPTY_C.to_bytes()
                        } else {
                            curbuf_mut().b_p_sua.as_bytes()
                        };
                        loop {
                            let vis_idx = search_ctx.ffsc_visited_list.unwrap();
                            let fp_cstr =
                                crate::memory::bytes_as_cstr(&file_path_vec[..=file_path_len]);
                            let fp_cstr_bytes = fp_cstr.to_bytes();
                            if (path_with_url(fp_cstr_bytes)
                                || (os_path_exists(fp_cstr)
                                    && (search_ctx.ffsc_find_what == FindFileWhat::Both
                                        || ((search_ctx.ffsc_find_what == FindFileWhat::Dir)
                                            == os_isdir(fp_cstr)))))
                                && ff_check_visited(
                                    &mut search_ctx.ffsc_visited_lists_list[vis_idx]
                                        .ffvl_visited_list,
                                    &file_path_vec[..file_path_len],
                                    Some(b""),
                                )
                            {
                                stackp.ffs_filearray_cur = i + 1;
                                ff_push(search_ctx, stack_elem);

                                let fp_cstr_bytes =
                                    crate::memory::bytes_as_cstr(&file_path_vec[..=file_path_len])
                                        .to_bytes();
                                if !path_with_url(fp_cstr_bytes) {
                                    file_path_len =
                                        simplify_filename(&mut file_path_vec[..=file_path_len]);
                                }

                                let mut expand_buf: [u8; MAXPATHL] = [0; MAXPATHL];
                                let fp_cstr =
                                    crate::memory::bytes_as_cstr(&file_path_vec[..=file_path_len]);
                                if os_dirname(&mut expand_buf)
                                    && let Some(p) = path_shorten_fname(
                                        Some(fp_cstr),
                                        crate::memory::bytes_as_cstr(&expand_buf),
                                    )
                                {
                                    let new_len = p.to_bytes().len();
                                    let p_off = file_path_len - new_len;
                                    file_path_vec.copy_within(p_off..=file_path_len, 0);
                                    file_path_len = new_len;
                                }
                                return Some(crate::memory::bytes_to_cstring(
                                    &file_path_vec[..file_path_len],
                                ));
                            }

                            if suf_bytes.is_empty() {
                                break;
                            }
                            let dst = &mut file_path_vec[len..MAXPATHL];
                            file_path_len = len + copy_option_part(&mut suf_bytes, dst, c",");
                        }
                    }
                } else {
                    for i in stackp.ffs_filearray_cur..stackp.ffs_filearray.len() {
                        let entry_cstr = &stackp.ffs_filearray.as_slice()[i];
                        if !os_isdir(entry_cstr) {
                            continue;
                        }
                        ff_push(
                            search_ctx,
                            ff_create_stack_element(
                                entry_cstr.as_bytes(),
                                rest_wc_bytes,
                                stackp.ffs_level - 1,
                                false,
                            ),
                        );
                    }
                }
                stackp.ffs_filearray_cur = 0;
                stackp.ffs_stage = 1;
            }

            if stackp.ffs_wc_path.as_bytes().starts_with(b"**") {
                for i in stackp.ffs_filearray_cur..stackp.ffs_filearray.len() {
                    let entry_cstr = &stackp.ffs_filearray.as_slice()[i];
                    if path_equal(
                        entry_cstr,
                        stackp.ffs_fix_path.as_cstr(),
                        K_PATH_CMP_LITERAL,
                    ) {
                        continue;
                    }
                    if !os_isdir(entry_cstr) {
                        continue;
                    }
                    ff_push(
                        search_ctx,
                        ff_create_stack_element(
                            entry_cstr.as_bytes(),
                            stackp.ffs_wc_path.as_bytes(),
                            stackp.ffs_level - 1,
                            true,
                        ),
                    );
                }
            }
        }

        if !search_ctx.ffsc_start_dir.data.is_null()
            && !search_ctx.ffsc_stopdirs_v.is_empty()
            && !g_st().got_int.get()
        {
            let start_dir_bytes = search_ctx.ffsc_start_dir.as_bytes_with_nul_mut();
            let plen = path_end_off
                + usize::from(start_dir_bytes.get(path_end_off).copied().unwrap_or(0) != 0);

            if ff_path_in_stoplist(
                search_ctx.ffsc_start_dir.slice(plen),
                &search_ctx.ffsc_stopdirs_v,
            ) {
                break;
            }

            let start_dir_bytes = search_ctx.ffsc_start_dir.as_bytes_with_nul_mut();
            while path_end_off > 0 && vim_ispathsep(i32::from(start_dir_bytes[path_end_off])) {
                path_end_off -= 1;
            }
            while path_end_off > 0 && !vim_ispathsep(i32::from(start_dir_bytes[path_end_off - 1])) {
                path_end_off -= 1;
            }
            start_dir_bytes[path_end_off] = 0;

            search_ctx.ffsc_start_dir.size = path_end_off;
            path_end_off = path_end_off.saturating_sub(1);

            if search_ctx.ffsc_start_dir.cstr_bytes().is_empty() {
                break;
            }

            if search_ctx.ffsc_start_dir.size + 1 + search_ctx.ffsc_fix_path.size >= MAXPATHL {
                break;
            }
            let add_sep = after_pathsep(search_ctx.ffsc_start_dir.cstr_bytes()) == 0;
            file_path_len = format_to_buf!(
                &mut file_path_vec,
                c"%s%s%s",
                search_ctx.ffsc_start_dir,
                if add_sep { PATHSEPSTR_C } else { EMPTY_C },
                search_ctx.ffsc_fix_path,
            ) as usize;
            if file_path_len >= MAXPATHL {
                break;
            }

            let sptr = ff_create_stack_element(
                &file_path_vec[..file_path_len],
                search_ctx.ffsc_wc_path.as_bytes(),
                search_ctx.ffsc_level,
                false,
            );
            ff_push(search_ctx, sptr);
        } else {
            break;
        }
    }

    None
}

/// Free the list of lists of visited files and directories.
fn vim_findfile_free_visited(search_ctx: Option<&mut FfSearchCtx>) {
    let Some(search_ctx) = search_ctx else {
        return;
    };

    search_ctx.ffsc_visited_lists_list.clear();
    search_ctx.ffsc_dir_visited_lists_list.clear();
}

fn ff_get_visited_list(filename: &[u8], list_headp: &mut Vec<FfVisitedListHdr>) -> usize {
    let fn_cstr = crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(filename));
    if let Some(pos) = list_headp
        .iter()
        .position(|item| path_equal(&fn_cstr, &item.ffvl_filename, K_PATH_CMP_LITERAL))
    {
        return pos;
    }

    list_headp.push(FfVisitedListHdr {
        ffvl_filename: fn_cstr,
        ffvl_visited_list: Vec::new(),
    });
    list_headp.len() - 1
}

/// Check if two wildcard paths are equal.
fn ff_wc_equal(s1: Option<&[u8]>, s2: Option<&[u8]>) -> bool {
    let (Some(s1), Some(s2)) = (s1, s2) else {
        return s1.is_none() && s2.is_none();
    };
    let mut b1 = crate::memory::bytes_until_nul(s1);
    let mut b2 = crate::memory::bytes_until_nul(s2);
    let mut prev1: i32 = 0;
    let mut prev2: i32 = 0;

    while !b1.is_empty() && !b2.is_empty() {
        let c1 = utf_slice2char(b1);
        let c2 = utf_slice2char(b2);

        let neq = if opt_vars().p_fic.get() {
            mb_tolower(c1) != mb_tolower(c2)
        } else {
            c1 != c2
        };
        if neq && (prev1 != b'*' as i32 || prev2 != b'*' as i32) {
            return false;
        }
        prev2 = prev1;
        prev1 = c1;

        b1 = &b1[utfc_slice2len(b1).min(b1.len())..];
        b2 = &b2[utfc_slice2len(b2).min(b2.len())..];
    }
    b1.is_empty() && b2.is_empty()
}

/// Maintain the list of already visited files and dirs.
fn ff_check_visited(
    visited_list: &mut Vec<FfVisited>,
    fname: &[u8],
    wc_path: Option<&[u8]>,
) -> bool {
    let fname_bytes = crate::memory::bytes_until_nul(fname);
    let fname_cstr = crate::memory::bytes_to_cstring(fname_bytes);
    let mut url = false;
    let mut file_id = FileID::new();
    let mut expand_buf: [u8; MAXPATHL] = [0; MAXPATHL];
    let mut expand_size: usize = 0;

    if path_with_url(fname_cstr.to_bytes()) {
        expand_size = crate::memory::copy_bytes_to_c_buf(&mut expand_buf, fname_bytes);
        url = true;
    } else if !os_fileid(&fname_cstr, &mut file_id) {
        return false;
    }

    for vp in &*visited_list {
        if ((url
            && path_equal(
                crate::memory::bytes_as_cstr(&vp.ffv_fname),
                crate::memory::bytes_as_cstr(&expand_buf),
                K_PATH_CMP_LITERAL,
            ))
            || (!url && vp.file_id_valid && os_fileid_equal(&vp.file_id, &file_id)))
            && ff_wc_equal(vp.ffv_wc_path.as_deref().map(CStr::to_bytes), wc_path)
        {
            return false;
        }
    }

    let mut ffv_fname = Vec::new();
    if !url {
        ffv_fname.push(0);
    } else {
        ffv_fname.extend_from_slice(&expand_buf[..expand_size + 1]);
    }

    let ffv_wc_path =
        wc_path.map(|bytes| crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(bytes)));

    visited_list.push(FfVisited {
        ffv_wc_path,
        file_id_valid: !url,
        file_id,
        ffv_fname,
    });

    true
}

fn ff_create_stack_element(
    fix_part: &[u8],
    wc_part: &[u8],
    level: i32,
    star_star_empty: bool,
) -> FfStack {
    let fix_bytes = crate::memory::bytes_until_nul(fix_part);
    let wc_bytes = crate::memory::bytes_until_nul(wc_part);
    let ffs_fix_path = cbuf_to_string(fix_bytes);
    let ffs_wc_path = cbuf_to_string(wc_bytes);

    FfStack {
        ffs_fix_path,
        ffs_wc_path,
        ffs_filearray: Vec::new(),
        ffs_filearray_cur: 0,
        ffs_stage: 0,
        ffs_level: level,
        ffs_star_star_empty: star_star_empty,
    }
}

fn ff_push(search_ctx: &mut FfSearchCtx, stack_elem: FfStack) {
    search_ctx.ffsc_stack.push(stack_elem);
}

fn ff_pop(search_ctx: &mut FfSearchCtx) -> Option<FfStack> {
    search_ctx.ffsc_stack.pop()
}

fn ff_clear(search_ctx: &mut FfSearchCtx) {
    search_ctx.ffsc_stack.clear();
    search_ctx.ffsc_stopdirs_v.clear();

    api_clear_string(&mut search_ctx.ffsc_file_to_search);
    api_clear_string(&mut search_ctx.ffsc_start_dir);
    api_clear_string(&mut search_ctx.ffsc_fix_path);
    api_clear_string(&mut search_ctx.ffsc_wc_path);
    search_ctx.ffsc_level = 0;
}

fn ff_path_in_stoplist(path: &[u8], stopdirs_v: &[std::ffi::CString]) -> bool {
    let path_slice = crate::memory::bytes_until_nul(path);
    let mut path_len = path_slice.len();
    while path_len > 1 && vim_ispathsep(i32::from(path_slice[path_len - 1])) {
        path_len -= 1;
    }

    if path_len == 0 {
        return true;
    }

    let path_cstr = crate::memory::bytes_to_cstring(&path_slice[..path_len]);
    for stopdir in stopdirs_v {
        let bytes = stopdir.to_bytes();
        if path_cmp(opt_vars().p_fic.get(), stopdir, &path_cstr, path_len) == 0
            && (bytes.len() <= path_len || vim_ispathsep(i32::from(bytes[path_len])))
        {
            return true;
        }
    }

    false
}

/// Find the file name `ptr` in the path. Also finds directory names.
pub fn find_file_in_path(
    ptr: &[u8],
    options: i32,
    first: bool,
    rel_fname: Option<&std::ffi::CStr>,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<FfSearchCtx>,
) -> Option<std::ffi::CString> {
    let path_opt = if curbuf_mut().b_p_path.is_empty() {
        opt_vars().p_path.to_owned()
    } else {
        curbuf_mut().b_p_path.to_owned()
    };
    let sua_opt = curbuf_mut().b_p_sua.to_owned();
    find_file_in_path_option(
        ptr,
        options,
        first,
        &path_opt,
        FindFileWhat::Both,
        rel_fname,
        &sua_opt,
        file_to_find,
        search_ctx,
    )
}

pub fn free_findfile() {}

/// Find the directory name `ptr` in the path.
pub fn find_directory_in_path(
    ptr: &[u8],
    options: i32,
    rel_fname: Option<&std::ffi::CStr>,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<FfSearchCtx>,
) -> Option<std::ffi::CString> {
    find_file_in_path_option(
        ptr,
        options,
        true,
        opt_vars().p_cdpath.as_cstr(),
        FindFileWhat::Dir,
        rel_fname,
        EMPTY_C,
        file_to_find,
        search_ctx,
    )
}

pub fn find_file_in_path_option(
    ptr_slice: &[u8],
    options: i32,
    first: bool,
    path_option_cstr: &std::ffi::CStr,
    find_what: FindFileWhat,
    mut rel_fname_opt: Option<&std::ffi::CStr>,
    suffixes_cstr: &std::ffi::CStr,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<FfSearchCtx>,
) -> Option<std::ffi::CString> {
    let ptr_slice = crate::memory::bytes_until_nul(ptr_slice);
    let len = ptr_slice.len();
    {
        struct FindFilePathOptState {
            dir: Vec<u8>,
            dir_off: usize,
            did_findfile_init: bool,
            file_to_findlen: usize,
        }
        std::thread_local! {
            static FF_OPT_STATE: std::cell::RefCell<FindFilePathOptState> =
                const {
                    std::cell::RefCell::new(FindFilePathOptState {
                        dir: Vec::new(),
                        dir_off: 0,
                        did_findfile_init: false,
                        file_to_findlen: 0,
                    })
                };
        }
        let mut ff_opt = FF_OPT_STATE.with(|cell| {
            std::mem::replace(
                &mut *cell.borrow_mut(),
                FindFilePathOptState {
                    dir: Vec::new(),
                    dir_off: 0,
                    did_findfile_init: false,
                    file_to_findlen: 0,
                },
            )
        });
        let mut file_name: Option<std::ffi::CString> = None;

        if rel_fname_opt.is_some_and(|rf| path_with_url(rf.to_bytes())) {
            rel_fname_opt = None;
        }

        if first {
            if len == 0 {
                FF_OPT_STATE.with(|cell| *cell.borrow_mut() = ff_opt);
                return None;
            }

            let ptr_cstr = std::ffi::CString::new(ptr_slice).unwrap_or_default();
            ff_opt.file_to_findlen = expand_env_esc(
                ptr_cstr.as_c_str(),
                &mut *g_st().name_buff_bytes_mut(),
                None,
                true,
                None,
            );

            let mut ftf_bytes = g_st().name_buff_bytes_mut()[..ff_opt.file_to_findlen].to_vec();
            if (options & FNAME_UNESC) != 0 {
                let mut i = 0;
                while i + 1 < ftf_bytes.len() {
                    if ftf_bytes[i] == b'\\' && ftf_bytes[i + 1] == b' ' {
                        ftf_bytes.remove(i);
                        ff_opt.file_to_findlen -= 1;
                    }
                    i += 1;
                }
            }
            *file_to_find = Some(std::ffi::CString::new(ftf_bytes).unwrap());
        }

        let ftf_cstr = file_to_find.as_deref().unwrap_or(c"");
        let ftf_bytes = ftf_cstr.to_bytes();
        let rel_to_curdir = ftf_bytes.first() == Some(&b'.')
            && (ftf_bytes.len() == 1
                || vim_ispathsep(i32::from(ftf_bytes[1]))
                || (ftf_bytes[1] == b'.'
                    && (ftf_bytes.len() == 2 || vim_ispathsep(i32::from(ftf_bytes[2])))));
        let is_mswin_abs = cfg!(windows)
            && (ftf_bytes
                .first()
                .is_some_and(|&b| vim_ispathsep(i32::from(b)))
                || ftf_bytes.get(1) == Some(&b':'));

        if vim_isAbsName(ftf_bytes) || rel_to_curdir || is_mswin_abs {
            if first {
                if path_with_url(ftf_bytes) {
                    let res = Some(
                        std::ffi::CString::new(&ftf_bytes[..ff_opt.file_to_findlen])
                            .unwrap_or_default(),
                    );
                    FF_OPT_STATE.with(|cell| *cell.borrow_mut() = ff_opt);
                    return res;
                }

                let rel_fnamelen = rel_fname_opt.map_or(0, |s| s.to_bytes().len());

                let mut run = 1;
                while run <= 2 {
                    let mut l = ff_opt.file_to_findlen;
                    if run == 1
                        && rel_to_curdir
                        && (options & FNAME_REL) != 0
                        && let Some(rel_fname) = rel_fname_opt
                        && rel_fnamelen + l < MAXPATHL
                    {
                        let tail_off = (path_tail(rel_fname).as_ptr() as usize
                            - rel_fname.as_ptr() as usize)
                            as i32;
                        l = format_to_buf!(
                            &mut *g_st().name_buff.borrow_mut(),
                            c"%.*s%s",
                            tail_off,
                            rel_fname,
                            ftf_cstr,
                        ) as usize;
                    } else {
                        crate::memory::copy_bytes_to_c_buf(
                            &mut *g_st().name_buff_bytes_mut(),
                            ftf_bytes,
                        );
                        run = 2;
                    }

                    let mut suf_bytes = suffixes_cstr.to_bytes();
                    loop {
                        let nb_cstr = g_st().name_buff_cstr();
                        if os_path_exists(nb_cstr)
                            && (find_what == FindFileWhat::Both
                                || ((find_what == FindFileWhat::Dir) == os_isdir(nb_cstr)))
                        {
                            let slen = simplify_filename(&mut *g_st().name_buff_bytes_mut());
                            let res = Some(
                                std::ffi::CString::new(&g_st().name_buff.borrow()[..slen])
                                    .unwrap_or_default(),
                            );
                            FF_OPT_STATE.with(|cell| *cell.borrow_mut() = ff_opt);
                            return res;
                        }
                        if suf_bytes.is_empty() {
                            break;
                        }
                        copy_option_part(
                            &mut suf_bytes,
                            &mut g_st().name_buff_bytes_mut()[l..MAXPATHL],
                            c",",
                        );
                    }
                    run += 1;
                }
            }
        } else {
            if first {
                vim_findfile_free_visited(search_ctx.as_mut());
                ff_opt.dir = path_option_cstr.to_bytes().to_vec();
                ff_opt.dir_off = 0;
                ff_opt.did_findfile_init = false;
            }

            loop {
                if ff_opt.did_findfile_init {
                    file_name = vim_findfile(search_ctx.as_mut());
                    if file_name.is_some() {
                        break;
                    }
                    ff_opt.did_findfile_init = false;
                } else {
                    let mut dir_bytes = &ff_opt.dir[ff_opt.dir_off..];
                    if dir_bytes.is_empty() {
                        *search_ctx = None;
                        break;
                    }

                    let mut buf = [0u8; MAXPATHL];
                    let old_len = dir_bytes.len();
                    copy_option_part(&mut dir_bytes, &mut buf, c" ,");
                    ff_opt.dir_off += old_len - dir_bytes.len();

                    let stop_off = vim_findfile_stopdir(&mut buf);
                    let stopdirs = stop_off.map(|off| crate::memory::bytes_as_cstr(&buf[off..]));
                    *search_ctx = vim_findfile_init(
                        crate::memory::bytes_as_cstr(&buf),
                        &ftf_bytes[..ff_opt.file_to_findlen],
                        stopdirs,
                        100,
                        false,
                        find_what,
                        search_ctx.take(),
                        false,
                        rel_fname_opt,
                    );
                    if search_ctx.is_some() {
                        ff_opt.did_findfile_init = true;
                    }
                }
            }
        }

        FF_OPT_STATE.with(|cell| *cell.borrow_mut() = ff_opt);

        if file_name.is_none() && (options & FNAME_MESS) != 0 {
            if first {
                if find_what == FindFileWhat::Dir {
                    semsg!(tr(E_CANT_FIND_DIRECTORY_STR_IN_CDPATH), ftf_cstr);
                } else {
                    semsg!(tr(E_CANT_FIND_FILE_STR_IN_PATH), ftf_cstr);
                }
            } else if find_what == FindFileWhat::Dir {
                semsg!(tr(E_NO_MORE_DIRECTORY_STR_FOUND_IN_CDPATH), ftf_cstr);
            } else {
                semsg!(tr(E_NO_MORE_FILE_STR_FOUND_IN_PATH), ftf_cstr);
            }
        }

        file_name
    }
}

/// Get the file name at the cursor.
pub fn grab_file_name(count: i32, file_lnum: &mut Linenr) -> Option<std::ffi::CString> {
    let options = FNAME_MESS | FNAME_EXP | FNAME_REL | FNAME_UNESC;
    let fname = if g_st().visual_global.borrow().active {
        let (vis_cstr, len) = get_visual_text(None)?;
        let vis_bytes = vis_cstr.to_bytes();
        if vis_bytes.get(len) == Some(&b':')
            && vis_bytes.get(len + 1).is_some_and(u8::is_ascii_digit)
        {
            let mut rem = &vis_bytes[len + 1..];
            *file_lnum = getdigits_int32(&mut rem, false, 0);
        }
        find_file_name_in_path(
            &vis_bytes[..len.min(vis_bytes.len())],
            options,
            count as i64,
            curbuf_mut().ffname_opt(),
        )
    } else {
        file_name_at_cursor(options | FNAME_HYP, count, Some(&mut *file_lnum))
    }?;
    #[cfg(windows)]
    let fname = {
        let mut bytes: Vec<u8> = fname.into_bytes_with_nul();
        path_to_slash(&mut bytes);
        crate::memory::cstring_until_nul(&bytes)
    };
    Some(fname)
}

/// Return the file name under or after the cursor.
pub fn file_name_at_cursor(
    options: i32,
    count: i32,
    file_lnum: Option<&mut Linenr>,
) -> Option<std::ffi::CString> {
    file_name_in_line(
        crate::cursor::get_cursor_line_slice(),
        curwin_mut().w_cursor.col,
        options,
        count,
        curbuf_mut().ffname_opt(),
        (file_lnum.is_some()).then(|| &mut *file_lnum.unwrap()),
    )
}

/// Return the name of the file under or after `line[col]`.
pub fn file_name_in_line(
    line_bytes: &[u8],
    col: i32,
    options: i32,
    count: i32,
    rel_fname: Option<&std::ffi::CStr>,
    file_lnum: Option<&mut Linenr>,
) -> Option<std::ffi::CString> {
    let line_bytes = crate::memory::bytes_until_nul(line_bytes);
    {
        let mut idx = (col.max(0) as usize).min(line_bytes.len());
        while idx < line_bytes.len() && !vim_isfilec(i32::from(line_bytes[idx])) {
            idx += utfc_slice2len(&line_bytes[idx..]).min(line_bytes.len() - idx);
        }
        if idx >= line_bytes.len() {
            if (options & FNAME_MESS) != 0 {
                emsg(tr(c"E446: No file name under cursor"));
            }
            return None;
        }

        let mut in_type = true;
        let mut is_url = false;

        while idx > 0 {
            let head_off = crate::mbyte::utf_head_off_slice(line_bytes, idx - 1) as usize;
            if head_off > 0 {
                idx = idx.saturating_sub(head_off + 1);
            } else if vim_isfilec(i32::from(line_bytes[idx - 1]))
                || ((options & FNAME_HYP) != 0 && path_is_url(&line_bytes[idx - 1..]))
            {
                idx -= 1;
            } else {
                break;
            }
        }

        let ptr_bytes = &line_bytes[idx..];
        let mut len = if path_has_drive_letter(ptr_bytes) {
            2
        } else {
            0
        };
        while len < ptr_bytes.len()
            && (vim_isfilec(i32::from(ptr_bytes[len]))
                || ptr_bytes[len..].starts_with(b"\\ ")
                || ((options & FNAME_HYP) != 0 && path_is_url(&ptr_bytes[len..]))
                || (is_url && matches!(ptr_bytes[len], b':' | b'?' | b'&' | b'=')))
        {
            let b = ptr_bytes[len];
            if b.is_ascii_alphabetic() {
                if in_type && path_is_url(&ptr_bytes[len + 1..]) {
                    is_url = true;
                }
            } else {
                in_type = false;
            }

            if ptr_bytes[len..].starts_with(b"\\ ") {
                len += 1;
            }
            len += utfc_slice2len(&ptr_bytes[len..]).min(ptr_bytes.len() - len);
        }

        if len > 2
            && matches!(ptr_bytes[len - 1], b'.' | b',' | b':' | b';' | b'!')
            && ptr_bytes[len - 2] != b'.'
        {
            len -= 1;
        }

        if let Some(file_lnum) = file_lnum {
            let english = b" line ";
            let mut p_bytes = &ptr_bytes[len..];
            if p_bytes.starts_with(english) {
                p_bytes = &p_bytes[english.len()..];
            } else {
                let loc_msg = tr(LINE_MSG).to_bytes();
                if p_bytes.starts_with(loc_msg) {
                    p_bytes = &p_bytes[loc_msg.len()..];
                } else {
                    p_bytes = skipwhite_slice(p_bytes);
                }
            }
            if !p_bytes.is_empty() {
                if !p_bytes[0].is_ascii_digit() {
                    p_bytes = &p_bytes[1..];
                }
                p_bytes = skipwhite_slice(p_bytes);
                if p_bytes.first().is_some_and(u8::is_ascii_digit) {
                    *file_lnum = getdigits_long(&mut p_bytes, false, 0) as Linenr;
                }
            }
        }

        find_file_name_in_path(&ptr_bytes[..len], options, count as i64, rel_fname)
    }
}

fn eval_includeexpr(ptr_slice: &[u8]) -> Option<std::ffi::CString> {
    let ptr_cstr =
        std::ffi::CString::new(crate::memory::bytes_until_nul(ptr_slice)).unwrap_or_default();
    {
        let save_sctx = g_st().current_sctx.get();
        set_vim_var_string(
            VV_FNAME,
            Some(&ptr_cstr),
            ptr_cstr.as_bytes().len() as isize,
        );
        g_st()
            .current_sctx
            .set(curbuf_mut().b_p_script_ctx[K_BUF_OPT_INCLUDEEXPR].get());

        let use_sandbox = was_set_insecurely(
            Some(crate::globals::curwin_mut()),
            K_OPT_INCLUDEEXPR,
            OPT_LOCAL,
        ) != 0;
        let mut tv: TypVal = TypVal::new();
        let mut res = None;
        if eval_expr_option_tv(&mut curbuf_mut().b_p_inex, use_sandbox, &mut tv) {
            let s = tv_get_string(&tv);
            res = Some((*s).to_owned());
            tv_clear(&mut tv);
        }

        set_vim_var_string(VV_FNAME, None, 0);
        g_st().current_sctx.set(save_sctx);
        res
    }
}

/// Return the name of the file `ptr` in `'path'`.
pub fn find_file_name_in_path(
    ptr_in: &[u8],
    options: i32,
    mut count: i64,
    rel_fname: Option<&std::ffi::CStr>,
) -> Option<std::ffi::CString> {
    let mut ptr_slice = crate::memory::bytes_until_nul(ptr_in);
    {
        let mut tofree: Option<std::ffi::CString> = None;

        if ptr_slice.is_empty() {
            return None;
        }

        if (options & FNAME_HYP) != 0
            && ptr_slice.len() > 6
            && &ptr_slice[..6] == b"file:/"
            && !vim_ispathsep(i32::from(ptr_slice[6]))
        {
            let off = if path_has_drive_letter(&ptr_slice[6..]) {
                6
            } else {
                5
            };
            ptr_slice = &ptr_slice[off..];
        }

        if (options & FNAME_INCL) != 0 && curbuf_mut().b_p_inex.cb_type() != K_CALLBACK_NONE_CONST {
            tofree = eval_includeexpr(ptr_slice);
            if let Some(ref tf) = tofree {
                ptr_slice = tf.as_bytes();
            }
        }

        let mut file_name: Option<std::ffi::CString>;
        if (options & FNAME_EXP) != 0 {
            let mut file_to_find: Option<std::ffi::CString> = None;
            let mut search_ctx: Option<FfSearchCtx> = None;

            file_name = find_file_in_path(
                ptr_slice,
                options & !FNAME_MESS,
                true,
                rel_fname,
                &mut file_to_find,
                &mut search_ctx,
            );

            if file_name.is_none()
                && (options & FNAME_INCL) == 0
                && curbuf_mut().b_p_inex.cb_type() != K_CALLBACK_NONE_CONST
                && let Some(tf) = eval_includeexpr(ptr_slice)
            {
                tofree = Some(tf);
                ptr_slice = tofree.as_ref().unwrap().as_bytes();
                file_name = find_file_in_path(
                    ptr_slice,
                    options & !FNAME_MESS,
                    true,
                    rel_fname,
                    &mut file_to_find,
                    &mut search_ctx,
                );
            }
            if file_name.is_none() && (options & FNAME_MESS) != 0 {
                let ptr_cstr = std::ffi::CString::new(ptr_slice).unwrap_or_default();
                semsg!(tr(c"E447: Can't find file \"%s\" in path"), &ptr_cstr);
            }

            while file_name.is_some() && {
                count -= 1;
                count > 0
            } {
                file_name = find_file_in_path(
                    ptr_slice,
                    options,
                    false,
                    rel_fname,
                    &mut file_to_find,
                    &mut search_ctx,
                );
            }
        } else {
            file_name = Some(std::ffi::CString::new(ptr_slice).unwrap_or_default());
        }

        drop(tofree);
        file_name
    }
}

pub fn do_autocmd_dirchanged(new_dir: &std::ffi::CStr, scope: CdScope, cause: CdCause, pre: bool) {
    let new_dir_opt = Some(new_dir);
    {
        static RECURSIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

        let event = if pre {
            EVENT_DIRCHANGEDPRE
        } else {
            EVENT_DIRCHANGED
        };

        if RECURSIVE.load(std::sync::atomic::Ordering::Relaxed) || !has_event(event) {
            return;
        }

        RECURSIVE.store(true, std::sync::atomic::Ordering::Relaxed);

        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);
        let mut buf: [u8; 8] = [0; 8];

        let scope_cstr = match scope {
            K_CD_SCOPE_GLOBAL => c"global",
            K_CD_SCOPE_TABPAGE => c"tabpage",
            K_CD_SCOPE_BUFFER => c"buffer",
            K_CD_SCOPE_WINDOW => c"window",
            _ => std::process::abort(),
        };
        crate::memory::copy_bytes_to_c_buf(&mut buf, scope_cstr.to_bytes());

        if pre {
            tv_dict_add_str(&dict, c"directory", new_dir_opt);
        } else {
            tv_dict_add_str(&dict, c"cwd", new_dir_opt);
        }
        tv_dict_add_str(&dict, c"scope", Some(crate::memory::bytes_as_cstr(&buf)));
        tv_dict_add_bool(
            &dict,
            c"changed_window",
            i32::from(cause == K_CD_CAUSE_WINDOW),
        );
        tv_dict_set_keys_readonly(&dict);

        match cause {
            K_CD_CAUSE_MANUAL | K_CD_CAUSE_WINDOW | K_CD_CAUSE_BUFFER => {}
            K_CD_CAUSE_AUTO => {
                crate::memory::copy_bytes_to_c_buf(&mut buf, b"auto");
            }
            _ => std::process::abort(),
        }

        apply_autocmds(
            event,
            Some(crate::memory::bytes_as_cstr(&buf)),
            new_dir_opt,
            false,
            curbuf_opt_mut(),
        );

        restore_v_event(&dict, &mut save_v_event);

        RECURSIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Change to a file's directory.
pub fn vim_chdirfile(fname: &std::ffi::CStr, cause: CdCause) -> bool {
    {
        let mut dir: [u8; MAXPATHL] = [0; MAXPATHL];

        crate::memory::copy_bytes_to_c_buf(&mut dir, fname.to_bytes());
        let dir_cstr = crate::memory::bytes_as_cstr(&dir);
        let tail_off = path_tail_with_sep(dir_cstr).as_ptr() as usize - dir_cstr.as_ptr() as usize;
        dir[tail_off] = NUL as u8;

        if !os_dirname(&mut *g_st().name_buff_bytes_mut()) {
            g_st().name_buff.borrow_mut()[0] = 0;
        }

        if path_equal(
            crate::memory::bytes_as_cstr(&dir),
            g_st().name_buff_cstr(),
            K_PATH_CMP_LITERAL,
        ) {
            return true;
        }

        let dir_cstr = crate::memory::bytes_as_cstr(&dir);
        if cause != K_CD_CAUSE_OTHER {
            do_autocmd_dirchanged(dir_cstr, K_CD_SCOPE_WINDOW, cause, true);
        }

        if os_chdir(dir_cstr) != 0 {
            return false;
        }

        if cause != K_CD_CAUSE_OTHER {
            let dir_cstr = crate::memory::bytes_as_cstr(&dir);
            do_autocmd_dirchanged(dir_cstr, K_CD_SCOPE_WINDOW, cause, false);
        }

        true
    }
}

/// Change directory to `new_dir`. Search `'cdpath'` for relative directory names.
pub fn vim_chdir(new_dir: &std::ffi::CStr) -> i32 {
    let mut file_to_find: Option<std::ffi::CString> = None;
    let mut search_ctx: Option<FfSearchCtx> = None;
    let Some(dir_name) = find_directory_in_path(
        new_dir.to_bytes(),
        FNAME_MESS,
        curbuf_mut().ffname_opt(),
        &mut file_to_find,
        &mut search_ctx,
    ) else {
        return -1;
    };

    os_chdir(&dir_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_findfile_stopdir_splits_and_unescapes_semicolons() {
        let mut buf = *b"/a/b;/c/d\0";
        let stop = vim_findfile_stopdir(&mut buf);
        assert!(stop.is_some());
        let first = crate::memory::bytes_until_nul(&buf);
        let rest = crate::memory::bytes_until_nul(&buf[stop.unwrap()..]);
        assert_eq!(first, b"/a/b");
        assert_eq!(rest, b"/c/d");

        let mut escaped = *b"/a\\;b;/c\0";
        let stop2 = vim_findfile_stopdir(&mut escaped);
        assert!(stop2.is_some());
        let first2 = crate::memory::bytes_until_nul(&escaped);
        let rest2 = crate::memory::bytes_until_nul(&escaped[stop2.unwrap()..]);
        assert_eq!(first2, b"/a;b");
        assert_eq!(rest2, b"/c");

        let mut none = *b"/a/b\0";
        let stop3 = vim_findfile_stopdir(&mut none);
        assert!(stop3.is_none());
    }
}
