//! File searching functions for `'path'`, `'tags'`, and `'cdpath'` options (`src/nvim/file_search.c`).

use crate::api::private::helpers::{api_clear_string, cbuf_to_string, copy_string, cstr_to_string};
use crate::api::private::validate::NvimString;
use crate::autocmd::{EVENT_DIRCHANGED, EVENT_DIRCHANGEDPRE, apply_autocmds, has_event};
use crate::charset::{NUL, getdigits, getdigits_int32, getdigits_long, skipwhite, vim_isfilec};
use crate::context::{CdCause, K_CD_CAUSE_WINDOW};
use crate::cursor::{Linenr, get_cursor_line_ptr};
use crate::errors::*;
use crate::eval::typval::{
    K_CALLBACK_NONE_CONST, TypVal, tv_clear, tv_dict_add_bool, tv_dict_add_str,
    tv_dict_set_keys_readonly, tv_get_string,
};
use crate::eval::vars::{VV_FNAME, set_vim_var_string};
use crate::eval::{eval_expr_option_tv, get_v_event, restore_v_event};
use crate::globals::*;
use crate::mbyte::{mb_tolower, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memory::xstrlcpy;
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
use crate::strings::{tr, vim_snprintf, vim_strchr};
use std::ffi::CStr;
use std::ptr;

use crate::ex_docmd::{CdScope, K_CD_CAUSE_MANUAL};
use crate::optionstr::OptIndex;

pub const FINDFILE_FILE: i32 = 0;
pub const FINDFILE_DIR: i32 = 1;
pub const FINDFILE_BOTH: i32 = 2;

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

pub(crate) const NULL_STRING: NvimString = NvimString {
    data: ptr::null_mut(),
    size: 0,
};

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
    pub ffs_star_star_empty: i32,
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
    pub ffsc_find_what: i32,
    pub ffsc_tagfile: i32,
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
            ffsc_find_what: 0,
            ffsc_tagfile: 0,
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
    path: impl crate::strings::IntoCCharPtr,
    filename: impl crate::strings::IntoCCharPtr,
    filenamelen: usize,
    stopdirs: impl crate::strings::IntoCCharPtr,
    level: i32,
    free_visited: i32,
    find_what: i32,
    search_ctx_arg: Option<FfSearchCtx>,
    tagfile: i32,
    rel_fname: impl crate::strings::IntoCCharPtr,
) -> Option<FfSearchCtx> {
    let mut path = path.into_c_char_ptr();
    let filename = filename.into_c_char_ptr();
    let stopdirs = stopdirs.into_c_char_ptr();
    let rel_fname = rel_fname.into_c_char_ptr();
    {
        let mut search_ctx = search_ctx_arg.unwrap_or_default();
        search_ctx.ffsc_find_what = find_what;
        search_ctx.ffsc_tagfile = tagfile;

        ff_clear(&mut search_ctx);

        if free_visited == i32::from(true) {
            vim_findfile_free_visited(Some(&mut search_ctx));
        } else {
            search_ctx.ffsc_visited_list = Some(ff_get_visited_list(
                filename,
                filenamelen,
                &mut search_ctx.ffsc_visited_lists_list,
            ));
            search_ctx.ffsc_dir_visited_list = Some(ff_get_visited_list(
                filename,
                filenamelen,
                &mut search_ctx.ffsc_dir_visited_lists_list,
            ));
        }

        let mut expand_buf: [u8; MAXPATHL] = [0; MAXPATHL];
        let expand_ptr = expand_buf.as_mut_ptr().cast();

        if crate::memory::cstr_first_byte(path) == b'.'
            && (vim_ispathsep(i32::from(crate::memory::cstr_byte_at(path, 1)))
                || crate::memory::cstr_byte_at(path, 1) == 0)
            && (tagfile == 0 || !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_DOTTAG))
            && !rel_fname.is_null()
        {
            let len = crate::memory::ptr_offset_from(path_tail(rel_fname), rel_fname) as usize;

            if !vim_isAbsName(rel_fname) && len + 1 < MAXPATHL {
                crate::memory::cstr_ncpy(expand_ptr, rel_fname, len);
                let full = FullName_save(expand_ptr, false).unwrap_or_default();
                search_ctx.ffsc_start_dir = cbuf_to_string(full.as_ptr(), full.as_bytes().len());
            } else {
                search_ctx.ffsc_start_dir = cbuf_to_string(rel_fname, len);
            }
            path = path.wrapping_add(1);
            if !crate::memory::cstr_is_empty(path) {
                path = path.wrapping_add(1);
            }
        } else if crate::memory::cstr_is_empty(path) || !vim_isAbsName(path) {
            #[cfg(windows)]
            let handled_drive = if !crate::memory::cstr_is_empty(path)
                && crate::memory::cstr_byte_at(path, 1) == b':'
            {
                let drive: [u8; 3] = [crate::memory::cstr_first_byte(path), b':', 0];
                if !vim_full_name(drive.as_ptr().cast(), expand_ptr.cast(), MAXPATHL, true) {
                    return None;
                }
                path = path.wrapping_add(2);
                true
            } else {
                false
            };
            #[cfg(not(windows))]
            let handled_drive = false;

            if !handled_drive && !os_dirname(expand_ptr, MAXPATHL) {
                return None;
            }
            let expand_len = crate::memory::cstr_len(expand_ptr);

            search_ctx.ffsc_start_dir = copy_string(
                NvimString {
                    data: expand_ptr,
                    size: expand_len,
                },
                None,
            );
        }

        if !stopdirs.is_null() {
            let mut walker = stopdirs;
            while crate::memory::cstr_first_byte(walker) == b';' {
                walker = walker.wrapping_add(1);
            }

            loop {
                let helper = walker;
                walker = vim_strchr(walker, b';' as i32);
                let len = if !walker.is_null() {
                    crate::memory::ptr_offset_from(walker, helper) as usize
                } else {
                    crate::memory::cstr_len(helper)
                };
                let dir_str = if !crate::memory::cstr_is_empty(helper)
                    && !vim_isAbsName(helper)
                    && len + 1 < MAXPATHL
                {
                    crate::memory::cstr_ncpy(expand_ptr, helper, len);
                    FullName_save(helper, len != 0).unwrap_or_default()
                } else {
                    std::ffi::CString::new(crate::memory::cstr_slice(helper, len))
                        .unwrap_or_default()
                };
                search_ctx.ffsc_stopdirs_v.push(dir_str);
                if !walker.is_null() {
                    walker = walker.wrapping_add(1);
                }
                if walker.is_null() {
                    break;
                }
            }
        }

        search_ctx.ffsc_level = level;

        let mut wc_part = vim_strchr(path, b'*' as i32);
        let mut expand_size: usize;
        if !wc_part.is_null() {
            search_ctx.ffsc_fix_path =
                cbuf_to_string(path, crate::memory::ptr_offset_from(wc_part, path) as usize);

            expand_size = 0;
            while !crate::memory::cstr_is_empty(wc_part) {
                if expand_size + 5 >= MAXPATHL {
                    emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION));
                    break;
                }
                if crate::memory::cstr_first_byte(wc_part) == b'*'
                    && crate::memory::cstr_byte_at(wc_part, 1) == b'*'
                {
                    unsafe {
                        *expand_ptr.wrapping_add(expand_size) = *wc_part;
                    }
                    expand_size += 1;
                    wc_part = wc_part.wrapping_add(1);
                    unsafe {
                        *expand_ptr.wrapping_add(expand_size) = *wc_part;
                    }
                    expand_size += 1;
                    wc_part = wc_part.wrapping_add(1);

                    let mut errpt = wc_part;
                    let llevel = getdigits(&mut errpt, false, 255);
                    if errpt != wc_part && llevel > 0 && llevel < 255 {
                        expand_buf[expand_size] = llevel as u8;
                        expand_size += 1;
                    } else if errpt != wc_part && llevel == 0 {
                        expand_size -= 2;
                    } else {
                        expand_buf[expand_size] = FF_MAX_STAR_STAR_EXPAND;
                        expand_size += 1;
                    }
                    wc_part = errpt;
                    if !crate::memory::cstr_is_empty(wc_part)
                        && !vim_ispathsep(i32::from(crate::memory::cstr_first_byte(wc_part)))
                    {
                        semsg!(
                            tr(c"E343: Invalid path: '**[number]' must be at the end of the path or be followed by '%s'."),
                            PATHSEPSTR_C,
                        );
                        return None;
                    }
                } else {
                    unsafe {
                        *expand_ptr.wrapping_add(expand_size) = *wc_part;
                    }
                    expand_size += 1;
                    wc_part = wc_part.wrapping_add(1);
                }
            }
            unsafe {
                *expand_ptr.wrapping_add(expand_size) = NUL;
            }
            search_ctx.ffsc_wc_path = copy_string(
                NvimString {
                    data: expand_ptr,
                    size: expand_size,
                },
                None,
            );
        } else {
            search_ctx.ffsc_fix_path = cstr_to_string(path);
        }

        if search_ctx.ffsc_start_dir.data.is_null() {
            search_ctx.ffsc_start_dir = copy_string(search_ctx.ffsc_fix_path, None);
            unsafe {
                *search_ctx.ffsc_fix_path.data = NUL;
            }
            search_ctx.ffsc_fix_path.size = 0;
        }

        if search_ctx.ffsc_start_dir.size + search_ctx.ffsc_fix_path.size + 3 >= MAXPATHL {
            emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION));
            return None;
        }

        let mut add_sep = after_pathsep(
            search_ctx.ffsc_start_dir.data,
            search_ctx
                .ffsc_start_dir
                .data
                .wrapping_add(search_ctx.ffsc_start_dir.size),
        ) == 0;
        expand_size = vim_snprintf!(
            expand_ptr,
            MAXPATHL,
            c"%s%s",
            search_ctx.ffsc_start_dir.data,
            if add_sep {
                PATHSEPSTR_C.as_ptr()
            } else {
                EMPTY_C.as_ptr()
            },
        ) as usize;

        {
            let bufsize = expand_size + search_ctx.ffsc_fix_path.size + 1;
            let mut buf_vec = vec![0u8; bufsize];
            let buf = buf_vec.as_mut_ptr().cast();

            vim_snprintf!(
                buf,
                bufsize,
                c"%s%s",
                expand_ptr,
                search_ctx.ffsc_fix_path.data,
            );
            if os_isdir(buf) {
                if search_ctx.ffsc_fix_path.size > 0 {
                    add_sep = after_pathsep(
                        search_ctx.ffsc_fix_path.data,
                        search_ctx
                            .ffsc_fix_path
                            .data
                            .wrapping_add(search_ctx.ffsc_fix_path.size),
                    ) == 0;
                    expand_size += vim_snprintf!(
                        expand_ptr.wrapping_add(expand_size),
                        MAXPATHL - expand_size,
                        c"%s%s",
                        search_ctx.ffsc_fix_path.data,
                        if add_sep {
                            PATHSEPSTR_C.as_ptr()
                        } else {
                            EMPTY_C.as_ptr()
                        },
                    ) as usize;
                }
            } else {
                let p = path_tail(search_ctx.ffsc_fix_path.data);
                let mut len = search_ctx.ffsc_fix_path.size as i32;

                if p > search_ctx.ffsc_fix_path.data {
                    len =
                        crate::memory::ptr_offset_from(p, search_ctx.ffsc_fix_path.data) as i32 - 1;
                    if len >= 2
                        && crate::memory::cstr_first_byte(search_ctx.ffsc_fix_path.data) == b'.'
                        && crate::memory::cstr_byte_at(search_ctx.ffsc_fix_path.data, 1) == b'.'
                        && (len == 2
                            || crate::memory::cstr_byte_at(search_ctx.ffsc_fix_path.data, 2)
                                == PATHSEP)
                    {
                        return None;
                    }

                    add_sep = after_pathsep(
                        search_ctx.ffsc_fix_path.data,
                        search_ctx
                            .ffsc_fix_path
                            .data
                            .wrapping_add(search_ctx.ffsc_fix_path.size),
                    ) == 0;
                    expand_size += vim_snprintf!(
                        expand_ptr.wrapping_add(expand_size),
                        MAXPATHL - expand_size,
                        c"%.*s%s",
                        len,
                        search_ctx.ffsc_fix_path.data,
                        if add_sep {
                            PATHSEPSTR_C.as_ptr()
                        } else {
                            EMPTY_C.as_ptr()
                        },
                    ) as usize;
                }

                if !search_ctx.ffsc_wc_path.data.is_null() {
                    let tempsize = (search_ctx.ffsc_fix_path.size - len as usize)
                        + search_ctx.ffsc_wc_path.size
                        + 1;
                    let mut temp_vec: Vec<u8> = vec![0; tempsize];
                    let temp_len = vim_snprintf!(
                        temp_vec.as_mut_ptr().cast(),
                        tempsize,
                        c"%s%s",
                        search_ctx.ffsc_fix_path.data.wrapping_add(len as usize),
                        search_ctx.ffsc_wc_path.data,
                    ) as usize;
                    api_clear_string(&mut search_ctx.ffsc_wc_path);
                    search_ctx.ffsc_wc_path = cbuf_to_string(temp_vec.as_ptr().cast(), temp_len);
                }
            }
        }

        let sptr = ff_create_stack_element(
            expand_ptr,
            expand_size,
            search_ctx.ffsc_wc_path.data,
            search_ctx.ffsc_wc_path.size,
            level,
            0,
        );

        ff_push(&mut search_ctx, sptr);
        search_ctx.ffsc_file_to_search = cbuf_to_string(filename, filenamelen);
        Some(search_ctx)
    }
}

/// Return the stopdir string. Check that `;` is not escaped.
pub fn vim_findfile_stopdir(mut buf: *mut u8) -> *mut u8 {
    while unsafe { *buf } != 0
        && unsafe { *buf } != b';'
        && (unsafe { *buf } != b'\\' || unsafe { *buf.wrapping_add(1) } != b';')
    {
        buf = buf.wrapping_add(1);
    }
    let mut dst = buf;
    if unsafe { *buf } == b';' {
        unsafe {
            *buf = 0;
        }
        return buf.wrapping_add(1);
    }
    if unsafe { *buf } == 0 {
        return ptr::null_mut();
    }
    unsafe {
        *dst = b';';
    }
    dst = dst.wrapping_add(1);
    buf = buf.wrapping_add(2);
    while unsafe { *buf } != 0 && unsafe { *buf } != b';' {
        if unsafe { *buf } == b'\\' && unsafe { *buf.wrapping_add(1) } == b';' {
            unsafe {
                *dst = b';';
            }
            dst = dst.wrapping_add(1);
            buf = buf.wrapping_add(2);
        } else {
            unsafe {
                *dst = *buf;
            }
            dst = dst.wrapping_add(1);
            buf = buf.wrapping_add(1);
        }
    }
    debug_assert!(dst < buf);
    unsafe {
        *dst = 0;
    }
    if unsafe { *buf } == b';' {
        unsafe {
            *buf = 0;
        }
        buf.wrapping_add(1)
    } else {
        ptr::null_mut()
    }
}

/// Find a file in a search context.
pub fn vim_findfile(search_ctx: Option<&mut FfSearchCtx>) -> Option<std::ffi::CString> {
    let search_ctx = search_ctx?;

    let mut file_path_vec: Vec<u8> = vec![0; MAXPATHL];
    let mut file_path = NvimString {
        data: file_path_vec.as_mut_ptr().cast(),
        size: 0,
    };
    let mut path_end = if !search_ctx.ffsc_start_dir.data.is_null() {
        search_ctx
            .ffsc_start_dir
            .data
            .wrapping_add(search_ctx.ffsc_start_dir.size)
    } else {
        ptr::null_mut()
    };

    'upward: loop {
        loop {
            os_breakcheck();
            if g_st().got_int {
                break;
            }

            let Some(mut stack_elem) = ff_pop(search_ctx) else {
                break;
            };
            let stackp = &mut stack_elem;
            if stackp.ffs_filearray.is_empty() {
                let dir_vis_idx = search_ctx.ffsc_dir_visited_list.unwrap();
                if !ff_check_visited(
                    &mut search_ctx.ffsc_dir_visited_lists_list[dir_vis_idx].ffvl_visited_list,
                    stackp.ffs_fix_path.data,
                    stackp.ffs_fix_path.size,
                    stackp.ffs_wc_path.data,
                    stackp.ffs_wc_path.size,
                ) {
                    continue;
                }
            }

            if stackp.ffs_level <= 0 {
                continue;
            }

            unsafe {
                *file_path.data = NUL;
            }
            file_path.size = 0;

            let mut rest_of_wildcards: NvimString;
            if stackp.ffs_filearray.is_empty() {
                let mut dirptrs = [file_path.data, ptr::null_mut()];

                if !vim_isAbsName(stackp.ffs_fix_path.data)
                    && !search_ctx.ffsc_start_dir.data.is_null()
                {
                    if search_ctx.ffsc_start_dir.size + 1 >= MAXPATHL {
                        break 'upward;
                    }
                    let add_sep = after_pathsep(
                        search_ctx.ffsc_start_dir.data,
                        search_ctx
                            .ffsc_start_dir
                            .data
                            .wrapping_add(search_ctx.ffsc_start_dir.size),
                    ) == 0;
                    file_path.size = vim_snprintf!(
                        file_path.data,
                        MAXPATHL,
                        c"%s%s",
                        search_ctx.ffsc_start_dir.data,
                        if add_sep {
                            PATHSEPSTR_C.as_ptr()
                        } else {
                            EMPTY_C.as_ptr()
                        },
                    ) as usize;
                    if file_path.size >= MAXPATHL {
                        break 'upward;
                    }
                }

                if file_path.size + stackp.ffs_fix_path.size + 1 >= MAXPATHL {
                    break 'upward;
                }
                let add_sep = after_pathsep(
                    stackp.ffs_fix_path.data,
                    stackp
                        .ffs_fix_path
                        .data
                        .wrapping_add(stackp.ffs_fix_path.size),
                ) == 0;
                file_path.size += vim_snprintf!(
                    file_path.data.wrapping_add(file_path.size),
                    MAXPATHL - file_path.size,
                    c"%s%s",
                    stackp.ffs_fix_path.data,
                    if add_sep {
                        PATHSEPSTR_C.as_ptr()
                    } else {
                        EMPTY_C.as_ptr()
                    },
                ) as usize;
                if file_path.size >= MAXPATHL {
                    break 'upward;
                }

                rest_of_wildcards = stackp.ffs_wc_path;
                if !crate::memory::cstr_is_empty(rest_of_wildcards.data) {
                    if crate::memory::cstr_first_byte(rest_of_wildcards.data) == b'*'
                        && crate::memory::cstr_byte_at(rest_of_wildcards.data, 1) == b'*'
                    {
                        let p = rest_of_wildcards.data.wrapping_add(2);

                        if unsafe { *p } > 0 {
                            unsafe {
                                *p -= 1;
                            }
                            if file_path.size + 1 >= MAXPATHL {
                                break 'upward;
                            }
                            unsafe {
                                *file_path.data.wrapping_add(file_path.size).cast::<u8>() = b'*';
                            }
                            file_path.size += 1;
                        }

                        if unsafe { *p } == 0 {
                            unsafe {
                                ptr::copy(
                                    rest_of_wildcards.data.wrapping_add(3),
                                    rest_of_wildcards.data,
                                    (rest_of_wildcards.size - 3) + 1,
                                )
                            };
                            rest_of_wildcards.size -= 3;
                            stackp.ffs_wc_path.size = rest_of_wildcards.size;
                        } else {
                            rest_of_wildcards.data = rest_of_wildcards.data.wrapping_add(3);
                            rest_of_wildcards.size -= 3;
                        }

                        if stackp.ffs_star_star_empty == 0 {
                            stackp.ffs_star_star_empty = 1;
                            dirptrs[1] = stackp.ffs_fix_path.data;
                        }
                    }

                    while !crate::memory::cstr_is_empty(rest_of_wildcards.data)
                        && !vim_ispathsep(i32::from(crate::memory::cstr_first_byte(
                            rest_of_wildcards.data,
                        )))
                    {
                        if file_path.size + 1 >= MAXPATHL {
                            break 'upward;
                        }
                        unsafe {
                            *file_path.data.wrapping_add(file_path.size) = *rest_of_wildcards.data;
                        }
                        file_path.size += 1;
                        rest_of_wildcards.data = rest_of_wildcards.data.wrapping_add(1);
                        rest_of_wildcards.size -= 1;
                    }

                    unsafe {
                        *file_path.data.wrapping_add(file_path.size) = NUL;
                    }
                    if vim_ispathsep(i32::from(crate::memory::cstr_first_byte(
                        rest_of_wildcards.data,
                    ))) {
                        rest_of_wildcards.data = rest_of_wildcards.data.wrapping_add(1);
                        rest_of_wildcards.size -= 1;
                    }
                }

                if path_with_url(dirptrs[0]) != 0 {
                    stackp.ffs_filearray = vec![crate::memory::cstr_from_bytes(unsafe {
                        std::slice::from_raw_parts(
                            dirptrs[0].cast::<u8>(),
                            crate::memory::xstrnlen(dirptrs[0], file_path.size),
                        )
                    })];
                } else {
                    expand_wildcards_vec(
                        if dirptrs[1].is_null() { 1 } else { 2 },
                        dirptrs.as_mut_ptr().cast(),
                        &mut stackp.ffs_filearray,
                        EW_DIR | EW_ADDSLASH | EW_SILENT | EW_NOTWILD,
                    );
                }

                stackp.ffs_filearray_cur = 0;
                stackp.ffs_stage = 0;
            } else {
                rest_of_wildcards = NvimString {
                    data: stackp
                        .ffs_wc_path
                        .data
                        .wrapping_add(stackp.ffs_wc_path.size),
                    size: 0,
                };
            }

            if stackp.ffs_stage == 0 {
                if crate::memory::cstr_is_empty(rest_of_wildcards.data) {
                    for i in stackp.ffs_filearray_cur..stackp.ffs_filearray.len() {
                        let entry = stackp.ffs_filearray.as_slice()[i].as_ptr().cast_mut();
                        if path_with_url(entry) == 0 && !os_isdir(entry) {
                            continue;
                        }
                        let mut len = stackp.ffs_filearray.as_slice()[i].as_bytes().len();
                        if len + 1 + search_ctx.ffsc_file_to_search.size >= MAXPATHL {
                            break 'upward;
                        }
                        let add_sep = after_pathsep(entry, entry.wrapping_add(len)) == 0;
                        file_path.size = vim_snprintf!(
                            file_path.data,
                            MAXPATHL,
                            c"%s%s%s",
                            entry,
                            if add_sep {
                                PATHSEPSTR_C.as_ptr()
                            } else {
                                EMPTY_C.as_ptr()
                            },
                            search_ctx.ffsc_file_to_search.data,
                        ) as usize;
                        if file_path.size >= MAXPATHL {
                            break 'upward;
                        }

                        len = file_path.size;
                        let mut suf = if search_ctx.ffsc_tagfile != 0 {
                            EMPTY_C.as_ptr().cast_mut()
                        } else {
                            curbuf_mut().b_p_sua
                        };
                        loop {
                            let vis_idx = search_ctx.ffsc_visited_list.unwrap();
                            if (path_with_url(file_path.data) != 0
                                || (os_path_exists(file_path.data)
                                    && (search_ctx.ffsc_find_what == FINDFILE_BOTH
                                        || ((search_ctx.ffsc_find_what == FINDFILE_DIR)
                                            == os_isdir(file_path.data)))))
                                && ff_check_visited(
                                    &mut search_ctx.ffsc_visited_lists_list[vis_idx]
                                        .ffvl_visited_list,
                                    file_path.data,
                                    file_path.size,
                                    EMPTY_C,
                                    0,
                                )
                            {
                                stackp.ffs_filearray_cur = i + 1;
                                ff_push(search_ctx, stack_elem);

                                if path_with_url(file_path.data) == 0 {
                                    file_path.size = simplify_filename(file_path.data.cast());
                                }

                                let mut expand_buf: [u8; MAXPATHL] = [0; MAXPATHL];
                                if os_dirname(expand_buf.as_mut_ptr(), MAXPATHL) {
                                    let p = path_shorten_fname(
                                        file_path.data,
                                        expand_buf.as_mut_ptr().cast(),
                                    );
                                    if !p.is_null() {
                                        let new_len = crate::memory::ptr_offset_from(
                                            file_path.data.wrapping_add(file_path.size),
                                            p,
                                        )
                                            as usize;
                                        unsafe { ptr::copy(p, file_path.data, new_len + 1) };
                                        file_path.size = new_len;
                                    }
                                }
                                return Some(crate::memory::cstr_to_cstring(file_path.data));
                            }

                            if crate::memory::cstr_is_empty(suf) {
                                break;
                            }
                            file_path.size = len
                                + copy_option_part(
                                    &raw mut suf,
                                    file_path.data.wrapping_add(len).cast(),
                                    MAXPATHL - len,
                                    c",",
                                );
                        }
                    }
                } else {
                    for i in stackp.ffs_filearray_cur..stackp.ffs_filearray.len() {
                        let entry = stackp.ffs_filearray.as_slice()[i].as_ptr();
                        let entry_len = stackp.ffs_filearray.as_slice()[i].as_bytes().len();
                        if !os_isdir(entry) {
                            continue;
                        }
                        ff_push(
                            search_ctx,
                            ff_create_stack_element(
                                entry,
                                entry_len,
                                rest_of_wildcards.data,
                                rest_of_wildcards.size,
                                stackp.ffs_level - 1,
                                0,
                            ),
                        );
                    }
                }
                stackp.ffs_filearray_cur = 0;
                stackp.ffs_stage = 1;
            }

            if crate::memory::cstr_first_byte(stackp.ffs_wc_path.data) == b'*'
                && crate::memory::cstr_byte_at(stackp.ffs_wc_path.data, 1) == b'*'
            {
                for i in stackp.ffs_filearray_cur..stackp.ffs_filearray.len() {
                    let entry = stackp.ffs_filearray.as_slice()[i].as_ptr();
                    let entry_len = stackp.ffs_filearray.as_slice()[i].as_bytes().len();
                    if path_equal(entry, stackp.ffs_fix_path.data, K_PATH_CMP_LITERAL) {
                        continue;
                    }
                    if !os_isdir(entry) {
                        continue;
                    }
                    ff_push(
                        search_ctx,
                        ff_create_stack_element(
                            entry,
                            entry_len,
                            stackp.ffs_wc_path.data,
                            stackp.ffs_wc_path.size,
                            stackp.ffs_level - 1,
                            1,
                        ),
                    );
                }
            }
        }

        if !search_ctx.ffsc_start_dir.data.is_null()
            && !search_ctx.ffsc_stopdirs_v.is_empty()
            && !g_st().got_int
        {
            let plen = crate::memory::ptr_offset_from(path_end, search_ctx.ffsc_start_dir.data)
                + isize::from(!crate::memory::cstr_is_empty(path_end));

            if ff_path_in_stoplist(
                search_ctx.ffsc_start_dir.data,
                plen as usize,
                &search_ctx.ffsc_stopdirs_v,
            ) {
                break;
            }

            while path_end > search_ctx.ffsc_start_dir.data
                && vim_ispathsep(i32::from(crate::memory::cstr_first_byte(path_end)))
            {
                path_end = path_end.wrapping_sub(1);
            }
            while path_end > search_ctx.ffsc_start_dir.data
                && !vim_ispathsep(i32::from(unsafe { *path_end.wrapping_sub(1) } as u8))
            {
                path_end = path_end.wrapping_sub(1);
            }
            unsafe {
                *path_end = NUL;
            }

            search_ctx.ffsc_start_dir.size =
                crate::memory::ptr_offset_from(path_end, search_ctx.ffsc_start_dir.data) as usize;
            path_end = path_end.wrapping_sub(1);

            if crate::memory::cstr_is_empty(search_ctx.ffsc_start_dir.data) {
                break;
            }

            if search_ctx.ffsc_start_dir.size + 1 + search_ctx.ffsc_fix_path.size >= MAXPATHL {
                break;
            }
            let add_sep = after_pathsep(
                search_ctx.ffsc_start_dir.data,
                search_ctx
                    .ffsc_start_dir
                    .data
                    .wrapping_add(search_ctx.ffsc_start_dir.size),
            ) == 0;
            file_path.size = vim_snprintf!(
                file_path.data,
                MAXPATHL,
                c"%s%s%s",
                search_ctx.ffsc_start_dir.data,
                if add_sep {
                    PATHSEPSTR_C.as_ptr()
                } else {
                    EMPTY_C.as_ptr()
                },
                search_ctx.ffsc_fix_path.data,
            ) as usize;
            if file_path.size >= MAXPATHL {
                break;
            }

            let sptr = ff_create_stack_element(
                file_path.data,
                file_path.size,
                search_ctx.ffsc_wc_path.data,
                search_ctx.ffsc_wc_path.size,
                search_ctx.ffsc_level,
                0,
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

fn ff_get_visited_list(
    filename: impl crate::strings::IntoCCharPtr,
    filenamelen: usize,
    list_headp: &mut Vec<FfVisitedListHdr>,
) -> usize {
    let filename = filename.into_c_char_ptr();
    {
        if let Some(pos) = list_headp.iter().position(|item| {
            path_equal(
                filename,
                item.ffvl_filename.as_ptr().cast_mut(),
                K_PATH_CMP_LITERAL,
            )
        }) {
            return pos;
        }

        let bytes = crate::memory::cstr_slice(filename, filenamelen);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        list_headp.push(FfVisitedListHdr {
            ffvl_filename: std::ffi::CString::new(&bytes[..end]).unwrap(),
            ffvl_visited_list: Vec::new(),
        });
        list_headp.len() - 1
    }
}

/// Check if two wildcard paths are equal.
fn ff_wc_equal(
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
) -> bool {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    unsafe {
        if s1 == s2 {
            return true;
        }
        if s1.is_null() || s2.is_null() {
            return false;
        }

        let mut i: usize = 0;
        let mut j: usize = 0;
        let mut prev1: i32 = 0;
        let mut prev2: i32 = 0;

        while crate::memory::cstr_byte_at(s1, i) != 0 && crate::memory::cstr_byte_at(s2, j) != 0 {
            let c1 = utf_ptr2char(s1.wrapping_add(i));
            let c2 = utf_ptr2char(s2.wrapping_add(j));

            let neq = if opt_vars().p_fic != 0 {
                mb_tolower(c1) != mb_tolower(c2)
            } else {
                c1 != c2
            };
            if neq && (prev1 != b'*' as i32 || prev2 != b'*' as i32) {
                return false;
            }
            prev2 = prev1;
            prev1 = c1;

            i += utfc_ptr2len(s1.wrapping_add(i)) as usize;
            j += utfc_ptr2len(s2.wrapping_add(j)) as usize;
        }
        *s1.wrapping_add(i) == *s2.wrapping_add(j)
    }
}

/// Maintain the list of already visited files and dirs.
fn ff_check_visited(
    visited_list: &mut Vec<FfVisited>,
    fname: impl crate::strings::IntoCCharPtr,
    fnamelen: usize,
    wc_path: impl crate::strings::IntoCCharPtr,
    wc_pathlen: usize,
) -> bool {
    let fname = fname.into_c_char_ptr();
    let wc_path = wc_path.into_c_char_ptr();
    {
        let mut url = false;
        let mut file_id = FileID::new();
        let mut expand_buf: [u8; MAXPATHL] = [0; MAXPATHL];
        let mut expand_size: usize = 0;

        if path_with_url(fname) != 0 {
            crate::memory::cstr_ncpy(expand_buf.as_mut_ptr().cast(), fname, fnamelen);
            expand_size = fnamelen;
            url = true;
        } else {
            if !os_fileid(fname, &mut file_id) {
                return false;
            }
        }

        for vp in &*visited_list {
            if ((url
                && path_equal(
                    vp.ffv_fname.as_ptr().cast(),
                    expand_buf.as_ptr().cast(),
                    K_PATH_CMP_LITERAL,
                ))
                || (!url && vp.file_id_valid && os_fileid_equal(&vp.file_id, &file_id)))
                && ff_wc_equal(
                    vp.ffv_wc_path
                        .as_ref()
                        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut()),
                    wc_path,
                )
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

        let ffv_wc_path = (!wc_path.is_null()).then(|| {
            let bytes = crate::memory::cstr_slice(wc_path, wc_pathlen);
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            std::ffi::CString::new(&bytes[..end]).unwrap()
        });

        visited_list.push(FfVisited {
            ffv_wc_path,
            file_id_valid: !url,
            file_id,
            ffv_fname,
        });

        true
    }
}

fn ff_create_stack_element(
    fix_part: impl crate::strings::IntoCCharPtr,
    mut fix_partlen: usize,
    wc_part: impl crate::strings::IntoCCharPtr,
    mut wc_partlen: usize,
    level: i32,
    star_star_empty: i32,
) -> FfStack {
    let mut fix_part = fix_part.into_c_char_ptr();
    let mut wc_part = wc_part.into_c_char_ptr();
    if fix_part.is_null() {
        fix_part = EMPTY_C.as_ptr();
        fix_partlen = 0;
    }
    let ffs_fix_path = cbuf_to_string(fix_part, fix_partlen);

    if wc_part.is_null() {
        wc_part = EMPTY_C.as_ptr();
        wc_partlen = 0;
    }
    let ffs_wc_path = cbuf_to_string(wc_part, wc_partlen);

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

fn ff_path_in_stoplist(
    path: impl crate::strings::IntoCCharPtr,
    mut path_len: usize,
    stopdirs_v: &[std::ffi::CString],
) -> bool {
    let path = path.into_c_char_ptr();
    {
        while path_len > 1
            && vim_ispathsep(i32::from(crate::memory::cstr_byte_at(path, path_len - 1)))
        {
            path_len -= 1;
        }

        if path_len == 0 {
            return true;
        }

        for stopdir in stopdirs_v {
            let bytes = stopdir.to_bytes();
            if path_cmp(opt_vars().p_fic != 0, stopdir.as_ptr(), path, path_len) == 0
                && (bytes.len() <= path_len || vim_ispathsep(i32::from(bytes[path_len])))
            {
                return true;
            }
        }

        false
    }
}

/// Find the file name `ptr[len]` in the path. Also finds directory names.
pub fn find_file_in_path(
    ptr: impl crate::strings::IntoCCharPtr,
    len: usize,
    options: i32,
    first: i32,
    rel_fname: impl crate::strings::IntoCCharPtr,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<FfSearchCtx>,
) -> Option<std::ffi::CString> {
    let rel_fname = rel_fname.into_c_char_ptr();
    {
        find_file_in_path_option(
            ptr,
            len,
            options,
            first,
            if crate::memory::cstr_is_empty(curbuf_mut().b_p_path) {
                opt_vars().p_path
            } else {
                curbuf_mut().b_p_path
            },
            FINDFILE_BOTH,
            rel_fname,
            curbuf_mut().b_p_sua,
            file_to_find,
            search_ctx,
        )
    }
}

pub fn free_findfile() {}

/// Find the directory name `ptr[len]` in the path.
pub fn find_directory_in_path(
    ptr: impl crate::strings::IntoCCharPtr,
    len: usize,
    options: i32,
    rel_fname: impl crate::strings::IntoCCharPtr,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<FfSearchCtx>,
) -> Option<std::ffi::CString> {
    let ptr = ptr.into_c_char_ptr().cast_mut();
    let rel_fname = rel_fname.into_c_char_ptr();
    find_file_in_path_option(
        ptr,
        len,
        options,
        i32::from(true),
        opt_vars().p_cdpath,
        FINDFILE_DIR,
        rel_fname,
        EMPTY_C,
        file_to_find,
        search_ctx,
    )
}

pub fn find_file_in_path_option(
    ptr: impl crate::strings::IntoCCharPtr,
    len: usize,
    options: i32,
    first: i32,
    path_option: impl crate::strings::IntoCCharPtr,
    find_what: i32,
    rel_fname: impl crate::strings::IntoCCharPtr,
    suffixes: impl crate::strings::IntoCCharPtr,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<FfSearchCtx>,
) -> Option<std::ffi::CString> {
    let ptr = ptr.into_c_char_ptr().cast_mut();
    let path_option = path_option.into_c_char_ptr().cast_mut();
    let mut rel_fname = rel_fname.into_c_char_ptr();
    let suffixes = suffixes.into_c_char_ptr().cast_mut();
    {
        struct FindFilePathOptState {
            dir: *mut u8,
            did_findfile_init: bool,
            file_to_findlen: usize,
        }
        static FF_OPT_STATE: GlobalState<FindFilePathOptState> =
            GlobalState::new(FindFilePathOptState {
                dir: ptr::null_mut(),
                did_findfile_init: false,
                file_to_findlen: 0,
            });
        let ff_opt = unsafe { &mut *FF_OPT_STATE.as_ptr() };
        let mut file_name: Option<std::ffi::CString> = None;
        let name_buff = g_st().name_buff.as_mut_ptr();

        if !rel_fname.is_null() && path_with_url(rel_fname) != 0 {
            rel_fname = ptr::null_mut();
        }

        if first == i32::from(true) {
            if len == 0 {
                return None;
            }

            let save_char = unsafe { *ptr.wrapping_add(len) };
            unsafe {
                *ptr.wrapping_add(len) = NUL;
            }
            ff_opt.file_to_findlen = expand_env_esc(
                ptr,
                name_buff,
                MAXPATHL as i32,
                ptr::null_mut(),
                true,
                ptr::null_mut(),
            );
            unsafe {
                *ptr.wrapping_add(len) = save_char;
            }

            let mut ftf_bytes =
                crate::memory::cstr_slice(name_buff, ff_opt.file_to_findlen).to_vec();
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

        let ftf = file_to_find
            .as_ref()
            .map_or(c"".as_ptr().cast_mut(), |s| s.as_ptr().cast_mut());
        let rel_to_curdir = crate::memory::cstr_first_byte(ftf) == b'.'
            && (crate::memory::cstr_byte_at(ftf, 1) == 0
                || vim_ispathsep(i32::from(crate::memory::cstr_byte_at(ftf, 1)))
                || (crate::memory::cstr_byte_at(ftf, 1) == b'.'
                    && (crate::memory::cstr_byte_at(ftf, 2) == 0
                        || vim_ispathsep(i32::from(crate::memory::cstr_byte_at(ftf, 2))))));
        let is_mswin_abs = cfg!(windows)
            && (vim_ispathsep(i32::from(crate::memory::cstr_first_byte(ftf)))
                || (!crate::memory::cstr_is_empty(ftf)
                    && crate::memory::cstr_byte_at(ftf, 1) == b':'));

        if vim_isAbsName(ftf) || rel_to_curdir || is_mswin_abs {
            if first == i32::from(true) {
                if path_with_url(ftf) != 0 {
                    return Some(
                        std::ffi::CString::new(unsafe {
                            std::slice::from_raw_parts(ftf.cast::<u8>(), ff_opt.file_to_findlen)
                        })
                        .unwrap_or_default(),
                    );
                }

                let rel_fnamelen = if !rel_fname.is_null() {
                    crate::memory::cstr_len(rel_fname)
                } else {
                    0
                };

                let mut run = 1;
                while run <= 2 {
                    let mut l = ff_opt.file_to_findlen;
                    if run == 1
                        && rel_to_curdir
                        && (options & FNAME_REL) != 0
                        && !rel_fname.is_null()
                        && rel_fnamelen + l < MAXPATHL
                    {
                        l = vim_snprintf!(
                            name_buff,
                            MAXPATHL,
                            c"%.*s%s",
                            crate::memory::ptr_offset_from(path_tail(rel_fname), rel_fname) as i32,
                            rel_fname,
                            ftf,
                        ) as usize;
                    } else {
                        xstrlcpy(name_buff, ftf, MAXPATHL);
                        run = 2;
                    }

                    let mut suffix = suffixes;
                    loop {
                        if os_path_exists(name_buff)
                            && (find_what == FINDFILE_BOTH
                                || ((find_what == FINDFILE_DIR) == os_isdir(name_buff)))
                        {
                            let slen = simplify_filename(name_buff.cast());
                            return Some(
                                std::ffi::CString::new(unsafe {
                                    std::slice::from_raw_parts(name_buff.cast::<u8>(), slen)
                                })
                                .unwrap_or_default(),
                            );
                        }
                        if crate::memory::cstr_is_empty(suffix) {
                            break;
                        }
                        copy_option_part(
                            &raw mut suffix,
                            name_buff.wrapping_add(l).cast(),
                            MAXPATHL - l,
                            c",",
                        );
                    }
                    run += 1;
                }
            }
        } else {
            if first == i32::from(true) {
                vim_findfile_free_visited(search_ctx.as_mut());
                ff_opt.dir = path_option.cast();
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
                    if crate::memory::cstr_is_empty(ff_opt.dir.cast()) {
                        *search_ctx = None;
                        break;
                    }

                    let mut buf = [0u8; MAXPATHL];
                    copy_option_part(&raw mut ff_opt.dir, buf.as_mut_ptr(), MAXPATHL, c" ,");

                    let r_ptr = vim_findfile_stopdir(buf.as_mut_ptr());
                    *search_ctx = vim_findfile_init(
                        buf.as_mut_ptr().cast(),
                        ftf,
                        ff_opt.file_to_findlen,
                        r_ptr.cast(),
                        100,
                        i32::from(false),
                        find_what,
                        search_ctx.take(),
                        i32::from(false),
                        rel_fname,
                    );
                    if search_ctx.is_some() {
                        ff_opt.did_findfile_init = true;
                    }
                }
            }
        }

        if file_name.is_none() && (options & FNAME_MESS) != 0 {
            if first == i32::from(true) {
                if find_what == FINDFILE_DIR {
                    semsg!(tr(E_CANT_FIND_DIRECTORY_STR_IN_CDPATH), ftf);
                } else {
                    semsg!(tr(E_CANT_FIND_FILE_STR_IN_PATH), ftf);
                }
            } else if find_what == FINDFILE_DIR {
                semsg!(tr(E_NO_MORE_DIRECTORY_STR_FOUND_IN_CDPATH), ftf);
            } else {
                semsg!(tr(E_NO_MORE_FILE_STR_FOUND_IN_PATH), ftf);
            }
        }

        file_name
    }
}

/// Get the file name at the cursor.
pub fn grab_file_name(count: i32, file_lnum: &mut Linenr) -> Option<std::ffi::CString> {
    let options = FNAME_MESS | FNAME_EXP | FNAME_REL | FNAME_UNESC;
    let fname = if g_st().visual_global.active {
        let (vis_cstr, len) = get_visual_text(None)?;
        let vis_bytes = vis_cstr.to_bytes();
        if vis_bytes.get(len) == Some(&b':')
            && vis_bytes.get(len + 1).is_some_and(u8::is_ascii_digit)
        {
            let mut p = vis_cstr.as_ptr().wrapping_add(len + 1).cast_mut();
            *file_lnum = getdigits_int32(&raw mut p, false, 0);
        }
        find_file_name_in_path(
            vis_cstr.as_ptr().cast_mut(),
            len,
            options,
            count as i64,
            curbuf_mut().b_ffname,
        )
    } else {
        file_name_at_cursor(options | FNAME_HYP, count, Some(&mut *file_lnum))
    }?;
    #[cfg(windows)]
    let fname = {
        let mut bytes: Vec<u8> = fname.into_bytes_with_nul();
        path_to_slash(bytes.as_mut_ptr().cast());
        crate::memory::cstr_to_cstring(bytes.as_ptr().cast())
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
        get_cursor_line_ptr(),
        curwin_mut().w_cursor.col,
        options,
        count,
        curbuf_mut().b_ffname,
        (file_lnum.is_some()).then(|| &mut *file_lnum.unwrap()),
    )
}

/// Return the name of the file under or after `line[col]`.
pub fn file_name_in_line(
    line: impl crate::strings::IntoCCharPtr,
    col: i32,
    options: i32,
    count: i32,
    rel_fname: impl crate::strings::IntoCCharPtr,
    file_lnum: Option<&mut Linenr>,
) -> Option<std::ffi::CString> {
    let line = line.into_c_char_ptr().cast_mut();
    let rel_fname = rel_fname.into_c_char_ptr();
    {
        let mut ptr = line.wrapping_add(col as usize);
        while !crate::memory::cstr_is_empty(ptr)
            && !vim_isfilec(i32::from(crate::memory::cstr_first_byte(ptr)))
        {
            ptr = ptr.wrapping_add(utfc_ptr2len(ptr) as usize);
        }
        if crate::memory::cstr_is_empty(ptr) {
            if (options & FNAME_MESS) != 0 {
                emsg(tr(c"E446: No file name under cursor"));
            }
            return None;
        }

        let mut in_type = true;
        let mut is_url = false;

        while ptr > line {
            let head_off = utf_head_off(line, ptr.wrapping_sub(1)) as usize;
            if head_off > 0 {
                ptr = ptr.wrapping_sub(head_off + 1);
            } else if vim_isfilec(i32::from(unsafe { *ptr.wrapping_sub(1) } as u8))
                || ((options & FNAME_HYP) != 0 && path_is_url(ptr.wrapping_sub(1)) != 0)
            {
                ptr = ptr.wrapping_sub(1);
            } else {
                break;
            }
        }

        let mut len = if path_has_drive_letter(ptr, crate::memory::cstr_len(ptr)) {
            2
        } else {
            0
        };
        while vim_isfilec(i32::from(crate::memory::cstr_byte_at(ptr, len)))
            || (crate::memory::cstr_byte_at(ptr, len) == b'\\'
                && crate::memory::cstr_byte_at(ptr, len + 1) == b' ')
            || ((options & FNAME_HYP) != 0 && path_is_url(ptr.wrapping_add(len)) != 0)
            || (is_url
                && crate::strings::vim_haschr(
                    c":?&=",
                    i32::from(crate::memory::cstr_byte_at(ptr, len)),
                ))
        {
            let b = crate::memory::cstr_byte_at(ptr, len);
            if b.is_ascii_alphabetic() {
                if in_type && path_is_url(ptr.wrapping_add(len + 1)) != 0 {
                    is_url = true;
                }
            } else {
                in_type = false;
            }

            if crate::memory::cstr_byte_at(ptr, len) == b'\\'
                && crate::memory::cstr_byte_at(ptr, len + 1) == b' '
            {
                len += 1;
            }
            len += utfc_ptr2len(ptr.wrapping_add(len)) as usize;
        }

        if len > 2
            && crate::strings::vim_haschr(
                c".,:;!",
                i32::from(crate::memory::cstr_byte_at(ptr, len - 1)),
            )
            && crate::memory::cstr_byte_at(ptr, len - 2) != b'.'
        {
            len -= 1;
        }

        if let Some(file_lnum) = file_lnum {
            let english = b" line ";
            let mut p = ptr.wrapping_add(len);
            let p_bytes = crate::memory::cstr_as_bytes(p);
            if p_bytes.starts_with(english) {
                p = p.wrapping_add(english.len());
            } else {
                let loc_msg = crate::memory::cstr_as_bytes(tr(LINE_MSG));
                if p_bytes.starts_with(loc_msg) {
                    p = p.wrapping_add(loc_msg.len());
                } else {
                    p = skipwhite(p);
                }
            }
            if !crate::memory::cstr_is_empty(p) {
                if !(crate::memory::cstr_first_byte(p)).is_ascii_digit() {
                    p = p.wrapping_add(1);
                }
                p = skipwhite(p);
                if (crate::memory::cstr_first_byte(p)).is_ascii_digit() {
                    *file_lnum = getdigits_long(&raw mut p, false, 0) as Linenr;
                }
            }
        }

        find_file_name_in_path(ptr, len, options, count as i64, rel_fname)
    }
}

fn eval_includeexpr(
    ptr: impl crate::strings::IntoCCharPtr,
    len: usize,
) -> Option<std::ffi::CString> {
    let ptr = ptr.into_c_char_ptr();
    {
        let save_sctx = g_st().current_sctx;
        set_vim_var_string(VV_FNAME, ptr, len as isize);
        g_st().current_sctx = curbuf_mut().b_p_script_ctx[K_BUF_OPT_INCLUDEEXPR];

        let use_sandbox = was_set_insecurely(
            Some(crate::globals::curwin_mut()),
            K_OPT_INCLUDEEXPR,
            OPT_LOCAL,
        ) != 0;
        let mut tv: TypVal = TypVal::new();
        let mut res = None;
        if eval_expr_option_tv(&mut curbuf_mut().b_p_inex, use_sandbox, &mut tv) {
            let s = tv_get_string(&tv);
            res = Some(crate::memory::cstr_to_cstring(s));
            tv_clear(&mut tv);
        }

        set_vim_var_string(VV_FNAME, ptr::null(), 0);
        g_st().current_sctx = save_sctx;
        res
    }
}

/// Return the name of the file `ptr[len]` in `'path'`.
pub fn find_file_name_in_path(
    ptr: impl crate::strings::IntoCCharPtr,
    mut len: usize,
    options: i32,
    mut count: i64,
    rel_fname: impl crate::strings::IntoCCharPtr,
) -> Option<std::ffi::CString> {
    let mut ptr = ptr.into_c_char_ptr().cast_mut();
    let rel_fname = rel_fname.into_c_char_ptr();
    {
        let mut tofree: Option<std::ffi::CString> = None;

        if len == 0 {
            return None;
        }

        if (options & FNAME_HYP) != 0
            && len > 6
            && crate::memory::cstr_slice(ptr, 6) == b"file:/"
            && !vim_ispathsep(i32::from(crate::memory::cstr_byte_at(ptr, 6)))
        {
            let off = if path_has_drive_letter(ptr.wrapping_add(6), len - 6) {
                6
            } else {
                5
            };
            ptr = ptr.wrapping_add(off);
            len -= off;
        }

        if (options & FNAME_INCL) != 0 && curbuf_mut().b_p_inex.cb_type() != K_CALLBACK_NONE_CONST {
            tofree = eval_includeexpr(ptr, len);
            if let Some(ref tf) = tofree {
                ptr = tf.as_ptr().cast_mut();
                len = tf.as_bytes().len();
            }
        }

        let mut file_name: Option<std::ffi::CString>;
        if (options & FNAME_EXP) != 0 {
            let mut file_to_find: Option<std::ffi::CString> = None;
            let mut search_ctx: Option<FfSearchCtx> = None;

            file_name = find_file_in_path(
                ptr,
                len,
                options & !FNAME_MESS,
                i32::from(true),
                rel_fname,
                &mut file_to_find,
                &mut search_ctx,
            );

            if file_name.is_none()
                && (options & FNAME_INCL) == 0
                && curbuf_mut().b_p_inex.cb_type() != K_CALLBACK_NONE_CONST
            {
                tofree = eval_includeexpr(ptr, len);
                if let Some(ref tf) = tofree {
                    ptr = tf.as_ptr().cast_mut();
                    len = tf.as_bytes().len();
                    file_name = find_file_in_path(
                        ptr,
                        len,
                        options & !FNAME_MESS,
                        i32::from(true),
                        rel_fname,
                        &mut file_to_find,
                        &mut search_ctx,
                    );
                }
            }
            if file_name.is_none() && (options & FNAME_MESS) != 0 {
                let c = unsafe { *ptr.wrapping_add(len) };
                unsafe {
                    *ptr.wrapping_add(len) = NUL;
                }
                semsg!(tr(c"E447: Can't find file \"%s\" in path"), ptr);
                unsafe {
                    *ptr.wrapping_add(len) = c;
                }
            }

            while file_name.is_some() && {
                count -= 1;
                count > 0
            } {
                file_name = find_file_in_path(
                    ptr,
                    len,
                    options,
                    i32::from(false),
                    rel_fname,
                    &mut file_to_find,
                    &mut search_ctx,
                );
            }
        } else {
            file_name = Some(
                std::ffi::CString::new(crate::memory::cstr_slice(ptr, len)).unwrap_or_default(),
            );
        }

        drop(tofree);
        file_name
    }
}

pub fn do_autocmd_dirchanged(
    new_dir: impl crate::strings::IntoCCharPtr,
    scope: CdScope,
    cause: CdCause,
    pre: bool,
) {
    let new_dir = new_dir.into_c_char_ptr();
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
        xstrlcpy(buf.as_mut_ptr().cast(), scope_cstr.as_ptr(), buf.len());

        if pre {
            tv_dict_add_str(&mut *dict, c"directory", new_dir);
        } else {
            tv_dict_add_str(&mut *dict, c"cwd", new_dir);
        }
        tv_dict_add_str(&mut *dict, c"scope", buf.as_ptr().cast());
        tv_dict_add_bool(
            &mut *dict,
            c"changed_window",
            i32::from(cause == K_CD_CAUSE_WINDOW),
        );
        tv_dict_set_keys_readonly(&mut *dict);

        match cause {
            K_CD_CAUSE_MANUAL | K_CD_CAUSE_WINDOW | K_CD_CAUSE_BUFFER => {}
            K_CD_CAUSE_AUTO => {
                xstrlcpy(buf.as_mut_ptr().cast(), c"auto", buf.len());
            }
            _ => std::process::abort(),
        }

        apply_autocmds(
            event,
            buf.as_mut_ptr().cast(),
            new_dir,
            false,
            g_st().curbuf,
        );

        restore_v_event(&mut *dict, &mut save_v_event);

        RECURSIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Change to a file's directory.
pub fn vim_chdirfile(fname: impl crate::strings::IntoCCharPtr, cause: CdCause) -> bool {
    let fname = fname.into_c_char_ptr();
    {
        let mut dir: [u8; MAXPATHL] = [0; MAXPATHL];
        let name_buff = g_st().name_buff.as_mut_ptr();

        xstrlcpy(dir.as_mut_ptr().cast(), fname, MAXPATHL);
        unsafe {
            *path_tail_with_sep(dir.as_mut_ptr().cast()) = NUL;
        }

        if !os_dirname(name_buff, MAXPATHL) {
            unsafe {
                *name_buff = NUL;
            }
        }

        if path_equal(dir.as_ptr().cast(), name_buff, K_PATH_CMP_LITERAL) {
            return true;
        }

        if cause != K_CD_CAUSE_OTHER {
            do_autocmd_dirchanged(dir.as_mut_ptr().cast(), K_CD_SCOPE_WINDOW, cause, true);
        }

        if os_chdir(dir.as_ptr().cast()) != 0 {
            return false;
        }

        if cause != K_CD_CAUSE_OTHER {
            do_autocmd_dirchanged(dir.as_mut_ptr().cast(), K_CD_SCOPE_WINDOW, cause, false);
        }

        true
    }
}

/// Change directory to `new_dir`. Search `'cdpath'` for relative directory names.
pub fn vim_chdir(new_dir: impl crate::strings::IntoCCharPtr) -> i32 {
    let new_dir = new_dir.into_c_char_ptr();
    let mut file_to_find: Option<std::ffi::CString> = None;
    let mut search_ctx: Option<FfSearchCtx> = None;
    let Some(dir_name) = find_directory_in_path(
        new_dir,
        crate::memory::cstr_len(new_dir),
        FNAME_MESS,
        curbuf_mut().b_ffname,
        &mut file_to_find,
        &mut search_ctx,
    ) else {
        return -1;
    };

    os_chdir(dir_name.as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_findfile_stopdir_splits_and_unescapes_semicolons() {
        let mut buf = *b"/a/b;/c/d\0";
        let stop = vim_findfile_stopdir(buf.as_mut_ptr());
        assert!(!stop.is_null());
        let first = crate::memory::cstr_as_bytes(buf.as_ptr().cast());
        let rest = crate::memory::cstr_as_bytes(stop.cast());
        assert_eq!(first, b"/a/b");
        assert_eq!(rest, b"/c/d");

        let mut escaped = *b"/a\\;b;/c\0";
        let stop2 = vim_findfile_stopdir(escaped.as_mut_ptr());
        assert!(!stop2.is_null());
        let first2 = crate::memory::cstr_as_bytes(escaped.as_ptr().cast());
        let rest2 = crate::memory::cstr_as_bytes(stop2.cast());
        assert_eq!(first2, b"/a;b");
        assert_eq!(rest2, b"/c");

        let mut none = *b"/a/b\0";
        let stop3 = vim_findfile_stopdir(none.as_mut_ptr());
        assert!(stop3.is_null());
    }
}
