#![forbid(unsafe_code)]

//! Code to handle tags and the tag stack (`src/nvim/tag.c`).

use crate::autocmd::{EVENT_BUFREADCMD, has_autocmd};
use crate::buffer::{
    GETF_SETMARK, GetFileResult, K_OPT_JOP_FLAG_VIEW, K_OPT_SWB_FLAG_NEWTAB, K_OPT_SWB_FLAG_VSPLIT,
    MAXLNUM, VAR_FIXED, bt_help, buflist_findname_exp, buflist_findnr, buflist_getfile,
};
use crate::charset::{ascii_islower, ascii_iswhite, vim_isblankline};
use crate::cmdexpand::{
    EXPAND_FILES, HLF_D, TAG_MANY, WILD_EXPAND_FREE, WILD_LIST_NOTFOUND, WILD_SILENT, expand_one,
};
use crate::cursor::{CursorPos, Linenr, check_cursor};
use crate::drawscreen::{UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::typval::{
    K_CALLBACK_NONE_CONST, TvDict, TvList, TypVal, TypvalVval, VAR_UNLOCKED, VarType, Varnumber,
    callback_free, tv_clear, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_dict_alloc_lock, tv_dict_find, tv_dict_find_opt,
    tv_dict_get_number, tv_dict_get_string, tv_get_number, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_dict, tv_list_append_number, tv_list_append_tv, tv_list_free, tv_list_iter,
};
use crate::eval::vars::{VV_SWAPCOMMAND, set_vim_var_string};
use crate::eval::{callback_call, list2fpos, set_ref_in_callback};
use crate::ex_cmds::{getfile, prepare_tagpreview};
use crate::ex_docmd::{
    DT_FIRST, DT_JUMP, DT_LAST, DT_LTAG, DT_NEXT, DT_POP, DT_PREV, DT_SELECT, DT_TAG, Exarg,
    do_cmdline_cmd, set_no_hlsearch,
};
use crate::file_search::{FindFileWhat, vim_findfile, vim_findfile_init, vim_findfile_stopdir};
use crate::fileio::{Off, vim_fgets};
use crate::fold::fold_open_cursor;
use crate::globals::*;
use crate::help::{
    DT_HELP, TAG_HELP, TAG_KEEP_LANG, TAG_NAMES, TAG_NO_TAGFUNC, TAG_REGEXP, TAG_VERBOSE,
    help_heuristic,
};
use crate::insexpand::{TAG_INS_COMP, TAG_NOIC, ins_compl_check_keys, ins_compl_interrupted};
use crate::lua::executor::nlua_call_excmd;
use crate::mark::{Fmark, Fmarkv, fm_getname, mark_view_make, mark_view_restore, setpcmark};
use crate::mbyte::{Vimconv, convert_setup, mb_strnicmp, string_convert, utfc_ptr2len};

use crate::message::{
    emsg, give_warning, msg, msg_delay, msg_outtrans, msg_putchar, msg_puts, msg_puts_title, semsg,
    smsg, verbose_enter, verbose_leave, wait_return,
};
use crate::r#move::{set_topline, validate_cursor};
use crate::option::{OptInt, copy_option_part};
use crate::option_vars::*;
use crate::os::fs::{CFile, os_path_exists};
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::path::{
    FullName_save, K_PATH_CMP_EXPAND, K_PATH_CMP_FULL, PathCmpFlags, path_equal, path_has_wildcard,
    path_tail, simplify_filename, to_slash_save, vim_isAbsName,
};
use crate::quickfix::{CMDBUFFSIZE, K_OPT_SWB_FLAG_USETAB, set_errorlist};
use crate::regexp::{MAXCOL, RE_MAGIC, Regmatch, skip_regexp, vim_regcomp, vim_regexec};
use crate::runtime::{DIP_ALL, RuntimeCallback, do_in_runtimepath};
use crate::search::{do_search, ignorecase, ignorecase_opt};
use crate::state::MODE_INSERT;
use crate::strings::{format_to_buf, format_to_buf_safelen, gettext, vim_strchr};
use crate::window::{
    WSP_VERT, WinInfoInfo, check_can_set_curbuf_forceit, swbuf_goto_win_with_buf, tabpage_index,
    win_close, win_enter, win_split, win_valid,
};
use std::ffi::CStr;

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

const TAG_SEP: u8 = 0x02;
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
    pub tn_np_off: usize,
    pub tn_did_filefind_init: bool,
    pub tn_hf_idx: i32,
    pub tn_search_ctx: Option<crate::file_search::FfSearchCtx>,
}

impl Tagname {
    pub const fn new() -> Self {
        Self {
            tn_tags: None,
            tn_np_off: 0,
            tn_did_filefind_init: false,
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
use crate::os::env::ExpandCtx;
use crate::os::fs::MAXPATHL;
use crate::viml::parser::parser::CONV_NONE;

#[derive(Clone, Copy, Default)]
struct Tagptrs<'a> {
    raw_line: &'a [u8],
    tagname: &'a [u8],
    fname: &'a [u8],
    command: &'a [u8],
    command_end: Option<usize>,
    tag_fname: Option<&'a std::ffi::CStr>,
    tagkind: Option<&'a [u8]>,
    tagkind_range: Option<(usize, usize)>,
    user_data: Option<&'a [u8]>,
    tagline: Linenr,
}

#[repr(C)]
struct Pat {
    pat: std::ffi::CString,
    len: i32,
    head_off: usize,
    headlen: i32,
    regmatch: Regmatch,
}

impl Pat {
    #[inline]
    fn head_bytes(&self) -> &[u8] {
        self.pat.to_bytes().get(self.head_off..).unwrap_or(&[])
    }
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
    lbuf: Vec<u8>,
    tag_fname: Vec<u8>,
    fp: Option<crate::os::fs::CFile>,
    flags: i32,
    tag_file_sorted: i32,
    get_searchpat: bool,
    help_only: bool,
    did_open: bool,
    mincount: i32,
    linear: bool,
    vimconv: Vimconv,
    help_lang: [u8; 3],
    help_pri: i32,
    help_lang_find: Option<[u8; 3]>,
    is_txt: bool,
    match_count: i32,
    ga_match: [Vec<Vec<u8>>; MT_COUNT],
    ht_match: [std::collections::HashSet<Vec<u8>>; MT_COUNT],
}

impl FindtagsState {
    fn new() -> Self {
        Self {
            state: TagsearchState::TsStart,
            stop_searching: false,
            orgpat: Pat {
                pat: std::ffi::CString::default(),
                len: 0,
                head_off: 0,
                headlen: 0,
                regmatch: Regmatch::new(),
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
    do_tag_matches: Vec<Vec<u8>>,
    do_tag_flags: i32,
    tag_listfiles_extra: Vec<(std::ffi::CString, std::ffi::CString)>,
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
fn tag_st() -> crate::globals::GraphPtr<TagModuleState> {
    TAG_STATE.handle()
}

use crate::charset::toupper_asc;

pub fn free_tagfunc_option() {
    callback_free(&mut opt_vars().p_tfu);
}

/// Mark the global `'tagfunc'` callback with `copyID` so that it is not garbage
/// collected.
pub fn set_ref_in_tagfunc(copy_id: i32) -> bool {
    set_ref_in_callback(&mut opt_vars().p_tfu, copy_id, None, None)
}

/// Jump to tag; handling of tag commands and tag stack.
pub fn do_tag(
    eap: Option<&Exarg>,
    tag: &std::ffi::CStr,
    mut type_: i32,
    count: i32,
    forceit: bool,
    verbose: bool,
) {
    {
        let tagstack = crate::globals::GraphPtr::<Taggy>::of(
            (&raw mut curwin_mut().w_tagstack).cast::<Taggy>(),
        )
        .as_ptr();
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
        let mut buf_ffname = curbuf_mut().ffname_opt();
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

        'end_do_tag: {
            if !opt_vars().p_tgst && !tag.is_empty() {
                use_tagstack = false;
                new_tag = true;
                if g_st().g_do_tagpreview != 0 {
                    tagstack_clear_entry(&mut tag_st().ptag_entry);
                    tag_st().ptag_entry.tagname = Some(tag.to_owned());
                }
            } else {
                use_tagstack = g_st().g_do_tagpreview == 0;

                // new pattern, add to the tag stack
                if !tag.is_empty()
                    && (type_ == DT_TAG
                        || type_ == DT_SELECT
                        || type_ == DT_JUMP
                        || type_ == DT_LTAG)
                {
                    if g_st().g_do_tagpreview != 0 {
                        if tag_st()
                            .ptag_entry
                            .tagname
                            .as_ref()
                            .is_some_and(|t| t.to_bytes() == tag.to_bytes())
                        {
                            cur_match = tag_st().ptag_entry.cur_match;
                            cur_fnum = tag_st().ptag_entry.cur_fnum;
                        } else {
                            tagstack_clear_entry(&mut tag_st().ptag_entry);
                            tag_st().ptag_entry.tagname = Some(tag.to_owned());
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
                            Some(tag.to_owned());
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
                                i32::from(forceit),
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

            if cur_fnum != curbuf_mut().b_fnum
                && let Some(buf) = buflist_findnr(cur_fnum)
            {
                buf_ffname = buf.ffname_opt();
            }

            loop {
                let mut name_cstr: &CStr = if use_tagstack {
                    tofree = curwin_mut().w_tagstack[tagstackidx as usize]
                        .tagname
                        .clone();
                    tofree.as_deref().unwrap_or(c"")
                } else if g_st().g_do_tagpreview != 0 {
                    tofree = tag_st().ptag_entry.tagname.clone();
                    tofree.as_deref().unwrap_or(c"")
                } else {
                    tag
                };

                let other_name = tag_st()
                    .tagmatchname
                    .as_ref()
                    .is_none_or(|m| m.to_bytes() != name_cstr.to_bytes());
                if new_tag
                    || (cur_match >= tag_st().do_tag_matches.len() as i32
                        && tag_st().do_tag_max_num_matches != MAXCOL)
                    || other_name
                {
                    if other_name {
                        tag_st().tagmatchname = Some(name_cstr.to_owned());
                    }

                    if type_ == DT_SELECT || type_ == DT_JUMP || type_ == DT_LTAG {
                        cur_match = MAXCOL - 1;
                    }
                    tag_st().do_tag_max_num_matches = if type_ == DT_TAG {
                        MAXCOL
                    } else {
                        cur_match + 1
                    };

                    if !no_regexp && name_cstr.to_bytes().first() == Some(&b'/') {
                        tag_st().do_tag_flags = TAG_REGEXP;
                        name_cstr = &name_cstr[1..];
                    } else {
                        tag_st().do_tag_flags = TAG_NOIC;
                    }

                    if verbose {
                        tag_st().do_tag_flags |= TAG_VERBOSE;
                    }
                    if !use_tfu {
                        tag_st().do_tag_flags |= TAG_NO_TAGFUNC;
                    }

                    let mut new_matches: Vec<Vec<u8>> = Vec::new();
                    if find_tags(
                        name_cstr,
                        &mut new_matches,
                        tag_st().do_tag_flags,
                        tag_st().do_tag_max_num_matches,
                        buf_ffname,
                    ) && (new_matches.len() as i32) < tag_st().do_tag_max_num_matches
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

                        for old_match in &tag_st().do_tag_matches {
                            let mut tagp = Tagptrs::default();
                            parse_match(old_match, &mut tagp);
                            for i in idx..new_matches.len() {
                                let matched = {
                                    let mut tagp2 = Tagptrs::default();
                                    parse_match(&new_matches[i], &mut tagp2);
                                    tagp.raw_line == tagp2.raw_line
                                };
                                if matched {
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
                        semsg!(gettext(E_TAG_NOT_FOUND_STR), name_cstr);
                    }
                    g_st().g_do_tagpreview = 0;
                } else {
                    if type_ == DT_TAG && !tag.is_empty() {
                        cur_match = if count > 0 { count - 1 } else { 0 };
                    } else if type_ == DT_SELECT || (type_ == DT_JUMP && do_tag_num_matches > 1) {
                        select_tag_match(
                            (eap.is_some()).then(|| eap.unwrap()),
                            new_tag,
                            use_tagstack,
                            &tag_st().do_tag_matches,
                            name_cstr,
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

                        let do_tag_matches = &tag_st().do_tag_matches;
                        if use_tfu
                            && parse_match(&do_tag_matches[cur_match as usize], &mut tagp2)
                            && let Some(ud_slice) = tagp2.user_data
                        {
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
                        smsg!(0, gettext(c"File \"%s\" does not exist"), nf,);
                    }

                    let cur_match_slice =
                        &tag_st().borrow_mut().do_tag_matches.as_slice()[cur_match as usize];
                    let ic = ((cur_match_slice[0] as usize) & MT_IC_OFF) != 0;
                    if type_ != DT_TAG
                        && type_ != DT_SELECT
                        && type_ != DT_JUMP
                        && (do_tag_num_matches > 1 || ic)
                        && !skip_msg
                    {
                        let more_str = if tag_st().do_tag_max_num_matches != MAXCOL {
                            gettext(c" or more")
                        } else {
                            c""
                        };
                        format_to_buf!(
                            g_st().iobuff_bytes_mut(),
                            gettext(c"tag %d of %d%s"),
                            cur_match + 1,
                            do_tag_num_matches,
                            more_str,
                        );
                        if ic {
                            crate::memory::append_bytes_to_c_buf(
                                g_st().iobuff_bytes_mut(),
                                gettext(c"  Using tag with different case!").to_bytes(),
                            );
                        }
                        let iobuf_cstr =
                            crate::memory::bytes_as_cstr(g_st().borrow_mut().iobuff_bytes());
                        if (do_tag_num_matches > prev_num_matches || new_tag)
                            && do_tag_num_matches > 1
                        {
                            msg(iobuf_cstr, if ic { HLF_W } else { 0 });
                            g_st().msg_scroll = 1;
                        } else {
                            give_warning(iobuf_cstr, ic, true);
                        }
                        if ic && g_st().msg_scrolled == 0 && g_st().msg_silent == 0 {
                            msg_delay(1007, true);
                        }
                    }

                    let iobufflen =
                        format_to_buf_safelen!(g_st().iobuff_bytes_mut(), c":ta %s\r", name_cstr,);
                    set_vim_var_string(
                        VV_SWAPCOMMAND,
                        Some(crate::memory::bytes_as_cstr(g_st().iobuff_bytes())),
                        iobufflen as isize,
                    );

                    let i = jumpto_tag(cur_match_slice, forceit, true);

                    set_vim_var_string(VV_SWAPCOMMAND, None, -1);

                    if i.is_none() {
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
                            tag_st().nofile_fname.as_deref().unwrap_or(c""),
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
    matches: &[Vec<u8>],
    name: &std::ffi::CStr,
) {
    {
        let tagstackidx = curwin_mut().w_tagstackidx;

        let mut items_tv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        tv_list_alloc_ret(&mut items_tv, matches.len() as isize);

        for (idx, m_vec) in matches.iter().enumerate() {
            let i = idx as i32;
            os_breakcheck();
            if g_st().got_int {
                tv_clear(&mut items_tv);
                return;
            }
            let mut tagp = Tagptrs::default();
            parse_match(m_vec, &mut tagp);

            let cur = !new_tag
                && ((g_st().g_do_tagpreview != 0 && i == tag_st().ptag_entry.cur_match)
                    || (use_tagstack
                        && i == curwin_mut().w_tagstack[tagstackidx as usize].cur_match));

            let d = tv_dict_alloc();
            tv_dict_add_str_len(&mut *d, c"tag", Some(tagp.tagname));
            let pri_idx = (m_vec[0] as usize) & MT_MASK;
            tv_dict_add_str(&mut *d, c"pri", Some(MT_NAMES[pri_idx]));
            if let Some(kind) = tagp.tagkind {
                tv_dict_add_str_len(&mut *d, c"kind", Some(kind));
            }
            let fname = tag_full_fname(&mut tagp);
            tv_dict_add_str(&mut *d, c"file", Some(fname.as_c_str()));
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
                vval: TypvalVval::Dict(crate::globals::GraphPtr::new(d)),
            };
            tv_list_append_tv(items_tv.vval.v_list_ref(), &mut item);
        }

        let extra_d = tv_dict_alloc();
        tv_dict_add_list(&mut *extra_d, c"items", items_tv.vval.v_list_opt());
        items_tv.vval.v_list_ref().lv_refcount += 1;
        tv_dict_add_str(&mut *extra_d, c"tagname", Some(name));
        let mut extra_tv = TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(crate::globals::GraphPtr::new(extra_d)),
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
fn add_llist_tags(tag: &std::ffi::CStr, matches: &[Vec<u8>]) -> bool {
    {
        let mut tag_name = [0u8; 128 + 1];

        let mut fname_vec = vec![0u8; MAXPATHL + 1];
        let fname = fname_vec.as_mut_ptr();
        let mut cmd_vec = Vec::with_capacity(CMDBUFFSIZE + 1);
        let list = tv_list_alloc(0);

        for m_vec in matches {
            let mut tagp = Tagptrs::default();
            parse_match(m_vec, &mut tagp);

            let len = tagp.tagname.len().min(128);
            tag_name[..len].copy_from_slice(&tagp.tagname[..len]);
            tag_name[len] = 0;

            let p = tag_full_fname(&mut tagp);
            crate::memory::copy_bytes_to_c_buf(
                crate::globals::GraphPtr::raw_slice_mut(fname, MAXPATHL),
                p.as_bytes(),
            );

            let mut lnum: Linenr = 0;
            let cmd_bytes = tagp.command;
            if cmd_bytes.first().is_some_and(u8::is_ascii_digit) {
                lnum = crate::charset::parse_i64_prefix(cmd_bytes) as i32;
            } else {
                let end_idx = tagp.command_end.unwrap_or_else(|| {
                    cmd_bytes
                        .iter()
                        .position(|&b| matches!(b, b'\r' | b'\n'))
                        .unwrap_or(cmd_bytes.len())
                });
                let mut cmd_slice = &cmd_bytes[..end_idx.min(cmd_bytes.len())];
                if let Some((&first, rest)) = cmd_slice.split_first()
                    && matches!(first, b'/' | b'?')
                {
                    cmd_slice = rest;
                }
                if let Some((&last, rest)) = cmd_slice.split_last()
                    && matches!(last, b'/' | b'?')
                {
                    cmd_slice = rest;
                }

                cmd_vec.clear();
                if let Some(rest) = cmd_slice.strip_prefix(b"^") {
                    cmd_vec.push(b'^');
                    cmd_slice = rest;
                }

                cmd_vec.extend_from_slice(b"\\V");

                let cmd_len = cmd_slice.len().min(CMDBUFFSIZE - 5);
                if cmd_len > 0 {
                    cmd_vec.extend_from_slice(&cmd_slice[..cmd_len]);
                }

                if cmd_vec.last() == Some(&b'$') {
                    let last = cmd_vec.len() - 1;
                    cmd_vec[last] = b'\\';
                    cmd_vec.push(b'$');
                }

                cmd_vec.push(0);
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(&mut *list, Some(dict));

            tv_dict_add_str(
                &mut *dict,
                c"text",
                Some(CStr::from_bytes_until_nul(&tag_name).unwrap_or_default()),
            );
            tv_dict_add_str(
                &mut *dict,
                c"filename",
                Some(CStr::from_bytes_until_nul(&fname_vec).unwrap_or_default()),
            );
            tv_dict_add_nr(&mut *dict, c"lnum", lnum as Varnumber);
            if lnum == 0 {
                tv_dict_add_str(
                    &mut *dict,
                    c"pattern",
                    Some(CStr::from_bytes_until_nul(&cmd_vec).unwrap_or_default()),
                );
            }
        }

        format_to_buf!(g_st().iobuff_bytes_mut(), c"ltag %s", tag);
        set_errorlist(
            Some(curwin_mut()),
            Some(&*list),
            b' ' as i32,
            Some(crate::memory::bytes_as_cstr(g_st().iobuff_bytes())),
            None,
        );

        tv_list_free(Some(list));

        true
    }
}

/// Free cached tags.
pub fn tag_freematch() {
    tag_st().tagmatchname = None;
}

/// Print the tag stack.
pub fn do_tags(_eap: &mut Exarg) {
    let tagstackidx = curwin_mut().w_tagstackidx;
    let tagstacklen = curwin_mut().w_tagstacklen;

    msg_puts_title(gettext(c"\n  # TO tag         FROM line  in file/text"));
    for i in 0..tagstacklen {
        let entry = &mut curwin_mut().w_tagstack[i as usize];
        if let Some(ref tagname) = entry.tagname {
            let Some(name) = fm_getname(&mut entry.fmark, 30) else {
                continue;
            };

            msg_putchar(b'\n' as i32);
            format_to_buf!(
                g_st().iobuff_bytes_mut(),
                c"%c%2d %2d %-15s %5d  ",
                if i == tagstackidx {
                    b'>' as i32
                } else {
                    b' ' as i32
                },
                i + 1,
                entry.cur_match + 1,
                tagname,
                entry.fmark.mark.lnum,
            );
            msg_outtrans(
                crate::memory::bytes_as_cstr(g_st().iobuff_bytes()),
                0,
                false,
            );
            msg_outtrans(
                &name,
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

fn tag_strnicmp(b1: &[u8], b2: &[u8], len: usize) -> i32 {
    for idx in 0..len {
        let c1 = b1.get(idx).copied().unwrap_or(0);
        let c2 = b2.get(idx).copied().unwrap_or(0);
        let diff = toupper_asc(i32::from(c1)) - toupper_asc(i32::from(c2));
        if diff != 0 {
            return diff;
        }
        if c1 == 0 {
            break;
        }
    }
    0
}

fn prepare_pats(pats: &mut Pat, has_re: bool) {
    pats.head_off = 0;
    pats.headlen = pats.len;
    if has_re {
        let pat_bytes = pats.pat.to_bytes();
        if pat_bytes.first() == Some(&b'^') {
            pats.head_off = 1;
        } else if pat_bytes.starts_with(b"\\<") {
            pats.head_off = 2;
        }
        if pats.head_off == 0 {
            pats.headlen = 0;
        } else {
            let head_bytes = pats.head_bytes();
            let magic_chars: &[u8] = if opt_vars().p_magic {
                b".[~*\\$"
            } else {
                b"\\$"
            };
            pats.headlen = head_bytes
                .iter()
                .take_while(|b| !magic_chars.contains(b))
                .count() as i32;
        }
        if opt_vars().p_tl != 0 && (pats.headlen as OptInt) > opt_vars().p_tl {
            pats.headlen = opt_vars().p_tl as i32;
        }
    }

    if has_re {
        pats.regmatch.set_prog(vim_regcomp(
            &pats.pat,
            if opt_vars().p_magic { RE_MAGIC } else { 0 },
        ));
    } else {
        pats.regmatch.free_prog();
    }
}

fn tagfunc_cmd_kind(cmd: &std::ffi::CStr) -> Tagcmd {
    let cmd_bytes = cmd.to_bytes();
    let first_b = cmd_bytes.first().copied().unwrap_or(0);
    if first_b.is_ascii_digit() {
        return if cmd_bytes.iter().all(u8::is_ascii_digit) {
            Tagcmd::TagcmdAddress
        } else {
            Tagcmd::TagcmdInvalid
        };
    }
    if first_b == b'/' || first_b == b'?' {
        let end = skip_regexp(&cmd[1..], first_b as i32, 0);
        let end_bytes = end.to_bytes();
        return if end_bytes == [first_b] {
            Tagcmd::TagcmdAddress
        } else {
            Tagcmd::TagcmdInvalid
        };
    }
    if cmd_bytes.iter().any(|b| b"\t\r\n".contains(b)) {
        return Tagcmd::TagcmdInvalid;
    }
    Tagcmd::TagcmdGeneric
}

fn find_tagfunc_tags(
    pat_cstr: &std::ffi::CStr,
    ga: &mut Vec<Vec<u8>>,
    match_count: &mut i32,
    flags: i32,
    buf_ffname: Option<&std::ffi::CStr>,
) -> Option<bool> {
    let mut ntags: i32 = 0;
    let mut args = [const {
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        }
    }; 4];
    let mut rettv = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval::Number(0),
    };
    let mut flag_string = [0u8; 4];

    let tag_idx = if curwin_mut().w_tagstacklen > 0 {
        Some(
            if curwin_mut().w_tagstackidx == curwin_mut().w_tagstacklen {
                (curwin_mut().w_tagstackidx - 1) as usize
            } else {
                curwin_mut().w_tagstackidx as usize
            },
        )
    } else {
        None
    };

    if curbuf_mut().b_p_tfu.cb_type() == K_CALLBACK_NONE_CONST {
        return Some(false);
    }

    format_to_buf!(
        &mut flag_string[..],
        c"%s%s%s",
        if g_st().g_tag_at_cursor { c"c" } else { c"" },
        if (flags & TAG_INS_COMP) != 0 {
            c"i"
        } else {
            c""
        },
        if (flags & TAG_REGEXP) != 0 { c"r" } else { c"" },
    );

    args[0].v_type = VarType::VarString;
    args[0].vval = crate::eval::typval::TypvalVval::from_cstr(pat_cstr);
    args[1].v_type = VarType::VarString;
    args[1].vval =
        crate::eval::typval::TypvalVval::from_cstr(crate::memory::bytes_as_cstr(&flag_string));

    let d = tv_dict_alloc_lock(VAR_FIXED);
    if (flags & TAG_INS_COMP) == 0
        && let Some(idx) = tag_idx
        && let Some(ref ud) = curwin_mut().w_tagstack[idx].user_data
    {
        tv_dict_add_str(d, c"user_data", Some(ud.as_c_str()));
    }
    if let Some(ffname) = buf_ffname {
        tv_dict_add_str(d, c"buf_ffname", Some(ffname));
    }

    d.dv_refcount += 1;
    args[2].v_type = VarType::VarDict;
    args[2].vval = crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::new(d));
    args[3].v_type = VarType::VarUnknown;

    let save_pos = curwin_mut().w_cursor;
    let mut result = callback_call(&mut curbuf_mut().b_p_tfu, 3, &mut args, &mut rettv);
    curwin_mut().w_cursor = save_pos;
    check_cursor(curwin_mut());
    d.dv_refcount -= 1;

    if !result {
        return Some(false);
    }
    if rettv.v_type == VarType::VarSpecial && rettv.vval.v_special() == K_SPECIAL_VAR_NULL_CONST {
        tv_clear(&mut rettv);
        return None;
    }
    if rettv.v_type != VarType::VarList || rettv.vval.v_list().is_null() {
        tv_clear(&mut rettv);
        emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
        return Some(false);
    }
    let taglist = rettv.vval.v_list_opt();

    for li in tv_list_iter(taglist.as_deref()) {
        let mut res_name: Option<&std::ffi::CStr> = None;
        let mut res_fname: Option<&std::ffi::CStr> = None;
        let mut res_cmd: Option<&std::ffi::CStr> = None;
        let mut res_kind: Option<&std::ffi::CStr> = None;
        let mut has_extra = false;
        let name_only = (flags & TAG_NAMES) != 0;

        if li.li_tv.v_type != VarType::VarDict {
            emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
            break;
        }

        let mut len: usize = 2;
        let item_dict = li.li_tv.vval.v_dict_opt();
        if let Some(ref item_dict) = item_dict {
            for (hi, di) in item_dict.dv_hashtab.iter_items() {
                let dict_key = hi.key_cstr();
                let tv = &di.di_tv;

                if tv.v_type == VarType::VarString && !tv.vval.v_string_is_null() {
                    len += tv.vval.v_string_bytes().len() + 1;
                    let dk_bytes = dict_key.map_or(&[][..], std::ffi::CStr::to_bytes);
                    if dk_bytes == b"name" {
                        res_name = tv.vval.v_string_cstr();
                    } else if dk_bytes == b"filename" {
                        res_fname = tv.vval.v_string_cstr();
                    } else if dk_bytes == b"cmd" {
                        res_cmd = tv.vval.v_string_cstr();
                    } else {
                        has_extra = true;
                        if dk_bytes == b"kind" {
                            res_kind = tv.vval.v_string_cstr();
                        } else {
                            len += dk_bytes.len() + 1;
                        }
                    }
                }
            }
        }

        if has_extra {
            len += 2;
        }

        let (Some(res_name), Some(res_fname), Some(res_cmd)) = (res_name, res_fname, res_cmd)
        else {
            emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
            break;
        };

        let cmdkind = tagfunc_cmd_kind(res_cmd);
        if cmdkind == Tagcmd::TagcmdInvalid {
            emsg(gettext(E_INVALID_RETURN_VALUE_FROM_TAGFUNC));
            break;
        }
        if cmdkind == Tagcmd::TagcmdGeneric {
            len += 3;
        }
        let mfp_vec: Vec<u8> = if name_only {
            res_name.to_owned().into_bytes_with_nul()
        } else {
            let mut v = Vec::with_capacity(len + 2);
            v.push((MT_GL_OTH + 1) as u8);
            v.push(TAG_SEP);
            v.extend_from_slice(res_name.to_bytes());
            v.push(TAB);
            v.extend_from_slice(res_fname.to_bytes());
            v.push(TAB);
            v.extend_from_slice(res_cmd.to_bytes());

            if cmdkind == Tagcmd::TagcmdGeneric {
                v.push(b'|');
            }
            if cmdkind == Tagcmd::TagcmdGeneric || has_extra {
                v.extend_from_slice(b";\"");
                if let Some(res_kind) = res_kind {
                    v.push(TAB);
                    v.extend_from_slice(res_kind.to_bytes());
                }
                if let Some(ref item_dict) = item_dict {
                    for (hi, di) in item_dict.dv_hashtab.iter_items() {
                        let dict_key = hi.key_cstr();
                        let tv = &di.di_tv;
                        let key_bytes = dict_key.map_or(&[][..], std::ffi::CStr::to_bytes);
                        if tv.v_type == VarType::VarString
                            && !tv.vval.v_string_is_null()
                            && key_bytes != b"name"
                            && key_bytes != b"filename"
                            && key_bytes != b"cmd"
                            && key_bytes != b"kind"
                        {
                            v.push(TAB);
                            v.extend_from_slice(key_bytes);
                            v.push(b':');
                            v.extend_from_slice(tv.vval.v_string_bytes());
                        }
                    }
                }
            }
            v.push(0);
            if v.len() < len + 2 {
                v.resize(len + 2, 0);
            }
            v
        };

        ga.push(mfp_vec);
        ntags += 1;
        result = true;
    }

    tv_clear(&mut rettv);

    *match_count = ntags;
    Some(result)
}

impl FindtagsState {
    #[inline]
    fn seek(&self, offset: Off, whence: i32) -> i32 {
        self.fp.as_ref().map_or(-1, |f| f.seek(offset, whence))
    }

    #[inline]
    fn tell(&self) -> Off {
        self.fp.as_ref().map_or(-1, |f| f.tell())
    }
}

fn findtags_state_init(st: &mut FindtagsState, pat: &std::ffi::CStr, flags: i32, mincount: i32) {
    {
        st.tag_fname = vec![0u8; MAXPATHL + 1];
        st.fp = None;
        st.orgpat = Pat {
            pat: pat.to_owned(),
            len: pat.to_bytes().len() as i32,
            head_off: 0,
            headlen: 0,
            regmatch: Regmatch::new(),
        };
        st.flags = flags;
        st.tag_file_sorted = 0;
        st.help_lang_find = None;
        st.is_txt = false;
        st.did_open = false;
        st.help_only = (flags & TAG_HELP) != 0;
        st.get_searchpat = false;
        st.help_lang = [0; 3];
        st.help_pri = 0;
        st.mincount = mincount;
        st.lbuf = vec![0u8; LSIZE as usize];
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
        self.orgpat.regmatch.free_prog();
    }
}

fn findtags_in_help_init(st: &mut FindtagsState) -> bool {
    if st.is_txt {
        st.help_lang[..3].copy_from_slice(b"en\0");
    } else {
        let i = crate::memory::bytes_until_nul(&st.tag_fname).len();
        if i > 3 && st.tag_fname[i - 3] == b'-' {
            st.help_lang[..2].copy_from_slice(&st.tag_fname[i - 2..i]);
            st.help_lang[2] = 0;
        } else {
            st.help_lang[..3].copy_from_slice(b"en\0");
        }
    }

    if let Some(ref lang_find) = st.help_lang_find
        && mb_strnicmp(
            crate::memory::bytes_until_nul(&st.help_lang),
            crate::memory::bytes_until_nul(lang_find),
            MAXCOL as usize,
        ) != 0
    {
        return false;
    }

    let fname_bytes = curbuf_mut().fname_bytes();
    if (st.flags & TAG_KEEP_LANG) != 0
        && st.help_lang_find.is_none()
        && fname_bytes.len() > 4
        && fname_bytes[fname_bytes.len() - 1] == b'x'
        && fname_bytes[fname_bytes.len() - 4] == b'.'
        && mb_strnicmp(
            &fname_bytes[fname_bytes.len() - 3..],
            crate::memory::bytes_until_nul(&st.help_lang),
            2,
        ) == 0
    {
        st.help_pri = 0;
    } else {
        st.help_pri = 1;
        let p_hlg = opt_vars().p_hlg.to_owned();
        let mut s = p_hlg.as_c_str();
        while !s.to_bytes().is_empty() {
            if mb_strnicmp(
                s.to_bytes(),
                crate::memory::bytes_until_nul(&st.help_lang),
                2,
            ) == 0
            {
                break;
            }
            st.help_pri += 1;
            let Some(found) = vim_strchr(s, b',' as i32) else {
                s = c"";
                break;
            };
            s = &found[1..];
        }
        if s.to_bytes().is_empty() {
            st.help_pri += 1;
            if mb_strnicmp(
                crate::memory::bytes_until_nul(&st.help_lang),
                (c"en").to_bytes(),
                MAXCOL as usize,
            ) != 0
            {
                st.help_pri += 1;
            }
        }
    }

    true
}

fn findtags_apply_tfu(
    st: &mut FindtagsState,
    pat: &std::ffi::CStr,
    buf_ffname: Option<&std::ffi::CStr>,
) -> Option<bool> {
    {
        let use_tfu = (st.flags & TAG_NO_TAGFUNC) == 0;

        if !use_tfu
            || tag_st().tfu_in_use
            || curbuf_mut().b_p_tfu.cb_type() == K_CALLBACK_NONE_CONST
        {
            return None;
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
            st.seek(0, SEEK_SET);
            st.state = TagsearchState::TsStepForward;
        }
    }

    let fp = st.fp.as_ref().unwrap();
    if st.state == TagsearchState::TsBinary || st.state == TagsearchState::TsSkipBack {
        sinfo_p.curr_offset_used = sinfo_p.curr_offset;
        g_st().vim_ignored = st.seek(sinfo_p.curr_offset, SEEK_SET);
        eof = vim_fgets(&mut st.lbuf, fp);
        if !eof && sinfo_p.curr_offset != 0 {
            sinfo_p.curr_offset = st.tell();
            if sinfo_p.curr_offset == sinfo_p.high_offset {
                g_st().vim_ignored = st.seek(sinfo_p.low_offset, SEEK_SET);
                sinfo_p.curr_offset = sinfo_p.low_offset;
            }
            eof = vim_fgets(&mut st.lbuf, st.fp.as_ref().unwrap());
        }
        while !eof && vim_isblankline(crate::memory::bytes_until_nul(&st.lbuf)) {
            sinfo_p.curr_offset = st.tell();
            eof = vim_fgets(&mut st.lbuf, st.fp.as_ref().unwrap());
        }
        if eof {
            st.state = TagsearchState::TsSkipBack;
            sinfo_p.match_offset = st.tell();
            sinfo_p.curr_offset = sinfo_p.curr_offset_used;
            return TagsReadStatus::TagsReadIgnore;
        }
    } else {
        loop {
            eof = vim_fgets(&mut st.lbuf, st.fp.as_ref().unwrap());
            if eof || !vim_isblankline(crate::memory::bytes_until_nul(&st.lbuf)) {
                break;
            }
        }
        if eof {
            return TagsReadStatus::TagsReadEof;
        }
    }

    TagsReadStatus::TagsReadSuccess
}

fn findtags_hdr_parse(st: &mut FindtagsState) -> bool {
    let lbuf_bytes = crate::memory::bytes_until_nul(&st.lbuf);
    if !lbuf_bytes.starts_with(b"!_TAG_") {
        return true;
    }

    if lbuf_bytes.starts_with(b"!_TAG_FILE_SORTED\t") {
        st.tag_file_sorted = i32::from(st.lbuf[18]);
    }
    if lbuf_bytes.starts_with(b"!_TAG_FILE_ENCODING\t") {
        let mut end = 20;
        while end < st.lbuf.len() && st.lbuf[end] > b' ' && st.lbuf[end] < 127 {
            end += 1;
        }
        if end < st.lbuf.len() {
            st.lbuf[end] = 0;
        }
        convert_setup(
            &mut st.vimconv,
            Some(crate::memory::bytes_as_cstr(&st.lbuf[20..])),
            Some(opt_vars().p_enc.as_cstr()),
        );
    }

    false
}

fn findtags_start_state_handler(
    st: &mut FindtagsState,
    sortic: &mut bool,
    sinfo_p: &mut TagsearchInfo,
) -> bool {
    let noic = (st.flags & TAG_NOIC) != 0;

    let lbuf_bytes = crate::memory::bytes_until_nul(&st.lbuf);
    if lbuf_bytes[..6.min(lbuf_bytes.len())] <= b"!_TAG_"[..]
        || (st.lbuf[0] == b'!' && ascii_islower(st.lbuf[1] as i32))
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
        st.orgpat.regmatch.rm_ic = opt_vars().p_ic || !noic;
    } else {
        st.state = TagsearchState::TsLinear;
    }

    if st.state == TagsearchState::TsBinary && st.orgpat.regmatch.rm_ic && !*sortic {
        st.linear = true;
        st.state = TagsearchState::TsLinear;
    }

    if st.state == TagsearchState::TsBinary {
        if st.seek(0, SEEK_END) != 0 {
            st.state = TagsearchState::TsLinear;
        } else {
            let filesize = st.tell();
            g_st().vim_ignored = st.seek(0, SEEK_SET);

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

fn findtags_parse_line<'a>(
    st: &mut FindtagsState,
    lbuf: &'a [u8],
    tagpp: &mut Tagptrs<'a>,
    margs: &mut FindtagsMatchArgs,
    sinfo_p: &mut TagsearchInfo,
) -> TagmatchStatus {
    let status: bool;

    if st.orgpat.headlen != 0 {
        *tagpp = Tagptrs::default();
        let raw_line = crate::memory::bytes_until_nul(lbuf);
        tagpp.raw_line = raw_line;
        let Some(tab_pos) = raw_line.iter().position(|&b| b == TAB) else {
            return TagmatchStatus::TagMatchFail;
        };
        tagpp.tagname = &raw_line[..tab_pos];

        let mut cmplen = tagpp.tagname.len() as i32;
        if opt_vars().p_tl != 0 && (cmplen as OptInt) > opt_vars().p_tl {
            cmplen = opt_vars().p_tl as i32;
        }
        if (st.flags & TAG_REGEXP) != 0 && st.orgpat.headlen < cmplen {
            cmplen = st.orgpat.headlen;
        } else if st.state == TagsearchState::TsLinear && st.orgpat.headlen != cmplen {
            return TagmatchStatus::TagMatchNext;
        }

        if st.state == TagsearchState::TsBinary {
            let first_b = tagpp.tagname.first().copied().unwrap_or(0);
            let mut i = i32::from(first_b);
            if margs.sortic {
                i = toupper_asc(i32::from(first_b));
            }
            if i < sinfo_p.low_char || i > sinfo_p.high_char {
                margs.sort_error = true;
            }

            let mut tagcmp = if margs.sortic {
                tag_strnicmp(tagpp.tagname, st.orgpat.head_bytes(), cmplen as usize)
            } else {
                let tn_b = tagpp.tagname;
                let hd_b = st.orgpat.head_bytes();
                tn_b[..(cmplen as usize).min(tn_b.len())]
                    .cmp(&hd_b[..(cmplen as usize).min(hd_b.len())]) as i32
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
                sinfo_p.curr_offset = st.tell();
                if sinfo_p.curr_offset < sinfo_p.high_offset {
                    sinfo_p.low_offset = sinfo_p.curr_offset;
                    if margs.sortic {
                        sinfo_p.low_char = toupper_asc(i32::from(first_b));
                    } else {
                        sinfo_p.low_char = i32::from(first_b);
                    }
                    return TagmatchStatus::TagMatchNext;
                }
            }
            if tagcmp > 0 && sinfo_p.curr_offset != sinfo_p.high_offset {
                sinfo_p.high_offset = sinfo_p.curr_offset;
                if margs.sortic {
                    sinfo_p.high_char = toupper_asc(i32::from(first_b));
                } else {
                    sinfo_p.high_char = i32::from(first_b);
                }
                return TagmatchStatus::TagMatchNext;
            }

            return TagmatchStatus::TagMatchStop;
        } else if st.state == TagsearchState::TsSkipBack {
            assert!(cmplen >= 0);
            if mb_strnicmp(tagpp.tagname, st.orgpat.head_bytes(), cmplen as usize) != 0 {
                st.state = TagsearchState::TsStepForward;
            } else {
                sinfo_p.curr_offset = sinfo_p.curr_offset_used;
            }
            return TagmatchStatus::TagMatchNext;
        } else if st.state == TagsearchState::TsStepForward {
            assert!(cmplen >= 0);
            if mb_strnicmp(tagpp.tagname, st.orgpat.head_bytes(), cmplen as usize) != 0 {
                return if st.tell() > sinfo_p.match_offset {
                    TagmatchStatus::TagMatchStop
                } else {
                    TagmatchStatus::TagMatchNext
                };
            }
        } else {
            assert!(cmplen >= 0);
            if mb_strnicmp(tagpp.tagname, st.orgpat.head_bytes(), cmplen as usize) != 0 {
                return TagmatchStatus::TagMatchNext;
            }
        }

        let after_tag = &raw_line[tab_pos + 1..];
        if let Some(fname_tab) = after_tag.iter().position(|&b| b == TAB) {
            tagpp.fname = &after_tag[..fname_tab];
            tagpp.command = &after_tag[fname_tab + 1..];
            status = true;
        } else {
            status = false;
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
    let mut match_: bool;

    let mut cmplen = tagpp.tagname.len() as i32;
    if opt_vars().p_tl != 0 && (cmplen as OptInt) > opt_vars().p_tl {
        cmplen = opt_vars().p_tl as i32;
    }
    if st.orgpat.len != cmplen {
        match_ = false;
    } else if st.orgpat.regmatch.rm_ic {
        assert!(cmplen >= 0);
        match_ = mb_strnicmp(tagpp.tagname, st.orgpat.pat.to_bytes(), cmplen as usize) == 0;
        if match_ {
            let tn_b = tagpp.tagname;
            let pat_b = st.orgpat.pat.to_bytes();
            margs.match_no_ic = tn_b[..(cmplen as usize).min(tn_b.len())]
                == pat_b[..(cmplen as usize).min(pat_b.len())];
        }
    } else {
        let tn_b = tagpp.tagname;
        let pat_b = st.orgpat.pat.to_bytes();
        match_ = tn_b[..(cmplen as usize).min(tn_b.len())]
            == pat_b[..(cmplen as usize).min(pat_b.len())];
    }

    margs.match_re = false;
    if !match_ && !st.orgpat.regmatch.regprog.is_null() {
        let tagname_cstr = std::ffi::CString::new(tagpp.tagname).unwrap_or_default();
        match_ = vim_regexec(&mut st.orgpat.regmatch, &tagname_cstr, 0);
        if match_ {
            margs.matchoff = st.orgpat.regmatch.start_offset(0, &tagname_cstr) as i32;
            if st.orgpat.regmatch.rm_ic {
                st.orgpat.regmatch.rm_ic = false;
                margs.match_no_ic = vim_regexec(&mut st.orgpat.regmatch, &tagname_cstr, 0);
                st.orgpat.regmatch.rm_ic = true;
            }
        }
        margs.match_re = true;
    }

    match_
}

fn findtags_string_convert(st: &mut FindtagsState) {
    let Some(conv_line) =
        string_convert(&st.vimconv, crate::memory::bytes_until_nul(&st.lbuf), None)
    else {
        return;
    };

    let len = conv_line.len();
    if len > st.lbuf.len() {
        st.lbuf.resize(len, 0);
    }
    let conv_bytes = crate::memory::bytes_as_cstr(&conv_line).to_bytes_with_nul();
    st.lbuf[..conv_bytes.len()].copy_from_slice(conv_bytes);
}

fn findtags_add_match(
    st: &mut FindtagsState,
    tagpp: &mut Tagptrs<'_>,
    margs: &FindtagsMatchArgs,
    buf_ffname: Option<&std::ffi::CStr>,
) {
    {
        let name_only = (st.flags & TAG_NAMES) != 0;
        let tag_fname_cstr = crate::memory::bytes_as_cstr(&st.tag_fname);
        let lbuf_bytes = tagpp.raw_line;

        let is_current = test_for_current(tagpp.fname, tag_fname_cstr, buf_ffname) != 0;
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

        let mut mfp_vec: Option<Vec<u8>> = None;

        if st.help_only {
            let tagname_slice = tagpp.tagname;
            let len = tagname_slice.len();
            let mfp_size = 1 + len + 10 + ML_EXTRA + 1;
            let mut v = vec![0u8; mfp_size];
            v[..len].copy_from_slice(tagname_slice);
            v[len] = b'@';
            let lang_bytes = crate::memory::bytes_as_cstr(&st.help_lang).to_bytes_with_nul();
            v[len + 1..len + 1 + lang_bytes.len()].copy_from_slice(lang_bytes);
            let heur = help_heuristic(
                tagname_slice,
                if margs.match_re { margs.matchoff } else { 0 },
                !margs.match_no_ic,
            ) + st.help_pri;
            format_to_buf!(&mut v[len + 1 + ML_EXTRA..mfp_size], c"%06d", heur,);
            mfp_vec = Some(v);
        } else if name_only {
            if st.get_searchpat {
                let cmd_bytes = tagpp.command;
                let temp_end_idx = if cmd_bytes.first() == Some(&b'/') {
                    cmd_bytes
                        .iter()
                        .position(|&b| matches!(b, b'\r' | b'\n' | b'$'))
                        .unwrap_or(cmd_bytes.len())
                } else {
                    0
                };

                if temp_end_idx > 2 {
                    let len = temp_end_idx - 2;
                    let mut v = vec![0u8; len + 2];
                    v[..len].copy_from_slice(&cmd_bytes[2..temp_end_idx]);
                    mfp_vec = Some(v);
                }
                st.get_searchpat = false;
            } else {
                let len = tagpp.tagname.len();
                let mut v = vec![0u8; 1 + len + 1];
                v[..len].copy_from_slice(tagpp.tagname);
                mfp_vec = Some(v);

                if (g_st().state_global & MODE_INSERT) != 0 {
                    st.get_searchpat = opt_vars().p_sft;
                }
            }
        } else {
            let tag_fname_bytes = tag_fname_cstr.to_bytes();
            let tag_fname_len = tag_fname_bytes.len();
            let len = tag_fname_len + lbuf_bytes.len() + 3;
            let mut v = vec![0u8; 1 + len + 1];
            v[0] = (mtt + 1) as u8;
            v[1..1 + tag_fname_len].copy_from_slice(tag_fname_bytes);
            v[tag_fname_len + 1] = TAG_SEP;
            v[1 + tag_fname_len + 1..1 + tag_fname_len + 1 + lbuf_bytes.len()]
                .copy_from_slice(lbuf_bytes);
            mfp_vec = Some(v);
        }

        if let Some(v) = mfp_vec {
            let mfp_bytes = crate::memory::bytes_until_nul(&v).to_vec();
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
    buf_ffname: Option<&std::ffi::CStr>,
) {
    {
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
            if st_ref.lbuf[st_ref.lbuf.len() - 2] != 0 {
                let new_size = st_ref.lbuf.len() * 2;
                st_ref.lbuf = vec![0u8; new_size];

                if st_ref.state == TagsearchState::TsStepForward
                    || st_ref.state == TagsearchState::TsLinear
                {
                    g_st().vim_ignored = st_ref.seek(search_info.curr_offset, SEEK_SET);
                }
                search_info.curr_offset = 0;
                continue;
            }

            let lbuf = std::mem::take(&mut st.lbuf);
            let mut tagp = Tagptrs::default();
            let retval = findtags_parse_line(st, &lbuf, &mut tagp, margs, &mut search_info);
            if retval == TagmatchStatus::TagMatchNext {
                st.lbuf = lbuf;
                continue;
            }
            if retval == TagmatchStatus::TagMatchStop {
                st.lbuf = lbuf;
                break;
            }
            if retval == TagmatchStatus::TagMatchFail {
                st.lbuf = lbuf;
                semsg!(
                    gettext(c"E431: Format error in tags file \"%s\""),
                    CStr::from_bytes_until_nul(&st.tag_fname).unwrap_or_default(),
                );
                semsg!(gettext(c"Before byte %lld"), st.tell());
                st.stop_searching = true;
                return;
            }

            if findtags_match_tag(st, &mut tagp, margs) {
                findtags_add_match(st, &mut tagp, margs, buf_ffname);
            }
            st.lbuf = lbuf;
        }
    }
}

fn findtags_in_file(st: &mut FindtagsState, _flags: i32, buf_ffname: Option<&std::ffi::CStr>) {
    {
        let mut margs = FindtagsMatchArgs::default();

        st.vimconv.vc_type = CONV_NONE;
        st.tag_file_sorted = 0;
        st.fp = None;
        findtags_matchargs_init(&mut margs, st.flags);

        if curbuf_mut().b_help && !findtags_in_help_init(st) {
            return;
        }

        let tag_fname_cstr = CStr::from_bytes_until_nul(&st.tag_fname).unwrap_or_default();
        st.fp = CFile::open(tag_fname_cstr, c"r");
        if st.fp.is_none() {
            return;
        }

        if opt_vars().p_verbose >= 5 {
            verbose_enter();
            let tag_fname_cstr = CStr::from_bytes_until_nul(&st.tag_fname).unwrap_or_default();
            smsg!(0, gettext(c"Searching tags file %s"), tag_fname_cstr,);
            verbose_leave();
        }
        st.did_open = true;
        st.state = TagsearchState::TsStart;

        findtags_get_all_tags(st, &mut margs, buf_ffname);

        st.fp = None;
        if st.vimconv.vc_type != CONV_NONE {
            convert_setup(&mut st.vimconv, None, None);
        }

        if margs.sort_error {
            let tag_fname_cstr = CStr::from_bytes_until_nul(&st.tag_fname).unwrap_or_default();
            semsg!(gettext(c"E432: Tags file not sorted: %s"), tag_fname_cstr,);
        }

        if st.match_count >= st.mincount {
            st.stop_searching = true;
        }
    }
}

fn findtags_copy_matches(st: &mut FindtagsState, matches: &mut Vec<Vec<u8>>) {
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
                    mfp_vec[0] = mfp_vec[0].wrapping_sub(1);
                    for b in &mut mfp_vec[1..] {
                        if *b == 0 {
                            break;
                        }
                        if *b == TAG_SEP {
                            *b = 0;
                        }
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

/// `find_tags()` - search for tags in tags files into owned `Vec<u8>` items.
pub fn find_tags(
    pat_cstr: &std::ffi::CStr,
    matches: &mut Vec<Vec<u8>>,
    flags: i32,
    mincount: i32,
    buf_ffname: Option<&std::ffi::CStr>,
) -> bool {
    {
        let mut st = FindtagsState::new();
        let mut retval: bool;

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
            K_OPT_TC_FLAG_IGNORE => opt_vars().p_ic = true,
            K_OPT_TC_FLAG_MATCH => opt_vars().p_ic = false,
            K_OPT_TC_FLAG_FOLLOWSCS => opt_vars().p_ic = ignorecase(pat_cstr),
            K_OPT_TC_FLAG_SMART => opt_vars().p_ic = ignorecase_opt(pat_cstr, true, true),
            _ => std::process::abort(),
        }

        let help_save = curbuf_mut().b_help;

        findtags_state_init(&mut st, pat_cstr, flags, mincount);

        if st.help_only {
            curbuf_mut().b_help = true;
        }

        let pat_bytes = pat_cstr.to_bytes();
        let pat_slice = &pat_bytes[..(st.orgpat.len.max(0) as usize).min(pat_bytes.len())];
        if curbuf_mut().b_help
            && pat_slice.len() > 3
            && pat_slice[pat_slice.len() - 3] == b'@'
            && pat_slice[pat_slice.len() - 2].is_ascii_alphabetic()
            && pat_slice[pat_slice.len() - 1].is_ascii_alphabetic()
        {
            let keep = pat_slice.len() - 3;
            let cstr = std::ffi::CString::new(&pat_slice[..keep]).unwrap();
            st.help_lang_find = Some([
                pat_slice[pat_slice.len() - 2],
                pat_slice[pat_slice.len() - 1],
                0,
            ]);
            st.orgpat.pat = cstr;
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
            retval = false;
        } else if let Some(tfu_ret) = findtags_apply_tfu(&mut st, pat_cstr, buf_ffname) {
            retval = tfu_ret;
        } else {
            retval = false;

            if (flags & TAG_KEEP_LANG) != 0
                && st.help_lang_find.is_none()
                && curbuf_mut().fname_opt().is_some()
            {
                let fname_bytes = curbuf_mut().fname_bytes();
                if fname_bytes.len() > 4
                    && mb_strnicmp(
                        &fname_bytes[fname_bytes.len() - 4..],
                        (c".txt").to_bytes(),
                        MAXCOL as usize,
                    ) == 0
                {
                    st.is_txt = true;
                }
            }

            st.orgpat.regmatch.rm_ic = (opt_vars().p_ic || !noic)
                && (findall || st.orgpat.headlen == 0 || !opt_vars().p_tbs);
            for round in 1..=2 {
                st.linear = st.orgpat.headlen == 0 || !opt_vars().p_tbs || round == 2;

                let mut tn = Tagname::new();
                let mut first_file: i32 = 1;
                while get_tagfname(&mut tn, first_file, &mut st.tag_fname) {
                    findtags_in_file(&mut st, flags, buf_ffname);
                    if st.stop_searching {
                        retval = true;
                        break;
                    }
                    first_file = 0;
                }

                if st.stop_searching
                    || st.linear
                    || (!opt_vars().p_ic && noic)
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
                retval = true;
            }
        }

        if !retval {
            st.match_count = 0;
        }

        findtags_copy_matches(&mut st, matches);

        curbuf_mut().b_help = help_save;

        opt_vars().p_ic = save_p_ic;

        retval
    }
}

pub fn found_tagfile_cb(fnames: &[std::ffi::CString], all: bool) -> bool {
    for fname in fnames {
        let mut tag_fname = fname.clone().into_bytes_with_nul();
        simplify_filename(&mut tag_fname);
        tag_st()
            .tag_fnames
            .push(crate::memory::cstring_until_nul(&tag_fname));

        if !all {
            break;
        }
    }

    !fnames.is_empty()
}

pub fn free_tag_stuff() {
    tag_st().tag_fnames = Vec::new();
    if !(curwin_ptr()).is_null() {
        do_tag(None, c"", DT_FREE, 0, false, false);
    }
    tag_freematch();
    tagstack_clear_entry(&mut tag_st().ptag_entry);
}

/// Get the next name of a tag file from the tag file list.
pub fn get_tagfname(tnp: &mut Tagname, first: i32, buf: &mut [u8]) -> bool {
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
            if tnp.tn_hf_idx > len || opt_vars().p_hf.is_empty() {
                return false;
            }
            tnp.tn_hf_idx += 1;
            crate::memory::copy_bytes_to_c_buf(
                &mut buf[..MAXPATHL - 4],
                opt_vars().p_hf.as_bytes(),
            );
            let buf_cstr = crate::memory::bytes_as_cstr(buf);
            let tail_off = path_tail(buf_cstr).as_ptr() as usize - buf_cstr.as_ptr() as usize;
            buf[tail_off..tail_off + 5].copy_from_slice(b"tags\0");
            simplify_filename(buf);

            let buf_bytes = crate::memory::bytes_until_nul(buf);
            if tag_st()
                .tag_fnames
                .iter()
                .any(|entry| entry.as_bytes() == buf_bytes)
            {
                return false;
            }
        } else {
            crate::memory::copy_bytes_to_c_buf(
                buf,
                tag_st().tag_fnames.as_slice()[tnp.tn_hf_idx as usize].as_bytes(),
            );
            tnp.tn_hf_idx += 1;
        }
        return true;
    }

    if first != 0 {
        let src = if !curbuf_mut().b_p_tags.is_empty() {
            curbuf_mut().b_p_tags.to_owned()
        } else {
            opt_vars().p_tags.to_owned()
        };
        tnp.tn_tags = Some(src);
        tnp.tn_np_off = 0;
    }

    loop {
        if tnp.tn_did_filefind_init {
            fname = vim_findfile(tnp.tn_search_ctx.as_mut());
            if fname.is_some() {
                break;
            }
            tnp.tn_did_filefind_init = false;
        } else {
            let tags_bytes = tnp.tn_tags.as_ref().map_or(&[][..], |s| s.as_bytes());
            if tnp.tn_np_off >= tags_bytes.len() {
                tnp.tn_search_ctx = None;
                return false;
            }

            buf[0] = 0;
            let mut np_bytes = &tags_bytes[tnp.tn_np_off..];
            let old_len = np_bytes.len();
            copy_option_part(&mut np_bytes, &mut buf[..MAXPATHL - 1], c" ,");
            tnp.tn_np_off += old_len - np_bytes.len();

            let stop_off = vim_findfile_stopdir(buf);
            let stopdirs_cstr =
                stop_off.map(|off| crate::memory::bytes_as_cstr(&buf[off..]).to_owned());
            let buf_cstr = crate::memory::bytes_as_cstr(buf);
            let tail_off = path_tail(buf_cstr).as_ptr() as usize - buf_cstr.as_ptr() as usize;
            let filename_cstr = crate::memory::cstring_until_nul(&buf[tail_off..]);
            buf[tail_off] = 0;

            tnp.tn_search_ctx = vim_findfile_init(
                crate::memory::bytes_as_cstr(buf),
                filename_cstr.as_bytes(),
                stopdirs_cstr.as_deref(),
                100,
                false,
                FindFileWhat::File,
                tnp.tn_search_ctx.take(),
                true,
                curbuf_mut().ffname_opt(),
            );
            if tnp.tn_search_ctx.is_some() {
                tnp.tn_did_filefind_init = true;
            }
        }
    }

    let fname_bytes = fname.as_ref().unwrap().to_bytes_with_nul();
    buf[..fname_bytes.len()].copy_from_slice(fname_bytes);
    true
}

fn parse_tag_line<'a>(lbuf_slice: &'a [u8], tagp: &mut Tagptrs<'a>) -> bool {
    let raw_line = crate::memory::bytes_until_nul(lbuf_slice);
    tagp.raw_line = raw_line;
    let Some(tab1) = raw_line.iter().position(|&b| b == TAB) else {
        return false;
    };
    tagp.tagname = &raw_line[..tab1];
    let rest = &raw_line[tab1 + 1..];
    let Some(tab2) = rest.iter().position(|&b| b == TAB) else {
        return false;
    };
    tagp.fname = &rest[..tab2];
    let cmd = &rest[tab2 + 1..];
    if cmd.is_empty() {
        return false;
    }
    tagp.command = cmd;
    true
}

fn test_for_static(tagp: &Tagptrs<'_>) -> bool {
    let mut p = tagp.command;
    while let Some(pos) = p.iter().position(|&b| b == b'\t') {
        p = &p[pos + 1..];
        if p.starts_with(b"file:") {
            return true;
        }
    }
    false
}

fn matching_line_len(lbuf_slice: &[u8]) -> usize {
    let first_len = crate::memory::bytes_until_nul(&lbuf_slice[1..]).len();
    let off = 1 + first_len + 1;
    off + crate::memory::bytes_until_nul(&lbuf_slice[off..]).len()
}

fn parse_match<'a>(lbuf_slice: &'a [u8], tagp: &mut Tagptrs<'a>) -> bool {
    let tag_fname = crate::memory::bytes_as_cstr(&lbuf_slice[1..]);
    tagp.tag_fname = Some(tag_fname);
    let tag_line_off = tag_fname.to_bytes().len() + 2;

    let retval = parse_tag_line(&lbuf_slice[tag_line_off.min(lbuf_slice.len())..], tagp);

    tagp.tagkind = None;
    tagp.tagkind_range = None;
    tagp.user_data = None;
    tagp.tagline = 0;
    tagp.command_end = None;

    if !retval {
        return false;
    }

    let cmd_bytes = tagp.command;
    if let Some(extra_off) = find_extra(cmd_bytes) {
        let cmd_end = if extra_off > 0 && cmd_bytes.get(extra_off - 1) == Some(&b'|') {
            extra_off - 1
        } else {
            extra_off
        };
        tagp.command_end = Some(cmd_end);
        let mut off = extra_off + 2;
        let mut kind_start: Option<usize> = None;
        let mut ud_start: Option<usize> = None;
        if cmd_bytes.get(off) == Some(&TAB) {
            off += 1;
            loop {
                let p_bytes = &cmd_bytes[off..];
                if !p_bytes.first().is_some_and(u8::is_ascii_alphabetic)
                    && utfc_ptr2len(p_bytes) <= 1
                {
                    break;
                }
                if p_bytes.starts_with(b"kind:") {
                    kind_start = Some(off + 5);
                } else if p_bytes.starts_with(b"user_data:") {
                    ud_start = Some(off + 10);
                } else if let Some(rest) = p_bytes.strip_prefix(b"line:") {
                    tagp.tagline = crate::charset::parse_i64_prefix(rest) as i32;
                }
                if kind_start.is_some() && ud_start.is_some() {
                    break;
                }

                let colon_pos = p_bytes.iter().position(|&b| b == b':');
                let tab_pos = p_bytes.iter().position(|&b| b == b'\t');
                if colon_pos.is_none()
                    || tab_pos.is_some_and(|t| colon_pos.unwrap_or(usize::MAX) > t)
                {
                    kind_start = Some(off);
                }
                let Some(t_idx) = tab_pos else {
                    break;
                };
                off += t_idx;
                off += crate::mbyte::utfc_ptr2len(&cmd_bytes[off..]) as usize;
            }
        }
        if let Some(ks) = kind_start {
            let kind_bytes = &cmd_bytes[ks..];
            let end_idx = kind_bytes
                .iter()
                .position(|&b| matches!(b, b'\t' | b'\r' | b'\n'))
                .unwrap_or(kind_bytes.len());
            tagp.tagkind = Some(&kind_bytes[..end_idx]);
            tagp.tagkind_range = Some((ks, ks + end_idx));
        }
        if let Some(us) = ud_start {
            let ud_bytes = &cmd_bytes[us..];
            let end_idx = ud_bytes
                .iter()
                .position(|&b| matches!(b, b'\t' | b'\r' | b'\n'))
                .unwrap_or(ud_bytes.len());
            tagp.user_data = Some(&ud_bytes[..end_idx]);
        }
    }
    true
}

fn tag_full_fname(tagp: &mut Tagptrs) -> std::ffi::CString {
    let fname_cstr = std::ffi::CString::new(tagp.fname).unwrap_or_default();
    expand_tag_fname(&fname_cstr, tagp.tag_fname.unwrap_or(c""), false)
}

fn jumpto_tag(lbuf_arg: &[u8], forceit: bool, keep_help: bool) -> Option<bool> {
    if g_st().postponed_split == 0 && !check_can_set_curbuf_forceit(forceit) {
        return Some(false);
    }

    let mut tofree_fname: Option<std::ffi::CString> = None;
    let mut tagp = Tagptrs::default();
    let mut retval = Some(false);
    let mut getfile_result = GetFileResult::Unused;
    let mut curwin_save = crate::globals::GraphPtr::<WinInfoInfo>::NULL;
    let mut full_fname: Option<std::ffi::CString> = None;
    let old_key_typed = g_st().key_typed;
    let l_g_do_tagpreview = g_st().g_do_tagpreview;
    let len = matching_line_len(lbuf_arg) + 1;
    let mut lbuf_vec = lbuf_arg[..len.min(lbuf_arg.len())].to_vec();
    if lbuf_vec.len() < len {
        lbuf_vec.resize(len, 0);
    }

    let mut pbuf_vec = vec![0u8; LSIZE as usize];
    let pbuf = pbuf_vec.as_mut_ptr();

    'erret: {
        if !parse_match(&lbuf_vec, &mut tagp) {
            break 'erret;
        }

        let mut pbuf_len: usize = 0;
        for &b in tagp.command {
            if b == b'\n' || b == b'\r' {
                break;
            }
            pbuf_vec[pbuf_len] = b;
            pbuf_len += 1;
            if pbuf_len as i32 + 1 >= LSIZE {
                break;
            }
        }
        pbuf_vec[pbuf_len] = 0;

        if let Some(extra_off) = find_extra(&pbuf_vec[..pbuf_len]) {
            pbuf_len = if extra_off > 0 && pbuf_vec[extra_off - 1] == b'|' {
                extra_off - 1
            } else {
                extra_off
            };
            pbuf_vec[pbuf_len] = 0;
        }
        let pbuf_end = pbuf.wrapping_add(pbuf_len);

        let fname_cstr = std::ffi::CString::new(tagp.fname).unwrap_or_default();
        let expanded = expand_tag_fname(&fname_cstr, tagp.tag_fname.unwrap_or(c""), true);
        tofree_fname = Some(expanded);

        let fname_cstr_ref = tofree_fname.as_deref().unwrap();
        let mut fname_opt_ref = Some(fname_cstr_ref);
        if !os_path_exists(fname_cstr_ref) && !has_autocmd(EVENT_BUFREADCMD, fname_opt_ref, None) {
            retval = None;
            tag_st().nofile_fname = tofree_fname.take();
            break 'erret;
        }

        g_st().redrawing_disabled += 1;

        if l_g_do_tagpreview != 0 {
            g_st().postponed_split = 0;
            curwin_save = crate::globals::GraphPtr::<WinInfoInfo>::of(curwin_ptr());

            if !curwin_mut().w_onebuf_opt.wo_pvw {
                full_fname = FullName_save(fname_opt_ref, false);
                fname_opt_ref = full_fname.as_deref();

                prepare_tagpreview(true, !opt_vars().p_pvp.is_empty());
            }
        }

        if g_st().postponed_split != 0
            && (opt_vars().swb_flags & (K_OPT_SWB_FLAG_USEOPEN | K_OPT_SWB_FLAG_USETAB)) != 0
            && let Some(existing_buf) = buflist_findname_exp(fname_opt_ref)
            && swbuf_goto_win_with_buf(Some(existing_buf)).is_some()
        {
            getfile_result = GetFileResult::SameFile;
        }
        if getfile_result == GetFileResult::Unused
            && (g_st().postponed_split != 0 || g_st().cmdmod.cmod_tab != 0)
        {
            if (opt_vars().swb_flags & K_OPT_SWB_FLAG_VSPLIT) != 0 {
                g_st().cmdmod.cmod_split |= WSP_VERT;
            }
            if (opt_vars().swb_flags & K_OPT_SWB_FLAG_NEWTAB) != 0 && g_st().cmdmod.cmod_tab == 0 {
                g_st().cmdmod.cmod_tab = tabpage_index(g_st().curtab.opt()) + 1;
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
            curwin_mut().w_onebuf_opt.wo_scb = false;
            curwin_mut().w_onebuf_opt.wo_crb = false;
        }

        if keep_help {
            if l_g_do_tagpreview != 0 {
                g_st().keep_help_flag = bt_help(curwin_save.buf_opt());
            } else {
                g_st().keep_help_flag = curbuf_mut().b_help;
            }
        }

        if getfile_result == GetFileResult::Unused {
            getfile_result = getfile(0, fname_opt_ref, None, true, 0, forceit);
        }
        g_st().keep_help_flag = false;

        if getfile_result.is_success() {
            curwin_mut().w_set_curswant = true;
            g_st().postponed_split = 0;

            let save_no_hlsearch = g_st().search_global.no_hlsearch;

            let search_options =
                if crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_TAGPAT) {
                    0
                } else {
                    SEARCH_KEEP
                };

            let mut str_ = pbuf;
            if pbuf_vec[0] == b'/' || pbuf_vec[0] == b'?' {
                str_ = skip_regexp(
                    crate::memory::bytes_as_cstr(&pbuf_vec[1..]),
                    i32::from(pbuf_vec[0]),
                    0,
                )
                .as_ptr()
                .wrapping_add(1)
                .cast_mut()
                .cast();
            }
            if str_ > pbuf_end.wrapping_sub(1) {
                let mut pbuflen = pbuf_len;

                let save_p_ws = opt_vars().p_ws;
                let save_p_ic = opt_vars().p_ic;
                let save_p_scs = opt_vars().p_scs;
                opt_vars().p_ws = true;
                opt_vars().p_ic = false;
                opt_vars().p_scs = false;
                let save_lnum = curwin_mut().w_cursor.lnum;

                curwin_mut().w_cursor.lnum = if tagp.tagline > 0 {
                    tagp.tagline - 1
                } else {
                    0
                };

                let search_dirc = i32::from(pbuf_vec[0]);
                let pbuf1_cstr = crate::memory::bytes_as_cstr(&pbuf_vec[1..]);
                let pbuf1_opt = Some(pbuf1_cstr);
                if do_search(
                    None,
                    search_dirc,
                    search_dirc,
                    pbuf1_opt,
                    pbuflen - 1,
                    1,
                    search_options,
                    false,
                    None,
                ) != 0
                {
                    retval = Some(true);
                } else {
                    let mut found = 1;
                    opt_vars().p_ic = true;
                    if do_search(
                        None,
                        search_dirc,
                        search_dirc,
                        pbuf1_opt,
                        pbuflen - 1,
                        1,
                        search_options,
                        false,
                        None,
                    ) == 0
                    {
                        found = 2;
                        test_for_static(&tagp);
                        let tagname_cstr = std::ffi::CString::new(tagp.tagname).unwrap_or_default();
                        pbuflen = format_to_buf!(
                            &mut pbuf_vec[..LSIZE as usize],
                            c"^%s\\s\\*(",
                            &tagname_cstr,
                        ) as usize;
                        let pbuf_cstr = crate::memory::bytes_as_cstr(&pbuf_vec);
                        if do_search(
                            None,
                            b'/' as i32,
                            b'/' as i32,
                            Some(pbuf_cstr),
                            pbuflen,
                            1,
                            search_options,
                            false,
                            None,
                        ) == 0
                        {
                            pbuflen = format_to_buf!(
                                &mut pbuf_vec[..LSIZE as usize],
                                c"^\\[#a-zA-Z_]\\.\\*\\<%s\\s\\*(",
                                &tagname_cstr,
                            ) as usize;
                            let pbuf_cstr = crate::memory::bytes_as_cstr(&pbuf_vec);
                            if do_search(
                                None,
                                b'/' as i32,
                                b'/' as i32,
                                Some(pbuf_cstr),
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
                    }
                    if found == 0 {
                        emsg(gettext(c"E434: Can't find tag pattern"));
                        curwin_mut().w_cursor.lnum = save_lnum;
                    } else {
                        if found == 2 || !save_p_ic {
                            msg(gettext(c"E435: Couldn't find tag, just guessing!"), 0);
                            if g_st().msg_scrolled == 0 && g_st().msg_silent == 0 {
                                msg_delay(1010, true);
                            }
                        }
                        retval = Some(true);
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
                do_cmdline_cmd(crate::memory::bytes_as_cstr(&pbuf_vec));
                retval = Some(true);

                if g_st().secure == 2 {
                    wait_return(1);
                }
                g_st().secure = save_secure;
                g_st().sandbox -= 1;
            }

            if search_options != 0 {
                set_no_hlsearch(save_no_hlsearch);
            }

            if getfile_result == GetFileResult::OpenOther {
                retval = Some(true);
            }

            if retval == Some(true) {
                if curbuf_mut().b_help {
                    set_topline(curwin_mut(), curwin_mut().w_cursor.lnum);
                }
                if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_TAG) != 0 && old_key_typed {
                    fold_open_cursor();
                }
            }

            if l_g_do_tagpreview != 0 && curwin_ptr() != curwin_save && win_valid(curwin_save.opt())
            {
                validate_cursor(curwin_mut());
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                win_enter(curwin_save.borrow_mut(), true);
            }

            g_st().redrawing_disabled -= 1;
        } else {
            g_st().redrawing_disabled -= 1;
            if g_st().postponed_split != 0 {
                win_close(curwin_mut(), false, false);
                g_st().postponed_split = 0;
            }
        }
    }

    g_st().g_do_tagpreview = 0;
    drop(tofree_fname);
    drop(full_fname);

    retval
}

fn expand_tag_fname(
    fname_arg: &std::ffi::CStr,
    tag_fname: &std::ffi::CStr,
    expand: bool,
) -> std::ffi::CString {
    {
        let mut xpc: ExpandCtx = ExpandCtx::new();

        let fname_arg_opt = Some(fname_arg);
        let mut fname = to_slash_save(fname_arg_opt).unwrap_or_default();

        if expand
            && path_has_wildcard(fname.as_bytes(), true)
            && !crate::strings::vim_haschr(&(fname), b'`' as i32)
        {
            xpc.xp_context = EXPAND_FILES;
            if let Some(exp) = expand_one(
                &mut xpc,
                Some(fname.as_c_str()),
                None,
                WILD_LIST_NOTFOUND | WILD_SILENT,
                WILD_EXPAND_FREE,
            ) {
                fname = exp;
            }
        }

        if (opt_vars().p_tr || curbuf_mut().b_help) && !vim_isAbsName(fname.to_bytes()) {
            let p = path_tail(tag_fname).as_ptr();
            if p != tag_fname.as_ptr() {
                let mut retval = [0u8; MAXPATHL];
                let off = p as usize - tag_fname.as_ptr() as usize;
                retval[..off].copy_from_slice(&tag_fname.to_bytes()[..off]);
                crate::memory::copy_bytes_to_c_buf(&mut retval[off..], fname.as_bytes());
                simplify_filename(&mut retval);
                return crate::memory::cstring_until_nul(&retval);
            }
        }

        fname
    }
}

fn test_for_current(
    fname: &[u8],
    tag_fname: &std::ffi::CStr,
    buf_ffname: Option<&std::ffi::CStr>,
) -> i32 {
    let mut retval = false;
    if let Some(buf_ffname) = buf_ffname {
        let fname_cstr =
            std::ffi::CString::new(crate::memory::bytes_until_nul(fname)).unwrap_or_default();
        let fullname = expand_tag_fname(&fname_cstr, tag_fname, true);
        retval = path_equal(
            &fullname,
            buf_ffname,
            (K_PATH_CMP_EXPAND | K_PATH_CMP_FULL) as PathCmpFlags,
        );
    }
    i32::from(retval)
}

fn find_extra(bytes: &[u8]) -> Option<usize> {
    let bytes = crate::memory::bytes_until_nul(bytes);
    let mut rem = bytes;

    loop {
        let first_char = rem.first().copied().unwrap_or(0);
        if first_char.is_ascii_digit() {
            let skip_len = 1 + rem[1..].iter().take_while(|b| b.is_ascii_digit()).count();
            rem = &rem[skip_len..];
        } else if first_char == b'/' || first_char == b'?' {
            rem = crate::regexp::skip_regexp_slice(&rem[1..], i32::from(first_char), 0);
            if rem.first().copied() != Some(first_char) {
                return None;
            }
            rem = &rem[1..];
        } else {
            let pos = rem.windows(3).position(|w| w == b"|;\"")?;
            rem = &rem[pos + 1..];
            break;
        }
        if rem.first() != Some(&b';')
            || !rem
                .get(1)
                .is_some_and(|&b| b.is_ascii_digit() || b == b'/' || b == b'?')
        {
            break;
        }
        rem = &rem[1..];
    }

    rem.starts_with(b";\"").then_some(bytes.len() - rem.len())
}

/// Free a single entry in a tag stack.
pub fn tagstack_clear_entry(item: &mut Taggy) {
    item.tagname = None;
    item.user_data = None;
}

pub fn tag_listfiles_extra(idx: usize) -> Option<(&'static CStr, &'static CStr)> {
    tag_st()
        .borrow_mut()
        .tag_listfiles_extra
        .as_slice()
        .get(idx)
        .map(|(k, f)| (k.as_c_str(), f.as_c_str()))
}

pub fn expand_tags(
    tagnames: bool,
    pat: &std::ffi::CStr,
    file: &mut Vec<std::ffi::CString>,
) -> bool {
    let extra_flag = if tagnames { TAG_NAMES } else { 0 };
    let mut raw_matches: Vec<Vec<u8>> = Vec::new();
    let b_ffname = curbuf_mut().ffname_opt();
    let ret = if pat.to_bytes().first() == Some(&b'/') {
        let pat_after = crate::memory::CStrSubslice::advance(pat, 1);
        find_tags(
            pat_after,
            &mut raw_matches,
            TAG_REGEXP | extra_flag | TAG_VERBOSE,
            TAG_MANY,
            b_ffname,
        )
    } else {
        find_tags(
            pat,
            &mut raw_matches,
            TAG_REGEXP | extra_flag | TAG_VERBOSE | TAG_NOIC,
            TAG_MANY,
            b_ffname,
        )
    };
    file.clear();
    tag_st().tag_listfiles_extra.clear();
    if ret {
        file.reserve_exact(raw_matches.len());
        if tagnames {
            for m in raw_matches {
                file.push(crate::memory::cstring_until_nul(&m));
            }
        } else {
            tag_st()
                .tag_listfiles_extra
                .reserve_exact(raw_matches.len());
            for m in raw_matches {
                let mut t_p = Tagptrs::default();
                parse_match(&m, &mut t_p);
                file.push(crate::memory::bytes_to_cstring(t_p.tagname));
                // The kind may be a multi-byte character, copy all of its bytes.
                let kind_bytes: &[u8] = if let Some(k) = t_p.tagkind
                    && !k.is_empty()
                {
                    let kind_len = (utfc_ptr2len(k) as usize).min(k.len());
                    &k[..kind_len]
                } else {
                    b"f"
                };
                let kind_cstr = crate::memory::bytes_to_cstring(kind_bytes);
                let fname_cstr = crate::memory::bytes_to_cstring(t_p.fname);
                tag_st().tag_listfiles_extra.push((kind_cstr, fname_cstr));
            }
        }
    }
    ret
}

fn add_tag_field(
    dict: Option<&mut TvDict>,
    field_name: &std::ffi::CStr,
    val: Option<&[u8]>,
) -> bool {
    if tv_dict_find(dict.as_deref(), field_name, -1).is_some() {
        if opt_vars().p_verbose > 0 {
            verbose_enter();
            smsg!(0, gettext(c"Duplicate field name: %s"), field_name);
            verbose_leave();
        }
        return false;
    }
    let mut len: usize = 0;
    let mut buf_vec = vec![0u8; MAXPATHL];
    if let Some(mut slice) = val {
        while slice.last().is_some_and(|&b| b == b'\r' || b == b'\n') {
            slice = &slice[..slice.len() - 1];
        }
        len = slice.len().min(MAXPATHL - 1);
        buf_vec[..len].copy_from_slice(&slice[..len]);
    }
    buf_vec[len] = 0;
    tv_dict_add_str(
        dict.unwrap(),
        field_name,
        Some(CStr::from_bytes_until_nul(&buf_vec).unwrap_or_default()),
    )
}

/// Add the tags matching the specified pattern `pat` to the list `list`
/// as a dictionary. Use `buf_fname` for priority, unless NULL.
pub fn get_tags(
    list: &mut TvList,
    pat: &std::ffi::CStr,
    buf_fname: Option<&std::ffi::CStr>,
) -> bool {
    {
        let mut matches: Vec<Vec<u8>> = Vec::new();

        let mut ret = find_tags(pat, &mut matches, TAG_REGEXP | TAG_NOIC, MAXCOL, buf_fname);
        if !ret || matches.is_empty() {
            return ret;
        }

        for match_vec in matches {
            let mut tp = Tagptrs::default();
            if !parse_match(&match_vec, &mut tp) {
                continue;
            }

            let is_static = test_for_static(&tp);

            if tp.tagname.starts_with(b"!_TAG_") {
                continue;
            }

            let dict = tv_dict_alloc();
            tv_list_append_dict(&mut *list, Some(dict));

            let full_fname = tag_full_fname(&mut tp);
            let tag_slice = Some(tp.tagname);
            let cmd_slice = Some(match tp.command_end {
                Some(end) => &tp.command[..end.min(tp.command.len())],
                None => tp.command,
            });
            let kind_slice = tp.tagkind;
            if !add_tag_field(Some(&mut *dict), c"name", tag_slice)
                || !add_tag_field(Some(&mut *dict), c"filename", Some(full_fname.as_bytes()))
                || !add_tag_field(Some(&mut *dict), c"cmd", cmd_slice)
                || !add_tag_field(Some(&mut *dict), c"kind", kind_slice)
                || !tv_dict_add_nr(&mut *dict, c"static", Varnumber::from(is_static))
            {
                ret = false;
            }
            drop(full_fname);

            if let Some(cmd_end) = tp.command_end {
                let cmd_bytes = tp.command;
                let mut off = (cmd_end + 3).min(cmd_bytes.len());
                loop {
                    let p_bytes = &cmd_bytes[off..];
                    if p_bytes.is_empty() || matches!(p_bytes[0], b'\n' | b'\r') {
                        break;
                    }
                    if let Some((ks, ke)) = tp.tagkind_range
                        && (off == ks || (off + 5 == ks && p_bytes.starts_with(b"kind:")))
                    {
                        off = ke.saturating_sub(1);
                    } else if p_bytes.starts_with(b"file:") {
                        off += 4;
                    } else if !ascii_iswhite(p_bytes[0] as i32) {
                        let name_len = p_bytes
                            .iter()
                            .take_while(|&&b| (b' '..127).contains(&b) && b != b':')
                            .count();
                        if p_bytes.get(name_len) == Some(&b':') && name_len > 0 {
                            let val_len = p_bytes[name_len + 1..]
                                .iter()
                                .take_while(|&&b| b >= b' ')
                                .count();
                            let field_name =
                                std::ffi::CString::new(&p_bytes[..name_len]).unwrap_or_default();
                            if !add_tag_field(
                                Some(&mut *dict),
                                &field_name,
                                Some(&p_bytes[name_len + 1..name_len + 1 + val_len]),
                            ) {
                                ret = false;
                            }
                            off += name_len + 1 + val_len;
                        } else {
                            let skip_len = p_bytes[name_len..]
                                .iter()
                                .take_while(|&&b| b >= b' ')
                                .count();
                            off += name_len + skip_len;
                        }
                        if off >= cmd_bytes.len() {
                            break;
                        }
                    }
                    off += crate::mbyte::utfc_ptr2len(&cmd_bytes[off..]) as usize;
                }
            }
        }
        ret
    }
}

fn get_tag_details(tag: &mut Taggy, retdict: &mut TvDict) {
    tv_dict_add_str(retdict, c"tagname", tag.tagname.as_deref());
    tv_dict_add_nr(retdict, c"matchnr", (tag.cur_match + 1) as Varnumber);
    tv_dict_add_nr(retdict, c"bufnr", tag.cur_fnum as Varnumber);
    if let Some(ref ud) = tag.user_data {
        tv_dict_add_str(retdict, c"user_data", Some(ud.as_c_str()));
    }

    let pos = tv_list_alloc(4);
    tv_dict_add_list(retdict, c"from", Some(pos));

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

/// Return the tag stack entries of the specified window `wp` in dictionary `retdict`.
pub fn get_tagstack(wp: &mut WinInfoInfo, retdict: &mut TvDict) {
    tv_dict_add_nr(retdict, c"length", wp.w_tagstacklen as Varnumber);
    tv_dict_add_nr(retdict, c"curidx", (wp.w_tagstackidx + 1) as Varnumber);
    let l = tv_list_alloc(2);
    tv_dict_add_list(retdict, c"items", Some(l));

    for i in 0..wp.w_tagstacklen as usize {
        let d = tv_dict_alloc();
        tv_list_append_dict(l, Some(d));
        get_tag_details(&mut wp.w_tagstack[i], d);
    }
}

fn tagstack_clear(wp: &mut WinInfoInfo) {
    for i in 0..wp.w_tagstacklen as usize {
        tagstack_clear_entry(&mut wp.w_tagstack[i]);
    }
    wp.w_tagstacklen = 0;
    wp.w_tagstackidx = 0;
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

fn tagstack_push_items(wp: &mut WinInfoInfo, l: Option<&TvList>) {
    for li in tv_list_iter(l) {
        if li.li_tv.v_type == VarType::VarDict
            && let Some(itemdict) = li.li_tv.vval.v_dict_opt()
            && let Some(di) = tv_dict_find_opt(Some(itemdict), c"from", -1)
        {
            let mut mark = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut fnum: i32 = 0;
            if list2fpos(&mut di.di_tv, &mut mark, Some(&mut fnum), None, false)
                && let Some(tagname) = tv_dict_get_string(Some(itemdict), c"tagname")
            {
                if mark.col > 0 {
                    mark.col -= 1;
                }
                let ud_ptr = tv_dict_get_string(Some(itemdict), c"user_data");
                tagstack_push_item(
                    wp,
                    Some(tagname.to_owned()),
                    tv_dict_get_number(Some(itemdict), c"bufnr") as i32,
                    tv_dict_get_number(Some(itemdict), c"matchnr") as i32 - 1,
                    mark,
                    fnum,
                    ud_ptr.map(std::ffi::CStr::to_owned),
                );
            }
        }
    }
}

fn tagstack_set_curidx(wp: &mut WinInfoInfo, curidx: i32) {
    wp.w_tagstackidx = curidx.max(0).min(wp.w_tagstacklen);
}

/// Set the tag stack entries of the specified window.
pub fn set_tagstack(wp: &mut WinInfoInfo, d: Option<&TvDict>, action: i32) -> bool {
    if tag_st().tfu_in_use {
        emsg(gettext(E_CANNOT_MODIFY_TAG_STACK_WITHIN_TAGFUNC));
        return false;
    }

    let mut l: Option<&TvList> = None;

    if let Some(di_items) = tv_dict_find_opt(d, c"items", -1) {
        if di_items.di_tv.v_type != VarType::VarList {
            emsg(gettext(E_LISTREQ));
            return false;
        }
        l = di_items.di_tv.vval.v_list_opt().map(|r| &*r);
    }

    if let Some(di_curidx) = tv_dict_find_opt(d, c"curidx", -1) {
        tagstack_set_curidx(wp, tv_get_number(&di_curidx.di_tv) as i32 - 1);
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

    if l.is_some() {
        if action == b'r' as i32 {
            tagstack_clear(wp);
        }

        tagstack_push_items(&mut *wp, l);
        wp.w_tagstackidx = wp.w_tagstacklen;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_strnicmp_compares_ascii_case_folded_to_uppercase() {
        assert_eq!(tag_strnicmp(b"abc", b"ABC", 3), 0);
        assert!(tag_strnicmp(b"abc", b"_bc", 3) < 0);
    }
}
