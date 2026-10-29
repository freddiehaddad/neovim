//! Functions for entering and editing an Ex command line (`src/nvim/ex_getln.c`).

use crate::api::extmark::nvim_create_namespace;
use crate::api::private::helpers::{
    HLF_E, TryState, add_c, api_clear_error, api_free_array, arena_array, array_obj,
    cbuf_as_string, cstr_as_obj, cstr_as_string, error_init, error_set, integer_obj, string_obj,
    try_enter, try_leave,
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
    NUL, ascii_isalpha, ascii_isdigit, ascii_isspace, ascii_iswhite, is_special, skipwhite,
    vim_is_idc, vim_isprintc, vim_iswordc, vim_str2nr,
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
    HIST_INVALID, HIST_SEARCH, add_to_history, get_hisidx, get_hislen, get_histentry,
    hist_char2type, init_history,
};
use crate::context::{CtxSwitch, K_CTX_KEEP_DIRS, ctx_restore, ctx_switch};
use crate::cursor::{
    CursorPos, clearpos, coladvance, equalpos, gchar_cursor, get_cursor_pos_ptr, lt,
};
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
    tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_get_string_chk, tv_list_first,
    tv_list_free, tv_list_last, tv_list_len,
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
use crate::math::{FAIL, OK};
use crate::mbyte::{
    UTF8LEN_TAB_ZERO, mb_cptr2char_adv, mb_get_class, mb_off_next, mb_prevptr, mb_tolower,
    ptr2cells, utf_char2bytes, utf_char2len, utf_head_off, utf_iscomposing_first, utf_ptr2cells,
    utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{decl, incl};
use crate::memory::{Arena, arena_alloc, arena_finish, arena_mem_free};
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
    CTRL_H_CONST, CTRL_V_CONST, MAGIC_ALL, MAGIC_ON, MAXCOL, NUMBUFLEN, RegMagic, skip_regexp_ex,
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
    WSP_BOT, WinInfoInfo, close_windows, global_stl_height, lastwin_nofloating,
    may_trigger_win_scrolled_resized, win_enter, win_size_restore, win_size_save, win_split,
};
use std::ffi::{CStr, c_char};
use std::ptr;

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
    xpc: ExpandCtx,
    b_im_use_insert: Option<bool>,
    b_im_ptr_buf: *mut FileBuf,
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
            xpc: ExpandCtx::new(),
            b_im_use_insert: None,
            b_im_ptr_buf: ptr::null_mut(),
            cmdline_type: 0,
            event_cmdlineleavepre_triggered: false,
            did_hist_navigate: false,
        }
    }
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
    buf: *mut FileBuf,
    save_b_p_ma: i32,
    save_b_changed: i32,
    save_b_op_start: CursorPos,
    save_b_op_end: CursorPos,
    undo_checkpoint: UndoCheckpoint,
}

impl CpBufInfo {
    const fn new() -> Self {
        Self {
            buf: ptr::null_mut(),
            save_b_p_ma: 0,
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
#[derive(Clone, Copy)]
struct CpWinInfo {
    win: *mut WinInfoInfo,
    save_w_cursor: CursorPos,
    save_viewstate: Viewstate,
    save_w_p_cul: i32,
    save_w_p_cuc: i32,
}

impl CpWinInfo {
    const fn new() -> Self {
        Self {
            win: ptr::null_mut(),
            save_w_cursor: CursorPos::new(),
            save_viewstate: Viewstate::new(),
            save_w_p_cul: 0,
            save_w_p_cuc: 0,
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
    last_prompt_id: u32,
    cmdline_level: i32,
    ccline: CmdlineInfo,
    new_cmdpos: i32,
    cmdline_block: Array,
    getln_interrupted_highlight: bool,
    cedit_key: i32,
    cmdpreview_bufnr: ObjHandle,
    cmdpreview_ns: i32,
    prev_prompt_id: u32,
    prev_prompt_errors: i32,
}

impl ExGetlnState {
    const fn new() -> Self {
        Self {
            last_prompt_id: 0,
            cmdline_level: 0,
            ccline: CmdlineInfo::new(),
            new_cmdpos: 0,
            cmdline_block: Array {
                size: 0,
                capacity: 0,
                items: ptr::null_mut(),
            },
            getln_interrupted_highlight: false,
            cedit_key: -1,
            cmdpreview_bufnr: 0,
            cmdpreview_ns: 0,
            prev_prompt_id: u32::MAX,
            prev_prompt_errors: 0,
        }
    }
}

static GETLN_STATE: GlobalState<ExGetlnState> = GlobalState::new(ExGetlnState::new());

#[inline]
fn getln_st() -> &'static mut ExGetlnState {
    unsafe { &mut *GETLN_STATE.as_ptr() }
}

use crate::api::private::helpers::array_add;

fn trigger_cmd_autocmd(typechar: i32, evt: AuEvent) {
    let mut typestr: [c_char; 2] = [typechar as c_char, NUL];
    apply_autocmds(
        evt,
        typestr.as_mut_ptr(),
        typestr.as_mut_ptr(),
        false,
        g_st().curbuf,
    );
}

fn init_incsearch_state(s: &mut IncsearchState) {
    s.winid = curwin_mut().handle;
    s.match_start = curwin_mut().w_cursor;
    s.did_incsearch = false;
    s.incsearch_postponed = false;
    clearpos(&mut s.match_end);
    s.save_cursor = curwin_mut().w_cursor;
    s.search_start = curwin_mut().w_cursor;
    save_viewstate(curwin_mut(), &mut s.init_viewstate);
    save_viewstate(curwin_mut(), &mut s.old_viewstate);
}

fn set_search_match(t: &mut CursorPos) {
    t.lnum += g_st().search_global.match_lines;
    t.col = g_st().search_global.match_endcol;
    if t.lnum > curbuf_mut().b_ml.ml_line_count {
        t.lnum = curbuf_mut().b_ml.ml_line_count;
        coladvance(curwin_mut(), MAXCOL);
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
    let mut dummy: *const c_char = ptr::null();
    let mut magic: RegMagic = 0;

    *skiplen = 0;
    *patlen = getln_st().ccline.cmdlen;

    // Default range
    g_st().search_global.first_line = 0;
    g_st().search_global.last_line = MAXLNUM;

    let mut ea: Exarg = Exarg::new();
    ea.line1 = 1;
    ea.line2 = 1;
    ea.cmd = getln_st().ccline.cmdbuff;
    ea.addr_type = ADDR_LINES;

    let mut dummy_cmdmod: Cmdmod = Cmdmod::new();
    // Skip over command modifiers
    parse_command_modifiers(&mut ea, &raw mut dummy, &mut dummy_cmdmod, true);

    // Skip over the range to find the command.
    let cmd = skip_range(ea.cmd, None);
    if !crate::strings::vim_haschr(c"sgvlu", (crate::memory::cstr_first_byte(cmd)) as i32) {
        return false;
    }

    // Skip over command name to find pattern separator
    let mut p = cmd;
    while ascii_isalpha(unsafe { *p } as i32) {
        p = p.wrapping_add(1);
    }
    if crate::memory::cstr_is_empty(skipwhite(p)) {
        return false;
    }

    let cmd_len = crate::memory::ptr_offset_from(p, cmd) as usize;
    if (crate::memory::cstr_prefix_bytes(cmd, cmd_len)
        == crate::memory::cstr_prefix_bytes(c"substitute", cmd_len))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len)
            == crate::memory::cstr_prefix_bytes(c"smagic", cmd_len))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(3))
            == crate::memory::cstr_prefix_bytes(c"snomagic", cmd_len.max(3)))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len)
            == crate::memory::cstr_prefix_bytes(c"vglobal", cmd_len))
    {
        if crate::memory::cstr_first_byte(cmd) == b's'
            && crate::memory::cstr_byte_at(cmd, 1) == b'm'
        {
            *is_magic = true;
        } else if crate::memory::cstr_first_byte(cmd) == b's'
            && crate::memory::cstr_byte_at(cmd, 1) == b'n'
        {
            *is_magic = false;
        }
    } else if (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(3))
        == crate::memory::cstr_prefix_bytes(c"sort", cmd_len.max(3)))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(3))
            == crate::memory::cstr_prefix_bytes(c"uniq", cmd_len.max(3)))
    {
        // skip over ! and flags
        if crate::memory::cstr_first_byte(p) == b'!' {
            p = skipwhite(p.wrapping_add(1));
        }
        loop {
            p = skipwhite(p);
            if !ascii_isalpha(unsafe { *p } as i32) {
                break;
            }
            p = p.wrapping_add(1);
        }
        if crate::memory::cstr_is_empty(p) {
            return false;
        }
    } else if (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(3))
        == crate::memory::cstr_prefix_bytes(c"vimgrep", cmd_len.max(3)))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(8))
            == crate::memory::cstr_prefix_bytes(c"vimgrepadd", cmd_len.max(8)))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(2))
            == crate::memory::cstr_prefix_bytes(c"lvimgrep", cmd_len.max(2)))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len.max(9))
            == crate::memory::cstr_prefix_bytes(c"lvimgrepadd", cmd_len.max(9)))
        || (crate::memory::cstr_prefix_bytes(cmd, cmd_len)
            == crate::memory::cstr_prefix_bytes(c"global", cmd_len))
    {
        // skip optional "!"
        if crate::memory::cstr_first_byte(p) == b'!' {
            p = p.wrapping_add(1);
            if crate::memory::cstr_is_empty(skipwhite(p)) {
                return false;
            }
        }
        if crate::memory::cstr_first_byte(cmd) != b'g' {
            delim_optional = true;
        }
    } else {
        return false;
    }

    p = skipwhite(p);
    let delim = if delim_optional && vim_is_idc((crate::memory::cstr_first_byte(p)) as i32) {
        b' ' as i32
    } else {
        let d = unsafe { *p } as i32;
        p = p.wrapping_add(1);
        d
    };
    *search_delim = delim;

    let end = skip_regexp_ex(
        p,
        delim,
        i32::from(*is_magic),
        false,
        None,
        None,
        Some(&mut magic),
    );
    let use_last_pat = end == p && unsafe { *end } as i32 == delim;

    if end == p && !use_last_pat {
        return false;
    }

    // Skip if the pattern matches everything (e.g., for 'hlsearch')
    if !use_last_pat {
        let c = unsafe { *end };
        unsafe {
            *end = NUL;
        }
        let empty = empty_pattern_magic(p, crate::memory::ptr_offset_from(end, p) as usize, magic);
        unsafe {
            *end = c;
        }
        if empty {
            return false;
        }
    }

    // Found a non-empty pattern or //
    *skiplen = crate::memory::ptr_offset_from(p, getln_st().ccline.cmdbuff) as i32;
    *patlen = crate::memory::ptr_offset_from(end, p) as i32;

    // Parse the address range
    let save_cursor = curwin_mut().w_cursor;
    curwin_mut().w_cursor = *incsearch_start;

    parse_cmd_address(&mut ea, &raw mut dummy, true);

    if ea.addr_count > 0 {
        // Allow for reverse match.
        g_st().search_global.first_line = ea.line2.min(ea.line1);
        g_st().search_global.last_line = ea.line2.max(ea.line1);
    } else if crate::memory::cstr_first_byte(cmd) == b's'
        && crate::memory::cstr_byte_at(cmd, 1) != b'o'
    {
        // :s defaults to the current line
        g_st().search_global.first_line = curwin_mut().w_cursor.lnum;
        g_st().search_global.last_line = curwin_mut().w_cursor.lnum;
    }

    curwin_mut().w_cursor = save_cursor;
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
    *patlen = getln_st().ccline.cmdlen;

    if opt_vars().p_is == 0 || g_st().cmd_silent {
        return false;
    }

    // By default search all lines
    g_st().search_global.first_line = 0;
    g_st().search_global.last_line = MAXLNUM;

    if firstc == b'/' as i32 || firstc == b'?' as i32 {
        *search_delim = firstc;
        return true;
    }

    if firstc != b':' as i32 {
        return false;
    }

    g_st().emsg_off += 1;
    let retval = parse_pattern_and_range(
        &mut is_state.search_start,
        &mut *search_delim,
        &mut *skiplen,
        &mut *patlen,
        &mut *magic,
    );
    g_st().emsg_off -= 1;

    retval
}

/// May do `'incsearch'` highlighting if desired.
fn may_do_incsearch_highlighting(firstc: i32, count: i32, s: &mut IncsearchState) {
    let mut skiplen: i32 = 0;
    let mut patlen: i32 = 0;
    let mut search_delim: i32 = 0;
    let mut magic: bool = opt_vars().p_magic != 0;

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
    let mut next_char = unsafe {
        *getln_st()
            .ccline
            .cmdbuff
            .wrapping_add((skiplen + patlen) as usize)
    };
    let use_last_pat = patlen == 0
        && skiplen > 0
        && unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add((skiplen - 1) as usize)
        } == next_char;

    if patlen != 0 || use_last_pat {
        ui_busy_start();
        ui_flush();
    }

    if g_st().search_global.first_line == 0 {
        // start at the original cursor position
        curwin_mut().w_cursor = s.search_start;
    } else if g_st().search_global.first_line > curbuf_mut().b_ml.ml_line_count {
        // start after the last line
        curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
        curwin_mut().w_cursor.col = MAXCOL;
    } else {
        // start at the first line in the range
        curwin_mut().w_cursor.lnum = g_st().search_global.first_line;
        curwin_mut().w_cursor.col = 0;
    }

    let mut found: i32 = 0; // do_search() result

    if patlen != 0 || use_last_pat {
        let mut search_flags = SEARCH_OPT + SEARCH_NOOF + SEARCH_PEEK;
        if opt_vars().p_hls == 0 {
            search_flags += SEARCH_KEEP;
        }
        if g_st().search_global.first_line != 0 {
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
        unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add((skiplen + patlen) as usize) = NUL;
        }
        g_st().emsg_off += 1; // So it doesn't beep if bad expr
        found = do_search(
            None,
            if firstc == b':' as i32 {
                b'/' as i32
            } else {
                firstc
            },
            search_delim,
            getln_st().ccline.cmdbuff.wrapping_add(skiplen as usize),
            patlen as usize,
            count,
            search_flags,
            magic,
            Some(&mut sia),
        );
        g_st().emsg_off -= 1;
        unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add((skiplen + patlen) as usize) = next_char;
        }
        if curwin_mut().w_cursor.lnum < g_st().search_global.first_line
            || curwin_mut().w_cursor.lnum > g_st().search_global.last_line
        {
            // match outside of address range
            found = 0;
            curwin_mut().w_cursor = s.search_start;
        }

        // if interrupted while searching, behave like it failed
        if g_st().got_int {
            vpeekc(); // remove <C-C> from input stream
            g_st().got_int = false; // don't abandon the command line
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

    g_st().search_global.hl_match = found != 0; // add or remove search match position

    // first restore the old curwin values, so the screen is
    // positioned in the same way as the actual search command
    restore_viewstate(curwin_mut(), &s.old_viewstate);
    changed_cline_bef_curs(curwin_mut());
    update_topline(curwin_mut());

    let mut end_pos = curwin_mut().w_cursor;
    if found != 0 {
        s.match_start = curwin_mut().w_cursor;
        set_search_match(&mut curwin_mut().w_cursor);
        validate_cursor(curwin_mut());
        s.match_end = curwin_mut().w_cursor;
        curwin_mut().w_cursor = end_pos;
        end_pos = s.match_end;
    }

    // Disable 'hlsearch' highlighting if the pattern matches
    // everything. Avoids a flash when typing "foo\|".
    if !use_last_pat {
        next_char = unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add((skiplen + patlen) as usize)
        };
        unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add((skiplen + patlen) as usize) = NUL;
        }
        if empty_pattern(
            getln_st().ccline.cmdbuff.wrapping_add(skiplen as usize),
            patlen as usize,
            search_delim,
        ) && !g_st().search_global.no_hlsearch
        {
            redraw_all_later(UPD_SOME_VALID);
            set_no_hlsearch(true);
        }
        unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add((skiplen + patlen) as usize) = next_char;
        }
    }

    validate_cursor(curwin_mut());

    // May redraw the status line to show the cursor position.
    if opt_vars().p_ru != 0 && (curwin_mut().w_status_height > 0 || global_stl_height() > 0) {
        curwin_mut().w_redr_status = true;
    }

    redraw_later(Some(crate::globals::curwin_mut()), UPD_SOME_VALID);
    update_screen();
    g_st().search_global.hl_match = false;
    restore_last_search_pattern();

    // Leave it at the end to make CTRL-R CTRL-W work.  But not when beyond the
    // end of the pattern, e.g. for ":s/pat/".
    if !crate::memory::cstr_is_empty(
        getln_st()
            .ccline
            .cmdbuff
            .wrapping_add((skiplen + patlen) as usize),
    ) {
        curwin_mut().w_cursor = s.search_start;
    } else if found != 0 {
        curwin_mut().w_cursor = end_pos;
        curwin_mut().w_valid_cursor = end_pos; // mark as valid for cmdline_show redraw
    }

    msg_starthere();
    redrawcmdline();
    s.did_incsearch = true;
    // Fire WinScrolled/WinResized last, after all state is finalized: the
    // autocmd may mutate curwin or getln_st().ccline via arbitrary user code.
    may_trigger_win_scrolled_resized();
}

/// When CTRL-L typed: add character from the match to the pattern.
/// May set `*c` to the added character.
/// Return `OK` when calling `command_line_not_changed`.
fn may_add_char_to_search(firstc: i32, c: &mut i32, s: &mut IncsearchState) -> bool {
    let mut skiplen: i32 = 0;
    let mut patlen: i32 = 0;
    let mut search_delim: i32 = 0;
    let mut magic: bool = opt_vars().p_magic != 0;

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
        curwin_mut().w_cursor = s.match_end;
        *c = gchar_cursor();
        if *c != NUL as i32 {
            // If 'ignorecase' and 'smartcase' are set and the
            // command line has no uppercase characters, convert
            // the character to lowercase
            if opt_vars().p_ic != 0
                && opt_vars().p_scs != 0
                && !pat_has_uppercase(getln_st().ccline.cmdbuff.wrapping_add(skiplen as usize))
            {
                *c = mb_tolower(*c);
            }
            let special_chars = if opt_vars().p_magic != 0 {
                c"\\~^$.*[".as_ptr()
            } else {
                c"\\^$".as_ptr()
            };
            if *c == search_delim || crate::strings::vim_haschr(special_chars, *c) {
                // put a backslash before special characters
                stuffchar_readbuff(*c);
                *c = b'\\' as i32;
            }
            // add any composing characters
            if utf_char2len(*c) != utfc_ptr2len(get_cursor_pos_ptr()) {
                let save_c = *c;
                while utf_char2len(*c) != utfc_ptr2len(get_cursor_pos_ptr()) {
                    curwin_mut().w_cursor.col += utf_char2len(*c);
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
        curwin_mut().w_cursor = s.save_cursor;
    } else {
        if !equalpos(s.save_cursor, s.search_start) {
            // put the '" mark at the original position
            curwin_mut().w_cursor = s.save_cursor;
            setpcmark();
        }
        curwin_mut().w_cursor = s.search_start;
    }
    restore_viewstate(curwin_mut(), &s.old_viewstate);
    g_st().search_global.hl_match = false;

    // by default search all lines
    g_st().search_global.first_line = 0;
    g_st().search_global.last_line = MAXLNUM;

    validate_cursor(curwin_mut()); // needed for TAB
    status_redraw_all();
    redraw_all_later(UPD_SOME_VALID);
    if call_update_screen {
        update_screen();
        may_trigger_win_scrolled_resized();
    }
}

/// Initialize the current command-line info.
fn init_ccline(firstc: i32, indent: i32) {
    getln_st().ccline.overstrike = 0; // always start in insert mode

    debug_assert!(indent >= 0);

    // set some variables for redrawcmd()
    getln_st().ccline.cmdfirstc = if firstc == b'@' as i32 { 0 } else { firstc };
    getln_st().ccline.cmdindent = if firstc > 0 { indent } else { 0 };

    // alloc initial getln_st().ccline.cmdbuff
    alloc_cmdbuff(indent + 50);
    getln_st().ccline.cmdpos = 0;
    getln_st().ccline.cmdlen = 0;
    unsafe {
        *getln_st().ccline.cmdbuff = NUL;
    }

    getln_st().ccline.last_colors = ColoredCmdline::new();
    sb_text_start_cmdline();

    // autoindent for :insert and :append
    if firstc <= 0 {
        unsafe {
            ptr::write_bytes(
                getln_st().ccline.cmdbuff.cast::<u8>(),
                b' ',
                indent as usize,
            )
        };
        unsafe {
            *getln_st().ccline.cmdbuff.wrapping_add(indent as usize) = NUL;
        }
        getln_st().ccline.cmdpos = indent;
        getln_st().ccline.cmdspos = indent;
        getln_st().ccline.cmdlen = indent;
    }
}

fn ui_ext_cmdline_hide(abort: bool) {
    if ui_has(K_UICMDLINE) {
        g_st().cmdline_was_last_drawn = false;
        getln_st().ccline.redraw_state = K_CMD_REDRAW_NONE;
        ui_call_cmdline_hide(getln_st().ccline.level as Integer, abort);
    }
}

/// Internal entry point for cmdline mode.
fn command_line_enter(
    firstc: i32,
    count: i32,
    indent: i32,
    clear_ccline: bool,
) -> Option<std::ffi::CString> {
    getln_st().cmdline_level += 1;

    let save_cmdpreview = g_st().cmdpreview;
    g_st().cmdpreview = false;
    let mut state = CommandLineState::new();
    state.firstc = firstc;
    state.count = count;
    state.indent = indent;
    state.save_msg_scroll = g_st().msg_scroll;
    state.save_state = g_st().state_global;
    state.prev_cmdpos = -1;
    state.ignore_drag_release = true;
    let s = &mut state;
    s.save_p_icm = Some(crate::memory::cstr_to_cstring(opt_vars().p_icm));
    init_incsearch_state(&mut s.is_state);
    let mut did_save_ccline = false;

    if !getln_st().ccline.cmdbuff.is_null() {
        debug_assert!(clear_ccline);
        save_cmdline();
        did_save_ccline = true;
    } else if clear_ccline {
        getln_st().ccline = CmdlineInfo::new();
    }

    if s.firstc == -1 {
        s.firstc = NUL as i32;
        s.break_ctrl_c = true;
    }

    init_ccline(s.firstc, s.indent);
    debug_assert!(!getln_st().ccline.cmdbuff.is_null());
    getln_st().ccline.prompt_id = getln_st().last_prompt_id;
    getln_st().last_prompt_id = getln_st().last_prompt_id.wrapping_add(1);
    getln_st().ccline.level = getln_st().cmdline_level;

    if getln_st().cmdline_level != 50 {
        expand_init(&mut s.xpc);
        getln_st().ccline.xpc = &mut s.xpc;
        clear_cmdline_orig();

        g_st().cmdmsg_rl = curwin_mut().w_onebuf_opt.wo_rl != 0
            && crate::memory::cstr_first_byte(curwin_mut().w_onebuf_opt.wo_rlc) == b's'
            && (s.firstc == b'/' as i32 || s.firstc == b'?' as i32);

        msg_grid_validate();

        g_st().redir_off = true; // don't redirect the typed command
        if !g_st().cmd_silent {
            gotocmdline(true);
            redrawcmdprompt(); // draw prompt or indent
            getln_st().ccline.cmdspos = cmd_startcol();
        }
        s.xpc.xp_context = EXPAND_NOTHING;
        s.xpc.xp_backslash = XP_BS_NONE;
        #[cfg(not(windows))]
        {
            s.xpc.xp_shell = false;
        }

        if getln_st().ccline.input_fn != 0 {
            s.xpc.xp_context = getln_st().ccline.xp_context;
            s.xpc.xp_pattern = getln_st().ccline.cmdbuff;
            s.xpc.xp_arg = unsafe { (&raw const getln_st().ccline.xp_arg).as_ref() }
                .unwrap()
                .as_ref()
                .map_or(ptr::null_mut(), |a| a.as_ptr().cast_mut());
        }

        g_st().msg_scroll = 0;

        g_st().state_global = MODE_CMDLINE;

        if s.firstc == b'/' as i32 || s.firstc == b'?' as i32 || s.firstc == b'@' as i32 {
            let (use_insert, im_val) = if curbuf_mut().b_p_imsearch == B_IMODE_USE_INSERT {
                (true, curbuf_mut().b_p_iminsert)
            } else {
                (false, curbuf_mut().b_p_imsearch)
            };
            s.b_im_use_insert = Some(use_insert);
            s.b_im_ptr_buf = g_st().curbuf;
            if im_val == B_IMODE_LMAP {
                g_st().state_global |= MODE_LANGMAP;
            }
        }

        setmouse();

        s.cmdline_type = if firstc > 0 { firstc } else { b'-' as i32 };
        let mut err = error_init();
        let mut firstcbuf: [c_char; 2] = [s.cmdline_type as c_char, 0];

        if has_event(EVENT_CMDLINEENTER) {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let dict = get_v_event(&mut save_v_event);

            tv_dict_add_str(&mut *dict, c"cmdtype", firstcbuf.as_ptr());
            tv_dict_add_nr(
                &mut *dict,
                c"cmdlevel",
                getln_st().ccline.level as Varnumber,
            );
            tv_dict_set_keys_readonly(&mut *dict);
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            apply_autocmds(
                EVENT_CMDLINEENTER,
                firstcbuf.as_mut_ptr(),
                firstcbuf.as_mut_ptr(),
                false,
                g_st().curbuf,
            );
            restore_v_event(&mut *dict, &mut save_v_event);
            try_leave(&tstate, &mut err);

            if error_set(&err) {
                if !ui_has(K_UIMESSAGES) {
                    msg_putchar(b'\n' as i32);
                }
                g_st().msg_scroll = 1;
                msg_puts_hl(err.msg_ptr(), HLF_E, true);
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

        if g_st().did_emsg != 0 {
            redrawcmd();
        }

        if !g_st().cmd_silent {
            let mut found_one = false;

            for wp in for_all_windows_mut() {
                if !crate::memory::cstr_is_empty(opt_vars().p_stl)
                    || !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_stl)
                    || !crate::memory::cstr_is_empty(opt_vars().p_wbr)
                    || !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_wbr)
                {
                    wp.w_redr_status = true;
                    found_one = true;
                }
            }

            if !crate::memory::cstr_is_empty(opt_vars().p_tal) {
                g_st().redraw_tabline = true;
                found_one = true;
            }

            if redraw_custom_title_later() {
                found_one = true;
            }

            if found_one {
                redraw_statuslines();
            }
        }

        g_st().did_emsg = 0;
        g_st().got_int = false;

        state_enter(&mut *s);

        if !s.event_cmdlineleavepre_triggered {
            set_vim_var_char(s.c);
            trigger_cmd_autocmd(s.cmdline_type, EVENT_CMDLINELEAVEPRE);
        }

        if has_event(EVENT_CMDLINELEAVE) {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let dict = get_v_event(&mut save_v_event);

            tv_dict_add_str(&mut *dict, c"cmdtype", firstcbuf.as_ptr());
            tv_dict_add_nr(
                &mut *dict,
                c"cmdlevel",
                getln_st().ccline.level as Varnumber,
            );
            tv_dict_set_keys_readonly(&mut *dict);
            tv_dict_add_bool(
                &mut *dict,
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
            apply_autocmds(
                EVENT_CMDLINELEAVE,
                firstcbuf.as_mut_ptr(),
                firstcbuf.as_mut_ptr(),
                false,
                g_st().curbuf,
            );
            try_leave(&tstate, &mut err);
            if tv_dict_get_number(Some(&*dict), c"abort") != 0 {
                s.gotesc = true;
            }
            restore_v_event(&mut *dict, &mut save_v_event);
        }

        g_st().cmdmsg_rl = false;

        if cmdline_pum_active() {
            cmdline_pum_remove(false);
        } else {
            pum_check_clear();
        }
        wildmenu_cleanup(&mut getln_st().ccline);
        s.did_wild_list = false;
        s.wim_index = 0;

        expand_cleanup(&mut s.xpc);
        getln_st().ccline.xpc = ptr::null_mut();
        clear_cmdline_orig();

        finish_incsearch_highlighting(s.gotesc, &mut s.is_state, false);

        if !getln_st().ccline.cmdbuff.is_null() {
            if s.histype != HIST_INVALID
                && getln_st().ccline.cmdlen != 0
                && s.firstc != NUL as i32
                && (s.some_key_typed || s.histype == HIST_SEARCH)
            {
                add_to_history(
                    s.histype,
                    getln_st().ccline.cmdbuff,
                    getln_st().ccline.cmdlen as usize,
                    true,
                    if s.histype == HIST_SEARCH {
                        s.firstc
                    } else {
                        NUL as i32
                    },
                );
                if s.firstc == b':' as i32 {
                    g_st().new_last_cmdline =
                        Some(crate::memory::cstr_from_bytes(crate::memory::cstr_slice(
                            getln_st().ccline.cmdbuff,
                            getln_st().ccline.cmdlen as usize,
                        )));
                }
            }

            if !s.gotesc && getln_st().ccline.level == 1 {
                atom_cmdline_set(
                    s.firstc,
                    getln_st().ccline.cmdbuff,
                    getln_st().ccline.cmdlen as usize,
                );
            }

            if s.gotesc {
                abandon_cmdline();
            }
        }

        msg_check();
        if opt_vars().p_ch == 0 && !ui_has(K_UIMESSAGES) {
            set_must_redraw(UPD_VALID);
        }
        g_st().msg_scroll = s.save_msg_scroll;
        g_st().redir_off = false;

        if error_set(&err) {
            if !ui_has(K_UIMESSAGES) {
                msg_putchar(b'\n' as i32);
            }
            emsg(err.msg_ptr());
            g_st().did_emsg = 0;
            api_clear_error(&mut err);
        }

        if s.some_key_typed && !error_set(&err) {
            g_st().need_wait_return = false;
        }

        set_option_direct(
            K_OPT_INCCOMMAND,
            cstr_as_obj(s.save_p_icm.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr())),
            0,
            SID_NONE,
        );
        g_st().state_global = s.save_state;
        if g_st().cmdpreview != save_cmdpreview {
            g_st().cmdpreview = save_cmdpreview;
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
    getln_st().ccline.last_colors.cmdbuff = None;
    getln_st().ccline.last_colors.colors = Vec::new();

    let ret = if !getln_st().ccline.cmdbuff.is_null() {
        Some(crate::memory::cstr_to_cstring(getln_st().ccline.cmdbuff))
    } else {
        None
    };

    if ui_has(K_UICMDLINE) {
        ui_ext_cmdline_hide(s.gotesc);
    }
    if !g_st().cmd_silent {
        redraw_custom_title_later();
        status_redraw_all();
    }

    getln_st().cmdline_level -= 1;

    if did_save_ccline {
        restore_cmdline();
    } else {
        dealloc_cmdbuff();
    }

    s.prev_cmdbuff = None;
    ret
}

fn command_line_check(s: &mut CommandLineState) -> i32 {
    s.prev_cmdpos = getln_st().ccline.cmdpos;
    s.prev_cmdbuff = None;

    g_st().redir_off = true;
    g_st().quit_more = false;
    g_st().did_emsg = 0;

    if g_st().ex_normal_busy == 0 && stuff_empty() && g_st().typebuf.tb_len == 0 {
        s.some_key_typed = true;
    }

    may_trigger_safestate(s.xpc.xp_numfiles <= 0);

    if !getln_st().ccline.cmdbuff.is_null() {
        s.prev_cmdbuff = Some(crate::memory::cstr_to_cstring(getln_st().ccline.cmdbuff));
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
    g_st().no_mapping += 1;
    g_st().allow_keys += 1;
    s.c = plain_vgetc();
    g_st().no_mapping -= 1;
    g_st().allow_keys -= 1;

    if s.c != CTRL_N
        && s.c != CTRL_G
        && (s.c != b'e' as i32
            || (getln_st().ccline.cmdfirstc == b'=' as i32 && g_st().key_typed)
            || g_st().cmdline_star > 0)
    {
        vungetc(s.c);
        return PROCESS_NEXT_KEY;
    }

    if s.c == b'e' as i32 {
        if getln_st().ccline.cmdpos == getln_st().ccline.cmdlen {
            getln_st().new_cmdpos = 99999;
        } else {
            getln_st().new_cmdpos = getln_st().ccline.cmdpos;
        }

        s.c = get_expr_register();
        if s.c == b'=' as i32 {
            g_st().textlock += 1;
            let p = get_expr_line();
            g_st().textlock -= 1;

            if let Some(p) = p {
                let len = p.as_bytes().len() as i32;
                realloc_cmdbuff(len + 1);
                getln_st().ccline.cmdlen = len;
                crate::memory::cstr_cpy(getln_st().ccline.cmdbuff, p.as_ptr());

                getln_st().ccline.cmdpos = getln_st().ccline.cmdlen.min(getln_st().new_cmdpos);

                g_st().key_typed = false;
                redrawcmd();
                return CMDLINE_CHANGED;
            }
        }
        beep_flush();
        g_st().got_int = false;
        g_st().did_emsg = 0;
        g_st().emsg_on_display = false;
        redrawcmd();
        return CMDLINE_NOT_CHANGED;
    }

    s.gotesc = true;
    GOTO_NORMAL_MODE
}

/// Completion for `'wildchar'` or `'wildcharm'` key.
fn command_line_wildchar_complete(s: &mut CommandLineState) -> i32 {
    let res: i32;
    let mut options = WILD_NO_BEEP;
    let escape = s.firstc != b'@' as i32;
    let redraw_if_menu_empty = s.c == K_WILD;
    let wim_noselect =
        opt_vars().p_wmnu != 0 && (g_st().wim_flags[0] & K_OPT_WIM_FLAG_NOSELECT) != 0;
    let wim_noinsert =
        opt_vars().p_wmnu != 0 && (g_st().wim_flags[0] & K_OPT_WIM_FLAG_NOINSERT) != 0;

    if (g_st().wim_flags[s.wim_index as usize] & K_OPT_WIM_FLAG_LASTUSED) != 0 {
        options |= WILD_BUFLASTUSED;
    }
    if s.xpc.xp_numfiles > 0 {
        if s.xpc.xp_numfiles > 1
            && !s.did_wild_list
            && (g_st().wim_flags[s.wim_index as usize] & K_OPT_WIM_FLAG_LIST) != 0
        {
            showmatches(
                &mut s.xpc,
                false,
                true,
                if opt_vars().p_wmnu != 0 {
                    g_st().wim_flags[s.wim_index as usize] as i32
                } else {
                    0
                },
            );
            redrawcmd();
            s.did_wild_list = true;
        }
        if (g_st().wim_flags[s.wim_index as usize] & K_OPT_WIM_FLAG_LONGEST) != 0 {
            res = if nextwild(&mut s.xpc, WILD_LONGEST, options, escape) {
                OK
            } else {
                FAIL
            };
        } else if (g_st().wim_flags[s.wim_index as usize] & K_OPT_WIM_FLAG_FULL) != 0 {
            res = if nextwild(&mut s.xpc, WILD_NEXT, options, escape) {
                OK
            } else {
                FAIL
            };
        } else {
            res = OK;
        }
    } else {
        let wim_longest = (g_st().wim_flags[0] & K_OPT_WIM_FLAG_LONGEST) != 0;
        let wim_list = (g_st().wim_flags[0] & K_OPT_WIM_FLAG_LIST) != 0;
        let wim_full = (g_st().wim_flags[0] & K_OPT_WIM_FLAG_FULL) != 0;

        s.wim_index = 0;
        if s.c as OptInt == opt_vars().p_wc
            || s.c as OptInt == opt_vars().p_wcm
            || s.c == K_WILD
            || s.c == CTRL_Z_CONST
        {
            options |= WILD_MAY_EXPAND_PATTERN;
            if s.c == K_WILD {
                options |= WILD_FUNC_TRIGGER;
            }
            s.xpc.xp_pre_incsearch_pos = s.is_state.search_start;
        }
        let cmdpos_before = getln_st().ccline.cmdpos;

        if wim_longest {
            res = if nextwild(&mut s.xpc, WILD_LONGEST, options, escape) {
                OK
            } else {
                FAIL
            };
        } else {
            if wim_noselect || (wim_list && !wim_full) {
                options |= WILD_NOSELECT;
            }
            if wim_noinsert {
                options |= WILD_NOINSERT;
            }
            res = if nextwild(&mut s.xpc, WILD_EXPAND_KEEP, options, escape) {
                OK
            } else {
                FAIL
            };
        }

        if redraw_if_menu_empty && s.xpc.xp_numfiles <= 0 {
            pum_check_clear();
        }

        if g_st().got_int {
            vpeekc();
            g_st().got_int = false;
            expand_one(&mut s.xpc, ptr::null_mut(), None, 0, WILD_FREE);
            s.xpc.xp_context = EXPAND_NOTHING;
            return CMDLINE_CHANGED;
        }

        if res == OK && s.xpc.xp_numfiles > if wim_noselect || wim_noinsert { 0 } else { 1 } {
            if wim_longest {
                let found_longest_prefix = getln_st().ccline.cmdpos != cmdpos_before;
                let show_menu = opt_vars().p_wmnu != 0 && wim_full;
                if wim_list || show_menu {
                    showmatches(
                        &mut s.xpc,
                        show_menu,
                        wim_list,
                        K_OPT_WIM_FLAG_NOSELECT as i32,
                    );
                } else if !found_longest_prefix {
                    let wim_list_next = (g_st().wim_flags[1] & K_OPT_WIM_FLAG_LIST) != 0;
                    let wim_full_next = (g_st().wim_flags[1] & K_OPT_WIM_FLAG_FULL) != 0;
                    let wim_noselect_next = (g_st().wim_flags[1] & K_OPT_WIM_FLAG_NOSELECT) != 0;
                    let wim_noinsert_next = (g_st().wim_flags[1] & K_OPT_WIM_FLAG_NOINSERT) != 0;
                    if wim_list_next
                        || (opt_vars().p_wmnu != 0
                            && (wim_full_next || wim_noselect_next || wim_noinsert_next))
                    {
                        if wim_full_next && !wim_noselect_next && !wim_noinsert_next {
                            nextwild(&mut s.xpc, WILD_NEXT, options, escape);
                        } else {
                            showmatches(
                                &mut s.xpc,
                                opt_vars().p_wmnu != 0 && (wim_noselect_next || wim_noinsert_next),
                                wim_list_next,
                                if opt_vars().p_wmnu != 0 {
                                    g_st().wim_flags[1] as i32
                                } else {
                                    0
                                },
                            );
                        }
                        if wim_list_next {
                            s.did_wild_list = true;
                        }
                    }
                }
            } else {
                let show_menu =
                    opt_vars().p_wmnu != 0 && (wim_full || wim_noselect || wim_noinsert);
                if wim_list || show_menu {
                    showmatches(
                        &mut s.xpc,
                        show_menu,
                        wim_list,
                        if opt_vars().p_wmnu != 0 {
                            g_st().wim_flags[0] as i32
                        } else {
                            0
                        },
                    );
                } else {
                    vim_beep(K_OPT_BO_FLAG_WILDMODE);
                }
            }

            redrawcmd();
            if wim_list {
                s.did_wild_list = true;
            }
        } else if s.xpc.xp_numfiles == -1 {
            s.xpc.xp_context = EXPAND_NOTHING;
        }
    }

    if s.wim_index < 3 {
        s.wim_index += 1;
    }

    if s.c == ESC {
        s.gotesc = true;
    }

    if res == OK {
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
    if s.xpc.xp_numfiles != -1 {
        expand_one(&mut s.xpc, ptr::null_mut(), None, 0, WILD_FREE);
    }
    s.did_wild_list = false;
    if opt_vars().p_wmnu == 0 || (c != K_UP && c != K_DOWN) {
        s.xpc.xp_context = EXPAND_NOTHING;
    }
    s.wim_index = 0;
    wildmenu_cleanup(&mut getln_st().ccline);
}

fn command_line_execute(s: &mut CommandLineState, key: i32) -> i32 {
    if key == K_IGNORE || key == K_NOP {
        return -1; // get another key
    }

    let display_tick_saved: Disptick = curwin_mut().w_display_tick;
    s.c = key;

    if getln_st().ccline.cmdbuff_replaced && s.xpc.xp_numfiles > 0 {
        command_line_end_wildmenu(s, false, -1);
    }
    getln_st().ccline.cmdbuff_replaced = false;

    if s.c == K_WILD && s.did_hist_navigate {
        s.did_hist_navigate = false;
        return 1;
    }

    if s.c == K_EVENT || s.c == K_COMMAND || s.c == K_LUA {
        if s.c == K_EVENT {
            state_handle_k_event();
        } else if s.c == K_COMMAND {
            do_cmdline(
                ptr::null_mut(),
                crate::cmdexpand::LineGetter::CmdKeyCmd,
                DOCMD_NOWAIT,
            );
        } else {
            map_execute_lua(false, false);
        }
        if s.is_state.winid != curwin_mut().handle {
            init_incsearch_state(&mut s.is_state);
        }
        if curwin_mut().w_display_tick > display_tick_saved && s.is_state.did_incsearch {
            may_do_incsearch_highlighting(s.firstc, s.count, &mut s.is_state);
        }
        if getln_st().ccline.cmdbuff_replaced {
            command_line_changed(s);
        }

        if g_st().pum_want.active {
            if cmdline_pum_active() {
                nextwild(&mut s.xpc, WILD_PUM_WANT, 0, s.firstc != b'@' as i32);
                if g_st().pum_want.finish {
                    nextwild(
                        &mut s.xpc,
                        WILD_APPLY,
                        WILD_NO_BEEP,
                        s.firstc != b'@' as i32,
                    );
                    command_line_end_wildmenu(s, false, s.c);
                }
            }
            g_st().pum_want.active = false;
        }

        if !g_st().cmdline_was_last_drawn {
            redrawcmdline();
        }
        return 1;
    }

    if g_st().key_typed {
        s.some_key_typed = true;

        if g_st().cmdmsg_rl && g_st().key_stuffed == 0 {
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

    if s.c == CTRL_C_CONST && s.firstc != b'@' as i32 && !s.break_ctrl_c && g_st().global_busy == 0
    {
        g_st().got_int = false;
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
        && (s.xpc.xp_numfiles > 0 || (s.c != CTRL_P && s.c != CTRL_N))
    {
        s.lookfor = None;
        s.lookforlen = 0;
    }

    if s.c as OptInt != opt_vars().p_wc && s.c == K_S_TAB && s.xpc.xp_numfiles > 0 {
        s.c = CTRL_P;
    }

    if opt_vars().p_wmnu != 0 {
        s.c = wildmenu_translate_key(&mut getln_st().ccline, s.c, &mut s.xpc, s.did_wild_list);
    }

    let mut wild_type: i32 = 0;
    let key_is_wc =
        (s.c as OptInt == opt_vars().p_wc && g_st().key_typed) || s.c as OptInt == opt_vars().p_wcm;
    if (cmdline_pum_active() || g_st().wild_menu_showing != 0 || s.did_wild_list)
        && !key_is_wc
        && s.xpc.xp_numfiles > 0
        && (s.c == CTRL_E || s.c == CTRL_Y)
    {
        wild_type = if s.c == CTRL_E {
            WILD_CANCEL
        } else {
            WILD_APPLY
        };
        nextwild(&mut s.xpc, wild_type, WILD_NO_BEEP, s.firstc != b'@' as i32);
    }

    if (g_st().key_typed
        && (s.c == b'\n' as i32 || s.c == b'\r' as i32 || s.c == K_KENTER || s.c == ESC))
        || s.c == CTRL_C_CONST
    {
        set_vim_var_char(s.c);
        trigger_cmd_autocmd(s.cmdline_type, EVENT_CMDLINELEAVEPRE);
        s.event_cmdlineleavepre_triggered = true;
        if (s.c == ESC || s.c == CTRL_C_CONST) && (g_st().wim_flags[0] & K_OPT_WIM_FLAG_LIST) != 0 {
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

    if opt_vars().p_wmnu != 0 {
        s.c = wildmenu_process_key(&mut getln_st().ccline, s.c, &mut s.xpc);
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

    if s.c == getln_st().cedit_key {
        if g_st().ex_normal_busy == 0 && !g_st().got_int {
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
            && (!g_st().key_typed || crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_ESC)))
    {
        s.gotesc = false;
        if ccheck_abbr(s.c + ABBR_OFF) != 0 {
            return command_line_changed(s);
        }

        if !g_st().cmd_silent {
            if !ui_has(K_UICMDLINE) {
                msg_cursor_goto(g_st().msg_row, 0);
            }
            ui_flush();
        }
        return 0;
    }

    if (s.c as OptInt == opt_vars().p_wc && !s.gotesc && g_st().key_typed)
        || s.c as OptInt == opt_vars().p_wcm
        || s.c == K_WILD
        || s.c == CTRL_Z_CONST
    {
        if s.c == K_WILD {
            g_st().emsg_silent += 1;
        }
        let res = command_line_wildchar_complete(s);
        if s.c == K_WILD {
            g_st().emsg_silent -= 1;
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
        && g_st().key_typed
        && nextwild(&mut s.xpc, WILD_EXPAND_KEEP, 0, s.firstc != b'@' as i32)
    {
        if s.xpc.xp_numfiles > 1
            && ((!s.did_wild_list
                && (g_st().wim_flags[s.wim_index as usize] & K_OPT_WIM_FLAG_LIST) != 0)
                || opt_vars().p_wmnu != 0)
        {
            let list = (g_st().wim_flags[s.wim_index as usize] & K_OPT_WIM_FLAG_LIST) != 0;
            showmatches(
                &mut s.xpc,
                opt_vars().p_wmnu != 0 && !list,
                list,
                if opt_vars().p_wmnu != 0 {
                    g_st().wim_flags[0] as i32
                } else {
                    0
                },
            );
        }
        nextwild(&mut s.xpc, WILD_PREV, 0, s.firstc != b'@' as i32);
        nextwild(&mut s.xpc, WILD_PREV, 0, s.firstc != b'@' as i32);
        return command_line_changed(s);
    }

    if s.c == NUL as i32 || s.c == K_ZERO {
        s.c = NL;
    }

    s.do_abbr = true;

    if wild_type == WILD_CANCEL || wild_type == WILD_APPLY {
        if s.is_state.winid != curwin_mut().handle {
            init_incsearch_state(&mut s.is_state);
        }
        if g_st().key_typed || vpeekc() == NUL as i32 {
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
    let mut magic: bool = opt_vars().p_magic != 0;

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
    if patlen == 0 && crate::memory::cstr_byte_at(getln_st().ccline.cmdbuff, skiplen as usize) == 0
    {
        restore_last_search_pattern();
        return false;
    }

    ui_busy_start();
    ui_flush();

    let mut t: CursorPos;
    let mut pat = getln_st().ccline.cmdbuff.wrapping_add(skiplen as usize);
    let mut dircp: *mut c_char = ptr::null_mut();
    let mut searchstr = pat;
    let mut strcopy: Vec<c_char> = Vec::new();
    let mut searchstrlen = patlen as usize;
    let mut offset = SearchOffset::new();
    let mut search_flags = SEARCH_NOOF;
    let mut patlen_s = (getln_st().ccline.cmdlen - skiplen) as usize;

    parse_search_pattern_offset(
        &raw mut pat,
        &mut patlen_s,
        search_delim,
        SEARCH_OPT,
        &mut strcopy,
        &raw mut searchstr,
        &mut searchstrlen,
        &raw mut dircp,
        &mut offset,
    );

    if next_match {
        t = s.match_end;
        if lt(s.match_start, s.match_end) {
            decl(&mut t);
        }
        search_flags += SEARCH_COL;
    } else {
        t = s.match_start;
    }
    if opt_vars().p_hls == 0 {
        search_flags += SEARCH_KEEP;
    }
    g_st().emsg_off += 1;
    let found = searchit(
        curwin_ptr(),
        curbuf_mut(),
        &mut t,
        None,
        if next_match { FORWARD } else { BACKWARD },
        searchstr,
        searchstrlen,
        count,
        search_flags,
        RE_SEARCH,
        magic,
        None,
    );
    g_st().emsg_off -= 1;
    if !dircp.is_null() {
        unsafe {
            *dircp = search_delim as c_char;
        }
    }
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
        curwin_mut().w_cursor = s.match_start;
        changed_cline_bef_curs(curwin_mut());
        update_topline(curwin_mut());
        validate_cursor(curwin_mut());
        g_st().search_global.hl_match = true;
        save_viewstate(curwin_mut(), &mut s.old_viewstate);
        redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
        update_screen();
        g_st().search_global.hl_match = false;
        redrawcmdline();
        curwin_mut().w_cursor = s.match_end;
    } else {
        vim_beep(K_OPT_BO_FLAG_ERROR);
    }
    restore_last_search_pattern();
    false
}

/// Handle backspace, delete and CTRL-W keys in the command-line mode.
fn command_line_erase_chars(s: &mut CommandLineState) -> i32 {
    unsafe {
        if s.c == K_KDEL {
            s.c = K_DEL;
        }

        if s.c == K_DEL && getln_st().ccline.cmdpos != getln_st().ccline.cmdlen {
            getln_st().ccline.cmdpos += 1;
        }
        if s.c == K_DEL {
            getln_st().ccline.cmdpos += mb_off_next(
                getln_st().ccline.cmdbuff,
                getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize),
            );
        }

        if getln_st().ccline.cmdpos > 0 {
            let mut j = getln_st().ccline.cmdpos;
            let mut p = mb_prevptr(
                getln_st().ccline.cmdbuff,
                getln_st().ccline.cmdbuff.wrapping_add(j as usize),
            );

            if s.c == CTRL_W_CONST {
                while p > getln_st().ccline.cmdbuff && ascii_isspace(*p as i32) {
                    p = mb_prevptr(getln_st().ccline.cmdbuff, p);
                }

                let cls = mb_get_class(p);
                while p > getln_st().ccline.cmdbuff && mb_get_class(p) == cls {
                    p = mb_prevptr(getln_st().ccline.cmdbuff, p);
                }

                if mb_get_class(p) != cls {
                    p = p.wrapping_add(utfc_ptr2len(p) as usize);
                }
            }

            getln_st().ccline.cmdpos =
                crate::memory::ptr_offset_from(p, getln_st().ccline.cmdbuff) as i32;
            getln_st().ccline.cmdlen -= j - getln_st().ccline.cmdpos;
            let mut i = getln_st().ccline.cmdpos;

            while i < getln_st().ccline.cmdlen {
                *getln_st().ccline.cmdbuff.wrapping_add(i as usize) =
                    *getln_st().ccline.cmdbuff.wrapping_add(j as usize);
                i += 1;
                j += 1;
            }

            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add(getln_st().ccline.cmdlen as usize) = NUL;
            if getln_st().ccline.cmdlen == 0 {
                s.is_state.search_start = s.is_state.save_cursor;
                s.is_state.old_viewstate = s.is_state.init_viewstate;
            }
            redrawcmd();
        } else if getln_st().ccline.cmdlen == 0
            && s.c != CTRL_W_CONST
            && getln_st().ccline.cmdprompt.is_none()
            && s.indent == 0
        {
            if getln_st().ccline.cmdfirstc == b'>' as i32 {
                return CMDLINE_NOT_CHANGED;
            }

            dealloc_cmdbuff();

            if !g_st().cmd_silent && !ui_has(K_UICMDLINE) {
                g_st().msg_col = 0;
                msg_putchar(b' ' as i32);
            }
            s.is_state.search_start = s.is_state.save_cursor;
            g_st().redraw_cmdline = true;
            return GOTO_NORMAL_MODE;
        }
        CMDLINE_CHANGED
    }
}

/// Handle the CTRL-^ key in the command-line mode and toggle the use of the
/// language `:lmap` mappings and/or Input Method.
fn command_line_toggle_langmap(s: &mut CommandLineState) {
    let use_insert_opt = if buf_valid(s.b_im_ptr_buf) {
        s.b_im_use_insert
    } else {
        None
    };
    if map_to_exists_mode(c"", MODE_LANGMAP, false) {
        g_st().state_global ^= MODE_LANGMAP;
        if let Some(use_insert) = use_insert_opt {
            let val = if (g_st().state_global & MODE_LANGMAP) != 0 {
                B_IMODE_LMAP
            } else {
                B_IMODE_NONE
            };
            let buf = s.b_im_ptr_buf;
            if use_insert {
                unsafe {
                    (*buf).b_p_iminsert = val;
                }
            } else {
                unsafe {
                    (*buf).b_p_imsearch = val;
                }
            }
        }
    }

    if let Some(use_insert) = use_insert_opt {
        if use_insert && ptr::eq(s.b_im_ptr_buf, g_st().curbuf) {
            set_iminsert_global(curbuf_mut());
        } else {
            set_imsearch_global(curbuf_mut());
        }
    }
    ui_cursor_shape();
    status_redraw_curbuf();
}

/// Handle the CTRL-R key in the command-line mode and insert the contents of a
/// numbered or named register.
fn command_line_insert_reg(s: &mut CommandLineState) -> i32 {
    let save_new_cmdpos = getln_st().new_cmdpos;

    putcmdline(b'"', true);
    g_st().no_mapping += 1;
    g_st().allow_keys += 1;
    s.c = plain_vgetc();
    let mut i = s.c;
    if i == CTRL_O {
        i = CTRL_R_CONST;
    }

    if i == CTRL_R_CONST {
        s.c = plain_vgetc();
    }
    g_st().no_mapping -= 1;
    g_st().allow_keys -= 1;
    getln_st().new_cmdpos = -1;
    if s.c == b'=' as i32 {
        if getln_st().ccline.cmdfirstc == b'=' as i32 || g_st().cmdline_star > 0 {
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
        g_st().key_typed = false;
        if getln_st().new_cmdpos >= 0 {
            getln_st().ccline.cmdpos = getln_st().ccline.cmdlen.min(getln_st().new_cmdpos);
        }
    }
    getln_st().new_cmdpos = save_new_cmdpos;

    getln_st().ccline.special_char = 0;
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

    getln_st().ccline.cmdspos = cmd_startcol();
    getln_st().ccline.cmdpos = 0;
    while getln_st().ccline.cmdpos < getln_st().ccline.cmdlen {
        let cells = cmdline_charsize(getln_st().ccline.cmdpos);
        if g_st().mouse_row
            <= g_st().cmdline_row + getln_st().ccline.cmdspos / g_st().columns_global
            && g_st().mouse_col < getln_st().ccline.cmdspos % g_st().columns_global + cells
        {
            break;
        }

        correct_screencol(
            getln_st().ccline.cmdpos,
            cells,
            &mut getln_st().ccline.cmdspos,
        );
        getln_st().ccline.cmdpos += utfc_ptr2len(
            getln_st()
                .ccline
                .cmdbuff
                .wrapping_add(getln_st().ccline.cmdpos as usize),
        ) - 1;
        getln_st().ccline.cmdspos += cells;
        getln_st().ccline.cmdpos += 1;
    }
}

fn command_line_next_histidx(s: &mut CommandLineState, next_match: bool) {
    let hisidx = get_hisidx(s.histype);
    let hist = get_histentry(s.histype);
    loop {
        if !next_match {
            if s.hiscnt == get_hislen() {
                s.hiscnt = hisidx;
            } else if s.hiscnt == 0 && hisidx != get_hislen() - 1 {
                s.hiscnt = get_hislen() - 1;
            } else if s.hiscnt != hisidx + 1 {
                s.hiscnt -= 1;
            } else {
                s.hiscnt = s.save_hiscnt;
                break;
            }
        } else {
            if s.hiscnt == hisidx {
                s.hiscnt = get_hislen();
                break;
            }

            if s.hiscnt == get_hislen() {
                break;
            }

            if s.hiscnt == get_hislen() - 1 {
                s.hiscnt = 0;
            } else {
                s.hiscnt += 1;
            }
        }

        if s.hiscnt < 0 || hist[s.hiscnt as usize].hisstr.is_none() {
            s.hiscnt = s.save_hiscnt;
            break;
        }

        let lookfor_ptr = s.lookfor.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr());
        if (s.c != K_UP && s.c != K_DOWN)
            || s.hiscnt == s.save_hiscnt
            || (crate::memory::cstr_prefix_bytes(
                hist[s.hiscnt as usize].hisstr_ptr(),
                s.lookforlen as usize,
            ) == crate::memory::cstr_prefix_bytes(lookfor_ptr, s.lookforlen as usize))
        {
            break;
        }
    }
}

/// Handle the Up, Down, Page Up, Page down, CTRL-N and CTRL-P key in the
/// command-line mode.
fn command_line_browse_history(s: &mut CommandLineState) -> i32 {
    unsafe {
        if s.histype == HIST_INVALID || get_hislen() == 0 || s.firstc == NUL as i32 {
            return CMDLINE_NOT_CHANGED;
        }

        s.save_hiscnt = s.hiscnt;

        if s.lookfor.is_none() {
            let prefix_len = getln_st().ccline.cmdpos as usize;
            let bytes = crate::memory::cstr_slice(getln_st().ccline.cmdbuff, prefix_len);
            s.lookfor = Some(std::ffi::CString::new(bytes).unwrap_or_default());
            s.lookforlen = getln_st().ccline.cmdpos;
        }

        let next_match = s.c == K_DOWN
            || s.c == K_S_DOWN
            || s.c == CTRL_N
            || s.c == K_PAGEDOWN
            || s.c == K_KPAGEDOWN;
        command_line_next_histidx(s, next_match);

        if s.hiscnt != s.save_hiscnt {
            let p: *mut c_char;
            let plen: i32;
            let old_firstc: i32;
            let mut sep: c_char = 0;

            dealloc_cmdbuff();

            let lookfor_ptr = s
                .lookfor
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            s.xpc.xp_context = EXPAND_NOTHING;
            if s.hiscnt == get_hislen() {
                p = lookfor_ptr;
                plen = s.lookforlen;
            } else {
                let entry = &get_histentry(s.histype)[s.hiscnt as usize];
                p = entry.hisstr_ptr();
                plen = entry.hisstrlen() as i32;
                sep = entry.sep as c_char;
            }

            if s.histype == HIST_SEARCH && p != lookfor_ptr && {
                old_firstc = (sep as u8) as i32;
                old_firstc != s.firstc
            } {
                let mut len: i32 = 0;
                for i in 0..=1 {
                    len = 0;
                    let mut j: usize = 0;
                    while crate::memory::cstr_byte_at(p, j) != 0 {
                        if crate::memory::cstr_char_at(p, j) == old_firstc
                            && (j == 0 || crate::memory::cstr_byte_at(p, j - 1) != b'\\')
                        {
                            if i > 0 {
                                *getln_st().ccline.cmdbuff.wrapping_add(len as usize) =
                                    s.firstc as c_char;
                            }
                        } else {
                            if crate::memory::cstr_char_at(p, j) == s.firstc
                                && (j == 0 || crate::memory::cstr_byte_at(p, j - 1) != b'\\')
                            {
                                if i > 0 {
                                    *getln_st().ccline.cmdbuff.wrapping_add(len as usize) =
                                        b'\\' as c_char;
                                }
                                len += 1;
                            }

                            if i > 0 {
                                *getln_st().ccline.cmdbuff.wrapping_add(len as usize) =
                                    *p.wrapping_add(j);
                            }
                        }
                        len += 1;
                        j += 1;
                    }

                    if i == 0 {
                        alloc_cmdbuff(len);
                    }
                }
                *getln_st().ccline.cmdbuff.wrapping_add(len as usize) = NUL;
                getln_st().ccline.cmdlen = len;
                getln_st().ccline.cmdpos = len;
            } else {
                alloc_cmdbuff(plen);
                crate::memory::cstr_cpy(getln_st().ccline.cmdbuff, p);
                getln_st().ccline.cmdlen = plen;
                getln_st().ccline.cmdpos = plen;
            }

            redrawcmd();
            return CMDLINE_CHANGED;
        }
        beep_flush();
        CMDLINE_NOT_CHANGED
    }
}

fn command_line_handle_key(s: &mut CommandLineState) -> i32 {
    unsafe {
        if !(getln_st().ccline.one_key && s.c != ESC && s.c != CTRL_C_CONST) {
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
                        getln_st().ccline.overstrike = if getln_st().ccline.overstrike != 0 {
                            0
                        } else {
                            1
                        };
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
                        let mut j = getln_st().ccline.cmdpos;
                        getln_st().ccline.cmdlen -= j;
                        getln_st().ccline.cmdpos = 0;
                        let mut i = 0;
                        while i < getln_st().ccline.cmdlen {
                            *getln_st().ccline.cmdbuff.wrapping_add(i as usize) =
                                *getln_st().ccline.cmdbuff.wrapping_add(j as usize);
                            i += 1;
                            j += 1;
                        }

                        *getln_st()
                            .ccline
                            .cmdbuff
                            .wrapping_add(getln_st().ccline.cmdlen as usize) = NUL;
                        if getln_st().ccline.cmdlen == 0 {
                            s.is_state.search_start = s.is_state.save_cursor;
                        }
                        redrawcmd();
                        return command_line_changed(s);
                    }

                    ESC | CTRL_C_CONST => {
                        if getln_st().getln_interrupted_highlight && s.c == CTRL_C_CONST {
                            getln_st().getln_interrupted_highlight = false;
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
                        if showmatches(
                            &mut s.xpc,
                            false,
                            true,
                            if opt_vars().p_wmnu != 0 {
                                g_st().wim_flags[0] as i32
                            } else {
                                0
                            },
                        ) == EXPAND_NOTHING
                        {
                            break 'key_switch;
                        }

                        redrawcmd();
                        return 1;
                    }

                    K_RIGHT | K_S_RIGHT | K_C_RIGHT => {
                        loop {
                            if getln_st().ccline.cmdpos >= getln_st().ccline.cmdlen {
                                break;
                            }

                            let cells = cmdline_charsize(getln_st().ccline.cmdpos);
                            if g_st().key_typed
                                && getln_st().ccline.cmdspos + cells
                                    >= g_st().columns_global * g_st().rows_global
                            {
                                break;
                            }

                            getln_st().ccline.cmdspos += cells;
                            getln_st().ccline.cmdpos += utfc_ptr2len(
                                getln_st()
                                    .ccline
                                    .cmdbuff
                                    .wrapping_add(getln_st().ccline.cmdpos as usize),
                            );

                            if !((s.c == K_S_RIGHT
                                || s.c == K_C_RIGHT
                                || (g_st().mod_mask & ((MOD_MASK_SHIFT | MOD_MASK_CTRL) as i32))
                                    != 0)
                                && *getln_st()
                                    .ccline
                                    .cmdbuff
                                    .wrapping_add(getln_st().ccline.cmdpos as usize)
                                    != b' ' as c_char)
                            {
                                break;
                            }
                        }
                        getln_st().ccline.cmdspos = cmd_screencol(getln_st().ccline.cmdpos);
                        return command_line_not_changed(s);
                    }

                    K_LEFT | K_S_LEFT | K_C_LEFT => {
                        if getln_st().ccline.cmdpos == 0 {
                            return command_line_not_changed(s);
                        }
                        loop {
                            getln_st().ccline.cmdpos -= 1;
                            getln_st().ccline.cmdpos -= utf_head_off(
                                getln_st().ccline.cmdbuff,
                                getln_st()
                                    .ccline
                                    .cmdbuff
                                    .wrapping_add(getln_st().ccline.cmdpos as usize),
                            );
                            getln_st().ccline.cmdspos -= cmdline_charsize(getln_st().ccline.cmdpos);

                            if !(getln_st().ccline.cmdpos > 0
                                && (s.c == K_S_LEFT
                                    || s.c == K_C_LEFT
                                    || (g_st().mod_mask
                                        & ((MOD_MASK_SHIFT | MOD_MASK_CTRL) as i32))
                                        != 0)
                                && *getln_st()
                                    .ccline
                                    .cmdbuff
                                    .wrapping_add((getln_st().ccline.cmdpos - 1) as usize)
                                    != b' ' as c_char)
                            {
                                break;
                            }
                        }

                        getln_st().ccline.cmdspos = cmd_screencol(getln_st().ccline.cmdpos);
                        if getln_st().ccline.special_char != 0 {
                            putcmdline(
                                getln_st().ccline.special_char,
                                getln_st().ccline.special_shift,
                            );
                        }

                        return command_line_not_changed(s);
                    }

                    K_IGNORE | K_MIDDLEDRAG | K_MIDDLERELEASE => {
                        return command_line_not_changed(s);
                    }

                    K_MIDDLEMOUSE => {
                        cmdline_paste(
                            if eval_has_provider(c"clipboard", false) {
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
                        if { getln_st().ccline.mouse_used }.is_some()
                            && g_st().mouse_row < g_st().cmdline_row
                        {
                            getln_st().ccline.mouse_used = Some(true);
                            return 0;
                        }
                        command_line_left_right_mouse(s);
                        return command_line_not_changed(s);
                    }

                    K_LEFTMOUSE => {
                        if { getln_st().ccline.mouse_used }.is_some()
                            && g_st().mouse_row < g_st().cmdline_row
                        {
                            getln_st().ccline.mouse_used = Some(true);
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
                        getln_st().ccline.cmdpos = 0;
                        getln_st().ccline.cmdspos = cmd_startcol();
                        return command_line_not_changed(s);
                    }

                    CTRL_E | K_END | K_KEND | K_S_END | K_C_END => {
                        getln_st().ccline.cmdpos = getln_st().ccline.cmdlen;
                        getln_st().ccline.cmdspos = cmd_screencol(getln_st().ccline.cmdpos);
                        return command_line_not_changed(s);
                    }

                    CTRL_A => {
                        if cmdline_pum_active() {
                            cmdline_pum_cleanup(&mut getln_st().ccline);
                        }

                        if !nextwild(&mut s.xpc, WILD_ALL, 0, s.firstc != b'@' as i32) {
                            break 'key_switch;
                        }
                        s.xpc.xp_context = EXPAND_NOTHING;
                        s.did_wild_list = false;
                        return command_line_changed(s);
                    }

                    CTRL_L_CONST => {
                        if may_add_char_to_search(s.firstc, &mut s.c, &mut s.is_state) {
                            return command_line_not_changed(s);
                        }

                        if !nextwild(&mut s.xpc, WILD_LONGEST, 0, s.firstc != b'@' as i32) {
                            break 'key_switch;
                        }
                        return command_line_changed(s);
                    }

                    CTRL_N | CTRL_P => {
                        if s.xpc.xp_numfiles > 0 {
                            let wild_type = if s.c == CTRL_P { WILD_PREV } else { WILD_NEXT };
                            if !nextwild(&mut s.xpc, wild_type, 0, s.firstc != b'@' as i32) {
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
                            if !nextwild(&mut s.xpc, wild_type, 0, s.firstc != b'@' as i32) {
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

                        s.c = get_literal((g_st().mod_mask & (MOD_MASK_SHIFT as i32)) != 0);

                        s.do_abbr = false;
                        getln_st().ccline.special_char = 0;
                        if utf_iscomposing_first(s.c) && !g_st().cmd_silent {
                            if ui_has(K_UICMDLINE) {
                                unputcmdline();
                            } else {
                                draw_cmdline(
                                    getln_st().ccline.cmdpos,
                                    getln_st().ccline.cmdlen - getln_st().ccline.cmdpos,
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
                        getln_st().ccline.special_char = 0;

                        if s.c != NUL as i32 {
                            break 'key_switch;
                        }

                        redrawcmd();
                        return command_line_not_changed(s);
                    }

                    CTRL_CONST_CONST => {
                        if opt_vars().p_ari == 0 {
                            break 'key_switch;
                        }
                        return command_line_not_changed(s);
                    }

                    c if c == b'q' as i32 => {
                        if { getln_st().ccline.mouse_used }.is_some() {
                            *getln_st().ccline.cmdbuff = NUL;
                            return 0;
                        }
                        if !is_special(s.c) {
                            g_st().mod_mask = 0x0;
                        }
                    }

                    _ => {
                        if !is_special(s.c) {
                            g_st().mod_mask = 0x0;
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
        if is_special(s.c) || g_st().mod_mask != 0 {
            put_on_cmdline(get_special_key_name(s.c, g_st().mod_mask), -1, true);
        } else {
            let iobuff_ptr = g_st().iobuff.as_mut_ptr();
            let j = utf_char2bytes(s.c, iobuff_ptr);
            *iobuff_ptr.wrapping_add(j as usize) = NUL;
            put_on_cmdline(iobuff_ptr, j, true);
        }
        if getln_st().ccline.one_key {
            0
        } else {
            command_line_changed(s)
        }
    }
}

/// Trigger `CursorMovedC` autocommands.
fn may_trigger_cursormovedc(s: &mut CommandLineState) {
    if getln_st().ccline.cmdpos != s.prev_cmdpos {
        trigger_cmd_autocmd(s.cmdline_type, EVENT_CURSORMOVEDC);
        getln_st().ccline.redraw_state = getln_st().ccline.redraw_state.max(K_CMD_REDRAW_POS);
    }
}

fn command_line_not_changed(s: &mut CommandLineState) -> i32 {
    may_trigger_cursormovedc(s);
    s.prev_cmdpos = getln_st().ccline.cmdpos;
    if !s.is_state.incsearch_postponed {
        return 1;
    }
    command_line_changed(s)
}

/// Guess that the pattern matches everything.
fn empty_pattern(p: impl crate::strings::IntoCCharPtr, len: usize, delim: i32) -> bool {
    let p = p.into_c_char_ptr().cast_mut();
    let mut magic_val: RegMagic = MAGIC_ON;

    if len > 0 {
        skip_regexp_ex(
            p,
            delim,
            opt_vars().p_magic,
            false,
            None,
            None,
            Some(&mut magic_val),
        );
    } else {
        return true;
    }

    empty_pattern_magic(p, len, magic_val)
}

fn empty_pattern_magic(
    p: impl crate::strings::IntoCCharPtr,
    mut len: usize,
    magic_val: RegMagic,
) -> bool {
    let p = p.into_c_char_ptr();
    {
        while len >= 2
            && crate::memory::cstr_byte_at(p, len - 2) == b'\\'
            && crate::strings::vim_haschr(
                c"mMvVcCZ",
                (crate::memory::cstr_byte_at(p, len - 1)) as i32,
            )
        {
            len -= 2;
        }

        len == 0
            || (len > 1
                && crate::memory::cstr_byte_at(p, len - 1) == b'|'
                && ((crate::memory::cstr_byte_at(p, len - 2) == b'\\' && magic_val == MAGIC_ON)
                    || (crate::memory::cstr_byte_at(p, len - 2) != b'\\'
                        && magic_val == MAGIC_ALL)))
    }
}

pub fn cmdpreview_get_bufnr() -> ObjHandle {
    getln_st().cmdpreview_bufnr
}

pub fn cmdpreview_get_ns() -> i32 {
    getln_st().cmdpreview_ns
}

/// Sets up command preview buffer.
fn cmdpreview_open_buf() -> *mut FileBuf {
    let mut cmdpreview_buf = if getln_st().cmdpreview_bufnr != 0 {
        buflist_findnr(getln_st().cmdpreview_bufnr)
    } else {
        ptr::null_mut()
    };

    if cmdpreview_buf.is_null() {
        let mut err = error_init();
        let bufnr = nvim_create_buf(false, true, &mut err);

        if error_set(&err) {
            return ptr::null_mut();
        }

        cmdpreview_buf = buflist_findnr(bufnr);
    }

    if cmdpreview_buf == g_st().curbuf {
        return ptr::null_mut();
    }

    let mut aco: CtxSwitch = CtxSwitch::new();
    ctx_switch(
        &mut aco,
        ptr::null_mut(),
        ptr::null_mut(),
        cmdpreview_buf,
        K_CTX_KEEP_DIRS,
    );
    let retv = rename_buffer(c"[Preview]");
    ctx_restore(&mut aco);

    if !retv {
        return ptr::null_mut();
    }

    ctx_switch(
        &mut aco,
        ptr::null_mut(),
        ptr::null_mut(),
        cmdpreview_buf,
        K_CTX_KEEP_DIRS,
    );
    buf_clear();
    curbuf_mut().b_p_ma = 1;
    curbuf_mut().b_p_ul = -1;
    curbuf_mut().b_p_tw = 0;
    ctx_restore(&mut aco);
    getln_st().cmdpreview_bufnr = unsafe { (*cmdpreview_buf).b_fnum };

    cmdpreview_buf
}

/// Open command preview window if it's not already open.
fn cmdpreview_open_win(cmdpreview_buf: &mut FileBuf) -> *mut WinInfoInfo {
    let save_curwin = curwin_ptr();

    if !win_split(opt_vars().p_cwh as i32, WSP_BOT) {
        return ptr::null_mut();
    }

    let preview_win = curwin_ptr();
    let mut err = error_init();

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let result = do_buffer(DOBUF_GOTO, DOBUF_FIRST, FORWARD, cmdpreview_buf.b_fnum, 0);
    try_leave(&tstate, &mut err);
    if error_set(&err) || !result {
        api_clear_error(&mut err);
        return ptr::null_mut();
    }

    curwin_mut().w_onebuf_opt.wo_cul = 0;
    curwin_mut().w_onebuf_opt.wo_cuc = 0;
    curwin_mut().w_onebuf_opt.wo_spell = 0;
    curwin_mut().w_onebuf_opt.wo_fen = 0;

    win_enter(save_curwin, false);
    preview_win
}

/// Closes any open command preview windows.
fn cmdpreview_close_win() {
    let buf = if getln_st().cmdpreview_bufnr != 0 {
        buflist_findnr(getln_st().cmdpreview_bufnr)
    } else {
        ptr::null_mut()
    };
    if !buf.is_null() {
        close_windows(buf, false);
    }
}

/// Save current state and prepare windows and buffers for command preview.
fn cmdpreview_prepare() -> CpInfo {
    let mut saved_bufs: std::collections::HashSet<*mut FileBuf> = std::collections::HashSet::new();
    let mut cpinfo = CpInfo {
        win_info: Vec::new(),
        buf_info: Vec::new(),
        save_hls: false,
        save_cmdmod: Cmdmod::new(),
        save_view: Vec::new(),
    };

    for win in for_all_windows_ptr() {
        let buf = win.w_buffer;
        let buf = unsafe { &mut *buf };

        if buf.b_fnum != getln_st().cmdpreview_bufnr {
            if saved_bufs.insert(buf) {
                let mut cp_bufinfo = CpBufInfo::new();
                cp_bufinfo.buf = buf;
                cp_bufinfo.save_b_p_ma = buf.b_p_ma;
                cp_bufinfo.save_b_changed = buf.b_changed;
                cp_bufinfo.save_b_op_start = buf.b_op_start;
                cp_bufinfo.save_b_op_end = buf.b_op_end;
                u_checkpoint(&mut cp_bufinfo.undo_checkpoint, &mut *buf);
                cpinfo.buf_info.push(cp_bufinfo);
            }

            let mut cp_wininfo = CpWinInfo::new();
            cp_wininfo.win = win;
            cp_wininfo.save_w_cursor = win.w_cursor;
            save_viewstate(&mut *win, &mut cp_wininfo.save_viewstate);
            cp_wininfo.save_w_p_cul = win.w_onebuf_opt.wo_cul;
            cp_wininfo.save_w_p_cuc = win.w_onebuf_opt.wo_cuc;

            cpinfo.win_info.push(cp_wininfo);

            win.w_onebuf_opt.wo_cul = 0;
            win.w_onebuf_opt.wo_cuc = 0;
        }
    }

    cpinfo.save_hls = opt_vars().p_hls != 0;
    cpinfo.save_cmdmod = unsafe { ptr::replace(&raw mut g_st().cmdmod, Cmdmod::new()) };
    win_size_save(&mut cpinfo.save_view);
    save_search_patterns();

    opt_vars().p_hls = 0;
    g_st().cmdmod.cmod_split = 0;
    g_st().cmdmod.cmod_tab = 0;
    g_st().cmdmod.cmod_flags |= CMOD_NOSWAPFILE;

    u_sync(true);
    cpinfo
}

/// Restore the state of buffers and windows for command preview.
fn cmdpreview_restore_state(cpinfo: &mut CpInfo) {
    for mut cp_bufinfo in cpinfo.buf_info.drain(..) {
        let buf = cp_bufinfo.buf;
        let buf = unsafe { &mut *buf };

        buf.b_changed = cp_bufinfo.save_b_changed;

        extmark_clear(
            &mut *buf,
            getln_st().cmdpreview_ns as u32,
            0,
            0,
            MAXLNUM,
            MAXCOL,
        );

        u_rollback(&mut cp_bufinfo.undo_checkpoint, &mut *buf);

        buf.b_op_start = cp_bufinfo.save_b_op_start;
        buf.b_op_end = cp_bufinfo.save_b_op_end;
        buf.b_p_ma = cp_bufinfo.save_b_p_ma;
    }

    for cp_wininfo in cpinfo.win_info.drain(..) {
        let win = cp_wininfo.win;
        let win = unsafe { &mut *win };

        win.w_cursor = cp_wininfo.save_w_cursor;
        restore_viewstate(&mut *win, &cp_wininfo.save_viewstate);

        win.w_onebuf_opt.wo_cul = cp_wininfo.save_w_p_cul;
        win.w_onebuf_opt.wo_cuc = cp_wininfo.save_w_p_cuc;

        update_topline(&mut *win);
    }

    g_st().cmdmod = std::mem::take(&mut cpinfo.save_cmdmod);
    opt_vars().p_hls = i32::from(cpinfo.save_hls);
    restore_search_patterns();
    win_size_restore(&cpinfo.save_view);
}

/// Show `'inccommand'` preview if command is previewable.
fn cmdpreview_may_show(_s: &mut CommandLineState) -> bool {
    let mut ea: Exarg = Exarg::new();
    let mut cmod: Cmdmod = Cmdmod::new();
    let mut cmdpreview_type: i32;
    let mut cmdline_vec: Vec<u8> =
        crate::memory::cstr_as_bytes_with_nul(getln_st().ccline.cmdbuff).to_vec();
    let mut cmdline = cmdline_vec.as_mut_ptr().cast::<c_char>();
    let mut errormsg: *const c_char = ptr::null();
    g_st().emsg_off += 1;
    let parsed = parse_cmdline(
        &raw mut cmdline,
        &raw mut cmdline_vec,
        &mut ea,
        &mut cmod,
        &mut errormsg,
        false,
    );
    g_st().emsg_off -= 1;
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

    let mut icm_split = crate::memory::cstr_first_byte(opt_vars().p_icm) == b's';
    let mut cmdpreview_buf: *mut FileBuf = ptr::null_mut();
    let mut cmdpreview_win: *mut WinInfoInfo = ptr::null_mut();

    g_st().emsg_silent += 1;
    g_st().msg_silent += 1;
    block_autocmds();

    let mut cpinfo = cmdpreview_prepare();

    if icm_split {
        cmdpreview_buf = cmdpreview_open_buf();
        if cmdpreview_buf.is_null() {
            set_option_direct(K_OPT_INCCOMMAND, cstr_as_obj(c"nosplit"), 0, SID_NONE);
            icm_split = false;
        }
    }
    if getln_st().cmdpreview_ns == 0 {
        getln_st().cmdpreview_ns = nvim_create_namespace(NvimString {
            data: ptr::null_mut(),
            size: 0,
        }) as i32;
    }

    g_st().cmdpreview = true;

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
        cmdpreview_win = cmdpreview_open_win(unsafe { &mut *cmdpreview_buf });
        if cmdpreview_win.is_null() {
            cmdpreview_type = 1;
        }
    }

    if cmdpreview_type != 0 {
        let save_rd = g_st().redrawing_disabled;
        g_st().redrawing_disabled = 0;
        update_screen();
        g_st().redrawing_disabled = save_rd;
    }

    if icm_split && cmdpreview_type == 2 && !cmdpreview_win.is_null() {
        cmdpreview_close_win();
    }

    cmdpreview_restore_state(&mut cpinfo);

    unblock_autocmds();
    g_st().msg_silent -= 1;
    g_st().emsg_silent -= 1;
    redrawcmdline();

    drop(cmdline_vec);
    cmdpreview_type != 0
}

/// Trigger `CmdlineChanged` autocommands.
fn do_autocmd_cmdlinechanged(firstc: i32) {
    if has_event(EVENT_CMDLINECHANGED) {
        let mut err = error_init();
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);

        let mut firstcbuf: [c_char; 2] = [firstc as c_char, 0];

        tv_dict_add_str(&mut *dict, c"cmdtype", firstcbuf.as_ptr());
        tv_dict_add_nr(
            &mut *dict,
            c"cmdlevel",
            getln_st().ccline.level as Varnumber,
        );
        tv_dict_set_keys_readonly(&mut *dict);
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        apply_autocmds(
            EVENT_CMDLINECHANGED,
            firstcbuf.as_mut_ptr(),
            firstcbuf.as_mut_ptr(),
            false,
            g_st().curbuf,
        );
        restore_v_event(&mut *dict, &mut save_v_event);
        try_leave(&tstate, &mut err);
        if error_set(&err) {
            if !ui_has(K_UIMESSAGES) {
                msg_putchar(b'\n' as i32);
            }
            g_st().msg_scroll = 1;
            msg_puts_hl(err.msg_ptr(), HLF_E, true);
            api_clear_error(&mut err);
            redrawcmd();
        }
    }
}

fn command_line_changed(s: &mut CommandLineState) -> i32 {
    let prev_cmdpreview = g_st().cmdpreview;
    if s.firstc == b':' as i32
        && g_st().current_sctx.sc_sid == 0
        && !crate::memory::cstr_is_empty(opt_vars().p_icm)
        && g_st().cmdline_star == 0
        && vpeekc_any() == 0
        && cmdpreview_may_show(s)
    {
        // 'inccommand' preview has been shown.
    } else {
        g_st().cmdpreview = false;
        if prev_cmdpreview {
            update_screen();
        }
        if s.xpc.xp_context == EXPAND_NOTHING && (g_st().key_typed || vpeekc() == NUL as i32) {
            may_do_incsearch_highlighting(s.firstc, s.count, &mut s.is_state);
        }
    }

    if !getln_st().ccline.cmdbuff_replaced
        && (getln_st().ccline.cmdpos != s.prev_cmdpos
            || s.prev_cmdbuff.as_ref().is_some_and(|prev| {
                crate::memory::cstr_as_bytes(prev.as_ptr())
                    != crate::memory::cstr_as_bytes(getln_st().ccline.cmdbuff)
            }))
    {
        do_autocmd_cmdlinechanged(if s.firstc > 0 { s.firstc } else { b'-' as i32 });
    }

    may_trigger_cursormovedc(s);

    if opt_vars().p_arshape != 0
        && opt_vars().p_tbidi == 0
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
    if g_st().msg_scrolled == 0 {
        compute_cmdrow();
    }
    if !getln_st().ccline.one_key {
        msg(c"", 0);
        g_st().redraw_cmdline = true;
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
    prompt: impl crate::strings::IntoCCharPtr,
    hl_id: i32,
    xp_context: i32,
    xp_arg: impl crate::strings::IntoCCharPtr,
    highlight_callback: Callback,
    one_key: bool,
    mouse_used: *mut bool,
) -> Option<std::ffi::CString> {
    let prompt = prompt.into_c_char_ptr();
    let xp_arg = xp_arg.into_c_char_ptr();
    {
        let msg_col_save = g_st().msg_col;

        let mut did_save_ccline = false;
        if !getln_st().ccline.cmdbuff.is_null() {
            save_cmdline();
            did_save_ccline = true;
        } else {
            getln_st().ccline = CmdlineInfo::new();
        }
        getln_st().ccline.prompt_id = getln_st().last_prompt_id;
        getln_st().last_prompt_id = getln_st().last_prompt_id.wrapping_add(1);
        getln_st().ccline.cmdprompt = if prompt.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(prompt))
        };
        getln_st().ccline.hl_id = hl_id;
        getln_st().ccline.xp_context = xp_context;
        getln_st().ccline.xp_arg = if xp_arg.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(xp_arg))
        };
        getln_st().ccline.input_fn = i32::from(firstc == b'@' as i32);
        getln_st().ccline.highlight_callback = highlight_callback;
        getln_st().ccline.one_key = one_key;
        getln_st().ccline.mouse_used = (!mouse_used.is_null()).then_some(false);

        let cmd_silent_saved = g_st().cmd_silent;
        let msg_silent_saved = g_st().msg_silent;
        g_st().msg_silent = 0;
        g_st().cmd_silent = false;

        let ret = command_line_enter(firstc, 1, 0, false);
        if !mouse_used.is_null() {
            unsafe {
                *mouse_used = getln_st().ccline.mouse_used.unwrap_or(false);
            }
        }
        getln_st().ccline.redraw_state = K_CMD_REDRAW_NONE;
        getln_st().ccline.highlight_callback = Callback::NONE;

        if did_save_ccline {
            restore_cmdline();
        }
        g_st().msg_silent = msg_silent_saved;
        g_st().cmd_silent = cmd_silent_saved;
        if !getln_st().ccline.cmdbuff.is_null() {
            g_st().msg_col = msg_col_save;
        }

        ret
    }
}

/// Parse `'wildmode'`, updating flags only when non-NULL and the value is valid.
pub fn check_opt_wim(value: impl crate::strings::IntoCCharPtr, flags: *mut u8) -> bool {
    let value = value.into_c_char_ptr();
    {
        let mut new_wim_flags: [u8; 4] = [0; 4];
        let mut idx: usize = 0;

        let mut p = value;
        while !crate::memory::cstr_is_empty(p) {
            let mut i: usize = 0;
            while ascii_isalpha(crate::memory::cstr_char_at(p, i)) {
                i += 1;
            }
            if crate::memory::cstr_byte_at(p, i) != 0
                && crate::memory::cstr_byte_at(p, i) != b','
                && crate::memory::cstr_byte_at(p, i) != b':'
            {
                return false;
            }
            if i == 7 && (crate::memory::cstr_prefix_bytes(p, 7) == &b"longest"[..7]) {
                new_wim_flags[idx] |= K_OPT_WIM_FLAG_LONGEST;
            } else if i == 4 && (crate::memory::cstr_prefix_bytes(p, 4) == &b"full"[..4]) {
                new_wim_flags[idx] |= K_OPT_WIM_FLAG_FULL;
            } else if i == 4 && (crate::memory::cstr_prefix_bytes(p, 4) == &b"list"[..4]) {
                new_wim_flags[idx] |= K_OPT_WIM_FLAG_LIST;
            } else if i == 8 && (crate::memory::cstr_prefix_bytes(p, 8) == &b"lastused"[..8]) {
                new_wim_flags[idx] |= K_OPT_WIM_FLAG_LASTUSED;
            } else if i == 8 && (crate::memory::cstr_prefix_bytes(p, 8) == &b"noselect"[..8]) {
                new_wim_flags[idx] |= K_OPT_WIM_FLAG_NOSELECT;
            } else if i == 8 && (crate::memory::cstr_prefix_bytes(p, 8) == &b"noinsert"[..8]) {
                new_wim_flags[idx] |= K_OPT_WIM_FLAG_NOINSERT;
            } else {
                return false;
            }
            p = p.wrapping_add(i);
            if crate::memory::cstr_is_empty(p) {
                break;
            }
            if crate::memory::cstr_first_byte(p) == b',' {
                if idx == 3 {
                    return false;
                }
                idx += 1;
            }
            p = p.wrapping_add(1);
        }

        while idx < 3 {
            new_wim_flags[idx + 1] = new_wim_flags[idx];
            idx += 1;
        }

        if !flags.is_null() {
            unsafe { ptr::copy_nonoverlapping(new_wim_flags.as_ptr(), flags, 4) };
        }
        true
    }
}

/// Return true when the text must not be changed and we can't switch to
/// another window or buffer.
pub fn text_locked() -> bool {
    if expr_map_locked() {
        return true;
    }
    g_st().textlock != 0
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
    if curbuf_mut().b_ro_locked > 0 {
        emsg(gettext(E_CANNOT_EDIT_OTHER_BUF));
        return true;
    }
    allbuf_locked()
}

pub fn allbuf_locked() -> bool {
    if g_st().allbuf_lock > 0 {
        emsg(gettext(
            c"E811: Not allowed to change buffer information now",
        ));
        return true;
    }
    false
}

fn cmdline_charsize(idx: i32) -> i32 {
    if g_st().cmdline_star > 0 {
        return 1;
    }
    ptr2cells(getln_st().ccline.cmdbuff.wrapping_add(idx as usize))
}

/// Compute the offset of the cursor on the command line for the prompt and
/// indent.
fn cmd_startcol() -> i32 {
    getln_st().ccline.cmdindent
        + if getln_st().ccline.cmdfirstc != NUL as i32 {
            1
        } else {
            0
        }
}

/// Compute the column position for a byte position on the command line.
pub fn cmd_screencol(bytepos: i32) -> i32 {
    unsafe {
        let mut m: i32;
        let mut col = cmd_startcol();
        if g_st().key_typed {
            m = if !g_st().cmdline_win.is_null() {
                (*g_st().cmdline_win).w_view_width * (*g_st().cmdline_win).w_view_height
            } else {
                g_st().columns_global * g_st().rows_global
            };
            if m < 0 {
                m = MAXCOL;
            }
        } else {
            m = MAXCOL;
        }

        let mut i: i32 = 0;
        while i < getln_st().ccline.cmdlen && i < bytepos {
            let c = cmdline_charsize(i);
            correct_screencol(i, c, &mut col);

            col += c;
            if col >= m {
                col -= c;
                break;
            }
            i += utfc_ptr2len(getln_st().ccline.cmdbuff.wrapping_add(i as usize));
        }
        col
    }
}

/// Check if the character at `idx`, which is `cells` wide, is a multi-byte
/// character that doesn't fit, so that a `>` must be displayed.
fn correct_screencol(idx: i32, cells: i32, col: &mut i32) {
    if utfc_ptr2len(getln_st().ccline.cmdbuff.wrapping_add(idx as usize)) > 1
        && utf_ptr2cells(getln_st().ccline.cmdbuff.wrapping_add(idx as usize)) > 1
        && (*col) % g_st().columns_global + cells > g_st().columns_global
    {
        *col += 1;
    }
}

/// Get an Ex command line for the `:` command.
pub fn getexline(c: i32, indent: i32, do_concat: bool) -> Option<std::ffi::CString> {
    if g_st().exec_from_reg && vpeekc() == b':' as i32 {
        vgetc();
    }

    getcmdline(c, 1, indent, do_concat)
}

pub fn cmdline_overstrike() -> bool {
    getln_st().ccline.overstrike != 0
}

/// Return true if the cursor is at the end of the cmdline.
pub fn cmdline_at_end() -> bool {
    getln_st().ccline.cmdpos >= getln_st().ccline.cmdlen
}

/// Deallocate a command line buffer, updating the buffer size and length.
fn dealloc_cmdbuff() {
    getln_st().ccline.cmdbuff_vec = Vec::new();
    getln_st().ccline.cmdbuff = ptr::null_mut();
    getln_st().ccline.cmdbufflen = 0;
    getln_st().ccline.cmdlen = 0;
}

/// Allocate a new command line buffer.
fn alloc_cmdbuff(mut len: i32) {
    if len < 80 {
        len = 100;
    } else {
        len += 20;
    }

    getln_st().ccline.cmdbuff_vec = vec![0; len as usize];
    getln_st().ccline.cmdbuff = getln_st().ccline.cmdbuff_vec.as_mut_ptr();
    getln_st().ccline.cmdbufflen = len;
}

/// Re-allocate the command line to length `len` + something extra.
pub fn realloc_cmdbuff(mut len: i32) {
    unsafe {
        if len < getln_st().ccline.cmdbufflen {
            return;
        }

        if len < 80 {
            len = 100;
        } else {
            len += 20;
        }

        let p = getln_st().ccline.cmdbuff;
        getln_st().ccline.cmdbuff_vec.resize(len as usize, 0);
        getln_st().ccline.cmdbuff = getln_st().ccline.cmdbuff_vec.as_mut_ptr();
        getln_st().ccline.cmdbufflen = len;
        *getln_st()
            .ccline
            .cmdbuff
            .wrapping_add(getln_st().ccline.cmdlen as usize) = NUL;

        if !getln_st().ccline.xpc.is_null()
            && !(*getln_st().ccline.xpc).xp_pattern.is_null()
            && (*getln_st().ccline.xpc).xp_context != EXPAND_NOTHING
            && (*getln_st().ccline.xpc).xp_context != EXPAND_UNSUCCESSFUL
        {
            let i = crate::memory::ptr_offset_from((*getln_st().ccline.xpc).xp_pattern, p) as i32;

            if i >= 0 && i <= getln_st().ccline.cmdlen {
                (*getln_st().ccline.xpc).xp_pattern =
                    getln_st().ccline.cmdbuff.wrapping_add(i as usize);
            }
        }
    }
}

/// Color expression cmdline using built-in expressions parser.
fn color_expr_cmdline(colored_ccline: &CmdlineInfo, ret_ccline_colors: &mut ColoredCmdline) {
    let parser_lines = [
        ParserLine {
            data: colored_ccline.cmdbuff.cast::<u8>(),
            size: crate::memory::cstr_len(colored_ccline.cmdbuff),
        },
        ParserLine {
            data: ptr::null(),
            size: 0,
        },
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
            hl_id: syn_name2id(chunk.group.as_ptr()),
        });
        prev_end = chunk.end_col;
    }
    if prev_end < colored_ccline.cmdlen as usize {
        ret_ccline_colors.colors.push(CmdlineColorChunk {
            start: prev_end as i32,
            end: colored_ccline.cmdlen,
            hl_id: 0,
        });
    }
}

/// Color command-line.
fn color_cmdline(colored_ccline: &mut CmdlineInfo) -> bool {
    let mut printed_errmsg = false;
    let mut ret = true;

    let ccline_colors: *mut _ = &raw mut colored_ccline.last_colors;
    let ccline_colors = unsafe { &mut *ccline_colors };

    if ccline_colors.prompt_id == colored_ccline.prompt_id
        && ccline_colors.cmdbuff.as_ref().is_some_and(|cb| {
            crate::memory::cstr_as_bytes(cb.as_ptr())
                == crate::memory::cstr_as_bytes(colored_ccline.cmdbuff)
        })
    {
        return ret;
    }

    ccline_colors.colors.clear();

    if crate::memory::cstr_is_empty(colored_ccline.cmdbuff) {
        ccline_colors.cmdbuff = None;
        return ret;
    }

    let mut arg_owned: Option<std::ffi::CString> = None;
    let mut arg = TypVal {
        v_type: VarType::VarString,
        v_lock: 0,
        vval: TypvalVval::String(colored_ccline.cmdbuff),
    };
    let mut tv = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    };

    let mut color_cb = Callback::NONE;
    let mut can_free_cb = false;
    let mut err = error_init();
    let mut err_errmsg: *const c_char = E_INTERN2.as_ptr();
    let mut dgc_ret = true;

    'color_cmdline_end: {
        'color_cmdline_error: {
            if colored_ccline.prompt_id != getln_st().prev_prompt_id {
                getln_st().prev_prompt_errors = 0;
                getln_st().prev_prompt_id = colored_ccline.prompt_id;
            } else if getln_st().prev_prompt_errors >= MAX_CB_ERRORS {
                break 'color_cmdline_end;
            }
            if colored_ccline.highlight_callback.cb_type() != K_CALLBACK_NONE_CONST {
                debug_assert!(colored_ccline.input_fn != 0);
                color_cb = colored_ccline.highlight_callback.clone();
            } else if colored_ccline.cmdfirstc == b':' as i32 {
                let mut tstate = TryState::new();
                try_enter(&mut tstate);
                err_errmsg = c"E5408: Unable to get g:Nvim_color_cmdline callback: %s".as_ptr();
                dgc_ret = tv_dict_get_callback(
                    Some(&mut *get_globvar_dict()),
                    c"Nvim_color_cmdline",
                    18,
                    &mut color_cb,
                );
                try_leave(&tstate, &mut err);
                can_free_cb = true;
            } else if colored_ccline.cmdfirstc == b'=' as i32 {
                color_expr_cmdline(colored_ccline, &mut *ccline_colors);
            }
            if error_set(&err) || !dgc_ret {
                break 'color_cmdline_error;
            }

            if color_cb.cb_type() == K_CALLBACK_NONE_CONST {
                break 'color_cmdline_end;
            }
            if crate::memory::cstr_byte_at(colored_ccline.cmdbuff, colored_ccline.cmdlen as usize)
                != 0
            {
                let cstr = std::ffi::CString::new(unsafe {
                    std::slice::from_raw_parts(
                        colored_ccline.cmdbuff.cast::<u8>(),
                        colored_ccline.cmdlen as usize,
                    )
                })
                .unwrap_or_default();
                arg.vval = crate::eval::typval::TypvalVval::String(cstr.as_ptr().cast_mut());
                arg_owned = Some(cstr);
            }

            getln_st().getln_interrupted_highlight = false;
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            err_errmsg = c"E5407: Callback has thrown an exception: %s".as_ptr();
            let saved_msg_col = g_st().msg_col;
            g_st().msg_silent += 1;
            let cbcall_ret = callback_call(&mut color_cb, 1, &raw mut arg, &mut tv);
            g_st().msg_silent -= 1;
            g_st().msg_col = saved_msg_col;
            if g_st().got_int {
                getln_st().getln_interrupted_highlight = true;
            }
            try_leave(&tstate, &mut err);
            if error_set(&err) || !cbcall_ret {
                break 'color_cmdline_error;
            }
            if tv.v_type != VarType::VarList {
                g_st().msg_scroll = 1;
                msg_putchar(b'\n' as i32);
                smsg!(HLF_E, c"%s", gettext(c"E5400: Callback should return list"),);
                printed_errmsg = true;
                break 'color_cmdline_error;
            }
            let v_list = tv.vval.v_list();
            if v_list.is_null() {
                break 'color_cmdline_end;
            }
            let mut prev_end: Varnumber = 0;
            for (i, li) in
                (0_i32..).zip(crate::eval::typval::list_iter_ptr(tv_list_first(unsafe {
                    v_list.as_ref()
                })))
            {
                let li_tv = &mut li.li_tv;
                if li_tv.v_type != VarType::VarList {
                    g_st().msg_scroll = 1;
                    msg_putchar(b'\n' as i32);
                    smsg!(HLF_E, gettext(c"E5401: List item %i is not a List"), i,);
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                let l = li_tv.vval.v_list();
                if tv_list_len(unsafe { l.as_ref() }) != 3 {
                    g_st().msg_scroll = 1;
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5402: List item %i has incorrect length: %d /= 3"),
                        i,
                        tv_list_len(unsafe { l.as_ref() }),
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                let mut error = false;
                let first_li = tv_list_first(unsafe { l.as_ref() });
                let start = tv_get_number_chk(unsafe { &(*first_li).li_tv }, Some(&mut error));
                if error {
                    break 'color_cmdline_error;
                } else if !(prev_end <= start && start < colored_ccline.cmdlen as Varnumber) {
                    g_st().msg_scroll = 1;
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5403: Chunk %i start %lld not in range [%lld, %i)"),
                        i,
                        start,
                        prev_end,
                        colored_ccline.cmdlen,
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                } else if UTF8LEN_TAB_ZERO
                    [crate::memory::cstr_byte_at(colored_ccline.cmdbuff, start as usize) as usize]
                    == 0
                {
                    g_st().msg_scroll = 1;
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
                    ccline_colors.colors.push(CmdlineColorChunk {
                        start: prev_end as i32,
                        end: start as i32,
                        hl_id: 0,
                    });
                }
                let second_li = unsafe { (*first_li).li_next };
                let end = tv_get_number_chk(unsafe { &(*second_li).li_tv }, Some(&mut error));
                if error {
                    break 'color_cmdline_error;
                } else if !(start < end && end <= colored_ccline.cmdlen as Varnumber) {
                    g_st().msg_scroll = 1;
                    msg_putchar(b'\n' as i32);
                    smsg!(
                        HLF_E,
                        gettext(c"E5404: Chunk %i end %lld not in range (%lld, %i]"),
                        i,
                        end,
                        start,
                        colored_ccline.cmdlen,
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                } else if end < colored_ccline.cmdlen as Varnumber
                    && UTF8LEN_TAB_ZERO
                        [crate::memory::cstr_byte_at(colored_ccline.cmdbuff, end as usize) as usize]
                        == 0
                {
                    g_st().msg_scroll = 1;
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
                let last_li = tv_list_last(unsafe { l.as_ref() });
                let group = tv_get_string_chk(unsafe { &(*last_li).li_tv });
                if group.is_null() {
                    break 'color_cmdline_error;
                }
                ccline_colors.colors.push(CmdlineColorChunk {
                    start: start as i32,
                    end: end as i32,
                    hl_id: syn_name2id(group),
                });
            }
            if prev_end < colored_ccline.cmdlen as Varnumber {
                ccline_colors.colors.push(CmdlineColorChunk {
                    start: prev_end as i32,
                    end: colored_ccline.cmdlen,
                    hl_id: 0,
                });
            }
            getln_st().prev_prompt_errors = 0;
            break 'color_cmdline_end;
        }

        // color_cmdline_error:
        if error_set(&err) {
            g_st().msg_scroll = 1;
            msg_putchar(b'\n' as i32);
            smsg!(HLF_E, gettext(err_errmsg), err.msg_ptr());
            printed_errmsg = true;
            api_clear_error(&mut err);
        }
        let _ = printed_errmsg;

        getln_st().prev_prompt_errors += 1;
        ccline_colors.colors.clear();
        redrawcmdline();
        ret = false;
    }

    // color_cmdline_end:
    debug_assert!(!error_set(&err));
    if can_free_cb {
        callback_free(&mut color_cb);
    }
    ccline_colors.prompt_id = colored_ccline.prompt_id;
    ccline_colors.cmdbuff = arg_owned.or_else(|| {
        Some(
            std::ffi::CString::new(unsafe {
                std::slice::from_raw_parts(
                    colored_ccline.cmdbuff.cast::<u8>(),
                    colored_ccline.cmdlen as usize,
                )
            })
            .unwrap_or_default(),
        )
    });
    tv_clear(&mut tv);
    ret
}

/// Draw part of the cmdline at the current cursor position.
fn draw_cmdline(start: i32, len: i32) {
    if getln_st().ccline.cmdbuff.is_null() || !color_cmdline(&mut getln_st().ccline) {
        return;
    }

    if ui_has(K_UICMDLINE) {
        getln_st().ccline.special_char = 0;
        getln_st().ccline.redraw_state = K_CMD_REDRAW_ALL;
        return;
    }

    if g_st().cmdline_star > 0 {
        let mut i: i32 = 0;
        while i < len {
            msg_putchar(b'*' as i32);
            i += utfc_ptr2len(getln_st().ccline.cmdbuff.wrapping_add((start + i) as usize)) - 1;
            i += 1;
        }
    } else if !getln_st().ccline.last_colors.colors.is_empty() {
        for &chunk in &getln_st().ccline.last_colors.colors {
            if chunk.end <= start {
                continue;
            }
            let chunk_start = chunk.start.max(start);
            msg_outtrans_len(
                getln_st().ccline.cmdbuff.wrapping_add(chunk_start as usize),
                chunk.end - chunk_start,
                chunk.hl_id,
                false,
            );
        }
    } else {
        msg_outtrans_len(
            getln_st().ccline.cmdbuff.wrapping_add(start as usize),
            len,
            0,
            false,
        );
    }
}

fn ui_ext_cmdline_show(line: &mut CmdlineInfo) {
    let mut arena = Arena {
        cur_blk: ptr::null_mut(),
        pos: 0,
        size: 0,
    };
    let mut content: Array;
    if g_st().cmdline_star != 0 {
        content = arena_array(&mut arena, 1);
        let mut len: usize = 0;
        let mut p = getln_st().ccline.cmdbuff;
        while !crate::memory::cstr_is_empty(p) {
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
            len += 1;
        }
        let buf = arena_alloc(&mut arena, len, false).cast::<c_char>();
        unsafe { ptr::write_bytes(buf.cast::<u8>(), b'*', len) };
        let mut item = arena_array(&mut arena, 3);
        add_c(&mut item, integer_obj(0));
        add_c(&mut item, string_obj(cbuf_as_string(buf, len)));
        add_c(&mut item, integer_obj(0));
        add_c(&mut content, array_obj(item));
    } else if !line.last_colors.colors.is_empty() {
        content = arena_array(&mut arena, line.last_colors.colors.len());
        for &chunk in &line.last_colors.colors {
            let mut item = arena_array(&mut arena, 3);
            add_c(
                &mut item,
                integer_obj(if chunk.hl_id == 0 {
                    0
                } else {
                    syn_id2attr(chunk.hl_id) as Integer
                }),
            );

            debug_assert!(chunk.end >= chunk.start);
            add_c(
                &mut item,
                string_obj(cbuf_as_string(
                    line.cmdbuff.wrapping_add(chunk.start as usize),
                    (chunk.end - chunk.start) as usize,
                )),
            );
            add_c(&mut item, integer_obj(chunk.hl_id as Integer));
            add_c(&mut content, array_obj(item));
        }
    } else {
        let mut item = arena_array(&mut arena, 3);
        add_c(&mut item, integer_obj(0));
        add_c(&mut item, cstr_as_obj(line.cmdbuff));
        add_c(&mut item, integer_obj(0));
        content = arena_array(&mut arena, 1);
        add_c(&mut content, array_obj(item));
    }
    let mut charbuf: [c_char; 2] = [line.cmdfirstc as c_char, 0];
    ui_call_cmdline_show(
        content,
        line.cmdpos as Integer,
        cstr_as_string(charbuf.as_ptr()),
        cstr_as_string(line.cmdprompt.as_ref().map_or(ptr::null(), |p| p.as_ptr())),
        line.cmdindent as Integer,
        line.level as Integer,
        if line.cmdprompt.is_some() {
            line.hl_id as Integer
        } else {
            -1
        },
    );
    if line.special_char != 0 {
        charbuf[0] = line.special_char as c_char;
        ui_call_cmdline_special_char(
            cstr_as_string(charbuf.as_ptr()),
            line.special_shift,
            line.level as Integer,
        );
    }
    arena_mem_free(arena_finish(&mut arena));
}

pub fn ui_ext_cmdline_block_append(indent: usize, line: impl crate::strings::IntoCCharPtr) {
    let line = line.into_c_char_ptr();
    {
        let line_len = crate::memory::cstr_len(line);
        let mut buf: Vec<c_char> = Vec::with_capacity(indent + line_len);
        buf.resize(indent, b' ' as c_char);
        buf.extend_from_slice(unsafe { std::slice::from_raw_parts(line, line_len) });

        let mut item = Array {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        array_add(&mut item, integer_obj(0));
        array_add(
            &mut item,
            string_obj(crate::api::private::helpers::cbuf_to_string(
                buf.as_ptr(),
                buf.len(),
            )),
        );
        array_add(&mut item, integer_obj(0));
        let mut content = Array {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        array_add(&mut content, array_obj(item));
        array_add(&mut getln_st().cmdline_block, array_obj(content));
        if getln_st().cmdline_block.size > 1 {
            ui_call_cmdline_block_append(content);
        } else {
            ui_call_cmdline_block_show(getln_st().cmdline_block);
        }
    }
}

pub fn ui_ext_cmdline_block_leave() {
    api_free_array(getln_st().cmdline_block);
    getln_st().cmdline_block = Array {
        size: 0,
        capacity: 0,
        items: ptr::null_mut(),
    };
    ui_call_cmdline_block_hide();
}

/// Extra redrawing needed for `redraw!` and on `ui_attach`.
pub fn cmdline_screen_cleared() {
    unsafe {
        if !ui_has(K_UICMDLINE) {
            return;
        }

        if getln_st().cmdline_block.size != 0 {
            ui_call_cmdline_block_show(getln_st().cmdline_block);
        }

        let mut prev_level = getln_st().ccline.level - 1;
        let mut line = (&raw mut getln_st().ccline.prev_ccline)
            .as_mut()
            .unwrap()
            .as_deref_mut()
            .map_or(ptr::null_mut(), ptr::from_mut);
        while prev_level > 0 && !line.is_null() {
            if (*line).level == prev_level {
                (*line).redraw_state = K_CMD_REDRAW_ALL;
                prev_level -= 1;
            }
            line = (*line)
                .prev_ccline
                .as_deref_mut()
                .map_or(ptr::null_mut(), ptr::from_mut);
        }
        redrawcmd();
    }
}

/// Called by `ui_flush`, do what redraws necessary to keep cmdline updated.
pub fn cmdline_ui_flush() {
    unsafe {
        if !ui_has(K_UICMDLINE) {
            return;
        }
        let mut level = getln_st().ccline.level;
        let mut line: *mut CmdlineInfo = &raw mut getln_st().ccline;
        while level > 0 && !line.is_null() {
            if (*line).level == level {
                let redraw_state = (*line).redraw_state;
                (*line).redraw_state = K_CMD_REDRAW_NONE;
                if redraw_state == K_CMD_REDRAW_ALL {
                    g_st().cmdline_was_last_drawn = true;
                    ui_ext_cmdline_show(&mut *line);
                } else if redraw_state == K_CMD_REDRAW_POS && g_st().cmdline_was_last_drawn {
                    ui_call_cmdline_pos((*line).cmdpos as Integer, (*line).level as Integer);
                }
                level -= 1;
            }
            line = (*line)
                .prev_ccline
                .as_deref_mut()
                .map_or(ptr::null_mut(), ptr::from_mut);
        }
    }
}

/// Put a character on the command line.
pub fn putcmdline(c: u8, shift: bool) {
    if g_st().cmd_silent {
        return;
    }
    if !ui_has(K_UICMDLINE) {
        g_st().msg_no_more = true;
        msg_putchar(i32::from(c));
        if shift {
            draw_cmdline(
                getln_st().ccline.cmdpos,
                getln_st().ccline.cmdlen - getln_st().ccline.cmdpos,
            );
        }
        g_st().msg_no_more = false;
    } else if getln_st().ccline.redraw_state != K_CMD_REDRAW_ALL {
        let charbuf: [u8; 2] = [c, 0];
        ui_call_cmdline_special_char(
            cstr_as_string(charbuf.as_ptr().cast()),
            shift,
            getln_st().ccline.level as Integer,
        );
    }
    cursorcmd();
    getln_st().ccline.special_char = c;
    getln_st().ccline.special_shift = shift;
    ui_cursor_shape();
}

/// Undo a `putcmdline(c, false)`.
pub fn unputcmdline() {
    if g_st().cmd_silent {
        return;
    }
    g_st().msg_no_more = true;
    if getln_st().ccline.cmdlen == getln_st().ccline.cmdpos && !ui_has(K_UICMDLINE) {
        msg_putchar(b' ' as i32);
    } else {
        draw_cmdline(
            getln_st().ccline.cmdpos,
            utfc_ptr2len(
                getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize),
            ),
        );
    }
    g_st().msg_no_more = false;
    cursorcmd();
    getln_st().ccline.special_char = 0;
    ui_cursor_shape();
}

/// Put the given string, of the given length, onto the command line.
pub fn put_on_cmdline(str: impl crate::strings::IntoCCharPtr, mut len: i32, redraw: bool) {
    let str = str.into_c_char_ptr();
    {
        if len < 0 {
            len = crate::memory::cstr_len(str) as i32;
        }

        realloc_cmdbuff(getln_st().ccline.cmdlen + len + 1);

        if getln_st().ccline.overstrike == 0 {
            unsafe {
                ptr::copy(
                    getln_st()
                        .ccline
                        .cmdbuff
                        .wrapping_add(getln_st().ccline.cmdpos as usize),
                    getln_st()
                        .ccline
                        .cmdbuff
                        .wrapping_add((getln_st().ccline.cmdpos + len) as usize),
                    (getln_st().ccline.cmdlen - getln_st().ccline.cmdpos) as usize,
                )
            };
            getln_st().ccline.cmdlen += len;
        } else {
            let mut m: i32 = 0;
            let mut i: i32 = 0;
            while i < len {
                m += 1;
                i += utfc_ptr2len(str.wrapping_add(i as usize));
            }
            i = getln_st().ccline.cmdpos;
            while i < getln_st().ccline.cmdlen && m > 0 {
                m -= 1;
                i += utfc_ptr2len(getln_st().ccline.cmdbuff.wrapping_add(i as usize));
            }
            if i < getln_st().ccline.cmdlen {
                unsafe {
                    ptr::copy(
                        getln_st().ccline.cmdbuff.wrapping_add(i as usize),
                        getln_st()
                            .ccline
                            .cmdbuff
                            .wrapping_add((getln_st().ccline.cmdpos + len) as usize),
                        (getln_st().ccline.cmdlen - i) as usize,
                    )
                };
                getln_st().ccline.cmdlen += getln_st().ccline.cmdpos + len - i;
            } else {
                getln_st().ccline.cmdlen = getln_st().ccline.cmdpos + len;
            }
        }
        unsafe {
            ptr::copy(
                str,
                getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize),
                len as usize,
            )
        };
        unsafe {
            *getln_st()
                .ccline
                .cmdbuff
                .wrapping_add(getln_st().ccline.cmdlen as usize) = NUL;
        }

        if getln_st().ccline.cmdpos > 0
            && (unsafe {
                *getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize)
            } as u8)
                >= 0x80
        {
            let i = utf_head_off(
                getln_st().ccline.cmdbuff,
                getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize),
            );
            if i != 0 {
                getln_st().ccline.cmdpos -= i;
                len += i;
                getln_st().ccline.cmdspos = cmd_screencol(getln_st().ccline.cmdpos);
            }
        }

        if redraw && !g_st().cmd_silent {
            g_st().msg_no_more = true;
            let i = g_st().cmdline_row;
            cursorcmd();
            draw_cmdline(
                getln_st().ccline.cmdpos,
                getln_st().ccline.cmdlen - getln_st().ccline.cmdpos,
            );
            if g_st().cmdline_row != i || getln_st().ccline.overstrike != 0 {
                msg_clr_eos();
            }
            g_st().msg_no_more = false;
        }
        let mut m: i32;
        if g_st().key_typed {
            m = g_st().columns_global * g_st().rows_global;
            if m < 0 {
                m = MAXCOL;
            }
        } else {
            m = MAXCOL;
        }
        let mut i: i32 = 0;
        while i < len {
            let mut c = cmdline_charsize(getln_st().ccline.cmdpos);
            correct_screencol(getln_st().ccline.cmdpos, c, &mut getln_st().ccline.cmdspos);
            if getln_st().ccline.cmdspos + c < m {
                getln_st().ccline.cmdspos += c;
            }
            c = utfc_ptr2len(
                getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize),
            ) - 1;
            c = c.min(len - i - 1);
            getln_st().ccline.cmdpos += c;
            i += c;
            getln_st().ccline.cmdpos += 1;
            i += 1;
        }

        if redraw {
            msg_check();
        }
    }
}

/// Save `getln_st().ccline`.
fn save_cmdline() {
    let prev = std::mem::replace(&mut getln_st().ccline, CmdlineInfo::new());
    getln_st().ccline.prev_ccline = Some(Box::new(prev));
}

/// Restore `getln_st().ccline` after it has been saved with `save_cmdline()`.
fn restore_cmdline() {
    let prev = getln_st().ccline.prev_ccline.take();
    getln_st().ccline = prev.map_or_else(CmdlineInfo::new, |b| *b);
}

/// Paste a yank register into the command line.
fn cmdline_paste(regname: i32, literally: bool, remcr: bool) -> bool {
    let mut arg: *mut c_char = ptr::null_mut();
    let mut owned: Option<Vec<c_char>> = None;

    if regname != CTRL_F_CONST
        && regname != CTRL_P
        && regname != CTRL_W_CONST
        && regname != CTRL_A
        && regname != CTRL_L_CONST
        && !valid_yank_reg(regname, false)
    {
        return FAIL != 0;
    }

    line_breakcheck();
    if g_st().got_int {
        return FAIL != 0;
    }

    g_st().textlock += 1;
    let i = get_spec_reg(regname, &raw mut arg, &mut owned, true);
    g_st().textlock -= 1;

    if i {
        if arg.is_null() {
            return FAIL != 0;
        }

        let mut p = arg;
        if opt_vars().p_is != 0 && regname == CTRL_W_CONST {
            let mut w = getln_st()
                .ccline
                .cmdbuff
                .wrapping_add(getln_st().ccline.cmdpos as usize);
            while w > getln_st().ccline.cmdbuff {
                let len = utf_head_off(getln_st().ccline.cmdbuff, w.wrapping_sub(1)) + 1;
                if !vim_iswordc(utf_ptr2char(w.wrapping_sub(len as usize))) {
                    break;
                }
                w = w.wrapping_sub(len as usize);
            }
            let len = crate::memory::ptr_offset_from(
                getln_st()
                    .ccline
                    .cmdbuff
                    .wrapping_add(getln_st().ccline.cmdpos as usize),
                w,
            ) as usize;
            let matches = if opt_vars().p_ic != 0 {
                crate::memory::cstr_prefix_bytes(w, len)
                    .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(arg, len))
            } else {
                crate::memory::cstr_prefix_bytes(w, len)
                    == crate::memory::cstr_prefix_bytes(arg, len)
            };
            if matches {
                p = p.wrapping_add(len);
            }
        }

        cmdline_paste_str(p, literally);
        return OK != 0;
    }

    cmdline_paste_reg(regname, literally, remcr)
}

/// Put a string on the command line.
pub fn cmdline_paste_str(s: impl crate::strings::IntoCCharPtr, literally: bool) {
    let mut s = s.into_c_char_ptr();
    {
        if literally {
            put_on_cmdline(s, -1, true);
        } else {
            while !crate::memory::cstr_is_empty(s) {
                let cv = (crate::memory::cstr_first_byte(s)) as i32;
                if cv == CTRL_V_CONST && crate::memory::cstr_byte_at(s, 1) != 0 {
                    s = s.wrapping_add(1);
                }
                let c = mb_cptr2char_adv(&raw mut s);
                if cv == CTRL_V_CONST
                    || c == ESC
                    || c == CTRL_C_CONST
                    || c == CAR
                    || c == NL
                    || c == CTRL_L_CONST
                    || (c == CTRL_BSL && unsafe { *s } as i32 == CTRL_N)
                {
                    stuffchar_readbuff(CTRL_V_CONST);
                }
                stuffchar_readbuff(c);
            }
        }
    }
}

pub fn redrawcmdline() {
    if g_st().cmd_silent {
        return;
    }
    g_st().need_wait_return = false;
    compute_cmdrow();
    redrawcmd();
    cursorcmd();
    ui_cursor_shape();
}

fn redrawcmdprompt() {
    unsafe {
        if g_st().cmd_silent {
            return;
        }
        if ui_has(K_UICMDLINE) {
            getln_st().ccline.redraw_state = K_CMD_REDRAW_ALL;
            return;
        }
        if getln_st().ccline.cmdfirstc != NUL as i32 {
            msg_putchar(getln_st().ccline.cmdfirstc);
        }
        let cp = &raw const getln_st().ccline.cmdprompt;
        if let Some(ref prompt) = *cp {
            msg_puts_hl(prompt.as_ptr(), getln_st().ccline.hl_id, false);
            getln_st().ccline.cmdindent =
                g_st().msg_col + (g_st().msg_row - g_st().cmdline_row) * g_st().columns_global;
            if getln_st().ccline.cmdfirstc != NUL as i32 {
                getln_st().ccline.cmdindent -= 1;
            }
        } else {
            let mut i = getln_st().ccline.cmdindent;
            while i > 0 {
                msg_putchar(b' ' as i32);
                i -= 1;
            }
        }
    }
}

/// Redraw what is currently on the command line.
pub fn redrawcmd() {
    if g_st().cmd_silent {
        return;
    }

    if ui_has(K_UICMDLINE) {
        draw_cmdline(0, getln_st().ccline.cmdlen);
        return;
    }

    if getln_st().ccline.cmdbuff.is_null() {
        msg_cursor_goto(g_st().cmdline_row, 0);
        msg_clr_eos();
        return;
    }

    g_st().redrawing_cmdline = true;

    sb_text_restart_cmdline();
    msg_start();
    msg_starthere();
    redrawcmdprompt();

    g_st().msg_no_more = true;
    draw_cmdline(0, getln_st().ccline.cmdlen);
    msg_clr_eos();
    g_st().msg_no_more = false;

    getln_st().ccline.cmdspos = cmd_screencol(getln_st().ccline.cmdpos);

    if getln_st().ccline.special_char != 0 {
        putcmdline(
            getln_st().ccline.special_char,
            getln_st().ccline.special_shift,
        );
    }

    g_st().msg_scroll = 0;
    g_st().skip_redraw = false;
    g_st().cmdline_was_last_drawn = true;
    g_st().redrawing_cmdline = false;
}

pub fn compute_cmdrow() {
    if g_st().msg_scrolled != 0 {
        g_st().cmdline_row = g_st().rows_global - 1;
    } else {
        let wp = lastwin_nofloating(ptr::null_mut());
        g_st().cmdline_row =
            wp.w_winrow + wp.w_height + wp.w_hsep_height + wp.w_status_height + global_stl_height();
    }
    if g_st().cmdline_row == g_st().rows_global && opt_vars().p_ch > 0 {
        g_st().cmdline_row -= 1;
    }
    g_st().lines_left = g_st().cmdline_row;
}

pub fn cursorcmd() {
    if g_st().cmd_silent || ui_has(K_UICMDLINE) {
        return;
    }

    g_st().msg_row = g_st().cmdline_row + (getln_st().ccline.cmdspos / g_st().columns_global);
    g_st().msg_col = getln_st().ccline.cmdspos % g_st().columns_global;
    g_st().msg_row = g_st().msg_row.min(g_st().rows_global - 1);

    msg_cursor_goto(g_st().msg_row, g_st().msg_col);
}

pub fn gotocmdline(clr: bool) {
    if ui_has(K_UICMDLINE) {
        return;
    }
    msg_start();
    g_st().msg_col = 0;
    if clr {
        msg_clr_eos();
    }
    msg_cursor_goto(g_st().cmdline_row, 0);
}

/// Check the word in front of the cursor for an abbreviation.
fn ccheck_abbr(c: i32) -> i32 {
    let mut spos: i32 = 0;

    if opt_vars().p_paste != 0 || g_st().no_abbr {
        return 0;
    }

    while spos < getln_st().ccline.cmdlen
        && ascii_iswhite(unsafe { *getln_st().ccline.cmdbuff.wrapping_add(spos as usize) } as i32)
    {
        spos += 1;
    }
    if getln_st().ccline.cmdlen - spos > 5
        && crate::memory::cstr_byte_at(getln_st().ccline.cmdbuff, spos as usize) == b'\''
        && unsafe { *getln_st().ccline.cmdbuff.wrapping_add((spos + 2) as usize) } == b',' as c_char
        && unsafe { *getln_st().ccline.cmdbuff.wrapping_add((spos + 3) as usize) }
            == b'\'' as c_char
    {
        spos += 5;
    } else {
        spos = 0;
    }

    i32::from(check_abbr(
        c,
        getln_st().ccline.cmdbuff,
        getln_st().ccline.cmdpos,
        spos,
    ))
}

/// Escape special characters in `fname`, depending on `what`.
pub fn vim_strsave_fnameescape(
    fname: impl crate::strings::IntoCCharPtr,
    what: i32,
) -> std::ffi::CString {
    let fname = fname.into_c_char_ptr();
    #[cfg(windows)]
    let mut p = {
        let mut buf: [c_char; 32] = [0; 32];
        let mut j: usize = 0;
        let chars = if what == VSE_BUFFER {
            BUFFER_ESC_CHARS.as_ptr()
        } else {
            PATH_ESC_CHARS.as_ptr()
        };
        let cp = chars;
        for &cp_byte in crate::memory::cstr_as_bytes(cp) {
            if (cp_byte != b'[' && cp_byte != b'{' && cp_byte != b'!')
                || !vim_isfilec((cp_byte) as i32)
            {
                buf[j] = cp_byte as c_char;
                j += 1;
            }
        }
        buf[j] = NUL;
        vim_strsave_escaped(fname, buf.as_ptr())
    };

    #[cfg(not(windows))]
    let mut p = {
        let chars = if what == VSE_SHELL {
            SHELL_ESC_CHARS
        } else if what == VSE_BUFFER {
            BUFFER_ESC_CHARS
        } else {
            PATH_ESC_CHARS
        };
        let mut res = vim_strsave_escaped(fname, chars);
        if what == VSE_SHELL && csh_like_shell() != 0 {
            res = vim_strsave_escaped(&res, c"!");
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
    *s = crate::memory::cstr_from_bytes(&out);
}

/// For each file name in `files`:
/// If `orig_pat` starts with `~/`, replace the home directory with `~`.
pub fn tilde_replace(orig_pat: impl crate::strings::IntoCCharPtr, files: &mut [std::ffi::CString]) {
    let orig_pat = orig_pat.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(orig_pat) == b'~'
            && vim_ispathsep((crate::memory::cstr_byte_at(orig_pat, 1)) as i32)
        {
            for f in files {
                *f = home_replace_save(None, f.as_ptr().cast_mut());
            }
        }
    }
}

/// Get a pointer to the current command line info.
pub fn get_cmdline_info() -> &'static mut CmdlineInfo {
    &mut getln_st().ccline
}

pub fn get_cmdline_last_prompt_id() -> u32 {
    getln_st().last_prompt_id
}

/// Get pointer to the command line info to use.
fn get_ccline_ptr() -> *mut CmdlineInfo {
    unsafe {
        let prev_ptr = &raw mut getln_st().ccline.prev_ccline;
        if (g_st().state_global & MODE_CMDLINE) == 0 {
            ptr::null_mut()
        } else if !getln_st().ccline.cmdbuff.is_null() {
            &raw mut getln_st().ccline
        } else if let Some(ref mut prev) = *prev_ptr
            && !prev.cmdbuff.is_null()
        {
            &raw mut **prev
        } else {
            ptr::null_mut()
        }
    }
}

/// Get the current command-line type.
fn get_cmdline_type() -> i32 {
    let p = get_ccline_ptr();

    if p.is_null() {
        return NUL as i32;
    }
    if unsafe { (*p).cmdfirstc } == NUL as i32 {
        return if unsafe { (*p).input_fn } != 0 {
            b'@' as i32
        } else {
            b'-' as i32
        };
    }
    unsafe { (*p).cmdfirstc }
}

/// Get the current command line in allocated memory.
fn get_cmdline_str() -> Option<std::ffi::CString> {
    if g_st().cmdline_star > 0 {
        return None;
    }
    let p = get_ccline_ptr();

    if p.is_null() {
        return None;
    }
    let slice = crate::memory::cstr_slice(unsafe { (*p).cmdbuff }, unsafe { (*p).cmdlen } as usize);
    let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    Some(crate::memory::cstr_from_bytes(&slice[..len]))
}

/// Get the current command-line completion pattern.
fn get_cmdline_completion_pattern() -> Option<std::ffi::CString> {
    unsafe {
        if g_st().cmdline_star > 0 {
            return None;
        }

        let p = get_ccline_ptr();
        if p.is_null() || (*p).xpc.is_null() {
            return None;
        }

        let mut xp_context = (*(*p).xpc).xp_context;
        if xp_context == EXPAND_NOTHING {
            set_expand_context(&mut *(*p).xpc);
            xp_context = (*(*p).xpc).xp_context;
            (*(*p).xpc).xp_context = EXPAND_NOTHING;
        }
        if xp_context == EXPAND_UNSUCCESSFUL {
            return None;
        }

        let compl_pat = (*(*p).xpc).xp_pattern;
        if compl_pat.is_null() {
            return None;
        }

        Some(crate::memory::cstr_to_cstring(compl_pat))
    }
}

/// Get the command-line completion type.
fn get_cmdline_completion() -> Option<std::ffi::CString> {
    unsafe {
        if g_st().cmdline_star > 0 {
            return None;
        }

        let p = get_ccline_ptr();
        if p.is_null() || (*p).xpc.is_null() {
            return None;
        }

        let mut xp_context = (*(*p).xpc).xp_context;
        if xp_context == EXPAND_NOTHING {
            set_expand_context(&mut *(*p).xpc);
            xp_context = (*(*p).xpc).xp_context;
            (*(*p).xpc).xp_context = EXPAND_NOTHING;
        }
        if xp_context == EXPAND_UNSUCCESSFUL {
            return None;
        }

        cmdcomplete_type_to_str(xp_context, (*(*p).xpc).xp_arg)
    }
}

/// `"getcmdcomplpat()"` function.
pub fn f_getcmdcomplpat(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(
        rettv,
        get_cmdline_completion_pattern()
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr()),
    );
}

/// `"getcmdcompltype()"` function.
pub fn f_getcmdcompltype(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(
        rettv,
        get_cmdline_completion()
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr()),
    );
}

/// `"getcmdline()"` function.
pub fn f_getcmdline(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if let Some(s) = get_cmdline_str() {
        crate::eval::typval::tv_set_string_len(rettv, s.as_ptr(), s.as_bytes().len());
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
    }
}

/// `"getcmdpos()"` function.
pub fn f_getcmdpos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let p = get_ccline_ptr();
    rettv.vval = crate::eval::typval::TypvalVval::Number(if !p.is_null() {
        (unsafe { (*p).cmdpos } + 1) as Varnumber
    } else {
        0
    });
}

/// `"getcmdprompt()"` function.
pub fn f_getcmdprompt(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let p = get_ccline_ptr();
        crate::eval::typval::tv_set_string(
            rettv,
            if !p.is_null() {
                (*p).cmdprompt.as_ref().map_or(ptr::null(), |s| s.as_ptr())
            } else {
                ptr::null()
            },
        );
    }
}

/// `"getcmdscreenpos()"` function.
pub fn f_getcmdscreenpos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let p = get_ccline_ptr();
    rettv.vval = crate::eval::typval::TypvalVval::Number(if !p.is_null() {
        (unsafe { (*p).cmdspos } + 1) as Varnumber
    } else {
        0
    });
}

/// `"getcmdtype()"` function.
pub fn f_getcmdtype(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let buf: [c_char; 2] = [get_cmdline_type() as c_char, 0];
    crate::eval::typval::tv_set_string(rettv, buf.as_ptr());
}

/// Set the command line str to `str`.
/// @return 1 when failed, 0 when OK.
fn set_cmdline_str(str: impl crate::strings::IntoCCharPtr, pos: i32) -> i32 {
    let str = str.into_c_char_ptr();
    unsafe {
        let p = get_ccline_ptr();

        if p.is_null() {
            return 1;
        }

        let len = crate::memory::cstr_len(str) as i32;
        realloc_cmdbuff(len + 1);
        (*p).cmdlen = len;
        crate::memory::cstr_cpy((*p).cmdbuff, str);

        (*p).cmdpos = if pos < 0 || pos > (*p).cmdlen {
            (*p).cmdlen
        } else {
            pos
        };
        getln_st().new_cmdpos = (*p).cmdpos;
        (*p).cmdbuff_replaced = true;

        redrawcmd();

        do_autocmd_cmdlinechanged(get_cmdline_type());

        0
    }
}

/// Set the command line byte position to `pos`.
fn set_cmdline_pos(pos: i32) -> i32 {
    let p = get_ccline_ptr();

    if p.is_null() {
        return 1;
    }

    getln_st().new_cmdpos = pos.max(0);

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
        tv_get_string(&argvars[0]),
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
    getln_st().ccline.cmdfirstc
}

/// Get indices that specify a range within a list from a string.
pub fn get_list_range(str: *mut *mut c_char, num1: &mut i32, num2: &mut i32) -> bool {
    unsafe {
        let mut len: i32 = 0;
        let mut first = false;
        let mut num: Varnumber = 0;

        *str = skipwhite(*str);
        if **str == b'-' as c_char || ascii_isdigit(**str as i32) {
            vim_str2nr(
                *str,
                None,
                Some(&mut len),
                0,
                Some(&mut num),
                None,
                0,
                false,
                None,
            );
            *str = (*str).wrapping_add(len as usize);
            if num > i32::MAX as Varnumber {
                return false;
            }

            *num1 = num as i32;
            first = true;
        }
        *str = skipwhite(*str);
        if **str == b',' as c_char {
            *str = skipwhite((*str).wrapping_add(1));
            vim_str2nr(
                *str,
                None,
                Some(&mut len),
                0,
                Some(&mut num),
                None,
                0,
                false,
                None,
            );
            if len > 0 {
                *str = skipwhite((*str).wrapping_add(len as usize));
                if num > i32::MAX as Varnumber {
                    return false;
                }

                *num2 = num as i32;
            } else if !first {
                return false;
            }
        } else if first {
            *num2 = *num1;
        }
        true
    }
}

pub fn cmdline_init() {
    getln_st().ccline = CmdlineInfo::new();
}

/// Validate the `'cedit'` option.
pub fn validate_cedit(args: &mut Optset) -> *const c_char {
    let value = args.os_newval.data.string().data;
    if !crate::memory::cstr_is_empty(value) {
        let n = string_to_key(value);
        if n == 0 || vim_isprintc(n) {
            return E_INVARG.as_ptr();
        }
    }
    ptr::null()
}

/// The `'cedit'` option is changed.
pub fn did_set_cedit(_args: &mut Optset) -> *const c_char {
    getln_st().cedit_key = if crate::memory::cstr_is_empty(opt_vars().p_cedit) {
        -1
    } else {
        string_to_key(opt_vars().p_cedit)
    };
    ptr::null()
}

/// Synchronously calls `vim._core.cmdwin.<action>(...)`.
fn cmdwin_invoke(
    action: impl crate::strings::IntoCCharPtr,
    firstc: i32,
    content: impl crate::strings::IntoCCharPtr,
    pos: i32,
) {
    let content = content.into_c_char_ptr().cast_mut();
    let action = action.into_c_char_ptr();
    {
        let mut fc: [c_char; 2] = [firstc as c_char, 0];
        let mut tv_args = [
            TypVal {
                v_type: VarType::VarString,
                v_lock: 0,
                vval: TypvalVval::String(fc.as_mut_ptr()),
            },
            TypVal {
                v_type: VarType::VarString,
                v_lock: 0,
                vval: TypvalVval::String(if !content.is_null() {
                    content
                } else {
                    c"".as_ptr().cast_mut()
                }),
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
                tv_args.as_mut_ptr()
            } else {
                tv_args.as_mut_ptr().wrapping_add(3)
            },
            None,
        );
    }
}

/// Calls `vim._core.cmdwin.<action>()`, synchronously.
pub fn cmdwin_do_action(action: &std::ffi::CStr) {
    cmdwin_invoke(action.as_ptr(), 0, ptr::null_mut(), 0);
}

/// Deferred event: opens cmdwin after the cmdline-reader unwinds.
pub fn open_cmdwin_event(firstc: i32, content: Option<std::ffi::CString>, pos: i32) {
    let content_ptr = content
        .as_ref()
        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut());
    cmdwin_invoke(c"open", firstc, content_ptr, pos);
}

/// Schedules cmdwin to open after the current cmdline-reader returns.
fn open_cmdwin() -> i32 {
    if text_locked() || g_st().cmdline_star > 0 {
        beep_flush();
        return K_IGNORE;
    }
    if !g_st().cmdwin_buf.is_null() {
        beep_flush();
        return K_IGNORE;
    }
    let ft = get_cmdline_type();
    if ft != b':' as i32 && ft != b'/' as i32 && ft != b'?' as i32 {
        beep_flush();
        return K_IGNORE;
    }

    let content = (!getln_st().ccline.cmdbuff.is_null()).then(|| {
        let bytes =
            crate::memory::cstr_slice(getln_st().ccline.cmdbuff, getln_st().ccline.cmdlen as usize);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        std::ffi::CString::new(&bytes[..end]).unwrap()
    });
    let pos = getln_st().ccline.cmdpos;
    getln_st().ccline.cmdlen = 0;
    getln_st().ccline.cmdpos = 0;

    loop_schedule_deferred(
        &mut g_st().main_loop,
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
    bt_cmdwin(g_st().curbuf) && get_cmdline_type() == NUL as i32
}

/// Get script string.
pub fn script_get(eap: &mut Exarg, lenp: &mut usize) -> Option<std::ffi::CString> {
    let mut cmd = eap.arg;

    if crate::memory::cstr_first_byte(cmd) != b'<'
        || crate::memory::cstr_byte_at(cmd, 1) != b'<'
        || eap.ea_getline.is_none()
    {
        *lenp = crate::memory::cstr_len(eap.arg);
        return if eap.skip != 0 {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(eap.arg))
        };
    }
    cmd = cmd.wrapping_add(2);

    let l = heredoc_get(eap, cmd, true);
    if l.is_null() {
        return None;
    }

    let mut buf: Vec<u8> = if eap.skip == 0 {
        Vec::with_capacity(0x400)
    } else {
        Vec::new()
    };

    for li in crate::eval::typval::list_iter_mut(tv_list_first(unsafe { l.as_ref() })) {
        if eap.skip == 0 {
            let s = tv_get_string(&li.li_tv);
            if !s.is_null() {
                buf.extend_from_slice(crate::memory::cstr_as_bytes(s));
            }
            buf.push(b'\n');
        }
    }
    *lenp = buf.len();
    tv_list_free(l);
    if eap.skip == 0 {
        Some(std::ffi::CString::new(buf).unwrap_or_default())
    } else {
        None
    }
}

/// Used by `f_input()` and `f_inputdialog()` functions.
pub fn get_user_input(argvars: &[TypVal], rettv: &mut TypVal, inputdialog: bool, secret: bool) {
    unsafe {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if g_st().cmdpreview {
            return;
        }

        let prompt: *const c_char;
        let mut defstr: *const c_char = c"".as_ptr();
        let mut cancelreturn: *mut TypVal = ptr::null_mut();
        let mut cancelreturn_strarg2 = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        let mut xp_name: *const c_char = ptr::null();
        let mut input_callback: Callback = Callback::new();
        let mut prompt_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let mut defstr_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let mut cancelreturn_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let mut xp_name_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let def: [c_char; 1] = [0];
        if argvars[0].v_type == VarType::VarDict {
            if argvars[1].v_type != VarType::VarUnknown {
                emsg(gettext(c"E5050: {opts} must be the only argument"));
                return;
            }
            let dict = argvars[0].vval.v_dict();
            prompt = tv_dict_get_string_buf_chk(
                dict.as_ref(),
                c"prompt",
                6,
                prompt_buf.as_mut_ptr(),
                c"",
            );
            if prompt.is_null() {
                return;
            }
            defstr = tv_dict_get_string_buf_chk(
                dict.as_ref(),
                c"default",
                7,
                defstr_buf.as_mut_ptr(),
                c"",
            );
            if defstr.is_null() {
                return;
            }
            let cancelreturn_di = tv_dict_find(dict.as_ref(), c"cancelreturn", 12);
            if !cancelreturn_di.is_null() {
                cancelreturn = &raw mut (*cancelreturn_di).di_tv;
            }
            xp_name = tv_dict_get_string_buf_chk(
                dict.as_ref(),
                c"completion",
                10,
                xp_name_buf.as_mut_ptr(),
                def.as_ptr(),
            );
            if xp_name.is_null() {
                return;
            }
            if xp_name == def.as_ptr() {
                xp_name = ptr::null();
            }
            if !tv_dict_get_callback(dict.as_mut(), c"highlight", 9, &mut input_callback) {
                return;
            }
        } else {
            prompt = tv_get_string_buf_chk(&argvars[0], prompt_buf.as_mut_ptr());
            if prompt.is_null() {
                return;
            }
            if argvars[1].v_type != VarType::VarUnknown {
                defstr = tv_get_string_buf_chk(&argvars[1], defstr_buf.as_mut_ptr());
                if defstr.is_null() {
                    return;
                }
                if argvars[2].v_type != VarType::VarUnknown {
                    let strarg2 = tv_get_string_buf_chk(&argvars[2], cancelreturn_buf.as_mut_ptr());
                    if strarg2.is_null() {
                        return;
                    }
                    if inputdialog {
                        cancelreturn_strarg2.v_type = VarType::VarString;
                        cancelreturn_strarg2.vval =
                            crate::eval::typval::TypvalVval::String(strarg2.cast_mut());
                        cancelreturn = &raw mut cancelreturn_strarg2;
                    } else {
                        xp_name = strarg2;
                    }
                }
            }
        }

        let mut xp_type: i32 = EXPAND_NOTHING;
        let mut xp_arg: Option<std::ffi::CString> = None;
        if !xp_name.is_null() {
            let xp_namelen = crate::memory::cstr_len(xp_name) as i32;
            let mut argt: u32 = 0;
            if !parse_compl_arg(xp_name, xp_namelen, &mut xp_type, &mut argt, &mut xp_arg) {
                return;
            }
        }

        let mut p = prompt;
        if !ui_has(K_UICMDLINE) {
            let lastnl = crate::memory::cstr_as_bytes(prompt)
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(ptr::null_mut(), |i| prompt.wrapping_add(i).cast_mut());
            if !lastnl.is_null() {
                p = lastnl.wrapping_add(1);
                msg_start();
                msg_clr_eos();
                msg_puts_len(
                    prompt,
                    crate::memory::ptr_offset_from(p, prompt),
                    get_echo_hl_id(),
                    false,
                );
                g_st().msg_didout = false;
                msg_starthere();
            }
        }
        g_st().cmdline_row = g_st().msg_row;

        stuff_readbuff_special(defstr);

        let save_ex_normal_busy = g_st().ex_normal_busy;
        g_st().ex_normal_busy = 0;
        atom_payload_start();
        let res = getcmdline_prompt(
            if secret { NUL as i32 } else { b'@' as i32 },
            p,
            get_echo_hl_id(),
            xp_type,
            xp_arg
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            input_callback.clone(),
            false,
            ptr::null_mut(),
        );
        atom_payload_end();
        g_st().ex_normal_busy = save_ex_normal_busy;
        callback_free(&mut input_callback);

        if let Some(res_cstr) = res {
            crate::eval::typval::tv_set_string_len(
                &mut *rettv,
                res_cstr.as_ptr(),
                res_cstr.as_bytes().len(),
            );
        } else if !cancelreturn.is_null() {
            tv_copy(&*cancelreturn, &mut *rettv);
        }

        g_st().need_wait_return = false;
        g_st().msg_didout = false;
    }
}

/// `"wildtrigger()"` function.
pub fn f_wildtrigger(_argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if (g_st().state_global & MODE_CMDLINE) == 0
        || char_avail()
        || g_st().wild_menu_showing != 0
        || cmdline_pum_active()
    {
        return;
    }

    let cmd_type = get_cmdline_type();

    if cmd_type == b':' as i32 || cmd_type == b'/' as i32 || cmd_type == b'?' as i32 {
        let mut key_string: [u8; 4] = [K_SPECIAL, KS_EXTRA, KE_WILD, NUL as u8];

        ins_typebuf(
            key_string.as_mut_ptr().cast::<c_char>(),
            REMAP_NONE,
            0,
            true,
            false,
        );
    }
}
