//! Functions for dealing with the buffer structure (`src/nvim/buffer.c`).

use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO, cstr_as_string, tv_dict_is_watched};
use crate::api::win_config::{UIExtension, kCtxNoDisplay, kWinStyleMinimal};
use crate::arglist::{ML_EMPTY, check_arg_idx};
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFDELETE, EVENT_BUFENTER, EVENT_BUFHIDDEN, EVENT_BUFLEAVE, EVENT_BUFNEW,
    EVENT_BUFUNLOAD, EVENT_BUFWINENTER, EVENT_BUFWINLEAVE, EVENT_BUFWIPEOUT, EVENT_STDINREADPOST,
    apply_autocmds, apply_autocmds_retval, apply_autocmds_win, aubuflocal_remove, augroup_exists,
    block_autocmds, do_doautocmd, unblock_autocmds,
};
use crate::buffer_updates::{buf_free_callbacks, buf_updates_unload};
use crate::change::{changed, deleted_lines_mark, file_ff_differs, save_file_ff, unchanged};
use crate::channel::channel_job_running;
use crate::charset::{
    ascii_isdigit, ascii_isspace, buf_init_isk_chartab, getdigits_int, skiptowhite_esc, skipwhite,
    trans_characters, try_getdigits, vim_strsize,
};
use crate::cmdexpand::{
    BUF_DIFF_FILTER, WILD_BUFLASTUSED, WILD_HOME_REPLACE, cmdidx_T, cmdline_fuzzy_complete,
};
use crate::context::{CtxSwitch, INIT_FMARK, INIT_FMARKV, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{check_cursor_col, check_cursor_lnum, colnr_T, linenr_T, pos_T};
use crate::diff::{diff_buf_add, diff_buf_delete, diff_mode_buf, diffopt_hiddenoff};
use crate::digraph::keymap_init;
use crate::drawscreen::{UPD_NOT_VALID, redraw_later, redrawing, status_redraw_all};
use crate::errors::*;
use crate::eval::typval::{
    TypvalVval, VarType, callback_free, tv_dict_add, tv_dict_alloc, tv_dict_find,
    tv_dict_item_copy, tv_dict_watcher_notify, typval_T, varnumber_T,
};
use crate::eval::vars::{init_var_dict, unref_var_dict, vars_clear};
use crate::ex_cmds::{ECMD_FORCEIT, ECMD_ONE, do_ecmd, getfile};
use crate::ex_cmds2::{autowrite, can_abandon, dialog_changed, dialog_close_terminal};
use crate::ex_docmd::{
    CMOD_CONFIRM, CMOD_HIDE, CMOD_KEEPALT, do_cmdline_cmd, ex_errmsg, exarg_T, tabpage_new,
};
use crate::ex_eval::{aborting, cleanup_T, enter_cleanup, leave_cleanup};
use crate::ex_getln::text_or_buf_locked;
use crate::extmark::extmark_free_all;
use crate::file_search::vim_chdirfile;
use crate::fileio::{
    READ_BUFFER, READ_DUMMY, READ_FIFO, READ_NEW, READ_NOFILE, READ_NOWINENTER, READ_STDIN,
    buf_check_timestamp, file_pat_to_reg_pat, prep_exarg, readfile, shorten_fnames,
};
use crate::fold::{clearFolding, foldUpdateAll};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::hashtab::{hash_find, hash_remove};
use crate::help::get_local_additions;
use crate::indent::inindent;
use crate::indent_c::parse_cino;
use crate::input::vgetc;
use crate::insexpand::clear_cpt_callbacks;
use crate::main_init::getout;
use crate::mapping::map_clear_mode;
use crate::mark::{
    WinInfo, clear_fmark, clrallmarks, fmark_T, fmarks_check_names, mark_adjust_buf,
    mark_forget_file, mark_jumplist_forget_file, mark_view_make, mark_view_restore,
    set_last_cursor, setpcmark,
};
use crate::math::{FAIL, OK};
use crate::mbyte::utf_cp_bounds;
use crate::mcursor::{mc_buf_clear, mc_buf_free};
use crate::memline::{
    ml_close, ml_delete, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_open, ml_recover,
    ml_setname, ml_timestamp,
};
use crate::memory::{cstr_from_bytes, cstr_to_cstring, mem_st, xstrlcpy};
use crate::message::{
    emsg, message_filtered, msg, msg_delay, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts,
    msg_start, msg_trunc, semsg, set_keep_msg, smsg,
};
use crate::r#move::{scroll_cursor_halfway, validate_virtcol};
use crate::normal::{end_visual_mode, reset_VIsual_and_resel};
use crate::option::{
    OPT_LOCAL, OptInt, buf_copy_options, clear_winopt, copy_winopt, didset_window_options, do_set,
    shortmess,
};
use crate::option_vars::*;
use crate::optionstr::{CharBuf, OptIndex, clear_string_option};
use crate::os::env::{home_replace, home_replace_save};
#[cfg(unix)]
use crate::os::fs::os_getperm;
#[cfg(windows)]
use crate::os::fs::os_resolve_shortcut;
use crate::os::fs::{CASE_INSENSITIVE_FILENAME, FileID, os_fileid, os_fileid_equal, os_isdir};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::time::os_time;
use crate::path::{
    FullName_save, after_pathsep, concat_fnames_cstr, fix_fname, get_past_head, kPathCmpLiteral,
    path_equal, path_fix_case, path_tail, path_tail_with_sep, to_slash,
};
use crate::plines::win_get_fill;
use crate::quickfix::qf_stack_get_bufnr;
use crate::regexp::{RE_MAGIC, regmatch_T, vim_regcomp, vim_regexec, vim_regfree};
use crate::runtime::{ETYPE_MODELINE, estack_pop, estack_push};
use crate::spell::parse_spelllang;
use crate::state::{MAP_ALL_MODES, MODE_INSERT};
use crate::statusline::build_stl_str_hl;
use crate::strings::{
    gettext, ngettext, vim_snprintf, vim_snprintf_safelen,
    vim_strchr,
};
use crate::syntax::{reset_synblock, synblock_T, syntax_clear};
use crate::terminal::{Terminal, terminal_check_size, terminal_close, terminal_running};
use crate::ui::{ui_call_set_icon, ui_call_set_title, ui_has};
use crate::undo::{u_clearallandblockfree, u_sync, undo_fmt_time};
use crate::version::min_vim_version;
use crate::window::{
    WSP_BELOW, WSP_ROOM, WSP_VERT, check_can_set_curbuf_forceit, check_colorcolumn, close_windows,
    global_stl_height, goto_tabpage_tp, goto_tabpage_win, last_window, lastwin_nofloating,
    one_window, reset_binding, swbuf_goto_win_with_buf, tabline_height, tabpage_T, tabpage_index,
    update_cwd, win_T, win_close, win_enter, win_locked, win_move_after, win_split, win_valid,
    win_valid_any_tab, window_layout_lock, window_layout_unlock,
};
use crate::winfloat::{win_float_update_preview, win_set_minimal_style};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

pub(crate) const BCO_ENTER: c_int = 1;
pub(crate) const BCO_NOHELP: c_int = 4;
pub(crate) const BF_DUMMY: c_int = 0x80;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct bufref_T {
    pub br_buf: *mut buf_T,
    pub br_fnum: c_int,
    pub br_buf_free_count: c_int,
}

impl bufref_T {
    #[inline]
    pub const fn new() -> Self {
        Self {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        }
    }
}

impl Default for bufref_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

pub type ChangedtickDictItem = crate::eval::typval::dictitem_T;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufSigncols {
    pub max: c_int,
    pub last_max: c_int,
    pub count: [c_int; 9],
    pub autom: bool,
}

#[repr(C)]
pub struct BufUpdateCallbacks {
    pub on_lines: crate::lua::converter::LuaRef,
    pub on_bytes: crate::lua::converter::LuaRef,
    pub on_changedtick: crate::lua::converter::LuaRef,
    pub on_detach: crate::lua::converter::LuaRef,
    pub on_reload: crate::lua::converter::LuaRef,
    pub utf_sizes: bool,
    pub preview: bool,
}

impl Drop for BufUpdateCallbacks {
    fn drop(&mut self) {
        crate::lua::executor::api_free_luaref(self.on_lines);
            crate::lua::executor::api_free_luaref(self.on_bytes);
            crate::lua::executor::api_free_luaref(self.on_changedtick);
            crate::lua::executor::api_free_luaref(self.on_reload);
            crate::lua::executor::api_free_luaref(self.on_detach);
    }
}

#[repr(C)]
pub struct buf_T {
    pub b_fnum: crate::msgpack_rpc::packer::handle_T,
    pub b_ml: crate::memline::memline_T,
    pub b_next: *mut buf_T,
    pub b_prev: *mut buf_T,
    pub b_nwindows: c_int,
    pub b_flags: c_int,
    pub b_locked: c_int,
    pub b_locked_split: c_int,
    pub b_ro_locked: c_int,
    pub b_ffname: *mut c_char,
    pub b_sfname: *mut c_char,
    pub b_fname: *mut c_char,
    pub b_ffname_cstr: Option<std::ffi::CString>,
    pub b_sfname_cstr: Option<std::ffi::CString>,
    pub file_id_valid: bool,
    pub file_id: FileID,
    pub b_changed: c_int,
    pub changedtick_di: ChangedtickDictItem,
    pub b_last_changedtick: i64,
    pub b_last_changedtick_i: i64,
    pub b_last_changedtick_pum: i64,
    pub b_saving: bool,
    pub b_mod_set: bool,
    pub b_mod_top: i32,
    pub b_mod_bot: i32,
    pub b_mod_xlines: i32,
    pub b_wininfo: Vec<WinInfo>,
    pub b_mod_tick_syn: u64,
    pub b_mod_tick_decor: u64,
    pub b_mtime: i64,
    pub b_mtime_ns: i64,
    pub b_mtime_read: i64,
    pub b_mtime_read_ns: i64,
    pub b_orig_size: u64,
    pub b_orig_mode: c_int,
    pub b_last_used: i64,
    pub b_namedm: [fmark_T; 26],
    pub b_visual: crate::normal::visualinfo_T,
    pub b_visual_mode_eval: c_int,
    pub b_last_cursor: fmark_T,
    pub b_last_insert: fmark_T,
    pub b_last_change: fmark_T,
    pub b_changelist: [fmark_T; 100],
    pub b_changelistlen: c_int,
    pub b_new_change: bool,
    pub b_chartab: [u64; 4],
    pub b_maphash: [Vec<crate::mapping::mapblock_T>; 256],
    pub b_first_abbr: Vec<crate::mapping::mapblock_T>,
    pub b_ucmds: Vec<crate::usercmd::ucmd_T>,
    pub b_op_start: pos_T,
    pub b_op_start_orig: pos_T,
    pub b_op_end: pos_T,
    pub b_marks_read: bool,
    pub b_modified_was_set: bool,
    pub b_did_filetype: bool,
    pub b_keep_filetype: bool,
    pub b_au_did_filetype: bool,
    pub b_u_headers: Vec<crate::undo::u_header_T>,
    pub b_u_free_headers: Vec<usize>,
    pub b_u_oldhead: Option<usize>,
    pub b_u_newhead: Option<usize>,
    pub b_u_curhead: Option<usize>,
    pub b_u_numhead: c_int,
    pub b_u_synced: bool,
    pub b_u_seq_last: c_int,
    pub b_u_save_nr_last: c_int,
    pub b_u_seq_cur: c_int,
    pub b_u_time_cur: i64,
    pub b_u_save_nr_cur: c_int,
    pub b_u_line_ptr: Option<std::ffi::CString>,
    pub b_u_line_lnum: i32,
    pub b_u_line_colnr: i32,
    pub b_scanned: bool,
    pub b_p_iminsert: i64,
    pub b_p_imsearch: i64,
    pub b_kmap_state: i16,
    pub b_kmap_ga: Vec<crate::digraph::kmap_T>,
    pub b_p_initialized: bool,
    pub b_p_script_ctx: [crate::runtime::sctx_T; 93],
    pub b_p_ac: c_int,
    pub b_p_ai: c_int,
    pub b_p_ai_nopaste: c_int,
    pub b_p_bkc: *mut c_char,
    pub b_bkc_flags: c_uint,
    pub b_p_ci: c_int,
    pub b_p_bin: c_int,
    pub b_p_bomb: c_int,
    pub b_p_bh: *mut c_char,
    pub b_p_bt: *mut c_char,
    pub b_p_busy: i64,
    pub b_has_qf_entry: c_int,
    pub b_p_bl: c_int,
    pub b_p_channel: i64,
    pub b_p_cin: c_int,
    pub b_p_cino: *mut c_char,
    pub b_p_cink: *mut c_char,
    pub b_p_cinw: *mut c_char,
    pub b_p_cinsd: *mut c_char,
    pub b_p_com: *mut c_char,
    pub b_p_cms: *mut c_char,
    pub b_p_cot: *mut c_char,
    pub b_cot_flags: c_uint,
    pub b_p_cpt: *mut c_char,
    #[cfg(windows)]
    pub b_p_csl: *mut c_char,
    pub b_p_cpt_flags: u32,
    pub b_p_cpt_cb: Vec<crate::eval::typval::Callback>,
    pub b_p_cfu: crate::eval::typval::Callback,
    pub b_p_ofu: crate::eval::typval::Callback,
    pub b_p_tfu: crate::eval::typval::Callback,
    pub b_p_ffu: crate::eval::typval::Callback,
    pub b_p_eof: c_int,
    pub b_p_eol: c_int,
    pub b_p_fixeol: c_int,
    pub b_p_et: c_int,
    pub b_p_et_nobin: c_int,
    pub b_p_et_nopaste: c_int,
    pub b_p_fenc: *mut c_char,
    pub b_p_ff: *mut c_char,
    pub b_p_ft: *mut c_char,
    pub b_p_follow: c_int,
    pub b_p_fo: *mut c_char,
    pub b_p_flp: *mut c_char,
    pub b_p_inf: c_int,
    pub b_p_isk: *mut c_char,
    pub b_p_def: *mut c_char,
    pub b_p_inc: *mut c_char,
    pub b_p_inex: crate::eval::typval::Callback,
    pub b_p_inex_flags: u32,
    pub b_p_inde: crate::eval::typval::Callback,
    pub b_p_inde_flags: u32,
    pub b_p_indk: *mut c_char,
    pub b_p_fp: *mut c_char,
    pub b_p_fex: crate::eval::typval::Callback,
    pub b_p_fex_flags: u32,
    pub b_p_fs: c_int,
    pub b_p_kp: *mut c_char,
    pub b_p_lisp: c_int,
    pub b_p_lop: *mut c_char,
    pub b_p_menc: *mut c_char,
    pub b_p_mps: *mut c_char,
    pub b_p_ml: c_int,
    pub b_p_ml_nobin: c_int,
    pub b_p_ma: c_int,
    pub b_p_nf: *mut c_char,
    pub b_p_pi: c_int,
    pub b_p_qe: *mut c_char,
    pub b_p_ro: c_int,
    pub b_p_sw: i64,
    pub b_p_scbk: i64,
    pub b_p_si: c_int,
    pub b_p_sts: i64,
    pub b_p_sts_nopaste: i64,
    pub b_p_sua: *mut c_char,
    pub b_p_swf: c_int,
    pub b_p_smc: i64,
    pub b_p_syn: *mut c_char,
    pub b_p_ts: i64,
    pub b_p_tw: i64,
    pub b_p_tw_nobin: i64,
    pub b_p_tw_nopaste: i64,
    pub b_p_wm: i64,
    pub b_p_wm_nobin: i64,
    pub b_p_wm_nopaste: i64,
    pub b_p_vsts: *mut c_char,
    pub b_p_vsts_array: Vec<i32>,
    pub b_p_vsts_nopaste: Option<std::ffi::CString>,
    pub b_p_vts: *mut c_char,
    pub b_p_vts_array: Vec<i32>,
    pub b_p_keymap: *mut c_char,
    pub b_p_gefm: *mut c_char,
    pub b_p_gp: *mut c_char,
    pub b_p_mp: *mut c_char,
    pub b_p_efm: *mut c_char,
    pub b_p_ep: *mut c_char,
    pub b_p_path: *mut c_char,
    pub b_p_ar: c_int,
    pub b_p_tags: *mut c_char,
    pub b_p_tc: *mut c_char,
    pub b_tc_flags: c_uint,
    pub b_p_dict: *mut c_char,
    pub b_p_dia: *mut c_char,
    pub b_p_tsr: *mut c_char,
    pub b_p_tsrfu: crate::eval::typval::Callback,
    pub b_p_ul: i64,
    pub b_p_udf: c_int,
    pub b_p_lw: *mut c_char,
    pub b_ind_level: c_int,
    pub b_ind_open_imag: c_int,
    pub b_ind_no_brace: c_int,
    pub b_ind_first_open: c_int,
    pub b_ind_open_extra: c_int,
    pub b_ind_close_extra: c_int,
    pub b_ind_open_left_imag: c_int,
    pub b_ind_jump_label: c_int,
    pub b_ind_case: c_int,
    pub b_ind_case_code: c_int,
    pub b_ind_case_break: c_int,
    pub b_ind_param: c_int,
    pub b_ind_func_type: c_int,
    pub b_ind_comment: c_int,
    pub b_ind_in_comment: c_int,
    pub b_ind_in_comment2: c_int,
    pub b_ind_cpp_baseclass: c_int,
    pub b_ind_continuation: c_int,
    pub b_ind_unclosed: c_int,
    pub b_ind_unclosed2: c_int,
    pub b_ind_unclosed_noignore: c_int,
    pub b_ind_unclosed_wrapped: c_int,
    pub b_ind_unclosed_whiteok: c_int,
    pub b_ind_matching_paren: c_int,
    pub b_ind_paren_prev: c_int,
    pub b_ind_maxparen: c_int,
    pub b_ind_maxcomment: c_int,
    pub b_ind_scopedecl: c_int,
    pub b_ind_scopedecl_code: c_int,
    pub b_ind_java: c_int,
    pub b_ind_js: c_int,
    pub b_ind_keep_case_label: c_int,
    pub b_ind_hash_comment: c_int,
    pub b_ind_cpp_namespace: c_int,
    pub b_ind_if_for_while: c_int,
    pub b_ind_cpp_extern_c: c_int,
    pub b_ind_pragma: c_int,
    pub b_no_eol_lnum: i32,
    pub b_start_eof: c_int,
    pub b_start_eol: c_int,
    pub b_start_ffc: c_int,
    pub b_start_fenc: Option<std::ffi::CString>,
    pub b_bad_char: c_int,
    pub b_start_bomb: c_int,
    pub b_bufvar: crate::eval::typval::ScopeDictDictItem,
    pub b_may_swap: bool,
    pub b_did_warn: bool,
    pub b_help: bool,
    pub b_spell: bool,
    pub b_prompt_text: Option<std::ffi::CString>,
    pub b_prompt_callback: crate::eval::typval::Callback,
    pub b_prompt_interrupt: crate::eval::typval::Callback,
    pub b_prompt_append_new_line: bool,
    pub b_prompt_insert: c_int,
    pub b_prompt_start: fmark_T,
    pub b_s: synblock_T,
    pub b_signcols: BufSigncols,
    pub terminal: *mut Terminal,
    pub additional_data: Option<crate::search::AdditionalData>,
    pub b_mapped_ctrl_c: c_int,
    pub b_marktree: crate::marktree::MarkTree,
    pub b_extmark_ns: std::collections::BTreeMap<u32, u32>,
    pub b_prev_line_count: c_int,
    pub update_channels: Vec<u64>,
    pub update_callbacks: Vec<BufUpdateCallbacks>,
    pub update_need_codepoints: bool,
    pub deleted_bytes: usize,
    pub deleted_bytes2: usize,
    pub deleted_codepoints: usize,
    pub deleted_codeunits: usize,
    pub flush_count: c_int,
    pub b_localdir: Option<std::ffi::CString>,
    pub b_prevdir: Option<std::ffi::CString>,
}

impl buf_T {
    pub const fn new() -> Self {
        const ZERO_POS: pos_T = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        const ZERO_SCTX: crate::runtime::sctx_T = crate::runtime::sctx_T {
            sc_sid: 0,
            sc_seq: 0,
            sc_lnum: 0,
            sc_chan: 0,
        };
        Self {
            b_fnum: 0,
            b_ml: crate::memline::memline_T::new(),
            b_next: ptr::null_mut(),
            b_prev: ptr::null_mut(),
            b_nwindows: 0,
            b_flags: 0,
            b_locked: 0,
            b_locked_split: 0,
            b_ro_locked: 0,
            b_ffname: ptr::null_mut(),
            b_sfname: ptr::null_mut(),
            b_fname: ptr::null_mut(),
            b_ffname_cstr: None,
            b_sfname_cstr: None,
            file_id_valid: false,
            file_id: FileID {
                inode: 0,
                device_id: 0,
            },
            b_changed: 0,
            changedtick_di: ChangedtickDictItem::new(),
            b_last_changedtick: 0,
            b_last_changedtick_i: 0,
            b_last_changedtick_pum: 0,
            b_saving: false,
            b_mod_set: false,
            b_mod_top: 0,
            b_mod_bot: 0,
            b_mod_xlines: 0,
            b_wininfo: Vec::new(),
            b_mod_tick_syn: 0,
            b_mod_tick_decor: 0,
            b_mtime: 0,
            b_mtime_ns: 0,
            b_mtime_read: 0,
            b_mtime_read_ns: 0,
            b_orig_size: 0,
            b_orig_mode: 0,
            b_last_used: 0,
            b_namedm: [const { fmark_T::ZERO }; 26],
            b_visual: crate::normal::visualinfo_T {
                vi_start: ZERO_POS,
                vi_end: ZERO_POS,
                vi_mode: 0,
                vi_curswant: 0,
            },
            b_visual_mode_eval: 0,
            b_last_cursor: fmark_T::ZERO,
            b_last_insert: fmark_T::ZERO,
            b_last_change: fmark_T::ZERO,
            b_changelist: [const { fmark_T::ZERO }; 100],
            b_changelistlen: 0,
            b_new_change: false,
            b_chartab: [0; 4],
            b_maphash: [const { Vec::new() }; 256],
            b_first_abbr: Vec::new(),
            b_ucmds: Vec::new(),
            b_op_start: ZERO_POS,
            b_op_start_orig: ZERO_POS,
            b_op_end: ZERO_POS,
            b_marks_read: false,
            b_modified_was_set: false,
            b_did_filetype: false,
            b_keep_filetype: false,
            b_au_did_filetype: false,
            b_u_headers: Vec::new(),
            b_u_free_headers: Vec::new(),
            b_u_oldhead: None,
            b_u_newhead: None,
            b_u_curhead: None,
            b_u_numhead: 0,
            b_u_synced: false,
            b_u_seq_last: 0,
            b_u_save_nr_last: 0,
            b_u_seq_cur: 0,
            b_u_time_cur: 0,
            b_u_save_nr_cur: 0,
            b_u_line_ptr: None,
            b_u_line_lnum: 0,
            b_u_line_colnr: 0,
            b_scanned: false,
            b_p_iminsert: 0,
            b_p_imsearch: 0,
            b_kmap_state: 0,
            b_kmap_ga: Vec::new(),
            b_p_initialized: false,
            b_p_script_ctx: [ZERO_SCTX; 93],
            b_p_ac: 0,
            b_p_ai: 0,
            b_p_ai_nopaste: 0,
            b_p_bkc: ptr::null_mut(),
            b_bkc_flags: 0,
            b_p_ci: 0,
            b_p_bin: 0,
            b_p_bomb: 0,
            b_p_bh: ptr::null_mut(),
            b_p_bt: ptr::null_mut(),
            b_p_busy: 0,
            b_has_qf_entry: 0,
            b_p_bl: 0,
            b_p_channel: 0,
            b_p_cin: 0,
            b_p_cino: ptr::null_mut(),
            b_p_cink: ptr::null_mut(),
            b_p_cinw: ptr::null_mut(),
            b_p_cinsd: ptr::null_mut(),
            b_p_com: ptr::null_mut(),
            b_p_cms: ptr::null_mut(),
            b_p_cot: ptr::null_mut(),
            b_cot_flags: 0,
            b_p_cpt: ptr::null_mut(),
            #[cfg(windows)]
            b_p_csl: ptr::null_mut(),
            b_p_cpt_flags: 0,
            b_p_cpt_cb: Vec::new(),
            b_p_cfu: crate::eval::typval::Callback::NONE,
            b_p_ofu: crate::eval::typval::Callback::NONE,
            b_p_tfu: crate::eval::typval::Callback::NONE,
            b_p_ffu: crate::eval::typval::Callback::NONE,
            b_p_eof: 0,
            b_p_eol: 0,
            b_p_fixeol: 0,
            b_p_et: 0,
            b_p_et_nobin: 0,
            b_p_et_nopaste: 0,
            b_p_fenc: ptr::null_mut(),
            b_p_ff: ptr::null_mut(),
            b_p_ft: ptr::null_mut(),
            b_p_follow: 0,
            b_p_fo: ptr::null_mut(),
            b_p_flp: ptr::null_mut(),
            b_p_inf: 0,
            b_p_isk: ptr::null_mut(),
            b_p_def: ptr::null_mut(),
            b_p_inc: ptr::null_mut(),
            b_p_inex: crate::eval::typval::Callback::NONE,
            b_p_inex_flags: 0,
            b_p_inde: crate::eval::typval::Callback::NONE,
            b_p_inde_flags: 0,
            b_p_indk: ptr::null_mut(),
            b_p_fp: ptr::null_mut(),
            b_p_fex: crate::eval::typval::Callback::NONE,
            b_p_fex_flags: 0,
            b_p_fs: 0,
            b_p_kp: ptr::null_mut(),
            b_p_lisp: 0,
            b_p_lop: ptr::null_mut(),
            b_p_menc: ptr::null_mut(),
            b_p_mps: ptr::null_mut(),
            b_p_ml: 0,
            b_p_ml_nobin: 0,
            b_p_ma: 0,
            b_p_nf: ptr::null_mut(),
            b_p_pi: 0,
            b_p_qe: ptr::null_mut(),
            b_p_ro: 0,
            b_p_sw: 0,
            b_p_scbk: 0,
            b_p_si: 0,
            b_p_sts: 0,
            b_p_sts_nopaste: 0,
            b_p_sua: ptr::null_mut(),
            b_p_swf: 0,
            b_p_smc: 0,
            b_p_syn: ptr::null_mut(),
            b_p_ts: 0,
            b_p_tw: 0,
            b_p_tw_nobin: 0,
            b_p_tw_nopaste: 0,
            b_p_wm: 0,
            b_p_wm_nobin: 0,
            b_p_wm_nopaste: 0,
            b_p_vsts: ptr::null_mut(),
            b_p_vsts_array: Vec::new(),
            b_p_vsts_nopaste: None,
            b_p_vts: ptr::null_mut(),
            b_p_vts_array: Vec::new(),
            b_p_keymap: ptr::null_mut(),
            b_p_gefm: ptr::null_mut(),
            b_p_gp: ptr::null_mut(),
            b_p_mp: ptr::null_mut(),
            b_p_efm: ptr::null_mut(),
            b_p_ep: ptr::null_mut(),
            b_p_path: ptr::null_mut(),
            b_p_ar: 0,
            b_p_tags: ptr::null_mut(),
            b_p_tc: ptr::null_mut(),
            b_tc_flags: 0,
            b_p_dict: ptr::null_mut(),
            b_p_dia: ptr::null_mut(),
            b_p_tsr: ptr::null_mut(),
            b_p_tsrfu: crate::eval::typval::Callback::NONE,
            b_p_ul: 0,
            b_p_udf: 0,
            b_p_lw: ptr::null_mut(),
            b_ind_level: 0,
            b_ind_open_imag: 0,
            b_ind_no_brace: 0,
            b_ind_first_open: 0,
            b_ind_open_extra: 0,
            b_ind_close_extra: 0,
            b_ind_open_left_imag: 0,
            b_ind_jump_label: 0,
            b_ind_case: 0,
            b_ind_case_code: 0,
            b_ind_case_break: 0,
            b_ind_param: 0,
            b_ind_func_type: 0,
            b_ind_comment: 0,
            b_ind_in_comment: 0,
            b_ind_in_comment2: 0,
            b_ind_cpp_baseclass: 0,
            b_ind_continuation: 0,
            b_ind_unclosed: 0,
            b_ind_unclosed2: 0,
            b_ind_unclosed_noignore: 0,
            b_ind_unclosed_wrapped: 0,
            b_ind_unclosed_whiteok: 0,
            b_ind_matching_paren: 0,
            b_ind_paren_prev: 0,
            b_ind_maxparen: 0,
            b_ind_maxcomment: 0,
            b_ind_scopedecl: 0,
            b_ind_scopedecl_code: 0,
            b_ind_java: 0,
            b_ind_js: 0,
            b_ind_keep_case_label: 0,
            b_ind_hash_comment: 0,
            b_ind_cpp_namespace: 0,
            b_ind_if_for_while: 0,
            b_ind_cpp_extern_c: 0,
            b_ind_pragma: 0,
            b_no_eol_lnum: 0,
            b_start_eof: 0,
            b_start_eol: 0,
            b_start_ffc: 0,
            b_start_fenc: None,
            b_bad_char: 0,
            b_start_bomb: 0,
            b_bufvar: crate::eval::typval::ScopeDictDictItem::new(),
            b_may_swap: false,
            b_did_warn: false,
            b_help: false,
            b_spell: false,
            b_prompt_text: None,
            b_prompt_callback: crate::eval::typval::Callback::NONE,
            b_prompt_interrupt: crate::eval::typval::Callback::NONE,
            b_prompt_append_new_line: false,
            b_prompt_insert: 0,
            b_prompt_start: fmark_T::ZERO,
            b_s: synblock_T::new(),
            b_signcols: BufSigncols {
                max: 0,
                last_max: 0,
                count: [0; 9],
                autom: false,
            },
            terminal: ptr::null_mut(),
            additional_data: None,
            b_mapped_ctrl_c: 0,
            b_marktree: crate::marktree::MarkTree::new(),
            b_extmark_ns: std::collections::BTreeMap::new(),
            b_prev_line_count: 0,
            update_channels: Vec::new(),
            update_callbacks: Vec::new(),
            update_need_codepoints: false,
            deleted_bytes: 0,
            deleted_bytes2: 0,
            deleted_codepoints: 0,
            deleted_codeunits: 0,
            flush_count: 0,
            b_localdir: None,
            b_prevdir: None,
        }
    }

    pub fn alloc() -> *mut Self {
        Box::into_raw(Box::new(Self::new()))
    }

    #[inline]
    pub const fn b_vars(&self) -> *mut crate::eval::typval::dict_T {
        self.b_bufvar.di_tv.vval.v_dict()
    }
}

pub const GETF_SETMARK: c_int = 0x01;
pub const GETF_ALT: c_int = 0x02;
pub const GETF_SWITCH: c_int = 0x04;

pub const GETFILE_ERROR: c_int = 1;
pub const GETFILE_NOT_WRITTEN: c_int = 2;
pub const GETFILE_SAME_FILE: c_int = 0;
pub const GETFILE_OPEN_OTHER: c_int = -1;
pub const GETFILE_UNUSED: c_int = 8;

pub const BLN_CURBUF: c_int = 1;
pub const BLN_LISTED: c_int = 2;
pub const BLN_DUMMY: c_int = 4;
pub const BLN_NEW: c_int = 8;
pub const BLN_NOOPT: c_int = 16;
pub const BLN_NOCURWIN: c_int = 128;

pub const DOBUF_GOTO: c_int = 0;
pub const DOBUF_SPLIT: c_int = 1;
pub const DOBUF_UNLOAD: c_int = 2;
pub const DOBUF_DEL: c_int = 3;
pub const DOBUF_WIPE: c_int = 4;

pub const DOBUF_CURRENT: c_int = 0;
pub const DOBUF_FIRST: c_int = 1;
pub const DOBUF_LAST: c_int = 2;
pub const DOBUF_MOD: c_int = 3;

pub const DOBUF_FORCEIT: c_int = 1;
pub const DOBUF_SKIPHELP: c_int = 4;

pub const BFA_DEL: c_int = 1;
pub const BFA_WIPE: c_int = 2;
pub const BFA_KEEP_UNDO: c_int = 4;
pub const BFA_IGNORE_ABORT: c_int = 8;

const kBffClearWinInfo: c_int = 1;
const kBffInitChangedtick: c_int = 2;

pub(crate) const NOTDONE: c_int = 2;
pub(crate) const MAXLNUM: linenr_T = 0x7fffffff;
pub(crate) const NMARKS: usize = 26;
pub(crate) const JUMPLISTSIZE: usize = 100;
pub(crate) const DO_NOT_FREE_CNT: c_int = c_int::MAX / 2;

pub(crate) const BF_CHECK_RO: c_int = 0x02;
pub(crate) const BF_NEVERLOADED: c_int = 0x04;
pub(crate) const BF_NOTEDITED: c_int = 0x08;
pub(crate) const BF_NEW: c_int = 0x10;
pub(crate) const BF_READERR: c_int = 0x40;
pub(crate) const BF_WRITE_MASK: c_int = BF_NOTEDITED + BF_NEW + BF_READERR;

pub(crate) const VALID_TOPLINE: c_int = 0x80;
pub(crate) const MF_DIRTY_YES: c_int = 1;
pub(crate) const MF_DIRTY_YES_NOSYNC: c_int = 2;

pub(crate) const kShmRo: c_int = b'r' as c_int;
const kShmMod: c_int = b'm' as c_int;
pub(crate) const kShmFileinfo: c_int = b'F' as c_int;
const kCpoIntmod: c_int = b'i' as c_int;

pub(crate) const kMarkAdjustNormal: c_int = 0;
pub(crate) const kExtmarkNoUndo: c_int = 2;

pub(crate) const SEA_NONE: c_int = 0;
pub(crate) const SEA_DIALOG: c_int = 1;
pub(crate) const SEA_QUIT: c_int = 2;
pub(crate) const SEA_RECOVER: c_int = 3;

pub(crate) const kOptJopFlagView: c_uint = 0x02;
const kOptJopFlagClean: c_uint = 0x04;

const kOptSwbFlagSplit: c_uint = 0x04;
pub(crate) const kOptSwbFlagNewtab: c_uint = 0x08;
pub(crate) const kOptSwbFlagVsplit: c_uint = 0x10;

pub(crate) const kCdCauseBuffer: c_int = 2;
pub(crate) const kCdCauseAuto: c_int = 3;

pub(crate) const BCO_ALWAYS: c_int = 2;

pub(crate) const KEYMAP_INIT: i16 = 1;

pub(crate) const VAR_FIXED: c_int = 2;
pub(crate) const VAR_SCOPE: c_int = 1;

pub(crate) const NO_LOCAL_UNDOLEVEL: OptInt = -123456;

pub(crate) const kUIMessages: UIExtension = 4;

pub(crate) const STL_IN_ICON: c_int = 1;
pub(crate) const STL_IN_TITLE: c_int = 2;
const kOptIconstring: OptIndex = 139;
const kOptTitlestring: OptIndex = 331;

pub(crate) const SID_MODELINE: c_int = -1;
pub(crate) const OPT_MODELINE: c_int = 0x04;

use crate::charset::{NL, NUL};
use crate::os::fs::MAXPATHL;
use crate::search::FORWARD;

#[repr(C)]
struct bufmatch_T {
    buf: *mut buf_T,
    match_: std::ffi::CString,
}

struct BufferModuleState {
    buf_free_count: c_int,
    top_file_num: c_int,
    no_position: fmark_T,
    lasttitle: Option<std::ffi::CString>,
    lasticon: Option<std::ffi::CString>,
    modelines_entered: c_int,
}

impl BufferModuleState {
    const fn new() -> Self {
        Self {
            buf_free_count: 0,
            top_file_num: 1,
            no_position: fmark_T { mark: pos_T { lnum: 1, col: 0, coladd: 0 }, fnum: 0, timestamp: 0, view: INIT_FMARKV, additional_data: None },
            lasttitle: None,
            lasticon: None,
            modelines_entered: 0,
        }
    }
}

static BUF_STATE: GlobalState<BufferModuleState> = GlobalState::new(BufferModuleState::new());

#[inline]
fn buf_st() -> &'static mut BufferModuleState {
    unsafe { &mut *BUF_STATE.as_ptr() }
}

const E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR: &CStr =
    c"E937: Attempt to delete a buffer that is in use: %s";

#[inline]
fn buf_is_changed(buf: &buf_T) -> bool {
    !bt_dontwrite(&raw const *buf) && (buf.b_changed != 0 || file_ff_differs((&raw const *buf).cast_mut(), true))
}

#[inline]
fn curbuf_is_changed() -> bool {
    unsafe { buf_is_changed(&*g_st().curbuf) }
}

#[inline]
pub fn buf_get_changedtick(buf: &buf_T) -> varnumber_T {
    buf.changedtick_di.di_tv.vval.v_number()
}

#[inline]
pub fn buf_meta_total(b: &buf_T, m: usize) -> u32 {
    b.b_marktree.meta_root[m]
}

#[inline]
pub fn first_win_in_tab(tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        if tp == g_st().curtab {
            g_st().firstwin
        } else {
            (*tp).tp_firstwin
        }
    }
}

#[inline]
pub fn modifiable(buf: &buf_T) -> bool {
    buf.b_p_ma != 0 && buf.b_ro_locked == 0
}

fn trigger_undo_ftplugin(buf: &mut buf_T, win: &mut win_T) {
    window_layout_lock();
        buf.b_locked += 1;
        win.w_locked += 1;
        do_cmdline_cmd(c"if exists('b:undo_ftplugin') | exe b:undo_ftplugin | endif".as_ptr());
        buf.b_locked -= 1;
        win.w_locked -= 1;
        window_layout_unlock();
}

/// Calculate the percentage that `part` is of the `whole`.
pub fn calc_percentage(part: i64, whole: i64) -> c_int {
    if part > 1_000_000 {
        (part / (whole / 100)) as c_int
    } else {
        ((part * 100) / whole) as c_int
    }
}

/// Return the highest possible buffer number.
pub fn get_highest_fnum() -> c_int {
    buf_st().top_file_num - 1
}

/// Read data from buffer for retrying.
fn read_buffer(read_stdin: bool, eap: *mut exarg_T, flags: c_int) -> c_int {
    unsafe {
        let silent = shortmess(kShmFileinfo);

        let mut line_count = (*g_st().curbuf).b_ml.ml_line_count;
        let mut retval = readfile(
            if read_stdin {
                ptr::null_mut()
            } else {
                (*g_st().curbuf).b_ffname
            },
            if read_stdin {
                ptr::null_mut()
            } else {
                (*g_st().curbuf).b_fname
            },
            line_count,
            0,
            MAXLNUM,
            eap,
            flags | READ_BUFFER,
            silent,
        );
        if retval == OK {
            while line_count > 0 {
                line_count -= 1;
                ml_delete(1);
            }
        } else {
            while (*g_st().curbuf).b_ml.ml_line_count > line_count {
                ml_delete(line_count);
            }
        }
        (**curwin.as_ptr()).w_cursor.lnum = 1;
        (**curwin.as_ptr()).w_cursor.col = 0;

        if read_stdin {
            if !g_st().readonlymode && !buf_is_empty(&mut *g_st().curbuf) {
                changed(g_st().curbuf);
            } else if retval != FAIL {
                unchanged(g_st().curbuf, false, true);
            }

            apply_autocmds_retval(
                EVENT_STDINREADPOST,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
                &mut retval,
            );
        }
        retval
    }
}

/// Ensure buffer `buf` is loaded.
pub fn buf_ensure_loaded(buf: &mut buf_T) -> bool {
    if buf.b_ml.ml_mfp.is_some() {
            return true;
        }

        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, &raw mut *buf);
        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), &raw mut *buf, 0);
        let status = open_buffer(false, ptr::null_mut(), 0);
        ctx_restore(&mut aco);
        status != FAIL && bufref_valid(Some(&bufref)) && buf.b_ml.ml_mfp.is_some()
}

/// Open current buffer, that is: open the memfile and read the file into
/// memory.
pub fn open_buffer(read_stdin: bool, eap: *mut exarg_T, flags_arg: c_int) -> c_int {
    unsafe {
        let mut flags = flags_arg;
        let mut retval = OK;
        let mut old_curbuf: bufref_T = bufref_T::new();
        let old_tw = (*g_st().curbuf).b_p_tw;
        #[cfg(unix)]
        let mut read_fifo = false;
        #[cfg(not(unix))]
        let read_fifo = false;
        let silent = shortmess(kShmFileinfo);

        if g_st().readonlymode
            && !(*g_st().curbuf).b_ffname.is_null()
            && ((*g_st().curbuf).b_flags & BF_NEVERLOADED) != 0
        {
            (*g_st().curbuf).b_p_ro = 1;
        }

        if ml_open(&mut *g_st().curbuf) == FAIL {
            close_buffer(*curwin.as_ptr(), g_st().curbuf, 0, false, false, false);

            g_st().curbuf = ptr::null_mut();
            let mut buf = g_st().firstbuf;
            while !buf.is_null() {
                if (*buf).b_ml.ml_mfp.is_some() {
                    g_st().curbuf = buf;
                    break;
                }
                buf = (*buf).b_next;
            }

            if g_st().curbuf.is_null() {
                emsg(gettext(
                    c"E82: Cannot allocate any buffer, exiting...".as_ptr(),
                ));
                g_st().v_dying = 2;
                getout(2);
            }

            emsg(gettext(
                c"E83: Cannot allocate buffer, using other one...".as_ptr(),
            ));
            enter_buffer(g_st().curbuf);
            if old_tw != (*g_st().curbuf).b_p_tw {
                check_colorcolumn(ptr::null_mut(), *curwin.as_ptr());
            }
            return FAIL;
        }

        if (*g_st().curbuf).b_ml.ml_mfp.is_some() {
            (*(*g_st().curbuf).b_ml.mfp_ptr()).mf_dirty = MF_DIRTY_YES_NOSYNC;
        }

        set_bufref(&mut old_curbuf, g_st().curbuf);
        (*g_st().curbuf).b_modified_was_set = false;

        (**curwin.as_ptr()).w_valid = 0;

        if bt_nofileread(g_st().curbuf) {
            flags |= READ_NOFILE;
        }

        if !(*g_st().curbuf).b_ffname.is_null() {
            #[cfg(unix)]
            let save_bin = (*g_st().curbuf).b_p_bin;
            #[cfg(unix)]
            {
                const S_IFMT: i32 = 0o170000;
                const S_IFIFO: i32 = 0o010000;
                const S_IFSOCK: i32 = 0o140000;
                let perm = os_getperm((*g_st().curbuf).b_ffname);
                if perm >= 0 {
                    let fmt = perm & S_IFMT;
                    #[cfg(any(
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                        target_os = "dragonfly",
                        target_os = "macos"
                    ))]
                    let is_chr_fd = fmt == 0o020000 && is_dev_fd_file((*g_st().curbuf).b_ffname);
                    #[cfg(not(any(
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                        target_os = "dragonfly",
                        target_os = "macos"
                    )))]
                    let is_chr_fd = false;

                    if fmt == S_IFIFO || fmt == S_IFSOCK || is_chr_fd {
                        read_fifo = true;
                    }
                }
                if read_fifo {
                    (*g_st().curbuf).b_p_bin = 1;
                }
            }

            retval = readfile(
                (*g_st().curbuf).b_ffname,
                (*g_st().curbuf).b_fname,
                0,
                0,
                MAXLNUM,
                eap,
                flags | READ_NEW | if read_fifo { READ_FIFO } else { 0 },
                silent,
            );
            #[cfg(unix)]
            if read_fifo {
                (*g_st().curbuf).b_p_bin = save_bin;
                if retval == OK {
                    retval = read_buffer(false, eap, flags);
                }
            }

            if bt_help(g_st().curbuf) {
                get_local_additions();
            }
        } else if read_stdin {
            let save_bin = (*g_st().curbuf).b_p_bin;

            (*g_st().curbuf).b_p_bin = 1;
            retval = readfile(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                ptr::null_mut(),
                flags | (READ_NEW + READ_STDIN),
                silent,
            );
            (*g_st().curbuf).b_p_bin = save_bin;
            if retval == OK {
                retval = read_buffer(true, eap, flags);
            }
        }

        if (*g_st().curbuf).b_ml.ml_mfp.is_some() {
            let mfp = (*g_st().curbuf).b_ml.mfp_ptr();
            if (*mfp).mf_dirty == MF_DIRTY_YES_NOSYNC {
                (*mfp).mf_dirty = MF_DIRTY_YES;
            }
        }

        if ((*g_st().curbuf).b_flags & BF_NEVERLOADED) != 0 {
            buf_init_isk_chartab(&mut *g_st().curbuf);
            parse_cino(&mut *g_st().curbuf);
        }

        if (g_st().got_int && !vim_strchr(opt_vars().p_cpo, kCpoIntmod).is_null())
            || (*g_st().curbuf).b_modified_was_set
            || (aborting() && !vim_strchr(opt_vars().p_cpo, kCpoIntmod).is_null())
        {
            changed(g_st().curbuf);
        } else if retval != FAIL && !read_stdin && !read_fifo {
            unchanged(g_st().curbuf, false, true);
        }
        save_file_ff(&mut *g_st().curbuf);

        let tick = buf_get_changedtick(&*g_st().curbuf);
        (*g_st().curbuf).b_last_changedtick = tick;
        (*g_st().curbuf).b_last_changedtick_i = tick;
        (*g_st().curbuf).b_last_changedtick_pum = tick;

        if aborting() {
            (*g_st().curbuf).b_flags |= BF_READERR;
        }

        if retval == NOTDONE
            && *(*g_st().curbuf).b_p_ft == NUL
            && !(*g_st().curbuf).b_ffname.is_null()
            && after_pathsep(
                (*g_st().curbuf).b_ffname,
                (*g_st().curbuf).b_ffname.add(std::ffi::CStr::from_ptr((*g_st().curbuf).b_ffname).to_bytes().len()),
            ) != 0
            && augroup_exists(c"filetypedetect".as_ptr())
        {
            let mut bufref: bufref_T = bufref_T::new();
            set_bufref(&mut bufref, g_st().curbuf);
            do_doautocmd(
                c"filetypedetect BufRead".as_ptr().cast_mut(),
                false,
                None,
            );
            if !bufref_valid(Some(&bufref)) || g_st().curbuf != old_curbuf.br_buf || aborting() {
                return FAIL;
            }
        }

        foldUpdateAll(&mut **curwin.as_ptr());

        if ((**curwin.as_ptr()).w_valid & VALID_TOPLINE) == 0 {
            (**curwin.as_ptr()).w_topline = 1;
            (**curwin.as_ptr()).w_topfill = 0;
        }
        apply_autocmds_retval(
            EVENT_BUFENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
            &mut retval,
        );

        if retval == FAIL {
            return retval;
        }

        if bufref_valid(Some(&old_curbuf)) && (*old_curbuf.br_buf).b_ml.ml_mfp.is_some() {
            let mut aco: CtxSwitch = CtxSwitch::new();

            ctx_switch(
                &mut aco,
                ptr::null_mut(),
                ptr::null_mut(),
                old_curbuf.br_buf,
                0,
            );
            do_modelines(0);
            (*g_st().curbuf).b_flags &= !(BF_CHECK_RO | BF_NEVERLOADED);

            if (flags & READ_NOWINENTER) == 0 {
                apply_autocmds_retval(
                    EVENT_BUFWINENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                    &mut retval,
                );
            }

            ctx_restore(&mut aco);
        }

        retval
    }
}

/// Store `buf` in `bufref` and set the free count.
pub fn set_bufref(bufref: &mut bufref_T, buf: *mut buf_T) {
    unsafe {
        bufref.br_buf = buf;
        bufref.br_fnum = if buf.is_null() { 0 } else { (*buf).b_fnum };
        bufref.br_buf_free_count = buf_st().buf_free_count;
    }
}

/// Return true if `bufref->br_buf` points to the same buffer as when
/// `set_bufref()` was called and it is a valid buffer.
pub fn bufref_valid(bufref: Option<&bufref_T>) -> bool {
    let bufref: *const bufref_T = bufref.map_or(std::ptr::null(), |p| &raw const *p);
    unsafe {
        if (*bufref).br_buf_free_count == buf_st().buf_free_count {
            true
        } else {
            buf_valid((*bufref).br_buf) && (*bufref).br_fnum == (*(*bufref).br_buf).b_fnum
        }
    }
}

/// Check that `buf` points to a valid buffer in the buffer list.
pub fn buf_valid(buf: *mut buf_T) -> bool {
    if buf.is_null() {
        return false;
    }
    unsafe {
        let mut bp = g_st().lastbuf;
        while !bp.is_null() {
            if bp == buf {
                return true;
            }
            bp = (*bp).b_prev;
        }
    }
    false
}

fn can_unload_buffer(buf: &mut buf_T) -> bool {
    unsafe {
        let mut can_unload = buf.b_locked == 0;

        if can_unload && g_st().updating_screen {
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    can_unload = false;
                    break;
                }
                wp = (*wp).w_next;
            }
        }
        if can_unload && buf.b_saving {
            can_unload = false;
        }

        if !can_unload {
            let fname = if !buf.b_fname.is_null() {
                buf.b_fname
            } else {
                buf.b_ffname
            };
            semsg!(
                gettext(E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR.as_ptr()),
                if !fname.is_null() {
                    fname
                } else {
                    c"[No Name]".as_ptr()
                },
            );
        }
        can_unload
    }
}

pub fn buf_close_terminal(buf: &mut buf_T) {
    debug_assert!(!buf.terminal.is_null());
        buf.b_locked += 1;
        terminal_close((&raw mut buf.terminal).cast::<*mut Terminal>(), -1);
        buf.b_locked -= 1;
}

/// Close the link to a buffer.
pub fn close_buffer(
    win: *mut win_T,
    buf: *mut buf_T,
    action: c_int,
    abort_if_last: bool,
    ignore_abort: bool,
    set_context: bool,
) -> bool {
    unsafe {
        let mut unload_buf = action != 0;
        let mut del_buf = action == DOBUF_DEL || action == DOBUF_WIPE;
        let mut wipe_buf = action == DOBUF_WIPE;

        let is_curwin = !(*curwin.as_ptr()).is_null() && *curwin.as_ptr() == win && (**curwin.as_ptr()).w_buffer == buf;
        let the_curtab = g_st().curtab;

        if (*buf).terminal.is_null() {
            let bh0 = *(*buf).b_p_bh;
            if bh0 == b'd' as c_char {
                del_buf = true;
                unload_buf = true;
            } else if bh0 == b'w' as c_char {
                del_buf = true;
                unload_buf = true;
                wipe_buf = true;
            } else if bh0 == b'u' as c_char {
                unload_buf = true;
            }
        }

        if !(*buf).terminal.is_null() && (unload_buf || del_buf || wipe_buf) {
            unload_buf = true;
            del_buf = true;
            wipe_buf = true;
        }

        if (del_buf || wipe_buf) && !can_unload_buffer(&mut *buf) {
            return false;
        }

        let mut win_valid = win_valid_any_tab(win);
        if set_context && win_valid && (*win).w_buffer == buf {
            if (*buf).b_nwindows == 1 {
                set_last_cursor(win);
            }
            buflist_setfpos(
                buf,
                win,
                if (*win).w_cursor.lnum == 1 {
                    0
                } else {
                    (*win).w_cursor.lnum
                },
                (*win).w_cursor.col,
                true,
            );
        }

        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, buf);

        if win_valid && (*win).w_buffer == buf && (*buf).b_nwindows == 1 {
            (*buf).b_locked += 1;
            (*buf).b_locked_split += 1;
            if apply_autocmds_win(
                EVENT_BUFWINLEAVE,
                (*buf).b_fname,
                (*buf).b_fname,
                false,
                buf,
                win,
            ) && !bufref_valid(Some(&bufref))
            {
                emsg(gettext(e_auabort.as_ptr()));
                return false;
            }
            (*buf).b_locked -= 1;
            (*buf).b_locked_split -= 1;
            if abort_if_last && one_window(win, ptr::null_mut()) {
                emsg(gettext(e_auabort.as_ptr()));
                return false;
            }

            if !unload_buf {
                (*buf).b_locked += 1;
                (*buf).b_locked_split += 1;
                if apply_autocmds(EVENT_BUFHIDDEN, (*buf).b_fname, (*buf).b_fname, false, buf)
                    && !bufref_valid(Some(&bufref))
                {
                    emsg(gettext(e_auabort.as_ptr()));
                    return false;
                }
                (*buf).b_locked -= 1;
                (*buf).b_locked_split -= 1;
                if abort_if_last && one_window(win, ptr::null_mut()) {
                    emsg(gettext(e_auabort.as_ptr()));
                    return false;
                }
            }
            if !ignore_abort && aborting() {
                return false;
            }
            win_valid = win_valid && win_valid_any_tab(win);
        }

        if is_curwin && *curwin.as_ptr() != win && win_valid {
            block_autocmds();
            goto_tabpage_win(&mut *the_curtab, win);
            unblock_autocmds();
        }

        if bt_quickfix(buf) && win_valid && (*win).w_buffer == buf {
            (*win).w_onebuf_opt.wo_wfh = 0;
        }

        let hiding_buf = (*buf).b_nwindows <= 0
            || (win_valid && (*win).w_buffer == buf && (*buf).b_nwindows == 1);

        if diffopt_hiddenoff() && !unload_buf && hiding_buf {
            diff_buf_delete(buf);
        }

        if !hiding_buf || !unload_buf {
            return false;
        }

        if (*buf).b_ffname.is_null() {
            del_buf = true;
        }

        if !buf_freeall(
            buf,
            (if del_buf { BFA_DEL } else { 0 })
                + (if wipe_buf { BFA_WIPE } else { 0 })
                + (if ignore_abort { BFA_IGNORE_ABORT } else { 0 }),
        ) {
            return false;
        }

        let mut clear_w_buf = false;
        win_valid = win_valid && win_valid_any_tab(win);
        if win_valid && (*win).w_buffer == buf {
            (*buf).b_nwindows -= 1;
            clear_w_buf = true;
        }

        if wipe_buf
            && (*buf).b_nwindows <= 0
            && (!(*buf).b_prev.is_null() || !(*buf).b_next.is_null() || mem_st().entered_free_all_mem)
        {
            if clear_w_buf {
                (*win).w_buffer = ptr::null_mut();
            }
            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == g_st().curtab {
                    g_st().firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    mark_forget_file(&mut *wp, (*buf).b_fnum);
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
            (*buf).clear_fnames();
            if (*buf).b_prev.is_null() {
                g_st().firstbuf = (*buf).b_next;
            } else {
                (*(*buf).b_prev).b_next = (*buf).b_next;
            }
            if (*buf).b_next.is_null() {
                g_st().lastbuf = (*buf).b_prev;
            } else {
                (*(*buf).b_next).b_prev = (*buf).b_prev;
            }
            free_buffer(buf);
        } else {
            if del_buf {
                free_buffer_stuff(buf, kBffClearWinInfo | kBffInitChangedtick);
                (*buf).b_flags = BF_CHECK_RO | BF_NEVERLOADED;
                (*buf).b_p_initialized = false;
            }
            g_st().textlock += 1;
            buf_clear_file(&mut *buf);
            g_st().textlock -= 1;
            if clear_w_buf {
                (*win).w_buffer = ptr::null_mut();
            }
            if del_buf {
                (*buf).b_p_bl = 0;
            }
        }

        debug_assert!(!win_valid || !is_curwin || win == *curwin.as_ptr());
        true
    }
}

/// Make buffer not contain a file.
pub fn buf_clear_file(buf: &mut buf_T) {
    buf.b_ml.ml_line_count = 1;
        unchanged(buf, true, true);
        buf.b_p_eof = 0;
        buf.b_start_eof = 0;
        buf.b_p_eol = 1;
        buf.b_start_eol = 1;
        buf.b_p_bomb = 0;
        buf.b_start_bomb = 0;
        buf.b_ml.ml_mfp = None;
        buf.b_ml.ml_flags = ML_EMPTY;
}

/// Clears the current buffer contents.
pub fn buf_clear() {
    unsafe {
        let line_count = (*g_st().curbuf).b_ml.ml_line_count;
        extmark_free_all(&mut *g_st().curbuf);
        mc_buf_free(g_st().curbuf);
        while ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
            ml_delete(1);
        }
        deleted_lines_mark(1, line_count);
    }
}

/// Free all things allocated for a buffer that are related to the file.
pub fn buf_freeall(buf: *mut buf_T, flags: c_int) -> bool {
    unsafe {
        let is_curbuf = buf == g_st().curbuf;
        let is_curwin = !(*curwin.as_ptr()).is_null() && (**curwin.as_ptr()).w_buffer == buf;
        let the_curwin = *curwin.as_ptr();
        let the_curtab = g_st().curtab;

        (*buf).b_locked += 1;
        (*buf).b_locked_split += 1;

        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, buf);

        if !(*buf).terminal.is_null() {
            buf_close_terminal(&mut *buf);
        }

        buf_updates_unload(&mut *buf, false);

        if (*buf).b_ml.ml_mfp.is_some()
            && apply_autocmds(EVENT_BUFUNLOAD, (*buf).b_fname, (*buf).b_fname, false, buf)
            && !bufref_valid(Some(&bufref))
        {
            return false;
        }
        if (flags & BFA_DEL) != 0
            && (*buf).b_p_bl != 0
            && apply_autocmds(EVENT_BUFDELETE, (*buf).b_fname, (*buf).b_fname, false, buf)
            && !bufref_valid(Some(&bufref))
        {
            return false;
        }
        if (flags & BFA_WIPE) != 0
            && apply_autocmds(EVENT_BUFWIPEOUT, (*buf).b_fname, (*buf).b_fname, false, buf)
            && !bufref_valid(Some(&bufref))
        {
            return false;
        }
        (*buf).b_locked -= 1;
        (*buf).b_locked_split -= 1;

        if is_curwin && *curwin.as_ptr() != the_curwin && win_valid_any_tab(the_curwin) {
            block_autocmds();
            goto_tabpage_win(&mut *the_curtab, the_curwin);
            unblock_autocmds();
        }
        if (flags & BFA_IGNORE_ABORT) == 0 && aborting() {
            return false;
        }

        if buf == g_st().curbuf && !is_curbuf {
            return false;
        }

        if buf == g_st().curbuf && g_st().Visual.active && !mem_st().entered_free_all_mem {
            end_visual_mode();
        }

        diff_buf_delete(buf);
        if !(*curwin.as_ptr()).is_null() && (**curwin.as_ptr()).w_buffer == buf {
            reset_synblock(&mut **curwin.as_ptr());
        }

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut win = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !win.is_null() {
                if (*win).w_buffer == buf {
                    clearFolding(&mut *win);
                }
                win = (*win).w_next;
            }
            tp = (*tp).tp_next;
        }

        if !(*buf).terminal.is_null() {
            block_autocmds();
            buf_close_terminal(&mut *buf);
            unblock_autocmds();
        }

        let count = (*buf).b_ml.ml_line_count;
        ml_close(buf, 1);
        (*buf).b_ml.ml_line_count = 0;
        mc_buf_clear(buf);

        if bt_nofilename(buf) && !g_st().exiting {
            mark_adjust_buf(
                &mut *buf,
                1,
                count,
                MAXLNUM,
                -count,
                false,
                kMarkAdjustNormal,
                kExtmarkNoUndo,
            );
        }

        if (flags & BFA_KEEP_UNDO) == 0 {
            u_clearallandblockfree(&mut *buf);
        }
        syntax_clear(&mut (*buf).b_s);
        (*buf).b_flags &= !BF_READERR;
        true
    }
}

impl buf_T {
    #[inline]
    pub fn set_ffname(&mut self, ffname: Option<std::ffi::CString>) {
        self.b_ffname = ffname
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        self.b_ffname_cstr = ffname;
    }

    #[inline]
    pub fn set_sfname(&mut self, sfname: Option<std::ffi::CString>) {
        self.b_sfname = sfname
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        self.b_sfname_cstr = sfname;
    }

    #[inline]
    pub fn clear_fnames(&mut self) {
        self.b_ffname_cstr = None;
        self.b_sfname_cstr = None;
        self.b_ffname = ptr::null_mut();
        self.b_sfname = ptr::null_mut();
    }

    #[inline]
    pub fn take_fnames(&mut self) -> (Option<std::ffi::CString>, Option<std::ffi::CString>) {
        self.b_ffname = ptr::null_mut();
        self.b_sfname = ptr::null_mut();
        (self.b_ffname_cstr.take(), self.b_sfname_cstr.take())
    }

    #[inline]
    pub fn restore_fnames(
        &mut self,
        ffname: Option<std::ffi::CString>,
        sfname: Option<std::ffi::CString>,
    ) {
        self.set_ffname(ffname);
        self.set_sfname(sfname);
    }

    #[inline]
    pub fn swap_fnames(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.b_fname, &mut other.b_fname);
        std::mem::swap(&mut self.b_ffname, &mut other.b_ffname);
        std::mem::swap(&mut self.b_sfname, &mut other.b_sfname);
        std::mem::swap(&mut self.b_ffname_cstr, &mut other.b_ffname_cstr);
        std::mem::swap(&mut self.b_sfname_cstr, &mut other.b_sfname_cstr);
    }
}

impl Drop for buf_T {
    fn drop(&mut self) {
        self.clear_fnames();
            free_buf_options(self, true);
            self.b_start_fenc = None;
            self.b_localdir = None;
            self.b_prevdir = None;
            self.additional_data = None;
            self.b_prompt_text = None;
            callback_free(&mut self.b_prompt_callback);
            callback_free(&mut self.b_prompt_interrupt);
            clear_fmark(&mut self.b_last_cursor, 0);
            clear_fmark(&mut self.b_last_insert, 0);
            clear_fmark(&mut self.b_last_change, 0);
            clear_fmark(&mut self.b_prompt_start, 0);
            self.b_changelistlen = 0;
    }
}

fn free_buffer(buf: *mut buf_T) {
    unsafe {
        let map_ptr = &raw mut g_st().buffer_handles;
        (*map_ptr).remove(&(*buf).b_fnum);
        buf_st().buf_free_count += 1;
        free_buffer_stuff(buf, kBffClearWinInfo);
        if (*(*buf).b_vars()).dv_refcount > DO_NOT_FREE_CNT {
            tv_dict_add(
                (*buf).b_vars(),
                tv_dict_item_copy(&mut *(&raw mut (*buf).changedtick_di).cast()),
            );
        }
        unref_var_dict(&mut *(*buf).b_vars());
        aubuflocal_remove(buf);
        if g_st().autocmd_busy {
            (*buf).additional_data = None;
            (*buf).b_prompt_text = None;
            callback_free(&mut (*buf).b_prompt_callback);
            callback_free(&mut (*buf).b_prompt_interrupt);
            clear_fmark(&mut (*buf).b_last_cursor, 0);
            clear_fmark(&mut (*buf).b_last_insert, 0);
            clear_fmark(&mut (*buf).b_last_change, 0);
            clear_fmark(&mut (*buf).b_prompt_start, 0);
            (*buf).b_changelistlen = 0;
            (*buf).b_namedm.fill(fmark_T::new());
            (*buf).b_changelist.fill(fmark_T::new());
            let pending_buf = &raw mut g_st().au_pending_free_buf;
            (*pending_buf).push(Box::from_raw(buf));
        } else {
            drop(Box::from_raw(buf));
            if g_st().curbuf == buf {
                g_st().curbuf = ptr::null_mut();
            }
        }
    }
}

fn clear_wininfo(buf: &mut buf_T) {
    buf.b_wininfo.clear();
}

fn free_buffer_stuff(buf: *mut buf_T, free_flags: c_int) {
    unsafe {
        if (free_flags & kBffClearWinInfo) != 0 {
            clear_wininfo(&mut *buf);
            free_buf_options(buf, true);
            (*buf).b_s.b_langp = Vec::new();
        }
        {
            let changedtick_hi = hash_find(
                &(*(*buf).b_vars()).dv_hashtab,
                c"changedtick".as_ptr(),
            );
            debug_assert!(!changedtick_hi.is_null());
            hash_remove(&mut (*(*buf).b_vars()).dv_hashtab, changedtick_hi);
        }
        vars_clear(&mut (*(*buf).b_vars()).dv_hashtab);
        if (free_flags & kBffInitChangedtick) != 0 {
            buf_init_changedtick(&mut *buf);
        }
        (*buf).b_ucmds.clear();
        extmark_free_all(&mut *buf);
        mc_buf_free(buf);
        map_clear_mode(buf, MAP_ALL_MODES, true, false);
        map_clear_mode(buf, MAP_ALL_MODES, true, true);
        (*buf).b_start_fenc = None;
        (*buf).b_localdir = None;
        (*buf).b_prevdir = None;

        buf_free_callbacks(&mut *buf);
    }
}

/// Go to another buffer. Handles the result of the ATTENTION dialog.
pub fn goto_buffer(eap: &mut exarg_T, start: c_int, dir: c_int, count: c_int) {
    unsafe {
        let save_sea = g_st().swap_exists_action;
        let skip_help_buf = matches!(
            cmdidx_T::from_raw(eap.cmdidx),
            cmdidx_T::CMD_bnext
                | cmdidx_T::CMD_sbnext
                | cmdidx_T::CMD_bNext
                | cmdidx_T::CMD_bprevious
                | cmdidx_T::CMD_sbNext
                | cmdidx_T::CMD_sbprevious
        );

        let mut old_curbuf: bufref_T = bufref_T::new();
        set_bufref(&mut old_curbuf, g_st().curbuf);

        if g_st().swap_exists_action == SEA_NONE {
            g_st().swap_exists_action = SEA_DIALOG;
        }
        let _ = do_buffer_ext(
            if *eap.cmd == b's' as c_char {
                DOBUF_SPLIT
            } else {
                DOBUF_GOTO
            },
            start,
            dir,
            count,
            (if eap.forceit != 0 {
                DOBUF_FORCEIT
            } else {
                0
            }) | (if skip_help_buf { DOBUF_SKIPHELP } else { 0 }),
        );

        if g_st().swap_exists_action == SEA_QUIT && *eap.cmd == b's' as c_char {
            let mut cs: cleanup_T = cleanup_T::new();
            enter_cleanup(&mut cs);
            win_close(*curwin.as_ptr(), true, false);
            g_st().swap_exists_action = save_sea;
            g_st().swap_exists_did_quit = true;
            leave_cleanup(&mut cs);
        } else {
            handle_swap_exists(Some(&mut old_curbuf));
        }
    }
}

/// Handle the situation of `swap_exists_action` being set.
pub fn handle_swap_exists(old_curbuf: Option<&mut bufref_T>) {
    let old_curbuf: *mut bufref_T = old_curbuf.map_or(std::ptr::null_mut(), |p| &raw mut *p);
    unsafe {
        let mut cs: cleanup_T = cleanup_T::new();
        let old_tw = (*g_st().curbuf).b_p_tw;

        if g_st().swap_exists_action == SEA_QUIT {
            enter_cleanup(&mut cs);

            g_st().swap_exists_action = SEA_NONE;
            g_st().swap_exists_did_quit = true;
            close_buffer(*curwin.as_ptr(), g_st().curbuf, DOBUF_UNLOAD, false, false, true);
            let buf = if old_curbuf.is_null()
                || !bufref_valid((!old_curbuf.is_null()).then(||  { &*old_curbuf }))
                || (*old_curbuf).br_buf == g_st().curbuf
            {
                block_autocmds();
                let b = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_CURBUF | BLN_LISTED);
                unblock_autocmds();
                b
            } else {
                (*old_curbuf).br_buf
            };
            if !buf.is_null() {
                enter_buffer(buf);
                if old_tw != (*g_st().curbuf).b_p_tw {
                    check_colorcolumn(ptr::null_mut(), *curwin.as_ptr());
                }
            }

            leave_cleanup(&mut cs);
        } else if g_st().swap_exists_action == SEA_RECOVER {
            enter_cleanup(&mut cs);

            g_st().msg_scroll = 1;
            ml_recover(false);
            msg_puts(c"\n".as_ptr());
            g_st().cmdline_row = g_st().msg_row;
            do_modelines(0);

            leave_cleanup(&mut cs);
        }
        g_st().swap_exists_action = SEA_NONE;
    }
}

/// Delete or unload buffer(s).
pub fn do_bufdel(
    command: c_int,
    mut arg: *mut c_char,
    addr_count: c_int,
    start_bnr: c_int,
    end_bnr: c_int,
    forceit: c_int,
) -> *mut c_char {
    unsafe {
        let mut do_current = 0;
        let mut deleted: c_int = 0;
        let mut errormsg: *mut c_char = ptr::null_mut();
        let mut bnr: c_int;

        if addr_count == 0 {
            do_buffer(command, DOBUF_CURRENT, FORWARD, 0, forceit);
        } else {
            if addr_count == 2 {
                if *arg != NUL {
                    return ex_errmsg(e_trailing_arg.as_ptr(), arg);
                }
                bnr = start_bnr;
            } else {
                bnr = end_bnr;
            }

            while !g_st().got_int {
                if bnr == (*g_st().curbuf).b_fnum {
                    do_current = bnr;
                } else if do_buffer(command, DOBUF_FIRST, FORWARD, bnr, forceit) == OK {
                    deleted += 1;
                }

                if addr_count == 2 {
                    bnr += 1;
                    if bnr > end_bnr {
                        break;
                    }
                } else {
                    arg = skipwhite(arg);
                    if *arg == NUL {
                        break;
                    }
                    if !ascii_isdigit(*arg as c_int) {
                        let p = skiptowhite_esc(arg);
                        bnr = buflist_findpat(arg, p, command == DOBUF_WIPE, false, false);
                        if bnr < 0 {
                            break;
                        }
                        arg = p;
                    } else {
                        bnr = getdigits_int(&raw mut arg, false, 0);
                    }
                }
                os_breakcheck();
            }
            if !g_st().got_int
                && do_current != 0
                && do_buffer(command, DOBUF_FIRST, FORWARD, do_current, forceit) == OK
            {
                deleted += 1;
            }

            if deleted == 0 {
                let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                if command == DOBUF_UNLOAD {
                    xstrlcpy(
                        iobuff_ptr,
                        gettext(c"E515: No buffers were unloaded".as_ptr()),
                        IOSIZE,
                    );
                } else if command == DOBUF_DEL {
                    xstrlcpy(
                        iobuff_ptr,
                        gettext(c"E516: No buffers were deleted".as_ptr()),
                        IOSIZE,
                    );
                } else {
                    xstrlcpy(
                        iobuff_ptr,
                        gettext(c"E517: No buffers were wiped out".as_ptr()),
                        IOSIZE,
                    );
                }
                errormsg = iobuff_ptr;
            } else if i64::from(deleted) >= opt_vars().p_report {
                if command == DOBUF_UNLOAD {
                    smsg!(
                        0,
                        ngettext(
                            c"%d buffer unloaded".as_ptr(),
                            c"%d buffers unloaded".as_ptr(),
                            deleted as usize,
                        ),
                        deleted,
                    );
                } else if command == DOBUF_DEL {
                    smsg!(
                        0,
                        ngettext(
                            c"%d buffer deleted".as_ptr(),
                            c"%d buffers deleted".as_ptr(),
                            deleted as usize,
                        ),
                        deleted,
                    );
                } else {
                    smsg!(
                        0,
                        ngettext(
                            c"%d buffer wiped out".as_ptr(),
                            c"%d buffers wiped out".as_ptr(),
                            deleted as usize,
                        ),
                        deleted,
                    );
                }
            }
        }

        errormsg
    }
}

fn empty_curbuf(close_others: bool, forceit: c_int, action: c_int) -> c_int {
    unsafe {
        let buf = g_st().curbuf;

        if action == DOBUF_UNLOAD {
            emsg(gettext(c"E90: Cannot unload last buffer".as_ptr()));
            return FAIL;
        }

        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, buf);

        if close_others {
            let mut can_close_all_others = true;
            if (**curwin.as_ptr()).w_floating {
                can_close_all_others = false;
                let mut wp = g_st().firstwin;
                while !(*wp).w_floating {
                    if (*wp).w_buffer != g_st().curbuf {
                        can_close_all_others = true;
                        break;
                    }
                    wp = (*wp).w_next;
                }
            }
            close_windows(buf, can_close_all_others);
        }

        setpcmark();
        let retval = do_ecmd(
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ECMD_ONE,
            if forceit != 0 { ECMD_FORCEIT } else { 0 },
            *curwin.as_ptr(),
        );

        if buf != g_st().curbuf && bufref_valid(Some(&bufref)) && (*buf).b_nwindows == 0 {
            close_buffer(ptr::null_mut(), buf, action, false, false, false);
        }

        if !close_others {
            g_st().need_fileinfo = false;
        } else if retval == OK && !shortmess(kShmFileinfo) {
            g_st().need_fileinfo = true;
        }

        retval
    }
}

fn do_buffer_ext(
    action: c_int,
    start: c_int,
    dir: c_int,
    mut count: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut buf: *mut buf_T;
        let mut bp: *mut buf_T;
        let mut update_jumplist = true;
        let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;

        buf = match start {
            DOBUF_FIRST => g_st().firstbuf,
            DOBUF_LAST => g_st().lastbuf,
            _ => g_st().curbuf,
        };
        if start == DOBUF_MOD {
            while count > 0 {
                count -= 1;
                loop {
                    buf = (*buf).b_next;
                    if buf.is_null() {
                        buf = g_st().firstbuf;
                    }
                    if buf == g_st().curbuf || buf_is_changed(&*buf) {
                        break;
                    }
                }
            }
            if !buf_is_changed(&*buf) {
                emsg(gettext(c"E84: No modified buffer found".as_ptr()));
                return FAIL;
            }
        } else if start == DOBUF_FIRST && count != 0 {
            while !buf.is_null() && (*buf).b_fnum != count {
                buf = (*buf).b_next;
            }
        } else {
            let help_only = (flags & DOBUF_SKIPHELP) != 0 && (*buf).b_help;

            bp = ptr::null_mut();
            while count > 0
                || (bp != buf
                    && !unload
                    && !(if help_only {
                        (*buf).b_help
                    } else {
                        (*buf).b_p_bl != 0
                    }))
            {
                if bp.is_null() {
                    bp = buf;
                }
                buf = if dir == FORWARD {
                    if !(*buf).b_next.is_null() {
                        (*buf).b_next
                    } else {
                        g_st().firstbuf
                    }
                } else if !(*buf).b_prev.is_null() {
                    (*buf).b_prev
                } else {
                    g_st().lastbuf
                };
                if unload
                    || (if help_only {
                        (*buf).b_help
                    } else {
                        (*buf).b_p_bl != 0 && ((flags & DOBUF_SKIPHELP) == 0 || !(*buf).b_help)
                    })
                {
                    count -= 1;
                    bp = ptr::null_mut();
                }
                if bp == buf {
                    emsg(gettext(c"E85: There is no listed buffer".as_ptr()));
                    return FAIL;
                }
            }
        }

        if buf.is_null() {
            if start == DOBUF_FIRST {
                if !unload {
                    semsg!(gettext(e_nobufnr.as_ptr()), i64::from(count));
                }
            } else if dir == FORWARD {
                emsg(gettext(c"E87: Cannot go beyond last buffer".as_ptr()));
            } else {
                emsg(gettext(c"E88: Cannot go before first buffer".as_ptr()));
            }
            return FAIL;
        }

        if action == DOBUF_GOTO
            && buf != g_st().curbuf
            && !check_can_set_curbuf_forceit(flags & DOBUF_FORCEIT)
        {
            return FAIL;
        }

        if (action == DOBUF_GOTO || action == DOBUF_SPLIT) && ((*buf).b_flags & BF_DUMMY) != 0 {
            semsg!(gettext(e_nobufnr.as_ptr()), count);
            return FAIL;
        }

        if unload {
            let mut bufref: bufref_T = bufref_T::new();
            if !can_unload_buffer(&mut *buf) {
                return FAIL;
            }
            set_bufref(&mut bufref, buf);

            if action != DOBUF_WIPE && (*buf).b_ml.ml_mfp.is_none() && (*buf).b_p_bl == 0 {
                return FAIL;
            }

            if (flags & DOBUF_FORCEIT) == 0 && buf_is_changed(&*buf) {
                if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && opt_vars().p_write != 0 {
                    dialog_changed(&mut *buf, false);
                    if !bufref_valid(Some(&bufref)) {
                        return FAIL;
                    }
                    if buf_is_changed(&*buf) {
                        return FAIL;
                    }
                } else {
                    semsg!(
                        gettext(
                            e_no_write_since_last_change_for_buffer_nr_add_bang_to_override
                                .as_ptr(),
                        ),
                        (*buf).b_fnum,
                    );
                    return FAIL;
                }
            }

            if (flags & DOBUF_FORCEIT) == 0
                && !(*buf).terminal.is_null()
                && terminal_running(&*(*buf).terminal)
            {
                if opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                    if !dialog_close_terminal(&mut *buf) {
                        return FAIL;
                    }
                } else {
                    semsg!(
                        gettext(c"E89: %s will be killed (add ! to override)".as_ptr()),
                        (*buf).b_fname,
                    );
                    return FAIL;
                }
            }

            let buf_fnum = (*buf).b_fnum;

            if buf == g_st().curbuf && g_st().Visual.active {
                end_visual_mode();
            }

            bp = ptr::null_mut();
            let mut bp2 = g_st().firstbuf;
            while !bp2.is_null() {
                if (*bp2).b_p_bl != 0 && bp2 != buf {
                    bp = bp2;
                    break;
                }
                bp2 = (*bp2).b_next;
            }
            if bp.is_null() && buf == g_st().curbuf {
                return empty_curbuf(true, flags & DOBUF_FORCEIT, action);
            }

            while buf == g_st().curbuf
                && !(win_locked(&mut **curwin.as_ptr()) != 0 || (*(**curwin.as_ptr()).w_buffer).b_locked > 0)
                && (is_ctx_win(g_st().lastwin) || !last_window(*curwin.as_ptr()))
            {
                if win_close(*curwin.as_ptr(), false, false) == FAIL {
                    break;
                }
            }

            if buf != g_st().curbuf {
                if (opt_vars().jop_flags & kOptJopFlagClean) != 0 {
                    mark_jumplist_forget_file(&mut **curwin.as_ptr(), buf_fnum);
                }

                close_windows(buf, false);
                if buf != g_st().curbuf
                    && bufref_valid(Some(&bufref))
                    && (*g_st().firstwin).w_buffer == buf
                    && one_window(g_st().firstwin, ptr::null_mut())
                {
                    debug_assert!((**curwin.as_ptr()).w_floating);
                    let mut switchwin: CtxSwitch = CtxSwitch::new();
                    let rv = ctx_switch(
                        &mut switchwin,
                        g_st().firstwin,
                        g_st().curtab,
                        ptr::null_mut(),
                        kCtxNoDisplay,
                    );
                    debug_assert!(rv);
                    do_buffer_ext(action, start, dir, count, flags);
                    ctx_restore(&mut switchwin);
                }

                if buf != g_st().curbuf && bufref_valid(Some(&bufref)) && (*buf).b_nwindows <= 0 {
                    close_buffer(ptr::null_mut(), buf, action, false, false, false);
                }
                return OK;
            }

            buf = ptr::null_mut();
            bp = ptr::null_mut();
            if !g_st().au_new_curbuf.br_buf.is_null()
                && bufref_valid(Some(&g_st().au_new_curbuf))
                && (*g_st().au_new_curbuf.br_buf).b_locked_split == 0
            {
                buf = g_st().au_new_curbuf.br_buf;
            } else if (**curwin.as_ptr()).w_jumplistlen > 0 {
                if (opt_vars().jop_flags & kOptJopFlagClean) != 0 {
                    mark_jumplist_forget_file(&mut **curwin.as_ptr(), buf_fnum);
                }

                if (**curwin.as_ptr()).w_jumplistlen > 0 {
                    let mut jumpidx = (**curwin.as_ptr()).w_jumplistidx;

                    if (opt_vars().jop_flags & kOptJopFlagClean) != 0 {
                        if jumpidx == (**curwin.as_ptr()).w_jumplistlen {
                            jumpidx = (**curwin.as_ptr()).w_jumplistlen - 1;
                            (**curwin.as_ptr()).w_jumplistidx = jumpidx;
                        }
                    } else {
                        jumpidx -= 1;
                        if jumpidx < 0 {
                            jumpidx = (**curwin.as_ptr()).w_jumplistlen - 1;
                        }
                    }

                    let forward = jumpidx;
                    while (opt_vars().jop_flags & kOptJopFlagClean) != 0 || jumpidx != (**curwin.as_ptr()).w_jumplistidx
                    {
                        buf = buflist_findnr((**curwin.as_ptr()).w_jumplist[jumpidx as usize].fmark.fnum);

                        if !buf.is_null() {
                            if buf == g_st().curbuf
                                || (*buf).b_p_bl == 0
                                || bt_quickfix(buf)
                                || (*buf).b_locked_split != 0
                            {
                                buf = ptr::null_mut();
                            } else if (*buf).b_ml.ml_mfp.is_none() {
                                if bp.is_null() {
                                    bp = buf;
                                }
                                buf = ptr::null_mut();
                            }
                        }
                        if !buf.is_null() {
                            if (opt_vars().jop_flags & kOptJopFlagClean) != 0 {
                                (**curwin.as_ptr()).w_jumplistidx = jumpidx;
                                update_jumplist = false;
                            }
                            break;
                        }
                        if jumpidx == 0 && (**curwin.as_ptr()).w_jumplistidx == (**curwin.as_ptr()).w_jumplistlen {
                            break;
                        }
                        jumpidx -= 1;
                        if jumpidx < 0 {
                            jumpidx = (**curwin.as_ptr()).w_jumplistlen - 1;
                        }
                        if jumpidx == forward {
                            break;
                        }
                    }
                }
            }

            if buf.is_null() {
                let mut forward = true;
                buf = (*g_st().curbuf).b_next;
                loop {
                    if buf.is_null() {
                        if !forward {
                            break;
                        }
                        buf = (*g_st().curbuf).b_prev;
                        forward = false;
                        continue;
                    }
                    if (*buf).b_help == (*g_st().curbuf).b_help
                        && (*buf).b_p_bl != 0
                        && !bt_quickfix(buf)
                        && (*buf).b_locked_split == 0
                    {
                        if (*buf).b_ml.ml_mfp.is_some() {
                            break;
                        }
                        if bp.is_null() {
                            bp = buf;
                        }
                    }
                    buf = if forward {
                        (*buf).b_next
                    } else {
                        (*buf).b_prev
                    };
                }
            }
            if buf.is_null() {
                buf = bp;
            }
            if buf.is_null() {
                let mut buf2 = g_st().firstbuf;
                while !buf2.is_null() {
                    if (*buf2).b_p_bl != 0
                        && buf2 != g_st().curbuf
                        && !bt_quickfix(buf2)
                        && (*buf2).b_locked_split == 0
                    {
                        buf = buf2;
                        break;
                    }
                    buf2 = (*buf2).b_next;
                }
            }
            if buf.is_null() {
                buf = if !(*g_st().curbuf).b_next.is_null() {
                    (*g_st().curbuf).b_next
                } else {
                    (*g_st().curbuf).b_prev
                };
                if bt_quickfix(buf)
                    || (!buf.is_null() && buf != g_st().curbuf && (*buf).b_locked_split != 0)
                {
                    buf = ptr::null_mut();
                }
            }
        }

        if buf.is_null() {
            return empty_curbuf(false, flags & DOBUF_FORCEIT, action);
        }

        if action == DOBUF_SPLIT && !swbuf_goto_win_with_buf(buf).is_null() {
            return OK;
        }
        if buf != g_st().curbuf && (*buf).b_locked_split != 0 {
            emsg(gettext(e_cannot_switch_to_a_closing_buffer.as_ptr()));
            return FAIL;
        }
        if action == DOBUF_SPLIT && win_split(0, 0) == FAIL {
            return FAIL;
        }

        if buf == g_st().curbuf {
            return OK;
        }

        if action == DOBUF_GOTO && !can_abandon(&mut *g_st().curbuf, (flags & DOBUF_FORCEIT) != 0) {
            if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && opt_vars().p_write != 0 {
                let mut bufref: bufref_T = bufref_T::new();
                set_bufref(&mut bufref, buf);
                dialog_changed(&mut *g_st().curbuf, false);
                if !bufref_valid(Some(&bufref)) {
                    return FAIL;
                }
            }
            if buf_is_changed(&*g_st().curbuf) {
                no_write_message();
                return FAIL;
            }
        }

        set_curbuf(buf, action, update_jumplist);

        if action == DOBUF_SPLIT {
            reset_binding(&mut **curwin.as_ptr());
        }

        if aborting() {
            return FAIL;
        }

        OK
    }
}

pub fn do_buffer(
    action: c_int,
    start: c_int,
    dir: c_int,
    count: c_int,
    forceit: c_int,
) -> c_int {
    do_buffer_ext(
            action,
            start,
            dir,
            count,
            if forceit != 0 { DOBUF_FORCEIT } else { 0 },
        )
}

/// Set current buffer to `buf`. Executes autocommands and closes current buffer.
pub fn set_curbuf(buf: *mut buf_T, action: c_int, update_jumplist: bool) {
    unsafe {
        let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;
        let old_tw = (*g_st().curbuf).b_p_tw;

        if update_jumplist {
            setpcmark();
        }

        if (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
            (**curwin.as_ptr()).w_alt_fnum = (*g_st().curbuf).b_fnum;
        }
        buflist_altfpos(&mut **curwin.as_ptr());

        g_st().Visual.reselect = 0;

        let prevbuf = g_st().curbuf;
        let mut newbufref: bufref_T = bufref_T::new();
        let mut prevbufref: bufref_T = bufref_T::new();
        set_bufref(&mut prevbufref, prevbuf);
        set_bufref(&mut newbufref, buf);

        if !apply_autocmds(
            EVENT_BUFLEAVE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        ) || (bufref_valid(Some(&prevbufref))
            && bufref_valid(Some(&newbufref))
            && !aborting())
        {
            if prevbuf == (**curwin.as_ptr()).w_buffer {
                reset_synblock(&mut **curwin.as_ptr());
            }
            if unload {
                close_windows(prevbuf, false);
            }
            if bufref_valid(Some(&prevbufref)) && !aborting() {
                if prevbuf == g_st().curbuf && ((g_st().State & MODE_INSERT) == 0 || (*g_st().curbuf).b_nwindows <= 1) {
                    u_sync(false);
                }
                close_buffer(
                    *curwin.as_ptr(),
                    prevbuf,
                    if unload {
                        action
                    } else if action == DOBUF_GOTO && !buf_hide(&*prevbuf) && !buf_is_changed(&*prevbuf)
                    {
                        DOBUF_UNLOAD
                    } else {
                        0
                    },
                    false,
                    false,
                    true,
                );
            }
        }

        let valid = buf_valid(buf);
        if (valid && buf != g_st().curbuf && !aborting()) || (**curwin.as_ptr()).w_buffer.is_null() {
            enter_buffer(if valid { buf } else { g_st().lastbuf });
            if old_tw != (*g_st().curbuf).b_p_tw {
                check_colorcolumn(ptr::null_mut(), *curwin.as_ptr());
            }
        }

        if bufref_valid(Some(&prevbufref)) && !(*prevbuf).terminal.is_null() {
            terminal_check_size(&mut *(*prevbuf).terminal);
        }

        update_cwd(kCdCauseBuffer);
    }
}

/// Enter a new current buffer.
fn enter_buffer(buf: *mut buf_T) {
    unsafe {
        if g_st().Visual.active && !mem_st().entered_free_all_mem {
            end_visual_mode();
        }

        if !(**curwin.as_ptr()).w_buffer.is_null() {
            (*(**curwin.as_ptr()).w_buffer).b_nwindows -= 1;
        }

        (**curwin.as_ptr()).w_buffer = buf;
        g_st().curbuf = buf;
        (*g_st().curbuf).b_nwindows += 1;

        buf_copy_options(buf, BCO_ENTER | BCO_NOHELP);
        if !(*buf).b_help {
            get_winopts(buf);
        } else {
            clearFolding(&mut **curwin.as_ptr());
        }
        foldUpdateAll(&mut **curwin.as_ptr());

        if (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0 {
            diff_buf_add(g_st().curbuf);
        }

        (**curwin.as_ptr()).w_s_owned = None;

        (**curwin.as_ptr()).w_cursor.lnum = 1;
        (**curwin.as_ptr()).w_cursor.col = 0;
        (**curwin.as_ptr()).w_cursor.coladd = 0;
        (**curwin.as_ptr()).w_set_curswant = true;
        (**curwin.as_ptr()).w_topline_was_set = false;

        (**curwin.as_ptr()).w_valid = 0;

        if (*g_st().curbuf).b_ml.ml_mfp.is_none() {
            if *(*g_st().curbuf).b_p_ft == NUL {
                (*g_st().curbuf).b_did_filetype = false;
            }
            open_buffer(false, ptr::null_mut(), 0);
        } else {
            if g_st().msg_silent == 0 && !shortmess(kShmFileinfo) {
                g_st().need_fileinfo = true;
            }
            buf_check_timestamp(&mut *g_st().curbuf);

            (**curwin.as_ptr()).w_topline = 1;
            (**curwin.as_ptr()).w_topfill = 0;
            apply_autocmds(
                EVENT_BUFENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            apply_autocmds(
                EVENT_BUFWINENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }

        if (**curwin.as_ptr()).w_cursor.lnum == 1 && inindent(0) {
            buflist_getfpos();
        }

        check_arg_idx(&mut **curwin.as_ptr());
        maketitle();
        win_float_update_preview(&mut **curwin.as_ptr());
        if (**curwin.as_ptr()).w_topline == 1 && !(**curwin.as_ptr()).w_topline_was_set {
            scroll_cursor_halfway(&mut **curwin.as_ptr(), false, false);
        }

        do_autochdir();

        if ((*g_st().curbuf).b_kmap_state & KEYMAP_INIT) != 0 {
            keymap_init();
        }
        if !(*g_st().curbuf).b_help
            && (**curwin.as_ptr()).w_onebuf_opt.wo_spell != 0
            && *(*(**curwin.as_ptr()).w_s_ptr()).b_p_spl != NUL
        {
            parse_spelllang(*curwin.as_ptr());
        }
        (*g_st().curbuf).b_last_used = os_time() as i64;

        if !(*g_st().curbuf).terminal.is_null() {
            terminal_check_size(&mut *(*g_st().curbuf).terminal);
        }

        redraw_later(*curwin.as_ptr(), UPD_NOT_VALID);
    }
}

/// Change to the directory of the current buffer.
pub fn do_autochdir() {
    unsafe {
        if opt_vars().p_acd != 0
            && *starting.as_ptr() == 0
            && !(*g_st().curbuf).b_ffname.is_null()
            && vim_chdirfile((*g_st().curbuf).b_ffname, kCdCauseAuto) == OK
        {
            g_st().last_chdir_reason = Some(c"autochdir");
            shorten_fnames(1);
        }
    }
}

pub fn no_write_message_buf(buf: &mut buf_T) {
    if !buf.terminal.is_null() && channel_job_running(buf.b_p_channel as u64) {
            emsg(gettext(
                e_job_still_running_add_bang_to_end_the_job.as_ptr(),
            ));
        } else {
            semsg!(
                gettext(e_no_write_since_last_change_for_buffer_nr_add_bang_to_override.as_ptr()),
                buf.b_fnum,
            );
        }
}

pub fn no_write_message() {
    unsafe {
        if !(*g_st().curbuf).terminal.is_null() && channel_job_running((*g_st().curbuf).b_p_channel as u64) {
            emsg(gettext(
                e_job_still_running_add_bang_to_end_the_job.as_ptr(),
            ));
        } else {
            emsg(gettext(
                e_no_write_since_last_change_add_bang_to_override.as_ptr(),
            ));
        }
    }
}

pub fn no_write_message_nobang(buf: &buf_T) {
    if !buf.terminal.is_null() && channel_job_running(buf.b_p_channel as u64) {
            emsg(gettext(e_job_still_running.as_ptr()));
        } else {
            emsg(gettext(e_no_write_since_last_change.as_ptr()));
        }
}

#[inline]
fn buf_init_changedtick(buf: &mut buf_T) {
    buf.changedtick_di.di_tv = typval_T {
            v_type: VarType::VAR_NUMBER,
            v_lock: VAR_FIXED,
            vval: TypvalVval::Number(buf_get_changedtick(buf)),
        };
        buf.changedtick_di.di_flags = DI_FLAGS_RO | DI_FLAGS_FIX;
        tv_dict_add(buf.b_vars(), (&raw mut buf.changedtick_di).cast());
}

/// Add a file name to the buffer list.
pub fn buflist_new(
    ffname_arg: *mut c_char,
    sfname_arg: *mut c_char,
    lnum: linenr_T,
    flags: c_int,
) -> *mut buf_T {
    unsafe {
        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let mut buf: *mut buf_T;

        let ffname_cstr = fname_expand(g_st().curbuf, &raw mut ffname, &raw mut sfname);

        let mut file_id: FileID = FileID::new();
        let file_id_valid = !sfname.is_null() && os_fileid(sfname, &mut *(&raw mut file_id).cast());
        if !ffname.is_null() && (flags & (BLN_DUMMY | BLN_NEW)) == 0 {
            buf = buflist_findname_file_id(ffname, &mut file_id, file_id_valid);
            if !buf.is_null() {
                drop(ffname_cstr);
                if lnum != 0 {
                    buflist_setfpos(
                        buf,
                        if (flags & BLN_NOCURWIN) != 0 {
                            ptr::null_mut()
                        } else {
                            *curwin.as_ptr()
                        },
                        lnum,
                        0,
                        false,
                    );
                }
                if (flags & BLN_NOOPT) == 0 {
                    buf_copy_options(buf, 0);
                }
                if (flags & BLN_LISTED) != 0 && (*buf).b_p_bl == 0 {
                    (*buf).b_p_bl = 1;
                    let mut bufref: bufref_T = bufref_T::new();
                    set_bufref(&mut bufref, buf);
                    if (flags & BLN_DUMMY) == 0
                        && apply_autocmds(
                            EVENT_BUFADD,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            false,
                            buf,
                        )
                        && !bufref_valid(Some(&bufref))
                    {
                        return ptr::null_mut();
                    }
                }
                return buf;
            }
        }

        buf = ptr::null_mut();
        if (flags & BLN_CURBUF) != 0 && curbuf_reusable() {
            let mut bufref: bufref_T = bufref_T::new();

            debug_assert!(!g_st().curbuf.is_null());
            buf = g_st().curbuf;
            set_bufref(&mut bufref, buf);
            trigger_undo_ftplugin(&mut *buf, &mut **curwin.as_ptr());
            buf_freeall(buf, BFA_WIPE | BFA_DEL);
            if aborting() {
                drop(ffname_cstr);
                return ptr::null_mut();
            }
            if !bufref_valid(Some(&bufref)) {
                buf = ptr::null_mut();
            }
        }
        if buf != g_st().curbuf || g_st().curbuf.is_null() {
            buf = buf_T::alloc();
            init_var_dict(&mut *tv_dict_alloc(), &mut *(&raw mut (*buf).b_bufvar).cast(), VAR_SCOPE);
            (*buf).changedtick_di.di_key = b"changedtick\0".iter().map(|&b| b as c_char).collect();
            buf_init_changedtick(&mut *buf);
        }

        if let Some(ff_cstr) = ffname_cstr {
            debug_assert!(!sfname.is_null());
            let mut sf_vec = cstr_to_cstring(sfname).into_bytes_with_nul();
            to_slash(sf_vec.as_mut_ptr().cast::<c_char>());
            (*buf).set_ffname(Some(ff_cstr));
            (*buf).set_sfname(Some(std::ffi::CString::from_vec_with_nul(sf_vec).unwrap()));
        }

        clear_wininfo(&mut *buf);
        (*buf).b_wininfo.push(WinInfo::new());

        if buf == g_st().curbuf {
            free_buffer_stuff(buf, kBffInitChangedtick);
            (*buf).b_p_initialized = false;
            buf_copy_options(buf, BCO_ENTER);
            (*g_st().curbuf).b_kmap_state |= KEYMAP_INIT;
        } else {
            (*buf).b_next = ptr::null_mut();
            if g_st().firstbuf.is_null() {
                (*buf).b_prev = ptr::null_mut();
                g_st().firstbuf = buf;
            } else {
                (*g_st().lastbuf).b_next = buf;
                (*buf).b_prev = g_st().lastbuf;
            }
            g_st().lastbuf = buf;

            (*buf).b_fnum = buf_st().top_file_num;
            buf_st().top_file_num += 1;
            let map_ptr = &raw mut g_st().buffer_handles;
            (*map_ptr).insert((*buf).b_fnum, buf);
            if buf_st().top_file_num < 0 {
                emsg(gettext(
                    c"W14: Warning: List of file names overflow".as_ptr(),
                ));
                if g_st().emsg_silent == 0 && !g_st().in_assert_fails {
                    msg_delay(3001, true);
                }
                buf_st().top_file_num = 1;
            }

            buf_copy_options(buf, BCO_ALWAYS);
        }

        let curwin_info = &mut (&mut (*buf).b_wininfo)[0];
        curwin_info.wi_mark = INIT_FMARK;
        curwin_info.wi_mark.mark.lnum = lnum;
        curwin_info.wi_win = *curwin.as_ptr();

        (*buf).b_fname = (*buf).b_sfname;
        if !file_id_valid {
            (*buf).file_id_valid = false;
        } else {
            (*buf).file_id_valid = true;
            (*buf).file_id = file_id;
        }
        (*buf).b_u_synced = true;
        (*buf).b_flags = BF_CHECK_RO | BF_NEVERLOADED;
        if (flags & BLN_DUMMY) != 0 {
            (*buf).b_flags |= BF_DUMMY;
        }
        buf_clear_file(&mut *buf);
        clrallmarks(&mut *buf, 0);
        fmarks_check_names(&mut *buf);
        (*buf).b_p_bl = c_int::from((flags & BLN_LISTED) != 0);
        (*buf).update_channels.clear();
        (*buf).update_callbacks.clear();
        if (flags & BLN_DUMMY) == 0 {
            let mut bufref: bufref_T = bufref_T::new();
            set_bufref(&mut bufref, buf);
            if apply_autocmds(EVENT_BUFNEW, ptr::null_mut(), ptr::null_mut(), false, buf)
                && !bufref_valid(Some(&bufref))
            {
                return ptr::null_mut();
            }
            if (flags & BLN_LISTED) != 0
                && apply_autocmds(EVENT_BUFADD, ptr::null_mut(), ptr::null_mut(), false, buf)
                && !bufref_valid(Some(&bufref))
            {
                return ptr::null_mut();
            }
            if aborting() {
                return ptr::null_mut();
            }
        }

        (*buf).b_prompt_callback = crate::eval::typval::Callback::NONE;
        (*buf).b_prompt_interrupt = crate::eval::typval::Callback::NONE;
        (*buf).b_prompt_text = None;
        (*buf).b_prompt_start = INIT_FMARK;
        (*buf).b_prompt_start.mark.col = 2;
        (*buf).b_prompt_append_new_line = true;

        buf
    }
}

/// Return true if the current buffer is empty, unnamed, unmodified and used in
/// only one window.
pub fn curbuf_reusable() -> bool {
    unsafe {
        !g_st().curbuf.is_null()
            && (*g_st().curbuf).b_ffname.is_null()
            && (*g_st().curbuf).b_nwindows <= 1
            && (*g_st().curbuf).terminal.is_null()
            && ((*g_st().curbuf).b_ml.ml_mfp.is_none() || buf_is_empty(&mut *g_st().curbuf))
            && !bt_quickfix(g_st().curbuf)
            && !curbuf_is_changed()
    }
}

/// Free the memory for the options of a buffer.
pub fn free_buf_options(buf: *mut buf_T, free_p_ff: bool) {
    unsafe {
        if free_p_ff {
            clear_string_option(&raw mut (*buf).b_p_fenc);
            clear_string_option(&raw mut (*buf).b_p_ff);
            clear_string_option(&raw mut (*buf).b_p_bh);
            clear_string_option(&raw mut (*buf).b_p_bt);
        }
        clear_string_option(&raw mut (*buf).b_p_def);
        clear_string_option(&raw mut (*buf).b_p_inc);
        callback_free(&mut (*buf).b_p_inex);
        callback_free(&mut (*buf).b_p_inde);
        clear_string_option(&raw mut (*buf).b_p_indk);
        clear_string_option(&raw mut (*buf).b_p_fp);
        callback_free(&mut (*buf).b_p_fex);
        clear_string_option(&raw mut (*buf).b_p_kp);
        clear_string_option(&raw mut (*buf).b_p_mps);
        clear_string_option(&raw mut (*buf).b_p_fo);
        clear_string_option(&raw mut (*buf).b_p_flp);
        clear_string_option(&raw mut (*buf).b_p_isk);
        clear_string_option(&raw mut (*buf).b_p_vsts);
        (*buf).b_p_vsts_nopaste = None;
        (*buf).b_p_vsts_array = Vec::new();
        clear_string_option(&raw mut (*buf).b_p_vts);
        (*buf).b_p_vts_array = Vec::new();
        clear_string_option(&raw mut (*buf).b_p_keymap);
        (*buf).b_kmap_ga.clear();
        clear_string_option(&raw mut (*buf).b_p_com);
        clear_string_option(&raw mut (*buf).b_p_cms);
        clear_string_option(&raw mut (*buf).b_p_nf);
        clear_string_option(&raw mut (*buf).b_p_syn);
        (*buf).b_s.b_syn_isk = None;
        clear_string_option(&raw mut (*buf).b_s.b_p_spc);
        clear_string_option(&raw mut (*buf).b_s.b_p_spf);
        (*buf).b_s.b_cap_prog = None;
        clear_string_option(&raw mut (*buf).b_s.b_p_spl);
        clear_string_option(&raw mut (*buf).b_s.b_p_spo);
        clear_string_option(&raw mut (*buf).b_p_sua);
        clear_string_option(&raw mut (*buf).b_p_ft);
        clear_string_option(&raw mut (*buf).b_p_cink);
        clear_string_option(&raw mut (*buf).b_p_cino);
        clear_string_option(&raw mut (*buf).b_p_lop);
        clear_string_option(&raw mut (*buf).b_p_cinsd);
        clear_string_option(&raw mut (*buf).b_p_cinw);
        clear_string_option(&raw mut (*buf).b_p_cot);
        clear_string_option(&raw mut (*buf).b_p_cpt);
        #[cfg(windows)]
        clear_string_option(&raw mut (*buf).b_p_csl);
        callback_free(&mut (*buf).b_p_cfu);
        callback_free(&mut (*buf).b_p_ofu);
        callback_free(&mut (*buf).b_p_tsrfu);
        clear_cpt_callbacks(&mut (*buf).b_p_cpt_cb);
        clear_string_option(&raw mut (*buf).b_p_gefm);
        clear_string_option(&raw mut (*buf).b_p_gp);
        clear_string_option(&raw mut (*buf).b_p_mp);
        clear_string_option(&raw mut (*buf).b_p_efm);
        clear_string_option(&raw mut (*buf).b_p_ep);
        clear_string_option(&raw mut (*buf).b_p_path);
        clear_string_option(&raw mut (*buf).b_p_tags);
        clear_string_option(&raw mut (*buf).b_p_tc);
        callback_free(&mut (*buf).b_p_tfu);
        callback_free(&mut (*buf).b_p_ffu);
        clear_string_option(&raw mut (*buf).b_p_dict);
        clear_string_option(&raw mut (*buf).b_p_dia);
        clear_string_option(&raw mut (*buf).b_p_tsr);
        clear_string_option(&raw mut (*buf).b_p_qe);
        (*buf).b_p_ac = -1;
        (*buf).b_p_ar = -1;
        (*buf).b_p_fs = -1;
        (*buf).b_p_ul = NO_LOCAL_UNDOLEVEL;
        clear_string_option(&raw mut (*buf).b_p_lw);
        clear_string_option(&raw mut (*buf).b_p_bkc);
        clear_string_option(&raw mut (*buf).b_p_menc);
    }
}

/// Get alternate file `n`.
pub fn buflist_getfile(
    n: c_int,
    mut lnum: linenr_T,
    options: c_int,
    forceit: c_int,
) -> c_int {
    unsafe {
        let mut fm: *mut fmark_T = ptr::null_mut();

        let buf = buflist_findnr(n);
        if buf.is_null() {
            if (options & GETF_ALT) != 0 && n == 0 {
                emsg(gettext(e_noalt.as_ptr()));
            } else {
                semsg!(gettext(e_buffer_nr_not_found.as_ptr()), n);
            }
            return FAIL;
        }

        if buf == g_st().curbuf {
            return OK;
        }

        if text_or_buf_locked() {
            return FAIL;
        }

        let col: colnr_T;
        let mut restore_view = false;
        if lnum == 0 {
            fm = buflist_findfmark(buf);
            lnum = (*fm).mark.lnum;
            col = (*fm).mark.col;
            restore_view = true;
        } else {
            col = 0;
        }

        if (options & GETF_SWITCH) != 0 {
            let wp = swbuf_goto_win_with_buf(buf);

            if wp.is_null()
                && (opt_vars().swb_flags & (kOptSwbFlagVsplit | kOptSwbFlagSplit | kOptSwbFlagNewtab)) != 0
                && !buf_is_empty(&mut *g_st().curbuf)
            {
                if (opt_vars().swb_flags & kOptSwbFlagNewtab) != 0 {
                    tabpage_new();
                } else if win_split(
                    0,
                    if (opt_vars().swb_flags & kOptSwbFlagVsplit) != 0 {
                        WSP_VERT
                    } else {
                        0
                    },
                ) == FAIL
                {
                    return FAIL;
                }
                reset_binding(&mut **curwin.as_ptr());
            }
        }

        g_st().RedrawingDisabled += 1;
        if getfile(
            (*buf).b_fnum,
            ptr::null_mut(),
            ptr::null_mut(),
            (options & GETF_SETMARK) != 0,
            lnum,
            forceit != 0,
        ) <= 0
        {
            g_st().RedrawingDisabled -= 1;

            if opt_vars().p_sol == 0 && col != 0 {
                (**curwin.as_ptr()).w_cursor.col = col;
                check_cursor_col(&mut **curwin.as_ptr());
                (**curwin.as_ptr()).w_cursor.coladd = 0;
                (**curwin.as_ptr()).w_set_curswant = true;
            }
            if (opt_vars().jop_flags & kOptJopFlagView) != 0 && restore_view {
                mark_view_restore(fm);
            }
            return OK;
        }
        g_st().RedrawingDisabled -= 1;
        FAIL
    }
}

fn buflist_getfpos() {
    unsafe {
        let fm = buflist_findfmark(g_st().curbuf);
        let fpos = &raw const (*fm).mark;

        (**curwin.as_ptr()).w_cursor.lnum = (*fpos).lnum;
        check_cursor_lnum(&mut **curwin.as_ptr());

        if opt_vars().p_sol != 0 {
            (**curwin.as_ptr()).w_cursor.col = 0;
        } else {
            (**curwin.as_ptr()).w_cursor.col = (*fpos).col;
            check_cursor_col(&mut **curwin.as_ptr());
            (**curwin.as_ptr()).w_cursor.coladd = 0;
            (**curwin.as_ptr()).w_set_curswant = true;
        }

        if (opt_vars().jop_flags & kOptJopFlagView) != 0 {
            mark_view_restore(fm);
        }
    }
}

/// Find file in buffer list by name (it has to be for the current window).
pub fn buflist_findname_exp(fname: *mut c_char) -> *mut buf_T {
    let mut buf: *mut buf_T = ptr::null_mut();
        if let Some(ffname) = FullName_save(fname, cfg!(unix)) {
            buf = buflist_findname(ffname.as_ptr().cast_mut());
        }
        buf
}

/// Find file in buffer list by name.
pub fn buflist_findname(ffname: *mut c_char) -> *mut buf_T {
    unsafe {
        let mut file_id: FileID = FileID::new();
        let file_id_valid = os_fileid(ffname, &mut *(&raw mut file_id).cast());
        buflist_findname_file_id(ffname, &mut file_id, file_id_valid)
    }
}

fn buflist_findname_file_id(
    ffname: *mut c_char,
    file_id: &mut FileID,
    file_id_valid: bool,
) -> *mut buf_T {
    unsafe {
        let mut buf = g_st().lastbuf;
        while !buf.is_null() {
            if ((*buf).b_flags & BF_DUMMY) == 0
                && !otherfile_buf(&mut *buf, ffname, file_id, file_id_valid)
            {
                return buf;
            }
            buf = (*buf).b_prev;
        }
        ptr::null_mut()
    }
}

/// Find file in buffer list by a regexp pattern.
pub fn buflist_findpat(
    pattern: *const c_char,
    pattern_end: *const c_char,
    unlisted: bool,
    diffmode: bool,
    curtab_only: bool,
) -> c_int {
    unsafe {
        let mut match_fnum = -1;

        if pattern_end == pattern.add(1)
            && (*pattern == b'%' as c_char || *pattern == b'#' as c_char)
        {
            match_fnum = if *pattern == b'%' as c_char {
                (*g_st().curbuf).b_fnum
            } else {
                (**curwin.as_ptr()).w_alt_fnum
            };
            let found_buf = buflist_findnr(match_fnum);
            if diffmode && (found_buf.is_null() || !diff_mode_buf(found_buf)) {
                match_fnum = -1;
            }
        } else {
            let Some(pat_cstr) = file_pat_to_reg_pat(pattern, pattern_end, ptr::null_mut(), 0)
            else {
                return -1;
            };
            let mut pat_vec: Vec<c_char> = pat_cstr
                .into_bytes_with_nul()
                .into_iter()
                .map(|b| b as c_char)
                .collect();
            let pat = pat_vec.as_mut_ptr();
            let patend = pat.add(std::ffi::CStr::from_ptr(pat).to_bytes().len().saturating_sub(1));
            let toggledollar = patend > pat && *patend == b'$' as c_char;

            let mut find_listed = 1;
            loop {
                for attempt in 0..=3 {
                    if toggledollar {
                        *patend = if attempt < 2 { NUL } else { b'$' as c_char };
                    }
                    let mut p = pat;
                    if *p == b'^' as c_char && (attempt & 1) == 0 {
                        p = p.add(1);
                    }

                    let mut regmatch: regmatch_T = regmatch_T::new();
                    regmatch.regprog = vim_regcomp(p, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });

                    let mut buf = g_st().lastbuf;
                    while !buf.is_null() {
                        if regmatch.regprog.is_null() {
                            return -1;
                        }
                        if (*buf).b_p_bl == find_listed
                            && (!diffmode || diff_mode_buf(buf))
                            && !buflist_match(&mut regmatch, &mut *buf, false).is_null()
                        {
                            if curtab_only {
                                let mut found_window = false;
                                let mut wp = g_st().firstwin;
                                while !wp.is_null() {
                                    if (*wp).w_buffer == buf {
                                        found_window = true;
                                        break;
                                    }
                                    wp = (*wp).w_next;
                                }
                                if !found_window {
                                    buf = (*buf).b_prev;
                                    continue;
                                }
                            }
                            if match_fnum >= 0 {
                                match_fnum = -2;
                                break;
                            }
                            match_fnum = (*buf).b_fnum;
                        }
                        buf = (*buf).b_prev;
                    }

                    vim_regfree(regmatch.regprog);
                    if match_fnum >= 0 {
                        break;
                    }
                }

                if !unlisted || find_listed == 0 || match_fnum != -1 {
                    break;
                }
                find_listed = 0;
            }
        }

        if match_fnum == -2 {
            semsg!(
                gettext(c"E93: More than one match for %s".as_ptr()),
                pattern,
            );
        } else if match_fnum < 0 {
            semsg!(gettext(c"E94: No matching buffer for %s".as_ptr()), pattern);
        }
        match_fnum
    }
}

fn buf_time_compare(buf1: &buf_T, buf2: &buf_T) -> std::cmp::Ordering {
    buf2.b_last_used.cmp(&buf1.b_last_used)
}

/// Find all buffer names that match.
pub fn ExpandBufnames(
    pat: *mut c_char,
    file: &mut Vec<std::ffi::CString>,
    options: c_int,
) -> c_int {
    unsafe {
        let mut matches: Vec<bufmatch_T> = Vec::new();

        file.clear();

        if (options & BUF_DIFF_FILTER) != 0 && (**curwin.as_ptr()).w_onebuf_opt.wo_diff == 0 {
            return FAIL;
        }

        let fuzzy = cmdline_fuzzy_complete(pat);
        let use_matches = !fuzzy && (options & WILD_BUFLASTUSED) != 0;

        let mut patc_owned: Option<std::ffi::CString> = None;
        let mut fuzmatch: Vec<fuzmatch_str_T> = Vec::new();
        let mut regmatch: regmatch_T = regmatch_T::new();

        if !fuzzy {
            let patc: *const c_char = if *pat == b'^' as c_char && *pat.add(1) != NUL {
                let pc = cstr_to_cstring(pat.add(1));
                let ptr = pc.as_ptr();
                patc_owned = Some(pc);
                ptr
            } else if *pat == b'^' as c_char {
                c"".as_ptr()
            } else {
                pat
            };
            regmatch.regprog = vim_regcomp(patc, RE_MAGIC);
            if regmatch.regprog.is_null() {
                return FAIL;
            }
        }

        let mut score = 0;
        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if (*buf).b_p_bl == 0 {
                buf = (*buf).b_next;
                continue;
            }
            if (options & BUF_DIFF_FILTER) != 0 && (buf == g_st().curbuf || !diff_mode_buf(buf)) {
                buf = (*buf).b_next;
                continue;
            }

            let mut p: *mut c_char = ptr::null_mut();
            if !fuzzy {
                p = buflist_match(&mut regmatch, &mut *buf, opt_vars().p_wic != 0);
            } else {
                score = fuzzy_match_str((*buf).b_sfname, pat);
                if score != FUZZY_SCORE_NONE {
                    p = (*buf).b_sfname;
                }
                if p.is_null() {
                    score = fuzzy_match_str((*buf).b_ffname, pat);
                    if score != FUZZY_SCORE_NONE {
                        p = (*buf).b_ffname;
                    }
                }
            }

            if p.is_null() {
                buf = (*buf).b_next;
                continue;
            }

            let p_cstr = if (options & WILD_HOME_REPLACE) != 0 {
                home_replace_save(buf, p)
            } else {
                cstr_to_cstring(p)
            };

            if !fuzzy {
                if use_matches {
                    matches.push(bufmatch_T {
                        buf,
                        match_: p_cstr,
                    });
                } else {
                    file.push(p_cstr);
                }
            } else {
                let idx = fuzmatch.len() as c_int;
                fuzmatch.push(fuzmatch_str_T {
                    idx,
                    str_cstr: p_cstr,
                    score,
                });
            }
            buf = (*buf).b_next;
        }

        if !fuzzy {
            vim_regfree(regmatch.regprog);
            drop(patc_owned);
        }

        if !fuzzy {
            if use_matches && !matches.is_empty() {
                if matches.len() > 1 {
                    matches.sort_unstable_by(|a, b| buf_time_compare(&*a.buf, &*b.buf));
                }

                file.reserve_exact(matches.len());
                if matches[0].buf == g_st().curbuf {
                    let mut iter = matches.into_iter();
                    let first = iter.next().unwrap();
                    for item in iter {
                        file.push(item.match_);
                    }
                    file.push(first.match_);
                } else {
                    for item in matches {
                        file.push(item.match_);
                    }
                }
            }
        } else {
            fuzzymatches_slice_to_strmatches(&mut fuzmatch, file, false);
        }

        if file.is_empty() { FAIL } else { OK }
    }
}

fn buflist_match(rmp: &mut regmatch_T, buf: &mut buf_T, ignore_case: bool) -> *mut c_char {
    let mut match_ = fname_match(rmp, buf.b_sfname, ignore_case);
        if match_.is_null() && !rmp.regprog.is_null() {
            match_ = fname_match(rmp, buf.b_ffname, ignore_case);
        }
        match_
}

fn fname_match(rmp: &mut regmatch_T, name: *mut c_char, ignore_case: bool) -> *mut c_char {
    let mut match_: *mut c_char = ptr::null_mut();

        if name.is_null() || rmp.regprog.is_null() {
            return ptr::null_mut();
        }

        rmp.rm_ic = opt_vars().p_fic != 0 || ignore_case;
        if vim_regexec(rmp, name, 0) {
            match_ = name;
        } else if !rmp.regprog.is_null() {
            let p = home_replace_save(ptr::null_mut(), name);
            if vim_regexec(rmp, p.as_ptr().cast_mut(), 0) {
                match_ = name;
            }
        }

        match_
}

/// Find a file in the buffer list by buffer number.
pub fn buflist_findnr(mut nr: c_int) -> *mut buf_T {
    unsafe {
        if nr == 0 {
            nr = (**curwin.as_ptr()).w_alt_fnum;
        }
        let map_ptr = &raw const g_st().buffer_handles;
        (*map_ptr).get(&nr).copied().unwrap_or(ptr::null_mut())
    }
}

pub fn buflist_nr2name(
    n: c_int,
    fullname: c_int,
    helptail: c_int,
) -> Option<std::ffi::CString> {
    unsafe {
        let buf = buflist_findnr(n);
        if buf.is_null() {
            return None;
        }
        Some(home_replace_save(
            if helptail != 0 { buf } else { ptr::null_mut() },
            if fullname != 0 {
                (*buf).b_ffname
            } else {
                (*buf).b_fname
            },
        ))
    }
}

/// Set the line and column numbers for the given buffer and window.
pub fn buflist_setfpos(
    buf: *mut buf_T,
    win: *mut win_T,
    mut lnum: linenr_T,
    col: colnr_T,
    copy_options: bool,
) {
    unsafe {
        let mut wip = if let Some(i) = (*buf).b_wininfo.iter().position(|w| w.wi_win == win) {
            let mut wip = (*buf).b_wininfo.remove(i);
            if copy_options && wip.wi_optset {
                clear_winopt(&mut wip.wi_opt);
                wip.wi_folds.clear();
                wip.wi_optset = false;
            }
            wip
        } else {
            let mut wip = WinInfo::new();
            wip.wi_win = win;
            if lnum == 0 {
                lnum = 1;
            }
            wip
        };
        if lnum != 0 {
            wip.wi_mark.mark.lnum = lnum;
            wip.wi_mark.mark.col = col;
            if !win.is_null() {
                wip.wi_mark.view = mark_view_make(&*win, wip.wi_mark.mark);
            }
        }
        if !win.is_null() {
            wip.wi_changelistidx = (*win).w_changelistidx;
        }
        if copy_options && !win.is_null() {
            copy_winopt(&mut (*win).w_onebuf_opt, &mut wip.wi_opt);
            wip.wi_fold_manual = (*win).w_fold_manual;
            wip.wi_folds.clone_from(&(*win).w_folds);
            wip.wi_optset = true;
        }

        (*buf).b_wininfo.insert(0, wip);
    }
}

fn wininfo_other_tab_diff(wip: &WinInfo) -> bool {
    unsafe {
        if wip.wi_opt.wo_diff == 0 {
            return false;
        }

        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            if wip.wi_win == wp {
                return false;
            }
            wp = (*wp).w_next;
        }
        true
    }
}

fn find_wininfo(
    buf: &mut buf_T,
    need_options: bool,
    skip_diff_buffer: bool,
) -> *mut WinInfo {
    unsafe {
        for wip in &mut buf.b_wininfo {
            if wip.wi_win == *curwin.as_ptr()
                && (!skip_diff_buffer || !wininfo_other_tab_diff(wip))
                && (!need_options || wip.wi_optset)
            {
                return wip;
            }
        }

        if skip_diff_buffer {
            let buf_ptr: *mut buf_T = &raw mut *buf;
            for wip in &mut buf.b_wininfo {
                if !wininfo_other_tab_diff(wip)
                    && (!need_options
                        || wip.wi_optset
                        || (!wip.wi_win.is_null() && (*wip.wi_win).w_buffer == buf_ptr))
                {
                    return wip;
                }
            }
        } else if let Some(wip) = buf.b_wininfo.first_mut() {
            return wip;
        }
        ptr::null_mut()
    }
}

/// Reset the local window options to the values last used in this window.
pub fn get_winopts(buf: *mut buf_T) {
    unsafe {
        clear_winopt(&mut (**curwin.as_ptr()).w_onebuf_opt);
        clearFolding(&mut **curwin.as_ptr());

        let wip = find_wininfo(&mut *buf, true, true);
        if !wip.is_null()
            && (*wip).wi_win != *curwin.as_ptr()
            && !(*wip).wi_win.is_null()
            && (*(*wip).wi_win).w_buffer == buf
            && (*(*wip).wi_win).w_config.style != kWinStyleMinimal
        {
            let wp = (*wip).wi_win;
            copy_winopt(&mut (*wp).w_onebuf_opt, &mut (**curwin.as_ptr()).w_onebuf_opt);
            (**curwin.as_ptr()).w_fold_manual = (*wp).w_fold_manual;
            (**curwin.as_ptr()).w_foldinvalid = true;
            (**curwin.as_ptr()).w_folds.clone_from(&(*wp).w_folds);
        } else if !wip.is_null()
            && (*wip).wi_optset
            && ((*wip).wi_win.is_null()
                || (*wip).wi_win == *curwin.as_ptr()
                || (*(*wip).wi_win).w_config.style != kWinStyleMinimal)
        {
            copy_winopt(&mut (*wip).wi_opt, &mut (**curwin.as_ptr()).w_onebuf_opt);
            (**curwin.as_ptr()).w_fold_manual = (*wip).wi_fold_manual;
            (**curwin.as_ptr()).w_foldinvalid = true;
            (**curwin.as_ptr()).w_folds.clone_from(&(*wip).wi_folds);
        } else {
            copy_winopt(
                &mut (**curwin.as_ptr()).w_allbuf_opt,
                &mut (**curwin.as_ptr()).w_onebuf_opt,
            );
        }
        if !wip.is_null() {
            (**curwin.as_ptr()).w_changelistidx = (*wip).wi_changelistidx;
        }

        if (**curwin.as_ptr()).w_config.style == kWinStyleMinimal {
            didset_window_options(*curwin.as_ptr(), false);
            win_set_minimal_style(&mut **curwin.as_ptr());
        }

        if opt_vars().p_fdls >= 0 {
            (**curwin.as_ptr()).w_onebuf_opt.wo_fdl = opt_vars().p_fdls;
        }
        didset_window_options(*curwin.as_ptr(), false);
    }
}

/// Find the mark for the buffer `buf` for the current window.
pub fn buflist_findfmark(buf: *mut buf_T) -> *mut fmark_T {
    unsafe {
        let wip = find_wininfo(&mut *buf, false, false);
        if wip.is_null() {
            &raw mut buf_st().no_position
        } else {
            &raw mut (*wip).wi_mark
        }
    }
}

/// Find the lnum for the buffer `buf` for the current window.
pub fn buflist_findlnum(buf: *mut buf_T) -> linenr_T {
    unsafe { (*buflist_findfmark(buf)).mark.lnum }
}

/// List all known file names (for `:files` and `:buffers` command).
pub fn buflist_list(eap: &mut exarg_T) {
    unsafe {
        let mut buf = g_st().firstbuf;
        let mut buflist: Vec<*mut buf_T> = Vec::new();
        let mut use_buflist = false;
        let mut buflist_idx = 0usize;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if !vim_strchr(eap.arg, b't' as c_int).is_null() {
            use_buflist = true;
            while !buf.is_null() {
                buflist.push(buf);
                buf = (*buf).b_next;
            }

            if buflist.len() > 1 {
                buflist.sort_unstable_by(|&a, &b| buf_time_compare(&*a, &*b));
            }

            buf = buflist.first().copied().unwrap_or(ptr::null_mut());
        }

        while !buf.is_null() && !g_st().got_int {
            let is_terminal = !(*buf).terminal.is_null();
            let job_running = is_terminal && terminal_running(&*(*buf).terminal);

            let mut next_buf = || {
                if use_buflist {
                    buflist_idx += 1;
                    buflist.get(buflist_idx).copied().unwrap_or(ptr::null_mut())
                } else {
                    (*buf).b_next
                }
            };

            if ((*buf).b_p_bl == 0
                && eap.forceit == 0
                && vim_strchr(eap.arg, b'u' as c_int).is_null())
                || (!vim_strchr(eap.arg, b'u' as c_int).is_null() && (*buf).b_p_bl != 0)
                || (!vim_strchr(eap.arg, b'+' as c_int).is_null()
                    && (((*buf).b_flags & BF_READERR) != 0 || !buf_is_changed(&*buf)))
                || (!vim_strchr(eap.arg, b'a' as c_int).is_null()
                    && ((*buf).b_ml.ml_mfp.is_none() || (*buf).b_nwindows == 0))
                || (!vim_strchr(eap.arg, b'h' as c_int).is_null()
                    && ((*buf).b_ml.ml_mfp.is_none() || (*buf).b_nwindows != 0))
                || (!vim_strchr(eap.arg, b'R' as c_int).is_null()
                    && (!is_terminal || !job_running))
                || (!vim_strchr(eap.arg, b'F' as c_int).is_null()
                    && (!is_terminal || job_running))
                || (!vim_strchr(eap.arg, b'-' as c_int).is_null() && (*buf).b_p_ma != 0)
                || (!vim_strchr(eap.arg, b'=' as c_int).is_null() && (*buf).b_p_ro == 0)
                || (!vim_strchr(eap.arg, b'x' as c_int).is_null()
                    && ((*buf).b_flags & BF_READERR) == 0)
                || (!vim_strchr(eap.arg, b'%' as c_int).is_null() && buf != g_st().curbuf)
                || (!vim_strchr(eap.arg, b'#' as c_int).is_null()
                    && (buf == g_st().curbuf || (**curwin.as_ptr()).w_alt_fnum != (*buf).b_fnum))
            {
                buf = next_buf();
                continue;
            }

            let name = buf_spname(buf);
            let namebuff_ptr = (&raw mut g_st().NameBuff).cast::<c_char>();
            if !name.is_null() {
                xstrlcpy(namebuff_ptr, name, MAXPATHL);
            } else {
                home_replace(buf, (*buf).b_fname, namebuff_ptr, MAXPATHL, true);
            }

            if message_filtered(namebuff_ptr) {
                buf = next_buf();
                continue;
            }

            let changed_char = if ((*buf).b_flags & BF_READERR) != 0 {
                b'x' as c_int
            } else if buf_is_changed(&*buf) {
                b'+' as c_int
            } else {
                b' ' as c_int
            };
            let mut ro_char = if (*buf).b_p_ma == 0 {
                b'-' as c_int
            } else if (*buf).b_p_ro != 0 {
                b'=' as c_int
            } else {
                b' ' as c_int
            };
            if is_terminal {
                ro_char = if terminal_running(&*(*buf).terminal) {
                    b'R' as c_int
                } else {
                    b'F' as c_int
                };
            }

            if !ui_has(kUIMessages) || g_st().msg_col > 0 {
                msg_putchar(b'\n' as c_int);
            }
            let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
            let mut len = vim_snprintf_safelen!(
                iobuff_ptr,
                IOSIZE - 20,
                c"%3d%c%c%c%c%c \"%s\"".as_ptr(),
                (*buf).b_fnum,
                if (*buf).b_p_bl != 0 {
                    b' ' as c_int
                } else {
                    b'u' as c_int
                },
                if buf == g_st().curbuf {
                    b'%' as c_int
                } else if (**curwin.as_ptr()).w_alt_fnum == (*buf).b_fnum {
                    b'#' as c_int
                } else {
                    b' ' as c_int
                },
                if (*buf).b_ml.ml_mfp.is_none() {
                    b' ' as c_int
                } else if (*buf).b_nwindows == 0 {
                    b'h' as c_int
                } else {
                    b'a' as c_int
                },
                ro_char,
                changed_char,
                namebuff_ptr,
            )
            .min(IOSIZE - 20);

            let mut i = 40 - vim_strsize(iobuff_ptr);
            loop {
                *iobuff_ptr.add(len) = b' ' as c_char;
                len += 1;
                i -= 1;
                if i <= 0 || len >= IOSIZE - 18 {
                    break;
                }
            }
            if !vim_strchr(eap.arg, b't' as c_int).is_null() && (*buf).b_last_used != 0 {
                undo_fmt_time(iobuff_ptr.add(len), IOSIZE - len, (*buf).b_last_used);
            } else {
                vim_snprintf!(
                    iobuff_ptr.add(len),
                    IOSIZE - len,
                    gettext(c"line %lld".as_ptr()),
                    if buf == g_st().curbuf {
                        i64::from((**curwin.as_ptr()).w_cursor.lnum)
                    } else {
                        i64::from(buflist_findlnum(buf))
                    },
                );
            }

            msg_outtrans(iobuff_ptr, 0, false);
            line_breakcheck();
            buf = next_buf();
        }
    }
}

/// Get file name and line number for file `fnum`.
pub fn buflist_name_nr(fnum: c_int, fname: *mut *mut c_char, lnum: &mut linenr_T) -> c_int {
    unsafe {
        let buf = buflist_findnr(fnum);
        if buf.is_null() || (*buf).b_fname.is_null() {
            return FAIL;
        }

        *fname = (*buf).b_fname;
        *lnum = buflist_findlnum(buf);

        OK
    }
}

/// Set the file name for `buf` to `ffname_arg`, short file name to `sfname_arg`.
pub fn setfname(
    buf: *mut buf_T,
    ffname_arg: *mut c_char,
    sfname_arg: *mut c_char,
    message: bool,
) -> c_int {
    unsafe {
        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let mut obuf: *mut buf_T = ptr::null_mut();
        let mut file_id: FileID = FileID::new();
        let mut file_id_valid = false;

        if ffname.is_null() || *ffname == NUL {
            (*buf).clear_fnames();
        } else {
            let Some(ffname_cstr) = fname_expand(buf, &raw mut ffname, &raw mut sfname) else {
                return FAIL;
            };

            file_id_valid = os_fileid(ffname, &mut *(&raw mut file_id).cast());
            if ((*buf).b_flags & BF_DUMMY) == 0 {
                obuf = buflist_findname_file_id(ffname, &mut file_id, file_id_valid);
            }
            if !obuf.is_null() && obuf != buf {
                let mut in_use = false;

                let mut tab = g_st().first_tabpage;
                while !tab.is_null() {
                    let mut win = if tab == g_st().curtab {
                        g_st().firstwin
                    } else {
                        (*tab).tp_firstwin
                    };
                    while !win.is_null() {
                        if (*win).w_buffer == obuf {
                            in_use = true;
                        }
                        win = (*win).w_next;
                    }
                    tab = (*tab).tp_next;
                }

                if (*obuf).b_ml.ml_mfp.is_some() || in_use {
                    if message {
                        emsg(gettext(
                            c"E95: Buffer with this name already exists".as_ptr(),
                        ));
                    }
                    return FAIL;
                }
                close_buffer(ptr::null_mut(), obuf, DOBUF_WIPE, false, false, false);
            }
            let mut sf_vec = cstr_to_cstring(sfname).into_bytes_with_nul();
            if CASE_INSENSITIVE_FILENAME {
                to_slash(sf_vec.as_mut_ptr().cast::<c_char>());
                path_fix_case(sf_vec.as_mut_ptr().cast::<c_char>());
            }
            let sf_len = CStr::from_ptr(sf_vec.as_ptr().cast::<c_char>())
                .to_bytes()
                .len();
            let sfname_cstr = cstr_from_bytes(&sf_vec[..sf_len]);
            (*buf).set_ffname(Some(ffname_cstr));
            (*buf).set_sfname(Some(sfname_cstr));
        }
        (*buf).b_fname = (*buf).b_sfname;
        if !file_id_valid {
            (*buf).file_id_valid = false;
        } else {
            (*buf).file_id = file_id;
            (*buf).file_id_valid = true;
        }

        buf_name_changed(buf);
        OK
    }
}

/// Crude way of changing the name of a buffer.
pub fn buf_set_name(fnum: c_int, name: *mut c_char) {
    unsafe {
        let buf = buflist_findnr(fnum);
        if buf.is_null() {
            return;
        }

        (*buf).clear_fnames();
        let name_cstr = cstr_to_cstring(name);
        let mut ffname = name_cstr.as_ptr().cast_mut();
        let mut sfname: *mut c_char = ptr::null_mut();
        let ffname_cstr = fname_expand(buf, &raw mut ffname, &raw mut sfname);
        let sfname_cstr = if sfname.is_null() {
            None
        } else if ptr::eq(sfname, name_cstr.as_ptr()) {
            Some(name_cstr)
        } else {
            Some(cstr_to_cstring(sfname))
        };
        (*buf).set_ffname(ffname_cstr);
        (*buf).set_sfname(sfname_cstr);
        (*buf).b_fname = (*buf).b_sfname;
    }
}

/// Take care of what needs to be done when the name of buffer `buf` has changed.
pub fn buf_name_changed(buf: *mut buf_T) {
    unsafe {
        if (*buf).b_ml.ml_mfp.is_some() {
            ml_setname(&mut *buf);
        }

        if (**curwin.as_ptr()).w_buffer == buf {
            check_arg_idx(&mut **curwin.as_ptr());
        }
        maketitle();
        status_redraw_all();
        fmarks_check_names(&mut *buf);
        ml_timestamp(buf);
    }
}

/// Set alternate file name for current window.
pub fn setaltfname(ffname: *mut c_char, sfname: *mut c_char, lnum: linenr_T) -> *mut buf_T {
    unsafe {
        let buf = buflist_new(ffname, sfname, lnum, 0);
        if !buf.is_null() && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
            (**curwin.as_ptr()).w_alt_fnum = (*buf).b_fnum;
        }
        buf
    }
}

/// Get alternate file name for current window.
pub fn getaltfname(errmsg: bool) -> *mut c_char {
    let mut fname: *mut c_char = ptr::null_mut();
        let mut dummy: linenr_T = 0;

        if buflist_name_nr(0, &raw mut fname, &mut dummy) == FAIL {
            if errmsg {
                emsg(gettext(e_noalt.as_ptr()));
            }
            return ptr::null_mut();
        }
        fname
}

/// Add a file name to the buflist and return its number.
pub fn buflist_add(fname: *mut c_char, flags: c_int) -> c_int {
    unsafe {
        let buf = buflist_new(fname, ptr::null_mut(), 0, flags);
        if !buf.is_null() { (*buf).b_fnum } else { 0 }
    }
}

/// Set alternate cursor position for the current buffer and window `win`.
pub fn buflist_altfpos(win: &mut win_T) {
    buflist_setfpos(g_st().curbuf, win, win.w_cursor.lnum, win.w_cursor.col, true);
}

/// Check that `ffname` is not the same file as current file.
pub fn otherfile(ffname: *mut c_char) -> bool {
    unsafe { otherfile_buf(&mut *g_st().curbuf, ffname, ptr::null_mut(), false) }
}

fn otherfile_buf(
    buf: &mut buf_T,
    ffname: *mut c_char,
    mut file_id_p: *mut FileID,
    mut file_id_valid: bool,
) -> bool {
    unsafe {
        if ffname.is_null() || *ffname == NUL || buf.b_ffname.is_null() {
            return true;
        }
        if path_equal(ffname, buf.b_ffname, kPathCmpLiteral) {
            return false;
        }
        {
            let mut file_id: FileID = FileID::new();
            if file_id_p.is_null() {
                file_id_p = &raw mut file_id;
                file_id_valid = os_fileid(ffname, &mut *file_id_p.cast());
            }
            if !file_id_valid {
                return true;
            }
            if buf_same_file_id(buf, file_id_p) {
                buf_set_file_id(&raw mut *buf);
                if buf_same_file_id(buf, file_id_p) {
                    return false;
                }
            }
        }
        true
    }
}

/// Set `file_id` for a buffer.
pub fn buf_set_file_id(buf: *mut buf_T) {
    unsafe {
        let mut file_id: FileID = FileID::new();
        if !(*buf).b_fname.is_null() && os_fileid((*buf).b_fname, &mut *(&raw mut file_id).cast()) {
            (*buf).file_id_valid = true;
            (*buf).file_id = file_id;
        } else {
            (*buf).file_id_valid = false;
        }
    }
}

#[inline]
fn buf_same_file_id(buf: &mut buf_T, file_id: *mut FileID) -> bool {
    unsafe {
        buf.file_id_valid && os_fileid_equal(&*(&raw const buf.file_id).cast(), &*file_id.cast())
    }
}

/// Print info about the current buffer.
pub fn fileinfo(fullname: c_int, shorthelp: c_int, dont_truncate: bool) {
    unsafe {
        let mut buffer_vec = vec![0 as c_char; IOSIZE];
        let buffer = buffer_vec.as_mut_ptr();
        let mut bufferlen = 0usize;

        if fullname > 1 {
            bufferlen =
                vim_snprintf_safelen!(buffer, IOSIZE, c"buf %d: ".as_ptr(), (*g_st().curbuf).b_fnum);
        }

        *buffer.add(bufferlen) = b'"' as c_char;
        bufferlen += 1;

        let mut name = buf_spname(g_st().curbuf);
        if !name.is_null() {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                c"%s".as_ptr(),
                name,
            );
        } else {
            name = if fullname == 0 && !(*g_st().curbuf).b_fname.is_null() {
                (*g_st().curbuf).b_fname
            } else {
                (*g_st().curbuf).b_ffname
            };
            bufferlen += home_replace(
                if shorthelp != 0 {
                    g_st().curbuf
                } else {
                    ptr::null_mut()
                },
                name,
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                true,
            );
        }

        let dontwrite = bt_dontwrite(g_st().curbuf);
        let is_changed = curbuf_is_changed();
        let mod_str = if is_changed {
            if shortmess(kShmMod) {
                c" [+]".as_ptr()
            } else {
                gettext(c" [Modified]".as_ptr())
            }
        } else {
            c" ".as_ptr()
        };
        let notedited_str = if ((*g_st().curbuf).b_flags & BF_NOTEDITED) != 0 && !dontwrite {
            gettext(c"[Not edited]".as_ptr())
        } else {
            c"".as_ptr()
        };
        let new_str = if ((*g_st().curbuf).b_flags & BF_NEW) != 0 && !dontwrite {
            gettext(c"[New]".as_ptr())
        } else {
            c"".as_ptr()
        };
        let readerr_str = if ((*g_st().curbuf).b_flags & BF_READERR) != 0 {
            gettext(c"[Read errors]".as_ptr())
        } else {
            c"".as_ptr()
        };
        let ro_str = if (*g_st().curbuf).b_p_ro != 0 {
            if shortmess(kShmRo) {
                gettext(c"[RO]".as_ptr())
            } else {
                gettext(c"[readonly]".as_ptr())
            }
        } else {
            c"".as_ptr()
        };
        let sep_str =
            if is_changed || ((*g_st().curbuf).b_flags & BF_WRITE_MASK) != 0 || (*g_st().curbuf).b_p_ro != 0 {
                c" ".as_ptr()
            } else {
                c"".as_ptr()
            };

        bufferlen += vim_snprintf_safelen!(
            buffer.add(bufferlen),
            IOSIZE - bufferlen,
            c"\"%s%s%s%s%s%s".as_ptr(),
            mod_str,
            notedited_str,
            new_str,
            readerr_str,
            ro_str,
            sep_str,
        );

        let line_count = i64::from((*g_st().curbuf).b_ml.ml_line_count);
        let cur_lnum = i64::from((**curwin.as_ptr()).w_cursor.lnum);
        if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                c"%s".as_ptr(),
                gettext(no_lines_msg.as_ptr()),
            );
        } else if opt_vars().p_ru != 0 {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                ngettext(
                    c"%lld line --%d%%--".as_ptr(),
                    c"%lld lines --%d%%--".as_ptr(),
                    line_count as usize,
                ),
                line_count,
                calc_percentage(cur_lnum, line_count),
            );
        } else {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                gettext(c"line %lld of %lld --%d%%-- col ".as_ptr()),
                cur_lnum,
                line_count,
                calc_percentage(cur_lnum, line_count),
            );
            validate_virtcol(*curwin.as_ptr());
            bufferlen += col_print(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                (**curwin.as_ptr()).w_cursor.col + 1,
                (**curwin.as_ptr()).w_virtcol + 1,
            ) as usize;
        }

        append_arg_number(&mut **curwin.as_ptr(), buffer.add(bufferlen), IOSIZE - bufferlen);

        if dont_truncate {
            msg_start();
            let n = g_st().msg_scroll;
            g_st().msg_scroll = 1;
            msg(buffer, 0);
            g_st().msg_scroll = n;
        } else {
            let p = msg_trunc(buffer, false, 0);
            if g_st().restart_edit != 0 || (g_st().msg_scrolled != 0 && !g_st().need_wait_return) {
                set_keep_msg(p, 0);
            }
        }
    }
}

pub fn col_print(buf: *mut c_char, buflen: usize, col: c_int, vcol: c_int) -> c_int {
    if col == vcol {
            vim_snprintf_safelen!(buf, buflen, c"%d".as_ptr(), col) as c_int
        } else {
            vim_snprintf_safelen!(buf, buflen, c"%d-%d".as_ptr(), col, vcol) as c_int
        }
}

/// Put the title name in the title bar and icon of the window.
pub fn maketitle() {
    unsafe {
        let mut title_str: *mut c_char = ptr::null_mut();
        let mut icon_str: *mut c_char = ptr::null_mut();
        let mut buf = [0 as c_char; IOSIZE];
        let outbuf = CharBuf {
            data: buf.as_mut_ptr(),
            size: IOSIZE,
        };

        if !redrawing() {
            g_st().need_maketitle = true;
            return;
        }

        g_st().need_maketitle = false;
        if opt_vars().p_title == 0
            && opt_vars().p_icon == 0
            && buf_st().lasttitle.is_none()
            && buf_st().lasticon.is_none()
        {
            return;
        }

        if opt_vars().p_title != 0 {
            let mut maxlen = 0;

            if opt_vars().p_titlelen > 0 {
                maxlen = ((opt_vars().p_titlelen * i64::from(g_st().Columns) / 100) as c_int).max(10);
            }

            if *opt_vars().p_titlestring != NUL {
                if (g_st().stl_syntax & STL_IN_TITLE) != 0 {
                    build_stl_str_hl(
                        *curwin.as_ptr(),
                        outbuf,
                        opt_vars().p_titlestring,
                        kOptTitlestring,
                        0,
                        0,
                        maxlen,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                    title_str = buf.as_mut_ptr();
                } else {
                    title_str = opt_vars().p_titlestring;
                }
            } else {
                #[cfg(windows)]
                let p_ssl_save = opt_vars().p_ssl;
                #[cfg(windows)]
                {
                    opt_vars().p_ssl = 1;
                }
                let default_titlestring = c"%t%( %M%)%( (%{expand('%:p:~:h')})%)%a - Nvim"
                    .as_ptr()
                    .cast_mut();
                build_stl_str_hl(
                    *curwin.as_ptr(),
                    outbuf,
                    default_titlestring,
                    kOptTitlestring,
                    0,
                    0,
                    maxlen,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                title_str = buf.as_mut_ptr();
                #[cfg(windows)]
                {
                    opt_vars().p_ssl = p_ssl_save;
                }
            }
        }
        let mut mustset = value_change(title_str, &mut buf_st().lasttitle);

        if opt_vars().p_icon != 0 {
            icon_str = buf.as_mut_ptr();
            if *opt_vars().p_iconstring != NUL {
                if (g_st().stl_syntax & STL_IN_ICON) != 0 {
                    build_stl_str_hl(
                        *curwin.as_ptr(),
                        outbuf,
                        opt_vars().p_iconstring,
                        kOptIconstring,
                        0,
                        0,
                        0,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                } else {
                    icon_str = opt_vars().p_iconstring;
                }
            } else {
                let mut name = buf_spname(g_st().curbuf);
                if name.is_null() {
                    name = path_tail((*g_st().curbuf).b_ffname);
                }
                let mut namelen = std::ffi::CStr::from_ptr(name).to_bytes().len() as c_int;
                if namelen > 100 {
                    namelen -= 100;
                    namelen += c_int::from(utf_cp_bounds(name, name.add(namelen as usize)).end_off);
                    name = name.add(namelen as usize);
                }
                { let s = name; std::ptr::copy_nonoverlapping(s, buf.as_mut_ptr(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                trans_characters(buf.as_mut_ptr(), IOSIZE as c_int);
            }
        }

        mustset |= value_change(icon_str, &mut buf_st().lasticon);

        if mustset {
            resettitle();
        }
    }
}

fn value_change(str_: *mut c_char, last: &mut Option<std::ffi::CString>) -> bool {
    unsafe {
        let last_ptr = last.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        if str_.is_null() != last_ptr.is_null()
            || (!str_.is_null() && !last_ptr.is_null() && std::ffi::CStr::from_ptr(str_).to_bytes() != std::ffi::CStr::from_ptr(last_ptr).to_bytes())
        {
            if str_.is_null() {
                *last = None;
                resettitle();
            } else {
                *last = Some(cstr_to_cstring(str_));
                return true;
            }
        }
        false
    }
}

/// Set current window title.
pub fn resettitle() {
    unsafe {
        let icon_ptr = (&raw const buf_st().lasticon)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr());
        let title_ptr = (&raw const buf_st().lasttitle)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr());
        ui_call_set_icon(cstr_as_string(icon_ptr));
        ui_call_set_title(cstr_as_string(title_ptr));
    }
}

pub fn free_titles() {
    buf_st().lasttitle = None;
        buf_st().lasticon = None;
}

/// Get relative cursor position in window into `buf[]`.
pub fn get_rel_pos(wp: &mut win_T, buf: *mut c_char, buflen: c_int) -> c_int {
    unsafe {
        if buflen < 3 {
            return 0;
        }

        let mut above = wp.w_topline - 1;
        above += win_get_fill(&raw mut *wp, wp.w_topline) - wp.w_topfill;
        if wp.w_topline == 1 && wp.w_topfill >= 1 {
            above = 0;
        }
        let below = (*wp.w_buffer).b_ml.ml_line_count - wp.w_botline + 1;
        if below <= 0 {
            return vim_snprintf_safelen!(
                buf,
                buflen as usize,
                c"%s".as_ptr(),
                if above == 0 {
                    gettext(c"All".as_ptr())
                } else {
                    gettext(c"Bot".as_ptr())
                },
            ) as c_int;
        }

        if above <= 0 {
            return vim_snprintf_safelen!(
                buf,
                buflen as usize,
                c"%s".as_ptr(),
                gettext(c"Top".as_ptr()),
            ) as c_int;
        }

        let perc = calc_percentage(i64::from(above), i64::from(above + below));
        let mut tmp = [0 as c_char; 8];
        vim_snprintf!(tmp.as_mut_ptr(), tmp.len(), gettext(c"%d%%".as_ptr()), perc);
        vim_snprintf_safelen!(buf, buflen as usize, gettext(c"%3s".as_ptr()), tmp.as_ptr()) as c_int
    }
}

/// Append `(2 of 8)` to `buf[]`, if editing more than one file.
pub fn append_arg_number(wp: &mut win_T, buf: *mut c_char, buflen: usize) -> c_int {
    let argcount = crate::arglist::argcount();
        if argcount <= 1 {
            return 0;
        }

        let msg = if wp.w_arg_idx_invalid {
            gettext(c" ((%d) of %d)".as_ptr())
        } else {
            gettext(c" (%d of %d)".as_ptr())
        };

        vim_snprintf_safelen!(buf, buflen, msg, wp.w_arg_idx + 1, argcount) as c_int
}

/// Make `*ffname` a full file name, set `*sfname` to `*ffname` if not NULL.
pub fn fname_expand(
    buf: *mut buf_T,
    ffname: *mut *mut c_char,
    sfname: *mut *mut c_char,
) -> Option<std::ffi::CString> {
    unsafe {
        #[cfg(not(windows))]
        let _ = buf;
        if (*ffname).is_null() {
            return None;
        }
        if (*sfname).is_null() {
            *sfname = *ffname;
        }
        let Some(mut full_cstr) = fix_fname(*ffname) else {
            *ffname = ptr::null_mut();
            *sfname = ptr::null_mut();
            return None;
        };
        if os_isdir(full_cstr.as_ptr()) {
            let mut name_vec: Vec<c_char> = cstr_to_cstring(*sfname)
                .into_bytes_with_nul()
                .into_iter()
                .map(|b| b as c_char)
                .collect();
            let name = name_vec.as_mut_ptr();
            let name_len = name_vec.len() - 1;
            if name_len > 0
                && after_pathsep(name, name.add(name_len)) != 0
                && name.add(name_len) > get_past_head(name)
            {
                *path_tail_with_sep(name) = NUL;
            }
            let full = fix_fname(name);
            drop(name_vec);
            if let Some(dir_full) = full {
                full_cstr = dir_full;
            }
            full_cstr = concat_fnames_cstr(full_cstr.as_ptr(), c"".as_ptr(), true);
        }

        #[cfg(windows)]
        if (*buf).b_p_bin == 0
            && let Some(rfname_cstr) = os_resolve_shortcut(full_cstr.as_ptr())
        {
            full_cstr = rfname_cstr;
            *sfname = full_cstr.as_ptr().cast_mut();
        }

        *ffname = full_cstr.as_ptr().cast_mut();
        Some(full_cstr)
    }
}

/// Return true if `buf` is a prompt buffer.
pub fn bt_prompt(buf: *mut buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == b'p' as c_char }
}

/// Return true if `buf` is the `|cmdwin|` scratch buffer.
pub fn bt_cmdwin(buf: *const buf_T) -> bool {
    !buf.is_null() && buf == g_st().cmdwin_buf.cast_const()
}

/// Open a window for a number of buffers.
pub fn ex_buffer_all(eap: &mut exarg_T) {
    unsafe {
        let mut wpnext: *mut win_T;
        let mut split_ret = OK;
        let mut open_wins = 0;
        let had_tab = g_st().cmdmod.cmod_tab;

        let count = if eap.addr_count == 0 {
            9999
        } else {
            eap.line2
        };

        let all = eap.cmdidx != cmdidx_T::CMD_unhide as c_int
            && eap.cmdidx != cmdidx_T::CMD_sunhide as c_int;

        reset_VIsual_and_resel();
        setpcmark();

        if had_tab > 0 {
            goto_tabpage_tp(g_st().first_tabpage, true, true);
        }
        loop {
            let mut tpnext = (*g_st().curtab).tp_next;
            let mut wp = if (*g_st().lastwin).w_floating {
                g_st().lastwin
            } else {
                g_st().firstwin
            };
            while !wp.is_null() {
                wpnext = if (*wp).w_floating {
                    if (*(*wp).w_prev).w_floating {
                        (*wp).w_prev
                    } else {
                        g_st().firstwin
                    }
                } else if (*wp).w_next.is_null() || (*(*wp).w_next).w_floating {
                    ptr::null_mut()
                } else {
                    (*wp).w_next
                };
                let size_mismatch = if (g_st().cmdmod.cmod_split & WSP_VERT) != 0 {
                    i64::from((*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height)
                        < i64::from(g_st().Rows)
                            - opt_vars().p_ch
                            - i64::from(tabline_height())
                            - i64::from(global_stl_height())
                } else {
                    (*wp).w_width != g_st().Columns
                };
                if ((*(*wp).w_buffer).b_nwindows > 1
                    || (*wp).w_floating
                    || size_mismatch
                    || (had_tab > 0 && wp != g_st().firstwin))
                    && g_st().firstwin != g_st().lastwin
                    && !(win_locked(&mut *wp) != 0 || (*(*wp).w_buffer).b_locked > 0)
                    && !is_ctx_win(wp)
                {
                    if win_close(wp, false, false) == FAIL {
                        break;
                    }
                    wpnext = if (*g_st().lastwin).w_floating {
                        g_st().lastwin
                    } else {
                        g_st().firstwin
                    };
                    tpnext = g_st().first_tabpage;
                    open_wins = 0;
                } else {
                    open_wins += 1;
                }
                wp = wpnext;
            }

            if had_tab == 0 || tpnext.is_null() {
                break;
            }
            goto_tabpage_tp(tpnext, true, true);
        }

        g_st().autocmd_no_enter += 1;
        win_enter(lastwin_nofloating(ptr::null_mut()), false);
        g_st().autocmd_no_leave += 1;
        let mut buf = g_st().firstbuf;
        while !buf.is_null() && open_wins < count {
            if (!all && (*buf).b_ml.ml_mfp.is_none()) || (*buf).b_p_bl == 0 {
                buf = (*buf).b_next;
                continue;
            }

            let mut wp: *mut win_T;
            if had_tab != 0 {
                wp = if (*buf).b_nwindows > 0 {
                    g_st().lastwin
                } else {
                    ptr::null_mut()
                };
            } else {
                wp = g_st().firstwin;
                while !wp.is_null() {
                    if !(*wp).w_floating && (*wp).w_buffer == buf {
                        break;
                    }
                    wp = (*wp).w_next;
                }
                if !wp.is_null() {
                    win_move_after(&mut *wp, &mut **curwin.as_ptr());
                }
            }

            if wp.is_null() && split_ret == OK {
                let mut bufref: bufref_T = bufref_T::new();
                set_bufref(&mut bufref, buf);
                let p_ea_save = opt_vars().p_ea;
                opt_vars().p_ea = 1;
                split_ret = win_split(0, WSP_ROOM | WSP_BELOW);
                open_wins += 1;
                opt_vars().p_ea = p_ea_save;
                if split_ret == FAIL {
                    buf = (*buf).b_next;
                    continue;
                }

                g_st().swap_exists_action = SEA_DIALOG;
                set_curbuf(buf, DOBUF_GOTO, (opt_vars().jop_flags & kOptJopFlagClean) == 0);
                if !bufref_valid(Some(&bufref)) {
                    g_st().swap_exists_action = SEA_NONE;
                    break;
                }
                if g_st().swap_exists_action == SEA_QUIT {
                    let mut cs: cleanup_T = cleanup_T::new();
                    enter_cleanup(&mut cs);
                    win_close(*curwin.as_ptr(), true, false);
                    open_wins -= 1;
                    g_st().swap_exists_action = SEA_NONE;
                    g_st().swap_exists_did_quit = true;
                    leave_cleanup(&mut cs);
                } else {
                    handle_swap_exists(None);
                }
            }

            os_breakcheck();
            if g_st().got_int {
                let _ = vgetc();
                break;
            }
            if aborting() {
                break;
            }
            if had_tab > 0 && i64::from(tabpage_index(ptr::null_mut())) <= opt_vars().p_tpm {
                g_st().cmdmod.cmod_tab = 9999;
            }
            buf = (*buf).b_next;
        }
        g_st().autocmd_no_enter -= 1;
        win_enter(g_st().firstwin, false);
        g_st().autocmd_no_leave -= 1;

        let mut wp = g_st().lastwin;
        while open_wins > count {
            let r = (buf_hide(&*(*wp).w_buffer)
                || !buf_is_changed(&*(*wp).w_buffer)
                || autowrite(&mut *(*wp).w_buffer, false) == OK)
                && !is_ctx_win(wp);
            if !win_valid(wp) {
                wp = g_st().lastwin;
            } else if r {
                win_close(wp, !buf_hide(&*(*wp).w_buffer), false);
                open_wins -= 1;
                wp = g_st().lastwin;
            } else {
                wp = (*wp).w_prev;
                if wp.is_null() {
                    break;
                }
            }
        }
    }
}

/// Process mode lines for the current file.
pub fn do_modelines(flags: c_int) {
    unsafe {
        let mut nmlines = opt_vars().p_mls as c_int;
        if (*g_st().curbuf).b_p_ml == 0 || nmlines == 0 {
            return;
        }

        if buf_st().modelines_entered != 0 {
            return;
        }

        buf_st().modelines_entered += 1;
        let mut lnum = 1;
        while (*g_st().curbuf).b_p_ml != 0 && lnum <= (*g_st().curbuf).b_ml.ml_line_count && lnum <= nmlines {
            if chk_modeline(lnum, flags) == FAIL {
                nmlines = 0;
            }
            lnum += 1;
        }

        lnum = (*g_st().curbuf).b_ml.ml_line_count;
        while (*g_st().curbuf).b_p_ml != 0
            && lnum > 0
            && lnum > nmlines
            && lnum > (*g_st().curbuf).b_ml.ml_line_count - nmlines
        {
            if chk_modeline(lnum, flags) == FAIL {
                nmlines = 0;
            }
            lnum -= 1;
        }
        buf_st().modelines_entered -= 1;
    }
}

fn chk_modeline(lnum: linenr_T, flags: c_int) -> c_int {
    unsafe {
        let mut e: *mut c_char;
        let mut retval = OK;

        let mut prev: c_int = -1;
        let mut s = ml_get(lnum);
        let mut line_end = s.add(ml_get_len(lnum) as usize);
        while *s != NUL {
            if prev == -1 || ascii_isspace(prev) {
                if (prev != -1 && (crate::memory::cstr_prefix_bytes(s, 3) == &b"ex:"[..3]))
                    || (crate::memory::cstr_prefix_bytes(s, 3) == &b"vi:"[..3])
                {
                    break;
                }
                if (*s == b'v' as c_char || *s == b'V' as c_char)
                    && *s.add(1) == b'i' as c_char
                    && *s.add(2) == b'm' as c_char
                {
                    let s3 = *s.add(3);
                    if s3 == b'<' as c_char || s3 == b'=' as c_char || s3 == b'>' as c_char {
                        e = s.add(4);
                    } else {
                        e = s.add(3);
                    }
                    let mut vers: i64 = 0;
                    if !try_getdigits(&raw mut e, &mut vers) {
                        prev = *s as u8 as c_int;
                        s = s.add(1);
                        continue;
                    }

                    let vim_version = i64::from(min_vim_version());
                    if *e == b':' as c_char
                        && (*s != b'V' as c_char
                            || (crate::memory::cstr_prefix_bytes(skipwhite(e.add(1)), 3) == &b"set"[..3]))
                        && (s3 == b':' as c_char
                            || (vim_version >= vers && (s3 as u8).is_ascii_digit())
                            || (vim_version < vers && s3 == b'<' as c_char)
                            || (vim_version > vers && s3 == b'>' as c_char)
                            || (vim_version == vers && s3 == b'=' as c_char))
                    {
                        break;
                    }
                }
            }
            prev = *s as u8 as c_int;
            s = s.add(1);
        }

        if *s == NUL {
            return retval;
        }

        loop {
            s = s.add(1);
            if *s.sub(1) == b':' as c_char {
                break;
            }
        }

        let len = line_end.offset_from(s) as usize;
        let linecopy_cstr = cstr_from_bytes(std::slice::from_raw_parts(s.cast::<u8>(), len));
        let mut linecopy_vec: Vec<c_char> = linecopy_cstr
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();
        s = linecopy_vec.as_mut_ptr();
        line_end = s.add(linecopy_vec.len() - 1);

        estack_push(ETYPE_MODELINE, c"modelines".as_ptr().cast_mut(), lnum);

        let mut end = false;
        while !end {
            s = skipwhite(s);
            if *s == NUL {
                break;
            }

            e = s;
            while *e != b':' as c_char && *e != NUL {
                if *e == b'\\' as c_char && *e.add(1) == b':' as c_char {
                    ptr::copy(e.add(1), e, line_end.offset_from(e.add(1)) as usize + 1);
                    line_end = line_end.sub(1);
                }
                e = e.add(1);
            }
            if *e == NUL {
                end = true;
            }

            if (crate::memory::cstr_prefix_bytes(s, 4) == &b"set "[..4]) || (crate::memory::cstr_prefix_bytes(s, 3) == &b"se "[..3]) {
                if *e != b':' as c_char {
                    break;
                }
                end = true;
                s = s.add(if *s.add(2) == b' ' as c_char { 3 } else { 4 });
            }
            *e = NUL;

            if *s != NUL {
                let secure_save = g_st().secure;
                let save_current_sctx = g_st().current_sctx;
                g_st().current_sctx.sc_sid = SID_MODELINE;
                g_st().current_sctx.sc_seq = 0;
                g_st().current_sctx.sc_lnum = lnum;
                g_st().secure = 1;

                retval = do_set(s, OPT_MODELINE | OPT_LOCAL | flags);

                g_st().secure = secure_save;
                g_st().current_sctx = save_current_sctx;
                if retval == FAIL {
                    break;
                }
            }
            s = if e == line_end { e } else { e.add(1) };
        }

        estack_pop();
        drop(linecopy_vec);

        retval
    }
}

/// Return true if `buf` is a help buffer.
pub fn bt_help(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && (*buf).b_help }
}

/// Return true if `buf` is a normal buffer, `'buftype'` is empty.
pub fn bt_normal(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == NUL }
}

/// Return true if `buf` is the quickfix buffer.
pub fn bt_quickfix(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == b'q' as c_char }
}

/// Return true if `buf` is a terminal buffer.
pub fn bt_terminal(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == b't' as c_char }
}

/// Return true if `buf` is a `"nofile"`, `"acwrite"`, `"terminal"` or `"prompt"` buffer.
pub fn bt_nofilename(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && ((*(*buf).b_p_bt == b'n' as c_char && *(*buf).b_p_bt.add(2) == b'f' as c_char)
                || *(*buf).b_p_bt == b'a' as c_char
                || !(*buf).terminal.is_null()
                || *(*buf).b_p_bt == b'p' as c_char)
    }
}

fn bt_nofileread(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && ((*(*buf).b_p_bt == b'n' as c_char && *(*buf).b_p_bt.add(2) == b'f' as c_char)
                || *(*buf).b_p_bt == b't' as c_char
                || *(*buf).b_p_bt == b'q' as c_char
                || *(*buf).b_p_bt == b'p' as c_char)
    }
}

/// Return true if `buf` has `'buftype'` set to `"nofile"`.
pub fn bt_nofile(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && *(*buf).b_p_bt == b'n' as c_char
            && *(*buf).b_p_bt.add(2) == b'f' as c_char
    }
}

/// Return true if `buf` is a `"nowrite"`, `"nofile"`, `"terminal"` or `"prompt"` buffer.
pub fn bt_dontwrite(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && (*(*buf).b_p_bt == b'n' as c_char
                || !(*buf).terminal.is_null()
                || *(*buf).b_p_bt == b'p' as c_char)
    }
}

pub fn bt_dontwrite_msg(buf: *const buf_T) -> bool {
    if bt_dontwrite(buf) {
            emsg(gettext(
                c"E382: Cannot write, 'buftype' option is set".as_ptr(),
            ));
            return true;
        }
        false
}

/// Return true if the buffer should be hidden, according to `'hidden'`, `":hide"` and `'bufhidden'`.
pub fn buf_hide(buf: &buf_T) -> bool {
    unsafe {
        match *buf.b_p_bh as u8 {
            b'u' | b'w' | b'd' => false,
            b'h' => true,
            _ => opt_vars().p_hid != 0 || (g_st().cmdmod.cmod_flags & CMOD_HIDE) != 0,
        }
    }
}

/// Return special buffer name or NULL when the buffer has a normal file name.
pub fn buf_spname(buf: *mut buf_T) -> *mut c_char {
    unsafe {
        if bt_quickfix(buf) {
            if (*buf).b_fnum == qf_stack_get_bufnr() {
                return gettext(msg_qflist.as_ptr());
            }
            return gettext(msg_loclist.as_ptr());
        }
        if bt_nofilename(buf) {
            if !(*buf).b_fname.is_null() {
                return (*buf).b_fname;
            }
            if bt_cmdwin(buf) {
                return gettext(c"[Command Line]".as_ptr());
            }
            if bt_prompt(buf) {
                return gettext(c"[Prompt]".as_ptr());
            }
            return gettext(c"[Scratch]".as_ptr());
        }
        if (*buf).b_fname.is_null() {
            return buf_get_fname(buf);
        }
        ptr::null_mut()
    }
}

/// Get `buf->b_fname`, use `"[No Name]"` if it is NULL.
pub fn buf_get_fname(buf: *const buf_T) -> *mut c_char {
    unsafe {
        if (*buf).b_fname.is_null() {
            gettext(c"[No Name]".as_ptr())
        } else {
            (*buf).b_fname
        }
    }
}

/// Set `'buflisted'` for `curbuf` to `on` and trigger autocommands if it changed.
pub fn set_buflisted(on: c_int) {
    unsafe {
        if on == (*g_st().curbuf).b_p_bl {
            return;
        }

        (*g_st().curbuf).b_p_bl = on;
        if on != 0 {
            apply_autocmds(
                EVENT_BUFADD,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        } else {
            apply_autocmds(
                EVENT_BUFDELETE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
    }
}

/// Read the file for `buf` again and check if the contents changed.
pub fn buf_contents_changed(buf: &mut buf_T) -> bool {
    let buf: *mut buf_T = &raw mut *buf;
    unsafe {
        let mut differ = true;

        let newbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if newbuf.is_null() {
            return true;
        }

        let mut ea: exarg_T = exarg_T::new();
        let _ea_cmd = prep_exarg(&mut ea, &*buf);

        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), newbuf, 0);

        block_autocmds();

        if ml_open(&mut *g_st().curbuf) == OK
            && readfile(
                (*buf).b_ffname,
                (*buf).b_fname,
                0,
                0,
                MAXLNUM,
                &raw mut ea,
                READ_NEW | READ_DUMMY,
                false,
            ) == OK
            && (*buf).b_ml.ml_line_count == (*g_st().curbuf).b_ml.ml_line_count
        {
            differ = false;
            for lnum in 1..=(*g_st().curbuf).b_ml.ml_line_count {
                if std::ffi::CStr::from_ptr(ml_get_buf(buf, lnum)).to_bytes() != std::ffi::CStr::from_ptr(ml_get(lnum)).to_bytes() {
                    differ = true;
                    break;
                }
            }
        }
        drop(_ea_cmd);

        ctx_restore(&mut aco);

        if g_st().curbuf != newbuf {
            wipe_buffer(newbuf, false);
        }

        unblock_autocmds();

        differ
    }
}

/// Wipe out a (typically temporary) buffer.
pub fn wipe_buffer(buf: *mut buf_T, aucmd: bool) {
    unsafe {
        if !aucmd {
            block_autocmds();
        }
        close_buffer(ptr::null_mut(), buf, DOBUF_WIPE, false, true, false);
        if !aucmd {
            unblock_autocmds();
        }
    }
}

pub fn buf_is_empty(buf: &mut buf_T) -> bool {
    unsafe { buf.b_ml.ml_line_count == 1 && *ml_get_buf(&raw mut *buf, 1) == NUL }
}

/// Increment `b:changedtick` value.
pub fn buf_inc_changedtick(buf: *mut buf_T) {
    unsafe {
        buf_set_changedtick(&mut *buf, buf_get_changedtick(&*buf) + 1);
    }
}

/// Set `b:changedtick`, also checking `b:` for consistency in debug build.
pub fn buf_set_changedtick(buf: &mut buf_T, changedtick: varnumber_T) {
    unsafe {
        let old_val = buf.changedtick_di.di_tv;

        let changedtick_di = tv_dict_find(buf.b_vars(), c"changedtick".as_ptr(), 11);
        debug_assert!(!changedtick_di.is_null());
        debug_assert!((*changedtick_di).di_tv.v_type == VarType::VAR_NUMBER);
        debug_assert!((*changedtick_di).di_tv.v_lock == VAR_FIXED);
        debug_assert!((*changedtick_di).di_flags == (DI_FLAGS_RO | DI_FLAGS_FIX));
        debug_assert!(changedtick_di == (&raw mut buf.changedtick_di).cast());
        let _ = changedtick_di;

        buf.changedtick_di.di_tv.vval = crate::eval::typval::TypvalVval::Number(changedtick);

        if tv_dict_is_watched(buf.b_vars()) {
            buf.b_locked += 1;
            tv_dict_watcher_notify(
                &mut *buf.b_vars(),
                buf.changedtick_di.di_key.as_ptr(),
                &raw mut buf.changedtick_di.di_tv,
                Some(&old_val),
            );
            buf.b_locked -= 1;
        }
    }
}

/// Read the given buffer contents into a string.
pub fn read_buffer_into(
    buf: *mut buf_T,
    start: linenr_T,
    end: linenr_T,
    sb: &mut Vec<c_char>,
) {
    unsafe {
        debug_assert!(!buf.is_null());

        if ((*buf).b_ml.ml_flags & ML_EMPTY) != 0 {
            return;
        }

        let mut written = 0usize;
        let mut len: usize;
        let mut lnum = start;
        let mut lp = ml_get_buf(buf, lnum);
        let mut lplen = ml_get_buf_len(buf, lnum) as usize;

        loop {
            if lplen == 0 {
                len = 0;
            } else if *lp.add(written) == NL {
                len = 1;
                sb.push(NUL);
            } else {
                let s = vim_strchr(lp.add(written), NL as c_int);
                len = if s.is_null() {
                    lplen - written
                } else {
                    s.offset_from(lp.add(written)) as usize
                };
                if len > 0 {
                    sb.extend_from_slice(std::slice::from_raw_parts(lp.add(written), len));
                }
            }

            if len == lplen - written {
                if lnum != end
                    || ((*buf).b_p_bin == 0 && (*buf).b_p_fixeol != 0)
                    || (lnum != (*buf).b_no_eol_lnum
                        && (lnum != (*buf).b_ml.ml_line_count || (*buf).b_p_eol != 0))
                {
                    sb.push(NL);
                }
                lnum += 1;
                if lnum > end {
                    break;
                }
                lp = ml_get_buf(buf, lnum);
                lplen = ml_get_buf_len(buf, lnum) as usize;
                written = 0;
            } else if len > 0 {
                written += len;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_percentage_and_bt_predicates_work() {
        assert_eq!(calc_percentage(50, 200), 25);
            assert_eq!(calc_percentage(2_000_000, 4_000_000), 50);
            assert!(!bt_help(ptr::null()));
            assert!(!bt_normal(ptr::null()));
            assert!(!bt_quickfix(ptr::null()));
            assert!(!bt_terminal(ptr::null()));
            assert!(!bt_nofile(ptr::null()));
            assert!(!bt_dontwrite(ptr::null()));
    }

    #[test]
    fn buf_valid_and_buflist_findpat_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            crate::main_init::ensure_test_editor_init();
            g_st().emsg_skip += 1;

            assert!(!buf_valid(ptr::null_mut()));

            let mut p1 = *b"test_file_path\0";
            let mut p2 = *b"file_path_test\0";
            let mut p3 = *b"path_test_file\0";

            let buf = buflist_new(
                p1.as_mut_ptr().cast(),
                p1.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            assert!(buf_valid(buf));
            close_buffer(ptr::null_mut(), buf, 0, false, false, false);
            assert!(buf_valid(buf));
            close_buffer(ptr::null_mut(), buf, DOBUF_UNLOAD, false, false, false);
            assert!(buf_valid(buf));
            close_buffer(ptr::null_mut(), buf, DOBUF_WIPE, false, false, false);
            assert!(!buf_valid(buf));

            let findpat = |pat: &CStr, allow_unlisted: bool| -> c_int {
                buflist_findpat(pat.as_ptr(), ptr::null(), allow_unlisted, false, false)
            };

            let buf1 = buflist_new(
                p1.as_mut_ptr().cast(),
                p1.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            let buf2 = buflist_new(
                p2.as_mut_ptr().cast(),
                p2.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            let buf3 = buflist_new(
                p3.as_mut_ptr().cast(),
                p3.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );

            assert_eq!(findpat(c"test_file_path", false), (*buf1).b_fnum);
            assert_eq!(findpat(c"test", false), (*buf1).b_fnum);
            assert_eq!(findpat(c"file", false), (*buf2).b_fnum);
            assert_eq!(findpat(c"path", false), (*buf3).b_fnum);
            assert_eq!(findpat(c"_test_", false), (*buf3).b_fnum);

            // Include / ignore unlisted buffers based on the flag
            close_buffer(ptr::null_mut(), buf3, DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"_test_", false), -1);
            assert_eq!(findpat(c"_test_", true), (*buf3).b_fnum);
            close_buffer(ptr::null_mut(), buf3, DOBUF_WIPE, false, false, false);
            assert_eq!(findpat(c"_test_", false), -1);
            assert_eq!(findpat(c"_test_", true), -1);

            // Unlisted vs listed preference
            close_buffer(ptr::null_mut(), buf1, DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"test", false), (*buf2).b_fnum);
            assert_eq!(findpat(c"test", true), (*buf2).b_fnum);
            close_buffer(ptr::null_mut(), buf2, DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"test", true), (*buf1).b_fnum);
            assert_eq!(findpat(c"test", false), -1);

            close_buffer(ptr::null_mut(), buf1, DOBUF_WIPE, false, false, false);
            close_buffer(ptr::null_mut(), buf2, DOBUF_WIPE, false, false, false);

            g_st().emsg_skip -= 1;
        }
    }
}
