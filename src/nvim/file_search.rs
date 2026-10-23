//! File searching functions for `'path'`, `'tags'`, and `'cdpath'` options (`src/nvim/file_search.c`).

use crate::api::private::helpers::{
    api_clear_string, cbuf_to_string, copy_string, cstr_as_string, cstr_to_string,
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
use crate::memory::{xcalloc, xfree, xmalloc, xmemcpyz, xmemdupz, xstrdup, xstrlcpy};
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
    EW_ADDSLASH, EW_DIR, EW_NOTWILD, EW_SILENT, FreeWild, FullName_save, PATHSEP, after_pathsep,
    expand_wildcards, kPathCmpLiteral, path_cmp, path_equal, path_has_drive_letter, path_is_url,
    path_shorten_fname, path_tail, path_tail_with_sep, path_with_url, simplify_filename,
    vim_isAbsName, vim_ispathsep,
};
#[cfg(windows)]
use crate::path::{path_to_slash, vim_FullName};
use crate::state::save_v_event_T;
use crate::strings::{tr, vim_snprintf, vim_strchr, xstrnsave};
use std::ffi::{CStr, c_char, c_int, c_long, c_void};
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
    pub ffs_prev: *mut ff_stack_T,
    pub ffs_fix_path: NvimString,
    pub ffs_wc_path: NvimString,
    pub ffs_filearray: *mut *mut c_char,
    pub ffs_filearray_size: c_int,
    pub ffs_filearray_cur: c_int,
    pub ffs_stage: c_int,
    pub ffs_level: c_int,
    pub ffs_star_star_empty: c_int,
}

#[repr(C)]
pub struct ff_visited_T {
    pub ffv_next: *mut ff_visited_T,
    pub ffv_wc_path: *mut c_char,
    pub file_id_valid: bool,
    pub file_id: FileID,
    pub ffv_fname: Vec<c_char>,
}

#[repr(C)]
pub struct ff_visited_list_hdr_T {
    pub ffvl_next: *mut ff_visited_list_hdr_T,
    pub ffvl_filename: *mut c_char,
    pub ffvl_visited_list: *mut ff_visited_T,
}

#[repr(C)]
pub struct ff_search_ctx_T {
    pub ffsc_stack_ptr: *mut ff_stack_T,
    pub ffsc_visited_list: *mut ff_visited_list_hdr_T,
    pub ffsc_dir_visited_list: *mut ff_visited_list_hdr_T,
    pub ffsc_visited_lists_list: *mut ff_visited_list_hdr_T,
    pub ffsc_dir_visited_lists_list: *mut ff_visited_list_hdr_T,
    pub ffsc_file_to_search: NvimString,
    pub ffsc_start_dir: NvimString,
    pub ffsc_fix_path: NvimString,
    pub ffsc_wc_path: NvimString,
    pub ffsc_level: c_int,
    pub ffsc_stopdirs_v: Vec<NvimString>,
    pub ffsc_find_what: c_int,
    pub ffsc_tagfile: c_int,
}

static mut ff_expand_buffer: NvimString = NULL_STRING;

/// Initialization routine for `vim_findfile()`.
pub unsafe fn vim_findfile_init(
    mut path: *mut c_char,
    filename: *mut c_char,
    filenamelen: usize,
    stopdirs: *mut c_char,
    level: c_int,
    free_visited: c_int,
    find_what: c_int,
    search_ctx_arg: *mut c_void,
    tagfile: c_int,
    rel_fname: *mut c_char,
) -> *mut c_void {
    unsafe {
        let search_ctx: *mut ff_search_ctx_T = if !search_ctx_arg.is_null() {
            search_ctx_arg.cast()
        } else {
            let ctx = xcalloc(1, size_of::<ff_search_ctx_T>()).cast::<ff_search_ctx_T>();
            ptr::write(&raw mut (*ctx).ffsc_stopdirs_v, Vec::new());
            ctx
        };
        (*search_ctx).ffsc_find_what = find_what;
        (*search_ctx).ffsc_tagfile = tagfile;

        ff_clear(search_ctx);

        if free_visited == c_int::from(true) {
            vim_findfile_free_visited(search_ctx.cast());
        } else {
            (*search_ctx).ffsc_visited_list = ff_get_visited_list(
                filename,
                filenamelen,
                &raw mut (*search_ctx).ffsc_visited_lists_list,
            );
            if (*search_ctx).ffsc_visited_list.is_null() {
                vim_findfile_cleanup(search_ctx.cast());
                return ptr::null_mut();
            }
            (*search_ctx).ffsc_dir_visited_list = ff_get_visited_list(
                filename,
                filenamelen,
                &raw mut (*search_ctx).ffsc_dir_visited_lists_list,
            );
            if (*search_ctx).ffsc_dir_visited_list.is_null() {
                vim_findfile_cleanup(search_ctx.cast());
                return ptr::null_mut();
            }
        }

        if ff_expand_buffer.data.is_null() {
            ff_expand_buffer.size = 0;
            ff_expand_buffer.data = xmalloc(MAXPATHL).cast();
        }

        if *path == b'.' as c_char
            && (vim_ispathsep(c_int::from(*path.add(1) as u8)) || *path.add(1) == NUL)
            && (tagfile == 0 || vim_strchr(p_cpo, kCpoDottag).is_null())
            && !rel_fname.is_null()
        {
            let len = path_tail(rel_fname).offset_from(rel_fname) as usize;

            if !vim_isAbsName(rel_fname) && len + 1 < MAXPATHL {
                xmemcpyz(ff_expand_buffer.data.cast(), rel_fname.cast(), len);
                ff_expand_buffer.size = len;
                (*search_ctx).ffsc_start_dir =
                    cstr_as_string(FullName_save(ff_expand_buffer.data, false));
            } else {
                (*search_ctx).ffsc_start_dir = cbuf_to_string(rel_fname, len);
            }
            path = path.add(1);
            if *path != NUL {
                path = path.add(1);
            }
        } else if *path == NUL || !vim_isAbsName(path) {
            #[cfg(windows)]
            let handled_drive = if *path != NUL && *path.add(1) == b':' as c_char {
                let drive: [c_char; 3] = [*path, b':' as c_char, NUL];
                if vim_FullName(drive.as_ptr(), ff_expand_buffer.data, MAXPATHL, true) == FAIL {
                    vim_findfile_cleanup(search_ctx.cast());
                    return ptr::null_mut();
                }
                path = path.add(2);
                true
            } else {
                false
            };
            #[cfg(not(windows))]
            let handled_drive = false;

            if !handled_drive && os_dirname(ff_expand_buffer.data, MAXPATHL) == FAIL {
                vim_findfile_cleanup(search_ctx.cast());
                return ptr::null_mut();
            }
            ff_expand_buffer.size = CStr::from_ptr(ff_expand_buffer.data).to_bytes().len();

            (*search_ctx).ffsc_start_dir = copy_string(ff_expand_buffer, ptr::null_mut());
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
                    xmemcpyz(ff_expand_buffer.data.cast(), helper.cast(), len);
                    ff_expand_buffer.size = len;
                    cstr_as_string(FullName_save(helper, len != 0))
                } else {
                    cbuf_to_string(helper, len)
                };
                (*search_ctx).ffsc_stopdirs_v.push(dir_str);
                if !walker.is_null() {
                    walker = walker.add(1);
                }
                if walker.is_null() {
                    break;
                }
            }
        }

        (*search_ctx).ffsc_level = level;

        let mut wc_part = vim_strchr(path, b'*' as c_int);
        if !wc_part.is_null() {
            (*search_ctx).ffsc_fix_path = cbuf_to_string(path, wc_part.offset_from(path) as usize);

            ff_expand_buffer.size = 0;
            while *wc_part != NUL {
                if ff_expand_buffer.size + 5 >= MAXPATHL {
                    emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION.as_ptr()));
                    break;
                }
                if *wc_part == b'*' as c_char && *wc_part.add(1) == b'*' as c_char {
                    *ff_expand_buffer.data.add(ff_expand_buffer.size) = *wc_part;
                    ff_expand_buffer.size += 1;
                    wc_part = wc_part.add(1);
                    *ff_expand_buffer.data.add(ff_expand_buffer.size) = *wc_part;
                    ff_expand_buffer.size += 1;
                    wc_part = wc_part.add(1);

                    let mut errpt: *mut c_char = wc_part;
                    let llevel = getdigits(&raw mut errpt, false, 255);
                    if errpt != wc_part && llevel > 0 && llevel < 255 {
                        *ff_expand_buffer.data.add(ff_expand_buffer.size) = llevel as c_char;
                        ff_expand_buffer.size += 1;
                    } else if errpt != wc_part && llevel == 0 {
                        ff_expand_buffer.size -= 2;
                    } else {
                        *ff_expand_buffer.data.add(ff_expand_buffer.size) = FF_MAX_STAR_STAR_EXPAND;
                        ff_expand_buffer.size += 1;
                    }
                    wc_part = errpt;
                    if *wc_part != NUL && !vim_ispathsep(c_int::from(*wc_part as u8)) {
                        semsg!(
                            tr(c"E343: Invalid path: '**[number]' must be at the end of the path or be followed by '%s'."
                                .as_ptr()),
                            PATHSEPSTR_C.as_ptr(),
                        );
                        vim_findfile_cleanup(search_ctx.cast());
                        return ptr::null_mut();
                    }
                } else {
                    *ff_expand_buffer.data.add(ff_expand_buffer.size) = *wc_part;
                    ff_expand_buffer.size += 1;
                    wc_part = wc_part.add(1);
                }
            }
            *ff_expand_buffer.data.add(ff_expand_buffer.size) = NUL;
            (*search_ctx).ffsc_wc_path = copy_string(ff_expand_buffer, ptr::null_mut());
        } else {
            (*search_ctx).ffsc_fix_path = cstr_to_string(path);
        }

        if (*search_ctx).ffsc_start_dir.data.is_null() {
            (*search_ctx).ffsc_start_dir =
                copy_string((*search_ctx).ffsc_fix_path, ptr::null_mut());
            *(*search_ctx).ffsc_fix_path.data = NUL;
            (*search_ctx).ffsc_fix_path.size = 0;
        }

        if (*search_ctx).ffsc_start_dir.size + (*search_ctx).ffsc_fix_path.size + 3 >= MAXPATHL {
            emsg(tr(E_PATH_TOO_LONG_FOR_COMPLETION.as_ptr()));
            vim_findfile_cleanup(search_ctx.cast());
            return ptr::null_mut();
        }

        let mut add_sep = after_pathsep(
            (*search_ctx).ffsc_start_dir.data,
            (*search_ctx)
                .ffsc_start_dir
                .data
                .add((*search_ctx).ffsc_start_dir.size),
        ) == 0;
        ff_expand_buffer.size = vim_snprintf!(
            ff_expand_buffer.data,
            MAXPATHL,
            c"%s%s".as_ptr(),
            (*search_ctx).ffsc_start_dir.data,
            if add_sep {
                PATHSEPSTR_C.as_ptr()
            } else {
                EMPTY_C.as_ptr()
            },
        ) as usize;

        {
            let bufsize = ff_expand_buffer.size + (*search_ctx).ffsc_fix_path.size + 1;
            let buf: *mut c_char = xmalloc(bufsize).cast();

            vim_snprintf!(
                buf,
                bufsize,
                c"%s%s".as_ptr(),
                ff_expand_buffer.data,
                (*search_ctx).ffsc_fix_path.data,
            );
            if os_isdir(buf) {
                if (*search_ctx).ffsc_fix_path.size > 0 {
                    add_sep = after_pathsep(
                        (*search_ctx).ffsc_fix_path.data,
                        (*search_ctx)
                            .ffsc_fix_path
                            .data
                            .add((*search_ctx).ffsc_fix_path.size),
                    ) == 0;
                    ff_expand_buffer.size += vim_snprintf!(
                        ff_expand_buffer.data.add(ff_expand_buffer.size),
                        MAXPATHL - ff_expand_buffer.size,
                        c"%s%s".as_ptr(),
                        (*search_ctx).ffsc_fix_path.data,
                        if add_sep {
                            PATHSEPSTR_C.as_ptr()
                        } else {
                            EMPTY_C.as_ptr()
                        },
                    ) as usize;
                }
            } else {
                let p = path_tail((*search_ctx).ffsc_fix_path.data);
                let mut len = (*search_ctx).ffsc_fix_path.size as c_int;

                if p > (*search_ctx).ffsc_fix_path.data {
                    len = p.offset_from((*search_ctx).ffsc_fix_path.data) as c_int - 1;
                    if len >= 2
                        && *(*search_ctx).ffsc_fix_path.data == b'.' as c_char
                        && *(*search_ctx).ffsc_fix_path.data.add(1) == b'.' as c_char
                        && (len == 2 || *(*search_ctx).ffsc_fix_path.data.add(2) == PATHSEP)
                    {
                        xfree(buf.cast());
                        vim_findfile_cleanup(search_ctx.cast());
                        return ptr::null_mut();
                    }

                    add_sep = after_pathsep(
                        (*search_ctx).ffsc_fix_path.data,
                        (*search_ctx)
                            .ffsc_fix_path
                            .data
                            .add((*search_ctx).ffsc_fix_path.size),
                    ) == 0;
                    ff_expand_buffer.size += vim_snprintf!(
                        ff_expand_buffer.data.add(ff_expand_buffer.size),
                        MAXPATHL - ff_expand_buffer.size,
                        c"%.*s%s".as_ptr(),
                        len,
                        (*search_ctx).ffsc_fix_path.data,
                        if add_sep {
                            PATHSEPSTR_C.as_ptr()
                        } else {
                            EMPTY_C.as_ptr()
                        },
                    ) as usize;
                }

                if !(*search_ctx).ffsc_wc_path.data.is_null() {
                    let tempsize = ((*search_ctx).ffsc_fix_path.size - len as usize)
                        + (*search_ctx).ffsc_wc_path.size
                        + 1;
                    let temp: *mut c_char = xmalloc(tempsize).cast();
                    (*search_ctx).ffsc_wc_path.size = vim_snprintf!(
                        temp,
                        tempsize,
                        c"%s%s".as_ptr(),
                        (*search_ctx).ffsc_fix_path.data.add(len as usize),
                        (*search_ctx).ffsc_wc_path.data,
                    ) as usize;
                    xfree((*search_ctx).ffsc_wc_path.data.cast());
                    (*search_ctx).ffsc_wc_path.data = temp;
                }
            }
            xfree(buf.cast());
        }

        let sptr = ff_create_stack_element(
            ff_expand_buffer.data,
            ff_expand_buffer.size,
            (*search_ctx).ffsc_wc_path.data,
            (*search_ctx).ffsc_wc_path.size,
            level,
            0,
        );

        ff_push(search_ctx, sptr);
        (*search_ctx).ffsc_file_to_search = cbuf_to_string(filename, filenamelen);
        search_ctx.cast()
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

/// Clean up the given search context. Can handle a NULL pointer.
pub unsafe fn vim_findfile_cleanup(ctx: *mut c_void) {
    unsafe {
        if ctx.is_null() {
            return;
        }

        vim_findfile_free_visited(ctx);
        ff_clear(ctx.cast());
        ptr::drop_in_place(&raw mut (*(ctx.cast::<ff_search_ctx_T>())).ffsc_stopdirs_v);
        xfree(ctx);
    }
}

/// Find a file in a search context.
pub unsafe fn vim_findfile(search_ctx_arg: *mut c_void) -> *mut c_char {
    unsafe {
        if search_ctx_arg.is_null() {
            return ptr::null_mut();
        }

        let search_ctx = search_ctx_arg.cast::<ff_search_ctx_T>();
        let mut file_path = NvimString {
            data: xmalloc(MAXPATHL).cast(),
            size: 0,
        };
        let mut path_end = if !(*search_ctx).ffsc_start_dir.data.is_null() {
            (*search_ctx)
                .ffsc_start_dir
                .data
                .add((*search_ctx).ffsc_start_dir.size)
        } else {
            ptr::null_mut()
        };

        'upward: loop {
            loop {
                os_breakcheck();
                if got_int {
                    break;
                }

                let stackp = ff_pop(search_ctx);
                if stackp.is_null() {
                    break;
                }

                if (*stackp).ffs_filearray.is_null()
                    && ff_check_visited(
                        &raw mut (*(*search_ctx).ffsc_dir_visited_list).ffvl_visited_list,
                        (*stackp).ffs_fix_path.data,
                        (*stackp).ffs_fix_path.size,
                        (*stackp).ffs_wc_path.data,
                        (*stackp).ffs_wc_path.size,
                    ) == FAIL
                {
                    ff_free_stack_element(stackp);
                    continue;
                }

                if (*stackp).ffs_level <= 0 {
                    ff_free_stack_element(stackp);
                    continue;
                }

                *file_path.data = NUL;
                file_path.size = 0;

                let mut rest_of_wildcards: NvimString;
                if (*stackp).ffs_filearray.is_null() {
                    let mut dirptrs: [*mut c_char; 2] = [file_path.data, ptr::null_mut()];

                    if !vim_isAbsName((*stackp).ffs_fix_path.data)
                        && !(*search_ctx).ffsc_start_dir.data.is_null()
                    {
                        if (*search_ctx).ffsc_start_dir.size + 1 >= MAXPATHL {
                            ff_free_stack_element(stackp);
                            break 'upward;
                        }
                        let add_sep = after_pathsep(
                            (*search_ctx).ffsc_start_dir.data,
                            (*search_ctx)
                                .ffsc_start_dir
                                .data
                                .add((*search_ctx).ffsc_start_dir.size),
                        ) == 0;
                        file_path.size = vim_snprintf!(
                            file_path.data,
                            MAXPATHL,
                            c"%s%s".as_ptr(),
                            (*search_ctx).ffsc_start_dir.data,
                            if add_sep {
                                PATHSEPSTR_C.as_ptr()
                            } else {
                                EMPTY_C.as_ptr()
                            },
                        ) as usize;
                        if file_path.size >= MAXPATHL {
                            ff_free_stack_element(stackp);
                            break 'upward;
                        }
                    }

                    if file_path.size + (*stackp).ffs_fix_path.size + 1 >= MAXPATHL {
                        ff_free_stack_element(stackp);
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
                        ff_free_stack_element(stackp);
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
                                    ff_free_stack_element(stackp);
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
                                ff_free_stack_element(stackp);
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
                        (*stackp).ffs_filearray = xmalloc(size_of::<*mut c_char>()).cast();
                        *(*stackp).ffs_filearray =
                            xmemdupz(dirptrs[0].cast(), file_path.size).cast();
                        (*stackp).ffs_filearray_size = 1;
                    } else {
                        expand_wildcards(
                            if dirptrs[1].is_null() { 1 } else { 2 },
                            dirptrs.as_mut_ptr(),
                            &raw mut (*stackp).ffs_filearray_size,
                            &raw mut (*stackp).ffs_filearray,
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
                        for i in (*stackp).ffs_filearray_cur..(*stackp).ffs_filearray_size {
                            let entry = *(*stackp).ffs_filearray.add(i as usize);
                            if path_with_url(entry) == 0 && !os_isdir(entry) {
                                continue;
                            }
                            let mut len = CStr::from_ptr(entry).to_bytes().len();
                            if len + 1 + (*search_ctx).ffsc_file_to_search.size >= MAXPATHL {
                                ff_free_stack_element(stackp);
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
                                (*search_ctx).ffsc_file_to_search.data,
                            ) as usize;
                            if file_path.size >= MAXPATHL {
                                ff_free_stack_element(stackp);
                                break 'upward;
                            }

                            len = file_path.size;
                            let mut suf: *mut c_char = if (*search_ctx).ffsc_tagfile != 0 {
                                EMPTY_C.as_ptr().cast_mut()
                            } else {
                                (*curbuf).b_p_sua
                            };
                            loop {
                                if (path_with_url(file_path.data) != 0
                                    || (os_path_exists(file_path.data)
                                        && ((*search_ctx).ffsc_find_what == FINDFILE_BOTH
                                            || (((*search_ctx).ffsc_find_what == FINDFILE_DIR)
                                                == os_isdir(file_path.data)))))
                                    && ff_check_visited(
                                        &raw mut (*(*search_ctx).ffsc_visited_list)
                                            .ffvl_visited_list,
                                        file_path.data,
                                        file_path.size,
                                        EMPTY_C.as_ptr(),
                                        0,
                                    ) == OK
                                {
                                    (*stackp).ffs_filearray_cur = i + 1;
                                    ff_push(search_ctx, stackp);

                                    if path_with_url(file_path.data) == 0 {
                                        file_path.size = simplify_filename(file_path.data);
                                    }

                                    if os_dirname(ff_expand_buffer.data, MAXPATHL) == OK {
                                        ff_expand_buffer.size =
                                            CStr::from_ptr(ff_expand_buffer.data).to_bytes().len();
                                        let p = path_shorten_fname(
                                            file_path.data,
                                            ff_expand_buffer.data,
                                        );
                                        if !p.is_null() {
                                            let new_len =
                                                file_path.data.add(file_path.size).offset_from(p)
                                                    as usize;
                                            ptr::copy(p, file_path.data, new_len + 1);
                                            file_path.size = new_len;
                                        }
                                    }
                                    return file_path.data;
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
                        for i in (*stackp).ffs_filearray_cur..(*stackp).ffs_filearray_size {
                            let entry = *(*stackp).ffs_filearray.add(i as usize);
                            if !os_isdir(entry) {
                                continue;
                            }
                            ff_push(
                                search_ctx,
                                ff_create_stack_element(
                                    entry,
                                    CStr::from_ptr(entry).to_bytes().len(),
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
                    for i in (*stackp).ffs_filearray_cur..(*stackp).ffs_filearray_size {
                        let entry = *(*stackp).ffs_filearray.add(i as usize);
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
                                CStr::from_ptr(entry).to_bytes().len(),
                                (*stackp).ffs_wc_path.data,
                                (*stackp).ffs_wc_path.size,
                                (*stackp).ffs_level - 1,
                                1,
                            ),
                        );
                    }
                }

                ff_free_stack_element(stackp);
            }

            if !(*search_ctx).ffsc_start_dir.data.is_null()
                && !(*search_ctx).ffsc_stopdirs_v.is_empty()
                && !got_int
            {
                let plen = path_end.offset_from((*search_ctx).ffsc_start_dir.data)
                    + isize::from(*path_end != NUL);

                if ff_path_in_stoplist(
                    (*search_ctx).ffsc_start_dir.data,
                    plen as usize,
                    &(*search_ctx).ffsc_stopdirs_v,
                ) {
                    break;
                }

                while path_end > (*search_ctx).ffsc_start_dir.data
                    && vim_ispathsep(c_int::from(*path_end as u8))
                {
                    path_end = path_end.sub(1);
                }
                while path_end > (*search_ctx).ffsc_start_dir.data
                    && !vim_ispathsep(c_int::from(*path_end.sub(1) as u8))
                {
                    path_end = path_end.sub(1);
                }
                *path_end = NUL;

                (*search_ctx).ffsc_start_dir.size =
                    path_end.offset_from((*search_ctx).ffsc_start_dir.data) as usize;
                path_end = path_end.sub(1);

                if *(*search_ctx).ffsc_start_dir.data == NUL {
                    break;
                }

                if (*search_ctx).ffsc_start_dir.size + 1 + (*search_ctx).ffsc_fix_path.size
                    >= MAXPATHL
                {
                    break;
                }
                let add_sep = after_pathsep(
                    (*search_ctx).ffsc_start_dir.data,
                    (*search_ctx)
                        .ffsc_start_dir
                        .data
                        .add((*search_ctx).ffsc_start_dir.size),
                ) == 0;
                file_path.size = vim_snprintf!(
                    file_path.data,
                    MAXPATHL,
                    c"%s%s%s".as_ptr(),
                    (*search_ctx).ffsc_start_dir.data,
                    if add_sep {
                        PATHSEPSTR_C.as_ptr()
                    } else {
                        EMPTY_C.as_ptr()
                    },
                    (*search_ctx).ffsc_fix_path.data,
                ) as usize;
                if file_path.size >= MAXPATHL {
                    break;
                }

                let sptr = ff_create_stack_element(
                    file_path.data,
                    file_path.size,
                    (*search_ctx).ffsc_wc_path.data,
                    (*search_ctx).ffsc_wc_path.size,
                    (*search_ctx).ffsc_level,
                    0,
                );
                ff_push(search_ctx, sptr);
            } else {
                break;
            }
        }

        xfree(file_path.data.cast());
        ptr::null_mut()
    }
}

/// Free the list of lists of visited files and directories.
unsafe fn vim_findfile_free_visited(search_ctx_arg: *mut c_void) {
    unsafe {
        if search_ctx_arg.is_null() {
            return;
        }

        let search_ctx = search_ctx_arg.cast::<ff_search_ctx_T>();
        vim_findfile_free_visited_list(&raw mut (*search_ctx).ffsc_visited_lists_list);
        vim_findfile_free_visited_list(&raw mut (*search_ctx).ffsc_dir_visited_lists_list);
    }
}

unsafe fn vim_findfile_free_visited_list(list_headp: *mut *mut ff_visited_list_hdr_T) {
    unsafe {
        while !(*list_headp).is_null() {
            let vp = (**list_headp).ffvl_next;
            ff_free_visited_list((**list_headp).ffvl_visited_list);
            xfree((**list_headp).ffvl_filename.cast());
            xfree((*list_headp).cast());
            *list_headp = vp;
        }
        *list_headp = ptr::null_mut();
    }
}

unsafe fn ff_free_visited_list(mut vl: *mut ff_visited_T) {
    unsafe {
        while !vl.is_null() {
            let vp = (*vl).ffv_next;
            xfree((*vl).ffv_wc_path.cast());
            ptr::drop_in_place(&raw mut (*vl).ffv_fname);
            xfree(vl.cast());
            vl = vp;
        }
    }
}

unsafe fn ff_get_visited_list(
    filename: *const c_char,
    filenamelen: usize,
    list_headp: *mut *mut ff_visited_list_hdr_T,
) -> *mut ff_visited_list_hdr_T {
    unsafe {
        if !(*list_headp).is_null() {
            let mut retptr = *list_headp;
            while !retptr.is_null() {
                if path_equal(filename, (*retptr).ffvl_filename, kPathCmpLiteral) {
                    return retptr;
                }
                retptr = (*retptr).ffvl_next;
            }
        }

        let retptr: *mut ff_visited_list_hdr_T = xmalloc(size_of::<ff_visited_list_hdr_T>()).cast();
        (*retptr).ffvl_visited_list = ptr::null_mut();
        (*retptr).ffvl_filename = xmemdupz(filename.cast(), filenamelen).cast();
        (*retptr).ffvl_next = *list_headp;
        *list_headp = retptr;

        retptr
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

            let neq = if p_fic != 0 {
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
    visited_list: *mut *mut ff_visited_T,
    fname: *const c_char,
    fnamelen: usize,
    wc_path: *const c_char,
    wc_pathlen: usize,
) -> c_int {
    unsafe {
        let mut url = false;
        let mut file_id = FileID::default();

        if path_with_url(fname) != 0 {
            xmemcpyz(ff_expand_buffer.data.cast(), fname.cast(), fnamelen);
            ff_expand_buffer.size = fnamelen;
            url = true;
        } else {
            *ff_expand_buffer.data = NUL;
            ff_expand_buffer.size = 0;
            if !os_fileid(fname, &raw mut file_id) {
                return FAIL;
            }
        }

        let mut vp = *visited_list;
        while !vp.is_null() {
            if ((url
                && path_equal(
                    (*vp).ffv_fname.as_ptr(),
                    ff_expand_buffer.data,
                    kPathCmpLiteral,
                ))
                || (!url
                    && (*vp).file_id_valid
                    && os_fileid_equal(&raw const (*vp).file_id, &raw const file_id)))
                && ff_wc_equal((*vp).ffv_wc_path, wc_path)
            {
                return FAIL;
            }
            vp = (*vp).ffv_next;
        }

        let vp: *mut ff_visited_T = xmalloc(size_of::<ff_visited_T>()).cast();
        ptr::write(&raw mut (*vp).ffv_fname, Vec::new());

        if !url {
            (*vp).file_id_valid = true;
            (*vp).file_id = file_id;
            (*vp).ffv_fname.push(NUL);
        } else {
            (*vp).file_id_valid = false;
            (*vp).ffv_fname.extend_from_slice(std::slice::from_raw_parts(
                ff_expand_buffer.data,
                ff_expand_buffer.size + 1,
            ));
        }

        if !wc_path.is_null() {
            (*vp).ffv_wc_path = xmemdupz(wc_path.cast(), wc_pathlen).cast();
        } else {
            (*vp).ffv_wc_path = ptr::null_mut();
        }

        (*vp).ffv_next = *visited_list;
        *visited_list = vp;

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
) -> *mut ff_stack_T {
    unsafe {
        let stack: *mut ff_stack_T = xmalloc(size_of::<ff_stack_T>()).cast();
        (*stack).ffs_prev = ptr::null_mut();
        (*stack).ffs_filearray = ptr::null_mut();
        (*stack).ffs_filearray_size = 0;
        (*stack).ffs_filearray_cur = 0;
        (*stack).ffs_stage = 0;
        (*stack).ffs_level = level;
        (*stack).ffs_star_star_empty = star_star_empty;

        if fix_part.is_null() {
            fix_part = EMPTY_C.as_ptr();
            fix_partlen = 0;
        }
        (*stack).ffs_fix_path = cbuf_to_string(fix_part, fix_partlen);

        if wc_part.is_null() {
            wc_part = EMPTY_C.as_ptr();
            wc_partlen = 0;
        }
        (*stack).ffs_wc_path = cbuf_to_string(wc_part, wc_partlen);

        stack
    }
}

unsafe fn ff_push(search_ctx: *mut ff_search_ctx_T, stack_ptr: *mut ff_stack_T) {
    unsafe {
        if stack_ptr.is_null() {
            return;
        }
        (*stack_ptr).ffs_prev = (*search_ctx).ffsc_stack_ptr;
        (*search_ctx).ffsc_stack_ptr = stack_ptr;
    }
}

unsafe fn ff_pop(search_ctx: *mut ff_search_ctx_T) -> *mut ff_stack_T {
    unsafe {
        let sptr = (*search_ctx).ffsc_stack_ptr;
        if !(*search_ctx).ffsc_stack_ptr.is_null() {
            (*search_ctx).ffsc_stack_ptr = (*(*search_ctx).ffsc_stack_ptr).ffs_prev;
        }
        sptr
    }
}

unsafe fn ff_free_stack_element(stack_ptr: *mut ff_stack_T) {
    unsafe {
        if stack_ptr.is_null() {
            return;
        }

        api_clear_string(&mut (*stack_ptr).ffs_fix_path);
        api_clear_string(&mut (*stack_ptr).ffs_wc_path);

        if !(*stack_ptr).ffs_filearray.is_null() {
            FreeWild((*stack_ptr).ffs_filearray_size, (*stack_ptr).ffs_filearray);
        }

        xfree(stack_ptr.cast());
    }
}

unsafe fn ff_clear(search_ctx: *mut ff_search_ctx_T) {
    unsafe {
        loop {
            let sptr = ff_pop(search_ctx);
            if sptr.is_null() {
                break;
            }
            ff_free_stack_element(sptr);
        }

        for dir in (*search_ctx).ffsc_stopdirs_v.drain(..) {
            xfree(dir.data.cast());
        }
        (*search_ctx).ffsc_stopdirs_v = Vec::new();

        api_clear_string(&mut (*search_ctx).ffsc_file_to_search);
        api_clear_string(&mut (*search_ctx).ffsc_start_dir);
        api_clear_string(&mut (*search_ctx).ffsc_fix_path);
        api_clear_string(&mut (*search_ctx).ffsc_wc_path);
        (*search_ctx).ffsc_level = 0;
    }
}

unsafe fn ff_path_in_stoplist(
    path: *const c_char,
    mut path_len: usize,
    stopdirs_v: &[NvimString],
) -> bool {
    unsafe {
        while path_len > 1 && vim_ispathsep(c_int::from(*path.add(path_len - 1) as u8)) {
            path_len -= 1;
        }

        if path_len == 0 {
            return true;
        }

        for &stopdir in stopdirs_v {
            if path_cmp(p_fic != 0, stopdir.data, path, path_len) == 0
                && (stopdir.size <= path_len
                    || vim_ispathsep(c_int::from(*stopdir.data.add(path_len) as u8)))
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
    file_to_find: *mut *mut c_char,
    search_ctx: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        find_file_in_path_option(
            ptr,
            len,
            options,
            first,
            if *(*curbuf).b_p_path == NUL {
                p_path
            } else {
                (*curbuf).b_p_path
            },
            FINDFILE_BOTH,
            rel_fname,
            (*curbuf).b_p_sua,
            file_to_find,
            search_ctx,
        )
    }
}

pub unsafe fn free_findfile() {
    unsafe {
        xfree(ff_expand_buffer.data.cast());
        ff_expand_buffer.data = ptr::null_mut();
        ff_expand_buffer.size = 0;
    }
}

/// Find the directory name `ptr[len]` in the path.
pub unsafe fn find_directory_in_path(
    ptr: *mut c_char,
    len: usize,
    options: c_int,
    rel_fname: *mut c_char,
    file_to_find: *mut *mut c_char,
    search_ctx: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        find_file_in_path_option(
            ptr,
            len,
            options,
            c_int::from(true),
            p_cdpath,
            FINDFILE_DIR,
            rel_fname,
            EMPTY_C.as_ptr().cast_mut(),
            file_to_find,
            search_ctx,
        )
    }
}

pub unsafe fn find_file_in_path_option(
    mut ptr: *mut c_char,
    len: usize,
    options: c_int,
    first: c_int,
    path_option: *mut c_char,
    find_what: c_int,
    mut rel_fname: *mut c_char,
    suffixes: *mut c_char,
    file_to_find: *mut *mut c_char,
    search_ctx_arg: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        let search_ctx = search_ctx_arg.cast::<*mut ff_search_ctx_T>();
        static mut dir: *mut c_char = ptr::null_mut();
        static mut did_findfile_init: bool = false;
        static mut file_to_findlen: usize = 0;
        let mut file_name: *mut c_char = ptr::null_mut();
        let name_buff = (&raw mut NameBuff).cast::<c_char>();

        if !rel_fname.is_null() && path_with_url(rel_fname) != 0 {
            rel_fname = ptr::null_mut();
        }

        if first == c_int::from(true) {
            if len == 0 {
                return ptr::null_mut();
            }

            let save_char = *ptr.add(len);
            *ptr.add(len) = NUL;
            file_to_findlen = expand_env_esc(
                ptr,
                name_buff,
                MAXPATHL as c_int,
                ptr::null_mut(),
                true,
                ptr::null_mut(),
            );
            *ptr.add(len) = save_char;

            xfree((*file_to_find).cast());
            *file_to_find = xmemdupz(name_buff.cast(), file_to_findlen).cast();
            if (options & FNAME_UNESC) != 0 {
                ptr = *file_to_find;
                while *ptr != NUL {
                    if *ptr == b'\\' as c_char && *ptr.add(1) == b' ' as c_char {
                        let count_bytes =
                            (*file_to_find).add(file_to_findlen).offset_from(ptr.add(1)) as usize
                                + 1;
                        ptr::copy(ptr.add(1), ptr, count_bytes);
                        file_to_findlen -= 1;
                    }
                    ptr = ptr.add(1);
                }
            }
        }

        let ftf = *file_to_find;
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
                    return xmemdupz(ftf.cast(), file_to_findlen).cast();
                }

                let rel_fnamelen = if !rel_fname.is_null() {
                    CStr::from_ptr(rel_fname).to_bytes().len()
                } else {
                    0
                };

                let mut run = 1;
                while run <= 2 {
                    let mut l = file_to_findlen;
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
                            return xmemdupz(name_buff.cast(), slen).cast();
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
                vim_findfile_free_visited((*search_ctx).cast());
                dir = path_option;
                did_findfile_init = false;
            }

            loop {
                if did_findfile_init {
                    file_name = vim_findfile((*search_ctx).cast());
                    if !file_name.is_null() {
                        break;
                    }
                    did_findfile_init = false;
                } else {
                    if dir.is_null() || *dir == NUL {
                        vim_findfile_cleanup((*search_ctx).cast());
                        *search_ctx = ptr::null_mut();
                        break;
                    }

                    let buf: *mut c_char = xmalloc(MAXPATHL).cast();
                    *buf = NUL;
                    copy_option_part(&raw mut dir, buf, MAXPATHL, c" ,".as_ptr());

                    let r_ptr = vim_findfile_stopdir(buf);
                    *search_ctx = vim_findfile_init(
                        buf,
                        *file_to_find,
                        file_to_findlen,
                        r_ptr,
                        100,
                        c_int::from(false),
                        find_what,
                        (*search_ctx).cast(),
                        c_int::from(false),
                        rel_fname,
                    )
                    .cast();
                    if !(*search_ctx).is_null() {
                        did_findfile_init = true;
                    }
                    xfree(buf.cast());
                }
            }
        }

        if file_name.is_null() && (options & FNAME_MESS) != 0 {
            if first == c_int::from(true) {
                if find_what == FINDFILE_DIR {
                    semsg!(
                        tr(e_cant_find_directory_str_in_cdpath.as_ptr()),
                        *file_to_find,
                    );
                } else {
                    semsg!(tr(e_cant_find_file_str_in_path.as_ptr()), *file_to_find);
                }
            } else if find_what == FINDFILE_DIR {
                semsg!(
                    tr(e_no_more_directory_str_found_in_cdpath.as_ptr()),
                    *file_to_find,
                );
            } else {
                semsg!(tr(e_no_more_file_str_found_in_path.as_ptr()), *file_to_find);
            }
        }

        file_name
    }
}

/// Get the file name at the cursor.
pub unsafe fn grab_file_name(count: c_int, file_lnum: *mut linenr_T) -> *mut c_char {
    unsafe {
        let options = FNAME_MESS | FNAME_EXP | FNAME_REL | FNAME_UNESC;
        let fname = if Visual.active {
            let mut len: usize = 0;
            let mut ptr: *mut c_char = ptr::null_mut();
            if !get_visual_text(ptr::null_mut(), &raw mut ptr, &raw mut len) {
                return ptr::null_mut();
            }
            if !file_lnum.is_null()
                && *ptr.add(len) == b':' as c_char
                && (*ptr.add(len + 1) as u8).is_ascii_digit()
            {
                let mut p: *mut c_char = ptr.add(len + 1);
                *file_lnum = getdigits_int32(&raw mut p, false, 0);
            }
            find_file_name_in_path(ptr, len, options, count as c_long, (*curbuf).b_ffname)
        } else {
            file_name_at_cursor(options | FNAME_HYP, count, file_lnum)
        };
        #[cfg(windows)]
        {
            path_to_slash(fname);
        }
        fname
    }
}

/// Return the file name under or after the cursor.
pub unsafe fn file_name_at_cursor(
    options: c_int,
    count: c_int,
    file_lnum: *mut linenr_T,
) -> *mut c_char {
    unsafe {
        file_name_in_line(
            get_cursor_line_ptr(),
            (*curwin).w_cursor.col,
            options,
            count,
            (*curbuf).b_ffname,
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
) -> *mut c_char {
    unsafe {
        let mut ptr = line.add(col as usize);
        while *ptr != NUL && !vim_isfilec(c_int::from(*ptr as u8)) {
            ptr = ptr.add(utfc_ptr2len(ptr) as usize);
        }
        if *ptr == NUL {
            if (options & FNAME_MESS) != 0 {
                emsg(tr(c"E446: No file name under cursor".as_ptr()));
            }
            return ptr::null_mut();
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

unsafe fn eval_includeexpr(ptr: *const c_char, len: usize) -> *mut c_char {
    unsafe {
        let save_sctx = current_sctx;
        set_vim_var_string(VV_FNAME, ptr, len as isize);
        current_sctx = (*curbuf).b_p_script_ctx[kBufOptIncludeexpr];

        let use_sandbox = was_set_insecurely(curwin, kOptIncludeexpr, OPT_LOCAL) != 0;
        let mut tv: typval_T = std::mem::zeroed();
        let mut res: *mut c_char = ptr::null_mut();
        if eval_expr_option_tv(&raw mut (*curbuf).b_p_inex, use_sandbox, &raw mut tv) {
            res = xstrdup(tv_get_string(&raw const tv));
            tv_clear(&raw mut tv);
        }

        set_vim_var_string(VV_FNAME, ptr::null(), 0);
        current_sctx = save_sctx;
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
) -> *mut c_char {
    unsafe {
        let mut tofree: *mut c_char = ptr::null_mut();

        if len == 0 {
            return ptr::null_mut();
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

        if (options & FNAME_INCL) != 0 && (*curbuf).b_p_inex.cb_type != kCallbackNone {
            tofree = eval_includeexpr(ptr, len);
            if !tofree.is_null() {
                ptr = tofree;
                len = CStr::from_ptr(ptr).to_bytes().len();
            }
        }

        let mut file_name: *mut c_char;
        if (options & FNAME_EXP) != 0 {
            let mut file_to_find: *mut c_char = ptr::null_mut();
            let mut search_ctx: *mut c_char = ptr::null_mut();

            file_name = find_file_in_path(
                ptr,
                len,
                options & !FNAME_MESS,
                c_int::from(true),
                rel_fname,
                &raw mut file_to_find,
                &raw mut search_ctx,
            );

            if file_name.is_null()
                && (options & FNAME_INCL) == 0
                && (*curbuf).b_p_inex.cb_type != kCallbackNone
            {
                tofree = eval_includeexpr(ptr, len);
                if !tofree.is_null() {
                    ptr = tofree;
                    len = CStr::from_ptr(ptr).to_bytes().len();
                    file_name = find_file_in_path(
                        ptr,
                        len,
                        options & !FNAME_MESS,
                        c_int::from(true),
                        rel_fname,
                        &raw mut file_to_find,
                        &raw mut search_ctx,
                    );
                }
            }
            if file_name.is_null() && (options & FNAME_MESS) != 0 {
                let c = *ptr.add(len);
                *ptr.add(len) = NUL;
                semsg!(tr(c"E447: Can't find file \"%s\" in path".as_ptr()), ptr);
                *ptr.add(len) = c;
            }

            while !file_name.is_null() && {
                count -= 1;
                count > 0
            } {
                xfree(file_name.cast());
                file_name = find_file_in_path(
                    ptr,
                    len,
                    options,
                    c_int::from(false),
                    rel_fname,
                    &raw mut file_to_find,
                    &raw mut search_ctx,
                );
            }

            xfree(file_to_find.cast());
            vim_findfile_cleanup(search_ctx.cast());
        } else {
            file_name = xstrnsave(ptr, len);
        }

        xfree(tofree.cast());
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
        static mut recursive: bool = false;

        let event = if pre {
            EVENT_DIRCHANGEDPRE
        } else {
            EVENT_DIRCHANGED
        };

        if recursive || !has_event(event) {
            return;
        }

        recursive = true;

        let mut save_v_event: save_v_event_T = std::mem::zeroed();
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

        apply_autocmds(event, buf.as_mut_ptr(), new_dir, false, curbuf);

        restore_v_event(dict, &raw mut save_v_event);

        recursive = false;
    }
}

/// Change to a file's directory.
pub unsafe fn vim_chdirfile(fname: *mut c_char, cause: CdCause) -> c_int {
    unsafe {
        let mut dir: [c_char; MAXPATHL] = [0; MAXPATHL];
        let name_buff = (&raw mut NameBuff).cast::<c_char>();

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
        let mut file_to_find: *mut c_char = ptr::null_mut();
        let mut search_ctx: *mut c_char = ptr::null_mut();
        let dir_name = find_directory_in_path(
            new_dir,
            CStr::from_ptr(new_dir).to_bytes().len(),
            FNAME_MESS,
            (*curbuf).b_ffname,
            &raw mut file_to_find,
            &raw mut search_ctx,
        );
        xfree(file_to_find.cast());
        vim_findfile_cleanup(search_ctx.cast());
        if dir_name.is_null() {
            return -1;
        }

        let r = os_chdir(dir_name);
        xfree(dir_name.cast());
        r
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
