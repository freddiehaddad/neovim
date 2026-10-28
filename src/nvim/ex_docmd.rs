//! Functions for executing an Ex command line (`src/nvim/ex_docmd.c`).

use crate::api::command::is_user_cmdidx;
#[cfg(windows)]
use crate::api::private::helpers::api_free_object;
use crate::api::private::helpers::{
    HLF_E, api_clear_error, boolean_obj, cstr_as_obj, cstr_as_string, error_init, error_set,
    integer_obj,
};
use crate::api::private::validate::{Array, Dict, KeyValuePair, Object, ObjectData, ObjectType};
use crate::api::ui::{remote_ui_connect, remote_ui_restart, ui_detach_channel, ui_detach_others};
use crate::api::vimscript::nvim_command;
use crate::arglist::{
    CCGD_AW, CCGD_EXCMD, CCGD_FORCEIT, CCGD_MULTWIN, ML_EMPTY, arg_all, argcount, check_arg_idx,
    ex_all, ex_argadd, ex_argdedupe, ex_argdelete, ex_argedit, ex_args, ex_argument, ex_last,
    ex_next, ex_previous, ex_rewind,
};
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_CHANINFO, EVENT_CMDUNDEFINED, EVENT_EXITPRE,
    EVENT_QUITPRE, EVENT_TABNEWENTERED, HLF_T, PROF_YES, apply_autocmds, apply_autocmds_win,
    check_nomodeline, do_augroup, do_autocmd, do_doautocmd, ex_doautoall, has_event,
    kOptEventignore, may_trigger_vim_suspend_resume,
};
use crate::buffer::{
    BF_DUMMY, DOBUF_CURRENT, DOBUF_DEL, DOBUF_FIRST, DOBUF_LAST, DOBUF_MOD, DOBUF_UNLOAD,
    DOBUF_WIPE, MAXLNUM, bt_prompt, bt_quickfix, buf_hide, buf_spname, buflist_findnr,
    buflist_findpat, buflist_list, bufref_T, bufref_valid, do_bufdel, do_modelines, ex_buffer_all,
    get_highest_fnum, goto_buffer, maketitle, no_write_message, otherfile, set_bufref, setaltfname,
    setfname,
};
use crate::channel::{CallbackReader, channel_event, channel_job_start, kChannelStdinPipe};
use crate::charset::{
    Ctrl_V, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_islower, ascii_isspace,
    ascii_isupper, ascii_iswhite, backslash_halve, getdigits, getdigits_int, getdigits_int32,
    skipdigits, skiptowhite_esc, skipwhite, tolower_asc,
};
use crate::clipboard::{end_batch_changes, start_batch_changes};
use crate::cmdexpand::{
    CompleteListItemGetter, EX_NOTRLCOM, EXPAND_FILES, EXPAND_NOTHING, ExpandGeneric, ExpandOne,
    LineGetter, WILD_ADD_SLASH, WILD_EXPAND_FREE, WILD_ICASE, WILD_LIST_NOTFOUND, WILD_NOERROR,
    cmd_addr_T, cmdidx_T, kUICmdline,
};
use crate::cmdhist::ex_history;
use crate::context::{CdCause, ctx_did_chdir, is_ctx_win};
use crate::cursor::{check_cursor, check_cursor_col, linenr_T, pos_T};
use crate::debugger::{
    dbg_breakpoint, dbg_check_breakpoint, dbg_find_breakpoint, do_debug, ex_breakadd, ex_breakdel,
    ex_breaklist, ex_debug, ex_debuggreedy,
};
use crate::dialog::{VIM_YES, ask_yesno, vim_dialog_yesno};
use crate::diff::{
    ex_diffgetput, ex_diffoff, ex_diffpatch, ex_diffsplit, ex_diffthis, ex_diffupdate,
};
use crate::digraph::{ex_loadkeymap, kBoolVarFalse, kBoolVarTrue, listdigraphs, putdigraph};
use crate::drawscreen::{
    UPD_CLEAR, UPD_INVERTED, UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, clearmode, redraw_all_later,
    redraw_curbuf_later, redraw_later, redraw_statuslines, screen_resize, setcursor_mayforce,
    showmode, status_redraw_all, status_redraw_curbuf, update_screen,
};
use crate::eval::fs::{
    VALID_HEAD, VALID_PATH, kCdScopeBuffer, kCdScopeTabpage, kCdScopeWindow, modify_fname,
};
use crate::eval::funcs::{
    DOCMD_KEYTYPED, RE_SEARCH, SEARCH_KEEP, VIM_QUESTION, create_environment, kMTLineWise,
};
use crate::eval::typval::{
    BoolVarValue, Callback, EvalFuncData, TypvalVval, VarType, callback_free, list_T, tv_clear,
    tv_dict_add_str, tv_dict_alloc, tv_get_string, tv_list_alloc_ret,
    tv_list_append_string, tv_list_copy, tv_list_find, tv_list_find_str, tv_list_free,
    typval_T,
};
use crate::eval::userfunc::{
    MSG_BUF_LEN, current_func_returned, do_return, ex_call, ex_delfunction, ex_function, ex_return,
    func_breakpoint, func_dbg_tick, func_has_abort, func_has_ended, func_level, func_name,
};
use crate::eval::vars::{
    BAD_DROP, BAD_KEEP, ENV_STARTREASON, FORCE_BIN, FORCE_NOBIN, VV_ARGV, VV_EXITREASON,
    VV_HLSEARCH, VV_OLDFILES, VV_PROGPATH, ex_let, ex_lockvar, ex_unlet, get_vim_var_list,
    get_vim_var_str, set_vim_var_nr, set_vim_var_string, v_exception, v_throwpoint,
    var_redir_start, var_redir_stop,
};
use crate::eval::window::{win_find_nr, win_has_winnr};
use crate::eval::{
    callback_call, eval_to_string, ex_echo, ex_echohl, ex_execute, get_copyID, kMarkAll,
    set_ref_in_callback, skip_expr,
};
use crate::event::r#loop::{loop_poll_events, loop_queues, uv_strerror};
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
use crate::event::proc::{proc_stop, proc_wait};
use crate::ex_cmds::{
    BL_FIX, BL_SOL, BL_WHITE, DIALOG_MSG_SIZE, ECMD_ADDBUF, ECMD_ALTBUF, ECMD_FORCEIT, ECMD_HIDE,
    ECMD_LAST, ECMD_OLDBUF, ECMD_ONE, EXFLAG_LIST, EXFLAG_NR, EXFLAG_PRINT, RE_SUBST, SEARCH_HIS,
    do_ascii, do_bang, do_ecmd, do_move, do_wqall, do_write, ex_align, ex_append, ex_change,
    ex_copy, ex_file, ex_global, ex_oldfiles, ex_sort, ex_substitute, ex_substitute_preview,
    ex_uniq, ex_update, ex_wnext, ex_write, ex_z, global_exe, kWinPreview, prepare_tagpreview,
    print_line, skip_vimgrep_pat,
};
use crate::ex_cmds2::{
    autowrite_all, check_changed, check_changed_any, check_fname, dialog_changed, ex_checktime,
    ex_compiler, ex_drop, ex_listdo, ex_perl, ex_perldo, ex_perlfile, ex_py3file, ex_pydo3,
    ex_python3, ex_ruby, ex_rubydo, ex_rubyfile,
};
use crate::ex_eval::{
    CSF_ACTIVE, CSF_CAUGHT, CSF_FINALLY, CSF_FOR, CSF_THROWN, CSF_TRUE, CSF_TRY, CSF_WHILE,
    CSL_HAD_CONT, CSL_HAD_ENDLOOP, CSL_HAD_FINA, CSL_HAD_LOOP, CSTP_ERROR, CSTP_INTERRUPT,
    CSTP_THROW, ET_ERROR, ET_INTERRUPT, ET_USER, PendingVal, aborting, cleanup_T,
    cleanup_conditionals, cstack_T, discard_current_exception, do_errthrow, do_intthrow, do_throw,
    enter_cleanup, ex_break, ex_catch, ex_continue, ex_else, ex_endfunction, ex_endif, ex_endtry,
    ex_endwhile, ex_eval, ex_finally, ex_if, ex_throw, ex_try, ex_while, except_T, has_loop_cmd,
    leave_cleanup, msglist_T, report_make_pending, rewind_conditionals,
};
use crate::ex_getln::{
    allbuf_locked, cmdpreview_get_bufnr, cmdpreview_get_ns, curbuf_locked, get_text_locked_msg,
    getcmdline, script_get, text_locked, text_locked_msg, text_or_buf_locked,
    ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave,
};
use crate::ex_session::{ex_loadview, ex_mkrc};
use crate::file_search::{
    FNAME_HYP, FNAME_MESS, do_autocmd_dirchanged, ff_search_ctx_T, file_name_at_cursor,
    find_file_in_path, vim_chdir,
};
use crate::fileio::{FILE, readfile, shorten_fnames};
use crate::fold::{foldCreate, foldManualAllowed, hasFolding, opFoldRange};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::help::{ex_exusage, ex_help, ex_helpclose, ex_helptags, ex_viusage};
use crate::highlight_group::{do_highlight, load_colors};
use crate::indent::ex_retab;
use crate::input::{
    beep_flush, flush_buffers, ins_typebuf, restore_typeahead, save_typeahead, stuff_empty,
    typebuf_typed, vpeekc,
};
use crate::insert::beginline;
use crate::log::{LOGLVL_INF, log_should_log, logmsg};
use crate::lua::executor::{ex_lua, ex_luado, ex_luafile, nlua_call_excmd, nlua_call_typval};
use crate::lua::secure::ex_trust;
use crate::main_init::getout;
use crate::mapping::{ex_abbreviate, ex_abclear, ex_map, ex_mapclear, ex_unmap};
use crate::mark::{
    checkpcmark, ex_changes, ex_clearjumps, ex_delmarks, ex_jumps, ex_marks, mark_check, mark_get,
    mark_get_visual, mark_move_to, setmark, setpcmark,
};
use crate::r#match::ex_match;
use crate::math::{FAIL, OK};
use crate::mbyte::{get_encoding_name, mb_copy_char, utf_head_off, utfc_ptr2len};
use crate::memline::{goto_byte, ml_clearmarked, ml_preserve, ml_recover, ml_setmarked};
use crate::memory::{
    ArenaMem, arena_mem_free, cstr_from_bytes, cstr_to_cstring, xstrlcpy,
};
use crate::menu::{ex_emenu, ex_menu, ex_menutranslate, save_state_T};
use crate::message::{
    emsg, emsg_multiline, ex_messages, iemsg, msg, msg_ext_set_kind, msg_make, msg_outtrans,
    msg_putchar, msg_puts, msg_start, redirecting, semsg, smsg, verbose_enter_scroll,
    verbose_leave_scroll, wait_return,
};
use crate::mouse::setmouse;
use crate::r#move::{
    check_cursor_moved, cursor_correct, cursor_valid, scrolldown, scrollup, update_curswant,
    update_topline, validate_cursor,
};
use crate::msgpack_rpc::channel::{find_channel, rpc_send_call};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::server::server_start;
use crate::normal::{
    FIND_EVAL, FIND_IDENT, FIND_STRING, do_check_scrollbind, end_visual_mode,
    find_ident_under_cursor, get_vtopline, normal_cmd, set_cursor_for_append_to_line,
};
use crate::ops::{OP_DELETE, OP_LSHIFT, OP_RSHIFT, OP_YANK, do_join, op_delete, op_shift};
use crate::option::{
    OPT_LOCAL, OptInt, ex_set, get_findfunc, get_option_sctx, get_scrolloff_value, kOptFiletype,
    set_option_direct, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::optionstr::{check_ff_value, get_fileformat_name};
use crate::os::env::{
    expand_T, expand_env, expand_env_esc, expand_env_save, home_replace, os_getenv_cstr,
};
use crate::os::fs::{os_chdir, os_dirname, os_fopen, os_mkdir, os_path_exists};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::lang::ex_language;
use crate::os::shell::shell_build_argv;
use crate::os::time::os_hrtime;
use crate::path::{
    FullName_save, kPathCmpLiteral, path_equal, path_has_wildcard, path_tail,
    path_try_shorten_fname, to_slash, to_slash_save,
};
use crate::plines::plines_m_win_fill;
use crate::popupmenu::pum_make_popup;
use crate::profile::{
    ex_profile, func_line_end, func_line_exec, func_line_start, script_line_end, script_line_exec,
    script_line_start,
};
use crate::quickfix::{
    ex_cbelow, ex_cbottom, ex_cbuffer, ex_cc, ex_cclose, ex_cexpr, ex_cfile, ex_cnext, ex_copen,
    ex_cwindow, ex_helpgrep, ex_make, ex_vimgrep, grep_internal, qf_age, qf_get_cur_idx,
    qf_get_cur_valid_idx, qf_get_size, qf_get_valid_size, qf_history, qf_list,
};
use crate::regexp::{
    MAXCOL, NUMBUFLEN, RE_MAGIC, kMarkBufLocal, regmatch_T, skip_regexp, vim_regcomp, vim_regfree,
};
use crate::register::{
    PUT_CURSLINE, PUT_FIXINDENT, PUT_LINE, do_execreg, do_put, ex_display, op_yank, set_expr_line,
    valid_yank_reg, write_reg_contents,
};
use crate::runtime::{
    DIP_ALL, ESTACK_SCRIPT, ESTACK_SFILE, ESTACK_STACK, ETYPE_EXCEPT, SID_NONE, do_finish,
    estack_pop, estack_push, estack_sfile, ex_finish, ex_options, ex_packadd, ex_packloadall,
    ex_runtime, ex_scriptencoding, ex_scriptnames, ex_source, rt_st, source_breakpoint,
    source_dbg_tick, source_finished, source_level, source_runtime, sourcing_name,
};
use crate::search::{
    do_search, find_pattern_in_path, restore_last_search_pattern, save_last_search_pattern,
    searchit,
};
use crate::shada::{shada_read_everything, shada_write_file};
use crate::sign::ex_sign;
use crate::spell::{ex_spelldump, ex_spellinfo, ex_spellrepall};
use crate::spellfile::{ex_mkspell, ex_spell};
use crate::state::{MODE_CMDLINE, MODE_INSERT, MODE_TERMINAL};
use crate::statusline::draw_tabline;
use crate::strings::{
    del_trailing_spaces, gettext, ngettext, strrep, vim_snprintf, vim_strchr, vim_strsave_escaped,
};
use crate::syntax::{ex_ownsyntax, ex_syntax, ex_syntime};
use crate::tag::{do_tag, do_tags};
use crate::textformat::oparg_T;
use crate::ui::{
    ui_active, ui_busy_start, ui_busy_stop, ui_call_error_exit, ui_call_suspend, ui_cursor_shape,
    ui_flush, ui_has,
};
use crate::undo::{
    bufIsChanged, curbufIsChanged, ex_undojoin, ex_undolist, u_clearline, u_compute_hash,
    u_read_undo, u_redo, u_save, u_undo, u_undo_and_forget, u_write_undo, uhp_ref, undo_time,
};
use crate::usercmd::{
    do_ucmd, ex_comclear, ex_command, ex_delcommand, expand_user_command_name, find_ucmd,
    get_user_command_name,
};
use crate::version::{ex_intro, ex_version};
use crate::window::{
    WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT, check_can_set_curbuf_forceit,
    close_others, do_window, find_tabpage, goto_tabpage, only_one_window, tabpage_T, tabpage_index,
    tabpage_move, trigger_tabclosedpre, update_cwd, valid_tabpage, win_T, win_close,
    win_close_othertab, win_enter, win_goto, win_new_tabpage, win_setheight_win, win_setwidth_win,
    win_split, win_valid, window_layout_locked,
};
use crate::winfloat::{win_float_close, win_float_remove};
use std::ffi::{CStr, c_char, c_int, c_long};
use std::ptr;

#[cfg(windows)]
use crate::lua::executor::{kRetObject, nlua_exec};
#[cfg(not(windows))]
use crate::msgpack_rpc::server::server_stop;
#[cfg(unix)]
use crate::os::fs::os_isdir;
#[cfg(windows)]
use crate::path::slash_adjust;

use crate::eval::typval::varnumber_T;
use crate::extmark::TriState;
pub type CdScope = c_int;
pub type ex_func_T = fn(&mut exarg_T);
pub type ex_preview_func_T = Option<fn(&mut exarg_T, c_int, handle_T) -> c_int>;

pub(crate) const CMD_SIZE: c_int = 564;
pub(crate) const CMD_USER: c_int = -1;
pub(crate) const CMD_USER_BUF: c_int = -2;

pub(crate) const EX_RANGE: u32 = 0x001;
pub(crate) const EX_BANG: u32 = 0x002;
pub(crate) const EX_EXTRA: u32 = 0x004;
pub(crate) const EX_XFILE: u32 = 0x008;
pub(crate) const EX_NOSPC: u32 = 0x010;
pub(crate) const EX_DFLALL: u32 = 0x020;
pub(crate) const EX_NEEDARG: u32 = 0x080;
pub(crate) const EX_TRLBAR: u32 = 0x100;
pub(crate) const EX_REGSTR: u32 = 0x200;
pub(crate) const EX_COUNT: u32 = 0x400;
pub(crate) const EX_ZEROR: u32 = 0x1000;
pub(crate) const EX_CMDARG: u32 = 0x4000;
pub(crate) const EX_ARGOPT: u32 = 0x20000;
pub(crate) const EX_SBOXOK: u32 = 0x40000;
pub(crate) const EX_KEEPSCRIPT: u32 = 0x4000000;
pub(crate) const EX_PREVIEW: u32 = 0x8000000;
pub(crate) const EX_ARGSPACE: u32 = 0x40000000;

pub(crate) const ADDR_LINES: cmd_addr_T = 0;
pub(crate) const ADDR_WINDOWS: cmd_addr_T = 1;
pub(crate) const ADDR_ARGUMENTS: cmd_addr_T = 2;
pub(crate) const ADDR_LOADED_BUFFERS: cmd_addr_T = 3;
pub(crate) const ADDR_BUFFERS: cmd_addr_T = 4;
pub(crate) const ADDR_TABS: cmd_addr_T = 5;
pub(crate) const ADDR_QUICKFIX: cmd_addr_T = 8;
pub(crate) const ADDR_OTHER: cmd_addr_T = 10;
pub(crate) const ADDR_NONE: cmd_addr_T = 11;

pub(crate) const CMOD_SANDBOX: c_int = 0x0001;
pub(crate) const CMOD_SILENT: c_int = 0x0002;
pub(crate) const CMOD_ERRSILENT: c_int = 0x0004;
pub(crate) const CMOD_UNSILENT: c_int = 0x0008;
pub(crate) const CMOD_NOAUTOCMD: c_int = 0x0010;
pub(crate) const CMOD_HIDE: c_int = 0x0020;
pub(crate) const CMOD_BROWSE: c_int = 0x0040;
pub(crate) const CMOD_CONFIRM: c_int = 0x0080;
pub(crate) const CMOD_KEEPALT: c_int = 0x0100;
pub(crate) const CMOD_KEEPMARKS: c_int = 0x0200;
pub(crate) const CMOD_KEEPJUMPS: c_int = 0x0400;
pub(crate) const CMOD_LOCKMARKS: c_int = 0x0800;
pub(crate) const CMOD_KEEPPATTERNS: c_int = 0x1000;
pub(crate) const CMOD_NOSWAPFILE: c_int = 0x2000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ExMagic {
    pub file: bool,
    pub bar: bool,
}

#[repr(C)]
#[derive(Clone)]
pub struct exarg_T {
    pub arg: *mut c_char,
    pub args: Vec<*mut c_char>,
    pub arglens: Vec<usize>,
    pub nextcmd: *mut c_char,
    pub cmd: *mut c_char,
    pub cmdlinep: *mut *mut c_char,
    pub cmdline_buf: *mut Vec<c_char>,
    pub cmdidx: c_int,
    pub argt: u32,
    pub skip: c_int,
    pub forceit: c_int,
    pub addr_count: c_int,
    pub line1: linenr_T,
    pub line2: linenr_T,
    pub addr_type: cmd_addr_T,
    pub flags: c_int,
    pub do_ecmd_cmd: *mut c_char,
    pub do_ecmd_lnum: linenr_T,
    pub append: c_int,
    pub usefilter: c_int,
    pub amount: c_int,
    pub regname: c_int,
    pub force_bin: c_int,
    pub read_edit: c_int,
    pub mkdir_p: c_int,
    pub force_ff: c_int,
    pub force_enc: c_int,
    pub bad_char: c_int,
    pub useridx: c_int,
    pub errmsg: *mut c_char,
    pub ea_getline: LineGetter,
    pub cstack: *mut crate::ex_eval::cstack_T,
    pub magic: ExMagic,
}

impl exarg_T {
    pub const fn new() -> Self {
        Self {
            arg: ptr::null_mut(),
            args: Vec::new(),
            arglens: Vec::new(),
            nextcmd: ptr::null_mut(),
            cmd: ptr::null_mut(),
            cmdlinep: ptr::null_mut(),
            cmdline_buf: ptr::null_mut(),
            cmdidx: 0,
            argt: 0,
            skip: 0,
            forceit: 0,
            addr_count: 0,
            line1: 0,
            line2: 0,
            addr_type: 0,
            flags: 0,
            do_ecmd_cmd: ptr::null_mut(),
            do_ecmd_lnum: 0,
            append: 0,
            usefilter: 0,
            amount: 0,
            regname: 0,
            force_bin: 0,
            read_edit: 0,
            mkdir_p: 0,
            force_ff: 0,
            force_enc: 0,
            bad_char: 0,
            useridx: 0,
            errmsg: ptr::null_mut(),
            ea_getline: LineGetter::None,
            cstack: ptr::null_mut(),
            magic: ExMagic {
                file: false,
                bar: false,
            },
        }
    }

    pub fn set_cmdline_vec(&mut self, cmdlinep: *mut *mut c_char, new_buf: Vec<c_char>) {
        unsafe {
            debug_assert!(!self.cmdline_buf.is_null());
            *self.cmdline_buf = new_buf;
            let new_ptr = (*self.cmdline_buf).as_mut_ptr();
            if !cmdlinep.is_null() {
                *cmdlinep = new_ptr;
            }
            if !self.cmdlinep.is_null() {
                *self.cmdlinep = new_ptr;
            }
        }
    }
}

impl Default for exarg_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct cmdmod_T {
    pub cmod_flags: c_int,
    pub cmod_split: c_int,
    pub cmod_tab: c_int,
    pub cmod_filter_pat: Option<std::ffi::CString>,
    pub cmod_filter_regmatch: regmatch_T,
    pub cmod_filter_force: bool,
    pub cmod_verbose: c_int,
    pub cmod_save_ei: Option<std::ffi::CString>,
    pub cmod_did_sandbox: c_int,
    pub cmod_verbose_save: i64,
    pub cmod_save_msg_silent: c_int,
    pub cmod_save_msg_scroll: c_int,
    pub cmod_did_esilent: c_int,
}

impl cmdmod_T {
    pub const fn new() -> Self {
        Self {
            cmod_flags: 0,
            cmod_split: 0,
            cmod_tab: 0,
            cmod_filter_pat: None,
            cmod_filter_regmatch: regmatch_T::new(),
            cmod_filter_force: false,
            cmod_verbose: 0,
            cmod_save_ei: None,
            cmod_did_sandbox: 0,
            cmod_verbose_save: 0,
            cmod_save_msg_silent: 0,
            cmod_save_msg_scroll: 0,
            cmod_did_esilent: 0,
        }
    }
}

impl Default for cmdmod_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
use crate::api::vim::kCdScopeGlobal;
use crate::charset::NUL;
use crate::eval::typval::kCallbackNone;
use crate::extmark::{kFalse, kNone, kTrue};
use crate::keycodes::{K_SPECIAL, KS_SPECIAL};
use crate::mapping::{REMAP_NONE, REMAP_YES};
use crate::os::fs::MAXPATHL;
use crate::search::{BACKWARD, FORWARD};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CommandDefinition {
    pub cmd_name: &'static CStr,
    pub cmd_func: ex_func_T,
    pub cmd_preview_func: ex_preview_func_T,
    pub cmd_argt: u32,
    pub cmd_addr_type: cmd_addr_T,
}

pub(crate) const UNDO_HASH_SIZE: usize = 32;

pub(crate) const DOCMD_EXCRESET: c_int = 0x10;
pub(crate) const DOCMD_KEEPLINE: c_int = 0x20;

const EX_WHOLEFOLD: u32 = 0x040;
const EX_CTRLV: u32 = 0x2000;
pub(crate) const EX_BUFNAME: u32 = 0x8000;
const EX_BUFUNL: u32 = 0x10000;
const EX_BUFLOCK_OK: u32 = 0x80000;
const EX_MODIFY: u32 = 0x100000;
const EX_FLAGS: u32 = 0x200000;
const EX_LOCK_OK: u32 = 0x1000000;

const ADDR_TABS_RELATIVE: cmd_addr_T = 6;
const ADDR_QUICKFIX_VALID: cmd_addr_T = 7;
const ADDR_UNSIGNED: cmd_addr_T = 9;

const Ctrl_B: c_char = 2;
pub(crate) const Ctrl_C: c_char = 3;
const Ctrl_D: c_char = 4;
const Ctrl_F: c_char = 6;
pub(crate) const Ctrl_G: c_char = 7;
const Ctrl_H: c_char = 8;
const Ctrl_I: c_char = 9;
const Ctrl_J: c_char = 10;
const Ctrl_K: c_char = 11;
const Ctrl_L: c_char = 12;
pub(crate) const CAR: c_char = 13;
const Ctrl_N: c_char = 14;
pub(crate) const Ctrl_O: c_char = 15;
const Ctrl_P: c_char = 16;
const Ctrl_Q: c_char = 17;
const Ctrl_R: c_char = 18;
const Ctrl_S: c_char = 19;
const Ctrl_T: c_char = 20;
const Ctrl_W: c_char = 23;
const Ctrl_X: c_char = 24;
pub(crate) const Ctrl_Z: c_char = 26;
const Ctrl_RSB: c_char = 29;
const Ctrl_HAT: c_char = 30;
const Ctrl__: c_char = 31;

pub(crate) const kOptFindfunc: c_int = 99;

const kCpoBar: c_int = b'b' as c_int;
const kCpoAltread: c_int = b'a' as c_int;
const kCpoNosymlinks: c_int = b'~' as c_int;
const kCpoExecbuf: c_int = b'e' as c_int;

pub(crate) const FLUSH_TYPEAHEAD: c_int = 1;

pub(crate) const kCdCauseManual: CdCause = 0;

pub(crate) const KE_FILLER: c_char = b'X' as c_char;

pub(crate) const FIND_ANY: c_int = 1;
pub(crate) const FIND_DEFINE: c_int = 2;
pub(crate) const CHECK_PATH: c_int = 3;
pub(crate) const ACTION_SHOW: c_int = 1;
pub(crate) const ACTION_GOTO: c_int = 2;
pub(crate) const ACTION_SPLIT: c_int = 3;
pub(crate) const ACTION_SHOW_ALL: c_int = 4;
pub(crate) const SEARCH_MSG: c_int = 0x0c;

pub(crate) const DT_TAG: c_int = 1;
pub(crate) const DT_POP: c_int = 2;
pub(crate) const DT_NEXT: c_int = 3;
pub(crate) const DT_PREV: c_int = 4;
pub(crate) const DT_FIRST: c_int = 5;
pub(crate) const DT_LAST: c_int = 6;
pub(crate) const DT_SELECT: c_int = 7;
pub(crate) const DT_JUMP: c_int = 9;
pub(crate) const DT_LTAG: c_int = 11;

const FILETYPE_FILE: &CStr = c"filetype.lua filetype.vim";
const FTPLUGIN_FILE: &CStr = c"ftplugin.vim";
const INDENT_FILE: &CStr = c"indent.vim";
const FTOFF_FILE: &CStr = c"ftoff.vim";
const FTPLUGOF_FILE: &CStr = c"ftplugof.vim";
const INDOFF_FILE: &CStr = c"indoff.vim";

const e_ambiguous_use_of_user_defined_command: &CStr =
    c"E464: Ambiguous use of user-defined command";
const e_no_call_stack_to_substitute_for_stack: &CStr =
    c"E489: No call stack to substitute for \"<stack>\"";
const e_not_an_editor_command: &CStr = c"E492: Not an editor command";
const e_no_autocommand_file_name_to_substitute_for_afile: &CStr =
    c"E495: No autocommand file name to substitute for \"<afile>\"";
const e_no_autocommand_buffer_number_to_substitute_for_abuf: &CStr =
    c"E496: No autocommand buffer number to substitute for \"<abuf>\"";
const e_no_autocommand_match_name_to_substitute_for_amatch: &CStr =
    c"E497: No autocommand match name to substitute for \"<amatch>\"";
const e_no_source_file_name_to_substitute_for_sfile: &CStr =
    c"E498: No :source file name to substitute for \"<sfile>\"";
const e_no_line_number_to_use_for_slnum: &CStr = c"E842: No line number to use for \"<slnum>\"";
const e_no_line_number_to_use_for_sflnum: &CStr = c"E961: No line number to use for \"<sflnum>\"";
const e_no_script_file_name_to_substitute_for_script: &CStr =
    c"E1274: No script file name to substitute for \"<script>\"";

use crate::errors::*;

const COMMAND_COUNT: c_int = 564;
const CMD_SIZE_I32: c_int = cmdidx_T::CMD_SIZE as c_int;

struct ExDocmdState {
    quitmore: c_int,
    cmdline_call_depth: c_int,
    dollar_command: [c_char; 2],
    prev_dir: Option<std::ffi::CString>,
    filetype_detect: TriState,
    filetype_plugin: TriState,
    filetype_indent: TriState,
    EX_ERROR_BUF: [c_char; MSG_BUF_LEN],
}

impl ExDocmdState {
    const fn new() -> Self {
        Self {
            quitmore: 0,
            cmdline_call_depth: 0,
            dollar_command: [b'$' as c_char, 0],
            prev_dir: None,
            filetype_detect: kNone,
            filetype_plugin: kNone,
            filetype_indent: kNone,
            EX_ERROR_BUF: [0; MSG_BUF_LEN],
        }
    }
}

static DOCMD_STATE: GlobalState<ExDocmdState> = GlobalState::new(ExDocmdState::new());

#[inline]
fn docmd_st() -> &'static mut ExDocmdState {
    unsafe { &mut *DOCMD_STATE.as_ptr() }
}

struct wcmd_T {
    line: std::ffi::CString,
    lnum: linenr_T,
}

#[repr(C)]
pub struct loop_cookie {
    lines: Vec<wcmd_T>,
    current_line: c_int,
    repeating: c_int,
    pub lc_getline: LineGetter,
}

impl loop_cookie {
    const fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_line: 0,
            repeating: 0,
            lc_getline: LineGetter::None,
        }
    }
}

impl Default for loop_cookie {
    fn default() -> Self {
        Self::new()
    }
}

struct dbg_stuff {
    trylevel: c_int,
    force_abort: c_int,
    caught_stack: Vec<std::rc::Rc<except_T>>,
    vv_exception: Option<std::ffi::CString>,
    vv_throwpoint: Option<std::ffi::CString>,
    did_emsg: c_int,
    got_int: c_int,
    did_throw: bool,
    need_rethrow: c_int,
    check_cstack: c_int,
    current_exception: Option<std::rc::Rc<except_T>>,
}

impl dbg_stuff {
    const fn new() -> Self {
        Self {
            trylevel: 0,
            force_abort: 0,
            caught_stack: Vec::new(),
            vv_exception: None,
            vv_throwpoint: None,
            did_emsg: 0,
            got_int: 0,
            did_throw: false,
            need_rethrow: 0,
            check_cstack: 0,
            current_exception: None,
        }
    }
}

struct CmdModInfo {
    name: &'static [u8],
    minlen: usize,
    has_count: bool,
}

const CMDMODS: [CmdModInfo; 24] = [
    CmdModInfo {
        name: b"aboveleft",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"belowright",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"botright",
        minlen: 2,
        has_count: false,
    },
    CmdModInfo {
        name: b"browse",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"confirm",
        minlen: 4,
        has_count: false,
    },
    CmdModInfo {
        name: b"filter",
        minlen: 4,
        has_count: false,
    },
    CmdModInfo {
        name: b"hide",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"horizontal",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"keepalt",
        minlen: 5,
        has_count: false,
    },
    CmdModInfo {
        name: b"keepjumps",
        minlen: 5,
        has_count: false,
    },
    CmdModInfo {
        name: b"keepmarks",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"keeppatterns",
        minlen: 5,
        has_count: false,
    },
    CmdModInfo {
        name: b"leftabove",
        minlen: 5,
        has_count: false,
    },
    CmdModInfo {
        name: b"lockmarks",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"noautocmd",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"noswapfile",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"rightbelow",
        minlen: 6,
        has_count: false,
    },
    CmdModInfo {
        name: b"sandbox",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"silent",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"tab",
        minlen: 3,
        has_count: true,
    },
    CmdModInfo {
        name: b"topleft",
        minlen: 2,
        has_count: false,
    },
    CmdModInfo {
        name: b"unsilent",
        minlen: 3,
        has_count: false,
    },
    CmdModInfo {
        name: b"verbose",
        minlen: 4,
        has_count: true,
    },
    CmdModInfo {
        name: b"vertical",
        minlen: 4,
        has_count: false,
    },
];

static CMDNAMES: [CommandDefinition; 564] = [
    CommandDefinition {
        cmd_name: c"append",
        cmd_func: ex_append,
        cmd_preview_func: None,
        cmd_argt: 18354435,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"abbreviate",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"abclear",
        cmd_func: ex_abclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"aboveleft",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"all",
        cmd_func: ex_all,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"amenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"anoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"args",
        cmd_func: ex_args,
        cmd_preview_func: None,
        cmd_argt: 147726,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"argadd",
        cmd_func: ex_argadd,
        cmd_preview_func: None,
        cmd_argt: 4367,
        cmd_addr_type: ADDR_ARGUMENTS,
    },
    CommandDefinition {
        cmd_name: c"argdelete",
        cmd_func: ex_argdelete,
        cmd_preview_func: None,
        cmd_argt: 271,
        cmd_addr_type: ADDR_ARGUMENTS,
    },
    CommandDefinition {
        cmd_name: c"argdo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2215,
        cmd_addr_type: ADDR_ARGUMENTS,
    },
    CommandDefinition {
        cmd_name: c"argdedupe",
        cmd_func: ex_argdedupe,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"argedit",
        cmd_func: ex_argedit,
        cmd_preview_func: None,
        cmd_argt: 151951,
        cmd_addr_type: ADDR_ARGUMENTS,
    },
    CommandDefinition {
        cmd_name: c"argglobal",
        cmd_func: ex_args,
        cmd_preview_func: None,
        cmd_argt: 147726,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"arglocal",
        cmd_func: ex_args,
        cmd_preview_func: None,
        cmd_argt: 147726,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"argument",
        cmd_func: ex_argument,
        cmd_preview_func: None,
        cmd_argt: 148743,
        cmd_addr_type: ADDR_ARGUMENTS,
    },
    CommandDefinition {
        cmd_name: c"ascii",
        cmd_func: do_ascii,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"autocmd",
        cmd_func: ex_autocmd,
        cmd_preview_func: None,
        cmd_argt: 17311750,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"augroup",
        cmd_func: ex_autocmd,
        cmd_preview_func: None,
        cmd_argt: 17301782,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"aunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"buffer",
        cmd_func: ex_buffer,
        cmd_preview_func: None,
        cmd_argt: 115975,
        cmd_addr_type: ADDR_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"bNext",
        cmd_func: ex_bprevious,
        cmd_preview_func: None,
        cmd_argt: 17667,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ball",
        cmd_func: ex_buffer_all,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"badd",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 17318300,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"balt",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 17318300,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"bcd",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"bchdir",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"bdelete",
        cmd_func: ex_bunload,
        cmd_preview_func: None,
        cmd_argt: 34055,
        cmd_addr_type: ADDR_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"belowright",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"bfirst",
        cmd_func: ex_brewind,
        cmd_preview_func: None,
        cmd_argt: 16643,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"blast",
        cmd_func: ex_blast,
        cmd_preview_func: None,
        cmd_argt: 16643,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"bmodified",
        cmd_func: ex_bmodified,
        cmd_preview_func: None,
        cmd_argt: 17667,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"bnext",
        cmd_func: ex_bnext,
        cmd_preview_func: None,
        cmd_argt: 17667,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"botright",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"bprevious",
        cmd_func: ex_bprevious,
        cmd_preview_func: None,
        cmd_argt: 17667,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"brewind",
        cmd_func: ex_brewind,
        cmd_preview_func: None,
        cmd_argt: 16643,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"break",
        cmd_func: ex_break,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"breakadd",
        cmd_func: ex_breakadd,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"breakdel",
        cmd_func: ex_breakdel,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"breaklist",
        cmd_func: ex_breaklist,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"browse",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 17303684,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"buffers",
        cmd_func: buflist_list,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"bufdo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2215,
        cmd_addr_type: ADDR_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"bunload",
        cmd_func: ex_bunload,
        cmd_preview_func: None,
        cmd_argt: 34055,
        cmd_addr_type: ADDR_LOADED_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"bwipeout",
        cmd_func: ex_bunload,
        cmd_preview_func: None,
        cmd_argt: 99591,
        cmd_addr_type: ADDR_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"change",
        cmd_func: ex_change,
        cmd_preview_func: None,
        cmd_argt: 18351427,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"cNext",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cNfile",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cabclear",
        cmd_func: ex_abclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cabove",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"caddbuffer",
        cmd_func: ex_cbuffer,
        cmd_preview_func: None,
        cmd_argt: 277,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"caddexpr",
        cmd_func: ex_cexpr,
        cmd_preview_func: None,
        cmd_argt: 2196,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"caddfile",
        cmd_func: ex_cfile,
        cmd_preview_func: None,
        cmd_argt: 284,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cafter",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"call",
        cmd_func: ex_call,
        cmd_preview_func: None,
        cmd_argt: 17565829,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"catch",
        cmd_func: ex_catch,
        cmd_preview_func: None,
        cmd_argt: 17563652,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cbuffer",
        cmd_func: ex_cbuffer,
        cmd_preview_func: None,
        cmd_argt: 279,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"cbefore",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cbelow",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cbottom",
        cmd_func: ex_cbottom,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cc",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_QUICKFIX,
    },
    CommandDefinition {
        cmd_name: c"cclose",
        cmd_func: ex_cclose,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cd",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cdo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2215,
        cmd_addr_type: ADDR_QUICKFIX_VALID,
    },
    CommandDefinition {
        cmd_name: c"center",
        cmd_func: ex_align,
        cmd_preview_func: None,
        cmd_argt: 18350405,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"cexpr",
        cmd_func: ex_cexpr,
        cmd_preview_func: None,
        cmd_argt: 2198,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cfile",
        cmd_func: ex_cfile,
        cmd_preview_func: None,
        cmd_argt: 286,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cfdo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2215,
        cmd_addr_type: ADDR_QUICKFIX_VALID,
    },
    CommandDefinition {
        cmd_name: c"cfirst",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cgetfile",
        cmd_func: ex_cfile,
        cmd_preview_func: None,
        cmd_argt: 284,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cgetbuffer",
        cmd_func: ex_cbuffer,
        cmd_preview_func: None,
        cmd_argt: 277,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"cgetexpr",
        cmd_func: ex_cexpr,
        cmd_preview_func: None,
        cmd_argt: 2196,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"chdir",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"changes",
        cmd_func: ex_changes,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"checkhealth",
        cmd_func: ex_checkhealth,
        cmd_preview_func: None,
        cmd_argt: 260,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"checkpath",
        cmd_func: ex_checkpath,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"checktime",
        cmd_func: ex_checktime,
        cmd_preview_func: None,
        cmd_argt: 34053,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"chistory",
        cmd_func: qf_history,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"clist",
        cmd_func: qf_list,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"clast",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"close",
        cmd_func: ex_close,
        cmd_preview_func: None,
        cmd_argt: 17302787,
        cmd_addr_type: ADDR_WINDOWS,
    },
    CommandDefinition {
        cmd_name: c"clearjumps",
        cmd_func: ex_clearjumps,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cmap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cmapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"cnext",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cnewer",
        cmd_func: qf_age,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cnfile",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cnoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cnoreabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cnoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"copy",
        cmd_func: ex_copymove,
        cmd_preview_func: None,
        cmd_argt: 18350405,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"colder",
        cmd_func: qf_age,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"colorscheme",
        cmd_func: ex_colorscheme,
        cmd_preview_func: None,
        cmd_argt: 17301780,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"command",
        cmd_func: ex_command,
        cmd_preview_func: None,
        cmd_argt: 17311750,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"comclear",
        cmd_func: ex_comclear,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"compiler",
        cmd_func: ex_compiler,
        cmd_preview_func: None,
        cmd_argt: 17301782,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"continue",
        cmd_func: ex_continue,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"confirm",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 17303684,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"connect",
        cmd_func: ex_connect,
        cmd_preview_func: None,
        cmd_argt: 2206,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"const",
        cmd_func: ex_let,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"copen",
        cmd_func: ex_copen,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"cprevious",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cpfile",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"cquit",
        cmd_func: ex_cquit,
        cmd_preview_func: None,
        cmd_argt: 5379,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"crewind",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"cunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cunabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"cwindow",
        cmd_func: ex_cwindow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"delete",
        cmd_func: ex_operators,
        cmd_preview_func: None,
        cmd_argt: 18351937,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"delmarks",
        cmd_func: ex_delmarks,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"debug",
        cmd_func: ex_debug,
        cmd_preview_func: None,
        cmd_argt: 17565828,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"debuggreedy",
        cmd_func: ex_debuggreedy,
        cmd_preview_func: None,
        cmd_argt: 17305857,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"defer",
        cmd_func: ex_call,
        cmd_preview_func: None,
        cmd_argt: 17565828,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"delcommand",
        cmd_func: ex_delcommand,
        cmd_preview_func: None,
        cmd_argt: 17301910,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"delfunction",
        cmd_func: ex_delfunction,
        cmd_preview_func: None,
        cmd_argt: 17301654,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"detach",
        cmd_func: ex_detach,
        cmd_preview_func: None,
        cmd_argt: 259,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"display",
        cmd_func: ex_display,
        cmd_preview_func: None,
        cmd_argt: 17565956,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"diffupdate",
        cmd_func: ex_diffupdate,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"diffget",
        cmd_func: ex_diffgetput,
        cmd_preview_func: None,
        cmd_argt: 1052933,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"diffoff",
        cmd_func: ex_diffoff,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"diffpatch",
        cmd_func: ex_diffpatch,
        cmd_preview_func: None,
        cmd_argt: 1048860,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"diffput",
        cmd_func: ex_diffgetput,
        cmd_preview_func: None,
        cmd_argt: 4357,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"diffsplit",
        cmd_func: ex_diffsplit,
        cmd_preview_func: None,
        cmd_argt: 284,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"diffthis",
        cmd_func: ex_diffthis,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"digraphs",
        cmd_func: ex_digraphs,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"djump",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 103,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"dlist",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 17301607,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"doautocmd",
        cmd_func: ex_doautocmd,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"doautoall",
        cmd_func: ex_doautoall,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"drop",
        cmd_func: ex_drop,
        cmd_preview_func: None,
        cmd_argt: 147854,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"dsearch",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 17301607,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"dsplit",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 103,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"edit",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 147742,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"earlier",
        cmd_func: ex_later,
        cmd_preview_func: None,
        cmd_argt: 17301780,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"echo",
        cmd_func: ex_echo,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"echoerr",
        cmd_func: ex_execute,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"echohl",
        cmd_func: ex_echohl,
        cmd_preview_func: None,
        cmd_argt: 17563908,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"echomsg",
        cmd_func: ex_execute,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"echon",
        cmd_func: ex_echo,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"else",
        cmd_func: ex_else,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"elseif",
        cmd_func: ex_else,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"emenu",
        cmd_func: ex_emenu,
        cmd_preview_func: None,
        cmd_argt: 17303941,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"endif",
        cmd_func: ex_endif,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"endfunction",
        cmd_func: ex_endfunction,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"endfor",
        cmd_func: ex_endwhile,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"endtry",
        cmd_func: ex_endtry,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"endwhile",
        cmd_func: ex_endwhile,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"enew",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"eval",
        cmd_func: ex_eval,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ex",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 147742,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"execute",
        cmd_func: ex_execute,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"exit",
        cmd_func: ex_exit,
        cmd_preview_func: None,
        cmd_argt: 17432959,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"exmode",
        cmd_func: ex_exmode,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"exusage",
        cmd_func: ex_exusage,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"file",
        cmd_func: ex_file,
        cmd_preview_func: None,
        cmd_argt: 4383,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"files",
        cmd_func: buflist_list,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"filetype",
        cmd_func: ex_filetype,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"filter",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2182,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"find",
        cmd_func: ex_find,
        cmd_preview_func: None,
        cmd_argt: 147871,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"finally",
        cmd_func: ex_finally,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"finish",
        cmd_func: ex_finish,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"first",
        cmd_func: ex_rewind,
        cmd_preview_func: None,
        cmd_argt: 147718,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"fold",
        cmd_func: ex_fold,
        cmd_preview_func: None,
        cmd_argt: 17563969,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"foldclose",
        cmd_func: ex_foldopen,
        cmd_preview_func: None,
        cmd_argt: 17563971,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"folddoopen",
        cmd_func: ex_folddo,
        cmd_preview_func: None,
        cmd_argt: 2213,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"folddoclosed",
        cmd_func: ex_folddo,
        cmd_preview_func: None,
        cmd_argt: 2213,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"foldopen",
        cmd_func: ex_foldopen,
        cmd_preview_func: None,
        cmd_argt: 17563971,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"for",
        cmd_func: ex_while,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"function",
        cmd_func: ex_function,
        cmd_preview_func: None,
        cmd_argt: 17563654,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"fclose",
        cmd_func: ex_fclose,
        cmd_preview_func: None,
        cmd_argt: 259,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"global",
        cmd_func: ex_global,
        cmd_preview_func: None,
        cmd_argt: 17563751,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"goto",
        cmd_func: ex_goto,
        cmd_preview_func: None,
        cmd_argt: 17564929,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"grep",
        cmd_func: ex_make,
        cmd_preview_func: None,
        cmd_argt: 2447,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"grepadd",
        cmd_func: ex_make,
        cmd_preview_func: None,
        cmd_argt: 2447,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"gui",
        cmd_func: ex_nogui,
        cmd_preview_func: None,
        cmd_argt: 17449230,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"gvim",
        cmd_func: ex_nogui,
        cmd_preview_func: None,
        cmd_argt: 17449230,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"help",
        cmd_func: ex_help,
        cmd_preview_func: None,
        cmd_argt: 2054,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"helpclose",
        cmd_func: ex_helpclose,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"helpgrep",
        cmd_func: ex_helpgrep,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"helptags",
        cmd_func: ex_helptags,
        cmd_preview_func: None,
        cmd_argt: 17301900,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"highlight",
        cmd_func: ex_highlight,
        cmd_preview_func: None,
        cmd_argt: 17563910,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"hide",
        cmd_func: ex_hide,
        cmd_preview_func: None,
        cmd_argt: 1287,
        cmd_addr_type: ADDR_WINDOWS,
    },
    CommandDefinition {
        cmd_name: c"history",
        cmd_func: ex_history,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"horizontal",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"insert",
        cmd_func: ex_append,
        cmd_preview_func: None,
        cmd_argt: 18350339,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"iabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"iabclear",
        cmd_func: ex_abclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"if",
        cmd_func: ex_if,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ijump",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 103,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"ilist",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 17301607,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"imap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"imapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"imenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"inoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"inoreabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"inoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"intro",
        cmd_func: ex_intro,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"iput",
        cmd_func: ex_iput,
        cmd_preview_func: None,
        cmd_argt: 18355011,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"isearch",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 17301607,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"isplit",
        cmd_func: ex_findpat,
        cmd_preview_func: None,
        cmd_argt: 103,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"iunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"iunabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"iunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"join",
        cmd_func: ex_join,
        cmd_preview_func: None,
        cmd_argt: 20448579,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"jumps",
        cmd_func: ex_jumps,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"k",
        cmd_func: ex_mark,
        cmd_preview_func: None,
        cmd_argt: 17563925,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"keepmarks",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"keepjumps",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"keeppatterns",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"keepalt",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"list",
        cmd_func: ex_print,
        cmd_preview_func: None,
        cmd_argt: 19400001,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"lNext",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lNfile",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"last",
        cmd_func: ex_last,
        cmd_preview_func: None,
        cmd_argt: 147718,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"labove",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"language",
        cmd_func: ex_language,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"laddexpr",
        cmd_func: ex_cexpr,
        cmd_preview_func: None,
        cmd_argt: 2196,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"laddbuffer",
        cmd_func: ex_cbuffer,
        cmd_preview_func: None,
        cmd_argt: 277,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"laddfile",
        cmd_func: ex_cfile,
        cmd_preview_func: None,
        cmd_argt: 284,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lafter",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"later",
        cmd_func: ex_later,
        cmd_preview_func: None,
        cmd_argt: 17301780,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lbuffer",
        cmd_func: ex_cbuffer,
        cmd_preview_func: None,
        cmd_argt: 279,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"lbefore",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lbelow",
        cmd_func: ex_cbelow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lbottom",
        cmd_func: ex_cbottom,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lcd",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lchdir",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lclose",
        cmd_func: ex_cclose,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ldo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2215,
        cmd_addr_type: ADDR_QUICKFIX_VALID,
    },
    CommandDefinition {
        cmd_name: c"left",
        cmd_func: ex_align,
        cmd_preview_func: None,
        cmd_argt: 18350405,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"leftabove",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"let",
        cmd_func: ex_let,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lexpr",
        cmd_func: ex_cexpr,
        cmd_preview_func: None,
        cmd_argt: 2198,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lfile",
        cmd_func: ex_cfile,
        cmd_preview_func: None,
        cmd_argt: 286,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lfdo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2215,
        cmd_addr_type: ADDR_QUICKFIX_VALID,
    },
    CommandDefinition {
        cmd_name: c"lfirst",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lgetfile",
        cmd_func: ex_cfile,
        cmd_preview_func: None,
        cmd_argt: 284,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lgetbuffer",
        cmd_func: ex_cbuffer,
        cmd_preview_func: None,
        cmd_argt: 277,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"lgetexpr",
        cmd_func: ex_cexpr,
        cmd_preview_func: None,
        cmd_argt: 2196,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lgrep",
        cmd_func: ex_make,
        cmd_preview_func: None,
        cmd_argt: 2447,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"lgrepadd",
        cmd_func: ex_make,
        cmd_preview_func: None,
        cmd_argt: 2447,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"lhelpgrep",
        cmd_func: ex_helpgrep,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lhistory",
        cmd_func: qf_history,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"ll",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_QUICKFIX,
    },
    CommandDefinition {
        cmd_name: c"llast",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"llist",
        cmd_func: qf_list,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lmap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lmapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lmake",
        cmd_func: ex_make,
        cmd_preview_func: None,
        cmd_argt: 2318,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lnoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lnext",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lnewer",
        cmd_func: qf_age,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lnfile",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"loadview",
        cmd_func: ex_loadview,
        cmd_preview_func: None,
        cmd_argt: 284,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"loadkeymap",
        cmd_func: ex_loadkeymap,
        cmd_preview_func: None,
        cmd_argt: 17301504,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lockmarks",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lockvar",
        cmd_func: ex_lockvar,
        cmd_preview_func: None,
        cmd_argt: 17563782,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"log",
        cmd_func: ex_log,
        cmd_preview_func: None,
        cmd_argt: 260,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lolder",
        cmd_func: qf_age,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lopen",
        cmd_func: ex_copen,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"lprevious",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"lpfile",
        cmd_func: ex_cnext,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"lrewind",
        cmd_func: ex_cc,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_UNSIGNED,
    },
    CommandDefinition {
        cmd_name: c"ltag",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lua",
        cmd_func: ex_lua,
        cmd_preview_func: None,
        cmd_argt: 17301509,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"luado",
        cmd_func: ex_luado,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"luafile",
        cmd_func: ex_luafile,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"lvimgrep",
        cmd_func: ex_vimgrep,
        cmd_preview_func: None,
        cmd_argt: 16779663,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"lvimgrepadd",
        cmd_func: ex_vimgrep,
        cmd_preview_func: None,
        cmd_argt: 16779663,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"lwindow",
        cmd_func: ex_cwindow,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ls",
        cmd_func: buflist_list,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"lsp",
        cmd_func: ex_lsp,
        cmd_preview_func: None,
        cmd_argt: 132,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"move",
        cmd_func: ex_copymove,
        cmd_preview_func: None,
        cmd_argt: 18350405,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"mark",
        cmd_func: ex_mark,
        cmd_preview_func: None,
        cmd_argt: 17563925,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"make",
        cmd_func: ex_make,
        cmd_preview_func: None,
        cmd_argt: 2318,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"map",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312006,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"marks",
        cmd_func: ex_marks,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"match",
        cmd_func: ex_match,
        cmd_preview_func: None,
        cmd_argt: 17301509,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"menu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316103,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"menutranslate",
        cmd_func: ex_menutranslate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"messages",
        cmd_func: ex_messages,
        cmd_preview_func: None,
        cmd_argt: 17301765,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"mkexrc",
        cmd_func: ex_mkrc,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mksession",
        cmd_func: ex_mkrc,
        cmd_preview_func: None,
        cmd_argt: 286,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mkspell",
        cmd_func: ex_mkspell,
        cmd_preview_func: None,
        cmd_argt: 2446,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mkvimrc",
        cmd_func: ex_mkrc,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mkview",
        cmd_func: ex_mkrc,
        cmd_preview_func: None,
        cmd_argt: 286,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mode",
        cmd_func: ex_mode,
        cmd_preview_func: None,
        cmd_argt: 17301780,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"mzscheme",
        cmd_func: ex_script_ni,
        cmd_preview_func: None,
        cmd_argt: 17563813,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"mzfile",
        cmd_func: ex_ni,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"next",
        cmd_func: ex_next,
        cmd_preview_func: None,
        cmd_argt: 147727,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"new",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 147743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"nmap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"nmapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"nmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"nnoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"nnoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"noremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312006,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"noautocmd",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"nohlsearch",
        cmd_func: ex_nohlsearch,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"noreabbrev",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"noremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316103,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"noswapfile",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"normal",
        cmd_func: ex_normal,
        cmd_preview_func: None,
        cmd_argt: 17574023,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"number",
        cmd_func: ex_print,
        cmd_preview_func: None,
        cmd_argt: 19400001,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"nunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"nunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"oldfiles",
        cmd_func: ex_oldfiles,
        cmd_preview_func: None,
        cmd_argt: 17563906,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"omap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"omapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"omenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"only",
        cmd_func: ex_only,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_WINDOWS,
    },
    CommandDefinition {
        cmd_name: c"onoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"onoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"options",
        cmd_func: ex_options,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ounmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ounmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ownsyntax",
        cmd_func: ex_ownsyntax,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"print",
        cmd_func: ex_print,
        cmd_preview_func: None,
        cmd_argt: 19662145,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"packadd",
        cmd_func: ex_packadd,
        cmd_preview_func: None,
        cmd_argt: 17564062,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"packdel",
        cmd_func: ex_packdel,
        cmd_preview_func: None,
        cmd_argt: 390,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"packloadall",
        cmd_func: ex_packloadall,
        cmd_preview_func: None,
        cmd_argt: 17563906,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"packupdate",
        cmd_func: ex_packupdate,
        cmd_preview_func: None,
        cmd_argt: 262,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"pbuffer",
        cmd_func: ex_pbuffer,
        cmd_preview_func: None,
        cmd_argt: 115975,
        cmd_addr_type: ADDR_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"pclose",
        cmd_func: ex_pclose,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"perl",
        cmd_func: ex_perl,
        cmd_preview_func: None,
        cmd_argt: 17563813,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"perldo",
        cmd_func: ex_perldo,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"perlfile",
        cmd_func: ex_perlfile,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pedit",
        cmd_func: ex_pedit,
        cmd_preview_func: None,
        cmd_argt: 147742,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"pop",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 5379,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"popup",
        cmd_func: ex_popup,
        cmd_preview_func: None,
        cmd_argt: 17303942,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ppop",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 5379,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"preserve",
        cmd_func: ex_preserve,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"previous",
        cmd_func: ex_previous,
        cmd_preview_func: None,
        cmd_argt: 148743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"profile",
        cmd_func: ex_profile,
        cmd_preview_func: None,
        cmd_argt: 17301766,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"profdel",
        cmd_func: ex_breakdel,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"psearch",
        cmd_func: ex_psearch,
        cmd_preview_func: None,
        cmd_argt: 103,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"ptag",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 4375,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ptNext",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ptfirst",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ptjump",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ptlast",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ptnext",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ptprevious",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ptrewind",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"ptselect",
        cmd_func: ex_ptag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"put",
        cmd_func: ex_put,
        cmd_preview_func: None,
        cmd_argt: 18355011,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pwd",
        cmd_func: ex_pwd,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"python",
        cmd_func: ex_python3,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pydo",
        cmd_func: ex_pydo3,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pyfile",
        cmd_func: ex_py3file,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"py3",
        cmd_func: ex_python3,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"py3do",
        cmd_func: ex_pydo3,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"python3",
        cmd_func: ex_python3,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"py3file",
        cmd_func: ex_py3file,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pyx",
        cmd_func: ex_python3,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pyxdo",
        cmd_func: ex_pydo3,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pythonx",
        cmd_func: ex_python3,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"pyxfile",
        cmd_func: ex_py3file,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"quit",
        cmd_func: ex_quit,
        cmd_preview_func: None,
        cmd_argt: 17302787,
        cmd_addr_type: ADDR_WINDOWS,
    },
    CommandDefinition {
        cmd_name: c"quitall",
        cmd_func: ex_quitall,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"qall",
        cmd_func: ex_quitall,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"read",
        cmd_func: ex_read,
        cmd_preview_func: None,
        cmd_argt: 18485599,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"recover",
        cmd_func: ex_recover,
        cmd_preview_func: None,
        cmd_argt: 286,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"redo",
        cmd_func: ex_redo,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"redir",
        cmd_func: ex_redir,
        cmd_preview_func: None,
        cmd_argt: 17301774,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"redraw",
        cmd_func: ex_redraw,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"redrawstatus",
        cmd_func: ex_redrawstatus,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"redrawtabline",
        cmd_func: ex_redrawtabline,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"registers",
        cmd_func: ex_display,
        cmd_preview_func: None,
        cmd_argt: 17565956,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"resize",
        cmd_func: ex_resize,
        cmd_preview_func: None,
        cmd_argt: 17301781,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"restart",
        cmd_func: ex_restart,
        cmd_preview_func: None,
        cmd_argt: 18438,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"retab",
        cmd_func: ex_retab,
        cmd_preview_func: None,
        cmd_argt: 18350455,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"return",
        cmd_func: ex_return,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"rewind",
        cmd_func: ex_rewind,
        cmd_preview_func: None,
        cmd_argt: 147718,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"right",
        cmd_func: ex_align,
        cmd_preview_func: None,
        cmd_argt: 18350405,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"rightbelow",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"rshada",
        cmd_func: ex_shada,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"runtime",
        cmd_func: ex_runtime,
        cmd_preview_func: None,
        cmd_argt: 17564046,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"rundo",
        cmd_func: ex_rundo,
        cmd_preview_func: None,
        cmd_argt: 156,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"ruby",
        cmd_func: ex_ruby,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"rubydo",
        cmd_func: ex_rubydo,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"rubyfile",
        cmd_func: ex_rubyfile,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"rviminfo",
        cmd_func: ex_shada,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"substitute",
        cmd_func: ex_substitute,
        cmd_preview_func: Some(ex_substitute_preview),
        cmd_argt: 151519301,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"sNext",
        cmd_func: ex_previous,
        cmd_preview_func: None,
        cmd_argt: 148743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sargument",
        cmd_func: ex_argument,
        cmd_preview_func: None,
        cmd_argt: 148743,
        cmd_addr_type: ADDR_ARGUMENTS,
    },
    CommandDefinition {
        cmd_name: c"sall",
        cmd_func: ex_all,
        cmd_preview_func: None,
        cmd_argt: 1283,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sandbox",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"saveas",
        cmd_func: ex_write,
        cmd_preview_func: None,
        cmd_argt: 17432862,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sbuffer",
        cmd_func: ex_buffer,
        cmd_preview_func: None,
        cmd_argt: 115975,
        cmd_addr_type: ADDR_BUFFERS,
    },
    CommandDefinition {
        cmd_name: c"sbNext",
        cmd_func: ex_bprevious,
        cmd_preview_func: None,
        cmd_argt: 17665,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sball",
        cmd_func: ex_buffer_all,
        cmd_preview_func: None,
        cmd_argt: 17665,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sbfirst",
        cmd_func: ex_brewind,
        cmd_preview_func: None,
        cmd_argt: 16640,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sblast",
        cmd_func: ex_blast,
        cmd_preview_func: None,
        cmd_argt: 16640,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sbmodified",
        cmd_func: ex_bmodified,
        cmd_preview_func: None,
        cmd_argt: 17665,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sbnext",
        cmd_func: ex_bnext,
        cmd_preview_func: None,
        cmd_argt: 17665,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sbprevious",
        cmd_func: ex_bprevious,
        cmd_preview_func: None,
        cmd_argt: 17665,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sbrewind",
        cmd_func: ex_brewind,
        cmd_preview_func: None,
        cmd_argt: 16640,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"scriptnames",
        cmd_func: ex_scriptnames,
        cmd_preview_func: None,
        cmd_argt: 17302799,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"scriptencoding",
        cmd_func: ex_scriptencoding,
        cmd_preview_func: None,
        cmd_argt: 17301780,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"set",
        cmd_func: ex_set,
        cmd_preview_func: None,
        cmd_argt: 17563910,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"setfiletype",
        cmd_func: ex_setfiletype,
        cmd_preview_func: None,
        cmd_argt: 17301892,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"setglobal",
        cmd_func: ex_set,
        cmd_preview_func: None,
        cmd_argt: 17563910,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"setlocal",
        cmd_func: ex_set,
        cmd_preview_func: None,
        cmd_argt: 17563910,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sfind",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 147871,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sfirst",
        cmd_func: ex_rewind,
        cmd_preview_func: None,
        cmd_argt: 147718,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"simalt",
        cmd_func: ex_ni,
        cmd_preview_func: None,
        cmd_argt: 17301908,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sign",
        cmd_func: ex_sign,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"silent",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 17565830,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sleep",
        cmd_func: ex_sleep,
        cmd_preview_func: None,
        cmd_argt: 17302791,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"slast",
        cmd_func: ex_last,
        cmd_preview_func: None,
        cmd_argt: 147718,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"smagic",
        cmd_func: ex_substitute,
        cmd_preview_func: Some(ex_substitute_preview),
        cmd_argt: 151519301,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"smap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"smapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"smenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"snext",
        cmd_func: ex_next,
        cmd_preview_func: None,
        cmd_argt: 147727,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"snomagic",
        cmd_func: ex_substitute,
        cmd_preview_func: Some(ex_substitute_preview),
        cmd_argt: 151519301,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"snoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"snoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"source",
        cmd_func: ex_source,
        cmd_preview_func: None,
        cmd_argt: 17563967,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"sort",
        cmd_func: ex_sort,
        cmd_preview_func: None,
        cmd_argt: 1050727,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"split",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 147743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"spellgood",
        cmd_func: ex_spell,
        cmd_preview_func: None,
        cmd_argt: 391,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"spelldump",
        cmd_func: ex_spelldump,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"spellinfo",
        cmd_func: ex_spellinfo,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"spellrepall",
        cmd_func: ex_spellrepall,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"spellrare",
        cmd_func: ex_spell,
        cmd_preview_func: None,
        cmd_argt: 391,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"spellundo",
        cmd_func: ex_spell,
        cmd_preview_func: None,
        cmd_argt: 391,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"spellwrong",
        cmd_func: ex_spell,
        cmd_preview_func: None,
        cmd_argt: 391,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sprevious",
        cmd_func: ex_previous,
        cmd_preview_func: None,
        cmd_argt: 148743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"srewind",
        cmd_func: ex_rewind,
        cmd_preview_func: None,
        cmd_argt: 147718,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"stop",
        cmd_func: ex_stop,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"stag",
        cmd_func: ex_stag,
        cmd_preview_func: None,
        cmd_argt: 4375,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"startinsert",
        cmd_func: ex_startinsert,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"startgreplace",
        cmd_func: ex_startinsert,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"startreplace",
        cmd_func: ex_startinsert,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"stopinsert",
        cmd_func: ex_stopinsert,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"stjump",
        cmd_func: ex_stag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"stselect",
        cmd_func: ex_stag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sunhide",
        cmd_func: ex_buffer_all,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"sunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"suspend",
        cmd_func: ex_stop,
        cmd_preview_func: None,
        cmd_argt: 17301762,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"sview",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 147743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"swapname",
        cmd_func: ex_swapname,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"syntax",
        cmd_func: ex_syntax,
        cmd_preview_func: None,
        cmd_argt: 17303556,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"syntime",
        cmd_func: ex_syntime,
        cmd_preview_func: None,
        cmd_argt: 17301908,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"syncbind",
        cmd_func: ex_syncbind,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"t",
        cmd_func: ex_copymove,
        cmd_preview_func: None,
        cmd_argt: 18350405,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"tcd",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tchdir",
        cmd_func: ex_cd,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tNext",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"tag",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 4375,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"tags",
        cmd_func: do_tags,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tab",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 6277,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabclose",
        cmd_func: ex_tabclose,
        cmd_preview_func: None,
        cmd_argt: 17305879,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabdo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2213,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabedit",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 151839,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabfind",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 151967,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabfirst",
        cmd_func: ex_tabnext,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tabmove",
        cmd_func: ex_tabmove,
        cmd_preview_func: None,
        cmd_argt: 4373,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tablast",
        cmd_func: ex_tabnext,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tabnext",
        cmd_func: ex_tabnext,
        cmd_preview_func: None,
        cmd_argt: 4373,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabnew",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 151839,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabonly",
        cmd_func: ex_tabonly,
        cmd_preview_func: None,
        cmd_argt: 17305879,
        cmd_addr_type: ADDR_TABS,
    },
    CommandDefinition {
        cmd_name: c"tabprevious",
        cmd_func: ex_tabnext,
        cmd_preview_func: None,
        cmd_argt: 4373,
        cmd_addr_type: ADDR_TABS_RELATIVE,
    },
    CommandDefinition {
        cmd_name: c"tabNext",
        cmd_func: ex_tabnext,
        cmd_preview_func: None,
        cmd_argt: 4373,
        cmd_addr_type: ADDR_TABS_RELATIVE,
    },
    CommandDefinition {
        cmd_name: c"tabrewind",
        cmd_func: ex_tabnext,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tabs",
        cmd_func: ex_tabs,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tcl",
        cmd_func: ex_script_ni,
        cmd_preview_func: None,
        cmd_argt: 17301637,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"tcldo",
        cmd_func: ex_ni,
        cmd_preview_func: None,
        cmd_argt: 17301669,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"tclfile",
        cmd_func: ex_ni,
        cmd_preview_func: None,
        cmd_argt: 17301661,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"terminal",
        cmd_func: ex_terminal,
        cmd_preview_func: None,
        cmd_argt: 17301518,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tfirst",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"throw",
        cmd_func: ex_throw,
        cmd_preview_func: None,
        cmd_argt: 17563780,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tjump",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tlast",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tlmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"tlnoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"tlunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"tmap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tmapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tnext",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"tnoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"topleft",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tprevious",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"trewind",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 4355,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"trust",
        cmd_func: ex_trust,
        cmd_preview_func: None,
        cmd_argt: 16777500,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"try",
        cmd_func: ex_try,
        cmd_preview_func: None,
        cmd_argt: 17563904,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tselect",
        cmd_func: ex_tag,
        cmd_preview_func: None,
        cmd_argt: 278,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"tunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"undo",
        cmd_func: ex_undo,
        cmd_preview_func: None,
        cmd_argt: 17306883,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"undojoin",
        cmd_func: ex_undojoin,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"undolist",
        cmd_func: ex_undolist,
        cmd_preview_func: None,
        cmd_argt: 17301760,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"unabbreviate",
        cmd_func: ex_abbreviate,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"unhide",
        cmd_func: ex_buffer_all,
        cmd_preview_func: None,
        cmd_argt: 1281,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"uniq",
        cmd_func: ex_uniq,
        cmd_preview_func: None,
        cmd_argt: 1050727,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"unlet",
        cmd_func: ex_unlet,
        cmd_preview_func: None,
        cmd_argt: 17563782,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"unlockvar",
        cmd_func: ex_lockvar,
        cmd_preview_func: None,
        cmd_argt: 17563782,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"unmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312006,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"unmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312006,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"unsilent",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 17565828,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"update",
        cmd_func: ex_update,
        cmd_preview_func: None,
        cmd_argt: 131455,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"uptime",
        cmd_func: ex_uptime,
        cmd_preview_func: None,
        cmd_argt: 17301504,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vglobal",
        cmd_func: ex_global,
        cmd_preview_func: None,
        cmd_argt: 17301605,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"version",
        cmd_func: ex_version,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"verbose",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 17565829,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"vertical",
        cmd_func: ex_wrongmodifier,
        cmd_preview_func: None,
        cmd_argt: 2180,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"visual",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 147742,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"view",
        cmd_func: ex_edit,
        cmd_preview_func: None,
        cmd_argt: 147742,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vimgrep",
        cmd_func: ex_vimgrep,
        cmd_preview_func: None,
        cmd_argt: 16779663,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"vimgrepadd",
        cmd_func: ex_vimgrep,
        cmd_preview_func: None,
        cmd_argt: 16779663,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"viusage",
        cmd_func: ex_viusage,
        cmd_preview_func: None,
        cmd_argt: 256,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vmap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vmapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"vnoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vnew",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 147743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"vnoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"vsplit",
        cmd_func: ex_splitview,
        cmd_preview_func: None,
        cmd_argt: 147743,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"vunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"vunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"write",
        cmd_func: ex_write,
        cmd_preview_func: None,
        cmd_argt: 17432959,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"wNext",
        cmd_func: ex_wnext,
        cmd_preview_func: None,
        cmd_argt: 131423,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"wall",
        cmd_func: do_wqall,
        cmd_preview_func: None,
        cmd_argt: 17432834,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"while",
        cmd_func: ex_while,
        cmd_preview_func: None,
        cmd_argt: 17565700,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"winsize",
        cmd_func: ex_winsize,
        cmd_preview_func: None,
        cmd_argt: 388,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"wincmd",
        cmd_func: ex_wincmd,
        cmd_preview_func: None,
        cmd_argt: 17302677,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"windo",
        cmd_func: ex_listdo,
        cmd_preview_func: None,
        cmd_argt: 2213,
        cmd_addr_type: ADDR_WINDOWS,
    },
    CommandDefinition {
        cmd_name: c"winpos",
        cmd_func: ex_ni,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"wnext",
        cmd_func: ex_wnext,
        cmd_preview_func: None,
        cmd_argt: 131359,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"wprevious",
        cmd_func: ex_wnext,
        cmd_preview_func: None,
        cmd_argt: 131359,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"wq",
        cmd_func: ex_exit,
        cmd_preview_func: None,
        cmd_argt: 131455,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"wqall",
        cmd_func: do_wqall,
        cmd_preview_func: None,
        cmd_argt: 131358,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"wshada",
        cmd_func: ex_shada,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"wundo",
        cmd_func: ex_wundo,
        cmd_preview_func: None,
        cmd_argt: 158,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"wviminfo",
        cmd_func: ex_shada,
        cmd_preview_func: None,
        cmd_argt: 17301790,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"xit",
        cmd_func: ex_exit,
        cmd_preview_func: None,
        cmd_argt: 17432959,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"xall",
        cmd_func: do_wqall,
        cmd_preview_func: None,
        cmd_argt: 258,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"xmap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"xmapclear",
        cmd_func: ex_mapclear,
        cmd_preview_func: None,
        cmd_argt: 17301764,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"xmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"xnoremap",
        cmd_func: ex_map,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"xnoremenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17316101,
        cmd_addr_type: ADDR_OTHER,
    },
    CommandDefinition {
        cmd_name: c"xunmap",
        cmd_func: ex_unmap,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"xunmenu",
        cmd_func: ex_menu,
        cmd_preview_func: None,
        cmd_argt: 17312004,
        cmd_addr_type: ADDR_NONE,
    },
    CommandDefinition {
        cmd_name: c"yank",
        cmd_func: ex_operators,
        cmd_preview_func: None,
        cmd_argt: 17303361,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"z",
        cmd_func: ex_z,
        cmd_preview_func: None,
        cmd_argt: 19398983,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"!",
        cmd_func: ex_bang,
        cmd_preview_func: None,
        cmd_argt: 17301583,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"#",
        cmd_func: ex_print,
        cmd_preview_func: None,
        cmd_argt: 19400001,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"&",
        cmd_func: ex_substitute,
        cmd_preview_func: None,
        cmd_argt: 18350149,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"<",
        cmd_func: ex_operators,
        cmd_preview_func: None,
        cmd_argt: 20448577,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"=",
        cmd_func: ex_equal,
        cmd_preview_func: None,
        cmd_argt: 17432613,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c">",
        cmd_func: ex_operators,
        cmd_preview_func: None,
        cmd_argt: 20448577,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"@",
        cmd_func: ex_at,
        cmd_preview_func: None,
        cmd_argt: 17301829,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"~",
        cmd_func: ex_substitute,
        cmd_preview_func: None,
        cmd_argt: 18350149,
        cmd_addr_type: ADDR_LINES,
    },
    CommandDefinition {
        cmd_name: c"Next",
        cmd_func: ex_previous,
        cmd_preview_func: None,
        cmd_argt: 148743,
        cmd_addr_type: ADDR_OTHER,
    },
];

const CMDIDXS1: [u16; 26] = [
    0, 20, 45, 111, 135, 157, 173, 179, 187, 206, 208, 213, 276, 294, 311, 322, 363, 366, 388, 453,
    498, 511, 529, 544, 553, 554,
];

const CMDIDXS2: [[u8; 26]; 26] = [
    [
        0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 5, 6, 0, 0, 0, 7, 16, 0, 17, 0, 0, 0, 0, 0,
    ],
    [
        2, 0, 5, 7, 8, 9, 0, 0, 0, 0, 0, 10, 11, 12, 13, 14, 0, 15, 0, 0, 0, 0, 24, 0, 0, 0,
    ],
    [
        3, 12, 16, 18, 20, 22, 25, 0, 0, 0, 0, 34, 38, 41, 47, 58, 60, 61, 0, 0, 62, 0, 65, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 8, 17, 0, 18, 0, 0, 19, 0, 0, 21, 22, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 9, 10, 0, 0, 0, 0, 0, 0, 0, 16, 0, 17, 0, 0,
    ],
    [
        0, 0, 15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0, 4, 5, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        1, 0, 0, 0, 0, 3, 0, 0, 0, 4, 0, 5, 6, 0, 0, 13, 0, 0, 14, 0, 16, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        3, 11, 15, 18, 19, 23, 26, 31, 0, 0, 0, 33, 36, 39, 43, 50, 0, 52, 61, 53, 54, 58, 60, 0,
        0, 0,
    ],
    [
        1, 0, 0, 0, 7, 0, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 5, 7, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 4, 0, 7, 0, 0, 0, 0, 8, 0, 10, 0, 0, 0,
    ],
    [
        1, 5, 6, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 11, 13, 0, 0, 18, 19, 28, 0, 29, 0, 30, 0,
    ],
    [
        2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 15, 0, 16, 21, 0, 0, 0, 0,
    ],
    [
        2, 6, 15, 0, 17, 21, 0, 0, 23, 0, 0, 26, 28, 32, 36, 38, 0, 47, 0, 48, 0, 60, 61, 0, 62, 0,
    ],
    [
        4, 0, 1, 0, 24, 25, 0, 26, 0, 27, 0, 28, 32, 35, 37, 38, 0, 39, 42, 0, 43, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 9, 12, 0, 0, 0, 0, 15, 0, 16, 0, 0, 0, 0, 0,
    ],
    [
        2, 0, 0, 0, 0, 0, 0, 3, 4, 0, 0, 0, 0, 8, 0, 9, 10, 0, 12, 0, 13, 14, 0, 0, 0, 0,
    ],
    [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 5, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
];

#[inline]
fn sourcing_lnum_ptr() -> *mut linenr_T {
    unsafe {
        let es = &raw mut rt_st().exestack;
        &raw mut (*es).last_mut().unwrap().es_lnum
    }
}

fn save_dbg_stuff(dsp: &mut dbg_stuff) {
    unsafe {
        dsp.trylevel = g_st().trylevel;
        g_st().trylevel = 0;
        dsp.force_abort = c_int::from(g_st().force_abort);
        g_st().force_abort = false;
        let stack = &raw mut g_st().caught_stack;
        dsp.caught_stack = std::mem::take(&mut *stack);
        dsp.vv_exception = v_exception(None);
        dsp.vv_throwpoint = v_throwpoint(None);

        // Necessary for debugging an inactive ":catch", ":finally", ":endtry".
        dsp.did_emsg = g_st().did_emsg;
        g_st().did_emsg = 0;
        dsp.got_int = c_int::from(g_st().got_int);
        g_st().got_int = false;
        dsp.did_throw = g_st().did_throw;
        g_st().did_throw = false;
        dsp.need_rethrow = c_int::from(g_st().need_rethrow);
        g_st().need_rethrow = false;
        dsp.check_cstack = c_int::from(g_st().check_cstack);
        g_st().check_cstack = false;
        let ce = &raw mut g_st().current_exception;
        dsp.current_exception = (*ce).take();
    }
}

fn restore_dbg_stuff(dsp: &mut dbg_stuff) {
    unsafe {
        g_st().suppress_errthrow = false;
        g_st().trylevel = dsp.trylevel;
        g_st().force_abort = dsp.force_abort != 0;
        let stack = &raw mut g_st().caught_stack;
        *stack = std::mem::take(&mut dsp.caught_stack);
        v_exception(dsp.vv_exception.take());
        v_throwpoint(dsp.vv_throwpoint.take());
        g_st().did_emsg = dsp.did_emsg;
        g_st().got_int = dsp.got_int != 0;
        g_st().did_throw = dsp.did_throw;
        g_st().need_rethrow = dsp.need_rethrow != 0;
        g_st().check_cstack = dsp.check_cstack != 0;
        g_st().current_exception = dsp.current_exception.take();
    }
}

/// Check if ffname differs from fnum.
fn is_other_file(fnum: c_int, ffname: *mut c_char) -> bool {
    unsafe {
        if fnum != 0 {
            if fnum == (*g_st().curbuf).b_fnum {
                return false;
            }
            return true;
        }

        if ffname.is_null() {
            return true;
        }

        if *ffname == NUL {
            return false;
        }

        if !(*g_st().curbuf).file_id_valid && !(*g_st().curbuf).b_sfname.is_null() && *(*g_st().curbuf).b_sfname != NUL {
            return !path_equal(ffname, (*g_st().curbuf).b_sfname, kPathCmpLiteral);
        }

        otherfile(ffname)
    }
}

/// Print the executed command for when 'verbose' is set.
fn msg_verbose_cmd(lnum: linenr_T, cmd: *mut c_char) {
    g_st().no_wait_return += 1;
        verbose_enter_scroll();

        if lnum == 0 {
            let _ = smsg!(0, gettext(c"Executing: %s"), cmd);
        } else {
            let _ = smsg!(0, gettext(c"line %d: %s"), lnum as c_int, cmd);
        }
        if g_st().msg_silent == 0 {
            msg_puts(c"\n");
        }

        verbose_leave_scroll();
        g_st().no_wait_return -= 1;
}

/// Start executing an Ex command line.
fn do_cmdline_start() -> bool {
    debug_assert!(docmd_st().cmdline_call_depth >= 0);
        if docmd_st().cmdline_call_depth >= 200 && OptInt::from(docmd_st().cmdline_call_depth) >= opt_vars().p_mfd {
            return false;
        }
        docmd_st().cmdline_call_depth += 1;
        start_batch_changes();
        true
}

/// End executing an Ex command line.
fn do_cmdline_end() {
    docmd_st().cmdline_call_depth -= 1;
        debug_assert!(docmd_st().cmdline_call_depth >= 0);
        end_batch_changes();
}

/// Execute a simple command line.  Used for translated commands like "*".
pub fn do_cmdline_cmd<S: crate::strings::IntoCCharPtr>(cmd: S) -> bool {
    let cmd = cmd.into_c_char_ptr();
    do_cmdline(
            cmd.cast_mut(),
            LineGetter::None,
            DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_KEYTYPED,
        )
}

/// do_cmdline(): execute one Ex command line
pub fn do_cmdline(cmdline: *mut c_char, fgetline: LineGetter, flags: c_int) -> bool {
    static RECURSIVE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    unsafe {
        let mut next_cmdline: *mut c_char;
        let mut cmdline_buf: Vec<c_char> = Vec::new();
        let mut cmdline_copy: *mut c_char = ptr::null_mut();
        let mut used_getline = false;
        let mut msg_didout_before_start = false;
        let mut count: c_int = 0;
        let mut did_inc = false;
        let mut did_block = false;
        let mut retval = true;
        let mut cstack = cstack_T::default();
        let mut current_line: c_int = 0;
        let mut fname: *mut c_char = ptr::null_mut();
        let mut breakpoint: *mut linenr_T = ptr::null_mut();
        let mut dbg_tick: *mut c_int = ptr::null_mut();
        let mut debug_saved = dbg_stuff::new();

        let mut cmd_getline: LineGetter;
        let mut cmd_loop_cookie = loop_cookie::new();

        let ml = &raw mut g_st().msg_list;
        let saved_msg_list = (*ml).replace(Vec::new());

        if !do_cmdline_start() {
            let _ = emsg(gettext(e_command_too_recursive));
            do_errthrow(None, ptr::null_mut());
            g_st().msg_list = saved_msg_list;
            return false;
        }

        let func_cookie = fgetline.as_func();
        let source_cookie = fgetline.as_source();

        if let Some(fcp) = func_cookie {
            if g_st().ex_nesting_level == func_level(&mut *fcp) {
                g_st().ex_nesting_level += 1;
            }
            fname = func_name(&mut *fcp);
            breakpoint = func_breakpoint(&mut *fcp);
            dbg_tick = func_dbg_tick(&mut *fcp);
        } else if let Some(sp) = source_cookie {
            fname = sourcing_name();
            breakpoint = source_breakpoint(&mut *sp);
            dbg_tick = source_dbg_tick(&mut *sp);
        }

        if RECURSIVE.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            g_st().force_abort = false;
            g_st().suppress_errthrow = false;
        }

        if (flags & DOCMD_EXCRESET) != 0 {
            save_dbg_stuff(&mut debug_saved);
        }

        let initial_trylevel = g_st().trylevel;

        g_st().did_throw = false;
        g_st().did_emsg = 0;

        if (flags & DOCMD_KEYTYPED) == 0 && !fgetline.is_exline() {
            g_st().KeyTyped = false;
        }

        next_cmdline = cmdline;
        loop {
            let func_cookie = fgetline.as_func();

            if next_cmdline.is_null()
                && !g_st().force_abort
                && cstack.cs_idx < 0
                && !func_cookie.is_some_and(|fcp| func_has_abort(&mut *fcp) != 0)
            {
                g_st().did_emsg = 0;
            }

            if cstack.cs_looplevel > 0 && (current_line as usize) < cmd_loop_cookie.lines.len() {
                cmdline_buf.clear();
                cmdline_copy = ptr::null_mut();

                if let Some(fcp) = func_cookie {
                    if g_st().do_profiling == PROF_YES {
                        func_line_end(&mut *fcp);
                    }
                    if func_has_ended(&mut *fcp) != 0 {
                        retval = false;
                        break;
                    }
                } else if g_st().do_profiling == PROF_YES && fgetline.is_source() {
                    script_line_end();
                }

                if source_finished(fgetline) {
                    retval = false;
                    break;
                }

                if !breakpoint.is_null() && !dbg_tick.is_null() && *dbg_tick != g_st().debug_tick {
                    *breakpoint =
                        dbg_find_breakpoint(fgetline.is_source(), fname, *sourcing_lnum_ptr());
                    *dbg_tick = g_st().debug_tick;
                }

                let wp = &cmd_loop_cookie.lines[current_line as usize];
                next_cmdline = wp.line.as_ptr().cast_mut();
                *sourcing_lnum_ptr() = wp.lnum;

                if !breakpoint.is_null() && *breakpoint != 0 && *breakpoint <= *sourcing_lnum_ptr()
                {
                    dbg_breakpoint(fname, *sourcing_lnum_ptr());
                    *breakpoint =
                        dbg_find_breakpoint(fgetline.is_source(), fname, *sourcing_lnum_ptr());
                    *dbg_tick = g_st().debug_tick;
                }
                if g_st().do_profiling == PROF_YES {
                    if let Some(fcp) = func_cookie {
                        func_line_start(&mut *fcp);
                    } else if fgetline.is_source() {
                        script_line_start();
                    }
                }
            }

            if next_cmdline.is_null() {
                let indent = if cstack.cs_idx < 0 {
                    0
                } else {
                    (cstack.cs_idx + 1) * 2
                };

                if count == 1 && fgetline.is_exline() {
                    if ui_has(kUICmdline) {
                        let last_cmd_ptr = &raw const g_st().last_cmdline;
                        ui_ext_cmdline_block_append(
                            0,
                            (*last_cmd_ptr).as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                        );
                        did_block = true;
                    }
                    g_st().msg_didout = true;
                }

                if fgetline.is_none() {
                    if g_st().KeyTyped && (flags & DOCMD_REPEAT) == 0 {
                        g_st().need_wait_return = false;
                    }
                    retval = false;
                    break;
                }
                let Some(next_cstr) = fgetline.call(b':' as c_int, indent, true) else {
                    if g_st().KeyTyped && (flags & DOCMD_REPEAT) == 0 {
                        g_st().need_wait_return = false;
                    }
                    retval = false;
                    break;
                };
                let bytes = next_cstr.as_bytes_with_nul();
                cmdline_buf.clear();
                cmdline_buf.extend_from_slice(std::slice::from_raw_parts(
                    bytes.as_ptr().cast::<c_char>(),
                    bytes.len(),
                ));
                next_cmdline = cmdline_buf.as_mut_ptr();
                used_getline = true;

                if ui_has(kUICmdline) && count > 0 && fgetline.is_exline() {
                    ui_ext_cmdline_block_append(indent as usize, next_cmdline);
                }

                if (flags & DOCMD_KEEPLINE) != 0 {
                    if count == 0 {
                        g_st().repeat_cmdline = Some(next_cstr);
                    } else {
                        g_st().repeat_cmdline = None;
                    }
                }
            } else if cmdline_copy.is_null() {
                let bytes = CStr::from_ptr(next_cmdline).to_bytes_with_nul();
                cmdline_buf.clear();
                cmdline_buf.extend_from_slice(std::slice::from_raw_parts(
                    bytes.as_ptr().cast::<c_char>(),
                    bytes.len(),
                ));
                next_cmdline = cmdline_buf.as_mut_ptr();
            }
            cmdline_copy = next_cmdline;

            let mut current_line_before: c_int = 0;
            let using_loop_cookie = cstack.cs_looplevel > 0 || has_loop_cmd(next_cmdline);
            if using_loop_cookie {
                cmd_loop_cookie.current_line = current_line;
                cmd_loop_cookie.lc_getline = fgetline;
                cmd_loop_cookie.repeating =
                    c_int::from((current_line as usize) < cmd_loop_cookie.lines.len());
                cmd_getline = LineGetter::Loop(&raw mut cmd_loop_cookie);

                if (current_line as usize) == cmd_loop_cookie.lines.len() {
                    store_loop_line(&mut cmd_loop_cookie.lines, next_cmdline);
                }
                current_line_before = current_line;
            } else {
                cmd_getline = fgetline;
            }

            g_st().did_endif = false;

            let prev_count = count;
            count += 1;
            if prev_count == 0
                && (flags & DOCMD_NOWAIT) == 0
                && RECURSIVE.load(std::sync::atomic::Ordering::Relaxed) == 0
            {
                msg_didout_before_start = g_st().msg_didout;
                g_st().msg_didany = false;
                msg_start();
                g_st().msg_scroll = c_int::from(true);
                g_st().no_wait_return += 1;
                g_st().RedrawingDisabled += 1;
                did_inc = true;
            }

            if (opt_vars().p_verbose >= 15 && !sourcing_name().is_null()) || opt_vars().p_verbose >= 16 {
                msg_verbose_cmd(*sourcing_lnum_ptr(), cmdline_copy);
            }

            RECURSIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            next_cmdline = do_one_cmd(
                &raw mut cmdline_copy,
                &raw mut cmdline_buf,
                flags,
                &mut cstack,
                cmd_getline,
            );
            RECURSIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

            if using_loop_cookie {
                current_line = cmd_loop_cookie.current_line;
            }

            if next_cmdline.is_null() {
                cmdline_buf.clear();
                cmdline_copy = ptr::null_mut();

                let new_last_ptr = &raw mut g_st().new_last_cmdline;
                if fgetline.is_exline()
                    && let Some(new_cmd) = (*new_last_ptr).take()
                {
                    g_st().last_cmdline = Some(new_cmd);
                }
            } else {
                ptr::copy(next_cmdline, cmdline_copy, CStr::from_ptr(next_cmdline).to_bytes_with_nul().len());
                next_cmdline = cmdline_copy;
            }

            if g_st().did_emsg != 0
                && !g_st().force_abort
                && fgetline
                    .as_func()
                    .is_some_and(|fcp| func_has_abort(&mut *fcp) == 0)
            {
                g_st().did_emsg = 0;
            }

            if cstack.cs_looplevel > 0 {
                current_line += 1;

                if (cstack.cs_lflags & (CSL_HAD_CONT | CSL_HAD_ENDLOOP)) != 0 {
                    cstack.cs_lflags &= !(CSL_HAD_CONT | CSL_HAD_ENDLOOP);

                    if g_st().did_emsg == 0
                        && !g_st().got_int
                        && !g_st().did_throw
                        && cstack.cs_idx >= 0
                        && (cstack.cs_flags[cstack.cs_idx as usize] & (CSF_WHILE | CSF_FOR)) != 0
                        && cstack.cs_line[cstack.cs_idx as usize] >= 0
                        && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_ACTIVE) != 0
                    {
                        current_line = cstack.cs_line[cstack.cs_idx as usize];
                        cstack.cs_lflags |= CSL_HAD_LOOP;
                        line_breakcheck();

                        if !breakpoint.is_null()
                            && cmd_loop_cookie.lines.len() > current_line as usize
                        {
                            let wp = &cmd_loop_cookie.lines[current_line as usize];
                            *breakpoint =
                                dbg_find_breakpoint(fgetline.is_source(), fname, wp.lnum - 1);
                            *dbg_tick = g_st().debug_tick;
                        }
                    } else if cstack.cs_idx >= 0 {
                        let prev_idx = cstack.cs_idx - 1;
                        let mut looplevel = cstack.cs_looplevel;
                        rewind_conditionals(
                            &mut cstack,
                            prev_idx,
                            CSF_WHILE | CSF_FOR,
                            &mut looplevel,
                        );
                        cstack.cs_looplevel = looplevel;
                    }
                } else if (cstack.cs_lflags & CSL_HAD_LOOP) != 0 {
                    cstack.cs_lflags &= !CSL_HAD_LOOP;
                    cstack.cs_line[cstack.cs_idx as usize] = current_line_before;
                }
            }

            if cstack.cs_looplevel == 0 {
                if let Some(wp) = cmd_loop_cookie.lines.last() {
                    *sourcing_lnum_ptr() = wp.lnum;
                    cmd_loop_cookie.lines.clear();
                }
                current_line = 0;
            }

            if (cstack.cs_lflags & CSL_HAD_FINA) != 0 {
                cstack.cs_lflags &= !CSL_HAD_FINA;
                let ce = &raw const g_st().current_exception;
                report_make_pending(
                    c_int::from(cstack.cs_pending[cstack.cs_idx as usize])
                        & (CSTP_ERROR | CSTP_INTERRUPT | CSTP_THROW),
                    if g_st().did_throw {
                        PendingVal::Throw((*ce).clone())
                    } else {
                        PendingVal::None
                    },
                );
                g_st().did_emsg = 0;
                g_st().got_int = false;
                g_st().did_throw = false;
                cstack.cs_flags[cstack.cs_idx as usize] |= CSF_ACTIVE | CSF_FINALLY;
            }

            g_st().trylevel = initial_trylevel + cstack.cs_trylevel;

            if g_st().trylevel == 0 && g_st().did_emsg == 0 && !g_st().got_int && !g_st().did_throw {
                g_st().force_abort = false;
            }

            do_intthrow(&raw mut cstack);

            let keep_going = !((g_st().got_int || (g_st().did_emsg != 0 && g_st().force_abort) || g_st().did_throw)
                && cstack.cs_trylevel == 0)
                && !(g_st().did_emsg != 0
                    && (cstack.cs_trylevel == 0 || g_st().did_emsg_syntax)
                    && used_getline
                    && fgetline.is_exline())
                && (!next_cmdline.is_null() || cstack.cs_idx >= 0 || (flags & DOCMD_REPEAT) != 0);
            if !keep_going {
                break;
            }
        }

        drop(cmdline_buf);
        g_st().did_emsg_syntax = false;
        drop(cmd_loop_cookie);

        if cstack.cs_idx >= 0 {
            if !g_st().got_int
                && !g_st().did_throw
                && !aborting()
                && ((fgetline.is_source() && !source_finished(fgetline))
                    || fgetline
                        .as_func()
                        .is_some_and(|fcp| func_has_ended(&mut *fcp) == 0))
            {
                let top_flags = cstack.cs_flags[cstack.cs_idx as usize];
                if (top_flags & CSF_TRY) != 0 {
                    let _ = emsg(gettext(e_endtry));
                } else if (top_flags & CSF_WHILE) != 0 {
                    let _ = emsg(gettext(e_endwhile));
                } else if (top_flags & CSF_FOR) != 0 {
                    let _ = emsg(gettext(e_endfor));
                } else {
                    let _ = emsg(gettext(e_endif));
                }
            }

            loop {
                let mut idx = cleanup_conditionals(&mut cstack, 0, c_int::from(true));
                if idx >= 0 {
                    idx -= 1;
                }
                let mut looplevel = cstack.cs_looplevel;
                rewind_conditionals(
                    &mut cstack,
                    idx,
                    CSF_WHILE | CSF_FOR,
                    &mut looplevel,
                );
                cstack.cs_looplevel = looplevel;
                if cstack.cs_idx < 0 {
                    break;
                }
            }
            g_st().trylevel = initial_trylevel;
        }

        do_errthrow(
            Some(&mut cstack),
            if fgetline.is_func() {
                c"endfunction".as_ptr().cast_mut()
            } else {
                ptr::null_mut()
            },
        );

        if g_st().trylevel == 0 {
            if g_st().did_throw {
                handle_did_throw();
            } else if g_st().got_int || (g_st().did_emsg != 0 && g_st().force_abort) {
                g_st().suppress_errthrow = true;
            }
        }

        if g_st().did_throw {
            g_st().need_rethrow = true;
        }
        if fgetline
            .as_source()
            .is_some_and(|sp| g_st().ex_nesting_level > source_level(&mut *sp))
            || fgetline
                .as_func()
                .is_some_and(|fcp| g_st().ex_nesting_level > func_level(&mut *fcp) + 1)
        {
            if !g_st().did_throw {
                g_st().check_cstack = true;
            }
        } else {
            if fgetline.is_func() {
                g_st().ex_nesting_level -= 1;
            }
            if (fgetline.is_source() || fgetline.is_func()) && g_st().ex_nesting_level < g_st().debug_break_level
            {
                do_debug(if fgetline.is_source() {
                    gettext(c"End of sourced file")
                } else {
                    gettext(c"End of function")
                });
            }
        }

        if (flags & DOCMD_EXCRESET) != 0 {
            restore_dbg_stuff(&mut debug_saved);
        }

        g_st().msg_list = saved_msg_list;

        cstack.cs_emsg_silent_list.clear();

        if did_inc {
            g_st().RedrawingDisabled -= 1;
            g_st().no_wait_return -= 1;
            g_st().msg_scroll = 0;

            if !retval || (g_st().did_endif && g_st().KeyTyped && g_st().did_emsg == 0) {
                g_st().need_wait_return = false;
                g_st().msg_didany = false;
            } else if g_st().need_wait_return {
                g_st().msg_didout |= msg_didout_before_start;
                wait_return(0);
            }
        }

        if did_block {
            ui_ext_cmdline_block_leave();
        }

        g_st().did_endif = false;

        do_cmdline_end();
        retval
    }
}

/// Handle when "did_throw" is set after executing commands.
pub fn handle_did_throw() {
    unsafe {
        let ce = &raw const g_st().current_exception;
        let cur_excp = (*ce).clone();
        debug_assert!(cur_excp.is_some());
        let excp = cur_excp.as_ref().unwrap();
        let mut p: Option<std::ffi::CString> = None;
        let mut messages: Vec<msglist_T> = Vec::new();

        match excp.r#type {
            ET_USER => {
                let _ = vim_snprintf!(
                    (&raw mut g_st().IObuff).cast(),
                    IOSIZE,
                    gettext(c"E605: Exception not caught: %s"),
                    excp.value.as_ptr(),
                );
                p = Some(cstr_to_cstring((&raw const g_st().IObuff).cast()));
            }
            ET_ERROR => {
                messages = excp.messages.clone();
            }
            ET_INTERRUPT => {}
            _ => {}
        }

        estack_push(
            ETYPE_EXCEPT,
            excp.throw_name.as_ptr().cast_mut(),
            excp.throw_lnum,
        );
        drop(cur_excp);

        discard_current_exception();

        if g_st().emsg_silent == 0 {
            g_st().suppress_errthrow = true;
            g_st().force_abort = true;
        }

        if !messages.is_empty() {
            for msg in messages {
                let _ = emsg_multiline(
                    msg.msg.as_ptr().cast_mut(),
                    c"emsg".as_ptr(),
                    HLF_E,
                    msg.multiline,
                );
            }
        } else if let Some(ref p) = p {
            let _ = emsg(p.as_ptr());
        }
        estack_pop();
    }
}

/// Obtain a line when inside a ":while" or ":for" loop.
pub fn get_loop_line(
    c: c_int,
    cp: &mut loop_cookie,
    indent: c_int,
    do_concat: bool,
) -> Option<std::ffi::CString> {
    unsafe {
        let lines = &mut cp.lines;

        if (cp.current_line + 1) as usize >= lines.len() {
            if cp.repeating != 0 {
                return None;
            }
            let line = if !cp.lc_getline.is_none() {
                cp.lc_getline.call(c, indent, do_concat)
            } else {
                getcmdline(c, 0, indent, do_concat)
            };
            if let Some(ref l) = line {
                store_loop_line(lines, l.as_ptr());
                cp.current_line += 1;
            }
            return line;
        }

        g_st().KeyTyped = false;
        cp.current_line += 1;
        let wp = &lines[cp.current_line as usize];
        *sourcing_lnum_ptr() = wp.lnum;
        Some(wp.line.clone())
    }
}

/// Store a line in "gap" so that a ":while" loop can execute it again.
fn store_loop_line(lines: &mut Vec<wcmd_T>, line: *const c_char) {
    unsafe {
        lines.push(wcmd_T {
            line: crate::memory::cstr_to_cstring(line),
            lnum: *sourcing_lnum_ptr(),
        });
    }
}

fn compute_buffer_local_count(
    addr_type: cmd_addr_T,
    lnum: linenr_T,
    offset: c_int,
) -> c_int {
    unsafe {
        let mut count = offset;
        let mut buf = g_st().firstbuf;
        while !(*buf).b_next.is_null() && (*buf).b_fnum < lnum {
            buf = (*buf).b_next;
        }
        while count != 0 {
            count += if count < 0 { 1 } else { -1 };
            let mut nextbuf = if offset < 0 {
                (*buf).b_prev
            } else {
                (*buf).b_next
            };
            if nextbuf.is_null() {
                break;
            }
            buf = nextbuf;
            if addr_type == ADDR_LOADED_BUFFERS {
                while (*buf).b_ml.ml_mfp.is_none() {
                    nextbuf = if offset < 0 {
                        (*buf).b_prev
                    } else {
                        (*buf).b_next
                    };
                    if nextbuf.is_null() {
                        break;
                    }
                    buf = nextbuf;
                }
            }
        }
        if addr_type == ADDR_LOADED_BUFFERS {
            while (*buf).b_ml.ml_mfp.is_none() {
                let nextbuf = if offset >= 0 {
                    (*buf).b_prev
                } else {
                    (*buf).b_next
                };
                if nextbuf.is_null() {
                    break;
                }
                buf = nextbuf;
            }
        }
        (*buf).b_fnum
    }
}

fn current_win_nr(win: *const win_T,cmdarg: Option<&exarg_T>) -> c_int {
    unsafe {
        let nonnr = cmdarg.is_some() && cmdarg.unwrap().cmdidx != cmdidx_T::CMD_quit as c_int;
        let mut nr = 0;

        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            let include_this = nonnr || win_has_winnr(&mut *wp, g_st().curtab);
            nr += c_int::from(include_this);
            if ptr::eq(wp, win) {
                if !include_this {
                    return 0;
                }
                break;
            }
            wp = (*wp).w_next;
        }
        nr
    }
}

fn current_tab_nr(tab: *mut tabpage_T) -> c_int {
    unsafe {
        let mut nr = 0;
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            nr += 1;
            if tp == tab {
                break;
            }
            tp = (*tp).tp_next;
        }
        nr
    }
}

fn get_wincmd_addr_type(arg: *const c_char, eap: &mut exarg_T) {
    unsafe {
        let ch = *arg;
        match ch {
            c if c == b'S' as c_char
                || c == Ctrl_S
                || c == b's' as c_char
                || c == Ctrl_N
                || c == b'n' as c_char
                || c == b'j' as c_char
                || c == Ctrl_J
                || c == b'k' as c_char
                || c == Ctrl_K
                || c == b'T' as c_char
                || c == Ctrl_R
                || c == b'r' as c_char
                || c == b'R' as c_char
                || c == b'K' as c_char
                || c == b'J' as c_char
                || c == b'+' as c_char
                || c == b'-' as c_char
                || c == Ctrl__
                || c == b'_' as c_char
                || c == b'|' as c_char
                || c == b']' as c_char
                || c == Ctrl_RSB
                || c == b'g' as c_char
                || c == Ctrl_G
                || c == Ctrl_V
                || c == b'v' as c_char
                || c == b'h' as c_char
                || c == Ctrl_H
                || c == b'l' as c_char
                || c == Ctrl_L
                || c == b'H' as c_char
                || c == b'L' as c_char
                || c == b'>' as c_char
                || c == b'<' as c_char
                || c == b'}' as c_char
                || c == b'f' as c_char
                || c == b'F' as c_char
                || c == Ctrl_F
                || c == b'i' as c_char
                || c == Ctrl_I
                || c == b'd' as c_char
                || c == Ctrl_D =>
            {
                eap.addr_type = ADDR_OTHER;
            }
            c if c == Ctrl_HAT || c == b'^' as c_char => {
                eap.addr_type = ADDR_BUFFERS;
            }
            c if c == Ctrl_Q
                || c == b'q' as c_char
                || c == Ctrl_C
                || c == b'c' as c_char
                || c == Ctrl_O
                || c == b'o' as c_char
                || c == Ctrl_W
                || c == b'w' as c_char
                || c == b'W' as c_char
                || c == b'x' as c_char
                || c == Ctrl_X =>
            {
                eap.addr_type = ADDR_WINDOWS;
            }
            c if c == Ctrl_Z
                || c == b'z' as c_char
                || c == b'P' as c_char
                || c == b't' as c_char
                || c == Ctrl_T
                || c == b'b' as c_char
                || c == Ctrl_B
                || c == b'p' as c_char
                || c == Ctrl_P
                || c == b'=' as c_char
                || c == CAR =>
            {
                eap.addr_type = ADDR_NONE;
            }
            _ => {}
        }
    }
}

fn skip_colon_white(mut p: *const c_char, skipleadingwhite: bool) -> *mut c_char {
    unsafe {
        if skipleadingwhite {
            p = skipwhite(p);
        }
        while *p == b':' as c_char {
            p = skipwhite(p.add(1));
        }
        p.cast_mut()
    }
}

pub fn set_cmd_addr_type(eap: &mut exarg_T, p: *mut c_char) {
    {
        if is_user_cmdidx(cmdidx_T::from_raw(eap.cmdidx)) {
            return;
        }
        if eap.cmdidx != CMD_SIZE_I32 {
            eap.addr_type = CMDNAMES[eap.cmdidx as usize].cmd_addr_type;
        } else {
            eap.addr_type = ADDR_LINES;
        }
        if eap.cmdidx == cmdidx_T::CMD_wincmd as c_int && !p.is_null() {
            get_wincmd_addr_type(skipwhite(p), eap);
        }
        if (eap.cmdidx == cmdidx_T::CMD_cc as c_int
            || eap.cmdidx == cmdidx_T::CMD_ll as c_int)
            && bt_quickfix(g_st().curbuf)
        {
            eap.addr_type = ADDR_OTHER;
        }
    }
}

pub fn get_cmd_default_range(eap: &mut exarg_T) -> linenr_T {
    unsafe {
        match eap.addr_type {
            ADDR_LINES | ADDR_OTHER => (**curwin.as_ptr()).w_cursor.lnum.min((*g_st().curbuf).b_ml.ml_line_count),
            ADDR_WINDOWS => current_win_nr(*curwin.as_ptr(), None),
            ADDR_ARGUMENTS => ((**curwin.as_ptr()).w_arg_idx + 1).min(argcount()),
            ADDR_LOADED_BUFFERS | ADDR_BUFFERS => (*g_st().curbuf).b_fnum,
            ADDR_TABS => current_tab_nr(g_st().curtab),
            ADDR_TABS_RELATIVE | ADDR_UNSIGNED => 1,
            ADDR_QUICKFIX => qf_get_cur_idx(eap) as linenr_T,
            ADDR_QUICKFIX_VALID => qf_get_cur_valid_idx(eap),
            _ => 0,
        }
    }
}

pub fn set_cmd_dflall_range(eap: &mut exarg_T) {
    unsafe {
        eap.line1 = 1;
        match eap.addr_type {
            ADDR_LINES | ADDR_OTHER => {
                eap.line2 = (*g_st().curbuf).b_ml.ml_line_count;
            }
            ADDR_LOADED_BUFFERS => {
                let mut buf = g_st().firstbuf;
                while !(*buf).b_next.is_null() && (*buf).b_ml.ml_mfp.is_none() {
                    buf = (*buf).b_next;
                }
                eap.line1 = (*buf).b_fnum;
                buf = g_st().lastbuf;
                while !(*buf).b_prev.is_null() && (*buf).b_ml.ml_mfp.is_none() {
                    buf = (*buf).b_prev;
                }
                eap.line2 = (*buf).b_fnum;
            }
            ADDR_BUFFERS => {
                eap.line1 = (*g_st().firstbuf).b_fnum;
                eap.line2 = (*g_st().lastbuf).b_fnum;
            }
            ADDR_WINDOWS => {
                eap.line2 = current_win_nr(ptr::null(), Some(&*eap));
            }
            ADDR_TABS => {
                eap.line2 = current_tab_nr(ptr::null_mut());
            }
            ADDR_TABS_RELATIVE => {
                eap.line2 = 1;
            }
            ADDR_ARGUMENTS => {
                let ac = argcount();
                if ac == 0 {
                    eap.line1 = 0;
                    eap.line2 = 0;
                } else {
                    eap.line2 = ac;
                }
            }
            ADDR_QUICKFIX_VALID => {
                eap.line2 = qf_get_valid_size(eap) as linenr_T;
                if eap.line2 == 0 {
                    eap.line2 = 1;
                }
            }
            _ => {
                iemsg(gettext(
                    c"INTERNAL: Cannot use EX_DFLALL with ADDR_NONE, ADDR_UNSIGNED or ADDR_QUICKFIX",
                ));
            }
        }
    }
}

fn parse_register(eap: &mut exarg_T) {
    unsafe {
        let is_user = eap.cmdidx < 0;
        if (eap.argt & EX_REGSTR) != 0
            && *eap.arg != NUL
            && (!is_user || *eap.arg != b'=' as c_char)
            && !((eap.argt & EX_COUNT) != 0 && ascii_isdigit(*eap.arg))
            && valid_yank_reg(
                c_int::from(*eap.arg as u8),
                !is_user
                    && eap.cmdidx != cmdidx_T::CMD_put as c_int
                    && eap.cmdidx != cmdidx_T::CMD_iput as c_int,
            )
        {
            eap.regname = c_int::from(*eap.arg as u8);
            eap.arg = eap.arg.add(1);
            if *eap.arg.sub(1) == b'=' as c_char && *eap.arg != NUL {
                if eap.skip == 0 {
                    set_expr_line(Some(crate::memory::cstr_to_cstring(eap.arg)));
                }
                eap.arg = eap.arg.add(crate::memory::cstr_len(eap.arg));
            }
            eap.arg = skipwhite(eap.arg);
        }
    }
}

pub fn set_cmd_count(eap: &mut exarg_T, count: linenr_T, validate: bool) {
    unsafe {
        if eap.addr_type != ADDR_LINES {
            eap.line2 = count;
            if eap.addr_count == 0 {
                eap.addr_count = 1;
            }
        } else {
            eap.line1 = eap.line2;
            if eap.line2 >= i32::MAX - (count - 1) {
                eap.line2 = i32::MAX;
            } else {
                eap.line2 += count - 1;
            }
            eap.addr_count += 1;
            if validate && eap.line2 > (*g_st().curbuf).b_ml.ml_line_count {
                eap.line2 = (*g_st().curbuf).b_ml.ml_line_count;
            }
        }
    }
}

fn parse_count(eap: &mut exarg_T, errormsg: *mut *const c_char, validate: bool) -> bool {
    unsafe {
        if (eap.argt & EX_COUNT) != 0 && ascii_isdigit(*eap.arg) {
            let p = skipdigits(eap.arg.add(1));
            if (eap.argt & EX_BUFNAME) == 0 || *p == NUL || ascii_iswhite(*p) {
                let n = getdigits_int32(&raw mut eap.arg, false, i32::MAX);
                eap.arg = skipwhite(eap.arg);

                if !eap.args.is_empty() {
                    let arg0 = eap.args.as_slice()[0];
                    let arglen0 = eap.arglens.as_slice()[0];
                    debug_assert!(eap.arg >= arg0);
                    if (eap.arg as usize) < (arg0 as usize) + arglen0 {
                        eap.arglens.as_mut_slice()[0] -= (eap.arg as usize) - (arg0 as usize);
                        eap.args.as_mut_slice()[0] = eap.arg;
                    } else {
                        shift_cmd_args(eap);
                    }
                }

                if n <= 0 && (eap.argt & EX_ZEROR) == 0 {
                    if !errormsg.is_null() {
                        *errormsg = gettext(e_zerocount);
                    }
                    return false;
                }
                set_cmd_count(eap, n, validate);
            }
        }
        true
    }
}

pub fn is_cmd_ni(cmdidx: cmdidx_T) -> bool {
    if is_user_cmdidx(cmdidx) {
        return false;
    }
    let func = CMDNAMES[cmdidx as usize].cmd_func;
    ptr::fn_addr_eq(func, ex_ni as ex_func_T) || ptr::fn_addr_eq(func, ex_script_ni as ex_func_T)
}

fn find_excmd_after_range(eap: &mut exarg_T) -> *mut c_char {
    let cmd = eap.cmd;
        eap.cmd = skip_range(eap.cmd, None);
        let p = find_ex_command(eap, None);
        eap.cmd = cmd;
        p
}

fn parse_bang(eap: &exarg_T, p: *mut *mut c_char) -> bool {
    unsafe {
        if **p == b'!' as c_char
            && eap.cmdidx != cmdidx_T::CMD_substitute as c_int
            && eap.cmdidx != cmdidx_T::CMD_smagic as c_int
            && eap.cmdidx != cmdidx_T::CMD_snomagic as c_int
        {
            *p = (*p).add(1);
            return true;
        }
        false
    }
}

pub fn cmd_has_expr_args(cmdidx: cmdidx_T) -> bool {
    matches!(
        cmdidx,
        cmdidx_T::CMD_execute
            | cmdidx_T::CMD_echo
            | cmdidx_T::CMD_echon
            | cmdidx_T::CMD_echomsg
            | cmdidx_T::CMD_echoerr
    )
}

pub fn parse_cmdline(
    cmdline: *mut *mut c_char,
    cmdline_buf: *mut Vec<c_char>,
    eap: &mut exarg_T,
    cmod: &mut cmdmod_T,
    errormsg: *mut *const c_char,
    skipmods: bool,
) -> bool {
    unsafe {
        let mut retval = false;
        let save_cursor = (**curwin.as_ptr()).w_cursor;
        save_last_search_pattern();

        *cmod = cmdmod_T::new();
        *eap = exarg_T::new();
        eap.line1 = 1;
        eap.line2 = 1;
        eap.cmd = *cmdline;
        eap.cmdlinep = cmdline;
        eap.cmdline_buf = cmdline_buf;

        let orig_cmd = eap.cmd;
        let result = if skipmods {
            true
        } else {
            parse_command_modifiers(eap, errormsg, cmod, false)
        };
        let after_modifier = eap.cmd;

        'end: {
            if !result && (!(*errormsg).is_null() || after_modifier == orig_cmd) {
                break 'end;
            }

            let mut p = find_excmd_after_range(eap);
            if p.is_null() {
                *errormsg = gettext(e_ambiguous_use_of_user_defined_command);
                break 'end;
            }

            set_cmd_addr_type(eap, p);
            if !parse_cmd_address(eap, errormsg, true) {
                break 'end;
            }

            eap.cmd = skip_colon_white(eap.cmd, true);
            if *eap.cmd == b'"' as c_char {
                break 'end;
            }
            if *eap.cmd == NUL && eap.addr_count == 0 && after_modifier == *cmdline {
                break 'end;
            }

            if *eap.cmd == NUL && eap.cmdidx == CMD_SIZE_I32 {
                eap.arg = eap.cmd;
                if eap.addr_count > 0 {
                    eap.argt = EX_RANGE;
                } else {
                    eap.argt = 0;
                    eap.addr_type = ADDR_NONE;
                }
                retval = true;
                break 'end;
            }

            if eap.cmdidx == CMD_SIZE_I32 {
                xstrlcpy(
                    (&raw mut g_st().IObuff).cast(),
                    gettext(e_not_an_editor_command),
                    IOSIZE,
                );
                let cmdname = if !after_modifier.is_null() {
                    after_modifier
                } else {
                    *cmdline
                };
                append_command(cmdname);
                *errormsg = (&raw const g_st().IObuff).cast();
                break 'end;
            }

            eap.forceit = c_int::from(parse_bang(eap, &raw mut p));

            if eap.cmdidx >= 0 {
                eap.argt = CMDNAMES[eap.cmdidx as usize].cmd_argt;
            }
            if skipmods {
                eap.arg = p.add(usize::from(*p == b' ' as c_char));
            } else {
                eap.arg = if eap.cmdidx == cmdidx_T::CMD_bang as c_int {
                    p
                } else {
                    skipwhite(p)
                };
            }

            if eap.cmdidx == cmdidx_T::CMD_read as c_int && eap.forceit != 0 {
                eap.forceit = 0;
            }

            if !skipmods && (eap.argt & EX_TRLBAR) != 0 {
                separate_nextcmd(&mut *eap);
            } else if !skipmods
                && eap.cmdidx >= 0
                && cmd_has_expr_args(cmdidx_T::from_raw(eap.cmdidx))
            {
                let mut arg = eap.arg;
                while *arg != NUL && *arg != b'|' as c_char && *arg != b'\n' as c_char {
                    let start = arg;
                    g_st().emsg_skip += 1;
                    skip_expr(&raw mut arg, None);
                    g_st().emsg_skip -= 1;
                    if arg == start {
                        arg = arg.add(1);
                    }
                }
                if *arg == b'|' as c_char || *arg == b'\n' as c_char {
                    eap.nextcmd = check_nextcmd(arg);
                    *arg = NUL;
                }
            }

            if (eap.argt & EX_BANG) == 0 && eap.forceit != 0 {
                *errormsg = gettext(e_nobang);
                break 'end;
            }
            if (eap.argt & EX_RANGE) == 0 && eap.addr_count > 0 {
                *errormsg = gettext(e_norange);
                break 'end;
            }
            if (eap.argt & EX_DFLALL) != 0 && eap.addr_count == 0 {
                set_cmd_dflall_range(eap);
            }

            if !skipmods {
                parse_register(eap);
                if !parse_count(eap, errormsg, false) {
                    break 'end;
                }
            }

            if !eap.nextcmd.is_null() {
                eap.nextcmd = skip_colon_white(eap.nextcmd, true);
            }

            eap.magic.file = (eap.argt & EX_XFILE) != 0;
            eap.magic.bar = (eap.argt & EX_TRLBAR) != 0;

            retval = true;
        }

        if !retval {
            undo_cmdmod(cmod);
        }
        (**curwin.as_ptr()).w_cursor = save_cursor;
        restore_last_search_pattern();
        retval
    }
}

fn shift_cmd_args(eap: &mut exarg_T) {
    unsafe {
        debug_assert!(!eap.args.is_empty());

        let oldarg0 = eap.args.remove(0);
        let oldarglen0 = eap.arglens.remove(0);

        eap.arg = if let Some(&first_arg) = eap.args.first() {
            first_arg
        } else {
            oldarg0.add(oldarglen0)
        };
    }
}

fn execute_cmd0(
    retv: &mut c_int,
    eap: &mut exarg_T,
    errormsg: *mut *const c_char,
    preview: bool,
) -> bool {
    unsafe {
        if (eap.argt & EX_XFILE) != 0 && !expand_filename(eap, eap.cmdlinep, errormsg)
        {
            return false;
        }

        if (eap.argt & EX_BUFNAME) != 0
            && *eap.arg != NUL
            && eap.addr_count == 0
            && eap.cmdidx >= 0
        {
            if eap.args.is_empty() {
                let mut p: *mut c_char;
                if eap.cmdidx == cmdidx_T::CMD_bdelete as c_int
                    || eap.cmdidx == cmdidx_T::CMD_bwipeout as c_int
                    || eap.cmdidx == cmdidx_T::CMD_bunload as c_int
                {
                    p = skiptowhite_esc(eap.arg);
                } else {
                    p = eap.arg.add(crate::memory::cstr_len(eap.arg));
                    while p > eap.arg && ascii_iswhite(*p.sub(1)) {
                        p = p.sub(1);
                    }
                }
                eap.line2 =
                    buflist_findpat(eap.arg, p, (eap.argt & EX_BUFUNL) != 0, false, false);
                eap.addr_count = 1;
                eap.arg = skipwhite(p);
            } else {
                let arg0 = eap.args.as_slice()[0];
                let arglen0 = eap.arglens.as_slice()[0];
                eap.line2 = buflist_findpat(
                    arg0,
                    arg0.add(arglen0),
                    (eap.argt & EX_BUFUNL) != 0,
                    false,
                    false,
                );
                eap.addr_count = 1;
                shift_cmd_args(&mut *eap);
            }
            if eap.line2 < 0 {
                return false;
            }
        }

        if eap.cmdidx == cmdidx_T::CMD_try as c_int && g_st().cmdmod.cmod_did_esilent > 0 {
            g_st().emsg_silent -= g_st().cmdmod.cmod_did_esilent;
            if g_st().emsg_silent < 0 {
                g_st().emsg_silent = 0;
            }
            g_st().cmdmod.cmod_did_esilent = 0;
        }

        if eap.cmdidx < 0 {
            *retv = do_ucmd(eap, preview);
        } else {
            eap.errmsg = ptr::null_mut();
            if preview {
                if let Some(preview_fn) = CMDNAMES[eap.cmdidx as usize].cmd_preview_func {
                    *retv = preview_fn(&mut *eap, cmdpreview_get_ns(), cmdpreview_get_bufnr());
                }
            } else {
                (CMDNAMES[eap.cmdidx as usize].cmd_func)(&mut *eap);
            }
            if !eap.errmsg.is_null() {
                *errormsg = eap.errmsg;
            }
        }

        true
    }
}

pub fn execute_cmd(eap: &mut exarg_T, cmod: &mut cmdmod_T, preview: bool) -> c_int {
    unsafe {
        let mut retv: c_int = 0;
        if !do_cmdline_start() {
            let _ = emsg(gettext(e_command_too_recursive));
            return retv;
        }

        let mut errormsg: *const c_char = ptr::null();
        let save_cmdmod = ptr::replace(&raw mut g_st().cmdmod, std::mem::take(&mut *cmod));

        apply_cmdmod(&mut g_st().cmdmod);

        'end: {
            if (*g_st().curbuf).b_p_ma == 0
                && (eap.argt & EX_MODIFY) != 0
                && !(!(*g_st().curbuf).terminal.is_null()
                    && (eap.cmdidx == cmdidx_T::CMD_put as c_int
                        || eap.cmdidx == cmdidx_T::CMD_iput as c_int))
            {
                errormsg = gettext(e_modifiable);
                break 'end;
            }
            if eap.cmdidx >= 0 && text_locked() && (eap.argt & EX_LOCK_OK) == 0 {
                errormsg = gettext(get_text_locked_msg());
                break 'end;
            }
            if (eap.argt & EX_BUFLOCK_OK) == 0
                && eap.cmdidx != cmdidx_T::CMD_checktime as c_int
                && eap.cmdidx != cmdidx_T::CMD_edit as c_int
                && !(eap.cmdidx == cmdidx_T::CMD_file as c_int && *eap.arg == NUL)
                && eap.cmdidx >= 0
                && curbuf_locked()
            {
                break 'end;
            }

            correct_range(eap);
            if eap.cmdidx == CMD_SIZE_I32 && eap.addr_count > 0 {
                errormsg = ex_range_without_command(eap);
                break 'end;
            }

            if ((eap.argt & EX_WHOLEFOLD) != 0 || eap.addr_count >= 2)
                && g_st().global_busy == 0
                && eap.addr_type == ADDR_LINES
            {
                hasFolding(*curwin.as_ptr(), eap.line1, Some(&mut eap.line1), ptr::null_mut());
                hasFolding(*curwin.as_ptr(), eap.line2, None, &raw mut eap.line2);
            }

            if !parse_count(&mut *eap, &mut errormsg, true) {
                break 'end;
            }

            let mut cstack = cstack_T::default();
            eap.cstack = (&raw mut cstack).cast();

            let _ = execute_cmd0(&mut retv, eap, &mut errormsg, preview);
        }

        if !errormsg.is_null() && *errormsg != NUL {
            let _ = emsg(errormsg);
        }

        undo_cmdmod(&mut g_st().cmdmod);
        g_st().cmdmod = save_cmdmod;

        do_cmdline_end();
        retv
    }
}

fn profile_cmd(eap: &exarg_T, cstack: &mut cstack_T, fgetline: LineGetter) {
    unsafe {
        if g_st().do_profiling == PROF_YES
            && (eap.skip == 0
                || cstack.cs_idx == 0
                || (cstack.cs_idx > 0
                    && (cstack.cs_flags[(cstack.cs_idx - 1) as usize] & CSF_ACTIVE) != 0))
        {
            let mut skip = g_st().did_emsg != 0 || g_st().got_int || g_st().did_throw;

            if eap.cmdidx == cmdidx_T::CMD_catch as c_int {
                skip = !skip
                    && !(cstack.cs_idx >= 0
                        && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_THROWN) != 0
                        && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_CAUGHT) == 0);
            } else if eap.cmdidx == cmdidx_T::CMD_else as c_int
                || eap.cmdidx == cmdidx_T::CMD_elseif as c_int
            {
                skip = skip
                    || !(cstack.cs_idx >= 0
                        && (cstack.cs_flags[cstack.cs_idx as usize]
                            & (CSF_ACTIVE | CSF_TRUE))
                            == 0);
            } else if eap.cmdidx == cmdidx_T::CMD_finally as c_int {
                skip = false;
            } else if eap.cmdidx != cmdidx_T::CMD_endif as c_int
                && eap.cmdidx != cmdidx_T::CMD_endfor as c_int
                && eap.cmdidx != cmdidx_T::CMD_endtry as c_int
                && eap.cmdidx != cmdidx_T::CMD_endwhile as c_int
            {
                skip = eap.skip != 0;
            }

            if !skip {
                if let Some(fcp) = fgetline.as_func() {
                    func_line_exec(&mut *fcp);
                } else if fgetline.is_source() {
                    script_line_exec();
                }
            }
        }
    }
}

fn skip_cmd(eap: &exarg_T) -> bool {
    if eap.skip != 0 {
        if eap.cmdidx < 0 || eap.cmdidx >= CMD_SIZE_I32 {
            return true;
        }
        let cmdidx = cmdidx_T::from_raw(eap.cmdidx);
        match cmdidx {
            cmdidx_T::CMD_while
            | cmdidx_T::CMD_endwhile
            | cmdidx_T::CMD_for
            | cmdidx_T::CMD_endfor
            | cmdidx_T::CMD_if
            | cmdidx_T::CMD_elseif
            | cmdidx_T::CMD_else
            | cmdidx_T::CMD_endif
            | cmdidx_T::CMD_try
            | cmdidx_T::CMD_catch
            | cmdidx_T::CMD_finally
            | cmdidx_T::CMD_endtry
            | cmdidx_T::CMD_function
            | cmdidx_T::CMD_append
            | cmdidx_T::CMD_change
            | cmdidx_T::CMD_insert
            | cmdidx_T::CMD_loadkeymap
            | cmdidx_T::CMD_aboveleft
            | cmdidx_T::CMD_and
            | cmdidx_T::CMD_belowright
            | cmdidx_T::CMD_botright
            | cmdidx_T::CMD_browse
            | cmdidx_T::CMD_call
            | cmdidx_T::CMD_confirm
            | cmdidx_T::CMD_const
            | cmdidx_T::CMD_delfunction
            | cmdidx_T::CMD_djump
            | cmdidx_T::CMD_dlist
            | cmdidx_T::CMD_dsearch
            | cmdidx_T::CMD_dsplit
            | cmdidx_T::CMD_echo
            | cmdidx_T::CMD_echoerr
            | cmdidx_T::CMD_echomsg
            | cmdidx_T::CMD_echon
            | cmdidx_T::CMD_eval
            | cmdidx_T::CMD_execute
            | cmdidx_T::CMD_filter
            | cmdidx_T::CMD_help
            | cmdidx_T::CMD_hide
            | cmdidx_T::CMD_horizontal
            | cmdidx_T::CMD_ijump
            | cmdidx_T::CMD_ilist
            | cmdidx_T::CMD_isearch
            | cmdidx_T::CMD_isplit
            | cmdidx_T::CMD_keepalt
            | cmdidx_T::CMD_keepjumps
            | cmdidx_T::CMD_keepmarks
            | cmdidx_T::CMD_keeppatterns
            | cmdidx_T::CMD_leftabove
            | cmdidx_T::CMD_let
            | cmdidx_T::CMD_lockmarks
            | cmdidx_T::CMD_lockvar
            | cmdidx_T::CMD_lua
            | cmdidx_T::CMD_match
            | cmdidx_T::CMD_mzscheme
            | cmdidx_T::CMD_noautocmd
            | cmdidx_T::CMD_noswapfile
            | cmdidx_T::CMD_perl
            | cmdidx_T::CMD_psearch
            | cmdidx_T::CMD_python
            | cmdidx_T::CMD_py3
            | cmdidx_T::CMD_python3
            | cmdidx_T::CMD_pythonx
            | cmdidx_T::CMD_pyx
            | cmdidx_T::CMD_return
            | cmdidx_T::CMD_rightbelow
            | cmdidx_T::CMD_ruby
            | cmdidx_T::CMD_silent
            | cmdidx_T::CMD_smagic
            | cmdidx_T::CMD_snomagic
            | cmdidx_T::CMD_substitute
            | cmdidx_T::CMD_syntax
            | cmdidx_T::CMD_tab
            | cmdidx_T::CMD_tcl
            | cmdidx_T::CMD_throw
            | cmdidx_T::CMD_tilde
            | cmdidx_T::CMD_topleft
            | cmdidx_T::CMD_unlet
            | cmdidx_T::CMD_unlockvar
            | cmdidx_T::CMD_verbose
            | cmdidx_T::CMD_vertical
            | cmdidx_T::CMD_wincmd => {}
            _ => return true,
        }
    }
    false
}

fn do_one_cmd(
    cmdlinep: *mut *mut c_char,
    cmdline_buf: *mut Vec<c_char>,
    flags: c_int,
    cstack: &mut cstack_T,
    fgetline: LineGetter,
) -> *mut c_char {
    unsafe {
        let mut errormsg: *const c_char = ptr::null();
        let save_reg_executing = g_st().reg_executing;
        let save_pending_end_reg_executing = g_st().pending_end_reg_executing;

        let mut ea: exarg_T = exarg_T::new();
        ea.line1 = 1;
        ea.line2 = 1;
        g_st().ex_nesting_level += 1;

        if docmd_st().quitmore != 0 && !fgetline.is_func() && !fgetline.is_autocmd() {
            docmd_st().quitmore -= 1;
        }

        let save_cmdmod = ptr::replace(&raw mut g_st().cmdmod, cmdmod_T::new());

        'doend: {
            if *(*cmdlinep) == b'#' as c_char && *(*cmdlinep).add(1) == b'!' as c_char {
                break 'doend;
            }

            ea.cmd = *cmdlinep;
            ea.cmdlinep = cmdlinep;
            ea.cmdline_buf = cmdline_buf;
            ea.ea_getline = fgetline;
            ea.cstack = &raw mut *cstack;

            if !parse_command_modifiers(&mut ea, &mut errormsg, &mut g_st().cmdmod, false)
            {
                break 'doend;
            }
            apply_cmdmod(&mut g_st().cmdmod);

            let after_modifier = ea.cmd;

            ea.skip = c_int::from(
                g_st().did_emsg != 0
                    || g_st().got_int
                    || g_st().did_throw
                    || (cstack.cs_idx >= 0
                        && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_ACTIVE) == 0),
            );

            let mut p = find_excmd_after_range(&mut ea);
            profile_cmd(&ea, cstack, fgetline);

            if !g_st().exiting {
                dbg_check_breakpoint(&mut ea);
            }
            if ea.skip == 0 && g_st().got_int {
                ea.skip = 1;
                do_intthrow(cstack);
            }

            set_cmd_addr_type(&mut ea, p);

            if !parse_cmd_address(&mut ea, &mut errormsg, false) {
                break 'doend;
            }

            ea.cmd = skip_colon_white(ea.cmd, true);

            if *ea.cmd == NUL || *ea.cmd == b'"' as c_char || {
                ea.nextcmd = check_nextcmd(ea.cmd);
                !ea.nextcmd.is_null()
            } {
                if ea.skip != 0 {
                    break 'doend;
                }
                debug_assert!(errormsg.is_null());
                errormsg = ex_range_without_command(&mut ea);
                break 'doend;
            }

            if !p.is_null()
                && ea.cmdidx == CMD_SIZE_I32
                && ea.skip == 0
                && ascii_isupper(*ea.cmd)
                && has_event(EVENT_CMDUNDEFINED)
            {
                let mut cmdend = ea.cmd;
                while ascii_isalnum(*cmdend) {
                    cmdend = cmdend.add(1);
                }
                let cmdname = cstr_from_bytes(std::slice::from_raw_parts(
                    ea.cmd.cast::<u8>(),
                    (cmdend as usize) - (ea.cmd as usize),
                ));
                let ret = apply_autocmds(
                    EVENT_CMDUNDEFINED,
                    cmdname.as_ptr().cast_mut(),
                    cmdname.as_ptr().cast_mut(),
                    true,
                    ptr::null_mut(),
                );
                p = if ret && !aborting() {
                    find_ex_command(&mut ea, None)
                } else {
                    ea.cmd
                };
            }

            if p.is_null() {
                if ea.skip == 0 {
                    errormsg = gettext(e_ambiguous_use_of_user_defined_command);
                }
                break 'doend;
            }

            if ea.cmdidx == CMD_SIZE_I32 {
                if ea.skip == 0 {
                    xstrlcpy(
                        (&raw mut g_st().IObuff).cast(),
                        gettext(e_not_an_editor_command),
                        IOSIZE,
                    );
                    let cmdname = if !after_modifier.is_null() {
                        after_modifier
                    } else {
                        *cmdlinep
                    };
                    if (flags & DOCMD_VERBOSE) == 0 {
                        append_command(cmdname);
                    }
                    errormsg = (&raw const g_st().IObuff).cast();
                    g_st().did_emsg_syntax = true;
                    verify_command(cmdname);
                }
                break 'doend;
            }

            let ni = is_cmd_ni(cmdidx_T::from_raw(ea.cmdidx));

            ea.forceit = c_int::from(parse_bang(&ea, &raw mut p));

            if ea.cmdidx >= 0 {
                ea.argt = CMDNAMES[ea.cmdidx as usize].cmd_argt;
            }

            if ea.skip == 0 {
                if g_st().sandbox != 0 && (ea.argt & EX_SBOXOK) == 0 {
                    errormsg = gettext(e_sandbox);
                    break 'doend;
                }
                if (*g_st().curbuf).b_p_ma == 0
                    && (ea.argt & EX_MODIFY) != 0
                    && !(!(*g_st().curbuf).terminal.is_null()
                        && (ea.cmdidx == cmdidx_T::CMD_put as c_int
                            || ea.cmdidx == cmdidx_T::CMD_iput as c_int))
                {
                    errormsg = gettext(e_modifiable);
                    break 'doend;
                }

                if ea.cmdidx >= 0 && text_locked() && (ea.argt & EX_LOCK_OK) == 0 {
                    errormsg = gettext(get_text_locked_msg());
                    break 'doend;
                }

                if (ea.argt & EX_BUFLOCK_OK) == 0
                    && ea.cmdidx != cmdidx_T::CMD_checktime as c_int
                    && ea.cmdidx != cmdidx_T::CMD_edit as c_int
                    && ea.cmdidx != cmdidx_T::CMD_file as c_int
                    && ea.cmdidx >= 0
                    && curbuf_locked()
                {
                    break 'doend;
                }

                if !ni && (ea.argt & EX_RANGE) == 0 && ea.addr_count > 0 {
                    errormsg = gettext(e_norange);
                    break 'doend;
                }
            }

            if !ni && (ea.argt & EX_BANG) == 0 && ea.forceit != 0 {
                errormsg = gettext(e_nobang);
                break 'doend;
            }

            if ea.skip == 0 && !ni && (ea.argt & EX_RANGE) != 0 {
                if g_st().global_busy == 0 && ea.line1 > ea.line2 {
                    if g_st().msg_silent == 0 {
                        if (flags & DOCMD_VERBOSE) != 0 || g_st().silent_mode {
                            errormsg = gettext(c"E493: Backwards range given");
                            break 'doend;
                        }
                        if ask_yesno(gettext(c"Backwards range given, OK to swap"))
                            != b'y' as c_int
                        {
                            break 'doend;
                        }
                    }
                    std::mem::swap(&mut ea.line1, &mut ea.line2);
                }
                errormsg = invalid_range(&mut ea);
                if !errormsg.is_null() {
                    break 'doend;
                }
            }

            if ea.addr_type == ADDR_OTHER && ea.addr_count == 0 {
                ea.line2 = 1;
            }

            correct_range(&mut ea);

            if ((ea.argt & EX_WHOLEFOLD) != 0 || ea.addr_count >= 2)
                && g_st().global_busy == 0
                && ea.addr_type == ADDR_LINES
            {
                hasFolding(*curwin.as_ptr(), ea.line1, Some(&mut ea.line1), ptr::null_mut());
                hasFolding(*curwin.as_ptr(), ea.line2, None, &raw mut ea.line2);
            }

            p = replace_makeprg(&mut ea, p, cmdlinep);
            if p.is_null() {
                break 'doend;
            }

            ea.arg = if ea.cmdidx == cmdidx_T::CMD_bang as c_int {
                p
            } else {
                skipwhite(p)
            };

            if ea.cmdidx == cmdidx_T::CMD_file as c_int && *ea.arg != NUL && curbuf_locked() {
                break 'doend;
            }

            if (ea.argt & EX_ARGOPT) != 0 {
                while *ea.arg == b'+' as c_char && *ea.arg.add(1) == b'+' as c_char {
                    if !getargopt(&mut ea) && !ni {
                        errormsg = gettext(e_invarg);
                        break 'doend;
                    }
                }
            }

            if ea.cmdidx == cmdidx_T::CMD_write as c_int
                || ea.cmdidx == cmdidx_T::CMD_update as c_int
            {
                if *ea.arg == b'>' as c_char {
                    ea.arg = ea.arg.add(1);
                    if *ea.arg != b'>' as c_char {
                        errormsg = gettext(c"E494: Use w or w>>");
                        break 'doend;
                    }
                    ea.arg = skipwhite(ea.arg.add(1));
                    ea.append = c_int::from(true);
                } else if *ea.arg == b'!' as c_char && ea.cmdidx == cmdidx_T::CMD_write as c_int {
                    ea.arg = ea.arg.add(1);
                    ea.usefilter = c_int::from(true);
                }
            } else if ea.cmdidx == cmdidx_T::CMD_read as c_int {
                if ea.forceit != 0 {
                    ea.usefilter = c_int::from(true);
                    ea.forceit = 0;
                } else if *ea.arg == b'!' as c_char {
                    ea.arg = ea.arg.add(1);
                    ea.usefilter = c_int::from(true);
                }
            } else if ea.cmdidx == cmdidx_T::CMD_lshift as c_int
                || ea.cmdidx == cmdidx_T::CMD_rshift as c_int
            {
                ea.amount = 1;
                while *ea.arg == *ea.cmd {
                    ea.arg = ea.arg.add(1);
                    ea.amount += 1;
                }
                ea.arg = skipwhite(ea.arg);
            }

            if (ea.argt & EX_CMDARG) != 0 && ea.usefilter == 0 {
                ea.do_ecmd_cmd = getargcmd(&raw mut ea.arg);
            }

            if (ea.argt & EX_TRLBAR) != 0 && ea.usefilter == 0 {
                separate_nextcmd(&mut ea);
            } else if ea.cmdidx == cmdidx_T::CMD_bang as c_int
                || ea.cmdidx == cmdidx_T::CMD_terminal as c_int
                || ea.cmdidx == cmdidx_T::CMD_global as c_int
                || ea.cmdidx == cmdidx_T::CMD_vglobal as c_int
                || ea.usefilter != 0
            {
                let mut s = ea.arg;
                while *s != NUL {
                    if *s == b'\\' as c_char && *s.add(1) == b'\n' as c_char {
                        ptr::copy(s.add(1), s, std::ffi::CStr::from_ptr(s.add(1)).to_bytes_with_nul().len());
                    } else if *s == b'\n' as c_char {
                        ea.nextcmd = s.add(1);
                        *s = NUL;
                        break;
                    }
                    s = s.add(1);
                }
            }

            if (ea.argt & EX_DFLALL) != 0 && ea.addr_count == 0 {
                set_cmd_dflall_range(&mut ea);
            }

            parse_register(&mut ea);
            if !parse_count(&mut ea, &mut errormsg, true) {
                break 'doend;
            }

            if (ea.argt & EX_FLAGS) != 0 {
                get_flags(&mut ea);
            }
            if !ni
                && (ea.argt & EX_EXTRA) == 0
                && *ea.arg != NUL
                && *ea.arg != b'"' as c_char
                && (*ea.arg != b'|' as c_char || (ea.argt & EX_TRLBAR) == 0)
            {
                errormsg = ex_errmsg(e_trailing_arg, ea.arg);
                break 'doend;
            }

            if !ni && (ea.argt & EX_NEEDARG) != 0 && *ea.arg == NUL {
                errormsg = gettext(e_argreq);
                break 'doend;
            }

            if skip_cmd(&ea) {
                break 'doend;
            }

            let mut retv: c_int = 0;
            if !execute_cmd0(&mut retv, &mut ea, &mut errormsg, false) {
                break 'doend;
            }

            if g_st().need_rethrow {
                do_throw(cstack);
            } else if g_st().check_cstack {
                if source_finished(fgetline) {
                    do_finish(&mut ea, true);
                } else if fgetline.is_func() && current_func_returned() != 0 {
                    let _ = do_return(&mut ea, true, false, None);
                }
            }
            g_st().need_rethrow = false;
            g_st().check_cstack = false;
        }

        if (**curwin.as_ptr()).w_cursor.lnum == 0 {
            (**curwin.as_ptr()).w_cursor.lnum = 1;
            (**curwin.as_ptr()).w_cursor.col = 0;
        }

        if !errormsg.is_null() && *errormsg != NUL && g_st().did_emsg == 0 {
            if (flags & DOCMD_VERBOSE) != 0 {
                let iobuf_ptr: *const c_char = (&raw const g_st().IObuff).cast();
                if errormsg != iobuf_ptr {
                    xstrlcpy((&raw mut g_st().IObuff).cast(), errormsg, IOSIZE);
                    errormsg = iobuf_ptr;
                }
                append_command(*ea.cmdlinep);
            }
            let _ = emsg(errormsg);
        }
        do_errthrow(
            Some(&mut *cstack),
            if ea.cmdidx != CMD_SIZE_I32 && ea.cmdidx >= 0 {
                CMDNAMES[ea.cmdidx as usize].cmd_name.as_ptr().cast_mut()
            } else {
                ptr::null_mut()
            },
        );

        undo_cmdmod(&mut g_st().cmdmod);
        g_st().cmdmod = save_cmdmod;
        g_st().reg_executing = save_reg_executing;
        g_st().pending_end_reg_executing = save_pending_end_reg_executing;

        if !ea.nextcmd.is_null() && *ea.nextcmd == NUL {
            ea.nextcmd = ptr::null_mut();
        }

        g_st().ex_nesting_level -= 1;

        ea.nextcmd
    }
}

fn ex_range_without_command(eap: &mut exarg_T) -> *mut c_char {
    unsafe {
        let mut errormsg: *mut c_char = ptr::null_mut();

        if *eap.cmd == b'|' as c_char {
            eap.cmdidx = cmdidx_T::CMD_print as c_int;
            eap.argt = EX_RANGE | EX_COUNT | EX_TRLBAR;
            errormsg = invalid_range(eap);
            if errormsg.is_null() {
                correct_range(eap);
                ex_print(&mut *eap);
            }
        } else if eap.addr_count != 0 {
            eap.line2 = eap.line2.min((*g_st().curbuf).b_ml.ml_line_count);

            if eap.line2 < 0 {
                errormsg = gettext(e_invrange);
            } else {
                if eap.line2 == 0 {
                    (**curwin.as_ptr()).w_cursor.lnum = 1;
                } else {
                    (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
                }
                beginline(BL_SOL | BL_FIX);
            }
        }
        errormsg
    }
}

pub fn parse_command_modifiers(
    eap: &mut exarg_T,
    errormsg: *mut *const c_char,
    cmod: &mut cmdmod_T,
    skip_only: bool,
) -> bool {
    unsafe {
        let orig_cmd = eap.cmd;
        let mut cmd_start: *mut c_char = ptr::null_mut();
        let mut has_visual_range = false;
        *cmod = cmdmod_T::new();

        if crate::memory::cstr_prefix_bytes(eap.cmd, 5) == &b"'<,'>"[..5] {
            let p = skipwhite(eap.cmd.add(5));
            if *p != NUL && *p != b'|' as c_char {
                eap.cmd = eap.cmd.add(5);
                cmd_start = eap.cmd;
                has_visual_range = true;
            }
        }

        loop {
            while *eap.cmd == b' ' as c_char
                || *eap.cmd == b'\t' as c_char
                || *eap.cmd == b':' as c_char
            {
                eap.cmd = eap.cmd.add(1);
            }

            if *eap.cmd == b'"' as c_char {
                eap.nextcmd = vim_strchr(eap.cmd, b'\n' as c_int);
                if !eap.nextcmd.is_null() {
                    eap.nextcmd = eap.nextcmd.add(1);
                }
                return false;
            }
            if *eap.cmd == b'\n' as c_char {
                eap.nextcmd = eap.cmd.add(1);
                return false;
            }
            if *eap.cmd == NUL {
                return false;
            }

            let mut p = skip_range(eap.cmd, None);
            match *p as u8 {
                b'a' => {
                    if !checkforcmd(&raw mut eap.cmd, c"aboveleft", 3) {
                        break;
                    }
                    cmod.cmod_split |= WSP_ABOVE;
                    continue;
                }
                b'b' => {
                    if checkforcmd(&raw mut eap.cmd, c"belowright", 3) {
                        cmod.cmod_split |= WSP_BELOW;
                        continue;
                    }
                    if checkforcmd(&raw mut eap.cmd, c"browse", 3) {
                        cmod.cmod_flags |= CMOD_BROWSE;
                        continue;
                    }
                    if !checkforcmd(&raw mut eap.cmd, c"botright", 2) {
                        break;
                    }
                    cmod.cmod_split |= WSP_BOT;
                    continue;
                }
                b'c' => {
                    if !checkforcmd(&raw mut eap.cmd, c"confirm", 4) {
                        break;
                    }
                    cmod.cmod_flags |= CMOD_CONFIRM;
                    continue;
                }
                b'k' => {
                    if checkforcmd(&raw mut eap.cmd, c"keepmarks", 3) {
                        cmod.cmod_flags |= CMOD_KEEPMARKS;
                        continue;
                    }
                    if checkforcmd(&raw mut eap.cmd, c"keepalt", 5) {
                        cmod.cmod_flags |= CMOD_KEEPALT;
                        continue;
                    }
                    if checkforcmd(&raw mut eap.cmd, c"keeppatterns", 5) {
                        cmod.cmod_flags |= CMOD_KEEPPATTERNS;
                        continue;
                    }
                    if !checkforcmd(&raw mut eap.cmd, c"keepjumps", 5) {
                        break;
                    }
                    cmod.cmod_flags |= CMOD_KEEPJUMPS;
                    continue;
                }
                b'f' => {
                    let mut reg_pat: *mut c_char = ptr::null_mut();
                    if !checkforcmd(&raw mut p, c"filter", 4)
                        || *p == NUL
                        || ends_excmd(c_int::from(*p)) != 0
                    {
                        break;
                    }
                    if *p == b'!' as c_char {
                        cmod.cmod_filter_force = true;
                        p = skipwhite(p.add(1));
                        if *p == NUL || ends_excmd(c_int::from(*p)) != 0 {
                            break;
                        }
                    }
                    if skip_only {
                        p = skip_vimgrep_pat(p, ptr::null_mut(), None);
                    } else {
                        p = skip_vimgrep_pat(p, &raw mut reg_pat, None);
                    }
                    if p.is_null() || *p == NUL {
                        break;
                    }
                    if !skip_only {
                        cmod.cmod_filter_pat = Some(cstr_to_cstring(reg_pat));
                        cmod.cmod_filter_regmatch.regprog = vim_regcomp(reg_pat, RE_MAGIC);
                        if cmod.cmod_filter_regmatch.regprog.is_null() {
                            break;
                        }
                    }
                    eap.cmd = p;
                    continue;
                }
                b'h' => {
                    if checkforcmd(&raw mut eap.cmd, c"horizontal", 3) {
                        cmod.cmod_split |= WSP_HOR;
                        continue;
                    }
                    if p != eap.cmd
                        || !checkforcmd(&raw mut p, c"hide", 3)
                        || *p == NUL
                        || ends_excmd(c_int::from(*p)) != 0
                    {
                        break;
                    }
                    eap.cmd = p;
                    cmod.cmod_flags |= CMOD_HIDE;
                    continue;
                }
                b'l' => {
                    if checkforcmd(&raw mut eap.cmd, c"lockmarks", 3) {
                        cmod.cmod_flags |= CMOD_LOCKMARKS;
                        continue;
                    }
                    if !checkforcmd(&raw mut eap.cmd, c"leftabove", 5) {
                        break;
                    }
                    cmod.cmod_split |= WSP_ABOVE;
                    continue;
                }
                b'n' => {
                    if checkforcmd(&raw mut eap.cmd, c"noautocmd", 3) {
                        cmod.cmod_flags |= CMOD_NOAUTOCMD;
                        continue;
                    }
                    if !checkforcmd(&raw mut eap.cmd, c"noswapfile", 3) {
                        break;
                    }
                    cmod.cmod_flags |= CMOD_NOSWAPFILE;
                    continue;
                }
                b'r' => {
                    if !checkforcmd(&raw mut eap.cmd, c"rightbelow", 6) {
                        break;
                    }
                    cmod.cmod_split |= WSP_BELOW;
                    continue;
                }
                b's' => {
                    if checkforcmd(&raw mut eap.cmd, c"sandbox", 3) {
                        cmod.cmod_flags |= CMOD_SANDBOX;
                        continue;
                    }
                    if !checkforcmd(&raw mut eap.cmd, c"silent", 3) {
                        break;
                    }
                    cmod.cmod_flags |= CMOD_SILENT;
                    if *eap.cmd == b'!' as c_char && !ascii_iswhite(*eap.cmd.sub(1)) {
                        eap.cmd = skipwhite(eap.cmd.add(1));
                        cmod.cmod_flags |= CMOD_ERRSILENT;
                    }
                    continue;
                }
                b't' => {
                    if checkforcmd(&raw mut p, c"tab", 3) {
                        if !skip_only {
                            let tabnr = get_address(
                                eap,
                                &raw mut eap.cmd,
                                ADDR_TABS,
                                eap.skip != 0,
                                skip_only,
                                0,
                                1,
                                errormsg,
                            );
                            if eap.cmd.is_null() {
                                return false;
                            }
                            if tabnr == MAXLNUM {
                                cmod.cmod_tab = tabpage_index(g_st().curtab) + 1;
                            } else {
                                if tabnr < 0 || tabnr > current_tab_nr(ptr::null_mut()) {
                                    *errormsg = gettext(e_invrange);
                                    return false;
                                }
                                cmod.cmod_tab = tabnr + 1;
                            }
                        }
                        eap.cmd = p;
                        continue;
                    }
                    if !checkforcmd(&raw mut eap.cmd, c"topleft", 2) {
                        break;
                    }
                    cmod.cmod_split |= WSP_TOP;
                    continue;
                }
                b'u' => {
                    if !checkforcmd(&raw mut eap.cmd, c"unsilent", 3) {
                        break;
                    }
                    cmod.cmod_flags |= CMOD_UNSILENT;
                    continue;
                }
                b'v' => {
                    if checkforcmd(&raw mut eap.cmd, c"vertical", 4) {
                        cmod.cmod_split |= WSP_VERT;
                        continue;
                    }
                    if !checkforcmd(&raw mut p, c"verbose", 4) {
                        break;
                    }
                    if ascii_isdigit(*eap.cmd) {
                        cmod.cmod_verbose = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.cmd)) as c_int + 1;
                    } else {
                        cmod.cmod_verbose = 2;
                    }
                    eap.cmd = p;
                    continue;
                }
                _ => {}
            }
            break;
        }

        if has_visual_range {
            if eap.cmd > cmd_start {
                ptr::copy(
                    cmd_start,
                    cmd_start.sub(5),
                    (eap.cmd as usize) - (cmd_start as usize),
                );
                eap.cmd = eap.cmd.sub(5);
                ptr::copy_nonoverlapping(c":'<,'>".as_ptr(), eap.cmd.sub(1), 6);
            } else {
                eap.cmd = orig_cmd;
            }
        }

        true
    }
}

pub fn apply_cmdmod(cmod: &mut cmdmod_T) {
    if (cmod.cmod_flags & CMOD_SANDBOX) != 0 && cmod.cmod_did_sandbox == 0 {
            g_st().sandbox += 1;
            cmod.cmod_did_sandbox = 1;
        }
        if cmod.cmod_verbose > 0 {
            if cmod.cmod_verbose_save == 0 {
                cmod.cmod_verbose_save = opt_vars().p_verbose + 1;
            }
            opt_vars().p_verbose = OptInt::from(cmod.cmod_verbose - 1);
        }

        if (cmod.cmod_flags & (CMOD_SILENT | CMOD_UNSILENT)) != 0
            && cmod.cmod_save_msg_silent == 0
        {
            cmod.cmod_save_msg_silent = g_st().msg_silent + 1;
            cmod.cmod_save_msg_scroll = g_st().msg_scroll;
        }
        if (cmod.cmod_flags & CMOD_SILENT) != 0 {
            g_st().msg_silent += 1;
        }
        if (cmod.cmod_flags & CMOD_UNSILENT) != 0 {
            g_st().msg_silent = 0;
        }

        if (cmod.cmod_flags & CMOD_ERRSILENT) != 0 {
            g_st().emsg_silent += 1;
            cmod.cmod_did_esilent += 1;
        }

        if (cmod.cmod_flags & CMOD_NOAUTOCMD) != 0 && cmod.cmod_save_ei.is_none() {
            cmod.cmod_save_ei = Some(cstr_to_cstring(opt_vars().p_ei));
            set_option_direct(kOptEventignore, cstr_as_obj(c"all"), 0, SID_NONE);
        }
}

pub fn undo_cmdmod(cmod: &mut cmdmod_T) {
    if cmod.cmod_verbose_save > 0 {
            opt_vars().p_verbose = cmod.cmod_verbose_save - 1;
            cmod.cmod_verbose_save = 0;
        }

        if cmod.cmod_did_sandbox != 0 {
            g_st().sandbox -= 1;
            cmod.cmod_did_sandbox = 0;
        }

        if let Some(save_ei) = cmod.cmod_save_ei.take() {
            set_option_direct(kOptEventignore, cstr_as_obj(save_ei.as_ptr()), 0, SID_NONE);
        }

        cmod.cmod_filter_pat = None;
        vim_regfree(cmod.cmod_filter_regmatch.regprog);

        if cmod.cmod_save_msg_silent > 0 {
            if g_st().did_emsg == 0 || g_st().msg_silent > cmod.cmod_save_msg_silent - 1 {
                g_st().msg_silent = cmod.cmod_save_msg_silent - 1;
            }
            g_st().emsg_silent -= cmod.cmod_did_esilent;
            if g_st().emsg_silent < 0 {
                g_st().emsg_silent = 0;
            }
            g_st().msg_scroll = cmod.cmod_save_msg_scroll;

            if redirecting() != 0 {
                g_st().msg_col = 0;
            }

            cmod.cmod_save_msg_silent = 0;
            cmod.cmod_did_esilent = 0;
        }
}

pub fn parse_cmd_address(
    eap: &mut exarg_T,
    errormsg: *mut *const c_char,
    silent: bool,
) -> bool {
    unsafe {
        let mut address_count: c_int = 1;
        let mut lnum: linenr_T;
        let mut need_check_cursor = false;
        let mut ret = false;

        'theend: {
            loop {
                eap.line1 = eap.line2;
                eap.line2 = get_cmd_default_range(eap);
                eap.cmd = skipwhite(eap.cmd);
                let cur_addr_cnt = address_count;
                address_count += 1;
                lnum = get_address(
                    eap,
                    &raw mut eap.cmd,
                    eap.addr_type,
                    eap.skip != 0,
                    silent,
                    c_int::from(eap.addr_count == 0),
                    cur_addr_cnt,
                    errormsg,
                );
                if eap.cmd.is_null() {
                    break 'theend;
                }
                if lnum == MAXLNUM {
                    if *eap.cmd == b'%' as c_char {
                        eap.cmd = eap.cmd.add(1);
                        match eap.addr_type {
                            ADDR_LINES | ADDR_OTHER => {
                                eap.line1 = 1;
                                eap.line2 = (*g_st().curbuf).b_ml.ml_line_count;
                            }
                            ADDR_LOADED_BUFFERS => {
                                let mut buf = g_st().firstbuf;
                                while !(*buf).b_next.is_null() && (*buf).b_ml.ml_mfp.is_none() {
                                    buf = (*buf).b_next;
                                }
                                eap.line1 = (*buf).b_fnum;
                                buf = g_st().lastbuf;
                                while !(*buf).b_prev.is_null() && (*buf).b_ml.ml_mfp.is_none() {
                                    buf = (*buf).b_prev;
                                }
                                eap.line2 = (*buf).b_fnum;
                            }
                            ADDR_BUFFERS => {
                                eap.line1 = (*g_st().firstbuf).b_fnum;
                                eap.line2 = (*g_st().lastbuf).b_fnum;
                            }
                            ADDR_WINDOWS | ADDR_TABS => {
                                if eap.cmdidx < 0 {
                                    eap.line1 = 1;
                                    eap.line2 = if eap.addr_type == ADDR_WINDOWS {
                                        current_win_nr(ptr::null(), None)
                                    } else {
                                        current_tab_nr(ptr::null_mut())
                                    };
                                } else {
                                    *errormsg = gettext(e_invrange);
                                    break 'theend;
                                }
                            }
                            ADDR_TABS_RELATIVE | ADDR_UNSIGNED | ADDR_QUICKFIX => {
                                *errormsg = gettext(e_invrange);
                                break 'theend;
                            }
                            ADDR_ARGUMENTS => {
                                let ac = argcount();
                                if ac == 0 {
                                    eap.line1 = 0;
                                    eap.line2 = 0;
                                } else {
                                    eap.line1 = 1;
                                    eap.line2 = ac;
                                }
                            }
                            ADDR_QUICKFIX_VALID => {
                                eap.line1 = 1;
                                eap.line2 = qf_get_valid_size(eap) as linenr_T;
                                if eap.line2 == 0 {
                                    eap.line2 = 1;
                                }
                            }
                            _ => {}
                        }
                        eap.addr_count += 1;
                    } else if *eap.cmd == b'*' as c_char {
                        if eap.addr_type != ADDR_LINES {
                            *errormsg = gettext(e_invrange);
                            break 'theend;
                        }

                        eap.cmd = eap.cmd.add(1);
                        if eap.skip == 0 {
                            let fm1 = mark_get_visual(&mut *g_st().curbuf, b'<' as c_int);
                            if !mark_check(fm1, errormsg) {
                                break 'theend;
                            }
                            eap.line1 = (*fm1).mark.lnum;
                            let fm2 = mark_get_visual(&mut *g_st().curbuf, b'>' as c_int);
                            if !mark_check(fm2, errormsg) {
                                break 'theend;
                            }
                            eap.line2 = (*fm2).mark.lnum;
                            eap.addr_count += 1;
                        }
                    }
                } else {
                    eap.line2 = lnum;
                }
                eap.addr_count += 1;

                if *eap.cmd == b';' as c_char {
                    if eap.skip == 0 {
                        (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
                        if eap.line2 > 0 {
                            check_cursor(*curwin.as_ptr());
                        } else {
                            check_cursor_col(&mut **curwin.as_ptr());
                        }
                        need_check_cursor = true;
                    }
                } else if *eap.cmd != b',' as c_char {
                    break;
                }
                eap.cmd = eap.cmd.add(1);
            }

            if eap.addr_count == 1 {
                eap.line1 = eap.line2;
                if lnum == MAXLNUM {
                    eap.addr_count = 0;
                }
            }
            ret = true;
        }

        if need_check_cursor {
            check_cursor(*curwin.as_ptr());
        }
        ret
    }
}

pub fn checkforcmd(pp: *mut *mut c_char, cmd: &std::ffi::CStr, len: c_int) -> bool {
    unsafe {
        let mut i: usize = 0;
        while *cmd.as_ptr().add(i) != NUL {
            if *cmd.as_ptr().add(i) != *(*pp).add(i) {
                break;
            }
            i += 1;
        }
        if (i as c_int) >= len && !ascii_isalpha(*(*pp).add(i)) {
            *pp = skipwhite((*pp).add(i));
            return true;
        }
        false
    }
}

fn append_command(cmd: *const c_char) {
    unsafe {
        let iobuf_ptr: *mut c_char = (&raw mut g_st().IObuff).cast();
        let len = crate::memory::cstr_len(iobuf_ptr);
        let mut s = cmd;
        let mut d: *mut c_char;

        if len > IOSIZE - 100 {
            d = iobuf_ptr.add(IOSIZE - 100);
            d = d.sub(utf_head_off(iobuf_ptr, d) as usize);
            crate::memory::cstr_cpy(d, c"...");
        }
        { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), c": ", _dsize - _dlen); };
        d = iobuf_ptr.add(crate::memory::cstr_len(iobuf_ptr));
        while *s != NUL && ((d as usize) - (iobuf_ptr as usize) + 5 < IOSIZE) {
            if (*s as u8) == 0xc2 && (*s.add(1) as u8) == 0xa0 {
                s = s.add(2);
                crate::memory::cstr_cpy(d, c"<a0>");
                d = d.add(4);
            } else if (d as usize) - (iobuf_ptr as usize) + (utfc_ptr2len(s) as usize) + 1 >= IOSIZE
            {
                break;
            } else {
                mb_copy_char(&raw mut s, &raw mut d);
            }
        }
        *d = NUL;
    }
}

fn one_letter_cmd(p: *const c_char, idx: &mut cmdidx_T) -> c_int {
    unsafe {
        if *p == b'k' as c_char
            && (*p.add(1) != b'e' as c_char
                || (*p.add(1) == b'e' as c_char && *p.add(2) != b'e' as c_char))
        {
            *idx = cmdidx_T::CMD_k;
            return c_int::from(true);
        }
        if *p == b's' as c_char
            && ((*p.add(1) == b'c' as c_char
                && (*p.add(2) == NUL
                    || (*p.add(2) != b's' as c_char
                        && *p.add(2) != b'r' as c_char
                        && (*p.add(3) == NUL
                            || (*p.add(3) != b'i' as c_char && *p.add(4) != b'p' as c_char)))))
                || *p.add(1) == b'g' as c_char
                || (*p.add(1) == b'i' as c_char
                    && *p.add(2) != b'm' as c_char
                    && *p.add(2) != b'l' as c_char
                    && *p.add(2) != b'g' as c_char)
                || *p.add(1) == b'I' as c_char
                || (*p.add(1) == b'r' as c_char && *p.add(2) != b'e' as c_char))
        {
            *idx = cmdidx_T::CMD_substitute;
            return c_int::from(true);
        }
        c_int::from(false)
    }
}

pub fn find_ex_command(eap: &mut exarg_T,mut full: Option<&mut c_int>) -> *mut c_char {
    unsafe {
        let mut p = eap.cmd;
        let mut cmdidx_out = cmdidx_T::CMD_append;
        if one_letter_cmd(p, &mut cmdidx_out) != 0 {
            eap.cmdidx = cmdidx_out as c_int;
            p = p.add(1);
            if let Some(ref mut p) = full {
            **p = c_int::from(true);
        }
        } else {
            while ascii_isalpha(*p) {
                p = p.add(1);
            }
            if *eap.cmd == b'p' as c_char && *eap.cmd.add(1) == b'y' as c_char {
                while ascii_isalnum(*p) {
                    p = p.add(1);
                }
            }

            if p == eap.cmd && !vim_strchr(c"@!=><&~#", c_int::from(*p as u8)).is_null()
            {
                p = p.add(1);
            }
            let mut len = ((p as usize) - (eap.cmd as usize)) as c_int;

            if *eap.cmd == b'd' as c_char
                && (*p.sub(1) == b'l' as c_char || *p.sub(1) == b'p' as c_char)
            {
                let delete_bytes = b"delete\0";
                let mut i: c_int = 0;
                while i < len {
                    if (*eap.cmd.add(i as usize) as u8) != delete_bytes[i as usize] {
                        break;
                    }
                    i += 1;
                }
                if i == len - 1 {
                    len -= 1;
                    if *p.sub(1) == b'l' as c_char {
                        eap.flags |= EXFLAG_LIST;
                    } else {
                        eap.flags |= EXFLAG_PRINT;
                    }
                }
            }

            if ascii_islower(*eap.cmd) {
                let c1 = *eap.cmd as u8;
                let c2 = if len == 1 {
                    0u8
                } else {
                    *eap.cmd.add(1) as u8
                };

                if COMMAND_COUNT != CMD_SIZE_I32 {
                    iemsg(gettext(
                        c"E943: Command table needs to be updated, run 'make'",
                    ));
                    getout(1);
                }

                let idx1 = (c1 - b'a') as usize;
                eap.cmdidx = c_int::from(CMDIDXS1[idx1]);
                if c2.is_ascii_lowercase() {
                    eap.cmdidx += c_int::from(CMDIDXS2[idx1][(c2 - b'a') as usize]);
                }
            } else if ascii_isupper(*eap.cmd) {
                eap.cmdidx = cmdidx_T::CMD_Next as c_int;
            } else {
                eap.cmdidx = cmdidx_T::CMD_bang as c_int;
            }

            if len == 3 && (b"def" == crate::memory::cstr_prefix_bytes(eap.cmd, 3)) {
                eap.cmdidx = CMD_SIZE_I32;
            }

            while eap.cmdidx < CMD_SIZE_I32 {
                let cname = CMDNAMES[eap.cmdidx as usize].cmd_name.as_ptr();
                if crate::memory::cstr_prefix_bytes(cname, len as usize) == crate::memory::cstr_prefix_bytes(eap.cmd, len as usize) {
                    if full.is_some() && *cname.add(len as usize) == NUL {
                        *full.as_deref_mut().unwrap() = c_int::from(true);
                    }
                    break;
                }
                eap.cmdidx += 1;
            }

            if eap.cmdidx == CMD_SIZE_I32
                && (*eap.cmd as u8) >= b'A'
                && (*eap.cmd as u8) <= b'Z'
            {
                while ascii_isalnum(*p) {
                    p = p.add(1);
                }
                p = find_ucmd(eap, p, (full.is_some()).then(||  { &mut *full.unwrap() }), None, None);
            }
            if p == eap.cmd {
                eap.cmdidx = CMD_SIZE_I32;
            }
        }

        if eap.cmdidx == cmdidx_T::CMD_horizontal as c_int
            && ((p as usize) - (eap.cmd as usize)) == 2
        {
            eap.cmdidx = CMD_SIZE_I32;
        }

        p
    }
}

pub fn modifier_len(cmd: *mut c_char) -> c_int {
    unsafe {
        let mut p = cmd;
        if ascii_isdigit(*cmd) {
            p = skipwhite(skipdigits(cmd.add(1)));
        }
        for info in &CMDMODS {
            if info.name[0] > (*p as u8) {
                break;
            }
            let mut j = 0usize;
            while *p.add(j) != NUL {
                if j >= info.name.len() || (*p.add(j) as u8) != info.name[j] {
                    break;
                }
                j += 1;
            }
            if j >= info.minlen && !ascii_isalpha(*p.add(j)) && (p == cmd || info.has_count) {
                return (j + ((p as usize) - (cmd as usize))) as c_int;
            }
        }
        0
    }
}

pub fn cmd_exists(name: *const c_char) -> c_int {
    unsafe {
        for info in &CMDMODS {
            let mut j = 0usize;
            while *name.add(j) != NUL {
                if j >= info.name.len() || (*name.add(j) as u8) != info.name[j] {
                    break;
                }
                j += 1;
            }
            if *name.add(j) == NUL && j >= info.minlen {
                return if j == info.name.len() { 2 } else { 1 };
            }
        }

        let mut ea: exarg_T = exarg_T::new();
        ea.cmd = if *name == b'2' as c_char || *name == b'3' as c_char {
            name.add(1).cast_mut()
        } else {
            name.cast_mut()
        };
        ea.cmdidx = 0;
        ea.flags = 0;
        let mut full: c_int = 0;
        let p = find_ex_command(&mut ea, Some(&mut full));
        if p.is_null() {
            return 3;
        }
        if ascii_isdigit(*name) && ea.cmdidx != cmdidx_T::CMD_match as c_int {
            return 0;
        }
        if *skipwhite(p) != NUL {
            return 0;
        }
        if ea.cmdidx == CMD_SIZE_I32 {
            0
        } else if full != 0 {
            2
        } else {
            1
        }
    }
}

pub fn f_fullcommand(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let mut name = tv_get_string(&argvars[0]).cast_mut();

        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        while *name == b':' as c_char {
            name = name.add(1);
        }
        name = skip_range(name, None);

        let mut ea: exarg_T = exarg_T::new();
        ea.cmd = if *name == b'2' as c_char || *name == b'3' as c_char {
            name.add(1)
        } else {
            name
        };
        ea.cmdidx = 0;
        ea.flags = 0;
        let p = find_ex_command(&mut ea, None);
        if p.is_null() || ea.cmdidx == CMD_SIZE_I32 {
            return;
        }

        crate::eval::typval::tv_set_string(
            rettv,
            if ea.cmdidx < 0 {
                get_user_command_name(ea.useridx, ea.cmdidx)
            } else {
                CMDNAMES[ea.cmdidx as usize].cmd_name.as_ptr()
            },
        );
    }
}

pub fn excmd_get_cmdidx(cmd: *const c_char, len: usize) -> cmdidx_T {
    if len == 3 && (b"def" == crate::memory::cstr_prefix_bytes(cmd, 3)) {
            return cmdidx_T::CMD_SIZE;
        }

        let mut idx = cmdidx_T::CMD_append;
        if one_letter_cmd(cmd, &mut idx) == 0 {
            let mut i: c_int = 0;
            while i < CMD_SIZE_I32 {
                if crate::memory::cstr_prefix_bytes(CMDNAMES[i as usize].cmd_name.as_ptr(), len) == crate::memory::cstr_prefix_bytes(cmd, len) {
                    break;
                }
                i += 1;
            }
            idx = cmdidx_T::from_raw(i);
        }
        idx
}

pub fn excmd_get_argt(idx: cmdidx_T) -> u32 {
    CMDNAMES[idx as usize].cmd_argt
}

pub fn skip_range(mut cmd: *const c_char, mut ctx: Option<&mut c_int>) -> *mut c_char {
    unsafe {
        while !vim_strchr(
            c" \t0123456789.$%'/?-+,;\\",
            c_int::from(*cmd as u8),
        )
        .is_null()
        {
            if *cmd == b'\\' as c_char {
                let next = *cmd.add(1);
                if next == b'?' as c_char || next == b'/' as c_char || next == b'&' as c_char {
                    cmd = cmd.add(1);
                } else {
                    break;
                }
            } else if *cmd == b'\'' as c_char {
                cmd = cmd.add(1);
                if *cmd == NUL
                    && let Some(ref mut ctx) = ctx
                {
                    **ctx = EXPAND_NOTHING;
                }
            } else if *cmd == b'/' as c_char || *cmd == b'?' as c_char {
                cmd = skip_regexp(cmd.add(1).cast_mut(), c_int::from(*cmd as u8), opt_vars().p_magic);
                if *cmd == NUL
                    && let Some(ref mut ctx) = ctx
                {
                    **ctx = EXPAND_NOTHING;
                }
            }
            if *cmd != NUL {
                cmd = cmd.add(1);
            }
        }

        cmd = skip_colon_white(cmd, false);

        if *cmd == b'*' as c_char {
            cmd = skipwhite(cmd.add(1));
        }

        cmd.cast_mut()
    }
}

fn addr_error(addr_type: cmd_addr_T) -> *const c_char {
    if addr_type == ADDR_NONE {
            gettext(e_norange)
        } else {
            gettext(e_invrange)
        }
}

pub fn get_address(
    eap: *mut exarg_T,
    ptr: *mut *mut c_char,
    addr_type: cmd_addr_T,
    skip: bool,
    silent: bool,
    to_other_file: c_int,
    address_count: c_int,
    errormsg: *mut *const c_char,
) -> linenr_T {
    unsafe {
        let mut cmd = skipwhite(*ptr);
        let mut lnum: linenr_T = MAXLNUM;

        'error: {
            loop {
                match *cmd as u8 {
                    b'.' => {
                        cmd = cmd.add(1);
                        match addr_type {
                            ADDR_LINES | ADDR_OTHER => lnum = (**curwin.as_ptr()).w_cursor.lnum,
                            ADDR_WINDOWS => lnum = current_win_nr(*curwin.as_ptr(), None),
                            ADDR_ARGUMENTS => lnum = (**curwin.as_ptr()).w_arg_idx + 1,
                            ADDR_LOADED_BUFFERS | ADDR_BUFFERS => lnum = (*g_st().curbuf).b_fnum,
                            ADDR_TABS => lnum = current_tab_nr(g_st().curtab),
                            ADDR_NONE | ADDR_TABS_RELATIVE | ADDR_UNSIGNED => {
                                *errormsg = addr_error(addr_type);
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            ADDR_QUICKFIX => lnum = qf_get_cur_idx(&mut *eap) as linenr_T,
                            ADDR_QUICKFIX_VALID => lnum = qf_get_cur_valid_idx(&mut *eap),
                            _ => {}
                        }
                    }
                    b'$' => {
                        cmd = cmd.add(1);
                        match addr_type {
                            ADDR_LINES | ADDR_OTHER => lnum = (*g_st().curbuf).b_ml.ml_line_count,
                            ADDR_WINDOWS => lnum = current_win_nr(ptr::null(), Some(&*eap)),
                            ADDR_ARGUMENTS => lnum = argcount(),
                            ADDR_LOADED_BUFFERS => {
                                let mut buf = g_st().lastbuf;
                                while (*buf).b_ml.ml_mfp.is_none() {
                                    if (*buf).b_prev.is_null() {
                                        break;
                                    }
                                    buf = (*buf).b_prev;
                                }
                                lnum = (*buf).b_fnum;
                            }
                            ADDR_BUFFERS => lnum = (*g_st().lastbuf).b_fnum,
                            ADDR_TABS => lnum = current_tab_nr(ptr::null_mut()),
                            ADDR_NONE | ADDR_TABS_RELATIVE | ADDR_UNSIGNED => {
                                *errormsg = addr_error(addr_type);
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            ADDR_QUICKFIX => {
                                lnum = qf_get_size(&mut *eap) as linenr_T;
                                if lnum == 0 {
                                    lnum = 1;
                                }
                            }
                            ADDR_QUICKFIX_VALID => {
                                lnum = qf_get_valid_size(&mut *eap) as linenr_T;
                                if lnum == 0 {
                                    lnum = 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    b'\'' => {
                        cmd = cmd.add(1);
                        if *cmd == NUL {
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        if addr_type != ADDR_LINES {
                            *errormsg = addr_error(addr_type);
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        if skip {
                            cmd = cmd.add(1);
                        } else {
                            let flag = if to_other_file != 0 && *cmd.add(1) == NUL {
                                kMarkAll
                            } else {
                                kMarkBufLocal
                            };
                            let fm = mark_get(
                                &mut *g_st().curbuf,
                                *curwin.as_ptr(),
                                None,
                                flag,
                                c_int::from(*cmd as u8),
                            );
                            cmd = cmd.add(1);
                            if !fm.is_null() && (*fm).fnum != (*g_st().curbuf).b_fnum {
                                mark_move_to(fm, 0);
                                lnum = (**curwin.as_ptr()).w_cursor.lnum;
                            } else {
                                if !mark_check(fm, errormsg) {
                                    cmd = ptr::null_mut();
                                    break 'error;
                                }
                                lnum = (*fm).mark.lnum;
                            }
                        }
                    }
                    b'/' | b'?' => {
                        let c = c_int::from(*cmd as u8);
                        cmd = cmd.add(1);
                        if addr_type != ADDR_LINES {
                            *errormsg = addr_error(addr_type);
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        if skip {
                            cmd = skip_regexp(cmd, c, opt_vars().p_magic);
                            if c_int::from(*cmd as u8) == c {
                                cmd = cmd.add(1);
                            }
                        } else {
                            let pos = (**curwin.as_ptr()).w_cursor;
                            if lnum > 0 && lnum != MAXLNUM {
                                (**curwin.as_ptr()).w_cursor.lnum = lnum.min((*g_st().curbuf).b_ml.ml_line_count);
                            }
                            (**curwin.as_ptr()).w_cursor.col =
                                if c == b'/' as c_int && (**curwin.as_ptr()).w_cursor.lnum > 0 {
                                    MAXCOL
                                } else {
                                    0
                                };
                            g_st().Search.cmdlen = 0;
                            let search_flags = if silent {
                                SEARCH_KEEP
                            } else {
                                SEARCH_HIS | SEARCH_MSG
                            };
                            if do_search(
                                ptr::null_mut(),
                                c,
                                c,
                                cmd,
                                crate::memory::cstr_len(cmd),
                                1,
                                search_flags,
                                opt_vars().p_magic != 0,
                                None,
                            ) == 0
                            {
                                (**curwin.as_ptr()).w_cursor = pos;
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            lnum = (**curwin.as_ptr()).w_cursor.lnum;
                            (**curwin.as_ptr()).w_cursor = pos;
                            cmd = cmd.add(g_st().Search.cmdlen as usize);
                        }
                    }
                    b'\\' => {
                        cmd = cmd.add(1);
                        if addr_type != ADDR_LINES {
                            *errormsg = addr_error(addr_type);
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        let pat_use = if *cmd == b'&' as c_char {
                            RE_SUBST
                        } else if *cmd == b'?' as c_char || *cmd == b'/' as c_char {
                            RE_SEARCH
                        } else {
                            *errormsg = gettext(e_backslash);
                            cmd = ptr::null_mut();
                            break 'error;
                        };
                        if !skip {
                            let mut pos = pos_T {
                                lnum: if lnum != MAXLNUM {
                                    lnum
                                } else {
                                    (**curwin.as_ptr()).w_cursor.lnum
                                },
                                col: if *cmd != b'?' as c_char { MAXCOL } else { 0 },
                                coladd: 0,
                            };
                            if searchit(
                                *curwin.as_ptr(),
                                g_st().curbuf,
                                &raw mut pos,
                                None,
                                if *cmd == b'?' as c_char {
                                    BACKWARD
                                } else {
                                    FORWARD
                                },
                                c"".as_ptr().cast_mut(),
                                0,
                                1,
                                SEARCH_MSG,
                                pat_use,
                                opt_vars().p_magic != 0,
                                None,
                            ) != FAIL
                            {
                                lnum = pos.lnum;
                            } else {
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                        }
                        cmd = cmd.add(1);
                    }
                    _ => {
                        if ascii_isdigit(*cmd) {
                            lnum = getdigits(&raw mut cmd, false, 0) as linenr_T;
                        }
                    }
                }

                loop {
                    cmd = skipwhite(cmd);
                    if *cmd != b'-' as c_char && *cmd != b'+' as c_char && !ascii_isdigit(*cmd) {
                        break;
                    }

                    if lnum == MAXLNUM {
                        match addr_type {
                            ADDR_LINES | ADDR_OTHER => lnum = (**curwin.as_ptr()).w_cursor.lnum,
                            ADDR_WINDOWS => lnum = current_win_nr(*curwin.as_ptr(), None),
                            ADDR_ARGUMENTS => lnum = (**curwin.as_ptr()).w_arg_idx + 1,
                            ADDR_LOADED_BUFFERS | ADDR_BUFFERS => lnum = (*g_st().curbuf).b_fnum,
                            ADDR_TABS => lnum = current_tab_nr(g_st().curtab),
                            ADDR_TABS_RELATIVE => lnum = 1,
                            ADDR_QUICKFIX => lnum = qf_get_cur_idx(&mut *eap) as linenr_T,
                            ADDR_QUICKFIX_VALID => lnum = qf_get_cur_valid_idx(&mut *eap),
                            _ => lnum = 0,
                        }
                    }

                    let op = if ascii_isdigit(*cmd) {
                        b'+' as c_int
                    } else {
                        let ch = c_int::from(*cmd as u8);
                        cmd = cmd.add(1);
                        ch
                    };
                    let n = if !ascii_isdigit(*cmd) {
                        1
                    } else {
                        let parsed = getdigits_int32(&raw mut cmd, false, MAXLNUM);
                        if parsed == MAXLNUM {
                            *errormsg = gettext(e_line_number_out_of_range);
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        parsed
                    };

                    if addr_type == ADDR_TABS_RELATIVE {
                        *errormsg = gettext(e_invrange);
                        cmd = ptr::null_mut();
                        break 'error;
                    } else if addr_type == ADDR_LOADED_BUFFERS || addr_type == ADDR_BUFFERS {
                        lnum = compute_buffer_local_count(
                            addr_type,
                            lnum,
                            if op == b'-' as c_int { -n } else { n },
                        );
                    } else {
                        if addr_type == ADDR_LINES
                            && (op == b'-' as c_int || op == b'+' as c_int)
                            && address_count >= 2
                        {
                            hasFolding(*curwin.as_ptr(), lnum, None, &raw mut lnum);
                        }
                        if op == b'-' as c_int {
                            lnum -= n;
                        } else {
                            if lnum >= 0 && n >= i32::MAX - lnum {
                                *errormsg = gettext(e_line_number_out_of_range);
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            lnum += n;
                        }
                    }
                }

                if *cmd != b'/' as c_char && *cmd != b'?' as c_char {
                    break;
                }
            }
        }

        *ptr = cmd;
        lnum
    }
}

fn get_flags(eap: &mut exarg_T) {
    unsafe {
        while !vim_strchr(c"lp#", c_int::from(*eap.arg as u8)).is_null() {
            if *eap.arg == b'l' as c_char {
                eap.flags |= EXFLAG_LIST;
            } else if *eap.arg == b'p' as c_char {
                eap.flags |= EXFLAG_PRINT;
            } else {
                eap.flags |= EXFLAG_NR;
            }
            eap.arg = skipwhite(eap.arg.add(1));
        }
    }
}

pub fn ex_ni(eap: &mut exarg_T) {
    if eap.skip == 0 {
            eap.errmsg = gettext(c"E319: The command is not available in this version");
        }
}

fn ex_script_ni(eap: &mut exarg_T) {
    if eap.skip == 0 {
            ex_ni(eap);
        } else {
            let mut len: usize = 0;
            let _ = script_get(eap, &mut len);
        }
}

pub fn invalid_range(eap: &mut exarg_T) -> *mut c_char {
    unsafe {
        if eap.line1 < 0 || eap.line2 < 0 || eap.line1 > eap.line2 {
            return gettext(e_invrange);
        }

        if (eap.argt & EX_RANGE) != 0 {
            let addr_type = eap.addr_type;
            match addr_type {
                ADDR_LINES => {
                    let extra = linenr_T::from(
                        eap.cmdidx == cmdidx_T::CMD_diffget as c_int
                            || eap.cmdidx == cmdidx_T::CMD_diffput as c_int,
                    );
                    if eap.line2 > (*g_st().curbuf).b_ml.ml_line_count + extra {
                        return gettext(e_invrange);
                    }
                }
                ADDR_ARGUMENTS => {
                    let ac = argcount();
                    if eap.line2 > ac + c_int::from(ac == 0) {
                        return gettext(e_invrange);
                    }
                }
                ADDR_BUFFERS => {
                    if eap.line1 < 1 || eap.line2 > get_highest_fnum() {
                        return gettext(e_invrange);
                    }
                }
                ADDR_LOADED_BUFFERS => {
                    let mut buf = g_st().firstbuf;
                    while (*buf).b_ml.ml_mfp.is_none() {
                        if (*buf).b_next.is_null() {
                            return gettext(e_invrange);
                        }
                        buf = (*buf).b_next;
                    }
                    if eap.line1 < (*buf).b_fnum {
                        return gettext(e_invrange);
                    }
                    buf = g_st().lastbuf;
                    while (*buf).b_ml.ml_mfp.is_none() {
                        if (*buf).b_prev.is_null() {
                            return gettext(e_invrange);
                        }
                        buf = (*buf).b_prev;
                    }
                    if eap.line2 > (*buf).b_fnum {
                        return gettext(e_invrange);
                    }
                }
                ADDR_WINDOWS => {
                    if eap.line2 > current_win_nr(ptr::null(), Some(&*eap)) {
                        return gettext(e_invrange);
                    }
                }
                ADDR_TABS => {
                    if eap.line2 > current_tab_nr(ptr::null_mut()) {
                        return gettext(e_invrange);
                    }
                }
                ADDR_TABS_RELATIVE | ADDR_OTHER => {}
                ADDR_QUICKFIX => {
                    if eap.line2 <= 0 {
                        if eap.addr_count == 0 {
                            return gettext(e_no_errors);
                        }
                        return gettext(e_invrange);
                    }
                }
                ADDR_QUICKFIX_VALID
                    if (eap.line2 != 1 && (eap.line2 as usize) > qf_get_valid_size(&mut *eap))
                        || eap.line2 < 0 =>
                {
                    return gettext(e_invrange);
                }
                _ => {}
            }
        }
        ptr::null_mut()
    }
}

fn correct_range(eap: &mut exarg_T) {
    if (eap.argt & EX_ZEROR) == 0 {
        if eap.line1 == 0 {
            eap.line1 = 1;
        }
        if eap.line2 == 0 {
            eap.line2 = 1;
        }
    }
}

fn skip_grep_pat(eap: &mut exarg_T) -> *mut c_char {
    unsafe {
        let mut p = eap.arg;
        if *p != NUL
            && (eap.cmdidx == cmdidx_T::CMD_vimgrep as c_int
                || eap.cmdidx == cmdidx_T::CMD_lvimgrep as c_int
                || eap.cmdidx == cmdidx_T::CMD_vimgrepadd as c_int
                || eap.cmdidx == cmdidx_T::CMD_lvimgrepadd as c_int
                || (eap.cmdidx >= 0 && grep_internal(eap.cmdidx) != 0))
        {
            p = skip_vimgrep_pat(p, ptr::null_mut(), None);
            if p.is_null() {
                p = eap.arg;
            }
        }
        p
    }
}

pub fn replace_makeprg(
    eap: &mut exarg_T,
    mut arg: *mut c_char,
    cmdlinep: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        let isgrep = eap.cmdidx == cmdidx_T::CMD_grep as c_int
            || eap.cmdidx == cmdidx_T::CMD_lgrep as c_int
            || eap.cmdidx == cmdidx_T::CMD_grepadd as c_int
            || eap.cmdidx == cmdidx_T::CMD_lgrepadd as c_int;

        if (eap.cmdidx == cmdidx_T::CMD_make as c_int
            || eap.cmdidx == cmdidx_T::CMD_lmake as c_int
            || isgrep)
            && !(eap.cmdidx >= 0 && grep_internal(eap.cmdidx) != 0)
        {
            let program = if isgrep {
                if *(*g_st().curbuf).b_p_gp == NUL {
                    opt_vars().p_gp
                } else {
                    (*g_st().curbuf).b_p_gp
                }
            } else if *(*g_st().curbuf).b_p_mp == NUL {
                opt_vars().p_mp
            } else {
                (*g_st().curbuf).b_p_mp
            };

            arg = skipwhite(arg);

            let new_buf: Vec<c_char> = if let Some(rep) = strrep(std::ffi::CStr::from_ptr(program), c"$*", std::ffi::CStr::from_ptr(arg)) {
                rep.into_bytes_with_nul()
                    .into_iter()
                    .map(|b| b as c_char)
                    .collect()
            } else {
                let prog_b = crate::memory::cstr_as_bytes(program);
                let arg_b = crate::memory::cstr_as_bytes(arg);
                let mut combined = Vec::with_capacity(prog_b.len() + 1 + arg_b.len() + 1);
                combined.extend(prog_b.iter().map(|&b| b as c_char));
                combined.push(b' ' as c_char);
                combined.extend(arg_b.iter().map(|&b| b as c_char));
                combined.push(NUL);
                combined
            };

            msg_make(arg);

            eap.set_cmdline_vec(cmdlinep, new_buf);
            arg = *cmdlinep;
        }
        arg
    }
}

pub fn expand_filename(
    eap: &mut exarg_T,
    cmdlinep: *mut *mut c_char,
    errormsgp: *mut *const c_char,
) -> bool {
    unsafe {
        let mut p = skip_grep_pat(&mut *eap);
        let mut has_wildcards = path_has_wildcard(p, true);

        while *p != NUL {
            if *p == b'`' as c_char && *p.add(1) == b'=' as c_char {
                p = p.add(2);
                skip_expr(&raw mut p, None);
                if *p == b'`' as c_char {
                    p = p.add(1);
                }
                continue;
            }
            if vim_strchr(c"%#<", c_int::from(*p as u8)).is_null() {
                p = p.add(1);
                continue;
            }

            let mut srclen: usize = 0;
            let mut escaped: c_int = 0;
            let repl_owned = eval_vars(
                p,
                eap.arg,
                &mut srclen,
                Some(&mut eap.do_ecmd_lnum),
                errormsgp,
                Some(&mut escaped),
                true,
            );
            if !(*errormsgp).is_null() {
                return false;
            }
            let Some(mut repl_cstr) = repl_owned else {
                p = p.add(srclen);
                continue;
            };

            if !vim_strchr(repl_cstr.as_ptr(), b'$' as c_int).is_null()
                || !vim_strchr(repl_cstr.as_ptr(), b'~' as c_int).is_null()
            {
                repl_cstr = expand_env_save(repl_cstr.as_ptr().cast_mut());
            }

            if eap.usefilter == 0
                && escaped == 0
                && eap.cmdidx != cmdidx_T::CMD_bang as c_int
                && eap.cmdidx != cmdidx_T::CMD_grep as c_int
                && eap.cmdidx != cmdidx_T::CMD_grepadd as c_int
                && eap.cmdidx != cmdidx_T::CMD_lgrep as c_int
                && eap.cmdidx != cmdidx_T::CMD_lgrepadd as c_int
                && eap.cmdidx != cmdidx_T::CMD_lmake as c_int
                && eap.cmdidx != cmdidx_T::CMD_make as c_int
                && eap.cmdidx != cmdidx_T::CMD_terminal as c_int
                && (eap.argt & EX_NOSPC) == 0
            {
                #[cfg(windows)]
                let esc_chars = c" \t\"|".as_ptr();
                #[cfg(not(windows))]
                let esc_chars: *const c_char = escape_chars.as_ptr();

                let mut l = repl_cstr.as_ptr();
                while *l != NUL {
                    if !vim_strchr(esc_chars, c_int::from(*l as u8)).is_null() {
                        repl_cstr = vim_strsave_escaped(&repl_cstr, std::ffi::CStr::from_ptr(esc_chars));
                        break;
                    }
                    l = l.add(1);
                }
            }

            if (eap.usefilter != 0
                || eap.cmdidx == cmdidx_T::CMD_bang as c_int
                || eap.cmdidx == cmdidx_T::CMD_terminal as c_int)
                && repl_cstr.as_bytes().contains(&b'!')
            {
                repl_cstr = vim_strsave_escaped(&repl_cstr, c"!");
            }

            p = repl_cmdline(eap, p, srclen, repl_cstr.as_ptr().cast_mut(), cmdlinep);
        }

        if (eap.argt & EX_NOSPC) != 0 && eap.usefilter == 0 {
            if has_wildcards {
                if !vim_strchr(eap.arg, b'$' as c_int).is_null()
                    || !vim_strchr(eap.arg, b'~' as c_int).is_null()
                {
                    #[cfg(windows)]
                    let esc_wildcards = c" \t*?[".as_ptr().cast_mut();
                    #[cfg(not(windows))]
                    let esc_wildcards = c" \t*?[{".as_ptr().cast_mut();
                    expand_env_esc(
                        eap.arg,
                        (&raw mut g_st().NameBuff).cast(),
                        MAXPATHL as c_int,
                        esc_wildcards,
                        true,
                        ptr::null_mut(),
                    );
                    has_wildcards = path_has_wildcard((&raw const g_st().NameBuff).cast(), true);
                    p = (&raw mut g_st().NameBuff).cast();
                } else {
                    p = ptr::null_mut();
                }
                if !p.is_null() {
                    repl_cmdline(eap, eap.arg, crate::memory::cstr_len(eap.arg), p, cmdlinep);
                }
            }

            #[cfg(unix)]
            if !has_wildcards {
                backslash_halve(eap.arg);
            }
            #[cfg(not(unix))]
            backslash_halve(eap.arg);

            if has_wildcards {
                let mut xpc: expand_T = expand_T::new();
                let mut options = WILD_LIST_NOTFOUND | WILD_NOERROR | WILD_ADD_SLASH;

                xpc.xp_context = EXPAND_FILES;
                if opt_vars().p_wic != 0 {
                    options += WILD_ICASE;
                }
                let Some(expanded) = ExpandOne(
                    &mut xpc,
                    eap.arg,
                    None,
                    options,
                    WILD_EXPAND_FREE,
                ) else {
                    return false;
                };
                repl_cmdline(
                    eap,
                    eap.arg,
                    crate::memory::cstr_len(eap.arg),
                    expanded.as_ptr().cast_mut(),
                    cmdlinep,
                );
            } else {
                to_slash(eap.arg);
            }
        }
        true
    }
}

fn repl_cmdline(
    eap: &mut exarg_T,
    mut src: *mut c_char,
    srclen: usize,
    repl: *mut c_char,
    cmdlinep: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        let len = crate::memory::cstr_len(repl);
        let offset = (src as usize) - ((*cmdlinep) as usize);
        let tail_len = crate::memory::cstr_len(src.add(srclen));
        let nextcmd_len = if !eap.nextcmd.is_null() {
            crate::memory::cstr_len(eap.nextcmd)
        } else {
            0
        };
        let total = offset + tail_len + len + 3 + nextcmd_len;
        let mut new_buf: Vec<c_char> = vec![0; total];
        let new_cmdline = new_buf.as_mut_ptr();

        let mut i = offset;
        ptr::copy_nonoverlapping(*cmdlinep, new_cmdline, i);
        ptr::copy_nonoverlapping(repl, new_cmdline.add(i), len);
        i += len;
        crate::memory::cstr_cpy(new_cmdline.add(i), src.add(srclen));
        src = new_cmdline.add(i);

        if !eap.nextcmd.is_null() {
            i = crate::memory::cstr_len(new_cmdline) + 1;
            crate::memory::cstr_cpy(new_cmdline.add(i), eap.nextcmd);
            eap.nextcmd = new_cmdline.add(i);
        }
        eap.cmd =
            new_cmdline.wrapping_add((eap.cmd as usize).wrapping_sub((*cmdlinep) as usize));
        eap.arg =
            new_cmdline.wrapping_add((eap.arg as usize).wrapping_sub((*cmdlinep) as usize));

        for arg_j in &mut eap.args {
            let arg_off = (*arg_j as usize).wrapping_sub((*cmdlinep) as usize);
            if offset >= arg_off {
                *arg_j = new_cmdline.wrapping_add(arg_off);
            } else {
                *arg_j = new_cmdline.wrapping_add(arg_off.wrapping_add(len).wrapping_sub(srclen));
            }
        }

        if !eap.do_ecmd_cmd.is_null()
            && eap.do_ecmd_cmd != (&raw mut docmd_st().dollar_command).cast::<c_char>()
        {
            eap.do_ecmd_cmd = new_cmdline
                .wrapping_add((eap.do_ecmd_cmd as usize).wrapping_sub((*cmdlinep) as usize));
        }
        eap.set_cmdline_vec(cmdlinep, new_buf);

        src
    }
}

pub fn separate_nextcmd(eap: &mut exarg_T) {
    unsafe {
        let mut p = skip_grep_pat(eap);

        while *p != NUL {
            if *p == Ctrl_V {
                if (eap.argt & (EX_CTRLV | EX_XFILE)) != 0 {
                    p = p.add(1);
                } else {
                    ptr::copy(p.add(1), p, std::ffi::CStr::from_ptr(p.add(1)).to_bytes_with_nul().len());
                }
                if *p == NUL {
                    break;
                }
            } else if *p == b'`' as c_char
                && *p.add(1) == b'=' as c_char
                && (eap.argt & EX_XFILE) != 0
            {
                p = p.add(2);
                skip_expr(&raw mut p, None);
                if *p == NUL {
                    break;
                }
            } else if (*p == b'"' as c_char
                && (eap.argt & EX_NOTRLCOM) == 0
                && (eap.cmdidx != cmdidx_T::CMD_at as c_int || p != eap.arg)
                && (eap.cmdidx != cmdidx_T::CMD_redir as c_int
                    || p != eap.arg.add(1)
                    || *p.sub(1) != b'@' as c_char))
                || (*p == b'|' as c_char
                    && eap.cmdidx != cmdidx_T::CMD_append as c_int
                    && eap.cmdidx != cmdidx_T::CMD_change as c_int
                    && eap.cmdidx != cmdidx_T::CMD_insert as c_int)
                || *p == b'\n' as c_char
            {
                if (vim_strchr(opt_vars().p_cpo, kCpoBar).is_null() || (eap.argt & EX_CTRLV) == 0)
                    && *p.sub(1) == b'\\' as c_char
                {
                    ptr::copy(p, p.sub(1), CStr::from_ptr(p).to_bytes_with_nul().len());
                    p = p.sub(1);
                } else {
                    eap.nextcmd = check_nextcmd(p);
                    *p = NUL;
                    break;
                }
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        if (eap.argt & EX_NOTRLCOM) == 0 {
            del_trailing_spaces(eap.arg);
        }
    }
}

pub fn getargcmd(argp: *mut *mut c_char) -> *mut c_char {
    unsafe {
        let mut arg = *argp;
        let mut command: *mut c_char = ptr::null_mut();

        if *arg == b'+' as c_char {
            arg = arg.add(1);
            if ascii_isspace(*arg) || *arg == NUL {
                command = (&raw mut docmd_st().dollar_command).cast();
            } else {
                command = arg;
                arg = skip_cmd_arg(command, true);
                if *arg != NUL {
                    *arg = NUL;
                    arg = arg.add(1);
                }
            }
            arg = skipwhite(arg);
            *argp = arg;
        }
        command
    }
}

pub fn skip_cmd_arg(mut p: *mut c_char, rembs: bool) -> *mut c_char {
    unsafe {
        while *p != NUL && !ascii_isspace(*p) {
            if *p == b'\\' as c_char && *p.add(1) != NUL {
                if rembs {
                    ptr::copy(p.add(1), p, std::ffi::CStr::from_ptr(p.add(1)).to_bytes_with_nul().len());
                } else {
                    p = p.add(1);
                }
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        p
    }
}

pub fn get_bad_opt(p: *const c_char, eap: &mut exarg_T) -> bool {
    unsafe {
        if crate::memory::cstr_as_bytes(p).eq_ignore_ascii_case(b"keep") {
            eap.bad_char = BAD_KEEP;
        } else if crate::memory::cstr_as_bytes(p).eq_ignore_ascii_case(b"drop") {
            eap.bad_char = BAD_DROP;
        } else if utfc_ptr2len(p) == 1 && *p.add(1) == NUL {
            eap.bad_char = c_int::from(*p as u8);
        } else {
            return false;
        }
        true
    }
}

fn get_bad_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    const P_BAD_VALUES: [&CStr; 3] = [c"?", c"keep", c"drop"];
    if idx >= 0 && (idx as usize) < P_BAD_VALUES.len() {
        P_BAD_VALUES[idx as usize].as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

pub fn getargopt(eap: &mut exarg_T) -> bool {
    unsafe {
        let mut arg = eap.arg.add(2);
        let mut pp: *mut c_int = ptr::null_mut();
        let mut bad_char_idx: c_int = 0;

        if (crate::memory::cstr_prefix_bytes(arg, 3) == &b"bin"[..3]) || (crate::memory::cstr_prefix_bytes(arg, 5) == &b"nobin"[..5]) {
            if *arg == b'n' as c_char {
                arg = arg.add(2);
                eap.force_bin = FORCE_NOBIN;
            } else {
                eap.force_bin = FORCE_BIN;
            }
            if !checkforcmd(&raw mut arg, c"binary", 3) {
                return false;
            }
            eap.arg = skipwhite(arg);
            return true;
        }

        if (crate::memory::cstr_prefix_bytes(arg, 4) == &b"edit"[..4]) && !ascii_isalpha(*arg.add(4)) {
            eap.read_edit = c_int::from(true);
            eap.arg = skipwhite(arg.add(4));
            return true;
        }

        if *arg == b'p' as c_char && !ascii_isalpha(*arg.add(1)) {
            eap.mkdir_p = c_int::from(true);
            eap.arg = skipwhite(arg.add(1));
            return true;
        }

        if crate::memory::cstr_prefix_bytes(arg, 2) == &b"ff"[..2] {
            arg = arg.add(2);
            pp = &raw mut eap.force_ff;
        } else if crate::memory::cstr_prefix_bytes(arg, 10) == &b"fileformat"[..10] {
            arg = arg.add(10);
            pp = &raw mut eap.force_ff;
        } else if crate::memory::cstr_prefix_bytes(arg, 3) == &b"enc"[..3] {
            if crate::memory::cstr_prefix_bytes(arg, 8) == &b"encoding"[..8] {
                arg = arg.add(8);
            } else {
                arg = arg.add(3);
            }
            pp = &raw mut eap.force_enc;
        } else if crate::memory::cstr_prefix_bytes(arg, 3) == &b"bad"[..3] {
            arg = arg.add(3);
            pp = &raw mut bad_char_idx;
        }

        if pp.is_null() || *arg != b'=' as c_char {
            return false;
        }

        arg = arg.add(1);
        *pp = ((arg as usize) - (eap.cmd as usize)) as c_int;
        arg = skip_cmd_arg(arg, false);
        eap.arg = skipwhite(arg);
        *arg = NUL;

        if ptr::eq(pp, &raw mut eap.force_ff) {
            if !check_ff_value(eap.cmd.add(eap.force_ff as usize)) {
                return false;
            }
            eap.force_ff = c_int::from(*eap.cmd.add(eap.force_ff as usize) as u8);
        } else if ptr::eq(pp, &raw mut eap.force_enc) {
            let mut p = eap.cmd.add(eap.force_enc as usize);
            while *p != NUL {
                *p = tolower_asc(c_int::from(*p as u8)) as c_char;
                p = p.add(1);
            }
        } else if !get_bad_opt(eap.cmd.add(bad_char_idx as usize), eap) {
            return false;
        }

        true
    }
}

fn get_argopt_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    const P_OPT_VALUES: [&CStr; 7] = [
        c"fileformat=",
        c"encoding=",
        c"binary",
        c"nobinary",
        c"bad=",
        c"edit",
        c"p",
    ];
    if idx >= 0 && (idx as usize) < P_OPT_VALUES.len() {
        P_OPT_VALUES[idx as usize].as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

pub fn expand_argopt(
    pat: *mut c_char,
    xp: &mut expand_T,
    rmp: &mut regmatch_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        if xp.xp_pattern > xp.xp_line && *xp.xp_pattern.sub(1) == b'=' as c_char {
            let mut cb: CompleteListItemGetter = None;
            let name_end = xp.xp_pattern.sub(1);
            let diff = (name_end as usize) - (xp.xp_line as usize);
            if (diff >= 2 && (crate::memory::cstr_prefix_bytes(name_end.sub(2), 2) == &b"ff"[..2]))
                || (diff >= 10 && (crate::memory::cstr_prefix_bytes(name_end.sub(10), 10) == &b"fileformat"[..10]))
            {
                cb = Some(get_fileformat_name);
            } else if (diff >= 3 && (crate::memory::cstr_prefix_bytes(name_end.sub(3), 3) == &b"enc"[..3]))
                || (diff >= 8 && (crate::memory::cstr_prefix_bytes(name_end.sub(8), 8) == &b"encoding"[..8]))
            {
                cb = Some(get_encoding_name);
            } else if diff >= 3 && (crate::memory::cstr_prefix_bytes(name_end.sub(3), 3) == &b"bad"[..3]) {
                cb = Some(get_bad_name);
            }

            if cb.is_some() {
                ExpandGeneric(pat, xp, rmp, matches, cb, false);
                return true;
            }
            return false;
        }

        if xp.xp_pattern_len == 2
            && (crate::memory::cstr_prefix_bytes(xp.xp_pattern, xp.xp_pattern_len) == crate::memory::cstr_prefix_bytes(c"ff", xp.xp_pattern_len))
        {
            *matches = vec![c"fileformat=".to_owned()];
            return true;
        }

        ExpandGeneric(pat, xp, rmp, matches, Some(get_argopt_name), false);
        true
    }
}

fn get_tabpage_arg(eap: &mut exarg_T) -> c_int {
    unsafe {
        let mut tab_number: c_int = 0;
        let unaccept_arg0 = c_int::from(eap.cmdidx != cmdidx_T::CMD_tabmove as c_int);
        let last_tab = current_tab_nr(ptr::null_mut());

        'theend: {
            if !eap.arg.is_null() && *eap.arg != NUL {
                let mut p = eap.arg;
                let mut relative: c_int = 0;

                if *p == b'-' as c_char {
                    relative = -1;
                    p = p.add(1);
                } else if *p == b'+' as c_char {
                    relative = 1;
                    p = p.add(1);
                }

                let p_save = p;
                tab_number = getdigits(&raw mut p, false, i64::from(tab_number)) as c_int;

                if relative == 0 {
                    if crate::memory::cstr_as_bytes(p) == b"$" {
                        tab_number = last_tab;
                    } else if crate::memory::cstr_as_bytes(p) == b"#" {
                        if valid_tabpage(g_st().lastused_tabpage) {
                            tab_number = tabpage_index(g_st().lastused_tabpage);
                        } else {
                            eap.errmsg = ex_errmsg(e_invargval, eap.arg);
                            tab_number = 0;
                            break 'theend;
                        }
                    } else if p == p_save
                        || *p_save == b'-' as c_char
                        || *p != NUL
                        || tab_number > last_tab
                    {
                        eap.errmsg = ex_errmsg(e_invarg2, eap.arg);
                        break 'theend;
                    }
                } else {
                    if *p_save == NUL {
                        tab_number = 1;
                    } else if p == p_save
                        || *p_save == b'-' as c_char
                        || *p != NUL
                        || tab_number == 0
                    {
                        eap.errmsg = ex_errmsg(e_invarg2, eap.arg);
                        break 'theend;
                    }
                    tab_number = tab_number * relative + tabpage_index(g_st().curtab);
                    if unaccept_arg0 == 0 && relative == -1 {
                        tab_number -= 1;
                    }
                }
                if tab_number < unaccept_arg0 || tab_number > last_tab {
                    eap.errmsg = ex_errmsg(e_invarg2, eap.arg);
                }
            } else if eap.addr_count > 0 {
                if unaccept_arg0 != 0 && eap.line2 == 0 {
                    eap.errmsg = gettext(e_invrange);
                    tab_number = 0;
                } else {
                    tab_number = eap.line2;
                    if unaccept_arg0 == 0 {
                        let mut cmdp = eap.cmd;
                        while {
                            cmdp = cmdp.sub(1);
                            cmdp > *eap.cmdlinep
                                && (ascii_iswhite(*cmdp) || ascii_isdigit(*cmdp))
                        } {}
                        if *cmdp == b'-' as c_char {
                            tab_number -= 1;
                            if tab_number < unaccept_arg0 {
                                eap.errmsg = gettext(e_invrange);
                            }
                        }
                    }
                }
            } else if eap.cmdidx == cmdidx_T::CMD_tabnext as c_int {
                tab_number = tabpage_index(g_st().curtab) + 1;
                if tab_number > last_tab {
                    tab_number = 1;
                }
            } else if eap.cmdidx == cmdidx_T::CMD_tabmove as c_int {
                tab_number = last_tab;
            } else {
                tab_number = tabpage_index(g_st().curtab);
            }
        }

        tab_number
    }
}

fn ex_autocmd(eap: &mut exarg_T) {
    if g_st().secure != 0 {
            g_st().secure = 2;
            eap.errmsg = gettext(e_curdir);
        } else if eap.cmdidx == cmdidx_T::CMD_autocmd as c_int {
            do_autocmd(eap, eap.arg, eap.forceit);
        } else {
            do_augroup(eap.arg, eap.forceit != 0);
        }
}

fn ex_doautocmd(eap: &mut exarg_T) {
    let mut arg = eap.arg;
        let call_do_modelines = check_nomodeline(&raw mut arg);
        let mut did_aucmd = false;

        do_doautocmd(arg, false, Some(&mut did_aucmd));
        if call_do_modelines && did_aucmd {
            do_modelines(0);
        }
}

fn ex_bunload(eap: &mut exarg_T) {
    let cmd = if eap.cmdidx == cmdidx_T::CMD_bdelete as c_int {
            DOBUF_DEL
        } else if eap.cmdidx == cmdidx_T::CMD_bwipeout as c_int {
            DOBUF_WIPE
        } else {
            DOBUF_UNLOAD
        };
        eap.errmsg = do_bufdel(
            cmd,
            eap.arg,
            eap.addr_count,
            eap.line1,
            eap.line2,
            eap.forceit,
        );
}

fn ex_buffer(eap: &mut exarg_T) {
    do_exbuffer(eap);
}

fn do_exbuffer(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg != NUL {
            eap.errmsg = ex_errmsg(e_trailing_arg, eap.arg);
        } else {
            if eap.addr_count == 0 {
                goto_buffer(eap, DOBUF_CURRENT, FORWARD, 0);
            } else {
                goto_buffer(eap, DOBUF_FIRST, FORWARD, eap.line2);
            }
            if !eap.do_ecmd_cmd.is_null() {
                do_cmdline_cmd(eap.do_ecmd_cmd);
            }
        }
    }
}

fn ex_bmodified(eap: &mut exarg_T) {
    goto_buffer(eap, DOBUF_MOD, FORWARD, eap.line2);
        if !eap.do_ecmd_cmd.is_null() {
            do_cmdline_cmd(eap.do_ecmd_cmd);
        }
}

fn ex_bnext(eap: &mut exarg_T) {
    goto_buffer(eap, DOBUF_CURRENT, FORWARD, eap.line2);
        if !eap.do_ecmd_cmd.is_null() {
            do_cmdline_cmd(eap.do_ecmd_cmd);
        }
}

fn ex_bprevious(eap: &mut exarg_T) {
    goto_buffer(eap, DOBUF_CURRENT, BACKWARD, eap.line2);
        if !eap.do_ecmd_cmd.is_null() {
            do_cmdline_cmd(eap.do_ecmd_cmd);
        }
}

fn ex_brewind(eap: &mut exarg_T) {
    goto_buffer(eap, DOBUF_FIRST, FORWARD, 0);
        if !eap.do_ecmd_cmd.is_null() {
            do_cmdline_cmd(eap.do_ecmd_cmd);
        }
}

fn ex_blast(eap: &mut exarg_T) {
    goto_buffer(eap, DOBUF_LAST, BACKWARD, 0);
        if !eap.do_ecmd_cmd.is_null() {
            do_cmdline_cmd(eap.do_ecmd_cmd);
        }
}

pub fn ends_excmd(c: c_int) -> c_int {
    c_int::from(
        c == c_int::from(NUL) || c == b'|' as c_int || c == b'"' as c_int || c == b'\n' as c_int,
    )
}

pub fn find_nextcmd(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while *p != b'|' as c_char && *p != b'\n' as c_char {
            if *p == NUL {
                return ptr::null_mut();
            }
            p = p.add(1);
        }
        p.add(1).cast_mut()
    }
}

pub fn check_nextcmd(p: *mut c_char) -> *mut c_char {
    unsafe {
        let s = skipwhite(p);
        if *s == b'|' as c_char || *s == b'\n' as c_char {
            return s.add(1);
        }
        ptr::null_mut()
    }
}

fn check_more(message: bool, forceit: bool) -> bool {
    unsafe {
        let ac = argcount();
        let n = ac - (**curwin.as_ptr()).w_arg_idx - 1;

        if !forceit && only_one_window() && ac > 1 && !g_st().arg_had_last && n > 0 && docmd_st().quitmore == 0 {
            if message {
                if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                    && !(*g_st().curbuf).b_fname.is_null()
                {
                    let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
                    let _ = vim_snprintf!(
                        buff.as_mut_ptr(),
                        DIALOG_MSG_SIZE,
                        ngettext(
                            c"%d more file to edit.  Quit anyway?",
                            c"%d more files to edit.  Quit anyway?",
                            n as c_long,
                        ),
                        n,
                    );
                    if vim_dialog_yesno(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 1)
                        == VIM_YES
                    {
                        return true;
                    }
                    return false;
                }
                let _ = semsg!(
                    ngettext(
                        c"E173: %d more file to edit",
                        c"E173: %d more files to edit",
                        n as c_long,
                    ),
                    n,
                );
                docmd_st().quitmore = 2;
            }
            return false;
        }
        true
    }
}

pub fn get_command_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx >= CMD_SIZE_I32 {
            return expand_user_command_name(idx);
        }
        CMDNAMES[idx as usize].cmd_name.as_ptr().cast_mut()
}

fn ex_colorscheme(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg == NUL {
            let mut expr = *b"g:colors_name\0";
            g_st().emsg_off += 1;
            let p = eval_to_string(expr.as_mut_ptr().cast(), false, false);
            g_st().emsg_off -= 1;

            msg_ext_set_kind(c"list_cmd");
            if let Some(ref p) = p {
                let _ = msg(p.as_ptr(), 0);
            } else {
                let _ = msg(c"default", 0);
            }
        } else if !load_colors(eap.arg) {
            let _ = semsg!(
                gettext(c"E185: Cannot find color scheme '%s'"),
                eap.arg,
            );
        }
    }
}

fn ex_highlight(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg == NUL && *eap.cmd.add(2) == b'!' as c_char {
            let _ = msg(gettext(c"Greetings, Vim user!"), 0);
        }
        do_highlight(eap.arg, eap.forceit != 0, false);
    }
}

pub fn not_exiting(save_exiting: bool) {
    g_st().exiting = save_exiting;
        set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
}

pub fn before_quit_autocmds(wp: *mut win_T, quit_all: bool, forceit: bool) -> bool {
    unsafe {
        if *get_vim_var_str(VV_EXITREASON) == NUL {
            set_vim_var_string(VV_EXITREASON, c"quit".as_ptr(), 4);
        }
        apply_autocmds_win(
            EVENT_QUITPRE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            (*wp).w_buffer,
            wp,
        );

        if !win_valid(wp)
            || curbuf_locked()
            || ((*(*wp).w_buffer).b_nwindows == 1 && (*(*wp).w_buffer).b_locked > 0)
        {
            set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
            return true;
        }

        if quit_all || (check_more(false, forceit) && only_one_window()) {
            apply_autocmds(
                EVENT_EXITPRE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if !win_valid(wp)
                || curbuf_locked()
                || ((*g_st().curbuf).b_nwindows == 1 && (*g_st().curbuf).b_locked > 0)
            {
                set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
                return true;
            }
        }

        false
    }
}

fn ex_quit(eap: &mut exarg_T) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }

        let mut wp: *mut win_T;
        if eap.addr_count > 0 {
            let mut wnr = eap.line2;
            wp = g_st().firstwin;
            while !(*wp).w_next.is_null() {
                wnr -= 1;
                if wnr <= 0 {
                    break;
                }
                wp = (*wp).w_next;
            }
        } else {
            wp = *curwin.as_ptr();
        }

        if curbuf_locked() {
            return;
        }

        if before_quit_autocmds(wp, false, eap.forceit != 0) {
            return;
        }

        let save_exiting = g_st().exiting;
        if check_more(false, eap.forceit != 0) && only_one_window() {
            g_st().exiting = true;
        }
        if (!buf_hide(&*(*wp).w_buffer)
            && check_changed(
                &mut *(*wp).w_buffer,
                (if opt_vars().p_awa != 0 { CCGD_AW } else { 0 })
                    | (if eap.forceit != 0 { CCGD_FORCEIT } else { 0 })
                    | CCGD_EXCMD,
            ))
            || !check_more(true, eap.forceit != 0)
            || (only_one_window() && check_changed_any(eap.forceit != 0, true))
        {
            not_exiting(save_exiting);
        } else {
            if only_one_window() && (g_st().firstwin == g_st().lastwin || eap.addr_count == 0) {
                getout(0);
            }
            not_exiting(save_exiting);
            win_close(
                wp,
                !buf_hide(&*(*wp).w_buffer) || eap.forceit != 0,
                eap.forceit != 0,
            );
        }
    }
}

fn ex_cquit(eap: &mut exarg_T) {
    let status = if eap.addr_count > 0 {
            eap.line2
        } else {
            1
        };
        ui_call_error_exit(Integer::from(status));
        getout(status);
}

pub fn before_quit_all(eap: &mut exarg_T) -> bool {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return false;
        }
        if before_quit_autocmds(*curwin.as_ptr(), true, eap.forceit != 0) {
            return false;
        }
        true
    }
}

fn ex_quitall(eap: &mut exarg_T) {
    if !before_quit_all(eap) {
            return;
        }
        let save_exiting = g_st().exiting;
        g_st().exiting = true;
        if eap.forceit != 0 || !check_changed_any(false, false) {
            getout(0);
        }
        not_exiting(save_exiting);
}

fn ex_exmode(_eap: &mut exarg_T) {
    if g_st().silent_mode {
            return;
        }
        if g_st().ex_normal_busy > 0 || g_st().global_busy != 0 {
            flush_buffers(FLUSH_TYPEAHEAD);
            return;
        }
        let mut args = [typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        }];
        nlua_call_typval(
            c"vim._core.exmode",
            c"open",
            args.as_mut_ptr(),
            None,
        );
}

fn ex_restart(eap: &mut exarg_T) {
    unsafe {
        if eap.forceit == 0 {
            let extra_d = tv_dict_alloc();
            tv_dict_add_str(
                extra_d,
                c"quit_cmd",
                if !eap.do_ecmd_cmd.is_null() {
                    eap.do_ecmd_cmd
                } else {
                    c"".as_ptr()
                },
            );
            let mut extra_tv = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: 0,
                vval: TypvalVval::Dict(extra_d),
            };
            nlua_call_excmd(
                c"vim._core.server",
                c"ex_session_restart",
                eap,
                &g_st().cmdmod,
                Some(&mut extra_tv),
            );
            tv_clear(&raw mut extra_tv);
            return;
        }

        let mut startreason = c"restart!".as_ptr();
        let mut quit_cmd = if !eap.do_ecmd_cmd.is_null() {
            eap.do_ecmd_cmd
        } else {
            c"qall".as_ptr().cast_mut()
        };
        let mut after_cmd = eap.arg;

        if crate::memory::cstr_as_opt_bytes(quit_cmd) == Some(b":::") {
            startreason = c"restart".as_ptr();
            if eap.args.len() > 1 {
                let arg1 = eap.args.as_slice()[1];
                let arglen1 = eap.arglens.as_slice()[1];
                *arg1.add(arglen1) = NUL;
                quit_cmd = arg1;
                after_cmd = if eap.args.len() > 2 {
                    eap.args.as_slice()[2]
                } else {
                    c"".as_ptr().cast_mut()
                };
            } else {
                let _ = semsg!(e_restart_failed_cmd_no_quit, quit_cmd);
                return;
            }
        }

        let mut err = error_init();
        let no_ui = ui_active() == 0;
        let exepath = get_vim_var_str(VV_PROGPATH);
        let l = get_vim_var_list(VV_ARGV);
        let argc = if l.is_null() { 0 } else { (*l).lv_len };

        let mut argv_vec = Vec::<std::ffi::CString>::with_capacity((argc as usize) + 2);
        let mut listen_arg: Option<std::ffi::CString> = None;

        let mut li = if l.is_null() {
            ptr::null_mut()
        } else {
            (*l).lv_first
        };
        while !li.is_null() {
            let arg = tv_get_string(&(*li).li_tv);
            let i = argv_vec.len();
            if i > 0 && (crate::memory::cstr_as_opt_bytes(arg) == Some(b"--")) {
                break;
            }
            if i > 0 && (crate::memory::cstr_as_opt_bytes(arg) == Some(b"-S")) {
                if !(*li).li_next.is_null() {
                    let next_arg = tv_get_string(&(*(*li).li_next).li_tv);
                    if *next_arg != b'-' as c_char {
                        li = (*li).li_next;
                    }
                }
                li = (*li).li_next;
                continue;
            }
            if i > 0 && (crate::memory::cstr_as_opt_bytes(arg) == Some(b"-s")) {
                li = (*li).li_next;
                if !li.is_null() {
                    li = (*li).li_next;
                }
                continue;
            }
            if i > 0 && (crate::memory::cstr_as_opt_bytes(arg) == Some(b"--listen")) {
                let next_li = (*li).li_next;
                if !next_li.is_null() {
                    let addr = tv_get_string(&(*next_li).li_tv);
                    if crate::memory::cstr_as_bytes(addr)
                        .iter()
                        .any(|b| matches!(b, b':' | b'/' | b'\\'))
                    {
                        listen_arg = to_slash_save(addr);
                        #[cfg(windows)]
                        {
                            li = (*next_li).li_next;
                            continue;
                        }
                    }
                }
            }
            if i == 0
                || (!(crate::memory::cstr_as_opt_bytes(arg) == Some(b"--embed"))
                    && !(crate::memory::cstr_as_opt_bytes(arg) == Some(b"--headless"))
                    && !(crate::memory::cstr_as_opt_bytes(arg) == Some(b"-")))
            {
                argv_vec.push(crate::memory::cstr_to_cstring(arg));
                if argv_vec.len() == 1 {
                    argv_vec.push(c"--embed".to_owned());
                    if no_ui {
                        argv_vec.push(c"--headless".to_owned());
                    }
                }
            }
            li = (*li).li_next;
        }

        #[cfg(windows)]
        let server_stopped = {
            if listen_arg.is_none() {
                let mut lua_err = error_init();
                let rv = nlua_exec(
                    cstr_as_string(
                        c"return require('vim._core.server').restart_canonical_addr",
                    ),
                    ptr::null(),
                    Array {
                        items: ptr::null_mut(),
                        size: 0,
                        capacity: 0,
                    },
                    kRetObject,
                    None,
                    &mut lua_err,
                );
                if !error_set(&lua_err)
                    && rv.obj_type == ObjectType::kObjectTypeString
                    && rv.data.string().size > 0
                {
                    listen_arg = Some(cstr_to_cstring(rv.data.string().data));
                }
                api_free_object(rv);
                api_clear_error(&mut lua_err);
            }
            false
        };
        #[cfg(not(windows))]
        let server_stopped = if let Some(ref la) = listen_arg {
            server_stop(la.as_ptr().cast_mut(), true)
        } else {
            false
        };

        let env = create_environment(ptr::null(), false, false, false, ptr::null());
        tv_dict_add_str(
            env,
            ENV_STARTREASON,
            startreason,
        );

        let on_stdout = CallbackReader::none();
        let mut on_err = CallbackReader::none();
        #[cfg(windows)]
        {
            on_err.fwd_err = false;
        }
        #[cfg(not(windows))]
        {
            on_err.fwd_err = true;
        }
        let on_exit = Callback::NONE;
        let mut exit_status: varnumber_T = 0;

        let channel = channel_job_start(
            argv_vec,
            exepath,
            on_stdout,
            on_err,
            on_exit,
            false,
            true,
            true,
            true,
            kChannelStdinPipe,
            (&raw const g_st().globaldir)
                .as_ref()
                .unwrap()
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr()),
            0,
            0,
            env,
            &raw mut exit_status,
        );
        if channel.is_null() {
            let _ = emsg(c"cannot create a channel job");
            let la_ptr = listen_arg.as_ref().map_or(ptr::null(), |s| s.as_ptr());
            if server_stopped && server_start(la_ptr) != 0 {
                let _ = semsg!(c"couldn't resume listening on %s", la_ptr);
            }
            return;
        }

        let chan_id = (*channel).id;
        let mut result_mem: ArenaMem = ptr::null_mut();

        'fail_2: {
            let mut detach_kv = [KeyValuePair {
                key: cstr_as_string(c"detach"),
                value: boolean_obj(true),
            }];
            let detach_opts = Dict {
                items: detach_kv.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            let mut detach_items = [
                integer_obj(0),
                Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData::Dict(detach_opts),
                },
            ];
            let detach_args = Array {
                items: detach_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            rpc_send_call(
                chan_id,
                c"nvim_chan_set".as_ptr(),
                detach_args,
                &mut result_mem,
                &mut err,
            );
            if error_set(&err) {
                break 'fail_2;
            }
            arena_mem_free(result_mem);
            result_mem = ptr::null_mut();

            if *after_cmd != NUL {
                let mut autocmd_kv = [
                    KeyValuePair {
                        key: cstr_as_string(c"once"),
                        value: boolean_obj(true),
                    },
                    KeyValuePair {
                        key: cstr_as_string(c"nested"),
                        value: boolean_obj(true),
                    },
                    KeyValuePair {
                        key: cstr_as_string(c"command"),
                        value: cstr_as_obj(after_cmd),
                    },
                ];
                let autocmd_opts = Dict {
                    items: autocmd_kv.as_mut_ptr(),
                    size: 3,
                    capacity: 3,
                };
                let mut autocmd_items = [
                    cstr_as_obj(c"UIEnter"),
                    Object {
                        obj_type: ObjectType::kObjectTypeDict,
                        data: ObjectData::Dict(autocmd_opts),
                    },
                ];
                let autocmd_args = Array {
                    items: autocmd_items.as_mut_ptr(),
                    size: 2,
                    capacity: 2,
                };
                rpc_send_call(
                    chan_id,
                    c"nvim_create_autocmd".as_ptr(),
                    autocmd_args,
                    &mut result_mem,
                    &mut err,
                );
                if error_set(&err) {
                    break 'fail_2;
                }
                arena_mem_free(result_mem);
                result_mem = ptr::null_mut();
            }

            let mut servername_items = [cstr_as_obj(c"servername")];
            let servername_args = Array {
                items: servername_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            let result = rpc_send_call(
                chan_id,
                c"nvim_get_vvar".as_ptr(),
                servername_args,
                &mut result_mem,
                &mut err,
            );
            if error_set(&err) {
                break 'fail_2;
            }
            if result.obj_type != ObjectType::kObjectTypeString || result.data.string().size == 0 {
                let _ =
                    emsg(c"restart failed: could not get listen address from new server");
                break 'fail_2;
            }
            let listen_addr = cstr_from_bytes(std::slice::from_raw_parts(
                result.data.string().data.cast::<u8>(),
                result.data.string().size,
            ));
            arena_mem_free(result_mem);
            result_mem = ptr::null_mut();

            #[cfg(windows)]
            if let Some(ref la) = listen_arg {
                let mut handoff_items =
                    [cstr_as_obj(la.as_ptr()), integer_obj(ui_active() as Integer)];
                let handoff_params = Array {
                    items: handoff_items.as_mut_ptr(),
                    size: 2,
                    capacity: 3,
                };
                let mut lua_items = [
                    cstr_as_obj(
                        c"return require('vim._core.server').rebind_after_restart(...)",
                    ),
                    Object {
                        obj_type: ObjectType::kObjectTypeArray,
                        data: ObjectData::Array(handoff_params),
                    },
                ];
                let lua_args = Array {
                    items: lua_items.as_mut_ptr(),
                    size: 2,
                    capacity: 2,
                };
                rpc_send_call(
                    chan_id,
                    c"nvim_exec_lua".as_ptr(),
                    lua_args,
                    &mut result_mem,
                    &mut err,
                );
                if error_set(&err) {
                    break 'fail_2;
                }
                arena_mem_free(result_mem);
                result_mem = ptr::null_mut();
            }

            remote_ui_restart(cstr_as_string(listen_addr.as_ptr()));
            ui_flush();
            drop(listen_addr);

            set_vim_var_string(VV_EXITREASON, startreason, -1);

            let mut quit_cmd_copy: Option<std::ffi::CString> = None;
            if (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                let mut bytes = Vec::from(&b"confirm "[..]);
                bytes.extend_from_slice(crate::memory::cstr_as_bytes(quit_cmd));
                let cstr = cstr_from_bytes(&bytes);
                quit_cmd = cstr.as_ptr().cast_mut();
                quit_cmd_copy = Some(cstr);
            }
            nvim_command(cstr_as_string(quit_cmd), &mut err);

            if error_set(&err) {
                let _ = emsg(err.msg_ptr());
                api_clear_error(&mut err);
            } else if !g_st().exiting {
                let _ = semsg!(e_restart_failed_cmd_no_quit, quit_cmd);
            }
            drop(quit_cmd_copy);
        }

        set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
        if error_set(&err) {
            let _ = emsg(err.msg_ptr());
            api_clear_error(&mut err);
        }
        arena_mem_free(result_mem);

        #[cfg(not(windows))]
        {
            result_mem = ptr::null_mut();
            let mut chanclose_items = [cstr_as_obj(c"chanclose(v:stderr)")];
            let chanclose_args = Array {
                items: chanclose_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            rpc_send_call(
                chan_id,
                c"nvim_eval".as_ptr(),
                chanclose_args,
                &mut result_mem,
                &mut err,
            );
            api_clear_error(&mut err);
            arena_mem_free(result_mem);
        }

        let proc_ptr = &raw mut (*channel).uv.proc_;
        proc_stop(&mut *proc_ptr);
        if proc_wait(proc_ptr, -1, ptr::null_mut()) < 0 {
            let _ = emsg(c"killing new nvim server failed");
        }

        let la_ptr = listen_arg.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        if server_stopped && server_start(la_ptr) != 0 {
            let _ = semsg!(c"couldn't resume listening on %s", la_ptr);
        }
    }
}

fn ex_close(eap: &mut exarg_T) {
    unsafe {
        if !text_locked() && !curbuf_locked() {
            let win = if eap.addr_count == 0 {
                *curwin.as_ptr()
            } else {
                let w = win_find_nr(eap.line2, g_st().curtab);
                if w.is_null() { g_st().lastwin } else { w }
            };
            ex_win_close(eap.forceit, &mut *win, ptr::null_mut());
        }
    }
}

fn ex_pclose(eap: &mut exarg_T) {
    unsafe {
        let mut win = g_st().firstwin;
        while !win.is_null() {
            if (*win).w_onebuf_opt.wo_pvw != 0 {
                ex_win_close(eap.forceit, &mut *win, ptr::null_mut());
                break;
            }
            win = (*win).w_next;
        }
        win_float_close(kWinPreview);
    }
}

pub fn ex_win_close(forceit: c_int, win: &mut win_T, tp: *mut tabpage_T) {
    unsafe {
        debug_assert!(tp.is_null() || tp != g_st().curtab);

        if is_ctx_win(&raw mut *win) {
            let _ = emsg(gettext(e_autocmd_close));
            return;
        }
        if !win.w_floating && window_layout_locked(cmdidx_T::CMD_close) {
            return;
        }

        let buf = win.w_buffer;

        let mut need_hide = bufIsChanged(&mut *buf) && (*buf).b_nwindows <= 1;
        if need_hide && !buf_hide(&*buf) && forceit == 0 {
            if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && opt_vars().p_write != 0 {
                let mut bufref: bufref_T = bufref_T::new();
                set_bufref(&mut bufref, buf);
                dialog_changed(&mut *buf, false);
                if bufref_valid(Some(&bufref)) && bufIsChanged(&mut *buf) {
                    return;
                }
                need_hide = false;
            } else {
                no_write_message();
                return;
            }
        }

        if tp.is_null() {
            win_close(&raw mut *win, !need_hide && !buf_hide(&*buf), forceit != 0);
        } else {
            win_close_othertab(
                &raw mut *win,
                c_int::from(!need_hide && !buf_hide(&*buf)),
                tp,
                forceit != 0,
            );
        }
    }
}

fn ex_tabclose(eap: &mut exarg_T) {
    unsafe {
        if (*g_st().first_tabpage).tp_next.is_null() {
            let _ = emsg(gettext(c"E784: Cannot close last tab page"));
            return;
        }

        if window_layout_locked(cmdidx_T::CMD_tabclose) {
            return;
        }

        let tab_number = get_tabpage_arg(eap);
        if !eap.errmsg.is_null() {
            return;
        }

        let tp = find_tabpage(tab_number);
        if tp.is_null() {
            beep_flush();
            return;
        }
        if tp != g_st().curtab {
            tabpage_close_other(&mut *tp, eap.forceit);
        } else if !text_locked() && !curbuf_locked() {
            tabpage_close(eap.forceit);
        }
    }
}

fn ex_tabonly(eap: &mut exarg_T) {
    unsafe {
        if (*g_st().first_tabpage).tp_next.is_null() {
            let _ = msg(gettext(c"Already only one tab page"), 0);
            return;
        }

        if window_layout_locked(cmdidx_T::CMD_tabonly) {
            return;
        }

        let tab_number = get_tabpage_arg(eap);
        if !eap.errmsg.is_null() {
            return;
        }

        goto_tabpage(tab_number);
        let mut done: c_int = 0;
        while done < 1000 {
            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                if (*tp).tp_topframe != g_st().topframe.cast() {
                    tabpage_close_other(&mut *tp, eap.forceit);
                    if valid_tabpage(tp) {
                        done = 1000;
                    }
                    break;
                }
                tp = (*tp).tp_next;
            }
            if (*g_st().first_tabpage).tp_next.is_null() {
                break;
            }
            done += 1;
        }
    }
}

pub fn tabpage_close(forceit: c_int) {
    unsafe {
        let mut done: c_int = 0;

        if window_layout_locked(cmdidx_T::CMD_tabclose) {
            return;
        }

        trigger_tabclosedpre(g_st().curtab);
        (*g_st().curtab).tp_did_tabclosedpre = true;
        let save_curtab = g_st().curtab;

        while (**curwin.as_ptr()).w_floating && {
            done += 1;
            done < 1000
        } {
            ex_win_close(forceit, &mut **curwin.as_ptr(), ptr::null_mut());
        }
        if done == 1000 {
            return;
        }

        if g_st().firstwin != g_st().lastwin {
            close_others(1, forceit, true);
        }
        if g_st().firstwin == g_st().lastwin {
            ex_win_close(forceit, &mut **curwin.as_ptr(), ptr::null_mut());
        }
        if g_st().curtab == save_curtab {
            (*g_st().curtab).tp_did_tabclosedpre = false;
        }
    }
}

pub fn tabpage_close_other(tp: &mut tabpage_T, forceit: c_int) {
    unsafe {
        let mut done: c_int = 0;
        let mut prev_idx = [0 as c_char; NUMBUFLEN];

        debug_assert!(!ptr::eq(tp, g_st().curtab));

        if window_layout_locked(cmdidx_T::CMD_SIZE) {
            return;
        }

        trigger_tabclosedpre(&raw mut *tp);
        tp.tp_did_tabclosedpre = true;

        while {
            done += 1;
            done < 1000
        } {
            let _ = vim_snprintf!(
                prev_idx.as_mut_ptr(),
                NUMBUFLEN,
                c"%i",
                tabpage_index(&raw mut *tp),
            );
            let wp = tp.tp_lastwin;

            if ptr::eq(tp, g_st().curtab) {
                done = 1000;
                break;
            }
            ex_win_close(forceit, &mut *wp, &raw mut *tp);

            if !valid_tabpage(&raw mut *tp) {
                break;
            }

            if tp.tp_lastwin == wp {
                done = 1000;
                break;
            }
        }
        if done >= 1000 {
            tp.tp_did_tabclosedpre = false;
        }
    }
}

fn ex_only(eap: &mut exarg_T) {
    unsafe {
        if window_layout_locked(cmdidx_T::CMD_only) {
            return;
        }

        if eap.addr_count > 0 {
            let mut wnr = eap.line2;
            let mut wp = g_st().firstwin;
            while {
                wnr -= 1;
                wnr > 0
            } {
                if (*wp).w_next.is_null() {
                    break;
                }
                wp = (*wp).w_next;
            }
            if wp != *curwin.as_ptr() {
                win_goto(wp);
            }
        }
        close_others(1, eap.forceit, false);
    }
}

fn ex_hide(eap: &mut exarg_T) {
    unsafe {
        if eap.skip != 0 {
            return;
        }

        let win = if eap.addr_count == 0 {
            *curwin.as_ptr()
        } else {
            let w = win_find_nr(eap.line2, g_st().curtab);
            if w.is_null() { g_st().lastwin } else { w }
        };

        if !(*win).w_floating && window_layout_locked(cmdidx_T::CMD_hide) {
            return;
        }

        win_close(win, false, eap.forceit != 0);
    }
}

fn ex_stop(eap: &mut exarg_T) {
    if eap.forceit == 0 {
            autowrite_all();
        }
        may_trigger_vim_suspend_resume(true);
        ui_call_suspend();
        ui_flush();
}

fn ex_exit(eap: &mut exarg_T) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }

        let save_exiting = g_st().exiting;
        if check_more(false, eap.forceit != 0) && only_one_window() {
            g_st().exiting = true;
        }
        if ((eap.cmdidx == cmdidx_T::CMD_wq as c_int || curbufIsChanged())
            && !do_write(eap))
            || before_quit_autocmds(*curwin.as_ptr(), false, eap.forceit != 0)
            || !check_more(true, eap.forceit != 0)
            || (only_one_window() && check_changed_any(eap.forceit != 0, false))
        {
            not_exiting(save_exiting);
        } else {
            if only_one_window() {
                getout(0);
            }
            not_exiting(save_exiting);
            win_close(*curwin.as_ptr(), !buf_hide(&*(**curwin.as_ptr()).w_buffer), eap.forceit != 0);
        }
    }
}

fn ex_print(eap: &mut exarg_T) {
    unsafe {
        if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            let _ = emsg(gettext(e_empty_buffer));
        } else {
            let mut line = eap.line1;
            while line <= eap.line2 && !g_st().got_int {
                print_line(
                    line,
                    eap.cmdidx == cmdidx_T::CMD_number as c_int
                        || eap.cmdidx == cmdidx_T::CMD_pound as c_int
                        || (eap.flags & EXFLAG_NR) != 0,
                    eap.cmdidx == cmdidx_T::CMD_list as c_int
                        || (eap.flags & EXFLAG_LIST) != 0,
                    line == eap.line1,
                );
                line += 1;
                os_breakcheck();
            }
            setpcmark();
            (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
            beginline(BL_SOL | BL_FIX);
        }
    }
}

fn ex_goto(eap: &mut exarg_T) {
    goto_byte(eap.line2);
}

fn ex_preserve(_eap: &mut exarg_T) {
    unsafe {
        ml_preserve(&mut *g_st().curbuf, true, true);
    }
}

fn ex_recover(eap: &mut exarg_T) {
    unsafe {
        g_st().recoverymode = true;
        if !check_changed(
            &mut *g_st().curbuf,
            (if opt_vars().p_awa != 0 { CCGD_AW } else { 0 })
                | CCGD_MULTWIN
                | (if eap.forceit != 0 { CCGD_FORCEIT } else { 0 })
                | CCGD_EXCMD,
        ) && (*eap.arg == NUL || setfname(g_st().curbuf, eap.arg, ptr::null_mut(), true))
        {
            ml_recover(true);
        }
        g_st().recoverymode = false;
    }
}

fn ex_wrongmodifier(eap: &mut exarg_T) {
    eap.errmsg = gettext(e_invcmd);
}

fn call_findfunc(pat: *mut c_char, cmdcomplete: BoolVarValue) -> *mut list_T {
    unsafe {
        let saved_sctx = g_st().current_sctx;

        let mut args = [
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: 0,
                vval: TypvalVval::String(pat),
            },
            typval_T {
                v_type: VarType::VAR_BOOL,
                v_lock: 0,
                vval: TypvalVval::Bool(cmdcomplete),
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
        ];

        g_st().textlock += 1;

        let ctx = get_option_sctx(kOptFindfunc);
        if !ctx.is_null() {
            g_st().current_sctx = *ctx;
        }

        let cb = get_findfunc();
        let mut rettv: typval_T = typval_T::new();
        let retval = callback_call(cb, 2, args.as_mut_ptr(), &mut rettv);

        g_st().current_sctx = saved_sctx;
        g_st().textlock -= 1;

        let mut retlist: *mut list_T = ptr::null_mut();
        if retval {
            if rettv.v_type == VarType::VAR_LIST {
                retlist = tv_list_copy(None, rettv.vval.v_list(), false, get_copyID());
            } else {
                let _ = emsg(gettext(e_invalid_return_type_from_findfunc));
            }
            tv_clear(&raw mut rettv);
        }

        retlist
    }
}

pub fn expand_findfunc(
    xp: &mut expand_T,
    pat: *mut c_char,
    files: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        files.clear();

        let l = call_findfunc(pat, kBoolVarTrue);
        if l.is_null() {
            return false;
        }

        let len = (*l).lv_len;
        if len == 0 {
            tv_list_free(l);
            return false;
        }

        crate::cmdexpand::expand_process_user_list(l, files, xp);
        tv_list_free(l);

        true
    }
}

fn findfunc_find_file(
    findarg: *mut c_char,
    findarg_len: usize,
    count: c_int,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut ret_fname: Option<std::ffi::CString> = None;
        let cc = *findarg.add(findarg_len);
        *findarg.add(findarg_len) = NUL;

        let fname_list = call_findfunc(findarg, kBoolVarFalse);
        let fname_count = if fname_list.is_null() {
            0
        } else {
            (*fname_list).lv_len
        };

        if fname_count == 0 {
            let _ = semsg!(gettext(e_cant_find_file_str_in_path), findarg);
        } else if count > fname_count {
            let _ = semsg!(gettext(e_no_more_file_str_found_in_path), findarg);
        } else {
            let li = tv_list_find(fname_list, count - 1);
            if !li.is_null() {
                let tv = &raw const (*li).li_tv;
                if (*tv).v_type == VarType::VAR_STRING && !(*tv).vval.v_string().is_null() {
                    ret_fname = Some(cstr_to_cstring((*tv).vval.v_string()));
                } else if (*tv).v_type == VarType::VAR_DICT && !(*tv).vval.v_dict().is_null() {
                    ret_fname =
                        crate::eval::typval::tv_dict_get_cstring((*tv).vval.v_dict(), c"word");
                }
            }
        }

        if !fname_list.is_null() {
            tv_list_free(fname_list);
        }

        *findarg.add(findarg_len) = cc;
        if let Some(ref rf) = ret_fname {
            to_slash(rf.as_ptr().cast_mut());
        }
        ret_fname
    }
}

pub fn free_findfunc_option() {
    callback_free(&mut opt_vars().p_ffu);
}

pub fn set_ref_in_findfunc(copyID: c_int) -> bool {
    set_ref_in_callback(&mut opt_vars().p_ffu, copyID, ptr::null_mut(), ptr::null_mut())
}

fn set_browse_edit_arg(eap: &mut exarg_T) {
    unsafe {
        if (g_st().cmdmod.cmod_flags & CMOD_BROWSE) != 0
            && *eap.arg == NUL
            && (eap.cmdidx == cmdidx_T::CMD_edit as c_int
                || eap.cmdidx == cmdidx_T::CMD_split as c_int
                || eap.cmdidx == cmdidx_T::CMD_vsplit as c_int
                || eap.cmdidx == cmdidx_T::CMD_tabedit as c_int
                || eap.cmdidx == cmdidx_T::CMD_tabnew as c_int)
        {
            eap.arg = c".".as_ptr().cast_mut();
        }
    }
}

pub fn ex_splitview(eap: &mut exarg_T) {
    unsafe {
        set_browse_edit_arg(eap);
        let old_curwin = *curwin.as_ptr();
        let mut fname: Option<std::ffi::CString> = None;
        let use_tab = eap.cmdidx == cmdidx_T::CMD_tabedit as c_int
            || eap.cmdidx == cmdidx_T::CMD_tabfind as c_int
            || eap.cmdidx == cmdidx_T::CMD_tabnew as c_int;

        if bt_quickfix(g_st().curbuf) && g_st().cmdmod.cmod_tab == 0 {
            if eap.cmdidx == cmdidx_T::CMD_split as c_int {
                eap.cmdidx = cmdidx_T::CMD_new as c_int;
            }
            if eap.cmdidx == cmdidx_T::CMD_vsplit as c_int {
                eap.cmdidx = cmdidx_T::CMD_vnew as c_int;
            }
        }

        'theend: {
            if eap.cmdidx == cmdidx_T::CMD_sfind as c_int
                || eap.cmdidx == cmdidx_T::CMD_tabfind as c_int
            {
                if (*get_findfunc()).cb_type() != kCallbackNone {
                    fname = findfunc_find_file(
                        eap.arg,
                        crate::memory::cstr_len(eap.arg),
                        if eap.addr_count > 0 {
                            eap.line2
                        } else {
                            1
                        },
                    );
                } else {
                    let mut file_to_find: Option<std::ffi::CString> = None;
                    let mut search_ctx: Option<ff_search_ctx_T> = None;
                    fname = find_file_in_path(
                        eap.arg,
                        crate::memory::cstr_len(eap.arg),
                        FNAME_MESS,
                        c_int::from(true),
                        (*g_st().curbuf).b_ffname,
                        &mut file_to_find,
                        &mut search_ctx,
                    );
                }
                let Some(ref f) = fname else {
                    break 'theend;
                };
                eap.arg = f.as_ptr().cast_mut();
            }

            if use_tab {
                let after = if g_st().cmdmod.cmod_tab != 0 {
                    g_st().cmdmod.cmod_tab
                } else if eap.addr_count == 0 {
                    0
                } else {
                    eap.line2 + 1
                };
                if !win_new_tabpage(after, eap.arg, true, ptr::null_mut()).is_null() {
                    do_exedit(eap, old_curwin);
                    apply_autocmds(
                        EVENT_TABNEWENTERED,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        g_st().curbuf,
                    );

                    if *curwin.as_ptr() != old_curwin
                        && win_valid(old_curwin)
                        && (*old_curwin).w_buffer != g_st().curbuf
                        && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0
                    {
                        (*old_curwin).w_alt_fnum = (*g_st().curbuf).b_fnum;
                    }
                }
            } else if win_split(
                if eap.addr_count > 0 {
                    eap.line2
                } else {
                    0
                },
                if *eap.cmd == b'v' as c_char {
                    WSP_VERT
                } else {
                    0
                },
            )
            {
                if *eap.arg != NUL {
                    (**curwin.as_ptr()).w_onebuf_opt.wo_scb = 0;
                    (**curwin.as_ptr()).w_onebuf_opt.wo_crb = 0;
                } else {
                    do_check_scrollbind(false);
                }
                do_exedit(eap, old_curwin);
            }
        }

        drop(fname);
    }
}

pub fn tabpage_new() {
    let mut ea: exarg_T = exarg_T::new();
        ea.cmdidx = cmdidx_T::CMD_tabnew as c_int;
        ea.cmd = c"tabn".as_ptr().cast_mut();
        ea.arg = c"".as_ptr().cast_mut();
        ex_splitview(&mut ea);
}

fn ex_tabnext(eap: &mut exarg_T) {
    unsafe {
        if eap.cmdidx == cmdidx_T::CMD_tabfirst as c_int
            || eap.cmdidx == cmdidx_T::CMD_tabrewind as c_int
        {
            goto_tabpage(1);
        } else if eap.cmdidx == cmdidx_T::CMD_tablast as c_int {
            goto_tabpage(9999);
        } else if eap.cmdidx == cmdidx_T::CMD_tabprevious as c_int
            || eap.cmdidx == cmdidx_T::CMD_tabNext as c_int
        {
            let tab_number: c_int;
            if !eap.arg.is_null() && *eap.arg != NUL {
                let mut p = eap.arg;
                let p_save = p;
                tab_number = getdigits(&raw mut p, false, 0) as c_int;
                if p == p_save
                    || *p_save == b'-' as c_char
                    || *p_save == b'+' as c_char
                    || *p != NUL
                    || tab_number == 0
                {
                    eap.errmsg = ex_errmsg(e_invarg2, eap.arg);
                    return;
                }
            } else if eap.addr_count == 0 {
                tab_number = 1;
            } else {
                tab_number = eap.line2;
                if tab_number < 1 {
                    eap.errmsg = gettext(e_invrange);
                    return;
                }
            }
            goto_tabpage(-tab_number);
        } else {
            let tab_number = get_tabpage_arg(eap);
            if eap.errmsg.is_null() {
                goto_tabpage(tab_number);
            }
        }
    }
}

fn ex_tabmove(eap: &mut exarg_T) {
    let tab_number = get_tabpage_arg(eap);
        if eap.errmsg.is_null() {
            tabpage_move(tab_number);
        }
}

fn ex_tabs(_eap: &mut exarg_T) {
    unsafe {
        let mut tabcount: c_int = 1;

        msg_ext_set_kind(c"list_cmd");
        msg_start();
        g_st().msg_scroll = c_int::from(true);

        let lastused_win = if valid_tabpage(g_st().lastused_tabpage) {
            (*g_st().lastused_tabpage).tp_curwin
        } else {
            ptr::null_mut()
        };

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            if g_st().got_int {
                break;
            }

            if g_st().msg_col > 0 {
                msg_putchar(b'\n' as c_int);
            }
            let _ = vim_snprintf!(
                (&raw mut g_st().IObuff).cast(),
                IOSIZE,
                gettext(c"Tab page %d"),
                tabcount,
            );
            tabcount += 1;
            let _ = msg_outtrans((&raw const g_st().IObuff).cast(), HLF_T, false);
            os_breakcheck();

            let mut wp = if ptr::eq(tp, g_st().curtab) {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if g_st().got_int {
                    break;
                }
                if !(*wp).w_config.focusable || (*wp).w_config.hide {
                    wp = (*wp).w_next;
                    continue;
                }

                msg_putchar(b'\n' as c_int);
                msg_putchar(if wp == *curwin.as_ptr() {
                    b'>' as c_int
                } else if wp == lastused_win {
                    b'#' as c_int
                } else {
                    b' ' as c_int
                });
                msg_putchar(b' ' as c_int);
                msg_putchar(if bufIsChanged(&mut *(*wp).w_buffer) {
                    b'+' as c_int
                } else {
                    b' ' as c_int
                });
                msg_putchar(b' ' as c_int);
                let spname = buf_spname((*wp).w_buffer);
                if !spname.is_null() {
                    xstrlcpy((&raw mut g_st().IObuff).cast(), spname, IOSIZE);
                } else {
                    home_replace(
                        (*wp).w_buffer,
                        (*(*wp).w_buffer).b_fname,
                        (&raw mut g_st().IObuff).cast(),
                        IOSIZE,
                        true,
                    );
                }
                let _ = msg_outtrans((&raw const g_st().IObuff).cast(), 0, false);
                os_breakcheck();

                wp = (*wp).w_next;
            }

            tp = (*tp).tp_next;
        }
    }
}

fn ex_detach(eap: &mut exarg_T) {
    unsafe {
        let chan = find_channel(g_st().current_ui);
        if chan.is_null() {
            let _ = emsg(gettext(e_noui));
            return;
        }

        if eap.forceit != 0 {
            (*chan).detach = true;
            channel_event(chan, EVENT_CHANINFO);
            let _ = msg(
                gettext(c"Nvim will continue running if the UI disconnects"),
                0,
            );
            return;
        }

        if eap.addr_count > 0 {
            if eap.line1 != 1 || eap.line2 != (*g_st().curbuf).b_ml.ml_line_count {
                let _ = emsg(gettext(e_invrange));
                return;
            }

            let n = ui_detach_others(g_st().current_ui);
            if n == 0 {
                let _ = msg(gettext(c"No other UIs are attached"), 0);
            } else {
                let _ = smsg!(
                    0,
                    gettext(c"Detached %d non-current UIs"),
                    n as c_int,
                );
            }
            if log_should_log(LOGLVL_INF) {
                logmsg!(
                    LOGLVL_INF,
                    ptr::null(),
                    c"ex_detach",
                    line!() as c_int,
                    true,
                    c"%%detach current_ui=%llu detached=%zu",
                    g_st().current_ui,
                    n,
                );
            }
        } else {
            let mut err = error_init();
            ui_detach_channel(g_st().current_ui, &mut err);
            if error_set(&err) {
                let _ = emsg(err.msg_ptr());
                api_clear_error(&mut err);
                return;
            }
            if log_should_log(LOGLVL_INF) {
                logmsg!(
                    LOGLVL_INF,
                    ptr::null(),
                    c"ex_detach",
                    line!() as c_int,
                    true,
                    c"detach current_ui=%llu",
                    g_st().current_ui,
                );
            }
        }
    }
}

fn ex_connect(eap: &mut exarg_T) {
    let stop_server = if eap.forceit != 0 {
            ui_active() == 1
        } else {
            false
        };

        let mut err = error_init();
        remote_ui_connect(g_st().current_ui, eap.arg, &mut err);

        if error_set(&err) {
            let _ = emsg(err.msg_ptr());
            api_clear_error(&mut err);
            return;
        }

        ex_detach(&mut crate::ex_docmd::exarg_T::new());
        if stop_server {
            g_st().exiting = true;
            getout(0);
        }
}

fn ex_mode(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg == NUL {
            g_st().must_redraw = UPD_CLEAR;
            ex_redraw(eap);
        } else {
            let _ = emsg(gettext(e_screenmode));
        }
    }
}

fn ex_resize(eap: &mut exarg_T) {
    unsafe {
        let mut wp = *curwin.as_ptr();

        if eap.addr_count > 0 {
            let mut n = eap.line2;
            wp = g_st().firstwin;
            while !(*wp).w_next.is_null() && {
                n -= 1;
                n > 0
            } {
                wp = (*wp).w_next;
            }
        }

        let mut n = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.arg)) as std::ffi::c_long as c_int;
        if (g_st().cmdmod.cmod_split & WSP_VERT) != 0 {
            if *eap.arg == b'-' as c_char || *eap.arg == b'+' as c_char {
                n += (*wp).w_width;
            } else if n == 0 && *eap.arg == NUL {
                n = g_st().Columns;
            }
            win_setwidth_win(n, wp, true);
        } else {
            if *eap.arg == b'-' as c_char || *eap.arg == b'+' as c_char {
                n += (*wp).w_height;
            } else if n == 0 && *eap.arg == NUL {
                n = g_st().Rows - 1;
            }
            win_setheight_win(n, wp, true);
        }
    }
}

fn ex_find(eap: &mut exarg_T) {
    unsafe {
        if !check_can_set_curbuf_forceit(eap.forceit) {
            return;
        }

        let mut fname: Option<std::ffi::CString>;
        if (*get_findfunc()).cb_type() != kCallbackNone {
                fname = findfunc_find_file(
                    eap.arg,
                    crate::memory::cstr_len(eap.arg),
                    if eap.addr_count > 0 {
                        eap.line2
                    } else {
                        1
                    },
                );
            } else {
                let mut file_to_find: Option<std::ffi::CString> = None;
                let mut search_ctx: Option<ff_search_ctx_T> = None;
                fname = find_file_in_path(
                    eap.arg,
                    crate::memory::cstr_len(eap.arg),
                    FNAME_MESS,
                    c_int::from(true),
                    (*g_st().curbuf).b_ffname,
                    &mut file_to_find,
                    &mut search_ctx,
                );
                if eap.addr_count > 0 {
                    let mut count = eap.line2;
                    while fname.is_some() && {
                        count -= 1;
                        count > 0
                    } {
                        fname = find_file_in_path(
                            ptr::null_mut(),
                            0,
                            FNAME_MESS,
                            c_int::from(false),
                            (*g_st().curbuf).b_ffname,
                            &mut file_to_find,
                            &mut search_ctx,
                        );
                    }
                }
            }

            let Some(fname) = fname else {
                return;
            };

            eap.arg = fname.as_ptr().cast_mut();
            do_exedit(eap, ptr::null_mut());
    }
}

fn ex_edit(eap: &mut exarg_T) {
    unsafe {
        set_browse_edit_arg(eap);
        let ffname = if eap.cmdidx == cmdidx_T::CMD_enew as c_int {
            ptr::null_mut()
        } else {
            eap.arg
        };

        if eap.cmdidx != cmdidx_T::CMD_badd as c_int
            && eap.cmdidx != cmdidx_T::CMD_balt as c_int
            && is_other_file(0, ffname)
            && !check_can_set_curbuf_forceit(eap.forceit)
        {
            return;
        }

        if bt_prompt(g_st().curbuf) && eap.cmdidx == cmdidx_T::CMD_edit as c_int && *eap.arg == NUL {
            let _ = emsg(c"cannot :edit a prompt buffer");
            return;
        }

        do_exedit(eap, ptr::null_mut());
    }
}

pub fn do_exedit(eap: &mut exarg_T, old_curwin: *mut win_T) {
    unsafe {
        if (eap.cmdidx == cmdidx_T::CMD_new as c_int
            || eap.cmdidx == cmdidx_T::CMD_tabnew as c_int
            || eap.cmdidx == cmdidx_T::CMD_tabedit as c_int
            || eap.cmdidx == cmdidx_T::CMD_vnew as c_int)
            && *eap.arg == NUL
        {
            setpcmark();
            do_ecmd(
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                Some(&*eap),
                ECMD_ONE,
                ECMD_HIDE + if eap.forceit != 0 { ECMD_FORCEIT } else { 0 },
                if old_curwin.is_null() {
                    *curwin.as_ptr()
                } else {
                    ptr::null_mut()
                },
            );
        } else if (eap.cmdidx != cmdidx_T::CMD_split as c_int
            && eap.cmdidx != cmdidx_T::CMD_vsplit as c_int)
            || *eap.arg != NUL
        {
            if *eap.arg != NUL && text_or_buf_locked() {
                return;
            }
            let n = g_st().readonlymode;
            if eap.cmdidx == cmdidx_T::CMD_view as c_int
                || eap.cmdidx == cmdidx_T::CMD_sview as c_int
            {
                g_st().readonlymode = true;
            } else if eap.cmdidx == cmdidx_T::CMD_enew as c_int {
                g_st().readonlymode = false;
            }
            if eap.cmdidx != cmdidx_T::CMD_balt as c_int
                && eap.cmdidx != cmdidx_T::CMD_badd as c_int
            {
                setpcmark();
            }
            let ecmd_flags = (if buf_hide(&*g_st().curbuf) { ECMD_HIDE } else { 0 })
                + (if eap.forceit != 0 { ECMD_FORCEIT } else { 0 })
                + (if !old_curwin.is_null() {
                    ECMD_OLDBUF
                } else {
                    0
                })
                + (if eap.cmdidx == cmdidx_T::CMD_badd as c_int {
                    ECMD_ADDBUF
                } else {
                    0
                })
                + (if eap.cmdidx == cmdidx_T::CMD_balt as c_int {
                    ECMD_ALTBUF
                } else {
                    0
                });
            if !do_ecmd(
                0,
                if eap.cmdidx == cmdidx_T::CMD_enew as c_int {
                    ptr::null_mut()
                } else {
                    eap.arg
                },
                ptr::null_mut(),
                Some(&*eap),
                eap.do_ecmd_lnum,
                ecmd_flags,
                if old_curwin.is_null() {
                    *curwin.as_ptr()
                } else {
                    ptr::null_mut()
                },
            )
            {
                if !old_curwin.is_null() {
                    let need_hide = curbufIsChanged() && (*g_st().curbuf).b_nwindows <= 1;
                    if !need_hide || buf_hide(&*g_st().curbuf) {
                        let mut cs: cleanup_T = cleanup_T::new();
                        enter_cleanup(&mut cs);
                        win_close(*curwin.as_ptr(), !need_hide && !buf_hide(&*g_st().curbuf), false);
                        leave_cleanup(&mut cs);
                    }
                }
            } else if g_st().readonlymode && (*g_st().curbuf).b_nwindows == 1 {
                (*g_st().curbuf).b_p_ro = c_int::from(true);
            }
            g_st().readonlymode = n;
        } else {
            if !eap.do_ecmd_cmd.is_null() {
                do_cmdline_cmd(eap.do_ecmd_cmd);
            }
            let n = (**curwin.as_ptr()).w_arg_idx_invalid;
            check_arg_idx(&mut **curwin.as_ptr());
            if n != (**curwin.as_ptr()).w_arg_idx_invalid {
                maketitle();
            }
        }

        if !old_curwin.is_null()
            && *eap.arg != NUL
            && *curwin.as_ptr() != old_curwin
            && win_valid(old_curwin)
            && (*old_curwin).w_buffer != g_st().curbuf
            && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0
        {
            (*old_curwin).w_alt_fnum = (*g_st().curbuf).b_fnum;
        }
    }
}

fn ex_nogui(eap: &mut exarg_T) {
    eap.errmsg = gettext(c"E25: Nvim does not have a built-in GUI");
}

fn ex_popup(eap: &mut exarg_T) {
    pum_make_popup(eap.arg, eap.forceit);
}

fn ex_swapname(_eap: &mut exarg_T) {
    unsafe {
        let mfp = (*g_st().curbuf).b_ml.mfp_ptr();
        if mfp.is_null() {
            let _ = msg(gettext(c"No swap file"), 0);
        } else if let Some(ref mf_fname) = (*mfp).mf_fname {
            let _ = msg(mf_fname.as_ptr(), 0);
        } else {
            let _ = msg(gettext(c"No swap file"), 0);
        }
    }
}

fn ex_syncbind(_eap: &mut exarg_T) {
    unsafe {
        let old_linenr = (**curwin.as_ptr()).w_cursor.lnum;
        setpcmark();

        let vtopline = if (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
            let mut vt = get_vtopline(&mut **curwin.as_ptr());
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if (*wp).w_onebuf_opt.wo_scb != 0 && !(*wp).w_buffer.is_null() {
                    let y = plines_m_win_fill(wp, 1, (*(*wp).w_buffer).b_ml.ml_line_count)
                        - (get_scrolloff_value(*curwin.as_ptr()) as c_int);
                    vt = vt.min(y);
                }
                wp = (*wp).w_next;
            }
            vt.max(1)
        } else {
            1
        };

        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            if (*wp).w_onebuf_opt.wo_scb != 0 {
                let y = vtopline - get_vtopline(&mut *wp);
                if y > 0 {
                    scrollup(&mut *wp, y, true);
                } else {
                    scrolldown(&mut *wp, -y, c_int::from(true));
                }
                (*wp).w_scbind_pos = vtopline;
                redraw_later(wp, UPD_VALID);
                cursor_correct(&mut *wp);
                (*wp).w_redr_status = true;
            }
            wp = (*wp).w_next;
        }

        if (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
            g_st().did_syncbind = true;
            checkpcmark();
            if old_linenr != (**curwin.as_ptr()).w_cursor.lnum {
                let mut ctrl_o = [Ctrl_O, 0];
                let _ = ins_typebuf(ctrl_o.as_mut_ptr(), REMAP_NONE, 0, true, false);
            }
        }
    }
}

fn ex_read(eap: &mut exarg_T) {
    unsafe {
        if eap.usefilter != 0 {
            do_bang(1, eap, false, false, true);
            return;
        }

        if !u_save(eap.line2, eap.line2 + 1) {
            return;
        }

        let i = if *eap.arg == NUL {
            if !check_fname() {
                return;
            }
            readfile(
                (*g_st().curbuf).b_ffname,
                (*g_st().curbuf).b_fname,
                eap.line2,
                0,
                MAXLNUM,
                Some(&*eap),
                0,
                false,
            )
        } else {
            if !vim_strchr(opt_vars().p_cpo, kCpoAltread).is_null() {
                setaltfname(eap.arg, eap.arg, 1);
            }
            readfile(
                eap.arg,
                ptr::null_mut(),
                eap.line2,
                0,
                MAXLNUM,
                Some(&*eap),
                0,
                false,
            )
        };
        if i != OK {
            if !aborting() {
                let _ = semsg!(gettext(e_notopen), eap.arg);
            }
        } else {
            redraw_curbuf_later(UPD_VALID);
        }
    }
}

pub fn free_cd_dir() {
    docmd_st().prev_dir = None;
        g_st().globaldir = None;
}

fn get_prevdir(scope: CdScope) -> *mut Option<std::ffi::CString> {
    unsafe {
        match scope {
            kCdScopeTabpage => &raw mut (*g_st().curtab).tp_prevdir,
            kCdScopeBuffer => &raw mut (*g_st().curbuf).b_prevdir,
            kCdScopeWindow => &raw mut (**curwin.as_ptr()).w_prevdir,
            _ => &raw mut docmd_st().prev_dir,
        }
    }
}

fn post_chdir(scope: CdScope, trigger_dirchanged: bool) {
    unsafe {
        if scope == kCdScopeBuffer {
            (*g_st().curbuf).b_localdir = None;
        }
        if scope >= kCdScopeWindow {
            (**curwin.as_ptr()).w_localdir = None;
        }
        if scope >= kCdScopeTabpage {
            (*g_st().curtab).tp_localdir = None;
        }

        if scope < kCdScopeGlobal {
            let pdir = &*get_prevdir(scope);
            if g_st().globaldir.is_none() && pdir.is_some() {
                g_st().globaldir = pdir.clone();
            }
        }

        let mut cwd = [0 as c_char; MAXPATHL];
        if !os_dirname(cwd.as_mut_ptr(), MAXPATHL) {
            return;
        }
        let cwd_cstr = crate::memory::cstr_to_cstring(cwd.as_ptr());
        match scope {
            kCdScopeGlobal => {
                g_st().globaldir = if (*g_st().curbuf).b_localdir.is_some() {
                    Some(cwd_cstr)
                } else {
                    None
                };
            }
            kCdScopeTabpage => {
                (*g_st().curtab).tp_localdir = Some(cwd_cstr);
            }
            kCdScopeBuffer => {
                (*g_st().curbuf).b_localdir = Some(cwd_cstr);
            }
            kCdScopeWindow => {
                (**curwin.as_ptr()).w_localdir = Some(cwd_cstr);
            }
            _ => std::process::abort(),
        }

        g_st().last_chdir_reason = None;
        shorten_fnames(c_int::from(vim_strchr(opt_vars().p_cpo, kCpoNosymlinks).is_null()));

        if trigger_dirchanged {
            do_autocmd_dirchanged(cwd.as_mut_ptr(), scope, kCdCauseManual, false);
        }
    }
}

pub fn do_chdir(mut new_dir: *mut c_char, scope: CdScope) -> bool {
    unsafe {
        if new_dir.is_null() || allbuf_locked() {
            return false;
        }

        if crate::memory::cstr_as_bytes(new_dir) == b"-" {
            let Some(ref prev) = *get_prevdir(scope) else {
                let _ = emsg(gettext(c"E186: No previous directory"));
                return false;
            };
            new_dir = prev.as_ptr().cast_mut();
        }

        let pdir = if os_dirname((&raw mut g_st().NameBuff).cast(), MAXPATHL) {
            Some(crate::memory::cstr_to_cstring((&raw const g_st().NameBuff).cast()))
        } else {
            None
        };

        if *new_dir == NUL && opt_vars().p_cdh != 0 {
            expand_env(
                c"$HOME".as_ptr().cast_mut(),
                (&raw mut g_st().NameBuff).cast(),
                MAXPATHL as c_int,
            );
            new_dir = (&raw mut g_st().NameBuff).cast();
        }

        let new_dir_owned = to_slash_save(new_dir).unwrap_or_default();
        new_dir = new_dir_owned.as_ptr().cast_mut();

        let bcd_active = scope != kCdScopeBuffer && (*g_st().curbuf).b_localdir.is_some();

        let dir_differs = pdir
            .as_ref()
            .is_none_or(|p| !path_equal(p.as_ptr(), new_dir, kPathCmpLiteral));
        if dir_differs {
            if !bcd_active {
                do_autocmd_dirchanged(new_dir, scope, kCdCauseManual, true);
            }
            if vim_chdir(new_dir) != 0 {
                let _ = emsg(gettext(e_failed));
                return false;
            }
        }
        drop(new_dir_owned);

        let pp = get_prevdir(scope);
        *pp = pdir;

        post_chdir(scope, dir_differs && !bcd_active);
        if bcd_active {
            os_chdir((*g_st().curbuf).b_localdir.as_ref().unwrap().as_ptr());
            shorten_fnames(c_int::from(true));
        }
        ctx_did_chdir();

        true
    }
}

fn chdir_clear(scope: CdScope) {
    unsafe {
        let localdir = if scope == kCdScopeBuffer {
            &raw mut (*g_st().curbuf).b_localdir
        } else if scope == kCdScopeWindow {
            &raw mut (**curwin.as_ptr()).w_localdir
        } else {
            &raw mut (*g_st().curtab).tp_localdir
        };
        if (*localdir).is_none() {
            return;
        }
        *localdir = None;
        update_cwd(kCdCauseManual);
        ctx_did_chdir();
    }
}

pub fn ex_cd(eap: &mut exarg_T) {
    unsafe {
        let new_dir = eap.arg;

        let scope = if eap.cmdidx == cmdidx_T::CMD_tcd as c_int
            || eap.cmdidx == cmdidx_T::CMD_tchdir as c_int
        {
            kCdScopeTabpage
        } else if eap.cmdidx == cmdidx_T::CMD_bcd as c_int
            || eap.cmdidx == cmdidx_T::CMD_bchdir as c_int
        {
            kCdScopeBuffer
        } else if eap.cmdidx == cmdidx_T::CMD_lcd as c_int
            || eap.cmdidx == cmdidx_T::CMD_lchdir as c_int
        {
            kCdScopeWindow
        } else {
            kCdScopeGlobal
        };

        if eap.forceit != 0 && *new_dir == NUL && scope != kCdScopeGlobal {
            chdir_clear(scope);
            if g_st().KeyTyped || opt_vars().p_verbose >= 5 {
                ex_pwd(eap);
            }
            return;
        }

        if *new_dir == NUL && opt_vars().p_cdh == 0 {
            ex_pwd(&mut crate::ex_docmd::exarg_T::new());
            return;
        }

        if do_chdir(new_dir, scope) && (g_st().KeyTyped || opt_vars().p_verbose >= 5) {
            ex_pwd(eap);
        }
    }
}

fn ex_pwd(_eap: &mut exarg_T) {
    unsafe {
        if os_dirname((&raw mut g_st().NameBuff).cast(), MAXPATHL) {
            #[cfg(windows)]
            slash_adjust((&raw mut g_st().NameBuff).cast());
            if opt_vars().p_verbose > 0 {
                let context = if let Some(reason) = g_st().last_chdir_reason {
                    reason.as_ptr()
                } else if (*g_st().curbuf).b_localdir.is_some() {
                    c"buffer".as_ptr()
                } else if (**curwin.as_ptr()).w_localdir.is_some() {
                    c"window".as_ptr()
                } else if (*g_st().curtab).tp_localdir.is_some() {
                    c"tabpage".as_ptr()
                } else {
                    c"global".as_ptr()
                };
                let _ = smsg!(
                    0,
                    c"[%s] %s",
                    context,
                    (&raw const g_st().NameBuff).cast::<c_char>(),
                );
            } else {
                let _ = msg((&raw const g_st().NameBuff).cast(), 0);
            }
        } else {
            let _ = emsg(gettext(c"E187: Unknown"));
        }
    }
}

fn ex_equal(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg != NUL && *eap.arg != b'|' as c_char {
            ex_lua(eap);
        } else {
            eap.nextcmd = find_nextcmd(eap.arg);
            let _ = smsg!(0, c"%lld", i64::from(eap.line2));
        }
    }
}

fn ex_sleep(eap: &mut exarg_T) {
    unsafe {
        if cursor_valid(&mut **curwin.as_ptr()) != 0 {
            setcursor_mayforce(&mut **curwin.as_ptr(), true);
        }

        let mut len = i64::from(eap.line2);
        match *eap.arg as u8 {
            b'm' => {}
            0 => len *= 1000,
            _ => {
                let _ = semsg!(gettext(e_invarg2), eap.arg);
                return;
            }
        }

        do_sleep(len, eap.forceit != 0);
    }
}

pub fn do_sleep(msec: i64, hide_cursor: bool) {
    unsafe {
        if hide_cursor {
            ui_busy_start();
        }

        ui_flush();
        let main_loop_ptr = (&raw mut g_st().main_loop).cast();
        let events = (*loop_queues(&mut *main_loop_ptr)).events;
        let mut remaining = msec;
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !g_st().got_int {
            if !events.is_null() && !multiqueue_empty(events) {
                multiqueue_process_events(events);
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

        if g_st().got_int {
            let _ = vpeekc();
        }

        if hide_cursor {
            ui_busy_stop();
        }
    }
}

fn ex_winsize(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        if !ascii_isdigit(*arg) {
            let _ = semsg!(gettext(e_invarg2), arg);
            return;
        }
        let w = getdigits_int(&raw mut arg, false, 10);
        arg = skipwhite(arg);
        let p = arg;
        let h = getdigits_int(&raw mut arg, false, 10);
        if *p != NUL && *arg == NUL {
            screen_resize(w, h);
        } else {
            let _ = emsg(gettext(
                c"E465: :winsize requires two number arguments",
            ));
        }
    }
}

fn ex_wincmd(eap: &mut exarg_T) {
    unsafe {
        let mut xchar: c_int = 0;
        let mut p: *mut c_char;

        if *eap.arg == b'g' as c_char || *eap.arg == Ctrl_G {
            if *eap.arg.add(1) == NUL {
                let _ = emsg(gettext(e_invarg));
                return;
            }
            xchar = c_int::from(*eap.arg.add(1) as u8);
            p = eap.arg.add(2);
        } else {
            p = eap.arg.add(1);
        }

        eap.nextcmd = check_nextcmd(p);
        p = skipwhite(p);
        if *p != NUL && *p != b'"' as c_char && eap.nextcmd.is_null() {
            let _ = emsg(gettext(e_invarg));
        } else if eap.skip == 0 {
            g_st().postponed_split_flags = g_st().cmdmod.cmod_split;
            g_st().postponed_split_tab = g_st().cmdmod.cmod_tab;
            do_window(
                c_int::from(*eap.arg as u8),
                if eap.addr_count > 0 {
                    eap.line2
                } else {
                    0
                },
                xchar,
            );
            g_st().postponed_split_flags = 0;
            g_st().postponed_split_tab = 0;
        }
    }
}

fn ex_operators(eap: &mut exarg_T) {
    unsafe {
        let mut oa = oparg_T::new();
        oa.regname = eap.regname;
        oa.start.lnum = eap.line1;
        oa.end.lnum = eap.line2;
        oa.line_count = eap.line2 - eap.line1 + 1;
        oa.motion_type = kMTLineWise;
        g_st().virtual_op = kFalse;
        if eap.cmdidx != cmdidx_T::CMD_yank as c_int {
            setpcmark();
            (**curwin.as_ptr()).w_cursor.lnum = eap.line1;
            beginline(BL_SOL | BL_FIX);
        }

        if g_st().Visual.active {
            end_visual_mode();
        }

        if eap.cmdidx == cmdidx_T::CMD_delete as c_int {
            oa.op_type = OP_DELETE;
            op_delete(&mut oa);
        } else if eap.cmdidx == cmdidx_T::CMD_yank as c_int {
            oa.op_type = OP_YANK;
            op_yank(&mut oa, true);
        } else {
            if (eap.cmdidx == cmdidx_T::CMD_rshift as c_int)
                ^ ((**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0)
            {
                oa.op_type = OP_RSHIFT;
            } else {
                oa.op_type = OP_LSHIFT;
            }
            op_shift(&mut oa, false, eap.amount);
        }
        g_st().virtual_op = kNone;
        ex_may_print(eap);
    }
}

fn ex_put(eap: &mut exarg_T) {
    unsafe {
        if eap.line2 == 0 {
            eap.line2 = 1;
            eap.forceit = c_int::from(true);
        }
        (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
        check_cursor_col(&mut **curwin.as_ptr());
        do_put(
            eap.regname,
            ptr::null_mut(),
            if eap.forceit != 0 {
                BACKWARD
            } else {
                FORWARD
            },
            1,
            PUT_LINE | PUT_CURSLINE,
        );
    }
}

fn ex_iput(eap: &mut exarg_T) {
    unsafe {
        if eap.line2 == 0 {
            eap.line2 = 1;
            eap.forceit = c_int::from(true);
        }
        (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
        check_cursor_col(&mut **curwin.as_ptr());
        do_put(
            eap.regname,
            ptr::null_mut(),
            if eap.forceit != 0 {
                BACKWARD
            } else {
                FORWARD
            },
            1,
            PUT_LINE | PUT_CURSLINE | PUT_FIXINDENT,
        );
    }
}

fn ex_copymove(eap: &mut exarg_T) {
    unsafe {
        let mut errormsg: *const c_char = ptr::null();
        let n = get_address(
            eap,
            &raw mut eap.arg,
            eap.addr_type,
            false,
            false,
            0,
            1,
            &mut errormsg,
        );
        if eap.arg.is_null() {
            if !errormsg.is_null() {
                let _ = emsg(errormsg);
            }
            eap.nextcmd = ptr::null_mut();
            return;
        }
        get_flags(eap);

        if n == MAXLNUM || n < 0 || n > (*g_st().curbuf).b_ml.ml_line_count {
            let _ = emsg(gettext(e_invrange));
            return;
        }

        if eap.cmdidx == cmdidx_T::CMD_move as c_int {
            if !do_move(eap.line1, eap.line2, n) {
                return;
            }
        } else {
            ex_copy(eap.line1, eap.line2, n);
        }
        u_clearline(&mut *g_st().curbuf);
        beginline(BL_SOL | BL_FIX);
        ex_may_print(eap);
    }
}

pub fn ex_may_print(eap: &mut exarg_T) {
    unsafe {
        if eap.flags != 0 {
            print_line(
                (**curwin.as_ptr()).w_cursor.lnum,
                (eap.flags & EXFLAG_NR) != 0,
                (eap.flags & EXFLAG_LIST) != 0,
                true,
            );
        }
    }
}

fn ex_join(eap: &mut exarg_T) {
    unsafe {
        (**curwin.as_ptr()).w_cursor.lnum = eap.line1;
        if eap.line1 == eap.line2 {
            if eap.addr_count >= 2 {
                return;
            }
            if eap.line2 == (*g_st().curbuf).b_ml.ml_line_count {
                beep_flush();
                return;
            }
            eap.line2 += 1;
        }
        do_join(
            ((eap.line2 as isize) - (eap.line1 as isize) + 1) as usize,
            eap.forceit == 0,
            true,
            true,
            true,
        );
        beginline(BL_WHITE | BL_FIX);
        ex_may_print(eap);
    }
}

fn ex_at(eap: &mut exarg_T) {
    unsafe {
        let prev_len = g_st().typebuf.tb_len;

        (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
        check_cursor_col(&mut **curwin.as_ptr());

        let mut c = c_int::from(*eap.arg as u8);
        if c == 0 {
            c = b'@' as c_int;
        }

        if !do_execreg(
            c,
            c_int::from(true),
            c_int::from(!vim_strchr(opt_vars().p_cpo, kCpoExecbuf).is_null()),
            c_int::from(true),
        )
        {
            beep_flush();
            return;
        }

        let save_efr = g_st().exec_from_reg;
        g_st().exec_from_reg = true;

        while !stuff_empty() || g_st().typebuf.tb_len > prev_len {
            do_cmdline(
                ptr::null_mut(),
                LineGetter::ExLine,
                DOCMD_NOWAIT | DOCMD_VERBOSE,
            );
        }

        g_st().exec_from_reg = save_efr;
    }
}

fn ex_bang(eap: &mut exarg_T) {
    do_bang(eap.addr_count, eap, eap.forceit != 0, true, true);
}

fn ex_undo(eap: &mut exarg_T) {
    unsafe {
        if eap.addr_count != 1 {
            if eap.forceit != 0 {
                u_undo_and_forget(1, true);
            } else {
                u_undo(1);
            }
            return;
        }

        let step = eap.line2;

        if eap.forceit != 0 {
            if step >= (*g_st().curbuf).b_u_seq_cur {
                let _ = emsg(gettext(e_undobang_cannot_redo_or_move_branch));
                return;
            }
            let mut uhp = (*g_st().curbuf).b_u_curhead.or((*g_st().curbuf).b_u_newhead);
            let mut count: c_int = 0;
            while let Some(uh) = uhp
                && uhp_ref(&*g_st().curbuf, uh).uh_seq > step
            {
                uhp = uhp_ref(&*g_st().curbuf, uh).uh_next;
                count += 1;
            }
            if step != 0 && uhp.is_none_or(|uh| uhp_ref(&*g_st().curbuf, uh).uh_seq < step) {
                let _ = emsg(gettext(e_undobang_cannot_redo_or_move_branch));
                return;
            }
            u_undo_and_forget(count, true);
        } else {
            undo_time(step, false, false, true);
        }
    }
}

fn ex_wundo(eap: &mut exarg_T) {
    unsafe {
        let mut hash = [0u8; UNDO_HASH_SIZE];
        u_compute_hash(&mut *g_st().curbuf, hash.as_mut_ptr());
        u_write_undo(eap.arg, eap.forceit != 0, g_st().curbuf, hash.as_mut_ptr());
    }
}

fn ex_rundo(eap: &mut exarg_T) {
    unsafe {
        let mut hash = [0u8; UNDO_HASH_SIZE];
        u_compute_hash(&mut *g_st().curbuf, hash.as_mut_ptr());
        u_read_undo(eap.arg, hash.as_ptr(), ptr::null());
    }
}

fn ex_redo(_eap: &mut exarg_T) {
    u_redo(1);
}

fn ex_later(eap: &mut exarg_T) {
    unsafe {
        let mut count: c_int = 0;
        let mut sec = false;
        let mut file = false;
        let mut p = eap.arg;

        if *p == NUL {
            count = 1;
        } else if ascii_isdigit(*p) {
            count = getdigits_int(&raw mut p, false, 0);
            match *p as u8 {
                b's' => {
                    p = p.add(1);
                    sec = true;
                }
                b'm' => {
                    p = p.add(1);
                    sec = true;
                    count *= 60;
                }
                b'h' => {
                    p = p.add(1);
                    sec = true;
                    count *= 60 * 60;
                }
                b'd' => {
                    p = p.add(1);
                    sec = true;
                    count *= 24 * 60 * 60;
                }
                b'f' => {
                    p = p.add(1);
                    file = true;
                }
                _ => {}
            }
        }

        if *p != NUL {
            let _ = semsg!(gettext(e_invarg2), eap.arg);
        } else {
            undo_time(
                if eap.cmdidx == cmdidx_T::CMD_earlier as c_int {
                    -count
                } else {
                    count
                },
                sec,
                file,
                false,
            );
        }
    }
}

fn ex_redir(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;

        if crate::memory::cstr_as_bytes(eap.arg).eq_ignore_ascii_case(b"END") {
            close_redir();
        } else if *arg == b'>' as c_char {
            arg = arg.add(1);
            let mode = if *arg == b'>' as c_char {
                arg = arg.add(1);
                c"a"
            } else {
                c"w"
            };
            arg = skipwhite(arg);

            close_redir();

            let fname = expand_env_save(arg);
            g_st().redir_fd = crate::os::fs::CFile::from_raw(open_exfile(
                fname.as_ptr().cast_mut(),
                eap.forceit,
                mode,
            ));
        } else if *arg == b'@' as c_char {
            close_redir();
            arg = arg.add(1);
            if valid_yank_reg(c_int::from(*arg as u8), true) && *arg != b'_' as c_char {
                g_st().redir_reg = c_int::from(*arg as u8);
                arg = arg.add(1);
                if *arg == b'>' as c_char && *arg.add(1) == b'>' as c_char {
                    arg = arg.add(2);
                } else {
                    if *arg == b'>' as c_char {
                        arg = arg.add(1);
                    }
                    if *arg == NUL && !ascii_isupper(g_st().redir_reg as c_char) {
                        write_reg_contents(g_st().redir_reg, c"".as_ptr(), 0, c_int::from(false));
                    }
                }
            }
            if *arg != NUL {
                g_st().redir_reg = 0;
                let _ = semsg!(gettext(e_invarg2), eap.arg);
            }
        } else if *arg == b'=' as c_char && *arg.add(1) == b'>' as c_char {
            close_redir();
            arg = arg.add(2);
            let append = if *arg == b'>' as c_char {
                arg = arg.add(1);
                true
            } else {
                false
            };

            if var_redir_start(skipwhite(arg), append) {
                g_st().redir_vname = true;
            }
        } else {
            let _ = semsg!(gettext(e_invarg2), eap.arg);
        }

        if has_redir_fd() || g_st().redir_reg != 0 || g_st().redir_vname {
            g_st().redir_off = false;
        }
    }
}

fn ex_redraw(eap: &mut exarg_T) {
    unsafe {
        if g_st().cmdpreview {
            return;
        }
        let r = g_st().RedrawingDisabled;
        let p = opt_vars().p_lz;

        g_st().RedrawingDisabled = 0;
        opt_vars().p_lz = 0;
        validate_cursor(*curwin.as_ptr());
        update_topline(*curwin.as_ptr());
        if eap.forceit != 0 {
            redraw_all_later(UPD_NOT_VALID);
            g_st().redraw_cmdline = true;
        } else if g_st().Visual.active {
            redraw_curbuf_later(UPD_INVERTED);
        }
        update_screen();
        if g_st().need_maketitle {
            maketitle();
        }
        g_st().RedrawingDisabled = r;
        opt_vars().p_lz = p;

        g_st().msg_didout = false;
        g_st().msg_col = 0;
        g_st().need_wait_return = false;

        ui_flush();
    }
}

fn ex_redrawstatus(eap: &mut exarg_T) {
    if g_st().cmdpreview {
            return;
        }
        let r = g_st().RedrawingDisabled;
        let p = opt_vars().p_lz;

        if eap.forceit != 0 {
            status_redraw_all();
        } else {
            status_redraw_curbuf();
        }

        g_st().RedrawingDisabled = 0;
        opt_vars().p_lz = 0;
        if (g_st().State & MODE_CMDLINE) != 0 {
            redraw_statuslines();
        } else {
            if g_st().Visual.active {
                redraw_curbuf_later(UPD_INVERTED);
            }
            update_screen();
        }
        g_st().RedrawingDisabled = r;
        opt_vars().p_lz = p;
        ui_flush();
}

fn ex_redrawtabline(_eap: &mut exarg_T) {
    let r = g_st().RedrawingDisabled;
        let p = opt_vars().p_lz;

        g_st().RedrawingDisabled = 0;
        opt_vars().p_lz = 0;

        draw_tabline();

        g_st().RedrawingDisabled = r;
        opt_vars().p_lz = p;
        ui_flush();
}

fn close_redir() {
    g_st().redir_fd = None;
        g_st().redir_reg = 0;
        if g_st().redir_vname {
            var_redir_stop();
            g_st().redir_vname = false;
        }
}

pub fn vim_mkdir_emsg(name: *const c_char, prot: c_int) -> bool {
    unsafe {
        let ret = os_mkdir(name, prot);
        if ret != 0 {
            let _ = semsg!(gettext(e_mkdir), name, uv_strerror(ret));
            return false;
        }
        true
    }
}

pub fn open_exfile(fname: *mut c_char, forceit: c_int, mode: &std::ffi::CStr) -> *mut FILE {
    #[cfg(unix)]
    if os_isdir(fname) {
        let _ = semsg!(gettext(e_isadir2), fname);
        return ptr::null_mut();
    }

    if forceit == 0 && mode.to_bytes().first() != Some(&b'a') && os_path_exists(fname) {
        let _ = semsg!(
            gettext(c"E189: \"%s\" exists (add ! to override)"),
            fname,
        );
        return ptr::null_mut();
    }

    let fd = os_fopen(fname, mode);
    if fd.is_null() {
        let _ = semsg!(
            gettext(c"E190: Cannot open \"%s\" for writing"),
            fname,
        );
    }
    fd.cast()
}

fn ex_mark(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg == NUL {
            let _ = emsg(gettext(e_argreq));
            return;
        }
        if *eap.arg.add(1) != NUL {
            let _ = semsg!(gettext(e_trailing_arg), eap.arg);
            return;
        }

        let pos = (**curwin.as_ptr()).w_cursor;
        (**curwin.as_ptr()).w_cursor.lnum = eap.line2;
        beginline(BL_WHITE | BL_FIX);
        if !setmark(c_int::from(*eap.arg as u8)) {
            let _ = emsg(gettext(
                c"E191: Argument must be a letter or forward/backward quote",
            ));
        }
        (**curwin.as_ptr()).w_cursor = pos;
    }
}

pub fn update_topline_cursor() {
    unsafe {
        check_cursor(*curwin.as_ptr());
        update_topline(*curwin.as_ptr());
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
            validate_cursor(*curwin.as_ptr());
        }
        update_curswant();
    }
}

pub fn save_current_state(sst: &mut save_state_T) {
    unsafe {
        sst.save_msg_scroll = g_st().msg_scroll;
        sst.save_restart_edit = g_st().restart_edit;
        sst.save_msg_didout = g_st().msg_didout;
        sst.save_State = g_st().State;
        sst.save_finish_op = g_st().finish_op;
        sst.save_opcount = g_st().opcount;
        sst.save_reg_executing = g_st().reg_executing;
        sst.save_pending_end_reg_executing = g_st().pending_end_reg_executing;

        g_st().msg_scroll = 0;
        g_st().restart_edit = 0;

        save_typeahead(&mut *(&raw mut sst.tabuf).cast());
    }
}

pub fn restore_current_state(sst: &mut save_state_T) {
    unsafe {
        restore_typeahead(&mut *(&raw mut sst.tabuf).cast());

        g_st().msg_scroll = sst.save_msg_scroll;
        if g_st().force_restart_edit {
            g_st().force_restart_edit = false;
        } else {
            g_st().restart_edit = sst.save_restart_edit;
        }
        g_st().finish_op = sst.save_finish_op;
        g_st().opcount = sst.save_opcount;
        g_st().reg_executing = sst.save_reg_executing;
        g_st().pending_end_reg_executing = sst.save_pending_end_reg_executing;

        g_st().msg_didout |= sst.save_msg_didout;

        g_st().State = sst.save_State;
        ui_cursor_shape();
    }
}

pub fn expr_map_locked() -> bool {
    unsafe { g_st().expr_map_lock > 0 && ((*g_st().curbuf).b_flags & BF_DUMMY) == 0 }
}

fn ex_normal(eap: &mut exarg_T) {
    unsafe {
        if !(*g_st().curbuf).terminal.is_null() && (g_st().State & MODE_TERMINAL) != 0 {
            let _ = emsg(c"Can't re-enter normal mode from terminal mode");
            return;
        }

        if expr_map_locked() {
            let _ = emsg(gettext(e_secure));
            return;
        }

        if OptInt::from(g_st().ex_normal_busy) >= opt_vars().p_mmd {
            let _ = emsg(gettext(c"E192: Recursive use of :normal too deep"));
            return;
        }

        let mut len: usize = 0;
        let mut p = eap.arg;
        while *p != NUL {
            let mut l = utfc_ptr2len(p) - 1;
            while l > 0 {
                p = p.add(1);
                if (*p as u8) == K_SPECIAL {
                    len += 2;
                }
                l -= 1;
            }
            p = p.add(1);
        }
        let mut arg_vec: Option<Vec<c_char>> = None;
        if len > 0 {
            let mut v = vec![0 as c_char; crate::memory::cstr_len(eap.arg) + len + 1];
            let arg = v.as_mut_ptr();
            len = 0;
            p = eap.arg;
            while *p != NUL {
                *arg.add(len) = *p;
                len += 1;
                let mut l = utfc_ptr2len(p) - 1;
                while l > 0 {
                    p = p.add(1);
                    *arg.add(len) = *p;
                    len += 1;
                    if (*p as u8) == K_SPECIAL {
                        *arg.add(len) = KS_SPECIAL as c_char;
                        len += 1;
                        *arg.add(len) = KE_FILLER;
                        len += 1;
                    }
                    l -= 1;
                }
                *arg.add(len) = NUL;
                p = p.add(1);
            }
            arg_vec = Some(v);
        }

        g_st().ex_normal_busy += 1;
        let mut save_state = save_state_T::new();
        save_current_state(&mut save_state);

        loop {
            if eap.addr_count != 0 {
                (**curwin.as_ptr()).w_cursor.lnum = eap.line1;
                eap.line1 += 1;
                (**curwin.as_ptr()).w_cursor.col = 0;
                check_cursor_moved(*curwin.as_ptr());
            }

            exec_normal_cmd(
                arg_vec
                    .as_ref()
                    .map_or(eap.arg, |v| v.as_ptr().cast_mut()),
                if eap.forceit != 0 {
                    REMAP_NONE
                } else {
                    REMAP_YES
                },
                false,
            );

            if !(eap.addr_count > 0 && eap.line1 <= eap.line2 && !g_st().got_int) {
                break;
            }
        }

        update_topline_cursor();
        restore_current_state(&mut save_state);
        g_st().ex_normal_busy -= 1;

        setmouse();
        ui_cursor_shape();
        drop(arg_vec);
    }
}

fn ex_startinsert(eap: &mut exarg_T) {
    unsafe {
        if (*g_st().curbuf).b_p_ma == 0 && (*g_st().curbuf).terminal.is_null() {
            let _ = emsg(gettext(e_modifiable));
            return;
        }
        if eap.forceit != 0 {
            if (**curwin.as_ptr()).w_cursor.lnum == 0 {
                (**curwin.as_ptr()).w_cursor.lnum = 1;
            }
            set_cursor_for_append_to_line();
        }

        if (g_st().State & MODE_INSERT) != 0 {
            return;
        }

        if eap.cmdidx == cmdidx_T::CMD_startinsert as c_int {
            g_st().restart_edit = b'a' as c_int;
        } else if eap.cmdidx == cmdidx_T::CMD_startreplace as c_int {
            g_st().restart_edit = b'R' as c_int;
        } else {
            g_st().restart_edit = b'V' as c_int;
        }

        if eap.forceit == 0 {
            if eap.cmdidx == cmdidx_T::CMD_startinsert as c_int {
                g_st().restart_edit = b'i' as c_int;
            }
            (**curwin.as_ptr()).w_curswant = 0;
        }

        if g_st().Visual.active {
            showmode();
        }
    }
}

fn ex_stopinsert(_eap: &mut exarg_T) {
    g_st().restart_edit = 0;
        g_st().Ins.stop_insert_mode = true;
        clearmode();
}

pub fn exec_normal_cmd(cmd: *mut c_char, remap: c_int, silent: bool) {
    let _ = ins_typebuf(cmd, remap, 0, true, silent);
        exec_normal(false, false);
}

pub fn exec_normal(was_typed: bool, use_vpeekc: bool) {
    let mut oa = oparg_T::new();
        g_st().finish_op = false;
        while (!stuff_empty()
            || ((was_typed || typebuf_typed() == 0) && g_st().typebuf.tb_len > 0)
            || (use_vpeekc && {
                let c = vpeekc();
                c != 0 && c != c_int::from(Ctrl_C)
            }))
            && !g_st().got_int
        {
            update_topline_cursor();
            normal_cmd(&mut oa, true);
        }
}

fn ex_checkpath(eap: &mut exarg_T) {
    find_pattern_in_path(
            ptr::null_mut(),
            0,
            0,
            false,
            false,
            CHECK_PATH,
            1,
            if eap.forceit != 0 {
                ACTION_SHOW_ALL
            } else {
                ACTION_SHOW
            },
            1,
            MAXLNUM,
            eap.forceit != 0,
            false,
        );
}

fn ex_psearch(eap: &mut exarg_T) {
    g_st().g_do_tagpreview = opt_vars().p_pvh as c_int;
        ex_findpat(eap);
        g_st().g_do_tagpreview = 0;
}

fn ex_findpat(eap: &mut exarg_T) {
    unsafe {
        let mut whole = true;
        let cname = CMDNAMES[eap.cmdidx as usize].cmd_name.to_bytes();
        let action = match cname[2] {
            b'e' => {
                if cname[0] == b'p' {
                    ACTION_GOTO
                } else {
                    ACTION_SHOW
                }
            }
            b'i' => ACTION_SHOW_ALL,
            b'u' => ACTION_GOTO,
            _ => ACTION_SPLIT,
        };

        let mut n: c_int = 1;
        if ascii_isdigit(*eap.arg) {
            n = getdigits_int(&raw mut eap.arg, false, 0);
            eap.arg = skipwhite(eap.arg);
        }
        if *eap.arg == b'/' as c_char {
            whole = false;
            eap.arg = eap.arg.add(1);
            let mut p = skip_regexp(eap.arg, b'/' as c_int, opt_vars().p_magic);
            if *p != NUL {
                *p = NUL;
                p = skipwhite(p.add(1));

                if ends_excmd(c_int::from(*p)) == 0 {
                    eap.errmsg = ex_errmsg(e_trailing_arg, p);
                } else {
                    eap.nextcmd = check_nextcmd(p);
                }
            }
        }
        if eap.skip == 0 {
            find_pattern_in_path(
                eap.arg,
                0,
                crate::memory::cstr_len(eap.arg),
                whole,
                eap.forceit == 0,
                if *eap.cmd == b'd' as c_char {
                    FIND_DEFINE
                } else {
                    FIND_ANY
                },
                n,
                action,
                eap.line1,
                eap.line2,
                eap.forceit != 0,
                false,
            );
        }
    }
}

fn ex_ptag(eap: &mut exarg_T) {
    unsafe {
        g_st().g_do_tagpreview = opt_vars().p_pvh as c_int;
        ex_tag_cmd(eap, CMDNAMES[eap.cmdidx as usize].cmd_name.as_ptr().add(1));
    }
}

fn ex_pedit(eap: &mut exarg_T) {
    unsafe {
        let curwin_save = *curwin.as_ptr();
        prepare_preview_window();
        do_exedit(eap, ptr::null_mut());
        back_to_current_window(curwin_save);
    }
}

fn ex_pbuffer(eap: &mut exarg_T) {
    unsafe {
        let curwin_save = *curwin.as_ptr();
        prepare_preview_window();
        do_exbuffer(eap);
        back_to_current_window(curwin_save);
    }
}

fn prepare_preview_window() {
    unsafe {
        g_st().g_do_tagpreview = opt_vars().p_pvh as c_int;
        prepare_tagpreview(true, *opt_vars().p_pvp != NUL);
    }
}

fn back_to_current_window(curwin_save: *mut win_T) {
    unsafe {
        if *curwin.as_ptr() != curwin_save && win_valid(curwin_save) {
            validate_cursor(*curwin.as_ptr());
            redraw_later(*curwin.as_ptr(), UPD_VALID);
            win_enter(curwin_save, true);
        }
        g_st().g_do_tagpreview = 0;
    }
}

fn ex_stag(eap: &mut exarg_T) {
    unsafe {
        g_st().postponed_split = -1;
        g_st().postponed_split_flags = g_st().cmdmod.cmod_split;
        g_st().postponed_split_tab = g_st().cmdmod.cmod_tab;
        ex_tag_cmd(eap, CMDNAMES[eap.cmdidx as usize].cmd_name.as_ptr().add(1));
        g_st().postponed_split_flags = 0;
        g_st().postponed_split_tab = 0;
    }
}

fn ex_tag(eap: &mut exarg_T) {
    ex_tag_cmd(eap, CMDNAMES[eap.cmdidx as usize].cmd_name.as_ptr());
}

fn ex_tag_cmd(eap: &mut exarg_T, name: *const c_char) {
    unsafe {
        let mut cmd = match *name.add(1) as u8 {
            b'j' => DT_JUMP,
            b's' => DT_SELECT,
            b'p' | b'N' => DT_PREV,
            b'n' => DT_NEXT,
            b'o' => DT_POP,
            b'f' | b'r' => DT_FIRST,
            b'l' => DT_LAST,
            _ => DT_TAG,
        };

        if *name == b'l' as c_char {
            cmd = DT_LTAG;
        }

        do_tag(
            Some(&*eap),
            eap.arg,
            cmd,
            if eap.addr_count > 0 {
                eap.line2
            } else {
                1
            },
            eap.forceit,
            true,
        );
    }
}

const SPEC_PERC: isize = 0;
const SPEC_HASH: isize = 1;
const SPEC_CWORD: isize = 2;
const SPEC_CCWORD: isize = 3;
const SPEC_CEXPR: isize = 4;
const SPEC_CFILE: isize = 5;
const SPEC_SFILE: isize = 6;
const SPEC_SLNUM: isize = 7;
const SPEC_STACK: isize = 8;
const SPEC_SCRIPT: isize = 9;
const SPEC_AFILE: isize = 10;
const SPEC_ABUF: isize = 11;
const SPEC_AMATCH: isize = 12;
const SPEC_SFLNUM: isize = 13;
const SPEC_SID: isize = 14;

pub fn find_cmdline_var(src: *const c_char, usedlen: &mut usize) -> isize {
    const SPEC_STR: [&CStr; 15] = [
        c"%",
        c"#",
        c"<cword>",
        c"<cWORD>",
        c"<cexpr>",
        c"<cfile>",
        c"<sfile>",
        c"<slnum>",
        c"<stack>",
        c"<script>",
        c"<afile>",
        c"<abuf>",
        c"<amatch>",
        c"<sflnum>",
        c"<SID>",
    ];

    for (i, s) in SPEC_STR.iter().enumerate() {
            let len = s.to_bytes().len();
            if crate::memory::cstr_prefix_bytes(src, len) == crate::memory::cstr_prefix_bytes(s.as_ptr(), len) {
                *usedlen = len;
                return i as isize;
            }
        }
        -1
}

pub fn eval_vars(
    src: *mut c_char,
    srcstart: *const c_char,
    usedlen: &mut usize,
    lnump: Option<&mut linenr_T>,
    errormsg: *mut *const c_char,
    mut escaped: Option<&mut c_int>,
    empty_is_error: bool,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut result: *mut c_char = c"".as_ptr().cast_mut();
        let mut resultbuf: Option<Vec<c_char>> = None;
        let mut resultlen: usize;
        let mut valid = VALID_HEAD | VALID_PATH;
        let mut tilde_file = false;
        let mut skip_mod = false;
        let mut strbuf = [0 as c_char; 30];
        let mut use_shellslash = false;
        let arg_all_cstr: Option<std::ffi::CString>;

        *errormsg = ptr::null();
        if let Some(escaped) = escaped.as_deref_mut() {
            *escaped = c_int::from(false);
        }

        let spec_idx = find_cmdline_var(src, &mut *usedlen);
        if spec_idx < 0 {
            *usedlen = 1;
            return None;
        }

        if (src as usize) > (srcstart as usize) && *src.sub(1) == b'\\' as c_char {
            *usedlen = 0;
            ptr::copy(src, src.sub(1), CStr::from_ptr(src).to_bytes_with_nul().len());
            return None;
        }

        if spec_idx == SPEC_CWORD || spec_idx == SPEC_CCWORD || spec_idx == SPEC_CEXPR {
            let find_type = if spec_idx == SPEC_CWORD {
                FIND_IDENT | FIND_STRING
            } else if spec_idx == SPEC_CEXPR {
                FIND_IDENT | FIND_STRING | FIND_EVAL
            } else {
                FIND_STRING
            };
            resultlen = find_ident_under_cursor(&raw mut result, find_type, None);
            if resultlen == 0 {
                *errormsg = c"".as_ptr();
                return None;
            }
        } else {
            match spec_idx {
                SPEC_PERC => {
                    if (*g_st().curbuf).b_fname.is_null() {
                        result = c"".as_ptr().cast_mut();
                        valid = 0;
                    } else {
                        result = (*g_st().curbuf).b_fname;
                        tilde_file = crate::memory::cstr_as_bytes(result) == b"~";
                    }
                    use_shellslash = true;
                }
                SPEC_HASH => {
                    if *src.add(1) == b'#' as c_char {
                        arg_all_cstr = Some(arg_all());
                        result = arg_all_cstr.as_ref().unwrap().as_ptr().cast_mut();
                        *usedlen = 2;
                        if let Some(escaped) = escaped {
                            *escaped = c_int::from(true);
                        }
                        skip_mod = true;
                    } else {
                        use_shellslash = true;
                        let mut s = src.add(1);
                        if *s == b'<' as c_char {
                            s = s.add(1);
                        }
                        let i = getdigits_int(&raw mut s, false, 0);
                        if s == src.add(2) && *src.add(1) == b'-' as c_char {
                            s = s.sub(1);
                        }
                        *usedlen = (s as usize) - (src as usize);

                        if *src.add(1) == b'<' as c_char && i != 0 {
                            if *usedlen < 2 {
                                *usedlen = 1;
                                return None;
                            }
                            result =
                                tv_list_find_str(get_vim_var_list(VV_OLDFILES), i - 1).cast_mut();
                            if result.is_null() {
                                *errormsg = c"".as_ptr();
                                return None;
                            }
                        } else {
                            if i == 0 && *src.add(1) == b'<' as c_char && *usedlen > 1 {
                                *usedlen = 1;
                            }
                            let buf = buflist_findnr(i);
                            if buf.is_null() {
                                *errormsg = gettext(
                                    c"E194: No alternate file name to substitute for '#'",
                                );
                                return None;
                            }
                            if let Some(lnump) = lnump {
                                *lnump = ECMD_LAST;
                            }
                            if (*buf).b_fname.is_null() {
                                result = c"".as_ptr().cast_mut();
                                valid = 0;
                            } else {
                                result = (*buf).b_fname;
                                tilde_file = crate::memory::cstr_as_bytes(result) == b"~";
                            }
                        }
                    }
                }
                SPEC_CFILE => {
                    let Some(cfile) = file_name_at_cursor(FNAME_MESS | FNAME_HYP, 1, None) else {
                        *errormsg = c"".as_ptr();
                        return None;
                    };
                    let mut v: Vec<c_char> = cfile
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    result = v.as_mut_ptr();
                    resultbuf = Some(v);
                }
                SPEC_AFILE => {
                    use_shellslash = !g_st().autocmd_fname_full;
                    if !g_st().autocmd_fname_full
                        && let Some(ref cur_fname) = g_st().autocmd_fname
                    {
                        g_st().autocmd_fname_full = true;
                        if let Some(full) = FullName_save(cur_fname.as_ptr(), false) {
                            g_st().autocmd_fname = Some(full);
                        }
                    }
                    result = (&raw const g_st().autocmd_fname)
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                    if result.is_null() {
                        *errormsg =
                            gettext(e_no_autocommand_file_name_to_substitute_for_afile);
                        return None;
                    }
                    result = path_try_shorten_fname(result);
                }
                SPEC_ABUF => {
                    if g_st().autocmd_bufnr <= 0 {
                        *errormsg =
                            gettext(e_no_autocommand_buffer_number_to_substitute_for_abuf);
                        return None;
                    }
                    let _ = vim_snprintf!(strbuf.as_mut_ptr(), 30, c"%d", g_st().autocmd_bufnr);
                    result = strbuf.as_mut_ptr();
                }
                SPEC_AMATCH => {
                    result = (&raw const g_st().autocmd_match)
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                    if result.is_null() {
                        *errormsg =
                            gettext(e_no_autocommand_match_name_to_substitute_for_amatch);
                        return None;
                    }
                }
                SPEC_SFILE => {
                    use_shellslash = true;
                    let Some(sfile) = estack_sfile(ESTACK_SFILE) else {
                        *errormsg = gettext(e_no_source_file_name_to_substitute_for_sfile);
                        return None;
                    };
                    let mut v: Vec<c_char> = sfile
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    result = v.as_mut_ptr();
                    resultbuf = Some(v);
                }
                SPEC_STACK => {
                    use_shellslash = true;
                    let Some(stack) = estack_sfile(ESTACK_STACK) else {
                        *errormsg = gettext(e_no_call_stack_to_substitute_for_stack);
                        return None;
                    };
                    let mut v: Vec<c_char> = stack
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    result = v.as_mut_ptr();
                    resultbuf = Some(v);
                }
                SPEC_SCRIPT => {
                    use_shellslash = true;
                    let Some(script) = estack_sfile(ESTACK_SCRIPT) else {
                        *errormsg =
                            gettext(e_no_script_file_name_to_substitute_for_script);
                        return None;
                    };
                    let mut v: Vec<c_char> = script
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    result = v.as_mut_ptr();
                    resultbuf = Some(v);
                }
                SPEC_SLNUM => {
                    if sourcing_name().is_null() || *sourcing_lnum_ptr() == 0 {
                        *errormsg = gettext(e_no_line_number_to_use_for_slnum);
                        return None;
                    }
                    let _ = vim_snprintf!(
                        strbuf.as_mut_ptr(),
                        30,
                        c"%d",
                        *sourcing_lnum_ptr() as c_int,
                    );
                    result = strbuf.as_mut_ptr();
                }
                SPEC_SFLNUM => {
                    let sflnum = g_st().current_sctx.sc_lnum + *sourcing_lnum_ptr();
                    if sflnum == 0 {
                        *errormsg = gettext(e_no_line_number_to_use_for_sflnum);
                        return None;
                    }
                    let _ = vim_snprintf!(strbuf.as_mut_ptr(), 30, c"%d", sflnum as c_int);
                    result = strbuf.as_mut_ptr();
                }
                SPEC_SID => {
                    if g_st().current_sctx.sc_sid <= 0 {
                        *errormsg = gettext(e_usingsid);
                        return None;
                    }
                    let _ = vim_snprintf!(
                        strbuf.as_mut_ptr(),
                        30,
                        c"<SNR>%d_",
                        g_st().current_sctx.sc_sid,
                    );
                    result = strbuf.as_mut_ptr();
                }
                _ => {
                    *errormsg = c"".as_ptr();
                }
            }

            resultlen = crate::memory::cstr_len(result);
            if *src.add(*usedlen) == b'<' as c_char {
                *usedlen += 1;
                let s = crate::memory::cstr_as_bytes(result).iter().rposition(|&b| b == b'.').map_or(ptr::null_mut(), |i| result.add(i));
                if !s.is_null() && s >= path_tail(result) {
                    resultlen = (s as usize) - (result as usize);
                }
            } else if !skip_mod {
                valid |= modify_fname(
                    src,
                    tilde_file,
                    usedlen,
                    &raw mut result,
                    &mut resultbuf,
                    &mut resultlen,
                    use_shellslash,
                );
                if result.is_null() {
                    *errormsg = c"".as_ptr();
                    return None;
                }
            }
        }

        let ret = if resultlen == 0 || valid != VALID_HEAD + VALID_PATH {
            if empty_is_error {
                if valid != VALID_HEAD + VALID_PATH {
                    *errormsg = gettext(
                        c"E499: Empty file name for '%' or '#', only works with \":p:h\"",
                    );
                } else {
                    *errormsg = gettext(c"E500: Evaluates to an empty string");
                }
            }
            None
        } else {
            Some(cstr_from_bytes(std::slice::from_raw_parts(
                result.cast::<u8>(),
                resultlen,
            )))
        };
        drop(resultbuf);
        ret
    }
}

pub fn expand_sfile(arg: *mut c_char) -> Option<std::ffi::CString> {
    unsafe {
        let mut result_vec: Vec<c_char> = cstr_to_cstring(arg)
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();
        let mut idx: usize = 0;

        while result_vec[idx] != NUL {
            let p = result_vec.as_mut_ptr().add(idx);
            if !(crate::memory::cstr_prefix_bytes(p, 7) == &b"<sfile>"[..7]) {
                idx += 1;
            } else {
                let mut srclen: usize = 0;
                let mut errormsg: *const c_char = ptr::null();
                let repl = eval_vars(
                    p,
                    result_vec.as_ptr(),
                    &mut srclen,
                    None,
                    &mut errormsg,
                    None,
                    true,
                );
                if !errormsg.is_null() {
                    if *errormsg != NUL {
                        let _ = emsg(errormsg);
                    }
                    return None;
                }
                let Some(repl_cstr) = repl else {
                    idx += srclen;
                    continue;
                };
                let repl_bytes = repl_cstr.as_bytes();
                let tail_slice = &result_vec[(idx + srclen)..];
                let mut new_vec =
                    Vec::with_capacity(idx + repl_bytes.len() + tail_slice.len());
                new_vec.extend_from_slice(&result_vec[..idx]);
                new_vec.extend(repl_bytes.iter().map(|&b| b as c_char));
                new_vec.extend_from_slice(tail_slice);
                result_vec = new_vec;
                idx += repl_bytes.len();
            }
        }

        Some(cstr_to_cstring(result_vec.as_ptr()))
    }
}

fn ex_shada(eap: &mut exarg_T) {
    unsafe {
        let save_shada = opt_vars().p_shada;
        if *opt_vars().p_shada == NUL {
            opt_vars().p_shada = c"'100".as_ptr().cast_mut();
        }
        if eap.cmdidx == cmdidx_T::CMD_rviminfo as c_int
            || eap.cmdidx == cmdidx_T::CMD_rshada as c_int
        {
            shada_read_everything(eap.arg, eap.forceit != 0, false);
        } else {
            shada_write_file(eap.arg, eap.forceit != 0);
        }
        opt_vars().p_shada = save_shada;
    }
}

fn ex_filetype(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg == NUL {
            let _ = smsg!(
                0,
                c"filetype detection:%s  plugin:%s  indent:%s",
                if docmd_st().filetype_detect == kTrue {
                    c"ON".as_ptr()
                } else {
                    c"OFF".as_ptr()
                },
                if docmd_st().filetype_plugin == kTrue {
                    if docmd_st().filetype_detect == kTrue {
                        c"ON".as_ptr()
                    } else {
                        c"(on)".as_ptr()
                    }
                } else {
                    c"OFF".as_ptr()
                },
                if docmd_st().filetype_indent == kTrue {
                    if docmd_st().filetype_detect == kTrue {
                        c"ON".as_ptr()
                    } else {
                        c"(on)".as_ptr()
                    }
                } else {
                    c"OFF".as_ptr()
                },
            );
            return;
        }

        let mut arg = eap.arg;
        let mut plugin = false;
        let mut indent = false;

        loop {
            if crate::memory::cstr_prefix_bytes(arg, 6) == &b"plugin"[..6] {
                plugin = true;
                arg = skipwhite(arg.add(6));
                continue;
            }
            if crate::memory::cstr_prefix_bytes(arg, 6) == &b"indent"[..6] {
                indent = true;
                arg = skipwhite(arg.add(6));
                continue;
            }
            break;
        }

        if crate::memory::cstr_as_bytes(arg) == b"on" || crate::memory::cstr_as_bytes(arg) == b"detect" {
            if *arg == b'o' as c_char || docmd_st().filetype_detect != kTrue {
                source_runtime(FILETYPE_FILE.as_ptr().cast_mut(), DIP_ALL);
                docmd_st().filetype_detect = kTrue;
                if plugin {
                    source_runtime(FTPLUGIN_FILE.as_ptr().cast_mut(), DIP_ALL);
                    docmd_st().filetype_plugin = kTrue;
                }
                if indent {
                    source_runtime(INDENT_FILE.as_ptr().cast_mut(), DIP_ALL);
                    docmd_st().filetype_indent = kTrue;
                }
            }
            if *arg == b'd' as c_char {
                do_doautocmd(
                    c"filetypedetect BufRead".as_ptr().cast_mut(),
                    true,
                    None,
                );
                do_modelines(0);
            }
        } else if crate::memory::cstr_as_bytes(arg) == b"off" {
            if plugin || indent {
                if plugin {
                    source_runtime(FTPLUGOF_FILE.as_ptr().cast_mut(), DIP_ALL);
                    docmd_st().filetype_plugin = kFalse;
                }
                if indent {
                    source_runtime(INDOFF_FILE.as_ptr().cast_mut(), DIP_ALL);
                    docmd_st().filetype_indent = kFalse;
                }
            } else {
                source_runtime(FTOFF_FILE.as_ptr().cast_mut(), DIP_ALL);
                docmd_st().filetype_detect = kFalse;
            }
        } else {
            let _ = semsg!(gettext(e_invarg2), arg);
        }
    }
}

pub fn ex_errmsg(msg: &std::ffi::CStr, arg: *const c_char) -> *mut c_char {
    let buf = (&raw mut docmd_st().EX_ERROR_BUF).cast::<c_char>();
        let _ = vim_snprintf!(buf, MSG_BUF_LEN, gettext(msg.as_ptr()), arg);
        buf
}

pub fn filetype_plugin_enable() {
    if docmd_st().filetype_plugin == kNone {
            source_runtime(FTPLUGIN_FILE.as_ptr().cast_mut(), DIP_ALL);
            docmd_st().filetype_plugin = kTrue;
        }
        if docmd_st().filetype_indent == kNone {
            source_runtime(INDENT_FILE.as_ptr().cast_mut(), DIP_ALL);
            docmd_st().filetype_indent = kTrue;
        }
}

pub fn filetype_maybe_enable() {
    if docmd_st().filetype_detect == kNone {
            source_runtime(FILETYPE_FILE.as_ptr().cast_mut(), DIP_ALL);
            docmd_st().filetype_detect = kTrue;
        }
}

fn ex_setfiletype(eap: &mut exarg_T) {
    unsafe {
        if (*g_st().curbuf).b_did_filetype {
            return;
        }

        let mut arg = eap.arg;
        if crate::memory::cstr_prefix_bytes(arg, 9) == &b"FALLBACK "[..9] {
            arg = arg.add(9);
        }

        set_option_value_give_err(kOptFiletype, cstr_as_obj(arg), OPT_LOCAL);
        if arg != eap.arg {
            (*g_st().curbuf).b_did_filetype = false;
        }
    }
}

fn ex_digraphs(eap: &mut exarg_T) {
    unsafe {
        if *eap.arg != NUL {
            putdigraph(eap.arg);
        } else {
            listdigraphs(eap.forceit != 0);
        }
    }
}

pub fn set_no_hlsearch(flag: bool) {
    g_st().Search.no_hlsearch = flag;
        set_vim_var_nr(
            VV_HLSEARCH,
            varnumber_T::from(!g_st().Search.no_hlsearch && opt_vars().p_hls != 0),
        );
}

fn ex_nohlsearch(_eap: &mut exarg_T) {
    set_no_hlsearch(true);
        redraw_all_later(UPD_SOME_VALID);
}

fn ex_fold(eap: &mut exarg_T) {
    unsafe {
        if foldManualAllowed(true) != 0 {
            let start = pos_T {
                lnum: eap.line1,
                col: 1,
                coladd: 0,
            };
            let end = pos_T {
                lnum: eap.line2,
                col: 1,
                coladd: 0,
            };
            foldCreate(&mut **curwin.as_ptr(), start, end);
        }
    }
}

fn ex_foldopen(eap: &mut exarg_T) {
    let start = pos_T {
            lnum: eap.line1,
            col: 1,
            coladd: 0,
        };
        let end = pos_T {
            lnum: eap.line2,
            col: 1,
            coladd: 0,
        };
        opFoldRange(
            start,
            end,
            c_int::from(eap.cmdidx == cmdidx_T::CMD_foldopen as c_int),
            eap.forceit,
            false,
        );
}

fn ex_folddo(eap: &mut exarg_T) {
    unsafe {
        let mut lnum = eap.line1;
        while lnum <= eap.line2 {
            if hasFolding(*curwin.as_ptr(), lnum, None, ptr::null_mut())
                == (eap.cmdidx == cmdidx_T::CMD_folddoclosed as c_int)
            {
                ml_setmarked(lnum);
            }
            lnum += 1;
        }

        global_exe(eap.arg);
        ml_clearmarked();
    }
}

pub fn is_loclist_cmd(cmdidx: c_int) -> bool {
    if !(0..CMD_SIZE_I32).contains(&cmdidx) {
        return false;
    }
    CMDNAMES[cmdidx as usize].cmd_name.to_bytes().first() == Some(&b'l')
}

fn ex_checkhealth(eap: &mut exarg_T) {
     {
        g_st().emsg_off += 1;
        let ok = nlua_call_excmd(
            c"vim.health",
            c"_check",
            eap,
            &g_st().cmdmod,
            None,
        );
        g_st().emsg_off -= 1;
        if ok {
            return;
        }

        let Some(vimruntime_env) = os_getenv_cstr(c"VIMRUNTIME") else {
            let _ = emsg(gettext(c"E5009: $VIMRUNTIME is empty or unset"));
            return;
        };
        let vr_bytes = vimruntime_env.to_bytes();
            let rtp_ok = vr_bytes.is_empty() || crate::memory::cstr_as_bytes(opt_vars().p_rtp).windows(vr_bytes.len()).any(|w| w == vr_bytes);
        if rtp_ok {
            let _ = semsg!(
                gettext(c"E5009: Invalid $VIMRUNTIME: %s"),
                vimruntime_env.as_ptr(),
            );
        } else {
            let _ = emsg(gettext(c"E5009: Invalid 'runtimepath'"));
        }
    }
}

fn ex_terminal(eap: &mut exarg_T) {
    unsafe {
        let scroll_save = g_st().msg_scroll;
        g_st().msg_scroll = 0;
        autowrite_all();
        g_st().msg_scroll = scroll_save;

        if *eap.arg != NUL {
            nlua_call_excmd(
                c"vim._core.ex_cmd",
                c"ex_terminal",
                eap,
                &g_st().cmdmod,
                None,
            );
        } else {
            if *opt_vars().p_sh == NUL {
                let _ = emsg(gettext(e_shellempty));
                return;
            }
            let argv = shell_build_argv(ptr::null(), ptr::null());
            let mut shell_tv: typval_T = typval_T::new();
            tv_list_alloc_ret(&mut shell_tv, 0);
            for arg in argv {
                tv_list_append_string(
                    shell_tv.vval.v_list(),
                    arg.as_ptr(),
                    arg.as_bytes().len() as isize,
                );
            }
            nlua_call_excmd(
                c"vim._core.ex_cmd",
                c"ex_terminal",
                eap,
                &g_st().cmdmod,
                Some(&mut shell_tv),
            );
            tv_clear(&raw mut shell_tv);
        }
    }
}

fn ex_log(eap: &mut exarg_T) {
    nlua_call_excmd(
            c"vim._core.ex_cmd",
            c"ex_log",
            eap,
            &g_st().cmdmod,
            None,
        );
}

fn ex_lsp(eap: &mut exarg_T) {
    nlua_call_excmd(
            c"vim._core.ex_cmd",
            c"ex_lsp",
            eap,
            &g_st().cmdmod,
            None,
        );
}

fn ex_packdel(eap: &mut exarg_T) {
    nlua_call_excmd(
            c"vim._core.ex_cmd",
            c"ex_packdel",
            eap,
            &g_st().cmdmod,
            None,
        );
}

fn ex_packupdate(eap: &mut exarg_T) {
    nlua_call_excmd(
            c"vim._core.ex_cmd",
            c"ex_packupdate",
            eap,
            &g_st().cmdmod,
            None,
        );
}

fn ex_uptime(eap: &mut exarg_T) {
    nlua_call_excmd(
            c"vim._core.ex_cmd",
            c"ex_uptime",
            eap,
            &g_st().cmdmod,
            None,
        );
}

fn ex_fclose(eap: &mut exarg_T) {
    win_float_remove(eap.forceit != 0, eap.line1);
}

pub fn verify_command(cmd: *mut c_char) {
    const SMILE_LINES: [&CStr; 105] = [
        c" #xxn`          #xnxx`        ,+x@##@Mz;`        .xxxxxxxxxnz+,      znnnnnnnnnnnnnnnn.",
        c" n###z          x####`      :x##########W+`      ,#############M;    W################.",
        c" n####;         x####`    `z##############W:     ,################   W################.",
        c" n####W.        x####`   ,W#################+    ,#################  W################.",
        c" n#####n        x####`   @###################    ,#################i W################.",
        c" n######i       x####`  .#########@W@########*   ,#################W`W################.",
        c" n######@.      x####`  x######W*.  `;n#######:  ,####x,,,,:*M######iW###@:,,,,,,,,,,,`",
        c" n#######n      x####` *######+`       :M#####M  ,####n      `x#####xW###@`",
        c" n########*     x####``@####@;          `x#####i ,####n       ,#####@W###@`",
        c" n########@     x####`*#####i            `M####M ,####n        x#########@`",
        c" n#########     x####`M####z              :#####:,####n        z#########@`",
        c" n#########*    x####,#####.               n####+,####n        n#########@`",
        c" n####@####@,   x####i####x                ;####x,####n       `W#####@####+++++++++++i",
        c" n####*#####M`  x#########*                `####@,####n       i#####MW###############W",
        c" n####.######+  x####z####;                 W####,####n      i@######W###############W",
        c" n####.`W#####: x####n####:                 M####:####@nnnnnW#######,W###############W",
        c" n####. :#####M`x####z####;                 W####,#################z W###############W",
        c" n####.  #######x#########*                `####W,################W` W###############W",
        c" n####.  `M#####W####i####x                ;####x,###############W,  W####+**********i",
        c" n####.   ,##########,#####.               n####+,##############n.   W###@`",
        c" n####.    ##########`M####z              :#####:,###########Wz:     W###@`",
        c" n####.    x#########`*#####i            `M####M ,####x.....`        W###@`",
        c" n####.    ,@########``@####@;          `x#####i ,####n              W###@`",
        c" n####.     *########` *#####@+`       ,M#####M  ,####n              W###@`",
        c" n####.      x#######`  x######W*.  `;n######@:  ,####n              W###@,,,,,,,,,,,,`",
        c" n####.      .@######`  .#########@W@########*   ,####n              W################,",
        c" n####.       i######`   @###################    ,####n              W################,",
        c" n####.        n#####`   ,W#################+    ,####n              W################,",
        c" n####.        .@####`    .n##############W;     ,####n              W################,",
        c" n####.         i####`      :x##########W+`      ,####n              W################,",
        c" +nnnn`          +nnn`        ,+x@##@Mz;`        .nnnn+              zxxxxxxxxxxxxxxxx.",
        c" ",
        c"                                                                                  ,+M@#Mi",
        c"                                                                                 .z########",
        c"                                                                                i@#########i",
        c"                                                                              `############W`",
        c"                                                                             `n#############i",
        c"                                                                            `n##############n",
        c"     ``                                                                     z###############@`",
        c"    `W@z,                                                                  ##################,",
        c"    *#####`                                                               i############@x@###i",
        c"    ######M.                                                             :#############n`,W##+",
        c"    +######@:                                                           .W#########M@##+  *##z",
        c"    :#######@:                                                         `x########@#x###*  ,##n",
        c"    `@#######@;                                                        z#########M*@nW#i  .##x",
        c"     z########@i                                                      *###########WM#@#,  `##x",
        c"     i##########+                                                    ;###########*n###@   `##x",
        c"     `@#MM#######x,                                                 ,@#########zM,`z##M   `@#x",
        c"      n##M#W#######n.               `.:i*+#zzzz##+i:.`             ,W#########Wii,`n@#@` n@##n",
        c"      ;###@#x#######n         `,i#nW@#####@@WWW@@####@Mzi.        ,W##########@z.. ;zM#+i####z",
        c"       x####nz########    .;#x@##@Wn#*;,.`      ``,:*#x@##M+,    ;@########xz@WM+#` `n@#######",
        c"       ,@####M########xi#@##@Mzi,`                     .+x###Mi:n##########Mz```.:i  *@######*",
        c"        *#####W#########ix+:`                             :n#############z:       `*.`M######i",
        c"        i#W##nW@+@##@#M@;                                   ;W@@##########W,        i`x@#####,",
        c"        `@@n@Wn#@iMW*#*:                                     `iz#z@######x.           M######`",
        c"         z##zM###x`*, .`                                          `iW#####W;:`        +#####M",
        c"         ,###nn##n`                                                ,#####x;`        ,;@######",
        c"          x###xz#.                                                  in###+        `:######@.",
        c"          ;####n+                                                    `Mnx##xi`   , zM#######",
        c"          `W####+                i.                                   `.+x###@#. :n,z######:",
        c"           z####@`              ;#:                                     .ii@###@;.*M*z####@`",
        c"           i####M         `   `i@#,           ::                           +#n##@+@##W####n",
        c"           :####x    ,i. ##xzM###@`     i.   .@@,                           .z####x#######*",
        c"           ,###W;   i##Wz#########     :##   z##n                           ,@########x###:",
        c"            n##n   `W###########M`;n,  i#x  ,###@i                           *W########W#@`",
        c"           .@##+  `x###########@. z#+ .M#W``x#####n`                         `;#######@z#x",
        c"           n###z :W############@  z#*  @##xM#######@n;                        `########nW+",
        c"          ;####nW##############W :@#* `@#############*                        :########z@i`",
        c"          M##################### M##:  @#############@:                       *W########M#",
        c"         ;#####################i.##x`  W#############W,                       :n########zx",
        c"         x####################@.`x;    @#############z.                       .@########W#",
        c"        ,######################`       W###############x*,`                    W######zM#i",
        c"        #######################:       z##################@x+*#zzi            `@#########.",
        c"        W########W#z#M#########;       *##########################z            :@#######@`",
        c"       `@#######x`;#z ,x#######;       z###########M###xnM@########*            :M######@",
        c"       i########, x#@`  z######;       *##########i *#@`  `+########+`            n######.",
        c"       n#######@` M##,  `W#####.       *#########z  ###;    z########M:           :W####n",
        c"       M#######M  n##.   x####x        `x########:  z##+    M#########@;           .n###+",
        c"       W#######@` :#W   `@####:         `@######W   i###   ;###########@.            n##n",
        c"       W########z` ,,  .x####z           @######@`  `W#;  `W############*            *###;",
        c"      `@#########Mi,:*n@####W`           W#######*   ..  `n#############i            i###x",
        c"      .#####################z           `@#######@*`    .x############n:`            ;####.",
        c"      :####################x`,,`        `W#########@x#+#@#############i              ,####:",
        c"      ;###################x#@###xi`      *############################:              `####i",
        c"      i##################+########M,      x##########################@`               W###i",
        c"      *################@; @########@,     .W#########################@                x###:",
        c"      .+M#############z.  M#########x      ,W########################@`               ####.",
        c"      *M*;z@########x:    :W#######i        .M########################i               i###:",
        c"      *##@z;#@####x:        :z###@i          `########################x               .###;",
        c"      *#####n;#@##            ;##*             ,x#####################@`               W##*",
        c"      *#######n;*            :M##W*,             *W####################`               n##z",
        c"      i########@.         ,*n#######M*`           `###################M                *##M",
        c"      i########n        `z#####@@#####Wi            ,M################;                ,##@`",
        c"      ;WMWW@###*       .x##@ni.``.:+zW##z`           `n##############z                  @##,",
        c"      .*++*i;;;.      .M#@+`          .##n            `x############x`                  n##i",
        c"      :########*      x#W,              *#+            *###########M`                   +##+",
        c"      ,#########     :#@:                ##:           #nzzzzzzzzzz.                    :##x",
        c"      .#####Wz+`     ##+                 `MM`          .znnnnnnnnn.                     `@#@`",
        c"      `@@ni;*nMz`    @W`                  :#+           .x#######n                       x##,",
        c"       i;z@#####,   .#*                    z#:           ;;;*zW##;                       ###i",
        c"       z########:   :#;                    `Wx          +###Wni;n.                       ;##z",
        c"       n########W:  .#*                     ,#,        ;#######@+                        `@#M",
        c"      .###########n;.MM                      n*        ;iM#######*                        x#@`",
        c"      :#############@;;                      .n`      ,#W*iW#####W`                       +##,",
        c"      ,##############.                        ix.    `x###M;#######                       ,##i",
    ];
    const SMILE_LINES_TAIL: [&CStr; 34] = [
        c"      .#############@`                         x@n**#W######z;M###@.                       W##",
        c"      .##############W:                        .x############@*;zW#;                       z#x",
        c"      ,###############@;                        `##############@n*;.                       i#@",
        c"      ,#################i                         :n##############W`                       .##,",
        c"      ,###################`                         .+W##########W,                        `##i",
        c"      :###################@zi,`                        ;zM@@@WMn*`                          @#z",
        c"      :#######################@x+*i;;:i#M,                 ``                               M#W",
        c"      ;################################@x.                                                  n##,",
        c"      i#####################@W@@@@Wxz*:`                                                    *##+",
        c"      *######################+```                                                           :##M",
        c"      ########################M;                                                            `@##,",
        c"      z#########################x,                                                           z###",
        c"      n###########################n:                                                         ;##W`",
        c"      x#############################Mz#++##*                                                 `W##i",
        c"      M####################################@`                                                 ###x",
        c"      W#####################################`                                                 .###,",
        c"      @####################################M                                                   n##z",
        c"      @##################z*i@WMMMx#x@#####,.                                                   :##@.",
        c"     `#####################@xi`     `::,*                                                       x##+",
        c"     .#####################@#M.                                                                 ;##@`",
        c"     ,#####################:.                                                                    M##i",
        c"     ;###################ni`                                                                     i##M",
        c"     *#################W#`                                                                       `W##,",
        c"     z#################@Wx+.                                                                      +###",
        c"     x######################z.                                                                    .@#@`",
        c"    `@#######################@;                                                                    z##;",
        c"    :##########################:                                                                   :##z",
        c"    +#########################W#                                                                    M#W",
        c"    W################@n+*i;:,`                                                                      +##,",
        c"   :##################WMxz+,                                                                        ,##i",
        c"   n#######################W..,                                                                      W##",
        c"  +#########################WW@+. .:.                                                                z#x",
        c" `@#############################@@###:                                                               *#W",
        c" #################################Wz:                                                                :#@",
    ];
    const SMILE_LINES_END: [&CStr; 3] = [
        c",@###############################i                                                                   .##",
        c"n@@@@@@@#########################+                                                                   `##",
        c"`      `.:.`.,:iii;;;;;;;;iii;;;:`       `.``                                                        `nW",
    ];

     {
        if b"smile" != crate::memory::cstr_as_bytes(cmd) {
            return;
        }
        for line in &SMILE_LINES {
            let _ = msg(line.as_ptr(), HLF_E);
        }
        for line in &SMILE_LINES_TAIL {
            let _ = msg(line.as_ptr(), HLF_E);
        }
        for line in &SMILE_LINES_END {
            let _ = msg(line.as_ptr(), HLF_E);
        }
    }
}

pub fn is_map_cmd(cmdidx: cmdidx_T) -> bool {
    if is_user_cmdidx(cmdidx) {
        return false;
    }

    let func = CMDNAMES[cmdidx as usize].cmd_func;
    ptr::fn_addr_eq(func, ex_map as ex_func_T)
        || ptr::fn_addr_eq(func, ex_unmap as ex_func_T)
        || ptr::fn_addr_eq(func, ex_mapclear as ex_func_T)
        || ptr::fn_addr_eq(func, ex_abbreviate as ex_func_T)
        || ptr::fn_addr_eq(func, ex_abclear as ex_func_T)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_excmd_identifies_command_terminators() {
        assert_ne!(ends_excmd(0), 0);
        assert_ne!(ends_excmd(b'|' as c_int), 0);
        assert_ne!(ends_excmd(b'"' as c_int), 0);
        assert_ne!(ends_excmd(b'\n' as c_int), 0);
        assert_eq!(ends_excmd(b'a' as c_int), 0);
    }

    #[test]
    fn cmd_has_expr_args_identifies_expr_commands() {
        assert!(cmd_has_expr_args(cmdidx_T::CMD_execute));
        assert!(cmd_has_expr_args(cmdidx_T::CMD_echo));
        assert!(cmd_has_expr_args(cmdidx_T::CMD_echon));
        assert!(cmd_has_expr_args(cmdidx_T::CMD_echomsg));
        assert!(cmd_has_expr_args(cmdidx_T::CMD_echoerr));
        assert!(!cmd_has_expr_args(cmdidx_T::CMD_edit));
    }

    #[test]
    fn find_cmdline_var_matches_special_variables() {
        let mut usedlen = 0usize;
            assert_eq!(
                find_cmdline_var(c"<cword>".as_ptr(), &mut usedlen),
                SPEC_CWORD
            );
            assert_eq!(usedlen, 7);
            assert_eq!(
                find_cmdline_var(c"%:p".as_ptr(), &mut usedlen),
                SPEC_PERC
            );
            assert_eq!(usedlen, 1);
            assert_eq!(find_cmdline_var(c"foo".as_ptr(), &mut usedlen), -1);
    }
}
