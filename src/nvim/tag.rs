//! Code to handle tags and the tag stack (`src/nvim/tag.c`).

use crate::autocmd::{EVENT_BUFREADCMD, has_autocmd};
use crate::buffer::{
    GETF_SETMARK, GETFILE_OPEN_OTHER, GETFILE_SAME_FILE, GETFILE_UNUSED, K_OPT_JOP_FLAG_VIEW,
    K_OPT_SWB_FLAG_NEWTAB, K_OPT_SWB_FLAG_VSPLIT, MAXLNUM, NOTDONE, VAR_FIXED, bt_help,
    buflist_findname_exp, buflist_findnr, buflist_getfile,
};
use crate::charset::{
    ascii_isalpha, ascii_isdigit, ascii_islower, ascii_iswhite, skipdigits, vim_isblankline,
};
use crate::cmdexpand::{
    EXPAND_FILES, HLF_D, TAG_MANY, WILD_EXPAND_FREE, WILD_LIST_NOTFOUND, WILD_SILENT, expand_one,
};
use crate::cursor::{CursorPos, Linenr, check_cursor};
use crate::drawscreen::{UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::typval::{
    K_CALLBACK_NONE_CONST, TvDict, TvList, TypVal, TypvalVval, VAR_UNLOCKED, VarType, Varnumber,
    callback_free, tv_clear, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_dict_alloc_lock, tv_dict_find, tv_dict_get_number,
    tv_dict_get_string, tv_get_number, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_number, tv_list_append_tv, tv_list_free,
};
use crate::eval::vars::{VV_SWAPCOMMAND, set_vim_var_string};
use crate::eval::{callback_call, list2fpos, set_ref_in_callback};
use crate::ex_cmds::{getfile, prepare_tagpreview};
use crate::ex_docmd::{
    DT_FIRST, DT_JUMP, DT_LAST, DT_LTAG, DT_NEXT, DT_POP, DT_PREV, DT_SELECT, DT_TAG, Exarg,
    do_cmdline_cmd, set_no_hlsearch,
};
use crate::file_search::{FINDFILE_FILE, vim_findfile, vim_findfile_init, vim_findfile_stopdir};
use crate::fileio::{FILE, Off, vim_fgets, vim_fseek, vim_ftell};
use crate::fold::fold_open_cursor;
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::help::{
    DT_HELP, TAG_HELP, TAG_KEEP_LANG, TAG_NAMES, TAG_NO_TAGFUNC, TAG_REGEXP, TAG_VERBOSE,
    help_heuristic,
};
use crate::insexpand::{TAG_INS_COMP, TAG_NOIC, ins_compl_check_keys, ins_compl_interrupted};
use crate::lua::executor::nlua_call_excmd;
use crate::mark::{Fmark, Fmarkv, fm_getname, mark_view_make, mark_view_restore, setpcmark};
use crate::mbyte::{Vimconv, convert_setup, mb_ptr_adv, mb_strnicmp, string_convert, utfc_ptr2len};
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
    FullName_save, K_PATH_CMP_EXPAND, K_PATH_CMP_FULL, PathCmpFlags, path_equal, path_has_wildcard,
    path_tail, simplify_filename, to_slash_save, vim_isAbsName,
};
use crate::quickfix::{CMDBUFFSIZE, K_OPT_SWB_FLAG_USETAB, set_errorlist};
use crate::regexp::{
    MAXCOL, NSUBEXP, RE_MAGIC, Regmatch, skip_regexp, vim_regcomp, vim_regexec, vim_regfree,
};
use crate::runtime::{DIP_ALL, RuntimeCallback, do_in_runtimepath};
use crate::search::{do_search, ignorecase, ignorecase_opt};
use crate::state::MODE_INSERT;
use crate::strings::{gettext, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::window::{
    WSP_VERT, WinInfoInfo, check_can_set_curbuf_forceit, swbuf_goto_win_with_buf, tabpage_index,
    win_close, win_enter, win_split, win_valid,
};
use std::ffi::{CStr, c_char};
use std::ptr;

#[repr(C)]
#[derive(Clone)]
pub struct Taggy {
    pub tagname: Option<std::ffi::CString>,
    pub fmark: Fmark,
    pub cur_match: i32,
    pub cur_fnum: i32,
    pub user_data: Option<std::ffi::CString>,
}

impl Taggy {
    pub const ZERO: Self = Self {
        tagname: None,
        fmark: Fmark::ZERO,
        cur_match: 0,
        cur_fnum: 0,
        user_data: None,
    };
}

const LSIZE: i32 = 512;
const TAGSTACKSIZE: i32 = 20;

const DT_FREE: i32 = 99;

const MT_ST_CUR: usize = 0;
const MT_GL_CUR: usize = 1;
const MT_GL_OTH: usize = 2;
const MT_ST_OTH: usize = 3;
const MT_IC_OFF: usize = 4;
const MT_RE_OFF: usize = 8;
const MT_MASK: usize = 7;
const MT_COUNT: usize = 16;

const NOTAGFILE: i32 = 99;
const TAG_SEP: c_char = 0x02;
const ML_EXTRA: usize = 3;

const K_CPO_TAGPAT: i32 = b't' as i32;
const K_OPT_FDO_FLAG_TAG: u32 = 0x80;
pub(crate) const K_OPT_SWB_FLAG_USEOPEN: u32 = 0x01;
const K_OPT_TC_FLAG_FOLLOWIC: u32 = 0x01;
const K_OPT_TC_FLAG_IGNORE: u32 = 0x02;
const K_OPT_TC_FLAG_MATCH: u32 = 0x04;
const K_OPT_TC_FLAG_FOLLOWSCS: u32 = 0x08;
const K_OPT_TC_FLAG_SMART: u32 = 0x10;

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
pub struct Tagname {
    pub tn_tags: Option<std::ffi::CString>,
    pub tn_np: *mut c_char,
    pub tn_did_filefind_init: i32,
    pub tn_hf_idx: i32,
    pub tn_search_ctx: Option<crate::file_search::FfSearchCtx>,
}

impl Tagname {
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

impl Default for Tagname {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Tagname {
    fn drop(&mut self) {
        tag_st().tag_fnames.clear();
    }
}

use crate::change::{HLF_W, TAB};
use crate::charset::NUL;
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::eval::funcs::{K_SPECIAL_VAR_NULL_CONST, SEARCH_KEEP};
use crate::fileio::{SEEK_END, SEEK_SET};
use crate::globals::IOSIZE;
use crate::math::{FAIL, OK};
use crate::os::env::ExpandCtx;
use crate::os::fs::MAXPATHL;
use crate::viml::parser::parser::CONV_NONE;

#[derive(Clone, Copy)]
struct Tagptrs {
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
    tagline: Linenr,
}

impl Default for Tagptrs {
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
struct Pat {
    pat: *mut c_char,
    len: i32,
    head: *mut c_char,
    headlen: i32,
    regmatch: Regmatch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tagcmd {
    TagcmdInvalid,
    TagcmdAddress,
    TagcmdGeneric,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum TagsReadStatus {
    TagsReadSuccess = 1,
    TagsReadEof = 2,
    TagsReadIgnore = 3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TagsearchState {
    TsStart,
    TsLinear,
    TsBinary,
    TsSkipBack,
    TsStepForward,
}

#[derive(Clone, Copy, Default)]
struct TagsearchInfo {
    low_offset: Off,
    high_offset: Off,
    curr_offset: Off,
    curr_offset_used: Off,
    match_offset: Off,
    low_char: i32,
    high_char: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum TagmatchStatus {
    TagMatchSuccess = 1,
    TagMatchFail = 2,
    TagMatchStop = 3,
    TagMatchNext = 4,
}

#[derive(Clone, Copy, Default)]
struct FindtagsMatchArgs {
    matchoff: i32,
    match_re: bool,
    match_no_ic: bool,
    has_re: bool,
    sortic: bool,
    sort_error: bool,
}

struct FindtagsState {
    state: TagsearchState,
    stop_searching: bool,
    orgpat: Pat,
    lbuf: Vec<c_char>,
    tag_fname: Vec<c_char>,
    fp: Option<crate::os::fs::CFile>,
    flags: i32,
    tag_file_sorted: i32,
    get_searchpat: bool,
    help_only: bool,
    did_open: bool,
    mincount: i32,
    linear: bool,
    vimconv: Vimconv,
    help_lang: [c_char; 3],
    help_pri: i32,
    help_lang_find: Option<[c_char; 3]>,
    is_txt: bool,
    match_count: i32,
    ga_match: [Vec<Vec<c_char>>; MT_COUNT],
    ht_match: [std::collections::HashSet<Vec<u8>>; MT_COUNT],
}

impl FindtagsState {
    fn new() -> Self {
        Self {
            state: TagsearchState::TsStart,
            stop_searching: false,
            orgpat: Pat {
                pat: ptr::null_mut(),
                len: 0,
                head: ptr::null_mut(),
                headlen: 0,
                regmatch: Regmatch {
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
            vimconv: Vimconv::new(),
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
    ptag_entry: Taggy,
    tfu_in_use: bool,
    tag_fnames: Vec<std::ffi::CString>,
    do_tag_max_num_matches: i32,
    do_tag_matches: Vec<Vec<c_char>>,
    do_tag_flags: i32,
    tag_listfiles_extra: Vec<([c_char; 2], std::ffi::CString)>,
}

impl TagModuleState {
    const fn new() -> Self {
        Self {
            nofile_fname: None,
            tagmatchname: None,
            ptag_entry: Taggy {
                tagname: None,
                fmark: Fmark {
                    mark: CursorPos {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    },
                    fnum: 0,
                    timestamp: 0,
                    view: Fmarkv {
                        topline_offset: MAXLNUM,
                        skipcol: 0,
                    },
                    additional_data: None,
                },
                cur_match: 0,
                cur_fnum: 0,
                user_data: None,
            },
            tfu_in_use: false,
            tag_fnames: Vec::new(),
            do_tag_max_num_matches: 0,
            do_tag_matches: Vec::new(),
            do_tag_flags: 0,
            tag_listfiles_extra: Vec::new(),
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
pub fn set_ref_in_tagfunc(copy_id: i32) -> bool {
    set_ref_in_callback(
        &mut opt_vars().p_tfu,
        copy_id,
        ptr::null_mut(),
        ptr::null_mut(),
    )
}

/// Jump to tag; handling of tag commands and tag stack.
pub fn do_tag(
    eap: Option<&Exarg>,
    tag: impl crate::strings::IntoCCharPtr,
    mut type_: i32,
    count: i32,
    forceit: i32,
    verbose: bool,
) {
    let tag = tag.into_c_char_ptr().cast_mut();
    unsafe {
        let tagstack: *mut Taggy = (&raw mut curwin_mut().w_tagstack).cast::<Taggy>();
        let mut tagstackidx = curwin_mut().w_tagstackidx;
        let mut tagstacklen = curwin_mut().w_tagstacklen;
        let mut cur_match: i32 = 0;
        let mut cur_fnum = curbuf_mut().b_fnum;
        let oldtagstackidx = tagstackidx;
        let mut prevtagstackidx = tagstackidx;
        let mut new_tag = false;
        let mut no_regexp = false;
        let mut error_cur_match: i32 = 0;
        let mut save_pos = false;
        let mut saved_fmark = Fmark {
            mark: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            fnum: 0,
            timestamp: 0,
            view: Fmarkv {
                topline_offset: MAXLNUM,
                skipcol: 0,
            },
            additional_data: None,
        };
        let use_tagstack: bool;
        let mut skip_msg = false;
        let mut buf_ffname = curbuf_mut().b_ffname;
        let mut use_tfu = true;
        let mut tofree: Option<std::ffi::CString> = None;
        if type_ == DT_FREE {
            tag_st().do_tag_matches.clear();
            return;
        }

        if tag_st().tfu_in_use {
            emsg(gettext(E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC));
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

        let prev_num_matches = tag_st().do_tag_matches.len() as i32;
        tag_st().nofile_fname = None;

        assert!(!tag.is_null());
        'end_do_tag: {
            if opt_vars().p_tgst == 0 && !crate::memory::cstr_is_empty(tag) {
                use_tagstack = false;
                new_tag = true;
                if g_st().g_do_tagpreview != 0 {
                    tagstack_clear_entry(&mut tag_st().ptag_entry);
                    tag_st().ptag_entry.tagname = Some(crate::memory::cstr_to_cstring(tag));
                }
            } else {
                use_tagstack = g_st().g_do_tagpreview == 0;

                // new pattern, add to the tag stack
                if !crate::memory::cstr_is_empty(tag)
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
                            .is_some_and(|t| {
                                crate::memory::cstr_as_bytes(t.as_ptr())
                                    == crate::memory::cstr_as_bytes(tag)
                            })
                        {
                            cur_match = tag_st().ptag_entry.cur_match;
                            cur_fnum = tag_st().ptag_entry.cur_fnum;
                        } else {
                            tagstack_clear_entry(&mut tag_st().ptag_entry);
                            tag_st().ptag_entry.tagname = Some(crate::memory::cstr_to_cstring(tag));
                        }
                    } else {
                        while tagstackidx < tagstacklen {
                            tagstacklen -= 1;
                            tagstack_clear_entry(
                                &mut curwin_mut().w_tagstack[tagstacklen as usize],
                            );
                        }

                        tagstacklen += 1;
                        if tagstacklen > TAGSTACKSIZE {
                            tagstacklen = TAGSTACKSIZE;
                            (&mut curwin_mut().w_tagstack)[..tagstacklen as usize].rotate_left(1);
                            tagstackidx -= 1;
                            tagstack_clear_entry(
                                &mut curwin_mut().w_tagstack[tagstackidx as usize],
                            );
                        }

                        curwin_mut().w_tagstack[tagstackidx as usize].tagname =
                            Some(crate::memory::cstr_to_cstring(tag));
                        curwin_mut().w_tagstacklen = tagstacklen;
                        save_pos = true;
                    }

                    new_tag = true;
                } else {
                    if if g_st().g_do_tagpreview != 0 {
                        tag_st().ptag_entry.tagname.is_none()
                    } else {
                        tagstacklen == 0
                    } {
                        emsg(gettext(E_TAG_STACK_EMPTY));
                        break 'end_do_tag;
                    }

                    if type_ == DT_POP {
                        let old_key_typed = g_st().key_typed;
                        tagstackidx -= count;
                        if tagstackidx < 0 {
                            emsg(gettext(E_AT_BOTTOM_OF_TAG_STACK));
                            if tagstackidx + count == 0 {
                                tagstackidx = 0;
                                break 'end_do_tag;
                            }
                            tagstackidx = 0;
                        } else if tagstackidx >= tagstacklen {
                            emsg(gettext(E_AT_TOP_OF_TAG_STACK));
                            break 'end_do_tag;
                        }

                        saved_fmark = curwin_mut().w_tagstack[tagstackidx as usize].fmark.clone();
                        if saved_fmark.fnum != curbuf_mut().b_fnum {
                            if !buflist_getfile(
                                saved_fmark.fnum,
                                saved_fmark.mark.lnum,
                                GETF_SETMARK,
                                forceit,
                            ) {
                                tagstackidx = oldtagstackidx;
                                break 'end_do_tag;
                            }
                            curwin_mut().w_cursor.lnum = saved_fmark.mark.lnum;
                        } else {
                            setpcmark();
                            curwin_mut().w_cursor.lnum = saved_fmark.mark.lnum;
                        }
                        curwin_mut().w_cursor.col = saved_fmark.mark.col;
                        curwin_mut().w_set_curswant = true;
                        if (opt_vars().jop_flags & K_OPT_JOP_FLAG_VIEW) != 0 {
                            mark_view_restore(Some(&saved_fmark));
                        }
                        check_cursor(curwin_mut());
                        if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_TAG) != 0 && old_key_typed {
                            fold_open_cursor();
                        }

                        tag_st().do_tag_matches.clear();
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
                                emsg(gettext(E_AT_TOP_OF_TAG_STACK));
                                save_pos = false;
                            } else if tagstackidx < 0 {
                                emsg(gettext(E_AT_BOTTOM_OF_TAG_STACK));
                                tagstackidx = 0;
                                break 'end_do_tag;
                            }
                            cur_match = curwin_mut().w_tagstack[tagstackidx as usize].cur_match;
                            cur_fnum = curwin_mut().w_tagstack[tagstackidx as usize].cur_fnum;
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
                            cur_match = curwin_mut().w_tagstack[tagstackidx as usize].cur_match;
                            cur_fnum = curwin_mut().w_tagstack[tagstackidx as usize].cur_fnum;
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
                            emsg(gettext(c"E425: Cannot go before first matching tag"));
                            skip_msg = true;
                            cur_match = 0;
                            cur_fnum = curbuf_mut().b_fnum;
                        }
                    }
                }

                if g_st().g_do_tagpreview != 0 {
                    if type_ != DT_SELECT && type_ != DT_JUMP {
                        tag_st().ptag_entry.cur_match = cur_match;
                        tag_st().ptag_entry.cur_fnum = cur_fnum;
                    }
                } else {
                    saved_fmark = curwin_mut().w_tagstack[tagstackidx as usize].fmark.clone();
                    if save_pos {
                        curwin_mut().w_tagstack[tagstackidx as usize].fmark.mark =
                            curwin_mut().w_cursor;
                        curwin_mut().w_tagstack[tagstackidx as usize].fmark.fnum =
                            curbuf_mut().b_fnum;
                        curwin_mut().w_tagstack[tagstackidx as usize].fmark.view =
                            mark_view_make(curwin_mut(), curwin_mut().w_cursor);
                    }

                    curwin_mut().w_tagstackidx = tagstackidx;
                    if type_ != DT_SELECT && type_ != DT_JUMP {
                        curwin_mut().w_tagstack[tagstackidx as usize].cur_match = cur_match;
                        curwin_mut().w_tagstack[tagstackidx as usize].cur_fnum = cur_fnum;
                    }
                }
            }

            if cur_fnum != curbuf_mut().b_fnum {
                let buf = buflist_findnr(cur_fnum);
                if !buf.is_null() {
                    buf_ffname = (*buf).b_ffname;
                }
            }

            loop {
                let mut name: *mut c_char = if use_tagstack {
                    tofree = curwin_mut().w_tagstack[tagstackidx as usize]
                        .tagname
                        .clone();
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
                    .is_none_or(|m| {
                        crate::memory::cstr_as_bytes(m.as_ptr())
                            != crate::memory::cstr_as_bytes(name)
                    });
                if new_tag
                    || (cur_match >= tag_st().do_tag_matches.len() as i32
                        && tag_st().do_tag_max_num_matches != MAXCOL)
                    || other_name
                {
                    if other_name {
                        tag_st().tagmatchname = Some(crate::memory::cstr_to_cstring(name));
                    }

                    if type_ == DT_SELECT || type_ == DT_JUMP || type_ == DT_LTAG {
                        cur_match = MAXCOL - 1;
                    }
                    tag_st().do_tag_max_num_matches = if type_ == DT_TAG {
                        MAXCOL
                    } else {
                        cur_match + 1
                    };

                    if !no_regexp && crate::memory::cstr_first_byte(name) == b'/' {
                        tag_st().do_tag_flags = TAG_REGEXP;
                        name = name.wrapping_add(1);
                    } else {
                        tag_st().do_tag_flags = TAG_NOIC;
                    }

                    if verbose {
                        tag_st().do_tag_flags |= TAG_VERBOSE;
                    }
                    if !use_tfu {
                        tag_st().do_tag_flags |= TAG_NO_TAGFUNC;
                    }

                    let mut new_matches: Vec<Vec<c_char>> = Vec::new();
                    if find_tags(
                        name,
                        &mut new_matches,
                        tag_st().do_tag_flags,
                        tag_st().do_tag_max_num_matches,
                        buf_ffname,
                    ) == OK
                        && (new_matches.len() as i32) < tag_st().do_tag_max_num_matches
                    {
                        tag_st().do_tag_max_num_matches = MAXCOL;
                    }

                    if tagstack != (&raw mut curwin_mut().w_tagstack).cast::<Taggy>() {
                        emsg(gettext(
                            E_WINDOW_UNEXPECTEDLY_CLOSE_WHILE_SEARCHING_FOR_TAGS,
                        ));
                        break;
                    }

                    if !new_tag && !other_name {
                        let mut idx: usize = 0;
                        let mut tagp = Tagptrs::default();
                        let mut tagp2 = Tagptrs::default();

                        for old_match in &tag_st().do_tag_matches {
                            parse_match(old_match.as_ptr().cast_mut(), &mut tagp);
                            for i in idx..new_matches.len() {
                                parse_match(new_matches[i].as_mut_ptr(), &mut tagp2);
                                if crate::memory::cstr_as_bytes(tagp.tagname)
                                    == crate::memory::cstr_as_bytes(tagp2.tagname)
                                {
                                    let p = new_matches.remove(i);
                                    new_matches.insert(idx, p);
                                    idx += 1;
                                    break;
                                }
                            }
                        }
                    }
                    tag_st().do_tag_matches = new_matches;
                }

                let do_tag_num_matches = tag_st().do_tag_matches.len() as i32;
                if do_tag_num_matches <= 0 {
                    if verbose {
                        semsg!(gettext(E_TAG_NOT_FOUND_STR), name);
                    }
                    g_st().g_do_tagpreview = 0;
                } else {
                    if type_ == DT_TAG && !crate::memory::cstr_is_empty(tag) {
                        cur_match = if count > 0 { count - 1 } else { 0 };
                    } else if type_ == DT_SELECT || (type_ == DT_JUMP && do_tag_num_matches > 1) {
                        select_tag_match(
                            (eap.is_some()).then(|| eap.unwrap()),
                            new_tag,
                            use_tagstack,
                            &tag_st().do_tag_matches,
                            name,
                        );
                        if use_tagstack {
                            curwin_mut().w_tagstack[tagstackidx as usize].fmark = saved_fmark;
                            tagstackidx = prevtagstackidx;
                        }
                        break;
                    } else if type_ == DT_LTAG {
                        if !add_llist_tags(tag, &tag_st().do_tag_matches) {
                            break 'end_do_tag;
                        }
                        cur_match = 0;
                    }

                    if cur_match >= do_tag_num_matches {
                        if (type_ == DT_NEXT || type_ == DT_FIRST)
                            && tag_st().nofile_fname.is_none()
                        {
                            if do_tag_num_matches == 1 {
                                emsg(gettext(c"E427: There is only one matching tag"));
                            } else {
                                emsg(gettext(c"E428: Cannot go beyond last matching tag"));
                            }
                            skip_msg = true;
                        }
                        cur_match = do_tag_num_matches - 1;
                    }
                    if use_tagstack {
                        let mut tagp2 = Tagptrs::default();

                        curwin_mut().w_tagstack[tagstackidx as usize].cur_match = cur_match;
                        curwin_mut().w_tagstack[tagstackidx as usize].cur_fnum = cur_fnum;

                        if use_tfu
                            && parse_match(
                                tag_st().do_tag_matches.as_slice()[cur_match as usize]
                                    .as_ptr()
                                    .cast_mut(),
                                &mut tagp2,
                            )
                            && !tagp2.user_data.is_null()
                        {
                            let ud_slice = crate::memory::cstr_slice(
                                tagp2.user_data,
                                crate::memory::ptr_offset_from(tagp2.user_data_end, tagp2.user_data)
                                    as usize,
                            );
                            curwin_mut().w_tagstack[tagstackidx as usize].user_data =
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
                        smsg!(0, gettext(c"File \"%s\" does not exist"), nf.as_ptr(),);
                    }

                    let cur_match_ptr = tag_st().do_tag_matches.as_slice()[cur_match as usize]
                        .as_ptr()
                        .cast_mut();
                    let ic = ((*cur_match_ptr as usize) & MT_IC_OFF) != 0;
                    if type_ != DT_TAG
                        && type_ != DT_SELECT
                        && type_ != DT_JUMP
                        && (do_tag_num_matches > 1 || ic)
                        && !skip_msg
                    {
                        let iobuf_ptr = g_st().iobuff.as_mut_ptr();
                        let more_str = if tag_st().do_tag_max_num_matches != MAXCOL {
                            gettext(c" or more").cast_const()
                        } else {
                            c"".as_ptr()
                        };
                        vim_snprintf!(
                            iobuf_ptr,
                            IOSIZE,
                            gettext(c"tag %d of %d%s"),
                            cur_match + 1,
                            do_tag_num_matches,
                            more_str,
                        );
                        if ic {
                            {
                                let _dst: *mut c_char = iobuf_ptr;
                                let _dsize: usize = IOSIZE;
                                let _dlen = crate::memory::cstr_len(_dst);
                                crate::memory::xstrlcpy(
                                    _dst.wrapping_add(_dlen),
                                    gettext(c"  Using tag with different case!"),
                                    _dsize - _dlen,
                                );
                            };
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

                    let iobuf_ptr = g_st().iobuff.as_mut_ptr();
                    let iobufflen = vim_snprintf_safelen!(iobuf_ptr, IOSIZE, c":ta %s\r", name);
                    set_vim_var_string(VV_SWAPCOMMAND, iobuf_ptr, iobufflen as isize);

                    let i = jumpto_tag(cur_match_ptr, forceit, true);

                    set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);

                    if i == NOTAGFILE {
                        if (type_ == DT_PREV && cur_match > 0)
                            || ((type_ == DT_TAG || type_ == DT_NEXT || type_ == DT_FIRST)
                                && (tag_st().do_tag_max_num_matches != MAXCOL
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
                            gettext(c"E429: File \"%s\" does not exist"),
                            (&raw const tag_st().nofile_fname)
                                .as_ref()
                                .unwrap()
                                .as_ref()
                                .map_or(c"".as_ptr(), |s| s.as_ptr()),
                        );
                    } else if use_tagstack && tagstackidx > curwin_mut().w_tagstacklen {
                        tagstackidx = curwin_mut().w_tagstackidx;
                    }
                }
                break;
            }
        }

        if use_tagstack && tagstackidx <= curwin_mut().w_tagstacklen {
            curwin_mut().w_tagstackidx = tagstackidx;
        }
        g_st().postponed_split = 0;
        g_st().g_do_tagpreview = 0;
        drop(tofree);
    }
}

/// Let the user pick from `matches`. Delegates to (async) `vim.ui.select()`.
fn select_tag_match(
    eap: Option<&Exarg>,
    new_tag: bool,
    use_tagstack: bool,
    matches: &[Vec<c_char>],
    name: impl crate::strings::IntoCCharPtr,
) {
    let name = name.into_c_char_ptr();
    {
        let tagstackidx = curwin_mut().w_tagstackidx;

        let mut items_tv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        tv_list_alloc_ret(&mut items_tv, matches.len() as isize);

        for (idx, m_vec) in matches.iter().enumerate() {
            let m = m_vec.as_ptr().cast_mut();
            let i = idx as i32;
            os_breakcheck();
            if g_st().got_int {
                tv_clear(&mut items_tv);
                return;
            }
            let mut tagp = Tagptrs::default();
            parse_match(m, &mut tagp);

            let cur = !new_tag
                && ((g_st().g_do_tagpreview != 0 && i == tag_st().ptag_entry.cur_match)
                    || (use_tagstack
                        && i == curwin_mut().w_tagstack[tagstackidx as usize].cur_match));

            let d = tv_dict_alloc();
            tv_dict_add_str_len(
                &mut *d,
                c"tag",
                tagp.tagname,
                crate::memory::ptr_offset_from(tagp.tagname_end, tagp.tagname) as i32,
            );
            let pri_idx = (unsafe { *m } as usize) & MT_MASK;
            tv_dict_add_str(&mut *d, c"pri", MT_NAMES[pri_idx].as_ptr());
            if !tagp.tagkind.is_null() {
                tv_dict_add_str_len(
                    &mut *d,
                    c"kind",
                    tagp.tagkind,
                    crate::memory::ptr_offset_from(tagp.tagkind_end, tagp.tagkind) as i32,
                );
            }
            let fname = tag_full_fname(&mut tagp);
            tv_dict_add_str(&mut *d, c"file", fname.as_ptr());
            tv_dict_add_bool(
                &mut *d,
                c"cur",
                if cur {
                    K_BOOL_VAR_TRUE_CONST
                } else {
                    K_BOOL_VAR_FALSE_CONST
                },
            );

            let mut item = TypVal {
                v_type: VarType::VarDict,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Dict(d),
            };
            tv_list_append_tv(unsafe { &mut *items_tv.vval.v_list() }, &mut item);
        }

        let extra_d = tv_dict_alloc();
        tv_dict_add_list(&mut *extra_d, c"items", items_tv.vval.v_list());
        unsafe {
            (*items_tv.vval.v_list()).lv_refcount += 1;
        }
        tv_dict_add_str(&mut *extra_d, c"tagname", name);
        let mut extra_tv = TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(extra_d),
        };

        nlua_call_excmd(
            c"vim._core.tag",
            c"select_tag",
            eap.unwrap(),
            &g_st().cmdmod,
            Some(&mut extra_tv),
        );

        tv_clear(&mut items_tv);
        tv_clear(&mut extra_tv);
    }
}

/// Add the matching tags to the location list for the current window.
fn add_llist_tags(tag: impl crate::strings::IntoCCharPtr, matches: &[Vec<c_char>]) -> bool {
    let tag = tag.into_c_char_ptr();
    unsafe {
        let mut tag_name = [0 as c_char; 128 + 1];
        let mut tagp = Tagptrs::default();

        let mut fname_vec = vec![0 as c_char; MAXPATHL + 1];
        let fname = fname_vec.as_mut_ptr();
        let mut cmd_vec = vec![0 as c_char; CMDBUFFSIZE + 1];
        let cmd = cmd_vec.as_mut_ptr();
        let list = tv_list_alloc(0);

        for m_vec in matches {
            let m = m_vec.as_ptr().cast_mut();
            parse_match(m, &mut tagp);

            let mut len =
                (crate::memory::ptr_offset_from(tagp.tagname_end, tagp.tagname) as i32).min(128);
            crate::memory::cstr_ncpy(tag_name.as_mut_ptr(), tagp.tagname, len as usize);
            tag_name[len as usize] = NUL;

            let p = tag_full_fname(&mut tagp);
            xstrlcpy(fname, p.as_ptr(), MAXPATHL);

            let mut lnum: Linenr = 0;
            if ascii_isdigit(*tagp.command) {
                lnum = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(tagp.command))
                    as i32;
            } else {
                let mut cmd_start = tagp.command;
                let mut cmd_end = tagp.command_end;
                if cmd_end.is_null() {
                    let mut p_iter = tagp.command;
                    while !crate::memory::cstr_is_empty(p_iter)
                        && crate::memory::cstr_first_byte(p_iter) != b'\r'
                        && crate::memory::cstr_first_byte(p_iter) != b'\n'
                    {
                        p_iter = p_iter.wrapping_add(1);
                    }
                    cmd_end = p_iter;
                }

                cmd_end = cmd_end.wrapping_sub(1);

                if crate::memory::cstr_first_byte(cmd_start) == b'/'
                    || crate::memory::cstr_first_byte(cmd_start) == b'?'
                {
                    cmd_start = cmd_start.wrapping_add(1);
                }
                if crate::memory::cstr_first_byte(cmd_end) == b'/'
                    || crate::memory::cstr_first_byte(cmd_end) == b'?'
                {
                    cmd_end = cmd_end.wrapping_sub(1);
                }

                len = 0;
                *cmd = NUL;

                if crate::memory::cstr_first_byte(cmd_start) == b'^' {
                    crate::memory::cstr_cpy(cmd, c"^");
                    cmd_start = cmd_start.wrapping_add(1);
                    len += 1;
                }

                crate::memory::cstr_cpy(cmd.wrapping_add(len as usize), c"\\V");
                len += 2;

                let cmd_len = ((crate::memory::ptr_offset_from(cmd_end, cmd_start) as i32) + 1)
                    .min(CMDBUFFSIZE as i32 - 5)
                    .max(0);
                if cmd_len > 0 {
                    ptr::copy_nonoverlapping(
                        cmd_start,
                        cmd.wrapping_add(len as usize),
                        cmd_len as usize,
                    );
                    len += cmd_len;
                }
                *cmd.wrapping_add(len as usize) = NUL;

                if crate::memory::cstr_first_byte(cmd.wrapping_add((len - 1) as usize)) == b'$' {
                    *cmd.wrapping_add((len - 1) as usize) = b'\\' as c_char;
                    *cmd.wrapping_add(len as usize) = b'$' as c_char;
                    len += 1;
                }

                *cmd.wrapping_add(len as usize) = NUL;
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(&mut *list, dict);

            tv_dict_add_str(&mut *dict, c"text", tag_name.as_ptr());
            tv_dict_add_str(&mut *dict, c"filename", fname);
            tv_dict_add_nr(&mut *dict, c"lnum", lnum as Varnumber);
            if lnum == 0 {
                tv_dict_add_str(&mut *dict, c"pattern", cmd);
            }
        }

        let iobuf_ptr = g_st().iobuff.as_mut_ptr();
        vim_snprintf!(iobuf_ptr, IOSIZE, c"ltag %s", tag);
        set_errorlist(curwin_ptr(), Some(&*list), b' ' as i32, iobuf_ptr, None);

        tv_list_free(list);

        true
    }
}

/// Free cached tags.
pub fn tag_freematch() {
    tag_st().tagmatchname = None;
}

/// Print the tag stack.
pub fn do_tags(_eap: &mut Exarg) {
    {
        let tagstackidx = curwin_mut().w_tagstackidx;
        let tagstacklen = curwin_mut().w_tagstacklen;

        msg_puts_title(gettext(c"\n  # TO tag         FROM line  in file/text"));
        for i in 0..tagstacklen {
            let entry = (&raw mut curwin_mut().w_tagstack[i as usize]);
            let entry = unsafe { &mut *entry };
            if let Some(ref tagname) = entry.tagname {
                let Some(name) = fm_getname(&mut entry.fmark, 30) else {
                    continue;
                };

                msg_putchar(b'\n' as i32);
                let iobuf_ptr = g_st().iobuff.as_mut_ptr();
                vim_snprintf!(
                    iobuf_ptr,
                    IOSIZE,
                    c"%c%2d %2d %-15s %5d  ",
                    if i == tagstackidx {
                        b'>' as i32
                    } else {
                        b' ' as i32
                    },
                    i + 1,
                    entry.cur_match + 1,
                    tagname.as_ptr(),
                    entry.fmark.mark.lnum,
                );
                msg_outtrans(iobuf_ptr, 0, false);
                msg_outtrans(
                    name.as_ptr(),
                    if entry.fmark.fnum == curbuf_mut().b_fnum {
                        HLF_D
                    } else {
                        0
                    },
                    false,
                );
            }
        }
        if tagstackidx == tagstacklen {
            msg_puts(c"\n>");
        }
    }
}

fn tag_strnicmp(
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
    mut len: usize,
) -> i32 {
    let mut s1 = s1.into_c_char_ptr();
    let mut s2 = s2.into_c_char_ptr();
    {
        while len > 0 {
            let i = toupper_asc(i32::from(crate::memory::cstr_first_byte(s1)))
                - toupper_asc(i32::from(crate::memory::cstr_first_byte(s2)));
            if i != 0 {
                return i;
            }
            if crate::memory::cstr_is_empty(s1) {
                break;
            }
            s1 = s1.wrapping_add(1);
            s2 = s2.wrapping_add(1);
            len -= 1;
        }
        0
    }
}

fn prepare_pats(pats: &mut Pat, has_re: bool) {
    {
        pats.head = pats.pat;
        pats.headlen = pats.len;
        if has_re {
            if crate::memory::cstr_first_byte(pats.pat) == b'^' {
                pats.head = pats.pat.wrapping_add(1);
            } else if crate::memory::cstr_first_byte(pats.pat) == b'\\'
                && crate::memory::cstr_byte_at(pats.pat, 1) == b'<'
            {
                pats.head = pats.pat.wrapping_add(2);
            }
            if pats.head == pats.pat {
                pats.headlen = 0;
            } else {
                pats.headlen = 0;
                while crate::memory::cstr_byte_at(pats.head, pats.headlen as usize) != 0 {
                    let magic_chars = if opt_vars().p_magic != 0 {
                        c".[~*\\$".as_ptr()
                    } else {
                        c"\\$".as_ptr()
                    };
                    if crate::strings::vim_haschr(
                        magic_chars,
                        crate::memory::cstr_byte_at(pats.head, pats.headlen as usize) as i32,
                    ) {
                        break;
                    }
                    pats.headlen += 1;
                }
            }
            if opt_vars().p_tl != 0 && (pats.headlen as OptInt) > opt_vars().p_tl {
                pats.headlen = opt_vars().p_tl as i32;
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

fn tagfunc_cmd_kind(cmd: impl crate::strings::IntoCCharPtr) -> Tagcmd {
    let cmd = cmd.into_c_char_ptr().cast_mut();
    {
        if ascii_isdigit(unsafe { *cmd }) {
            return if crate::memory::cstr_is_empty(skipdigits(cmd)) {
                Tagcmd::TagcmdAddress
            } else {
                Tagcmd::TagcmdInvalid
            };
        }
        if crate::memory::cstr_first_byte(cmd) == b'/'
            || crate::memory::cstr_first_byte(cmd) == b'?'
        {
            let end = skip_regexp(cmd.wrapping_add(1), unsafe { *cmd } as i32, 0);
            return if unsafe { *end } == unsafe { *cmd } && crate::memory::cstr_byte_at(end, 1) == 0
            {
                Tagcmd::TagcmdAddress
            } else {
                Tagcmd::TagcmdInvalid
            };
        }
        if crate::memory::cstr_as_bytes(cmd)
            .iter()
            .any(|b| b"\t\r\n".contains(b))
        {
            return Tagcmd::TagcmdInvalid;
        }
        Tagcmd::TagcmdGeneric
    }
}

fn find_tagfunc_tags(
    pat: impl crate::strings::IntoCCharPtr,
    ga: &mut Vec<Vec<c_char>>,
    match_count: &mut i32,
    flags: i32,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let pat = pat.into_c_char_ptr().cast_mut();
    let buf_ffname = buf_ffname.into_c_char_ptr();
    unsafe {
        let mut ntags: i32 = 0;
        let mut args = [TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        }; 4];
        let mut rettv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        let mut flag_string = [0 as c_char; 4];
        let mut tag: *mut Taggy = ptr::null_mut();

        if curwin_mut().w_tagstacklen > 0 {
            let tagstack_ptr = (&raw mut curwin_mut().w_tagstack).cast::<Taggy>();
            if curwin_mut().w_tagstackidx == curwin_mut().w_tagstacklen {
                tag = tagstack_ptr.wrapping_add((curwin_mut().w_tagstackidx - 1) as usize);
            } else {
                tag = tagstack_ptr.wrapping_add(curwin_mut().w_tagstackidx as usize);
            }
        }

        if curbuf_mut().b_p_tfu.cb_type() == K_CALLBACK_NONE_CONST {
            return FAIL;
        }

        args[0].v_type = VarType::VarString;
        args[0].vval = crate::eval::typval::TypvalVval::String(pat);
        args[1].v_type = VarType::VarString;
        args[1].vval = crate::eval::typval::TypvalVval::String(flag_string.as_mut_ptr());

        let d = tv_dict_alloc_lock(VAR_FIXED);
        if (flags & TAG_INS_COMP) == 0
            && !tag.is_null()
            && let Some(ref ud) = (*tag).user_data
        {
            tv_dict_add_str(&mut *d, c"user_data", ud.as_ptr());
        }
        if !buf_ffname.is_null() {
            tv_dict_add_str(&mut *d, c"buf_ffname", buf_ffname);
        }

        d.dv_refcount += 1;
        args[2].v_type = VarType::VarDict;
        args[2].vval = crate::eval::typval::TypvalVval::Dict(d);
        args[3].v_type = VarType::VarUnknown;

        vim_snprintf!(
            flag_string.as_mut_ptr(),
            flag_string.len(),
            c"%s%s%s",
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

        let save_pos = curwin_mut().w_cursor;
        let mut result =
            if callback_call(&mut curbuf_mut().b_p_tfu, 3, args.as_mut_ptr(), &mut rettv) {
                OK
            } else {
                FAIL
            };
        curwin_mut().w_cursor = save_pos;
        check_cursor(curwin_mut());
        d.dv_refcount -= 1;

        if result == FAIL {
            return FAIL;
        }
        if rettv.v_type == VarType::VarSpecial && rettv.vval.v_special() == K_SPECIAL_VAR_NULL_CONST
        {
            tv_clear(&mut rettv);
            return NOTDONE;
        }
        if rettv.v_type != VarType::VarList || rettv.vval.v_list().is_null() {
            tv_clear(&mut rettv);
            emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
            return FAIL;
        }
        let taglist = rettv.vval.v_list();

        for li in crate::eval::typval::list_iter_mut((*taglist).lv_first) {
            let mut res_name: *mut c_char = ptr::null_mut();
            let mut res_fname: *mut c_char = ptr::null_mut();
            let mut res_cmd: *mut c_char = ptr::null_mut();
            let mut res_kind: *mut c_char = ptr::null_mut();
            let mut has_extra = false;
            let name_only = (flags & TAG_NAMES) != 0;

            if li.li_tv.v_type != VarType::VarDict {
                emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
                break;
            }

            let mut len: usize = 2;
            let item_dict = li.li_tv.vval.v_dict();
            if !item_dict.is_null() {
                let mut todo = (*item_dict).dv_hashtab.ht_used;
                let mut hi = (*item_dict).dv_hashtab.array_ptr();
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        todo -= 1;
                        let di = crate::eval::typval::tv_dict_hi2di(hi);
                        let dict_key = (&*hi).key_ptr();
                        let tv = &raw mut di.di_tv;
                        let tv = { &mut *tv };

                        if tv.v_type == VarType::VarString && !tv.vval.v_string().is_null() {
                            len += crate::memory::cstr_len(tv.vval.v_string()) + 1;
                            if crate::memory::cstr_as_bytes(dict_key) == b"name" {
                                res_name = tv.vval.v_string();
                            } else if crate::memory::cstr_as_bytes(dict_key) == b"filename" {
                                res_fname = tv.vval.v_string();
                            } else if crate::memory::cstr_as_bytes(dict_key) == b"cmd" {
                                res_cmd = tv.vval.v_string();
                            } else {
                                has_extra = true;
                                if crate::memory::cstr_as_bytes(dict_key) == b"kind" {
                                    res_kind = tv.vval.v_string();
                                } else {
                                    len += crate::memory::cstr_len(dict_key) + 1;
                                }
                            }
                        }
                    }
                    hi = hi.wrapping_add(1);
                }
            }

            if has_extra {
                len += 2;
            }

            if res_name.is_null() || res_fname.is_null() || res_cmd.is_null() {
                emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
                break;
            }

            let cmdkind = tagfunc_cmd_kind(res_cmd);
            if cmdkind == Tagcmd::TagcmdInvalid {
                emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
                break;
            }
            if cmdkind == Tagcmd::TagcmdGeneric {
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
                p = p.wrapping_add(1);
                *p = TAG_SEP;
                p = p.wrapping_add(1);

                crate::memory::cstr_cpy(p, res_name);
                p = p.wrapping_add(crate::memory::cstr_len(p));

                *p = TAB;
                p = p.wrapping_add(1);
                crate::memory::cstr_cpy(p, res_fname);
                p = p.wrapping_add(crate::memory::cstr_len(p));

                *p = TAB;
                p = p.wrapping_add(1);
                crate::memory::cstr_cpy(p, res_cmd);
                p = p.wrapping_add(crate::memory::cstr_len(p));

                if cmdkind == Tagcmd::TagcmdGeneric {
                    *p = b'|' as c_char;
                    p = p.wrapping_add(1);
                }
                if cmdkind == Tagcmd::TagcmdGeneric || has_extra {
                    {
                        let s = c";\"".as_ptr();
                        std::ptr::copy_nonoverlapping(
                            s,
                            p,
                            crate::memory::cstr_from_ptr(s).to_bytes_with_nul().len(),
                        );
                    };
                    p = p.wrapping_add(crate::memory::cstr_len(p));

                    if !res_kind.is_null() {
                        *p = TAB;
                        p = p.wrapping_add(1);
                        crate::memory::cstr_cpy(p, res_kind);
                        p = p.wrapping_add(crate::memory::cstr_len(p));
                    }

                    if !item_dict.is_null() {
                        let item_dict = { &mut *item_dict };
                        let mut todo = item_dict.dv_hashtab.ht_used;
                        let mut hi = item_dict.dv_hashtab.array_ptr();
                        while todo > 0 {
                            if !hashitem_empty(&*hi) {
                                todo -= 1;
                                let di = crate::eval::typval::tv_dict_hi2di(hi);
                                let dict_key = (&*hi).key_ptr();
                                let tv = &mut di.di_tv;
                                if tv.v_type == VarType::VarString
                                    && !tv.vval.v_string().is_null()
                                    && crate::memory::cstr_as_bytes(dict_key) != b"name"
                                    && crate::memory::cstr_as_bytes(dict_key) != b"filename"
                                    && crate::memory::cstr_as_bytes(dict_key) != b"cmd"
                                    && crate::memory::cstr_as_bytes(dict_key) != b"kind"
                                {
                                    *p = TAB;
                                    p = p.wrapping_add(1);
                                    crate::memory::cstr_cpy(p, dict_key);
                                    p = p.wrapping_add(crate::memory::cstr_len(p));
                                    crate::memory::cstr_cpy(p, c":");
                                    p = p.wrapping_add(crate::memory::cstr_len(p));
                                    crate::memory::cstr_cpy(p, tv.vval.v_string());
                                    p = p.wrapping_add(crate::memory::cstr_len(p));
                                }
                            }
                            hi = hi.wrapping_add(1);
                        }
                    }
                }
            }

            ga.push(mfp_vec);
            ntags += 1;
            result = OK;
        }

        tv_clear(&mut rettv);

        *match_count = ntags;
        result
    }
}

impl FindtagsState {
    #[inline]
    fn fp_ptr(&self) -> *mut FILE {
        self.fp.as_ref().map_or(ptr::null_mut(), |f| f.as_ptr())
    }
}

fn findtags_state_init(
    st: &mut FindtagsState,
    pat: impl crate::strings::IntoCCharPtr,
    flags: i32,
    mincount: i32,
) {
    let pat = pat.into_c_char_ptr().cast_mut();
    {
        st.tag_fname = vec![0 as c_char; MAXPATHL + 1];
        st.fp = None;
        st.orgpat = Pat {
            pat,
            len: crate::memory::cstr_len(pat) as i32,
            head: ptr::null_mut(),
            headlen: 0,
            regmatch: Regmatch {
                regprog: ptr::null_mut(),
                startp: [ptr::null_mut(); NSUBEXP],
                endp: [ptr::null_mut(); NSUBEXP],
                rm_matchcol: 0,
                rm_ic: false,
            },
        };
        st.flags = flags;
        st.tag_file_sorted = NUL as i32;
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

impl Drop for FindtagsState {
    fn drop(&mut self) {
        vim_regfree(self.orgpat.regmatch.regprog);
    }
}

fn findtags_in_help_init(st: &mut FindtagsState) -> bool {
    {
        let tag_fname = st.tag_fname.as_ptr();
        if st.is_txt {
            crate::memory::cstr_cpy(st.help_lang.as_mut_ptr(), c"en");
        } else {
            let i = crate::memory::cstr_len(tag_fname) as i32;
            if i > 3 && unsafe { *tag_fname.wrapping_add((i - 3) as usize) } == b'-' as c_char {
                crate::memory::cstr_ncpy(
                    st.help_lang.as_mut_ptr(),
                    tag_fname.wrapping_add((i - 2) as usize),
                    2,
                );
            } else {
                crate::memory::cstr_cpy(st.help_lang.as_mut_ptr(), c"en");
            }
        }

        if let Some(ref lang_find) = st.help_lang_find
            && mb_strnicmp(st.help_lang.as_ptr(), lang_find.as_ptr(), MAXCOL as usize) != 0
        {
            return false;
        }

        let fname_len = if !curbuf_mut().b_fname.is_null() {
            crate::memory::cstr_len(curbuf_mut().b_fname) as i32
        } else {
            0
        };
        if (st.flags & TAG_KEEP_LANG) != 0
            && st.help_lang_find.is_none()
            && !curbuf_mut().b_fname.is_null()
            && fname_len > 4
            && unsafe { *curbuf_mut().b_fname.wrapping_add((fname_len - 1) as usize) }
                == b'x' as c_char
            && unsafe { *curbuf_mut().b_fname.wrapping_add((fname_len - 4) as usize) }
                == b'.' as c_char
            && mb_strnicmp(
                curbuf_mut().b_fname.wrapping_add((fname_len - 3) as usize),
                st.help_lang.as_ptr(),
                2,
            ) == 0
        {
            st.help_pri = 0;
        } else {
            st.help_pri = 1;
            let mut s = opt_vars().p_hlg;
            while !crate::memory::cstr_is_empty(s) {
                if mb_strnicmp(s, st.help_lang.as_ptr(), 2) == 0 {
                    break;
                }
                st.help_pri += 1;
                s = vim_strchr(s, b',' as i32);
                if s.is_null() {
                    break;
                }
                s = s.wrapping_add(1);
            }
            if crate::memory::cstr_is_empty(s) {
                st.help_pri += 1;
                if mb_strnicmp(st.help_lang.as_ptr(), c"en", MAXCOL as usize) != 0 {
                    st.help_pri += 1;
                }
            }
        }

        true
    }
}

fn findtags_apply_tfu(
    st: &mut FindtagsState,
    pat: impl crate::strings::IntoCCharPtr,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let pat = pat.into_c_char_ptr().cast_mut();
    let buf_ffname = buf_ffname.into_c_char_ptr();
    {
        let use_tfu = (st.flags & TAG_NO_TAGFUNC) == 0;

        if !use_tfu
            || tag_st().tfu_in_use
            || curbuf_mut().b_p_tfu.cb_type() == K_CALLBACK_NONE_CONST
        {
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

fn findtags_get_next_line(st: &mut FindtagsState, sinfo_p: &mut TagsearchInfo) -> TagsReadStatus {
    unsafe {
        let mut eof: bool;

        if st.state == TagsearchState::TsBinary {
            let offset = sinfo_p.low_offset + ((sinfo_p.high_offset - sinfo_p.low_offset) / 2);
            if offset == sinfo_p.curr_offset {
                return TagsReadStatus::TagsReadEof;
            } else {
                sinfo_p.curr_offset = offset;
            }
        } else if st.state == TagsearchState::TsSkipBack {
            let lbuf_size = st.lbuf.len() as i32;
            sinfo_p.curr_offset -= (lbuf_size * 2) as Off;
            if sinfo_p.curr_offset < 0 {
                sinfo_p.curr_offset = 0;
                vim_fseek(st.fp_ptr(), 0, SEEK_SET);
                st.state = TagsearchState::TsStepForward;
            }
        }

        let lbuf_ptr = st.lbuf.as_mut_ptr();
        let lbuf_size = st.lbuf.len() as i32;
        let fp = st.fp_ptr();
        if st.state == TagsearchState::TsBinary || st.state == TagsearchState::TsSkipBack {
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
                st.state = TagsearchState::TsSkipBack;
                sinfo_p.match_offset = vim_ftell(fp);
                sinfo_p.curr_offset = sinfo_p.curr_offset_used;
                return TagsReadStatus::TagsReadIgnore;
            }
        } else {
            loop {
                eof = vim_fgets(lbuf_ptr, lbuf_size, fp);
                if eof || !vim_isblankline(lbuf_ptr) {
                    break;
                }
            }
            if eof {
                return TagsReadStatus::TagsReadEof;
            }
        }

        TagsReadStatus::TagsReadSuccess
    }
}

fn findtags_hdr_parse(st: &mut FindtagsState) -> bool {
    {
        let lbuf = st.lbuf.as_mut_ptr();
        if !(crate::memory::cstr_prefix_bytes(lbuf, 6) == &b"!_TAG_"[..6]) {
            return true;
        }

        if crate::memory::cstr_prefix_bytes(lbuf, 18) == &b"!_TAG_FILE_SORTED\t"[..18] {
            st.tag_file_sorted = crate::memory::cstr_byte_at(lbuf, 18) as i32;
        }
        if crate::memory::cstr_prefix_bytes(lbuf, 20) == &b"!_TAG_FILE_ENCODING\t"[..20] {
            let mut p = lbuf.wrapping_add(20);
            while (crate::memory::cstr_first_byte(p)) > b' '
                && (crate::memory::cstr_first_byte(p)) < 127
            {
                p = p.wrapping_add(1);
            }
            unsafe {
                *p = NUL;
            }
            convert_setup(&mut st.vimconv, lbuf.wrapping_add(20), opt_vars().p_enc);
        }

        false
    }
}

fn findtags_start_state_handler(
    st: &mut FindtagsState,
    sortic: &mut bool,
    sinfo_p: &mut TagsearchInfo,
) -> bool {
    {
        let noic = (st.flags & TAG_NOIC) != 0;
        let lbuf = st.lbuf.as_ptr();

        if (crate::memory::cstr_prefix_bytes(lbuf, 6) <= b"!_TAG_")
            || (crate::memory::cstr_first_byte(lbuf) == b'!'
                && ascii_islower(crate::memory::cstr_char_at(lbuf, 1)))
        {
            return findtags_hdr_parse(st);
        }

        if st.linear {
            st.state = TagsearchState::TsLinear;
        } else if st.tag_file_sorted == NUL as i32 || st.tag_file_sorted == b'1' as i32 {
            st.state = TagsearchState::TsBinary;
        } else if st.tag_file_sorted == b'2' as i32 {
            st.state = TagsearchState::TsBinary;
            *sortic = true;
            st.orgpat.regmatch.rm_ic = opt_vars().p_ic != 0 || !noic;
        } else {
            st.state = TagsearchState::TsLinear;
        }

        if st.state == TagsearchState::TsBinary && st.orgpat.regmatch.rm_ic && !*sortic {
            st.linear = true;
            st.state = TagsearchState::TsLinear;
        }

        if st.state == TagsearchState::TsBinary {
            let fp = st.fp_ptr();
            if unsafe { vim_fseek(fp, 0, SEEK_END) } != 0 {
                st.state = TagsearchState::TsLinear;
            } else {
                let filesize = unsafe { vim_ftell(fp) };
                g_st().vim_ignored = unsafe { vim_fseek(fp, 0, SEEK_SET) };

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
    st: &mut FindtagsState,
    tagpp: &mut Tagptrs,
    margs: &mut FindtagsMatchArgs,
    sinfo_p: &mut TagsearchInfo,
) -> TagmatchStatus {
    {
        let status: bool;
        let lbuf = st.lbuf.as_mut_ptr();

        if st.orgpat.headlen != 0 {
            *tagpp = Tagptrs::default();
            tagpp.tagname = lbuf;
            tagpp.tagname_end = vim_strchr(lbuf, TAB as i32);
            if tagpp.tagname_end.is_null() {
                return TagmatchStatus::TagMatchFail;
            }

            let mut cmplen =
                crate::memory::ptr_offset_from(tagpp.tagname_end, tagpp.tagname) as i32;
            if opt_vars().p_tl != 0 && (cmplen as OptInt) > opt_vars().p_tl {
                cmplen = opt_vars().p_tl as i32;
            }
            if (st.flags & TAG_REGEXP) != 0 && st.orgpat.headlen < cmplen {
                cmplen = st.orgpat.headlen;
            } else if st.state == TagsearchState::TsLinear && st.orgpat.headlen != cmplen {
                return TagmatchStatus::TagMatchNext;
            }

            if st.state == TagsearchState::TsBinary {
                let mut i = crate::memory::cstr_first_byte(tagpp.tagname) as i32;
                if margs.sortic {
                    i = toupper_asc(i32::from(crate::memory::cstr_first_byte(tagpp.tagname)));
                }
                if i < sinfo_p.low_char || i > sinfo_p.high_char {
                    margs.sort_error = true;
                }

                let mut tagcmp = if margs.sortic {
                    tag_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize)
                } else {
                    crate::memory::cstr_prefix_bytes(tagpp.tagname, cmplen as usize).cmp(
                        crate::memory::cstr_prefix_bytes(st.orgpat.head, cmplen as usize),
                    ) as i32
                };

                if tagcmp == 0 {
                    if cmplen < st.orgpat.headlen {
                        tagcmp = -1;
                    } else if cmplen > st.orgpat.headlen {
                        tagcmp = 1;
                    }
                }

                if tagcmp == 0 {
                    st.state = TagsearchState::TsSkipBack;
                    sinfo_p.match_offset = sinfo_p.curr_offset;
                    return TagmatchStatus::TagMatchNext;
                }
                if tagcmp < 0 {
                    sinfo_p.curr_offset = unsafe { vim_ftell(st.fp_ptr()) };
                    if sinfo_p.curr_offset < sinfo_p.high_offset {
                        sinfo_p.low_offset = sinfo_p.curr_offset;
                        if margs.sortic {
                            sinfo_p.low_char = toupper_asc(i32::from(
                                crate::memory::cstr_first_byte(tagpp.tagname),
                            ));
                        } else {
                            sinfo_p.low_char = crate::memory::cstr_first_byte(tagpp.tagname) as i32;
                        }
                        return TagmatchStatus::TagMatchNext;
                    }
                }
                if tagcmp > 0 && sinfo_p.curr_offset != sinfo_p.high_offset {
                    sinfo_p.high_offset = sinfo_p.curr_offset;
                    if margs.sortic {
                        sinfo_p.high_char =
                            toupper_asc(i32::from(crate::memory::cstr_first_byte(tagpp.tagname)));
                    } else {
                        sinfo_p.high_char = crate::memory::cstr_first_byte(tagpp.tagname) as i32;
                    }
                    return TagmatchStatus::TagMatchNext;
                }

                return TagmatchStatus::TagMatchStop;
            } else if st.state == TagsearchState::TsSkipBack {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize) != 0 {
                    st.state = TagsearchState::TsStepForward;
                } else {
                    sinfo_p.curr_offset = sinfo_p.curr_offset_used;
                }
                return TagmatchStatus::TagMatchNext;
            } else if st.state == TagsearchState::TsStepForward {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize) != 0 {
                    return if unsafe { vim_ftell(st.fp_ptr()) } > sinfo_p.match_offset {
                        TagmatchStatus::TagMatchStop
                    } else {
                        TagmatchStatus::TagMatchNext
                    };
                }
            } else {
                assert!(cmplen >= 0);
                if mb_strnicmp(tagpp.tagname, st.orgpat.head, cmplen as usize) != 0 {
                    return TagmatchStatus::TagMatchNext;
                }
            }

            tagpp.fname = tagpp.tagname_end.wrapping_add(1);
            tagpp.fname_end = vim_strchr(tagpp.fname, TAB as i32);
            if tagpp.fname_end.is_null() {
                status = false;
            } else {
                tagpp.command = tagpp.fname_end.wrapping_add(1);
                status = true;
            }
        } else {
            status = parse_tag_line(lbuf, tagpp);
        }

        if !status {
            TagmatchStatus::TagMatchFail
        } else {
            TagmatchStatus::TagMatchSuccess
        }
    }
}

fn findtags_matchargs_init(margs: &mut FindtagsMatchArgs, flags: i32) {
    margs.matchoff = 0;
    margs.match_re = false;
    margs.match_no_ic = false;
    margs.has_re = (flags & TAG_REGEXP) != 0;
    margs.sortic = false;
    margs.sort_error = false;
}

fn findtags_match_tag(
    st: &mut FindtagsState,
    tagpp: &mut Tagptrs,
    margs: &mut FindtagsMatchArgs,
) -> bool {
    {
        let mut match_: bool;

        let mut cmplen = crate::memory::ptr_offset_from(tagpp.tagname_end, tagpp.tagname) as i32;
        if opt_vars().p_tl != 0 && (cmplen as OptInt) > opt_vars().p_tl {
            cmplen = opt_vars().p_tl as i32;
        }
        if st.orgpat.len != cmplen {
            match_ = false;
        } else if st.orgpat.regmatch.rm_ic {
            assert!(cmplen >= 0);
            match_ = mb_strnicmp(tagpp.tagname, st.orgpat.pat, cmplen as usize) == 0;
            if match_ {
                margs.match_no_ic =
                    crate::memory::cstr_prefix_bytes(tagpp.tagname, cmplen as usize)
                        == crate::memory::cstr_prefix_bytes(st.orgpat.pat, cmplen as usize);
            }
        } else {
            match_ = crate::memory::cstr_prefix_bytes(tagpp.tagname, cmplen as usize)
                == crate::memory::cstr_prefix_bytes(st.orgpat.pat, cmplen as usize);
        }

        margs.match_re = false;
        if !match_ && !st.orgpat.regmatch.regprog.is_null() {
            let cc = unsafe { *tagpp.tagname_end };
            unsafe {
                *tagpp.tagname_end = NUL;
            }
            match_ = vim_regexec(&mut st.orgpat.regmatch, tagpp.tagname, 0);
            if match_ {
                margs.matchoff =
                    crate::memory::ptr_offset_from(st.orgpat.regmatch.startp[0], tagpp.tagname)
                        as i32;
                if st.orgpat.regmatch.rm_ic {
                    st.orgpat.regmatch.rm_ic = false;
                    margs.match_no_ic = vim_regexec(&mut st.orgpat.regmatch, tagpp.tagname, 0);
                    st.orgpat.regmatch.rm_ic = true;
                }
            }
            unsafe {
                *tagpp.tagname_end = cc;
            }
            margs.match_re = true;
        }

        match_
    }
}

fn findtags_string_convert(st: &mut FindtagsState) {
    {
        let Some(conv_line) = string_convert(&st.vimconv, st.lbuf.as_mut_ptr(), None) else {
            return;
        };

        let len = conv_line.len();
        if len > st.lbuf.len() {
            st.lbuf.resize(len, 0);
        }
        crate::memory::cstr_cpy(st.lbuf.as_mut_ptr(), conv_line.as_ptr().cast());
    }
}

fn findtags_add_match(
    st: &mut FindtagsState,
    tagpp: &mut Tagptrs,
    margs: &FindtagsMatchArgs,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) {
    let buf_ffname = buf_ffname.into_c_char_ptr();
    {
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
            unsafe {
                *tagpp.tagname_end = NUL;
            }
            let len = crate::memory::ptr_offset_from(tagpp.tagname_end, tagpp.tagname) as usize;
            let mfp_size = 1 + len + 10 + ML_EXTRA + 1;
            let mut v = vec![0 as c_char; mfp_size];
            let p = v.as_mut_ptr();
            crate::memory::cstr_cpy(p, tagpp.tagname);
            unsafe {
                *p.wrapping_add(len) = b'@' as c_char;
            }
            crate::memory::cstr_cpy(p.wrapping_add(len + 1), st.help_lang.as_ptr());
            let heur = help_heuristic(
                tagpp.tagname,
                if margs.match_re { margs.matchoff } else { 0 },
                !margs.match_no_ic,
            ) + st.help_pri;
            vim_snprintf!(
                p.wrapping_add(len + 1 + ML_EXTRA),
                mfp_size - (len + 1 + ML_EXTRA),
                c"%06d",
                heur,
            );
            mfp_vec = Some(v);

            unsafe {
                *tagpp.tagname_end = TAB;
            }
        } else if name_only {
            if st.get_searchpat {
                let mut temp_end = tagpp.command;
                if crate::memory::cstr_first_byte(temp_end) == b'/' {
                    while !crate::memory::cstr_is_empty(temp_end)
                        && crate::memory::cstr_first_byte(temp_end) != b'\r'
                        && crate::memory::cstr_first_byte(temp_end) != b'\n'
                        && crate::memory::cstr_first_byte(temp_end) != b'$'
                    {
                        temp_end = temp_end.wrapping_add(1);
                    }
                }

                if tagpp.command.wrapping_add(2) < temp_end {
                    let len =
                        crate::memory::ptr_offset_from(temp_end, tagpp.command.wrapping_add(2))
                            as usize;
                    let mut v = vec![0 as c_char; len + 2];
                    crate::memory::cstr_ncpy(v.as_mut_ptr(), tagpp.command.wrapping_add(2), len);
                    mfp_vec = Some(v);
                }
                st.get_searchpat = false;
            } else {
                let len = crate::memory::ptr_offset_from(tagpp.tagname_end, tagpp.tagname) as usize;
                let mut v = vec![0 as c_char; 1 + len + 1];
                crate::memory::cstr_ncpy(v.as_mut_ptr(), tagpp.tagname, len);
                mfp_vec = Some(v);

                if (g_st().state_global & MODE_INSERT) != 0 {
                    st.get_searchpat = opt_vars().p_sft != 0;
                }
            }
        } else {
            let tag_fname_len = crate::memory::cstr_len(tag_fname_ptr);
            let len = tag_fname_len + crate::memory::cstr_len(lbuf_ptr) + 3;
            let mut v = vec![0 as c_char; 1 + len + 1];
            let p = v.as_mut_ptr();
            unsafe {
                *p = (mtt + 1) as c_char;
            }
            crate::memory::cstr_cpy(p.wrapping_add(1), tag_fname_ptr);
            unsafe {
                *p.wrapping_add(tag_fname_len + 1) = TAG_SEP;
            }
            let s = p.wrapping_add(1 + tag_fname_len + 1);
            crate::memory::cstr_cpy(s, lbuf_ptr);
            mfp_vec = Some(v);
        }

        if let Some(v) = mfp_vec {
            let mfp_bytes = crate::memory::cstr_as_bytes(v.as_ptr()).to_vec();
            if st.ht_match[mtt].insert(mfp_bytes) {
                st.ga_match[mtt].push(v);
                st.match_count += 1;
            }
        }
    }
}

fn findtags_get_all_tags(
    st: &mut FindtagsState,
    margs: &mut FindtagsMatchArgs,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) {
    let buf_ffname = buf_ffname.into_c_char_ptr();
    {
        let mut tagp = Tagptrs::default();
        let mut search_info = TagsearchInfo::default();

        loop {
            if st.state == TagsearchState::TsBinary || st.state == TagsearchState::TsSkipBack {
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
                if retval == TagsReadStatus::TagsReadIgnore {
                    continue;
                }
                if retval == TagsReadStatus::TagsReadEof {
                    break;
                }
            }

            if st.vimconv.vc_type != CONV_NONE {
                findtags_string_convert(st);
            }

            if st.state == TagsearchState::TsStart
                && !findtags_start_state_handler(st, &mut margs.sortic, &mut search_info)
            {
                continue;
            }

            let st_ref = &mut *st;
            if st_ref.lbuf[st_ref.lbuf.len() - 2] != NUL {
                let new_size = st_ref.lbuf.len() * 2;
                st_ref.lbuf = vec![0 as c_char; new_size];

                if st_ref.state == TagsearchState::TsStepForward
                    || st_ref.state == TagsearchState::TsLinear
                {
                    g_st().vim_ignored =
                        unsafe { vim_fseek(st_ref.fp_ptr(), search_info.curr_offset, SEEK_SET) };
                }
                search_info.curr_offset = 0;
                continue;
            }

            let retval = findtags_parse_line(st, &mut tagp, margs, &mut search_info);
            if retval == TagmatchStatus::TagMatchNext {
                continue;
            }
            if retval == TagmatchStatus::TagMatchStop {
                break;
            }
            if retval == TagmatchStatus::TagMatchFail {
                semsg!(
                    gettext(c"E431: Format error in tags file \"%s\""),
                    st.tag_fname.as_ptr(),
                );
                semsg!(
                    gettext(c"Before byte %lld"),
                    unsafe { vim_ftell(st.fp_ptr()) } as i64,
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

fn findtags_in_file(
    st: &mut FindtagsState,
    _flags: i32,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) {
    let buf_ffname = buf_ffname.into_c_char_ptr();
    {
        let mut margs = FindtagsMatchArgs::default();

        st.vimconv.vc_type = CONV_NONE;
        st.tag_file_sorted = NUL as i32;
        st.fp = None;
        findtags_matchargs_init(&mut margs, st.flags);

        if curbuf_mut().b_help && !findtags_in_help_init(st) {
            return;
        }

        let tag_fname_ptr = st.tag_fname.as_ptr();
        st.fp = crate::os::fs::CFile::from_raw(os_fopen(tag_fname_ptr, c"r"));
        if st.fp.is_none() {
            return;
        }

        if opt_vars().p_verbose >= 5 {
            verbose_enter();
            smsg!(0, gettext(c"Searching tags file %s"), tag_fname_ptr,);
            verbose_leave();
        }
        st.did_open = true;
        st.state = TagsearchState::TsStart;

        findtags_get_all_tags(st, &mut margs, buf_ffname);

        st.fp = None;
        if st.vimconv.vc_type != CONV_NONE {
            convert_setup(&mut st.vimconv, ptr::null_mut(), ptr::null_mut());
        }

        if margs.sort_error {
            semsg!(
                gettext(c"E432: Tags file not sorted: %s"),
                st.tag_fname.as_ptr(),
            );
        }

        if st.match_count >= st.mincount {
            st.stop_searching = true;
        }
    }
}

fn findtags_copy_matches(st: &mut FindtagsState, matches: &mut Vec<Vec<c_char>>) {
    {
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
                        unsafe {
                            *mfp -= 1;
                        }
                        let mut p = mfp.wrapping_add(1);
                        while !crate::memory::cstr_is_empty(p) {
                            if unsafe { *p } == TAG_SEP {
                                unsafe {
                                    *p = NUL;
                                }
                            }
                            p = p.wrapping_add(1);
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
    flags: i32,
    mincount: i32,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let buf_ffname = buf_ffname.into_c_char_ptr();
    {
        let mut st = FindtagsState::new();
        let mut retval: i32;
        let mut saved_pat: Option<std::ffi::CString> = None;

        let findall = mincount == MAXCOL || mincount == TAG_MANY;
        let has_re = (flags & TAG_REGEXP) != 0;
        let noic = (flags & TAG_NOIC) != 0;
        let verbose = (flags & TAG_VERBOSE) != 0;
        let save_p_ic = opt_vars().p_ic;

        match if curbuf_mut().b_tc_flags != 0 {
            curbuf_mut().b_tc_flags
        } else {
            opt_vars().tc_flags
        } {
            K_OPT_TC_FLAG_FOLLOWIC => {}
            K_OPT_TC_FLAG_IGNORE => opt_vars().p_ic = 1,
            K_OPT_TC_FLAG_MATCH => opt_vars().p_ic = 0,
            K_OPT_TC_FLAG_FOLLOWSCS => opt_vars().p_ic = ignorecase(pat),
            K_OPT_TC_FLAG_SMART => opt_vars().p_ic = ignorecase_opt(pat, 1, 1),
            _ => std::process::abort(),
        }

        let help_save = curbuf_mut().b_help;

        findtags_state_init(&mut st, pat, flags, mincount);

        if st.help_only {
            curbuf_mut().b_help = true;
        }

        if curbuf_mut().b_help
            && st.orgpat.len > 3
            && unsafe { *pat.wrapping_add((st.orgpat.len - 3) as usize) } == b'@' as c_char
            && ascii_isalpha(unsafe { *pat.wrapping_add((st.orgpat.len - 2) as usize) })
            && ascii_isalpha(unsafe { *pat.wrapping_add((st.orgpat.len - 1) as usize) })
        {
            let keep = (st.orgpat.len - 3) as usize;
            let cstr = std::ffi::CString::new(crate::memory::cstr_slice(pat, keep)).unwrap();
            st.help_lang_find = Some([
                unsafe { *pat.wrapping_add((st.orgpat.len - 2) as usize) },
                unsafe { *pat.wrapping_add((st.orgpat.len - 1) as usize) },
                NUL,
            ]);
            st.orgpat.pat = cstr.as_ptr().cast_mut();
            saved_pat = Some(cstr);
            st.orgpat.len -= 3;
        }
        if opt_vars().p_tl != 0 && (st.orgpat.len as OptInt) > opt_vars().p_tl {
            st.orgpat.len = opt_vars().p_tl as i32;
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
                    && !curbuf_mut().b_fname.is_null()
                {
                    let i = crate::memory::cstr_len(curbuf_mut().b_fname) as i32;
                    if i > 4
                        && mb_strnicmp(
                            curbuf_mut().b_fname.wrapping_add((i - 4) as usize),
                            c".txt",
                            MAXCOL as usize,
                        ) == 0
                    {
                        st.is_txt = true;
                    }
                }

                st.orgpat.regmatch.rm_ic = (opt_vars().p_ic != 0 || !noic)
                    && (findall || st.orgpat.headlen == 0 || opt_vars().p_tbs == 0);
                for round in 1..=2 {
                    st.linear = st.orgpat.headlen == 0 || opt_vars().p_tbs == 0 || round == 2;

                    let mut tn = Tagname::new();
                    let mut first_file: i32 = 1;
                    while get_tagfname(&mut tn, first_file, st.tag_fname.as_mut_ptr()) {
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
                        emsg(gettext(c"E433: No tags file"));
                    }
                    retval = OK;
                }
            }
        }

        if retval == FAIL {
            st.match_count = 0;
        }

        findtags_copy_matches(&mut st, matches);

        curbuf_mut().b_help = help_save;
        drop(saved_pat);

        opt_vars().p_ic = save_p_ic;

        retval
    }
}

pub fn found_tagfile_cb(fnames: &[std::ffi::CString], all: bool) -> bool {
    {
        for fname in fnames {
            let mut tag_fname = fname.clone().into_bytes_with_nul();
            simplify_filename(tag_fname.as_mut_ptr().cast());
            tag_st()
                .tag_fnames
                .push(crate::memory::cstr_to_cstring(tag_fname.as_ptr().cast()));

            if !all {
                break;
            }
        }

        !fnames.is_empty()
    }
}

pub fn free_tag_stuff() {
    {
        tag_st().tag_fnames = Vec::new();
        if !(curwin_ptr()).is_null() {
            do_tag(None, ptr::null_mut(), DT_FREE, 0, 0, false);
        }
        tag_freematch();
        tagstack_clear_entry(&mut tag_st().ptag_entry);
    }
}

/// Get the next name of a tag file from the tag file list.
pub fn get_tagfname(tnp: &mut Tagname, first: i32, buf: *mut c_char) -> bool {
    {
        let mut fname: Option<std::ffi::CString>;

        if first != 0 {
            *tnp = Tagname::new();
        }

        if curbuf_mut().b_help {
            if first != 0 {
                tag_st().tag_fnames.clear();
                do_in_runtimepath(
                    c"doc/tags doc/tags-??",
                    DIP_ALL,
                    &mut RuntimeCallback::FoundTagfile,
                );
            }

            let len = tag_st().tag_fnames.len() as i32;
            if tnp.tn_hf_idx >= len {
                if tnp.tn_hf_idx > len || crate::memory::cstr_is_empty(opt_vars().p_hf) {
                    return false;
                }
                tnp.tn_hf_idx += 1;
                xstrlcpy(buf, opt_vars().p_hf, MAXPATHL - 4);
                crate::memory::cstr_cpy(path_tail(buf), c"tags");
                simplify_filename(buf);

                let buf_bytes = crate::memory::cstr_as_bytes(buf);
                if tag_st()
                    .tag_fnames
                    .iter()
                    .any(|entry| entry.as_bytes() == buf_bytes)
                {
                    return false;
                }
            } else {
                xstrlcpy(
                    buf,
                    tag_st().tag_fnames.as_slice()[tnp.tn_hf_idx as usize].as_ptr(),
                    MAXPATHL,
                );
                tnp.tn_hf_idx += 1;
            }
            return true;
        }

        if first != 0 {
            let src = if !crate::memory::cstr_is_empty(curbuf_mut().b_p_tags) {
                curbuf_mut().b_p_tags
            } else {
                opt_vars().p_tags
            };
            tnp.tn_tags = Some(crate::memory::cstr_to_cstring(src));
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
                if crate::memory::cstr_is_empty(tnp.tn_np) {
                    tnp.tn_search_ctx = None;
                    return false;
                }

                unsafe {
                    *buf = NUL;
                }
                copy_option_part(&raw mut tnp.tn_np, buf, MAXPATHL - 1, c" ,");

                let mut r_ptr = vim_findfile_stopdir(buf);
                let mut filename = path_tail(buf);
                if !r_ptr.is_null() {
                    unsafe {
                        ptr::copy(
                            r_ptr,
                            r_ptr.wrapping_add(1),
                            crate::memory::cstr_len(r_ptr) + 1,
                        )
                    };
                    r_ptr = r_ptr.wrapping_add(1);
                }
                unsafe {
                    ptr::copy(
                        filename,
                        filename.wrapping_add(1),
                        crate::memory::cstr_len(filename) + 1,
                    )
                };
                unsafe {
                    *filename = NUL;
                }
                filename = filename.wrapping_add(1);

                tnp.tn_search_ctx = vim_findfile_init(
                    buf,
                    filename,
                    crate::memory::cstr_len(filename),
                    r_ptr,
                    100,
                    0,
                    FINDFILE_FILE,
                    tnp.tn_search_ctx.take(),
                    1,
                    curbuf_mut().b_ffname,
                );
                if tnp.tn_search_ctx.is_some() {
                    tnp.tn_did_filefind_init = 1;
                }
            }
        }

        crate::memory::cstr_cpy(buf, fname.as_ref().unwrap().as_ptr());
        true
    }
}

fn parse_tag_line(lbuf: impl crate::strings::IntoCCharPtr, tagp: &mut Tagptrs) -> bool {
    let lbuf = lbuf.into_c_char_ptr().cast_mut();
    {
        tagp.tagname = lbuf;
        let mut p = vim_strchr(lbuf, TAB as i32);
        if p.is_null() {
            return false;
        }
        tagp.tagname_end = p;

        if !crate::memory::cstr_is_empty(p) {
            p = p.wrapping_add(1);
        }
        tagp.fname = p;
        p = vim_strchr(p, TAB as i32);
        if p.is_null() {
            return false;
        }
        tagp.fname_end = p;

        if !crate::memory::cstr_is_empty(p) {
            p = p.wrapping_add(1);
        }
        if crate::memory::cstr_is_empty(p) {
            return false;
        }
        tagp.command = p;

        true
    }
}

fn test_for_static(tagp: &Tagptrs) -> bool {
    {
        let mut p = tagp.command;
        loop {
            p = vim_strchr(p, b'\t' as i32);
            if p.is_null() {
                break;
            }
            p = p.wrapping_add(1);
            if crate::memory::cstr_prefix_bytes(p, 5) == &b"file:"[..5] {
                return true;
            }
        }
        false
    }
}

fn matching_line_len(lbuf: impl crate::strings::IntoCCharPtr) -> usize {
    let lbuf = lbuf.into_c_char_ptr();
    {
        let mut p = lbuf.wrapping_add(1);
        p = p.wrapping_add(crate::memory::cstr_len(p) + 1);
        crate::memory::ptr_offset_from(p, lbuf) as usize + crate::memory::cstr_len(p)
    }
}

fn parse_match(lbuf: impl crate::strings::IntoCCharPtr, tagp: &mut Tagptrs) -> bool {
    let mut lbuf = lbuf.into_c_char_ptr().cast_mut();
    {
        tagp.tag_fname = lbuf.wrapping_add(1);
        lbuf = lbuf.wrapping_add(crate::memory::cstr_len(tagp.tag_fname) + 2);

        let retval = parse_tag_line(lbuf, tagp);

        tagp.tagkind = ptr::null_mut();
        tagp.user_data = ptr::null_mut();
        tagp.tagline = 0;
        tagp.command_end = ptr::null_mut();

        if !retval {
            return retval;
        }

        let mut p = tagp.command;
        if find_extra(&raw mut p) {
            tagp.command_end = p;
            if p > tagp.command && unsafe { *p.wrapping_sub(1) } == b'|' as c_char {
                tagp.command_end = p.wrapping_sub(1);
            }
            p = p.wrapping_add(2);
            if unsafe { *p } == TAB {
                p = p.wrapping_add(1);
                while ascii_isalpha(unsafe { *p }) || utfc_ptr2len(p) > 1 {
                    if crate::memory::cstr_prefix_bytes(p, 5) == &b"kind:"[..5] {
                        tagp.tagkind = p.wrapping_add(5);
                    } else if crate::memory::cstr_prefix_bytes(p, 10) == &b"user_data:"[..10] {
                        tagp.user_data = p.wrapping_add(10);
                    } else if crate::memory::cstr_prefix_bytes(p, 5) == &b"line:"[..5] {
                        tagp.tagline =
                            crate::charset::parse_i64_prefix(&crate::memory::cstr_as_bytes(p)[5..])
                                as i32;
                    }
                    if !tagp.tagkind.is_null() && !tagp.user_data.is_null() {
                        break;
                    }

                    let pc = vim_strchr(p, b':' as i32);
                    let pt = vim_strchr(p, b'\t' as i32);
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
            while !crate::memory::cstr_is_empty(p)
                && crate::memory::cstr_first_byte(p) != b'\t'
                && crate::memory::cstr_first_byte(p) != b'\r'
                && crate::memory::cstr_first_byte(p) != b'\n'
            {
                mb_ptr_adv(&mut p);
            }
            tagp.tagkind_end = p;
        }
        if !tagp.user_data.is_null() {
            p = tagp.user_data;
            while !crate::memory::cstr_is_empty(p)
                && crate::memory::cstr_first_byte(p) != b'\t'
                && crate::memory::cstr_first_byte(p) != b'\r'
                && crate::memory::cstr_first_byte(p) != b'\n'
            {
                mb_ptr_adv(&mut p);
            }
            tagp.user_data_end = p;
        }
        retval
    }
}

fn tag_full_fname(tagp: &mut Tagptrs) -> std::ffi::CString {
    unsafe {
        let c = *tagp.fname_end;
        *tagp.fname_end = NUL;
        let fullname = expand_tag_fname(tagp.fname, tagp.tag_fname, false);
        *tagp.fname_end = c;
        fullname
    }
}

fn jumpto_tag(lbuf_arg: impl crate::strings::IntoCCharPtr, forceit: i32, keep_help: bool) -> i32 {
    let lbuf_arg = lbuf_arg.into_c_char_ptr();
    unsafe {
        if g_st().postponed_split == 0 && !check_can_set_curbuf_forceit(forceit) {
            return FAIL;
        }

        let mut pbuf_end: *mut c_char;
        let mut tofree_fname: Option<std::ffi::CString> = None;
        let mut tagp = Tagptrs::default();
        let mut retval = FAIL;
        let mut getfile_result = GETFILE_UNUSED;
        let mut curwin_save: *mut WinInfoInfo = ptr::null_mut();
        let mut full_fname: Option<std::ffi::CString> = None;
        let old_key_typed = g_st().key_typed;
        let l_g_do_tagpreview = g_st().g_do_tagpreview;
        let len = matching_line_len(lbuf_arg) + 1;
        let mut lbuf_vec = vec![0 as c_char; len];
        let lbuf = lbuf_vec.as_mut_ptr();
        ptr::copy(lbuf_arg, lbuf, len);

        let mut pbuf_vec = vec![0 as c_char; LSIZE as usize];
        let pbuf = pbuf_vec.as_mut_ptr();

        'erret: {
            if !parse_match(lbuf, &mut tagp) {
                break 'erret;
            }

            *tagp.fname_end = NUL;
            let mut fname = tagp.fname;

            let mut str_ = tagp.command;
            pbuf_end = pbuf;
            while !crate::memory::cstr_is_empty(str_)
                && crate::memory::cstr_first_byte(str_) != b'\n'
                && crate::memory::cstr_first_byte(str_) != b'\r'
            {
                *pbuf_end = *str_;
                pbuf_end = pbuf_end.wrapping_add(1);
                str_ = str_.wrapping_add(1);
                if crate::memory::ptr_offset_from(pbuf_end, pbuf) as i32 + 1 >= LSIZE {
                    break;
                }
            }
            *pbuf_end = NUL;

            {
                str_ = pbuf;
                if find_extra(&raw mut str_) {
                    if str_ > pbuf && *str_.wrapping_sub(1) == b'|' as c_char {
                        pbuf_end = str_.wrapping_sub(1);
                    } else {
                        pbuf_end = str_;
                    }
                    *pbuf_end = NUL;
                }
            }

            let expanded = expand_tag_fname(fname, tagp.tag_fname, true);
            fname = expanded.as_ptr().cast_mut();
            tofree_fname = Some(expanded);

            if !os_path_exists(fname) && !has_autocmd(EVENT_BUFREADCMD, fname, None) {
                retval = NOTAGFILE;
                tag_st().nofile_fname = tofree_fname.take();
                break 'erret;
            }

            g_st().redrawing_disabled += 1;

            if l_g_do_tagpreview != 0 {
                g_st().postponed_split = 0;
                curwin_save = curwin_ptr();

                if curwin_mut().w_onebuf_opt.wo_pvw == 0 {
                    full_fname = FullName_save(fname, false);
                    fname = full_fname
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());

                    prepare_tagpreview(true, !crate::memory::cstr_is_empty(opt_vars().p_pvp));
                }
            }

            if g_st().postponed_split != 0
                && (opt_vars().swb_flags & (K_OPT_SWB_FLAG_USEOPEN | K_OPT_SWB_FLAG_USETAB)) != 0
            {
                let existing_buf = buflist_findname_exp(fname);
                if !existing_buf.is_null() && !swbuf_goto_win_with_buf(existing_buf).is_null() {
                    getfile_result = GETFILE_SAME_FILE;
                }
            }
            if getfile_result == GETFILE_UNUSED
                && (g_st().postponed_split != 0 || g_st().cmdmod.cmod_tab != 0)
            {
                if (opt_vars().swb_flags & K_OPT_SWB_FLAG_VSPLIT) != 0 {
                    g_st().cmdmod.cmod_split |= WSP_VERT;
                }
                if (opt_vars().swb_flags & K_OPT_SWB_FLAG_NEWTAB) != 0
                    && g_st().cmdmod.cmod_tab == 0
                {
                    g_st().cmdmod.cmod_tab = tabpage_index(g_st().curtab) + 1;
                }
                if !win_split(
                    if g_st().postponed_split > 0 {
                        g_st().postponed_split
                    } else {
                        0
                    },
                    g_st().postponed_split_flags,
                ) {
                    g_st().redrawing_disabled -= 1;
                    break 'erret;
                }
                curwin_mut().w_onebuf_opt.wo_scb = 0;
                curwin_mut().w_onebuf_opt.wo_crb = 0;
            }

            if keep_help {
                if l_g_do_tagpreview != 0 {
                    g_st().keep_help_flag = bt_help(((*curwin_save).w_buffer).as_ref());
                } else {
                    g_st().keep_help_flag = curbuf_mut().b_help;
                }
            }

            if getfile_result == GETFILE_UNUSED {
                getfile_result = getfile(0, fname, ptr::null_mut(), true, 0, forceit != 0);
            }
            g_st().keep_help_flag = false;

            if getfile_result <= 0 {
                curwin_mut().w_set_curswant = true;
                g_st().postponed_split = 0;

                let save_no_hlsearch = g_st().search_global.no_hlsearch;

                let search_options = if crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_TAGPAT) {
                    0
                } else {
                    SEARCH_KEEP
                };

                str_ = pbuf;
                if crate::memory::cstr_first_byte(pbuf) == b'/'
                    || crate::memory::cstr_first_byte(pbuf) == b'?'
                {
                    str_ = skip_regexp(pbuf.wrapping_add(1), *pbuf as i32, 0).wrapping_add(1);
                }
                if str_ > pbuf_end.wrapping_sub(1) {
                    let mut pbuflen = crate::memory::ptr_offset_from(pbuf_end, pbuf) as usize;

                    let save_p_ws = opt_vars().p_ws;
                    let save_p_ic = opt_vars().p_ic;
                    let save_p_scs = opt_vars().p_scs;
                    opt_vars().p_ws = 1;
                    opt_vars().p_ic = 0;
                    opt_vars().p_scs = 0;
                    let save_lnum = curwin_mut().w_cursor.lnum;

                    curwin_mut().w_cursor.lnum = if tagp.tagline > 0 {
                        tagp.tagline - 1
                    } else {
                        0
                    };

                    if do_search(
                        None,
                        *pbuf as i32,
                        *pbuf as i32,
                        pbuf.wrapping_add(1),
                        pbuflen - 1,
                        1,
                        search_options,
                        false,
                        None,
                    ) != 0
                    {
                        retval = OK;
                    } else {
                        let mut found = 1;
                        opt_vars().p_ic = 1;
                        if do_search(
                            None,
                            *pbuf as i32,
                            *pbuf as i32,
                            pbuf.wrapping_add(1),
                            pbuflen - 1,
                            1,
                            search_options,
                            false,
                            None,
                        ) == 0
                        {
                            found = 2;
                            test_for_static(&tagp);
                            let cc = *tagp.tagname_end;
                            *tagp.tagname_end = NUL;
                            pbuflen =
                                vim_snprintf!(pbuf, LSIZE as usize, c"^%s\\s\\*(", tagp.tagname,)
                                    as usize;
                            if do_search(
                                None,
                                b'/' as i32,
                                b'/' as i32,
                                pbuf,
                                pbuflen,
                                1,
                                search_options,
                                false,
                                None,
                            ) == 0
                            {
                                pbuflen = vim_snprintf!(
                                    pbuf,
                                    LSIZE as usize,
                                    c"^\\[#a-zA-Z_]\\.\\*\\<%s\\s\\*(",
                                    tagp.tagname,
                                ) as usize;
                                if do_search(
                                    None,
                                    b'/' as i32,
                                    b'/' as i32,
                                    pbuf,
                                    pbuflen,
                                    1,
                                    search_options,
                                    false,
                                    None,
                                ) == 0
                                {
                                    found = 0;
                                }
                            }
                            *tagp.tagname_end = cc;
                        }
                        if found == 0 {
                            emsg(gettext(c"E434: Can't find tag pattern"));
                            curwin_mut().w_cursor.lnum = save_lnum;
                        } else {
                            if found == 2 || save_p_ic == 0 {
                                msg(gettext(c"E435: Couldn't find tag, just guessing!"), 0);
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

                    check_cursor(curwin_mut());
                } else {
                    let save_secure = g_st().secure;

                    g_st().secure = 1;
                    g_st().sandbox += 1;
                    curwin_mut().w_cursor.lnum = 1;
                    curwin_mut().w_cursor.col = 0;
                    curwin_mut().w_cursor.coladd = 0;
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
                    if curbuf_mut().b_help {
                        set_topline(curwin_mut(), curwin_mut().w_cursor.lnum);
                    }
                    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_TAG) != 0 && old_key_typed {
                        fold_open_cursor();
                    }
                }

                if l_g_do_tagpreview != 0 && curwin_ptr() != curwin_save && win_valid(curwin_save) {
                    validate_cursor(curwin_mut());
                    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                    win_enter(curwin_save, true);
                }

                g_st().redrawing_disabled -= 1;
            } else {
                g_st().redrawing_disabled -= 1;
                if g_st().postponed_split != 0 {
                    win_close(curwin_ptr(), false, false);
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
    fname_arg: impl crate::strings::IntoCCharPtr,
    tag_fname: impl crate::strings::IntoCCharPtr,
    expand: bool,
) -> std::ffi::CString {
    let tag_fname = tag_fname.into_c_char_ptr().cast_mut();
    let fname_arg = fname_arg.into_c_char_ptr();
    {
        let mut xpc: ExpandCtx = ExpandCtx::new();

        let mut fname = to_slash_save(fname_arg).unwrap_or_default();

        if expand
            && path_has_wildcard(fname.as_ptr(), true)
            && !crate::strings::vim_haschr(fname.as_ptr(), b'`' as i32)
        {
            xpc.xp_context = EXPAND_FILES;
            if let Some(exp) = expand_one(
                &mut xpc,
                fname.as_ptr().cast_mut(),
                None,
                WILD_LIST_NOTFOUND | WILD_SILENT,
                WILD_EXPAND_FREE,
            ) {
                fname = exp;
            }
        }

        if (opt_vars().p_tr != 0 || curbuf_mut().b_help) && !vim_isAbsName(fname.as_ptr()) {
            let p = path_tail(tag_fname);
            if p != tag_fname {
                let mut retval = [0 as c_char; MAXPATHL];
                crate::memory::cstr_cpy(retval.as_mut_ptr(), tag_fname);
                let off = crate::memory::ptr_offset_from(p, tag_fname) as usize;
                xstrlcpy(
                    retval.as_mut_ptr().wrapping_add(off),
                    fname.as_ptr(),
                    MAXPATHL - off,
                );
                simplify_filename(retval.as_mut_ptr());
                return crate::memory::cstr_to_cstring(retval.as_ptr());
            }
        }

        fname
    }
}

fn test_for_current(
    fname: impl crate::strings::IntoCCharPtr,
    fname_end: *mut c_char,
    tag_fname: impl crate::strings::IntoCCharPtr,
    buf_ffname: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let fname = fname.into_c_char_ptr().cast_mut();
    let tag_fname = tag_fname.into_c_char_ptr().cast_mut();
    let buf_ffname = buf_ffname.into_c_char_ptr();
    {
        let mut retval = false;
        if !buf_ffname.is_null() {
            let c = unsafe { *fname_end };
            unsafe {
                *fname_end = NUL;
            }
            let fullname = expand_tag_fname(fname, tag_fname, true);
            retval = path_equal(
                fullname.as_ptr(),
                buf_ffname,
                (K_PATH_CMP_EXPAND | K_PATH_CMP_FULL) as PathCmpFlags,
            );
            unsafe {
                *fname_end = c;
            }
        }
        i32::from(retval)
    }
}

fn find_extra(pp: *mut *mut c_char) -> bool {
    unsafe {
        let mut str_ = *pp;
        let mut first_char = **pp;

        loop {
            if ascii_isdigit(*str_) {
                str_ = skipdigits(str_.wrapping_add(1));
            } else if crate::memory::cstr_first_byte(str_) == b'/'
                || crate::memory::cstr_first_byte(str_) == b'?'
            {
                str_ = skip_regexp(str_.wrapping_add(1), *str_ as i32, 0);
                if *str_ != first_char {
                    str_ = ptr::null_mut();
                } else {
                    str_ = str_.wrapping_add(1);
                }
            } else {
                str_ = crate::memory::cstr_as_bytes(str_)
                    .windows(3)
                    .position(|w| w == b"|;\"")
                    .map_or(ptr::null_mut(), |i| str_.wrapping_add(i));
                if !str_.is_null() {
                    str_ = str_.wrapping_add(1);
                    break;
                }
            }
            if str_.is_null()
                || crate::memory::cstr_first_byte(str_) != b';'
                || !(ascii_isdigit(crate::memory::cstr_char_at(str_, 1))
                    || crate::memory::cstr_byte_at(str_, 1) == b'/'
                    || crate::memory::cstr_byte_at(str_, 1) == b'?')
            {
                break;
            }
            str_ = str_.wrapping_add(1);
            first_char = *str_;
        }

        if !str_.is_null() && {
            let _p = str_;
            crate::memory::xstrnlen(_p, 2) == 2 && crate::memory::cstr_slice(_p, 2) == &b";\""[..2]
        } {
            *pp = str_;
            return true;
        }
        false
    }
}

/// Free a single entry in a tag stack.
pub fn tagstack_clear_entry(item: &mut Taggy) {
    item.tagname = None;
    item.user_data = None;
}

pub fn tag_listfiles_extra(idx: usize) -> Option<(*const c_char, *const c_char)> {
    {
        tag_st()
            .tag_listfiles_extra
            .as_slice()
            .get(idx)
            .map(|(k, f)| (k.as_ptr(), f.as_ptr()))
    }
}

pub fn expand_tags(tagnames: bool, pat: *mut c_char, file: &mut Vec<std::ffi::CString>) -> bool {
    {
        let extra_flag = if tagnames { TAG_NAMES } else { 0 };
        let mut raw_matches: Vec<Vec<c_char>> = Vec::new();
        let ret = if crate::memory::cstr_first_byte(pat) == b'/' {
            find_tags(
                pat.wrapping_add(1),
                &mut raw_matches,
                TAG_REGEXP | extra_flag | TAG_VERBOSE,
                TAG_MANY,
                curbuf_mut().b_ffname,
            )
        } else {
            find_tags(
                pat,
                &mut raw_matches,
                TAG_REGEXP | extra_flag | TAG_VERBOSE | TAG_NOIC,
                TAG_MANY,
                curbuf_mut().b_ffname,
            )
        };
        file.clear();
        tag_st().tag_listfiles_extra.clear();
        if ret == OK {
            file.reserve_exact(raw_matches.len());
            if tagnames {
                for m in raw_matches {
                    file.push(crate::memory::cstr_to_cstring(m.as_ptr()));
                }
            } else {
                tag_st()
                    .tag_listfiles_extra
                    .reserve_exact(raw_matches.len());
                for mut m in raw_matches {
                    let mut t_p = Tagptrs::default();
                    parse_match(m.as_mut_ptr(), &mut t_p);
                    let len = crate::memory::ptr_offset_from(t_p.tagname_end, t_p.tagname) as usize;
                    file.push(crate::memory::cstr_from_bytes(unsafe {
                        std::slice::from_raw_parts(t_p.tagname.cast::<u8>(), len)
                    }));
                    let kind_c = if !crate::memory::cstr_is_empty(t_p.tagkind) {
                        unsafe { *t_p.tagkind }
                    } else {
                        b'f' as c_char
                    };
                    let fname_len =
                        crate::memory::ptr_offset_from(t_p.fname_end, t_p.fname) as usize;
                    let fname_cstr = crate::memory::cstr_from_bytes(unsafe {
                        std::slice::from_raw_parts(t_p.fname.cast::<u8>(), fname_len)
                    });
                    tag_st().tag_listfiles_extra.push(([kind_c, 0], fname_cstr));
                }
            }
        }
        ret == OK
    }
}

fn add_tag_field(
    dict: Option<&mut TvDict>,
    field_name: impl crate::strings::IntoCCharPtr,
    start: impl crate::strings::IntoCCharPtr,
    end: impl crate::strings::IntoCCharPtr,
) -> bool {
    let start = start.into_c_char_ptr();
    let mut end = end.into_c_char_ptr();
    let field_name = field_name.into_c_char_ptr();
    {
        if !tv_dict_find(dict.as_deref(), field_name, -1).is_null() {
            if opt_vars().p_verbose > 0 {
                verbose_enter();
                smsg!(0, gettext(c"Duplicate field name: %s"), field_name);
                verbose_leave();
            }
            return false;
        }
        let mut len: i32 = 0;
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let buf = buf_vec.as_mut_ptr();
        if !start.is_null() {
            if end.is_null() {
                end = start.wrapping_add(crate::memory::cstr_len(start));
                while end > start
                    && (crate::memory::cstr_first_byte(end.wrapping_sub(1)) == b'\r'
                        || crate::memory::cstr_first_byte(end.wrapping_sub(1)) == b'\n')
                {
                    end = end.wrapping_sub(1);
                }
            }
            len = (crate::memory::ptr_offset_from(end, start) as i32).min(MAXPATHL as i32 - 1);
            crate::memory::cstr_ncpy(buf, start, len as usize);
        }
        unsafe {
            *buf.wrapping_add(len as usize) = NUL;
        }
        tv_dict_add_str(dict.unwrap(), field_name, buf)
    }
}

/// Add the tags matching the specified pattern `pat` to the list `list`
/// as a dictionary. Use `buf_fname` for priority, unless NULL.
pub fn get_tags(
    list: &mut TvList,
    pat: *mut c_char,
    buf_fname: impl crate::strings::IntoCCharPtr,
) -> bool {
    let buf_fname = buf_fname.into_c_char_ptr();
    {
        let mut matches: Vec<Vec<c_char>> = Vec::new();
        let mut tp = Tagptrs::default();

        let mut ret = find_tags(pat, &mut matches, TAG_REGEXP | TAG_NOIC, MAXCOL, buf_fname) == OK;
        if !ret || matches.is_empty() {
            return ret;
        }

        for mut match_vec in matches {
            let match_i = match_vec.as_mut_ptr();
            if !parse_match(match_i, &mut tp) {
                continue;
            }

            let is_static = test_for_static(&tp);

            if crate::memory::cstr_prefix_bytes(tp.tagname, 6) == &b"!_TAG_"[..6] {
                continue;
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(&mut *list, dict);

            let full_fname = tag_full_fname(&mut tp);
            if !add_tag_field(Some(&mut *dict), c"name", tp.tagname, tp.tagname_end)
                || !add_tag_field(
                    Some(&mut *dict),
                    c"filename",
                    full_fname.as_ptr().cast_mut(),
                    ptr::null(),
                )
                || !add_tag_field(Some(&mut *dict), c"cmd", tp.command, tp.command_end)
                || !add_tag_field(
                    Some(&mut *dict),
                    c"kind",
                    tp.tagkind,
                    if !tp.tagkind.is_null() {
                        tp.tagkind_end.cast_const()
                    } else {
                        ptr::null()
                    },
                )
                || !tv_dict_add_nr(&mut *dict, c"static", Varnumber::from(is_static))
            {
                ret = false;
            }
            drop(full_fname);

            if !tp.command_end.is_null() {
                let mut p = tp.command_end.wrapping_add(3);
                while !crate::memory::cstr_is_empty(p)
                    && crate::memory::cstr_first_byte(p) != b'\n'
                    && crate::memory::cstr_first_byte(p) != b'\r'
                {
                    if p == tp.tagkind
                        || (p.wrapping_add(5) == tp.tagkind
                            && (crate::memory::cstr_prefix_bytes(p, 5) == &b"kind:"[..5]))
                    {
                        p = tp.tagkind_end.wrapping_sub(1);
                    } else if crate::memory::cstr_prefix_bytes(p, 5) == &b"file:"[..5] {
                        p = p.wrapping_add(4);
                    } else if !ascii_iswhite(unsafe { *p }) {
                        let n = p;
                        while !crate::memory::cstr_is_empty(p)
                            && (crate::memory::cstr_first_byte(p)) >= b' '
                            && (crate::memory::cstr_first_byte(p)) < 127
                            && crate::memory::cstr_first_byte(p) != b':'
                        {
                            p = p.wrapping_add(1);
                        }
                        let len = crate::memory::ptr_offset_from(p, n) as i32;
                        if crate::memory::cstr_first_byte(p) == b':' && len > 0 {
                            p = p.wrapping_add(1);
                            let s = p;
                            while !crate::memory::cstr_is_empty(p)
                                && (crate::memory::cstr_first_byte(p)) >= b' '
                            {
                                p = p.wrapping_add(1);
                            }
                            unsafe {
                                *n.wrapping_add(len as usize) = NUL;
                            }
                            if !add_tag_field(Some(&mut *dict), n, s, p) {
                                ret = false;
                            }
                            unsafe {
                                *n.wrapping_add(len as usize) = b':' as c_char;
                            }
                        } else {
                            while !crate::memory::cstr_is_empty(p)
                                && (crate::memory::cstr_first_byte(p)) >= b' '
                            {
                                p = p.wrapping_add(1);
                            }
                        }
                        if crate::memory::cstr_is_empty(p) {
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

fn get_tag_details(tag: &mut Taggy, retdict: &mut TvDict) {
    {
        tv_dict_add_str(
            retdict,
            c"tagname",
            tag.tagname.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        );
        tv_dict_add_nr(retdict, c"matchnr", (tag.cur_match + 1) as Varnumber);
        tv_dict_add_nr(retdict, c"bufnr", tag.cur_fnum as Varnumber);
        if let Some(ref ud) = tag.user_data {
            tv_dict_add_str(retdict, c"user_data", ud.as_ptr());
        }

        let pos = tv_list_alloc(4);
        tv_dict_add_list(retdict, c"from", pos);

        let fmark = &mut tag.fmark;
        tv_list_append_number(
            &mut *pos,
            if fmark.fnum != -1 {
                fmark.fnum as Varnumber
            } else {
                0
            },
        );
        tv_list_append_number(&mut *pos, fmark.mark.lnum as Varnumber);
        tv_list_append_number(
            &mut *pos,
            if fmark.mark.col == MAXCOL {
                MAXCOL as Varnumber
            } else {
                (fmark.mark.col + 1) as Varnumber
            },
        );
        tv_list_append_number(&mut *pos, fmark.mark.coladd as Varnumber);
    }
}

/// Return the tag stack entries of the specified window `wp` in dictionary `retdict`.
pub fn get_tagstack(wp: &mut WinInfoInfo, retdict: &mut TvDict) {
    {
        tv_dict_add_nr(retdict, c"length", wp.w_tagstacklen as Varnumber);
        tv_dict_add_nr(retdict, c"curidx", (wp.w_tagstackidx + 1) as Varnumber);
        let l = tv_list_alloc(2);
        tv_dict_add_list(retdict, c"items", l);

        for i in 0..wp.w_tagstacklen as usize {
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            get_tag_details(&mut wp.w_tagstack[i], d);
        }
    }
}

fn tagstack_clear(wp: &mut WinInfoInfo) {
    {
        for i in 0..wp.w_tagstacklen as usize {
            tagstack_clear_entry(&mut wp.w_tagstack[i]);
        }
        wp.w_tagstacklen = 0;
        wp.w_tagstackidx = 0;
    }
}

fn tagstack_shift(wp: &mut WinInfoInfo) {
    let len = wp.w_tagstacklen as usize;
    (&mut wp.w_tagstack)[..len].rotate_left(1);
    tagstack_clear_entry(&mut wp.w_tagstack[len - 1]);
    wp.w_tagstacklen -= 1;
}

fn tagstack_push_item(
    wp: &mut WinInfoInfo,
    tagname: Option<std::ffi::CString>,
    cur_fnum: i32,
    cur_match: i32,
    mark: CursorPos,
    fnum: i32,
    user_data: Option<std::ffi::CString>,
) {
    {
        let mut idx = wp.w_tagstacklen;

        if idx >= TAGSTACKSIZE {
            tagstack_shift(wp);
            idx = TAGSTACKSIZE - 1;
        }

        wp.w_tagstacklen += 1;
        let entry = &mut wp.w_tagstack[idx as usize];
        entry.tagname = tagname;
        entry.cur_fnum = cur_fnum;
        entry.cur_match = cur_match.max(0);
        entry.fmark.mark = mark;
        entry.fmark.fnum = fnum;
        entry.fmark.view = Fmarkv {
            topline_offset: MAXLNUM,
            skipcol: 0,
        };
        entry.user_data = user_data;
    }
}

fn tagstack_push_items(wp: &mut WinInfoInfo, l: Option<&TvList>) {
    unsafe {
        for li in crate::eval::typval::list_iter_mut(l.map_or(ptr::null_mut(), |l| l.lv_first)) {
            if li.li_tv.v_type == VarType::VarDict && !li.li_tv.vval.v_dict().is_null() {
                let itemdict = li.li_tv.vval.v_dict();
                let di = tv_dict_find(itemdict.as_ref(), c"from", -1);
                if !di.is_null() {
                    let mut mark = CursorPos {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    };
                    let mut fnum: i32 = 0;
                    if list2fpos(&mut (*di).di_tv, &mut mark, Some(&mut fnum), None, false) {
                        let tagname = tv_dict_get_string(itemdict.as_ref(), c"tagname");
                        if !tagname.is_null() {
                            if mark.col > 0 {
                                mark.col -= 1;
                            }
                            let ud_ptr = tv_dict_get_string(itemdict.as_ref(), c"user_data");
                            tagstack_push_item(
                                wp,
                                Some(crate::memory::cstr_to_cstring(tagname)),
                                tv_dict_get_number(itemdict.as_ref(), c"bufnr") as i32,
                                tv_dict_get_number(itemdict.as_ref(), c"matchnr") as i32 - 1,
                                mark,
                                fnum,
                                (!ud_ptr.is_null()).then(|| crate::memory::cstr_to_cstring(ud_ptr)),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn tagstack_set_curidx(wp: &mut WinInfoInfo, curidx: i32) {
    wp.w_tagstackidx = curidx.max(0).min(wp.w_tagstacklen);
}

/// Set the tag stack entries of the specified window.
pub fn set_tagstack(wp: &mut WinInfoInfo, d: Option<&TvDict>, action: i32) -> bool {
    {
        if tag_st().tfu_in_use {
            emsg(gettext(E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC));
            return false;
        }

        let mut l: *mut TvList = ptr::null_mut();

        let di_items = tv_dict_find(d, c"items", -1);
        if !di_items.is_null() {
            let di_items = { unsafe { &mut *di_items } };
            if di_items.di_tv.v_type != VarType::VarList {
                emsg(gettext(E_LISTREQ));
                return false;
            }
            l = di_items.di_tv.vval.v_list();
        }

        let di_curidx = tv_dict_find(d, c"curidx", -1);
        if !di_curidx.is_null() {
            tagstack_set_curidx(wp, tv_get_number(unsafe { &(*di_curidx).di_tv }) as i32 - 1);
        }

        if action == b't' as i32 {
            let tagstackidx = wp.w_tagstackidx;
            let mut tagstacklen = wp.w_tagstacklen;

            while tagstackidx < tagstacklen {
                tagstacklen -= 1;
                tagstack_clear_entry(&mut curwin_mut().w_tagstack[tagstacklen as usize]);
            }
            wp.w_tagstacklen = tagstacklen;
        }

        if !l.is_null() {
            if action == b'r' as i32 {
                tagstack_clear(wp);
            }

            tagstack_push_items(&mut *wp, unsafe { l.as_ref() });
            wp.w_tagstackidx = wp.w_tagstacklen;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_strnicmp_compares_ascii_case_folded_to_uppercase() {
        assert_eq!(tag_strnicmp(c"abc", c"ABC", 3), 0);
        assert!(tag_strnicmp(c"abc", c"_bc", 3) < 0);
    }
}
