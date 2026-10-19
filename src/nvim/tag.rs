//! Code to handle tags and the tag stack (`src/nvim/tag.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::buffer::bt_help;
#[allow(unused_imports)]
use crate::buffer::{buflist_findname_exp, buflist_findnr, buflist_getfile};
#[allow(unused_imports)]
use crate::cmdexpand::{ExpandInit, ExpandOne};
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::set_ref_in_callback;
#[allow(unused_imports)]
use crate::eval::typval::{
    callback_free, tv_clear, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_dict_alloc_lock, tv_dict_find, tv_get_number,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_append_number,
    tv_list_append_tv, tv_list_free,
};
#[allow(unused_imports)]
use crate::eval::typval::{tv_dict_get_number, tv_dict_get_string};
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_string;
#[allow(unused_imports)]
use crate::eval::{callback_call, list2fpos};
#[allow(unused_imports)]
use crate::ex_cmds::{getfile, prepare_tagpreview};
#[allow(unused_imports)]
use crate::ex_docmd::{do_cmdline_cmd, set_no_hlsearch};
#[allow(unused_imports)]
use crate::file_search::{
    vim_findfile, vim_findfile_cleanup, vim_findfile_init, vim_findfile_stopdir,
};
#[allow(unused_imports)]
use crate::fileio::fclose;
#[allow(unused_imports)]
use crate::fileio::vim_fgets;
#[allow(unused_imports)]
use crate::fold::foldOpenCursor;
use crate::globals::*;
#[allow(unused_imports)]
use crate::insexpand::{ins_compl_check_keys, ins_compl_interrupted};
#[allow(unused_imports)]
use crate::lua::executor::nlua_call_excmd;
#[allow(unused_imports)]
use crate::message::msg_delay;
#[allow(unused_imports)]
use crate::message::{
    emsg, give_warning, msg, msg_advance, msg_outtrans, msg_putchar, msg_puts, msg_puts_title,
    verbose_enter, verbose_leave, wait_return,
};
use crate::message::{semsg, smsg};
#[allow(unused_imports)]
use crate::option::copy_option_part;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::optionstr::free_string_option;
#[allow(unused_imports)]
use crate::path::FreeWild;
#[allow(unused_imports)]
use crate::quickfix::set_errorlist;
#[allow(unused_imports)]
use crate::regexp::{skip_regexp, vim_regcomp, vim_regexec, vim_regfree};
#[allow(unused_imports)]
use crate::runtime::do_in_runtimepath;
#[allow(unused_imports)]
use crate::search::do_search;
#[allow(unused_imports)]
use crate::search::{ignorecase, ignorecase_opt};
#[allow(unused_imports)]
use crate::strings::{atoi, gettext, strcmp, strcpy, strlen, strncmp, strpbrk, strstr};
#[allow(unused_imports)]
use crate::strings::{vim_snprintf, vim_snprintf_safelen};
#[allow(unused_imports)]
use crate::strings::{vim_strchr, xstrnsave};
#[allow(unused_imports)]
use crate::window::{
    check_can_set_curbuf_forceit, swbuf_goto_win_with_buf, tabpage_index, win_close, win_enter,
    win_split, win_valid,
};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::command::exarg_T;
use crate::api::tabpage::{taggy_T, win_T};
use crate::autocmd::{EVENT_BUFREADCMD, has_autocmd};
use crate::charset::{skipdigits, vim_isblankline};
use crate::clipboard::{TypvalVval, VarType, list_T, typval_T};
use crate::cursor::{check_cursor, linenr_T};
use crate::drawscreen::redraw_later;
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::{fmark_T, fmarkv_T, pos_T};
use crate::garray::{ga_clear, ga_clear_strings, ga_grow, ga_init, garray_T};
use crate::hashtab::{
    hash_T, hash_add_item, hash_clear, hash_hash, hash_init, hash_lookup, hashitem_empty, hashtab_T,
};
use crate::help::help_heuristic;
use crate::mark::{fm_getname, mark_view_make, mark_view_restore, setpcmark};
use crate::mbyte::{convert_setup, mb_strnicmp, string_convert, utfc_ptr2len, vimconv_T};
use crate::memory::{xfree, xmalloc, xmemcpyz, xmemdupz, xrealloc, xstrdup, xstrlcat, xstrlcpy};
use crate::r#move::{set_topline, validate_cursor};
use crate::os::fs::{os_fopen, os_path_exists};
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::path::{
    FullName_save, PathCmpFlags, kPathCmpExpand, kPathCmpFull, path_equal, path_has_wildcard,
    path_tail, simplify_filename, vim_isAbsName,
};
use crate::state::MODE_INSERT;

pub use crate::eval::buffer::varnumber_T;
pub use crate::eval::funcs::SpecialVarValue;
pub use crate::eval::typval::BoolVarValue;
pub use crate::eval::typval::VarLockStatus;
pub use crate::fileio::FILE;
pub use crate::fileio::off_T;
pub use crate::option::OptInt;
pub use crate::regexp::regprog_T;

const NUL: c_char = 0;
const TAB: c_char = b'\t' as c_char;
const FAIL: c_int = 0;
const OK: c_int = 1;
const NOTDONE: c_int = 2;

const MAXLNUM: linenr_T = 0x7fffffff;
const MAXCOL: c_int = 0x7fffffff;
const MAXPATHL: usize = 4096;
const CMDBUFFSIZE: usize = 1024;
const IOSIZE: usize = 1025;
const LSIZE: c_int = 512;
const TAGSTACKSIZE: c_int = 20;
const NSUBEXP: usize = 10;

const DT_TAG: c_int = 1;
const DT_POP: c_int = 2;
const DT_NEXT: c_int = 3;
const DT_PREV: c_int = 4;
const DT_FIRST: c_int = 5;
const DT_LAST: c_int = 6;
const DT_SELECT: c_int = 7;
const DT_HELP: c_int = 8;
const DT_JUMP: c_int = 9;
const DT_LTAG: c_int = 11;
const DT_FREE: c_int = 99;

const TAG_HELP: c_int = 1;
const TAG_NAMES: c_int = 2;
const TAG_REGEXP: c_int = 4;
const TAG_NOIC: c_int = 8;
const TAG_VERBOSE: c_int = 32;
const TAG_INS_COMP: c_int = 64;
const TAG_KEEP_LANG: c_int = 128;
const TAG_NO_TAGFUNC: c_int = 256;
const TAG_MANY: c_int = 300;

const MT_ST_CUR: usize = 0;
const MT_GL_CUR: usize = 1;
const MT_GL_OTH: usize = 2;
const MT_ST_OTH: usize = 3;
const MT_IC_OFF: usize = 4;
const MT_RE_OFF: usize = 8;
const MT_MASK: usize = 7;
const MT_COUNT: usize = 16;

const NOTAGFILE: c_int = 99;
const TAG_SEP: c_char = 0x02;
const ML_EXTRA: usize = 3;

const GETF_SETMARK: c_int = 0x01;
const GETFILE_SAME_FILE: c_int = 0;
const GETFILE_OPEN_OTHER: c_int = -1;
const GETFILE_UNUSED: c_int = 8;

const WSP_VERT: c_int = 0x02;
const UPD_VALID: c_int = 10;
const SEARCH_KEEP: c_int = 0x400;
const RE_MAGIC: c_int = 1;
const CONV_NONE: c_int = 0;
const FINDFILE_FILE: c_int = 0;
const DIP_ALL: c_int = 0x01;

const EXPAND_FILES: c_int = 2;
const WILD_LIST_NOTFOUND: c_int = 0x01;
const WILD_SILENT: c_int = 0x40;
const WILD_EXPAND_FREE: c_int = 2;

const HLF_D: c_int = 5;
const HLF_W: c_int = 26;

const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;

const VV_SWAPCOMMAND: c_int = 49;

const kCallbackNone: c_int = 0;
const kBoolVarFalse: BoolVarValue = 0;
const kBoolVarTrue: BoolVarValue = 1;
const kSpecialVarNull: SpecialVarValue = 0;
const VAR_UNLOCKED: VarLockStatus = 0;
const VAR_FIXED: VarLockStatus = 2;

const kCpoTagpat: c_int = b't' as c_int;
const kOptJopFlagView: c_uint = 0x02;
const kOptFdoFlagTag: c_uint = 0x80;
const kOptSwbFlagUseopen: c_uint = 0x01;
const kOptSwbFlagUsetab: c_uint = 0x02;
const kOptSwbFlagNewtab: c_uint = 0x08;
const kOptSwbFlagVsplit: c_uint = 0x10;
const kOptTcFlagFollowic: c_uint = 0x01;
const kOptTcFlagIgnore: c_uint = 0x02;
const kOptTcFlagMatch: c_uint = 0x04;
const kOptTcFlagFollowscs: c_uint = 0x08;
const kOptTcFlagSmart: c_uint = 0x10;

const E_TAG_STACK_EMPTY: &CStr = c"E73: Tag stack empty";
const E_TAG_NOT_FOUND_STR: &CStr = c"E426: Tag not found: %s";
const E_AT_BOTTOM_OF_TAG_STACK: &CStr = c"E555: At bottom of tag stack";
const E_AT_TOP_OF_TAG_STACK: &CStr = c"E556: At top of tag stack";
const E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC: &CStr =
    c"E986: Cannot modify the tag stack within tagfunc";
const E_INVALID_RETURN_VALUE_FROM_TAGFUNC: &CStr = c"E987: Invalid return value from tagfunc";
const E_WINDOW_UNEXPECTEDLY_CLOSE_WHILE_SEARCHING_FOR_TAGS: &CStr =
    c"E1299: Window unexpectedly closed while searching for tags";

const MT_NAMES: [&CStr; MT_COUNT / 2] = [
    c"FSC", c"F C", c"F  ", c"FS ", c" SC", c"  C", c"   ", c" S ",
];

/// Structure used for `get_tagfname()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct tagname_T {
    pub tn_tags: *mut c_char,
    pub tn_np: *mut c_char,
    pub tn_did_filefind_init: c_int,
    pub tn_hf_idx: c_int,
    pub tn_search_ctx: *mut c_void,
}

pub use crate::api::command::regmatch_T;

pub use crate::r#match::SearchState;

pub use crate::os::env::expand_T;

#[derive(Clone, Copy)]
struct tagptrs_T {
    tagname: *mut c_char,
    tagname_end: *mut c_char,
    fname: *mut c_char,
    fname_end: *mut c_char,
    command: *mut c_char,
    command_end: *mut c_char,
    tag_fname: *mut c_char,
    tagkind: *mut c_char,
    tagkind_end: *mut c_char,
    user_data: *mut c_char,
    user_data_end: *mut c_char,
    tagline: linenr_T,
}

impl Default for tagptrs_T {
    fn default() -> Self {
        Self {
            tagname: ptr::null_mut(),
            tagname_end: ptr::null_mut(),
            fname: ptr::null_mut(),
            fname_end: ptr::null_mut(),
            command: ptr::null_mut(),
            command_end: ptr::null_mut(),
            tag_fname: ptr::null_mut(),
            tagkind: ptr::null_mut(),
            tagkind_end: ptr::null_mut(),
            user_data: ptr::null_mut(),
            user_data_end: ptr::null_mut(),
            tagline: 0,
        }
    }
}

#[repr(C)]
struct pat_T {
    pat: *mut c_char,
    len: c_int,
    head: *mut c_char,
    headlen: c_int,
    regmatch: regmatch_T,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum tagcmd_T {
    TAGCMD_INVALID,
    TAGCMD_ADDRESS,
    TAGCMD_GENERIC,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum tags_read_status_T {
    TAGS_READ_SUCCESS = 1,
    TAGS_READ_EOF = 2,
    TAGS_READ_IGNORE = 3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum tagsearch_state_T {
    TS_START,
    TS_LINEAR,
    TS_BINARY,
    TS_SKIP_BACK,
    TS_STEP_FORWARD,
}

#[derive(Clone, Copy, Default)]
struct tagsearch_info_T {
    low_offset: off_T,
    high_offset: off_T,
    curr_offset: off_T,
    curr_offset_used: off_T,
    match_offset: off_T,
    low_char: c_int,
    high_char: c_int,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum tagmatch_status_T {
    TAG_MATCH_SUCCESS = 1,
    TAG_MATCH_FAIL = 2,
    TAG_MATCH_STOP = 3,
    TAG_MATCH_NEXT = 4,
}

#[derive(Clone, Copy, Default)]
struct findtags_match_args_T {
    matchoff: c_int,
    match_re: bool,
    match_no_ic: bool,
    has_re: bool,
    sortic: bool,
    sort_error: bool,
}

struct findtags_state_T {
    state: tagsearch_state_T,
    stop_searching: bool,
    orgpat: *mut pat_T,
    lbuf: *mut c_char,
    lbuf_size: c_int,
    tag_fname: *mut c_char,
    fp: *mut FILE,
    flags: c_int,
    tag_file_sorted: c_int,
    get_searchpat: bool,
    help_only: bool,
    did_open: bool,
    mincount: c_int,
    linear: bool,
    vimconv: vimconv_T,
    help_lang: [c_char; 3],
    help_pri: c_int,
    help_lang_find: *mut c_char,
    is_txt: bool,
    match_count: c_int,
    ga_match: [garray_T; MT_COUNT],
    ht_match: [hashtab_T; MT_COUNT],
}

static mut nofile_fname: *mut c_char = ptr::null_mut();
static mut tagmatchname: *mut c_char = ptr::null_mut();
static mut ptag_entry: taggy_T = taggy_T {
    tagname: ptr::null_mut(),
    fmark: fmark_T {
        mark: pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        },
        fnum: 0,
        timestamp: 0,
        view: fmarkv_T {
            topline_offset: MAXLNUM,
            skipcol: 0,
        },
        additional_data: ptr::null_mut(),
    },
    cur_match: 0,
    cur_fnum: 0,
    user_data: ptr::null_mut(),
};
static mut tfu_in_use: bool = false;
static mut tag_fnames: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};

static mut DO_TAG_NUM_MATCHES: c_int = 0;
static mut DO_TAG_MAX_NUM_MATCHES: c_int = 0;
static mut DO_TAG_MATCHES: *mut *mut c_char = ptr::null_mut();
static mut DO_TAG_FLAGS: c_int = 0;

unsafe extern "C" {

    #[cfg(windows)]
    fn _fseeki64(stream: *mut FILE, offset: i64, origin: c_int) -> c_int;
    #[cfg(not(windows))]
    fn fseeko(stream: *mut FILE, offset: off_T, whence: c_int) -> c_int;
    #[cfg(windows)]
    fn _ftelli64(stream: *mut FILE) -> i64;
    #[cfg(not(windows))]
    fn ftello(stream: *mut FILE) -> off_T;

}

#[inline]
unsafe fn vim_fseek(stream: *mut FILE, offset: off_T, whence: c_int) -> c_int {
    #[cfg(windows)]
    {
        unsafe { _fseeki64(stream, offset, whence) }
    }
    #[cfg(not(windows))]
    {
        unsafe { fseeko(stream, offset, whence) }
    }
}

#[inline]
unsafe fn vim_ftell(stream: *mut FILE) -> off_T {
    #[cfg(windows)]
    {
        unsafe { _ftelli64(stream) }
    }
    #[cfg(not(windows))]
    {
        unsafe { ftello(stream) }
    }
}

#[inline]
const fn toupper_asc(c: u8) -> c_int {
    if c < b'a' || c > b'z' {
        c as c_int
    } else {
        (c - (b'a' - b'A')) as c_int
    }
}

#[inline]
const fn ascii_islower(c: c_char) -> bool {
    (c as u8).is_ascii_lowercase()
}

#[inline]
const fn ascii_isalpha(c: c_char) -> bool {
    (c as u8).is_ascii_alphabetic()
}

#[inline]
const fn ascii_isdigit(c: c_char) -> bool {
    (c as u8).is_ascii_digit()
}

#[inline]
const fn ascii_iswhite(c: c_char) -> bool {
    c == b' ' as c_char || c == b'\t' as c_char
}

#[inline]
unsafe fn mb_ptr_adv(p: &mut *mut c_char) {
    unsafe {
        *p = (*p).add(utfc_ptr2len(*p) as usize);
    }
}

#[inline]
unsafe fn to_slash_save(p: *const c_char) -> *mut c_char {
    #[cfg(windows)]
    {
        unsafe { crate::path::path_to_slash_save(p) }
    }
    #[cfg(not(windows))]
    {
        unsafe { xstrdup(p) }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_tagfunc_option() {
    unsafe {
        callback_free(&raw mut p_tfu);
    }
}

/// Mark the global `'tagfunc'` callback with `copyID` so that it is not garbage
/// collected.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_ref_in_tagfunc(copyID: c_int) -> bool {
    unsafe { set_ref_in_callback(&raw mut p_tfu, copyID, ptr::null_mut(), ptr::null_mut()) }
}

/// Jump to tag; handling of tag commands and tag stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_tag(
    eap: *mut exarg_T,
    tag: *mut c_char,
    mut type_: c_int,
    count: c_int,
    forceit: c_int,
    verbose: bool,
) {
    unsafe {
        let tagstack: *mut taggy_T = (&raw mut (*curwin).w_tagstack).cast::<taggy_T>();
        let mut tagstackidx = (*curwin).w_tagstackidx;
        let mut tagstacklen = (*curwin).w_tagstacklen;
        let mut cur_match: c_int = 0;
        let mut cur_fnum = (*curbuf).b_fnum;
        let oldtagstackidx = tagstackidx;
        let mut prevtagstackidx = tagstackidx;
        let mut new_tag = false;
        let mut no_regexp = false;
        let mut error_cur_match: c_int = 0;
        let mut save_pos = false;
        let mut saved_fmark = fmark_T {
            mark: pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            fnum: 0,
            timestamp: 0,
            view: fmarkv_T {
                topline_offset: MAXLNUM,
                skipcol: 0,
            },
            additional_data: ptr::null_mut(),
        };
        let mut new_num_matches: c_int = 0;
        let mut new_matches: *mut *mut c_char = ptr::null_mut();
        let use_tagstack: bool;
        let mut skip_msg = false;
        let mut buf_ffname = (*curbuf).b_ffname;
        let mut use_tfu = true;
        let mut tofree: *mut c_char = ptr::null_mut();

        if type_ == DT_FREE {
            FreeWild(DO_TAG_NUM_MATCHES, DO_TAG_MATCHES);
            DO_TAG_NUM_MATCHES = 0;
            return;
        }

        if tfu_in_use {
            emsg(gettext(E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC.as_ptr()));
            return;
        }

        if postponed_split == 0 && !check_can_set_curbuf_forceit(forceit) {
            return;
        }

        if type_ == DT_HELP {
            type_ = DT_TAG;
            no_regexp = true;
            use_tfu = false;
        }

        let prev_num_matches = DO_TAG_NUM_MATCHES;
        free_string_option(nofile_fname);
        nofile_fname = ptr::null_mut();

        assert!(!tag.is_null());
        'end_do_tag: {
            if p_tgst == 0 && *tag != NUL {
                use_tagstack = false;
                new_tag = true;
                if g_do_tagpreview != 0 {
                    tagstack_clear_entry(&raw mut ptag_entry);
                    ptag_entry.tagname = xstrdup(tag);
                }
            } else {
                use_tagstack = g_do_tagpreview == 0;

                // new pattern, add to the tag stack
                if *tag != NUL
                    && (type_ == DT_TAG
                        || type_ == DT_SELECT
                        || type_ == DT_JUMP
                        || type_ == DT_LTAG)
                {
                    if g_do_tagpreview != 0 {
                        if !ptag_entry.tagname.is_null() && strcmp(ptag_entry.tagname, tag) == 0 {
                            cur_match = ptag_entry.cur_match;
                            cur_fnum = ptag_entry.cur_fnum;
                        } else {
                            tagstack_clear_entry(&raw mut ptag_entry);
                            ptag_entry.tagname = xstrdup(tag);
                        }
                    } else {
                        while tagstackidx < tagstacklen {
                            tagstacklen -= 1;
                            tagstack_clear_entry(tagstack.add(tagstacklen as usize));
                        }

                        tagstacklen += 1;
                        if tagstacklen > TAGSTACKSIZE {
                            tagstacklen = TAGSTACKSIZE;
                            tagstack_clear_entry(tagstack);
                            for i in 1..tagstacklen as usize {
                                *tagstack.add(i - 1) = *tagstack.add(i);
                            }
                            tagstackidx -= 1;
                            (*tagstack.add(tagstackidx as usize)).user_data = ptr::null_mut();
                        }

                        (*tagstack.add(tagstackidx as usize)).tagname = xstrdup(tag);
                        (*curwin).w_tagstacklen = tagstacklen;
                        save_pos = true;
                    }

                    new_tag = true;
                } else {
                    if if g_do_tagpreview != 0 {
                        ptag_entry.tagname.is_null()
                    } else {
                        tagstacklen == 0
                    } {
                        emsg(gettext(E_TAG_STACK_EMPTY.as_ptr()));
                        break 'end_do_tag;
                    }

                    if type_ == DT_POP {
                        let old_KeyTyped = KeyTyped;
                        tagstackidx -= count;
                        if tagstackidx < 0 {
                            emsg(gettext(E_AT_BOTTOM_OF_TAG_STACK.as_ptr()));
                            if tagstackidx + count == 0 {
                                tagstackidx = 0;
                                break 'end_do_tag;
                            }
                            tagstackidx = 0;
                        } else if tagstackidx >= tagstacklen {
                            emsg(gettext(E_AT_TOP_OF_TAG_STACK.as_ptr()));
                            break 'end_do_tag;
                        }

                        saved_fmark = (*tagstack.add(tagstackidx as usize)).fmark;
                        if saved_fmark.fnum != (*curbuf).b_fnum {
                            if buflist_getfile(
                                saved_fmark.fnum,
                                saved_fmark.mark.lnum,
                                GETF_SETMARK,
                                forceit,
                            ) == FAIL
                            {
                                tagstackidx = oldtagstackidx;
                                break 'end_do_tag;
                            }
                            (*curwin).w_cursor.lnum = saved_fmark.mark.lnum;
                        } else {
                            setpcmark();
                            (*curwin).w_cursor.lnum = saved_fmark.mark.lnum;
                        }
                        (*curwin).w_cursor.col = saved_fmark.mark.col;
                        (*curwin).w_set_curswant = true;
                        if (jop_flags & kOptJopFlagView) != 0 {
                            mark_view_restore(&raw mut saved_fmark);
                        }
                        check_cursor(curwin);
                        if (fdo_flags & kOptFdoFlagTag) != 0 && old_KeyTyped {
                            foldOpenCursor();
                        }

                        FreeWild(DO_TAG_NUM_MATCHES, DO_TAG_MATCHES);
                        DO_TAG_NUM_MATCHES = 0;
                        tag_freematch();
                        break 'end_do_tag;
                    }

                    if type_ == DT_TAG || type_ == DT_LTAG {
                        if g_do_tagpreview != 0 {
                            cur_match = ptag_entry.cur_match;
                            cur_fnum = ptag_entry.cur_fnum;
                        } else {
                            save_pos = true;
                            tagstackidx += count - 1;
                            if tagstackidx >= tagstacklen {
                                tagstackidx = tagstacklen - 1;
                                emsg(gettext(E_AT_TOP_OF_TAG_STACK.as_ptr()));
                                save_pos = false;
                            } else if tagstackidx < 0 {
                                emsg(gettext(E_AT_BOTTOM_OF_TAG_STACK.as_ptr()));
                                tagstackidx = 0;
                                break 'end_do_tag;
                            }
                            cur_match = (*tagstack.add(tagstackidx as usize)).cur_match;
                            cur_fnum = (*tagstack.add(tagstackidx as usize)).cur_fnum;
                        }
                        new_tag = true;
                    } else {
                        prevtagstackidx = tagstackidx;

                        if g_do_tagpreview != 0 {
                            cur_match = ptag_entry.cur_match;
                            cur_fnum = ptag_entry.cur_fnum;
                        } else {
                            tagstackidx -= 1;
                            if tagstackidx < 0 {
                                tagstackidx = 0;
                            }
                            cur_match = (*tagstack.add(tagstackidx as usize)).cur_match;
                            cur_fnum = (*tagstack.add(tagstackidx as usize)).cur_fnum;
                        }
                        match type_ {
                            DT_FIRST => cur_match = count - 1,
                            DT_SELECT | DT_JUMP | DT_LAST => cur_match = MAXCOL - 1,
                            DT_NEXT => cur_match = cur_match.saturating_add(count),
                            DT_PREV => cur_match = cur_match.saturating_sub(count),
                            _ => {}
                        }
                        if cur_match == MAXCOL {
                            cur_match = MAXCOL - 1;
                        } else if cur_match < 0 {
                            emsg(gettext(
                                c"E425: Cannot go before first matching tag".as_ptr(),
                            ));
                            skip_msg = true;
                            cur_match = 0;
                            cur_fnum = (*curbuf).b_fnum;
                        }
                    }
                }

                if g_do_tagpreview != 0 {
                    if type_ != DT_SELECT && type_ != DT_JUMP {
                        ptag_entry.cur_match = cur_match;
                        ptag_entry.cur_fnum = cur_fnum;
                    }
                } else {
                    saved_fmark = (*tagstack.add(tagstackidx as usize)).fmark;
                    if save_pos {
                        (*tagstack.add(tagstackidx as usize)).fmark.mark = (*curwin).w_cursor;
                        (*tagstack.add(tagstackidx as usize)).fmark.fnum = (*curbuf).b_fnum;
                        (*tagstack.add(tagstackidx as usize)).fmark.view =
                            mark_view_make(curwin, (*curwin).w_cursor);
                    }

                    (*curwin).w_tagstackidx = tagstackidx;
                    if type_ != DT_SELECT && type_ != DT_JUMP {
                        (*curwin).w_tagstack[tagstackidx as usize].cur_match = cur_match;
                        (*curwin).w_tagstack[tagstackidx as usize].cur_fnum = cur_fnum;
                    }
                }
            }

            if cur_fnum != (*curbuf).b_fnum {
                let buf = buflist_findnr(cur_fnum);
                if !buf.is_null() {
                    buf_ffname = (*buf).b_ffname;
                }
            }

            loop {
                let mut name: *mut c_char = if use_tagstack {
                    let n = xstrdup((*tagstack.add(tagstackidx as usize)).tagname);
                    xfree(tofree.cast());
                    tofree = n;
                    n
                } else if g_do_tagpreview != 0 {
                    ptag_entry.tagname
                } else {
                    tag
                };

                let other_name = tagmatchname.is_null() || strcmp(tagmatchname, name) != 0;
                if new_tag
                    || (cur_match >= DO_TAG_NUM_MATCHES && DO_TAG_MAX_NUM_MATCHES != MAXCOL)
                    || other_name
                {
                    if other_name {
                        xfree(tagmatchname.cast());
                        tagmatchname = xstrdup(name);
                    }

                    if type_ == DT_SELECT || type_ == DT_JUMP || type_ == DT_LTAG {
                        cur_match = MAXCOL - 1;
                    }
                    DO_TAG_MAX_NUM_MATCHES = if type_ == DT_TAG {
                        MAXCOL
                    } else {
                        cur_match + 1
                    };

                    if !no_regexp && *name == b'/' as c_char {
                        DO_TAG_FLAGS = TAG_REGEXP;
                        name = name.add(1);
                    } else {
                        DO_TAG_FLAGS = TAG_NOIC;
                    }

                    if verbose {
                        DO_TAG_FLAGS |= TAG_VERBOSE;
                    }
                    if !use_tfu {
                        DO_TAG_FLAGS |= TAG_NO_TAGFUNC;
                    }

                    if find_tags(
                        name,
                        &raw mut new_num_matches,
                        &raw mut new_matches,
                        DO_TAG_FLAGS,
                        DO_TAG_MAX_NUM_MATCHES,
                        buf_ffname,
                    ) == OK
                        && new_num_matches < DO_TAG_MAX_NUM_MATCHES
                    {
                        DO_TAG_MAX_NUM_MATCHES = MAXCOL;
                    }

                    if tagstack != (&raw mut (*curwin).w_tagstack).cast::<taggy_T>() {
                        emsg(gettext(
                            E_WINDOW_UNEXPECTEDLY_CLOSE_WHILE_SEARCHING_FOR_TAGS.as_ptr(),
                        ));
                        FreeWild(new_num_matches, new_matches);
                        break;
                    }

                    if !new_tag && !other_name {
                        let mut idx: c_int = 0;
                        let mut tagp = tagptrs_T::default();
                        let mut tagp2 = tagptrs_T::default();

                        for j in 0..DO_TAG_NUM_MATCHES as usize {
                            parse_match(*DO_TAG_MATCHES.add(j), &mut tagp);
                            for i in idx..new_num_matches {
                                parse_match(*new_matches.add(i as usize), &mut tagp2);
                                if strcmp(tagp.tagname, tagp2.tagname) == 0 {
                                    let p = *new_matches.add(i as usize);
                                    let mut k = i;
                                    while k > idx {
                                        *new_matches.add(k as usize) =
                                            *new_matches.add((k - 1) as usize);
                                        k -= 1;
                                    }
                                    *new_matches.add(idx as usize) = p;
                                    idx += 1;
                                    break;
                                }
                            }
                        }
                    }
                    FreeWild(DO_TAG_NUM_MATCHES, DO_TAG_MATCHES);
                    DO_TAG_NUM_MATCHES = new_num_matches;
                    DO_TAG_MATCHES = new_matches;
                }

                if DO_TAG_NUM_MATCHES <= 0 {
                    if verbose {
                        semsg!(gettext(E_TAG_NOT_FOUND_STR.as_ptr()), name);
                    }
                    g_do_tagpreview = 0;
                } else {
                    if type_ == DT_TAG && *tag != NUL {
                        cur_match = if count > 0 { count - 1 } else { 0 };
                    } else if type_ == DT_SELECT || (type_ == DT_JUMP && DO_TAG_NUM_MATCHES > 1) {
                        select_tag_match(
                            eap,
                            new_tag,
                            use_tagstack,
                            DO_TAG_NUM_MATCHES,
                            DO_TAG_MATCHES,
                            name,
                        );
                        if use_tagstack {
                            (*tagstack.add(tagstackidx as usize)).fmark = saved_fmark;
                            tagstackidx = prevtagstackidx;
                        }
                        break;
                    } else if type_ == DT_LTAG {
                        if add_llist_tags(tag, DO_TAG_NUM_MATCHES, DO_TAG_MATCHES) == FAIL {
                            break 'end_do_tag;
                        }
                        cur_match = 0;
                    }

                    if cur_match >= DO_TAG_NUM_MATCHES {
                        if (type_ == DT_NEXT || type_ == DT_FIRST) && nofile_fname.is_null() {
                            if DO_TAG_NUM_MATCHES == 1 {
                                emsg(gettext(c"E427: There is only one matching tag".as_ptr()));
                            } else {
                                emsg(gettext(
                                    c"E428: Cannot go beyond last matching tag".as_ptr(),
                                ));
                            }
                            skip_msg = true;
                        }
                        cur_match = DO_TAG_NUM_MATCHES - 1;
                    }
                    if use_tagstack {
                        let mut tagp2 = tagptrs_T::default();

                        (*tagstack.add(tagstackidx as usize)).cur_match = cur_match;
                        (*tagstack.add(tagstackidx as usize)).cur_fnum = cur_fnum;

                        if use_tfu
                            && parse_match(*DO_TAG_MATCHES.add(cur_match as usize), &mut tagp2)
                                == OK
                            && !tagp2.user_data.is_null()
                        {
                            xfree((*tagstack.add(tagstackidx as usize)).user_data.cast());
                            (*tagstack.add(tagstackidx as usize)).user_data = xmemdupz(
                                tagp2.user_data.cast(),
                                tagp2.user_data_end.offset_from(tagp2.user_data) as usize,
                            )
                            .cast();
                        }

                        tagstackidx += 1;
                    } else if g_do_tagpreview != 0 {
                        ptag_entry.cur_match = cur_match;
                        ptag_entry.cur_fnum = cur_fnum;
                    }

                    if !nofile_fname.is_null() && error_cur_match != cur_match {
                        smsg!(
                            0,
                            gettext(c"File \"%s\" does not exist".as_ptr()),
                            nofile_fname,
                        );
                    }

                    let ic =
                        ((*(*DO_TAG_MATCHES.add(cur_match as usize)) as usize) & MT_IC_OFF) != 0;
                    if type_ != DT_TAG
                        && type_ != DT_SELECT
                        && type_ != DT_JUMP
                        && (DO_TAG_NUM_MATCHES > 1 || ic)
                        && !skip_msg
                    {
                        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
                        let more_str = if DO_TAG_MAX_NUM_MATCHES != MAXCOL {
                            gettext(c" or more".as_ptr()).cast_const()
                        } else {
                            c"".as_ptr()
                        };
                        vim_snprintf!(
                            iobuf_ptr,
                            IOSIZE,
                            gettext(c"tag %d of %d%s".as_ptr()),
                            cur_match + 1,
                            DO_TAG_NUM_MATCHES,
                            more_str,
                        );
                        if ic {
                            xstrlcat(
                                iobuf_ptr,
                                gettext(c"  Using tag with different case!".as_ptr()),
                                IOSIZE,
                            );
                        }
                        if (DO_TAG_NUM_MATCHES > prev_num_matches || new_tag)
                            && DO_TAG_NUM_MATCHES > 1
                        {
                            msg(iobuf_ptr, if ic { HLF_W } else { 0 });
                            msg_scroll = 1;
                        } else {
                            give_warning(iobuf_ptr, ic, true);
                        }
                        if ic && msg_scrolled == 0 && msg_silent == 0 {
                            msg_delay(1007, true);
                        }
                    }

                    let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
                    let iobufflen =
                        vim_snprintf_safelen!(iobuf_ptr, IOSIZE, c":ta %s\r".as_ptr(), name);
                    set_vim_var_string(VV_SWAPCOMMAND, iobuf_ptr, iobufflen as isize);

                    let i = jumpto_tag(*DO_TAG_MATCHES.add(cur_match as usize), forceit, true);

                    set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);

                    if i == NOTAGFILE {
                        if (type_ == DT_PREV && cur_match > 0)
                            || ((type_ == DT_TAG || type_ == DT_NEXT || type_ == DT_FIRST)
                                && (DO_TAG_MAX_NUM_MATCHES != MAXCOL
                                    || cur_match < DO_TAG_NUM_MATCHES - 1))
                        {
                            error_cur_match = cur_match;
                            if use_tagstack {
                                tagstackidx -= 1;
                            }
                            if type_ == DT_PREV {
                                cur_match -= 1;
                            } else {
                                type_ = DT_NEXT;
                                cur_match += 1;
                            }
                            continue;
                        }
                        semsg!(
                            gettext(c"E429: File \"%s\" does not exist".as_ptr()),
                            nofile_fname,
                        );
                    } else if use_tagstack && tagstackidx > (*curwin).w_tagstacklen {
                        tagstackidx = (*curwin).w_tagstackidx;
                    }
                }
                break;
            }
        }

        if use_tagstack && tagstackidx <= (*curwin).w_tagstacklen {
            (*curwin).w_tagstackidx = tagstackidx;
        }
        postponed_split = 0;
        g_do_tagpreview = 0;
        xfree(tofree.cast());
    }
}

/// Let the user pick from `matches`. Delegates to (async) `vim.ui.select()`.
unsafe fn select_tag_match(
    eap: *mut exarg_T,
    new_tag: bool,
    use_tagstack: bool,
    num_matches: c_int,
    matches: *mut *mut c_char,
    name: *const c_char,
) {
    unsafe {
        let tagstack: *mut taggy_T = (&raw mut (*curwin).w_tagstack).cast::<taggy_T>();
        let tagstackidx = (*curwin).w_tagstackidx;

        let mut items_tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_number: 0 },
        };
        tv_list_alloc_ret(&raw mut items_tv, num_matches as isize);

        for i in 0..num_matches {
            os_breakcheck();
            if got_int {
                tv_clear(&raw mut items_tv);
                return;
            }
            let mut tagp = tagptrs_T::default();
            parse_match(*matches.add(i as usize), &mut tagp);

            let cur = !new_tag
                && ((g_do_tagpreview != 0 && i == ptag_entry.cur_match)
                    || (use_tagstack && i == (*tagstack.add(tagstackidx as usize)).cur_match));

            let d = tv_dict_alloc();
            tv_dict_add_str_len(
                d,
                c"tag".as_ptr(),
                3,
                tagp.tagname,
                tagp.tagname_end.offset_from(tagp.tagname) as c_int,
            );
            let pri_idx = (*(*matches.add(i as usize)) as usize) & MT_MASK;
            tv_dict_add_str(d, c"pri".as_ptr(), 3, MT_NAMES[pri_idx].as_ptr());
            if !tagp.tagkind.is_null() {
                tv_dict_add_str_len(
                    d,
                    c"kind".as_ptr(),
                    4,
                    tagp.tagkind,
                    tagp.tagkind_end.offset_from(tagp.tagkind) as c_int,
                );
            }
            let fname = tag_full_fname(&mut tagp);
            tv_dict_add_str(
                d,
                c"file".as_ptr(),
                4,
                if !fname.is_null() {
                    fname.cast_const()
                } else {
                    c"".as_ptr()
                },
            );
            xfree(fname.cast());
            tv_dict_add_bool(
                d,
                c"cur".as_ptr(),
                3,
                if cur { kBoolVarTrue } else { kBoolVarFalse },
            );

            let mut item = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_dict: d },
            };
            tv_list_append_tv(items_tv.vval.v_list, &raw mut item);
        }

        let extra_d = tv_dict_alloc();
        tv_dict_add_list(extra_d, c"items".as_ptr(), 5, items_tv.vval.v_list);
        (*items_tv.vval.v_list).lv_refcount += 1;
        tv_dict_add_str(extra_d, c"tagname".as_ptr(), 7, name);
        let mut extra_tv = typval_T {
            v_type: VarType::VAR_DICT,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_dict: extra_d },
        };

        nlua_call_excmd(
            c"vim._core.tag".as_ptr(),
            c"select_tag".as_ptr(),
            eap,
            &raw const cmdmod,
            &raw mut extra_tv,
        );

        tv_clear(&raw mut items_tv);
        tv_clear(&raw mut extra_tv);
    }
}

/// Add the matching tags to the location list for the current window.
unsafe fn add_llist_tags(tag: *mut c_char, num_matches: c_int, matches: *mut *mut c_char) -> c_int {
    unsafe {
        let mut tag_name = [0 as c_char; 128 + 1];
        let mut tagp = tagptrs_T::default();

        let fname = xmalloc(MAXPATHL + 1).cast::<c_char>();
        let cmd = xmalloc(CMDBUFFSIZE + 1).cast::<c_char>();
        let list = tv_list_alloc(0);

        for i in 0..num_matches as usize {
            parse_match(*matches.add(i), &mut tagp);

            let mut len = (tagp.tagname_end.offset_from(tagp.tagname) as c_int).min(128);
            xmemcpyz(
                tag_name.as_mut_ptr().cast(),
                tagp.tagname.cast(),
                len as usize,
            );
            tag_name[len as usize] = NUL;

            let p = tag_full_fname(&mut tagp);
            if p.is_null() {
                continue;
            }
            xstrlcpy(fname, p, MAXPATHL);
            xfree(p.cast());

            let mut lnum: linenr_T = 0;
            if ascii_isdigit(*tagp.command) {
                lnum = atoi(tagp.command);
            } else {
                let mut cmd_start = tagp.command;
                let mut cmd_end = tagp.command_end;
                if cmd_end.is_null() {
                    let mut p_iter = tagp.command;
                    while *p_iter != NUL && *p_iter != b'\r' as c_char && *p_iter != b'\n' as c_char
                    {
                        p_iter = p_iter.add(1);
                    }
                    cmd_end = p_iter;
                }

                cmd_end = cmd_end.sub(1);

                if *cmd_start == b'/' as c_char || *cmd_start == b'?' as c_char {
                    cmd_start = cmd_start.add(1);
                }
                if *cmd_end == b'/' as c_char || *cmd_end == b'?' as c_char {
                    cmd_end = cmd_end.sub(1);
                }

                len = 0;
                *cmd = NUL;

                if *cmd_start == b'^' as c_char {
                    strcpy(cmd, c"^".as_ptr());
                    cmd_start = cmd_start.add(1);
                    len += 1;
                }

                strcpy(cmd.add(len as usize), c"\\V".as_ptr());
                len += 2;

                let cmd_len = ((cmd_end.offset_from(cmd_start) as c_int) + 1)
                    .min(CMDBUFFSIZE as c_int - 5)
                    .max(0);
                if cmd_len > 0 {
                    ptr::copy_nonoverlapping(cmd_start, cmd.add(len as usize), cmd_len as usize);
                    len += cmd_len;
                }
                *cmd.add(len as usize) = NUL;

                if *cmd.add((len - 1) as usize) == b'$' as c_char {
                    *cmd.add((len - 1) as usize) = b'\\' as c_char;
                    *cmd.add(len as usize) = b'$' as c_char;
                    len += 1;
                }

                *cmd.add(len as usize) = NUL;
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(list, dict);

            tv_dict_add_str(dict, c"text".as_ptr(), 4, tag_name.as_ptr());
            tv_dict_add_str(dict, c"filename".as_ptr(), 8, fname);
            tv_dict_add_nr(dict, c"lnum".as_ptr(), 4, lnum as varnumber_T);
            if lnum == 0 {
                tv_dict_add_str(dict, c"pattern".as_ptr(), 7, cmd);
            }
        }

        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
        vim_snprintf!(iobuf_ptr, IOSIZE, c"ltag %s".as_ptr(), tag);
        set_errorlist(curwin, list, b' ' as c_int, iobuf_ptr, ptr::null_mut());

        tv_list_free(list);
        xfree(fname.cast());
        xfree(cmd.cast());

        OK
    }
}

/// Free cached tags.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tag_freematch() {
    unsafe {
        xfree(tagmatchname.cast());
        tagmatchname = ptr::null_mut();
    }
}

unsafe fn taglen_advance(l: c_int) {
    unsafe {
        if l == MAXCOL {
            msg_putchar(b'\n' as c_int);
            msg_advance(24);
        } else {
            msg_advance(13 + l);
        }
    }
}

/// Print the tag stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_tags(_eap: *mut exarg_T) {
    unsafe {
        let tagstack: *mut taggy_T = (&raw mut (*curwin).w_tagstack).cast::<taggy_T>();
        let tagstackidx = (*curwin).w_tagstackidx;
        let tagstacklen = (*curwin).w_tagstacklen;

        msg_puts_title(gettext(
            c"\n  # TO tag         FROM line  in file/text".as_ptr(),
        ));
        for i in 0..tagstacklen {
            let entry = tagstack.add(i as usize);
            if !(*entry).tagname.is_null() {
                let name = fm_getname(&raw mut (*entry).fmark, 30);
                if name.is_null() {
                    continue;
                }

                msg_putchar(b'\n' as c_int);
                let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
                vim_snprintf!(
                    iobuf_ptr,
                    IOSIZE,
                    c"%c%2d %2d %-15s %5d  ".as_ptr(),
                    if i == tagstackidx {
                        b'>' as c_int
                    } else {
                        b' ' as c_int
                    },
                    i + 1,
                    (*entry).cur_match + 1,
                    (*entry).tagname,
                    (*entry).fmark.mark.lnum,
                );
                msg_outtrans(iobuf_ptr, 0, false);
                msg_outtrans(
                    name,
                    if (*entry).fmark.fnum == (*curbuf).b_fnum {
                        HLF_D
                    } else {
                        0
                    },
                    false,
                );
                xfree(name.cast());
            }
        }
        if tagstackidx == tagstacklen {
            msg_puts(c"\n>".as_ptr());
        }
    }
}

unsafe fn tag_strnicmp(mut s1: *const c_char, mut s2: *const c_char, mut len: usize) -> c_int {
    unsafe {
        while len > 0 {
            let i = toupper_asc(*s1 as u8) - toupper_asc(*s2 as u8);
            if i != 0 {
                return i;
            }
            if *s1 == NUL {
                break;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            len -= 1;
        }
        0
    }
}

unsafe fn prepare_pats(pats: *mut pat_T, has_re: bool) {
    unsafe {
        (*pats).head = (*pats).pat;
        (*pats).headlen = (*pats).len;
        if has_re {
            if *(*pats).pat == b'^' as c_char {
                (*pats).head = (*pats).pat.add(1);
            } else if *(*pats).pat == b'\\' as c_char && *(*pats).pat.add(1) == b'<' as c_char {
                (*pats).head = (*pats).pat.add(2);
            }
            if (*pats).head == (*pats).pat {
                (*pats).headlen = 0;
            } else {
                (*pats).headlen = 0;
                while *(*pats).head.add((*pats).headlen as usize) != NUL {
                    let magic_chars = if p_magic != 0 {
                        c".[~*\\$".as_ptr()
                    } else {
                        c"\\$".as_ptr()
                    };
                    if !vim_strchr(
                        magic_chars,
                        *(*pats).head.add((*pats).headlen as usize) as u8 as c_int,
                    )
                    .is_null()
                    {
                        break;
                    }
                    (*pats).headlen += 1;
                }
            }
            if p_tl != 0 && ((*pats).headlen as OptInt) > p_tl {
                (*pats).headlen = p_tl as c_int;
            }
        }

        if has_re {
            (*pats).regmatch.regprog =
                vim_regcomp((*pats).pat, if p_magic != 0 { RE_MAGIC } else { 0 });
        } else {
            (*pats).regmatch.regprog = ptr::null_mut();
        }
    }
}

unsafe fn tagfunc_cmd_kind(cmd: *mut c_char) -> tagcmd_T {
    unsafe {
        if ascii_isdigit(*cmd) {
            return if *skipdigits(cmd) == NUL {
                tagcmd_T::TAGCMD_ADDRESS
            } else {
                tagcmd_T::TAGCMD_INVALID
            };
        }
        if *cmd == b'/' as c_char || *cmd == b'?' as c_char {
            let end = skip_regexp(cmd.add(1), *cmd as c_int, 0);
            return if *end == *cmd && *end.add(1) == NUL {
                tagcmd_T::TAGCMD_ADDRESS
            } else {
                tagcmd_T::TAGCMD_INVALID
            };
        }
        if !strpbrk(cmd, c"\t\r\n".as_ptr()).is_null() {
            return tagcmd_T::TAGCMD_INVALID;
        }
        tagcmd_T::TAGCMD_GENERIC
    }
}

unsafe fn find_tagfunc_tags(
    pat: *mut c_char,
    ga: *mut garray_T,
    match_count: *mut c_int,
    flags: c_int,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let mut ntags: c_int = 0;
        let mut args = [typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_number: 0 },
        }; 4];
        let mut rettv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_number: 0 },
        };
        let mut flagString = [0 as c_char; 4];
        let mut tag: *mut taggy_T = ptr::null_mut();

        if (*curwin).w_tagstacklen > 0 {
            let tagstack_ptr = (&raw mut (*curwin).w_tagstack).cast::<taggy_T>();
            if (*curwin).w_tagstackidx == (*curwin).w_tagstacklen {
                tag = tagstack_ptr.add(((*curwin).w_tagstackidx - 1) as usize);
            } else {
                tag = tagstack_ptr.add((*curwin).w_tagstackidx as usize);
            }
        }

        if (*curbuf).b_p_tfu.cb_type == kCallbackNone {
            return FAIL;
        }

        args[0].v_type = VarType::VAR_STRING;
        args[0].vval.v_string = pat;
        args[1].v_type = VarType::VAR_STRING;
        args[1].vval.v_string = flagString.as_mut_ptr();

        let d = tv_dict_alloc_lock(VAR_FIXED);
        if (flags & TAG_INS_COMP) == 0 && !tag.is_null() && !(*tag).user_data.is_null() {
            tv_dict_add_str(d, c"user_data".as_ptr(), 9, (*tag).user_data);
        }
        if !buf_ffname.is_null() {
            tv_dict_add_str(d, c"buf_ffname".as_ptr(), 10, buf_ffname);
        }

        (*d).dv_refcount += 1;
        args[2].v_type = VarType::VAR_DICT;
        args[2].vval.v_dict = d;
        args[3].v_type = VarType::VAR_UNKNOWN;

        vim_snprintf!(
            flagString.as_mut_ptr(),
            flagString.len(),
            c"%s%s%s".as_ptr(),
            if g_tag_at_cursor {
                c"c".as_ptr()
            } else {
                c"".as_ptr()
            },
            if (flags & TAG_INS_COMP) != 0 {
                c"i".as_ptr()
            } else {
                c"".as_ptr()
            },
            if (flags & TAG_REGEXP) != 0 {
                c"r".as_ptr()
            } else {
                c"".as_ptr()
            },
        );

        let save_pos = (*curwin).w_cursor;
        let mut result = if callback_call(
            &raw mut (*curbuf).b_p_tfu,
            3,
            args.as_mut_ptr(),
            &raw mut rettv,
        ) {
            OK
        } else {
            FAIL
        };
        (*curwin).w_cursor = save_pos;
        check_cursor(curwin);
        (*d).dv_refcount -= 1;

        if result == FAIL {
            return FAIL;
        }
        if rettv.v_type == VarType::VAR_SPECIAL && rettv.vval.v_special == kSpecialVarNull {
            tv_clear(&raw mut rettv);
            return NOTDONE;
        }
        if rettv.v_type != VarType::VAR_LIST || rettv.vval.v_list.is_null() {
            tv_clear(&raw mut rettv);
            emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC.as_ptr()));
            return FAIL;
        }
        let taglist = rettv.vval.v_list;
        let key_off = std::mem::offset_of!(dictitem_T, di_key);

        let mut li = (*taglist).lv_first;
        while !li.is_null() {
            let mut res_name: *mut c_char = ptr::null_mut();
            let mut res_fname: *mut c_char = ptr::null_mut();
            let mut res_cmd: *mut c_char = ptr::null_mut();
            let mut res_kind: *mut c_char = ptr::null_mut();
            let mut has_extra = false;
            let name_only = (flags & TAG_NAMES) != 0;

            if (*li).li_tv.v_type != VarType::VAR_DICT {
                emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC.as_ptr()));
                break;
            }

            let mut len: usize = 2;
            let item_dict = (*li).li_tv.vval.v_dict;
            if !item_dict.is_null() {
                let mut todo = (*item_dict).dv_hashtab.ht_used;
                let mut hi = (*item_dict).dv_hashtab.ht_array;
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        todo -= 1;
                        let di = (*hi).hi_key.sub(key_off).cast::<dictitem_T>();
                        let dict_key = (*hi).hi_key;
                        let tv = &raw mut (*di).di_tv;

                        if (*tv).v_type == VarType::VAR_STRING && !(*tv).vval.v_string.is_null() {
                            len += strlen((*tv).vval.v_string) + 1;
                            if strcmp(dict_key, c"name".as_ptr()) == 0 {
                                res_name = (*tv).vval.v_string;
                            } else if strcmp(dict_key, c"filename".as_ptr()) == 0 {
                                res_fname = (*tv).vval.v_string;
                            } else if strcmp(dict_key, c"cmd".as_ptr()) == 0 {
                                res_cmd = (*tv).vval.v_string;
                            } else {
                                has_extra = true;
                                if strcmp(dict_key, c"kind".as_ptr()) == 0 {
                                    res_kind = (*tv).vval.v_string;
                                } else {
                                    len += strlen(dict_key) + 1;
                                }
                            }
                        }
                    }
                    hi = hi.add(1);
                }
            }

            if has_extra {
                len += 2;
            }

            if res_name.is_null() || res_fname.is_null() || res_cmd.is_null() {
                emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC.as_ptr()));
                break;
            }

            let cmdkind = tagfunc_cmd_kind(res_cmd);
            if cmdkind == tagcmd_T::TAGCMD_INVALID {
                emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC.as_ptr()));
                break;
            }
            if cmdkind == tagcmd_T::TAGCMD_GENERIC {
                len += 3;
            }
            let mfp = if name_only {
                xstrdup(res_name)
            } else {
                xmalloc(len + 2).cast::<c_char>()
            };

            if !name_only {
                let mut p = mfp;
                *p = (MT_GL_OTH + 1) as c_char;
                p = p.add(1);
                *p = TAG_SEP;
                p = p.add(1);

                strcpy(p, res_name);
                p = p.add(strlen(p));

                *p = TAB;
                p = p.add(1);
                strcpy(p, res_fname);
                p = p.add(strlen(p));

                *p = TAB;
                p = p.add(1);
                strcpy(p, res_cmd);
                p = p.add(strlen(p));

                if cmdkind == tagcmd_T::TAGCMD_GENERIC {
                    *p = b'|' as c_char;
                    p = p.add(1);
                }
                if cmdkind == tagcmd_T::TAGCMD_GENERIC || has_extra {
                    strcpy(p, c";\"".as_ptr());
                    p = p.add(strlen(p));

                    if !res_kind.is_null() {
                        *p = TAB;
                        p = p.add(1);
                        strcpy(p, res_kind);
                        p = p.add(strlen(p));
                    }

                    if !item_dict.is_null() {
                        let mut todo = (*item_dict).dv_hashtab.ht_used;
                        let mut hi = (*item_dict).dv_hashtab.ht_array;
                        while todo > 0 {
                            if !hashitem_empty(&*hi) {
                                todo -= 1;
                                let di = (*hi).hi_key.sub(key_off).cast::<dictitem_T>();
                                let dict_key = (*hi).hi_key;
                                let tv = &raw mut (*di).di_tv;

                                if (*tv).v_type == VarType::VAR_STRING
                                    && !(*tv).vval.v_string.is_null()
                                    && strcmp(dict_key, c"name".as_ptr()) != 0
                                    && strcmp(dict_key, c"filename".as_ptr()) != 0
                                    && strcmp(dict_key, c"cmd".as_ptr()) != 0
                                    && strcmp(dict_key, c"kind".as_ptr()) != 0
                                {
                                    *p = TAB;
                                    p = p.add(1);
                                    strcpy(p, dict_key);
                                    p = p.add(strlen(p));
                                    strcpy(p, c":".as_ptr());
                                    p = p.add(strlen(p));
                                    strcpy(p, (*tv).vval.v_string);
                                    p = p.add(strlen(p));
                                }
                            }
                            hi = hi.add(1);
                        }
                    }
                }
            }

            ga_grow(ga, 1);
            *((*ga).ga_data.cast::<*mut c_char>()).add((*ga).ga_len as usize) = mfp;
            (*ga).ga_len += 1;
            ntags += 1;
            result = OK;

            li = (*li).li_next;
        }

        tv_clear(&raw mut rettv);

        *match_count = ntags;
        result
    }
}

unsafe fn findtags_state_init(
    st: *mut findtags_state_T,
    pat: *mut c_char,
    flags: c_int,
    mincount: c_int,
) {
    unsafe {
        (*st).tag_fname = xmalloc(MAXPATHL + 1).cast::<c_char>();
        (*st).fp = ptr::null_mut();
        (*st).orgpat = xmalloc(size_of::<pat_T>()).cast::<pat_T>();
        (*(*st).orgpat).pat = pat;
        (*(*st).orgpat).len = strlen(pat) as c_int;
        (*(*st).orgpat).head = ptr::null_mut();
        (*(*st).orgpat).headlen = 0;
        (*(*st).orgpat).regmatch = regmatch_T {
            regprog: ptr::null_mut(),
            startp: [ptr::null_mut(); NSUBEXP],
            endp: [ptr::null_mut(); NSUBEXP],
            rm_matchcol: 0,
            rm_ic: false,
        };
        (*st).flags = flags;
        (*st).tag_file_sorted = NUL as c_int;
        (*st).help_lang_find = ptr::null_mut();
        (*st).is_txt = false;
        (*st).did_open = false;
        (*st).help_only = (flags & TAG_HELP) != 0;
        (*st).get_searchpat = false;
        (*st).help_lang = [NUL; 3];
        (*st).help_pri = 0;
        (*st).mincount = mincount;
        (*st).lbuf_size = LSIZE;
        (*st).lbuf = xmalloc((*st).lbuf_size as usize).cast::<c_char>();
        (*st).match_count = 0;
        (*st).stop_searching = false;

        for mtt in 0..MT_COUNT {
            ga_init(
                &raw mut (*st).ga_match[mtt],
                size_of::<*mut c_char>() as c_int,
                100,
            );
            hash_init(&raw mut (*st).ht_match[mtt]);
        }
    }
}

unsafe fn findtags_state_free(st: *mut findtags_state_T) {
    unsafe {
        xfree((*st).tag_fname.cast());
        xfree((*st).lbuf.cast());
        vim_regfree((*(*st).orgpat).regmatch.regprog);
        xfree((*st).orgpat.cast());
    }
}

unsafe fn findtags_in_help_init(st: *mut findtags_state_T) -> bool {
    unsafe {
        if (*st).is_txt {
            strcpy((*st).help_lang.as_mut_ptr(), c"en".as_ptr());
        } else {
            let i = strlen((*st).tag_fname) as c_int;
            if i > 3 && *(*st).tag_fname.add((i - 3) as usize) == b'-' as c_char {
                xmemcpyz(
                    (*st).help_lang.as_mut_ptr().cast(),
                    (*st).tag_fname.add((i - 2) as usize).cast(),
                    2,
                );
            } else {
                strcpy((*st).help_lang.as_mut_ptr(), c"en".as_ptr());
            }
        }

        if !(*st).help_lang_find.is_null()
            && mb_strnicmp(
                (*st).help_lang.as_ptr(),
                (*st).help_lang_find,
                MAXCOL as usize,
            ) != 0
        {
            return false;
        }

        let fname_len = if !(*curbuf).b_fname.is_null() {
            strlen((*curbuf).b_fname) as c_int
        } else {
            0
        };
        if ((*st).flags & TAG_KEEP_LANG) != 0
            && (*st).help_lang_find.is_null()
            && !(*curbuf).b_fname.is_null()
            && fname_len > 4
            && *(*curbuf).b_fname.add((fname_len - 1) as usize) == b'x' as c_char
            && *(*curbuf).b_fname.add((fname_len - 4) as usize) == b'.' as c_char
            && mb_strnicmp(
                (*curbuf).b_fname.add((fname_len - 3) as usize),
                (*st).help_lang.as_ptr(),
                2,
            ) == 0
        {
            (*st).help_pri = 0;
        } else {
            (*st).help_pri = 1;
            let mut s = p_hlg;
            while *s != NUL {
                if mb_strnicmp(s, (*st).help_lang.as_ptr(), 2) == 0 {
                    break;
                }
                (*st).help_pri += 1;
                s = vim_strchr(s, b',' as c_int);
                if s.is_null() {
                    break;
                }
                s = s.add(1);
            }
            if s.is_null() || *s == NUL {
                (*st).help_pri += 1;
                if mb_strnicmp((*st).help_lang.as_ptr(), c"en".as_ptr(), MAXCOL as usize) != 0 {
                    (*st).help_pri += 1;
                }
            }
        }

        true
    }
}

unsafe fn findtags_apply_tfu(
    st: *mut findtags_state_T,
    pat: *mut c_char,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let use_tfu = ((*st).flags & TAG_NO_TAGFUNC) == 0;

        if !use_tfu || tfu_in_use || (*curbuf).b_p_tfu.cb_type == kCallbackNone {
            return NOTDONE;
        }

        tfu_in_use = true;
        let retval = find_tagfunc_tags(
            pat,
            (&raw mut (*st).ga_match).cast::<garray_T>(),
            &raw mut (*st).match_count,
            (*st).flags,
            buf_ffname,
        );
        tfu_in_use = false;

        retval
    }
}

unsafe fn findtags_get_next_line(
    st: *mut findtags_state_T,
    sinfo_p: *mut tagsearch_info_T,
) -> tags_read_status_T {
    unsafe {
        let mut eof: bool;

        if (*st).state == tagsearch_state_T::TS_BINARY {
            let offset =
                (*sinfo_p).low_offset + (((*sinfo_p).high_offset - (*sinfo_p).low_offset) / 2);
            if offset == (*sinfo_p).curr_offset {
                return tags_read_status_T::TAGS_READ_EOF;
            } else {
                (*sinfo_p).curr_offset = offset;
            }
        } else if (*st).state == tagsearch_state_T::TS_SKIP_BACK {
            (*sinfo_p).curr_offset -= ((*st).lbuf_size * 2) as off_T;
            if (*sinfo_p).curr_offset < 0 {
                (*sinfo_p).curr_offset = 0;
                vim_fseek((*st).fp, 0, SEEK_SET);
                (*st).state = tagsearch_state_T::TS_STEP_FORWARD;
            }
        }

        if (*st).state == tagsearch_state_T::TS_BINARY
            || (*st).state == tagsearch_state_T::TS_SKIP_BACK
        {
            (*sinfo_p).curr_offset_used = (*sinfo_p).curr_offset;
            vim_ignored = vim_fseek((*st).fp, (*sinfo_p).curr_offset, SEEK_SET);
            eof = vim_fgets((*st).lbuf, (*st).lbuf_size, (*st).fp);
            if !eof && (*sinfo_p).curr_offset != 0 {
                (*sinfo_p).curr_offset = vim_ftell((*st).fp);
                if (*sinfo_p).curr_offset == (*sinfo_p).high_offset {
                    vim_ignored = vim_fseek((*st).fp, (*sinfo_p).low_offset, SEEK_SET);
                    (*sinfo_p).curr_offset = (*sinfo_p).low_offset;
                }
                eof = vim_fgets((*st).lbuf, (*st).lbuf_size, (*st).fp);
            }
            while !eof && vim_isblankline((*st).lbuf) {
                (*sinfo_p).curr_offset = vim_ftell((*st).fp);
                eof = vim_fgets((*st).lbuf, (*st).lbuf_size, (*st).fp);
            }
            if eof {
                (*st).state = tagsearch_state_T::TS_SKIP_BACK;
                (*sinfo_p).match_offset = vim_ftell((*st).fp);
                (*sinfo_p).curr_offset = (*sinfo_p).curr_offset_used;
                return tags_read_status_T::TAGS_READ_IGNORE;
            }
        } else {
            loop {
                eof = vim_fgets((*st).lbuf, (*st).lbuf_size, (*st).fp);
                if eof || !vim_isblankline((*st).lbuf) {
                    break;
                }
            }
            if eof {
                return tags_read_status_T::TAGS_READ_EOF;
            }
        }

        tags_read_status_T::TAGS_READ_SUCCESS
    }
}

unsafe fn findtags_hdr_parse(st: *mut findtags_state_T) -> bool {
    unsafe {
        if strncmp((*st).lbuf, c"!_TAG_".as_ptr(), 6) != 0 {
            return true;
        }

        if strncmp((*st).lbuf, c"!_TAG_FILE_SORTED\t".as_ptr(), 18) == 0 {
            (*st).tag_file_sorted = *(*st).lbuf.add(18) as u8 as c_int;
        }
        if strncmp((*st).lbuf, c"!_TAG_FILE_ENCODING\t".as_ptr(), 20) == 0 {
            let mut p = (*st).lbuf.add(20);
            while (*p as u8) > b' ' && (*p as u8) < 127 {
                p = p.add(1);
            }
            *p = NUL;
            convert_setup(&raw mut (*st).vimconv, (*st).lbuf.add(20), p_enc);
        }

        false
    }
}

unsafe fn findtags_start_state_handler(
    st: *mut findtags_state_T,
    sortic: *mut bool,
    sinfo_p: *mut tagsearch_info_T,
) -> bool {
    unsafe {
        let noic = ((*st).flags & TAG_NOIC) != 0;

        if strncmp((*st).lbuf, c"!_TAG_".as_ptr(), 6) <= 0
            || (*(*st).lbuf == b'!' as c_char && ascii_islower(*(*st).lbuf.add(1)))
        {
            return findtags_hdr_parse(st);
        }

        if (*st).linear {
            (*st).state = tagsearch_state_T::TS_LINEAR;
        } else if (*st).tag_file_sorted == NUL as c_int || (*st).tag_file_sorted == b'1' as c_int {
            (*st).state = tagsearch_state_T::TS_BINARY;
        } else if (*st).tag_file_sorted == b'2' as c_int {
            (*st).state = tagsearch_state_T::TS_BINARY;
            *sortic = true;
            (*(*st).orgpat).regmatch.rm_ic = p_ic != 0 || !noic;
        } else {
            (*st).state = tagsearch_state_T::TS_LINEAR;
        }

        if (*st).state == tagsearch_state_T::TS_BINARY && (*(*st).orgpat).regmatch.rm_ic && !*sortic
        {
            (*st).linear = true;
            (*st).state = tagsearch_state_T::TS_LINEAR;
        }

        if (*st).state == tagsearch_state_T::TS_BINARY {
            if vim_fseek((*st).fp, 0, SEEK_END) != 0 {
                (*st).state = tagsearch_state_T::TS_LINEAR;
            } else {
                let filesize = vim_ftell((*st).fp);
                vim_ignored = vim_fseek((*st).fp, 0, SEEK_SET);

                (*sinfo_p).low_offset = 0;
                (*sinfo_p).low_char = 0;
                (*sinfo_p).high_offset = filesize;
                (*sinfo_p).curr_offset = 0;
                (*sinfo_p).high_char = 0xff;
            }
            return false;
        }

        true
    }
}

unsafe fn findtags_parse_line(
    st: *mut findtags_state_T,
    tagpp: &mut tagptrs_T,
    margs: &mut findtags_match_args_T,
    sinfo_p: *mut tagsearch_info_T,
) -> tagmatch_status_T {
    unsafe {
        let status: c_int;

        if (*(*st).orgpat).headlen != 0 {
            *tagpp = tagptrs_T::default();
            tagpp.tagname = (*st).lbuf;
            tagpp.tagname_end = vim_strchr((*st).lbuf, TAB as c_int);
            if tagpp.tagname_end.is_null() {
                return tagmatch_status_T::TAG_MATCH_FAIL;
            }

            let mut cmplen = tagpp.tagname_end.offset_from(tagpp.tagname) as c_int;
            if p_tl != 0 && (cmplen as OptInt) > p_tl {
                cmplen = p_tl as c_int;
            }
            if ((*st).flags & TAG_REGEXP) != 0 && (*(*st).orgpat).headlen < cmplen {
                cmplen = (*(*st).orgpat).headlen;
            } else if (*st).state == tagsearch_state_T::TS_LINEAR
                && (*(*st).orgpat).headlen != cmplen
            {
                return tagmatch_status_T::TAG_MATCH_NEXT;
            }

            if (*st).state == tagsearch_state_T::TS_BINARY {
                let mut i = *tagpp.tagname as u8 as c_int;
                if margs.sortic {
                    i = toupper_asc(*tagpp.tagname as u8);
                }
                if i < (*sinfo_p).low_char || i > (*sinfo_p).high_char {
                    margs.sort_error = true;
                }

                let mut tagcmp = if margs.sortic {
                    tag_strnicmp(tagpp.tagname, (*(*st).orgpat).head, cmplen as usize)
                } else {
                    strncmp(tagpp.tagname, (*(*st).orgpat).head, cmplen as usize)
                };

                if tagcmp == 0 {
                    if cmplen < (*(*st).orgpat).headlen {
                        tagcmp = -1;
                    } else if cmplen > (*(*st).orgpat).headlen {
                        tagcmp = 1;
                    }
                }

                if tagcmp == 0 {
                    (*st).state = tagsearch_state_T::TS_SKIP_BACK;
                    (*sinfo_p).match_offset = (*sinfo_p).curr_offset;
                    return tagmatch_status_T::TAG_MATCH_NEXT;
                }
                if tagcmp < 0 {
                    (*sinfo_p).curr_offset = vim_ftell((*st).fp);
                    if (*sinfo_p).curr_offset < (*sinfo_p).high_offset {
                        (*sinfo_p).low_offset = (*sinfo_p).curr_offset;
                        if margs.sortic {
                            (*sinfo_p).low_char = toupper_asc(*tagpp.tagname as u8);
                        } else {
                            (*sinfo_p).low_char = *tagpp.tagname as u8 as c_int;
                        }
                        return tagmatch_status_T::TAG_MATCH_NEXT;
                    }
                }
                if tagcmp > 0 && (*sinfo_p).curr_offset != (*sinfo_p).high_offset {
                    (*sinfo_p).high_offset = (*sinfo_p).curr_offset;
                    if margs.sortic {
                        (*sinfo_p).high_char = toupper_asc(*tagpp.tagname as u8);
                    } else {
                        (*sinfo_p).high_char = *tagpp.tagname as u8 as c_int;
                    }
                    return tagmatch_status_T::TAG_MATCH_NEXT;
                }

                return tagmatch_status_T::TAG_MATCH_STOP;
            } else if (*st).state == tagsearch_state_T::TS_SKIP_BACK {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, (*(*st).orgpat).head, cmplen as usize) != 0 {
                    (*st).state = tagsearch_state_T::TS_STEP_FORWARD;
                } else {
                    (*sinfo_p).curr_offset = (*sinfo_p).curr_offset_used;
                }
                return tagmatch_status_T::TAG_MATCH_NEXT;
            } else if (*st).state == tagsearch_state_T::TS_STEP_FORWARD {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, (*(*st).orgpat).head, cmplen as usize) != 0 {
                    return if vim_ftell((*st).fp) > (*sinfo_p).match_offset {
                        tagmatch_status_T::TAG_MATCH_STOP
                    } else {
                        tagmatch_status_T::TAG_MATCH_NEXT
                    };
                }
            } else {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, (*(*st).orgpat).head, cmplen as usize) != 0 {
                    return tagmatch_status_T::TAG_MATCH_NEXT;
                }
            }

            tagpp.fname = tagpp.tagname_end.add(1);
            tagpp.fname_end = vim_strchr(tagpp.fname, TAB as c_int);
            if tagpp.fname_end.is_null() {
                status = FAIL;
            } else {
                tagpp.command = tagpp.fname_end.add(1);
                status = OK;
            }
        } else {
            status = parse_tag_line((*st).lbuf, tagpp);
        }

        if status == FAIL {
            tagmatch_status_T::TAG_MATCH_FAIL
        } else {
            tagmatch_status_T::TAG_MATCH_SUCCESS
        }
    }
}

fn findtags_matchargs_init(margs: &mut findtags_match_args_T, flags: c_int) {
    margs.matchoff = 0;
    margs.match_re = false;
    margs.match_no_ic = false;
    margs.has_re = (flags & TAG_REGEXP) != 0;
    margs.sortic = false;
    margs.sort_error = false;
}

unsafe fn findtags_match_tag(
    st: *mut findtags_state_T,
    tagpp: &mut tagptrs_T,
    margs: &mut findtags_match_args_T,
) -> bool {
    unsafe {
        let mut match_: bool;

        let mut cmplen = tagpp.tagname_end.offset_from(tagpp.tagname) as c_int;
        if p_tl != 0 && (cmplen as OptInt) > p_tl {
            cmplen = p_tl as c_int;
        }
        if (*(*st).orgpat).len != cmplen {
            match_ = false;
        } else if (*(*st).orgpat).regmatch.rm_ic {
            assert!(cmplen >= 0);
            match_ = mb_strnicmp(tagpp.tagname, (*(*st).orgpat).pat, cmplen as usize) == 0;
            if match_ {
                margs.match_no_ic =
                    strncmp(tagpp.tagname, (*(*st).orgpat).pat, cmplen as usize) == 0;
            }
        } else {
            match_ = strncmp(tagpp.tagname, (*(*st).orgpat).pat, cmplen as usize) == 0;
        }

        margs.match_re = false;
        if !match_ && !(*(*st).orgpat).regmatch.regprog.is_null() {
            let cc = *tagpp.tagname_end;
            *tagpp.tagname_end = NUL;
            match_ = vim_regexec(&raw mut (*(*st).orgpat).regmatch, tagpp.tagname, 0);
            if match_ {
                margs.matchoff =
                    (*(*st).orgpat).regmatch.startp[0].offset_from(tagpp.tagname) as c_int;
                if (*(*st).orgpat).regmatch.rm_ic {
                    (*(*st).orgpat).regmatch.rm_ic = false;
                    margs.match_no_ic =
                        vim_regexec(&raw mut (*(*st).orgpat).regmatch, tagpp.tagname, 0);
                    (*(*st).orgpat).regmatch.rm_ic = true;
                }
            }
            *tagpp.tagname_end = cc;
            margs.match_re = true;
        }

        match_
    }
}

unsafe fn findtags_string_convert(st: *mut findtags_state_T) {
    unsafe {
        let conv_line = string_convert(&raw const (*st).vimconv, (*st).lbuf, ptr::null_mut());
        if conv_line.is_null() {
            return;
        }

        let len = strlen(conv_line) as c_int + 1;
        if len > (*st).lbuf_size {
            xfree((*st).lbuf.cast());
            (*st).lbuf = conv_line;
            (*st).lbuf_size = len;
        } else {
            strcpy((*st).lbuf, conv_line);
            xfree(conv_line.cast());
        }
    }
}

unsafe fn findtags_add_match(
    st: *mut findtags_state_T,
    tagpp: &mut tagptrs_T,
    margs: &findtags_match_args_T,
    buf_ffname: *mut c_char,
    hash: *mut hash_T,
) {
    unsafe {
        let name_only = ((*st).flags & TAG_NAMES) != 0;
        let mfp: *mut c_char;

        let is_current =
            test_for_current(tagpp.fname, tagpp.fname_end, (*st).tag_fname, buf_ffname) != 0;
        let is_static = test_for_static(tagpp);

        let mut mtt = if is_static {
            if is_current { MT_ST_CUR } else { MT_ST_OTH }
        } else if is_current {
            MT_GL_CUR
        } else {
            MT_GL_OTH
        };

        if (*(*st).orgpat).regmatch.rm_ic && !margs.match_no_ic {
            mtt += MT_IC_OFF;
        }
        if margs.match_re {
            mtt += MT_RE_OFF;
        }

        if (*st).help_only {
            *tagpp.tagname_end = NUL;
            let len = tagpp.tagname_end.offset_from(tagpp.tagname) as usize;
            let mfp_size = 1 + len + 10 + ML_EXTRA + 1;
            mfp = xmalloc(mfp_size).cast::<c_char>();

            let p = mfp;
            strcpy(p, tagpp.tagname);
            *p.add(len) = b'@' as c_char;
            strcpy(p.add(len + 1), (*st).help_lang.as_ptr());
            let heur = help_heuristic(
                tagpp.tagname,
                if margs.match_re { margs.matchoff } else { 0 },
                !margs.match_no_ic,
            ) + (*st).help_pri;
            vim_snprintf!(
                p.add(len + 1 + ML_EXTRA),
                mfp_size - (len + 1 + ML_EXTRA),
                c"%06d".as_ptr(),
                heur,
            );

            *tagpp.tagname_end = TAB;
        } else if name_only {
            if (*st).get_searchpat {
                let mut temp_end = tagpp.command;
                if *temp_end == b'/' as c_char {
                    while *temp_end != NUL
                        && *temp_end != b'\r' as c_char
                        && *temp_end != b'\n' as c_char
                        && *temp_end != b'$' as c_char
                    {
                        temp_end = temp_end.add(1);
                    }
                }

                if tagpp.command.add(2) < temp_end {
                    let len = temp_end.offset_from(tagpp.command.add(2)) as usize;
                    mfp = xmalloc(len + 2).cast::<c_char>();
                    xmemcpyz(mfp.cast(), tagpp.command.add(2).cast(), len);
                } else {
                    mfp = ptr::null_mut();
                }
                (*st).get_searchpat = false;
            } else {
                let len = tagpp.tagname_end.offset_from(tagpp.tagname) as usize;
                mfp = xmalloc(1 + len + 1).cast::<c_char>();
                xmemcpyz(mfp.cast(), tagpp.tagname.cast(), len);

                if (State & MODE_INSERT) != 0 {
                    (*st).get_searchpat = p_sft != 0;
                }
            }
        } else {
            let tag_fname_len = strlen((*st).tag_fname);
            let len = tag_fname_len + strlen((*st).lbuf) + 3;
            mfp = xmalloc(1 + len + 1).cast::<c_char>();
            let p = mfp;
            *p = (mtt + 1) as c_char;
            strcpy(p.add(1), (*st).tag_fname);
            *p.add(tag_fname_len + 1) = TAG_SEP;
            let s = p.add(1 + tag_fname_len + 1);
            strcpy(s, (*st).lbuf);
        }

        if !mfp.is_null() {
            *hash = hash_hash(mfp);
            let hi = hash_lookup(&raw mut (*st).ht_match[mtt], mfp, strlen(mfp), *hash);
            if hashitem_empty(&*hi) {
                hash_add_item(&raw mut (*st).ht_match[mtt], hi, mfp, *hash);
                let ga = &raw mut (*st).ga_match[mtt];
                ga_grow(ga, 1);
                *((*ga).ga_data.cast::<*mut c_char>()).add((*ga).ga_len as usize) = mfp;
                (*ga).ga_len += 1;
                (*st).match_count += 1;
            } else {
                xfree(mfp.cast());
            }
        }
    }
}

unsafe fn findtags_get_all_tags(
    st: *mut findtags_state_T,
    margs: &mut findtags_match_args_T,
    buf_ffname: *mut c_char,
) {
    unsafe {
        let mut tagp = tagptrs_T::default();
        let mut search_info = tagsearch_info_T::default();
        let mut hash: hash_T = 0;

        loop {
            if (*st).state == tagsearch_state_T::TS_BINARY
                || (*st).state == tagsearch_state_T::TS_SKIP_BACK
            {
                line_breakcheck();
            } else {
                fast_breakcheck();
            }
            if ((*st).flags & TAG_INS_COMP) != 0 {
                ins_compl_check_keys(30, false);
            }
            if got_int || ins_compl_interrupted() {
                (*st).stop_searching = true;
                break;
            }
            if (*st).mincount == TAG_MANY && (*st).match_count >= TAG_MANY {
                (*st).stop_searching = true;
                break;
            }
            if !(*st).get_searchpat {
                let retval = findtags_get_next_line(st, &raw mut search_info);
                if retval == tags_read_status_T::TAGS_READ_IGNORE {
                    continue;
                }
                if retval == tags_read_status_T::TAGS_READ_EOF {
                    break;
                }
            }

            if (*st).vimconv.vc_type != CONV_NONE {
                findtags_string_convert(st);
            }

            if (*st).state == tagsearch_state_T::TS_START
                && !findtags_start_state_handler(st, &raw mut margs.sortic, &raw mut search_info)
            {
                continue;
            }

            if *(*st).lbuf.add(((*st).lbuf_size - 2) as usize) != NUL {
                (*st).lbuf_size *= 2;
                xfree((*st).lbuf.cast());
                (*st).lbuf = xmalloc((*st).lbuf_size as usize).cast::<c_char>();

                if (*st).state == tagsearch_state_T::TS_STEP_FORWARD
                    || (*st).state == tagsearch_state_T::TS_LINEAR
                {
                    vim_ignored = vim_fseek((*st).fp, search_info.curr_offset, SEEK_SET);
                }
                search_info.curr_offset = 0;
                continue;
            }

            let retval = findtags_parse_line(st, &mut tagp, margs, &raw mut search_info);
            if retval == tagmatch_status_T::TAG_MATCH_NEXT {
                continue;
            }
            if retval == tagmatch_status_T::TAG_MATCH_STOP {
                break;
            }
            if retval == tagmatch_status_T::TAG_MATCH_FAIL {
                semsg!(
                    gettext(c"E431: Format error in tags file \"%s\"".as_ptr()),
                    (*st).tag_fname,
                );
                semsg!(
                    gettext(c"Before byte %lld".as_ptr()),
                    vim_ftell((*st).fp) as i64,
                );
                (*st).stop_searching = true;
                return;
            }

            if findtags_match_tag(st, &mut tagp, margs) {
                findtags_add_match(st, &mut tagp, margs, buf_ffname, &raw mut hash);
            }
        }
    }
}

unsafe fn findtags_in_file(st: *mut findtags_state_T, _flags: c_int, buf_ffname: *mut c_char) {
    unsafe {
        let mut margs = findtags_match_args_T::default();

        (*st).vimconv.vc_type = CONV_NONE;
        (*st).tag_file_sorted = NUL as c_int;
        (*st).fp = ptr::null_mut();
        findtags_matchargs_init(&mut margs, (*st).flags);

        if (*curbuf).b_help && !findtags_in_help_init(st) {
            return;
        }

        (*st).fp = os_fopen((*st).tag_fname, c"r".as_ptr());
        if (*st).fp.is_null() {
            return;
        }

        if p_verbose >= 5 {
            verbose_enter();
            smsg!(
                0,
                gettext(c"Searching tags file %s".as_ptr()),
                (*st).tag_fname,
            );
            verbose_leave();
        }
        (*st).did_open = true;
        (*st).state = tagsearch_state_T::TS_START;

        findtags_get_all_tags(st, &mut margs, buf_ffname);

        if !(*st).fp.is_null() {
            fclose((*st).fp);
            (*st).fp = ptr::null_mut();
        }
        if (*st).vimconv.vc_type != CONV_NONE {
            convert_setup(&raw mut (*st).vimconv, ptr::null_mut(), ptr::null_mut());
        }

        if margs.sort_error {
            semsg!(
                gettext(c"E432: Tags file not sorted: %s".as_ptr()),
                (*st).tag_fname,
            );
        }

        if (*st).match_count >= (*st).mincount {
            (*st).stop_searching = true;
        }
    }
}

unsafe fn findtags_copy_matches(
    st: *mut findtags_state_T,
    matchesp: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let name_only = ((*st).flags & TAG_NAMES) != 0;

        let matches = if (*st).match_count > 0 {
            xmalloc(((*st).match_count as usize) * size_of::<*mut c_char>()).cast::<*mut c_char>()
        } else {
            ptr::null_mut()
        };

        (*st).match_count = 0;
        for mtt in 0..MT_COUNT {
            let ga = &raw mut (*st).ga_match[mtt];
            for i in 0..(*ga).ga_len as usize {
                let mfp = *((*ga).ga_data.cast::<*mut c_char>()).add(i);
                if matches.is_null() {
                    xfree(mfp.cast());
                } else {
                    if !name_only {
                        *mfp -= 1;
                        let mut p = mfp.add(1);
                        while *p != NUL {
                            if *p == TAG_SEP {
                                *p = NUL;
                            }
                            p = p.add(1);
                        }
                    }
                    *matches.add((*st).match_count as usize) = mfp;
                    (*st).match_count += 1;
                }
            }

            ga_clear(ga);
            hash_clear(&raw mut (*st).ht_match[mtt]);
        }

        *matchesp = matches;
        (*st).match_count
    }
}

/// `find_tags()` - search for tags in tags files.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_tags(
    pat: *mut c_char,
    num_matches: *mut c_int,
    matchesp: *mut *mut *mut c_char,
    flags: c_int,
    mincount: c_int,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let mut st: findtags_state_T = std::mem::zeroed();
        let mut tn = tagname_T {
            tn_tags: ptr::null_mut(),
            tn_np: ptr::null_mut(),
            tn_did_filefind_init: 0,
            tn_hf_idx: 0,
            tn_search_ctx: ptr::null_mut(),
        };
        let mut retval: c_int;
        let mut saved_pat: *mut c_char = ptr::null_mut();

        let findall = mincount == MAXCOL || mincount == TAG_MANY;
        let has_re = (flags & TAG_REGEXP) != 0;
        let noic = (flags & TAG_NOIC) != 0;
        let verbose = (flags & TAG_VERBOSE) != 0;
        let save_p_ic = p_ic;

        match if (*curbuf).b_tc_flags != 0 {
            (*curbuf).b_tc_flags
        } else {
            tc_flags
        } {
            kOptTcFlagFollowic => {}
            kOptTcFlagIgnore => p_ic = 1,
            kOptTcFlagMatch => p_ic = 0,
            kOptTcFlagFollowscs => p_ic = ignorecase(pat),
            kOptTcFlagSmart => p_ic = ignorecase_opt(pat, 1, 1),
            _ => std::process::abort(),
        }

        let help_save = (*curbuf).b_help;

        findtags_state_init(&raw mut st, pat, flags, mincount);

        if st.help_only {
            (*curbuf).b_help = true;
        }

        if (*curbuf).b_help
            && (*st.orgpat).len > 3
            && *pat.add(((*st.orgpat).len - 3) as usize) == b'@' as c_char
            && ascii_isalpha(*pat.add(((*st.orgpat).len - 2) as usize))
            && ascii_isalpha(*pat.add(((*st.orgpat).len - 1) as usize))
        {
            saved_pat = xstrnsave(pat, ((*st.orgpat).len - 3) as usize);
            st.help_lang_find = pat.add(((*st.orgpat).len - 2) as usize);
            (*st.orgpat).pat = saved_pat;
            (*st.orgpat).len -= 3;
        }
        if p_tl != 0 && ((*st.orgpat).len as OptInt) > p_tl {
            (*st.orgpat).len = p_tl as c_int;
        }

        let save_emsg_off = emsg_off;
        emsg_off = 1;
        prepare_pats(st.orgpat, has_re);
        emsg_off = save_emsg_off;
        if has_re && (*st.orgpat).regmatch.regprog.is_null() {
            retval = FAIL;
        } else {
            retval = findtags_apply_tfu(&raw mut st, pat, buf_ffname);
            if retval == NOTDONE {
                retval = FAIL;

                if (flags & TAG_KEEP_LANG) != 0
                    && st.help_lang_find.is_null()
                    && !(*curbuf).b_fname.is_null()
                {
                    let i = strlen((*curbuf).b_fname) as c_int;
                    if i > 4
                        && mb_strnicmp(
                            (*curbuf).b_fname.add((i - 4) as usize),
                            c".txt".as_ptr(),
                            MAXCOL as usize,
                        ) == 0
                    {
                        st.is_txt = true;
                    }
                }

                (*st.orgpat).regmatch.rm_ic =
                    (p_ic != 0 || !noic) && (findall || (*st.orgpat).headlen == 0 || p_tbs == 0);
                for round in 1..=2 {
                    st.linear = (*st.orgpat).headlen == 0 || p_tbs == 0 || round == 2;

                    let mut first_file: c_int = 1;
                    while get_tagfname(&raw mut tn, first_file, st.tag_fname) == OK {
                        findtags_in_file(&raw mut st, flags, buf_ffname);
                        if st.stop_searching {
                            retval = OK;
                            break;
                        }
                        first_file = 0;
                    }

                    tagname_free(&raw mut tn);

                    if st.stop_searching
                        || st.linear
                        || (p_ic == 0 && noic)
                        || (*st.orgpat).regmatch.rm_ic
                    {
                        break;
                    }

                    (*st.orgpat).regmatch.rm_ic = true;
                }

                if !st.stop_searching {
                    if !st.did_open && verbose {
                        emsg(gettext(c"E433: No tags file".as_ptr()));
                    }
                    retval = OK;
                }
            }
        }

        findtags_state_free(&raw mut st);

        if retval == FAIL {
            st.match_count = 0;
        }

        *num_matches = findtags_copy_matches(&raw mut st, matchesp);

        (*curbuf).b_help = help_save;
        xfree(saved_pat.cast());

        p_ic = save_p_ic;

        retval
    }
}

unsafe extern "C" fn found_tagfile_cb(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    _cookie: *mut c_void,
) -> bool {
    unsafe {
        for i in 0..num_fnames as usize {
            let tag_fname = xstrdup(*fnames.add(i));
            simplify_filename(tag_fname);
            ga_grow(&raw mut tag_fnames, 1);
            *(tag_fnames.ga_data.cast::<*mut c_char>()).add(tag_fnames.ga_len as usize) = tag_fname;
            tag_fnames.ga_len += 1;

            if !all {
                break;
            }
        }

        num_fnames > 0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_tag_stuff() {
    unsafe {
        ga_clear_strings(&raw mut tag_fnames);
        if !curwin.is_null() {
            do_tag(ptr::null_mut(), ptr::null_mut(), DT_FREE, 0, 0, false);
        }
        tag_freematch();
        tagstack_clear_entry(&raw mut ptag_entry);
    }
}

/// Get the next name of a tag file from the tag file list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_tagfname(
    tnp: *mut tagname_T,
    first: c_int,
    buf: *mut c_char,
) -> c_int {
    unsafe {
        let mut fname: *mut c_char;

        if first != 0 {
            *tnp = tagname_T {
                tn_tags: ptr::null_mut(),
                tn_np: ptr::null_mut(),
                tn_did_filefind_init: 0,
                tn_hf_idx: 0,
                tn_search_ctx: ptr::null_mut(),
            };
        }

        if (*curbuf).b_help {
            if first != 0 {
                ga_clear_strings(&raw mut tag_fnames);
                ga_init(&raw mut tag_fnames, size_of::<*mut c_char>() as c_int, 10);
                do_in_runtimepath(
                    c"doc/tags doc/tags-??".as_ptr().cast_mut(),
                    DIP_ALL,
                    Some(found_tagfile_cb),
                    ptr::null_mut(),
                );
            }

            if (*tnp).tn_hf_idx >= tag_fnames.ga_len {
                if (*tnp).tn_hf_idx > tag_fnames.ga_len || *p_hf == NUL {
                    return FAIL;
                }
                (*tnp).tn_hf_idx += 1;
                xstrlcpy(buf, p_hf, MAXPATHL - 4);
                strcpy(path_tail(buf), c"tags".as_ptr());
                simplify_filename(buf);

                for i in 0..tag_fnames.ga_len as usize {
                    if strcmp(buf, *(tag_fnames.ga_data.cast::<*mut c_char>()).add(i)) == 0 {
                        return FAIL;
                    }
                }
            } else {
                xstrlcpy(
                    buf,
                    *(tag_fnames.ga_data.cast::<*mut c_char>()).add((*tnp).tn_hf_idx as usize),
                    MAXPATHL,
                );
                (*tnp).tn_hf_idx += 1;
            }
            return OK;
        }

        if first != 0 {
            (*tnp).tn_tags = xstrdup(if *(*curbuf).b_p_tags != NUL {
                (*curbuf).b_p_tags
            } else {
                p_tags
            });
            (*tnp).tn_np = (*tnp).tn_tags;
        }

        loop {
            if (*tnp).tn_did_filefind_init != 0 {
                fname = vim_findfile((*tnp).tn_search_ctx);
                if !fname.is_null() {
                    break;
                }
                (*tnp).tn_did_filefind_init = 0;
            } else {
                if *(*tnp).tn_np == NUL {
                    vim_findfile_cleanup((*tnp).tn_search_ctx);
                    (*tnp).tn_search_ctx = ptr::null_mut();
                    return FAIL;
                }

                *buf = NUL;
                copy_option_part(&raw mut (*tnp).tn_np, buf, MAXPATHL - 1, c" ,".as_ptr());

                let mut r_ptr = vim_findfile_stopdir(buf);
                let mut filename = path_tail(buf);
                if !r_ptr.is_null() {
                    ptr::copy(r_ptr, r_ptr.add(1), strlen(r_ptr) + 1);
                    r_ptr = r_ptr.add(1);
                }
                ptr::copy(filename, filename.add(1), strlen(filename) + 1);
                *filename = NUL;
                filename = filename.add(1);

                (*tnp).tn_search_ctx = vim_findfile_init(
                    buf,
                    filename,
                    strlen(filename),
                    r_ptr,
                    100,
                    0,
                    FINDFILE_FILE,
                    (*tnp).tn_search_ctx,
                    1,
                    (*curbuf).b_ffname,
                );
                if !(*tnp).tn_search_ctx.is_null() {
                    (*tnp).tn_did_filefind_init = 1;
                }
            }
        }

        strcpy(buf, fname);
        xfree(fname.cast());
        OK
    }
}

/// Free the contents of a `tagname_T` that was filled by `get_tagfname()`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tagname_free(tnp: *mut tagname_T) {
    unsafe {
        xfree((*tnp).tn_tags.cast());
        vim_findfile_cleanup((*tnp).tn_search_ctx);
        (*tnp).tn_search_ctx = ptr::null_mut();
        ga_clear_strings(&raw mut tag_fnames);
    }
}

unsafe fn parse_tag_line(lbuf: *mut c_char, tagp: &mut tagptrs_T) -> c_int {
    unsafe {
        tagp.tagname = lbuf;
        let mut p = vim_strchr(lbuf, TAB as c_int);
        if p.is_null() {
            return FAIL;
        }
        tagp.tagname_end = p;

        if *p != NUL {
            p = p.add(1);
        }
        tagp.fname = p;
        p = vim_strchr(p, TAB as c_int);
        if p.is_null() {
            return FAIL;
        }
        tagp.fname_end = p;

        if *p != NUL {
            p = p.add(1);
        }
        if *p == NUL {
            return FAIL;
        }
        tagp.command = p;

        OK
    }
}

unsafe fn test_for_static(tagp: &tagptrs_T) -> bool {
    unsafe {
        let mut p = tagp.command;
        loop {
            p = vim_strchr(p, b'\t' as c_int);
            if p.is_null() {
                break;
            }
            p = p.add(1);
            if strncmp(p, c"file:".as_ptr(), 5) == 0 {
                return true;
            }
        }
        false
    }
}

unsafe fn matching_line_len(lbuf: *const c_char) -> usize {
    unsafe {
        let mut p = lbuf.add(1);
        p = p.add(strlen(p) + 1);
        p.offset_from(lbuf) as usize + strlen(p)
    }
}

unsafe fn parse_match(mut lbuf: *mut c_char, tagp: &mut tagptrs_T) -> c_int {
    unsafe {
        tagp.tag_fname = lbuf.add(1);
        lbuf = lbuf.add(strlen(tagp.tag_fname) + 2);

        let retval = parse_tag_line(lbuf, tagp);

        tagp.tagkind = ptr::null_mut();
        tagp.user_data = ptr::null_mut();
        tagp.tagline = 0;
        tagp.command_end = ptr::null_mut();

        if retval != OK {
            return retval;
        }

        let mut p = tagp.command;
        if find_extra(&raw mut p) == OK {
            tagp.command_end = p;
            if p > tagp.command && *p.sub(1) == b'|' as c_char {
                tagp.command_end = p.sub(1);
            }
            p = p.add(2);
            if *p == TAB {
                p = p.add(1);
                while ascii_isalpha(*p) || utfc_ptr2len(p) > 1 {
                    if strncmp(p, c"kind:".as_ptr(), 5) == 0 {
                        tagp.tagkind = p.add(5);
                    } else if strncmp(p, c"user_data:".as_ptr(), 10) == 0 {
                        tagp.user_data = p.add(10);
                    } else if strncmp(p, c"line:".as_ptr(), 5) == 0 {
                        tagp.tagline = atoi(p.add(5));
                    }
                    if !tagp.tagkind.is_null() && !tagp.user_data.is_null() {
                        break;
                    }

                    let pc = vim_strchr(p, b':' as c_int);
                    let pt = vim_strchr(p, b'\t' as c_int);
                    if pc.is_null() || (!pt.is_null() && pc > pt) {
                        tagp.tagkind = p;
                    }
                    if pt.is_null() {
                        break;
                    }
                    p = pt;
                    mb_ptr_adv(&mut p);
                }
            }
        }
        if !tagp.tagkind.is_null() {
            p = tagp.tagkind;
            while *p != NUL
                && *p != b'\t' as c_char
                && *p != b'\r' as c_char
                && *p != b'\n' as c_char
            {
                mb_ptr_adv(&mut p);
            }
            tagp.tagkind_end = p;
        }
        if !tagp.user_data.is_null() {
            p = tagp.user_data;
            while *p != NUL
                && *p != b'\t' as c_char
                && *p != b'\r' as c_char
                && *p != b'\n' as c_char
            {
                mb_ptr_adv(&mut p);
            }
            tagp.user_data_end = p;
        }
        retval
    }
}

unsafe fn tag_full_fname(tagp: &mut tagptrs_T) -> *mut c_char {
    unsafe {
        let c = *tagp.fname_end;
        *tagp.fname_end = NUL;
        let fullname = expand_tag_fname(tagp.fname, tagp.tag_fname, false);
        *tagp.fname_end = c;
        fullname
    }
}

unsafe fn jumpto_tag(lbuf_arg: *const c_char, forceit: c_int, keep_help: bool) -> c_int {
    unsafe {
        if postponed_split == 0 && !check_can_set_curbuf_forceit(forceit) {
            return FAIL;
        }

        let mut pbuf_end: *mut c_char;
        let mut tofree_fname: *mut c_char = ptr::null_mut();
        let mut tagp = tagptrs_T::default();
        let mut retval = FAIL;
        let mut getfile_result = GETFILE_UNUSED;
        let mut curwin_save: *mut win_T = ptr::null_mut();
        let mut full_fname: *mut c_char = ptr::null_mut();
        let old_KeyTyped = KeyTyped;
        let l_g_do_tagpreview = g_do_tagpreview;
        let len = matching_line_len(lbuf_arg) + 1;
        let lbuf = xmalloc(len).cast::<c_char>();
        ptr::copy(lbuf_arg, lbuf, len);

        let pbuf = xmalloc(LSIZE as usize).cast::<c_char>();

        'erret: {
            if parse_match(lbuf, &mut tagp) == FAIL {
                break 'erret;
            }

            *tagp.fname_end = NUL;
            let mut fname = tagp.fname;

            let mut str_ = tagp.command;
            pbuf_end = pbuf;
            while *str_ != NUL && *str_ != b'\n' as c_char && *str_ != b'\r' as c_char {
                *pbuf_end = *str_;
                pbuf_end = pbuf_end.add(1);
                str_ = str_.add(1);
                if pbuf_end.offset_from(pbuf) as c_int + 1 >= LSIZE {
                    break;
                }
            }
            *pbuf_end = NUL;

            {
                str_ = pbuf;
                if find_extra(&raw mut str_) == OK {
                    if str_ > pbuf && *str_.sub(1) == b'|' as c_char {
                        pbuf_end = str_.sub(1);
                    } else {
                        pbuf_end = str_;
                    }
                    *pbuf_end = NUL;
                }
            }

            fname = expand_tag_fname(fname, tagp.tag_fname, true);
            tofree_fname = fname;

            if !os_path_exists(fname) && !has_autocmd(EVENT_BUFREADCMD, fname, ptr::null_mut()) {
                retval = NOTAGFILE;
                xfree(nofile_fname.cast());
                nofile_fname = xstrdup(fname);
                break 'erret;
            }

            RedrawingDisabled += 1;

            if l_g_do_tagpreview != 0 {
                postponed_split = 0;
                curwin_save = curwin;

                if (*curwin).w_onebuf_opt.wo_pvw == 0 {
                    full_fname = FullName_save(fname, false);
                    fname = full_fname;

                    prepare_tagpreview(true, *p_pvp != NUL);
                }
            }

            if postponed_split != 0 && (swb_flags & (kOptSwbFlagUseopen | kOptSwbFlagUsetab)) != 0 {
                let existing_buf = buflist_findname_exp(fname);
                if !existing_buf.is_null() && !swbuf_goto_win_with_buf(existing_buf).is_null() {
                    getfile_result = GETFILE_SAME_FILE;
                }
            }
            if getfile_result == GETFILE_UNUSED && (postponed_split != 0 || cmdmod.cmod_tab != 0) {
                if (swb_flags & kOptSwbFlagVsplit) != 0 {
                    cmdmod.cmod_split |= WSP_VERT;
                }
                if (swb_flags & kOptSwbFlagNewtab) != 0 && cmdmod.cmod_tab == 0 {
                    cmdmod.cmod_tab = tabpage_index(curtab) + 1;
                }
                if win_split(
                    if postponed_split > 0 {
                        postponed_split
                    } else {
                        0
                    },
                    postponed_split_flags,
                ) == FAIL
                {
                    RedrawingDisabled -= 1;
                    break 'erret;
                }
                (*curwin).w_onebuf_opt.wo_scb = 0;
                (*curwin).w_onebuf_opt.wo_crb = 0;
            }

            if keep_help {
                if l_g_do_tagpreview != 0 {
                    keep_help_flag = bt_help((*curwin_save).w_buffer);
                } else {
                    keep_help_flag = (*curbuf).b_help;
                }
            }

            if getfile_result == GETFILE_UNUSED {
                getfile_result = getfile(0, fname, ptr::null_mut(), true, 0, forceit != 0);
            }
            keep_help_flag = false;

            if getfile_result <= 0 {
                (*curwin).w_set_curswant = true;
                postponed_split = 0;

                let save_no_hlsearch = Search.no_hlsearch;

                let search_options = if !vim_strchr(p_cpo, kCpoTagpat).is_null() {
                    0
                } else {
                    SEARCH_KEEP
                };

                str_ = pbuf;
                if *pbuf == b'/' as c_char || *pbuf == b'?' as c_char {
                    str_ = skip_regexp(pbuf.add(1), *pbuf as c_int, 0).add(1);
                }
                if str_ > pbuf_end.sub(1) {
                    let mut pbuflen = pbuf_end.offset_from(pbuf) as usize;

                    let save_p_ws = p_ws;
                    let save_p_ic = p_ic;
                    let save_p_scs = p_scs;
                    p_ws = 1;
                    p_ic = 0;
                    p_scs = 0;
                    let save_lnum = (*curwin).w_cursor.lnum;

                    (*curwin).w_cursor.lnum = if tagp.tagline > 0 {
                        tagp.tagline - 1
                    } else {
                        0
                    };

                    if do_search(
                        ptr::null_mut(),
                        *pbuf as c_int,
                        *pbuf as c_int,
                        pbuf.add(1),
                        pbuflen - 1,
                        1,
                        search_options,
                        false,
                        ptr::null_mut(),
                    ) != 0
                    {
                        retval = OK;
                    } else {
                        let mut found = 1;
                        p_ic = 1;
                        if do_search(
                            ptr::null_mut(),
                            *pbuf as c_int,
                            *pbuf as c_int,
                            pbuf.add(1),
                            pbuflen - 1,
                            1,
                            search_options,
                            false,
                            ptr::null_mut(),
                        ) == 0
                        {
                            found = 2;
                            test_for_static(&tagp);
                            let cc = *tagp.tagname_end;
                            *tagp.tagname_end = NUL;
                            pbuflen = vim_snprintf!(
                                pbuf,
                                LSIZE as usize,
                                c"^%s\\s\\*(".as_ptr(),
                                tagp.tagname,
                            ) as usize;
                            if do_search(
                                ptr::null_mut(),
                                b'/' as c_int,
                                b'/' as c_int,
                                pbuf,
                                pbuflen,
                                1,
                                search_options,
                                false,
                                ptr::null_mut(),
                            ) == 0
                            {
                                pbuflen = vim_snprintf!(
                                    pbuf,
                                    LSIZE as usize,
                                    c"^\\[#a-zA-Z_]\\.\\*\\<%s\\s\\*(".as_ptr(),
                                    tagp.tagname,
                                ) as usize;
                                if do_search(
                                    ptr::null_mut(),
                                    b'/' as c_int,
                                    b'/' as c_int,
                                    pbuf,
                                    pbuflen,
                                    1,
                                    search_options,
                                    false,
                                    ptr::null_mut(),
                                ) == 0
                                {
                                    found = 0;
                                }
                            }
                            *tagp.tagname_end = cc;
                        }
                        if found == 0 {
                            emsg(gettext(c"E434: Can't find tag pattern".as_ptr()));
                            (*curwin).w_cursor.lnum = save_lnum;
                        } else {
                            if found == 2 || save_p_ic == 0 {
                                msg(
                                    gettext(c"E435: Couldn't find tag, just guessing!".as_ptr()),
                                    0,
                                );
                                if msg_scrolled == 0 && msg_silent == 0 {
                                    msg_delay(1010, true);
                                }
                            }
                            retval = OK;
                        }
                    }
                    p_ws = save_p_ws;
                    p_ic = save_p_ic;
                    p_scs = save_p_scs;

                    check_cursor(curwin);
                } else {
                    let save_secure = secure;

                    secure = 1;
                    sandbox += 1;
                    (*curwin).w_cursor.lnum = 1;
                    (*curwin).w_cursor.col = 0;
                    (*curwin).w_cursor.coladd = 0;
                    do_cmdline_cmd(pbuf);
                    retval = OK;

                    if secure == 2 {
                        wait_return(1);
                    }
                    secure = save_secure;
                    sandbox -= 1;
                }

                if search_options != 0 {
                    set_no_hlsearch(save_no_hlsearch);
                }

                if getfile_result == GETFILE_OPEN_OTHER {
                    retval = OK;
                }

                if retval == OK {
                    if (*curbuf).b_help {
                        set_topline(curwin, (*curwin).w_cursor.lnum);
                    }
                    if (fdo_flags & kOptFdoFlagTag) != 0 && old_KeyTyped {
                        foldOpenCursor();
                    }
                }

                if l_g_do_tagpreview != 0 && curwin != curwin_save && win_valid(curwin_save) {
                    validate_cursor(curwin);
                    redraw_later(curwin, UPD_VALID);
                    win_enter(curwin_save, true);
                }

                RedrawingDisabled -= 1;
            } else {
                RedrawingDisabled -= 1;
                if postponed_split != 0 {
                    win_close(curwin, false, false);
                    postponed_split = 0;
                }
            }
        }

        g_do_tagpreview = 0;
        xfree(lbuf.cast());
        xfree(pbuf.cast());
        xfree(tofree_fname.cast());
        xfree(full_fname.cast());

        retval
    }
}

unsafe fn expand_tag_fname(
    fname_arg: *mut c_char,
    tag_fname: *mut c_char,
    expand: bool,
) -> *mut c_char {
    unsafe {
        let mut xpc: expand_T = std::mem::zeroed();

        let mut fname = to_slash_save(fname_arg);

        if expand && path_has_wildcard(fname, true) && vim_strchr(fname, b'`' as c_int).is_null() {
            ExpandInit(&raw mut xpc);
            xpc.xp_context = EXPAND_FILES;
            let expanded_fname = ExpandOne(
                &raw mut xpc,
                fname,
                ptr::null_mut(),
                WILD_LIST_NOTFOUND | WILD_SILENT,
                WILD_EXPAND_FREE,
            );
            if !expanded_fname.is_null() {
                xfree(fname.cast());
                fname = expanded_fname;
            }
        }

        let retval: *mut c_char;
        if (p_tr != 0 || (*curbuf).b_help) && !vim_isAbsName(fname) {
            let p = path_tail(tag_fname);
            if p != tag_fname {
                retval = xmalloc(MAXPATHL).cast::<c_char>();
                strcpy(retval, tag_fname);
                let off = p.offset_from(tag_fname) as usize;
                xstrlcpy(retval.add(off), fname, MAXPATHL - off);
                simplify_filename(retval);
            } else {
                retval = xstrdup(fname);
            }
        } else {
            retval = xstrdup(fname);
        }

        xfree(fname.cast());
        retval
    }
}

unsafe fn test_for_current(
    fname: *mut c_char,
    fname_end: *mut c_char,
    tag_fname: *mut c_char,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let mut retval = false;
        if !buf_ffname.is_null() {
            let c = *fname_end;
            *fname_end = NUL;
            let fullname = expand_tag_fname(fname, tag_fname, true);
            retval = path_equal(
                fullname,
                buf_ffname,
                (kPathCmpExpand | kPathCmpFull) as PathCmpFlags,
            );
            xfree(fullname.cast());
            *fname_end = c;
        }
        c_int::from(retval)
    }
}

unsafe fn find_extra(pp: *mut *mut c_char) -> c_int {
    unsafe {
        let mut str_ = *pp;
        let mut first_char = **pp;

        loop {
            if ascii_isdigit(*str_) {
                str_ = skipdigits(str_.add(1));
            } else if *str_ == b'/' as c_char || *str_ == b'?' as c_char {
                str_ = skip_regexp(str_.add(1), *str_ as c_int, 0);
                if *str_ != first_char {
                    str_ = ptr::null_mut();
                } else {
                    str_ = str_.add(1);
                }
            } else {
                str_ = strstr(str_, c"|;\"".as_ptr());
                if !str_.is_null() {
                    str_ = str_.add(1);
                    break;
                }
            }
            if str_.is_null()
                || *str_ != b';' as c_char
                || !(ascii_isdigit(*str_.add(1))
                    || *str_.add(1) == b'/' as c_char
                    || *str_.add(1) == b'?' as c_char)
            {
                break;
            }
            str_ = str_.add(1);
            first_char = *str_;
        }

        if !str_.is_null() && strncmp(str_, c";\"".as_ptr(), 2) == 0 {
            *pp = str_;
            return OK;
        }
        FAIL
    }
}

/// Free a single entry in a tag stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tagstack_clear_entry(item: *mut taggy_T) {
    unsafe {
        xfree((*item).tagname.cast());
        (*item).tagname = ptr::null_mut();
        xfree((*item).user_data.cast());
        (*item).user_data = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn expand_tags(
    tagnames: bool,
    pat: *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut name_buf_size: usize = 100;
        let mut name_buf = xmalloc(name_buf_size).cast::<c_char>();

        let extra_flag = if tagnames { TAG_NAMES } else { 0 };
        let ret = if *pat == b'/' as c_char {
            find_tags(
                pat.add(1),
                num_file,
                file,
                TAG_REGEXP | extra_flag | TAG_VERBOSE,
                TAG_MANY,
                (*curbuf).b_ffname,
            )
        } else {
            find_tags(
                pat,
                num_file,
                file,
                TAG_REGEXP | extra_flag | TAG_VERBOSE | TAG_NOIC,
                TAG_MANY,
                (*curbuf).b_ffname,
            )
        };
        if ret == OK && !tagnames {
            for i in 0..(*num_file) as usize {
                let mut t_p = tagptrs_T::default();
                let item_ptr = *(*file).add(i);
                parse_match(item_ptr, &mut t_p);
                let mut len = t_p.tagname_end.offset_from(t_p.tagname) as usize;
                if len > name_buf_size - 3 {
                    name_buf_size = len + 3;
                    name_buf = xrealloc(name_buf.cast(), name_buf_size).cast::<c_char>();
                }

                ptr::copy(t_p.tagname, name_buf, len);
                *name_buf.add(len) = 0;
                len += 1;
                *name_buf.add(len) = if !t_p.tagkind.is_null() && *t_p.tagkind != NUL {
                    *t_p.tagkind
                } else {
                    b'f' as c_char
                };
                len += 1;
                *name_buf.add(len) = 0;
                len += 1;
                let fname_len = t_p.fname_end.offset_from(t_p.fname) as usize;
                ptr::copy(t_p.fname, item_ptr.add(len), fname_len);
                *item_ptr.add(len + fname_len) = 0;
                ptr::copy(name_buf, item_ptr, len);
            }
        }
        xfree(name_buf.cast());
        ret
    }
}

unsafe fn add_tag_field(
    dict: *mut dict_T,
    field_name: *const c_char,
    start: *const c_char,
    mut end: *const c_char,
) -> c_int {
    unsafe {
        if !tv_dict_find(dict, field_name, -1).is_null() {
            if p_verbose > 0 {
                verbose_enter();
                smsg!(0, gettext(c"Duplicate field name: %s".as_ptr()), field_name);
                verbose_leave();
            }
            return FAIL;
        }
        let mut len: c_int = 0;
        let buf = xmalloc(MAXPATHL).cast::<c_char>();
        if !start.is_null() {
            if end.is_null() {
                end = start.add(strlen(start));
                while end > start
                    && (*end.sub(1) == b'\r' as c_char || *end.sub(1) == b'\n' as c_char)
                {
                    end = end.sub(1);
                }
            }
            len = (end.offset_from(start) as c_int).min(MAXPATHL as c_int - 1);
            xmemcpyz(buf.cast(), start.cast(), len as usize);
        }
        *buf.add(len as usize) = NUL;
        let retval = tv_dict_add_str(dict, field_name, strlen(field_name), buf);
        xfree(buf.cast());
        retval
    }
}

/// Add the tags matching the specified pattern `pat` to the list `list`
/// as a dictionary. Use `buf_fname` for priority, unless NULL.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_tags(
    list: *mut list_T,
    pat: *mut c_char,
    buf_fname: *mut c_char,
) -> c_int {
    unsafe {
        let mut num_matches: c_int = 0;
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let mut tp = tagptrs_T::default();

        let mut ret = find_tags(
            pat,
            &raw mut num_matches,
            &raw mut matches,
            TAG_REGEXP | TAG_NOIC,
            MAXCOL,
            buf_fname,
        );
        if ret != OK || num_matches <= 0 {
            return ret;
        }

        for i in 0..num_matches as usize {
            let match_i = *matches.add(i);
            if parse_match(match_i, &mut tp) == FAIL {
                xfree(match_i.cast());
                continue;
            }

            let is_static = test_for_static(&tp);

            if strncmp(tp.tagname, c"!_TAG_".as_ptr(), 6) == 0 {
                xfree(match_i.cast());
                continue;
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(list, dict);

            let full_fname = tag_full_fname(&mut tp);
            if add_tag_field(dict, c"name".as_ptr(), tp.tagname, tp.tagname_end) == FAIL
                || add_tag_field(dict, c"filename".as_ptr(), full_fname, ptr::null()) == FAIL
                || add_tag_field(dict, c"cmd".as_ptr(), tp.command, tp.command_end) == FAIL
                || add_tag_field(
                    dict,
                    c"kind".as_ptr(),
                    tp.tagkind,
                    if !tp.tagkind.is_null() {
                        tp.tagkind_end.cast_const()
                    } else {
                        ptr::null()
                    },
                ) == FAIL
                || tv_dict_add_nr(dict, c"static".as_ptr(), 6, varnumber_T::from(is_static)) == FAIL
            {
                ret = FAIL;
            }

            xfree(full_fname.cast());

            if !tp.command_end.is_null() {
                let mut p = tp.command_end.add(3);
                while *p != NUL && *p != b'\n' as c_char && *p != b'\r' as c_char {
                    if p == tp.tagkind
                        || (p.add(5) == tp.tagkind && strncmp(p, c"kind:".as_ptr(), 5) == 0)
                    {
                        p = tp.tagkind_end.sub(1);
                    } else if strncmp(p, c"file:".as_ptr(), 5) == 0 {
                        p = p.add(4);
                    } else if !ascii_iswhite(*p) {
                        let n = p;
                        while *p != NUL
                            && (*p as u8) >= b' '
                            && (*p as u8) < 127
                            && *p != b':' as c_char
                        {
                            p = p.add(1);
                        }
                        let len = p.offset_from(n) as c_int;
                        if *p == b':' as c_char && len > 0 {
                            p = p.add(1);
                            let s = p;
                            while *p != NUL && (*p as u8) >= b' ' {
                                p = p.add(1);
                            }
                            *n.add(len as usize) = NUL;
                            if add_tag_field(dict, n, s, p) == FAIL {
                                ret = FAIL;
                            }
                            *n.add(len as usize) = b':' as c_char;
                        } else {
                            while *p != NUL && (*p as u8) >= b' ' {
                                p = p.add(1);
                            }
                        }
                        if *p == NUL {
                            break;
                        }
                    }
                    mb_ptr_adv(&mut p);
                }
            }

            xfree(match_i.cast());
        }
        xfree(matches.cast());
        ret
    }
}

unsafe fn get_tag_details(tag: *mut taggy_T, retdict: *mut dict_T) {
    unsafe {
        tv_dict_add_str(retdict, c"tagname".as_ptr(), 7, (*tag).tagname);
        tv_dict_add_nr(
            retdict,
            c"matchnr".as_ptr(),
            7,
            ((*tag).cur_match + 1) as varnumber_T,
        );
        tv_dict_add_nr(
            retdict,
            c"bufnr".as_ptr(),
            5,
            (*tag).cur_fnum as varnumber_T,
        );
        if !(*tag).user_data.is_null() {
            tv_dict_add_str(retdict, c"user_data".as_ptr(), 9, (*tag).user_data);
        }

        let pos = tv_list_alloc(4);
        tv_dict_add_list(retdict, c"from".as_ptr(), 4, pos);

        let fmark = &raw mut (*tag).fmark;
        tv_list_append_number(
            pos,
            if (*fmark).fnum != -1 {
                (*fmark).fnum as varnumber_T
            } else {
                0
            },
        );
        tv_list_append_number(pos, (*fmark).mark.lnum as varnumber_T);
        tv_list_append_number(
            pos,
            if (*fmark).mark.col == MAXCOL {
                MAXCOL as varnumber_T
            } else {
                ((*fmark).mark.col + 1) as varnumber_T
            },
        );
        tv_list_append_number(pos, (*fmark).mark.coladd as varnumber_T);
    }
}

/// Return the tag stack entries of the specified window `wp` in dictionary `retdict`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_tagstack(wp: *mut win_T, retdict: *mut dict_T) {
    unsafe {
        tv_dict_add_nr(
            retdict,
            c"length".as_ptr(),
            6,
            (*wp).w_tagstacklen as varnumber_T,
        );
        tv_dict_add_nr(
            retdict,
            c"curidx".as_ptr(),
            6,
            ((*wp).w_tagstackidx + 1) as varnumber_T,
        );
        let l = tv_list_alloc(2);
        tv_dict_add_list(retdict, c"items".as_ptr(), 5, l);

        let tagstack = (&raw mut (*wp).w_tagstack).cast::<taggy_T>();
        for i in 0..(*wp).w_tagstacklen as usize {
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            get_tag_details(tagstack.add(i), d);
        }
    }
}

unsafe fn tagstack_clear(wp: *mut win_T) {
    unsafe {
        let tagstack = (&raw mut (*wp).w_tagstack).cast::<taggy_T>();
        for i in 0..(*wp).w_tagstacklen as usize {
            tagstack_clear_entry(tagstack.add(i));
        }
        (*wp).w_tagstacklen = 0;
        (*wp).w_tagstackidx = 0;
    }
}

unsafe fn tagstack_shift(wp: *mut win_T) {
    unsafe {
        let tagstack = (&raw mut (*wp).w_tagstack).cast::<taggy_T>();
        tagstack_clear_entry(tagstack);
        for i in 1..(*wp).w_tagstacklen as usize {
            *tagstack.add(i - 1) = *tagstack.add(i);
        }
        (*wp).w_tagstacklen -= 1;
    }
}

unsafe fn tagstack_push_item(
    wp: *mut win_T,
    tagname: *mut c_char,
    cur_fnum: c_int,
    cur_match: c_int,
    mark: pos_T,
    fnum: c_int,
    user_data: *mut c_char,
) {
    unsafe {
        let tagstack = (&raw mut (*wp).w_tagstack).cast::<taggy_T>();
        let mut idx = (*wp).w_tagstacklen;

        if idx >= TAGSTACKSIZE {
            tagstack_shift(wp);
            idx = TAGSTACKSIZE - 1;
        }

        (*wp).w_tagstacklen += 1;
        let entry = tagstack.add(idx as usize);
        (*entry).tagname = tagname;
        (*entry).cur_fnum = cur_fnum;
        (*entry).cur_match = cur_match.max(0);
        (*entry).fmark.mark = mark;
        (*entry).fmark.fnum = fnum;
        (*entry).fmark.view = fmarkv_T {
            topline_offset: MAXLNUM,
            skipcol: 0,
        };
        (*entry).user_data = user_data;
    }
}

unsafe fn tagstack_push_items(wp: *mut win_T, l: *mut list_T) {
    unsafe {
        let mut li = if l.is_null() {
            ptr::null_mut()
        } else {
            (*l).lv_first
        };
        while !li.is_null() {
            if (*li).li_tv.v_type == VarType::VAR_DICT && !(*li).li_tv.vval.v_dict.is_null() {
                let itemdict = (*li).li_tv.vval.v_dict;
                let di = tv_dict_find(itemdict, c"from".as_ptr(), -1);
                if !di.is_null() {
                    let mut mark = pos_T {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    };
                    let mut fnum: c_int = 0;
                    if list2fpos(
                        &raw mut (*di).di_tv,
                        &raw mut mark,
                        &raw mut fnum,
                        ptr::null_mut(),
                        false,
                    ) == OK
                    {
                        let tagname = tv_dict_get_string(itemdict, c"tagname".as_ptr(), true);
                        if !tagname.is_null() {
                            if mark.col > 0 {
                                mark.col -= 1;
                            }
                            tagstack_push_item(
                                wp,
                                tagname,
                                tv_dict_get_number(itemdict, c"bufnr".as_ptr()) as c_int,
                                tv_dict_get_number(itemdict, c"matchnr".as_ptr()) as c_int - 1,
                                mark,
                                fnum,
                                tv_dict_get_string(itemdict, c"user_data".as_ptr(), true),
                            );
                        }
                    }
                }
            }
            li = (*li).li_next;
        }
    }
}

unsafe fn tagstack_set_curidx(wp: *mut win_T, curidx: c_int) {
    unsafe {
        (*wp).w_tagstackidx = curidx.max(0).min((*wp).w_tagstacklen);
    }
}

/// Set the tag stack entries of the specified window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_tagstack(wp: *mut win_T, d: *const dict_T, action: c_int) -> c_int {
    unsafe {
        if tfu_in_use {
            emsg(gettext(E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC.as_ptr()));
            return FAIL;
        }

        let mut l: *mut list_T = ptr::null_mut();

        let di_items = tv_dict_find(d, c"items".as_ptr(), -1);
        if !di_items.is_null() {
            if (*di_items).di_tv.v_type != VarType::VAR_LIST {
                emsg(gettext((&raw const e_listreq).cast()));
                return FAIL;
            }
            l = (*di_items).di_tv.vval.v_list;
        }

        let di_curidx = tv_dict_find(d, c"curidx".as_ptr(), -1);
        if !di_curidx.is_null() {
            tagstack_set_curidx(
                wp,
                tv_get_number(&raw const (*di_curidx).di_tv) as c_int - 1,
            );
        }

        if action == b't' as c_int {
            let tagstack = (&raw mut (*wp).w_tagstack).cast::<taggy_T>();
            let tagstackidx = (*wp).w_tagstackidx;
            let mut tagstacklen = (*wp).w_tagstacklen;

            while tagstackidx < tagstacklen {
                tagstacklen -= 1;
                tagstack_clear_entry(tagstack.add(tagstacklen as usize));
            }
            (*wp).w_tagstacklen = tagstacklen;
        }

        if !l.is_null() {
            if action == b'r' as c_int {
                tagstack_clear(wp);
            }

            tagstack_push_items(wp, l);
            (*wp).w_tagstackidx = (*wp).w_tagstacklen;
        }

        OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_strnicmp_compares_ascii_case_folded_to_uppercase() {
        unsafe {
            assert_eq!(tag_strnicmp(c"abc".as_ptr(), c"ABC".as_ptr(), 3), 0);
            assert!(tag_strnicmp(c"abc".as_ptr(), c"_bc".as_ptr(), 3) < 0);
        }
    }
}
