//! Builtin Vimscript functions (`src/nvim/eval/funcs.c`).

use crate::api::private::converter::{object_to_vim, object_to_vim_take_luaref, vim_to_object};
use crate::api::private::helpers::{
    DI_FLAGS_LOCK, api_clear_error, api_free_object, api_metadata, cstr_as_string, dict_set_var,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::vim::nvim_feedkeys;
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_BUFFILEPOST, EVENT_BUFFILEPRE, apply_autocmds,
    au_exists, autocmd_supported,
};
use crate::buffer::{
    FileBuf, K_UIMESSAGES, VAR_FIXED, buf_close_terminal, buflist_findnr, buflist_findpat, setfname,
};
use crate::buffer_updates::luaret_truthy;
use crate::channel::{
    CallbackReader, Channel, ChannelStdinMode, K_CHANNEL_PART_RPC_CONST, K_CHANNEL_PART_STDERR,
    K_CHANNEL_PART_STDIN, K_CHANNEL_PART_STDOUT, K_CHANNEL_STDIN_PIPE_CONST, K_CHANNEL_STREAM_PROC,
    K_PROC_TYPE_PTY, channel_close, channel_connect, channel_create_event, channel_decref,
    channel_from_stdio, channel_incref, channel_job_start, channel_send, channel_terminal_alloc,
};
use crate::charset::{CTRL_V_CONST, VARNUMBER_MIN, ascii_isdigit, getdigits_int, skipwhite};
use crate::cmdexpand::{
    Cmdidx, EXPAND_FILES, SEARCH_START, WILD_ALL, WILD_ALL_KEEP, WILD_ICASE, WILD_KEEP_ALL,
    WILD_LIST_NOTFOUND, WILD_SILENT, WILD_USE_NL, WILD_USE_SHELLSLASH, cmdline_pum_active,
    expand_one,
};
use crate::cursor::{
    Colnr, CursorPos, Linenr, VALID_VIRTCOL, check_cursor, equalpos, get_cursor_pos_ptr,
};
use crate::dialog::{do_dialog, prompt_for_input};
use crate::errors::*;
use crate::eval::buffer::find_buffer;
use crate::eval::decode::{
    json_decode_string, mpack_parse_typval, typval_parser_error_free, unpack_typval,
};
use crate::eval::encode::{
    ListReaderState, encode_init_lrstate, encode_list_write, encode_read_from_list, encode_tv2echo,
    encode_tv2json, encode_vim_list_to_buf, encode_vim_to_msgpack,
};
#[cfg(unix)]
use crate::eval::typval::tv_dict_extend;
use crate::eval::typval::{
    Callback, Dictitem, EvalFuncData, Listitem, TV_TRANSLATE, TvBlob, TvDict, TvFloat, TvList,
    TvPartial, TypVal, TypvalVval, VAR_UNLOCKED, VarType, Varnumber, callback_free,
    check_repeat_count, tv_blob_alloc_ret, tv_blob_get, tv_blob_len, tv_blob_set_range,
    tv_check_for_buffer_arg, tv_check_for_dict_arg, tv_check_for_list_arg,
    tv_check_for_list_or_blob_arg, tv_check_for_lnum_arg, tv_check_for_nonempty_string_arg,
    tv_check_for_nonnull_dict_arg, tv_check_for_number_arg, tv_check_for_opt_bool_arg,
    tv_check_for_opt_dict_arg, tv_check_for_opt_number_arg, tv_check_for_string_arg,
    tv_check_for_string_or_func_arg, tv_check_for_string_or_list_arg, tv_check_num,
    tv_check_str_or_nr, tv_clear, tv_copy, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_dict_alloc_ret, tv_dict_find,
    tv_dict_get_bool, tv_dict_get_callback, tv_dict_get_number, tv_dict_get_number_def,
    tv_dict_get_string, tv_dict_hi2di, tv_dict_item_remove, tv_dict_watcher_add,
    tv_dict_watcher_remove, tv_equal, tv_get_bool, tv_get_bool_chk, tv_get_lnum, tv_get_lnum_buf,
    tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf, tv_get_string_buf_chk,
    tv_get_string_chk, tv_initial_value, tv_is_func, tv_islocked, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_dict, tv_list_append_list, tv_list_append_number, tv_list_append_owned_tv,
    tv_list_append_string, tv_list_append_tv, tv_list_copy, tv_list_extend, tv_list_find,
    tv_list_find_nr, tv_list_first, tv_list_flatten, tv_list_item_remove, tv_list_len, tv_list_ref,
    tv_list_set_lock, tv_list_set_ret, tv_list_unref, value_check_lock,
};
use crate::eval::userfunc::{
    EvalFuncDef, FCERR_NONE, FCERR_NOTMETHOD, FCERR_TOOFEW, FCERR_TOOMANY, FCERR_UNKNOWN,
    emsg_funcname, find_func, func_call, func_ptr_ref, func_ref, func_unref, function_exists,
    get_func_arity, get_scriptlocal_funcname, get_user_func_name, printable_func_name,
    restore_funccal, save_funccal, save_function_name, set_current_funccal, trans_function_name,
    translated_function_exists,
};
use crate::eval::vars::{
    Lval, VV_KEY, VV_REG, VV_SEND_SERVER, VV_SHELL_ERROR, VV_VAL, cat_prefix_varname, find_var,
    get_user_var_name, get_vim_var_nr, get_vim_var_str, get_vim_var_tv, prepare_vimvar,
    restore_vimvar, set_vim_var_nr, set_vim_var_type, tv_list_locked, var_exists,
};
use crate::eval::window::{find_tabwin, find_win_by_nr_or_id, win_id2wp_tp};
use crate::eval::{
    FNE_CHECK_START, GLV_NO_AUTOLOAD, GLV_READ_ONLY, K_LIST_LEN_MAY_KNOW_CONST,
    K_LIST_LEN_SHOULD_KNOW, K_LIST_LEN_UNKNOWN_CONST, TFN_NO_AUTOLOAD, TFN_QUIET, VARNUMBER_MAX,
    add_timer_info, add_timer_info_all, buf_byteidx_to_charidx, buf_charidx_to_byteidx,
    callback_from_typval, common_job_callbacks, do_string_sub, eval_expr_to_bool, eval_expr_typval,
    eval_expr_valid_arg, eval_has_provider, eval_option, eval1, find_job, find_timer_by_nr,
    get_callback_depth, get_copy_id, get_lval, list2fpos, partial_name, save_tv_as_string,
    script_host_eval, string2float, timer_start, timer_stop, timer_stop_all, tv_dict_set_ret,
    tv_to_argv, var_item_copy, var2fpos,
};
#[cfg(unix)]
use crate::event::r#loop::uv_kill;
use crate::event::r#loop::{Loop, loop_closing_ptr, loop_poll_events, loop_queues, uv_strerror};
use crate::event::multiqueue::{
    multiqueue_empty, multiqueue_free, multiqueue_new, multiqueue_process_events,
    multiqueue_replace_parent,
};
use crate::event::proc::{Proc, PtyProc, proc_stop, proc_wait};
use crate::event::time::{
    TimeWatcher, TimeWatcherData, time_watcher_close, time_watcher_init, time_watcher_start,
    time_watcher_stop,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{
    EX_NOSPC, Exarg, cmd_exists, do_cmdline, do_cmdline_cmd, eval_vars, expand_filename,
};
use crate::ex_eval::aborting;
use crate::ex_getln::{
    Hlf, VSE_NONE, get_user_input, text_locked, text_locked_msg, vim_strsave_fnameescape,
};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::globals::*;
use crate::grid::{
    Schar, ScreenGrid, grid_getchar, schar_from_char, schar_get, schar_get_first_codepoint,
};
use crate::highlight_group::{
    get_highlight_name_ext, highlight_color, highlight_exists, highlight_has_attr,
    syn_get_final_id, syn_name2id,
};
use crate::indent::{get_sw_value, get_sw_value_col};
use crate::input::{Tasave, restore_typeahead, save_typeahead, stuff_empty, using_script, vgetc};
use crate::insexpand::ins_compl_active;
use crate::keycodes::vim_strsave_escape_ks;
use crate::lua::executor::{
    K_RET_NIL_BOOL, K_RET_OBJECT, nlua_call_luaeval, nlua_call_typval, nlua_exec, nlua_func_exists,
    nlua_is_table_from_lua, nlua_register_table_as_callable,
};
use crate::mark::{
    cleanup_jumplist, get_buf_local_marks, get_global_marks, setmark_pos, setpcmark,
};
use crate::math::{FAIL, OK, xisinf, xisnan};
use crate::mbyte::{
    Vimconv, convert_setup, enc_locale, mb_adjust_cursor, string_convert, utf_char2bytes,
    utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memline::{
    decl, incl, ml_find_line_or_offset, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_open,
    recover_names, swapfile_dict,
};
use crate::memory::{
    ARENA_BLOCK_SIZE, Arena, ArenaMem, alloc_block, arena_finish, arena_mem_free, cstr_from_bytes,
    cstr_to_cstring, free_block,
};
use crate::menu::{get_menu_cmd_modes, menu_get};
use crate::message::{
    emsg, internal_error, msg_clr_eos, msg_ext_set_kind, msg_putchar, msg_puts, msg_scroll_flush,
    msg_start, semsg, semsg_multiline, str2special_save, verb_msg,
};
use crate::r#move::{update_curswant, win_col_off};
use crate::mpack::mpack_core::MPACK_OK;
use crate::mpack::object::MpackParser;
use crate::msgpack_rpc::channel::{find_channel, get_client_info, rpc_send_call, rpc_send_event};
use crate::msgpack_rpc::packer::{Integer, PackerBuffer, packer_string_buffer, packer_take_string};
use crate::msgpack_rpc::server::{
    server_address_list, server_address_new, server_start, server_stop,
};
use crate::normal::{find_decl, op_pending};
use crate::ops::{
    block_prep, charwise_block_prep, cursor_pos_info, getregionpos_line, getregionpos_prep,
};
use crate::option::set_option_value_give_err;
use crate::option_vars::*;
use crate::optionstr::{OptIndex, free_string_option};
use crate::os::dl::os_libcall;
use crate::os::env::{
    ExpandCtx, expand_env_save, home_replace, os_env_exists, os_get_pid, os_getenv_cstr,
    vim_env_iter, vim_getenv, vim_setenv_ext, vim_unsetenv_ext,
};
use crate::os::fs::{MAXPATHL, os_isdir, os_setperm};
use crate::os::pty_proc::pty_proc_resize;
use crate::os::stdpaths::{XDGVarType, get_appname, get_xdg_home, stdpaths_get_xdg_var};
use crate::os::time::{
    NS_PER_SEC, Tm, os_hrtime, os_localtime_r, os_strptime, os_time, strftime, time_t,
};
use crate::path::{ENV_SEPCHAR, concat_fnames_cstr, to_slash_save, vim_full_name};
use crate::plines::{getvvcol, win_chartabsize};
use crate::popupmenu::{pum_set_event_info, pum_visible};
use crate::profile::{
    Proftime, profile_end, profile_msg, profile_setlimit, profile_signed, profile_start,
    profile_sub,
};
use crate::regexp::{
    MAXCOL, NSUBEXP, NUMBUFLEN, RE_MAGIC, RE_STRING, Regmatch, reg_submatch, reg_submatch_list,
    vim_regcomp, vim_regexec_nl, vim_regfree,
};
use crate::register::{
    BlockDef, K_GREG_EXPR_SRC, YREG_YANK, format_reg_type, get_reg_contents_list,
    get_reg_contents_str, get_reg_type, get_register_name, get_unname_register, get_yank_register,
    op_reg_set_previous, write_reg_contents_ex, write_reg_contents_lst,
};
use crate::runtime::{Sctx, rt_st};
use crate::search::{
    BACKWARD, SearchitArg, last_csearch, last_csearch_forward, last_csearch_until, searchit,
    set_csearch_direction, set_csearch_until, set_last_csearch,
};
use crate::sha256::sha256_hex;
use crate::spell::{eval_soundfold, parse_spelllang, spell_check, spell_move_to};
use crate::spellsuggest::spell_suggest_list;
use crate::state::{MODE_CMDLINE, MODE_MAX_LENGTH, get_mode, get_was_safe_state, virtual_active};
#[cfg(windows)]
use crate::strings::mb_strup_buf;
use crate::strings::{
    gettext, vim_snprintf, vim_snprintf_safelen, vim_snprintf_typval_only, vim_strsave_escaped,
    vim_strsave_shellescape,
};
use crate::syntax::{
    HL_CONCEAL, get_syntax_info, syn_get_id, syn_get_stack_item, syn_get_sub_char, syntax_present,
};
use crate::tag::{Tagname, get_tagfname, get_tags, get_tagstack, set_tagstack};
use crate::textformat::Oparg;
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_current_col, ui_current_row, ui_flush, ui_gui_attached, ui_has,
    ui_rgb_attached,
};
use crate::ui_compositor::ui_comp_get_grid_at_coord;
use crate::version::{has_nvim_version, has_vim_patch};
use crate::window::{TabPageInfo, WinInfoInfo, find_tabpage};
use std::ffi::{CStr, c_char};
use std::ptr;

pub type SpecialVarValue = i32;

const BASE_NONE: u8 = 0;
const BASE_LAST: u8 = u8::MAX;

pub(crate) const MAX_FUNC_ARGS: usize = 20;

pub(crate) const K_SPECIAL_VAR_NULL_CONST: SpecialVarValue = 0;

use crate::eval::vars::{
    VAR_TYPE_BLOB, VAR_TYPE_BOOL, VAR_TYPE_DICT, VAR_TYPE_FLOAT, VAR_TYPE_FUNC, VAR_TYPE_LIST,
    VAR_TYPE_NUMBER, VAR_TYPE_SPECIAL, VAR_TYPE_STRING,
};

pub(crate) const TFN_INT: i32 = 1;
pub(crate) const TFN_NO_DEREF: i32 = 8;

const K_CHANNEL_STDIN_NULL: ChannelStdinMode = 1;

const VIM_GENERIC: i32 = 0;
const VIM_ERROR: i32 = 1;
pub(crate) const VIM_WARNING: i32 = 2;
const VIM_INFO: i32 = 3;
pub(crate) const VIM_QUESTION: i32 = 4;

pub(crate) const DOCMD_KEYTYPED: i32 = 0x08;

pub(crate) const MENU_INDEX_TIP: i32 = 7;
pub(crate) const MENU_ALL_MODES: i32 = (1 << MENU_INDEX_TIP) - 1;
const AUTOLOAD_CHAR: i32 = b'#' as i32;

pub(crate) const RE_SEARCH: i32 = 0;

pub(crate) const MPACK_EOF: i32 = 1;
pub(crate) const MPACK_ERROR: i32 = 2;
pub(crate) const MPACK_NOMEM: i32 = 3;

pub(crate) const SEARCH_END: i32 = 0x40;
pub(crate) const SEARCH_KEEP: i32 = 0x400;
pub(crate) const SEARCH_COL: i32 = 0x1000;

const SP_NOMOVE: i32 = 0x01;
const SP_REPEAT: i32 = 0x02;
const SP_RETCOUNT: i32 = 0x04;
const SP_SETPCMARK: i32 = 0x08;
const SP_START: i32 = 0x10;
const SP_SUBPAT: i32 = 0x20;
const SP_END: i32 = 0x40;
const SP_COLUMN: i32 = 0x80;

pub(crate) const K_OPT_CPOPTIONS: OptIndex = 61;

pub(crate) const MT_UNKNOWN: i32 = -1;
pub(crate) const MT_CHAR_WISE: i32 = 0;
pub(crate) const MT_LINE_WISE: i32 = 1;
pub(crate) const MT_BLOCK_WISE: i32 = 2;

pub(crate) const HLF_SPB: Hlf = 37;
pub(crate) const HLF_SPC: Hlf = 38;
pub(crate) const HLF_SPR: Hlf = 39;
pub(crate) const HLF_SPL: Hlf = 40;

pub(crate) const HL_INVERSE: i32 = 0x01;
pub(crate) const HL_BOLD: i32 = 0x02;
pub(crate) const HL_ITALIC: i32 = 0x04;
pub(crate) const HL_UNDERLINE: i32 = 0x08;
pub(crate) const HL_UNDERCURL: i32 = 0x10;
pub(crate) const HL_UNDERDOUBLE: i32 = 0x18;
pub(crate) const HL_UNDERDOTTED: i32 = 0x20;
pub(crate) const HL_UNDERDASHED: i32 = 0x28;
pub(crate) const HL_STANDOUT: i32 = 0x0040;
pub(crate) const HL_DIM: i32 = 0x0200;
pub(crate) const HL_NOCOMBINE: i32 = 0x0400;
pub(crate) const HL_BLINK: i32 = 0x8000;
pub(crate) const HL_CONCEALED: i32 = 0x10000;

const E_INVALID_SUBMATCH_NUMBER_NR: &CStr = c"E935: Invalid submatch number: %d";
pub(crate) const E_STRING_LIST_OR_BLOB_REQUIRED: &CStr = c"E1098: String, List or Blob required";
const E_MISSING_FUNCTION_ARGUMENT: &CStr = c"E1132: Missing function argument";
const E_INVACT2: &CStr = c"E962: Invalid action: '%s'";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SomeMatchType {
    SomeMatch,
    SomeMatchEnd,
    SomeMatchList,
    SomeMatchStr,
    SomeMatchStrPos,
}

#[repr(C)]
pub struct GetListLineCookie {
    li: *const Listitem,
}

struct EvalFuncsState {
    inputsecret_flag: bool,
    ga_userinput: Vec<Tasave>,
    has_wsl: TriState,
    rand_gx: u32,
    rand_gy: u32,
    rand_gz: u32,
    rand_gw: u32,
    rand_initialized: bool,
    get_func_name_intidx: i32,
    get_expr_name_intidx: i32,
}

impl EvalFuncsState {
    const fn new() -> Self {
        Self {
            inputsecret_flag: false,
            ga_userinput: Vec::new(),
            has_wsl: K_NONE,
            rand_gx: 0,
            rand_gy: 0,
            rand_gz: 0,
            rand_gw: 0,
            rand_initialized: false,
            get_func_name_intidx: -1,
            get_expr_name_intidx: -1,
        }
    }
}

static FUNCS_STATE: GlobalState<EvalFuncsState> = GlobalState::new(EvalFuncsState::new());

#[inline]
fn funcs_st() -> &'static mut EvalFuncsState {
    unsafe { &mut *FUNCS_STATE.as_ptr() }
}

const PTY_IGNORED_ENV_VARS: &[&CStr] = &[
    #[cfg(not(windows))]
    c"COLUMNS",
    #[cfg(not(windows))]
    c"LINES",
    #[cfg(not(windows))]
    c"TERMCAP",
    #[cfg(not(windows))]
    c"COLORFGBG",
    c"COLORTERM",
    c"VIM",
    c"VIMRUNTIME",
];

#[cfg(windows)]
const REQUIRED_ENV_VARS: &[&CStr] = &[
    c"HOMEDRIVE",
    c"HOMEPATH",
    c"LOGONSERVER",
    c"PATH",
    c"SYSTEMDRIVE",
    c"SYSTEMROOT",
    c"TEMP",
    c"USERDOMAIN",
    c"USERNAME",
    c"USERPROFILE",
    c"WINDIR",
];
#[cfg(not(windows))]
const REQUIRED_ENV_VARS: &[&CStr] = &[];

#[cfg(windows)]
pub(crate) const PROCESS_ALL_ACCESS: u32 = 0x001F0FFF;
#[cfg(windows)]
use crate::os::os_win_console::{CloseHandle, DebugBreakProcess, OpenProcess};

const fn compute_build_has_flags() -> u32 {
    let mut flags = 0u32;
    if match option_env!("NVIM_HAVE_UNIBILIUM") {
        Some(s) => !s.is_empty() && s.as_bytes()[0] != b'0',
        None => true,
    } {
        flags |= 1 << 1;
    }
    if cfg!(target_os = "linux") {
        flags |= 1 << 2;
    }
    if cfg!(target_os = "android") {
        flags |= 1 << 3;
    }
    if crate::os::lang::HAVE_LIBINTL {
        flags |= 1 << 4;
    }
    flags
}

const NVIM_BUILD_HAS_FLAGS: u32 = compute_build_has_flags();

use crate::charset::{tolower_asc, toupper_asc};
use crate::cursor::clearpos;
use crate::eval::typval::tv_dict_len;
use crate::event::r#loop::{uv_env_item_t, uv_os_environ, uv_os_free_environ, uv_random};
use crate::mpack::object::mpack_parser_init;
use crate::os::time::mktime;
use crate::terminal::{terminal_buf, terminal_open, terminal_running};

#[inline]
fn tv_list_uidx(l: Option<&TvList>, mut n: i32) -> i32 {
    if n < 0 {
        n += tv_list_len(l);
    }
    if n < 0 || n >= tv_list_len(l) { -1 } else { n }
}

#[inline]
fn tv_get_float_chk(tv: &TypVal, ret_f: &mut TvFloat) -> bool {
    {
        if tv.v_type == VarType::VarFloat {
            *ret_f = tv.vval.v_float();
            return true;
        }
        if tv.v_type == VarType::VarNumber {
            *ret_f = tv.vval.v_number() as TvFloat;
            return true;
        }
        semsg!(c"%s", gettext(c"E808: Number or Float required"),);
        false
    }
}

#[inline]
fn proc_is_stopped(proc_: &mut Proc) -> bool {
    proc_.status >= 0 || proc_.stopped_time != 0
}

/// Function given to `ExpandGeneric()` to obtain the list of internal
/// or user defined function names.
pub fn get_function_name(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    {
        if idx == 0 {
            funcs_st().get_func_name_intidx = -1;
        }
        if funcs_st().get_func_name_intidx < 0 {
            let name = get_user_func_name(xp, idx);
            if !name.is_null() {
                if !crate::memory::cstr_is_empty(name)
                    && crate::memory::cstr_first_byte(name) != b'<'
                    && (b"g:" == crate::memory::cstr_prefix_bytes(xp.xp_pattern, 2))
                {
                    return cat_prefix_varname(b'g' as i32, name);
                }
                return name;
            }
        }

        funcs_st().get_func_name_intidx += 1;
        let fdef = eval_func_def_at(funcs_st().get_func_name_intidx);
        let Some(key_cstr) = fdef.name else {
            return ptr::null_mut();
        };
        let key = key_cstr.as_ptr();
        let key_len = key_cstr.to_bytes().len();
        let iobuff = g_st().iobuff.as_mut_ptr();
        unsafe { ptr::copy_nonoverlapping(key, iobuff, key_len) };
        unsafe {
            *iobuff.wrapping_add(key_len) = b'(' as c_char;
        }
        if fdef.max_argc == 0 {
            unsafe {
                *iobuff.wrapping_add(key_len + 1) = b')' as c_char;
            }
            unsafe {
                *iobuff.wrapping_add(key_len + 2) = NUL;
            }
        } else {
            unsafe {
                *iobuff.wrapping_add(key_len + 1) = NUL;
            }
        }
        iobuff
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of internal or
/// user defined variable or function names.
pub fn get_expr_name(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if idx == 0 {
        funcs_st().get_expr_name_intidx = -1;
    }
    if funcs_st().get_expr_name_intidx < 0 {
        let name = get_function_name(xp, idx);
        if !name.is_null() {
            return name;
        }
    }
    funcs_st().get_expr_name_intidx += 1;
    get_user_var_name(xp, funcs_st().get_expr_name_intidx)
}

const INTERNAL_FUNC_NAME_MAP: &[(&[u8], i32)] = &[
    (b"abs", 3),
    (b"acos", 39),
    (b"add", 4),
    (b"and", 5),
    (b"api_info", 160),
    (b"append", 71),
    (b"appendbufline", 424),
    (b"argc", 22),
    (b"argidx", 110),
    (b"arglistid", 251),
    (b"argv", 44),
    (b"asin", 35),
    (b"assert_beeps", 409),
    (b"assert_equal", 410),
    (b"assert_equalfile", 522),
    (b"assert_exception", 504),
    (b"assert_fails", 411),
    (b"assert_false", 412),
    (b"assert_inrange", 478),
    (b"assert_match", 413),
    (b"assert_nobeep", 452),
    (b"assert_notequal", 497),
    (b"assert_notmatch", 498),
    (b"assert_report", 453),
    (b"assert_true", 372),
    (b"atan", 36),
    (b"atan2", 65),
    (b"blob2list", 218),
    (b"browse", 78),
    (b"browsedir", 266),
    (b"bufadd", 72),
    (b"bufexists", 272),
    (b"buffer_exists", 446),
    (b"buffer_name", 362),
    (b"buffer_number", 447),
    (b"buflisted", 252),
    (b"bufload", 112),
    (b"bufloaded", 262),
    (b"bufname", 113),
    (b"bufnr", 68),
    (b"bufwinid", 212),
    (b"bufwinnr", 213),
    (b"byte2line", 219),
    (b"byteidx", 114),
    (b"byteidxcomp", 333),
    (b"call", 31),
    (b"ceil", 32),
    (b"chanclose", 234),
    (b"changenr", 192),
    (b"chansend", 193),
    (b"char2nr", 115),
    (b"charclass", 235),
    (b"charcol", 116),
    (b"charidx", 117),
    (b"chdir", 49),
    (b"cindent", 118),
    (b"clearmatches", 398),
    (b"cmdcomplete_info", 518),
    (b"col", 6),
    (b"complete", 197),
    (b"complete_add", 388),
    (b"complete_check", 463),
    (b"complete_info", 428),
    (b"confirm", 119),
    (b"copy", 45),
    (b"cos", 7),
    (b"cosh", 28),
    (b"count", 57),
    (b"cursor", 94),
    (b"debugbreak", 280),
    (b"deepcopy", 198),
    (b"delete", 79),
    (b"deletebufline", 429),
    (b"dictwatcheradd", 459),
    (b"dictwatcherdel", 460),
    (b"did_filetype", 396),
    (b"diff_filler", 343),
    (b"diff_hlID", 223),
    (b"digraph_get", 360),
    (b"digraph_getlist", 492),
    (b"digraph_set", 361),
    (b"digraph_setlist", 493),
    (b"empty", 54),
    (b"environ", 120),
    (b"escape", 80),
    (b"eval", 33),
    (b"eventhandler", 395),
    (b"executable", 320),
    (b"execute", 121),
    (b"exepath", 122),
    (b"exists", 98),
    (b"exp", 8),
    (b"expand", 73),
    (b"expandcmd", 260),
    (b"extend", 74),
    (b"extendnew", 261),
    (b"feedkeys", 172),
    (b"file_readable", 448),
    (b"filecopy", 177),
    (b"filereadable", 389),
    (b"filewritable", 404),
    (b"filter", 95),
    (b"finddir", 123),
    (b"findfile", 173),
    (b"flatten", 124),
    (b"flattennew", 288),
    (b"float2nr", 161),
    (b"floor", 52),
    (b"fmod", 23),
    (b"fnameescape", 338),
    (b"fnamemodify", 352),
    (b"foldclosed", 301),
    (b"foldclosedend", 439),
    (b"foldlevel", 256),
    (b"foldtext", 174),
    (b"foldtextresult", 464),
    (b"foreach", 125),
    (b"foreground", 315),
    (b"fullcommand", 357),
    (b"funcref", 126),
    (b"function", 164),
    (b"garbagecollect", 466),
    (b"get", 9),
    (b"getbufinfo", 291),
    (b"getbufline", 292),
    (b"getbufoneline", 431),
    (b"getbufvar", 269),
    (b"getcellwidths", 440),
    (b"getchangelist", 418),
    (b"getchar", 127),
    (b"getcharmod", 275),
    (b"getcharpos", 276),
    (b"getcharsearch", 419),
    (b"getcharstr", 277),
    (b"getcmdcomplpat", 462),
    (b"getcmdcompltype", 485),
    (b"getcmdline", 284),
    (b"getcmdpos", 258),
    (b"getcmdprompt", 386),
    (b"getcmdscreenpos", 486),
    (b"getcmdtype", 285),
    (b"getcmdwintype", 425),
    (b"getcompletion", 443),
    (b"getcompletiontype", 541),
    (b"getcurpos", 270),
    (b"getcursorcharpos", 505),
    (b"getcwd", 75),
    (b"getenv", 107),
    (b"getfontname", 354),
    (b"getfperm", 178),
    (b"getfsize", 179),
    (b"getftime", 180),
    (b"getftype", 181),
    (b"getjumplist", 353),
    (b"getline", 128),
    (b"getloclist", 282),
    (b"getmarklist", 363),
    (b"getmatches", 321),
    (b"getmousepos", 373),
    (b"getpid", 76),
    (b"getpos", 99),
    (b"getqflist", 246),
    (b"getreg", 85),
    (b"getreginfo", 295),
    (b"getregion", 239),
    (b"getregionpos", 392),
    (b"getregtype", 296),
    (b"getscriptinfo", 449),
    (b"getstacktrace", 420),
    (b"gettabinfo", 281),
    (b"gettabvar", 229),
    (b"gettabwinvar", 380),
    (b"gettagstack", 345),
    (b"gettext", 129),
    (b"getwininfo", 305),
    (b"getwinpos", 253),
    (b"getwinposx", 306),
    (b"getwinposy", 307),
    (b"getwinvar", 254),
    (b"glob", 21),
    (b"glob2regpat", 364),
    (b"globpath", 162),
    (b"has", 10),
    (b"has_key", 130),
    (b"haslocaldir", 331),
    (b"hasmapto", 186),
    (b"highlightID", 350),
    (b"highlight_exists", 501),
    (b"histadd", 131),
    (b"histdel", 132),
    (b"histget", 133),
    (b"histnr", 96),
    (b"hlID", 20),
    (b"hlexists", 216),
    (b"hostname", 207),
    (b"iconv", 48),
    (b"id", 0),
    (b"indent", 102),
    (b"index", 55),
    (b"indexof", 134),
    (b"input", 56),
    (b"inputdialog", 334),
    (b"inputlist", 267),
    (b"inputrestore", 405),
    (b"inputsave", 268),
    (b"inputsecret", 367),
    (b"insert", 103),
    (b"interrupt", 265),
    (b"invert", 104),
    (b"isabsolutepath", 472),
    (b"isdirectory", 339),
    (b"isinf", 63),
    (b"islocked", 194),
    (b"isnan", 64),
    (b"items", 66),
    (b"jobclose", 165),
    (b"jobpid", 77),
    (b"jobresize", 240),
    (b"jobsend", 135),
    (b"jobstart", 204),
    (b"jobstop", 136),
    (b"jobwait", 137),
    (b"join", 37),
    (b"json_decode", 335),
    (b"json_encode", 340),
    (b"keys", 40),
    (b"keytrans", 208),
    (b"last_buffer_nr", 461),
    (b"len", 11),
    (b"libcall", 138),
    (b"libcallnr", 230),
    (b"line", 25),
    (b"line2byte", 220),
    (b"lispindent", 308),
    (b"list2blob", 221),
    (b"list2str", 209),
    (b"localtime", 257),
    (b"log", 12),
    (b"log10", 58),
    (b"luaeval", 139),
    (b"map", 13),
    (b"maparg", 86),
    (b"mapcheck", 166),
    (b"maplist", 140),
    (b"mapnew", 109),
    (b"mapset", 105),
    (b"match", 46),
    (b"matchadd", 167),
    (b"matchaddpos", 329),
    (b"matcharg", 168),
    (b"matchbufline", 381),
    (b"matchdelete", 336),
    (b"matchend", 169),
    (b"matchfuzzy", 293),
    (b"matchfuzzypos", 432),
    (b"matchlist", 250),
    (b"matchstr", 170),
    (b"matchstrlist", 406),
    (b"matchstrpos", 368),
    (b"max", 14),
    (b"menu_get", 211),
    (b"menu_info", 224),
    (b"min", 15),
    (b"mkdir", 51),
    (b"mode", 26),
    (b"msgpackdump", 332),
    (b"msgpackparse", 384),
    (b"nextnonblank", 402),
    (b"nr2char", 141),
    (b"nvim__buf_debug_extmarks", 633),
    (b"nvim__buf_stats", 481),
    (b"nvim__cmdwin_set", 523),
    (b"nvim__complete_set", 542),
    (b"nvim__get_lib_dir", 525),
    (b"nvim__get_runtime", 526),
    (b"nvim__id", 187),
    (b"nvim__id_array", 457),
    (b"nvim__id_dict", 416),
    (b"nvim__id_float", 458),
    (b"nvim__inspect_cell", 543),
    (b"nvim__invalidate_glyph_cache", 640),
    (b"nvim__ns_get", 376),
    (b"nvim__ns_set", 377),
    (b"nvim__redraw", 378),
    (b"nvim__runtime_inspect", 612),
    (b"nvim__screenshot", 510),
    (b"nvim__set_restart_on_crash", 637),
    (b"nvim__stats", 326),
    (b"nvim__unpack", 379),
    (b"nvim_buf_add_highlight", 616),
    (b"nvim_buf_attach", 482),
    (b"nvim_buf_clear_highlight", 630),
    (b"nvim_buf_clear_namespace", 631),
    (b"nvim_buf_create_user_command", 641),
    (b"nvim_buf_del_extmark", 584),
    (b"nvim_buf_del_keymap", 560),
    (b"nvim_buf_del_mark", 531),
    (b"nvim_buf_del_user_command", 635),
    (b"nvim_buf_del_var", 508),
    (b"nvim_buf_delete", 483),
    (b"nvim_buf_get_changedtick", 626),
    (b"nvim_buf_get_commands", 605),
    (b"nvim_buf_get_extmark_by_id", 638),
    (b"nvim_buf_get_extmarks", 606),
    (b"nvim_buf_get_keymap", 561),
    (b"nvim_buf_get_lines", 544),
    (b"nvim_buf_get_mark", 532),
    (b"nvim_buf_get_name", 533),
    (b"nvim_buf_get_number", 579),
    (b"nvim_buf_get_offset", 565),
    (b"nvim_buf_get_option", 570),
    (b"nvim_buf_get_text", 534),
    (b"nvim_buf_get_var", 511),
    (b"nvim_buf_is_loaded", 545),
    (b"nvim_buf_is_valid", 536),
    (b"nvim_buf_line_count", 559),
    (b"nvim_buf_set_extmark", 585),
    (b"nvim_buf_set_keymap", 562),
    (b"nvim_buf_set_lines", 546),
    (b"nvim_buf_set_mark", 538),
    (b"nvim_buf_set_name", 539),
    (b"nvim_buf_set_option", 571),
    (b"nvim_buf_set_text", 540),
    (b"nvim_buf_set_var", 515),
    (b"nvim_buf_set_virtual_text", 636),
    (b"nvim_call_dict_function", 622),
    (b"nvim_call_function", 547),
    (b"nvim_chan_set", 422),
    (b"nvim_clear_autocmds", 567),
    (b"nvim_cmd", 188),
    (b"nvim_command", 385),
    (b"nvim_command_output", 580),
    (b"nvim_create_augroup", 566),
    (b"nvim_create_autocmd", 577),
    (b"nvim_create_buf", 484),
    (b"nvim_create_namespace", 613),
    (b"nvim_create_user_command", 632),
    (b"nvim_del_augroup_by_id", 621),
    (b"nvim_del_augroup_by_name", 629),
    (b"nvim_del_autocmd", 502),
    (b"nvim_del_current_line", 601),
    (b"nvim_del_keymap", 487),
    (b"nvim_del_mark", 426),
    (b"nvim_del_user_command", 614),
    (b"nvim_del_var", 387),
    (b"nvim_echo", 225),
    (b"nvim_err_write", 465),
    (b"nvim_err_writeln", 524),
    (b"nvim_eval", 226),
    (b"nvim_eval_statusline", 591),
    (b"nvim_exec", 227),
    (b"nvim_exec2", 289),
    (b"nvim_exec_autocmds", 548),
    (b"nvim_feedkeys", 433),
    (b"nvim_get_all_options_info", 634),
    (b"nvim_get_autocmds", 529),
    (b"nvim_get_chan_info", 549),
    (b"nvim_get_color_by_name", 619),
    (b"nvim_get_color_map", 550),
    (b"nvim_get_commands", 530),
    (b"nvim_get_context", 506),
    (b"nvim_get_current_buf", 587),
    (b"nvim_get_current_line", 602),
    (b"nvim_get_current_tabpage", 627),
    (b"nvim_get_current_win", 596),
    (b"nvim_get_hl", 346),
    (b"nvim_get_hl_by_id", 535),
    (b"nvim_get_hl_by_name", 557),
    (b"nvim_get_hl_id_by_name", 618),
    (b"nvim_get_hl_ns", 467),
    (b"nvim_get_keymap", 488),
    (b"nvim_get_mark", 434),
    (b"nvim_get_mode", 435),
    (b"nvim_get_namespaces", 572),
    (b"nvim_get_option", 489),
    (b"nvim_get_option_info", 592),
    (b"nvim_get_option_info2", 608),
    (b"nvim_get_option_value", 609),
    (b"nvim_get_proc", 436),
    (b"nvim_get_proc_children", 620),
    (b"nvim_get_runtime_file", 611),
    (b"nvim_get_var", 393),
    (b"nvim_get_vvar", 437),
    (b"nvim_input", 299),
    (b"nvim_input_mouse", 519),
    (b"nvim_list_bufs", 470),
    (b"nvim_list_chans", 491),
    (b"nvim_list_runtime_paths", 624),
    (b"nvim_list_tabpages", 551),
    (b"nvim_list_uis", 441),
    (b"nvim_list_wins", 471),
    (b"nvim_load_context", 527),
    (b"nvim_notify", 355),
    (b"nvim_open_tabpage", 528),
    (b"nvim_open_term", 473),
    (b"nvim_open_win", 444),
    (b"nvim_out_write", 474),
    (b"nvim_parse_cmd", 475),
    (b"nvim_parse_expression", 604),
    (b"nvim_paste", 312),
    (b"nvim_put", 189),
    (b"nvim_replace_termcodes", 615),
    (b"nvim_select_popupmenu_item", 639),
    (b"nvim_set_current_buf", 588),
    (b"nvim_set_current_dir", 589),
    (b"nvim_set_current_line", 603),
    (b"nvim_set_current_tabpage", 628),
    (b"nvim_set_current_win", 597),
    (b"nvim_set_hl", 369),
    (b"nvim_set_hl_ns", 477),
    (b"nvim_set_hl_ns_fast", 558),
    (b"nvim_set_keymap", 494),
    (b"nvim_set_option", 495),
    (b"nvim_set_option_value", 610),
    (b"nvim_set_var", 407),
    (b"nvim_set_vvar", 450),
    (b"nvim_strwidth", 451),
    (b"nvim_tabpage_del_var", 593),
    (b"nvim_tabpage_get_number", 625),
    (b"nvim_tabpage_get_var", 594),
    (b"nvim_tabpage_get_win", 598),
    (b"nvim_tabpage_is_valid", 600),
    (b"nvim_tabpage_list_wins", 617),
    (b"nvim_tabpage_set_var", 595),
    (b"nvim_tabpage_set_win", 599),
    (b"nvim_ui_send", 414),
    (b"nvim_win_close", 479),
    (b"nvim_win_del_var", 509),
    (b"nvim_win_get_buf", 512),
    (b"nvim_win_get_config", 568),
    (b"nvim_win_get_cursor", 581),
    (b"nvim_win_get_height", 563),
    (b"nvim_win_get_number", 582),
    (b"nvim_win_get_option", 573),
    (b"nvim_win_get_position", 607),
    (b"nvim_win_get_tabpage", 586),
    (b"nvim_win_get_var", 513),
    (b"nvim_win_get_width", 554),
    (b"nvim_win_hide", 455),
    (b"nvim_win_is_valid", 537),
    (b"nvim_win_resize", 500),
    (b"nvim_win_set_buf", 516),
    (b"nvim_win_set_config", 569),
    (b"nvim_win_set_cursor", 583),
    (b"nvim_win_set_height", 564),
    (b"nvim_win_set_hl_ns", 555),
    (b"nvim_win_set_option", 574),
    (b"nvim_win_set_var", 517),
    (b"nvim_win_set_width", 556),
    (b"nvim_win_text_height", 590),
    (b"or", 1),
    (b"pathshorten", 348),
    (b"perleval", 185),
    (b"pow", 16),
    (b"preinserted", 370),
    (b"prevnonblank", 403),
    (b"printf", 84),
    (b"prompt_appendbuf", 514),
    (b"prompt_getinput", 499),
    (b"prompt_getprompt", 520),
    (b"prompt_setcallback", 553),
    (b"prompt_setinterrupt", 575),
    (b"prompt_setprompt", 521),
    (b"pum_getpos", 290),
    (b"pumvisible", 318),
    (b"py3eval", 142),
    (b"pyeval", 90),
    (b"pyxeval", 143),
    (b"rand", 24),
    (b"range", 47),
    (b"readblob", 175),
    (b"readdir", 144),
    (b"readfile", 176),
    (b"reduce", 81),
    (b"reg_executing", 456),
    (b"reg_recorded", 390),
    (b"reg_recording", 430),
    (b"reltime", 145),
    (b"reltimefloat", 399),
    (b"reltimestr", 303),
    (b"remove", 82),
    (b"rename", 83),
    (b"repeat", 106),
    (b"resolve", 146),
    (b"reverse", 147),
    (b"round", 59),
    (b"rpcnotify", 263),
    (b"rpcrequest", 314),
    (b"rpcstart", 205),
    (b"rpcstop", 148),
    (b"rubyeval", 217),
    (b"screenattr", 309),
    (b"screenchar", 310),
    (b"screenchars", 356),
    (b"screencol", 241),
    (b"screenpos", 242),
    (b"screenrow", 243),
    (b"screenstring", 400),
    (b"search", 89),
    (b"searchcount", 349),
    (b"searchdecl", 297),
    (b"searchpair", 298),
    (b"searchpairpos", 438),
    (b"searchpos", 236),
    (b"serverlist", 316),
    (b"serverstart", 365),
    (b"serverstop", 317),
    (b"setbufline", 294),
    (b"setbufvar", 271),
    (b"setcellwidths", 442),
    (b"setcharpos", 278),
    (b"setcharsearch", 421),
    (b"setcmdline", 286),
    (b"setcmdpos", 259),
    (b"setcursorcharpos", 507),
    (b"setenv", 108),
    (b"setfperm", 182),
    (b"setline", 149),
    (b"setloclist", 283),
    (b"setmatches", 322),
    (b"setpos", 100),
    (b"setqflist", 247),
    (b"setreg", 87),
    (b"settabvar", 231),
    (b"settabwinvar", 382),
    (b"settagstack", 347),
    (b"setwinvar", 255),
    (b"sha256", 70),
    (b"shellescape", 341),
    (b"shiftwidth", 325),
    (b"sign_define", 337),
    (b"sign_getdefined", 490),
    (b"sign_getplaced", 468),
    (b"sign_jump", 228),
    (b"sign_place", 313),
    (b"sign_placelist", 476),
    (b"sign_undefine", 454),
    (b"sign_unplace", 415),
    (b"sign_unplacelist", 503),
    (b"simplify", 199),
    (b"sin", 17),
    (b"sinh", 29),
    (b"slice", 53),
    (b"sockconnect", 358),
    (b"sort", 41),
    (b"soundfold", 238),
    (b"spellbadword", 383),
    (b"spellsuggest", 408),
    (b"split", 60),
    (b"sqrt", 42),
    (b"srand", 61),
    (b"state", 67),
    (b"stdioopen", 264),
    (b"stdpath", 150),
    (b"str2float", 248),
    (b"str2list", 159),
    (b"str2nr", 97),
    (b"strcharlen", 279),
    (b"strcharpart", 330),
    (b"strchars", 171),
    (b"strdisplaywidth", 496),
    (b"strftime", 183),
    (b"strgetchar", 323),
    (b"stridx", 111),
    (b"string", 88),
    (b"strlen", 93),
    (b"strpart", 151),
    (b"strptime", 200),
    (b"strridx", 152),
    (b"strtrans", 210),
    (b"strutf16len", 344),
    (b"strwidth", 214),
    (b"submatch", 190),
    (b"substitute", 300),
    (b"swapfilelist", 397),
    (b"swapinfo", 201),
    (b"swapname", 202),
    (b"synID", 69),
    (b"synIDattr", 222),
    (b"synIDtrans", 324),
    (b"synconcealed", 401),
    (b"synstack", 206),
    (b"system", 92),
    (b"systemlist", 304),
    (b"tabpagebuflist", 469),
    (b"tabpagenr", 232),
    (b"tabpagewinnr", 394),
    (b"tagfiles", 184),
    (b"taglist", 153),
    (b"tan", 18),
    (b"tanh", 30),
    (b"tempname", 203),
    (b"termopen", 191),
    (b"test_garbagecollect_now", 623),
    (b"test_write_list_log", 578),
    (b"timer_info", 273),
    (b"timer_pause", 327),
    (b"timer_start", 328),
    (b"timer_stop", 274),
    (b"timer_stopall", 417),
    (b"tolower", 154),
    (b"toupper", 155),
    (b"tr", 2),
    (b"trim", 34),
    (b"trunc", 62),
    (b"type", 27),
    (b"undofile", 195),
    (b"undotree", 196),
    (b"uniq", 38),
    (b"utf16idx", 158),
    (b"values", 101),
    (b"virtcol", 156),
    (b"virtcol2col", 359),
    (b"visualmode", 302),
    (b"wait", 43),
    (b"wildmenumode", 391),
    (b"wildtrigger", 366),
    (b"win_execute", 375),
    (b"win_findbuf", 351),
    (b"win_getid", 249),
    (b"win_gettype", 342),
    (b"win_gotoid", 311),
    (b"win_id2tabwin", 427),
    (b"win_id2win", 287),
    (b"win_move_separator", 552),
    (b"win_move_statusline", 576),
    (b"win_screenpos", 423),
    (b"win_splitmove", 445),
    (b"winbufnr", 163),
    (b"wincol", 91),
    (b"windowsversion", 480),
    (b"winheight", 244),
    (b"winlayout", 233),
    (b"winline", 157),
    (b"winnr", 50),
    (b"winrestcmd", 319),
    (b"winrestview", 371),
    (b"winsaveview", 374),
    (b"winwidth", 215),
    (b"wordcount", 237),
    (b"writefile", 245),
    (b"xor", 19),
];

#[inline]
pub fn find_internal_func_hash(slice: &[u8]) -> i32 {
    match INTERNAL_FUNC_NAME_MAP.binary_search_by_key(&slice, |&(k, _)| k) {
        Ok(i) => INTERNAL_FUNC_NAME_MAP[i].1,
        Err(_) => -1,
    }
}

/// Gets a builtin (aka "vimfn", "eval") function from the generated hash table.
pub fn find_internal_func(name: impl crate::strings::IntoCCharPtr) -> *const EvalFuncDef {
    let name = name.into_c_char_ptr();
    {
        let index = find_internal_func_hash(crate::memory::cstr_as_bytes(name));
        if index >= 0 {
            eval_func_def_at(index)
        } else {
            ptr::null()
        }
    }
}

/// Gets the Lua name of a Lua-implemented "vimfn" function, or NULL if not found.
pub fn find_internal_func_lua(name: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let name = name.into_c_char_ptr();
    {
        let fdef = find_internal_func(name);
        if !fdef.is_null()
            && let EvalFuncData::LuaFunc(lua_name) = unsafe { (*fdef).data }
        {
            return lua_name.as_ptr();
        }
        ptr::null()
    }
}

/// Check the argument count to use for internal function "fdef".
pub fn check_internal_func(fdef: &EvalFuncDef, argcount: i32) -> i32 {
    let res = if argcount < i32::from(fdef.min_argc) {
        FCERR_TOOFEW
    } else if argcount > i32::from(fdef.max_argc) {
        FCERR_TOOMANY
    } else {
        return i32::from(fdef.base_arg);
    };

    let name = fdef.name.map_or(ptr::null(), |s| s.as_ptr());
    if res == FCERR_TOOMANY {
        semsg!(gettext(E_TOOMANYARG), name);
    } else {
        semsg!(gettext(E_TOOFEWARG), name);
    }
    -1
}

pub fn call_internal_func(
    fname: impl crate::strings::IntoCCharPtr,
    argcount: i32,
    argvars: *mut TypVal,
    rettv: &mut TypVal,
) -> i32 {
    let fname = fname.into_c_char_ptr();
    unsafe {
        let fdef = find_internal_func(fname);
        if fdef.is_null() {
            return FCERR_UNKNOWN;
        } else if argcount < i32::from((*fdef).min_argc) {
            return FCERR_TOOFEW;
        } else if argcount > i32::from((*fdef).max_argc) {
            return FCERR_TOOMANY;
        }
        (*argvars.wrapping_add(argcount as usize)).v_type = VarType::VarUnknown;
        if let Some(func) = (*fdef).func {
            let slice_len = (argcount as usize + 1).max(usize::from((*fdef).max_argc) + 1);
            func(
                std::slice::from_raw_parts_mut(argvars, slice_len),
                rettv,
                (*fdef).data,
            );
        }
        FCERR_NONE
    }
}

/// Invoke a method for `base->method()`.
pub fn call_internal_method(
    fname: impl crate::strings::IntoCCharPtr,
    argcount: i32,
    argvars: *mut TypVal,
    rettv: &mut TypVal,
    basetv: *mut TypVal,
) -> i32 {
    let fname = fname.into_c_char_ptr();
    unsafe {
        let fdef = find_internal_func(fname);
        if fdef.is_null() {
            return FCERR_UNKNOWN;
        } else if (*fdef).base_arg == BASE_NONE {
            return FCERR_NOTMETHOD;
        } else if argcount + 1 < i32::from((*fdef).min_argc) {
            return FCERR_TOOFEW;
        } else if argcount + 1 > i32::from((*fdef).max_argc) {
            return FCERR_TOOMANY;
        }

        let mut argv = [tv_initial_value(); MAX_FUNC_ARGS + 1];
        let base_index: isize = if (*fdef).base_arg == BASE_LAST {
            argcount as isize
        } else {
            isize::from((*fdef).base_arg) - 1
        };
        if (argcount as isize) < base_index {
            return FCERR_TOOFEW;
        }
        ptr::copy_nonoverlapping(argvars, argv.as_mut_ptr(), base_index as usize);
        argv[base_index as usize] = *basetv;
        ptr::copy_nonoverlapping(
            argvars.wrapping_add(base_index as usize),
            argv.as_mut_ptr().wrapping_add((base_index + 1) as usize),
            (argcount as isize - base_index) as usize,
        );
        argv[(argcount + 1) as usize].v_type = VarType::VarUnknown;

        if let Some(func) = (*fdef).func {
            func(&mut argv, rettv, (*fdef).data);
        }
        FCERR_NONE
    }
}

/// Returns true for a non-zero Number and a non-empty String.
#[inline]
fn non_zero_arg(argvars: &TypVal) -> bool {
    {
        (argvars.v_type == VarType::VarNumber && argvars.vval.v_number() != 0)
            || (argvars.v_type == VarType::VarBool
                && argvars.vval.v_bool() == K_BOOL_VAR_TRUE_CONST)
            || (argvars.v_type == VarType::VarString
                && !argvars.vval.v_string().is_null()
                && !crate::memory::cstr_is_empty(argvars.vval.v_string()))
    }
}

/// Apply a floating point C function on a typval with one `TvFloat`.
pub fn float_op_wrapper(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
    let mut f: TvFloat = 0.0;
    rettv.v_type = VarType::VarFloat;
    if tv_get_float_chk(&argvars[0], &mut f)
        && let EvalFuncData::FloatOp(func_float) = fptr
    {
        rettv.vval = crate::eval::typval::TypvalVval::Float(func_float(f));
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Float(0.0);
    }
}

/// Invokes an API (`nvim_`) function from Vimscript.
pub fn api_wrapper(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
    {
        if check_secure() {
            return;
        }

        let EvalFuncData::ApiHandler(handler) = fptr else {
            return;
        };
        let mut args_storage = [Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Boolean(false),
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = argvars.as_mut_ptr();
        while unsafe { (*tv).v_type } != VarType::VarUnknown {
            *args.next_slot_mut() = vim_to_object(tv, Some(&mut arena), false);
            args.size += 1;
            tv = tv.wrapping_add(1);
        }

        let mut err = Error::new();
        let fn_ptr = handler.r#fn.unwrap();
        let mut result = fn_ptr(VIML_INTERNAL_CALL, args, &mut arena, &mut err);

        if err.err_type != ErrorType::ErrorTypeNone {
            semsg_multiline!(c"emsg", E_API_ERROR, err.msg_ptr());
        } else {
            object_to_vim_take_luaref(&mut result, &mut *rettv, true, &mut err);
        }

        if handler.ret_alloc {
            api_free_object(result);
        }
        arena_mem_free(arena_finish(&mut arena));
        api_clear_error(&mut err);
    }
}

/// Invokes a Lua-implemented vimfn/"f_xx" function from Vimscript.
pub fn lua_wrapper(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
    {
        let mut args_storage = [Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Boolean(false),
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = argvars.as_mut_ptr();
        while unsafe { (*tv).v_type } != VarType::VarUnknown {
            *args.next_slot_mut() = vim_to_object(tv, Some(&mut arena), false);
            args.size += 1;
            tv = tv.wrapping_add(1);
        }

        let EvalFuncData::LuaFunc(lua_name) = fptr else {
            return;
        };
        let mut buf = [0 as c_char; 256];
        vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"return require('vim._core.vimfn').%s(...)",
            lua_name.as_ptr(),
        );

        let mut err = Error::new();
        let mut result = nlua_exec(
            cstr_as_string(buf.as_ptr()),
            ptr::null(),
            args,
            K_RET_OBJECT,
            Some(&mut arena),
            &mut err,
        );

        if err.err_type != ErrorType::ErrorTypeNone {
            semsg_multiline!(c"emsg", E_API_ERROR, err.msg_ptr());
        } else {
            object_to_vim_take_luaref(&mut result, &mut *rettv, true, &mut err);
        }

        arena_mem_free(arena_finish(&mut arena));
        api_clear_error(&mut err);
    }
}

/// "abs(expr)" function
pub fn f_abs(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type == VarType::VarFloat {
            let mut f: TvFloat = 0.0;
            rettv.v_type = VarType::VarFloat;
            if tv_get_float_chk(&argvars[0], &mut f) {
                rettv.vval = crate::eval::typval::TypvalVval::Float(f.abs());
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::Float(0.0);
            }
        } else {
            let mut error = false;
            let n = tv_get_number_chk(&argvars[0], Some(&mut error));
            if error {
                rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
            } else if n > 0 {
                rettv.vval = crate::eval::typval::TypvalVval::Number(n);
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::Number(n.wrapping_neg());
            }
        }
    }
}

/// "and(expr, expr)" function
pub fn f_and(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        tv_get_number_chk(&argvars[0], None) & tv_get_number_chk(&argvars[1], None),
    );
}

/// "api_info()" function
pub fn f_api_info(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    object_to_vim(
        api_metadata(),
        &mut *rettv,
        &mut crate::api::private::validate::Error::new(),
    );
}

/// "atan2()" function
pub fn f_atan2(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut fx: TvFloat = 0.0;
    let mut fy: TvFloat = 0.0;
    rettv.v_type = VarType::VarFloat;
    if tv_get_float_chk(&argvars[0], &mut fx) && tv_get_float_chk(&argvars[1], &mut fy) {
        rettv.vval = crate::eval::typval::TypvalVval::Float(fx.atan2(fy));
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Float(0.0);
    }
}

/// Get buffer by number or pattern.
pub fn tv_get_buf(tv: &mut TypVal, curtab_only: i32) -> *mut FileBuf {
    {
        if tv.v_type == VarType::VarNumber {
            return buflist_findnr(tv.vval.v_number() as i32);
        }
        if tv.v_type != VarType::VarString {
            return ptr::null_mut();
        }

        let name = tv.vval.v_string();
        if crate::memory::cstr_is_empty(name) {
            return g_st().curbuf;
        }
        if crate::memory::cstr_first_byte(name) == b'$' && crate::memory::cstr_byte_at(name, 1) == 0
        {
            return g_st().lastbuf;
        }

        let save_magic = opt_vars().p_magic;
        opt_vars().p_magic = 1;
        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        let buf = buflist_findnr(buflist_findpat(
            name,
            name.wrapping_add(crate::memory::cstr_len(name)),
            true,
            false,
            curtab_only != 0,
        ));

        opt_vars().p_magic = save_magic;
        opt_vars().p_cpo = save_cpo;

        if buf.is_null() {
            return find_buffer(tv);
        }

        buf
    }
}

/// Like `tv_get_buf()` but give an error message if the type is wrong.
pub fn tv_get_buf_from_arg(tv: &mut TypVal) -> *mut FileBuf {
    {
        if !tv_check_str_or_nr(&*tv) {
            return ptr::null_mut();
        }
        g_st().emsg_off += 1;
        let buf = tv_get_buf(tv, 0);
        g_st().emsg_off -= 1;
        buf
    }
}

/// Get the buffer from "arg" and give an error and return NULL if it is not valid.
pub fn get_buf_arg(arg: &mut TypVal) -> *mut FileBuf {
    {
        g_st().emsg_off += 1;
        let buf = tv_get_buf(arg, 0);
        g_st().emsg_off -= 1;
        if buf.is_null() {
            semsg!(gettext(E_INVALID_BUFFER_NAME_STR), tv_get_string(&*arg),);
        }
        buf
    }
}

/// "byte2line(byte)" function
pub fn f_byte2line(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut boff = (tv_get_number(&argvars[0]) as i32) - 1;
        if boff < 0 {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(ml_find_line_or_offset(
                curbuf_mut(),
                0,
                Some(&mut boff),
                false,
            ) as Varnumber);
        }
    }
}

/// "call(func, arglist [, dict])" function
pub fn f_call(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if !tv_check_for_list_arg(argvars, 1) {
            return;
        }
        if argvars[1].vval.v_list().is_null() {
            return;
        }

        let mut owned = false;
        let mut func: *mut c_char;
        let mut partial: *mut TvPartial = ptr::null_mut();
        if argvars[0].v_type == VarType::VarFunc {
            func = argvars[0].vval.v_string();
        } else if argvars[0].v_type == VarType::VarPartial {
            partial = argvars[0].vval.v_partial();
            func = partial_name(unsafe { partial.as_ref() });
        } else if nlua_is_table_from_lua(&argvars[0]) {
            func = nlua_register_table_as_callable(&argvars[0]);
            owned = true;
        } else {
            func = tv_get_string(&argvars[0]).cast_mut();
        }

        if crate::memory::cstr_is_empty(func) {
            return;
        }
        let mut tofree: Option<std::ffi::CString> = None;
        if argvars[0].v_type == VarType::VarString {
            let mut p = func;
            tofree = trans_function_name(
                &raw mut p,
                false,
                TFN_INT | TFN_QUIET,
                None,
                ptr::null_mut(),
            );
            let Some(ref tf) = tofree else {
                emsg_funcname(E_UNKNOWN_FUNCTION_STR, func);
                return;
            };
            func = tf.as_ptr().cast_mut();
        }

        'done: {
            let mut selfdict: *mut TvDict = ptr::null_mut();
            if argvars[2].v_type != VarType::VarUnknown {
                if !tv_check_for_dict_arg(argvars, 2) {
                    break 'done;
                }
                selfdict = argvars[2].vval.v_dict();
            }

            func_call(func, &mut argvars[1], partial, selfdict, &mut *rettv);
        }

        if owned {
            func_unref(func);
        }
        drop(tofree);
    }
}

/// "changenr()" function
pub fn f_changenr(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(Varnumber::from(curbuf_mut().b_u_seq_cur));
    }
}

/// "chanclose(id[, stream])" function
pub fn f_chanclose(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }

        if argvars[0].v_type != VarType::VarNumber
            || (argvars[1].v_type != VarType::VarString && argvars[1].v_type != VarType::VarUnknown)
        {
            emsg(gettext(E_INVARG));
            return;
        }

        let mut part = K_CHANNEL_PART_ALL;
        if argvars[1].v_type == VarType::VarString {
            let stream = tv_get_string(&argvars[1]);
            if crate::memory::cstr_as_bytes(stream) == b"stdin" {
                part = K_CHANNEL_PART_STDIN;
            } else if crate::memory::cstr_as_bytes(stream) == b"stdout" {
                part = K_CHANNEL_PART_STDOUT;
            } else if crate::memory::cstr_as_bytes(stream) == b"stderr" {
                part = K_CHANNEL_PART_STDERR;
            } else if crate::memory::cstr_as_bytes(stream) == b"rpc" {
                part = K_CHANNEL_PART_RPC_CONST;
            } else {
                semsg!(gettext(c"Invalid channel stream \"%s\""), stream);
                return;
            }
        }
        let mut error: *const c_char = ptr::null();
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(channel_close(
            argvars[0].vval.v_number() as u64,
            part,
            &mut error,
        )));
        if rettv.vval.v_number() == 0 {
            emsg(error);
        }
    }
}

/// "chansend(id, data)" function
pub fn f_chansend(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }

        if argvars[0].v_type != VarType::VarNumber || argvars[1].v_type == VarType::VarUnknown {
            emsg(gettext(E_INVARG));
            return;
        }

        let id = argvars[0].vval.v_number() as u64;
        #[cfg(not(windows))]
        let crlf = false;
        #[cfg(windows)]
        let crlf = {
            let chan = find_channel(id);
            !chan.is_null() && !(*chan).term.is_null()
        };

        let input = if argvars[1].v_type == VarType::VarBlob {
            let b = argvars[1].vval.v_blob().cast::<TvBlob>();
            let input_len = tv_blob_len(b.as_ref()) as usize;
            if input_len == 0 {
                return;
            }
            std::slice::from_raw_parts((*b).bv_ga.as_ptr().cast::<c_char>(), input_len).to_vec()
        } else {
            let Some(vec) = save_tv_as_string(&mut argvars[1], false, crlf) else {
                return;
            };
            if vec.is_empty() {
                return;
            }
            vec
        };

        let mut error: *const c_char = ptr::null();
        rettv.vval = crate::eval::typval::TypvalVval::Number(
            channel_send(id, input, &mut error) as Varnumber
        );
        if !error.is_null() {
            emsg(error);
        }
    }
}

/// "char2nr(string)" function
pub fn f_char2nr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[1].v_type != VarType::VarUnknown && !tv_check_num(&argvars[1]) {
            return;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(utf_ptr2char(
            tv_get_string(&argvars[0]),
        )));
    }
}

/// Get the current cursor column and store it in `rettv`.
fn get_col(argvars: &mut [TypVal], rettv: &mut TypVal, charcol: bool) {
    unsafe {
        if !tv_check_for_string_or_list_arg(argvars, 0) || !tv_check_for_opt_number_arg(argvars, 1)
        {
            return;
        }

        let mut wp = curwin_ptr();
        if argvars[1].v_type != VarType::VarUnknown {
            let mut tp: *mut TabPageInfo = ptr::null_mut();
            wp = win_id2wp_tp(tv_get_number(&argvars[1]) as i32, &raw mut tp);
            if wp.is_null() || tp.is_null() {
                return;
            }
            check_cursor(&mut *wp);
        }

        let bp = (*wp).w_buffer;
        let bp = { &mut *bp };
        let mut col: Colnr = 0;
        let mut fnum = bp.b_fnum;
        let fp = var2fpos(&argvars[0], false, &mut fnum, charcol, &mut *wp);
        if !fp.is_null() && fnum == bp.b_fnum {
            if (*fp).col == MAXCOL {
                if (*fp).lnum <= bp.b_ml.ml_line_count {
                    col = ml_get_buf_len(&mut *bp, (*fp).lnum) + 1;
                } else {
                    col = MAXCOL;
                }
            } else {
                col = (*fp).col + 1;
                if virtual_active(&mut *wp) && ptr::eq(fp, &raw const (*wp).w_cursor) {
                    let p = ml_get_buf(&mut *bp, (*wp).w_cursor.lnum)
                        .wrapping_add((*wp).w_cursor.col as usize);
                    if (*wp).w_cursor.coladd
                        >= win_chartabsize(&mut *wp, p, (*wp).w_virtcol - (*wp).w_cursor.coladd)
                        && !crate::memory::cstr_is_empty(p)
                    {
                        let l = utfc_ptr2len(p);
                        if crate::memory::cstr_byte_at(p, l as usize) == 0 {
                            col += l;
                        }
                    }
                }
            }
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(col));
    }
}

/// "charcol()" function
pub fn f_charcol(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_col(argvars, rettv, true);
}

pub fn get_optional_window(argvars: &mut [TypVal], idx: i32) -> *mut WinInfoInfo {
    {
        if argvars[idx as usize].v_type == VarType::VarUnknown {
            return curwin_ptr();
        }
        let win = find_win_by_nr_or_id(&argvars[idx as usize]);
        if win.is_null() {
            emsg(gettext(E_INVALWINDOW));
            return ptr::null_mut();
        }
        win
    }
}

/// "col(string)" function
pub fn f_col(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_col(argvars, rettv, false);
}

/// "confirm(message, buttons[, default [, type]])" function
pub fn f_confirm(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let mut buttons: *const c_char = ptr::null();
        let mut def: i32 = 1;
        let mut type_ = VIM_GENERIC;
        let mut error = false;

        let message = tv_get_string_chk(&argvars[0]);
        if message.is_null() {
            error = true;
        }
        if argvars[1].v_type != VarType::VarUnknown {
            buttons = tv_get_string_buf_chk(&argvars[1], buf.as_mut_ptr());
            if buttons.is_null() {
                error = true;
            }
            if argvars[2].v_type != VarType::VarUnknown {
                def = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
                if argvars[3].v_type != VarType::VarUnknown {
                    let typestr = tv_get_string_buf_chk(&argvars[3], buf2.as_mut_ptr());
                    if typestr.is_null() {
                        error = true;
                    } else {
                        match toupper_asc(i32::from(crate::memory::cstr_first_byte(typestr))) as u8
                        {
                            b'E' => type_ = VIM_ERROR,
                            b'Q' => type_ = VIM_QUESTION,
                            b'I' => type_ = VIM_INFO,
                            b'W' => type_ = VIM_WARNING,
                            b'G' => type_ = VIM_GENERIC,
                            _ => {}
                        }
                    }
                }
            }
        }

        if crate::memory::cstr_is_empty(buttons) {
            buttons = gettext(c"&Ok");
        }

        if !error {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(do_dialog(
                type_,
                ptr::null(),
                message,
                buttons,
                def,
                ptr::null(),
                0,
            )));
        }
    }
}

/// "copy()" function
pub fn f_copy(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    var_item_copy(None, &mut argvars[0], rettv, false, 0);
}

/// Set the cursor position.
fn set_cursorpos(argvars: &mut [TypVal], rettv: &mut TypVal, charcol: bool) {
    {
        let lnum: Linenr;
        let mut col: Colnr;
        let mut coladd: Colnr = 0;
        let mut set_curswant = true;

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        if argvars[0].v_type == VarType::VarList {
            let mut pos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut curswant: Colnr = -1;
            if !list2fpos(
                &mut argvars[0],
                &mut pos,
                None,
                Some(&mut curswant),
                charcol,
            ) {
                emsg(gettext(E_INVARG));
                return;
            }
            lnum = pos.lnum;
            col = pos.col;
            coladd = pos.coladd;
            if curswant >= 0 {
                curwin_mut().w_curswant = curswant - 1;
                set_curswant = false;
            }
        } else if (argvars[0].v_type == VarType::VarNumber
            || argvars[0].v_type == VarType::VarString)
            && (argvars[1].v_type == VarType::VarNumber || argvars[1].v_type == VarType::VarString)
        {
            let mut ln = tv_get_lnum(&argvars[0]);
            if ln < 0 {
                semsg!(gettext(E_INVARG2), tv_get_string(&argvars[0]));
            } else if ln == 0 {
                ln = curwin_mut().w_cursor.lnum;
            }
            lnum = ln;
            col = tv_get_number_chk(&argvars[1], None) as Colnr;
            if charcol {
                col = buf_charidx_to_byteidx(Some(crate::globals::curbuf_mut()), lnum, col) + 1;
            }
            if argvars[2].v_type != VarType::VarUnknown {
                coladd = tv_get_number_chk(&argvars[2], None) as Colnr;
            }
        } else {
            emsg(gettext(E_INVARG));
            return;
        }
        if lnum < 0 || col < 0 || coladd < 0 {
            return;
        }
        if lnum > 0 {
            curwin_mut().w_cursor.lnum = lnum;
        }
        if col != MAXCOL {
            col -= 1;
            if col < 0 {
                col = 0;
            }
        }
        curwin_mut().w_cursor.col = col;
        curwin_mut().w_cursor.coladd = coladd;

        check_cursor(curwin_mut());
        mb_adjust_cursor();

        curwin_mut().w_set_curswant = set_curswant;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
    }
}

/// "cursor(lnum, col)" or "cursor(list)" function
pub fn f_cursor(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    set_cursorpos(argvars, rettv, false);
}

/// "debugbreak()" function
pub fn f_debugbreak(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(FAIL));
        let pid = tv_get_number(&argvars[0]) as i32;
        if pid == 0 {
            emsg(gettext(E_INVARG));
            return;
        }

        #[cfg(windows)]
        {
            let h_process = unsafe { OpenProcess(PROCESS_ALL_ACCESS, 0, pid as u32) };
            if h_process.is_null() {
                return;
            }
            unsafe { DebugBreakProcess(h_process) };
            unsafe { CloseHandle(h_process) };
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(OK));
        }
        #[cfg(not(windows))]
        {
            uv_kill(pid, crate::os::signal::SIGINT);
        }
    }
}

/// "deepcopy()" function
pub fn f_deepcopy(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if !tv_check_for_opt_bool_arg(argvars, 1) {
            return;
        }
        let mut noref: Varnumber = 0;
        if argvars[1].v_type != VarType::VarUnknown {
            noref = tv_get_bool_chk(&argvars[1], None);
        }
        var_item_copy(
            None,
            &mut argvars[0],
            rettv,
            true,
            if noref == 0 { get_copy_id() } else { 0 },
        );
    }
}

/// `dictwatcheradd(dict, key, funcref)` function
pub fn f_dictwatcheradd(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarDict {
            semsg!(gettext(E_INVARG2), c"dict");
            return;
        } else if argvars[0].vval.v_dict().is_null() {
            let arg_errmsg = gettext(c"dictwatcheradd() argument");
            let arg_errmsg_len = crate::memory::cstr_len(arg_errmsg);
            semsg!(
                gettext(E_CANNOT_CHANGE_READONLY_VARIABLE_STR),
                arg_errmsg_len as i32,
                arg_errmsg,
            );
            return;
        }

        if argvars[1].v_type != VarType::VarString && argvars[1].v_type != VarType::VarNumber {
            semsg!(gettext(E_INVARG2), c"key");
            return;
        }

        let key_pattern = tv_get_string_chk(&argvars[1]);
        if key_pattern.is_null() {
            return;
        }
        let key_pattern_len = crate::memory::cstr_len(key_pattern);

        let mut callback = Callback::NONE;
        if !callback_from_typval(&mut callback, &argvars[2]) {
            semsg!(gettext(E_INVARG2), c"funcref");
            return;
        }

        tv_dict_watcher_add(
            unsafe { argvars[0].vval.v_dict().as_mut() },
            key_pattern,
            key_pattern_len,
            callback,
        );
    }
}

/// `dictwatcherdel(dict, key, funcref)` function
pub fn f_dictwatcherdel(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarDict {
            semsg!(gettext(E_INVARG2), c"dict");
            return;
        }
        if argvars[2].v_type != VarType::VarFunc && argvars[2].v_type != VarType::VarString {
            semsg!(gettext(E_INVARG2), c"funcref");
            return;
        }

        let key_pattern = tv_get_string_chk(&argvars[1]);
        if key_pattern.is_null() {
            return;
        }

        let mut callback = Callback::NONE;
        if !callback_from_typval(&mut callback, &argvars[2]) {
            return;
        }

        if !tv_dict_watcher_remove(
            unsafe { argvars[0].vval.v_dict().as_mut() },
            key_pattern,
            crate::memory::cstr_len(key_pattern),
            &callback,
        ) {
            emsg(c"Couldn't find a watcher matching key and callback");
        }

        callback_free(&mut callback);
    }
}

/// "did_filetype()" function
pub fn f_did_filetype(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(Varnumber::from(curbuf_mut().b_did_filetype));
    }
}

/// "empty({expr})" function
pub fn f_empty(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut n = true;
        match argvars[0].v_type {
            VarType::VarString | VarType::VarFunc => {
                n = argvars[0].vval.v_string().is_null()
                    || crate::memory::cstr_is_empty(argvars[0].vval.v_string());
            }
            VarType::VarPartial => {
                n = argvars[0].vval.v_partial().is_null();
            }
            VarType::VarNumber => {
                n = argvars[0].vval.v_number() == 0;
            }
            VarType::VarFloat => {
                n = argvars[0].vval.v_float() == 0.0;
            }
            VarType::VarList => {
                n = tv_list_len(unsafe { argvars[0].vval.v_list().as_ref() }) == 0;
            }
            VarType::VarDict => {
                n = tv_dict_len(unsafe { argvars[0].vval.v_dict().as_ref() }) == 0;
            }
            VarType::VarBool => {
                n = argvars[0].vval.v_bool() != K_BOOL_VAR_TRUE_CONST;
            }
            VarType::VarSpecial => {
                n = argvars[0].vval.v_special() == K_SPECIAL_VAR_NULL_CONST;
            }
            VarType::VarBlob => {
                n = tv_blob_len(unsafe { argvars[0].vval.v_blob().as_ref() }) == 0;
            }
            VarType::VarUnknown => {
                internal_error(c"f_empty(UNKNOWN)");
            }
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(n));
    }
}

/// "escape({string}, {chars})" function
pub fn f_escape(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let escaped = vim_strsave_escaped(
            tv_get_string(&argvars[0]),
            tv_get_string_buf(&argvars[1], buf.as_mut_ptr()),
        );
        crate::eval::typval::tv_set_string_len(rettv, escaped.as_ptr(), escaped.as_bytes().len());
    }
}

/// "getenv()" function
pub fn f_getenv(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let Some(p) = vim_getenv(tv_get_string(&argvars[0])) else {
        rettv.v_type = VarType::VarSpecial;
        rettv.vval = crate::eval::typval::TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST);
        return;
    };
    crate::eval::typval::tv_set_string(rettv, p.as_ptr());
}

/// "eval()" function
pub fn f_eval(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut s = tv_get_string_chk(&argvars[0]).cast_mut();
        if !s.is_null() {
            s = skipwhite(s);
        }
        let expr_start = s;
        if s.is_null() || !eval1(&raw mut s, &mut *rettv, Some(&mut g_st().evalarg_evaluate)) {
            if !expr_start.is_null() && !aborting() {
                semsg!(gettext(E_INVEXPR2), expr_start);
            }
            g_st().need_clr_eos = false;
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        } else if !crate::memory::cstr_is_empty(s) {
            semsg!(gettext(E_TRAILING_ARG), s);
        }
    }
}

/// "eventhandler()" function
pub fn f_eventhandler(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(g_st().vgetc_busy));
}

pub fn get_list_line(
    _c: i32,
    p: &mut GetListLineCookie,
    _indent: i32,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    {
        let item = p.li;
        if item.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; NUMBUFLEN];
        let s = tv_get_string_buf_chk(unsafe { &(*item).li_tv }, buf.as_mut_ptr());
        p.li = unsafe { (*item).li_next };
        if s.is_null() {
            None
        } else {
            Some(cstr_to_cstring(s))
        }
    }
}

pub fn execute_common(argvars: &mut [TypVal], rettv: &mut TypVal, arg_off: i32) {
    {
        let save_msg_silent = g_st().msg_silent;
        let save_emsg_silent = g_st().emsg_silent;
        let save_emsg_noredir = g_st().emsg_noredir;
        let save_redir_off = g_st().redir_off;
        let save_capture_ga = g_st().capture_ga;
        let save_msg_col = g_st().msg_col;
        let mut echo_output = false;

        if check_secure() {
            return;
        }

        let off = arg_off as usize;
        if argvars[off + 1].v_type != VarType::VarUnknown {
            let mut buf = [0 as c_char; NUMBUFLEN];
            let s = tv_get_string_buf_chk(&argvars[off + 1], buf.as_mut_ptr());
            if s.is_null() {
                return;
            }
            if crate::memory::cstr_is_empty(s) {
                echo_output = true;
            }
            if crate::memory::cstr_prefix_bytes(s, 6) == &b"silent"[..6] {
                g_st().msg_silent += 1;
            }
            if crate::memory::cstr_as_bytes(s) == b"silent!" {
                g_st().emsg_silent = 1;
                g_st().emsg_noredir = true;
            }
        } else {
            g_st().msg_silent += 1;
        }

        let mut capture_local: Vec<c_char> = Vec::new();
        g_st().capture_ga = &raw mut capture_local;
        g_st().redir_off = false;
        if !echo_output {
            g_st().msg_col = 0;
        }

        if argvars[off].v_type != VarType::VarList {
            do_cmdline_cmd(tv_get_string(&argvars[off]));
        } else if !argvars[off].vval.v_list().is_null() {
            let list = argvars[off].vval.v_list();
            tv_list_ref(unsafe { list.as_mut() });
            let mut cookie = GetListLineCookie {
                li: tv_list_first(unsafe { list.as_ref() }),
            };
            do_cmdline(
                ptr::null_mut(),
                crate::cmdexpand::LineGetter::List(&raw mut cookie),
                DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT | DOCMD_KEYTYPED,
            );
            tv_list_unref(list);
        }
        g_st().msg_silent = save_msg_silent;
        g_st().emsg_silent = save_emsg_silent;
        g_st().emsg_noredir = save_emsg_noredir;
        g_st().redir_off = save_redir_off;
        if echo_output {
            g_st().msg_col = 0;
        } else {
            g_st().msg_col = save_msg_col;
        }

        crate::eval::typval::tv_set_string_len(
            &mut *rettv,
            capture_local.as_ptr().cast(),
            capture_local.len(),
        );

        g_st().capture_ga = save_capture_ga;
    }
}

/// "execute(command)" function
pub fn f_execute(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    execute_common(argvars, rettv, 0);
}

/// "exists()" function
pub fn f_exists(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut n: i32 = 0;
        let mut p = tv_get_string(&argvars[0]);
        if crate::memory::cstr_first_byte(p) == b'$' {
            if os_env_exists(p.wrapping_add(1), false) {
                n = 1;
            } else {
                let exp = expand_env_save(p.cast_mut());
                if crate::memory::cstr_first_byte(exp.as_ptr()) != b'$' {
                    n = 1;
                }
            }
        } else if crate::memory::cstr_first_byte(p) == b'&'
            || crate::memory::cstr_first_byte(p) == b'+'
        {
            n = i32::from(eval_option(&raw mut p, None, true));
            if !crate::memory::cstr_is_empty(skipwhite(p)) {
                n = 0;
            }
        } else if crate::memory::cstr_first_byte(p) == b'*' {
            n = i32::from(if crate::memory::cstr_as_bytes(p).starts_with(b"*v:lua.") {
                nlua_func_exists(p.wrapping_add(7))
            } else {
                function_exists(p.wrapping_add(1), false)
            });
        } else if crate::memory::cstr_first_byte(p) == b':' {
            n = cmd_exists(p.wrapping_add(1));
        } else if crate::memory::cstr_first_byte(p) == b'#' {
            if crate::memory::cstr_byte_at(p, 1) == b'#' {
                n = i32::from(autocmd_supported(p.wrapping_add(2)));
            } else {
                n = i32::from(au_exists(p.wrapping_add(1)));
            }
        } else {
            n = i32::from(var_exists(p));
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(n));
    }
}

/// "expand()" function
pub fn f_expand(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut options = WILD_SILENT | WILD_USE_NL | WILD_LIST_NOTFOUND | WILD_USE_SHELLSLASH;
        let mut error = false;

        rettv.v_type = VarType::VarString;
        if argvars[1].v_type != VarType::VarUnknown
            && argvars[2].v_type != VarType::VarUnknown
            && tv_get_bool_chk(&argvars[2], Some(&mut error)) != 0
            && !error
        {
            tv_list_set_ret(rettv, ptr::null_mut());
        }

        let s = tv_get_string(&argvars[0]);
        if crate::memory::cstr_first_byte(s) == b'%'
            || crate::memory::cstr_first_byte(s) == b'#'
            || crate::memory::cstr_first_byte(s) == b'<'
        {
            if opt_vars().p_verbose == 0 {
                g_st().emsg_off += 1;
            }
            let mut len: usize = 0;
            let mut errormsg: *const c_char = ptr::null();
            let result = eval_vars(s.cast_mut(), s, &mut len, None, &mut errormsg, None, false);
            if opt_vars().p_verbose == 0 {
                g_st().emsg_off -= 1;
            } else if !errormsg.is_null() {
                emsg(errormsg);
            }
            if rettv.v_type == VarType::VarList {
                tv_list_alloc_ret(rettv, isize::from(result.is_some()));
                if let Some(ref r) = result {
                    tv_list_append_string(rettv.vval.v_list_ref(), r.as_ptr(), -1);
                }
            } else if let Some(ref r) = result {
                crate::eval::typval::tv_set_string(rettv, r.as_ptr());
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
        } else {
            if argvars[1].v_type != VarType::VarUnknown
                && tv_get_bool_chk(&argvars[1], Some(&mut error)) != 0
            {
                options |= WILD_KEEP_ALL;
            }
            if !error {
                let mut xpc: ExpandCtx = ExpandCtx::new();
                xpc.xp_context = EXPAND_FILES;
                if opt_vars().p_wic != 0 {
                    options += WILD_ICASE;
                }
                if rettv.v_type == VarType::VarString {
                    if let Some(exp) = expand_one(&mut xpc, s.cast_mut(), None, options, WILD_ALL) {
                        crate::eval::typval::tv_set_string(rettv, exp.as_ptr());
                    } else {
                        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                    }
                } else {
                    expand_one(&mut xpc, s.cast_mut(), None, options, WILD_ALL_KEEP);
                    tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);
                    for f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
                        tv_list_append_string(
                            rettv.vval.v_list_ref(),
                            f.as_ptr(),
                            f.as_bytes().len() as isize,
                        );
                    }
                }
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
        }
    }
}

/// "menu_get(path [, modes])" function
pub fn f_menu_get(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        let mut modes = MENU_ALL_MODES;
        if argvars[1].v_type == VarType::VarString {
            let strmodes = tv_get_string(&argvars[1]);
            modes = get_menu_cmd_modes(strmodes, false, None, None);
        }
        menu_get(tv_get_string(&argvars[0]).cast_mut(), modes, {
            rettv.vval.v_list_ref()
        });
    }
}

/// "expandcmd()" function
pub fn f_expandcmd(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut errormsg: *const c_char = ptr::null();
        let mut emsgoff = true;

        if argvars[1].v_type == VarType::VarDict
            && tv_dict_get_bool(
                unsafe { argvars[1].vval.v_dict().as_ref() },
                c"errmsg",
                K_BOOL_VAR_FALSE_CONST,
            ) != 0
        {
            emsgoff = false;
        }

        let arg_str = tv_get_string(&argvars[0]);
        if crate::memory::cstr_is_empty(arg_str) {
            crate::eval::typval::tv_set_string(rettv, arg_str);
            return;
        }
        let mut cmdline_buf: Vec<c_char> = crate::memory::cstr_to_cchar_vec(arg_str);
        let mut cmdstr = cmdline_buf.as_mut_ptr();

        let mut eap: Exarg = Exarg::new();
        eap.cmd = cmdstr;
        eap.arg = cmdstr;
        eap.cmdlinep = &raw mut cmdstr;
        eap.cmdline_buf = &raw mut cmdline_buf;
        eap.usefilter = 0;
        eap.nextcmd = ptr::null_mut();
        eap.cmdidx = Cmdidx::CmdUser as i32;
        eap.argt |= EX_NOSPC;

        if emsgoff {
            g_st().emsg_off += 1;
        }
        if !expand_filename(&mut eap, &raw mut cmdstr, &mut errormsg)
            && !emsgoff
            && !crate::memory::cstr_is_empty(errormsg)
        {
            emsg(errormsg);
        }
        if emsgoff {
            g_st().emsg_off -= 1;
        }

        crate::eval::typval::tv_set_string(rettv, cmdstr);
    }
}

/// "flatten()" and "flattennew()" functions
fn flatten_common(argvars: &mut [TypVal], rettv: &mut TypVal, make_copy: bool) {
    {
        let mut error = false;
        if argvars[0].v_type != VarType::VarList {
            semsg!(gettext(E_LISTARG), c"flatten()");
            return;
        }

        let maxdepth: i32 = if argvars[1].v_type == VarType::VarUnknown {
            999999
        } else {
            let md = tv_get_number_chk(&argvars[1], Some(&mut error)) as i32;
            if error {
                return;
            }
            if md < 0 {
                emsg(gettext(c"E900: maxdepth must be non-negative number"));
                return;
            }
            md
        };

        let mut list = argvars[0].vval.v_list();
        rettv.v_type = VarType::VarList;
        rettv.vval = crate::eval::typval::TypvalVval::List(list);
        if list.is_null() {
            return;
        }

        if make_copy {
            list = tv_list_copy(None, unsafe { list.as_mut() }, false, get_copy_id());
            rettv.vval = crate::eval::typval::TypvalVval::List(list);
            if list.is_null() {
                return;
            }
        } else {
            if value_check_lock(
                tv_list_locked(unsafe { list.as_ref() }),
                c"flatten() argument",
                TV_TRANSLATE,
            ) {
                return;
            }
            tv_list_ref(unsafe { list.as_mut() });
        }

        tv_list_flatten(
            unsafe { &mut *list },
            ptr::null_mut(),
            i64::from(tv_list_len(unsafe { list.as_ref() })),
            i64::from(maxdepth),
        );
    }
}

/// "flatten(list[, {maxdepth}])" function
pub fn f_flatten(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    flatten_common(argvars, rettv, false);
}

/// "flattennew(list[, {maxdepth}])" function
pub fn f_flattennew(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    flatten_common(argvars, rettv, true);
}

/// "feedkeys()" function
pub fn f_feedkeys(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if check_secure() {
            return;
        }
        let keys = tv_get_string(&argvars[0]);
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut flags: *const c_char = ptr::null();
        if argvars[1].v_type != VarType::VarUnknown {
            flags = tv_get_string_buf(&argvars[1], nbuf.as_mut_ptr());
        }
        nvim_feedkeys(cstr_as_string(keys), cstr_as_string(flags), true);
    }
}

/// "float2nr({float})" function
pub fn f_float2nr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut f: TvFloat = 0.0;
    if !tv_get_float_chk(&argvars[0], &mut f) {
        return;
    }
    if f <= (-VARNUMBER_MAX as TvFloat) + f64::EPSILON {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-VARNUMBER_MAX);
    } else if f >= (VARNUMBER_MAX as TvFloat) - f64::EPSILON {
        rettv.vval = crate::eval::typval::TypvalVval::Number(VARNUMBER_MAX);
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Number(f as Varnumber);
    }
}

/// "fmod()" function
pub fn f_fmod(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut fx: TvFloat = 0.0;
    let mut fy: TvFloat = 0.0;
    rettv.v_type = VarType::VarFloat;
    if tv_get_float_chk(&argvars[0], &mut fx) && tv_get_float_chk(&argvars[1], &mut fy) {
        rettv.vval = crate::eval::typval::TypvalVval::Float(fx % fy);
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Float(0.0);
    }
}

/// "fnameescape({string})" function
pub fn f_fnameescape(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let escaped = vim_strsave_fnameescape(tv_get_string(&argvars[0]), VSE_NONE);
    crate::eval::typval::tv_set_string_len(rettv, escaped.as_ptr(), escaped.as_bytes().len());
}

/// "foreground()" function
pub fn f_foreground(_argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {}

/// "function()" and "funcref()" common implementation
fn common_function(argvars: &mut [TypVal], rettv: &mut TypVal, is_funcref: bool) {
    unsafe {
        let mut s: *mut c_char;
        let mut name: *mut c_char;
        let mut use_string = false;
        let mut arg_pt: *mut TvPartial = ptr::null_mut();
        let mut trans_name_owned: Option<std::ffi::CString> = None;

        if argvars[0].v_type == VarType::VarFunc {
            s = argvars[0].vval.v_string();
        } else if argvars[0].v_type == VarType::VarPartial && !argvars[0].vval.v_partial().is_null()
        {
            arg_pt = argvars[0].vval.v_partial();
            s = partial_name(arg_pt.as_ref());
        } else {
            s = tv_get_string(&argvars[0]).cast_mut();
            use_string = true;
        }

        if (use_string && !crate::strings::vim_haschr(s, AUTOLOAD_CHAR)) || is_funcref {
            name = s;
            trans_name_owned = save_function_name(
                &raw mut name,
                false,
                TFN_INT | TFN_QUIET | TFN_NO_AUTOLOAD | TFN_NO_DEREF,
                None,
            );
            if !crate::memory::cstr_is_empty(name) {
                s = ptr::null_mut();
            }
        }
        let trans_name = trans_name_owned
            .as_ref()
            .map_or(ptr::null_mut(), |tn| tn.as_ptr().cast_mut());
        'theend: {
            if crate::memory::cstr_is_empty(s)
                || (use_string && ascii_isdigit(crate::memory::cstr_first_char(s)))
                || (is_funcref && trans_name.is_null())
            {
                semsg!(
                    gettext(E_INVARG2),
                    if use_string {
                        tv_get_string(&argvars[0])
                    } else {
                        s
                    },
                );
            } else if !trans_name.is_null()
                && if is_funcref {
                    find_func(trans_name).is_null()
                } else {
                    !translated_function_exists(trans_name)
                }
            {
                semsg!(gettext(c"E700: Unknown function: %s"), s);
            } else {
                let mut dict_idx: usize = 0;
                let mut arg_idx: usize = 0;
                let mut list: *mut TvList = ptr::null_mut();
                let name_cstr = if (crate::memory::cstr_prefix_bytes(s, 2) == &b"s:"[..2])
                    || (crate::memory::cstr_prefix_bytes(s, 5) == &b"<SID>"[..5])
                {
                    get_scriptlocal_funcname(s).unwrap_or_default()
                } else {
                    cstr_to_cstring(s)
                };

                if argvars[1].v_type != VarType::VarUnknown {
                    if argvars[2].v_type != VarType::VarUnknown {
                        arg_idx = 1;
                        dict_idx = 2;
                    } else if argvars[1].v_type == VarType::VarDict {
                        dict_idx = 1;
                    } else {
                        arg_idx = 1;
                    }
                    if dict_idx > 0 {
                        if !tv_check_for_dict_arg(argvars, dict_idx as i32) {
                            break 'theend;
                        }
                        if argvars[dict_idx].vval.v_dict().is_null() {
                            dict_idx = 0;
                        }
                    }
                    if arg_idx > 0 {
                        if argvars[arg_idx].v_type != VarType::VarList {
                            emsg(gettext(
                                c"E923: Second argument of function() must be a list or a dict",
                            ));
                            break 'theend;
                        }
                        list = argvars[arg_idx].vval.v_list();
                        if tv_list_len(list.as_ref()) == 0 {
                            arg_idx = 0;
                        } else if (tv_list_len(list.as_ref()) as usize) > MAX_FUNC_ARGS {
                            emsg_funcname(E_TOOMANYARG, s);
                            break 'theend;
                        }
                    }
                }
                if dict_idx > 0 || arg_idx > 0 || !arg_pt.is_null() || is_funcref {
                    let pt = Box::leak(Box::new(TvPartial::default()));
                    if arg_idx > 0 || (!arg_pt.is_null() && !(*arg_pt).pt_argv.is_empty()) {
                        let arg_len = if arg_pt.is_null() {
                            0
                        } else {
                            (*arg_pt).pt_argv.len() as i32
                        };
                        let lv_len = tv_list_len(list.as_ref());
                        let pt_argc = arg_len + lv_len;
                        pt.pt_argv.resize(pt_argc as usize, TypVal::new());
                        let mut i: i32 = 0;
                        while i < arg_len {
                            tv_copy(
                                &(&(*arg_pt).pt_argv)[i as usize],
                                &mut (&mut pt.pt_argv)[i as usize],
                            );
                            i += 1;
                        }
                        if lv_len > 0 {
                            for li in
                                crate::eval::typval::list_iter_mut(tv_list_first(list.as_ref()))
                            {
                                tv_copy(&li.li_tv, &mut (&mut pt.pt_argv)[i as usize]);
                                i += 1;
                            }
                        }
                    }

                    if dict_idx > 0 {
                        pt.pt_dict = argvars[dict_idx].vval.v_dict();
                        (*pt.pt_dict).dv_refcount += 1;
                    } else if !arg_pt.is_null() {
                        let arg_pt = { &mut *arg_pt };
                        pt.pt_dict = arg_pt.pt_dict;
                        pt.pt_auto = arg_pt.pt_auto;
                        if !pt.pt_dict.is_null() {
                            (*pt.pt_dict).dv_refcount += 1;
                        }
                    }

                    pt.pt_refcount = 1;
                    if !arg_pt.is_null() && !(*arg_pt).pt_func.is_null() {
                        pt.pt_func = (*arg_pt).pt_func;
                        func_ptr_ref((pt.pt_func).as_mut());
                    } else if is_funcref {
                        pt.pt_func = find_func(trans_name);
                        func_ptr_ref((pt.pt_func).as_mut());
                    } else {
                        func_ref(name_cstr.as_ptr().cast_mut());
                        pt.pt_name = Some(name_cstr);
                    }

                    rettv.v_type = VarType::VarPartial;
                    rettv.vval = crate::eval::typval::TypvalVval::Partial(pt);
                } else {
                    crate::eval::typval::tv_set_string(&mut *rettv, name_cstr.as_ptr());
                    rettv.v_type = VarType::VarFunc;
                    func_ref(rettv.vval.v_string());
                }
            }
        }
        drop(trans_name_owned);
    }
}

pub fn f_funcref(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    common_function(argvars, rettv, true);
}

pub fn f_function(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    common_function(argvars, rettv, false);
}

/// "garbagecollect()" function
pub fn f_garbagecollect(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        g_st().want_garbage_collect = true;
        if argvars[0].v_type != VarType::VarUnknown && tv_get_bool(&argvars[0]) == 1 {
            g_st().garbage_collect_at_exit = true;
        }
    }
}

/// "get()" function
pub fn f_get(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut tv: *mut TypVal = ptr::null_mut();
        let mut what_is_dict = false;

        if argvars[0].v_type == VarType::VarBlob {
            let mut error = false;
            let mut idx = tv_get_number_chk(&argvars[1], Some(&mut error)) as i32;
            if !error {
                let blob = argvars[0].vval.v_blob().cast::<TvBlob>();
                rettv.v_type = VarType::VarNumber;
                if idx < 0 {
                    idx += tv_blob_len(blob.as_ref());
                }
                if idx < 0 || idx >= tv_blob_len(blob.as_ref()) {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
                } else {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
                        tv_blob_get(&*blob, idx),
                    ));
                    tv = rettv;
                }
            }
        } else if argvars[0].v_type == VarType::VarList {
            let l = argvars[0].vval.v_list();
            if !l.is_null() {
                let mut error = false;
                let li = tv_list_find(
                    l.as_mut(),
                    tv_get_number_chk(&argvars[1], Some(&mut error)) as i32,
                );
                if !error && !li.is_null() {
                    tv = &raw mut (*li).li_tv;
                }
            }
        } else if argvars[0].v_type == VarType::VarDict {
            let d = argvars[0].vval.v_dict();
            if !d.is_null() {
                let di = tv_dict_find(d.as_ref(), tv_get_string(&argvars[1]), -1);
                if !di.is_null() {
                    tv = &raw mut (*di).di_tv;
                }
            }
        } else if tv_is_func(&argvars[0]) {
            let mut fref_pt = std::mem::ManuallyDrop::new(TvPartial::default());
            let pt: *mut TvPartial = if argvars[0].v_type == VarType::VarPartial {
                argvars[0].vval.v_partial()
            } else {
                fref_pt.pt_name = Some(cstr_to_cstring(argvars[0].vval.v_string()));
                &raw mut *fref_pt
            };

            if !pt.is_null() {
                let pt = { &mut *pt };
                let what = tv_get_string(&argvars[1]);
                if crate::memory::cstr_as_bytes(what) == b"func"
                    || crate::memory::cstr_as_bytes(what) == b"name"
                {
                    let mut name = partial_name(Some(&*pt));
                    debug_assert!(!name.is_null());
                    if crate::memory::cstr_first_byte(what) == b'f' {
                        func_ref(name);
                    }
                    if crate::memory::cstr_first_byte(what) == b'n'
                        && pt.pt_name.is_none()
                        && !pt.pt_func.is_null()
                    {
                        name = printable_func_name(&mut *pt.pt_func);
                    }
                    crate::eval::typval::tv_set_string(rettv, name);
                    rettv.v_type = if crate::memory::cstr_first_byte(what) == b'f' {
                        VarType::VarFunc
                    } else {
                        VarType::VarString
                    };
                } else if crate::memory::cstr_as_bytes(what) == b"dict" {
                    what_is_dict = true;
                    if !pt.pt_dict.is_null() {
                        tv_dict_set_ret(rettv, pt.pt_dict);
                    }
                } else if crate::memory::cstr_as_bytes(what) == b"args" {
                    rettv.v_type = VarType::VarList;
                    tv_list_alloc_ret(rettv, pt.pt_argv.len() as isize);
                    for arg in &mut pt.pt_argv {
                        tv_list_append_tv(rettv.vval.v_list_ref(), &mut *arg);
                    }
                } else if crate::memory::cstr_as_bytes(what) == b"arity" {
                    let mut required: i32 = 0;
                    let mut optional: i32 = 0;
                    let mut varargs = false;
                    let name = partial_name(Some(&*pt));
                    get_func_arity(name, &mut required, &mut optional, &mut varargs);

                    rettv.v_type = VarType::VarDict;
                    let dict = tv_dict_alloc_ret(rettv);
                    let pt_argc = pt.pt_argv.len() as i32;

                    if pt_argc >= required + optional {
                        required = 0;
                        optional = 0;
                    } else if pt_argc > required {
                        optional -= pt_argc - required;
                        required = 0;
                    } else {
                        required -= pt_argc;
                    }

                    tv_dict_add_nr(dict, c"required", Varnumber::from(required));
                    tv_dict_add_nr(dict, c"optional", Varnumber::from(optional));
                    tv_dict_add_bool(
                        dict,
                        c"varargs",
                        if varargs {
                            K_BOOL_VAR_TRUE_CONST
                        } else {
                            K_BOOL_VAR_FALSE_CONST
                        },
                    );
                } else {
                    semsg!(gettext(E_INVARG2), what);
                }

                if !what_is_dict {
                    return;
                }
            }
        } else {
            semsg!(gettext(E_LISTDICTBLOBARG), c"get()");
        }

        if tv.is_null() {
            if argvars[2].v_type != VarType::VarUnknown {
                tv_copy(&argvars[2], rettv);
            }
        } else {
            tv_copy(&*tv, rettv);
        }
    }
}

/// "getchangelist()" function
pub fn f_getchangelist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);
        let buf: *const FileBuf = if argvars[0].v_type == VarType::VarUnknown {
            g_st().curbuf
        } else {
            g_st().vim_ignored = tv_get_number(&argvars[0]) as i32;
            g_st().emsg_off += 1;
            let b = tv_get_buf(&mut argvars[0], 0);
            g_st().emsg_off -= 1;
            b
        };
        if buf.is_null() {
            return;
        }

        let l = tv_list_alloc((*buf).b_changelistlen as isize);
        tv_list_append_list(rettv.vval.v_list_ref(), l);
        let mut changelistindex = if ptr::eq(buf, curwin_mut().w_buffer) {
            curwin_mut().w_changelistidx
        } else {
            let mut idx = (*buf).b_changelistlen;
            for wip in &(*buf).b_wininfo {
                if wip.wi_win == curwin_ptr() {
                    idx = wip.wi_changelistidx;
                    break;
                }
            }
            idx
        };
        let _ = &mut changelistindex;
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(changelistindex));

        for i in 0..((*buf).b_changelistlen as usize) {
            if (*buf).b_changelist[i].mark.lnum == 0 {
                continue;
            }
            let d = tv_dict_alloc();
            tv_list_append_dict(&mut *l, d);
            tv_dict_add_nr(
                &mut *d,
                c"lnum",
                Varnumber::from((*buf).b_changelist[i].mark.lnum),
            );
            tv_dict_add_nr(
                &mut *d,
                c"col",
                Varnumber::from((*buf).b_changelist[i].mark.col),
            );
            tv_dict_add_nr(
                &mut *d,
                c"coladd",
                Varnumber::from((*buf).b_changelist[i].mark.coladd),
            );
        }
    }
}

fn getpos_both(argvars: &mut [TypVal], rettv: &mut TypVal, getcurpos: bool, charcol: bool) {
    unsafe {
        let mut fp: *mut CursorPos = ptr::null_mut();
        let mut pos: CursorPos;
        let mut wp = curwin_ptr();
        let mut fnum: i32 = -1;

        if getcurpos {
            if argvars[0].v_type != VarType::VarUnknown {
                wp = find_win_by_nr_or_id(&argvars[0]);
                if !wp.is_null() {
                    fp = (&raw mut (*wp).w_cursor).cast();
                }
            } else {
                fp = (&raw mut curwin_mut().w_cursor).cast();
            }
            if !fp.is_null() && charcol {
                pos = *fp;
                pos.col = buf_byteidx_to_charidx(((*wp).w_buffer).as_mut(), pos.lnum, pos.col);
                fp = &raw mut pos;
            }
        } else {
            fp = var2fpos(&argvars[0], true, &mut fnum, charcol, curwin_mut());
        }

        let l = tv_list_alloc_ret(&mut *rettv, 4 + isize::from(getcurpos));
        tv_list_append_number(&mut *l, if fnum != -1 { Varnumber::from(fnum) } else { 0 });
        tv_list_append_number(
            &mut *l,
            if !fp.is_null() {
                Varnumber::from((*fp).lnum)
            } else {
                0
            },
        );
        tv_list_append_number(
            &mut *l,
            if !fp.is_null() {
                let fp = { &mut *fp };
                if fp.col == MAXCOL {
                    Varnumber::from(MAXCOL)
                } else {
                    Varnumber::from(fp.col) + 1
                }
            } else {
                0
            },
        );
        tv_list_append_number(
            &mut *l,
            if !fp.is_null() {
                Varnumber::from((*fp).coladd)
            } else {
                0
            },
        );
        if getcurpos {
            let save_set_curswant = curwin_mut().w_set_curswant;
            let save_curswant = curwin_mut().w_curswant;
            let save_virtcol = curwin_mut().w_virtcol;

            if wp == curwin_ptr() {
                update_curswant();
            }
            tv_list_append_number(
                &mut *l,
                if wp.is_null() {
                    0
                } else if (*wp).w_curswant == MAXCOL {
                    Varnumber::from(MAXCOL)
                } else {
                    Varnumber::from((*wp).w_curswant) + 1
                },
            );

            if wp == curwin_ptr() && save_set_curswant {
                curwin_mut().w_set_curswant = save_set_curswant;
                curwin_mut().w_curswant = save_curswant;
                curwin_mut().w_virtcol = save_virtcol;
                curwin_mut().w_valid &= !VALID_VIRTCOL;
            }
        }
    }
}

/// "getcharpos()" function
pub fn f_getcharpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getpos_both(argvars, rettv, false, true);
}

/// "getcharsearch()" function
pub fn f_getcharsearch(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let dict = tv_dict_alloc_ret(rettv);
    tv_dict_add_str(dict, c"char", last_csearch());
    tv_dict_add_nr(dict, c"forward", Varnumber::from(last_csearch_forward()));
    tv_dict_add_nr(dict, c"until", Varnumber::from(last_csearch_until()));
}

/// "getfontname()" function
pub fn f_getfontname(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
}

/// "getjumplist()" function
pub fn f_getjumplist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        let wp = find_tabwin(&argvars[0], &argvars[1]);
        if wp.is_null() {
            return;
        }
        let wp = unsafe { &mut *wp };

        cleanup_jumplist(&mut *wp, true);

        let l = tv_list_alloc(wp.w_jumplistlen as isize);
        tv_list_append_list(rettv.vval.v_list_ref(), l);
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(wp.w_jumplistidx));

        for i in 0..(wp.w_jumplistlen as usize) {
            if wp.w_jumplist[i].fmark.mark.lnum == 0 {
                continue;
            }
            let d = tv_dict_alloc();
            tv_list_append_dict(&mut *l, d);
            tv_dict_add_nr(
                &mut *d,
                c"lnum",
                Varnumber::from(wp.w_jumplist[i].fmark.mark.lnum),
            );
            tv_dict_add_nr(
                &mut *d,
                c"col",
                Varnumber::from(wp.w_jumplist[i].fmark.mark.col),
            );
            tv_dict_add_nr(
                &mut *d,
                c"coladd",
                Varnumber::from(wp.w_jumplist[i].fmark.mark.coladd),
            );
            tv_dict_add_nr(
                &mut *d,
                c"bufnr",
                Varnumber::from(wp.w_jumplist[i].fmark.fnum),
            );
            if let Some(ref fname) = wp.w_jumplist[i].fname {
                tv_dict_add_str(&mut *d, c"filename", fname.as_ptr());
            }
        }
    }
}

/// "getmarklist()" function
pub fn f_getmarklist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        if argvars[0].v_type == VarType::VarUnknown {
            get_global_marks(rettv.vval.v_list_ref());
            return;
        }
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() {
            return;
        }
        get_buf_local_marks(unsafe { &*buf }, rettv.vval.v_list_ref());
    }
}

/// "getpid()" function
pub fn f_getpid(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(os_get_pid());
}

/// "getcurpos(string)" function
pub fn f_getcurpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getpos_both(argvars, rettv, true, false);
}

pub fn f_getcursorcharpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getpos_both(argvars, rettv, true, true);
}

/// "getpos(string)" function
pub fn f_getpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getpos_both(argvars, rettv, false, false);
}

/// Convert from `BlockDef` to a byte vector.
fn block_def2str(bd: &BlockDef) -> Vec<c_char> {
    {
        let startspaces = bd.startspaces as usize;
        let endspaces = bd.endspaces as usize;
        let textlen = bd.textlen as usize;
        let size = startspaces + endspaces + textlen;
        let mut data = vec![b' ' as c_char; size];
        unsafe {
            ptr::copy(
                bd.textstart,
                data.as_mut_ptr().wrapping_add(startspaces),
                textlen,
            )
        };
        data
    }
}

fn getregionpos(
    argvars: &mut [TypVal],
    rettv: &mut TypVal,
    p1: &mut CursorPos,
    p2: &mut CursorPos,
    inclusive: &mut bool,
    region_type: &mut i32,
    oap: &mut Oparg,
) -> bool {
    unsafe {
        tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_MAY_KNOW_CONST);

        if !tv_check_for_list_arg(argvars, 0)
            || !tv_check_for_list_arg(argvars, 1)
            || !tv_check_for_opt_dict_arg(argvars, 2)
        {
            return false;
        }

        let mut fnum1: i32 = -1;
        let mut fnum2: i32 = -1;
        if !list2fpos(&mut argvars[0], p1, Some(&mut fnum1), None, false)
            || !list2fpos(&mut argvars[1], p2, Some(&mut fnum2), None, false)
            || fnum1 != fnum2
        {
            return false;
        }

        let is_select_exclusive: bool;
        let mut type_str: *mut c_char;
        let mut default_type = [b'v' as c_char, NUL];
        if argvars[2].v_type == VarType::VarDict {
            let d = argvars[2].vval.v_dict();
            is_select_exclusive = tv_dict_get_bool(
                d.as_ref(),
                c"exclusive",
                i32::from(crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'),
            ) != 0;
            type_str = tv_dict_get_string(d.as_ref(), c"type");
            if type_str.is_null() {
                type_str = default_type.as_mut_ptr();
            }
        } else {
            is_select_exclusive = crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e';
            type_str = default_type.as_mut_ptr();
        }

        let mut block_width: i32 = 0;
        if crate::memory::cstr_first_byte(type_str) == b'v'
            && crate::memory::cstr_byte_at(type_str, 1) == 0
        {
            *region_type = MT_CHAR_WISE;
        } else if crate::memory::cstr_first_byte(type_str) == b'V'
            && crate::memory::cstr_byte_at(type_str, 1) == 0
        {
            *region_type = MT_LINE_WISE;
        } else if *type_str == CTRL_V_CONST {
            let mut p = type_str.wrapping_add(1);
            if !crate::memory::cstr_is_empty(p) {
                block_width = getdigits_int(&raw mut p, false, 0);
                if block_width <= 0 || !crate::memory::cstr_is_empty(p) {
                    semsg!(gettext(E_INVARG_NVAL), c"type", type_str);
                    return false;
                }
            }
            *region_type = MT_BLOCK_WISE;
        } else {
            semsg!(gettext(E_INVARG_NVAL), c"type", type_str);
            return false;
        }

        let findbuf = if fnum1 != 0 {
            buflist_findnr(fnum1)
        } else {
            g_st().curbuf
        };
        if findbuf.is_null() || (*findbuf).b_ml.ml_mfp.is_none() {
            emsg(gettext(E_BUFFER_IS_NOT_LOADED));
            return false;
        }

        if p1.lnum < 1 || p1.lnum > (*findbuf).b_ml.ml_line_count {
            semsg!(gettext(E_INVALID_LINE_NUMBER_NR), p1.lnum);
            return false;
        }
        if p1.col == MAXCOL {
            p1.col = ml_get_buf_len(&mut *findbuf, p1.lnum) + 1;
        } else if p1.col < 1 || p1.col > ml_get_buf_len(&mut *findbuf, p1.lnum) + 1 {
            semsg!(gettext(E_INVALID_COLUMN_NUMBER_NR), p1.col);
            return false;
        }

        if p2.lnum < 1 || p2.lnum > (*findbuf).b_ml.ml_line_count {
            semsg!(gettext(E_INVALID_LINE_NUMBER_NR), p2.lnum);
            return false;
        }
        if p2.col == MAXCOL {
            p2.col = ml_get_buf_len(&mut *findbuf, p2.lnum) + 1;
        } else if p2.col < 1 || p2.col > ml_get_buf_len(&mut *findbuf, p2.lnum) + 1 {
            semsg!(gettext(E_INVALID_COLUMN_NUMBER_NR), p2.col);
            return false;
        }

        g_st().curbuf = findbuf;
        curwin_mut().w_buffer = g_st().curbuf;
        g_st().virtual_op = TriState::from(virtual_active(curwin_mut()));

        p1.col -= 1;
        p2.col -= 1;

        getregionpos_prep(
            p1,
            p2,
            *region_type,
            is_select_exclusive,
            block_width,
            &mut *inclusive,
            oap,
        );
        true
    }
}

impl CursorPos {
    #[inline]
    fn cast(&mut self) -> *mut crate::cursor::CursorPos {
        (self as *mut CursorPos).cast()
    }
}

/// "getregion()" function
pub fn f_getregion(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let save_curbuf = g_st().curbuf;
        let save_virtual = g_st().virtual_op;

        let mut p1 = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut p2 = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut region_type = MT_UNKNOWN;
        let mut oa = Oparg::new();

        if !getregionpos(
            argvars,
            &mut *rettv,
            &mut p1,
            &mut p2,
            &mut inclusive,
            &mut region_type,
            &mut oa,
        ) {
            return;
        }

        for lnum in p1.lnum..=p2.lnum {
            if region_type == MT_BLOCK_WISE {
                let mut bd = BlockDef::new();
                block_prep(&mut oa, &mut bd, lnum, false);
                let akt = block_def2str(&bd);
                tv_list_append_string(rettv.vval.v_list_ref(), akt.as_ptr(), akt.len() as isize);
            } else if region_type == MT_LINE_WISE || (p1.lnum < lnum && lnum < p2.lnum) {
                tv_list_append_string(
                    rettv.vval.v_list_ref(),
                    ml_get(lnum),
                    ml_get_len(lnum) as isize,
                );
            } else {
                let mut bd = BlockDef::new();
                charwise_block_prep(
                    crate::cursor::CursorPos {
                        lnum: p1.lnum,
                        col: p1.col,
                        coladd: p1.coladd,
                    },
                    crate::cursor::CursorPos {
                        lnum: p2.lnum,
                        col: p2.col,
                        coladd: p2.coladd,
                    },
                    &mut bd,
                    lnum,
                    inclusive,
                );
                let akt = block_def2str(&bd);
                tv_list_append_string(rettv.vval.v_list_ref(), akt.as_ptr(), akt.len() as isize);
            }
        }

        g_st().curbuf = save_curbuf;
        curwin_mut().w_buffer = g_st().curbuf;
        g_st().virtual_op = save_virtual;
    }
}

fn add_regionpos_range(rettv: &mut TypVal, p1: CursorPos, p2: CursorPos) {
    {
        let l1 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), l1);

        let l2 = tv_list_alloc(4);
        tv_list_append_list(&mut *l1, l2);

        let l3 = tv_list_alloc(4);
        tv_list_append_list(&mut *l1, l3);

        tv_list_append_number(&mut *l2, Varnumber::from(curbuf_mut().b_fnum));
        tv_list_append_number(&mut *l2, Varnumber::from(p1.lnum));
        tv_list_append_number(&mut *l2, Varnumber::from(p1.col));
        tv_list_append_number(&mut *l2, Varnumber::from(p1.coladd));

        tv_list_append_number(&mut *l3, Varnumber::from(curbuf_mut().b_fnum));
        tv_list_append_number(&mut *l3, Varnumber::from(p2.lnum));
        tv_list_append_number(&mut *l3, Varnumber::from(p2.col));
        tv_list_append_number(&mut *l3, Varnumber::from(p2.coladd));
    }
}

/// "getregionpos()" function
pub fn f_getregionpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let save_curbuf = g_st().curbuf;
        let save_virtual = g_st().virtual_op;

        let mut p1 = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut p2 = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut region_type = MT_UNKNOWN;
        let mut allow_eol = false;
        let mut bounds_only = false;
        let mut oa = Oparg::new();

        if !getregionpos(
            argvars,
            &mut *rettv,
            &mut p1,
            &mut p2,
            &mut inclusive,
            &mut region_type,
            &mut oa,
        ) {
            return;
        }

        if argvars[2].v_type == VarType::VarDict {
            let d = argvars[2].vval.v_dict();
            allow_eol = tv_dict_get_bool(unsafe { d.as_ref() }, c"eol", 0) != 0;
            bounds_only = tv_dict_get_bool(unsafe { d.as_ref() }, c"bounds", 0) != 0;
        }

        let tab_p1 = crate::cursor::CursorPos {
            lnum: p1.lnum,
            col: p1.col,
            coladd: p1.coladd,
        };
        let tab_p2 = crate::cursor::CursorPos {
            lnum: p2.lnum,
            col: p2.col,
            coladd: p2.coladd,
        };

        if bounds_only {
            let mut start_pos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut end_pos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            getregionpos_line(
                p1.lnum,
                tab_p1,
                tab_p2,
                inclusive,
                region_type,
                &mut oa,
                allow_eol,
                &mut start_pos,
                &mut end_pos,
            );
            if p2.lnum != p1.lnum {
                let mut unused = CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                getregionpos_line(
                    p2.lnum,
                    tab_p1,
                    tab_p2,
                    inclusive,
                    region_type,
                    &mut oa,
                    allow_eol,
                    &mut unused,
                    &mut end_pos,
                );
            }
            add_regionpos_range(rettv, start_pos, end_pos);
        } else {
            for lnum in p1.lnum..=p2.lnum {
                let mut ret_p1 = CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                let mut ret_p2 = CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                getregionpos_line(
                    lnum,
                    tab_p1,
                    tab_p2,
                    inclusive,
                    region_type,
                    &mut oa,
                    allow_eol,
                    &mut ret_p1,
                    &mut ret_p2,
                );
                add_regionpos_range(rettv, ret_p1, ret_p2);
            }
        }

        g_st().curbuf = save_curbuf;
        curwin_mut().w_buffer = g_st().curbuf;
        g_st().virtual_op = save_virtual;
    }
}

/// Common between `getreg()`, `getreginfo()` and `getregtype()`.
fn getreg_get_regname(argvars: &mut [TypVal]) -> i32 {
    {
        let strregname = if argvars[0].v_type != VarType::VarUnknown {
            let s = tv_get_string_chk(&argvars[0]);
            if s.is_null() {
                return 0;
            }
            s
        } else {
            get_vim_var_str(VV_REG)
        };
        if unsafe { *strregname } == 0 {
            b'"' as i32
        } else {
            i32::from(crate::memory::cstr_first_byte(strregname))
        }
    }
}

/// "getreg()" function
pub fn f_getreg(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut arg2: i32 = 0;
        let mut return_list = false;

        let regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }

        if argvars[0].v_type != VarType::VarUnknown && argvars[1].v_type != VarType::VarUnknown {
            let mut error = false;
            arg2 = tv_get_bool_chk(&argvars[1], Some(&mut error)) as i32;
            if !error && argvars[2].v_type != VarType::VarUnknown {
                return_list = tv_get_bool_chk(&argvars[2], Some(&mut error)) != 0;
            }
            if error {
                return;
            }
        }

        if return_list {
            rettv.v_type = VarType::VarList;
            rettv.vval = crate::eval::typval::TypvalVval::List(get_reg_contents_list(
                regname,
                if arg2 != 0 { K_GREG_EXPR_SRC } else { 0 },
            ));
            if rettv.vval.v_list().is_null() {
                rettv.vval = crate::eval::typval::TypvalVval::List(tv_list_alloc(0));
            }
            tv_list_ref(unsafe { rettv.vval.v_list().as_mut() });
        } else {
            let reg_str =
                get_reg_contents_str(regname, if arg2 != 0 { K_GREG_EXPR_SRC } else { 0 });
            crate::eval::typval::tv_set_string(
                rettv,
                reg_str.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            );
        }
    }
}

/// "getregtype()" function
pub fn f_getregtype(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    let regname = getreg_get_regname(argvars);
    if regname == 0 {
        return;
    }

    let mut reglen: Colnr = 0;
    let mut buf = [0 as c_char; NUMBUFLEN + 2];
    let reg_type = get_reg_type(regname, Some(&mut reglen));
    format_reg_type(reg_type, reglen, buf.as_mut_ptr(), buf.len());

    crate::eval::typval::tv_set_string(rettv, buf.as_ptr());
}

/// "gettagstack()" function
pub fn f_gettagstack(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut wp = curwin_ptr();
        tv_dict_alloc_ret(rettv);
        if argvars[0].v_type != VarType::VarUnknown {
            wp = find_win_by_nr_or_id(&argvars[0]);
            if wp.is_null() {
                return;
            }
        }
        get_tagstack(unsafe { &mut *wp }, rettv.vval.v_dict_ref());
    }
}

pub(crate) fn dummy_timer_due_cb(tw: &mut TimeWatcher) {
    {
        if unsafe { *loop_closing_ptr(&mut g_st().main_loop) } {
            time_watcher_stop(tw);
            time_watcher_close(tw);
        }
    }
}

/// "wait(timeout, condition[, interval])" function
pub fn f_wait(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if argvars[0].v_type != VarType::VarNumber {
            semsg!(gettext(E_INVARGVAL), c"1");
            return;
        }
        if (argvars[2].v_type != VarType::VarNumber && argvars[2].v_type != VarType::VarUnknown)
            || (argvars[2].v_type == VarType::VarNumber && argvars[2].vval.v_number() <= 0)
        {
            semsg!(gettext(E_INVARGVAL), c"3");
            return;
        }

        let timeout = argvars[0].vval.v_number() as i32;
        let expr = argvars[1];
        let interval = if argvars[2].v_type == VarType::VarNumber {
            argvars[2].vval.v_number() as i32
        } else {
            200
        };
        let tw = Box::into_raw(Box::new(TimeWatcher::new()));

        let main_loop_ptr: *mut Loop = (&raw mut g_st().main_loop).cast();
        time_watcher_init(&mut *main_loop_ptr, &mut *tw, TimeWatcherData::WaitDummy);
        (*tw).events = ptr::null_mut();
        time_watcher_start(&mut *tw, interval as u64, interval as u64);

        let mut argv = tv_initial_value();
        let mut exprval = tv_initial_value();
        let mut error = false;
        let called_emsg_before = g_st().called_emsg;

        ui_flush();

        let mut remaining = i64::from(timeout);
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !(!eval_expr_typval(&expr, false, &raw mut argv, 0, &mut exprval)
            || tv_get_number_chk(&exprval, Some(&mut error)) != 0
            || g_st().called_emsg > called_emsg_before
            || error
            || g_st().got_int)
        {
            let events = loop_queues(&mut *main_loop_ptr).events;
            if !events.is_null() && !multiqueue_empty(&*events) {
                multiqueue_process_events(&mut *events);
            } else {
                loop_poll_events(&mut *main_loop_ptr, remaining);
            }
            if remaining == 0 {
                break;
            } else if remaining > 0 {
                let now = os_hrtime();
                remaining -= ((now - before) / 1_000_000) as i64;
                before = now;
                if remaining <= 0 {
                    break;
                }
            }
        }

        if g_st().called_emsg > called_emsg_before || error {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-3);
        } else if g_st().got_int {
            g_st().got_int = false;
            vgetc();
            rettv.vval = crate::eval::typval::TypvalVval::Number(-2);
        } else if tv_get_number_chk(&exprval, Some(&mut error)) != 0 {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }

        time_watcher_stop(&mut *tw);
        time_watcher_close(&mut *tw);
    }
}

/// "gettext()" function
pub fn f_gettext(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if !tv_check_for_nonempty_string_arg(argvars, 0) {
            return;
        }
        crate::eval::typval::tv_set_string(rettv, gettext(argvars[0].vval.v_string()));
    }
}

/// "has()" function
pub fn f_has(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut x = false;
        let mut n = false;
        let name = tv_get_string(&argvars[0]);

        if crate::memory::cstr_prefix_bytes(name, 5).eq_ignore_ascii_case(b"patch") {
            x = true;
            if crate::memory::cstr_byte_at(name, 5) == b'-'
                && crate::memory::cstr_len(name) >= 11
                && (crate::memory::cstr_byte_at(name, 6) >= b'1'
                    && crate::memory::cstr_byte_at(name, 6) <= b'9')
            {
                let s6 = name.wrapping_add(6);
                let dlen = crate::memory::cstr_as_bytes(s6)
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                let end = s6.wrapping_add(dlen);
                let major =
                    unsafe { std::str::from_utf8_unchecked(crate::memory::cstr_slice(s6, dlen)) }
                        .parse::<i32>()
                        .unwrap_or(0);
                if crate::memory::cstr_first_byte(end) == b'.'
                    && ascii_isdigit(crate::memory::cstr_char_at(end, 1))
                    && crate::memory::cstr_byte_at(end, 2) == b'.'
                    && ascii_isdigit(crate::memory::cstr_char_at(end, 3))
                {
                    let minor =
                        crate::charset::parse_i64_prefix(&crate::memory::cstr_as_bytes(end)[1..])
                            as i32;
                    n = has_vim_patch(
                        crate::charset::parse_i64_prefix(&crate::memory::cstr_as_bytes(end)[3..])
                            as i32,
                        major * 100 + minor,
                    );
                }
            } else if ascii_isdigit(crate::memory::cstr_char_at(name, 5)) {
                n = has_vim_patch(
                    crate::charset::parse_i64_prefix(&crate::memory::cstr_as_bytes(name)[5..])
                        as i32,
                    0,
                );
            }
        } else if crate::memory::cstr_prefix_bytes(name, 5).eq_ignore_ascii_case(b"nvim-") {
            x = true;
            n = has_nvim_version(&crate::memory::cstr_as_bytes(name)[5..]);
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"vim_starting") {
            x = true;
            n = starting_val() != 0;
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"ttyin") {
            x = true;
            n = g_st().stdin_isatty;
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"ttyout") {
            x = true;
            n = g_st().stdout_isatty;
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"multi_byte_encoding") {
            x = true;
            n = true;
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"gui_running") {
            x = true;
            n = ui_gui_attached();
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"syntax_items") {
            x = true;
            n = syntax_present(curwin_mut());
        } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"wsl") {
            x = true;
            n = has_wsl();
        }

        if !x && eval_has_static_feature(name) {
            x = true;
            n = true;
        }

        if !x {
            let save_shell_error = get_vim_var_nr(VV_SHELL_ERROR) as i32;
            if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"clipboard_working")
                || (cfg!(not(windows))
                    && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"unnamedplus"))
            {
                n = eval_has_provider(c"clipboard", true);
            } else if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"pythonx") {
                n = eval_has_provider(c"python3", true);
            } else if eval_has_provider(name, true) {
                n = true;
            }
            set_vim_var_nr(VV_SHELL_ERROR, Varnumber::from(save_shell_error));
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(n));
    }
}

static EVAL_HAS_LIST: &[&CStr] = &[
    #[cfg(target_os = "android")]
    c"android",
    #[cfg(any(
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    c"bsd",
    #[cfg(target_os = "hurd")]
    c"hurd",
    #[cfg(target_os = "linux")]
    c"linux",
    #[cfg(target_os = "solaris")]
    c"sun",
    #[cfg(unix)]
    c"unix",
    #[cfg(windows)]
    c"win32",
    #[cfg(all(windows, target_pointer_width = "64"))]
    c"win64",
    #[cfg(not(any(windows, target_os = "macos")))]
    c"fname_case",
    c"autochdir",
    c"arabic",
    c"autocmd",
    c"browsefilter",
    c"byte_offset",
    c"cindent",
    c"cmdline_compl",
    c"cmdline_hist",
    c"cmdwin",
    c"comments",
    c"conceal",
    c"cursorbind",
    c"cursorshape",
    c"dialog_con",
    c"diff",
    c"digraphs",
    c"eval",
    c"ex_extra",
    c"extra_search",
    c"file_in_path",
    c"filterpipe",
    c"find_in_path",
    c"float",
    c"folding",
    #[cfg(unix)]
    c"fork",
    c"iconv",
    c"insert_expand",
    c"jumplist",
    c"keymap",
    c"lambda",
    c"langmap",
    c"libcall",
    c"linebreak",
    c"lispindent",
    c"listcmds",
    c"localmap",
    #[cfg(target_os = "macos")]
    c"mac",
    #[cfg(target_os = "macos")]
    c"macunix",
    #[cfg(target_os = "macos")]
    c"osx",
    #[cfg(target_os = "macos")]
    c"osxdarwin",
    c"menu",
    c"mksession",
    c"modify_fname",
    c"mouse",
    c"multi_byte",
    c"multi_lang",
    c"nanotime",
    c"num64",
    c"packages",
    c"path_extra",
    c"persistent_undo",
    c"profile",
    c"reltime",
    c"quickfix",
    c"rightleft",
    c"scrollbind",
    c"showcmd",
    c"cmdline_info",
    c"shada",
    c"signs",
    c"smartindent",
    c"startuptime",
    c"statusline",
    c"spell",
    c"syntax",
    #[cfg(not(unix))]
    c"system",
    c"tablineat",
    c"tag_binary",
    c"termguicolors",
    c"termresponse",
    c"textobjects",
    c"timers",
    c"title",
    c"user-commands",
    c"user_commands",
    c"vartabs",
    c"vertsplit",
    c"vimscript-1",
    c"virtualedit",
    c"visual",
    c"visualextra",
    c"vreplace",
    c"wildignore",
    c"wildmenu",
    c"windows",
    c"winaltkeys",
    c"writebackup",
    c"nvim",
];

fn eval_has_static_feature(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    {
        for &feat in EVAL_HAS_LIST {
            if crate::memory::cstr_as_bytes(name)
                .eq_ignore_ascii_case(crate::memory::cstr_as_bytes(feat.as_ptr()))
            {
                return true;
            }
        }
        let flags = NVIM_BUILD_HAS_FLAGS;
        ((flags & (1 << 0)) != 0 && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"acl"))
            || ((flags & (1 << 1)) != 0
                && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"terminfo"))
            || ((flags & (1 << 2)) != 0
                && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"xattr"))
            || ((flags & (1 << 3)) != 0
                && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"termux"))
            || ((flags & (1 << 4)) != 0
                && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"gettext"))
    }
}

fn has_wsl() -> bool {
    if funcs_st().has_wsl == K_NONE {
        let mut err = Error::new();
        let cmd = c"return vim.uv.os_uname()['release']:lower():match('microsoft')";
        let o = nlua_exec(
            NvimString {
                data: cmd.as_ptr().cast_mut(),
                size: cmd.to_bytes().len(),
            },
            ptr::null(),
            Array {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            },
            K_RET_NIL_BOOL,
            None,
            &mut err,
        );
        debug_assert!(err.err_type == ErrorType::ErrorTypeNone);
        funcs_st().has_wsl = if luaret_truthy(o) { K_TRUE } else { K_FALSE };
    }
    funcs_st().has_wsl == K_TRUE
}

/// "highlightID(name)" function
pub fn f_hl_id(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(syn_name2id(
        tv_get_string(&argvars[0]),
    )));
}

/// "highlight_exists()" function
pub fn f_hlexists(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(highlight_exists(
        tv_get_string(&argvars[0]),
    )));
}

/// "index()" function
pub fn f_index(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut idx: i32 = 0;
        let mut ic = false;

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        if argvars[0].v_type == VarType::VarBlob {
            let mut error = false;
            let mut start: i32 = 0;
            if argvars[2].v_type != VarType::VarUnknown {
                start = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
                if error {
                    return;
                }
            }
            let b = argvars[0].vval.v_blob().cast::<TvBlob>();
            if b.is_null() {
                return;
            }
            if start < 0 {
                start += tv_blob_len(b.as_ref());
                if start < 0 {
                    start = 0;
                }
            }
            idx = start;
            while idx < tv_blob_len(b.as_ref()) {
                let tv = TypVal {
                    v_type: VarType::VarNumber,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(Varnumber::from(tv_blob_get(&*b, idx))),
                };
                if tv_equal(&tv, &argvars[1], ic) {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(idx));
                    return;
                }
                idx += 1;
            }
            return;
        } else if argvars[0].v_type != VarType::VarList {
            emsg(gettext(E_LISTBLOBREQ));
            return;
        }

        let l = argvars[0].vval.v_list();
        if l.is_null() {
            return;
        }

        let mut item = tv_list_first(l.as_ref());
        if argvars[2].v_type != VarType::VarUnknown {
            let mut error = false;
            idx = tv_list_uidx(
                l.as_ref(),
                tv_get_number_chk(&argvars[2], Some(&mut error)) as i32,
            );
            if error || idx == -1 {
                item = ptr::null_mut();
            } else {
                item = tv_list_find(l.as_mut(), idx);
                debug_assert!(!item.is_null());
            }
            if argvars[3].v_type != VarType::VarUnknown {
                ic = tv_get_bool_chk(&argvars[3], Some(&mut error)) != 0;
                if error {
                    item = ptr::null_mut();
                }
            }
        }

        while !item.is_null() {
            if tv_equal(&(*item).li_tv, &argvars[1], ic) {
                rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(idx));
                break;
            }
            item = (*item).li_next;
            idx += 1;
        }
    }
}

fn indexof_eval_expr(expr: &mut TypVal) -> Varnumber {
    {
        let mut argv = [tv_initial_value(); 3];
        argv[0] = *get_vim_var_tv(VV_KEY);
        argv[1] = *get_vim_var_tv(VV_VAL);
        let mut newtv = tv_initial_value();

        if !eval_expr_typval(&*expr, false, argv.as_mut_ptr(), 2, &mut newtv) {
            return 0;
        }

        let mut error = false;
        let found = tv_get_bool_chk(&newtv, Some(&mut error));
        tv_clear(&mut newtv);

        if error { 0 } else { found }
    }
}

fn indexof_blob(b: Option<&TvBlob>, mut startidx: Varnumber, expr: &mut TypVal) -> Varnumber {
    if b.is_none() {
        return -1;
    }
    if startidx < 0 {
        startidx += Varnumber::from(tv_blob_len(b));
        if startidx < 0 {
            startidx = 0;
        }
    }

    set_vim_var_type(VV_KEY, VarType::VarNumber);
    set_vim_var_type(VV_VAL, VarType::VarNumber);

    let called_emsg_start = g_st().called_emsg;
    let mut idx = startidx;
    while idx < Varnumber::from(tv_blob_len(b)) {
        set_vim_var_nr(VV_KEY, idx);
        set_vim_var_nr(VV_VAL, Varnumber::from(tv_blob_get(b.unwrap(), idx as i32)));

        if indexof_eval_expr(expr) != 0 {
            return idx;
        }
        if g_st().called_emsg != called_emsg_start {
            return -1;
        }
        idx += 1;
    }
    -1
}

fn indexof_list(l: Option<&mut TvList>, startidx: Varnumber, expr: &mut TypVal) -> Varnumber {
    {
        if l.is_none() {
            return -1;
        }
        let mut item: *mut Listitem;
        let mut idx: Varnumber = 0;
        if startidx == 0 {
            item = tv_list_first(l.as_deref());
        } else {
            idx = Varnumber::from(tv_list_uidx(l.as_deref(), startidx as i32));
            if idx == -1 {
                item = ptr::null_mut();
            } else {
                item = tv_list_find(l, idx as i32);
                debug_assert!(!item.is_null());
            }
        }

        set_vim_var_type(VV_KEY, VarType::VarNumber);

        let called_emsg_start = g_st().called_emsg;
        while !item.is_null() {
            set_vim_var_nr(VV_KEY, idx);
            tv_copy(unsafe { &(*item).li_tv }, &mut *get_vim_var_tv(VV_VAL));

            let found = indexof_eval_expr(expr) != 0;
            tv_clear(get_vim_var_tv(VV_VAL));

            if found {
                return idx;
            }
            if g_st().called_emsg != called_emsg_start {
                return -1;
            }
            item = unsafe { (*item).li_next };
            idx += 1;
        }
        -1
    }
}

/// "indexof()" function
pub fn f_indexof(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if !tv_check_for_list_or_blob_arg(argvars, 0)
            || !tv_check_for_string_or_func_arg(argvars, 1)
            || !tv_check_for_opt_dict_arg(argvars, 2)
        {
            return;
        }

        if (argvars[1].v_type == VarType::VarString
            && (argvars[1].vval.v_string().is_null()
                || crate::memory::cstr_is_empty(argvars[1].vval.v_string())))
            || (argvars[1].v_type == VarType::VarFunc && argvars[1].vval.v_string().is_null())
        {
            return;
        }

        let mut startidx: Varnumber = 0;
        if argvars[2].v_type == VarType::VarDict {
            startidx = tv_dict_get_number_def(
                unsafe { argvars[2].vval.v_dict().as_ref() },
                c"startidx",
                0,
            );
        }

        let mut save_val = tv_initial_value();
        let mut save_key = tv_initial_value();
        prepare_vimvar(VV_VAL, &mut save_val);
        prepare_vimvar(VV_KEY, &mut save_key);

        let save_did_emsg = g_st().did_emsg;
        g_st().did_emsg = 0;

        if argvars[0].v_type == VarType::VarBlob {
            rettv.vval = crate::eval::typval::TypvalVval::Number(indexof_blob(
                unsafe { argvars[0].vval.v_blob().as_ref() },
                startidx,
                &mut argvars[1],
            ));
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(indexof_list(
                unsafe { argvars[0].vval.v_list().as_mut() },
                startidx,
                &mut argvars[1],
            ));
        }

        restore_vimvar(VV_KEY, &mut save_key);
        restore_vimvar(VV_VAL, &mut save_val);
        g_st().did_emsg |= save_did_emsg;
    }
}

/// "input()" function
pub fn f_input(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_user_input(argvars, rettv, false, funcs_st().inputsecret_flag);
}

/// "inputdialog()" function
pub fn f_inputdialog(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_user_input(argvars, rettv, true, funcs_st().inputsecret_flag);
}

/// "inputlist()" function
pub fn f_inputlist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarList {
            semsg!(gettext(E_LISTARG), c"inputlist()");
            return;
        }

        msg_ext_set_kind(c"confirm");
        msg_start();
        g_st().msg_row = g_st().rows_global - 1;
        g_st().lines_left = g_st().rows_global;
        g_st().msg_scroll = 1;
        msg_clr_eos();

        let l = argvars[0].vval.v_list();
        for li in crate::eval::typval::list_iter_mut(tv_list_first(unsafe { l.as_ref() })) {
            msg_puts(tv_get_string(&li.li_tv));
            if !ui_has(K_UIMESSAGES) || !li.li_next.is_null() {
                msg_putchar(b'\n' as i32);
            }
        }

        let mut mouse_used = false;
        let mut selected = prompt_for_input(ptr::null_mut(), 0, false, Some(&mut mouse_used));
        if mouse_used {
            selected = tv_list_len(unsafe { l.as_ref() }) - (g_st().cmdline_row - g_st().mouse_row);
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(selected));
    }
}

/// "inputrestore()" function
pub fn f_inputrestore(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if let Some(mut saved) = funcs_st().ga_userinput.pop() {
            restore_typeahead(&mut saved);
        } else if opt_vars().p_verbose > 1 {
            verb_msg(gettext(
                c"called inputrestore() more often than inputsave()",
            ));
            rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        }
    }
}

/// "inputsave()" function
pub fn f_inputsave(_argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut saved = Tasave::new();
        save_typeahead(&mut saved);
        funcs_st().ga_userinput.push(saved);
    }
}

/// "inputsecret()" function
pub fn f_inputsecret(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
    g_st().cmdline_star += 1;
    funcs_st().inputsecret_flag = true;
    f_input(argvars, rettv, fptr);
    g_st().cmdline_star -= 1;
    funcs_st().inputsecret_flag = false;
}

/// "interrupt()" function
pub fn f_interrupt(_argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    g_st().got_int = true;
}

/// "invert(expr)" function
pub fn f_invert(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(!tv_get_number_chk(&argvars[0], None));
}

/// "islocked()" function
pub fn f_islocked(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut lv: Lval = Lval::new();
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        let end = get_lval(
            tv_get_string(&argvars[0]).cast_mut(),
            None,
            &mut lv,
            false,
            false,
            GLV_NO_AUTOLOAD | GLV_READ_ONLY,
            FNE_CHECK_START,
        );
        if !end.is_null() && !lv.ll_name.is_null() {
            if !crate::memory::cstr_is_empty(end) {
                semsg!(
                    gettext(if lv.ll_name_len == 0 {
                        E_INVARG2.as_ptr()
                    } else {
                        E_TRAILING_ARG.as_ptr()
                    }),
                    end,
                );
            } else if lv.ll_tv.is_null() {
                let di = find_var(lv.ll_name, lv.ll_name_len, ptr::null_mut(), 1);
                if !di.is_null() {
                    let di = { unsafe { &mut *di } };
                    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
                        (di.di_flags & DI_FLAGS_LOCK) != 0 || tv_islocked(&di.di_tv),
                    ));
                }
            } else if lv.ll_range {
                emsg(gettext(c"E786: Range not allowed"));
            } else if let Some(ref newkey) = lv.ll_newkey {
                semsg!(gettext(E_DICTKEY), newkey.as_ptr());
            } else if !lv.ll_list.is_null() {
                rettv.vval =
                    crate::eval::typval::TypvalVval::Number(Varnumber::from(tv_islocked(unsafe {
                        &(*lv.ll_li).li_tv
                    })));
            } else {
                rettv.vval =
                    crate::eval::typval::TypvalVval::Number(Varnumber::from(tv_islocked(unsafe {
                        &(*lv.ll_di).di_tv
                    })));
            }
        }
    }
}

/// "isinf()" function
pub fn f_isinf(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type == VarType::VarFloat && xisinf(argvars[0].vval.v_float()) != 0 {
            rettv.vval =
                crate::eval::typval::TypvalVval::Number(if argvars[0].vval.v_float() > 0.0 {
                    1
                } else {
                    -1
                });
        }
    }
}

/// "isnan()" function
pub fn f_isnan(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
            argvars[0].v_type == VarType::VarFloat && xisnan(argvars[0].vval.v_float()) != 0,
        ));
    }
}

/// "id()" function
pub fn f_id(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let len = vim_snprintf_typval_only(ptr::null_mut(), 0, c"%p", (&raw mut argvars[0]));
    let mut buf = vec![0 as c_char; (len as usize) + 1];
    vim_snprintf_typval_only(
        buf.as_mut_ptr(),
        (len as usize) + 1,
        c"%p",
        (&raw mut argvars[0]),
    );
    crate::eval::typval::tv_set_string_len(rettv, buf.as_ptr(), len as usize);
}

/// "jobpid(id)" function
pub fn f_jobpid(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarNumber {
            emsg(gettext(E_INVARG));
            return;
        }

        let data = find_job(argvars[0].vval.v_number() as u64, true);
        if data.is_null() {
            return;
        }
        let data = unsafe { &mut *data };
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(data.uv.proc_.pid));
    }
}

/// "jobresize(job, width, height)" function
pub fn f_jobresize(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarNumber
            || argvars[1].v_type != VarType::VarNumber
            || argvars[2].v_type != VarType::VarNumber
        {
            emsg(gettext(E_INVARG));
            return;
        }

        let data = find_job(argvars[0].vval.v_number() as u64, true);
        if data.is_null() {
            return;
        }

        let proc_ptr = &raw mut (*data).uv.proc_;
        if (*proc_ptr).r#type != K_PROC_TYPE_PTY {
            emsg(gettext(E_CHANNOTPTY));
            return;
        }

        pty_proc_resize(
            &mut *proc_ptr.cast::<PtyProc>(),
            argvars[1].vval.v_number() as u16,
            argvars[2].vval.v_number() as u16,
        );
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    }
}

/// Builds an environment dict for a child process (job).
pub fn create_environment(
    job_env: Option<&Dictitem>,
    clear_env: bool,
    pty: bool,
    set_nvim_addr: bool,
    pty_term_name: impl crate::strings::IntoCCharPtr,
) -> &'static mut TvDict {
    let pty_term_name = pty_term_name.into_c_char_ptr();
    unsafe {
        let env = tv_dict_alloc();

        if !clear_env {
            let mut envitems: *mut uv_env_item_t = ptr::null_mut();
            let mut envcount: i32 = 0;
            if uv_os_environ(&raw mut envitems, &mut envcount) == 0 {
                for i in 0..(envcount as usize) {
                    #[cfg(windows)]
                    let mut name: *const c_char = (*envitems.wrapping_add(i)).name;
                    #[cfg(not(windows))]
                    let name: *const c_char = (*envitems.add(i)).name;
                    #[cfg(windows)]
                    let mut namelen = crate::memory::cstr_len(name);
                    #[cfg(not(windows))]
                    let namelen = crate::memory::cstr_len(name);
                    #[cfg(windows)]
                    {
                        if namelen < MAXPATHL / MB_MAXBYTES {
                            let namebuff = g_st().name_buff.as_mut_ptr();
                            namelen = mb_strup_buf(name, namebuff);
                            name = namebuff;
                        }
                    }
                    let _ = namelen;
                    tv_dict_add_str(&mut *env, name, (*envitems.wrapping_add(i)).value);
                }
                uv_os_free_environ(envitems, envcount);
            }

            if pty {
                for &ignored in PTY_IGNORED_ENV_VARS {
                    let dv = tv_dict_find(Some(&*env), ignored.as_ptr(), -1);
                    if !dv.is_null() {
                        tv_dict_item_remove(&mut *env, &mut *dv);
                    }
                }
                if opt_vars().p_tgc != 0 {
                    tv_dict_add_str(&mut *env, c"COLORTERM", c"truecolor");
                }
            }
        }

        if pty {
            let dv = tv_dict_find(Some(&*env), c"TERM", 4);
            if !dv.is_null() {
                tv_dict_item_remove(&mut *env, &mut *dv);
            }
            tv_dict_add_str(&mut *env, c"TERM", pty_term_name);
        }

        if set_nvim_addr {
            let nvim_addr = get_vim_var_str(VV_SEND_SERVER);
            if !crate::memory::cstr_is_empty(nvim_addr) {
                let dv = tv_dict_find(Some(&*env), c"NVIM", 4);
                if !dv.is_null() {
                    tv_dict_item_remove(&mut *env, &mut *dv);
                }
                tv_dict_add_str(&mut *env, c"NVIM", nvim_addr);
            }
        }

        if let Some(job_env) = job_env {
            #[cfg(windows)]
            {
                let d = job_env.di_tv.vval.v_dict();
                if !d.is_null() {
                    let ht = &(*d).dv_hashtab;
                    let mut todo = ht.ht_used;
                    let mut hi = (ht).array_ptr();
                    while todo > 0 {
                        if !crate::hashtab::hashitem_empty(&*hi) {
                            todo -= 1;
                            let var = tv_dict_hi2di(hi);
                            let mut name = var.di_key.as_ptr();
                            let mut namelen = crate::memory::cstr_len(name);
                            if namelen < MAXPATHL / MB_MAXBYTES {
                                let namebuff = g_st().name_buff.as_mut_ptr();
                                namelen = mb_strup_buf(name, namebuff);
                                name = namebuff;
                            }
                            let dv = tv_dict_find(Some(&*env), name, namelen as isize);
                            if !dv.is_null() {
                                tv_dict_item_remove(&mut *env, &mut *dv);
                            }
                            tv_dict_add_str(&mut *env, name, tv_get_string(&var.di_tv));
                        }
                        hi = hi.wrapping_add(1);
                    }
                }
            }
            #[cfg(not(windows))]
            {
                tv_dict_extend(&mut *env, &mut *job_env.di_tv.vval.v_dict(), c"force");
            }
        }

        if pty {
            for &req in REQUIRED_ENV_VARS {
                let len = req.to_bytes().len();
                let dv = tv_dict_find(Some(&*env), req.as_ptr(), len as isize);
                if dv.is_null()
                    && let Some(env_var) = os_getenv_cstr(req.as_ptr())
                {
                    tv_dict_add_str(&mut *env, req, env_var.as_ptr());
                }
            }
        }

        env
    }
}

/// "jobstart()" function
pub fn f_jobstart(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }

        let mut cmd = std::ffi::CString::default();
        let mut executable = true;
        let argv = tv_to_argv(&mut argvars[0], Some(&mut cmd), Some(&mut executable));
        if argv.is_empty() {
            rettv.vval = crate::eval::typval::TypvalVval::Number(if executable { 0 } else { -1 });
            return;
        }

        if argvars[1].v_type != VarType::VarDict && argvars[1].v_type != VarType::VarUnknown {
            semsg!(gettext(E_INVARG2), c"expected dictionary");
            return;
        }

        let mut job_opts: *mut TvDict = ptr::null_mut();
        let mut detach = false;
        let mut rpc = false;
        let mut pty = false;
        let mut term = false;
        let mut clear_env = false;
        let mut overlapped = false;
        let mut stdin_mode = K_CHANNEL_STDIN_PIPE_CONST;
        let mut on_stdout = CallbackReader::none();
        let mut on_stderr = CallbackReader::none();
        let mut on_exit = Callback::NONE;
        let mut cwd: *const c_char = ptr::null();
        let mut job_env: *mut Dictitem = ptr::null_mut();

        if argvars[1].v_type == VarType::VarDict {
            job_opts = argvars[1].vval.v_dict();

            detach = tv_dict_get_number(job_opts.as_ref(), c"detach") != 0;
            rpc = tv_dict_get_number(job_opts.as_ref(), c"rpc") != 0;
            term = tv_dict_get_number(job_opts.as_ref(), c"term") != 0;
            pty = term || tv_dict_get_number(job_opts.as_ref(), c"pty") != 0;
            clear_env = tv_dict_get_number(job_opts.as_ref(), c"clear_env") != 0;
            overlapped = tv_dict_get_number(job_opts.as_ref(), c"overlapped") != 0;

            let s = tv_dict_get_string(job_opts.as_ref(), c"stdin");
            if !s.is_null() {
                if crate::memory::cstr_prefix_bytes(s, NUMBUFLEN)
                    == crate::memory::cstr_prefix_bytes(c"null", NUMBUFLEN)
                {
                    stdin_mode = K_CHANNEL_STDIN_NULL;
                } else if crate::memory::cstr_prefix_bytes(s, NUMBUFLEN)
                    == crate::memory::cstr_prefix_bytes(c"pipe", NUMBUFLEN)
                {
                    // default
                } else {
                    semsg!(gettext(E_INVARG_NVAL), c"stdin", s);
                }
            }

            let job_term = tv_dict_find(job_opts.as_ref(), c"term", 4);
            if !job_term.is_null() && (*job_term).di_tv.v_type != VarType::VarBool {
                semsg!(gettext(E_INVARG2), c"'term' must be Boolean",);
                return;
            }

            if pty && rpc {
                semsg!(
                    gettext(E_INVARG2),
                    c"job cannot have both 'pty' and 'rpc' options set",
                );
                return;
            }

            #[cfg(windows)]
            if pty && overlapped {
                semsg!(
                    gettext(E_INVARG2),
                    c"job cannot have both 'pty' and 'overlapped' options set",
                );
                return;
            }

            let new_cwd = tv_dict_get_string(job_opts.as_ref(), c"cwd");
            if !crate::memory::cstr_is_empty(new_cwd) {
                cwd = new_cwd;
                if !os_isdir(cwd) {
                    semsg!(gettext(E_INVARG2), c"expected valid directory",);
                    return;
                }
            }

            job_env = tv_dict_find(job_opts.as_ref(), c"env", 3);
            if !job_env.is_null() && (*job_env).di_tv.v_type != VarType::VarDict {
                semsg!(gettext(E_INVARG2), c"env");
                return;
            }

            if !common_job_callbacks(&mut *job_opts, &mut on_stdout, &mut on_stderr, &mut on_exit) {
                return;
            }
        }

        let mut width = tv_dict_get_number(job_opts.as_ref(), c"width") as u16;
        let mut height = tv_dict_get_number(job_opts.as_ref(), c"height") as u16;
        let mut term_name: *const c_char = ptr::null();

        if term {
            if text_locked() {
                text_locked_msg();
                return;
            }
            if curbuf_mut().b_changed != 0 {
                emsg(gettext(
                    c"jobstart(...,{term=true}) requires unmodified buffer",
                ));
                return;
            }
            if !curbuf_mut().terminal.is_null() {
                if terminal_running(&*curbuf_mut().terminal) {
                    semsg!(
                        gettext(c"Terminal already connected to buffer %d"),
                        curbuf_mut().b_fnum,
                    );
                    return;
                }
                buf_close_terminal(curbuf_mut());
            }
            debug_assert!(!rpc);
            term_name = c"xterm-256color".as_ptr();
            if cwd.is_null() {
                cwd = c".".as_ptr();
            }
            overlapped = false;
            detach = false;
            stdin_mode = K_CHANNEL_STDIN_PIPE_CONST;
            if width == 0 {
                width = (curwin_mut().w_view_width - win_col_off(curwin_ptr())).max(0) as u16;
            }
            if height == 0 {
                height = curwin_mut().w_view_height as u16;
            }
        }

        if pty {
            if term_name.is_null() {
                term_name = tv_dict_get_string(job_opts.as_ref(), c"TERM");
            }
            if term_name.is_null() {
                term_name = c"ansi".as_ptr();
            }
        }

        let env = create_environment(job_env.as_ref(), clear_env, pty, true, term_name);
        let chan = channel_job_start(
            argv,
            ptr::null(),
            on_stdout,
            on_stderr,
            on_exit,
            pty,
            rpc,
            overlapped,
            detach,
            stdin_mode,
            cwd,
            width,
            height,
            env,
            rettv.vval.v_number_mut(),
        );
        if chan.is_null() {
            // do nothing
        } else if !term {
            channel_create_event(&mut *chan, ptr::null());
        } else {
            if rettv.vval.v_number() <= 0 {
                return;
            }

            let pid = (*chan).uv.proc_.pid;
            let buf = g_st().curbuf;
            let buf = { &mut *buf };

            buf.b_p_swf = 0;
            if buf.b_ml.ml_mfp.is_none() && !ml_open(&mut *buf) {
                proc_stop(&mut (*chan).uv.proc_);
                channel_decref(&mut *chan);
                return;
            }

            channel_incref(&mut *chan);
            channel_terminal_alloc(&mut *buf, &mut *chan);

            let ct = chan;
            'term_done: {
                apply_autocmds(
                    EVENT_BUFFILEPRE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    buf,
                );

                if (*ct).term.is_null() || terminal_buf(&*(*ct).term) == 0 {
                    break 'term_done;
                }

                let namebuff = g_st().name_buff.as_mut_ptr();
                let iobuff = g_st().iobuff.as_mut_ptr();
                vim_full_name(cwd, namebuff, MAXPATHL, false);
                let len = home_replace(None, namebuff, iobuff, IOSIZE, true);
                if len != 1
                    && (crate::memory::cstr_byte_at(iobuff, len - 1) == b'\\'
                        || crate::memory::cstr_byte_at(iobuff, len - 1) == b'/')
                {
                    *iobuff.wrapping_add(len - 1) = NUL;
                }
                if len == 1 && crate::memory::cstr_first_byte(iobuff) == b'/' {
                    *iobuff.wrapping_add(1) = b'.' as c_char;
                    *iobuff.wrapping_add(2) = NUL;
                }

                vim_snprintf!(
                    namebuff,
                    MAXPATHL,
                    c"term://%s//%d:%s",
                    iobuff,
                    pid,
                    cmd.as_ptr(),
                );

                setfname(&mut *buf, namebuff, ptr::null_mut(), true);
                apply_autocmds(
                    EVENT_BUFFILEPOST,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    buf,
                );

                if (*ct).term.is_null() || terminal_buf(&*(*ct).term) == 0 {
                    break 'term_done;
                }

                let mut err = Error::new();
                buf.b_locked += 1;
                let cp = chan;
                dict_set_var(
                    &mut *(*buf).b_vars(),
                    cstr_as_string(c"terminal_job_id"),
                    Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer((*cp).id as Integer),
                    },
                    false,
                    false,
                    None,
                    &mut err,
                );
                api_clear_error(&mut err);
                dict_set_var(
                    &mut *(*buf).b_vars(),
                    cstr_as_string(c"terminal_job_pid"),
                    Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(Integer::from(pid)),
                    },
                    false,
                    false,
                    None,
                    &mut err,
                );
                api_clear_error(&mut err);
                buf.b_locked -= 1;

                if (*ct).term.is_null() || terminal_buf(&*(*ct).term) == 0 {
                    break 'term_done;
                }

                terminal_open(&raw mut (*ct).term, &mut *buf);
            }
            channel_create_event(&mut *chan, ptr::null());
            channel_decref(&mut *chan);
        }
    }
}

/// "jobstop()" function
pub fn f_jobstop(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarNumber {
            emsg(gettext(E_INVARG));
            return;
        }

        let data = find_job(argvars[0].vval.v_number() as u64, false);
        if data.is_null() {
            return;
        }
        let data = unsafe { &mut *data };

        let mut error: *const c_char = ptr::null();
        if data.is_rpc {
            channel_close(data.id, K_CHANNEL_PART_RPC_CONST, &mut error);
        }
        proc_stop(&mut data.uv.proc_);
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        if !error.is_null() {
            emsg(error);
        }
    }
}

/// "jobwait(ids[, timeout])" function
pub fn f_jobwait(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarList
            || (argvars[1].v_type != VarType::VarNumber && argvars[1].v_type != VarType::VarUnknown)
        {
            emsg(gettext(E_INVARG));
            return;
        }

        let args = argvars[0].vval.v_list();
        let len = tv_list_len(args.as_ref()) as usize;
        let mut jobs: Vec<*mut Channel> = vec![ptr::null_mut(); len];
        let main_loop_ptr: *mut Loop = &raw mut g_st().main_loop;
        let waiting_jobs = multiqueue_new(main_loop_ptr);

        for (i, arg) in crate::eval::typval::list_iter_ptr(tv_list_first(args.as_ref())).enumerate()
        {
            let tv = &arg.li_tv;
            let chan = if tv.v_type == VarType::VarNumber {
                find_channel(tv.vval.v_number() as u64)
            } else {
                ptr::null_mut()
            };
            if chan.is_null() || (*chan).streamtype != K_CHANNEL_STREAM_PROC {
                jobs[i] = ptr::null_mut();
            } else {
                let proc_ptr = &mut (*chan).uv.proc_;
                if proc_is_stopped(proc_ptr) {
                    proc_wait(proc_ptr, -1, ptr::null_mut());
                    jobs[i] = ptr::null_mut();
                } else {
                    jobs[i] = chan;
                    channel_incref(&mut *chan);
                    if proc_ptr.status < 0 {
                        let events = (*chan).events;
                        multiqueue_process_events(&mut *events);
                        multiqueue_replace_parent(&mut *events, &mut *waiting_jobs);
                    }
                }
            }
        }

        let mut remaining: i32 = -1;
        let mut before: u64 = 0;
        if argvars[1].v_type == VarType::VarNumber && argvars[1].vval.v_number() >= 0 {
            remaining = argvars[1].vval.v_number() as i32;
            before = os_hrtime();
        }

        let busy = remaining != 0;
        if busy {
            ui_busy_start();
            ui_flush();
        }

        for &job in &jobs {
            if remaining == 0 {
                break;
            }
            if job.is_null() {
                continue;
            }
            let status = proc_wait(&mut (*job).uv.proc_, remaining, waiting_jobs);
            if status < 0 {
                break;
            }
            if remaining > 0 {
                let now = os_hrtime();
                remaining = 0.min(remaining - ((now - before) / 1_000_000) as i32);
                before = now;
            }
        }

        let rv = tv_list_alloc(len as isize);
        for &job in &jobs {
            if job.is_null() {
                tv_list_append_number(&mut *rv, -3);
                continue;
            }
            let events = (*job).events;
            multiqueue_process_events(&mut *events);
            multiqueue_replace_parent(&mut *events, &mut *loop_queues(&mut *main_loop_ptr).events);

            let status = (*job).uv.proc_.status;
            tv_list_append_number(&mut *rv, Varnumber::from(status));
            channel_decref(&mut *job);
        }

        multiqueue_free(waiting_jobs);
        if busy {
            ui_busy_stop();
        }
        tv_list_ref(Some(&mut *rv));
        rettv.v_type = VarType::VarList;
        rettv.vval = crate::eval::typval::TypvalVval::List(rv);
    }
}

/// "json_decode()" function
pub fn f_json_decode(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let s: *const c_char;
        let mut owned_buf: Vec<c_char> = Vec::new();
        let len: usize;
        if argvars[0].v_type == VarType::VarList {
            let Some(buf) = encode_vim_list_to_buf(unsafe { argvars[0].vval.v_list().as_ref() })
            else {
                emsg(gettext(c"E474: Failed to convert list to string"));
                return;
            };
            len = buf.len();
            owned_buf = buf;
            if owned_buf.is_empty() {
                s = c"".as_ptr();
            } else {
                s = owned_buf.as_ptr();
            }
        } else {
            s = tv_get_string_buf_chk(&argvars[0], numbuf.as_mut_ptr());
            if !s.is_null() {
                len = crate::memory::cstr_len(s);
            } else {
                return;
            }
        }
        if !json_decode_string(s, len, rettv) {
            semsg!(gettext(c"E474: Failed to parse %.*s"), len as i32, s,);
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
        debug_assert!(rettv.v_type != VarType::VarUnknown);
        drop(owned_buf);
    }
}

/// "json_encode()" function
pub fn f_json_encode(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, encode_tv2json(&mut argvars[0], None).as_ptr());
}

/// "keytrans()" function
pub fn f_keytrans(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarString;
        if !tv_check_for_string_arg(argvars, 0) || argvars[0].vval.v_string().is_null() {
            return;
        }
        let escaped = vim_strsave_escape_ks(argvars[0].vval.v_string());
        crate::eval::typval::tv_set_string(
            rettv,
            str2special_save(escaped.as_ptr(), true, 1).as_ptr(),
        );
    }
}

/// "len()" function
pub fn f_len(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        match argvars[0].v_type {
            VarType::VarString | VarType::VarNumber => {
                rettv.vval = crate::eval::typval::TypvalVval::Number(crate::memory::cstr_len(
                    tv_get_string(&argvars[0]),
                )
                    as Varnumber);
            }
            VarType::VarBlob => {
                rettv.vval =
                    crate::eval::typval::TypvalVval::Number(Varnumber::from(tv_blob_len(unsafe {
                        argvars[0].vval.v_blob().as_ref()
                    })));
            }
            VarType::VarList => {
                rettv.vval =
                    crate::eval::typval::TypvalVval::Number(Varnumber::from(tv_list_len(unsafe {
                        argvars[0].vval.v_list().as_ref()
                    })));
            }
            VarType::VarDict => {
                rettv.vval = crate::eval::typval::TypvalVval::Number(tv_dict_len(unsafe {
                    argvars[0].vval.v_dict().as_ref()
                })
                    as Varnumber);
            }
            VarType::VarUnknown
            | VarType::VarBool
            | VarType::VarSpecial
            | VarType::VarFloat
            | VarType::VarPartial
            | VarType::VarFunc => {
                emsg(gettext(c"E701: Invalid type for len()"));
            }
        }
    }
}

fn libcall_common(argvars: &mut [TypVal], rettv: &mut TypVal, out_type: VarType) {
    {
        rettv.v_type = out_type;
        if out_type != VarType::VarNumber {
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarString || argvars[1].v_type != VarType::VarString {
            return;
        }

        let libname = argvars[0].vval.v_string();
        let funcname = argvars[1].vval.v_string();
        let in_type = argvars[2].v_type;

        let str_in = if in_type == VarType::VarString {
            argvars[2].vval.v_string()
        } else {
            ptr::null()
        };
        let int_in = argvars[2].vval.v_number() as i32;

        let mut str_out: Option<std::ffi::CString> = None;
        let mut int_out: i32 = 0;

        let success = os_libcall(
            libname,
            funcname,
            str_in,
            int_in,
            (out_type == VarType::VarString).then_some(&mut str_out),
            Some(&mut int_out),
        );
        if !success {
            semsg!(gettext(E_LIBCALL), funcname);
            return;
        }
        if out_type == VarType::VarString {
            if let Some(s) = str_out {
                crate::eval::typval::tv_set_string(&mut *rettv, s.as_ptr());
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
        } else if out_type == VarType::VarNumber {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(int_out));
        }
    }
}

/// "libcall()" function
pub fn f_libcall(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    libcall_common(argvars, rettv, VarType::VarString);
}

/// "libcallnr()" function
pub fn f_libcallnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    libcall_common(argvars, rettv, VarType::VarNumber);
}

/// "line(string, [winid])" function
pub fn f_line(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut lnum: Linenr = 0;
        let mut fp: *mut CursorPos = ptr::null_mut();
        let mut fnum: i32 = 0;

        if argvars[1].v_type != VarType::VarUnknown {
            let id = tv_get_number(&argvars[1]) as i32;
            let mut tp: *mut TabPageInfo = ptr::null_mut();
            let wp = win_id2wp_tp(id, &raw mut tp);
            if !wp.is_null() && !tp.is_null() {
                if crate::memory::cstr_first_byte(opt_vars().p_spk) != b'c'
                    || (unsafe { (*wp).w_onebuf_opt.wo_diff } != 0
                        && curwin_mut().w_onebuf_opt.wo_diff != 0)
                {
                    g_st().skip_update_topline = true;
                }
                check_cursor(unsafe { &mut *wp });
                fp = var2fpos(&argvars[0], true, &mut fnum, false, unsafe { &mut *wp });
                g_st().skip_update_topline = false;
            }
        } else {
            fp = var2fpos(&argvars[0], true, &mut fnum, false, curwin_mut());
        }

        if !fp.is_null() {
            lnum = unsafe { (*fp).lnum };
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(lnum));
    }
}

/// "line2byte(lnum)" function
pub fn f_line2byte(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let lnum = tv_get_lnum(&argvars[0]);
        if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count + 1 {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(ml_find_line_or_offset(
                curbuf_mut(),
                lnum,
                None,
                false,
            ) as Varnumber);
        }
        if rettv.vval.v_number() >= 0 {
            *rettv.vval.v_number_mut() += 1;
        }
    }
}

/// "localtime()" function
pub fn f_localtime(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number((os_time() as time_t) as Varnumber);
}

/// "luaeval()" function
pub fn f_luaeval(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let str_ = tv_get_string_chk(&argvars[0]);
    if str_.is_null() {
        return;
    }
    nlua_call_luaeval(cstr_as_string(str_), &mut argvars[1], rettv);
}

fn find_some_match(argvars: &mut [TypVal], rettv: &mut TypVal, type_: SomeMatchType) {
    unsafe {
        let mut str_: *mut c_char = ptr::null_mut();
        let mut len: i64 = 0;
        let mut expr: *mut c_char = ptr::null_mut();
        let mut regmatch: Regmatch = Regmatch::new();
        let mut start: i64;
        let mut nth: i64 = 1;
        let mut startcol: Colnr = 0;
        let mut match_found = false;
        let mut l: *mut TvList = ptr::null_mut();
        let mut idx: i32 = 0;
        let mut tofree_owned: Option<std::ffi::CString> = None;

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        match type_ {
            SomeMatchType::SomeMatchList => {
                tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_MAY_KNOW_CONST);
            }
            SomeMatchType::SomeMatchStrPos => {
                tv_list_alloc_ret(&mut *rettv, 4);
                tv_list_append_string(rettv.vval.v_list_ref(), c"", 0);
                tv_list_append_number(rettv.vval.v_list_ref(), -1);
                tv_list_append_number(rettv.vval.v_list_ref(), -1);
                tv_list_append_number(rettv.vval.v_list_ref(), -1);
            }
            SomeMatchType::SomeMatchStr => {
                rettv.v_type = VarType::VarString;
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
            SomeMatchType::SomeMatch | SomeMatchType::SomeMatchEnd => {}
        }

        let mut li: *mut Listitem = ptr::null_mut();
        'theend: {
            if argvars[0].v_type == VarType::VarList {
                l = argvars[0].vval.v_list();
                if l.is_null() {
                    break 'theend;
                }
                li = tv_list_first(l.as_ref());
            } else {
                str_ = tv_get_string(&argvars[0]).cast_mut();
                expr = str_;
                len = crate::memory::cstr_len(str_) as i64;
            }

            let mut patbuf = [0 as c_char; NUMBUFLEN];
            let pat = tv_get_string_buf_chk(&argvars[1], patbuf.as_mut_ptr());
            if pat.is_null() {
                break 'theend;
            }

            if argvars[2].v_type != VarType::VarUnknown {
                let mut error = false;
                start = tv_get_number_chk(&argvars[2], Some(&mut error));
                if error {
                    break 'theend;
                }
                if !l.is_null() {
                    idx = tv_list_uidx(l.as_ref(), start as i32);
                    if idx == -1 {
                        break 'theend;
                    }
                    li = tv_list_find(l.as_mut(), idx);
                } else {
                    if start < 0 {
                        start = 0;
                    }
                    if start > len {
                        break 'theend;
                    }
                    if argvars[3].v_type != VarType::VarUnknown {
                        startcol = start as Colnr;
                    } else {
                        str_ = str_.wrapping_add(start as usize);
                        len -= start;
                    }
                }

                if argvars[3].v_type != VarType::VarUnknown {
                    nth = tv_get_number_chk(&argvars[3], Some(&mut error));
                }
                if error {
                    break 'theend;
                }
            }

            regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
            if !regmatch.regprog.is_null() {
                regmatch.rm_ic = opt_vars().p_ic != 0;

                loop {
                    if !l.is_null() {
                        if li.is_null() {
                            match_found = false;
                            break;
                        }
                        tofree_owned = Some(encode_tv2echo(&mut (*li).li_tv, None));
                        expr = tofree_owned
                            .as_ref()
                            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                        str_ = expr;
                        if str_.is_null() {
                            break;
                        }
                    }

                    match_found = vim_regexec_nl(&mut regmatch, str_, startcol);

                    if match_found {
                        nth -= 1;
                        if nth <= 0 {
                            break;
                        }
                    }
                    if l.is_null() && !match_found {
                        break;
                    }

                    if !l.is_null() {
                        li = (*li).li_next;
                        idx += 1;
                    } else {
                        startcol = (regmatch.startp[0]
                            .wrapping_add(utfc_ptr2len(regmatch.startp[0]) as usize)
                            .offset_from(str_)) as Colnr;
                        if i64::from(startcol) > len
                            || str_.wrapping_add(startcol as usize) <= regmatch.startp[0]
                        {
                            match_found = false;
                            break;
                        }
                    }
                }

                if match_found {
                    match type_ {
                        SomeMatchType::SomeMatchStrPos => {
                            let ret_l = rettv.vval.v_list();
                            let li1 = tv_list_first(ret_l.as_ref());
                            let li1 = { &mut *li1 };
                            let li2 = li1.li_next;
                            let li3 = (*li2).li_next;
                            let li4 = (*li3).li_next;
                            tv_clear(&mut li1.li_tv);

                            let rd = crate::memory::ptr_offset_from(
                                regmatch.endp[0],
                                regmatch.startp[0],
                            ) as usize;
                            crate::eval::typval::tv_set_string_len(
                                &mut li1.li_tv,
                                regmatch.startp[0],
                                rd,
                            );
                            (*li3).li_tv.vval = crate::eval::typval::TypvalVval::Number(
                                crate::memory::ptr_offset_from(regmatch.startp[0], expr)
                                    as Varnumber,
                            );
                            (*li4).li_tv.vval = crate::eval::typval::TypvalVval::Number(
                                crate::memory::ptr_offset_from(regmatch.endp[0], expr) as Varnumber,
                            );
                            if !l.is_null() {
                                (*li2).li_tv.vval =
                                    crate::eval::typval::TypvalVval::Number(Varnumber::from(idx));
                            }
                        }
                        SomeMatchType::SomeMatchList => {
                            for i in 0..NSUBEXP {
                                if regmatch.endp[i].is_null() {
                                    tv_list_append_string(rettv.vval.v_list_ref(), ptr::null(), 0);
                                } else {
                                    tv_list_append_string(
                                        rettv.vval.v_list_ref(),
                                        regmatch.startp[i],
                                        crate::memory::ptr_offset_from(
                                            regmatch.endp[i],
                                            regmatch.startp[i],
                                        ),
                                    );
                                }
                            }
                        }
                        SomeMatchType::SomeMatchStr => {
                            if !l.is_null() {
                                tv_copy(&(*li).li_tv, &mut *rettv);
                            } else {
                                crate::eval::typval::tv_set_string_len(
                                    &mut *rettv,
                                    regmatch.startp[0],
                                    crate::memory::ptr_offset_from(
                                        regmatch.endp[0],
                                        regmatch.startp[0],
                                    ) as usize,
                                );
                            }
                        }
                        SomeMatchType::SomeMatch | SomeMatchType::SomeMatchEnd => {
                            if !l.is_null() {
                                rettv.vval =
                                    crate::eval::typval::TypvalVval::Number(Varnumber::from(idx));
                            } else {
                                if type_ == SomeMatchType::SomeMatch {
                                    rettv.vval = crate::eval::typval::TypvalVval::Number(
                                        crate::memory::ptr_offset_from(regmatch.startp[0], str_)
                                            as Varnumber,
                                    );
                                } else {
                                    rettv.vval = crate::eval::typval::TypvalVval::Number(
                                        crate::memory::ptr_offset_from(regmatch.endp[0], str_)
                                            as Varnumber,
                                    );
                                }
                                *rettv.vval.v_number_mut() +=
                                    crate::memory::ptr_offset_from(str_, expr) as Varnumber;
                            }
                        }
                    }
                }
                vim_regfree(regmatch.regprog);
            }
        }

        if type_ == SomeMatchType::SomeMatchStrPos && l.is_null() && !rettv.vval.v_list().is_null()
        {
            let ret_l = rettv.vval.v_list();
            let first = tv_list_first(ret_l.as_ref());
            tv_list_item_remove(&mut *ret_l, (*first).li_next);
        }

        drop(tofree_owned);
        opt_vars().p_cpo = save_cpo;
    }
}

fn get_matches_in_str(
    str_: impl crate::strings::IntoCCharPtr,
    rmp: &mut Regmatch,
    mlist: &mut TvList,
    idx: i32,
    submatches: bool,
    matchbuf: bool,
) {
    let str_ = str_.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(str_);
        let mut startidx: Colnr = 0;

        loop {
            if !vim_regexec_nl(rmp, str_, startidx) {
                break;
            }

            let d = tv_dict_alloc();
            tv_list_append_dict(&mut *mlist, d);

            if matchbuf {
                tv_dict_add_nr(&mut *d, c"lnum", Varnumber::from(idx));
            } else {
                tv_dict_add_nr(&mut *d, c"idx", Varnumber::from(idx));
            }

            tv_dict_add_nr(
                &mut *d,
                c"byteidx",
                crate::memory::ptr_offset_from(rmp.startp[0], str_) as Varnumber,
            );

            tv_dict_add_str_len(
                &mut *d,
                c"text",
                rmp.startp[0],
                crate::memory::ptr_offset_from(rmp.endp[0], rmp.startp[0]) as i32,
            );

            if submatches {
                let sml = tv_list_alloc((NSUBEXP - 1) as isize);
                tv_dict_add_list(&mut *d, c"submatches", sml);
                for i in 1..NSUBEXP {
                    if rmp.endp[i].is_null() {
                        tv_list_append_string(&mut *sml, c"", 0);
                    } else {
                        tv_list_append_string(
                            &mut *sml,
                            rmp.startp[i],
                            crate::memory::ptr_offset_from(rmp.endp[i], rmp.startp[i]),
                        );
                    }
                }
            }
            startidx = crate::memory::ptr_offset_from(rmp.endp[0], str_) as Colnr;
            if (startidx as usize) >= len || str_.wrapping_add(startidx as usize) <= rmp.startp[0] {
                break;
            }
        }
    }
}

/// "matchbufline()" function
pub fn f_matchbufline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);
        let retlist = rettv.vval.v_list();

        if !tv_check_for_buffer_arg(argvars, 0)
            || !tv_check_for_string_arg(argvars, 1)
            || !tv_check_for_lnum_arg(argvars, 2)
            || !tv_check_for_lnum_arg(argvars, 3)
            || !tv_check_for_opt_dict_arg(argvars, 4)
        {
            return;
        }

        let prev_did_emsg = g_st().did_emsg;
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() {
            if g_st().did_emsg == prev_did_emsg {
                semsg!(
                    gettext(E_INVALID_BUFFER_NAME_STR),
                    tv_get_string(&argvars[0]),
                );
            }
            return;
        }
        if (*buf).b_ml.ml_mfp.is_none() {
            emsg(gettext(E_BUFFER_IS_NOT_LOADED));
            return;
        }

        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let pat = tv_get_string_buf(&argvars[1], patbuf.as_mut_ptr());

        let did_emsg_before = g_st().did_emsg;
        let mut slnum = tv_get_lnum_buf(&argvars[2], buf.as_ref());
        if g_st().did_emsg > did_emsg_before {
            return;
        }
        if slnum < 1 {
            semsg!(gettext(E_INVARGVAL), c"lnum");
            return;
        }

        let mut elnum = tv_get_lnum_buf(&argvars[3], buf.as_ref());
        if g_st().did_emsg > did_emsg_before {
            return;
        }
        if elnum < 1 || elnum < slnum {
            semsg!(gettext(E_INVARGVAL), c"end_lnum");
            return;
        }

        if elnum > (*buf).b_ml.ml_line_count {
            elnum = (*buf).b_ml.ml_line_count;
        }

        let mut submatches = false;
        if argvars[4].v_type != VarType::VarUnknown {
            let d = argvars[4].vval.v_dict();
            if !d.is_null() {
                let di = tv_dict_find(d.as_ref(), c"submatches", 10);
                if !di.is_null() {
                    let di = { &mut *di };
                    if di.di_tv.v_type != VarType::VarBool {
                        semsg!(gettext(E_INVARGVAL), c"submatches");
                        return;
                    }
                    submatches = tv_get_bool(&di.di_tv) != 0;
                }
            }
        }

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        let mut regmatch: Regmatch = Regmatch::new();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = opt_vars().p_ic != 0;

            while slnum <= elnum {
                let str_ = ml_get_buf(&mut *buf, slnum);
                get_matches_in_str(str_, &mut regmatch, &mut *retlist, slnum, submatches, true);
                slnum += 1;
            }

            vim_regfree(regmatch.regprog);
        }

        opt_vars().p_cpo = save_cpo;
    }
}

/// "match()" function
pub fn f_match(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    find_some_match(argvars, rettv, SomeMatchType::SomeMatch);
}

/// "matchend()" function
pub fn f_matchend(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    find_some_match(argvars, rettv, SomeMatchType::SomeMatchEnd);
}

/// "matchlist()" function
pub fn f_matchlist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    find_some_match(argvars, rettv, SomeMatchType::SomeMatchList);
}

/// "matchstr()" function
pub fn f_matchstr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    find_some_match(argvars, rettv, SomeMatchType::SomeMatchStr);
}

/// "matchstrlist()" function
pub fn f_matchstrlist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);
        let retlist = rettv.vval.v_list();

        if !tv_check_for_list_arg(argvars, 0)
            || !tv_check_for_string_arg(argvars, 1)
            || !tv_check_for_opt_dict_arg(argvars, 2)
        {
            return;
        }

        let l = argvars[0].vval.v_list();
        if l.is_null() {
            return;
        }

        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let pat = tv_get_string_buf_chk(&argvars[1], patbuf.as_mut_ptr());
        if pat.is_null() {
            return;
        }

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        let mut regmatch: Regmatch = Regmatch::new();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = opt_vars().p_ic != 0;

            'cleanup: {
                let mut submatches = false;
                if argvars[2].v_type != VarType::VarUnknown {
                    let d = argvars[2].vval.v_dict();
                    if !d.is_null() {
                        let di = tv_dict_find(unsafe { d.as_ref() }, c"submatches", 10);
                        if !di.is_null() {
                            let di = { unsafe { &mut *di } };
                            if di.di_tv.v_type != VarType::VarBool {
                                semsg!(gettext(E_INVARGVAL), c"submatches");
                                break 'cleanup;
                            }
                            submatches = tv_get_bool(&di.di_tv) != 0;
                        }
                    }
                }

                for (idx, li) in
                    (0_i32..).zip(crate::eval::typval::list_iter_ptr(tv_list_first(unsafe {
                        l.as_ref()
                    })))
                {
                    let li_tv = &li.li_tv;
                    if li_tv.v_type == VarType::VarString && !li_tv.vval.v_string().is_null() {
                        get_matches_in_str(
                            li_tv.vval.v_string(),
                            &mut regmatch,
                            unsafe { &mut *retlist },
                            idx,
                            submatches,
                            false,
                        );
                    }
                }
            }

            vim_regfree(regmatch.regprog);
        }

        opt_vars().p_cpo = save_cpo;
    }
}

/// "matchstrpos()" function
pub fn f_matchstrpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    find_some_match(argvars, rettv, SomeMatchType::SomeMatchStrPos);
}

/// Get maximal/minimal number value in a list or dictionary.
fn max_min(tv: &TypVal, rettv: &mut TypVal, domax: bool) {
    {
        let mut error = false;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        let mut n: Varnumber = if domax { VARNUMBER_MIN } else { VARNUMBER_MAX };
        if tv.v_type == VarType::VarList {
            if tv_list_len(unsafe { tv.vval.v_list().as_ref() }) == 0 {
                return;
            }
            for li in crate::eval::typval::list_iter_mut(tv_list_first(unsafe {
                tv.vval.v_list().as_ref()
            })) {
                let i = tv_get_number_chk(&li.li_tv, Some(&mut error));
                if error {
                    return;
                }
                if if domax { i > n } else { i < n } {
                    n = i;
                }
            }
        } else if tv.v_type == VarType::VarDict {
            let d = tv.vval.v_dict();
            if tv_dict_len(unsafe { d.as_ref() }) == 0 {
                return;
            }
            let ht = unsafe { &(*d).dv_hashtab };
            let mut todo = ht.ht_used;
            let mut hi = (ht).array_ptr();
            while todo > 0 {
                if !crate::hashtab::hashitem_empty(unsafe { &*hi }) {
                    todo -= 1;
                    let di = tv_dict_hi2di(hi);
                    let i = tv_get_number_chk(&di.di_tv, Some(&mut error));
                    if error {
                        return;
                    }
                    if if domax { i > n } else { i < n } {
                        n = i;
                    }
                }
                hi = hi.wrapping_add(1);
            }
        } else {
            semsg!(
                gettext(E_LISTDICTARG),
                if domax {
                    c"max()".as_ptr()
                } else {
                    c"min()".as_ptr()
                },
            );
            return;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(n);
    }
}

/// "max()" function
pub fn f_max(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    max_min(&argvars[0], rettv, true);
}

/// "min()" function
pub fn f_min(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    max_min(&argvars[0], rettv, false);
}

/// "mode()" function
pub fn f_mode(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf = [0 as c_char; MODE_MAX_LENGTH];
    get_mode(buf.as_mut_ptr());
    if !non_zero_arg(&argvars[0]) {
        buf[1] = NUL;
    }
    crate::eval::typval::tv_set_string(rettv, buf.as_ptr());
}

fn may_add_state_char(
    buf: &mut [c_char],
    len: &mut usize,
    include: impl crate::strings::IntoCCharPtr,
    c: u8,
) {
    let include = include.into_c_char_ptr();
    if include.is_null() || crate::strings::vim_haschr(include, i32::from(c)) {
        buf[*len] = c as c_char;
        *len += 1;
    }
}

/// "state()" function
pub fn f_state(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut buf = [0 as c_char; 16];
        let mut len = 0usize;
        let mut include: *const c_char = ptr::null();
        if argvars[0].v_type != VarType::VarUnknown {
            include = tv_get_string(&argvars[0]);
        }

        if !(stuff_empty() && g_st().typebuf.tb_len == 0 && using_script() == 0) {
            may_add_state_char(&mut buf, &mut len, include, b'm');
        }
        if op_pending() {
            may_add_state_char(&mut buf, &mut len, include, b'o');
        }
        if g_st().autocmd_busy {
            may_add_state_char(&mut buf, &mut len, include, b'x');
        }
        if ins_compl_active() {
            may_add_state_char(&mut buf, &mut len, include, b'a');
        }
        if !get_was_safe_state() {
            may_add_state_char(&mut buf, &mut len, include, b'S');
        }
        let depth = get_callback_depth();
        for _ in 0..(depth.min(3)) {
            may_add_state_char(&mut buf, &mut len, include, b'c');
        }
        if g_st().msg_scrolled > 0 {
            may_add_state_char(&mut buf, &mut len, include, b's');
        }

        if len == 0 {
            rettv.v_type = VarType::VarString;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        } else {
            crate::eval::typval::tv_set_string_len(rettv, buf.as_ptr(), len);
        }
    }
}

/// "msgpackdump()" function
pub fn f_msgpackdump(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarList {
            semsg!(gettext(E_LISTARG), c"msgpackdump()");
            return;
        }
        let list = argvars[0].vval.v_list();
        let mut packer: PackerBuffer = packer_string_buffer();
        let msg = gettext(c"msgpackdump() argument, index %i");
        let mut msgbuf = [0 as c_char; 31 * 4 + NUMBUFLEN];
        for (idx, li) in (0_i32..).zip(crate::eval::typval::list_iter_mut(tv_list_first(unsafe {
            list.as_ref()
        }))) {
            vim_snprintf!(msgbuf.as_mut_ptr(), msgbuf.len(), msg, idx);
            if !encode_vim_to_msgpack(&mut packer, &mut li.li_tv, msgbuf.as_ptr()) {
                break;
            }
        }
        let data = packer_take_string(&mut packer);
        if argvars[1].v_type != VarType::VarUnknown
            && (crate::memory::cstr_as_opt_bytes(tv_get_string(&argvars[1])) == Some(b"B"))
        {
            let b = &mut *tv_blob_alloc_ret(rettv);
            if !data.is_empty() {
                b.bv_ga.extend_from_slice(&data);
            }
        } else {
            encode_list_write(
                Some(&mut *(tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST))),
                data.as_ptr().cast::<c_char>(),
                data.len(),
            );
        }
    }
}

fn emsg_mpack_error(status: i32) {
    match status {
        MPACK_ERROR => {
            semsg!(gettext(E_INVARG2), c"Failed to parse msgpack string",);
        }
        MPACK_EOF => {
            semsg!(gettext(E_INVARG2), c"Incomplete msgpack string",);
        }
        MPACK_NOMEM => {
            semsg!(gettext(E_INVARG2), c"object was too deep to unpack",);
        }
        _ => {}
    }
}

fn msgpackparse_unpack_list(list: Option<&TvList>, ret_list: &mut TvList) {
    {
        if tv_list_len(list) == 0 {
            return;
        }
        if unsafe { (*tv_list_first(list)).li_tv.v_type } != VarType::VarString {
            semsg!(gettext(E_INVARG2), c"List item is not a string",);
            return;
        }
        let mut lrstate: ListReaderState = encode_init_lrstate(list);
        let buf = alloc_block();
        let mut buf_size: usize = 0;

        let mut cur_item = tv_initial_value();
        let mut parser = MpackParser::new(0);
        mpack_parser_init(&mut parser, 0);
        parser.data = crate::mpack::object::MpackData::Typval(&raw mut cur_item);

        let mut status = MPACK_OK;
        'end: {
            loop {
                let mut read_bytes: usize = 0;
                let rlret = encode_read_from_list(
                    &raw mut lrstate,
                    buf.wrapping_add(buf_size),
                    ARENA_BLOCK_SIZE - buf_size,
                    &mut read_bytes,
                );
                if rlret == FAIL {
                    semsg!(gettext(E_INVARG2), c"List item is not a string",);
                    break 'end;
                }
                buf_size += read_bytes;

                let mut ptr_ = buf.cast::<c_char>().cast_const();
                while buf_size != 0 {
                    status = mpack_parse_typval(&mut parser, &mut ptr_, &mut buf_size);
                    if status == MPACK_OK {
                        tv_list_append_owned_tv(&mut *ret_list, cur_item);
                        cur_item.v_type = VarType::VarUnknown;
                    } else {
                        break;
                    }
                }

                if rlret == OK {
                    break;
                }

                if status == MPACK_EOF {
                    if buf_size != 0 && ptr_ > buf.cast::<c_char>().cast_const() {
                        unsafe { ptr::copy(ptr_.cast_mut(), buf, buf_size) };
                    }
                } else if status != MPACK_OK {
                    break;
                }
            }

            if status != MPACK_OK {
                typval_parser_error_free(&mut parser);
                emsg_mpack_error(status);
            }
        }

        free_block(buf);
    }
}

fn msgpackparse_unpack_blob(blob: Option<&TvBlob>, ret_list: &mut TvList) {
    {
        let len = tv_blob_len(blob);
        if len == 0 {
            return;
        }

        let mut data = blob.unwrap().bv_ga.as_ptr().cast::<c_char>();
        let mut remaining = len as usize;
        while remaining != 0 {
            let mut tv = tv_initial_value();
            let status = unpack_typval(&mut data, &mut remaining, &mut tv);
            if status != MPACK_OK {
                emsg_mpack_error(status);
                return;
            }
            tv_list_append_owned_tv(&mut *ret_list, tv);
        }
    }
}

/// "msgpackparse()" function
pub fn f_msgpackparse(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarList && argvars[0].v_type != VarType::VarBlob {
            semsg!(gettext(E_LISTBLOBARG), c"msgpackparse()");
            return;
        }
        let ret_list = tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        if argvars[0].v_type == VarType::VarList {
            msgpackparse_unpack_list(unsafe { argvars[0].vval.v_list().as_ref() }, &mut *ret_list);
        } else {
            msgpackparse_unpack_blob(unsafe { argvars[0].vval.v_blob().as_ref() }, &mut *ret_list);
        }
    }
}

/// "nextnonblank()" function
pub fn f_nextnonblank(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut lnum = tv_get_lnum(&argvars[0]);
        loop {
            if lnum < 0 || lnum > curbuf_mut().b_ml.ml_line_count {
                lnum = 0;
                break;
            }
            if !crate::memory::cstr_is_empty(skipwhite(ml_get(lnum))) {
                break;
            }
            lnum += 1;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(lnum));
    }
}

/// "nr2char()" function
pub fn f_nr2char(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[1].v_type != VarType::VarUnknown && !tv_check_num(&argvars[1]) {
            return;
        }
        let mut error = false;
        let num = tv_get_number_chk(&argvars[0], Some(&mut error));
        if error {
            return;
        }
        if num < 0 {
            emsg(gettext(
                c"E5070: Character number must not be less than zero",
            ));
            return;
        }
        if num > Varnumber::from(i32::MAX) {
            semsg!(
                gettext(c"E5071: Character number must not be greater than INT_MAX (%i)"),
                i32::MAX,
            );
            return;
        }

        let mut buf = [0 as c_char; MB_MAXCHAR];
        let len = utf_char2bytes(num as i32, buf.as_mut_ptr());

        crate::eval::typval::tv_set_string_len(rettv, buf.as_ptr(), len as usize);
    }
}

/// "or(expr, expr)" function
pub fn f_or(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        tv_get_number_chk(&argvars[0], None) | tv_get_number_chk(&argvars[1], None),
    );
}

/// "pow()" function
pub fn f_pow(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut fx: TvFloat = 0.0;
    let mut fy: TvFloat = 0.0;
    rettv.v_type = VarType::VarFloat;
    if tv_get_float_chk(&argvars[0], &mut fx) && tv_get_float_chk(&argvars[1], &mut fy) {
        rettv.vval = crate::eval::typval::TypvalVval::Float(fx.powf(fy));
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Float(0.0);
    }
}

/// "prevnonblank()" function
pub fn f_prevnonblank(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut lnum = tv_get_lnum(&argvars[0]);
        if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count {
            lnum = 0;
        } else {
            while lnum >= 1 && unsafe { *skipwhite(ml_get(lnum)) } == NUL {
                lnum -= 1;
            }
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(lnum));
    }
}

/// "printf()" function
pub fn f_printf(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        let saved_did_emsg = g_st().did_emsg;
        g_st().did_emsg = 0;
        let mut buf = [0 as c_char; NUMBUFLEN];
        let fmt = tv_get_string_buf(&argvars[0], buf.as_mut_ptr());
        let len = vim_snprintf_typval_only(ptr::null_mut(), 0, fmt, (&raw mut argvars[1]));
        if g_st().did_emsg == 0 {
            let mut s = vec![0 as c_char; (len as usize) + 1];
            vim_snprintf_typval_only(
                s.as_mut_ptr(),
                (len as usize) + 1,
                fmt,
                (&raw mut argvars[1]),
            );
            crate::eval::typval::tv_set_string_len(rettv, s.as_ptr(), len as usize);
        }
        g_st().did_emsg |= saved_did_emsg;
    }
}

/// "pum_getpos()" function
pub fn f_pum_getpos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    pum_set_event_info(tv_dict_alloc_ret(rettv));
}

/// "pumvisible()" function
pub fn f_pumvisible(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if pum_visible() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    }
}

/// "py3eval()" and "pyxeval()" functions
pub fn f_py3eval(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    script_host_eval(c"python3", argvars, rettv);
}

fn init_srand(x: &mut u32) {
    {
        let mut number: u32 = 0;
        if unsafe {
            uv_random(
                ptr::null_mut(),
                ptr::null_mut(),
                (&raw mut number).cast(),
                size_of::<u32>(),
                0,
                ptr::null(),
            )
        } == 0
        {
            *x = number;
            return;
        }
        *x = (os_hrtime() as u32) ^ (os_get_pid() as u32);
    }
}

#[inline(always)]
fn splitmix32(x: &mut u32) -> u32 {
    *x = (*x).wrapping_add(0x9e3779b9);
    let mut z = *x;
    z = (z ^ (z >> 16)).wrapping_mul(0x85ebca6b);
    z = (z ^ (z >> 13)).wrapping_mul(0xc2b2ae35);
    z ^ (z >> 16)
}

#[inline(always)]
fn shuffle_xoshiro128starstar(x: &mut u32, y: &mut u32, z: &mut u32, w: &mut u32) -> u32 {
    let result = ((*y).wrapping_mul(5)).rotate_left(7).wrapping_mul(9);
    let t = *y << 9;
    *z ^= *x;
    *w ^= *y;
    *y ^= *z;
    *x ^= *w;
    *z ^= t;
    *w = (*w).rotate_left(11);
    result
}

/// "rand()" function
pub fn f_rand(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let result: u32;

        if argvars[0].v_type == VarType::VarUnknown {
            if !funcs_st().rand_initialized {
                let mut x: u32 = 0;
                init_srand(&mut x);
                funcs_st().rand_gx = splitmix32(&mut x);
                funcs_st().rand_gy = splitmix32(&mut x);
                funcs_st().rand_gz = splitmix32(&mut x);
                funcs_st().rand_gw = splitmix32(&mut x);
                funcs_st().rand_initialized = true;
            }
            result = shuffle_xoshiro128starstar(
                &mut funcs_st().rand_gx,
                &mut funcs_st().rand_gy,
                &mut funcs_st().rand_gz,
                &mut funcs_st().rand_gw,
            );
        } else if argvars[0].v_type == VarType::VarList {
            let l = argvars[0].vval.v_list();
            if tv_list_len(l.as_ref()) != 4 {
                semsg!(gettext(E_INVARG2), tv_get_string(&argvars[0]));
                rettv.v_type = VarType::VarNumber;
                rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
                return;
            }

            let tvx = &mut (*tv_list_find(l.as_mut(), 0)).li_tv;
            let tvy = &mut (*tv_list_find(l.as_mut(), 1)).li_tv;
            let tvz = &mut (*tv_list_find(l.as_mut(), 2)).li_tv;
            let tvw = &mut (*tv_list_find(l.as_mut(), 3)).li_tv;
            if tvx.v_type != VarType::VarNumber
                || tvy.v_type != VarType::VarNumber
                || tvz.v_type != VarType::VarNumber
                || tvw.v_type != VarType::VarNumber
            {
                semsg!(gettext(E_INVARG2), tv_get_string(&argvars[0]));
                rettv.v_type = VarType::VarNumber;
                rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
                return;
            }
            let mut x = tvx.vval.v_number() as u32;
            let mut y = tvy.vval.v_number() as u32;
            let mut z = tvz.vval.v_number() as u32;
            let mut w = tvw.vval.v_number() as u32;

            result = shuffle_xoshiro128starstar(&mut x, &mut y, &mut z, &mut w);

            tvx.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(x));
            tvy.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(y));
            tvz.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(z));
            tvw.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(w));
        } else {
            semsg!(gettext(E_INVARG2), tv_get_string(&argvars[0]));
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
            return;
        }

        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(result));
    }
}

/// "srand()" function
pub fn f_srand(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut x: u32 = 0;
        tv_list_alloc_ret(rettv, 4);
        if argvars[0].v_type == VarType::VarUnknown {
            init_srand(&mut x);
        } else {
            let mut error = false;
            x = tv_get_number_chk(&argvars[0], Some(&mut error)) as u32;
            if error {
                return;
            }
        }
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(splitmix32(&mut x)));
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(splitmix32(&mut x)));
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(splitmix32(&mut x)));
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(splitmix32(&mut x)));
    }
}

/// "perleval()" function
pub fn f_perleval(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    script_host_eval(c"perl", argvars, rettv);
}

/// "rubyeval()" function
pub fn f_rubyeval(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    script_host_eval(c"ruby", argvars, rettv);
}

/// "range()" function
pub fn f_range(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let end: Varnumber;
        let mut stride: Varnumber = 1;
        let mut error = false;

        let mut start = tv_get_number_chk(&argvars[0], Some(&mut error));
        if argvars[1].v_type == VarType::VarUnknown {
            end = start - 1;
            start = 0;
        } else {
            end = tv_get_number_chk(&argvars[1], Some(&mut error));
            if argvars[2].v_type != VarType::VarUnknown {
                stride = tv_get_number_chk(&argvars[2], Some(&mut error));
            }
        }

        if error {
            return;
        }
        if stride == 0 {
            emsg(gettext(c"E726: Stride is zero"));
            return;
        }
        if if stride > 0 {
            end.saturating_add(1) < start
        } else {
            end.saturating_sub(1) > start
        } {
            emsg(gettext(c"E727: Start past end"));
            return;
        }

        tv_list_alloc_ret(rettv, ((end - start) / stride) as isize);
        let mut i = start;
        while if stride > 0 { i <= end } else { i >= end } {
            tv_list_append_number(rettv.vval.v_list_ref(), i);
            i += stride;
        }
    }
}

/// "getreginfo()" function
pub fn f_getreginfo(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut regname = getreg_get_regname(argvars);
    if regname == 0 {
        return;
    }
    if regname == b'@' as i32 {
        regname = b'"' as i32;
    }

    let dict = tv_dict_alloc_ret(rettv);

    let list = get_reg_contents_list(regname, K_GREG_EXPR_SRC);
    if list.is_null() {
        return;
    }
    tv_dict_add_list(dict, c"regcontents", list);

    let mut buf = [0 as c_char; NUMBUFLEN + 2];
    let mut reglen: Colnr = 0;
    let mut buflen = match get_reg_type(regname, Some(&mut reglen)) {
        crate::register::MotionType::MtLineWise => {
            buf[0] = b'V' as c_char;
            buf[1] = NUL;
            1
        }
        crate::register::MotionType::MtCharWise => {
            buf[0] = b'v' as c_char;
            buf[1] = NUL;
            1
        }
        crate::register::MotionType::MtBlockWise => vim_snprintf_safelen!(
            buf.as_mut_ptr(),
            buf.len(),
            c"%c%d",
            i32::from(CTRL_V_CONST),
            reglen + 1,
        ),
        crate::register::MotionType::MtUnknown => std::process::abort(),
    };
    tv_dict_add_str_len(dict, c"regtype", buf.as_ptr(), buflen as i32);

    buf[0] = get_register_name(get_unname_register()) as c_char;
    buf[1] = NUL;
    buflen = usize::from(buf[0] != NUL);
    if regname == b'"' as i32 {
        tv_dict_add_str_len(dict, c"points_to", buf.as_ptr(), buflen as i32);
    } else {
        tv_dict_add_bool(
            dict,
            c"isunnamed",
            if regname == i32::from(buf[0]) {
                K_BOOL_VAR_TRUE_CONST
            } else {
                K_BOOL_VAR_FALSE_CONST
            },
        );
    }
}

fn return_register(regname: i32, rettv: &mut TypVal) {
    {
        let buf = [regname as c_char, NUL];
        crate::eval::typval::tv_set_string_len(
            &mut *rettv,
            buf.as_ptr(),
            usize::from(buf[0] != NUL),
        );
    }
}

/// "reg_executing()" function
pub fn f_reg_executing(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    return_register(g_st().reg_executing, rettv);
}

/// "reg_recording()" function
pub fn f_reg_recording(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    return_register(g_st().reg_recording, rettv);
}

pub fn f_reg_recorded(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    return_register(g_st().reg_recorded, rettv);
}

fn list2proftime(arg: &mut TypVal, tm: &mut Proftime) -> bool {
    {
        if arg.v_type != VarType::VarList || tv_list_len(unsafe { arg.vval.v_list().as_ref() }) != 2
        {
            return false;
        }
        let mut error = false;
        let n1 = tv_list_find_nr(unsafe { arg.vval.v_list().as_mut() }, 0, Some(&mut error));
        let n2 = tv_list_find_nr(unsafe { arg.vval.v_list().as_mut() }, 1, Some(&mut error));
        if error {
            return false;
        }
        let high = (n1 as i32) as u32 as u64;
        let low = (n2 as i32) as u32 as u64;
        *tm = (high << 32) | low;
        true
    }
}

/// "reltime()" function
pub fn f_reltime(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut res: Proftime = 0;
        let mut start: Proftime = 0;

        if argvars[0].v_type == VarType::VarUnknown {
            res = profile_start();
        } else if argvars[1].v_type == VarType::VarUnknown {
            if !list2proftime(&mut argvars[0], &mut res) {
                return;
            }
            res = profile_end(res);
        } else {
            if !list2proftime(&mut argvars[0], &mut start)
                || !list2proftime(&mut argvars[1], &mut res)
            {
                return;
            }
            res = profile_sub(res, start);
        }

        let high = (res >> 32) as i32;
        let low = (res & 0xffff_ffff) as i32;

        tv_list_alloc_ret(rettv, 2);
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(high));
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(low));
    }
}

/// "reltimestr()" function
pub fn f_reltimestr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut tm: Proftime = 0;
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
    if list2proftime(&mut argvars[0], &mut tm) {
        crate::eval::typval::tv_set_string(rettv, profile_msg(tm));
    }
}

fn repeat_list(l: Option<&TvList>, mut n: Varnumber, rettv: &mut TypVal) {
    let slen = tv_list_len(l);
    tv_list_alloc_ret(
        &mut *rettv,
        (Varnumber::from(n > 0) * n * Varnumber::from(slen)) as isize,
    );
    if l.is_none() || n <= 0 || slen == 0 {
        return;
    }
    if !check_repeat_count(Varnumber::from(slen), n) {
        return;
    }
    while n > 0 {
        n -= 1;
        tv_list_extend(rettv.vval.v_list_ref(), l, ptr::null_mut());
    }
}

fn repeat_blob(blob_tv: &mut TypVal, n: Varnumber, rettv: &mut TypVal) {
    unsafe {
        let blob = blob_tv.vval.v_blob().cast::<TvBlob>();
        tv_blob_alloc_ret(rettv);
        if blob.is_null() || n <= 0 {
            return;
        }
        let slen = (*blob).bv_ga.len() as i32;
        if slen <= 0 {
            return;
        }
        if !check_repeat_count(Varnumber::from(slen), n) {
            return;
        }
        let len = slen * (n as i32);
        let ret_blob = rettv.vval.v_blob().cast::<TvBlob>();
        let ret_bv = &mut (*ret_blob).bv_ga;
        ret_bv.resize(len as usize, 0);

        let mut i: i32 = 0;
        while i < slen {
            if tv_blob_get(&*blob, i) != 0 {
                break;
            }
            i += 1;
        }
        if i == slen {
            return;
        }
        for idx in 0..(n as i32) {
            tv_blob_set_range(
                &mut *ret_blob,
                Varnumber::from(idx * slen),
                Varnumber::from((idx + 1) * slen - 1),
                &mut *blob_tv,
            );
        }
    }
}

fn repeat_string(str_tv: &mut TypVal, n: Varnumber, rettv: &mut TypVal) {
    {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        if n <= 0 {
            return;
        }
        let p = tv_get_string(&*str_tv);
        let slen = crate::memory::cstr_len(p);
        if slen == 0 || n <= 0 {
            return;
        }
        if !check_repeat_count(slen as Varnumber, n) {
            return;
        }
        let len = slen * (n as usize);
        let mut r_vec = vec![0 as c_char; len];
        let r = r_vec.as_mut_ptr();
        unsafe { ptr::copy(p, r, slen) };
        let mut done = slen;
        while done < len {
            let copy_len = done.min(len - done);
            unsafe { ptr::copy(r, r.wrapping_add(done), copy_len) };
            done += copy_len;
        }
        crate::eval::typval::tv_set_string_len(&mut *rettv, r, len);
    }
}

/// "repeat()" function
pub fn f_repeat(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let n = tv_get_number(&argvars[1]);
        if argvars[0].v_type == VarType::VarList {
            repeat_list(unsafe { argvars[0].vval.v_list().as_ref() }, n, rettv);
        } else if argvars[0].v_type == VarType::VarBlob {
            repeat_blob(&mut argvars[0], n, rettv);
        } else {
            repeat_string(&mut argvars[0], n, rettv);
        }
    }
}

fn reduce_list(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let expr = &argvars[1];
    unsafe {
        let l = argvars[0].vval.v_list();
        let called_emsg_start = g_st().called_emsg;

        let initial: TypVal;
        let mut li: *const Listitem;
        if argvars[2].v_type == VarType::VarUnknown {
            if tv_list_len(l.as_ref()) == 0 {
                semsg!(
                    gettext(E_REDUCE_OF_AN_EMPTY_STR_WITH_NO_INITIAL_VALUE),
                    c"List",
                );
                return;
            }
            let first = tv_list_first(l.as_ref());
            initial = (*first).li_tv;
            li = (*first).li_next;
        } else {
            initial = argvars[2];
            li = tv_list_first(l.as_ref());
        }

        tv_copy(&initial, &mut *rettv);
        if l.is_null() {
            return;
        }

        let prev_locked = tv_list_locked(l.as_ref());
        tv_list_set_lock(l.as_mut(), VAR_FIXED);
        while !li.is_null() {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            argv[1] = (*li).li_tv;
            rettv.v_type = VarType::VarUnknown;

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, &mut *rettv);
            tv_clear(&mut argv[0]);
            if !r || g_st().called_emsg != called_emsg_start {
                break;
            }
            li = (*li).li_next;
        }
        tv_list_set_lock(l.as_mut(), prev_locked);
    }
}

fn reduce_string(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let expr = &argvars[1];
    {
        let mut p = tv_get_string(&argvars[0]);
        let mut len: i32;
        let called_emsg_start = g_st().called_emsg;

        if argvars[2].v_type == VarType::VarUnknown {
            if crate::memory::cstr_is_empty(p) {
                semsg!(
                    gettext(E_REDUCE_OF_AN_EMPTY_STR_WITH_NO_INITIAL_VALUE),
                    c"String",
                );
                return;
            }
            len = utfc_ptr2len(p);
            *rettv = TypVal {
                v_type: VarType::VarUnknown,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            crate::eval::typval::tv_set_string_len(&mut *rettv, p, len as usize);
            p = p.wrapping_add(len as usize);
        } else if !tv_check_for_string_arg(argvars, 2) {
            return;
        } else {
            tv_copy(&argvars[2], &mut *rettv);
        }

        while !crate::memory::cstr_is_empty(p) {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            rettv.v_type = VarType::VarUnknown;
            len = utfc_ptr2len(p);
            crate::eval::typval::tv_set_string_len(&mut argv[1], p, len as usize);

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, &mut *rettv);
            tv_clear(&mut argv[0]);
            tv_clear(&mut argv[1]);
            if !r || g_st().called_emsg != called_emsg_start {
                break;
            }
            p = p.wrapping_add(len as usize);
        }
    }
}

fn reduce_blob(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let expr = &argvars[1];
    {
        let b = argvars[0].vval.v_blob().cast::<TvBlob>();
        let called_emsg_start = g_st().called_emsg;

        let initial: TypVal;
        let mut i: i32;
        if argvars[2].v_type == VarType::VarUnknown {
            if tv_blob_len(unsafe { b.as_ref() }) == 0 {
                semsg!(
                    gettext(E_REDUCE_OF_AN_EMPTY_STR_WITH_NO_INITIAL_VALUE),
                    c"Blob",
                );
                return;
            }
            initial = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(Varnumber::from(tv_blob_get(unsafe { &*b }, 0))),
            };
            i = 1;
        } else if !tv_check_for_number_arg(argvars, 2) {
            return;
        } else {
            initial = argvars[2];
            i = 0;
        }

        tv_copy(&initial, &mut *rettv);
        while i < tv_blob_len(unsafe { b.as_ref() }) {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            argv[1] = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(Varnumber::from(tv_blob_get(unsafe { &*b }, i))),
            };

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, &mut *rettv);
            if !r || g_st().called_emsg != called_emsg_start {
                return;
            }
            i += 1;
        }
    }
}

/// "reduce()" function
pub fn f_reduce(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarString
            && argvars[0].v_type != VarType::VarList
            && argvars[0].v_type != VarType::VarBlob
        {
            emsg(gettext(E_STRING_LIST_OR_BLOB_REQUIRED));
            return;
        }

        let func_name = if argvars[1].v_type == VarType::VarFunc {
            argvars[1].vval.v_string().cast_const()
        } else if argvars[1].v_type == VarType::VarPartial {
            partial_name(unsafe { argvars[1].vval.v_partial().as_ref() })
        } else {
            tv_get_string(&argvars[1])
        };
        if crate::memory::cstr_is_empty(func_name) {
            emsg(gettext(E_MISSING_FUNCTION_ARGUMENT));
            return;
        }

        if argvars[0].v_type == VarType::VarList {
            reduce_list(argvars, rettv);
        } else if argvars[0].v_type == VarType::VarString {
            reduce_string(argvars, rettv);
        } else {
            reduce_blob(argvars, rettv);
        }
    }
}

fn get_search_arg(varp: &mut TypVal, mut flagsp: Option<&mut i32>) -> i32 {
    {
        let mut dir = FORWARD;
        if varp.v_type == VarType::VarUnknown {
            return FORWARD;
        }
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut flags = tv_get_string_buf_chk(&*varp, nbuf.as_mut_ptr());
        if flags.is_null() {
            return 0;
        }
        while !crate::memory::cstr_is_empty(flags) {
            match crate::memory::cstr_first_byte(flags) {
                b'b' => dir = BACKWARD,
                b'w' => opt_vars().p_ws = 1,
                b'W' => opt_vars().p_ws = 0,
                b => {
                    let mut mask: i32 = 0;
                    if flagsp.is_some() {
                        mask = match b {
                            b'c' => SP_START,
                            b'e' => SP_END,
                            b'm' => SP_RETCOUNT,
                            b'n' => SP_NOMOVE,
                            b'p' => SP_SUBPAT,
                            b'r' => SP_REPEAT,
                            b's' => SP_SETPCMARK,
                            b'z' => SP_COLUMN,
                            _ => 0,
                        };
                    }
                    if mask == 0 {
                        semsg!(gettext(E_INVARG2), flags);
                        dir = 0;
                    } else {
                        *flagsp.as_deref_mut().unwrap() |= mask;
                    }
                }
            }
            if dir == 0 {
                break;
            }
            flags = flags.wrapping_add(1);
        }
        dir
    }
}

fn search_cmn(argvars: &mut [TypVal], match_pos: Option<&mut CursorPos>, flagsp: &mut i32) -> i32 {
    {
        let save_p_ws = opt_vars().p_ws;
        let mut retval: i32 = 0;
        let mut lnum_stop: Linenr = 0;
        let mut time_limit: i64 = 0;
        let mut options = SEARCH_KEEP;
        let mut use_skip = false;

        'theend: {
            let pat = tv_get_string(&argvars[0]);
            let dir = get_search_arg(&mut argvars[1], Some(&mut *flagsp));
            if dir == 0 {
                break 'theend;
            }
            let flags = *flagsp;
            if (flags & SP_START) != 0 {
                options |= SEARCH_START;
            }
            if (flags & SP_END) != 0 {
                options |= SEARCH_END;
            }
            if (flags & SP_COLUMN) != 0 {
                options |= SEARCH_COL;
            }

            if argvars[1].v_type != VarType::VarUnknown && argvars[2].v_type != VarType::VarUnknown
            {
                lnum_stop = tv_get_number_chk(&argvars[2], None) as Linenr;
                if lnum_stop < 0 {
                    break 'theend;
                }
                if argvars[3].v_type != VarType::VarUnknown {
                    time_limit = tv_get_number_chk(&argvars[3], None);
                    if time_limit < 0 {
                        break 'theend;
                    }
                    use_skip = eval_expr_valid_arg(&argvars[4]);
                }
            }

            let tm = profile_setlimit(time_limit);

            if ((flags & (SP_REPEAT | SP_RETCOUNT)) != 0)
                || ((flags & SP_NOMOVE) != 0 && (flags & SP_SETPCMARK) != 0)
            {
                semsg!(gettext(E_INVARG2), tv_get_string(&argvars[1]));
                break 'theend;
            }

            let save_cursor = curwin_mut().w_cursor;
            let mut pos = CursorPos {
                lnum: save_cursor.lnum,
                col: save_cursor.col,
                coladd: save_cursor.coladd,
            };
            let mut firstpos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut sia = SearchitArg {
                sa_stop_lnum: lnum_stop,
                sa_tm: Some(tm),
                sa_timed_out: 0,
                sa_wrapped: 0,
            };

            let patlen = crate::memory::cstr_len(pat);
            let mut subpatnum: i32;

            loop {
                subpatnum = searchit(
                    curwin_ptr(),
                    curbuf_mut(),
                    unsafe { &mut *(pos.cast()) },
                    None,
                    dir,
                    pat.cast_mut(),
                    patlen,
                    1,
                    options,
                    RE_SEARCH,
                    opt_vars().p_magic != 0,
                    Some(&mut sia),
                );
                if firstpos.lnum != 0 && equalpos(pos, firstpos) {
                    subpatnum = FAIL;
                }
                if subpatnum == FAIL || !use_skip {
                    break;
                }
                if firstpos.lnum == 0 {
                    firstpos = pos;
                }

                let save_pos = curwin_mut().w_cursor;
                curwin_mut().w_cursor.lnum = pos.lnum;
                curwin_mut().w_cursor.col = pos.col;
                curwin_mut().w_cursor.coladd = pos.coladd;
                let mut err = false;
                let do_skip = eval_expr_to_bool(&argvars[4], &mut err);
                curwin_mut().w_cursor = save_pos;
                if err {
                    subpatnum = FAIL;
                    break;
                }
                if !do_skip {
                    break;
                }

                options &= !SEARCH_START;
            }

            if subpatnum != FAIL {
                if (flags & SP_SUBPAT) != 0 {
                    retval = subpatnum;
                } else {
                    retval = pos.lnum;
                }
                if (flags & SP_SETPCMARK) != 0 {
                    setpcmark();
                }
                curwin_mut().w_cursor.lnum = pos.lnum;
                curwin_mut().w_cursor.col = pos.col;
                curwin_mut().w_cursor.coladd = pos.coladd;
                if let Some(match_pos) = match_pos {
                    match_pos.lnum = pos.lnum;
                    match_pos.col = pos.col + 1;
                }
                check_cursor(curwin_mut());
            }

            if (flags & SP_NOMOVE) != 0 {
                curwin_mut().w_cursor = save_cursor;
            } else {
                curwin_mut().w_set_curswant = true;
            }
        }
        opt_vars().p_ws = save_p_ws;
        retval
    }
}

/// "rpcnotify()" function
pub fn f_rpcnotify(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarNumber || argvars[0].vval.v_number() < 0 {
            semsg!(gettext(E_INVARG2), c"Channel id must be a positive integer",);
            return;
        }
        if argvars[1].v_type != VarType::VarString {
            semsg!(gettext(E_INVARG2), c"Event type must be a string",);
            return;
        }

        let mut args_storage = [Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Boolean(false),
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = (&raw mut argvars[2]);
        while unsafe { (*tv).v_type } != VarType::VarUnknown {
            *args.next_slot_mut() = vim_to_object(tv, Some(&mut arena), true);
            args.size += 1;
            tv = tv.wrapping_add(1);
        }

        let ok = rpc_send_event(
            argvars[0].vval.v_number() as u64,
            tv_get_string(&argvars[1]),
            args,
        );

        arena_mem_free(arena_finish(&mut arena));

        if !ok {
            semsg!(gettext(E_INVARG2), c"Channel doesn't exist",);
            return;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    }
}

/// "rpcrequest()" function
pub fn f_rpcrequest(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        let l_provider_call_nesting = g_st().provider_call_nesting;

        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarNumber || argvars[0].vval.v_number() <= 0 {
            semsg!(gettext(E_INVARG2), c"Channel id must be a positive integer",);
            return;
        }
        if argvars[1].v_type != VarType::VarString {
            semsg!(gettext(E_INVARG2), c"Method name must be a string",);
            return;
        }

        let mut args_storage = [Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Boolean(false),
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = (&raw mut argvars[2]);
        while unsafe { (*tv).v_type } != VarType::VarUnknown {
            *args.next_slot_mut() = vim_to_object(tv, Some(&mut arena), true);
            args.size += 1;
            tv = tv.wrapping_add(1);
        }

        let mut save_current_sctx = Sctx::new();
        let mut save_autocmd_fname: Option<std::ffi::CString> = None;
        let mut save_autocmd_match: Option<std::ffi::CString> = None;
        let mut save_autocmd_fname_full = false;
        let mut save_autocmd_bufnr: i32 = 0;

        if l_provider_call_nesting != 0 {
            save_current_sctx = g_st().current_sctx;
            save_autocmd_fname = g_st().autocmd_fname.take();
            save_autocmd_match = g_st().autocmd_match.take();
            save_autocmd_fname_full = g_st().autocmd_fname_full;
            save_autocmd_bufnr = g_st().autocmd_bufnr;
            save_funccal();

            g_st().current_sctx = g_st().provider_caller_scope.script_ctx;
            rt_st().exestack.push(
                unsafe { (&raw const g_st().provider_caller_scope.es_entry).as_ref() }
                    .unwrap()
                    .clone(),
            );
            g_st().autocmd_fname =
                unsafe { (&raw const g_st().provider_caller_scope.autocmd_fname).as_ref() }
                    .unwrap()
                    .clone();
            g_st().autocmd_match =
                unsafe { (&raw const g_st().provider_caller_scope.autocmd_match).as_ref() }
                    .unwrap()
                    .clone();
            g_st().autocmd_fname_full = g_st().provider_caller_scope.autocmd_fname_full;
            g_st().autocmd_bufnr = g_st().provider_caller_scope.autocmd_bufnr;
            set_current_funccal(g_st().provider_caller_scope.funccalp);
        }

        let mut err = Error::new();
        let chan_id = argvars[0].vval.v_number() as u64;
        let method = tv_get_string(&argvars[1]);

        let mut res_mem: ArenaMem = ptr::null_mut();
        let result = rpc_send_call(chan_id, method, args, &mut res_mem, &mut err);
        arena_mem_free(arena_finish(&mut arena));

        if l_provider_call_nesting != 0 {
            g_st().current_sctx = save_current_sctx;
            rt_st().exestack.pop();
            g_st().autocmd_fname = save_autocmd_fname;
            g_st().autocmd_match = save_autocmd_match;
            g_st().autocmd_fname_full = save_autocmd_fname_full;
            g_st().autocmd_bufnr = save_autocmd_bufnr;
            restore_funccal();
        }

        if err.err_type != ErrorType::ErrorTypeNone {
            let mut name: *const c_char = ptr::null();
            let chan = find_channel(chan_id);
            if !chan.is_null() {
                name = get_client_info(unsafe { &mut *chan }, c"name");
            }
            if !name.is_null() {
                semsg_multiline!(
                    c"rpc_error",
                    c"Invoking '%s' on channel %llu (%s):\n%s",
                    method,
                    chan_id,
                    name,
                    err.msg_ptr(),
                );
            } else {
                semsg_multiline!(
                    c"rpc_error",
                    c"Invoking '%s' on channel %llu:\n%s",
                    method,
                    chan_id,
                    err.msg_ptr(),
                );
            }
        } else {
            object_to_vim(result, &mut *rettv, &mut err);
        }

        arena_mem_free(res_mem);
        api_clear_error(&mut err);
    }
}

fn screenchar_adjust(grid: *mut *mut ScreenGrid, row: &mut i32, col: &mut i32) {
    unsafe {
        msg_scroll_flush();
        *grid = ui_comp_get_grid_at_coord(*row, *col);
        *row -= (**grid).comp_row;
        *col -= (**grid).comp_col;
    }
}

/// "screenattr()" function
pub fn f_screenattr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut row = (tv_get_number_chk(&argvars[0], None) as i32) - 1;
        let mut col = (tv_get_number_chk(&argvars[1], None) as i32) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &mut row, &mut col);

        let grid = unsafe { &*grid };
        let c = if row < 0 || row >= grid.rows || col < 0 || col >= grid.cols {
            -1
        } else {
            let line_off = grid.line_offset[row as usize];
            grid.attrs[line_off + (col as usize)]
        };
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(c));
    }
}

/// "screenchar()" function
pub fn f_screenchar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut row = (tv_get_number_chk(&argvars[0], None) as i32) - 1;
        let mut col = (tv_get_number_chk(&argvars[1], None) as i32) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &mut row, &mut col);

        rettv.vval = crate::eval::typval::TypvalVval::Number(
            if row < 0
                || row >= unsafe { (*grid).rows }
                || col < 0
                || col >= unsafe { (*grid).cols }
            {
                -1
            } else {
                Varnumber::from(schar_get_first_codepoint(grid_getchar(
                    unsafe { &*grid },
                    row,
                    col,
                    None,
                )))
            },
        );
    }
}

/// "screenchars()" function
pub fn f_screenchars(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut row = (tv_get_number_chk(&argvars[0], None) as i32) - 1;
        let mut col = (tv_get_number_chk(&argvars[1], None) as i32) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &mut row, &mut col);

        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        if row < 0 || row >= unsafe { (*grid).rows } || col < 0 || col >= unsafe { (*grid).cols } {
            return;
        }

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
        schar_get(
            buf.as_mut_ptr(),
            grid_getchar(unsafe { &*grid }, row, col, None),
        );

        let mut i: usize = 0;
        loop {
            let c = utf_ptr2char(buf.as_ptr().wrapping_add(i));
            tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(c));
            i += utf_ptr2len(buf.as_ptr().wrapping_add(i)) as usize;
            if buf[i] == NUL {
                break;
            }
        }
    }
}

/// "screencol()" function
pub fn f_screencol(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(ui_current_col() + 1));
}

/// "screenrow()" function
pub fn f_screenrow(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(ui_current_row() + 1));
}

/// "screenstring()" function
pub fn f_screenstring(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        rettv.v_type = VarType::VarString;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        let mut row = (tv_get_number_chk(&argvars[0], None) as i32) - 1;
        let mut col = (tv_get_number_chk(&argvars[1], None) as i32) - 1;

        screenchar_adjust(&raw mut grid, &mut row, &mut col);
        if row < 0 || row >= unsafe { (*grid).rows } || col < 0 || col >= unsafe { (*grid).cols } {
            return;
        }

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
        schar_get(
            buf.as_mut_ptr(),
            grid_getchar(unsafe { &*grid }, row, col, None),
        );
        crate::eval::typval::tv_set_string(rettv, buf.as_ptr());
    }
}

/// "search()" function
pub fn f_search(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut flags: i32 = 0;
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(search_cmn(
        argvars, None, &mut flags,
    )));
}

/// "searchdecl()" function
pub fn f_searchdecl(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut locally: i32 = 1;
        let mut thisblock: i32 = 0;
        let mut error = false;

        rettv.vval = crate::eval::typval::TypvalVval::Number(1);

        let name = tv_get_string_chk(&argvars[0]);
        if argvars[1].v_type != VarType::VarUnknown {
            locally = i32::from(tv_get_bool_chk(&argvars[1], Some(&mut error)) == 0);
            if !error && argvars[2].v_type != VarType::VarUnknown {
                thisblock = tv_get_bool_chk(&argvars[2], Some(&mut error)) as i32;
            }
        }
        if !error && !name.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(!find_decl(
                name.cast_mut(),
                crate::memory::cstr_len(name),
                locally != 0,
                thisblock != 0,
                SEARCH_KEEP,
            )));
        }
    }
}

fn searchpair_cmn(argvars: &mut [TypVal], match_pos: Option<&mut CursorPos>) -> i32 {
    {
        let save_p_ws = opt_vars().p_ws;
        let mut flags: i32 = 0;
        let mut retval: i32 = 0;
        let mut lnum_stop: Linenr = 0;
        let mut time_limit: i64 = 0;

        'theend: {
            let mut nbuf1 = [0 as c_char; NUMBUFLEN];
            let mut nbuf2 = [0 as c_char; NUMBUFLEN];
            let spat = tv_get_string_chk(&argvars[0]);
            let mpat = tv_get_string_buf_chk(&argvars[1], nbuf1.as_mut_ptr());
            let epat = tv_get_string_buf_chk(&argvars[2], nbuf2.as_mut_ptr());
            if spat.is_null() || mpat.is_null() || epat.is_null() {
                break 'theend;
            }

            let dir = get_search_arg(&mut argvars[3], Some(&mut flags));
            if dir == 0 {
                break 'theend;
            }

            if ((flags & (SP_END | SP_SUBPAT)) != 0)
                || ((flags & SP_NOMOVE) != 0 && (flags & SP_SETPCMARK) != 0)
            {
                semsg!(gettext(E_INVARG2), tv_get_string(&argvars[3]));
                break 'theend;
            }

            if (flags & SP_REPEAT) != 0 {
                opt_vars().p_ws = 0;
            }

            let skip: *const TypVal = if argvars[3].v_type == VarType::VarUnknown
                || argvars[4].v_type == VarType::VarUnknown
            {
                ptr::null()
            } else {
                let s = (&raw mut argvars[4]).cast_const();
                if argvars[5].v_type != VarType::VarUnknown {
                    lnum_stop = tv_get_number_chk(&argvars[5], None) as Linenr;
                    if lnum_stop < 0 {
                        semsg!(gettext(E_INVARG2), tv_get_string(&argvars[5]));
                        break 'theend;
                    }
                    if argvars[6].v_type != VarType::VarUnknown {
                        time_limit = tv_get_number_chk(&argvars[6], None);
                        if time_limit < 0 {
                            semsg!(gettext(E_INVARG2), tv_get_string(&argvars[6]));
                            break 'theend;
                        }
                    }
                }
                s
            };

            retval = do_searchpair(
                spat,
                mpat,
                epat,
                dir,
                unsafe { skip.as_ref() },
                flags,
                unsafe { (match_pos.map_or(ptr::null_mut(), |p| &raw mut *p)).as_mut() },
                lnum_stop,
                time_limit,
            );
        }

        opt_vars().p_ws = save_p_ws;
        retval
    }
}

/// "searchpair()" function
pub fn f_searchpair(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval =
        crate::eval::typval::TypvalVval::Number(Varnumber::from(searchpair_cmn(argvars, None)));
}

/// "searchpairpos()" function
pub fn f_searchpairpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut match_pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };
    let mut lnum: i32 = 0;
    let mut col: i32 = 0;

    tv_list_alloc_ret(rettv, 2);
    if searchpair_cmn(argvars, Some(&mut match_pos)) > 0 {
        lnum = match_pos.lnum;
        col = match_pos.col;
    }
    tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(lnum));
    tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(col));
}

/// Search for a start/middle/end thing.
pub fn do_searchpair(
    spat: impl crate::strings::IntoCCharPtr,
    mpat: impl crate::strings::IntoCCharPtr,
    epat: impl crate::strings::IntoCCharPtr,
    dir: i32,
    skip: Option<&TypVal>,
    flags: i32,
    match_pos: Option<&mut CursorPos>,
    lnum_stop: Linenr,
    time_limit: i64,
) -> i32 {
    let spat = spat.into_c_char_ptr();
    let mpat = mpat.into_c_char_ptr();
    let epat = epat.into_c_char_ptr();
    {
        let mut retval: i32 = 0;
        let mut nest: i32 = 1;
        let mut use_skip = false;
        let mut options = SEARCH_KEEP;

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        let tm = profile_setlimit(time_limit);

        let spatlen = crate::memory::cstr_len(spat);
        let epatlen = crate::memory::cstr_len(epat);
        let pat2size = spatlen + epatlen + 17;
        let mut pat2_vec = vec![0 as c_char; pat2size];
        let pat2 = pat2_vec.as_mut_ptr();
        let pat3size = spatlen + crate::memory::cstr_len(mpat) + epatlen + 25;
        let mut pat3_vec = vec![0 as c_char; pat3size];
        let pat3 = pat3_vec.as_mut_ptr();
        let pat2len = vim_snprintf!(pat2, pat2size, c"\\m\\(%s\\m\\)\\|\\(%s\\m\\)", spat, epat,);
        let pat3len = if crate::memory::cstr_is_empty(mpat) {
            unsafe { ptr::copy_nonoverlapping(pat2, pat3, (pat2len as usize) + 1) };
            pat2len
        } else {
            vim_snprintf!(
                pat3,
                pat3size,
                c"\\m\\(%s\\m\\)\\|\\(%s\\m\\)\\|\\(%s\\m\\)",
                spat,
                epat,
                mpat,
            )
        };
        if (flags & SP_START) != 0 {
            options |= SEARCH_START;
        }
        if let Some(skip) = skip {
            use_skip = eval_expr_valid_arg(skip);
        }

        let save_cursor = curwin_mut().w_cursor;
        let mut pos = CursorPos {
            lnum: save_cursor.lnum,
            col: save_cursor.col,
            coladd: save_cursor.coladd,
        };
        let mut firstpos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        clearpos(&mut firstpos);
        let mut foundpos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        clearpos(&mut foundpos);
        let mut pat = pat3;
        debug_assert!(pat3len >= 0);
        let patlen = pat3len as usize;

        loop {
            let mut sia = SearchitArg {
                sa_stop_lnum: lnum_stop,
                sa_tm: Some(tm),
                sa_timed_out: 0,
                sa_wrapped: 0,
            };

            let n = searchit(
                curwin_ptr(),
                curbuf_mut(),
                unsafe { &mut *(pos.cast()) },
                None,
                dir,
                pat,
                patlen,
                1,
                options,
                RE_SEARCH,
                true,
                Some(&mut sia),
            );
            if n == FAIL || (firstpos.lnum != 0 && equalpos(pos, firstpos)) {
                break;
            }
            if firstpos.lnum == 0 {
                firstpos = pos;
            }
            if equalpos(pos, foundpos) {
                if dir == BACKWARD {
                    decl(&mut pos);
                } else {
                    incl(&mut pos);
                }
            }
            foundpos = pos;

            options &= !SEARCH_START;

            if use_skip {
                let save_pos = curwin_mut().w_cursor;
                curwin_mut().w_cursor.lnum = pos.lnum;
                curwin_mut().w_cursor.col = pos.col;
                curwin_mut().w_cursor.coladd = pos.coladd;
                let mut err = false;
                let r = eval_expr_to_bool(skip.unwrap(), &mut err);
                curwin_mut().w_cursor = save_pos;
                if err {
                    curwin_mut().w_cursor = save_cursor;
                    retval = -1;
                    break;
                }
                if r {
                    continue;
                }
            }

            if (dir == BACKWARD && n == 3) || (dir == FORWARD && n == 2) {
                nest += 1;
                pat = pat2;
            } else {
                nest -= 1;
                if nest == 1 {
                    pat = pat3;
                }
            }

            if nest == 0 {
                if (flags & SP_RETCOUNT) != 0 {
                    retval += 1;
                } else {
                    retval = pos.lnum;
                }
                if (flags & SP_SETPCMARK) != 0 {
                    setpcmark();
                }
                curwin_mut().w_cursor.lnum = pos.lnum;
                curwin_mut().w_cursor.col = pos.col;
                curwin_mut().w_cursor.coladd = pos.coladd;
                if (flags & SP_REPEAT) == 0 {
                    break;
                }
                nest = 1;
            }
        }

        if let Some(match_pos) = match_pos {
            match_pos.lnum = curwin_mut().w_cursor.lnum;
            match_pos.col = curwin_mut().w_cursor.col + 1;
        }

        if (flags & SP_NOMOVE) != 0 || retval == 0 {
            curwin_mut().w_cursor = save_cursor;
        }

        drop(pat2_vec);
        drop(pat3_vec);
        if opt_vars().p_cpo == EMPTY_STRING_OPTION.as_ptr().cast() {
            opt_vars().p_cpo = save_cpo;
        } else {
            if crate::memory::cstr_is_empty(opt_vars().p_cpo) {
                set_option_value_give_err(
                    K_OPT_CPOPTIONS,
                    Object {
                        obj_type: ObjectType::ObjectTypeString,
                        data: ObjectData::String(cstr_as_string(save_cpo)),
                    },
                    0,
                );
            }
            free_string_option(save_cpo);
        }

        retval
    }
}

/// "searchpos()" function
pub fn f_searchpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut match_pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };
    let mut flags: i32 = 0;

    let n = search_cmn(argvars, Some(&mut match_pos), &mut flags);
    tv_list_alloc_ret(rettv, 2 + isize::from((flags & SP_SUBPAT) != 0));

    let lnum = if n > 0 { match_pos.lnum } else { 0 };
    let col = if n > 0 { match_pos.col } else { 0 };

    tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(lnum));
    tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(col));
    if (flags & SP_SUBPAT) != 0 {
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(n));
    }
}

/// "serverlist()" function
pub fn f_serverlist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let addrs = server_address_list();

        let mut addrs_tv = tv_initial_value();
        tv_list_alloc_ret(&mut addrs_tv, addrs.len() as isize);
        for addr in addrs {
            tv_list_append_string(unsafe { &mut *addrs_tv.vval.v_list() }, addr.as_ptr(), -1);
        }

        let opts = if argvars[0].v_type != VarType::VarUnknown {
            argvars[0]
        } else {
            TypVal {
                v_type: VarType::VarSpecial,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST),
            }
        };
        let mut lua_args = [opts, addrs_tv, tv_initial_value()];
        nlua_call_typval(
            c"vim._core.server",
            c"serverlist",
            lua_args.as_mut_ptr(),
            Some(&mut *rettv),
        );
        tv_clear(&mut addrs_tv);
    }
}

/// "serverstart()" function
pub fn f_serverstart(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if check_secure() {
            return;
        }

        let address = if argvars[0].v_type != VarType::VarUnknown {
            if argvars[0].v_type != VarType::VarString {
                emsg(gettext(E_INVARG));
                return;
            }
            to_slash_save(tv_get_string(&argvars[0])).unwrap_or_default()
        } else {
            server_address_new(ptr::null())
        };

        let result = server_start(address.as_ptr());
        drop(address);

        if result != 0 {
            semsg!(
                c"Failed to start server: %s",
                if result > 0 {
                    c"Unknown system error".as_ptr()
                } else {
                    unsafe { uv_strerror(result) }
                },
            );
            return;
        }

        let mut addrs = server_address_list();
        if let Some(last) = addrs.pop() {
            crate::eval::typval::tv_set_string(rettv, last.as_ptr());
        }
    }
}

/// "serverstop()" function
pub fn f_serverstop(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if check_secure() {
            return;
        }
        if argvars[0].v_type != VarType::VarString {
            emsg(gettext(E_INVARG));
            return;
        }
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        if !argvars[0].vval.v_string().is_null() {
            let rv = server_stop(argvars[0].vval.v_string(), false);
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(rv));
        }
    }
}

fn set_position(argvars: &mut [TypVal], rettv: &mut TypVal, charpos: bool) {
    {
        let mut curswant: Colnr = -1;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        let name = tv_get_string_chk(&argvars[0]);
        if name.is_null() {
            return;
        }

        let mut pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut fnum: i32 = 0;
        if !list2fpos(
            &mut argvars[1],
            &mut pos,
            Some(&mut fnum),
            Some(&mut curswant),
            charpos,
        ) {
            return;
        }

        if pos.col != MAXCOL {
            pos.col -= 1;
            if pos.col < 0 {
                pos.col = 0;
            }
        }
        if crate::memory::cstr_first_byte(name) == b'.' && crate::memory::cstr_byte_at(name, 1) == 0
        {
            curwin_mut().w_cursor.lnum = pos.lnum;
            curwin_mut().w_cursor.col = pos.col;
            curwin_mut().w_cursor.coladd = pos.coladd;
            if curswant >= 0 {
                curwin_mut().w_curswant = curswant - 1;
                curwin_mut().w_set_curswant = false;
            }
            check_cursor(curwin_mut());
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        } else if crate::memory::cstr_first_byte(name) == b'\''
            && crate::memory::cstr_byte_at(name, 1) != 0
            && crate::memory::cstr_byte_at(name, 2) == 0
        {
            if setmark_pos(
                i32::from(crate::memory::cstr_byte_at(name, 1)),
                &mut pos,
                fnum,
                None,
            ) {
                rettv.vval = crate::eval::typval::TypvalVval::Number(0);
            }
        } else {
            emsg(gettext(E_INVARG));
        }
    }
}

/// "setcharpos()" function
pub fn f_setcharpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    set_position(argvars, rettv, true);
}

pub fn f_setcharsearch(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if !tv_check_for_dict_arg(argvars, 0) {
            return;
        }
        let d = argvars[0].vval.v_dict();
        if d.is_null() {
            return;
        }

        let csearch = tv_dict_get_string(unsafe { d.as_ref() }, c"char");
        if !csearch.is_null() {
            let c = utf_ptr2char(csearch);
            set_last_csearch(c, csearch, utfc_ptr2len(csearch));
        }

        let mut di = tv_dict_find(unsafe { d.as_ref() }, c"forward", 7);
        if !di.is_null() {
            set_csearch_direction(if tv_get_number(unsafe { &(*di).di_tv }) != 0 {
                FORWARD
            } else {
                BACKWARD
            });
        }

        di = tv_dict_find(unsafe { d.as_ref() }, c"until", 5);
        if !di.is_null() {
            set_csearch_until(i32::from(tv_get_number(unsafe { &(*di).di_tv }) != 0));
        }
    }
}

/// "setcursorcharpos()" function
pub fn f_setcursorcharpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    set_cursorpos(argvars, rettv, true);
}

/// "setenv()" function
pub fn f_setenv(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut namebuf = [0 as c_char; NUMBUFLEN];
        let mut valbuf = [0 as c_char; NUMBUFLEN];
        let name = tv_get_string_buf(&argvars[0], namebuf.as_mut_ptr());

        if check_secure() {
            return;
        }
        if argvars[1].v_type == VarType::VarSpecial
            && argvars[1].vval.v_special() == K_SPECIAL_VAR_NULL_CONST
        {
            vim_unsetenv_ext(name);
        } else {
            vim_setenv_ext(name, tv_get_string_buf(&argvars[1], valbuf.as_mut_ptr()));
        }
    }
}

/// "setfperm({fname}, {mode})" function
pub fn f_setfperm(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        let fname = tv_get_string_chk(&argvars[0]);
        if fname.is_null() {
            return;
        }
        let mut modebuf = [0 as c_char; NUMBUFLEN];
        let mode_str = tv_get_string_buf_chk(&argvars[1], modebuf.as_mut_ptr());
        if mode_str.is_null() {
            return;
        }
        if crate::memory::cstr_len(mode_str) != 9 {
            semsg!(gettext(E_INVARG2), mode_str);
            return;
        }

        let mut mask: i32 = 1;
        let mut mode: i32 = 0;
        for i in (0..9).rev() {
            if crate::memory::cstr_byte_at(mode_str, i) != b'-' {
                mode |= mask;
            }
            mask <<= 1;
        }
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(Varnumber::from(os_setperm(fname, mode)));
    }
}

/// "setpos()" function
pub fn f_setpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    set_position(argvars, rettv, false);
}

fn get_yank_type(
    pp: *mut *mut c_char,
    yank_type: &mut crate::register::MotionType,
    block_len: &mut i32,
) -> bool {
    {
        let mut stropt = unsafe { *pp };
        match unsafe { *stropt } {
            c if c == b'v' as c_char || c == b'c' as c_char => {
                *yank_type = crate::register::MotionType::MtCharWise;
            }
            c if c == b'V' as c_char || c == b'l' as c_char => {
                *yank_type = crate::register::MotionType::MtLineWise;
            }
            c if c == b'b' as c_char || c == CTRL_V_CONST => {
                *yank_type = crate::register::MotionType::MtBlockWise;
                if ascii_isdigit(crate::memory::cstr_char_at(stropt, 1)) {
                    stropt = stropt.wrapping_add(1);
                    *block_len = getdigits_int(&raw mut stropt, false, 0) - 1;
                    stropt = stropt.wrapping_sub(1);
                }
            }
            _ => return false,
        }
        unsafe {
            *pp = stropt;
        }
        true
    }
}

/// "setreg()" function
pub fn f_setreg(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut append = false;
        let mut block_len: i32 = -1;
        let mut yank_type = crate::register::MotionType::MtUnknown;

        rettv.vval = crate::eval::typval::TypvalVval::Number(1);

        let strregname = tv_get_string_chk(&argvars[0]);
        if strregname.is_null() {
            return;
        }
        let mut regname = *strregname;
        if regname == 0 || regname == b'@' as c_char {
            regname = b'"' as c_char;
        }

        let mut regcontents: *const TypVal = ptr::null();
        let mut pointreg: c_char = 0;
        if argvars[1].v_type == VarType::VarDict {
            let d = argvars[1].vval.v_dict();
            if tv_dict_len(d.as_ref()) == 0 {
                let mut lstval: [*mut c_char; 2] = [ptr::null_mut(), ptr::null_mut()];
                write_reg_contents_lst(
                    i32::from(regname as u8),
                    lstval.as_mut_ptr(),
                    false,
                    crate::register::MotionType::MtUnknown,
                    -1,
                );
                return;
            }

            let di = tv_dict_find(d.as_ref(), c"regcontents", -1);
            if !di.is_null() {
                regcontents = &raw const (*di).di_tv;
            }

            let mut stropt = tv_dict_get_string(d.as_ref(), c"regtype");
            if !stropt.is_null() {
                let ret = get_yank_type(&raw mut stropt, &mut yank_type, &mut block_len);
                stropt = stropt.wrapping_add(1);
                if !ret || !crate::memory::cstr_is_empty(stropt) {
                    semsg!(gettext(E_INVARGVAL), c"value");
                    return;
                }
            }

            if regname == b'"' as c_char {
                stropt = tv_dict_get_string(d.as_ref(), c"points_to");
                if !stropt.is_null() {
                    pointreg = *stropt;
                    regname = pointreg;
                }
            } else if tv_dict_get_bool(d.as_ref(), c"isunnamed", -1) > 0 {
                pointreg = regname;
            }
        } else {
            regcontents = (&raw mut argvars[1]);
        }

        let mut set_unnamed = false;
        if argvars[2].v_type != VarType::VarUnknown {
            if yank_type != crate::register::MotionType::MtUnknown {
                semsg!(gettext(E_TOOMANYARG), c"setreg");
                return;
            }

            let mut stropt = tv_get_string_chk(&argvars[2]).cast_mut();
            if stropt.is_null() {
                return;
            }
            while !crate::memory::cstr_is_empty(stropt) {
                match crate::memory::cstr_first_byte(stropt) {
                    b'a' | b'A' => append = true,
                    b'u' | b'"' => set_unnamed = true,
                    _ => {
                        get_yank_type(&raw mut stropt, &mut yank_type, &mut block_len);
                    }
                }
                stropt = stropt.wrapping_add(1);
            }
        }

        if !regcontents.is_null() && (*regcontents).v_type == VarType::VarList {
            let ll = (*regcontents).vval.v_list();
            let len = tv_list_len(ll.as_ref()) as usize;

            let mut lstval: Vec<*const c_char> = Vec::with_capacity(len + 1);
            let mut allocval: Vec<std::ffi::CString> = Vec::new();

            'free_lstval: {
                for li in crate::eval::typval::list_iter_mut(tv_list_first(ll.as_ref())) {
                    let mut buf = [0 as c_char; NUMBUFLEN];
                    let mut val = tv_get_string_buf_chk(&li.li_tv, buf.as_mut_ptr());
                    if val.is_null() {
                        break 'free_lstval;
                    }
                    if ptr::eq(val, buf.as_ptr()) {
                        let copied = cstr_to_cstring(val);
                        val = copied.as_ptr();
                        allocval.push(copied);
                    }
                    lstval.push(val);
                }
                lstval.push(ptr::null());

                write_reg_contents_lst(
                    i32::from(regname as u8),
                    lstval.as_mut_ptr().cast::<*mut c_char>(),
                    append,
                    yank_type,
                    block_len as Colnr,
                );
            }

            drop(allocval);
        } else if !regcontents.is_null() {
            let strval = tv_get_string_chk(&*regcontents);
            if strval.is_null() {
                return;
            }
            write_reg_contents_ex(
                i32::from(regname as u8),
                strval,
                crate::memory::cstr_len(strval) as isize,
                append,
                yank_type,
                block_len as Colnr,
            );
        }
        if pointreg != 0 {
            get_yank_register(i32::from(pointreg as u8), YREG_YANK);
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        if set_unnamed {
            op_reg_set_previous(regname);
        }
    }
}

/// "settagstack()" function
pub fn f_settagstack(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut action: c_char = b'r' as c_char;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let wp = find_win_by_nr_or_id(&argvars[0]);
        if wp.is_null() {
            return;
        }
        if !tv_check_for_dict_arg(argvars, 1) {
            return;
        }
        let d = argvars[1].vval.v_dict();
        if d.is_null() {
            return;
        }

        if argvars[2].v_type == VarType::VarUnknown {
            // default 'r'
        } else if !tv_check_for_string_arg(argvars, 2) {
            return;
        } else {
            let actstr = tv_get_string_chk(&argvars[2]);
            if actstr.is_null() {
                return;
            }
            if (crate::memory::cstr_first_byte(actstr) == b'r'
                || crate::memory::cstr_first_byte(actstr) == b'a'
                || crate::memory::cstr_first_byte(actstr) == b't')
                && crate::memory::cstr_byte_at(actstr, 1) == 0
            {
                action = unsafe { *actstr };
            } else {
                semsg!(gettext(E_INVACT2), actstr);
                return;
            }
        }

        if set_tagstack(
            unsafe { &mut *wp },
            unsafe { d.as_ref() },
            i32::from(action),
        ) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
    }
}

/// "sha256({expr})" function
pub fn f_sha256(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if argvars[0].v_type == VarType::VarBlob {
            let blob = argvars[0].vval.v_blob().cast::<TvBlob>();
            let slice: &[u8] = if !blob.is_null() {
                unsafe { &(*blob).bv_ga }
            } else {
                &[]
            };
            let hex = sha256_hex(slice, None);
            crate::eval::typval::tv_set_string_len(rettv, hex.as_ptr().cast(), hex.len());
        } else {
            let p = tv_get_string(&argvars[0]);
            let len = crate::memory::cstr_len(p);
            let slice = if len > 0 {
                crate::memory::cstr_slice(p, len)
            } else {
                &[]
            };
            let hex = sha256_hex(slice, None);
            crate::eval::typval::tv_set_string_len(rettv, hex.as_ptr().cast(), hex.len());
        }
    }
}

/// "shellescape({string})" function
pub fn f_shellescape(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let do_special = non_zero_arg(&argvars[1]);
        crate::eval::typval::tv_set_string(
            rettv,
            vim_strsave_shellescape(tv_get_string(&argvars[0]), do_special, do_special).as_ptr(),
        );
        rettv.v_type = VarType::VarString;
    }
}

/// "shiftwidth()" function
pub fn f_shiftwidth(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        if argvars[0].v_type != VarType::VarUnknown {
            let col = tv_get_number_chk(&argvars[0], None) as Colnr;
            if col < 0 {
                return;
            }
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
                get_sw_value_col(curbuf_mut(), col, false),
            ));
            return;
        }
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(Varnumber::from(get_sw_value(curbuf_mut())));
    }
}

/// "sockconnect()" function
pub fn f_sockconnect(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarString || argvars[1].v_type != VarType::VarString {
            emsg(gettext(E_INVARG));
            return;
        }
        if argvars[2].v_type != VarType::VarDict && argvars[2].v_type != VarType::VarUnknown {
            semsg!(gettext(E_INVARG2), c"expected dictionary");
            return;
        }

        let mode = tv_get_string(&argvars[0]);
        let address = to_slash_save(tv_get_string(&argvars[1])).unwrap_or_default();

        'cleanup: {
            let tcp = if crate::memory::cstr_as_bytes(mode) == b"tcp" {
                true
            } else if crate::memory::cstr_as_bytes(mode) == b"pipe" {
                false
            } else {
                semsg!(gettext(E_INVARG2), c"invalid mode");
                break 'cleanup;
            };

            let mut rpc = false;
            let mut on_data = CallbackReader::none();
            if argvars[2].v_type == VarType::VarDict {
                let opts = argvars[2].vval.v_dict();
                rpc = tv_dict_get_number(unsafe { opts.as_ref() }, c"rpc") != 0;

                if !tv_dict_get_callback(unsafe { opts.as_mut() }, c"on_data", 7, &mut on_data.cb) {
                    break 'cleanup;
                }
                on_data.buffered =
                    tv_dict_get_number(unsafe { opts.as_ref() }, c"data_buffered") != 0;
                if on_data.buffered && on_data.cb.cb_type() == K_CALLBACK_NONE_CONST {
                    on_data.self_dict = opts;
                }
            }

            let mut error: *const c_char = ptr::null();
            let id = channel_connect(tcp, address.as_ptr(), rpc, on_data, 50, &mut error);
            if !error.is_null() {
                semsg!(gettext(c"connection failed: %s"), error);
            }

            rettv.vval = crate::eval::typval::TypvalVval::Number(id as Varnumber);
            rettv.v_type = VarType::VarNumber;
        }

        drop(address);
    }
}

/// "stdioopen()" function
pub fn f_stdioopen(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarDict {
            emsg(gettext(E_INVARG));
            return;
        }

        let mut on_stdin = CallbackReader::none();
        let opts = argvars[0].vval.v_dict();
        let rpc = tv_dict_get_number(unsafe { opts.as_ref() }, c"rpc") != 0;

        if !tv_dict_get_callback(unsafe { opts.as_mut() }, c"on_stdin", 8, &mut on_stdin.cb) {
            return;
        }
        if !tv_dict_get_callback(
            unsafe { opts.as_mut() },
            c"on_print",
            8,
            &mut g_st().on_print,
        ) {
            return;
        }

        on_stdin.buffered = tv_dict_get_number(unsafe { opts.as_ref() }, c"stdin_buffered") != 0;
        if on_stdin.buffered && on_stdin.cb.cb_type() == K_CALLBACK_NONE_CONST {
            on_stdin.self_dict = opts;
        }

        let mut error: *const c_char = ptr::null();
        let id = channel_from_stdio(rpc, on_stdin, &mut error);
        if id == 0 {
            semsg!(E_STDIOCHAN2, error);
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(id as Varnumber);
        rettv.v_type = VarType::VarNumber;
    }
}

/// "reltimefloat()" function
pub fn f_reltimefloat(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut tm: Proftime = 0;
    rettv.v_type = VarType::VarFloat;
    rettv.vval = crate::eval::typval::TypvalVval::Float(0.0);
    if list2proftime(&mut argvars[0], &mut tm) {
        rettv.vval = crate::eval::typval::TypvalVval::Float(
            (profile_signed(tm) as TvFloat) / (NS_PER_SEC as TvFloat),
        );
    }
}

/// "soundfold({word})" function
pub fn f_soundfold(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    let s = tv_get_string(&argvars[0]);
    crate::eval::typval::tv_set_string(rettv, eval_soundfold(s).as_ptr());
}

/// "spellbadword()" function
pub fn f_spellbadword(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let wo_spell_save = curwin_mut().w_onebuf_opt.wo_spell;

        if curwin_mut().w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin_mut());
            curwin_mut().w_onebuf_opt.wo_spell = 1;
        }

        let ws = curwin_mut().w_s_ptr();
        if crate::memory::cstr_is_empty(ws.b_p_spl) {
            emsg(gettext(E_NO_SPELL));
            curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
            return;
        }

        let mut word: *const c_char = c"".as_ptr();
        let mut attr: Hlf = HLF_COUNT;
        let mut len: usize = 0;
        if argvars[0].v_type == VarType::VarUnknown {
            len = spell_move_to(curwin_mut(), FORWARD, SMT_ALL, true, &raw mut attr);
            if len != 0 {
                word = get_cursor_pos_ptr();
                curwin_mut().w_set_curswant = true;
            }
        } else if !crate::memory::cstr_is_empty(curbuf_mut().b_s.b_p_spl) {
            let mut str_ = tv_get_string_chk(&argvars[0]);
            let mut capcol: i32 = -1;
            if !str_.is_null() {
                while !crate::memory::cstr_is_empty(str_) {
                    len = spell_check(
                        curwin_mut(),
                        str_.cast_mut(),
                        &mut attr,
                        Some(&mut capcol),
                        false,
                    );
                    if attr != HLF_COUNT {
                        word = str_;
                        break;
                    }
                    str_ = str_.wrapping_add(len);
                    capcol -= len as i32;
                    len = 0;
                }
            }
        }
        curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;

        debug_assert!(len <= i32::MAX as usize);
        tv_list_alloc_ret(rettv, 2);
        tv_list_append_string(rettv.vval.v_list_ref(), word, len as isize);
        match attr {
            HLF_SPB => tv_list_append_string(rettv.vval.v_list_ref(), c"bad", 3),
            HLF_SPR => tv_list_append_string(rettv.vval.v_list_ref(), c"rare", 4),
            HLF_SPL => tv_list_append_string(rettv.vval.v_list_ref(), c"local", 5),
            HLF_SPC => tv_list_append_string(rettv.vval.v_list_ref(), c"caps", 4),
            _ => tv_list_append_string(rettv.vval.v_list_ref(), ptr::null(), -1),
        }
    }
}

/// "spellsuggest()" function
pub fn f_spellsuggest(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut ga: Vec<std::ffi::CString> = Vec::new();
        let wo_spell_save = curwin_mut().w_onebuf_opt.wo_spell;

        if curwin_mut().w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin_mut());
            curwin_mut().w_onebuf_opt.wo_spell = 1;
        }

        let ws = curwin_mut().w_s_ptr();
        if crate::memory::cstr_is_empty(ws.b_p_spl) {
            emsg(gettext(E_NO_SPELL));
            curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
            return;
        }

        'ret: {
            let maxcount: i32;
            let mut need_capital = false;
            let str_ = tv_get_string(&argvars[0]);
            if argvars[1].v_type != VarType::VarUnknown {
                let mut typeerr = false;
                maxcount = tv_get_number_chk(&argvars[1], Some(&mut typeerr)) as i32;
                if maxcount <= 0 {
                    break 'ret;
                }
                if argvars[2].v_type != VarType::VarUnknown {
                    need_capital = tv_get_bool_chk(&argvars[2], Some(&mut typeerr)) != 0;
                    if typeerr {
                        break 'ret;
                    }
                }
            } else {
                maxcount = 25;
            }

            ga = spell_suggest_list(str_.cast_mut(), maxcount, need_capital, false);
        }

        tv_list_alloc_ret(rettv, ga.len() as isize);
        for s in ga {
            tv_list_append_string(
                rettv.vval.v_list_ref(),
                s.as_ptr(),
                s.as_bytes().len() as isize,
            );
        }
        curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
    }
}

pub fn f_split(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut col: Colnr = 0;
        let mut keepempty = false;
        let mut typeerr = false;

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        let mut str_ = tv_get_string(&argvars[0]);
        let mut pat: *const c_char = ptr::null();
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        if argvars[1].v_type != VarType::VarUnknown {
            pat = tv_get_string_buf_chk(&argvars[1], patbuf.as_mut_ptr());
            if pat.is_null() {
                typeerr = true;
            }
            if argvars[2].v_type != VarType::VarUnknown {
                keepempty = tv_get_bool_chk(&argvars[2], Some(&mut typeerr)) != 0;
            }
        }
        if crate::memory::cstr_is_empty(pat) {
            pat = c"[\\x01- ]\\+".as_ptr();
        }

        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

        if !typeerr {
            let mut regmatch = Regmatch {
                regprog: vim_regcomp(pat, RE_MAGIC + RE_STRING),
                startp: [ptr::null_mut(); NSUBEXP],
                endp: [ptr::null_mut(); NSUBEXP],
                rm_matchcol: 0,
                rm_ic: false,
            };
            if !regmatch.regprog.is_null() {
                while !crate::memory::cstr_is_empty(str_) || keepempty {
                    let match_found = if crate::memory::cstr_is_empty(str_) {
                        false
                    } else {
                        vim_regexec_nl(&mut regmatch, str_, col)
                    };
                    let end: *const c_char = if match_found {
                        regmatch.startp[0]
                    } else {
                        str_.wrapping_add(crate::memory::cstr_len(str_))
                    };
                    if keepempty
                        || end > str_
                        || (tv_list_len(unsafe { rettv.vval.v_list().as_ref() }) > 0
                            && !crate::memory::cstr_is_empty(str_)
                            && match_found
                            && end < regmatch.endp[0].cast_const())
                    {
                        tv_list_append_string(
                            rettv.vval.v_list_ref(),
                            str_,
                            crate::memory::ptr_offset_from(end, str_),
                        );
                    }
                    if !match_found {
                        break;
                    }
                    if regmatch.endp[0].cast_const() > str_ {
                        col = 0;
                    } else {
                        col = utfc_ptr2len(regmatch.endp[0]);
                    }
                    str_ = regmatch.endp[0];
                }
                vim_regfree(regmatch.regprog);
            }
        }

        opt_vars().p_cpo = save_cpo;
    }
}

fn get_xdg_var_list(xdg: XDGVarType, rettv: &mut TypVal) {
    {
        let list = tv_list_alloc(K_LIST_LEN_SHOULD_KNOW);
        rettv.v_type = VarType::VarList;
        rettv.vval = crate::eval::typval::TypvalVval::List(list);
        tv_list_ref(Some(&mut *list));
        let Some(dirs) = stdpaths_get_xdg_var(xdg) else {
            return;
        };
        let mut iter: *const c_char = ptr::null();
        let appname = get_appname(false);
        loop {
            let mut dir_len: usize = 0;
            let mut dir: *const c_char = ptr::null();
            iter = vim_env_iter(ENV_SEPCHAR, dirs.as_ptr(), iter, &raw mut dir, &mut dir_len);
            if !dir.is_null() && dir_len > 0 {
                let dir_cstr = cstr_from_bytes(crate::memory::cstr_slice(dir, dir_len));
                let dir_with_nvim = concat_fnames_cstr(dir_cstr.as_ptr(), appname, true);
                tv_list_append_string(&mut *list, dir_with_nvim.as_ptr(), -1);
            }
            if iter.is_null() {
                break;
            }
        }
    }
}

/// "stdpath(type)" function
pub fn f_stdpath(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    let p = tv_get_string_chk(&argvars[0]);
    if p.is_null() {
        return;
    }

    if crate::memory::cstr_as_opt_bytes(p) == Some(b"config") {
        if let Some(s) = get_xdg_home(XDGVarType::XDGConfigHome) {
            crate::eval::typval::tv_set_string(rettv, s.as_ptr());
        }
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"data") {
        if let Some(s) = get_xdg_home(XDGVarType::XDGDataHome) {
            crate::eval::typval::tv_set_string(rettv, s.as_ptr());
        }
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"cache") {
        if let Some(s) = get_xdg_home(XDGVarType::XDGCacheHome) {
            crate::eval::typval::tv_set_string(rettv, s.as_ptr());
        }
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"state") {
        if let Some(s) = get_xdg_home(XDGVarType::XDGStateHome) {
            crate::eval::typval::tv_set_string(rettv, s.as_ptr());
        }
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"log") {
        if let Some(state_home) = get_xdg_home(XDGVarType::XDGStateHome) {
            let log_dir = concat_fnames_cstr(state_home.as_ptr(), c"logs", true);
            crate::eval::typval::tv_set_string(rettv, log_dir.as_ptr());
        }
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"run") {
        if let Some(s) = stdpaths_get_xdg_var(XDGVarType::XDGRuntimeDir) {
            crate::eval::typval::tv_set_string(rettv, s.as_ptr());
        }
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"config_dirs") {
        get_xdg_var_list(XDGVarType::XDGConfigDirs, rettv);
    } else if crate::memory::cstr_as_opt_bytes(p) == Some(b"data_dirs") {
        get_xdg_var_list(XDGVarType::XDGDataDirs, rettv);
    } else {
        semsg!(gettext(c"E6100: \"%s\" is not a valid stdpath"), p);
    }
}

/// "str2float()" function
pub fn f_str2float(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut p = skipwhite(tv_get_string(&argvars[0]));
        let isneg = crate::memory::cstr_first_byte(p) == b'-';
        if crate::memory::cstr_first_byte(p) == b'+' || crate::memory::cstr_first_byte(p) == b'-' {
            p = skipwhite(p.wrapping_add(1));
        }
        string2float(p, rettv.vval.v_float_mut());
        if isneg {
            *rettv.vval.v_float_mut() *= -1.0;
        }
        rettv.v_type = VarType::VarFloat;
    }
}

/// "strftime({format}[, {time}])" function
pub fn f_strftime(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        rettv.v_type = VarType::VarString;
        let mut p = tv_get_string(&argvars[0]).cast_mut();
        let seconds: time_t = if argvars[1].v_type == VarType::VarUnknown {
            os_time() as time_t
        } else {
            tv_get_number(&argvars[1]) as time_t
        };

        let mut curtime = Tm { _fields: [0; 16] };
        let curtime_ptr = os_localtime_r(&seconds, &raw mut curtime);
        if curtime_ptr.is_null() {
            crate::eval::typval::tv_set_string(rettv, gettext(c"(Invalid)"));
            return;
        }

        let mut conv = Vimconv::new();
        conv.vc_type = CONV_NONE;
        let enc = enc_locale();
        let enc_ptr = enc
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        convert_setup(&mut conv, opt_vars().p_enc, enc_ptr);
        let p_owned: Option<Vec<u8>>;
        if conv.vc_type != CONV_NONE {
            p_owned = string_convert(&conv, p, None);
            p = p_owned
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast::<c_char>().cast_mut());
        } else {
            p_owned = None;
        }
        let mut result_buf = [0 as c_char; 256];
        if p.is_null()
            || unsafe { strftime(result_buf.as_mut_ptr(), result_buf.len(), p, curtime_ptr) } == 0
        {
            result_buf[0] = NUL;
        }
        drop(p_owned);

        convert_setup(&mut conv, enc_ptr, opt_vars().p_enc);
        if conv.vc_type != CONV_NONE {
            if let Some(conv_bytes) = string_convert(&conv, result_buf.as_mut_ptr(), None) {
                crate::eval::typval::tv_set_string(rettv, conv_bytes.as_ptr().cast());
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
        } else {
            crate::eval::typval::tv_set_string(rettv, result_buf.as_ptr());
        }
    }
}

/// "strptime({format}, {timestring})" function
pub fn f_strptime(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut fmt_buf = [0 as c_char; NUMBUFLEN];
        let mut str_buf = [0 as c_char; NUMBUFLEN];

        let mut tmval = Tm { _fields: [0; 16] };
        tmval._fields[8] = -1; // tm_isdst = -1
        let mut fmt = tv_get_string_buf(&argvars[0], fmt_buf.as_mut_ptr()).cast_mut();
        let str_ = tv_get_string_buf(&argvars[1], str_buf.as_mut_ptr());

        let mut conv = Vimconv::new();
        conv.vc_type = CONV_NONE;
        let enc = enc_locale();
        let enc_ptr = enc
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        convert_setup(&mut conv, opt_vars().p_enc, enc_ptr);
        let fmt_owned: Option<Vec<u8>>;
        if conv.vc_type != CONV_NONE {
            fmt_owned = string_convert(&conv, fmt, None);
            fmt = fmt_owned
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast::<c_char>().cast_mut());
        } else {
            fmt_owned = None;
        }
        if fmt.is_null() || os_strptime(str_, fmt, &raw mut tmval).is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        } else {
            let t = unsafe { mktime(&raw mut tmval) };
            rettv.vval =
                crate::eval::typval::TypvalVval::Number(if t == -1 { 0 } else { t as Varnumber });
        }
        drop(fmt_owned);
    }
}

/// "submatch()" function
pub fn f_submatch(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut error = false;
        let no = tv_get_number_chk(&argvars[0], Some(&mut error)) as i32;
        if error {
            return;
        }
        if no < 0 || (no as usize) >= NSUBEXP {
            semsg!(gettext(E_INVALID_SUBMATCH_NUMBER_NR), no);
            return;
        }
        let mut ret_list: i32 = 0;
        if argvars[1].v_type != VarType::VarUnknown {
            ret_list = tv_get_bool_chk(&argvars[1], Some(&mut error)) as i32;
            if error {
                return;
            }
        }
        if ret_list == 0 {
            crate::eval::typval::tv_set_string(
                rettv,
                reg_submatch(no)
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr()),
            );
        } else {
            rettv.v_type = VarType::VarList;
            rettv.vval = crate::eval::typval::TypvalVval::List(reg_submatch_list(no));
        }
    }
}

/// "substitute()" function
pub fn f_substitute(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let mut subbuf = [0 as c_char; NUMBUFLEN];
        let mut flagsbuf = [0 as c_char; NUMBUFLEN];

        let str_ = tv_get_string_chk(&argvars[0]);
        let pat = tv_get_string_buf_chk(&argvars[1], patbuf.as_mut_ptr());
        let mut sub: *const c_char = ptr::null();
        let flg = tv_get_string_buf_chk(&argvars[3], flagsbuf.as_mut_ptr());

        let mut expr: *mut TypVal = ptr::null_mut();
        if tv_is_func(&argvars[2]) {
            expr = &raw mut argvars[2];
        } else {
            sub = tv_get_string_buf_chk(&argvars[2], subbuf.as_mut_ptr());
        }

        rettv.v_type = VarType::VarString;
        if str_.is_null() || pat.is_null() || (sub.is_null() && expr.is_null()) || flg.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        } else {
            let sub_res = do_string_sub(
                str_.cast_mut(),
                crate::memory::cstr_len(str_),
                pat.cast_mut(),
                sub.cast_mut(),
                unsafe { expr.as_ref() },
                flg.cast_mut(),
            );
            crate::eval::typval::tv_set_string_len(
                rettv,
                sub_res.as_ptr(),
                sub_res.as_bytes().len(),
            );
        }
    }
}

/// "swapfilelist()" function
pub fn f_swapfilelist(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);
    recover_names(ptr::null_mut(), false, rettv.vval.v_list_ref());
}

/// "swapinfo(swap_filename)" function
pub fn f_swapinfo(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    swapfile_dict(tv_get_string(&argvars[0]), tv_dict_alloc_ret(rettv));
}

/// "swapname(expr)" function
pub fn f_swapname(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.v_type = VarType::VarString;
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        } else if let Some(ref mf_fname) = (*(*buf).b_ml.mfp_ptr()).mf_fname {
            crate::eval::typval::tv_set_string(rettv, mf_fname.as_ptr());
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

/// "synID(lnum, col, trans)" function
pub fn f_syn_id(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let lnum = tv_get_lnum(&argvars[0]);
        let col = (tv_get_number(&argvars[1]) as Colnr) - 1;

        let mut transerr = false;
        let trans = tv_get_bool_chk(&argvars[2], Some(&mut transerr)) as i32;

        let mut id: i32 = 0;
        if !transerr
            && lnum >= 1
            && lnum <= curbuf_mut().b_ml.ml_line_count
            && col >= 0
            && col < ml_get_len(lnum)
        {
            id = syn_get_id(curwin_mut(), lnum, col, trans, None, 0);
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(id));
    }
}

/// "synIDattr(id, what [, mode])" function
pub fn f_syn_idattr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let id = tv_get_number(&argvars[0]) as i32;
        let what = tv_get_string(&argvars[1]);
        let mut modec: i32 = if argvars[2].v_type != VarType::VarUnknown {
            let mut modebuf = [0 as c_char; NUMBUFLEN];
            let mode = tv_get_string_buf(&argvars[2], modebuf.as_mut_ptr());
            let mc = tolower_asc(i32::from(crate::memory::cstr_first_byte(mode)));
            if mc != b'c' as i32 && mc != b'g' as i32 {
                0
            } else {
                mc
            }
        } else if ui_rgb_attached() {
            b'g' as i32
        } else {
            b'c' as i32
        };
        let _ = &mut modec;

        let mut p: *const c_char = ptr::null();
        let w = |i: usize| tolower_asc(i32::from(crate::memory::cstr_byte_at(what, i))) as u8;
        match w(0) {
            b'b' => {
                if w(1) == b'g' {
                    p = highlight_color(id, what, modec);
                } else if w(1) == b'l' {
                    p = highlight_has_attr(id, HL_BLINK, modec);
                } else {
                    p = highlight_has_attr(id, HL_BOLD, modec);
                }
            }
            b'c' => p = highlight_has_attr(id, HL_CONCEALED, modec),
            b'd' => p = highlight_has_attr(id, HL_DIM, modec),
            b'o' => p = highlight_has_attr(id, HL_OVERLINE, modec),
            b'f' => p = highlight_color(id, what, modec),
            b'i' => {
                if w(1) == b'n' {
                    p = highlight_has_attr(id, HL_INVERSE, modec);
                } else {
                    p = highlight_has_attr(id, HL_ITALIC, modec);
                }
            }
            b'n' => {
                if w(1) == b'o' {
                    p = highlight_has_attr(id, HL_NOCOMBINE, modec);
                } else {
                    p = get_highlight_name_ext(
                        &mut crate::os::env::ExpandCtx::new(),
                        id - 1,
                        false,
                    );
                }
            }
            b'r' => p = highlight_has_attr(id, HL_INVERSE, modec),
            b's' => {
                if w(1) == b'p' {
                    p = highlight_color(id, what, modec);
                } else if w(1) == b't' && w(2) == b'r' {
                    p = highlight_has_attr(id, HL_STRIKETHROUGH, modec);
                } else {
                    p = highlight_has_attr(id, HL_STANDOUT, modec);
                }
            }
            b'u' => {
                if crate::memory::cstr_len(what) >= 9 {
                    if w(5) == b'l' {
                        p = highlight_has_attr(id, HL_UNDERLINE, modec);
                    } else if w(5) != b'd' {
                        p = highlight_has_attr(id, HL_UNDERCURL, modec);
                    } else if w(6) != b'o' {
                        p = highlight_has_attr(id, HL_UNDERDASHED, modec);
                    } else if w(7) == b'u' {
                        p = highlight_has_attr(id, HL_UNDERDOUBLE, modec);
                    } else {
                        p = highlight_has_attr(id, HL_UNDERDOTTED, modec);
                    }
                } else {
                    p = highlight_color(id, what, modec);
                }
            }
            _ => {}
        }

        crate::eval::typval::tv_set_string(rettv, p);
    }
}

/// "synIDtrans(id)" function
pub fn f_syn_idtrans(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut id = tv_get_number(&argvars[0]) as i32;
    if id > 0 {
        id = syn_get_final_id(id);
    } else {
        id = 0;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(id));
}

/// "synconcealed(lnum, col)" function
pub fn f_synconcealed(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut syntax_flags: i32 = 0;
        let mut matchid: i32 = 0;
        let mut str_ = [0 as c_char; NUMBUFLEN];

        tv_list_set_ret(rettv, ptr::null_mut());

        let lnum = tv_get_lnum(&argvars[0]);
        let col = (tv_get_number(&argvars[1]) as Colnr) - 1;

        if lnum >= 1
            && lnum <= curbuf_mut().b_ml.ml_line_count
            && col >= 0
            && col <= ml_get_len(lnum)
            && curwin_mut().w_onebuf_opt.wo_cole > 0
        {
            syn_get_id(curwin_mut(), lnum, col, 0, None, 0);
            syntax_flags = get_syntax_info(&mut matchid);

            if (syntax_flags & HL_CONCEAL) != 0 && curwin_mut().w_onebuf_opt.wo_cole < 3 {
                let mut cchar = schar_from_char(syn_get_sub_char());
                if cchar == 0 && curwin_mut().w_onebuf_opt.wo_cole == 1 {
                    cchar = if curwin_mut().w_p_lcs_chars.conceal == 0 {
                        b' ' as Schar
                    } else {
                        curwin_mut().w_p_lcs_chars.conceal
                    };
                }
                if cchar != 0 {
                    schar_get(str_.as_mut_ptr(), cchar);
                }
            }
        }

        tv_list_alloc_ret(rettv, 3);
        tv_list_append_number(
            rettv.vval.v_list_ref(),
            Varnumber::from((syntax_flags & HL_CONCEAL) != 0),
        );
        tv_list_append_string(rettv.vval.v_list_ref(), str_.as_ptr(), -1);
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(matchid));
    }
}

/// "synstack(lnum, col)" function
pub fn f_synstack(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        tv_list_set_ret(rettv, ptr::null_mut());

        let lnum = tv_get_lnum(&argvars[0]);
        let col = (tv_get_number(&argvars[1]) as Colnr) - 1;

        if lnum >= 1
            && lnum <= curbuf_mut().b_ml.ml_line_count
            && col >= 0
            && col <= ml_get_len(lnum)
        {
            tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
            syn_get_id(curwin_mut(), lnum, col, 0, None, 1);

            let mut i: i32 = 0;
            loop {
                let id = syn_get_stack_item(i);
                i += 1;
                if id < 0 {
                    break;
                }
                tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(id));
            }
        }
    }
}

/// "tabpagebuflist()" function
pub fn f_tabpagebuflist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut wp: *mut WinInfoInfo = ptr::null_mut();
        if argvars[0].v_type == VarType::VarUnknown {
            wp = g_st().firstwin;
        } else {
            let tp = find_tabpage(tv_get_number(&argvars[0]) as i32);
            if !tp.is_null() {
                wp = if tp == g_st().curtab {
                    g_st().firstwin
                } else {
                    (*tp).tp_firstwin
                };
            }
        }
        if !wp.is_null() {
            tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
            while !wp.is_null() {
                tv_list_append_number(
                    rettv.vval.v_list_ref(),
                    Varnumber::from((*wp).buf_mut().b_fnum),
                );
                wp = (*wp).w_next;
            }
        }
    }
}

/// "tagfiles()" function
pub fn f_tagfiles(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);
    let mut fname = [0 as c_char; MAXPATHL];

    let mut first = true;
    let mut tn = Tagname::new();
    while get_tagfname(&mut tn, i32::from(first), fname.as_mut_ptr()) {
        tv_list_append_string(rettv.vval.v_list_ref(), fname.as_ptr(), -1);
        first = false;
    }
}

/// "taglist()" function
pub fn f_taglist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let tag_pattern = tv_get_string(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        if crate::memory::cstr_is_empty(tag_pattern) {
            return;
        }
        let mut fname: *const c_char = ptr::null();
        if argvars[1].v_type != VarType::VarUnknown {
            fname = tv_get_string(&argvars[1]);
        }
        get_tags(
            &mut *tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST),
            tag_pattern.cast_mut(),
            fname.cast_mut(),
        );
    }
}

/// "timer_info([timer])" function
pub fn f_timer_info(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);
        if !tv_check_for_opt_number_arg(argvars, 0) {
            return;
        }
        if argvars[0].v_type != VarType::VarUnknown {
            let timer = find_timer_by_nr(tv_get_number(&argvars[0]));
            if !timer.is_null()
                && (!unsafe { (*timer).stopped } || unsafe { (*timer).refcount } > 1)
            {
                add_timer_info(rettv, unsafe { &mut *timer });
            }
        } else {
            add_timer_info_all(&mut *rettv);
        }
    }
}

/// "timer_pause(timer, paused)" function
pub fn f_timer_pause(argvars: &mut [TypVal], _unused: &mut TypVal, _fptr: EvalFuncData) {
    {
        if argvars[0].v_type != VarType::VarNumber {
            emsg(gettext(E_NUMBER_EXP));
            return;
        }
        let paused = tv_get_bool(&argvars[1]) != 0;
        let timer = find_timer_by_nr(tv_get_number(&argvars[0]));
        if !timer.is_null() {
            let timer = unsafe { &mut *timer };
            let timer = { &mut *timer };
            if !timer.paused && paused {
                time_watcher_stop(&mut timer.watcher);
            } else if timer.paused && !paused {
                time_watcher_start(
                    &mut timer.watcher,
                    timer.timeout as u64,
                    timer.timeout as u64,
                );
            }
            timer.paused = paused;
        }
    }
}

/// "timer_start(timeout, callback, opts)" function
pub fn f_timer_start(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut repeat: i32 = 1;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        if check_secure() {
            return;
        }

        if argvars[2].v_type != VarType::VarUnknown {
            if !tv_check_for_nonnull_dict_arg(argvars, 2) {
                return;
            }
            let dict = argvars[2].vval.v_dict();
            let di = tv_dict_find(unsafe { dict.as_ref() }, c"repeat", 6);
            if !di.is_null() {
                repeat = tv_get_number(unsafe { &(*di).di_tv }) as i32;
                if repeat == 0 {
                    repeat = 1;
                }
            }
        }

        let mut callback = Callback::NONE;
        if !callback_from_typval(&mut callback, &argvars[1]) {
            return;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(timer_start(
            tv_get_number(&argvars[0]),
            repeat,
            callback,
        ) as Varnumber);
    }
}

/// "timer_stop(timerid)" function
pub fn f_timer_stop(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        if !tv_check_for_number_arg(argvars, 0) {
            return;
        }
        let timer = find_timer_by_nr(tv_get_number(&argvars[0]));
        if timer.is_null() {
            return;
        }
        timer_stop(unsafe { &mut *timer });
    }
}

pub fn f_timer_stopall(_argvars: &mut [TypVal], _unused: &mut TypVal, _fptr: EvalFuncData) {
    timer_stop_all();
}

/// "type(expr)" function
pub fn f_type(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut n: Varnumber = -1;
        match argvars[0].v_type {
            VarType::VarNumber => n = VAR_TYPE_NUMBER,
            VarType::VarString => n = VAR_TYPE_STRING,
            VarType::VarPartial | VarType::VarFunc => n = VAR_TYPE_FUNC,
            VarType::VarList => n = VAR_TYPE_LIST,
            VarType::VarDict => n = VAR_TYPE_DICT,
            VarType::VarFloat => n = VAR_TYPE_FLOAT,
            VarType::VarBool => n = VAR_TYPE_BOOL,
            VarType::VarSpecial => n = VAR_TYPE_SPECIAL,
            VarType::VarBlob => n = VAR_TYPE_BLOB,
            VarType::VarUnknown => internal_error(c"f_type(UNKNOWN)"),
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(n);
    }
}

/// "virtcol({expr}, [, {list} [, {winid}]])" function
pub fn f_virtcol(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut vcol_start: Colnr = 0;
        let mut vcol_end: Colnr = 0;
        let mut wp = curwin_ptr();

        'theend: {
            if argvars[1].v_type != VarType::VarUnknown && argvars[2].v_type != VarType::VarUnknown
            {
                let mut tp: *mut TabPageInfo = ptr::null_mut();
                wp = win_id2wp_tp(tv_get_number(&argvars[2]) as i32, &raw mut tp);
                if wp.is_null() || tp.is_null() {
                    break 'theend;
                }
                check_cursor(&mut *wp);
            }

            let bp = (*wp).w_buffer;
            let bp = { &mut *bp };
            let mut fnum = bp.b_fnum;
            let fp = var2fpos(&argvars[0], false, &mut fnum, false, &mut *wp);
            if !fp.is_null() && (*fp).lnum <= bp.b_ml.ml_line_count && fnum == bp.b_fnum {
                if (*fp).col < 0 {
                    (*fp).col = 0;
                } else {
                    let len = ml_get_buf_len(&mut *bp, (*fp).lnum);
                    if (*fp).col > len {
                        (*fp).col = len;
                    }
                }
                getvvcol(
                    &mut *(wp),
                    &mut *fp,
                    Some(&mut vcol_start),
                    None,
                    Some(&mut vcol_end),
                    0,
                );
                vcol_start += 1;
                vcol_end += 1;
            }
        }

        if argvars[1].v_type != VarType::VarUnknown && tv_get_bool(&argvars[1]) != 0 {
            tv_list_alloc_ret(rettv, 2);
            tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(vcol_start));
            tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(vcol_end));
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(vcol_end));
        }
    }
}

/// "visualmode()" function
pub fn f_visualmode(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let str_ = [curbuf_mut().b_visual_mode_eval as c_char, NUL];
        crate::eval::typval::tv_set_string(rettv, str_.as_ptr());

        if non_zero_arg(&argvars[0]) {
            curbuf_mut().b_visual_mode_eval = 0;
        }
    }
}

/// "wildmenumode()" function
pub fn f_wildmenumode(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if g_st().wild_menu_showing != 0
        || ((g_st().state_global & MODE_CMDLINE) != 0 && cmdline_pum_active())
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    }
}

/// "windowsversion()" function
pub fn f_windowsversion(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, (&raw const g_st().windows_version).cast());
}

/// "wordcount()" function
pub fn f_wordcount(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_dict_alloc_ret(rettv);
    cursor_pos_info(unsafe { rettv.vval.v_dict().as_mut() });
}

/// "xor(expr, expr)" function
pub fn f_xor(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        tv_get_number_chk(&argvars[0], None) ^ tv_get_number_chk(&argvars[1], None),
    );
}

use crate::api::buffer::VIML_INTERNAL_CALL;
use crate::api::ui::{K_CHANNEL_PART_ALL, MAX_SCHAR_SIZE};
use crate::arglist::{f_argc, f_argidx, f_arglistid, f_argv};
use crate::change::MB_MAXCHAR;
use crate::charset::NUL;
#[cfg(windows)]
use crate::cmdexpand::MB_MAXBYTES;
use crate::cmdexpand::{f_cmdcomplete_info, f_getcompletion, f_getcompletiontype};
use crate::cmdhist::{f_histadd, f_histdel, f_histget, f_histnr};
use crate::diff::{f_diff_filler, f_diff_hl_id};
use crate::digraph::{
    K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST, f_digraph_get, f_digraph_getlist, f_digraph_set,
    f_digraph_setlist,
};
use crate::drawline::{HL_OVERLINE, HL_STRIKETHROUGH, HLF_COUNT, SMT_ALL};
use crate::eval::buffer::{
    f_append, f_appendbufline, f_bufadd, f_bufexists, f_buflisted, f_bufload, f_bufloaded,
    f_bufname, f_bufnr, f_bufwinid, f_bufwinnr, f_deletebufline, f_getbufinfo, f_getbufline,
    f_getbufoneline, f_getline, f_prompt_appendbuf, f_prompt_getinput, f_prompt_getprompt,
    f_prompt_setcallback, f_prompt_setinterrupt, f_prompt_setprompt, f_setbufline, f_setline,
};
use crate::eval::deprecated::{f_last_buffer_nr, f_rpcstart, f_rpcstop, f_termopen};
use crate::eval::fs::{
    f_browse, f_browsedir, f_chdir, f_delete, f_executable, f_exepath, f_filecopy, f_filereadable,
    f_filewritable, f_finddir, f_findfile, f_fnamemodify, f_getcwd, f_getfperm, f_getfsize,
    f_getftime, f_getftype, f_glob, f_glob2regpat, f_globpath, f_haslocaldir, f_isabsolutepath,
    f_isdirectory, f_mkdir, f_pathshorten, f_readblob, f_readdir, f_readfile, f_rename, f_resolve,
    f_simplify, f_tempname, f_writefile,
};
use crate::eval::list::{
    f_add, f_count, f_extend, f_extendnew, f_filter, f_foreach, f_insert, f_map, f_mapnew,
    f_remove, f_reverse,
};
use crate::eval::typval::{
    K_CALLBACK_NONE_CONST, f_blob2list, f_has_key, f_items, f_join, f_keys, f_list2blob,
    f_list2str, f_sort, f_uniq, f_values,
};
use crate::eval::vars::{
    f_getbufvar, f_gettabvar, f_gettabwinvar, f_getwinvar, f_setbufvar, f_settabvar,
    f_settabwinvar, f_setwinvar,
};
use crate::eval::window::{
    f_getcmdwintype, f_gettabinfo, f_getwininfo, f_getwinpos, f_getwinposx, f_getwinposy,
    f_tabpagenr, f_tabpagewinnr, f_win_execute, f_win_findbuf, f_win_getid, f_win_gettype,
    f_win_gotoid, f_win_id2tabwin, f_win_id2win, f_win_move_separator, f_win_move_statusline,
    f_win_screenpos, f_win_splitmove, f_winbufnr, f_wincol, f_winheight, f_winlayout, f_winline,
    f_winnr, f_winrestcmd, f_winrestview, f_winsaveview, f_winwidth,
};
use crate::eval::{f_slice, f_system, f_systemlist};
use crate::ex_docmd::f_fullcommand;
use crate::ex_getln::{
    f_getcmdcomplpat, f_getcmdcompltype, f_getcmdline, f_getcmdpos, f_getcmdprompt,
    f_getcmdscreenpos, f_getcmdtype, f_setcmdline, f_setcmdpos, f_wildtrigger,
};
use crate::fold::{f_foldclosed, f_foldclosedend, f_foldlevel, f_foldtext, f_foldtextresult};
use crate::fuzzy::{f_matchfuzzy, f_matchfuzzypos};
use crate::globals::IOSIZE;
use crate::indent::{f_indent, f_lispindent};
use crate::indent_c::f_cindent;
use crate::input::{f_getchar, f_getcharmod, f_getcharstr};
use crate::insexpand::{
    f_complete, f_complete_add, f_complete_check, f_complete_info, f_preinserted,
};
use crate::mapping::{f_hasmapto, f_maparg, f_mapcheck, f_maplist, f_mapset};
use crate::r#match::{
    f_clearmatches, f_getmatches, f_matchadd, f_matchaddpos, f_matcharg, f_matchdelete,
    f_setmatches,
};
use crate::mbyte::{f_charclass, f_getcellwidths, f_iconv, f_setcellwidths};
use crate::menu::f_menu_info;
use crate::mouse::f_getmousepos;
use crate::r#move::{f_screenpos, f_virtcol2col};
use crate::quickfix::{f_getloclist, f_getqflist, f_setloclist, f_setqflist};
use crate::runtime::{f_getscriptinfo, f_getstacktrace};
use crate::search::{FORWARD, f_searchcount};
use crate::sign::{
    f_sign_define, f_sign_getdefined, f_sign_getplaced, f_sign_jump, f_sign_place,
    f_sign_placelist, f_sign_undefine, f_sign_unplace, f_sign_unplacelist,
};
use crate::strings::{
    f_byteidx, f_byteidxcomp, f_charidx, f_str2list, f_str2nr, f_strcharlen, f_strcharpart,
    f_strchars, f_strdisplaywidth, f_strgetchar, f_stridx, f_string, f_strlen, f_strpart,
    f_strridx, f_strtrans, f_strutf16len, f_strwidth, f_tolower, f_toupper, f_tr, f_trim,
    f_utf16idx,
};
use crate::testing::{
    f_assert_beeps, f_assert_equal, f_assert_equalfile, f_assert_exception, f_assert_fails,
    f_assert_false, f_assert_inrange, f_assert_match, f_assert_nobeep, f_assert_notequal,
    f_assert_notmatch, f_assert_report, f_assert_true, f_test_garbagecollect_now,
    f_test_write_list_log,
};
use crate::undo::{f_undofile, f_undotree};
use crate::viml::parser::parser::CONV_NONE;

macro_rules! viml_fn {
    ($f:ident) => {{ { Some($f) } }};
}

static FUNCTIONS: [EvalFuncDef; 643] = [
    EvalFuncDef {
        name: Some(c"id"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_id),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"or"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_or),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"tr"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_tr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"abs"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_abs),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"add"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_add),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"and"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_and),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"col"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_col),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"cos"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::cos),
    },
    EvalFuncDef {
        name: Some(c"exp"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::exp),
    },
    EvalFuncDef {
        name: Some(c"get"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_get),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"has"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_has),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"len"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_len),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"log"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::ln),
    },
    EvalFuncDef {
        name: Some(c"map"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_map),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"max"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_max),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"min"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_min),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"pow"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_pow),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sin"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::sin),
    },
    EvalFuncDef {
        name: Some(c"tan"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::tan),
    },
    EvalFuncDef {
        name: Some(c"xor"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_xor),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"hlID"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hl_id),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"glob"),
        min_argc: 1,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_glob),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"argc"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_argc),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"fmod"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_fmod),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rand"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_rand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"line"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_line),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"mode"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"type"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_type),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"cosh"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::cosh),
    },
    EvalFuncDef {
        name: Some(c"sinh"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::sinh),
    },
    EvalFuncDef {
        name: Some(c"tanh"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::tanh),
    },
    EvalFuncDef {
        name: Some(c"call"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_call),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"ceil"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::ceil),
    },
    EvalFuncDef {
        name: Some(c"eval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"trim"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_trim),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"asin"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::asin),
    },
    EvalFuncDef {
        name: Some(c"atan"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::atan),
    },
    EvalFuncDef {
        name: Some(c"join"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_join),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"uniq"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_uniq),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"acos"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::acos),
    },
    EvalFuncDef {
        name: Some(c"keys"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_keys),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sort"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sort),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sqrt"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::sqrt),
    },
    EvalFuncDef {
        name: Some(c"wait"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wait),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"argv"),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_argv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"copy"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_copy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"match"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_match),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"range"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_range),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"iconv"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_iconv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"chdir"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_chdir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winnr"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"mkdir"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mkdir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"floor"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::floor),
    },
    EvalFuncDef {
        name: Some(c"slice"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_slice),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"empty"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_empty),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"index"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_index),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"input"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_input),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"count"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_count),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"log10"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::log10),
    },
    EvalFuncDef {
        name: Some(c"round"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::round),
    },
    EvalFuncDef {
        name: Some(c"split"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_split),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"srand"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_srand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"trunc"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::trunc),
    },
    EvalFuncDef {
        name: Some(c"isinf"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_isinf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"isnan"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_isnan),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"atan2"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_atan2),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"items"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_items),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"state"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_state),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufnr"),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"synID"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_syn_id),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sha256"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sha256),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"append"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_append),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufadd"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufadd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"expand"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_expand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"extend"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_extend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcwd"),
        min_argc: 0,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcwd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getpid"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_getpid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobpid"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobpid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"browse"),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_browse),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"delete"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_delete),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"escape"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_escape),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reduce"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_reduce),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"remove"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_remove),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rename"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_rename),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"printf"),
        min_argc: 1,
        max_argc: MAX_FUNC_ARGS as u8,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_printf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getreg"),
        min_argc: 0,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getreg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"maparg"),
        min_argc: 1,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_maparg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setreg"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setreg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"string"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_string),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"search"),
        min_argc: 1,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_search),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"pyeval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_py3eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"wincol"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wincol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"system"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_system),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strlen"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strlen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"cursor"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_cursor),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"filter"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_filter),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"histnr"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_histnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"str2nr"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_str2nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"exists"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_exists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getpos"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setpos"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"values"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_values),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"indent"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_indent),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"insert"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_insert),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"invert"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_invert),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"mapset"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mapset),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"repeat"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_repeat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getenv"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getenv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setenv"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setenv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"mapnew"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mapnew),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"argidx"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_argidx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"stridx"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_stridx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufload"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufload),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufname"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"byteidx"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_byteidx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"char2nr"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_char2nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"charcol"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_charcol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"charidx"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_charidx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"cindent"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_cindent),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"confirm"),
        min_argc: 1,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_confirm),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"environ"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(lua_wrapper),
        data: EvalFuncData::LuaFunc(c"f_environ"),
    },
    EvalFuncDef {
        name: Some(c"execute"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_execute),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"exepath"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_exepath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"finddir"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_finddir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"flatten"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_flatten),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foreach"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foreach),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"funcref"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_funcref),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getchar"),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getchar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getline"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"gettext"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettext),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"has_key"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_has_key),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"histadd"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_histadd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"histdel"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_histdel),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"histget"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_histget),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"indexof"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_indexof),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobsend"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chansend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobstop"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobstop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobwait"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobwait),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"libcall"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_libcall),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"luaeval"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_luaeval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"maplist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_maplist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nr2char"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_nr2char),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"py3eval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_py3eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"pyxeval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_py3eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"readdir"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_readdir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reltime"),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_reltime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"resolve"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_resolve),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reverse"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_reverse),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rpcstop"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcstop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setline"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"stdpath"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_stdpath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strpart"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strpart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strridx"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strridx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"taglist"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_taglist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"tolower"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_tolower),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"toupper"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_toupper),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"virtcol"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_virtcol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winline"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_winline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"utf16idx"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_utf16idx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"str2list"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_str2list),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"api_info"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_api_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"float2nr"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_float2nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"globpath"),
        min_argc: 2,
        max_argc: 5,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_globpath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winbufnr"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winbufnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"function"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_function),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobclose"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chanclose),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"mapcheck"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mapcheck),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchadd"),
        min_argc: 2,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchadd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matcharg"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matcharg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchend"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchstr"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchstr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strchars"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strchars),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"feedkeys"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_feedkeys),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"findfile"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_findfile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foldtext"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_foldtext),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"readblob"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_readblob),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"readfile"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_readfile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"filecopy"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_filecopy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getfperm"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getfperm),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getfsize"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getfsize),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getftime"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getftime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getftype"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getftype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setfperm"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setfperm),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strftime"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strftime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"tagfiles"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_tagfiles),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"perleval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_perleval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"hasmapto"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hasmapto),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__id"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[1]),
    },
    EvalFuncDef {
        name: Some(c"nvim_cmd"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[2]),
    },
    EvalFuncDef {
        name: Some(c"nvim_put"),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[3]),
    },
    EvalFuncDef {
        name: Some(c"submatch"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_submatch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"termopen"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_termopen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"changenr"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_changenr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"chansend"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chansend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"islocked"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_islocked),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"undofile"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_undofile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"undotree"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_undotree),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"complete"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_complete),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"deepcopy"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_deepcopy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"simplify"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_simplify),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strptime"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strptime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"swapinfo"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_swapinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"swapname"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_swapname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"tempname"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_tempname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobstart"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobstart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rpcstart"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcstart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"synstack"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_synstack),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"hostname"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(lua_wrapper),
        data: EvalFuncData::LuaFunc(c"f_hostname"),
    },
    EvalFuncDef {
        name: Some(c"keytrans"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_keytrans),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"list2str"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_list2str),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strtrans"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strtrans),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"menu_get"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_menu_get),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufwinid"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufwinid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufwinnr"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufwinnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strwidth"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strwidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winwidth"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winwidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"hlexists"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hlexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rubyeval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_rubyeval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"blob2list"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_blob2list),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"byte2line"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_byte2line),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"line2byte"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_line2byte),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"list2blob"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_list2blob),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"synIDattr"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_syn_idattr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"diff_hlID"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_diff_hl_id),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"menu_info"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_menu_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_echo"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[7]),
    },
    EvalFuncDef {
        name: Some(c"nvim_eval"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[9]),
    },
    EvalFuncDef {
        name: Some(c"nvim_exec"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[10]),
    },
    EvalFuncDef {
        name: Some(c"sign_jump"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_jump),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"gettabvar"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettabvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"libcallnr"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_libcallnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"settabvar"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_settabvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"tabpagenr"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_tabpagenr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winlayout"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winlayout),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"chanclose"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chanclose),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"charclass"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_charclass),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"searchpos"),
        min_argc: 1,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_searchpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"wordcount"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wordcount),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"soundfold"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_soundfold),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getregion"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getregion),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"jobresize"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobresize),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"screencol"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_screencol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"screenpos"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"screenrow"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_screenrow),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winheight"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winheight),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"writefile"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_writefile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getqflist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getqflist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setqflist"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setqflist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"str2float"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_str2float),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_getid"),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_getid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchlist"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"arglistid"),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_arglistid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"buflisted"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_buflisted),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getwinpos"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getwinpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getwinvar"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setwinvar"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_setwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foldlevel"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldlevel),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"localtime"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_localtime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcmdpos"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setcmdpos"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcmdpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"expandcmd"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_expandcmd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"extendnew"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_extendnew),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufloaded"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufloaded),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rpcnotify"),
        min_argc: 2,
        max_argc: MAX_FUNC_ARGS as u8,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcnotify),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"stdioopen"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_stdioopen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"interrupt"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_interrupt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"browsedir"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_browsedir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"inputlist"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_inputlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"inputsave"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_inputsave),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getbufvar"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcurpos"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcurpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setbufvar"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_setbufvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"bufexists"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"timer_info"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"timer_stop"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_stop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcharmod"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcharmod),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcharpos"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcharstr"),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcharstr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setcharpos"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strcharlen"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strcharlen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"debugbreak"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_debugbreak),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"gettabinfo"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettabinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getloclist"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getloclist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setloclist"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setloclist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcmdline"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcmdtype"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdtype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setcmdline"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcmdline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_id2win"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_id2win),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"flattennew"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_flattennew),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_exec2"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[11]),
    },
    EvalFuncDef {
        name: Some(c"pum_getpos"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_pum_getpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getbufinfo"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getbufline"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchfuzzy"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchfuzzy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setbufline"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_setbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getreginfo"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getreginfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getregtype"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getregtype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"searchdecl"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_searchdecl),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"searchpair"),
        min_argc: 3,
        max_argc: 7,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_searchpair),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_input"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[12]),
    },
    EvalFuncDef {
        name: Some(c"substitute"),
        min_argc: 4,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_substitute),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foldclosed"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldclosed),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"visualmode"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_visualmode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reltimestr"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_reltimestr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"systemlist"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_systemlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getwininfo"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getwininfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getwinposx"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getwinposx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getwinposy"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getwinposy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"lispindent"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_lispindent),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"screenattr"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenattr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"screenchar"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenchar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_gotoid"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_gotoid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_paste"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[13]),
    },
    EvalFuncDef {
        name: Some(c"sign_place"),
        min_argc: 4,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_place),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"rpcrequest"),
        min_argc: 2,
        max_argc: MAX_FUNC_ARGS as u8,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcrequest),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foreground"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_foreground),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"serverlist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_serverlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"serverstop"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_serverstop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"pumvisible"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_pumvisible),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winrestcmd"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_winrestcmd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"executable"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_executable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getmatches"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getmatches),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setmatches"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setmatches),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strgetchar"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strgetchar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"synIDtrans"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_syn_idtrans),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"shiftwidth"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_shiftwidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__stats"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[19]),
    },
    EvalFuncDef {
        name: Some(c"timer_pause"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_pause),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"timer_start"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_start),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchaddpos"),
        min_argc: 2,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchaddpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strcharpart"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strcharpart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"haslocaldir"),
        min_argc: 0,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_haslocaldir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"msgpackdump"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_msgpackdump),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"byteidxcomp"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_byteidxcomp),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"inputdialog"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_inputdialog),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"json_decode"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_json_decode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchdelete"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchdelete),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sign_define"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_define),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"fnameescape"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_fnameescape),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"isdirectory"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_isdirectory),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"json_encode"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_json_encode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"shellescape"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_shellescape),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_gettype"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_gettype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"diff_filler"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_diff_filler),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"strutf16len"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strutf16len),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"gettagstack"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettagstack),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_get_hl"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[14]),
    },
    EvalFuncDef {
        name: Some(c"settagstack"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_settagstack),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"pathshorten"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_pathshorten),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"searchcount"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_searchcount),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"highlightID"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hl_id),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_findbuf"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_findbuf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"fnamemodify"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_fnamemodify),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getjumplist"),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getjumplist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getfontname"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getfontname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_notify"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[18]),
    },
    EvalFuncDef {
        name: Some(c"screenchars"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenchars),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"fullcommand"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_fullcommand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sockconnect"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_sockconnect),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"virtcol2col"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_virtcol2col),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"digraph_get"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_get),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"digraph_set"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_set),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"buffer_name"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getmarklist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getmarklist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"glob2regpat"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_glob2regpat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"serverstart"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_serverstart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"wildtrigger"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wildtrigger),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"inputsecret"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_inputsecret),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchstrpos"),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchstrpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_set_hl"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[15]),
    },
    EvalFuncDef {
        name: Some(c"preinserted"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_preinserted),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winrestview"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winrestview),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_true"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_true),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getmousepos"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getmousepos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"winsaveview"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_winsaveview),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_execute"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_win_execute),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__ns_get"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[29]),
    },
    EvalFuncDef {
        name: Some(c"nvim__ns_set"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[30]),
    },
    EvalFuncDef {
        name: Some(c"nvim__redraw"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[25]),
    },
    EvalFuncDef {
        name: Some(c"nvim__unpack"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[28]),
    },
    EvalFuncDef {
        name: Some(c"gettabwinvar"),
        min_argc: 3,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettabwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchbufline"),
        min_argc: 4,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"settabwinvar"),
        min_argc: 4,
        max_argc: 4,
        base_arg: 4,
        fast: false,
        func: viml_fn!(f_settabwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"spellbadword"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_spellbadword),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"msgpackparse"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_msgpackparse),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_command"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[27]),
    },
    EvalFuncDef {
        name: Some(c"getcmdprompt"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdprompt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_del_var"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[26]),
    },
    EvalFuncDef {
        name: Some(c"complete_add"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_complete_add),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"filereadable"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_filereadable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reg_recorded"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_reg_recorded),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"wildmenumode"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wildmenumode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getregionpos"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getregionpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_get_var"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[31]),
    },
    EvalFuncDef {
        name: Some(c"tabpagewinnr"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_tabpagewinnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"eventhandler"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_eventhandler),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"did_filetype"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_did_filetype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"swapfilelist"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_swapfilelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"clearmatches"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_clearmatches),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reltimefloat"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_reltimefloat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"screenstring"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenstring),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"synconcealed"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_synconcealed),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nextnonblank"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_nextnonblank),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"prevnonblank"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prevnonblank),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"filewritable"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_filewritable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"inputrestore"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_inputrestore),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchstrlist"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchstrlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_set_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[32]),
    },
    EvalFuncDef {
        name: Some(c"spellsuggest"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_spellsuggest),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_beeps"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_beeps),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_equal"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_equal),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_fails"),
        min_argc: 1,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_fails),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_false"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_false),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_match"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_match),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_ui_send"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[22]),
    },
    EvalFuncDef {
        name: Some(c"sign_unplace"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_unplace),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__id_dict"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[37]),
    },
    EvalFuncDef {
        name: Some(c"timer_stopall"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_timer_stopall),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getchangelist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getchangelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcharsearch"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcharsearch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getstacktrace"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getstacktrace),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"setcharsearch"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcharsearch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_chan_set"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[40]),
    },
    EvalFuncDef {
        name: Some(c"win_screenpos"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_screenpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"appendbufline"),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_appendbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcmdwintype"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdwintype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_del_mark"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[45]),
    },
    EvalFuncDef {
        name: Some(c"win_id2tabwin"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_id2tabwin),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"complete_info"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_complete_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"deletebufline"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_deletebufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"reg_recording"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_reg_recording),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getbufoneline"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufoneline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"matchfuzzypos"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchfuzzypos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_feedkeys"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[51]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_mark"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[46]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_mode"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[38]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_proc"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[43]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_vvar"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[34]),
    },
    EvalFuncDef {
        name: Some(c"searchpairpos"),
        min_argc: 3,
        max_argc: 7,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_searchpairpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foldclosedend"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldclosedend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcellwidths"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcellwidths),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_list_uis"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[41]),
    },
    EvalFuncDef {
        name: Some(c"setcellwidths"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcellwidths),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcompletion"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcompletion),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_open_win"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[42]),
    },
    EvalFuncDef {
        name: Some(c"win_splitmove"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_splitmove),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"buffer_exists"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"buffer_number"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"file_readable"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_filereadable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getscriptinfo"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getscriptinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_set_vvar"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[35]),
    },
    EvalFuncDef {
        name: Some(c"nvim_strwidth"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[47]),
    },
    EvalFuncDef {
        name: Some(c"assert_nobeep"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_nobeep),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_report"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_report),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"sign_undefine"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_undefine),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_win_hide"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[39]),
    },
    EvalFuncDef {
        name: Some(c"reg_executing"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_reg_executing),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__id_array"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[53]),
    },
    EvalFuncDef {
        name: Some(c"nvim__id_float"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[54]),
    },
    EvalFuncDef {
        name: Some(c"dictwatcheradd"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_dictwatcheradd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"dictwatcherdel"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_dictwatcherdel),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"last_buffer_nr"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_last_buffer_nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcmdcomplpat"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdcomplpat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"complete_check"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_complete_check),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"foldtextresult"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldtextresult),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_err_write"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[56]),
    },
    EvalFuncDef {
        name: Some(c"garbagecollect"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_garbagecollect),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_get_hl_ns"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[59]),
    },
    EvalFuncDef {
        name: Some(c"sign_getplaced"),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_getplaced),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"tabpagebuflist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_tabpagebuflist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_list_bufs"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[60]),
    },
    EvalFuncDef {
        name: Some(c"nvim_list_wins"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[61]),
    },
    EvalFuncDef {
        name: Some(c"isabsolutepath"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_isabsolutepath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_open_term"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[62]),
    },
    EvalFuncDef {
        name: Some(c"nvim_out_write"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[63]),
    },
    EvalFuncDef {
        name: Some(c"nvim_parse_cmd"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[64]),
    },
    EvalFuncDef {
        name: Some(c"sign_placelist"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_placelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_set_hl_ns"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[68]),
    },
    EvalFuncDef {
        name: Some(c"assert_inrange"),
        min_argc: 3,
        max_argc: 4,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_assert_inrange),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_win_close"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[72]),
    },
    EvalFuncDef {
        name: Some(c"windowsversion"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_windowsversion),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__buf_stats"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[100]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_attach"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[83]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_delete"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[84]),
    },
    EvalFuncDef {
        name: Some(c"nvim_create_buf"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[82]),
    },
    EvalFuncDef {
        name: Some(c"getcmdcompltype"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdcompltype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcmdscreenpos"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdscreenpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_del_keymap"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[91]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_keymap"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[96]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_option"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[97]),
    },
    EvalFuncDef {
        name: Some(c"sign_getdefined"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_getdefined),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_list_chans"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[95]),
    },
    EvalFuncDef {
        name: Some(c"digraph_getlist"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_getlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"digraph_setlist"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_setlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_set_keymap"),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[98]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_option"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[99]),
    },
    EvalFuncDef {
        name: Some(c"strdisplaywidth"),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strdisplaywidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_notequal"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_notequal),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_notmatch"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_notmatch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"prompt_getinput"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_getinput),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_win_resize"),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[92]),
    },
    EvalFuncDef {
        name: Some(c"highlight_exists"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hlexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_del_autocmd"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[108]),
    },
    EvalFuncDef {
        name: Some(c"sign_unplacelist"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_unplacelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_exception"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_assert_exception),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"getcursorcharpos"),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcursorcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_get_context"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[109]),
    },
    EvalFuncDef {
        name: Some(c"setcursorcharpos"),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcursorcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_del_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[124]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_del_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[111]),
    },
    EvalFuncDef {
        name: Some(c"nvim__screenshot"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[121]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[125]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_buf"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[112]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[113]),
    },
    EvalFuncDef {
        name: Some(c"prompt_appendbuf"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_prompt_appendbuf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_var"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[126]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_buf"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[114]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_var"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[115]),
    },
    EvalFuncDef {
        name: Some(c"cmdcomplete_info"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_cmdcomplete_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_input_mouse"),
        min_argc: 6,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[116]),
    },
    EvalFuncDef {
        name: Some(c"prompt_getprompt"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_getprompt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"prompt_setprompt"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_setprompt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"assert_equalfile"),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_equalfile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__cmdwin_set"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[107]),
    },
    EvalFuncDef {
        name: Some(c"nvim_err_writeln"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[119]),
    },
    EvalFuncDef {
        name: Some(c"nvim__get_lib_dir"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[137]),
    },
    EvalFuncDef {
        name: Some(c"nvim__get_runtime"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[146]),
    },
    EvalFuncDef {
        name: Some(c"nvim_load_context"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[140]),
    },
    EvalFuncDef {
        name: Some(c"nvim_open_tabpage"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[134]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_autocmds"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[153]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_commands"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[156]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_del_mark"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[129]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_mark"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[130]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_name"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[131]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_text"),
        min_argc: 5,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[138]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_hl_by_id"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[128]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_is_valid"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[150]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_is_valid"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[151]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_mark"),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[132]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_name"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[133]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_text"),
        min_argc: 6,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[139]),
    },
    EvalFuncDef {
        name: Some(c"getcompletiontype"),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcompletiontype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim__complete_set"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[172]),
    },
    EvalFuncDef {
        name: Some(c"nvim__inspect_cell"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[173]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_lines"),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[168]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_is_loaded"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[170]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_lines"),
        min_argc: 5,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[175]),
    },
    EvalFuncDef {
        name: Some(c"nvim_call_function"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[160]),
    },
    EvalFuncDef {
        name: Some(c"nvim_exec_autocmds"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[161]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_chan_info"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[163]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_color_map"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[164]),
    },
    EvalFuncDef {
        name: Some(c"nvim_list_tabpages"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[162]),
    },
    EvalFuncDef {
        name: Some(c"win_move_separator"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_move_separator),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"prompt_setcallback"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_setcallback),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_width"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[169]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_hl_ns"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[176]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_width"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[177]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_hl_by_name"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[180]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_hl_ns_fast"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[181]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_line_count"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[182]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_del_keymap"),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[183]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_keymap"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[184]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_keymap"),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[185]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_height"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[186]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_height"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[187]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_offset"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[188]),
    },
    EvalFuncDef {
        name: Some(c"nvim_create_augroup"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[189]),
    },
    EvalFuncDef {
        name: Some(c"nvim_clear_autocmds"),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[192]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_config"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[193]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_config"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[194]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_option"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[195]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_option"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[196]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_namespaces"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[197]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_option"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[198]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_option"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[199]),
    },
    EvalFuncDef {
        name: Some(c"prompt_setinterrupt"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_setinterrupt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"win_move_statusline"),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_move_statusline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_create_autocmd"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[200]),
    },
    EvalFuncDef {
        name: Some(c"test_write_list_log"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_test_write_list_log),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_number"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[201]),
    },
    EvalFuncDef {
        name: Some(c"nvim_command_output"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[202]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_cursor"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[203]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_number"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[204]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_set_cursor"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[205]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_del_extmark"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[206]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_extmark"),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[207]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_tabpage"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[208]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_current_buf"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[209]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_current_buf"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[210]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_current_dir"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[211]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_text_height"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[213]),
    },
    EvalFuncDef {
        name: Some(c"nvim_eval_statusline"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[214]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_option_info"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[218]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_del_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[221]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_get_var"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[222]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_set_var"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[223]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_current_win"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[224]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_current_win"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[225]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_get_win"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[226]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_set_win"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[227]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_is_valid"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[244]),
    },
    EvalFuncDef {
        name: Some(c"nvim_del_current_line"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[232]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_current_line"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[235]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_current_line"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[242]),
    },
    EvalFuncDef {
        name: Some(c"nvim_parse_expression"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[239]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_commands"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[229]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_extmarks"),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[230]),
    },
    EvalFuncDef {
        name: Some(c"nvim_win_get_position"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[245]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_option_info2"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[236]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_option_value"),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[237]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_option_value"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[243]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_runtime_file"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[238]),
    },
    EvalFuncDef {
        name: Some(c"nvim__runtime_inspect"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[228]),
    },
    EvalFuncDef {
        name: Some(c"nvim_create_namespace"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[231]),
    },
    EvalFuncDef {
        name: Some(c"nvim_del_user_command"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[233]),
    },
    EvalFuncDef {
        name: Some(c"nvim_replace_termcodes"),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[255]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_add_highlight"),
        min_argc: 6,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[252]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_list_wins"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[256]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_hl_id_by_name"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[260]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_color_by_name"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[259]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_proc_children"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[254]),
    },
    EvalFuncDef {
        name: Some(c"nvim_del_augroup_by_id"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[246]),
    },
    EvalFuncDef {
        name: Some(c"nvim_call_dict_function"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[261]),
    },
    EvalFuncDef {
        name: Some(c"test_garbagecollect_now"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_test_garbagecollect_now),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: Some(c"nvim_list_runtime_paths"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[264]),
    },
    EvalFuncDef {
        name: Some(c"nvim_tabpage_get_number"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[265]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_changedtick"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[267]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_current_tabpage"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[268]),
    },
    EvalFuncDef {
        name: Some(c"nvim_set_current_tabpage"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[269]),
    },
    EvalFuncDef {
        name: Some(c"nvim_del_augroup_by_name"),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[270]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_clear_highlight"),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[271]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_clear_namespace"),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[272]),
    },
    EvalFuncDef {
        name: Some(c"nvim_create_user_command"),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[273]),
    },
    EvalFuncDef {
        name: Some(c"nvim__buf_debug_extmarks"),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[274]),
    },
    EvalFuncDef {
        name: Some(c"nvim_get_all_options_info"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[275]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_del_user_command"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[276]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_set_virtual_text"),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[277]),
    },
    EvalFuncDef {
        name: Some(c"nvim__set_restart_on_crash"),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[278]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_get_extmark_by_id"),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[279]),
    },
    EvalFuncDef {
        name: Some(c"nvim_select_popupmenu_item"),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[280]),
    },
    EvalFuncDef {
        name: Some(c"nvim__invalidate_glyph_cache"),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[281]),
    },
    EvalFuncDef {
        name: Some(c"nvim_buf_create_user_command"),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::METHOD_HANDLERS[282]),
    },
    EvalFuncDef {
        name: None,
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: None,
        data: EvalFuncData::None,
    },
];

pub fn eval_func_def_at(idx: i32) -> &'static EvalFuncDef {
    &FUNCTIONS[idx as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix32_and_xoshiro_are_deterministic() {
        let mut seed: u32 = 42;
        let mut x = splitmix32(&mut seed);
        let mut y = splitmix32(&mut seed);
        let mut z = splitmix32(&mut seed);
        let mut w = splitmix32(&mut seed);
        let r1 = shuffle_xoshiro128starstar(&mut x, &mut y, &mut z, &mut w);
        assert_ne!(r1, 0);
    }

    #[test]
    fn non_zero_arg_checks_number_bool_and_string() {
        let mut tv = tv_initial_value();
        tv.v_type = VarType::VarNumber;
        tv.vval = crate::eval::typval::TypvalVval::Number(0);
        assert!(!non_zero_arg(&tv));
        tv.vval = crate::eval::typval::TypvalVval::Number(5);
        assert!(non_zero_arg(&tv));

        tv.v_type = VarType::VarBool;
        tv.vval = crate::eval::typval::TypvalVval::Bool(K_BOOL_VAR_FALSE_CONST);
        assert!(!non_zero_arg(&tv));
        tv.vval = crate::eval::typval::TypvalVval::Bool(K_BOOL_VAR_TRUE_CONST);
        assert!(non_zero_arg(&tv));
    }

    #[test]
    fn find_internal_func_hash_resolves_builtin_functions() {
        {
            for (idx, fdef) in FUNCTIONS[..FUNCTIONS.len() - 1].iter().enumerate() {
                let fdef_name = fdef.name.unwrap();
                let name = fdef_name.to_bytes();
                assert_eq!(find_internal_func_hash(name), idx as i32);
                assert_eq!(
                    find_internal_func(fdef_name.as_ptr()),
                    eval_func_def_at(idx as i32) as *const _
                );
            }
            assert!(
                eval_func_def_at((FUNCTIONS.len() - 1) as i32)
                    .name
                    .is_none()
            );
            assert_eq!(find_internal_func_hash(b"not_a_builtin_fn"), -1);
            assert!(find_internal_func(c"not_a_builtin_fn").is_null());
            assert!(eval_has_static_feature(c"nvim"));
            assert!(eval_has_static_feature(c"AUTOCMD"));
            assert!(!eval_has_static_feature(c"not_a_feature"));
        }
    }
}
