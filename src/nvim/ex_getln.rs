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
    DOCMD_NOWAIT, EVENT_CMDLINECHANGED, EVENT_CMDLINEENTER, EVENT_CMDLINELEAVE,
    EVENT_CMDLINELEAVEPRE, EVENT_CURSORMOVEDC, apply_autocmds, block_autocmds, event_T, has_event,
    unblock_autocmds,
};
use crate::buffer::bt_cmdwin;
use crate::buffer::{
    DOBUF_FIRST, DOBUF_GOTO, MAXLNUM, buf_T, buf_clear, buf_valid, buflist_findnr, do_buffer,
    kUIMessages,
};
use crate::charset::{
    NUL, ascii_isalpha, ascii_isdigit, ascii_isspace, ascii_iswhite, is_special, skipwhite,
    vim_isIDc, vim_isprintc, vim_iswordc, vim_str2nr,
};
use crate::cmdexpand::{
    CmdlineColorChunk, CmdlineInfo, ColoredCmdline, Ctrl_N, Ctrl_P, EXPAND_NOTHING,
    EXPAND_UNSUCCESSFUL, ExpandCleanup, ExpandInit, ExpandOne, SEARCH_NOOF, SEARCH_OPT,
    SEARCH_PEEK, SEARCH_START, WILD_ALL, WILD_APPLY, WILD_BUFLASTUSED, WILD_CANCEL,
    WILD_EXPAND_KEEP, WILD_FREE, WILD_FUNC_TRIGGER, WILD_LONGEST, WILD_MAY_EXPAND_PATTERN,
    WILD_NEXT, WILD_NO_BEEP, WILD_NOINSERT, WILD_NOSELECT, WILD_PAGEDOWN, WILD_PAGEUP, WILD_PREV,
    WILD_PUM_WANT, XP_BS_NONE, clear_cmdline_orig, cmdline_pum_active, cmdline_pum_cleanup,
    cmdline_pum_remove, kOptBoFlagWildmode, kUICmdline, nextwild, set_expand_context, showmatches,
    wildmenu_cleanup, wildmenu_process_key, wildmenu_translate_key,
};
use crate::cmdhist::{
    HIST_INVALID, HIST_SEARCH, add_to_history, get_hisidx, get_hislen, get_histentry,
    hist_char2type, init_history,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, kCtxKeepDirs};
use crate::cursor::{clearpos, coladvance, equalpos, gchar_cursor, get_cursor_pos_ptr, lt, pos_T};
use crate::digraph::{do_digraph, get_digraph, kBoolVarFalse, kBoolVarTrue};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, redraw_all_later, redraw_custom_title_later,
    redraw_later, redraw_statuslines, set_must_redraw, status_redraw_all, status_redraw_curbuf,
    update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Callback, EvalFuncData, TypvalVval, VarType, callback_free, kCallbackNone,
    tv_check_for_opt_number_arg, tv_check_for_string_arg, tv_clear, tv_copy, tv_dict_add_bool,
    tv_dict_add_nr, tv_dict_add_str, tv_dict_find, tv_dict_get_callback, tv_dict_get_number,
    tv_dict_get_string_buf_chk, tv_dict_set_keys_readonly, tv_get_number, tv_get_number_chk,
    tv_get_string, tv_get_string_buf_chk, tv_get_string_chk, tv_list_first, tv_list_free,
    tv_list_last, tv_list_len, typval_T, varnumber_T,
};
use crate::eval::vars::{get_globvar_dict, heredoc_get, set_vim_var_char};
use crate::eval::window::viewstate_T;
use crate::eval::{callback_call, eval_has_provider, get_echo_hl_id, get_v_event, restore_v_event};
use crate::event::r#loop::loop_schedule_deferred;
use crate::event::multiqueue::Event;
use crate::ex_cmds::{B_IMODE_LMAP, Ctrl_C, Ctrl_E, Ctrl_Y, kOptInccommand, rename_buffer};
use crate::ex_docmd::{
    ADDR_LINES, CMOD_NOSWAPFILE, EX_PREVIEW, EX_RANGE, cmdmod_T, do_cmdline, exarg_T, execute_cmd,
    expr_map_locked, parse_cmd_address, parse_cmdline, parse_command_modifiers, set_no_hlsearch,
    skip_range, undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::extmark::extmark_clear;
use crate::globals::*;
use crate::highlight_group::{syn_id2attr, syn_name2id};
use crate::input::{
    beep_flush, char_avail, getcmdkeycmd, ins_typebuf, map_execute_lua, plain_vgetc, stuff_empty,
    stuffReadbuffSpecial, stuffcharReadbuff, vgetc, vpeekc, vpeekc_any, vungetc,
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
use crate::map::{MHPutStatus, Set, mh_get, mh_put, ptr_t};
use crate::mapping::{REMAP_NONE, check_abbr, map_to_exists_mode};
use crate::mark::setpcmark;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_cptr2char_adv, mb_get_class, mb_off_next, mb_prevptr, mb_tolower, ptr2cells, utf_char2bytes,
    utf_char2len, utf_head_off, utf_iscomposing_first, utf_ptr2cells, utf_ptr2char,
    utf8len_tab_zero, utfc_ptr2len, vimconv_T,
};
use crate::memline::{decl, incl};
use crate::memory::{
    Arena, arena_alloc, arena_finish, arena_mem_free, xfree, xmalloc, xmallocz, xmemdupz, xstrdup,
};
use crate::message::{
    emsg, msg, msg_check, msg_clr_eos, msg_cursor_goto, msg_grid_validate, msg_outtrans_len,
    msg_putchar, msg_puts_hl, msg_puts_len, msg_start, msg_starthere, sb_text_end_cmdline,
    sb_text_restart_cmdline, sb_text_start_cmdline, smsg,
};
use crate::mouse::{MSCR_DOWN, MSCR_LEFT, MSCR_RIGHT, MSCR_UP, cmdline_mousescroll, setmouse};
use crate::r#move::{
    changed_cline_bef_curs, restore_viewstate, save_viewstate, update_topline, validate_cursor,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
#[cfg(unix)]
use crate::option::csh_like_shell;
use crate::option::{
    OptInt, set_iminsert_global, set_imsearch_global, set_option_direct, string_to_key,
};
use crate::option_vars::*;
use crate::optionstr::optset_T;
use crate::os::env::{expand_T, home_replace_save};
use crate::os::input::line_breakcheck;
use crate::path::vim_ispathsep;
use crate::popupmenu::pum_check_clear;
use crate::profile::{profile_setlimit, proftime_T};
use crate::regexp::{
    Ctrl_H, Ctrl_V, MAGIC_ALL, MAGIC_ON, MAXCOL, NUMBUFLEN, magic_T, skip_regexp_ex,
};
use crate::register::{
    cmdline_paste_reg, get_expr_line, get_expr_register, get_spec_reg, is_literal_register,
    valid_yank_reg,
};
use crate::runtime::SID_NONE;
use crate::search::{
    BACKWARD, FORWARD, SearchOffset, do_search, parse_search_pattern_offset, pat_has_uppercase,
    restore_last_search_pattern, restore_search_patterns, save_last_search_pattern,
    save_search_patterns, searchit, searchit_arg_T,
};
use crate::state::{
    MODE_CMDLINE, MODE_LANGMAP, VimState, may_trigger_modechanged, may_trigger_safestate,
    save_v_event_T, state_enter, state_handle_k_event,
};
use crate::strings::{
    gettext, strcmp, strcpy, strlen, strncmp, strrchr, vim_strchr, vim_strnicmp,
    vim_strsave_escaped, xstrnsave,
};
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_call_cmdline_block_append, ui_call_cmdline_block_hide,
    ui_call_cmdline_block_show, ui_call_cmdline_hide, ui_call_cmdline_pos, ui_call_cmdline_show,
    ui_call_cmdline_special_char, ui_cursor_shape, ui_flush, ui_has, vim_beep,
};
use crate::undo::{UndoCheckpoint, u_checkpoint, u_rollback, u_sync};
use crate::usercmd::{cmdcomplete_type_to_str, parse_compl_arg};
use crate::viml::parser::expressions::{
    ExprAST, kExprFlagsDisallowEOC, viml_pexpr_free_ast, viml_pexpr_parse,
};
use crate::viml::parser::parser::{
    CONV_NONE, ParserHighlight, ParserInputReader, ParserLine, ParserPosition, ParserState,
    parser_simple_get_line, viml_parser_destroy,
};
use crate::window::{
    WSP_BOT, close_windows, global_stl_height, lastwin_nofloating,
    may_trigger_win_scrolled_resized, win_T, win_enter, win_size_restore, win_size_save, win_split,
};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

#[cfg(windows)]
use crate::charset::vim_isfilec;

pub type hlf_T = c_int;
pub type disptick_T = u64;
use crate::eval::funcs::{RE_SEARCH, SEARCH_COL, SEARCH_KEEP};

pub(crate) const Ctrl_A: c_int = 1;
pub(crate) const Ctrl_B: c_int = 2;
pub(crate) const Ctrl_D: c_int = 4;
pub(crate) const Ctrl_F: c_int = 6;
pub(crate) const Ctrl_G: c_int = 7;
pub(crate) const Ctrl_K: c_int = 11;
pub(crate) const Ctrl_L: c_int = 12;
pub(crate) const Ctrl_O: c_int = 15;
pub(crate) const Ctrl_Q: c_int = 17;
pub(crate) const Ctrl_R: c_int = 18;
pub(crate) const Ctrl_T: c_int = 20;
pub(crate) const Ctrl_U: c_int = 21;
pub(crate) const Ctrl_W: c_int = 23;
pub(crate) const Ctrl_Z: c_int = 26;
pub(crate) const Ctrl_BSL: c_int = 28;
pub(crate) const Ctrl_RSB: c_int = 29;
pub(crate) const Ctrl_HAT: c_int = 30;
pub(crate) const Ctrl__: c_int = 31;

pub(crate) const KS_SELECT: c_int = 245;
pub(crate) const K_SELECT: c_int = termcap2key(KS_SELECT, KE_FILLER as c_int);
pub(crate) const K_EVENT: c_int = termcap2key(KS_EXTRA as c_int, KE_EVENT as c_int);
pub(crate) const K_LUA: c_int = termcap2key(KS_EXTRA as c_int, KE_LUA as c_int);
const K_WILD: c_int = termcap2key(KS_EXTRA as c_int, KE_WILD as c_int);

pub(crate) const ABBR_OFF: c_int = 0x100;

pub(crate) const B_IMODE_USE_INSERT: OptInt = -1;
pub(crate) const B_IMODE_NONE: OptInt = 0;

const kOptWimFlagFull: u8 = 0x01;
const kOptWimFlagLongest: u8 = 0x02;
const kOptWimFlagList: u8 = 0x04;
const kOptWimFlagLastused: u8 = 0x08;
pub(crate) const kOptWimFlagNoselect: u8 = 0x10;
pub(crate) const kOptWimFlagNoinsert: u8 = 0x20;

pub(crate) const kOptBoFlagError: c_uint = 0x40;

const kCpoEsc: c_int = b'x' as c_int;

const kCmdRedrawNone: c_int = 0;
const kCmdRedrawPos: c_int = 1;
const kCmdRedrawAll: c_int = 2;

pub const VSE_NONE: c_int = 0;
#[cfg(unix)]
pub const VSE_SHELL: c_int = 1;
pub const VSE_BUFFER: c_int = 2;

const CMDLINE_NOT_CHANGED: c_int = 1;
const CMDLINE_CHANGED: c_int = 2;
const GOTO_NORMAL_MODE: c_int = 3;
const PROCESS_NEXT_KEY: c_int = 4;

const MAX_CB_ERRORS: c_int = 1;

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
#[derive(Clone, Copy)]
struct incsearch_state_T {
    search_start: pos_T,
    save_cursor: pos_T,
    winid: handle_T,
    init_viewstate: viewstate_T,
    old_viewstate: viewstate_T,
    match_start: pos_T,
    match_end: pos_T,
    did_incsearch: bool,
    incsearch_postponed: bool,
}

#[repr(C)]
struct CommandLineState {
    state: VimState,
    firstc: c_int,
    count: c_int,
    indent: c_int,
    c: c_int,
    gotesc: bool,
    do_abbr: bool,
    lookfor: *mut c_char,
    lookforlen: c_int,
    hiscnt: c_int,
    save_hiscnt: c_int,
    histype: c_int,
    is_state: incsearch_state_T,
    did_wild_list: bool,
    wim_index: c_int,
    save_msg_scroll: c_int,
    save_State: c_int,
    prev_cmdpos: c_int,
    prev_cmdbuff: *mut c_char,
    save_p_icm: *mut c_char,
    skip_pum_redraw: bool,
    some_key_typed: bool,
    ignore_drag_release: bool,
    break_ctrl_c: bool,
    xpc: expand_T,
    b_im_ptr: *mut OptInt,
    b_im_ptr_buf: *mut buf_T,
    cmdline_type: c_int,
    event_cmdlineleavepre_triggered: bool,
    did_hist_navigate: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CpBufInfo {
    buf: *mut buf_T,
    save_b_p_ma: c_int,
    save_b_changed: c_int,
    save_b_op_start: pos_T,
    save_b_op_end: pos_T,
    undo_checkpoint: UndoCheckpoint,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CpWinInfo {
    win: *mut win_T,
    save_w_cursor: pos_T,
    save_viewstate: viewstate_T,
    save_w_p_cul: c_int,
    save_w_p_cuc: c_int,
}

struct CpInfo {
    win_info: Vec<CpWinInfo>,
    buf_info: Vec<CpBufInfo>,
    save_hls: bool,
    save_cmdmod: cmdmod_T,
    save_view: Vec<c_int>,
}

#[repr(C)]
struct CmdwinOpenArgs {
    firstc: c_int,
    content: *mut c_char,
    pos: c_int,
}

/// Last value of `prompt_id`, incremented when doing new prompt.
static mut last_prompt_id: c_uint = 0;

/// Recursive invocation depth of `command_line_enter`.
static mut cmdline_level: c_int = 0;

/// The current `cmdline_info`.
static mut ccline: CmdlineInfo = CmdlineInfo::new();

/// Position set by `set_cmdline_pos()`.
static mut new_cmdpos: c_int = 0;

/// Currently displayed block of context.
static mut cmdline_block: Array = Array {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};

/// Flag for `command_line_handle_key` to ignore `<C-c>`.
static mut getln_interrupted_highlight: bool = false;

/// Key value of `'cedit'` option.
static mut cedit_key: c_int = -1;

static mut cmdpreview_bufnr: handle_T = 0;
static mut cmdpreview_ns: c_int = 0;

static mut prev_prompt_id: c_uint = c_uint::MAX;
static mut prev_prompt_errors: c_int = 0;

use crate::api::private::helpers::array_add;

unsafe fn trigger_cmd_autocmd(typechar: c_int, evt: event_T) {
    unsafe {
        let mut typestr: [c_char; 2] = [typechar as c_char, NUL];
        apply_autocmds(
            evt,
            typestr.as_mut_ptr(),
            typestr.as_mut_ptr(),
            false,
            curbuf,
        );
    }
}

unsafe fn init_incsearch_state(s: *mut incsearch_state_T) {
    unsafe {
        let s = &mut *s;
        s.winid = (*curwin).handle;
        s.match_start = (*curwin).w_cursor;
        s.did_incsearch = false;
        s.incsearch_postponed = false;
        clearpos(&mut s.match_end);
        s.save_cursor = (*curwin).w_cursor;
        s.search_start = (*curwin).w_cursor;
        save_viewstate(curwin, &raw mut s.init_viewstate);
        save_viewstate(curwin, &raw mut s.old_viewstate);
    }
}

unsafe fn set_search_match(t: *mut pos_T) {
    unsafe {
        (*t).lnum += Search.match_lines;
        (*t).col = Search.match_endcol;
        if (*t).lnum > (*curbuf).b_ml.ml_line_count {
            (*t).lnum = (*curbuf).b_ml.ml_line_count;
            coladvance(curwin, MAXCOL);
        }
    }
}

/// Parses the `:[range]s/foo` like commands and returns details needed for
/// incsearch and wildmenu completion.
/// Returns true if pattern is valid.
/// Sets `skiplen`, `patlen`, `Search.first_line`, and `Search.last_line` and `magic` if overruled.
pub unsafe fn parse_pattern_and_range(
    incsearch_start: *mut pos_T,
    search_delim: *mut c_int,
    skiplen: *mut c_int,
    patlen: *mut c_int,
    is_magic: *mut bool,
) -> bool {
    unsafe {
        let mut delim_optional = false;
        let mut dummy: *const c_char = ptr::null();
        let mut magic: magic_T = 0;

        *skiplen = 0;
        *patlen = ccline.cmdlen;

        // Default range
        Search.first_line = 0;
        Search.last_line = MAXLNUM;

        let mut ea: exarg_T = std::mem::zeroed();
        ea.line1 = 1;
        ea.line2 = 1;
        ea.cmd = ccline.cmdbuff;
        ea.addr_type = ADDR_LINES;

        let mut dummy_cmdmod: cmdmod_T = std::mem::zeroed();
        // Skip over command modifiers
        parse_command_modifiers(&raw mut ea, &raw mut dummy, &raw mut dummy_cmdmod, true);

        // Skip over the range to find the command.
        let cmd = skip_range(ea.cmd, ptr::null_mut());
        if vim_strchr(c"sgvlu".as_ptr(), (*cmd as u8) as c_int).is_null() {
            return false;
        }

        // Skip over command name to find pattern separator
        let mut p = cmd;
        while ascii_isalpha(*p as c_int) {
            p = p.add(1);
        }
        if *skipwhite(p) == NUL {
            return false;
        }

        let cmd_len = p.offset_from(cmd) as usize;
        if strncmp(cmd, c"substitute".as_ptr(), cmd_len) == 0
            || strncmp(cmd, c"smagic".as_ptr(), cmd_len) == 0
            || strncmp(cmd, c"snomagic".as_ptr(), cmd_len.max(3)) == 0
            || strncmp(cmd, c"vglobal".as_ptr(), cmd_len) == 0
        {
            if *cmd == b's' as c_char && *cmd.add(1) == b'm' as c_char {
                *is_magic = true;
            } else if *cmd == b's' as c_char && *cmd.add(1) == b'n' as c_char {
                *is_magic = false;
            }
        } else if strncmp(cmd, c"sort".as_ptr(), cmd_len.max(3)) == 0
            || strncmp(cmd, c"uniq".as_ptr(), cmd_len.max(3)) == 0
        {
            // skip over ! and flags
            if *p == b'!' as c_char {
                p = skipwhite(p.add(1));
            }
            loop {
                p = skipwhite(p);
                if !ascii_isalpha(*p as c_int) {
                    break;
                }
                p = p.add(1);
            }
            if *p == NUL {
                return false;
            }
        } else if strncmp(cmd, c"vimgrep".as_ptr(), cmd_len.max(3)) == 0
            || strncmp(cmd, c"vimgrepadd".as_ptr(), cmd_len.max(8)) == 0
            || strncmp(cmd, c"lvimgrep".as_ptr(), cmd_len.max(2)) == 0
            || strncmp(cmd, c"lvimgrepadd".as_ptr(), cmd_len.max(9)) == 0
            || strncmp(cmd, c"global".as_ptr(), cmd_len) == 0
        {
            // skip optional "!"
            if *p == b'!' as c_char {
                p = p.add(1);
                if *skipwhite(p) == NUL {
                    return false;
                }
            }
            if *cmd != b'g' as c_char {
                delim_optional = true;
            }
        } else {
            return false;
        }

        p = skipwhite(p);
        let delim = if delim_optional && vim_isIDc((*p as u8) as c_int) {
            b' ' as c_int
        } else {
            let d = *p as c_int;
            p = p.add(1);
            d
        };
        *search_delim = delim;

        let end = skip_regexp_ex(
            p,
            delim,
            c_int::from(*is_magic),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut magic,
        );
        let use_last_pat = end == p && *end as c_int == delim;

        if end == p && !use_last_pat {
            return false;
        }

        // Skip if the pattern matches everything (e.g., for 'hlsearch')
        if !use_last_pat {
            let c = *end;
            *end = NUL;
            let empty = empty_pattern_magic(p, end.offset_from(p) as usize, magic);
            *end = c;
            if empty {
                return false;
            }
        }

        // Found a non-empty pattern or //
        *skiplen = p.offset_from(ccline.cmdbuff) as c_int;
        *patlen = end.offset_from(p) as c_int;

        // Parse the address range
        let save_cursor = (*curwin).w_cursor;
        (*curwin).w_cursor = *incsearch_start;

        parse_cmd_address(&raw mut ea, &raw mut dummy, true);

        if ea.addr_count > 0 {
            // Allow for reverse match.
            Search.first_line = ea.line2.min(ea.line1);
            Search.last_line = ea.line2.max(ea.line1);
        } else if *cmd == b's' as c_char && *cmd.add(1) != b'o' as c_char {
            // :s defaults to the current line
            Search.first_line = (*curwin).w_cursor.lnum;
            Search.last_line = (*curwin).w_cursor.lnum;
        }

        (*curwin).w_cursor = save_cursor;
        true
    }
}

/// Return true when `'incsearch'` highlighting is to be done.
/// Sets `Search.first_line` and `Search.last_line` to the address range.
/// May change the last search pattern.
unsafe fn do_incsearch_highlighting(
    firstc: c_int,
    search_delim: *mut c_int,
    is_state: *mut incsearch_state_T,
    skiplen: *mut c_int,
    patlen: *mut c_int,
    magic: *mut bool,
) -> bool {
    unsafe {
        *skiplen = 0;
        *patlen = ccline.cmdlen;

        if p_is == 0 || cmd_silent {
            return false;
        }

        // By default search all lines
        Search.first_line = 0;
        Search.last_line = MAXLNUM;

        if firstc == b'/' as c_int || firstc == b'?' as c_int {
            *search_delim = firstc;
            return true;
        }

        if firstc != b':' as c_int {
            return false;
        }

        emsg_off += 1;
        let retval = parse_pattern_and_range(
            &raw mut (*is_state).search_start,
            search_delim,
            skiplen,
            patlen,
            magic,
        );
        emsg_off -= 1;

        retval
    }
}

/// May do `'incsearch'` highlighting if desired.
unsafe fn may_do_incsearch_highlighting(firstc: c_int, count: c_int, s: *mut incsearch_state_T) {
    unsafe {
        let mut skiplen: c_int = 0;
        let mut patlen: c_int = 0;
        let mut search_delim: c_int = 0;
        let mut magic: bool = p_magic != 0;

        // Parsing range may already set the last search pattern.
        // NOTE: must call restore_last_search_pattern() before returning!
        save_last_search_pattern();

        if !do_incsearch_highlighting(
            firstc,
            &raw mut search_delim,
            s,
            &raw mut skiplen,
            &raw mut patlen,
            &raw mut magic,
        ) {
            restore_last_search_pattern();
            finish_incsearch_highlighting(false, s, true);
            return;
        }

        // if there is a character waiting, search and redraw later
        if char_avail() {
            restore_last_search_pattern();
            (*s).incsearch_postponed = true;
            return;
        }
        (*s).incsearch_postponed = false;

        // Use the previous pattern for ":s//".
        let mut next_char = *ccline.cmdbuff.add((skiplen + patlen) as usize);
        let use_last_pat =
            patlen == 0 && skiplen > 0 && *ccline.cmdbuff.add((skiplen - 1) as usize) == next_char;

        if patlen != 0 || use_last_pat {
            ui_busy_start();
            ui_flush();
        }

        if Search.first_line == 0 {
            // start at the original cursor position
            (*curwin).w_cursor = (*s).search_start;
        } else if Search.first_line > (*curbuf).b_ml.ml_line_count {
            // start after the last line
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
            (*curwin).w_cursor.col = MAXCOL;
        } else {
            // start at the first line in the range
            (*curwin).w_cursor.lnum = Search.first_line;
            (*curwin).w_cursor.col = 0;
        }

        let mut found: c_int = 0; // do_search() result

        if patlen != 0 || use_last_pat {
            let mut search_flags = SEARCH_OPT + SEARCH_NOOF + SEARCH_PEEK;
            if p_hls == 0 {
                search_flags += SEARCH_KEEP;
            }
            if Search.first_line != 0 {
                search_flags += SEARCH_START;
            }
            // Set the time limit to half a second.
            let mut tm: proftime_T = profile_setlimit(500);
            let mut sia = searchit_arg_T {
                sa_stop_lnum: 0,
                sa_tm: &raw mut tm,
                sa_timed_out: 0,
                sa_wrapped: 0,
            };
            *ccline.cmdbuff.add((skiplen + patlen) as usize) = NUL;
            emsg_off += 1; // So it doesn't beep if bad expr
            found = do_search(
                ptr::null_mut(),
                if firstc == b':' as c_int {
                    b'/' as c_int
                } else {
                    firstc
                },
                search_delim,
                ccline.cmdbuff.add(skiplen as usize),
                patlen as usize,
                count,
                search_flags,
                magic,
                &raw mut sia,
            );
            emsg_off -= 1;
            *ccline.cmdbuff.add((skiplen + patlen) as usize) = next_char;
            if (*curwin).w_cursor.lnum < Search.first_line
                || (*curwin).w_cursor.lnum > Search.last_line
            {
                // match outside of address range
                found = 0;
                (*curwin).w_cursor = (*s).search_start;
            }

            // if interrupted while searching, behave like it failed
            if got_int {
                vpeekc(); // remove <C-C> from input stream
                got_int = false; // don't abandon the command line
                found = 0;
            } else if char_avail() {
                // cancelled searching because a char was typed
                (*s).incsearch_postponed = true;
            }
            ui_busy_stop();
        } else {
            set_no_hlsearch(true); // turn off previous highlight
            redraw_all_later(UPD_SOME_VALID);
        }

        Search.hl_match = found != 0; // add or remove search match position

        // first restore the old curwin values, so the screen is
        // positioned in the same way as the actual search command
        restore_viewstate(curwin, &raw mut (*s).old_viewstate);
        changed_cline_bef_curs(curwin);
        update_topline(curwin);

        let mut end_pos = (*curwin).w_cursor;
        if found != 0 {
            (*s).match_start = (*curwin).w_cursor;
            set_search_match(&raw mut (*curwin).w_cursor);
            validate_cursor(curwin);
            (*s).match_end = (*curwin).w_cursor;
            (*curwin).w_cursor = end_pos;
            end_pos = (*s).match_end;
        }

        // Disable 'hlsearch' highlighting if the pattern matches
        // everything. Avoids a flash when typing "foo\|".
        if !use_last_pat {
            next_char = *ccline.cmdbuff.add((skiplen + patlen) as usize);
            *ccline.cmdbuff.add((skiplen + patlen) as usize) = NUL;
            if empty_pattern(
                ccline.cmdbuff.add(skiplen as usize),
                patlen as usize,
                search_delim,
            ) && !Search.no_hlsearch
            {
                redraw_all_later(UPD_SOME_VALID);
                set_no_hlsearch(true);
            }
            *ccline.cmdbuff.add((skiplen + patlen) as usize) = next_char;
        }

        validate_cursor(curwin);

        // May redraw the status line to show the cursor position.
        if p_ru != 0 && ((*curwin).w_status_height > 0 || global_stl_height() > 0) {
            (*curwin).w_redr_status = true;
        }

        redraw_later(curwin, UPD_SOME_VALID);
        update_screen();
        Search.hl_match = false;
        restore_last_search_pattern();

        // Leave it at the end to make CTRL-R CTRL-W work.  But not when beyond the
        // end of the pattern, e.g. for ":s/pat/".
        if *ccline.cmdbuff.add((skiplen + patlen) as usize) != NUL {
            (*curwin).w_cursor = (*s).search_start;
        } else if found != 0 {
            (*curwin).w_cursor = end_pos;
            (*curwin).w_valid_cursor = end_pos; // mark as valid for cmdline_show redraw
        }

        msg_starthere();
        redrawcmdline();
        (*s).did_incsearch = true;
        // Fire WinScrolled/WinResized last, after all state is finalized: the
        // autocmd may mutate curwin or ccline via arbitrary user code.
        may_trigger_win_scrolled_resized();
    }
}

/// When CTRL-L typed: add character from the match to the pattern.
/// May set `*c` to the added character.
/// Return `OK` when calling `command_line_not_changed`.
unsafe fn may_add_char_to_search(firstc: c_int, c: *mut c_int, s: *mut incsearch_state_T) -> c_int {
    unsafe {
        let mut skiplen: c_int = 0;
        let mut patlen: c_int = 0;
        let mut search_delim: c_int = 0;
        let mut magic: bool = p_magic != 0;

        // Parsing range may already set the last search pattern.
        // NOTE: must call restore_last_search_pattern() before returning!
        save_last_search_pattern();

        // Add a character from under the cursor for 'incsearch'
        if !do_incsearch_highlighting(
            firstc,
            &raw mut search_delim,
            s,
            &raw mut skiplen,
            &raw mut patlen,
            &raw mut magic,
        ) {
            restore_last_search_pattern();
            return FAIL;
        }
        restore_last_search_pattern();

        if (*s).did_incsearch {
            (*curwin).w_cursor = (*s).match_end;
            *c = gchar_cursor();
            if *c != NUL as c_int {
                // If 'ignorecase' and 'smartcase' are set and the
                // command line has no uppercase characters, convert
                // the character to lowercase
                if p_ic != 0
                    && p_scs != 0
                    && !pat_has_uppercase(ccline.cmdbuff.add(skiplen as usize))
                {
                    *c = mb_tolower(*c);
                }
                let special_chars = if p_magic != 0 {
                    c"\\~^$.*[".as_ptr()
                } else {
                    c"\\^$".as_ptr()
                };
                if *c == search_delim || !vim_strchr(special_chars, *c).is_null() {
                    // put a backslash before special characters
                    stuffcharReadbuff(*c);
                    *c = b'\\' as c_int;
                }
                // add any composing characters
                if utf_char2len(*c) != utfc_ptr2len(get_cursor_pos_ptr()) {
                    let save_c = *c;
                    while utf_char2len(*c) != utfc_ptr2len(get_cursor_pos_ptr()) {
                        (*curwin).w_cursor.col += utf_char2len(*c);
                        *c = gchar_cursor();
                        stuffcharReadbuff(*c);
                    }
                    *c = save_c;
                }
                return FAIL;
            }
        }
        OK
    }
}

unsafe fn finish_incsearch_highlighting(
    gotesc: bool,
    s: *mut incsearch_state_T,
    call_update_screen: bool,
) {
    unsafe {
        if !(*s).did_incsearch {
            return;
        }

        (*s).did_incsearch = false;
        if gotesc {
            (*curwin).w_cursor = (*s).save_cursor;
        } else {
            if !equalpos((*s).save_cursor, (*s).search_start) {
                // put the '" mark at the original position
                (*curwin).w_cursor = (*s).save_cursor;
                setpcmark();
            }
            (*curwin).w_cursor = (*s).search_start;
        }
        restore_viewstate(curwin, &raw mut (*s).old_viewstate);
        Search.hl_match = false;

        // by default search all lines
        Search.first_line = 0;
        Search.last_line = MAXLNUM;

        validate_cursor(curwin); // needed for TAB
        status_redraw_all();
        redraw_all_later(UPD_SOME_VALID);
        if call_update_screen {
            update_screen();
            may_trigger_win_scrolled_resized();
        }
    }
}

/// Initialize the current command-line info.
unsafe fn init_ccline(firstc: c_int, indent: c_int) {
    unsafe {
        ccline.overstrike = 0; // always start in insert mode

        debug_assert!(indent >= 0);

        // set some variables for redrawcmd()
        ccline.cmdfirstc = if firstc == b'@' as c_int { 0 } else { firstc };
        ccline.cmdindent = if firstc > 0 { indent } else { 0 };

        // alloc initial ccline.cmdbuff
        alloc_cmdbuff(indent + 50);
        ccline.cmdpos = 0;
        ccline.cmdlen = 0;
        *ccline.cmdbuff = NUL;

        ccline.last_colors = ColoredCmdline::new();
        sb_text_start_cmdline();

        // autoindent for :insert and :append
        if firstc <= 0 {
            ptr::write_bytes(ccline.cmdbuff.cast::<u8>(), b' ', indent as usize);
            *ccline.cmdbuff.add(indent as usize) = NUL;
            ccline.cmdpos = indent;
            ccline.cmdspos = indent;
            ccline.cmdlen = indent;
        }
    }
}

unsafe fn ui_ext_cmdline_hide(abort: bool) {
    unsafe {
        if ui_has(kUICmdline) {
            cmdline_was_last_drawn = false;
            ccline.redraw_state = kCmdRedrawNone;
            ui_call_cmdline_hide(ccline.level as Integer, abort);
        }
    }
}

/// Internal entry point for cmdline mode.
unsafe fn command_line_enter(
    firstc: c_int,
    count: c_int,
    indent: c_int,
    clear_ccline: bool,
) -> *mut u8 {
    unsafe {
        cmdline_level += 1;

        let save_cmdpreview = cmdpreview;
        cmdpreview = false;
        let mut state: CommandLineState = std::mem::zeroed();
        state.firstc = firstc;
        state.count = count;
        state.indent = indent;
        state.save_msg_scroll = msg_scroll;
        state.save_State = State;
        state.prev_cmdpos = -1;
        state.ignore_drag_release = true;
        let s = &raw mut state;
        (*s).save_p_icm = xstrdup(p_icm);
        init_incsearch_state(&raw mut (*s).is_state);
        let mut save_ccline = CmdlineInfo::new();
        let mut did_save_ccline = false;

        let cc = &raw mut ccline;
        if !(*cc).cmdbuff.is_null() {
            debug_assert!(clear_ccline);
            save_cmdline(&raw mut save_ccline);
            did_save_ccline = true;
        } else if clear_ccline {
            *cc = CmdlineInfo::new();
        }

        if (*s).firstc == -1 {
            (*s).firstc = NUL as c_int;
            (*s).break_ctrl_c = true;
        }

        init_ccline((*s).firstc, (*s).indent);
        debug_assert!(!ccline.cmdbuff.is_null());
        ccline.prompt_id = last_prompt_id;
        last_prompt_id = last_prompt_id.wrapping_add(1);
        ccline.level = cmdline_level;

        if cmdline_level != 50 {
            ExpandInit(&raw mut (*s).xpc);
            ccline.xpc = &raw mut (*s).xpc;
            clear_cmdline_orig();

            cmdmsg_rl = (*curwin).w_onebuf_opt.wo_rl != 0
                && *(*curwin).w_onebuf_opt.wo_rlc == b's' as c_char
                && ((*s).firstc == b'/' as c_int || (*s).firstc == b'?' as c_int);

            msg_grid_validate();

            redir_off = true; // don't redirect the typed command
            if !cmd_silent {
                gotocmdline(true);
                redrawcmdprompt(); // draw prompt or indent
                ccline.cmdspos = cmd_startcol();
            }
            (*s).xpc.xp_context = EXPAND_NOTHING;
            (*s).xpc.xp_backslash = XP_BS_NONE;
            #[cfg(not(windows))]
            {
                (*s).xpc.xp_shell = false;
            }

            if ccline.input_fn != 0 {
                (*s).xpc.xp_context = ccline.xp_context;
                (*s).xpc.xp_pattern = ccline.cmdbuff;
                (*s).xpc.xp_arg = ccline.xp_arg;
            }

            msg_scroll = 0;

            State = MODE_CMDLINE;

            if (*s).firstc == b'/' as c_int
                || (*s).firstc == b'?' as c_int
                || (*s).firstc == b'@' as c_int
            {
                if (*curbuf).b_p_imsearch == B_IMODE_USE_INSERT {
                    (*s).b_im_ptr = &raw mut (*curbuf).b_p_iminsert;
                } else {
                    (*s).b_im_ptr = &raw mut (*curbuf).b_p_imsearch;
                }
                (*s).b_im_ptr_buf = curbuf;
                if *(*s).b_im_ptr == B_IMODE_LMAP {
                    State |= MODE_LANGMAP;
                }
            }

            setmouse();

            (*s).cmdline_type = if firstc > 0 { firstc } else { b'-' as c_int };
            let mut err = error_init();
            let mut firstcbuf: [c_char; 2] = [(*s).cmdline_type as c_char, 0];

            if has_event(EVENT_CMDLINEENTER) {
                let mut save_v_event: save_v_event_T = std::mem::zeroed();
                let dict = get_v_event(&raw mut save_v_event);

                tv_dict_add_str(dict, c"cmdtype".as_ptr(), 7, firstcbuf.as_ptr());
                tv_dict_add_nr(dict, c"cmdlevel".as_ptr(), 8, ccline.level as varnumber_T);
                tv_dict_set_keys_readonly(dict);
                let mut tstate: TryState = std::mem::zeroed();
                try_enter(&raw mut tstate);
                apply_autocmds(
                    EVENT_CMDLINEENTER,
                    firstcbuf.as_mut_ptr(),
                    firstcbuf.as_mut_ptr(),
                    false,
                    curbuf,
                );
                restore_v_event(dict, &raw mut save_v_event);
                try_leave(&raw const tstate, &raw mut err);

                if error_set(&err) {
                    if !ui_has(kUIMessages) {
                        msg_putchar(b'\n' as c_int);
                    }
                    msg_scroll = 1;
                    msg_puts_hl(err.msg, HLF_E, true);
                    api_clear_error(&raw mut err);
                    redrawcmd();
                }
                err = error_init();
            }
            may_trigger_modechanged();

            init_history();
            (*s).hiscnt = get_hislen(); // set hiscnt to impossible history value
            (*s).histype = hist_char2type((*s).firstc);
            do_digraph(-1); // init digraph typeahead

            if did_emsg != 0 {
                redrawcmd();
            }

            if !cmd_silent {
                let mut found_one = false;

                let mut wp = firstwin;
                while !wp.is_null() {
                    if *p_stl != NUL
                        || *(*wp).w_onebuf_opt.wo_stl != NUL
                        || *p_wbr != NUL
                        || *(*wp).w_onebuf_opt.wo_wbr != NUL
                    {
                        (*wp).w_redr_status = true;
                        found_one = true;
                    }
                    wp = (*wp).w_next;
                }

                if *p_tal != NUL {
                    redraw_tabline = true;
                    found_one = true;
                }

                if redraw_custom_title_later() {
                    found_one = true;
                }

                if found_one {
                    redraw_statuslines();
                }
            }

            did_emsg = 0;
            got_int = false;
            (*s).state.check = Some(command_line_check);
            (*s).state.execute = Some(command_line_execute);

            state_enter(&raw mut (*s).state);

            if !(*s).event_cmdlineleavepre_triggered {
                set_vim_var_char((*s).c);
                trigger_cmd_autocmd((*s).cmdline_type, EVENT_CMDLINELEAVEPRE);
            }

            if has_event(EVENT_CMDLINELEAVE) {
                let mut save_v_event: save_v_event_T = std::mem::zeroed();
                let dict = get_v_event(&raw mut save_v_event);

                tv_dict_add_str(dict, c"cmdtype".as_ptr(), 7, firstcbuf.as_ptr());
                tv_dict_add_nr(dict, c"cmdlevel".as_ptr(), 8, ccline.level as varnumber_T);
                tv_dict_set_keys_readonly(dict);
                tv_dict_add_bool(
                    dict,
                    c"abort".as_ptr(),
                    5,
                    if (*s).gotesc {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                set_vim_var_char((*s).c);
                let mut tstate: TryState = std::mem::zeroed();
                try_enter(&raw mut tstate);
                apply_autocmds(
                    EVENT_CMDLINELEAVE,
                    firstcbuf.as_mut_ptr(),
                    firstcbuf.as_mut_ptr(),
                    false,
                    curbuf,
                );
                try_leave(&raw const tstate, &raw mut err);
                if tv_dict_get_number(dict, c"abort".as_ptr()) != 0 {
                    (*s).gotesc = true;
                }
                restore_v_event(dict, &raw mut save_v_event);
            }

            cmdmsg_rl = false;

            if cmdline_pum_active() {
                cmdline_pum_remove(false);
            } else {
                pum_check_clear();
            }
            wildmenu_cleanup(&raw mut ccline);
            (*s).did_wild_list = false;
            (*s).wim_index = 0;

            ExpandCleanup(&raw mut (*s).xpc);
            ccline.xpc = ptr::null_mut();
            clear_cmdline_orig();

            finish_incsearch_highlighting((*s).gotesc, &raw mut (*s).is_state, false);

            if !ccline.cmdbuff.is_null() {
                if (*s).histype != HIST_INVALID
                    && ccline.cmdlen != 0
                    && (*s).firstc != NUL as c_int
                    && ((*s).some_key_typed || (*s).histype == HIST_SEARCH)
                {
                    add_to_history(
                        (*s).histype,
                        ccline.cmdbuff,
                        ccline.cmdlen as usize,
                        true,
                        if (*s).histype == HIST_SEARCH {
                            (*s).firstc
                        } else {
                            NUL as c_int
                        },
                    );
                    if (*s).firstc == b':' as c_int {
                        xfree(new_last_cmdline.cast::<c_void>());
                        new_last_cmdline = xstrnsave(ccline.cmdbuff, ccline.cmdlen as usize);
                    }
                }

                if !(*s).gotesc && ccline.level == 1 {
                    atom_cmdline_set((*s).firstc, ccline.cmdbuff, ccline.cmdlen as usize);
                }

                if (*s).gotesc {
                    abandon_cmdline();
                }
            }

            msg_check();
            if p_ch == 0 && !ui_has(kUIMessages) {
                set_must_redraw(UPD_VALID);
            }
            msg_scroll = (*s).save_msg_scroll;
            redir_off = false;

            if error_set(&err) {
                if !ui_has(kUIMessages) {
                    msg_putchar(b'\n' as c_int);
                }
                emsg(err.msg);
                did_emsg = 0;
                api_clear_error(&raw mut err);
            }

            if (*s).some_key_typed && !error_set(&err) {
                need_wait_return = false;
            }

            set_option_direct(kOptInccommand, cstr_as_obj((*s).save_p_icm), 0, SID_NONE);
            State = (*s).save_State;
            if cmdpreview != save_cmdpreview {
                cmdpreview = save_cmdpreview;
                redraw_all_later(UPD_SOME_VALID);
            }
            may_trigger_modechanged();
            setmouse();
            sb_text_end_cmdline();
        } else {
            emsg(gettext(e_command_too_recursive.as_ptr()));
        }

        // theend:
        xfree((*s).save_p_icm.cast::<c_void>());
        xfree(ccline.last_colors.cmdbuff.cast::<c_void>());
        ccline.last_colors.cmdbuff = ptr::null_mut();
        let lc_colors = &raw mut ccline.last_colors.colors;
        *lc_colors = Vec::new();

        let p = ccline.cmdbuff;

        if ui_has(kUICmdline) {
            ui_ext_cmdline_hide((*s).gotesc);
        }
        if !cmd_silent {
            redraw_custom_title_later();
            status_redraw_all();
        }

        cmdline_level -= 1;

        if did_save_ccline {
            restore_cmdline(&raw mut save_ccline);
        } else {
            ccline.cmdbuff = ptr::null_mut();
        }

        xfree((*s).prev_cmdbuff.cast::<c_void>());
        p.cast::<u8>()
    }
}

unsafe fn command_line_check(state: *mut VimState) -> c_int {
    unsafe {
        let s = state.cast::<CommandLineState>();

        (*s).prev_cmdpos = ccline.cmdpos;
        xfree((*s).prev_cmdbuff.cast::<c_void>());
        (*s).prev_cmdbuff = ptr::null_mut();

        redir_off = true;
        quit_more = false;
        did_emsg = 0;

        if ex_normal_busy == 0 && stuff_empty() && typebuf.tb_len == 0 {
            (*s).some_key_typed = true;
        }

        may_trigger_safestate((*s).xpc.xp_numfiles <= 0);

        if !ccline.cmdbuff.is_null() {
            (*s).prev_cmdbuff = xstrdup(ccline.cmdbuff);
        }

        if (*s).c == K_WILD && (*s).firstc != b'@' as c_int {
            (*s).skip_pum_redraw = true;
        }

        cursorcmd();
        ui_cursor_shape();
        1
    }
}

/// Handle `CTRL-\` pressed in Command-line mode.
unsafe fn command_line_handle_ctrl_bsl(s: *mut CommandLineState) -> c_int {
    unsafe {
        no_mapping += 1;
        allow_keys += 1;
        (*s).c = plain_vgetc();
        no_mapping -= 1;
        allow_keys -= 1;

        if (*s).c != Ctrl_N
            && (*s).c != Ctrl_G
            && ((*s).c != b'e' as c_int
                || (ccline.cmdfirstc == b'=' as c_int && KeyTyped)
                || cmdline_star > 0)
        {
            vungetc((*s).c);
            return PROCESS_NEXT_KEY;
        }

        if (*s).c == b'e' as c_int {
            if ccline.cmdpos == ccline.cmdlen {
                new_cmdpos = 99999;
            } else {
                new_cmdpos = ccline.cmdpos;
            }

            (*s).c = get_expr_register();
            if (*s).c == b'=' as c_int {
                textlock += 1;
                let p = get_expr_line();
                textlock -= 1;

                if !p.is_null() {
                    let len = strlen(p) as c_int;
                    realloc_cmdbuff(len + 1);
                    ccline.cmdlen = len;
                    strcpy(ccline.cmdbuff, p);
                    xfree(p.cast::<c_void>());

                    ccline.cmdpos = ccline.cmdlen.min(new_cmdpos);

                    KeyTyped = false;
                    redrawcmd();
                    return CMDLINE_CHANGED;
                }
            }
            beep_flush();
            got_int = false;
            did_emsg = 0;
            emsg_on_display = false;
            redrawcmd();
            return CMDLINE_NOT_CHANGED;
        }

        (*s).gotesc = true;
        GOTO_NORMAL_MODE
    }
}

/// Completion for `'wildchar'` or `'wildcharm'` key.
unsafe fn command_line_wildchar_complete(s: *mut CommandLineState) -> c_int {
    unsafe {
        let res: c_int;
        let mut options = WILD_NO_BEEP;
        let escape = (*s).firstc != b'@' as c_int;
        let redraw_if_menu_empty = (*s).c == K_WILD;
        let wim_noselect = p_wmnu != 0 && (wim_flags[0] & kOptWimFlagNoselect) != 0;
        let wim_noinsert = p_wmnu != 0 && (wim_flags[0] & kOptWimFlagNoinsert) != 0;

        if (wim_flags[(*s).wim_index as usize] & kOptWimFlagLastused) != 0 {
            options |= WILD_BUFLASTUSED;
        }
        if (*s).xpc.xp_numfiles > 0 {
            if (*s).xpc.xp_numfiles > 1
                && !(*s).did_wild_list
                && (wim_flags[(*s).wim_index as usize] & kOptWimFlagList) != 0
            {
                showmatches(
                    &raw mut (*s).xpc,
                    false,
                    true,
                    if p_wmnu != 0 {
                        wim_flags[(*s).wim_index as usize] as c_int
                    } else {
                        0
                    },
                );
                redrawcmd();
                (*s).did_wild_list = true;
            }
            if (wim_flags[(*s).wim_index as usize] & kOptWimFlagLongest) != 0 {
                res = nextwild(&raw mut (*s).xpc, WILD_LONGEST, options, escape);
            } else if (wim_flags[(*s).wim_index as usize] & kOptWimFlagFull) != 0 {
                res = nextwild(&raw mut (*s).xpc, WILD_NEXT, options, escape);
            } else {
                res = OK;
            }
        } else {
            let wim_longest = (wim_flags[0] & kOptWimFlagLongest) != 0;
            let wim_list = (wim_flags[0] & kOptWimFlagList) != 0;
            let wim_full = (wim_flags[0] & kOptWimFlagFull) != 0;

            (*s).wim_index = 0;
            if (*s).c as OptInt == p_wc
                || (*s).c as OptInt == p_wcm
                || (*s).c == K_WILD
                || (*s).c == Ctrl_Z
            {
                options |= WILD_MAY_EXPAND_PATTERN;
                if (*s).c == K_WILD {
                    options |= WILD_FUNC_TRIGGER;
                }
                (*s).xpc.xp_pre_incsearch_pos = (*s).is_state.search_start;
            }
            let cmdpos_before = ccline.cmdpos;

            if wim_longest {
                res = nextwild(&raw mut (*s).xpc, WILD_LONGEST, options, escape);
            } else {
                if wim_noselect || (wim_list && !wim_full) {
                    options |= WILD_NOSELECT;
                }
                if wim_noinsert {
                    options |= WILD_NOINSERT;
                }
                res = nextwild(&raw mut (*s).xpc, WILD_EXPAND_KEEP, options, escape);
            }

            if redraw_if_menu_empty && (*s).xpc.xp_numfiles <= 0 {
                pum_check_clear();
            }

            if got_int {
                vpeekc();
                got_int = false;
                ExpandOne(
                    &raw mut (*s).xpc,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    WILD_FREE,
                );
                (*s).xpc.xp_context = EXPAND_NOTHING;
                return CMDLINE_CHANGED;
            }

            if res == OK && (*s).xpc.xp_numfiles > if wim_noselect || wim_noinsert { 0 } else { 1 }
            {
                if wim_longest {
                    let found_longest_prefix = ccline.cmdpos != cmdpos_before;
                    let show_menu = p_wmnu != 0 && wim_full;
                    if wim_list || show_menu {
                        showmatches(
                            &raw mut (*s).xpc,
                            show_menu,
                            wim_list,
                            kOptWimFlagNoselect as c_int,
                        );
                    } else if !found_longest_prefix {
                        let wim_list_next = (wim_flags[1] & kOptWimFlagList) != 0;
                        let wim_full_next = (wim_flags[1] & kOptWimFlagFull) != 0;
                        let wim_noselect_next = (wim_flags[1] & kOptWimFlagNoselect) != 0;
                        let wim_noinsert_next = (wim_flags[1] & kOptWimFlagNoinsert) != 0;
                        if wim_list_next
                            || (p_wmnu != 0
                                && (wim_full_next || wim_noselect_next || wim_noinsert_next))
                        {
                            if wim_full_next && !wim_noselect_next && !wim_noinsert_next {
                                nextwild(&raw mut (*s).xpc, WILD_NEXT, options, escape);
                            } else {
                                showmatches(
                                    &raw mut (*s).xpc,
                                    p_wmnu != 0 && (wim_noselect_next || wim_noinsert_next),
                                    wim_list_next,
                                    if p_wmnu != 0 {
                                        wim_flags[1] as c_int
                                    } else {
                                        0
                                    },
                                );
                            }
                            if wim_list_next {
                                (*s).did_wild_list = true;
                            }
                        }
                    }
                } else {
                    let show_menu = p_wmnu != 0 && (wim_full || wim_noselect || wim_noinsert);
                    if wim_list || show_menu {
                        showmatches(
                            &raw mut (*s).xpc,
                            show_menu,
                            wim_list,
                            if p_wmnu != 0 {
                                wim_flags[0] as c_int
                            } else {
                                0
                            },
                        );
                    } else {
                        vim_beep(kOptBoFlagWildmode);
                    }
                }

                redrawcmd();
                if wim_list {
                    (*s).did_wild_list = true;
                }
            } else if (*s).xpc.xp_numfiles == -1 {
                (*s).xpc.xp_context = EXPAND_NOTHING;
            }
        }

        if (*s).wim_index < 3 {
            (*s).wim_index += 1;
        }

        if (*s).c == ESC {
            (*s).gotesc = true;
        }

        if res == OK {
            CMDLINE_CHANGED
        } else {
            CMDLINE_NOT_CHANGED
        }
    }
}

unsafe fn command_line_end_wildmenu(s: *mut CommandLineState, key_is_wc: bool, c: c_int) {
    unsafe {
        if cmdline_pum_active() {
            if c != -1 {
                (*s).skip_pum_redraw = (*s).skip_pum_redraw
                    && !key_is_wc
                    && !ascii_iswhite(c)
                    && (vim_isprintc(c)
                        || c == K_BS
                        || c == Ctrl_H
                        || c == K_DEL
                        || c == K_KDEL
                        || c == Ctrl_W
                        || c == Ctrl_U);
            }
            cmdline_pum_remove(c != -1 && (*s).skip_pum_redraw);
        }
        if (*s).xpc.xp_numfiles != -1 {
            ExpandOne(
                &raw mut (*s).xpc,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                WILD_FREE,
            );
        }
        (*s).did_wild_list = false;
        if p_wmnu == 0 || (c != K_UP && c != K_DOWN) {
            (*s).xpc.xp_context = EXPAND_NOTHING;
        }
        (*s).wim_index = 0;
        wildmenu_cleanup(&raw mut ccline);
    }
}

unsafe fn command_line_execute(state: *mut VimState, key: c_int) -> c_int {
    unsafe {
        if key == K_IGNORE || key == K_NOP {
            return -1; // get another key
        }

        let display_tick_saved: disptick_T = (*curwin).w_display_tick;
        let s = state.cast::<CommandLineState>();
        (*s).c = key;

        if ccline.cmdbuff_replaced && (*s).xpc.xp_numfiles > 0 {
            command_line_end_wildmenu(s, false, -1);
        }
        ccline.cmdbuff_replaced = false;

        if (*s).c == K_WILD && (*s).did_hist_navigate {
            (*s).did_hist_navigate = false;
            return 1;
        }

        if (*s).c == K_EVENT || (*s).c == K_COMMAND || (*s).c == K_LUA {
            if (*s).c == K_EVENT {
                state_handle_k_event();
            } else if (*s).c == K_COMMAND {
                do_cmdline(
                    ptr::null_mut(),
                    Some(getcmdkeycmd),
                    ptr::null_mut(),
                    DOCMD_NOWAIT,
                );
            } else {
                map_execute_lua(false, false);
            }
            if (*s).is_state.winid != (*curwin).handle {
                init_incsearch_state(&raw mut (*s).is_state);
            }
            if (*curwin).w_display_tick > display_tick_saved && (*s).is_state.did_incsearch {
                may_do_incsearch_highlighting((*s).firstc, (*s).count, &raw mut (*s).is_state);
            }
            if ccline.cmdbuff_replaced {
                command_line_changed(s);
            }

            if pum_want.active {
                if cmdline_pum_active() {
                    nextwild(
                        &raw mut (*s).xpc,
                        WILD_PUM_WANT,
                        0,
                        (*s).firstc != b'@' as c_int,
                    );
                    if pum_want.finish {
                        nextwild(
                            &raw mut (*s).xpc,
                            WILD_APPLY,
                            WILD_NO_BEEP,
                            (*s).firstc != b'@' as c_int,
                        );
                        command_line_end_wildmenu(s, false, (*s).c);
                    }
                }
                pum_want.active = false;
            }

            if !cmdline_was_last_drawn {
                redrawcmdline();
            }
            return 1;
        }

        if KeyTyped {
            (*s).some_key_typed = true;

            if cmdmsg_rl && KeyStuffed == 0 {
                match (*s).c {
                    K_RIGHT => (*s).c = K_LEFT,
                    K_S_RIGHT => (*s).c = K_S_LEFT,
                    K_C_RIGHT => (*s).c = K_C_LEFT,
                    K_LEFT => (*s).c = K_RIGHT,
                    K_S_LEFT => (*s).c = K_S_RIGHT,
                    K_C_LEFT => (*s).c = K_C_RIGHT,
                    _ => {}
                }
            }
        }

        if (*s).c == Ctrl_C
            && (*s).firstc != b'@' as c_int
            && !(*s).break_ctrl_c
            && global_busy == 0
        {
            got_int = false;
        }

        if !(*s).lookfor.is_null()
            && (*s).c != K_S_DOWN
            && (*s).c != K_S_UP
            && (*s).c != K_DOWN
            && (*s).c != K_UP
            && (*s).c != K_PAGEDOWN
            && (*s).c != K_PAGEUP
            && (*s).c != K_KPAGEDOWN
            && (*s).c != K_KPAGEUP
            && (*s).c != K_LEFT
            && (*s).c != K_RIGHT
            && ((*s).xpc.xp_numfiles > 0 || ((*s).c != Ctrl_P && (*s).c != Ctrl_N))
        {
            xfree((*s).lookfor.cast::<c_void>());
            (*s).lookfor = ptr::null_mut();
            (*s).lookforlen = 0;
        }

        if (*s).c as OptInt != p_wc && (*s).c == K_S_TAB && (*s).xpc.xp_numfiles > 0 {
            (*s).c = Ctrl_P;
        }

        if p_wmnu != 0 {
            (*s).c = wildmenu_translate_key(
                &raw mut ccline,
                (*s).c,
                &raw mut (*s).xpc,
                (*s).did_wild_list,
            );
        }

        let mut wild_type: c_int = 0;
        let key_is_wc = ((*s).c as OptInt == p_wc && KeyTyped) || (*s).c as OptInt == p_wcm;
        if (cmdline_pum_active() || wild_menu_showing != 0 || (*s).did_wild_list)
            && !key_is_wc
            && (*s).xpc.xp_numfiles > 0
            && ((*s).c == Ctrl_E || (*s).c == Ctrl_Y)
        {
            wild_type = if (*s).c == Ctrl_E {
                WILD_CANCEL
            } else {
                WILD_APPLY
            };
            nextwild(
                &raw mut (*s).xpc,
                wild_type,
                WILD_NO_BEEP,
                (*s).firstc != b'@' as c_int,
            );
        }

        if (KeyTyped
            && ((*s).c == b'\n' as c_int
                || (*s).c == b'\r' as c_int
                || (*s).c == K_KENTER
                || (*s).c == ESC))
            || (*s).c == Ctrl_C
        {
            set_vim_var_char((*s).c);
            trigger_cmd_autocmd((*s).cmdline_type, EVENT_CMDLINELEAVEPRE);
            (*s).event_cmdlineleavepre_triggered = true;
            if ((*s).c == ESC || (*s).c == Ctrl_C) && (wim_flags[0] & kOptWimFlagList) != 0 {
                set_no_hlsearch(true);
            }
        }

        let mut end_wildmenu = !key_is_wc
            && (*s).c != Ctrl_Z
            && (*s).c != Ctrl_N
            && (*s).c != Ctrl_P
            && (*s).c != Ctrl_A
            && (*s).c != Ctrl_L;
        end_wildmenu = end_wildmenu
            && (!cmdline_pum_active()
                || ((*s).c != K_PAGEDOWN
                    && (*s).c != K_PAGEUP
                    && (*s).c != K_KPAGEDOWN
                    && (*s).c != K_KPAGEUP
                    && (*s).c != K_MOUSEDOWN
                    && (*s).c != K_MOUSEUP
                    && (*s).c != K_MOUSELEFT
                    && (*s).c != K_MOUSERIGHT));

        if end_wildmenu {
            command_line_end_wildmenu(s, key_is_wc, (*s).c);
        }

        if p_wmnu != 0 {
            (*s).c = wildmenu_process_key(&raw mut ccline, (*s).c, &raw mut (*s).xpc);
        }

        if (*s).c == Ctrl_BSL {
            match command_line_handle_ctrl_bsl(s) {
                CMDLINE_CHANGED => return command_line_changed(s),
                CMDLINE_NOT_CHANGED => return command_line_not_changed(s),
                GOTO_NORMAL_MODE => return 0,
                _ => {
                    (*s).c = Ctrl_BSL;
                }
            }
        }

        if (*s).c == cedit_key {
            if ex_normal_busy == 0 && !got_int {
                (*s).c = open_cmdwin();
                (*s).some_key_typed = true;
            }
        } else {
            (*s).c = do_digraph((*s).c);
        }

        if (*s).c == b'\n' as c_int
            || (*s).c == b'\r' as c_int
            || (*s).c == K_KENTER
            || ((*s).c == ESC && (!KeyTyped || !vim_strchr(p_cpo, kCpoEsc).is_null()))
        {
            (*s).gotesc = false;
            if ccheck_abbr((*s).c + ABBR_OFF) != 0 {
                return command_line_changed(s);
            }

            if !cmd_silent {
                if !ui_has(kUICmdline) {
                    msg_cursor_goto(msg_row, 0);
                }
                ui_flush();
            }
            return 0;
        }

        if ((*s).c as OptInt == p_wc && !(*s).gotesc && KeyTyped)
            || (*s).c as OptInt == p_wcm
            || (*s).c == K_WILD
            || (*s).c == Ctrl_Z
        {
            if (*s).c == K_WILD {
                emsg_silent += 1;
            }
            let res = command_line_wildchar_complete(s);
            if (*s).c == K_WILD {
                emsg_silent -= 1;
            }
            if res == CMDLINE_CHANGED {
                return command_line_changed(s);
            }
            if (*s).c == K_WILD {
                return command_line_not_changed(s);
            }
        }

        (*s).gotesc = false;

        if (*s).c == K_S_TAB
            && KeyTyped
            && nextwild(
                &raw mut (*s).xpc,
                WILD_EXPAND_KEEP,
                0,
                (*s).firstc != b'@' as c_int,
            ) == OK
        {
            if (*s).xpc.xp_numfiles > 1
                && ((!(*s).did_wild_list
                    && (wim_flags[(*s).wim_index as usize] & kOptWimFlagList) != 0)
                    || p_wmnu != 0)
            {
                let list = (wim_flags[(*s).wim_index as usize] & kOptWimFlagList) != 0;
                showmatches(
                    &raw mut (*s).xpc,
                    p_wmnu != 0 && !list,
                    list,
                    if p_wmnu != 0 {
                        wim_flags[0] as c_int
                    } else {
                        0
                    },
                );
            }
            nextwild(
                &raw mut (*s).xpc,
                WILD_PREV,
                0,
                (*s).firstc != b'@' as c_int,
            );
            nextwild(
                &raw mut (*s).xpc,
                WILD_PREV,
                0,
                (*s).firstc != b'@' as c_int,
            );
            return command_line_changed(s);
        }

        if (*s).c == NUL as c_int || (*s).c == K_ZERO {
            (*s).c = NL;
        }

        (*s).do_abbr = true;

        if wild_type == WILD_CANCEL || wild_type == WILD_APPLY {
            if (*s).is_state.winid != (*curwin).handle {
                init_incsearch_state(&raw mut (*s).is_state);
            }
            if KeyTyped || vpeekc() == NUL as c_int {
                may_do_incsearch_highlighting((*s).firstc, (*s).count, &raw mut (*s).is_state);
            }
            return command_line_not_changed(s);
        }

        command_line_handle_key(s)
    }
}

/// May adjust `'incsearch'` highlighting for typing CTRL-G and CTRL-T, go to next
/// or previous match.
/// Returns `FAIL` when calling `command_line_not_changed`.
unsafe fn may_do_command_line_next_incsearch(
    firstc: c_int,
    count: c_int,
    s: *mut incsearch_state_T,
    next_match: bool,
) -> c_int {
    unsafe {
        let mut skiplen: c_int = 0;
        let mut patlen: c_int = 0;
        let mut search_delim: c_int = 0;
        let mut magic: bool = p_magic != 0;

        save_last_search_pattern();

        if !do_incsearch_highlighting(
            firstc,
            &raw mut search_delim,
            s,
            &raw mut skiplen,
            &raw mut patlen,
            &raw mut magic,
        ) {
            restore_last_search_pattern();
            return OK;
        }
        if patlen == 0 && *ccline.cmdbuff.add(skiplen as usize) == NUL {
            restore_last_search_pattern();
            return FAIL;
        }

        ui_busy_start();
        ui_flush();

        let mut t: pos_T;
        let mut pat = ccline.cmdbuff.add(skiplen as usize);
        let mut dircp: *mut c_char = ptr::null_mut();
        let mut searchstr = pat;
        let mut strcopy: *mut c_char = ptr::null_mut();
        let mut searchstrlen = patlen as usize;
        let mut offset: SearchOffset = std::mem::zeroed();
        let mut search_flags = SEARCH_NOOF;
        let mut patlen_s = (ccline.cmdlen - skiplen) as usize;

        parse_search_pattern_offset(
            &raw mut pat,
            &raw mut patlen_s,
            search_delim,
            SEARCH_OPT,
            &raw mut strcopy,
            &raw mut searchstr,
            &raw mut searchstrlen,
            &raw mut dircp,
            &raw mut offset,
        );

        if next_match {
            t = (*s).match_end;
            if lt((*s).match_start, (*s).match_end) {
                decl(&raw mut t);
            }
            search_flags += SEARCH_COL;
        } else {
            t = (*s).match_start;
        }
        if p_hls == 0 {
            search_flags += SEARCH_KEEP;
        }
        emsg_off += 1;
        let found = searchit(
            curwin,
            curbuf,
            &raw mut t,
            ptr::null_mut(),
            if next_match { FORWARD } else { BACKWARD },
            searchstr,
            searchstrlen,
            count,
            search_flags,
            RE_SEARCH,
            magic,
            ptr::null_mut(),
        );
        emsg_off -= 1;
        if !dircp.is_null() {
            *dircp = search_delim as c_char;
        }
        ui_busy_stop();
        if found != 0 {
            let match_start = (*s).match_start;
            let match_end = (*s).match_end;
            let mut off = offset.off;

            (*s).search_start = match_start;
            if !offset.line && (offset.end || off != 0) {
                if offset.end {
                    (*s).search_start = match_end;
                    decl(&raw mut (*s).search_start);
                }
                while off > 0 {
                    if incl(&raw mut (*s).search_start) == -1 {
                        break;
                    }
                    off -= 1;
                }
                while off < 0 {
                    if decl(&raw mut (*s).search_start) == -1 {
                        break;
                    }
                    off += 1;
                }
            }
            (*s).match_end = t;
            (*s).match_start = t;
            if !next_match && firstc != b'?' as c_int {
                (*s).search_start = t;
                decl(&raw mut (*s).search_start);
            } else if next_match && firstc == b'?' as c_int {
                (*s).search_start = t;
                incl(&raw mut (*s).search_start);
            }
            if lt(t, (*s).search_start) && next_match {
                (*s).search_start = t;
                if firstc == b'?' as c_int {
                    incl(&raw mut (*s).search_start);
                } else {
                    decl(&raw mut (*s).search_start);
                }
            }

            set_search_match(&raw mut (*s).match_end);
            (*curwin).w_cursor = (*s).match_start;
            changed_cline_bef_curs(curwin);
            update_topline(curwin);
            validate_cursor(curwin);
            Search.hl_match = true;
            save_viewstate(curwin, &raw mut (*s).old_viewstate);
            redraw_later(curwin, UPD_NOT_VALID);
            update_screen();
            Search.hl_match = false;
            redrawcmdline();
            (*curwin).w_cursor = (*s).match_end;
        } else {
            vim_beep(kOptBoFlagError);
        }
        xfree(strcopy.cast::<c_void>());
        restore_last_search_pattern();
        FAIL
    }
}

/// Handle backspace, delete and CTRL-W keys in the command-line mode.
unsafe fn command_line_erase_chars(s: *mut CommandLineState) -> c_int {
    unsafe {
        if (*s).c == K_KDEL {
            (*s).c = K_DEL;
        }

        if (*s).c == K_DEL && ccline.cmdpos != ccline.cmdlen {
            ccline.cmdpos += 1;
        }
        if (*s).c == K_DEL {
            ccline.cmdpos +=
                mb_off_next(ccline.cmdbuff, ccline.cmdbuff.add(ccline.cmdpos as usize));
        }

        if ccline.cmdpos > 0 {
            let mut j = ccline.cmdpos;
            let mut p = mb_prevptr(ccline.cmdbuff, ccline.cmdbuff.add(j as usize));

            if (*s).c == Ctrl_W {
                while p > ccline.cmdbuff && ascii_isspace(*p as c_int) {
                    p = mb_prevptr(ccline.cmdbuff, p);
                }

                let cls = mb_get_class(p);
                while p > ccline.cmdbuff && mb_get_class(p) == cls {
                    p = mb_prevptr(ccline.cmdbuff, p);
                }

                if mb_get_class(p) != cls {
                    p = p.add(utfc_ptr2len(p) as usize);
                }
            }

            ccline.cmdpos = p.offset_from(ccline.cmdbuff) as c_int;
            ccline.cmdlen -= j - ccline.cmdpos;
            let mut i = ccline.cmdpos;

            while i < ccline.cmdlen {
                *ccline.cmdbuff.add(i as usize) = *ccline.cmdbuff.add(j as usize);
                i += 1;
                j += 1;
            }

            *ccline.cmdbuff.add(ccline.cmdlen as usize) = NUL;
            if ccline.cmdlen == 0 {
                (*s).is_state.search_start = (*s).is_state.save_cursor;
                (*s).is_state.old_viewstate = (*s).is_state.init_viewstate;
            }
            redrawcmd();
        } else if ccline.cmdlen == 0
            && (*s).c != Ctrl_W
            && ccline.cmdprompt.is_null()
            && (*s).indent == 0
        {
            if ccline.cmdfirstc == b'>' as c_int {
                return CMDLINE_NOT_CHANGED;
            }

            dealloc_cmdbuff();

            if !cmd_silent && !ui_has(kUICmdline) {
                msg_col = 0;
                msg_putchar(b' ' as c_int);
            }
            (*s).is_state.search_start = (*s).is_state.save_cursor;
            redraw_cmdline = true;
            return GOTO_NORMAL_MODE;
        }
        CMDLINE_CHANGED
    }
}

/// Handle the CTRL-^ key in the command-line mode and toggle the use of the
/// language `:lmap` mappings and/or Input Method.
unsafe fn command_line_toggle_langmap(s: *mut CommandLineState) {
    unsafe {
        let b_im_ptr = if buf_valid((*s).b_im_ptr_buf) {
            (*s).b_im_ptr
        } else {
            ptr::null_mut()
        };
        if map_to_exists_mode(c"".as_ptr(), MODE_LANGMAP, false) {
            State ^= MODE_LANGMAP;
            if !b_im_ptr.is_null() {
                if (State & MODE_LANGMAP) != 0 {
                    *b_im_ptr = B_IMODE_LMAP;
                } else {
                    *b_im_ptr = B_IMODE_NONE;
                }
            }
        }

        if !b_im_ptr.is_null() {
            if b_im_ptr == &raw mut (*curbuf).b_p_iminsert {
                set_iminsert_global(curbuf);
            } else {
                set_imsearch_global(curbuf);
            }
        }
        ui_cursor_shape();
        status_redraw_curbuf();
    }
}

/// Handle the CTRL-R key in the command-line mode and insert the contents of a
/// numbered or named register.
unsafe fn command_line_insert_reg(s: *mut CommandLineState) -> c_int {
    unsafe {
        let save_new_cmdpos = new_cmdpos;

        putcmdline(b'"' as c_char, true);
        no_mapping += 1;
        allow_keys += 1;
        (*s).c = plain_vgetc();
        let mut i = (*s).c;
        if i == Ctrl_O {
            i = Ctrl_R;
        }

        if i == Ctrl_R {
            (*s).c = plain_vgetc();
        }
        no_mapping -= 1;
        allow_keys -= 1;
        new_cmdpos = -1;
        if (*s).c == b'=' as c_int {
            if ccline.cmdfirstc == b'=' as c_int || cmdline_star > 0 {
                beep_flush();
                (*s).c = ESC;
            } else {
                (*s).c = get_expr_register();
            }
        }

        let mut literally = false;
        if (*s).c != ESC {
            literally = i == Ctrl_R || is_literal_register((*s).c);
            cmdline_paste((*s).c, literally, false);

            if aborting() {
                (*s).gotesc = true;
                return GOTO_NORMAL_MODE;
            }
            KeyTyped = false;
            if new_cmdpos >= 0 {
                ccline.cmdpos = ccline.cmdlen.min(new_cmdpos);
            }
        }
        new_cmdpos = save_new_cmdpos;

        ccline.special_char = NUL;
        redrawcmd();

        if literally {
            CMDLINE_CHANGED
        } else {
            CMDLINE_NOT_CHANGED
        }
    }
}

/// Handle the Left and Right mouse clicks in the command-line mode.
unsafe fn command_line_left_right_mouse(s: *mut CommandLineState) {
    unsafe {
        (*s).ignore_drag_release = (*s).c == K_LEFTRELEASE || (*s).c == K_RIGHTRELEASE;

        ccline.cmdspos = cmd_startcol();
        ccline.cmdpos = 0;
        while ccline.cmdpos < ccline.cmdlen {
            let cells = cmdline_charsize(ccline.cmdpos);
            if mouse_row <= cmdline_row + ccline.cmdspos / Columns
                && mouse_col < ccline.cmdspos % Columns + cells
            {
                break;
            }

            correct_screencol(ccline.cmdpos, cells, &raw mut ccline.cmdspos);
            ccline.cmdpos += utfc_ptr2len(ccline.cmdbuff.add(ccline.cmdpos as usize)) - 1;
            ccline.cmdspos += cells;
            ccline.cmdpos += 1;
        }
    }
}

unsafe fn command_line_next_histidx(s: *mut CommandLineState, next_match: bool) {
    unsafe {
        loop {
            if !next_match {
                if (*s).hiscnt == get_hislen() {
                    (*s).hiscnt = *get_hisidx((*s).histype);
                } else if (*s).hiscnt == 0 && *get_hisidx((*s).histype) != get_hislen() - 1 {
                    (*s).hiscnt = get_hislen() - 1;
                } else if (*s).hiscnt != *get_hisidx((*s).histype) + 1 {
                    (*s).hiscnt -= 1;
                } else {
                    (*s).hiscnt = (*s).save_hiscnt;
                    break;
                }
            } else {
                if (*s).hiscnt == *get_hisidx((*s).histype) {
                    (*s).hiscnt = get_hislen();
                    break;
                }

                if (*s).hiscnt == get_hislen() {
                    break;
                }

                if (*s).hiscnt == get_hislen() - 1 {
                    (*s).hiscnt = 0;
                } else {
                    (*s).hiscnt += 1;
                }
            }

            if (*s).hiscnt < 0
                || (*get_histentry((*s).histype).add((*s).hiscnt as usize))
                    .hisstr
                    .is_null()
            {
                (*s).hiscnt = (*s).save_hiscnt;
                break;
            }

            if ((*s).c != K_UP && (*s).c != K_DOWN)
                || (*s).hiscnt == (*s).save_hiscnt
                || strncmp(
                    (*get_histentry((*s).histype).add((*s).hiscnt as usize)).hisstr,
                    (*s).lookfor,
                    (*s).lookforlen as usize,
                ) == 0
            {
                break;
            }
        }
    }
}

/// Handle the Up, Down, Page Up, Page down, CTRL-N and CTRL-P key in the
/// command-line mode.
unsafe fn command_line_browse_history(s: *mut CommandLineState) -> c_int {
    unsafe {
        if (*s).histype == HIST_INVALID || get_hislen() == 0 || (*s).firstc == NUL as c_int {
            return CMDLINE_NOT_CHANGED;
        }

        (*s).save_hiscnt = (*s).hiscnt;

        if (*s).lookfor.is_null() {
            (*s).lookfor = xstrnsave(ccline.cmdbuff, ccline.cmdlen as usize);
            *(*s).lookfor.add(ccline.cmdpos as usize) = NUL;
            (*s).lookforlen = ccline.cmdpos;
        }

        let next_match = (*s).c == K_DOWN
            || (*s).c == K_S_DOWN
            || (*s).c == Ctrl_N
            || (*s).c == K_PAGEDOWN
            || (*s).c == K_KPAGEDOWN;
        command_line_next_histidx(s, next_match);

        if (*s).hiscnt != (*s).save_hiscnt {
            let p: *mut c_char;
            let plen: c_int;
            let old_firstc: c_int;

            dealloc_cmdbuff();

            (*s).xpc.xp_context = EXPAND_NOTHING;
            if (*s).hiscnt == get_hislen() {
                p = (*s).lookfor;
                plen = (*s).lookforlen;
            } else {
                p = (*get_histentry((*s).histype).add((*s).hiscnt as usize)).hisstr;
                plen = (*get_histentry((*s).histype).add((*s).hiscnt as usize)).hisstrlen as c_int;
            }

            if (*s).histype == HIST_SEARCH && p != (*s).lookfor && {
                old_firstc = (*p.add((plen + 1) as usize) as u8) as c_int;
                old_firstc != (*s).firstc
            } {
                let mut len: c_int = 0;
                for i in 0..=1 {
                    len = 0;
                    let mut j: usize = 0;
                    while *p.add(j) != NUL {
                        if *p.add(j) as c_int == old_firstc
                            && (j == 0 || *p.add(j - 1) != b'\\' as c_char)
                        {
                            if i > 0 {
                                *ccline.cmdbuff.add(len as usize) = (*s).firstc as c_char;
                            }
                        } else {
                            if *p.add(j) as c_int == (*s).firstc
                                && (j == 0 || *p.add(j - 1) != b'\\' as c_char)
                            {
                                if i > 0 {
                                    *ccline.cmdbuff.add(len as usize) = b'\\' as c_char;
                                }
                                len += 1;
                            }

                            if i > 0 {
                                *ccline.cmdbuff.add(len as usize) = *p.add(j);
                            }
                        }
                        len += 1;
                        j += 1;
                    }

                    if i == 0 {
                        alloc_cmdbuff(len);
                    }
                }
                *ccline.cmdbuff.add(len as usize) = NUL;
                ccline.cmdlen = len;
                ccline.cmdpos = len;
            } else {
                alloc_cmdbuff(plen);
                strcpy(ccline.cmdbuff, p);
                ccline.cmdlen = plen;
                ccline.cmdpos = plen;
            }

            redrawcmd();
            return CMDLINE_CHANGED;
        }
        beep_flush();
        CMDLINE_NOT_CHANGED
    }
}

unsafe fn command_line_handle_key(s: *mut CommandLineState) -> c_int {
    unsafe {
        if !(ccline.one_key && (*s).c != ESC && (*s).c != Ctrl_C) {
            'key_switch: {
                match (*s).c {
                    K_BS | Ctrl_H | K_DEL | K_KDEL | Ctrl_W => {
                        return match command_line_erase_chars(s) {
                            CMDLINE_NOT_CHANGED => command_line_not_changed(s),
                            GOTO_NORMAL_MODE => 0,
                            _ => command_line_changed(s),
                        };
                    }

                    K_INS | K_KINS => {
                        ccline.overstrike = if ccline.overstrike != 0 { 0 } else { 1 };
                        ui_cursor_shape();
                        may_trigger_modechanged();
                        status_redraw_curbuf();
                        redraw_statuslines();
                        return command_line_not_changed(s);
                    }

                    Ctrl_HAT => {
                        command_line_toggle_langmap(s);
                        return command_line_not_changed(s);
                    }

                    Ctrl_U => {
                        let mut j = ccline.cmdpos;
                        ccline.cmdlen -= j;
                        ccline.cmdpos = 0;
                        let mut i = 0;
                        while i < ccline.cmdlen {
                            *ccline.cmdbuff.add(i as usize) = *ccline.cmdbuff.add(j as usize);
                            i += 1;
                            j += 1;
                        }

                        *ccline.cmdbuff.add(ccline.cmdlen as usize) = NUL;
                        if ccline.cmdlen == 0 {
                            (*s).is_state.search_start = (*s).is_state.save_cursor;
                        }
                        redrawcmd();
                        return command_line_changed(s);
                    }

                    ESC | Ctrl_C => {
                        if getln_interrupted_highlight && (*s).c == Ctrl_C {
                            getln_interrupted_highlight = false;
                            return command_line_not_changed(s);
                        }

                        (*s).gotesc = true;
                        return 0;
                    }

                    Ctrl_R => {
                        return match command_line_insert_reg(s) {
                            GOTO_NORMAL_MODE => 0,
                            CMDLINE_CHANGED => command_line_changed(s),
                            _ => command_line_not_changed(s),
                        };
                    }

                    Ctrl_D => {
                        if showmatches(
                            &raw mut (*s).xpc,
                            false,
                            true,
                            if p_wmnu != 0 {
                                wim_flags[0] as c_int
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
                            if ccline.cmdpos >= ccline.cmdlen {
                                break;
                            }

                            let cells = cmdline_charsize(ccline.cmdpos);
                            if KeyTyped && ccline.cmdspos + cells >= Columns * Rows {
                                break;
                            }

                            ccline.cmdspos += cells;
                            ccline.cmdpos +=
                                utfc_ptr2len(ccline.cmdbuff.add(ccline.cmdpos as usize));

                            if !(((*s).c == K_S_RIGHT
                                || (*s).c == K_C_RIGHT
                                || (mod_mask & ((MOD_MASK_SHIFT | MOD_MASK_CTRL) as c_int)) != 0)
                                && *ccline.cmdbuff.add(ccline.cmdpos as usize) != b' ' as c_char)
                            {
                                break;
                            }
                        }
                        ccline.cmdspos = cmd_screencol(ccline.cmdpos);
                        return command_line_not_changed(s);
                    }

                    K_LEFT | K_S_LEFT | K_C_LEFT => {
                        if ccline.cmdpos == 0 {
                            return command_line_not_changed(s);
                        }
                        loop {
                            ccline.cmdpos -= 1;
                            ccline.cmdpos -= utf_head_off(
                                ccline.cmdbuff,
                                ccline.cmdbuff.add(ccline.cmdpos as usize),
                            );
                            ccline.cmdspos -= cmdline_charsize(ccline.cmdpos);

                            if !(ccline.cmdpos > 0
                                && ((*s).c == K_S_LEFT
                                    || (*s).c == K_C_LEFT
                                    || (mod_mask & ((MOD_MASK_SHIFT | MOD_MASK_CTRL) as c_int))
                                        != 0)
                                && *ccline.cmdbuff.add((ccline.cmdpos - 1) as usize)
                                    != b' ' as c_char)
                            {
                                break;
                            }
                        }

                        ccline.cmdspos = cmd_screencol(ccline.cmdpos);
                        if ccline.special_char != NUL {
                            putcmdline(ccline.special_char, ccline.special_shift);
                        }

                        return command_line_not_changed(s);
                    }

                    K_IGNORE | K_MIDDLEDRAG | K_MIDDLERELEASE => {
                        return command_line_not_changed(s);
                    }

                    K_MIDDLEMOUSE => {
                        cmdline_paste(
                            if eval_has_provider(c"clipboard".as_ptr(), false) {
                                b'*' as c_int
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
                        if (*s).ignore_drag_release {
                            return command_line_not_changed(s);
                        }
                        if !ccline.mouse_used.is_null() && mouse_row < cmdline_row {
                            *ccline.mouse_used = true;
                            return 0;
                        }
                        command_line_left_right_mouse(s);
                        return command_line_not_changed(s);
                    }

                    K_LEFTMOUSE => {
                        if !ccline.mouse_used.is_null() && mouse_row < cmdline_row {
                            *ccline.mouse_used = true;
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
                            cmdline_mousescroll(if (*s).c == K_MOUSEDOWN {
                                MSCR_DOWN
                            } else if (*s).c == K_MOUSEUP {
                                MSCR_UP
                            } else if (*s).c == K_MOUSELEFT {
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

                    Ctrl_B | K_HOME | K_KHOME | K_S_HOME | K_C_HOME => {
                        ccline.cmdpos = 0;
                        ccline.cmdspos = cmd_startcol();
                        return command_line_not_changed(s);
                    }

                    Ctrl_E | K_END | K_KEND | K_S_END | K_C_END => {
                        ccline.cmdpos = ccline.cmdlen;
                        ccline.cmdspos = cmd_screencol(ccline.cmdpos);
                        return command_line_not_changed(s);
                    }

                    Ctrl_A => {
                        if cmdline_pum_active() {
                            cmdline_pum_cleanup(&raw mut ccline);
                        }

                        if nextwild(&raw mut (*s).xpc, WILD_ALL, 0, (*s).firstc != b'@' as c_int)
                            == FAIL
                        {
                            break 'key_switch;
                        }
                        (*s).xpc.xp_context = EXPAND_NOTHING;
                        (*s).did_wild_list = false;
                        return command_line_changed(s);
                    }

                    Ctrl_L => {
                        if may_add_char_to_search(
                            (*s).firstc,
                            &raw mut (*s).c,
                            &raw mut (*s).is_state,
                        ) == OK
                        {
                            return command_line_not_changed(s);
                        }

                        if nextwild(
                            &raw mut (*s).xpc,
                            WILD_LONGEST,
                            0,
                            (*s).firstc != b'@' as c_int,
                        ) == FAIL
                        {
                            break 'key_switch;
                        }
                        return command_line_changed(s);
                    }

                    Ctrl_N | Ctrl_P => {
                        if (*s).xpc.xp_numfiles > 0 {
                            let wild_type = if (*s).c == Ctrl_P {
                                WILD_PREV
                            } else {
                                WILD_NEXT
                            };
                            if nextwild(
                                &raw mut (*s).xpc,
                                wild_type,
                                0,
                                (*s).firstc != b'@' as c_int,
                            ) == FAIL
                            {
                                break 'key_switch;
                            }
                            return command_line_changed(s);
                        }
                        match command_line_browse_history(s) {
                            CMDLINE_CHANGED => {
                                (*s).did_hist_navigate = true;
                                return command_line_changed(s);
                            }
                            GOTO_NORMAL_MODE => return 0,
                            _ => return command_line_not_changed(s),
                        }
                    }

                    K_UP | K_DOWN | K_S_UP | K_S_DOWN | K_PAGEUP | K_KPAGEUP | K_PAGEDOWN
                    | K_KPAGEDOWN => {
                        if cmdline_pum_active()
                            && ((*s).c == K_PAGEUP
                                || (*s).c == K_PAGEDOWN
                                || (*s).c == K_KPAGEUP
                                || (*s).c == K_KPAGEDOWN)
                        {
                            let wild_type = if (*s).c == K_PAGEDOWN || (*s).c == K_KPAGEDOWN {
                                WILD_PAGEDOWN
                            } else {
                                WILD_PAGEUP
                            };
                            if nextwild(
                                &raw mut (*s).xpc,
                                wild_type,
                                0,
                                (*s).firstc != b'@' as c_int,
                            ) == FAIL
                            {
                                break 'key_switch;
                            }
                            return command_line_changed(s);
                        } else {
                            match command_line_browse_history(s) {
                                CMDLINE_CHANGED => {
                                    (*s).did_hist_navigate = true;
                                    return command_line_changed(s);
                                }
                                GOTO_NORMAL_MODE => return 0,
                                _ => return command_line_not_changed(s),
                            }
                        }
                    }

                    Ctrl_G | Ctrl_T => {
                        if may_do_command_line_next_incsearch(
                            (*s).firstc,
                            (*s).count,
                            &raw mut (*s).is_state,
                            (*s).c == Ctrl_G,
                        ) == FAIL
                        {
                            return command_line_not_changed(s);
                        }
                    }

                    Ctrl_V | Ctrl_Q => {
                        (*s).ignore_drag_release = true;
                        putcmdline(b'^' as c_char, true);

                        (*s).c = get_literal((mod_mask & (MOD_MASK_SHIFT as c_int)) != 0);

                        (*s).do_abbr = false;
                        ccline.special_char = NUL;
                        if utf_iscomposing_first((*s).c) && !cmd_silent {
                            if ui_has(kUICmdline) {
                                unputcmdline();
                            } else {
                                draw_cmdline(ccline.cmdpos, ccline.cmdlen - ccline.cmdpos);
                                msg_putchar(b' ' as c_int);
                                cursorcmd();
                            }
                        }
                    }

                    Ctrl_K => {
                        (*s).ignore_drag_release = true;
                        putcmdline(b'?' as c_char, true);
                        (*s).c = get_digraph(true);
                        ccline.special_char = NUL;

                        if (*s).c != NUL as c_int {
                            break 'key_switch;
                        }

                        redrawcmd();
                        return command_line_not_changed(s);
                    }

                    Ctrl__ => {
                        if p_ari == 0 {
                            break 'key_switch;
                        }
                        return command_line_not_changed(s);
                    }

                    c if c == b'q' as c_int => {
                        if !ccline.mouse_used.is_null() {
                            *ccline.cmdbuff = NUL;
                            return 0;
                        }
                        if !is_special((*s).c) {
                            mod_mask = 0x0;
                        }
                    }

                    _ => {
                        if !is_special((*s).c) {
                            mod_mask = 0x0;
                        }
                    }
                }
            }

            if (*s).do_abbr
                && (is_special((*s).c) || !vim_iswordc((*s).c))
                && (ccheck_abbr(if (*s).c >= 0x100 {
                    (*s).c + ABBR_OFF
                } else {
                    (*s).c
                }) != 0
                    || (*s).c == Ctrl_RSB)
            {
                return command_line_changed(s);
            }
        }

        // end:
        if is_special((*s).c) || mod_mask != 0 {
            put_on_cmdline(get_special_key_name((*s).c, mod_mask), -1, true);
        } else {
            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            let j = utf_char2bytes((*s).c, iobuff_ptr);
            *iobuff_ptr.add(j as usize) = NUL;
            put_on_cmdline(iobuff_ptr, j, true);
        }
        if ccline.one_key {
            0
        } else {
            command_line_changed(s)
        }
    }
}

/// Trigger `CursorMovedC` autocommands.
unsafe fn may_trigger_cursormovedc(s: *mut CommandLineState) {
    unsafe {
        if ccline.cmdpos != (*s).prev_cmdpos {
            trigger_cmd_autocmd((*s).cmdline_type, EVENT_CURSORMOVEDC);
            ccline.redraw_state = ccline.redraw_state.max(kCmdRedrawPos);
        }
    }
}

unsafe fn command_line_not_changed(s: *mut CommandLineState) -> c_int {
    unsafe {
        may_trigger_cursormovedc(s);
        (*s).prev_cmdpos = ccline.cmdpos;
        if !(*s).is_state.incsearch_postponed {
            return 1;
        }
        command_line_changed(s)
    }
}

/// Guess that the pattern matches everything.
unsafe fn empty_pattern(p: *mut c_char, len: usize, delim: c_int) -> bool {
    unsafe {
        let mut magic_val: magic_T = MAGIC_ON;

        if len > 0 {
            skip_regexp_ex(
                p,
                delim,
                p_magic,
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut magic_val,
            );
        } else {
            return true;
        }

        empty_pattern_magic(p, len, magic_val)
    }
}

unsafe fn empty_pattern_magic(p: *mut c_char, mut len: usize, magic_val: magic_T) -> bool {
    unsafe {
        while len >= 2
            && *p.add(len - 2) == b'\\' as c_char
            && !vim_strchr(c"mMvVcCZ".as_ptr(), (*p.add(len - 1) as u8) as c_int).is_null()
        {
            len -= 2;
        }

        len == 0
            || (len > 1
                && *p.add(len - 1) == b'|' as c_char
                && ((*p.add(len - 2) == b'\\' as c_char && magic_val == MAGIC_ON)
                    || (*p.add(len - 2) != b'\\' as c_char && magic_val == MAGIC_ALL)))
    }
}

pub unsafe fn cmdpreview_get_bufnr() -> handle_T {
    unsafe { cmdpreview_bufnr }
}

pub unsafe fn cmdpreview_get_ns() -> c_int {
    unsafe { cmdpreview_ns }
}

/// Sets up command preview buffer.
unsafe fn cmdpreview_open_buf() -> *mut buf_T {
    unsafe {
        let mut cmdpreview_buf = if cmdpreview_bufnr != 0 {
            buflist_findnr(cmdpreview_bufnr)
        } else {
            ptr::null_mut()
        };

        if cmdpreview_buf.is_null() {
            let mut err = error_init();
            let bufnr = nvim_create_buf(false, true, &raw mut err);

            if error_set(&err) {
                return ptr::null_mut();
            }

            cmdpreview_buf = buflist_findnr(bufnr);
        }

        if cmdpreview_buf == curbuf {
            return ptr::null_mut();
        }

        let mut aco: CtxSwitch = std::mem::zeroed();
        ctx_switch(
            &raw mut aco,
            ptr::null_mut(),
            ptr::null_mut(),
            cmdpreview_buf,
            kCtxKeepDirs,
        );
        let retv = rename_buffer(c"[Preview]".as_ptr().cast_mut());
        ctx_restore(&raw mut aco);

        if retv == FAIL {
            return ptr::null_mut();
        }

        ctx_switch(
            &raw mut aco,
            ptr::null_mut(),
            ptr::null_mut(),
            cmdpreview_buf,
            kCtxKeepDirs,
        );
        buf_clear();
        (*curbuf).b_p_ma = 1;
        (*curbuf).b_p_ul = -1;
        (*curbuf).b_p_tw = 0;
        ctx_restore(&raw mut aco);
        cmdpreview_bufnr = (*cmdpreview_buf).b_fnum;

        cmdpreview_buf
    }
}

/// Open command preview window if it's not already open.
unsafe fn cmdpreview_open_win(cmdpreview_buf: *mut buf_T) -> *mut win_T {
    unsafe {
        let save_curwin = curwin;

        if win_split(p_cwh as c_int, WSP_BOT) == FAIL {
            return ptr::null_mut();
        }

        let preview_win = curwin;
        let mut err = error_init();

        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&raw mut tstate);
        let result = do_buffer(
            DOBUF_GOTO,
            DOBUF_FIRST,
            FORWARD,
            (*cmdpreview_buf).b_fnum,
            0,
        );
        try_leave(&raw const tstate, &raw mut err);
        if error_set(&err) || result == FAIL {
            api_clear_error(&raw mut err);
            return ptr::null_mut();
        }

        (*curwin).w_onebuf_opt.wo_cul = 0;
        (*curwin).w_onebuf_opt.wo_cuc = 0;
        (*curwin).w_onebuf_opt.wo_spell = 0;
        (*curwin).w_onebuf_opt.wo_fen = 0;

        win_enter(save_curwin, false);
        preview_win
    }
}

/// Closes any open command preview windows.
unsafe fn cmdpreview_close_win() {
    unsafe {
        let buf = if cmdpreview_bufnr != 0 {
            buflist_findnr(cmdpreview_bufnr)
        } else {
            ptr::null_mut()
        };
        if !buf.is_null() {
            close_windows(buf, false);
        }
    }
}

/// Save current state and prepare windows and buffers for command preview.
unsafe fn cmdpreview_prepare() -> CpInfo {
    unsafe {
        let mut saved_bufs: Set<ptr_t> = std::mem::zeroed();
        let mut cpinfo = CpInfo {
            win_info: Vec::new(),
            buf_info: Vec::new(),
            save_hls: false,
            save_cmdmod: std::mem::zeroed(),
            save_view: Vec::new(),
        };

        let mut win = firstwin;
        while !win.is_null() {
            let buf = (*win).w_buffer;

            if (*buf).b_fnum != cmdpreview_bufnr {
                if mh_get(&raw mut saved_bufs, buf.cast::<c_void>()) == u32::MAX {
                    let mut cp_bufinfo: CpBufInfo = std::mem::zeroed();
                    cp_bufinfo.buf = buf;
                    cp_bufinfo.save_b_p_ma = (*buf).b_p_ma;
                    cp_bufinfo.save_b_changed = (*buf).b_changed;
                    cp_bufinfo.save_b_op_start = (*buf).b_op_start;
                    cp_bufinfo.save_b_op_end = (*buf).b_op_end;
                    u_checkpoint(&raw mut cp_bufinfo.undo_checkpoint, buf);
                    cpinfo.buf_info.push(cp_bufinfo);
                    let mut status = MHPutStatus::kMHExisting;
                    mh_put(&raw mut saved_bufs, buf.cast::<c_void>(), &raw mut status);
                }

                let mut cp_wininfo: CpWinInfo = std::mem::zeroed();
                cp_wininfo.win = win;
                cp_wininfo.save_w_cursor = (*win).w_cursor;
                save_viewstate(win, &raw mut cp_wininfo.save_viewstate);
                cp_wininfo.save_w_p_cul = (*win).w_onebuf_opt.wo_cul;
                cp_wininfo.save_w_p_cuc = (*win).w_onebuf_opt.wo_cuc;

                cpinfo.win_info.push(cp_wininfo);

                (*win).w_onebuf_opt.wo_cul = 0;
                (*win).w_onebuf_opt.wo_cuc = 0;
            }

            win = (*win).w_next;
        }

        xfree(saved_bufs.h.hash.cast::<c_void>());
        xfree(saved_bufs.keys.cast::<c_void>());

        cpinfo.save_hls = p_hls != 0;
        cpinfo.save_cmdmod = cmdmod;
        win_size_save(&mut cpinfo.save_view);
        save_search_patterns();

        p_hls = 0;
        cmdmod.cmod_split = 0;
        cmdmod.cmod_tab = 0;
        cmdmod.cmod_flags |= CMOD_NOSWAPFILE;

        u_sync(true);
        cpinfo
    }
}

/// Restore the state of buffers and windows for command preview.
unsafe fn cmdpreview_restore_state(cpinfo: &mut CpInfo) {
    unsafe {
        for mut cp_bufinfo in cpinfo.buf_info.drain(..) {
            let buf = cp_bufinfo.buf;

            (*buf).b_changed = cp_bufinfo.save_b_changed;

            extmark_clear(buf, cmdpreview_ns as u32, 0, 0, MAXLNUM, MAXCOL);

            u_rollback(&raw mut cp_bufinfo.undo_checkpoint, buf);

            (*buf).b_op_start = cp_bufinfo.save_b_op_start;
            (*buf).b_op_end = cp_bufinfo.save_b_op_end;
            (*buf).b_p_ma = cp_bufinfo.save_b_p_ma;
        }

        for mut cp_wininfo in cpinfo.win_info.drain(..) {
            let win = cp_wininfo.win;

            (*win).w_cursor = cp_wininfo.save_w_cursor;
            restore_viewstate(win, &raw mut cp_wininfo.save_viewstate);

            (*win).w_onebuf_opt.wo_cul = cp_wininfo.save_w_p_cul;
            (*win).w_onebuf_opt.wo_cuc = cp_wininfo.save_w_p_cuc;

            update_topline(win);
        }

        cmdmod = cpinfo.save_cmdmod;
        p_hls = c_int::from(cpinfo.save_hls);
        restore_search_patterns();
        win_size_restore(&cpinfo.save_view);
    }
}

/// Show `'inccommand'` preview if command is previewable.
unsafe fn cmdpreview_may_show(_s: *mut CommandLineState) -> bool {
    unsafe {
        let mut ea: exarg_T = std::mem::zeroed();
        let mut cmod: cmdmod_T = std::mem::zeroed();
        let mut cmdpreview_type: c_int;
        let mut cmdline = xstrdup(ccline.cmdbuff);
        let mut errormsg: *const c_char = ptr::null();
        emsg_off += 1;
        let parsed = parse_cmdline(
            &raw mut cmdline,
            &raw mut ea,
            &raw mut cmod,
            &raw mut errormsg,
            false,
        );
        emsg_off -= 1;
        if !parsed {
            xfree(cmdline.cast::<c_void>());
            return false;
        }

        if (ea.argt & EX_PREVIEW) == 0 {
            undo_cmdmod(&raw mut cmod);
            xfree(cmdline.cast::<c_void>());
            return false;
        }

        cursorcmd();
        cmdline_ui_flush();

        if (ea.argt & EX_RANGE) != 0 && ea.line1 > ea.line2 {
            std::mem::swap(&mut ea.line1, &mut ea.line2);
        }

        let mut icm_split = *p_icm == b's' as c_char;
        let mut cmdpreview_buf: *mut buf_T = ptr::null_mut();
        let mut cmdpreview_win: *mut win_T = ptr::null_mut();

        emsg_silent += 1;
        msg_silent += 1;
        block_autocmds();

        let mut cpinfo = cmdpreview_prepare();

        if icm_split {
            cmdpreview_buf = cmdpreview_open_buf();
            if cmdpreview_buf.is_null() {
                set_option_direct(
                    kOptInccommand,
                    cstr_as_obj(c"nosplit".as_ptr()),
                    0,
                    SID_NONE,
                );
                icm_split = false;
            }
        }
        if cmdpreview_ns == 0 {
            cmdpreview_ns = nvim_create_namespace(NvimString {
                data: ptr::null_mut(),
                size: 0,
            }) as c_int;
        }

        cmdpreview = true;

        let mut err = error_init();
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&raw mut tstate);
        cmdpreview_type = execute_cmd(&raw mut ea, &raw mut cmod, true);
        try_leave(&raw const tstate, &raw mut err);
        if error_set(&err) {
            api_clear_error(&raw mut err);
            cmdpreview_type = 0;
        }

        if icm_split && cmdpreview_type == 2 {
            cmdpreview_win = cmdpreview_open_win(cmdpreview_buf);
            if cmdpreview_win.is_null() {
                cmdpreview_type = 1;
            }
        }

        if cmdpreview_type != 0 {
            let save_rd = RedrawingDisabled;
            RedrawingDisabled = 0;
            update_screen();
            RedrawingDisabled = save_rd;
        }

        if icm_split && cmdpreview_type == 2 && !cmdpreview_win.is_null() {
            cmdpreview_close_win();
        }

        cmdpreview_restore_state(&mut cpinfo);

        unblock_autocmds();
        msg_silent -= 1;
        emsg_silent -= 1;
        redrawcmdline();

        xfree(cmdline.cast::<c_void>());
        cmdpreview_type != 0
    }
}

/// Trigger `CmdlineChanged` autocommands.
unsafe fn do_autocmd_cmdlinechanged(firstc: c_int) {
    unsafe {
        if has_event(EVENT_CMDLINECHANGED) {
            let mut err = error_init();
            let mut save_v_event: save_v_event_T = std::mem::zeroed();
            let dict = get_v_event(&raw mut save_v_event);

            let mut firstcbuf: [c_char; 2] = [firstc as c_char, 0];

            tv_dict_add_str(dict, c"cmdtype".as_ptr(), 7, firstcbuf.as_ptr());
            tv_dict_add_nr(dict, c"cmdlevel".as_ptr(), 8, ccline.level as varnumber_T);
            tv_dict_set_keys_readonly(dict);
            let mut tstate: TryState = std::mem::zeroed();
            try_enter(&raw mut tstate);
            apply_autocmds(
                EVENT_CMDLINECHANGED,
                firstcbuf.as_mut_ptr(),
                firstcbuf.as_mut_ptr(),
                false,
                curbuf,
            );
            restore_v_event(dict, &raw mut save_v_event);
            try_leave(&raw const tstate, &raw mut err);
            if error_set(&err) {
                if !ui_has(kUIMessages) {
                    msg_putchar(b'\n' as c_int);
                }
                msg_scroll = 1;
                msg_puts_hl(err.msg, HLF_E, true);
                api_clear_error(&raw mut err);
                redrawcmd();
            }
        }
    }
}

unsafe fn command_line_changed(s: *mut CommandLineState) -> c_int {
    unsafe {
        let prev_cmdpreview = cmdpreview;
        if (*s).firstc == b':' as c_int
            && current_sctx.sc_sid == 0
            && *p_icm != NUL
            && cmdline_star == 0
            && vpeekc_any() == 0
            && cmdpreview_may_show(s)
        {
            // 'inccommand' preview has been shown.
        } else {
            cmdpreview = false;
            if prev_cmdpreview {
                update_screen();
            }
            if (*s).xpc.xp_context == EXPAND_NOTHING && (KeyTyped || vpeekc() == NUL as c_int) {
                may_do_incsearch_highlighting((*s).firstc, (*s).count, &raw mut (*s).is_state);
            }
        }

        if !ccline.cmdbuff_replaced
            && (ccline.cmdpos != (*s).prev_cmdpos
                || (!(*s).prev_cmdbuff.is_null() && strcmp((*s).prev_cmdbuff, ccline.cmdbuff) != 0))
        {
            do_autocmd_cmdlinechanged(if (*s).firstc > 0 {
                (*s).firstc
            } else {
                b'-' as c_int
            });
        }

        may_trigger_cursormovedc(s);

        if p_arshape != 0 && p_tbidi == 0 && !ui_has(kUICmdline) && vpeekc() == NUL as c_int {
            redrawcmd();
        }

        1
    }
}

/// Abandon the command line.
unsafe fn abandon_cmdline() {
    unsafe {
        dealloc_cmdbuff();
        if msg_scrolled == 0 {
            compute_cmdrow();
        }
        if !ccline.one_key {
            msg(c"".as_ptr(), 0);
            redraw_cmdline = true;
        }
    }
}

/// `getcmdline()` - accept a command line starting with `firstc`.
pub unsafe fn getcmdline(
    firstc: c_int,
    count: c_int,
    indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe { command_line_enter(firstc, count, indent, true).cast::<c_char>() }
}

/// Get a command line with a prompt.
pub unsafe fn getcmdline_prompt(
    firstc: c_int,
    prompt: *const c_char,
    hl_id: c_int,
    xp_context: c_int,
    xp_arg: *const c_char,
    highlight_callback: Callback,
    one_key: bool,
    mouse_used: *mut bool,
) -> *mut c_char {
    unsafe {
        let msg_col_save = msg_col;

        let mut save_ccline = CmdlineInfo::new();
        let mut did_save_ccline = false;
        let cc = &raw mut ccline;
        if !(*cc).cmdbuff.is_null() {
            save_cmdline(&raw mut save_ccline);
            did_save_ccline = true;
        } else {
            *cc = CmdlineInfo::new();
        }
        ccline.prompt_id = last_prompt_id;
        last_prompt_id = last_prompt_id.wrapping_add(1);
        ccline.cmdprompt = prompt.cast_mut();
        ccline.hl_id = hl_id;
        ccline.xp_context = xp_context;
        ccline.xp_arg = xp_arg.cast_mut();
        ccline.input_fn = c_int::from(firstc == b'@' as c_int);
        ccline.highlight_callback = highlight_callback;
        ccline.one_key = one_key;
        ccline.mouse_used = mouse_used;

        let cmd_silent_saved = cmd_silent;
        let msg_silent_saved = msg_silent;
        msg_silent = 0;
        cmd_silent = false;

        let ret = command_line_enter(firstc, 1, 0, false).cast::<c_char>();
        ccline.redraw_state = kCmdRedrawNone;

        if did_save_ccline {
            restore_cmdline(&raw mut save_ccline);
        }
        msg_silent = msg_silent_saved;
        cmd_silent = cmd_silent_saved;
        if !ccline.cmdbuff.is_null() {
            msg_col = msg_col_save;
        }

        ret
    }
}

/// Parse `'wildmode'`, updating flags only when non-NULL and the value is valid.
pub unsafe fn check_opt_wim(value: *const c_char, flags: *mut u8) -> c_int {
    unsafe {
        let mut new_wim_flags: [u8; 4] = [0; 4];
        let mut idx: usize = 0;

        let mut p = value;
        while *p != NUL {
            let mut i: usize = 0;
            while ascii_isalpha(*p.add(i) as c_int) {
                i += 1;
            }
            if *p.add(i) != NUL && *p.add(i) != b',' as c_char && *p.add(i) != b':' as c_char {
                return FAIL;
            }
            if i == 7 && strncmp(p, c"longest".as_ptr(), 7) == 0 {
                new_wim_flags[idx] |= kOptWimFlagLongest;
            } else if i == 4 && strncmp(p, c"full".as_ptr(), 4) == 0 {
                new_wim_flags[idx] |= kOptWimFlagFull;
            } else if i == 4 && strncmp(p, c"list".as_ptr(), 4) == 0 {
                new_wim_flags[idx] |= kOptWimFlagList;
            } else if i == 8 && strncmp(p, c"lastused".as_ptr(), 8) == 0 {
                new_wim_flags[idx] |= kOptWimFlagLastused;
            } else if i == 8 && strncmp(p, c"noselect".as_ptr(), 8) == 0 {
                new_wim_flags[idx] |= kOptWimFlagNoselect;
            } else if i == 8 && strncmp(p, c"noinsert".as_ptr(), 8) == 0 {
                new_wim_flags[idx] |= kOptWimFlagNoinsert;
            } else {
                return FAIL;
            }
            p = p.add(i);
            if *p == NUL {
                break;
            }
            if *p == b',' as c_char {
                if idx == 3 {
                    return FAIL;
                }
                idx += 1;
            }
            p = p.add(1);
        }

        while idx < 3 {
            new_wim_flags[idx + 1] = new_wim_flags[idx];
            idx += 1;
        }

        if !flags.is_null() {
            ptr::copy_nonoverlapping(new_wim_flags.as_ptr(), flags, 4);
        }
        OK
    }
}

/// Return true when the text must not be changed and we can't switch to
/// another window or buffer.
pub unsafe fn text_locked() -> bool {
    unsafe {
        if expr_map_locked() {
            return true;
        }
        textlock != 0
    }
}

pub unsafe fn text_locked_msg() {
    unsafe {
        emsg(gettext(get_text_locked_msg()));
    }
}

pub unsafe fn get_text_locked_msg() -> *const c_char {
    e_textlock.as_ptr()
}

/// Check for text, window or buffer locked.
pub unsafe fn text_or_buf_locked() -> bool {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return true;
        }
        curbuf_locked()
    }
}

/// Check if `curbuf->b_ro_locked` or `allbuf_lock` is set.
pub unsafe fn curbuf_locked() -> bool {
    unsafe {
        if (*curbuf).b_ro_locked > 0 {
            emsg(gettext(e_cannot_edit_other_buf.as_ptr()));
            return true;
        }
        allbuf_locked()
    }
}

pub unsafe fn allbuf_locked() -> bool {
    unsafe {
        if allbuf_lock > 0 {
            emsg(gettext(
                c"E811: Not allowed to change buffer information now".as_ptr(),
            ));
            return true;
        }
        false
    }
}

unsafe fn cmdline_charsize(idx: c_int) -> c_int {
    unsafe {
        if cmdline_star > 0 {
            return 1;
        }
        ptr2cells(ccline.cmdbuff.add(idx as usize))
    }
}

/// Compute the offset of the cursor on the command line for the prompt and
/// indent.
unsafe fn cmd_startcol() -> c_int {
    unsafe {
        ccline.cmdindent
            + if ccline.cmdfirstc != NUL as c_int {
                1
            } else {
                0
            }
    }
}

/// Compute the column position for a byte position on the command line.
pub unsafe fn cmd_screencol(bytepos: c_int) -> c_int {
    unsafe {
        let mut m: c_int;
        let mut col = cmd_startcol();
        if KeyTyped {
            m = if !cmdline_win.is_null() {
                (*cmdline_win).w_view_width * (*cmdline_win).w_view_height
            } else {
                Columns * Rows
            };
            if m < 0 {
                m = MAXCOL;
            }
        } else {
            m = MAXCOL;
        }

        let mut i: c_int = 0;
        while i < ccline.cmdlen && i < bytepos {
            let c = cmdline_charsize(i);
            correct_screencol(i, c, &raw mut col);

            col += c;
            if col >= m {
                col -= c;
                break;
            }
            i += utfc_ptr2len(ccline.cmdbuff.add(i as usize));
        }
        col
    }
}

/// Check if the character at `idx`, which is `cells` wide, is a multi-byte
/// character that doesn't fit, so that a `>` must be displayed.
unsafe fn correct_screencol(idx: c_int, cells: c_int, col: *mut c_int) {
    unsafe {
        if utfc_ptr2len(ccline.cmdbuff.add(idx as usize)) > 1
            && utf_ptr2cells(ccline.cmdbuff.add(idx as usize)) > 1
            && (*col) % Columns + cells > Columns
        {
            *col += 1;
        }
    }
}

/// Get an Ex command line for the `:` command.
pub unsafe fn getexline(
    c: c_int,
    _cookie: *mut c_void,
    indent: c_int,
    do_concat: bool,
) -> *mut c_char {
    unsafe {
        if exec_from_reg && vpeekc() == b':' as c_int {
            vgetc();
        }

        getcmdline(c, 1, indent, do_concat)
    }
}

pub unsafe fn cmdline_overstrike() -> bool {
    unsafe { ccline.overstrike != 0 }
}

/// Return true if the cursor is at the end of the cmdline.
pub unsafe fn cmdline_at_end() -> bool {
    unsafe { ccline.cmdpos >= ccline.cmdlen }
}

/// Deallocate a command line buffer, updating the buffer size and length.
unsafe fn dealloc_cmdbuff() {
    unsafe {
        xfree(ccline.cmdbuff.cast::<c_void>());
        ccline.cmdbuff = ptr::null_mut();
        ccline.cmdbufflen = 0;
        ccline.cmdlen = 0;
    }
}

/// Allocate a new command line buffer.
unsafe fn alloc_cmdbuff(mut len: c_int) {
    unsafe {
        if len < 80 {
            len = 100;
        } else {
            len += 20;
        }

        ccline.cmdbuff = xmalloc(len as usize).cast::<c_char>();
        ccline.cmdbufflen = len;
    }
}

/// Re-allocate the command line to length `len` + something extra.
pub unsafe fn realloc_cmdbuff(len: c_int) {
    unsafe {
        if len < ccline.cmdbufflen {
            return;
        }

        let p = ccline.cmdbuff;

        alloc_cmdbuff(len);
        ptr::copy(p, ccline.cmdbuff, ccline.cmdlen as usize);
        *ccline.cmdbuff.add(ccline.cmdlen as usize) = NUL;

        if !ccline.xpc.is_null()
            && !(*ccline.xpc).xp_pattern.is_null()
            && (*ccline.xpc).xp_context != EXPAND_NOTHING
            && (*ccline.xpc).xp_context != EXPAND_UNSUCCESSFUL
        {
            let i = (*ccline.xpc).xp_pattern.offset_from(p) as c_int;

            if i >= 0 && i <= ccline.cmdlen {
                (*ccline.xpc).xp_pattern = ccline.cmdbuff.add(i as usize);
            }
        }

        xfree(p.cast::<c_void>());
    }
}

/// Color expression cmdline using built-in expressions parser.
unsafe fn color_expr_cmdline(
    colored_ccline: *const CmdlineInfo,
    ret_ccline_colors: *mut ColoredCmdline,
) {
    unsafe {
        let parser_lines = [
            ParserLine {
                data: (*colored_ccline).cmdbuff,
                size: strlen((*colored_ccline).cmdbuff),
                allocated: false,
            },
            ParserLine {
                data: ptr::null(),
                size: 0,
                allocated: false,
            },
        ];
        let mut plines_p = parser_lines.as_ptr();
        let mut colors = ParserHighlight::new();
        let mut pstate = ParserState {
            reader: ParserInputReader {
                get_line: Some(parser_simple_get_line),
                cookie: (&raw mut plines_p).cast::<c_void>(),
                lines: Vec::new(),
                conv: vimconv_T {
                    vc_type: CONV_NONE,
                    vc_factor: 1,
                    vc_fd: ptr::null_mut(),
                    vc_fail: false,
                },
            },
            pos: ParserPosition { line: 0, col: 0 },
            colors: &raw mut colors,
        };
        let east: ExprAST = viml_pexpr_parse(&raw mut pstate, kExprFlagsDisallowEOC);
        viml_pexpr_free_ast(east);
        viml_parser_destroy(&raw mut pstate);

        (*ret_ccline_colors).colors.reserve(colors.len());
        let mut prev_end: usize = 0;
        for chunk in colors {
            debug_assert!(chunk.start.col < c_int::MAX as usize);
            debug_assert!(chunk.end_col < c_int::MAX as usize);
            if chunk.start.col != prev_end {
                (*ret_ccline_colors).colors.push(CmdlineColorChunk {
                    start: prev_end as c_int,
                    end: chunk.start.col as c_int,
                    hl_id: 0,
                });
            }
            (*ret_ccline_colors).colors.push(CmdlineColorChunk {
                start: chunk.start.col as c_int,
                end: chunk.end_col as c_int,
                hl_id: syn_name2id(chunk.group),
            });
            prev_end = chunk.end_col;
        }
        if prev_end < (*colored_ccline).cmdlen as usize {
            (*ret_ccline_colors).colors.push(CmdlineColorChunk {
                start: prev_end as c_int,
                end: (*colored_ccline).cmdlen,
                hl_id: 0,
            });
        }
    }
}

/// Color command-line.
unsafe fn color_cmdline(colored_ccline: *mut CmdlineInfo) -> bool {
    unsafe {
        let mut printed_errmsg = false;
        let mut ret = true;

        let ccline_colors = &raw mut (*colored_ccline).last_colors;

        if (*ccline_colors).prompt_id == (*colored_ccline).prompt_id
            && !(*ccline_colors).cmdbuff.is_null()
            && strcmp((*ccline_colors).cmdbuff, (*colored_ccline).cmdbuff) == 0
        {
            return ret;
        }

        (*ccline_colors).colors.clear();

        if (*colored_ccline).cmdbuff.is_null() || *(*colored_ccline).cmdbuff == NUL {
            xfree((*ccline_colors).cmdbuff.cast::<c_void>());
            (*ccline_colors).cmdbuff = ptr::null_mut();
            return ret;
        }

        let mut arg_allocated = false;
        let mut arg = typval_T {
            v_type: VarType::VAR_STRING,
            v_lock: 0,
            vval: TypvalVval {
                v_string: (*colored_ccline).cmdbuff,
            },
        };
        let mut tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        };

        let mut color_cb = Callback {
            data: ptr::null_mut(),
            cb_type: kCallbackNone,
        };
        let mut can_free_cb = false;
        let mut err = error_init();
        let mut err_errmsg: *const c_char = e_intern2.as_ptr();
        let mut dgc_ret = true;

        'color_cmdline_end: {
            'color_cmdline_error: {
                if (*colored_ccline).prompt_id != prev_prompt_id {
                    prev_prompt_errors = 0;
                    prev_prompt_id = (*colored_ccline).prompt_id;
                } else if prev_prompt_errors >= MAX_CB_ERRORS {
                    break 'color_cmdline_end;
                }
                if (*colored_ccline).highlight_callback.cb_type != kCallbackNone {
                    debug_assert!((*colored_ccline).input_fn != 0);
                    color_cb = (*colored_ccline).highlight_callback;
                } else if (*colored_ccline).cmdfirstc == b':' as c_int {
                    let mut tstate: TryState = std::mem::zeroed();
                    try_enter(&raw mut tstate);
                    err_errmsg = c"E5408: Unable to get g:Nvim_color_cmdline callback: %s".as_ptr();
                    dgc_ret = tv_dict_get_callback(
                        get_globvar_dict(),
                        c"Nvim_color_cmdline".as_ptr(),
                        18,
                        &raw mut color_cb,
                    );
                    try_leave(&raw const tstate, &raw mut err);
                    can_free_cb = true;
                } else if (*colored_ccline).cmdfirstc == b'=' as c_int {
                    color_expr_cmdline(colored_ccline, ccline_colors);
                }
                if error_set(&err) || !dgc_ret {
                    break 'color_cmdline_error;
                }

                if color_cb.cb_type == kCallbackNone {
                    break 'color_cmdline_end;
                }
                if *(*colored_ccline)
                    .cmdbuff
                    .add((*colored_ccline).cmdlen as usize)
                    != NUL
                {
                    arg_allocated = true;
                    arg.vval.v_string = xmemdupz(
                        (*colored_ccline).cmdbuff.cast::<c_void>(),
                        (*colored_ccline).cmdlen as usize,
                    )
                    .cast::<c_char>();
                }

                getln_interrupted_highlight = false;
                let mut tstate: TryState = std::mem::zeroed();
                try_enter(&raw mut tstate);
                err_errmsg = c"E5407: Callback has thrown an exception: %s".as_ptr();
                let saved_msg_col = msg_col;
                msg_silent += 1;
                let cbcall_ret = callback_call(&raw mut color_cb, 1, &raw mut arg, &raw mut tv);
                msg_silent -= 1;
                msg_col = saved_msg_col;
                if got_int {
                    getln_interrupted_highlight = true;
                }
                try_leave(&raw const tstate, &raw mut err);
                if error_set(&err) || !cbcall_ret {
                    break 'color_cmdline_error;
                }
                if tv.v_type != VarType::VAR_LIST {
                    msg_scroll = 1;
                    msg_putchar(b'\n' as c_int);
                    smsg!(
                        HLF_E,
                        c"%s".as_ptr(),
                        gettext(c"E5400: Callback should return list".as_ptr()),
                    );
                    printed_errmsg = true;
                    break 'color_cmdline_error;
                }
                let v_list = tv.vval.v_list;
                if v_list.is_null() {
                    break 'color_cmdline_end;
                }
                let mut prev_end: varnumber_T = 0;
                let mut i: c_int = 0;
                let mut li = tv_list_first(v_list);
                while !li.is_null() {
                    let li_tv = &raw mut (*li).li_tv;
                    if (*li_tv).v_type != VarType::VAR_LIST {
                        msg_scroll = 1;
                        msg_putchar(b'\n' as c_int);
                        smsg!(
                            HLF_E,
                            gettext(c"E5401: List item %i is not a List".as_ptr()),
                            i,
                        );
                        printed_errmsg = true;
                        break 'color_cmdline_error;
                    }
                    let l = (*li_tv).vval.v_list;
                    if tv_list_len(l) != 3 {
                        msg_scroll = 1;
                        msg_putchar(b'\n' as c_int);
                        smsg!(
                            HLF_E,
                            gettext(c"E5402: List item %i has incorrect length: %d /= 3".as_ptr()),
                            i,
                            tv_list_len(l),
                        );
                        printed_errmsg = true;
                        break 'color_cmdline_error;
                    }
                    let mut error = false;
                    let first_li = tv_list_first(l);
                    let start = tv_get_number_chk(&raw const (*first_li).li_tv, &raw mut error);
                    if error {
                        break 'color_cmdline_error;
                    } else if !(prev_end <= start
                        && start < (*colored_ccline).cmdlen as varnumber_T)
                    {
                        msg_scroll = 1;
                        msg_putchar(b'\n' as c_int);
                        smsg!(
                            HLF_E,
                            gettext(c"E5403: Chunk %i start %lld not in range [%lld, %i)".as_ptr()),
                            i,
                            start,
                            prev_end,
                            (*colored_ccline).cmdlen,
                        );
                        printed_errmsg = true;
                        break 'color_cmdline_error;
                    } else if utf8len_tab_zero
                        [*(*colored_ccline).cmdbuff.add(start as usize) as u8 as usize]
                        == 0
                    {
                        msg_scroll = 1;
                        msg_putchar(b'\n' as c_int);
                        smsg!(
                            HLF_E,
                            gettext(
                                c"E5405: Chunk %i start %lld splits multibyte character".as_ptr(),
                            ),
                            i,
                            start,
                        );
                        printed_errmsg = true;
                        break 'color_cmdline_error;
                    }
                    if start != prev_end {
                        (*ccline_colors).colors.push(CmdlineColorChunk {
                            start: prev_end as c_int,
                            end: start as c_int,
                            hl_id: 0,
                        });
                    }
                    let second_li = (*first_li).li_next;
                    let end = tv_get_number_chk(&raw const (*second_li).li_tv, &raw mut error);
                    if error {
                        break 'color_cmdline_error;
                    } else if !(start < end && end <= (*colored_ccline).cmdlen as varnumber_T) {
                        msg_scroll = 1;
                        msg_putchar(b'\n' as c_int);
                        smsg!(
                            HLF_E,
                            gettext(c"E5404: Chunk %i end %lld not in range (%lld, %i]".as_ptr()),
                            i,
                            end,
                            start,
                            (*colored_ccline).cmdlen,
                        );
                        printed_errmsg = true;
                        break 'color_cmdline_error;
                    } else if end < (*colored_ccline).cmdlen as varnumber_T
                        && utf8len_tab_zero
                            [*(*colored_ccline).cmdbuff.add(end as usize) as u8 as usize]
                            == 0
                    {
                        msg_scroll = 1;
                        msg_putchar(b'\n' as c_int);
                        smsg!(
                            HLF_E,
                            gettext(
                                c"E5406: Chunk %i end %lld splits multibyte character".as_ptr(),
                            ),
                            i,
                            end,
                        );
                        printed_errmsg = true;
                        break 'color_cmdline_error;
                    }
                    prev_end = end;
                    let last_li = tv_list_last(l);
                    let group = tv_get_string_chk(&raw const (*last_li).li_tv);
                    if group.is_null() {
                        break 'color_cmdline_error;
                    }
                    (*ccline_colors).colors.push(CmdlineColorChunk {
                        start: start as c_int,
                        end: end as c_int,
                        hl_id: syn_name2id(group),
                    });
                    i += 1;
                    li = (*li).li_next;
                }
                if prev_end < (*colored_ccline).cmdlen as varnumber_T {
                    (*ccline_colors).colors.push(CmdlineColorChunk {
                        start: prev_end as c_int,
                        end: (*colored_ccline).cmdlen,
                        hl_id: 0,
                    });
                }
                prev_prompt_errors = 0;
                break 'color_cmdline_end;
            }

            // color_cmdline_error:
            if error_set(&err) {
                msg_scroll = 1;
                msg_putchar(b'\n' as c_int);
                smsg!(HLF_E, gettext(err_errmsg), err.msg);
                printed_errmsg = true;
                api_clear_error(&raw mut err);
            }
            let _ = printed_errmsg;

            prev_prompt_errors += 1;
            (*ccline_colors).colors.clear();
            redrawcmdline();
            ret = false;
        }

        // color_cmdline_end:
        debug_assert!(!error_set(&err));
        if can_free_cb {
            callback_free(&raw mut color_cb);
        }
        xfree((*ccline_colors).cmdbuff.cast::<c_void>());
        (*ccline_colors).prompt_id = (*colored_ccline).prompt_id;
        if arg_allocated {
            (*ccline_colors).cmdbuff = arg.vval.v_string;
        } else {
            (*ccline_colors).cmdbuff = xmemdupz(
                (*colored_ccline).cmdbuff.cast::<c_void>(),
                (*colored_ccline).cmdlen as usize,
            )
            .cast::<c_char>();
        }
        tv_clear(&raw mut tv);
        ret
    }
}

/// Draw part of the cmdline at the current cursor position.
unsafe fn draw_cmdline(start: c_int, len: c_int) {
    unsafe {
        if ccline.cmdbuff.is_null() || !color_cmdline(&raw mut ccline) {
            return;
        }

        if ui_has(kUICmdline) {
            ccline.special_char = NUL;
            ccline.redraw_state = kCmdRedrawAll;
            return;
        }

        let lc_colors = &raw const ccline.last_colors.colors;
        if cmdline_star > 0 {
            let mut i: c_int = 0;
            while i < len {
                msg_putchar(b'*' as c_int);
                i += utfc_ptr2len(ccline.cmdbuff.add((start + i) as usize)) - 1;
                i += 1;
            }
        } else if !(*lc_colors).is_empty() {
            for &chunk in &*lc_colors {
                if chunk.end <= start {
                    continue;
                }
                let chunk_start = chunk.start.max(start);
                msg_outtrans_len(
                    ccline.cmdbuff.add(chunk_start as usize),
                    chunk.end - chunk_start,
                    chunk.hl_id,
                    false,
                );
            }
        } else {
            msg_outtrans_len(ccline.cmdbuff.add(start as usize), len, 0, false);
        }
    }
}

unsafe fn ui_ext_cmdline_show(line: *mut CmdlineInfo) {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut content: Array;
        if cmdline_star != 0 {
            content = arena_array(&raw mut arena, 1);
            let mut len: usize = 0;
            let mut p = ccline.cmdbuff;
            while *p != NUL {
                p = p.add(utfc_ptr2len(p) as usize);
                len += 1;
            }
            let buf = arena_alloc(&raw mut arena, len, false).cast::<c_char>();
            ptr::write_bytes(buf.cast::<u8>(), b'*', len);
            let mut item = arena_array(&raw mut arena, 3);
            add_c(&mut item, integer_obj(0));
            add_c(&mut item, string_obj(cbuf_as_string(buf, len)));
            add_c(&mut item, integer_obj(0));
            add_c(&mut content, array_obj(item));
        } else if !(*line).last_colors.colors.is_empty() {
            content = arena_array(&raw mut arena, (*line).last_colors.colors.len());
            for &chunk in &(*line).last_colors.colors {
                let mut item = arena_array(&raw mut arena, 3);
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
                        (*line).cmdbuff.add(chunk.start as usize),
                        (chunk.end - chunk.start) as usize,
                    )),
                );
                add_c(&mut item, integer_obj(chunk.hl_id as Integer));
                add_c(&mut content, array_obj(item));
            }
        } else {
            let mut item = arena_array(&raw mut arena, 3);
            add_c(&mut item, integer_obj(0));
            add_c(&mut item, cstr_as_obj((*line).cmdbuff));
            add_c(&mut item, integer_obj(0));
            content = arena_array(&raw mut arena, 1);
            add_c(&mut content, array_obj(item));
        }
        let mut charbuf: [c_char; 2] = [(*line).cmdfirstc as c_char, 0];
        ui_call_cmdline_show(
            content,
            (*line).cmdpos as Integer,
            cstr_as_string(charbuf.as_ptr()),
            cstr_as_string((*line).cmdprompt),
            (*line).cmdindent as Integer,
            (*line).level as Integer,
            if !(*line).cmdprompt.is_null() {
                (*line).hl_id as Integer
            } else {
                -1
            },
        );
        if (*line).special_char != NUL {
            charbuf[0] = (*line).special_char;
            ui_call_cmdline_special_char(
                cstr_as_string(charbuf.as_ptr()),
                (*line).special_shift,
                (*line).level as Integer,
            );
        }
        arena_mem_free(arena_finish(&raw mut arena));
    }
}

pub unsafe fn ui_ext_cmdline_block_append(indent: usize, line: *const c_char) {
    unsafe {
        let line_len = strlen(line);
        let buf = xmallocz(indent + line_len).cast::<c_char>();
        ptr::write_bytes(buf.cast::<u8>(), b' ', indent);
        ptr::copy_nonoverlapping(line, buf.add(indent), line_len);

        let mut item = Array {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        array_add(&mut item, integer_obj(0));
        array_add(&mut item, cstr_as_obj(buf));
        array_add(&mut item, integer_obj(0));
        let mut content = Array {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        array_add(&mut content, array_obj(item));
        let block_ptr = &raw mut cmdline_block;
        array_add(&mut *block_ptr, array_obj(content));
        if (*block_ptr).size > 1 {
            ui_call_cmdline_block_append(content);
        } else {
            ui_call_cmdline_block_show(*block_ptr);
        }
    }
}

pub unsafe fn ui_ext_cmdline_block_leave() {
    unsafe {
        api_free_array(cmdline_block);
        cmdline_block = Array {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        ui_call_cmdline_block_hide();
    }
}

/// Extra redrawing needed for `redraw!` and on `ui_attach`.
pub unsafe fn cmdline_screen_cleared() {
    unsafe {
        if !ui_has(kUICmdline) {
            return;
        }

        if cmdline_block.size != 0 {
            ui_call_cmdline_block_show(cmdline_block);
        }

        let mut prev_level = ccline.level - 1;
        let mut line = ccline.prev_ccline;
        while prev_level > 0 && !line.is_null() {
            if (*line).level == prev_level {
                (*line).redraw_state = kCmdRedrawAll;
                prev_level -= 1;
            }
            line = (*line).prev_ccline;
        }
        redrawcmd();
    }
}

/// Called by `ui_flush`, do what redraws necessary to keep cmdline updated.
pub unsafe fn cmdline_ui_flush() {
    unsafe {
        if !ui_has(kUICmdline) {
            return;
        }
        let mut level = ccline.level;
        let mut line: *mut CmdlineInfo = &raw mut ccline;
        while level > 0 && !line.is_null() {
            if (*line).level == level {
                let redraw_state = (*line).redraw_state;
                (*line).redraw_state = kCmdRedrawNone;
                if redraw_state == kCmdRedrawAll {
                    cmdline_was_last_drawn = true;
                    ui_ext_cmdline_show(line);
                } else if redraw_state == kCmdRedrawPos && cmdline_was_last_drawn {
                    ui_call_cmdline_pos((*line).cmdpos as Integer, (*line).level as Integer);
                }
                level -= 1;
            }
            line = (*line).prev_ccline;
        }
    }
}

/// Put a character on the command line.
pub unsafe fn putcmdline(c: c_char, shift: bool) {
    unsafe {
        if cmd_silent {
            return;
        }
        if !ui_has(kUICmdline) {
            msg_no_more = true;
            msg_putchar((c as u8) as c_int);
            if shift {
                draw_cmdline(ccline.cmdpos, ccline.cmdlen - ccline.cmdpos);
            }
            msg_no_more = false;
        } else if ccline.redraw_state != kCmdRedrawAll {
            let charbuf: [c_char; 2] = [c, 0];
            ui_call_cmdline_special_char(
                cstr_as_string(charbuf.as_ptr()),
                shift,
                ccline.level as Integer,
            );
        }
        cursorcmd();
        ccline.special_char = c;
        ccline.special_shift = shift;
        ui_cursor_shape();
    }
}

/// Undo a `putcmdline(c, false)`.
pub unsafe fn unputcmdline() {
    unsafe {
        if cmd_silent {
            return;
        }
        msg_no_more = true;
        if ccline.cmdlen == ccline.cmdpos && !ui_has(kUICmdline) {
            msg_putchar(b' ' as c_int);
        } else {
            draw_cmdline(
                ccline.cmdpos,
                utfc_ptr2len(ccline.cmdbuff.add(ccline.cmdpos as usize)),
            );
        }
        msg_no_more = false;
        cursorcmd();
        ccline.special_char = NUL;
        ui_cursor_shape();
    }
}

/// Put the given string, of the given length, onto the command line.
pub unsafe fn put_on_cmdline(str: *const c_char, mut len: c_int, redraw: bool) {
    unsafe {
        if len < 0 {
            len = strlen(str) as c_int;
        }

        realloc_cmdbuff(ccline.cmdlen + len + 1);

        if ccline.overstrike == 0 {
            ptr::copy(
                ccline.cmdbuff.add(ccline.cmdpos as usize),
                ccline.cmdbuff.add((ccline.cmdpos + len) as usize),
                (ccline.cmdlen - ccline.cmdpos) as usize,
            );
            ccline.cmdlen += len;
        } else {
            let mut m: c_int = 0;
            let mut i: c_int = 0;
            while i < len {
                m += 1;
                i += utfc_ptr2len(str.add(i as usize));
            }
            i = ccline.cmdpos;
            while i < ccline.cmdlen && m > 0 {
                m -= 1;
                i += utfc_ptr2len(ccline.cmdbuff.add(i as usize));
            }
            if i < ccline.cmdlen {
                ptr::copy(
                    ccline.cmdbuff.add(i as usize),
                    ccline.cmdbuff.add((ccline.cmdpos + len) as usize),
                    (ccline.cmdlen - i) as usize,
                );
                ccline.cmdlen += ccline.cmdpos + len - i;
            } else {
                ccline.cmdlen = ccline.cmdpos + len;
            }
        }
        ptr::copy(
            str,
            ccline.cmdbuff.add(ccline.cmdpos as usize),
            len as usize,
        );
        *ccline.cmdbuff.add(ccline.cmdlen as usize) = NUL;

        if ccline.cmdpos > 0 && (*ccline.cmdbuff.add(ccline.cmdpos as usize) as u8) >= 0x80 {
            let i = utf_head_off(ccline.cmdbuff, ccline.cmdbuff.add(ccline.cmdpos as usize));
            if i != 0 {
                ccline.cmdpos -= i;
                len += i;
                ccline.cmdspos = cmd_screencol(ccline.cmdpos);
            }
        }

        if redraw && !cmd_silent {
            msg_no_more = true;
            let i = cmdline_row;
            cursorcmd();
            draw_cmdline(ccline.cmdpos, ccline.cmdlen - ccline.cmdpos);
            if cmdline_row != i || ccline.overstrike != 0 {
                msg_clr_eos();
            }
            msg_no_more = false;
        }
        let mut m: c_int;
        if KeyTyped {
            m = Columns * Rows;
            if m < 0 {
                m = MAXCOL;
            }
        } else {
            m = MAXCOL;
        }
        let mut i: c_int = 0;
        while i < len {
            let mut c = cmdline_charsize(ccline.cmdpos);
            correct_screencol(ccline.cmdpos, c, &raw mut ccline.cmdspos);
            if ccline.cmdspos + c < m {
                ccline.cmdspos += c;
            }
            c = utfc_ptr2len(ccline.cmdbuff.add(ccline.cmdpos as usize)) - 1;
            c = c.min(len - i - 1);
            ccline.cmdpos += c;
            i += c;
            ccline.cmdpos += 1;
            i += 1;
        }

        if redraw {
            msg_check();
        }
    }
}

/// Save `ccline`.
unsafe fn save_cmdline(ccp: *mut CmdlineInfo) {
    unsafe {
        let cc = &raw mut ccline;
        *ccp = std::mem::replace(&mut *cc, CmdlineInfo::new());
        (*cc).prev_ccline = ccp;
    }
}

/// Restore `ccline` after it has been saved with `save_cmdline()`.
unsafe fn restore_cmdline(ccp: *mut CmdlineInfo) {
    unsafe {
        let cc = &raw mut ccline;
        *cc = std::mem::replace(&mut *ccp, CmdlineInfo::new());
    }
}

/// Paste a yank register into the command line.
unsafe fn cmdline_paste(regname: c_int, literally: bool, remcr: bool) -> bool {
    unsafe {
        let mut arg: *mut c_char = ptr::null_mut();
        let mut allocated = false;

        if regname != Ctrl_F
            && regname != Ctrl_P
            && regname != Ctrl_W
            && regname != Ctrl_A
            && regname != Ctrl_L
            && !valid_yank_reg(regname, false)
        {
            return FAIL != 0;
        }

        line_breakcheck();
        if got_int {
            return FAIL != 0;
        }

        textlock += 1;
        let i = get_spec_reg(regname, &raw mut arg, &raw mut allocated, true);
        textlock -= 1;

        if i {
            if arg.is_null() {
                return FAIL != 0;
            }

            let mut p = arg;
            if p_is != 0 && regname == Ctrl_W {
                let mut w = ccline.cmdbuff.add(ccline.cmdpos as usize);
                while w > ccline.cmdbuff {
                    let len = utf_head_off(ccline.cmdbuff, w.sub(1)) + 1;
                    if !vim_iswordc(utf_ptr2char(w.sub(len as usize))) {
                        break;
                    }
                    w = w.sub(len as usize);
                }
                let len = ccline.cmdbuff.add(ccline.cmdpos as usize).offset_from(w) as usize;
                let matches = if p_ic != 0 {
                    vim_strnicmp(w, arg, len) == 0
                } else {
                    strncmp(w, arg, len) == 0
                };
                if matches {
                    p = p.add(len);
                }
            }

            cmdline_paste_str(p, literally);
            if allocated {
                xfree(arg.cast::<c_void>());
            }
            return OK != 0;
        }

        cmdline_paste_reg(regname, literally, remcr)
    }
}

/// Put a string on the command line.
pub unsafe fn cmdline_paste_str(mut s: *const c_char, literally: bool) {
    unsafe {
        if literally {
            put_on_cmdline(s, -1, true);
        } else {
            while *s != NUL {
                let cv = (*s as u8) as c_int;
                if cv == Ctrl_V && *s.add(1) != NUL {
                    s = s.add(1);
                }
                let c = mb_cptr2char_adv(&raw mut s);
                if cv == Ctrl_V
                    || c == ESC
                    || c == Ctrl_C
                    || c == CAR
                    || c == NL
                    || c == Ctrl_L
                    || (c == Ctrl_BSL && *s as c_int == Ctrl_N)
                {
                    stuffcharReadbuff(Ctrl_V);
                }
                stuffcharReadbuff(c);
            }
        }
    }
}

pub unsafe fn redrawcmdline() {
    unsafe {
        if cmd_silent {
            return;
        }
        need_wait_return = false;
        compute_cmdrow();
        redrawcmd();
        cursorcmd();
        ui_cursor_shape();
    }
}

unsafe fn redrawcmdprompt() {
    unsafe {
        if cmd_silent {
            return;
        }
        if ui_has(kUICmdline) {
            ccline.redraw_state = kCmdRedrawAll;
            return;
        }
        if ccline.cmdfirstc != NUL as c_int {
            msg_putchar(ccline.cmdfirstc);
        }
        if !ccline.cmdprompt.is_null() {
            msg_puts_hl(ccline.cmdprompt, ccline.hl_id, false);
            ccline.cmdindent = msg_col + (msg_row - cmdline_row) * Columns;
            if ccline.cmdfirstc != NUL as c_int {
                ccline.cmdindent -= 1;
            }
        } else {
            let mut i = ccline.cmdindent;
            while i > 0 {
                msg_putchar(b' ' as c_int);
                i -= 1;
            }
        }
    }
}

/// Redraw what is currently on the command line.
pub unsafe fn redrawcmd() {
    unsafe {
        if cmd_silent {
            return;
        }

        if ui_has(kUICmdline) {
            draw_cmdline(0, ccline.cmdlen);
            return;
        }

        if ccline.cmdbuff.is_null() {
            msg_cursor_goto(cmdline_row, 0);
            msg_clr_eos();
            return;
        }

        redrawing_cmdline = true;

        sb_text_restart_cmdline();
        msg_start();
        msg_starthere();
        redrawcmdprompt();

        msg_no_more = true;
        draw_cmdline(0, ccline.cmdlen);
        msg_clr_eos();
        msg_no_more = false;

        ccline.cmdspos = cmd_screencol(ccline.cmdpos);

        if ccline.special_char != NUL {
            putcmdline(ccline.special_char, ccline.special_shift);
        }

        msg_scroll = 0;
        skip_redraw = false;
        cmdline_was_last_drawn = true;
        redrawing_cmdline = false;
    }
}

pub unsafe fn compute_cmdrow() {
    unsafe {
        if msg_scrolled != 0 {
            cmdline_row = Rows - 1;
        } else {
            let wp = lastwin_nofloating(ptr::null_mut());
            cmdline_row = (*wp).w_winrow
                + (*wp).w_height
                + (*wp).w_hsep_height
                + (*wp).w_status_height
                + global_stl_height();
        }
        if cmdline_row == Rows && p_ch > 0 {
            cmdline_row -= 1;
        }
        lines_left = cmdline_row;
    }
}

pub unsafe fn cursorcmd() {
    unsafe {
        if cmd_silent || ui_has(kUICmdline) {
            return;
        }

        msg_row = cmdline_row + (ccline.cmdspos / Columns);
        msg_col = ccline.cmdspos % Columns;
        msg_row = msg_row.min(Rows - 1);

        msg_cursor_goto(msg_row, msg_col);
    }
}

pub unsafe fn gotocmdline(clr: bool) {
    unsafe {
        if ui_has(kUICmdline) {
            return;
        }
        msg_start();
        msg_col = 0;
        if clr {
            msg_clr_eos();
        }
        msg_cursor_goto(cmdline_row, 0);
    }
}

/// Check the word in front of the cursor for an abbreviation.
unsafe fn ccheck_abbr(c: c_int) -> c_int {
    unsafe {
        let mut spos: c_int = 0;

        if p_paste != 0 || no_abbr {
            return 0;
        }

        while spos < ccline.cmdlen && ascii_iswhite(*ccline.cmdbuff.add(spos as usize) as c_int) {
            spos += 1;
        }
        if ccline.cmdlen - spos > 5
            && *ccline.cmdbuff.add(spos as usize) == b'\'' as c_char
            && *ccline.cmdbuff.add((spos + 2) as usize) == b',' as c_char
            && *ccline.cmdbuff.add((spos + 3) as usize) == b'\'' as c_char
        {
            spos += 5;
        } else {
            spos = 0;
        }

        c_int::from(check_abbr(c, ccline.cmdbuff, ccline.cmdpos, spos))
    }
}

/// Escape special characters in `fname`, depending on `what`.
pub unsafe fn vim_strsave_fnameescape(fname: *const c_char, what: c_int) -> *mut c_char {
    unsafe {
        #[cfg(windows)]
        let mut p = {
            let mut buf: [c_char; 32] = [0; 32];
            let mut j: usize = 0;
            let chars = if what == VSE_BUFFER {
                BUFFER_ESC_CHARS.as_ptr()
            } else {
                PATH_ESC_CHARS.as_ptr()
            };
            let mut cp = chars;
            while *cp != NUL {
                if (*cp != b'[' as c_char && *cp != b'{' as c_char && *cp != b'!' as c_char)
                    || !vim_isfilec((*cp as u8) as c_int)
                {
                    buf[j] = *cp;
                    j += 1;
                }
                cp = cp.add(1);
            }
            buf[j] = NUL;
            vim_strsave_escaped(fname, buf.as_ptr())
        };

        #[cfg(not(windows))]
        let mut p = {
            let chars = if what == VSE_SHELL {
                SHELL_ESC_CHARS.as_ptr()
            } else if what == VSE_BUFFER {
                BUFFER_ESC_CHARS.as_ptr()
            } else {
                PATH_ESC_CHARS.as_ptr()
            };
            let mut res = vim_strsave_escaped(fname, chars);
            if what == VSE_SHELL && csh_like_shell() != 0 {
                let s = vim_strsave_escaped(res, c"!".as_ptr());
                xfree(res.cast::<c_void>());
                res = s;
            }
            res
        };

        if *p == b'>' as c_char
            || *p == b'+' as c_char
            || (*p == b'-' as c_char && *p.add(1) == NUL)
        {
            escape_fname(&raw mut p);
        }

        p
    }
}

/// Put a backslash before the file name in `pp`, which is in allocated memory.
pub unsafe fn escape_fname(pp: *mut *mut c_char) {
    unsafe {
        let p = xmalloc(strlen(*pp) + 2).cast::<c_char>();
        *p = b'\\' as c_char;
        strcpy(p.add(1), *pp);
        xfree((*pp).cast::<c_void>());
        *pp = p;
    }
}

/// For each file name in `files[num_files]`:
/// If `orig_pat` starts with `~/`, replace the home directory with `~`.
pub unsafe fn tilde_replace(orig_pat: *mut c_char, num_files: c_int, files: *mut *mut c_char) {
    unsafe {
        if *orig_pat == b'~' as c_char && vim_ispathsep((*orig_pat.add(1) as u8) as c_int) {
            for i in 0..(num_files as usize) {
                let p = home_replace_save(ptr::null_mut(), *files.add(i));
                xfree((*files.add(i)).cast::<c_void>());
                *files.add(i) = p;
            }
        }
    }
}

/// Get a pointer to the current command line info.
pub unsafe fn get_cmdline_info() -> *mut CmdlineInfo {
    &raw mut ccline
}

pub unsafe fn get_cmdline_last_prompt_id() -> c_uint {
    unsafe { last_prompt_id }
}

/// Get pointer to the command line info to use.
unsafe fn get_ccline_ptr() -> *mut CmdlineInfo {
    unsafe {
        if (State & MODE_CMDLINE) == 0 {
            ptr::null_mut()
        } else if !ccline.cmdbuff.is_null() {
            &raw mut ccline
        } else if !ccline.prev_ccline.is_null() && !(*ccline.prev_ccline).cmdbuff.is_null() {
            ccline.prev_ccline
        } else {
            ptr::null_mut()
        }
    }
}

/// Get the current command-line type.
unsafe fn get_cmdline_type() -> c_int {
    unsafe {
        let p = get_ccline_ptr();

        if p.is_null() {
            return NUL as c_int;
        }
        if (*p).cmdfirstc == NUL as c_int {
            return if (*p).input_fn != 0 {
                b'@' as c_int
            } else {
                b'-' as c_int
            };
        }
        (*p).cmdfirstc
    }
}

/// Get the current command line in allocated memory.
unsafe fn get_cmdline_str() -> *mut c_char {
    unsafe {
        if cmdline_star > 0 {
            return ptr::null_mut();
        }
        let p = get_ccline_ptr();

        if p.is_null() {
            return ptr::null_mut();
        }
        xstrnsave((*p).cmdbuff, (*p).cmdlen as usize)
    }
}

/// Get the current command-line completion pattern.
unsafe fn get_cmdline_completion_pattern() -> *mut c_char {
    unsafe {
        if cmdline_star > 0 {
            return ptr::null_mut();
        }

        let p = get_ccline_ptr();
        if p.is_null() || (*p).xpc.is_null() {
            return ptr::null_mut();
        }

        let mut xp_context = (*(*p).xpc).xp_context;
        if xp_context == EXPAND_NOTHING {
            set_expand_context((*p).xpc);
            xp_context = (*(*p).xpc).xp_context;
            (*(*p).xpc).xp_context = EXPAND_NOTHING;
        }
        if xp_context == EXPAND_UNSUCCESSFUL {
            return ptr::null_mut();
        }

        let compl_pat = (*(*p).xpc).xp_pattern;
        if compl_pat.is_null() {
            return ptr::null_mut();
        }

        xstrdup(compl_pat)
    }
}

/// Get the command-line completion type.
unsafe fn get_cmdline_completion() -> *mut c_char {
    unsafe {
        if cmdline_star > 0 {
            return ptr::null_mut();
        }

        let p = get_ccline_ptr();
        if p.is_null() || (*p).xpc.is_null() {
            return ptr::null_mut();
        }

        let mut xp_context = (*(*p).xpc).xp_context;
        if xp_context == EXPAND_NOTHING {
            set_expand_context((*p).xpc);
            xp_context = (*(*p).xpc).xp_context;
            (*(*p).xpc).xp_context = EXPAND_NOTHING;
        }
        if xp_context == EXPAND_UNSUCCESSFUL {
            return ptr::null_mut();
        }

        cmdcomplete_type_to_str(xp_context, (*(*p).xpc).xp_arg)
    }
}

/// `"getcmdcomplpat()"` function.
pub unsafe fn f_getcmdcomplpat(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = get_cmdline_completion_pattern();
    }
}

/// `"getcmdcompltype()"` function.
pub unsafe fn f_getcmdcompltype(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = get_cmdline_completion();
    }
}

/// `"getcmdline()"` function.
pub unsafe fn f_getcmdline(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = get_cmdline_str();
    }
}

/// `"getcmdpos()"` function.
pub unsafe fn f_getcmdpos(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let p = get_ccline_ptr();
        (*rettv).vval.v_number = if !p.is_null() {
            ((*p).cmdpos + 1) as varnumber_T
        } else {
            0
        };
    }
}

/// `"getcmdprompt()"` function.
pub unsafe fn f_getcmdprompt(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let p = get_ccline_ptr();
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = if !p.is_null() && !(*p).cmdprompt.is_null() {
            xstrdup((*p).cmdprompt)
        } else {
            ptr::null_mut()
        };
    }
}

/// `"getcmdscreenpos()"` function.
pub unsafe fn f_getcmdscreenpos(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let p = get_ccline_ptr();
        (*rettv).vval.v_number = if !p.is_null() {
            ((*p).cmdspos + 1) as varnumber_T
        } else {
            0
        };
    }
}

/// `"getcmdtype()"` function.
pub unsafe fn f_getcmdtype(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmallocz(1).cast::<c_char>();
        *(*rettv).vval.v_string = get_cmdline_type() as c_char;
    }
}

/// Set the command line str to `str`.
/// @return 1 when failed, 0 when OK.
unsafe fn set_cmdline_str(str: *const c_char, pos: c_int) -> c_int {
    unsafe {
        let p = get_ccline_ptr();

        if p.is_null() {
            return 1;
        }

        let len = strlen(str) as c_int;
        realloc_cmdbuff(len + 1);
        (*p).cmdlen = len;
        strcpy((*p).cmdbuff, str);

        (*p).cmdpos = if pos < 0 || pos > (*p).cmdlen {
            (*p).cmdlen
        } else {
            pos
        };
        new_cmdpos = (*p).cmdpos;
        (*p).cmdbuff_replaced = true;

        redrawcmd();

        do_autocmd_cmdlinechanged(get_cmdline_type());

        0
    }
}

/// Set the command line byte position to `pos`.
unsafe fn set_cmdline_pos(pos: c_int) -> c_int {
    unsafe {
        let p = get_ccline_ptr();

        if p.is_null() {
            return 1;
        }

        new_cmdpos = pos.max(0);

        0
    }
}

/// `"setcmdline()"` function.
pub unsafe fn f_setcmdline(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_string_arg(argvars, 0) == FAIL
            || tv_check_for_opt_number_arg(argvars, 1) == FAIL
        {
            return;
        }

        let mut pos: c_int = -1;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;

            pos = (tv_get_number_chk(argvars.add(1), &raw mut error) - 1) as c_int;
            if error {
                return;
            }
            if pos < 0 {
                emsg(gettext(e_positive.as_ptr()));
                return;
            }
        }

        (*rettv).vval.v_number = set_cmdline_str(tv_get_string(argvars), pos) as varnumber_T;
    }
}

/// `"setcmdpos()"` function.
pub unsafe fn f_setcmdpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let pos = (tv_get_number(argvars) - 1) as c_int;

        if pos >= 0 {
            (*rettv).vval.v_number = set_cmdline_pos(pos) as varnumber_T;
        }
    }
}

/// Return the first character of the current command line.
pub unsafe fn get_cmdline_firstc() -> c_int {
    unsafe { ccline.cmdfirstc }
}

/// Get indices that specify a range within a list from a string.
pub unsafe fn get_list_range(str: *mut *mut c_char, num1: *mut c_int, num2: *mut c_int) -> c_int {
    unsafe {
        let mut len: c_int = 0;
        let mut first = false;
        let mut num: varnumber_T = 0;

        *str = skipwhite(*str);
        if **str == b'-' as c_char || ascii_isdigit(**str as c_int) {
            vim_str2nr(
                *str,
                ptr::null_mut(),
                &raw mut len,
                0,
                &raw mut num,
                ptr::null_mut(),
                0,
                false,
                ptr::null_mut(),
            );
            *str = (*str).add(len as usize);
            if num > c_int::MAX as varnumber_T {
                return FAIL;
            }

            *num1 = num as c_int;
            first = true;
        }
        *str = skipwhite(*str);
        if **str == b',' as c_char {
            *str = skipwhite((*str).add(1));
            vim_str2nr(
                *str,
                ptr::null_mut(),
                &raw mut len,
                0,
                &raw mut num,
                ptr::null_mut(),
                0,
                false,
                ptr::null_mut(),
            );
            if len > 0 {
                *str = skipwhite((*str).add(len as usize));
                if num > c_int::MAX as varnumber_T {
                    return FAIL;
                }

                *num2 = num as c_int;
            } else if !first {
                return FAIL;
            }
        } else if first {
            *num2 = *num1;
        }
        OK
    }
}

pub unsafe fn cmdline_init() {
    unsafe {
        let cc = &raw mut ccline;
        *cc = CmdlineInfo::new();
    }
}

/// Validate the `'cedit'` option.
pub unsafe fn validate_cedit(args: *mut optset_T) -> *const c_char {
    unsafe {
        let value = (*args).os_newval.data.string.data;
        if *value != NUL {
            let n = string_to_key(value);
            if n == 0 || vim_isprintc(n) {
                return e_invarg.as_ptr();
            }
        }
        ptr::null()
    }
}

/// The `'cedit'` option is changed.
pub unsafe fn did_set_cedit(_args: *mut optset_T) -> *const c_char {
    unsafe {
        cedit_key = if *p_cedit == NUL {
            -1
        } else {
            string_to_key(p_cedit)
        };
        ptr::null()
    }
}

/// Synchronously calls `vim._core.cmdwin.<action>(...)`.
unsafe fn cmdwin_invoke(action: *const c_char, firstc: c_int, content: *mut c_char, pos: c_int) {
    unsafe {
        let mut fc: [c_char; 2] = [firstc as c_char, 0];
        let mut tv_args = [
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: 0,
                vval: TypvalVval {
                    v_string: fc.as_mut_ptr(),
                },
            },
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: 0,
                vval: TypvalVval {
                    v_string: if !content.is_null() {
                        content
                    } else {
                        c"".as_ptr().cast_mut()
                    },
                },
            },
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: 0,
                vval: TypvalVval {
                    v_number: (pos + 1) as varnumber_T,
                },
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        nlua_call_typval(
            c"vim._core.cmdwin".as_ptr(),
            action,
            if firstc != 0 {
                tv_args.as_mut_ptr()
            } else {
                tv_args.as_mut_ptr().add(3)
            },
            ptr::null_mut(),
        );
        xfree(content.cast::<c_void>());
    }
}

/// Calls `vim._core.cmdwin.<action>()`, synchronously.
pub unsafe fn cmdwin_do_action(action: *const c_char) {
    unsafe {
        cmdwin_invoke(action, 0, ptr::null_mut(), 0);
    }
}

/// Deferred event: opens cmdwin after the cmdline-reader unwinds.
unsafe fn open_cmdwin_event(argv: *mut *mut c_void) {
    unsafe {
        let a = (*argv).cast::<CmdwinOpenArgs>();
        cmdwin_invoke(c"open".as_ptr(), (*a).firstc, (*a).content, (*a).pos);
        xfree(a.cast::<c_void>());
    }
}

/// Schedules cmdwin to open after the current cmdline-reader returns.
unsafe fn open_cmdwin() -> c_int {
    unsafe {
        if text_locked() || cmdline_star > 0 {
            beep_flush();
            return K_IGNORE;
        }
        if !cmdwin_buf.is_null() {
            beep_flush();
            return K_IGNORE;
        }
        let ft = get_cmdline_type();
        if ft != b':' as c_int && ft != b'/' as c_int && ft != b'?' as c_int {
            beep_flush();
            return K_IGNORE;
        }

        let a = xmalloc(size_of::<CmdwinOpenArgs>()).cast::<CmdwinOpenArgs>();
        (*a).firstc = ft;
        (*a).pos = ccline.cmdpos;

        (*a).content = if !ccline.cmdbuff.is_null() {
            xstrnsave(ccline.cmdbuff, ccline.cmdlen as usize)
        } else {
            ptr::null_mut()
        };
        ccline.cmdlen = 0;
        ccline.cmdpos = 0;

        let mut ev = Event::nil();
        ev.handler = Some(open_cmdwin_event);
        ev.argv[0] = a.cast::<c_void>();
        loop_schedule_deferred((&raw mut main_loop).cast(), ev);
        Ctrl_C
    }
}

/// @return true if `curbuf` is the cmdwin and we're not editing the command line.
pub unsafe fn is_in_cmdwin() -> bool {
    unsafe { bt_cmdwin(curbuf) && get_cmdline_type() == NUL as c_int }
}

/// Get script string.
pub unsafe fn script_get(eap: *mut exarg_T, lenp: *mut usize) -> *mut c_char {
    unsafe {
        let mut cmd = (*eap).arg;

        if *cmd != b'<' as c_char || *cmd.add(1) != b'<' as c_char || (*eap).ea_getline.is_none() {
            *lenp = strlen((*eap).arg);
            return if (*eap).skip != 0 {
                ptr::null_mut()
            } else {
                xmemdupz((*eap).arg.cast::<c_void>(), *lenp).cast::<c_char>()
            };
        }
        cmd = cmd.add(2);

        let l = heredoc_get(eap, cmd, true);
        if l.is_null() {
            return ptr::null_mut();
        }

        let mut buf: Vec<u8> = if (*eap).skip == 0 {
            Vec::with_capacity(0x400)
        } else {
            Vec::new()
        };

        let mut li = tv_list_first(l);
        while !li.is_null() {
            if (*eap).skip == 0 {
                let s = tv_get_string(&raw const (*li).li_tv);
                if !s.is_null() {
                    buf.extend_from_slice(CStr::from_ptr(s).to_bytes());
                }
                buf.push(b'\n');
            }
            li = (*li).li_next;
        }
        *lenp = buf.len();
        tv_list_free(l);
        if (*eap).skip == 0 {
            xmemdupz(buf.as_ptr().cast(), buf.len()).cast::<c_char>()
        } else {
            ptr::null_mut()
        }
    }
}

/// Used by `f_input()` and `f_inputdialog()` functions.
pub unsafe fn get_user_input(
    argvars: *const typval_T,
    rettv: *mut typval_T,
    inputdialog: bool,
    secret: bool,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if cmdpreview {
            return;
        }

        let prompt: *const c_char;
        let mut defstr: *const c_char = c"".as_ptr();
        let mut cancelreturn: *mut typval_T = ptr::null_mut();
        let mut cancelreturn_strarg2 = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        };
        let mut xp_name: *const c_char = ptr::null();
        let mut input_callback: Callback = std::mem::zeroed();
        let mut prompt_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let mut defstr_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let mut cancelreturn_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let mut xp_name_buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let def: [c_char; 1] = [0];
        if (*argvars).v_type == VarType::VAR_DICT {
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                emsg(gettext(c"E5050: {opts} must be the only argument".as_ptr()));
                return;
            }
            let dict = (*argvars).vval.v_dict;
            prompt = tv_dict_get_string_buf_chk(
                dict,
                c"prompt".as_ptr(),
                6,
                prompt_buf.as_mut_ptr(),
                c"".as_ptr(),
            );
            if prompt.is_null() {
                return;
            }
            defstr = tv_dict_get_string_buf_chk(
                dict,
                c"default".as_ptr(),
                7,
                defstr_buf.as_mut_ptr(),
                c"".as_ptr(),
            );
            if defstr.is_null() {
                return;
            }
            let cancelreturn_di = tv_dict_find(dict, c"cancelreturn".as_ptr(), 12);
            if !cancelreturn_di.is_null() {
                cancelreturn = &raw mut (*cancelreturn_di).di_tv;
            }
            xp_name = tv_dict_get_string_buf_chk(
                dict,
                c"completion".as_ptr(),
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
            if !tv_dict_get_callback(dict, c"highlight".as_ptr(), 9, &raw mut input_callback) {
                return;
            }
        } else {
            prompt = tv_get_string_buf_chk(argvars, prompt_buf.as_mut_ptr());
            if prompt.is_null() {
                return;
            }
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                defstr = tv_get_string_buf_chk(argvars.add(1), defstr_buf.as_mut_ptr());
                if defstr.is_null() {
                    return;
                }
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    let strarg2 =
                        tv_get_string_buf_chk(argvars.add(2), cancelreturn_buf.as_mut_ptr());
                    if strarg2.is_null() {
                        return;
                    }
                    if inputdialog {
                        cancelreturn_strarg2.v_type = VarType::VAR_STRING;
                        cancelreturn_strarg2.vval.v_string = strarg2.cast_mut();
                        cancelreturn = &raw mut cancelreturn_strarg2;
                    } else {
                        xp_name = strarg2;
                    }
                }
            }
        }

        let mut xp_type: c_int = EXPAND_NOTHING;
        let mut xp_arg: *mut c_char = ptr::null_mut();
        if !xp_name.is_null() {
            let xp_namelen = strlen(xp_name) as c_int;
            let mut argt: u32 = 0;
            if parse_compl_arg(
                xp_name,
                xp_namelen,
                &raw mut xp_type,
                &raw mut argt,
                &raw mut xp_arg,
            ) == FAIL
            {
                return;
            }
        }

        let mut p = prompt;
        if !ui_has(kUICmdline) {
            let lastnl = strrchr(prompt, b'\n' as c_int);
            if !lastnl.is_null() {
                p = lastnl.add(1);
                msg_start();
                msg_clr_eos();
                msg_puts_len(prompt, p.offset_from(prompt), get_echo_hl_id(), false);
                msg_didout = false;
                msg_starthere();
            }
        }
        cmdline_row = msg_row;

        stuffReadbuffSpecial(defstr);

        let save_ex_normal_busy = ex_normal_busy;
        ex_normal_busy = 0;
        atom_payload_start();
        (*rettv).vval.v_string = getcmdline_prompt(
            if secret { NUL as c_int } else { b'@' as c_int },
            p,
            get_echo_hl_id(),
            xp_type,
            xp_arg,
            input_callback,
            false,
            ptr::null_mut(),
        );
        atom_payload_end();
        ex_normal_busy = save_ex_normal_busy;
        callback_free(&raw mut input_callback);

        if (*rettv).vval.v_string.is_null() && !cancelreturn.is_null() {
            tv_copy(cancelreturn, rettv);
        }

        xfree(xp_arg.cast::<c_void>());

        need_wait_return = false;
        msg_didout = false;
    }
}

/// `"wildtrigger()"` function.
pub unsafe fn f_wildtrigger(_argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (State & MODE_CMDLINE) == 0
            || char_avail()
            || wild_menu_showing != 0
            || cmdline_pum_active()
        {
            return;
        }

        let cmd_type = get_cmdline_type();

        if cmd_type == b':' as c_int || cmd_type == b'/' as c_int || cmd_type == b'?' as c_int {
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
}
