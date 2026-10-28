//! Code to handle tags and the tag stack (`src/nvim/tag.c`).

use crate::autocmd::{EVENT_BUFREADCMD, has_autocmd};
use crate::buffer::{
    GETF_SETMARK, GETFILE_OPEN_OTHER, GETFILE_SAME_FILE, GETFILE_UNUSED, MAXLNUM, NOTDONE,
    VAR_FIXED, bt_help, buflist_findname_exp, buflist_findnr, buflist_getfile, kOptJopFlagView,
    kOptSwbFlagNewtab, kOptSwbFlagVsplit,
};
use crate::charset::{
    ascii_isalpha, ascii_isdigit, ascii_islower, ascii_iswhite, skipdigits, vim_isblankline,
};
use crate::cmdexpand::{
    EXPAND_FILES, ExpandOne, HLF_D, TAG_MANY, WILD_EXPAND_FREE, WILD_LIST_NOTFOUND, WILD_SILENT,
};
use crate::cursor::{check_cursor, linenr_T, pos_T};
use crate::drawscreen::{UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::typval::{
    TypvalVval, VAR_UNLOCKED, VarType, callback_free, dict_T, kCallbackNone, list_T, tv_clear,
    tv_dict_add_bool, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len,
    tv_dict_alloc, tv_dict_alloc_lock, tv_dict_find, tv_dict_get_number, tv_dict_get_string,
    tv_get_number, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_append_number,
    tv_list_append_tv, tv_list_free, typval_T, varnumber_T,
};
use crate::eval::vars::{VV_SWAPCOMMAND, set_vim_var_string};
use crate::eval::{callback_call, list2fpos, set_ref_in_callback};
use crate::ex_cmds::{getfile, prepare_tagpreview};
use crate::ex_docmd::{
    DT_FIRST, DT_JUMP, DT_LAST, DT_LTAG, DT_NEXT, DT_POP, DT_PREV, DT_SELECT, DT_TAG,
    do_cmdline_cmd, exarg_T, set_no_hlsearch,
};
use crate::file_search::{FINDFILE_FILE, vim_findfile, vim_findfile_init, vim_findfile_stopdir};
use crate::fileio::{FILE, off_T, vim_fgets, vim_fseek, vim_ftell};
use crate::fold::foldOpenCursor;
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::help::{
    DT_HELP, TAG_HELP, TAG_KEEP_LANG, TAG_NAMES, TAG_NO_TAGFUNC, TAG_REGEXP, TAG_VERBOSE,
    help_heuristic,
};
use crate::insexpand::{TAG_INS_COMP, TAG_NOIC, ins_compl_check_keys, ins_compl_interrupted};
use crate::lua::executor::nlua_call_excmd;
use crate::mark::{fm_getname, fmark_T, fmarkv_T, mark_view_make, mark_view_restore, setpcmark};
use crate::mbyte::{
    convert_setup, mb_ptr_adv, mb_strnicmp, string_convert, utfc_ptr2len, vimconv_T,
};
use crate::memory::{cstr_to_cstring, xstrlcpy};
use crate::message::{
    emsg, give_warning, msg, msg_delay, msg_outtrans, msg_putchar, msg_puts, msg_puts_title, semsg,
    smsg, verbose_enter, verbose_leave, wait_return,
};
use crate::r#move::{set_topline, validate_cursor};
use crate::option::{OptInt, copy_option_part};
use crate::option_vars::*;
use crate::os::fs::{os_fopen, os_path_exists};
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::path::{
    FullName_save, PathCmpFlags, kPathCmpExpand, kPathCmpFull, path_equal, path_has_wildcard,
    path_tail, simplify_filename, to_slash_save, vim_isAbsName,
};
use crate::quickfix::{CMDBUFFSIZE, kOptSwbFlagUsetab, set_errorlist};
use crate::regexp::{
    MAXCOL, NSUBEXP, RE_MAGIC, regmatch_T, skip_regexp, vim_regcomp, vim_regexec, vim_regfree,
};
use crate::runtime::{DIP_ALL, RuntimeCallback, do_in_runtimepath};
use crate::search::{do_search, ignorecase, ignorecase_opt};
use crate::state::MODE_INSERT;
use crate::strings::{
    gettext, vim_snprintf,
    vim_snprintf_safelen, vim_strchr,
};
use crate::window::{
    WSP_VERT, check_can_set_curbuf_forceit, swbuf_goto_win_with_buf, tabpage_index, win_T,
    win_close, win_enter, win_split, win_valid,
};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

#[repr(C)]
#[derive(Clone)]
pub struct taggy_T {
    pub tagname: Option<std::ffi::CString>,
    pub fmark: fmark_T,
    pub cur_match: c_int,
    pub cur_fnum: c_int,
    pub user_data: Option<std::ffi::CString>,
}

impl taggy_T {
    pub const ZERO: Self = Self {
        tagname: None,
        fmark: fmark_T::ZERO,
        cur_match: 0,
        cur_fnum: 0,
        user_data: None,
    };
}

const LSIZE: c_int = 512;
const TAGSTACKSIZE: c_int = 20;

const DT_FREE: c_int = 99;

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

const kCpoTagpat: c_int = b't' as c_int;
const kOptFdoFlagTag: c_uint = 0x80;
pub(crate) const kOptSwbFlagUseopen: c_uint = 0x01;
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
pub struct tagname_T {
    pub tn_tags: Option<std::ffi::CString>,
    pub tn_np: *mut c_char,
    pub tn_did_filefind_init: c_int,
    pub tn_hf_idx: c_int,
    pub tn_search_ctx: Option<crate::file_search::ff_search_ctx_T>,
}

impl tagname_T {
    pub const fn new() -> Self {
        Self {
            tn_tags: None,
            tn_np: ptr::null_mut(),
            tn_did_filefind_init: 0,
            tn_hf_idx: 0,
            tn_search_ctx: None,
        }
    }
}

impl Default for tagname_T {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for tagname_T {
    fn drop(&mut self) {
        tag_st().tag_fnames.clear();
    }
}

use crate::change::{HLF_W, TAB};
use crate::charset::NUL;
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::eval::funcs::{SEARCH_KEEP, kSpecialVarNull};
use crate::fileio::{SEEK_END, SEEK_SET};
use crate::globals::IOSIZE;
use crate::math::{FAIL, OK};
use crate::os::env::expand_T;
use crate::os::fs::MAXPATHL;
use crate::viml::parser::parser::CONV_NONE;

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
    orgpat: pat_T,
    lbuf: Vec<c_char>,
    tag_fname: Vec<c_char>,
    fp: Option<crate::os::fs::CFile>,
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
    help_lang_find: Option<[c_char; 3]>,
    is_txt: bool,
    match_count: c_int,
    ga_match: [Vec<Vec<c_char>>; MT_COUNT],
    ht_match: [std::collections::HashSet<Vec<u8>>; MT_COUNT],
}

impl findtags_state_T {
    fn new() -> Self {
        Self {
            state: tagsearch_state_T::TS_START,
            stop_searching: false,
            orgpat: pat_T {
                pat: ptr::null_mut(),
                len: 0,
                head: ptr::null_mut(),
                headlen: 0,
                regmatch: regmatch_T {
                    regprog: ptr::null_mut(),
                    startp: [ptr::null_mut(); NSUBEXP],
                    endp: [ptr::null_mut(); NSUBEXP],
                    rm_matchcol: 0,
                    rm_ic: false,
                },
            },
            lbuf: Vec::new(),
            tag_fname: Vec::new(),
            fp: None,
            flags: 0,
            tag_file_sorted: 0,
            get_searchpat: false,
            help_only: false,
            did_open: false,
            mincount: 0,
            linear: false,
            vimconv: vimconv_T::new(),
            help_lang: [0; 3],
            help_pri: 0,
            help_lang_find: None,
            is_txt: false,
            match_count: 0,
            ga_match: [const { Vec::new() }; MT_COUNT],
            ht_match: std::array::from_fn(|_| std::collections::HashSet::new()),
        }
    }
}

struct TagModuleState {
    nofile_fname: Option<std::ffi::CString>,
    tagmatchname: Option<std::ffi::CString>,
    ptag_entry: taggy_T,
    tfu_in_use: bool,
    tag_fnames: Vec<std::ffi::CString>,
    DO_TAG_MAX_NUM_MATCHES: c_int,
    DO_TAG_MATCHES: Vec<Vec<c_char>>,
    DO_TAG_FLAGS: c_int,
    TAG_LISTFILES_EXTRA: Vec<([c_char; 2], std::ffi::CString)>,
}

impl TagModuleState {
    const fn new() -> Self {
        Self {
            nofile_fname: None,
            tagmatchname: None,
            ptag_entry: taggy_T { tagname: None, fmark: fmark_T { mark: pos_T { lnum: 0, col: 0, coladd: 0 }, fnum: 0, timestamp: 0, view: fmarkv_T { topline_offset: MAXLNUM, skipcol: 0 }, additional_data: None }, cur_match: 0, cur_fnum: 0, user_data: None },
            tfu_in_use: false,
            tag_fnames: Vec::new(),
            DO_TAG_MAX_NUM_MATCHES: 0,
            DO_TAG_MATCHES: Vec::new(),
            DO_TAG_FLAGS: 0,
            TAG_LISTFILES_EXTRA: Vec::new(),
        }
    }
}

static TAG_STATE: GlobalState<TagModuleState> = GlobalState::new(TagModuleState::new());

#[inline]
fn tag_st() -> &'static mut TagModuleState {
    unsafe { &mut *TAG_STATE.as_ptr() }
}

use crate::charset::toupper_asc;

pub fn free_tagfunc_option() {
    callback_free(&mut opt_vars().p_tfu);
}

/// Mark the global `'tagfunc'` callback with `copyID` so that it is not garbage
/// collected.
pub fn set_ref_in_tagfunc(copyID: c_int) -> bool {
    set_ref_in_callback(&mut opt_vars().p_tfu, copyID, ptr::null_mut(), ptr::null_mut())
}

/// Jump to tag; handling of tag commands and tag stack.
pub fn do_tag(
    eap: *mut exarg_T,
    tag: *mut c_char,
    mut type_: c_int,
    count: c_int,
    forceit: c_int,
    verbose: bool,
) {
    unsafe {
        let tagstack: *mut taggy_T = (&raw mut (**curwin.as_ptr()).w_tagstack).cast::<taggy_T>();
        let mut tagstackidx = (**curwin.as_ptr()).w_tagstackidx;
        let mut tagstacklen = (**curwin.as_ptr()).w_tagstacklen;
        let mut cur_match: c_int = 0;
        let mut cur_fnum = (*g_st().curbuf).b_fnum;
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
            additional_data: None,
        };
        let use_tagstack: bool;
        let mut skip_msg = false;
        let mut buf_ffname = (*g_st().curbuf).b_ffname;
        let mut use_tfu = true;
        let mut tofree: Option<std::ffi::CString> = None;
        let do_tag_matches = &raw mut tag_st().DO_TAG_MATCHES;

        if type_ == DT_FREE {
            (*do_tag_matches).clear();
            return;
        }

        if tag_st().tfu_in_use {
            emsg(gettext(E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC.as_ptr()));
            return;
        }

        if g_st().postponed_split == 0 && !check_can_set_curbuf_forceit(forceit) {
            return;
        }

        if type_ == DT_HELP {
            type_ = DT_TAG;
            no_regexp = true;
            use_tfu = false;
        }

        let prev_num_matches = (*do_tag_matches).len() as c_int;
        tag_st().nofile_fname = None;

        assert!(!tag.is_null());
        'end_do_tag: {
            if opt_vars().p_tgst == 0 && *tag != NUL {
                use_tagstack = false;
                new_tag = true;
                if g_st().g_do_tagpreview != 0 {
                    tagstack_clear_entry(&mut tag_st().ptag_entry);
                    tag_st().ptag_entry.tagname = Some(CStr::from_ptr(tag).to_owned());
                }
            } else {
                use_tagstack = g_st().g_do_tagpreview == 0;

                // new pattern, add to the tag stack
                if *tag != NUL
                    && (type_ == DT_TAG
                        || type_ == DT_SELECT
                        || type_ == DT_JUMP
                        || type_ == DT_LTAG)
                {
                    if g_st().g_do_tagpreview != 0 {
                        if (&raw const tag_st().ptag_entry.tagname)
                            .as_ref()
                            .unwrap()
                            .as_ref()
                            .is_some_and(|t| std::ffi::CStr::from_ptr(t.as_ptr()).to_bytes() == std::ffi::CStr::from_ptr(tag).to_bytes())
                        {
                            cur_match = tag_st().ptag_entry.cur_match;
                            cur_fnum = tag_st().ptag_entry.cur_fnum;
                        } else {
                            tagstack_clear_entry(&mut tag_st().ptag_entry);
                            tag_st().ptag_entry.tagname = Some(CStr::from_ptr(tag).to_owned());
                        }
                    } else {
                        while tagstackidx < tagstacklen {
                            tagstacklen -= 1;
                            tagstack_clear_entry(&mut *tagstack.add(tagstacklen as usize));
                        }

                        tagstacklen += 1;
                        if tagstacklen > TAGSTACKSIZE {
                            tagstacklen = TAGSTACKSIZE;
                            (&mut (**curwin.as_ptr()).w_tagstack)[..tagstacklen as usize].rotate_left(1);
                            tagstackidx -= 1;
                            tagstack_clear_entry(&mut *tagstack.add(tagstackidx as usize));
                        }

                        (*tagstack.add(tagstackidx as usize)).tagname =
                            Some(CStr::from_ptr(tag).to_owned());
                        (**curwin.as_ptr()).w_tagstacklen = tagstacklen;
                        save_pos = true;
                    }

                    new_tag = true;
                } else {
                    if if g_st().g_do_tagpreview != 0 {
                        tag_st().ptag_entry.tagname.is_none()
                    } else {
                        tagstacklen == 0
                    } {
                        emsg(gettext(E_TAG_STACK_EMPTY.as_ptr()));
                        break 'end_do_tag;
                    }

                    if type_ == DT_POP {
                        let old_KeyTyped = g_st().KeyTyped;
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

                        saved_fmark = (*tagstack.add(tagstackidx as usize)).fmark.clone();
                        if saved_fmark.fnum != (*g_st().curbuf).b_fnum {
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
                            (**curwin.as_ptr()).w_cursor.lnum = saved_fmark.mark.lnum;
                        } else {
                            setpcmark();
                            (**curwin.as_ptr()).w_cursor.lnum = saved_fmark.mark.lnum;
                        }
                        (**curwin.as_ptr()).w_cursor.col = saved_fmark.mark.col;
                        (**curwin.as_ptr()).w_set_curswant = true;
                        if (opt_vars().jop_flags & kOptJopFlagView) != 0 {
                            mark_view_restore(&raw mut saved_fmark);
                        }
                        check_cursor(*curwin.as_ptr());
                        if (opt_vars().fdo_flags & kOptFdoFlagTag) != 0 && old_KeyTyped {
                            foldOpenCursor();
                        }

                        (*do_tag_matches).clear();
                        tag_freematch();
                        break 'end_do_tag;
                    }

                    if type_ == DT_TAG || type_ == DT_LTAG {
                        if g_st().g_do_tagpreview != 0 {
                            cur_match = tag_st().ptag_entry.cur_match;
                            cur_fnum = tag_st().ptag_entry.cur_fnum;
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

                        if g_st().g_do_tagpreview != 0 {
                            cur_match = tag_st().ptag_entry.cur_match;
                            cur_fnum = tag_st().ptag_entry.cur_fnum;
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
                            cur_fnum = (*g_st().curbuf).b_fnum;
                        }
                    }
                }

                if g_st().g_do_tagpreview != 0 {
                    if type_ != DT_SELECT && type_ != DT_JUMP {
                        tag_st().ptag_entry.cur_match = cur_match;
                        tag_st().ptag_entry.cur_fnum = cur_fnum;
                    }
                } else {
                    saved_fmark = (*tagstack.add(tagstackidx as usize)).fmark.clone();
                    if save_pos {
                        (*tagstack.add(tagstackidx as usize)).fmark.mark = (**curwin.as_ptr()).w_cursor;
                        (*tagstack.add(tagstackidx as usize)).fmark.fnum = (*g_st().curbuf).b_fnum;
                        (*tagstack.add(tagstackidx as usize)).fmark.view =
                            mark_view_make(&**curwin.as_ptr(), (**curwin.as_ptr()).w_cursor);
                    }

                    (**curwin.as_ptr()).w_tagstackidx = tagstackidx;
                    if type_ != DT_SELECT && type_ != DT_JUMP {
                        (**curwin.as_ptr()).w_tagstack[tagstackidx as usize].cur_match = cur_match;
                        (**curwin.as_ptr()).w_tagstack[tagstackidx as usize].cur_fnum = cur_fnum;
                    }
                }
            }

            if cur_fnum != (*g_st().curbuf).b_fnum {
                let buf = buflist_findnr(cur_fnum);
                if !buf.is_null() {
                    buf_ffname = (*buf).b_ffname;
                }
            }

            loop {
                let mut name: *mut c_char = if use_tagstack {
                    tofree = (*tagstack.add(tagstackidx as usize)).tagname.clone();
                    tofree
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
                } else if g_st().g_do_tagpreview != 0 {
                    (&raw const tag_st().ptag_entry.tagname)
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
                } else {
                    tag
                };

                let other_name = (&raw const tag_st().tagmatchname)
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .is_none_or(|m| std::ffi::CStr::from_ptr(m.as_ptr()).to_bytes() != std::ffi::CStr::from_ptr(name).to_bytes());
                if new_tag
                    || (cur_match >= (*do_tag_matches).len() as c_int
                        && tag_st().DO_TAG_MAX_NUM_MATCHES != MAXCOL)
                    || other_name
                {
                    if other_name {
                        tag_st().tagmatchname = Some(CStr::from_ptr(name).to_owned());
                    }

                    if type_ == DT_SELECT || type_ == DT_JUMP || type_ == DT_LTAG {
                        cur_match = MAXCOL - 1;
                    }
                    tag_st().DO_TAG_MAX_NUM_MATCHES = if type_ == DT_TAG {
                        MAXCOL
                    } else {
                        cur_match + 1
                    };

                    if !no_regexp && *name == b'/' as c_char {
                        tag_st().DO_TAG_FLAGS = TAG_REGEXP;
                        name = name.add(1);
                    } else {
                        tag_st().DO_TAG_FLAGS = TAG_NOIC;
                    }

                    if verbose {
                        tag_st().DO_TAG_FLAGS |= TAG_VERBOSE;
                    }
                    if !use_tfu {
                        tag_st().DO_TAG_FLAGS |= TAG_NO_TAGFUNC;
                    }

                    let mut new_matches: Vec<Vec<c_char>> = Vec::new();
                    if find_tags(
                        name,
                        &mut new_matches,
                        tag_st().DO_TAG_FLAGS,
                        tag_st().DO_TAG_MAX_NUM_MATCHES,
                        buf_ffname,
                    ) == OK
                        && (new_matches.len() as c_int) < tag_st().DO_TAG_MAX_NUM_MATCHES
                    {
                        tag_st().DO_TAG_MAX_NUM_MATCHES = MAXCOL;
                    }

                    if tagstack != (&raw mut (**curwin.as_ptr()).w_tagstack).cast::<taggy_T>() {
                        emsg(gettext(
                            E_WINDOW_UNEXPECTEDLY_CLOSE_WHILE_SEARCHING_FOR_TAGS.as_ptr(),
                        ));
                        break;
                    }

                    if !new_tag && !other_name {
                        let mut idx: usize = 0;
                        let mut tagp = tagptrs_T::default();
                        let mut tagp2 = tagptrs_T::default();

                        for old_match in &*do_tag_matches {
                            parse_match(old_match.as_ptr().cast_mut(), &mut tagp);
                            for i in idx..new_matches.len() {
                                parse_match(new_matches[i].as_mut_ptr(), &mut tagp2);
                                if std::ffi::CStr::from_ptr(tagp.tagname).to_bytes() == std::ffi::CStr::from_ptr(tagp2.tagname).to_bytes() {
                                    let p = new_matches.remove(i);
                                    new_matches.insert(idx, p);
                                    idx += 1;
                                    break;
                                }
                            }
                        }
                    }
                    *do_tag_matches = new_matches;
                }

                let do_tag_num_matches = (*do_tag_matches).len() as c_int;
                if do_tag_num_matches <= 0 {
                    if verbose {
                        semsg!(gettext(E_TAG_NOT_FOUND_STR.as_ptr()), name);
                    }
                    g_st().g_do_tagpreview = 0;
                } else {
                    if type_ == DT_TAG && *tag != NUL {
                        cur_match = if count > 0 { count - 1 } else { 0 };
                    } else if type_ == DT_SELECT || (type_ == DT_JUMP && do_tag_num_matches > 1) {
                        select_tag_match(eap, new_tag, use_tagstack, &*do_tag_matches, name);
                        if use_tagstack {
                            (*tagstack.add(tagstackidx as usize)).fmark = saved_fmark;
                            tagstackidx = prevtagstackidx;
                        }
                        break;
                    } else if type_ == DT_LTAG {
                        if add_llist_tags(tag, &*do_tag_matches) == FAIL {
                            break 'end_do_tag;
                        }
                        cur_match = 0;
                    }

                    if cur_match >= do_tag_num_matches {
                        if (type_ == DT_NEXT || type_ == DT_FIRST)
                            && tag_st().nofile_fname.is_none()
                        {
                            if do_tag_num_matches == 1 {
                                emsg(gettext(c"E427: There is only one matching tag".as_ptr()));
                            } else {
                                emsg(gettext(
                                    c"E428: Cannot go beyond last matching tag".as_ptr(),
                                ));
                            }
                            skip_msg = true;
                        }
                        cur_match = do_tag_num_matches - 1;
                    }
                    if use_tagstack {
                        let mut tagp2 = tagptrs_T::default();

                        (*tagstack.add(tagstackidx as usize)).cur_match = cur_match;
                        (*tagstack.add(tagstackidx as usize)).cur_fnum = cur_fnum;

                        if use_tfu
                            && parse_match(
                                (*do_tag_matches).as_slice()[cur_match as usize]
                                    .as_ptr()
                                    .cast_mut(),
                                &mut tagp2,
                            ) == OK
                            && !tagp2.user_data.is_null()
                        {
                            let ud_slice = std::slice::from_raw_parts(
                                tagp2.user_data.cast::<u8>(),
                                tagp2.user_data_end.offset_from(tagp2.user_data) as usize,
                            );
                            (*tagstack.add(tagstackidx as usize)).user_data =
                                Some(std::ffi::CString::new(ud_slice).unwrap());
                        }

                        tagstackidx += 1;
                    } else if g_st().g_do_tagpreview != 0 {
                        tag_st().ptag_entry.cur_match = cur_match;
                        tag_st().ptag_entry.cur_fnum = cur_fnum;
                    }

                    if let Some(ref nf) = tag_st().nofile_fname
                        && error_cur_match != cur_match
                    {
                        smsg!(
                            0,
                            gettext(c"File \"%s\" does not exist".as_ptr()),
                            nf.as_ptr(),
                        );
                    }

                    let cur_match_ptr = (*do_tag_matches).as_slice()[cur_match as usize]
                        .as_ptr()
                        .cast_mut();
                    let ic = ((*cur_match_ptr as usize) & MT_IC_OFF) != 0;
                    if type_ != DT_TAG
                        && type_ != DT_SELECT
                        && type_ != DT_JUMP
                        && (do_tag_num_matches > 1 || ic)
                        && !skip_msg
                    {
                        let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                        let more_str = if tag_st().DO_TAG_MAX_NUM_MATCHES != MAXCOL {
                            gettext(c" or more".as_ptr()).cast_const()
                        } else {
                            c"".as_ptr()
                        };
                        vim_snprintf!(
                            iobuf_ptr,
                            IOSIZE,
                            gettext(c"tag %d of %d%s".as_ptr()),
                            cur_match + 1,
                            do_tag_num_matches,
                            more_str,
                        );
                        if ic {
                            { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = std::ffi::CStr::from_ptr(_dst).to_bytes().len(); crate::memory::xstrlcpy(_dst.add(_dlen), gettext(c"  Using tag with different case!".as_ptr()), _dsize - _dlen); };
                        }
                        if (do_tag_num_matches > prev_num_matches || new_tag)
                            && do_tag_num_matches > 1
                        {
                            msg(iobuf_ptr, if ic { HLF_W } else { 0 });
                            g_st().msg_scroll = 1;
                        } else {
                            give_warning(iobuf_ptr, ic, true);
                        }
                        if ic && g_st().msg_scrolled == 0 && g_st().msg_silent == 0 {
                            msg_delay(1007, true);
                        }
                    }

                    let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                    let iobufflen =
                        vim_snprintf_safelen!(iobuf_ptr, IOSIZE, c":ta %s\r".as_ptr(), name);
                    set_vim_var_string(VV_SWAPCOMMAND, iobuf_ptr, iobufflen as isize);

                    let i = jumpto_tag(cur_match_ptr, forceit, true);

                    set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);

                    if i == NOTAGFILE {
                        if (type_ == DT_PREV && cur_match > 0)
                            || ((type_ == DT_TAG || type_ == DT_NEXT || type_ == DT_FIRST)
                                && (tag_st().DO_TAG_MAX_NUM_MATCHES != MAXCOL
                                    || cur_match < do_tag_num_matches - 1))
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
                            (&raw const tag_st().nofile_fname)
                                .as_ref()
                                .unwrap()
                                .as_ref()
                                .map_or(c"".as_ptr(), |s| s.as_ptr()),
                        );
                    } else if use_tagstack && tagstackidx > (**curwin.as_ptr()).w_tagstacklen {
                        tagstackidx = (**curwin.as_ptr()).w_tagstackidx;
                    }
                }
                break;
            }
        }

        if use_tagstack && tagstackidx <= (**curwin.as_ptr()).w_tagstacklen {
            (**curwin.as_ptr()).w_tagstackidx = tagstackidx;
        }
        g_st().postponed_split = 0;
        g_st().g_do_tagpreview = 0;
        drop(tofree);
    }
}

/// Let the user pick from `matches`. Delegates to (async) `vim.ui.select()`.
fn select_tag_match(
    eap: *mut exarg_T,
    new_tag: bool,
    use_tagstack: bool,
    matches: &[Vec<c_char>],
    name: *const c_char,
) {
    unsafe {
        let tagstack: *mut taggy_T = (&raw mut (**curwin.as_ptr()).w_tagstack).cast::<taggy_T>();
        let tagstackidx = (**curwin.as_ptr()).w_tagstackidx;

        let mut items_tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        tv_list_alloc_ret(&mut items_tv, matches.len() as isize);

        for (idx, m_vec) in matches.iter().enumerate() {
            let m = m_vec.as_ptr().cast_mut();
            let i = idx as c_int;
            os_breakcheck();
            if g_st().got_int {
                tv_clear(&raw mut items_tv);
                return;
            }
            let mut tagp = tagptrs_T::default();
            parse_match(m, &mut tagp);

            let cur = !new_tag
                && ((g_st().g_do_tagpreview != 0 && i == tag_st().ptag_entry.cur_match)
                    || (use_tagstack && i == (*tagstack.add(tagstackidx as usize)).cur_match));

            let d = tv_dict_alloc();
            tv_dict_add_str_len(
                d,
                c"tag".as_ptr(),
                3,
                tagp.tagname,
                tagp.tagname_end.offset_from(tagp.tagname) as c_int,
            );
            let pri_idx = (*m as usize) & MT_MASK;
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
            tv_dict_add_str(d, c"file".as_ptr(), 4, fname.as_ptr());
            tv_dict_add_bool(
                d,
                c"cur".as_ptr(),
                3,
                if cur { kBoolVarTrue } else { kBoolVarFalse },
            );

            let mut item = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Dict(d),
            };
            tv_list_append_tv(items_tv.vval.v_list(), &raw mut item);
        }

        let extra_d = tv_dict_alloc();
        tv_dict_add_list(extra_d, c"items".as_ptr(), 5, items_tv.vval.v_list());
        (*items_tv.vval.v_list()).lv_refcount += 1;
        tv_dict_add_str(extra_d, c"tagname".as_ptr(), 7, name);
        let mut extra_tv = typval_T {
            v_type: VarType::VAR_DICT,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(extra_d),
        };

        nlua_call_excmd(
            c"vim._core.tag".as_ptr(),
            c"select_tag".as_ptr(),
            &mut *eap,
            &g_st().cmdmod,
            Some(&mut extra_tv),
        );

        tv_clear(&raw mut items_tv);
        tv_clear(&raw mut extra_tv);
    }
}

/// Add the matching tags to the location list for the current window.
fn add_llist_tags(tag: *mut c_char, matches: &[Vec<c_char>]) -> c_int {
    unsafe {
        let mut tag_name = [0 as c_char; 128 + 1];
        let mut tagp = tagptrs_T::default();

        let mut fname_vec = vec![0 as c_char; MAXPATHL + 1];
        let fname = fname_vec.as_mut_ptr();
        let mut cmd_vec = vec![0 as c_char; CMDBUFFSIZE + 1];
        let cmd = cmd_vec.as_mut_ptr();
        let list = tv_list_alloc(0);

        for m_vec in matches {
            let m = m_vec.as_ptr().cast_mut();
            parse_match(m, &mut tagp);

            let mut len = (tagp.tagname_end.offset_from(tagp.tagname) as c_int).min(128);
            { let _d = (tag_name.as_mut_ptr()).cast::<u8>(); let _n = len as usize; if _n > 0 { std::ptr::copy_nonoverlapping((tagp.tagname).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
            tag_name[len as usize] = NUL;

            let p = tag_full_fname(&mut tagp);
            xstrlcpy(fname, p.as_ptr(), MAXPATHL);

            let mut lnum: linenr_T = 0;
            if ascii_isdigit(*tagp.command) {
                lnum = crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(tagp.command).to_bytes()) as c_int;
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
                    { let s = c"^".as_ptr(); std::ptr::copy_nonoverlapping(s, cmd, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    cmd_start = cmd_start.add(1);
                    len += 1;
                }

                { let s = c"\\V".as_ptr(); std::ptr::copy_nonoverlapping(s, cmd.add(len as usize), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
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

        let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        vim_snprintf!(iobuf_ptr, IOSIZE, c"ltag %s".as_ptr(), tag);
        set_errorlist(*curwin.as_ptr(), list, b' ' as c_int, iobuf_ptr, ptr::null_mut());

        tv_list_free(list);

        OK
    }
}

/// Free cached tags.
pub fn tag_freematch() {
    tag_st().tagmatchname = None;
}

/// Print the tag stack.
pub fn do_tags(_eap: &mut exarg_T) {
    unsafe {
        let tagstack: *mut taggy_T = (&raw mut (**curwin.as_ptr()).w_tagstack).cast::<taggy_T>();
        let tagstackidx = (**curwin.as_ptr()).w_tagstackidx;
        let tagstacklen = (**curwin.as_ptr()).w_tagstacklen;

        msg_puts_title(gettext(
            c"\n  # TO tag         FROM line  in file/text".as_ptr(),
        ));
        for i in 0..tagstacklen {
            let entry = tagstack.add(i as usize);
            if let Some(ref tagname) = (*entry).tagname {
                let Some(name) = fm_getname(&mut (*entry).fmark, 30) else {
                    continue;
                };

                msg_putchar(b'\n' as c_int);
                let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
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
                    tagname.as_ptr(),
                    (*entry).fmark.mark.lnum,
                );
                msg_outtrans(iobuf_ptr, 0, false);
                msg_outtrans(
                    name.as_ptr(),
                    if (*entry).fmark.fnum == (*g_st().curbuf).b_fnum {
                        HLF_D
                    } else {
                        0
                    },
                    false,
                );
            }
        }
        if tagstackidx == tagstacklen {
            msg_puts(c"\n>".as_ptr());
        }
    }
}

fn tag_strnicmp(mut s1: *const c_char, mut s2: *const c_char, mut len: usize) -> c_int {
    unsafe {
        while len > 0 {
            let i = toupper_asc(c_int::from(*s1 as u8)) - toupper_asc(c_int::from(*s2 as u8));
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

fn prepare_pats(pats: &mut pat_T, has_re: bool) {
    unsafe {
        pats.head = pats.pat;
        pats.headlen = pats.len;
        if has_re {
            if *pats.pat == b'^' as c_char {
                pats.head = pats.pat.add(1);
            } else if *pats.pat == b'\\' as c_char && *pats.pat.add(1) == b'<' as c_char {
                pats.head = pats.pat.add(2);
            }
            if pats.head == pats.pat {
                pats.headlen = 0;
            } else {
                pats.headlen = 0;
                while *pats.head.add(pats.headlen as usize) != NUL {
                    let magic_chars = if opt_vars().p_magic != 0 {
                        c".[~*\\$".as_ptr()
                    } else {
                        c"\\$".as_ptr()
                    };
                    if !vim_strchr(
                        magic_chars,
                        *pats.head.add(pats.headlen as usize) as u8 as c_int,
                    )
                    .is_null()
                    {
                        break;
                    }
                    pats.headlen += 1;
                }
            }
            if opt_vars().p_tl != 0 && (pats.headlen as OptInt) > opt_vars().p_tl {
                pats.headlen = opt_vars().p_tl as c_int;
            }
        }

        if has_re {
            pats.regmatch.regprog =
                vim_regcomp(pats.pat, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
        } else {
            pats.regmatch.regprog = ptr::null_mut();
        }
    }
}

fn tagfunc_cmd_kind(cmd: *mut c_char) -> tagcmd_T {
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
        if CStr::from_ptr(cmd).to_bytes().iter().any(|b| b"\t\r\n".contains(b)) {
            return tagcmd_T::TAGCMD_INVALID;
        }
        tagcmd_T::TAGCMD_GENERIC
    }
}

fn find_tagfunc_tags(
    pat: *mut c_char,
    ga: &mut Vec<Vec<c_char>>,
    match_count: &mut c_int,
    flags: c_int,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let mut ntags: c_int = 0;
        let mut args = [typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        }; 4];
        let mut rettv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        let mut flagString = [0 as c_char; 4];
        let mut tag: *mut taggy_T = ptr::null_mut();

        if (**curwin.as_ptr()).w_tagstacklen > 0 {
            let tagstack_ptr = (&raw mut (**curwin.as_ptr()).w_tagstack).cast::<taggy_T>();
            if (**curwin.as_ptr()).w_tagstackidx == (**curwin.as_ptr()).w_tagstacklen {
                tag = tagstack_ptr.add(((**curwin.as_ptr()).w_tagstackidx - 1) as usize);
            } else {
                tag = tagstack_ptr.add((**curwin.as_ptr()).w_tagstackidx as usize);
            }
        }

        if (*g_st().curbuf).b_p_tfu.cb_type() == kCallbackNone {
            return FAIL;
        }

        args[0].v_type = VarType::VAR_STRING;
        args[0].vval = crate::eval::typval::TypvalVval::String(pat);
        args[1].v_type = VarType::VAR_STRING;
        args[1].vval = crate::eval::typval::TypvalVval::String(flagString.as_mut_ptr());

        let d = tv_dict_alloc_lock(VAR_FIXED);
        if (flags & TAG_INS_COMP) == 0
            && !tag.is_null()
            && let Some(ref ud) = (*tag).user_data
        {
            tv_dict_add_str(d, c"user_data".as_ptr(), 9, ud.as_ptr());
        }
        if !buf_ffname.is_null() {
            tv_dict_add_str(d, c"buf_ffname".as_ptr(), 10, buf_ffname);
        }

        (*d).dv_refcount += 1;
        args[2].v_type = VarType::VAR_DICT;
        args[2].vval = crate::eval::typval::TypvalVval::Dict(d);
        args[3].v_type = VarType::VAR_UNKNOWN;

        vim_snprintf!(
            flagString.as_mut_ptr(),
            flagString.len(),
            c"%s%s%s".as_ptr(),
            if g_st().g_tag_at_cursor {
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

        let save_pos = (**curwin.as_ptr()).w_cursor;
        let mut result = if callback_call(
            &raw mut (*g_st().curbuf).b_p_tfu,
            3,
            args.as_mut_ptr(),
            &raw mut rettv,
        ) {
            OK
        } else {
            FAIL
        };
        (**curwin.as_ptr()).w_cursor = save_pos;
        check_cursor(*curwin.as_ptr());
        (*d).dv_refcount -= 1;

        if result == FAIL {
            return FAIL;
        }
        if rettv.v_type == VarType::VAR_SPECIAL && rettv.vval.v_special() == kSpecialVarNull {
            tv_clear(&raw mut rettv);
            return NOTDONE;
        }
        if rettv.v_type != VarType::VAR_LIST || rettv.vval.v_list().is_null() {
            tv_clear(&raw mut rettv);
            emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC.as_ptr()));
            return FAIL;
        }
        let taglist = rettv.vval.v_list();

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
            let item_dict = (*li).li_tv.vval.v_dict();
            if !item_dict.is_null() {
                let mut todo = (*item_dict).dv_hashtab.ht_used;
                let mut hi = (*item_dict).dv_hashtab.array_ptr();
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        todo -= 1;
                        let di = crate::eval::typval::tv_dict_hi2di(hi);
                        let dict_key = (&*hi).key_ptr();
                        let tv = &raw mut (*di).di_tv;

                        if (*tv).v_type == VarType::VAR_STRING && !(*tv).vval.v_string().is_null() {
                            len += std::ffi::CStr::from_ptr((*tv).vval.v_string()).to_bytes().len() + 1;
                            if std::ffi::CStr::from_ptr(dict_key).to_bytes() == b"name" {
                                res_name = (*tv).vval.v_string();
                            } else if std::ffi::CStr::from_ptr(dict_key).to_bytes() == b"filename" {
                                res_fname = (*tv).vval.v_string();
                            } else if std::ffi::CStr::from_ptr(dict_key).to_bytes() == b"cmd" {
                                res_cmd = (*tv).vval.v_string();
                            } else {
                                has_extra = true;
                                if std::ffi::CStr::from_ptr(dict_key).to_bytes() == b"kind" {
                                    res_kind = (*tv).vval.v_string();
                                } else {
                                    len += std::ffi::CStr::from_ptr(dict_key).to_bytes().len() + 1;
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
            let mut mfp_vec: Vec<c_char> = if name_only {
                cstr_to_cstring(res_name)
                    .into_bytes_with_nul()
                    .into_iter()
                    .map(|b| b as c_char)
                    .collect()
            } else {
                vec![0 as c_char; len + 2]
            };
            let mfp = mfp_vec.as_mut_ptr();

            if !name_only {
                let mut p = mfp;
                *p = (MT_GL_OTH + 1) as c_char;
                p = p.add(1);
                *p = TAG_SEP;
                p = p.add(1);

                { let s = res_name; std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());

                *p = TAB;
                p = p.add(1);
                { let s = res_fname; std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());

                *p = TAB;
                p = p.add(1);
                { let s = res_cmd; std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());

                if cmdkind == tagcmd_T::TAGCMD_GENERIC {
                    *p = b'|' as c_char;
                    p = p.add(1);
                }
                if cmdkind == tagcmd_T::TAGCMD_GENERIC || has_extra {
                    { let s = c";\"".as_ptr(); std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());

                    if !res_kind.is_null() {
                        *p = TAB;
                        p = p.add(1);
                        { let s = res_kind; std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                        p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());
                    }

                    if !item_dict.is_null() {
                        let mut todo = (*item_dict).dv_hashtab.ht_used;
                        let mut hi = (*item_dict).dv_hashtab.array_ptr();
                        while todo > 0 {
                            if !hashitem_empty(&*hi) {
                                todo -= 1;
                                let di = crate::eval::typval::tv_dict_hi2di(hi);
                                let dict_key = (&*hi).key_ptr();
                                let tv = &raw mut (*di).di_tv;

                                if (*tv).v_type == VarType::VAR_STRING
                                    && !(*tv).vval.v_string().is_null()
                                    && std::ffi::CStr::from_ptr(dict_key).to_bytes() != b"name"
                                    && std::ffi::CStr::from_ptr(dict_key).to_bytes() != b"filename"
                                    && std::ffi::CStr::from_ptr(dict_key).to_bytes() != b"cmd"
                                    && std::ffi::CStr::from_ptr(dict_key).to_bytes() != b"kind"
                                {
                                    *p = TAB;
                                    p = p.add(1);
                                    { let s = dict_key; std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                                    p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());
                                    { let s = c":".as_ptr(); std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                                    p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());
                                    { let s = (*tv).vval.v_string(); std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                                    p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());
                                }
                            }
                            hi = hi.add(1);
                        }
                    }
                }
            }

            ga.push(mfp_vec);
            ntags += 1;
            result = OK;

            li = (*li).li_next;
        }

        tv_clear(&raw mut rettv);

        *match_count = ntags;
        result
    }
}

impl findtags_state_T {
    #[inline]
    fn fp_ptr(&self) -> *mut FILE {
        self.fp.as_ref().map_or(ptr::null_mut(), |f| f.as_ptr())
    }
}

fn findtags_state_init(
    st: &mut findtags_state_T,
    pat: *mut c_char,
    flags: c_int,
    mincount: c_int,
) {
    unsafe {
        let st = &mut *st;
        st.tag_fname = vec![0 as c_char; MAXPATHL + 1];
        st.fp = None;
        st.orgpat = pat_T {
            pat,
            len: std::ffi::CStr::from_ptr(pat).to_bytes().len() as c_int,
            head: ptr::null_mut(),
            headlen: 0,
            regmatch: regmatch_T {
                regprog: ptr::null_mut(),
                startp: [ptr::null_mut(); NSUBEXP],
                endp: [ptr::null_mut(); NSUBEXP],
                rm_matchcol: 0,
                rm_ic: false,
            },
        };
        st.flags = flags;
        st.tag_file_sorted = NUL as c_int;
        st.help_lang_find = None;
        st.is_txt = false;
        st.did_open = false;
        st.help_only = (flags & TAG_HELP) != 0;
        st.get_searchpat = false;
        st.help_lang = [NUL; 3];
        st.help_pri = 0;
        st.mincount = mincount;
        st.lbuf = vec![0 as c_char; LSIZE as usize];
        st.match_count = 0;
        st.stop_searching = false;

        for mtt in 0..MT_COUNT {
            st.ga_match[mtt].clear();
            st.ht_match[mtt].clear();
        }
    }
}

impl Drop for findtags_state_T {
    fn drop(&mut self) {
        vim_regfree(self.orgpat.regmatch.regprog);
    }
}

fn findtags_in_help_init(st: *mut findtags_state_T) -> bool {
    unsafe {
        let st = &mut *st;
        let tag_fname = st.tag_fname.as_ptr();
        if st.is_txt {
            { let s = c"en".as_ptr(); std::ptr::copy_nonoverlapping(s, st.help_lang.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
        } else {
            let i = std::ffi::CStr::from_ptr(tag_fname).to_bytes().len() as c_int;
            if i > 3 && *tag_fname.add((i - 3) as usize) == b'-' as c_char {
                { let _d = (st.help_lang.as_mut_ptr()).cast::<u8>(); let _n = 2; if _n > 0 { std::ptr::copy_nonoverlapping((tag_fname.add((i - 2) as usize)).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
            } else {
                { let s = c"en".as_ptr(); std::ptr::copy_nonoverlapping(s, st.help_lang.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            }
        }

        if let Some(ref lang_find) = st.help_lang_find
            && mb_strnicmp(
                st.help_lang.as_ptr(),
                lang_find.as_ptr(),
                MAXCOL as usize,
            ) != 0
        {
            return false;
        }

        let fname_len = if !(*g_st().curbuf).b_fname.is_null() {
            std::ffi::CStr::from_ptr((*g_st().curbuf).b_fname).to_bytes().len() as c_int
        } else {
            0
        };
        if (st.flags & TAG_KEEP_LANG) != 0
            && st.help_lang_find.is_none()
            && !(*g_st().curbuf).b_fname.is_null()
            && fname_len > 4
            && *(*g_st().curbuf).b_fname.add((fname_len - 1) as usize) == b'x' as c_char
            && *(*g_st().curbuf).b_fname.add((fname_len - 4) as usize) == b'.' as c_char
            && mb_strnicmp(
                (*g_st().curbuf).b_fname.add((fname_len - 3) as usize),
                st.help_lang.as_ptr(),
                2,
            ) == 0
        {
            st.help_pri = 0;
        } else {
            st.help_pri = 1;
            let mut s = opt_vars().p_hlg;
            while *s != NUL {
                if mb_strnicmp(s, st.help_lang.as_ptr(), 2) == 0 {
                    break;
                }
                st.help_pri += 1;
                s = vim_strchr(s, b',' as c_int);
                if s.is_null() {
                    break;
                }
                s = s.add(1);
            }
            if s.is_null() || *s == NUL {
                st.help_pri += 1;
                if mb_strnicmp(st.help_lang.as_ptr(), c"en".as_ptr(), MAXCOL as usize) != 0 {
                    st.help_pri += 1;
                }
            }
        }

        true
    }
}

fn findtags_apply_tfu(
    st: &mut findtags_state_T,
    pat: *mut c_char,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let use_tfu = (st.flags & TAG_NO_TAGFUNC) == 0;

        if !use_tfu || tag_st().tfu_in_use || (*g_st().curbuf).b_p_tfu.cb_type() == kCallbackNone {
            return NOTDONE;
        }

        tag_st().tfu_in_use = true;
        let retval = find_tagfunc_tags(
            pat,
            &mut st.ga_match[0],
            &mut st.match_count,
            st.flags,
            buf_ffname,
        );
        tag_st().tfu_in_use = false;

        retval
    }
}

fn findtags_get_next_line(
    st: &mut findtags_state_T,
    sinfo_p: &mut tagsearch_info_T,
) -> tags_read_status_T {
    unsafe {
        let mut eof: bool;

        if st.state == tagsearch_state_T::TS_BINARY {
            let offset =
                sinfo_p.low_offset + ((sinfo_p.high_offset - sinfo_p.low_offset) / 2);
            if offset == sinfo_p.curr_offset {
                return tags_read_status_T::TAGS_READ_EOF;
            } else {
                sinfo_p.curr_offset = offset;
            }
        } else if st.state == tagsearch_state_T::TS_SKIP_BACK {
            let lbuf_size = st.lbuf.len() as c_int;
            sinfo_p.curr_offset -= (lbuf_size * 2) as off_T;
            if sinfo_p.curr_offset < 0 {
                sinfo_p.curr_offset = 0;
                vim_fseek((*st).fp_ptr(), 0, SEEK_SET);
                st.state = tagsearch_state_T::TS_STEP_FORWARD;
            }
        }

        let lbuf_ptr = st.lbuf.as_mut_ptr();
        let lbuf_size = st.lbuf.len() as c_int;
        let fp = (*st).fp_ptr();
        if st.state == tagsearch_state_T::TS_BINARY
            || st.state == tagsearch_state_T::TS_SKIP_BACK
        {
            sinfo_p.curr_offset_used = sinfo_p.curr_offset;
            g_st().vim_ignored = vim_fseek(fp, sinfo_p.curr_offset, SEEK_SET);
            eof = vim_fgets(lbuf_ptr, lbuf_size, fp);
            if !eof && sinfo_p.curr_offset != 0 {
                sinfo_p.curr_offset = vim_ftell(fp);
                if sinfo_p.curr_offset == sinfo_p.high_offset {
                    g_st().vim_ignored = vim_fseek(fp, sinfo_p.low_offset, SEEK_SET);
                    sinfo_p.curr_offset = sinfo_p.low_offset;
                }
                eof = vim_fgets(lbuf_ptr, lbuf_size, fp);
            }
            while !eof && vim_isblankline(lbuf_ptr) {
                sinfo_p.curr_offset = vim_ftell(fp);
                eof = vim_fgets(lbuf_ptr, lbuf_size, fp);
            }
            if eof {
                st.state = tagsearch_state_T::TS_SKIP_BACK;
                sinfo_p.match_offset = vim_ftell(fp);
                sinfo_p.curr_offset = sinfo_p.curr_offset_used;
                return tags_read_status_T::TAGS_READ_IGNORE;
            }
        } else {
            loop {
                eof = vim_fgets(lbuf_ptr, lbuf_size, fp);
                if eof || !vim_isblankline(lbuf_ptr) {
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

fn findtags_hdr_parse(st: &mut findtags_state_T) -> bool {
    unsafe {
        let lbuf = st.lbuf.as_mut_ptr();
        if !(crate::memory::cstr_prefix_bytes(lbuf, 6) == &b"!_TAG_"[..6]) {
            return true;
        }

        if crate::memory::cstr_prefix_bytes(lbuf, 18) == &b"!_TAG_FILE_SORTED\t"[..18] {
            st.tag_file_sorted = *lbuf.add(18) as u8 as c_int;
        }
        if crate::memory::cstr_prefix_bytes(lbuf, 20) == &b"!_TAG_FILE_ENCODING\t"[..20] {
            let mut p = lbuf.add(20);
            while (*p as u8) > b' ' && (*p as u8) < 127 {
                p = p.add(1);
            }
            *p = NUL;
            convert_setup(&mut st.vimconv, lbuf.add(20), opt_vars().p_enc);
        }

        false
    }
}

fn findtags_start_state_handler(
    st: &mut findtags_state_T,
    sortic: &mut bool,
    sinfo_p: &mut tagsearch_info_T,
) -> bool {
    unsafe {
        let noic = (st.flags & TAG_NOIC) != 0;
        let lbuf = st.lbuf.as_ptr();

        if { let _p1 = lbuf; let _p2 = c"!_TAG_".as_ptr(); let _n = 6; std::slice::from_raw_parts(_p1.cast::<u8>(), crate::memory::xstrnlen(_p1, _n)) <= std::slice::from_raw_parts(_p2.cast::<u8>(), crate::memory::xstrnlen(_p2, _n)) }
            || (*lbuf == b'!' as c_char && ascii_islower(*lbuf.add(1)))
        {
            return findtags_hdr_parse(st);
        }

        if st.linear {
            st.state = tagsearch_state_T::TS_LINEAR;
        } else if st.tag_file_sorted == NUL as c_int || st.tag_file_sorted == b'1' as c_int {
            st.state = tagsearch_state_T::TS_BINARY;
        } else if st.tag_file_sorted == b'2' as c_int {
            st.state = tagsearch_state_T::TS_BINARY;
            *sortic = true;
            st.orgpat.regmatch.rm_ic = opt_vars().p_ic != 0 || !noic;
        } else {
            st.state = tagsearch_state_T::TS_LINEAR;
        }

        if st.state == tagsearch_state_T::TS_BINARY && st.orgpat.regmatch.rm_ic && !*sortic {
            st.linear = true;
            st.state = tagsearch_state_T::TS_LINEAR;
        }

        if st.state == tagsearch_state_T::TS_BINARY {
            let fp = (*st).fp_ptr();
            if vim_fseek(fp, 0, SEEK_END) != 0 {
                st.state = tagsearch_state_T::TS_LINEAR;
            } else {
                let filesize = vim_ftell(fp);
                g_st().vim_ignored = vim_fseek(fp, 0, SEEK_SET);

                sinfo_p.low_offset = 0;
                sinfo_p.low_char = 0;
                sinfo_p.high_offset = filesize;
                sinfo_p.curr_offset = 0;
                sinfo_p.high_char = 0xff;
            }
            return false;
        }

        true
    }
}

fn findtags_parse_line(
    st: &mut findtags_state_T,
    tagpp: &mut tagptrs_T,
    margs: &mut findtags_match_args_T,
    sinfo_p: &mut tagsearch_info_T,
) -> tagmatch_status_T {
    unsafe {
        let status: c_int;
        let lbuf = st.lbuf.as_mut_ptr();

        if st.orgpat.headlen != 0 {
            *tagpp = tagptrs_T::default();
            tagpp.tagname = lbuf;
            tagpp.tagname_end = vim_strchr(lbuf, TAB as c_int);
            if tagpp.tagname_end.is_null() {
                return tagmatch_status_T::TAG_MATCH_FAIL;
            }

            let mut cmplen = tagpp.tagname_end.offset_from(tagpp.tagname) as c_int;
            if opt_vars().p_tl != 0 && (cmplen as OptInt) > opt_vars().p_tl {
                cmplen = opt_vars().p_tl as c_int;
            }
            if (st.flags & TAG_REGEXP) != 0 && st.orgpat.headlen < cmplen {
                cmplen = st.orgpat.headlen;
            } else if st.state == tagsearch_state_T::TS_LINEAR && st.orgpat.headlen != cmplen
            {
                return tagmatch_status_T::TAG_MATCH_NEXT;
            }

            if st.state == tagsearch_state_T::TS_BINARY {
                let mut i = *tagpp.tagname as u8 as c_int;
                if margs.sortic {
                    i = toupper_asc(c_int::from(*tagpp.tagname as u8));
                }
                if i < sinfo_p.low_char || i > sinfo_p.high_char {
                    margs.sort_error = true;
                }

                let mut tagcmp = if margs.sortic {
                    tag_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize)
                } else {
                    { let _p1 = tagpp.tagname; let _p2 = st.orgpat.head; let _n = cmplen as usize; std::slice::from_raw_parts(_p1.cast::<u8>(), crate::memory::xstrnlen(_p1, _n)).cmp(std::slice::from_raw_parts(_p2.cast::<u8>(), crate::memory::xstrnlen(_p2, _n))) as c_int }
                };

                if tagcmp == 0 {
                    if cmplen < st.orgpat.headlen {
                        tagcmp = -1;
                    } else if cmplen > st.orgpat.headlen {
                        tagcmp = 1;
                    }
                }

                if tagcmp == 0 {
                    st.state = tagsearch_state_T::TS_SKIP_BACK;
                    sinfo_p.match_offset = sinfo_p.curr_offset;
                    return tagmatch_status_T::TAG_MATCH_NEXT;
                }
                if tagcmp < 0 {
                    sinfo_p.curr_offset = vim_ftell((*st).fp_ptr());
                    if sinfo_p.curr_offset < sinfo_p.high_offset {
                        sinfo_p.low_offset = sinfo_p.curr_offset;
                        if margs.sortic {
                            sinfo_p.low_char = toupper_asc(c_int::from(*tagpp.tagname as u8));
                        } else {
                            sinfo_p.low_char = *tagpp.tagname as u8 as c_int;
                        }
                        return tagmatch_status_T::TAG_MATCH_NEXT;
                    }
                }
                if tagcmp > 0 && sinfo_p.curr_offset != sinfo_p.high_offset {
                    sinfo_p.high_offset = sinfo_p.curr_offset;
                    if margs.sortic {
                        sinfo_p.high_char = toupper_asc(c_int::from(*tagpp.tagname as u8));
                    } else {
                        sinfo_p.high_char = *tagpp.tagname as u8 as c_int;
                    }
                    return tagmatch_status_T::TAG_MATCH_NEXT;
                }

                return tagmatch_status_T::TAG_MATCH_STOP;
            } else if st.state == tagsearch_state_T::TS_SKIP_BACK {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize) != 0 {
                    st.state = tagsearch_state_T::TS_STEP_FORWARD;
                } else {
                    sinfo_p.curr_offset = sinfo_p.curr_offset_used;
                }
                return tagmatch_status_T::TAG_MATCH_NEXT;
            } else if st.state == tagsearch_state_T::TS_STEP_FORWARD {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize) != 0 {
                    return if vim_ftell((*st).fp_ptr()) > sinfo_p.match_offset {
                        tagmatch_status_T::TAG_MATCH_STOP
                    } else {
                        tagmatch_status_T::TAG_MATCH_NEXT
                    };
                }
            } else {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize) != 0 {
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
            status = parse_tag_line(lbuf, tagpp);
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

fn findtags_match_tag(
    st: &mut findtags_state_T,
    tagpp: &mut tagptrs_T,
    margs: &mut findtags_match_args_T,
) -> bool {
    unsafe {
        let mut match_: bool;

        let mut cmplen = tagpp.tagname_end.offset_from(tagpp.tagname) as c_int;
        if opt_vars().p_tl != 0 && (cmplen as OptInt) > opt_vars().p_tl {
            cmplen = opt_vars().p_tl as c_int;
        }
        if st.orgpat.len != cmplen {
            match_ = false;
        } else if st.orgpat.regmatch.rm_ic {
            assert!(cmplen >= 0);
            match_ = mb_strnicmp(tagpp.tagname, st.orgpat.pat, cmplen as usize) == 0;
            if match_ {
                margs.match_no_ic = crate::memory::cstr_prefix_bytes(tagpp.tagname, cmplen as usize) == crate::memory::cstr_prefix_bytes(st.orgpat.pat, cmplen as usize);
            }
        } else {
            match_ = crate::memory::cstr_prefix_bytes(tagpp.tagname, cmplen as usize) == crate::memory::cstr_prefix_bytes(st.orgpat.pat, cmplen as usize);
        }

        margs.match_re = false;
        if !match_ && !st.orgpat.regmatch.regprog.is_null() {
            let cc = *tagpp.tagname_end;
            *tagpp.tagname_end = NUL;
            match_ = vim_regexec(&mut st.orgpat.regmatch, tagpp.tagname, 0);
            if match_ {
                margs.matchoff =
                    st.orgpat.regmatch.startp[0].offset_from(tagpp.tagname) as c_int;
                if st.orgpat.regmatch.rm_ic {
                    st.orgpat.regmatch.rm_ic = false;
                    margs.match_no_ic =
                        vim_regexec(&mut st.orgpat.regmatch, tagpp.tagname, 0);
                    st.orgpat.regmatch.rm_ic = true;
                }
            }
            *tagpp.tagname_end = cc;
            margs.match_re = true;
        }

        match_
    }
}

fn findtags_string_convert(st: *mut findtags_state_T) {
    unsafe {
        let st = &mut *st;
        let Some(conv_line) = string_convert(
            &st.vimconv,
            st.lbuf.as_mut_ptr(),
            ptr::null_mut(),
        ) else {
            return;
        };

        let len = conv_line.len();
        if len > st.lbuf.len() {
            st.lbuf.resize(len, 0);
        }
        { let s = conv_line.as_ptr().cast(); std::ptr::copy_nonoverlapping(s, st.lbuf.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
    }
}

fn findtags_add_match(
    st: &mut findtags_state_T,
    tagpp: &mut tagptrs_T,
    margs: &findtags_match_args_T,
    buf_ffname: *mut c_char,
) {
    unsafe {
        let name_only = (st.flags & TAG_NAMES) != 0;
        let tag_fname_ptr = st.tag_fname.as_mut_ptr();
        let lbuf_ptr = st.lbuf.as_ptr();

        let is_current =
            test_for_current(tagpp.fname, tagpp.fname_end, tag_fname_ptr, buf_ffname) != 0;
        let is_static = test_for_static(tagpp);

        let mut mtt = if is_static {
            if is_current { MT_ST_CUR } else { MT_ST_OTH }
        } else if is_current {
            MT_GL_CUR
        } else {
            MT_GL_OTH
        };

        if st.orgpat.regmatch.rm_ic && !margs.match_no_ic {
            mtt += MT_IC_OFF;
        }
        if margs.match_re {
            mtt += MT_RE_OFF;
        }

        let mut mfp_vec: Option<Vec<c_char>> = None;

        if st.help_only {
            *tagpp.tagname_end = NUL;
            let len = tagpp.tagname_end.offset_from(tagpp.tagname) as usize;
            let mfp_size = 1 + len + 10 + ML_EXTRA + 1;
            let mut v = vec![0 as c_char; mfp_size];
            let p = v.as_mut_ptr();
            { let s = tagpp.tagname; std::ptr::copy_nonoverlapping(s, p, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            *p.add(len) = b'@' as c_char;
            { let s = st.help_lang.as_ptr(); std::ptr::copy_nonoverlapping(s, p.add(len + 1), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            let heur = help_heuristic(
                tagpp.tagname,
                if margs.match_re { margs.matchoff } else { 0 },
                !margs.match_no_ic,
            ) + st.help_pri;
            vim_snprintf!(
                p.add(len + 1 + ML_EXTRA),
                mfp_size - (len + 1 + ML_EXTRA),
                c"%06d".as_ptr(),
                heur,
            );
            mfp_vec = Some(v);

            *tagpp.tagname_end = TAB;
        } else if name_only {
            if st.get_searchpat {
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
                    let mut v = vec![0 as c_char; len + 2];
                    { let _d = (v.as_mut_ptr()).cast::<u8>(); let _n = len; if _n > 0 { std::ptr::copy_nonoverlapping((tagpp.command.add(2)).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
                    mfp_vec = Some(v);
                }
                st.get_searchpat = false;
            } else {
                let len = tagpp.tagname_end.offset_from(tagpp.tagname) as usize;
                let mut v = vec![0 as c_char; 1 + len + 1];
                { let _d = (v.as_mut_ptr()).cast::<u8>(); let _n = len; if _n > 0 { std::ptr::copy_nonoverlapping((tagpp.tagname).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
                mfp_vec = Some(v);

                if (g_st().State & MODE_INSERT) != 0 {
                    st.get_searchpat = opt_vars().p_sft != 0;
                }
            }
        } else {
            let tag_fname_len = std::ffi::CStr::from_ptr(tag_fname_ptr).to_bytes().len();
            let len = tag_fname_len + std::ffi::CStr::from_ptr(lbuf_ptr).to_bytes().len() + 3;
            let mut v = vec![0 as c_char; 1 + len + 1];
            let p = v.as_mut_ptr();
            *p = (mtt + 1) as c_char;
            { let s = tag_fname_ptr; std::ptr::copy_nonoverlapping(s, p.add(1), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            *p.add(tag_fname_len + 1) = TAG_SEP;
            let s = p.add(1 + tag_fname_len + 1);
            { let _src_p = lbuf_ptr; std::ptr::copy_nonoverlapping(_src_p, s, std::ffi::CStr::from_ptr(_src_p).to_bytes_with_nul().len()); };
            mfp_vec = Some(v);
        }

        if let Some(v) = mfp_vec {
            let mfp_bytes = CStr::from_ptr(v.as_ptr()).to_bytes().to_vec();
            if st.ht_match[mtt].insert(mfp_bytes) {
                st.ga_match[mtt].push(v);
                st.match_count += 1;
            }
        }
    }
}

fn findtags_get_all_tags(
    st: &mut findtags_state_T,
    margs: &mut findtags_match_args_T,
    buf_ffname: *mut c_char,
) {
    unsafe {
        let mut tagp = tagptrs_T::default();
        let mut search_info = tagsearch_info_T::default();

        loop {
            if st.state == tagsearch_state_T::TS_BINARY
                || st.state == tagsearch_state_T::TS_SKIP_BACK
            {
                line_breakcheck();
            } else {
                fast_breakcheck();
            }
            if (st.flags & TAG_INS_COMP) != 0 {
                ins_compl_check_keys(30, false);
            }
            if g_st().got_int || ins_compl_interrupted() {
                st.stop_searching = true;
                break;
            }
            if st.mincount == TAG_MANY && st.match_count >= TAG_MANY {
                st.stop_searching = true;
                break;
            }
            if !st.get_searchpat {
                let retval = findtags_get_next_line(st, &mut search_info);
                if retval == tags_read_status_T::TAGS_READ_IGNORE {
                    continue;
                }
                if retval == tags_read_status_T::TAGS_READ_EOF {
                    break;
                }
            }

            if st.vimconv.vc_type != CONV_NONE {
                findtags_string_convert(&raw mut *st);
            }

            if st.state == tagsearch_state_T::TS_START
                && !findtags_start_state_handler(st, &mut margs.sortic, &mut search_info)
            {
                continue;
            }

            let st_ref = &mut *st;
            if st_ref.lbuf[st_ref.lbuf.len() - 2] != NUL {
                let new_size = st_ref.lbuf.len() * 2;
                st_ref.lbuf = vec![0 as c_char; new_size];

                if st_ref.state == tagsearch_state_T::TS_STEP_FORWARD
                    || st_ref.state == tagsearch_state_T::TS_LINEAR
                {
                    g_st().vim_ignored = vim_fseek(st_ref.fp_ptr(), search_info.curr_offset, SEEK_SET);
                }
                search_info.curr_offset = 0;
                continue;
            }

            let retval = findtags_parse_line(st, &mut tagp, margs, &mut search_info);
            if retval == tagmatch_status_T::TAG_MATCH_NEXT {
                continue;
            }
            if retval == tagmatch_status_T::TAG_MATCH_STOP {
                break;
            }
            if retval == tagmatch_status_T::TAG_MATCH_FAIL {
                semsg!(
                    gettext(c"E431: Format error in tags file \"%s\"".as_ptr()),
                    st.tag_fname.as_ptr(),
                );
                semsg!(
                    gettext(c"Before byte %lld".as_ptr()),
                    vim_ftell((*st).fp_ptr()) as i64,
                );
                st.stop_searching = true;
                return;
            }

            if findtags_match_tag(st, &mut tagp, margs) {
                findtags_add_match(st, &mut tagp, margs, buf_ffname);
            }
        }
    }
}

fn findtags_in_file(st: &mut findtags_state_T, _flags: c_int, buf_ffname: *mut c_char) {
    unsafe {
        let mut margs = findtags_match_args_T::default();

        st.vimconv.vc_type = CONV_NONE;
        st.tag_file_sorted = NUL as c_int;
        st.fp = None;
        findtags_matchargs_init(&mut margs, st.flags);

        if (*g_st().curbuf).b_help && !findtags_in_help_init(&raw mut *st) {
            return;
        }

        let tag_fname_ptr = st.tag_fname.as_ptr();
        st.fp = crate::os::fs::CFile::from_raw(os_fopen(tag_fname_ptr, c"r".as_ptr()));
        if st.fp.is_none() {
            return;
        }

        if opt_vars().p_verbose >= 5 {
            verbose_enter();
            smsg!(
                0,
                gettext(c"Searching tags file %s".as_ptr()),
                tag_fname_ptr,
            );
            verbose_leave();
        }
        st.did_open = true;
        st.state = tagsearch_state_T::TS_START;

        findtags_get_all_tags(st, &mut margs, buf_ffname);

        st.fp = None;
        if st.vimconv.vc_type != CONV_NONE {
            convert_setup(&mut st.vimconv, ptr::null_mut(), ptr::null_mut());
        }

        if margs.sort_error {
            semsg!(
                gettext(c"E432: Tags file not sorted: %s".as_ptr()),
                st.tag_fname.as_ptr(),
            );
        }

        if st.match_count >= st.mincount {
            st.stop_searching = true;
        }
    }
}

fn findtags_copy_matches(st: &mut findtags_state_T, matches: &mut Vec<Vec<c_char>>) {
    unsafe {
        let name_only = (st.flags & TAG_NAMES) != 0;
        let keep = st.match_count > 0;
        matches.clear();
        if keep {
            matches.reserve_exact(st.match_count as usize);
        }

        st.match_count = 0;
        for mtt in 0..MT_COUNT {
            for mut mfp_vec in st.ga_match[mtt].drain(..) {
                if keep {
                    if !name_only {
                        let mfp = mfp_vec.as_mut_ptr();
                        *mfp -= 1;
                        let mut p = mfp.add(1);
                        while *p != NUL {
                            if *p == TAG_SEP {
                                *p = NUL;
                            }
                            p = p.add(1);
                        }
                    }
                    matches.push(mfp_vec);
                    st.match_count += 1;
                }
            }

            st.ga_match[mtt] = Vec::new();
            st.ht_match[mtt].clear();
        }
    }
}

/// `find_tags()` - search for tags in tags files into owned `Vec<c_char>` items.
pub fn find_tags(
    pat: *mut c_char,
    matches: &mut Vec<Vec<c_char>>,
    flags: c_int,
    mincount: c_int,
    buf_ffname: *mut c_char,
) -> c_int {
    unsafe {
        let mut st = findtags_state_T::new();
        let mut retval: c_int;
        let mut saved_pat: Option<std::ffi::CString> = None;

        let findall = mincount == MAXCOL || mincount == TAG_MANY;
        let has_re = (flags & TAG_REGEXP) != 0;
        let noic = (flags & TAG_NOIC) != 0;
        let verbose = (flags & TAG_VERBOSE) != 0;
        let save_p_ic = opt_vars().p_ic;

        match if (*g_st().curbuf).b_tc_flags != 0 {
            (*g_st().curbuf).b_tc_flags
        } else {
            opt_vars().tc_flags
        } {
            kOptTcFlagFollowic => {}
            kOptTcFlagIgnore => opt_vars().p_ic = 1,
            kOptTcFlagMatch => opt_vars().p_ic = 0,
            kOptTcFlagFollowscs => opt_vars().p_ic = ignorecase(pat),
            kOptTcFlagSmart => opt_vars().p_ic = ignorecase_opt(pat, 1, 1),
            _ => std::process::abort(),
        }

        let help_save = (*g_st().curbuf).b_help;

        findtags_state_init(&mut st, pat, flags, mincount);

        if st.help_only {
            (*g_st().curbuf).b_help = true;
        }

        if (*g_st().curbuf).b_help
            && st.orgpat.len > 3
            && *pat.add((st.orgpat.len - 3) as usize) == b'@' as c_char
            && ascii_isalpha(*pat.add((st.orgpat.len - 2) as usize))
            && ascii_isalpha(*pat.add((st.orgpat.len - 1) as usize))
        {
            let keep = (st.orgpat.len - 3) as usize;
            let cstr =
                std::ffi::CString::new(std::slice::from_raw_parts(pat.cast::<u8>(), keep)).unwrap();
            st.help_lang_find = Some([
                *pat.add((st.orgpat.len - 2) as usize),
                *pat.add((st.orgpat.len - 1) as usize),
                NUL,
            ]);
            st.orgpat.pat = cstr.as_ptr().cast_mut();
            saved_pat = Some(cstr);
            st.orgpat.len -= 3;
        }
        if opt_vars().p_tl != 0 && (st.orgpat.len as OptInt) > opt_vars().p_tl {
            st.orgpat.len = opt_vars().p_tl as c_int;
        }

        let save_emsg_off = g_st().emsg_off;
        g_st().emsg_off = 1;
        prepare_pats(&mut st.orgpat, has_re);
        g_st().emsg_off = save_emsg_off;
        if has_re && st.orgpat.regmatch.regprog.is_null() {
            retval = FAIL;
        } else {
            retval = findtags_apply_tfu(&mut st, pat, buf_ffname);
            if retval == NOTDONE {
                retval = FAIL;

                if (flags & TAG_KEEP_LANG) != 0
                    && st.help_lang_find.is_none()
                    && !(*g_st().curbuf).b_fname.is_null()
                {
                    let i = std::ffi::CStr::from_ptr((*g_st().curbuf).b_fname).to_bytes().len() as c_int;
                    if i > 4
                        && mb_strnicmp(
                            (*g_st().curbuf).b_fname.add((i - 4) as usize),
                            c".txt".as_ptr(),
                            MAXCOL as usize,
                        ) == 0
                    {
                        st.is_txt = true;
                    }
                }

                st.orgpat.regmatch.rm_ic =
                    (opt_vars().p_ic != 0 || !noic) && (findall || st.orgpat.headlen == 0 || opt_vars().p_tbs == 0);
                for round in 1..=2 {
                    st.linear = st.orgpat.headlen == 0 || opt_vars().p_tbs == 0 || round == 2;

                    let mut tn = tagname_T::new();
                    let mut first_file: c_int = 1;
                    while get_tagfname(&mut tn, first_file, st.tag_fname.as_mut_ptr()) == OK {
                        findtags_in_file(&mut st, flags, buf_ffname);
                        if st.stop_searching {
                            retval = OK;
                            break;
                        }
                        first_file = 0;
                    }

                    if st.stop_searching
                        || st.linear
                        || (opt_vars().p_ic == 0 && noic)
                        || st.orgpat.regmatch.rm_ic
                    {
                        break;
                    }

                    st.orgpat.regmatch.rm_ic = true;
                }

                if !st.stop_searching {
                    if !st.did_open && verbose {
                        emsg(gettext(c"E433: No tags file".as_ptr()));
                    }
                    retval = OK;
                }
            }
        }

        if retval == FAIL {
            st.match_count = 0;
        }

        findtags_copy_matches(&mut st, matches);

        (*g_st().curbuf).b_help = help_save;
        drop(saved_pat);

        opt_vars().p_ic = save_p_ic;

        retval
    }
}

pub fn found_tagfile_cb(fnames: &[std::ffi::CString], all: bool) -> bool {
    unsafe {
        let tf = &raw mut tag_st().tag_fnames;
        for fname in fnames {
            let mut tag_fname = fname.clone().into_bytes_with_nul();
            simplify_filename(tag_fname.as_mut_ptr().cast());
            (*tf).push(CStr::from_ptr(tag_fname.as_ptr().cast()).to_owned());

            if !all {
                break;
            }
        }

        !fnames.is_empty()
    }
}

pub fn free_tag_stuff() {
    unsafe {
        let tf = &raw mut tag_st().tag_fnames;
        *tf = Vec::new();
        if !(*curwin.as_ptr()).is_null() {
            do_tag(ptr::null_mut(), ptr::null_mut(), DT_FREE, 0, 0, false);
        }
        tag_freematch();
        tagstack_clear_entry(&mut tag_st().ptag_entry);
    }
}

/// Get the next name of a tag file from the tag file list.
pub fn get_tagfname(tnp: &mut tagname_T, first: c_int, buf: *mut c_char) -> c_int {
    unsafe {
        let mut fname: Option<std::ffi::CString>;

        if first != 0 {
            *tnp = tagname_T::new();
        }

        if (*g_st().curbuf).b_help {
            let tf = &raw mut tag_st().tag_fnames;
            if first != 0 {
                (*tf).clear();
                do_in_runtimepath(
                    c"doc/tags doc/tags-??".as_ptr().cast_mut(),
                    DIP_ALL,
                    &mut RuntimeCallback::FoundTagfile,
                );
            }

            let len = (*tf).len() as c_int;
            if tnp.tn_hf_idx >= len {
                if tnp.tn_hf_idx > len || *opt_vars().p_hf == NUL {
                    return FAIL;
                }
                tnp.tn_hf_idx += 1;
                xstrlcpy(buf, opt_vars().p_hf, MAXPATHL - 4);
                { let s = c"tags".as_ptr(); std::ptr::copy_nonoverlapping(s, path_tail(buf), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                simplify_filename(buf);

                let buf_bytes = CStr::from_ptr(buf).to_bytes();
                if (*tf).iter().any(|entry| entry.as_bytes() == buf_bytes) {
                    return FAIL;
                }
            } else {
                xstrlcpy(
                    buf,
                    (*tf).as_slice()[tnp.tn_hf_idx as usize].as_ptr(),
                    MAXPATHL,
                );
                tnp.tn_hf_idx += 1;
            }
            return OK;
        }

        if first != 0 {
            let src = if *(*g_st().curbuf).b_p_tags != NUL {
                (*g_st().curbuf).b_p_tags
            } else {
                opt_vars().p_tags
            };
            tnp.tn_tags = Some(CStr::from_ptr(src).to_owned());
            tnp.tn_np = tnp.tn_tags.as_ref().unwrap().as_ptr().cast_mut();
        }

        loop {
            if tnp.tn_did_filefind_init != 0 {
                fname = vim_findfile(tnp.tn_search_ctx.as_mut());
                if fname.is_some() {
                    break;
                }
                tnp.tn_did_filefind_init = 0;
            } else {
                if *tnp.tn_np == NUL {
                    tnp.tn_search_ctx = None;
                    return FAIL;
                }

                *buf = NUL;
                copy_option_part(&raw mut tnp.tn_np, buf, MAXPATHL - 1, c" ,".as_ptr());

                let mut r_ptr = vim_findfile_stopdir(buf);
                let mut filename = path_tail(buf);
                if !r_ptr.is_null() {
                    ptr::copy(r_ptr, r_ptr.add(1), std::ffi::CStr::from_ptr(r_ptr).to_bytes().len() + 1);
                    r_ptr = r_ptr.add(1);
                }
                ptr::copy(filename, filename.add(1), std::ffi::CStr::from_ptr(filename).to_bytes().len() + 1);
                *filename = NUL;
                filename = filename.add(1);

                tnp.tn_search_ctx = vim_findfile_init(
                    buf,
                    filename,
                    std::ffi::CStr::from_ptr(filename).to_bytes().len(),
                    r_ptr,
                    100,
                    0,
                    FINDFILE_FILE,
                    tnp.tn_search_ctx.take(),
                    1,
                    (*g_st().curbuf).b_ffname,
                );
                if tnp.tn_search_ctx.is_some() {
                    tnp.tn_did_filefind_init = 1;
                }
            }
        }

        { let s = fname.as_ref().unwrap().as_ptr(); std::ptr::copy_nonoverlapping(s, buf, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
        OK
    }
}

fn parse_tag_line(lbuf: *mut c_char, tagp: &mut tagptrs_T) -> c_int {
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

fn test_for_static(tagp: &tagptrs_T) -> bool {
    unsafe {
        let mut p = tagp.command;
        loop {
            p = vim_strchr(p, b'\t' as c_int);
            if p.is_null() {
                break;
            }
            p = p.add(1);
            if crate::memory::cstr_prefix_bytes(p, 5) == &b"file:"[..5] {
                return true;
            }
        }
        false
    }
}

fn matching_line_len(lbuf: *const c_char) -> usize {
    unsafe {
        let mut p = lbuf.add(1);
        p = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len() + 1);
        p.offset_from(lbuf) as usize + std::ffi::CStr::from_ptr(p).to_bytes().len()
    }
}

fn parse_match(mut lbuf: *mut c_char, tagp: &mut tagptrs_T) -> c_int {
    unsafe {
        tagp.tag_fname = lbuf.add(1);
        lbuf = lbuf.add(std::ffi::CStr::from_ptr(tagp.tag_fname).to_bytes().len() + 2);

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
                    if crate::memory::cstr_prefix_bytes(p, 5) == &b"kind:"[..5] {
                        tagp.tagkind = p.add(5);
                    } else if crate::memory::cstr_prefix_bytes(p, 10) == &b"user_data:"[..10] {
                        tagp.user_data = p.add(10);
                    } else if crate::memory::cstr_prefix_bytes(p, 5) == &b"line:"[..5] {
                        tagp.tagline = crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(p.add(5)).to_bytes()) as c_int;
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

fn tag_full_fname(tagp: &mut tagptrs_T) -> std::ffi::CString {
    unsafe {
        let c = *tagp.fname_end;
        *tagp.fname_end = NUL;
        let fullname = expand_tag_fname(tagp.fname, tagp.tag_fname, false);
        *tagp.fname_end = c;
        fullname
    }
}

fn jumpto_tag(lbuf_arg: *const c_char, forceit: c_int, keep_help: bool) -> c_int {
    unsafe {
        if g_st().postponed_split == 0 && !check_can_set_curbuf_forceit(forceit) {
            return FAIL;
        }

        let mut pbuf_end: *mut c_char;
        let mut tofree_fname: Option<std::ffi::CString> = None;
        let mut tagp = tagptrs_T::default();
        let mut retval = FAIL;
        let mut getfile_result = GETFILE_UNUSED;
        let mut curwin_save: *mut win_T = ptr::null_mut();
        let mut full_fname: Option<std::ffi::CString> = None;
        let old_KeyTyped = g_st().KeyTyped;
        let l_g_do_tagpreview = g_st().g_do_tagpreview;
        let len = matching_line_len(lbuf_arg) + 1;
        let mut lbuf_vec = vec![0 as c_char; len];
        let lbuf = lbuf_vec.as_mut_ptr();
        ptr::copy(lbuf_arg, lbuf, len);

        let mut pbuf_vec = vec![0 as c_char; LSIZE as usize];
        let pbuf = pbuf_vec.as_mut_ptr();

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

            let expanded = expand_tag_fname(fname, tagp.tag_fname, true);
            fname = expanded.as_ptr().cast_mut();
            tofree_fname = Some(expanded);

            if !os_path_exists(fname) && !has_autocmd(EVENT_BUFREADCMD, fname, ptr::null_mut()) {
                retval = NOTAGFILE;
                tag_st().nofile_fname = tofree_fname.take();
                break 'erret;
            }

            g_st().RedrawingDisabled += 1;

            if l_g_do_tagpreview != 0 {
                g_st().postponed_split = 0;
                curwin_save = *curwin.as_ptr();

                if (**curwin.as_ptr()).w_onebuf_opt.wo_pvw == 0 {
                    full_fname = FullName_save(fname, false);
                    fname = full_fname
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());

                    prepare_tagpreview(true, *opt_vars().p_pvp != NUL);
                }
            }

            if g_st().postponed_split != 0 && (opt_vars().swb_flags & (kOptSwbFlagUseopen | kOptSwbFlagUsetab)) != 0 {
                let existing_buf = buflist_findname_exp(fname);
                if !existing_buf.is_null() && !swbuf_goto_win_with_buf(existing_buf).is_null() {
                    getfile_result = GETFILE_SAME_FILE;
                }
            }
            if getfile_result == GETFILE_UNUSED && (g_st().postponed_split != 0 || g_st().cmdmod.cmod_tab != 0) {
                if (opt_vars().swb_flags & kOptSwbFlagVsplit) != 0 {
                    g_st().cmdmod.cmod_split |= WSP_VERT;
                }
                if (opt_vars().swb_flags & kOptSwbFlagNewtab) != 0 && g_st().cmdmod.cmod_tab == 0 {
                    g_st().cmdmod.cmod_tab = tabpage_index(g_st().curtab) + 1;
                }
                if win_split(
                    if g_st().postponed_split > 0 {
                        g_st().postponed_split
                    } else {
                        0
                    },
                    g_st().postponed_split_flags,
                ) == FAIL
                {
                    g_st().RedrawingDisabled -= 1;
                    break 'erret;
                }
                (**curwin.as_ptr()).w_onebuf_opt.wo_scb = 0;
                (**curwin.as_ptr()).w_onebuf_opt.wo_crb = 0;
            }

            if keep_help {
                if l_g_do_tagpreview != 0 {
                    g_st().keep_help_flag = bt_help((*curwin_save).w_buffer);
                } else {
                    g_st().keep_help_flag = (*g_st().curbuf).b_help;
                }
            }

            if getfile_result == GETFILE_UNUSED {
                getfile_result = getfile(0, fname, ptr::null_mut(), true, 0, forceit != 0);
            }
            g_st().keep_help_flag = false;

            if getfile_result <= 0 {
                (**curwin.as_ptr()).w_set_curswant = true;
                g_st().postponed_split = 0;

                let save_no_hlsearch = g_st().Search.no_hlsearch;

                let search_options = if !vim_strchr(opt_vars().p_cpo, kCpoTagpat).is_null() {
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

                    let save_p_ws = opt_vars().p_ws;
                    let save_p_ic = opt_vars().p_ic;
                    let save_p_scs = opt_vars().p_scs;
                    opt_vars().p_ws = 1;
                    opt_vars().p_ic = 0;
                    opt_vars().p_scs = 0;
                    let save_lnum = (**curwin.as_ptr()).w_cursor.lnum;

                    (**curwin.as_ptr()).w_cursor.lnum = if tagp.tagline > 0 {
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
                        opt_vars().p_ic = 1;
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
                            (**curwin.as_ptr()).w_cursor.lnum = save_lnum;
                        } else {
                            if found == 2 || save_p_ic == 0 {
                                msg(
                                    gettext(c"E435: Couldn't find tag, just guessing!".as_ptr()),
                                    0,
                                );
                                if g_st().msg_scrolled == 0 && g_st().msg_silent == 0 {
                                    msg_delay(1010, true);
                                }
                            }
                            retval = OK;
                        }
                    }
                    opt_vars().p_ws = save_p_ws;
                    opt_vars().p_ic = save_p_ic;
                    opt_vars().p_scs = save_p_scs;

                    check_cursor(*curwin.as_ptr());
                } else {
                    let save_secure = g_st().secure;

                    g_st().secure = 1;
                    g_st().sandbox += 1;
                    (**curwin.as_ptr()).w_cursor.lnum = 1;
                    (**curwin.as_ptr()).w_cursor.col = 0;
                    (**curwin.as_ptr()).w_cursor.coladd = 0;
                    do_cmdline_cmd(pbuf);
                    retval = OK;

                    if g_st().secure == 2 {
                        wait_return(1);
                    }
                    g_st().secure = save_secure;
                    g_st().sandbox -= 1;
                }

                if search_options != 0 {
                    set_no_hlsearch(save_no_hlsearch);
                }

                if getfile_result == GETFILE_OPEN_OTHER {
                    retval = OK;
                }

                if retval == OK {
                    if (*g_st().curbuf).b_help {
                        set_topline(&mut **curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
                    }
                    if (opt_vars().fdo_flags & kOptFdoFlagTag) != 0 && old_KeyTyped {
                        foldOpenCursor();
                    }
                }

                if l_g_do_tagpreview != 0 && *curwin.as_ptr() != curwin_save && win_valid(curwin_save) {
                    validate_cursor(*curwin.as_ptr());
                    redraw_later(*curwin.as_ptr(), UPD_VALID);
                    win_enter(curwin_save, true);
                }

                g_st().RedrawingDisabled -= 1;
            } else {
                g_st().RedrawingDisabled -= 1;
                if g_st().postponed_split != 0 {
                    win_close(*curwin.as_ptr(), false, false);
                    g_st().postponed_split = 0;
                }
            }
        }

        g_st().g_do_tagpreview = 0;
        drop(tofree_fname);
        drop(full_fname);

        retval
    }
}

fn expand_tag_fname(
    fname_arg: *mut c_char,
    tag_fname: *mut c_char,
    expand: bool,
) -> std::ffi::CString {
    unsafe {
        let mut xpc: expand_T = expand_T::new();

        let mut fname = to_slash_save(fname_arg).unwrap_or_default();

        if expand
            && path_has_wildcard(fname.as_ptr(), true)
            && vim_strchr(fname.as_ptr(), b'`' as c_int).is_null()
        {
            xpc.xp_context = EXPAND_FILES;
            if let Some(exp) = ExpandOne(
                &mut xpc,
                fname.as_ptr().cast_mut(),
                None,
                WILD_LIST_NOTFOUND | WILD_SILENT,
                WILD_EXPAND_FREE,
            ) {
                fname = exp;
            }
        }

        if (opt_vars().p_tr != 0 || (*g_st().curbuf).b_help) && !vim_isAbsName(fname.as_ptr()) {
            let p = path_tail(tag_fname);
            if p != tag_fname {
                let mut retval = [0 as c_char; MAXPATHL];
                { let s = tag_fname; std::ptr::copy_nonoverlapping(s, retval.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                let off = p.offset_from(tag_fname) as usize;
                xstrlcpy(retval.as_mut_ptr().add(off), fname.as_ptr(), MAXPATHL - off);
                simplify_filename(retval.as_mut_ptr());
                return CStr::from_ptr(retval.as_ptr()).to_owned();
            }
        }

        fname
    }
}

fn test_for_current(
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
                fullname.as_ptr(),
                buf_ffname,
                (kPathCmpExpand | kPathCmpFull) as PathCmpFlags,
            );
            *fname_end = c;
        }
        c_int::from(retval)
    }
}

fn find_extra(pp: *mut *mut c_char) -> c_int {
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
                str_ = CStr::from_ptr(str_).to_bytes().windows(3).position(|w| w == b"|;\"").map_or(ptr::null_mut(), |i| str_.add(i));
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

        if !str_.is_null() && { let _p = str_; crate::memory::xstrnlen(_p, 2) == 2 && std::slice::from_raw_parts(_p.cast::<u8>(), 2) == &b";\""[..2] } {
            *pp = str_;
            return OK;
        }
        FAIL
    }
}

/// Free a single entry in a tag stack.
pub fn tagstack_clear_entry(item: &mut taggy_T) {
    item.tagname = None;
        item.user_data = None;
}

pub fn tag_listfiles_extra(idx: usize) -> Option<(*const c_char, *const c_char)> {
    unsafe {
        let extra = &raw const tag_st().TAG_LISTFILES_EXTRA;
        (*extra)
            .as_slice()
            .get(idx)
            .map(|(k, f)| (k.as_ptr(), f.as_ptr()))
    }
}

pub fn expand_tags(
    tagnames: bool,
    pat: *mut c_char,
    file: &mut Vec<std::ffi::CString>,
) -> c_int {
    unsafe {
        let extra_flag = if tagnames { TAG_NAMES } else { 0 };
        let mut raw_matches: Vec<Vec<c_char>> = Vec::new();
        let ret = if *pat == b'/' as c_char {
            find_tags(
                pat.add(1),
                &mut raw_matches,
                TAG_REGEXP | extra_flag | TAG_VERBOSE,
                TAG_MANY,
                (*g_st().curbuf).b_ffname,
            )
        } else {
            find_tags(
                pat,
                &mut raw_matches,
                TAG_REGEXP | extra_flag | TAG_VERBOSE | TAG_NOIC,
                TAG_MANY,
                (*g_st().curbuf).b_ffname,
            )
        };
        file.clear();
        let extra = &raw mut tag_st().TAG_LISTFILES_EXTRA;
        (*extra).clear();
        if ret == OK {
            file.reserve_exact(raw_matches.len());
            if tagnames {
                for m in raw_matches {
                    file.push(CStr::from_ptr(m.as_ptr()).to_owned());
                }
            } else {
                (*extra).reserve_exact(raw_matches.len());
                for mut m in raw_matches {
                    let mut t_p = tagptrs_T::default();
                    parse_match(m.as_mut_ptr(), &mut t_p);
                    let len = t_p.tagname_end.offset_from(t_p.tagname) as usize;
                    file.push(crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
                        t_p.tagname.cast::<u8>(),
                        len,
                    )));
                    let kind_c = if !t_p.tagkind.is_null() && *t_p.tagkind != NUL {
                        *t_p.tagkind
                    } else {
                        b'f' as c_char
                    };
                    let fname_len = t_p.fname_end.offset_from(t_p.fname) as usize;
                    let fname_cstr = crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
                        t_p.fname.cast::<u8>(),
                        fname_len,
                    ));
                    (*extra).push(([kind_c, 0], fname_cstr));
                }
            }
        }
        ret
    }
}

fn add_tag_field(
    dict: *mut dict_T,
    field_name: *const c_char,
    start: *const c_char,
    mut end: *const c_char,
) -> c_int {
    unsafe {
        if !tv_dict_find(dict, field_name, -1).is_null() {
            if opt_vars().p_verbose > 0 {
                verbose_enter();
                smsg!(0, gettext(c"Duplicate field name: %s".as_ptr()), field_name);
                verbose_leave();
            }
            return FAIL;
        }
        let mut len: c_int = 0;
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let buf = buf_vec.as_mut_ptr();
        if !start.is_null() {
            if end.is_null() {
                end = start.add(std::ffi::CStr::from_ptr(start).to_bytes().len());
                while end > start
                    && (*end.sub(1) == b'\r' as c_char || *end.sub(1) == b'\n' as c_char)
                {
                    end = end.sub(1);
                }
            }
            len = (end.offset_from(start) as c_int).min(MAXPATHL as c_int - 1);
            { let _d = (buf).cast::<u8>(); let _n = len as usize; if _n > 0 { std::ptr::copy_nonoverlapping((start).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
        }
        *buf.add(len as usize) = NUL;
        tv_dict_add_str(dict, field_name, std::ffi::CStr::from_ptr(field_name).to_bytes().len(), buf)
    }
}

/// Add the tags matching the specified pattern `pat` to the list `list`
/// as a dictionary. Use `buf_fname` for priority, unless NULL.
pub fn get_tags(list: &mut list_T, pat: *mut c_char, buf_fname: *mut c_char) -> c_int {
    unsafe {
        let mut matches: Vec<Vec<c_char>> = Vec::new();
        let mut tp = tagptrs_T::default();

        let mut ret = find_tags(pat, &mut matches, TAG_REGEXP | TAG_NOIC, MAXCOL, buf_fname);
        if ret != OK || matches.is_empty() {
            return ret;
        }

        for mut match_vec in matches {
            let match_i = match_vec.as_mut_ptr();
            if parse_match(match_i, &mut tp) == FAIL {
                continue;
            }

            let is_static = test_for_static(&tp);

            if crate::memory::cstr_prefix_bytes(tp.tagname, 6) == &b"!_TAG_"[..6] {
                continue;
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(list, dict);

            let full_fname = tag_full_fname(&mut tp);
            if add_tag_field(dict, c"name".as_ptr(), tp.tagname, tp.tagname_end) == FAIL
                || add_tag_field(
                    dict,
                    c"filename".as_ptr(),
                    full_fname.as_ptr().cast_mut(),
                    ptr::null(),
                ) == FAIL
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
            drop(full_fname);

            if !tp.command_end.is_null() {
                let mut p = tp.command_end.add(3);
                while *p != NUL && *p != b'\n' as c_char && *p != b'\r' as c_char {
                    if p == tp.tagkind
                        || (p.add(5) == tp.tagkind && (crate::memory::cstr_prefix_bytes(p, 5) == &b"kind:"[..5]))
                    {
                        p = tp.tagkind_end.sub(1);
                    } else if crate::memory::cstr_prefix_bytes(p, 5) == &b"file:"[..5] {
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
        }
        ret
    }
}

fn get_tag_details(tag: &mut taggy_T, retdict: *mut dict_T) {
    unsafe {
        tv_dict_add_str(
            retdict,
            c"tagname".as_ptr(),
            7,
            tag.tagname.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        );
        tv_dict_add_nr(
            retdict,
            c"matchnr".as_ptr(),
            7,
            (tag.cur_match + 1) as varnumber_T,
        );
        tv_dict_add_nr(
            retdict,
            c"bufnr".as_ptr(),
            5,
            tag.cur_fnum as varnumber_T,
        );
        if let Some(ref ud) = tag.user_data {
            tv_dict_add_str(retdict, c"user_data".as_ptr(), 9, ud.as_ptr());
        }

        let pos = tv_list_alloc(4);
        tv_dict_add_list(retdict, c"from".as_ptr(), 4, pos);

        let fmark = &raw mut tag.fmark;
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
pub fn get_tagstack(wp: &mut win_T, retdict: &mut dict_T) {
    unsafe {
        tv_dict_add_nr(
            retdict,
            c"length".as_ptr(),
            6,
            wp.w_tagstacklen as varnumber_T,
        );
        tv_dict_add_nr(
            retdict,
            c"curidx".as_ptr(),
            6,
            (wp.w_tagstackidx + 1) as varnumber_T,
        );
        let l = tv_list_alloc(2);
        tv_dict_add_list(retdict, c"items".as_ptr(), 5, l);

        let tagstack = (&raw mut wp.w_tagstack).cast::<taggy_T>();
        for i in 0..wp.w_tagstacklen as usize {
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            get_tag_details(&mut *tagstack.add(i), d);
        }
    }
}

fn tagstack_clear(wp: &mut win_T) {
    unsafe {
        let tagstack = (&raw mut wp.w_tagstack).cast::<taggy_T>();
        for i in 0..wp.w_tagstacklen as usize {
            tagstack_clear_entry(&mut *tagstack.add(i));
        }
        wp.w_tagstacklen = 0;
        wp.w_tagstackidx = 0;
    }
}

fn tagstack_shift(wp: &mut win_T) {
    let len = wp.w_tagstacklen as usize;
        (&mut wp.w_tagstack)[..len].rotate_left(1);
        tagstack_clear_entry(&mut wp.w_tagstack[len - 1]);
        wp.w_tagstacklen -= 1;
}

fn tagstack_push_item(
    wp: &mut win_T,
    tagname: Option<std::ffi::CString>,
    cur_fnum: c_int,
    cur_match: c_int,
    mark: pos_T,
    fnum: c_int,
    user_data: Option<std::ffi::CString>,
) {
    unsafe {
        let tagstack = (&raw mut wp.w_tagstack).cast::<taggy_T>();
        let mut idx = wp.w_tagstacklen;

        if idx >= TAGSTACKSIZE {
            tagstack_shift(wp);
            idx = TAGSTACKSIZE - 1;
        }

        wp.w_tagstacklen += 1;
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

fn tagstack_push_items(wp: *mut win_T, l: *mut list_T) {
    unsafe {
        let mut li = if l.is_null() {
            ptr::null_mut()
        } else {
            (*l).lv_first
        };
        while !li.is_null() {
            if (*li).li_tv.v_type == VarType::VAR_DICT && !(*li).li_tv.vval.v_dict().is_null() {
                let itemdict = (*li).li_tv.vval.v_dict();
                let di = tv_dict_find(itemdict, c"from".as_ptr(), -1);
                if !di.is_null() {
                    let mut mark = pos_T {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    };
                    let mut fnum: c_int = 0;
                    if list2fpos(
                        &mut (*di).di_tv,
                        &mut mark,
                        &raw mut fnum,
                        ptr::null_mut(),
                        false,
                    ) == OK
                    {
                        let tagname = tv_dict_get_string(itemdict, c"tagname".as_ptr());
                        if !tagname.is_null() {
                            if mark.col > 0 {
                                mark.col -= 1;
                            }
                            let ud_ptr = tv_dict_get_string(itemdict, c"user_data".as_ptr());
                            tagstack_push_item(
                                &mut *wp,
                                Some(CStr::from_ptr(tagname).to_owned()),
                                tv_dict_get_number(itemdict, c"bufnr".as_ptr()) as c_int,
                                tv_dict_get_number(itemdict, c"matchnr".as_ptr()) as c_int - 1,
                                mark,
                                fnum,
                                (!ud_ptr.is_null()).then(|| CStr::from_ptr(ud_ptr).to_owned()),
                            );
                        }
                    }
                }
            }
            li = (*li).li_next;
        }
    }
}

fn tagstack_set_curidx(wp: &mut win_T, curidx: c_int) {
    wp.w_tagstackidx = curidx.max(0).min(wp.w_tagstacklen);
}

/// Set the tag stack entries of the specified window.
pub fn set_tagstack(wp: &mut win_T, d: *const dict_T, action: c_int) -> c_int {
    unsafe {
        if tag_st().tfu_in_use {
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
            l = (*di_items).di_tv.vval.v_list();
        }

        let di_curidx = tv_dict_find(d, c"curidx".as_ptr(), -1);
        if !di_curidx.is_null() {
            tagstack_set_curidx(
                wp,
                tv_get_number(&raw const (*di_curidx).di_tv) as c_int - 1,
            );
        }

        if action == b't' as c_int {
            let tagstack = (&raw mut wp.w_tagstack).cast::<taggy_T>();
            let tagstackidx = wp.w_tagstackidx;
            let mut tagstacklen = wp.w_tagstacklen;

            while tagstackidx < tagstacklen {
                tagstacklen -= 1;
                tagstack_clear_entry(&mut *tagstack.add(tagstacklen as usize));
            }
            wp.w_tagstacklen = tagstacklen;
        }

        if !l.is_null() {
            if action == b'r' as c_int {
                tagstack_clear(wp);
            }

            tagstack_push_items(&raw mut *wp, l);
            wp.w_tagstackidx = wp.w_tagstacklen;
        }

        OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_strnicmp_compares_ascii_case_folded_to_uppercase() {
        assert_eq!(tag_strnicmp(c"abc".as_ptr(), c"ABC".as_ptr(), 3), 0);
            assert!(tag_strnicmp(c"abc".as_ptr(), c"_bc".as_ptr(), 3) < 0);
    }
}
