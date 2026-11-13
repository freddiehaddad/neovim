#![forbid(unsafe_code)]

//! Functions for entering and editing an Ex command line (`src/nvim/ex_getln.c`).

use crate::api::extmark::nvim_create_namespace;
use crate::api::private::helpers::{
    HLF_E, TryState, add_c, api_clear_error, api_free_array, array_obj, cstr_as_obj,
    cstr_as_string, cstrn_as_string, error_init, error_set, integer_obj, string_obj, try_enter,
    try_leave,
};
use crate::api::private::validate::{Array, NvimString};
use crate::api::vim::nvim_create_buf;
use crate::autocmd::{
    AuEvent, DOCMD_NOWAIT, EVENT_CMDLINECHANGED, EVENT_CMDLINEENTER, EVENT_CMDLINELEAVE,
    EVENT_CMDLINELEAVEPRE, EVENT_CURSORMOVEDC, apply_autocmds, block_autocmds, has_event,
    unblock_autocmds,
};
use crate::buffer::bt_cmdwin;
use crate::buffer::{
    DOBUF_FIRST, DOBUF_GOTO, FileBuf, K_UIMESSAGES, MAXLNUM, buf_clear, buf_valid, buflist_findnr,
    do_buffer,
};
use crate::charset::{
    ascii_isalpha, ascii_isspace, ascii_iswhite, is_special, skipwhite_slice, vim_is_idc,
    vim_isprintc, vim_iswordc, vim_slice2nr,
};
use crate::cmdexpand::{
    CTRL_N, CTRL_P, CmdlineColorChunk, CmdlineInfo, ColoredCmdline, EXPAND_NOTHING,
    EXPAND_UNSUCCESSFUL, K_OPT_BO_FLAG_WILDMODE, K_UICMDLINE, SEARCH_NOOF, SEARCH_OPT, SEARCH_PEEK,
    SEARCH_START, WILD_ALL, WILD_APPLY, WILD_BUFLASTUSED, WILD_CANCEL, WILD_EXPAND_KEEP, WILD_FREE,
    WILD_FUNC_TRIGGER, WILD_LONGEST, WILD_MAY_EXPAND_PATTERN, WILD_NEXT, WILD_NO_BEEP,
    WILD_NOINSERT, WILD_NOSELECT, WILD_PAGEDOWN, WILD_PAGEUP, WILD_PREV, WILD_PUM_WANT, XP_BS_NONE,
    clear_cmdline_orig, cmdline_pum_active, cmdline_pum_cleanup, cmdline_pum_remove,
    expand_cleanup, expand_init, expand_one, nextwild, set_expand_context, showmatches,
    wildmenu_cleanup, wildmenu_process_key, wildmenu_translate_key,
};
use crate::cmdhist::{
    HIST_INVALID, HIST_SEARCH, add_to_history, get_hisidx, get_hislen, hist_char2type,
    init_history, with_histentry,
};
use crate::context::{CtxSwitch, K_CTX_KEEP_DIRS, ctx_restore, ctx_switch};
use crate::cursor::{CursorPos, clearpos, coladvance, equalpos, gchar_cursor, lt};
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST, do_digraph, get_digraph};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, redraw_all_later, redraw_custom_title_later,
    redraw_later, redraw_statuslines, set_must_redraw, status_redraw_all, status_redraw_curbuf,
    update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Callback, EvalFuncData, K_CALLBACK_NONE_CONST, TypVal, TypvalVval, VarType, Varnumber,
    callback_free, tv_check_for_opt_number_arg, tv_check_for_string_arg, tv_clear, tv_copy,
    tv_dict_add_bool, tv_dict_add_nr, tv_dict_add_str, tv_dict_find, tv_dict_get_callback,
    tv_dict_get_number, tv_dict_get_string_buf_chk, tv_dict_set_keys_readonly, tv_get_number,
    tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_get_string_chk, tv_list_free,
    tv_list_len,
};
use crate::eval::vars::{get_globvar_dict, heredoc_get, set_vim_var_char};
use crate::eval::window::Viewstate;
use crate::eval::{callback_call, eval_has_provider, get_echo_hl_id, get_v_event, restore_v_event};
use crate::event::r#loop::loop_schedule_deferred;
use crate::event::multiqueue::Event;
use crate::ex_cmds::{B_IMODE_LMAP, CTRL_C_CONST, CTRL_E, CTRL_Y, K_OPT_INCCOMMAND, rename_buffer};
use crate::ex_docmd::{
    ADDR_LINES, CMOD_NOSWAPFILE, Cmdmod, EX_PREVIEW, EX_RANGE, Exarg, do_cmdline, execute_cmd,
    expr_map_locked, parse_cmd_address, parse_cmdline, parse_command_modifiers, set_no_hlsearch,
    skip_range, undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::extmark::extmark_clear;
use crate::globals::*;
use crate::highlight_group::{syn_id2attr, syn_name2id};
use crate::input::{
    beep_flush, char_avail, ins_typebuf, map_execute_lua, plain_vgetc, stuff_empty,
    stuff_readbuff_special, stuffchar_readbuff, vgetc, vpeekc, vpeekc_any, vungetc,
};
use crate::input_cmdatom::{atom_cmdline_set, atom_payload_end, atom_payload_start};
use crate::insert::get_literal;
use crate::keycodes::NUL;
use crate::keycodes::{
    CAR, ESC, K_BS, K_C_END, K_C_HOME, K_C_LEFT, K_C_RIGHT, K_COMMAND, K_DEL, K_DOWN, K_END,
    K_HOME, K_IGNORE, K_INS, K_KDEL, K_KEND, K_KENTER, K_KHOME, K_KINS, K_KPAGEDOWN, K_KPAGEUP,
    K_LEFT, K_LEFTDRAG, K_LEFTMOUSE, K_LEFTRELEASE, K_MIDDLEDRAG, K_MIDDLEMOUSE, K_MIDDLERELEASE,
    K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP, K_NOP, K_PAGEDOWN, K_PAGEUP,
    K_RIGHT, K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_S_DOWN, K_S_END, K_S_HOME, K_S_LEFT,
    K_S_RIGHT, K_S_TAB, K_S_UP, K_SPECIAL, K_UP, K_X1DRAG, K_X1MOUSE, K_X1RELEASE, K_X2DRAG,
    K_X2MOUSE, K_X2RELEASE, K_ZERO, KE_EVENT, KE_FILLER, KE_LUA, KE_WILD, KS_EXTRA, MOD_MASK_CTRL,
    MOD_MASK_SHIFT, NL, get_special_key_name, termcap2key,
};
use crate::lua::executor::nlua_call_typval;
use crate::mapping::{REMAP_NONE, check_abbr, map_to_exists_mode};
use crate::mark::setpcmark;
use crate::mbyte::{
    UTF8LEN_TAB_ZERO, mb_cptr2char_adv, mb_get_class, mb_tolower, ptr2cells, utf_char2bytes,
    utf_char2len, utf_iscomposing_first, utf_ptr2cells, utfc_ptr2len,
};
use crate::memline::{decl, incl};
use crate::message::{
    emsg, msg, msg_check, msg_clr_eos, msg_cursor_goto, msg_grid_validate, msg_outtrans_len,
    msg_putchar, msg_puts_hl, msg_puts_len, msg_start, msg_starthere, sb_text_end_cmdline,
    sb_text_restart_cmdline, sb_text_start_cmdline, smsg,
};
use crate::mouse::{MSCR_DOWN, MSCR_LEFT, MSCR_RIGHT, MSCR_UP, cmdline_mousescroll, setmouse};
use crate::r#move::{
    changed_cline_bef_curs, restore_viewstate, save_viewstate, update_topline, validate_cursor,
};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
#[cfg(unix)]
use crate::option::csh_like_shell;
use crate::option::{
    OptInt, set_iminsert_global, set_imsearch_global, set_option_direct, string_to_key,
};
use crate::option_vars::*;
use crate::optionstr::Optset;
use crate::os::env::{ExpandCtx, home_replace_save};
use crate::os::input::line_breakcheck;
use crate::path::vim_ispathsep;
use crate::popupmenu::pum_check_clear;
use crate::profile::{Proftime, profile_setlimit};
use crate::regexp::{
    CTRL_H_CONST, CTRL_V_CONST, MAGIC_ALL, MAGIC_ON, MAXCOL, NUMBUFLEN, RegMagic, skip_regexp_magic,
};
use crate::register::{
    cmdline_paste_reg, get_expr_line, get_expr_register, get_spec_reg, is_literal_register,
    valid_yank_reg,
};
use crate::runtime::SID_NONE;
use crate::search::{
    BACKWARD, FORWARD, SearchOffset, SearchitArg, do_search, parse_search_pattern_offset,
    pat_has_uppercase, restore_last_search_pattern, restore_search_patterns,
    save_last_search_pattern, save_search_patterns, searchit,
};
use crate::state::{
    EditorState, MODE_CMDLINE, MODE_LANGMAP, SaveVEvent, may_trigger_modechanged,
    may_trigger_safestate, state_enter, state_handle_k_event,
};
use crate::strings::{gettext, vim_strsave_escaped};
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_call_cmdline_block_append, ui_call_cmdline_block_hide,
    ui_call_cmdline_block_show, ui_call_cmdline_hide, ui_call_cmdline_pos, ui_call_cmdline_show,
    ui_call_cmdline_special_char, ui_cursor_shape, ui_flush, ui_has, vim_beep,
};
use crate::undo::{UndoCheckpoint, u_checkpoint, u_rollback, u_sync};
use crate::usercmd::{cmdcomplete_type_to_str, parse_compl_arg};
use crate::viml::parser::expressions::{K_EXPR_FLAGS_DISALLOW_EOC, viml_pexpr_parse};
use crate::viml::parser::parser::{
    ParserHighlight, ParserInputReader, ParserLine, ParserPosition, ParserState,
};
use crate::window::{
    WSP_BOT, close_windows, global_stl_height, lastwin_nofloating,
    may_trigger_win_scrolled_resized, win_enter, win_size_restore, win_size_save, win_split,
};
use std::ffi::CStr;

#[cfg(windows)]
use crate::charset::vim_isfilec;

pub type Hlf = i32;
pub type Disptick = u64;
use crate::eval::funcs::{RE_SEARCH, SEARCH_COL, SEARCH_KEEP};

pub(crate) const CTRL_A: i32 = 1;
pub(crate) const CTRL_B_CONST: i32 = 2;
pub(crate) const CTRL_D_CONST: i32 = 4;
pub(crate) const CTRL_F_CONST: i32 = 6;
pub(crate) const CTRL_G: i32 = 7;
pub(crate) const CTRL_K_CONST: i32 = 11;
pub(crate) const CTRL_L_CONST: i32 = 12;
pub(crate) const CTRL_O: i32 = 15;
pub(crate) const CTRL_Q_CONST: i32 = 17;
pub(crate) const CTRL_R_CONST: i32 = 18;
pub(crate) const CTRL_T_CONST: i32 = 20;
pub(crate) const CTRL_U: i32 = 21;
pub(crate) const CTRL_W_CONST: i32 = 23;
pub(crate) const CTRL_Z_CONST: i32 = 26;
pub(crate) const CTRL_BSL: i32 = 28;
pub(crate) const CTRL_RSB_CONST: i32 = 29;
pub(crate) const CTRL_HAT_CONST: i32 = 30;
pub(crate) const CTRL_CONST_CONST: i32 = 31;

pub(crate) const KS_SELECT: i32 = 245;
pub(crate) const K_SELECT: i32 = termcap2key(KS_SELECT, KE_FILLER as i32);
pub(crate) const K_EVENT: i32 = termcap2key(KS_EXTRA as i32, KE_EVENT as i32);
pub(crate) const K_LUA: i32 = termcap2key(KS_EXTRA as i32, KE_LUA as i32);
const K_WILD: i32 = termcap2key(KS_EXTRA as i32, KE_WILD as i32);

pub(crate) const ABBR_OFF: i32 = 0x100;

pub(crate) const B_IMODE_USE_INSERT: OptInt = -1;
pub(crate) const B_IMODE_NONE: OptInt = 0;

const K_OPT_WIM_FLAG_FULL: u8 = 0x01;
const K_OPT_WIM_FLAG_LONGEST: u8 = 0x02;
const K_OPT_WIM_FLAG_LIST: u8 = 0x04;
const K_OPT_WIM_FLAG_LASTUSED: u8 = 0x08;
pub(crate) const K_OPT_WIM_FLAG_NOSELECT: u8 = 0x10;
pub(crate) const K_OPT_WIM_FLAG_NOINSERT: u8 = 0x20;

pub(crate) const K_OPT_BO_FLAG_ERROR: u32 = 0x40;

const K_CPO_ESC: i32 = b'x' as i32;

const K_CMD_REDRAW_NONE: i32 = 0;
const K_CMD_REDRAW_POS: i32 = 1;
const K_CMD_REDRAW_ALL: i32 = 2;

pub const VSE_NONE: i32 = 0;
#[cfg(unix)]
pub const VSE_SHELL: i32 = 1;
pub const VSE_BUFFER: i32 = 2;

const CMDLINE_NOT_CHANGED: i32 = 1;
const CMDLINE_CHANGED: i32 = 2;
const GOTO_NORMAL_MODE: i32 = 3;
const PROCESS_NEXT_KEY: i32 = 4;

const MAX_CB_ERRORS: i32 = 1;

#[cfg(windows)]
const PATH_ESC_CHARS: &CStr = c" \t\n*?[{`%#'\"|!<";
#[cfg(windows)]
const BUFFER_ESC_CHARS: &CStr = c" \t\n*?[`'\"|!<";

#[cfg(not(windows))]
const PATH_ESC_CHARS: &CStr = c" \t\n*?[{`$\\%#'\"|!<";
#[cfg(not(windows))]
const SHELL_ESC_CHARS: &CStr = c" \t\n*?[{`$\\%#'\"|!<>();&";
#[cfg(not(windows))]
const BUFFER_ESC_CHARS: &CStr = c" \t\n*?[`$\\%#'\"|!<";

/// Struct to store the state of `'incsearch'` highlighting.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IncsearchState {
    search_start: CursorPos,
    save_cursor: CursorPos,
    winid: ObjHandle,
    init_viewstate: Viewstate,
    old_viewstate: Viewstate,
    match_start: CursorPos,
    match_end: CursorPos,
    did_incsearch: bool,
    incsearch_postponed: bool,
}

impl IncsearchState {
    const fn new() -> Self {
        Self {
            search_start: CursorPos::new(),
            save_cursor: CursorPos::new(),
            winid: 0,
            init_viewstate: Viewstate::new(),
            old_viewstate: Viewstate::new(),
            match_start: CursorPos::new(),
            match_end: CursorPos::new(),
            did_incsearch: false,
            incsearch_postponed: false,
        }
    }
}

#[repr(C)]
struct CommandLineState {
    firstc: i32,
    count: i32,
    indent: i32,
    c: i32,
    gotesc: bool,
    do_abbr: bool,
    lookfor: Option<std::ffi::CString>,
    lookforlen: i32,
    hiscnt: i32,
    save_hiscnt: i32,
    histype: i32,
    is_state: IncsearchState,
    did_wild_list: bool,
    wim_index: i32,
    save_msg_scroll: i32,
    save_state: i32,
    prev_cmdpos: i32,
    prev_cmdbuff: Option<std::ffi::CString>,
    save_p_icm: Option<std::ffi::CString>,
    skip_pum_redraw: bool,
    some_key_typed: bool,
    ignore_drag_release: bool,
    break_ctrl_c: bool,
    b_im_use_insert: Option<bool>,
    b_im_ptr_buf: crate::globals::BufPtr,
    cmdline_type: i32,
    event_cmdlineleavepre_triggered: bool,
    did_hist_navigate: bool,
}

impl CommandLineState {
    const fn new() -> Self {
        Self {
            firstc: 0,
            count: 0,
            indent: 0,
            c: 0,
            gotesc: false,
            do_abbr: false,
            lookfor: None,
            lookforlen: 0,
            hiscnt: 0,
            save_hiscnt: 0,
            histype: 0,
            is_state: IncsearchState::new(),
            did_wild_list: false,
            wim_index: 0,
            save_msg_scroll: 0,
            save_state: 0,
            prev_cmdpos: 0,
            prev_cmdbuff: None,
            save_p_icm: None,
            skip_pum_redraw: false,
            some_key_typed: false,
            ignore_drag_release: false,
            break_ctrl_c: false,
            b_im_use_insert: None,
            b_im_ptr_buf: crate::globals::BufPtr::NULL,
            cmdline_type: 0,
            event_cmdlineleavepre_triggered: false,
            did_hist_navigate: false,
        }
    }
}

#[inline]
fn with_ccline_xpc<R>(f: impl FnOnce(&mut ExpandCtx) -> R) -> R {
    let ccline = get_cmdline_info();
    let mut xpc_borrow = ccline.xpc.borrow_mut();
    let xpc = xpc_borrow.get_or_insert_with(ExpandCtx::new);
    f(xpc)
}

impl EditorState for CommandLineState {
    fn check(&mut self) -> i32 {
        command_line_check(self)
    }

    fn execute(&mut self, key: i32) -> i32 {
        command_line_execute(self, key)
    }
}

impl Default for CommandLineState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct CpBufInfo {
    buf: crate::globals::BufPtr,
    save_b_p_ma: bool,
    save_b_changed: i32,
    save_b_op_start: CursorPos,
    save_b_op_end: CursorPos,
    undo_checkpoint: UndoCheckpoint,
}

impl CpBufInfo {
    const fn new() -> Self {
        Self {
            buf: crate::globals::BufPtr::NULL,
            save_b_p_ma: false,
            save_b_changed: 0,
            save_b_op_start: CursorPos::new(),
            save_b_op_end: CursorPos::new(),
            undo_checkpoint: UndoCheckpoint::new(),
        }
    }
}

impl Default for CpBufInfo {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
struct CpWinInfo {
    win: crate::globals::WinPtr,
    save_w_cursor: CursorPos,
    save_viewstate: Viewstate,
    save_w_p_cul: bool,
    save_w_p_cuc: bool,
}

impl CpWinInfo {
    const fn new() -> Self {
        Self {
            win: crate::globals::WinPtr::NULL,
            save_w_cursor: CursorPos::new(),
            save_viewstate: Viewstate::new(),
            save_w_p_cul: false,
            save_w_p_cuc: false,
        }
    }
}

impl Default for CpWinInfo {
    fn default() -> Self {
        Self::new()
    }
}

struct CpInfo {
    win_info: Vec<CpWinInfo>,
    buf_info: Vec<CpBufInfo>,
    save_hls: bool,
    save_cmdmod: Cmdmod,
    save_view: Vec<i32>,
}

struct ExGetlnState {
    last_prompt_id: std::cell::Cell<u32>,
    cmdline_level: std::cell::Cell<i32>,
    new_cmdpos: std::cell::Cell<i32>,
    cmdline_block: crate::api::private::validate::CloneCell<Array>,
    getln_interrupted_highlight: std::cell::Cell<bool>,
    cedit_key: std::cell::Cell<i32>,
    cmdpreview_bufnr: std::cell::Cell<ObjHandle>,
    cmdpreview_ns: std::cell::Cell<i32>,
    prev_prompt_id: std::cell::Cell<u32>,
    prev_prompt_errors: std::cell::Cell<i32>,
}

impl ExGetlnState {
    const fn new() -> Self {
        Self {
            last_prompt_id: std::cell::Cell::new(0),
            cmdline_level: std::cell::Cell::new(0),
            new_cmdpos: std::cell::Cell::new(0),
            cmdline_block: crate::api::private::validate::CloneCell::new(Array::new()),
            getln_interrupted_highlight: std::cell::Cell::new(false),
            cedit_key: std::cell::Cell::new(-1),
            cmdpreview_bufnr: std::cell::Cell::new(0),
            cmdpreview_ns: std::cell::Cell::new(0),
            prev_prompt_id: std::cell::Cell::new(u32::MAX),
            prev_prompt_errors: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static GETLN_STATE: ExGetlnState = const { ExGetlnState::new() };
    static CCLINE: std::cell::RefCell<std::rc::Rc<CmdlineInfo>> = std::cell::RefCell::new(
        std::rc::Rc::new(CmdlineInfo::new())
    );
}

use crate::api::private::helpers::array_add;

fn trigger_cmd_autocmd(typechar: i32, evt: AuEvent) {
    let typestr: [u8; 2] = [typechar as u8, 0];
    let typestr_cstr = crate::memory::bytes_as_cstr(&typestr);
    apply_autocmds(
        evt,
        Some(typestr_cstr),
        Some(typestr_cstr),
        false,
        curbuf_opt().as_deref(),
    );
}

fn init_incsearch_state(s: &mut IncsearchState) {
    s.winid = curwin().handle;
    s.match_start = curwin().w_cursor.get();
    s.did_incsearch = false;
    s.incsearch_postponed = false;
    clearpos(&mut s.match_end);
    s.save_cursor = curwin().w_cursor.get();
    s.search_start = curwin().w_cursor.get();
    save_viewstate(&curwin(), &mut s.init_viewstate);
    save_viewstate(&curwin(), &mut s.old_viewstate);
}

fn set_search_match(t: &mut CursorPos) {
    t.lnum += g_st().search_global.match_lines.get();
    t.col = g_st().search_global.match_endcol.get();
    if t.lnum > curbuf().b_ml.ml_line_count.get() {
        t.lnum = curbuf().b_ml.ml_line_count.get();
        curwin().w_cursor.set(*t);
        coladvance(&curwin(), MAXCOL);
        *t = curwin().w_cursor.get();
    }
}

/// Parses the `:[range]s/foo` like commands and returns details needed for
/// incsearch and wildmenu completion.
/// Returns true if pattern is valid.
/// Sets `skiplen`, `patlen`, `Search.first_line`, and `Search.last_line` and `magic` if overruled.
pub fn parse_pattern_and_range(
    incsearch_start: &mut CursorPos,
    search_delim: &mut i32,
    skiplen: &mut i32,
    patlen: &mut i32,
    is_magic: &mut bool,
) -> bool {
    let mut delim_optional = false;
    let mut dummy = None;
    let mut magic: RegMagic = 0;

    *skiplen = 0;
    *patlen = get_cmdline_info().cmdlen.get();

    // Default range
    g_st().search_global.first_line.set(0);
    g_st().search_global.last_line.set(MAXLNUM);

    let mut ea: Exarg = Exarg::new();
    ea.line1 = 1;
    ea.line2 = 1;
    ea.cmd = get_cmdline_info().cmdbuff();
    ea.addr_type = ADDR_LINES;

    let mut dummy_cmdmod: Cmdmod = Cmdmod::new();
    // Skip over command modifiers
    parse_command_modifiers(&mut ea, &mut dummy, &mut dummy_cmdmod, true);

    // Skip over the range to find the command.
    let cmd = ea.cmd.advance_to_cstr(skip_range(ea.cmd.as_cstr(), None));
    let cmd_bytes = cmd.as_bytes();
    let Some(&first_b) = cmd_bytes.first() else {
        return false;
    };
    if !matches!(first_b, b's' | b'g' | b'v' | b'l' | b'u') {
        return false;
    }

    // Skip over command name to find pattern separator
    let cmd_len = cmd_bytes
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    let mut p = cmd.wrapping_add(cmd_len);
    if p.skipwhite().is_empty() {
        return false;
    }

    let cmd_name = &cmd_bytes[..cmd_len];
    if b"substitute".starts_with(cmd_name)
        || b"smagic".starts_with(cmd_name)
        || (cmd_len >= 3 && b"snomagic".starts_with(cmd_name))
        || b"vglobal".starts_with(cmd_name)
    {
        if cmd_name.starts_with(b"sm") {
            *is_magic = true;
        } else if cmd_name.starts_with(b"sn") {
            *is_magic = false;
        }
    } else if cmd_len >= 3 && (b"sort".starts_with(cmd_name) || b"uniq".starts_with(cmd_name)) {
        // skip over ! and flags
        if p.first_byte() == b'!' {
            p = p.wrapping_add(1).skipwhite();
        }
        loop {
            p = p.skipwhite();
            if !ascii_isalpha(p.first_byte()) {
                break;
            }
            p = p.wrapping_add(1);
        }
        if p.is_empty() {
            return false;
        }
    } else if (cmd_len >= 3 && b"vimgrep".starts_with(cmd_name))
        || (cmd_len >= 8 && b"vimgrepadd".starts_with(cmd_name))
        || (cmd_len >= 2 && b"lvimgrep".starts_with(cmd_name))
        || (cmd_len >= 9 && b"lvimgrepadd".starts_with(cmd_name))
        || b"global".starts_with(cmd_name)
    {
        // skip optional "!"
        if p.first_byte() == b'!' {
            p = p.wrapping_add(1);
            if p.skipwhite().is_empty() {
                return false;
            }
        }
        if first_b != b'g' {
            delim_optional = true;
        }
    } else {
        return false;
    }

    p = p.skipwhite();
    let delim = if delim_optional && vim_is_idc(i32::from(p.first_byte())) {
        b' ' as i32
    } else {
        let d = i32::from(p.first_byte());
        p = p.wrapping_add(1);
        d
    };
    *search_delim = delim;

    let end_off = skip_regexp_magic(p.as_bytes(), delim, i32::from(*is_magic), Some(&mut magic));
    let end = p.wrapping_add(end_off);
    let use_last_pat = end.addr() == p.addr() && i32::from(end.first_byte()) == delim;

    if end.addr() == p.addr() && !use_last_pat {
        return false;
    }

    // Skip if the pattern matches everything (e.g., for 'hlsearch')
    if !use_last_pat {
        let pat_len = end.offset_from(&p);
        if empty_pattern_magic(p.slice(pat_len), magic) {
            return false;
        }
    }

    // Found a non-empty pattern or //
    *skiplen = p.offset_from(get_cmdline_info().cmdbuff()) as i32;
    *patlen = end.offset_from(&p) as i32;

    // Parse the address range
    let save_cursor = curwin().w_cursor.get();
    curwin().w_cursor.set(*incsearch_start);

    parse_cmd_address(&mut ea, &mut dummy, true);

    if ea.addr_count > 0 {
        // Allow for reverse match.
        g_st().search_global.first_line.set(ea.line2.min(ea.line1));
        g_st().search_global.last_line.set(ea.line2.max(ea.line1));
    } else if cmd.first_byte() == b's' && cmd_bytes.get(1) != Some(&b'o') {
        // :s defaults to the current line
        g_st()
            .search_global
            .first_line
            .set(curwin().w_cursor.get().lnum);
        g_st()
            .search_global
            .last_line
            .set(curwin().w_cursor.get().lnum);
    }

    curwin().w_cursor.set(save_cursor);
    true
}

/// Return true when `'incsearch'` highlighting is to be done.
/// Sets `Search.first_line` and `Search.last_line` to the address range.
/// May change the last search pattern.
fn do_incsearch_highlighting(
    firstc: i32,
    search_delim: &mut i32,
    is_state: &mut IncsearchState,
    skiplen: &mut i32,
    patlen: &mut i32,
    magic: &mut bool,
) -> bool {
    *skiplen = 0;
    *patlen = get_cmdline_info().cmdlen.get();

    if !opt_vars().p_is.get() || g_st().cmd_silent.get() {
        return false;
    }

    // By default search all lines
    g_st().search_global.first_line.set(0);
    g_st().search_global.last_line.set(MAXLNUM);

    if firstc == b'/' as i32 || firstc == b'?' as i32 {
        *search_delim = firstc;
        return true;
    }

    if firstc != b':' as i32 {
        return false;
    }

    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    let retval = parse_pattern_and_range(
        &mut is_state.search_start,
        &mut *search_delim,
        &mut *skiplen,
        &mut *patlen,
        &mut *magic,
    );
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));

    retval
}

/// May do `'incsearch'` highlighting if desired.
fn may_do_incsearch_highlighting(firstc: i32, count: i32, s: &mut IncsearchState) {
    let mut skiplen: i32 = 0;
    let mut patlen: i32 = 0;
    let mut search_delim: i32 = 0;
    let mut magic: bool = opt_vars().p_magic.get();

    // Parsing range may already set the last search pattern.
    // NOTE: must call restore_last_search_pattern() before returning!
    save_last_search_pattern();

    if !do_incsearch_highlighting(
        firstc,
        &mut search_delim,
        s,
        &mut skiplen,
        &mut patlen,
        &mut magic,
    ) {
        restore_last_search_pattern();
        finish_incsearch_highlighting(false, &mut *s, true);
        return;
    }

    // if there is a character waiting, search and redraw later
    if char_avail() {
        restore_last_search_pattern();
        s.incsearch_postponed = true;
        return;
    }
    s.incsearch_postponed = false;

    // Use the previous pattern for ":s//".
    let end_idx = (skiplen + patlen) as usize;
    let next_char = get_cmdline_info().cmdbuff_vec.borrow()[end_idx];
    let use_last_pat = patlen == 0
        && skiplen > 0
        && get_cmdline_info().cmdbuff_vec.borrow()[(skiplen - 1) as usize] == next_char;

    if patlen != 0 || use_last_pat {
        ui_busy_start();
        ui_flush();
    }

    if g_st().search_global.first_line.get() == 0 {
        // start at the original cursor position
        curwin().w_cursor.set(s.search_start);
    } else if g_st().search_global.first_line.get() > curbuf().b_ml.ml_line_count.get() {
        // start after the last line
        curwin()
            .w_cursor
            .set_lnum(curbuf().b_ml.ml_line_count.get());
        curwin().w_cursor.set_col(MAXCOL);
    } else {
        // start at the first line in the range
        curwin()
            .w_cursor
            .set_lnum(g_st().search_global.first_line.get());
        curwin().w_cursor.set_col(0);
    }

    let mut found: i32 = 0; // do_search() result

    if patlen != 0 || use_last_pat {
        let mut search_flags = SEARCH_OPT + SEARCH_NOOF + SEARCH_PEEK;
        if !opt_vars().p_hls.get() {
            search_flags += SEARCH_KEEP;
        }
        if g_st().search_global.first_line.get() != 0 {
            search_flags += SEARCH_START;
        }
        // Set the time limit to half a second.
        let tm: Proftime = profile_setlimit(500);
        let mut sia = SearchitArg {
            sa_stop_lnum: 0,
            sa_tm: Some(tm),
            sa_timed_out: 0,
            sa_wrapped: 0,
        };
        get_cmdline_info().cmdbuff_vec.borrow_mut()[end_idx] = 0;
        g_st().emsg_off.set(g_st().emsg_off.get() + (1)); // So it doesn't beep if bad expr
        found = do_search(
            None,
            if firstc == b':' as i32 {
                b'/' as i32
            } else {
                firstc
            },
            search_delim,
            (get_cmdline_info().cmdbuff().wrapping_add(skiplen as usize)).as_opt_cstr(),
            patlen as usize,
            count,
            search_flags,
            magic,
            Some(&mut sia),
        );
        g_st().emsg_off.set(g_st().emsg_off.get() - (1));
        get_cmdline_info().cmdbuff_vec.borrow_mut()[end_idx] = next_char;
        if curwin().w_cursor.get().lnum < g_st().search_global.first_line.get()
            || curwin().w_cursor.get().lnum > g_st().search_global.last_line.get()
        {
            // match outside of address range
            found = 0;
            curwin().w_cursor.set(s.search_start);
        }

        // if interrupted while searching, behave like it failed
        if g_st().got_int.get() {
            vpeekc(); // remove <C-C> from input stream
            g_st().got_int.set(false); // don't abandon the command line
            found = 0;
        } else if char_avail() {
            // cancelled searching because a char was typed
            s.incsearch_postponed = true;
        }
        ui_busy_stop();
    } else {
        set_no_hlsearch(true); // turn off previous highlight
        redraw_all_later(UPD_SOME_VALID);
    }

    g_st().search_global.hl_match.set(found != 0); // add or remove search match position

    // first restore the old curwin values, so the screen is
    // positioned in the same way as the actual search command
    restore_viewstate(&curwin(), &s.old_viewstate);
    changed_cline_bef_curs(&curwin());
    update_topline(&curwin());

    let mut end_pos = curwin().w_cursor.get();
    if found != 0 {
        s.match_start = curwin().w_cursor.get();
        let mut match_pos = curwin().w_cursor.get();
        set_search_match(&mut match_pos);
        curwin().w_cursor.set(match_pos);
        validate_cursor(&curwin());
        s.match_end = curwin().w_cursor.get();
        curwin().w_cursor.set(end_pos);
        end_pos = s.match_end;
    }

    // Disable 'hlsearch' highlighting if the pattern matches
    // everything. Avoids a flash when typing "foo\|".
    if !use_last_pat {
        let ccline_rc = get_cmdline_info();
        let cmd_vec = ccline_rc.cmdbuff_vec.borrow();
        let pat_slice = &cmd_vec[skiplen as usize..(skiplen + patlen) as usize];
        if empty_pattern(pat_slice, search_delim) && !g_st().search_global.no_hlsearch.get() {
            redraw_all_later(UPD_SOME_VALID);
            set_no_hlsearch(true);
        }
    }

    validate_cursor(&curwin());

    // May redraw the status line to show the cursor position.
    if opt_vars().p_ru.get() && (curwin().w_status_height.get() > 0 || global_stl_height() > 0) {
        curwin().w_redr_status.set(true);
    }

    redraw_later(Some(&crate::globals::curwin()), UPD_SOME_VALID);
    update_screen();
    g_st().search_global.hl_match.set(false);
    restore_last_search_pattern();

    // Leave it at the end to make CTRL-R CTRL-W work.  But not when beyond the
    // end of the pattern, e.g. for ":s/pat/".
    if !get_cmdline_info()
        .cmdbuff()
        .wrapping_add((skiplen + patlen) as usize)
        .is_empty()
    {
        curwin().w_cursor.set(s.search_start);
    } else if found != 0 {
        curwin().w_cursor.set(end_pos);
        curwin().w_valid_cursor.set(end_pos); // mark as valid for cmdline_show redraw
        crate::r#move::set_topline_valid(&curwin());
    }

    msg_starthere();
    redrawcmdline();
    s.did_incsearch = true;
    // Fire WinScrolled/WinResized last, after all state is finalized: the
    // autocmd may mutate curwin or get_cmdline_info() via arbitrary user code.
    may_trigger_win_scrolled_resized();
}

/// When CTRL-L typed: add character from the match to the pattern.
/// May set `*c` to the added character.
/// Return `OK` when calling `command_line_not_changed`.
fn may_add_char_to_search(firstc: i32, c: &mut i32, s: &mut IncsearchState) -> bool {
    let mut skiplen: i32 = 0;
    let mut patlen: i32 = 0;
    let mut search_delim: i32 = 0;
    let mut magic: bool = opt_vars().p_magic.get();

    // Parsing range may already set the last search pattern.
    // NOTE: must call restore_last_search_pattern() before returning!
    save_last_search_pattern();

    // Add a character from under the cursor for 'incsearch'
    if !do_incsearch_highlighting(
        firstc,
        &mut search_delim,
        s,
        &mut skiplen,
        &mut patlen,
        &mut magic,
    ) {
        restore_last_search_pattern();
        return false;
    }
    restore_last_search_pattern();

    if s.did_incsearch {
        curwin().w_cursor.set(s.match_end);
        *c = gchar_cursor();
        if *c != NUL as i32 {
            // If 'ignorecase' and 'smartcase' are set and the
            // command line has no uppercase characters, convert
            // the character to lowercase
            if opt_vars().p_ic.get()
                && opt_vars().p_scs.get()
                && !pat_has_uppercase(
                    (get_cmdline_info().cmdbuff().wrapping_add(skiplen as usize)).as_cstr(),
                )
            {
                *c = mb_tolower(*c);
            }
            let special_chars = if opt_vars().p_magic.get() {
                c"\\~^$.*["
            } else {
                c"\\^$"
            };
            if *c == search_delim || crate::strings::vim_haschr(special_chars, *c) {
                // put a backslash before special characters
                stuffchar_readbuff(*c);
                *c = b'\\' as i32;
            }
            // add any composing characters
            if utf_char2len(*c)
                != crate::mbyte::utfc_slice2len(&crate::cursor::get_cursor_pos_slice()) as i32
            {
                let save_c = *c;
                while utf_char2len(*c)
                    != crate::mbyte::utfc_slice2len(&crate::cursor::get_cursor_pos_slice()) as i32
                {
                    curwin().w_cursor.add_col(utf_char2len(*c));
                    *c = gchar_cursor();
                    stuffchar_readbuff(*c);
                }
                *c = save_c;
            }
            return false;
        }
    }
    true
}

fn finish_incsearch_highlighting(gotesc: bool, s: &mut IncsearchState, call_update_screen: bool) {
    if !s.did_incsearch {
        return;
    }

    s.did_incsearch = false;
    if gotesc {
        curwin().w_cursor.set(s.save_cursor);
    } else {
        if !equalpos(s.save_cursor, s.search_start) {
            // put the '" mark at the original position
            curwin().w_cursor.set(s.save_cursor);
            setpcmark();
        }
        curwin().w_cursor.set(s.search_start);
    }
    restore_viewstate(&curwin(), &s.old_viewstate);
    g_st().search_global.hl_match.set(false);

    // by default search all lines
    g_st().search_global.first_line.set(0);
    g_st().search_global.last_line.set(MAXLNUM);

    validate_cursor(&curwin()); // needed for TAB
    status_redraw_all();
    redraw_all_later(UPD_SOME_VALID);
    if call_update_screen {
        update_screen();
        may_trigger_win_scrolled_resized();
    }
}

/// Initialize the current command-line info.
fn init_ccline(firstc: i32, indent: i32) {
    get_cmdline_info().overstrike.set(0); // always start in insert mode

    debug_assert!(indent >= 0);

    // set some variables for redrawcmd()
    get_cmdline_info()
        .cmdfirstc
        .set(if firstc == b'@' as i32 { 0 } else { firstc });
    get_cmdline_info()
        .cmdindent
        .set(if firstc > 0 { indent } else { 0 });

    // alloc initial get_cmdline_info().cmdbuff()
    alloc_cmdbuff(indent + 50);
    get_cmdline_info().cmdpos.set(0);
    get_cmdline_info().cmdlen.set(0);
    get_cmdline_info().cmdbuff_vec.borrow_mut()[0] = 0;

    *get_cmdline_info().last_colors.borrow_mut() = ColoredCmdline::new();
    sb_text_start_cmdline();

    // autoindent for :insert and :append
    if firstc <= 0 {
        let ind_usize = indent as usize;
        get_cmdline_info().cmdbuff_vec.borrow_mut()[..ind_usize].fill(b' ');
        get_cmdline_info().cmdbuff_vec.borrow_mut()[ind_usize] = 0;
        get_cmdline_info().cmdpos.set(indent);
        get_cmdline_info().cmdspos.set(indent);
        get_cmdline_info().cmdlen.set(indent);
    }
}

fn ui_ext_cmdline_hide(abort: bool) {
    if ui_has(K_UICMDLINE) {
        g_st().cmdline_was_last_drawn.set(false);
        get_cmdline_info().redraw_state.set(K_CMD_REDRAW_NONE);
        ui_call_cmdline_hide(get_cmdline_info().level.get() as Integer, abort);
    }
}

/// Internal entry point for cmdline mode.
fn command_line_enter(
    firstc: i32,
    count: i32,
    indent: i32,
    clear_ccline: bool,
) -> Option<std::ffi::CString> {
    GETLN_STATE.with(|s| s.cmdline_level.set(s.cmdline_level.get() + (1)));

    let save_cmdpreview = g_st().cmdpreview.get();
    g_st().cmdpreview.set(false);
    let mut state = CommandLineState::new();
    state.firstc = firstc;
    state.count = count;
    state.indent = indent;
    state.save_msg_scroll = g_st().msg_scroll.get();
    state.save_state = g_st().state_global.get();
    state.prev_cmdpos = -1;
    state.ignore_drag_release = true;
    let s = &mut state;
    s.save_p_icm = Some(opt_vars().p_icm.to_owned());
    init_incsearch_state(&mut s.is_state);
    let mut did_save_ccline = false;

    if !get_cmdline_info().cmdbuff().is_null() {
        debug_assert!(clear_ccline);
        save_cmdline();
        did_save_ccline = true;
    } else if clear_ccline {
        CCLINE.with(|c| *c.borrow_mut() = std::rc::Rc::new(CmdlineInfo::new()));
    }

    if s.firstc == -1 {
        s.firstc = NUL as i32;
        s.break_ctrl_c = true;
    }

    init_ccline(s.firstc, s.indent);
    debug_assert!(!get_cmdline_info().cmdbuff().is_null());
    get_cmdline_info()
        .prompt_id
        .set(GETLN_STATE.with(|s| s.last_prompt_id.get()));
    GETLN_STATE.with(|s| {
        s.last_prompt_id
            .set(GETLN_STATE.with(|s| s.last_prompt_id.get()).wrapping_add(1))
    });
    get_cmdline_info()
        .level
        .set(GETLN_STATE.with(|s| s.cmdline_level.get()));

    if GETLN_STATE.with(|s| s.cmdline_level.get()) != 50 {
        let mut xpc = ExpandCtx::new();
        expand_init(&mut xpc);
        clear_cmdline_orig();

        g_st().cmdmsg_rl.set(
            curwin().w_onebuf_opt.wo_rl.get()
                && curwin().w_onebuf_opt.wo_rlc.first_byte() == b's'
                && (s.firstc == b'/' as i32 || s.firstc == b'?' as i32),
        );

        msg_grid_validate();

        g_st().redir_off.set(true); // don't redirect the typed command
        if !g_st().cmd_silent.get() {
            gotocmdline(true);
            redrawcmdprompt(); // draw prompt or indent
            get_cmdline_info().cmdspos.set(cmd_startcol());
        }
        xpc.xp_context = EXPAND_NOTHING;
        xpc.xp_backslash = XP_BS_NONE;
        #[cfg(not(windows))]
        {
            xpc.xp_shell = false;
        }

        if get_cmdline_info().input_fn.get() != 0 {
            xpc.xp_context = get_cmdline_info().xp_context.get();
            xpc.xp_pattern = get_cmdline_info().cmdbuff();
            xpc.xp_arg = get_cmdline_info().xp_arg.borrow().clone();
        }
        *get_cmdline_info().xpc.borrow_mut() = Some(xpc);

        g_st().msg_scroll.set(0);

        g_st().state_global.set(MODE_CMDLINE);

        if s.firstc == b'/' as i32 || s.firstc == b'?' as i32 || s.firstc == b'@' as i32 {
            let (use_insert, im_val) = if curbuf().b_p_imsearch.get() == B_IMODE_USE_INSERT {
                (true, curbuf().b_p_iminsert.get())
            } else {
                (false, curbuf().b_p_imsearch.get())
            };
            s.b_im_use_insert = Some(use_insert);
            s.b_im_ptr_buf = g_st().curbuf.get();
            if im_val == B_IMODE_LMAP {
                g_st()
                    .state_global
                    .set(g_st().state_global.get() | (MODE_LANGMAP));
            }
        }

        setmouse();

        s.cmdline_type = if firstc > 0 { firstc } else { b'-' as i32 };
        let mut err = error_init();
        let firstcbuf: [u8; 2] = [s.cmdline_type as u8, 0];

        if has_event(EVENT_CMDLINEENTER) {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let dict = get_v_event(&mut save_v_event);
            let firstc_cstr = crate::memory::bytes_as_cstr(&firstcbuf);

            tv_dict_add_str(&dict, c"cmdtype", Some(firstc_cstr));
            tv_dict_add_nr(
                &dict,
                c"cmdlevel",
                get_cmdline_info().level.get() as Varnumber,
            );
            tv_dict_set_keys_readonly(&dict);
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            apply_autocmds(
                EVENT_CMDLINEENTER,
                Some(firstc_cstr),
                Some(firstc_cstr),
                false,
                curbuf_opt().as_deref(),
            );
            restore_v_event(&dict, &mut save_v_event);
            try_leave(&tstate, &mut err);

            if error_set(&err) {
                if !ui_has(K_UIMESSAGES) {
                    msg_putchar(b'\n' as i32);
                }
                g_st().msg_scroll.set(1);
                msg_puts_hl(err.msg_cstr().unwrap_or(c""), HLF_E, true);
                api_clear_error(&mut err);
                redrawcmd();
            }
            err = error_init();
        }
        may_trigger_modechanged();

        init_history();
        s.hiscnt = get_hislen(); // set hiscnt to impossible history value
        s.histype = hist_char2type(s.firstc);
        do_digraph(-1); // init digraph typeahead

        if g_st().did_emsg.get() != 0 {
            redrawcmd();
        }

        if !g_st().cmd_silent.get() {
            let mut found_one = false;

            for wp in for_all_windows_ptr() {
                if !opt_vars().p_stl.is_empty()
                    || !wp.w_onebuf_opt.wo_stl.is_empty()
                    || !opt_vars().p_wbr.is_empty()
                    || !wp.w_onebuf_opt.wo_wbr.is_empty()
                {
                    wp.w_redr_status.set(true);
                    found_one = true;
                }
            }

            if !opt_vars().p_tal.is_empty() {
                g_st().redraw_tabline.set(true);
                found_one = true;
            }

            if redraw_custom_title_later() {
                found_one = true;
            }

            if found_one {
                redraw_statuslines();
            }
        }

        g_st().did_emsg.set(0);
        g_st().got_int.set(false);

        state_enter(&mut *s);

        if !s.event_cmdlineleavepre_triggered {
            set_vim_var_char(s.c);
            trigger_cmd_autocmd(s.cmdline_type, EVENT_CMDLINELEAVEPRE);
        }

        if has_event(EVENT_CMDLINELEAVE) {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let dict = get_v_event(&mut save_v_event);
            let firstc_cstr = crate::memory::bytes_as_cstr(&firstcbuf);

            tv_dict_add_str(&dict, c"cmdtype", Some(firstc_cstr));
            tv_dict_add_nr(
                &dict,
                c"cmdlevel",
                get_cmdline_info().level.get() as Varnumber,
            );
            tv_dict_set_keys_readonly(&dict);
            tv_dict_add_bool(
                &dict,
                c"abort",
                if s.gotesc {
                    K_BOOL_VAR_TRUE_CONST
                } else {
                    K_BOOL_VAR_FALSE_CONST
                },
            );
            set_vim_var_char(s.c);
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            let firstc_cstr = crate::memory::bytes_as_cstr(&firstcbuf);
            apply_autocmds(
                EVENT_CMDLINELEAVE,
                Some(firstc_cstr),
                Some(firstc_cstr),
                false,
                curbuf_opt().as_deref(),
            );
            try_leave(&tstate, &mut err);
            if tv_dict_get_number(Some(&dict), c"abort") != 0 {
                s.gotesc = true;
            }
            restore_v_event(&dict, &mut save_v_event);
        }

        g_st().cmdmsg_rl.set(false);

        if cmdline_pum_active() {
            cmdline_pum_remove(false);
        } else {
            pum_check_clear();
        }
        wildmenu_cleanup(&get_cmdline_info());
        s.did_wild_list = false;
        s.wim_index = 0;

        if let Some(mut xpc) = get_cmdline_info().xpc.borrow_mut().take() {
            expand_cleanup(&mut xpc);
        }
        clear_cmdline_orig();

        finish_incsearch_highlighting(s.gotesc, &mut s.is_state, false);

        if !get_cmdline_info().cmdbuff().is_null() {
            if s.histype != HIST_INVALID
                && get_cmdline_info().cmdlen.get() != 0
                && s.firstc != NUL as i32
                && (s.some_key_typed || s.histype == HIST_SEARCH)
            {
                add_to_history(
                    s.histype,
                    (get_cmdline_info().cmdbuff()).slice(get_cmdline_info().cmdlen.get() as usize),
                    true,
                    if s.histype == HIST_SEARCH {
                        s.firstc
                    } else {
                        NUL as i32
                    },
                );
                if s.firstc == b':' as i32 {
                    g_st()
                        .new_last_cmdline
                        .set(Some(crate::memory::bytes_to_cstring(
                            (get_cmdline_info().cmdbuff())
                                .slice(get_cmdline_info().cmdlen.get() as usize),
                        )));
                }
            }

            if !s.gotesc && get_cmdline_info().level.get() == 1 {
                atom_cmdline_set(
                    s.firstc,
                    (get_cmdline_info().cmdbuff()).slice(get_cmdline_info().cmdlen.get() as usize),
                );
            }

            if s.gotesc {
                abandon_cmdline();
            }
        }

        msg_check();
        if opt_vars().p_ch.get() == 0 && !ui_has(K_UIMESSAGES) {
            set_must_redraw(UPD_VALID);
        }
        g_st().msg_scroll.set(s.save_msg_scroll);
        g_st().redir_off.set(false);

        if error_set(&err) {
            if !ui_has(K_UIMESSAGES) {
                msg_putchar(b'\n' as i32);
            }
            emsg(err.msg_cstr().unwrap_or(c""));
            g_st().did_emsg.set(0);
            api_clear_error(&mut err);
        }

        if s.some_key_typed && !error_set(&err) {
            g_st().need_wait_return.set(false);
        }

        set_option_direct(
            K_OPT_INCCOMMAND,
            cstr_as_obj(Some(s.save_p_icm.as_deref().unwrap_or(c""))),
            0,
            SID_NONE,
        );
        g_st().state_global.set(s.save_state);
        if g_st().cmdpreview.get() != save_cmdpreview {
            g_st().cmdpreview.set(save_cmdpreview);
            redraw_all_later(UPD_SOME_VALID);
        }
        may_trigger_modechanged();
        setmouse();
        sb_text_end_cmdline();
    } else {
        emsg(gettext(E_COMMAND_TOO_RECURSIVE));
    }

    // theend:
    s.save_p_icm = None;
    get_cmdline_info().last_colors.borrow_mut().cmdbuff = None;
    get_cmdline_info().last_colors.borrow_mut().colors = Vec::new();

    let ret = if !get_cmdline_info().cmdbuff().is_null() {
        Some(get_cmdline_info().cmdbuff().to_cstring())
    } else {
        None
    };

    if ui_has(K_UICMDLINE) {
        ui_ext_cmdline_hide(s.gotesc);
    }
    if !g_st().cmd_silent.get() {
        redraw_custom_title_later();
        status_redraw_all();
    }

    GETLN_STATE.with(|s| s.cmdline_level.set(s.cmdline_level.get() - (1)));

    if did_save_ccline {
        restore_cmdline();
    } else {
        dealloc_cmdbuff();
    }

    s.prev_cmdbuff = None;
    ret
}

fn command_line_check(s: &mut CommandLineState) -> i32 {
    s.prev_cmdpos = get_cmdline_info().cmdpos.get();
    s.prev_cmdbuff = None;

    g_st().redir_off.set(true);
    g_st().quit_more.set(false);
    g_st().did_emsg.set(0);

    if g_st().ex_normal_busy.get() == 0 && stuff_empty() && g_st().typebuf.tb_len.get() == 0 {
        s.some_key_typed = true;
    }

    may_trigger_safestate(with_ccline_xpc(|xpc| xpc.xp_numfiles <= 0));

    if !get_cmdline_info().cmdbuff().is_null() {
        s.prev_cmdbuff = Some(get_cmdline_info().cmdbuff().to_cstring());
    }

    if s.c == K_WILD && s.firstc != b'@' as i32 {
        s.skip_pum_redraw = true;
    }

    cursorcmd();
    ui_cursor_shape();
    1
}

/// Handle `CTRL-\` pressed in Command-line mode.
fn command_line_handle_ctrl_bsl(s: &mut CommandLineState) -> i32 {
    g_st().no_mapping.set(g_st().no_mapping.get() + (1));
    g_st().allow_keys.set(g_st().allow_keys.get() + (1));
    s.c = plain_vgetc();
    g_st().no_mapping.set(g_st().no_mapping.get() - (1));
    g_st().allow_keys.set(g_st().allow_keys.get() - (1));

    if s.c != CTRL_N
        && s.c != CTRL_G
        && (s.c != b'e' as i32
            || (get_cmdline_info().cmdfirstc.get() == b'=' as i32 && g_st().key_typed.get())
            || g_st().cmdline_star.get() > 0)
    {
        vungetc(s.c);
        return PROCESS_NEXT_KEY;
    }

    if s.c == b'e' as i32 {
        if get_cmdline_info().cmdpos.get() == get_cmdline_info().cmdlen.get() {
            GETLN_STATE.with(|s| s.new_cmdpos.set(99999));
        } else {
            GETLN_STATE.with(|s| s.new_cmdpos.set(get_cmdline_info().cmdpos.get()));
        }

        s.c = get_expr_register();
        if s.c == b'=' as i32 {
            g_st().textlock.set(g_st().textlock.get() + (1));
            let p = get_expr_line();
            g_st().textlock.set(g_st().textlock.get() - (1));

            if let Some(p) = p {
                let bytes = p.as_bytes_with_nul();
                let len = (bytes.len() - 1) as i32;
                realloc_cmdbuff(len + 1);
                get_cmdline_info().cmdlen.set(len);
                get_cmdline_info().cmdbuff_vec.borrow_mut()[..bytes.len()].copy_from_slice(bytes);

                get_cmdline_info().cmdpos.set(
                    get_cmdline_info()
                        .cmdlen
                        .get()
                        .min(GETLN_STATE.with(|s| s.new_cmdpos.get())),
                );

                g_st().key_typed.set(false);
                redrawcmd();
                return CMDLINE_CHANGED;
            }
        }
        beep_flush();
        g_st().got_int.set(false);
        g_st().did_emsg.set(0);
        g_st().emsg_on_display.set(false);
        redrawcmd();
        return CMDLINE_NOT_CHANGED;
    }

    s.gotesc = true;
    GOTO_NORMAL_MODE
}

/// Completion for `'wildchar'` or `'wildcharm'` key.
fn command_line_wildchar_complete(s: &mut CommandLineState) -> i32 {
    let res: bool;
    let mut options = WILD_NO_BEEP;
    let escape = s.firstc != b'@' as i32;
    let redraw_if_menu_empty = s.c == K_WILD;
    let wim_noselect =
        opt_vars().p_wmnu.get() && (g_st().wim_flags[0].get() & K_OPT_WIM_FLAG_NOSELECT) != 0;
    let wim_noinsert =
        opt_vars().p_wmnu.get() && (g_st().wim_flags[0].get() & K_OPT_WIM_FLAG_NOINSERT) != 0;

    if (g_st().wim_flags[s.wim_index as usize].get() & K_OPT_WIM_FLAG_LASTUSED) != 0 {
        options |= WILD_BUFLASTUSED;
    }
    if with_ccline_xpc(|xpc| xpc.xp_numfiles > 0) {
        if with_ccline_xpc(|xpc| xpc.xp_numfiles > 1)
            && !s.did_wild_list
            && (g_st().wim_flags[s.wim_index as usize].get() & K_OPT_WIM_FLAG_LIST) != 0
        {
            with_ccline_xpc(|xpc| {
                showmatches(
                    xpc,
                    false,
                    true,
                    if opt_vars().p_wmnu.get() {
                        g_st().wim_flags[s.wim_index as usize].get() as i32
                    } else {
                        0
                    },
                );
            });
            redrawcmd();
            s.did_wild_list = true;
        }
        if (g_st().wim_flags[s.wim_index as usize].get() & K_OPT_WIM_FLAG_LONGEST) != 0 {
            res = with_ccline_xpc(|xpc| nextwild(xpc, WILD_LONGEST, options, escape));
        } else if (g_st().wim_flags[s.wim_index as usize].get() & K_OPT_WIM_FLAG_FULL) != 0 {
            res = with_ccline_xpc(|xpc| nextwild(xpc, WILD_NEXT, options, escape));
        } else {
            res = true;
        }
    } else {
        let wim_longest = (g_st().wim_flags[0].get() & K_OPT_WIM_FLAG_LONGEST) != 0;
        let wim_list = (g_st().wim_flags[0].get() & K_OPT_WIM_FLAG_LIST) != 0;
        let wim_full = (g_st().wim_flags[0].get() & K_OPT_WIM_FLAG_FULL) != 0;

        s.wim_index = 0;
        if s.c as OptInt == opt_vars().p_wc.get()
            || s.c as OptInt == opt_vars().p_wcm.get()
            || s.c == K_WILD
            || s.c == CTRL_Z_CONST
        {
            options |= WILD_MAY_EXPAND_PATTERN;
            if s.c == K_WILD {
                options |= WILD_FUNC_TRIGGER;
            }
            with_ccline_xpc(|xpc| xpc.xp_pre_incsearch_pos = s.is_state.search_start);
        }
        let cmdpos_before = get_cmdline_info().cmdpos.get();

        if wim_longest {
            res = with_ccline_xpc(|xpc| nextwild(xpc, WILD_LONGEST, options, escape));
        } else {
            if wim_noselect || (wim_list && !wim_full) {
                options |= WILD_NOSELECT;
            }
            if wim_noinsert {
                options |= WILD_NOINSERT;
            }
            res = with_ccline_xpc(|xpc| nextwild(xpc, WILD_EXPAND_KEEP, options, escape));
        }

        if redraw_if_menu_empty && with_ccline_xpc(|xpc| xpc.xp_numfiles <= 0) {
            pum_check_clear();
        }

        if g_st().got_int.get() {
            vpeekc();
            g_st().got_int.set(false);
            with_ccline_xpc(|xpc| {
                expand_one(xpc, None, None, 0, WILD_FREE);
                xpc.xp_context = EXPAND_NOTHING;
            });
            return CMDLINE_CHANGED;
        }

        if res
            && with_ccline_xpc(|xpc| {
                xpc.xp_numfiles > if wim_noselect || wim_noinsert { 0 } else { 1 }
            })
        {
            if wim_longest {
                let found_longest_prefix = get_cmdline_info().cmdpos.get() != cmdpos_before;
                let show_menu = opt_vars().p_wmnu.get() && wim_full;
                if wim_list || show_menu {
                    with_ccline_xpc(|xpc| {
                        showmatches(xpc, show_menu, wim_list, K_OPT_WIM_FLAG_NOSELECT as i32);
                    });
                } else if !found_longest_prefix {
                    let wim_list_next = (g_st().wim_flags[1].get() & K_OPT_WIM_FLAG_LIST) != 0;
                    let wim_full_next = (g_st().wim_flags[1].get() & K_OPT_WIM_FLAG_FULL) != 0;
                    let wim_noselect_next =
                        (g_st().wim_flags[1].get() & K_OPT_WIM_FLAG_NOSELECT) != 0;
                    let wim_noinsert_next =
                        (g_st().wim_flags[1].get() & K_OPT_WIM_FLAG_NOINSERT) != 0;
                    if wim_list_next
                        || (opt_vars().p_wmnu.get()
                            && (wim_full_next || wim_noselect_next || wim_noinsert_next))
                    {
                        if wim_full_next && !wim_noselect_next && !wim_noinsert_next {
                            with_ccline_xpc(|xpc| {
                                nextwild(xpc, WILD_NEXT, options, escape);
                            });
                        } else {
                            with_ccline_xpc(|xpc| {
                                showmatches(
                                    xpc,
                                    opt_vars().p_wmnu.get()
                                        && (wim_noselect_next || wim_noinsert_next),
                                    wim_list_next,
                                    if opt_vars().p_wmnu.get() {
                                        g_st().wim_flags[1].get() as i32
                                    } else {
                                        0
                                    },
                                );
                            });
                        }
                        if wim_list_next {
                            s.did_wild_list = true;
                        }
                    }
                }
            } else {
                let show_menu =
                    opt_vars().p_wmnu.get() && (wim_full || wim_noselect || wim_noinsert);
                if wim_list || show_menu {
                    with_ccline_xpc(|xpc| {
                        showmatches(
                            xpc,
                            show_menu,
                            wim_list,
                            if opt_vars().p_wmnu.get() {
                                g_st().wim_flags[0].get() as i32
                            } else {
                                0
                            },
                        );
                    });
                } else {
                    vim_beep(K_OPT_BO_FLAG_WILDMODE);
                }
            }

            redrawcmd();
            if wim_list {
                s.did_wild_list = true;
            }
        } else if with_ccline_xpc(|xpc| xpc.xp_numfiles == -1) {
            with_ccline_xpc(|xpc| xpc.xp_context = EXPAND_NOTHING);
        }
    }

    if s.wim_index < 3 {
        s.wim_index += 1;
    }

    if s.c == ESC {
        s.gotesc = true;
    }

    if res {
        CMDLINE_CHANGED
    } else {
        CMDLINE_NOT_CHANGED
    }
}

fn command_line_end_wildmenu(s: &mut CommandLineState, key_is_wc: bool, c: i32) {
    if cmdline_pum_active() {
        if c != -1 {
            s.skip_pum_redraw = s.skip_pum_redraw
                && !key_is_wc
                && !ascii_iswhite(c)
                && (vim_isprintc(c)
                    || c == K_BS
                    || c == CTRL_H_CONST
                    || c == K_DEL
                    || c == K_KDEL
                    || c == CTRL_W_CONST
                    || c == CTRL_U);
        }
        cmdline_pum_remove(c != -1 && s.skip_pum_redraw);
    }
    if with_ccline_xpc(|xpc| xpc.xp_numfiles != -1) {
        with_ccline_xpc(|xpc| {
            expand_one(xpc, None, None, 0, WILD_FREE);
        });
    }
    s.did_wild_list = false;
    if !opt_vars().p_wmnu.get() || (c != K_UP && c != K_DOWN) {
        with_ccline_xpc(|xpc| xpc.xp_context = EXPAND_NOTHING);
    }
    s.wim_index = 0;
    wildmenu_cleanup(&get_cmdline_info());
}

fn command_line_execute(s: &mut CommandLineState, key: i32) -> i32 {
    if key == K_IGNORE || key == K_NOP {
        return -1; // get another key
    }

    let display_tick_saved: Disptick = curwin().w_display_tick.get();
    s.c = key;

    if get_cmdline_info().cmdbuff_replaced.get() && with_ccline_xpc(|xpc| xpc.xp_numfiles > 0) {
        command_line_end_wildmenu(s, false, -1);
    }
    get_cmdline_info().cmdbuff_replaced.set(false);

    if s.c == K_WILD && s.did_hist_navigate {
        s.did_hist_navigate = false;
        return 1;
    }

    if s.c == K_EVENT || s.c == K_COMMAND || s.c == K_LUA {
        if s.c == K_EVENT {
            state_handle_k_event();
        } else if s.c == K_COMMAND {
            do_cmdline(None, crate::cmdexpand::LineGetter::CmdKeyCmd, DOCMD_NOWAIT);
        } else {
            map_execute_lua(false, false);
        }
        if s.is_state.winid != curwin().handle {
            init_incsearch_state(&mut s.is_state);
        }
        if curwin().w_display_tick.get() > display_tick_saved && s.is_state.did_incsearch {
            may_do_incsearch_highlighting(s.firstc, s.count, &mut s.is_state);
        }
        if get_cmdline_info().cmdbuff_replaced.get() {
            command_line_changed(s);
        }

        if g_st().pum_want.active.get() {
            if cmdline_pum_active() {
                with_ccline_xpc(|xpc| {
                    nextwild(xpc, WILD_PUM_WANT, 0, s.firstc != b'@' as i32);
                });
                if g_st().pum_want.finish.get() {
                    with_ccline_xpc(|xpc| {
                        nextwild(xpc, WILD_APPLY, WILD_NO_BEEP, s.firstc != b'@' as i32);
                    });
                    command_line_end_wildmenu(s, false, s.c);
                }
            }
            g_st().pum_want.active.set(false);
        }

        if !g_st().cmdline_was_last_drawn.get() {
            redrawcmdline();
        }
        return 1;
    }

    if g_st().key_typed.get() {
        s.some_key_typed = true;

        if g_st().cmdmsg_rl.get() && g_st().key_stuffed.get() == 0 {
            match s.c {
                K_RIGHT => s.c = K_LEFT,
                K_S_RIGHT => s.c = K_S_LEFT,
                K_C_RIGHT => s.c = K_C_LEFT,
                K_LEFT => s.c = K_RIGHT,
                K_S_LEFT => s.c = K_S_RIGHT,
                K_C_LEFT => s.c = K_C_RIGHT,
                _ => {}
            }
        }
    }

    if s.c == CTRL_C_CONST
        && s.firstc != b'@' as i32
        && !s.break_ctrl_c
        && g_st().global_busy.get() == 0
    {
        g_st().got_int.set(false);
    }

    if s.lookfor.is_some()
        && s.c != K_S_DOWN
        && s.c != K_S_UP
        && s.c != K_DOWN
        && s.c != K_UP
        && s.c != K_PAGEDOWN
        && s.c != K_PAGEUP
        && s.c != K_KPAGEDOWN
        && s.c != K_KPAGEUP
        && s.c != K_LEFT
        && s.c != K_RIGHT
        && (with_ccline_xpc(|xpc| xpc.xp_numfiles > 0) || (s.c != CTRL_P && s.c != CTRL_N))
    {
        s.lookfor = None;
        s.lookforlen = 0;
    }

    if s.c as OptInt != opt_vars().p_wc.get()
        && s.c == K_S_TAB
        && with_ccline_xpc(|xpc| xpc.xp_numfiles > 0)
    {
        s.c = CTRL_P;
    }

    if opt_vars().p_wmnu.get() {
        let ccline = get_cmdline_info();
        s.c = with_ccline_xpc(|xpc| wildmenu_translate_key(&ccline, s.c, xpc, s.did_wild_list));
    }

    let mut wild_type: i32 = 0;
    let key_is_wc = (s.c as OptInt == opt_vars().p_wc.get() && g_st().key_typed.get())
        || s.c as OptInt == opt_vars().p_wcm.get();
    if (cmdline_pum_active() || g_st().wild_menu_showing.get() != 0 || s.did_wild_list)
        && !key_is_wc
        && with_ccline_xpc(|xpc| xpc.xp_numfiles > 0)
        && (s.c == CTRL_E || s.c == CTRL_Y)
    {
        wild_type = if s.c == CTRL_E {
            WILD_CANCEL
        } else {
            WILD_APPLY
        };
        with_ccline_xpc(|xpc| {
            nextwild(xpc, wild_type, WILD_NO_BEEP, s.firstc != b'@' as i32);
        });
    }

    if (g_st().key_typed.get()
        && (s.c == b'\n' as i32 || s.c == b'\r' as i32 || s.c == K_KENTER || s.c == ESC))
        || s.c == CTRL_C_CONST
    {
        set_vim_var_char(s.c);
        trigger_cmd_autocmd(s.cmdline_type, EVENT_CMDLINELEAVEPRE);
        s.event_cmdlineleavepre_triggered = true;
        if (s.c == ESC || s.c == CTRL_C_CONST)
            && (g_st().wim_flags[0].get() & K_OPT_WIM_FLAG_LIST) != 0
        {
            set_no_hlsearch(true);
        }
    }

    let mut end_wildmenu = !key_is_wc
        && s.c != CTRL_Z_CONST
        && s.c != CTRL_N
        && s.c != CTRL_P
        && s.c != CTRL_A
        && s.c != CTRL_L_CONST;
    end_wildmenu = end_wildmenu
        && (!cmdline_pum_active()
            || (s.c != K_PAGEDOWN
                && s.c != K_PAGEUP
                && s.c != K_KPAGEDOWN
                && s.c != K_KPAGEUP
                && s.c != K_MOUSEDOWN
                && s.c != K_MOUSEUP
                && s.c != K_MOUSELEFT
                && s.c != K_MOUSERIGHT));

    if end_wildmenu {
        command_line_end_wildmenu(s, key_is_wc, s.c);
    }

    if opt_vars().p_wmnu.get() {
        let ccline = get_cmdline_info();
        s.c = with_ccline_xpc(|xpc| wildmenu_process_key(&ccline, s.c, xpc));
    }

    if s.c == CTRL_BSL {
        match command_line_handle_ctrl_bsl(&mut *s) {
            CMDLINE_CHANGED => return command_line_changed(s),
            CMDLINE_NOT_CHANGED => return command_line_not_changed(s),
            GOTO_NORMAL_MODE => return 0,
            _ => {
                s.c = CTRL_BSL;
            }
        }
    }

    if s.c == GETLN_STATE.with(|s| s.cedit_key.get()) {
        if g_st().ex_normal_busy.get() == 0 && !g_st().got_int.get() {
            s.c = open_cmdwin();
            s.some_key_typed = true;
        }
    } else {
        s.c = do_digraph(s.c);
    }

    if s.c == b'\n' as i32
        || s.c == b'\r' as i32
        || s.c == K_KENTER
        || (s.c == ESC
            && (!g_st().key_typed.get()
                || crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_ESC)))
    {
        s.gotesc = false;
        if ccheck_abbr(s.c + ABBR_OFF) != 0 {
            return command_line_changed(s);
        }

        if !g_st().cmd_silent.get() {
            if !ui_has(K_UICMDLINE) {
                msg_cursor_goto(g_st().msg_row.get(), 0);
            }
            ui_flush();
        }
        return 0;
    }

    if (s.c as OptInt == opt_vars().p_wc.get() && !s.gotesc && g_st().key_typed.get())
        || s.c as OptInt == opt_vars().p_wcm.get()
        || s.c == K_WILD
        || s.c == CTRL_Z_CONST
    {
        if s.c == K_WILD {
            g_st().emsg_silent.set(g_st().emsg_silent.get() + (1));
        }
        let res = command_line_wildchar_complete(s);
        if s.c == K_WILD {
            g_st().emsg_silent.set(g_st().emsg_silent.get() - (1));
        }
        if res == CMDLINE_CHANGED {
            return command_line_changed(s);
        }
        if s.c == K_WILD {
            return command_line_not_changed(s);
        }
    }

    s.gotesc = false;

    if s.c == K_S_TAB
        && g_st().key_typed.get()
        && with_ccline_xpc(|xpc| nextwild(xpc, WILD_EXPAND_KEEP, 0, s.firstc != b'@' as i32))
    {
        if with_ccline_xpc(|xpc| xpc.xp_numfiles > 1)
            && ((!s.did_wild_list
                && (g_st().wim_flags[s.wim_index as usize].get() & K_OPT_WIM_FLAG_LIST) != 0)
                || opt_vars().p_wmnu.get())
        {
            let list = (g_st().wim_flags[s.wim_index as usize].get() & K_OPT_WIM_FLAG_LIST) != 0;
            with_ccline_xpc(|xpc| {
                showmatches(
                    xpc,
                    opt_vars().p_wmnu.get() && !list,
                    list,
                    if opt_vars().p_wmnu.get() {
                        g_st().wim_flags[0].get() as i32
                    } else {
                        0
                    },
                );
            });
        }
        with_ccline_xpc(|xpc| {
            nextwild(xpc, WILD_PREV, 0, s.firstc != b'@' as i32);
            nextwild(xpc, WILD_PREV, 0, s.firstc != b'@' as i32);
        });
        return command_line_changed(s);
    }

    if s.c == NUL as i32 || s.c == K_ZERO {
        s.c = NL;
    }

    s.do_abbr = true;

    if wild_type == WILD_CANCEL || wild_type == WILD_APPLY {
        if s.is_state.winid != curwin().handle {
            init_incsearch_state(&mut s.is_state);
        }
        if g_st().key_typed.get() || vpeekc() == NUL as i32 {
            may_do_incsearch_highlighting(s.firstc, s.count, &mut s.is_state);
        }
        return command_line_not_changed(s);
    }

    command_line_handle_key(s)
}

/// May adjust `'incsearch'` highlighting for typing CTRL-G and CTRL-T, go to next
/// or previous match.
/// Returns `FAIL` when calling `command_line_not_changed`.
fn may_do_command_line_next_incsearch(
    firstc: i32,
    count: i32,
    s: &mut IncsearchState,
    next_match: bool,
) -> bool {
    let mut skiplen: i32 = 0;
    let mut patlen: i32 = 0;
    let mut search_delim: i32 = 0;
    let mut magic: bool = opt_vars().p_magic.get();

    save_last_search_pattern();

    if !do_incsearch_highlighting(
        firstc,
        &mut search_delim,
        s,
        &mut skiplen,
        &mut patlen,
        &mut magic,
    ) {
        restore_last_search_pattern();
        return true;
    }
    if patlen == 0 && get_cmdline_info().cmdbuff_vec.borrow()[skiplen as usize] == 0 {
        restore_last_search_pattern();
        return false;
    }

    ui_busy_start();
    ui_flush();

    let mut t: CursorPos;
    let pat = get_cmdline_info().cmdbuff().wrapping_add(skiplen as usize);
    let mut pat_buf = (pat)
        .slice((get_cmdline_info().cmdlen.get() - skiplen) as usize)
        .to_vec();
    pat_buf.push(0);
    let mut pat_off = 0usize;
    let mut had_copy = false;
    let mut offset = SearchOffset::new();
    let mut search_flags = SEARCH_NOOF;
    let mut patlen_s = (get_cmdline_info().cmdlen.get() - skiplen) as usize;

    let (_, searchstr_cstr) = parse_search_pattern_offset(
        &mut pat_buf,
        &mut pat_off,
        &mut patlen_s,
        search_delim,
        SEARCH_OPT,
        &mut had_copy,
        &mut offset,
    );
    let searchstrlen = searchstr_cstr.as_bytes().len();

    if next_match {
        t = s.match_end;
        if lt(s.match_start, s.match_end) {
            decl(&mut t);
        }
        search_flags += SEARCH_COL;
    } else {
        t = s.match_start;
    }
    if !opt_vars().p_hls.get() {
        search_flags += SEARCH_KEEP;
    }
    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    let found = searchit(
        Some(&curwin()),
        &curbuf(),
        &mut t,
        None,
        if next_match { FORWARD } else { BACKWARD },
        &searchstr_cstr,
        searchstrlen,
        count,
        search_flags,
        RE_SEARCH,
        magic,
        None,
    );
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    ui_busy_stop();
    if found != 0 {
        let match_start = s.match_start;
        let match_end = s.match_end;
        let mut off = offset.off;

        s.search_start = match_start;
        if !offset.line && (offset.end || off != 0) {
            if offset.end {
                s.search_start = match_end;
                decl(&mut s.search_start);
            }
            while off > 0 {
                if incl(&mut s.search_start) == -1 {
                    break;
                }
                off -= 1;
            }
            while off < 0 {
                if decl(&mut s.search_start) == -1 {
                    break;
                }
                off += 1;
            }
        }
        s.match_end = t;
        s.match_start = t;
        if !next_match && firstc != b'?' as i32 {
            s.search_start = t;
            decl(&mut s.search_start);
        } else if next_match && firstc == b'?' as i32 {
            s.search_start = t;
            incl(&mut s.search_start);
        }
        if lt(t, s.search_start) && next_match {
            s.search_start = t;
            if firstc == b'?' as i32 {
                incl(&mut s.search_start);
            } else {
                decl(&mut s.search_start);
            }
        }

        set_search_match(&mut s.match_end);
        curwin().w_cursor.set(s.match_start);
        changed_cline_bef_curs(&curwin());
        update_topline(&curwin());
        validate_cursor(&curwin());
        g_st().search_global.hl_match.set(true);
        save_viewstate(&curwin(), &mut s.old_viewstate);
        redraw_later(Some(&crate::globals::curwin()), UPD_NOT_VALID);
        update_screen();
        g_st().search_global.hl_match.set(false);
        redrawcmdline();
        curwin().w_cursor.set(s.match_end);
    } else {
        vim_beep(K_OPT_BO_FLAG_ERROR);
    }
    restore_last_search_pattern();
    false
}

/// Handle backspace, delete and CTRL-W keys in the command-line mode.
fn command_line_erase_chars(s: &mut CommandLineState) -> i32 {
    if s.c == K_KDEL {
        s.c = K_DEL;
    }

    if s.c == K_DEL && get_cmdline_info().cmdpos.get() != get_cmdline_info().cmdlen.get() {
        get_cmdline_info()
            .cmdpos
            .set(get_cmdline_info().cmdpos.get() + (1));
    }
    if s.c == K_DEL {
        get_cmdline_info().cmdpos.set(
            get_cmdline_info().cmdpos.get()
                + (crate::mbyte::mb_off_next(
                    &get_cmdline_info().cmdbuff_vec.borrow(),
                    get_cmdline_info().cmdpos.get() as usize,
                )),
        );
    }

    if get_cmdline_info().cmdpos.get() > 0 {
        let j = get_cmdline_info().cmdpos.get();
        let mut p_idx = j as usize;
        crate::mbyte::mb_idx_back(&get_cmdline_info().cmdbuff_vec.borrow(), &mut p_idx);

        if s.c == CTRL_W_CONST {
            while p_idx > 0 && ascii_isspace(get_cmdline_info().cmdbuff_vec.borrow()[p_idx]) {
                crate::mbyte::mb_idx_back(&get_cmdline_info().cmdbuff_vec.borrow(), &mut p_idx);
            }

            let cls = mb_get_class(crate::memory::bytes_until_nul(
                &get_cmdline_info().cmdbuff_vec.borrow()[p_idx..],
            ));
            while p_idx > 0
                && mb_get_class(crate::memory::bytes_until_nul(
                    &get_cmdline_info().cmdbuff_vec.borrow()[p_idx..],
                )) == cls
            {
                crate::mbyte::mb_idx_back(&get_cmdline_info().cmdbuff_vec.borrow(), &mut p_idx);
            }

            if mb_get_class(crate::memory::bytes_until_nul(
                &get_cmdline_info().cmdbuff_vec.borrow()[p_idx..],
            )) != cls
            {
                p_idx += crate::mbyte::utfc_slice2len(crate::memory::bytes_until_nul(
                    &get_cmdline_info().cmdbuff_vec.borrow()[p_idx..],
                ));
            }
        }

        let old_cmdlen = get_cmdline_info().cmdlen.get() as usize;
        get_cmdline_info().cmdpos.set(p_idx as i32);
        let new_cmdpos = get_cmdline_info().cmdpos.get() as usize;
        get_cmdline_info()
            .cmdlen
            .set(get_cmdline_info().cmdlen.get() - (j - get_cmdline_info().cmdpos.get()));
        let new_cmdlen = get_cmdline_info().cmdlen.get() as usize;
        get_cmdline_info()
            .cmdbuff_vec
            .borrow_mut()
            .copy_within(j as usize..old_cmdlen, new_cmdpos);
        get_cmdline_info().cmdbuff_vec.borrow_mut()[new_cmdlen] = 0;

        if get_cmdline_info().cmdlen.get() == 0 {
            s.is_state.search_start = s.is_state.save_cursor;
            s.is_state.old_viewstate = s.is_state.init_viewstate;
        }
        redrawcmd();
    } else if get_cmdline_info().cmdlen.get() == 0
        && s.c != CTRL_W_CONST
        && get_cmdline_info().cmdprompt.borrow().is_none()
        && s.indent == 0
    {
        if get_cmdline_info().cmdfirstc.get() == b'>' as i32 {
            return CMDLINE_NOT_CHANGED;
        }

        dealloc_cmdbuff();

        if !g_st().cmd_silent.get() && !ui_has(K_UICMDLINE) {
            g_st().msg_col.set(0);
            msg_putchar(b' ' as i32);
        }
        s.is_state.search_start = s.is_state.save_cursor;
        g_st().redraw_cmdline.set(true);
        return GOTO_NORMAL_MODE;
    }
    CMDLINE_CHANGED
}

/// Handle the CTRL-^ key in the command-line mode and toggle the use of the
/// language `:lmap` mappings and/or Input Method.
fn command_line_toggle_langmap(s: &mut CommandLineState) {
    let use_insert_opt = if buf_valid(s.b_im_ptr_buf.opt()) {
        s.b_im_use_insert
    } else {
        None
    };
    if map_to_exists_mode(c"", MODE_LANGMAP, false) {
        g_st()
            .state_global
            .set(g_st().state_global.get() ^ (MODE_LANGMAP));
        if let Some(use_insert) = use_insert_opt {
            let val = if (g_st().state_global.get() & MODE_LANGMAP) != 0 {
                B_IMODE_LMAP
            } else {
                B_IMODE_NONE
            };
            let buf = s.b_im_ptr_buf.borrow();
            if use_insert {
                buf.b_p_iminsert.set(val);
            } else {
                buf.b_p_imsearch.set(val);
            }
        }
    }

    if let Some(use_insert) = use_insert_opt {
        if use_insert && s.b_im_ptr_buf == g_st().curbuf.get() {
            set_iminsert_global(&curbuf());
        } else {
            set_imsearch_global(&curbuf());
        }
    }
    ui_cursor_shape();
    status_redraw_curbuf();
}

/// Handle the CTRL-R key in the command-line mode and insert the contents of a
/// numbered or named register.
fn command_line_insert_reg(s: &mut CommandLineState) -> i32 {
    let save_new_cmdpos = GETLN_STATE.with(|s| s.new_cmdpos.get());

    putcmdline(b'"', true);
    g_st().no_mapping.set(g_st().no_mapping.get() + (1));
    g_st().allow_keys.set(g_st().allow_keys.get() + (1));
    s.c = plain_vgetc();
    let mut i = s.c;
    if i == CTRL_O {
        i = CTRL_R_CONST;
    }

    if i == CTRL_R_CONST {
        s.c = plain_vgetc();
    }
    g_st().no_mapping.set(g_st().no_mapping.get() - (1));
    g_st().allow_keys.set(g_st().allow_keys.get() - (1));
    GETLN_STATE.with(|s| s.new_cmdpos.set(-1));
    if s.c == b'=' as i32 {
        if get_cmdline_info().cmdfirstc.get() == b'=' as i32 || g_st().cmdline_star.get() > 0 {
            beep_flush();
            s.c = ESC;
        } else {
            s.c = get_expr_register();
        }
    }

    let mut literally = false;
    if s.c != ESC {
        literally = i == CTRL_R_CONST || is_literal_register(s.c);
        cmdline_paste(s.c, literally, false);

        if aborting() {
            s.gotesc = true;
            return GOTO_NORMAL_MODE;
        }
        g_st().key_typed.set(false);
        if GETLN_STATE.with(|s| s.new_cmdpos.get()) >= 0 {
            get_cmdline_info().cmdpos.set(
                get_cmdline_info()
                    .cmdlen
                    .get()
                    .min(GETLN_STATE.with(|s| s.new_cmdpos.get())),
            );
        }
    }
    GETLN_STATE.with(|s| s.new_cmdpos.set(save_new_cmdpos));

    get_cmdline_info().special_char.set(0);
    redrawcmd();

    if literally {
        CMDLINE_CHANGED
    } else {
        CMDLINE_NOT_CHANGED
    }
}

/// Handle the Left and Right mouse clicks in the command-line mode.
fn command_line_left_right_mouse(s: &mut CommandLineState) {
    s.ignore_drag_release = s.c == K_LEFTRELEASE || s.c == K_RIGHTRELEASE;

    get_cmdline_info().cmdspos.set(cmd_startcol());
    get_cmdline_info().cmdpos.set(0);
    while get_cmdline_info().cmdpos.get() < get_cmdline_info().cmdlen.get() {
        let cells = cmdline_charsize(get_cmdline_info().cmdpos.get());
        if g_st().mouse_row.get()
            <= g_st().cmdline_row.get()
                + get_cmdline_info().cmdspos.get() / g_st().columns_global.get()
            && g_st().mouse_col.get()
                < get_cmdline_info().cmdspos.get() % g_st().columns_global.get() + cells
        {
            break;
        }

        let mut cmdspos = get_cmdline_info().cmdspos.get();
        correct_screencol(get_cmdline_info().cmdpos.get(), cells, &mut cmdspos);
        get_cmdline_info().cmdspos.set(cmdspos);
        get_cmdline_info().cmdpos.set(
            get_cmdline_info().cmdpos.get()
                + (utfc_ptr2len(
                    get_cmdline_info()
                        .cmdbuff()
                        .wrapping_add(get_cmdline_info().cmdpos.get() as usize),
                ) - 1),
        );
        get_cmdline_info()
            .cmdspos
            .set(get_cmdline_info().cmdspos.get() + (cells));
        get_cmdline_info()
            .cmdpos
            .set(get_cmdline_info().cmdpos.get() + (1));
    }
}

fn command_line_next_histidx(s: &mut CommandLineState, next_match: bool) {
    let hisidx = get_hisidx(s.histype);
    let hislen = get_hislen();
    with_histentry(s.histype, |hist| {
        loop {
            if !next_match {
                if s.hiscnt == hislen {
                    s.hiscnt = hisidx;
                } else if s.hiscnt == 0 && hisidx != hislen - 1 {
                    s.hiscnt = hislen - 1;
                } else if s.hiscnt != hisidx + 1 {
                    s.hiscnt -= 1;
                } else {
                    s.hiscnt = s.save_hiscnt;
                    break;
                }
            } else {
                if s.hiscnt == hisidx {
                    s.hiscnt = hislen;
                    break;
                }

                if s.hiscnt == hislen {
                    break;
                }

                if s.hiscnt == hislen - 1 {
                    s.hiscnt = 0;
                } else {
                    s.hiscnt += 1;
                }
            }

            if s.hiscnt < 0 || hist[s.hiscnt as usize].hisstr.is_none() {
                s.hiscnt = s.save_hiscnt;
                break;
            }

            let lookfor_bytes = s.lookfor.as_ref().map_or(&b""[..], |lf| lf.as_bytes());
            let his_bytes = hist[s.hiscnt as usize]
                .hisstr_cstr()
                .map_or(&b""[..], CStr::to_bytes);
            let lflen = s.lookforlen as usize;
            if (s.c != K_UP && s.c != K_DOWN)
                || s.hiscnt == s.save_hiscnt
                || his_bytes[..lflen.min(his_bytes.len())]
                    == lookfor_bytes[..lflen.min(lookfor_bytes.len())]
            {
                break;
            }
        }
    });
}

/// Handle the Up, Down, Page Up, Page down, CTRL-N and CTRL-P key in the
/// command-line mode.
fn command_line_browse_history(s: &mut CommandLineState) -> i32 {
    if s.histype == HIST_INVALID || get_hislen() == 0 || s.firstc == NUL as i32 {
        return CMDLINE_NOT_CHANGED;
    }

    s.save_hiscnt = s.hiscnt;

    if s.lookfor.is_none() {
        let prefix_len = get_cmdline_info().cmdpos.get() as usize;
        let cmdbuff = get_cmdline_info().cmdbuff();
        let bytes = cmdbuff.slice(prefix_len);
        s.lookfor = Some(std::ffi::CString::new(bytes).unwrap_or_default());
        s.lookforlen = get_cmdline_info().cmdpos.get();
    }

    let next_match = s.c == K_DOWN
        || s.c == K_S_DOWN
        || s.c == CTRL_N
        || s.c == K_PAGEDOWN
        || s.c == K_KPAGEDOWN;
    command_line_next_histidx(s, next_match);

    if s.hiscnt != s.save_hiscnt {
        let old_firstc: i32;

        dealloc_cmdbuff();

        with_ccline_xpc(|xpc| xpc.xp_context = EXPAND_NOTHING);
        let entry_owned;
        let (p_cstr, plen, is_lookfor, sep) = if s.hiscnt == get_hislen() {
            (s.lookfor.as_deref().unwrap_or(c""), s.lookforlen, true, 0u8)
        } else {
            entry_owned = with_histentry(s.histype, |hist| hist[s.hiscnt as usize].clone());
            (
                entry_owned.hisstr_cstr().unwrap_or(c""),
                entry_owned.hisstrlen() as i32,
                false,
                entry_owned.sep,
            )
        };

        if s.histype == HIST_SEARCH && !is_lookfor && {
            old_firstc = sep as i32;
            old_firstc != s.firstc
        } {
            let mut len: i32 = 0;
            for i in 0..=1 {
                len = 0;
                let p_bytes = p_cstr.to_bytes();
                for (j, &pj) in p_bytes.iter().enumerate() {
                    if i32::from(pj) == old_firstc && (j == 0 || p_bytes[j - 1] != b'\\') {
                        if i > 0 {
                            get_cmdline_info().cmdbuff_vec.borrow_mut()[len as usize] =
                                s.firstc as u8;
                        }
                    } else {
                        if i32::from(pj) == s.firstc && (j == 0 || p_bytes[j - 1] != b'\\') {
                            if i > 0 {
                                get_cmdline_info().cmdbuff_vec.borrow_mut()[len as usize] = b'\\';
                            }
                            len += 1;
                        }
                        if i > 0 {
                            get_cmdline_info().cmdbuff_vec.borrow_mut()[len as usize] = pj;
                        }
                    }
                    len += 1;
                }

                if i == 0 {
                    alloc_cmdbuff(len);
                }
            }
            get_cmdline_info().cmdbuff_vec.borrow_mut()[len as usize] = 0;
            get_cmdline_info().cmdlen.set(len);
            get_cmdline_info().cmdpos.set(len);
        } else {
            alloc_cmdbuff(plen);
            let p_bytes = p_cstr.to_bytes_with_nul();
            get_cmdline_info().cmdbuff_vec.borrow_mut()[..p_bytes.len()].copy_from_slice(p_bytes);
            get_cmdline_info().cmdlen.set(plen);
            get_cmdline_info().cmdpos.set(plen);
        }

        redrawcmd();
        return CMDLINE_CHANGED;
    }
    beep_flush();
    CMDLINE_NOT_CHANGED
}

fn command_line_handle_key(s: &mut CommandLineState) -> i32 {
    if !(get_cmdline_info().one_key.get() && s.c != ESC && s.c != CTRL_C_CONST) {
        'key_switch: {
            match s.c {
                K_BS | CTRL_H_CONST | K_DEL | K_KDEL | CTRL_W_CONST => {
                    return match command_line_erase_chars(s) {
                        CMDLINE_NOT_CHANGED => command_line_not_changed(s),
                        GOTO_NORMAL_MODE => 0,
                        _ => command_line_changed(s),
                    };
                }

                K_INS | K_KINS => {
                    get_cmdline_info().overstrike.set(
                        if get_cmdline_info().overstrike.get() != 0 {
                            0
                        } else {
                            1
                        },
                    );
                    ui_cursor_shape();
                    may_trigger_modechanged();
                    status_redraw_curbuf();
                    redraw_statuslines();
                    return command_line_not_changed(s);
                }

                CTRL_HAT_CONST => {
                    command_line_toggle_langmap(s);
                    return command_line_not_changed(s);
                }

                CTRL_U => {
                    let j = get_cmdline_info().cmdpos.get() as usize;
                    let old_len = get_cmdline_info().cmdlen.get() as usize;
                    get_cmdline_info()
                        .cmdlen
                        .set(get_cmdline_info().cmdlen.get() - (get_cmdline_info().cmdpos.get()));
                    get_cmdline_info().cmdpos.set(0);
                    let new_len = get_cmdline_info().cmdlen.get() as usize;
                    get_cmdline_info()
                        .cmdbuff_vec
                        .borrow_mut()
                        .copy_within(j..old_len, 0);
                    get_cmdline_info().cmdbuff_vec.borrow_mut()[new_len] = NUL;

                    if get_cmdline_info().cmdlen.get() == 0 {
                        s.is_state.search_start = s.is_state.save_cursor;
                    }
                    redrawcmd();
                    return command_line_changed(s);
                }

                ESC | CTRL_C_CONST => {
                    if GETLN_STATE.with(|s| s.getln_interrupted_highlight.get())
                        && s.c == CTRL_C_CONST
                    {
                        GETLN_STATE.with(|s| s.getln_interrupted_highlight.set(false));
                        return command_line_not_changed(s);
                    }

                    s.gotesc = true;
                    return 0;
                }

                CTRL_R_CONST => {
                    return match command_line_insert_reg(s) {
                        GOTO_NORMAL_MODE => 0,
                        CMDLINE_CHANGED => command_line_changed(s),
                        _ => command_line_not_changed(s),
                    };
                }

                CTRL_D_CONST => {
                    if with_ccline_xpc(|xpc| {
                        showmatches(
                            xpc,
                            false,
                            true,
                            if opt_vars().p_wmnu.get() {
                                g_st().wim_flags[0].get() as i32
                            } else {
                                0
                            },
                        )
                    }) == EXPAND_NOTHING
                    {
                        break 'key_switch;
                    }

                    redrawcmd();
                    return 1;
                }

                K_RIGHT | K_S_RIGHT | K_C_RIGHT => {
                    loop {
                        if get_cmdline_info().cmdpos.get() >= get_cmdline_info().cmdlen.get() {
                            break;
                        }

                        let cells = cmdline_charsize(get_cmdline_info().cmdpos.get());
                        if g_st().key_typed.get()
                            && get_cmdline_info().cmdspos.get() + cells
                                >= g_st().columns_global.get() * g_st().rows_global.get()
                        {
                            break;
                        }

                        get_cmdline_info()
                            .cmdspos
                            .set(get_cmdline_info().cmdspos.get() + (cells));
                        get_cmdline_info().cmdpos.set(
                            get_cmdline_info().cmdpos.get()
                                + (utfc_ptr2len(
                                    get_cmdline_info()
                                        .cmdbuff()
                                        .wrapping_add(get_cmdline_info().cmdpos.get() as usize),
                                )),
                        );

                        if !((s.c == K_S_RIGHT
                            || s.c == K_C_RIGHT
                            || (g_st().mod_mask.get() & ((MOD_MASK_SHIFT | MOD_MASK_CTRL) as i32))
                                != 0)
                            && get_cmdline_info().cmdbuff_vec.borrow()
                                [get_cmdline_info().cmdpos.get() as usize]
                                != b' ')
                        {
                            break;
                        }
                    }
                    get_cmdline_info()
                        .cmdspos
                        .set(cmd_screencol(get_cmdline_info().cmdpos.get()));
                    return command_line_not_changed(s);
                }

                K_LEFT | K_S_LEFT | K_C_LEFT => {
                    if get_cmdline_info().cmdpos.get() == 0 {
                        return command_line_not_changed(s);
                    }
                    loop {
                        get_cmdline_info()
                            .cmdpos
                            .set(get_cmdline_info().cmdpos.get() - (1));
                        get_cmdline_info().cmdpos.set(
                            get_cmdline_info().cmdpos.get()
                                - (crate::mbyte::utf_head_off(
                                    &get_cmdline_info().cmdbuff_vec.borrow(),
                                    get_cmdline_info().cmdpos.get() as usize,
                                )),
                        );
                        get_cmdline_info().cmdspos.set(
                            get_cmdline_info().cmdspos.get()
                                - (cmdline_charsize(get_cmdline_info().cmdpos.get())),
                        );

                        if !(get_cmdline_info().cmdpos.get() > 0
                            && (s.c == K_S_LEFT
                                || s.c == K_C_LEFT
                                || (g_st().mod_mask.get()
                                    & ((MOD_MASK_SHIFT | MOD_MASK_CTRL) as i32))
                                    != 0)
                            && get_cmdline_info().cmdbuff_vec.borrow()
                                [(get_cmdline_info().cmdpos.get() - 1) as usize]
                                != b' ')
                        {
                            break;
                        }
                    }

                    get_cmdline_info()
                        .cmdspos
                        .set(cmd_screencol(get_cmdline_info().cmdpos.get()));
                    if get_cmdline_info().special_char.get() != 0 {
                        putcmdline(
                            get_cmdline_info().special_char.get(),
                            get_cmdline_info().special_shift.get(),
                        );
                    }

                    return command_line_not_changed(s);
                }

                K_IGNORE | K_MIDDLEDRAG | K_MIDDLERELEASE => {
                    return command_line_not_changed(s);
                }

                K_MIDDLEMOUSE => {
                    cmdline_paste(
                        if eval_has_provider(b"clipboard", false) {
                            b'*' as i32
                        } else {
                            0
                        },
                        true,
                        true,
                    );
                    redrawcmd();
                    return command_line_changed(s);
                }

                K_LEFTDRAG | K_LEFTRELEASE | K_RIGHTDRAG | K_RIGHTRELEASE => {
                    if s.ignore_drag_release {
                        return command_line_not_changed(s);
                    }
                    if { get_cmdline_info().mouse_used.get() }.is_some()
                        && g_st().mouse_row.get() < g_st().cmdline_row.get()
                    {
                        get_cmdline_info().mouse_used.set(Some(true));
                        return 0;
                    }
                    command_line_left_right_mouse(s);
                    return command_line_not_changed(s);
                }

                K_LEFTMOUSE => {
                    if { get_cmdline_info().mouse_used.get() }.is_some()
                        && g_st().mouse_row.get() < g_st().cmdline_row.get()
                    {
                        get_cmdline_info().mouse_used.set(Some(true));
                        return 0;
                    }
                    command_line_left_right_mouse(s);
                    return command_line_not_changed(s);
                }

                K_RIGHTMOUSE => {
                    command_line_left_right_mouse(s);
                    return command_line_not_changed(s);
                }

                K_MOUSEDOWN | K_MOUSEUP | K_MOUSELEFT | K_MOUSERIGHT => {
                    if cmdline_pum_active() {
                        cmdline_mousescroll(if s.c == K_MOUSEDOWN {
                            MSCR_DOWN
                        } else if s.c == K_MOUSEUP {
                            MSCR_UP
                        } else if s.c == K_MOUSELEFT {
                            MSCR_LEFT
                        } else {
                            MSCR_RIGHT
                        });
                    }
                    return command_line_not_changed(s);
                }

                K_X1MOUSE | K_X1DRAG | K_X1RELEASE | K_X2MOUSE | K_X2DRAG | K_X2RELEASE
                | K_MOUSEMOVE | K_SELECT => {
                    return command_line_not_changed(s);
                }

                CTRL_B_CONST | K_HOME | K_KHOME | K_S_HOME | K_C_HOME => {
                    get_cmdline_info().cmdpos.set(0);
                    get_cmdline_info().cmdspos.set(cmd_startcol());
                    return command_line_not_changed(s);
                }

                CTRL_E | K_END | K_KEND | K_S_END | K_C_END => {
                    get_cmdline_info()
                        .cmdpos
                        .set(get_cmdline_info().cmdlen.get());
                    get_cmdline_info()
                        .cmdspos
                        .set(cmd_screencol(get_cmdline_info().cmdpos.get()));
                    return command_line_not_changed(s);
                }

                CTRL_A => {
                    if cmdline_pum_active() {
                        cmdline_pum_cleanup(&get_cmdline_info());
                    }

                    if !with_ccline_xpc(|xpc| nextwild(xpc, WILD_ALL, 0, s.firstc != b'@' as i32)) {
                        break 'key_switch;
                    }
                    with_ccline_xpc(|xpc| xpc.xp_context = EXPAND_NOTHING);
                    s.did_wild_list = false;
                    return command_line_changed(s);
                }

                CTRL_L_CONST => {
                    if may_add_char_to_search(s.firstc, &mut s.c, &mut s.is_state) {
                        return command_line_not_changed(s);
                    }

                    if !with_ccline_xpc(|xpc| {
                        nextwild(xpc, WILD_LONGEST, 0, s.firstc != b'@' as i32)
                    }) {
                        break 'key_switch;
                    }
                    return command_line_changed(s);
                }

                CTRL_N | CTRL_P => {
                    if with_ccline_xpc(|xpc| xpc.xp_numfiles > 0) {
                        let wild_type = if s.c == CTRL_P { WILD_PREV } else { WILD_NEXT };
                        if !with_ccline_xpc(|xpc| {
                            nextwild(xpc, wild_type, 0, s.firstc != b'@' as i32)
                        }) {
                            break 'key_switch;
                        }
                        return command_line_changed(s);
                    }
                    match command_line_browse_history(s) {
                        CMDLINE_CHANGED => {
                            s.did_hist_navigate = true;
                            return command_line_changed(s);
                        }
                        GOTO_NORMAL_MODE => return 0,
                        _ => return command_line_not_changed(s),
                    }
                }

                K_UP | K_DOWN | K_S_UP | K_S_DOWN | K_PAGEUP | K_KPAGEUP | K_PAGEDOWN
                | K_KPAGEDOWN => {
                    if cmdline_pum_active()
                        && (s.c == K_PAGEUP
                            || s.c == K_PAGEDOWN
                            || s.c == K_KPAGEUP
                            || s.c == K_KPAGEDOWN)
                    {
                        let wild_type = if s.c == K_PAGEDOWN || s.c == K_KPAGEDOWN {
                            WILD_PAGEDOWN
                        } else {
                            WILD_PAGEUP
                        };
                        if !with_ccline_xpc(|xpc| {
                            nextwild(xpc, wild_type, 0, s.firstc != b'@' as i32)
                        }) {
                            break 'key_switch;
                        }
                        return command_line_changed(s);
                    } else {
                        match command_line_browse_history(s) {
                            CMDLINE_CHANGED => {
                                s.did_hist_navigate = true;
                                return command_line_changed(s);
                            }
                            GOTO_NORMAL_MODE => return 0,
                            _ => return command_line_not_changed(s),
                        }
                    }
                }

                CTRL_G | CTRL_T_CONST => {
                    if !may_do_command_line_next_incsearch(
                        s.firstc,
                        s.count,
                        &mut s.is_state,
                        s.c == CTRL_G,
                    ) {
                        return command_line_not_changed(s);
                    }
                }

                CTRL_V_CONST | CTRL_Q_CONST => {
                    s.ignore_drag_release = true;
                    putcmdline(b'^', true);

                    s.c = get_literal((g_st().mod_mask.get() & (MOD_MASK_SHIFT as i32)) != 0);

                    s.do_abbr = false;
                    get_cmdline_info().special_char.set(0);
                    if utf_iscomposing_first(s.c) && !g_st().cmd_silent.get() {
                        if ui_has(K_UICMDLINE) {
                            unputcmdline();
                        } else {
                            draw_cmdline(
                                get_cmdline_info().cmdpos.get(),
                                get_cmdline_info().cmdlen.get() - get_cmdline_info().cmdpos.get(),
                            );
                            msg_putchar(b' ' as i32);
                            cursorcmd();
                        }
                    }
                }

                CTRL_K_CONST => {
                    s.ignore_drag_release = true;
                    putcmdline(b'?', true);
                    s.c = get_digraph(true);
                    get_cmdline_info().special_char.set(0);

                    if s.c != NUL as i32 {
                        break 'key_switch;
                    }

                    redrawcmd();
                    return command_line_not_changed(s);
                }

                CTRL_CONST_CONST => {
                    if !opt_vars().p_ari.get() {
                        break 'key_switch;
                    }
                    return command_line_not_changed(s);
                }

                c if c == b'q' as i32 => {
                    if { get_cmdline_info().mouse_used.get() }.is_some() {
                        get_cmdline_info().cmdbuff_vec.borrow_mut()[0] = NUL;

                        return 0;
                    }
                    if !is_special(s.c) {
                        g_st().mod_mask.set(0x0);
                    }
                }

                _ => {
                    if !is_special(s.c) {
                        g_st().mod_mask.set(0x0);
                    }
                }
            }
        }

        if s.do_abbr
            && (is_special(s.c) || !vim_iswordc(s.c))
            && (ccheck_abbr(if s.c >= 0x100 { s.c + ABBR_OFF } else { s.c }) != 0
                || s.c == CTRL_RSB_CONST)
        {
            return command_line_changed(s);
        }
    }

    // end:
    if is_special(s.c) || g_st().mod_mask.get() != 0 {
        put_on_cmdline(
            get_special_key_name(s.c, g_st().mod_mask.get()).to_bytes(),
            -1,
            true,
        );
    } else {
        let mut char_buf = [0u8; crate::cmdexpand::MB_MAXBYTES + 1];
        let j = utf_char2bytes(s.c, &mut char_buf);
        put_on_cmdline(&char_buf[..j as usize], j, true);
    }
    if get_cmdline_info().one_key.get() {
        0
    } else {
        command_line_changed(s)
    }
}

/// Trigger `CursorMovedC` autocommands.
fn may_trigger_cursormovedc(s: &mut CommandLineState) {
    if get_cmdline_info().cmdpos.get() != s.prev_cmdpos {
        trigger_cmd_autocmd(s.cmdline_type, EVENT_CURSORMOVEDC);
        get_cmdline_info()
            .redraw_state
            .set(get_cmdline_info().redraw_state.get().max(K_CMD_REDRAW_POS));
    }
}

fn command_line_not_changed(s: &mut CommandLineState) -> i32 {
    may_trigger_cursormovedc(s);
    s.prev_cmdpos = get_cmdline_info().cmdpos.get();
    if !s.is_state.incsearch_postponed {
        return 1;
    }
    command_line_changed(s)
}

/// Guess that the pattern matches everything.
fn empty_pattern(p: &[u8], delim: i32) -> bool {
    let slice = crate::memory::bytes_until_nul(p);
    let mut magic_val: RegMagic = MAGIC_ON;

    if !slice.is_empty() {
        skip_regexp_magic(
            slice,
            delim,
            i32::from(opt_vars().p_magic.get()),
            Some(&mut magic_val),
        );
    } else {
        return true;
    }

    empty_pattern_magic(slice, magic_val)
}

fn empty_pattern_magic(p: &[u8], magic_val: RegMagic) -> bool {
    let mut slice = crate::memory::bytes_until_nul(p);
    while slice.len() >= 2
        && slice[slice.len() - 2] == b'\\'
        && matches!(
            slice[slice.len() - 1],
            b'm' | b'M' | b'v' | b'V' | b'c' | b'C' | b'Z'
        )
    {
        slice = &slice[..slice.len() - 2];
    }

    slice.is_empty()
        || (slice.len() > 1
            && slice[slice.len() - 1] == b'|'
            && ((slice[slice.len() - 2] == b'\\' && magic_val == MAGIC_ON)
                || (slice[slice.len() - 2] != b'\\' && magic_val == MAGIC_ALL)))
}

pub fn cmdpreview_get_bufnr() -> ObjHandle {
    GETLN_STATE.with(|s| s.cmdpreview_bufnr.get())
}

pub fn cmdpreview_get_ns() -> i32 {
    GETLN_STATE.with(|s| s.cmdpreview_ns.get())
}

/// Sets up command preview buffer.
fn cmdpreview_open_buf() -> Option<crate::globals::BufPtr> {
    let mut cmdpreview_buf = if GETLN_STATE.with(|s| s.cmdpreview_bufnr.get()) != 0 {
        crate::globals::BufPtr::from(buflist_findnr(
            GETLN_STATE.with(|s| s.cmdpreview_bufnr.get()),
        ))
    } else {
        crate::globals::BufPtr::NULL
    };
    if cmdpreview_buf.is_null() {
        let mut err = error_init();
        let bufnr = nvim_create_buf(false, true, &mut err);

        if error_set(&err) {
            return None;
        }

        cmdpreview_buf = crate::globals::BufPtr::from(buflist_findnr(bufnr));
    }

    if cmdpreview_buf == g_st().curbuf.get() {
        return None;
    }

    let mut aco: CtxSwitch = CtxSwitch::new();
    ctx_switch(&mut aco, None, None, cmdpreview_buf.opt(), K_CTX_KEEP_DIRS);
    let retv = rename_buffer(Some(c"[Preview]"));
    ctx_restore(&mut aco);

    if !retv {
        return None;
    }

    ctx_switch(&mut aco, None, None, cmdpreview_buf.opt(), K_CTX_KEEP_DIRS);
    buf_clear();
    curbuf().b_p_ma.set(true);
    curbuf().b_p_ul.set(-1);
    curbuf().b_p_tw.set(0);
    ctx_restore(&mut aco);
    if cmdpreview_buf.is_null() {
        return None;
    }
    GETLN_STATE.with(|s| s.cmdpreview_bufnr.set(cmdpreview_buf.b_fnum.get()));

    Some(cmdpreview_buf)
}

/// Open command preview window if it's not already open.
fn cmdpreview_open_win(cmdpreview_buf: &FileBuf) -> Option<crate::globals::WinPtr> {
    let save_curwin = curwin_ptr();

    if !win_split(opt_vars().p_cwh.get() as i32, WSP_BOT) {
        return None;
    }

    let preview_win = curwin_ptr();
    let mut err = error_init();

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let result = do_buffer(
        DOBUF_GOTO,
        DOBUF_FIRST,
        FORWARD,
        cmdpreview_buf.b_fnum.get(),
        0,
    );
    try_leave(&tstate, &mut err);
    if error_set(&err) || !result {
        api_clear_error(&mut err);
        return None;
    }

    curwin().w_onebuf_opt.wo_cul.set(false);
    curwin().w_onebuf_opt.wo_cuc.set(false);
    curwin().w_onebuf_opt.wo_spell.set(false);
    curwin().w_onebuf_opt.wo_fen.set(false);

    win_enter(save_curwin.borrow(), false);
    (!preview_win.is_null()).then_some(preview_win)
}

/// Closes any open command preview windows.
fn cmdpreview_close_win() {
    let bufnr = GETLN_STATE.with(|s| s.cmdpreview_bufnr.get());
    if bufnr != 0
        && let Some(buf) = buflist_findnr(bufnr)
    {
        close_windows(Some(&buf), false);
    }
}

/// Save current state and prepare windows and buffers for command preview.
fn cmdpreview_prepare() -> CpInfo {
    let mut saved_bufs: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut cpinfo = CpInfo {
        win_info: Vec::new(),
        buf_info: Vec::new(),
        save_hls: false,
        save_cmdmod: Cmdmod::new(),
        save_view: Vec::new(),
    };

    for win in for_all_windows_ptr() {
        let buf = win.w_buffer.get();

        if buf.b_fnum.get() != GETLN_STATE.with(|s| s.cmdpreview_bufnr.get()) {
            if saved_bufs.insert(buf.addr()) {
                let mut cp_bufinfo = CpBufInfo::new();
                cp_bufinfo.buf = buf.clone();
                cp_bufinfo.save_b_p_ma = buf.b_p_ma.get();
                cp_bufinfo.save_b_changed = buf.b_changed.get();
                cp_bufinfo.save_b_op_start = buf.b_op_start.get();
                cp_bufinfo.save_b_op_end = buf.b_op_end.get();
                u_checkpoint(&mut cp_bufinfo.undo_checkpoint, &buf);
                cpinfo.buf_info.push(cp_bufinfo);
            }

            let mut cp_wininfo = CpWinInfo::new();
            cp_wininfo.win = win.clone();
            cp_wininfo.save_w_cursor = win.w_cursor.get();
            save_viewstate(&win, &mut cp_wininfo.save_viewstate);
            cp_wininfo.save_w_p_cul = win.w_onebuf_opt.wo_cul.get();
            cp_wininfo.save_w_p_cuc = win.w_onebuf_opt.wo_cuc.get();

            cpinfo.win_info.push(cp_wininfo);

            win.w_onebuf_opt.wo_cul.set(false);
            win.w_onebuf_opt.wo_cuc.set(false);
        }
    }

    cpinfo.save_hls = opt_vars().p_hls.get();
    cpinfo.save_cmdmod = g_st().cmdmod.take();
    win_size_save(&mut cpinfo.save_view);
    save_search_patterns();

    opt_vars().p_hls.set(false);
    g_st().cmdmod.borrow_mut().cmod_split = 0;
    g_st().cmdmod.borrow_mut().cmod_tab = 0;
    g_st().cmdmod.borrow_mut().cmod_flags |= CMOD_NOSWAPFILE;

    u_sync(true);
    cpinfo
}

/// Restore the state of buffers and windows for command preview.
fn cmdpreview_restore_state(cpinfo: &mut CpInfo) {
    for mut cp_bufinfo in cpinfo.buf_info.drain(..) {
        let buf = cp_bufinfo.buf.borrow();

        buf.b_changed.set(cp_bufinfo.save_b_changed);

        extmark_clear(
            buf,
            GETLN_STATE.with(|s| s.cmdpreview_ns.get()) as u32,
            0,
            0,
            MAXLNUM,
            MAXCOL,
        );

        u_rollback(&mut cp_bufinfo.undo_checkpoint, buf);

        buf.b_op_start.set(cp_bufinfo.save_b_op_start);
        buf.b_op_end.set(cp_bufinfo.save_b_op_end);
        buf.b_p_ma.set(cp_bufinfo.save_b_p_ma);
    }

    for cp_wininfo in cpinfo.win_info.drain(..) {
        let win = cp_wininfo.win.borrow();

        win.w_cursor.set(cp_wininfo.save_w_cursor);
        restore_viewstate(win, &cp_wininfo.save_viewstate);

        win.w_onebuf_opt.wo_cul.set(cp_wininfo.save_w_p_cul);
        win.w_onebuf_opt.wo_cuc.set(cp_wininfo.save_w_p_cuc);

        update_topline(win);
    }

    *g_st().cmdmod.borrow_mut() = std::mem::take(&mut cpinfo.save_cmdmod);
    opt_vars().p_hls.set(cpinfo.save_hls);
    restore_search_patterns();
    win_size_restore(&cpinfo.save_view);
}

/// Show `'inccommand'` preview if command is previewable.
fn cmdpreview_may_show(_s: &mut CommandLineState) -> bool {
    let mut ea: Exarg = Exarg::new();
    let mut cmod: Cmdmod = Cmdmod::new();
    let mut cmdpreview_type: i32;
    ea.cmdline_buf = get_cmdline_info()
        .cmdbuff()
        .as_cstr()
        .to_bytes_with_nul()
        .to_vec();
    let mut errormsg = None;
    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    let parsed = parse_cmdline(&mut ea, &mut cmod, &mut errormsg, false);
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    if !parsed {
        return false;
    }

    if (ea.argt & EX_PREVIEW) == 0 {
        undo_cmdmod(&mut cmod);
        return false;
    }

    cursorcmd();
    cmdline_ui_flush();

    if (ea.argt & EX_RANGE) != 0 && ea.line1 > ea.line2 {
        std::mem::swap(&mut ea.line1, &mut ea.line2);
    }

    let mut icm_split = opt_vars().p_icm.first_byte() == b's';
    let mut cmdpreview_buf = crate::globals::BufPtr::NULL;
    let mut cmdpreview_win = crate::globals::WinPtr::NULL;

    g_st().emsg_silent.set(g_st().emsg_silent.get() + (1));
    g_st().msg_silent.set(g_st().msg_silent.get() + (1));
    block_autocmds();

    let mut cpinfo = cmdpreview_prepare();

    if icm_split {
        cmdpreview_buf = crate::globals::BufPtr::from(cmdpreview_open_buf());
        if cmdpreview_buf.is_null() {
            set_option_direct(K_OPT_INCCOMMAND, cstr_as_obj(Some(c"nosplit")), 0, SID_NONE);
            icm_split = false;
        }
    }
    if GETLN_STATE.with(|s| s.cmdpreview_ns.get()) == 0 {
        GETLN_STATE.with(|s| {
            s.cmdpreview_ns
                .set(nvim_create_namespace(NvimString::new()) as i32)
        });
    }

    g_st().cmdpreview.set(true);

    let mut err = error_init();
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    cmdpreview_type = execute_cmd(&mut ea, &mut cmod, true);
    try_leave(&tstate, &mut err);
    if error_set(&err) {
        api_clear_error(&mut err);
        cmdpreview_type = 0;
    }

    if icm_split && cmdpreview_type == 2 {
        cmdpreview_win = crate::globals::WinPtr::from(cmdpreview_open_win(cmdpreview_buf.borrow()));
        if cmdpreview_win.is_null() {
            cmdpreview_type = 1;
        }
    }

    if cmdpreview_type != 0 {
        let save_rd = g_st().redrawing_disabled.get();
        g_st().redrawing_disabled.set(0);
        update_screen();
        g_st().redrawing_disabled.set(save_rd);
    }

    if icm_split && cmdpreview_type == 2 && !cmdpreview_win.is_null() {
        cmdpreview_close_win();
    }

    cmdpreview_restore_state(&mut cpinfo);

    unblock_autocmds();
    g_st().msg_silent.set(g_st().msg_silent.get() - (1));
    g_st().emsg_silent.set(g_st().emsg_silent.get() - (1));
    redrawcmdline();

    cmdpreview_type != 0
}

/// Trigger `CmdlineChanged` autocommands.
fn do_autocmd_cmdlinechanged(firstc: i32) {
    if has_event(EVENT_CMDLINECHANGED) {
        let mut err = error_init();
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);

        let firstcbuf: [u8; 2] = [firstc as u8, 0];
        let firstc_cstr = crate::memory::bytes_as_cstr(&firstcbuf);

        tv_dict_add_str(&dict, c"cmdtype", Some(firstc_cstr));
        tv_dict_add_nr(
            &dict,
            c"cmdlevel",
            get_cmdline_info().level.get() as Varnumber,
        );
        tv_dict_set_keys_readonly(&dict);
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        apply_autocmds(
            EVENT_CMDLINECHANGED,
            Some(firstc_cstr),
            Some(firstc_cstr),
            false,
            curbuf_opt().as_deref(),
        );
        restore_v_event(&dict, &mut save_v_event);
        try_leave(&tstate, &mut err);
        if error_set(&err) {
            if !ui_has(K_UIMESSAGES) {
                msg_putchar(b'\n' as i32);
            }
            g_st().msg_scroll.set(1);
            msg_puts_hl(err.msg_cstr().unwrap_or(c""), HLF_E, true);
            api_clear_error(&mut err);
            redrawcmd();
        }
    }
}

fn command_line_changed(s: &mut CommandLineState) -> i32 {
    let prev_cmdpreview = g_st().cmdpreview.get();
    if s.firstc == b':' as i32
        && g_st().current_sctx.get().sc_sid == 0
        && !opt_vars().p_icm.is_empty()
        && g_st().cmdline_star.get() == 0
        && vpeekc_any() == 0
        && cmdpreview_may_show(s)
    {
        // 'inccommand' preview has been shown.
    } else {
        g_st().cmdpreview.set(false);
        if prev_cmdpreview {
            update_screen();
        }
        if with_ccline_xpc(|xpc| xpc.xp_context == EXPAND_NOTHING)
            && (g_st().key_typed.get() || vpeekc() == NUL as i32)
        {
            may_do_incsearch_highlighting(s.firstc, s.count, &mut s.is_state);
        }
    }

    if !get_cmdline_info().cmdbuff_replaced.get()
        && (get_cmdline_info().cmdpos.get() != s.prev_cmdpos
            || s.prev_cmdbuff
                .as_ref()
                .is_some_and(|prev| prev.to_bytes() != get_cmdline_info().cmdbuff().as_bytes()))
    {
        do_autocmd_cmdlinechanged(if s.firstc > 0 { s.firstc } else { b'-' as i32 });
    }

    may_trigger_cursormovedc(s);

    if opt_vars().p_arshape.get()
        && !opt_vars().p_tbidi.get()
        && !ui_has(K_UICMDLINE)
        && vpeekc() == NUL as i32
    {
        redrawcmd();
    }

    1
}

/// Abandon the command line.
fn abandon_cmdline() {
    dealloc_cmdbuff();
    if g_st().msg_scrolled.get() == 0 {
        compute_cmdrow();
    }
    if !get_cmdline_info().one_key.get() {
        msg(c"", 0);
        g_st().redraw_cmdline.set(true);
    }
}

/// `getcmdline()` - accept a command line starting with `firstc`.
pub fn getcmdline(
    firstc: i32,
    count: i32,
    indent: i32,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    command_line_enter(firstc, count, indent, true)
}

/// Get a command line with a prompt.
pub fn getcmdline_prompt(
    firstc: i32,
    prompt: Option<&CStr>,
    hl_id: i32,
    xp_context: i32,
    xp_arg: Option<&CStr>,
    highlight_callback: Callback,
    one_key: bool,
    mouse_used: Option<&mut bool>,
) -> Option<std::ffi::CString> {
    let msg_col_save = g_st().msg_col.get();

    let mut did_save_ccline = false;
    if !get_cmdline_info().cmdbuff().is_null() {
        save_cmdline();
        did_save_ccline = true;
    } else {
        CCLINE.with(|c| *c.borrow_mut() = std::rc::Rc::new(CmdlineInfo::new()));
    }
    get_cmdline_info()
        .prompt_id
        .set(GETLN_STATE.with(|s| s.last_prompt_id.get()));
    GETLN_STATE.with(|s| {
        s.last_prompt_id
            .set(GETLN_STATE.with(|s| s.last_prompt_id.get()).wrapping_add(1))
    });
    *get_cmdline_info().cmdprompt.borrow_mut() = prompt.map(CStr::to_owned);
    get_cmdline_info().hl_id.set(hl_id);
    get_cmdline_info().xp_context.set(xp_context);
    *get_cmdline_info().xp_arg.borrow_mut() = xp_arg.map(CStr::to_owned);
    get_cmdline_info()
        .input_fn
        .set(i32::from(firstc == b'@' as i32));
    *get_cmdline_info().highlight_callback.borrow_mut() = highlight_callback;
    get_cmdline_info().one_key.set(one_key);
    get_cmdline_info()
        .mouse_used
        .set(mouse_used.is_some().then_some(false));

    let cmd_silent_saved = g_st().cmd_silent.get();
    let msg_silent_saved = g_st().msg_silent.get();
    g_st().msg_silent.set(0);
    g_st().cmd_silent.set(false);

    let ret = command_line_enter(firstc, 1, 0, false);
    if let Some(mouse_used_ref) = mouse_used {
        *mouse_used_ref = get_cmdline_info().mouse_used.get().unwrap_or(false);
    }
    get_cmdline_info().redraw_state.set(K_CMD_REDRAW_NONE);
    *get_cmdline_info().highlight_callback.borrow_mut() = Callback::NONE;

    if did_save_ccline {
        restore_cmdline();
    }
    g_st().msg_silent.set(msg_silent_saved);
    g_st().cmd_silent.set(cmd_silent_saved);
    if !get_cmdline_info().cmdbuff().is_null() {
        g_st().msg_col.set(msg_col_save);
    }

    ret
}

/// Parse `'wildmode'`, updating `flags` when the value is valid.
pub fn check_opt_wim(value: &[u8], flags: &mut [u8; 4]) -> bool {
    let mut rem = crate::memory::bytes_until_nul(value);
    let mut new_wim_flags: [u8; 4] = [0; 4];
    let mut idx: usize = 0;

    while !rem.is_empty() {
        let i = rem.iter().take_while(|b| b.is_ascii_alphabetic()).count();
        if let Some(&sep) = rem.get(i)
            && sep != b','
            && sep != b':'
        {
            return false;
        }
        match &rem[..i] {
            b"longest" => new_wim_flags[idx] |= K_OPT_WIM_FLAG_LONGEST,
            b"full" => new_wim_flags[idx] |= K_OPT_WIM_FLAG_FULL,
            b"list" => new_wim_flags[idx] |= K_OPT_WIM_FLAG_LIST,
            b"lastused" => new_wim_flags[idx] |= K_OPT_WIM_FLAG_LASTUSED,
            b"noselect" => new_wim_flags[idx] |= K_OPT_WIM_FLAG_NOSELECT,
            b"noinsert" => new_wim_flags[idx] |= K_OPT_WIM_FLAG_NOINSERT,
            _ => return false,
        }
        rem = &rem[i..];
        if rem.is_empty() {
            break;
        }
        if rem[0] == b',' {
            if idx == 3 {
                return false;
            }
            idx += 1;
        }
        rem = &rem[1..];
    }

    while idx < 3 {
        new_wim_flags[idx + 1] = new_wim_flags[idx];
        idx += 1;
    }

    *flags = new_wim_flags;
    true
}

/// Return true when the text must not be changed and we can't switch to
/// another window or buffer.
pub fn text_locked() -> bool {
    if expr_map_locked() {
        return true;
    }
    g_st().textlock.get() != 0
}

pub fn text_locked_msg() {
    emsg(gettext(get_text_locked_msg()));
}

pub const fn get_text_locked_msg() -> &'static std::ffi::CStr {
    E_TEXTLOCK
}

/// Check for text, window or buffer locked.
pub fn text_or_buf_locked() -> bool {
    if text_locked() {
        text_locked_msg();
        return true;
    }
    curbuf_locked()
}

/// Check if `curbuf->b_ro_locked` or `allbuf_lock` is set.
pub fn curbuf_locked() -> bool {
    if curbuf().b_ro_locked.get() > 0 {
        emsg(gettext(E_CANNOT_EDIT_OTHER_BUF));
        return true;
    }
    allbuf_locked()
}

pub fn allbuf_locked() -> bool {
    if g_st().allbuf_lock.get() > 0 {
        emsg(gettext(
            c"E811: Not allowed to change buffer information now",
        ));
        return true;
    }
    false
}

fn cmdline_charsize(idx: i32) -> i32 {
    if g_st().cmdline_star.get() > 0 {
        return 1;
    }
    ptr2cells(get_cmdline_info().cmdbuff().wrapping_add(idx as usize))
}

/// Compute the offset of the cursor on the command line for the prompt and
/// indent.
fn cmd_startcol() -> i32 {
    get_cmdline_info().cmdindent.get()
        + if get_cmdline_info().cmdfirstc.get() != NUL as i32 {
            1
        } else {
            0
        }
}

/// Compute the column position for a byte position on the command line.
pub fn cmd_screencol(bytepos: i32) -> i32 {
    let mut m: i32;
    let mut col = cmd_startcol();
    if g_st().key_typed.get() {
        m = if !g_st().cmdline_win.get().is_null() {
            let cw = g_st().cmdline_win.get();
            cw.w_view_width.get() * cw.w_view_height.get()
        } else {
            g_st().columns_global.get() * g_st().rows_global.get()
        };
        if m < 0 {
            m = MAXCOL;
        }
    } else {
        m = MAXCOL;
    }

    let mut i: i32 = 0;
    while i < get_cmdline_info().cmdlen.get() && i < bytepos {
        let c = cmdline_charsize(i);
        correct_screencol(i, c, &mut col);

        col += c;
        if col >= m {
            col -= c;
            break;
        }
        i += utfc_ptr2len(get_cmdline_info().cmdbuff().wrapping_add(i as usize));
    }
    col
}

/// Check if the character at `idx`, which is `cells` wide, is a multi-byte
/// character that doesn't fit, so that a `>` must be displayed.
fn correct_screencol(idx: i32, cells: i32, col: &mut i32) {
    if utfc_ptr2len(get_cmdline_info().cmdbuff().wrapping_add(idx as usize)) > 1
        && utf_ptr2cells(get_cmdline_info().cmdbuff().wrapping_add(idx as usize)) > 1
        && (*col) % g_st().columns_global.get() + cells > g_st().columns_global.get()
    {
        *col += 1;
    }
}

/// Get an Ex command line for the `:` command.
pub fn getexline(c: i32, indent: i32, do_concat: bool) -> Option<std::ffi::CString> {
    if g_st().exec_from_reg.get() && vpeekc() == b':' as i32 {
        vgetc();
    }

    getcmdline(c, 1, indent, do_concat)
}

pub fn cmdline_overstrike() -> bool {
    get_cmdline_info().overstrike.get() != 0
}

/// Return true if the cursor is at the end of the cmdline.
pub fn cmdline_at_end() -> bool {
    get_cmdline_info().cmdpos.get() >= get_cmdline_info().cmdlen.get()
}

/// Deallocate a command line buffer, updating the buffer size and length.
fn dealloc_cmdbuff() {
    *get_cmdline_info().cmdbuff_vec.borrow_mut() = Vec::new();
    get_cmdline_info().cmdbufflen.set(0);
    get_cmdline_info().cmdlen.set(0);
}

/// Allocate a new command line buffer.
fn alloc_cmdbuff(mut len: i32) {
    if len < 80 {
        len = 100;
    } else {
        len += 20;
    }

    *get_cmdline_info().cmdbuff_vec.borrow_mut() = vec![0; len as usize];
    get_cmdline_info().cmdbufflen.set(len);
}

/// Re-allocate the command line to length `len` + something extra.
pub fn realloc_cmdbuff(mut len: i32) {
    if len < get_cmdline_info().cmdbufflen.get() {
        return;
    }

    if len < 80 {
        len = 100;
    } else {
        len += 20;
    }

    let p = get_cmdline_info().cmdbuff();
    get_cmdline_info()
        .cmdbuff_vec
        .borrow_mut()
        .resize(len as usize, 0);
    get_cmdline_info().cmdbufflen.set(len);

    let cmdlen = get_cmdline_info().cmdlen.get() as usize;
    get_cmdline_info().cmdbuff_vec.borrow_mut()[cmdlen] = NUL;

    let ccline = get_cmdline_info();
    if let Ok(mut xpc_borrow) = ccline.xpc.try_borrow_mut()
        && let Some(ref mut xpc) = *xpc_borrow
        && !xpc.xp_pattern.is_null()
        && xpc.xp_context != EXPAND_NOTHING
        && xpc.xp_context != EXPAND_UNSUCCESSFUL
    {
        let i = ((xpc.xp_pattern).offset_from(p) as isize) as i32;

        if i >= 0 && i <= ccline.cmdlen.get() {
            xpc.xp_pattern = ccline.cmdbuff().wrapping_add(i as usize);
        }
    }
}

/// Color expression cmdline using built-in expressions parser.
fn color_expr_cmdline(colored_ccline: &CmdlineInfo, ret_ccline_colors: &mut ColoredCmdline) {
    let parser_lines = [
        ParserLine::from_slice(colored_ccline.cmdbuff().as_bytes()),
        ParserLine::NONE,
    ];
    let mut pstate = ParserState {
        reader: ParserInputReader::new(&parser_lines),
        pos: ParserPosition { line: 0, col: 0 },
        colors: Some(ParserHighlight::new()),
    };
    drop(viml_pexpr_parse(&mut pstate, K_EXPR_FLAGS_DISALLOW_EOC));
    let colors = pstate.colors.take().unwrap_or_default();
    drop(pstate);

    ret_ccline_colors.colors.reserve(colors.len());
    let mut prev_end: usize = 0;
    for chunk in colors {
        debug_assert!(chunk.start.col < i32::MAX as usize);
        debug_assert!(chunk.end_col < i32::MAX as usize);
        if chunk.start.col != prev_end {
            ret_ccline_colors.colors.push(CmdlineColorChunk {
                start: prev_end as i32,
                end: chunk.start.col as i32,
                hl_id: 0,
            });
        }
        ret_ccline_colors.colors.push(CmdlineColorChunk {
            start: chunk.start.col as i32,
            end: chunk.end_col as i32,
            hl_id: syn_name2id(chunk.group),
        });
        prev_end = chunk.end_col;
    }
    if prev_end < colored_ccline.cmdlen.get() as usize {
        ret_ccline_colors.colors.push(CmdlineColorChunk {
            start: prev_end as i32,
            end: colored_ccline.cmdlen.get(),
            hl_id: 0,
        });
    }
}

/// Color command-line.
fn color_cmdline(colored_ccline: &CmdlineInfo) -> bool {
    let mut printed_errmsg = false;
    let mut ret = true;

    {
        let mut ccline_colors = colored_ccline.last_colors.borrow_mut();

        if ccline_colors.prompt_id == colored_ccline.prompt_id.get()
            && ccline_colors
                .cmdbuff
                .as_ref()
                .is_some_and(|cb| cb.to_bytes() == colored_ccline.cmdbuff().as_bytes())
        {
            return ret;
        }

        ccline_colors.colors.clear();

        if colored_ccline.cmdbuff().is_empty() {
            ccline_colors.cmdbuff = None;
            return ret;
        }
    }

    let mut arg_owned: Option<std::ffi::CString> = None;
    let mut arg = TypVal {
        v_type: VarType::VarString,
        v_lock: 0,
        vval: TypvalVval::from_opt_cstr(colored_ccline.cmdbuff().as_opt_cstr()),
    };
    let mut tv = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    };

    let mut color_cb = Callback::NONE;
    let mut can_free_cb = false;
    let mut err = error_init();
    let mut err_errmsg: &'static CStr = E_INTERN2;
    let mut dgc_ret = true;

    'color_cmdline_end: {
        'color_cmdline_error: {
            if colored_ccline.prompt_id.get() != GETLN_STATE.with(|s| s.prev_prompt_id.get()) {
                GETLN_STATE.with(|s| s.prev_prompt_errors.set(0));
                GETLN_STATE.with(|s| s.prev_prompt_id.set(colored_ccline.prompt_id.get()));
            } else if GETLN_STATE.with(|s| s.prev_prompt_errors.get()) >= MAX_CB_ERRORS {
                break 'color_cmdline_end;
            }
            if colored_ccline.highlight_callback.borrow().cb_type() != K_CALLBACK_NONE_CONST {
                debug_assert!(colored_ccline.input_fn.get() != 0);
                color_cb = colored_ccline.highlight_callback.borrow().clone();
            } else if colored_ccline.cmdfirstc.get() == b':' as i32 {
                let mut tstate = TryState::new();
                try_enter(&mut tstate);
                err_errmsg = c"E5408: Unable to get g:Nvim_color_cmdline callback: %s";
                dgc_ret = tv_dict_get_callback(
                    Some(&get_globvar_dict()),
                    c"Nvim_color_cmdline",
                    18,
                    &color_cb,
                );
                try_leave(&tstate, &mut err);
                can_free_cb = true;
            } else if colored_ccline.cmdfirstc.get() == b'=' as i32 {
                color_expr_cmdline(colored_ccline, &mut colored_ccline.last_colors.borrow_mut());
            }
            if error_set(&err) || !dgc_ret {
                break 'color_cmdline_error;
            }

            if color_cb.cb_type() == K_CALLBACK_NONE_CONST {
                break 'color_cmdline_end;
            }
            if colored_ccline
                .cmdbuff_vec
                .borrow()
                .get(colored_ccline.cmdlen.get() as usize)
                .copied()
                != Some(0)
            {
                let cstr = std::ffi::CString::new(
                    (colored_ccline.cmdbuff()).slice(colored_ccline.cmdlen.get() as usize),
                )
                .unwrap_or_default();
                arg.vval = crate::eval::typval::TypvalVval::from_cstr(&cstr);
                arg_owned = Some(cstr);
            }

            GETLN_STATE.with(|s| s.getln_interrupted_highlight.set(false));
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            err_errmsg = c"E5407: Callback has thrown an exception: %s";
            let saved_msg_col = g_st().msg_col.get();
            g_st().msg_silent.set(g_st().msg_silent.get() + (1));
            let cbcall_ret = callback_call(&color_cb, 1, std::slice::from_mut(&mut arg), &mut tv);
            g_st().msg_silent.set(g_st().msg_silent.get() - (1));
            g_st().msg_col.set(saved_msg_col);
            if g_st().got_int.get() {
                GETLN_STATE.with(|s| s.getln_interrupted_highlight.set(true));
            }
            try_leave(&tstate, &mut err);
            if error_set(&err) || !cbcall_ret {
                break 'color_cmdline_error;
            }
            if tv.v_type != VarType::VarList {
                g_st().msg_scroll.set(1);
                msg_putchar(b'\n' as i32);
                smsg!(HLF_E, c"%s", gettext(c"E5400: Callback should return list"),);
                printed_errmsg = true;
                break 'color_cmdline_error;
            }
            let Some(v_list) = tv.vval.v_list_opt() else {
                break 'color_cmdline_end;
            };
            let mut prev_end: Varnumber = 0;
            for (i, li) in (0_i32..).zip(crate::eval::typval::tv_list_iter(Some(v_list))) {
                let li_tv = li.tv();
                if li_tv.v_type != VarType::VarList {
                    g_st().msg_scroll.set(1);
                    msg_putchar(b'\n' as i32);
                    smsg!(HLF_E, gettext(c"E5401: List item %i is not a List"), i,);
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                let l = li_tv.vval.v_list_opt();
                if tv_list_len(l) != 3 {
                    g_st().msg_scroll.set(1);
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5402: List item %i has incorrect length: %d /= 3"),
                        i,
                        tv_list_len(l),
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                let mut inner = crate::eval::typval::tv_list_iter(l);
                let first_li = inner.next().unwrap();
                let second_li = inner.next().unwrap();
                let last_li = inner.next().unwrap();
                let mut error = false;
                let start = tv_get_number_chk(&first_li.tv(), Some(&mut error));
                if error {
                    break 'color_cmdline_error;
                } else if !(prev_end <= start && start < colored_ccline.cmdlen.get() as Varnumber) {
                    g_st().msg_scroll.set(1);
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5403: Chunk %i start %lld not in range [%lld, %i)"),
                        i,
                        start,
                        prev_end,
                        colored_ccline.cmdlen.get(),
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                } else if UTF8LEN_TAB_ZERO
                    [colored_ccline.cmdbuff_vec.borrow()[start as usize] as usize]
                    == 0
                {
                    g_st().msg_scroll.set(1);
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5405: Chunk %i start %lld splits multibyte character",),
                        i,
                        start,
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                if start != prev_end {
                    colored_ccline
                        .last_colors
                        .borrow_mut()
                        .colors
                        .push(CmdlineColorChunk {
                            start: prev_end as i32,
                            end: start as i32,
                            hl_id: 0,
                        });
                }
                let end = tv_get_number_chk(&second_li.tv(), Some(&mut error));
                if error {
                    break 'color_cmdline_error;
                } else if !(start < end && end <= colored_ccline.cmdlen.get() as Varnumber) {
                    g_st().msg_scroll.set(1);
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5404: Chunk %i end %lld not in range (%lld, %i]"),
                        i,
                        end,
                        start,
                        colored_ccline.cmdlen.get(),
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                } else if end < colored_ccline.cmdlen.get() as Varnumber
                    && UTF8LEN_TAB_ZERO[colored_ccline.cmdbuff_vec.borrow()[end as usize] as usize]
                        == 0
                {
                    g_st().msg_scroll.set(1);
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5406: Chunk %i end %lld splits multibyte character",),
                        i,
                        end,
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                prev_end = end;
                let Some(group) =
                    tv_get_string_chk(&last_li.tv()).map(std::borrow::Cow::into_owned)
                else {
                    break 'color_cmdline_error;
                };
                let hl_id = syn_name2id(&group);
                colored_ccline
                    .last_colors
                    .borrow_mut()
                    .colors
                    .push(CmdlineColorChunk {
                        start: start as i32,
                        end: end as i32,
                        hl_id,
                    });
            }
            if prev_end < colored_ccline.cmdlen.get() as Varnumber {
                let end_col = colored_ccline.cmdlen.get();
                colored_ccline
                    .last_colors
                    .borrow_mut()
                    .colors
                    .push(CmdlineColorChunk {
                        start: prev_end as i32,
                        end: end_col,
                        hl_id: 0,
                    });
            }
            GETLN_STATE.with(|s| s.prev_prompt_errors.set(0));
            break 'color_cmdline_end;
        }

        // color_cmdline_error:
        if error_set(&err) {
            g_st().msg_scroll.set(1);
            msg_putchar(b'\n' as i32);
            smsg!(HLF_E, gettext(err_errmsg), err.msg_cstr());
            printed_errmsg = true;
            api_clear_error(&mut err);
        }
        let _ = printed_errmsg;

        GETLN_STATE.with(|s| s.prev_prompt_errors.set(s.prev_prompt_errors.get() + (1)));
        colored_ccline.last_colors.borrow_mut().colors.clear();
        redrawcmdline();
        ret = false;
    }

    // color_cmdline_end:
    debug_assert!(!error_set(&err));
    if can_free_cb {
        callback_free(&color_cb);
    }
    {
        let mut ccline_colors = colored_ccline.last_colors.borrow_mut();
        ccline_colors.prompt_id = colored_ccline.prompt_id.get();
        ccline_colors.cmdbuff = arg_owned.or_else(|| {
            Some(
                std::ffi::CString::new(
                    (colored_ccline.cmdbuff()).slice(colored_ccline.cmdlen.get() as usize),
                )
                .unwrap_or_default(),
            )
        });
    }
    tv_clear(&mut tv);
    ret
}

/// Draw part of the cmdline at the current cursor position.
fn draw_cmdline(start: i32, len: i32) {
    if get_cmdline_info().cmdbuff().is_null() || !color_cmdline(&get_cmdline_info()) {
        return;
    }

    if ui_has(K_UICMDLINE) {
        get_cmdline_info().special_char.set(0);
        get_cmdline_info().redraw_state.set(K_CMD_REDRAW_ALL);
        return;
    }

    if g_st().cmdline_star.get() > 0 {
        let mut i: i32 = 0;
        while i < len {
            msg_putchar(b'*' as i32);
            i += utfc_ptr2len(
                get_cmdline_info()
                    .cmdbuff()
                    .wrapping_add((start + i) as usize),
            ) - 1;
            i += 1;
        }
    } else if !get_cmdline_info().last_colors.borrow().colors.is_empty() {
        let colors = get_cmdline_info().last_colors.borrow().colors.clone();
        for &chunk in &colors {
            if chunk.end <= start {
                continue;
            }
            let chunk_start = chunk.start.max(start);
            let sub = get_cmdline_info()
                .cmdbuff()
                .wrapping_add(chunk_start as usize);
            let slice = sub.slice((chunk.end - chunk_start).max(0) as usize);
            msg_outtrans_len(slice, chunk.end - chunk_start, chunk.hl_id, false);
        }
    } else {
        let sub = get_cmdline_info().cmdbuff().wrapping_add(start as usize);
        let slice = sub.slice(len.max(0) as usize);
        msg_outtrans_len(slice, len, 0, false);
    }
}

fn ui_ext_cmdline_show(line: &CmdlineInfo) {
    let mut content: Array;
    let cmdbuff = line.cmdbuff();
    if g_st().cmdline_star.get() != 0 {
        content = Array::with_capacity(1);
        let mut len: usize = 0;
        let mut p = get_cmdline_info().cmdbuff();
        while !p.is_empty() {
            let step = utfc_ptr2len(&p) as usize;
            p = p.wrapping_add(step);
            len += 1;
        }
        let stars = vec![b'*'; len];
        let mut item = Array::with_capacity(3);
        add_c(&mut item, integer_obj(0));
        add_c(&mut item, string_obj(NvimString::from_bytes(&stars)));
        add_c(&mut item, integer_obj(0));
        add_c(&mut content, array_obj(item));
    } else if !line.last_colors.borrow().colors.is_empty() {
        let last_colors = line.last_colors.borrow();
        content = Array::with_capacity(last_colors.colors.len());
        let cmd_bytes = cmdbuff.as_bytes();
        for &chunk in &last_colors.colors {
            let mut item = Array::with_capacity(3);
            add_c(
                &mut item,
                integer_obj(if chunk.hl_id == 0 {
                    0
                } else {
                    syn_id2attr(chunk.hl_id) as Integer
                }),
            );

            debug_assert!(chunk.end >= chunk.start);
            let start_idx = (chunk.start as usize).min(cmd_bytes.len());
            let end_idx = (chunk.end as usize).min(cmd_bytes.len());
            add_c(
                &mut item,
                string_obj(cstrn_as_string(&cmd_bytes[start_idx..end_idx])),
            );
            add_c(&mut item, integer_obj(chunk.hl_id as Integer));
            add_c(&mut content, array_obj(item));
        }
    } else {
        let mut item = Array::with_capacity(3);
        add_c(&mut item, integer_obj(0));
        add_c(&mut item, cstr_as_obj(cmdbuff.as_opt_cstr()));
        add_c(&mut item, integer_obj(0));
        content = Array::with_capacity(1);
        add_c(&mut content, array_obj(item));
    }
    let mut charbuf: [u8; 2] = [line.cmdfirstc.get() as u8, 0];
    ui_call_cmdline_show(
        content,
        line.cmdpos.get() as Integer,
        cstr_as_string(Some(crate::memory::bytes_as_cstr(&charbuf))),
        cstr_as_string(line.cmdprompt.borrow().as_deref()),
        line.cmdindent.get() as Integer,
        line.level.get() as Integer,
        if line.cmdprompt.borrow().is_some() {
            line.hl_id.get() as Integer
        } else {
            -1
        },
    );
    if line.special_char.get() != 0 {
        charbuf[0] = line.special_char.get();
        ui_call_cmdline_special_char(
            cstr_as_string(Some(crate::memory::bytes_as_cstr(&charbuf))),
            line.special_shift.get(),
            line.level.get() as Integer,
        );
    }
}

pub fn ui_ext_cmdline_block_append(indent: usize, line: &[u8]) {
    let line_bytes = crate::memory::bytes_until_nul(line);
    let mut buf: Vec<u8> = Vec::with_capacity(indent + line_bytes.len());
    buf.resize(indent, b' ');
    buf.extend_from_slice(line_bytes);

    let mut item = Array::new();
    array_add(&mut item, integer_obj(0));
    array_add(
        &mut item,
        string_obj(crate::api::private::helpers::cbuf_to_string(&buf)),
    );
    array_add(&mut item, integer_obj(0));
    let mut content = Array::new();
    array_add(&mut content, array_obj(item));
    let mut blk = GETLN_STATE.with(|s| s.cmdline_block.get());
    array_add(&mut blk, array_obj(content.clone()));
    GETLN_STATE.with(|s| s.cmdline_block.set(blk));
    if GETLN_STATE.with(|s| s.cmdline_block.get()).size > 1 {
        ui_call_cmdline_block_append(content);
    } else {
        ui_call_cmdline_block_show(GETLN_STATE.with(|s| s.cmdline_block.get()));
    }
}

pub fn ui_ext_cmdline_block_leave() {
    api_free_array(GETLN_STATE.with(|s| s.cmdline_block.get()));
    GETLN_STATE.with(|s| s.cmdline_block.set(Array::new()));
    ui_call_cmdline_block_hide();
}

/// Extra redrawing needed for `redraw!` and on `ui_attach`.
pub fn cmdline_screen_cleared() {
    if !ui_has(K_UICMDLINE) {
        return;
    }

    if GETLN_STATE.with(|s| s.cmdline_block.get()).size != 0 {
        ui_call_cmdline_block_show(GETLN_STATE.with(|s| s.cmdline_block.get()));
    }

    let mut prev_level = get_cmdline_info().level.get() - 1;
    let mut line = get_cmdline_info().prev_ccline.borrow().clone();
    while prev_level > 0
        && let Some(line_ref) = line
    {
        if line_ref.level.get() == prev_level {
            line_ref.redraw_state.set(K_CMD_REDRAW_ALL);

            prev_level -= 1;
        }

        line = line_ref.prev_ccline.borrow().clone();
    }
    redrawcmd();
}

/// Called by `ui_flush`, do what redraws necessary to keep cmdline updated.
pub fn cmdline_ui_flush() {
    if !ui_has(K_UICMDLINE) {
        return;
    }
    let mut level = get_cmdline_info().level.get();
    let mut line = Some(get_cmdline_info());
    while level > 0
        && let Some(line_ref) = line
    {
        if line_ref.level.get() == level {
            let redraw_state = line_ref.redraw_state.get();

            line_ref.redraw_state.set(K_CMD_REDRAW_NONE);

            if redraw_state == K_CMD_REDRAW_ALL {
                g_st().cmdline_was_last_drawn.set(true);

                ui_ext_cmdline_show(&line_ref);
            } else if redraw_state == K_CMD_REDRAW_POS && g_st().cmdline_was_last_drawn.get() {
                ui_call_cmdline_pos(
                    line_ref.cmdpos.get() as Integer,
                    line_ref.level.get() as Integer,
                );
            }
            level -= 1;
        }

        line = line_ref.prev_ccline.borrow().clone();
    }
}

/// Put a character on the command line.
pub fn putcmdline(c: u8, shift: bool) {
    if g_st().cmd_silent.get() {
        return;
    }
    if !ui_has(K_UICMDLINE) {
        g_st().msg_no_more.set(true);
        msg_putchar(i32::from(c));
        if shift {
            draw_cmdline(
                get_cmdline_info().cmdpos.get(),
                get_cmdline_info().cmdlen.get() - get_cmdline_info().cmdpos.get(),
            );
        }
        g_st().msg_no_more.set(false);
    } else if get_cmdline_info().redraw_state.get() != K_CMD_REDRAW_ALL {
        let charbuf: [u8; 2] = [c, 0];
        ui_call_cmdline_special_char(
            cstr_as_string(Some(crate::memory::bytes_as_cstr(&charbuf))),
            shift,
            get_cmdline_info().level.get() as Integer,
        );
    }
    cursorcmd();
    get_cmdline_info().special_char.set(c);
    get_cmdline_info().special_shift.set(shift);
    ui_cursor_shape();
}

/// Undo a `putcmdline(c, false)`.
pub fn unputcmdline() {
    if g_st().cmd_silent.get() {
        return;
    }
    g_st().msg_no_more.set(true);
    if get_cmdline_info().cmdlen.get() == get_cmdline_info().cmdpos.get() && !ui_has(K_UICMDLINE) {
        msg_putchar(b' ' as i32);
    } else {
        draw_cmdline(
            get_cmdline_info().cmdpos.get(),
            utfc_ptr2len(
                get_cmdline_info()
                    .cmdbuff()
                    .wrapping_add(get_cmdline_info().cmdpos.get() as usize),
            ),
        );
    }
    g_st().msg_no_more.set(false);
    cursorcmd();
    get_cmdline_info().special_char.set(0);
    ui_cursor_shape();
}

/// Put the given string, of the given length, onto the command line.
pub fn put_on_cmdline(str_bytes: &[u8], mut len: i32, redraw: bool) {
    let bytes = crate::memory::bytes_until_nul(str_bytes);
    if len < 0 {
        len = bytes.len() as i32;
    } else {
        len = len.min(bytes.len() as i32);
    }
    let slice = &bytes[..len as usize];

    realloc_cmdbuff(get_cmdline_info().cmdlen.get() + len + 1);
    let cmdpos_usize = get_cmdline_info().cmdpos.get() as usize;
    let len_usize = len as usize;

    if get_cmdline_info().overstrike.get() == 0 {
        let old_cmdlen = get_cmdline_info().cmdlen.get() as usize;
        get_cmdline_info()
            .cmdbuff_vec
            .borrow_mut()
            .copy_within(cmdpos_usize..old_cmdlen, cmdpos_usize + len_usize);
        get_cmdline_info()
            .cmdlen
            .set(get_cmdline_info().cmdlen.get() + (len));
    } else {
        let mut m: i32 = 0;
        let mut i: usize = 0;
        while (i as i32) < len {
            m += 1;
            i += crate::mbyte::utfc_slice2len(&slice[i..]).max(1);
        }
        let mut ci = get_cmdline_info().cmdpos.get();
        while ci < get_cmdline_info().cmdlen.get() && m > 0 {
            m -= 1;
            ci += utfc_ptr2len(get_cmdline_info().cmdbuff().wrapping_add(ci as usize));
        }
        if ci < get_cmdline_info().cmdlen.get() {
            let old_cmdlen = get_cmdline_info().cmdlen.get() as usize;
            get_cmdline_info()
                .cmdbuff_vec
                .borrow_mut()
                .copy_within(ci as usize..old_cmdlen, cmdpos_usize + len_usize);
            get_cmdline_info().cmdlen.set(
                get_cmdline_info().cmdlen.get() + (get_cmdline_info().cmdpos.get() + len - ci),
            );
        } else {
            get_cmdline_info()
                .cmdlen
                .set(get_cmdline_info().cmdpos.get() + len);
        }
    }
    get_cmdline_info().cmdbuff_vec.borrow_mut()[cmdpos_usize..cmdpos_usize + len_usize]
        .copy_from_slice(slice);
    let new_cmdlen = get_cmdline_info().cmdlen.get() as usize;
    get_cmdline_info().cmdbuff_vec.borrow_mut()[new_cmdlen] = 0;

    if get_cmdline_info().cmdpos.get() > 0
        && get_cmdline_info().cmdbuff_vec.borrow()[cmdpos_usize] >= 0x80
    {
        let i = crate::mbyte::utf_head_off(
            &get_cmdline_info().cmdbuff_vec.borrow(),
            get_cmdline_info().cmdpos.get() as usize,
        );
        if i != 0 {
            get_cmdline_info()
                .cmdpos
                .set(get_cmdline_info().cmdpos.get() - (i));
            len += i;
            get_cmdline_info()
                .cmdspos
                .set(cmd_screencol(get_cmdline_info().cmdpos.get()));
        }
    }

    if redraw && !g_st().cmd_silent.get() {
        g_st().msg_no_more.set(true);
        let i = g_st().cmdline_row.get();
        cursorcmd();
        draw_cmdline(
            get_cmdline_info().cmdpos.get(),
            get_cmdline_info().cmdlen.get() - get_cmdline_info().cmdpos.get(),
        );
        if g_st().cmdline_row.get() != i || get_cmdline_info().overstrike.get() != 0 {
            msg_clr_eos();
        }
        g_st().msg_no_more.set(false);
    }
    let mut m: i32;
    if g_st().key_typed.get() {
        m = g_st().columns_global.get() * g_st().rows_global.get();
        if m < 0 {
            m = MAXCOL;
        }
    } else {
        m = MAXCOL;
    }
    let mut i: i32 = 0;
    while i < len {
        let mut c = cmdline_charsize(get_cmdline_info().cmdpos.get());
        let mut cmdspos = get_cmdline_info().cmdspos.get();
        correct_screencol(get_cmdline_info().cmdpos.get(), c, &mut cmdspos);
        get_cmdline_info().cmdspos.set(cmdspos);
        if get_cmdline_info().cmdspos.get() + c < m {
            get_cmdline_info()
                .cmdspos
                .set(get_cmdline_info().cmdspos.get() + (c));
        }
        c = utfc_ptr2len(
            get_cmdline_info()
                .cmdbuff()
                .wrapping_add(get_cmdline_info().cmdpos.get() as usize),
        ) - 1;
        c = c.min(len - i - 1);
        get_cmdline_info()
            .cmdpos
            .set(get_cmdline_info().cmdpos.get() + (c));
        i += c;
        get_cmdline_info()
            .cmdpos
            .set(get_cmdline_info().cmdpos.get() + (1));
        i += 1;
    }

    if redraw {
        msg_check();
    }
}

/// Save `get_cmdline_info()`.
fn save_cmdline() {
    let new_cc = std::rc::Rc::new(CmdlineInfo::new());
    let prev = CCLINE.with(|c| std::mem::replace(&mut *c.borrow_mut(), new_cc.clone()));
    *new_cc.prev_ccline.borrow_mut() = Some(prev);
}

/// Restore `get_cmdline_info()` after it has been saved with `save_cmdline()`.
fn restore_cmdline() {
    let prev = get_cmdline_info().prev_ccline.borrow_mut().take();
    CCLINE.with(|c| {
        *c.borrow_mut() = prev.unwrap_or_else(|| std::rc::Rc::new(CmdlineInfo::new()));
    });
}

/// Paste a yank register into the command line.
fn cmdline_paste(regname: i32, literally: bool, remcr: bool) -> bool {
    if regname != CTRL_F_CONST
        && regname != CTRL_P
        && regname != CTRL_W_CONST
        && regname != CTRL_A
        && regname != CTRL_L_CONST
        && !valid_yank_reg(regname, false)
    {
        return false;
    }

    line_breakcheck();
    if g_st().got_int.get() {
        return false;
    }

    g_st().textlock.set(g_st().textlock.get() + (1));
    let spec_res = get_spec_reg(regname, true);
    g_st().textlock.set(g_st().textlock.get() - (1));

    if let Some(opt_vec) = spec_res {
        let Some(vec_val) = opt_vec else {
            return false;
        };
        let arg_b = crate::memory::bytes_until_nul(&vec_val);

        let mut p_slice = arg_b;
        if opt_vars().p_is.get() && regname == CTRL_W_CONST {
            let ccline_rc = get_cmdline_info();
            let cmd_vec = ccline_rc.cmdbuff_vec.borrow();
            let cmd_bytes = &cmd_vec[..ccline_rc.cmdpos.get() as usize];
            let mut w_idx = cmd_bytes.len();
            while w_idx > 0 {
                let step = (crate::mbyte::utf_head_off_slice(cmd_bytes, w_idx - 1) + 1) as usize;
                if !vim_iswordc(crate::mbyte::utf_slice2char(&cmd_bytes[w_idx - step..])) {
                    break;
                }
                w_idx -= step;
            }
            let len = cmd_bytes.len() - w_idx;
            let w_slice = &cmd_bytes[w_idx..];
            let arg_prefix = &arg_b[..len.min(arg_b.len())];
            let matches = if opt_vars().p_ic.get() {
                w_slice.eq_ignore_ascii_case(arg_prefix)
            } else {
                w_slice == arg_prefix
            };
            if matches {
                p_slice = &arg_b[len.min(arg_b.len())..];
            }
        }

        cmdline_paste_str(p_slice, literally);
        return true;
    }

    cmdline_paste_reg(regname, literally, remcr)
}

/// Put a string on the command line.
pub fn cmdline_paste_str(s: &[u8], literally: bool) {
    let bytes = crate::memory::bytes_until_nul(s);
    if literally {
        put_on_cmdline(bytes, -1, true);
    } else {
        let mut rem = bytes;
        while !rem.is_empty() {
            let cv = i32::from(rem[0]);
            if cv == CTRL_V_CONST && rem.len() > 1 {
                rem = &rem[1..];
            }
            let c = mb_cptr2char_adv(&mut rem);
            if cv == CTRL_V_CONST
                || c == ESC
                || c == CTRL_C_CONST
                || c == CAR
                || c == NL
                || c == CTRL_L_CONST
                || (c == CTRL_BSL && rem.first().copied().map(i32::from) == Some(CTRL_N))
            {
                stuffchar_readbuff(CTRL_V_CONST);
            }
            stuffchar_readbuff(c);
        }
    }
}

pub fn redrawcmdline() {
    if g_st().cmd_silent.get() {
        return;
    }
    g_st().need_wait_return.set(false);
    compute_cmdrow();
    redrawcmd();
    cursorcmd();
    ui_cursor_shape();
}

fn redrawcmdprompt() {
    if g_st().cmd_silent.get() {
        return;
    }
    if ui_has(K_UICMDLINE) {
        get_cmdline_info().redraw_state.set(K_CMD_REDRAW_ALL);
        return;
    }
    if get_cmdline_info().cmdfirstc.get() != NUL as i32 {
        msg_putchar(get_cmdline_info().cmdfirstc.get());
    }
    let ccline_rc = get_cmdline_info();
    if let Some(ref prompt) = *ccline_rc.cmdprompt.borrow() {
        msg_puts_hl(prompt, get_cmdline_info().hl_id.get(), false);
        get_cmdline_info().cmdindent.set(
            g_st().msg_col.get()
                + (g_st().msg_row.get() - g_st().cmdline_row.get()) * g_st().columns_global.get(),
        );
        if get_cmdline_info().cmdfirstc.get() != NUL as i32 {
            get_cmdline_info()
                .cmdindent
                .set(get_cmdline_info().cmdindent.get() - (1));
        }
    } else {
        let mut i = get_cmdline_info().cmdindent.get();
        while i > 0 {
            msg_putchar(b' ' as i32);
            i -= 1;
        }
    }
}

/// Redraw what is currently on the command line.
pub fn redrawcmd() {
    if g_st().cmd_silent.get() {
        return;
    }

    if ui_has(K_UICMDLINE) {
        draw_cmdline(0, get_cmdline_info().cmdlen.get());
        return;
    }

    if get_cmdline_info().cmdbuff().is_null() {
        msg_cursor_goto(g_st().cmdline_row.get(), 0);
        msg_clr_eos();
        return;
    }

    g_st().redrawing_cmdline.set(true);

    sb_text_restart_cmdline();
    msg_start();
    msg_starthere();
    redrawcmdprompt();

    g_st().msg_no_more.set(true);
    draw_cmdline(0, get_cmdline_info().cmdlen.get());
    msg_clr_eos();
    g_st().msg_no_more.set(false);

    get_cmdline_info()
        .cmdspos
        .set(cmd_screencol(get_cmdline_info().cmdpos.get()));

    if get_cmdline_info().special_char.get() != 0 {
        putcmdline(
            get_cmdline_info().special_char.get(),
            get_cmdline_info().special_shift.get(),
        );
    }

    g_st().msg_scroll.set(0);
    g_st().skip_redraw.set(false);
    g_st().cmdline_was_last_drawn.set(true);
    g_st().redrawing_cmdline.set(false);
}

pub fn compute_cmdrow() {
    if g_st().msg_scrolled.get() != 0 {
        g_st().cmdline_row.set(g_st().rows_global.get() - 1);
    } else {
        let wp = lastwin_nofloating(None);
        g_st().cmdline_row.set(
            wp.w_winrow.get()
                + wp.w_height.get()
                + wp.w_hsep_height.get()
                + wp.w_status_height.get()
                + global_stl_height(),
        );
    }
    if g_st().cmdline_row.get() == g_st().rows_global.get() && opt_vars().p_ch.get() > 0 {
        g_st().cmdline_row.set(g_st().cmdline_row.get() - (1));
    }
    g_st().lines_left.set(g_st().cmdline_row.get());
}

pub fn cursorcmd() {
    if g_st().cmd_silent.get() || ui_has(K_UICMDLINE) {
        return;
    }

    g_st().msg_row.set(
        g_st().cmdline_row.get() + (get_cmdline_info().cmdspos.get() / g_st().columns_global.get()),
    );
    g_st()
        .msg_col
        .set(get_cmdline_info().cmdspos.get() % g_st().columns_global.get());
    g_st()
        .msg_row
        .set(g_st().msg_row.get().min(g_st().rows_global.get() - 1));

    msg_cursor_goto(g_st().msg_row.get(), g_st().msg_col.get());
}

pub fn gotocmdline(clr: bool) {
    if ui_has(K_UICMDLINE) {
        return;
    }
    msg_start();
    g_st().msg_col.set(0);
    if clr {
        msg_clr_eos();
    }
    msg_cursor_goto(g_st().cmdline_row.get(), 0);
}

/// Check the word in front of the cursor for an abbreviation.
fn ccheck_abbr(c: i32) -> i32 {
    let mut spos: i32 = 0;

    if opt_vars().p_paste.get() || g_st().no_abbr.get() {
        return 0;
    }

    while spos < get_cmdline_info().cmdlen.get()
        && ascii_iswhite(i32::from(
            get_cmdline_info().cmdbuff_vec.borrow()[spos as usize],
        ))
    {
        spos += 1;
    }
    if get_cmdline_info().cmdlen.get() - spos > 5
        && get_cmdline_info().cmdbuff_vec.borrow()[spos as usize] == b'\''
        && get_cmdline_info().cmdbuff_vec.borrow()[(spos + 2) as usize] == b','
        && get_cmdline_info().cmdbuff_vec.borrow()[(spos + 3) as usize] == b'\''
    {
        spos += 5;
    } else {
        spos = 0;
    }

    i32::from(check_abbr(
        c,
        get_cmdline_info().cmdbuff().as_cstr(),
        get_cmdline_info().cmdpos.get(),
        spos,
    ))
}

/// Escape special characters in `fname`, depending on `what`.
pub fn vim_strsave_fnameescape(fname: &[u8], what: i32) -> std::ffi::CString {
    let fname_bytes = crate::memory::bytes_until_nul(fname);
    #[cfg(windows)]
    let mut p = {
        let mut buf: [u8; 32] = [0; 32];
        let mut j: usize = 0;
        let chars = if what == VSE_BUFFER {
            BUFFER_ESC_CHARS.to_bytes()
        } else {
            PATH_ESC_CHARS.to_bytes()
        };
        for &cp_byte in chars {
            if (cp_byte != b'[' && cp_byte != b'{' && cp_byte != b'!')
                || !vim_isfilec(i32::from(cp_byte))
            {
                buf[j] = cp_byte;
                j += 1;
            }
        }
        vim_strsave_escaped(fname_bytes, &buf[..j])
    };

    #[cfg(not(windows))]
    let mut p = {
        let chars = if what == VSE_SHELL {
            SHELL_ESC_CHARS.to_bytes()
        } else if what == VSE_BUFFER {
            BUFFER_ESC_CHARS.to_bytes()
        } else {
            PATH_ESC_CHARS.to_bytes()
        };
        let mut res = vim_strsave_escaped(fname_bytes, chars);
        if what == VSE_SHELL && csh_like_shell() != 0 {
            res = vim_strsave_escaped(res.as_bytes(), b"!");
        }
        res
    };

    let bytes = p.as_bytes();
    if bytes.first() == Some(&b'>') || bytes.first() == Some(&b'+') || bytes == b"-" {
        escape_fname_cstr(&mut p);
    }

    p
}

pub fn escape_fname_cstr(s: &mut std::ffi::CString) {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(b'\\');
    out.extend_from_slice(bytes);
    *s = crate::memory::bytes_to_cstring(&out);
}

/// For each file name in `files`:
/// If `orig_pat` starts with `~/`, replace the home directory with `~`.
pub fn tilde_replace(orig_pat: &[u8], files: &mut [std::ffi::CString]) {
    let orig_bytes = crate::memory::bytes_until_nul(orig_pat);
    if orig_bytes.first() == Some(&b'~')
        && orig_bytes
            .get(1)
            .is_some_and(|&b| vim_ispathsep(i32::from(b)))
    {
        for f in files {
            *f = home_replace_save(None, Some(f.as_c_str()));
        }
    }
}

/// Get a handle to the current command line info.
pub fn get_cmdline_info() -> std::rc::Rc<CmdlineInfo> {
    CCLINE.with(|c| c.borrow().clone())
}

pub fn get_cmdline_last_prompt_id() -> u32 {
    GETLN_STATE.with(|s| s.last_prompt_id.get())
}

/// Get reference to the command line info to use.
fn get_ccline_opt() -> Option<std::rc::Rc<CmdlineInfo>> {
    if (g_st().state_global.get() & MODE_CMDLINE) == 0 {
        return None;
    }
    let cc = get_cmdline_info();
    if !cc.cmdbuff().is_null() {
        Some(cc)
    } else if let Some(ref prev) = *cc.prev_ccline.borrow()
        && !prev.cmdbuff().is_null()
    {
        Some(prev.clone())
    } else {
        None
    }
}

/// Get the current command-line type.
fn get_cmdline_type() -> i32 {
    let Some(p) = get_ccline_opt() else {
        return NUL as i32;
    };
    if p.cmdfirstc.get() == NUL as i32 {
        return if p.input_fn.get() != 0 {
            b'@' as i32
        } else {
            b'-' as i32
        };
    }
    p.cmdfirstc.get()
}

/// Get the current command line in allocated memory.
fn get_cmdline_str() -> Option<std::ffi::CString> {
    if g_st().cmdline_star.get() > 0 {
        return None;
    }
    let p = get_ccline_opt()?;
    let cmdbuff = p.cmdbuff();
    let slice = cmdbuff.slice(p.cmdlen.get() as usize);
    let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    Some(crate::memory::bytes_to_cstring(&slice[..len]))
}

/// Get the current command-line completion pattern.
fn get_cmdline_completion_pattern() -> Option<std::ffi::CString> {
    if g_st().cmdline_star.get() > 0 {
        return None;
    }

    let p = get_ccline_opt()?;
    let mut xpc_borrow = p.xpc.try_borrow_mut().ok()?;
    let xpc = xpc_borrow.as_mut()?;

    let mut xp_context = xpc.xp_context;
    if xp_context == EXPAND_NOTHING {
        set_expand_context(xpc);
        xp_context = xpc.xp_context;
        xpc.xp_context = EXPAND_NOTHING;
    }
    if xp_context == EXPAND_UNSUCCESSFUL {
        return None;
    }

    let compl_pat = &xpc.xp_pattern;
    if compl_pat.is_null() {
        return None;
    }

    Some(compl_pat.to_cstring())
}

/// Get the command-line completion type.
fn get_cmdline_completion() -> Option<std::ffi::CString> {
    if g_st().cmdline_star.get() > 0 {
        return None;
    }

    let p = get_ccline_opt()?;
    let mut xpc_borrow = p.xpc.try_borrow_mut().ok()?;
    let xpc = xpc_borrow.as_mut()?;

    let mut xp_context = xpc.xp_context;
    if xp_context == EXPAND_NOTHING {
        set_expand_context(xpc);
        xp_context = xpc.xp_context;
        xpc.xp_context = EXPAND_NOTHING;
    }
    if xp_context == EXPAND_UNSUCCESSFUL {
        return None;
    }

    cmdcomplete_type_to_str(xp_context, xpc.xp_arg.as_deref().map(CStr::to_bytes))
}

/// `"getcmdcomplpat()"` function.
pub fn f_getcmdcomplpat(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, get_cmdline_completion_pattern().as_deref());
}

/// `"getcmdcompltype()"` function.
pub fn f_getcmdcompltype(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, get_cmdline_completion().as_deref());
}

/// `"getcmdline()"` function.
pub fn f_getcmdline(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if let Some(s) = get_cmdline_str() {
        crate::eval::typval::tv_set_string_len(rettv, Some(s.as_bytes()));
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// `"getcmdpos()"` function.
pub fn f_getcmdpos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        get_ccline_opt().map_or(0, |p| Varnumber::from(p.cmdpos.get() + 1)),
    );
}

/// `"getcmdprompt()"` function.
pub fn f_getcmdprompt(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let prompt = get_ccline_opt().and_then(|p| p.cmdprompt.borrow().clone());
    crate::eval::typval::tv_set_string(rettv, prompt.as_deref());
}

/// `"getcmdscreenpos()"` function.
pub fn f_getcmdscreenpos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        get_ccline_opt().map_or(0, |p| Varnumber::from(p.cmdspos.get() + 1)),
    );
}

/// `"getcmdtype()"` function.
pub fn f_getcmdtype(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let buf: [u8; 2] = [get_cmdline_type() as u8, 0];
    crate::eval::typval::tv_set_string(
        rettv,
        Some(CStr::from_bytes_until_nul(&buf).unwrap_or(c"")),
    );
}

/// Set the command line str to `str`.
/// @return 1 when failed, 0 when OK.
fn set_cmdline_str(str: &CStr, pos: i32) -> i32 {
    let Some(p) = get_ccline_opt() else {
        return 1;
    };

    let bytes = str.to_bytes_with_nul();
    let len = (bytes.len() - 1) as i32;
    realloc_cmdbuff(len + 1);
    p.cmdlen.set(len);
    p.cmdbuff_vec.borrow_mut()[..bytes.len()].copy_from_slice(bytes);

    p.cmdpos.set(if pos < 0 || pos > p.cmdlen.get() {
        p.cmdlen.get()
    } else {
        pos
    });
    GETLN_STATE.with(|s| s.new_cmdpos.set(p.cmdpos.get()));
    p.cmdbuff_replaced.set(true);

    redrawcmd();

    do_autocmd_cmdlinechanged(get_cmdline_type());

    0
}

/// Set the command line byte position to `pos`.
fn set_cmdline_pos(pos: i32) -> i32 {
    if get_ccline_opt().is_none() {
        return 1;
    }

    GETLN_STATE.with(|s| s.new_cmdpos.set(pos.max(0)));

    0
}

/// `"setcmdline()"` function.
pub fn f_setcmdline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_string_arg(argvars, 0) || !tv_check_for_opt_number_arg(argvars, 1) {
        return;
    }

    let mut pos: i32 = -1;
    if argvars[1].v_type != VarType::VarUnknown {
        let mut error = false;

        pos = (tv_get_number_chk(&argvars[1], Some(&mut error)) - 1) as i32;
        if error {
            return;
        }
        if pos < 0 {
            emsg(gettext(E_POSITIVE));
            return;
        }
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(set_cmdline_str(
        &tv_get_string(&argvars[0]),
        pos,
    ) as Varnumber);
}

/// `"setcmdpos()"` function.
pub fn f_setcmdpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let pos = (tv_get_number(&argvars[0]) - 1) as i32;

    if pos >= 0 {
        rettv.vval = crate::eval::typval::TypvalVval::Number(set_cmdline_pos(pos) as Varnumber);
    }
}

/// Return the first character of the current command line.
pub fn get_cmdline_firstc() -> i32 {
    get_cmdline_info().cmdfirstc.get()
}

/// Get indices that specify a range within a list from a string.
pub fn get_list_range(str_p: &mut &[u8], num1: &mut i32, num2: &mut i32) -> bool {
    let mut len: i32 = 0;
    let mut first = false;
    let mut num: Varnumber = 0;

    let mut p = skipwhite_slice(str_p);
    if p.first()
        .is_some_and(|&b0| b0 == b'-' || b0.is_ascii_digit())
    {
        vim_slice2nr(
            p,
            None,
            Some(&mut len),
            0,
            Some(&mut num),
            None,
            false,
            None,
        );
        p = &p[len as usize..];
        if num > i32::MAX as Varnumber {
            *str_p = p;
            return false;
        }

        *num1 = num as i32;
        first = true;
    }
    p = skipwhite_slice(p);
    if let Some(after_comma) = p.strip_prefix(b",") {
        p = skipwhite_slice(after_comma);
        vim_slice2nr(
            p,
            None,
            Some(&mut len),
            0,
            Some(&mut num),
            None,
            false,
            None,
        );
        if len > 0 {
            p = skipwhite_slice(&p[len as usize..]);
            if num > i32::MAX as Varnumber {
                *str_p = p;
                return false;
            }

            *num2 = num as i32;
        } else if !first {
            *str_p = p;
            return false;
        }
    } else if first {
        *num2 = *num1;
    }
    *str_p = p;
    true
}

pub fn cmdline_init() {
    CCLINE.with(|c| *c.borrow_mut() = std::rc::Rc::new(CmdlineInfo::new()));
}

/// Validate the `'cedit'` option.
pub fn validate_cedit(args: &mut Optset) -> Option<&'static CStr> {
    let newval_str = args.os_newval.data.string();
    let value = newval_str.as_bytes();
    if !value.is_empty() {
        let n = string_to_key(value);
        if n == 0 || vim_isprintc(n) {
            return Some(E_INVARG);
        }
    }
    None
}

/// The `'cedit'` option is changed.
pub fn did_set_cedit(_args: &mut Optset) -> Option<&'static CStr> {
    GETLN_STATE.with(|s| {
        s.cedit_key.set(if opt_vars().p_cedit.is_empty() {
            -1
        } else {
            string_to_key(&opt_vars().p_cedit.as_bytes())
        })
    });
    None
}

/// Synchronously calls `vim._core.cmdwin.<action>(...)`.
fn cmdwin_invoke(action: &CStr, firstc: i32, content: Option<&CStr>, pos: i32) {
    let fc: [u8; 2] = [firstc as u8, 0];
    let mut tv_args = [
        TypVal {
            v_type: VarType::VarString,
            v_lock: 0,
            vval: TypvalVval::from_cstr(crate::memory::bytes_as_cstr(&fc)),
        },
        TypVal {
            v_type: VarType::VarString,
            v_lock: 0,
            vval: TypvalVval::from_cstr(content.unwrap_or(c"")),
        },
        TypVal {
            v_type: VarType::VarNumber,
            v_lock: 0,
            vval: TypvalVval::Number((pos + 1) as Varnumber),
        },
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    nlua_call_typval(
        c"vim._core.cmdwin",
        action,
        if firstc != 0 {
            &mut tv_args
        } else {
            &mut tv_args[3..]
        },
        None,
    );
}

/// Calls `vim._core.cmdwin.<action>()`, synchronously.
pub fn cmdwin_do_action(action: &std::ffi::CStr) {
    cmdwin_invoke(action, 0, None, 0);
}

/// Deferred event: opens cmdwin after the cmdline-reader unwinds.
pub fn open_cmdwin_event(firstc: i32, content: Option<std::ffi::CString>, pos: i32) {
    cmdwin_invoke(c"open", firstc, content.as_deref(), pos);
}

/// Schedules cmdwin to open after the current cmdline-reader returns.
fn open_cmdwin() -> i32 {
    if text_locked() || g_st().cmdline_star.get() > 0 {
        beep_flush();
        return K_IGNORE;
    }
    if !g_st().cmdwin_buf.get().is_null() {
        beep_flush();
        return K_IGNORE;
    }
    let ft = get_cmdline_type();
    if ft != b':' as i32 && ft != b'/' as i32 && ft != b'?' as i32 {
        beep_flush();
        return K_IGNORE;
    }

    let content = (!get_cmdline_info().cmdbuff().is_null()).then(|| {
        let cmdbuff = get_cmdline_info().cmdbuff();
        let bytes = cmdbuff.slice(get_cmdline_info().cmdlen.get() as usize);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        std::ffi::CString::new(&bytes[..end]).unwrap()
    });
    let pos = get_cmdline_info().cmdpos.get();
    get_cmdline_info().cmdlen.set(0);
    get_cmdline_info().cmdpos.set(0);

    loop_schedule_deferred(
        crate::event::r#loop::main_loop_ptr(),
        Event::CmdwinOpen {
            firstc: ft,
            content,
            pos,
        },
    );
    CTRL_C_CONST
}

/// @return true if `curbuf` is the cmdwin and we're not editing the command line.
pub fn is_in_cmdwin() -> bool {
    bt_cmdwin(g_st().curbuf.get().opt()) && get_cmdline_type() == NUL as i32
}

/// Get script string.
pub fn script_get(eap: &mut Exarg, lenp: &mut usize) -> Option<std::ffi::CString> {
    let mut cmd = eap.arg.clone();

    if !(cmd).as_bytes().starts_with(b"<<") || eap.ea_getline.is_none() {
        *lenp = eap.arg.len();
        return if eap.skip {
            None
        } else {
            Some(eap.arg.to_cstring())
        };
    }
    cmd = cmd.wrapping_add(2);

    let l = heredoc_get(eap, (cmd).as_cstr(), true)?;

    let mut buf: Vec<u8> = if !eap.skip {
        Vec::with_capacity(0x400)
    } else {
        Vec::new()
    };

    for li in crate::eval::typval::tv_list_iter(Some(&*l)) {
        if !eap.skip {
            let li_tv = li.tv();
            let s = tv_get_string(&li_tv);
            buf.extend_from_slice(s.to_bytes());
            buf.push(b'\n');
        }
    }
    *lenp = buf.len();
    tv_list_free(Some(&l));
    if !eap.skip {
        Some(std::ffi::CString::new(buf).unwrap_or_default())
    } else {
        None
    }
}

/// Used by `f_input()` and `f_inputdialog()` functions.
pub fn get_user_input(argvars: &[TypVal], rettv: &mut TypVal, inputdialog: bool, secret: bool) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    if g_st().cmdpreview.get() {
        return;
    }

    let prompt_cow: std::borrow::Cow<'_, CStr>;
    let mut defstr_cow: std::borrow::Cow<'_, CStr> = std::borrow::Cow::Borrowed(c"");
    let mut cancelreturn: Option<TypVal> = None;
    let mut cancelreturn_strarg2 = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    };
    let mut xp_name_cow: Option<std::borrow::Cow<'_, CStr>> = None;
    let input_callback: Callback = Callback::new();
    let mut prompt_buf: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
    let mut defstr_buf: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
    let mut cancelreturn_buf: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
    let mut xp_name_buf: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
    let dict;
    if argvars[0].v_type == VarType::VarDict {
        if argvars[1].v_type != VarType::VarUnknown {
            emsg(gettext(c"E5050: {opts} must be the only argument"));
            return;
        }
        dict = argvars[0].vval.v_dict();
        let dict_ref = dict.opt();
        let Some(p_cstr) =
            tv_dict_get_string_buf_chk(dict_ref, c"prompt", 6, &mut prompt_buf, Some(c""))
        else {
            return;
        };
        prompt_cow = p_cstr;
        let Some(defstr_cstr) =
            tv_dict_get_string_buf_chk(dict_ref, c"default", 7, &mut defstr_buf, Some(c""))
        else {
            return;
        };
        defstr_cow = defstr_cstr;
        if let Some(cancelreturn_di) = tv_dict_find(dict_ref, c"cancelreturn", 12) {
            cancelreturn = Some(cancelreturn_di.tv().clone());
        }
        if tv_dict_find(dict_ref, c"completion", 10).is_none() {
            xp_name_cow = None;
        } else {
            let Some(xp_cstr) =
                tv_dict_get_string_buf_chk(dict_ref, c"completion", 10, &mut xp_name_buf, None)
            else {
                return;
            };
            xp_name_cow = Some(xp_cstr);
        }
        if !tv_dict_get_callback(dict.opt(), c"highlight", 9, &input_callback) {
            return;
        }
    } else {
        let Some(p_cstr) = tv_get_string_buf_chk(&argvars[0], &mut prompt_buf) else {
            return;
        };
        prompt_cow = std::borrow::Cow::Borrowed(p_cstr);
        if argvars[1].v_type != VarType::VarUnknown {
            let Some(defstr_cstr) = tv_get_string_buf_chk(&argvars[1], &mut defstr_buf) else {
                return;
            };
            defstr_cow = std::borrow::Cow::Borrowed(defstr_cstr);
            if argvars[2].v_type != VarType::VarUnknown {
                let Some(strarg2) = tv_get_string_buf_chk(&argvars[2], &mut cancelreturn_buf)
                else {
                    return;
                };
                if inputdialog {
                    cancelreturn_strarg2.v_type = VarType::VarString;
                    cancelreturn_strarg2.vval = crate::eval::typval::TypvalVval::from_cstr(strarg2);
                    cancelreturn = Some(cancelreturn_strarg2);
                } else {
                    xp_name_cow = Some(std::borrow::Cow::Borrowed(strarg2));
                }
            }
        }
    }
    let prompt_cstr: &CStr = &prompt_cow;
    let defstr: &CStr = &defstr_cow;
    let xp_name: Option<&CStr> = xp_name_cow.as_deref();

    let mut xp_type: i32 = EXPAND_NOTHING;
    let mut xp_arg: Option<std::ffi::CString> = None;
    if let Some(xp_name_cstr) = xp_name {
        let mut argt: u32 = 0;
        if !parse_compl_arg(
            xp_name_cstr.to_bytes(),
            &mut xp_type,
            &mut argt,
            &mut xp_arg,
        ) {
            return;
        }
    }

    let mut p_cstr = prompt_cstr;
    if !ui_has(K_UICMDLINE) {
        let prompt_bytes = prompt_cstr.to_bytes();
        if let Some(nl_idx) = prompt_bytes.iter().rposition(|&b| b == b'\n') {
            p_cstr = crate::memory::CStrSubslice::advance(prompt_cstr, nl_idx + 1);
            msg_start();
            msg_clr_eos();
            let plen = (nl_idx + 1) as isize;
            msg_puts_len(&prompt_bytes[..=nl_idx], plen, get_echo_hl_id(), false);
            g_st().msg_didout.set(false);
            msg_starthere();
        }
    }
    g_st().cmdline_row.set(g_st().msg_row.get());

    stuff_readbuff_special(defstr);

    let save_ex_normal_busy = g_st().ex_normal_busy.get();
    g_st().ex_normal_busy.set(0);
    atom_payload_start();
    let res = getcmdline_prompt(
        if secret { NUL as i32 } else { b'@' as i32 },
        Some(p_cstr),
        get_echo_hl_id(),
        xp_type,
        xp_arg.as_deref(),
        input_callback.clone(),
        false,
        None,
    );
    atom_payload_end();
    g_st().ex_normal_busy.set(save_ex_normal_busy);
    callback_free(&input_callback);

    if let Some(res_cstr) = res {
        crate::eval::typval::tv_set_string_len(&mut *rettv, Some(res_cstr.as_bytes()));
    } else if let Some(ref cancelreturn) = cancelreturn {
        tv_copy(cancelreturn, &mut *rettv);
    }

    g_st().need_wait_return.set(false);
    g_st().msg_didout.set(false);
}

/// `"wildtrigger()"` function.
pub fn f_wildtrigger(_argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if (g_st().state_global.get() & MODE_CMDLINE) == 0
        || char_avail()
        || g_st().wild_menu_showing.get() != 0
        || cmdline_pum_active()
    {
        return;
    }

    let cmd_type = get_cmdline_type();

    if cmd_type == b':' as i32 || cmd_type == b'/' as i32 || cmd_type == b'?' as i32 {
        let key_string: [u8; 4] = [K_SPECIAL, KS_EXTRA, KE_WILD, NUL];

        ins_typebuf(
            CStr::from_bytes_with_nul(&key_string).unwrap(),
            REMAP_NONE,
            0,
            true,
            false,
        );
    }
}
