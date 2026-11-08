//! Functions for Insert mode completion (`src/nvim/insexpand.c`).

use crate::api::private::helpers::{
    HLF_E, api_clear_string, cbuf_to_string, copy_string, static_cstr_as_string,
};
use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_COMPLETECHANGED, EVENT_COMPLETEDONE, EVENT_COMPLETEDONEPRE, apply_autocmds, has_event,
};
use crate::buffer::{FileBuf, K_UIMESSAGES, MAXLNUM, VAR_FIXED, bt_cmdwin, buf_spname, buf_valid};
use crate::change::{
    HLF_W, MB_MAXCHAR, OPENLINE_FORCE_INDENT, OPENLINE_KEEPTRAIL, deleted_lines_mark,
    ins_bytes_len, ins_char, ins_char_bytes, ins_str, open_line,
};
use crate::charset::{
    ascii_iswhite, ascii_iswhite_or_nul, getwhitecols, str_foldcase, vim_is_idc, vim_isfilec,
    vim_isprintc, vim_iswordc, vim_iswordp, vim_strsize,
};
use crate::cmdexpand::{
    CTRL_N, CTRL_P, EXPAND_FILES, EXPAND_LUA, EXPAND_NOTHING, EXPAND_OK, EXPAND_UNSUCCESSFUL,
    RE_LAST, SEARCH_NFMSG, TAG_MANY, addstar, expand_cmdline, free_xp_files_extra, set_cmd_context,
};
use crate::cursor::{
    Colnr, CursorPos, Linenr, check_cursor, dec_cursor, equalpos, get_cursor_line_len,
    get_cursor_line_slice, get_cursor_pos_slice, inc_cursor,
};
use crate::drawscreen::{
    K_SHM_COMPLETIONMENU, conceal_cursor_line, redraw_winline, setcursor, showmode, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Callback, EvalFuncData, TvDict, TvList, TypVal, VarType, Varnumber, callback_copy,
    callback_free, tv_clear, tv_dict_add_bool, tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_add_str_len, tv_dict_add_tv, tv_dict_alloc, tv_dict_alloc_lock,
    tv_dict_alloc_ret, tv_dict_get_number, tv_dict_get_string, tv_dict_get_tv,
    tv_dict_set_keys_readonly, tv_dict_unref, tv_get_number_chk, tv_get_string, tv_get_string_chk,
    tv_list_alloc, tv_list_append_dict, tv_list_unref,
};
use crate::eval::userfunc::callback_call_retnr;
use crate::eval::vars::{VV_COMPLETED_ITEM, set_vim_var_dict};
use crate::eval::{
    K_LIST_LEN_MAY_KNOW_CONST, callback_call, get_v_event, restore_v_event, set_ref_in_callback,
};
use crate::ex_eval::aborting;
use crate::ex_getln::{
    CTRL_D_CONST, CTRL_F_CONST, CTRL_K_CONST, CTRL_L_CONST, CTRL_O, CTRL_Q_CONST, CTRL_R_CONST,
    CTRL_RSB_CONST, CTRL_T_CONST, CTRL_U, CTRL_Z_CONST, K_EVENT, K_LUA, K_SELECT, tilde_replace,
};
use crate::extmark::{ExtmarkUndoObject, extmark_apply_undo, extmark_splice_delete};
use crate::fileio::vim_fgets;
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzzy_match_str, fuzzy_match_str_in_line, search_for_fuzzy_match,
};
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2attr, syn_id2name, syn_name2attr};
use crate::indent::{get_indent, inindent};
use crate::indent_c::{KEY_COMPLETE, do_c_expr_indent, in_cinkeys};
use crate::input::{
    K_INS_NONE, redo_append_char, redo_append_lit, safe_vgetc, using_script, vgetc, vpeekc,
    vpeekc_any, vungetc,
};
use crate::insert::{
    BS_START, CTRL_S_CONST, CTRL_X_CONST, backspace_until_column, get_can_cindent,
    ins_apply_autocmds, ins_eol, ins_redraw, insertchar, start_arrow, stop_arrow,
};
use crate::keycodes::{
    CAR, K_BS, K_COMMAND, K_DOWN, K_IGNORE, K_KENTER, K_KPAGEDOWN, K_KPAGEUP, K_MOUSEDOWN,
    K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP, K_PAGEDOWN, K_PAGEUP, K_S_DOWN, K_S_TAB,
    K_S_UP, K_UP, NL, TAB,
};
use crate::lua::executor::nlua_expand_pat;
use crate::mbyte::{
    mb_get_class, mb_islower, mb_isupper, mb_ptr2char_adv, mb_tolower, mb_toupper, ptr2cells,
    utf_char2bytes, utf_char2len, utf_ptr2char, utf_ptr2len, utf_slice2char, utfc_ptr2len,
};
use crate::memline::{dec, ml_delete, ml_get, ml_get_len};
use crate::message::{
    emsg, internal_error, msg, msg_clr_cmdline, msg_delay, msg_ext_set_kind, msg_progress, semsg,
};
use crate::r#move::{changed_cline_bef_curs, curs_columns, validate_cursor};
use crate::option::{
    OptInt, can_bs, copy_option_part, option_set_callback_func, shortmess, was_set_insecurely,
};
use crate::option_vars::*;
use crate::optionstr::{OptIndex, Optset};
use crate::os::env::ExpandCtx;
use crate::os::fs::CFile;
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::os::time::os_hrtime;
use crate::path::{
    EW_ADDSLASH, EW_DIR, EW_FILE, EW_PATH, EW_SILENT, PATHSEP, expand_wildcards_vec, path_tail,
};
#[cfg(windows)]
use crate::path::{path_has_drive_letter, path_to_slash};
use crate::popupmenu::{
    Pumitem, pum_clear, pum_display, pum_get_height, pum_set_event_info, pum_undisplay, pum_visible,
};
use crate::regexp::{CTRL_V_CONST, MAXCOL, RE_MAGIC, Regmatch, vim_regcomp, vim_regexec};
use crate::register::{NUM_REGISTERS, copy_register, get_register_name, valid_yank_reg};
use crate::search::{Direction, find_pattern_in_path, ignorecase, search_for_exact_line, searchit};
use crate::spell::{
    expand_spelling, spell_dump_compl, spell_expand_check_cap, spell_move_to, spell_word_start,
};
use crate::state::{MODE_INSERT, REPLACE_FLAG, SaveVEvent, may_trigger_modechanged};
use crate::strings::{format_to_buf, gettext, vim_strchr, vim_strsave_escaped};
use crate::tag::find_tags;
use crate::textformat::auto_format;
use crate::ui::{ui_flush, ui_has, vim_beep};
use crate::undo::undo_allowed;
use crate::window::{WinInfoInfo, WinKind, win_valid};
use crate::winfloat::win_float_find;
use std::ffi::CStr;
use std::ptr;

pub const CPT_ABBR: usize = 0;
pub const CPT_KIND: usize = 1;
pub const CPT_MENU: usize = 2;
pub const CPT_INFO: usize = 3;
pub const CPT_COUNT: usize = 4;

const K_DIRECTION_NOT_SET: Direction = 0;

pub(crate) const ACTION_EXPAND: i32 = 5;

pub(crate) const LSIZE: usize = 512;
const MIN_SPACE: i32 = 75;
const LOOKBACK_LINE_COUNT: Linenr = 1000;

pub(crate) const CTRL_I_CONST: i32 = 9;

pub(crate) const K_WIN_INFO: WinKind = 1;

const VAR_UNKNOWN: VarType = VarType::VarUnknown;
const VAR_NUMBER: VarType = VarType::VarNumber;
pub(crate) const VAR_STRING: VarType = VarType::VarString;
const VAR_LIST: VarType = VarType::VarList;
const VAR_DICT: VarType = VarType::VarDict;

pub(crate) const K_OPT_COMPLETE: OptIndex = 51;

const K_OPT_BO_FLAG_COMPLETE: u32 = 0x08;

pub(crate) const K_OPT_COT_FLAG_FUZZY: u32 = 0x01;
const K_OPT_COT_FLAG_LONGEST: u32 = 0x02;
const K_OPT_COT_FLAG_MENU: u32 = 0x04;
const K_OPT_COT_FLAG_MENUONE: u32 = 0x08;
const K_OPT_COT_FLAG_NEAREST: u32 = 0x10;
const K_OPT_COT_FLAG_NOINSERT: u32 = 0x20;
const K_OPT_COT_FLAG_NOSELECT: u32 = 0x40;
const K_OPT_COT_FLAG_NOSORT: u32 = 0x80;
const K_OPT_COT_FLAG_PATHOPT: u32 = 0x100;
const K_OPT_COT_FLAG_PREINSERT: u32 = 0x400;
const K_OPT_COT_FLAG_PRESELECT: u32 = 0x800;

pub(crate) const K_SHM_COMPLETIONSCAN: i32 = b'C' as i32;

pub(crate) const TAG_NOIC: i32 = 8;
pub(crate) const TAG_INS_COMP: i32 = 64;

const CTRL_X_WANT_IDENT: i32 = 0x100;
const CTRL_X_NORMAL: i32 = 0;
const CTRL_X_NOT_DEFINED_YET: i32 = 1;
const CTRL_X_SCROLL: i32 = 2;
const CTRL_X_WHOLE_LINE: i32 = 3;
const CTRL_X_FILES: i32 = 4;
const CTRL_X_TAGS: i32 = 5 + CTRL_X_WANT_IDENT;
const CTRL_X_PATH_PATTERNS: i32 = 6 + CTRL_X_WANT_IDENT;
const CTRL_X_PATH_DEFINES: i32 = 7 + CTRL_X_WANT_IDENT;
const CTRL_X_FINISHED: i32 = 8;
const CTRL_X_DICTIONARY: i32 = 9 + CTRL_X_WANT_IDENT;
const CTRL_X_THESAURUS: i32 = 10 + CTRL_X_WANT_IDENT;
const CTRL_X_CMDLINE: i32 = 11;
const CTRL_X_FUNCTION: i32 = 12;
const CTRL_X_OMNI: i32 = 13;
const CTRL_X_SPELL: i32 = 14;
const CTRL_X_LOCAL_MSG: i32 = 15;
const CTRL_X_EVAL: i32 = 16;
const CTRL_X_CMDLINE_CTRL_X: i32 = 17;
const CTRL_X_BUFNAMES: i32 = 18;
const CTRL_X_REGISTER: i32 = 19;

const CP_ORIGINAL_TEXT: i32 = 1;
const CP_CONT_S_IPOS: i32 = 4;
const CP_EQUAL: i32 = 8;
const CP_ICASE: i32 = 16;
const CP_FAST: i32 = 32;

const COMPL_INITIAL_TIMEOUT_MS: u64 = 80;
const COMPL_MIN_TIMEOUT_MS: u64 = 5;
const COMPL_FUNC_TIMEOUT_MS: u64 = 300;
const COMPL_FUNC_TIMEOUT_NON_KW_MS: u64 = 1000;

const CONT_ADDING: i32 = 1;
const CONT_INTRPT: i32 = 2 + 4;
const CONT_N_ADDS: i32 = 4;
const CONT_S_IPOS: i32 = 8;
const CONT_SOL: i32 = 16;
const CONT_LOCAL: i32 = 32;

const DICT_FIRST: i32 = 1;
const DICT_EXACT: i32 = 2;

const CI_WHAT_MODE: i32 = 0x01;
const CI_WHAT_PUM_VISIBLE: i32 = 0x02;
const CI_WHAT_ITEMS: i32 = 0x04;
const CI_WHAT_SELECTED: i32 = 0x08;
const CI_WHAT_COMPLETED: i32 = 0x10;
const CI_WHAT_MATCHES: i32 = 0x20;
const CI_WHAT_PREINSERTED_TEXT: i32 = 0x40;
const CI_WHAT_AUTO: i32 = 0x80;
const CI_WHAT_ALL: i32 = 0xff;

const INS_COMPL_CPT_OK: i32 = 1;
const INS_COMPL_CPT_CONT: i32 = 2;
const INS_COMPL_CPT_END: i32 = 3;

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
fn ctrl_x_msg(i: i32) -> Option<&'static CStr> {
    let idx = (i & !CTRL_X_WANT_IDENT) as usize;
    CTRL_X_MSGS.get(idx).copied().flatten()
}

#[inline]
const fn empty_nvim_string() -> NvimString {
    NvimString::new()
}

use crate::mbyte::utf_byte2len;
use crate::path::vim_ispathsep;

/// Structure used to store one match for insert completion (`Compl`).
#[repr(C)]
pub struct Compl {
    pub cp_str: NvimString,
    pub cp_text: [Option<std::ffi::CString>; CPT_COUNT],
    pub cp_user_data: TypVal,
    pub cp_fname: Option<std::ffi::CString>,
    pub cp_commit_chars: Option<std::ffi::CString>,
    pub cp_flags: i32,
    pub cp_number: i32,
    pub cp_order: i32,
    pub cp_preselect: bool,
    pub cp_score: i32,
    pub cp_in_match_array: bool,
    pub cp_user_abbr_hl_id: i32,
    pub cp_user_kind_hl_id: i32,
    pub cp_cpt_source_idx: i32,
}

impl Drop for Compl {
    fn drop(&mut self) {
        let ht_ptr = &mut insexp().compl_strings_ht;
        if !ht_ptr.is_empty()
            && !self.cp_str.data.is_null()
            && (self.cp_flags & CP_ORIGINAL_TEXT) == 0
        {
            let key = self.cp_str.cstr_bytes();
            if let Some(count) = ht_ptr.get_mut(key) {
                *count -= 1;
                if *count <= 0 {
                    ht_ptr.remove(key);
                }
            }
        }
        api_clear_string(&mut self.cp_str);
        tv_clear(&mut self.cp_user_data);
    }
}

/// State information used for getting the next set of insert completion matches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CptFuncSource {
    None,
    Ofu,
    Cfu,
    CptCb(usize),
}

impl CptFuncSource {
    fn as_mut(self) -> Option<&'static mut Callback> {
        match self {
            Self::None => None,
            Self::Ofu => Some(&mut curbuf_mut().b_p_ofu),
            Self::Cfu => Some(&mut curbuf_mut().b_p_cfu),
            Self::CptCb(idx) => curbuf_mut().b_p_cpt_cb.get_mut(idx),
        }
    }
}

struct InsComplNextState {
    e_cpt_copy: Vec<u8>,
    e_cpt_idx: usize,
    ins_buf: crate::globals::BufPtr,
    cur_match_forward: bool,
    prev_match_pos: CursorPos,
    set_match_pos: bool,
    first_match_pos: CursorPos,
    last_match_pos: CursorPos,
    found_all: bool,
    dict: Option<std::ffi::CString>,
    dict_f: i32,
    func_cb: CptFuncSource,
}

impl InsComplNextState {
    const fn new() -> Self {
        Self {
            e_cpt_copy: Vec::new(),
            e_cpt_idx: 0,
            ins_buf: crate::globals::BufPtr::NULL,
            cur_match_forward: false,
            prev_match_pos: CursorPos::new(),
            set_match_pos: false,
            first_match_pos: CursorPos::new(),
            last_match_pos: CursorPos::new(),
            found_all: false,
            dict: None,
            dict_f: 0,
            func_cb: CptFuncSource::None,
        }
    }

    #[inline]
    fn e_cpt_char(&self) -> u8 {
        self.e_cpt_copy.get(self.e_cpt_idx).copied().unwrap_or(0)
    }

    #[inline]
    fn e_cpt_cstr(&self) -> &CStr {
        crate::memory::bytes_as_cstr(&self.e_cpt_copy[self.e_cpt_idx..])
    }

    #[inline]
    fn cur_match_pos(&self) -> CursorPos {
        if self.cur_match_forward {
            self.last_match_pos
        } else {
            self.first_match_pos
        }
    }

    #[inline]
    fn cur_match_pos_ptr(&mut self) -> &mut CursorPos {
        if self.cur_match_forward {
            &mut self.last_match_pos
        } else {
            &mut self.first_match_pos
        }
    }
}

impl Default for InsComplNextState {
    fn default() -> Self {
        Self::new()
    }
}

/// Structure for completion source (in `'cpt'` option) information (`CptSource`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CptSource {
    cs_refresh_always: bool,
    cs_startcol: i32,
    cs_max_matches: i32,
    compl_start_tv: u64,
    cs_flag: u8,
}

use crate::charset::NUL;
use crate::drawline::{HLF_COUNT, SMT_ALL};
use crate::eval::funcs::SEARCH_KEEP;
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_cmds::{CTRL_C_CONST, CTRL_E, CTRL_Y, HLF_R};
use crate::ex_docmd::{FIND_ANY, FIND_DEFINE};
use crate::extmark::K_EXTMARK_UNDO;
use crate::globals::IOSIZE;
use crate::help::{TAG_NAMES, TAG_REGEXP, TAG_VERBOSE};
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::search::{BACKWARD, FORWARD};

struct InsexpandState {
    compl_list: Vec<Compl>,
    compl_is_cyclic: bool,
    compl_curr_idx: Option<usize>,
    compl_shown_idx: Option<usize>,
    compl_old_idx: Option<usize>,
    compl_preselect_idx: Option<usize>,
    compl_strings_ht: std::collections::BTreeMap<Vec<u8>, i32>,
    compl_num_bests: i32,
    compl_enter_selects: bool,
    compl_leader: NvimString,
    compl_get_longest: bool,
    compl_used_match: bool,
    compl_was_interrupted: bool,
    compl_interrupted: bool,
    compl_restarting: bool,
    compl_started: bool,
    ctrl_x_mode: i32,
    compl_matches: i32,
    compl_pattern: Option<std::ffi::CString>,
    cpt_compl_pattern: NvimString,
    compl_direction: Direction,
    compl_shows_dir: Direction,
    compl_pending: i32,
    compl_startpos: CursorPos,
    compl_length: i32,
    compl_lnum: Linenr,
    compl_col: Colnr,
    compl_ins_end_col: Colnr,
    compl_longest_end_col: Colnr,
    compl_orig_text: NvimString,
    compl_orig_extmarks: Vec<ExtmarkUndoObject>,
    compl_cont_mode: i32,
    compl_xp: ExpandCtx,
    compl_curr_win: crate::globals::WinPtr,
    compl_curr_buf: crate::globals::BufPtr,
    compl_autocomplete: bool,
    compl_autostarted: bool,
    compl_autostart_pending: bool,
    compl_autocomplete_pending: bool,
    compl_autocomplete_start_tv: u64,
    compl_timeout_ms: u64,
    compl_time_slice_expired: bool,
    compl_from_nonkeyword: bool,
    compl_hi_on_autocompl_longest: bool,
    compl_cont_status: i32,
    compl_opt_refresh_always: bool,
    spell_bad_len: usize,
    compl_selected_item: i32,
    cpt_sources_array: Vec<CptSource>,
    cpt_sources_index: i32,
    compl_match_array: Vec<Pumitem>,
    cpt_cb: Vec<Callback>,
    adjusted_leader: NvimString,
    next_buf_wp: crate::globals::WinPtr,
    next_state: InsComplNextState,
    next_state_cleared: bool,
    match_ref: [u8; 81],
}

impl InsexpandState {
    const fn new() -> Self {
        Self {
            compl_list: Vec::new(),
            compl_is_cyclic: false,
            compl_curr_idx: None,
            compl_shown_idx: None,
            compl_old_idx: None,
            compl_preselect_idx: None,
            compl_strings_ht: std::collections::BTreeMap::new(),
            compl_num_bests: 0,
            compl_enter_selects: false,
            compl_leader: empty_nvim_string(),
            compl_get_longest: false,
            compl_used_match: false,
            compl_was_interrupted: false,
            compl_interrupted: false,
            compl_restarting: false,
            compl_started: false,
            ctrl_x_mode: CTRL_X_NORMAL,
            compl_matches: 0,
            compl_pattern: None,
            cpt_compl_pattern: empty_nvim_string(),
            compl_direction: FORWARD,
            compl_shows_dir: FORWARD,
            compl_pending: 0,
            compl_startpos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            compl_length: 0,
            compl_lnum: 0,
            compl_col: 0,
            compl_ins_end_col: 0,
            compl_longest_end_col: 0,
            compl_orig_text: empty_nvim_string(),
            compl_orig_extmarks: Vec::new(),
            compl_cont_mode: 0,
            compl_xp: ExpandCtx::new(),
            compl_curr_win: crate::globals::WinPtr::NULL,
            compl_curr_buf: crate::globals::BufPtr::NULL,
            compl_autocomplete: false,
            compl_autostarted: false,
            compl_autostart_pending: false,
            compl_autocomplete_pending: false,
            compl_autocomplete_start_tv: 0,
            compl_timeout_ms: COMPL_INITIAL_TIMEOUT_MS,
            compl_time_slice_expired: false,
            compl_from_nonkeyword: false,
            compl_hi_on_autocompl_longest: false,
            compl_cont_status: 0,
            compl_opt_refresh_always: false,
            spell_bad_len: 0,
            compl_selected_item: -1,
            cpt_sources_array: Vec::new(),
            cpt_sources_index: -1,
            compl_match_array: Vec::new(),
            cpt_cb: Vec::new(),
            adjusted_leader: empty_nvim_string(),
            next_buf_wp: crate::globals::WinPtr::NULL,
            next_state: InsComplNextState::new(),
            next_state_cleared: false,
            match_ref: [0; 81],
        }
    }
}

static INSEXPAND: GlobalState<InsexpandState> = GlobalState::new(InsexpandState::new());

#[inline]
fn insexp() -> crate::globals::GraphPtr<InsexpandState> {
    INSEXPAND.handle()
}

#[inline]
fn compl_next_idx(idx: usize) -> Option<usize> {
    let len = insexp().compl_list.len();
    if idx + 1 < len {
        Some(idx + 1)
    } else if insexp().compl_is_cyclic && len > 0 {
        Some(0)
    } else {
        None
    }
}

#[inline]
fn compl_prev_idx(idx: usize) -> Option<usize> {
    let len = insexp().compl_list.len();
    if idx > 0 {
        Some(idx - 1)
    } else if insexp().compl_is_cyclic && len > 0 {
        Some(len - 1)
    } else {
        None
    }
}

#[inline]
fn compl_list_mut() -> &'static mut Vec<Compl> {
    &mut insexp().borrow_mut().compl_list
}

#[inline]
fn compl_at(idx: usize) -> &'static mut Compl {
    &mut compl_list_mut()[idx]
}

#[inline]
fn compl_opt(idx: Option<usize>) -> Option<&'static mut Compl> {
    idx.map(compl_at)
}

#[inline]
fn compl_pattern_cstr() -> Option<&'static CStr> {
    insexp().borrow_mut().compl_pattern.as_deref()
}

#[inline]
fn compl_pattern_len() -> usize {
    insexp()
        .compl_pattern
        .as_ref()
        .map_or(0, |s| s.to_bytes().len())
}

#[inline]
fn set_compl_pattern_bytes(bytes: &[u8]) {
    insexp().compl_pattern = Some(crate::memory::bytes_to_cstring(
        crate::memory::bytes_until_nul(bytes),
    ));
}

#[inline]
fn decay_compl_timeout() {
    if insexp().compl_timeout_ms > COMPL_MIN_TIMEOUT_MS {
        insexp().compl_timeout_ms /= 2;
    }
}

/// Count the string of a new match in the duplicate-check hashtab.
fn compl_strings_add(bytes: &[u8]) {
    let key = crate::memory::bytes_until_nul(bytes);
    let ht_ptr = &mut insexp().compl_strings_ht;
    *ht_ptr.entry(key.to_vec()).or_insert(0) += 1;
}

/// CTRL-X pressed in Insert mode.
pub fn ins_ctrl_x() {
    if !ctrl_x_mode_cmdline() {
        if (insexp().compl_cont_status & CONT_N_ADDS) != 0 {
            insexp().compl_cont_status |= CONT_INTRPT;
        } else {
            insexp().compl_cont_status = 0;
        }
        insexp().ctrl_x_mode = CTRL_X_NOT_DEFINED_YET;
        g_st().edit_submode = Some(gettext((ctrl_x_msg(insexp().ctrl_x_mode)).unwrap_or(c"")));
        g_st().edit_submode_pre = None;
        g_st().redraw_mode = true;
    } else {
        insexp().ctrl_x_mode = CTRL_X_CMDLINE_CTRL_X;
    }

    may_trigger_modechanged();
}

pub fn ctrl_x_mode_none() -> bool {
    insexp().ctrl_x_mode == 0
}

pub fn ctrl_x_mode_normal() -> bool {
    insexp().ctrl_x_mode == CTRL_X_NORMAL
}

pub fn ctrl_x_mode_scroll() -> bool {
    insexp().ctrl_x_mode == CTRL_X_SCROLL
}

pub fn ctrl_x_mode_whole_line() -> bool {
    insexp().ctrl_x_mode == CTRL_X_WHOLE_LINE
}

pub fn ctrl_x_mode_files() -> bool {
    insexp().ctrl_x_mode == CTRL_X_FILES
}

pub fn ctrl_x_mode_tags() -> bool {
    insexp().ctrl_x_mode == CTRL_X_TAGS
}

pub fn ctrl_x_mode_path_patterns() -> bool {
    insexp().ctrl_x_mode == CTRL_X_PATH_PATTERNS
}

pub fn ctrl_x_mode_path_defines() -> bool {
    insexp().ctrl_x_mode == CTRL_X_PATH_DEFINES
}

pub fn ctrl_x_mode_dictionary() -> bool {
    insexp().ctrl_x_mode == CTRL_X_DICTIONARY
}

pub fn ctrl_x_mode_thesaurus() -> bool {
    insexp().ctrl_x_mode == CTRL_X_THESAURUS
}

pub fn ctrl_x_mode_cmdline() -> bool {
    insexp().ctrl_x_mode == CTRL_X_CMDLINE || insexp().ctrl_x_mode == CTRL_X_CMDLINE_CTRL_X
}

pub fn ctrl_x_mode_function() -> bool {
    insexp().ctrl_x_mode == CTRL_X_FUNCTION
}

pub fn ctrl_x_mode_omni() -> bool {
    insexp().ctrl_x_mode == CTRL_X_OMNI
}

pub fn ctrl_x_mode_spell() -> bool {
    insexp().ctrl_x_mode == CTRL_X_SPELL
}

pub fn ctrl_x_mode_eval() -> bool {
    insexp().ctrl_x_mode == CTRL_X_EVAL
}

pub fn ctrl_x_mode_line_or_eval() -> bool {
    insexp().ctrl_x_mode == CTRL_X_WHOLE_LINE || insexp().ctrl_x_mode == CTRL_X_EVAL
}

pub fn ctrl_x_mode_register() -> bool {
    insexp().ctrl_x_mode == CTRL_X_REGISTER
}

pub fn ctrl_x_mode_not_default() -> bool {
    insexp().ctrl_x_mode != CTRL_X_NORMAL
}

pub fn ctrl_x_mode_not_defined_yet() -> bool {
    insexp().ctrl_x_mode == CTRL_X_NOT_DEFINED_YET
}

pub fn compl_status_adding() -> bool {
    (insexp().compl_cont_status & CONT_ADDING) != 0
}

pub fn compl_status_sol() -> bool {
    (insexp().compl_cont_status & CONT_SOL) != 0
}

pub fn compl_status_local() -> bool {
    (insexp().compl_cont_status & CONT_LOCAL) != 0
}

pub fn compl_status_clear() {
    insexp().compl_cont_status = 0;
}

#[inline]
fn compl_dir_forward() -> bool {
    insexp().compl_direction == FORWARD
}

#[inline]
fn compl_shows_dir_forward() -> bool {
    insexp().compl_shows_dir == FORWARD
}

#[inline]
fn compl_shows_dir_backward() -> bool {
    insexp().compl_shows_dir == BACKWARD
}

/// Check that the 'dictionary' or 'thesaurus' option can be used.
pub fn check_compl_option(dict_opt: bool) -> bool {
    let empty = if dict_opt {
        curbuf_mut().b_p_dict.is_empty()
            && opt_vars().p_dict.is_empty()
            && !curwin_mut().w_onebuf_opt.wo_spell
    } else {
        curbuf_mut().b_p_tsr.is_empty()
            && opt_vars().p_tsr.is_empty()
            && curbuf_mut().b_p_tsrfu.cb_type() == K_CALLBACK_NONE_CONST
            && crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_tsrfu)
                == K_CALLBACK_NONE_CONST
    };
    if empty {
        insexp().ctrl_x_mode = CTRL_X_NORMAL;
        g_st().edit_submode = None;
        emsg(gettext(if dict_opt {
            c"'dictionary' option is empty"
        } else {
            c"'thesaurus' option is empty"
        }));
        if g_st().emsg_silent == 0 && !g_st().in_assert_fails {
            vim_beep(K_OPT_BO_FLAG_COMPLETE);
            setcursor();
            msg_delay(2004, false);
        }
        return false;
    }
    true
}

/// Check that the character "c" a valid key to go to or keep us in CTRL-X mode.
pub fn vim_is_ctrl_x_key(c: i32) -> bool {
    if ins_compl_pum_key(c) {
        return true;
    }

    match insexp().ctrl_x_mode {
        0 => c == CTRL_N || c == CTRL_P || c == CTRL_X_CONST,
        CTRL_X_NOT_DEFINED_YET | CTRL_X_CMDLINE_CTRL_X => {
            c == CTRL_X_CONST
                || c == CTRL_Y
                || c == CTRL_E
                || c == CTRL_L_CONST
                || c == CTRL_F_CONST
                || c == CTRL_RSB_CONST
                || c == CTRL_I_CONST
                || c == CTRL_D_CONST
                || c == CTRL_P
                || c == CTRL_N
                || c == CTRL_T_CONST
                || c == CTRL_V_CONST
                || c == CTRL_Q_CONST
                || c == CTRL_U
                || c == CTRL_O
                || c == CTRL_S_CONST
                || c == CTRL_K_CONST
                || c == b's' as i32
                || c == CTRL_Z_CONST
                || c == CTRL_R_CONST
        }
        CTRL_X_SCROLL => c == CTRL_Y || c == CTRL_E,
        CTRL_X_WHOLE_LINE => c == CTRL_L_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_FILES => c == CTRL_F_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_DICTIONARY => c == CTRL_K_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_THESAURUS => c == CTRL_T_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_TAGS => c == CTRL_RSB_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_PATH_PATTERNS => c == CTRL_P || c == CTRL_N,
        CTRL_X_PATH_DEFINES => c == CTRL_D_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_CMDLINE => {
            c == CTRL_V_CONST
                || c == CTRL_Q_CONST
                || c == CTRL_P
                || c == CTRL_N
                || c == CTRL_X_CONST
        }
        CTRL_X_FUNCTION => c == CTRL_U || c == CTRL_P || c == CTRL_N,
        CTRL_X_OMNI => c == CTRL_O || c == CTRL_P || c == CTRL_N,
        CTRL_X_SPELL => c == CTRL_S_CONST || c == CTRL_P || c == CTRL_N,
        CTRL_X_EVAL => c == CTRL_P || c == CTRL_N,
        CTRL_X_BUFNAMES => c == CTRL_P || c == CTRL_N,
        CTRL_X_REGISTER => c == CTRL_R_CONST || c == CTRL_P || c == CTRL_N,
        _ => {
            internal_error(c"vim_is_ctrl_x_key()");
            false
        }
    }
}

#[inline]
fn match_at_original_text(match_: &Compl) -> bool {
    (match_.cp_flags & CP_ORIGINAL_TEXT) != 0
}

/// Return the index of the entry holding the original text, None if not found.
fn find_original_text_match_idx() -> Option<usize> {
    let list = compl_list_mut();
    if list.is_empty() {
        return None;
    }
    if match_at_original_text(&list[0]) {
        return Some(0);
    }
    if insexp().compl_is_cyclic && match_at_original_text(list.last().unwrap()) {
        return Some(list.len() - 1);
    }
    None
}

fn do_autocmd_completedone(c: i32, mode: i32, word: Option<&NvimString>) {
    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let v_event = get_v_event(&mut save_v_event);

    if let Some(word) = word
        && !word.data.is_null()
    {
        tv_dict_add_str_len(&mut *v_event, c"complete_word", Some(word.as_bytes()));
    } else {
        tv_dict_add_str_len(&mut *v_event, c"complete_word", Some(b""));
    }

    let idx = (mode & !CTRL_X_WANT_IDENT) as usize;
    match CTRL_X_MODE_NAMES.get(idx).copied().flatten() {
        Some(mode_cstr) => {
            tv_dict_add_str_len(&mut *v_event, c"complete_type", Some(mode_cstr.to_bytes()));
        }
        None => {
            tv_dict_add_str_len(&mut *v_event, c"complete_type", Some(b""));
        }
    }

    let reason = if c == CTRL_Y || word.is_some_and(|w| !w.data.is_null()) {
        c"accept"
    } else if c == CTRL_E {
        c"cancel"
    } else {
        c"discard"
    };
    tv_dict_add_str(&mut *v_event, c"reason", Some(reason));
    tv_dict_set_keys_readonly(&mut *v_event);

    ins_apply_autocmds(EVENT_COMPLETEDONE);
    restore_v_event(&mut *v_event, &mut save_v_event);
}

/// Check that character "c" is part of the item currently being completed.
pub fn ins_compl_accept_char(c: i32) -> bool {
    if insexp().compl_autocomplete && insexp().compl_from_nonkeyword {
        return false;
    }

    if (insexp().ctrl_x_mode & CTRL_X_WANT_IDENT) != 0 {
        return vim_is_idc(c);
    }

    match insexp().ctrl_x_mode {
        CTRL_X_FILES => vim_isfilec(c) && !vim_ispathsep(c),
        CTRL_X_CMDLINE | CTRL_X_CMDLINE_CTRL_X | CTRL_X_OMNI => {
            vim_isprintc(c) && !ascii_iswhite(c)
        }
        CTRL_X_WHOLE_LINE => vim_isprintc(c),
        _ => vim_iswordc(c),
    }
}

/// Check if "c" is a commit character for the currently selected completion match.
pub fn ins_compl_commit_char(c: i32) -> bool {
    if c < b' ' as i32 {
        return false;
    }
    let Some(shown) = compl_opt(insexp().compl_shown_idx) else {
        return false;
    };
    !match_at_original_text(shown)
        && shown
            .cp_commit_chars
            .as_ref()
            .is_some_and(|s| crate::strings::vim_haschr(s, c))
}

/// Get the completed text by inferring the case of the originally typed text.
fn ins_compl_infercase_gettext<'a>(
    str_bytes: &[u8],
    char_len: i32,
    compl_char_len: i32,
    min_len: i32,
    tofree: &'a mut Vec<u8>,
) -> &'a [u8] {
    let mut has_lower = false;

    let mut wca = vec![0_i32; char_len as usize];
    {
        let mut rem = crate::memory::bytes_until_nul(str_bytes);
        for item in &mut wca {
            *item = mb_ptr2char_adv(&mut rem);
        }
    }

    let orig_bytes = insexp().compl_orig_text.as_bytes();
    // Rule 1: Were any chars converted to lower?
    {
        let mut rem = orig_bytes;
        for i in 0..(min_len as usize) {
            let c = mb_ptr2char_adv(&mut rem);
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
        let mut rem = orig_bytes;
        for i in 0..(min_len as usize) {
            let c = mb_ptr2char_adv(&mut rem);
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
        let mut rem = orig_bytes;
        for item in &mut wca[..(min_len as usize)] {
            let c = mb_ptr2char_adv(&mut rem);
            if mb_islower(c) {
                *item = mb_tolower(*item);
            } else if mb_isupper(c) {
                *item = mb_toupper(*item);
            }
        }
    }

    // Generate encoding specific output from wide character array.
    let iobuff = g_st().borrow_mut().iobuff_bytes_mut();
    let mut p_idx = 0usize;
    let mut i: i32 = 0;
    let mut gap: Vec<u8> = Vec::new();
    while i < char_len {
        if !gap.is_empty() {
            let mut ch_buf = [0u8; 6];
            let n = utf_char2bytes(wca[i as usize], &mut ch_buf) as usize;
            gap.extend_from_slice(&ch_buf[..n]);
            i += 1;
        } else if p_idx + 6 >= IOSIZE {
            gap.reserve(IOSIZE);
            gap.extend_from_slice(&iobuff[..p_idx]);
        } else {
            p_idx += utf_char2bytes(
                wca[i as usize],
                crate::globals::GraphPtr::raw_slice_mut(iobuff[p_idx..].as_mut_ptr().cast(), 6),
            ) as usize;
            i += 1;
        }
    }

    if !gap.is_empty() {
        gap.push(0);
        *tofree = gap;
        return &tofree[..tofree.len() - 1];
    }

    iobuff[p_idx] = 0;
    &iobuff[..p_idx]
}

/// Like `ins_compl_add()`, but infers case if `'ic'` and `'inf'` are set.
pub fn ins_compl_add_infercase(
    str_arg: &[u8],
    len: i32,
    icase: bool,
    fname: Option<&CStr>,
    dir: Direction,
    cont_s_ipos: bool,
    score: i32,
) -> Option<bool> {
    let str_slice = if len >= 0 && (len as usize) <= str_arg.len() {
        crate::memory::bytes_until_nul(&str_arg[..len as usize])
    } else {
        crate::memory::bytes_until_nul(str_arg)
    };
    let len = if len < 0 { str_slice.len() as i32 } else { len };
    let mut flags: i32 = 0;
    let mut tofree: Vec<u8> = Vec::new();

    let final_slice = if opt_vars().p_ic && curbuf_mut().b_p_inf && len > 0 {
        let mut char_len: i32 = 0;
        {
            let mut rem = str_slice;
            while !rem.is_empty() {
                let step = utf_ptr2len(rem.as_ptr()) as usize;
                rem = &rem[step.max(1).min(rem.len())..];
                char_len += 1;
            }
        }

        let mut compl_char_len: i32 = 0;
        {
            let mut rem = insexp().compl_orig_text.as_bytes();
            while !rem.is_empty() {
                let step = utf_ptr2len(rem.as_ptr()) as usize;
                rem = &rem[step.max(1).min(rem.len())..];
                compl_char_len += 1;
            }
        }

        let min_len = char_len.min(compl_char_len);
        ins_compl_infercase_gettext(str_slice, char_len, compl_char_len, min_len, &mut tofree)
    } else {
        str_slice
    };
    if cont_s_ipos {
        flags |= CP_CONT_S_IPOS;
    }
    if icase {
        flags |= CP_ICASE;
    }

    ins_compl_add(
        final_slice,
        len,
        fname,
        [const { None }; CPT_COUNT],
        None,
        dir,
        flags,
        false,
        None,
        score,
        false,
        None,
    )
}

#[inline]
fn cot_fuzzy() -> bool {
    (get_cot_flags() & K_OPT_COT_FLAG_FUZZY) != 0 && !ctrl_x_mode_thesaurus()
}

#[inline]
fn is_nearest_active() -> bool {
    (insexp().compl_autocomplete || (get_cot_flags() & K_OPT_COT_FLAG_NEAREST) != 0) && !cot_fuzzy()
}

/// True if a match is selected (even if it is not inserted).
pub fn ins_compl_is_match_selected() -> bool {
    !matches!(insexp().compl_shown_idx, None | Some(0))
}

/// Returns true if autocomplete is active and the pre-insert effect targets the longest prefix.
pub fn ins_compl_preinsert_longest() -> bool {
    insexp().compl_autocomplete
        && (get_cot_flags()
            & (K_OPT_COT_FLAG_LONGEST | K_OPT_COT_FLAG_PREINSERT | K_OPT_COT_FLAG_FUZZY))
            == K_OPT_COT_FLAG_LONGEST
}

/// Add a match to the list of matches.
fn ins_compl_add(
    str_slice: &[u8],
    mut len: i32,
    fname: Option<&CStr>,
    cptext: [Option<std::ffi::CString>; CPT_COUNT],
    user_data: Option<&mut TypVal>,
    cdir: Direction,
    flags_arg: i32,
    adup: bool,
    user_hl: Option<&[i32; 2]>,
    score: i32,
    preselect: bool,
    commit_chars: Option<std::ffi::CString>,
) -> Option<bool> {
    let dir = if cdir == K_DIRECTION_NOT_SET {
        insexp().compl_direction
    } else {
        cdir
    };
    let flags = flags_arg;

    if (flags & CP_FAST) != 0 {
        fast_breakcheck();
    } else {
        os_breakcheck();
    }
    if g_st().got_int {
        return Some(false);
    }
    let str_bytes = crate::memory::bytes_until_nul(str_slice);
    if len < 0 {
        len = str_bytes.len() as i32;
    }
    let key_prefix = &str_bytes[..(len as usize).min(str_bytes.len())];

    // If the same match is already present, don't add it.
    if !insexp().compl_list.is_empty()
        && !adup
        && !insexp().compl_strings_ht.is_empty()
        && insexp().compl_strings_ht.contains_key(key_prefix)
    {
        if is_nearest_active() && score > 0 {
            for match_ in &mut insexp().compl_list {
                let cp_bytes = match_.cp_str.as_bytes();
                if !match_at_original_text(match_)
                    && &cp_bytes[..(len as usize).min(cp_bytes.len())] == key_prefix
                    && (match_.cp_str.size as i32 <= len
                        || cp_bytes.get(len as usize).is_none_or(|&b| b == 0))
                {
                    if score < match_.cp_score {
                        match_.cp_score = score;
                    }
                    break;
                }
            }
        }
        return None;
    }

    ins_compl_del_pum();

    let cp_fname = fname.map(CStr::to_owned);
    let mut cp_text = [const { None }; CPT_COUNT];
    for (slot, item) in cp_text.iter_mut().zip(cptext) {
        if let Some(s) = item
            && !s.as_bytes().is_empty()
        {
            *slot = Some(s);
        }
    }
    let cp_commit_chars = commit_chars;
    let cp_user_data = if let Some(user_data) = user_data {
        std::mem::take(user_data)
    } else {
        crate::eval::typval::tv_initial_value()
    };

    let match_ = Compl {
        cp_str: crate::api::private::helpers::cbuf_to_string(key_prefix),
        cp_text,
        cp_user_data,
        cp_fname,
        cp_commit_chars,
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
        cp_user_abbr_hl_id: user_hl.map_or(0, |hl| hl[0]),
        cp_user_kind_hl_id: user_hl.map_or(0, |hl| hl[1]),
        cp_cpt_source_idx: insexp().cpt_sources_index,
    };

    let list = compl_list_mut();
    let insert_idx = if list.is_empty() {
        0
    } else if cot_fuzzy() && score != FUZZY_SCORE_NONE && insexp().compl_get_longest {
        if let Some(pos) = list[1..].iter().position(|m| m.cp_score < score) {
            pos + 1
        } else {
            insexp().compl_is_cyclic = true;
            list.len()
        }
    } else if dir == FORWARD {
        insexp().compl_curr_idx.unwrap() + 1
    } else {
        let curr = insexp().compl_curr_idx.unwrap();
        if curr == 0 && insexp().compl_is_cyclic {
            list.len()
        } else {
            curr
        }
    };

    for idx_opt in [
        &mut insexp().compl_curr_idx,
        &mut insexp().compl_shown_idx,
        &mut insexp().compl_old_idx,
        &mut insexp().compl_preselect_idx,
    ] {
        if let Some(idx) = idx_opt
            && *idx >= insert_idx
        {
            *idx += 1;
        }
    }
    list.insert(insert_idx, match_);
    insexp().compl_curr_idx = Some(insert_idx);
    if preselect
        && { insexp().compl_preselect_idx }.is_none()
        && (get_cot_flags() & K_OPT_COT_FLAG_PRESELECT) != 0
    {
        insexp().compl_preselect_idx = Some(insert_idx);
    }

    let match_ptr = compl_at(insert_idx);
    if !match_at_original_text(&*match_ptr) {
        compl_strings_add(match_ptr.cp_str.as_bytes());
    }

    if insexp().compl_get_longest
        && (flags & CP_ORIGINAL_TEXT) == 0
        && !cot_fuzzy()
        && !ins_compl_preinsert_longest()
        && !ctrl_x_mode_thesaurus()
    {
        ins_compl_longest_match(&mut *match_ptr);
    }

    Some(true)
}

/// Check that "str[len]" matches with "match->cp_str", considering "match->cp_flags".
fn ins_compl_equal(match_: &mut Compl, str_bytes: &[u8]) -> bool {
    if (match_.cp_flags & CP_EQUAL) != 0 {
        return true;
    }
    let str_prefix = crate::memory::bytes_until_nul(str_bytes);
    let len = str_prefix.len();
    let cp_b = match_.cp_str.as_bytes();
    let cp_prefix = &cp_b[..len.min(cp_b.len())];
    if (match_.cp_flags & CP_ICASE) != 0 {
        return cp_prefix.eq_ignore_ascii_case(str_prefix);
    }
    cp_prefix == str_prefix
}

/// Like `ins_compl_equal()`, but ignore case in the `'longest'`-inserted part of the leader.
fn ins_compl_equal_sc(match_: &mut Compl, str_bytes: &[u8]) -> bool {
    let typed = insexp().compl_length;
    let longest_end =
        if insexp().compl_get_longest && insexp().compl_longest_end_col > insexp().compl_col {
            insexp().compl_longest_end_col - insexp().compl_col
        } else {
            typed
        };

    if (match_.cp_flags & (CP_EQUAL | CP_ICASE)) != 0 || longest_end <= typed {
        return ins_compl_equal(match_, str_bytes);
    }

    let str_b = crate::memory::bytes_until_nul(str_bytes);
    let len = str_b.len();
    let cp_b = match_.cp_str.as_bytes();
    if cp_b.len() < len {
        return false;
    }

    for (i, (&c1, &c2)) in cp_b[..len].iter().zip(str_b.iter()).enumerate() {
        let idx = i as i32;
        let mismatch = if idx >= typed && idx < longest_end {
            !c1.eq_ignore_ascii_case(&c2)
        } else {
            c1 != c2
        };
        if mismatch {
            return false;
        }
    }
    true
}

fn ins_compl_insert_bytes(p: &[u8], mut len: i32) {
    let bytes = crate::memory::bytes_until_nul(p);
    if len == -1 {
        len = bytes.len() as i32;
    }
    debug_assert!(len >= 0);
    let n = (len as usize).min(bytes.len());
    ins_bytes_len(&bytes[..n], n);
    insexp().compl_ins_end_col = curwin_mut().w_cursor.col;
}

/// Get current completion leader.
pub fn ins_compl_leader() -> Option<&'static CStr> {
    if !insexp().compl_leader.data.is_null() {
        Some(insexp().compl_leader.as_cstr())
    } else if !insexp().compl_orig_text.data.is_null() {
        Some(insexp().compl_orig_text.as_cstr())
    } else {
        None
    }
}

#[inline]
fn ins_compl_leader_slice() -> &'static [u8] {
    if !insexp().compl_leader.data.is_null() {
        insexp().compl_leader.as_bytes()
    } else {
        insexp().compl_orig_text.as_bytes()
    }
}

#[inline]
fn ins_compl_leader_len() -> usize {
    if !insexp().compl_leader.data.is_null() {
        insexp().compl_leader.size
    } else {
        insexp().compl_orig_text.size
    }
}

/// Checks if the column is within the currently inserted completion text column range.
pub fn ins_compl_col_range_attr(lnum: Linenr, col: i32) -> i32 {
    let has_preinsert = ins_compl_has_preinsert() || ins_compl_preinsert_longest();

    if cot_fuzzy() || (!insexp().compl_hi_on_autocompl_longest && ins_compl_preinsert_longest()) {
        return -1;
    }
    let attr = syn_name2attr(if has_preinsert {
        c"PreInsert"
    } else {
        c"ComplMatchIns"
    });
    if attr == 0 {
        return -1;
    }

    let start_col = insexp().compl_col + ins_compl_leader_len() as i32;
    if !ins_compl_has_multiple() {
        return if col >= start_col && col < insexp().compl_ins_end_col {
            attr
        } else {
            -1
        };
    }

    if (lnum == insexp().compl_lnum && col >= start_col && col < MAXCOL)
        || (lnum > insexp().compl_lnum && lnum < curwin_mut().w_cursor.lnum)
        || (lnum == curwin_mut().w_cursor.lnum && col <= insexp().compl_ins_end_col)
    {
        return attr;
    }

    -1
}

#[inline]
fn ins_compl_has_multiple() -> bool {
    compl_opt(insexp().compl_shown_idx)
        .is_some_and(|shown| crate::strings::vim_haschr(shown.cp_str.as_cstr(), b'\n' as i32))
}

/// Returns true if the given line number falls within the range of a multi-line completion.
pub fn ins_compl_lnum_in_range(lnum: Linenr) -> bool {
    if !ins_compl_has_multiple() {
        return false;
    }
    lnum >= insexp().compl_lnum && lnum <= curwin_mut().w_cursor.lnum
}

/// Reduce the longest common string for match "match".
fn ins_compl_longest_match(match_: &mut Compl) {
    if insexp().compl_leader.data.is_null() {
        insexp().compl_leader = copy_string(match_.cp_str, None);

        let had_match = curwin_mut().w_cursor.col > insexp().compl_col;
        ins_compl_longest_insert(insexp().compl_leader.as_bytes());

        if !had_match {
            ins_compl_delete(false);
        }
        insexp().compl_used_match = false;
        return;
    }

    let leader_ptr = insexp().compl_leader.data.as_ptr().cast::<u8>();
    let leader_slice = insexp().compl_leader.cstr_bytes();
    let s_slice = match_.cp_str.cstr_bytes();
    let mut p_off = 0usize;
    let mut s_off = 0usize;
    while p_off < leader_slice.len() {
        let p_rem = &leader_slice[p_off..];
        let s_rem = &s_slice[s_off.min(s_slice.len())..];
        let c1 = utf_ptr2char(p_rem);
        let c2 = utf_ptr2char(s_rem);

        let diff = if (match_.cp_flags & CP_ICASE) != 0 {
            mb_tolower(c1) != mb_tolower(c2)
        } else {
            c1 != c2
        };
        if diff {
            break;
        }
        p_off += (crate::mbyte::utfc_ptr2len(p_rem) as usize).min(p_rem.len());
        s_off += (crate::mbyte::utfc_ptr2len(s_rem) as usize).min(s_rem.len());
    }

    if p_off < leader_slice.len() && !leader_ptr.is_null() {
        *crate::globals::GraphPtr::of(leader_ptr.wrapping_add(p_off)).borrow_mut() = 0;

        insexp().compl_leader.size = p_off;

        let had_match = curwin_mut().w_cursor.col > insexp().compl_col;
        ins_compl_longest_insert(insexp().compl_leader.as_bytes());

        if !had_match {
            ins_compl_delete(false);
        }
    }

    insexp().compl_used_match = false;
}

/// Add an array of matches to the list of matches. Frees matches[].
fn ins_compl_add_matches(matches: &mut Vec<std::ffi::CString>, icase: i32) {
    let mut add_r = Some(true);
    let mut dir = insexp().compl_direction;

    for m in matches.drain(..) {
        if add_r == Some(false) {
            break;
        }
        add_r = ins_compl_add(
            m.as_bytes(),
            m.as_bytes().len() as i32,
            None,
            [const { None }; CPT_COUNT],
            None,
            dir,
            CP_FAST | if icase != 0 { CP_ICASE } else { 0 },
            false,
            None,
            FUZZY_SCORE_NONE,
            false,
            None,
        );
        if add_r == Some(true) {
            dir = FORWARD;
        }
    }
}

/// Make the completion list cyclic.
/// Return the number of matches (excluding the original).
fn ins_compl_make_cyclic() -> i32 {
    let list = &insexp().compl_list;
    if list.is_empty() {
        return 0;
    }
    insexp().compl_is_cyclic = true;
    (list.len() - 1) as i32
}

pub fn ins_compl_has_shown_match() -> bool {
    let Some(shown_idx) = insexp().compl_shown_idx else {
        return true;
    };
    compl_next_idx(shown_idx) != Some(shown_idx)
}

pub fn ins_compl_long_shown_match() -> bool {
    compl_opt(insexp().compl_shown_idx).is_some_and(|shown| {
        !shown.cp_str.data.is_null()
            && (shown.cp_str.size as Colnr) > curwin_mut().w_cursor.col - insexp().compl_col
    })
}

pub fn get_cot_flags() -> u32 {
    if curbuf_mut().b_cot_flags != 0 {
        curbuf_mut().b_cot_flags
    } else {
        opt_vars().cot_flags
    }
}

fn ins_compl_del_pum() {
    if insexp().compl_match_array.is_empty() {
        return;
    }

    pum_undisplay(false);
    insexp().compl_match_array.clear();
}

pub fn pum_wanted() -> bool {
    (get_cot_flags() & (K_OPT_COT_FLAG_MENU | K_OPT_COT_FLAG_MENUONE)) != 0
        || insexp().compl_autocomplete
}

fn pum_enough_matches() -> bool {
    let list = &insexp().compl_list;
    let mut i: i32 = 0;
    for comp in list {
        if !match_at_original_text(comp) {
            i += 1;
            if i == 2 {
                break;
            }
        }
    }

    if (get_cot_flags() & K_OPT_COT_FLAG_MENUONE) != 0 || insexp().compl_autocomplete {
        return i >= 1;
    }
    i >= 2
}

fn ins_compl_dict_alloc(match_: Option<&mut Compl>) -> &'static mut TvDict {
    let dict = tv_dict_alloc_lock(VAR_FIXED);
    if let Some(m) = match_ {
        fill_complete_info_dict(&mut *dict, m, false);
    }
    dict
}

fn trigger_complete_changed_event(cur: i32) {
    static RECURSIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    {
        if RECURSIVE.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        let item = if cur < 0 {
            tv_dict_alloc()
        } else {
            ins_compl_dict_alloc(compl_opt(insexp().compl_curr_idx))
        };
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);
        tv_dict_add_dict(&mut *v_event, c"completed_item", Some(item));
        pum_set_event_info(&mut *v_event);
        tv_dict_set_keys_readonly(&mut *v_event);

        RECURSIVE.store(true, std::sync::atomic::Ordering::Relaxed);
        g_st().textlock += 1;
        apply_autocmds(EVENT_COMPLETECHANGED, None, None, false, curbuf_opt_mut());
        g_st().textlock -= 1;
        RECURSIVE.store(false, std::sync::atomic::Ordering::Relaxed);

        restore_v_event(&mut *v_event, &mut save_v_event);
    }
}

fn cp_compare_fuzzy(a: &Compl, b: &Compl) -> i32 {
    let score_a = a.cp_score;
    let score_b = b.cp_score;

    if score_a != score_b {
        return if score_b > score_a { 1 } else { -1 };
    }

    a.cp_order - b.cp_order
}

fn cp_compare_nearest(a: &Compl, b: &Compl) -> i32 {
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

fn prepend_startcol_text(dest: &mut NvimString, src: &NvimString, startcol: i32) {
    let prepend_len = (insexp().compl_col - startcol) as usize;
    let new_length = prepend_len + src.size;
    let mut buf: Vec<u8> = vec![0; new_length + 1];

    let line_slice = crate::cursor::get_cursor_line_slice();
    buf[..prepend_len]
        .copy_from_slice(&line_slice[startcol as usize..startcol as usize + prepend_len]);
    buf[prepend_len..new_length].copy_from_slice(src.as_bytes());
    *dest = cbuf_to_string(&buf[..new_length]);
}

fn get_leader_for_startcol(match_: Option<&Compl>, cached: bool) -> NvimString {
    if match_.is_none() {
        api_clear_string(&mut insexp().adjusted_leader);
        return NvimString::new();
    }

    let cpt_sources = &insexp().cpt_sources_array;
    if cpt_sources.is_empty() {
        return insexp().compl_leader;
    }

    let cpt_idx = match_.unwrap().cp_cpt_source_idx;
    if cpt_idx < 0 {
        return insexp().compl_leader;
    }
    let startcol = insexp().cpt_sources_array[cpt_idx as usize].cs_startcol;

    if insexp().compl_leader.data.is_null() {
        if startcol < 0 || startcol >= insexp().compl_col {
            return insexp().compl_orig_text;
        }
        return insexp().compl_leader;
    }

    if insexp().compl_col <= 0 {
        return insexp().compl_leader;
    }

    if startcol >= 0 && startcol < insexp().compl_col {
        let prepend_len = insexp().compl_col - startcol;
        let new_length = prepend_len + insexp().compl_leader.size as i32;
        if cached
            && (new_length as usize) == insexp().adjusted_leader.size
            && !insexp().adjusted_leader.data.is_null()
        {
            return insexp().adjusted_leader;
        }

        api_clear_string(&mut insexp().adjusted_leader);
        let compl_leader = insexp().compl_leader;
        prepend_startcol_text(&mut insexp().adjusted_leader, &compl_leader, startcol);
        return insexp().adjusted_leader;
    }

    insexp().compl_leader
}

fn set_fuzzy_score() {
    let list = &mut insexp().compl_list;
    if list.is_empty() {
        return;
    }

    let use_leader = !insexp().compl_leader.data.is_null() && insexp().compl_leader.size > 0;
    let mut pattern;
    if !use_leader {
        if insexp().compl_orig_text.data.is_null() || insexp().compl_orig_text.size == 0 {
            return;
        }
        pattern = insexp().compl_orig_text.data.cast::<u8>();
    } else {
        let _ = get_leader_for_startcol(None, true);
        pattern = ptr::null_mut();
    }

    let mut next: i32 = 0;
    for comp in list {
        if comp.cp_order < 0 {
            comp.cp_order = next;
        }
        next = next.max(comp.cp_order + 1);

        if use_leader {
            pattern = get_leader_for_startcol(Some(&*comp), true).data.cast();
        }

        comp.cp_score = fuzzy_match_str(
            comp.cp_str.as_opt_cstr().map(CStr::to_bytes),
            (!pattern.is_null()).then(|| unsafe { CStr::from_ptr(pattern.cast()) }.to_bytes()),
        );
    }
}

fn sort_compl_match_list(compare: fn(&Compl, &Compl) -> i32) {
    let list = compl_list_mut();
    if list.len() <= 1 {
        return;
    }

    let Some(orig_idx) = find_original_text_match_idx() else {
        return;
    };

    let saved_ptrs = [
        insexp().compl_curr_idx.map(|i| list[i].cp_str.data),
        insexp().compl_shown_idx.map(|i| list[i].cp_str.data),
        insexp().compl_old_idx.map(|i| list[i].cp_str.data),
        insexp().compl_preselect_idx.map(|i| list[i].cp_str.data),
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

    if let Some(p) = saved_ptrs[0] {
        insexp().compl_curr_idx = list.iter().position(|m| m.cp_str.data == p);
    }
    if let Some(p) = saved_ptrs[1] {
        insexp().compl_shown_idx = list.iter().position(|m| m.cp_str.data == p);
    }
    if let Some(p) = saved_ptrs[2] {
        insexp().compl_old_idx = list.iter().position(|m| m.cp_str.data == p);
    }
    if let Some(p) = saved_ptrs[3] {
        insexp().compl_preselect_idx = list.iter().position(|m| m.cp_str.data == p);
    }
    insexp().compl_is_cyclic = true;
}

fn get_user_highlight_attr(hl_id: i32) -> i32 {
    if hl_id <= 0 {
        return -1;
    }
    let attr = syn_id2attr(hl_id);
    if attr > 0 { attr } else { -1 }
}

fn ins_compl_build_pum() -> i32 {
    if ins_compl_need_restart() {
        api_clear_string(&mut insexp().compl_leader);
    }

    let compl_no_select = (get_cot_flags() & K_OPT_COT_FLAG_NOSELECT) != 0
        || (insexp().compl_autocomplete && !ins_compl_has_preinsert());

    let is_forward = compl_shows_dir_forward();
    let cpt_sources = &insexp().cpt_sources_array;
    let is_cpt_completion = !cpt_sources.is_empty();

    let list = &mut insexp().compl_list;
    let mut shown_match_ok = compl_opt(insexp().compl_shown_idx)
        .as_deref()
        .is_some_and(match_at_original_text);

    if (insexp().compl_leader.opt_bytes() == insexp().compl_orig_text.opt_bytes())
        && !shown_match_ok
    {
        insexp().compl_shown_idx = if compl_no_select {
            Some(0)
        } else {
            compl_next_idx(0)
        };
    }

    let mut did_find_shown_match = false;
    let mut preselect_ok = false;
    let mut shown_compl: Option<usize> = None;
    let mut i: i32 = 0;
    let mut cur: i32 = -1;

    let mut match_count = if is_cpt_completion {
        vec![0_i32; cpt_sources.len()]
    } else {
        Vec::new()
    };

    let _ = get_leader_for_startcol(None, true);

    let mut items: Vec<Pumitem> = Vec::new();
    for (comp_idx, comp) in list.iter_mut().enumerate() {
        comp.cp_in_match_array = false;

        let leader = get_leader_for_startcol(Some(&*comp), true);

        if ctrl_x_mode_normal()
            && !opt_vars().p_inf
            && !insexp().compl_orig_text.data.is_null()
            && !ignorecase(insexp().compl_orig_text.as_cstr())
            && !cot_fuzzy()
        {
            comp.cp_flags &= !CP_ICASE;
        }

        if !match_at_original_text(comp)
            && (leader.data.is_null()
                || ins_compl_equal_sc(comp, leader.as_bytes())
                || (cot_fuzzy() && comp.cp_score != FUZZY_SCORE_NONE))
        {
            let mut match_limit_exceeded = false;
            let cur_source = comp.cp_cpt_source_idx;
            if is_forward && cur_source != -1 && is_cpt_completion {
                match_count[cur_source as usize] += 1;
                let max_matches = insexp().cpt_sources_array[cur_source as usize].cs_max_matches;
                if max_matches > 0 && match_count[cur_source as usize] > max_matches {
                    match_limit_exceeded = true;
                }
            }

            if !match_limit_exceeded {
                comp.cp_in_match_array = true;
                items.push(Pumitem {
                    pum_text: comp.cp_text[CPT_ABBR].clone().or_else(|| {
                        if comp.cp_str.data.is_null() {
                            None
                        } else {
                            Some(comp.cp_str.to_cstring())
                        }
                    }),
                    pum_kind: comp.cp_text[CPT_KIND].clone(),
                    pum_extra: comp.cp_text[CPT_MENU]
                        .clone()
                        .or_else(|| comp.cp_fname.clone()),
                    pum_info: comp.cp_text[CPT_INFO].clone(),
                    pum_cpt_source_idx: comp.cp_cpt_source_idx,
                    pum_user_abbr_hlattr: get_user_highlight_attr(comp.cp_user_abbr_hl_id),
                    pum_user_kind_hlattr: get_user_highlight_attr(comp.cp_user_kind_hl_id),
                    pum_comp_idx: Some(comp_idx),
                });

                if !shown_match_ok && !cot_fuzzy() {
                    if insexp().compl_shown_idx == Some(comp_idx) || did_find_shown_match {
                        insexp().compl_shown_idx = Some(comp_idx);
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

                    if !shown_match_ok && insexp().compl_shown_idx == Some(comp_idx) {
                        cur = i;
                        shown_match_ok = true;
                    }
                }
                if { insexp().compl_preselect_idx }.is_some() && comp.cp_preselect && !preselect_ok
                {
                    cur = i;
                    insexp().compl_shown_idx = Some(comp_idx);
                    shown_match_ok = true;
                    preselect_ok = true;
                }
                i += 1;
            }
        }

        if insexp().compl_shown_idx == Some(comp_idx) && !cot_fuzzy() {
            did_find_shown_match = true;
            if match_at_original_text(comp) {
                shown_match_ok = true;
            }
            if !shown_match_ok && shown_compl.is_some() {
                insexp().compl_shown_idx = shown_compl;
                shown_match_ok = true;
            }
        }
    }

    if items.is_empty() {
        return -1;
    }

    if cot_fuzzy() && !compl_no_select && !shown_match_ok {
        insexp().compl_shown_idx = shown_compl;
        shown_match_ok = true;
        cur = 0;
    }

    insexp().compl_match_array = items;

    if !shown_match_ok {
        cur = -1;
    }

    cur
}

/// Show the popup menu for the list of matches.
pub fn ins_compl_show_pum() {
    if !pum_wanted() || !pum_enough_matches() {
        return;
    }

    update_screen();

    let mut cur: i32 = -1;
    let mut array_changed = false;

    let arr = &mut insexp().compl_match_array;
    if arr.is_empty() {
        array_changed = true;
        cur = ins_compl_build_pum();
    } else {
        for (i, item) in arr.iter().enumerate() {
            if item.pum_comp_idx == insexp().compl_shown_idx {
                cur = i as i32;
                break;
            }
        }
    }

    if arr.is_empty() {
        if insexp().compl_started && has_event(EVENT_COMPLETECHANGED) {
            trigger_complete_changed_event(cur);
        }
        return;
    }

    g_st().dollar_vcol = -1;

    let col = curwin_mut().w_cursor.col;
    curwin_mut().w_cursor.col = insexp().compl_col;
    insexp().compl_selected_item = cur;
    pum_display(arr, cur, array_changed, 0);
    curwin_mut().w_cursor.col = col;

    if curwin_mut().w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(curwin_mut()) {
        redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
    }

    if insexp().compl_started && insexp().compl_curr_idx != insexp().compl_shown_idx {
        insexp().compl_curr_idx = insexp().compl_shown_idx;
    }

    if has_event(EVENT_COMPLETECHANGED) {
        trigger_complete_changed_event(cur);
    }
}

pub fn compl_match_curr_select(selected: i32) -> bool {
    if selected < 0 {
        return false;
    }
    let list = &insexp().compl_list;
    let curr_num = compl_opt(insexp().compl_curr_idx).map(|c| c.cp_number);
    let mut selected_idx: i32 = -1;
    let mut list_idx: i32 = 0;
    for match_ in list {
        if !match_at_original_text(match_) {
            if curr_num == Some(match_.cp_number) {
                selected_idx = list_idx;
                break;
            }
            list_idx += 1;
        }
    }

    selected == selected_idx
}

fn ins_compl_dictionaries(
    dict_start: Option<&CStr>,
    pat_arg: Option<&CStr>,
    flags: i32,
    thesaurus: bool,
) {
    let mut dict: &CStr = dict_start.unwrap_or(c"");
    let pat_cstr: &CStr = pat_arg.unwrap_or(c"");
    let mut regmatch: Regmatch = Regmatch::new();
    let mut files: Vec<std::ffi::CString> = Vec::new();
    let mut count: i32;
    let mut dir = insexp().compl_direction;

    if dict.to_bytes().is_empty() {
        if !thesaurus && curwin_mut().w_onebuf_opt.wo_spell {
            dict = c"spell";
        } else {
            return;
        }
    }

    let mut buf_vec = vec![0u8; LSIZE];
    regmatch.regprog = crate::globals::GraphPtr::NULL;

    let save_p_scs = opt_vars().p_scs;
    if curbuf_mut().b_p_inf {
        opt_vars().p_scs = false;
    }

    if ctrl_x_mode_line_or_eval() {
        let pat_esc = vim_strsave_escaped(pat_cstr.to_bytes(), b"\\");
        let len = pat_esc.as_bytes().len() + 10;
        let mut ptr_vec = vec![0u8; len];
        format_to_buf!(&mut ptr_vec, c"^\\s*\\zs\\V%s", &pat_esc);
        regmatch.set_prog(vim_regcomp(
            crate::memory::bytes_as_cstr(&ptr_vec),
            RE_MAGIC,
        ));
    } else {
        regmatch.set_prog(vim_regcomp(
            pat_cstr,
            if opt_vars().p_magic { RE_MAGIC } else { 0 },
        ));
        if regmatch.regprog.is_null() {
            opt_vars().p_scs = save_p_scs;
            regmatch.free_prog();
            return;
        }
    }

    regmatch.rm_ic = ignorecase(pat_cstr);
    while !dict.to_bytes().is_empty() && !g_st().got_int && !insexp().compl_interrupted {
        if flags == DICT_EXACT {
            count = 1;
        } else {
            let mut dict_bytes = dict.to_bytes();
            let old_len = dict_bytes.len();
            copy_option_part(&mut dict_bytes, &mut buf_vec, c",");
            dict = crate::memory::CStrSubslice::advance(dict, old_len - dict_bytes.len());
            let buf_cstr = crate::memory::bytes_as_cstr(&buf_vec);
            if !thesaurus && buf_cstr.to_bytes() == b"spell" {
                count = -1;
            } else if crate::strings::vim_haschr(buf_cstr, b'`' as i32)
                || !expand_wildcards_vec(&[buf_cstr], &mut files, EW_FILE | EW_SILENT)
            {
                count = 0;
            } else {
                count = files.len() as i32;
            }
        }

        if count == -1 {
            let ptr_cstr = if pat_cstr.to_bytes().starts_with(b"\\<") {
                crate::memory::CStrSubslice::advance(pat_cstr, 2)
            } else {
                pat_cstr
            };
            spell_dump_compl(Some(ptr_cstr), i32::from(regmatch.rm_ic), Some(&mut dir), 0);
        } else if count > 0 {
            if flags == DICT_EXACT {
                ins_compl_files(
                    &[dict.to_owned()],
                    thesaurus,
                    flags,
                    &mut regmatch,
                    &mut buf_vec,
                    &mut dir,
                );
            } else {
                ins_compl_files(
                    &files,
                    thesaurus,
                    flags,
                    &mut regmatch,
                    &mut buf_vec,
                    &mut dir,
                );
                files.clear();
            }
        }
        if flags != 0 {
            break;
        }
    }

    opt_vars().p_scs = save_p_scs;
    regmatch.free_prog();
}

fn thesaurus_add_words_in_line(
    fname: Option<&CStr>,
    buf_arg: &mut &[u8],
    dir: i32,
    skip_word_ptr: usize,
) -> bool {
    let mut status = true;
    let mut rem = crate::memory::bytes_until_nul(buf_arg);

    while !g_st().got_int {
        rem = find_word_start(rem);
        if rem.is_empty() || rem[0] == NL as u8 {
            break;
        }
        let wstart = rem;

        while !rem.is_empty() {
            let l = utfc_ptr2len(rem.as_ptr()) as usize;
            if l < 2 && !vim_iswordc(i32::from(rem[0])) {
                break;
            }
            rem = &rem[l.max(1).min(rem.len())..];
        }

        let wlen = wstart.len() - rem.len();
        if wstart.as_ptr() as usize != skip_word_ptr {
            status = ins_compl_add_infercase(
                &wstart[..wlen],
                wlen as i32,
                opt_vars().p_ic,
                fname,
                dir,
                false,
                FUZZY_SCORE_NONE,
            ) != Some(false);
            if !status {
                break;
            }
        }
    }

    *buf_arg = rem;
    status
}

fn ins_compl_files(
    files: &[std::ffi::CString],
    thesaurus: bool,
    flags: i32,
    regmatch: &mut Regmatch,
    buf_slice: &mut [u8],
    dir: &mut Direction,
) {
    let leader_slice = if cot_fuzzy() {
        ins_compl_leader_slice()
    } else {
        &[]
    };
    let leader_len = leader_slice.len() as i32;

    for file_cstr in files {
        if g_st().got_int || ins_compl_interrupted() {
            break;
        }
        let fp = CFile::open(file_cstr, c"r");
        if flags != DICT_EXACT && !shortmess(K_SHM_COMPLETIONSCAN) && !insexp().compl_autocomplete {
            format_to_buf!(
                &mut g_st().iobuff,
                gettext(c"Scanning dictionary: %s"),
                file_cstr,
            );
            msg_progress(
                Some(g_st().iobuff_bytes_mut()),
                c"nvim.completion",
                c"running",
                -1,
                HLF_R,
                false,
                true,
                false,
            );
        }

        let Some(fp) = fp else {
            continue;
        };

        while !g_st().got_int && !ins_compl_interrupted() && !vim_fgets(buf_slice, &fp) {
            let line_bytes = crate::memory::bytes_until_nul(buf_slice);
            if cot_fuzzy() && leader_len > 0 {
                let end_tail = find_line_end(line_bytes);
                let mut rem_slice = &line_bytes[..line_bytes.len() - end_tail.len()];
                while !rem_slice.is_empty() {
                    let mut score: i32 = 0;
                    let mut len: i32 = 0;
                    let matched = fuzzy_match_str_in_line(
                        &mut rem_slice,
                        Some(leader_slice),
                        &mut len,
                        None,
                        &mut score,
                    );
                    if matched {
                        let end_tail = if ctrl_x_mode_line_or_eval() {
                            find_line_end(rem_slice)
                        } else {
                            find_word_end(rem_slice)
                        };
                        let match_len = (rem_slice.len() - end_tail.len()) as i32;
                        let add_r = ins_compl_add_infercase(
                            &rem_slice[..match_len as usize],
                            match_len,
                            opt_vars().p_ic,
                            Some(file_cstr.as_c_str()),
                            *dir,
                            false,
                            score,
                        );
                        if add_r == Some(false) {
                            break;
                        }
                        rem_slice = &rem_slice[(match_len as usize).min(rem_slice.len())..];
                        if insexp().compl_get_longest
                            && ctrl_x_mode_normal()
                            && let Some(next_i) = compl_next_idx(0)
                            && score == compl_at(next_i).cp_score
                        {
                            insexp().compl_num_bests += 1;
                        }
                    }
                }
            } else {
                let buf_cstr = crate::memory::bytes_as_cstr(buf_slice);
                let buf_base = buf_cstr.as_ptr() as usize;
                let mut col_off: usize = 0;
                while vim_regexec(&mut *regmatch, buf_cstr, col_off as Colnr) {
                    let start_off = regmatch.start_offset(0, buf_cstr);
                    let from_start = &line_bytes[start_off.min(line_bytes.len())..];
                    let end_tail = if ctrl_x_mode_line_or_eval() {
                        find_line_end(from_start)
                    } else {
                        find_word_end(from_start)
                    };
                    let match_len = from_start.len() - end_tail.len();
                    let mut add_r = ins_compl_add_infercase(
                        &from_start[..match_len],
                        match_len as i32,
                        opt_vars().p_ic,
                        Some(file_cstr.as_c_str()),
                        *dir,
                        false,
                        FUZZY_SCORE_NONE,
                    );
                    col_off = start_off + match_len;
                    if thesaurus {
                        let mut rem_line = line_bytes;
                        add_r = Some(thesaurus_add_words_in_line(
                            Some(file_cstr.as_c_str()),
                            &mut rem_line,
                            *dir,
                            buf_base + start_off,
                        ));
                        col_off = line_bytes.len() - rem_line.len();
                    }
                    if add_r == Some(true) {
                        *dir = FORWARD;
                    } else if add_r == Some(false) {
                        break;
                    }
                    if line_bytes.get(col_off) == Some(&b'\n') || g_st().got_int {
                        break;
                    }
                }
            }
            line_breakcheck();
            ins_compl_check_keys(50, false);
        }
    }
}

pub fn find_word_start(s: &[u8]) -> &[u8] {
    let bytes = crate::memory::bytes_until_nul(s);
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx] != b'\n' && mb_get_class(&bytes[idx..]) <= 1 {
        idx += crate::mbyte::utfc_slice2len(&bytes[idx..]).max(1);
    }
    &s[idx.min(s.len())..]
}

pub fn find_word_end(s: &[u8]) -> &[u8] {
    let bytes = crate::memory::bytes_until_nul(s);
    let mut idx = 0usize;
    let start_class = mb_get_class(bytes);
    if start_class > 1 {
        while idx < bytes.len() {
            idx += crate::mbyte::utfc_slice2len(&bytes[idx..]).max(1);
            if mb_get_class(&bytes[idx.min(bytes.len())..]) != start_class {
                break;
            }
        }
    }
    &s[idx.min(s.len())..]
}

pub fn find_line_end(s: &[u8]) -> &[u8] {
    let bytes = crate::memory::bytes_until_nul(s);
    let mut len = bytes.len();
    while len > 0 && (bytes[len - 1] == CAR as u8 || bytes[len - 1] == NL as u8) {
        len -= 1;
    }
    &s[len..]
}

fn ins_compl_free() {
    insexp().compl_pattern = None;
    api_clear_string(&mut insexp().compl_leader);

    let list = &mut insexp().compl_list;
    if list.is_empty() {
        return;
    }

    ins_compl_del_pum();
    pum_clear();

    let ht_ptr = &mut insexp().compl_strings_ht;
    ht_ptr.clear();

    list.clear();
    insexp().compl_is_cyclic = false;
    insexp().compl_curr_idx = None;
    insexp().compl_shown_idx = None;
    insexp().compl_preselect_idx = None;
    insexp().compl_old_idx = None;
}

pub fn ins_compl_clear() {
    insexp().compl_cont_status = 0;
    insexp().compl_started = false;
    insexp().compl_matches = 0;
    insexp().compl_selected_item = -1;
    insexp().compl_ins_end_col = 0;
    insexp().compl_longest_end_col = 0;
    insexp().compl_curr_win = crate::globals::WinPtr::NULL;
    insexp().compl_curr_buf = crate::globals::BufPtr::NULL;
    insexp().compl_pattern = None;
    api_clear_string(&mut insexp().compl_leader);
    g_st().edit_submode_extra = None;
    let extmarks = &mut insexp().compl_orig_extmarks;
    extmarks.clear();
    api_clear_string(&mut insexp().compl_orig_text);
    insexp().compl_enter_selects = false;
    cpt_sources_clear();
    insexp().compl_autocomplete = false;
    insexp().compl_from_nonkeyword = false;
    insexp().compl_autostarted = false;
    insexp().compl_num_bests = 0;
    set_vim_var_dict(VV_COMPLETED_ITEM, Some(tv_dict_alloc_lock(VAR_FIXED)));
}

pub fn ins_compl_active() -> bool {
    insexp().compl_started
}

pub fn ins_compl_win_active(wp: &mut WinInfoInfo) -> bool {
    ins_compl_active() && insexp().compl_curr_win == wp && insexp().compl_curr_buf == wp.w_buffer
}

pub fn ins_compl_used_match() -> bool {
    insexp().compl_used_match
}

pub fn ins_compl_init_get_longest() {
    insexp().compl_get_longest = false;
}

pub fn ins_compl_interrupted() -> bool {
    insexp().compl_interrupted || insexp().compl_time_slice_expired
}

pub fn ins_compl_enter_selects() -> bool {
    insexp().compl_enter_selects
}

pub fn ins_compl_col() -> Colnr {
    insexp().compl_col
}

pub fn ins_compl_len() -> i32 {
    insexp().compl_length
}

pub fn ins_compl_has_preinsert() -> bool {
    let cur_cot_flags = get_cot_flags();
    if insexp().compl_autocomplete && opt_vars().p_ic && !opt_vars().p_inf {
        return false;
    }
    if !insexp().compl_autocomplete {
        (cur_cot_flags & (K_OPT_COT_FLAG_PREINSERT | K_OPT_COT_FLAG_FUZZY | K_OPT_COT_FLAG_MENUONE))
            == (K_OPT_COT_FLAG_PREINSERT | K_OPT_COT_FLAG_MENUONE)
    } else {
        (cur_cot_flags & (K_OPT_COT_FLAG_PREINSERT | K_OPT_COT_FLAG_FUZZY))
            == K_OPT_COT_FLAG_PREINSERT
    }
}

pub fn ins_compl_preinsert_effect() -> bool {
    if !ins_compl_has_preinsert() && !ins_compl_preinsert_longest() {
        return false;
    }
    curwin_mut().w_cursor.col < insexp().compl_ins_end_col
}

pub fn ins_compl_bs() -> i32 {
    if ins_compl_preinsert_effect() {
        ins_compl_delete(false);
    }

    let line_slice = get_cursor_line_slice();
    let mut p_idx = (curwin_mut().w_cursor.col as usize).min(line_slice.len());
    crate::mbyte::mb_idx_back(line_slice, &mut p_idx);
    let p_off = p_idx as isize;

    if (p_off as i32) - insexp().compl_col < 0
        || ((p_off as i32) - insexp().compl_col == 0 && !ctrl_x_mode_omni())
        || ctrl_x_mode_eval()
        || (!can_bs(BS_START) && (p_off as i32) - insexp().compl_col - insexp().compl_length < 0)
    {
        return K_BS;
    }

    if curwin_mut().w_cursor.col <= insexp().compl_col + insexp().compl_length
        || ins_compl_need_restart()
    {
        ins_compl_restart();
    }

    let line_slice = crate::cursor::get_cursor_line_slice();
    let start_u = (insexp().compl_col.max(0) as usize).min(line_slice.len());
    let len_u =
        ((p_off - insexp().compl_col as isize).max(0) as usize).min(line_slice.len() - start_u);

    api_clear_string(&mut insexp().compl_leader);
    insexp().compl_leader = cbuf_to_string(&line_slice[start_u..start_u + len_u]);

    if insexp().compl_autocomplete && !insexp().compl_list.is_empty() && !ins_compl_has_preinsert()
    {
        insexp().compl_shown_idx = Some(0);
    }

    ins_compl_new_leader();
    if { insexp().compl_shown_idx }.is_some() {
        insexp().compl_curr_idx = insexp().compl_shown_idx;
    }
    NUL as i32
}

#[inline]
fn ins_compl_refresh_always() -> bool {
    (ctrl_x_mode_function() || ctrl_x_mode_omni()) && insexp().compl_opt_refresh_always
}

#[inline]
fn ins_compl_need_restart() -> bool {
    insexp().compl_was_interrupted || ins_compl_refresh_always()
}

pub fn ins_compl_has_autocomplete() -> bool {
    curbuf_mut().b_p_ac.unwrap_or(opt_vars().p_ac)
}

fn ins_compl_fuzzy_sort() {
    let cur_cot_flags = get_cot_flags();

    set_fuzzy_score();
    if (cur_cot_flags & K_OPT_COT_FLAG_NOSORT) == 0 {
        sort_compl_match_list(cp_compare_fuzzy);
        if (cur_cot_flags & (K_OPT_COT_FLAG_NOINSERT | K_OPT_COT_FLAG_NOSELECT))
            == K_OPT_COT_FLAG_NOINSERT
        {
            let target = if compl_shows_dir_forward() {
                Some(0)
            } else {
                compl_prev_idx(0)
            };
            let none_selected = insexp().compl_shown_idx == target;
            if !none_selected {
                insexp().compl_shown_idx =
                    if !insexp().compl_autocomplete && compl_shows_dir_forward() {
                        compl_next_idx(0)
                    } else {
                        Some(0)
                    };
            }
        }
    }
}

fn ins_compl_new_leader() {
    ins_compl_del_pum();
    ins_compl_delete(true);
    let lead_b = insexp().compl_leader.as_bytes();
    let skip = (get_compl_len().max(0) as usize).min(lead_b.len());
    ins_compl_insert_bytes(&lead_b[skip..], -1);
    insexp().compl_used_match = false;

    let save_w_wrow = curwin_mut().w_wrow;
    let save_w_leftcol = curwin_mut().w_leftcol;
    if insexp().compl_started {
        ins_compl_set_original_text(insexp().compl_leader.as_bytes());
        if is_cpt_func_refresh_always() {
            cpt_compl_refresh();
        }
        if cot_fuzzy() {
            ins_compl_fuzzy_sort();
        }
    } else {
        insexp().spell_bad_len = 0;
        insexp().compl_restarting = true;
        if ins_compl_has_autocomplete() {
            ins_compl_enable_autocomplete();
        } else {
            insexp().compl_autocomplete = false;
        }
        if !ins_complete(CTRL_N, false) {
            insexp().compl_cont_status = 0;
        }
        insexp().compl_restarting = false;
    }

    insexp().compl_enter_selects = !insexp().compl_used_match && insexp().compl_selected_item != -1;

    if !insexp().compl_interrupted {
        show_pum(save_w_wrow, save_w_leftcol);
    }

    let arr = &insexp().compl_match_array;
    if arr.is_empty() {
        insexp().compl_enter_selects = false;
    } else if ins_compl_has_preinsert() && insexp().compl_leader.size > 0 {
        ins_compl_insert(true, false);
    } else if insexp().compl_started
        && ins_compl_preinsert_longest()
        && insexp().compl_leader.size > 0
        && !ins_compl_preinsert_effect()
    {
        ins_compl_insert(true, true);
    }
    if ins_compl_refresh_always() {
        insexp().compl_enter_selects = false;
    }
}

#[inline]
fn get_compl_len() -> i32 {
    (curwin_mut().w_cursor.col - insexp().compl_col).max(0)
}

pub fn ins_compl_addleader(c: i32) {
    if ins_compl_preinsert_effect() {
        ins_compl_delete(false);
    }

    if !stop_arrow() {
        return;
    }
    let cc = utf_char2len(c);
    if cc > 1 {
        let mut buf: [u8; MB_MAXCHAR + 1] = [0; MB_MAXCHAR + 1];
        utf_char2bytes(c, &mut buf);
        buf[cc as usize] = 0;
        ins_char_bytes(&buf, cc as usize);
    } else {
        ins_char(c);
    }

    if ins_compl_need_restart() {
        ins_compl_restart();
    }

    api_clear_string(&mut insexp().compl_leader);
    let line_slice = crate::cursor::get_cursor_line_slice();
    let start_u = (insexp().compl_col.max(0) as usize).min(line_slice.len());
    let len_u = ((curwin_mut().w_cursor.col - insexp().compl_col).max(0) as usize)
        .min(line_slice.len() - start_u);
    insexp().compl_leader = cbuf_to_string(&line_slice[start_u..start_u + len_u]);
    ins_compl_new_leader();
}

fn ins_compl_restart() {
    update_screen();
    ins_compl_free();
    insexp().compl_started = false;
    insexp().compl_matches = 0;
    insexp().compl_cont_status = 0;
    insexp().compl_cont_mode = 0;
    cpt_sources_clear();
    insexp().compl_autocomplete = false;
    insexp().compl_from_nonkeyword = false;
    insexp().compl_autostarted = false;
    insexp().compl_num_bests = 0;
}

fn ins_compl_set_original_text(bytes: &[u8]) {
    let Some(orig_idx) = find_original_text_match_idx() else {
        return;
    };
    let match_ = compl_at(orig_idx);
    api_clear_string(&mut match_.cp_str);
    let b = crate::memory::bytes_until_nul(bytes);
    match_.cp_str = cbuf_to_string(b);
}

pub fn ins_compl_addfrommatch() {
    let len = curwin_mut().w_cursor.col - insexp().compl_col;
    let shown_idx = insexp().compl_shown_idx.unwrap();
    let shown = compl_at(shown_idx);
    let mut p = shown.cp_str.data;
    if (shown.cp_str.size as i32) <= len {
        if !match_at_original_text(&*shown) {
            return;
        }

        p = crate::globals::GraphPtr::NULL;
        let mut plen: usize = 0;
        let mut cp_idx = compl_next_idx(shown_idx);
        while let Some(ci) = cp_idx
            && ci != 0
        {
            let cp = compl_at(ci);
            if insexp().compl_leader.data.is_null()
                || ins_compl_equal(&mut *cp, insexp().compl_leader.as_bytes())
            {
                p = cp.cp_str.data;
                plen = cp.cp_str.size;
                break;
            }
            cp_idx = compl_next_idx(ci);
        }
        if p.is_null() || (plen as i32) <= len {
            return;
        }
    }
    p = p.wrapping_add(len as usize);
    let c = utf_ptr2char(p);
    ins_compl_addleader(c);
}

fn set_ctrl_x_mode(c: i32) -> bool {
    let mut retval = false;

    match c {
        CTRL_E | CTRL_Y => {
            insexp().ctrl_x_mode = CTRL_X_SCROLL;
            if (g_st().state_global & REPLACE_FLAG) == 0 {
                g_st().edit_submode = Some(gettext(c" (insert) Scroll (^E/^Y)"));
            } else {
                g_st().edit_submode = Some(gettext(c" (replace) Scroll (^E/^Y)"));
            }
            g_st().edit_submode_pre = None;
            g_st().redraw_mode = true;
        }
        CTRL_L_CONST => {
            insexp().ctrl_x_mode = CTRL_X_WHOLE_LINE;
        }
        CTRL_F_CONST => {
            insexp().ctrl_x_mode = CTRL_X_FILES;
        }
        CTRL_K_CONST => {
            insexp().ctrl_x_mode = CTRL_X_DICTIONARY;
        }
        CTRL_R_CONST => {
            if vpeekc() != b'=' as i32 {
                insexp().ctrl_x_mode = CTRL_X_REGISTER;
            }
        }
        CTRL_T_CONST => {
            insexp().ctrl_x_mode = CTRL_X_THESAURUS;
        }
        CTRL_U => {
            insexp().ctrl_x_mode = CTRL_X_FUNCTION;
        }
        CTRL_O => {
            insexp().ctrl_x_mode = CTRL_X_OMNI;
        }
        x if x == b's' as i32 || x == CTRL_S_CONST => {
            insexp().ctrl_x_mode = CTRL_X_SPELL;
            g_st().emsg_off += 1;
            spell_back_to_badword();
            g_st().emsg_off -= 1;
        }
        CTRL_RSB_CONST => {
            insexp().ctrl_x_mode = CTRL_X_TAGS;
        }
        CTRL_I_CONST | K_S_TAB => {
            insexp().ctrl_x_mode = CTRL_X_PATH_PATTERNS;
        }
        CTRL_D_CONST => {
            insexp().ctrl_x_mode = CTRL_X_PATH_DEFINES;
        }
        CTRL_V_CONST | CTRL_Q_CONST => {
            insexp().ctrl_x_mode = CTRL_X_CMDLINE;
        }
        CTRL_Z_CONST => {
            insexp().ctrl_x_mode = CTRL_X_NORMAL;
            g_st().edit_submode = None;
            g_st().redraw_mode = true;
            retval = true;
        }
        CTRL_P | CTRL_N => {
            if (insexp().compl_cont_status & CONT_INTRPT) == 0 {
                insexp().compl_cont_status |= CONT_LOCAL;
            } else if insexp().compl_cont_mode != 0 {
                insexp().compl_cont_status &= !CONT_LOCAL;
            }
            insexp().ctrl_x_mode = CTRL_X_NORMAL;
            g_st().edit_submode = None;
            g_st().redraw_mode = true;
        }
        _ => {
            if c == CTRL_X_CONST {
                if insexp().compl_cont_mode != 0 {
                    insexp().compl_cont_status = 0;
                } else {
                    insexp().compl_cont_mode = CTRL_X_NOT_DEFINED_YET;
                }
            }
            insexp().ctrl_x_mode = CTRL_X_NORMAL;
            g_st().edit_submode = None;
            g_st().redraw_mode = true;
        }
    }

    retval
}

fn ins_compl_stop(c: i32, prev_mode: i32, mut retval: bool) -> bool {
    if ins_compl_preinsert_effect() && ins_compl_win_active(curwin_mut()) {
        ins_compl_delete(false);
    }

    let has_curr = compl_opt(insexp().compl_curr_idx).is_some();
    if has_curr || !insexp().compl_leader.data.is_null() || c == CTRL_E {
        let ptr_opt = if insexp().compl_used_match && c != CTRL_E {
            compl_opt(insexp().compl_curr_idx).and_then(|curr| curr.cp_str.as_opt_cstr())
        } else {
            None
        };
        ins_compl_fix_redo_buf_for_leader(ptr_opt);
    }

    let mut want_cindent = get_can_cindent() && crate::indent_c::cindent_on();

    if insexp().compl_cont_mode == CTRL_X_WHOLE_LINE {
        if want_cindent {
            do_c_expr_indent();
            want_cindent = false;
        }
    } else if !insexp().compl_autocomplete || insexp().compl_used_match {
        let prev_col = curwin_mut().w_cursor.col;

        if prev_col > 0 {
            dec_cursor();
        }

        if g_st().ins_global.moved == K_INS_NONE && !g_st().ins_global.need_undo && c != CTRL_E {
            insertchar(NUL as i32, 0, -1);
        }

        if prev_col > 0 && !crate::cursor::get_cursor_pos_slice().is_empty() {
            inc_cursor();
        }
    }

    let mut word_owned: Option<std::ffi::CString> = None;
    let is_commit = pum_visible() && ins_compl_commit_char(c);
    if (c == CTRL_Y
        || is_commit
        || (insexp().compl_enter_selects && (c == CAR || c == K_KENTER || c == NL)))
        && pum_visible()
    {
        if let Some(shown) = compl_opt(insexp().compl_shown_idx) {
            word_owned = Some(shown.cp_str.to_cstring());
        }

        if !is_commit {
            retval = true;
        }
        redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
    }

    let arr = &insexp().compl_match_array;
    if word_owned.is_none()
        && c != CTRL_E
        && insexp().compl_used_match
        && arr.is_empty()
        && let Some(curr) = compl_opt(insexp().compl_curr_idx)
        && !curr.cp_str.data.is_null()
    {
        word_owned = Some(curr.cp_str.to_cstring());
    }

    if c == CTRL_E {
        ins_compl_delete(false);
        let p_slice: Option<&[u8]> = if !insexp().compl_leader.data.is_null() {
            Some(insexp().compl_leader.as_bytes())
        } else if !insexp().compl_list.is_empty() {
            Some(insexp().compl_orig_text.as_bytes())
        } else {
            None
        };
        if let Some(p_b) = p_slice {
            let compl_len = get_compl_len();
            if (p_b.len() as i32) > compl_len {
                let start = compl_len.max(0) as usize;
                ins_compl_insert_bytes(&p_b[start..], (p_b.len() as i32) - compl_len);
            }
        }
        restore_orig_extmarks();
        retval = true;
    }

    auto_format(false, true);

    insexp().ctrl_x_mode = prev_mode;
    ins_apply_autocmds(EVENT_COMPLETEDONEPRE);

    ins_compl_free();
    insexp().compl_started = false;
    insexp().compl_matches = 0;
    if !shortmess(K_SHM_COMPLETIONMENU) {
        msg_clr_cmdline();
    }
    insexp().ctrl_x_mode = CTRL_X_NORMAL;
    insexp().compl_enter_selects = false;
    if g_st().edit_submode.is_some() {
        g_st().edit_submode = None;
        g_st().redraw_mode = true;
    }
    insexp().compl_autocomplete = false;
    insexp().compl_from_nonkeyword = false;
    insexp().compl_autostarted = false;
    insexp().compl_num_bests = 0;
    insexp().compl_ins_end_col = 0;

    if c == CTRL_C_CONST && bt_cmdwin(g_st().curbuf.opt()) {
        update_screen();
    }

    if want_cindent && in_cinkeys(KEY_COMPLETE, b' ' as i32, inindent(0)) {
        do_c_expr_indent();
    }
    let word = match word_owned {
        Some(ref w) => NvimString::from_cstr(w),
        None => empty_nvim_string(),
    };
    do_autocmd_completedone(c, prev_mode, Some(&word));

    retval
}

pub fn ins_compl_cancel() -> bool {
    ins_compl_stop(b' ' as i32, insexp().ctrl_x_mode, true)
}

pub fn ins_compl_prep(c: i32) -> bool {
    let mut retval = false;
    let prev_mode = insexp().ctrl_x_mode;

    if c != CTRL_R_CONST && vim_is_ctrl_x_key(c) {
        g_st().edit_submode_extra = None;
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

    if insexp().ctrl_x_mode == CTRL_X_CMDLINE_CTRL_X && c != CTRL_X_CONST {
        if c == CTRL_V_CONST
            || c == CTRL_Q_CONST
            || c == CTRL_Z_CONST
            || ins_compl_pum_key(c)
            || !vim_is_ctrl_x_key(c)
        {
            insexp().ctrl_x_mode = CTRL_X_CMDLINE;
            if c == CTRL_Z_CONST {
                retval = true;
            }
        } else {
            insexp().ctrl_x_mode = CTRL_X_CMDLINE;
            ins_compl_prep(b' ' as i32);
            insexp().ctrl_x_mode = CTRL_X_NOT_DEFINED_YET;
        }
    }

    if ctrl_x_mode_not_defined_yet() || (ctrl_x_mode_normal() && !insexp().compl_started) {
        insexp().compl_get_longest = (get_cot_flags() & K_OPT_COT_FLAG_LONGEST) != 0;
        insexp().compl_used_match = true;
    }

    if ctrl_x_mode_not_defined_yet() {
        retval = set_ctrl_x_mode(c);
    } else if ctrl_x_mode_not_default() {
        if c != CTRL_R_CONST && !vim_is_ctrl_x_key(c) {
            insexp().ctrl_x_mode = if ctrl_x_mode_scroll() {
                CTRL_X_NORMAL
            } else {
                CTRL_X_FINISHED
            };
            g_st().edit_submode = None;
        }
        g_st().redraw_mode = true;
    }

    if insexp().compl_started || insexp().ctrl_x_mode == CTRL_X_FINISHED {
        g_st().redraw_mode = true;
        if (ctrl_x_mode_normal()
            && c != CTRL_N
            && c != CTRL_P
            && c != CTRL_R_CONST
            && !ins_compl_pum_key(c))
            || insexp().ctrl_x_mode == CTRL_X_FINISHED
        {
            retval = ins_compl_stop(c, prev_mode, retval);
        }
    } else if insexp().ctrl_x_mode == CTRL_X_LOCAL_MSG {
        do_autocmd_completedone(c, insexp().ctrl_x_mode, None);
    }

    may_trigger_modechanged();

    if c != CTRL_R_CONST && !vim_is_ctrl_x_key(c) {
        insexp().compl_cont_status = 0;
        insexp().compl_cont_mode = 0;
    }

    retval
}

fn ins_compl_fix_redo_buf_for_leader(ptr_arg: Option<&CStr>) {
    let mut len: usize = 0;
    let ptr_cstr = match ptr_arg {
        Some(s) => s,
        None => {
            if !insexp().compl_leader.data.is_null() {
                insexp().compl_leader.as_cstr()
            } else {
                return;
            }
        }
    };
    let ptr_bytes = ptr_cstr.to_bytes();
    if !insexp().compl_orig_text.data.is_null() {
        let orig_bytes = insexp().compl_orig_text.as_bytes();
        len = orig_bytes
            .iter()
            .zip(ptr_bytes.iter())
            .take_while(|&(a, b)| a == b)
            .count();
        if len > 0 {
            len -= crate::mbyte::utf_head_off(orig_bytes, len) as usize;
        }
        let mut rem = &orig_bytes[len..];
        while !rem.is_empty() {
            redo_append_char(K_BS);
            let step = utf_ptr2len(rem.as_ptr()) as usize;
            rem = &rem[step.max(1).min(rem.len())..];
        }
    }
    redo_append_lit(&ptr_bytes[len.min(ptr_bytes.len())..]);
}

fn ins_compl_next_buf(buf_arg: &mut FileBuf, flag: i32) -> &'static mut FileBuf {
    let mut buf = crate::globals::GraphPtr::<FileBuf>::from_ref(buf_arg);
    if flag == b'w' as i32 {
        if g_st().curbuf == buf || !win_valid(insexp().next_buf_wp.opt()) {
            insexp().next_buf_wp = crate::globals::curwin_ptr();
        }

        debug_assert!(!insexp().next_buf_wp.is_null());
        loop {
            let wp = insexp().next_buf_wp;
            let next_wp = if !wp.w_next.is_null() {
                wp.w_next.as_ptr()
            } else {
                g_st().firstwin.as_ptr()
            };
            insexp().next_buf_wp = crate::globals::WinPtr::new(next_wp);
            if next_wp == curwin_ptr()
                || ({
                    let nw = crate::globals::GraphPtr::of(next_wp).borrow_mut();
                    !nw.buf_mut().b_scanned && nw.w_config.focusable
                })
            {
                break;
            }
        }

        buf = insexp().next_buf_wp.w_buffer;
    } else {
        loop {
            buf = if !buf.b_next.is_null() {
                buf.b_next
            } else {
                g_st().firstbuf
            };
            if g_st().curbuf == buf {
                break;
            }

            let skip_buffer = if flag == b'U' as i32 {
                buf.b_p_bl
            } else {
                !buf.b_p_bl || buf.b_ml.ml_mfp.is_none() != (flag == b'u' as i32)
            };

            if !skip_buffer && !buf.b_scanned {
                break;
            }
        }
    }
    buf.borrow_mut()
}

fn get_cpt_sources_count(cpt: &CStr) -> i32 {
    let mut dummy: [u8; LSIZE] = [0; LSIZE];
    let mut count: i32 = 0;
    let mut p = cpt.to_bytes();

    while !p.is_empty() {
        while matches!(p.first(), Some(b',' | b' ')) {
            p = &p[1..];
        }
        if !p.is_empty() {
            let _ = copy_option_part(&mut p, &mut dummy, c",");
            count += 1;
        }
    }

    count
}

pub fn clear_cpt_callbacks(callbacks: &mut Vec<Callback>) {
    for cb in callbacks.iter_mut() {
        callback_free(cb);
    }
    callbacks.clear();
}

fn copy_cpt_callbacks(dest: &mut Vec<Callback>, src: &mut [Callback]) {
    clear_cpt_callbacks(dest);

    if src.is_empty() {
        return;
    }

    dest.resize(src.len(), Callback::new());

    for (dst_cb, src_cb) in dest.iter_mut().zip(src.iter_mut()) {
        if src_cb.cb_type() != K_CALLBACK_NONE_CONST {
            callback_copy(dst_cb, src_cb);
        }
    }
}

pub fn set_buflocal_cpt_callbacks(buf: Option<&mut FileBuf>) {
    if buf.is_none() {
        return;
    }
    let global_cb = &mut insexp().cpt_cb;
    copy_cpt_callbacks(&mut buf.unwrap().b_p_cpt_cb, global_cb);
}

fn parse_cpt_callbacks(cpt: &CStr, cbp: &mut Vec<Callback>) {
    clear_cpt_callbacks(cbp);

    let count = get_cpt_sources_count(cpt);
    if count == 0 {
        return;
    }

    cbp.resize(count as usize, Callback::new());

    let mut buf: [u8; LSIZE] = [0; LSIZE];
    let mut idx: usize = 0;
    let mut p = cpt.to_bytes();
    while !p.is_empty() {
        while matches!(p.first(), Some(b',' | b' ')) {
            p = &p[1..];
        }
        if !p.is_empty() {
            let slen = copy_option_part(&mut p, &mut buf, c",");
            if slen > 0 && buf[0] == b'F' && buf[1] != 0 {
                if let Some(caret_idx) = buf[..slen].iter().position(|&b| b == b'^') {
                    buf[caret_idx] = 0;
                }
                if !option_set_callback_func(&buf[1..], &mut cbp[idx]) {
                    cbp[idx] = Callback::NONE;
                }
            }
            idx += 1;
        }
    }
}

pub fn set_cpt_callbacks(args: &mut Optset) -> bool {
    let opt_flags = args.os_flags;

    if g_st().curbuf.is_null() {
        return false;
    }

    if (opt_flags & OPT_GLOBAL) == 0 {
        parse_cpt_callbacks(curbuf_mut().b_p_cpt.as_cstr(), &mut curbuf_mut().b_p_cpt_cb);
    }

    if (opt_flags & OPT_LOCAL) != 0 {
        return true;
    }

    let global_cb = &mut insexp().cpt_cb;
    if (opt_flags & OPT_GLOBAL) != 0 {
        parse_cpt_callbacks(opt_vars().p_cpt.as_cstr(), global_cb);
        return true;
    }

    copy_cpt_callbacks(global_cb, &mut curbuf_mut().b_p_cpt_cb);
    true
}

pub fn set_ref_in_cpt_callbacks(callbacks: &mut [Callback], copy_id: i32) -> bool {
    let mut abort = false;
    for cb in callbacks {
        abort = abort || set_ref_in_callback(cb, copy_id, None, None);
    }
    abort
}

pub fn set_ref_in_insexpand_funcs(copy_id: i32) -> bool {
    let mut abort = set_ref_in_callback(&mut opt_vars().p_cfu, copy_id, None, None);
    abort = abort || set_ref_in_callback(&mut opt_vars().p_ofu, copy_id, None, None);
    abort = abort || set_ref_in_callback(&mut opt_vars().p_tsrfu, copy_id, None, None);
    let global_cb = &mut insexp().cpt_cb;
    abort = abort || set_ref_in_cpt_callbacks(global_cb, copy_id);
    abort
}

fn get_insert_callback(type_: i32) -> &'static mut Callback {
    if type_ == CTRL_X_FUNCTION {
        return &mut curbuf_mut().b_p_cfu;
    }
    if type_ == CTRL_X_OMNI {
        return &mut curbuf_mut().b_p_ofu;
    }
    if curbuf_mut().b_p_tsrfu.cb_type() != K_CALLBACK_NONE_CONST {
        &mut curbuf_mut().b_p_tsrfu
    } else {
        &mut opt_vars().borrow_mut().p_tsrfu
    }
}

fn expand_by_function(type_: i32, base: Option<&CStr>, cb_opt: Option<&mut Callback>) {
    let mut matchlist = crate::globals::GraphPtr::<TvList>::NULL;
    let mut matchdict = crate::globals::GraphPtr::<TvDict>::NULL;
    let mut rettv: TypVal = TypVal::new();
    let save_state = g_st().state_global;

    debug_assert!(!g_st().curbuf.is_null());

    let is_cpt_function = cb_opt.is_some();
    let use_sandbox = is_cpt_function
        && was_set_insecurely(
            Some(crate::globals::curwin_mut()),
            K_OPT_COMPLETE,
            OPT_LOCAL,
        ) != 0;
    let cb = match cb_opt {
        Some(c) => c,
        None => {
            let c = get_insert_callback(type_);
            if c.cb_type() == K_CALLBACK_NONE_CONST {
                return;
            }
            c
        }
    };

    let mut args: [TypVal; 3] = [const { TypVal::new() }; 3];
    args[0].v_type = VAR_NUMBER;
    args[1].v_type = VAR_STRING;
    args[2].v_type = VAR_UNKNOWN;
    args[0].vval = crate::eval::typval::TypvalVval::Number(0);
    args[1].vval = crate::eval::typval::TypvalVval::from_cstr(base.unwrap_or(c""));

    let pos = curwin_mut().w_cursor;
    g_st().textlock += 1;
    if use_sandbox {
        g_st().sandbox += 1;
    }

    let called = callback_call(cb, 2, &mut args, &mut rettv);
    if use_sandbox {
        g_st().sandbox -= 1;
    }
    if called {
        match rettv.v_type {
            VAR_LIST => {
                matchlist = crate::globals::GraphPtr::<TvList>::of(rettv.vval.v_list());
            }
            VAR_DICT => {
                matchdict = crate::globals::GraphPtr::<TvDict>::of(rettv.vval.v_dict());
            }
            _ => {
                tv_clear(&mut rettv);
            }
        }
    }
    g_st().textlock -= 1;

    curwin_mut().w_cursor = pos;
    check_cursor(curwin_mut());
    validate_cursor(curwin_mut());
    if !equalpos(curwin_mut().w_cursor, pos) {
        emsg(gettext(E_COMPLDEL));
    } else if !matchlist.is_null() {
        ins_compl_add_list(matchlist.opt());
    } else if !matchdict.is_null() {
        ins_compl_add_dict(matchdict.borrow_mut());
    }

    g_st().state_global = save_state;

    if !matchdict.is_null() {
        tv_dict_unref(matchdict.opt_mut());
    }
    if !matchlist.is_null() {
        tv_list_unref(matchlist.opt_mut());
    }
}

#[inline]
fn get_user_highlight_id(hlname: Option<&CStr>) -> i32 {
    if let Some(s) = hlname
        && !s.to_bytes().is_empty()
    {
        return syn_check_group(s.to_bytes());
    }
    0
}

fn ins_compl_add_tv(tv: &mut TypVal, dir: Direction, fast: bool) -> Option<bool> {
    let word;
    let mut dup = false;
    let mut empty = false;
    let mut flags = if fast { CP_FAST } else { 0 };
    let mut preselect = false;
    let mut cptext: [Option<std::ffi::CString>; CPT_COUNT] = [const { None }; CPT_COUNT];
    let mut commit_chars: Option<std::ffi::CString> = None;
    let mut user_hl: [i32; 2] = [0, 0];
    let mut user_data: TypVal = TypVal::new();
    user_data.v_type = VAR_UNKNOWN;

    if tv.v_type == VAR_DICT && !tv.vval.v_dict().is_null() {
        let d = tv.vval.v_dict_ref();
        word =
            tv_dict_get_string(Some(d), c"word").map_or(ptr::null(), |s| s.as_ptr().cast::<u8>());
        cptext[CPT_ABBR] = crate::eval::typval::tv_dict_get_cstring(Some(d), c"abbr");
        cptext[CPT_MENU] = crate::eval::typval::tv_dict_get_cstring(Some(d), c"menu");
        cptext[CPT_KIND] = crate::eval::typval::tv_dict_get_cstring(Some(d), c"kind");
        cptext[CPT_INFO] = crate::eval::typval::tv_dict_get_cstring(Some(d), c"info");
        commit_chars = crate::eval::typval::tv_dict_get_cstring(Some(d), c"commit_chars");

        let user_abbr_hlname = tv_dict_get_string(Some(d), c"abbr_hlgroup");
        user_hl[0] = get_user_highlight_id(user_abbr_hlname);

        let user_kind_hlname = tv_dict_get_string(Some(d), c"kind_hlgroup");
        user_hl[1] = get_user_highlight_id(user_kind_hlname);

        tv_dict_get_tv(Some(d), c"user_data", &mut user_data);

        if tv_dict_get_number(Some(d), c"icase") != 0 {
            flags |= CP_ICASE;
        }
        dup = tv_dict_get_number(Some(d), c"dup") != 0;
        empty = tv_dict_get_number(Some(d), c"empty") != 0;
        preselect = tv_dict_get_number(Some(d), c"preselect") != 0;
        if tv_dict_get_string(Some(d), c"equal").is_some()
            && tv_dict_get_number(Some(d), c"equal") != 0
        {
            flags |= CP_EQUAL;
        }
    } else {
        word = tv_get_string_chk(&*tv).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
    }
    if word.is_null() || (!empty && *crate::globals::GraphPtr::of(word).borrow() == 0) {
        tv_clear(&mut user_data);
        return Some(false);
    }
    let status = ins_compl_add(
        unsafe { CStr::from_ptr(word.cast()) }.to_bytes(),
        -1,
        None,
        cptext,
        Some(&mut user_data),
        dir,
        flags,
        dup,
        Some(&user_hl),
        FUZZY_SCORE_NONE,
        preselect,
        commit_chars,
    );
    if status == Some(false) {
        tv_clear(&mut user_data);
    }
    status
}

fn ins_compl_add_list(list: Option<&TvList>) {
    let mut dir = insexp().compl_direction;

    if list.is_none() {
        return;
    }
    for li in crate::eval::typval::list_iter_mut(list.unwrap().first_opt()) {
        if ins_compl_add_tv(&mut li.li_tv, dir, true) != Some(false) {
            dir = FORWARD;
        } else if g_st().did_emsg != 0 {
            break;
        }
    }
}

fn ins_compl_add_dict(dict: &mut TvDict) {
    insexp().compl_opt_refresh_always = false;
    if let Some(di_refresh) = crate::eval::typval::tv_dict_find_opt(Some(dict), c"refresh", 7)
        && di_refresh.di_tv.v_type == VAR_STRING
        && di_refresh
            .di_tv
            .vval
            .v_string_cstr()
            .is_some_and(|s| s.to_bytes() == b"always")
    {
        insexp().compl_opt_refresh_always = true;
    }

    if let Some(di_words) = crate::eval::typval::tv_dict_find_opt(Some(dict), c"words", 5)
        && di_words.di_tv.v_type == VAR_LIST
    {
        ins_compl_add_list(di_words.di_tv.vval.v_list_opt().as_deref());
    }
}

fn save_orig_extmarks() {
    extmark_splice_delete(
        curbuf_mut(),
        curwin_mut().w_cursor.lnum - 1,
        insexp().compl_col,
        curwin_mut().w_cursor.lnum - 1,
        insexp().compl_col + insexp().compl_length,
        Some(&mut insexp().compl_orig_extmarks),
        true,
        K_EXTMARK_UNDO,
    );
}

fn restore_orig_extmarks() {
    let extmarks = &insexp().compl_orig_extmarks;
    for &undo_info in extmarks.iter().rev() {
        extmark_apply_undo(undo_info, true);
    }
}

fn set_completion(mut startcol: Colnr, list: Option<&TvList>) {
    let mut flags = CP_ORIGINAL_TEXT;
    let cur_cot_flags = get_cot_flags();
    let compl_longest = (cur_cot_flags & K_OPT_COT_FLAG_LONGEST) != 0;
    let compl_no_insert = (cur_cot_flags & K_OPT_COT_FLAG_NOINSERT) != 0;
    let compl_no_select = (cur_cot_flags & K_OPT_COT_FLAG_NOSELECT) != 0;

    if insexp().compl_started || ctrl_x_mode_not_default() {
        ins_compl_prep(b' ' as i32);
    }
    ins_compl_clear();
    ins_compl_free();
    insexp().compl_get_longest = compl_longest;

    insexp().compl_direction = FORWARD;
    if startcol > curwin_mut().w_cursor.col {
        startcol = curwin_mut().w_cursor.col;
    }
    insexp().compl_col = startcol;
    insexp().compl_lnum = curwin_mut().w_cursor.lnum;
    insexp().compl_length = curwin_mut().w_cursor.col - startcol;
    let line_slice = crate::cursor::get_cursor_line_slice();
    let start_u = (insexp().compl_col.max(0) as usize).min(line_slice.len());
    let len_u = (insexp().compl_length.max(0) as usize).min(line_slice.len() - start_u);
    insexp().compl_orig_text = cbuf_to_string(&line_slice[start_u..start_u + len_u]);
    save_orig_extmarks();
    if opt_vars().p_ic {
        flags |= CP_ICASE;
    }
    if ins_compl_add(
        insexp().compl_orig_text.as_bytes(),
        insexp().compl_orig_text.size as i32,
        None,
        [const { None }; CPT_COUNT],
        None,
        0,
        flags | CP_FAST,
        false,
        None,
        FUZZY_SCORE_NONE,
        false,
        None,
    ) != Some(true)
    {
        return;
    }

    insexp().ctrl_x_mode = CTRL_X_EVAL;

    ins_compl_add_list(list);
    insexp().compl_matches = ins_compl_make_cyclic();
    insexp().compl_started = true;
    insexp().compl_used_match = true;
    insexp().compl_cont_status = 0;
    let save_w_wrow = curwin_mut().w_wrow;
    let save_w_leftcol = curwin_mut().w_leftcol;

    insexp().compl_curr_idx = Some(0);
    let no_select = compl_no_select || compl_longest;
    if let Some(pre_idx) = insexp().compl_preselect_idx
        && !no_select
    {
        insexp().compl_curr_idx = compl_prev_idx(pre_idx);
        ins_complete(if compl_no_insert { K_DOWN } else { CTRL_N }, false);
    } else if compl_no_insert || no_select {
        ins_complete(K_DOWN, false);
        if no_select {
            ins_complete(K_UP, false);
        }
    } else {
        ins_complete(CTRL_N, false);
    }
    insexp().compl_enter_selects = compl_no_insert;

    if !insexp().compl_interrupted {
        show_pum(save_w_wrow, save_w_leftcol);
    }

    may_trigger_modechanged();
    ui_flush();
}

pub fn f_complete(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if (g_st().state_global & MODE_INSERT) == 0 {
        emsg(gettext(c"E785: complete() can only be used in Insert mode"));
        return;
    }

    if !undo_allowed(curbuf_mut()) {
        return;
    }

    if argvars[1].v_type != VAR_LIST {
        emsg(gettext(E_INVARG));
    } else {
        let startcol = tv_get_number_chk(&argvars[0], None) as Colnr;
        if startcol > 0 {
            set_completion(startcol - 1, argvars[1].vval.v_list_opt().as_deref());
        }
    }
}

pub fn f_complete_add(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let rc = match ins_compl_add_tv(&mut argvars[0], 0, false) {
        Some(true) => 1,
        Some(false) => 0,
        None => 2,
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(rc);
}

pub fn f_complete_check(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let saved = g_st().redrawing_disabled;
    g_st().redrawing_disabled = 0;
    ins_compl_check_keys(0, true);
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(ins_compl_interrupted()));
    g_st().redrawing_disabled = saved;
}

fn ins_compl_mode() -> NvimString {
    if ctrl_x_mode_not_defined_yet() || ctrl_x_mode_scroll() || insexp().compl_started {
        let idx = (insexp().ctrl_x_mode & !CTRL_X_WANT_IDENT) as usize;
        if let Some(mode_cstr) = CTRL_X_MODE_NAMES.get(idx).copied().flatten() {
            return static_cstr_as_string(mode_cstr);
        }
    }
    static_cstr_as_string(c"")
}

fn ins_compl_update_sequence_numbers() {
    let mut number: i32 = 0;
    let Some(curr_idx) = insexp().compl_curr_idx else {
        return;
    };

    if compl_dir_forward() {
        let mut match_idx = compl_prev_idx(curr_idx);
        while let Some(mi) = match_idx
            && mi != 0
        {
            let m = compl_at(mi);
            if m.cp_number != -1 {
                number = m.cp_number;
                break;
            }
            match_idx = compl_prev_idx(mi);
        }
        if let Some(mi) = match_idx {
            match_idx = compl_next_idx(mi);
            while let Some(cur_mi) = match_idx {
                let m = compl_at(cur_mi);
                if m.cp_number != -1 {
                    break;
                }
                number += 1;
                m.cp_number = number;
                match_idx = compl_next_idx(cur_mi);
            }
        }
    } else {
        debug_assert!(insexp().compl_direction == BACKWARD);
        let mut match_idx = compl_next_idx(curr_idx);
        while let Some(mi) = match_idx
            && mi != 0
        {
            let m = compl_at(mi);
            if m.cp_number != -1 {
                number = m.cp_number;
                break;
            }
            match_idx = compl_next_idx(mi);
        }
        if let Some(mi) = match_idx {
            match_idx = compl_prev_idx(mi);
            while let Some(cur_mi) = match_idx {
                let m = compl_at(cur_mi);
                if m.cp_number != -1 {
                    break;
                }
                number += 1;
                m.cp_number = number;
                match_idx = compl_prev_idx(cur_mi);
            }
        }
    }
}

fn fill_complete_info_dict(di: &mut TvDict, match_: &mut Compl, add_match: bool) {
    tv_dict_add_str_len(
        di,
        c"word",
        (!match_.cp_str.data.is_null()).then(|| match_.cp_str.as_bytes()),
    );
    tv_dict_add_str(di, c"abbr", match_.cp_text[CPT_ABBR].as_deref());
    tv_dict_add_str(
        di,
        c"menu",
        match_.cp_text[CPT_MENU]
            .as_deref()
            .or(match_.cp_fname.as_deref()),
    );
    tv_dict_add_str(di, c"kind", match_.cp_text[CPT_KIND].as_deref());
    tv_dict_add_str(di, c"info", match_.cp_text[CPT_INFO].as_deref());
    tv_dict_add_str(
        di,
        c"abbr_hlgroup",
        Some(syn_id2name(match_.cp_user_abbr_hl_id)),
    );
    tv_dict_add_str(
        di,
        c"kind_hlgroup",
        Some(syn_id2name(match_.cp_user_kind_hl_id)),
    );
    if add_match {
        tv_dict_add_bool(di, c"match", if match_.cp_in_match_array { 1 } else { 0 });
    }
    if (match_.cp_flags & CP_EQUAL) != 0 {
        tv_dict_add_nr(di, c"equal", 1);
    }
    if match_.cp_preselect {
        tv_dict_add_nr(di, c"preselect", 1);
    }
    if let Some(ref commit_chars) = match_.cp_commit_chars {
        tv_dict_add_str(di, c"commit_chars", Some(commit_chars.as_c_str()));
    }
    if match_.cp_user_data.v_type == VAR_UNKNOWN {
        tv_dict_add_str_len(di, c"user_data", Some(b""));
    } else {
        tv_dict_add_tv(di, b"user_data", &mut match_.cp_user_data);
    }
}

fn get_complete_info(what_list: Option<&TvList>, retdict: &mut TvDict) {
    let mut what_flag: i32;

    if let Some(what_list) = what_list {
        what_flag = 0;
        for item in crate::eval::typval::list_iter_mut(what_list.first_opt()) {
            let what = tv_get_string(&item.li_tv);
            if what.to_bytes() == b"mode" {
                what_flag |= CI_WHAT_MODE;
            } else if what.to_bytes() == b"pum_visible" {
                what_flag |= CI_WHAT_PUM_VISIBLE;
            } else if what.to_bytes() == b"items" {
                what_flag |= CI_WHAT_ITEMS;
            } else if what.to_bytes() == b"selected" {
                what_flag |= CI_WHAT_SELECTED;
            } else if what.to_bytes() == b"completed" {
                what_flag |= CI_WHAT_COMPLETED;
            } else if what.to_bytes() == b"preinserted_text" {
                what_flag |= CI_WHAT_PREINSERTED_TEXT;
            } else if what.to_bytes() == b"matches" {
                what_flag |= CI_WHAT_MATCHES;
            } else if what.to_bytes() == b"auto" {
                what_flag |= CI_WHAT_AUTO;
            }
        }
    } else {
        what_flag = CI_WHAT_ALL & !(CI_WHAT_MATCHES | CI_WHAT_COMPLETED | CI_WHAT_AUTO);
    }

    let mut ret = true;
    if (what_flag & CI_WHAT_MODE) != 0 {
        let mode = ins_compl_mode();
        ret = tv_dict_add_str_len(
            retdict,
            c"mode",
            (!mode.data.is_null()).then(|| mode.as_bytes()),
        );
    }

    if ret && (what_flag & CI_WHAT_PUM_VISIBLE) != 0 {
        ret = tv_dict_add_nr(retdict, c"pum_visible", Varnumber::from(pum_visible()));
    }

    if ret && (what_flag & CI_WHAT_AUTO) != 0 {
        ret = tv_dict_add_nr(
            retdict,
            c"auto",
            Varnumber::from(insexp().compl_autostarted),
        );
    }

    if ret && (what_flag & CI_WHAT_PREINSERTED_TEXT) != 0 {
        let line_slice = crate::cursor::get_cursor_line_slice();
        let len = insexp().compl_ins_end_col - curwin_mut().w_cursor.col;
        let col_u = (curwin_mut().w_cursor.col.max(0) as usize).min(line_slice.len());
        ret = tv_dict_add_str_len(
            retdict,
            c"preinserted_text",
            Some(if len > 0 {
                let end_u = (col_u + len as usize).min(line_slice.len());
                &line_slice[col_u..end_u]
            } else {
                b""
            }),
        );
    }

    if ret
        && (what_flag & (CI_WHAT_ITEMS | CI_WHAT_SELECTED | CI_WHAT_MATCHES | CI_WHAT_COMPLETED))
            != 0
    {
        let mut li = crate::globals::GraphPtr::<TvList>::NULL;
        let mut selected_idx: i32 = -1;
        let has_items = (what_flag & CI_WHAT_ITEMS) != 0;
        let has_matches = (what_flag & CI_WHAT_MATCHES) != 0;
        let has_completed = (what_flag & CI_WHAT_COMPLETED) != 0;
        if has_items || has_matches {
            li = crate::globals::GraphPtr::<TvList>::of(tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST));
            let key = if has_matches && !has_items {
                c"matches"
            } else {
                c"items"
            };
            ret = tv_dict_add_list(retdict, key, li.opt_mut());
        }
        if ret
            && (what_flag & CI_WHAT_SELECTED) != 0
            && let Some(ci) = insexp().compl_curr_idx
            && compl_at(ci).cp_number == -1
        {
            ins_compl_update_sequence_numbers();
        }
        let curr_num = compl_opt(insexp().compl_curr_idx).map(|c| c.cp_number);
        let list = &mut insexp().compl_list;
        if ret && !list.is_empty() {
            let mut list_idx: i32 = 0;
            for match_ in list {
                if !match_at_original_text(match_) {
                    if has_items || (has_matches && match_.cp_in_match_array) {
                        let di = tv_dict_alloc();
                        tv_list_append_dict(li.borrow_mut(), Some(di));
                        fill_complete_info_dict(&mut *di, match_, has_matches && has_items);
                    }
                    if curr_num == Some(match_.cp_number) {
                        selected_idx = list_idx;
                    }
                    if !has_matches || match_.cp_in_match_array {
                        list_idx += 1;
                    }
                }
            }
        }
        if ret && (what_flag & CI_WHAT_SELECTED) != 0 {
            ret = tv_dict_add_nr(retdict, c"selected", Varnumber::from(selected_idx));
            if let Some(wp) = win_float_find(K_WIN_INFO) {
                tv_dict_add_nr(retdict, c"preview_winid", Varnumber::from(wp.handle));
                tv_dict_add_nr(
                    retdict,
                    c"preview_bufnr",
                    Varnumber::from(wp.buf_mut().b_fnum),
                );
            }
        }
        if ret
            && selected_idx != -1
            && has_completed
            && let Some(curr) = compl_opt(insexp().compl_curr_idx)
        {
            let di = tv_dict_alloc();
            fill_complete_info_dict(&mut *di, curr, false);
            let _ = tv_dict_add_dict(retdict, c"completed", Some(di));
        }
    }
}

pub fn f_complete_info(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_dict_alloc_ret(rettv);

    let mut what_list = crate::globals::GraphPtr::<TvList>::NULL;

    if argvars[0].v_type != VAR_UNKNOWN {
        if argvars[0].v_type != VAR_LIST {
            emsg(gettext(E_LISTREQ));
            return;
        }
        what_list = crate::globals::GraphPtr::<TvList>::of(argvars[0].vval.v_list());
    }
    get_complete_info(what_list.opt(), rettv.vval.v_dict_ref());
}

#[inline]
fn thesaurus_func_complete(type_: i32) -> bool {
    type_ == CTRL_X_THESAURUS
        && (curbuf_mut().b_p_tsrfu.cb_type() != K_CALLBACK_NONE_CONST
            || crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_tsrfu)
                != K_CALLBACK_NONE_CONST)
}

fn may_advance_cpt_index(cpt: &CStr) -> bool {
    if insexp().cpt_sources_index == -1 {
        return false;
    }
    let bytes = cpt.to_bytes();
    bytes.iter().any(|&b| b != b',' && b != b' ')
}

fn process_next_cpt_value(
    st: &mut InsComplNextState,
    compl_type_arg: &mut i32,
    start_match_pos: &mut CursorPos,
    fuzzy_collect: bool,
    advance_cpt_idx: &mut bool,
) -> i32 {
    let mut compl_type: i32 = -1;
    let mut status = INS_COMPL_CPT_OK;
    let skip_source = insexp().compl_autocomplete && insexp().compl_from_nonkeyword;

    st.found_all = false;
    *advance_cpt_idx = false;

    while st.e_cpt_char() == b',' || st.e_cpt_char() == b' ' {
        st.e_cpt_idx += 1;
    }

    if st.e_cpt_char() == b'.'
        && !curbuf_mut().b_scanned
        && !skip_source
        && !insexp().compl_time_slice_expired
    {
        st.ins_buf = g_st().curbuf;
        st.first_match_pos = *start_match_pos;
        if ctrl_x_mode_normal() && !fuzzy_collect && dec(&mut st.first_match_pos) < 0 {
            st.first_match_pos.lnum = st.ins_buf.b_ml.ml_line_count;

            st.first_match_pos.col = ml_get_len(st.first_match_pos.lnum);
        }
        st.last_match_pos = st.first_match_pos;
        compl_type = 0;
        st.set_match_pos = true;
    } else if !skip_source
        && !insexp().compl_time_slice_expired
        && crate::strings::vim_haschr(c"buwU", i32::from(st.e_cpt_char()))
        && {
            st.ins_buf = crate::globals::BufPtr::new(ins_compl_next_buf(
                st.ins_buf.borrow_mut(),
                i32::from(st.e_cpt_char()),
            ));
            st.ins_buf != g_st().curbuf
        }
    {
        let ins_buf = st.ins_buf.borrow();
        if ins_buf.b_ml.ml_mfp.is_some() {
            insexp().compl_started = true;
            st.first_match_pos.col = 0;
            st.last_match_pos.col = 0;

            st.first_match_pos.lnum = ins_buf.b_ml.ml_line_count + 1;

            st.last_match_pos.lnum = 0;
            compl_type = 0;
        } else {
            st.found_all = true;
            if ins_buf.fname_opt().is_none() {
                *compl_type_arg = compl_type;
                return INS_COMPL_CPT_CONT;
            }
            compl_type = CTRL_X_DICTIONARY;

            st.dict = Some(ins_buf.fname_cstr().to_owned());

            st.dict_f = DICT_EXACT;
        }
        if !shortmess(K_SHM_COMPLETIONSCAN) && !insexp().compl_autocomplete {
            let name = if ins_buf.fname_opt().is_none() {
                buf_spname(ins_buf)
            } else if ins_buf.b_sfname.is_none() {
                ins_buf.fname_opt()
            } else {
                ins_buf.sfname_opt()
            };
            format_to_buf!(
                &mut g_st().iobuff,
                gettext(c"Scanning: %s"),
                name.unwrap_or(c"")
            );
            msg_progress(
                Some(g_st().iobuff_bytes_mut()),
                c"nvim.completion",
                c"running",
                -1,
                HLF_R,
                false,
                true,
                false,
            );
        }
    } else if st.e_cpt_char() == 0 {
        status = INS_COMPL_CPT_END;
    } else {
        if ctrl_x_mode_line_or_eval() {
            // compl_type = -1;
        } else if st.e_cpt_char() == b'F' || st.e_cpt_char() == b'o' {
            compl_type = CTRL_X_FUNCTION;
            st.func_cb = get_callback_if_cpt_func(st.e_cpt_cstr(), insexp().cpt_sources_index);
            if st.func_cb == CptFuncSource::None {
                compl_type = -1;
            }
        } else if !skip_source {
            if st.e_cpt_char() == b'k' || st.e_cpt_char() == b's' {
                if st.e_cpt_char() == b'k' {
                    compl_type = CTRL_X_DICTIONARY;
                } else {
                    compl_type = CTRL_X_THESAURUS;
                }
                st.e_cpt_idx += 1;
                if st.e_cpt_char() != b',' && st.e_cpt_char() != 0 {
                    st.dict = Some(st.e_cpt_cstr().to_owned());
                    st.dict_f = DICT_FIRST;
                }
            } else if st.e_cpt_char() == b'i' {
                compl_type = CTRL_X_PATH_PATTERNS;
            } else if st.e_cpt_char() == b'd' {
                compl_type = CTRL_X_PATH_DEFINES;
            } else if st.e_cpt_char() == b'f' {
                compl_type = CTRL_X_BUFNAMES;
            } else if st.e_cpt_char() == b']' || st.e_cpt_char() == b't' {
                compl_type = CTRL_X_TAGS;
                if !shortmess(K_SHM_COMPLETIONSCAN) && !insexp().compl_autocomplete {
                    format_to_buf!(&mut g_st().iobuff, c"%s", gettext(c"Scanning tags."));
                    msg_progress(
                        Some(g_st().iobuff_bytes_mut()),
                        c"nvim.completion",
                        c"running",
                        -1,
                        HLF_R,
                        false,
                        true,
                        false,
                    );
                }
            }
        }

        let cpt_cstr = st.e_cpt_cstr();
        let mut cpt_bytes = cpt_cstr.to_bytes();
        let old_len = cpt_bytes.len();
        copy_option_part(&mut cpt_bytes, g_st().iobuff_bytes_mut(), c",");
        let consumed = old_len - cpt_bytes.len();
        st.e_cpt_idx += consumed;
        *advance_cpt_idx = may_advance_cpt_index(st.e_cpt_cstr());

        st.found_all = true;
        if compl_type == -1 {
            status = INS_COMPL_CPT_CONT;
        }
    }

    *compl_type_arg = compl_type;
    status
}

fn get_next_include_file_completion(compl_type: i32) {
    find_pattern_in_path(
        compl_pattern_cstr().map_or(&[], CStr::to_bytes),
        insexp().compl_direction,
        false,
        false,
        if compl_type == CTRL_X_PATH_DEFINES && (insexp().compl_cont_status & CONT_SOL) == 0 {
            FIND_DEFINE
        } else {
            FIND_ANY
        },
        1,
        ACTION_EXPAND,
        1,
        MAXLNUM,
        false,
        insexp().compl_autocomplete,
    );
}

fn get_next_dict_tsr_completion(compl_type: i32, dict: Option<&CStr>, dict_f: i32) {
    if thesaurus_func_complete(compl_type) {
        expand_by_function(compl_type, compl_pattern_cstr(), None);
    } else {
        let dict_owned = if dict.is_some() {
            None
        } else if compl_type == CTRL_X_THESAURUS {
            if curbuf_mut().b_p_tsr.is_empty() {
                Some(opt_vars().p_tsr.to_owned())
            } else {
                Some(curbuf_mut().b_p_tsr.to_owned())
            }
        } else if curbuf_mut().b_p_dict.is_empty() {
            Some(opt_vars().p_dict.to_owned())
        } else {
            Some(curbuf_mut().b_p_dict.to_owned())
        };
        let dict_arg = dict.or(dict_owned.as_deref());
        ins_compl_dictionaries(
            dict_arg,
            compl_pattern_cstr(),
            if dict.is_some() { dict_f } else { 0 },
            compl_type == CTRL_X_THESAURUS,
        );
    }
}

fn get_next_tag_completion() {
    let save_p_ic = opt_vars().p_ic;
    let pat_cstr = compl_pattern_cstr().unwrap_or(c"");
    opt_vars().p_ic = ignorecase(pat_cstr);

    g_st().g_tag_at_cursor = true;
    let mut matches: Vec<Vec<u8>> = Vec::new();
    if find_tags(
        pat_cstr,
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
        curbuf_mut().ffname_opt(),
    ) && !matches.is_empty()
    {
        let mut add_r = Some(true);
        let mut dir = insexp().compl_direction;
        for m in &matches {
            if add_r == Some(false) {
                break;
            }
            add_r = ins_compl_add(
                m,
                -1,
                None,
                [const { None }; CPT_COUNT],
                None,
                dir,
                CP_FAST | if opt_vars().p_ic { CP_ICASE } else { 0 },
                false,
                None,
                FUZZY_SCORE_NONE,
                false,
                None,
            );
            if add_r == Some(true) {
                dir = FORWARD;
            }
        }
    }
    g_st().g_tag_at_cursor = false;
    opt_vars().p_ic = save_p_ic;
}

fn ins_compl_longest_insert(prefix: &[u8]) {
    let prefix = crate::memory::bytes_until_nul(prefix);
    ins_compl_delete(false);
    let skip = (get_compl_len().max(0) as usize).min(prefix.len());
    ins_compl_insert_bytes(&prefix[skip..], -1);
    insexp().compl_longest_end_col = curwin_mut().w_cursor.col;
    ins_redraw(false);
}

fn fuzzy_longest_match() {
    if insexp().compl_num_bests == 0 {
        return;
    }

    let nn_idx = compl_next_idx(0).and_then(compl_next_idx);
    let more_candidates = nn_idx.is_some_and(|i| i != 0);

    let mut compl_idx = if ctrl_x_mode_whole_line() {
        Some(0)
    } else {
        compl_next_idx(0)
    };
    if insexp().compl_num_bests == 1 {
        if !more_candidates && let Some(ci) = compl_idx {
            ins_compl_longest_insert(compl_at(ci).cp_str.as_bytes());
            insexp().compl_num_bests = 0;
        }
        insexp().compl_num_bests = 0;
        return;
    }

    if (insexp().compl_num_bests as usize) > usize::MAX / size_of::<*mut Compl>() {
        return;
    }
    let mut compl_best_matches = Vec::with_capacity(insexp().compl_num_bests as usize);

    let mut i: i32 = 0;
    while let Some(ci) = compl_idx
        && i < insexp().compl_num_bests
    {
        compl_best_matches.push(compl_at(ci));
        compl_idx = compl_next_idx(ci);
        i += 1;
    }

    let prefix = compl_best_matches[0].cp_str;
    let mut prefix_len = prefix.size as i32;

    for best in &compl_best_matches[1..] {
        let mut prefix_ptr = prefix.as_bytes();
        let mut match_ptr = best.cp_str.as_bytes();
        let mut j: i32 = 0;

        while j < prefix_len && !match_ptr.is_empty() && !prefix_ptr.is_empty() {
            let clen = utfc_ptr2len(prefix_ptr) as usize;
            if prefix_ptr.get(..clen) != match_ptr.get(..clen) {
                break;
            }

            let adv1 = crate::mbyte::utfc_ptr2len(prefix_ptr) as usize;
            prefix_ptr = &prefix_ptr[adv1.min(prefix_ptr.len())..];
            let adv2 = crate::mbyte::utfc_ptr2len(match_ptr) as usize;
            match_ptr = &match_ptr[adv2.min(match_ptr.len())..];
            j += 1;
        }

        if j > 0 {
            prefix_len = j;
        }
    }

    let lead_b = ins_compl_leader_slice();
    let leader_len = lead_b.len();
    let pref_b = prefix.as_bytes();

    if leader_len == 0 || &pref_b[..leader_len.min(pref_b.len())] == lead_b {
        let dup_slice = &pref_b[..(prefix_len as usize).min(pref_b.len())];
        ins_compl_longest_insert(dup_slice);
    }

    insexp().compl_num_bests = 0;
}

fn get_next_filename_completion() {
    let mut matches: Vec<std::ffi::CString> = Vec::new();
    let mut leader_slice = ins_compl_leader_slice();
    let leader_len = leader_slice.len();
    let mut in_fuzzy_collect = cot_fuzzy() && leader_len > 0;
    let need_collect_bests = in_fuzzy_collect && insexp().compl_get_longest;
    let mut max_score: i32 = 0;
    let mut dir = insexp().compl_direction;

    if in_fuzzy_collect {
        #[cfg(windows)]
        {
            if !insexp().compl_leader.data.is_null() {
                path_to_slash(insexp().compl_leader.as_bytes_with_nul_mut());
            } else if !insexp().compl_orig_text.data.is_null() {
                path_to_slash(insexp().compl_orig_text.as_bytes_with_nul_mut());
            }
            leader_slice = ins_compl_leader_slice();
        }
        if let Some(sep_idx) = leader_slice.iter().rposition(|&b| b == PATHSEP) {
            if sep_idx + 1 >= leader_slice.len() {
                in_fuzzy_collect = false;
            } else {
                let path_len = sep_idx + 1;
                let mut path_with_wildcard: Vec<u8> = Vec::with_capacity(path_len + 1);
                path_with_wildcard.extend_from_slice(&leader_slice[..path_len]);
                path_with_wildcard.push(b'*');
                set_compl_pattern_bytes(&path_with_wildcard);
                leader_slice = &leader_slice[sep_idx + 1..];
            }
        } else {
            insexp().compl_pattern = Some(c"*".to_owned());
        }
    }

    let mut flags = EW_FILE | EW_DIR | EW_ADDSLASH | EW_SILENT;
    if (get_cot_flags() & K_OPT_COT_FLAG_PATHOPT) != 0 {
        flags |= EW_PATH;
    }
    let pat_cstr = compl_pattern_cstr().unwrap_or(c"");
    if !expand_wildcards_vec(&[pat_cstr], &mut matches, flags) {
        return;
    }

    tilde_replace(
        compl_pattern_cstr().map_or(&[], CStr::to_bytes),
        &mut matches,
    );
    #[cfg(windows)]
    {
        if (curbuf_mut().b_p_csl.is_empty() && !opt_vars().p_ssl)
            || curbuf_mut().b_p_csl.first_byte() == b'b'
        {
            for slot in &mut matches {
                if slot.as_bytes().contains(&b'/') {
                    let bytes: Vec<u8> = slot
                        .as_bytes()
                        .iter()
                        .map(|&b| if b == b'/' { b'\\' } else { b })
                        .collect();
                    *slot = crate::memory::bytes_to_cstring(&bytes);
                }
            }
        }
    }

    if in_fuzzy_collect {
        let mut fuzzy_indices: Vec<i32> = Vec::with_capacity(10);
        let mut compl_fuzzy_scores = vec![0_i32; matches.len()];

        for (i, m) in matches.iter().enumerate() {
            let score = fuzzy_match_str(Some(m.to_bytes()), Some(leader_slice));
            if score != FUZZY_SCORE_NONE {
                fuzzy_indices.push(i as i32);
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
                let match_ = &matches[idx];
                let current_score = compl_fuzzy_scores[idx];
                if ins_compl_add(
                    match_.as_bytes(),
                    match_.as_bytes().len() as i32,
                    None,
                    [const { None }; CPT_COUNT],
                    None,
                    dir,
                    CP_FAST
                        | if opt_vars().p_fic || opt_vars().p_wic {
                            CP_ICASE
                        } else {
                            0
                        },
                    false,
                    None,
                    current_score,
                    false,
                    None,
                ) == Some(true)
                {
                    dir = FORWARD;
                }

                if need_collect_bests && (i == 0 || current_score == max_score) {
                    insexp().compl_num_bests += 1;
                    max_score = current_score;
                }
            }
        }

        if insexp().compl_num_bests > 0 && insexp().compl_get_longest {
            fuzzy_longest_match();
        }
        return;
    }

    if !matches.is_empty() {
        ins_compl_add_matches(
            &mut matches,
            i32::from(opt_vars().p_fic || opt_vars().p_wic),
        );
    }
}

fn get_next_cmdline_completion() {
    let mut matches: Vec<std::ffi::CString> = Vec::new();
    if expand_cmdline(
        &mut insexp().compl_xp,
        compl_pattern_cstr().unwrap_or(c""),
        compl_pattern_len() as i32,
        &mut matches,
    ) == EXPAND_OK
    {
        let mut add_r = Some(true);
        let mut dir = insexp().compl_direction;

        for (i, match_i) in matches.iter().enumerate() {
            if add_r == Some(false) {
                break;
            }
            let mut cptext: [Option<std::ffi::CString>; CPT_COUNT] = [const { None }; CPT_COUNT];

            let cxp = &insexp().compl_xp;
            if let Some(Some(abbr)) = cxp.xp_files_abbr.as_slice().get(i) {
                cptext[CPT_ABBR] = Some(abbr.clone());
            }
            if let Some(Some(kind)) = cxp.xp_files_kind.as_slice().get(i) {
                cptext[CPT_KIND] = Some(kind.clone());
            }
            if let Some(Some(menu)) = cxp.xp_files_menu.as_slice().get(i) {
                cptext[CPT_MENU] = Some(menu.clone());
            }
            if let Some(Some(info)) = cxp.xp_files_info.as_slice().get(i) {
                cptext[CPT_INFO] = Some(info.clone());
            }

            add_r = ins_compl_add(
                match_i.as_bytes(),
                match_i.as_bytes().len() as i32,
                None,
                cptext,
                None,
                dir,
                CP_FAST,
                false,
                None,
                FUZZY_SCORE_NONE,
                false,
                None,
            );
            if add_r == Some(true) {
                dir = FORWARD;
            }
        }
        free_xp_files_extra(&mut insexp().compl_xp);
    }
}

fn get_next_spell_completion(lnum: Linenr) {
    let mut matches: Vec<std::ffi::CString> = Vec::new();
    expand_spelling(lnum, compl_pattern_cstr().unwrap_or(c""), &mut matches);
    if !matches.is_empty() {
        ins_compl_add_matches(&mut matches, i32::from(opt_vars().p_ic));
    }
}

fn ins_compl_get_next_word_or_line(
    ins_buf: &mut FileBuf,
    cur_match_pos: &mut CursorPos,
    match_len: &mut i32,
    cont_s_ipos: &mut bool,
) -> Option<&'static [u8]> {
    *match_len = 0;
    let line_slice = crate::memline::ml_get_buf_slice(ins_buf, cur_match_pos.lnum);
    let col_u = (cur_match_pos.col.max(0) as usize).min(line_slice.len());
    let mut res_slice = &line_slice[col_u..];
    let mut len = res_slice.len() as i32;
    if ctrl_x_mode_line_or_eval() {
        if compl_status_adding() {
            if cur_match_pos.lnum >= ins_buf.b_ml.ml_line_count {
                return None;
            }
            res_slice = crate::memline::ml_get_buf_slice(ins_buf, cur_match_pos.lnum + 1);
            if !opt_vars().p_paste {
                res_slice = crate::charset::skipwhite_slice(res_slice);
            }
            len = res_slice.len() as i32;
        }
    } else {
        let mut rem = res_slice;

        if compl_status_adding() && insexp().compl_length <= len {
            rem = &rem[insexp().compl_length.max(0) as usize..];
            if vim_iswordp(rem) {
                return None;
            }
            rem = find_word_start(rem);
        }
        let end_rem = find_word_end(rem);
        len = (res_slice.len() - end_rem.len()) as i32;

        if compl_status_adding() && len == insexp().compl_length {
            if cur_match_pos.lnum < ins_buf.b_ml.ml_line_count {
                let iobuff = g_st().borrow_mut().iobuff_bytes_mut();
                {
                    let len_usize = len as usize;
                    let n = len_usize.min(res_slice.len());
                    iobuff[..n].copy_from_slice(&res_slice[..n]);
                    iobuff[n..len_usize].fill(0);
                }
                let next_line = crate::memline::ml_get_buf_slice(ins_buf, cur_match_pos.lnum + 1);
                let next_s = crate::charset::skipwhite_slice(next_line);
                let wstart = find_word_start(next_s);
                let wend = find_word_end(wstart);
                let next_wlen = next_s.len() - wend.len();
                if next_wlen > 0 {
                    if next_s.first() != Some(&b')') && iobuff[(len - 1) as usize] != TAB as u8 {
                        if iobuff[(len - 1) as usize] != b' ' {
                            iobuff[len as usize] = b' ';
                            len += 1;
                        }
                        if opt_vars().p_js {
                            let prev = iobuff[(len - 2) as usize];
                            if prev == b'.' || prev == b'?' || prev == b'!' {
                                iobuff[len as usize] = b' ';
                                len += 1;
                            }
                        }
                    }
                    let copy_n = next_wlen.min(IOSIZE - len as usize - 1);
                    crate::memory::copy_bytes_to_c_buf(
                        &mut iobuff[len as usize..],
                        &next_s[..copy_n],
                    );
                    len += copy_n as i32;
                    *cont_s_ipos = true;
                }
                iobuff[len as usize] = 0;
                res_slice = &iobuff[..len as usize];
            }
            if len == insexp().compl_length {
                return None;
            }
        }
    }

    *match_len = len;
    Some(&res_slice[..(len.max(0) as usize).min(res_slice.len())])
}

fn get_next_default_completion(st: &mut InsComplNextState, start_pos: &mut CursorPos) -> bool {
    let mut matched_slice: Option<&'static [u8]> = None;
    let mut len: i32 = 0;
    let in_fuzzy_collect = !compl_status_adding() && cot_fuzzy() && insexp().compl_length > 0;
    let leader_slice = ins_compl_leader_slice();
    let mut score = FUZZY_SCORE_NONE;
    let in_curbuf = st.ins_buf == g_st().curbuf;

    let save_p_scs = opt_vars().p_scs;
    debug_assert!(!st.ins_buf.is_null());
    if st.ins_buf.b_p_inf {
        opt_vars().p_scs = false;
    }

    let save_p_ws = opt_vars().p_ws;
    if !in_curbuf {
        opt_vars().p_ws = false;
    } else if st.e_cpt_char() == b'.' {
        opt_vars().p_ws = true;
    }
    let mut looped_around = false;
    let mut found_new_match: bool;
    loop {
        let mut cont_s_ipos = false;
        let ins_buf_ref = st.ins_buf.borrow_mut();

        g_st().msg_silent += 1;

        if in_fuzzy_collect {
            let mut matched_line: &'static [u8] = &[];
            found_new_match = search_for_fuzzy_match(
                ins_buf_ref,
                &mut *st.cur_match_pos_ptr(),
                Some(leader_slice),
                insexp().compl_direction,
                start_pos,
                &mut len,
                &mut matched_line,
                &mut score,
            );
            matched_slice = Some(matched_line);
        } else if ctrl_x_mode_whole_line()
            || ctrl_x_mode_eval()
            || (insexp().compl_cont_status & CONT_SOL) != 0
        {
            found_new_match = search_for_exact_line(
                ins_buf_ref,
                &mut *st.cur_match_pos_ptr(),
                insexp().compl_direction,
                compl_pattern_cstr().unwrap_or(c""),
            );
        } else {
            found_new_match = searchit(
                None,
                ins_buf_ref,
                st.cur_match_pos_ptr(),
                None,
                insexp().compl_direction,
                compl_pattern_cstr().unwrap_or(c""),
                compl_pattern_len(),
                1,
                SEARCH_KEEP + SEARCH_NFMSG,
                RE_LAST,
                opt_vars().p_magic,
                None,
            ) != 0;
        }
        g_st().msg_silent -= 1;
        let cur_pos = st.cur_match_pos();
        if !insexp().compl_started || st.set_match_pos {
            insexp().compl_started = true;
            st.first_match_pos = cur_pos;
            st.last_match_pos = cur_pos;
            st.set_match_pos = false;
        } else if st.first_match_pos.lnum == st.last_match_pos.lnum
            && st.first_match_pos.col == st.last_match_pos.col
        {
            found_new_match = false;
        } else if (compl_dir_forward()
            && (st.prev_match_pos.lnum > cur_pos.lnum
                || (st.prev_match_pos.lnum == cur_pos.lnum
                    && st.prev_match_pos.col >= cur_pos.col)))
            || (!compl_dir_forward()
                && (st.prev_match_pos.lnum < cur_pos.lnum
                    || (st.prev_match_pos.lnum == cur_pos.lnum
                        && st.prev_match_pos.col <= cur_pos.col)))
        {
            if looped_around {
                found_new_match = false;
            } else {
                looped_around = true;
            }
        }
        st.prev_match_pos = cur_pos;
        if !found_new_match {
            break;
        }

        if compl_status_adding()
            && in_curbuf
            && start_pos.lnum == cur_pos.lnum
            && start_pos.col == cur_pos.col
        {
            continue;
        }

        if !in_fuzzy_collect {
            matched_slice = ins_compl_get_next_word_or_line(
                st.ins_buf.borrow_mut(),
                &mut *st.cur_match_pos_ptr(),
                &mut len,
                &mut cont_s_ipos,
            );
        }
        let Some(m_slice) = matched_slice else {
            continue;
        };
        if ins_compl_has_preinsert()
            && crate::memory::bytes_until_nul(m_slice) == ins_compl_leader_slice()
        {
            continue;
        }

        if is_nearest_active() && in_curbuf {
            score = (st.cur_match_pos().lnum - curwin_mut().w_cursor.lnum).abs();
        }

        if ins_compl_add_infercase(
            m_slice,
            len,
            opt_vars().p_ic,
            if in_curbuf {
                None
            } else {
                st.ins_buf.sfname_opt()
            },
            0,
            cont_s_ipos,
            score,
        )
        .is_some()
        {
            if in_fuzzy_collect
                && let Some(next_i) = compl_next_idx(0)
                && score == compl_at(next_i).cp_score
            {
                insexp().compl_num_bests += 1;
            }
            found_new_match = true;
            break;
        }
    }
    opt_vars().p_scs = save_p_scs;
    opt_vars().p_ws = save_p_ws;

    found_new_match
}

fn get_register_completion() {
    let mut dir = insexp().compl_direction;
    let adding_mode = compl_status_adding();

    for i in 0..(NUM_REGISTERS as i32) {
        let regname = get_register_name(i);
        if !valid_yank_reg(regname, false) || regname == b'_' as i32 {
            continue;
        }

        let reg = copy_register(regname);

        if !reg.y_allocated || reg.y_array.is_empty() {
            continue;
        }

        for j in 0..reg.y_array.len() {
            let str_val = reg.y_array[j];
            if str_val.data.is_null() {
                continue;
            }
            let str_b = str_val.cstr_bytes();

            if adding_mode {
                let str_len = str_b.len() as i32;
                if str_len == 0 {
                    continue;
                }

                let orig_bytes = insexp().compl_orig_text.as_bytes();
                let str_prefix = &str_b[..orig_bytes.len().min(str_b.len())];
                let matches = insexp().compl_orig_text.data.is_null()
                    || if opt_vars().p_ic {
                        str_prefix.eq_ignore_ascii_case(orig_bytes)
                    } else {
                        str_prefix == orig_bytes
                    };
                if matches
                    && ins_compl_add_infercase(
                        str_b,
                        str_len,
                        opt_vars().p_ic,
                        None,
                        dir,
                        false,
                        FUZZY_SCORE_NONE,
                    ) == Some(true)
                {
                    dir = FORWARD;
                }
            } else {
                let mut rem = str_b;

                while !rem.is_empty() {
                    let old_len = rem.len();
                    rem = find_word_start(rem);
                    if rem.is_empty() {
                        break;
                    }

                    let word_end_rem = find_word_end(rem);
                    let mut len = rem.len() - word_end_rem.len();
                    if len == 0 {
                        len = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
                    }

                    let orig_bytes = insexp().compl_orig_text.as_bytes();
                    let p_prefix = &rem[..orig_bytes.len().min(rem.len())];
                    let matches = insexp().compl_orig_text.data.is_null()
                        || if opt_vars().p_ic {
                            p_prefix.eq_ignore_ascii_case(orig_bytes)
                        } else {
                            p_prefix == orig_bytes
                        };
                    if len > 0
                        && matches
                        && ins_compl_add_infercase(
                            &rem[..len],
                            len as i32,
                            opt_vars().p_ic,
                            None,
                            dir,
                            false,
                            FUZZY_SCORE_NONE,
                        ) == Some(true)
                    {
                        dir = FORWARD;
                    }

                    rem = &rem[len..];
                    if rem.len() >= old_len {
                        let step = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
                        rem = &rem[step..];
                    }
                }
            }
        }
    }
}

fn get_callback_if_cpt_func(p: &CStr, idx: i32) -> CptFuncSource {
    let p_bytes = p.to_bytes();
    if p_bytes.first() == Some(&b'o') {
        return CptFuncSource::Ofu;
    }

    if p_bytes.first() == Some(&b'F') {
        let next = p_bytes.get(1).copied().unwrap_or(0);
        if next != b',' && next != 0 {
            let cbs = &curbuf_mut().b_p_cpt_cb;
            if idx < 0 || (idx as usize) >= cbs.len() {
                return CptFuncSource::None;
            }
            return if cbs[idx as usize].cb_type() != K_CALLBACK_NONE_CONST {
                CptFuncSource::CptCb(idx as usize)
            } else {
                CptFuncSource::None
            };
        } else {
            return CptFuncSource::Cfu;
        }
    }

    CptFuncSource::None
}

fn get_next_completion_match(type_: i32, st: &mut InsComplNextState, ini: &mut CursorPos) -> bool {
    let mut found_new_match = false;

    match type_ {
        -1 => {}
        CTRL_X_PATH_PATTERNS | CTRL_X_PATH_DEFINES => {
            get_next_include_file_completion(type_);
        }
        CTRL_X_DICTIONARY | CTRL_X_THESAURUS => {
            let dict_cstr = st.dict.take();
            get_next_dict_tsr_completion(type_, dict_cstr.as_deref(), st.dict_f);
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
                if let Some(cb) = st.func_cb.as_mut() {
                    get_cpt_func_completion_matches(cb);
                }
            } else {
                expand_by_function(type_, compl_pattern_cstr(), None);
            }
        }
        CTRL_X_OMNI => {
            expand_by_function(type_, compl_pattern_cstr(), None);
        }
        CTRL_X_SPELL => {
            get_next_spell_completion(st.first_match_pos.lnum);
        }
        CTRL_X_BUFNAMES => {
            get_next_bufname_token();
        }
        CTRL_X_REGISTER => {
            get_register_completion();
        }
        _ => {
            found_new_match = get_next_default_completion(st, ini);
            if !found_new_match && st.ins_buf == g_st().curbuf {
                st.found_all = true;
            }
        }
    }

    if type_ != 0 && insexp().compl_curr_idx != insexp().compl_old_idx {
        found_new_match = true;
    }

    found_new_match
}

fn get_next_bufname_token() {
    for b in for_all_buffers_mut() {
        if b.b_p_bl && b.b_sfname.is_some() {
            let tail = path_tail(b.sfname_cstr());
            let orig_bytes = insexp().compl_orig_text.as_bytes();
            let tail_b = tail.to_bytes();
            if &tail_b[..orig_bytes.len().min(tail_b.len())] == orig_bytes {
                ins_compl_add(
                    tail_b,
                    tail_b.len() as i32,
                    None,
                    [const { None }; CPT_COUNT],
                    None,
                    0,
                    if opt_vars().p_ic { CP_ICASE } else { 0 },
                    false,
                    None,
                    FUZZY_SCORE_NONE,
                    false,
                    None,
                );
            }
        }
    }
}

fn strip_caret_numbers_in_place(buf: &mut [u8]) {
    let mut read = 0usize;
    let mut write = 0usize;

    while read < buf.len() && buf[read] != 0 {
        if buf[read] == b'^' {
            let mut p = read + 1;
            while p < buf.len() && buf[p].is_ascii_digit() {
                p += 1;
            }
            let next_b = buf.get(p).copied().unwrap_or(0);
            if (next_b == b',' || next_b == 0) && p != read + 1 {
                read = p;
                continue;
            }
        }
        buf[write] = buf[read];
        write += 1;
        read += 1;
    }
    if write < buf.len() {
        buf[write] = 0;
    }
}

fn prepare_cpt_compl_funcs() {
    let mut cpt_vec: Vec<u8> = curbuf_mut().b_p_cpt.to_owned().into_bytes_with_nul();
    strip_caret_numbers_in_place(&mut cpt_vec);
    let mut p_cstr = crate::memory::bytes_as_cstr(&cpt_vec);

    let mut idx: i32 = 0;
    while !p_cstr.to_bytes().is_empty() {
        while matches!(p_cstr.to_bytes().first(), Some(&b',' | &b' ')) {
            p_cstr = crate::memory::CStrSubslice::advance(p_cstr, 1);
        }
        if p_cstr.to_bytes().is_empty() {
            break;
        }

        if let Some(cb) = get_callback_if_cpt_func(p_cstr, idx).as_mut() {
            let mut startcol: i32 = 0;
            if !get_userdefined_compl_info(curwin_mut().w_cursor.col, Some(cb), Some(&mut startcol))
            {
                if startcol == -3 {
                    insexp().cpt_sources_array[idx as usize].cs_refresh_always = false;
                } else {
                    startcol = -2;
                }
            } else if startcol < 0 || startcol > curwin_mut().w_cursor.col {
                startcol = curwin_mut().w_cursor.col;
            }
            insexp().cpt_sources_array[idx as usize].cs_startcol = startcol;
        } else {
            insexp().cpt_sources_array[idx as usize].cs_startcol = -3;
        }

        let mut p_bytes = p_cstr.to_bytes();
        let old_len = p_bytes.len();
        let _ = copy_option_part(&mut p_bytes, g_st().iobuff_bytes_mut(), c",");
        p_cstr = crate::memory::CStrSubslice::advance(p_cstr, old_len - p_bytes.len());
        idx += 1;
    }
}

fn compl_source_start_timer(source_idx: i32) {
    if insexp().compl_autocomplete || opt_vars().p_cto > 0 {
        insexp().cpt_sources_array[source_idx as usize].compl_start_tv = os_hrtime();
        insexp().compl_time_slice_expired = false;
    }
}

fn advance_cpt_sources_index_safe() -> bool {
    let cpt_sources = &insexp().cpt_sources_array;
    if insexp().cpt_sources_index >= 0
        && (insexp().cpt_sources_index as usize) + 1 < cpt_sources.len()
    {
        insexp().cpt_sources_index += 1;
        return true;
    }
    semsg!(
        gettext(E_LIST_INDEX_OUT_OF_RANGE_NR),
        i64::from(insexp().cpt_sources_index),
    );
    false
}

fn ins_compl_get_exp(ini: &mut CursorPos) -> i32 {
    let st = &mut insexp().next_state;
    let mut found_new_match: bool;
    let mut type_ = insexp().ctrl_x_mode;
    let mut may_advance_cpt_idx = false;
    let mut start_pos = *ini;

    debug_assert!(!g_st().curbuf.is_null());

    if !insexp().compl_started {
        for buf in for_all_buffers_mut() {
            buf.b_scanned = false;
        }
        if !insexp().next_state_cleared {
            *st = InsComplNextState::new();
            insexp().next_state_cleared = true;
        }
        st.found_all = false;
        st.ins_buf = g_st().curbuf;
        let mut e_cpt_copy: Vec<u8> = if (insexp().compl_cont_status & CONT_LOCAL) != 0 {
            c".".to_owned().into_bytes_with_nul()
        } else {
            curbuf_mut().b_p_cpt.to_owned().into_bytes_with_nul()
        };
        strip_caret_numbers_in_place(&mut e_cpt_copy);
        st.e_cpt_copy = e_cpt_copy;
        st.e_cpt_idx = 0;

        if insexp().compl_autocomplete && is_nearest_active() {
            start_pos.lnum = (start_pos.lnum - LOOKBACK_LINE_COUNT).max(1);
            start_pos.col = 0;
        }
        st.first_match_pos = start_pos;
        st.last_match_pos = start_pos;
    } else if st.ins_buf != g_st().curbuf && !buf_valid(st.ins_buf.opt()) {
        st.ins_buf = g_st().curbuf;
    }
    debug_assert!(!st.ins_buf.is_null());

    insexp().compl_old_idx = insexp().compl_curr_idx;
    st.cur_match_forward = compl_dir_forward();

    let cpt_sources = &insexp().cpt_sources_array;
    let normal_mode_strict = ctrl_x_mode_normal()
        && !ctrl_x_mode_line_or_eval()
        && (insexp().compl_cont_status & CONT_LOCAL) == 0
        && !cpt_sources.is_empty();
    if normal_mode_strict {
        insexp().cpt_sources_index = 0;
        if insexp().compl_autocomplete || opt_vars().p_cto > 0 {
            compl_source_start_timer(0);
            insexp().compl_time_slice_expired = false;
            insexp().compl_timeout_ms = if insexp().compl_autocomplete {
                (COMPL_INITIAL_TIMEOUT_MS as OptInt).max(opt_vars().p_act) as u64
            } else {
                opt_vars().p_cto as u64
            };
        }
    }

    loop {
        found_new_match = false;
        st.set_match_pos = false;

        if (ctrl_x_mode_normal() || ctrl_x_mode_line_or_eval())
            && (!insexp().compl_started || st.found_all)
        {
            let status = process_next_cpt_value(
                st,
                &mut type_,
                &mut start_pos,
                cot_fuzzy(),
                &mut may_advance_cpt_idx,
            );
            if status == INS_COMPL_CPT_END {
                break;
            }
            if status == INS_COMPL_CPT_CONT {
                if may_advance_cpt_idx {
                    if !advance_cpt_sources_index_safe() {
                        break;
                    }
                    compl_source_start_timer(insexp().cpt_sources_index);
                }
                continue;
            }
        }

        let mut compl_timeout_save: u64 = 0;
        if normal_mode_strict
            && type_ == CTRL_X_FUNCTION
            && (insexp().compl_autocomplete || opt_vars().p_cto > 0)
        {
            compl_timeout_save = insexp().compl_timeout_ms;
            insexp().compl_timeout_ms = if insexp().compl_from_nonkeyword {
                COMPL_FUNC_TIMEOUT_NON_KW_MS
            } else {
                COMPL_FUNC_TIMEOUT_MS
            };
        }

        found_new_match = get_next_completion_match(type_, st, &mut start_pos);

        if compl_pattern_cstr().is_none() {
            break;
        }

        if may_advance_cpt_idx {
            if !advance_cpt_sources_index_safe() {
                break;
            }
            compl_source_start_timer(insexp().cpt_sources_index);
        }

        if (ctrl_x_mode_not_default() && !ctrl_x_mode_line_or_eval()) || found_new_match {
            if g_st().got_int {
                break;
            }
            if type_ != -1 {
                ins_compl_check_keys(0, false);
            }

            if (ctrl_x_mode_not_default() && !ctrl_x_mode_line_or_eval())
                || insexp().compl_interrupted
            {
                break;
            }
            insexp().compl_started = !insexp().compl_time_slice_expired;
        } else {
            if buf_valid(st.ins_buf.opt()) && (type_ == 0 || type_ == CTRL_X_PATH_PATTERNS) {
                st.ins_buf.b_scanned = true;
            }

            insexp().compl_started = false;
        }

        if normal_mode_strict
            && type_ == CTRL_X_FUNCTION
            && (insexp().compl_autocomplete || opt_vars().p_cto > 0)
        {
            insexp().compl_timeout_ms = compl_timeout_save;
        }

        if !compl_dir_forward() {
            while let Some(curr_i) = insexp().compl_curr_idx
                && let Some(prev_i) = compl_prev_idx(curr_i)
                && !match_at_original_text(&*compl_at(prev_i))
            {
                insexp().compl_curr_idx = Some(prev_i);
            }
        }
    }
    insexp().cpt_sources_index = -1;
    insexp().compl_started = true;

    let st_ptr = &st;
    if (ctrl_x_mode_normal() || ctrl_x_mode_line_or_eval()) && st_ptr.e_cpt_char() == 0 {
        found_new_match = false;
    }

    let mut match_count: i32 = -1;
    if !found_new_match || (ctrl_x_mode_not_default() && !ctrl_x_mode_line_or_eval()) {
        match_count = ins_compl_make_cyclic();
    }

    if cot_fuzzy() && insexp().compl_get_longest && insexp().compl_num_bests > 0 {
        fuzzy_longest_match();
    }

    if let Some(old_i) = insexp().compl_old_idx {
        let next_curr = if compl_dir_forward() {
            compl_next_idx(old_i)
        } else {
            compl_prev_idx(old_i)
        };
        insexp().compl_curr_idx = Some(next_curr.unwrap_or(old_i));
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

fn ins_compl_update_shown_match() {
    let _ = get_leader_for_startcol(None, true);
    let Some(mut shown_idx) = insexp().compl_shown_idx else {
        return;
    };
    let mut leader = get_leader_for_startcol(Some(&*compl_at(shown_idx)), true);

    while !ins_compl_equal(&mut *compl_at(shown_idx), leader.as_bytes())
        && let Some(next_i) = compl_next_idx(shown_idx)
        && next_i != 0
    {
        shown_idx = next_i;
        insexp().compl_shown_idx = Some(shown_idx);
        leader = get_leader_for_startcol(Some(&*compl_at(shown_idx)), true);
    }

    if compl_shows_dir_backward()
        && !ins_compl_equal(&mut *compl_at(shown_idx), leader.as_bytes())
        && compl_next_idx(shown_idx).is_none_or(|ni| ni == 0)
    {
        while !ins_compl_equal(&mut *compl_at(shown_idx), leader.as_bytes())
            && let Some(prev_i) = compl_prev_idx(shown_idx)
            && prev_i != 0
        {
            shown_idx = prev_i;
            insexp().compl_shown_idx = Some(shown_idx);
            leader = get_leader_for_startcol(Some(&*compl_at(shown_idx)), true);
        }
    }
}

pub fn ins_compl_delete(new_leader: bool) {
    let mut orig_col: i32 = 0;
    if new_leader {
        let orig_slice = insexp().compl_orig_text.cstr_bytes();
        let mut orig_off = 0usize;
        let mut leader = ins_compl_leader_slice();
        while orig_off < orig_slice.len()
            && !leader.is_empty()
            && utf_ptr2char(&orig_slice[orig_off..]) == utf_ptr2char(leader)
        {
            let step = utf_ptr2len(leader) as usize;
            leader = &leader[step.max(1).min(leader.len())..];
            orig_off +=
                (utf_ptr2len(&orig_slice[orig_off..]) as usize).min(orig_slice.len() - orig_off);
        }
        orig_col = orig_off as i32;
    }

    let mut col = insexp().compl_col
        + if compl_status_adding() {
            insexp().compl_length
        } else {
            orig_col
        };

    if ins_compl_preinsert_effect() {
        col += ins_compl_leader_len() as i32;
        curwin_mut().w_cursor.col = insexp().compl_ins_end_col;
    }

    let mut remaining_vec: Option<Vec<u8>> = None;
    if curwin_mut().w_cursor.lnum > insexp().compl_lnum {
        if curwin_mut().w_cursor.col < get_cursor_line_len() {
            let rem = get_cursor_pos_slice();
            let mut v = Vec::with_capacity(rem.len() + 1);
            v.extend_from_slice(rem);
            v.push(0);
            remaining_vec = Some(v);
        }

        while curwin_mut().w_cursor.lnum > insexp().compl_lnum {
            if !ml_delete(curwin_mut().w_cursor.lnum) {
                return;
            }
            deleted_lines_mark(curwin_mut().w_cursor.lnum, 1);
            curwin_mut().w_cursor.lnum -= 1;
        }
        curwin_mut().w_cursor.col = get_cursor_line_len();
    }

    if curwin_mut().w_cursor.col > col {
        if !stop_arrow() {
            return;
        }
        backspace_until_column(col);
        insexp().compl_ins_end_col = curwin_mut().w_cursor.col;
    }

    if let Some(ref rem) = remaining_vec {
        orig_col = curwin_mut().w_cursor.col;
        ins_str(rem, rem.len() - 1);
        curwin_mut().w_cursor.col = orig_col;
    }

    changed_cline_bef_curs(curwin_mut());
    set_vim_var_dict(VV_COMPLETED_ITEM, Some(tv_dict_alloc_lock(VAR_FIXED)));
}

fn ins_compl_expand_multiple(str_slice: &[u8]) {
    let bytes = crate::memory::bytes_until_nul(str_slice);
    let mut start = 0usize;
    let mut curr = 0usize;
    let base_indent = get_indent();
    while curr < bytes.len() {
        if bytes[curr] == b'\n' {
            if curr > start {
                let n = curr - start;
                ins_char_bytes(&bytes[start..curr], n);
            }
            open_line(
                FORWARD,
                OPENLINE_KEEPTRAIL | OPENLINE_FORCE_INDENT,
                base_indent,
                None,
            );
            start = curr + 1;
        }
        curr += 1;
    }

    if curr > start {
        let n = curr - start;
        ins_char_bytes(&bytes[start..curr], n);
    }

    insexp().compl_ins_end_col = curwin_mut().w_cursor.col;
}

fn find_common_prefix(prefix_len: &mut usize, curbuf_only: bool) -> Option<&'static [u8]> {
    let cpt_sources = &insexp().cpt_sources_array;
    let is_cpt_completion = !cpt_sources.is_empty();

    if !is_cpt_completion {
        return None;
    }

    let mut match_count = vec![0_i32; cpt_sources.len()];

    let _ = get_leader_for_startcol(None, true);

    let list = &mut insexp().compl_list;
    let mut first = crate::globals::GraphPtr::<u8>::NULL.as_ptr();
    let mut len: i32 = -1;
    for compl in list {
        let leader = get_leader_for_startcol(Some(&*compl), true);

        if ctrl_x_mode_normal()
            && !opt_vars().p_inf
            && !insexp().compl_orig_text.data.is_null()
            && !ignorecase(insexp().compl_orig_text.as_cstr())
        {
            compl.cp_flags &= !CP_ICASE;
        }

        if !match_at_original_text(compl)
            && (leader.data.is_null() || ins_compl_equal_sc(compl, leader.as_bytes()))
        {
            let mut match_limit_exceeded = false;
            let cur_source = compl.cp_cpt_source_idx;

            if cur_source != -1 {
                match_count[cur_source as usize] += 1;
                let max_matches = insexp().cpt_sources_array[cur_source as usize].cs_max_matches;
                if max_matches > 0 && match_count[cur_source as usize] > max_matches {
                    match_limit_exceeded = true;
                }
            }

            if !match_limit_exceeded
                && (!curbuf_only
                    || (cur_source != -1
                        && insexp().cpt_sources_array[cur_source as usize].cs_flag == b'.'))
            {
                if first.is_null() {
                    let lead_b = ins_compl_leader_slice();
                    let lead_len = lead_b.len();
                    let cp_b = compl.cp_str.as_bytes();
                    if lead_b == &cp_b[..lead_len.min(cp_b.len())] {
                        first = compl.cp_str.data.cast();
                        len = compl.cp_str.size as i32;
                    }
                } else if !first.is_null() {
                    let s1_bytes = crate::globals::GraphPtr::raw_slice(first, len.max(0) as usize);
                    let s2_bytes = compl.cp_str.as_bytes();
                    let mut j: usize = 0;

                    while j < s1_bytes.len() && j < s2_bytes.len() {
                        let l1 = utf_byte2len(i32::from(s1_bytes[j])) as usize;
                        let l2 = utf_byte2len(i32::from(s2_bytes[j])) as usize;
                        if l1 != l2 || s1_bytes.get(j..j + l1) != s2_bytes.get(j..j + l1) {
                            break;
                        }
                        j += l1;
                    }
                    len = j as i32;

                    if len == 0 {
                        break;
                    }
                }
            }
        }
    }

    if len > ins_compl_leader_len() as i32 {
        debug_assert!(!first.is_null());
        let first_cstr = unsafe { CStr::from_ptr(first.cast()) };
        if len == first_cstr.to_bytes().len() as i32 {
            let line = get_cursor_line_slice();
            let col_u = (curwin_mut().w_cursor.col.max(0) as usize).min(line.len());
            let p_slice = &line[col_u..];
            if !p_slice.is_empty() && !ascii_iswhite_or_nul(i32::from(p_slice[0])) {
                let end_tail = find_word_end(p_slice);
                let text_len = (p_slice.len() - end_tail.len()) as i32;
                if text_len > 0
                    && text_len < len - ins_compl_leader_len() as i32
                    && crate::globals::GraphPtr::raw_slice(
                        first.wrapping_add((len - text_len) as usize),
                        text_len as usize,
                    ) == &p_slice[..text_len as usize]
                {
                    len -= text_len;
                }
            }
        }
        *prefix_len = len as usize;
        return Some(crate::globals::GraphPtr::raw_slice(first, len as usize));
    }
    None
}

pub fn ins_compl_insert(move_cursor: bool, insert_prefix: bool) {
    let compl_len = get_compl_len();
    let preinsert = ins_compl_has_preinsert();
    let shown = compl_opt(insexp().compl_shown_idx).unwrap();
    let mut cp_str = shown.cp_str.data;
    let mut cp_str_len = shown.cp_str.size;
    let leader_len = ins_compl_leader_len();
    let has_multiple = shown.cp_str.cstr_bytes().contains(&b'\n');

    if insert_prefix {
        if let Some(pref) = find_common_prefix(&mut cp_str_len, false)
            .or_else(|| find_common_prefix(&mut cp_str_len, true))
        {
            cp_str = crate::globals::GraphPtr::new(pref.as_ptr().cast_mut());
        } else {
            let shown = compl_opt(insexp().compl_shown_idx).unwrap();
            cp_str = shown.cp_str.data;
            cp_str_len = shown.cp_str.size;
        }
    } else if !insexp().cpt_sources_array.is_empty() {
        let cpt_idx = shown.cp_cpt_source_idx;
        if cpt_idx >= 0 && insexp().compl_col >= 0 {
            let startcol = insexp().cpt_sources_array[cpt_idx as usize].cs_startcol;
            if startcol >= 0 && startcol < insexp().compl_col {
                let skip = insexp().compl_col - startcol;
                if (skip as usize) <= cp_str_len {
                    cp_str_len -= skip as usize;
                    cp_str = cp_str.wrapping_add(skip as usize);
                }
            }
        }
    }

    if compl_len < cp_str_len as i32 {
        let cp_slice = crate::globals::GraphPtr::raw_slice(cp_str.as_ptr(), cp_str_len);
        let skip = (compl_len.max(0) as usize).min(cp_slice.len());
        if has_multiple {
            ins_compl_expand_multiple(&cp_slice[skip..]);
        } else {
            ins_compl_insert_bytes(
                &cp_slice[skip..],
                if insert_prefix {
                    (cp_str_len as i32) - compl_len
                } else {
                    -1
                },
            );
            if (preinsert || insert_prefix) && move_cursor {
                curwin_mut().w_cursor.col -= (cp_str_len - leader_len) as Colnr;
            }
        }
    }
    let shown = compl_opt(insexp().compl_shown_idx);

    insexp().compl_used_match =
        !(shown.as_deref().is_some_and(match_at_original_text) || (preinsert && !insert_prefix));

    let dict = ins_compl_dict_alloc(shown);
    set_vim_var_dict(VV_COMPLETED_ITEM, Some(dict));
    insexp().compl_hi_on_autocompl_longest = insert_prefix && move_cursor;
}

fn ins_compl_show_filename() {
    let lead = gettext(c"match in file");

    let mut space = (if ui_has(K_UIMESSAGES) {
        g_st().columns_global
    } else {
        g_st().sc_col
    }) - vim_strsize(lead)
        - 2;
    if space <= 0 {
        return;
    }

    let fname_cstr = compl_opt(insexp().compl_shown_idx)
        .and_then(|shown| shown.cp_fname.as_deref())
        .unwrap_or(c"");
    let mut s = fname_cstr;
    let mut e = s;
    while !e.to_bytes().is_empty() {
        space -= ptr2cells(e.to_bytes());
        while space < 0 {
            space += ptr2cells(s.to_bytes());
            let adv = crate::mbyte::utfc_ptr2len(s.to_bytes()) as usize;
            s = crate::memory::CStrSubslice::advance(s, adv);
        }
        let adv = crate::mbyte::utfc_ptr2len(e.to_bytes()) as usize;
        e = crate::memory::CStrSubslice::advance(e, adv);
    }
    if !insexp().compl_autocomplete {
        g_st().msg_hist_off = true;
        format_to_buf!(
            &mut g_st().iobuff,
            c"%s %s%s",
            lead,
            if (s.as_ptr() as usize) > (fname_cstr.as_ptr() as usize) {
                c"<"
            } else {
                c""
            },
            s,
        );
        msg(crate::memory::bytes_as_cstr(g_st().iobuff_bytes()), 0);
        g_st().msg_hist_off = false;
        g_st().redraw_cmdline = false;
    }
}

fn find_next_match_in_menu() -> Option<usize> {
    let is_forward = compl_shows_dir_forward();
    let mut match_idx = insexp().compl_shown_idx?;

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
        if compl_next_idx(match_idx).is_none() || m.cp_in_match_array || match_at_original_text(&*m)
        {
            break;
        }
    }
    Some(match_idx)
}

fn find_next_completion_match(
    allow_get_expansion: bool,
    mut todo: i32,
    advance: bool,
    num_matches: &mut i32,
) -> bool {
    let mut found_end: bool;
    let mut found_compl: Option<usize> = None;
    let cur_cot_flags = get_cot_flags();
    let compl_no_select = (cur_cot_flags & K_OPT_COT_FLAG_NOSELECT) != 0
        || (insexp().compl_autocomplete && !ins_compl_has_preinsert());

    loop {
        todo -= 1;
        if todo < 0 {
            break;
        }
        let arr = &insexp().compl_match_array;
        let shown_idx = insexp().compl_shown_idx.unwrap();
        if compl_shows_dir_forward() && compl_next_idx(shown_idx).is_some() {
            if !arr.is_empty() {
                insexp().compl_shown_idx = find_next_match_in_menu();
            } else {
                insexp().compl_shown_idx = compl_next_idx(shown_idx);
            }
            let new_shown = insexp().compl_shown_idx.unwrap();
            found_end = !insexp().compl_list.is_empty()
                && (compl_next_idx(new_shown) == Some(0) || new_shown == 0);
        } else if compl_shows_dir_backward() && compl_prev_idx(shown_idx).is_some() {
            found_end = shown_idx == 0;
            if !arr.is_empty() {
                insexp().compl_shown_idx = find_next_match_in_menu();
            } else {
                insexp().compl_shown_idx = compl_prev_idx(shown_idx);
            }
            found_end |= insexp().compl_shown_idx == Some(0);
        } else {
            if !allow_get_expansion {
                if advance {
                    if compl_shows_dir_backward() {
                        insexp().compl_pending -= todo + 1;
                    } else {
                        insexp().compl_pending += todo + 1;
                    }
                }
                return false;
            }

            if !compl_no_select && advance {
                if compl_shows_dir_backward() {
                    insexp().compl_pending -= 1;
                } else {
                    insexp().compl_pending += 1;
                }
            }

            *num_matches = ins_compl_get_exp(&mut insexp().compl_startpos);

            while insexp().compl_pending != 0
                && insexp().compl_direction == insexp().compl_shows_dir
                && advance
            {
                let cur_shown = insexp().compl_shown_idx.unwrap();
                if insexp().compl_pending > 0
                    && let Some(ni) = compl_next_idx(cur_shown)
                {
                    insexp().compl_shown_idx = Some(ni);
                    insexp().compl_pending -= 1;
                } else if insexp().compl_pending < 0
                    && let Some(pi) = compl_prev_idx(cur_shown)
                {
                    insexp().compl_shown_idx = Some(pi);
                    insexp().compl_pending += 1;
                } else {
                    break;
                }
            }
            found_end = false;
        }

        let leader = get_leader_for_startcol(compl_opt(insexp().compl_shown_idx).as_deref(), false);
        let shown = compl_opt(insexp().compl_shown_idx).unwrap();

        if !match_at_original_text(shown)
            && !leader.data.is_null()
            && !ins_compl_equal(shown, leader.as_bytes())
            && !(cot_fuzzy() && shown.cp_score != FUZZY_SCORE_NONE)
        {
            todo += 1;
        } else {
            found_compl = insexp().compl_shown_idx;
        }

        if found_end {
            if found_compl.is_some() {
                insexp().compl_shown_idx = found_compl;
                break;
            }
            todo = 1;
        }
    }

    true
}

fn ins_compl_next(allow_get_expansion: bool, count: i32, insert_match: bool) -> i32 {
    let mut num_matches: i32 = -1;
    let todo = count;
    let started = insexp().compl_started;
    let orig_curbuf = g_st().curbuf;
    let cur_cot_flags = get_cot_flags();
    let compl_no_insert = (cur_cot_flags & K_OPT_COT_FLAG_NOINSERT) != 0
        || (insexp().compl_autocomplete && !ins_compl_has_preinsert());
    let compl_preinsert = ins_compl_has_preinsert();

    if { insexp().compl_shown_idx }.is_none() {
        return -1;
    }

    if !insexp().compl_leader.data.is_null()
        && !compl_opt(insexp().compl_shown_idx)
            .as_deref()
            .is_some_and(match_at_original_text)
        && !cot_fuzzy()
    {
        ins_compl_update_shown_match();
    }

    if allow_get_expansion
        && insert_match
        && (!insexp().compl_get_longest || insexp().compl_used_match)
    {
        ins_compl_delete(false);
    }

    let mut advance = count != 1 || !allow_get_expansion || !insexp().compl_get_longest;

    if insexp().compl_restarting {
        advance = false;
        insexp().compl_restarting = false;
    }

    if !find_next_completion_match(allow_get_expansion, todo, advance, &mut num_matches) {
        return -1;
    }

    if g_st().curbuf != orig_curbuf {
        return -1;
    }

    if !started && ins_compl_preinsert_longest() {
        ins_compl_insert(true, true);
    } else if compl_no_insert && !started && !compl_preinsert {
        let orig_b = insexp().compl_orig_text.as_bytes();
        let skip = (get_compl_len().max(0) as usize).min(orig_b.len());
        ins_compl_insert_bytes(&orig_b[skip..], -1);
        insexp().compl_used_match = false;
        restore_orig_extmarks();
    } else if insert_match {
        if !insexp().compl_get_longest || insexp().compl_used_match {
            let preinsert_longest = ins_compl_preinsert_longest()
                && compl_opt(insexp().compl_shown_idx)
                    .as_deref()
                    .is_some_and(match_at_original_text);
            ins_compl_insert(compl_preinsert || preinsert_longest, preinsert_longest);
        } else {
            debug_assert!(!insexp().compl_leader.data.is_null());
            let lead_b = insexp().compl_leader.as_bytes();
            let skip = (get_compl_len().max(0) as usize).min(lead_b.len());
            ins_compl_insert_bytes(&lead_b[skip..], -1);
        }
        if compl_opt(insexp().compl_shown_idx)
            .is_some_and(|shown| shown.cp_str.opt_bytes() == insexp().compl_orig_text.opt_bytes())
        {
            restore_orig_extmarks();
        }
    } else {
        insexp().compl_used_match = false;
    }

    if !allow_get_expansion {
        update_screen();
        ins_compl_show_pum();
        ins_compl_delete(false);
    }

    let shown = compl_opt(insexp().compl_shown_idx).unwrap();
    if compl_no_insert && !started && !match_at_original_text(shown) {
        insexp().compl_enter_selects = true;
    } else {
        let arr = &insexp().compl_match_array;
        insexp().compl_enter_selects = !insert_match && !arr.is_empty();
    }

    if compl_opt(insexp().compl_shown_idx).is_some_and(|s| s.cp_fname.is_some()) {
        ins_compl_show_filename();
    }

    num_matches
}

fn check_elapsed_time() {
    let start_tv = insexp().cpt_sources_array[insexp().cpt_sources_index as usize].compl_start_tv;
    let elapsed_ms = (os_hrtime() - start_tv) / 1_000_000;

    if elapsed_ms > insexp().compl_timeout_ms {
        insexp().compl_time_slice_expired = true;
        decay_compl_timeout();
    }
}

pub fn ins_compl_check_keys(frequency: i32, in_compl_func: bool) {
    static COUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    {
        if !in_compl_func && (using_script() != 0 || g_st().ex_normal_busy != 0) {
            return;
        }

        let cur = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if cur < frequency {
            return;
        }
        COUNT.store(0, std::sync::atomic::Ordering::Relaxed);

        let mut c = vpeekc_any();
        if c != NUL as i32 && !test_disable_char_avail.get() {
            if vim_is_ctrl_x_key(c) && c != CTRL_X_CONST && c != CTRL_R_CONST {
                c = safe_vgetc();
                insexp().compl_shows_dir = ins_compl_key2dir(c);
                let _ = ins_compl_next(false, ins_compl_key2count(c), c != K_UP && c != K_DOWN);
            } else {
                c = safe_vgetc();
                if c != K_IGNORE {
                    if c != CTRL_R_CONST
                        && (g_st().key_typed || (in_compl_func && g_st().reg_executing == 0))
                    {
                        insexp().compl_interrupted = true;
                    }

                    vungetc(c);
                }
            }
        } else {
            let cpt_sources = &insexp().cpt_sources_array;
            let normal_mode_strict = ctrl_x_mode_normal()
                && !ctrl_x_mode_line_or_eval()
                && (insexp().compl_cont_status & CONT_LOCAL) == 0
                && !cpt_sources.is_empty()
                && insexp().cpt_sources_index >= 0;
            if normal_mode_strict && (insexp().compl_autocomplete || opt_vars().p_cto > 0) {
                check_elapsed_time();
            }
        }

        if insexp().compl_pending != 0
            && !g_st().got_int
            && (opt_vars().cot_flags & (K_OPT_COT_FLAG_NOINSERT | K_OPT_COT_FLAG_FUZZY)) == 0
            && (!insexp().compl_autocomplete || ins_compl_has_preinsert())
        {
            let todo = insexp().compl_pending.abs();
            insexp().compl_pending = 0;
            let _ = ins_compl_next(false, todo, true);
        }
    }
}

fn ins_compl_key2dir(c: i32) -> Direction {
    if c == K_EVENT || c == K_COMMAND || c == K_LUA {
        return if g_st().pum_want.item < insexp().compl_selected_item {
            BACKWARD
        } else {
            FORWARD
        };
    }
    if c == CTRL_P
        || c == CTRL_L_CONST
        || c == K_PAGEUP
        || c == K_KPAGEUP
        || c == K_S_UP
        || c == K_UP
    {
        return BACKWARD;
    }
    FORWARD
}

fn ins_compl_pum_key(c: i32) -> bool {
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

fn ins_compl_key2count(c: i32) -> i32 {
    if c == K_EVENT || c == K_COMMAND || c == K_LUA {
        return (g_st().pum_want.item - insexp().compl_selected_item).abs();
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

fn ins_compl_use_match(c: i32) -> bool {
    match c {
        K_UP | K_DOWN | K_PAGEDOWN | K_KPAGEDOWN | K_S_DOWN | K_PAGEUP | K_KPAGEUP | K_S_UP => {
            false
        }
        K_EVENT | K_COMMAND | K_LUA => g_st().pum_want.active && g_st().pum_want.insert,
        _ => true,
    }
}

fn get_normal_compl_info(line_cstr: &CStr, mut startcol: i32, curs_col: Colnr) -> bool {
    let line = line_cstr.as_ptr().cast_mut();
    let line_bytes = line_cstr.to_bytes();
    if (insexp().compl_cont_status & CONT_SOL) != 0 || ctrl_x_mode_path_defines() {
        if !compl_status_adding() {
            while {
                startcol -= 1;
                startcol >= 0
                    && vim_is_idc(i32::from(
                        line_bytes.get(startcol as usize).copied().unwrap_or(0),
                    ))
            } {}
            startcol += 1;
            insexp().compl_col += startcol;
            insexp().compl_length = curs_col - startcol;
        }
        let pat_bytes = crate::globals::GraphPtr::raw_slice(
            line.wrapping_add(insexp().compl_col as usize).cast::<u8>(),
            insexp().compl_length.max(0) as usize,
        );
        if opt_vars().p_ic {
            insexp().compl_pattern = str_foldcase(pat_bytes, None);
        } else {
            set_compl_pattern_bytes(pat_bytes);
        }
    } else if compl_status_adding() {
        let mut prefix: &[u8] = b"\\<";
        let mut prefixlen: usize = 2;

        let line_at_col =
            unsafe { CStr::from_ptr(line.wrapping_add(insexp().compl_col as usize).cast()) }
                .to_bytes();
        if !vim_iswordp(line_at_col)
            || (insexp().compl_col > 0 && {
                let prev_p = {
                    let _l = line;
                    let _p = line.wrapping_add(insexp().compl_col as usize);
                    if (_p as usize) > (_l as usize) {
                        _p.wrapping_sub(
                            1 + crate::mbyte::utf_head_off_ptr(_l, _p.wrapping_sub(1)) as usize,
                        )
                    } else {
                        _p
                    }
                };
                vim_iswordp(unsafe { CStr::from_ptr(prev_p.cast()) }.to_bytes())
            })
        {
            prefix = b"";
            prefixlen = 0;
        }

        let word_slice = crate::globals::GraphPtr::raw_slice(
            line.wrapping_add(insexp().compl_col as usize).cast::<u8>(),
            insexp().compl_length.max(0) as usize,
        );
        let n = quote_meta(None, word_slice) + prefixlen;
        let mut pat_vec: Vec<u8> = vec![0; n];
        pat_vec[..prefixlen].copy_from_slice(&prefix[..prefixlen]);
        quote_meta(Some(&mut pat_vec[prefixlen..]), word_slice);
        set_compl_pattern_bytes(&pat_vec[..n - 1]);
    } else {
        startcol -= 1;
        if startcol < 0 || {
            let prev_p = {
                let _l = line;
                let _p = line.wrapping_add((startcol + 1) as usize);
                if (_p as usize) > (_l as usize) {
                    _p.wrapping_sub(
                        1 + crate::mbyte::utf_head_off_ptr(_l, _p.wrapping_sub(1)) as usize,
                    )
                } else {
                    _p
                }
            };
            !vim_iswordp(unsafe { CStr::from_ptr(prev_p.cast()) }.to_bytes())
        } {
            insexp().compl_pattern = Some(c"\\<\\k\\k".to_owned());
            insexp().compl_col += curs_col;
            insexp().compl_length = 0;
            insexp().compl_from_nonkeyword = true;
        } else {
            startcol -= crate::mbyte::utf_head_off_ptr(line, line.wrapping_add(startcol as usize));
            let base_class = mb_get_class(
                unsafe { CStr::from_ptr(line.wrapping_add(startcol as usize).cast()) }.to_bytes(),
            );
            while {
                startcol -= 1;
                startcol >= 0
            } {
                let head_off =
                    crate::mbyte::utf_head_off_ptr(line, line.wrapping_add(startcol as usize));
                if base_class
                    != mb_get_class(
                        unsafe {
                            CStr::from_ptr(line.wrapping_add((startcol - head_off) as usize).cast())
                        }
                        .to_bytes(),
                    )
                {
                    break;
                }
                startcol -= head_off;
            }

            startcol += 1;
            insexp().compl_col += startcol;
            insexp().compl_length = curs_col - startcol;
            if insexp().compl_length == 1 {
                let mut pat_vec = [0u8; 7];
                pat_vec[..2].copy_from_slice(b"\\<");
                let word_slice = crate::globals::GraphPtr::raw_slice(
                    line.wrapping_add(insexp().compl_col as usize).cast::<u8>(),
                    1,
                );
                let qlen = quote_meta(Some(&mut pat_vec[2..]), word_slice) - 1;
                pat_vec[2 + qlen..2 + qlen + 2].copy_from_slice(b"\\k");
                set_compl_pattern_bytes(&pat_vec[..4 + qlen]);
            } else {
                let word_slice = crate::globals::GraphPtr::raw_slice(
                    line.wrapping_add(insexp().compl_col as usize).cast::<u8>(),
                    insexp().compl_length.max(0) as usize,
                );
                let n = quote_meta(None, word_slice) + 2;
                let mut pat_vec: Vec<u8> = vec![0; n];
                pat_vec[..2].copy_from_slice(b"\\<");
                quote_meta(Some(&mut pat_vec[2..]), word_slice);
                set_compl_pattern_bytes(&pat_vec[..n - 1]);
            }
        }
    }

    if ctrl_x_mode_normal() && (insexp().compl_cont_status & CONT_LOCAL) == 0 {
        setup_cpt_sources();
        prepare_cpt_compl_funcs();
    }

    true
}

fn get_wholeline_compl_info(line: &CStr, curs_col: Colnr) -> bool {
    let line_bytes = line.to_bytes();
    insexp().compl_col = getwhitecols(line_bytes) as Colnr;
    insexp().compl_length = curs_col - insexp().compl_col;
    if insexp().compl_length < 0 {
        insexp().compl_length = 0;
    }
    let start = (insexp().compl_col as usize).min(line_bytes.len());
    let end = (start + insexp().compl_length as usize).min(line_bytes.len());
    let pat_bytes = &line_bytes[start..end];
    if opt_vars().p_ic {
        insexp().compl_pattern = str_foldcase(pat_bytes, None);
    } else {
        set_compl_pattern_bytes(pat_bytes);
    }

    true
}

fn get_filename_compl_info(line: &CStr, mut startcol: i32, curs_col: Colnr) -> bool {
    let line_bytes = line.to_bytes();
    if startcol > 0 {
        let mut p_idx = (startcol as usize).min(line_bytes.len());

        crate::mbyte::mb_idx_back(line_bytes, &mut p_idx);
        while p_idx > 0 && vim_isfilec(utf_slice2char(&line_bytes[p_idx..])) {
            crate::mbyte::mb_idx_back(line_bytes, &mut p_idx);
        }
        let mut p_is_filec = false;
        #[cfg(windows)]
        {
            if p_idx > 0
                && path_has_drive_letter(
                    &line_bytes[p_idx - 1..(startcol as usize).min(line_bytes.len())],
                )
            {
                p_idx -= if p_idx == 1 { 1 } else { 2 };
                p_is_filec = true;
            }
        }
        p_is_filec = p_is_filec || vim_isfilec(utf_slice2char(&line_bytes[p_idx..]));

        if p_idx == 0 && p_is_filec {
            startcol = 0;
        } else {
            startcol = p_idx as i32 + 1;
        }
    }

    insexp().compl_col += startcol;
    insexp().compl_length = curs_col - startcol;
    let start = (insexp().compl_col as usize).min(line_bytes.len());
    let end = (start + insexp().compl_length as usize).min(line_bytes.len());
    insexp().compl_pattern = Some(addstar(&line_bytes[start..end], EXPAND_FILES));

    true
}

fn get_cmdline_compl_info(line: &CStr, curs_col: Colnr) -> bool {
    let line_bytes = line.to_bytes();
    set_compl_pattern_bytes(&line_bytes[..(curs_col as usize).min(line_bytes.len())]);
    let pat_cstr = compl_pattern_cstr().unwrap_or(c"");
    let pat_len = compl_pattern_len();
    set_cmd_context(
        &mut insexp().compl_xp,
        crate::globals::GraphPtr::raw_slice_mut(
            pat_cstr.as_ptr().cast_mut().cast::<u8>(),
            pat_len + 1,
        ),
        pat_len as i32,
        curs_col,
        false,
    );
    if insexp().compl_xp.xp_context == EXPAND_LUA {
        nlua_expand_pat(&mut insexp().compl_xp);
    }
    if insexp().compl_xp.xp_context == EXPAND_UNSUCCESSFUL
        || insexp().compl_xp.xp_context == EXPAND_NOTHING
    {
        insexp().compl_col = curs_col;
    } else {
        insexp().compl_col =
            ((insexp().compl_xp.xp_pattern).offset_from(pat_cstr.as_ptr()) as isize) as Colnr;
    }
    insexp().compl_length = curs_col - insexp().compl_col;

    true
}

fn set_compl_globals(mut startcol: i32, curs_col: Colnr, is_cpt_compl: bool) {
    if is_cpt_compl {
        api_clear_string(&mut insexp().cpt_compl_pattern);
        if startcol < insexp().compl_col {
            prepend_startcol_text(
                &mut insexp().cpt_compl_pattern,
                &insexp().compl_orig_text,
                startcol,
            );
        } else {
            insexp().cpt_compl_pattern = copy_string(insexp().compl_orig_text, None);
        }
    } else {
        if startcol < 0 || startcol > curs_col {
            startcol = curs_col;
        }

        let line_slice = crate::cursor::get_cursor_line_slice();
        let len = curs_col - startcol;

        set_compl_pattern_bytes(&line_slice[startcol as usize..(startcol + len) as usize]);
        insexp().compl_col = startcol;
        insexp().compl_length = len;
    }
}

fn get_userdefined_compl_info(
    curs_col: Colnr,
    cb_opt: Option<&mut Callback>,
    startcol: Option<&mut i32>,
) -> bool {
    let save_state = g_st().state_global;

    let is_cpt_function = cb_opt.is_some();
    let use_sandbox = is_cpt_function
        && was_set_insecurely(
            Some(crate::globals::curwin_mut()),
            K_OPT_COMPLETE,
            OPT_LOCAL,
        ) != 0;
    let cb = match cb_opt {
        Some(c) => c,
        None => {
            let c = get_insert_callback(insexp().ctrl_x_mode);
            if c.cb_type() == K_CALLBACK_NONE_CONST {
                semsg!(
                    gettext(E_NOTSET),
                    if ctrl_x_mode_function() {
                        c"completefunc"
                    } else {
                        c"omnifunc"
                    },
                );
                return false;
            }
            c
        }
    };

    let mut args: [TypVal; 3] = [const { TypVal::new() }; 3];
    args[0].v_type = VAR_NUMBER;
    args[1].v_type = VAR_STRING;
    args[2].v_type = VAR_UNKNOWN;
    args[0].vval = crate::eval::typval::TypvalVval::Number(1);
    args[1].vval = crate::eval::typval::TypvalVval::from_cstr(c"");

    let pos = curwin_mut().w_cursor;
    g_st().textlock += 1;
    if use_sandbox {
        g_st().sandbox += 1;
    }
    let col = callback_call_retnr(cb, 2, &mut args) as Colnr;
    if use_sandbox {
        g_st().sandbox -= 1;
    }
    g_st().textlock -= 1;

    g_st().state_global = save_state;
    curwin_mut().w_cursor = pos;
    check_cursor(curwin_mut());
    validate_cursor(curwin_mut());
    if !equalpos(curwin_mut().w_cursor, pos) {
        emsg(gettext(E_COMPLDEL));
        return false;
    }

    if let Some(startcol) = startcol {
        *startcol = col;
    }

    if col == -2 || aborting() {
        return false;
    }

    if col == -3 {
        if is_cpt_function {
            return false;
        }
        insexp().ctrl_x_mode = CTRL_X_NORMAL;
        g_st().edit_submode = None;
        if !shortmess(K_SHM_COMPLETIONMENU) {
            msg_clr_cmdline();
        }
        return false;
    }

    insexp().compl_opt_refresh_always = false;

    if !is_cpt_function {
        set_compl_globals(col, curs_col, false);
    }
    true
}

fn get_spell_compl_info(startcol: i32, curs_col: Colnr) -> bool {
    if insexp().spell_bad_len > 0 {
        debug_assert!(insexp().spell_bad_len <= i32::MAX as usize);
        insexp().compl_col = curs_col - insexp().spell_bad_len as i32;
    } else {
        insexp().compl_col = spell_word_start(startcol);
    }
    if insexp().compl_col >= startcol {
        insexp().compl_length = 0;
        insexp().compl_col = curs_col;
    } else {
        spell_expand_check_cap(insexp().compl_col);
        insexp().compl_length = curs_col - insexp().compl_col;
    }
    let line_slice = crate::cursor::get_cursor_line_slice();
    let start = insexp().compl_col as usize;
    let end = start + insexp().compl_length as usize;
    set_compl_pattern_bytes(&line_slice[start..end.min(line_slice.len())]);

    true
}

fn compl_get_info(line: &CStr, startcol: i32, curs_col: Colnr, line_invalid: &mut bool) -> bool {
    if ctrl_x_mode_normal()
        || ctrl_x_mode_register()
        || ((insexp().ctrl_x_mode & CTRL_X_WANT_IDENT) != 0
            && !thesaurus_func_complete(insexp().ctrl_x_mode))
    {
        if !get_normal_compl_info(line, startcol, curs_col) {
            return false;
        }
        *line_invalid = true;
    } else if ctrl_x_mode_line_or_eval() {
        return get_wholeline_compl_info(line, curs_col);
    } else if ctrl_x_mode_files() {
        return get_filename_compl_info(line, startcol, curs_col);
    } else if insexp().ctrl_x_mode == CTRL_X_CMDLINE {
        return get_cmdline_compl_info(line, curs_col);
    } else if ctrl_x_mode_function()
        || ctrl_x_mode_omni()
        || thesaurus_func_complete(insexp().ctrl_x_mode)
    {
        if !get_userdefined_compl_info(curs_col, None, None) {
            return false;
        }
        *line_invalid = true;
    } else if ctrl_x_mode_spell() {
        if !get_spell_compl_info(startcol, curs_col) {
            return false;
        }
        *line_invalid = true;
    } else {
        internal_error(c"ins_complete()");
        return false;
    }

    true
}

fn ins_compl_continue_search(line_cstr: &CStr) {
    let line_bytes = line_cstr.to_bytes();
    insexp().compl_cont_status &= !CONT_INTRPT;
    if ctrl_x_mode_normal() || ctrl_x_mode_path_patterns() || ctrl_x_mode_path_defines() {
        if insexp().compl_startpos.lnum != curwin_mut().w_cursor.lnum {
            insexp().compl_col = getwhitecols(line_bytes) as Colnr;
            insexp().compl_startpos.col = insexp().compl_col;
            insexp().compl_startpos.lnum = curwin_mut().w_cursor.lnum;
            insexp().compl_cont_status &= !CONT_SOL;
        } else {
            if (insexp().compl_cont_status & CONT_S_IPOS) != 0 {
                insexp().compl_cont_status |= CONT_SOL;
                let off = ((insexp().compl_length + insexp().compl_startpos.col).max(0) as usize)
                    .min(line_bytes.len());
                insexp().compl_startpos.col =
                    (off + crate::charset::skipwhite_len(&line_bytes[off..])) as Colnr;
            }
            insexp().compl_col = insexp().compl_startpos.col;
        }
        insexp().compl_length = curwin_mut().w_cursor.col - insexp().compl_col;
        if insexp().compl_length > (IOSIZE as i32 - MIN_SPACE) {
            insexp().compl_cont_status &= !CONT_SOL;
            insexp().compl_length = IOSIZE as i32 - MIN_SPACE;
            insexp().compl_col = curwin_mut().w_cursor.col - insexp().compl_length;
        }
        insexp().compl_cont_status |= CONT_ADDING | CONT_N_ADDS;
        if insexp().compl_length < 1 {
            insexp().compl_cont_status &= CONT_LOCAL;
        }
    } else if ctrl_x_mode_line_or_eval() || ctrl_x_mode_register() {
        insexp().compl_cont_status = CONT_ADDING | CONT_N_ADDS;
    } else {
        insexp().compl_cont_status = 0;
    }
}

fn ins_compl_start() -> bool {
    let save_did_ai = g_st().ins_global.did_ai;

    g_st().ins_global.did_ai = false;
    g_st().ins_global.did_si = false;
    g_st().ins_global.can_si = false;
    g_st().ins_global.can_si_back = false;
    if !stop_arrow() {
        g_st().ins_global.did_ai = save_did_ai;
        return false;
    }

    let mut line_cstr = ml_get(curwin_mut().w_cursor.lnum);
    let curs_col = curwin_mut().w_cursor.col;
    insexp().compl_pending = 0;
    insexp().compl_lnum = curwin_mut().w_cursor.lnum;

    if (insexp().compl_cont_status & CONT_INTRPT) == CONT_INTRPT
        && insexp().compl_cont_mode == insexp().ctrl_x_mode
    {
        ins_compl_continue_search(line_cstr);
    } else {
        insexp().compl_cont_status &= CONT_LOCAL;
    }

    let mut startcol: i32 = 0;
    if !compl_status_adding() {
        insexp().compl_cont_mode = insexp().ctrl_x_mode;
        if ctrl_x_mode_not_default() {
            insexp().compl_cont_status = 0;
        }
        insexp().compl_cont_status |= CONT_N_ADDS;
        insexp().compl_startpos = curwin_mut().w_cursor;
        startcol = curs_col;
        insexp().compl_col = 0;
    }

    let mut line_invalid = false;
    if !compl_get_info(line_cstr, startcol, curs_col, &mut line_invalid) {
        if ctrl_x_mode_function()
            || ctrl_x_mode_omni()
            || thesaurus_func_complete(insexp().ctrl_x_mode)
        {
            g_st().ins_global.did_ai = save_did_ai;
        }
        return false;
    }
    if line_invalid {
        line_cstr = ml_get(curwin_mut().w_cursor.lnum);
    }

    if compl_status_adding() {
        if !shortmess(K_SHM_COMPLETIONMENU) {
            g_st().edit_submode_pre = Some(gettext(c" Adding"));
        }
        if ctrl_x_mode_line_or_eval() {
            let old = std::mem::take(&mut curbuf_mut().b_p_com);
            insexp().compl_startpos.lnum = curwin_mut().w_cursor.lnum;
            insexp().compl_startpos.col = insexp().compl_col;
            ins_eol(b'\r' as i32);
            curbuf_mut().b_p_com = old;
            insexp().compl_length = 0;
            insexp().compl_col = curwin_mut().w_cursor.col;
            insexp().compl_lnum = curwin_mut().w_cursor.lnum;
        }
    } else {
        g_st().edit_submode_pre = None;
        insexp().compl_startpos.col = insexp().compl_col;
    }

    if !shortmess(K_SHM_COMPLETIONMENU) && !insexp().compl_autocomplete {
        if (insexp().compl_cont_status & CONT_LOCAL) != 0 {
            g_st().edit_submode = Some(gettext((ctrl_x_msg(CTRL_X_LOCAL_MSG)).unwrap_or(c"")));
        } else {
            g_st().edit_submode = Some(gettext((ctrl_x_msg(insexp().ctrl_x_mode)).unwrap_or(c"")));
        }
    }

    ins_compl_fix_redo_buf_for_leader(None);

    api_clear_string(&mut insexp().compl_orig_text);
    insexp().compl_orig_extmarks.clear();

    let line_with_nul = line_cstr.to_bytes_with_nul();
    let col_u = (insexp().compl_col.max(0) as usize).min(line_with_nul.len());
    let len_u = (insexp().compl_length.max(0) as usize).min(line_with_nul.len() - col_u);
    insexp().compl_orig_text = cbuf_to_string(&line_with_nul[col_u..col_u + len_u]);
    save_orig_extmarks();
    let mut flags = CP_ORIGINAL_TEXT;
    if opt_vars().p_ic {
        flags |= CP_ICASE;
    }
    if ins_compl_add(
        insexp().compl_orig_text.as_bytes(),
        insexp().compl_orig_text.size as i32,
        None,
        [const { None }; CPT_COUNT],
        None,
        0,
        flags,
        false,
        None,
        FUZZY_SCORE_NONE,
        false,
        None,
    ) != Some(true)
    {
        insexp().compl_pattern = None;
        api_clear_string(&mut insexp().compl_orig_text);
        let extmarks = &mut insexp().compl_orig_extmarks;
        extmarks.clear();
        g_st().ins_global.did_ai = save_did_ai;
        return false;
    }

    if !shortmess(K_SHM_COMPLETIONMENU) && !insexp().compl_autocomplete {
        g_st().edit_submode_extra = Some(gettext(c"-- Searching..."));
        g_st().edit_submode_highl = HLF_COUNT;
        showmode();
        g_st().edit_submode_extra = None;
        ui_flush();
    }

    g_st().ins_global.did_ai = save_did_ai;
    true
}

fn ins_compl_show_statusmsg() {
    if compl_next_idx(0) == Some(0) {
        g_st().edit_submode_extra = Some(if compl_status_adding() && insexp().compl_length > 1 {
            gettext(c"Hit end of paragraph")
        } else {
            gettext(c"Pattern not found")
        });
        g_st().edit_submode_highl = HLF_E;
    }

    if g_st().edit_submode_extra.is_none() {
        let curr_idx = insexp().compl_curr_idx.unwrap();
        let curr = compl_at(curr_idx);
        if match_at_original_text(&*curr) {
            g_st().edit_submode_extra = Some(gettext(c"Back at original"));
            g_st().edit_submode_highl = HLF_W;
        } else if (insexp().compl_cont_status & CONT_S_IPOS) != 0 {
            g_st().edit_submode_extra = Some(gettext(c"Word from other line"));
            g_st().edit_submode_highl = HLF_COUNT;
        } else if compl_next_idx(curr_idx) == compl_prev_idx(curr_idx) {
            g_st().edit_submode_extra = Some(gettext(c"The only match"));
            g_st().edit_submode_highl = HLF_COUNT;
            curr.cp_number = 1;
        } else {
            if curr.cp_number == -1 {
                ins_compl_update_sequence_numbers();
            }

            let curr = compl_at(curr_idx);
            if curr.cp_number != -1 {
                if insexp().compl_matches > 0 {
                    format_to_buf!(
                        &mut insexp().match_ref,
                        gettext(c"match %d of %d"),
                        curr.cp_number,
                        insexp().compl_matches,
                    );
                } else {
                    format_to_buf!(
                        &mut insexp().match_ref,
                        gettext(c"match %d"),
                        curr.cp_number,
                    );
                }
                g_st().edit_submode_extra =
                    Some(unsafe { CStr::from_ptr(insexp().match_ref.as_ptr().cast()) });
                g_st().edit_submode_highl = HLF_R;
                if g_st().dollar_vcol >= 0 {
                    curs_columns(curwin_mut(), false);
                }
            }
        }
    }

    g_st().redraw_mode = true;
    if !shortmess(K_SHM_COMPLETIONMENU) {
        if let Some(extra) = g_st().edit_submode_extra {
            if !opt_vars().p_smd {
                g_st().msg_hist_off = true;
                msg_ext_set_kind(c"completion");
                msg(
                    extra,
                    if g_st().edit_submode_highl < HLF_COUNT {
                        g_st().edit_submode_highl + 1
                    } else {
                        0
                    },
                );
                g_st().msg_hist_off = false;
            }
        } else {
            msg_clr_cmdline();
        }
    }
}

pub fn ins_complete(c: i32, enable_pum: bool) -> bool {
    insexp().compl_direction = ins_compl_key2dir(c);
    let insert_match = ins_compl_use_match(c);

    if !insexp().compl_started {
        insexp().compl_autostarted = insexp().compl_autostart_pending;
        insexp().compl_autostart_pending = false;
        if !ins_compl_start() {
            return false;
        }
    } else if insert_match && !stop_arrow() {
        return false;
    }

    insexp().compl_curr_win = crate::globals::curwin_ptr();
    insexp().compl_curr_buf = crate::globals::BufPtr::new(curwin_mut().w_buffer.as_ptr());
    insexp().compl_shown_idx = insexp().compl_curr_idx;
    insexp().compl_shows_dir = insexp().compl_direction;
    insexp().compl_num_bests = 0;

    let save_w_wrow = curwin_mut().w_wrow;
    let save_w_leftcol = curwin_mut().w_leftcol;
    let n = ins_compl_next(true, ins_compl_key2count(c), insert_match);

    if insexp().compl_autocomplete {
        insexp().compl_time_slice_expired = false;
    }

    if n > 1 {
        insexp().compl_matches = n;
    }
    insexp().compl_curr_idx = insexp().compl_shown_idx;
    insexp().compl_direction = insexp().compl_shows_dir;

    if g_st().got_int && g_st().global_busy == 0 {
        let _ = vgetc();
        g_st().got_int = false;
    }

    let no_matches_found = compl_next_idx(0) == Some(0);
    if no_matches_found
        && (insexp().compl_length > 1
            || compl_status_adding()
            || (ctrl_x_mode_not_default()
                && !ctrl_x_mode_path_patterns()
                && !ctrl_x_mode_path_defines()))
    {
        insexp().compl_cont_status &= !CONT_N_ADDS;
    }

    if compl_opt(insexp().compl_curr_idx).is_some_and(|curr| (curr.cp_flags & CP_CONT_S_IPOS) != 0)
    {
        insexp().compl_cont_status |= CONT_S_IPOS;
    } else {
        insexp().compl_cont_status &= !CONT_S_IPOS;
    }

    if !shortmess(K_SHM_COMPLETIONMENU) && !insexp().compl_autocomplete {
        ins_compl_show_statusmsg();
    }

    if !shortmess(K_SHM_COMPLETIONSCAN) && !insexp().compl_autocomplete {
        msg_progress(
            None,
            c"nvim.completion",
            c"success",
            -1,
            0,
            false,
            false,
            false,
        );
    }

    if enable_pum && !insexp().compl_interrupted {
        show_pum(save_w_wrow, save_w_leftcol);
    }
    insexp().compl_was_interrupted = insexp().compl_interrupted;
    insexp().compl_interrupted = false;

    true
}

pub fn ins_compl_enable_autocomplete() {
    insexp().compl_autocomplete = true;
    insexp().compl_get_longest = false;
}

pub fn ins_compl_disable_autocomplete() {
    insexp().compl_autocomplete = false;
}

pub fn ins_compl_arm_autostart() {
    insexp().compl_autostart_pending = true;
}

pub fn ins_compl_disarm_autostart() {
    insexp().compl_autostart_pending = false;
}

pub fn ins_compl_arm_autocomplete_delay() -> bool {
    if opt_vars().p_acl > 0 {
        insexp().compl_autocomplete_start_tv = os_hrtime();
        insexp().compl_autocomplete_pending = true;
        return true;
    }
    false
}

pub fn ins_compl_clear_autocomplete_delay() {
    insexp().compl_autocomplete_pending = false;
}

pub fn ins_compl_autocomplete_pending() -> bool {
    insexp().compl_autocomplete_pending
}

pub fn ins_compl_autocomplete_elapsed() -> i64 {
    ((os_hrtime() - insexp().compl_autocomplete_start_tv) / 1_000_000) as i64
}

fn show_pum(prev_w_wrow: i32, prev_w_leftcol: i32) {
    let n = g_st().redrawing_disabled;
    g_st().redrawing_disabled = 0;

    setcursor();
    if prev_w_wrow != curwin_mut().w_wrow || prev_w_leftcol != curwin_mut().w_leftcol {
        ins_compl_del_pum();
    }

    ins_compl_show_pum();
    setcursor();
    g_st().redrawing_disabled = n;
}

fn quote_meta(mut dest: Option<&mut [u8]>, src: &[u8]) -> usize {
    let mut m = src.len() + 1;
    let mut out_idx = 0usize;
    let mut idx = 0usize;

    while idx < src.len() {
        let ch = src[idx];
        let quote = match ch {
            b'.' | b'*' | b'[' => {
                if ctrl_x_mode_dictionary() || ctrl_x_mode_thesaurus() {
                    false
                } else {
                    opt_vars().p_magic
                }
            }
            b'~' => opt_vars().p_magic,
            b'\\' => !(ctrl_x_mode_dictionary() || ctrl_x_mode_thesaurus()),
            b'^' | b'$' => true,
            _ => false,
        };
        if quote {
            m += 1;
            if let Some(out) = dest.as_deref_mut() {
                out[out_idx] = b'\\';
                out_idx += 1;
            }
        }
        let step = crate::mbyte::utfc_slice2len(&src[idx..])
            .max(1)
            .min(src.len() - idx);
        if let Some(out) = dest.as_deref_mut() {
            out[out_idx..out_idx + step].copy_from_slice(&src[idx..idx + step]);
            out_idx += step;
        }
        idx += step;
    }
    if let Some(out) = dest {
        out[out_idx] = 0;
    }

    m
}

pub fn free_insexpand_stuff() {
    api_clear_string(&mut insexp().compl_orig_text);
    insexp().compl_orig_extmarks.clear();
    callback_free(&mut opt_vars().p_cfu);
    callback_free(&mut opt_vars().p_ofu);
    callback_free(&mut opt_vars().p_tsrfu);
    let global_cb = &mut insexp().cpt_cb;
    clear_cpt_callbacks(global_cb);
}

fn spell_back_to_badword() {
    let tpos = curwin_mut().w_cursor;
    insexp().spell_bad_len = spell_move_to(curwin_mut(), BACKWARD, SMT_ALL, true, None);
    if curwin_mut().w_cursor.col != tpos.col {
        start_arrow(Some(tpos), true, NUL as i32);
    }
}

fn cpt_sources_clear() {
    insexp().cpt_sources_array = Vec::new();
    insexp().cpt_sources_index = -1;
}

fn setup_cpt_sources() {
    cpt_sources_clear();

    let count = get_cpt_sources_count(curbuf_mut().b_p_cpt.as_cstr());
    if count == 0 {
        return;
    }

    let mut sources = vec![
        CptSource {
            cs_startcol: 0,
            cs_max_matches: 0,
            compl_start_tv: 0,
            cs_flag: 0,
            cs_refresh_always: false,
        };
        count as usize
    ];

    let mut buf: [u8; LSIZE] = [0; LSIZE];
    let mut idx: usize = 0;
    let mut p = curbuf_mut().b_p_cpt.as_bytes();
    while !p.is_empty() {
        while matches!(p.first(), Some(b',' | b' ')) {
            p = &p[1..];
        }
        if !p.is_empty() {
            sources[idx].cs_flag = p[0];
            buf.fill(0);
            let slen = copy_option_part(&mut p, &mut buf, c",");
            if slen > 0
                && let Some(caret_cstr) =
                    vim_strchr(crate::memory::bytes_as_cstr(&buf), b'^' as i32)
            {
                let after_caret = crate::memory::CStrSubslice::advance(caret_cstr, 1);
                sources[idx].cs_max_matches =
                    crate::charset::parse_i64_prefix(after_caret.to_bytes()) as i32;
            }
            idx += 1;
        }
    }

    insexp().cpt_sources_array = sources;
}

fn is_cpt_func_refresh_always() -> bool {
    let cpt_sources = &insexp().cpt_sources_array;
    cpt_sources.iter().any(|s| s.cs_refresh_always)
}

fn ins_compl_make_linear() {
    insexp().compl_is_cyclic = false;
}

fn remove_old_matches() {
    let list = compl_list_mut();
    if list.is_empty() || insexp().cpt_sources_index < 0 {
        return;
    }
    let forward = list[0].cp_cpt_source_idx < 0;
    insexp().compl_direction = if forward { FORWARD } else { BACKWARD };
    insexp().compl_shows_dir = insexp().compl_direction;

    let shown_ptr = insexp().compl_shown_idx.map(|i| list[i].cp_str.data);
    let preselect_ptr = insexp().compl_preselect_idx.map(|i| list[i].cp_str.data);
    let old_ptr = insexp().compl_old_idx.map(|i| list[i].cp_str.data);

    let mut i = 0;
    while i < list.len() {
        if list[i].cp_cpt_source_idx == insexp().cpt_sources_index {
            list.remove(i);
        } else {
            i += 1;
        }
    }

    insexp().compl_preselect_idx =
        preselect_ptr.and_then(|p| list.iter().position(|m| m.cp_str.data == p));
    insexp().compl_old_idx = old_ptr.and_then(|p| list.iter().position(|m| m.cp_str.data == p));

    if let Some(sp) = shown_ptr
        && let Some(new_shown) = list.iter().position(|m| m.cp_str.data == sp)
    {
        insexp().compl_shown_idx = Some(new_shown);
    } else if !list.is_empty() {
        insexp().compl_shown_idx = Some(if forward { 0 } else { list.len() - 1 });
    } else {
        insexp().compl_shown_idx = None;
    }

    insexp().compl_curr_idx = if list.is_empty() { None } else { Some(0) };
    for idx in 0..list.len() {
        let cond = if forward {
            list[idx].cp_cpt_source_idx < insexp().cpt_sources_index
        } else {
            list[idx].cp_cpt_source_idx > insexp().cpt_sources_index
        };
        if cond {
            insexp().compl_curr_idx = if forward {
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

fn get_cpt_func_completion_matches(cb: &mut Callback) {
    let cpt_src = &mut insexp().cpt_sources_array[insexp().cpt_sources_index as usize];
    let startcol = cpt_src.cs_startcol;

    if startcol == -2 || startcol == -3 {
        return;
    }

    set_compl_globals(startcol, curwin_mut().w_cursor.col, true);

    if !cpt_src.cs_refresh_always {
        ins_compl_insert_bytes(ins_compl_leader_slice(), -1);
    }

    expand_by_function(0, insexp().cpt_compl_pattern.as_opt_cstr(), Some(cb));

    if !cpt_src.cs_refresh_always {
        ins_compl_delete(false);
    }

    cpt_src.cs_refresh_always = insexp().compl_opt_refresh_always;
    insexp().compl_opt_refresh_always = false;
}

fn cpt_compl_refresh() {
    ins_compl_make_linear();
    let mut cpt_vec: Vec<u8> = curbuf_mut().b_p_cpt.to_owned().into_bytes_with_nul();
    strip_caret_numbers_in_place(&mut cpt_vec);
    let mut p_cstr = crate::memory::bytes_as_cstr(&cpt_vec);

    insexp().cpt_sources_index = 0;
    while !p_cstr.to_bytes().is_empty() {
        while matches!(p_cstr.to_bytes().first(), Some(&b',' | &b' ')) {
            p_cstr = crate::memory::CStrSubslice::advance(p_cstr, 1);
        }
        if p_cstr.to_bytes().is_empty() {
            break;
        }

        if insexp().cpt_sources_array[insexp().cpt_sources_index as usize].cs_refresh_always {
            let src = get_callback_if_cpt_func(p_cstr, insexp().cpt_sources_index);
            if src != CptFuncSource::None {
                remove_old_matches();
                let mut startcol: i32 = 0;
                let ret = get_userdefined_compl_info(
                    curwin_mut().w_cursor.col,
                    src.as_mut(),
                    Some(&mut startcol),
                );
                if !ret {
                    if startcol == -3 {
                        insexp().cpt_sources_array[insexp().cpt_sources_index as usize]
                            .cs_refresh_always = false;
                    } else {
                        startcol = -2;
                    }
                } else if startcol < 0 || startcol > curwin_mut().w_cursor.col {
                    startcol = curwin_mut().w_cursor.col;
                }
                insexp().cpt_sources_array[insexp().cpt_sources_index as usize].cs_startcol =
                    startcol;
                if ret && let Some(cb) = src.as_mut() {
                    compl_source_start_timer(insexp().cpt_sources_index);
                    get_cpt_func_completion_matches(cb);
                }
            }
        }

        let mut p_bytes = p_cstr.to_bytes();
        let old_len = p_bytes.len();
        let _ = copy_option_part(&mut p_bytes, g_st().iobuff_bytes_mut(), c",");
        p_cstr = crate::memory::CStrSubslice::advance(p_cstr, old_len - p_bytes.len());
        if may_advance_cpt_index(p_cstr) {
            let _ = advance_cpt_sources_index_safe();
        }
    }
    insexp().cpt_sources_index = -1;

    insexp().compl_matches = ins_compl_make_cyclic();
}

pub fn f_preinserted(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if ins_compl_preinsert_effect() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_caret_numbers_in_place_strips_max_matches_suffixes() {
        let mut buf: Vec<u8> = b".,w^5,b,u^10,Ffunc^3,k\0".to_vec();
        {
            strip_caret_numbers_in_place(&mut buf);
            let s = crate::memory::bytes_as_cstr(&buf).to_str().unwrap();
            assert_eq!(s, ".,w,b,u,Ffunc,k");
        }
    }
}
