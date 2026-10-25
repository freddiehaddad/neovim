//! Builtin Vimscript functions (`src/nvim/eval/funcs.c`).

use crate::api::private::converter::{object_to_vim, object_to_vim_take_luaref, vim_to_object};
use crate::api::private::helpers::{
    DI_FLAGS_LOCK, api_clear_error, api_free_object, api_free_string, api_metadata, cbuf_to_string,
    cstr_as_string, dict_set_var,
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
    VAR_FIXED, buf_T, buf_close_terminal, buflist_findnr, buflist_findpat, kUIMessages, setfname,
};
use crate::buffer_updates::luaret_truthy;
use crate::channel::{
    CallbackReader, Channel, ChannelStdinMode, channel_close, channel_connect,
    channel_create_event, channel_decref, channel_from_stdio, channel_incref, channel_job_start,
    channel_send, channel_terminal_alloc, kChannelPartRpc, kChannelPartStderr, kChannelPartStdin,
    kChannelPartStdout, kChannelStdinPipe, kChannelStreamProc, kProcTypePty,
};
use crate::charset::{Ctrl_V, VARNUMBER_MIN, ascii_isdigit, getdigits_int, skipwhite};
use crate::cmdexpand::{
    EXPAND_FILES, ExpandOne, SEARCH_START, WILD_ALL, WILD_ALL_KEEP, WILD_ICASE, WILD_KEEP_ALL,
    WILD_LIST_NOTFOUND, WILD_SILENT, WILD_USE_NL, WILD_USE_SHELLSLASH, cmdidx_T,
    cmdline_pum_active,
};
use crate::cursor::{
    VALID_VIRTCOL, check_cursor, colnr_T, equalpos, get_cursor_pos_ptr, linenr_T, pos_T,
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
    Callback, EvalFuncData, TV_TRANSLATE, TypvalVval, VAR_UNLOCKED, VarType, blob_T, callback_free,
    check_repeat_count, dict_T, dictitem_T, float_T, list_T, listitem_T, partial_T,
    tv_blob_alloc_ret, tv_blob_get, tv_blob_len, tv_blob_set_range, tv_check_for_buffer_arg,
    tv_check_for_dict_arg, tv_check_for_list_arg, tv_check_for_list_or_blob_arg,
    tv_check_for_lnum_arg, tv_check_for_nonempty_string_arg, tv_check_for_nonnull_dict_arg,
    tv_check_for_number_arg, tv_check_for_opt_bool_arg, tv_check_for_opt_dict_arg,
    tv_check_for_opt_number_arg, tv_check_for_string_arg, tv_check_for_string_or_func_arg,
    tv_check_for_string_or_list_arg, tv_check_num, tv_check_str_or_nr, tv_clear, tv_copy,
    tv_dict_add_allocated_str, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool,
    tv_dict_get_callback, tv_dict_get_number, tv_dict_get_number_def, tv_dict_get_string,
    tv_dict_hi2di, tv_dict_item_remove, tv_dict_watcher_add, tv_dict_watcher_remove, tv_equal,
    tv_get_bool, tv_get_bool_chk, tv_get_lnum, tv_get_lnum_buf, tv_get_number, tv_get_number_chk,
    tv_get_string, tv_get_string_buf, tv_get_string_buf_chk, tv_get_string_chk, tv_initial_value,
    tv_is_func, tv_islocked, tv_list_alloc, tv_list_alloc_ret, tv_list_append_allocated_string,
    tv_list_append_dict, tv_list_append_list, tv_list_append_number, tv_list_append_owned_tv,
    tv_list_append_string, tv_list_append_tv, tv_list_copy, tv_list_extend, tv_list_find,
    tv_list_find_nr, tv_list_first, tv_list_flatten, tv_list_item_remove, tv_list_len, tv_list_ref,
    tv_list_set_lock, tv_list_set_ret, tv_list_unref, typval_T, value_check_lock, varnumber_T,
};
use crate::eval::userfunc::{
    EvalFuncDef, FCERR_NONE, FCERR_NOTMETHOD, FCERR_TOOFEW, FCERR_TOOMANY, FCERR_UNKNOWN,
    emsg_funcname, find_func, func_call, func_ptr_ref, func_ref, func_unref, function_exists,
    get_func_arity, get_scriptlocal_funcname, get_user_func_name, printable_func_name,
    restore_funccal, save_funccal, save_function_name, set_current_funccal, trans_function_name,
    translated_function_exists,
};
use crate::eval::vars::{
    VV_KEY, VV_REG, VV_SEND_SERVER, VV_SHELL_ERROR, VV_VAL, cat_prefix_varname, find_var,
    get_user_var_name, get_vim_var_nr, get_vim_var_str, get_vim_var_tv, lval_T, prepare_vimvar,
    restore_vimvar, set_vim_var_nr, set_vim_var_type, tv_list_locked, var_exists,
};
use crate::eval::window::{find_tabwin, find_win_by_nr_or_id, win_id2wp_tp};
use crate::eval::{
    FNE_CHECK_START, GLV_NO_AUTOLOAD, GLV_READ_ONLY, TFN_NO_AUTOLOAD, TFN_QUIET, VARNUMBER_MAX,
    add_timer_info, add_timer_info_all, buf_byteidx_to_charidx, buf_charidx_to_byteidx,
    callback_from_typval, common_job_callbacks, do_string_sub, eval_expr_to_bool,
    eval_expr_typval, eval_expr_valid_arg, eval_has_provider, eval_option, eval1, find_job,
    find_timer_by_nr, get_callback_depth, get_copyID, get_lval, kListLenMayKnow,
    kListLenShouldKnow, kListLenUnknown, list2fpos, partial_name, save_tv_as_string,
    script_host_eval, string2float, timer_due_cb, timer_start, timer_stop, timer_stop_all,
    tv_dict_set_ret, tv_to_argv, var_item_copy, var2fpos,
};
#[cfg(unix)]
use crate::event::r#loop::uv_kill;
use crate::event::r#loop::{
    Loop, loop_closing_ptr, loop_poll_events, loop_queues, uv_strerror,
};
use crate::event::multiqueue::{
    multiqueue_empty, multiqueue_free, multiqueue_new, multiqueue_process_events,
    multiqueue_replace_parent,
};
use crate::event::proc::{Proc, PtyProc, proc_stop, proc_wait};
use crate::event::time::{
    TimeWatcher, time_watcher_close, time_watcher_init, time_watcher_start, time_watcher_stop,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{
    EX_NOSPC, cmd_exists, do_cmdline, do_cmdline_cmd, eval_vars, exarg_T, expand_filename,
};
use crate::ex_eval::aborting;
use crate::ex_getln::{
    VSE_NONE, get_user_input, hlf_T, text_locked, text_locked_msg, vim_strsave_fnameescape,
};
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::globals::*;
use crate::grid::{
    ScreenGrid, grid_getchar, schar_T, schar_from_char, schar_get, schar_get_first_codepoint,
};
use crate::highlight_group::{
    get_highlight_name_ext, highlight_color, highlight_exists, highlight_has_attr,
    syn_get_final_id, syn_name2id,
};
use crate::indent::{get_sw_value, get_sw_value_col};
use crate::input::{restore_typeahead, save_typeahead, stuff_empty, tasave_T, using_script, vgetc};
use crate::insexpand::ins_compl_active;
use crate::keycodes::vim_strsave_escape_ks;
use crate::lua::executor::{
    kRetNilBool, kRetObject, nlua_call_luaeval, nlua_call_typval, nlua_exec, nlua_func_exists,
    nlua_is_table_from_lua, nlua_register_table_as_callable,
};
use crate::mark::{
    cleanup_jumplist, get_buf_local_marks, get_global_marks, setmark_pos, setpcmark,
};
use crate::math::{FAIL, OK, xisinf, xisnan};
use crate::mbyte::{
    convert_setup, enc_locale, mb_adjust_cursor, string_convert, utf_char2bytes, utf_ptr2char,
    utf_ptr2len, utfc_ptr2len, vimconv_T,
};
use crate::memfile::memfile_T;
use crate::memline::{
    decl, incl, ml_find_line_or_offset, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_open,
    recover_names, swapfile_dict,
};
use crate::memory::{
    ARENA_BLOCK_SIZE, Arena, ArenaMem, alloc_block, arena_finish, arena_mem_free, free_block,
    strequal, strnequal, xfree, xmalloc, xmallocz, xmemdup, xmemdupz, xstrdup,
};
use crate::menu::{get_menu_cmd_modes, menu_get};
use crate::message::{
    emsg, internal_error, msg_clr_eos, msg_ext_set_kind, msg_putchar, msg_puts, msg_scroll_flush,
    msg_start, semsg, semsg_multiline, str2special_save, verb_msg,
};
use crate::r#move::{update_curswant, win_col_off};
use crate::mpack::mpack_core::MPACK_OK;
use crate::mpack::object::mpack_parser_t;
use crate::msgpack_rpc::channel::{
    find_channel, get_client_info, rpc_send_call, rpc_send_event,
};
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
    expand_T, expand_env_save, home_replace, os_env_exists, os_get_pid, os_getenv, vim_env_iter,
    vim_getenv, vim_setenv_ext, vim_unsetenv_ext,
};
use crate::os::fs::{MAXPATHL, os_isdir, os_setperm};
use crate::os::pty_proc::pty_proc_resize;
use crate::os::stdpaths::{XDGVarType, get_appname, get_xdg_home, stdpaths_get_xdg_var};
use crate::os::time::{
    NS_PER_SEC, Tm, os_hrtime, os_localtime_r, os_strptime, os_time, strftime, time_t,
};
use crate::path::{ENV_SEPCHAR, concat_fnames_realloc, to_slash_save, vim_FullName};
use crate::plines::{getvvcol, win_chartabsize};
use crate::popupmenu::{pum_set_event_info, pum_visible};
use crate::profile::{
    profile_end, profile_msg, profile_setlimit, profile_signed, profile_start, profile_sub,
    proftime_T,
};
use crate::regexp::{
    MAXCOL, NSUBEXP, NUMBUFLEN, RE_MAGIC, RE_STRING, reg_submatch, reg_submatch_list, regmatch_T,
    vim_regcomp, vim_regexec_nl, vim_regfree,
};
use crate::register::{
    YREG_YANK, block_def, format_reg_type, get_reg_contents, get_reg_type, get_register_name,
    get_unname_register, get_yank_register, kGRegExprSrc, kGRegList, op_reg_set_previous,
    write_reg_contents_ex, write_reg_contents_lst,
};
use crate::runtime::{exestack, sctx_T};
use crate::search::{
    BACKWARD, last_csearch, last_csearch_forward, last_csearch_until, searchit, searchit_arg_T,
    set_csearch_direction, set_csearch_until, set_last_csearch,
};
use crate::sha256::sha256_hex;
use crate::spell::{eval_soundfold, parse_spelllang, spell_check, spell_move_to};
use crate::spellsuggest::spell_suggest_list;
use crate::state::{MODE_CMDLINE, MODE_MAX_LENGTH, get_mode, get_was_safe_state, virtual_active};
#[cfg(windows)]
use crate::strings::mb_strup_buf;
use crate::strings::{
    atoi, gettext, strcmp, strlen, strncmp, strtoul, vim_snprintf, vim_snprintf_safelen,
    vim_snprintf_typval_only, vim_strchr, vim_stricmp, vim_strnicmp, vim_strsave_escaped,
    vim_strsave_shellescape,
};
use crate::syntax::{
    HL_CONCEAL, get_syntax_info, syn_get_id, syn_get_stack_item, syn_get_sub_char, synblock_T,
    syntax_present,
};
use crate::tag::{get_tagfname, get_tags, get_tagstack, set_tagstack, tagname_T, tagname_free};
use crate::textformat::oparg_T;
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_current_col, ui_current_row, ui_flush, ui_gui_attached, ui_has,
    ui_rgb_attached,
};
use crate::ui_compositor::ui_comp_get_grid_at_coord;
use crate::version::{has_nvim_version, has_vim_patch};
use crate::window::{find_tabpage, tabpage_T, win_T};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

pub type SpecialVarValue = c_int;

const BASE_NONE: u8 = 0;
const BASE_LAST: u8 = u8::MAX;

pub(crate) const MAX_FUNC_ARGS: usize = 20;

pub(crate) const kSpecialVarNull: SpecialVarValue = 0;

use crate::eval::vars::{
    VAR_TYPE_BLOB, VAR_TYPE_BOOL, VAR_TYPE_DICT, VAR_TYPE_FLOAT, VAR_TYPE_FUNC, VAR_TYPE_LIST,
    VAR_TYPE_NUMBER, VAR_TYPE_SPECIAL, VAR_TYPE_STRING,
};

pub(crate) const TFN_INT: c_int = 1;
pub(crate) const TFN_NO_DEREF: c_int = 8;

const kChannelStdinNull: ChannelStdinMode = 1;

const VIM_GENERIC: c_int = 0;
const VIM_ERROR: c_int = 1;
pub(crate) const VIM_WARNING: c_int = 2;
const VIM_INFO: c_int = 3;
pub(crate) const VIM_QUESTION: c_int = 4;

pub(crate) const DOCMD_KEYTYPED: c_int = 0x08;

pub(crate) const MENU_INDEX_TIP: c_int = 7;
pub(crate) const MENU_ALL_MODES: c_int = (1 << MENU_INDEX_TIP) - 1;
const AUTOLOAD_CHAR: c_int = b'#' as c_int;

pub(crate) const RE_SEARCH: c_int = 0;

pub(crate) const MPACK_EOF: c_int = 1;
pub(crate) const MPACK_ERROR: c_int = 2;
pub(crate) const MPACK_NOMEM: c_int = 3;

pub(crate) const SEARCH_END: c_int = 0x40;
pub(crate) const SEARCH_KEEP: c_int = 0x400;
pub(crate) const SEARCH_COL: c_int = 0x1000;

const SP_NOMOVE: c_int = 0x01;
const SP_REPEAT: c_int = 0x02;
const SP_RETCOUNT: c_int = 0x04;
const SP_SETPCMARK: c_int = 0x08;
const SP_START: c_int = 0x10;
const SP_SUBPAT: c_int = 0x20;
const SP_END: c_int = 0x40;
const SP_COLUMN: c_int = 0x80;

pub(crate) const kOptCpoptions: OptIndex = 61;

pub(crate) const kMTUnknown: c_int = -1;
pub(crate) const kMTCharWise: c_int = 0;
pub(crate) const kMTLineWise: c_int = 1;
pub(crate) const kMTBlockWise: c_int = 2;

pub(crate) const HLF_SPB: hlf_T = 37;
pub(crate) const HLF_SPC: hlf_T = 38;
pub(crate) const HLF_SPR: hlf_T = 39;
pub(crate) const HLF_SPL: hlf_T = 40;

pub(crate) const HL_INVERSE: c_int = 0x01;
pub(crate) const HL_BOLD: c_int = 0x02;
pub(crate) const HL_ITALIC: c_int = 0x04;
pub(crate) const HL_UNDERLINE: c_int = 0x08;
pub(crate) const HL_UNDERCURL: c_int = 0x10;
pub(crate) const HL_UNDERDOUBLE: c_int = 0x18;
pub(crate) const HL_UNDERDOTTED: c_int = 0x20;
pub(crate) const HL_UNDERDASHED: c_int = 0x28;
pub(crate) const HL_STANDOUT: c_int = 0x0040;
pub(crate) const HL_DIM: c_int = 0x0200;
pub(crate) const HL_NOCOMBINE: c_int = 0x0400;
pub(crate) const HL_BLINK: c_int = 0x8000;
pub(crate) const HL_CONCEALED: c_int = 0x10000;

const E_INVALID_SUBMATCH_NUMBER_NR: &CStr = c"E935: Invalid submatch number: %d";
pub(crate) const E_STRING_LIST_OR_BLOB_REQUIRED: &CStr = c"E1098: String, List or Blob required";
const E_MISSING_FUNCTION_ARGUMENT: &CStr = c"E1132: Missing function argument";
const E_INVACT2: &CStr = c"E962: Invalid action: '%s'";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SomeMatchType {
    kSomeMatch,
    kSomeMatchEnd,
    kSomeMatchList,
    kSomeMatchStr,
    kSomeMatchStrPos,
}

#[repr(C)]
pub struct GetListLineCookie {
    l: *const list_T,
    li: *const listitem_T,
}

static mut INPUTSECRET_FLAG: bool = false;
static mut GA_USERINPUT: Vec<tasave_T> = Vec::new();
static mut HAS_WSL: TriState = kNone;
static mut RAND_GX: u32 = 0;
static mut RAND_GY: u32 = 0;
static mut RAND_GZ: u32 = 0;
static mut RAND_GW: u32 = 0;
static mut RAND_INITIALIZED: bool = false;
static mut GET_FUNC_NAME_INTIDX: c_int = -1;
static mut GET_EXPR_NAME_INTIDX: c_int = -1;

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
unsafe fn tv_list_uidx(l: *const list_T, mut n: c_int) -> c_int {
    unsafe {
        if n < 0 {
            n += tv_list_len(l);
        }
        if n < 0 || n >= tv_list_len(l) { -1 } else { n }
    }
}

#[inline]
unsafe fn tv_get_float_chk(tv: *const typval_T, ret_f: *mut float_T) -> bool {
    unsafe {
        if (*tv).v_type == VarType::VAR_FLOAT {
            *ret_f = (*tv).vval.v_float;
            return true;
        }
        if (*tv).v_type == VarType::VAR_NUMBER {
            *ret_f = (*tv).vval.v_number as float_T;
            return true;
        }
        semsg!(
            c"%s".as_ptr(),
            gettext(c"E808: Number or Float required".as_ptr()),
        );
        false
    }
}

#[inline]
unsafe fn proc_is_stopped(proc_: *mut Proc) -> bool {
    unsafe { (*proc_).status >= 0 || (*proc_).stopped_time != 0 }
}

/// Function given to `ExpandGeneric()` to obtain the list of internal
/// or user defined function names.
pub unsafe fn get_function_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            GET_FUNC_NAME_INTIDX = -1;
        }
        if GET_FUNC_NAME_INTIDX < 0 {
            let name = get_user_func_name(xp, idx);
            if !name.is_null() {
                if *name != NUL
                    && *name != b'<' as c_char
                    && strncmp(c"g:".as_ptr(), (*xp).xp_pattern, 2) == 0
                {
                    return cat_prefix_varname(b'g' as c_int, name);
                }
                return name;
            }
        }

        GET_FUNC_NAME_INTIDX += 1;
        let fdef = eval_func_def_at(GET_FUNC_NAME_INTIDX);
        let key = (*fdef).name;
        if key.is_null() {
            return ptr::null_mut();
        }
        let key_len = strlen(key);
        let iobuff = (&raw mut IObuff).cast::<c_char>();
        ptr::copy_nonoverlapping(key, iobuff, key_len);
        *iobuff.add(key_len) = b'(' as c_char;
        if (*fdef).max_argc == 0 {
            *iobuff.add(key_len + 1) = b')' as c_char;
            *iobuff.add(key_len + 2) = NUL;
        } else {
            *iobuff.add(key_len + 1) = NUL;
        }
        iobuff
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of internal or
/// user defined variable or function names.
pub unsafe fn get_expr_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            GET_EXPR_NAME_INTIDX = -1;
        }
        if GET_EXPR_NAME_INTIDX < 0 {
            let name = get_function_name(xp, idx);
            if !name.is_null() {
                return name;
            }
        }
        GET_EXPR_NAME_INTIDX += 1;
        get_user_var_name(xp, GET_EXPR_NAME_INTIDX)
    }
}

const INTERNAL_FUNC_NAME_MAP: &[(&[u8], c_int)] = &[
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
pub unsafe fn find_internal_func_hash(str_: *const c_char, len: usize) -> c_int {
    if str_.is_null() {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts(str_.cast::<u8>(), len) };
    match INTERNAL_FUNC_NAME_MAP.binary_search_by_key(&slice, |&(k, _)| k) {
        Ok(i) => INTERNAL_FUNC_NAME_MAP[i].1,
        Err(_) => -1,
    }
}

/// Gets a builtin (aka "vimfn", "eval") function from the generated hash table.
pub unsafe fn find_internal_func(name: *const c_char) -> *const EvalFuncDef {
    unsafe {
        let len = strlen(name);
        let index = find_internal_func_hash(name, len);
        if index >= 0 {
            eval_func_def_at(index)
        } else {
            ptr::null()
        }
    }
}

/// Gets the Lua name of a Lua-implemented "vimfn" function, or NULL if not found.
pub unsafe fn find_internal_func_lua(name: *const c_char) -> *const c_char {
    unsafe {
        let fdef = find_internal_func(name);
        if !fdef.is_null()
            && let EvalFuncData::LuaFunc(lua_name) = (*fdef).data
        {
            return lua_name.as_ptr();
        }
        ptr::null()
    }
}

/// Check the argument count to use for internal function "fdef".
pub unsafe fn check_internal_func(fdef: *const EvalFuncDef, argcount: c_int) -> c_int {
    unsafe {
        let res = if argcount < c_int::from((*fdef).min_argc) {
            FCERR_TOOFEW
        } else if argcount > c_int::from((*fdef).max_argc) {
            FCERR_TOOMANY
        } else {
            return c_int::from((*fdef).base_arg);
        };

        let name = (*fdef).name;
        if res == FCERR_TOOMANY {
            semsg!(gettext(e_toomanyarg.as_ptr()), name);
        } else {
            semsg!(gettext(e_toofewarg.as_ptr()), name);
        }
        -1
    }
}

pub unsafe fn call_internal_func(
    fname: *const c_char,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let fdef = find_internal_func(fname);
        if fdef.is_null() {
            return FCERR_UNKNOWN;
        } else if argcount < c_int::from((*fdef).min_argc) {
            return FCERR_TOOFEW;
        } else if argcount > c_int::from((*fdef).max_argc) {
            return FCERR_TOOMANY;
        }
        (*argvars.add(argcount as usize)).v_type = VarType::VAR_UNKNOWN;
        if let Some(func) = (*fdef).func {
            func(argvars, rettv, (*fdef).data);
        }
        FCERR_NONE
    }
}

/// Invoke a method for `base->method()`.
pub unsafe fn call_internal_method(
    fname: *const c_char,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    basetv: *mut typval_T,
) -> c_int {
    unsafe {
        let fdef = find_internal_func(fname);
        if fdef.is_null() {
            return FCERR_UNKNOWN;
        } else if (*fdef).base_arg == BASE_NONE {
            return FCERR_NOTMETHOD;
        } else if argcount + 1 < c_int::from((*fdef).min_argc) {
            return FCERR_TOOFEW;
        } else if argcount + 1 > c_int::from((*fdef).max_argc) {
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
            argvars.add(base_index as usize),
            argv.as_mut_ptr().add((base_index + 1) as usize),
            (argcount as isize - base_index) as usize,
        );
        argv[(argcount + 1) as usize].v_type = VarType::VAR_UNKNOWN;

        if let Some(func) = (*fdef).func {
            func(argv.as_mut_ptr(), rettv, (*fdef).data);
        }
        FCERR_NONE
    }
}

/// Returns true for a non-zero Number and a non-empty String.
#[inline]
unsafe fn non_zero_arg(argvars: *const typval_T) -> bool {
    unsafe {
        ((*argvars).v_type == VarType::VAR_NUMBER && (*argvars).vval.v_number != 0)
            || ((*argvars).v_type == VarType::VAR_BOOL
                && (*argvars).vval.v_bool as c_int == kBoolVarTrue)
            || ((*argvars).v_type == VarType::VAR_STRING
                && !(*argvars).vval.v_string.is_null()
                && *(*argvars).vval.v_string != NUL)
    }
}

/// Apply a floating point C function on a typval with one `float_T`.
pub unsafe fn float_op_wrapper(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData) {
    unsafe {
        let mut f: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut f)
            && let EvalFuncData::FloatOp(func_float) = fptr
        {
            (*rettv).vval.v_float = func_float(f);
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// Invokes an API (`nvim_`) function from Vimscript.
pub unsafe fn api_wrapper(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }

        let EvalFuncData::ApiHandler(handler) = fptr else {
            return;
        };
        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
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

        let mut tv = argvars;
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, false);
            args.size += 1;
            tv = tv.add(1);
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let fn_ptr = handler.r#fn.unwrap();
        let mut result = fn_ptr(VIML_INTERNAL_CALL, args, &raw mut arena, &raw mut err);

        if err.err_type != ErrorType::kErrorTypeNone {
            semsg_multiline!(c"emsg".as_ptr(), e_api_error.as_ptr(), err.msg);
        } else {
            object_to_vim_take_luaref(&raw mut result, rettv, true, &raw mut err);
        }

        if handler.ret_alloc {
            api_free_object(result);
        }
        arena_mem_free(arena_finish(&raw mut arena));
        api_clear_error(&raw mut err);
    }
}

/// Invokes a Lua-implemented vimfn/"f_xx" function from Vimscript.
pub unsafe fn lua_wrapper(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData) {
    unsafe {
        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
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

        let mut tv = argvars;
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, false);
            args.size += 1;
            tv = tv.add(1);
        }

        let EvalFuncData::LuaFunc(lua_name) = fptr else {
            return;
        };
        let mut buf = [0 as c_char; 256];
        vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"return require('vim._core.vimfn').%s(...)".as_ptr(),
            lua_name.as_ptr(),
        );

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut result = nlua_exec(
            cstr_as_string(buf.as_ptr()),
            ptr::null(),
            args,
            kRetObject,
            &raw mut arena,
            &raw mut err,
        );

        if err.err_type != ErrorType::kErrorTypeNone {
            semsg_multiline!(c"emsg".as_ptr(), e_api_error.as_ptr(), err.msg);
        } else {
            object_to_vim_take_luaref(&raw mut result, rettv, true, &raw mut err);
        }

        arena_mem_free(arena_finish(&raw mut arena));
        api_clear_error(&raw mut err);
    }
}

/// "abs(expr)" function
pub unsafe fn f_abs(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_FLOAT {
            let mut f: float_T = 0.0;
            (*rettv).v_type = VarType::VAR_FLOAT;
            if tv_get_float_chk(argvars, &raw mut f) {
                (*rettv).vval.v_float = f.abs();
            } else {
                (*rettv).vval.v_float = 0.0;
            }
        } else {
            let mut error = false;
            let n = tv_get_number_chk(argvars, &raw mut error);
            if error {
                (*rettv).vval.v_number = -1;
            } else if n > 0 {
                (*rettv).vval.v_number = n;
            } else {
                (*rettv).vval.v_number = n.wrapping_neg();
            }
        }
    }
}

/// "and(expr, expr)" function
pub unsafe fn f_and(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = tv_get_number_chk(argvars, ptr::null_mut())
            & tv_get_number_chk(argvars.add(1), ptr::null_mut());
    }
}

/// "api_info()" function
pub unsafe fn f_api_info(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        object_to_vim(api_metadata(), rettv, ptr::null_mut());
    }
}

/// "atan2()" function
pub unsafe fn f_atan2(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut fx: float_T = 0.0;
        let mut fy: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut fx) && tv_get_float_chk(argvars.add(1), &raw mut fy) {
            (*rettv).vval.v_float = fx.atan2(fy);
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// Get buffer by number or pattern.
pub unsafe fn tv_get_buf(tv: *mut typval_T, curtab_only: c_int) -> *mut buf_T {
    unsafe {
        if (*tv).v_type == VarType::VAR_NUMBER {
            return buflist_findnr((*tv).vval.v_number as c_int);
        }
        if (*tv).v_type != VarType::VAR_STRING {
            return ptr::null_mut();
        }

        let name = (*tv).vval.v_string;
        if name.is_null() || *name == NUL {
            return curbuf;
        }
        if *name == b'$' as c_char && *name.add(1) == NUL {
            return lastbuf;
        }

        let save_magic = p_magic;
        p_magic = 1;
        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let buf = buflist_findnr(buflist_findpat(
            name,
            name.add(strlen(name)),
            true,
            false,
            curtab_only != 0,
        ));

        p_magic = save_magic;
        p_cpo = save_cpo;

        if buf.is_null() {
            return find_buffer(tv);
        }

        buf
    }
}

/// Like `tv_get_buf()` but give an error message if the type is wrong.
pub unsafe fn tv_get_buf_from_arg(tv: *mut typval_T) -> *mut buf_T {
    unsafe {
        if !tv_check_str_or_nr(tv) {
            return ptr::null_mut();
        }
        emsg_off += 1;
        let buf = tv_get_buf(tv, 0);
        emsg_off -= 1;
        buf
    }
}

/// Get the buffer from "arg" and give an error and return NULL if it is not valid.
pub unsafe fn get_buf_arg(arg: *mut typval_T) -> *mut buf_T {
    unsafe {
        emsg_off += 1;
        let buf = tv_get_buf(arg, 0);
        emsg_off -= 1;
        if buf.is_null() {
            semsg!(
                gettext(e_invalid_buffer_name_str.as_ptr()),
                tv_get_string(arg),
            );
        }
        buf
    }
}

/// "byte2line(byte)" function
pub unsafe fn f_byte2line(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut boff = (tv_get_number(argvars) as c_int) - 1;
        if boff < 0 {
            (*rettv).vval.v_number = -1;
        } else {
            (*rettv).vval.v_number =
                ml_find_line_or_offset(curbuf, 0, &raw mut boff, false) as varnumber_T;
        }
    }
}

/// "call(func, arglist [, dict])" function
pub unsafe fn f_call(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_list_arg(argvars, 1) == FAIL {
            return;
        }
        if (*argvars.add(1)).vval.v_list.is_null() {
            return;
        }

        let mut owned = false;
        let mut func: *mut c_char;
        let mut partial: *mut partial_T = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_FUNC {
            func = (*argvars).vval.v_string;
        } else if (*argvars).v_type == VarType::VAR_PARTIAL {
            partial = (*argvars).vval.v_partial;
            func = partial_name(partial);
        } else if nlua_is_table_from_lua(argvars) {
            func = nlua_register_table_as_callable(argvars);
            owned = true;
        } else {
            func = tv_get_string(argvars).cast_mut();
        }

        if func.is_null() || *func == NUL {
            return;
        }
        let mut tofree: *mut c_char = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_STRING {
            let mut p = func;
            tofree = trans_function_name(
                &raw mut p,
                false,
                TFN_INT | TFN_QUIET,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if tofree.is_null() {
                emsg_funcname(e_unknown_function_str.as_ptr(), func);
                return;
            }
            func = tofree;
        }

        'done: {
            let mut selfdict: *mut dict_T = ptr::null_mut();
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                if tv_check_for_dict_arg(argvars, 2) == FAIL {
                    break 'done;
                }
                selfdict = (*argvars.add(2)).vval.v_dict.cast();
            }

            func_call(func, argvars.add(1), partial, selfdict, rettv);
        }

        if owned {
            func_unref(func);
        }
        xfree(tofree.cast());
    }
}

/// "changenr()" function
pub unsafe fn f_changenr(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from((*curbuf).b_u_seq_cur);
    }
}

/// "chanclose(id[, stream])" function
pub unsafe fn f_chanclose(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        if (*argvars).v_type != VarType::VAR_NUMBER
            || ((*argvars.add(1)).v_type != VarType::VAR_STRING
                && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN)
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut part = kChannelPartAll;
        if (*argvars.add(1)).v_type == VarType::VAR_STRING {
            let stream = tv_get_string(argvars.add(1));
            if strcmp(stream, c"stdin".as_ptr()) == 0 {
                part = kChannelPartStdin;
            } else if strcmp(stream, c"stdout".as_ptr()) == 0 {
                part = kChannelPartStdout;
            } else if strcmp(stream, c"stderr".as_ptr()) == 0 {
                part = kChannelPartStderr;
            } else if strcmp(stream, c"rpc".as_ptr()) == 0 {
                part = kChannelPartRpc;
            } else {
                semsg!(gettext(c"Invalid channel stream \"%s\"".as_ptr()), stream);
                return;
            }
        }
        let mut error: *const c_char = ptr::null();
        (*rettv).vval.v_number = varnumber_T::from(channel_close(
            (*argvars).vval.v_number as u64,
            part,
            &raw mut error,
        ));
        if (*rettv).vval.v_number == 0 {
            emsg(error);
        }
    }
}

/// "chansend(id, data)" function
pub unsafe fn f_chansend(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        if (*argvars).v_type != VarType::VAR_NUMBER
            || (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut input_len: isize = 0;
        let mut input: *mut c_char = ptr::null_mut();
        let id = (*argvars).vval.v_number as u64;
        #[cfg(not(windows))]
        let crlf = false;
        #[cfg(windows)]
        let crlf = {
            let chan = find_channel(id);
            !chan.is_null() && !(*chan).term.is_null()
        };

        if (*argvars.add(1)).v_type == VarType::VAR_BLOB {
            let b = (*argvars.add(1)).vval.v_blob.cast::<blob_T>();
            input_len = tv_blob_len(b) as isize;
            if input_len > 0 {
                input = xmemdup((*b).bv_ga.as_ptr().cast(), input_len as usize).cast();
            }
        } else {
            input = save_tv_as_string(argvars.add(1), &raw mut input_len, false, crlf);
        }

        if input.is_null() {
            return;
        }
        let mut error: *const c_char = ptr::null();
        (*rettv).vval.v_number =
            channel_send(id, input, input_len as usize, true, &raw mut error) as varnumber_T;
        if !error.is_null() {
            emsg(error);
        }
    }
}

/// "char2nr(string)" function
pub unsafe fn f_char2nr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN && !tv_check_num(argvars.add(1)) {
            return;
        }
        (*rettv).vval.v_number = varnumber_T::from(utf_ptr2char(tv_get_string(argvars)));
    }
}

/// Get the current cursor column and store it in `rettv`.
unsafe fn get_col(argvars: *mut typval_T, rettv: *mut typval_T, charcol: bool) {
    unsafe {
        if tv_check_for_string_or_list_arg(argvars, 0) == FAIL
            || tv_check_for_opt_number_arg(argvars, 1) == FAIL
        {
            return;
        }

        let mut wp = curwin;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let mut tp: *mut tabpage_T = ptr::null_mut();
            wp = win_id2wp_tp(tv_get_number(argvars.add(1)) as c_int, &raw mut tp);
            if wp.is_null() || tp.is_null() {
                return;
            }
            check_cursor(wp);
        }

        let bp = (*wp).w_buffer;
        let mut col: colnr_T = 0;
        let mut fnum = (*bp).b_fnum;
        let fp = var2fpos(argvars, false, &raw mut fnum, charcol, wp);
        if !fp.is_null() && fnum == (*bp).b_fnum {
            if (*fp).col == MAXCOL {
                if (*fp).lnum <= (*bp).b_ml.ml_line_count {
                    col = ml_get_buf_len(bp, (*fp).lnum) + 1;
                } else {
                    col = MAXCOL;
                }
            } else {
                col = (*fp).col + 1;
                if virtual_active(wp) && ptr::eq(fp, &raw const (*wp).w_cursor) {
                    let p = ml_get_buf(bp, (*wp).w_cursor.lnum).add((*wp).w_cursor.col as usize);
                    if (*wp).w_cursor.coladd
                        >= win_chartabsize(wp, p, (*wp).w_virtcol - (*wp).w_cursor.coladd)
                        && *p != NUL
                    {
                        let l = utfc_ptr2len(p);
                        if *p.add(l as usize) == NUL {
                            col += l;
                        }
                    }
                }
            }
        }
        (*rettv).vval.v_number = varnumber_T::from(col);
    }
}

/// "charcol()" function
pub unsafe fn f_charcol(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        get_col(argvars, rettv, true);
    }
}

pub unsafe fn get_optional_window(argvars: *mut typval_T, idx: c_int) -> *mut win_T {
    unsafe {
        if (*argvars.add(idx as usize)).v_type == VarType::VAR_UNKNOWN {
            return curwin;
        }
        let win = find_win_by_nr_or_id(argvars.add(idx as usize));
        if win.is_null() {
            emsg(gettext(e_invalwindow.as_ptr()));
            return ptr::null_mut();
        }
        win
    }
}

/// "col(string)" function
pub unsafe fn f_col(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        get_col(argvars, rettv, false);
    }
}

/// "confirm(message, buttons[, default [, type]])" function
pub unsafe fn f_confirm(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let mut buttons: *const c_char = ptr::null();
        let mut def: c_int = 1;
        let mut type_ = VIM_GENERIC;
        let mut error = false;

        let message = tv_get_string_chk(argvars);
        if message.is_null() {
            error = true;
        }
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            buttons = tv_get_string_buf_chk(argvars.add(1), buf.as_mut_ptr());
            if buttons.is_null() {
                error = true;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                def = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    let typestr = tv_get_string_buf_chk(argvars.add(3), buf2.as_mut_ptr());
                    if typestr.is_null() {
                        error = true;
                    } else {
                        match toupper_asc(c_int::from(*typestr as u8)) as u8 {
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

        if buttons.is_null() || *buttons == NUL {
            buttons = gettext(c"&Ok".as_ptr());
        }

        if !error {
            (*rettv).vval.v_number = varnumber_T::from(do_dialog(
                type_,
                ptr::null(),
                message,
                buttons,
                def,
                ptr::null(),
                0,
            ));
        }
    }
}

/// "copy()" function
pub unsafe fn f_copy(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        var_item_copy(ptr::null_mut(), argvars, rettv, false, 0);
    }
}

/// Set the cursor position.
unsafe fn set_cursorpos(argvars: *mut typval_T, rettv: *mut typval_T, charcol: bool) {
    unsafe {
        let lnum: linenr_T;
        let mut col: colnr_T;
        let mut coladd: colnr_T = 0;
        let mut set_curswant = true;

        (*rettv).vval.v_number = -1;
        if (*argvars).v_type == VarType::VAR_LIST {
            let mut pos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut curswant: colnr_T = -1;
            if list2fpos(
                argvars,
                &raw mut pos,
                ptr::null_mut(),
                &raw mut curswant,
                charcol,
            ) == FAIL
            {
                emsg(gettext(e_invarg.as_ptr()));
                return;
            }
            lnum = pos.lnum;
            col = pos.col;
            coladd = pos.coladd;
            if curswant >= 0 {
                (*curwin).w_curswant = curswant - 1;
                set_curswant = false;
            }
        } else if ((*argvars).v_type == VarType::VAR_NUMBER
            || (*argvars).v_type == VarType::VAR_STRING)
            && ((*argvars.add(1)).v_type == VarType::VAR_NUMBER
                || (*argvars.add(1)).v_type == VarType::VAR_STRING)
        {
            let mut ln = tv_get_lnum(argvars);
            if ln < 0 {
                semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
            } else if ln == 0 {
                ln = (*curwin).w_cursor.lnum;
            }
            lnum = ln;
            col = tv_get_number_chk(argvars.add(1), ptr::null_mut()) as colnr_T;
            if charcol {
                col = buf_charidx_to_byteidx(curbuf, lnum, col) + 1;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                coladd = tv_get_number_chk(argvars.add(2), ptr::null_mut()) as colnr_T;
            }
        } else {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }
        if lnum < 0 || col < 0 || coladd < 0 {
            return;
        }
        if lnum > 0 {
            (*curwin).w_cursor.lnum = lnum;
        }
        if col != MAXCOL {
            col -= 1;
            if col < 0 {
                col = 0;
            }
        }
        (*curwin).w_cursor.col = col;
        (*curwin).w_cursor.coladd = coladd;

        check_cursor(curwin);
        mb_adjust_cursor();

        (*curwin).w_set_curswant = set_curswant;
        (*rettv).vval.v_number = 0;
    }
}

/// "cursor(lnum, col)" or "cursor(list)" function
pub unsafe fn f_cursor(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        set_cursorpos(argvars, rettv, false);
    }
}

/// "debugbreak()" function
pub unsafe fn f_debugbreak(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(FAIL);
        let pid = tv_get_number(argvars) as c_int;
        if pid == 0 {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        #[cfg(windows)]
        {
            let h_process = OpenProcess(PROCESS_ALL_ACCESS, 0, pid as u32);
            if h_process.is_null() {
                return;
            }
            DebugBreakProcess(h_process);
            CloseHandle(h_process);
            (*rettv).vval.v_number = varnumber_T::from(OK);
        }
        #[cfg(not(windows))]
        {
            uv_kill(pid, crate::os::signal::SIGINT);
        }
    }
}

/// "deepcopy()" function
pub unsafe fn f_deepcopy(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_opt_bool_arg(argvars, 1) == FAIL {
            return;
        }
        let mut noref: varnumber_T = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            noref = tv_get_bool_chk(argvars.add(1), ptr::null_mut());
        }
        var_item_copy(
            ptr::null_mut(),
            argvars,
            rettv,
            true,
            if noref == 0 { get_copyID() } else { 0 },
        );
    }
}

/// `dictwatcheradd(dict, key, funcref)` function
pub unsafe fn f_dictwatcheradd(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_DICT {
            semsg!(gettext(e_invarg2.as_ptr()), c"dict".as_ptr());
            return;
        } else if (*argvars).vval.v_dict.is_null() {
            let arg_errmsg = gettext(c"dictwatcheradd() argument".as_ptr());
            let arg_errmsg_len = strlen(arg_errmsg);
            semsg!(
                gettext(e_cannot_change_readonly_variable_str.as_ptr()),
                arg_errmsg_len as c_int,
                arg_errmsg,
            );
            return;
        }

        if (*argvars.add(1)).v_type != VarType::VAR_STRING
            && (*argvars.add(1)).v_type != VarType::VAR_NUMBER
        {
            semsg!(gettext(e_invarg2.as_ptr()), c"key".as_ptr());
            return;
        }

        let key_pattern = tv_get_string_chk(argvars.add(1));
        if key_pattern.is_null() {
            return;
        }
        let key_pattern_len = strlen(key_pattern);

        let mut callback = Callback::NONE;
        if !callback_from_typval(&raw mut callback, argvars.add(2)) {
            semsg!(gettext(e_invarg2.as_ptr()), c"funcref".as_ptr());
            return;
        }

        tv_dict_watcher_add(
            (*argvars).vval.v_dict.cast(),
            key_pattern,
            key_pattern_len,
            callback,
        );
    }
}

/// `dictwatcherdel(dict, key, funcref)` function
pub unsafe fn f_dictwatcherdel(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_DICT {
            semsg!(gettext(e_invarg2.as_ptr()), c"dict".as_ptr());
            return;
        }
        if (*argvars.add(2)).v_type != VarType::VAR_FUNC
            && (*argvars.add(2)).v_type != VarType::VAR_STRING
        {
            semsg!(gettext(e_invarg2.as_ptr()), c"funcref".as_ptr());
            return;
        }

        let key_pattern = tv_get_string_chk(argvars.add(1));
        if key_pattern.is_null() {
            return;
        }

        let mut callback = Callback::NONE;
        if !callback_from_typval(&raw mut callback, argvars.add(2)) {
            return;
        }

        if !tv_dict_watcher_remove(
            (*argvars).vval.v_dict.cast(),
            key_pattern,
            strlen(key_pattern),
            callback,
        ) {
            emsg(c"Couldn't find a watcher matching key and callback".as_ptr());
        }

        callback_free(&raw mut callback);
    }
}

/// "did_filetype()" function
pub unsafe fn f_did_filetype(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from((*curbuf).b_did_filetype);
    }
}

/// "empty({expr})" function
pub unsafe fn f_empty(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut n = true;
        match (*argvars).v_type {
            VarType::VAR_STRING | VarType::VAR_FUNC => {
                n = (*argvars).vval.v_string.is_null() || *(*argvars).vval.v_string == NUL;
            }
            VarType::VAR_PARTIAL => {
                n = (*argvars).vval.v_partial.is_null();
            }
            VarType::VAR_NUMBER => {
                n = (*argvars).vval.v_number == 0;
            }
            VarType::VAR_FLOAT => {
                n = (*argvars).vval.v_float == 0.0;
            }
            VarType::VAR_LIST => {
                n = tv_list_len((*argvars).vval.v_list) == 0;
            }
            VarType::VAR_DICT => {
                n = tv_dict_len((*argvars).vval.v_dict.cast()) == 0;
            }
            VarType::VAR_BOOL => {
                n = (*argvars).vval.v_bool as c_int != kBoolVarTrue;
            }
            VarType::VAR_SPECIAL => {
                n = (*argvars).vval.v_special as c_int == kSpecialVarNull;
            }
            VarType::VAR_BLOB => {
                n = tv_blob_len((*argvars).vval.v_blob.cast()) == 0;
            }
            VarType::VAR_UNKNOWN => {
                internal_error(c"f_empty(UNKNOWN)".as_ptr());
            }
        }
        (*rettv).vval.v_number = varnumber_T::from(n);
    }
}

/// "escape({string}, {chars})" function
pub unsafe fn f_escape(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        (*rettv).vval.v_string = vim_strsave_escaped(
            tv_get_string(argvars),
            tv_get_string_buf(argvars.add(1), buf.as_mut_ptr()),
        );
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "getenv()" function
pub unsafe fn f_getenv(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let p = vim_getenv(tv_get_string(argvars));
        if p.is_null() {
            (*rettv).v_type = VarType::VAR_SPECIAL;
            (*rettv).vval.v_special = kSpecialVarNull;
            return;
        }
        (*rettv).vval.v_string = p;
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "eval()" function
pub unsafe fn f_eval(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut s = tv_get_string_chk(argvars).cast_mut();
        if !s.is_null() {
            s = skipwhite(s);
        }
        let expr_start = s;
        if s.is_null() || eval1(&raw mut s, rettv, &raw mut EVALARG_EVALUATE) == FAIL {
            if !expr_start.is_null() && !aborting() {
                semsg!(gettext(e_invexpr2.as_ptr()), expr_start);
            }
            need_clr_eos = false;
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = 0;
        } else if *s != NUL {
            semsg!(gettext(e_trailing_arg.as_ptr()), s);
        }
    }
}

/// "eventhandler()" function
pub unsafe fn f_eventhandler(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(vgetc_busy);
    }
}

pub unsafe fn get_list_line(
    _c: c_int,
    p: *mut GetListLineCookie,
    _indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe {
        let item = (*p).li;
        if item.is_null() {
            return ptr::null_mut();
        }
        let mut buf = [0 as c_char; NUMBUFLEN];
        let s = tv_get_string_buf_chk(&raw const (*item).li_tv, buf.as_mut_ptr());
        (*p).li = (*item).li_next;
        if s.is_null() {
            ptr::null_mut()
        } else {
            xstrdup(s)
        }
    }
}

pub unsafe fn execute_common(argvars: *mut typval_T, rettv: *mut typval_T, arg_off: c_int) {
    unsafe {
        let save_msg_silent = msg_silent;
        let save_emsg_silent = emsg_silent;
        let save_emsg_noredir = emsg_noredir;
        let save_redir_off = redir_off;
        let save_capture_ga = capture_ga;
        let save_msg_col = msg_col;
        let mut echo_output = false;

        if check_secure() {
            return;
        }

        let off = arg_off as usize;
        if (*argvars.add(off + 1)).v_type != VarType::VAR_UNKNOWN {
            let mut buf = [0 as c_char; NUMBUFLEN];
            let s = tv_get_string_buf_chk(argvars.add(off + 1), buf.as_mut_ptr());
            if s.is_null() {
                return;
            }
            if *s == NUL {
                echo_output = true;
            }
            if strncmp(s, c"silent".as_ptr(), 6) == 0 {
                msg_silent += 1;
            }
            if strcmp(s, c"silent!".as_ptr()) == 0 {
                emsg_silent = 1;
                emsg_noredir = true;
            }
        } else {
            msg_silent += 1;
        }

        let mut capture_local: Vec<c_char> = Vec::new();
        capture_ga = &raw mut capture_local;
        redir_off = false;
        if !echo_output {
            msg_col = 0;
        }

        if (*argvars.add(off)).v_type != VarType::VAR_LIST {
            do_cmdline_cmd(tv_get_string(argvars.add(off)));
        } else if !(*argvars.add(off)).vval.v_list.is_null() {
            let list = (*argvars.add(off)).vval.v_list;
            tv_list_ref(list);
            let mut cookie = GetListLineCookie {
                l: list,
                li: tv_list_first(list),
            };
            do_cmdline(
                ptr::null_mut(),
                crate::cmdexpand::LineGetter::List(&raw mut cookie),
                DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT | DOCMD_KEYTYPED,
            );
            tv_list_unref(list);
        }
        msg_silent = save_msg_silent;
        emsg_silent = save_emsg_silent;
        emsg_noredir = save_emsg_noredir;
        redir_off = save_redir_off;
        if echo_output {
            msg_col = 0;
        } else {
            msg_col = save_msg_col;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(capture_local.as_ptr().cast(), capture_local.len()).cast();

        capture_ga = save_capture_ga;
    }
}

/// "execute(command)" function
pub unsafe fn f_execute(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        execute_common(argvars, rettv, 0);
    }
}

/// "exists()" function
pub unsafe fn f_exists(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut n: c_int = 0;
        let mut p = tv_get_string(argvars);
        if *p == b'$' as c_char {
            if os_env_exists(p.add(1), false) {
                n = 1;
            } else {
                let exp = expand_env_save(p.cast_mut());
                if !exp.is_null() && *exp != b'$' as c_char {
                    n = 1;
                }
                xfree(exp.cast());
            }
        } else if *p == b'&' as c_char || *p == b'+' as c_char {
            n = c_int::from(eval_option(&raw mut p, ptr::null_mut(), true) == OK);
            if *skipwhite(p) != NUL {
                n = 0;
            }
        } else if *p == b'*' as c_char {
            n = c_int::from(if strnequal(p, c"*v:lua.".as_ptr(), 7) {
                nlua_func_exists(p.add(7))
            } else {
                function_exists(p.add(1), false)
            });
        } else if *p == b':' as c_char {
            n = cmd_exists(p.add(1));
        } else if *p == b'#' as c_char {
            if *p.add(1) == b'#' as c_char {
                n = c_int::from(autocmd_supported(p.add(2)));
            } else {
                n = c_int::from(au_exists(p.add(1)));
            }
        } else {
            n = c_int::from(var_exists(p));
        }
        (*rettv).vval.v_number = varnumber_T::from(n);
    }
}

/// "expand()" function
pub unsafe fn f_expand(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut options = WILD_SILENT | WILD_USE_NL | WILD_LIST_NOTFOUND | WILD_USE_SHELLSLASH;
        let mut error = false;

        (*rettv).v_type = VarType::VAR_STRING;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
            && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            && tv_get_bool_chk(argvars.add(2), &raw mut error) != 0
            && !error
        {
            tv_list_set_ret(rettv, ptr::null_mut());
        }

        let s = tv_get_string(argvars);
        if *s == b'%' as c_char || *s == b'#' as c_char || *s == b'<' as c_char {
            if p_verbose == 0 {
                emsg_off += 1;
            }
            let mut len: usize = 0;
            let mut errormsg: *const c_char = ptr::null();
            let result = eval_vars(
                s.cast_mut(),
                s,
                &raw mut len,
                ptr::null_mut(),
                &raw mut errormsg,
                ptr::null_mut(),
                false,
            );
            if p_verbose == 0 {
                emsg_off -= 1;
            } else if !errormsg.is_null() {
                emsg(errormsg);
            }
            if (*rettv).v_type == VarType::VAR_LIST {
                tv_list_alloc_ret(rettv, isize::from(!result.is_null()));
                if !result.is_null() {
                    tv_list_append_string((*rettv).vval.v_list, result, -1);
                }
                xfree(result.cast());
            } else {
                (*rettv).vval.v_string = result;
            }
        } else {
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && tv_get_bool_chk(argvars.add(1), &raw mut error) != 0
            {
                options |= WILD_KEEP_ALL;
            }
            if !error {
                let mut xpc: expand_T = expand_T::new();
                xpc.xp_context = EXPAND_FILES;
                if p_wic != 0 {
                    options += WILD_ICASE;
                }
                if (*rettv).v_type == VarType::VAR_STRING {
                    (*rettv).vval.v_string = ExpandOne(
                        &raw mut xpc,
                        s.cast_mut(),
                        ptr::null_mut(),
                        options,
                        WILD_ALL,
                    );
                } else {
                    ExpandOne(
                        &raw mut xpc,
                        s.cast_mut(),
                        ptr::null_mut(),
                        options,
                        WILD_ALL_KEEP,
                    );
                    tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);
                    for &f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
                        tv_list_append_string((*rettv).vval.v_list, f, -1);
                    }
                }
            } else {
                (*rettv).vval.v_string = ptr::null_mut();
            }
        }
    }
}

/// "menu_get(path [, modes])" function
pub unsafe fn f_menu_get(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        let mut modes = MENU_ALL_MODES;
        if (*argvars.add(1)).v_type == VarType::VAR_STRING {
            let strmodes = tv_get_string(argvars.add(1));
            modes = get_menu_cmd_modes(strmodes, false, ptr::null_mut(), ptr::null_mut());
        }
        menu_get(
            tv_get_string(argvars).cast_mut(),
            modes,
            (*rettv).vval.v_list,
        );
    }
}

/// "expandcmd()" function
pub unsafe fn f_expandcmd(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut errormsg: *const c_char = ptr::null();
        let mut emsgoff = true;

        if (*argvars.add(1)).v_type == VarType::VAR_DICT
            && tv_dict_get_bool(
                (*argvars.add(1)).vval.v_dict.cast(),
                c"errmsg".as_ptr(),
                kBoolVarFalse,
            ) != 0
        {
            emsgoff = false;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        let mut cmdstr = xstrdup(tv_get_string(argvars));

        let mut eap: exarg_T = exarg_T::new();
        eap.cmd = cmdstr;
        eap.arg = cmdstr;
        eap.usefilter = 0;
        eap.nextcmd = ptr::null_mut();
        eap.cmdidx = cmdidx_T::CMD_USER as c_int;
        eap.argt |= EX_NOSPC;

        if emsgoff {
            emsg_off += 1;
        }
        if expand_filename(&raw mut eap, &raw mut cmdstr, &raw mut errormsg) == FAIL
            && !emsgoff
            && !errormsg.is_null()
            && *errormsg != NUL
        {
            emsg(errormsg);
        }
        if emsgoff {
            emsg_off -= 1;
        }

        (*rettv).vval.v_string = cmdstr;
    }
}

/// "flatten()" and "flattennew()" functions
unsafe fn flatten_common(argvars: *mut typval_T, rettv: *mut typval_T, make_copy: bool) {
    unsafe {
        let mut error = false;
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg!(gettext(e_listarg.as_ptr()), c"flatten()".as_ptr());
            return;
        }

        let maxdepth: c_int = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            999999
        } else {
            let md = tv_get_number_chk(argvars.add(1), &raw mut error) as c_int;
            if error {
                return;
            }
            if md < 0 {
                emsg(gettext(
                    c"E900: maxdepth must be non-negative number".as_ptr(),
                ));
                return;
            }
            md
        };

        let mut list = (*argvars).vval.v_list;
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_list = list;
        if list.is_null() {
            return;
        }

        if make_copy {
            list = tv_list_copy(ptr::null_mut(), list, false, get_copyID());
            (*rettv).vval.v_list = list;
            if list.is_null() {
                return;
            }
        } else {
            if value_check_lock(
                tv_list_locked(list),
                c"flatten() argument".as_ptr(),
                TV_TRANSLATE,
            ) {
                return;
            }
            tv_list_ref(list);
        }

        tv_list_flatten(
            list,
            ptr::null_mut(),
            i64::from(tv_list_len(list)),
            i64::from(maxdepth),
        );
    }
}

/// "flatten(list[, {maxdepth}])" function
pub unsafe fn f_flatten(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        flatten_common(argvars, rettv, false);
    }
}

/// "flattennew(list[, {maxdepth}])" function
pub unsafe fn f_flattennew(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        flatten_common(argvars, rettv, true);
    }
}

/// "feedkeys()" function
pub unsafe fn f_feedkeys(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }
        let keys = tv_get_string(argvars);
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut flags: *const c_char = ptr::null();
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            flags = tv_get_string_buf(argvars.add(1), nbuf.as_mut_ptr());
        }
        nvim_feedkeys(cstr_as_string(keys), cstr_as_string(flags), true);
    }
}

/// "float2nr({float})" function
pub unsafe fn f_float2nr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut f: float_T = 0.0;
        if !tv_get_float_chk(argvars, &raw mut f) {
            return;
        }
        if f <= (-VARNUMBER_MAX as float_T) + f64::EPSILON {
            (*rettv).vval.v_number = -VARNUMBER_MAX;
        } else if f >= (VARNUMBER_MAX as float_T) - f64::EPSILON {
            (*rettv).vval.v_number = VARNUMBER_MAX;
        } else {
            (*rettv).vval.v_number = f as varnumber_T;
        }
    }
}

/// "fmod()" function
pub unsafe fn f_fmod(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut fx: float_T = 0.0;
        let mut fy: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut fx) && tv_get_float_chk(argvars.add(1), &raw mut fy) {
            (*rettv).vval.v_float = fx % fy;
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// "fnameescape({string})" function
pub unsafe fn f_fnameescape(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_string = vim_strsave_fnameescape(tv_get_string(argvars), VSE_NONE);
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "foreground()" function
pub unsafe fn f_foreground(_argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {}

/// "function()" and "funcref()" common implementation
unsafe fn common_function(argvars: *mut typval_T, rettv: *mut typval_T, is_funcref: bool) {
    unsafe {
        let mut s: *mut c_char;
        let mut name: *mut c_char;
        let mut use_string = false;
        let mut arg_pt: *mut partial_T = ptr::null_mut();
        let mut trans_name: *mut c_char = ptr::null_mut();

        if (*argvars).v_type == VarType::VAR_FUNC {
            s = (*argvars).vval.v_string;
        } else if (*argvars).v_type == VarType::VAR_PARTIAL && !(*argvars).vval.v_partial.is_null()
        {
            arg_pt = (*argvars).vval.v_partial;
            s = partial_name(arg_pt);
        } else {
            s = tv_get_string(argvars).cast_mut();
            use_string = true;
        }

        if (use_string && vim_strchr(s, AUTOLOAD_CHAR).is_null()) || is_funcref {
            name = s;
            trans_name = save_function_name(
                &raw mut name,
                false,
                TFN_INT | TFN_QUIET | TFN_NO_AUTOLOAD | TFN_NO_DEREF,
                ptr::null_mut(),
            );
            if *name != NUL {
                s = ptr::null_mut();
            }
        }
        'theend: {
            if s.is_null()
                || *s == NUL
                || (use_string && ascii_isdigit(*s))
                || (is_funcref && trans_name.is_null())
            {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    if use_string {
                        tv_get_string(argvars)
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
                semsg!(gettext(c"E700: Unknown function: %s".as_ptr()), s);
            } else {
                let mut dict_idx: usize = 0;
                let mut arg_idx: usize = 0;
                let mut list: *mut list_T = ptr::null_mut();
                if strncmp(s, c"s:".as_ptr(), 2) == 0 || strncmp(s, c"<SID>".as_ptr(), 5) == 0 {
                    name = get_scriptlocal_funcname(s);
                } else {
                    name = xstrdup(s);
                }

                if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                    if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                        arg_idx = 1;
                        dict_idx = 2;
                    } else if (*argvars.add(1)).v_type == VarType::VAR_DICT {
                        dict_idx = 1;
                    } else {
                        arg_idx = 1;
                    }
                    if dict_idx > 0 {
                        if tv_check_for_dict_arg(argvars, dict_idx as c_int) == FAIL {
                            xfree(name.cast());
                            break 'theend;
                        }
                        if (*argvars.add(dict_idx)).vval.v_dict.is_null() {
                            dict_idx = 0;
                        }
                    }
                    if arg_idx > 0 {
                        if (*argvars.add(arg_idx)).v_type != VarType::VAR_LIST {
                            emsg(gettext(
                                c"E923: Second argument of function() must be a list or a dict"
                                    .as_ptr(),
                            ));
                            xfree(name.cast());
                            break 'theend;
                        }
                        list = (*argvars.add(arg_idx)).vval.v_list;
                        if tv_list_len(list) == 0 {
                            arg_idx = 0;
                        } else if (tv_list_len(list) as usize) > MAX_FUNC_ARGS {
                            emsg_funcname(e_toomanyarg.as_ptr(), s);
                            xfree(name.cast());
                            break 'theend;
                        }
                    }
                }
                if dict_idx > 0 || arg_idx > 0 || !arg_pt.is_null() || is_funcref {
                    let pt = Box::into_raw(Box::new(partial_T::default()));
                    if arg_idx > 0 || (!arg_pt.is_null() && !(*arg_pt).pt_argv.is_empty()) {
                        let arg_len = if arg_pt.is_null() {
                            0
                        } else {
                            (*arg_pt).pt_argv.len() as c_int
                        };
                        let lv_len = tv_list_len(list);
                        let pt_argc = arg_len + lv_len;
                        (*pt).pt_argv.resize(pt_argc as usize, typval_T::new());
                        let mut i: c_int = 0;
                        while i < arg_len {
                            tv_copy(
                                (*arg_pt).pt_argv.as_ptr().add(i as usize),
                                (*pt).pt_argv.as_mut_ptr().add(i as usize),
                            );
                            i += 1;
                        }
                        if lv_len > 0 {
                            let mut li = tv_list_first(list);
                            while !li.is_null() {
                                tv_copy(
                                    &raw const (*li).li_tv,
                                    (*pt).pt_argv.as_mut_ptr().add(i as usize),
                                );
                                i += 1;
                                li = (*li).li_next;
                            }
                        }
                    }

                    if dict_idx > 0 {
                        (*pt).pt_dict = (*argvars.add(dict_idx)).vval.v_dict.cast();
                        (*(*pt).pt_dict).dv_refcount += 1;
                    } else if !arg_pt.is_null() {
                        (*pt).pt_dict = (*arg_pt).pt_dict;
                        (*pt).pt_auto = (*arg_pt).pt_auto;
                        if !(*pt).pt_dict.is_null() {
                            (*(*pt).pt_dict).dv_refcount += 1;
                        }
                    }

                    (*pt).pt_refcount = 1;
                    if !arg_pt.is_null() && !(*arg_pt).pt_func.is_null() {
                        (*pt).pt_func = (*arg_pt).pt_func;
                        func_ptr_ref((*pt).pt_func);
                        xfree(name.cast());
                    } else if is_funcref {
                        (*pt).pt_func = find_func(trans_name);
                        func_ptr_ref((*pt).pt_func);
                        xfree(name.cast());
                    } else {
                        (*pt).pt_name = name;
                        func_ref(name);
                    }

                    (*rettv).v_type = VarType::VAR_PARTIAL;
                    (*rettv).vval.v_partial = pt;
                } else {
                    (*rettv).v_type = VarType::VAR_FUNC;
                    (*rettv).vval.v_string = name;
                    func_ref(name);
                }
            }
        }
        xfree(trans_name.cast());
    }
}

pub unsafe fn f_funcref(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        common_function(argvars, rettv, true);
    }
}

pub unsafe fn f_function(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        common_function(argvars, rettv, false);
    }
}

/// "garbagecollect()" function
pub unsafe fn f_garbagecollect(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        want_garbage_collect = true;
        if (*argvars).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars) == 1 {
            garbage_collect_at_exit = true;
        }
    }
}

/// "get()" function
pub unsafe fn f_get(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut tv: *mut typval_T = ptr::null_mut();
        let mut what_is_dict = false;

        if (*argvars).v_type == VarType::VAR_BLOB {
            let mut error = false;
            let mut idx = tv_get_number_chk(argvars.add(1), &raw mut error) as c_int;
            if !error {
                let blob = (*argvars).vval.v_blob.cast::<blob_T>();
                (*rettv).v_type = VarType::VAR_NUMBER;
                if idx < 0 {
                    idx += tv_blob_len(blob);
                }
                if idx < 0 || idx >= tv_blob_len(blob) {
                    (*rettv).vval.v_number = -1;
                } else {
                    (*rettv).vval.v_number = varnumber_T::from(tv_blob_get(blob, idx));
                    tv = rettv;
                }
            }
        } else if (*argvars).v_type == VarType::VAR_LIST {
            let l = (*argvars).vval.v_list;
            if !l.is_null() {
                let mut error = false;
                let li = tv_list_find(
                    l,
                    tv_get_number_chk(argvars.add(1), &raw mut error) as c_int,
                );
                if !error && !li.is_null() {
                    tv = &raw mut (*li).li_tv;
                }
            }
        } else if (*argvars).v_type == VarType::VAR_DICT {
            let d = (*argvars).vval.v_dict.cast::<dict_T>();
            if !d.is_null() {
                let di = tv_dict_find(d, tv_get_string(argvars.add(1)), -1);
                if !di.is_null() {
                    tv = &raw mut (*di).di_tv;
                }
            }
        } else if tv_is_func(*argvars) {
            let mut fref_pt = partial_T::default();
            let pt: *mut partial_T = if (*argvars).v_type == VarType::VAR_PARTIAL {
                (*argvars).vval.v_partial
            } else {
                fref_pt.pt_name = (*argvars).vval.v_string;
                &raw mut fref_pt
            };

            if !pt.is_null() {
                let what = tv_get_string(argvars.add(1));
                if strcmp(what, c"func".as_ptr()) == 0 || strcmp(what, c"name".as_ptr()) == 0 {
                    let mut name = partial_name(pt);
                    (*rettv).v_type = if *what == b'f' as c_char {
                        VarType::VAR_FUNC
                    } else {
                        VarType::VAR_STRING
                    };
                    debug_assert!(!name.is_null());
                    if (*rettv).v_type == VarType::VAR_FUNC {
                        func_ref(name);
                    }
                    if *what == b'n' as c_char
                        && (*pt).pt_name.is_null()
                        && !(*pt).pt_func.is_null()
                    {
                        name = printable_func_name((*pt).pt_func);
                    }
                    (*rettv).vval.v_string = xstrdup(name);
                } else if strcmp(what, c"dict".as_ptr()) == 0 {
                    what_is_dict = true;
                    if !(*pt).pt_dict.is_null() {
                        tv_dict_set_ret(rettv, (*pt).pt_dict);
                    }
                } else if strcmp(what, c"args".as_ptr()) == 0 {
                    (*rettv).v_type = VarType::VAR_LIST;
                    tv_list_alloc_ret(rettv, (*pt).pt_argv.len() as isize);
                    for i in 0..(*pt).pt_argv.len() {
                        tv_list_append_tv((*rettv).vval.v_list, (*pt).pt_argv.as_mut_ptr().add(i));
                    }
                } else if strcmp(what, c"arity".as_ptr()) == 0 {
                    let mut required: c_int = 0;
                    let mut optional: c_int = 0;
                    let mut varargs = false;
                    let name = partial_name(pt);
                    get_func_arity(name, &raw mut required, &raw mut optional, &raw mut varargs);

                    (*rettv).v_type = VarType::VAR_DICT;
                    tv_dict_alloc_ret(rettv);
                    let dict = (*rettv).vval.v_dict.cast::<dict_T>();
                    let pt_argc = (*pt).pt_argv.len() as c_int;

                    if pt_argc >= required + optional {
                        required = 0;
                        optional = 0;
                    } else if pt_argc > required {
                        optional -= pt_argc - required;
                        required = 0;
                    } else {
                        required -= pt_argc;
                    }

                    tv_dict_add_nr(dict, c"required".as_ptr(), 8, varnumber_T::from(required));
                    tv_dict_add_nr(dict, c"optional".as_ptr(), 8, varnumber_T::from(optional));
                    tv_dict_add_bool(
                        dict,
                        c"varargs".as_ptr(),
                        7,
                        if varargs { kBoolVarTrue } else { kBoolVarFalse },
                    );
                } else {
                    semsg!(gettext(e_invarg2.as_ptr()), what);
                }

                if !what_is_dict {
                    return;
                }
            }
        } else {
            semsg!(gettext(e_listdictblobarg.as_ptr()), c"get()".as_ptr());
        }

        if tv.is_null() {
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                tv_copy(argvars.add(2), rettv);
            }
        } else {
            tv_copy(tv, rettv);
        }
    }
}

/// "getchangelist()" function
pub unsafe fn f_getchangelist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);
        let buf: *const buf_T = if (*argvars).v_type == VarType::VAR_UNKNOWN {
            curbuf
        } else {
            vim_ignored = tv_get_number(argvars) as c_int;
            emsg_off += 1;
            let b = tv_get_buf(argvars, 0);
            emsg_off -= 1;
            b
        };
        if buf.is_null() {
            return;
        }

        let l = tv_list_alloc((*buf).b_changelistlen as isize);
        tv_list_append_list((*rettv).vval.v_list, l);
        let mut changelistindex = if ptr::eq(buf, (*curwin).w_buffer) {
            (*curwin).w_changelistidx
        } else {
            let mut idx = (*buf).b_changelistlen;
            for wip in &(*buf).b_wininfo {
                if wip.wi_win == curwin {
                    idx = wip.wi_changelistidx;
                    break;
                }
            }
            idx
        };
        let _ = &mut changelistindex;
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(changelistindex));

        for i in 0..((*buf).b_changelistlen as usize) {
            if (*buf).b_changelist[i].mark.lnum == 0 {
                continue;
            }
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            tv_dict_add_nr(
                d,
                c"lnum".as_ptr(),
                4,
                varnumber_T::from((*buf).b_changelist[i].mark.lnum),
            );
            tv_dict_add_nr(
                d,
                c"col".as_ptr(),
                3,
                varnumber_T::from((*buf).b_changelist[i].mark.col),
            );
            tv_dict_add_nr(
                d,
                c"coladd".as_ptr(),
                6,
                varnumber_T::from((*buf).b_changelist[i].mark.coladd),
            );
        }
    }
}

unsafe fn getpos_both(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    getcurpos: bool,
    charcol: bool,
) {
    unsafe {
        let mut fp: *mut pos_T = ptr::null_mut();
        let mut pos: pos_T;
        let mut wp = curwin;
        let mut fnum: c_int = -1;

        if getcurpos {
            if (*argvars).v_type != VarType::VAR_UNKNOWN {
                wp = find_win_by_nr_or_id(argvars);
                if !wp.is_null() {
                    fp = (&raw mut (*wp).w_cursor).cast();
                }
            } else {
                fp = (&raw mut (*curwin).w_cursor).cast();
            }
            if !fp.is_null() && charcol {
                pos = *fp;
                pos.col = buf_byteidx_to_charidx((*wp).w_buffer, pos.lnum, pos.col);
                fp = &raw mut pos;
            }
        } else {
            fp = var2fpos(argvars, true, &raw mut fnum, charcol, curwin);
        }

        let l = tv_list_alloc_ret(rettv, 4 + isize::from(getcurpos));
        tv_list_append_number(
            l,
            if fnum != -1 {
                varnumber_T::from(fnum)
            } else {
                0
            },
        );
        tv_list_append_number(
            l,
            if !fp.is_null() {
                varnumber_T::from((*fp).lnum)
            } else {
                0
            },
        );
        tv_list_append_number(
            l,
            if !fp.is_null() {
                if (*fp).col == MAXCOL {
                    varnumber_T::from(MAXCOL)
                } else {
                    varnumber_T::from((*fp).col) + 1
                }
            } else {
                0
            },
        );
        tv_list_append_number(
            l,
            if !fp.is_null() {
                varnumber_T::from((*fp).coladd)
            } else {
                0
            },
        );
        if getcurpos {
            let save_set_curswant = (*curwin).w_set_curswant;
            let save_curswant = (*curwin).w_curswant;
            let save_virtcol = (*curwin).w_virtcol;

            if wp == curwin {
                update_curswant();
            }
            tv_list_append_number(
                l,
                if wp.is_null() {
                    0
                } else if (*wp).w_curswant == MAXCOL {
                    varnumber_T::from(MAXCOL)
                } else {
                    varnumber_T::from((*wp).w_curswant) + 1
                },
            );

            if wp == curwin && save_set_curswant {
                (*curwin).w_set_curswant = save_set_curswant;
                (*curwin).w_curswant = save_curswant;
                (*curwin).w_virtcol = save_virtcol;
                (*curwin).w_valid &= !VALID_VIRTCOL;
            }
        }
    }
}

/// "getcharpos()" function
pub unsafe fn f_getcharpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getpos_both(argvars, rettv, false, true);
    }
}

/// "getcharsearch()" function
pub unsafe fn f_getcharsearch(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        let dict = (*rettv).vval.v_dict.cast::<dict_T>();
        tv_dict_add_str(dict, c"char".as_ptr(), 4, last_csearch());
        tv_dict_add_nr(
            dict,
            c"forward".as_ptr(),
            7,
            varnumber_T::from(last_csearch_forward()),
        );
        tv_dict_add_nr(
            dict,
            c"until".as_ptr(),
            5,
            varnumber_T::from(last_csearch_until()),
        );
    }
}

/// "getfontname()" function
pub unsafe fn f_getfontname(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
    }
}

/// "getjumplist()" function
pub unsafe fn f_getjumplist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        let wp = find_tabwin(argvars, argvars.add(1));
        if wp.is_null() {
            return;
        }

        cleanup_jumplist(wp, true);

        let l = tv_list_alloc((*wp).w_jumplistlen as isize);
        tv_list_append_list((*rettv).vval.v_list, l);
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from((*wp).w_jumplistidx));

        for i in 0..((*wp).w_jumplistlen as usize) {
            if (*wp).w_jumplist[i].fmark.mark.lnum == 0 {
                continue;
            }
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            tv_dict_add_nr(
                d,
                c"lnum".as_ptr(),
                4,
                varnumber_T::from((*wp).w_jumplist[i].fmark.mark.lnum),
            );
            tv_dict_add_nr(
                d,
                c"col".as_ptr(),
                3,
                varnumber_T::from((*wp).w_jumplist[i].fmark.mark.col),
            );
            tv_dict_add_nr(
                d,
                c"coladd".as_ptr(),
                6,
                varnumber_T::from((*wp).w_jumplist[i].fmark.mark.coladd),
            );
            tv_dict_add_nr(
                d,
                c"bufnr".as_ptr(),
                5,
                varnumber_T::from((*wp).w_jumplist[i].fmark.fnum),
            );
            if !(*wp).w_jumplist[i].fname.is_null() {
                tv_dict_add_str(d, c"filename".as_ptr(), 8, (*wp).w_jumplist[i].fname);
            }
        }
    }
}

/// "getmarklist()" function
pub unsafe fn f_getmarklist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            get_global_marks((*rettv).vval.v_list);
            return;
        }
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            return;
        }
        get_buf_local_marks(buf, (*rettv).vval.v_list);
    }
}

/// "getpid()" function
pub unsafe fn f_getpid(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = os_get_pid();
    }
}

/// "getcurpos(string)" function
pub unsafe fn f_getcurpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getpos_both(argvars, rettv, true, false);
    }
}

pub unsafe fn f_getcursorcharpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getpos_both(argvars, rettv, true, true);
    }
}

/// "getpos(string)" function
pub unsafe fn f_getpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getpos_both(argvars, rettv, false, false);
    }
}

/// Convert from `block_def` to `NvimString`.
unsafe fn block_def2str(bd: *const block_def) -> NvimString {
    unsafe {
        let startspaces = (*bd).startspaces as usize;
        let endspaces = (*bd).endspaces as usize;
        let textlen = (*bd).textlen as usize;
        let size = startspaces + endspaces + textlen;
        let data = xmalloc(size + 1).cast::<c_char>();

        ptr::write_bytes(data, b' ', startspaces);
        let mut off = startspaces;
        ptr::copy((*bd).textstart, data.add(off), textlen);
        off += textlen;
        ptr::write_bytes(data.add(off), b' ', endspaces);
        off += endspaces;
        *data.add(off) = NUL;

        NvimString { data, size: off }
    }
}

unsafe fn getregionpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    p1: *mut pos_T,
    p2: *mut pos_T,
    inclusive: *mut bool,
    region_type: *mut c_int,
    oap: *mut oparg_T,
) -> c_int {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if tv_check_for_list_arg(argvars, 0) == FAIL
            || tv_check_for_list_arg(argvars, 1) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 2) == FAIL
        {
            return FAIL;
        }

        let mut fnum1: c_int = -1;
        let mut fnum2: c_int = -1;
        if list2fpos(argvars, p1, &raw mut fnum1, ptr::null_mut(), false) != OK
            || list2fpos(argvars.add(1), p2, &raw mut fnum2, ptr::null_mut(), false) != OK
            || fnum1 != fnum2
        {
            return FAIL;
        }

        let is_select_exclusive: bool;
        let mut type_str: *mut c_char;
        let mut default_type = [b'v' as c_char, NUL];
        if (*argvars.add(2)).v_type == VarType::VAR_DICT {
            let d = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
            is_select_exclusive = tv_dict_get_bool(
                d,
                c"exclusive".as_ptr(),
                c_int::from(*p_sel == b'e' as c_char),
            ) != 0;
            type_str = tv_dict_get_string(d, c"type".as_ptr(), false);
            if type_str.is_null() {
                type_str = default_type.as_mut_ptr();
            }
        } else {
            is_select_exclusive = *p_sel == b'e' as c_char;
            type_str = default_type.as_mut_ptr();
        }

        let mut block_width: c_int = 0;
        if *type_str == b'v' as c_char && *type_str.add(1) == NUL {
            *region_type = kMTCharWise;
        } else if *type_str == b'V' as c_char && *type_str.add(1) == NUL {
            *region_type = kMTLineWise;
        } else if *type_str == Ctrl_V {
            let mut p = type_str.add(1);
            if *p != NUL {
                block_width = getdigits_int(&raw mut p, false, 0);
                if block_width <= 0 || *p != NUL {
                    semsg!(gettext(e_invargNval.as_ptr()), c"type".as_ptr(), type_str);
                    return FAIL;
                }
            }
            *region_type = kMTBlockWise;
        } else {
            semsg!(gettext(e_invargNval.as_ptr()), c"type".as_ptr(), type_str);
            return FAIL;
        }

        let findbuf = if fnum1 != 0 {
            buflist_findnr(fnum1)
        } else {
            curbuf
        };
        if findbuf.is_null() || (*findbuf).b_ml.ml_mfp.is_null() {
            emsg(gettext(e_buffer_is_not_loaded.as_ptr()));
            return FAIL;
        }

        if (*p1).lnum < 1 || (*p1).lnum > (*findbuf).b_ml.ml_line_count {
            semsg!(gettext(e_invalid_line_number_nr.as_ptr()), (*p1).lnum);
            return FAIL;
        }
        if (*p1).col == MAXCOL {
            (*p1).col = ml_get_buf_len(findbuf, (*p1).lnum) + 1;
        } else if (*p1).col < 1 || (*p1).col > ml_get_buf_len(findbuf, (*p1).lnum) + 1 {
            semsg!(gettext(e_invalid_column_number_nr.as_ptr()), (*p1).col);
            return FAIL;
        }

        if (*p2).lnum < 1 || (*p2).lnum > (*findbuf).b_ml.ml_line_count {
            semsg!(gettext(e_invalid_line_number_nr.as_ptr()), (*p2).lnum);
            return FAIL;
        }
        if (*p2).col == MAXCOL {
            (*p2).col = ml_get_buf_len(findbuf, (*p2).lnum) + 1;
        } else if (*p2).col < 1 || (*p2).col > ml_get_buf_len(findbuf, (*p2).lnum) + 1 {
            semsg!(gettext(e_invalid_column_number_nr.as_ptr()), (*p2).col);
            return FAIL;
        }

        curbuf = findbuf;
        (*curwin).w_buffer = curbuf;
        virtual_op = TriState::from(virtual_active(curwin));

        (*p1).col -= 1;
        (*p2).col -= 1;

        getregionpos_prep(
            (*p1).cast(),
            (*p2).cast(),
            *region_type,
            is_select_exclusive,
            block_width,
            inclusive,
            oap,
        );
        OK
    }
}

impl pos_T {
    #[inline]
    unsafe fn cast(&mut self) -> *mut crate::cursor::pos_T {
        (self as *mut pos_T).cast()
    }
}

/// "getregion()" function
pub unsafe fn f_getregion(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let save_curbuf = curbuf;
        let save_virtual = virtual_op;

        let mut p1 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut p2 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut region_type = kMTUnknown;
        let mut oa = oparg_T::new();

        if getregionpos(
            argvars,
            rettv,
            &raw mut p1,
            &raw mut p2,
            &raw mut inclusive,
            &raw mut region_type,
            &raw mut oa,
        ) == FAIL
        {
            return;
        }

        for lnum in p1.lnum..=p2.lnum {
            let akt = if region_type == kMTBlockWise {
                let mut bd = block_def::new();
                block_prep(&raw mut oa, &raw mut bd, lnum, false);
                block_def2str(&raw const bd)
            } else if region_type == kMTLineWise || (p1.lnum < lnum && lnum < p2.lnum) {
                cbuf_to_string(ml_get(lnum), ml_get_len(lnum) as usize)
            } else {
                let mut bd = block_def::new();
                charwise_block_prep(
                    crate::cursor::pos_T {
                        lnum: p1.lnum,
                        col: p1.col,
                        coladd: p1.coladd,
                    },
                    crate::cursor::pos_T {
                        lnum: p2.lnum,
                        col: p2.col,
                        coladd: p2.coladd,
                    },
                    &raw mut bd,
                    lnum,
                    inclusive,
                );
                block_def2str(&raw const bd)
            };

            debug_assert!(!akt.data.is_null());
            tv_list_append_allocated_string((*rettv).vval.v_list, akt.data);
        }

        curbuf = save_curbuf;
        (*curwin).w_buffer = curbuf;
        virtual_op = save_virtual;
    }
}

unsafe fn add_regionpos_range(rettv: *mut typval_T, p1: pos_T, p2: pos_T) {
    unsafe {
        let l1 = tv_list_alloc(2);
        tv_list_append_list((*rettv).vval.v_list, l1);

        let l2 = tv_list_alloc(4);
        tv_list_append_list(l1, l2);

        let l3 = tv_list_alloc(4);
        tv_list_append_list(l1, l3);

        tv_list_append_number(l2, varnumber_T::from((*curbuf).b_fnum));
        tv_list_append_number(l2, varnumber_T::from(p1.lnum));
        tv_list_append_number(l2, varnumber_T::from(p1.col));
        tv_list_append_number(l2, varnumber_T::from(p1.coladd));

        tv_list_append_number(l3, varnumber_T::from((*curbuf).b_fnum));
        tv_list_append_number(l3, varnumber_T::from(p2.lnum));
        tv_list_append_number(l3, varnumber_T::from(p2.col));
        tv_list_append_number(l3, varnumber_T::from(p2.coladd));
    }
}

/// "getregionpos()" function
pub unsafe fn f_getregionpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let save_curbuf = curbuf;
        let save_virtual = virtual_op;

        let mut p1 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut p2 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut region_type = kMTUnknown;
        let mut allow_eol = false;
        let mut bounds_only = false;
        let mut oa = oparg_T::new();

        if getregionpos(
            argvars,
            rettv,
            &raw mut p1,
            &raw mut p2,
            &raw mut inclusive,
            &raw mut region_type,
            &raw mut oa,
        ) == FAIL
        {
            return;
        }

        if (*argvars.add(2)).v_type == VarType::VAR_DICT {
            let d = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
            allow_eol = tv_dict_get_bool(d, c"eol".as_ptr(), 0) != 0;
            bounds_only = tv_dict_get_bool(d, c"bounds".as_ptr(), 0) != 0;
        }

        let tab_p1 = crate::cursor::pos_T {
            lnum: p1.lnum,
            col: p1.col,
            coladd: p1.coladd,
        };
        let tab_p2 = crate::cursor::pos_T {
            lnum: p2.lnum,
            col: p2.col,
            coladd: p2.coladd,
        };

        if bounds_only {
            let mut start_pos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut end_pos = pos_T {
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
                &raw mut oa,
                allow_eol,
                start_pos.cast(),
                end_pos.cast(),
            );
            if p2.lnum != p1.lnum {
                let mut unused = pos_T {
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
                    &raw mut oa,
                    allow_eol,
                    unused.cast(),
                    end_pos.cast(),
                );
            }
            add_regionpos_range(rettv, start_pos, end_pos);
        } else {
            for lnum in p1.lnum..=p2.lnum {
                let mut ret_p1 = pos_T {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                let mut ret_p2 = pos_T {
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
                    &raw mut oa,
                    allow_eol,
                    ret_p1.cast(),
                    ret_p2.cast(),
                );
                add_regionpos_range(rettv, ret_p1, ret_p2);
            }
        }

        curbuf = save_curbuf;
        (*curwin).w_buffer = curbuf;
        virtual_op = save_virtual;
    }
}

/// Common between `getreg()`, `getreginfo()` and `getregtype()`.
unsafe fn getreg_get_regname(argvars: *mut typval_T) -> c_int {
    unsafe {
        let strregname = if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let s = tv_get_string_chk(argvars);
            if s.is_null() {
                return 0;
            }
            s
        } else {
            get_vim_var_str(VV_REG)
        };
        if *strregname == 0 {
            b'"' as c_int
        } else {
            c_int::from(*strregname as u8)
        }
    }
}

/// "getreg()" function
pub unsafe fn f_getreg(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut arg2: c_int = 0;
        let mut return_list = false;

        let regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }

        if (*argvars).v_type != VarType::VAR_UNKNOWN
            && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
        {
            let mut error = false;
            arg2 = tv_get_bool_chk(argvars.add(1), &raw mut error) as c_int;
            if !error && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                return_list = tv_get_bool_chk(argvars.add(2), &raw mut error) != 0;
            }
            if error {
                return;
            }
        }

        if return_list {
            (*rettv).v_type = VarType::VAR_LIST;
            (*rettv).vval.v_list = get_reg_contents(
                regname,
                (if arg2 != 0 { kGRegExprSrc } else { 0 }) | kGRegList,
            )
            .cast();
            if (*rettv).vval.v_list.is_null() {
                (*rettv).vval.v_list = tv_list_alloc(0);
            }
            tv_list_ref((*rettv).vval.v_list);
        } else {
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval.v_string =
                get_reg_contents(regname, if arg2 != 0 { kGRegExprSrc } else { 0 }).cast();
        }
    }
}

/// "getregtype()" function
pub unsafe fn f_getregtype(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }

        let mut reglen: colnr_T = 0;
        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        let reg_type = get_reg_type(regname, &raw mut reglen);
        format_reg_type(reg_type, reglen, buf.as_mut_ptr(), buf.len());

        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
    }
}

/// "gettagstack()" function
pub unsafe fn f_gettagstack(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut wp = curwin;
        tv_dict_alloc_ret(rettv);
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            wp = find_win_by_nr_or_id(argvars);
            if wp.is_null() {
                return;
            }
        }
        get_tagstack(wp, (*rettv).vval.v_dict.cast());
    }
}

unsafe fn dummy_timer_due_cb(tw: *mut TimeWatcher, _data: *mut crate::eval::timer_T) {
    unsafe {
        if *loop_closing_ptr((&raw mut main_loop).cast()) {
            time_watcher_stop(tw);
            time_watcher_close(tw, Some(dummy_timer_close_cb));
        }
    }
}

unsafe fn dummy_timer_close_cb(tw: *mut TimeWatcher, _data: *mut crate::eval::timer_T) {
    unsafe {
        drop(Box::from_raw(tw));
    }
}

/// "wait(timeout, condition[, interval])" function
pub unsafe fn f_wait(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = -1;

        if (*argvars).v_type != VarType::VAR_NUMBER {
            semsg!(gettext(e_invargval.as_ptr()), c"1".as_ptr());
            return;
        }
        if ((*argvars.add(2)).v_type != VarType::VAR_NUMBER
            && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN)
            || ((*argvars.add(2)).v_type == VarType::VAR_NUMBER
                && (*argvars.add(2)).vval.v_number <= 0)
        {
            semsg!(gettext(e_invargval.as_ptr()), c"3".as_ptr());
            return;
        }

        let timeout = (*argvars).vval.v_number as c_int;
        let mut expr = *argvars.add(1);
        let interval = if (*argvars.add(2)).v_type == VarType::VAR_NUMBER {
            (*argvars.add(2)).vval.v_number as c_int
        } else {
            200
        };
        let tw = Box::into_raw(Box::new(TimeWatcher::new()));

        let main_loop_ptr: *mut Loop = (&raw mut main_loop).cast();
        time_watcher_init(main_loop_ptr, tw, ptr::null_mut());
        (*tw).events = ptr::null_mut();
        time_watcher_start(
            tw,
            Some(dummy_timer_due_cb),
            interval as u64,
            interval as u64,
        );

        let mut argv = tv_initial_value();
        let mut exprval = tv_initial_value();
        let mut error = false;
        let called_emsg_before = called_emsg;

        ui_flush();

        let mut remaining = i64::from(timeout);
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !(eval_expr_typval(&raw mut expr, false, &raw mut argv, 0, &raw mut exprval) != OK
            || tv_get_number_chk(&raw const exprval, &raw mut error) != 0
            || called_emsg > called_emsg_before
            || error
            || got_int)
        {
            let events = (*loop_queues(main_loop_ptr)).events;
            if !events.is_null() && !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events(main_loop_ptr, remaining);
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

        if called_emsg > called_emsg_before || error {
            (*rettv).vval.v_number = -3;
        } else if got_int {
            got_int = false;
            vgetc();
            (*rettv).vval.v_number = -2;
        } else if tv_get_number_chk(&raw const exprval, &raw mut error) != 0 {
            (*rettv).vval.v_number = 0;
        }

        time_watcher_stop(tw);
        time_watcher_close(tw, Some(dummy_timer_close_cb));
    }
}

/// "gettext()" function
pub unsafe fn f_gettext(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_nonempty_string_arg(argvars, 0) == FAIL {
            return;
        }
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xstrdup(gettext((*argvars).vval.v_string));
    }
}

/// "has()" function
pub unsafe fn f_has(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut x = false;
        let mut n = false;
        let name = tv_get_string(argvars);

        if vim_strnicmp(name, c"patch".as_ptr(), 5) == 0 {
            x = true;
            if *name.add(5) == b'-' as c_char
                && strlen(name) >= 11
                && (*name.add(6) >= b'1' as c_char && *name.add(6) <= b'9' as c_char)
            {
                let mut end: *mut c_char = ptr::null_mut();
                let major = strtoul(name.add(6), &raw mut end, 10) as c_int;
                if *end == b'.' as c_char
                    && ascii_isdigit(*end.add(1))
                    && *end.add(2) == b'.' as c_char
                    && ascii_isdigit(*end.add(3))
                {
                    let minor = atoi(end.add(1));
                    n = has_vim_patch(atoi(end.add(3)), major * 100 + minor);
                }
            } else if ascii_isdigit(*name.add(5)) {
                n = has_vim_patch(atoi(name.add(5)), 0);
            }
        } else if vim_strnicmp(name, c"nvim-".as_ptr(), 5) == 0 {
            x = true;
            n = has_nvim_version(CStr::from_ptr(name.add(5)).to_bytes());
        } else if vim_stricmp(name, c"vim_starting".as_ptr()) == 0 {
            x = true;
            n = starting != 0;
        } else if vim_stricmp(name, c"ttyin".as_ptr()) == 0 {
            x = true;
            n = stdin_isatty;
        } else if vim_stricmp(name, c"ttyout".as_ptr()) == 0 {
            x = true;
            n = stdout_isatty;
        } else if vim_stricmp(name, c"multi_byte_encoding".as_ptr()) == 0 {
            x = true;
            n = true;
        } else if vim_stricmp(name, c"gui_running".as_ptr()) == 0 {
            x = true;
            n = ui_gui_attached();
        } else if vim_stricmp(name, c"syntax_items".as_ptr()) == 0 {
            x = true;
            n = syntax_present(curwin);
        } else if vim_stricmp(name, c"wsl".as_ptr()) == 0 {
            x = true;
            n = has_wsl();
        }

        if !x && eval_has_static_feature(name) {
            x = true;
            n = true;
        }

        if !x {
            let save_shell_error = get_vim_var_nr(VV_SHELL_ERROR) as c_int;
            if vim_stricmp(name, c"clipboard_working".as_ptr()) == 0
                || (cfg!(not(windows)) && vim_stricmp(name, c"unnamedplus".as_ptr()) == 0)
            {
                n = eval_has_provider(c"clipboard".as_ptr(), true);
            } else if vim_stricmp(name, c"pythonx".as_ptr()) == 0 {
                n = eval_has_provider(c"python3".as_ptr(), true);
            } else if eval_has_provider(name, true) {
                n = true;
            }
            set_vim_var_nr(VV_SHELL_ERROR, varnumber_T::from(save_shell_error));
        }

        (*rettv).vval.v_number = varnumber_T::from(n);
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

unsafe fn eval_has_static_feature(name: *const c_char) -> bool {
    unsafe {
        for &feat in EVAL_HAS_LIST {
            if vim_stricmp(name, feat.as_ptr()) == 0 {
                return true;
            }
        }
        let flags = NVIM_BUILD_HAS_FLAGS;
        ((flags & (1 << 0)) != 0 && vim_stricmp(name, c"acl".as_ptr()) == 0)
            || ((flags & (1 << 1)) != 0 && vim_stricmp(name, c"terminfo".as_ptr()) == 0)
            || ((flags & (1 << 2)) != 0 && vim_stricmp(name, c"xattr".as_ptr()) == 0)
            || ((flags & (1 << 3)) != 0 && vim_stricmp(name, c"termux".as_ptr()) == 0)
            || ((flags & (1 << 4)) != 0 && vim_stricmp(name, c"gettext".as_ptr()) == 0)
    }
}

unsafe fn has_wsl() -> bool {
    unsafe {
        if HAS_WSL == kNone {
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
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
                kRetNilBool,
                ptr::null_mut(),
                &raw mut err,
            );
            debug_assert!(err.err_type == ErrorType::kErrorTypeNone);
            HAS_WSL = if luaret_truthy(o) { kTrue } else { kFalse };
        }
        HAS_WSL == kTrue
    }
}

/// "highlightID(name)" function
pub unsafe fn f_hlID(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(syn_name2id(tv_get_string(argvars)));
    }
}

/// "highlight_exists()" function
pub unsafe fn f_hlexists(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(highlight_exists(tv_get_string(argvars)));
    }
}

/// "index()" function
pub unsafe fn f_index(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut idx: c_int = 0;
        let mut ic = false;

        (*rettv).vval.v_number = -1;
        if (*argvars).v_type == VarType::VAR_BLOB {
            let mut error = false;
            let mut start: c_int = 0;
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                start = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
                if error {
                    return;
                }
            }
            let b = (*argvars).vval.v_blob.cast::<blob_T>();
            if b.is_null() {
                return;
            }
            if start < 0 {
                start += tv_blob_len(b);
                if start < 0 {
                    start = 0;
                }
            }
            idx = start;
            while idx < tv_blob_len(b) {
                let mut tv = typval_T {
                    v_type: VarType::VAR_NUMBER,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval {
                        v_number: varnumber_T::from(tv_blob_get(b, idx)),
                    },
                };
                if tv_equal(&raw mut tv, argvars.add(1), ic) {
                    (*rettv).vval.v_number = varnumber_T::from(idx);
                    return;
                }
                idx += 1;
            }
            return;
        } else if (*argvars).v_type != VarType::VAR_LIST {
            emsg(gettext(e_listblobreq.as_ptr()));
            return;
        }

        let l = (*argvars).vval.v_list;
        if l.is_null() {
            return;
        }

        let mut item = tv_list_first(l);
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;
            idx = tv_list_uidx(
                l,
                tv_get_number_chk(argvars.add(2), &raw mut error) as c_int,
            );
            if error || idx == -1 {
                item = ptr::null_mut();
            } else {
                item = tv_list_find(l, idx);
                debug_assert!(!item.is_null());
            }
            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                ic = tv_get_bool_chk(argvars.add(3), &raw mut error) != 0;
                if error {
                    item = ptr::null_mut();
                }
            }
        }

        while !item.is_null() {
            if tv_equal(&raw mut (*item).li_tv, argvars.add(1), ic) {
                (*rettv).vval.v_number = varnumber_T::from(idx);
                break;
            }
            item = (*item).li_next;
            idx += 1;
        }
    }
}

unsafe fn indexof_eval_expr(expr: *mut typval_T) -> varnumber_T {
    unsafe {
        let mut argv = [tv_initial_value(); 3];
        argv[0] = *get_vim_var_tv(VV_KEY);
        argv[1] = *get_vim_var_tv(VV_VAL);
        let mut newtv = tv_initial_value();

        if eval_expr_typval(expr, false, argv.as_mut_ptr(), 2, &raw mut newtv) == FAIL {
            return 0;
        }

        let mut error = false;
        let found = tv_get_bool_chk(&raw const newtv, &raw mut error);
        tv_clear(&raw mut newtv);

        if error { 0 } else { found }
    }
}

unsafe fn indexof_blob(
    b: *mut blob_T,
    mut startidx: varnumber_T,
    expr: *mut typval_T,
) -> varnumber_T {
    unsafe {
        if b.is_null() {
            return -1;
        }
        if startidx < 0 {
            startidx += varnumber_T::from(tv_blob_len(b));
            if startidx < 0 {
                startidx = 0;
            }
        }

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);
        set_vim_var_type(VV_VAL, VarType::VAR_NUMBER);

        let called_emsg_start = called_emsg;
        let mut idx = startidx;
        while idx < varnumber_T::from(tv_blob_len(b)) {
            set_vim_var_nr(VV_KEY, idx);
            set_vim_var_nr(VV_VAL, varnumber_T::from(tv_blob_get(b, idx as c_int)));

            if indexof_eval_expr(expr) != 0 {
                return idx;
            }
            if called_emsg != called_emsg_start {
                return -1;
            }
            idx += 1;
        }
        -1
    }
}

unsafe fn indexof_list(l: *mut list_T, startidx: varnumber_T, expr: *mut typval_T) -> varnumber_T {
    unsafe {
        if l.is_null() {
            return -1;
        }
        let mut item: *mut listitem_T;
        let mut idx: varnumber_T = 0;
        if startidx == 0 {
            item = tv_list_first(l);
        } else {
            idx = varnumber_T::from(tv_list_uidx(l, startidx as c_int));
            if idx == -1 {
                item = ptr::null_mut();
            } else {
                item = tv_list_find(l, idx as c_int);
                debug_assert!(!item.is_null());
            }
        }

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let called_emsg_start = called_emsg;
        while !item.is_null() {
            set_vim_var_nr(VV_KEY, idx);
            tv_copy(&raw const (*item).li_tv, get_vim_var_tv(VV_VAL));

            let found = indexof_eval_expr(expr) != 0;
            tv_clear(get_vim_var_tv(VV_VAL));

            if found {
                return idx;
            }
            if called_emsg != called_emsg_start {
                return -1;
            }
            item = (*item).li_next;
            idx += 1;
        }
        -1
    }
}

/// "indexof()" function
pub unsafe fn f_indexof(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = -1;

        if tv_check_for_list_or_blob_arg(argvars, 0) == FAIL
            || tv_check_for_string_or_func_arg(argvars, 1) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 2) == FAIL
        {
            return;
        }

        if ((*argvars.add(1)).v_type == VarType::VAR_STRING
            && ((*argvars.add(1)).vval.v_string.is_null()
                || *(*argvars.add(1)).vval.v_string == NUL))
            || ((*argvars.add(1)).v_type == VarType::VAR_FUNC
                && (*argvars.add(1)).vval.v_string.is_null())
        {
            return;
        }

        let mut startidx: varnumber_T = 0;
        if (*argvars.add(2)).v_type == VarType::VAR_DICT {
            startidx = tv_dict_get_number_def(
                (*argvars.add(2)).vval.v_dict.cast(),
                c"startidx".as_ptr(),
                0,
            );
        }

        let mut save_val = tv_initial_value();
        let mut save_key = tv_initial_value();
        prepare_vimvar(VV_VAL, &raw mut save_val);
        prepare_vimvar(VV_KEY, &raw mut save_key);

        let save_did_emsg = did_emsg;
        did_emsg = 0;

        if (*argvars).v_type == VarType::VAR_BLOB {
            (*rettv).vval.v_number =
                indexof_blob((*argvars).vval.v_blob.cast(), startidx, argvars.add(1));
        } else {
            (*rettv).vval.v_number = indexof_list((*argvars).vval.v_list, startidx, argvars.add(1));
        }

        restore_vimvar(VV_KEY, &raw mut save_key);
        restore_vimvar(VV_VAL, &raw mut save_val);
        did_emsg |= save_did_emsg;
    }
}

/// "input()" function
pub unsafe fn f_input(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        get_user_input(argvars, rettv, false, INPUTSECRET_FLAG);
    }
}

/// "inputdialog()" function
pub unsafe fn f_inputdialog(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        get_user_input(argvars, rettv, true, INPUTSECRET_FLAG);
    }
}

/// "inputlist()" function
pub unsafe fn f_inputlist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg!(gettext(e_listarg.as_ptr()), c"inputlist()".as_ptr());
            return;
        }

        msg_ext_set_kind(c"confirm".as_ptr());
        msg_start();
        msg_row = Rows - 1;
        lines_left = Rows;
        msg_scroll = 1;
        msg_clr_eos();

        let l = (*argvars).vval.v_list;
        let mut li = tv_list_first(l);
        while !li.is_null() {
            msg_puts(tv_get_string(&raw const (*li).li_tv));
            if !ui_has(kUIMessages) || !(*li).li_next.is_null() {
                msg_putchar(b'\n' as c_int);
            }
            li = (*li).li_next;
        }

        let mut mouse_used = false;
        let mut selected = prompt_for_input(ptr::null_mut(), 0, false, &raw mut mouse_used);
        if mouse_used {
            selected = tv_list_len(l) - (cmdline_row - mouse_row);
        }
        (*rettv).vval.v_number = varnumber_T::from(selected);
    }
}

/// "inputrestore()" function
pub unsafe fn f_inputrestore(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let userinput = &raw mut GA_USERINPUT;
        if let Some(mut saved) = (*userinput).pop() {
            restore_typeahead(&mut saved);
        } else if p_verbose > 1 {
            verb_msg(gettext(
                c"called inputrestore() more often than inputsave()".as_ptr(),
            ));
            (*rettv).vval.v_number = 1;
        }
    }
}

/// "inputsave()" function
pub unsafe fn f_inputsave(_argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut saved = tasave_T::new();
        save_typeahead(&mut saved);
        let userinput = &raw mut GA_USERINPUT;
        (*userinput).push(saved);
    }
}

/// "inputsecret()" function
pub unsafe fn f_inputsecret(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData) {
    unsafe {
        cmdline_star += 1;
        INPUTSECRET_FLAG = true;
        f_input(argvars, rettv, fptr);
        cmdline_star -= 1;
        INPUTSECRET_FLAG = false;
    }
}

/// "interrupt()" function
pub unsafe fn f_interrupt(_argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        got_int = true;
    }
}

/// "invert(expr)" function
pub unsafe fn f_invert(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = !tv_get_number_chk(argvars, ptr::null_mut());
    }
}

/// "islocked()" function
pub unsafe fn f_islocked(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut lv: lval_T = lval_T::new();
        (*rettv).vval.v_number = -1;
        let end = get_lval(
            tv_get_string(argvars).cast_mut(),
            ptr::null_mut(),
            &raw mut lv,
            false,
            false,
            GLV_NO_AUTOLOAD | GLV_READ_ONLY,
            FNE_CHECK_START,
        );
        if !end.is_null() && !lv.ll_name.is_null() {
            if *end != NUL {
                semsg!(
                    gettext(if lv.ll_name_len == 0 {
                        e_invarg2.as_ptr()
                    } else {
                        e_trailing_arg.as_ptr()
                    }),
                    end,
                );
            } else if lv.ll_tv.is_null() {
                let di = find_var(lv.ll_name, lv.ll_name_len, ptr::null_mut(), 1);
                if !di.is_null() {
                    (*rettv).vval.v_number = varnumber_T::from(
                        ((*di).di_flags & DI_FLAGS_LOCK) != 0
                            || tv_islocked(&raw const (*di).di_tv),
                    );
                }
            } else if lv.ll_range {
                emsg(gettext(c"E786: Range not allowed".as_ptr()));
            } else if !lv.ll_newkey.is_null() {
                semsg!(gettext(e_dictkey.as_ptr()), lv.ll_newkey);
            } else if !lv.ll_list.is_null() {
                (*rettv).vval.v_number =
                    varnumber_T::from(tv_islocked(&raw const (*lv.ll_li).li_tv));
            } else {
                (*rettv).vval.v_number =
                    varnumber_T::from(tv_islocked(&raw const (*lv.ll_di).di_tv));
            }
        }
    }
}

/// "isinf()" function
pub unsafe fn f_isinf(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_FLOAT && xisinf((*argvars).vval.v_float) != 0 {
            (*rettv).vval.v_number = if (*argvars).vval.v_float > 0.0 { 1 } else { -1 };
        }
    }
}

/// "isnan()" function
pub unsafe fn f_isnan(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(
            (*argvars).v_type == VarType::VAR_FLOAT && xisnan((*argvars).vval.v_float) != 0,
        );
    }
}

/// "id()" function
pub unsafe fn f_id(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let len = vim_snprintf_typval_only(ptr::null_mut(), 0, c"%p".as_ptr(), argvars);
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmalloc((len as usize) + 1).cast();
        vim_snprintf_typval_only(
            (*rettv).vval.v_string,
            (len as usize) + 1,
            c"%p".as_ptr(),
            argvars,
        );
    }
}

/// "jobpid(id)" function
pub unsafe fn f_jobpid(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let data = find_job((*argvars).vval.v_number as u64, true);
        if data.is_null() {
            return;
        }
        (*rettv).vval.v_number = varnumber_T::from((*data).uv.proc_.pid);
    }
}

/// "jobresize(job, width, height)" function
pub unsafe fn f_jobresize(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER
            || (*argvars.add(1)).v_type != VarType::VAR_NUMBER
            || (*argvars.add(2)).v_type != VarType::VAR_NUMBER
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let data = find_job((*argvars).vval.v_number as u64, true);
        if data.is_null() {
            return;
        }

        let proc_ptr = &raw mut (*data).uv.proc_;
        if (*proc_ptr).r#type != kProcTypePty {
            emsg(gettext(e_channotpty.as_ptr()));
            return;
        }

        pty_proc_resize(
            proc_ptr as *mut PtyProc,
            (*argvars.add(1)).vval.v_number as u16,
            (*argvars.add(2)).vval.v_number as u16,
        );
        (*rettv).vval.v_number = 1;
    }
}

/// Builds an environment dict for a child process (job).
pub unsafe fn create_environment(
    job_env: *const dictitem_T,
    clear_env: bool,
    pty: bool,
    set_nvim_addr: bool,
    pty_term_name: *const c_char,
) -> *mut dict_T {
    unsafe {
        let env = tv_dict_alloc();

        if !clear_env {
            let mut envitems: *mut uv_env_item_t = ptr::null_mut();
            let mut envcount: c_int = 0;
            if uv_os_environ(&raw mut envitems, &raw mut envcount) == 0 {
                for i in 0..(envcount as usize) {
                    #[cfg(windows)]
                    let mut name: *const c_char = (*envitems.add(i)).name;
                    #[cfg(not(windows))]
                    let name: *const c_char = (*envitems.add(i)).name;
                    #[cfg(windows)]
                    let mut namelen = strlen(name);
                    #[cfg(not(windows))]
                    let namelen = strlen(name);
                    #[cfg(windows)]
                    {
                        if namelen < MAXPATHL / MB_MAXBYTES {
                            let namebuff = (&raw mut NameBuff).cast::<c_char>();
                            namelen = mb_strup_buf(name, namebuff);
                            name = namebuff;
                        }
                    }
                    tv_dict_add_str(env, name, namelen, (*envitems.add(i)).value);
                }
                uv_os_free_environ(envitems, envcount);
            }

            if pty {
                for &ignored in PTY_IGNORED_ENV_VARS {
                    let dv = tv_dict_find(env, ignored.as_ptr(), -1);
                    if !dv.is_null() {
                        tv_dict_item_remove(env, dv);
                    }
                }
                if p_tgc != 0 {
                    tv_dict_add_str(env, c"COLORTERM".as_ptr(), 9, c"truecolor".as_ptr());
                }
            }
        }

        if pty {
            let dv = tv_dict_find(env, c"TERM".as_ptr(), 4);
            if !dv.is_null() {
                tv_dict_item_remove(env, dv);
            }
            tv_dict_add_str(env, c"TERM".as_ptr(), 4, pty_term_name);
        }

        if set_nvim_addr {
            let nvim_addr = get_vim_var_str(VV_SEND_SERVER);
            if *nvim_addr != NUL {
                let dv = tv_dict_find(env, c"NVIM".as_ptr(), 4);
                if !dv.is_null() {
                    tv_dict_item_remove(env, dv);
                }
                tv_dict_add_str(env, c"NVIM".as_ptr(), 4, nvim_addr);
            }
        }

        if !job_env.is_null() {
            #[cfg(windows)]
            {
                let d = (*job_env).di_tv.vval.v_dict.cast::<dict_T>();
                if !d.is_null() {
                    let ht = &raw const (*d).dv_hashtab;
                    let mut todo = (*ht).ht_used;
                    let mut hi = (*ht).ht_array;
                    while todo > 0 {
                        if !crate::hashtab::hashitem_empty(&*hi) {
                            todo -= 1;
                            let var = tv_dict_hi2di(hi);
                            let mut name = (*var).di_key.as_ptr();
                            let mut namelen = strlen(name);
                            if namelen < MAXPATHL / MB_MAXBYTES {
                                let namebuff = (&raw mut NameBuff).cast::<c_char>();
                                namelen = mb_strup_buf(name, namebuff);
                                name = namebuff;
                            }
                            let dv = tv_dict_find(env, name, namelen as isize);
                            if !dv.is_null() {
                                tv_dict_item_remove(env, dv);
                            }
                            tv_dict_add_str(
                                env,
                                name,
                                namelen,
                                tv_get_string(&raw const (*var).di_tv),
                            );
                        }
                        hi = hi.add(1);
                    }
                }
            }
            #[cfg(not(windows))]
            {
                tv_dict_extend(env, (*job_env).di_tv.vval.v_dict.cast(), c"force".as_ptr());
            }
        }

        if pty {
            for &req in REQUIRED_ENV_VARS {
                let len = req.to_bytes().len();
                let dv = tv_dict_find(env, req.as_ptr(), len as isize);
                if dv.is_null() {
                    let env_var = os_getenv(req.as_ptr());
                    if !env_var.is_null() {
                        tv_dict_add_allocated_str(env, req.as_ptr(), len, env_var);
                    }
                }
            }
        }

        env
    }
}

/// "jobstart()" function
pub unsafe fn f_jobstart(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        let mut cmd: *const c_char = ptr::null();
        let mut executable = true;
        let mut argv = tv_to_argv(argvars, &raw mut cmd, &raw mut executable);
        if argv.is_empty() {
            (*rettv).vval.v_number = if executable { 0 } else { -1 };
            return;
        }

        if (*argvars.add(1)).v_type != VarType::VAR_DICT
            && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
        {
            semsg!(gettext(e_invarg2.as_ptr()), c"expected dictionary".as_ptr());
            crate::path::clear_cstrings(&mut argv);
            return;
        }

        let mut job_opts: *mut dict_T = ptr::null_mut();
        let mut detach = false;
        let mut rpc = false;
        let mut pty = false;
        let mut term = false;
        let mut clear_env = false;
        let mut overlapped = false;
        let mut stdin_mode = kChannelStdinPipe;
        let mut on_stdout = CallbackReader::none();
        let mut on_stderr = CallbackReader::none();
        let mut on_exit = Callback::NONE;
        let mut cwd: *const c_char = ptr::null();
        let mut job_env: *mut dictitem_T = ptr::null_mut();

        if (*argvars.add(1)).v_type == VarType::VAR_DICT {
            job_opts = (*argvars.add(1)).vval.v_dict.cast();

            detach = tv_dict_get_number(job_opts, c"detach".as_ptr()) != 0;
            rpc = tv_dict_get_number(job_opts, c"rpc".as_ptr()) != 0;
            term = tv_dict_get_number(job_opts, c"term".as_ptr()) != 0;
            pty = term || tv_dict_get_number(job_opts, c"pty".as_ptr()) != 0;
            clear_env = tv_dict_get_number(job_opts, c"clear_env".as_ptr()) != 0;
            overlapped = tv_dict_get_number(job_opts, c"overlapped".as_ptr()) != 0;

            let s = tv_dict_get_string(job_opts, c"stdin".as_ptr(), false);
            if !s.is_null() {
                if strncmp(s, c"null".as_ptr(), NUMBUFLEN) == 0 {
                    stdin_mode = kChannelStdinNull;
                } else if strncmp(s, c"pipe".as_ptr(), NUMBUFLEN) == 0 {
                    // default
                } else {
                    semsg!(gettext(e_invargNval.as_ptr()), c"stdin".as_ptr(), s);
                }
            }

            let job_term = tv_dict_find(job_opts, c"term".as_ptr(), 4);
            if !job_term.is_null() && (*job_term).di_tv.v_type != VarType::VAR_BOOL {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    c"'term' must be Boolean".as_ptr(),
                );
                crate::path::clear_cstrings(&mut argv);
                return;
            }

            if pty && rpc {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    c"job cannot have both 'pty' and 'rpc' options set".as_ptr(),
                );
                crate::path::clear_cstrings(&mut argv);
                return;
            }

            #[cfg(windows)]
            if pty && overlapped {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    c"job cannot have both 'pty' and 'overlapped' options set".as_ptr(),
                );
                crate::path::clear_cstrings(&mut argv);
                return;
            }

            let new_cwd = tv_dict_get_string(job_opts, c"cwd".as_ptr(), false);
            if !new_cwd.is_null() && *new_cwd != NUL {
                cwd = new_cwd;
                if !os_isdir(cwd) {
                    semsg!(
                        gettext(e_invarg2.as_ptr()),
                        c"expected valid directory".as_ptr(),
                    );
                    crate::path::clear_cstrings(&mut argv);
                    return;
                }
            }

            job_env = tv_dict_find(job_opts, c"env".as_ptr(), 3);
            if !job_env.is_null() && (*job_env).di_tv.v_type != VarType::VAR_DICT {
                semsg!(gettext(e_invarg2.as_ptr()), c"env".as_ptr());
                crate::path::clear_cstrings(&mut argv);
                return;
            }

            if !common_job_callbacks(
                job_opts,
                &raw mut on_stdout,
                &raw mut on_stderr,
                &raw mut on_exit,
            ) {
                crate::path::clear_cstrings(&mut argv);
                return;
            }
        }

        let mut width = tv_dict_get_number(job_opts, c"width".as_ptr()) as u16;
        let mut height = tv_dict_get_number(job_opts, c"height".as_ptr()) as u16;
        let mut term_name: *const c_char = ptr::null();

        if term {
            if text_locked() {
                text_locked_msg();
                crate::path::clear_cstrings(&mut argv);
                return;
            }
            if (*curbuf).b_changed != 0 {
                emsg(gettext(
                    c"jobstart(...,{term=true}) requires unmodified buffer".as_ptr(),
                ));
                crate::path::clear_cstrings(&mut argv);
                return;
            }
            if !(*curbuf).terminal.is_null() {
                if terminal_running((*curbuf).terminal) {
                    semsg!(
                        gettext(c"Terminal already connected to buffer %d".as_ptr()),
                        (*curbuf).b_fnum,
                    );
                    crate::path::clear_cstrings(&mut argv);
                    return;
                }
                buf_close_terminal(curbuf);
            }
            debug_assert!(!rpc);
            term_name = c"xterm-256color".as_ptr();
            if cwd.is_null() {
                cwd = c".".as_ptr();
            }
            overlapped = false;
            detach = false;
            stdin_mode = kChannelStdinPipe;
            if width == 0 {
                width = ((*curwin).w_view_width - win_col_off(curwin)).max(0) as u16;
            }
            if height == 0 {
                height = (*curwin).w_view_height as u16;
            }
        }

        if pty {
            if term_name.is_null() {
                term_name = tv_dict_get_string(job_opts, c"TERM".as_ptr(), false);
            }
            if term_name.is_null() {
                term_name = c"ansi".as_ptr();
            }
        }

        let env = create_environment(job_env, clear_env, pty, true, term_name);
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
            &raw mut (*rettv).vval.v_number,
        );
        if chan.is_null() {
            // do nothing
        } else if !term {
            channel_create_event(chan, ptr::null());
        } else {
            if (*rettv).vval.v_number <= 0 {
                return;
            }

            let pid = (*chan).uv.proc_.pid;
            let buf = curbuf;

            (*buf).b_p_swf = 0;
            if (*buf).b_ml.ml_mfp.is_null() && ml_open(buf) == FAIL {
                proc_stop(&raw mut (*chan).uv.proc_);
                channel_decref(chan);
                return;
            }

            channel_incref(chan);
            channel_terminal_alloc(buf, chan);

            let ct = chan;
            'term_done: {
                apply_autocmds(
                    EVENT_BUFFILEPRE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    buf,
                );

                if (*ct).term.is_null() || terminal_buf((*ct).term) == 0 {
                    break 'term_done;
                }

                let namebuff = (&raw mut NameBuff).cast::<c_char>();
                let iobuff = (&raw mut IObuff).cast::<c_char>();
                vim_FullName(cwd, namebuff, MAXPATHL, false);
                let len = home_replace(ptr::null_mut(), namebuff, iobuff, IOSIZE, true);
                if len != 1
                    && (*iobuff.add(len - 1) == b'\\' as c_char
                        || *iobuff.add(len - 1) == b'/' as c_char)
                {
                    *iobuff.add(len - 1) = NUL;
                }
                if len == 1 && *iobuff == b'/' as c_char {
                    *iobuff.add(1) = b'.' as c_char;
                    *iobuff.add(2) = NUL;
                }

                vim_snprintf!(
                    namebuff,
                    MAXPATHL,
                    c"term://%s//%d:%s".as_ptr(),
                    iobuff,
                    pid,
                    cmd,
                );

                setfname(buf, namebuff, ptr::null_mut(), true);
                apply_autocmds(
                    EVENT_BUFFILEPOST,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    buf,
                );

                if (*ct).term.is_null() || terminal_buf((*ct).term) == 0 {
                    break 'term_done;
                }

                let mut err = Error {
                    err_type: ErrorType::kErrorTypeNone,
                    msg: ptr::null_mut(),
                };
                (*buf).b_locked += 1;
                let cp = chan;
                dict_set_var(
                    (*buf).b_vars,
                    cstr_as_string(c"terminal_job_id".as_ptr()),
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: (*cp).id as Integer,
                        },
                    },
                    false,
                    false,
                    ptr::null_mut(),
                    &raw mut err,
                );
                api_clear_error(&raw mut err);
                dict_set_var(
                    (*buf).b_vars,
                    cstr_as_string(c"terminal_job_pid".as_ptr()),
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: Integer::from(pid),
                        },
                    },
                    false,
                    false,
                    ptr::null_mut(),
                    &raw mut err,
                );
                api_clear_error(&raw mut err);
                (*buf).b_locked -= 1;

                if (*ct).term.is_null() || terminal_buf((*ct).term) == 0 {
                    break 'term_done;
                }

                terminal_open(&raw mut (*ct).term, buf);
            }
            channel_create_event(chan, ptr::null());
            channel_decref(chan);
        }
    }
}

/// "jobstop()" function
pub unsafe fn f_jobstop(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let data = find_job((*argvars).vval.v_number as u64, false);
        if data.is_null() {
            return;
        }

        let mut error: *const c_char = ptr::null();
        if (*data).is_rpc {
            channel_close((*data).id, kChannelPartRpc, &raw mut error);
        }
        proc_stop(&raw mut (*data).uv.proc_);
        (*rettv).vval.v_number = 1;
        if !error.is_null() {
            emsg(error);
        }
    }
}

/// "jobwait(ids[, timeout])" function
pub unsafe fn f_jobwait(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_LIST
            || ((*argvars.add(1)).v_type != VarType::VAR_NUMBER
                && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN)
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let args = (*argvars).vval.v_list;
        let len = tv_list_len(args) as usize;
        let mut jobs: Vec<*mut Channel> = vec![ptr::null_mut(); len];
        let main_loop_ptr: *mut Loop = &raw mut main_loop;
        let waiting_jobs = multiqueue_new(main_loop_ptr);

        let mut i: usize = 0;
        let mut arg = tv_list_first(args);
        while !arg.is_null() {
            let tv = &raw const (*arg).li_tv;
            let chan = if (*tv).v_type == VarType::VAR_NUMBER {
                find_channel((*tv).vval.v_number as u64)
            } else {
                ptr::null_mut()
            };
            if chan.is_null() || (*chan).streamtype != kChannelStreamProc {
                jobs[i] = ptr::null_mut();
            } else {
                let proc_ptr = &raw mut (*chan).uv.proc_;
                if proc_is_stopped(proc_ptr) {
                    proc_wait(proc_ptr, -1, ptr::null_mut());
                    jobs[i] = ptr::null_mut();
                } else {
                    jobs[i] = chan;
                    channel_incref(chan);
                    if (*proc_ptr).status < 0 {
                        let events = (*chan).events;
                        multiqueue_process_events(events);
                        multiqueue_replace_parent(events, waiting_jobs);
                    }
                }
            }
            i += 1;
            arg = (*arg).li_next;
        }

        let mut remaining: c_int = -1;
        let mut before: u64 = 0;
        if (*argvars.add(1)).v_type == VarType::VAR_NUMBER && (*argvars.add(1)).vval.v_number >= 0 {
            remaining = (*argvars.add(1)).vval.v_number as c_int;
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
            let status = proc_wait(&raw mut (*job).uv.proc_, remaining, waiting_jobs);
            if status < 0 {
                break;
            }
            if remaining > 0 {
                let now = os_hrtime();
                remaining = 0.min(remaining - ((now - before) / 1_000_000) as c_int);
                before = now;
            }
        }

        let rv = tv_list_alloc(len as isize);
        for &job in &jobs {
            if job.is_null() {
                tv_list_append_number(rv, -3);
                continue;
            }
            let events = (*job).events;
            multiqueue_process_events(events);
            multiqueue_replace_parent(events, (*loop_queues(main_loop_ptr)).events);

            let status = (*job).uv.proc_.status;
            tv_list_append_number(rv, varnumber_T::from(status));
            channel_decref(job);
        }

        multiqueue_free(waiting_jobs);
        if busy {
            ui_busy_stop();
        }
        tv_list_ref(rv);
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_list = rv;
    }
}

/// "json_decode()" function
pub unsafe fn f_json_decode(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let s: *const c_char;
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        if (*argvars).v_type == VarType::VAR_LIST {
            if !encode_vim_list_to_buf((*argvars).vval.v_list, &raw mut len, &raw mut tofree) {
                emsg(gettext(c"E474: Failed to convert list to string".as_ptr()));
                return;
            }
            if tofree.is_null() {
                debug_assert!(len == 0);
                s = c"".as_ptr();
            } else {
                s = tofree;
            }
        } else {
            s = tv_get_string_buf_chk(argvars, numbuf.as_mut_ptr());
            if !s.is_null() {
                len = strlen(s);
            } else {
                return;
            }
        }
        if json_decode_string(s, len, rettv) == FAIL {
            semsg!(
                gettext(c"E474: Failed to parse %.*s".as_ptr()),
                len as c_int,
                s,
            );
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = 0;
        }
        debug_assert!((*rettv).v_type != VarType::VAR_UNKNOWN);
        xfree(tofree.cast());
    }
}

/// "json_encode()" function
pub unsafe fn f_json_encode(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = encode_tv2json(argvars, ptr::null_mut());
    }
}

/// "keytrans()" function
pub unsafe fn f_keytrans(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        if tv_check_for_string_arg(argvars, 0) == FAIL || (*argvars).vval.v_string.is_null() {
            return;
        }
        let escaped = vim_strsave_escape_ks((*argvars).vval.v_string);
        (*rettv).vval.v_string = str2special_save(escaped, true, 1);
        xfree(escaped.cast());
    }
}

/// "len()" function
pub unsafe fn f_len(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        match (*argvars).v_type {
            VarType::VAR_STRING | VarType::VAR_NUMBER => {
                (*rettv).vval.v_number = strlen(tv_get_string(argvars)) as varnumber_T;
            }
            VarType::VAR_BLOB => {
                (*rettv).vval.v_number =
                    varnumber_T::from(tv_blob_len((*argvars).vval.v_blob.cast()));
            }
            VarType::VAR_LIST => {
                (*rettv).vval.v_number = varnumber_T::from(tv_list_len((*argvars).vval.v_list));
            }
            VarType::VAR_DICT => {
                (*rettv).vval.v_number = tv_dict_len((*argvars).vval.v_dict.cast()) as varnumber_T;
            }
            VarType::VAR_UNKNOWN
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_FLOAT
            | VarType::VAR_PARTIAL
            | VarType::VAR_FUNC => {
                emsg(gettext(c"E701: Invalid type for len()".as_ptr()));
            }
        }
    }
}

unsafe fn libcall_common(argvars: *mut typval_T, rettv: *mut typval_T, out_type: VarType) {
    unsafe {
        (*rettv).v_type = out_type;
        if out_type != VarType::VAR_NUMBER {
            (*rettv).vval.v_string = ptr::null_mut();
        }

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_STRING
            || (*argvars.add(1)).v_type != VarType::VAR_STRING
        {
            return;
        }

        let libname = (*argvars).vval.v_string;
        let funcname = (*argvars.add(1)).vval.v_string;
        let in_type = (*argvars.add(2)).v_type;

        let str_in = if in_type == VarType::VAR_STRING {
            (*argvars.add(2)).vval.v_string
        } else {
            ptr::null()
        };
        let int_in = (*argvars.add(2)).vval.v_number as c_int;

        let str_out = if out_type == VarType::VAR_STRING {
            &raw mut (*rettv).vval.v_string
        } else {
            ptr::null_mut()
        };
        let mut int_out: c_int = 0;

        let success = os_libcall(libname, funcname, str_in, int_in, str_out, &raw mut int_out);
        if !success {
            semsg!(gettext(e_libcall.as_ptr()), funcname);
            return;
        }
        if out_type == VarType::VAR_NUMBER {
            (*rettv).vval.v_number = varnumber_T::from(int_out);
        }
    }
}

/// "libcall()" function
pub unsafe fn f_libcall(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        libcall_common(argvars, rettv, VarType::VAR_STRING);
    }
}

/// "libcallnr()" function
pub unsafe fn f_libcallnr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        libcall_common(argvars, rettv, VarType::VAR_NUMBER);
    }
}

/// "line(string, [winid])" function
pub unsafe fn f_line(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut lnum: linenr_T = 0;
        let mut fp: *mut pos_T = ptr::null_mut();
        let mut fnum: c_int = 0;

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let id = tv_get_number(argvars.add(1)) as c_int;
            let mut tp: *mut tabpage_T = ptr::null_mut();
            let wp = win_id2wp_tp(id, &raw mut tp);
            if !wp.is_null() && !tp.is_null() {
                if *p_spk != b'c' as c_char
                    || ((*wp).w_onebuf_opt.wo_diff != 0 && (*curwin).w_onebuf_opt.wo_diff != 0)
                {
                    skip_update_topline = true;
                }
                check_cursor(wp);
                fp = var2fpos(argvars, true, &raw mut fnum, false, wp);
                skip_update_topline = false;
            }
        } else {
            fp = var2fpos(argvars, true, &raw mut fnum, false, curwin);
        }

        if !fp.is_null() {
            lnum = (*fp).lnum;
        }
        (*rettv).vval.v_number = varnumber_T::from(lnum);
    }
}

/// "line2byte(lnum)" function
pub unsafe fn f_line2byte(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let lnum = tv_get_lnum(argvars);
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count + 1 {
            (*rettv).vval.v_number = -1;
        } else {
            (*rettv).vval.v_number =
                ml_find_line_or_offset(curbuf, lnum, ptr::null_mut(), false) as varnumber_T;
        }
        if (*rettv).vval.v_number >= 0 {
            (*rettv).vval.v_number += 1;
        }
    }
}

/// "localtime()" function
pub unsafe fn f_localtime(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = (os_time() as time_t) as varnumber_T;
    }
}

/// "luaeval()" function
pub unsafe fn f_luaeval(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let str_ = tv_get_string_chk(argvars);
        if str_.is_null() {
            return;
        }
        nlua_call_luaeval(cstr_as_string(str_), argvars.add(1), rettv);
    }
}

unsafe fn find_some_match(argvars: *mut typval_T, rettv: *mut typval_T, type_: SomeMatchType) {
    unsafe {
        let mut str_: *mut c_char = ptr::null_mut();
        let mut len: i64 = 0;
        let mut expr: *mut c_char = ptr::null_mut();
        let mut regmatch: regmatch_T = regmatch_T::new();
        let mut start: i64;
        let mut nth: i64 = 1;
        let mut startcol: colnr_T = 0;
        let mut match_found = false;
        let mut l: *mut list_T = ptr::null_mut();
        let mut idx: c_int = 0;
        let mut tofree: *mut c_char = ptr::null_mut();

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        (*rettv).vval.v_number = -1;
        match type_ {
            SomeMatchType::kSomeMatchList => {
                tv_list_alloc_ret(rettv, kListLenMayKnow);
            }
            SomeMatchType::kSomeMatchStrPos => {
                tv_list_alloc_ret(rettv, 4);
                tv_list_append_string((*rettv).vval.v_list, c"".as_ptr(), 0);
                tv_list_append_number((*rettv).vval.v_list, -1);
                tv_list_append_number((*rettv).vval.v_list, -1);
                tv_list_append_number((*rettv).vval.v_list, -1);
            }
            SomeMatchType::kSomeMatchStr => {
                (*rettv).v_type = VarType::VAR_STRING;
                (*rettv).vval.v_string = ptr::null_mut();
            }
            SomeMatchType::kSomeMatch | SomeMatchType::kSomeMatchEnd => {}
        }

        let mut li: *mut listitem_T = ptr::null_mut();
        'theend: {
            if (*argvars).v_type == VarType::VAR_LIST {
                l = (*argvars).vval.v_list;
                if l.is_null() {
                    break 'theend;
                }
                li = tv_list_first(l);
            } else {
                str_ = tv_get_string(argvars).cast_mut();
                expr = str_;
                len = strlen(str_) as i64;
            }

            let mut patbuf = [0 as c_char; NUMBUFLEN];
            let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
            if pat.is_null() {
                break 'theend;
            }

            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                let mut error = false;
                start = tv_get_number_chk(argvars.add(2), &raw mut error);
                if error {
                    break 'theend;
                }
                if !l.is_null() {
                    idx = tv_list_uidx(l, start as c_int);
                    if idx == -1 {
                        break 'theend;
                    }
                    li = tv_list_find(l, idx);
                } else {
                    if start < 0 {
                        start = 0;
                    }
                    if start > len {
                        break 'theend;
                    }
                    if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                        startcol = start as colnr_T;
                    } else {
                        str_ = str_.add(start as usize);
                        len -= start;
                    }
                }

                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    nth = tv_get_number_chk(argvars.add(3), &raw mut error);
                }
                if error {
                    break 'theend;
                }
            }

            regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING).cast();
            if !regmatch.regprog.is_null() {
                regmatch.rm_ic = p_ic != 0;

                loop {
                    if !l.is_null() {
                        if li.is_null() {
                            match_found = false;
                            break;
                        }
                        xfree(tofree.cast());
                        tofree = encode_tv2echo(&raw mut (*li).li_tv, ptr::null_mut());
                        expr = tofree;
                        str_ = tofree;
                        if str_.is_null() {
                            break;
                        }
                    }

                    match_found = vim_regexec_nl((&raw mut regmatch).cast(), str_, startcol);

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
                            .add(utfc_ptr2len(regmatch.startp[0]) as usize)
                            .offset_from(str_)) as colnr_T;
                        if i64::from(startcol) > len
                            || str_.add(startcol as usize) <= regmatch.startp[0]
                        {
                            match_found = false;
                            break;
                        }
                    }
                }

                if match_found {
                    match type_ {
                        SomeMatchType::kSomeMatchStrPos => {
                            let ret_l = (*rettv).vval.v_list;
                            let li1 = tv_list_first(ret_l);
                            let li2 = (*li1).li_next;
                            let li3 = (*li2).li_next;
                            let li4 = (*li3).li_next;
                            xfree((*li1).li_tv.vval.v_string.cast());

                            let rd = regmatch.endp[0].offset_from(regmatch.startp[0]) as usize;
                            (*li1).li_tv.vval.v_string =
                                xmemdupz(regmatch.startp[0].cast(), rd).cast();
                            (*li3).li_tv.vval.v_number =
                                regmatch.startp[0].offset_from(expr) as varnumber_T;
                            (*li4).li_tv.vval.v_number =
                                regmatch.endp[0].offset_from(expr) as varnumber_T;
                            if !l.is_null() {
                                (*li2).li_tv.vval.v_number = varnumber_T::from(idx);
                            }
                        }
                        SomeMatchType::kSomeMatchList => {
                            for i in 0..NSUBEXP {
                                if regmatch.endp[i].is_null() {
                                    tv_list_append_string((*rettv).vval.v_list, ptr::null(), 0);
                                } else {
                                    tv_list_append_string(
                                        (*rettv).vval.v_list,
                                        regmatch.startp[i],
                                        regmatch.endp[i].offset_from(regmatch.startp[i]),
                                    );
                                }
                            }
                        }
                        SomeMatchType::kSomeMatchStr => {
                            if !l.is_null() {
                                tv_copy(&raw const (*li).li_tv, rettv);
                            } else {
                                (*rettv).vval.v_string = xmemdupz(
                                    regmatch.startp[0].cast(),
                                    regmatch.endp[0].offset_from(regmatch.startp[0]) as usize,
                                )
                                .cast();
                            }
                        }
                        SomeMatchType::kSomeMatch | SomeMatchType::kSomeMatchEnd => {
                            if !l.is_null() {
                                (*rettv).vval.v_number = varnumber_T::from(idx);
                            } else {
                                if type_ == SomeMatchType::kSomeMatch {
                                    (*rettv).vval.v_number =
                                        regmatch.startp[0].offset_from(str_) as varnumber_T;
                                } else {
                                    (*rettv).vval.v_number =
                                        regmatch.endp[0].offset_from(str_) as varnumber_T;
                                }
                                (*rettv).vval.v_number += str_.offset_from(expr) as varnumber_T;
                            }
                        }
                    }
                }
                vim_regfree(regmatch.regprog.cast());
            }
        }

        if type_ == SomeMatchType::kSomeMatchStrPos
            && l.is_null()
            && !(*rettv).vval.v_list.is_null()
        {
            let ret_l = (*rettv).vval.v_list;
            let first = tv_list_first(ret_l);
            tv_list_item_remove(ret_l, (*first).li_next);
        }

        xfree(tofree.cast());
        p_cpo = save_cpo;
    }
}

unsafe fn get_matches_in_str(
    str_: *const c_char,
    rmp: *mut regmatch_T,
    mlist: *mut list_T,
    idx: c_int,
    submatches: bool,
    matchbuf: bool,
) {
    unsafe {
        let len = strlen(str_);
        let mut startidx: colnr_T = 0;

        loop {
            if !vim_regexec_nl(rmp.cast(), str_, startidx) {
                break;
            }

            let d = tv_dict_alloc();
            tv_list_append_dict(mlist, d);

            if matchbuf {
                tv_dict_add_nr(d, c"lnum".as_ptr(), 4, varnumber_T::from(idx));
            } else {
                tv_dict_add_nr(d, c"idx".as_ptr(), 3, varnumber_T::from(idx));
            }

            tv_dict_add_nr(
                d,
                c"byteidx".as_ptr(),
                7,
                (*rmp).startp[0].offset_from(str_) as varnumber_T,
            );

            tv_dict_add_str_len(
                d,
                c"text".as_ptr(),
                4,
                (*rmp).startp[0],
                (*rmp).endp[0].offset_from((*rmp).startp[0]) as c_int,
            );

            if submatches {
                let sml = tv_list_alloc((NSUBEXP - 1) as isize);
                tv_dict_add_list(d, c"submatches".as_ptr(), 10, sml);
                for i in 1..NSUBEXP {
                    if (*rmp).endp[i].is_null() {
                        tv_list_append_string(sml, c"".as_ptr(), 0);
                    } else {
                        tv_list_append_string(
                            sml,
                            (*rmp).startp[i],
                            (*rmp).endp[i].offset_from((*rmp).startp[i]),
                        );
                    }
                }
            }
            startidx = (*rmp).endp[0].offset_from(str_) as colnr_T;
            if (startidx as usize) >= len || str_.add(startidx as usize) <= (*rmp).startp[0] {
                break;
            }
        }
    }
}

/// "matchbufline()" function
pub unsafe fn f_matchbufline(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = -1;
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let retlist = (*rettv).vval.v_list;

        if tv_check_for_buffer_arg(argvars, 0) == FAIL
            || tv_check_for_string_arg(argvars, 1) == FAIL
            || tv_check_for_lnum_arg(argvars, 2) == FAIL
            || tv_check_for_lnum_arg(argvars, 3) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 4) == FAIL
        {
            return;
        }

        let prev_did_emsg = did_emsg;
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            if did_emsg == prev_did_emsg {
                semsg!(
                    gettext(e_invalid_buffer_name_str.as_ptr()),
                    tv_get_string(argvars),
                );
            }
            return;
        }
        if (*buf).b_ml.ml_mfp.is_null() {
            emsg(gettext(e_buffer_is_not_loaded.as_ptr()));
            return;
        }

        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let pat = tv_get_string_buf(argvars.add(1), patbuf.as_mut_ptr());

        let did_emsg_before = did_emsg;
        let mut slnum = tv_get_lnum_buf(argvars.add(2), buf);
        if did_emsg > did_emsg_before {
            return;
        }
        if slnum < 1 {
            semsg!(gettext(e_invargval.as_ptr()), c"lnum".as_ptr());
            return;
        }

        let mut elnum = tv_get_lnum_buf(argvars.add(3), buf);
        if did_emsg > did_emsg_before {
            return;
        }
        if elnum < 1 || elnum < slnum {
            semsg!(gettext(e_invargval.as_ptr()), c"end_lnum".as_ptr());
            return;
        }

        if elnum > (*buf).b_ml.ml_line_count {
            elnum = (*buf).b_ml.ml_line_count;
        }

        let mut submatches = false;
        if (*argvars.add(4)).v_type != VarType::VAR_UNKNOWN {
            let d = (*argvars.add(4)).vval.v_dict.cast::<dict_T>();
            if !d.is_null() {
                let di = tv_dict_find(d, c"submatches".as_ptr(), 10);
                if !di.is_null() {
                    if (*di).di_tv.v_type != VarType::VAR_BOOL {
                        semsg!(gettext(e_invargval.as_ptr()), c"submatches".as_ptr());
                        return;
                    }
                    submatches = tv_get_bool(&raw const (*di).di_tv) != 0;
                }
            }
        }

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut regmatch: regmatch_T = regmatch_T::new();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING).cast();
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = p_ic != 0;

            while slnum <= elnum {
                let str_ = ml_get_buf(buf, slnum);
                get_matches_in_str(str_, &raw mut regmatch, retlist, slnum, submatches, true);
                slnum += 1;
            }

            vim_regfree(regmatch.regprog.cast());
        }

        p_cpo = save_cpo;
    }
}

/// "match()" function
pub unsafe fn f_match(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatch);
    }
}

/// "matchend()" function
pub unsafe fn f_matchend(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchEnd);
    }
}

/// "matchlist()" function
pub unsafe fn f_matchlist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchList);
    }
}

/// "matchstr()" function
pub unsafe fn f_matchstr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchStr);
    }
}

/// "matchstrlist()" function
pub unsafe fn f_matchstrlist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = -1;
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let retlist = (*rettv).vval.v_list;

        if tv_check_for_list_arg(argvars, 0) == FAIL
            || tv_check_for_string_arg(argvars, 1) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 2) == FAIL
        {
            return;
        }

        let l = (*argvars).vval.v_list;
        if l.is_null() {
            return;
        }

        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
        if pat.is_null() {
            return;
        }

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut regmatch: regmatch_T = regmatch_T::new();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING).cast();
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = p_ic != 0;

            'cleanup: {
                let mut submatches = false;
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    let d = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
                    if !d.is_null() {
                        let di = tv_dict_find(d, c"submatches".as_ptr(), 10);
                        if !di.is_null() {
                            if (*di).di_tv.v_type != VarType::VAR_BOOL {
                                semsg!(gettext(e_invargval.as_ptr()), c"submatches".as_ptr());
                                break 'cleanup;
                            }
                            submatches = tv_get_bool(&raw const (*di).di_tv) != 0;
                        }
                    }
                }

                let mut idx: c_int = 0;
                let mut li = tv_list_first(l);
                while !li.is_null() {
                    let li_tv = &raw const (*li).li_tv;
                    if (*li_tv).v_type == VarType::VAR_STRING && !(*li_tv).vval.v_string.is_null() {
                        get_matches_in_str(
                            (*li_tv).vval.v_string,
                            &raw mut regmatch,
                            retlist,
                            idx,
                            submatches,
                            false,
                        );
                    }
                    idx += 1;
                    li = (*li).li_next;
                }
            }

            vim_regfree(regmatch.regprog.cast());
        }

        p_cpo = save_cpo;
    }
}

/// "matchstrpos()" function
pub unsafe fn f_matchstrpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchStrPos);
    }
}

/// Get maximal/minimal number value in a list or dictionary.
unsafe fn max_min(tv: *const typval_T, rettv: *mut typval_T, domax: bool) {
    unsafe {
        let mut error = false;
        (*rettv).vval.v_number = 0;
        let mut n: varnumber_T = if domax { VARNUMBER_MIN } else { VARNUMBER_MAX };
        if (*tv).v_type == VarType::VAR_LIST {
            if tv_list_len((*tv).vval.v_list) == 0 {
                return;
            }
            let mut li = tv_list_first((*tv).vval.v_list);
            while !li.is_null() {
                let i = tv_get_number_chk(&raw const (*li).li_tv, &raw mut error);
                if error {
                    return;
                }
                if if domax { i > n } else { i < n } {
                    n = i;
                }
                li = (*li).li_next;
            }
        } else if (*tv).v_type == VarType::VAR_DICT {
            let d = (*tv).vval.v_dict.cast::<dict_T>();
            if tv_dict_len(d) == 0 {
                return;
            }
            let ht = &raw const (*d).dv_hashtab;
            let mut todo = (*ht).ht_used;
            let mut hi = (*ht).ht_array;
            while todo > 0 {
                if !crate::hashtab::hashitem_empty(&*hi) {
                    todo -= 1;
                    let di = tv_dict_hi2di(hi);
                    let i = tv_get_number_chk(&raw const (*di).di_tv, &raw mut error);
                    if error {
                        return;
                    }
                    if if domax { i > n } else { i < n } {
                        n = i;
                    }
                }
                hi = hi.add(1);
            }
        } else {
            semsg!(
                gettext(e_listdictarg.as_ptr()),
                if domax {
                    c"max()".as_ptr()
                } else {
                    c"min()".as_ptr()
                },
            );
            return;
        }
        (*rettv).vval.v_number = n;
    }
}

/// "max()" function
pub unsafe fn f_max(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        max_min(argvars, rettv, true);
    }
}

/// "min()" function
pub unsafe fn f_min(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        max_min(argvars, rettv, false);
    }
}

/// "mode()" function
pub unsafe fn f_mode(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut buf = [0 as c_char; MODE_MAX_LENGTH];
        get_mode(buf.as_mut_ptr());
        if !non_zero_arg(argvars) {
            buf[1] = NUL;
        }
        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

unsafe fn may_add_state_char(
    buf: &mut [c_char],
    len: &mut usize,
    include: *const c_char,
    c: u8,
) {
    unsafe {
        if include.is_null() || !vim_strchr(include, c_int::from(c)).is_null() {
            buf[*len] = c as c_char;
            *len += 1;
        }
    }
}

/// "state()" function
pub unsafe fn f_state(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut buf = [0 as c_char; 16];
        let mut len = 0usize;
        let mut include: *const c_char = ptr::null();
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            include = tv_get_string(argvars);
        }

        if !(stuff_empty() && typebuf.tb_len == 0 && using_script() == 0) {
            may_add_state_char(&mut buf, &mut len, include, b'm');
        }
        if op_pending() {
            may_add_state_char(&mut buf, &mut len, include, b'o');
        }
        if autocmd_busy {
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
        if msg_scrolled > 0 {
            may_add_state_char(&mut buf, &mut len, include, b's');
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = if len == 0 {
            ptr::null_mut()
        } else {
            xmemdupz(buf.as_ptr().cast(), len).cast()
        };
    }
}

/// "msgpackdump()" function
pub unsafe fn f_msgpackdump(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg!(gettext(e_listarg.as_ptr()), c"msgpackdump()".as_ptr());
            return;
        }
        let list = (*argvars).vval.v_list;
        let mut packer: PackerBuffer = packer_string_buffer();
        let msg = gettext(c"msgpackdump() argument, index %i".as_ptr());
        let mut msgbuf = [0 as c_char; 31 * 4 + NUMBUFLEN];
        let mut idx: c_int = 0;
        let mut li = tv_list_first(list);
        while !li.is_null() {
            vim_snprintf!(msgbuf.as_mut_ptr(), msgbuf.len(), msg, idx);
            idx += 1;
            if encode_vim_to_msgpack(&raw mut packer, &raw mut (*li).li_tv, msgbuf.as_ptr()) == FAIL
            {
                break;
            }
            li = (*li).li_next;
        }
        let data = packer_take_string(&raw mut packer);
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
            && strequal(tv_get_string(argvars.add(1)), c"B".as_ptr())
        {
            let b = &mut *tv_blob_alloc_ret(rettv);
            if !data.data.is_null() && data.size > 0 {
                b.bv_ga.extend_from_slice(std::slice::from_raw_parts(
                    data.data.cast::<u8>(),
                    data.size,
                ));
            }
            api_free_string(data);
        } else {
            encode_list_write(
                tv_list_alloc_ret(rettv, kListLenMayKnow),
                data.data,
                data.size,
            );
            api_free_string(data);
        }
    }
}

unsafe fn emsg_mpack_error(status: c_int) {
    unsafe {
        match status {
            MPACK_ERROR => {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    c"Failed to parse msgpack string".as_ptr(),
                );
            }
            MPACK_EOF => {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    c"Incomplete msgpack string".as_ptr(),
                );
            }
            MPACK_NOMEM => {
                semsg!(
                    gettext(e_invarg2.as_ptr()),
                    c"object was too deep to unpack".as_ptr(),
                );
            }
            _ => {}
        }
    }
}

unsafe fn msgpackparse_unpack_list(list: *const list_T, ret_list: *mut list_T) {
    unsafe {
        if tv_list_len(list) == 0 {
            return;
        }
        if (*tv_list_first(list)).li_tv.v_type != VarType::VAR_STRING {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                c"List item is not a string".as_ptr(),
            );
            return;
        }
        let mut lrstate: ListReaderState = encode_init_lrstate(list);
        let buf = alloc_block();
        let mut buf_size: usize = 0;

        let mut cur_item = tv_initial_value();
        let mut parser = mpack_parser_t::new(0);
        mpack_parser_init(&raw mut parser, 0);
        parser.data = crate::mpack::object::mpack_data_t::Typval(&raw mut cur_item);

        let mut status = MPACK_OK;
        'end: {
            loop {
                let mut read_bytes: usize = 0;
                let rlret = encode_read_from_list(
                    &raw mut lrstate,
                    buf.add(buf_size),
                    ARENA_BLOCK_SIZE - buf_size,
                    &raw mut read_bytes,
                );
                if rlret == FAIL {
                    semsg!(
                        gettext(e_invarg2.as_ptr()),
                        c"List item is not a string".as_ptr(),
                    );
                    break 'end;
                }
                buf_size += read_bytes;

                let mut ptr_ = buf.cast::<c_char>().cast_const();
                while buf_size != 0 {
                    status = mpack_parse_typval(&raw mut parser, &raw mut ptr_, &raw mut buf_size);
                    if status == MPACK_OK {
                        tv_list_append_owned_tv(ret_list, cur_item);
                        cur_item.v_type = VarType::VAR_UNKNOWN;
                    } else {
                        break;
                    }
                }

                if rlret == OK {
                    break;
                }

                if status == MPACK_EOF {
                    if buf_size != 0 && ptr_ > buf.cast::<c_char>().cast_const() {
                        ptr::copy(ptr_.cast_mut(), buf, buf_size);
                    }
                } else if status != MPACK_OK {
                    break;
                }
            }

            if status != MPACK_OK {
                typval_parser_error_free(&raw mut parser);
                emsg_mpack_error(status);
            }
        }

        free_block(buf);
    }
}

unsafe fn msgpackparse_unpack_blob(blob: *const blob_T, ret_list: *mut list_T) {
    unsafe {
        let len = tv_blob_len(blob);
        if len == 0 {
            return;
        }

        let mut data = (*blob).bv_ga.as_ptr().cast::<c_char>();
        let mut remaining = len as usize;
        while remaining != 0 {
            let mut tv = tv_initial_value();
            let status = unpack_typval(&raw mut data, &raw mut remaining, &raw mut tv);
            if status != MPACK_OK {
                emsg_mpack_error(status);
                return;
            }
            tv_list_append_owned_tv(ret_list, tv);
        }
    }
}

/// "msgpackparse()" function
pub unsafe fn f_msgpackparse(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST && (*argvars).v_type != VarType::VAR_BLOB {
            semsg!(gettext(e_listblobarg.as_ptr()), c"msgpackparse()".as_ptr());
            return;
        }
        let ret_list = tv_list_alloc_ret(rettv, kListLenMayKnow);
        if (*argvars).v_type == VarType::VAR_LIST {
            msgpackparse_unpack_list((*argvars).vval.v_list, ret_list);
        } else {
            msgpackparse_unpack_blob((*argvars).vval.v_blob.cast(), ret_list);
        }
    }
}

/// "nextnonblank()" function
pub unsafe fn f_nextnonblank(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut lnum = tv_get_lnum(argvars);
        loop {
            if lnum < 0 || lnum > (*curbuf).b_ml.ml_line_count {
                lnum = 0;
                break;
            }
            if *skipwhite(ml_get(lnum)) != NUL {
                break;
            }
            lnum += 1;
        }
        (*rettv).vval.v_number = varnumber_T::from(lnum);
    }
}

/// "nr2char()" function
pub unsafe fn f_nr2char(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN && !tv_check_num(argvars.add(1)) {
            return;
        }
        let mut error = false;
        let num = tv_get_number_chk(argvars, &raw mut error);
        if error {
            return;
        }
        if num < 0 {
            emsg(gettext(
                c"E5070: Character number must not be less than zero".as_ptr(),
            ));
            return;
        }
        if num > varnumber_T::from(c_int::MAX) {
            semsg!(
                gettext(c"E5071: Character number must not be greater than INT_MAX (%i)".as_ptr()),
                c_int::MAX,
            );
            return;
        }

        let mut buf = [0 as c_char; MB_MAXCHAR];
        let len = utf_char2bytes(num as c_int, buf.as_mut_ptr());

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(buf.as_ptr().cast(), len as usize).cast();
    }
}

/// "or(expr, expr)" function
pub unsafe fn f_or(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = tv_get_number_chk(argvars, ptr::null_mut())
            | tv_get_number_chk(argvars.add(1), ptr::null_mut());
    }
}

/// "pow()" function
pub unsafe fn f_pow(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut fx: float_T = 0.0;
        let mut fy: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut fx) && tv_get_float_chk(argvars.add(1), &raw mut fy) {
            (*rettv).vval.v_float = fx.powf(fy);
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// "prevnonblank()" function
pub unsafe fn f_prevnonblank(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut lnum = tv_get_lnum(argvars);
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count {
            lnum = 0;
        } else {
            while lnum >= 1 && *skipwhite(ml_get(lnum)) == NUL {
                lnum -= 1;
            }
        }
        (*rettv).vval.v_number = varnumber_T::from(lnum);
    }
}

/// "printf()" function
pub unsafe fn f_printf(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let saved_did_emsg = did_emsg;
        did_emsg = 0;
        let mut buf = [0 as c_char; NUMBUFLEN];
        let fmt = tv_get_string_buf(argvars, buf.as_mut_ptr());
        let len = vim_snprintf_typval_only(ptr::null_mut(), 0, fmt, argvars.add(1));
        if did_emsg == 0 {
            let s = xmalloc((len as usize) + 1).cast::<c_char>();
            (*rettv).vval.v_string = s;
            vim_snprintf_typval_only(s, (len as usize) + 1, fmt, argvars.add(1));
        }
        did_emsg |= saved_did_emsg;
    }
}

/// "pum_getpos()" function
pub unsafe fn f_pum_getpos(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        pum_set_event_info((*rettv).vval.v_dict.cast());
    }
}

/// "pumvisible()" function
pub unsafe fn f_pumvisible(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if pum_visible() {
            (*rettv).vval.v_number = 1;
        }
    }
}

/// "py3eval()" and "pyxeval()" functions
pub unsafe fn f_py3eval(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        script_host_eval(c"python3".as_ptr().cast_mut(), argvars, rettv);
    }
}

unsafe fn init_srand(x: *mut u32) {
    unsafe {
        let mut number: u32 = 0;
        if uv_random(
            ptr::null_mut(),
            ptr::null_mut(),
            (&raw mut number).cast(),
            size_of::<u32>(),
            0,
            ptr::null(),
        ) == 0
        {
            *x = number;
            return;
        }
        *x = (os_hrtime() as u32) ^ (os_get_pid() as u32);
    }
}

#[inline(always)]
unsafe fn splitmix32(x: *mut u32) -> u32 {
    unsafe {
        *x = (*x).wrapping_add(0x9e3779b9);
        let mut z = *x;
        z = (z ^ (z >> 16)).wrapping_mul(0x85ebca6b);
        z = (z ^ (z >> 13)).wrapping_mul(0xc2b2ae35);
        z ^ (z >> 16)
    }
}

#[inline(always)]
unsafe fn shuffle_xoshiro128starstar(x: *mut u32, y: *mut u32, z: *mut u32, w: *mut u32) -> u32 {
    unsafe {
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
}

/// "rand()" function
pub unsafe fn f_rand(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let result: u32;

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            if !RAND_INITIALIZED {
                let mut x: u32 = 0;
                init_srand(&raw mut x);
                RAND_GX = splitmix32(&raw mut x);
                RAND_GY = splitmix32(&raw mut x);
                RAND_GZ = splitmix32(&raw mut x);
                RAND_GW = splitmix32(&raw mut x);
                RAND_INITIALIZED = true;
            }
            result = shuffle_xoshiro128starstar(
                &raw mut RAND_GX,
                &raw mut RAND_GY,
                &raw mut RAND_GZ,
                &raw mut RAND_GW,
            );
        } else if (*argvars).v_type == VarType::VAR_LIST {
            let l = (*argvars).vval.v_list;
            if tv_list_len(l) != 4 {
                semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
                (*rettv).v_type = VarType::VAR_NUMBER;
                (*rettv).vval.v_number = -1;
                return;
            }

            let tvx = &raw mut (*tv_list_find(l, 0)).li_tv;
            let tvy = &raw mut (*tv_list_find(l, 1)).li_tv;
            let tvz = &raw mut (*tv_list_find(l, 2)).li_tv;
            let tvw = &raw mut (*tv_list_find(l, 3)).li_tv;
            if (*tvx).v_type != VarType::VAR_NUMBER
                || (*tvy).v_type != VarType::VAR_NUMBER
                || (*tvz).v_type != VarType::VAR_NUMBER
                || (*tvw).v_type != VarType::VAR_NUMBER
            {
                semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
                (*rettv).v_type = VarType::VAR_NUMBER;
                (*rettv).vval.v_number = -1;
                return;
            }
            let mut x = (*tvx).vval.v_number as u32;
            let mut y = (*tvy).vval.v_number as u32;
            let mut z = (*tvz).vval.v_number as u32;
            let mut w = (*tvw).vval.v_number as u32;

            result = shuffle_xoshiro128starstar(&raw mut x, &raw mut y, &raw mut z, &raw mut w);

            (*tvx).vval.v_number = varnumber_T::from(x);
            (*tvy).vval.v_number = varnumber_T::from(y);
            (*tvz).vval.v_number = varnumber_T::from(z);
            (*tvw).vval.v_number = varnumber_T::from(w);
        } else {
            semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = -1;
            return;
        }

        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = varnumber_T::from(result);
    }
}

/// "srand()" function
pub unsafe fn f_srand(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut x: u32 = 0;
        tv_list_alloc_ret(rettv, 4);
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            init_srand(&raw mut x);
        } else {
            let mut error = false;
            x = tv_get_number_chk(argvars, &raw mut error) as u32;
            if error {
                return;
            }
        }
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
    }
}

/// "perleval()" function
pub unsafe fn f_perleval(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        script_host_eval(c"perl".as_ptr().cast_mut(), argvars, rettv);
    }
}

/// "rubyeval()" function
pub unsafe fn f_rubyeval(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        script_host_eval(c"ruby".as_ptr().cast_mut(), argvars, rettv);
    }
}

/// "range()" function
pub unsafe fn f_range(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let end: varnumber_T;
        let mut stride: varnumber_T = 1;
        let mut error = false;

        let mut start = tv_get_number_chk(argvars, &raw mut error);
        if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            end = start - 1;
            start = 0;
        } else {
            end = tv_get_number_chk(argvars.add(1), &raw mut error);
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                stride = tv_get_number_chk(argvars.add(2), &raw mut error);
            }
        }

        if error {
            return;
        }
        if stride == 0 {
            emsg(gettext(c"E726: Stride is zero".as_ptr()));
            return;
        }
        if if stride > 0 {
            end.saturating_add(1) < start
        } else {
            end.saturating_sub(1) > start
        } {
            emsg(gettext(c"E727: Start past end".as_ptr()));
            return;
        }

        tv_list_alloc_ret(rettv, ((end - start) / stride) as isize);
        let mut i = start;
        while if stride > 0 { i <= end } else { i >= end } {
            tv_list_append_number((*rettv).vval.v_list, i);
            i += stride;
        }
    }
}

/// "getreginfo()" function
pub unsafe fn f_getreginfo(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }
        if regname == b'@' as c_int {
            regname = b'"' as c_int;
        }

        tv_dict_alloc_ret(rettv);
        let dict = (*rettv).vval.v_dict.cast::<dict_T>();

        let list = get_reg_contents(regname, kGRegExprSrc | kGRegList).cast::<list_T>();
        if list.is_null() {
            return;
        }
        tv_dict_add_list(dict, c"regcontents".as_ptr(), 11, list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        let mut reglen: colnr_T = 0;
        let mut buflen = match get_reg_type(regname, &raw mut reglen) {
            crate::register::MotionType::kMTLineWise => {
                buf[0] = b'V' as c_char;
                buf[1] = NUL;
                1
            }
            crate::register::MotionType::kMTCharWise => {
                buf[0] = b'v' as c_char;
                buf[1] = NUL;
                1
            }
            crate::register::MotionType::kMTBlockWise => vim_snprintf_safelen!(
                buf.as_mut_ptr(),
                buf.len(),
                c"%c%d".as_ptr(),
                c_int::from(Ctrl_V),
                reglen + 1,
            ),
            crate::register::MotionType::kMTUnknown => std::process::abort(),
        };
        tv_dict_add_str_len(dict, c"regtype".as_ptr(), 7, buf.as_ptr(), buflen as c_int);

        buf[0] = get_register_name(get_unname_register()) as c_char;
        buf[1] = NUL;
        buflen = usize::from(buf[0] != NUL);
        if regname == b'"' as c_int {
            tv_dict_add_str_len(
                dict,
                c"points_to".as_ptr(),
                9,
                buf.as_ptr(),
                buflen as c_int,
            );
        } else {
            tv_dict_add_bool(
                dict,
                c"isunnamed".as_ptr(),
                9,
                if regname == c_int::from(buf[0]) {
                    kBoolVarTrue
                } else {
                    kBoolVarFalse
                },
            );
        }
    }
}

unsafe fn return_register(regname: c_int, rettv: *mut typval_T) {
    unsafe {
        let buf = [regname as c_char, NUL];
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(buf.as_ptr().cast(), usize::from(buf[0] != NUL)).cast();
    }
}

/// "reg_executing()" function
pub unsafe fn f_reg_executing(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        return_register(reg_executing, rettv);
    }
}

/// "reg_recording()" function
pub unsafe fn f_reg_recording(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        return_register(reg_recording, rettv);
    }
}

pub unsafe fn f_reg_recorded(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        return_register(reg_recorded, rettv);
    }
}

unsafe fn list2proftime(arg: *mut typval_T, tm: *mut proftime_T) -> c_int {
    unsafe {
        if (*arg).v_type != VarType::VAR_LIST || tv_list_len((*arg).vval.v_list) != 2 {
            return FAIL;
        }
        let mut error = false;
        let n1 = tv_list_find_nr((*arg).vval.v_list, 0, &raw mut error);
        let n2 = tv_list_find_nr((*arg).vval.v_list, 1, &raw mut error);
        if error {
            return FAIL;
        }
        let high = (n1 as i32) as u32 as u64;
        let low = (n2 as i32) as u32 as u64;
        *tm = (high << 32) | low;
        OK
    }
}

/// "reltime()" function
pub unsafe fn f_reltime(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut res: proftime_T = 0;
        let mut start: proftime_T = 0;

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            res = profile_start();
        } else if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            if list2proftime(argvars, &raw mut res) == FAIL {
                return;
            }
            res = profile_end(res);
        } else {
            if list2proftime(argvars, &raw mut start) == FAIL
                || list2proftime(argvars.add(1), &raw mut res) == FAIL
            {
                return;
            }
            res = profile_sub(res, start);
        }

        let high = (res >> 32) as i32;
        let low = (res & 0xffff_ffff) as i32;

        tv_list_alloc_ret(rettv, 2);
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(high));
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(low));
    }
}

/// "reltimestr()" function
pub unsafe fn f_reltimestr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut tm: proftime_T = 0;
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if list2proftime(argvars, &raw mut tm) == OK {
            (*rettv).vval.v_string = xstrdup(profile_msg(tm));
        }
    }
}

unsafe fn repeat_list(l: *mut list_T, mut n: varnumber_T, rettv: *mut typval_T) {
    unsafe {
        let slen = tv_list_len(l);
        tv_list_alloc_ret(
            rettv,
            (varnumber_T::from(n > 0) * n * varnumber_T::from(slen)) as isize,
        );
        if l.is_null() || n <= 0 || slen == 0 {
            return;
        }
        if check_repeat_count(varnumber_T::from(slen), n) == FAIL {
            return;
        }
        while n > 0 {
            n -= 1;
            tv_list_extend((*rettv).vval.v_list, l, ptr::null_mut());
        }
    }
}

unsafe fn repeat_blob(blob_tv: *mut typval_T, n: varnumber_T, rettv: *mut typval_T) {
    unsafe {
        let blob = (*blob_tv).vval.v_blob.cast::<blob_T>();
        tv_blob_alloc_ret(rettv);
        if blob.is_null() || n <= 0 {
            return;
        }
        let slen = (*blob).bv_ga.len() as c_int;
        if slen <= 0 {
            return;
        }
        if check_repeat_count(varnumber_T::from(slen), n) == FAIL {
            return;
        }
        let len = slen * (n as c_int);
        let ret_blob = (*rettv).vval.v_blob.cast::<blob_T>();
        let ret_bv = &mut (*ret_blob).bv_ga;
        ret_bv.resize(len as usize, 0);

        let mut i: c_int = 0;
        while i < slen {
            if tv_blob_get(blob, i) != 0 {
                break;
            }
            i += 1;
        }
        if i == slen {
            return;
        }
        for idx in 0..(n as c_int) {
            tv_blob_set_range(
                ret_blob,
                varnumber_T::from(idx * slen),
                varnumber_T::from((idx + 1) * slen - 1),
                blob_tv,
            );
        }
    }
}

unsafe fn repeat_string(str_tv: *mut typval_T, n: varnumber_T, rettv: *mut typval_T) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if n <= 0 {
            return;
        }
        let p = tv_get_string(str_tv);
        let slen = strlen(p);
        if slen == 0 || n <= 0 {
            return;
        }
        if check_repeat_count(slen as varnumber_T, n) == FAIL {
            return;
        }
        let len = slen * (n as usize);
        let r = xmallocz(len).cast::<c_char>();
        ptr::copy(p, r, slen);
        let mut done = slen;
        while done < len {
            let copy_len = done.min(len - done);
            ptr::copy(r, r.add(done), copy_len);
            done += copy_len;
        }
        (*rettv).vval.v_string = r;
    }
}

/// "repeat()" function
pub unsafe fn f_repeat(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let n = tv_get_number(argvars.add(1));
        if (*argvars).v_type == VarType::VAR_LIST {
            repeat_list((*argvars).vval.v_list, n, rettv);
        } else if (*argvars).v_type == VarType::VAR_BLOB {
            repeat_blob(argvars, n, rettv);
        } else {
            repeat_string(argvars, n, rettv);
        }
    }
}

unsafe fn reduce_list(argvars: *mut typval_T, expr: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let l = (*argvars).vval.v_list;
        let called_emsg_start = called_emsg;

        let initial: typval_T;
        let mut li: *const listitem_T;
        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            if tv_list_len(l) == 0 {
                semsg!(
                    gettext(e_reduce_of_an_empty_str_with_no_initial_value.as_ptr()),
                    c"List".as_ptr(),
                );
                return;
            }
            let first = tv_list_first(l);
            initial = (*first).li_tv;
            li = (*first).li_next;
        } else {
            initial = *argvars.add(2);
            li = tv_list_first(l);
        }

        tv_copy(&raw const initial, rettv);
        if l.is_null() {
            return;
        }

        let prev_locked = tv_list_locked(l);
        tv_list_set_lock(l, VAR_FIXED);
        while !li.is_null() {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            argv[1] = (*li).li_tv;
            (*rettv).v_type = VarType::VAR_UNKNOWN;

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, rettv);
            tv_clear(&raw mut argv[0]);
            if r == FAIL || called_emsg != called_emsg_start {
                break;
            }
            li = (*li).li_next;
        }
        tv_list_set_lock(l, prev_locked);
    }
}

unsafe fn reduce_string(argvars: *mut typval_T, expr: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let mut p = tv_get_string(argvars);
        let mut len: c_int;
        let called_emsg_start = called_emsg;

        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            if *p == NUL {
                semsg!(
                    gettext(e_reduce_of_an_empty_str_with_no_initial_value.as_ptr()),
                    c"String".as_ptr(),
                );
                return;
            }
            len = utfc_ptr2len(p);
            *rettv = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: xmemdupz(p.cast(), len as usize).cast(),
                },
            };
            p = p.add(len as usize);
        } else if tv_check_for_string_arg(argvars, 2) == FAIL {
            return;
        } else {
            tv_copy(argvars.add(2), rettv);
        }

        while *p != NUL {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            (*rettv).v_type = VarType::VAR_UNKNOWN;
            len = utfc_ptr2len(p);
            argv[1] = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: xmemdupz(p.cast(), len as usize).cast(),
                },
            };

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, rettv);
            tv_clear(&raw mut argv[0]);
            tv_clear(&raw mut argv[1]);
            if r == FAIL || called_emsg != called_emsg_start {
                break;
            }
            p = p.add(len as usize);
        }
    }
}

unsafe fn reduce_blob(argvars: *mut typval_T, expr: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let b = (*argvars).vval.v_blob.cast::<blob_T>();
        let called_emsg_start = called_emsg;

        let initial: typval_T;
        let mut i: c_int;
        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            if tv_blob_len(b) == 0 {
                semsg!(
                    gettext(e_reduce_of_an_empty_str_with_no_initial_value.as_ptr()),
                    c"Blob".as_ptr(),
                );
                return;
            }
            initial = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_number: varnumber_T::from(tv_blob_get(b, 0)),
                },
            };
            i = 1;
        } else if tv_check_for_number_arg(argvars, 2) == FAIL {
            return;
        } else {
            initial = *argvars.add(2);
            i = 0;
        }

        tv_copy(&raw const initial, rettv);
        while i < tv_blob_len(b) {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            argv[1] = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_number: varnumber_T::from(tv_blob_get(b, i)),
                },
            };

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, rettv);
            if r == FAIL || called_emsg != called_emsg_start {
                return;
            }
            i += 1;
        }
    }
}

/// "reduce()" function
pub unsafe fn f_reduce(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_STRING
            && (*argvars).v_type != VarType::VAR_LIST
            && (*argvars).v_type != VarType::VAR_BLOB
        {
            emsg(gettext(E_STRING_LIST_OR_BLOB_REQUIRED.as_ptr()));
            return;
        }

        let func_name = if (*argvars.add(1)).v_type == VarType::VAR_FUNC {
            (*argvars.add(1)).vval.v_string.cast_const()
        } else if (*argvars.add(1)).v_type == VarType::VAR_PARTIAL {
            partial_name((*argvars.add(1)).vval.v_partial)
        } else {
            tv_get_string(argvars.add(1))
        };
        if func_name.is_null() || *func_name == NUL {
            emsg(gettext(E_MISSING_FUNCTION_ARGUMENT.as_ptr()));
            return;
        }

        if (*argvars).v_type == VarType::VAR_LIST {
            reduce_list(argvars, argvars.add(1), rettv);
        } else if (*argvars).v_type == VarType::VAR_STRING {
            reduce_string(argvars, argvars.add(1), rettv);
        } else {
            reduce_blob(argvars, argvars.add(1), rettv);
        }
    }
}

unsafe fn get_search_arg(varp: *mut typval_T, flagsp: *mut c_int) -> c_int {
    unsafe {
        let mut dir = FORWARD;
        if (*varp).v_type == VarType::VAR_UNKNOWN {
            return FORWARD;
        }
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut flags = tv_get_string_buf_chk(varp, nbuf.as_mut_ptr());
        if flags.is_null() {
            return 0;
        }
        while *flags != NUL {
            match *flags as u8 {
                b'b' => dir = BACKWARD,
                b'w' => p_ws = 1,
                b'W' => p_ws = 0,
                b => {
                    let mut mask: c_int = 0;
                    if !flagsp.is_null() {
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
                        semsg!(gettext(e_invarg2.as_ptr()), flags);
                        dir = 0;
                    } else {
                        *flagsp |= mask;
                    }
                }
            }
            if dir == 0 {
                break;
            }
            flags = flags.add(1);
        }
        dir
    }
}

unsafe fn search_cmn(argvars: *mut typval_T, match_pos: *mut pos_T, flagsp: *mut c_int) -> c_int {
    unsafe {
        let save_p_ws = p_ws;
        let mut retval: c_int = 0;
        let mut lnum_stop: linenr_T = 0;
        let mut time_limit: i64 = 0;
        let mut options = SEARCH_KEEP;
        let mut use_skip = false;

        'theend: {
            let pat = tv_get_string(argvars);
            let dir = get_search_arg(argvars.add(1), flagsp);
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

            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            {
                lnum_stop = tv_get_number_chk(argvars.add(2), ptr::null_mut()) as linenr_T;
                if lnum_stop < 0 {
                    break 'theend;
                }
                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    time_limit = tv_get_number_chk(argvars.add(3), ptr::null_mut());
                    if time_limit < 0 {
                        break 'theend;
                    }
                    use_skip = eval_expr_valid_arg(argvars.add(4));
                }
            }

            let mut tm = profile_setlimit(time_limit);

            if ((flags & (SP_REPEAT | SP_RETCOUNT)) != 0)
                || ((flags & SP_NOMOVE) != 0 && (flags & SP_SETPCMARK) != 0)
            {
                semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(1)));
                break 'theend;
            }

            let save_cursor = (*curwin).w_cursor;
            let mut pos = pos_T {
                lnum: save_cursor.lnum,
                col: save_cursor.col,
                coladd: save_cursor.coladd,
            };
            let mut firstpos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut sia = searchit_arg_T {
                sa_stop_lnum: lnum_stop,
                sa_tm: &raw mut tm,
                sa_timed_out: 0,
                sa_wrapped: 0,
            };

            let patlen = strlen(pat);
            let mut subpatnum: c_int;

            loop {
                subpatnum = searchit(
                    curwin,
                    curbuf,
                    pos.cast(),
                    ptr::null_mut(),
                    dir,
                    pat.cast_mut(),
                    patlen,
                    1,
                    options,
                    RE_SEARCH,
                    p_magic != 0,
                    &raw mut sia,
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

                let save_pos = (*curwin).w_cursor;
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                let mut err = false;
                let do_skip = eval_expr_to_bool(argvars.add(4), &raw mut err);
                (*curwin).w_cursor = save_pos;
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
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                if !match_pos.is_null() {
                    (*match_pos).lnum = pos.lnum;
                    (*match_pos).col = pos.col + 1;
                }
                check_cursor(curwin);
            }

            if (flags & SP_NOMOVE) != 0 {
                (*curwin).w_cursor = save_cursor;
            } else {
                (*curwin).w_set_curswant = true;
            }
        }
        p_ws = save_p_ws;
        retval
    }
}

/// "rpcnotify()" function
pub unsafe fn f_rpcnotify(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER || (*argvars).vval.v_number < 0 {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                c"Channel id must be a positive integer".as_ptr(),
            );
            return;
        }
        if (*argvars.add(1)).v_type != VarType::VAR_STRING {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                c"Event type must be a string".as_ptr(),
            );
            return;
        }

        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
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

        let mut tv = argvars.add(2);
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, true);
            args.size += 1;
            tv = tv.add(1);
        }

        let ok = rpc_send_event(
            (*argvars).vval.v_number as u64,
            tv_get_string(argvars.add(1)),
            args,
        );

        arena_mem_free(arena_finish(&raw mut arena));

        if !ok {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                c"Channel doesn't exist".as_ptr(),
            );
            return;
        }
        (*rettv).vval.v_number = 1;
    }
}

/// "rpcrequest()" function
pub unsafe fn f_rpcrequest(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;
        let l_provider_call_nesting = provider_call_nesting;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER || (*argvars).vval.v_number <= 0 {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                c"Channel id must be a positive integer".as_ptr(),
            );
            return;
        }
        if (*argvars.add(1)).v_type != VarType::VAR_STRING {
            semsg!(
                gettext(e_invarg2.as_ptr()),
                c"Method name must be a string".as_ptr(),
            );
            return;
        }

        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
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

        let mut tv = argvars.add(2);
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, true);
            args.size += 1;
            tv = tv.add(1);
        }

        let mut save_current_sctx = sctx_T::new();
        let mut save_autocmd_fname: *mut c_char = ptr::null_mut();
        let mut save_autocmd_match: *mut c_char = ptr::null_mut();
        let mut save_autocmd_fname_full = false;
        let mut save_autocmd_bufnr: c_int = 0;

        if l_provider_call_nesting != 0 {
            save_current_sctx = current_sctx;
            save_autocmd_fname = autocmd_fname;
            save_autocmd_match = autocmd_match;
            save_autocmd_fname_full = autocmd_fname_full;
            save_autocmd_bufnr = autocmd_bufnr;
            save_funccal();

            current_sctx = provider_caller_scope.script_ctx;
            let es = &raw mut exestack;
            (*es).push(provider_caller_scope.es_entry);
            autocmd_fname = provider_caller_scope.autocmd_fname;
            autocmd_match = provider_caller_scope.autocmd_match;
            autocmd_fname_full = provider_caller_scope.autocmd_fname_full;
            autocmd_bufnr = provider_caller_scope.autocmd_bufnr;
            set_current_funccal(provider_caller_scope.funccalp);
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let chan_id = (*argvars).vval.v_number as u64;
        let method = tv_get_string(argvars.add(1));

        let mut res_mem: ArenaMem = ptr::null_mut();
        let result = rpc_send_call(chan_id, method, args, &raw mut res_mem, &raw mut err);
        arena_mem_free(arena_finish(&raw mut arena));

        if l_provider_call_nesting != 0 {
            current_sctx = save_current_sctx;
            let es = &raw mut exestack;
            (*es).pop();
            autocmd_fname = save_autocmd_fname;
            autocmd_match = save_autocmd_match;
            autocmd_fname_full = save_autocmd_fname_full;
            autocmd_bufnr = save_autocmd_bufnr;
            restore_funccal();
        }

        if err.err_type != ErrorType::kErrorTypeNone {
            let mut name: *const c_char = ptr::null();
            let chan = find_channel(chan_id);
            if !chan.is_null() {
                name = get_client_info(chan, c"name".as_ptr());
            }
            if !name.is_null() {
                semsg_multiline!(
                    c"rpc_error".as_ptr(),
                    c"Invoking '%s' on channel %llu (%s):\n%s".as_ptr(),
                    method,
                    chan_id,
                    name,
                    err.msg,
                );
            } else {
                semsg_multiline!(
                    c"rpc_error".as_ptr(),
                    c"Invoking '%s' on channel %llu:\n%s".as_ptr(),
                    method,
                    chan_id,
                    err.msg,
                );
            }
        } else {
            object_to_vim(result, rettv, &raw mut err);
        }

        arena_mem_free(res_mem);
        api_clear_error(&raw mut err);
    }
}

unsafe fn screenchar_adjust(grid: *mut *mut ScreenGrid, row: *mut c_int, col: *mut c_int) {
    unsafe {
        msg_scroll_flush();
        *grid = ui_comp_get_grid_at_coord(*row, *col);
        *row -= (**grid).comp_row;
        *col -= (**grid).comp_col;
    }
}

/// "screenattr()" function
pub unsafe fn f_screenattr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);

        let grid = &*grid;
        let c = if row < 0 || row >= grid.rows || col < 0 || col >= grid.cols {
            -1
        } else {
            let line_off = grid.line_offset[row as usize];
            grid.attrs[line_off + (col as usize)]
        };
        (*rettv).vval.v_number = varnumber_T::from(c);
    }
}

/// "screenchar()" function
pub unsafe fn f_screenchar(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);

        (*rettv).vval.v_number = if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols
        {
            -1
        } else {
            varnumber_T::from(schar_get_first_codepoint(grid_getchar(
                grid,
                row,
                col,
                ptr::null_mut(),
            )))
        };
    }
}

/// "screenchars()" function
pub unsafe fn f_screenchars(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);

        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols {
            return;
        }

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
        schar_get(
            buf.as_mut_ptr(),
            grid_getchar(grid, row, col, ptr::null_mut()),
        );

        let mut i: usize = 0;
        loop {
            let c = utf_ptr2char(buf.as_ptr().add(i));
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(c));
            i += utf_ptr2len(buf.as_ptr().add(i)) as usize;
            if buf[i] == NUL {
                break;
            }
        }
    }
}

/// "screencol()" function
pub unsafe fn f_screencol(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(ui_current_col() + 1);
    }
}

/// "screenrow()" function
pub unsafe fn f_screenrow(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(ui_current_row() + 1);
    }
}

/// "screenstring()" function
pub unsafe fn f_screenstring(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_string = ptr::null_mut();
        (*rettv).v_type = VarType::VAR_STRING;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);
        if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols {
            return;
        }

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
        schar_get(
            buf.as_mut_ptr(),
            grid_getchar(grid, row, col, ptr::null_mut()),
        );
        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
    }
}

/// "search()" function
pub unsafe fn f_search(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut flags: c_int = 0;
        (*rettv).vval.v_number =
            varnumber_T::from(search_cmn(argvars, ptr::null_mut(), &raw mut flags));
    }
}

/// "searchdecl()" function
pub unsafe fn f_searchdecl(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut locally: c_int = 1;
        let mut thisblock: c_int = 0;
        let mut error = false;

        (*rettv).vval.v_number = 1;

        let name = tv_get_string_chk(argvars);
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            locally = c_int::from(tv_get_bool_chk(argvars.add(1), &raw mut error) == 0);
            if !error && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                thisblock = tv_get_bool_chk(argvars.add(2), &raw mut error) as c_int;
            }
        }
        if !error && !name.is_null() {
            (*rettv).vval.v_number = varnumber_T::from(!find_decl(
                name.cast_mut(),
                strlen(name),
                locally != 0,
                thisblock != 0,
                SEARCH_KEEP,
            ));
        }
    }
}

unsafe fn searchpair_cmn(argvars: *mut typval_T, match_pos: *mut pos_T) -> c_int {
    unsafe {
        let save_p_ws = p_ws;
        let mut flags: c_int = 0;
        let mut retval: c_int = 0;
        let mut lnum_stop: linenr_T = 0;
        let mut time_limit: i64 = 0;

        'theend: {
            let mut nbuf1 = [0 as c_char; NUMBUFLEN];
            let mut nbuf2 = [0 as c_char; NUMBUFLEN];
            let spat = tv_get_string_chk(argvars);
            let mpat = tv_get_string_buf_chk(argvars.add(1), nbuf1.as_mut_ptr());
            let epat = tv_get_string_buf_chk(argvars.add(2), nbuf2.as_mut_ptr());
            if spat.is_null() || mpat.is_null() || epat.is_null() {
                break 'theend;
            }

            let dir = get_search_arg(argvars.add(3), &raw mut flags);
            if dir == 0 {
                break 'theend;
            }

            if ((flags & (SP_END | SP_SUBPAT)) != 0)
                || ((flags & SP_NOMOVE) != 0 && (flags & SP_SETPCMARK) != 0)
            {
                semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(3)));
                break 'theend;
            }

            if (flags & SP_REPEAT) != 0 {
                p_ws = 0;
            }

            let skip: *const typval_T = if (*argvars.add(3)).v_type == VarType::VAR_UNKNOWN
                || (*argvars.add(4)).v_type == VarType::VAR_UNKNOWN
            {
                ptr::null()
            } else {
                let s = argvars.add(4).cast_const();
                if (*argvars.add(5)).v_type != VarType::VAR_UNKNOWN {
                    lnum_stop = tv_get_number_chk(argvars.add(5), ptr::null_mut()) as linenr_T;
                    if lnum_stop < 0 {
                        semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(5)));
                        break 'theend;
                    }
                    if (*argvars.add(6)).v_type != VarType::VAR_UNKNOWN {
                        time_limit = tv_get_number_chk(argvars.add(6), ptr::null_mut());
                        if time_limit < 0 {
                            semsg!(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(6)));
                            break 'theend;
                        }
                    }
                }
                s
            };

            retval = do_searchpair(
                spat, mpat, epat, dir, skip, flags, match_pos, lnum_stop, time_limit,
            );
        }

        p_ws = save_p_ws;
        retval
    }
}

/// "searchpair()" function
pub unsafe fn f_searchpair(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(searchpair_cmn(argvars, ptr::null_mut()));
    }
}

/// "searchpairpos()" function
pub unsafe fn f_searchpairpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut lnum: c_int = 0;
        let mut col: c_int = 0;

        tv_list_alloc_ret(rettv, 2);
        if searchpair_cmn(argvars, &raw mut match_pos) > 0 {
            lnum = match_pos.lnum;
            col = match_pos.col;
        }
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(lnum));
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(col));
    }
}

/// Search for a start/middle/end thing.
pub unsafe fn do_searchpair(
    spat: *const c_char,
    mpat: *const c_char,
    epat: *const c_char,
    dir: c_int,
    skip: *const typval_T,
    flags: c_int,
    match_pos: *mut pos_T,
    lnum_stop: linenr_T,
    time_limit: i64,
) -> c_int {
    unsafe {
        let mut retval: c_int = 0;
        let mut nest: c_int = 1;
        let mut use_skip = false;
        let mut options = SEARCH_KEEP;

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut tm = profile_setlimit(time_limit);

        let spatlen = strlen(spat);
        let epatlen = strlen(epat);
        let pat2size = spatlen + epatlen + 17;
        let mut pat2_vec = vec![0 as c_char; pat2size];
        let pat2 = pat2_vec.as_mut_ptr();
        let pat3size = spatlen + strlen(mpat) + epatlen + 25;
        let mut pat3_vec = vec![0 as c_char; pat3size];
        let pat3 = pat3_vec.as_mut_ptr();
        let pat2len = vim_snprintf!(
            pat2,
            pat2size,
            c"\\m\\(%s\\m\\)\\|\\(%s\\m\\)".as_ptr(),
            spat,
            epat,
        );
        let pat3len = if *mpat == NUL {
            ptr::copy_nonoverlapping(pat2, pat3, (pat2len as usize) + 1);
            pat2len
        } else {
            vim_snprintf!(
                pat3,
                pat3size,
                c"\\m\\(%s\\m\\)\\|\\(%s\\m\\)\\|\\(%s\\m\\)".as_ptr(),
                spat,
                epat,
                mpat,
            )
        };
        if (flags & SP_START) != 0 {
            options |= SEARCH_START;
        }
        if !skip.is_null() {
            use_skip = eval_expr_valid_arg(skip);
        }

        let save_cursor = (*curwin).w_cursor;
        let mut pos = pos_T {
            lnum: save_cursor.lnum,
            col: save_cursor.col,
            coladd: save_cursor.coladd,
        };
        let mut firstpos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        clearpos(&mut firstpos);
        let mut foundpos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        clearpos(&mut foundpos);
        let mut pat = pat3;
        debug_assert!(pat3len >= 0);
        let patlen = pat3len as usize;

        loop {
            let mut sia = searchit_arg_T {
                sa_stop_lnum: lnum_stop,
                sa_tm: &raw mut tm,
                sa_timed_out: 0,
                sa_wrapped: 0,
            };

            let n = searchit(
                curwin,
                curbuf,
                pos.cast(),
                ptr::null_mut(),
                dir,
                pat,
                patlen,
                1,
                options,
                RE_SEARCH,
                true,
                &raw mut sia,
            );
            if n == FAIL || (firstpos.lnum != 0 && equalpos(pos, firstpos)) {
                break;
            }
            if firstpos.lnum == 0 {
                firstpos = pos;
            }
            if equalpos(pos, foundpos) {
                if dir == BACKWARD {
                    decl(&raw mut pos);
                } else {
                    incl(&raw mut pos);
                }
            }
            foundpos = pos;

            options &= !SEARCH_START;

            if use_skip {
                let save_pos = (*curwin).w_cursor;
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                let mut err = false;
                let r = eval_expr_to_bool(skip, &raw mut err);
                (*curwin).w_cursor = save_pos;
                if err {
                    (*curwin).w_cursor = save_cursor;
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
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                if (flags & SP_REPEAT) == 0 {
                    break;
                }
                nest = 1;
            }
        }

        if !match_pos.is_null() {
            (*match_pos).lnum = (*curwin).w_cursor.lnum;
            (*match_pos).col = (*curwin).w_cursor.col + 1;
        }

        if (flags & SP_NOMOVE) != 0 || retval == 0 {
            (*curwin).w_cursor = save_cursor;
        }

        drop(pat2_vec);
        drop(pat3_vec);
        if p_cpo == (&raw mut empty_string_option).cast() {
            p_cpo = save_cpo;
        } else {
            if *p_cpo == NUL {
                set_option_value_give_err(
                    kOptCpoptions,
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: cstr_as_string(save_cpo),
                        },
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
pub unsafe fn f_searchpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut flags: c_int = 0;

        let n = search_cmn(argvars, &raw mut match_pos, &raw mut flags);
        tv_list_alloc_ret(rettv, 2 + isize::from((flags & SP_SUBPAT) != 0));

        let lnum = if n > 0 { match_pos.lnum } else { 0 };
        let col = if n > 0 { match_pos.col } else { 0 };

        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(lnum));
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(col));
        if (flags & SP_SUBPAT) != 0 {
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(n));
        }
    }
}

/// "serverlist()" function
pub unsafe fn f_serverlist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let addrs = server_address_list();

        let mut addrs_tv = tv_initial_value();
        tv_list_alloc_ret(&raw mut addrs_tv, addrs.len() as isize);
        for addr in addrs {
            tv_list_append_allocated_string(addrs_tv.vval.v_list, addr);
        }

        let opts = if (*argvars).v_type != VarType::VAR_UNKNOWN {
            *argvars
        } else {
            typval_T {
                v_type: VarType::VAR_SPECIAL,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_special: kSpecialVarNull,
                },
            }
        };
        let mut lua_args = [opts, addrs_tv, tv_initial_value()];
        nlua_call_typval(
            c"vim._core.server".as_ptr(),
            c"serverlist".as_ptr(),
            lua_args.as_mut_ptr(),
            rettv,
        );
        tv_clear(&raw mut addrs_tv);
    }
}

/// "serverstart()" function
pub unsafe fn f_serverstart(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if check_secure() {
            return;
        }

        let address = if (*argvars).v_type != VarType::VAR_UNKNOWN {
            if (*argvars).v_type != VarType::VAR_STRING {
                emsg(gettext(e_invarg.as_ptr()));
                return;
            }
            to_slash_save(tv_get_string(argvars))
        } else {
            server_address_new(ptr::null())
        };

        let result = server_start(address);
        xfree(address.cast());

        if result != 0 {
            semsg!(
                c"Failed to start server: %s".as_ptr(),
                if result > 0 {
                    c"Unknown system error".as_ptr()
                } else {
                    uv_strerror(result)
                },
            );
            return;
        }

        let mut addrs = server_address_list();
        (*rettv).vval.v_string = addrs.pop().unwrap_or(ptr::null_mut());

        for addr in addrs {
            xfree(addr.cast());
        }
    }
}

/// "serverstop()" function
pub unsafe fn f_serverstop(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_STRING {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;
        if !(*argvars).vval.v_string.is_null() {
            let rv = server_stop((*argvars).vval.v_string, false);
            (*rettv).vval.v_number = varnumber_T::from(rv);
        }
    }
}

unsafe fn set_position(argvars: *mut typval_T, rettv: *mut typval_T, charpos: bool) {
    unsafe {
        let mut curswant: colnr_T = -1;
        (*rettv).vval.v_number = -1;
        let name = tv_get_string_chk(argvars);
        if name.is_null() {
            return;
        }

        let mut pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut fnum: c_int = 0;
        if list2fpos(
            argvars.add(1),
            &raw mut pos,
            &raw mut fnum,
            &raw mut curswant,
            charpos,
        ) != OK
        {
            return;
        }

        if pos.col != MAXCOL {
            pos.col -= 1;
            if pos.col < 0 {
                pos.col = 0;
            }
        }
        if *name == b'.' as c_char && *name.add(1) == NUL {
            (*curwin).w_cursor.lnum = pos.lnum;
            (*curwin).w_cursor.col = pos.col;
            (*curwin).w_cursor.coladd = pos.coladd;
            if curswant >= 0 {
                (*curwin).w_curswant = curswant - 1;
                (*curwin).w_set_curswant = false;
            }
            check_cursor(curwin);
            (*rettv).vval.v_number = 0;
        } else if *name == b'\'' as c_char && *name.add(1) != NUL && *name.add(2) == NUL {
            if setmark_pos(
                c_int::from(*name.add(1) as u8),
                pos.cast(),
                fnum,
                ptr::null_mut(),
            ) == OK
            {
                (*rettv).vval.v_number = 0;
            }
        } else {
            emsg(gettext(e_invarg.as_ptr()));
        }
    }
}

/// "setcharpos()" function
pub unsafe fn f_setcharpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        set_position(argvars, rettv, true);
    }
}

pub unsafe fn f_setcharsearch(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_dict_arg(argvars, 0) == FAIL {
            return;
        }
        let d = (*argvars).vval.v_dict.cast::<dict_T>();
        if d.is_null() {
            return;
        }

        let csearch = tv_dict_get_string(d, c"char".as_ptr(), false);
        if !csearch.is_null() {
            let c = utf_ptr2char(csearch);
            set_last_csearch(c, csearch, utfc_ptr2len(csearch));
        }

        let mut di = tv_dict_find(d, c"forward".as_ptr(), 7);
        if !di.is_null() {
            set_csearch_direction(if tv_get_number(&raw const (*di).di_tv) != 0 {
                FORWARD
            } else {
                BACKWARD
            });
        }

        di = tv_dict_find(d, c"until".as_ptr(), 5);
        if !di.is_null() {
            set_csearch_until(c_int::from(tv_get_number(&raw const (*di).di_tv) != 0));
        }
    }
}

/// "setcursorcharpos()" function
pub unsafe fn f_setcursorcharpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        set_cursorpos(argvars, rettv, true);
    }
}

/// "setenv()" function
pub unsafe fn f_setenv(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut namebuf = [0 as c_char; NUMBUFLEN];
        let mut valbuf = [0 as c_char; NUMBUFLEN];
        let name = tv_get_string_buf(argvars, namebuf.as_mut_ptr());

        if check_secure() {
            return;
        }
        if (*argvars.add(1)).v_type == VarType::VAR_SPECIAL
            && (*argvars.add(1)).vval.v_special as c_int == kSpecialVarNull
        {
            vim_unsetenv_ext(name);
        } else {
            vim_setenv_ext(name, tv_get_string_buf(argvars.add(1), valbuf.as_mut_ptr()));
        }
    }
}

/// "setfperm({fname}, {mode})" function
pub unsafe fn f_setfperm(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = 0;

        let fname = tv_get_string_chk(argvars);
        if fname.is_null() {
            return;
        }
        let mut modebuf = [0 as c_char; NUMBUFLEN];
        let mode_str = tv_get_string_buf_chk(argvars.add(1), modebuf.as_mut_ptr());
        if mode_str.is_null() {
            return;
        }
        if strlen(mode_str) != 9 {
            semsg!(gettext(e_invarg2.as_ptr()), mode_str);
            return;
        }

        let mut mask: c_int = 1;
        let mut mode: c_int = 0;
        for i in (0..9).rev() {
            if *mode_str.add(i) != b'-' as c_char {
                mode |= mask;
            }
            mask <<= 1;
        }
        (*rettv).vval.v_number = varnumber_T::from(os_setperm(fname, mode) == OK);
    }
}

/// "setpos()" function
pub unsafe fn f_setpos(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        set_position(argvars, rettv, false);
    }
}

unsafe fn get_yank_type(
    pp: *mut *mut c_char,
    yank_type: *mut crate::register::MotionType,
    block_len: *mut c_int,
) -> c_int {
    unsafe {
        let mut stropt = *pp;
        match *stropt {
            c if c == b'v' as c_char || c == b'c' as c_char => {
                *yank_type = crate::register::MotionType::kMTCharWise;
            }
            c if c == b'V' as c_char || c == b'l' as c_char => {
                *yank_type = crate::register::MotionType::kMTLineWise;
            }
            c if c == b'b' as c_char || c == Ctrl_V => {
                *yank_type = crate::register::MotionType::kMTBlockWise;
                if ascii_isdigit(*stropt.add(1)) {
                    stropt = stropt.add(1);
                    *block_len = getdigits_int(&raw mut stropt, false, 0) - 1;
                    stropt = stropt.sub(1);
                }
            }
            _ => return FAIL,
        }
        *pp = stropt;
        OK
    }
}

/// "setreg()" function
pub unsafe fn f_setreg(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut append = false;
        let mut block_len: c_int = -1;
        let mut yank_type = crate::register::MotionType::kMTUnknown;

        (*rettv).vval.v_number = 1;

        let strregname = tv_get_string_chk(argvars);
        if strregname.is_null() {
            return;
        }
        let mut regname = *strregname;
        if regname == 0 || regname == b'@' as c_char {
            regname = b'"' as c_char;
        }

        let mut regcontents: *const typval_T = ptr::null();
        let mut pointreg: c_char = 0;
        if (*argvars.add(1)).v_type == VarType::VAR_DICT {
            let d = (*argvars.add(1)).vval.v_dict.cast::<dict_T>();
            if tv_dict_len(d) == 0 {
                let mut lstval: [*mut c_char; 2] = [ptr::null_mut(), ptr::null_mut()];
                write_reg_contents_lst(
                    c_int::from(regname as u8),
                    lstval.as_mut_ptr(),
                    false,
                    crate::register::MotionType::kMTUnknown,
                    -1,
                );
                return;
            }

            let di = tv_dict_find(d, c"regcontents".as_ptr(), -1);
            if !di.is_null() {
                regcontents = &raw const (*di).di_tv;
            }

            let mut stropt = tv_dict_get_string(d, c"regtype".as_ptr(), false);
            if !stropt.is_null() {
                let ret = get_yank_type(&raw mut stropt, &raw mut yank_type, &raw mut block_len);
                stropt = stropt.add(1);
                if ret == FAIL || *stropt != NUL {
                    semsg!(gettext(e_invargval.as_ptr()), c"value".as_ptr());
                    return;
                }
            }

            if regname == b'"' as c_char {
                stropt = tv_dict_get_string(d, c"points_to".as_ptr(), false);
                if !stropt.is_null() {
                    pointreg = *stropt;
                    regname = pointreg;
                }
            } else if tv_dict_get_bool(d, c"isunnamed".as_ptr(), -1) > 0 {
                pointreg = regname;
            }
        } else {
            regcontents = argvars.add(1);
        }

        let mut set_unnamed = false;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if yank_type != crate::register::MotionType::kMTUnknown {
                semsg!(gettext(e_toomanyarg.as_ptr()), c"setreg".as_ptr());
                return;
            }

            let mut stropt = tv_get_string_chk(argvars.add(2)).cast_mut();
            if stropt.is_null() {
                return;
            }
            while *stropt != NUL {
                match *stropt as u8 {
                    b'a' | b'A' => append = true,
                    b'u' | b'"' => set_unnamed = true,
                    _ => {
                        get_yank_type(&raw mut stropt, &raw mut yank_type, &raw mut block_len);
                    }
                }
                stropt = stropt.add(1);
            }
        }

        if !regcontents.is_null() && (*regcontents).v_type == VarType::VAR_LIST {
            let ll = (*regcontents).vval.v_list;
            let len = tv_list_len(ll) as usize;

            let mut lstval: Vec<*const c_char> = Vec::with_capacity(len + 1);
            let mut allocval: Vec<*mut c_char> = Vec::new();

            'free_lstval: {
                let mut li = tv_list_first(ll);
                while !li.is_null() {
                    let mut buf = [0 as c_char; NUMBUFLEN];
                    let mut val = tv_get_string_buf_chk(&raw const (*li).li_tv, buf.as_mut_ptr());
                    if val.is_null() {
                        break 'free_lstval;
                    }
                    if ptr::eq(val, buf.as_ptr()) {
                        let copied = xstrdup(val);
                        allocval.push(copied);
                        val = copied;
                    }
                    lstval.push(val);
                    li = (*li).li_next;
                }
                lstval.push(ptr::null());

                write_reg_contents_lst(
                    c_int::from(regname as u8),
                    lstval.as_mut_ptr().cast::<*mut c_char>(),
                    append,
                    yank_type,
                    block_len as colnr_T,
                );
            }

            for ptr in allocval {
                xfree(ptr.cast());
            }
        } else if !regcontents.is_null() {
            let strval = tv_get_string_chk(regcontents);
            if strval.is_null() {
                return;
            }
            write_reg_contents_ex(
                c_int::from(regname as u8),
                strval,
                strlen(strval) as isize,
                append,
                yank_type,
                block_len as colnr_T,
            );
        }
        if pointreg != 0 {
            get_yank_register(c_int::from(pointreg as u8), YREG_YANK);
        }
        (*rettv).vval.v_number = 0;

        if set_unnamed {
            op_reg_set_previous(regname);
        }
    }
}

/// "settagstack()" function
pub unsafe fn f_settagstack(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut action: c_char = b'r' as c_char;
        (*rettv).vval.v_number = -1;

        let wp = find_win_by_nr_or_id(argvars);
        if wp.is_null() {
            return;
        }
        if tv_check_for_dict_arg(argvars, 1) == FAIL {
            return;
        }
        let d = (*argvars.add(1)).vval.v_dict.cast::<dict_T>();
        if d.is_null() {
            return;
        }

        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            // default 'r'
        } else if tv_check_for_string_arg(argvars, 2) == FAIL {
            return;
        } else {
            let actstr = tv_get_string_chk(argvars.add(2));
            if actstr.is_null() {
                return;
            }
            if (*actstr == b'r' as c_char || *actstr == b'a' as c_char || *actstr == b't' as c_char)
                && *actstr.add(1) == NUL
            {
                action = *actstr;
            } else {
                semsg!(gettext(E_INVACT2.as_ptr()), actstr);
                return;
            }
        }

        if set_tagstack(wp, d, c_int::from(action)) == OK {
            (*rettv).vval.v_number = 0;
        }
    }
}

/// "sha256({expr})" function
pub unsafe fn f_sha256(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if (*argvars).v_type == VarType::VAR_BLOB {
            let blob = (*argvars).vval.v_blob.cast::<blob_T>();
            let slice: &[u8] = if !blob.is_null() {
                &(*blob).bv_ga
            } else {
                &[]
            };
            let hex = sha256_hex(slice, None);
            (*rettv).vval.v_string = xmemdupz(hex.as_ptr().cast(), hex.len()).cast();
        } else {
            let p = tv_get_string(argvars);
            let len = strlen(p);
            let slice = if len > 0 {
                std::slice::from_raw_parts(p.cast::<u8>(), len)
            } else {
                &[]
            };
            let hex = sha256_hex(slice, None);
            (*rettv).vval.v_string = xmemdupz(hex.as_ptr().cast(), hex.len()).cast();
        }
    }
}

/// "shellescape({string})" function
pub unsafe fn f_shellescape(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let do_special = non_zero_arg(argvars.add(1));
        (*rettv).vval.v_string =
            vim_strsave_shellescape(tv_get_string(argvars), do_special, do_special);
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "shiftwidth()" function
pub unsafe fn f_shiftwidth(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = 0;
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let col = tv_get_number_chk(argvars, ptr::null_mut()) as colnr_T;
            if col < 0 {
                return;
            }
            (*rettv).vval.v_number = varnumber_T::from(get_sw_value_col(curbuf, col, false));
            return;
        }
        (*rettv).vval.v_number = varnumber_T::from(get_sw_value(curbuf));
    }
}

/// "sockconnect()" function
pub unsafe fn f_sockconnect(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_STRING
            || (*argvars.add(1)).v_type != VarType::VAR_STRING
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }
        if (*argvars.add(2)).v_type != VarType::VAR_DICT
            && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
        {
            semsg!(gettext(e_invarg2.as_ptr()), c"expected dictionary".as_ptr());
            return;
        }

        let mode = tv_get_string(argvars);
        let address = to_slash_save(tv_get_string(argvars.add(1)));

        'cleanup: {
            let tcp = if strcmp(mode, c"tcp".as_ptr()) == 0 {
                true
            } else if strcmp(mode, c"pipe".as_ptr()) == 0 {
                false
            } else {
                semsg!(gettext(e_invarg2.as_ptr()), c"invalid mode".as_ptr());
                break 'cleanup;
            };

            let mut rpc = false;
            let mut on_data = CallbackReader::none();
            if (*argvars.add(2)).v_type == VarType::VAR_DICT {
                let opts = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
                rpc = tv_dict_get_number(opts, c"rpc".as_ptr()) != 0;

                if !tv_dict_get_callback(opts, c"on_data".as_ptr(), 7, &raw mut on_data.cb) {
                    break 'cleanup;
                }
                on_data.buffered = tv_dict_get_number(opts, c"data_buffered".as_ptr()) != 0;
                if on_data.buffered && on_data.cb.cb_type() == kCallbackNone {
                    on_data.self_dict = opts;
                }
            }

            let mut error: *const c_char = ptr::null();
            let id = channel_connect(tcp, address, rpc, on_data, 50, &raw mut error);
            if !error.is_null() {
                semsg!(gettext(c"connection failed: %s".as_ptr()), error);
            }

            (*rettv).vval.v_number = id as varnumber_T;
            (*rettv).v_type = VarType::VAR_NUMBER;
        }

        xfree(address.cast());
    }
}

/// "stdioopen()" function
pub unsafe fn f_stdioopen(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_DICT {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut on_stdin = CallbackReader::none();
        let opts = (*argvars).vval.v_dict.cast::<dict_T>();
        let rpc = tv_dict_get_number(opts, c"rpc".as_ptr()) != 0;

        if !tv_dict_get_callback(opts, c"on_stdin".as_ptr(), 8, &raw mut on_stdin.cb) {
            return;
        }
        if !tv_dict_get_callback(opts, c"on_print".as_ptr(), 8, &raw mut on_print) {
            return;
        }

        on_stdin.buffered = tv_dict_get_number(opts, c"stdin_buffered".as_ptr()) != 0;
        if on_stdin.buffered && on_stdin.cb.cb_type() == kCallbackNone {
            on_stdin.self_dict = opts;
        }

        let mut error: *const c_char = ptr::null();
        let id = channel_from_stdio(rpc, on_stdin, &raw mut error);
        if id == 0 {
            semsg!(e_stdiochan2.as_ptr(), error);
        }

        (*rettv).vval.v_number = id as varnumber_T;
        (*rettv).v_type = VarType::VAR_NUMBER;
    }
}

/// "reltimefloat()" function
pub unsafe fn f_reltimefloat(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut tm: proftime_T = 0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        (*rettv).vval.v_float = 0.0;
        if list2proftime(argvars, &raw mut tm) == OK {
            (*rettv).vval.v_float = (profile_signed(tm) as float_T) / (NS_PER_SEC as float_T);
        }
    }
}

/// "soundfold({word})" function
pub unsafe fn f_soundfold(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let s = tv_get_string(argvars);
        (*rettv).vval.v_string = eval_soundfold(s);
    }
}

/// "spellbadword()" function
pub unsafe fn f_spellbadword(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let wo_spell_save = (*curwin).w_onebuf_opt.wo_spell;

        if (*curwin).w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin);
            (*curwin).w_onebuf_opt.wo_spell = 1;
        }

        let ws = (*curwin).w_s.cast::<synblock_T>();
        if *(*ws).b_p_spl == NUL {
            emsg(gettext(e_no_spell.as_ptr()));
            (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
            return;
        }

        let mut word: *const c_char = c"".as_ptr();
        let mut attr: hlf_T = HLF_COUNT;
        let mut len: usize = 0;
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            len = spell_move_to(curwin, FORWARD, SMT_ALL, true, &raw mut attr);
            if len != 0 {
                word = get_cursor_pos_ptr();
                (*curwin).w_set_curswant = true;
            }
        } else if *(*curbuf).b_s.b_p_spl != NUL {
            let mut str_ = tv_get_string_chk(argvars);
            let mut capcol: c_int = -1;
            if !str_.is_null() {
                while *str_ != NUL {
                    len = spell_check(
                        curwin,
                        str_.cast_mut(),
                        &raw mut attr,
                        &raw mut capcol,
                        false,
                    );
                    if attr != HLF_COUNT {
                        word = str_;
                        break;
                    }
                    str_ = str_.add(len);
                    capcol -= len as c_int;
                    len = 0;
                }
            }
        }
        (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;

        debug_assert!(len <= c_int::MAX as usize);
        tv_list_alloc_ret(rettv, 2);
        tv_list_append_string((*rettv).vval.v_list, word, len as isize);
        match attr {
            HLF_SPB => tv_list_append_string((*rettv).vval.v_list, c"bad".as_ptr(), 3),
            HLF_SPR => tv_list_append_string((*rettv).vval.v_list, c"rare".as_ptr(), 4),
            HLF_SPL => tv_list_append_string((*rettv).vval.v_list, c"local".as_ptr(), 5),
            HLF_SPC => tv_list_append_string((*rettv).vval.v_list, c"caps".as_ptr(), 4),
            _ => tv_list_append_string((*rettv).vval.v_list, ptr::null(), -1),
        }
    }
}

/// "spellsuggest()" function
pub unsafe fn f_spellsuggest(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut ga: Vec<*mut c_char> = Vec::new();
        let wo_spell_save = (*curwin).w_onebuf_opt.wo_spell;

        if (*curwin).w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin);
            (*curwin).w_onebuf_opt.wo_spell = 1;
        }

        let ws = (*curwin).w_s.cast::<synblock_T>();
        if *(*ws).b_p_spl == NUL {
            emsg(gettext(e_no_spell.as_ptr()));
            (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
            return;
        }

        'ret: {
            let maxcount: c_int;
            let mut need_capital = false;
            let str_ = tv_get_string(argvars);
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                let mut typeerr = false;
                maxcount = tv_get_number_chk(argvars.add(1), &raw mut typeerr) as c_int;
                if maxcount <= 0 {
                    break 'ret;
                }
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    need_capital = tv_get_bool_chk(argvars.add(2), &raw mut typeerr) != 0;
                    if typeerr {
                        break 'ret;
                    }
                }
            } else {
                maxcount = 25;
            }

            spell_suggest_list(&mut ga, str_.cast_mut(), maxcount, need_capital, false);
        }

        tv_list_alloc_ret(rettv, ga.len() as isize);
        for p in ga {
            tv_list_append_allocated_string((*rettv).vval.v_list, p);
        }
        (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
    }
}

pub unsafe fn f_split(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut col: colnr_T = 0;
        let mut keepempty = false;
        let mut typeerr = false;

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut str_ = tv_get_string(argvars);
        let mut pat: *const c_char = ptr::null();
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
            if pat.is_null() {
                typeerr = true;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                keepempty = tv_get_bool_chk(argvars.add(2), &raw mut typeerr) != 0;
            }
        }
        if pat.is_null() || *pat == NUL {
            pat = c"[\\x01- ]\\+".as_ptr();
        }

        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if !typeerr {
            let mut regmatch = regmatch_T {
                regprog: vim_regcomp(pat, RE_MAGIC + RE_STRING).cast(),
                startp: [ptr::null_mut(); NSUBEXP],
                endp: [ptr::null_mut(); NSUBEXP],
                rm_matchcol: 0,
                rm_ic: false,
            };
            if !regmatch.regprog.is_null() {
                while *str_ != NUL || keepempty {
                    let match_found = if *str_ == NUL {
                        false
                    } else {
                        vim_regexec_nl((&raw mut regmatch).cast(), str_, col)
                    };
                    let end: *const c_char = if match_found {
                        regmatch.startp[0]
                    } else {
                        str_.add(strlen(str_))
                    };
                    if keepempty
                        || end > str_
                        || (tv_list_len((*rettv).vval.v_list) > 0
                            && *str_ != NUL
                            && match_found
                            && end < regmatch.endp[0].cast_const())
                    {
                        tv_list_append_string((*rettv).vval.v_list, str_, end.offset_from(str_));
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
                vim_regfree(regmatch.regprog.cast());
            }
        }

        p_cpo = save_cpo;
    }
}

unsafe fn get_xdg_var_list(xdg: XDGVarType, rettv: *mut typval_T) {
    unsafe {
        let list = tv_list_alloc(kListLenShouldKnow);
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_list = list;
        tv_list_ref(list);
        let dirs = stdpaths_get_xdg_var(xdg);
        if dirs.is_null() {
            return;
        }
        let mut iter: *const c_void = ptr::null();
        let appname = get_appname(false);
        loop {
            let mut dir_len: usize = 0;
            let mut dir: *const c_char = ptr::null();
            iter = vim_env_iter(ENV_SEPCHAR, dirs, iter, &raw mut dir, &raw mut dir_len);
            if !dir.is_null() && dir_len > 0 {
                let mut dir_with_nvim = xmemdupz(dir.cast(), dir_len).cast::<c_char>();
                dir_with_nvim = concat_fnames_realloc(dir_with_nvim, appname, true);
                tv_list_append_allocated_string(list, dir_with_nvim);
            }
            if iter.is_null() {
                break;
            }
        }
        xfree(dirs.cast());
    }
}

/// "stdpath(type)" function
pub unsafe fn f_stdpath(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let p = tv_get_string_chk(argvars);
        if p.is_null() {
            return;
        }

        if strequal(p, c"config".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGConfigHome);
        } else if strequal(p, c"data".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGDataHome);
        } else if strequal(p, c"cache".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGCacheHome);
        } else if strequal(p, c"state".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGStateHome);
        } else if strequal(p, c"log".as_ptr()) {
            (*rettv).vval.v_string = concat_fnames_realloc(
                get_xdg_home(XDGVarType::kXDGStateHome),
                c"logs".as_ptr(),
                true,
            );
        } else if strequal(p, c"run".as_ptr()) {
            (*rettv).vval.v_string = stdpaths_get_xdg_var(XDGVarType::kXDGRuntimeDir);
        } else if strequal(p, c"config_dirs".as_ptr()) {
            get_xdg_var_list(XDGVarType::kXDGConfigDirs, rettv);
        } else if strequal(p, c"data_dirs".as_ptr()) {
            get_xdg_var_list(XDGVarType::kXDGDataDirs, rettv);
        } else {
            semsg!(gettext(c"E6100: \"%s\" is not a valid stdpath".as_ptr()), p);
        }
    }
}

/// "str2float()" function
pub unsafe fn f_str2float(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut p = skipwhite(tv_get_string(argvars));
        let isneg = *p == b'-' as c_char;
        if *p == b'+' as c_char || *p == b'-' as c_char {
            p = skipwhite(p.add(1));
        }
        string2float(p, &raw mut (*rettv).vval.v_float);
        if isneg {
            (*rettv).vval.v_float *= -1.0;
        }
        (*rettv).v_type = VarType::VAR_FLOAT;
    }
}

/// "strftime({format}[, {time}])" function
pub unsafe fn f_strftime(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let mut p = tv_get_string(argvars).cast_mut();
        let seconds: time_t = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            os_time() as time_t
        } else {
            tv_get_number(argvars.add(1)) as time_t
        };

        let mut curtime = Tm { _fields: [0; 16] };
        let curtime_ptr = os_localtime_r(&raw const seconds, &raw mut curtime);
        if curtime_ptr.is_null() {
            (*rettv).vval.v_string = xstrdup(gettext(c"(Invalid)".as_ptr()));
            return;
        }

        let mut conv = vimconv_T::new();
        conv.vc_type = CONV_NONE;
        let enc = enc_locale();
        convert_setup(&raw mut conv, p_enc, enc);
        if conv.vc_type != CONV_NONE {
            p = string_convert(&raw const conv, p, ptr::null_mut());
        }
        let mut result_buf = [0 as c_char; 256];
        if p.is_null() || strftime(result_buf.as_mut_ptr(), result_buf.len(), p, curtime_ptr) == 0 {
            result_buf[0] = NUL;
        }

        if conv.vc_type != CONV_NONE {
            xfree(p.cast());
        }
        convert_setup(&raw mut conv, enc, p_enc);
        if conv.vc_type != CONV_NONE {
            (*rettv).vval.v_string =
                string_convert(&raw const conv, result_buf.as_mut_ptr(), ptr::null_mut());
        } else {
            (*rettv).vval.v_string = xstrdup(result_buf.as_ptr());
        }

        xfree(enc.cast());
    }
}

/// "strptime({format}, {timestring})" function
pub unsafe fn f_strptime(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut fmt_buf = [0 as c_char; NUMBUFLEN];
        let mut str_buf = [0 as c_char; NUMBUFLEN];

        let mut tmval = Tm { _fields: [0; 16] };
        tmval._fields[8] = -1; // tm_isdst = -1
        let mut fmt = tv_get_string_buf(argvars, fmt_buf.as_mut_ptr()).cast_mut();
        let str_ = tv_get_string_buf(argvars.add(1), str_buf.as_mut_ptr());

        let mut conv = vimconv_T::new();
        conv.vc_type = CONV_NONE;
        let enc = enc_locale();
        convert_setup(&raw mut conv, p_enc, enc);
        if conv.vc_type != CONV_NONE {
            fmt = string_convert(&raw const conv, fmt, ptr::null_mut());
        }
        if fmt.is_null() || os_strptime(str_, fmt, &raw mut tmval).is_null() {
            (*rettv).vval.v_number = 0;
        } else {
            let t = mktime(&raw mut tmval);
            (*rettv).vval.v_number = if t == -1 { 0 } else { t as varnumber_T };
        }
        if conv.vc_type != CONV_NONE {
            xfree(fmt.cast());
        }
        xfree(enc.cast());
    }
}

/// "submatch()" function
pub unsafe fn f_submatch(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut error = false;
        let no = tv_get_number_chk(argvars, &raw mut error) as c_int;
        if error {
            return;
        }
        if no < 0 || (no as usize) >= NSUBEXP {
            semsg!(gettext(E_INVALID_SUBMATCH_NUMBER_NR.as_ptr()), no);
            return;
        }
        let mut ret_list: c_int = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            ret_list = tv_get_bool_chk(argvars.add(1), &raw mut error) as c_int;
            if error {
                return;
            }
        }
        if ret_list == 0 {
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval.v_string = reg_submatch(no);
        } else {
            (*rettv).v_type = VarType::VAR_LIST;
            (*rettv).vval.v_list = reg_submatch_list(no);
        }
    }
}

/// "substitute()" function
pub unsafe fn f_substitute(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let mut subbuf = [0 as c_char; NUMBUFLEN];
        let mut flagsbuf = [0 as c_char; NUMBUFLEN];

        let str_ = tv_get_string_chk(argvars);
        let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
        let mut sub: *const c_char = ptr::null();
        let flg = tv_get_string_buf_chk(argvars.add(3), flagsbuf.as_mut_ptr());

        let mut expr: *mut typval_T = ptr::null_mut();
        if tv_is_func(*argvars.add(2)) {
            expr = argvars.add(2);
        } else {
            sub = tv_get_string_buf_chk(argvars.add(2), subbuf.as_mut_ptr());
        }

        (*rettv).v_type = VarType::VAR_STRING;
        if str_.is_null() || pat.is_null() || (sub.is_null() && expr.is_null()) || flg.is_null() {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            (*rettv).vval.v_string = do_string_sub(
                str_.cast_mut(),
                strlen(str_),
                pat.cast_mut(),
                sub.cast_mut(),
                expr,
                flg.cast_mut(),
                ptr::null_mut(),
            );
        }
    }
}

/// "swapfilelist()" function
pub unsafe fn f_swapfilelist(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        recover_names(ptr::null_mut(), false, (*rettv).vval.v_list);
    }
}

/// "swapinfo(swap_filename)" function
pub unsafe fn f_swapinfo(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        swapfile_dict(tv_get_string(argvars), (*rettv).vval.v_dict.cast());
    }
}

/// "swapname(expr)" function
pub unsafe fn f_swapname(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null()
            || (*buf).b_ml.ml_mfp.is_null()
            || (*(*buf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fname.is_null()
        {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            (*rettv).vval.v_string = xstrdup((*(*buf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fname);
        }
    }
}

/// "synID(lnum, col, trans)" function
pub unsafe fn f_synID(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let lnum = tv_get_lnum(argvars);
        let col = (tv_get_number(argvars.add(1)) as colnr_T) - 1;

        let mut transerr = false;
        let trans = tv_get_bool_chk(argvars.add(2), &raw mut transerr) as c_int;

        let mut id: c_int = 0;
        if !transerr
            && lnum >= 1
            && lnum <= (*curbuf).b_ml.ml_line_count
            && col >= 0
            && col < ml_get_len(lnum)
        {
            id = syn_get_id(curwin, lnum, col, trans, ptr::null_mut(), 0);
        }
        (*rettv).vval.v_number = varnumber_T::from(id);
    }
}

/// "synIDattr(id, what [, mode])" function
pub unsafe fn f_synIDattr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let id = tv_get_number(argvars) as c_int;
        let what = tv_get_string(argvars.add(1));
        let mut modec: c_int = if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut modebuf = [0 as c_char; NUMBUFLEN];
            let mode = tv_get_string_buf(argvars.add(2), modebuf.as_mut_ptr());
            let mc = tolower_asc(c_int::from(*mode as u8));
            if mc != b'c' as c_int && mc != b'g' as c_int {
                0
            } else {
                mc
            }
        } else if ui_rgb_attached() {
            b'g' as c_int
        } else {
            b'c' as c_int
        };
        let _ = &mut modec;

        let mut p: *const c_char = ptr::null();
        let w = |i: usize| tolower_asc(c_int::from(*what.add(i) as u8)) as u8;
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
                    p = get_highlight_name_ext(ptr::null_mut(), id - 1, false);
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
                if strlen(what) >= 9 {
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

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = if p.is_null() {
            ptr::null_mut()
        } else {
            xstrdup(p)
        };
    }
}

/// "synIDtrans(id)" function
pub unsafe fn f_synIDtrans(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut id = tv_get_number(argvars) as c_int;
        if id > 0 {
            id = syn_get_final_id(id);
        } else {
            id = 0;
        }
        (*rettv).vval.v_number = varnumber_T::from(id);
    }
}

/// "synconcealed(lnum, col)" function
pub unsafe fn f_synconcealed(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut syntax_flags: c_int = 0;
        let mut matchid: c_int = 0;
        let mut str_ = [0 as c_char; NUMBUFLEN];

        tv_list_set_ret(rettv, ptr::null_mut());

        let lnum = tv_get_lnum(argvars);
        let col = (tv_get_number(argvars.add(1)) as colnr_T) - 1;

        if lnum >= 1
            && lnum <= (*curbuf).b_ml.ml_line_count
            && col >= 0
            && col <= ml_get_len(lnum)
            && (*curwin).w_onebuf_opt.wo_cole > 0
        {
            syn_get_id(curwin, lnum, col, 0, ptr::null_mut(), 0);
            syntax_flags = get_syntax_info(&raw mut matchid);

            if (syntax_flags & HL_CONCEAL) != 0 && (*curwin).w_onebuf_opt.wo_cole < 3 {
                let mut cchar = schar_from_char(syn_get_sub_char());
                if cchar == 0 && (*curwin).w_onebuf_opt.wo_cole == 1 {
                    cchar = if (*curwin).w_p_lcs_chars.conceal == 0 {
                        b' ' as schar_T
                    } else {
                        (*curwin).w_p_lcs_chars.conceal
                    };
                }
                if cchar != 0 {
                    schar_get(str_.as_mut_ptr(), cchar);
                }
            }
        }

        tv_list_alloc_ret(rettv, 3);
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from((syntax_flags & HL_CONCEAL) != 0),
        );
        tv_list_append_string((*rettv).vval.v_list, str_.as_ptr(), -1);
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(matchid));
    }
}

/// "synstack(lnum, col)" function
pub unsafe fn f_synstack(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_set_ret(rettv, ptr::null_mut());

        let lnum = tv_get_lnum(argvars);
        let col = (tv_get_number(argvars.add(1)) as colnr_T) - 1;

        if lnum >= 1 && lnum <= (*curbuf).b_ml.ml_line_count && col >= 0 && col <= ml_get_len(lnum)
        {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            syn_get_id(curwin, lnum, col, 0, ptr::null_mut(), 1);

            let mut i: c_int = 0;
            loop {
                let id = syn_get_stack_item(i);
                i += 1;
                if id < 0 {
                    break;
                }
                tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(id));
            }
        }
    }
}

/// "tabpagebuflist()" function
pub unsafe fn f_tabpagebuflist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            wp = firstwin;
        } else {
            let tp = find_tabpage(tv_get_number(argvars) as c_int);
            if !tp.is_null() {
                wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
            }
        }
        if !wp.is_null() {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            while !wp.is_null() {
                tv_list_append_number(
                    (*rettv).vval.v_list,
                    varnumber_T::from((*(*wp).w_buffer).b_fnum),
                );
                wp = (*wp).w_next;
            }
        }
    }
}

/// "tagfiles()" function
pub unsafe fn f_tagfiles(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let mut fname = [0 as c_char; MAXPATHL];

        let mut first = true;
        let mut tn = tagname_T::new();
        while get_tagfname(&raw mut tn, c_int::from(first), fname.as_mut_ptr()) == OK {
            tv_list_append_string((*rettv).vval.v_list, fname.as_ptr(), -1);
            first = false;
        }

        tagname_free(&raw mut tn);
    }
}

/// "taglist()" function
pub unsafe fn f_taglist(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let tag_pattern = tv_get_string(argvars);
        (*rettv).vval.v_number = 0;
        if *tag_pattern == NUL {
            return;
        }
        let mut fname: *const c_char = ptr::null();
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            fname = tv_get_string(argvars.add(1));
        }
        get_tags(
            tv_list_alloc_ret(rettv, kListLenUnknown),
            tag_pattern.cast_mut(),
            fname.cast_mut(),
        );
    }
}

/// "timer_info([timer])" function
pub unsafe fn f_timer_info(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        if tv_check_for_opt_number_arg(argvars, 0) == FAIL {
            return;
        }
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let timer = find_timer_by_nr(tv_get_number(argvars));
            if !timer.is_null() && (!(*timer).stopped || (*timer).refcount > 1) {
                add_timer_info(rettv, timer);
            }
        } else {
            add_timer_info_all(rettv);
        }
    }
}

/// "timer_pause(timer, paused)" function
pub unsafe fn f_timer_pause(argvars: *mut typval_T, _unused: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_NUMBER {
            emsg(gettext(e_number_exp.as_ptr()));
            return;
        }
        let paused = tv_get_bool(argvars.add(1)) != 0;
        let timer = find_timer_by_nr(tv_get_number(argvars));
        if !timer.is_null() {
            if !(*timer).paused && paused {
                time_watcher_stop(&raw mut (*timer).watcher);
            } else if (*timer).paused && !paused {
                time_watcher_start(
                    &raw mut (*timer).watcher,
                    Some(timer_due_cb),
                    (*timer).timeout as u64,
                    (*timer).timeout as u64,
                );
            }
            (*timer).paused = paused;
        }
    }
}

/// "timer_start(timeout, callback, opts)" function
pub unsafe fn f_timer_start(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut repeat: c_int = 1;
        (*rettv).vval.v_number = -1;
        if check_secure() {
            return;
        }

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_nonnull_dict_arg(argvars, 2) == FAIL {
                return;
            }
            let dict = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
            let di = tv_dict_find(dict, c"repeat".as_ptr(), 6);
            if !di.is_null() {
                repeat = tv_get_number(&raw const (*di).di_tv) as c_int;
                if repeat == 0 {
                    repeat = 1;
                }
            }
        }

        let mut callback = Callback::NONE;
        if !callback_from_typval(&raw mut callback, argvars.add(1)) {
            return;
        }
        (*rettv).vval.v_number =
            timer_start(tv_get_number(argvars), repeat, &raw const callback) as varnumber_T;
    }
}

/// "timer_stop(timerid)" function
pub unsafe fn f_timer_stop(argvars: *mut typval_T, _rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_number_arg(argvars, 0) == FAIL {
            return;
        }
        let timer = find_timer_by_nr(tv_get_number(argvars));
        if timer.is_null() {
            return;
        }
        timer_stop(timer);
    }
}

pub unsafe fn f_timer_stopall(
    _argvars: *mut typval_T,
    _unused: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        timer_stop_all();
    }
}

/// "type(expr)" function
pub unsafe fn f_type(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut n: varnumber_T = -1;
        match (*argvars).v_type {
            VarType::VAR_NUMBER => n = VAR_TYPE_NUMBER,
            VarType::VAR_STRING => n = VAR_TYPE_STRING,
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => n = VAR_TYPE_FUNC,
            VarType::VAR_LIST => n = VAR_TYPE_LIST,
            VarType::VAR_DICT => n = VAR_TYPE_DICT,
            VarType::VAR_FLOAT => n = VAR_TYPE_FLOAT,
            VarType::VAR_BOOL => n = VAR_TYPE_BOOL,
            VarType::VAR_SPECIAL => n = VAR_TYPE_SPECIAL,
            VarType::VAR_BLOB => n = VAR_TYPE_BLOB,
            VarType::VAR_UNKNOWN => internal_error(c"f_type(UNKNOWN)".as_ptr()),
        }
        (*rettv).vval.v_number = n;
    }
}

/// "virtcol({expr}, [, {list} [, {winid}]])" function
pub unsafe fn f_virtcol(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut vcol_start: colnr_T = 0;
        let mut vcol_end: colnr_T = 0;
        let mut wp = curwin;

        'theend: {
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            {
                let mut tp: *mut tabpage_T = ptr::null_mut();
                wp = win_id2wp_tp(tv_get_number(argvars.add(2)) as c_int, &raw mut tp);
                if wp.is_null() || tp.is_null() {
                    break 'theend;
                }
                check_cursor(wp);
            }

            let bp = (*wp).w_buffer;
            let mut fnum = (*bp).b_fnum;
            let fp = var2fpos(argvars, false, &raw mut fnum, false, wp);
            if !fp.is_null() && (*fp).lnum <= (*bp).b_ml.ml_line_count && fnum == (*bp).b_fnum {
                if (*fp).col < 0 {
                    (*fp).col = 0;
                } else {
                    let len = ml_get_buf_len(bp, (*fp).lnum);
                    if (*fp).col > len {
                        (*fp).col = len;
                    }
                }
                getvvcol(
                    wp,
                    (*fp).cast(),
                    &raw mut vcol_start,
                    ptr::null_mut(),
                    &raw mut vcol_end,
                    0,
                );
                vcol_start += 1;
                vcol_end += 1;
            }
        }

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars.add(1)) != 0 {
            tv_list_alloc_ret(rettv, 2);
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(vcol_start));
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(vcol_end));
        } else {
            (*rettv).vval.v_number = varnumber_T::from(vcol_end);
        }
    }
}

/// "visualmode()" function
pub unsafe fn f_visualmode(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let str_ = [(*curbuf).b_visual_mode_eval as c_char, NUL];
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xstrdup(str_.as_ptr());

        if non_zero_arg(argvars) {
            (*curbuf).b_visual_mode_eval = 0;
        }
    }
}

/// "wildmenumode()" function
pub unsafe fn f_wildmenumode(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if wild_menu_showing != 0 || ((State & MODE_CMDLINE) != 0 && cmdline_pum_active()) {
            (*rettv).vval.v_number = 1;
        }
    }
}

/// "windowsversion()" function
pub unsafe fn f_windowsversion(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xstrdup((&raw const windowsVersion).cast());
    }
}

/// "wordcount()" function
pub unsafe fn f_wordcount(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        cursor_pos_info((*rettv).vval.v_dict.cast());
    }
}

/// "xor(expr, expr)" function
pub unsafe fn f_xor(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = tv_get_number_chk(argvars, ptr::null_mut())
            ^ tv_get_number_chk(argvars.add(1), ptr::null_mut());
    }
}

use crate::api::buffer::VIML_INTERNAL_CALL;
use crate::api::ui::{MAX_SCHAR_SIZE, kChannelPartAll};
use crate::arglist::{f_argc, f_argidx, f_arglistid, f_argv};
use crate::change::MB_MAXCHAR;
use crate::charset::NUL;
#[cfg(windows)]
use crate::cmdexpand::MB_MAXBYTES;
use crate::cmdexpand::{f_cmdcomplete_info, f_getcompletion, f_getcompletiontype};
use crate::cmdhist::{f_histadd, f_histdel, f_histget, f_histnr};
use crate::diff::{f_diff_filler, f_diff_hlID};
use crate::digraph::{
    f_digraph_get, f_digraph_getlist, f_digraph_set, f_digraph_setlist, kBoolVarFalse, kBoolVarTrue,
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
    f_blob2list, f_has_key, f_items, f_join, f_keys, f_list2blob, f_list2str, f_sort, f_uniq,
    f_values, kCallbackNone,
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
        name: c"id".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_id),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"or".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_or),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"tr".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_tr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"abs".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_abs),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"add".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_add),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"and".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_and),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"col".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_col),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"cos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::cos),
    },
    EvalFuncDef {
        name: c"exp".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::exp),
    },
    EvalFuncDef {
        name: c"get".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_get),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"has".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_has),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"len".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_len),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"log".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::ln),
    },
    EvalFuncDef {
        name: c"map".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_map),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"max".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_max),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"min".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_min),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"pow".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_pow),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sin".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::sin),
    },
    EvalFuncDef {
        name: c"tan".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::tan),
    },
    EvalFuncDef {
        name: c"xor".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_xor),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"hlID".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hlID),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"glob".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_glob),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"argc".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_argc),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"fmod".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_fmod),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rand".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_rand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"line".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_line),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"mode".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"type".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_type),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"cosh".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::cosh),
    },
    EvalFuncDef {
        name: c"sinh".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::sinh),
    },
    EvalFuncDef {
        name: c"tanh".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::tanh),
    },
    EvalFuncDef {
        name: c"call".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_call),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"ceil".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::ceil),
    },
    EvalFuncDef {
        name: c"eval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"trim".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_trim),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"asin".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::asin),
    },
    EvalFuncDef {
        name: c"atan".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::atan),
    },
    EvalFuncDef {
        name: c"join".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_join),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"uniq".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_uniq),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"acos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::acos),
    },
    EvalFuncDef {
        name: c"keys".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_keys),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sort".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sort),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sqrt".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::sqrt),
    },
    EvalFuncDef {
        name: c"wait".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wait),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"argv".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_argv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"copy".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_copy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"match".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_match),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"range".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_range),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"iconv".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_iconv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"chdir".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_chdir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winnr".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"mkdir".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mkdir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"floor".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::floor),
    },
    EvalFuncDef {
        name: c"slice".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_slice),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"empty".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_empty),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"index".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_index),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"input".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_input),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"count".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_count),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"log10".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::log10),
    },
    EvalFuncDef {
        name: c"round".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::round),
    },
    EvalFuncDef {
        name: c"split".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_split),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"srand".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_srand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"trunc".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(float_op_wrapper),
        data: EvalFuncData::FloatOp(f64::trunc),
    },
    EvalFuncDef {
        name: c"isinf".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_isinf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"isnan".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_isnan),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"atan2".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_atan2),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"items".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_items),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"state".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_state),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufnr".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"synID".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_synID),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sha256".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sha256),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"append".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_append),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufadd".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufadd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"expand".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_expand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"extend".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_extend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcwd".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcwd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getpid".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_getpid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobpid".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobpid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"browse".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_browse),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"delete".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_delete),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"escape".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_escape),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reduce".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_reduce),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"remove".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_remove),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rename".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_rename),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"printf".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: MAX_FUNC_ARGS as u8,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_printf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getreg".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getreg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"maparg".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_maparg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setreg".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setreg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"string".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_string),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"search".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_search),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"pyeval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_py3eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"wincol".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wincol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"system".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_system),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strlen".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strlen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"cursor".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_cursor),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"filter".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_filter),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"histnr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_histnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"str2nr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_str2nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"exists".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_exists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getpos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setpos".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"values".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_values),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"indent".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_indent),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"insert".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_insert),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"invert".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_invert),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"mapset".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mapset),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"repeat".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_repeat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getenv".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getenv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setenv".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setenv),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"mapnew".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mapnew),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"argidx".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_argidx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"stridx".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_stridx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufload".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufload),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufname".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"byteidx".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_byteidx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"char2nr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_char2nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"charcol".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_charcol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"charidx".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_charidx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"cindent".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_cindent),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"confirm".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_confirm),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"environ".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(lua_wrapper),
        data: EvalFuncData::LuaFunc(c"f_environ"),
    },
    EvalFuncDef {
        name: c"execute".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_execute),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"exepath".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_exepath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"finddir".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_finddir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"flatten".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_flatten),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foreach".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foreach),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"funcref".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_funcref),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getchar".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getchar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getline".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"gettext".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettext),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"has_key".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_has_key),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"histadd".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_histadd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"histdel".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_histdel),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"histget".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_histget),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"indexof".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_indexof),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobsend".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chansend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobstop".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobstop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobwait".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobwait),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"libcall".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_libcall),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"luaeval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_luaeval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"maplist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_maplist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nr2char".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_nr2char),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"py3eval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_py3eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"pyxeval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_py3eval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"readdir".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_readdir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reltime".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_reltime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"resolve".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_resolve),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reverse".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_reverse),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rpcstop".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcstop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setline".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"stdpath".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_stdpath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strpart".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strpart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strridx".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strridx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"taglist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_taglist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"tolower".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_tolower),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"toupper".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_toupper),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"virtcol".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_virtcol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winline".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_winline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"utf16idx".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_utf16idx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"str2list".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_str2list),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"api_info".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_api_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"float2nr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_float2nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"globpath".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 5,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_globpath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winbufnr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winbufnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"function".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_function),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobclose".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chanclose),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"mapcheck".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_mapcheck),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchadd".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchadd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matcharg".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matcharg),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchend".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchstr".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchstr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strchars".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strchars),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"feedkeys".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_feedkeys),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"findfile".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_findfile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foldtext".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_foldtext),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"readblob".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_readblob),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"readfile".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_readfile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"filecopy".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_filecopy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getfperm".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getfperm),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getfsize".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getfsize),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getftime".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getftime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getftype".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_getftype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setfperm".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setfperm),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strftime".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strftime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"tagfiles".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_tagfiles),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"perleval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_perleval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"hasmapto".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hasmapto),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__id".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[1]),
    },
    EvalFuncDef {
        name: c"nvim_cmd".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[2]),
    },
    EvalFuncDef {
        name: c"nvim_put".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[3]),
    },
    EvalFuncDef {
        name: c"submatch".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_submatch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"termopen".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_termopen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"changenr".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_changenr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"chansend".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chansend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"islocked".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_islocked),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"undofile".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_undofile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"undotree".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_undotree),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"complete".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_complete),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"deepcopy".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_deepcopy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"simplify".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_simplify),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strptime".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strptime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"swapinfo".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_swapinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"swapname".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_swapname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"tempname".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_tempname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobstart".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobstart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rpcstart".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcstart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"synstack".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_synstack),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"hostname".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(lua_wrapper),
        data: EvalFuncData::LuaFunc(c"f_hostname"),
    },
    EvalFuncDef {
        name: c"keytrans".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_keytrans),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"list2str".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_list2str),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strtrans".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strtrans),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"menu_get".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_menu_get),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufwinid".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufwinid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufwinnr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufwinnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strwidth".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strwidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winwidth".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winwidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"hlexists".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hlexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rubyeval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_rubyeval),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"blob2list".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_blob2list),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"byte2line".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_byte2line),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"line2byte".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_line2byte),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"list2blob".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_list2blob),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"synIDattr".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_synIDattr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"diff_hlID".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_diff_hlID),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"menu_info".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_menu_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_echo".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[7]),
    },
    EvalFuncDef {
        name: c"nvim_eval".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[9]),
    },
    EvalFuncDef {
        name: c"nvim_exec".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[10]),
    },
    EvalFuncDef {
        name: c"sign_jump".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_jump),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"gettabvar".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettabvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"libcallnr".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_libcallnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"settabvar".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_settabvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"tabpagenr".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_tabpagenr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winlayout".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winlayout),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"chanclose".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_chanclose),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"charclass".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_charclass),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"searchpos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_searchpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"wordcount".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wordcount),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"soundfold".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_soundfold),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getregion".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getregion),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"jobresize".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_jobresize),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"screencol".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_screencol),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"screenpos".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"screenrow".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_screenrow),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winheight".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winheight),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"writefile".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_writefile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getqflist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getqflist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setqflist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setqflist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"str2float".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_str2float),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_getid".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_getid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchlist".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"arglistid".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_arglistid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"buflisted".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_buflisted),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getwinpos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getwinpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getwinvar".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setwinvar".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_setwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foldlevel".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldlevel),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"localtime".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_localtime),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcmdpos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setcmdpos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcmdpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"expandcmd".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_expandcmd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"extendnew".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_extendnew),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufloaded".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufloaded),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rpcnotify".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: MAX_FUNC_ARGS as u8,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcnotify),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"stdioopen".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_stdioopen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"interrupt".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_interrupt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"browsedir".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_browsedir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"inputlist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_inputlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"inputsave".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_inputsave),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getbufvar".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcurpos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcurpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setbufvar".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_setbufvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"bufexists".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"timer_info".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"timer_stop".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_stop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcharmod".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcharmod),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcharpos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcharstr".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcharstr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setcharpos".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strcharlen".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strcharlen),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"debugbreak".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_debugbreak),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"gettabinfo".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettabinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getloclist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getloclist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setloclist".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_setloclist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcmdline".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcmdtype".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdtype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setcmdline".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcmdline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_id2win".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_id2win),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"flattennew".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_flattennew),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_exec2".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[11]),
    },
    EvalFuncDef {
        name: c"pum_getpos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_pum_getpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getbufinfo".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getbufline".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchfuzzy".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchfuzzy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setbufline".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_setbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getreginfo".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getreginfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getregtype".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getregtype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"searchdecl".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_searchdecl),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"searchpair".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 7,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_searchpair),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_input".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[12]),
    },
    EvalFuncDef {
        name: c"substitute".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_substitute),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foldclosed".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldclosed),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"visualmode".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_visualmode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reltimestr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_reltimestr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"systemlist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_systemlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getwininfo".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getwininfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getwinposx".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getwinposx),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getwinposy".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getwinposy),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"lispindent".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_lispindent),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"screenattr".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenattr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"screenchar".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenchar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_gotoid".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_gotoid),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_paste".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[13]),
    },
    EvalFuncDef {
        name: c"sign_place".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_place),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"rpcrequest".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: MAX_FUNC_ARGS as u8,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_rpcrequest),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foreground".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_foreground),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"serverlist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_serverlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"serverstop".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_serverstop),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"pumvisible".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_pumvisible),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winrestcmd".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_winrestcmd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"executable".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_executable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getmatches".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getmatches),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setmatches".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setmatches),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strgetchar".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strgetchar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"synIDtrans".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_synIDtrans),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"shiftwidth".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_shiftwidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__stats".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[19]),
    },
    EvalFuncDef {
        name: c"timer_pause".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_pause),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"timer_start".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_timer_start),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchaddpos".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchaddpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strcharpart".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strcharpart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"haslocaldir".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_haslocaldir),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"msgpackdump".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_msgpackdump),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"byteidxcomp".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_byteidxcomp),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"inputdialog".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_inputdialog),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"json_decode".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_json_decode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchdelete".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchdelete),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sign_define".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_define),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"fnameescape".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_fnameescape),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"isdirectory".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_isdirectory),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"json_encode".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_json_encode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"shellescape".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_shellescape),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_gettype".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_gettype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"diff_filler".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_diff_filler),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"strutf16len".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_strutf16len),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"gettagstack".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettagstack),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_get_hl".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[14]),
    },
    EvalFuncDef {
        name: c"settagstack".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_settagstack),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"pathshorten".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_pathshorten),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"searchcount".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_searchcount),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"highlightID".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hlID),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_findbuf".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_findbuf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"fnamemodify".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_fnamemodify),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getjumplist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getjumplist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getfontname".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getfontname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_notify".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[18]),
    },
    EvalFuncDef {
        name: c"screenchars".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenchars),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"fullcommand".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_fullcommand),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sockconnect".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_sockconnect),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"virtcol2col".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_virtcol2col),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"digraph_get".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_get),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"digraph_set".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_set),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"buffer_name".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufname),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getmarklist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getmarklist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"glob2regpat".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_glob2regpat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"serverstart".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_serverstart),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"wildtrigger".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wildtrigger),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"inputsecret".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_inputsecret),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchstrpos".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchstrpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_set_hl".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[15]),
    },
    EvalFuncDef {
        name: c"preinserted".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_preinserted),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winrestview".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_winrestview),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_true".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_true),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getmousepos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getmousepos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"winsaveview".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_winsaveview),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_execute".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_win_execute),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__ns_get".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[29]),
    },
    EvalFuncDef {
        name: c"nvim__ns_set".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[30]),
    },
    EvalFuncDef {
        name: c"nvim__redraw".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[25]),
    },
    EvalFuncDef {
        name: c"nvim__unpack".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[28]),
    },
    EvalFuncDef {
        name: c"gettabwinvar".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_gettabwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchbufline".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"settabwinvar".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: 4,
        fast: false,
        func: viml_fn!(f_settabwinvar),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"spellbadword".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_spellbadword),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"msgpackparse".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_msgpackparse),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_command".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[27]),
    },
    EvalFuncDef {
        name: c"getcmdprompt".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdprompt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_del_var".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[26]),
    },
    EvalFuncDef {
        name: c"complete_add".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_complete_add),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"filereadable".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_filereadable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reg_recorded".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_reg_recorded),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"wildmenumode".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_wildmenumode),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getregionpos".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getregionpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_get_var".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[31]),
    },
    EvalFuncDef {
        name: c"tabpagewinnr".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_tabpagewinnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"eventhandler".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_eventhandler),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"did_filetype".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_did_filetype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"swapfilelist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_swapfilelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"clearmatches".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_clearmatches),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reltimefloat".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_reltimefloat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"screenstring".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_screenstring),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"synconcealed".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_synconcealed),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nextnonblank".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_nextnonblank),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"prevnonblank".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prevnonblank),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"filewritable".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_filewritable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"inputrestore".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_inputrestore),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchstrlist".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchstrlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_set_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[32]),
    },
    EvalFuncDef {
        name: c"spellsuggest".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_spellsuggest),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_beeps".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_beeps),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_equal".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_equal),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_fails".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 5,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_fails),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_false".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_false),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_match".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_match),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_ui_send".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[22]),
    },
    EvalFuncDef {
        name: c"sign_unplace".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_unplace),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__id_dict".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[37]),
    },
    EvalFuncDef {
        name: c"timer_stopall".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_timer_stopall),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getchangelist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getchangelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcharsearch".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcharsearch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getstacktrace".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getstacktrace),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"setcharsearch".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcharsearch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_chan_set".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[40]),
    },
    EvalFuncDef {
        name: c"win_screenpos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_screenpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"appendbufline".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_appendbufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcmdwintype".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdwintype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_del_mark".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[45]),
    },
    EvalFuncDef {
        name: c"win_id2tabwin".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_id2tabwin),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"complete_info".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_complete_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"deletebufline".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_deletebufline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"reg_recording".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_reg_recording),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getbufoneline".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getbufoneline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"matchfuzzypos".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_matchfuzzypos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_feedkeys".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[51]),
    },
    EvalFuncDef {
        name: c"nvim_get_mark".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[46]),
    },
    EvalFuncDef {
        name: c"nvim_get_mode".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[38]),
    },
    EvalFuncDef {
        name: c"nvim_get_proc".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[43]),
    },
    EvalFuncDef {
        name: c"nvim_get_vvar".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[34]),
    },
    EvalFuncDef {
        name: c"searchpairpos".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 7,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_searchpairpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foldclosedend".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldclosedend),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcellwidths".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcellwidths),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_list_uis".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[41]),
    },
    EvalFuncDef {
        name: c"setcellwidths".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcellwidths),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcompletion".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcompletion),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_open_win".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[42]),
    },
    EvalFuncDef {
        name: c"win_splitmove".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_splitmove),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"buffer_exists".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"buffer_number".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_bufnr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"file_readable".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_filereadable),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getscriptinfo".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getscriptinfo),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_set_vvar".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[35]),
    },
    EvalFuncDef {
        name: c"nvim_strwidth".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[47]),
    },
    EvalFuncDef {
        name: c"assert_nobeep".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_nobeep),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_report".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_report),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"sign_undefine".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_undefine),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_win_hide".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[39]),
    },
    EvalFuncDef {
        name: c"reg_executing".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_reg_executing),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__id_array".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[53]),
    },
    EvalFuncDef {
        name: c"nvim__id_float".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[54]),
    },
    EvalFuncDef {
        name: c"dictwatcheradd".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_dictwatcheradd),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"dictwatcherdel".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_dictwatcherdel),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"last_buffer_nr".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_last_buffer_nr),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcmdcomplpat".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdcomplpat),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"complete_check".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_complete_check),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"foldtextresult".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_foldtextresult),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_err_write".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[56]),
    },
    EvalFuncDef {
        name: c"garbagecollect".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_garbagecollect),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_get_hl_ns".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[59]),
    },
    EvalFuncDef {
        name: c"sign_getplaced".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_getplaced),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"tabpagebuflist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_tabpagebuflist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_list_bufs".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[60]),
    },
    EvalFuncDef {
        name: c"nvim_list_wins".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[61]),
    },
    EvalFuncDef {
        name: c"isabsolutepath".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_isabsolutepath),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_open_term".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[62]),
    },
    EvalFuncDef {
        name: c"nvim_out_write".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[63]),
    },
    EvalFuncDef {
        name: c"nvim_parse_cmd".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[64]),
    },
    EvalFuncDef {
        name: c"sign_placelist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_placelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_set_hl_ns".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[68]),
    },
    EvalFuncDef {
        name: c"assert_inrange".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: 3,
        fast: false,
        func: viml_fn!(f_assert_inrange),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_win_close".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[72]),
    },
    EvalFuncDef {
        name: c"windowsversion".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: true,
        func: viml_fn!(f_windowsversion),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__buf_stats".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[100]),
    },
    EvalFuncDef {
        name: c"nvim_buf_attach".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[83]),
    },
    EvalFuncDef {
        name: c"nvim_buf_delete".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[84]),
    },
    EvalFuncDef {
        name: c"nvim_create_buf".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[82]),
    },
    EvalFuncDef {
        name: c"getcmdcompltype".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdcompltype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcmdscreenpos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_getcmdscreenpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_del_keymap".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[91]),
    },
    EvalFuncDef {
        name: c"nvim_get_keymap".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[96]),
    },
    EvalFuncDef {
        name: c"nvim_get_option".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[97]),
    },
    EvalFuncDef {
        name: c"sign_getdefined".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_getdefined),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_list_chans".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[95]),
    },
    EvalFuncDef {
        name: c"digraph_getlist".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_getlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"digraph_setlist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_digraph_setlist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_set_keymap".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[98]),
    },
    EvalFuncDef {
        name: c"nvim_set_option".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[99]),
    },
    EvalFuncDef {
        name: c"strdisplaywidth".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: 1,
        fast: true,
        func: viml_fn!(f_strdisplaywidth),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_notequal".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_notequal),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_notmatch".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_assert_notmatch),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"prompt_getinput".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_getinput),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_win_resize".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[92]),
    },
    EvalFuncDef {
        name: c"highlight_exists".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_hlexists),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_del_autocmd".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[108]),
    },
    EvalFuncDef {
        name: c"sign_unplacelist".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_sign_unplacelist),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_exception".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_assert_exception),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"getcursorcharpos".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcursorcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_get_context".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[109]),
    },
    EvalFuncDef {
        name: c"setcursorcharpos".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_setcursorcharpos),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_buf_del_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[124]),
    },
    EvalFuncDef {
        name: c"nvim_win_del_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[111]),
    },
    EvalFuncDef {
        name: c"nvim__screenshot".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[121]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[125]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_buf".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[112]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[113]),
    },
    EvalFuncDef {
        name: c"prompt_appendbuf".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 2,
        fast: false,
        func: viml_fn!(f_prompt_appendbuf),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_buf_set_var".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[126]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_buf".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[114]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_var".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[115]),
    },
    EvalFuncDef {
        name: c"cmdcomplete_info".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_cmdcomplete_info),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_input_mouse".as_ptr().cast_mut(),
        min_argc: 6,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[116]),
    },
    EvalFuncDef {
        name: c"prompt_getprompt".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_getprompt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"prompt_setprompt".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_setprompt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"assert_equalfile".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_assert_equalfile),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__cmdwin_set".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[107]),
    },
    EvalFuncDef {
        name: c"nvim_err_writeln".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[119]),
    },
    EvalFuncDef {
        name: c"nvim__get_lib_dir".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[137]),
    },
    EvalFuncDef {
        name: c"nvim__get_runtime".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[146]),
    },
    EvalFuncDef {
        name: c"nvim_load_context".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[140]),
    },
    EvalFuncDef {
        name: c"nvim_open_tabpage".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[134]),
    },
    EvalFuncDef {
        name: c"nvim_get_autocmds".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[153]),
    },
    EvalFuncDef {
        name: c"nvim_get_commands".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[156]),
    },
    EvalFuncDef {
        name: c"nvim_buf_del_mark".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[129]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_mark".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[130]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_name".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[131]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_text".as_ptr().cast_mut(),
        min_argc: 5,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[138]),
    },
    EvalFuncDef {
        name: c"nvim_get_hl_by_id".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[128]),
    },
    EvalFuncDef {
        name: c"nvim_buf_is_valid".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[150]),
    },
    EvalFuncDef {
        name: c"nvim_win_is_valid".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[151]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_mark".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[132]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_name".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[133]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_text".as_ptr().cast_mut(),
        min_argc: 6,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[139]),
    },
    EvalFuncDef {
        name: c"getcompletiontype".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_getcompletiontype),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim__complete_set".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[172]),
    },
    EvalFuncDef {
        name: c"nvim__inspect_cell".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[173]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_lines".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[168]),
    },
    EvalFuncDef {
        name: c"nvim_buf_is_loaded".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[170]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_lines".as_ptr().cast_mut(),
        min_argc: 5,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[175]),
    },
    EvalFuncDef {
        name: c"nvim_call_function".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[160]),
    },
    EvalFuncDef {
        name: c"nvim_exec_autocmds".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[161]),
    },
    EvalFuncDef {
        name: c"nvim_get_chan_info".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[163]),
    },
    EvalFuncDef {
        name: c"nvim_get_color_map".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[164]),
    },
    EvalFuncDef {
        name: c"nvim_list_tabpages".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[162]),
    },
    EvalFuncDef {
        name: c"win_move_separator".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_move_separator),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"prompt_setcallback".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_setcallback),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_win_get_width".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[169]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_hl_ns".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[176]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_width".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[177]),
    },
    EvalFuncDef {
        name: c"nvim_get_hl_by_name".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[180]),
    },
    EvalFuncDef {
        name: c"nvim_set_hl_ns_fast".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[181]),
    },
    EvalFuncDef {
        name: c"nvim_buf_line_count".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[182]),
    },
    EvalFuncDef {
        name: c"nvim_buf_del_keymap".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[183]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_keymap".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[184]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_keymap".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[185]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_height".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[186]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_height".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[187]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_offset".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[188]),
    },
    EvalFuncDef {
        name: c"nvim_create_augroup".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[189]),
    },
    EvalFuncDef {
        name: c"nvim_clear_autocmds".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[192]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_config".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[193]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_config".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[194]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_option".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[195]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_option".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[196]),
    },
    EvalFuncDef {
        name: c"nvim_get_namespaces".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[197]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_option".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[198]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_option".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[199]),
    },
    EvalFuncDef {
        name: c"prompt_setinterrupt".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_prompt_setinterrupt),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"win_move_statusline".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: 1,
        fast: false,
        func: viml_fn!(f_win_move_statusline),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_create_autocmd".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[200]),
    },
    EvalFuncDef {
        name: c"test_write_list_log".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_test_write_list_log),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_buf_get_number".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[201]),
    },
    EvalFuncDef {
        name: c"nvim_command_output".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[202]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_cursor".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[203]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_number".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[204]),
    },
    EvalFuncDef {
        name: c"nvim_win_set_cursor".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[205]),
    },
    EvalFuncDef {
        name: c"nvim_buf_del_extmark".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[206]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_extmark".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[207]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_tabpage".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[208]),
    },
    EvalFuncDef {
        name: c"nvim_get_current_buf".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[209]),
    },
    EvalFuncDef {
        name: c"nvim_set_current_buf".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[210]),
    },
    EvalFuncDef {
        name: c"nvim_set_current_dir".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[211]),
    },
    EvalFuncDef {
        name: c"nvim_win_text_height".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[213]),
    },
    EvalFuncDef {
        name: c"nvim_eval_statusline".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[214]),
    },
    EvalFuncDef {
        name: c"nvim_get_option_info".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[218]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_del_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[221]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_get_var".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[222]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_set_var".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[223]),
    },
    EvalFuncDef {
        name: c"nvim_get_current_win".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[224]),
    },
    EvalFuncDef {
        name: c"nvim_set_current_win".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[225]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_get_win".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[226]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_set_win".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[227]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_is_valid".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[244]),
    },
    EvalFuncDef {
        name: c"nvim_del_current_line".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[232]),
    },
    EvalFuncDef {
        name: c"nvim_get_current_line".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[235]),
    },
    EvalFuncDef {
        name: c"nvim_set_current_line".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[242]),
    },
    EvalFuncDef {
        name: c"nvim_parse_expression".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[239]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_commands".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[229]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_extmarks".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[230]),
    },
    EvalFuncDef {
        name: c"nvim_win_get_position".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[245]),
    },
    EvalFuncDef {
        name: c"nvim_get_option_info2".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[236]),
    },
    EvalFuncDef {
        name: c"nvim_get_option_value".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[237]),
    },
    EvalFuncDef {
        name: c"nvim_set_option_value".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[243]),
    },
    EvalFuncDef {
        name: c"nvim_get_runtime_file".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[238]),
    },
    EvalFuncDef {
        name: c"nvim__runtime_inspect".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[228]),
    },
    EvalFuncDef {
        name: c"nvim_create_namespace".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[231]),
    },
    EvalFuncDef {
        name: c"nvim_del_user_command".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[233]),
    },
    EvalFuncDef {
        name: c"nvim_replace_termcodes".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[255]),
    },
    EvalFuncDef {
        name: c"nvim_buf_add_highlight".as_ptr().cast_mut(),
        min_argc: 6,
        max_argc: 6,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[252]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_list_wins".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[256]),
    },
    EvalFuncDef {
        name: c"nvim_get_hl_id_by_name".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[260]),
    },
    EvalFuncDef {
        name: c"nvim_get_color_by_name".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[259]),
    },
    EvalFuncDef {
        name: c"nvim_get_proc_children".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[254]),
    },
    EvalFuncDef {
        name: c"nvim_del_augroup_by_id".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[246]),
    },
    EvalFuncDef {
        name: c"nvim_call_dict_function".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[261]),
    },
    EvalFuncDef {
        name: c"test_garbagecollect_now".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(f_test_garbagecollect_now),
        data: EvalFuncData::None,
    },
    EvalFuncDef {
        name: c"nvim_list_runtime_paths".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[264]),
    },
    EvalFuncDef {
        name: c"nvim_tabpage_get_number".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[265]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_changedtick".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[267]),
    },
    EvalFuncDef {
        name: c"nvim_get_current_tabpage".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[268]),
    },
    EvalFuncDef {
        name: c"nvim_set_current_tabpage".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[269]),
    },
    EvalFuncDef {
        name: c"nvim_del_augroup_by_name".as_ptr().cast_mut(),
        min_argc: 1,
        max_argc: 1,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[270]),
    },
    EvalFuncDef {
        name: c"nvim_buf_clear_highlight".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[271]),
    },
    EvalFuncDef {
        name: c"nvim_buf_clear_namespace".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[272]),
    },
    EvalFuncDef {
        name: c"nvim_create_user_command".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[273]),
    },
    EvalFuncDef {
        name: c"nvim__buf_debug_extmarks".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 3,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[274]),
    },
    EvalFuncDef {
        name: c"nvim_get_all_options_info".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[275]),
    },
    EvalFuncDef {
        name: c"nvim_buf_del_user_command".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[276]),
    },
    EvalFuncDef {
        name: c"nvim_buf_set_virtual_text".as_ptr().cast_mut(),
        min_argc: 4,
        max_argc: 5,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[277]),
    },
    EvalFuncDef {
        name: c"nvim__set_restart_on_crash".as_ptr().cast_mut(),
        min_argc: 2,
        max_argc: 2,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[278]),
    },
    EvalFuncDef {
        name: c"nvim_buf_get_extmark_by_id".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[279]),
    },
    EvalFuncDef {
        name: c"nvim_select_popupmenu_item".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[280]),
    },
    EvalFuncDef {
        name: c"nvim__invalidate_glyph_cache".as_ptr().cast_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[281]),
    },
    EvalFuncDef {
        name: c"nvim_buf_create_user_command".as_ptr().cast_mut(),
        min_argc: 3,
        max_argc: 4,
        base_arg: BASE_NONE,
        fast: false,
        func: viml_fn!(api_wrapper),
        data: EvalFuncData::ApiHandler(&crate::api::private::dispatch::method_handlers[282]),
    },
    EvalFuncDef {
        name: ptr::null_mut(),
        min_argc: 0,
        max_argc: 0,
        base_arg: BASE_NONE,
        fast: false,
        func: None,
        data: EvalFuncData::None,
    },
];

pub unsafe fn eval_func_def_at(idx: c_int) -> *const EvalFuncDef {
    &raw const FUNCTIONS[idx as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix32_and_xoshiro_are_deterministic() {
        unsafe {
            let mut seed: u32 = 42;
            let mut x = splitmix32(&raw mut seed);
            let mut y = splitmix32(&raw mut seed);
            let mut z = splitmix32(&raw mut seed);
            let mut w = splitmix32(&raw mut seed);
            let r1 = shuffle_xoshiro128starstar(&raw mut x, &raw mut y, &raw mut z, &raw mut w);
            assert_ne!(r1, 0);
        }
    }

    #[test]
    fn non_zero_arg_checks_number_bool_and_string() {
        unsafe {
            let mut tv = tv_initial_value();
            tv.v_type = VarType::VAR_NUMBER;
            tv.vval.v_number = 0;
            assert!(!non_zero_arg(&raw const tv));
            tv.vval.v_number = 5;
            assert!(non_zero_arg(&raw const tv));

            tv.v_type = VarType::VAR_BOOL;
            tv.vval.v_bool = kBoolVarFalse;
            assert!(!non_zero_arg(&raw const tv));
            tv.vval.v_bool = kBoolVarTrue;
            assert!(non_zero_arg(&raw const tv));
        }
    }

    #[test]
    fn find_internal_func_hash_resolves_builtin_functions() {
        unsafe {
            for (idx, fdef) in FUNCTIONS[..FUNCTIONS.len() - 1].iter().enumerate() {
                let name = CStr::from_ptr(fdef.name).to_bytes();
                assert_eq!(find_internal_func_hash(fdef.name, name.len()), idx as c_int);
                assert_eq!(
                    find_internal_func(fdef.name),
                    eval_func_def_at(idx as c_int)
                );
            }
            assert!(
                (*eval_func_def_at((FUNCTIONS.len() - 1) as c_int))
                    .name
                    .is_null()
            );
            assert_eq!(
                find_internal_func_hash(c"not_a_builtin_fn".as_ptr(), 16),
                -1
            );
            assert!(find_internal_func(c"not_a_builtin_fn".as_ptr()).is_null());
            assert!(eval_has_static_feature(c"nvim".as_ptr()));
            assert!(eval_has_static_feature(c"AUTOCMD".as_ptr()));
            assert!(!eval_has_static_feature(c"not_a_feature".as_ptr()));
        }
    }
}
