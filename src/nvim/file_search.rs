//! File searching functions for `'path'`, `'tags'`, and `'cdpath'` options (`src/nvim/file_search.c`).

use crate::api::private::helpers::{
    api_clear_string, cbuf_to_string, copy_string, cstr_to_string,
};
use crate::api::private::validate::NvimString;
use crate::autocmd::{EVENT_DIRCHANGED, EVENT_DIRCHANGEDPRE, apply_autocmds, has_event};
use crate::charset::{NUL, getdigits, getdigits_int32, getdigits_long, skipwhite, vim_isfilec};
use crate::context::{CdCause, kCdCauseWindow};
use crate::cursor::{get_cursor_line_ptr, linenr_T};
use crate::errors::*;
use crate::eval::typval::{
    kCallbackNone, tv_clear, tv_dict_add_bool, tv_dict_add_str, tv_dict_set_keys_readonly,
    tv_get_string, typval_T,
};
use crate::eval::vars::{VV_FNAME, set_vim_var_string};
use crate::eval::{eval_expr_option_tv, get_v_event, restore_v_event};
use crate::globals::*;
use crate::mbyte::{mb_tolower, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memory::{xstrlcpy};
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
    EW_ADDSLASH, EW_DIR, EW_NOTWILD, EW_SILENT, FullName_save, PATHSEP, after_pathsep,
    expand_wildcards_vec, kPathCmpLiteral, path_cmp, path_equal, path_has_drive_letter,
    path_is_url, path_shorten_fname, path_tail, path_tail_with_sep, path_with_url,
    simplify_filename, vim_isAbsName, vim_ispathsep,
};
#[cfg(windows)]
use crate::path::{path_to_slash, vim_FullName};
use crate::state::save_v_event_T;
use crate::strings::{tr, vim_snprintf, vim_strchr};
use std::ffi::{CStr, c_char, c_int, c_long};
use std::ptr;

use crate::ex_docmd::{CdScope, kCdCauseManual};
use crate::optionstr::OptIndex;

pub const FINDFILE_FILE: c_int = 0;
pub const FINDFILE_DIR: c_int = 1;
pub const FINDFILE_BOTH: c_int = 2;

pub const FNAME_MESS: c_int = 1;
pub const FNAME_EXP: c_int = 2;
pub const FNAME_HYP: c_int = 4;
pub const FNAME_INCL: c_int = 8;
pub const FNAME_REL: c_int = 16;
pub const FNAME_UNESC: c_int = 32;

const PATHSEPSTR_C: &CStr = c"/";
const EMPTY_C: &CStr = c"";

const FF_MAX_STAR_STAR_EXPAND: c_char = 30;

const kCpoDottag: c_int = b'd' as c_int;

pub(crate) const kOptIncludeexpr: OptIndex = 147;
pub(crate) const kBufOptIncludeexpr: usize = 47;

pub(crate) const NULL_STRING: NvimString = NvimString {
    data: ptr::null_mut(),
    size: 0,
};

const E_PATH_TOO_LONG_FOR_COMPLETION: &CStr = c"E854: Path too long for completion";

use crate::api::vim::kCdScopeGlobal;
use crate::buffer::{kCdCauseAuto, kCdCauseBuffer};
use crate::eval::fs::{kCdScopeBuffer, kCdScopeTabpage, kCdScopeWindow};
use crate::ex_session::kCdCauseOther;
use crate::math::{FAIL, OK};

#[repr(C)]
pub struct ff_stack_T {
    pub ffs_fix_path: NvimString,
    pub ffs_wc_path: NvimString,
    pub ffs_filearray: Vec<std::ffi::CString>,
    pub ffs_filearray_cur: usize,
    pub ffs_stage: c_int,
    pub ffs_level: c_int,
    pub ffs_star_star_empty: c_int,
}

impl Drop for ff_stack_T {
    fn drop(&mut self) {
        unsafe {
            api_clear_string(&mut self.ffs_fix_path);
            api_clear_string(&mut self.ffs_wc_path);
        }
    }
}

#[repr(C)]
pub struct ff_visited_T {
    pub ffv_wc_path: Option<std::ffi::CString>,
    pub file_id_valid: bool,
    pub file_id: FileID,
    pub ffv_fname: Vec<c_char>,
}

#[repr(C)]
pub struct ff_visited_list_hdr_T {
    pub ffvl_filename: std::ffi::CString,
    pub ffvl_visited_list: Vec<ff_visited_T>,
}

#[repr(C)]
pub struct ff_search_ctx_T {
    pub ffsc_stack: Vec<ff_stack_T>,
    pub ffsc_visited_list: Option<usize>,
    pub ffsc_dir_visited_list: Option<usize>,
    pub ffsc_visited_lists_list: Vec<ff_visited_list_hdr_T>,
    pub ffsc_dir_visited_lists_list: Vec<ff_visited_list_hdr_T>,
    pub ffsc_file_to_search: NvimString,
    pub ffsc_start_dir: NvimString,
    pub ffsc_fix_path: NvimString,
    pub ffsc_wc_path: NvimString,
    pub ffsc_level: c_int,
    pub ffsc_stopdirs_v: Vec<std::ffi::CString>,
    pub ffsc_find_what: c_int,
    pub ffsc_tagfile: c_int,
}

impl ff_search_ctx_T {
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

impl Default for ff_search_ctx_T {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ff_search_ctx_T {
    fn drop(&mut self) {
        unsafe {
            ff_clear(self);
        }
    }
}

/// Initialization routine for `vim_findfile()`.
pub unsafe fn vim_findfile_init(
    mut path: *mut c_char,
    filename: *mut c_char,
    filenamelen: usize,
    stopdirs: *mut c_char,
    level: c_int,
    free_visited: c_int,
    find_what: c_int,
    search_ctx_arg: Option<ff_search_ctx_T>,
    tagfile: c_int,
    rel_fname: *mut c_char,
) -> Option<ff_search_ctx_T> {
    unsafe {
        let mut search_ctx = search_ctx_arg.unwrap_or_default();
        search_ctx.ffsc_find_what = find_what;
        search_ctx.ffsc_tagfile = tagfile;

        ff_clear(&mut search_ctx);

        if free_visited == c_int::from(true) {
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

        let mut expand_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
        let expand_ptr = expand_buf.as_mut_ptr();

        if *path == b'.' as c_char
            && (vim_ispathsep(c_int::from(*path.add(1) as u8)) || *path.add(1) == NUL)
            && (tagfile == 0 || vim_strchr(opt_vars().p_cpo, kCpoDottag).is_null())
            && !rel_fname.is_null()
        {
            let len = path_tail(rel_fname).offset_from(rel_fname) as usize;

            if !vim_isAbsName(rel_fname) && len + 1 < MAXPATHL {
                { let _d = (expand_ptr).cast::<u8>(); let _n = len; if _n > 0 { std::ptr::copy_nonoverlapping((rel_fname).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
                let full = FullName_save(expand_ptr, false).unwrap_or_default();
                search_ctx.ffsc_start_dir =
                    cbuf_to_string(full.as_ptr(), full.as_bytes().len());
            } else {
                search_ctx.ffsc_start_dir = cbuf_to_string(rel_fname, len);
            }
            path = path.add(1);
            if *path != NUL {
                path = path.add(1);
            }
        } else if *path == NUL || !vim_isAbsName(path) {
            #[cfg(windows)]
            let handled_drive = if *path != NUL && *path.add(1) == b':' as c_char {
                let drive: [c_char; 3] = [*path, b':' as c_char, NUL];
                if vim_FullName(drive.as_ptr(), expand_ptr, MAXPATHL, true) == FAIL {
                    return None;
                }
                path = path.add(2);
                true
            } else {
                false
            };
            #[cfg(not(windows))]
            let handled_drive = false;

            if !handled_drive && os_dirname(expand_ptr, MAXPATHL) == FAIL {
                return None;
            }
            let expand_len = CStr::from_ptr(expand_ptr).to_bytes().len();

            search_ctx.ffsc_start_dir = copy_string(
                NvimString {
                    data: expand_ptr,
                    size: expand_len,
                },
                ptr::null_mut(),
            );
        }

        if !stopdirs.is_null() {
            let mut walker = stopdirs;
            while *walker == b';' as c_char {
                walker = walker.add(1);
            }

            loop {
                let helper = walker;
                walker = vim_strchr(walker, b';' as c_int);
                let len = if !walker.is_null() {
                    walker.offset_from(helper) as usize
                } else {
                    CStr::from_ptr(helper).to_bytes().len()
                };
                let dir_str = if *helper != NUL && !vim_isAbsName(helper) && len + 1 < MAXPATHL {
                    { let _d = (expand_ptr).cast::<u8>(); let _n = len; if _n > 0 { std::ptr::copy_nonoverlapping((helper).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
                    FullName_save(helper, len != 0).unwrap_or_default()
                } else {
                    std::ffi::CString::new(std::slice::from_raw_parts(helper.cast::<u8>(), len))
                        .unwrap_or_default()
                };
                search_ctx.ffsc_stopdirs_v.push(dir_str);
                if !walker.is_null() {
                    walker = walker.add(1);
                }
                if walker.is_null() {
                    break;
                }
            }
        }

        search_ctx.ffsc_level = level;

        let mut wc_part = vim_strchr(path, b'*' as c_int);
        let mut expand_size: usize;
        if !wc_part.is_null() {
            search_ctx.ffsc_fix_path = cbuf_to_string(path, wc_part.offset_from(path) as usize);

            expand_size = 0;
            while *wc_part != NUL {
                if expand_size + 5 >= MAXPATHL {
                    emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION.as_ptr()));
                    break;
                }
                if *wc_part == b'*' as c_char && *wc_part.add(1) == b'*' as c_char {
                    *expand_ptr.add(expand_size) = *wc_part;
                    expand_size += 1;
                    wc_part = wc_part.add(1);
                    *expand_ptr.add(expand_size) = *wc_part;
                    expand_size += 1;
                    wc_part = wc_part.add(1);

                    let mut errpt: *mut c_char = wc_part;
                    let llevel = getdigits(&raw mut errpt, false, 255);
                    if errpt != wc_part && llevel > 0 && llevel < 255 {
                        *expand_ptr.add(expand_size) = llevel as c_char;
                        expand_size += 1;
                    } else if errpt != wc_part && llevel == 0 {
                        expand_size -= 2;
                    } else {
                        *expand_ptr.add(expand_size) = FF_MAX_STAR_STAR_EXPAND;
                        expand_size += 1;
                    }
                    wc_part = errpt;
                    if *wc_part != NUL && !vim_ispathsep(c_int::from(*wc_part as u8)) {
                        semsg!(
                            tr(c"E343: Invalid path: '**[number]' must be at the end of the path or be followed by '%s'."
                                .as_ptr()),
                            PATHSEPSTR_C.as_ptr(),
                        );
                        return None;
                    }
                } else {
                    *expand_ptr.add(expand_size) = *wc_part;
                    expand_size += 1;
                    wc_part = wc_part.add(1);
                }
            }
            *expand_ptr.add(expand_size) = NUL;
            search_ctx.ffsc_wc_path = copy_string(
                NvimString {
                    data: expand_ptr,
                    size: expand_size,
                },
                ptr::null_mut(),
            );
        } else {
            search_ctx.ffsc_fix_path = cstr_to_string(path);
        }

        if search_ctx.ffsc_start_dir.data.is_null() {
            search_ctx.ffsc_start_dir = copy_string(search_ctx.ffsc_fix_path, ptr::null_mut());
            *search_ctx.ffsc_fix_path.data = NUL;
            search_ctx.ffsc_fix_path.size = 0;
        }

        if search_ctx.ffsc_start_dir.size + search_ctx.ffsc_fix_path.size + 3 >= MAXPATHL {
            emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION.as_ptr()));
            return None;
        }

        let mut add_sep = after_pathsep(
            search_ctx.ffsc_start_dir.data,
            search_ctx
                .ffsc_start_dir
                .data
                .add(search_ctx.ffsc_start_dir.size),
        ) == 0;
        expand_size = vim_snprintf!(
            expand_ptr,
            MAXPATHL,
            c"%s%s".as_ptr(),
            search_ctx.ffsc_start_dir.data,
            if add_sep {
                PATHSEPSTR_C.as_ptr()
            } else {
                EMPTY_C.as_ptr()
            },
        ) as usize;

        {
            let bufsize = expand_size + search_ctx.ffsc_fix_path.size + 1;
            let mut buf_vec = vec![0 as c_char; bufsize];
            let buf = buf_vec.as_mut_ptr();

            vim_snprintf!(
                buf,
                bufsize,
                c"%s%s".as_ptr(),
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
                            .add(search_ctx.ffsc_fix_path.size),
                    ) == 0;
                    expand_size += vim_snprintf!(
                        expand_ptr.add(expand_size),
                        MAXPATHL - expand_size,
                        c"%s%s".as_ptr(),
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
                let mut len = search_ctx.ffsc_fix_path.size as c_int;

                if p > search_ctx.ffsc_fix_path.data {
                    len = p.offset_from(search_ctx.ffsc_fix_path.data) as c_int - 1;
                    if len >= 2
                        && *search_ctx.ffsc_fix_path.data == b'.' as c_char
                        && *search_ctx.ffsc_fix_path.data.add(1) == b'.' as c_char
                        && (len == 2 || *search_ctx.ffsc_fix_path.data.add(2) == PATHSEP)
                    {
                        return None;
                    }

                    add_sep = after_pathsep(
                        search_ctx.ffsc_fix_path.data,
                        search_ctx
                            .ffsc_fix_path
                            .data
                            .add(search_ctx.ffsc_fix_path.size),
                    ) == 0;
                    expand_size += vim_snprintf!(
                        expand_ptr.add(expand_size),
                        MAXPATHL - expand_size,
                        c"%.*s%s".as_ptr(),
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
                    let mut temp_vec: Vec<c_char> = vec![0; tempsize];
                    let temp_len = vim_snprintf!(
                        temp_vec.as_mut_ptr(),
                        tempsize,
                        c"%s%s".as_ptr(),
                        search_ctx.ffsc_fix_path.data.add(len as usize),
                        search_ctx.ffsc_wc_path.data,
                    ) as usize;
                    api_clear_string(&mut search_ctx.ffsc_wc_path);
                    search_ctx.ffsc_wc_path = cbuf_to_string(temp_vec.as_ptr(), temp_len);
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
pub unsafe fn vim_findfile_stopdir(mut buf: *mut c_char) -> *mut c_char {
    unsafe {
        while *buf != NUL
            && *buf != b';' as c_char
            && (*buf != b'\\' as c_char || *buf.add(1) != b';' as c_char)
        {
            buf = buf.add(1);
        }
        let mut dst = buf;
        if *buf == b';' as c_char {
            *buf = NUL;
            return buf.add(1);
        }
        if *buf == NUL {
            return ptr::null_mut();
        }
        *dst = b';' as c_char;
        dst = dst.add(1);
        buf = buf.add(2);
        while *buf != NUL && *buf != b';' as c_char {
            if *buf == b'\\' as c_char && *buf.add(1) == b';' as c_char {
                *dst = b';' as c_char;
                dst = dst.add(1);
                buf = buf.add(2);
            } else {
                *dst = *buf;
                dst = dst.add(1);
                buf = buf.add(1);
            }
        }
        debug_assert!(dst < buf);
        *dst = NUL;
        if *buf == b';' as c_char {
            *buf = NUL;
            buf.add(1)
        } else {
            ptr::null_mut()
        }
    }
}

/// Find a file in a search context.
pub unsafe fn vim_findfile(search_ctx: Option<&mut ff_search_ctx_T>) -> Option<std::ffi::CString> {
    unsafe {
        let search_ctx = search_ctx?;

        let mut file_path_vec: Vec<c_char> = vec![0; MAXPATHL];
        let mut file_path = NvimString {
            data: file_path_vec.as_mut_ptr(),
            size: 0,
        };
        let mut path_end = if !search_ctx.ffsc_start_dir.data.is_null() {
            search_ctx
                .ffsc_start_dir
                .data
                .add(search_ctx.ffsc_start_dir.size)
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
                let stackp: *mut ff_stack_T = &raw mut stack_elem;

                if (*stackp).ffs_filearray.is_empty() {
                    let dir_vis_idx = search_ctx.ffsc_dir_visited_list.unwrap();
                    if ff_check_visited(
                        &mut search_ctx.ffsc_dir_visited_lists_list[dir_vis_idx].ffvl_visited_list,
                        (*stackp).ffs_fix_path.data,
                        (*stackp).ffs_fix_path.size,
                        (*stackp).ffs_wc_path.data,
                        (*stackp).ffs_wc_path.size,
                    ) == FAIL
                    {
                        continue;
                    }
                }

                if (*stackp).ffs_level <= 0 {
                    continue;
                }

                *file_path.data = NUL;
                file_path.size = 0;

                let mut rest_of_wildcards: NvimString;
                if (*stackp).ffs_filearray.is_empty() {
                    let mut dirptrs: [*mut c_char; 2] = [file_path.data, ptr::null_mut()];

                    if !vim_isAbsName((*stackp).ffs_fix_path.data)
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
                                .add(search_ctx.ffsc_start_dir.size),
                        ) == 0;
                        file_path.size = vim_snprintf!(
                            file_path.data,
                            MAXPATHL,
                            c"%s%s".as_ptr(),
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

                    if file_path.size + (*stackp).ffs_fix_path.size + 1 >= MAXPATHL {
                        break 'upward;
                    }
                    let add_sep = after_pathsep(
                        (*stackp).ffs_fix_path.data,
                        (*stackp).ffs_fix_path.data.add((*stackp).ffs_fix_path.size),
                    ) == 0;
                    file_path.size += vim_snprintf!(
                        file_path.data.add(file_path.size),
                        MAXPATHL - file_path.size,
                        c"%s%s".as_ptr(),
                        (*stackp).ffs_fix_path.data,
                        if add_sep {
                            PATHSEPSTR_C.as_ptr()
                        } else {
                            EMPTY_C.as_ptr()
                        },
                    ) as usize;
                    if file_path.size >= MAXPATHL {
                        break 'upward;
                    }

                    rest_of_wildcards = (*stackp).ffs_wc_path;
                    if *rest_of_wildcards.data != NUL {
                        if *rest_of_wildcards.data == b'*' as c_char
                            && *rest_of_wildcards.data.add(1) == b'*' as c_char
                        {
                            let p = rest_of_wildcards.data.add(2);

                            if *p > 0 {
                                *p -= 1;
                                if file_path.size + 1 >= MAXPATHL {
                                    break 'upward;
                                }
                                *file_path.data.add(file_path.size) = b'*' as c_char;
                                file_path.size += 1;
                            }

                            if *p == 0 {
                                ptr::copy(
                                    rest_of_wildcards.data.add(3),
                                    rest_of_wildcards.data,
                                    (rest_of_wildcards.size - 3) + 1,
                                );
                                rest_of_wildcards.size -= 3;
                                (*stackp).ffs_wc_path.size = rest_of_wildcards.size;
                            } else {
                                rest_of_wildcards.data = rest_of_wildcards.data.add(3);
                                rest_of_wildcards.size -= 3;
                            }

                            if (*stackp).ffs_star_star_empty == 0 {
                                (*stackp).ffs_star_star_empty = 1;
                                dirptrs[1] = (*stackp).ffs_fix_path.data;
                            }
                        }

                        while *rest_of_wildcards.data != NUL
                            && !vim_ispathsep(c_int::from(*rest_of_wildcards.data as u8))
                        {
                            if file_path.size + 1 >= MAXPATHL {
                                break 'upward;
                            }
                            *file_path.data.add(file_path.size) = *rest_of_wildcards.data;
                            file_path.size += 1;
                            rest_of_wildcards.data = rest_of_wildcards.data.add(1);
                            rest_of_wildcards.size -= 1;
                        }

                        *file_path.data.add(file_path.size) = NUL;
                        if vim_ispathsep(c_int::from(*rest_of_wildcards.data as u8)) {
                            rest_of_wildcards.data = rest_of_wildcards.data.add(1);
                            rest_of_wildcards.size -= 1;
                        }
                    }

                    if path_with_url(dirptrs[0]) != 0 {
                        (*stackp).ffs_filearray = vec![crate::memory::cstr_from_bytes(std::slice::from_raw_parts(dirptrs[0].cast::<u8>(), crate::memory::xstrnlen(dirptrs[0], file_path.size)))];
                    } else {
                        expand_wildcards_vec(
                            if dirptrs[1].is_null() { 1 } else { 2 },
                            dirptrs.as_mut_ptr(),
                            &mut (*stackp).ffs_filearray,
                            EW_DIR | EW_ADDSLASH | EW_SILENT | EW_NOTWILD,
                        );
                    }

                    (*stackp).ffs_filearray_cur = 0;
                    (*stackp).ffs_stage = 0;
                } else {
                    rest_of_wildcards = NvimString {
                        data: (*stackp).ffs_wc_path.data.add((*stackp).ffs_wc_path.size),
                        size: 0,
                    };
                }

                if (*stackp).ffs_stage == 0 {
                    if *rest_of_wildcards.data == NUL {
                        for i in (*stackp).ffs_filearray_cur..(*stackp).ffs_filearray.len() {
                            let entry = (*stackp).ffs_filearray.as_slice()[i].as_ptr().cast_mut();
                            if path_with_url(entry) == 0 && !os_isdir(entry) {
                                continue;
                            }
                            let mut len = (*stackp).ffs_filearray.as_slice()[i].as_bytes().len();
                            if len + 1 + search_ctx.ffsc_file_to_search.size >= MAXPATHL {
                                break 'upward;
                            }
                            let add_sep = after_pathsep(entry, entry.add(len)) == 0;
                            file_path.size = vim_snprintf!(
                                file_path.data,
                                MAXPATHL,
                                c"%s%s%s".as_ptr(),
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
                            let mut suf: *mut c_char = if search_ctx.ffsc_tagfile != 0 {
                                EMPTY_C.as_ptr().cast_mut()
                            } else {
                                (*g_st().curbuf).b_p_sua
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
                                        EMPTY_C.as_ptr(),
                                        0,
                                    ) == OK
                                {
                                    (*stackp).ffs_filearray_cur = i + 1;
                                    ff_push(search_ctx, stack_elem);

                                    if path_with_url(file_path.data) == 0 {
                                        file_path.size = simplify_filename(file_path.data);
                                    }

                                    let mut expand_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
                                    if os_dirname(expand_buf.as_mut_ptr(), MAXPATHL) == OK {
                                        let p = path_shorten_fname(
                                            file_path.data,
                                            expand_buf.as_mut_ptr(),
                                        );
                                        if !p.is_null() {
                                            let new_len =
                                                file_path.data.add(file_path.size).offset_from(p)
                                                    as usize;
                                            ptr::copy(p, file_path.data, new_len + 1);
                                            file_path.size = new_len;
                                        }
                                    }
                                    return Some(CStr::from_ptr(file_path.data).to_owned());
                                }

                                if *suf == NUL {
                                    break;
                                }
                                file_path.size = len
                                    + copy_option_part(
                                        &raw mut suf,
                                        file_path.data.add(len),
                                        MAXPATHL - len,
                                        c",".as_ptr(),
                                    );
                            }
                        }
                    } else {
                        for i in (*stackp).ffs_filearray_cur..(*stackp).ffs_filearray.len() {
                            let entry = (*stackp).ffs_filearray.as_slice()[i].as_ptr();
                            let entry_len =
                                (*stackp).ffs_filearray.as_slice()[i].as_bytes().len();
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
                                    (*stackp).ffs_level - 1,
                                    0,
                                ),
                            );
                        }
                    }
                    (*stackp).ffs_filearray_cur = 0;
                    (*stackp).ffs_stage = 1;
                }

                if *(*stackp).ffs_wc_path.data == b'*' as c_char
                    && *(*stackp).ffs_wc_path.data.add(1) == b'*' as c_char
                {
                    for i in (*stackp).ffs_filearray_cur..(*stackp).ffs_filearray.len() {
                        let entry = (*stackp).ffs_filearray.as_slice()[i].as_ptr();
                        let entry_len = (*stackp).ffs_filearray.as_slice()[i].as_bytes().len();
                        if path_equal(entry, (*stackp).ffs_fix_path.data, kPathCmpLiteral) {
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
                                (*stackp).ffs_wc_path.data,
                                (*stackp).ffs_wc_path.size,
                                (*stackp).ffs_level - 1,
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
                let plen = path_end.offset_from(search_ctx.ffsc_start_dir.data)
                    + isize::from(*path_end != NUL);

                if ff_path_in_stoplist(
                    search_ctx.ffsc_start_dir.data,
                    plen as usize,
                    &search_ctx.ffsc_stopdirs_v,
                ) {
                    break;
                }

                while path_end > search_ctx.ffsc_start_dir.data
                    && vim_ispathsep(c_int::from(*path_end as u8))
                {
                    path_end = path_end.sub(1);
                }
                while path_end > search_ctx.ffsc_start_dir.data
                    && !vim_ispathsep(c_int::from(*path_end.sub(1) as u8))
                {
                    path_end = path_end.sub(1);
                }
                *path_end = NUL;

                search_ctx.ffsc_start_dir.size =
                    path_end.offset_from(search_ctx.ffsc_start_dir.data) as usize;
                path_end = path_end.sub(1);

                if *search_ctx.ffsc_start_dir.data == NUL {
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
                        .add(search_ctx.ffsc_start_dir.size),
                ) == 0;
                file_path.size = vim_snprintf!(
                    file_path.data,
                    MAXPATHL,
                    c"%s%s%s".as_ptr(),
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
}

/// Free the list of lists of visited files and directories.
fn vim_findfile_free_visited(search_ctx: Option<&mut ff_search_ctx_T>) {
    let Some(search_ctx) = search_ctx else {
        return;
    };

    search_ctx.ffsc_visited_lists_list.clear();
    search_ctx.ffsc_dir_visited_lists_list.clear();
}

unsafe fn ff_get_visited_list(
    filename: *const c_char,
    filenamelen: usize,
    list_headp: &mut Vec<ff_visited_list_hdr_T>,
) -> usize {
    unsafe {
        if let Some(pos) = list_headp.iter().position(|item| {
            path_equal(
                filename,
                item.ffvl_filename.as_ptr().cast_mut(),
                kPathCmpLiteral,
            )
        }) {
            return pos;
        }

        let bytes = std::slice::from_raw_parts(filename.cast::<u8>(), filenamelen);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        list_headp.push(ff_visited_list_hdr_T {
            ffvl_filename: std::ffi::CString::new(&bytes[..end]).unwrap(),
            ffvl_visited_list: Vec::new(),
        });
        list_headp.len() - 1
    }
}

/// Check if two wildcard paths are equal.
unsafe fn ff_wc_equal(s1: *const c_char, s2: *const c_char) -> bool {
    unsafe {
        if s1 == s2 {
            return true;
        }
        if s1.is_null() || s2.is_null() {
            return false;
        }

        let mut i: usize = 0;
        let mut j: usize = 0;
        let mut prev1: c_int = 0;
        let mut prev2: c_int = 0;

        while *s1.add(i) != NUL && *s2.add(j) != NUL {
            let c1 = utf_ptr2char(s1.add(i));
            let c2 = utf_ptr2char(s2.add(j));

            let neq = if opt_vars().p_fic != 0 {
                mb_tolower(c1) != mb_tolower(c2)
            } else {
                c1 != c2
            };
            if neq && (prev1 != b'*' as c_int || prev2 != b'*' as c_int) {
                return false;
            }
            prev2 = prev1;
            prev1 = c1;

            i += utfc_ptr2len(s1.add(i)) as usize;
            j += utfc_ptr2len(s2.add(j)) as usize;
        }
        *s1.add(i) == *s2.add(j)
    }
}

/// Maintain the list of already visited files and dirs.
unsafe fn ff_check_visited(
    visited_list: &mut Vec<ff_visited_T>,
    fname: *const c_char,
    fnamelen: usize,
    wc_path: *const c_char,
    wc_pathlen: usize,
) -> c_int {
    unsafe {
        let mut url = false;
        let mut file_id = FileID::new();
        let mut expand_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut expand_size: usize = 0;

        if path_with_url(fname) != 0 {
            { let _d = (expand_buf.as_mut_ptr()).cast::<u8>(); let _n = fnamelen; if _n > 0 { std::ptr::copy_nonoverlapping((fname).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
            expand_size = fnamelen;
            url = true;
        } else {
            if !os_fileid(fname, &raw mut file_id) {
                return FAIL;
            }
        }

        for vp in &*visited_list {
            if ((url
                && path_equal(
                    vp.ffv_fname.as_ptr(),
                    expand_buf.as_ptr(),
                    kPathCmpLiteral,
                ))
                || (!url
                    && vp.file_id_valid
                    && os_fileid_equal(&raw const vp.file_id, &raw const file_id)))
                && ff_wc_equal(
                    vp.ffv_wc_path
                        .as_ref()
                        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut()),
                    wc_path,
                )
            {
                return FAIL;
            }
        }

        let mut ffv_fname = Vec::new();
        if !url {
            ffv_fname.push(NUL);
        } else {
            ffv_fname.extend_from_slice(&expand_buf[..expand_size + 1]);
        }

        let ffv_wc_path = (!wc_path.is_null()).then(|| {
            let bytes = std::slice::from_raw_parts(wc_path.cast::<u8>(), wc_pathlen);
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            std::ffi::CString::new(&bytes[..end]).unwrap()
        });

        visited_list.push(ff_visited_T {
            ffv_wc_path,
            file_id_valid: !url,
            file_id,
            ffv_fname,
        });

        OK
    }
}

unsafe fn ff_create_stack_element(
    mut fix_part: *const c_char,
    mut fix_partlen: usize,
    mut wc_part: *const c_char,
    mut wc_partlen: usize,
    level: c_int,
    star_star_empty: c_int,
) -> ff_stack_T {
    unsafe {
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

        ff_stack_T {
            ffs_fix_path,
            ffs_wc_path,
            ffs_filearray: Vec::new(),
            ffs_filearray_cur: 0,
            ffs_stage: 0,
            ffs_level: level,
            ffs_star_star_empty: star_star_empty,
        }
    }
}

fn ff_push(search_ctx: &mut ff_search_ctx_T, stack_elem: ff_stack_T) {
    search_ctx.ffsc_stack.push(stack_elem);
}

fn ff_pop(search_ctx: &mut ff_search_ctx_T) -> Option<ff_stack_T> {
    search_ctx.ffsc_stack.pop()
}

unsafe fn ff_clear(search_ctx: &mut ff_search_ctx_T) {
    unsafe {
        search_ctx.ffsc_stack.clear();
        search_ctx.ffsc_stopdirs_v.clear();

        api_clear_string(&mut search_ctx.ffsc_file_to_search);
        api_clear_string(&mut search_ctx.ffsc_start_dir);
        api_clear_string(&mut search_ctx.ffsc_fix_path);
        api_clear_string(&mut search_ctx.ffsc_wc_path);
        search_ctx.ffsc_level = 0;
    }
}

unsafe fn ff_path_in_stoplist(
    path: *const c_char,
    mut path_len: usize,
    stopdirs_v: &[std::ffi::CString],
) -> bool {
    unsafe {
        while path_len > 1 && vim_ispathsep(c_int::from(*path.add(path_len - 1) as u8)) {
            path_len -= 1;
        }

        if path_len == 0 {
            return true;
        }

        for stopdir in stopdirs_v {
            let bytes = stopdir.to_bytes();
            if path_cmp(opt_vars().p_fic != 0, stopdir.as_ptr(), path, path_len) == 0
                && (bytes.len() <= path_len || vim_ispathsep(c_int::from(bytes[path_len])))
            {
                return true;
            }
        }

        false
    }
}

/// Find the file name `ptr[len]` in the path. Also finds directory names.
pub unsafe fn find_file_in_path(
    ptr: *mut c_char,
    len: usize,
    options: c_int,
    first: c_int,
    rel_fname: *mut c_char,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<ff_search_ctx_T>,
) -> Option<std::ffi::CString> {
    unsafe {
        find_file_in_path_option(
            ptr,
            len,
            options,
            first,
            if *(*g_st().curbuf).b_p_path == NUL {
                opt_vars().p_path
            } else {
                (*g_st().curbuf).b_p_path
            },
            FINDFILE_BOTH,
            rel_fname,
            (*g_st().curbuf).b_p_sua,
            file_to_find,
            search_ctx,
        )
    }
}

pub unsafe fn free_findfile() {}

/// Find the directory name `ptr[len]` in the path.
pub unsafe fn find_directory_in_path(
    ptr: *mut c_char,
    len: usize,
    options: c_int,
    rel_fname: *mut c_char,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<ff_search_ctx_T>,
) -> Option<std::ffi::CString> {
    unsafe {
        find_file_in_path_option(
            ptr,
            len,
            options,
            c_int::from(true),
            opt_vars().p_cdpath,
            FINDFILE_DIR,
            rel_fname,
            EMPTY_C.as_ptr().cast_mut(),
            file_to_find,
            search_ctx,
        )
    }
}

pub unsafe fn find_file_in_path_option(
    ptr: *mut c_char,
    len: usize,
    options: c_int,
    first: c_int,
    path_option: *mut c_char,
    find_what: c_int,
    mut rel_fname: *mut c_char,
    suffixes: *mut c_char,
    file_to_find: &mut Option<std::ffi::CString>,
    search_ctx: &mut Option<ff_search_ctx_T>,
) -> Option<std::ffi::CString> {
    unsafe {
        struct FindFilePathOptState {
            dir: *mut c_char,
            did_findfile_init: bool,
            file_to_findlen: usize,
        }
        static FF_OPT_STATE: GlobalState<FindFilePathOptState> = GlobalState::new(FindFilePathOptState {
            dir: ptr::null_mut(),
            did_findfile_init: false,
            file_to_findlen: 0,
        });
        let ff_opt = &mut *FF_OPT_STATE.as_ptr();
        let mut file_name: Option<std::ffi::CString> = None;
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();

        if !rel_fname.is_null() && path_with_url(rel_fname) != 0 {
            rel_fname = ptr::null_mut();
        }

        if first == c_int::from(true) {
            if len == 0 {
                return None;
            }

            let save_char = *ptr.add(len);
            *ptr.add(len) = NUL;
            ff_opt.file_to_findlen = expand_env_esc(
                ptr,
                name_buff,
                MAXPATHL as c_int,
                ptr::null_mut(),
                true,
                ptr::null_mut(),
            );
            *ptr.add(len) = save_char;

            let mut ftf_bytes =
                std::slice::from_raw_parts(name_buff.cast::<u8>(), ff_opt.file_to_findlen).to_vec();
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
        let rel_to_curdir = *ftf == b'.' as c_char
            && (*ftf.add(1) == NUL
                || vim_ispathsep(c_int::from(*ftf.add(1) as u8))
                || (*ftf.add(1) == b'.' as c_char
                    && (*ftf.add(2) == NUL || vim_ispathsep(c_int::from(*ftf.add(2) as u8)))));
        let is_mswin_abs = cfg!(windows)
            && (vim_ispathsep(c_int::from(*ftf as u8))
                || (*ftf != NUL && *ftf.add(1) == b':' as c_char));

        if vim_isAbsName(ftf) || rel_to_curdir || is_mswin_abs {
            if first == c_int::from(true) {
                if path_with_url(ftf) != 0 {
                    return Some(
                        std::ffi::CString::new(std::slice::from_raw_parts(
                            ftf.cast::<u8>(),
                            ff_opt.file_to_findlen,
                        ))
                        .unwrap_or_default(),
                    );
                }

                let rel_fnamelen = if !rel_fname.is_null() {
                    CStr::from_ptr(rel_fname).to_bytes().len()
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
                            c"%.*s%s".as_ptr(),
                            path_tail(rel_fname).offset_from(rel_fname) as c_int,
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
                            let slen = simplify_filename(name_buff);
                            return Some(
                                std::ffi::CString::new(std::slice::from_raw_parts(
                                    name_buff.cast::<u8>(),
                                    slen,
                                ))
                                .unwrap_or_default(),
                            );
                        }
                        if *suffix == NUL {
                            break;
                        }
                        copy_option_part(
                            &raw mut suffix,
                            name_buff.add(l),
                            MAXPATHL - l,
                            c",".as_ptr(),
                        );
                    }
                    run += 1;
                }
            }
        } else {
            if first == c_int::from(true) {
                vim_findfile_free_visited(search_ctx.as_mut());
                ff_opt.dir = path_option;
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
                    if ff_opt.dir.is_null() || *ff_opt.dir == NUL {
                        *search_ctx = None;
                        break;
                    }

                    let mut buf = [0 as c_char; MAXPATHL];
                    copy_option_part(&raw mut ff_opt.dir, buf.as_mut_ptr(), MAXPATHL, c" ,".as_ptr());

                    let r_ptr = vim_findfile_stopdir(buf.as_mut_ptr());
                    *search_ctx = vim_findfile_init(
                        buf.as_mut_ptr(),
                        ftf,
                        ff_opt.file_to_findlen,
                        r_ptr,
                        100,
                        c_int::from(false),
                        find_what,
                        search_ctx.take(),
                        c_int::from(false),
                        rel_fname,
                    );
                    if search_ctx.is_some() {
                        ff_opt.did_findfile_init = true;
                    }
                }
            }
        }

        if file_name.is_none() && (options & FNAME_MESS) != 0 {
            if first == c_int::from(true) {
                if find_what == FINDFILE_DIR {
                    semsg!(tr(e_cant_find_directory_str_in_cdpath.as_ptr()), ftf);
                } else {
                    semsg!(tr(e_cant_find_file_str_in_path.as_ptr()), ftf);
                }
            } else if find_what == FINDFILE_DIR {
                semsg!(tr(e_no_more_directory_str_found_in_cdpath.as_ptr()), ftf);
            } else {
                semsg!(tr(e_no_more_file_str_found_in_path.as_ptr()), ftf);
            }
        }

        file_name
    }
}

/// Get the file name at the cursor.
pub unsafe fn grab_file_name(count: c_int, file_lnum: *mut linenr_T) -> Option<std::ffi::CString> {
    unsafe {
        let options = FNAME_MESS | FNAME_EXP | FNAME_REL | FNAME_UNESC;
        let fname = if g_st().Visual.active {
            let mut len: usize = 0;
            let mut ptr: *mut c_char = ptr::null_mut();
            if !get_visual_text(ptr::null_mut(), &raw mut ptr, &raw mut len) {
                return None;
            }
            if !file_lnum.is_null()
                && *ptr.add(len) == b':' as c_char
                && (*ptr.add(len + 1) as u8).is_ascii_digit()
            {
                let mut p: *mut c_char = ptr.add(len + 1);
                *file_lnum = getdigits_int32(&raw mut p, false, 0);
            }
            find_file_name_in_path(ptr, len, options, count as c_long, (*g_st().curbuf).b_ffname)
        } else {
            file_name_at_cursor(options | FNAME_HYP, count, file_lnum)
        }?;
        #[cfg(windows)]
        let fname = {
            let mut bytes: Vec<c_char> = fname
                .into_bytes_with_nul()
                .into_iter()
                .map(|b| b as c_char)
                .collect();
            path_to_slash(bytes.as_mut_ptr());
            CStr::from_ptr(bytes.as_ptr()).to_owned()
        };
        Some(fname)
    }
}

/// Return the file name under or after the cursor.
pub unsafe fn file_name_at_cursor(
    options: c_int,
    count: c_int,
    file_lnum: *mut linenr_T,
) -> Option<std::ffi::CString> {
    unsafe {
        file_name_in_line(
            get_cursor_line_ptr(),
            (**curwin.as_ptr()).w_cursor.col,
            options,
            count,
            (*g_st().curbuf).b_ffname,
            file_lnum,
        )
    }
}

/// Return the name of the file under or after `line[col]`.
pub unsafe fn file_name_in_line(
    line: *mut c_char,
    col: c_int,
    options: c_int,
    count: c_int,
    rel_fname: *mut c_char,
    file_lnum: *mut linenr_T,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut ptr = line.add(col as usize);
        while *ptr != NUL && !vim_isfilec(c_int::from(*ptr as u8)) {
            ptr = ptr.add(utfc_ptr2len(ptr) as usize);
        }
        if *ptr == NUL {
            if (options & FNAME_MESS) != 0 {
                emsg(tr(c"E446: No file name under cursor".as_ptr()));
            }
            return None;
        }

        let mut in_type = true;
        let mut is_url = false;

        while ptr > line {
            let head_off = utf_head_off(line, ptr.sub(1)) as usize;
            if head_off > 0 {
                ptr = ptr.sub(head_off + 1);
            } else if vim_isfilec(c_int::from(*ptr.sub(1) as u8))
                || ((options & FNAME_HYP) != 0 && path_is_url(ptr.sub(1)) != 0)
            {
                ptr = ptr.sub(1);
            } else {
                break;
            }
        }

        let mut len = if path_has_drive_letter(ptr, CStr::from_ptr(ptr).to_bytes().len()) {
            2
        } else {
            0
        };
        while vim_isfilec(c_int::from(*ptr.add(len) as u8))
            || (*ptr.add(len) == b'\\' as c_char && *ptr.add(len + 1) == b' ' as c_char)
            || ((options & FNAME_HYP) != 0 && path_is_url(ptr.add(len)) != 0)
            || (is_url && !vim_strchr(c":?&=".as_ptr(), c_int::from(*ptr.add(len) as u8)).is_null())
        {
            let b = *ptr.add(len) as u8;
            if b.is_ascii_alphabetic() {
                if in_type && path_is_url(ptr.add(len + 1)) != 0 {
                    is_url = true;
                }
            } else {
                in_type = false;
            }

            if *ptr.add(len) == b'\\' as c_char && *ptr.add(len + 1) == b' ' as c_char {
                len += 1;
            }
            len += utfc_ptr2len(ptr.add(len)) as usize;
        }

        if len > 2
            && !vim_strchr(c".,:;!".as_ptr(), c_int::from(*ptr.add(len - 1) as u8)).is_null()
            && *ptr.add(len - 2) != b'.' as c_char
        {
            len -= 1;
        }

        if !file_lnum.is_null() {
            let english = b" line ";
            let mut p = ptr.add(len);
            let p_bytes = CStr::from_ptr(p).to_bytes();
            if p_bytes.starts_with(english) {
                p = p.add(english.len());
            } else {
                let loc_msg = CStr::from_ptr(tr(line_msg.as_ptr())).to_bytes();
                if p_bytes.starts_with(loc_msg) {
                    p = p.add(loc_msg.len());
                } else {
                    p = skipwhite(p);
                }
            }
            if *p != NUL {
                if !(*p as u8).is_ascii_digit() {
                    p = p.add(1);
                }
                p = skipwhite(p);
                if (*p as u8).is_ascii_digit() {
                    *file_lnum = getdigits_long(&raw mut p, false, 0) as linenr_T;
                }
            }
        }

        find_file_name_in_path(ptr, len, options, count as c_long, rel_fname)
    }
}

unsafe fn eval_includeexpr(ptr: *const c_char, len: usize) -> Option<std::ffi::CString> {
    unsafe {
        let save_sctx = g_st().current_sctx;
        set_vim_var_string(VV_FNAME, ptr, len as isize);
        g_st().current_sctx = (*g_st().curbuf).b_p_script_ctx[kBufOptIncludeexpr];

        let use_sandbox = was_set_insecurely(*curwin.as_ptr(), kOptIncludeexpr, OPT_LOCAL) != 0;
        let mut tv: typval_T = typval_T::new();
        let mut res = None;
        if eval_expr_option_tv(&raw mut (*g_st().curbuf).b_p_inex, use_sandbox, &raw mut tv) {
            let s = tv_get_string(&raw const tv);
            res = Some(CStr::from_ptr(s).to_owned());
            tv_clear(&raw mut tv);
        }

        set_vim_var_string(VV_FNAME, ptr::null(), 0);
        g_st().current_sctx = save_sctx;
        res
    }
}

/// Return the name of the file `ptr[len]` in `'path'`.
pub unsafe fn find_file_name_in_path(
    mut ptr: *mut c_char,
    mut len: usize,
    options: c_int,
    mut count: c_long,
    rel_fname: *mut c_char,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut tofree: Option<std::ffi::CString> = None;

        if len == 0 {
            return None;
        }

        if (options & FNAME_HYP) != 0
            && len > 6
            && std::slice::from_raw_parts(ptr.cast::<u8>(), 6) == b"file:/"
            && !vim_ispathsep(c_int::from(*ptr.add(6) as u8))
        {
            let off = if path_has_drive_letter(ptr.add(6), len - 6) {
                6
            } else {
                5
            };
            ptr = ptr.add(off);
            len -= off;
        }

        if (options & FNAME_INCL) != 0 && (*g_st().curbuf).b_p_inex.cb_type() != kCallbackNone {
            tofree = eval_includeexpr(ptr, len);
            if let Some(ref tf) = tofree {
                ptr = tf.as_ptr().cast_mut();
                len = tf.as_bytes().len();
            }
        }

        let mut file_name: Option<std::ffi::CString>;
        if (options & FNAME_EXP) != 0 {
            let mut file_to_find: Option<std::ffi::CString> = None;
            let mut search_ctx: Option<ff_search_ctx_T> = None;

            file_name = find_file_in_path(
                ptr,
                len,
                options & !FNAME_MESS,
                c_int::from(true),
                rel_fname,
                &mut file_to_find,
                &mut search_ctx,
            );

            if file_name.is_none()
                && (options & FNAME_INCL) == 0
                && (*g_st().curbuf).b_p_inex.cb_type() != kCallbackNone
            {
                tofree = eval_includeexpr(ptr, len);
                if let Some(ref tf) = tofree {
                    ptr = tf.as_ptr().cast_mut();
                    len = tf.as_bytes().len();
                    file_name = find_file_in_path(
                        ptr,
                        len,
                        options & !FNAME_MESS,
                        c_int::from(true),
                        rel_fname,
                        &mut file_to_find,
                        &mut search_ctx,
                    );
                }
            }
            if file_name.is_none() && (options & FNAME_MESS) != 0 {
                let c = *ptr.add(len);
                *ptr.add(len) = NUL;
                semsg!(tr(c"E447: Can't find file \"%s\" in path".as_ptr()), ptr);
                *ptr.add(len) = c;
            }

            while file_name.is_some() && {
                count -= 1;
                count > 0
            } {
                file_name = find_file_in_path(
                    ptr,
                    len,
                    options,
                    c_int::from(false),
                    rel_fname,
                    &mut file_to_find,
                    &mut search_ctx,
                );
            }
        } else {
            file_name = Some(
                std::ffi::CString::new(std::slice::from_raw_parts(ptr.cast::<u8>(), len))
                    .unwrap_or_default(),
            );
        }

        drop(tofree);
        file_name
    }
}

pub unsafe fn do_autocmd_dirchanged(
    new_dir: *mut c_char,
    scope: CdScope,
    cause: CdCause,
    pre: bool,
) {
    unsafe {
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

        let mut save_v_event: save_v_event_T = save_v_event_T::new();
        let dict = get_v_event(&raw mut save_v_event);
        let mut buf: [c_char; 8] = [0; 8];

        let scope_cstr = match scope {
            kCdScopeGlobal => c"global",
            kCdScopeTabpage => c"tabpage",
            kCdScopeBuffer => c"buffer",
            kCdScopeWindow => c"window",
            _ => std::process::abort(),
        };
        xstrlcpy(buf.as_mut_ptr(), scope_cstr.as_ptr(), buf.len());

        if pre {
            tv_dict_add_str(dict, c"directory".as_ptr(), "directory".len(), new_dir);
        } else {
            tv_dict_add_str(dict, c"cwd".as_ptr(), "cwd".len(), new_dir);
        }
        tv_dict_add_str(dict, c"scope".as_ptr(), "scope".len(), buf.as_ptr());
        tv_dict_add_bool(
            dict,
            c"changed_window".as_ptr(),
            "changed_window".len(),
            c_int::from(cause == kCdCauseWindow),
        );
        tv_dict_set_keys_readonly(dict);

        match cause {
            kCdCauseManual | kCdCauseWindow | kCdCauseBuffer => {}
            kCdCauseAuto => {
                xstrlcpy(buf.as_mut_ptr(), c"auto".as_ptr(), buf.len());
            }
            _ => std::process::abort(),
        }

        apply_autocmds(event, buf.as_mut_ptr(), new_dir, false, g_st().curbuf);

        restore_v_event(dict, &raw mut save_v_event);

        RECURSIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Change to a file's directory.
pub unsafe fn vim_chdirfile(fname: *mut c_char, cause: CdCause) -> c_int {
    unsafe {
        let mut dir: [c_char; MAXPATHL] = [0; MAXPATHL];
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();

        xstrlcpy(dir.as_mut_ptr(), fname, MAXPATHL);
        *path_tail_with_sep(dir.as_mut_ptr()) = NUL;

        if os_dirname(name_buff, MAXPATHL) != OK {
            *name_buff = NUL;
        }

        if path_equal(dir.as_ptr(), name_buff, kPathCmpLiteral) {
            return OK;
        }

        if cause != kCdCauseOther {
            do_autocmd_dirchanged(dir.as_mut_ptr(), kCdScopeWindow, cause, true);
        }

        if os_chdir(dir.as_ptr()) != 0 {
            return FAIL;
        }

        if cause != kCdCauseOther {
            do_autocmd_dirchanged(dir.as_mut_ptr(), kCdScopeWindow, cause, false);
        }

        OK
    }
}

/// Change directory to `new_dir`. Search `'cdpath'` for relative directory names.
pub unsafe fn vim_chdir(new_dir: *mut c_char) -> c_int {
    unsafe {
        let mut file_to_find: Option<std::ffi::CString> = None;
        let mut search_ctx: Option<ff_search_ctx_T> = None;
        let Some(dir_name) = find_directory_in_path(
            new_dir,
            CStr::from_ptr(new_dir).to_bytes().len(),
            FNAME_MESS,
            (*g_st().curbuf).b_ffname,
            &mut file_to_find,
            &mut search_ctx,
        ) else {
            return -1;
        };

        os_chdir(dir_name.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_findfile_stopdir_splits_and_unescapes_semicolons() {
        let mut buf = *b"/a/b;/c/d\0";
        let stop = unsafe { vim_findfile_stopdir(buf.as_mut_ptr().cast()) };
        assert!(!stop.is_null());
        let first = unsafe { CStr::from_ptr(buf.as_ptr().cast()).to_bytes() };
        let rest = unsafe { CStr::from_ptr(stop).to_bytes() };
        assert_eq!(first, b"/a/b");
        assert_eq!(rest, b"/c/d");

        let mut escaped = *b"/a\\;b;/c\0";
        let stop2 = unsafe { vim_findfile_stopdir(escaped.as_mut_ptr().cast()) };
        assert!(!stop2.is_null());
        let first2 = unsafe { CStr::from_ptr(escaped.as_ptr().cast()).to_bytes() };
        let rest2 = unsafe { CStr::from_ptr(stop2).to_bytes() };
        assert_eq!(first2, b"/a;b");
        assert_eq!(rest2, b"/c");

        let mut none = *b"/a/b\0";
        let stop3 = unsafe { vim_findfile_stopdir(none.as_mut_ptr().cast()) };
        assert!(stop3.is_null());
    }
}
