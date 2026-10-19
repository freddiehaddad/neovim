//! Functions for Insert mode completion (`src/nvim/insexpand.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::blocks_in_conditions,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::if_same_then_else,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::useless_conversion
)]

#[allow(unused_imports)]
use crate::api::private::helpers::copy_string;
#[allow(unused_imports)]
use crate::buffer::{bt_cmdwin, buf_spname};
#[allow(unused_imports)]
use crate::change::deleted_lines_mark;
#[allow(unused_imports)]
use crate::charset::{getwhitecols, str_foldcase, vim_isIDc};
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::set_ref_in_callback;
#[allow(unused_imports)]
use crate::eval::typval::{
    callback_copy, tv_dict_get_number, tv_dict_get_string, tv_get_string, tv_get_string_chk,
};
#[allow(unused_imports)]
use crate::eval::userfunc::callback_call_retnr;
#[allow(unused_imports)]
use crate::eval::{callback_call, get_v_event, restore_v_event};
#[allow(unused_imports)]
use crate::fileio::fclose;
#[allow(unused_imports)]
use crate::fileio::vim_fgets;
use crate::globals::*;
#[allow(unused_imports)]
use crate::input::using_script;
#[allow(unused_imports)]
use crate::lua::executor::nlua_expand_pat;
#[allow(unused_imports)]
use crate::memline::dec;
#[allow(unused_imports)]
use crate::memory::strequal;
#[allow(unused_imports)]
use crate::message::msg_delay;
#[allow(unused_imports)]
use crate::message::msg_progress;
use crate::message::semsg;
#[allow(unused_imports)]
use crate::message::{emsg, internal_error, msg, msg_clr_cmdline, msg_ext_set_kind};
#[allow(unused_imports)]
use crate::option::copy_option_part;
#[allow(unused_imports)]
use crate::option::was_set_insecurely;
#[allow(unused_imports)]
use crate::option::{can_bs, option_set_callback_func, shortmess};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::path::{path_has_drive_letter, path_to_slash};
#[allow(unused_imports)]
use crate::regexp::vim_regexec;
#[allow(unused_imports)]
use crate::regexp::{vim_regcomp, vim_regfree};
#[allow(unused_imports)]
use crate::strings::vim_snprintf;
#[allow(unused_imports)]
use crate::strings::vim_strnicmp;
#[allow(unused_imports)]
use crate::strings::{atoi, gettext, qsort, strcat, strchr, strcmp, strcpy, strlen, strncmp};
#[allow(unused_imports)]
use crate::strings::{memcmp, strncasecmp, strrchr};
#[allow(unused_imports)]
use crate::strings::{strncpy, tolower};
#[allow(unused_imports)]
use crate::strings::{vim_strchr, vim_strsave_escaped, xstrnsave};
#[allow(unused_imports)]
use crate::ui::{ui_flush, ui_has, vim_beep};
#[allow(unused_imports)]
use crate::window::win_valid;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::command::regmatch_T;
use crate::api::tabpage::{pos_T, win_T};
use crate::autocmd::{
    EVENT_COMPLETECHANGED, EVENT_COMPLETEDONE, EVENT_COMPLETEDONEPRE, apply_autocmds, has_event,
};
use crate::buffer::buf_valid;
use crate::change::{ins_bytes_len, ins_char, ins_char_bytes, ins_str, open_line};
use crate::charset::{skipwhite, vim_isfilec, vim_isprintc, vim_iswordc, vim_iswordp, vim_strsize};
use crate::clipboard::{VarType, list_T, listitem_T, typval_T};
use crate::cmdexpand::{addstar, expand_cmdline, free_xp_files_extra, set_cmd_context};
use crate::cursor::{
    check_cursor, colnr_T, dec_cursor, get_cursor_line_len, get_cursor_line_ptr,
    get_cursor_pos_len, get_cursor_pos_ptr, inc_cursor, linenr_T,
};
use crate::drawscreen::{conceal_cursor_line, redrawWinline, setcursor, showmode, update_screen};
use crate::eval::buffer::varnumber_T;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{Callback, EvalFuncData, buf_T};
use crate::eval::typval::{
    VarLockStatus, callback_free, tv_clear, tv_dict_add_bool, tv_dict_add_dict, tv_dict_add_list,
    tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_add_tv, tv_dict_alloc,
    tv_dict_alloc_lock, tv_dict_alloc_ret, tv_dict_find, tv_dict_get_tv, tv_dict_set_keys_readonly,
    tv_dict_unref, tv_get_number_chk, tv_list_alloc, tv_list_append_dict, tv_list_unref,
};
use crate::eval::vars::{VV_COMPLETED_ITEM, set_vim_var_dict};
use crate::ex_eval::aborting;
use crate::ex_getln::tilde_replace;
use crate::extmark::{ExtmarkOp, ExtmarkUndoObject, extmark_apply_undo, extmark_splice_delete};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzzy_match_str, fuzzy_match_str_in_line, search_for_fuzzy_match,
};
use crate::garray::{ga_append_via_ptr, ga_clear, ga_grow, ga_init, garray_T};
use crate::hashtab::{
    hash_T, hash_add_item, hash_clear_all, hash_find, hash_hash, hash_init, hash_lookup,
    hash_remove, hashitem_T, hashitem_empty, hashtab_T,
};
use crate::highlight_group::{syn_check_group, syn_id2attr, syn_id2name, syn_name2attr};
use crate::indent::{get_indent, inindent};
use crate::indent_c::{do_c_expr_indent, in_cinkeys};
use crate::input::{
    redo_append_char, redo_append_lit, safe_vgetc, vgetc, vpeekc, vpeekc_any, vungetc,
};
use crate::insert::{
    backspace_until_column, get_can_cindent, ins_apply_autocmds, ins_eol, ins_redraw, insertchar,
    start_arrow, stop_arrow,
};
use crate::keycodes::{
    CAR, K_BS, K_COMMAND, K_DOWN, K_IGNORE, K_KENTER, K_KPAGEDOWN, K_KPAGEUP, K_MOUSEDOWN,
    K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP, K_PAGEDOWN, K_PAGEUP, K_S_DOWN, K_S_TAB,
    K_S_UP, K_UP, KE_EVENT, KE_FILLER, KE_LUA, KS_EXTRA, NL, TAB, termcap2key,
};
use crate::kvec::KVec;
use crate::map_glyph_cache::NvimString;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_get_class, mb_islower, mb_isupper, mb_prevptr, mb_ptr2char_adv, mb_tolower, mb_toupper,
    ptr2cells, utf_char2bytes, utf_char2len, utf_head_off, utf_ptr2char, utf_ptr2len, utf8len_tab,
    utfc_ptr2len,
};
use crate::memline::{ml_delete, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len};
use crate::memory::{
    MergeSortCompareFunc, mergesort_list, xcalloc, xfree, xmalloc, xmemcpyz, xmemdupz, xstrdup,
    xstrlcpy,
};
use crate::r#move::{changed_cline_bef_curs, curs_columns, validate_cursor};
use crate::optionstr::{OptIndex, optset_T};
use crate::os::env::expand_T;
use crate::os::fs::os_fopen;
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::os::time::os_hrtime;
use crate::path::{EW_ADDSLASH, EW_DIR, EW_FILE, EW_SILENT, FreeWild, expand_wildcards, path_tail};
use crate::popupmenu::{
    pum_clear, pum_display, pum_get_height, pum_set_event_info, pum_undisplay, pum_visible,
    pumitem_T,
};
use crate::register::{
    NUM_REGISTERS, copy_register, free_register, get_register_name, valid_yank_reg,
};
use crate::search::{Direction, find_pattern_in_path, ignorecase, search_for_exact_line, searchit};
use crate::spell::{
    expand_spelling, spell_dump_compl, spell_expand_check_cap, spell_move_to, spell_word_start,
};
use crate::state::{MODE_INSERT, REPLACE_FLAG, may_trigger_modechanged};
use crate::tag::find_tags;
use crate::textformat::auto_format;
use crate::undo::undo_allowed;
use crate::winfloat::{WinKind, win_float_find};

pub use crate::api::win_config::UIExtension;
pub use crate::ex_getln::hlf_T;
pub use crate::option::OptInt;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum cpitem_T {
    CPT_ABBR = 0,
    CPT_KIND = 1,
    CPT_MENU = 2,
    CPT_INFO = 3,
    CPT_COUNT = 4,
}

pub const CPT_ABBR: usize = 0;
pub const CPT_KIND: usize = 1;
pub const CPT_MENU: usize = 2;
pub const CPT_INFO: usize = 3;
pub const CPT_COUNT: usize = 4;

const NUL: c_char = 0;
const NOTDONE: c_int = 2;
const FORWARD: Direction = 1;
const BACKWARD: Direction = -1;
const kDirectionNotSet: Direction = 0;

const FIND_ANY: c_int = 1;
const FIND_DEFINE: c_int = 2;
const ACTION_EXPAND: c_int = 5;

const MAXCOL: colnr_T = 0x7fffffff;
const MAXLNUM: linenr_T = 0x7fffffff;
const IOSIZE: usize = 1025;
const LSIZE: usize = 512;
const MB_MAXCHAR: usize = 6;
const MIN_SPACE: c_int = 75;
const LOOKBACK_LINE_COUNT: linenr_T = 1000;
const PATHSEP: c_char = b'/' as c_char;

const K_EVENT: c_int = termcap2key(KS_EXTRA as c_int, KE_EVENT as c_int);
const K_LUA: c_int = termcap2key(KS_EXTRA as c_int, KE_LUA as c_int);
const K_SELECT: c_int = termcap2key(0xf5, KE_FILLER as c_int);

const Ctrl_C: c_int = 3;
const Ctrl_D: c_int = 4;
const Ctrl_E: c_int = 5;
const Ctrl_F: c_int = 6;
const Ctrl_I: c_int = 9;
const Ctrl_K: c_int = 11;
const Ctrl_L: c_int = 12;
const Ctrl_N: c_int = 14;
const Ctrl_O: c_int = 15;
const Ctrl_P: c_int = 16;
const Ctrl_Q: c_int = 17;
const Ctrl_R: c_int = 18;
const Ctrl_S: c_int = 19;
const Ctrl_T: c_int = 20;
const Ctrl_U: c_int = 21;
const Ctrl_V: c_int = 22;
const Ctrl_X: c_int = 24;
const Ctrl_Y: c_int = 25;
const Ctrl_Z: c_int = 26;
const Ctrl_RSB: c_int = 29;

const kInsNone: c_int = 0;
const kCallbackNone: c_int = 0;
const kExtmarkUndo: ExtmarkOp = 1;
const kWinInfo: WinKind = 1;
const kUIMessages: UIExtension = 4;
const kListLenMayKnow: isize = -1;

const VAR_UNKNOWN: VarType = VarType::VAR_UNKNOWN;
const VAR_NUMBER: VarType = VarType::VAR_NUMBER;
const VAR_STRING: VarType = VarType::VAR_STRING;
const VAR_LIST: VarType = VarType::VAR_LIST;
const VAR_DICT: VarType = VarType::VAR_DICT;
const VAR_SPECIAL: VarType = VarType::VAR_SPECIAL;
const VAR_FIXED: VarLockStatus = 2;

const HLF_E: hlf_T = 6;
const HLF_R: hlf_T = 18;
const HLF_W: hlf_T = 26;
const HLF_COUNT: hlf_T = 76;

const BS_START: c_int = b's' as c_int;
const KEY_COMPLETE: c_int = 0x103;
const OPENLINE_KEEPTRAIL: c_int = 0x04;
const OPENLINE_FORCE_INDENT: c_int = 0x40;

const OPT_GLOBAL: c_int = 0x01;
const OPT_LOCAL: c_int = 0x02;
const kOptComplete: OptIndex = 51;

const kOptBoFlagComplete: c_uint = 0x08;

const kOptCotFlagFuzzy: c_uint = 0x01;
const kOptCotFlagLongest: c_uint = 0x02;
const kOptCotFlagMenu: c_uint = 0x04;
const kOptCotFlagMenuone: c_uint = 0x08;
const kOptCotFlagNearest: c_uint = 0x10;
const kOptCotFlagNoinsert: c_uint = 0x20;
const kOptCotFlagNoselect: c_uint = 0x40;
const kOptCotFlagNosort: c_uint = 0x80;
const kOptCotFlagPreinsert: c_uint = 0x200;
const kOptCotFlagPreselect: c_uint = 0x400;

const kShmCompletionmenu: c_int = b'c' as c_int;
const kShmCompletionscan: c_int = b'C' as c_int;

const EXPAND_UNSUCCESSFUL: c_int = -2;
const EXPAND_OK: c_int = -1;
const EXPAND_NOTHING: c_int = 0;
const EXPAND_FILES: c_int = 2;
const EXPAND_LUA: c_int = 63;

const RE_MAGIC: c_int = 1;
const RE_LAST: c_int = 2;
const SEARCH_NFMSG: c_int = 0x08;
const SEARCH_KEEP: c_int = 0x400;

const TAG_NAMES: c_int = 2;
const TAG_REGEXP: c_int = 4;
const TAG_NOIC: c_int = 8;
const TAG_VERBOSE: c_int = 32;
const TAG_INS_COMP: c_int = 64;
const TAG_MANY: c_int = 300;

const SMT_ALL: c_int = 0;

const CTRL_X_WANT_IDENT: c_int = 0x100;
const CTRL_X_NORMAL: c_int = 0;
const CTRL_X_NOT_DEFINED_YET: c_int = 1;
const CTRL_X_SCROLL: c_int = 2;
const CTRL_X_WHOLE_LINE: c_int = 3;
const CTRL_X_FILES: c_int = 4;
const CTRL_X_TAGS: c_int = 5 + CTRL_X_WANT_IDENT;
const CTRL_X_PATH_PATTERNS: c_int = 6 + CTRL_X_WANT_IDENT;
const CTRL_X_PATH_DEFINES: c_int = 7 + CTRL_X_WANT_IDENT;
const CTRL_X_FINISHED: c_int = 8;
const CTRL_X_DICTIONARY: c_int = 9 + CTRL_X_WANT_IDENT;
const CTRL_X_THESAURUS: c_int = 10 + CTRL_X_WANT_IDENT;
const CTRL_X_CMDLINE: c_int = 11;
const CTRL_X_FUNCTION: c_int = 12;
const CTRL_X_OMNI: c_int = 13;
const CTRL_X_SPELL: c_int = 14;
const CTRL_X_LOCAL_MSG: c_int = 15;
const CTRL_X_EVAL: c_int = 16;
const CTRL_X_CMDLINE_CTRL_X: c_int = 17;
const CTRL_X_BUFNAMES: c_int = 18;
const CTRL_X_REGISTER: c_int = 19;

const CP_ORIGINAL_TEXT: c_int = 1;
const CP_FREE_FNAME: c_int = 2;
const CP_CONT_S_IPOS: c_int = 4;
const CP_EQUAL: c_int = 8;
const CP_ICASE: c_int = 16;
const CP_FAST: c_int = 32;

const COMPL_INITIAL_TIMEOUT_MS: u64 = 80;
const COMPL_MIN_TIMEOUT_MS: u64 = 5;
const COMPL_FUNC_TIMEOUT_MS: u64 = 300;
const COMPL_FUNC_TIMEOUT_NON_KW_MS: u64 = 1000;

const CONT_ADDING: c_int = 1;
const CONT_INTRPT: c_int = 2 + 4;
const CONT_N_ADDS: c_int = 4;
const CONT_S_IPOS: c_int = 8;
const CONT_SOL: c_int = 16;
const CONT_LOCAL: c_int = 32;

const DICT_FIRST: c_int = 1;
const DICT_EXACT: c_int = 2;

const CI_WHAT_MODE: c_int = 0x01;
const CI_WHAT_PUM_VISIBLE: c_int = 0x02;
const CI_WHAT_ITEMS: c_int = 0x04;
const CI_WHAT_SELECTED: c_int = 0x08;
const CI_WHAT_COMPLETED: c_int = 0x10;
const CI_WHAT_MATCHES: c_int = 0x20;
const CI_WHAT_PREINSERTED_TEXT: c_int = 0x40;
const CI_WHAT_AUTO: c_int = 0x80;
const CI_WHAT_ALL: c_int = 0xff;

const INS_COMPL_CPT_OK: c_int = 1;
const INS_COMPL_CPT_CONT: c_int = 2;
const INS_COMPL_CPT_END: c_int = 3;

static E_COMPLDEL: &CStr = c"E840: Completion function deleted text";

static CTRL_X_MSGS: [Option<&CStr>; 20] = [
    Some(c" Keyword completion (^N^P)"),
    Some(c" ^X mode (^]^D^E^F^I^K^L^N^O^P^Rs^U^V^Y)"),
    None,
    Some(c" Whole line completion (^L^N^P)"),
    Some(c" File name completion (^F^N^P)"),
    Some(c" Tag completion (^]^N^P)"),
    Some(c" Path pattern completion (^N^P)"),
    Some(c" Definition completion (^D^N^P)"),
    None,
    Some(c" Dictionary completion (^K^N^P)"),
    Some(c" Thesaurus completion (^T^N^P)"),
    Some(c" Command-line completion (^V^N^P)"),
    Some(c" User defined completion (^U^N^P)"),
    Some(c" Omni completion (^O^N^P)"),
    Some(c" Spelling suggestion (^S^N^P)"),
    Some(c" Keyword Local completion (^N^P)"),
    None,
    Some(c" Command-line completion (^V^N^P)"),
    None,
    Some(c" Register completion (^N^P)"),
];

static CTRL_X_MODE_NAMES: [Option<&CStr>; 20] = [
    Some(c"keyword"),
    Some(c"ctrl_x"),
    Some(c"scroll"),
    Some(c"whole_line"),
    Some(c"files"),
    Some(c"tags"),
    Some(c"path_patterns"),
    Some(c"path_defines"),
    Some(c"unknown"),
    Some(c"dictionary"),
    Some(c"thesaurus"),
    Some(c"cmdline"),
    Some(c"function"),
    Some(c"omni"),
    Some(c"spell"),
    None,
    Some(c"eval"),
    Some(c"cmdline"),
    None,
    Some(c"register"),
];

#[inline]
fn ctrl_x_msg(i: c_int) -> *const c_char {
    let idx = (i & !CTRL_X_WANT_IDENT) as usize;
    match CTRL_X_MSGS.get(idx).copied().flatten() {
        Some(s) => s.as_ptr(),
        None => ptr::null(),
    }
}

#[inline]
const fn empty_nvim_string() -> NvimString {
    NvimString {
        data: ptr::null_mut(),
        size: 0,
    }
}

#[inline]
unsafe fn api_clear_string(s: *mut NvimString) {
    unsafe {
        xfree((*s).data.cast());
        *s = empty_nvim_string();
    }
}

#[inline]
unsafe fn cbuf_as_string(data: *mut c_char, size: usize) -> NvimString {
    NvimString { data, size }
}

#[inline]
unsafe fn cbuf_to_string(data: *const c_char, size: usize) -> NvimString {
    unsafe {
        NvimString {
            data: xmemdupz(data.cast(), size).cast(),
            size,
        }
    }
}

#[inline]
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
unsafe fn kv_destroy_extmarks(v: *mut KVec<ExtmarkUndoObject>) {
    unsafe {
        xfree((*v).items.cast());
        *v = KVec::new();
    }
}

#[inline]
unsafe fn cstr_as_string(str_ptr: *mut c_char) -> NvimString {
    unsafe {
        if str_ptr.is_null() {
            empty_nvim_string()
        } else {
            NvimString {
                data: str_ptr,
                size: strlen(str_ptr),
            }
        }
    }
}

#[inline]
const fn ascii_isdigit(c: c_char) -> bool {
    (c as u8).is_ascii_digit()
}

#[inline]
const fn ascii_iswhite(c: c_int) -> bool {
    c == b' ' as c_int || c == b'\t' as c_int
}

#[inline]
const fn ascii_iswhite_or_nul(c: c_char) -> bool {
    c == b' ' as c_char || c == b'\t' as c_char || c == NUL
}

#[inline]
fn mb_byte2len(b: u8) -> c_int {
    c_int::from(utf8len_tab[b as usize])
}

#[inline]
unsafe fn mb_ptr_adv(p: *mut *mut c_char) {
    unsafe {
        *p = (*p).add(utf_ptr2len(*p) as usize);
    }
}

#[inline]
unsafe fn mb_ptr_adv_const(p: *mut *const c_char) {
    unsafe {
        *p = (*p).add(utf_ptr2len(*p) as usize);
    }
}

#[inline]
unsafe fn mb_ptr_back(s: *const c_char, p: *mut *mut c_char) {
    unsafe {
        let off = utf_head_off(s, (*p).sub(1));
        *p = (*p).sub(1 + off as usize);
    }
}

#[inline]
const fn vim_ispathsep(c: c_int) -> bool {
    #[cfg(unix)]
    {
        c == PATHSEP as c_int
    }
    #[cfg(windows)]
    {
        c == b':' as c_int || c == b'/' as c_int || c == b'\\' as c_int
    }
}

#[inline]
unsafe fn strnicmp_impl(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    unsafe {
        #[cfg(unix)]
        {
            strncasecmp(s1, s2, n)
        }
        #[cfg(windows)]
        {
            vim_strnicmp(s1, s2, n)
        }
    }
}

/// Structure used to store one match for insert completion (`compl_T`).
#[repr(C)]
pub struct compl_T {
    pub cp_next: *mut compl_T,
    pub cp_prev: *mut compl_T,
    pub cp_match_next: *mut compl_T,
    pub cp_str: NvimString,
    pub cp_text: [*mut c_char; CPT_COUNT],
    pub cp_user_data: typval_T,
    pub cp_fname: *mut c_char,
    pub cp_commit_chars: *mut c_char,
    pub cp_flags: c_int,
    pub cp_number: c_int,
    pub cp_order: c_int,
    pub cp_preselect: bool,
    pub cp_score: c_int,
    pub cp_in_match_array: bool,
    pub cp_user_abbr_hl_id: c_int,
    pub cp_user_kind_hl_id: c_int,
    pub cp_cpt_source_idx: c_int,
}

/// State information used for getting the next set of insert completion matches.
#[repr(C)]
struct ins_compl_next_state_T {
    e_cpt_copy: *mut c_char,
    e_cpt: *mut c_char,
    ins_buf: *mut buf_T,
    cur_match_pos: *mut pos_T,
    prev_match_pos: pos_T,
    set_match_pos: bool,
    first_match_pos: pos_T,
    last_match_pos: pos_T,
    found_all: bool,
    dict: *mut c_char,
    dict_f: c_int,
    func_cb: *mut Callback,
}

#[repr(C)]
struct complstr_T {
    cse_count: c_int,
    cse_str: [c_char; 1],
}

const CSE_OFF: usize = std::mem::offset_of!(complstr_T, cse_str);

#[inline]
unsafe fn hi2cse(hi: *mut hashitem_T) -> *mut complstr_T {
    unsafe { (*hi).hi_key.byte_sub(CSE_OFF).cast::<complstr_T>() }
}

/// Structure for completion source (in `'cpt'` option) information (`cpt_source_T`).
#[repr(C)]
struct cpt_source_T {
    cs_refresh_always: bool,
    cs_startcol: c_int,
    cs_max_matches: c_int,
    compl_start_tv: u64,
    cs_flag: c_char,
}

use crate::state::save_v_event_T;

static mut compl_first_match: *mut compl_T = ptr::null_mut();
static mut compl_curr_match: *mut compl_T = ptr::null_mut();
static mut compl_shown_match: *mut compl_T = ptr::null_mut();
static mut compl_old_match: *mut compl_T = ptr::null_mut();
static mut compl_preselect_match: *mut compl_T = ptr::null_mut();

static mut compl_strings_ht: hashtab_T = unsafe { std::mem::zeroed() };

static mut compl_best_matches: *mut *mut compl_T = ptr::null_mut();
static mut compl_num_bests: c_int = 0;
static mut compl_enter_selects: bool = false;
static mut compl_leader: NvimString = empty_nvim_string();
static mut compl_get_longest: bool = false;
static mut compl_used_match: bool = false;
static mut compl_was_interrupted: bool = false;
static mut compl_interrupted: bool = false;
static mut compl_restarting: bool = false;
static mut compl_started: bool = false;
static mut ctrl_x_mode: c_int = CTRL_X_NORMAL;
static mut compl_matches: c_int = 0;
static mut compl_pattern: NvimString = empty_nvim_string();
static mut cpt_compl_pattern: NvimString = empty_nvim_string();
static mut compl_direction: Direction = FORWARD;
static mut compl_shows_dir: Direction = FORWARD;
static mut compl_pending: c_int = 0;
static mut compl_startpos: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};
static mut compl_length: c_int = 0;
static mut compl_lnum: linenr_T = 0;
static mut compl_col: colnr_T = 0;
static mut compl_ins_end_col: colnr_T = 0;
static mut compl_longest_end_col: colnr_T = 0;
static mut compl_orig_text: NvimString = empty_nvim_string();
static mut compl_orig_extmarks: KVec<ExtmarkUndoObject> = KVec::new();
static mut compl_cont_mode: c_int = 0;
static mut compl_xp: expand_T = unsafe { std::mem::zeroed() };
static mut compl_curr_win: *mut win_T = ptr::null_mut();
static mut compl_curr_buf: *mut buf_T = ptr::null_mut();

static mut compl_autocomplete: bool = false;
static mut compl_autostarted: bool = false;
static mut compl_autostart_pending: bool = false;
static mut compl_autocomplete_pending: bool = false;
static mut compl_autocomplete_start_tv: u64 = 0;
static mut compl_timeout_ms: u64 = COMPL_INITIAL_TIMEOUT_MS;
static mut compl_time_slice_expired: bool = false;
static mut compl_from_nonkeyword: bool = false;
static mut compl_hi_on_autocompl_longest: bool = false;

static mut compl_cont_status: c_int = 0;
static mut compl_opt_refresh_always: bool = false;
static mut spell_bad_len: usize = 0;
static mut compl_selected_item: c_int = -1;
static mut compl_fuzzy_scores: *mut c_int = ptr::null_mut();

static mut cpt_sources_array: *mut cpt_source_T = ptr::null_mut();
static mut cpt_sources_count: c_int = 0;
static mut cpt_sources_index: c_int = -1;

static mut compl_match_array: *mut pumitem_T = ptr::null_mut();
static mut compl_match_arraysize: c_int = 0;

static mut cpt_cb: *mut Callback = ptr::null_mut();
static mut cpt_cb_count: c_int = 0;

#[inline]
unsafe fn decay_compl_timeout() {
    unsafe {
        if compl_timeout_ms > COMPL_MIN_TIMEOUT_MS {
            compl_timeout_ms /= 2;
        }
    }
}

/// Count the string of a new match in the duplicate-check hashtab.
/// "hash" is the hash of "str" when it is not zero.
unsafe fn compl_strings_add(str_ptr: *const c_char, len: usize, mut hash: hash_T) {
    unsafe {
        if compl_strings_ht.ht_array.is_null() {
            hash_init(&raw mut compl_strings_ht);
        }
        if hash == 0 {
            hash = hash_hash(str_ptr);
        }
        let hi = hash_lookup(&raw mut compl_strings_ht, str_ptr, len, hash);
        if hashitem_empty(&*hi) {
            let entry = xmalloc(CSE_OFF + len + 1).cast::<complstr_T>();
            (*entry).cse_count = 1;
            let entry_str = (&raw mut (*entry).cse_str).cast::<c_char>();
            xmemcpyz(entry_str.cast(), str_ptr.cast(), len);
            hash_add_item(&raw mut compl_strings_ht, hi, entry_str, hash);
        } else {
            (*hi2cse(hi)).cse_count += 1;
        }
    }
}

/// CTRL-X pressed in Insert mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_ctrl_x() {
    unsafe {
        if !ctrl_x_mode_cmdline() {
            if (compl_cont_status & CONT_N_ADDS) != 0 {
                compl_cont_status |= CONT_INTRPT;
            } else {
                compl_cont_status = 0;
            }
            ctrl_x_mode = CTRL_X_NOT_DEFINED_YET;
            edit_submode = gettext(ctrl_x_msg(ctrl_x_mode));
            edit_submode_pre = ptr::null();
            redraw_mode = true;
        } else {
            ctrl_x_mode = CTRL_X_CMDLINE_CTRL_X;
        }

        may_trigger_modechanged();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_none() -> bool {
    unsafe { ctrl_x_mode == 0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_normal() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_NORMAL }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_scroll() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_SCROLL }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_whole_line() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_WHOLE_LINE }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_files() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_FILES }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_tags() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_TAGS }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_path_patterns() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_PATH_PATTERNS }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_path_defines() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_PATH_DEFINES }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_dictionary() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_DICTIONARY }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_thesaurus() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_THESAURUS }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_cmdline() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_CMDLINE || ctrl_x_mode == CTRL_X_CMDLINE_CTRL_X }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_function() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_FUNCTION }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_omni() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_OMNI }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_spell() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_SPELL }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_eval() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_EVAL }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_line_or_eval() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_WHOLE_LINE || ctrl_x_mode == CTRL_X_EVAL }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_register() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_REGISTER }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_not_default() -> bool {
    unsafe { ctrl_x_mode != CTRL_X_NORMAL }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctrl_x_mode_not_defined_yet() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_NOT_DEFINED_YET }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compl_status_adding() -> bool {
    unsafe { (compl_cont_status & CONT_ADDING) != 0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compl_status_sol() -> bool {
    unsafe { (compl_cont_status & CONT_SOL) != 0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compl_status_local() -> bool {
    unsafe { (compl_cont_status & CONT_LOCAL) != 0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compl_status_clear() {
    unsafe {
        compl_cont_status = 0;
    }
}

#[inline]
unsafe fn compl_dir_forward() -> bool {
    unsafe { compl_direction == FORWARD }
}

#[inline]
unsafe fn compl_shows_dir_forward() -> bool {
    unsafe { compl_shows_dir == FORWARD }
}

#[inline]
unsafe fn compl_shows_dir_backward() -> bool {
    unsafe { compl_shows_dir == BACKWARD }
}

/// Check that the 'dictionary' or 'thesaurus' option can be used.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_compl_option(dict_opt: bool) -> bool {
    unsafe {
        let empty = if dict_opt {
            *(*curbuf).b_p_dict == NUL && *p_dict == NUL && (*curwin).w_onebuf_opt.wo_spell == 0
        } else {
            *(*curbuf).b_p_tsr == NUL
                && *p_tsr == NUL
                && (*curbuf).b_p_tsrfu.cb_type == kCallbackNone
                && p_tsrfu.cb_type == kCallbackNone
        };
        if empty {
            ctrl_x_mode = CTRL_X_NORMAL;
            edit_submode = ptr::null();
            emsg(gettext(if dict_opt {
                c"'dictionary' option is empty".as_ptr()
            } else {
                c"'thesaurus' option is empty".as_ptr()
            }));
            if emsg_silent == 0 && !in_assert_fails {
                vim_beep(kOptBoFlagComplete);
                setcursor();
                msg_delay(2004, false);
            }
            return false;
        }
        true
    }
}

/// Check that the character "c" a valid key to go to or keep us in CTRL-X mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_is_ctrl_x_key(c: c_int) -> bool {
    unsafe {
        if ins_compl_pum_key(c) {
            return true;
        }

        match ctrl_x_mode {
            0 => c == Ctrl_N || c == Ctrl_P || c == Ctrl_X,
            CTRL_X_NOT_DEFINED_YET | CTRL_X_CMDLINE_CTRL_X => {
                c == Ctrl_X
                    || c == Ctrl_Y
                    || c == Ctrl_E
                    || c == Ctrl_L
                    || c == Ctrl_F
                    || c == Ctrl_RSB
                    || c == Ctrl_I
                    || c == Ctrl_D
                    || c == Ctrl_P
                    || c == Ctrl_N
                    || c == Ctrl_T
                    || c == Ctrl_V
                    || c == Ctrl_Q
                    || c == Ctrl_U
                    || c == Ctrl_O
                    || c == Ctrl_S
                    || c == Ctrl_K
                    || c == b's' as c_int
                    || c == Ctrl_Z
                    || c == Ctrl_R
            }
            CTRL_X_SCROLL => c == Ctrl_Y || c == Ctrl_E,
            CTRL_X_WHOLE_LINE => c == Ctrl_L || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_FILES => c == Ctrl_F || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_DICTIONARY => c == Ctrl_K || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_THESAURUS => c == Ctrl_T || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_TAGS => c == Ctrl_RSB || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_PATH_PATTERNS => c == Ctrl_P || c == Ctrl_N,
            CTRL_X_PATH_DEFINES => c == Ctrl_D || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_CMDLINE => {
                c == Ctrl_V || c == Ctrl_Q || c == Ctrl_P || c == Ctrl_N || c == Ctrl_X
            }
            CTRL_X_FUNCTION => c == Ctrl_U || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_OMNI => c == Ctrl_O || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_SPELL => c == Ctrl_S || c == Ctrl_P || c == Ctrl_N,
            CTRL_X_EVAL => c == Ctrl_P || c == Ctrl_N,
            CTRL_X_BUFNAMES => c == Ctrl_P || c == Ctrl_N,
            CTRL_X_REGISTER => c == Ctrl_R || c == Ctrl_P || c == Ctrl_N,
            _ => {
                internal_error(c"vim_is_ctrl_x_key()".as_ptr());
                false
            }
        }
    }
}

#[inline]
unsafe fn match_at_original_text(match_: *const compl_T) -> bool {
    unsafe { ((*match_).cp_flags & CP_ORIGINAL_TEXT) != 0 }
}

#[inline]
unsafe fn is_first_match(match_: *const compl_T) -> bool {
    unsafe { ptr::eq(match_, compl_first_match) }
}

/// Return the entry holding the original text, NULL if not found.
unsafe fn find_original_text_match() -> *mut compl_T {
    unsafe {
        if compl_first_match.is_null() {
            return ptr::null_mut();
        }
        if match_at_original_text(compl_first_match) {
            return compl_first_match;
        }
        if !(*compl_first_match).cp_prev.is_null()
            && match_at_original_text((*compl_first_match).cp_prev)
        {
            return (*compl_first_match).cp_prev;
        }
        ptr::null_mut()
    }
}

unsafe fn do_autocmd_completedone(c: c_int, mode: c_int, word: *mut NvimString) {
    unsafe {
        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let v_event = get_v_event(&raw mut save_v_event);

        if word.is_null() || (*word).data.is_null() {
            tv_dict_add_str_len(v_event, c"complete_word".as_ptr(), 13, c"".as_ptr(), 0);
        } else {
            tv_dict_add_str_len(
                v_event,
                c"complete_word".as_ptr(),
                13,
                (*word).data,
                (*word).size as c_int,
            );
        }

        let idx = (mode & !CTRL_X_WANT_IDENT) as usize;
        match CTRL_X_MODE_NAMES.get(idx).copied().flatten() {
            Some(mode_cstr) => {
                tv_dict_add_str_len(
                    v_event,
                    c"complete_type".as_ptr(),
                    13,
                    mode_cstr.as_ptr(),
                    mode_cstr.to_bytes().len() as c_int,
                );
            }
            None => {
                tv_dict_add_str_len(v_event, c"complete_type".as_ptr(), 13, c"".as_ptr(), 0);
            }
        }

        let reason = if c == Ctrl_Y || (!word.is_null() && !(*word).data.is_null()) {
            c"accept".as_ptr()
        } else if c == Ctrl_E {
            c"cancel".as_ptr()
        } else {
            c"discard".as_ptr()
        };
        tv_dict_add_str(v_event, c"reason".as_ptr(), 6, reason);
        tv_dict_set_keys_readonly(v_event);

        ins_apply_autocmds(EVENT_COMPLETEDONE);
        restore_v_event(v_event, &raw mut save_v_event);
    }
}

/// Check that character "c" is part of the item currently being completed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_accept_char(c: c_int) -> bool {
    unsafe {
        if compl_autocomplete && compl_from_nonkeyword {
            return false;
        }

        if (ctrl_x_mode & CTRL_X_WANT_IDENT) != 0 {
            return vim_isIDc(c);
        }

        match ctrl_x_mode {
            CTRL_X_FILES => vim_isfilec(c) && !vim_ispathsep(c),
            CTRL_X_CMDLINE | CTRL_X_CMDLINE_CTRL_X | CTRL_X_OMNI => {
                vim_isprintc(c) && !ascii_iswhite(c)
            }
            CTRL_X_WHOLE_LINE => vim_isprintc(c),
            _ => vim_iswordc(c),
        }
    }
}

/// Check if "c" is a commit character for the currently selected completion match.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_commit_char(c: c_int) -> bool {
    unsafe {
        if c < b' ' as c_int {
            return false;
        }
        !compl_shown_match.is_null()
            && !match_at_original_text(compl_shown_match)
            && !(*compl_shown_match).cp_commit_chars.is_null()
            && !vim_strchr((*compl_shown_match).cp_commit_chars, c).is_null()
    }
}

/// Get the completed text by inferring the case of the originally typed text.
unsafe fn ins_compl_infercase_gettext(
    str_ptr: *const c_char,
    char_len: c_int,
    compl_char_len: c_int,
    min_len: c_int,
    tofree: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        let mut has_lower = false;

        let wca = xmalloc((char_len as usize) * size_of::<c_int>()).cast::<c_int>();
        {
            let mut p = str_ptr;
            for i in 0..(char_len as usize) {
                *wca.add(i) = mb_ptr2char_adv(&raw mut p);
            }
        }

        // Rule 1: Were any chars converted to lower?
        {
            let mut p = compl_orig_text.data.cast_const();
            for i in 0..(min_len as usize) {
                let c = mb_ptr2char_adv(&raw mut p);
                if mb_islower(c) {
                    has_lower = true;
                    if mb_isupper(*wca.add(i)) {
                        for j in (compl_char_len as usize)..(char_len as usize) {
                            *wca.add(j) = mb_tolower(*wca.add(j));
                        }
                        break;
                    }
                }
            }
        }

        // Rule 2: No lower case, 2nd consecutive letter converted to upper case.
        if !has_lower {
            let mut was_letter = false;
            let mut p = compl_orig_text.data.cast_const();
            for i in 0..(min_len as usize) {
                let c = mb_ptr2char_adv(&raw mut p);
                if was_letter && mb_isupper(c) && mb_islower(*wca.add(i)) {
                    for j in (compl_char_len as usize)..(char_len as usize) {
                        *wca.add(j) = mb_toupper(*wca.add(j));
                    }
                    break;
                }
                was_letter = mb_islower(c) || mb_isupper(c);
            }
        }

        // Copy the original case of the part we typed.
        {
            let mut p = compl_orig_text.data.cast_const();
            for i in 0..(min_len as usize) {
                let c = mb_ptr2char_adv(&raw mut p);
                if mb_islower(c) {
                    *wca.add(i) = mb_tolower(*wca.add(i));
                } else if mb_isupper(c) {
                    *wca.add(i) = mb_toupper(*wca.add(i));
                }
            }
        }

        // Generate encoding specific output from wide character array.
        let mut gap = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        let mut p = iobuff_ptr;
        let mut i: c_int = 0;
        ga_init(&raw mut gap, 1, 500);
        while i < char_len {
            if !gap.ga_data.is_null() {
                ga_grow(&raw mut gap, 10);
                p = gap.ga_data.cast::<c_char>().add(gap.ga_len as usize);
                gap.ga_len += utf_char2bytes(*wca.add(i as usize), p);
                i += 1;
            } else if (p.offset_from(iobuff_ptr) as usize) + 6 >= IOSIZE {
                ga_grow(&raw mut gap, IOSIZE as c_int);
                *p = NUL;
                strcpy(gap.ga_data.cast::<c_char>(), iobuff_ptr);
                gap.ga_len = p.offset_from(iobuff_ptr) as c_int;
            } else {
                p = p.add(utf_char2bytes(*wca.add(i as usize), p) as usize);
                i += 1;
            }
        }
        xfree(wca.cast());

        if !gap.ga_data.is_null() {
            *tofree = gap.ga_data.cast::<c_char>();
            return gap.ga_data.cast::<c_char>();
        }

        *p = NUL;
        iobuff_ptr
    }
}

/// Like `ins_compl_add()`, but infers case if `'ic'` and `'inf'` are set.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_add_infercase(
    str_arg: *mut c_char,
    len: c_int,
    icase: bool,
    fname: *mut c_char,
    dir: Direction,
    cont_s_ipos: bool,
    score: c_int,
) -> c_int {
    unsafe {
        let mut str_ptr = str_arg;
        let mut flags: c_int = 0;
        let mut tofree: *mut c_char = ptr::null_mut();

        if p_ic != 0 && (*curbuf).b_p_inf != 0 && len > 0 {
            let mut char_len: c_int = 0;
            {
                let mut p = str_ptr.cast_const();
                while *p != NUL {
                    mb_ptr_adv_const(&raw mut p);
                    char_len += 1;
                }
            }

            let mut compl_char_len: c_int = 0;
            {
                let mut p = compl_orig_text.data.cast_const();
                while *p != NUL {
                    mb_ptr_adv_const(&raw mut p);
                    compl_char_len += 1;
                }
            }

            let min_len = char_len.min(compl_char_len);
            str_ptr = ins_compl_infercase_gettext(
                str_ptr,
                char_len,
                compl_char_len,
                min_len,
                &raw mut tofree,
            );
        }
        if cont_s_ipos {
            flags |= CP_CONT_S_IPOS;
        }
        if icase {
            flags |= CP_ICASE;
        }

        let res = ins_compl_add(
            str_ptr,
            len,
            fname,
            ptr::null(),
            false,
            ptr::null_mut(),
            dir,
            flags,
            false,
            ptr::null(),
            score,
            false,
            ptr::null_mut(),
        );
        xfree(tofree.cast());
        res
    }
}

#[inline]
unsafe fn free_cptext(cptext: *const *mut c_char) {
    unsafe {
        if !cptext.is_null() {
            for i in 0..CPT_COUNT {
                xfree((*cptext.add(i)).cast());
            }
        }
    }
}

#[inline]
unsafe fn cot_fuzzy() -> bool {
    unsafe { (get_cot_flags() & kOptCotFlagFuzzy) != 0 && !ctrl_x_mode_thesaurus() }
}

#[inline]
unsafe fn is_nearest_active() -> bool {
    unsafe { (compl_autocomplete || (get_cot_flags() & kOptCotFlagNearest) != 0) && !cot_fuzzy() }
}

/// True if a match is selected (even if it is not inserted).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_is_match_selected() -> bool {
    unsafe { !compl_shown_match.is_null() && !is_first_match(compl_shown_match) }
}

/// Returns true if autocomplete is active and the pre-insert effect targets the longest prefix.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_preinsert_longest() -> bool {
    unsafe {
        compl_autocomplete
            && (get_cot_flags() & (kOptCotFlagLongest | kOptCotFlagPreinsert | kOptCotFlagFuzzy))
                == kOptCotFlagLongest
    }
}

/// Add a match to the list of matches.
unsafe fn ins_compl_add(
    str_ptr: *mut c_char,
    mut len: c_int,
    fname: *mut c_char,
    cptext: *const *mut c_char,
    cptext_allocated: bool,
    user_data: *mut typval_T,
    cdir: Direction,
    flags_arg: c_int,
    adup: bool,
    user_hl: *const c_int,
    score: c_int,
    preselect: bool,
    commit_chars: *mut c_char,
) -> c_int {
    unsafe {
        let dir = if cdir == kDirectionNotSet {
            compl_direction
        } else {
            cdir
        };
        let mut flags = flags_arg;
        let mut new_str: *mut c_char = ptr::null_mut();
        let mut str_hash: hash_T = 0;

        if (flags & CP_FAST) != 0 {
            fast_breakcheck();
        } else {
            os_breakcheck();
        }
        if got_int {
            if cptext_allocated {
                free_cptext(cptext);
            }
            xfree(commit_chars.cast());
            return FAIL;
        }
        if len < 0 {
            len = strlen(str_ptr) as c_int;
        }

        // If the same match is already present, don't add it.
        if !compl_first_match.is_null() && !adup && compl_strings_ht.ht_used > 0 {
            let mut keybuf: [c_char; 128] = [0; 128];
            let key = if (len as usize) < keybuf.len() {
                ptr::copy(str_ptr, keybuf.as_mut_ptr(), len as usize);
                keybuf[len as usize] = NUL;
                keybuf.as_mut_ptr()
            } else {
                new_str = xstrnsave(str_ptr, len as usize);
                new_str
            };
            str_hash = hash_hash(key);
            let hi = hash_lookup(&raw mut compl_strings_ht, key, len as usize, str_hash);
            if !hashitem_empty(&*hi) {
                if is_nearest_active() && score > 0 {
                    let mut match_ = compl_first_match;
                    loop {
                        if !match_at_original_text(match_)
                            && strncmp((*match_).cp_str.data, str_ptr, len as usize) == 0
                            && ((*match_).cp_str.size as c_int <= len
                                || *(*match_).cp_str.data.add(len as usize) == NUL)
                        {
                            if score < (*match_).cp_score {
                                (*match_).cp_score = score;
                            }
                            break;
                        }
                        match_ = (*match_).cp_next;
                        if match_.is_null() || is_first_match(match_) {
                            break;
                        }
                    }
                }
                xfree(new_str.cast());
                if cptext_allocated {
                    free_cptext(cptext);
                }
                xfree(commit_chars.cast());
                return NOTDONE;
            }
        }

        ins_compl_del_pum();

        let match_ = xcalloc(1, size_of::<compl_T>()).cast::<compl_T>();
        (*match_).cp_number = if (flags & CP_ORIGINAL_TEXT) != 0 {
            0
        } else {
            -1
        };
        (*match_).cp_order = -1;
        if new_str.is_null() {
            new_str = xstrnsave(str_ptr, len as usize);
        }
        (*match_).cp_str = cbuf_as_string(new_str, len as usize);
        (*match_).cp_commit_chars = commit_chars;
        (*match_).cp_preselect = preselect;
        if preselect
            && compl_preselect_match.is_null()
            && (get_cot_flags() & kOptCotFlagPreselect) != 0
        {
            compl_preselect_match = match_;
        }

        if !fname.is_null()
            && !compl_curr_match.is_null()
            && !(*compl_curr_match).cp_fname.is_null()
            && strcmp(fname, (*compl_curr_match).cp_fname) == 0
        {
            (*match_).cp_fname = (*compl_curr_match).cp_fname;
        } else if !fname.is_null() {
            (*match_).cp_fname = xstrdup(fname);
            flags |= CP_FREE_FNAME;
        } else {
            (*match_).cp_fname = ptr::null_mut();
        }
        (*match_).cp_flags = flags;
        (*match_).cp_user_abbr_hl_id = if !user_hl.is_null() { *user_hl } else { 0 };
        (*match_).cp_user_kind_hl_id = if !user_hl.is_null() {
            *user_hl.add(1)
        } else {
            0
        };
        (*match_).cp_score = score;
        (*match_).cp_cpt_source_idx = cpt_sources_index;

        if !cptext.is_null() {
            for i in 0..CPT_COUNT {
                let s = *cptext.add(i);
                if s.is_null() {
                    continue;
                }
                if *s != NUL {
                    (*match_).cp_text[i] = if cptext_allocated { s } else { xstrdup(s) };
                } else if cptext_allocated {
                    xfree(s.cast());
                }
            }
        }

        if !user_data.is_null() {
            (*match_).cp_user_data = *user_data;
        }

        if compl_first_match.is_null() {
            (*match_).cp_next = ptr::null_mut();
            (*match_).cp_prev = ptr::null_mut();
        } else if cot_fuzzy() && score != FUZZY_SCORE_NONE && compl_get_longest {
            let mut current = (*compl_first_match).cp_next;
            let mut prev = compl_first_match;
            let mut inserted = false;
            while !current.is_null() && !ptr::eq(current, compl_first_match) {
                if (*current).cp_score < score {
                    (*match_).cp_next = current;
                    (*match_).cp_prev = (*current).cp_prev;
                    if !(*current).cp_prev.is_null() {
                        (*(*current).cp_prev).cp_next = match_;
                    }
                    (*current).cp_prev = match_;
                    inserted = true;
                    break;
                }
                prev = current;
                current = (*current).cp_next;
            }
            if !inserted {
                (*prev).cp_next = match_;
                (*match_).cp_prev = prev;
                (*match_).cp_next = compl_first_match;
                (*compl_first_match).cp_prev = match_;
            }
        } else if dir == FORWARD {
            (*match_).cp_next = (*compl_curr_match).cp_next;
            (*match_).cp_prev = compl_curr_match;
        } else {
            (*match_).cp_next = compl_curr_match;
            (*match_).cp_prev = (*compl_curr_match).cp_prev;
        }
        if !(*match_).cp_next.is_null() {
            (*(*match_).cp_next).cp_prev = match_;
        }
        if !(*match_).cp_prev.is_null() {
            (*(*match_).cp_prev).cp_next = match_;
        } else {
            compl_first_match = match_;
        }
        compl_curr_match = match_;

        if !match_at_original_text(match_) {
            compl_strings_add((*match_).cp_str.data, (*match_).cp_str.size, str_hash);
        }

        if compl_get_longest
            && (flags & CP_ORIGINAL_TEXT) == 0
            && !cot_fuzzy()
            && !ins_compl_preinsert_longest()
            && !ctrl_x_mode_thesaurus()
        {
            ins_compl_longest_match(match_);
        }

        OK
    }
}

/// Check that "str[len]" matches with "match->cp_str", considering "match->cp_flags".
unsafe fn ins_compl_equal(match_: *mut compl_T, str_ptr: *const c_char, len: usize) -> bool {
    unsafe {
        if ((*match_).cp_flags & CP_EQUAL) != 0 {
            return true;
        }
        if ((*match_).cp_flags & CP_ICASE) != 0 {
            return strnicmp_impl((*match_).cp_str.data, str_ptr, len) == 0;
        }
        strncmp((*match_).cp_str.data, str_ptr, len) == 0
    }
}

/// Like `ins_compl_equal()`, but ignore case in the `'longest'`-inserted part of the leader.
unsafe fn ins_compl_equal_sc(match_: *mut compl_T, str_ptr: *const c_char, len: usize) -> bool {
    unsafe {
        let typed = compl_length;
        let longest_end = if compl_get_longest && compl_longest_end_col > compl_col {
            compl_longest_end_col - compl_col
        } else {
            typed
        };

        if ((*match_).cp_flags & (CP_EQUAL | CP_ICASE)) != 0 || longest_end <= typed {
            return ins_compl_equal(match_, str_ptr, len);
        }

        if (*match_).cp_str.size < len {
            return false;
        }

        for i in 0..len {
            let idx = i as c_int;
            let mismatch = if idx >= typed && idx < longest_end {
                let c1 = *(*match_).cp_str.data.add(i) as u8 as c_int;
                let c2 = *str_ptr.add(i) as u8 as c_int;
                tolower(c1) != tolower(c2)
            } else {
                *(*match_).cp_str.data.add(i) != *str_ptr.add(i)
            };
            if mismatch {
                return false;
            }
        }
        true
    }
}

unsafe fn ins_compl_insert_bytes(p: *const c_char, mut len: c_int) {
    unsafe {
        if len == -1 {
            len = strlen(p) as c_int;
        }
        debug_assert!(len >= 0);
        ins_bytes_len(p.cast_mut(), len as usize);
        compl_ins_end_col = (*curwin).w_cursor.col;
    }
}

/// Get current completion leader.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_leader() -> *mut c_char {
    unsafe {
        if !compl_leader.data.is_null() {
            compl_leader.data
        } else {
            compl_orig_text.data
        }
    }
}

#[inline]
unsafe fn ins_compl_leader_len() -> usize {
    unsafe {
        if !compl_leader.data.is_null() {
            compl_leader.size
        } else {
            compl_orig_text.size
        }
    }
}

/// Checks if the column is within the currently inserted completion text column range.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_col_range_attr(lnum: linenr_T, col: c_int) -> c_int {
    unsafe {
        let has_preinsert = ins_compl_has_preinsert() || ins_compl_preinsert_longest();

        if cot_fuzzy() || (!compl_hi_on_autocompl_longest && ins_compl_preinsert_longest()) {
            return -1;
        }
        let attr = syn_name2attr(if has_preinsert {
            c"PreInsert".as_ptr()
        } else {
            c"ComplMatchIns".as_ptr()
        });
        if attr == 0 {
            return -1;
        }

        let start_col = compl_col + ins_compl_leader_len() as c_int;
        if !ins_compl_has_multiple() {
            return if col >= start_col && col < compl_ins_end_col {
                attr
            } else {
                -1
            };
        }

        if (lnum == compl_lnum && col >= start_col && col < MAXCOL)
            || (lnum > compl_lnum && lnum < (*curwin).w_cursor.lnum)
            || (lnum == (*curwin).w_cursor.lnum && col <= compl_ins_end_col)
        {
            return attr;
        }

        -1
    }
}

#[inline]
unsafe fn ins_compl_has_multiple() -> bool {
    unsafe { !vim_strchr((*compl_shown_match).cp_str.data, b'\n' as c_int).is_null() }
}

/// Returns true if the given line number falls within the range of a multi-line completion.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_lnum_in_range(lnum: linenr_T) -> bool {
    unsafe {
        if !ins_compl_has_multiple() {
            return false;
        }
        lnum >= compl_lnum && lnum <= (*curwin).w_cursor.lnum
    }
}

/// Reduce the longest common string for match "match".
unsafe fn ins_compl_longest_match(match_: *mut compl_T) {
    unsafe {
        if compl_leader.data.is_null() {
            compl_leader = copy_string((*match_).cp_str, ptr::null_mut());

            let had_match = (*curwin).w_cursor.col > compl_col;
            ins_compl_longest_insert(compl_leader.data);

            if !had_match {
                ins_compl_delete(false);
            }
            compl_used_match = false;
            return;
        }

        let mut p = compl_leader.data;
        let mut s = (*match_).cp_str.data;
        while *p != NUL {
            let c1 = utf_ptr2char(p);
            let c2 = utf_ptr2char(s);

            let diff = if ((*match_).cp_flags & CP_ICASE) != 0 {
                mb_tolower(c1) != mb_tolower(c2)
            } else {
                c1 != c2
            };
            if diff {
                break;
            }
            mb_ptr_adv(&raw mut p);
            mb_ptr_adv(&raw mut s);
        }

        if *p != NUL {
            *p = NUL;
            compl_leader.size = p.offset_from(compl_leader.data) as usize;

            let had_match = (*curwin).w_cursor.col > compl_col;
            ins_compl_longest_insert(compl_leader.data);

            if !had_match {
                ins_compl_delete(false);
            }
        }

        compl_used_match = false;
    }
}

/// Add an array of matches to the list of matches. Frees matches[].
unsafe fn ins_compl_add_matches(num_matches: c_int, matches: *mut *mut c_char, icase: c_int) {
    unsafe {
        let mut add_r = OK;
        let mut dir = compl_direction;

        let mut i: c_int = 0;
        while i < num_matches && add_r != FAIL {
            add_r = ins_compl_add(
                *matches.add(i as usize),
                -1,
                ptr::null_mut(),
                ptr::null(),
                false,
                ptr::null_mut(),
                dir,
                CP_FAST | if icase != 0 { CP_ICASE } else { 0 },
                false,
                ptr::null(),
                FUZZY_SCORE_NONE,
                false,
                ptr::null_mut(),
            );
            if add_r == OK {
                dir = FORWARD;
            }
            i += 1;
        }
        FreeWild(num_matches, matches);
    }
}

/// Make the completion list cyclic.
/// Return the number of matches (excluding the original).
unsafe fn ins_compl_make_cyclic() -> c_int {
    unsafe {
        if compl_first_match.is_null() {
            return 0;
        }

        let mut match_ = compl_first_match;
        let mut count: c_int = 0;
        while !(*match_).cp_next.is_null() && !is_first_match((*match_).cp_next) {
            match_ = (*match_).cp_next;
            count += 1;
        }
        (*match_).cp_next = compl_first_match;
        (*compl_first_match).cp_prev = match_;

        count
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_has_shown_match() -> bool {
    unsafe {
        compl_shown_match.is_null() || !ptr::eq(compl_shown_match, (*compl_shown_match).cp_next)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_long_shown_match() -> bool {
    unsafe {
        !compl_shown_match.is_null()
            && !(*compl_shown_match).cp_str.data.is_null()
            && ((*compl_shown_match).cp_str.size as colnr_T) > (*curwin).w_cursor.col - compl_col
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_cot_flags() -> c_uint {
    unsafe {
        if (*curbuf).b_cot_flags != 0 {
            (*curbuf).b_cot_flags
        } else {
            cot_flags
        }
    }
}

unsafe fn ins_compl_del_pum() {
    unsafe {
        if compl_match_array.is_null() {
            return;
        }

        pum_undisplay(false);
        xfree(compl_match_array.cast());
        compl_match_array = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn pum_wanted() -> bool {
    unsafe { (get_cot_flags() & (kOptCotFlagMenu | kOptCotFlagMenuone)) != 0 || compl_autocomplete }
}

unsafe fn pum_enough_matches() -> bool {
    unsafe {
        let mut comp = compl_first_match;
        let mut i: c_int = 0;
        loop {
            if comp.is_null() {
                break;
            }
            if !match_at_original_text(comp) {
                i += 1;
                if i == 2 {
                    break;
                }
            }
            comp = (*comp).cp_next;
            if is_first_match(comp) {
                break;
            }
        }

        if (get_cot_flags() & kOptCotFlagMenuone) != 0 || compl_autocomplete {
            return i >= 1;
        }
        i >= 2
    }
}

unsafe fn ins_compl_dict_alloc(match_: *mut compl_T) -> *mut dict_T {
    unsafe {
        let dict = tv_dict_alloc_lock(VAR_FIXED);
        fill_complete_info_dict(dict, match_, false);
        dict
    }
}

unsafe fn trigger_complete_changed_event(cur: c_int) {
    static mut recursive: bool = false;
    unsafe {
        if recursive {
            return;
        }

        let item = if cur < 0 {
            tv_dict_alloc()
        } else {
            ins_compl_dict_alloc(compl_curr_match)
        };
        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let v_event = get_v_event(&raw mut save_v_event);
        tv_dict_add_dict(v_event, c"completed_item".as_ptr(), 14, item);
        pum_set_event_info(v_event);
        tv_dict_set_keys_readonly(v_event);

        recursive = true;
        textlock += 1;
        apply_autocmds(
            EVENT_COMPLETECHANGED,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        textlock -= 1;
        recursive = false;

        restore_v_event(v_event, &raw mut save_v_event);
    }
}

unsafe extern "C" fn cp_get_next(node: *mut c_void) -> *mut c_void {
    unsafe { (*node.cast::<compl_T>()).cp_next.cast() }
}

unsafe extern "C" fn cp_set_next(node: *mut c_void, next: *mut c_void) {
    unsafe {
        (*node.cast::<compl_T>()).cp_next = next.cast();
    }
}

unsafe extern "C" fn cp_get_prev(node: *mut c_void) -> *mut c_void {
    unsafe { (*node.cast::<compl_T>()).cp_prev.cast() }
}

unsafe extern "C" fn cp_set_prev(node: *mut c_void, prev: *mut c_void) {
    unsafe {
        (*node.cast::<compl_T>()).cp_prev = prev.cast();
    }
}

unsafe extern "C" fn cp_compare_fuzzy(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let score_a = (*a.cast::<compl_T>()).cp_score;
        let score_b = (*b.cast::<compl_T>()).cp_score;

        if score_a != score_b {
            return if score_b > score_a { 1 } else { -1 };
        }

        (*a.cast::<compl_T>()).cp_order - (*b.cast::<compl_T>()).cp_order
    }
}

unsafe extern "C" fn cp_compare_nearest(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let score_a = (*a.cast::<compl_T>()).cp_score;
        let score_b = (*b.cast::<compl_T>()).cp_score;
        if score_a == FUZZY_SCORE_NONE || score_b == FUZZY_SCORE_NONE {
            return 0;
        }
        if score_a > score_b {
            1
        } else if score_a < score_b {
            -1
        } else {
            0
        }
    }
}

unsafe fn prepend_startcol_text(dest: *mut NvimString, src: *const NvimString, startcol: c_int) {
    unsafe {
        let prepend_len = compl_col - startcol;
        let new_length = prepend_len + (*src).size as c_int;

        (*dest).size = new_length as usize;
        (*dest).data = xmalloc(new_length as usize + 1).cast::<c_char>();

        let line = ml_get((*curwin).w_cursor.lnum);

        ptr::copy(
            line.add(startcol as usize),
            (*dest).data,
            prepend_len as usize,
        );
        ptr::copy(
            (*src).data,
            (*dest).data.add(prepend_len as usize),
            (*src).size,
        );
        *(*dest).data.add(new_length as usize) = NUL;
    }
}

unsafe fn get_leader_for_startcol(match_: *mut compl_T, cached: bool) -> *mut NvimString {
    static mut adjusted_leader: NvimString = empty_nvim_string();
    unsafe {
        if match_.is_null() {
            api_clear_string(&raw mut adjusted_leader);
            return ptr::null_mut();
        }

        if cpt_sources_array.is_null() {
            return &raw mut compl_leader;
        }

        let cpt_idx = (*match_).cp_cpt_source_idx;
        if cpt_idx < 0 {
            return &raw mut compl_leader;
        }
        let startcol = (*cpt_sources_array.add(cpt_idx as usize)).cs_startcol;

        if compl_leader.data.is_null() {
            if startcol < 0 || startcol >= compl_col {
                return &raw mut compl_orig_text;
            }
            return &raw mut compl_leader;
        }

        if compl_col <= 0 {
            return &raw mut compl_leader;
        }

        if startcol >= 0 && startcol < compl_col {
            let prepend_len = compl_col - startcol;
            let new_length = prepend_len + compl_leader.size as c_int;
            if cached
                && (new_length as usize) == adjusted_leader.size
                && !adjusted_leader.data.is_null()
            {
                return &raw mut adjusted_leader;
            }

            api_clear_string(&raw mut adjusted_leader);
            prepend_startcol_text(&raw mut adjusted_leader, &raw const compl_leader, startcol);
            return &raw mut adjusted_leader;
        }

        &raw mut compl_leader
    }
}

unsafe fn set_fuzzy_score() {
    unsafe {
        if compl_first_match.is_null() {
            return;
        }

        let use_leader = !compl_leader.data.is_null() && compl_leader.size > 0;
        let mut pattern: *mut c_char;
        if !use_leader {
            if compl_orig_text.data.is_null() || compl_orig_text.size == 0 {
                return;
            }
            pattern = compl_orig_text.data;
        } else {
            let _ = get_leader_for_startcol(ptr::null_mut(), true);
            pattern = ptr::null_mut();
        }

        let mut next: c_int = 0;
        let mut comp = compl_first_match;
        loop {
            if (*comp).cp_order < 0 {
                (*comp).cp_order = next;
            }
            next = next.max((*comp).cp_order + 1);

            if use_leader {
                pattern = (*get_leader_for_startcol(comp, true)).data;
            }

            (*comp).cp_score = fuzzy_match_str((*comp).cp_str.data, pattern);
            comp = (*comp).cp_next;
            if comp.is_null() || is_first_match(comp) {
                break;
            }
        }
    }
}

unsafe fn sort_compl_match_list(compare: MergeSortCompareFunc) {
    unsafe {
        if compl_first_match.is_null() || is_first_match((*compl_first_match).cp_next) {
            return;
        }

        let orig_text = find_original_text_match();
        if orig_text.is_null() {
            return;
        }

        ins_compl_make_linear();
        if ptr::eq(orig_text, compl_first_match) {
            (*(*compl_first_match).cp_next).cp_prev = ptr::null_mut();
            (*compl_first_match).cp_next = mergesort_list(
                (*compl_first_match).cp_next.cast(),
                Some(cp_get_next),
                Some(cp_set_next),
                Some(cp_get_prev),
                Some(cp_set_prev),
                compare,
            )
            .cast();
            (*(*compl_first_match).cp_next).cp_prev = compl_first_match;
        } else {
            (*(*orig_text).cp_prev).cp_next = ptr::null_mut();
            compl_first_match = mergesort_list(
                compl_first_match.cast(),
                Some(cp_get_next),
                Some(cp_set_next),
                Some(cp_get_prev),
                Some(cp_set_prev),
                compare,
            )
            .cast();
            let mut tail = compl_first_match;
            while !(*tail).cp_next.is_null() {
                tail = (*tail).cp_next;
            }
            (*tail).cp_next = orig_text;
            (*orig_text).cp_prev = tail;
        }
        let _ = ins_compl_make_cyclic();
    }
}

unsafe fn get_user_highlight_attr(hl_id: c_int) -> c_int {
    unsafe {
        if hl_id <= 0 {
            return -1;
        }
        let attr = syn_id2attr(hl_id);
        if attr > 0 { attr } else { -1 }
    }
}

unsafe fn ins_compl_build_pum() -> c_int {
    unsafe {
        compl_match_arraysize = 0;

        if ins_compl_need_restart() {
            api_clear_string(&raw mut compl_leader);
        }

        let compl_no_select = (get_cot_flags() & kOptCotFlagNoselect) != 0
            || (compl_autocomplete && !ins_compl_has_preinsert());

        let mut match_head: *mut compl_T = ptr::null_mut();
        let mut match_tail: *mut compl_T = ptr::null_mut();
        let mut match_count: *mut c_int = ptr::null_mut();
        let is_forward = compl_shows_dir_forward();
        let is_cpt_completion = !cpt_sources_array.is_null();

        let mut shown_match_ok = match_at_original_text(compl_shown_match);

        if strequal(compl_leader.data, compl_orig_text.data) && !shown_match_ok {
            compl_shown_match = if compl_no_select {
                compl_first_match
            } else {
                (*compl_first_match).cp_next
            };
        }

        let mut did_find_shown_match = false;
        let mut preselect_ok = false;
        let mut shown_compl: *mut compl_T = ptr::null_mut();
        let mut i: c_int = 0;
        let mut cur: c_int = -1;

        if is_cpt_completion {
            match_count = xcalloc(cpt_sources_count as usize, size_of::<c_int>()).cast::<c_int>();
        }

        let _ = get_leader_for_startcol(ptr::null_mut(), true);

        let mut comp = compl_first_match;
        loop {
            (*comp).cp_in_match_array = false;

            let leader = get_leader_for_startcol(comp, true);

            if ctrl_x_mode_normal()
                && p_inf == 0
                && !compl_orig_text.data.is_null()
                && ignorecase(compl_orig_text.data) == 0
                && !cot_fuzzy()
            {
                (*comp).cp_flags &= !CP_ICASE;
            }

            if !match_at_original_text(comp)
                && ((*leader).data.is_null()
                    || ins_compl_equal_sc(comp, (*leader).data, (*leader).size)
                    || (cot_fuzzy() && (*comp).cp_score != FUZZY_SCORE_NONE))
            {
                let mut match_limit_exceeded = false;
                let cur_source = (*comp).cp_cpt_source_idx;
                if is_forward && cur_source != -1 && is_cpt_completion {
                    *match_count.add(cur_source as usize) += 1;
                    let max_matches = (*cpt_sources_array.add(cur_source as usize)).cs_max_matches;
                    if max_matches > 0 && *match_count.add(cur_source as usize) > max_matches {
                        match_limit_exceeded = true;
                    }
                }

                if !match_limit_exceeded {
                    compl_match_arraysize += 1;
                    (*comp).cp_in_match_array = true;
                    if match_head.is_null() {
                        match_head = comp;
                    } else {
                        (*match_tail).cp_match_next = comp;
                    }
                    match_tail = comp;

                    if !shown_match_ok && !cot_fuzzy() {
                        if ptr::eq(comp, compl_shown_match) || did_find_shown_match {
                            compl_shown_match = comp;
                            did_find_shown_match = true;
                            shown_match_ok = true;
                        } else {
                            shown_compl = comp;
                        }
                        cur = i;
                    } else if cot_fuzzy() {
                        if i == 0 {
                            shown_compl = comp;
                        }

                        if !shown_match_ok && ptr::eq(comp, compl_shown_match) {
                            cur = i;
                            shown_match_ok = true;
                        }
                    }
                    if !compl_preselect_match.is_null() && (*comp).cp_preselect && !preselect_ok {
                        cur = i;
                        compl_shown_match = comp;
                        shown_match_ok = true;
                        preselect_ok = true;
                    }
                    i += 1;
                }
            }

            if ptr::eq(comp, compl_shown_match) && !cot_fuzzy() {
                did_find_shown_match = true;
                if match_at_original_text(comp) {
                    shown_match_ok = true;
                }
                if !shown_match_ok && !shown_compl.is_null() {
                    compl_shown_match = shown_compl;
                    shown_match_ok = true;
                }
            }
            comp = (*comp).cp_next;
            if comp.is_null() || is_first_match(comp) {
                break;
            }
        }

        xfree(match_count.cast());

        if compl_match_arraysize == 0 {
            return -1;
        }

        if cot_fuzzy() && !compl_no_select && !shown_match_ok {
            compl_shown_match = shown_compl;
            shown_match_ok = true;
            cur = 0;
        }

        debug_assert!(compl_match_arraysize >= 0);
        compl_match_array =
            xcalloc(compl_match_arraysize as usize, size_of::<pumitem_T>()).cast::<pumitem_T>();

        let mut idx: usize = 0;
        comp = match_head;
        while !comp.is_null() {
            let item = &mut *compl_match_array.add(idx);
            item.pum_text = if !(*comp).cp_text[CPT_ABBR].is_null() {
                (*comp).cp_text[CPT_ABBR]
            } else {
                (*comp).cp_str.data
            };
            item.pum_kind = (*comp).cp_text[CPT_KIND];
            item.pum_info = (*comp).cp_text[CPT_INFO];
            item.pum_cpt_source_idx = (*comp).cp_cpt_source_idx;
            item.pum_user_abbr_hlattr = get_user_highlight_attr((*comp).cp_user_abbr_hl_id);
            item.pum_user_kind_hlattr = get_user_highlight_attr((*comp).cp_user_kind_hl_id);
            item.pum_extra = if !(*comp).cp_text[CPT_MENU].is_null() {
                (*comp).cp_text[CPT_MENU]
            } else {
                (*comp).cp_fname
            };
            idx += 1;
            let match_next = (*comp).cp_match_next;
            (*comp).cp_match_next = ptr::null_mut();
            comp = match_next;
        }

        if !shown_match_ok {
            cur = -1;
        }

        cur
    }
}

/// Show the popup menu for the list of matches.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_show_pum() {
    unsafe {
        if !pum_wanted() || !pum_enough_matches() {
            return;
        }

        update_screen();

        let mut cur: c_int = -1;
        let mut array_changed = false;

        if compl_match_array.is_null() {
            array_changed = true;
            cur = ins_compl_build_pum();
        } else {
            for i in 0..(compl_match_arraysize as usize) {
                let pum_text = (*compl_match_array.add(i)).pum_text;
                if ptr::eq(pum_text, (*compl_shown_match).cp_str.data)
                    || ptr::eq(pum_text, (*compl_shown_match).cp_text[CPT_ABBR])
                {
                    cur = i as c_int;
                    break;
                }
            }
        }

        if compl_match_array.is_null() {
            if compl_started && has_event(EVENT_COMPLETECHANGED) {
                trigger_complete_changed_event(cur);
            }
            return;
        }

        dollar_vcol = -1;

        let col = (*curwin).w_cursor.col;
        (*curwin).w_cursor.col = compl_col;
        compl_selected_item = cur;
        pum_display(
            compl_match_array,
            compl_match_arraysize,
            cur,
            array_changed,
            0,
        );
        (*curwin).w_cursor.col = col;

        if (*curwin).w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(curwin) {
            redrawWinline(curwin, (*curwin).w_cursor.lnum);
        }

        if compl_started && !ptr::eq(compl_curr_match, compl_shown_match) {
            compl_curr_match = compl_shown_match;
        }

        if has_event(EVENT_COMPLETECHANGED) {
            trigger_complete_changed_event(cur);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compl_match_curr_select(selected: c_int) -> bool {
    unsafe {
        if selected < 0 {
            return false;
        }
        let mut match_ = compl_first_match;
        let mut selected_idx: c_int = -1;
        let mut list_idx: c_int = 0;
        loop {
            if !match_at_original_text(match_) {
                if !compl_curr_match.is_null()
                    && (*compl_curr_match).cp_number == (*match_).cp_number
                {
                    selected_idx = list_idx;
                    break;
                }
                list_idx += 1;
            }
            match_ = (*match_).cp_next;
            if match_.is_null() || is_first_match(match_) {
                break;
            }
        }

        selected == selected_idx
    }
}

unsafe fn ins_compl_dictionaries(
    dict_start: *mut c_char,
    pat: *mut c_char,
    flags: c_int,
    thesaurus: bool,
) {
    unsafe {
        let mut dict = dict_start;
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut files: *mut *mut c_char = ptr::null_mut();
        let mut count: c_int = 0;
        let mut dir = compl_direction;

        if *dict == NUL {
            if !thesaurus && (*curwin).w_onebuf_opt.wo_spell != 0 {
                dict = c"spell".as_ptr().cast_mut();
            } else {
                return;
            }
        }

        let mut buf = xmalloc(LSIZE).cast::<c_char>();
        regmatch.regprog = ptr::null_mut();

        let save_p_scs = p_scs;
        if (*curbuf).b_p_inf != 0 {
            p_scs = 0;
        }

        if ctrl_x_mode_line_or_eval() {
            let pat_esc = vim_strsave_escaped(pat, c"\\".as_ptr());
            let len = strlen(pat_esc) + 10;
            let ptr = xmalloc(len).cast::<c_char>();
            vim_snprintf!(ptr, len, c"^\\s*\\zs\\V%s".as_ptr(), pat_esc);
            regmatch.regprog = vim_regcomp(ptr, RE_MAGIC);
            xfree(pat_esc.cast());
            xfree(ptr.cast());
        } else {
            regmatch.regprog = vim_regcomp(pat, if p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                p_scs = save_p_scs;
                vim_regfree(regmatch.regprog);
                xfree(buf.cast());
                return;
            }
        }

        regmatch.rm_ic = ignorecase(pat) != 0;
        while *dict != NUL && !got_int && !compl_interrupted {
            if flags == DICT_EXACT {
                count = 1;
                files = &raw mut dict;
            } else {
                copy_option_part(&raw mut dict, buf, LSIZE, c",".as_ptr());
                if !thesaurus && strcmp(buf, c"spell".as_ptr()) == 0 {
                    count = -1;
                } else if !vim_strchr(buf, b'`' as c_int).is_null()
                    || expand_wildcards(
                        1,
                        &raw mut buf,
                        &raw mut count,
                        &raw mut files,
                        EW_FILE | EW_SILENT,
                    ) != OK
                {
                    count = 0;
                }
            }

            if count == -1 {
                let ptr = if *pat == b'\\' as c_char && *pat.add(1) == b'<' as c_char {
                    pat.add(2)
                } else {
                    pat
                };
                spell_dump_compl(ptr, c_int::from(regmatch.rm_ic), &raw mut dir, 0);
            } else if count > 0 {
                ins_compl_files(
                    count,
                    files,
                    thesaurus,
                    flags,
                    &raw mut regmatch,
                    buf,
                    &raw mut dir,
                );
                if flags != DICT_EXACT {
                    FreeWild(count, files);
                }
            }
            if flags != 0 {
                break;
            }
        }

        p_scs = save_p_scs;
        vim_regfree(regmatch.regprog);
        xfree(buf.cast());
    }
}

unsafe fn thesaurus_add_words_in_line(
    fname: *mut c_char,
    buf_arg: *mut *mut c_char,
    dir: c_int,
    skip_word: *const c_char,
) -> c_int {
    unsafe {
        let mut status = OK;
        let mut ptr = *buf_arg;

        while !got_int {
            ptr = find_word_start(ptr);
            if *ptr == NUL || *ptr == NL as c_char {
                break;
            }
            let wstart = ptr;

            while *ptr != NUL {
                let l = utfc_ptr2len(ptr);
                if l < 2 && !vim_iswordc(c_int::from(*ptr as u8)) {
                    break;
                }
                ptr = ptr.add(l as usize);
            }

            if !ptr::eq(wstart.cast_const(), skip_word) {
                status = ins_compl_add_infercase(
                    wstart,
                    ptr.offset_from(wstart) as c_int,
                    p_ic != 0,
                    fname,
                    dir,
                    false,
                    FUZZY_SCORE_NONE,
                );
                if status == FAIL {
                    break;
                }
            }
        }

        *buf_arg = ptr;
        status
    }
}

unsafe fn ins_compl_files(
    count: c_int,
    files: *mut *mut c_char,
    thesaurus: bool,
    flags: c_int,
    regmatch: *mut regmatch_T,
    buf: *mut c_char,
    dir: *mut Direction,
) {
    unsafe {
        let leader = if cot_fuzzy() {
            ins_compl_leader()
        } else {
            ptr::null_mut()
        };
        let leader_len = if cot_fuzzy() {
            ins_compl_leader_len() as c_int
        } else {
            0
        };

        let mut i: c_int = 0;
        while i < count && !got_int && !ins_compl_interrupted() {
            let fp = os_fopen(*files.add(i as usize), c"r".as_ptr());
            if flags != DICT_EXACT && !shortmess(kShmCompletionscan) && !compl_autocomplete {
                let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                vim_snprintf!(
                    iobuff_ptr,
                    IOSIZE,
                    gettext(c"Scanning dictionary: %s".as_ptr()),
                    *files.add(i as usize),
                );
                msg_progress(
                    iobuff_ptr,
                    c"nvim.completion".as_ptr().cast_mut(),
                    c"running".as_ptr().cast_mut(),
                    -1,
                    HLF_R,
                    false,
                    true,
                    false,
                );
            }

            if fp.is_null() {
                i += 1;
                continue;
            }

            while !got_int && !ins_compl_interrupted() && !vim_fgets(buf, LSIZE as c_int, fp) {
                let mut ptr = buf;
                if cot_fuzzy() && leader_len > 0 {
                    let line_end = find_line_end(ptr);
                    while ptr < line_end {
                        let mut score: c_int = 0;
                        let mut len: c_int = 0;
                        if fuzzy_match_str_in_line(
                            &raw mut ptr,
                            leader,
                            &raw mut len,
                            ptr::null_mut(),
                            &raw mut score,
                        ) {
                            let end_ptr = if ctrl_x_mode_line_or_eval() {
                                find_line_end(ptr)
                            } else {
                                find_word_end(ptr)
                            };
                            let add_r = ins_compl_add_infercase(
                                ptr,
                                end_ptr.offset_from(ptr) as c_int,
                                p_ic != 0,
                                *files.add(i as usize),
                                *dir,
                                false,
                                score,
                            );
                            if add_r == FAIL {
                                break;
                            }
                            ptr = end_ptr;
                            if compl_get_longest
                                && ctrl_x_mode_normal()
                                && !(*compl_first_match).cp_next.is_null()
                                && score == (*(*compl_first_match).cp_next).cp_score
                            {
                                compl_num_bests += 1;
                            }
                        }
                    }
                } else if !regmatch.is_null() {
                    while vim_regexec(regmatch, buf, ptr.offset_from(buf) as colnr_T) {
                        ptr = (*regmatch).startp[0];
                        ptr = if ctrl_x_mode_line_or_eval() {
                            find_line_end(ptr)
                        } else {
                            find_word_end(ptr)
                        };
                        let mut add_r = ins_compl_add_infercase(
                            (*regmatch).startp[0],
                            ptr.offset_from((*regmatch).startp[0]) as c_int,
                            p_ic != 0,
                            *files.add(i as usize),
                            *dir,
                            false,
                            FUZZY_SCORE_NONE,
                        );
                        if thesaurus {
                            ptr = buf;
                            add_r = thesaurus_add_words_in_line(
                                *files.add(i as usize),
                                &raw mut ptr,
                                *dir,
                                (*regmatch).startp[0],
                            );
                        }
                        if add_r == OK {
                            *dir = FORWARD;
                        } else if add_r == FAIL {
                            break;
                        }
                        if *ptr == b'\n' as c_char || got_int {
                            break;
                        }
                    }
                }
                line_breakcheck();
                ins_compl_check_keys(50, false);
            }
            fclose(fp);
            i += 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_word_start(mut ptr: *mut c_char) -> *mut c_char {
    unsafe {
        while *ptr != NUL && *ptr != b'\n' as c_char && mb_get_class(ptr) <= 1 {
            ptr = ptr.add(utfc_ptr2len(ptr) as usize);
        }
        ptr
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_word_end(mut ptr: *mut c_char) -> *mut c_char {
    unsafe {
        let start_class = mb_get_class(ptr);
        if start_class > 1 {
            while *ptr != NUL {
                ptr = ptr.add(utfc_ptr2len(ptr) as usize);
                if mb_get_class(ptr) != start_class {
                    break;
                }
            }
        }
        ptr
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_line_end(ptr: *mut c_char) -> *mut c_char {
    unsafe {
        let mut s = ptr.add(strlen(ptr));
        while s > ptr && (*s.sub(1) == CAR as c_char || *s.sub(1) == NL as c_char) {
            s = s.sub(1);
        }
        s
    }
}

unsafe fn ins_compl_item_free(match_: *mut compl_T) {
    unsafe {
        if compl_strings_ht.ht_used > 0
            && !(*match_).cp_str.data.is_null()
            && !match_at_original_text(match_)
        {
            let hi = hash_find(&raw const compl_strings_ht, (*match_).cp_str.data);
            if !hashitem_empty(&*hi) {
                let entry = hi2cse(hi);
                (*entry).cse_count -= 1;
                if (*entry).cse_count <= 0 {
                    hash_remove(&raw mut compl_strings_ht, hi);
                    xfree(entry.cast());
                }
            }
        }
        api_clear_string(&raw mut (*match_).cp_str);
        if ((*match_).cp_flags & CP_FREE_FNAME) != 0 {
            xfree((*match_).cp_fname.cast());
        }
        free_cptext((*match_).cp_text.as_ptr());
        tv_clear(&raw mut (*match_).cp_user_data);
        xfree((*match_).cp_commit_chars.cast());
        xfree(match_.cast());
    }
}

unsafe fn ins_compl_free() {
    unsafe {
        api_clear_string(&raw mut compl_pattern);
        api_clear_string(&raw mut compl_leader);

        if compl_first_match.is_null() {
            return;
        }

        ins_compl_del_pum();
        pum_clear();

        hash_clear_all(&raw mut compl_strings_ht, CSE_OFF as c_uint);
        hash_init(&raw mut compl_strings_ht);

        compl_curr_match = compl_first_match;
        loop {
            let match_ = compl_curr_match;
            compl_curr_match = (*compl_curr_match).cp_next;
            ins_compl_item_free(match_);
            if compl_curr_match.is_null() || is_first_match(compl_curr_match) {
                break;
            }
        }
        compl_first_match = ptr::null_mut();
        compl_curr_match = ptr::null_mut();
        compl_shown_match = ptr::null_mut();
        compl_preselect_match = ptr::null_mut();
        compl_old_match = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_clear() {
    unsafe {
        compl_cont_status = 0;
        compl_started = false;
        compl_matches = 0;
        compl_selected_item = -1;
        compl_ins_end_col = 0;
        compl_longest_end_col = 0;
        compl_curr_win = ptr::null_mut();
        compl_curr_buf = ptr::null_mut();
        api_clear_string(&raw mut compl_pattern);
        api_clear_string(&raw mut compl_leader);
        edit_submode_extra = ptr::null_mut();
        kv_destroy_extmarks(&raw mut compl_orig_extmarks);
        api_clear_string(&raw mut compl_orig_text);
        compl_enter_selects = false;
        cpt_sources_clear();
        compl_autocomplete = false;
        compl_from_nonkeyword = false;
        compl_autostarted = false;
        compl_num_bests = 0;
        set_vim_var_dict(VV_COMPLETED_ITEM, tv_dict_alloc_lock(VAR_FIXED));
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_active() -> bool {
    unsafe { compl_started }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_win_active(wp: *mut win_T) -> bool {
    unsafe {
        ins_compl_active() && ptr::eq(wp, compl_curr_win) && ptr::eq((*wp).w_buffer, compl_curr_buf)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_used_match() -> bool {
    unsafe { compl_used_match }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_init_get_longest() {
    unsafe {
        compl_get_longest = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_interrupted() -> bool {
    unsafe { compl_interrupted || compl_time_slice_expired }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_enter_selects() -> bool {
    unsafe { compl_enter_selects }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_col() -> colnr_T {
    unsafe { compl_col }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_len() -> c_int {
    unsafe { compl_length }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_has_preinsert() -> bool {
    unsafe {
        let cur_cot_flags = get_cot_flags();
        if compl_autocomplete && p_ic != 0 && p_inf == 0 {
            return false;
        }
        if !compl_autocomplete {
            (cur_cot_flags & (kOptCotFlagPreinsert | kOptCotFlagFuzzy | kOptCotFlagMenuone))
                == (kOptCotFlagPreinsert | kOptCotFlagMenuone)
        } else {
            (cur_cot_flags & (kOptCotFlagPreinsert | kOptCotFlagFuzzy)) == kOptCotFlagPreinsert
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_preinsert_effect() -> bool {
    unsafe {
        if !ins_compl_has_preinsert() && !ins_compl_preinsert_longest() {
            return false;
        }
        (*curwin).w_cursor.col < compl_ins_end_col
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_bs() -> c_int {
    unsafe {
        if ins_compl_preinsert_effect() {
            ins_compl_delete(false);
        }

        let mut line = get_cursor_line_ptr();
        let mut p = line.add((*curwin).w_cursor.col as usize);
        mb_ptr_back(line, &raw mut p);
        let p_off = p.offset_from(line);

        if (p_off as c_int) - compl_col < 0
            || ((p_off as c_int) - compl_col == 0 && !ctrl_x_mode_omni())
            || ctrl_x_mode_eval()
            || (!can_bs(BS_START) && (p_off as c_int) - compl_col - compl_length < 0)
        {
            return K_BS;
        }

        if (*curwin).w_cursor.col <= compl_col + compl_length || ins_compl_need_restart() {
            ins_compl_restart();
        }

        line = get_cursor_line_ptr();

        api_clear_string(&raw mut compl_leader);
        compl_leader = cbuf_to_string(
            line.add(compl_col as usize),
            (p_off - compl_col as isize) as usize,
        );

        if compl_autocomplete && !compl_first_match.is_null() && !ins_compl_has_preinsert() {
            compl_shown_match = compl_first_match;
        }

        ins_compl_new_leader();
        if !compl_shown_match.is_null() {
            compl_curr_match = compl_shown_match;
        }
        NUL as c_int
    }
}

#[inline]
unsafe fn ins_compl_refresh_always() -> bool {
    unsafe { (ctrl_x_mode_function() || ctrl_x_mode_omni()) && compl_opt_refresh_always }
}

#[inline]
unsafe fn ins_compl_need_restart() -> bool {
    unsafe { compl_was_interrupted || ins_compl_refresh_always() }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_has_autocomplete() -> bool {
    unsafe {
        if (*curbuf).b_p_ac >= 0 {
            (*curbuf).b_p_ac != 0
        } else {
            p_ac != 0
        }
    }
}

unsafe fn ins_compl_fuzzy_sort() {
    unsafe {
        let cur_cot_flags = get_cot_flags();

        set_fuzzy_score();
        if (cur_cot_flags & kOptCotFlagNosort) == 0 {
            sort_compl_match_list(Some(cp_compare_fuzzy));
            if (cur_cot_flags & (kOptCotFlagNoinsert | kOptCotFlagNoselect)) == kOptCotFlagNoinsert
            {
                let target = if compl_shows_dir_forward() {
                    compl_first_match
                } else {
                    (*compl_first_match).cp_prev
                };
                let none_selected = ptr::eq(compl_shown_match, target);
                if !none_selected {
                    compl_shown_match = if !compl_autocomplete && compl_shows_dir_forward() {
                        (*compl_first_match).cp_next
                    } else {
                        compl_first_match
                    };
                }
            }
        }
    }
}

unsafe fn ins_compl_new_leader() {
    unsafe {
        ins_compl_del_pum();
        ins_compl_delete(true);
        ins_compl_insert_bytes(compl_leader.data.add(get_compl_len() as usize), -1);
        compl_used_match = false;

        let save_w_wrow = (*curwin).w_wrow;
        let save_w_leftcol = (*curwin).w_leftcol;
        if compl_started {
            ins_compl_set_original_text(compl_leader.data, compl_leader.size);
            if is_cpt_func_refresh_always() {
                cpt_compl_refresh();
            }
            if cot_fuzzy() {
                ins_compl_fuzzy_sort();
            }
        } else {
            spell_bad_len = 0;
            compl_restarting = true;
            if ins_compl_has_autocomplete() {
                ins_compl_enable_autocomplete();
            } else {
                compl_autocomplete = false;
            }
            if ins_complete(Ctrl_N, false) == FAIL {
                compl_cont_status = 0;
            }
            compl_restarting = false;
        }

        compl_enter_selects = !compl_used_match && compl_selected_item != -1;

        if !compl_interrupted {
            show_pum(save_w_wrow, save_w_leftcol);
        }

        if compl_match_array.is_null() {
            compl_enter_selects = false;
        } else if ins_compl_has_preinsert() && compl_leader.size > 0 {
            ins_compl_insert(true, false);
        } else if compl_started
            && ins_compl_preinsert_longest()
            && compl_leader.size > 0
            && !ins_compl_preinsert_effect()
        {
            ins_compl_insert(true, true);
        }
        if ins_compl_refresh_always() {
            compl_enter_selects = false;
        }
    }
}

#[inline]
unsafe fn get_compl_len() -> c_int {
    unsafe { ((*curwin).w_cursor.col - compl_col).max(0) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_addleader(c: c_int) {
    unsafe {
        if ins_compl_preinsert_effect() {
            ins_compl_delete(false);
        }

        if stop_arrow() == FAIL {
            return;
        }
        let cc = utf_char2len(c);
        if cc > 1 {
            let mut buf: [c_char; MB_MAXCHAR + 1] = [0; MB_MAXCHAR + 1];
            utf_char2bytes(c, buf.as_mut_ptr());
            buf[cc as usize] = NUL;
            ins_char_bytes(buf.as_mut_ptr(), cc as usize);
        } else {
            ins_char(c);
        }

        if ins_compl_need_restart() {
            ins_compl_restart();
        }

        api_clear_string(&raw mut compl_leader);
        compl_leader = cbuf_to_string(
            get_cursor_line_ptr().add(compl_col as usize),
            ((*curwin).w_cursor.col - compl_col) as usize,
        );
        ins_compl_new_leader();
    }
}

unsafe fn ins_compl_restart() {
    unsafe {
        update_screen();
        ins_compl_free();
        compl_started = false;
        compl_matches = 0;
        compl_cont_status = 0;
        compl_cont_mode = 0;
        cpt_sources_clear();
        compl_autocomplete = false;
        compl_from_nonkeyword = false;
        compl_autostarted = false;
        compl_num_bests = 0;
    }
}

unsafe fn ins_compl_set_original_text(str_ptr: *const c_char, len: usize) {
    unsafe {
        let match_ = find_original_text_match();
        if match_.is_null() {
            return;
        }

        api_clear_string(&raw mut (*match_).cp_str);
        (*match_).cp_str = cbuf_to_string(str_ptr, len);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_addfrommatch() {
    unsafe {
        let len = (*curwin).w_cursor.col - compl_col;
        debug_assert!(!compl_shown_match.is_null());
        let mut p = (*compl_shown_match).cp_str.data;
        if ((*compl_shown_match).cp_str.size as c_int) <= len {
            if !match_at_original_text(compl_shown_match) {
                return;
            }

            p = ptr::null_mut();
            let mut plen: usize = 0;
            let mut cp = (*compl_shown_match).cp_next;
            while !cp.is_null() && !is_first_match(cp) {
                if compl_leader.data.is_null()
                    || ins_compl_equal(cp, compl_leader.data, compl_leader.size)
                {
                    p = (*cp).cp_str.data;
                    plen = (*cp).cp_str.size;
                    break;
                }
                cp = (*cp).cp_next;
            }
            if p.is_null() || (plen as c_int) <= len {
                return;
            }
        }
        p = p.add(len as usize);
        let c = utf_ptr2char(p);
        ins_compl_addleader(c);
    }
}

unsafe fn set_ctrl_x_mode(c: c_int) -> bool {
    unsafe {
        let mut retval = false;

        match c {
            Ctrl_E | Ctrl_Y => {
                ctrl_x_mode = CTRL_X_SCROLL;
                if (State & REPLACE_FLAG) == 0 {
                    edit_submode = gettext(c" (insert) Scroll (^E/^Y)".as_ptr());
                } else {
                    edit_submode = gettext(c" (replace) Scroll (^E/^Y)".as_ptr());
                }
                edit_submode_pre = ptr::null();
                redraw_mode = true;
            }
            Ctrl_L => {
                ctrl_x_mode = CTRL_X_WHOLE_LINE;
            }
            Ctrl_F => {
                ctrl_x_mode = CTRL_X_FILES;
            }
            Ctrl_K => {
                ctrl_x_mode = CTRL_X_DICTIONARY;
            }
            Ctrl_R => {
                if vpeekc() != b'=' as c_int {
                    ctrl_x_mode = CTRL_X_REGISTER;
                }
            }
            Ctrl_T => {
                ctrl_x_mode = CTRL_X_THESAURUS;
            }
            Ctrl_U => {
                ctrl_x_mode = CTRL_X_FUNCTION;
            }
            Ctrl_O => {
                ctrl_x_mode = CTRL_X_OMNI;
            }
            x if x == b's' as c_int || x == Ctrl_S => {
                ctrl_x_mode = CTRL_X_SPELL;
                emsg_off += 1;
                spell_back_to_badword();
                emsg_off -= 1;
            }
            Ctrl_RSB => {
                ctrl_x_mode = CTRL_X_TAGS;
            }
            Ctrl_I | K_S_TAB => {
                ctrl_x_mode = CTRL_X_PATH_PATTERNS;
            }
            Ctrl_D => {
                ctrl_x_mode = CTRL_X_PATH_DEFINES;
            }
            Ctrl_V | Ctrl_Q => {
                ctrl_x_mode = CTRL_X_CMDLINE;
            }
            Ctrl_Z => {
                ctrl_x_mode = CTRL_X_NORMAL;
                edit_submode = ptr::null();
                redraw_mode = true;
                retval = true;
            }
            Ctrl_P | Ctrl_N => {
                if (compl_cont_status & CONT_INTRPT) == 0 {
                    compl_cont_status |= CONT_LOCAL;
                } else if compl_cont_mode != 0 {
                    compl_cont_status &= !CONT_LOCAL;
                }
                ctrl_x_mode = CTRL_X_NORMAL;
                edit_submode = ptr::null();
                redraw_mode = true;
            }
            _ => {
                if c == Ctrl_X {
                    if compl_cont_mode != 0 {
                        compl_cont_status = 0;
                    } else {
                        compl_cont_mode = CTRL_X_NOT_DEFINED_YET;
                    }
                }
                ctrl_x_mode = CTRL_X_NORMAL;
                edit_submode = ptr::null();
                redraw_mode = true;
            }
        }

        retval
    }
}

unsafe fn ins_compl_stop(c: c_int, prev_mode: c_int, mut retval: bool) -> bool {
    unsafe {
        if ins_compl_preinsert_effect() && ins_compl_win_active(curwin) {
            ins_compl_delete(false);
        }

        if !compl_curr_match.is_null() || !compl_leader.data.is_null() || c == Ctrl_E {
            let mut ptr: *mut c_char = ptr::null_mut();
            if !compl_curr_match.is_null() && compl_used_match && c != Ctrl_E {
                ptr = (*compl_curr_match).cp_str.data;
            }
            ins_compl_fixRedoBufForLeader(ptr);
        }

        let mut want_cindent = get_can_cindent() && crate::indent_c::cindent_on();

        if compl_cont_mode == CTRL_X_WHOLE_LINE {
            if want_cindent {
                do_c_expr_indent();
                want_cindent = false;
            }
        } else if !compl_autocomplete || compl_used_match {
            let prev_col = (*curwin).w_cursor.col;

            if prev_col > 0 {
                dec_cursor();
            }

            if Ins.moved == kInsNone && !Ins.need_undo && c != Ctrl_E {
                insertchar(NUL as c_int, 0, -1);
            }

            if prev_col > 0 && *get_cursor_line_ptr().add((*curwin).w_cursor.col as usize) != NUL {
                inc_cursor();
            }
        }

        let mut word = empty_nvim_string();
        let is_commit = pum_visible() && ins_compl_commit_char(c);
        if (c == Ctrl_Y
            || is_commit
            || (compl_enter_selects && (c == CAR || c == K_KENTER || c == NL)))
            && pum_visible()
        {
            word = copy_string((*compl_shown_match).cp_str, ptr::null_mut());
            if !is_commit {
                retval = true;
            }
            redrawWinline(curwin, (*curwin).w_cursor.lnum);
        }

        if word.data.is_null()
            && c != Ctrl_E
            && compl_used_match
            && compl_match_array.is_null()
            && !compl_curr_match.is_null()
            && !(*compl_curr_match).cp_str.data.is_null()
        {
            word = copy_string((*compl_curr_match).cp_str, ptr::null_mut());
        }

        if c == Ctrl_E {
            ins_compl_delete(false);
            let mut p: *const c_char = ptr::null();
            let mut plen: usize = 0;
            if !compl_leader.data.is_null() {
                p = compl_leader.data;
                plen = compl_leader.size;
            } else if !compl_first_match.is_null() {
                p = compl_orig_text.data;
                plen = compl_orig_text.size;
            }
            if !p.is_null() {
                let compl_len = get_compl_len();
                if (plen as c_int) > compl_len {
                    ins_compl_insert_bytes(p.add(compl_len as usize), (plen as c_int) - compl_len);
                }
            }
            restore_orig_extmarks();
            retval = true;
        }

        auto_format(false, true);

        ctrl_x_mode = prev_mode;
        ins_apply_autocmds(EVENT_COMPLETEDONEPRE);

        ins_compl_free();
        compl_started = false;
        compl_matches = 0;
        if !shortmess(kShmCompletionmenu) {
            msg_clr_cmdline();
        }
        ctrl_x_mode = CTRL_X_NORMAL;
        compl_enter_selects = false;
        if !edit_submode.is_null() {
            edit_submode = ptr::null();
            redraw_mode = true;
        }
        compl_autocomplete = false;
        compl_from_nonkeyword = false;
        compl_autostarted = false;
        compl_num_bests = 0;
        compl_ins_end_col = 0;

        if c == Ctrl_C && bt_cmdwin(curbuf) {
            update_screen();
        }

        if want_cindent && in_cinkeys(KEY_COMPLETE, b' ' as c_int, inindent(0)) {
            do_c_expr_indent();
        }
        do_autocmd_completedone(c, prev_mode, &raw mut word);
        xfree(word.data.cast());

        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_cancel() -> bool {
    unsafe { ins_compl_stop(b' ' as c_int, ctrl_x_mode, true) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_prep(c: c_int) -> bool {
    unsafe {
        let mut retval = false;
        let prev_mode = ctrl_x_mode;

        if c != Ctrl_R && vim_is_ctrl_x_key(c) {
            edit_submode_extra = ptr::null_mut();
        }

        if c == K_SELECT
            || c == K_MOUSEDOWN
            || c == K_MOUSEUP
            || c == K_MOUSELEFT
            || c == K_MOUSERIGHT
            || c == K_MOUSEMOVE
            || c == K_EVENT
            || c == K_COMMAND
            || c == K_LUA
        {
            return retval;
        }

        if ctrl_x_mode == CTRL_X_CMDLINE_CTRL_X && c != Ctrl_X {
            if c == Ctrl_V
                || c == Ctrl_Q
                || c == Ctrl_Z
                || ins_compl_pum_key(c)
                || !vim_is_ctrl_x_key(c)
            {
                ctrl_x_mode = CTRL_X_CMDLINE;
                if c == Ctrl_Z {
                    retval = true;
                }
            } else {
                ctrl_x_mode = CTRL_X_CMDLINE;
                ins_compl_prep(b' ' as c_int);
                ctrl_x_mode = CTRL_X_NOT_DEFINED_YET;
            }
        }

        if ctrl_x_mode_not_defined_yet() || (ctrl_x_mode_normal() && !compl_started) {
            compl_get_longest = (get_cot_flags() & kOptCotFlagLongest) != 0;
            compl_used_match = true;
        }

        if ctrl_x_mode_not_defined_yet() {
            retval = set_ctrl_x_mode(c);
        } else if ctrl_x_mode_not_default() {
            if c != Ctrl_R && !vim_is_ctrl_x_key(c) {
                ctrl_x_mode = if ctrl_x_mode_scroll() {
                    CTRL_X_NORMAL
                } else {
                    CTRL_X_FINISHED
                };
                edit_submode = ptr::null();
            }
            redraw_mode = true;
        }

        if compl_started || ctrl_x_mode == CTRL_X_FINISHED {
            redraw_mode = true;
            if (ctrl_x_mode_normal()
                && c != Ctrl_N
                && c != Ctrl_P
                && c != Ctrl_R
                && !ins_compl_pum_key(c))
                || ctrl_x_mode == CTRL_X_FINISHED
            {
                retval = ins_compl_stop(c, prev_mode, retval);
            }
        } else if ctrl_x_mode == CTRL_X_LOCAL_MSG {
            do_autocmd_completedone(c, ctrl_x_mode, ptr::null_mut());
        }

        may_trigger_modechanged();

        if c != Ctrl_R && !vim_is_ctrl_x_key(c) {
            compl_cont_status = 0;
            compl_cont_mode = 0;
        }

        retval
    }
}

unsafe fn ins_compl_fixRedoBufForLeader(ptr_arg: *const c_char) {
    unsafe {
        let mut len: usize = 0;
        let mut ptr = ptr_arg;

        if ptr.is_null() {
            if !compl_leader.data.is_null() {
                ptr = compl_leader.data;
            } else {
                return;
            }
        }
        if !compl_orig_text.data.is_null() {
            let mut p = compl_orig_text.data.cast_const();
            while *p.add(len) != NUL && *p.add(len) == *ptr.add(len) {
                len += 1;
            }
            if len > 0 {
                len -= utf_head_off(p, p.add(len)) as usize;
            }
            p = p.add(len);
            while *p != NUL {
                redo_append_char(K_BS);
                mb_ptr_adv_const(&raw mut p);
            }
        }
        redo_append_lit(ptr.add(len), -1);
    }
}

unsafe fn ins_compl_next_buf(mut buf: *mut buf_T, flag: c_int) -> *mut buf_T {
    static mut wp: *mut win_T = ptr::null_mut();
    unsafe {
        if flag == b'w' as c_int {
            if ptr::eq(buf, curbuf) || !win_valid(wp) {
                wp = curwin;
            }

            debug_assert!(!wp.is_null());
            loop {
                wp = if !(*wp).w_next.is_null() {
                    (*wp).w_next
                } else {
                    firstwin
                };
                if ptr::eq(wp, curwin) || (!(*(*wp).w_buffer).b_scanned && (*wp).w_config.focusable)
                {
                    break;
                }
            }
            buf = (*wp).w_buffer;
        } else {
            loop {
                buf = if !(*buf).b_next.is_null() {
                    (*buf).b_next
                } else {
                    firstbuf
                };
                if ptr::eq(buf, curbuf) {
                    break;
                }

                let skip_buffer = if flag == b'U' as c_int {
                    (*buf).b_p_bl != 0
                } else {
                    (*buf).b_p_bl == 0 || (*buf).b_ml.ml_mfp.is_null() != (flag == b'u' as c_int)
                };

                if !skip_buffer && !(*buf).b_scanned {
                    break;
                }
            }
        }
        buf
    }
}

unsafe fn get_cpt_sources_count(cpt: *mut c_char) -> c_int {
    unsafe {
        let mut dummy: [c_char; LSIZE] = [0; LSIZE];
        let mut count: c_int = 0;
        let mut p = cpt;

        while *p != NUL {
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
            if *p != NUL {
                let _ = copy_option_part(&raw mut p, dummy.as_mut_ptr(), LSIZE, c",".as_ptr());
                count += 1;
            }
        }

        count
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_cpt_callbacks(callbacks: *mut *mut Callback, count: c_int) {
    unsafe {
        if callbacks.is_null() || (*callbacks).is_null() {
            return;
        }

        for i in 0..(count as usize) {
            callback_free((*callbacks).add(i));
        }

        xfree((*callbacks).cast());
        *callbacks = ptr::null_mut();
    }
}

unsafe fn copy_cpt_callbacks(
    dest: *mut *mut Callback,
    dest_cnt: *mut c_int,
    src: *mut Callback,
    cnt: c_int,
) {
    unsafe {
        clear_cpt_callbacks(dest, *dest_cnt);
        *dest_cnt = 0;

        if cnt == 0 {
            return;
        }

        *dest = xcalloc(cnt as usize, size_of::<Callback>()).cast::<Callback>();
        *dest_cnt = cnt;

        for i in 0..(cnt as usize) {
            if (*src.add(i)).cb_type != kCallbackNone {
                callback_copy((*dest).add(i), src.add(i));
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_buflocal_cpt_callbacks(buf: *mut buf_T) {
    unsafe {
        if buf.is_null() {
            return;
        }
        copy_cpt_callbacks(
            &raw mut (*buf).b_p_cpt_cb,
            &raw mut (*buf).b_p_cpt_count,
            cpt_cb,
            cpt_cb_count,
        );
    }
}

unsafe fn parse_cpt_callbacks(cpt: *mut c_char, cbp: *mut *mut Callback, cnt: *mut c_int) {
    unsafe {
        clear_cpt_callbacks(cbp, *cnt);
        *cnt = 0;

        let count = get_cpt_sources_count(cpt);
        if count == 0 {
            return;
        }

        *cbp = xcalloc(count as usize, size_of::<Callback>()).cast::<Callback>();
        *cnt = count;

        let mut buf: [c_char; LSIZE] = [0; LSIZE];
        let mut idx: usize = 0;
        let mut p = cpt;
        while *p != NUL {
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
            if *p != NUL {
                let slen = copy_option_part(&raw mut p, buf.as_mut_ptr(), LSIZE, c",".as_ptr());
                if slen > 0 && buf[0] == b'F' as c_char && buf[1] != NUL {
                    let caret = vim_strchr(buf.as_mut_ptr(), b'^' as c_int);
                    if !caret.is_null() {
                        *caret = NUL;
                    }
                    if option_set_callback_func(buf.as_mut_ptr().add(1), (*cbp).add(idx)) != OK {
                        (*(*cbp).add(idx)).cb_type = kCallbackNone;
                    }
                }
                idx += 1;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_cpt_callbacks(args: *mut optset_T) -> c_int {
    unsafe {
        let opt_flags = (*args).os_flags;

        if curbuf.is_null() {
            return FAIL;
        }

        if (opt_flags & OPT_GLOBAL) == 0 {
            parse_cpt_callbacks(
                (*curbuf).b_p_cpt,
                &raw mut (*curbuf).b_p_cpt_cb,
                &raw mut (*curbuf).b_p_cpt_count,
            );
        }

        if (opt_flags & OPT_LOCAL) != 0 {
            return OK;
        }

        if (opt_flags & OPT_GLOBAL) != 0 {
            parse_cpt_callbacks(p_cpt, &raw mut cpt_cb, &raw mut cpt_cb_count);
            return OK;
        }

        copy_cpt_callbacks(
            &raw mut cpt_cb,
            &raw mut cpt_cb_count,
            (*curbuf).b_p_cpt_cb,
            (*curbuf).b_p_cpt_count,
        );
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_ref_in_cpt_callbacks(
    callbacks: *mut Callback,
    count: c_int,
    copyID: c_int,
) -> bool {
    unsafe {
        let mut abort = false;

        if callbacks.is_null() {
            return false;
        }

        for i in 0..(count as usize) {
            abort = abort
                || set_ref_in_callback(callbacks.add(i), copyID, ptr::null_mut(), ptr::null_mut());
        }
        abort
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_ref_in_insexpand_funcs(copyID: c_int) -> bool {
    unsafe {
        let mut abort =
            set_ref_in_callback(&raw mut p_cfu, copyID, ptr::null_mut(), ptr::null_mut());
        abort =
            abort || set_ref_in_callback(&raw mut p_ofu, copyID, ptr::null_mut(), ptr::null_mut());
        abort = abort
            || set_ref_in_callback(&raw mut p_tsrfu, copyID, ptr::null_mut(), ptr::null_mut());
        abort = abort || set_ref_in_cpt_callbacks(cpt_cb, cpt_cb_count, copyID);
        abort
    }
}

unsafe fn get_insert_callback(type_: c_int) -> *mut Callback {
    unsafe {
        if type_ == CTRL_X_FUNCTION {
            return &raw mut (*curbuf).b_p_cfu;
        }
        if type_ == CTRL_X_OMNI {
            return &raw mut (*curbuf).b_p_ofu;
        }
        if (*curbuf).b_p_tsrfu.cb_type != kCallbackNone {
            &raw mut (*curbuf).b_p_tsrfu
        } else {
            &raw mut p_tsrfu
        }
    }
}

unsafe fn expand_by_function(type_: c_int, base: *mut c_char, mut cb: *mut Callback) {
    unsafe {
        let mut matchlist: *mut list_T = ptr::null_mut();
        let mut matchdict: *mut dict_T = ptr::null_mut();
        let mut rettv: typval_T = std::mem::zeroed();
        let save_State = State;

        debug_assert!(!curbuf.is_null());

        let is_cpt_function = !cb.is_null();
        let use_sandbox =
            is_cpt_function && was_set_insecurely(curwin, kOptComplete, OPT_LOCAL) != 0;
        if !is_cpt_function {
            cb = get_insert_callback(type_);
            if (*cb).cb_type == kCallbackNone {
                return;
            }
        }

        let mut args: [typval_T; 3] = std::mem::zeroed();
        args[0].v_type = VAR_NUMBER;
        args[1].v_type = VAR_STRING;
        args[2].v_type = VAR_UNKNOWN;
        args[0].vval.v_number = 0;
        args[1].vval.v_string = if !base.is_null() {
            base
        } else {
            c"".as_ptr().cast_mut()
        };

        let pos = (*curwin).w_cursor;
        textlock += 1;
        if use_sandbox {
            sandbox += 1;
        }

        let called = callback_call(cb, 2, args.as_mut_ptr(), &raw mut rettv);
        if use_sandbox {
            sandbox -= 1;
        }
        if called {
            match rettv.v_type {
                VAR_LIST => {
                    matchlist = rettv.vval.v_list;
                }
                VAR_DICT => {
                    matchdict = rettv.vval.v_dict;
                }
                _ => {
                    tv_clear(&raw mut rettv);
                }
            }
        }
        textlock -= 1;

        (*curwin).w_cursor = pos;
        check_cursor(curwin);
        validate_cursor(curwin);
        if !equalpos((*curwin).w_cursor, pos) {
            emsg(gettext(E_COMPLDEL.as_ptr()));
        } else if !matchlist.is_null() {
            ins_compl_add_list(matchlist);
        } else if !matchdict.is_null() {
            ins_compl_add_dict(matchdict);
        }

        State = save_State;

        if !matchdict.is_null() {
            tv_dict_unref(matchdict);
        }
        if !matchlist.is_null() {
            tv_list_unref(matchlist);
        }
    }
}

#[inline]
unsafe fn get_user_highlight_id(hlname: *const c_char) -> c_int {
    unsafe {
        if !hlname.is_null() && *hlname != NUL {
            return syn_check_group(hlname, strlen(hlname));
        }
        0
    }
}

unsafe fn ins_compl_add_tv(tv: *mut typval_T, dir: Direction, fast: bool) -> c_int {
    unsafe {
        let word: *const c_char;
        let mut dup = false;
        let mut empty = false;
        let mut flags = if fast { CP_FAST } else { 0 };
        let mut preselect = false;
        let mut cptext: [*mut c_char; CPT_COUNT] = [ptr::null_mut(); CPT_COUNT];
        let mut commit_chars: *mut c_char = ptr::null_mut();
        let mut user_hl: [c_int; 2] = [0, 0];
        let mut user_data: typval_T = std::mem::zeroed();
        user_data.v_type = VAR_UNKNOWN;

        if (*tv).v_type == VAR_DICT && !(*tv).vval.v_dict.is_null() {
            let d = (*tv).vval.v_dict;
            word = tv_dict_get_string(d, c"word".as_ptr(), false);
            cptext[CPT_ABBR] = tv_dict_get_string(d, c"abbr".as_ptr(), true);
            cptext[CPT_MENU] = tv_dict_get_string(d, c"menu".as_ptr(), true);
            cptext[CPT_KIND] = tv_dict_get_string(d, c"kind".as_ptr(), true);
            cptext[CPT_INFO] = tv_dict_get_string(d, c"info".as_ptr(), true);
            commit_chars = tv_dict_get_string(d, c"commit_chars".as_ptr(), true);

            let user_abbr_hlname = tv_dict_get_string(d, c"abbr_hlgroup".as_ptr(), false);
            user_hl[0] = get_user_highlight_id(user_abbr_hlname);

            let user_kind_hlname = tv_dict_get_string(d, c"kind_hlgroup".as_ptr(), false);
            user_hl[1] = get_user_highlight_id(user_kind_hlname);

            tv_dict_get_tv(d, c"user_data".as_ptr(), &raw mut user_data);

            if tv_dict_get_number(d, c"icase".as_ptr()) != 0 {
                flags |= CP_ICASE;
            }
            dup = tv_dict_get_number(d, c"dup".as_ptr()) != 0;
            empty = tv_dict_get_number(d, c"empty".as_ptr()) != 0;
            preselect = tv_dict_get_number(d, c"preselect".as_ptr()) != 0;
            if !tv_dict_get_string(d, c"equal".as_ptr(), false).is_null()
                && tv_dict_get_number(d, c"equal".as_ptr()) != 0
            {
                flags |= CP_EQUAL;
            }
        } else {
            word = tv_get_string_chk(tv);
        }
        if word.is_null() || (!empty && *word == NUL) {
            free_cptext(cptext.as_ptr());
            tv_clear(&raw mut user_data);
            xfree(commit_chars.cast());
            return FAIL;
        }
        let status = ins_compl_add(
            word.cast_mut(),
            -1,
            ptr::null_mut(),
            cptext.as_ptr(),
            true,
            &raw mut user_data,
            dir,
            flags,
            dup,
            user_hl.as_ptr(),
            FUZZY_SCORE_NONE,
            preselect,
            commit_chars,
        );
        if status != OK {
            tv_clear(&raw mut user_data);
        }
        status
    }
}

unsafe fn ins_compl_add_list(list: *mut list_T) {
    unsafe {
        let mut dir = compl_direction;

        if list.is_null() {
            return;
        }
        let mut li = (*list).lv_first;
        while !li.is_null() {
            if ins_compl_add_tv(&raw mut (*li).li_tv, dir, true) == OK {
                dir = FORWARD;
            } else if did_emsg != 0 {
                break;
            }
            li = (*li).li_next;
        }
    }
}

unsafe fn ins_compl_add_dict(dict: *mut dict_T) {
    unsafe {
        compl_opt_refresh_always = false;
        let di_refresh = tv_dict_find(dict, c"refresh".as_ptr(), 7);
        if !di_refresh.is_null() && (*di_refresh).di_tv.v_type == VAR_STRING {
            let v = (*di_refresh).di_tv.vval.v_string;
            if !v.is_null() && strcmp(v, c"always".as_ptr()) == 0 {
                compl_opt_refresh_always = true;
            }
        }

        let di_words = tv_dict_find(dict, c"words".as_ptr(), 5);
        if !di_words.is_null() && (*di_words).di_tv.v_type == VAR_LIST {
            ins_compl_add_list((*di_words).di_tv.vval.v_list);
        }
    }
}

unsafe fn save_orig_extmarks() {
    unsafe {
        extmark_splice_delete(
            curbuf,
            (*curwin).w_cursor.lnum - 1,
            compl_col,
            (*curwin).w_cursor.lnum - 1,
            compl_col + compl_length,
            &raw mut compl_orig_extmarks,
            true,
            kExtmarkUndo,
        );
    }
}

unsafe fn restore_orig_extmarks() {
    unsafe {
        let mut i = compl_orig_extmarks.size as isize - 1;
        while i > -1 {
            let undo_info = *compl_orig_extmarks.items.add(i as usize);
            extmark_apply_undo(undo_info, true);
            i -= 1;
        }
    }
}

unsafe fn set_completion(mut startcol: colnr_T, list: *mut list_T) {
    unsafe {
        let mut flags = CP_ORIGINAL_TEXT;
        let cur_cot_flags = get_cot_flags();
        let compl_longest = (cur_cot_flags & kOptCotFlagLongest) != 0;
        let compl_no_insert = (cur_cot_flags & kOptCotFlagNoinsert) != 0;
        let compl_no_select = (cur_cot_flags & kOptCotFlagNoselect) != 0;

        if compl_started || ctrl_x_mode_not_default() {
            ins_compl_prep(b' ' as c_int);
        }
        ins_compl_clear();
        ins_compl_free();
        compl_get_longest = compl_longest;

        compl_direction = FORWARD;
        if startcol > (*curwin).w_cursor.col {
            startcol = (*curwin).w_cursor.col;
        }
        compl_col = startcol;
        compl_lnum = (*curwin).w_cursor.lnum;
        compl_length = (*curwin).w_cursor.col - startcol;
        compl_orig_text = cbuf_to_string(
            get_cursor_line_ptr().add(compl_col as usize),
            compl_length as usize,
        );
        save_orig_extmarks();
        if p_ic != 0 {
            flags |= CP_ICASE;
        }
        if ins_compl_add(
            compl_orig_text.data,
            compl_orig_text.size as c_int,
            ptr::null_mut(),
            ptr::null(),
            false,
            ptr::null_mut(),
            0,
            flags | CP_FAST,
            false,
            ptr::null(),
            FUZZY_SCORE_NONE,
            false,
            ptr::null_mut(),
        ) != OK
        {
            return;
        }

        ctrl_x_mode = CTRL_X_EVAL;

        ins_compl_add_list(list);
        compl_matches = ins_compl_make_cyclic();
        compl_started = true;
        compl_used_match = true;
        compl_cont_status = 0;
        let save_w_wrow = (*curwin).w_wrow;
        let save_w_leftcol = (*curwin).w_leftcol;

        compl_curr_match = compl_first_match;
        let no_select = compl_no_select || compl_longest;
        if !compl_preselect_match.is_null() && !no_select {
            compl_curr_match = (*compl_preselect_match).cp_prev;
            ins_complete(if compl_no_insert { K_DOWN } else { Ctrl_N }, false);
        } else if compl_no_insert || no_select {
            ins_complete(K_DOWN, false);
            if no_select {
                ins_complete(K_UP, false);
            }
        } else {
            ins_complete(Ctrl_N, false);
        }
        compl_enter_selects = compl_no_insert;

        if !compl_interrupted {
            show_pum(save_w_wrow, save_w_leftcol);
        }

        may_trigger_modechanged();
        ui_flush();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_complete(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (State & MODE_INSERT) == 0 {
            emsg(gettext(
                c"E785: complete() can only be used in Insert mode".as_ptr(),
            ));
            return;
        }

        if !undo_allowed(curbuf) {
            return;
        }

        if (*argvars.add(1)).v_type != VAR_LIST {
            emsg(gettext((&raw const e_invarg).cast()));
        } else {
            let startcol = tv_get_number_chk(argvars, ptr::null_mut()) as colnr_T;
            if startcol > 0 {
                set_completion(startcol - 1, (*argvars.add(1)).vval.v_list);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_complete_add(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(ins_compl_add_tv(argvars, 0, false));
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_complete_check(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let saved = RedrawingDisabled;
        RedrawingDisabled = 0;
        ins_compl_check_keys(0, true);
        (*rettv).vval.v_number = varnumber_T::from(ins_compl_interrupted());
        RedrawingDisabled = saved;
    }
}

unsafe fn ins_compl_mode() -> NvimString {
    unsafe {
        if ctrl_x_mode_not_defined_yet() || ctrl_x_mode_scroll() || compl_started {
            let idx = (ctrl_x_mode & !CTRL_X_WANT_IDENT) as usize;
            if let Some(mode_cstr) = CTRL_X_MODE_NAMES.get(idx).copied().flatten() {
                return cbuf_as_string(mode_cstr.as_ptr().cast_mut(), mode_cstr.to_bytes().len());
            }
        }
        cbuf_as_string(c"".as_ptr().cast_mut(), 0)
    }
}

unsafe fn ins_compl_update_sequence_numbers() {
    unsafe {
        let mut number: c_int = 0;

        if compl_dir_forward() {
            let mut match_ = (*compl_curr_match).cp_prev;
            while !match_.is_null() && !is_first_match(match_) {
                if (*match_).cp_number != -1 {
                    number = (*match_).cp_number;
                    break;
                }
                match_ = (*match_).cp_prev;
            }
            if !match_.is_null() {
                match_ = (*match_).cp_next;
                while !match_.is_null() && (*match_).cp_number == -1 {
                    number += 1;
                    (*match_).cp_number = number;
                    match_ = (*match_).cp_next;
                }
            }
        } else {
            debug_assert!(compl_direction == BACKWARD);
            let mut match_ = (*compl_curr_match).cp_next;
            while !match_.is_null() && !is_first_match(match_) {
                if (*match_).cp_number != -1 {
                    number = (*match_).cp_number;
                    break;
                }
                match_ = (*match_).cp_next;
            }
            if !match_.is_null() {
                match_ = (*match_).cp_prev;
                while !match_.is_null() && (*match_).cp_number == -1 {
                    number += 1;
                    (*match_).cp_number = number;
                    match_ = (*match_).cp_prev;
                }
            }
        }
    }
}

unsafe fn fill_complete_info_dict(di: *mut dict_T, match_: *mut compl_T, add_match: bool) {
    unsafe {
        tv_dict_add_str_len(
            di,
            c"word".as_ptr(),
            4,
            (*match_).cp_str.data,
            (*match_).cp_str.size as c_int,
        );
        tv_dict_add_str(di, c"abbr".as_ptr(), 4, (*match_).cp_text[CPT_ABBR]);
        tv_dict_add_str(
            di,
            c"menu".as_ptr(),
            4,
            if !(*match_).cp_text[CPT_MENU].is_null() {
                (*match_).cp_text[CPT_MENU]
            } else {
                (*match_).cp_fname
            },
        );
        tv_dict_add_str(di, c"kind".as_ptr(), 4, (*match_).cp_text[CPT_KIND]);
        tv_dict_add_str(di, c"info".as_ptr(), 4, (*match_).cp_text[CPT_INFO]);
        tv_dict_add_str(
            di,
            c"abbr_hlgroup".as_ptr(),
            12,
            syn_id2name((*match_).cp_user_abbr_hl_id),
        );
        tv_dict_add_str(
            di,
            c"kind_hlgroup".as_ptr(),
            12,
            syn_id2name((*match_).cp_user_kind_hl_id),
        );
        if add_match {
            tv_dict_add_bool(
                di,
                c"match".as_ptr(),
                5,
                if (*match_).cp_in_match_array { 1 } else { 0 },
            );
        }
        if ((*match_).cp_flags & CP_EQUAL) != 0 {
            tv_dict_add_nr(di, c"equal".as_ptr(), 5, 1);
        }
        if (*match_).cp_preselect {
            tv_dict_add_nr(di, c"preselect".as_ptr(), 9, 1);
        }
        if !(*match_).cp_commit_chars.is_null() {
            tv_dict_add_str(di, c"commit_chars".as_ptr(), 12, (*match_).cp_commit_chars);
        }
        if (*match_).cp_user_data.v_type == VAR_UNKNOWN {
            tv_dict_add_str_len(di, c"user_data".as_ptr(), 9, c"".as_ptr(), 0);
        } else {
            tv_dict_add_tv(
                di,
                c"user_data".as_ptr(),
                9,
                &raw mut (*match_).cp_user_data,
            );
        }
    }
}

unsafe fn get_complete_info(what_list: *mut list_T, retdict: *mut dict_T) {
    unsafe {
        let mut what_flag: c_int;

        if what_list.is_null() {
            what_flag = CI_WHAT_ALL & !(CI_WHAT_MATCHES | CI_WHAT_COMPLETED | CI_WHAT_AUTO);
        } else {
            what_flag = 0;
            let mut item: *mut listitem_T = (*what_list).lv_first;
            while !item.is_null() {
                let what = tv_get_string(&raw const (*item).li_tv);
                if strcmp(what, c"mode".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_MODE;
                } else if strcmp(what, c"pum_visible".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_PUM_VISIBLE;
                } else if strcmp(what, c"items".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_ITEMS;
                } else if strcmp(what, c"selected".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_SELECTED;
                } else if strcmp(what, c"completed".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_COMPLETED;
                } else if strcmp(what, c"preinserted_text".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_PREINSERTED_TEXT;
                } else if strcmp(what, c"matches".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_MATCHES;
                } else if strcmp(what, c"auto".as_ptr()) == 0 {
                    what_flag |= CI_WHAT_AUTO;
                }
                item = (*item).li_next;
            }
        }

        let mut ret = OK;
        if (what_flag & CI_WHAT_MODE) != 0 {
            let mode = ins_compl_mode();
            ret = tv_dict_add_str_len(retdict, c"mode".as_ptr(), 4, mode.data, mode.size as c_int);
        }

        if ret == OK && (what_flag & CI_WHAT_PUM_VISIBLE) != 0 {
            ret = tv_dict_add_nr(
                retdict,
                c"pum_visible".as_ptr(),
                11,
                varnumber_T::from(pum_visible()),
            );
        }

        if ret == OK && (what_flag & CI_WHAT_AUTO) != 0 {
            ret = tv_dict_add_nr(
                retdict,
                c"auto".as_ptr(),
                4,
                varnumber_T::from(compl_autostarted),
            );
        }

        if ret == OK && (what_flag & CI_WHAT_PREINSERTED_TEXT) != 0 {
            let line = get_cursor_line_ptr();
            let len = compl_ins_end_col - (*curwin).w_cursor.col;
            ret = tv_dict_add_str_len(
                retdict,
                c"preinserted_text".as_ptr(),
                16,
                if len > 0 {
                    line.add((*curwin).w_cursor.col as usize)
                } else {
                    c"".as_ptr()
                },
                len.max(0),
            );
        }

        if ret == OK
            && (what_flag
                & (CI_WHAT_ITEMS | CI_WHAT_SELECTED | CI_WHAT_MATCHES | CI_WHAT_COMPLETED))
                != 0
        {
            let mut li: *mut list_T = ptr::null_mut();
            let mut selected_idx: c_int = -1;
            let has_items = (what_flag & CI_WHAT_ITEMS) != 0;
            let has_matches = (what_flag & CI_WHAT_MATCHES) != 0;
            let has_completed = (what_flag & CI_WHAT_COMPLETED) != 0;
            if has_items || has_matches {
                li = tv_list_alloc(kListLenMayKnow);
                let key = if has_matches && !has_items {
                    c"matches"
                } else {
                    c"items"
                };
                ret = tv_dict_add_list(retdict, key.as_ptr(), key.to_bytes().len(), li);
            }
            if ret == OK && (what_flag & CI_WHAT_SELECTED) != 0 {
                if !compl_curr_match.is_null() && (*compl_curr_match).cp_number == -1 {
                    ins_compl_update_sequence_numbers();
                }
            }
            if ret == OK && !compl_first_match.is_null() {
                let mut list_idx: c_int = 0;
                let mut match_ = compl_first_match;
                loop {
                    if !match_at_original_text(match_) {
                        if has_items || (has_matches && (*match_).cp_in_match_array) {
                            let di = tv_dict_alloc();
                            tv_list_append_dict(li, di);
                            fill_complete_info_dict(di, match_, has_matches && has_items);
                        }
                        if !compl_curr_match.is_null()
                            && (*compl_curr_match).cp_number == (*match_).cp_number
                        {
                            selected_idx = list_idx;
                        }
                        if !has_matches || (*match_).cp_in_match_array {
                            list_idx += 1;
                        }
                    }
                    match_ = (*match_).cp_next;
                    if match_.is_null() || is_first_match(match_) {
                        break;
                    }
                }
            }
            if ret == OK && (what_flag & CI_WHAT_SELECTED) != 0 {
                ret = tv_dict_add_nr(
                    retdict,
                    c"selected".as_ptr(),
                    8,
                    varnumber_T::from(selected_idx),
                );
                let wp = win_float_find(kWinInfo);
                if !wp.is_null() {
                    tv_dict_add_nr(
                        retdict,
                        c"preview_winid".as_ptr(),
                        13,
                        varnumber_T::from((*wp).handle),
                    );
                    tv_dict_add_nr(
                        retdict,
                        c"preview_bufnr".as_ptr(),
                        13,
                        varnumber_T::from((*(*wp).w_buffer).b_fnum),
                    );
                }
            }
            if ret == OK && selected_idx != -1 && has_completed {
                let di = tv_dict_alloc();
                fill_complete_info_dict(di, compl_curr_match, false);
                let _ = tv_dict_add_dict(retdict, c"completed".as_ptr(), 9, di);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_complete_info(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);

        let mut what_list: *mut list_T = ptr::null_mut();

        if (*argvars).v_type != VAR_UNKNOWN {
            if (*argvars).v_type != VAR_LIST {
                emsg(gettext((&raw const e_listreq).cast()));
                return;
            }
            what_list = (*argvars).vval.v_list;
        }
        get_complete_info(what_list, (*rettv).vval.v_dict);
    }
}

#[inline]
unsafe fn thesaurus_func_complete(type_: c_int) -> bool {
    unsafe {
        type_ == CTRL_X_THESAURUS
            && ((*curbuf).b_p_tsrfu.cb_type != kCallbackNone || p_tsrfu.cb_type != kCallbackNone)
    }
}

unsafe fn may_advance_cpt_index(cpt: *const c_char) -> bool {
    unsafe {
        let mut p = cpt;

        if cpt_sources_index == -1 {
            return false;
        }
        while *p == b',' as c_char || *p == b' ' as c_char {
            p = p.add(1);
        }
        *p != NUL
    }
}

unsafe fn process_next_cpt_value(
    st: *mut ins_compl_next_state_T,
    compl_type_arg: *mut c_int,
    start_match_pos: *mut pos_T,
    fuzzy_collect: bool,
    advance_cpt_idx: *mut bool,
) -> c_int {
    unsafe {
        let mut compl_type: c_int = -1;
        let mut status = INS_COMPL_CPT_OK;
        let skip_source = compl_autocomplete && compl_from_nonkeyword;

        (*st).found_all = false;
        *advance_cpt_idx = false;

        while *(*st).e_cpt == b',' as c_char || *(*st).e_cpt == b' ' as c_char {
            (*st).e_cpt = (*st).e_cpt.add(1);
        }

        if *(*st).e_cpt == b'.' as c_char
            && !(*curbuf).b_scanned
            && !skip_source
            && !compl_time_slice_expired
        {
            (*st).ins_buf = curbuf;
            (*st).first_match_pos = *start_match_pos;
            if ctrl_x_mode_normal() && !fuzzy_collect && dec(&raw mut (*st).first_match_pos) < 0 {
                (*st).first_match_pos.lnum = (*(*st).ins_buf).b_ml.ml_line_count;
                (*st).first_match_pos.col = ml_get_len((*st).first_match_pos.lnum);
            }
            (*st).last_match_pos = (*st).first_match_pos;
            compl_type = 0;
            (*st).set_match_pos = true;
        } else if !skip_source
            && !compl_time_slice_expired
            && !vim_strchr(c"buwU".as_ptr(), c_int::from(*(*st).e_cpt as u8)).is_null()
            && {
                (*st).ins_buf = ins_compl_next_buf((*st).ins_buf, c_int::from(*(*st).e_cpt));
                !ptr::eq((*st).ins_buf, curbuf)
            }
        {
            if !(*(*st).ins_buf).b_ml.ml_mfp.is_null() {
                compl_started = true;
                (*st).first_match_pos.col = 0;
                (*st).last_match_pos.col = 0;
                (*st).first_match_pos.lnum = (*(*st).ins_buf).b_ml.ml_line_count + 1;
                (*st).last_match_pos.lnum = 0;
                compl_type = 0;
            } else {
                (*st).found_all = true;
                if (*(*st).ins_buf).b_fname.is_null() {
                    *compl_type_arg = compl_type;
                    return INS_COMPL_CPT_CONT;
                }
                compl_type = CTRL_X_DICTIONARY;
                (*st).dict = (*(*st).ins_buf).b_fname;
                (*st).dict_f = DICT_EXACT;
            }
            if !shortmess(kShmCompletionscan) && !compl_autocomplete {
                let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                let name = if (*(*st).ins_buf).b_fname.is_null() {
                    buf_spname((*st).ins_buf)
                } else if (*(*st).ins_buf).b_sfname.is_null() {
                    (*(*st).ins_buf).b_fname
                } else {
                    (*(*st).ins_buf).b_sfname
                };
                vim_snprintf!(iobuff_ptr, IOSIZE, gettext(c"Scanning: %s".as_ptr()), name);
                msg_progress(
                    iobuff_ptr,
                    c"nvim.completion".as_ptr().cast_mut(),
                    c"running".as_ptr().cast_mut(),
                    -1,
                    HLF_R,
                    false,
                    true,
                    false,
                );
            }
        } else if *(*st).e_cpt == NUL {
            status = INS_COMPL_CPT_END;
        } else {
            if ctrl_x_mode_line_or_eval() {
                // compl_type = -1;
            } else if *(*st).e_cpt == b'F' as c_char || *(*st).e_cpt == b'o' as c_char {
                compl_type = CTRL_X_FUNCTION;
                (*st).func_cb = get_callback_if_cpt_func((*st).e_cpt, cpt_sources_index);
                if (*st).func_cb.is_null() {
                    compl_type = -1;
                }
            } else if !skip_source {
                if *(*st).e_cpt == b'k' as c_char || *(*st).e_cpt == b's' as c_char {
                    if *(*st).e_cpt == b'k' as c_char {
                        compl_type = CTRL_X_DICTIONARY;
                    } else {
                        compl_type = CTRL_X_THESAURUS;
                    }
                    (*st).e_cpt = (*st).e_cpt.add(1);
                    if *(*st).e_cpt != b',' as c_char && *(*st).e_cpt != NUL {
                        (*st).dict = (*st).e_cpt;
                        (*st).dict_f = DICT_FIRST;
                    }
                } else if *(*st).e_cpt == b'i' as c_char {
                    compl_type = CTRL_X_PATH_PATTERNS;
                } else if *(*st).e_cpt == b'd' as c_char {
                    compl_type = CTRL_X_PATH_DEFINES;
                } else if *(*st).e_cpt == b'f' as c_char {
                    compl_type = CTRL_X_BUFNAMES;
                } else if *(*st).e_cpt == b']' as c_char || *(*st).e_cpt == b't' as c_char {
                    compl_type = CTRL_X_TAGS;
                    if !shortmess(kShmCompletionscan) && !compl_autocomplete {
                        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                        vim_snprintf!(
                            iobuff_ptr,
                            IOSIZE,
                            c"%s".as_ptr(),
                            gettext(c"Scanning tags.".as_ptr()),
                        );
                        msg_progress(
                            iobuff_ptr,
                            c"nvim.completion".as_ptr().cast_mut(),
                            c"running".as_ptr().cast_mut(),
                            -1,
                            HLF_R,
                            false,
                            true,
                            false,
                        );
                    }
                }
            }

            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            copy_option_part(&raw mut (*st).e_cpt, iobuff_ptr, IOSIZE, c",".as_ptr());
            *advance_cpt_idx = may_advance_cpt_index((*st).e_cpt);

            (*st).found_all = true;
            if compl_type == -1 {
                status = INS_COMPL_CPT_CONT;
            }
        }

        *compl_type_arg = compl_type;
        status
    }
}

unsafe fn get_next_include_file_completion(compl_type: c_int) {
    unsafe {
        find_pattern_in_path(
            compl_pattern.data,
            compl_direction,
            compl_pattern.size,
            false,
            false,
            if compl_type == CTRL_X_PATH_DEFINES && (compl_cont_status & CONT_SOL) == 0 {
                FIND_DEFINE
            } else {
                FIND_ANY
            },
            1,
            ACTION_EXPAND,
            1,
            MAXLNUM,
            false,
            compl_autocomplete,
        );
    }
}

unsafe fn get_next_dict_tsr_completion(compl_type: c_int, dict: *mut c_char, dict_f: c_int) {
    unsafe {
        if thesaurus_func_complete(compl_type) {
            expand_by_function(compl_type, compl_pattern.data, ptr::null_mut());
        } else {
            let dict_arg = if !dict.is_null() {
                dict
            } else if compl_type == CTRL_X_THESAURUS {
                if *(*curbuf).b_p_tsr == NUL {
                    p_tsr
                } else {
                    (*curbuf).b_p_tsr
                }
            } else if *(*curbuf).b_p_dict == NUL {
                p_dict
            } else {
                (*curbuf).b_p_dict
            };
            ins_compl_dictionaries(
                dict_arg,
                compl_pattern.data,
                if !dict.is_null() { dict_f } else { 0 },
                compl_type == CTRL_X_THESAURUS,
            );
        }
    }
}

unsafe fn get_next_tag_completion() {
    unsafe {
        let save_p_ic = p_ic;
        p_ic = ignorecase(compl_pattern.data);

        g_tag_at_cursor = true;
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let mut num_matches: c_int = 0;
        if find_tags(
            compl_pattern.data,
            &raw mut num_matches,
            &raw mut matches,
            TAG_REGEXP
                | TAG_NAMES
                | TAG_NOIC
                | TAG_INS_COMP
                | if ctrl_x_mode_not_default() {
                    TAG_VERBOSE
                } else {
                    0
                },
            TAG_MANY,
            (*curbuf).b_ffname,
        ) == OK
            && num_matches > 0
        {
            ins_compl_add_matches(num_matches, matches, p_ic);
        }
        g_tag_at_cursor = false;
        p_ic = save_p_ic;
    }
}

unsafe extern "C" fn compare_scores(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let idx_a = *a.cast::<c_int>();
        let idx_b = *b.cast::<c_int>();
        let score_a = *compl_fuzzy_scores.add(idx_a as usize);
        let score_b = *compl_fuzzy_scores.add(idx_b as usize);
        if score_a == score_b {
            if idx_a == idx_b {
                0
            } else if idx_a < idx_b {
                -1
            } else {
                1
            }
        } else if score_a > score_b {
            -1
        } else {
            1
        }
    }
}

unsafe fn ins_compl_longest_insert(prefix: *const c_char) {
    unsafe {
        ins_compl_delete(false);
        ins_compl_insert_bytes(prefix.add(get_compl_len() as usize), -1);
        compl_longest_end_col = (*curwin).w_cursor.col;
        ins_redraw(false);
    }
}

unsafe fn fuzzy_longest_match() {
    unsafe {
        if compl_num_bests == 0 {
            return;
        }

        let nn_compl = (*(*compl_first_match).cp_next).cp_next;
        let more_candidates = !nn_compl.is_null() && !ptr::eq(nn_compl, compl_first_match);

        let mut compl = if ctrl_x_mode_whole_line() {
            compl_first_match
        } else {
            (*compl_first_match).cp_next
        };
        if compl_num_bests == 1 {
            if !more_candidates {
                ins_compl_longest_insert((*compl).cp_str.data);
                compl_num_bests = 0;
            }
            compl_num_bests = 0;
            return;
        }

        if (compl_num_bests as usize) > usize::MAX / size_of::<*mut compl_T>() {
            return;
        }
        compl_best_matches =
            xmalloc((compl_num_bests as usize) * size_of::<*mut compl_T>()).cast::<*mut compl_T>();

        let mut i: c_int = 0;
        while !compl.is_null() && i < compl_num_bests {
            *compl_best_matches.add(i as usize) = compl;
            compl = (*compl).cp_next;
            i += 1;
        }

        let prefix = (**compl_best_matches).cp_str.data;
        let mut prefix_len = (**compl_best_matches).cp_str.size as c_int;

        for idx in 1..(compl_num_bests as usize) {
            let match_str = (**compl_best_matches.add(idx)).cp_str.data;
            let mut prefix_ptr = prefix;
            let mut match_ptr = match_str;
            let mut j: c_int = 0;

            while j < prefix_len && *match_ptr != NUL && *prefix_ptr != NUL {
                if strncmp(prefix_ptr, match_ptr, utfc_ptr2len(prefix_ptr) as usize) != 0 {
                    break;
                }

                mb_ptr_adv(&raw mut prefix_ptr);
                mb_ptr_adv(&raw mut match_ptr);
                j += 1;
            }

            if j > 0 {
                prefix_len = j;
            }
        }

        let leader = ins_compl_leader();
        let leader_len = ins_compl_leader_len();

        if leader_len == 0 || strncmp(prefix, leader, leader_len) == 0 {
            let dup = xmemdupz(prefix.cast(), prefix_len as usize).cast::<c_char>();
            ins_compl_longest_insert(dup);
            xfree(dup.cast());
        }

        xfree(compl_best_matches.cast());
        compl_best_matches = ptr::null_mut();
        compl_num_bests = 0;
    }
}

unsafe fn get_next_filename_completion() {
    unsafe {
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let mut num_matches: c_int = 0;
        let mut leader = ins_compl_leader();
        let mut leader_len = ins_compl_leader_len();
        let mut in_fuzzy_collect = cot_fuzzy() && leader_len > 0;
        let need_collect_bests = in_fuzzy_collect && compl_get_longest;
        let mut max_score: c_int = 0;
        let mut dir = compl_direction;

        if in_fuzzy_collect {
            #[cfg(windows)]
            {
                path_to_slash(leader);
            }
            let last_sep = strrchr(leader, PATHSEP as c_int);
            if last_sep.is_null() {
                api_clear_string(&raw mut compl_pattern);
                compl_pattern = cbuf_to_string(c"*".as_ptr(), 1);
            } else if *last_sep.add(1) == NUL {
                in_fuzzy_collect = false;
            } else {
                let path_len = last_sep.offset_from(leader) as usize + 1;
                let path_with_wildcard = xmalloc(path_len + 2).cast::<c_char>();
                vim_snprintf!(
                    path_with_wildcard,
                    path_len + 2,
                    c"%*.*s*".as_ptr(),
                    path_len as c_int,
                    path_len as c_int,
                    leader,
                );
                api_clear_string(&raw mut compl_pattern);
                compl_pattern.data = path_with_wildcard;
                compl_pattern.size = path_len + 1;

                leader = last_sep.add(1);
                leader_len -= path_len;
            }
        }

        if expand_wildcards(
            1,
            &raw mut compl_pattern.data,
            &raw mut num_matches,
            &raw mut matches,
            EW_FILE | EW_DIR | EW_ADDSLASH | EW_SILENT,
        ) != OK
        {
            return;
        }

        tilde_replace(compl_pattern.data, num_matches, matches);
        #[cfg(windows)]
        {
            if (*(*curbuf).b_p_csl == NUL && p_ssl == 0) || *(*curbuf).b_p_csl == b'b' as c_char {
                for i in 0..(num_matches as usize) {
                    let mut ptr = *matches.add(i);
                    while *ptr != NUL {
                        if *ptr == b'/' as c_char {
                            *ptr = b'\\' as c_char;
                        }
                        ptr = ptr.add(1);
                    }
                }
            }
        }

        if in_fuzzy_collect {
            let mut fuzzy_indices = garray_T {
                ga_len: 0,
                ga_maxlen: 0,
                ga_itemsize: 0,
                ga_growsize: 0,
                ga_data: ptr::null_mut(),
            };
            ga_init(&raw mut fuzzy_indices, size_of::<c_int>() as c_int, 10);
            compl_fuzzy_scores =
                xmalloc(size_of::<c_int>() * (num_matches as usize)).cast::<c_int>();

            for i in 0..num_matches {
                let ptr = *matches.add(i as usize);
                let score = fuzzy_match_str(ptr, leader);
                if score != FUZZY_SCORE_NONE {
                    *ga_append_via_ptr(&raw mut fuzzy_indices, size_of::<c_int>())
                        .cast::<c_int>() = i;
                    *compl_fuzzy_scores.add(i as usize) = score;
                }
            }

            if fuzzy_indices.ga_len > 0 {
                let fuzzy_indices_data = fuzzy_indices.ga_data.cast::<c_int>();
                qsort(
                    fuzzy_indices_data.cast(),
                    fuzzy_indices.ga_len as usize,
                    size_of::<c_int>(),
                    compare_scores,
                );

                for i in 0..(fuzzy_indices.ga_len as usize) {
                    let idx = *fuzzy_indices_data.add(i) as usize;
                    let match_ = *matches.add(idx);
                    let current_score = *compl_fuzzy_scores.add(idx);
                    if ins_compl_add(
                        match_,
                        -1,
                        ptr::null_mut(),
                        ptr::null(),
                        false,
                        ptr::null_mut(),
                        dir,
                        CP_FAST
                            | if p_fic != 0 || p_wic != 0 {
                                CP_ICASE
                            } else {
                                0
                            },
                        false,
                        ptr::null(),
                        current_score,
                        false,
                        ptr::null_mut(),
                    ) == OK
                    {
                        dir = FORWARD;
                    }

                    if need_collect_bests && (i == 0 || current_score == max_score) {
                        compl_num_bests += 1;
                        max_score = current_score;
                    }
                }

                FreeWild(num_matches, matches);
            } else if leader_len > 0 {
                FreeWild(num_matches, matches);
            }

            xfree(compl_fuzzy_scores.cast());
            ga_clear(&raw mut fuzzy_indices);

            if compl_num_bests > 0 && compl_get_longest {
                fuzzy_longest_match();
            }
            return;
        }

        if num_matches > 0 {
            ins_compl_add_matches(num_matches, matches, c_int::from(p_fic != 0 || p_wic != 0));
        }
    }
}

unsafe fn get_next_cmdline_completion() {
    unsafe {
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let mut num_matches: c_int = 0;
        if expand_cmdline(
            &raw mut compl_xp,
            compl_pattern.data,
            compl_pattern.size as c_int,
            &raw mut num_matches,
            &raw mut matches,
        ) == EXPAND_OK
        {
            let mut add_r = OK;
            let mut dir = compl_direction;

            let mut i: usize = 0;
            while (i as c_int) < num_matches && add_r != FAIL {
                let mut cptext: [*mut c_char; CPT_COUNT] = [ptr::null_mut(); CPT_COUNT];

                if !compl_xp.xp_files_abbr.is_null() {
                    cptext[CPT_ABBR] = *compl_xp.xp_files_abbr.add(i);
                }
                if !compl_xp.xp_files_kind.is_null() {
                    cptext[CPT_KIND] = *compl_xp.xp_files_kind.add(i);
                }
                if !compl_xp.xp_files_menu.is_null() {
                    cptext[CPT_MENU] = *compl_xp.xp_files_menu.add(i);
                }
                if !compl_xp.xp_files_info.is_null() {
                    cptext[CPT_INFO] = *compl_xp.xp_files_info.add(i);
                }

                add_r = ins_compl_add(
                    *matches.add(i),
                    -1,
                    ptr::null_mut(),
                    cptext.as_ptr(),
                    false,
                    ptr::null_mut(),
                    dir,
                    CP_FAST,
                    false,
                    ptr::null(),
                    FUZZY_SCORE_NONE,
                    false,
                    ptr::null_mut(),
                );
                if add_r == OK {
                    dir = FORWARD;
                }
                i += 1;
            }
            FreeWild(num_matches, matches);
            free_xp_files_extra(&raw mut compl_xp, num_matches);
        }
    }
}

unsafe fn get_next_spell_completion(lnum: linenr_T) {
    unsafe {
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let num_matches = expand_spelling(lnum, compl_pattern.data, &raw mut matches);
        if num_matches > 0 {
            ins_compl_add_matches(num_matches, matches, p_ic);
        } else {
            xfree(matches.cast());
        }
    }
}

unsafe fn ins_compl_get_next_word_or_line(
    ins_buf: *mut buf_T,
    cur_match_pos: *mut pos_T,
    match_len: *mut c_int,
    cont_s_ipos: *mut bool,
) -> *mut c_char {
    unsafe {
        *match_len = 0;
        let mut ptr = ml_get_buf(ins_buf, (*cur_match_pos).lnum).add((*cur_match_pos).col as usize);
        let mut len = ml_get_buf_len(ins_buf, (*cur_match_pos).lnum) - (*cur_match_pos).col;
        if ctrl_x_mode_line_or_eval() {
            if compl_status_adding() {
                if (*cur_match_pos).lnum >= (*ins_buf).b_ml.ml_line_count {
                    return ptr::null_mut();
                }
                ptr = ml_get_buf(ins_buf, (*cur_match_pos).lnum + 1);
                len = ml_get_buf_len(ins_buf, (*cur_match_pos).lnum + 1);
                if p_paste == 0 {
                    let tmp_ptr = ptr;
                    ptr = skipwhite(tmp_ptr);
                    len -= ptr.offset_from(tmp_ptr) as c_int;
                }
            }
        } else {
            let mut tmp_ptr = ptr;

            if compl_status_adding() && compl_length <= len {
                tmp_ptr = tmp_ptr.add(compl_length as usize);
                if vim_iswordp(tmp_ptr) {
                    return ptr::null_mut();
                }
                tmp_ptr = find_word_start(tmp_ptr);
            }
            tmp_ptr = find_word_end(tmp_ptr);
            len = tmp_ptr.offset_from(ptr) as c_int;

            if compl_status_adding() && len == compl_length {
                if (*cur_match_pos).lnum < (*ins_buf).b_ml.ml_line_count {
                    let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                    strncpy(iobuff_ptr, ptr, len as usize);
                    ptr = ml_get_buf(ins_buf, (*cur_match_pos).lnum + 1);
                    ptr = skipwhite(ptr);
                    tmp_ptr = find_word_start(ptr);
                    tmp_ptr = find_word_end(tmp_ptr);
                    if tmp_ptr > ptr {
                        if *ptr != b')' as c_char
                            && *iobuff_ptr.add((len - 1) as usize) != TAB as c_char
                        {
                            if *iobuff_ptr.add((len - 1) as usize) != b' ' as c_char {
                                *iobuff_ptr.add(len as usize) = b' ' as c_char;
                                len += 1;
                            }
                            if p_js != 0 {
                                let prev = *iobuff_ptr.add((len - 2) as usize);
                                if prev == b'.' as c_char
                                    || prev == b'?' as c_char
                                    || prev == b'!' as c_char
                                {
                                    *iobuff_ptr.add(len as usize) = b' ' as c_char;
                                    len += 1;
                                }
                            }
                        }
                        if tmp_ptr.offset_from(ptr) as usize >= IOSIZE - len as usize {
                            tmp_ptr = ptr.add(IOSIZE - len as usize - 1);
                        }
                        xstrlcpy(iobuff_ptr.add(len as usize), ptr, IOSIZE - len as usize);
                        len += tmp_ptr.offset_from(ptr) as c_int;
                        *cont_s_ipos = true;
                    }
                    *iobuff_ptr.add(len as usize) = NUL;
                    ptr = iobuff_ptr;
                }
                if len == compl_length {
                    return ptr::null_mut();
                }
            }
        }

        *match_len = len;
        ptr
    }
}

unsafe fn get_next_default_completion(
    st: *mut ins_compl_next_state_T,
    start_pos: *mut pos_T,
) -> c_int {
    unsafe {
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut len: c_int = 0;
        let in_fuzzy_collect = !compl_status_adding() && cot_fuzzy() && compl_length > 0;
        let leader = ins_compl_leader();
        let mut score = FUZZY_SCORE_NONE;
        let in_curbuf = ptr::eq((*st).ins_buf, curbuf);

        let save_p_scs = p_scs;
        debug_assert!(!(*st).ins_buf.is_null());
        if (*(*st).ins_buf).b_p_inf != 0 {
            p_scs = 0;
        }

        let save_p_ws = p_ws;
        if !in_curbuf {
            p_ws = 0;
        } else if *(*st).e_cpt == b'.' as c_char {
            p_ws = 1;
        }
        let mut looped_around = false;
        let mut found_new_match: c_int;
        loop {
            let mut cont_s_ipos = false;

            msg_silent += 1;

            if in_fuzzy_collect {
                found_new_match = c_int::from(search_for_fuzzy_match(
                    (*st).ins_buf,
                    (*st).cur_match_pos.cast(),
                    leader,
                    compl_direction,
                    start_pos.cast(),
                    &raw mut len,
                    &raw mut ptr,
                    &raw mut score,
                ));
            } else if ctrl_x_mode_whole_line()
                || ctrl_x_mode_eval()
                || (compl_cont_status & CONT_SOL) != 0
            {
                found_new_match = search_for_exact_line(
                    (*st).ins_buf,
                    (*st).cur_match_pos,
                    compl_direction,
                    compl_pattern.data,
                );
            } else {
                found_new_match = searchit(
                    ptr::null_mut(),
                    (*st).ins_buf,
                    (*st).cur_match_pos,
                    ptr::null_mut(),
                    compl_direction,
                    compl_pattern.data,
                    compl_pattern.size,
                    1,
                    SEARCH_KEEP + SEARCH_NFMSG,
                    RE_LAST,
                    p_magic != 0,
                    ptr::null_mut(),
                );
            }
            msg_silent -= 1;
            if !compl_started || (*st).set_match_pos {
                compl_started = true;
                (*st).first_match_pos = *(*st).cur_match_pos;
                (*st).last_match_pos = *(*st).cur_match_pos;
                (*st).set_match_pos = false;
            } else if (*st).first_match_pos.lnum == (*st).last_match_pos.lnum
                && (*st).first_match_pos.col == (*st).last_match_pos.col
            {
                found_new_match = FAIL;
            } else if compl_dir_forward()
                && ((*st).prev_match_pos.lnum > (*(*st).cur_match_pos).lnum
                    || ((*st).prev_match_pos.lnum == (*(*st).cur_match_pos).lnum
                        && (*st).prev_match_pos.col >= (*(*st).cur_match_pos).col))
            {
                if looped_around {
                    found_new_match = FAIL;
                } else {
                    looped_around = true;
                }
            } else if !compl_dir_forward()
                && ((*st).prev_match_pos.lnum < (*(*st).cur_match_pos).lnum
                    || ((*st).prev_match_pos.lnum == (*(*st).cur_match_pos).lnum
                        && (*st).prev_match_pos.col <= (*(*st).cur_match_pos).col))
            {
                if looped_around {
                    found_new_match = FAIL;
                } else {
                    looped_around = true;
                }
            }
            (*st).prev_match_pos = *(*st).cur_match_pos;
            if found_new_match == FAIL {
                break;
            }

            if compl_status_adding()
                && in_curbuf
                && (*start_pos).lnum == (*(*st).cur_match_pos).lnum
                && (*start_pos).col == (*(*st).cur_match_pos).col
            {
                continue;
            }

            if !in_fuzzy_collect {
                ptr = ins_compl_get_next_word_or_line(
                    (*st).ins_buf,
                    (*st).cur_match_pos,
                    &raw mut len,
                    &raw mut cont_s_ipos,
                );
            }
            if ptr.is_null() || (ins_compl_has_preinsert() && strcmp(ptr, ins_compl_leader()) == 0)
            {
                continue;
            }

            if is_nearest_active() && in_curbuf {
                score = ((*(*st).cur_match_pos).lnum - (*curwin).w_cursor.lnum).abs();
            }

            if ins_compl_add_infercase(
                ptr,
                len,
                p_ic != 0,
                if in_curbuf {
                    ptr::null_mut()
                } else {
                    (*(*st).ins_buf).b_sfname
                },
                0,
                cont_s_ipos,
                score,
            ) != NOTDONE
            {
                if in_fuzzy_collect && score == (*(*compl_first_match).cp_next).cp_score {
                    compl_num_bests += 1;
                }
                found_new_match = OK;
                break;
            }
        }
        p_scs = save_p_scs;
        p_ws = save_p_ws;

        found_new_match
    }
}

unsafe fn get_register_completion() {
    unsafe {
        let mut dir = compl_direction;
        let adding_mode = compl_status_adding();

        for i in 0..(NUM_REGISTERS as c_int) {
            let regname = get_register_name(i);
            if !valid_yank_reg(regname, false) || regname == b'_' as c_int {
                continue;
            }

            let reg = copy_register(regname);

            if (*reg).y_array.is_null() || (*reg).y_size == 0 {
                free_register(reg);
                xfree(reg.cast());
                continue;
            }

            for j in 0..(*reg).y_size {
                let str_ptr = (*(*reg).y_array.add(j)).data;
                if str_ptr.is_null() {
                    continue;
                }

                if adding_mode {
                    let str_len = strlen(str_ptr) as c_int;
                    if str_len == 0 {
                        continue;
                    }

                    let matches = compl_orig_text.data.is_null()
                        || if p_ic != 0 {
                            strnicmp_impl(str_ptr, compl_orig_text.data, compl_orig_text.size) == 0
                        } else {
                            strncmp(str_ptr, compl_orig_text.data, compl_orig_text.size) == 0
                        };
                    if matches
                        && ins_compl_add_infercase(
                            str_ptr,
                            str_len,
                            p_ic != 0,
                            ptr::null_mut(),
                            dir,
                            false,
                            FUZZY_SCORE_NONE,
                        ) == OK
                    {
                        dir = FORWARD;
                    }
                } else {
                    let str_end = str_ptr.add(strlen(str_ptr));
                    let mut p = str_ptr;

                    while p < str_end && *p != NUL {
                        let old_p = p;
                        p = find_word_start(p);
                        if p >= str_end || *p == NUL {
                            break;
                        }

                        let mut word_end = find_word_end(p);
                        if word_end <= p {
                            word_end = p.add(utfc_ptr2len(p) as usize);
                        }
                        if word_end > str_end {
                            word_end = str_end;
                        }

                        let len = word_end.offset_from(p) as c_int;
                        let matches = compl_orig_text.data.is_null()
                            || if p_ic != 0 {
                                strnicmp_impl(p, compl_orig_text.data, compl_orig_text.size) == 0
                            } else {
                                strncmp(p, compl_orig_text.data, compl_orig_text.size) == 0
                            };
                        if len > 0
                            && matches
                            && ins_compl_add_infercase(
                                p,
                                len,
                                p_ic != 0,
                                ptr::null_mut(),
                                dir,
                                false,
                                FUZZY_SCORE_NONE,
                            ) == OK
                        {
                            dir = FORWARD;
                        }

                        p = word_end;
                        if p <= old_p {
                            p = old_p.add(utfc_ptr2len(old_p) as usize);
                        }
                    }
                }
            }

            free_register(reg);
            xfree(reg.cast());
        }
    }
}

unsafe fn get_callback_if_cpt_func(p: *mut c_char, idx: c_int) -> *mut Callback {
    unsafe {
        if *p == b'o' as c_char {
            return &raw mut (*curbuf).b_p_ofu;
        }

        if *p == b'F' as c_char {
            let next = *p.add(1);
            if next != b',' as c_char && next != NUL {
                if (*curbuf).b_p_cpt_cb.is_null() || idx < 0 || idx >= (*curbuf).b_p_cpt_count {
                    return ptr::null_mut();
                }
                let cb = (*curbuf).b_p_cpt_cb.add(idx as usize);
                return if (*cb).cb_type != kCallbackNone {
                    cb
                } else {
                    ptr::null_mut()
                };
            } else {
                return &raw mut (*curbuf).b_p_cfu;
            }
        }

        ptr::null_mut()
    }
}

unsafe fn get_next_completion_match(
    type_: c_int,
    st: *mut ins_compl_next_state_T,
    ini: *mut pos_T,
) -> bool {
    unsafe {
        let mut found_new_match = FAIL;

        match type_ {
            -1 => {}
            CTRL_X_PATH_PATTERNS | CTRL_X_PATH_DEFINES => {
                get_next_include_file_completion(type_);
            }
            CTRL_X_DICTIONARY | CTRL_X_THESAURUS => {
                get_next_dict_tsr_completion(type_, (*st).dict, (*st).dict_f);
                (*st).dict = ptr::null_mut();
            }
            CTRL_X_TAGS => {
                get_next_tag_completion();
            }
            CTRL_X_FILES => {
                get_next_filename_completion();
            }
            CTRL_X_CMDLINE | CTRL_X_CMDLINE_CTRL_X => {
                get_next_cmdline_completion();
            }
            CTRL_X_FUNCTION => {
                if ctrl_x_mode_normal() {
                    get_cpt_func_completion_matches((*st).func_cb);
                } else {
                    expand_by_function(type_, compl_pattern.data, ptr::null_mut());
                }
            }
            CTRL_X_OMNI => {
                expand_by_function(type_, compl_pattern.data, ptr::null_mut());
            }
            CTRL_X_SPELL => {
                get_next_spell_completion((*st).first_match_pos.lnum);
            }
            CTRL_X_BUFNAMES => {
                get_next_bufname_token();
            }
            CTRL_X_REGISTER => {
                get_register_completion();
            }
            _ => {
                found_new_match = get_next_default_completion(st, ini);
                if found_new_match == FAIL && ptr::eq((*st).ins_buf, curbuf) {
                    (*st).found_all = true;
                }
            }
        }

        if type_ != 0 && !ptr::eq(compl_curr_match, compl_old_match) {
            found_new_match = OK;
        }

        found_new_match != FAIL
    }
}

unsafe fn get_next_bufname_token() {
    unsafe {
        let mut b = firstbuf;
        while !b.is_null() {
            if (*b).b_p_bl != 0 && !(*b).b_sfname.is_null() {
                let tail = path_tail((*b).b_sfname);
                if strncmp(tail, compl_orig_text.data, compl_orig_text.size) == 0 {
                    ins_compl_add(
                        tail,
                        strlen(tail) as c_int,
                        ptr::null_mut(),
                        ptr::null(),
                        false,
                        ptr::null_mut(),
                        0,
                        if p_ic != 0 { CP_ICASE } else { 0 },
                        false,
                        ptr::null(),
                        FUZZY_SCORE_NONE,
                        false,
                        ptr::null_mut(),
                    );
                }
            }
            b = (*b).b_next;
        }
    }
}

unsafe fn strip_caret_numbers_in_place(str_ptr: *mut c_char) {
    unsafe {
        if str_ptr.is_null() {
            return;
        }

        let mut read = str_ptr;
        let mut write = str_ptr;

        while *read != NUL {
            if *read == b'^' as c_char {
                let mut p = read.add(1);
                while ascii_isdigit(*p) {
                    p = p.add(1);
                }
                if (*p == b',' as c_char || *p == NUL) && !ptr::eq(p, read.add(1)) {
                    read = p;
                    continue;
                } else {
                    *write = *read;
                    write = write.add(1);
                    read = read.add(1);
                }
            } else {
                *write = *read;
                write = write.add(1);
                read = read.add(1);
            }
        }
        *write = NUL;
    }
}

unsafe fn prepare_cpt_compl_funcs() {
    unsafe {
        let cpt = xstrdup((*curbuf).b_p_cpt);
        strip_caret_numbers_in_place(cpt);

        let mut idx: c_int = 0;
        let mut p = cpt;
        while *p != NUL {
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
            if *p == NUL {
                break;
            }

            let cb = get_callback_if_cpt_func(p, idx);
            if !cb.is_null() {
                let mut startcol: c_int = 0;
                if get_userdefined_compl_info((*curwin).w_cursor.col, cb, &raw mut startcol) == FAIL
                {
                    if startcol == -3 {
                        (*cpt_sources_array.add(idx as usize)).cs_refresh_always = false;
                    } else {
                        startcol = -2;
                    }
                } else if startcol < 0 || startcol > (*curwin).w_cursor.col {
                    startcol = (*curwin).w_cursor.col;
                }
                (*cpt_sources_array.add(idx as usize)).cs_startcol = startcol;
            } else {
                (*cpt_sources_array.add(idx as usize)).cs_startcol = -3;
            }

            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            let _ = copy_option_part(&raw mut p, iobuff_ptr, IOSIZE, c",".as_ptr());
            idx += 1;
        }

        xfree(cpt.cast());
    }
}

unsafe fn compl_source_start_timer(source_idx: c_int) {
    unsafe {
        if compl_autocomplete || p_cto > 0 {
            (*cpt_sources_array.add(source_idx as usize)).compl_start_tv = os_hrtime();
            compl_time_slice_expired = false;
        }
    }
}

unsafe fn advance_cpt_sources_index_safe() -> c_int {
    unsafe {
        if cpt_sources_index >= 0 && cpt_sources_index < cpt_sources_count - 1 {
            cpt_sources_index += 1;
            return OK;
        }
        semsg!(
            gettext((&raw const e_list_index_out_of_range_nr).cast()),
            i64::from(cpt_sources_index),
        );
        FAIL
    }
}

unsafe fn ins_compl_get_exp(ini: *mut pos_T) -> c_int {
    static mut st: ins_compl_next_state_T = unsafe { std::mem::zeroed() };
    static mut st_cleared: bool = false;
    unsafe {
        let mut found_new_match: c_int;
        let mut type_ = ctrl_x_mode;
        let mut may_advance_cpt_idx = false;
        let mut start_pos = *ini;

        debug_assert!(!curbuf.is_null());

        if !compl_started {
            let mut buf = firstbuf;
            while !buf.is_null() {
                (*buf).b_scanned = false;
                buf = (*buf).b_next;
            }
            if !st_cleared {
                st = std::mem::zeroed();
                st_cleared = true;
            }
            st.found_all = false;
            st.ins_buf = curbuf;
            xfree(st.e_cpt_copy.cast());
            st.e_cpt_copy = xstrdup(if (compl_cont_status & CONT_LOCAL) != 0 {
                c".".as_ptr()
            } else {
                (*curbuf).b_p_cpt
            });
            strip_caret_numbers_in_place(st.e_cpt_copy);
            st.e_cpt = st.e_cpt_copy;

            if compl_autocomplete && is_nearest_active() {
                start_pos.lnum = (start_pos.lnum - LOOKBACK_LINE_COUNT).max(1);
                start_pos.col = 0;
            }
            st.first_match_pos = start_pos;
            st.last_match_pos = start_pos;
        } else if !ptr::eq(st.ins_buf, curbuf) && !buf_valid(st.ins_buf) {
            st.ins_buf = curbuf;
        }
        debug_assert!(!st.ins_buf.is_null());

        compl_old_match = compl_curr_match;
        st.cur_match_pos = if compl_dir_forward() {
            &raw mut st.last_match_pos
        } else {
            &raw mut st.first_match_pos
        };

        let normal_mode_strict = ctrl_x_mode_normal()
            && !ctrl_x_mode_line_or_eval()
            && (compl_cont_status & CONT_LOCAL) == 0
            && !cpt_sources_array.is_null();
        if normal_mode_strict {
            cpt_sources_index = 0;
            if compl_autocomplete || p_cto > 0 {
                compl_source_start_timer(0);
                compl_time_slice_expired = false;
                compl_timeout_ms = if compl_autocomplete {
                    (COMPL_INITIAL_TIMEOUT_MS as OptInt).max(p_act) as u64
                } else {
                    p_cto as u64
                };
            }
        }

        loop {
            found_new_match = FAIL;
            st.set_match_pos = false;

            if (ctrl_x_mode_normal() || ctrl_x_mode_line_or_eval())
                && (!compl_started || st.found_all)
            {
                let status = process_next_cpt_value(
                    &raw mut st,
                    &raw mut type_,
                    &raw mut start_pos,
                    cot_fuzzy(),
                    &raw mut may_advance_cpt_idx,
                );
                if status == INS_COMPL_CPT_END {
                    break;
                }
                if status == INS_COMPL_CPT_CONT {
                    if may_advance_cpt_idx {
                        if advance_cpt_sources_index_safe() == 0 {
                            break;
                        }
                        compl_source_start_timer(cpt_sources_index);
                    }
                    continue;
                }
            }

            let mut compl_timeout_save: u64 = 0;
            if normal_mode_strict && type_ == CTRL_X_FUNCTION && (compl_autocomplete || p_cto > 0) {
                compl_timeout_save = compl_timeout_ms;
                compl_timeout_ms = if compl_from_nonkeyword {
                    COMPL_FUNC_TIMEOUT_NON_KW_MS
                } else {
                    COMPL_FUNC_TIMEOUT_MS
                };
            }

            found_new_match = c_int::from(get_next_completion_match(
                type_,
                &raw mut st,
                &raw mut start_pos,
            ));

            if compl_pattern.data.is_null() {
                break;
            }

            if may_advance_cpt_idx {
                if advance_cpt_sources_index_safe() == 0 {
                    break;
                }
                compl_source_start_timer(cpt_sources_index);
            }

            if (ctrl_x_mode_not_default() && !ctrl_x_mode_line_or_eval()) || found_new_match != FAIL
            {
                if got_int {
                    break;
                }
                if type_ != -1 {
                    ins_compl_check_keys(0, false);
                }

                if (ctrl_x_mode_not_default() && !ctrl_x_mode_line_or_eval()) || compl_interrupted {
                    break;
                }
                compl_started = !compl_time_slice_expired;
            } else {
                if buf_valid(st.ins_buf) && (type_ == 0 || type_ == CTRL_X_PATH_PATTERNS) {
                    (*st.ins_buf).b_scanned = true;
                }

                compl_started = false;
            }

            if normal_mode_strict && type_ == CTRL_X_FUNCTION && (compl_autocomplete || p_cto > 0) {
                compl_timeout_ms = compl_timeout_save;
            }

            if !compl_dir_forward() {
                while !(*compl_curr_match).cp_prev.is_null()
                    && !match_at_original_text((*compl_curr_match).cp_prev)
                {
                    compl_curr_match = (*compl_curr_match).cp_prev;
                }
            }
        }
        cpt_sources_index = -1;
        compl_started = true;

        if (ctrl_x_mode_normal() || ctrl_x_mode_line_or_eval()) && *st.e_cpt == NUL {
            found_new_match = FAIL;
        }

        let mut match_count: c_int = -1;
        if found_new_match == FAIL || (ctrl_x_mode_not_default() && !ctrl_x_mode_line_or_eval()) {
            match_count = ins_compl_make_cyclic();
        }

        if cot_fuzzy() && compl_get_longest && compl_num_bests > 0 {
            fuzzy_longest_match();
        }

        if !compl_old_match.is_null() {
            compl_curr_match = if compl_dir_forward() {
                (*compl_old_match).cp_next
            } else {
                (*compl_old_match).cp_prev
            };
            if compl_curr_match.is_null() {
                compl_curr_match = compl_old_match;
            }
        }
        may_trigger_modechanged();

        if match_count > 0 && !ctrl_x_mode_spell() {
            if is_nearest_active() && !ins_compl_has_preinsert() {
                sort_compl_match_list(Some(cp_compare_nearest));
            }

            if cot_fuzzy() && ins_compl_leader_len() > 0 {
                ins_compl_fuzzy_sort();
            }
        }

        match_count
    }
}

unsafe fn ins_compl_update_shown_match() {
    unsafe {
        let _ = get_leader_for_startcol(ptr::null_mut(), true);
        let mut leader = get_leader_for_startcol(compl_shown_match, true);

        while !ins_compl_equal(compl_shown_match, (*leader).data, (*leader).size)
            && !(*compl_shown_match).cp_next.is_null()
            && !is_first_match((*compl_shown_match).cp_next)
        {
            compl_shown_match = (*compl_shown_match).cp_next;
            leader = get_leader_for_startcol(compl_shown_match, true);
        }

        if compl_shows_dir_backward()
            && !ins_compl_equal(compl_shown_match, (*leader).data, (*leader).size)
            && ((*compl_shown_match).cp_next.is_null()
                || is_first_match((*compl_shown_match).cp_next))
        {
            while !ins_compl_equal(compl_shown_match, (*leader).data, (*leader).size)
                && !(*compl_shown_match).cp_prev.is_null()
                && !is_first_match((*compl_shown_match).cp_prev)
            {
                compl_shown_match = (*compl_shown_match).cp_prev;
                leader = get_leader_for_startcol(compl_shown_match, true);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_delete(new_leader: bool) {
    unsafe {
        let mut orig_col: c_int = 0;
        if new_leader {
            let mut orig = compl_orig_text.data;
            let mut leader = ins_compl_leader();
            while *orig != NUL && utf_ptr2char(orig) == utf_ptr2char(leader) {
                leader = leader.add(utf_ptr2len(leader) as usize);
                orig = orig.add(utf_ptr2len(orig) as usize);
            }
            orig_col = orig.offset_from(compl_orig_text.data) as c_int;
        }

        let mut col = compl_col
            + if compl_status_adding() {
                compl_length
            } else {
                orig_col
            };

        if ins_compl_preinsert_effect() {
            col += ins_compl_leader_len() as c_int;
            (*curwin).w_cursor.col = compl_ins_end_col;
        }

        let mut remaining = empty_nvim_string();
        if (*curwin).w_cursor.lnum > compl_lnum {
            if (*curwin).w_cursor.col < get_cursor_line_len() {
                remaining = cbuf_to_string(get_cursor_pos_ptr(), get_cursor_pos_len() as usize);
            }

            while (*curwin).w_cursor.lnum > compl_lnum {
                if ml_delete((*curwin).w_cursor.lnum) == FAIL {
                    if !remaining.data.is_null() {
                        xfree(remaining.data.cast());
                    }
                    return;
                }
                deleted_lines_mark((*curwin).w_cursor.lnum, 1);
                (*curwin).w_cursor.lnum -= 1;
            }
            (*curwin).w_cursor.col = get_cursor_line_len();
        }

        if (*curwin).w_cursor.col > col {
            if stop_arrow() == FAIL {
                if !remaining.data.is_null() {
                    xfree(remaining.data.cast());
                }
                return;
            }
            backspace_until_column(col);
            compl_ins_end_col = (*curwin).w_cursor.col;
        }

        if !remaining.data.is_null() {
            orig_col = (*curwin).w_cursor.col;
            ins_str(remaining.data, remaining.size);
            (*curwin).w_cursor.col = orig_col;
            xfree(remaining.data.cast());
        }

        changed_cline_bef_curs(curwin);
        set_vim_var_dict(VV_COMPLETED_ITEM, tv_dict_alloc_lock(VAR_FIXED));
    }
}

unsafe fn ins_compl_expand_multiple(str_ptr: *mut c_char) {
    unsafe {
        let mut start = str_ptr;
        let mut curr = str_ptr;
        let base_indent = get_indent();
        while *curr != NUL {
            if *curr == b'\n' as c_char {
                if curr > start {
                    ins_char_bytes(start, curr.offset_from(start) as usize);
                }
                open_line(
                    FORWARD,
                    OPENLINE_KEEPTRAIL | OPENLINE_FORCE_INDENT,
                    base_indent,
                    ptr::null_mut(),
                );
                start = curr.add(1);
            }
            curr = curr.add(1);
        }

        if curr > start {
            ins_char_bytes(start, curr.offset_from(start) as usize);
        }

        compl_ins_end_col = (*curwin).w_cursor.col;
    }
}

unsafe fn find_common_prefix(prefix_len: *mut usize, curbuf_only: bool) -> *mut c_char {
    unsafe {
        let is_cpt_completion = !cpt_sources_array.is_null();

        if !is_cpt_completion {
            return ptr::null_mut();
        }

        let match_count = xcalloc(cpt_sources_count as usize, size_of::<c_int>()).cast::<c_int>();

        let _ = get_leader_for_startcol(ptr::null_mut(), true);

        let mut compl = compl_first_match;
        let mut first: *mut c_char = ptr::null_mut();
        let mut len: c_int = -1;
        loop {
            let leader = get_leader_for_startcol(compl, true);

            if ctrl_x_mode_normal()
                && p_inf == 0
                && !compl_orig_text.data.is_null()
                && ignorecase(compl_orig_text.data) == 0
            {
                (*compl).cp_flags &= !CP_ICASE;
            }

            if !match_at_original_text(compl)
                && ((*leader).data.is_null()
                    || ins_compl_equal_sc(compl, (*leader).data, (*leader).size))
            {
                let mut match_limit_exceeded = false;
                let cur_source = (*compl).cp_cpt_source_idx;

                if cur_source != -1 {
                    *match_count.add(cur_source as usize) += 1;
                    let max_matches = (*cpt_sources_array.add(cur_source as usize)).cs_max_matches;
                    if max_matches > 0 && *match_count.add(cur_source as usize) > max_matches {
                        match_limit_exceeded = true;
                    }
                }

                if !match_limit_exceeded
                    && (!curbuf_only
                        || (cur_source != -1
                            && (*cpt_sources_array.add(cur_source as usize)).cs_flag
                                == b'.' as c_char))
                {
                    if first.is_null()
                        && strncmp(
                            ins_compl_leader(),
                            (*compl).cp_str.data,
                            ins_compl_leader_len(),
                        ) == 0
                    {
                        first = (*compl).cp_str.data;
                        len = strlen(first) as c_int;
                    } else if !first.is_null() {
                        let mut j: c_int = 0;
                        let mut s1 = first;
                        let mut s2 = (*compl).cp_str.data;

                        while j < len && *s1 != NUL && *s2 != NUL {
                            let l1 = mb_byte2len(*s1 as u8);
                            let l2 = mb_byte2len(*s2 as u8);
                            if l1 != l2 || memcmp(s1.cast(), s2.cast(), l1 as usize) != 0 {
                                break;
                            }

                            j += l1;
                            mb_ptr_adv(&raw mut s1);
                            mb_ptr_adv(&raw mut s2);
                        }
                        len = j;

                        if len == 0 {
                            break;
                        }
                    }
                }
            }
            compl = (*compl).cp_next;
            if compl.is_null() || is_first_match(compl) {
                break;
            }
        }

        xfree(match_count.cast());

        if len > ins_compl_leader_len() as c_int {
            debug_assert!(!first.is_null());
            if len == strlen(first) as c_int {
                let line = get_cursor_line_ptr();
                let p = line.add((*curwin).w_cursor.col as usize);
                if !p.is_null() && !ascii_iswhite_or_nul(*p) {
                    let end = find_word_end(p);
                    let text_len = end.offset_from(p) as c_int;
                    if text_len > 0
                        && text_len < len - ins_compl_leader_len() as c_int
                        && strncmp(first.add((len - text_len) as usize), p, text_len as usize) == 0
                    {
                        len -= text_len;
                    }
                }
            }
            *prefix_len = len as usize;
            return first;
        }
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_insert(move_cursor: bool, insert_prefix: bool) {
    unsafe {
        let compl_len = get_compl_len();
        let preinsert = ins_compl_has_preinsert();
        let mut cp_str = (*compl_shown_match).cp_str.data;
        let mut cp_str_len = (*compl_shown_match).cp_str.size;
        let leader_len = ins_compl_leader_len();
        let has_multiple = !strchr(cp_str, b'\n' as c_int).is_null();

        if insert_prefix {
            cp_str = find_common_prefix(&raw mut cp_str_len, false);
            if cp_str.is_null() {
                cp_str = find_common_prefix(&raw mut cp_str_len, true);
                if cp_str.is_null() {
                    cp_str = (*compl_shown_match).cp_str.data;
                    cp_str_len = (*compl_shown_match).cp_str.size;
                }
            }
        } else if !cpt_sources_array.is_null() {
            let cpt_idx = (*compl_shown_match).cp_cpt_source_idx;
            if cpt_idx >= 0 && compl_col >= 0 {
                let startcol = (*cpt_sources_array.add(cpt_idx as usize)).cs_startcol;
                if startcol >= 0 && startcol < compl_col {
                    let skip = compl_col - startcol;
                    if (skip as usize) <= cp_str_len {
                        cp_str_len -= skip as usize;
                        cp_str = cp_str.add(skip as usize);
                    }
                }
            }
        }

        if compl_len < cp_str_len as c_int {
            if has_multiple {
                ins_compl_expand_multiple(cp_str.add(compl_len as usize));
            } else {
                ins_compl_insert_bytes(
                    cp_str.add(compl_len as usize),
                    if insert_prefix {
                        (cp_str_len as c_int) - compl_len
                    } else {
                        -1
                    },
                );
                if (preinsert || insert_prefix) && move_cursor {
                    (*curwin).w_cursor.col -= (cp_str_len - leader_len) as colnr_T;
                }
            }
        }
        compl_used_match =
            !(match_at_original_text(compl_shown_match) || (preinsert && !insert_prefix));

        let dict = ins_compl_dict_alloc(compl_shown_match);
        set_vim_var_dict(VV_COMPLETED_ITEM, dict);
        compl_hi_on_autocompl_longest = insert_prefix && move_cursor;
    }
}

unsafe fn ins_compl_show_filename() {
    unsafe {
        let lead = gettext(c"match in file".as_ptr());

        let mut space =
            (if ui_has(kUIMessages) { Columns } else { sc_col }) - vim_strsize(lead) - 2;
        if space <= 0 {
            return;
        }

        let mut s = (*compl_shown_match).cp_fname;
        let mut e = s;
        while *e != NUL {
            space -= ptr2cells(e);
            while space < 0 {
                space += ptr2cells(s);
                mb_ptr_adv(&raw mut s);
            }
            mb_ptr_adv(&raw mut e);
        }
        if !compl_autocomplete {
            msg_hist_off = true;
            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                c"%s %s%s".as_ptr(),
                lead,
                if s > (*compl_shown_match).cp_fname {
                    c"<".as_ptr()
                } else {
                    c"".as_ptr()
                },
                s,
            );
            msg(iobuff_ptr, 0);
            msg_hist_off = false;
            redraw_cmdline = false;
        }
    }
}

unsafe fn find_next_match_in_menu() -> *mut compl_T {
    unsafe {
        let is_forward = compl_shows_dir_forward();
        let mut match_ = compl_shown_match;

        loop {
            match_ = if is_forward {
                (*match_).cp_next
            } else {
                (*match_).cp_prev
            };
            if (*match_).cp_next.is_null()
                || (*match_).cp_in_match_array
                || match_at_original_text(match_)
            {
                break;
            }
        }
        match_
    }
}

unsafe fn find_next_completion_match(
    allow_get_expansion: bool,
    mut todo: c_int,
    advance: bool,
    num_matches: *mut c_int,
) -> c_int {
    unsafe {
        let mut found_end: bool;
        let mut found_compl: *mut compl_T = ptr::null_mut();
        let cur_cot_flags = get_cot_flags();
        let compl_no_select = (cur_cot_flags & kOptCotFlagNoselect) != 0
            || (compl_autocomplete && !ins_compl_has_preinsert());

        loop {
            todo -= 1;
            if todo < 0 {
                break;
            }
            if compl_shows_dir_forward() && !(*compl_shown_match).cp_next.is_null() {
                if !compl_match_array.is_null() {
                    compl_shown_match = find_next_match_in_menu();
                } else {
                    compl_shown_match = (*compl_shown_match).cp_next;
                }
                found_end = !compl_first_match.is_null()
                    && (is_first_match((*compl_shown_match).cp_next)
                        || is_first_match(compl_shown_match));
            } else if compl_shows_dir_backward() && !(*compl_shown_match).cp_prev.is_null() {
                found_end = is_first_match(compl_shown_match);
                if !compl_match_array.is_null() {
                    compl_shown_match = find_next_match_in_menu();
                } else {
                    compl_shown_match = (*compl_shown_match).cp_prev;
                }
                found_end |= is_first_match(compl_shown_match);
            } else {
                if !allow_get_expansion {
                    if advance {
                        if compl_shows_dir_backward() {
                            compl_pending -= todo + 1;
                        } else {
                            compl_pending += todo + 1;
                        }
                    }
                    return -1;
                }

                if !compl_no_select && advance {
                    if compl_shows_dir_backward() {
                        compl_pending -= 1;
                    } else {
                        compl_pending += 1;
                    }
                }

                *num_matches = ins_compl_get_exp(&raw mut compl_startpos);

                while compl_pending != 0 && compl_direction == compl_shows_dir && advance {
                    if compl_pending > 0 && !(*compl_shown_match).cp_next.is_null() {
                        compl_shown_match = (*compl_shown_match).cp_next;
                        compl_pending -= 1;
                    } else if compl_pending < 0 && !(*compl_shown_match).cp_prev.is_null() {
                        compl_shown_match = (*compl_shown_match).cp_prev;
                        compl_pending += 1;
                    } else {
                        break;
                    }
                }
                found_end = false;
            }

            let leader = get_leader_for_startcol(compl_shown_match, false);

            if !match_at_original_text(compl_shown_match)
                && !(*leader).data.is_null()
                && !ins_compl_equal(compl_shown_match, (*leader).data, (*leader).size)
                && !(cot_fuzzy() && (*compl_shown_match).cp_score != FUZZY_SCORE_NONE)
            {
                todo += 1;
            } else {
                found_compl = compl_shown_match;
            }

            if found_end {
                if !found_compl.is_null() {
                    compl_shown_match = found_compl;
                    break;
                }
                todo = 1;
            }
        }

        OK
    }
}

unsafe fn ins_compl_next(allow_get_expansion: bool, count: c_int, insert_match: bool) -> c_int {
    unsafe {
        let mut num_matches: c_int = -1;
        let todo = count;
        let started = compl_started;
        let orig_curbuf = curbuf;
        let cur_cot_flags = get_cot_flags();
        let compl_no_insert = (cur_cot_flags & kOptCotFlagNoinsert) != 0
            || (compl_autocomplete && !ins_compl_has_preinsert());
        let compl_preinsert = ins_compl_has_preinsert();

        if compl_shown_match.is_null() {
            return -1;
        }

        if !compl_leader.data.is_null()
            && !match_at_original_text(compl_shown_match)
            && !cot_fuzzy()
        {
            ins_compl_update_shown_match();
        }

        if allow_get_expansion && insert_match && (!compl_get_longest || compl_used_match) {
            ins_compl_delete(false);
        }

        let mut advance = count != 1 || !allow_get_expansion || !compl_get_longest;

        if compl_restarting {
            advance = false;
            compl_restarting = false;
        }

        if find_next_completion_match(allow_get_expansion, todo, advance, &raw mut num_matches)
            == -1
        {
            return -1;
        }

        if !ptr::eq(curbuf, orig_curbuf) {
            return -1;
        }

        if !started && ins_compl_preinsert_longest() {
            ins_compl_insert(true, true);
        } else if compl_no_insert && !started && !compl_preinsert {
            ins_compl_insert_bytes(compl_orig_text.data.add(get_compl_len() as usize), -1);
            compl_used_match = false;
            restore_orig_extmarks();
        } else if insert_match {
            if !compl_get_longest || compl_used_match {
                let preinsert_longest =
                    ins_compl_preinsert_longest() && match_at_original_text(compl_shown_match);
                ins_compl_insert(compl_preinsert || preinsert_longest, preinsert_longest);
            } else {
                debug_assert!(!compl_leader.data.is_null());
                ins_compl_insert_bytes(compl_leader.data.add(get_compl_len() as usize), -1);
            }
            if strequal((*compl_shown_match).cp_str.data, compl_orig_text.data) {
                restore_orig_extmarks();
            }
        } else {
            compl_used_match = false;
        }

        if !allow_get_expansion {
            update_screen();
            ins_compl_show_pum();
            ins_compl_delete(false);
        }

        if compl_no_insert && !started && !match_at_original_text(compl_shown_match) {
            compl_enter_selects = true;
        } else {
            compl_enter_selects = !insert_match && !compl_match_array.is_null();
        }

        if !(*compl_shown_match).cp_fname.is_null() {
            ins_compl_show_filename();
        }

        num_matches
    }
}

unsafe fn check_elapsed_time() {
    unsafe {
        let start_tv = (*cpt_sources_array.add(cpt_sources_index as usize)).compl_start_tv;
        let elapsed_ms = (os_hrtime() - start_tv) / 1_000_000;

        if elapsed_ms > compl_timeout_ms {
            compl_time_slice_expired = true;
            decay_compl_timeout();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_check_keys(frequency: c_int, in_compl_func: bool) {
    static mut count: c_int = 0;
    unsafe {
        if !in_compl_func && (using_script() != 0 || ex_normal_busy != 0) {
            return;
        }

        count += 1;
        if count < frequency {
            return;
        }
        count = 0;

        let mut c = vpeekc_any();
        if c != NUL as c_int && !test_disable_char_avail {
            if vim_is_ctrl_x_key(c) && c != Ctrl_X && c != Ctrl_R {
                c = safe_vgetc();
                compl_shows_dir = ins_compl_key2dir(c);
                let _ = ins_compl_next(false, ins_compl_key2count(c), c != K_UP && c != K_DOWN);
            } else {
                c = safe_vgetc();
                if c != K_IGNORE {
                    if c != Ctrl_R && (KeyTyped || (in_compl_func && reg_executing == 0)) {
                        compl_interrupted = true;
                    }

                    vungetc(c);
                }
            }
        } else {
            let normal_mode_strict = ctrl_x_mode_normal()
                && !ctrl_x_mode_line_or_eval()
                && (compl_cont_status & CONT_LOCAL) == 0
                && !cpt_sources_array.is_null()
                && cpt_sources_index >= 0;
            if normal_mode_strict && (compl_autocomplete || p_cto > 0) {
                check_elapsed_time();
            }
        }

        if compl_pending != 0
            && !got_int
            && (cot_flags & (kOptCotFlagNoinsert | kOptCotFlagFuzzy)) == 0
            && (!compl_autocomplete || ins_compl_has_preinsert())
        {
            let todo = compl_pending.abs();
            compl_pending = 0;
            let _ = ins_compl_next(false, todo, true);
        }
    }
}

unsafe fn ins_compl_key2dir(c: c_int) -> Direction {
    unsafe {
        if c == K_EVENT || c == K_COMMAND || c == K_LUA {
            return if pum_want.item < compl_selected_item {
                BACKWARD
            } else {
                FORWARD
            };
        }
        if c == Ctrl_P || c == Ctrl_L || c == K_PAGEUP || c == K_KPAGEUP || c == K_S_UP || c == K_UP
        {
            return BACKWARD;
        }
        FORWARD
    }
}

unsafe fn ins_compl_pum_key(c: c_int) -> bool {
    unsafe {
        pum_visible()
            && (c == K_PAGEUP
                || c == K_KPAGEUP
                || c == K_S_UP
                || c == K_PAGEDOWN
                || c == K_KPAGEDOWN
                || c == K_S_DOWN
                || c == K_UP
                || c == K_DOWN)
    }
}

unsafe fn ins_compl_key2count(c: c_int) -> c_int {
    unsafe {
        if c == K_EVENT || c == K_COMMAND || c == K_LUA {
            return (pum_want.item - compl_selected_item).abs();
        }

        if ins_compl_pum_key(c) && c != K_UP && c != K_DOWN {
            let mut h = pum_get_height();
            if h > 3 {
                h -= 2;
            }
            return h;
        }
        1
    }
}

unsafe fn ins_compl_use_match(c: c_int) -> bool {
    unsafe {
        match c {
            K_UP | K_DOWN | K_PAGEDOWN | K_KPAGEDOWN | K_S_DOWN | K_PAGEUP | K_KPAGEUP | K_S_UP => {
                false
            }
            K_EVENT | K_COMMAND | K_LUA => pum_want.active && pum_want.insert,
            _ => true,
        }
    }
}

unsafe fn get_normal_compl_info(
    line: *mut c_char,
    mut startcol: c_int,
    curs_col: colnr_T,
) -> c_int {
    unsafe {
        if (compl_cont_status & CONT_SOL) != 0 || ctrl_x_mode_path_defines() {
            if !compl_status_adding() {
                while {
                    startcol -= 1;
                    startcol >= 0 && vim_isIDc(c_int::from(*line.add(startcol as usize) as u8))
                } {}
                startcol += 1;
                compl_col += startcol;
                compl_length = curs_col - startcol;
            }
            if p_ic != 0 {
                compl_pattern = cstr_as_string(str_foldcase(
                    line.add(compl_col as usize),
                    compl_length,
                    ptr::null_mut(),
                    0,
                ));
            } else {
                compl_pattern = cbuf_to_string(line.add(compl_col as usize), compl_length as usize);
            }
        } else if compl_status_adding() {
            let mut prefix = c"\\<".as_ptr();
            let mut prefixlen: usize = 2;

            if !vim_iswordp(line.add(compl_col as usize))
                || (compl_col > 0 && vim_iswordp(mb_prevptr(line, line.add(compl_col as usize))))
            {
                prefix = c"".as_ptr();
                prefixlen = 0;
            }

            let n = quote_meta(ptr::null_mut(), line.add(compl_col as usize), compl_length)
                as usize
                + prefixlen;
            compl_pattern.data = xmalloc(n).cast::<c_char>();
            strcpy(compl_pattern.data, prefix);
            quote_meta(
                compl_pattern.data.add(prefixlen),
                line.add(compl_col as usize),
                compl_length,
            );
            compl_pattern.size = n - 1;
        } else if {
            startcol -= 1;
            startcol < 0 || !vim_iswordp(mb_prevptr(line, line.add((startcol + 1) as usize)))
        } {
            compl_pattern = cbuf_to_string(c"\\<\\k\\k".as_ptr(), 6);
            compl_col += curs_col;
            compl_length = 0;
            compl_from_nonkeyword = true;
        } else {
            startcol -= utf_head_off(line, line.add(startcol as usize));
            let base_class = mb_get_class(line.add(startcol as usize));
            while {
                startcol -= 1;
                startcol >= 0
            } {
                let head_off = utf_head_off(line, line.add(startcol as usize));
                if base_class != mb_get_class(line.add((startcol - head_off) as usize)) {
                    break;
                }
                startcol -= head_off;
            }

            startcol += 1;
            compl_col += startcol;
            compl_length = curs_col - startcol;
            if compl_length == 1 {
                compl_pattern.data = xmalloc(7).cast::<c_char>();
                strcpy(compl_pattern.data, c"\\<".as_ptr());
                quote_meta(compl_pattern.data.add(2), line.add(compl_col as usize), 1);
                strcat(compl_pattern.data, c"\\k".as_ptr());
                compl_pattern.size = strlen(compl_pattern.data);
            } else {
                let n = quote_meta(ptr::null_mut(), line.add(compl_col as usize), compl_length)
                    as usize
                    + 2;
                compl_pattern.data = xmalloc(n).cast::<c_char>();
                strcpy(compl_pattern.data, c"\\<".as_ptr());
                quote_meta(
                    compl_pattern.data.add(2),
                    line.add(compl_col as usize),
                    compl_length,
                );
                compl_pattern.size = n - 1;
            }
        }

        if ctrl_x_mode_normal() && (compl_cont_status & CONT_LOCAL) == 0 {
            setup_cpt_sources();
            prepare_cpt_compl_funcs();
        }

        OK
    }
}

unsafe fn get_wholeline_compl_info(line: *mut c_char, curs_col: colnr_T) -> c_int {
    unsafe {
        compl_col = getwhitecols(line) as colnr_T;
        compl_length = curs_col - compl_col;
        if compl_length < 0 {
            compl_length = 0;
        }
        if p_ic != 0 {
            compl_pattern = cstr_as_string(str_foldcase(
                line.add(compl_col as usize),
                compl_length,
                ptr::null_mut(),
                0,
            ));
        } else {
            compl_pattern = cbuf_to_string(line.add(compl_col as usize), compl_length as usize);
        }

        OK
    }
}

unsafe fn get_filename_compl_info(
    line: *mut c_char,
    mut startcol: c_int,
    curs_col: colnr_T,
) -> c_int {
    unsafe {
        if startcol > 0 {
            let mut p = line.add(startcol as usize);

            mb_ptr_back(line, &raw mut p);
            while p > line && vim_isfilec(utf_ptr2char(p)) {
                mb_ptr_back(line, &raw mut p);
            }
            let mut p_is_filec = false;
            #[cfg(windows)]
            {
                if p > line
                    && path_has_drive_letter(
                        p.sub(1),
                        line.add(startcol as usize).offset_from(p.sub(1)) as usize,
                    )
                {
                    p = p.sub(if ptr::eq(p, line.add(1)) { 1 } else { 2 });
                    p_is_filec = true;
                }
            }
            p_is_filec = p_is_filec || vim_isfilec(utf_ptr2char(p));

            if ptr::eq(p, line) && p_is_filec {
                startcol = 0;
            } else {
                startcol = p.offset_from(line) as c_int + 1;
            }
        }

        compl_col += startcol;
        compl_length = curs_col - startcol;
        compl_pattern = cstr_as_string(addstar(
            line.add(compl_col as usize),
            compl_length as usize,
            EXPAND_FILES,
        ));

        OK
    }
}

unsafe fn get_cmdline_compl_info(line: *mut c_char, curs_col: colnr_T) -> c_int {
    unsafe {
        compl_pattern = cbuf_to_string(line, curs_col as usize);
        set_cmd_context(
            &raw mut compl_xp,
            compl_pattern.data,
            compl_pattern.size as c_int,
            curs_col,
            0,
        );
        if compl_xp.xp_context == EXPAND_LUA {
            nlua_expand_pat(&raw mut compl_xp);
        }
        if compl_xp.xp_context == EXPAND_UNSUCCESSFUL || compl_xp.xp_context == EXPAND_NOTHING {
            compl_col = curs_col;
        } else {
            compl_col = compl_xp.xp_pattern.offset_from(compl_pattern.data) as colnr_T;
        }
        compl_length = curs_col - compl_col;

        OK
    }
}

unsafe fn set_compl_globals(mut startcol: c_int, curs_col: colnr_T, is_cpt_compl: bool) {
    unsafe {
        if is_cpt_compl {
            api_clear_string(&raw mut cpt_compl_pattern);
            if startcol < compl_col {
                prepend_startcol_text(
                    &raw mut cpt_compl_pattern,
                    &raw const compl_orig_text,
                    startcol,
                );
            } else {
                cpt_compl_pattern = copy_string(compl_orig_text, ptr::null_mut());
            }
        } else {
            if startcol < 0 || startcol > curs_col {
                startcol = curs_col;
            }

            let line = ml_get((*curwin).w_cursor.lnum);
            let len = curs_col - startcol;

            compl_pattern = cbuf_to_string(line.add(startcol as usize), len as usize);
            compl_col = startcol;
            compl_length = len;
        }
    }
}

unsafe fn get_userdefined_compl_info(
    curs_col: colnr_T,
    mut cb: *mut Callback,
    startcol: *mut c_int,
) -> c_int {
    unsafe {
        let save_State = State;

        let is_cpt_function = !cb.is_null();
        let use_sandbox =
            is_cpt_function && was_set_insecurely(curwin, kOptComplete, OPT_LOCAL) != 0;
        if !is_cpt_function {
            cb = get_insert_callback(ctrl_x_mode);
            if (*cb).cb_type == kCallbackNone {
                semsg!(
                    gettext((&raw const e_notset).cast()),
                    if ctrl_x_mode_function() {
                        c"completefunc".as_ptr()
                    } else {
                        c"omnifunc".as_ptr()
                    },
                );
                return FAIL;
            }
        }

        let mut args: [typval_T; 3] = std::mem::zeroed();
        args[0].v_type = VAR_NUMBER;
        args[1].v_type = VAR_STRING;
        args[2].v_type = VAR_UNKNOWN;
        args[0].vval.v_number = 1;
        args[1].vval.v_string = c"".as_ptr().cast_mut();

        let pos = (*curwin).w_cursor;
        textlock += 1;
        if use_sandbox {
            sandbox += 1;
        }
        let col = callback_call_retnr(cb, 2, args.as_mut_ptr()) as colnr_T;
        if use_sandbox {
            sandbox -= 1;
        }
        textlock -= 1;

        State = save_State;
        (*curwin).w_cursor = pos;
        check_cursor(curwin);
        validate_cursor(curwin);
        if !equalpos((*curwin).w_cursor, pos) {
            emsg(gettext(E_COMPLDEL.as_ptr()));
            return FAIL;
        }

        if !startcol.is_null() {
            *startcol = col;
        }

        if col == -2 || aborting() {
            return FAIL;
        }

        if col == -3 {
            if is_cpt_function {
                return FAIL;
            }
            ctrl_x_mode = CTRL_X_NORMAL;
            edit_submode = ptr::null();
            if !shortmess(kShmCompletionmenu) {
                msg_clr_cmdline();
            }
            return FAIL;
        }

        compl_opt_refresh_always = false;

        if !is_cpt_function {
            set_compl_globals(col, curs_col, false);
        }
        OK
    }
}

unsafe fn get_spell_compl_info(startcol: c_int, curs_col: colnr_T) -> c_int {
    unsafe {
        if spell_bad_len > 0 {
            debug_assert!(spell_bad_len <= c_int::MAX as usize);
            compl_col = curs_col - spell_bad_len as c_int;
        } else {
            compl_col = spell_word_start(startcol);
        }
        if compl_col >= startcol {
            compl_length = 0;
            compl_col = curs_col;
        } else {
            spell_expand_check_cap(compl_col);
            compl_length = curs_col - compl_col;
        }
        let line = ml_get((*curwin).w_cursor.lnum);
        compl_pattern = cbuf_to_string(line.add(compl_col as usize), compl_length as usize);

        OK
    }
}

unsafe fn compl_get_info(
    line: *mut c_char,
    startcol: c_int,
    curs_col: colnr_T,
    line_invalid: *mut bool,
) -> c_int {
    unsafe {
        if ctrl_x_mode_normal()
            || ctrl_x_mode_register()
            || ((ctrl_x_mode & CTRL_X_WANT_IDENT) != 0 && !thesaurus_func_complete(ctrl_x_mode))
        {
            if get_normal_compl_info(line, startcol, curs_col) != OK {
                return FAIL;
            }
            *line_invalid = true;
        } else if ctrl_x_mode_line_or_eval() {
            return get_wholeline_compl_info(line, curs_col);
        } else if ctrl_x_mode_files() {
            return get_filename_compl_info(line, startcol, curs_col);
        } else if ctrl_x_mode == CTRL_X_CMDLINE {
            return get_cmdline_compl_info(line, curs_col);
        } else if ctrl_x_mode_function()
            || ctrl_x_mode_omni()
            || thesaurus_func_complete(ctrl_x_mode)
        {
            if get_userdefined_compl_info(curs_col, ptr::null_mut(), ptr::null_mut()) != OK {
                return FAIL;
            }
            *line_invalid = true;
        } else if ctrl_x_mode_spell() {
            if get_spell_compl_info(startcol, curs_col) == FAIL {
                return FAIL;
            }
            *line_invalid = true;
        } else {
            internal_error(c"ins_complete()".as_ptr());
            return FAIL;
        }

        OK
    }
}

unsafe fn ins_compl_continue_search(line: *mut c_char) {
    unsafe {
        compl_cont_status &= !CONT_INTRPT;
        if ctrl_x_mode_normal() || ctrl_x_mode_path_patterns() || ctrl_x_mode_path_defines() {
            if compl_startpos.lnum != (*curwin).w_cursor.lnum {
                compl_col = getwhitecols(line) as colnr_T;
                compl_startpos.col = compl_col;
                compl_startpos.lnum = (*curwin).w_cursor.lnum;
                compl_cont_status &= !CONT_SOL;
            } else {
                if (compl_cont_status & CONT_S_IPOS) != 0 {
                    compl_cont_status |= CONT_SOL;
                    let p = skipwhite(line.add((compl_length + compl_startpos.col) as usize));
                    compl_startpos.col = p.offset_from(line) as colnr_T;
                }
                compl_col = compl_startpos.col;
            }
            compl_length = (*curwin).w_cursor.col - compl_col;
            if compl_length > (IOSIZE as c_int - MIN_SPACE) {
                compl_cont_status &= !CONT_SOL;
                compl_length = IOSIZE as c_int - MIN_SPACE;
                compl_col = (*curwin).w_cursor.col - compl_length;
            }
            compl_cont_status |= CONT_ADDING | CONT_N_ADDS;
            if compl_length < 1 {
                compl_cont_status &= CONT_LOCAL;
            }
        } else if ctrl_x_mode_line_or_eval() || ctrl_x_mode_register() {
            compl_cont_status = CONT_ADDING | CONT_N_ADDS;
        } else {
            compl_cont_status = 0;
        }
    }
}

unsafe fn ins_compl_start() -> c_int {
    unsafe {
        let save_did_ai = Ins.did_ai;

        Ins.did_ai = false;
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;
        if stop_arrow() == FAIL {
            Ins.did_ai = save_did_ai;
            return FAIL;
        }

        let mut line = ml_get((*curwin).w_cursor.lnum);
        let curs_col = (*curwin).w_cursor.col;
        compl_pending = 0;
        compl_lnum = (*curwin).w_cursor.lnum;

        if (compl_cont_status & CONT_INTRPT) == CONT_INTRPT && compl_cont_mode == ctrl_x_mode {
            ins_compl_continue_search(line);
        } else {
            compl_cont_status &= CONT_LOCAL;
        }

        let mut startcol: c_int = 0;
        if !compl_status_adding() {
            compl_cont_mode = ctrl_x_mode;
            if ctrl_x_mode_not_default() {
                compl_cont_status = 0;
            }
            compl_cont_status |= CONT_N_ADDS;
            compl_startpos = (*curwin).w_cursor;
            startcol = curs_col;
            compl_col = 0;
        }

        let mut line_invalid = false;
        if compl_get_info(line, startcol, curs_col, &raw mut line_invalid) == FAIL {
            if ctrl_x_mode_function() || ctrl_x_mode_omni() || thesaurus_func_complete(ctrl_x_mode)
            {
                Ins.did_ai = save_did_ai;
            }
            return FAIL;
        }
        if line_invalid {
            line = ml_get((*curwin).w_cursor.lnum);
        }

        if compl_status_adding() {
            if !shortmess(kShmCompletionmenu) {
                edit_submode_pre = gettext(c" Adding".as_ptr());
            }
            if ctrl_x_mode_line_or_eval() {
                let old = (*curbuf).b_p_com;
                (*curbuf).b_p_com = c"".as_ptr().cast_mut();
                compl_startpos.lnum = (*curwin).w_cursor.lnum;
                compl_startpos.col = compl_col;
                ins_eol(b'\r' as c_int);
                (*curbuf).b_p_com = old;
                compl_length = 0;
                compl_col = (*curwin).w_cursor.col;
                compl_lnum = (*curwin).w_cursor.lnum;
            }
        } else {
            edit_submode_pre = ptr::null();
            compl_startpos.col = compl_col;
        }

        if !shortmess(kShmCompletionmenu) && !compl_autocomplete {
            if (compl_cont_status & CONT_LOCAL) != 0 {
                edit_submode = gettext(ctrl_x_msg(CTRL_X_LOCAL_MSG));
            } else {
                edit_submode = gettext(ctrl_x_msg(ctrl_x_mode));
            }
        }

        ins_compl_fixRedoBufForLeader(ptr::null());

        api_clear_string(&raw mut compl_orig_text);
        kv_destroy_extmarks(&raw mut compl_orig_extmarks);
        compl_orig_text = cbuf_to_string(line.add(compl_col as usize), compl_length as usize);
        save_orig_extmarks();
        let mut flags = CP_ORIGINAL_TEXT;
        if p_ic != 0 {
            flags |= CP_ICASE;
        }
        if ins_compl_add(
            compl_orig_text.data,
            compl_orig_text.size as c_int,
            ptr::null_mut(),
            ptr::null(),
            false,
            ptr::null_mut(),
            0,
            flags,
            false,
            ptr::null(),
            FUZZY_SCORE_NONE,
            false,
            ptr::null_mut(),
        ) != OK
        {
            api_clear_string(&raw mut compl_pattern);
            api_clear_string(&raw mut compl_orig_text);
            kv_destroy_extmarks(&raw mut compl_orig_extmarks);
            Ins.did_ai = save_did_ai;
            return FAIL;
        }

        if !shortmess(kShmCompletionmenu) && !compl_autocomplete {
            edit_submode_extra = gettext(c"-- Searching...".as_ptr());
            edit_submode_highl = HLF_COUNT;
            showmode();
            edit_submode_extra = ptr::null_mut();
            ui_flush();
        }

        Ins.did_ai = save_did_ai;
        OK
    }
}

unsafe fn ins_compl_show_statusmsg() {
    static mut match_ref: [c_char; 81] = [0; 81];
    unsafe {
        if is_first_match((*compl_first_match).cp_next) {
            edit_submode_extra = if compl_status_adding() && compl_length > 1 {
                gettext(c"Hit end of paragraph".as_ptr())
            } else {
                gettext(c"Pattern not found".as_ptr())
            };
            edit_submode_highl = HLF_E;
        }

        if edit_submode_extra.is_null() {
            if match_at_original_text(compl_curr_match) {
                edit_submode_extra = gettext(c"Back at original".as_ptr());
                edit_submode_highl = HLF_W;
            } else if (compl_cont_status & CONT_S_IPOS) != 0 {
                edit_submode_extra = gettext(c"Word from other line".as_ptr());
                edit_submode_highl = HLF_COUNT;
            } else if ptr::eq((*compl_curr_match).cp_next, (*compl_curr_match).cp_prev) {
                edit_submode_extra = gettext(c"The only match".as_ptr());
                edit_submode_highl = HLF_COUNT;
                (*compl_curr_match).cp_number = 1;
            } else {
                if (*compl_curr_match).cp_number == -1 {
                    ins_compl_update_sequence_numbers();
                }

                if (*compl_curr_match).cp_number != -1 {
                    let match_ref_ptr = (&raw mut match_ref).cast::<c_char>();
                    if compl_matches > 0 {
                        vim_snprintf!(
                            match_ref_ptr,
                            81,
                            gettext(c"match %d of %d".as_ptr()),
                            (*compl_curr_match).cp_number,
                            compl_matches,
                        );
                    } else {
                        vim_snprintf!(
                            match_ref_ptr,
                            81,
                            gettext(c"match %d".as_ptr()),
                            (*compl_curr_match).cp_number,
                        );
                    }
                    edit_submode_extra = match_ref_ptr;
                    edit_submode_highl = HLF_R;
                    if dollar_vcol >= 0 {
                        curs_columns(curwin, 0);
                    }
                }
            }
        }

        redraw_mode = true;
        if !shortmess(kShmCompletionmenu) {
            if !edit_submode_extra.is_null() {
                if p_smd == 0 {
                    msg_hist_off = true;
                    msg_ext_set_kind(c"completion".as_ptr());
                    msg(
                        edit_submode_extra,
                        if edit_submode_highl < HLF_COUNT {
                            edit_submode_highl + 1
                        } else {
                            0
                        },
                    );
                    msg_hist_off = false;
                }
            } else {
                msg_clr_cmdline();
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_complete(c: c_int, enable_pum: bool) -> c_int {
    unsafe {
        compl_direction = ins_compl_key2dir(c);
        let insert_match = ins_compl_use_match(c);

        if !compl_started {
            compl_autostarted = compl_autostart_pending;
            compl_autostart_pending = false;
            if ins_compl_start() == FAIL {
                return FAIL;
            }
        } else if insert_match && stop_arrow() == FAIL {
            return FAIL;
        }

        compl_curr_win = curwin;
        compl_curr_buf = (*curwin).w_buffer;
        compl_shown_match = compl_curr_match;
        compl_shows_dir = compl_direction;
        compl_num_bests = 0;

        let save_w_wrow = (*curwin).w_wrow;
        let save_w_leftcol = (*curwin).w_leftcol;
        let n = ins_compl_next(true, ins_compl_key2count(c), insert_match);

        if compl_autocomplete {
            compl_time_slice_expired = false;
        }

        if n > 1 {
            compl_matches = n;
        }
        compl_curr_match = compl_shown_match;
        compl_direction = compl_shows_dir;

        if got_int && global_busy == 0 {
            let _ = vgetc();
            got_int = false;
        }

        let no_matches_found = is_first_match((*compl_first_match).cp_next);
        if no_matches_found {
            if compl_length > 1
                || compl_status_adding()
                || (ctrl_x_mode_not_default()
                    && !ctrl_x_mode_path_patterns()
                    && !ctrl_x_mode_path_defines())
            {
                compl_cont_status &= !CONT_N_ADDS;
            }
        }

        if ((*compl_curr_match).cp_flags & CP_CONT_S_IPOS) != 0 {
            compl_cont_status |= CONT_S_IPOS;
        } else {
            compl_cont_status &= !CONT_S_IPOS;
        }

        if !shortmess(kShmCompletionmenu) && !compl_autocomplete {
            ins_compl_show_statusmsg();
        }

        if !shortmess(kShmCompletionscan) && !compl_autocomplete {
            msg_progress(
                ptr::null_mut(),
                c"nvim.completion".as_ptr().cast_mut(),
                c"success".as_ptr().cast_mut(),
                -1,
                0,
                false,
                false,
                false,
            );
        }

        if enable_pum && !compl_interrupted {
            show_pum(save_w_wrow, save_w_leftcol);
        }
        compl_was_interrupted = compl_interrupted;
        compl_interrupted = false;

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_enable_autocomplete() {
    unsafe {
        compl_autocomplete = true;
        compl_get_longest = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_disable_autocomplete() {
    unsafe {
        compl_autocomplete = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_arm_autostart() {
    unsafe {
        compl_autostart_pending = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_disarm_autostart() {
    unsafe {
        compl_autostart_pending = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_arm_autocomplete_delay() -> bool {
    unsafe {
        if p_acl > 0 {
            compl_autocomplete_start_tv = os_hrtime();
            compl_autocomplete_pending = true;
            return true;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_clear_autocomplete_delay() {
    unsafe {
        compl_autocomplete_pending = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_autocomplete_pending() -> bool {
    unsafe { compl_autocomplete_pending }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ins_compl_autocomplete_elapsed() -> i64 {
    unsafe { ((os_hrtime() - compl_autocomplete_start_tv) / 1_000_000) as i64 }
}

unsafe fn show_pum(prev_w_wrow: c_int, prev_w_leftcol: c_int) {
    unsafe {
        let n = RedrawingDisabled;
        RedrawingDisabled = 0;

        setcursor();
        if prev_w_wrow != (*curwin).w_wrow || prev_w_leftcol != (*curwin).w_leftcol {
            ins_compl_del_pum();
        }

        ins_compl_show_pum();
        setcursor();
        RedrawingDisabled = n;
    }
}

unsafe fn quote_meta(mut dest: *mut c_char, mut src: *const c_char, mut len: c_int) -> c_uint {
    unsafe {
        let mut m = (len + 1) as c_uint;

        while {
            len -= 1;
            len >= 0
        } {
            let ch = *src as u8;
            let quote = match ch {
                b'.' | b'*' | b'[' => {
                    if ctrl_x_mode_dictionary() || ctrl_x_mode_thesaurus() {
                        false
                    } else {
                        p_magic != 0
                    }
                }
                b'~' => p_magic != 0,
                b'\\' => !(ctrl_x_mode_dictionary() || ctrl_x_mode_thesaurus()),
                b'^' | b'$' => true,
                _ => false,
            };
            if quote {
                m += 1;
                if !dest.is_null() {
                    *dest = b'\\' as c_char;
                    dest = dest.add(1);
                }
            }
            if !dest.is_null() {
                *dest = *src;
                dest = dest.add(1);
            }
            let mb_len = utfc_ptr2len(src) - 1;
            if mb_len > 0 && len >= mb_len {
                for _ in 0..mb_len {
                    len -= 1;
                    src = src.add(1);
                    if !dest.is_null() {
                        *dest = *src;
                        dest = dest.add(1);
                    }
                }
            }
            src = src.add(1);
        }
        if !dest.is_null() {
            *dest = NUL;
        }

        m
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_insexpand_stuff() {
    unsafe {
        api_clear_string(&raw mut compl_orig_text);
        kv_destroy_extmarks(&raw mut compl_orig_extmarks);
        callback_free(&raw mut p_cfu);
        callback_free(&raw mut p_ofu);
        callback_free(&raw mut p_tsrfu);
        clear_cpt_callbacks(&raw mut cpt_cb, cpt_cb_count);
    }
}

unsafe fn spell_back_to_badword() {
    unsafe {
        let mut tpos = (*curwin).w_cursor;
        spell_bad_len = spell_move_to(curwin, BACKWARD, SMT_ALL, true, ptr::null_mut());
        if (*curwin).w_cursor.col != tpos.col {
            start_arrow(&raw mut tpos, true, NUL as c_int);
        }
    }
}

unsafe fn cpt_sources_clear() {
    unsafe {
        xfree(cpt_sources_array.cast());
        cpt_sources_array = ptr::null_mut();
        cpt_sources_index = -1;
        cpt_sources_count = 0;
    }
}

unsafe fn setup_cpt_sources() {
    unsafe {
        cpt_sources_clear();

        let count = get_cpt_sources_count((*curbuf).b_p_cpt);
        if count == 0 {
            return;
        }

        cpt_sources_array =
            xcalloc(count as usize, size_of::<cpt_source_T>()).cast::<cpt_source_T>();

        let mut buf: [c_char; LSIZE] = [0; LSIZE];
        let mut idx: usize = 0;
        let mut p = (*curbuf).b_p_cpt;
        while *p != NUL {
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
            if *p != NUL {
                (*cpt_sources_array.add(idx)).cs_flag = *p;
                buf.fill(0);
                let slen = copy_option_part(&raw mut p, buf.as_mut_ptr(), LSIZE, c",".as_ptr());
                if slen > 0 {
                    let caret = vim_strchr(buf.as_mut_ptr(), b'^' as c_int);
                    if !caret.is_null() {
                        (*cpt_sources_array.add(idx)).cs_max_matches = atoi(caret.add(1));
                    }
                }
                idx += 1;
            }
        }

        cpt_sources_count = count;
    }
}

unsafe fn is_cpt_func_refresh_always() -> bool {
    unsafe {
        for i in 0..(cpt_sources_count as usize) {
            if (*cpt_sources_array.add(i)).cs_refresh_always {
                return true;
            }
        }
        false
    }
}

unsafe fn ins_compl_make_linear() {
    unsafe {
        if compl_first_match.is_null() || (*compl_first_match).cp_prev.is_null() {
            return;
        }
        let m = (*compl_first_match).cp_prev;
        (*m).cp_next = ptr::null_mut();
        (*compl_first_match).cp_prev = ptr::null_mut();
    }
}

unsafe fn remove_old_matches() {
    unsafe {
        let mut shown_match_removed = false;
        let forward = (*compl_first_match).cp_cpt_source_idx < 0;

        if cpt_sources_index < 0 {
            return;
        }

        compl_direction = if forward { FORWARD } else { BACKWARD };
        compl_shows_dir = compl_direction;

        let mut current = compl_first_match;
        while !current.is_null() {
            if (*current).cp_cpt_source_idx == cpt_sources_index {
                let to_delete = current;

                if !shown_match_removed && ptr::eq(compl_shown_match, current) {
                    shown_match_removed = true;
                }

                if ptr::eq(to_delete, compl_preselect_match) {
                    compl_preselect_match = ptr::null_mut();
                }

                current = (*current).cp_next;

                if ptr::eq(to_delete, compl_first_match) {
                    compl_first_match = (*to_delete).cp_next;
                    (*compl_first_match).cp_prev = ptr::null_mut();
                } else if (*to_delete).cp_next.is_null() {
                    (*(*to_delete).cp_prev).cp_next = ptr::null_mut();
                } else {
                    (*(*to_delete).cp_prev).cp_next = (*to_delete).cp_next;
                    (*(*to_delete).cp_next).cp_prev = (*to_delete).cp_prev;
                }
                ins_compl_item_free(to_delete);
            } else {
                current = (*current).cp_next;
            }
        }

        if shown_match_removed {
            if forward {
                compl_shown_match = compl_first_match;
            } else {
                let mut cur = compl_first_match;
                while !(*cur).cp_next.is_null() {
                    cur = (*cur).cp_next;
                }
                compl_shown_match = cur;
            }
        }

        compl_curr_match = compl_first_match;
        let mut cur = compl_first_match;
        while !cur.is_null() {
            let cond = if forward {
                (*cur).cp_cpt_source_idx < cpt_sources_index
            } else {
                (*cur).cp_cpt_source_idx > cpt_sources_index
            };
            if cond {
                compl_curr_match = if forward { cur } else { (*cur).cp_next };
                cur = (*cur).cp_next;
            } else {
                break;
            }
        }
    }
}

unsafe fn get_cpt_func_completion_matches(cb: *mut Callback) {
    unsafe {
        let cpt_src = &raw mut *cpt_sources_array.add(cpt_sources_index as usize);
        let startcol = (*cpt_src).cs_startcol;

        if startcol == -2 || startcol == -3 {
            return;
        }

        set_compl_globals(startcol, (*curwin).w_cursor.col, true);

        if !(*cpt_src).cs_refresh_always {
            ins_compl_insert_bytes(ins_compl_leader(), -1);
        }

        expand_by_function(0, cpt_compl_pattern.data, cb);

        if !(*cpt_src).cs_refresh_always {
            ins_compl_delete(false);
        }

        (*cpt_src).cs_refresh_always = compl_opt_refresh_always;
        compl_opt_refresh_always = false;
    }
}

unsafe fn cpt_compl_refresh() {
    unsafe {
        ins_compl_make_linear();
        let cpt = xstrdup((*curbuf).b_p_cpt);
        strip_caret_numbers_in_place(cpt);

        cpt_sources_index = 0;
        let mut p = cpt;
        while *p != NUL {
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
            if *p == NUL {
                break;
            }

            if (*cpt_sources_array.add(cpt_sources_index as usize)).cs_refresh_always {
                let cb = get_callback_if_cpt_func(p, cpt_sources_index);
                if !cb.is_null() {
                    remove_old_matches();
                    let mut startcol: c_int = 0;
                    let ret =
                        get_userdefined_compl_info((*curwin).w_cursor.col, cb, &raw mut startcol);
                    if ret == FAIL {
                        if startcol == -3 {
                            (*cpt_sources_array.add(cpt_sources_index as usize))
                                .cs_refresh_always = false;
                        } else {
                            startcol = -2;
                        }
                    } else if startcol < 0 || startcol > (*curwin).w_cursor.col {
                        startcol = (*curwin).w_cursor.col;
                    }
                    (*cpt_sources_array.add(cpt_sources_index as usize)).cs_startcol = startcol;
                    if ret == OK {
                        compl_source_start_timer(cpt_sources_index);
                        get_cpt_func_completion_matches(cb);
                    }
                }
            }

            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            let _ = copy_option_part(&raw mut p, iobuff_ptr, IOSIZE, c",".as_ptr());
            if may_advance_cpt_index(p) {
                let _ = advance_cpt_sources_index_safe();
            }
        }
        cpt_sources_index = -1;

        xfree(cpt.cast());
        compl_matches = ins_compl_make_cyclic();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_preinserted(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if ins_compl_preinsert_effect() {
            (*rettv).vval.v_number = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_caret_numbers_in_place_strips_max_matches_suffixes() {
        let mut buf: Vec<c_char> = b".,w^5,b,u^10,Ffunc^3,k\0"
            .iter()
            .map(|&b| b as c_char)
            .collect();
        unsafe {
            strip_caret_numbers_in_place(buf.as_mut_ptr());
            let s = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
            assert_eq!(s, ".,w,b,u,Ffunc,k");
        }
    }
}
