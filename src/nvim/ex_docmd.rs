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
    EVENT_QUITPRE, EVENT_TABNEWENTERED, HLF_T, K_OPT_EVENTIGNORE, PROF_YES, apply_autocmds,
    apply_autocmds_win, check_nomodeline, do_augroup, do_autocmd, do_doautocmd, ex_doautoall,
    has_event, may_trigger_vim_suspend_resume,
};
use crate::buffer::{
    BF_DUMMY, Bufref, DOBUF_CURRENT, DOBUF_DEL, DOBUF_FIRST, DOBUF_LAST, DOBUF_MOD, DOBUF_UNLOAD,
    DOBUF_WIPE, MAXLNUM, bt_prompt, bt_quickfix, buf_hide, buf_spname, buflist_findnr,
    buflist_findpat, buflist_list, bufref_valid, do_bufdel, do_modelines, ex_buffer_all,
    get_highest_fnum, goto_buffer, maketitle, no_write_message, otherfile, set_bufref, setaltfname,
    setfname,
};
use crate::channel::{
    CallbackReader, K_CHANNEL_STDIN_PIPE_CONST, channel_event, channel_job_start,
};
use crate::charset::{
    CTRL_V_CONST, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_islower, ascii_isspace,
    ascii_isupper, ascii_iswhite, backslash_halve, getdigits, getdigits_int, getdigits_int32,
    skipdigits, skiptowhite_esc, skipwhite, tolower_asc,
};
use crate::clipboard::{end_batch_changes, start_batch_changes};
use crate::cmdexpand::{
    CmdAddr, Cmdidx, CompleteListItemGetter, EX_NOTRLCOM, EXPAND_FILES, EXPAND_NOTHING,
    K_UICMDLINE, LineGetter, WILD_ADD_SLASH, WILD_EXPAND_FREE, WILD_ICASE, WILD_LIST_NOTFOUND,
    WILD_NOERROR, expand_generic, expand_one,
};
use crate::cmdhist::ex_history;
use crate::context::{CdCause, ctx_did_chdir, is_ctx_win};
use crate::cursor::{CursorPos, Linenr, check_cursor, check_cursor_col};
use crate::debugger::{
    dbg_breakpoint, dbg_check_breakpoint, dbg_find_breakpoint, do_debug, ex_breakadd, ex_breakdel,
    ex_breaklist, ex_debug, ex_debuggreedy,
};
use crate::dialog::{VIM_YES, ask_yesno, vim_dialog_yesno};
use crate::diff::{
    ex_diffgetput, ex_diffoff, ex_diffpatch, ex_diffsplit, ex_diffthis, ex_diffupdate,
};
use crate::digraph::{
    K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST, ex_loadkeymap, listdigraphs, putdigraph,
};
use crate::drawscreen::{
    UPD_CLEAR, UPD_INVERTED, UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, clearmode, redraw_all_later,
    redraw_curbuf_later, redraw_later, redraw_statuslines, screen_resize, setcursor_mayforce,
    showmode, status_redraw_all, status_redraw_curbuf, update_screen,
};
use crate::eval::fs::{
    K_CD_SCOPE_BUFFER, K_CD_SCOPE_TABPAGE, K_CD_SCOPE_WINDOW, VALID_HEAD, VALID_PATH, modify_fname,
};
use crate::eval::funcs::{
    DOCMD_KEYTYPED, MT_LINE_WISE, RE_SEARCH, SEARCH_KEEP, VIM_QUESTION, create_environment,
};
use crate::eval::typval::{
    BoolVarValue, Callback, EvalFuncData, TvList, TypVal, TypvalVval, VarType, callback_free,
    tv_clear, tv_dict_add_str, tv_dict_alloc, tv_get_string, tv_list_alloc_ret,
    tv_list_append_string, tv_list_copy, tv_list_find, tv_list_find_str, tv_list_free,
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
    K_MARK_ALL, callback_call, eval_to_string, ex_echo, ex_echohl, ex_execute, get_copy_id,
    set_ref_in_callback, skip_expr,
};
use crate::event::r#loop::{loop_poll_events, loop_queues, uv_strerror};
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
use crate::event::proc::{proc_stop, proc_wait};
use crate::ex_cmds::{
    BL_FIX, BL_SOL, BL_WHITE, DIALOG_MSG_SIZE, ECMD_ADDBUF, ECMD_ALTBUF, ECMD_FORCEIT, ECMD_HIDE,
    ECMD_LAST, ECMD_OLDBUF, ECMD_ONE, EXFLAG_LIST, EXFLAG_NR, EXFLAG_PRINT, K_WIN_PREVIEW,
    RE_SUBST, SEARCH_HIS, do_ascii, do_bang, do_ecmd, do_move, do_wqall, do_write, ex_align,
    ex_append, ex_change, ex_copy, ex_file, ex_global, ex_oldfiles, ex_sort, ex_substitute,
    ex_substitute_preview, ex_uniq, ex_update, ex_wnext, ex_write, ex_z, global_exe,
    prepare_tagpreview, print_line, skip_vimgrep_pat,
};
use crate::ex_cmds2::{
    autowrite_all, check_changed, check_changed_any, check_fname, dialog_changed, ex_checktime,
    ex_compiler, ex_drop, ex_listdo, ex_perl, ex_perldo, ex_perlfile, ex_py3file, ex_pydo3,
    ex_python3, ex_ruby, ex_rubydo, ex_rubyfile,
};
use crate::ex_eval::{
    CSF_ACTIVE, CSF_CAUGHT, CSF_FINALLY, CSF_FOR, CSF_THROWN, CSF_TRUE, CSF_TRY, CSF_WHILE,
    CSL_HAD_CONT, CSL_HAD_ENDLOOP, CSL_HAD_FINA, CSL_HAD_LOOP, CSTP_ERROR, CSTP_INTERRUPT,
    CSTP_THROW, Cstack, ET_ERROR, ET_INTERRUPT, ET_USER, ExCleanup, Except, Msglist, PendingVal,
    aborting, cleanup_conditionals, discard_current_exception, do_errthrow, do_intthrow, do_throw,
    enter_cleanup, ex_break, ex_catch, ex_continue, ex_else, ex_endfunction, ex_endif, ex_endtry,
    ex_endwhile, ex_eval, ex_finally, ex_if, ex_throw, ex_try, ex_while, has_loop_cmd,
    leave_cleanup, report_make_pending, rewind_conditionals,
};
use crate::ex_getln::{
    allbuf_locked, cmdpreview_get_bufnr, cmdpreview_get_ns, curbuf_locked, get_text_locked_msg,
    getcmdline, script_get, text_locked, text_locked_msg, text_or_buf_locked,
    ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave,
};
use crate::ex_session::{ex_loadview, ex_mkrc};
use crate::file_search::{
    FNAME_HYP, FNAME_MESS, FfSearchCtx, do_autocmd_dirchanged, file_name_at_cursor,
    find_file_in_path, vim_chdir,
};
use crate::fileio::{FILE, readfile, shorten_fnames};
use crate::fold::{fold_create, fold_manual_allowed, has_folding, op_fold_range};
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
use crate::mbyte::{get_encoding_name, utf_head_off_slice, utfc_ptr2len, utfc_slice2len};
use crate::memline::{goto_byte, ml_clearmarked, ml_preserve, ml_recover, ml_setmarked};
use crate::memory::{ArenaMem, arena_mem_free, cstr_from_bytes, cstr_to_cstring, xstrlcpy};
use crate::menu::{SaveState, ex_emenu, ex_menu, ex_menutranslate};
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
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::msgpack_rpc::server::server_start;
use crate::normal::{
    FIND_EVAL, FIND_IDENT, FIND_STRING, do_check_scrollbind, end_visual_mode,
    find_ident_under_cursor, get_vtopline, normal_cmd, set_cursor_for_append_to_line,
};
use crate::ops::{OP_DELETE, OP_LSHIFT, OP_RSHIFT, OP_YANK, do_join, op_delete, op_shift};
use crate::option::{
    K_OPT_FILETYPE, OPT_LOCAL, OptInt, ex_set, get_findfunc, get_option_sctx, get_scrolloff_value,
    set_option_direct, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::optionstr::{check_ff_value, get_fileformat_name};
use crate::os::env::{
    ExpandCtx, expand_env, expand_env_esc, expand_env_save, home_replace, os_getenv_cstr,
};
use crate::os::fs::{os_chdir, os_dirname, os_fopen, os_mkdir, os_path_exists};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::lang::ex_language;
use crate::os::shell::shell_build_argv;
use crate::os::time::os_hrtime;
use crate::path::{
    FullName_save, K_PATH_CMP_LITERAL, path_equal, path_has_wildcard, path_tail,
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
    K_MARK_BUF_LOCAL, MAXCOL, NUMBUFLEN, RE_MAGIC, Regmatch, skip_regexp, vim_regcomp, vim_regfree,
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
    RawCCharPtr, RawCCharPtrMut, del_trailing_spaces, gettext, ngettext, strrep, vim_snprintf,
    vim_strchr, vim_strsave_escaped,
};
use crate::syntax::{ex_ownsyntax, ex_syntax, ex_syntime};
use crate::tag::{do_tag, do_tags};
use crate::textformat::Oparg;
use crate::ui::{
    ui_active, ui_busy_start, ui_busy_stop, ui_call_error_exit, ui_call_suspend, ui_cursor_shape,
    ui_flush, ui_has,
};
use crate::undo::{
    buf_is_changed_fn, curbuf_is_changed_fn, ex_undojoin, ex_undolist, u_clearline, u_compute_hash,
    u_read_undo, u_redo, u_save, u_undo, u_undo_and_forget, u_write_undo, uhp_ref, undo_time,
};
use crate::usercmd::{
    do_ucmd, ex_comclear, ex_command, ex_delcommand, expand_user_command_name, find_ucmd,
    get_user_command_name,
};
use crate::version::{ex_intro, ex_version};
use crate::window::{
    TabPageInfo, WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT, WinInfoInfo,
    check_can_set_curbuf_forceit, close_others, do_window, find_tabpage, goto_tabpage,
    only_one_window, tabpage_index, tabpage_move, trigger_tabclosedpre, update_cwd, valid_tabpage,
    win_close, win_close_othertab, win_enter, win_goto, win_new_tabpage, win_setheight_win,
    win_setwidth_win, win_split, win_valid, window_layout_locked,
};
use crate::winfloat::{win_float_close, win_float_remove};
use std::ffi::CStr;
use std::ptr;

#[cfg(windows)]
use crate::lua::executor::{K_RET_OBJECT, nlua_exec};
#[cfg(not(windows))]
use crate::msgpack_rpc::server::server_stop;
#[cfg(unix)]
use crate::os::fs::os_isdir;
#[cfg(windows)]
use crate::path::slash_adjust;

use crate::eval::typval::Varnumber;
use crate::extmark::TriState;
pub type CdScope = i32;
pub type ExFunc = fn(&mut Exarg);
pub type ExPreviewFunc = Option<fn(&mut Exarg, i32, ObjHandle) -> i32>;

pub(crate) const CMD_SIZE: i32 = 564;
pub(crate) const CMD_USER: i32 = -1;
pub(crate) const CMD_USER_BUF: i32 = -2;

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

pub(crate) const ADDR_LINES: CmdAddr = 0;
pub(crate) const ADDR_WINDOWS: CmdAddr = 1;
pub(crate) const ADDR_ARGUMENTS: CmdAddr = 2;
pub(crate) const ADDR_LOADED_BUFFERS: CmdAddr = 3;
pub(crate) const ADDR_BUFFERS: CmdAddr = 4;
pub(crate) const ADDR_TABS: CmdAddr = 5;
pub(crate) const ADDR_QUICKFIX: CmdAddr = 8;
pub(crate) const ADDR_OTHER: CmdAddr = 10;
pub(crate) const ADDR_NONE: CmdAddr = 11;

pub(crate) const CMOD_SANDBOX: i32 = 0x0001;
pub(crate) const CMOD_SILENT: i32 = 0x0002;
pub(crate) const CMOD_ERRSILENT: i32 = 0x0004;
pub(crate) const CMOD_UNSILENT: i32 = 0x0008;
pub(crate) const CMOD_NOAUTOCMD: i32 = 0x0010;
pub(crate) const CMOD_HIDE: i32 = 0x0020;
pub(crate) const CMOD_BROWSE: i32 = 0x0040;
pub(crate) const CMOD_CONFIRM: i32 = 0x0080;
pub(crate) const CMOD_KEEPALT: i32 = 0x0100;
pub(crate) const CMOD_KEEPMARKS: i32 = 0x0200;
pub(crate) const CMOD_KEEPJUMPS: i32 = 0x0400;
pub(crate) const CMOD_LOCKMARKS: i32 = 0x0800;
pub(crate) const CMOD_KEEPPATTERNS: i32 = 0x1000;
pub(crate) const CMOD_NOSWAPFILE: i32 = 0x2000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ExMagic {
    pub file: bool,
    pub bar: bool,
}

#[repr(C)]
#[derive(Clone)]
pub struct Exarg {
    pub arg: RawCCharPtrMut,
    pub args: Vec<RawCCharPtrMut>,
    pub arglens: Vec<usize>,
    pub nextcmd: RawCCharPtrMut,
    pub cmd: RawCCharPtrMut,
    pub cmdlinep: *mut RawCCharPtrMut,
    pub cmdline_buf: *mut Vec<u8>,
    pub cmdidx: i32,
    pub argt: u32,
    pub skip: i32,
    pub forceit: i32,
    pub addr_count: i32,
    pub line1: Linenr,
    pub line2: Linenr,
    pub addr_type: CmdAddr,
    pub flags: i32,
    pub do_ecmd_cmd: RawCCharPtrMut,
    pub do_ecmd_lnum: Linenr,
    pub append: i32,
    pub usefilter: i32,
    pub amount: i32,
    pub regname: i32,
    pub force_bin: i32,
    pub read_edit: i32,
    pub mkdir_p: i32,
    pub force_ff: i32,
    pub force_enc: i32,
    pub bad_char: i32,
    pub useridx: i32,
    pub errmsg: Option<&'static std::ffi::CStr>,
    pub ea_getline: LineGetter,
    pub cstack: *mut crate::ex_eval::Cstack,
    pub magic: ExMagic,
}

impl Exarg {
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
            errmsg: None,
            ea_getline: LineGetter::None,
            cstack: ptr::null_mut(),
            magic: ExMagic {
                file: false,
                bar: false,
            },
        }
    }

    pub fn set_cmdline_vec(&mut self, cmdlinep: *mut RawCCharPtrMut, new_buf: Vec<u8>) {
        unsafe {
            debug_assert!(!self.cmdline_buf.is_null());
            *self.cmdline_buf = new_buf;
            let new_ptr = (*self.cmdline_buf).as_mut_ptr().cast();
            if !cmdlinep.is_null() {
                *cmdlinep = new_ptr;
            }
            if !self.cmdlinep.is_null() {
                *self.cmdlinep = new_ptr;
            }
        }
    }
}

impl Exarg {
    #[inline]
    pub fn cstack_mut(&self) -> &'static mut crate::ex_eval::Cstack {
        unsafe { &mut *self.cstack.cast::<crate::ex_eval::Cstack>() }
    }

    #[inline]
    pub fn cmdline(&self) -> crate::strings::RawCCharPtr {
        if self.cmdlinep.is_null() {
            ptr::null()
        } else {
            unsafe { (*self.cmdlinep).cast() }
        }
    }
}

impl Default for Exarg {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct Cmdmod {
    pub cmod_flags: i32,
    pub cmod_split: i32,
    pub cmod_tab: i32,
    pub cmod_filter_pat: Option<std::ffi::CString>,
    pub cmod_filter_regmatch: Regmatch,
    pub cmod_filter_force: bool,
    pub cmod_verbose: i32,
    pub cmod_save_ei: Option<std::ffi::CString>,
    pub cmod_did_sandbox: i32,
    pub cmod_verbose_save: i64,
    pub cmod_save_msg_silent: i32,
    pub cmod_save_msg_scroll: i32,
    pub cmod_did_esilent: i32,
}

impl Cmdmod {
    pub const fn new() -> Self {
        Self {
            cmod_flags: 0,
            cmod_split: 0,
            cmod_tab: 0,
            cmod_filter_pat: None,
            cmod_filter_regmatch: Regmatch::new(),
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

impl Default for Cmdmod {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
use crate::api::vim::K_CD_SCOPE_GLOBAL;
use crate::charset::NUL;
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::extmark::{K_FALSE, K_NONE, K_TRUE};
use crate::keycodes::{K_SPECIAL, KS_SPECIAL};
use crate::mapping::{REMAP_NONE, REMAP_YES};
use crate::os::fs::MAXPATHL;
use crate::search::{BACKWARD, FORWARD};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CommandDefinition {
    pub cmd_name: &'static CStr,
    pub cmd_func: ExFunc,
    pub cmd_preview_func: ExPreviewFunc,
    pub cmd_argt: u32,
    pub cmd_addr_type: CmdAddr,
}

pub(crate) const UNDO_HASH_SIZE: usize = 32;

pub(crate) const DOCMD_EXCRESET: i32 = 0x10;
pub(crate) const DOCMD_KEEPLINE: i32 = 0x20;

const EX_WHOLEFOLD: u32 = 0x040;
const EX_CTRLV: u32 = 0x2000;
pub(crate) const EX_BUFNAME: u32 = 0x8000;
const EX_BUFUNL: u32 = 0x10000;
const EX_BUFLOCK_OK: u32 = 0x80000;
const EX_MODIFY: u32 = 0x100000;
const EX_FLAGS: u32 = 0x200000;
const EX_LOCK_OK: u32 = 0x1000000;

const ADDR_TABS_RELATIVE: CmdAddr = 6;
const ADDR_QUICKFIX_VALID: CmdAddr = 7;
const ADDR_UNSIGNED: CmdAddr = 9;

const CTRL_B: u8 = 2;
pub(crate) const CTRL_C_CONST: u8 = 3;
const CTRL_D: u8 = 4;
const CTRL_F: u8 = 6;
pub(crate) const CTRL_G: u8 = 7;
const CTRL_H: u8 = 8;
const CTRL_I: u8 = 9;
const CTRL_J: u8 = 10;
const CTRL_K: u8 = 11;
const CTRL_L: u8 = 12;
pub(crate) const CAR: u8 = 13;
const CTRL_N_CONST: u8 = 14;
pub(crate) const CTRL_O: u8 = 15;
const CTRL_P_CONST: u8 = 16;
const CTRL_Q: u8 = 17;
const CTRL_R: u8 = 18;
const CTRL_S: u8 = 19;
const CTRL_T: u8 = 20;
const CTRL_W: u8 = 23;
const CTRL_X: u8 = 24;
pub(crate) const CTRL_Z_CONST: u8 = 26;
const CTRL_RSB: u8 = 29;
const CTRL_HAT: u8 = 30;
const CTRL_CONST: u8 = 31;

pub(crate) const K_OPT_FINDFUNC: i32 = 99;

const K_CPO_BAR: i32 = b'b' as i32;
const K_CPO_ALTREAD: i32 = b'a' as i32;
const K_CPO_NOSYMLINKS: i32 = b'~' as i32;
const K_CPO_EXECBUF: i32 = b'e' as i32;

pub(crate) const FLUSH_TYPEAHEAD: i32 = 1;

pub(crate) const K_CD_CAUSE_MANUAL: CdCause = 0;

pub(crate) const KE_FILLER: u8 = b'X';

pub(crate) const FIND_ANY: i32 = 1;
pub(crate) const FIND_DEFINE: i32 = 2;
pub(crate) const CHECK_PATH: i32 = 3;
pub(crate) const ACTION_SHOW: i32 = 1;
pub(crate) const ACTION_GOTO: i32 = 2;
pub(crate) const ACTION_SPLIT: i32 = 3;
pub(crate) const ACTION_SHOW_ALL: i32 = 4;
pub(crate) const SEARCH_MSG: i32 = 0x0c;

pub(crate) const DT_TAG: i32 = 1;
pub(crate) const DT_POP: i32 = 2;
pub(crate) const DT_NEXT: i32 = 3;
pub(crate) const DT_PREV: i32 = 4;
pub(crate) const DT_FIRST: i32 = 5;
pub(crate) const DT_LAST: i32 = 6;
pub(crate) const DT_SELECT: i32 = 7;
pub(crate) const DT_JUMP: i32 = 9;
pub(crate) const DT_LTAG: i32 = 11;

const FILETYPE_FILE: &CStr = c"filetype.lua filetype.vim";
const FTPLUGIN_FILE: &CStr = c"ftplugin.vim";
const INDENT_FILE: &CStr = c"indent.vim";
const FTOFF_FILE: &CStr = c"ftoff.vim";
const FTPLUGOF_FILE: &CStr = c"ftplugof.vim";
const INDOFF_FILE: &CStr = c"indoff.vim";

const E_AMBIGUOUS_USE_OF_USER_DEFINED_COMMAND: &CStr =
    c"E464: Ambiguous use of user-defined command";
const E_NO_CALL_STACK_TO_SUBSTITUTE_FOR_STACK: &CStr =
    c"E489: No call stack to substitute for \"<stack>\"";
const E_NOT_AN_EDITOR_COMMAND: &CStr = c"E492: Not an editor command";
const E_NO_AUTOCOMMAND_FILE_NAME_TO_SUBSTITUTE_FOR_AFILE: &CStr =
    c"E495: No autocommand file name to substitute for \"<afile>\"";
const E_NO_AUTOCOMMAND_BUFFER_NUMBER_TO_SUBSTITUTE_FOR_ABUF: &CStr =
    c"E496: No autocommand buffer number to substitute for \"<abuf>\"";
const E_NO_AUTOCOMMAND_MATCH_NAME_TO_SUBSTITUTE_FOR_AMATCH: &CStr =
    c"E497: No autocommand match name to substitute for \"<amatch>\"";
const E_NO_SOURCE_FILE_NAME_TO_SUBSTITUTE_FOR_SFILE: &CStr =
    c"E498: No :source file name to substitute for \"<sfile>\"";
const E_NO_LINE_NUMBER_TO_USE_FOR_SLNUM: &CStr = c"E842: No line number to use for \"<slnum>\"";
const E_NO_LINE_NUMBER_TO_USE_FOR_SFLNUM: &CStr = c"E961: No line number to use for \"<sflnum>\"";
const E_NO_SCRIPT_FILE_NAME_TO_SUBSTITUTE_FOR_SCRIPT: &CStr =
    c"E1274: No script file name to substitute for \"<script>\"";

use crate::errors::*;

const COMMAND_COUNT: i32 = 564;
const CMD_SIZE_I32: i32 = Cmdidx::CmdSize as i32;

struct ExDocmdState {
    quitmore: i32,
    cmdline_call_depth: i32,
    dollar_command: [u8; 2],
    prev_dir: Option<std::ffi::CString>,
    filetype_detect: TriState,
    filetype_plugin: TriState,
    filetype_indent: TriState,
    ex_error_buf: [u8; MSG_BUF_LEN],
}

impl ExDocmdState {
    const fn new() -> Self {
        Self {
            quitmore: 0,
            cmdline_call_depth: 0,
            dollar_command: [b'$', 0],
            prev_dir: None,
            filetype_detect: K_NONE,
            filetype_plugin: K_NONE,
            filetype_indent: K_NONE,
            ex_error_buf: [0; MSG_BUF_LEN],
        }
    }
}

static DOCMD_STATE: GlobalState<ExDocmdState> = GlobalState::new(ExDocmdState::new());

#[inline]
fn docmd_st() -> &'static mut ExDocmdState {
    unsafe { &mut *DOCMD_STATE.as_ptr() }
}

struct Wcmd {
    line: std::ffi::CString,
    lnum: Linenr,
}

#[repr(C)]
pub struct LoopCookie {
    lines: Vec<Wcmd>,
    current_line: i32,
    repeating: i32,
    pub lc_getline: LineGetter,
}

impl LoopCookie {
    const fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_line: 0,
            repeating: 0,
            lc_getline: LineGetter::None,
        }
    }
}

impl Default for LoopCookie {
    fn default() -> Self {
        Self::new()
    }
}

struct DbgStuff {
    trylevel: i32,
    force_abort: i32,
    caught_stack: Vec<std::rc::Rc<Except>>,
    vv_exception: Option<std::ffi::CString>,
    vv_throwpoint: Option<std::ffi::CString>,
    did_emsg: i32,
    got_int: i32,
    did_throw: bool,
    need_rethrow: i32,
    check_cstack: i32,
    current_exception: Option<std::rc::Rc<Except>>,
}

impl DbgStuff {
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
fn sourcing_lnum_ptr() -> &'static mut Linenr {
    &mut rt_st().exestack.last_mut().unwrap().es_lnum
}

fn save_dbg_stuff(dsp: &mut DbgStuff) {
    dsp.trylevel = g_st().trylevel;
    g_st().trylevel = 0;
    dsp.force_abort = i32::from(g_st().force_abort);
    g_st().force_abort = false;
    dsp.caught_stack = std::mem::take(&mut g_st().caught_stack);
    dsp.vv_exception = v_exception(None);
    dsp.vv_throwpoint = v_throwpoint(None);

    // Necessary for debugging an inactive ":catch", ":finally", ":endtry".
    dsp.did_emsg = g_st().did_emsg;
    g_st().did_emsg = 0;
    dsp.got_int = i32::from(g_st().got_int);
    g_st().got_int = false;
    dsp.did_throw = g_st().did_throw;
    g_st().did_throw = false;
    dsp.need_rethrow = i32::from(g_st().need_rethrow);
    g_st().need_rethrow = false;
    dsp.check_cstack = i32::from(g_st().check_cstack);
    g_st().check_cstack = false;
    dsp.current_exception = g_st().current_exception.take();
}

fn restore_dbg_stuff(dsp: &mut DbgStuff) {
    g_st().suppress_errthrow = false;
    g_st().trylevel = dsp.trylevel;
    g_st().force_abort = dsp.force_abort != 0;
    g_st().caught_stack = std::mem::take(&mut dsp.caught_stack);
    v_exception(dsp.vv_exception.take());
    v_throwpoint(dsp.vv_throwpoint.take());
    g_st().did_emsg = dsp.did_emsg;
    g_st().got_int = dsp.got_int != 0;
    g_st().did_throw = dsp.did_throw;
    g_st().need_rethrow = dsp.need_rethrow != 0;
    g_st().check_cstack = dsp.check_cstack != 0;
    g_st().current_exception = dsp.current_exception.take();
}

/// Check if ffname differs from fnum.
fn is_other_file(fnum: i32, ffname: impl crate::strings::IntoCCharPtr) -> bool {
    let ffname = ffname.into_c_char_ptr();
    {
        if fnum != 0 {
            if fnum == curbuf_mut().b_fnum {
                return false;
            }
            return true;
        }

        if ffname.is_null() {
            return true;
        }

        if crate::memory::cstr_is_empty(ffname) {
            return false;
        }

        if !curbuf_mut().file_id_valid
            && !curbuf_mut().b_sfname.is_null()
            && !crate::memory::cstr_is_empty(curbuf_mut().b_sfname)
        {
            return !path_equal(ffname, curbuf_mut().b_sfname, K_PATH_CMP_LITERAL);
        }

        otherfile(ffname)
    }
}

/// Print the executed command for when 'verbose' is set.
fn msg_verbose_cmd(lnum: Linenr, cmd: impl crate::strings::IntoCCharPtr) {
    let cmd = cmd.into_c_char_ptr();
    g_st().no_wait_return += 1;
    verbose_enter_scroll();

    if lnum == 0 {
        let _ = smsg!(0, gettext(c"Executing: %s"), cmd);
    } else {
        let _ = smsg!(0, gettext(c"line %d: %s"), lnum, cmd);
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
    if docmd_st().cmdline_call_depth >= 200
        && OptInt::from(docmd_st().cmdline_call_depth) >= opt_vars().p_mfd
    {
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
pub fn do_cmdline(
    cmdline: impl crate::strings::IntoCCharPtr,
    fgetline: LineGetter,
    flags: i32,
) -> bool {
    let cmdline = cmdline.into_c_char_ptr().cast_mut();
    static RECURSIVE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    unsafe {
        let mut next_cmdline: RawCCharPtrMut;
        let mut cmdline_buf: Vec<u8> = Vec::new();
        let mut cmdline_copy: RawCCharPtrMut = ptr::null_mut();
        let mut used_getline = false;
        let mut msg_didout_before_start = false;
        let mut count: i32 = 0;
        let mut did_inc = false;
        let mut did_block = false;
        let mut retval = true;
        let mut cstack = Cstack::default();
        let mut current_line: i32 = 0;
        let mut fname: RawCCharPtrMut = ptr::null_mut();
        let mut breakpoint: *mut Linenr = ptr::null_mut();
        let mut dbg_tick: *mut i32 = ptr::null_mut();
        let mut debug_saved = DbgStuff::new();

        let mut cmd_getline: LineGetter;
        let mut cmd_loop_cookie = LoopCookie::new();

        let saved_msg_list = g_st().msg_list.replace(Vec::new());

        if !do_cmdline_start() {
            let _ = emsg(gettext(E_COMMAND_TOO_RECURSIVE));
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
            fname = func_name(&mut *fcp).as_ptr().cast_mut();
            breakpoint = func_breakpoint(&mut *fcp);
            dbg_tick = func_dbg_tick(&mut *fcp);
        } else if let Some(sp) = source_cookie {
            fname = sourcing_name().map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
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
            g_st().key_typed = false;
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
                    if ui_has(K_UICMDLINE) {
                        ui_ext_cmdline_block_append(
                            0,
                            g_st()
                                .last_cmdline
                                .as_ref()
                                .map_or(ptr::null(), |s| s.as_ptr()),
                        );
                        did_block = true;
                    }
                    g_st().msg_didout = true;
                }

                if fgetline.is_none() {
                    if g_st().key_typed && (flags & DOCMD_REPEAT) == 0 {
                        g_st().need_wait_return = false;
                    }
                    retval = false;
                    break;
                }
                let Some(next_cstr) = fgetline.call(b':' as i32, indent, true) else {
                    if g_st().key_typed && (flags & DOCMD_REPEAT) == 0 {
                        g_st().need_wait_return = false;
                    }
                    retval = false;
                    break;
                };
                let bytes = next_cstr.as_bytes_with_nul();
                cmdline_buf.clear();
                cmdline_buf.extend_from_slice(bytes);
                next_cmdline = cmdline_buf.as_mut_ptr().cast();
                used_getline = true;

                if ui_has(K_UICMDLINE) && count > 0 && fgetline.is_exline() {
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
                cmdline_buf = crate::memory::cstr_as_bytes_with_nul(next_cmdline).to_vec();
                next_cmdline = cmdline_buf.as_mut_ptr().cast();
            }
            cmdline_copy = next_cmdline;

            let mut current_line_before: i32 = 0;
            let using_loop_cookie = cstack.cs_looplevel > 0 || has_loop_cmd(next_cmdline);
            if using_loop_cookie {
                cmd_loop_cookie.current_line = current_line;
                cmd_loop_cookie.lc_getline = fgetline;
                cmd_loop_cookie.repeating =
                    i32::from((current_line as usize) < cmd_loop_cookie.lines.len());
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
                g_st().msg_scroll = i32::from(true);
                g_st().no_wait_return += 1;
                g_st().redrawing_disabled += 1;
                did_inc = true;
            }

            if (opt_vars().p_verbose >= 15 && sourcing_name().is_some())
                || opt_vars().p_verbose >= 16
            {
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

                if fgetline.is_exline()
                    && let Some(new_cmd) = g_st().new_last_cmdline.take()
                {
                    g_st().last_cmdline = Some(new_cmd);
                }
            } else {
                crate::memory::cstr_move(cmdline_copy, next_cmdline);
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
                report_make_pending(
                    i32::from(cstack.cs_pending[cstack.cs_idx as usize])
                        & (CSTP_ERROR | CSTP_INTERRUPT | CSTP_THROW),
                    if g_st().did_throw {
                        PendingVal::Throw(g_st().current_exception.clone())
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

            if g_st().trylevel == 0 && g_st().did_emsg == 0 && !g_st().got_int && !g_st().did_throw
            {
                g_st().force_abort = false;
            }

            do_intthrow(&raw mut cstack);

            let keep_going = !((g_st().got_int
                || (g_st().did_emsg != 0 && g_st().force_abort)
                || g_st().did_throw)
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
                    let _ = emsg(gettext(E_ENDTRY));
                } else if (top_flags & CSF_WHILE) != 0 {
                    let _ = emsg(gettext(E_ENDWHILE));
                } else if (top_flags & CSF_FOR) != 0 {
                    let _ = emsg(gettext(E_ENDFOR));
                } else {
                    let _ = emsg(gettext(E_ENDIF));
                }
            }

            loop {
                let mut idx = cleanup_conditionals(&mut cstack, 0, i32::from(true));
                if idx >= 0 {
                    idx -= 1;
                }
                let mut looplevel = cstack.cs_looplevel;
                rewind_conditionals(&mut cstack, idx, CSF_WHILE | CSF_FOR, &mut looplevel);
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
            if (fgetline.is_source() || fgetline.is_func())
                && g_st().ex_nesting_level < g_st().debug_break_level
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
            g_st().redrawing_disabled -= 1;
            g_st().no_wait_return -= 1;
            g_st().msg_scroll = 0;

            if !retval || (g_st().did_endif && g_st().key_typed && g_st().did_emsg == 0) {
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
    let cur_excp = g_st().current_exception.clone();
    debug_assert!(cur_excp.is_some());
    let excp = cur_excp.as_ref().unwrap();
    let mut p: Option<std::ffi::CString> = None;
    let mut messages: Vec<Msglist> = Vec::new();

    match excp.r#type {
        ET_USER => {
            let _ = vim_snprintf!(
                g_st().iobuff.as_mut_ptr(),
                IOSIZE,
                gettext(c"E605: Exception not caught: %s"),
                excp.value.as_ptr(),
            );
            p = Some(cstr_to_cstring(g_st().iobuff.as_ptr()));
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
            let _ = emsg_multiline(msg.msg.as_ptr().cast_mut(), c"emsg", HLF_E, msg.multiline);
        }
    } else if let Some(ref p) = p {
        let _ = emsg(p.as_ptr());
    }
    estack_pop();
}

/// Obtain a line when inside a ":while" or ":for" loop.
pub fn get_loop_line(
    c: i32,
    cp: &mut LoopCookie,
    indent: i32,
    do_concat: bool,
) -> Option<std::ffi::CString> {
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

    g_st().key_typed = false;
    cp.current_line += 1;
    let wp = &lines[cp.current_line as usize];
    *sourcing_lnum_ptr() = wp.lnum;
    Some(wp.line.clone())
}

/// Store a line in "gap" so that a ":while" loop can execute it again.
fn store_loop_line(lines: &mut Vec<Wcmd>, line: impl crate::strings::IntoCCharPtr) {
    let line = line.into_c_char_ptr();
    {
        lines.push(Wcmd {
            line: crate::memory::cstr_to_cstring(line),
            lnum: *sourcing_lnum_ptr(),
        });
    }
}

fn compute_buffer_local_count(addr_type: CmdAddr, lnum: Linenr, offset: i32) -> i32 {
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

fn current_win_nr(win: *const WinInfoInfo, cmdarg: Option<&Exarg>) -> i32 {
    let nonnr = cmdarg.is_some() && cmdarg.unwrap().cmdidx != Cmdidx::CmdQuit as i32;
    let mut nr = 0;

    for wp in for_all_windows_ptr() {
        let include_this = nonnr || win_has_winnr(&mut *wp, curtab_mut());
        nr += i32::from(include_this);
        if ptr::eq(wp, win) {
            if !include_this {
                return 0;
            }
            break;
        }
    }
    nr
}

fn current_tab_nr(tab: *mut TabPageInfo) -> i32 {
    let mut nr = 0;
    for tp in for_all_tabpages_ptr() {
        nr += 1;
        if std::ptr::eq(tp, tab) {
            break;
        }
    }
    nr
}

fn get_wincmd_addr_type(arg: impl crate::strings::IntoCCharPtr, eap: &mut Exarg) {
    let arg = arg.into_c_char_ptr();
    {
        let ch = crate::memory::cstr_first_byte(arg);
        match ch {
            b'S' | CTRL_S | b's' | CTRL_N_CONST | b'n' | b'j' | CTRL_J | b'k' | CTRL_K | b'T'
            | CTRL_R | b'r' | b'R' | b'K' | b'J' | b'+' | b'-' | CTRL_CONST | b'_' | b'|'
            | b']' | CTRL_RSB | b'g' | CTRL_G | b'v' | b'h' | CTRL_H | b'l' | CTRL_L | b'H'
            | b'L' | b'>' | b'<' | b'}' | b'f' | b'F' | CTRL_F | b'i' | CTRL_I | b'd' | CTRL_D => {
                eap.addr_type = ADDR_OTHER;
            }
            c if c == CTRL_V_CONST as u8 => {
                eap.addr_type = ADDR_OTHER;
            }
            CTRL_HAT | b'^' => {
                eap.addr_type = ADDR_BUFFERS;
            }
            CTRL_Q | b'q' | CTRL_C_CONST | b'c' | CTRL_O | b'o' | CTRL_W | b'w' | b'W' | b'x'
            | CTRL_X => {
                eap.addr_type = ADDR_WINDOWS;
            }
            CTRL_Z_CONST | b'z' | b'P' | b't' | CTRL_T | b'b' | CTRL_B | b'p' | CTRL_P_CONST
            | b'=' | CAR => {
                eap.addr_type = ADDR_NONE;
            }
            _ => {}
        }
    }
}

fn skip_colon_white(
    p: impl crate::strings::IntoCCharPtr,
    skipleadingwhite: bool,
) -> RawCCharPtrMut {
    let mut p = p.into_c_char_ptr();
    {
        if skipleadingwhite {
            p = skipwhite(p);
        }
        while crate::memory::cstr_first_byte(p) == b':' {
            p = skipwhite(p.wrapping_add(1));
        }
        p.cast_mut()
    }
}

pub fn set_cmd_addr_type(eap: &mut Exarg, p: impl crate::strings::IntoCCharPtr) {
    let p = p.into_c_char_ptr();
    {
        if is_user_cmdidx(Cmdidx::from_raw(eap.cmdidx)) {
            return;
        }
        if eap.cmdidx != CMD_SIZE_I32 {
            eap.addr_type = CMDNAMES[eap.cmdidx as usize].cmd_addr_type;
        } else {
            eap.addr_type = ADDR_LINES;
        }
        if eap.cmdidx == Cmdidx::CmdWincmd as i32 && !p.is_null() {
            get_wincmd_addr_type(skipwhite(p), eap);
        }
        if (eap.cmdidx == Cmdidx::CmdCc as i32 || eap.cmdidx == Cmdidx::CmdLl as i32)
            && bt_quickfix(Some(crate::globals::curbuf_mut()))
        {
            eap.addr_type = ADDR_OTHER;
        }
    }
}

pub fn get_cmd_default_range(eap: &mut Exarg) -> Linenr {
    match eap.addr_type {
        ADDR_LINES | ADDR_OTHER => curwin_mut()
            .w_cursor
            .lnum
            .min(curbuf_mut().b_ml.ml_line_count),
        ADDR_WINDOWS => current_win_nr(curwin_ptr(), None),
        ADDR_ARGUMENTS => (curwin_mut().w_arg_idx + 1).min(argcount()),
        ADDR_LOADED_BUFFERS | ADDR_BUFFERS => curbuf_mut().b_fnum,
        ADDR_TABS => current_tab_nr(g_st().curtab),
        ADDR_TABS_RELATIVE | ADDR_UNSIGNED => 1,
        ADDR_QUICKFIX => qf_get_cur_idx(eap) as Linenr,
        ADDR_QUICKFIX_VALID => qf_get_cur_valid_idx(eap),
        _ => 0,
    }
}

pub fn set_cmd_dflall_range(eap: &mut Exarg) {
    unsafe {
        eap.line1 = 1;
        match eap.addr_type {
            ADDR_LINES | ADDR_OTHER => {
                eap.line2 = curbuf_mut().b_ml.ml_line_count;
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
                eap.line2 = qf_get_valid_size(eap) as Linenr;
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

fn parse_register(eap: &mut Exarg) {
    let is_user = eap.cmdidx < 0;
    if (eap.argt & EX_REGSTR) != 0
        && !crate::memory::cstr_is_empty(eap.arg)
        && (!is_user || crate::memory::cstr_first_byte(eap.arg) != b'=')
        && !((eap.argt & EX_COUNT) != 0 && ascii_isdigit(unsafe { *eap.arg }))
        && valid_yank_reg(
            i32::from(crate::memory::cstr_first_byte(eap.arg)),
            !is_user && eap.cmdidx != Cmdidx::CmdPut as i32 && eap.cmdidx != Cmdidx::CmdIput as i32,
        )
    {
        eap.regname = i32::from(crate::memory::cstr_first_byte(eap.arg));
        eap.arg = eap.arg.wrapping_add(1);
        if crate::memory::cstr_first_byte(eap.arg.wrapping_sub(1)) == b'='
            && !crate::memory::cstr_is_empty(eap.arg)
        {
            if eap.skip == 0 {
                set_expr_line(Some(crate::memory::cstr_to_cstring(eap.arg)));
            }
            eap.arg = eap.arg.wrapping_add(crate::memory::cstr_len(eap.arg));
        }
        eap.arg = skipwhite(eap.arg);
    }
}

pub fn set_cmd_count(eap: &mut Exarg, count: Linenr, validate: bool) {
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
        if validate && eap.line2 > curbuf_mut().b_ml.ml_line_count {
            eap.line2 = curbuf_mut().b_ml.ml_line_count;
        }
    }
}

fn parse_count(
    eap: &mut Exarg,
    errormsg: &mut Option<&'static std::ffi::CStr>,
    validate: bool,
) -> bool {
    if (eap.argt & EX_COUNT) != 0 && ascii_isdigit(unsafe { *eap.arg }) {
        let p = skipdigits(eap.arg.wrapping_add(1));
        if (eap.argt & EX_BUFNAME) == 0
            || crate::memory::cstr_is_empty(p)
            || ascii_iswhite(unsafe { *p })
        {
            let n = getdigits_int32(&mut eap.arg, false, i32::MAX);
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
                *errormsg = Some(gettext(E_ZEROCOUNT));
                return false;
            }
            set_cmd_count(eap, n, validate);
        }
    }
    true
}

pub fn is_cmd_ni(cmdidx: Cmdidx) -> bool {
    if is_user_cmdidx(cmdidx) {
        return false;
    }
    let func = CMDNAMES[cmdidx as usize].cmd_func;
    ptr::fn_addr_eq(func, ex_ni as ExFunc) || ptr::fn_addr_eq(func, ex_script_ni as ExFunc)
}

fn find_excmd_after_range(eap: &mut Exarg) -> RawCCharPtrMut {
    let cmd = eap.cmd;
    eap.cmd = skip_range(eap.cmd, None);
    let p = find_ex_command(eap, None);
    eap.cmd = cmd;
    p
}

fn parse_bang<T>(eap: &Exarg, p: *mut *mut T) -> bool {
    unsafe {
        if *(*p).cast::<u8>() == b'!'
            && eap.cmdidx != Cmdidx::CmdSubstitute as i32
            && eap.cmdidx != Cmdidx::CmdSmagic as i32
            && eap.cmdidx != Cmdidx::CmdSnomagic as i32
        {
            *p = (*p).cast::<u8>().wrapping_add(1).cast();
            return true;
        }
        false
    }
}

pub fn cmd_has_expr_args(cmdidx: Cmdidx) -> bool {
    matches!(
        cmdidx,
        Cmdidx::CmdExecute
            | Cmdidx::CmdEcho
            | Cmdidx::CmdEchon
            | Cmdidx::CmdEchomsg
            | Cmdidx::CmdEchoerr
    )
}

pub fn parse_cmdline(
    cmdline: *mut RawCCharPtrMut,
    cmdline_buf: *mut Vec<u8>,
    eap: &mut Exarg,
    cmod: &mut Cmdmod,
    errormsg: &mut Option<&'static std::ffi::CStr>,
    skipmods: bool,
) -> bool {
    unsafe {
        let mut retval = false;
        let save_cursor = curwin_mut().w_cursor;
        save_last_search_pattern();

        *cmod = Cmdmod::new();
        *eap = Exarg::new();
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
            if !result && (errormsg.is_some() || after_modifier == orig_cmd) {
                break 'end;
            }

            let mut p = find_excmd_after_range(eap);
            if p.is_null() {
                *errormsg = Some(gettext(E_AMBIGUOUS_USE_OF_USER_DEFINED_COMMAND));
                break 'end;
            }

            set_cmd_addr_type(eap, p);
            if !parse_cmd_address(eap, errormsg, true) {
                break 'end;
            }

            eap.cmd = skip_colon_white(eap.cmd, true);
            if crate::memory::cstr_first_byte(eap.cmd) == b'"' {
                break 'end;
            }
            if crate::memory::cstr_is_empty(eap.cmd)
                && eap.addr_count == 0
                && after_modifier == *cmdline
            {
                break 'end;
            }

            if crate::memory::cstr_is_empty(eap.cmd) && eap.cmdidx == CMD_SIZE_I32 {
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
                    g_st().iobuff.as_mut_ptr(),
                    gettext(E_NOT_AN_EDITOR_COMMAND),
                    IOSIZE,
                );
                let cmdname = if !after_modifier.is_null() {
                    after_modifier
                } else {
                    *cmdline
                };
                append_command(cmdname);
                *errormsg = Some(crate::memory::cstr_from_ptr(g_st().iobuff.as_ptr()));
                break 'end;
            }

            eap.forceit = i32::from(parse_bang(eap, &raw mut p));

            if eap.cmdidx >= 0 {
                eap.argt = CMDNAMES[eap.cmdidx as usize].cmd_argt;
            }
            if skipmods {
                eap.arg = p.wrapping_add(usize::from(crate::memory::cstr_first_byte(p) == b' '));
            } else {
                eap.arg = if eap.cmdidx == Cmdidx::CmdBang as i32 {
                    p
                } else {
                    skipwhite(p)
                };
            }

            if eap.cmdidx == Cmdidx::CmdRead as i32 && eap.forceit != 0 {
                eap.forceit = 0;
            }

            if !skipmods && (eap.argt & EX_TRLBAR) != 0 {
                separate_nextcmd(&mut *eap);
            } else if !skipmods
                && eap.cmdidx >= 0
                && cmd_has_expr_args(Cmdidx::from_raw(eap.cmdidx))
            {
                let mut arg = eap.arg;
                while !crate::memory::cstr_is_empty(arg)
                    && crate::memory::cstr_first_byte(arg) != b'|'
                    && crate::memory::cstr_first_byte(arg) != b'\n'
                {
                    let start = arg;
                    g_st().emsg_skip += 1;
                    skip_expr(&raw mut arg, None);
                    g_st().emsg_skip -= 1;
                    if arg == start {
                        arg = arg.wrapping_add(1);
                    }
                }
                if crate::memory::cstr_first_byte(arg) == b'|'
                    || crate::memory::cstr_first_byte(arg) == b'\n'
                {
                    eap.nextcmd = check_nextcmd(arg);
                    *arg = NUL;
                }
            }

            if (eap.argt & EX_BANG) == 0 && eap.forceit != 0 {
                *errormsg = Some(gettext(E_NOBANG));
                break 'end;
            }
            if (eap.argt & EX_RANGE) == 0 && eap.addr_count > 0 {
                *errormsg = Some(gettext(E_NORANGE));
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
        curwin_mut().w_cursor = save_cursor;
        restore_last_search_pattern();
        retval
    }
}

fn shift_cmd_args(eap: &mut Exarg) {
    debug_assert!(!eap.args.is_empty());

    let oldarg0 = eap.args.remove(0);
    let oldarglen0 = eap.arglens.remove(0);

    eap.arg = if let Some(&first_arg) = eap.args.first() {
        first_arg
    } else {
        oldarg0.wrapping_add(oldarglen0)
    };
}

fn execute_cmd0(
    retv: &mut i32,
    eap: &mut Exarg,
    errormsg: &mut Option<&'static std::ffi::CStr>,
    preview: bool,
) -> bool {
    if (eap.argt & EX_XFILE) != 0 && !expand_filename(eap, eap.cmdlinep, errormsg) {
        return false;
    }

    if (eap.argt & EX_BUFNAME) != 0
        && !crate::memory::cstr_is_empty(eap.arg)
        && eap.addr_count == 0
        && eap.cmdidx >= 0
    {
        if eap.args.is_empty() {
            let mut p: RawCCharPtrMut;
            if eap.cmdidx == Cmdidx::CmdBdelete as i32
                || eap.cmdidx == Cmdidx::CmdBwipeout as i32
                || eap.cmdidx == Cmdidx::CmdBunload as i32
            {
                p = skiptowhite_esc(eap.arg);
            } else {
                p = eap.arg.wrapping_add(crate::memory::cstr_len(eap.arg));
                while p > eap.arg && ascii_iswhite(unsafe { *p.wrapping_sub(1) }) {
                    p = p.wrapping_sub(1);
                }
            }
            eap.line2 = buflist_findpat(eap.arg, p, (eap.argt & EX_BUFUNL) != 0, false, false);
            eap.addr_count = 1;
            eap.arg = skipwhite(p);
        } else {
            let arg0 = eap.args.as_slice()[0];
            let arglen0 = eap.arglens.as_slice()[0];
            eap.line2 = buflist_findpat(
                arg0,
                arg0.wrapping_add(arglen0),
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

    if eap.cmdidx == Cmdidx::CmdTry as i32 && g_st().cmdmod.cmod_did_esilent > 0 {
        g_st().emsg_silent -= g_st().cmdmod.cmod_did_esilent;
        if g_st().emsg_silent < 0 {
            g_st().emsg_silent = 0;
        }
        g_st().cmdmod.cmod_did_esilent = 0;
    }

    if eap.cmdidx < 0 {
        *retv = do_ucmd(eap, preview);
    } else {
        eap.errmsg = None;
        if preview {
            if let Some(preview_fn) = CMDNAMES[eap.cmdidx as usize].cmd_preview_func {
                *retv = preview_fn(&mut *eap, cmdpreview_get_ns(), cmdpreview_get_bufnr());
            }
        } else {
            (CMDNAMES[eap.cmdidx as usize].cmd_func)(&mut *eap);
        }
        if eap.errmsg.is_some() {
            *errormsg = eap.errmsg;
        }
    }

    true
}

pub fn execute_cmd(eap: &mut Exarg, cmod: &mut Cmdmod, preview: bool) -> i32 {
    let mut retv: i32 = 0;
    if !do_cmdline_start() {
        let _ = emsg(gettext(E_COMMAND_TOO_RECURSIVE));
        return retv;
    }

    let mut errormsg: Option<&'static std::ffi::CStr> = None;
    let save_cmdmod = unsafe { ptr::replace(&raw mut g_st().cmdmod, std::mem::take(&mut *cmod)) };

    apply_cmdmod(&mut g_st().cmdmod);

    'end: {
        if curbuf_mut().b_p_ma == 0
            && (eap.argt & EX_MODIFY) != 0
            && !(!curbuf_mut().terminal.is_null()
                && (eap.cmdidx == Cmdidx::CmdPut as i32 || eap.cmdidx == Cmdidx::CmdIput as i32))
        {
            errormsg = Some(gettext(E_MODIFIABLE));
            break 'end;
        }
        if eap.cmdidx >= 0 && text_locked() && (eap.argt & EX_LOCK_OK) == 0 {
            errormsg = Some(gettext(get_text_locked_msg()));
            break 'end;
        }
        if (eap.argt & EX_BUFLOCK_OK) == 0
            && eap.cmdidx != Cmdidx::CmdChecktime as i32
            && eap.cmdidx != Cmdidx::CmdEdit as i32
            && !(eap.cmdidx == Cmdidx::CmdFile as i32 && crate::memory::cstr_is_empty(eap.arg))
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
            has_folding(
                curwin_ptr(),
                eap.line1,
                Some(&mut eap.line1),
                ptr::null_mut(),
            );
            has_folding(curwin_ptr(), eap.line2, None, &raw mut eap.line2);
        }

        if !parse_count(&mut *eap, &mut errormsg, true) {
            break 'end;
        }

        let mut cstack = Cstack::default();
        eap.cstack = (&raw mut cstack).cast();

        let _ = execute_cmd0(&mut retv, eap, &mut errormsg, preview);
    }

    if let Some(msg) = errormsg
        && !msg.to_bytes().is_empty()
    {
        let _ = emsg(msg);
    }

    undo_cmdmod(&mut g_st().cmdmod);
    g_st().cmdmod = save_cmdmod;

    do_cmdline_end();
    retv
}

fn profile_cmd(eap: &Exarg, cstack: &mut Cstack, fgetline: LineGetter) {
    if g_st().do_profiling == PROF_YES
        && (eap.skip == 0
            || cstack.cs_idx == 0
            || (cstack.cs_idx > 0
                && (cstack.cs_flags[(cstack.cs_idx - 1) as usize] & CSF_ACTIVE) != 0))
    {
        let mut skip = g_st().did_emsg != 0 || g_st().got_int || g_st().did_throw;

        if eap.cmdidx == Cmdidx::CmdCatch as i32 {
            skip = !skip
                && !(cstack.cs_idx >= 0
                    && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_THROWN) != 0
                    && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_CAUGHT) == 0);
        } else if eap.cmdidx == Cmdidx::CmdElse as i32 || eap.cmdidx == Cmdidx::CmdElseif as i32 {
            skip = skip
                || !(cstack.cs_idx >= 0
                    && (cstack.cs_flags[cstack.cs_idx as usize] & (CSF_ACTIVE | CSF_TRUE)) == 0);
        } else if eap.cmdidx == Cmdidx::CmdFinally as i32 {
            skip = false;
        } else if eap.cmdidx != Cmdidx::CmdEndif as i32
            && eap.cmdidx != Cmdidx::CmdEndfor as i32
            && eap.cmdidx != Cmdidx::CmdEndtry as i32
            && eap.cmdidx != Cmdidx::CmdEndwhile as i32
        {
            skip = eap.skip != 0;
        }

        if !skip {
            if let Some(fcp) = fgetline.as_func() {
                func_line_exec(unsafe { &mut *fcp });
            } else if fgetline.is_source() {
                script_line_exec();
            }
        }
    }
}

fn skip_cmd(eap: &Exarg) -> bool {
    if eap.skip != 0 {
        if eap.cmdidx < 0 || eap.cmdidx >= CMD_SIZE_I32 {
            return true;
        }
        let cmdidx = Cmdidx::from_raw(eap.cmdidx);
        match cmdidx {
            Cmdidx::CmdWhile
            | Cmdidx::CmdEndwhile
            | Cmdidx::CmdFor
            | Cmdidx::CmdEndfor
            | Cmdidx::CmdIf
            | Cmdidx::CmdElseif
            | Cmdidx::CmdElse
            | Cmdidx::CmdEndif
            | Cmdidx::CmdTry
            | Cmdidx::CmdCatch
            | Cmdidx::CmdFinally
            | Cmdidx::CmdEndtry
            | Cmdidx::CmdFunction
            | Cmdidx::CmdAppend
            | Cmdidx::CmdChange
            | Cmdidx::CmdInsert
            | Cmdidx::CmdLoadkeymap
            | Cmdidx::CmdAboveleft
            | Cmdidx::CmdAnd
            | Cmdidx::CmdBelowright
            | Cmdidx::CmdBotright
            | Cmdidx::CmdBrowse
            | Cmdidx::CmdCall
            | Cmdidx::CmdConfirm
            | Cmdidx::CmdConst
            | Cmdidx::CmdDelfunction
            | Cmdidx::CmdDjump
            | Cmdidx::CmdDlist
            | Cmdidx::CmdDsearch
            | Cmdidx::CmdDsplit
            | Cmdidx::CmdEcho
            | Cmdidx::CmdEchoerr
            | Cmdidx::CmdEchomsg
            | Cmdidx::CmdEchon
            | Cmdidx::CmdEval
            | Cmdidx::CmdExecute
            | Cmdidx::CmdFilter
            | Cmdidx::CmdHelp
            | Cmdidx::CmdHide
            | Cmdidx::CmdHorizontal
            | Cmdidx::CmdIjump
            | Cmdidx::CmdIlist
            | Cmdidx::CmdIsearch
            | Cmdidx::CmdIsplit
            | Cmdidx::CmdKeepalt
            | Cmdidx::CmdKeepjumps
            | Cmdidx::CmdKeepmarks
            | Cmdidx::CmdKeeppatterns
            | Cmdidx::CmdLeftabove
            | Cmdidx::CmdLet
            | Cmdidx::CmdLockmarks
            | Cmdidx::CmdLockvar
            | Cmdidx::CmdLua
            | Cmdidx::CmdMatch
            | Cmdidx::CmdMzscheme
            | Cmdidx::CmdNoautocmd
            | Cmdidx::CmdNoswapfile
            | Cmdidx::CmdPerl
            | Cmdidx::CmdPsearch
            | Cmdidx::CmdPython
            | Cmdidx::CmdPy3
            | Cmdidx::CmdPython3
            | Cmdidx::CmdPythonx
            | Cmdidx::CmdPyx
            | Cmdidx::CmdReturn
            | Cmdidx::CmdRightbelow
            | Cmdidx::CmdRuby
            | Cmdidx::CmdSilent
            | Cmdidx::CmdSmagic
            | Cmdidx::CmdSnomagic
            | Cmdidx::CmdSubstitute
            | Cmdidx::CmdSyntax
            | Cmdidx::CmdTab
            | Cmdidx::CmdTcl
            | Cmdidx::CmdThrow
            | Cmdidx::CmdTilde
            | Cmdidx::CmdTopleft
            | Cmdidx::CmdUnlet
            | Cmdidx::CmdUnlockvar
            | Cmdidx::CmdVerbose
            | Cmdidx::CmdVertical
            | Cmdidx::CmdWincmd => {}
            _ => return true,
        }
    }
    false
}

fn do_one_cmd(
    cmdlinep: *mut RawCCharPtrMut,
    cmdline_buf: *mut Vec<u8>,
    flags: i32,
    cstack: &mut Cstack,
    fgetline: LineGetter,
) -> RawCCharPtrMut {
    let mut errormsg: Option<&'static std::ffi::CStr> = None;
    let save_reg_executing = g_st().reg_executing;
    let save_pending_end_reg_executing = g_st().pending_end_reg_executing;

    let mut ea: Exarg = Exarg::new();
    ea.line1 = 1;
    ea.line2 = 1;
    g_st().ex_nesting_level += 1;

    if docmd_st().quitmore != 0 && !fgetline.is_func() && !fgetline.is_autocmd() {
        docmd_st().quitmore -= 1;
    }

    let save_cmdmod = unsafe { ptr::replace(&raw mut g_st().cmdmod, Cmdmod::new()) };

    'doend: {
        if crate::memory::cstr_first_byte(unsafe { *cmdlinep }) == b'#'
            && crate::memory::cstr_byte_at(unsafe { *cmdlinep }, 1) == b'!'
        {
            break 'doend;
        }

        ea.cmd = unsafe { *cmdlinep };
        ea.cmdlinep = cmdlinep;
        ea.cmdline_buf = cmdline_buf;
        ea.ea_getline = fgetline;
        ea.cstack = &raw mut *cstack;

        if !parse_command_modifiers(&mut ea, &mut errormsg, &mut g_st().cmdmod, false) {
            break 'doend;
        }
        apply_cmdmod(&mut g_st().cmdmod);

        let after_modifier = ea.cmd;

        ea.skip = i32::from(
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

        if crate::memory::cstr_is_empty(ea.cmd)
            || crate::memory::cstr_first_byte(ea.cmd) == b'"'
            || {
                ea.nextcmd = check_nextcmd(ea.cmd);
                !ea.nextcmd.is_null()
            }
        {
            if ea.skip != 0 {
                break 'doend;
            }
            debug_assert!(errormsg.is_none());
            errormsg = ex_range_without_command(&mut ea);
            break 'doend;
        }

        if !p.is_null()
            && ea.cmdidx == CMD_SIZE_I32
            && ea.skip == 0
            && ascii_isupper(unsafe { *ea.cmd })
            && has_event(EVENT_CMDUNDEFINED)
        {
            let mut cmdend = ea.cmd;
            while ascii_isalnum(unsafe { *cmdend }) {
                cmdend = cmdend.wrapping_add(1);
            }
            let cmdname = cstr_from_bytes(crate::memory::cstr_slice(
                ea.cmd,
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
                errormsg = Some(gettext(E_AMBIGUOUS_USE_OF_USER_DEFINED_COMMAND));
            }
            break 'doend;
        }

        if ea.cmdidx == CMD_SIZE_I32 {
            if ea.skip == 0 {
                xstrlcpy(
                    g_st().iobuff.as_mut_ptr(),
                    gettext(E_NOT_AN_EDITOR_COMMAND),
                    IOSIZE,
                );
                let cmdname = if !after_modifier.is_null() {
                    after_modifier
                } else {
                    unsafe { *cmdlinep }
                };
                if (flags & DOCMD_VERBOSE) == 0 {
                    append_command(cmdname);
                }
                errormsg = Some(crate::memory::cstr_from_ptr(g_st().iobuff.as_ptr()));
                g_st().did_emsg_syntax = true;
                verify_command(cmdname);
            }
            break 'doend;
        }

        let ni = is_cmd_ni(Cmdidx::from_raw(ea.cmdidx));

        ea.forceit = i32::from(parse_bang(&ea, &raw mut p));

        if ea.cmdidx >= 0 {
            ea.argt = CMDNAMES[ea.cmdidx as usize].cmd_argt;
        }

        if ea.skip == 0 {
            if g_st().sandbox != 0 && (ea.argt & EX_SBOXOK) == 0 {
                errormsg = Some(gettext(E_SANDBOX));
                break 'doend;
            }
            if curbuf_mut().b_p_ma == 0
                && (ea.argt & EX_MODIFY) != 0
                && !(!curbuf_mut().terminal.is_null()
                    && (ea.cmdidx == Cmdidx::CmdPut as i32 || ea.cmdidx == Cmdidx::CmdIput as i32))
            {
                errormsg = Some(gettext(E_MODIFIABLE));
                break 'doend;
            }

            if ea.cmdidx >= 0 && text_locked() && (ea.argt & EX_LOCK_OK) == 0 {
                errormsg = Some(gettext(get_text_locked_msg()));
                break 'doend;
            }

            if (ea.argt & EX_BUFLOCK_OK) == 0
                && ea.cmdidx != Cmdidx::CmdChecktime as i32
                && ea.cmdidx != Cmdidx::CmdEdit as i32
                && ea.cmdidx != Cmdidx::CmdFile as i32
                && ea.cmdidx >= 0
                && curbuf_locked()
            {
                break 'doend;
            }

            if !ni && (ea.argt & EX_RANGE) == 0 && ea.addr_count > 0 {
                errormsg = Some(gettext(E_NORANGE));
                break 'doend;
            }
        }

        if !ni && (ea.argt & EX_BANG) == 0 && ea.forceit != 0 {
            errormsg = Some(gettext(E_NOBANG));
            break 'doend;
        }

        if ea.skip == 0 && !ni && (ea.argt & EX_RANGE) != 0 {
            if g_st().global_busy == 0 && ea.line1 > ea.line2 {
                if g_st().msg_silent == 0 {
                    if (flags & DOCMD_VERBOSE) != 0 || g_st().silent_mode {
                        errormsg = Some(gettext(c"E493: Backwards range given"));
                        break 'doend;
                    }
                    if ask_yesno(gettext(c"Backwards range given, OK to swap")) != b'y' as i32 {
                        break 'doend;
                    }
                }
                std::mem::swap(&mut ea.line1, &mut ea.line2);
            }
            errormsg = invalid_range(&mut ea);
            if errormsg.is_some() {
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
            has_folding(curwin_ptr(), ea.line1, Some(&mut ea.line1), ptr::null_mut());
            has_folding(curwin_ptr(), ea.line2, None, &raw mut ea.line2);
        }

        p = replace_makeprg(&mut ea, p, cmdlinep);
        if p.is_null() {
            break 'doend;
        }

        ea.arg = if ea.cmdidx == Cmdidx::CmdBang as i32 {
            p
        } else {
            skipwhite(p)
        };

        if ea.cmdidx == Cmdidx::CmdFile as i32
            && !crate::memory::cstr_is_empty(ea.arg)
            && curbuf_locked()
        {
            break 'doend;
        }

        if (ea.argt & EX_ARGOPT) != 0 {
            while crate::memory::cstr_first_byte(ea.arg) == b'+'
                && crate::memory::cstr_byte_at(ea.arg, 1) == b'+'
            {
                if !getargopt(&mut ea) && !ni {
                    errormsg = Some(gettext(E_INVARG));
                    break 'doend;
                }
            }
        }

        if ea.cmdidx == Cmdidx::CmdWrite as i32 || ea.cmdidx == Cmdidx::CmdUpdate as i32 {
            if crate::memory::cstr_first_byte(ea.arg) == b'>' {
                ea.arg = ea.arg.wrapping_add(1);
                if crate::memory::cstr_first_byte(ea.arg) != b'>' {
                    errormsg = Some(gettext(c"E494: Use w or w>>"));
                    break 'doend;
                }
                ea.arg = skipwhite(ea.arg.wrapping_add(1));
                ea.append = i32::from(true);
            } else if crate::memory::cstr_first_byte(ea.arg) == b'!'
                && ea.cmdidx == Cmdidx::CmdWrite as i32
            {
                ea.arg = ea.arg.wrapping_add(1);
                ea.usefilter = i32::from(true);
            }
        } else if ea.cmdidx == Cmdidx::CmdRead as i32 {
            if ea.forceit != 0 {
                ea.usefilter = i32::from(true);
                ea.forceit = 0;
            } else if crate::memory::cstr_first_byte(ea.arg) == b'!' {
                ea.arg = ea.arg.wrapping_add(1);
                ea.usefilter = i32::from(true);
            }
        } else if ea.cmdidx == Cmdidx::CmdLshift as i32 || ea.cmdidx == Cmdidx::CmdRshift as i32 {
            ea.amount = 1;
            while unsafe { *ea.arg } == unsafe { *ea.cmd } {
                ea.arg = ea.arg.wrapping_add(1);
                ea.amount += 1;
            }
            ea.arg = skipwhite(ea.arg);
        }

        if (ea.argt & EX_CMDARG) != 0 && ea.usefilter == 0 {
            ea.do_ecmd_cmd = getargcmd(&raw mut ea.arg);
        }

        if (ea.argt & EX_TRLBAR) != 0 && ea.usefilter == 0 {
            separate_nextcmd(&mut ea);
        } else if ea.cmdidx == Cmdidx::CmdBang as i32
            || ea.cmdidx == Cmdidx::CmdTerminal as i32
            || ea.cmdidx == Cmdidx::CmdGlobal as i32
            || ea.cmdidx == Cmdidx::CmdVglobal as i32
            || ea.usefilter != 0
        {
            let mut s = ea.arg;
            while !crate::memory::cstr_is_empty(s) {
                if crate::memory::cstr_first_byte(s) == b'\\'
                    && crate::memory::cstr_byte_at(s, 1) == b'\n'
                {
                    crate::memory::cstr_move(s, s.wrapping_add(1));
                } else if crate::memory::cstr_first_byte(s) == b'\n' {
                    ea.nextcmd = s.wrapping_add(1);
                    unsafe {
                        *s = NUL;
                    }
                    break;
                }
                s = s.wrapping_add(1);
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
            && !crate::memory::cstr_is_empty(ea.arg)
            && crate::memory::cstr_first_byte(ea.arg) != b'"'
            && (crate::memory::cstr_first_byte(ea.arg) != b'|' || (ea.argt & EX_TRLBAR) == 0)
        {
            errormsg = Some(ex_errmsg(E_TRAILING_ARG, ea.arg));
            break 'doend;
        }

        if !ni && (ea.argt & EX_NEEDARG) != 0 && crate::memory::cstr_is_empty(ea.arg) {
            errormsg = Some(gettext(E_ARGREQ));
            break 'doend;
        }

        if skip_cmd(&ea) {
            break 'doend;
        }

        let mut retv: i32 = 0;
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

    if curwin_mut().w_cursor.lnum == 0 {
        curwin_mut().w_cursor.lnum = 1;
        curwin_mut().w_cursor.col = 0;
    }

    if let Some(mut msg) = errormsg
        && !msg.to_bytes().is_empty()
        && g_st().did_emsg == 0
    {
        if (flags & DOCMD_VERBOSE) != 0 {
            let iobuf_ptr: RawCCharPtr = g_st().iobuff.as_ptr();
            if !ptr::eq(msg.as_ptr(), iobuf_ptr) {
                xstrlcpy(g_st().iobuff.as_mut_ptr(), msg, IOSIZE);
                msg = crate::memory::cstr_from_ptr(iobuf_ptr);
            }
            append_command(unsafe { *ea.cmdlinep });
        }
        let _ = emsg(msg);
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

    if !ea.nextcmd.is_null() && crate::memory::cstr_is_empty(ea.nextcmd) {
        ea.nextcmd = ptr::null_mut();
    }

    g_st().ex_nesting_level -= 1;

    ea.nextcmd
}

fn ex_range_without_command(eap: &mut Exarg) -> Option<&'static std::ffi::CStr> {
    let mut errormsg: Option<&'static std::ffi::CStr> = None;

    if crate::memory::cstr_first_byte(eap.cmd) == b'|' {
        eap.cmdidx = Cmdidx::CmdPrint as i32;
        eap.argt = EX_RANGE | EX_COUNT | EX_TRLBAR;
        errormsg = invalid_range(eap);
        if errormsg.is_none() {
            correct_range(eap);
            ex_print(&mut *eap);
        }
    } else if eap.addr_count != 0 {
        eap.line2 = eap.line2.min(curbuf_mut().b_ml.ml_line_count);

        if eap.line2 < 0 {
            errormsg = Some(gettext(E_INVRANGE));
        } else {
            if eap.line2 == 0 {
                curwin_mut().w_cursor.lnum = 1;
            } else {
                curwin_mut().w_cursor.lnum = eap.line2;
            }
            beginline(BL_SOL | BL_FIX);
        }
    }
    errormsg
}

pub fn parse_command_modifiers(
    eap: &mut Exarg,
    errormsg: &mut Option<&'static std::ffi::CStr>,
    cmod: &mut Cmdmod,
    skip_only: bool,
) -> bool {
    let orig_cmd = eap.cmd;
    let mut cmd_start: RawCCharPtrMut = ptr::null_mut();
    let mut has_visual_range = false;
    *cmod = Cmdmod::new();

    if crate::memory::cstr_prefix_bytes(eap.cmd, 5) == &b"'<,'>"[..5] {
        let p = skipwhite(eap.cmd.wrapping_add(5));
        if !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'|' {
            eap.cmd = eap.cmd.wrapping_add(5);
            cmd_start = eap.cmd;
            has_visual_range = true;
        }
    }

    loop {
        while crate::memory::cstr_first_byte(eap.cmd) == b' '
            || crate::memory::cstr_first_byte(eap.cmd) == b'\t'
            || crate::memory::cstr_first_byte(eap.cmd) == b':'
        {
            eap.cmd = eap.cmd.wrapping_add(1);
        }

        if crate::memory::cstr_first_byte(eap.cmd) == b'"' {
            eap.nextcmd = vim_strchr(eap.cmd, b'\n' as i32);
            if !eap.nextcmd.is_null() {
                eap.nextcmd = eap.nextcmd.wrapping_add(1);
            }
            return false;
        }
        if crate::memory::cstr_first_byte(eap.cmd) == b'\n' {
            eap.nextcmd = eap.cmd.wrapping_add(1);
            return false;
        }
        if crate::memory::cstr_is_empty(eap.cmd) {
            return false;
        }

        let mut p = skip_range(eap.cmd, None);
        match crate::memory::cstr_first_byte(p) {
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
                let mut reg_pat: RawCCharPtrMut = ptr::null_mut();
                if !checkforcmd(&raw mut p, c"filter", 4)
                    || crate::memory::cstr_is_empty(p)
                    || ends_excmd(i32::from(unsafe { *p })) != 0
                {
                    break;
                }
                if crate::memory::cstr_first_byte(p) == b'!' {
                    cmod.cmod_filter_force = true;
                    p = skipwhite(p.wrapping_add(1));
                    if crate::memory::cstr_is_empty(p) || ends_excmd(i32::from(unsafe { *p })) != 0
                    {
                        break;
                    }
                }
                if skip_only {
                    p = skip_vimgrep_pat(p, None, None);
                } else {
                    p = skip_vimgrep_pat(p, Some(&mut reg_pat), None);
                }
                if crate::memory::cstr_is_empty(p) {
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
                    || crate::memory::cstr_is_empty(p)
                    || ends_excmd(i32::from(unsafe { *p })) != 0
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
                if crate::memory::cstr_first_byte(eap.cmd) == b'!'
                    && !ascii_iswhite(unsafe { *eap.cmd.wrapping_sub(1) })
                {
                    eap.cmd = skipwhite(eap.cmd.wrapping_add(1));
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
                                *errormsg = Some(gettext(E_INVRANGE));
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
                if ascii_isdigit(unsafe { *eap.cmd }) {
                    cmod.cmod_verbose =
                        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.cmd))
                            as i32
                            + 1;
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
            unsafe {
                ptr::copy(
                    cmd_start,
                    cmd_start.wrapping_sub(5),
                    (eap.cmd as usize) - (cmd_start as usize),
                )
            };
            eap.cmd = eap.cmd.wrapping_sub(5);
            unsafe { ptr::copy_nonoverlapping(c":'<,'>".as_ptr(), eap.cmd.wrapping_sub(1), 6) };
        } else {
            eap.cmd = orig_cmd;
        }
    }

    true
}

pub fn apply_cmdmod(cmod: &mut Cmdmod) {
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

    if (cmod.cmod_flags & (CMOD_SILENT | CMOD_UNSILENT)) != 0 && cmod.cmod_save_msg_silent == 0 {
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
        set_option_direct(K_OPT_EVENTIGNORE, cstr_as_obj(c"all"), 0, SID_NONE);
    }
}

pub fn undo_cmdmod(cmod: &mut Cmdmod) {
    if cmod.cmod_verbose_save > 0 {
        opt_vars().p_verbose = cmod.cmod_verbose_save - 1;
        cmod.cmod_verbose_save = 0;
    }

    if cmod.cmod_did_sandbox != 0 {
        g_st().sandbox -= 1;
        cmod.cmod_did_sandbox = 0;
    }

    if let Some(save_ei) = cmod.cmod_save_ei.take() {
        set_option_direct(
            K_OPT_EVENTIGNORE,
            cstr_as_obj(save_ei.as_ptr()),
            0,
            SID_NONE,
        );
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
    eap: &mut Exarg,
    errormsg: &mut Option<&'static std::ffi::CStr>,
    silent: bool,
) -> bool {
    unsafe {
        let mut address_count: i32 = 1;
        let mut lnum: Linenr;
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
                    i32::from(eap.addr_count == 0),
                    cur_addr_cnt,
                    errormsg,
                );
                if eap.cmd.is_null() {
                    break 'theend;
                }
                if lnum == MAXLNUM {
                    if crate::memory::cstr_first_byte(eap.cmd) == b'%' {
                        eap.cmd = eap.cmd.wrapping_add(1);
                        match eap.addr_type {
                            ADDR_LINES | ADDR_OTHER => {
                                eap.line1 = 1;
                                eap.line2 = curbuf_mut().b_ml.ml_line_count;
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
                                    *errormsg = Some(gettext(E_INVRANGE));
                                    break 'theend;
                                }
                            }
                            ADDR_TABS_RELATIVE | ADDR_UNSIGNED | ADDR_QUICKFIX => {
                                *errormsg = Some(gettext(E_INVRANGE));
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
                                eap.line2 = qf_get_valid_size(eap) as Linenr;
                                if eap.line2 == 0 {
                                    eap.line2 = 1;
                                }
                            }
                            _ => {}
                        }
                        eap.addr_count += 1;
                    } else if crate::memory::cstr_first_byte(eap.cmd) == b'*' {
                        if eap.addr_type != ADDR_LINES {
                            *errormsg = Some(gettext(E_INVRANGE));
                            break 'theend;
                        }

                        eap.cmd = eap.cmd.wrapping_add(1);
                        if eap.skip == 0 {
                            let fm1 = mark_get_visual(curbuf_mut(), b'<' as i32);
                            if !mark_check(fm1.as_mut(), errormsg) {
                                break 'theend;
                            }
                            eap.line1 = (*fm1).mark.lnum;
                            let fm2 = mark_get_visual(curbuf_mut(), b'>' as i32);
                            if !mark_check(fm2.as_mut(), errormsg) {
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

                if crate::memory::cstr_first_byte(eap.cmd) == b';' {
                    if eap.skip == 0 {
                        curwin_mut().w_cursor.lnum = eap.line2;
                        if eap.line2 > 0 {
                            check_cursor(curwin_mut());
                        } else {
                            check_cursor_col(curwin_mut());
                        }
                        need_check_cursor = true;
                    }
                } else if crate::memory::cstr_first_byte(eap.cmd) != b',' {
                    break;
                }
                eap.cmd = eap.cmd.wrapping_add(1);
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
            check_cursor(curwin_mut());
        }
        ret
    }
}

pub fn checkforcmd<T>(pp: *mut *mut T, cmd: &std::ffi::CStr, len: i32) -> bool {
    unsafe {
        let p = (*pp).cast::<u8>();
        let mut i: usize = 0;
        let cmd_bytes = cmd.to_bytes();
        while i < cmd_bytes.len() {
            if cmd_bytes[i] != *p.wrapping_add(i) {
                break;
            }
            i += 1;
        }
        if (i as i32) >= len && !ascii_isalpha(i32::from(*p.wrapping_add(i))) {
            *pp = skipwhite(p.wrapping_add(i).cast()).cast();
            return true;
        }
        false
    }
}

fn append_command(cmd: impl crate::strings::IntoCCharPtr) {
    let cmd = cmd.into_c_char_ptr();
    let iobuf =
        unsafe { std::slice::from_raw_parts_mut(g_st().iobuff.as_mut_ptr().cast::<u8>(), IOSIZE) };
    let mut d = iobuf.iter().position(|&b| b == 0).unwrap_or(IOSIZE);
    if d > IOSIZE - 100 {
        d = IOSIZE - 100;
        d -= utf_head_off_slice(iobuf, d) as usize;
        iobuf[d..d + 3].copy_from_slice(b"...");
        d += 3;
    }
    if d + 2 < IOSIZE {
        iobuf[d..d + 2].copy_from_slice(b": ");
        d += 2;
    }
    let mut s = crate::memory::cstr_as_bytes(cmd);
    while !s.is_empty() && d + 5 < IOSIZE {
        if s.starts_with(&[0xc2, 0xa0]) {
            s = &s[2..];
            iobuf[d..d + 4].copy_from_slice(b"<a0>");
            d += 4;
        } else {
            let l = utfc_slice2len(s);
            if d + l + 1 >= IOSIZE {
                break;
            }
            iobuf[d..d + l].copy_from_slice(&s[..l]);
            d += l;
            s = &s[l..];
        }
    }
    iobuf[d] = 0;
}

fn one_letter_cmd(p: impl crate::strings::IntoCCharPtr, idx: &mut Cmdidx) -> i32 {
    let p = p.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(p) == b'k'
            && (crate::memory::cstr_byte_at(p, 1) != b'e'
                || (crate::memory::cstr_byte_at(p, 1) == b'e'
                    && crate::memory::cstr_byte_at(p, 2) != b'e'))
        {
            *idx = Cmdidx::CmdK;
            return i32::from(true);
        }
        if crate::memory::cstr_first_byte(p) == b's'
            && ((crate::memory::cstr_byte_at(p, 1) == b'c'
                && (crate::memory::cstr_byte_at(p, 2) == 0
                    || (crate::memory::cstr_byte_at(p, 2) != b's'
                        && crate::memory::cstr_byte_at(p, 2) != b'r'
                        && (crate::memory::cstr_byte_at(p, 3) == 0
                            || (crate::memory::cstr_byte_at(p, 3) != b'i'
                                && crate::memory::cstr_byte_at(p, 4) != b'p')))))
                || crate::memory::cstr_byte_at(p, 1) == b'g'
                || (crate::memory::cstr_byte_at(p, 1) == b'i'
                    && crate::memory::cstr_byte_at(p, 2) != b'm'
                    && crate::memory::cstr_byte_at(p, 2) != b'l'
                    && crate::memory::cstr_byte_at(p, 2) != b'g')
                || crate::memory::cstr_byte_at(p, 1) == b'I'
                || (crate::memory::cstr_byte_at(p, 1) == b'r'
                    && crate::memory::cstr_byte_at(p, 2) != b'e'))
        {
            *idx = Cmdidx::CmdSubstitute;
            return i32::from(true);
        }
        i32::from(false)
    }
}

pub fn find_ex_command(eap: &mut Exarg, mut full: Option<&mut i32>) -> RawCCharPtrMut {
    let mut p = eap.cmd;
    let mut cmdidx_out = Cmdidx::CmdAppend;
    if one_letter_cmd(p, &mut cmdidx_out) != 0 {
        eap.cmdidx = cmdidx_out as i32;
        p = p.wrapping_add(1);
        if let Some(ref mut p) = full {
            **p = i32::from(true);
        }
    } else {
        while ascii_isalpha(unsafe { *p }) {
            p = p.wrapping_add(1);
        }
        if crate::memory::cstr_first_byte(eap.cmd) == b'p'
            && crate::memory::cstr_byte_at(eap.cmd, 1) == b'y'
        {
            while ascii_isalnum(unsafe { *p }) {
                p = p.wrapping_add(1);
            }
        }

        if p == eap.cmd
            && crate::strings::vim_haschr(c"@!=><&~#", i32::from(crate::memory::cstr_first_byte(p)))
        {
            p = p.wrapping_add(1);
        }
        let mut len = ((p as usize) - (eap.cmd as usize)) as i32;

        if crate::memory::cstr_first_byte(eap.cmd) == b'd'
            && (crate::memory::cstr_first_byte(p.wrapping_sub(1)) == b'l'
                || crate::memory::cstr_first_byte(p.wrapping_sub(1)) == b'p')
        {
            let delete_bytes = b"delete\0";
            let mut i: i32 = 0;
            while i < len {
                if (crate::memory::cstr_byte_at(eap.cmd, i as usize)) != delete_bytes[i as usize] {
                    break;
                }
                i += 1;
            }
            if i == len - 1 {
                len -= 1;
                if crate::memory::cstr_first_byte(p.wrapping_sub(1)) == b'l' {
                    eap.flags |= EXFLAG_LIST;
                } else {
                    eap.flags |= EXFLAG_PRINT;
                }
            }
        }

        if ascii_islower(unsafe { *eap.cmd }) {
            let c1 = crate::memory::cstr_first_byte(eap.cmd);
            let c2 = if len == 1 {
                0u8
            } else {
                crate::memory::cstr_byte_at(eap.cmd, 1)
            };

            if COMMAND_COUNT != CMD_SIZE_I32 {
                iemsg(gettext(
                    c"E943: Command table needs to be updated, run 'make'",
                ));
                getout(1);
            }

            let idx1 = (c1 - b'a') as usize;
            eap.cmdidx = i32::from(CMDIDXS1[idx1]);
            if c2.is_ascii_lowercase() {
                eap.cmdidx += i32::from(CMDIDXS2[idx1][(c2 - b'a') as usize]);
            }
        } else if ascii_isupper(unsafe { *eap.cmd }) {
            eap.cmdidx = Cmdidx::CmdCapNext as i32;
        } else {
            eap.cmdidx = Cmdidx::CmdBang as i32;
        }

        if len == 3 && (b"def" == crate::memory::cstr_prefix_bytes(eap.cmd, 3)) {
            eap.cmdidx = CMD_SIZE_I32;
        }

        while eap.cmdidx < CMD_SIZE_I32 {
            let cname = CMDNAMES[eap.cmdidx as usize].cmd_name.as_ptr();
            if crate::memory::cstr_prefix_bytes(cname, len as usize)
                == crate::memory::cstr_prefix_bytes(eap.cmd, len as usize)
            {
                if full.is_some() && crate::memory::cstr_byte_at(cname, len as usize) == 0 {
                    *full.as_deref_mut().unwrap() = i32::from(true);
                }
                break;
            }
            eap.cmdidx += 1;
        }

        if eap.cmdidx == CMD_SIZE_I32
            && (crate::memory::cstr_first_byte(eap.cmd)) >= b'A'
            && (crate::memory::cstr_first_byte(eap.cmd)) <= b'Z'
        {
            while ascii_isalnum(unsafe { *p }) {
                p = p.wrapping_add(1);
            }
            p = find_ucmd(
                eap,
                p,
                (full.is_some()).then(|| &mut *full.unwrap()),
                None,
                None,
            );
        }
        if p == eap.cmd {
            eap.cmdidx = CMD_SIZE_I32;
        }
    }

    if eap.cmdidx == Cmdidx::CmdHorizontal as i32 && ((p as usize) - (eap.cmd as usize)) == 2 {
        eap.cmdidx = CMD_SIZE_I32;
    }

    p
}

pub fn modifier_len(cmd: impl crate::strings::IntoCCharPtr) -> i32 {
    let cmd = cmd.into_c_char_ptr();
    {
        let mut p = cmd;
        if ascii_isdigit(unsafe { *cmd }) {
            p = skipwhite(skipdigits(cmd.wrapping_add(1)));
        }
        for info in &CMDMODS {
            if info.name[0] > (crate::memory::cstr_first_byte(p)) {
                break;
            }
            let mut j = 0usize;
            while crate::memory::cstr_byte_at(p, j) != 0 {
                if j >= info.name.len() || (crate::memory::cstr_byte_at(p, j)) != info.name[j] {
                    break;
                }
                j += 1;
            }
            if j >= info.minlen
                && !ascii_isalpha(crate::memory::cstr_char_at(p, j))
                && (p == cmd || info.has_count)
            {
                return (j + ((p as usize) - (cmd as usize))) as i32;
            }
        }
        0
    }
}

pub fn cmd_exists(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        for info in &CMDMODS {
            let mut j = 0usize;
            while crate::memory::cstr_byte_at(name, j) != 0 {
                if j >= info.name.len() || (crate::memory::cstr_byte_at(name, j)) != info.name[j] {
                    break;
                }
                j += 1;
            }
            if crate::memory::cstr_byte_at(name, j) == 0 && j >= info.minlen {
                return if j == info.name.len() { 2 } else { 1 };
            }
        }

        let mut ea: Exarg = Exarg::new();
        ea.cmd = if crate::memory::cstr_first_byte(name) == b'2'
            || crate::memory::cstr_first_byte(name) == b'3'
        {
            name.wrapping_add(1).cast_mut()
        } else {
            name.cast_mut()
        };
        ea.cmdidx = 0;
        ea.flags = 0;
        let mut full: i32 = 0;
        let p = find_ex_command(&mut ea, Some(&mut full));
        if p.is_null() {
            return 3;
        }
        if ascii_isdigit(unsafe { *name }) && ea.cmdidx != Cmdidx::CmdMatch as i32 {
            return 0;
        }
        if !crate::memory::cstr_is_empty(skipwhite(p)) {
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

pub fn f_fullcommand(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut name = tv_get_string(&argvars[0]).as_ptr().cast_mut();

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    while crate::memory::cstr_first_byte(name) == b':' {
        name = name.wrapping_add(1);
    }
    name = skip_range(name, None);

    let mut ea: Exarg = Exarg::new();
    ea.cmd = if crate::memory::cstr_first_byte(name) == b'2'
        || crate::memory::cstr_first_byte(name) == b'3'
    {
        name.wrapping_add(1)
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
            Some(CMDNAMES[ea.cmdidx as usize].cmd_name)
        },
    );
}

pub fn excmd_get_cmdidx(cmd: impl crate::strings::IntoCCharPtr, len: usize) -> Cmdidx {
    let cmd = cmd.into_c_char_ptr();
    if len == 3 && (b"def" == crate::memory::cstr_prefix_bytes(cmd, 3)) {
        return Cmdidx::CmdSize;
    }

    let mut idx = Cmdidx::CmdAppend;
    if one_letter_cmd(cmd, &mut idx) == 0 {
        let mut i: i32 = 0;
        while i < CMD_SIZE_I32 {
            if crate::memory::cstr_prefix_bytes(CMDNAMES[i as usize].cmd_name.as_ptr(), len)
                == crate::memory::cstr_prefix_bytes(cmd, len)
            {
                break;
            }
            i += 1;
        }
        idx = Cmdidx::from_raw(i);
    }
    idx
}

pub fn excmd_get_argt(idx: Cmdidx) -> u32 {
    CMDNAMES[idx as usize].cmd_argt
}

pub fn skip_range(
    cmd: impl crate::strings::IntoCCharPtr,
    mut ctx: Option<&mut i32>,
) -> RawCCharPtrMut {
    let mut cmd = cmd.into_c_char_ptr();
    {
        while crate::strings::vim_haschr(
            c" \t0123456789.$%'/?-+,;\\",
            i32::from(crate::memory::cstr_first_byte(cmd)),
        ) {
            if crate::memory::cstr_first_byte(cmd) == b'\\' {
                let next = crate::memory::cstr_byte_at(cmd, 1);
                if next == b'?' || next == b'/' || next == b'&' {
                    cmd = cmd.wrapping_add(1);
                } else {
                    break;
                }
            } else if crate::memory::cstr_first_byte(cmd) == b'\'' {
                cmd = cmd.wrapping_add(1);
                if crate::memory::cstr_is_empty(cmd)
                    && let Some(ref mut ctx) = ctx
                {
                    **ctx = EXPAND_NOTHING;
                }
            } else if crate::memory::cstr_first_byte(cmd) == b'/'
                || crate::memory::cstr_first_byte(cmd) == b'?'
            {
                cmd = skip_regexp(
                    cmd.wrapping_add(1).cast_mut(),
                    i32::from(crate::memory::cstr_first_byte(cmd)),
                    opt_vars().p_magic,
                );
                if crate::memory::cstr_is_empty(cmd)
                    && let Some(ref mut ctx) = ctx
                {
                    **ctx = EXPAND_NOTHING;
                }
            }
            if !crate::memory::cstr_is_empty(cmd) {
                cmd = cmd.wrapping_add(1);
            }
        }

        cmd = skip_colon_white(cmd, false);

        if crate::memory::cstr_first_byte(cmd) == b'*' {
            cmd = skipwhite(cmd.wrapping_add(1));
        }

        cmd.cast_mut()
    }
}

fn addr_error(addr_type: CmdAddr) -> &'static std::ffi::CStr {
    if addr_type == ADDR_NONE {
        gettext(E_NORANGE)
    } else {
        gettext(E_INVRANGE)
    }
}

pub fn get_address(
    eap: *mut Exarg,
    ptr: *mut RawCCharPtrMut,
    addr_type: CmdAddr,
    skip: bool,
    silent: bool,
    to_other_file: i32,
    address_count: i32,
    errormsg: &mut Option<&'static std::ffi::CStr>,
) -> Linenr {
    unsafe {
        let mut cmd = skipwhite(*ptr);
        let mut lnum: Linenr = MAXLNUM;

        'error: {
            loop {
                match crate::memory::cstr_first_byte(cmd) {
                    b'.' => {
                        cmd = cmd.wrapping_add(1);
                        match addr_type {
                            ADDR_LINES | ADDR_OTHER => lnum = curwin_mut().w_cursor.lnum,
                            ADDR_WINDOWS => lnum = current_win_nr(curwin_ptr(), None),
                            ADDR_ARGUMENTS => lnum = curwin_mut().w_arg_idx + 1,
                            ADDR_LOADED_BUFFERS | ADDR_BUFFERS => lnum = curbuf_mut().b_fnum,
                            ADDR_TABS => lnum = current_tab_nr(g_st().curtab),
                            ADDR_NONE | ADDR_TABS_RELATIVE | ADDR_UNSIGNED => {
                                *errormsg = Some(addr_error(addr_type));
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            ADDR_QUICKFIX => lnum = qf_get_cur_idx(&mut *eap) as Linenr,
                            ADDR_QUICKFIX_VALID => lnum = qf_get_cur_valid_idx(&mut *eap),
                            _ => {}
                        }
                    }
                    b'$' => {
                        cmd = cmd.wrapping_add(1);
                        match addr_type {
                            ADDR_LINES | ADDR_OTHER => lnum = curbuf_mut().b_ml.ml_line_count,
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
                                *errormsg = Some(addr_error(addr_type));
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            ADDR_QUICKFIX => {
                                lnum = qf_get_size(&mut *eap) as Linenr;
                                if lnum == 0 {
                                    lnum = 1;
                                }
                            }
                            ADDR_QUICKFIX_VALID => {
                                lnum = qf_get_valid_size(&mut *eap) as Linenr;
                                if lnum == 0 {
                                    lnum = 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    b'\'' => {
                        cmd = cmd.wrapping_add(1);
                        if crate::memory::cstr_is_empty(cmd) {
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        if addr_type != ADDR_LINES {
                            *errormsg = Some(addr_error(addr_type));
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        if skip {
                            cmd = cmd.wrapping_add(1);
                        } else {
                            let flag =
                                if to_other_file != 0 && crate::memory::cstr_byte_at(cmd, 1) == 0 {
                                    K_MARK_ALL
                                } else {
                                    K_MARK_BUF_LOCAL
                                };
                            let fm = mark_get(
                                curbuf_mut(),
                                curwin_mut(),
                                None,
                                flag,
                                i32::from(crate::memory::cstr_first_byte(cmd)),
                            );
                            cmd = cmd.wrapping_add(1);
                            if !fm.is_null() && (*fm).fnum != curbuf_mut().b_fnum {
                                mark_move_to(fm, 0);
                                lnum = curwin_mut().w_cursor.lnum;
                            } else {
                                if !mark_check(fm.as_mut(), errormsg) {
                                    cmd = ptr::null_mut();
                                    break 'error;
                                }
                                lnum = (*fm).mark.lnum;
                            }
                        }
                    }
                    b'/' | b'?' => {
                        let c = i32::from(crate::memory::cstr_first_byte(cmd));
                        cmd = cmd.wrapping_add(1);
                        if addr_type != ADDR_LINES {
                            *errormsg = Some(addr_error(addr_type));
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        if skip {
                            cmd = skip_regexp(cmd, c, opt_vars().p_magic);
                            if i32::from(crate::memory::cstr_first_byte(cmd)) == c {
                                cmd = cmd.wrapping_add(1);
                            }
                        } else {
                            let pos = curwin_mut().w_cursor;
                            if lnum > 0 && lnum != MAXLNUM {
                                curwin_mut().w_cursor.lnum =
                                    lnum.min(curbuf_mut().b_ml.ml_line_count);
                            }
                            curwin_mut().w_cursor.col =
                                if c == b'/' as i32 && curwin_mut().w_cursor.lnum > 0 {
                                    MAXCOL
                                } else {
                                    0
                                };
                            g_st().search_global.cmdlen = 0;
                            let search_flags = if silent {
                                SEARCH_KEEP
                            } else {
                                SEARCH_HIS | SEARCH_MSG
                            };
                            if do_search(
                                None,
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
                                curwin_mut().w_cursor = pos;
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            lnum = curwin_mut().w_cursor.lnum;
                            curwin_mut().w_cursor = pos;
                            cmd = cmd.wrapping_add(g_st().search_global.cmdlen as usize);
                        }
                    }
                    b'\\' => {
                        cmd = cmd.wrapping_add(1);
                        if addr_type != ADDR_LINES {
                            *errormsg = Some(addr_error(addr_type));
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        let pat_use = if crate::memory::cstr_first_byte(cmd) == b'&' {
                            RE_SUBST
                        } else if crate::memory::cstr_first_byte(cmd) == b'?'
                            || crate::memory::cstr_first_byte(cmd) == b'/'
                        {
                            RE_SEARCH
                        } else {
                            *errormsg = Some(gettext(E_BACKSLASH));
                            cmd = ptr::null_mut();
                            break 'error;
                        };
                        if !skip {
                            let mut pos = CursorPos {
                                lnum: if lnum != MAXLNUM {
                                    lnum
                                } else {
                                    curwin_mut().w_cursor.lnum
                                },
                                col: if crate::memory::cstr_first_byte(cmd) != b'?' {
                                    MAXCOL
                                } else {
                                    0
                                },
                                coladd: 0,
                            };
                            if searchit(
                                curwin_ptr(),
                                curbuf_mut(),
                                &mut pos,
                                None,
                                if crate::memory::cstr_first_byte(cmd) == b'?' {
                                    BACKWARD
                                } else {
                                    FORWARD
                                },
                                c"",
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
                        cmd = cmd.wrapping_add(1);
                    }
                    _ => {
                        if ascii_isdigit(*cmd) {
                            lnum = getdigits(&mut cmd, false, 0) as Linenr;
                        }
                    }
                }

                loop {
                    cmd = skipwhite(cmd);
                    if crate::memory::cstr_first_byte(cmd) != b'-'
                        && crate::memory::cstr_first_byte(cmd) != b'+'
                        && !ascii_isdigit(*cmd)
                    {
                        break;
                    }

                    if lnum == MAXLNUM {
                        match addr_type {
                            ADDR_LINES | ADDR_OTHER => lnum = curwin_mut().w_cursor.lnum,
                            ADDR_WINDOWS => lnum = current_win_nr(curwin_ptr(), None),
                            ADDR_ARGUMENTS => lnum = curwin_mut().w_arg_idx + 1,
                            ADDR_LOADED_BUFFERS | ADDR_BUFFERS => lnum = curbuf_mut().b_fnum,
                            ADDR_TABS => lnum = current_tab_nr(g_st().curtab),
                            ADDR_TABS_RELATIVE => lnum = 1,
                            ADDR_QUICKFIX => lnum = qf_get_cur_idx(&mut *eap) as Linenr,
                            ADDR_QUICKFIX_VALID => lnum = qf_get_cur_valid_idx(&mut *eap),
                            _ => lnum = 0,
                        }
                    }

                    let op = if ascii_isdigit(*cmd) {
                        b'+' as i32
                    } else {
                        let ch = i32::from(crate::memory::cstr_first_byte(cmd));
                        cmd = cmd.wrapping_add(1);
                        ch
                    };
                    let n = if !ascii_isdigit(*cmd) {
                        1
                    } else {
                        let parsed = getdigits_int32(&mut cmd, false, MAXLNUM);
                        if parsed == MAXLNUM {
                            *errormsg = Some(gettext(E_LINE_NUMBER_OUT_OF_RANGE));
                            cmd = ptr::null_mut();
                            break 'error;
                        }
                        parsed
                    };

                    if addr_type == ADDR_TABS_RELATIVE {
                        *errormsg = Some(gettext(E_INVRANGE));
                        cmd = ptr::null_mut();
                        break 'error;
                    } else if addr_type == ADDR_LOADED_BUFFERS || addr_type == ADDR_BUFFERS {
                        lnum = compute_buffer_local_count(
                            addr_type,
                            lnum,
                            if op == b'-' as i32 { -n } else { n },
                        );
                    } else {
                        if addr_type == ADDR_LINES
                            && (op == b'-' as i32 || op == b'+' as i32)
                            && address_count >= 2
                        {
                            has_folding(curwin_ptr(), lnum, None, &raw mut lnum);
                        }
                        if op == b'-' as i32 {
                            lnum -= n;
                        } else {
                            if lnum >= 0 && n >= i32::MAX - lnum {
                                *errormsg = Some(gettext(E_LINE_NUMBER_OUT_OF_RANGE));
                                cmd = ptr::null_mut();
                                break 'error;
                            }
                            lnum += n;
                        }
                    }
                }

                if crate::memory::cstr_first_byte(cmd) != b'/'
                    && crate::memory::cstr_first_byte(cmd) != b'?'
                {
                    break;
                }
            }
        }

        *ptr = cmd;
        lnum
    }
}

fn get_flags(eap: &mut Exarg) {
    while crate::strings::vim_haschr(c"lp#", i32::from(crate::memory::cstr_first_byte(eap.arg))) {
        if crate::memory::cstr_first_byte(eap.arg) == b'l' {
            eap.flags |= EXFLAG_LIST;
        } else if crate::memory::cstr_first_byte(eap.arg) == b'p' {
            eap.flags |= EXFLAG_PRINT;
        } else {
            eap.flags |= EXFLAG_NR;
        }
        eap.arg = skipwhite(eap.arg.wrapping_add(1));
    }
}

pub fn ex_ni(eap: &mut Exarg) {
    if eap.skip == 0 {
        eap.errmsg = Some(gettext(
            c"E319: The command is not available in this version",
        ));
    }
}

fn ex_script_ni(eap: &mut Exarg) {
    if eap.skip == 0 {
        ex_ni(eap);
    } else {
        let mut len: usize = 0;
        let _ = script_get(eap, &mut len);
    }
}

pub fn invalid_range(eap: &mut Exarg) -> Option<&'static std::ffi::CStr> {
    unsafe {
        if eap.line1 < 0 || eap.line2 < 0 || eap.line1 > eap.line2 {
            return Some(gettext(E_INVRANGE));
        }

        if (eap.argt & EX_RANGE) != 0 {
            let addr_type = eap.addr_type;
            match addr_type {
                ADDR_LINES => {
                    let extra = Linenr::from(
                        eap.cmdidx == Cmdidx::CmdDiffget as i32
                            || eap.cmdidx == Cmdidx::CmdDiffput as i32,
                    );
                    if eap.line2 > curbuf_mut().b_ml.ml_line_count + extra {
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_ARGUMENTS => {
                    let ac = argcount();
                    if eap.line2 > ac + i32::from(ac == 0) {
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_BUFFERS => {
                    if eap.line1 < 1 || eap.line2 > get_highest_fnum() {
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_LOADED_BUFFERS => {
                    let mut buf = g_st().firstbuf;
                    while (*buf).b_ml.ml_mfp.is_none() {
                        if (*buf).b_next.is_null() {
                            return Some(gettext(E_INVRANGE));
                        }
                        buf = (*buf).b_next;
                    }
                    if eap.line1 < (*buf).b_fnum {
                        return Some(gettext(E_INVRANGE));
                    }
                    buf = g_st().lastbuf;
                    while (*buf).b_ml.ml_mfp.is_none() {
                        if (*buf).b_prev.is_null() {
                            return Some(gettext(E_INVRANGE));
                        }
                        buf = (*buf).b_prev;
                    }
                    if eap.line2 > (*buf).b_fnum {
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_WINDOWS => {
                    if eap.line2 > current_win_nr(ptr::null(), Some(&*eap)) {
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_TABS => {
                    if eap.line2 > current_tab_nr(ptr::null_mut()) {
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_TABS_RELATIVE | ADDR_OTHER => {}
                ADDR_QUICKFIX => {
                    if eap.line2 <= 0 {
                        if eap.addr_count == 0 {
                            return Some(gettext(E_NO_ERRORS));
                        }
                        return Some(gettext(E_INVRANGE));
                    }
                }
                ADDR_QUICKFIX_VALID
                    if (eap.line2 != 1 && (eap.line2 as usize) > qf_get_valid_size(&mut *eap))
                        || eap.line2 < 0 =>
                {
                    return Some(gettext(E_INVRANGE));
                }
                _ => {}
            }
        }
        None
    }
}

fn correct_range(eap: &mut Exarg) {
    if (eap.argt & EX_ZEROR) == 0 {
        if eap.line1 == 0 {
            eap.line1 = 1;
        }
        if eap.line2 == 0 {
            eap.line2 = 1;
        }
    }
}

fn skip_grep_pat(eap: &mut Exarg) -> RawCCharPtrMut {
    let mut p = eap.arg;
    if !crate::memory::cstr_is_empty(p)
        && (eap.cmdidx == Cmdidx::CmdVimgrep as i32
            || eap.cmdidx == Cmdidx::CmdLvimgrep as i32
            || eap.cmdidx == Cmdidx::CmdVimgrepadd as i32
            || eap.cmdidx == Cmdidx::CmdLvimgrepadd as i32
            || (eap.cmdidx >= 0 && grep_internal(eap.cmdidx) != 0))
    {
        p = skip_vimgrep_pat(p, None, None);
        if p.is_null() {
            p = eap.arg;
        }
    }
    p
}

pub fn replace_makeprg(
    eap: &mut Exarg,
    mut arg: RawCCharPtrMut,
    cmdlinep: *mut RawCCharPtrMut,
) -> RawCCharPtrMut {
    let isgrep = eap.cmdidx == Cmdidx::CmdGrep as i32
        || eap.cmdidx == Cmdidx::CmdLgrep as i32
        || eap.cmdidx == Cmdidx::CmdGrepadd as i32
        || eap.cmdidx == Cmdidx::CmdLgrepadd as i32;

    if (eap.cmdidx == Cmdidx::CmdMake as i32 || eap.cmdidx == Cmdidx::CmdLmake as i32 || isgrep)
        && !(eap.cmdidx >= 0 && grep_internal(eap.cmdidx) != 0)
    {
        let program = if isgrep {
            if crate::memory::cstr_is_empty(curbuf_mut().b_p_gp) {
                opt_vars().p_gp
            } else {
                curbuf_mut().b_p_gp
            }
        } else if crate::memory::cstr_is_empty(curbuf_mut().b_p_mp) {
            opt_vars().p_mp
        } else {
            curbuf_mut().b_p_mp
        };

        arg = skipwhite(arg);

        let new_buf: Vec<u8> = if let Some(rep) = strrep(program, c"$*", arg) {
            rep.into_bytes_with_nul()
        } else {
            let prog_b = crate::memory::cstr_as_bytes(program);
            let arg_b = crate::memory::cstr_as_bytes(arg);
            let mut combined = Vec::with_capacity(prog_b.len() + 1 + arg_b.len() + 1);
            combined.extend_from_slice(prog_b);
            combined.push(b' ');
            combined.extend_from_slice(arg_b);
            combined.push(0);
            combined
        };

        msg_make(arg);

        eap.set_cmdline_vec(cmdlinep, new_buf);
        arg = unsafe { *cmdlinep };
    }
    arg
}

pub fn expand_filename(
    eap: &mut Exarg,
    cmdlinep: *mut RawCCharPtrMut,
    errormsgp: &mut Option<&'static std::ffi::CStr>,
) -> bool {
    let mut p = skip_grep_pat(&mut *eap);
    let mut has_wildcards = path_has_wildcard(p, true);

    while !crate::memory::cstr_is_empty(p) {
        if crate::memory::cstr_first_byte(p) == b'`' && crate::memory::cstr_byte_at(p, 1) == b'=' {
            p = p.wrapping_add(2);
            skip_expr(&raw mut p, None);
            if crate::memory::cstr_first_byte(p) == b'`' {
                p = p.wrapping_add(1);
            }
            continue;
        }
        if !crate::strings::vim_haschr(c"%#<", i32::from(crate::memory::cstr_first_byte(p))) {
            p = p.wrapping_add(1);
            continue;
        }

        let mut srclen: usize = 0;
        let mut escaped: i32 = 0;
        let repl_owned = eval_vars(
            p,
            eap.arg,
            &mut srclen,
            Some(&mut eap.do_ecmd_lnum),
            errormsgp,
            Some(&mut escaped),
            true,
        );
        if errormsgp.is_some() {
            return false;
        }
        let Some(mut repl_cstr) = repl_owned else {
            p = p.wrapping_add(srclen);
            continue;
        };

        if crate::strings::vim_haschr(repl_cstr.as_ptr(), b'$' as i32)
            || crate::strings::vim_haschr(repl_cstr.as_ptr(), b'~' as i32)
        {
            repl_cstr = expand_env_save(repl_cstr.as_ptr().cast_mut());
        }

        if eap.usefilter == 0
            && escaped == 0
            && eap.cmdidx != Cmdidx::CmdBang as i32
            && eap.cmdidx != Cmdidx::CmdGrep as i32
            && eap.cmdidx != Cmdidx::CmdGrepadd as i32
            && eap.cmdidx != Cmdidx::CmdLgrep as i32
            && eap.cmdidx != Cmdidx::CmdLgrepadd as i32
            && eap.cmdidx != Cmdidx::CmdLmake as i32
            && eap.cmdidx != Cmdidx::CmdMake as i32
            && eap.cmdidx != Cmdidx::CmdTerminal as i32
            && (eap.argt & EX_NOSPC) == 0
        {
            #[cfg(windows)]
            let esc_chars = c" \t\"|";
            #[cfg(not(windows))]
            let esc_chars = ESCAPE_CHARS;

            let mut l = repl_cstr.as_ptr();
            while !crate::memory::cstr_is_empty(l) {
                if crate::strings::vim_haschr(
                    esc_chars,
                    i32::from(crate::memory::cstr_first_byte(l)),
                ) {
                    repl_cstr = vim_strsave_escaped(&repl_cstr, esc_chars);
                    break;
                }
                l = l.wrapping_add(1);
            }
        }

        if (eap.usefilter != 0
            || eap.cmdidx == Cmdidx::CmdBang as i32
            || eap.cmdidx == Cmdidx::CmdTerminal as i32)
            && repl_cstr.as_bytes().contains(&b'!')
        {
            repl_cstr = vim_strsave_escaped(&repl_cstr, c"!");
        }

        p = repl_cmdline(eap, p, srclen, repl_cstr.as_ptr().cast_mut(), cmdlinep);
    }

    if (eap.argt & EX_NOSPC) != 0 && eap.usefilter == 0 {
        if has_wildcards {
            if crate::strings::vim_haschr(eap.arg, b'$' as i32)
                || crate::strings::vim_haschr(eap.arg, b'~' as i32)
            {
                #[cfg(windows)]
                let esc_wildcards = c" \t*?[".as_ptr().cast_mut();
                #[cfg(not(windows))]
                let esc_wildcards = c" \t*?[{".as_ptr().cast_mut();
                expand_env_esc(
                    eap.arg,
                    g_st().name_buff.as_mut_ptr(),
                    MAXPATHL as i32,
                    esc_wildcards,
                    true,
                    ptr::null_mut(),
                );
                has_wildcards = path_has_wildcard(g_st().name_buff.as_ptr(), true);
                p = g_st().name_buff.as_mut_ptr();
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
            let mut xpc: ExpandCtx = ExpandCtx::new();
            let mut options = WILD_LIST_NOTFOUND | WILD_NOERROR | WILD_ADD_SLASH;

            xpc.xp_context = EXPAND_FILES;
            if opt_vars().p_wic != 0 {
                options += WILD_ICASE;
            }
            let Some(expanded) = expand_one(&mut xpc, eap.arg, None, options, WILD_EXPAND_FREE)
            else {
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
            to_slash(eap.arg.cast());
        }
    }
    true
}

fn repl_cmdline(
    eap: &mut Exarg,
    src: impl crate::strings::IntoCCharPtr,
    srclen: usize,
    repl: impl crate::strings::IntoCCharPtr,
    cmdlinep: *mut RawCCharPtrMut,
) -> RawCCharPtrMut {
    let mut src = src.into_c_char_ptr().cast_mut();
    let repl = repl.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(repl);
        let offset = (src as usize) - (unsafe { *cmdlinep } as usize);
        let tail_len = crate::memory::cstr_len(src.wrapping_add(srclen));
        let nextcmd_len = if !eap.nextcmd.is_null() {
            crate::memory::cstr_len(eap.nextcmd)
        } else {
            0
        };
        let total = offset + tail_len + len + 3 + nextcmd_len;
        let mut new_buf: Vec<u8> = vec![0; total];
        let new_cmdline: RawCCharPtrMut = new_buf.as_mut_ptr().cast();

        let mut i = offset;
        unsafe { ptr::copy_nonoverlapping(*cmdlinep, new_cmdline, i) };
        unsafe { ptr::copy_nonoverlapping(repl, new_cmdline.wrapping_add(i), len) };
        i += len;
        crate::memory::cstr_cpy(new_cmdline.wrapping_add(i), src.wrapping_add(srclen));
        src = new_cmdline.wrapping_add(i);

        if !eap.nextcmd.is_null() {
            i = crate::memory::cstr_len(new_cmdline) + 1;
            crate::memory::cstr_cpy(new_cmdline.wrapping_add(i), eap.nextcmd);
            eap.nextcmd = new_cmdline.wrapping_add(i);
        }
        eap.cmd = new_cmdline
            .wrapping_add((eap.cmd as usize).wrapping_sub(unsafe { *cmdlinep } as usize));
        eap.arg = new_cmdline
            .wrapping_add((eap.arg as usize).wrapping_sub(unsafe { *cmdlinep } as usize));

        for arg_j in &mut eap.args {
            let arg_off = (*arg_j as usize).wrapping_sub(unsafe { *cmdlinep } as usize);
            if offset >= arg_off {
                *arg_j = new_cmdline.wrapping_add(arg_off);
            } else {
                *arg_j = new_cmdline.wrapping_add(arg_off.wrapping_add(len).wrapping_sub(srclen));
            }
        }

        if !eap.do_ecmd_cmd.is_null()
            && !ptr::eq(
                eap.do_ecmd_cmd.cast::<u8>(),
                docmd_st().dollar_command.as_ptr(),
            )
        {
            eap.do_ecmd_cmd = new_cmdline.wrapping_add(
                (eap.do_ecmd_cmd as usize).wrapping_sub(unsafe { *cmdlinep } as usize),
            );
        }
        eap.set_cmdline_vec(cmdlinep, new_buf);

        src
    }
}

pub fn separate_nextcmd(eap: &mut Exarg) {
    let mut p = skip_grep_pat(eap);

    while !crate::memory::cstr_is_empty(p) {
        if unsafe { *p } == CTRL_V_CONST {
            if (eap.argt & (EX_CTRLV | EX_XFILE)) != 0 {
                p = p.wrapping_add(1);
            } else {
                crate::memory::cstr_move(p, p.wrapping_add(1));
            }
            if crate::memory::cstr_is_empty(p) {
                break;
            }
        } else if crate::memory::cstr_first_byte(p) == b'`'
            && crate::memory::cstr_byte_at(p, 1) == b'='
            && (eap.argt & EX_XFILE) != 0
        {
            p = p.wrapping_add(2);
            skip_expr(&raw mut p, None);
            if crate::memory::cstr_is_empty(p) {
                break;
            }
        } else if (crate::memory::cstr_first_byte(p) == b'"'
            && (eap.argt & EX_NOTRLCOM) == 0
            && (eap.cmdidx != Cmdidx::CmdAt as i32 || p != eap.arg)
            && (eap.cmdidx != Cmdidx::CmdRedir as i32
                || p != eap.arg.wrapping_add(1)
                || crate::memory::cstr_first_byte(p.wrapping_sub(1)) != b'@'))
            || (crate::memory::cstr_first_byte(p) == b'|'
                && eap.cmdidx != Cmdidx::CmdAppend as i32
                && eap.cmdidx != Cmdidx::CmdChange as i32
                && eap.cmdidx != Cmdidx::CmdInsert as i32)
            || crate::memory::cstr_first_byte(p) == b'\n'
        {
            if (!crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_BAR)
                || (eap.argt & EX_CTRLV) == 0)
                && crate::memory::cstr_first_byte(p.wrapping_sub(1)) == b'\\'
            {
                crate::memory::cstr_move(p.wrapping_sub(1), p);
                p = p.wrapping_sub(1);
            } else {
                eap.nextcmd = check_nextcmd(p);
                unsafe {
                    *p = NUL;
                }
                break;
            }
        }
        p = p.wrapping_add(utfc_ptr2len(p) as usize);
    }

    if (eap.argt & EX_NOTRLCOM) == 0 {
        del_trailing_spaces(eap.arg);
    }
}

pub fn getargcmd(argp: *mut RawCCharPtrMut) -> RawCCharPtrMut {
    let mut arg = unsafe { *argp };
    let mut command: RawCCharPtrMut = ptr::null_mut();

    if crate::memory::cstr_first_byte(arg) == b'+' {
        arg = arg.wrapping_add(1);
        if ascii_isspace(unsafe { *arg }) || crate::memory::cstr_is_empty(arg) {
            command = (&raw mut docmd_st().dollar_command).cast();
        } else {
            command = arg;
            arg = skip_cmd_arg(command, true);
            if !crate::memory::cstr_is_empty(arg) {
                unsafe {
                    *arg = NUL;
                }
                arg = arg.wrapping_add(1);
            }
        }
        arg = skipwhite(arg);
        unsafe {
            *argp = arg;
        }
    }
    command
}

pub fn skip_cmd_arg<T>(p: *mut T, rembs: bool) -> *mut T {
    let mut p = p.cast::<u8>();
    while !crate::memory::cstr_is_empty(p.cast()) && !ascii_isspace(i32::from(unsafe { *p })) {
        if crate::memory::cstr_first_byte(p.cast()) == b'\\'
            && crate::memory::cstr_byte_at(p.cast(), 1) != 0
        {
            if rembs {
                crate::memory::cstr_move(p.cast(), p.wrapping_add(1).cast());
            } else {
                p = p.wrapping_add(1);
            }
        }
        p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
    }
    p.cast()
}

pub fn get_bad_opt(p: impl crate::strings::IntoCCharPtr, eap: &mut Exarg) -> bool {
    let p = p.into_c_char_ptr();
    {
        if crate::memory::cstr_as_bytes(p).eq_ignore_ascii_case(b"keep") {
            eap.bad_char = BAD_KEEP;
        } else if crate::memory::cstr_as_bytes(p).eq_ignore_ascii_case(b"drop") {
            eap.bad_char = BAD_DROP;
        } else if utfc_ptr2len(p) == 1 && crate::memory::cstr_byte_at(p, 1) == 0 {
            eap.bad_char = i32::from(crate::memory::cstr_first_byte(p));
        } else {
            return false;
        }
        true
    }
}

fn get_bad_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    const P_BAD_VALUES: [&CStr; 3] = [c"?", c"keep", c"drop"];
    (idx >= 0)
        .then(|| P_BAD_VALUES.get(idx as usize).copied())
        .flatten()
}

pub fn getargopt(eap: &mut Exarg) -> bool {
    let mut arg = eap.arg.wrapping_add(2);
    let mut pp: *mut i32 = ptr::null_mut();
    let mut bad_char_idx: i32 = 0;

    if (crate::memory::cstr_prefix_bytes(arg, 3) == &b"bin"[..3])
        || (crate::memory::cstr_prefix_bytes(arg, 5) == &b"nobin"[..5])
    {
        if crate::memory::cstr_first_byte(arg) == b'n' {
            arg = arg.wrapping_add(2);
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

    if (crate::memory::cstr_prefix_bytes(arg, 4) == &b"edit"[..4])
        && !ascii_isalpha(crate::memory::cstr_char_at(arg, 4))
    {
        eap.read_edit = i32::from(true);
        eap.arg = skipwhite(arg.wrapping_add(4));
        return true;
    }

    if crate::memory::cstr_first_byte(arg) == b'p'
        && !ascii_isalpha(crate::memory::cstr_char_at(arg, 1))
    {
        eap.mkdir_p = i32::from(true);
        eap.arg = skipwhite(arg.wrapping_add(1));
        return true;
    }

    if crate::memory::cstr_prefix_bytes(arg, 2) == &b"ff"[..2] {
        arg = arg.wrapping_add(2);
        pp = &raw mut eap.force_ff;
    } else if crate::memory::cstr_prefix_bytes(arg, 10) == &b"fileformat"[..10] {
        arg = arg.wrapping_add(10);
        pp = &raw mut eap.force_ff;
    } else if crate::memory::cstr_prefix_bytes(arg, 3) == &b"enc"[..3] {
        if crate::memory::cstr_prefix_bytes(arg, 8) == &b"encoding"[..8] {
            arg = arg.wrapping_add(8);
        } else {
            arg = arg.wrapping_add(3);
        }
        pp = &raw mut eap.force_enc;
    } else if crate::memory::cstr_prefix_bytes(arg, 3) == &b"bad"[..3] {
        arg = arg.wrapping_add(3);
        pp = &raw mut bad_char_idx;
    }

    if pp.is_null() || crate::memory::cstr_first_byte(arg) != b'=' {
        return false;
    }

    arg = arg.wrapping_add(1);
    unsafe {
        *pp = ((arg as usize) - (eap.cmd as usize)) as i32;
    }
    arg = skip_cmd_arg(arg, false);
    eap.arg = skipwhite(arg);
    unsafe {
        *arg = NUL;
    }

    if ptr::eq(pp, &raw mut eap.force_ff) {
        if !check_ff_value(eap.cmd.wrapping_add(eap.force_ff as usize)) {
            return false;
        }
        eap.force_ff = i32::from(crate::memory::cstr_byte_at(eap.cmd, eap.force_ff as usize));
    } else if ptr::eq(pp, &raw mut eap.force_enc) {
        let mut p = eap.cmd.wrapping_add(eap.force_enc as usize);
        while !crate::memory::cstr_is_empty(p) {
            unsafe {
                *p.cast::<u8>() = tolower_asc(i32::from(crate::memory::cstr_first_byte(p))) as u8;
            }
            p = p.wrapping_add(1);
        }
    } else if !get_bad_opt(eap.cmd.wrapping_add(bad_char_idx as usize), eap) {
        return false;
    }

    true
}

fn get_argopt_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    const P_OPT_VALUES: [&CStr; 7] = [
        c"fileformat=",
        c"encoding=",
        c"binary",
        c"nobinary",
        c"bad=",
        c"edit",
        c"p",
    ];
    (idx >= 0)
        .then(|| P_OPT_VALUES.get(idx as usize).copied())
        .flatten()
}

pub fn expand_argopt(
    pat: impl crate::strings::IntoCCharPtr,
    xp: &mut ExpandCtx,
    rmp: &mut Regmatch,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    let pat = pat.into_c_char_ptr();
    {
        if xp.xp_pattern > xp.xp_line && unsafe { *xp.xp_pattern.wrapping_sub(1) } == b'=' {
            let mut cb: CompleteListItemGetter = None;
            let name_end = xp.xp_pattern.wrapping_sub(1);
            let diff = (name_end as usize) - (xp.xp_line as usize);
            if (diff >= 2
                && (crate::memory::cstr_prefix_bytes(name_end.wrapping_sub(2).cast(), 2)
                    == &b"ff"[..2]))
                || (diff >= 10
                    && (crate::memory::cstr_prefix_bytes(name_end.wrapping_sub(10).cast(), 10)
                        == &b"fileformat"[..10]))
            {
                cb = Some(get_fileformat_name);
            } else if (diff >= 3
                && (crate::memory::cstr_prefix_bytes(name_end.wrapping_sub(3).cast(), 3)
                    == &b"enc"[..3]))
                || (diff >= 8
                    && (crate::memory::cstr_prefix_bytes(name_end.wrapping_sub(8).cast(), 8)
                        == &b"encoding"[..8]))
            {
                cb = Some(get_encoding_name);
            } else if diff >= 3
                && (crate::memory::cstr_prefix_bytes(name_end.wrapping_sub(3).cast(), 3)
                    == &b"bad"[..3])
            {
                cb = Some(get_bad_name);
            }

            if cb.is_some() {
                expand_generic(pat, xp, rmp, matches, cb, false);
                return true;
            }
            return false;
        }

        if xp.xp_pattern_len == 2
            && (crate::memory::cstr_prefix_bytes(xp.xp_pattern.cast(), xp.xp_pattern_len)
                == crate::memory::cstr_prefix_bytes(c"ff", xp.xp_pattern_len))
        {
            *matches = vec![c"fileformat=".to_owned()];
            return true;
        }

        expand_generic(pat, xp, rmp, matches, Some(get_argopt_name), false);
        true
    }
}

fn get_tabpage_arg(eap: &mut Exarg) -> i32 {
    let mut tab_number: i32 = 0;
    let unaccept_arg0 = i32::from(eap.cmdidx != Cmdidx::CmdTabmove as i32);
    let last_tab = current_tab_nr(ptr::null_mut());

    'theend: {
        if !crate::memory::cstr_is_empty(eap.arg) {
            let mut p = eap.arg;
            let mut relative: i32 = 0;

            if crate::memory::cstr_first_byte(p) == b'-' {
                relative = -1;
                p = p.wrapping_add(1);
            } else if crate::memory::cstr_first_byte(p) == b'+' {
                relative = 1;
                p = p.wrapping_add(1);
            }

            let p_save = p;
            tab_number = getdigits(&mut p, false, i64::from(tab_number)) as i32;

            if relative == 0 {
                if crate::memory::cstr_as_bytes(p) == b"$" {
                    tab_number = last_tab;
                } else if crate::memory::cstr_as_bytes(p) == b"#" {
                    if valid_tabpage(g_st().lastused_tabpage) {
                        tab_number = tabpage_index(g_st().lastused_tabpage);
                    } else {
                        eap.errmsg = Some(ex_errmsg(E_INVARGVAL, eap.arg));
                        tab_number = 0;
                        break 'theend;
                    }
                } else if p == p_save
                    || crate::memory::cstr_first_byte(p_save) == b'-'
                    || !crate::memory::cstr_is_empty(p)
                    || tab_number > last_tab
                {
                    eap.errmsg = Some(ex_errmsg(E_INVARG2, eap.arg));
                    break 'theend;
                }
            } else {
                if crate::memory::cstr_is_empty(p_save) {
                    tab_number = 1;
                } else if p == p_save
                    || crate::memory::cstr_first_byte(p_save) == b'-'
                    || !crate::memory::cstr_is_empty(p)
                    || tab_number == 0
                {
                    eap.errmsg = Some(ex_errmsg(E_INVARG2, eap.arg));
                    break 'theend;
                }
                tab_number = tab_number * relative + tabpage_index(g_st().curtab);
                if unaccept_arg0 == 0 && relative == -1 {
                    tab_number -= 1;
                }
            }
            if tab_number < unaccept_arg0 || tab_number > last_tab {
                eap.errmsg = Some(ex_errmsg(E_INVARG2, eap.arg));
            }
        } else if eap.addr_count > 0 {
            if unaccept_arg0 != 0 && eap.line2 == 0 {
                eap.errmsg = Some(gettext(E_INVRANGE));
                tab_number = 0;
            } else {
                tab_number = eap.line2;
                if unaccept_arg0 == 0 {
                    let mut cmdp = eap.cmd;
                    while {
                        cmdp = cmdp.wrapping_sub(1);
                        cmdp > unsafe { *eap.cmdlinep }
                            && (ascii_iswhite(unsafe { *cmdp }) || ascii_isdigit(unsafe { *cmdp }))
                    } {}
                    if crate::memory::cstr_first_byte(cmdp) == b'-' {
                        tab_number -= 1;
                        if tab_number < unaccept_arg0 {
                            eap.errmsg = Some(gettext(E_INVRANGE));
                        }
                    }
                }
            }
        } else if eap.cmdidx == Cmdidx::CmdTabnextVariant as i32 {
            tab_number = tabpage_index(g_st().curtab) + 1;
            if tab_number > last_tab {
                tab_number = 1;
            }
        } else if eap.cmdidx == Cmdidx::CmdTabmove as i32 {
            tab_number = last_tab;
        } else {
            tab_number = tabpage_index(g_st().curtab);
        }
    }

    tab_number
}

fn ex_autocmd(eap: &mut Exarg) {
    if g_st().secure != 0 {
        g_st().secure = 2;
        eap.errmsg = Some(gettext(E_CURDIR));
    } else if eap.cmdidx == Cmdidx::CmdAutocmd as i32 {
        do_autocmd(eap, eap.arg, eap.forceit);
    } else {
        do_augroup(eap.arg, eap.forceit != 0);
    }
}

fn ex_doautocmd(eap: &mut Exarg) {
    let mut arg = eap.arg;
    let call_do_modelines = check_nomodeline(&mut arg);
    let mut did_aucmd = false;

    do_doautocmd(arg, false, Some(&mut did_aucmd));
    if call_do_modelines && did_aucmd {
        do_modelines(0);
    }
}

fn ex_bunload(eap: &mut Exarg) {
    let cmd = if eap.cmdidx == Cmdidx::CmdBdelete as i32 {
        DOBUF_DEL
    } else if eap.cmdidx == Cmdidx::CmdBwipeout as i32 {
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

fn ex_buffer(eap: &mut Exarg) {
    do_exbuffer(eap);
}

fn do_exbuffer(eap: &mut Exarg) {
    if !crate::memory::cstr_is_empty(eap.arg) {
        eap.errmsg = Some(ex_errmsg(E_TRAILING_ARG, eap.arg));
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

fn ex_bmodified(eap: &mut Exarg) {
    goto_buffer(eap, DOBUF_MOD, FORWARD, eap.line2);
    if !eap.do_ecmd_cmd.is_null() {
        do_cmdline_cmd(eap.do_ecmd_cmd);
    }
}

fn ex_bnext(eap: &mut Exarg) {
    goto_buffer(eap, DOBUF_CURRENT, FORWARD, eap.line2);
    if !eap.do_ecmd_cmd.is_null() {
        do_cmdline_cmd(eap.do_ecmd_cmd);
    }
}

fn ex_bprevious(eap: &mut Exarg) {
    goto_buffer(eap, DOBUF_CURRENT, BACKWARD, eap.line2);
    if !eap.do_ecmd_cmd.is_null() {
        do_cmdline_cmd(eap.do_ecmd_cmd);
    }
}

fn ex_brewind(eap: &mut Exarg) {
    goto_buffer(eap, DOBUF_FIRST, FORWARD, 0);
    if !eap.do_ecmd_cmd.is_null() {
        do_cmdline_cmd(eap.do_ecmd_cmd);
    }
}

fn ex_blast(eap: &mut Exarg) {
    goto_buffer(eap, DOBUF_LAST, BACKWARD, 0);
    if !eap.do_ecmd_cmd.is_null() {
        do_cmdline_cmd(eap.do_ecmd_cmd);
    }
}

pub fn ends_excmd(c: i32) -> i32 {
    i32::from(c == i32::from(NUL) || c == b'|' as i32 || c == b'"' as i32 || c == b'\n' as i32)
}

pub fn find_nextcmd(p: impl crate::strings::IntoCCharPtr) -> RawCCharPtrMut {
    let mut p = p.into_c_char_ptr();
    {
        while crate::memory::cstr_first_byte(p) != b'|'
            && crate::memory::cstr_first_byte(p) != b'\n'
        {
            if crate::memory::cstr_is_empty(p) {
                return ptr::null_mut();
            }
            p = p.wrapping_add(1);
        }
        p.wrapping_add(1).cast_mut()
    }
}

pub fn check_nextcmd(p: impl crate::strings::IntoCCharPtr) -> RawCCharPtrMut {
    let p = p.into_c_char_ptr();
    {
        let s = skipwhite(p);
        if crate::memory::cstr_first_byte(s) == b'|' || crate::memory::cstr_first_byte(s) == b'\n' {
            return s.wrapping_add(1);
        }
        ptr::null_mut()
    }
}

fn check_more(message: bool, forceit: bool) -> bool {
    let ac = argcount();
    let n = ac - curwin_mut().w_arg_idx - 1;

    if !forceit
        && only_one_window()
        && ac > 1
        && !g_st().arg_had_last
        && n > 0
        && docmd_st().quitmore == 0
    {
        if message {
            if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                && !curbuf_mut().b_fname.is_null()
            {
                let mut buff = [0u8; DIALOG_MSG_SIZE];
                let _ = vim_snprintf!(
                    buff.as_mut_ptr().cast(),
                    DIALOG_MSG_SIZE,
                    ngettext(
                        c"%d more file to edit.  Quit anyway?",
                        c"%d more files to edit.  Quit anyway?",
                        n as i64,
                    ),
                    n,
                );
                if vim_dialog_yesno(VIM_QUESTION, ptr::null_mut(), &buff, 1) == VIM_YES {
                    return true;
                }
                return false;
            }
            let _ = semsg!(
                ngettext(
                    c"E173: %d more file to edit",
                    c"E173: %d more files to edit",
                    n as i64,
                ),
                n,
            );
            docmd_st().quitmore = 2;
        }
        return false;
    }
    true
}

pub fn get_command_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx >= CMD_SIZE_I32 {
        return expand_user_command_name(idx);
    }
    (idx >= 0).then(|| CMDNAMES[idx as usize].cmd_name)
}

fn ex_colorscheme(eap: &mut Exarg) {
    if crate::memory::cstr_is_empty(eap.arg) {
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
        let _ = semsg!(gettext(c"E185: Cannot find color scheme '%s'"), eap.arg,);
    }
}

fn ex_highlight(eap: &mut Exarg) {
    if crate::memory::cstr_is_empty(eap.arg) && crate::memory::cstr_byte_at(eap.cmd, 2) == b'!' {
        let _ = msg(gettext(c"Greetings, Vim user!"), 0);
    }
    do_highlight(eap.arg, eap.forceit != 0, false);
}

pub fn not_exiting(save_exiting: bool) {
    g_st().exiting = save_exiting;
    set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
}

pub fn before_quit_autocmds(wp: *mut WinInfoInfo, quit_all: bool, forceit: bool) -> bool {
    unsafe {
        if crate::memory::cstr_is_empty(get_vim_var_str(VV_EXITREASON)) {
            set_vim_var_string(VV_EXITREASON, c"quit", 4);
        }
        apply_autocmds_win(
            EVENT_QUITPRE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            (*wp).w_buffer,
            wp.as_ref(),
        );

        if !win_valid(wp)
            || curbuf_locked()
            || ((*wp).buf_mut().b_nwindows == 1 && (*wp).buf_mut().b_locked > 0)
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
                || (curbuf_mut().b_nwindows == 1 && curbuf_mut().b_locked > 0)
            {
                set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
                return true;
            }
        }

        false
    }
}

fn ex_quit(eap: &mut Exarg) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }

        let mut wp: *mut WinInfoInfo;
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
            wp = curwin_ptr();
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
        if (!buf_hide((*wp).buf_mut())
            && check_changed(
                (*wp).buf_mut(),
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
                !buf_hide((*wp).buf_mut()) || eap.forceit != 0,
                eap.forceit != 0,
            );
        }
    }
}

fn ex_cquit(eap: &mut Exarg) {
    let status = if eap.addr_count > 0 { eap.line2 } else { 1 };
    ui_call_error_exit(Integer::from(status));
    getout(status);
}

pub fn before_quit_all(eap: &mut Exarg) -> bool {
    if text_locked() {
        text_locked_msg();
        return false;
    }
    if before_quit_autocmds(curwin_ptr(), true, eap.forceit != 0) {
        return false;
    }
    true
}

fn ex_quitall(eap: &mut Exarg) {
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

fn ex_exmode(_eap: &mut Exarg) {
    if g_st().silent_mode {
        return;
    }
    if g_st().ex_normal_busy > 0 || g_st().global_busy != 0 {
        flush_buffers(FLUSH_TYPEAHEAD);
        return;
    }
    let mut args = [TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    }];
    nlua_call_typval(c"vim._core.exmode", c"open", args.as_mut_ptr(), None);
}

fn ex_restart(eap: &mut Exarg) {
    unsafe {
        if eap.forceit == 0 {
            let extra_d = tv_dict_alloc();
            tv_dict_add_str(
                &mut *extra_d,
                c"quit_cmd",
                if !eap.do_ecmd_cmd.is_null() {
                    eap.do_ecmd_cmd
                } else {
                    c"".as_ptr()
                },
            );
            let mut extra_tv = TypVal {
                v_type: VarType::VarDict,
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
            tv_clear(&mut extra_tv);
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
                *arg1.wrapping_add(arglen1) = NUL;
                quit_cmd = arg1;
                after_cmd = if eap.args.len() > 2 {
                    eap.args.as_slice()[2]
                } else {
                    c"".as_ptr().cast_mut()
                };
            } else {
                let _ = semsg!(E_RESTART_FAILED_CMD_NO_QUIT, quit_cmd);
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
                    if crate::memory::cstr_first_byte(next_arg) != b'-' {
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
                    let next_li = &mut *next_li;
                    let addr = tv_get_string(&next_li.li_tv);
                    if crate::memory::cstr_as_bytes(addr)
                        .iter()
                        .any(|b| matches!(b, b':' | b'/' | b'\\'))
                    {
                        listen_arg = to_slash_save(addr);
                        #[cfg(windows)]
                        {
                            li = next_li.li_next;
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
                    cstr_as_string(c"return require('vim._core.server').restart_canonical_addr"),
                    ptr::null(),
                    Array {
                        items: ptr::null_mut(),
                        size: 0,
                        capacity: 0,
                    },
                    K_RET_OBJECT,
                    None,
                    &mut lua_err,
                );
                if !error_set(&lua_err)
                    && rv.obj_type == ObjectType::ObjectTypeString
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
            server_stop(la, true)
        } else {
            false
        };

        let env = create_environment(None, false, false, false, ptr::null());
        tv_dict_add_str(&mut *env, ENV_STARTREASON, startreason);

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
        let mut exit_status: Varnumber = 0;

        let channel = channel_job_start(
            argv_vec,
            crate::memory::cstr_as_opt_cstr(exepath),
            on_stdout,
            on_err,
            on_exit,
            false,
            true,
            true,
            true,
            K_CHANNEL_STDIN_PIPE_CONST,
            (&raw const g_st().globaldir).as_ref().unwrap().as_deref(),
            0,
            0,
            env,
            &raw mut exit_status,
        );
        if channel.is_null() {
            let _ = emsg(c"cannot create a channel job");
            let la_ptr = listen_arg.as_ref().map_or(ptr::null(), |s| s.as_ptr());
            if server_stopped && server_start(listen_arg.as_deref()) != 0 {
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
                    obj_type: ObjectType::ObjectTypeDict,
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
                c"nvim_chan_set",
                detach_args,
                &mut result_mem,
                &mut err,
            );
            if error_set(&err) {
                break 'fail_2;
            }
            arena_mem_free(result_mem);
            result_mem = ptr::null_mut();

            if !crate::memory::cstr_is_empty(after_cmd) {
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
                        obj_type: ObjectType::ObjectTypeDict,
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
                    c"nvim_create_autocmd",
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
                c"nvim_get_vvar",
                servername_args,
                &mut result_mem,
                &mut err,
            );
            if error_set(&err) {
                break 'fail_2;
            }
            if result.obj_type != ObjectType::ObjectTypeString || result.data.string().size == 0 {
                let _ = emsg(c"restart failed: could not get listen address from new server");
                break 'fail_2;
            }
            let listen_addr = cstr_from_bytes(crate::memory::cstr_slice(
                result.data.string().data,
                result.data.string().size,
            ));
            arena_mem_free(result_mem);
            result_mem = ptr::null_mut();

            #[cfg(windows)]
            if let Some(ref la) = listen_arg {
                let mut handoff_items = [
                    cstr_as_obj(la.as_ptr()),
                    integer_obj(ui_active() as Integer),
                ];
                let handoff_params = Array {
                    items: handoff_items.as_mut_ptr(),
                    size: 2,
                    capacity: 3,
                };
                let mut lua_items = [
                    cstr_as_obj(c"return require('vim._core.server').rebind_after_restart(...)"),
                    Object {
                        obj_type: ObjectType::ObjectTypeArray,
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
                    c"nvim_exec_lua",
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
                let _ = emsg(err.msg_cstr());
                api_clear_error(&mut err);
            } else if !g_st().exiting {
                let _ = semsg!(E_RESTART_FAILED_CMD_NO_QUIT, quit_cmd);
            }
            drop(quit_cmd_copy);
        }

        set_vim_var_string(VV_EXITREASON, ptr::null(), -1);
        if error_set(&err) {
            let _ = emsg(err.msg_cstr());
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
                c"nvim_eval",
                chanclose_args,
                &mut result_mem,
                &mut err,
            );
            api_clear_error(&mut err);
            arena_mem_free(result_mem);
        }

        let proc_ptr = &raw mut (*channel).uv.proc_;
        proc_stop(proc_ptr);
        if proc_wait(proc_ptr, -1, ptr::null_mut()) < 0 {
            let _ = emsg(c"killing new nvim server failed");
        }

        let la_ptr = listen_arg.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        if server_stopped && server_start(listen_arg.as_deref()) != 0 {
            let _ = semsg!(c"couldn't resume listening on %s", la_ptr);
        }
    }
}

fn ex_close(eap: &mut Exarg) {
    if !text_locked() && !curbuf_locked() {
        let win = if eap.addr_count == 0 {
            curwin_ptr()
        } else {
            let w = win_find_nr(eap.line2, g_st().curtab);
            if w.is_null() { g_st().lastwin } else { w }
        };
        ex_win_close(eap.forceit, unsafe { &mut *win }, ptr::null_mut());
    }
}

fn ex_pclose(eap: &mut Exarg) {
    for win in for_all_windows_mut() {
        if win.w_onebuf_opt.wo_pvw != 0 {
            ex_win_close(eap.forceit, win, ptr::null_mut());
            break;
        }
    }
    win_float_close(K_WIN_PREVIEW);
}

pub fn ex_win_close(forceit: i32, win: &mut WinInfoInfo, tp: *mut TabPageInfo) {
    debug_assert!(tp.is_null() || tp != g_st().curtab);

    if is_ctx_win(&raw mut *win) {
        let _ = emsg(gettext(E_AUTOCMD_CLOSE));
        return;
    }
    if !win.w_floating && window_layout_locked(Cmdidx::CmdClose) {
        return;
    }

    let buf = win.w_buffer;

    let mut need_hide =
        buf_is_changed_fn(unsafe { &mut *buf }) && unsafe { (*buf).b_nwindows } <= 1;
    if need_hide && !buf_hide(unsafe { &*buf }) && forceit == 0 {
        if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
            && opt_vars().p_write != 0
        {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, buf);
            dialog_changed(unsafe { &mut *buf }, false);
            if bufref_valid(Some(&bufref)) && buf_is_changed_fn(unsafe { &mut *buf }) {
                return;
            }
            need_hide = false;
        } else {
            no_write_message();
            return;
        }
    }

    if tp.is_null() {
        win_close(
            &raw mut *win,
            !need_hide && !buf_hide(unsafe { &*buf }),
            forceit != 0,
        );
    } else {
        win_close_othertab(
            &raw mut *win,
            i32::from(!need_hide && !buf_hide(unsafe { &*buf })),
            tp,
            forceit != 0,
        );
    }
}

fn ex_tabclose(eap: &mut Exarg) {
    if crate::globals::first_tabpage_mut().tp_next.is_null() {
        let _ = emsg(gettext(c"E784: Cannot close last tab page"));
        return;
    }

    if window_layout_locked(Cmdidx::CmdTabclose) {
        return;
    }

    let tab_number = get_tabpage_arg(eap);
    if eap.errmsg.is_some() {
        return;
    }

    let tp = find_tabpage(tab_number);
    if tp.is_null() {
        beep_flush();
        return;
    }
    if tp != g_st().curtab {
        tabpage_close_other(tp, eap.forceit);
    } else if !text_locked() && !curbuf_locked() {
        tabpage_close(eap.forceit);
    }
}

fn ex_tabonly(eap: &mut Exarg) {
    if crate::globals::first_tabpage_mut().tp_next.is_null() {
        let _ = msg(gettext(c"Already only one tab page"), 0);
        return;
    }

    if window_layout_locked(Cmdidx::CmdTabonly) {
        return;
    }

    let tab_number = get_tabpage_arg(eap);
    if eap.errmsg.is_some() {
        return;
    }

    goto_tabpage(tab_number);
    let mut done: i32 = 0;
    while done < 1000 {
        for tp in for_all_tabpages_ptr() {
            if tp.tp_topframe != g_st().topframe.cast() {
                tabpage_close_other(tp, eap.forceit);
                if valid_tabpage(tp) {
                    done = 1000;
                }
                break;
            }
        }
        if crate::globals::first_tabpage_mut().tp_next.is_null() {
            break;
        }
        done += 1;
    }
}

pub fn tabpage_close(forceit: i32) {
    let mut done: i32 = 0;

    if window_layout_locked(Cmdidx::CmdTabclose) {
        return;
    }

    trigger_tabclosedpre(g_st().curtab);
    curtab_mut().tp_did_tabclosedpre = true;
    let save_curtab = g_st().curtab;

    while curwin_mut().w_floating && {
        done += 1;
        done < 1000
    } {
        ex_win_close(forceit, curwin_mut(), ptr::null_mut());
    }
    if done == 1000 {
        return;
    }

    if g_st().firstwin != g_st().lastwin {
        close_others(1, forceit, true);
    }
    if g_st().firstwin == g_st().lastwin {
        ex_win_close(forceit, curwin_mut(), ptr::null_mut());
    }
    if g_st().curtab == save_curtab {
        curtab_mut().tp_did_tabclosedpre = false;
    }
}

pub fn tabpage_close_other(tp: *mut TabPageInfo, forceit: i32) {
    unsafe {
        let mut done: i32 = 0;
        let mut prev_idx = [0u8; NUMBUFLEN];

        debug_assert!(!ptr::eq(tp, g_st().curtab));

        if window_layout_locked(Cmdidx::CmdSize) {
            return;
        }

        trigger_tabclosedpre(&mut *tp);
        (*tp).tp_did_tabclosedpre = true;

        while {
            done += 1;
            done < 1000
        } {
            let _ = vim_snprintf!(
                prev_idx.as_mut_ptr().cast(),
                NUMBUFLEN,
                c"%i",
                tabpage_index(tp),
            );
            let wp = (*tp).tp_lastwin;

            if ptr::eq(tp, g_st().curtab) {
                done = 1000;
                break;
            }
            ex_win_close(forceit, &mut *wp, tp);

            if !valid_tabpage(tp) {
                break;
            }

            if (*tp).tp_lastwin == wp {
                done = 1000;
                break;
            }
        }
        if done >= 1000 && valid_tabpage(tp) {
            (*tp).tp_did_tabclosedpre = false;
        }
    }
}

fn ex_only(eap: &mut Exarg) {
    unsafe {
        if window_layout_locked(Cmdidx::CmdOnly) {
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
            if wp != curwin_ptr() {
                win_goto(wp);
            }
        }
        close_others(1, eap.forceit, false);
    }
}

fn ex_hide(eap: &mut Exarg) {
    if eap.skip != 0 {
        return;
    }

    let win = if eap.addr_count == 0 {
        curwin_ptr()
    } else {
        let w = win_find_nr(eap.line2, g_st().curtab);
        if w.is_null() { g_st().lastwin } else { w }
    };

    if !unsafe { (*win).w_floating } && window_layout_locked(Cmdidx::CmdHide) {
        return;
    }

    win_close(win, false, eap.forceit != 0);
}

fn ex_stop(eap: &mut Exarg) {
    if eap.forceit == 0 {
        autowrite_all();
    }
    may_trigger_vim_suspend_resume(true);
    ui_call_suspend();
    ui_flush();
}

fn ex_exit(eap: &mut Exarg) {
    if text_locked() {
        text_locked_msg();
        return;
    }

    let save_exiting = g_st().exiting;
    if check_more(false, eap.forceit != 0) && only_one_window() {
        g_st().exiting = true;
    }
    if ((eap.cmdidx == Cmdidx::CmdWq as i32 || curbuf_is_changed_fn()) && !do_write(eap))
        || before_quit_autocmds(curwin_ptr(), false, eap.forceit != 0)
        || !check_more(true, eap.forceit != 0)
        || (only_one_window() && check_changed_any(eap.forceit != 0, false))
    {
        not_exiting(save_exiting);
    } else {
        if only_one_window() {
            getout(0);
        }
        not_exiting(save_exiting);
        win_close(
            curwin_ptr(),
            !buf_hide(curwin_mut().buf_mut()),
            eap.forceit != 0,
        );
    }
}

fn ex_print(eap: &mut Exarg) {
    if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
        let _ = emsg(gettext(E_EMPTY_BUFFER));
    } else {
        let mut line = eap.line1;
        while line <= eap.line2 && !g_st().got_int {
            print_line(
                line,
                eap.cmdidx == Cmdidx::CmdNumber as i32
                    || eap.cmdidx == Cmdidx::CmdPound as i32
                    || (eap.flags & EXFLAG_NR) != 0,
                eap.cmdidx == Cmdidx::CmdList as i32 || (eap.flags & EXFLAG_LIST) != 0,
                line == eap.line1,
            );
            line += 1;
            os_breakcheck();
        }
        setpcmark();
        curwin_mut().w_cursor.lnum = eap.line2;
        beginline(BL_SOL | BL_FIX);
    }
}

fn ex_goto(eap: &mut Exarg) {
    goto_byte(eap.line2);
}

fn ex_preserve(_eap: &mut Exarg) {
    ml_preserve(curbuf_mut(), true, true);
}

fn ex_recover(eap: &mut Exarg) {
    g_st().recoverymode = true;
    if !check_changed(
        curbuf_mut(),
        (if opt_vars().p_awa != 0 { CCGD_AW } else { 0 })
            | CCGD_MULTWIN
            | (if eap.forceit != 0 { CCGD_FORCEIT } else { 0 })
            | CCGD_EXCMD,
    ) && (crate::memory::cstr_is_empty(eap.arg)
        || setfname(curbuf_mut(), eap.arg, ptr::null_mut(), true))
    {
        ml_recover(true);
    }
    g_st().recoverymode = false;
}

fn ex_wrongmodifier(eap: &mut Exarg) {
    eap.errmsg = Some(gettext(E_INVCMD));
}

fn call_findfunc(pat: impl crate::strings::IntoCCharPtr, cmdcomplete: BoolVarValue) -> *mut TvList {
    let pat = pat.into_c_char_ptr().cast_mut();
    {
        let saved_sctx = g_st().current_sctx;

        let mut args = [
            TypVal {
                v_type: VarType::VarString,
                v_lock: 0,
                vval: TypvalVval::String(pat),
            },
            TypVal {
                v_type: VarType::VarBool,
                v_lock: 0,
                vval: TypvalVval::Bool(cmdcomplete),
            },
            TypVal {
                v_type: VarType::VarUnknown,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
        ];

        g_st().textlock += 1;

        let ctx = get_option_sctx(K_OPT_FINDFUNC);
        g_st().current_sctx = *ctx;

        let cb = get_findfunc();
        let mut rettv: TypVal = TypVal::new();
        let retval = callback_call(&mut *cb, 2, args.as_mut_ptr(), &mut rettv);

        g_st().current_sctx = saved_sctx;
        g_st().textlock -= 1;

        let mut retlist: *mut TvList = ptr::null_mut();
        if retval {
            if rettv.v_type == VarType::VarList {
                retlist = tv_list_copy(
                    None,
                    unsafe { rettv.vval.v_list().as_mut() },
                    false,
                    get_copy_id(),
                );
            } else {
                let _ = emsg(gettext(E_INVALID_RETURN_TYPE_FROM_FINDFUNC));
            }
            tv_clear(&mut rettv);
        }

        retlist
    }
}

pub fn expand_findfunc(
    xp: &mut ExpandCtx,
    pat: impl crate::strings::IntoCCharPtr,
    files: &mut Vec<std::ffi::CString>,
) -> bool {
    files.clear();

    let l = call_findfunc(pat, K_BOOL_VAR_TRUE_CONST);
    if l.is_null() {
        return false;
    }

    let len = unsafe { (*l).lv_len };
    if len == 0 {
        tv_list_free(l);
        return false;
    }

    crate::cmdexpand::expand_process_user_list(Some(unsafe { &*l }), files, xp);
    tv_list_free(l);

    true
}

fn findfunc_find_file<T>(
    findarg: *mut T,
    findarg_len: usize,
    count: i32,
) -> Option<std::ffi::CString> {
    let findarg = findarg.cast::<u8>();
    let mut ret_fname: Option<std::ffi::CString> = None;
    let cc = unsafe { *findarg.wrapping_add(findarg_len) };
    unsafe {
        *findarg.wrapping_add(findarg_len) = 0;
    }

    let fname_list = call_findfunc(findarg.cast(), K_BOOL_VAR_FALSE_CONST);
    let fname_count = if fname_list.is_null() {
        0
    } else {
        unsafe { (*fname_list).lv_len }
    };

    if fname_count == 0 {
        let _ = semsg!(gettext(E_CANT_FIND_FILE_STR_IN_PATH), findarg);
    } else if count > fname_count {
        let _ = semsg!(gettext(E_NO_MORE_FILE_STR_FOUND_IN_PATH), findarg);
    } else {
        let li = tv_list_find(Some(unsafe { &mut *fname_list }), count - 1);
        if !li.is_null() {
            let tv = unsafe { &(*li).li_tv };
            if tv.v_type == VarType::VarString && !tv.vval.v_string().is_null() {
                ret_fname = Some(cstr_to_cstring(tv.vval.v_string()));
            } else if tv.v_type == VarType::VarDict && !tv.vval.v_dict().is_null() {
                ret_fname = crate::eval::typval::tv_dict_get_cstring(
                    unsafe { (tv.vval.v_dict()).as_ref() },
                    c"word",
                );
            }
        }
    }

    if !fname_list.is_null() {
        tv_list_free(fname_list);
    }

    unsafe {
        *findarg.wrapping_add(findarg_len) = cc;
    }
    if let Some(ref rf) = ret_fname {
        to_slash(rf.as_ptr().cast_mut().cast());
    }
    ret_fname
}

pub fn free_findfunc_option() {
    callback_free(&mut opt_vars().p_ffu);
}

pub fn set_ref_in_findfunc(copy_id: i32) -> bool {
    set_ref_in_callback(
        &mut opt_vars().p_ffu,
        copy_id,
        ptr::null_mut(),
        ptr::null_mut(),
    )
}

fn set_browse_edit_arg(eap: &mut Exarg) {
    if (g_st().cmdmod.cmod_flags & CMOD_BROWSE) != 0
        && crate::memory::cstr_is_empty(eap.arg)
        && (eap.cmdidx == Cmdidx::CmdEdit as i32
            || eap.cmdidx == Cmdidx::CmdSplit as i32
            || eap.cmdidx == Cmdidx::CmdVsplit as i32
            || eap.cmdidx == Cmdidx::CmdTabedit as i32
            || eap.cmdidx == Cmdidx::CmdTabnew as i32)
    {
        eap.arg = c".".as_ptr().cast_mut();
    }
}

pub fn ex_splitview(eap: &mut Exarg) {
    set_browse_edit_arg(eap);
    let old_curwin = curwin_ptr();
    let mut fname: Option<std::ffi::CString> = None;
    let use_tab = eap.cmdidx == Cmdidx::CmdTabedit as i32
        || eap.cmdidx == Cmdidx::CmdTabfind as i32
        || eap.cmdidx == Cmdidx::CmdTabnew as i32;

    if bt_quickfix(Some(crate::globals::curbuf_mut())) && g_st().cmdmod.cmod_tab == 0 {
        if eap.cmdidx == Cmdidx::CmdSplit as i32 {
            eap.cmdidx = Cmdidx::CmdNew as i32;
        }
        if eap.cmdidx == Cmdidx::CmdVsplit as i32 {
            eap.cmdidx = Cmdidx::CmdVnew as i32;
        }
    }

    'theend: {
        if eap.cmdidx == Cmdidx::CmdSfind as i32 || eap.cmdidx == Cmdidx::CmdTabfind as i32 {
            if get_findfunc().cb_type() != K_CALLBACK_NONE_CONST {
                fname = findfunc_find_file(
                    eap.arg,
                    crate::memory::cstr_len(eap.arg),
                    if eap.addr_count > 0 { eap.line2 } else { 1 },
                );
            } else {
                let mut file_to_find: Option<std::ffi::CString> = None;
                let mut search_ctx: Option<FfSearchCtx> = None;
                fname = find_file_in_path(
                    eap.arg,
                    crate::memory::cstr_len(eap.arg),
                    FNAME_MESS,
                    i32::from(true),
                    curbuf_mut().b_ffname,
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

                if curwin_ptr() != old_curwin
                    && win_valid(old_curwin)
                    && unsafe { (*old_curwin).w_buffer } != g_st().curbuf
                    && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0
                {
                    unsafe {
                        (*old_curwin).w_alt_fnum = curbuf_mut().b_fnum;
                    }
                }
            }
        } else if win_split(
            if eap.addr_count > 0 { eap.line2 } else { 0 },
            if crate::memory::cstr_first_byte(eap.cmd) == b'v' {
                WSP_VERT
            } else {
                0
            },
        ) {
            if !crate::memory::cstr_is_empty(eap.arg) {
                curwin_mut().w_onebuf_opt.wo_scb = 0;
                curwin_mut().w_onebuf_opt.wo_crb = 0;
            } else {
                do_check_scrollbind(false);
            }
            do_exedit(eap, old_curwin);
        }
    }

    drop(fname);
}

pub fn tabpage_new() {
    let mut ea: Exarg = Exarg::new();
    ea.cmdidx = Cmdidx::CmdTabnew as i32;
    ea.cmd = c"tabn".as_ptr().cast_mut();
    ea.arg = c"".as_ptr().cast_mut();
    ex_splitview(&mut ea);
}

fn ex_tabnext(eap: &mut Exarg) {
    if eap.cmdidx == Cmdidx::CmdTabfirst as i32 || eap.cmdidx == Cmdidx::CmdTabrewind as i32 {
        goto_tabpage(1);
    } else if eap.cmdidx == Cmdidx::CmdTablast as i32 {
        goto_tabpage(9999);
    } else if eap.cmdidx == Cmdidx::CmdTabprevious as i32 || eap.cmdidx == Cmdidx::CmdTabnext as i32
    {
        let tab_number: i32;
        if !crate::memory::cstr_is_empty(eap.arg) {
            let mut p = eap.arg;
            let p_save = p;
            tab_number = getdigits(&mut p, false, 0) as i32;
            if p == p_save
                || crate::memory::cstr_first_byte(p_save) == b'-'
                || crate::memory::cstr_first_byte(p_save) == b'+'
                || !crate::memory::cstr_is_empty(p)
                || tab_number == 0
            {
                eap.errmsg = Some(ex_errmsg(E_INVARG2, eap.arg));
                return;
            }
        } else if eap.addr_count == 0 {
            tab_number = 1;
        } else {
            tab_number = eap.line2;
            if tab_number < 1 {
                eap.errmsg = Some(gettext(E_INVRANGE));
                return;
            }
        }
        goto_tabpage(-tab_number);
    } else {
        let tab_number = get_tabpage_arg(eap);
        if eap.errmsg.is_none() {
            goto_tabpage(tab_number);
        }
    }
}

fn ex_tabmove(eap: &mut Exarg) {
    let tab_number = get_tabpage_arg(eap);
    if eap.errmsg.is_none() {
        tabpage_move(tab_number);
    }
}

fn ex_tabs(_eap: &mut Exarg) {
    let mut tabcount: i32 = 1;

    msg_ext_set_kind(c"list_cmd");
    msg_start();
    g_st().msg_scroll = i32::from(true);

    let lastused_win = if valid_tabpage(g_st().lastused_tabpage) {
        unsafe { (*g_st().lastused_tabpage).tp_curwin }
    } else {
        ptr::null_mut()
    };

    let mut tp = g_st().first_tabpage;
    while !tp.is_null() {
        if g_st().got_int {
            break;
        }

        if g_st().msg_col > 0 {
            msg_putchar(b'\n' as i32);
        }
        let _ = vim_snprintf!(
            g_st().iobuff.as_mut_ptr(),
            IOSIZE,
            gettext(c"Tab page %d"),
            tabcount,
        );
        tabcount += 1;
        let _ = msg_outtrans(g_st().iobuff.as_ptr(), HLF_T, false);
        os_breakcheck();

        for wp in win_iter_ptr(if ptr::eq(tp, g_st().curtab) {
            g_st().firstwin
        } else {
            unsafe { (*tp).tp_firstwin }
        }) {
            if g_st().got_int {
                break;
            }
            if !wp.w_config.focusable || wp.w_config.hide {
                continue;
            }

            msg_putchar(b'\n' as i32);
            msg_putchar(if std::ptr::eq(wp, curwin_ptr()) {
                b'>' as i32
            } else if std::ptr::eq(wp, lastused_win) {
                b'#' as i32
            } else {
                b' ' as i32
            });
            msg_putchar(b' ' as i32);
            msg_putchar(if buf_is_changed_fn((*wp).buf_mut()) {
                b'+' as i32
            } else {
                b' ' as i32
            });
            msg_putchar(b' ' as i32);
            if let Some(spname) = buf_spname(unsafe { &*(wp.w_buffer) }) {
                xstrlcpy(g_st().iobuff.as_mut_ptr(), spname, IOSIZE);
            } else {
                home_replace(
                    unsafe { (wp.w_buffer).as_ref() },
                    (*wp).buf_mut().b_fname,
                    g_st().iobuff.as_mut_ptr(),
                    IOSIZE,
                    true,
                );
            }
            let _ = msg_outtrans(g_st().iobuff.as_ptr(), 0, false);
            os_breakcheck();
        }

        tp = unsafe { (*tp).tp_next };
    }
}

fn ex_detach(eap: &mut Exarg) {
    let chan = find_channel(g_st().current_ui);
    if chan.is_null() {
        let _ = emsg(gettext(E_NOUI));
        return;
    }

    if eap.forceit != 0 {
        unsafe {
            (*chan).detach = true;
        }
        channel_event(unsafe { &mut *chan }, EVENT_CHANINFO);
        let _ = msg(
            gettext(c"Nvim will continue running if the UI disconnects"),
            0,
        );
        return;
    }

    if eap.addr_count > 0 {
        if eap.line1 != 1 || eap.line2 != curbuf_mut().b_ml.ml_line_count {
            let _ = emsg(gettext(E_INVRANGE));
            return;
        }

        let n = ui_detach_others(g_st().current_ui);
        if n == 0 {
            let _ = msg(gettext(c"No other UIs are attached"), 0);
        } else {
            let _ = smsg!(0, gettext(c"Detached %d non-current UIs"), n as i32,);
        }
        if log_should_log(LOGLVL_INF) {
            logmsg!(
                LOGLVL_INF,
                ptr::null(),
                c"ex_detach",
                line!() as i32,
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
            let _ = emsg(err.msg_cstr());
            api_clear_error(&mut err);
            return;
        }
        if log_should_log(LOGLVL_INF) {
            logmsg!(
                LOGLVL_INF,
                ptr::null(),
                c"ex_detach",
                line!() as i32,
                true,
                c"detach current_ui=%llu",
                g_st().current_ui,
            );
        }
    }
}

fn ex_connect(eap: &mut Exarg) {
    let stop_server = if eap.forceit != 0 {
        ui_active() == 1
    } else {
        false
    };

    let mut err = error_init();
    remote_ui_connect(
        g_st().current_ui,
        crate::memory::cstr_as_bytes(eap.arg),
        &mut err,
    );

    if error_set(&err) {
        let _ = emsg(err.msg_cstr());
        api_clear_error(&mut err);
        return;
    }

    ex_detach(&mut crate::ex_docmd::Exarg::new());
    if stop_server {
        g_st().exiting = true;
        getout(0);
    }
}

fn ex_mode(eap: &mut Exarg) {
    if crate::memory::cstr_is_empty(eap.arg) {
        g_st().must_redraw = UPD_CLEAR;
        ex_redraw(eap);
    } else {
        let _ = emsg(gettext(E_SCREENMODE));
    }
}

fn ex_resize(eap: &mut Exarg) {
    unsafe {
        let mut wp = curwin_ptr();

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

        let mut n = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.arg)) as i32;
        if (g_st().cmdmod.cmod_split & WSP_VERT) != 0 {
            if crate::memory::cstr_first_byte(eap.arg) == b'-'
                || crate::memory::cstr_first_byte(eap.arg) == b'+'
            {
                n += (*wp).w_width;
            } else if n == 0 && crate::memory::cstr_is_empty(eap.arg) {
                n = g_st().columns_global;
            }
            win_setwidth_win(n, &mut *wp, true);
        } else {
            if crate::memory::cstr_first_byte(eap.arg) == b'-'
                || crate::memory::cstr_first_byte(eap.arg) == b'+'
            {
                n += (*wp).w_height;
            } else if n == 0 && crate::memory::cstr_is_empty(eap.arg) {
                n = g_st().rows_global - 1;
            }
            win_setheight_win(n, &mut *wp, true);
        }
    }
}

fn ex_find(eap: &mut Exarg) {
    if !check_can_set_curbuf_forceit(eap.forceit) {
        return;
    }

    let mut fname: Option<std::ffi::CString>;
    if get_findfunc().cb_type() != K_CALLBACK_NONE_CONST {
        fname = findfunc_find_file(
            eap.arg,
            crate::memory::cstr_len(eap.arg),
            if eap.addr_count > 0 { eap.line2 } else { 1 },
        );
    } else {
        let mut file_to_find: Option<std::ffi::CString> = None;
        let mut search_ctx: Option<FfSearchCtx> = None;
        fname = find_file_in_path(
            eap.arg,
            crate::memory::cstr_len(eap.arg),
            FNAME_MESS,
            i32::from(true),
            curbuf_mut().b_ffname,
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
                    i32::from(false),
                    curbuf_mut().b_ffname,
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

fn ex_edit(eap: &mut Exarg) {
    set_browse_edit_arg(eap);
    let ffname = if eap.cmdidx == Cmdidx::CmdEnew as i32 {
        ptr::null_mut()
    } else {
        eap.arg
    };

    if eap.cmdidx != Cmdidx::CmdBadd as i32
        && eap.cmdidx != Cmdidx::CmdBalt as i32
        && is_other_file(0, ffname)
        && !check_can_set_curbuf_forceit(eap.forceit)
    {
        return;
    }

    if bt_prompt(Some(crate::globals::curbuf_mut()))
        && eap.cmdidx == Cmdidx::CmdEdit as i32
        && crate::memory::cstr_is_empty(eap.arg)
    {
        let _ = emsg(c"cannot :edit a prompt buffer");
        return;
    }

    do_exedit(eap, ptr::null_mut());
}

pub fn do_exedit(eap: &mut Exarg, old_curwin: *mut WinInfoInfo) {
    if (eap.cmdidx == Cmdidx::CmdNew as i32
        || eap.cmdidx == Cmdidx::CmdTabnew as i32
        || eap.cmdidx == Cmdidx::CmdTabedit as i32
        || eap.cmdidx == Cmdidx::CmdVnew as i32)
        && crate::memory::cstr_is_empty(eap.arg)
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
                curwin_ptr()
            } else {
                ptr::null_mut()
            },
        );
    } else if (eap.cmdidx != Cmdidx::CmdSplit as i32 && eap.cmdidx != Cmdidx::CmdVsplit as i32)
        || !crate::memory::cstr_is_empty(eap.arg)
    {
        if !crate::memory::cstr_is_empty(eap.arg) && text_or_buf_locked() {
            return;
        }
        let n = g_st().readonlymode;
        if eap.cmdidx == Cmdidx::CmdView as i32 || eap.cmdidx == Cmdidx::CmdSview as i32 {
            g_st().readonlymode = true;
        } else if eap.cmdidx == Cmdidx::CmdEnew as i32 {
            g_st().readonlymode = false;
        }
        if eap.cmdidx != Cmdidx::CmdBalt as i32 && eap.cmdidx != Cmdidx::CmdBadd as i32 {
            setpcmark();
        }
        let ecmd_flags = (if buf_hide(curbuf_mut()) { ECMD_HIDE } else { 0 })
            + (if eap.forceit != 0 { ECMD_FORCEIT } else { 0 })
            + (if !old_curwin.is_null() {
                ECMD_OLDBUF
            } else {
                0
            })
            + (if eap.cmdidx == Cmdidx::CmdBadd as i32 {
                ECMD_ADDBUF
            } else {
                0
            })
            + (if eap.cmdidx == Cmdidx::CmdBalt as i32 {
                ECMD_ALTBUF
            } else {
                0
            });
        if !do_ecmd(
            0,
            if eap.cmdidx == Cmdidx::CmdEnew as i32 {
                ptr::null_mut()
            } else {
                eap.arg
            },
            ptr::null_mut(),
            Some(&*eap),
            eap.do_ecmd_lnum,
            ecmd_flags,
            if old_curwin.is_null() {
                curwin_ptr()
            } else {
                ptr::null_mut()
            },
        ) {
            if !old_curwin.is_null() {
                let need_hide = curbuf_is_changed_fn() && curbuf_mut().b_nwindows <= 1;
                if !need_hide || buf_hide(curbuf_mut()) {
                    let mut cs: ExCleanup = ExCleanup::new();
                    enter_cleanup(&mut cs);
                    win_close(curwin_ptr(), !need_hide && !buf_hide(curbuf_mut()), false);
                    leave_cleanup(&mut cs);
                }
            }
        } else if g_st().readonlymode && curbuf_mut().b_nwindows == 1 {
            curbuf_mut().b_p_ro = i32::from(true);
        }
        g_st().readonlymode = n;
    } else {
        if !eap.do_ecmd_cmd.is_null() {
            do_cmdline_cmd(eap.do_ecmd_cmd);
        }
        let n = curwin_mut().w_arg_idx_invalid;
        check_arg_idx(curwin_mut());
        if n != curwin_mut().w_arg_idx_invalid {
            maketitle();
        }
    }

    if !old_curwin.is_null()
        && !crate::memory::cstr_is_empty(eap.arg)
        && curwin_ptr() != old_curwin
        && win_valid(old_curwin)
        && unsafe { (*old_curwin).w_buffer } != g_st().curbuf
        && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0
    {
        unsafe {
            (*old_curwin).w_alt_fnum = curbuf_mut().b_fnum;
        }
    }
}

fn ex_nogui(eap: &mut Exarg) {
    eap.errmsg = Some(gettext(c"E25: Nvim does not have a built-in GUI"));
}

fn ex_popup(eap: &mut Exarg) {
    pum_make_popup(eap.arg, eap.forceit);
}

fn ex_swapname(_eap: &mut Exarg) {
    unsafe {
        let mfp = curbuf_mut().b_ml.mfp_ptr();
        if mfp.is_null() {
            let _ = msg(gettext(c"No swap file"), 0);
        } else if let Some(ref mf_fname) = (*mfp).mf_fname {
            let _ = msg(mf_fname.as_ptr(), 0);
        } else {
            let _ = msg(gettext(c"No swap file"), 0);
        }
    }
}

fn ex_syncbind(_eap: &mut Exarg) {
    let old_linenr = curwin_mut().w_cursor.lnum;
    setpcmark();

    let vtopline = if curwin_mut().w_onebuf_opt.wo_scb != 0 {
        let mut vt = get_vtopline(curwin_mut());
        for wp in for_all_windows_ptr() {
            if wp.w_onebuf_opt.wo_scb != 0 && !wp.w_buffer.is_null() {
                let y = plines_m_win_fill(wp, 1, wp.buf_mut().b_ml.ml_line_count)
                    - (get_scrolloff_value(curwin_mut()) as i32);
                vt = vt.min(y);
            }
        }
        vt.max(1)
    } else {
        1
    };

    for wp in for_all_windows_ptr() {
        if wp.w_onebuf_opt.wo_scb != 0 {
            let y = vtopline - get_vtopline(&mut *wp);
            if y > 0 {
                scrollup(&mut *wp, y, true);
            } else {
                scrolldown(&mut *wp, -y, i32::from(true));
            }
            wp.w_scbind_pos = vtopline;
            redraw_later(Some(&mut *wp), UPD_VALID);
            cursor_correct(&mut *wp);
            wp.w_redr_status = true;
        }
    }

    if curwin_mut().w_onebuf_opt.wo_scb != 0 {
        g_st().did_syncbind = true;
        checkpcmark();
        if old_linenr != curwin_mut().w_cursor.lnum {
            let ctrl_o = [CTRL_O, 0];
            let _ = ins_typebuf(&ctrl_o, REMAP_NONE, 0, true, false);
        }
    }
}

fn ex_read(eap: &mut Exarg) {
    if eap.usefilter != 0 {
        do_bang(1, eap, false, false, true);
        return;
    }

    if !u_save(eap.line2, eap.line2 + 1) {
        return;
    }

    let i = if crate::memory::cstr_is_empty(eap.arg) {
        if !check_fname() {
            return;
        }
        readfile(
            curbuf_mut().b_ffname,
            curbuf_mut().b_fname,
            eap.line2,
            0,
            MAXLNUM,
            Some(&*eap),
            0,
            false,
        )
    } else {
        if crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_ALTREAD) {
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
            let _ = semsg!(gettext(E_NOTOPEN), eap.arg);
        }
    } else {
        redraw_curbuf_later(UPD_VALID);
    }
}

pub fn free_cd_dir() {
    docmd_st().prev_dir = None;
    g_st().globaldir = None;
}

fn get_prevdir(scope: CdScope) -> *mut Option<std::ffi::CString> {
    unsafe {
        match scope {
            K_CD_SCOPE_TABPAGE => &raw mut (*g_st().curtab).tp_prevdir,
            K_CD_SCOPE_BUFFER => &raw mut curbuf_mut().b_prevdir,
            K_CD_SCOPE_WINDOW => &raw mut curwin_mut().w_prevdir,
            _ => &raw mut docmd_st().prev_dir,
        }
    }
}

fn post_chdir(scope: CdScope, trigger_dirchanged: bool) {
    if scope == K_CD_SCOPE_BUFFER {
        curbuf_mut().b_localdir = None;
    }
    if scope >= K_CD_SCOPE_WINDOW {
        curwin_mut().w_localdir = None;
    }
    if scope >= K_CD_SCOPE_TABPAGE {
        curtab_mut().tp_localdir = None;
    }

    if scope < K_CD_SCOPE_GLOBAL {
        let pdir = unsafe { &*get_prevdir(scope) };
        if g_st().globaldir.is_none() && pdir.is_some() {
            g_st().globaldir = pdir.clone();
        }
    }

    let mut cwd = [0u8; MAXPATHL];
    if !os_dirname(cwd.as_mut_ptr(), MAXPATHL) {
        return;
    }
    let cwd_cstr = crate::memory::cstr_to_cstring(&cwd);
    match scope {
        K_CD_SCOPE_GLOBAL => {
            g_st().globaldir = if curbuf_mut().b_localdir.is_some() {
                Some(cwd_cstr)
            } else {
                None
            };
        }
        K_CD_SCOPE_TABPAGE => {
            curtab_mut().tp_localdir = Some(cwd_cstr);
        }
        K_CD_SCOPE_BUFFER => {
            curbuf_mut().b_localdir = Some(cwd_cstr);
        }
        K_CD_SCOPE_WINDOW => {
            curwin_mut().w_localdir = Some(cwd_cstr);
        }
        _ => std::process::abort(),
    }

    g_st().last_chdir_reason = None;
    shorten_fnames(i32::from(!crate::strings::vim_haschr(
        opt_vars().p_cpo,
        K_CPO_NOSYMLINKS,
    )));

    if trigger_dirchanged {
        do_autocmd_dirchanged(&cwd, scope, K_CD_CAUSE_MANUAL, false);
    }
}

pub fn do_chdir(new_dir: impl crate::strings::IntoCCharPtr, scope: CdScope) -> bool {
    let mut new_dir = new_dir.into_c_char_ptr().cast_mut();
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

        let pdir = if os_dirname(g_st().name_buff.as_mut_ptr(), MAXPATHL) {
            Some(crate::memory::cstr_to_cstring(g_st().name_buff.as_ptr()))
        } else {
            None
        };

        if crate::memory::cstr_is_empty(new_dir) && opt_vars().p_cdh != 0 {
            expand_env(c"$HOME", g_st().name_buff.as_mut_ptr(), MAXPATHL as i32);
            new_dir = g_st().name_buff.as_mut_ptr();
        }

        let new_dir_owned = to_slash_save(new_dir).unwrap_or_default();
        new_dir = new_dir_owned.as_ptr().cast_mut();

        let bcd_active = scope != K_CD_SCOPE_BUFFER && curbuf_mut().b_localdir.is_some();

        let dir_differs = pdir
            .as_ref()
            .is_none_or(|p| !path_equal(p.as_ptr(), new_dir, K_PATH_CMP_LITERAL));
        if dir_differs {
            if !bcd_active {
                do_autocmd_dirchanged(new_dir, scope, K_CD_CAUSE_MANUAL, true);
            }
            if vim_chdir(new_dir) != 0 {
                let _ = emsg(gettext(E_FAILED));
                return false;
            }
        }
        drop(new_dir_owned);

        let pp = get_prevdir(scope);
        *pp = pdir;

        post_chdir(scope, dir_differs && !bcd_active);
        if bcd_active {
            os_chdir(curbuf_mut().b_localdir.as_ref().unwrap().as_ptr());
            shorten_fnames(i32::from(true));
        }
        ctx_did_chdir();

        true
    }
}

fn chdir_clear(scope: CdScope) {
    unsafe {
        let localdir = if scope == K_CD_SCOPE_BUFFER {
            &raw mut curbuf_mut().b_localdir
        } else if scope == K_CD_SCOPE_WINDOW {
            &raw mut curwin_mut().w_localdir
        } else {
            &raw mut (*g_st().curtab).tp_localdir
        };
        if (*localdir).is_none() {
            return;
        }
        *localdir = None;
        update_cwd(K_CD_CAUSE_MANUAL);
        ctx_did_chdir();
    }
}

pub fn ex_cd(eap: &mut Exarg) {
    let new_dir = eap.arg;

    let scope = if eap.cmdidx == Cmdidx::CmdTcd as i32 || eap.cmdidx == Cmdidx::CmdTchdir as i32 {
        K_CD_SCOPE_TABPAGE
    } else if eap.cmdidx == Cmdidx::CmdBcd as i32 || eap.cmdidx == Cmdidx::CmdBchdir as i32 {
        K_CD_SCOPE_BUFFER
    } else if eap.cmdidx == Cmdidx::CmdLcd as i32 || eap.cmdidx == Cmdidx::CmdLchdir as i32 {
        K_CD_SCOPE_WINDOW
    } else {
        K_CD_SCOPE_GLOBAL
    };

    if eap.forceit != 0 && crate::memory::cstr_is_empty(new_dir) && scope != K_CD_SCOPE_GLOBAL {
        chdir_clear(scope);
        if g_st().key_typed || opt_vars().p_verbose >= 5 {
            ex_pwd(eap);
        }
        return;
    }

    if crate::memory::cstr_is_empty(new_dir) && opt_vars().p_cdh == 0 {
        ex_pwd(&mut crate::ex_docmd::Exarg::new());
        return;
    }

    if do_chdir(new_dir, scope) && (g_st().key_typed || opt_vars().p_verbose >= 5) {
        ex_pwd(eap);
    }
}

fn ex_pwd(_eap: &mut Exarg) {
    if os_dirname(g_st().name_buff.as_mut_ptr(), MAXPATHL) {
        #[cfg(windows)]
        slash_adjust(g_st().name_buff.as_mut_ptr().cast());
        if opt_vars().p_verbose > 0 {
            let context = if let Some(reason) = g_st().last_chdir_reason {
                reason.as_ptr()
            } else if curbuf_mut().b_localdir.is_some() {
                c"buffer".as_ptr()
            } else if curwin_mut().w_localdir.is_some() {
                c"window".as_ptr()
            } else if curtab_mut().tp_localdir.is_some() {
                c"tabpage".as_ptr()
            } else {
                c"global".as_ptr()
            };
            let _ = smsg!(0, c"[%s] %s", context, g_st().name_buff.as_ptr(),);
        } else {
            let _ = msg(g_st().name_buff.as_ptr(), 0);
        }
    } else {
        let _ = emsg(gettext(c"E187: Unknown"));
    }
}

fn ex_equal(eap: &mut Exarg) {
    if !crate::memory::cstr_is_empty(eap.arg) && crate::memory::cstr_first_byte(eap.arg) != b'|' {
        ex_lua(eap);
    } else {
        eap.nextcmd = find_nextcmd(eap.arg);
        let _ = smsg!(0, c"%lld", i64::from(eap.line2));
    }
}

fn ex_sleep(eap: &mut Exarg) {
    if cursor_valid(curwin_mut()) != 0 {
        setcursor_mayforce(curwin_mut(), true);
    }

    let mut len = i64::from(eap.line2);
    match crate::memory::cstr_first_byte(eap.arg) {
        b'm' => {}
        0 => len *= 1000,
        _ => {
            let _ = semsg!(gettext(E_INVARG2), eap.arg);
            return;
        }
    }

    do_sleep(len, eap.forceit != 0);
}

pub fn do_sleep(msec: i64, hide_cursor: bool) {
    if hide_cursor {
        ui_busy_start();
    }

    ui_flush();
    let main_loop_ptr = (&raw mut g_st().main_loop).cast();
    let events = loop_queues(unsafe { &mut *main_loop_ptr }).events;
    let timeout_ms = msec;
    let start = if timeout_ms >= 0 { os_hrtime() } else { 0 };
    let mut remaining = timeout_ms;
    let mut expired = false;
    while !g_st().got_int && !expired {
        if !events.is_null() && !multiqueue_empty(unsafe { &*events }) {
            multiqueue_process_events(unsafe { &mut *events });
        } else {
            loop_poll_events(unsafe { &mut *main_loop_ptr }, remaining);
        }
        if timeout_ms >= 0 {
            remaining = timeout_ms - ((os_hrtime() - start) / 1_000_000) as i64;
            expired = remaining <= 0;
        }
    }

    if g_st().got_int {
        let _ = vpeekc();
    }

    if hide_cursor {
        ui_busy_stop();
    }
}

fn ex_winsize(eap: &mut Exarg) {
    let mut arg = eap.arg;
    if !ascii_isdigit(unsafe { *arg }) {
        let _ = semsg!(gettext(E_INVARG2), arg);
        return;
    }
    let w = getdigits_int(&mut arg, false, 10);
    arg = skipwhite(arg);
    let p = arg;
    let h = getdigits_int(&mut arg, false, 10);
    if !crate::memory::cstr_is_empty(p) && crate::memory::cstr_is_empty(arg) {
        screen_resize(w, h);
    } else {
        let _ = emsg(gettext(c"E465: :winsize requires two number arguments"));
    }
}

fn ex_wincmd(eap: &mut Exarg) {
    let mut xchar: i32 = 0;
    let mut p: RawCCharPtrMut;

    if crate::memory::cstr_first_byte(eap.arg) == b'g'
        || crate::memory::cstr_first_byte(eap.arg) == CTRL_G
    {
        if crate::memory::cstr_byte_at(eap.arg, 1) == 0 {
            let _ = emsg(gettext(E_INVARG));
            return;
        }
        xchar = i32::from(crate::memory::cstr_byte_at(eap.arg, 1));
        p = eap.arg.wrapping_add(2);
    } else {
        p = eap.arg.wrapping_add(1);
    }

    eap.nextcmd = check_nextcmd(p);
    p = skipwhite(p);
    if !crate::memory::cstr_is_empty(p)
        && crate::memory::cstr_first_byte(p) != b'"'
        && eap.nextcmd.is_null()
    {
        let _ = emsg(gettext(E_INVARG));
    } else if eap.skip == 0 {
        g_st().postponed_split_flags = g_st().cmdmod.cmod_split;
        g_st().postponed_split_tab = g_st().cmdmod.cmod_tab;
        do_window(
            i32::from(crate::memory::cstr_first_byte(eap.arg)),
            if eap.addr_count > 0 { eap.line2 } else { 0 },
            xchar,
        );
        g_st().postponed_split_flags = 0;
        g_st().postponed_split_tab = 0;
    }
}

fn ex_operators(eap: &mut Exarg) {
    let mut oa = Oparg::new();
    oa.regname = eap.regname;
    oa.start.lnum = eap.line1;
    oa.end.lnum = eap.line2;
    oa.line_count = eap.line2 - eap.line1 + 1;
    oa.motion_type = MT_LINE_WISE;
    g_st().virtual_op = K_FALSE;
    if eap.cmdidx != Cmdidx::CmdYank as i32 {
        setpcmark();
        curwin_mut().w_cursor.lnum = eap.line1;
        beginline(BL_SOL | BL_FIX);
    }

    if g_st().visual_global.active {
        end_visual_mode();
    }

    if eap.cmdidx == Cmdidx::CmdDelete as i32 {
        oa.op_type = OP_DELETE;
        op_delete(&mut oa);
    } else if eap.cmdidx == Cmdidx::CmdYank as i32 {
        oa.op_type = OP_YANK;
        op_yank(&mut oa, true);
    } else {
        if (eap.cmdidx == Cmdidx::CmdRshift as i32) ^ (curwin_mut().w_onebuf_opt.wo_rl != 0) {
            oa.op_type = OP_RSHIFT;
        } else {
            oa.op_type = OP_LSHIFT;
        }
        op_shift(&mut oa, false, eap.amount);
    }
    g_st().virtual_op = K_NONE;
    ex_may_print(eap);
}

fn ex_put(eap: &mut Exarg) {
    if eap.line2 == 0 {
        eap.line2 = 1;
        eap.forceit = i32::from(true);
    }
    curwin_mut().w_cursor.lnum = eap.line2;
    check_cursor_col(curwin_mut());
    do_put(
        eap.regname,
        ptr::null_mut(),
        if eap.forceit != 0 { BACKWARD } else { FORWARD },
        1,
        PUT_LINE | PUT_CURSLINE,
    );
}

fn ex_iput(eap: &mut Exarg) {
    if eap.line2 == 0 {
        eap.line2 = 1;
        eap.forceit = i32::from(true);
    }
    curwin_mut().w_cursor.lnum = eap.line2;
    check_cursor_col(curwin_mut());
    do_put(
        eap.regname,
        ptr::null_mut(),
        if eap.forceit != 0 { BACKWARD } else { FORWARD },
        1,
        PUT_LINE | PUT_CURSLINE | PUT_FIXINDENT,
    );
}

fn ex_copymove(eap: &mut Exarg) {
    let mut errormsg: Option<&'static std::ffi::CStr> = None;
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
        if let Some(msg) = errormsg {
            let _ = emsg(msg);
        }
        eap.nextcmd = ptr::null_mut();
        return;
    }
    get_flags(eap);

    if n == MAXLNUM || n < 0 || n > curbuf_mut().b_ml.ml_line_count {
        let _ = emsg(gettext(E_INVRANGE));
        return;
    }

    if eap.cmdidx == Cmdidx::CmdMove as i32 {
        if !do_move(eap.line1, eap.line2, n) {
            return;
        }
    } else {
        ex_copy(eap.line1, eap.line2, n);
    }
    u_clearline(curbuf_mut());
    beginline(BL_SOL | BL_FIX);
    ex_may_print(eap);
}

pub fn ex_may_print(eap: &mut Exarg) {
    if eap.flags != 0 {
        print_line(
            curwin_mut().w_cursor.lnum,
            (eap.flags & EXFLAG_NR) != 0,
            (eap.flags & EXFLAG_LIST) != 0,
            true,
        );
    }
}

fn ex_join(eap: &mut Exarg) {
    curwin_mut().w_cursor.lnum = eap.line1;
    if eap.line1 == eap.line2 {
        if eap.addr_count >= 2 {
            return;
        }
        if eap.line2 == curbuf_mut().b_ml.ml_line_count {
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

fn ex_at(eap: &mut Exarg) {
    let prev_len = g_st().typebuf.tb_len;

    curwin_mut().w_cursor.lnum = eap.line2;
    check_cursor_col(curwin_mut());

    let mut c = i32::from(crate::memory::cstr_first_byte(eap.arg));
    if c == 0 {
        c = b'@' as i32;
    }

    if !do_execreg(
        c,
        i32::from(true),
        i32::from(crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_EXECBUF)),
        i32::from(true),
    ) {
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

fn ex_bang(eap: &mut Exarg) {
    do_bang(eap.addr_count, eap, eap.forceit != 0, true, true);
}

fn ex_undo(eap: &mut Exarg) {
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
        if step >= curbuf_mut().b_u_seq_cur {
            let _ = emsg(gettext(E_UNDOBANG_CANNOT_REDO_OR_MOVE_BRANCH));
            return;
        }
        let mut uhp = curbuf_mut().b_u_curhead.or(curbuf_mut().b_u_newhead);
        let mut count: i32 = 0;
        while let Some(uh) = uhp
            && uhp_ref(curbuf_mut(), uh).uh_seq > step
        {
            uhp = uhp_ref(curbuf_mut(), uh).uh_next;
            count += 1;
        }
        if step != 0 && uhp.is_none_or(|uh| uhp_ref(curbuf_mut(), uh).uh_seq < step) {
            let _ = emsg(gettext(E_UNDOBANG_CANNOT_REDO_OR_MOVE_BRANCH));
            return;
        }
        u_undo_and_forget(count, true);
    } else {
        undo_time(step, false, false, true);
    }
}

fn ex_wundo(eap: &mut Exarg) {
    let mut hash = [0u8; UNDO_HASH_SIZE];
    u_compute_hash(curbuf_mut(), hash.as_mut_ptr());
    u_write_undo(eap.arg, eap.forceit != 0, curbuf_mut(), hash.as_mut_ptr());
}

fn ex_rundo(eap: &mut Exarg) {
    let mut hash = [0u8; UNDO_HASH_SIZE];
    u_compute_hash(curbuf_mut(), hash.as_mut_ptr());
    u_read_undo(eap.arg, hash.as_ptr(), ptr::null());
}

fn ex_redo(_eap: &mut Exarg) {
    u_redo(1);
}

fn ex_later(eap: &mut Exarg) {
    let mut count: i32 = 0;
    let mut sec = false;
    let mut file = false;
    let mut p = eap.arg;

    if crate::memory::cstr_is_empty(p) {
        count = 1;
    } else if ascii_isdigit(unsafe { *p }) {
        count = getdigits_int(&mut p, false, 0);
        match crate::memory::cstr_first_byte(p) {
            b's' => {
                p = p.wrapping_add(1);
                sec = true;
            }
            b'm' => {
                p = p.wrapping_add(1);
                sec = true;
                count *= 60;
            }
            b'h' => {
                p = p.wrapping_add(1);
                sec = true;
                count *= 60 * 60;
            }
            b'd' => {
                p = p.wrapping_add(1);
                sec = true;
                count *= 24 * 60 * 60;
            }
            b'f' => {
                p = p.wrapping_add(1);
                file = true;
            }
            _ => {}
        }
    }

    if !crate::memory::cstr_is_empty(p) {
        let _ = semsg!(gettext(E_INVARG2), eap.arg);
    } else {
        undo_time(
            if eap.cmdidx == Cmdidx::CmdEarlier as i32 {
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

fn ex_redir(eap: &mut Exarg) {
    let mut arg = eap.arg;

    if crate::memory::cstr_as_bytes(eap.arg).eq_ignore_ascii_case(b"END") {
        close_redir();
    } else if crate::memory::cstr_first_byte(arg) == b'>' {
        arg = arg.wrapping_add(1);
        let mode = if crate::memory::cstr_first_byte(arg) == b'>' {
            arg = arg.wrapping_add(1);
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
    } else if crate::memory::cstr_first_byte(arg) == b'@' {
        close_redir();
        arg = arg.wrapping_add(1);
        if valid_yank_reg(i32::from(crate::memory::cstr_first_byte(arg)), true)
            && crate::memory::cstr_first_byte(arg) != b'_'
        {
            g_st().redir_reg = i32::from(crate::memory::cstr_first_byte(arg));
            arg = arg.wrapping_add(1);
            if crate::memory::cstr_first_byte(arg) == b'>'
                && crate::memory::cstr_byte_at(arg, 1) == b'>'
            {
                arg = arg.wrapping_add(2);
            } else {
                if crate::memory::cstr_first_byte(arg) == b'>' {
                    arg = arg.wrapping_add(1);
                }
                if crate::memory::cstr_is_empty(arg) && !ascii_isupper(g_st().redir_reg) {
                    write_reg_contents(g_st().redir_reg, c"", 0, i32::from(false));
                }
            }
        }
        if !crate::memory::cstr_is_empty(arg) {
            g_st().redir_reg = 0;
            let _ = semsg!(gettext(E_INVARG2), eap.arg);
        }
    } else if crate::memory::cstr_first_byte(arg) == b'='
        && crate::memory::cstr_byte_at(arg, 1) == b'>'
    {
        close_redir();
        arg = arg.wrapping_add(2);
        let append = if crate::memory::cstr_first_byte(arg) == b'>' {
            arg = arg.wrapping_add(1);
            true
        } else {
            false
        };

        if var_redir_start(skipwhite(arg), append) {
            g_st().redir_vname = true;
        }
    } else {
        let _ = semsg!(gettext(E_INVARG2), eap.arg);
    }

    if has_redir_fd() || g_st().redir_reg != 0 || g_st().redir_vname {
        g_st().redir_off = false;
    }
}

fn ex_redraw(eap: &mut Exarg) {
    if g_st().cmdpreview {
        return;
    }
    let r = g_st().redrawing_disabled;
    let p = opt_vars().p_lz;

    g_st().redrawing_disabled = 0;
    opt_vars().p_lz = 0;
    validate_cursor(curwin_mut());
    update_topline(curwin_mut());
    if eap.forceit != 0 {
        redraw_all_later(UPD_NOT_VALID);
        g_st().redraw_cmdline = true;
    } else if g_st().visual_global.active {
        redraw_curbuf_later(UPD_INVERTED);
    }
    update_screen();
    if g_st().need_maketitle {
        maketitle();
    }
    g_st().redrawing_disabled = r;
    opt_vars().p_lz = p;

    g_st().msg_didout = false;
    g_st().msg_col = 0;
    g_st().need_wait_return = false;

    ui_flush();
}

fn ex_redrawstatus(eap: &mut Exarg) {
    if g_st().cmdpreview {
        return;
    }
    let r = g_st().redrawing_disabled;
    let p = opt_vars().p_lz;

    if eap.forceit != 0 {
        status_redraw_all();
    } else {
        status_redraw_curbuf();
    }

    g_st().redrawing_disabled = 0;
    opt_vars().p_lz = 0;
    if (g_st().state_global & MODE_CMDLINE) != 0 {
        redraw_statuslines();
    } else {
        if g_st().visual_global.active {
            redraw_curbuf_later(UPD_INVERTED);
        }
        update_screen();
    }
    g_st().redrawing_disabled = r;
    opt_vars().p_lz = p;
    ui_flush();
}

fn ex_redrawtabline(_eap: &mut Exarg) {
    let r = g_st().redrawing_disabled;
    let p = opt_vars().p_lz;

    g_st().redrawing_disabled = 0;
    opt_vars().p_lz = 0;

    draw_tabline();

    g_st().redrawing_disabled = r;
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

pub fn vim_mkdir_emsg(name: impl crate::strings::IntoCCharPtr, prot: i32) -> bool {
    let name = name.into_c_char_ptr();
    {
        let ret = os_mkdir(name, prot);
        if ret != 0 {
            let _ = semsg!(gettext(E_MKDIR), name, unsafe { uv_strerror(ret) });
            return false;
        }
        true
    }
}

pub fn open_exfile(
    fname: impl crate::strings::IntoCCharPtr,
    forceit: i32,
    mode: &std::ffi::CStr,
) -> *mut FILE {
    let fname = fname.into_c_char_ptr();
    #[cfg(unix)]
    if os_isdir(fname) {
        let _ = semsg!(gettext(E_ISADIR2), fname);
        return ptr::null_mut();
    }

    if forceit == 0 && mode.to_bytes().first() != Some(&b'a') && os_path_exists(fname) {
        let _ = semsg!(gettext(c"E189: \"%s\" exists (add ! to override)"), fname,);
        return ptr::null_mut();
    }

    let fd = os_fopen(fname, mode);
    if fd.is_null() {
        let _ = semsg!(gettext(c"E190: Cannot open \"%s\" for writing"), fname,);
    }
    fd.cast()
}

fn ex_mark(eap: &mut Exarg) {
    if crate::memory::cstr_is_empty(eap.arg) {
        let _ = emsg(gettext(E_ARGREQ));
        return;
    }
    if crate::memory::cstr_byte_at(eap.arg, 1) != 0 {
        let _ = semsg!(gettext(E_TRAILING_ARG), eap.arg);
        return;
    }

    let pos = curwin_mut().w_cursor;
    curwin_mut().w_cursor.lnum = eap.line2;
    beginline(BL_WHITE | BL_FIX);
    if !setmark(i32::from(crate::memory::cstr_first_byte(eap.arg))) {
        let _ = emsg(gettext(
            c"E191: Argument must be a letter or forward/backward quote",
        ));
    }
    curwin_mut().w_cursor = pos;
}

pub fn update_topline_cursor() {
    check_cursor(curwin_mut());
    update_topline(curwin_mut());
    if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
        validate_cursor(curwin_mut());
    }
    update_curswant();
}

pub fn save_current_state(sst: &mut SaveState) {
    sst.save_msg_scroll = g_st().msg_scroll;
    sst.save_restart_edit = g_st().restart_edit;
    sst.save_msg_didout = g_st().msg_didout;
    sst.save_state = g_st().state_global;
    sst.save_finish_op = g_st().finish_op;
    sst.save_opcount = g_st().opcount;
    sst.save_reg_executing = g_st().reg_executing;
    sst.save_pending_end_reg_executing = g_st().pending_end_reg_executing;

    g_st().msg_scroll = 0;
    g_st().restart_edit = 0;

    save_typeahead(unsafe { &mut *(&raw mut sst.tabuf).cast() });
}

pub fn restore_current_state(sst: &mut SaveState) {
    restore_typeahead(unsafe { &mut *(&raw mut sst.tabuf).cast() });

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

    g_st().state_global = sst.save_state;
    ui_cursor_shape();
}

pub fn expr_map_locked() -> bool {
    g_st().expr_map_lock > 0 && (curbuf_mut().b_flags & BF_DUMMY) == 0
}

fn ex_normal(eap: &mut Exarg) {
    if !curbuf_mut().terminal.is_null() && (g_st().state_global & MODE_TERMINAL) != 0 {
        let _ = emsg(c"Can't re-enter normal mode from terminal mode");
        return;
    }

    if expr_map_locked() {
        let _ = emsg(gettext(E_SECURE));
        return;
    }

    if OptInt::from(g_st().ex_normal_busy) >= opt_vars().p_mmd {
        let _ = emsg(gettext(c"E192: Recursive use of :normal too deep"));
        return;
    }

    let mut len: usize = 0;
    let mut p = eap.arg;
    while !crate::memory::cstr_is_empty(p) {
        let mut l = utfc_ptr2len(p) - 1;
        while l > 0 {
            p = p.wrapping_add(1);
            if (crate::memory::cstr_first_byte(p)) == K_SPECIAL {
                len += 2;
            }
            l -= 1;
        }
        p = p.wrapping_add(1);
    }
    let mut arg_vec: Option<Vec<u8>> = None;
    if len > 0 {
        let mut v = vec![0u8; crate::memory::cstr_len(eap.arg) + len + 1];
        let arg = v.as_mut_ptr();
        len = 0;
        p = eap.arg;
        while !crate::memory::cstr_is_empty(p) {
            unsafe {
                *arg.wrapping_add(len) = *p as u8;
            }
            len += 1;
            let mut l = utfc_ptr2len(p) - 1;
            while l > 0 {
                p = p.wrapping_add(1);
                unsafe {
                    *arg.wrapping_add(len) = *p as u8;
                }
                len += 1;
                if (crate::memory::cstr_first_byte(p)) == K_SPECIAL {
                    unsafe {
                        *arg.wrapping_add(len) = KS_SPECIAL;
                    }
                    len += 1;
                    unsafe {
                        *arg.wrapping_add(len) = KE_FILLER;
                    }
                    len += 1;
                }
                l -= 1;
            }
            unsafe {
                *arg.wrapping_add(len) = 0;
            }
            p = p.wrapping_add(1);
        }
        arg_vec = Some(v);
    }

    g_st().ex_normal_busy += 1;
    let mut save_state = SaveState::new();
    save_current_state(&mut save_state);

    loop {
        if eap.addr_count != 0 {
            curwin_mut().w_cursor.lnum = eap.line1;
            eap.line1 += 1;
            curwin_mut().w_cursor.col = 0;
            check_cursor_moved(curwin_mut());
        }

        exec_normal_cmd(
            arg_vec
                .as_ref()
                .map_or(eap.arg, |v| v.as_ptr().cast_mut().cast()),
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

fn ex_startinsert(eap: &mut Exarg) {
    if curbuf_mut().b_p_ma == 0 && curbuf_mut().terminal.is_null() {
        let _ = emsg(gettext(E_MODIFIABLE));
        return;
    }
    if eap.forceit != 0 {
        if curwin_mut().w_cursor.lnum == 0 {
            curwin_mut().w_cursor.lnum = 1;
        }
        set_cursor_for_append_to_line();
    }

    if (g_st().state_global & MODE_INSERT) != 0 {
        return;
    }

    if eap.cmdidx == Cmdidx::CmdStartinsert as i32 {
        g_st().restart_edit = b'a' as i32;
    } else if eap.cmdidx == Cmdidx::CmdStartreplace as i32 {
        g_st().restart_edit = b'R' as i32;
    } else {
        g_st().restart_edit = b'V' as i32;
    }

    if eap.forceit == 0 {
        if eap.cmdidx == Cmdidx::CmdStartinsert as i32 {
            g_st().restart_edit = b'i' as i32;
        }
        curwin_mut().w_curswant = 0;
    }

    if g_st().visual_global.active {
        showmode();
    }
}

fn ex_stopinsert(_eap: &mut Exarg) {
    g_st().restart_edit = 0;
    g_st().ins_global.stop_insert_mode = true;
    clearmode();
}

pub fn exec_normal_cmd(cmd: impl crate::strings::IntoCCharPtr, remap: i32, silent: bool) {
    let cmd = cmd.into_c_char_ptr();
    let _ = ins_typebuf(cmd, remap, 0, true, silent);
    exec_normal(false, false);
}

pub fn exec_normal(was_typed: bool, use_vpeekc: bool) {
    let mut oa = Oparg::new();
    g_st().finish_op = false;
    while (!stuff_empty()
        || ((was_typed || typebuf_typed() == 0) && g_st().typebuf.tb_len > 0)
        || (use_vpeekc && {
            let c = vpeekc();
            c != 0 && c != i32::from(CTRL_C_CONST)
        }))
        && !g_st().got_int
    {
        update_topline_cursor();
        normal_cmd(&mut oa, true);
    }
}

fn ex_checkpath(eap: &mut Exarg) {
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

fn ex_psearch(eap: &mut Exarg) {
    g_st().g_do_tagpreview = opt_vars().p_pvh as i32;
    ex_findpat(eap);
    g_st().g_do_tagpreview = 0;
}

fn ex_findpat(eap: &mut Exarg) {
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

    let mut n: i32 = 1;
    if ascii_isdigit(unsafe { *eap.arg }) {
        n = getdigits_int(&mut eap.arg, false, 0);
        eap.arg = skipwhite(eap.arg);
    }
    if crate::memory::cstr_first_byte(eap.arg) == b'/' {
        whole = false;
        eap.arg = eap.arg.wrapping_add(1);
        let mut p = skip_regexp(eap.arg, b'/' as i32, opt_vars().p_magic);
        if !crate::memory::cstr_is_empty(p) {
            unsafe {
                *p = NUL;
            }
            p = skipwhite(p.wrapping_add(1));

            if ends_excmd(i32::from(unsafe { *p })) == 0 {
                eap.errmsg = Some(ex_errmsg(E_TRAILING_ARG, p));
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
            if crate::memory::cstr_first_byte(eap.cmd) == b'd' {
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

fn ex_ptag(eap: &mut Exarg) {
    g_st().g_do_tagpreview = opt_vars().p_pvh as i32;
    ex_tag_cmd(
        eap,
        CMDNAMES[eap.cmdidx as usize]
            .cmd_name
            .as_ptr()
            .wrapping_add(1),
    );
}

fn ex_pedit(eap: &mut Exarg) {
    let curwin_save = curwin_ptr();
    prepare_preview_window();
    do_exedit(eap, ptr::null_mut());
    back_to_current_window(curwin_save);
}

fn ex_pbuffer(eap: &mut Exarg) {
    let curwin_save = curwin_ptr();
    prepare_preview_window();
    do_exbuffer(eap);
    back_to_current_window(curwin_save);
}

fn prepare_preview_window() {
    g_st().g_do_tagpreview = opt_vars().p_pvh as i32;
    prepare_tagpreview(true, !crate::memory::cstr_is_empty(opt_vars().p_pvp));
}

fn back_to_current_window(curwin_save: *mut WinInfoInfo) {
    if curwin_ptr() != curwin_save && win_valid(curwin_save) {
        validate_cursor(curwin_mut());
        redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
        win_enter(curwin_save, true);
    }
    g_st().g_do_tagpreview = 0;
}

fn ex_stag(eap: &mut Exarg) {
    g_st().postponed_split = -1;
    g_st().postponed_split_flags = g_st().cmdmod.cmod_split;
    g_st().postponed_split_tab = g_st().cmdmod.cmod_tab;
    ex_tag_cmd(
        eap,
        CMDNAMES[eap.cmdidx as usize]
            .cmd_name
            .as_ptr()
            .wrapping_add(1),
    );
    g_st().postponed_split_flags = 0;
    g_st().postponed_split_tab = 0;
}

fn ex_tag(eap: &mut Exarg) {
    ex_tag_cmd(eap, CMDNAMES[eap.cmdidx as usize].cmd_name.as_ptr());
}

fn ex_tag_cmd(eap: &mut Exarg, name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr();
    {
        let mut cmd = match crate::memory::cstr_byte_at(name, 1) {
            b'j' => DT_JUMP,
            b's' => DT_SELECT,
            b'p' | b'N' => DT_PREV,
            b'n' => DT_NEXT,
            b'o' => DT_POP,
            b'f' | b'r' => DT_FIRST,
            b'l' => DT_LAST,
            _ => DT_TAG,
        };

        if crate::memory::cstr_first_byte(name) == b'l' {
            cmd = DT_LTAG;
        }

        do_tag(
            Some(&*eap),
            eap.arg,
            cmd,
            if eap.addr_count > 0 { eap.line2 } else { 1 },
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

pub fn find_cmdline_var(src: impl crate::strings::IntoCCharPtr, usedlen: &mut usize) -> isize {
    let src = src.into_c_char_ptr();
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
        if crate::memory::cstr_prefix_bytes(src, len)
            == crate::memory::cstr_prefix_bytes(s.as_ptr(), len)
        {
            *usedlen = len;
            return i as isize;
        }
    }
    -1
}

pub fn eval_vars(
    src: RawCCharPtrMut,
    srcstart: impl crate::strings::IntoCCharPtr,
    usedlen: &mut usize,
    lnump: Option<&mut Linenr>,
    errormsg: &mut Option<&'static std::ffi::CStr>,
    mut escaped: Option<&mut i32>,
    empty_is_error: bool,
) -> Option<std::ffi::CString> {
    let srcstart = srcstart.into_c_char_ptr();
    unsafe {
        let mut result: RawCCharPtrMut = c"".as_ptr().cast_mut();
        let mut resultbuf: Option<Vec<u8>> = None;
        let mut resultlen: usize;
        let mut valid = VALID_HEAD | VALID_PATH;
        let mut tilde_file = false;
        let mut skip_mod = false;
        let mut strbuf = [0u8; 30];
        let mut use_shellslash = false;
        let arg_all_cstr: Option<std::ffi::CString>;

        *errormsg = None;
        if let Some(escaped) = escaped.as_deref_mut() {
            *escaped = i32::from(false);
        }

        let spec_idx = find_cmdline_var(src, &mut *usedlen);
        if spec_idx < 0 {
            *usedlen = 1;
            return None;
        }

        if (src as usize) > (srcstart as usize)
            && crate::memory::cstr_first_byte(src.wrapping_sub(1)) == b'\\'
        {
            *usedlen = 0;
            crate::memory::cstr_move(src.wrapping_sub(1), src);
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
            let ident = find_ident_under_cursor(find_type, None);
            if ident.is_empty() {
                *errormsg = Some(c"");
                return None;
            }
            result = ident.as_ptr().cast_mut().cast();
            resultlen = ident.len();
        } else {
            match spec_idx {
                SPEC_PERC => {
                    if curbuf_mut().b_fname.is_null() {
                        result = c"".as_ptr().cast_mut();
                        valid = 0;
                    } else {
                        result = curbuf_mut().b_fname;
                        tilde_file = crate::memory::cstr_as_bytes(result) == b"~";
                    }
                    use_shellslash = true;
                }
                SPEC_HASH => {
                    if crate::memory::cstr_byte_at(src, 1) == b'#' {
                        arg_all_cstr = Some(arg_all());
                        result = arg_all_cstr.as_ref().unwrap().as_ptr().cast_mut();
                        *usedlen = 2;
                        if let Some(escaped) = escaped {
                            *escaped = i32::from(true);
                        }
                        skip_mod = true;
                    } else {
                        use_shellslash = true;
                        let mut s = src.wrapping_add(1);
                        if crate::memory::cstr_first_byte(s) == b'<' {
                            s = s.wrapping_add(1);
                        }
                        let i = getdigits_int(&mut s, false, 0);
                        if s == src.wrapping_add(2) && crate::memory::cstr_byte_at(src, 1) == b'-' {
                            s = s.wrapping_sub(1);
                        }
                        *usedlen = (s as usize) - (src as usize);

                        if crate::memory::cstr_byte_at(src, 1) == b'<' && i != 0 {
                            if *usedlen < 2 {
                                *usedlen = 1;
                                return None;
                            }
                            let Some(oldfile) =
                                tv_list_find_str(get_vim_var_list(VV_OLDFILES).as_mut(), i - 1)
                            else {
                                *errormsg = Some(c"");
                                return None;
                            };
                            result = oldfile.as_ptr().cast_mut();
                        } else {
                            if i == 0 && crate::memory::cstr_byte_at(src, 1) == b'<' && *usedlen > 1
                            {
                                *usedlen = 1;
                            }
                            let buf = buflist_findnr(i);
                            if buf.is_null() {
                                *errormsg = Some(gettext(
                                    c"E194: No alternate file name to substitute for '#'",
                                ));
                                return None;
                            }
                            let buf = &mut *buf;
                            if let Some(lnump) = lnump {
                                *lnump = ECMD_LAST;
                            }
                            if buf.b_fname.is_null() {
                                result = c"".as_ptr().cast_mut();
                                valid = 0;
                            } else {
                                result = buf.b_fname;
                                tilde_file = crate::memory::cstr_as_bytes(result) == b"~";
                            }
                        }
                    }
                }
                SPEC_CFILE => {
                    let Some(cfile) = file_name_at_cursor(FNAME_MESS | FNAME_HYP, 1, None) else {
                        *errormsg = Some(c"");
                        return None;
                    };
                    let mut v: Vec<u8> = cfile.into_bytes_with_nul();
                    result = v.as_mut_ptr().cast();
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
                            Some(gettext(E_NO_AUTOCOMMAND_FILE_NAME_TO_SUBSTITUTE_FOR_AFILE));
                        return None;
                    }
                    result = path_try_shorten_fname(result);
                }
                SPEC_ABUF => {
                    if g_st().autocmd_bufnr <= 0 {
                        *errormsg = Some(gettext(
                            E_NO_AUTOCOMMAND_BUFFER_NUMBER_TO_SUBSTITUTE_FOR_ABUF,
                        ));
                        return None;
                    }
                    let _ =
                        vim_snprintf!(strbuf.as_mut_ptr().cast(), 30, c"%d", g_st().autocmd_bufnr);
                    result = strbuf.as_mut_ptr().cast();
                }
                SPEC_AMATCH => {
                    result = (&raw const g_st().autocmd_match)
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                    if result.is_null() {
                        *errormsg = Some(gettext(
                            E_NO_AUTOCOMMAND_MATCH_NAME_TO_SUBSTITUTE_FOR_AMATCH,
                        ));
                        return None;
                    }
                }
                SPEC_SFILE => {
                    use_shellslash = true;
                    let Some(sfile) = estack_sfile(ESTACK_SFILE) else {
                        *errormsg = Some(gettext(E_NO_SOURCE_FILE_NAME_TO_SUBSTITUTE_FOR_SFILE));
                        return None;
                    };
                    let mut v: Vec<u8> = sfile.into_bytes_with_nul();
                    result = v.as_mut_ptr().cast();
                    resultbuf = Some(v);
                }
                SPEC_STACK => {
                    use_shellslash = true;
                    let Some(stack) = estack_sfile(ESTACK_STACK) else {
                        *errormsg = Some(gettext(E_NO_CALL_STACK_TO_SUBSTITUTE_FOR_STACK));
                        return None;
                    };
                    let mut v: Vec<u8> = stack.into_bytes_with_nul();
                    result = v.as_mut_ptr().cast();
                    resultbuf = Some(v);
                }
                SPEC_SCRIPT => {
                    use_shellslash = true;
                    let Some(script) = estack_sfile(ESTACK_SCRIPT) else {
                        *errormsg = Some(gettext(E_NO_SCRIPT_FILE_NAME_TO_SUBSTITUTE_FOR_SCRIPT));
                        return None;
                    };
                    let mut v: Vec<u8> = script.into_bytes_with_nul();
                    result = v.as_mut_ptr().cast();
                    resultbuf = Some(v);
                }
                SPEC_SLNUM => {
                    if sourcing_name().is_none() || *sourcing_lnum_ptr() == 0 {
                        *errormsg = Some(gettext(E_NO_LINE_NUMBER_TO_USE_FOR_SLNUM));
                        return None;
                    }
                    let _ =
                        vim_snprintf!(strbuf.as_mut_ptr().cast(), 30, c"%d", *sourcing_lnum_ptr(),);
                    result = strbuf.as_mut_ptr().cast();
                }
                SPEC_SFLNUM => {
                    let sflnum = g_st().current_sctx.sc_lnum + *sourcing_lnum_ptr();
                    if sflnum == 0 {
                        *errormsg = Some(gettext(E_NO_LINE_NUMBER_TO_USE_FOR_SFLNUM));
                        return None;
                    }
                    let _ = vim_snprintf!(strbuf.as_mut_ptr().cast(), 30, c"%d", sflnum);
                    result = strbuf.as_mut_ptr().cast();
                }
                SPEC_SID => {
                    if g_st().current_sctx.sc_sid <= 0 {
                        *errormsg = Some(gettext(E_USINGSID));
                        return None;
                    }
                    let _ = vim_snprintf!(
                        strbuf.as_mut_ptr().cast(),
                        30,
                        c"<SNR>%d_",
                        g_st().current_sctx.sc_sid,
                    );
                    result = strbuf.as_mut_ptr().cast();
                }
                _ => {
                    *errormsg = Some(c"");
                }
            }

            resultlen = crate::memory::cstr_len(result);
            if crate::memory::cstr_byte_at(src, *usedlen) == b'<' {
                *usedlen += 1;
                let s = crate::memory::cstr_as_bytes(result)
                    .iter()
                    .rposition(|&b| b == b'.')
                    .map_or(ptr::null_mut(), |i| result.wrapping_add(i));
                if !s.is_null() && s >= path_tail(result) {
                    resultlen = (s as usize) - (result as usize);
                }
            } else if !skip_mod {
                let mut res_u8 = result.cast::<u8>();
                valid |= modify_fname(
                    src,
                    tilde_file,
                    usedlen,
                    &raw mut res_u8,
                    &mut resultbuf,
                    &mut resultlen,
                    use_shellslash,
                );
                result = res_u8.cast();
                if result.is_null() {
                    *errormsg = Some(c"");
                    return None;
                }
            }
        }

        let ret = if resultlen == 0 || valid != VALID_HEAD + VALID_PATH {
            if empty_is_error {
                if valid != VALID_HEAD + VALID_PATH {
                    *errormsg = Some(gettext(
                        c"E499: Empty file name for '%' or '#', only works with \":p:h\"",
                    ));
                } else {
                    *errormsg = Some(gettext(c"E500: Evaluates to an empty string"));
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

pub fn expand_sfile(arg: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let arg = arg.into_c_char_ptr();
    {
        let mut result_vec: Vec<u8> = cstr_to_cstring(arg).into_bytes_with_nul();
        let mut idx: usize = 0;

        while result_vec[idx] != 0 {
            let p: RawCCharPtrMut = result_vec.as_mut_ptr().wrapping_add(idx).cast();
            if !(crate::memory::cstr_prefix_bytes(p, 7) == &b"<sfile>"[..7]) {
                idx += 1;
            } else {
                let mut srclen: usize = 0;
                let mut errormsg: Option<&'static std::ffi::CStr> = None;
                let repl = eval_vars(p, &result_vec, &mut srclen, None, &mut errormsg, None, true);
                if let Some(msg) = errormsg {
                    if !msg.to_bytes().is_empty() {
                        let _ = emsg(msg);
                    }
                    return None;
                }
                let Some(repl_cstr) = repl else {
                    idx += srclen;
                    continue;
                };
                let repl_bytes = repl_cstr.as_bytes();
                let tail_slice = &result_vec[(idx + srclen)..];
                let mut new_vec = Vec::with_capacity(idx + repl_bytes.len() + tail_slice.len());
                new_vec.extend_from_slice(&result_vec[..idx]);
                new_vec.extend_from_slice(repl_bytes);
                new_vec.extend_from_slice(tail_slice);
                result_vec = new_vec;
                idx += repl_bytes.len();
            }
        }

        Some(cstr_to_cstring(&result_vec))
    }
}

fn ex_shada(eap: &mut Exarg) {
    let save_shada = opt_vars().p_shada;
    if crate::memory::cstr_is_empty(opt_vars().p_shada) {
        opt_vars().p_shada = c"'100".as_ptr().cast_mut();
    }
    if eap.cmdidx == Cmdidx::CmdRviminfo as i32 || eap.cmdidx == Cmdidx::CmdRshada as i32 {
        shada_read_everything(eap.arg, eap.forceit != 0, false);
    } else {
        shada_write_file(eap.arg, eap.forceit != 0);
    }
    opt_vars().p_shada = save_shada;
}

fn ex_filetype(eap: &mut Exarg) {
    if crate::memory::cstr_is_empty(eap.arg) {
        let _ = smsg!(
            0,
            c"filetype detection:%s  plugin:%s  indent:%s",
            if docmd_st().filetype_detect == K_TRUE {
                c"ON".as_ptr()
            } else {
                c"OFF".as_ptr()
            },
            if docmd_st().filetype_plugin == K_TRUE {
                if docmd_st().filetype_detect == K_TRUE {
                    c"ON".as_ptr()
                } else {
                    c"(on)".as_ptr()
                }
            } else {
                c"OFF".as_ptr()
            },
            if docmd_st().filetype_indent == K_TRUE {
                if docmd_st().filetype_detect == K_TRUE {
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
            arg = skipwhite(arg.wrapping_add(6));
            continue;
        }
        if crate::memory::cstr_prefix_bytes(arg, 6) == &b"indent"[..6] {
            indent = true;
            arg = skipwhite(arg.wrapping_add(6));
            continue;
        }
        break;
    }

    if crate::memory::cstr_as_bytes(arg) == b"on" || crate::memory::cstr_as_bytes(arg) == b"detect"
    {
        if crate::memory::cstr_first_byte(arg) == b'o' || docmd_st().filetype_detect != K_TRUE {
            source_runtime(FILETYPE_FILE, DIP_ALL);
            docmd_st().filetype_detect = K_TRUE;
            if plugin {
                source_runtime(FTPLUGIN_FILE, DIP_ALL);
                docmd_st().filetype_plugin = K_TRUE;
            }
            if indent {
                source_runtime(INDENT_FILE, DIP_ALL);
                docmd_st().filetype_indent = K_TRUE;
            }
        }
        if crate::memory::cstr_first_byte(arg) == b'd' {
            do_doautocmd(c"filetypedetect BufRead", true, None);
            do_modelines(0);
        }
    } else if crate::memory::cstr_as_bytes(arg) == b"off" {
        if plugin || indent {
            if plugin {
                source_runtime(FTPLUGOF_FILE, DIP_ALL);
                docmd_st().filetype_plugin = K_FALSE;
            }
            if indent {
                source_runtime(INDOFF_FILE, DIP_ALL);
                docmd_st().filetype_indent = K_FALSE;
            }
        } else {
            source_runtime(FTOFF_FILE, DIP_ALL);
            docmd_st().filetype_detect = K_FALSE;
        }
    } else {
        let _ = semsg!(gettext(E_INVARG2), arg);
    }
}

pub fn ex_errmsg(
    msg: &std::ffi::CStr,
    arg: impl crate::strings::IntoCCharPtr,
) -> &'static std::ffi::CStr {
    let arg = arg.into_c_char_ptr();
    let buf: RawCCharPtrMut = (&raw mut docmd_st().ex_error_buf).cast();
    let _ = vim_snprintf!(buf, MSG_BUF_LEN, gettext(msg.as_ptr()), arg);
    crate::memory::cstr_from_ptr(buf)
}

pub fn filetype_plugin_enable() {
    if docmd_st().filetype_plugin == K_NONE {
        source_runtime(FTPLUGIN_FILE, DIP_ALL);
        docmd_st().filetype_plugin = K_TRUE;
    }
    if docmd_st().filetype_indent == K_NONE {
        source_runtime(INDENT_FILE, DIP_ALL);
        docmd_st().filetype_indent = K_TRUE;
    }
}

pub fn filetype_maybe_enable() {
    if docmd_st().filetype_detect == K_NONE {
        source_runtime(FILETYPE_FILE, DIP_ALL);
        docmd_st().filetype_detect = K_TRUE;
    }
}

fn ex_setfiletype(eap: &mut Exarg) {
    if curbuf_mut().b_did_filetype {
        return;
    }

    let mut arg = eap.arg;
    if crate::memory::cstr_prefix_bytes(arg, 9) == &b"FALLBACK "[..9] {
        arg = arg.wrapping_add(9);
    }

    set_option_value_give_err(K_OPT_FILETYPE, cstr_as_obj(arg), OPT_LOCAL);
    if arg != eap.arg {
        curbuf_mut().b_did_filetype = false;
    }
}

fn ex_digraphs(eap: &mut Exarg) {
    if !crate::memory::cstr_is_empty(eap.arg) {
        putdigraph(eap.arg);
    } else {
        listdigraphs(eap.forceit != 0);
    }
}

pub fn set_no_hlsearch(flag: bool) {
    g_st().search_global.no_hlsearch = flag;
    set_vim_var_nr(
        VV_HLSEARCH,
        Varnumber::from(!g_st().search_global.no_hlsearch && opt_vars().p_hls != 0),
    );
}

fn ex_nohlsearch(_eap: &mut Exarg) {
    set_no_hlsearch(true);
    redraw_all_later(UPD_SOME_VALID);
}

fn ex_fold(eap: &mut Exarg) {
    if fold_manual_allowed(true) != 0 {
        let start = CursorPos {
            lnum: eap.line1,
            col: 1,
            coladd: 0,
        };
        let end = CursorPos {
            lnum: eap.line2,
            col: 1,
            coladd: 0,
        };
        fold_create(curwin_mut(), start, end);
    }
}

fn ex_foldopen(eap: &mut Exarg) {
    let start = CursorPos {
        lnum: eap.line1,
        col: 1,
        coladd: 0,
    };
    let end = CursorPos {
        lnum: eap.line2,
        col: 1,
        coladd: 0,
    };
    op_fold_range(
        start,
        end,
        i32::from(eap.cmdidx == Cmdidx::CmdFoldopen as i32),
        eap.forceit,
        false,
    );
}

fn ex_folddo(eap: &mut Exarg) {
    let mut lnum = eap.line1;
    while lnum <= eap.line2 {
        if has_folding(curwin_ptr(), lnum, None, ptr::null_mut())
            == (eap.cmdidx == Cmdidx::CmdFolddoclosed as i32)
        {
            ml_setmarked(lnum);
        }
        lnum += 1;
    }

    global_exe(eap.arg);
    ml_clearmarked();
}

pub fn is_loclist_cmd(cmdidx: i32) -> bool {
    if !(0..CMD_SIZE_I32).contains(&cmdidx) {
        return false;
    }
    CMDNAMES[cmdidx as usize].cmd_name.to_bytes().first() == Some(&b'l')
}

fn ex_checkhealth(eap: &mut Exarg) {
    g_st().emsg_off += 1;
    let ok = nlua_call_excmd(c"vim.health", c"_check", eap, &g_st().cmdmod, None);
    g_st().emsg_off -= 1;
    if ok {
        return;
    }

    let Some(vimruntime_env) = os_getenv_cstr(c"VIMRUNTIME") else {
        let _ = emsg(gettext(c"E5009: $VIMRUNTIME is empty or unset"));
        return;
    };
    let vr_bytes = vimruntime_env.to_bytes();
    let rtp_ok = vr_bytes.is_empty()
        || crate::memory::cstr_as_bytes(opt_vars().p_rtp)
            .windows(vr_bytes.len())
            .any(|w| w == vr_bytes);
    if rtp_ok {
        let _ = semsg!(
            gettext(c"E5009: Invalid $VIMRUNTIME: %s"),
            vimruntime_env.as_ptr(),
        );
    } else {
        let _ = emsg(gettext(c"E5009: Invalid 'runtimepath'"));
    }
}

fn ex_terminal(eap: &mut Exarg) {
    let scroll_save = g_st().msg_scroll;
    g_st().msg_scroll = 0;
    autowrite_all();
    g_st().msg_scroll = scroll_save;

    if !crate::memory::cstr_is_empty(eap.arg) {
        nlua_call_excmd(
            c"vim._core.ex_cmd",
            c"ex_terminal",
            eap,
            &g_st().cmdmod,
            None,
        );
    } else {
        if crate::memory::cstr_is_empty(opt_vars().p_sh) {
            let _ = emsg(gettext(E_SHELLEMPTY));
            return;
        }
        let argv = shell_build_argv(ptr::null(), ptr::null());
        let mut shell_tv: TypVal = TypVal::new();
        tv_list_alloc_ret(&mut shell_tv, 0);
        for arg in argv {
            tv_list_append_string(
                unsafe { &mut *shell_tv.vval.v_list() },
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
        tv_clear(&mut shell_tv);
    }
}

fn ex_log(eap: &mut Exarg) {
    nlua_call_excmd(c"vim._core.ex_cmd", c"ex_log", eap, &g_st().cmdmod, None);
}

fn ex_lsp(eap: &mut Exarg) {
    nlua_call_excmd(c"vim._core.ex_cmd", c"ex_lsp", eap, &g_st().cmdmod, None);
}

fn ex_packdel(eap: &mut Exarg) {
    nlua_call_excmd(
        c"vim._core.ex_cmd",
        c"ex_packdel",
        eap,
        &g_st().cmdmod,
        None,
    );
}

fn ex_packupdate(eap: &mut Exarg) {
    nlua_call_excmd(
        c"vim._core.ex_cmd",
        c"ex_packupdate",
        eap,
        &g_st().cmdmod,
        None,
    );
}

fn ex_uptime(eap: &mut Exarg) {
    nlua_call_excmd(c"vim._core.ex_cmd", c"ex_uptime", eap, &g_st().cmdmod, None);
}

fn ex_fclose(eap: &mut Exarg) {
    win_float_remove(eap.forceit != 0, eap.line1);
}

pub fn verify_command(cmd: impl crate::strings::IntoCCharPtr) {
    let cmd = cmd.into_c_char_ptr();
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

pub fn is_map_cmd(cmdidx: Cmdidx) -> bool {
    if is_user_cmdidx(cmdidx) {
        return false;
    }

    let func = CMDNAMES[cmdidx as usize].cmd_func;
    ptr::fn_addr_eq(func, ex_map as ExFunc)
        || ptr::fn_addr_eq(func, ex_unmap as ExFunc)
        || ptr::fn_addr_eq(func, ex_mapclear as ExFunc)
        || ptr::fn_addr_eq(func, ex_abbreviate as ExFunc)
        || ptr::fn_addr_eq(func, ex_abclear as ExFunc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_excmd_identifies_command_terminators() {
        assert_ne!(ends_excmd(0), 0);
        assert_ne!(ends_excmd(b'|' as i32), 0);
        assert_ne!(ends_excmd(b'"' as i32), 0);
        assert_ne!(ends_excmd(b'\n' as i32), 0);
        assert_eq!(ends_excmd(b'a' as i32), 0);
    }

    #[test]
    fn cmd_has_expr_args_identifies_expr_commands() {
        assert!(cmd_has_expr_args(Cmdidx::CmdExecute));
        assert!(cmd_has_expr_args(Cmdidx::CmdEcho));
        assert!(cmd_has_expr_args(Cmdidx::CmdEchon));
        assert!(cmd_has_expr_args(Cmdidx::CmdEchomsg));
        assert!(cmd_has_expr_args(Cmdidx::CmdEchoerr));
        assert!(!cmd_has_expr_args(Cmdidx::CmdEdit));
    }

    #[test]
    fn find_cmdline_var_matches_special_variables() {
        let mut usedlen = 0usize;
        assert_eq!(find_cmdline_var(c"<cword>", &mut usedlen), SPEC_CWORD);
        assert_eq!(usedlen, 7);
        assert_eq!(find_cmdline_var(c"%:p", &mut usedlen), SPEC_PERC);
        assert_eq!(usedlen, 1);
        assert_eq!(find_cmdline_var(c"foo", &mut usedlen), -1);
    }
}
