//! Functions for Insert mode completion (`src/nvim/insexpand.c`).

use crate::api::private::helpers::{
    HLF_E, api_clear_string, cbuf_as_string, cbuf_to_string, copy_string,
};
use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_COMPLETECHANGED, EVENT_COMPLETEDONE, EVENT_COMPLETEDONEPRE, apply_autocmds, has_event,
};
use crate::buffer::{
    MAXLNUM, NOTDONE, VAR_FIXED, bt_cmdwin, buf_T, buf_spname, buf_valid, kUIMessages,
};
use crate::change::{
    HLF_W, MB_MAXCHAR, OPENLINE_FORCE_INDENT, OPENLINE_KEEPTRAIL, deleted_lines_mark,
    ins_bytes_len, ins_char, ins_char_bytes, ins_str, open_line,
};
use crate::charset::{
    ascii_isdigit, ascii_iswhite, ascii_iswhite_or_nul, getwhitecols, skipwhite, str_foldcase,
    vim_isIDc, vim_isfilec, vim_isprintc, vim_iswordc, vim_iswordp, vim_strsize,
};
use crate::cmdexpand::{
    Ctrl_N, Ctrl_P, EXPAND_FILES, EXPAND_LUA, EXPAND_NOTHING, EXPAND_OK, EXPAND_UNSUCCESSFUL,
    RE_LAST, SEARCH_NFMSG, TAG_MANY, addstar, expand_cmdline, free_xp_files_extra, set_cmd_context,
};
use crate::cursor::{
    check_cursor, colnr_T, dec_cursor, equalpos, get_cursor_line_len, get_cursor_line_ptr,
    get_cursor_pos_len, get_cursor_pos_ptr, inc_cursor, linenr_T, pos_T,
};
use crate::drawscreen::{
    conceal_cursor_line, kShmCompletionmenu, redrawWinline, setcursor, showmode, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Callback, EvalFuncData, VarType, callback_copy, callback_free, dict_T, list_T, listitem_T,
    tv_clear, tv_dict_add_bool, tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_add_str_len, tv_dict_add_tv, tv_dict_alloc, tv_dict_alloc_lock,
    tv_dict_alloc_ret, tv_dict_find, tv_dict_get_number, tv_dict_get_string, tv_dict_get_tv,
    tv_dict_set_keys_readonly, tv_dict_unref, tv_get_number_chk, tv_get_string, tv_get_string_chk,
    tv_list_alloc, tv_list_append_dict, tv_list_unref, typval_T, varnumber_T,
};
use crate::eval::userfunc::callback_call_retnr;
use crate::eval::vars::{VV_COMPLETED_ITEM, set_vim_var_dict};
use crate::eval::{
    callback_call, get_v_event, kListLenMayKnow, restore_v_event, set_ref_in_callback,
};
use crate::ex_eval::aborting;
use crate::ex_getln::{
    Ctrl_D, Ctrl_F, Ctrl_K, Ctrl_L, Ctrl_O, Ctrl_Q, Ctrl_R, Ctrl_RSB, Ctrl_T, Ctrl_U, Ctrl_Z,
    K_EVENT, K_LUA, K_SELECT, tilde_replace,
};
use crate::extmark::{ExtmarkUndoObject, extmark_apply_undo, extmark_splice_delete};
use crate::fileio::{fclose, vim_fgets};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzzy_match_str, fuzzy_match_str_in_line, search_for_fuzzy_match,
};
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2attr, syn_id2name, syn_name2attr};
use crate::indent::{get_indent, inindent};
use crate::indent_c::{KEY_COMPLETE, do_c_expr_indent, in_cinkeys};
use crate::input::{
    kInsNone, redo_append_char, redo_append_lit, safe_vgetc, using_script, vgetc, vpeekc,
    vpeekc_any, vungetc,
};
use crate::insert::{
    BS_START, Ctrl_S, Ctrl_X, backspace_until_column, get_can_cindent, ins_apply_autocmds, ins_eol,
    ins_redraw, insertchar, start_arrow, stop_arrow,
};
use crate::keycodes::{
    CAR, K_BS, K_COMMAND, K_DOWN, K_IGNORE, K_KENTER, K_KPAGEDOWN, K_KPAGEUP, K_MOUSEDOWN,
    K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP, K_PAGEDOWN, K_PAGEUP, K_S_DOWN, K_S_TAB,
    K_S_UP, K_UP, NL, TAB,
};
use crate::lua::executor::nlua_expand_pat;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_get_class, mb_islower, mb_isupper, mb_prevptr, mb_ptr_back, mb_ptr2char_adv, mb_tolower,
    mb_toupper, ptr2cells, utf_char2bytes, utf_char2len, utf_head_off, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
};
use crate::memline::{dec, ml_delete, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len};
use crate::memory::{strequal, xfree, xmalloc, xmemdupz, xstrdup, xstrlcpy};
use crate::message::{
    emsg, internal_error, msg, msg_clr_cmdline, msg_delay, msg_ext_set_kind, msg_progress, semsg,
};
use crate::r#move::{changed_cline_bef_curs, curs_columns, validate_cursor};
use crate::option::{
    OptInt, can_bs, copy_option_part, option_set_callback_func, shortmess, was_set_insecurely,
};
use crate::option_vars::*;
use crate::optionstr::{OptIndex, optset_T};
use crate::os::env::expand_T;
use crate::os::fs::os_fopen;
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::os::time::os_hrtime;
use crate::path::{
    EW_ADDSLASH, EW_DIR, EW_FILE, EW_SILENT, PATHSEP, expand_wildcards_vec, path_tail,
};
#[cfg(windows)]
use crate::path::{path_has_drive_letter, path_to_slash};
use crate::popupmenu::{
    pum_clear, pum_display, pum_get_height, pum_set_event_info, pum_undisplay, pum_visible,
    pumitem_T,
};
use crate::regexp::{Ctrl_V, MAXCOL, RE_MAGIC, regmatch_T, vim_regcomp, vim_regexec, vim_regfree};
use crate::register::{NUM_REGISTERS, copy_register, get_register_name, valid_yank_reg};
use crate::search::{Direction, find_pattern_in_path, ignorecase, search_for_exact_line, searchit};
use crate::spell::{
    expand_spelling, spell_dump_compl, spell_expand_check_cap, spell_move_to, spell_word_start,
};
use crate::state::{MODE_INSERT, REPLACE_FLAG, may_trigger_modechanged, save_v_event_T};
#[cfg(unix)]
use crate::strings::vim_strnicmp;
#[cfg(windows)]
use crate::strings::vim_strnicmp;
use crate::strings::{
    atoi, gettext, memcmp, strcat, strchr, strcmp, strcpy, strlen, strncmp, strncpy, strrchr,
    tolower, vim_snprintf, vim_strchr, vim_strsave_escaped, xstrnsave,
};
use crate::tag::find_tags_vec;
use crate::textformat::auto_format;
use crate::ui::{ui_flush, ui_has, vim_beep};
use crate::undo::undo_allowed;
use crate::window::{WinKind, win_T, win_valid};
use crate::winfloat::win_float_find;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

pub const CPT_ABBR: usize = 0;
pub const CPT_KIND: usize = 1;
pub const CPT_MENU: usize = 2;
pub const CPT_INFO: usize = 3;
pub const CPT_COUNT: usize = 4;

const kDirectionNotSet: Direction = 0;

pub(crate) const ACTION_EXPAND: c_int = 5;

pub(crate) const LSIZE: usize = 512;
const MIN_SPACE: c_int = 75;
const LOOKBACK_LINE_COUNT: linenr_T = 1000;

pub(crate) const Ctrl_I: c_int = 9;

pub(crate) const kWinInfo: WinKind = 1;

const VAR_UNKNOWN: VarType = VarType::VAR_UNKNOWN;
const VAR_NUMBER: VarType = VarType::VAR_NUMBER;
pub(crate) const VAR_STRING: VarType = VarType::VAR_STRING;
const VAR_LIST: VarType = VarType::VAR_LIST;
const VAR_DICT: VarType = VarType::VAR_DICT;

pub(crate) const kOptComplete: OptIndex = 51;

const kOptBoFlagComplete: c_uint = 0x08;

pub(crate) const kOptCotFlagFuzzy: c_uint = 0x01;
const kOptCotFlagLongest: c_uint = 0x02;
const kOptCotFlagMenu: c_uint = 0x04;
const kOptCotFlagMenuone: c_uint = 0x08;
const kOptCotFlagNearest: c_uint = 0x10;
const kOptCotFlagNoinsert: c_uint = 0x20;
const kOptCotFlagNoselect: c_uint = 0x40;
const kOptCotFlagNosort: c_uint = 0x80;
const kOptCotFlagPreinsert: c_uint = 0x200;
const kOptCotFlagPreselect: c_uint = 0x400;

pub(crate) const kShmCompletionscan: c_int = b'C' as c_int;

pub(crate) const TAG_NOIC: c_int = 8;
pub(crate) const TAG_INS_COMP: c_int = 64;

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

use crate::api::private::helpers::cstr_as_string;
use crate::mbyte::{mb_ptr_adv, utf_byte2len};
use crate::path::vim_ispathsep;

#[inline]
unsafe fn mb_ptr_adv_const(p: *mut *const c_char) {
    unsafe {
        *p = (*p).add(utf_ptr2len(*p) as usize);
    }
}

/// Structure used to store one match for insert completion (`compl_T`).
#[repr(C)]
pub struct compl_T {
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

impl Drop for compl_T {
    fn drop(&mut self) {
        unsafe {
            let ht_ptr = &raw mut compl_strings_ht;
            if !(*ht_ptr).is_empty()
                && !self.cp_str.data.is_null()
                && (self.cp_flags & CP_ORIGINAL_TEXT) == 0
            {
                let key = CStr::from_ptr(self.cp_str.data).to_bytes();
                if let Some(count) = (*ht_ptr).get_mut(key) {
                    *count -= 1;
                    if *count <= 0 {
                        (*ht_ptr).remove(key);
                    }
                }
            }
            api_clear_string(&raw mut self.cp_str);
            xfree(self.cp_fname.cast());
            free_cptext(self.cp_text.as_ptr());
            tv_clear(&raw mut self.cp_user_data);
            xfree(self.cp_commit_chars.cast());
        }
    }
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

impl ins_compl_next_state_T {
    const fn new() -> Self {
        Self {
            e_cpt_copy: ptr::null_mut(),
            e_cpt: ptr::null_mut(),
            ins_buf: ptr::null_mut(),
            cur_match_pos: ptr::null_mut(),
            prev_match_pos: pos_T::new(),
            set_match_pos: false,
            first_match_pos: pos_T::new(),
            last_match_pos: pos_T::new(),
            found_all: false,
            dict: ptr::null_mut(),
            dict_f: 0,
            func_cb: ptr::null_mut(),
        }
    }
}

impl Default for ins_compl_next_state_T {
    fn default() -> Self {
        Self::new()
    }
}

/// Structure for completion source (in `'cpt'` option) information (`cpt_source_T`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct cpt_source_T {
    cs_refresh_always: bool,
    cs_startcol: c_int,
    cs_max_matches: c_int,
    compl_start_tv: u64,
    cs_flag: c_char,
}

use crate::charset::NUL;
use crate::drawline::{HLF_COUNT, SMT_ALL};
use crate::eval::funcs::SEARCH_KEEP;
use crate::eval::typval::kCallbackNone;
use crate::ex_cmds::{Ctrl_C, Ctrl_E, Ctrl_Y, HLF_R};
use crate::ex_docmd::{FIND_ANY, FIND_DEFINE};
use crate::extmark::kExtmarkUndo;
use crate::globals::IOSIZE;
use crate::help::{TAG_NAMES, TAG_REGEXP, TAG_VERBOSE};
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::search::{BACKWARD, FORWARD};

static mut compl_list: Vec<compl_T> = Vec::new();
static mut compl_is_cyclic: bool = false;
static mut compl_curr_idx: Option<usize> = None;
static mut compl_shown_idx: Option<usize> = None;
static mut compl_old_idx: Option<usize> = None;
static mut compl_preselect_idx: Option<usize> = None;

#[inline]
unsafe fn compl_next_idx(idx: usize) -> Option<usize> {
    unsafe {
        let len = (&raw const compl_list).as_ref().unwrap().len();
        if idx + 1 < len {
            Some(idx + 1)
        } else if compl_is_cyclic && len > 0 {
            Some(0)
        } else {
            None
        }
    }
}

#[inline]
unsafe fn compl_prev_idx(idx: usize) -> Option<usize> {
    unsafe {
        let len = (&raw const compl_list).as_ref().unwrap().len();
        if idx > 0 {
            Some(idx - 1)
        } else if compl_is_cyclic && len > 0 {
            Some(len - 1)
        } else {
            None
        }
    }
}

#[inline]
unsafe fn compl_list_mut() -> &'static mut Vec<compl_T> {
    unsafe { (&raw mut compl_list).as_mut().unwrap() }
}

#[inline]
unsafe fn compl_at(idx: usize) -> *mut compl_T {
    unsafe { &raw mut compl_list_mut()[idx] }
}

#[inline]
unsafe fn compl_opt(idx: Option<usize>) -> *mut compl_T {
    unsafe { idx.map_or(ptr::null_mut(), |i| compl_at(i)) }
}

static mut compl_strings_ht: std::collections::BTreeMap<Vec<u8>, c_int> =
    std::collections::BTreeMap::new();

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
static mut compl_orig_extmarks: Vec<ExtmarkUndoObject> = Vec::new();
static mut compl_cont_mode: c_int = 0;
static mut compl_xp: expand_T = expand_T::new();
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

static mut cpt_sources_array: Vec<cpt_source_T> = Vec::new();
static mut cpt_sources_index: c_int = -1;

static mut compl_match_array: Vec<pumitem_T> = Vec::new();

static mut cpt_cb: Vec<Callback> = Vec::new();

#[inline]
unsafe fn decay_compl_timeout() {
    unsafe {
        if compl_timeout_ms > COMPL_MIN_TIMEOUT_MS {
            compl_timeout_ms /= 2;
        }
    }
}

/// Count the string of a new match in the duplicate-check hashtab.
unsafe fn compl_strings_add(str_ptr: *const c_char, len: usize) {
    unsafe {
        let bytes = std::slice::from_raw_parts(str_ptr.cast::<u8>(), len);
        let key_len = bytes.iter().position(|&b| b == 0).unwrap_or(len);
        let ht_ptr = &raw mut compl_strings_ht;
        *(*ht_ptr).entry(bytes[..key_len].to_vec()).or_insert(0) += 1;
    }
}

/// CTRL-X pressed in Insert mode.
pub unsafe fn ins_ctrl_x() {
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

pub unsafe fn ctrl_x_mode_none() -> bool {
    unsafe { ctrl_x_mode == 0 }
}

pub unsafe fn ctrl_x_mode_normal() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_NORMAL }
}

pub unsafe fn ctrl_x_mode_scroll() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_SCROLL }
}

pub unsafe fn ctrl_x_mode_whole_line() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_WHOLE_LINE }
}

pub unsafe fn ctrl_x_mode_files() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_FILES }
}

pub unsafe fn ctrl_x_mode_tags() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_TAGS }
}

pub unsafe fn ctrl_x_mode_path_patterns() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_PATH_PATTERNS }
}

pub unsafe fn ctrl_x_mode_path_defines() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_PATH_DEFINES }
}

pub unsafe fn ctrl_x_mode_dictionary() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_DICTIONARY }
}

pub unsafe fn ctrl_x_mode_thesaurus() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_THESAURUS }
}

pub unsafe fn ctrl_x_mode_cmdline() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_CMDLINE || ctrl_x_mode == CTRL_X_CMDLINE_CTRL_X }
}

pub unsafe fn ctrl_x_mode_function() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_FUNCTION }
}

pub unsafe fn ctrl_x_mode_omni() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_OMNI }
}

pub unsafe fn ctrl_x_mode_spell() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_SPELL }
}

pub unsafe fn ctrl_x_mode_eval() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_EVAL }
}

pub unsafe fn ctrl_x_mode_line_or_eval() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_WHOLE_LINE || ctrl_x_mode == CTRL_X_EVAL }
}

pub unsafe fn ctrl_x_mode_register() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_REGISTER }
}

pub unsafe fn ctrl_x_mode_not_default() -> bool {
    unsafe { ctrl_x_mode != CTRL_X_NORMAL }
}

pub unsafe fn ctrl_x_mode_not_defined_yet() -> bool {
    unsafe { ctrl_x_mode == CTRL_X_NOT_DEFINED_YET }
}

pub unsafe fn compl_status_adding() -> bool {
    unsafe { (compl_cont_status & CONT_ADDING) != 0 }
}

pub unsafe fn compl_status_sol() -> bool {
    unsafe { (compl_cont_status & CONT_SOL) != 0 }
}

pub unsafe fn compl_status_local() -> bool {
    unsafe { (compl_cont_status & CONT_LOCAL) != 0 }
}

pub unsafe fn compl_status_clear() {
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
pub unsafe fn check_compl_option(dict_opt: bool) -> bool {
    unsafe {
        let empty = if dict_opt {
            *(*curbuf).b_p_dict == NUL && *p_dict == NUL && (*curwin).w_onebuf_opt.wo_spell == 0
        } else {
            *(*curbuf).b_p_tsr == NUL
                && *p_tsr == NUL
                && (*curbuf).b_p_tsrfu.cb_type() == kCallbackNone
                && p_tsrfu.cb_type() == kCallbackNone
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
pub unsafe fn vim_is_ctrl_x_key(c: c_int) -> bool {
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

/// Return the index of the entry holding the original text, None if not found.
unsafe fn find_original_text_match_idx() -> Option<usize> {
    unsafe {
        let list = compl_list_mut();
        if list.is_empty() {
            return None;
        }
        if match_at_original_text(&list[0]) {
            return Some(0);
        }
        if compl_is_cyclic && match_at_original_text(list.last().unwrap()) {
            return Some(list.len() - 1);
        }
        None
    }
}

unsafe fn do_autocmd_completedone(c: c_int, mode: c_int, word: *mut NvimString) {
    unsafe {
        let mut save_v_event: save_v_event_T = save_v_event_T::new();
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
pub unsafe fn ins_compl_accept_char(c: c_int) -> bool {
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
pub unsafe fn ins_compl_commit_char(c: c_int) -> bool {
    unsafe {
        if c < b' ' as c_int {
            return false;
        }
        let shown = compl_opt(compl_shown_idx);
        !shown.is_null()
            && !match_at_original_text(shown)
            && !(*shown).cp_commit_chars.is_null()
            && !vim_strchr((*shown).cp_commit_chars, c).is_null()
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

        let mut wca = vec![0 as c_int; char_len as usize];
        {
            let mut p = str_ptr;
            for item in &mut wca {
                *item = mb_ptr2char_adv(&raw mut p);
            }
        }

        // Rule 1: Were any chars converted to lower?
        {
            let mut p = compl_orig_text.data.cast_const();
            for i in 0..(min_len as usize) {
                let c = mb_ptr2char_adv(&raw mut p);
                if mb_islower(c) {
                    has_lower = true;
                    if mb_isupper(wca[i]) {
                        for item in &mut wca[(compl_char_len as usize)..] {
                            *item = mb_tolower(*item);
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
                if was_letter && mb_isupper(c) && mb_islower(wca[i]) {
                    for item in &mut wca[(compl_char_len as usize)..] {
                        *item = mb_toupper(*item);
                    }
                    break;
                }
                was_letter = mb_islower(c) || mb_isupper(c);
            }
        }

        // Copy the original case of the part we typed.
        {
            let mut p = compl_orig_text.data.cast_const();
            for item in &mut wca[..(min_len as usize)] {
                let c = mb_ptr2char_adv(&raw mut p);
                if mb_islower(c) {
                    *item = mb_tolower(*item);
                } else if mb_isupper(c) {
                    *item = mb_toupper(*item);
                }
            }
        }

        // Generate encoding specific output from wide character array.
        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        let mut p = iobuff_ptr;
        let mut i: c_int = 0;
        let mut gap: Vec<c_char> = Vec::new();
        while i < char_len {
            if !gap.is_empty() {
                let mut ch_buf = [0 as c_char; 6];
                let n = utf_char2bytes(wca[i as usize], ch_buf.as_mut_ptr()) as usize;
                gap.extend_from_slice(&ch_buf[..n]);
                i += 1;
            } else if (p.offset_from(iobuff_ptr) as usize) + 6 >= IOSIZE {
                let cur_len = p.offset_from(iobuff_ptr) as usize;
                gap.reserve(IOSIZE);
                gap.extend_from_slice(std::slice::from_raw_parts(iobuff_ptr, cur_len));
            } else {
                p = p.add(utf_char2bytes(wca[i as usize], p) as usize);
                i += 1;
            }
        }

        if !gap.is_empty() {
            let out = xmemdupz(gap.as_ptr().cast(), gap.len()).cast::<c_char>();
            *tofree = out;
            return out;
        }

        *p = NUL;
        iobuff_ptr
    }
}

/// Like `ins_compl_add()`, but infers case if `'ic'` and `'inf'` are set.
pub unsafe fn ins_compl_add_infercase(
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
pub unsafe fn ins_compl_is_match_selected() -> bool {
    unsafe { !matches!(compl_shown_idx, None | Some(0)) }
}

/// Returns true if autocomplete is active and the pre-insert effect targets the longest prefix.
pub unsafe fn ins_compl_preinsert_longest() -> bool {
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
        let flags = flags_arg;

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
        let ht_ptr = &raw const compl_strings_ht;
        let list = &raw mut compl_list;
        if !(*list).is_empty() && !adup && !(*ht_ptr).is_empty() {
            let bytes = std::slice::from_raw_parts(str_ptr.cast::<u8>(), len as usize);
            let key_len = bytes.iter().position(|&b| b == 0).unwrap_or(len as usize);
            if (*ht_ptr).contains_key(&bytes[..key_len]) {
                if is_nearest_active() && score > 0 {
                    for match_ in &mut *list {
                        if !match_at_original_text(match_)
                            && strncmp(match_.cp_str.data, str_ptr, len as usize) == 0
                            && (match_.cp_str.size as c_int <= len
                                || *match_.cp_str.data.add(len as usize) == NUL)
                        {
                            if score < match_.cp_score {
                                match_.cp_score = score;
                            }
                            break;
                        }
                    }
                }
                if cptext_allocated {
                    free_cptext(cptext);
                }
                xfree(commit_chars.cast());
                return NOTDONE;
            }
        }

        ins_compl_del_pum();

        let new_str = xstrnsave(str_ptr, len as usize);
        let cp_fname = if !fname.is_null() {
            xstrdup(fname)
        } else {
            ptr::null_mut()
        };
        let mut cp_text = [ptr::null_mut(); CPT_COUNT];
        if !cptext.is_null() {
            for (slot, i) in cp_text.iter_mut().zip(0..CPT_COUNT) {
                let s = *cptext.add(i);
                if s.is_null() {
                    continue;
                }
                if *s != NUL {
                    *slot = if cptext_allocated { s } else { xstrdup(s) };
                } else if cptext_allocated {
                    xfree(s.cast());
                }
            }
        }
        let cp_user_data = if !user_data.is_null() {
            *user_data
        } else {
            crate::eval::typval::tv_initial_value()
        };

        let match_ = compl_T {
            cp_str: cbuf_as_string(new_str, len as usize),
            cp_text,
            cp_user_data,
            cp_fname,
            cp_commit_chars: commit_chars,
            cp_flags: flags,
            cp_number: if (flags & CP_ORIGINAL_TEXT) != 0 {
                0
            } else {
                -1
            },
            cp_order: -1,
            cp_preselect: preselect,
            cp_score: score,
            cp_in_match_array: false,
            cp_user_abbr_hl_id: if !user_hl.is_null() { *user_hl } else { 0 },
            cp_user_kind_hl_id: if !user_hl.is_null() {
                *user_hl.add(1)
            } else {
                0
            },
            cp_cpt_source_idx: cpt_sources_index,
        };

        let list = compl_list_mut();
        let insert_idx = if list.is_empty() {
            0
        } else if cot_fuzzy() && score != FUZZY_SCORE_NONE && compl_get_longest {
            if let Some(pos) = list[1..].iter().position(|m| m.cp_score < score) {
                pos + 1
            } else {
                compl_is_cyclic = true;
                list.len()
            }
        } else if dir == FORWARD {
            compl_curr_idx.unwrap() + 1
        } else {
            let curr = compl_curr_idx.unwrap();
            if curr == 0 && compl_is_cyclic {
                list.len()
            } else {
                curr
            }
        };

        for idx_opt in [
            &raw mut compl_curr_idx,
            &raw mut compl_shown_idx,
            &raw mut compl_old_idx,
            &raw mut compl_preselect_idx,
        ] {
            if let Some(idx) = &mut *idx_opt
                && *idx >= insert_idx
            {
                *idx += 1;
            }
        }
        list.insert(insert_idx, match_);
        compl_curr_idx = Some(insert_idx);
        if preselect
            && { compl_preselect_idx }.is_none()
            && (get_cot_flags() & kOptCotFlagPreselect) != 0
        {
            compl_preselect_idx = Some(insert_idx);
        }

        let match_ptr = compl_at(insert_idx);
        if !match_at_original_text(match_ptr) {
            compl_strings_add((*match_ptr).cp_str.data, (*match_ptr).cp_str.size);
        }

        if compl_get_longest
            && (flags & CP_ORIGINAL_TEXT) == 0
            && !cot_fuzzy()
            && !ins_compl_preinsert_longest()
            && !ctrl_x_mode_thesaurus()
        {
            ins_compl_longest_match(match_ptr);
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
            return vim_strnicmp((*match_).cp_str.data, str_ptr, len) == 0;
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
pub unsafe fn ins_compl_leader() -> *mut c_char {
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
pub unsafe fn ins_compl_col_range_attr(lnum: linenr_T, col: c_int) -> c_int {
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
    unsafe {
        let shown = compl_opt(compl_shown_idx);
        !shown.is_null() && !vim_strchr((*shown).cp_str.data, b'\n' as c_int).is_null()
    }
}

/// Returns true if the given line number falls within the range of a multi-line completion.
pub unsafe fn ins_compl_lnum_in_range(lnum: linenr_T) -> bool {
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
            mb_ptr_adv(&mut p);
            mb_ptr_adv(&mut s);
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
unsafe fn ins_compl_add_matches(matches: &mut Vec<*mut c_char>, icase: c_int) {
    unsafe {
        let mut add_r = OK;
        let mut dir = compl_direction;

        for &m in matches.iter() {
            if add_r == FAIL {
                break;
            }
            add_r = ins_compl_add(
                m,
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
        }
        crate::path::clear_cstrings(matches);
    }
}

/// Make the completion list cyclic.
/// Return the number of matches (excluding the original).
unsafe fn ins_compl_make_cyclic() -> c_int {
    unsafe {
        let list = &raw const compl_list;
        if (*list).is_empty() {
            return 0;
        }
        compl_is_cyclic = true;
        ((*list).len() - 1) as c_int
    }
}

pub unsafe fn ins_compl_has_shown_match() -> bool {
    unsafe {
        let Some(shown_idx) = compl_shown_idx else {
            return true;
        };
        compl_next_idx(shown_idx) != Some(shown_idx)
    }
}

pub unsafe fn ins_compl_long_shown_match() -> bool {
    unsafe {
        let shown = compl_opt(compl_shown_idx);
        !shown.is_null()
            && !(*shown).cp_str.data.is_null()
            && ((*shown).cp_str.size as colnr_T) > (*curwin).w_cursor.col - compl_col
    }
}

pub unsafe fn get_cot_flags() -> c_uint {
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
        let arr = &raw mut compl_match_array;
        if (*arr).is_empty() {
            return;
        }

        pum_undisplay(false);
        *arr = Vec::new();
    }
}

pub unsafe fn pum_wanted() -> bool {
    unsafe { (get_cot_flags() & (kOptCotFlagMenu | kOptCotFlagMenuone)) != 0 || compl_autocomplete }
}

unsafe fn pum_enough_matches() -> bool {
    unsafe {
        let list = &raw const compl_list;
        let mut i: c_int = 0;
        for comp in &*list {
            if !match_at_original_text(comp) {
                i += 1;
                if i == 2 {
                    break;
                }
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
            ins_compl_dict_alloc(compl_opt(compl_curr_idx))
        };
        let mut save_v_event: save_v_event_T = save_v_event_T::new();
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

fn cp_compare_fuzzy(a: &compl_T, b: &compl_T) -> c_int {
    let score_a = a.cp_score;
    let score_b = b.cp_score;

    if score_a != score_b {
        return if score_b > score_a { 1 } else { -1 };
    }

    a.cp_order - b.cp_order
}

fn cp_compare_nearest(a: &compl_T, b: &compl_T) -> c_int {
    let score_a = a.cp_score;
    let score_b = b.cp_score;
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

        let cpt_sources = &raw const cpt_sources_array;
        if (*cpt_sources).is_empty() {
            return &raw mut compl_leader;
        }

        let cpt_idx = (*match_).cp_cpt_source_idx;
        if cpt_idx < 0 {
            return &raw mut compl_leader;
        }
        let startcol = cpt_sources_array[cpt_idx as usize].cs_startcol;

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
        let list = &raw mut compl_list;
        if (*list).is_empty() {
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
        for comp in &mut *list {
            if comp.cp_order < 0 {
                comp.cp_order = next;
            }
            next = next.max(comp.cp_order + 1);

            if use_leader {
                pattern = (*get_leader_for_startcol(comp, true)).data;
            }

            comp.cp_score = fuzzy_match_str(comp.cp_str.data, pattern);
        }
    }
}

unsafe fn sort_compl_match_list(compare: fn(&compl_T, &compl_T) -> c_int) {
    unsafe {
        let list = compl_list_mut();
        if list.len() <= 1 {
            return;
        }

        let Some(orig_idx) = find_original_text_match_idx() else {
            return;
        };

        let saved_ptrs = [
            compl_curr_idx.map(|i| list[i].cp_str.data),
            compl_shown_idx.map(|i| list[i].cp_str.data),
            compl_old_idx.map(|i| list[i].cp_str.data),
            compl_preselect_idx.map(|i| list[i].cp_str.data),
        ];

        let range = if orig_idx == 0 {
            1..list.len()
        } else {
            0..orig_idx
        };
        list[range].sort_by(|a, b| {
            let cmp = compare(a, b);
            if cmp < 0 {
                std::cmp::Ordering::Less
            } else if cmp > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });

        for (idx_opt, ptr_opt) in [
            (&raw mut compl_curr_idx, saved_ptrs[0]),
            (&raw mut compl_shown_idx, saved_ptrs[1]),
            (&raw mut compl_old_idx, saved_ptrs[2]),
            (&raw mut compl_preselect_idx, saved_ptrs[3]),
        ] {
            if let Some(p) = ptr_opt {
                *idx_opt = list.iter().position(|m| ptr::eq(m.cp_str.data, p));
            }
        }
        compl_is_cyclic = true;
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
        if ins_compl_need_restart() {
            api_clear_string(&raw mut compl_leader);
        }

        let compl_no_select = (get_cot_flags() & kOptCotFlagNoselect) != 0
            || (compl_autocomplete && !ins_compl_has_preinsert());

        let is_forward = compl_shows_dir_forward();
        let cpt_sources = &raw const cpt_sources_array;
        let is_cpt_completion = !(*cpt_sources).is_empty();

        let list = &raw mut compl_list;
        let mut shown_match_ok = match_at_original_text(compl_opt(compl_shown_idx));

        if strequal(compl_leader.data, compl_orig_text.data) && !shown_match_ok {
            compl_shown_idx = if compl_no_select {
                Some(0)
            } else {
                compl_next_idx(0)
            };
        }

        let mut did_find_shown_match = false;
        let mut preselect_ok = false;
        let mut shown_compl: Option<usize> = None;
        let mut i: c_int = 0;
        let mut cur: c_int = -1;

        let mut match_count = if is_cpt_completion {
            vec![0 as c_int; (*cpt_sources).len()]
        } else {
            Vec::new()
        };

        let _ = get_leader_for_startcol(ptr::null_mut(), true);

        let mut items: Vec<pumitem_T> = Vec::new();
        for (comp_idx, comp) in (*list).iter_mut().enumerate() {
            comp.cp_in_match_array = false;

            let leader = get_leader_for_startcol(comp, true);

            if ctrl_x_mode_normal()
                && p_inf == 0
                && !compl_orig_text.data.is_null()
                && ignorecase(compl_orig_text.data) == 0
                && !cot_fuzzy()
            {
                comp.cp_flags &= !CP_ICASE;
            }

            if !match_at_original_text(comp)
                && ((*leader).data.is_null()
                    || ins_compl_equal_sc(comp, (*leader).data, (*leader).size)
                    || (cot_fuzzy() && comp.cp_score != FUZZY_SCORE_NONE))
            {
                let mut match_limit_exceeded = false;
                let cur_source = comp.cp_cpt_source_idx;
                if is_forward && cur_source != -1 && is_cpt_completion {
                    match_count[cur_source as usize] += 1;
                    let max_matches = cpt_sources_array[cur_source as usize].cs_max_matches;
                    if max_matches > 0 && match_count[cur_source as usize] > max_matches {
                        match_limit_exceeded = true;
                    }
                }

                if !match_limit_exceeded {
                    comp.cp_in_match_array = true;
                    items.push(pumitem_T {
                        pum_text: if !comp.cp_text[CPT_ABBR].is_null() {
                            comp.cp_text[CPT_ABBR]
                        } else {
                            comp.cp_str.data
                        },
                        pum_kind: comp.cp_text[CPT_KIND],
                        pum_extra: if !comp.cp_text[CPT_MENU].is_null() {
                            comp.cp_text[CPT_MENU]
                        } else {
                            comp.cp_fname
                        },
                        pum_info: comp.cp_text[CPT_INFO],
                        pum_cpt_source_idx: comp.cp_cpt_source_idx,
                        pum_user_abbr_hlattr: get_user_highlight_attr(comp.cp_user_abbr_hl_id),
                        pum_user_kind_hlattr: get_user_highlight_attr(comp.cp_user_kind_hl_id),
                    });

                    if !shown_match_ok && !cot_fuzzy() {
                        if compl_shown_idx == Some(comp_idx) || did_find_shown_match {
                            compl_shown_idx = Some(comp_idx);
                            did_find_shown_match = true;
                            shown_match_ok = true;
                        } else {
                            shown_compl = Some(comp_idx);
                        }
                        cur = i;
                    } else if cot_fuzzy() {
                        if i == 0 {
                            shown_compl = Some(comp_idx);
                        }

                        if !shown_match_ok && compl_shown_idx == Some(comp_idx) {
                            cur = i;
                            shown_match_ok = true;
                        }
                    }
                    if { compl_preselect_idx }.is_some() && comp.cp_preselect && !preselect_ok {
                        cur = i;
                        compl_shown_idx = Some(comp_idx);
                        shown_match_ok = true;
                        preselect_ok = true;
                    }
                    i += 1;
                }
            }

            if compl_shown_idx == Some(comp_idx) && !cot_fuzzy() {
                did_find_shown_match = true;
                if match_at_original_text(comp) {
                    shown_match_ok = true;
                }
                if !shown_match_ok && shown_compl.is_some() {
                    compl_shown_idx = shown_compl;
                    shown_match_ok = true;
                }
            }
        }

        if items.is_empty() {
            return -1;
        }

        if cot_fuzzy() && !compl_no_select && !shown_match_ok {
            compl_shown_idx = shown_compl;
            shown_match_ok = true;
            cur = 0;
        }

        compl_match_array = items;

        if !shown_match_ok {
            cur = -1;
        }

        cur
    }
}

/// Show the popup menu for the list of matches.
pub unsafe fn ins_compl_show_pum() {
    unsafe {
        if !pum_wanted() || !pum_enough_matches() {
            return;
        }

        update_screen();

        let mut cur: c_int = -1;
        let mut array_changed = false;

        let arr = &raw mut compl_match_array;
        if (*arr).is_empty() {
            array_changed = true;
            cur = ins_compl_build_pum();
        } else {
            let shown = compl_opt(compl_shown_idx);
            for (i, item) in (*arr).iter().enumerate() {
                let pum_text = item.pum_text;
                if ptr::eq(pum_text, (*shown).cp_str.data)
                    || ptr::eq(pum_text, (*shown).cp_text[CPT_ABBR])
                {
                    cur = i as c_int;
                    break;
                }
            }
        }

        if (*arr).is_empty() {
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
            (*arr).as_mut_ptr(),
            (*arr).len() as c_int,
            cur,
            array_changed,
            0,
        );
        (*curwin).w_cursor.col = col;

        if (*curwin).w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(curwin) {
            redrawWinline(curwin, (*curwin).w_cursor.lnum);
        }

        if compl_started && compl_curr_idx != compl_shown_idx {
            compl_curr_idx = compl_shown_idx;
        }

        if has_event(EVENT_COMPLETECHANGED) {
            trigger_complete_changed_event(cur);
        }
    }
}

pub unsafe fn compl_match_curr_select(selected: c_int) -> bool {
    unsafe {
        if selected < 0 {
            return false;
        }
        let list = &raw const compl_list;
        let curr = compl_opt(compl_curr_idx);
        let mut selected_idx: c_int = -1;
        let mut list_idx: c_int = 0;
        for match_ in &*list {
            if !match_at_original_text(match_) {
                if !curr.is_null() && (*curr).cp_number == match_.cp_number {
                    selected_idx = list_idx;
                    break;
                }
                list_idx += 1;
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
        let mut regmatch: regmatch_T = regmatch_T::new();
        let mut files: Vec<*mut c_char> = Vec::new();
        let mut count: c_int;
        let mut dir = compl_direction;

        if *dict == NUL {
            if !thesaurus && (*curwin).w_onebuf_opt.wo_spell != 0 {
                dict = c"spell".as_ptr().cast_mut();
            } else {
                return;
            }
        }

        let mut buf_vec = vec![0 as c_char; LSIZE];
        let mut buf = buf_vec.as_mut_ptr();
        regmatch.regprog = ptr::null_mut();

        let save_p_scs = p_scs;
        if (*curbuf).b_p_inf != 0 {
            p_scs = 0;
        }

        if ctrl_x_mode_line_or_eval() {
            let pat_esc = vim_strsave_escaped(pat, c"\\".as_ptr());
            let len = strlen(pat_esc) + 10;
            let mut ptr_vec = vec![0 as c_char; len];
            let ptr = ptr_vec.as_mut_ptr();
            vim_snprintf!(ptr, len, c"^\\s*\\zs\\V%s".as_ptr(), pat_esc);
            regmatch.regprog = vim_regcomp(ptr, RE_MAGIC);
            xfree(pat_esc.cast());
        } else {
            regmatch.regprog = vim_regcomp(pat, if p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                p_scs = save_p_scs;
                vim_regfree(regmatch.regprog);
                return;
            }
        }

        regmatch.rm_ic = ignorecase(pat) != 0;
        while *dict != NUL && !got_int && !compl_interrupted {
            if flags == DICT_EXACT {
                count = 1;
            } else {
                copy_option_part(&raw mut dict, buf, LSIZE, c",".as_ptr());
                if !thesaurus && strcmp(buf, c"spell".as_ptr()) == 0 {
                    count = -1;
                } else if !vim_strchr(buf, b'`' as c_int).is_null()
                    || expand_wildcards_vec(1, &raw mut buf, &mut files, EW_FILE | EW_SILENT) != OK
                {
                    count = 0;
                } else {
                    count = files.len() as c_int;
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
                if flags == DICT_EXACT {
                    ins_compl_files(
                        &[dict],
                        thesaurus,
                        flags,
                        &raw mut regmatch,
                        buf,
                        &raw mut dir,
                    );
                } else {
                    ins_compl_files(
                        &files,
                        thesaurus,
                        flags,
                        &raw mut regmatch,
                        buf,
                        &raw mut dir,
                    );
                    crate::path::clear_cstrings(&mut files);
                }
            }
            if flags != 0 {
                break;
            }
        }

        p_scs = save_p_scs;
        vim_regfree(regmatch.regprog);
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
    files: &[*mut c_char],
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

        for &file in files {
            if got_int || ins_compl_interrupted() {
                break;
            }
            let fp = os_fopen(file, c"r".as_ptr());
            if flags != DICT_EXACT && !shortmess(kShmCompletionscan) && !compl_autocomplete {
                let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                vim_snprintf!(
                    iobuff_ptr,
                    IOSIZE,
                    gettext(c"Scanning dictionary: %s".as_ptr()),
                    file,
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
                                file,
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
                                && let Some(next_i) = compl_next_idx(0)
                                && score == (*compl_at(next_i)).cp_score
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
                            file,
                            *dir,
                            false,
                            FUZZY_SCORE_NONE,
                        );
                        if thesaurus {
                            ptr = buf;
                            add_r = thesaurus_add_words_in_line(
                                file,
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
        }
    }
}

pub unsafe fn find_word_start(mut ptr: *mut c_char) -> *mut c_char {
    unsafe {
        while *ptr != NUL && *ptr != b'\n' as c_char && mb_get_class(ptr) <= 1 {
            ptr = ptr.add(utfc_ptr2len(ptr) as usize);
        }
        ptr
    }
}

pub unsafe fn find_word_end(mut ptr: *mut c_char) -> *mut c_char {
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

pub unsafe fn find_line_end(ptr: *mut c_char) -> *mut c_char {
    unsafe {
        let mut s = ptr.add(strlen(ptr));
        while s > ptr && (*s.sub(1) == CAR as c_char || *s.sub(1) == NL as c_char) {
            s = s.sub(1);
        }
        s
    }
}

unsafe fn ins_compl_free() {
    unsafe {
        api_clear_string(&raw mut compl_pattern);
        api_clear_string(&raw mut compl_leader);

        let list = &raw mut compl_list;
        if (*list).is_empty() {
            return;
        }

        ins_compl_del_pum();
        pum_clear();

        let ht_ptr = &raw mut compl_strings_ht;
        (*ht_ptr).clear();

        (*list).clear();
        compl_is_cyclic = false;
        compl_curr_idx = None;
        compl_shown_idx = None;
        compl_preselect_idx = None;
        compl_old_idx = None;
    }
}

pub unsafe fn ins_compl_clear() {
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
        let extmarks = &raw mut compl_orig_extmarks;
        (*extmarks).clear();
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

pub unsafe fn ins_compl_active() -> bool {
    unsafe { compl_started }
}

pub unsafe fn ins_compl_win_active(wp: *mut win_T) -> bool {
    unsafe {
        ins_compl_active() && ptr::eq(wp, compl_curr_win) && ptr::eq((*wp).w_buffer, compl_curr_buf)
    }
}

pub unsafe fn ins_compl_used_match() -> bool {
    unsafe { compl_used_match }
}

pub unsafe fn ins_compl_init_get_longest() {
    unsafe {
        compl_get_longest = false;
    }
}

pub unsafe fn ins_compl_interrupted() -> bool {
    unsafe { compl_interrupted || compl_time_slice_expired }
}

pub unsafe fn ins_compl_enter_selects() -> bool {
    unsafe { compl_enter_selects }
}

pub unsafe fn ins_compl_col() -> colnr_T {
    unsafe { compl_col }
}

pub unsafe fn ins_compl_len() -> c_int {
    unsafe { compl_length }
}

pub unsafe fn ins_compl_has_preinsert() -> bool {
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

pub unsafe fn ins_compl_preinsert_effect() -> bool {
    unsafe {
        if !ins_compl_has_preinsert() && !ins_compl_preinsert_longest() {
            return false;
        }
        (*curwin).w_cursor.col < compl_ins_end_col
    }
}

pub unsafe fn ins_compl_bs() -> c_int {
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

        if compl_autocomplete
            && !(&raw const compl_list).as_ref().unwrap().is_empty()
            && !ins_compl_has_preinsert()
        {
            compl_shown_idx = Some(0);
        }

        ins_compl_new_leader();
        if { compl_shown_idx }.is_some() {
            compl_curr_idx = compl_shown_idx;
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

pub unsafe fn ins_compl_has_autocomplete() -> bool {
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
            sort_compl_match_list(cp_compare_fuzzy);
            if (cur_cot_flags & (kOptCotFlagNoinsert | kOptCotFlagNoselect)) == kOptCotFlagNoinsert
            {
                let target = if compl_shows_dir_forward() {
                    Some(0)
                } else {
                    compl_prev_idx(0)
                };
                let none_selected = compl_shown_idx == target;
                if !none_selected {
                    compl_shown_idx = if !compl_autocomplete && compl_shows_dir_forward() {
                        compl_next_idx(0)
                    } else {
                        Some(0)
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

        let arr = &raw const compl_match_array;
        if (*arr).is_empty() {
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

pub unsafe fn ins_compl_addleader(c: c_int) {
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
        let Some(orig_idx) = find_original_text_match_idx() else {
            return;
        };
        let match_ = compl_at(orig_idx);
        api_clear_string(&raw mut (*match_).cp_str);
        (*match_).cp_str = cbuf_to_string(str_ptr, len);
    }
}

pub unsafe fn ins_compl_addfrommatch() {
    unsafe {
        let len = (*curwin).w_cursor.col - compl_col;
        let shown_idx = compl_shown_idx.unwrap();
        let shown = compl_at(shown_idx);
        let mut p = (*shown).cp_str.data;
        if ((*shown).cp_str.size as c_int) <= len {
            if !match_at_original_text(shown) {
                return;
            }

            p = ptr::null_mut();
            let mut plen: usize = 0;
            let mut cp_idx = compl_next_idx(shown_idx);
            while let Some(ci) = cp_idx
                && ci != 0
            {
                let cp = compl_at(ci);
                if compl_leader.data.is_null()
                    || ins_compl_equal(cp, compl_leader.data, compl_leader.size)
                {
                    p = (*cp).cp_str.data;
                    plen = (*cp).cp_str.size;
                    break;
                }
                cp_idx = compl_next_idx(ci);
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

        let curr = compl_opt(compl_curr_idx);
        if !curr.is_null() || !compl_leader.data.is_null() || c == Ctrl_E {
            let mut ptr: *mut c_char = ptr::null_mut();
            if !curr.is_null() && compl_used_match && c != Ctrl_E {
                ptr = (*curr).cp_str.data;
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
            let shown = compl_opt(compl_shown_idx);
            word = copy_string((*shown).cp_str, ptr::null_mut());
            if !is_commit {
                retval = true;
            }
            redrawWinline(curwin, (*curwin).w_cursor.lnum);
        }

        let arr = &raw const compl_match_array;
        let curr = compl_opt(compl_curr_idx);
        if word.data.is_null()
            && c != Ctrl_E
            && compl_used_match
            && (*arr).is_empty()
            && !curr.is_null()
            && !(*curr).cp_str.data.is_null()
        {
            word = copy_string((*curr).cp_str, ptr::null_mut());
        }

        if c == Ctrl_E {
            ins_compl_delete(false);
            let mut p: *const c_char = ptr::null();
            let mut plen: usize = 0;
            if !compl_leader.data.is_null() {
                p = compl_leader.data;
                plen = compl_leader.size;
            } else if !(&raw const compl_list).as_ref().unwrap().is_empty() {
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

pub unsafe fn ins_compl_cancel() -> bool {
    unsafe { ins_compl_stop(b' ' as c_int, ctrl_x_mode, true) }
}

pub unsafe fn ins_compl_prep(c: c_int) -> bool {
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

pub unsafe fn clear_cpt_callbacks(callbacks: &mut Vec<Callback>) {
    unsafe {
        for cb in callbacks.iter_mut() {
            callback_free(cb);
        }
        callbacks.clear();
    }
}

unsafe fn copy_cpt_callbacks(dest: &mut Vec<Callback>, src: &mut [Callback]) {
    unsafe {
        clear_cpt_callbacks(dest);

        if src.is_empty() {
            return;
        }

        dest.resize(src.len(), Callback::new());

        for (dst_cb, src_cb) in dest.iter_mut().zip(src.iter_mut()) {
            if src_cb.cb_type() != kCallbackNone {
                callback_copy(dst_cb, src_cb);
            }
        }
    }
}

pub unsafe fn set_buflocal_cpt_callbacks(buf: *mut buf_T) {
    unsafe {
        if buf.is_null() {
            return;
        }
        let global_cb = &raw mut cpt_cb;
        copy_cpt_callbacks(&mut (*buf).b_p_cpt_cb, &mut *global_cb);
    }
}

unsafe fn parse_cpt_callbacks(cpt: *mut c_char, cbp: &mut Vec<Callback>) {
    unsafe {
        clear_cpt_callbacks(cbp);

        let count = get_cpt_sources_count(cpt);
        if count == 0 {
            return;
        }

        cbp.resize(count as usize, Callback::new());

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
                    if option_set_callback_func(buf.as_mut_ptr().add(1), &raw mut cbp[idx]) != OK {
                        cbp[idx] = Callback::NONE;
                    }
                }
                idx += 1;
            }
        }
    }
}

pub unsafe fn set_cpt_callbacks(args: *mut optset_T) -> c_int {
    unsafe {
        let opt_flags = (*args).os_flags;

        if curbuf.is_null() {
            return FAIL;
        }

        if (opt_flags & OPT_GLOBAL) == 0 {
            parse_cpt_callbacks((*curbuf).b_p_cpt, &mut (*curbuf).b_p_cpt_cb);
        }

        if (opt_flags & OPT_LOCAL) != 0 {
            return OK;
        }

        let global_cb = &raw mut cpt_cb;
        if (opt_flags & OPT_GLOBAL) != 0 {
            parse_cpt_callbacks(p_cpt, &mut *global_cb);
            return OK;
        }

        copy_cpt_callbacks(&mut *global_cb, &mut (*curbuf).b_p_cpt_cb);
        OK
    }
}

pub unsafe fn set_ref_in_cpt_callbacks(callbacks: &mut [Callback], copyID: c_int) -> bool {
    unsafe {
        let mut abort = false;
        for cb in callbacks {
            abort = abort || set_ref_in_callback(cb, copyID, ptr::null_mut(), ptr::null_mut());
        }
        abort
    }
}

pub unsafe fn set_ref_in_insexpand_funcs(copyID: c_int) -> bool {
    unsafe {
        let mut abort =
            set_ref_in_callback(&raw mut p_cfu, copyID, ptr::null_mut(), ptr::null_mut());
        abort =
            abort || set_ref_in_callback(&raw mut p_ofu, copyID, ptr::null_mut(), ptr::null_mut());
        abort = abort
            || set_ref_in_callback(&raw mut p_tsrfu, copyID, ptr::null_mut(), ptr::null_mut());
        let global_cb = &raw mut cpt_cb;
        abort = abort || set_ref_in_cpt_callbacks(&mut *global_cb, copyID);
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
        if (*curbuf).b_p_tsrfu.cb_type() != kCallbackNone {
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
        let mut rettv: typval_T = typval_T::new();
        let save_State = State;

        debug_assert!(!curbuf.is_null());

        let is_cpt_function = !cb.is_null();
        let use_sandbox =
            is_cpt_function && was_set_insecurely(curwin, kOptComplete, OPT_LOCAL) != 0;
        if !is_cpt_function {
            cb = get_insert_callback(type_);
            if (*cb).cb_type() == kCallbackNone {
                return;
            }
        }

        let mut args: [typval_T; 3] = [typval_T::new(); 3];
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
        let mut user_data: typval_T = typval_T::new();
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
        let extmarks = &raw const compl_orig_extmarks;
        for &undo_info in (*extmarks).iter().rev() {
            extmark_apply_undo(undo_info, true);
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

        compl_curr_idx = Some(0);
        let no_select = compl_no_select || compl_longest;
        if let Some(pre_idx) = compl_preselect_idx
            && !no_select
        {
            compl_curr_idx = compl_prev_idx(pre_idx);
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

pub unsafe fn f_complete(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
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

pub unsafe fn f_complete_add(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(ins_compl_add_tv(argvars, 0, false));
    }
}

pub unsafe fn f_complete_check(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
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
        let Some(curr_idx) = compl_curr_idx else {
            return;
        };

        if compl_dir_forward() {
            let mut match_idx = compl_prev_idx(curr_idx);
            while let Some(mi) = match_idx
                && mi != 0
            {
                let m = compl_at(mi);
                if (*m).cp_number != -1 {
                    number = (*m).cp_number;
                    break;
                }
                match_idx = compl_prev_idx(mi);
            }
            if let Some(mi) = match_idx {
                match_idx = compl_next_idx(mi);
                while let Some(cur_mi) = match_idx {
                    let m = compl_at(cur_mi);
                    if (*m).cp_number != -1 {
                        break;
                    }
                    number += 1;
                    (*m).cp_number = number;
                    match_idx = compl_next_idx(cur_mi);
                }
            }
        } else {
            debug_assert!(compl_direction == BACKWARD);
            let mut match_idx = compl_next_idx(curr_idx);
            while let Some(mi) = match_idx
                && mi != 0
            {
                let m = compl_at(mi);
                if (*m).cp_number != -1 {
                    number = (*m).cp_number;
                    break;
                }
                match_idx = compl_next_idx(mi);
            }
            if let Some(mi) = match_idx {
                match_idx = compl_prev_idx(mi);
                while let Some(cur_mi) = match_idx {
                    let m = compl_at(cur_mi);
                    if (*m).cp_number != -1 {
                        break;
                    }
                    number += 1;
                    (*m).cp_number = number;
                    match_idx = compl_prev_idx(cur_mi);
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
            if ret == OK
                && (what_flag & CI_WHAT_SELECTED) != 0
                && let Some(ci) = compl_curr_idx
                && (*compl_at(ci)).cp_number == -1
            {
                ins_compl_update_sequence_numbers();
            }
            let list = &raw mut compl_list;
            let curr = compl_opt(compl_curr_idx);
            if ret == OK && !(*list).is_empty() {
                let mut list_idx: c_int = 0;
                for match_ in &mut *list {
                    if !match_at_original_text(match_) {
                        if has_items || (has_matches && match_.cp_in_match_array) {
                            let di = tv_dict_alloc();
                            tv_list_append_dict(li, di);
                            fill_complete_info_dict(di, match_, has_matches && has_items);
                        }
                        if !curr.is_null() && (*curr).cp_number == match_.cp_number {
                            selected_idx = list_idx;
                        }
                        if !has_matches || match_.cp_in_match_array {
                            list_idx += 1;
                        }
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
                fill_complete_info_dict(di, compl_opt(compl_curr_idx), false);
                let _ = tv_dict_add_dict(retdict, c"completed".as_ptr(), 9, di);
            }
        }
    }
}

pub unsafe fn f_complete_info(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
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
            && ((*curbuf).b_p_tsrfu.cb_type() != kCallbackNone
                || p_tsrfu.cb_type() != kCallbackNone)
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
        let mut matches: Vec<*mut c_char> = Vec::new();
        if find_tags_vec(
            compl_pattern.data,
            &mut matches,
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
            && !matches.is_empty()
        {
            ins_compl_add_matches(&mut matches, p_ic);
        }
        g_tag_at_cursor = false;
        p_ic = save_p_ic;
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

        let nn_idx = compl_next_idx(0).and_then(|i| compl_next_idx(i));
        let more_candidates = nn_idx.is_some_and(|i| i != 0);

        let mut compl_idx = if ctrl_x_mode_whole_line() {
            Some(0)
        } else {
            compl_next_idx(0)
        };
        if compl_num_bests == 1 {
            if !more_candidates && let Some(ci) = compl_idx {
                ins_compl_longest_insert((*compl_at(ci)).cp_str.data);
                compl_num_bests = 0;
            }
            compl_num_bests = 0;
            return;
        }

        if (compl_num_bests as usize) > usize::MAX / size_of::<*mut compl_T>() {
            return;
        }
        let mut compl_best_matches = Vec::with_capacity(compl_num_bests as usize);

        let mut i: c_int = 0;
        while let Some(ci) = compl_idx
            && i < compl_num_bests
        {
            compl_best_matches.push(compl_at(ci));
            compl_idx = compl_next_idx(ci);
            i += 1;
        }

        let prefix = (*compl_best_matches[0]).cp_str.data;
        let mut prefix_len = (*compl_best_matches[0]).cp_str.size as c_int;

        for &best in &compl_best_matches[1..] {
            let match_str = (*best).cp_str.data;
            let mut prefix_ptr = prefix;
            let mut match_ptr = match_str;
            let mut j: c_int = 0;

            while j < prefix_len && *match_ptr != NUL && *prefix_ptr != NUL {
                if strncmp(prefix_ptr, match_ptr, utfc_ptr2len(prefix_ptr) as usize) != 0 {
                    break;
                }

                mb_ptr_adv(&mut prefix_ptr);
                mb_ptr_adv(&mut match_ptr);
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

        compl_num_bests = 0;
    }
}

unsafe fn get_next_filename_completion() {
    unsafe {
        let mut matches: Vec<*mut c_char> = Vec::new();
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

        if expand_wildcards_vec(
            1,
            &raw mut compl_pattern.data,
            &mut matches,
            EW_FILE | EW_DIR | EW_ADDSLASH | EW_SILENT,
        ) != OK
        {
            return;
        }

        tilde_replace(compl_pattern.data, &mut matches);
        #[cfg(windows)]
        {
            if (*(*curbuf).b_p_csl == NUL && p_ssl == 0) || *(*curbuf).b_p_csl == b'b' as c_char {
                for &match_ptr in &matches {
                    let mut ptr = match_ptr;
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
            let mut fuzzy_indices: Vec<c_int> = Vec::with_capacity(10);
            let mut compl_fuzzy_scores = vec![0 as c_int; matches.len()];

            for (i, &ptr) in matches.iter().enumerate() {
                let score = fuzzy_match_str(ptr, leader);
                if score != FUZZY_SCORE_NONE {
                    fuzzy_indices.push(i as c_int);
                    compl_fuzzy_scores[i] = score;
                }
            }

            if !fuzzy_indices.is_empty() {
                fuzzy_indices.sort_unstable_by(|&idx_a, &idx_b| {
                    let score_a = compl_fuzzy_scores[idx_a as usize];
                    let score_b = compl_fuzzy_scores[idx_b as usize];
                    score_b.cmp(&score_a).then_with(|| idx_a.cmp(&idx_b))
                });

                for (i, &idx_c) in fuzzy_indices.iter().enumerate() {
                    let idx = idx_c as usize;
                    let match_ = matches[idx];
                    let current_score = compl_fuzzy_scores[idx];
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

                crate::path::clear_cstrings(&mut matches);
            } else if leader_len > 0 {
                crate::path::clear_cstrings(&mut matches);
            }

            if compl_num_bests > 0 && compl_get_longest {
                fuzzy_longest_match();
            }
            return;
        }

        if !matches.is_empty() {
            ins_compl_add_matches(&mut matches, c_int::from(p_fic != 0 || p_wic != 0));
        }
    }
}

unsafe fn get_next_cmdline_completion() {
    unsafe {
        let mut matches: Vec<*mut c_char> = Vec::new();
        if expand_cmdline(
            &raw mut compl_xp,
            compl_pattern.data,
            compl_pattern.size as c_int,
            &mut matches,
        ) == EXPAND_OK
        {
            let mut add_r = OK;
            let mut dir = compl_direction;

            for (i, &match_i) in matches.iter().enumerate() {
                if add_r == FAIL {
                    break;
                }
                let mut cptext: [*mut c_char; CPT_COUNT] = [ptr::null_mut(); CPT_COUNT];

                let cxp = &raw const compl_xp;
                if let Some(&abbr) = (*cxp).xp_files_abbr.as_slice().get(i) {
                    cptext[CPT_ABBR] = abbr;
                }
                if let Some(&kind) = (*cxp).xp_files_kind.as_slice().get(i) {
                    cptext[CPT_KIND] = kind;
                }
                if let Some(&menu) = (*cxp).xp_files_menu.as_slice().get(i) {
                    cptext[CPT_MENU] = menu;
                }
                if let Some(&info) = (*cxp).xp_files_info.as_slice().get(i) {
                    cptext[CPT_INFO] = info;
                }

                add_r = ins_compl_add(
                    match_i,
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
            }
            crate::path::clear_cstrings(&mut matches);
            free_xp_files_extra(&raw mut compl_xp);
        }
    }
}

unsafe fn get_next_spell_completion(lnum: linenr_T) {
    unsafe {
        let mut matches: Vec<*mut c_char> = Vec::new();
        expand_spelling(lnum, compl_pattern.data, &mut matches);
        if !matches.is_empty() {
            ins_compl_add_matches(&mut matches, p_ic);
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
            } else if (compl_dir_forward()
                && ((*st).prev_match_pos.lnum > (*(*st).cur_match_pos).lnum
                    || ((*st).prev_match_pos.lnum == (*(*st).cur_match_pos).lnum
                        && (*st).prev_match_pos.col >= (*(*st).cur_match_pos).col)))
                || (!compl_dir_forward()
                    && ((*st).prev_match_pos.lnum < (*(*st).cur_match_pos).lnum
                        || ((*st).prev_match_pos.lnum == (*(*st).cur_match_pos).lnum
                            && (*st).prev_match_pos.col <= (*(*st).cur_match_pos).col)))
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
                if in_fuzzy_collect
                    && let Some(next_i) = compl_next_idx(0)
                    && score == (*compl_at(next_i)).cp_score
                {
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

            if !reg.y_allocated || reg.y_array.is_empty() {
                continue;
            }

            for j in 0..reg.y_array.len() {
                let str_ptr = reg.y_array[j].data;
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
                            vim_strnicmp(str_ptr, compl_orig_text.data, compl_orig_text.size) == 0
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
                                vim_strnicmp(p, compl_orig_text.data, compl_orig_text.size) == 0
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
                let cbs = &mut (*curbuf).b_p_cpt_cb;
                if idx < 0 || (idx as usize) >= cbs.len() {
                    return ptr::null_mut();
                }
                let cb = &raw mut cbs[idx as usize];
                return if (*cb).cb_type() != kCallbackNone {
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

        if type_ != 0 && compl_curr_idx != compl_old_idx {
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
                        cpt_sources_array[idx as usize].cs_refresh_always = false;
                    } else {
                        startcol = -2;
                    }
                } else if startcol < 0 || startcol > (*curwin).w_cursor.col {
                    startcol = (*curwin).w_cursor.col;
                }
                cpt_sources_array[idx as usize].cs_startcol = startcol;
            } else {
                cpt_sources_array[idx as usize].cs_startcol = -3;
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
            cpt_sources_array[source_idx as usize].compl_start_tv = os_hrtime();
            compl_time_slice_expired = false;
        }
    }
}

unsafe fn advance_cpt_sources_index_safe() -> c_int {
    unsafe {
        let cpt_sources = &raw const cpt_sources_array;
        if cpt_sources_index >= 0 && (cpt_sources_index as usize) + 1 < (*cpt_sources).len() {
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
    static mut st: ins_compl_next_state_T = ins_compl_next_state_T::new();
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
                st = ins_compl_next_state_T::new();
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

        compl_old_idx = compl_curr_idx;
        st.cur_match_pos = if compl_dir_forward() {
            &raw mut st.last_match_pos
        } else {
            &raw mut st.first_match_pos
        };

        let cpt_sources = &raw const cpt_sources_array;
        let normal_mode_strict = ctrl_x_mode_normal()
            && !ctrl_x_mode_line_or_eval()
            && (compl_cont_status & CONT_LOCAL) == 0
            && !(*cpt_sources).is_empty();
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
                while let Some(curr_i) = compl_curr_idx
                    && let Some(prev_i) = compl_prev_idx(curr_i)
                    && !match_at_original_text(compl_at(prev_i))
                {
                    compl_curr_idx = Some(prev_i);
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

        if let Some(old_i) = compl_old_idx {
            let next_curr = if compl_dir_forward() {
                compl_next_idx(old_i)
            } else {
                compl_prev_idx(old_i)
            };
            compl_curr_idx = Some(next_curr.unwrap_or(old_i));
        }
        may_trigger_modechanged();

        if match_count > 0 && !ctrl_x_mode_spell() {
            if is_nearest_active() && !ins_compl_has_preinsert() {
                sort_compl_match_list(cp_compare_nearest);
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
        let Some(mut shown_idx) = compl_shown_idx else {
            return;
        };
        let mut leader = get_leader_for_startcol(compl_at(shown_idx), true);

        while !ins_compl_equal(compl_at(shown_idx), (*leader).data, (*leader).size)
            && let Some(next_i) = compl_next_idx(shown_idx)
            && next_i != 0
        {
            shown_idx = next_i;
            compl_shown_idx = Some(shown_idx);
            leader = get_leader_for_startcol(compl_at(shown_idx), true);
        }

        if compl_shows_dir_backward()
            && !ins_compl_equal(compl_at(shown_idx), (*leader).data, (*leader).size)
            && compl_next_idx(shown_idx).is_none_or(|ni| ni == 0)
        {
            while !ins_compl_equal(compl_at(shown_idx), (*leader).data, (*leader).size)
                && let Some(prev_i) = compl_prev_idx(shown_idx)
                && prev_i != 0
            {
                shown_idx = prev_i;
                compl_shown_idx = Some(shown_idx);
                leader = get_leader_for_startcol(compl_at(shown_idx), true);
            }
        }
    }
}

pub unsafe fn ins_compl_delete(new_leader: bool) {
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
        let cpt_sources = &raw const cpt_sources_array;
        let is_cpt_completion = !(*cpt_sources).is_empty();

        if !is_cpt_completion {
            return ptr::null_mut();
        }

        let mut match_count = vec![0 as c_int; (*cpt_sources).len()];

        let _ = get_leader_for_startcol(ptr::null_mut(), true);

        let list = &raw mut compl_list;
        let mut first: *mut c_char = ptr::null_mut();
        let mut len: c_int = -1;
        for compl in &mut *list {
            let leader = get_leader_for_startcol(compl, true);

            if ctrl_x_mode_normal()
                && p_inf == 0
                && !compl_orig_text.data.is_null()
                && ignorecase(compl_orig_text.data) == 0
            {
                compl.cp_flags &= !CP_ICASE;
            }

            if !match_at_original_text(compl)
                && ((*leader).data.is_null()
                    || ins_compl_equal_sc(compl, (*leader).data, (*leader).size))
            {
                let mut match_limit_exceeded = false;
                let cur_source = compl.cp_cpt_source_idx;

                if cur_source != -1 {
                    match_count[cur_source as usize] += 1;
                    let max_matches = cpt_sources_array[cur_source as usize].cs_max_matches;
                    if max_matches > 0 && match_count[cur_source as usize] > max_matches {
                        match_limit_exceeded = true;
                    }
                }

                if !match_limit_exceeded
                    && (!curbuf_only
                        || (cur_source != -1
                            && cpt_sources_array[cur_source as usize].cs_flag == b'.' as c_char))
                {
                    if first.is_null()
                        && strncmp(
                            ins_compl_leader(),
                            compl.cp_str.data,
                            ins_compl_leader_len(),
                        ) == 0
                    {
                        first = compl.cp_str.data;
                        len = strlen(first) as c_int;
                    } else if !first.is_null() {
                        let mut j: c_int = 0;
                        let mut s1 = first;
                        let mut s2 = compl.cp_str.data;

                        while j < len && *s1 != NUL && *s2 != NUL {
                            let l1 = utf_byte2len(c_int::from(*s1 as u8));
                            let l2 = utf_byte2len(c_int::from(*s2 as u8));
                            if l1 != l2 || memcmp(s1.cast(), s2.cast(), l1 as usize) != 0 {
                                break;
                            }

                            j += l1;
                            mb_ptr_adv(&mut s1);
                            mb_ptr_adv(&mut s2);
                        }
                        len = j;

                        if len == 0 {
                            break;
                        }
                    }
                }
            }
        }

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

pub unsafe fn ins_compl_insert(move_cursor: bool, insert_prefix: bool) {
    unsafe {
        let compl_len = get_compl_len();
        let preinsert = ins_compl_has_preinsert();
        let shown = compl_opt(compl_shown_idx);
        let mut cp_str = (*shown).cp_str.data;
        let mut cp_str_len = (*shown).cp_str.size;
        let leader_len = ins_compl_leader_len();
        let has_multiple = !strchr(cp_str, b'\n' as c_int).is_null();

        let cpt_sources = &raw const cpt_sources_array;
        if insert_prefix {
            cp_str = find_common_prefix(&raw mut cp_str_len, false);
            if cp_str.is_null() {
                cp_str = find_common_prefix(&raw mut cp_str_len, true);
                if cp_str.is_null() {
                    let shown = compl_opt(compl_shown_idx);
                    cp_str = (*shown).cp_str.data;
                    cp_str_len = (*shown).cp_str.size;
                }
            }
        } else if !(*cpt_sources).is_empty() {
            let cpt_idx = (*shown).cp_cpt_source_idx;
            if cpt_idx >= 0 && compl_col >= 0 {
                let startcol = cpt_sources_array[cpt_idx as usize].cs_startcol;
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
        let shown = compl_opt(compl_shown_idx);
        compl_used_match = !(match_at_original_text(shown) || (preinsert && !insert_prefix));

        let dict = ins_compl_dict_alloc(shown);
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

        let shown = compl_opt(compl_shown_idx);
        let mut s = (*shown).cp_fname;
        let mut e = s;
        while *e != NUL {
            space -= ptr2cells(e);
            while space < 0 {
                space += ptr2cells(s);
                mb_ptr_adv(&mut s);
            }
            mb_ptr_adv(&mut e);
        }
        if !compl_autocomplete {
            msg_hist_off = true;
            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                c"%s %s%s".as_ptr(),
                lead,
                if s > (*shown).cp_fname {
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

unsafe fn find_next_match_in_menu() -> Option<usize> {
    unsafe {
        let is_forward = compl_shows_dir_forward();
        let mut match_idx = compl_shown_idx?;

        loop {
            let next_opt = if is_forward {
                compl_next_idx(match_idx)
            } else {
                compl_prev_idx(match_idx)
            };
            let Some(ni) = next_opt else {
                break;
            };
            match_idx = ni;
            let m = compl_at(match_idx);
            if compl_next_idx(match_idx).is_none()
                || (*m).cp_in_match_array
                || match_at_original_text(m)
            {
                break;
            }
        }
        Some(match_idx)
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
        let mut found_compl: Option<usize> = None;
        let cur_cot_flags = get_cot_flags();
        let compl_no_select = (cur_cot_flags & kOptCotFlagNoselect) != 0
            || (compl_autocomplete && !ins_compl_has_preinsert());

        loop {
            todo -= 1;
            if todo < 0 {
                break;
            }
            let arr = &raw const compl_match_array;
            let shown_idx = compl_shown_idx.unwrap();
            if compl_shows_dir_forward() && compl_next_idx(shown_idx).is_some() {
                if !(*arr).is_empty() {
                    compl_shown_idx = find_next_match_in_menu();
                } else {
                    compl_shown_idx = compl_next_idx(shown_idx);
                }
                let new_shown = compl_shown_idx.unwrap();
                found_end = !(&raw const compl_list).as_ref().unwrap().is_empty()
                    && (compl_next_idx(new_shown) == Some(0) || new_shown == 0);
            } else if compl_shows_dir_backward() && compl_prev_idx(shown_idx).is_some() {
                found_end = shown_idx == 0;
                if !(*arr).is_empty() {
                    compl_shown_idx = find_next_match_in_menu();
                } else {
                    compl_shown_idx = compl_prev_idx(shown_idx);
                }
                found_end |= compl_shown_idx == Some(0);
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
                    let cur_shown = compl_shown_idx.unwrap();
                    if compl_pending > 0
                        && let Some(ni) = compl_next_idx(cur_shown)
                    {
                        compl_shown_idx = Some(ni);
                        compl_pending -= 1;
                    } else if compl_pending < 0
                        && let Some(pi) = compl_prev_idx(cur_shown)
                    {
                        compl_shown_idx = Some(pi);
                        compl_pending += 1;
                    } else {
                        break;
                    }
                }
                found_end = false;
            }

            let shown = compl_opt(compl_shown_idx);
            let leader = get_leader_for_startcol(shown, false);

            if !match_at_original_text(shown)
                && !(*leader).data.is_null()
                && !ins_compl_equal(shown, (*leader).data, (*leader).size)
                && !(cot_fuzzy() && (*shown).cp_score != FUZZY_SCORE_NONE)
            {
                todo += 1;
            } else {
                found_compl = compl_shown_idx;
            }

            if found_end {
                if found_compl.is_some() {
                    compl_shown_idx = found_compl;
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

        if { compl_shown_idx }.is_none() {
            return -1;
        }

        if !compl_leader.data.is_null()
            && !match_at_original_text(compl_opt(compl_shown_idx))
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
                let preinsert_longest = ins_compl_preinsert_longest()
                    && match_at_original_text(compl_opt(compl_shown_idx));
                ins_compl_insert(compl_preinsert || preinsert_longest, preinsert_longest);
            } else {
                debug_assert!(!compl_leader.data.is_null());
                ins_compl_insert_bytes(compl_leader.data.add(get_compl_len() as usize), -1);
            }
            let shown = compl_opt(compl_shown_idx);
            if strequal((*shown).cp_str.data, compl_orig_text.data) {
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

        let shown = compl_opt(compl_shown_idx);
        if compl_no_insert && !started && !match_at_original_text(shown) {
            compl_enter_selects = true;
        } else {
            let arr = &raw const compl_match_array;
            compl_enter_selects = !insert_match && !(*arr).is_empty();
        }

        if !(*shown).cp_fname.is_null() {
            ins_compl_show_filename();
        }

        num_matches
    }
}

unsafe fn check_elapsed_time() {
    unsafe {
        let start_tv = cpt_sources_array[cpt_sources_index as usize].compl_start_tv;
        let elapsed_ms = (os_hrtime() - start_tv) / 1_000_000;

        if elapsed_ms > compl_timeout_ms {
            compl_time_slice_expired = true;
            decay_compl_timeout();
        }
    }
}

pub unsafe fn ins_compl_check_keys(frequency: c_int, in_compl_func: bool) {
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
            let cpt_sources = &raw const cpt_sources_array;
            let normal_mode_strict = ctrl_x_mode_normal()
                && !ctrl_x_mode_line_or_eval()
                && (compl_cont_status & CONT_LOCAL) == 0
                && !(*cpt_sources).is_empty()
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
        } else {
            startcol -= 1;
            if startcol < 0 || !vim_iswordp(mb_prevptr(line, line.add((startcol + 1) as usize))) {
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
            if (*cb).cb_type() == kCallbackNone {
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

        let mut args: [typval_T; 3] = [typval_T::new(); 3];
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
        let extmarks = &raw mut compl_orig_extmarks;
        (*extmarks).clear();
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
            let extmarks = &raw mut compl_orig_extmarks;
            (*extmarks).clear();
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
        if compl_next_idx(0) == Some(0) {
            edit_submode_extra = if compl_status_adding() && compl_length > 1 {
                gettext(c"Hit end of paragraph".as_ptr())
            } else {
                gettext(c"Pattern not found".as_ptr())
            };
            edit_submode_highl = HLF_E;
        }

        if edit_submode_extra.is_null() {
            let curr_idx = compl_curr_idx.unwrap();
            let curr = compl_at(curr_idx);
            if match_at_original_text(curr) {
                edit_submode_extra = gettext(c"Back at original".as_ptr());
                edit_submode_highl = HLF_W;
            } else if (compl_cont_status & CONT_S_IPOS) != 0 {
                edit_submode_extra = gettext(c"Word from other line".as_ptr());
                edit_submode_highl = HLF_COUNT;
            } else if compl_next_idx(curr_idx) == compl_prev_idx(curr_idx) {
                edit_submode_extra = gettext(c"The only match".as_ptr());
                edit_submode_highl = HLF_COUNT;
                (*curr).cp_number = 1;
            } else {
                if (*curr).cp_number == -1 {
                    ins_compl_update_sequence_numbers();
                }

                let curr = compl_at(curr_idx);
                if (*curr).cp_number != -1 {
                    let match_ref_ptr = (&raw mut match_ref).cast::<c_char>();
                    if compl_matches > 0 {
                        vim_snprintf!(
                            match_ref_ptr,
                            81,
                            gettext(c"match %d of %d".as_ptr()),
                            (*curr).cp_number,
                            compl_matches,
                        );
                    } else {
                        vim_snprintf!(
                            match_ref_ptr,
                            81,
                            gettext(c"match %d".as_ptr()),
                            (*curr).cp_number,
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

pub unsafe fn ins_complete(c: c_int, enable_pum: bool) -> c_int {
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
        compl_shown_idx = compl_curr_idx;
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
        compl_curr_idx = compl_shown_idx;
        compl_direction = compl_shows_dir;

        if got_int && global_busy == 0 {
            let _ = vgetc();
            got_int = false;
        }

        let no_matches_found = compl_next_idx(0) == Some(0);
        if no_matches_found
            && (compl_length > 1
                || compl_status_adding()
                || (ctrl_x_mode_not_default()
                    && !ctrl_x_mode_path_patterns()
                    && !ctrl_x_mode_path_defines()))
        {
            compl_cont_status &= !CONT_N_ADDS;
        }

        let curr = compl_opt(compl_curr_idx);
        if ((*curr).cp_flags & CP_CONT_S_IPOS) != 0 {
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

pub unsafe fn ins_compl_enable_autocomplete() {
    unsafe {
        compl_autocomplete = true;
        compl_get_longest = false;
    }
}

pub unsafe fn ins_compl_disable_autocomplete() {
    unsafe {
        compl_autocomplete = false;
    }
}

pub unsafe fn ins_compl_arm_autostart() {
    unsafe {
        compl_autostart_pending = true;
    }
}

pub unsafe fn ins_compl_disarm_autostart() {
    unsafe {
        compl_autostart_pending = false;
    }
}

pub unsafe fn ins_compl_arm_autocomplete_delay() -> bool {
    unsafe {
        if p_acl > 0 {
            compl_autocomplete_start_tv = os_hrtime();
            compl_autocomplete_pending = true;
            return true;
        }
        false
    }
}

pub unsafe fn ins_compl_clear_autocomplete_delay() {
    unsafe {
        compl_autocomplete_pending = false;
    }
}

pub unsafe fn ins_compl_autocomplete_pending() -> bool {
    unsafe { compl_autocomplete_pending }
}

pub unsafe fn ins_compl_autocomplete_elapsed() -> i64 {
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

pub unsafe fn free_insexpand_stuff() {
    unsafe {
        api_clear_string(&raw mut compl_orig_text);
        let extmarks = &raw mut compl_orig_extmarks;
        *extmarks = Vec::new();
        callback_free(&raw mut p_cfu);
        callback_free(&raw mut p_ofu);
        callback_free(&raw mut p_tsrfu);
        let global_cb = &raw mut cpt_cb;
        clear_cpt_callbacks(&mut *global_cb);
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
        cpt_sources_array = Vec::new();
        cpt_sources_index = -1;
    }
}

unsafe fn setup_cpt_sources() {
    unsafe {
        cpt_sources_clear();

        let count = get_cpt_sources_count((*curbuf).b_p_cpt);
        if count == 0 {
            return;
        }

        let mut sources = vec![
            cpt_source_T {
                cs_startcol: 0,
                cs_max_matches: 0,
                compl_start_tv: 0,
                cs_flag: 0,
                cs_refresh_always: false,
            };
            count as usize
        ];

        let mut buf: [c_char; LSIZE] = [0; LSIZE];
        let mut idx: usize = 0;
        let mut p = (*curbuf).b_p_cpt;
        while *p != NUL {
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
            if *p != NUL {
                sources[idx].cs_flag = *p;
                buf.fill(0);
                let slen = copy_option_part(&raw mut p, buf.as_mut_ptr(), LSIZE, c",".as_ptr());
                if slen > 0 {
                    let caret = vim_strchr(buf.as_mut_ptr(), b'^' as c_int);
                    if !caret.is_null() {
                        sources[idx].cs_max_matches = atoi(caret.add(1));
                    }
                }
                idx += 1;
            }
        }

        cpt_sources_array = sources;
    }
}

unsafe fn is_cpt_func_refresh_always() -> bool {
    unsafe {
        let cpt_sources = &raw const cpt_sources_array;
        (*cpt_sources).iter().any(|s| s.cs_refresh_always)
    }
}

unsafe fn ins_compl_make_linear() {
    unsafe {
        compl_is_cyclic = false;
    }
}

unsafe fn remove_old_matches() {
    unsafe {
        let list = compl_list_mut();
        if list.is_empty() || cpt_sources_index < 0 {
            return;
        }
        let forward = list[0].cp_cpt_source_idx < 0;
        compl_direction = if forward { FORWARD } else { BACKWARD };
        compl_shows_dir = compl_direction;

        let shown_ptr = compl_shown_idx.map(|i| list[i].cp_str.data);
        let preselect_ptr = compl_preselect_idx.map(|i| list[i].cp_str.data);
        let old_ptr = compl_old_idx.map(|i| list[i].cp_str.data);

        let mut i = 0;
        while i < list.len() {
            if list[i].cp_cpt_source_idx == cpt_sources_index {
                list.remove(i);
            } else {
                i += 1;
            }
        }

        compl_preselect_idx =
            preselect_ptr.and_then(|p| list.iter().position(|m| ptr::eq(m.cp_str.data, p)));
        compl_old_idx = old_ptr.and_then(|p| list.iter().position(|m| ptr::eq(m.cp_str.data, p)));

        if let Some(sp) = shown_ptr
            && let Some(new_shown) = list.iter().position(|m| ptr::eq(m.cp_str.data, sp))
        {
            compl_shown_idx = Some(new_shown);
        } else if !list.is_empty() {
            compl_shown_idx = Some(if forward { 0 } else { list.len() - 1 });
        } else {
            compl_shown_idx = None;
        }

        compl_curr_idx = if list.is_empty() { None } else { Some(0) };
        for idx in 0..list.len() {
            let cond = if forward {
                list[idx].cp_cpt_source_idx < cpt_sources_index
            } else {
                list[idx].cp_cpt_source_idx > cpt_sources_index
            };
            if cond {
                compl_curr_idx = if forward {
                    Some(idx)
                } else if idx + 1 < list.len() {
                    Some(idx + 1)
                } else {
                    None
                };
            } else {
                break;
            }
        }
    }
}

unsafe fn get_cpt_func_completion_matches(cb: *mut Callback) {
    unsafe {
        let cpt_src = &raw mut cpt_sources_array[cpt_sources_index as usize];
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

            if cpt_sources_array[cpt_sources_index as usize].cs_refresh_always {
                let cb = get_callback_if_cpt_func(p, cpt_sources_index);
                if !cb.is_null() {
                    remove_old_matches();
                    let mut startcol: c_int = 0;
                    let ret =
                        get_userdefined_compl_info((*curwin).w_cursor.col, cb, &raw mut startcol);
                    if ret == FAIL {
                        if startcol == -3 {
                            cpt_sources_array[cpt_sources_index as usize].cs_refresh_always = false;
                        } else {
                            startcol = -2;
                        }
                    } else if startcol < 0 || startcol > (*curwin).w_cursor.col {
                        startcol = (*curwin).w_cursor.col;
                    }
                    cpt_sources_array[cpt_sources_index as usize].cs_startcol = startcol;
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

pub unsafe fn f_preinserted(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
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
