//! Functions for dealing with the buffer structure (`src/nvim/buffer.c`).

use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO, cstr_as_string, tv_dict_is_watched};
use crate::api::win_config::{K_CTX_NO_DISPLAY, K_WIN_STYLE_MINIMAL, UIExtension};
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
    BUF_DIFF_FILTER, Cmdidx, WILD_BUFLASTUSED, WILD_HOME_REPLACE, cmdline_fuzzy_complete,
};
use crate::context::{CtxSwitch, INIT_FMARK, INIT_FMARKV, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor_col, check_cursor_lnum};
use crate::diff::{diff_buf_add, diff_buf_delete, diff_mode_buf, diffopt_hiddenoff};
use crate::digraph::keymap_init;
use crate::drawscreen::{UPD_NOT_VALID, redraw_later, redrawing, status_redraw_all};
use crate::errors::*;
use crate::eval::typval::{
    TypVal, TypvalVval, VarType, Varnumber, callback_free, tv_dict_add, tv_dict_alloc,
    tv_dict_find, tv_dict_item_copy, tv_dict_watcher_notify,
};
use crate::eval::vars::{init_var_dict, unref_var_dict, vars_clear};
use crate::ex_cmds::{ECMD_FORCEIT, ECMD_ONE, do_ecmd, getfile};
use crate::ex_cmds2::{autowrite, can_abandon, dialog_changed, dialog_close_terminal};
use crate::ex_docmd::{
    CMOD_CONFIRM, CMOD_HIDE, CMOD_KEEPALT, Exarg, do_cmdline_cmd, ex_errmsg, tabpage_new,
};
use crate::ex_eval::{ExCleanup, aborting, enter_cleanup, leave_cleanup};
use crate::ex_getln::text_or_buf_locked;
use crate::extmark::extmark_free_all;
use crate::file_search::vim_chdirfile;
#[cfg(any(
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly",
    target_os = "macos"
))]
use crate::fileio::is_dev_fd_file;
use crate::fileio::{
    READ_BUFFER, READ_DUMMY, READ_FIFO, READ_NEW, READ_NOFILE, READ_NOWINENTER, READ_STDIN,
    buf_check_timestamp, file_pat_to_reg_pat, prep_exarg, readfile, shorten_fnames,
};
use crate::fold::{clear_folding, fold_update_all};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, FuzmatchStr, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
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
    Fmark, WinInfo, clear_fmark, clrallmarks, fmarks_check_names, mark_adjust_buf,
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
use crate::normal::{end_visual_mode, reset_visual_and_resel};
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
    FullName_save, K_PATH_CMP_LITERAL, after_pathsep, concat_fnames_cstr, fix_fname, get_past_head,
    path_equal, path_fix_case, path_tail, path_tail_with_sep, to_slash,
};
use crate::plines::win_get_fill;
use crate::quickfix::qf_stack_get_bufnr;
use crate::regexp::{RE_MAGIC, Regmatch, vim_regcomp, vim_regexec, vim_regfree};
use crate::runtime::{ETYPE_MODELINE, estack_pop, estack_push};
use crate::spell::parse_spelllang;
use crate::state::{MAP_ALL_MODES, MODE_INSERT};
use crate::statusline::build_stl_str_hl;
use crate::strings::{
    RawCCharPtr, RawCCharPtrMut, gettext, ngettext, vim_snprintf, vim_snprintf_safelen, vim_strchr,
};
use crate::syntax::{Synblock, reset_synblock, syntax_clear};
use crate::terminal::{Terminal, terminal_check_size, terminal_close, terminal_running};
use crate::ui::{ui_call_set_icon, ui_call_set_title, ui_has};
use crate::undo::{u_clearallandblockfree, u_sync, undo_fmt_time};
use crate::version::min_vim_version;
use crate::window::{
    TabPageInfo, WSP_BELOW, WSP_ROOM, WSP_VERT, WinInfoInfo, check_can_set_curbuf_forceit,
    check_colorcolumn, close_windows, global_stl_height, goto_tabpage_tp, goto_tabpage_win,
    last_window, lastwin_nofloating, one_window, reset_binding, swbuf_goto_win_with_buf,
    tabline_height, tabpage_index, update_cwd, win_close, win_enter, win_locked, win_move_after,
    win_split, win_valid, win_valid_any_tab, window_layout_lock, window_layout_unlock,
};
use crate::winfloat::{win_float_update_preview, win_set_minimal_style};
use std::ffi::CStr;
use std::ptr;

pub(crate) const BCO_ENTER: i32 = 1;
pub(crate) const BCO_NOHELP: i32 = 4;
pub(crate) const BF_DUMMY: i32 = 0x80;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bufref {
    pub br_buf: *mut FileBuf,
    pub br_fnum: i32,
    pub br_buf_free_count: i32,
}

impl Bufref {
    #[inline]
    pub const fn new() -> Self {
        Self {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        }
    }
}

impl Default for Bufref {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

pub type ChangedtickDictItem = crate::eval::typval::Dictitem;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufSigncols {
    pub max: i32,
    pub last_max: i32,
    pub count: [i32; 9],
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
pub struct FileBuf {
    pub b_fnum: crate::msgpack_rpc::packer::ObjHandle,
    pub b_ml: crate::memline::Memline,
    pub b_next: *mut FileBuf,
    pub b_prev: *mut FileBuf,
    pub b_nwindows: i32,
    pub b_flags: i32,
    pub b_locked: i32,
    pub b_locked_split: i32,
    pub b_ro_locked: i32,
    pub b_ffname: RawCCharPtrMut,
    pub b_sfname: RawCCharPtrMut,
    pub b_fname: RawCCharPtrMut,
    pub b_ffname_cstr: Option<std::ffi::CString>,
    pub b_sfname_cstr: Option<std::ffi::CString>,
    pub file_id_valid: bool,
    pub file_id: FileID,
    pub b_changed: i32,
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
    pub b_orig_mode: i32,
    pub b_last_used: i64,
    pub b_namedm: [Fmark; 26],
    pub b_visual: crate::normal::Visualinfo,
    pub b_visual_mode_eval: i32,
    pub b_last_cursor: Fmark,
    pub b_last_insert: Fmark,
    pub b_last_change: Fmark,
    pub b_changelist: [Fmark; 100],
    pub b_changelistlen: i32,
    pub b_new_change: bool,
    pub b_chartab: [u64; 4],
    pub b_maphash: [Vec<crate::mapping::Mapblock>; 256],
    pub b_first_abbr: Vec<crate::mapping::Mapblock>,
    pub b_ucmds: Vec<crate::usercmd::Ucmd>,
    pub b_op_start: CursorPos,
    pub b_op_start_orig: CursorPos,
    pub b_op_end: CursorPos,
    pub b_marks_read: bool,
    pub b_modified_was_set: bool,
    pub b_did_filetype: bool,
    pub b_keep_filetype: bool,
    pub b_au_did_filetype: bool,
    pub b_u_headers: Vec<crate::undo::UHeader>,
    pub b_u_free_headers: Vec<usize>,
    pub b_u_oldhead: Option<usize>,
    pub b_u_newhead: Option<usize>,
    pub b_u_curhead: Option<usize>,
    pub b_u_numhead: i32,
    pub b_u_synced: bool,
    pub b_u_seq_last: i32,
    pub b_u_save_nr_last: i32,
    pub b_u_seq_cur: i32,
    pub b_u_time_cur: i64,
    pub b_u_save_nr_cur: i32,
    pub b_u_line_ptr: Option<std::ffi::CString>,
    pub b_u_line_lnum: i32,
    pub b_u_line_colnr: i32,
    pub b_scanned: bool,
    pub b_p_iminsert: i64,
    pub b_p_imsearch: i64,
    pub b_kmap_state: i16,
    pub b_kmap_ga: Vec<crate::digraph::Kmap>,
    pub b_p_initialized: bool,
    pub b_p_script_ctx: [crate::runtime::Sctx; 93],
    pub b_p_ac: i32,
    pub b_p_ai: i32,
    pub b_p_ai_nopaste: i32,
    pub b_p_bkc: RawCCharPtrMut,
    pub b_bkc_flags: u32,
    pub b_p_ci: i32,
    pub b_p_bin: i32,
    pub b_p_bomb: i32,
    pub b_p_bh: RawCCharPtrMut,
    pub b_p_bt: RawCCharPtrMut,
    pub b_p_busy: i64,
    pub b_has_qf_entry: i32,
    pub b_p_bl: i32,
    pub b_p_channel: i64,
    pub b_p_cin: i32,
    pub b_p_cino: RawCCharPtrMut,
    pub b_p_cink: RawCCharPtrMut,
    pub b_p_cinw: RawCCharPtrMut,
    pub b_p_cinsd: RawCCharPtrMut,
    pub b_p_com: RawCCharPtrMut,
    pub b_p_cms: RawCCharPtrMut,
    pub b_p_cot: RawCCharPtrMut,
    pub b_cot_flags: u32,
    pub b_p_cpt: RawCCharPtrMut,
    #[cfg(windows)]
    pub b_p_csl: RawCCharPtrMut,
    pub b_p_cpt_flags: u32,
    pub b_p_cpt_cb: Vec<crate::eval::typval::Callback>,
    pub b_p_cfu: crate::eval::typval::Callback,
    pub b_p_ofu: crate::eval::typval::Callback,
    pub b_p_tfu: crate::eval::typval::Callback,
    pub b_p_ffu: crate::eval::typval::Callback,
    pub b_p_eof: i32,
    pub b_p_eol: i32,
    pub b_p_fixeol: i32,
    pub b_p_et: i32,
    pub b_p_et_nobin: i32,
    pub b_p_et_nopaste: i32,
    pub b_p_fenc: RawCCharPtrMut,
    pub b_p_ff: RawCCharPtrMut,
    pub b_p_ft: RawCCharPtrMut,
    pub b_p_follow: i32,
    pub b_p_fo: RawCCharPtrMut,
    pub b_p_flp: RawCCharPtrMut,
    pub b_p_inf: i32,
    pub b_p_isk: RawCCharPtrMut,
    pub b_p_def: RawCCharPtrMut,
    pub b_p_inc: RawCCharPtrMut,
    pub b_p_inex: crate::eval::typval::Callback,
    pub b_p_inex_flags: u32,
    pub b_p_inde: crate::eval::typval::Callback,
    pub b_p_inde_flags: u32,
    pub b_p_indk: RawCCharPtrMut,
    pub b_p_fp: RawCCharPtrMut,
    pub b_p_fex: crate::eval::typval::Callback,
    pub b_p_fex_flags: u32,
    pub b_p_fs: i32,
    pub b_p_kp: RawCCharPtrMut,
    pub b_p_lisp: i32,
    pub b_p_lop: RawCCharPtrMut,
    pub b_p_menc: RawCCharPtrMut,
    pub b_p_mps: RawCCharPtrMut,
    pub b_p_ml: i32,
    pub b_p_ml_nobin: i32,
    pub b_p_ma: i32,
    pub b_p_nf: RawCCharPtrMut,
    pub b_p_pi: i32,
    pub b_p_qe: RawCCharPtrMut,
    pub b_p_ro: i32,
    pub b_p_sw: i64,
    pub b_p_scbk: i64,
    pub b_p_si: i32,
    pub b_p_sts: i64,
    pub b_p_sts_nopaste: i64,
    pub b_p_sua: RawCCharPtrMut,
    pub b_p_swf: i32,
    pub b_p_smc: i64,
    pub b_p_syn: RawCCharPtrMut,
    pub b_p_ts: i64,
    pub b_p_tw: i64,
    pub b_p_tw_nobin: i64,
    pub b_p_tw_nopaste: i64,
    pub b_p_wm: i64,
    pub b_p_wm_nobin: i64,
    pub b_p_wm_nopaste: i64,
    pub b_p_vsts: RawCCharPtrMut,
    pub b_p_vsts_array: Vec<i32>,
    pub b_p_vsts_nopaste: Option<std::ffi::CString>,
    pub b_p_vts: RawCCharPtrMut,
    pub b_p_vts_array: Vec<i32>,
    pub b_p_keymap: RawCCharPtrMut,
    pub b_p_gefm: RawCCharPtrMut,
    pub b_p_gp: RawCCharPtrMut,
    pub b_p_mp: RawCCharPtrMut,
    pub b_p_efm: RawCCharPtrMut,
    pub b_p_ep: RawCCharPtrMut,
    pub b_p_path: RawCCharPtrMut,
    pub b_p_ar: i32,
    pub b_p_tags: RawCCharPtrMut,
    pub b_p_tc: RawCCharPtrMut,
    pub b_tc_flags: u32,
    pub b_p_dict: RawCCharPtrMut,
    pub b_p_dia: RawCCharPtrMut,
    pub b_p_tsr: RawCCharPtrMut,
    pub b_p_tsrfu: crate::eval::typval::Callback,
    pub b_p_ul: i64,
    pub b_p_udf: i32,
    pub b_p_lw: RawCCharPtrMut,
    pub b_ind_level: i32,
    pub b_ind_open_imag: i32,
    pub b_ind_no_brace: i32,
    pub b_ind_first_open: i32,
    pub b_ind_open_extra: i32,
    pub b_ind_close_extra: i32,
    pub b_ind_open_left_imag: i32,
    pub b_ind_jump_label: i32,
    pub b_ind_case: i32,
    pub b_ind_case_code: i32,
    pub b_ind_case_break: i32,
    pub b_ind_param: i32,
    pub b_ind_func_type: i32,
    pub b_ind_comment: i32,
    pub b_ind_in_comment: i32,
    pub b_ind_in_comment2: i32,
    pub b_ind_cpp_baseclass: i32,
    pub b_ind_continuation: i32,
    pub b_ind_unclosed: i32,
    pub b_ind_unclosed2: i32,
    pub b_ind_unclosed_noignore: i32,
    pub b_ind_unclosed_wrapped: i32,
    pub b_ind_unclosed_whiteok: i32,
    pub b_ind_matching_paren: i32,
    pub b_ind_paren_prev: i32,
    pub b_ind_maxparen: i32,
    pub b_ind_maxcomment: i32,
    pub b_ind_scopedecl: i32,
    pub b_ind_scopedecl_code: i32,
    pub b_ind_java: i32,
    pub b_ind_js: i32,
    pub b_ind_keep_case_label: i32,
    pub b_ind_hash_comment: i32,
    pub b_ind_cpp_namespace: i32,
    pub b_ind_if_for_while: i32,
    pub b_ind_cpp_extern_c: i32,
    pub b_ind_pragma: i32,
    pub b_no_eol_lnum: i32,
    pub b_start_eof: i32,
    pub b_start_eol: i32,
    pub b_start_ffc: i32,
    pub b_start_fenc: Option<std::ffi::CString>,
    pub b_bad_char: i32,
    pub b_start_bomb: i32,
    pub b_bufvar: crate::eval::typval::ScopeDictDictItem,
    pub b_may_swap: bool,
    pub b_did_warn: bool,
    pub b_help: bool,
    pub b_spell: bool,
    pub b_prompt_text: Option<std::ffi::CString>,
    pub b_prompt_callback: crate::eval::typval::Callback,
    pub b_prompt_interrupt: crate::eval::typval::Callback,
    pub b_prompt_append_new_line: bool,
    pub b_prompt_insert: i32,
    pub b_prompt_start: Fmark,
    pub b_s: Synblock,
    pub b_signcols: BufSigncols,
    pub terminal: *mut Terminal,
    pub additional_data: Option<crate::search::AdditionalData>,
    pub b_mapped_ctrl_c: i32,
    pub b_marktree: crate::marktree::MarkTree,
    pub b_extmark_ns: std::collections::BTreeMap<u32, u32>,
    pub b_prev_line_count: i32,
    pub update_channels: Vec<u64>,
    pub update_callbacks: Vec<BufUpdateCallbacks>,
    pub update_need_codepoints: bool,
    pub deleted_bytes: usize,
    pub deleted_bytes2: usize,
    pub deleted_codepoints: usize,
    pub deleted_codeunits: usize,
    pub flush_count: i32,
    pub b_localdir: Option<std::ffi::CString>,
    pub b_prevdir: Option<std::ffi::CString>,
}

impl FileBuf {
    #[inline]
    pub fn terminal_mut(&self) -> &'static mut Terminal {
        unsafe { &mut *self.terminal }
    }

    pub const fn new() -> Self {
        const ZERO_POS: CursorPos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        const ZERO_SCTX: crate::runtime::Sctx = crate::runtime::Sctx {
            sc_sid: 0,
            sc_seq: 0,
            sc_lnum: 0,
            sc_chan: 0,
        };
        Self {
            b_fnum: 0,
            b_ml: crate::memline::Memline::new(),
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
            b_namedm: [const { Fmark::ZERO }; 26],
            b_visual: crate::normal::Visualinfo {
                vi_start: ZERO_POS,
                vi_end: ZERO_POS,
                vi_mode: 0,
                vi_curswant: 0,
            },
            b_visual_mode_eval: 0,
            b_last_cursor: Fmark::ZERO,
            b_last_insert: Fmark::ZERO,
            b_last_change: Fmark::ZERO,
            b_changelist: [const { Fmark::ZERO }; 100],
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
            b_prompt_start: Fmark::ZERO,
            b_s: Synblock::new(),
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
    pub fn b_vars(&self) -> &'static mut crate::eval::typval::TvDict {
        unsafe { &mut *self.b_bufvar.di_tv.vval.v_dict() }
    }
}

pub const GETF_SETMARK: i32 = 0x01;
pub const GETF_ALT: i32 = 0x02;
pub const GETF_SWITCH: i32 = 0x04;

pub const GETFILE_ERROR: i32 = 1;
pub const GETFILE_NOT_WRITTEN: i32 = 2;
pub const GETFILE_SAME_FILE: i32 = 0;
pub const GETFILE_OPEN_OTHER: i32 = -1;
pub const GETFILE_UNUSED: i32 = 8;

pub const BLN_CURBUF: i32 = 1;
pub const BLN_LISTED: i32 = 2;
pub const BLN_DUMMY: i32 = 4;
pub const BLN_NEW: i32 = 8;
pub const BLN_NOOPT: i32 = 16;
pub const BLN_NOCURWIN: i32 = 128;

pub const DOBUF_GOTO: i32 = 0;
pub const DOBUF_SPLIT: i32 = 1;
pub const DOBUF_UNLOAD: i32 = 2;
pub const DOBUF_DEL: i32 = 3;
pub const DOBUF_WIPE: i32 = 4;

pub const DOBUF_CURRENT: i32 = 0;
pub const DOBUF_FIRST: i32 = 1;
pub const DOBUF_LAST: i32 = 2;
pub const DOBUF_MOD: i32 = 3;

pub const DOBUF_FORCEIT: i32 = 1;
pub const DOBUF_SKIPHELP: i32 = 4;

pub const BFA_DEL: i32 = 1;
pub const BFA_WIPE: i32 = 2;
pub const BFA_KEEP_UNDO: i32 = 4;
pub const BFA_IGNORE_ABORT: i32 = 8;

const K_BFF_CLEAR_WIN_INFO: i32 = 1;
const K_BFF_INIT_CHANGEDTICK: i32 = 2;

pub(crate) const NOTDONE: i32 = 2;
pub(crate) const MAXLNUM: Linenr = 0x7fffffff;
pub(crate) const NMARKS: usize = 26;
pub(crate) const JUMPLISTSIZE: usize = 100;
pub(crate) const DO_NOT_FREE_CNT: i32 = i32::MAX / 2;

pub(crate) const BF_CHECK_RO: i32 = 0x02;
pub(crate) const BF_NEVERLOADED: i32 = 0x04;
pub(crate) const BF_NOTEDITED: i32 = 0x08;
pub(crate) const BF_NEW: i32 = 0x10;
pub(crate) const BF_READERR: i32 = 0x40;
pub(crate) const BF_WRITE_MASK: i32 = BF_NOTEDITED + BF_NEW + BF_READERR;

pub(crate) const VALID_TOPLINE: i32 = 0x80;
pub(crate) const MF_DIRTY_YES: i32 = 1;
pub(crate) const MF_DIRTY_YES_NOSYNC: i32 = 2;

pub(crate) const K_SHM_RO: i32 = b'r' as i32;
const K_SHM_MOD: i32 = b'm' as i32;
pub(crate) const K_SHM_FILEINFO: i32 = b'F' as i32;
const K_CPO_INTMOD: i32 = b'i' as i32;

pub(crate) const K_MARK_ADJUST_NORMAL: i32 = 0;
pub(crate) const K_EXTMARK_NO_UNDO: i32 = 2;

pub(crate) const SEA_NONE: i32 = 0;
pub(crate) const SEA_DIALOG: i32 = 1;
pub(crate) const SEA_QUIT: i32 = 2;
pub(crate) const SEA_RECOVER: i32 = 3;

pub(crate) const K_OPT_JOP_FLAG_VIEW: u32 = 0x02;
const K_OPT_JOP_FLAG_CLEAN: u32 = 0x04;

const K_OPT_SWB_FLAG_SPLIT: u32 = 0x04;
pub(crate) const K_OPT_SWB_FLAG_NEWTAB: u32 = 0x08;
pub(crate) const K_OPT_SWB_FLAG_VSPLIT: u32 = 0x10;

pub(crate) const K_CD_CAUSE_BUFFER: i32 = 2;
pub(crate) const K_CD_CAUSE_AUTO: i32 = 3;

pub(crate) const BCO_ALWAYS: i32 = 2;

pub(crate) const KEYMAP_INIT: i16 = 1;

pub(crate) const VAR_FIXED: i32 = 2;
pub(crate) const VAR_SCOPE: i32 = 1;

pub(crate) const NO_LOCAL_UNDOLEVEL: OptInt = -123456;

pub(crate) const K_UIMESSAGES: UIExtension = 4;

pub(crate) const STL_IN_ICON: i32 = 1;
pub(crate) const STL_IN_TITLE: i32 = 2;
const K_OPT_ICONSTRING: OptIndex = 139;
const K_OPT_TITLESTRING: OptIndex = 331;

pub(crate) const SID_MODELINE: i32 = -1;
pub(crate) const OPT_MODELINE: i32 = 0x04;

use crate::charset::{NL, NUL};
use crate::os::fs::MAXPATHL;
use crate::search::FORWARD;

#[repr(C)]
struct Bufmatch {
    buf: *mut FileBuf,
    match_: std::ffi::CString,
}

struct BufferModuleState {
    buf_free_count: i32,
    top_file_num: i32,
    no_position: Fmark,
    lasttitle: Option<std::ffi::CString>,
    lasticon: Option<std::ffi::CString>,
    modelines_entered: i32,
}

impl BufferModuleState {
    const fn new() -> Self {
        Self {
            buf_free_count: 0,
            top_file_num: 1,
            no_position: Fmark {
                mark: CursorPos {
                    lnum: 1,
                    col: 0,
                    coladd: 0,
                },
                fnum: 0,
                timestamp: 0,
                view: INIT_FMARKV,
                additional_data: None,
            },
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
fn buf_is_changed(buf: &mut FileBuf) -> bool {
    !bt_dontwrite(Some(&*buf)) && (buf.b_changed != 0 || file_ff_differs(buf, true))
}

#[inline]
fn curbuf_is_changed() -> bool {
    buf_is_changed(curbuf_mut())
}

#[inline]
pub fn buf_get_changedtick(buf: &FileBuf) -> Varnumber {
    buf.changedtick_di.di_tv.vval.v_number()
}

#[inline]
pub fn buf_meta_total(b: &FileBuf, m: usize) -> u32 {
    b.b_marktree.meta_root[m]
}

#[inline]
pub fn first_win_in_tab(tp: &mut TabPageInfo) -> &'static mut WinInfoInfo {
    unsafe {
        if (&raw mut *tp) == g_st().curtab {
            &mut *g_st().firstwin
        } else {
            &mut *tp.tp_firstwin
        }
    }
}

#[inline]
pub fn modifiable(buf: &FileBuf) -> bool {
    buf.b_p_ma != 0 && buf.b_ro_locked == 0
}

fn trigger_undo_ftplugin(buf: &mut FileBuf, win: &mut WinInfoInfo) {
    window_layout_lock();
    buf.b_locked += 1;
    win.w_locked += 1;
    do_cmdline_cmd(c"if exists('b:undo_ftplugin') | exe b:undo_ftplugin | endif");
    buf.b_locked -= 1;
    win.w_locked -= 1;
    window_layout_unlock();
}

/// Calculate the percentage that `part` is of the `whole`.
pub fn calc_percentage(part: i64, whole: i64) -> i32 {
    if part > 1_000_000 {
        (part / (whole / 100)) as i32
    } else {
        ((part * 100) / whole) as i32
    }
}

/// Return the highest possible buffer number.
pub fn get_highest_fnum() -> i32 {
    buf_st().top_file_num - 1
}

/// Read data from buffer for retrying.
fn read_buffer(read_stdin: bool, eap: Option<&Exarg>, flags: i32) -> bool {
    let silent = shortmess(K_SHM_FILEINFO);

    let mut line_count = curbuf_mut().b_ml.ml_line_count;
    let mut retval = readfile(
        if read_stdin {
            ptr::null_mut()
        } else {
            curbuf_mut().b_ffname
        },
        if read_stdin {
            ptr::null_mut()
        } else {
            curbuf_mut().b_fname
        },
        line_count,
        0,
        MAXLNUM,
        (eap.is_some()).then(|| eap.unwrap()),
        flags | READ_BUFFER,
        silent,
    );
    if retval == OK {
        while line_count > 0 {
            line_count -= 1;
            ml_delete(1);
        }
    } else {
        while curbuf_mut().b_ml.ml_line_count > line_count {
            ml_delete(line_count);
        }
    }
    curwin_mut().w_cursor.lnum = 1;
    curwin_mut().w_cursor.col = 0;

    if read_stdin {
        if !g_st().readonlymode && !buf_is_empty(curbuf_mut()) {
            changed(crate::globals::curbuf_mut());
        } else if retval == OK {
            unchanged(curbuf_mut(), false, true);
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
    retval == OK
}

/// Ensure buffer `buf` is loaded.
pub fn buf_ensure_loaded(buf: &mut FileBuf) -> bool {
    if buf.b_ml.ml_mfp.is_some() {
        return true;
    }

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, &raw mut *buf);
    let mut aco: CtxSwitch = CtxSwitch::new();
    ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), &raw mut *buf, 0);
    let status = open_buffer(false, None, 0);
    ctx_restore(&mut aco);
    status != FAIL && bufref_valid(Some(&bufref)) && buf.b_ml.ml_mfp.is_some()
}

/// Open current buffer, that is: open the memfile and read the file into
/// memory.
pub fn open_buffer(read_stdin: bool, eap: Option<&Exarg>, flags_arg: i32) -> i32 {
    unsafe {
        let mut flags = flags_arg;
        let mut retval = OK;
        let mut old_curbuf: Bufref = Bufref::new();
        let old_tw = curbuf_mut().b_p_tw;
        #[cfg(unix)]
        let mut read_fifo = false;
        #[cfg(not(unix))]
        let read_fifo = false;
        let silent = shortmess(K_SHM_FILEINFO);

        if g_st().readonlymode
            && !curbuf_mut().b_ffname.is_null()
            && (curbuf_mut().b_flags & BF_NEVERLOADED) != 0
        {
            curbuf_mut().b_p_ro = 1;
        }

        if !ml_open(curbuf_mut()) {
            close_buffer(curwin_ptr(), g_st().curbuf, 0, false, false, false);

            g_st().curbuf = ptr::null_mut();
            for buf in for_all_buffers_ptr() {
                if buf.b_ml.ml_mfp.is_some() {
                    g_st().curbuf = buf;
                    break;
                }
            }

            if g_st().curbuf.is_null() {
                emsg(gettext(c"E82: Cannot allocate any buffer, exiting..."));
                g_st().v_dying = 2;
                getout(2);
            }

            emsg(gettext(c"E83: Cannot allocate buffer, using other one..."));
            enter_buffer(g_st().curbuf);
            if old_tw != curbuf_mut().b_p_tw {
                check_colorcolumn(None::<&CStr>, Some(crate::globals::curwin_mut()));
            }
            return FAIL;
        }

        if curbuf_mut().b_ml.ml_mfp.is_some() {
            (*curbuf_mut().b_ml.mfp_ptr()).mf_dirty = MF_DIRTY_YES_NOSYNC;
        }

        set_bufref(&mut old_curbuf, g_st().curbuf);
        curbuf_mut().b_modified_was_set = false;

        curwin_mut().w_valid = 0;

        if bt_nofileread(Some(crate::globals::curbuf_mut())) {
            flags |= READ_NOFILE;
        }

        if !curbuf_mut().b_ffname.is_null() {
            #[cfg(unix)]
            let save_bin = curbuf_mut().b_p_bin;
            #[cfg(unix)]
            {
                const S_IFMT: i32 = 0o170000;
                const S_IFIFO: i32 = 0o010000;
                const S_IFSOCK: i32 = 0o140000;
                let perm = os_getperm(curbuf_mut().b_ffname);
                if perm >= 0 {
                    let fmt = perm & S_IFMT;
                    #[cfg(any(
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                        target_os = "dragonfly",
                        target_os = "macos"
                    ))]
                    let is_chr_fd = fmt == 0o020000 && is_dev_fd_file(curbuf_mut().b_ffname);
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
                    curbuf_mut().b_p_bin = 1;
                }
            }

            retval = readfile(
                curbuf_mut().b_ffname,
                curbuf_mut().b_fname,
                0,
                0,
                MAXLNUM,
                (eap.is_some()).then(|| eap.unwrap()),
                flags | READ_NEW | if read_fifo { READ_FIFO } else { 0 },
                silent,
            );
            #[cfg(unix)]
            if read_fifo {
                curbuf_mut().b_p_bin = save_bin;
                if retval == OK {
                    retval = if read_buffer(false, eap, flags) {
                        OK
                    } else {
                        FAIL
                    };
                }
            }

            if bt_help(Some(crate::globals::curbuf_mut())) {
                get_local_additions();
            }
        } else if read_stdin {
            let save_bin = curbuf_mut().b_p_bin;

            curbuf_mut().b_p_bin = 1;
            retval = readfile(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                None,
                flags | (READ_NEW + READ_STDIN),
                silent,
            );
            curbuf_mut().b_p_bin = save_bin;
            if retval == OK {
                retval = if read_buffer(true, (eap.is_some()).then(|| eap.unwrap()), flags) {
                    OK
                } else {
                    FAIL
                };
            }
        }

        if curbuf_mut().b_ml.ml_mfp.is_some() {
            let mfp = curbuf_mut().b_ml.mfp_ptr();
            if (*mfp).mf_dirty == MF_DIRTY_YES_NOSYNC {
                (*mfp).mf_dirty = MF_DIRTY_YES;
            }
        }

        if (curbuf_mut().b_flags & BF_NEVERLOADED) != 0 {
            buf_init_isk_chartab(curbuf_mut());
            parse_cino(curbuf_mut());
        }

        if (g_st().got_int && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_INTMOD))
            || curbuf_mut().b_modified_was_set
            || (aborting() && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_INTMOD))
        {
            changed(crate::globals::curbuf_mut());
        } else if retval != FAIL && !read_stdin && !read_fifo {
            unchanged(curbuf_mut(), false, true);
        }
        save_file_ff(curbuf_mut());

        let tick = buf_get_changedtick(curbuf_mut());
        curbuf_mut().b_last_changedtick = tick;
        curbuf_mut().b_last_changedtick_i = tick;
        curbuf_mut().b_last_changedtick_pum = tick;

        if aborting() {
            curbuf_mut().b_flags |= BF_READERR;
        }

        if retval == NOTDONE
            && crate::memory::cstr_is_empty(curbuf_mut().b_p_ft)
            && !curbuf_mut().b_ffname.is_null()
            && after_pathsep(
                curbuf_mut().b_ffname,
                curbuf_mut()
                    .b_ffname
                    .wrapping_add(crate::memory::cstr_len(curbuf_mut().b_ffname)),
            ) != 0
            && augroup_exists(c"filetypedetect")
        {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, g_st().curbuf);
            do_doautocmd(c"filetypedetect BufRead", false, None);
            if !bufref_valid(Some(&bufref)) || g_st().curbuf != old_curbuf.br_buf || aborting() {
                return FAIL;
            }
        }

        fold_update_all(curwin_mut());

        if (curwin_mut().w_valid & VALID_TOPLINE) == 0 {
            curwin_mut().w_topline = 1;
            curwin_mut().w_topfill = 0;
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
            curbuf_mut().b_flags &= !(BF_CHECK_RO | BF_NEVERLOADED);

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
pub fn set_bufref(bufref: &mut Bufref, buf: *mut FileBuf) {
    bufref.br_buf = buf;
    bufref.br_fnum = if buf.is_null() {
        0
    } else {
        unsafe { (*buf).b_fnum }
    };
    bufref.br_buf_free_count = buf_st().buf_free_count;
}

/// Return true if `bufref->br_buf` points to the same buffer as when
/// `set_bufref()` was called and it is a valid buffer.
pub fn bufref_valid(bufref: Option<&Bufref>) -> bool {
    if bufref.unwrap().br_buf_free_count == buf_st().buf_free_count {
        true
    } else {
        buf_valid(bufref.unwrap().br_buf)
            && bufref.unwrap().br_fnum == unsafe { (*bufref.unwrap().br_buf).b_fnum }
    }
}

/// Check that `buf` points to a valid buffer in the buffer list.
pub fn buf_valid(buf: *const FileBuf) -> bool {
    if buf.is_null() {
        return false;
    }
    for bp in for_all_buffers_rev_ptr() {
        if ptr::eq(bp, buf) {
            return true;
        }
    }
    false
}

fn can_unload_buffer(buf: &mut FileBuf) -> bool {
    let mut can_unload = buf.b_locked == 0;

    if can_unload && g_st().updating_screen {
        for wp in for_all_windows_mut() {
            if wp.w_buffer == buf {
                can_unload = false;
                break;
            }
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
            gettext(E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR),
            if !fname.is_null() {
                fname
            } else {
                c"[No Name]".as_ptr()
            },
        );
    }
    can_unload
}

pub fn buf_close_terminal(buf: &mut FileBuf) {
    debug_assert!(!buf.terminal.is_null());
    buf.b_locked += 1;
    terminal_close((&raw mut buf.terminal).cast::<*mut Terminal>(), -1);
    buf.b_locked -= 1;
}

/// Close the link to a buffer.
pub fn close_buffer(
    win: *mut WinInfoInfo,
    buf: *mut FileBuf,
    action: i32,
    abort_if_last: bool,
    ignore_abort: bool,
    set_context: bool,
) -> bool {
    unsafe {
        let buf = &mut *buf;
        let mut unload_buf = action != 0;
        let mut del_buf = action == DOBUF_DEL || action == DOBUF_WIPE;
        let mut wipe_buf = action == DOBUF_WIPE;

        let is_curwin =
            !(curwin_ptr()).is_null() && curwin_ptr() == win && curwin_mut().w_buffer == buf;
        let the_curtab = g_st().curtab;

        if buf.terminal.is_null() {
            let bh0 = crate::memory::cstr_first_byte(buf.b_p_bh);
            if bh0 == b'd' {
                del_buf = true;
                unload_buf = true;
            } else if bh0 == b'w' {
                del_buf = true;
                unload_buf = true;
                wipe_buf = true;
            } else if bh0 == b'u' {
                unload_buf = true;
            }
        }

        if !buf.terminal.is_null() && (unload_buf || del_buf || wipe_buf) {
            unload_buf = true;
            del_buf = true;
            wipe_buf = true;
        }

        if (del_buf || wipe_buf) && !can_unload_buffer(&mut *buf) {
            return false;
        }

        let mut win_valid = win_valid_any_tab(win);
        if set_context && win_valid && (*win).w_buffer == buf {
            if buf.b_nwindows == 1 {
                set_last_cursor(&mut *win);
            }
            buflist_setfpos(
                &mut *buf,
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

        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, buf);

        if win_valid && (*win).w_buffer == buf && buf.b_nwindows == 1 {
            buf.b_locked += 1;
            buf.b_locked_split += 1;
            if apply_autocmds_win(
                EVENT_BUFWINLEAVE,
                buf.b_fname,
                buf.b_fname,
                false,
                buf,
                win.as_ref(),
            ) && !bufref_valid(Some(&bufref))
            {
                emsg(gettext(E_AUABORT));
                return false;
            }
            buf.b_locked -= 1;
            buf.b_locked_split -= 1;
            if abort_if_last && one_window(&mut *win, ptr::null_mut()) {
                emsg(gettext(E_AUABORT));
                return false;
            }

            if !unload_buf {
                buf.b_locked += 1;
                buf.b_locked_split += 1;
                if apply_autocmds(EVENT_BUFHIDDEN, buf.b_fname, buf.b_fname, false, buf)
                    && !bufref_valid(Some(&bufref))
                {
                    emsg(gettext(E_AUABORT));
                    return false;
                }
                buf.b_locked -= 1;
                buf.b_locked_split -= 1;
                if abort_if_last && one_window(&mut *win, ptr::null_mut()) {
                    emsg(gettext(E_AUABORT));
                    return false;
                }
            }
            if !ignore_abort && aborting() {
                return false;
            }
            win_valid = win_valid && win_valid_any_tab(win);
        }

        if is_curwin && curwin_ptr() != win && win_valid {
            block_autocmds();
            goto_tabpage_win(&mut *the_curtab, win);
            unblock_autocmds();
        }

        if bt_quickfix(Some(&*buf)) && win_valid && (*win).w_buffer == buf {
            (*win).w_onebuf_opt.wo_wfh = 0;
        }

        let hiding_buf =
            buf.b_nwindows <= 0 || (win_valid && (*win).w_buffer == buf && buf.b_nwindows == 1);

        if diffopt_hiddenoff() && !unload_buf && hiding_buf {
            diff_buf_delete(buf);
        }

        if !hiding_buf || !unload_buf {
            return false;
        }

        if buf.b_ffname.is_null() {
            del_buf = true;
        }

        if !buf_freeall(
            &mut *buf,
            (if del_buf { BFA_DEL } else { 0 })
                + (if wipe_buf { BFA_WIPE } else { 0 })
                + (if ignore_abort { BFA_IGNORE_ABORT } else { 0 }),
        ) {
            return false;
        }

        let mut clear_w_buf = false;
        win_valid = win_valid && win_valid_any_tab(win);
        if win_valid && (*win).w_buffer == buf {
            buf.b_nwindows -= 1;
            clear_w_buf = true;
        }

        if wipe_buf
            && buf.b_nwindows <= 0
            && (!buf.b_prev.is_null() || !buf.b_next.is_null() || mem_st().entered_free_all_mem)
        {
            if clear_w_buf {
                (*win).w_buffer = ptr::null_mut();
            }
            for tp in for_all_tabpages_ptr() {
                for wp in for_all_windows_in_tab_mut(&*tp) {
                    mark_forget_file(wp, buf.b_fnum);
                }
            }
            (*buf).clear_fnames();
            if buf.b_prev.is_null() {
                g_st().firstbuf = buf.b_next;
            } else {
                (*buf.b_prev).b_next = buf.b_next;
            }
            if buf.b_next.is_null() {
                g_st().lastbuf = buf.b_prev;
            } else {
                (*buf.b_next).b_prev = buf.b_prev;
            }
            free_buffer(buf);
        } else {
            if del_buf {
                free_buffer_stuff(&mut *buf, K_BFF_CLEAR_WIN_INFO | K_BFF_INIT_CHANGEDTICK);
                buf.b_flags = BF_CHECK_RO | BF_NEVERLOADED;
                buf.b_p_initialized = false;
            }
            g_st().textlock += 1;
            buf_clear_file(&mut *buf);
            g_st().textlock -= 1;
            if clear_w_buf {
                (*win).w_buffer = ptr::null_mut();
            }
            if del_buf {
                buf.b_p_bl = 0;
            }
        }

        debug_assert!(!win_valid || !is_curwin || win == curwin_ptr());
        true
    }
}

/// Make buffer not contain a file.
pub fn buf_clear_file(buf: &mut FileBuf) {
    buf.b_ml.ml_line_count = 1;
    unchanged(&mut *buf, true, true);
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
    let line_count = curbuf_mut().b_ml.ml_line_count;
    extmark_free_all(curbuf_mut());
    mc_buf_free(curbuf_mut());
    while (curbuf_mut().b_ml.ml_flags & ML_EMPTY) == 0 {
        ml_delete(1);
    }
    deleted_lines_mark(1, line_count);
}

/// Free all things allocated for a buffer that are related to the file.
pub fn buf_freeall(buf: &mut FileBuf, flags: i32) -> bool {
    let is_curbuf = (&raw mut *buf) == g_st().curbuf;
    let is_curwin = !(curwin_ptr()).is_null() && curwin_mut().w_buffer == (&raw mut *buf);
    let the_curwin = curwin_ptr();
    let the_curtab = g_st().curtab;

    buf.b_locked += 1;
    buf.b_locked_split += 1;

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, (&raw mut *buf));

    if !buf.terminal.is_null() {
        buf_close_terminal(buf);
    }

    buf_updates_unload(buf, false);

    if buf.b_ml.ml_mfp.is_some()
        && apply_autocmds(
            EVENT_BUFUNLOAD,
            buf.b_fname,
            buf.b_fname,
            false,
            (&raw mut *buf),
        )
        && !bufref_valid(Some(&bufref))
    {
        return false;
    }
    if (flags & BFA_DEL) != 0
        && buf.b_p_bl != 0
        && apply_autocmds(
            EVENT_BUFDELETE,
            buf.b_fname,
            buf.b_fname,
            false,
            (&raw mut *buf),
        )
        && !bufref_valid(Some(&bufref))
    {
        return false;
    }
    if (flags & BFA_WIPE) != 0
        && apply_autocmds(
            EVENT_BUFWIPEOUT,
            buf.b_fname,
            buf.b_fname,
            false,
            (&raw mut *buf),
        )
        && !bufref_valid(Some(&bufref))
    {
        return false;
    }
    buf.b_locked -= 1;
    buf.b_locked_split -= 1;

    if is_curwin && curwin_ptr() != the_curwin && win_valid_any_tab(the_curwin) {
        block_autocmds();
        goto_tabpage_win(unsafe { &mut *the_curtab }, the_curwin);
        unblock_autocmds();
    }
    if (flags & BFA_IGNORE_ABORT) == 0 && aborting() {
        return false;
    }

    if (&raw mut *buf) == g_st().curbuf && !is_curbuf {
        return false;
    }

    if (&raw mut *buf) == g_st().curbuf
        && g_st().visual_global.active
        && !mem_st().entered_free_all_mem
    {
        end_visual_mode();
    }

    diff_buf_delete(&raw mut *buf);
    if !(curwin_ptr()).is_null() && curwin_mut().w_buffer == (&raw mut *buf) {
        reset_synblock(curwin_mut());
    }

    for tp in for_all_tabpages_ptr() {
        for win in for_all_windows_in_tab_mut(&*tp) {
            if win.w_buffer == (&raw mut *buf) {
                clear_folding(win);
            }
        }
    }

    if !buf.terminal.is_null() {
        block_autocmds();
        buf_close_terminal(buf);
        unblock_autocmds();
    }

    let count = buf.b_ml.ml_line_count;
    ml_close(buf, 1);
    buf.b_ml.ml_line_count = 0;
    mc_buf_clear(buf);

    if bt_nofilename(Some(&*buf)) && !g_st().exiting {
        mark_adjust_buf(
            buf,
            1,
            count,
            MAXLNUM,
            -count,
            false,
            K_MARK_ADJUST_NORMAL,
            K_EXTMARK_NO_UNDO,
        );
    }

    if (flags & BFA_KEEP_UNDO) == 0 {
        u_clearallandblockfree(buf);
    }
    syntax_clear(&mut buf.b_s);
    buf.b_flags &= !BF_READERR;
    true
}

impl FileBuf {
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

impl Drop for FileBuf {
    fn drop(&mut self) {
        self.clear_fnames();
        free_buf_options(&mut *self, true);
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

fn free_buffer(buf: *mut FileBuf) {
    unsafe {
        g_st().buffer_handles.remove(&(*buf).b_fnum);
        buf_st().buf_free_count += 1;
        free_buffer_stuff(&mut *buf, K_BFF_CLEAR_WIN_INFO);
        if (*buf).b_vars().dv_refcount > DO_NOT_FREE_CNT {
            tv_dict_add(
                &mut *(*buf).b_vars(),
                tv_dict_item_copy(&mut *(&raw mut (*buf).changedtick_di).cast()),
            );
        }
        unref_var_dict(&mut *(*buf).b_vars());
        aubuflocal_remove(&mut *buf);
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
            (*buf).b_namedm.fill(Fmark::new());
            (*buf).b_changelist.fill(Fmark::new());
            g_st().au_pending_free_buf.push(Box::from_raw(buf));
        } else {
            drop(Box::from_raw(buf));
            if g_st().curbuf == buf {
                g_st().curbuf = ptr::null_mut();
            }
        }
    }
}

fn clear_wininfo(buf: &mut FileBuf) {
    buf.b_wininfo.clear();
}

fn free_buffer_stuff(buf: &mut FileBuf, free_flags: i32) {
    if (free_flags & K_BFF_CLEAR_WIN_INFO) != 0 {
        clear_wininfo(buf);
        free_buf_options(buf, true);
        buf.b_s.b_langp = Vec::new();
    }
    {
        let changedtick_hi = hash_find(&buf.b_vars().dv_hashtab, c"changedtick");
        debug_assert!(!changedtick_hi.is_null());
        hash_remove(&mut buf.b_vars().dv_hashtab, unsafe {
            &mut *changedtick_hi
        });
    }
    vars_clear(&mut buf.b_vars().dv_hashtab);
    if (free_flags & K_BFF_INIT_CHANGEDTICK) != 0 {
        buf_init_changedtick(buf);
    }
    buf.b_ucmds.clear();
    extmark_free_all(buf);
    mc_buf_free(buf);
    map_clear_mode(buf, MAP_ALL_MODES, true, false);
    map_clear_mode(buf, MAP_ALL_MODES, true, true);
    buf.b_start_fenc = None;
    buf.b_localdir = None;
    buf.b_prevdir = None;

    buf_free_callbacks(buf);
}

/// Go to another buffer. Handles the result of the ATTENTION dialog.
pub fn goto_buffer(eap: &mut Exarg, start: i32, dir: i32, count: i32) {
    let save_sea = g_st().swap_exists_action;
    let skip_help_buf = matches!(
        Cmdidx::from_raw(eap.cmdidx),
        Cmdidx::CmdBnextVariant
            | Cmdidx::CmdSbnextVariant
            | Cmdidx::CmdBnext
            | Cmdidx::CmdBprevious
            | Cmdidx::CmdSbnext
            | Cmdidx::CmdSbprevious
    );

    let mut old_curbuf: Bufref = Bufref::new();
    set_bufref(&mut old_curbuf, g_st().curbuf);

    if g_st().swap_exists_action == SEA_NONE {
        g_st().swap_exists_action = SEA_DIALOG;
    }
    let _ = do_buffer_ext(
        if crate::memory::cstr_first_byte(eap.cmd) == b's' {
            DOBUF_SPLIT
        } else {
            DOBUF_GOTO
        },
        start,
        dir,
        count,
        (if eap.forceit != 0 { DOBUF_FORCEIT } else { 0 })
            | (if skip_help_buf { DOBUF_SKIPHELP } else { 0 }),
    );

    if g_st().swap_exists_action == SEA_QUIT && crate::memory::cstr_first_byte(eap.cmd) == b's' {
        let mut cs: ExCleanup = ExCleanup::new();
        enter_cleanup(&mut cs);
        win_close(curwin_ptr(), true, false);
        g_st().swap_exists_action = save_sea;
        g_st().swap_exists_did_quit = true;
        leave_cleanup(&mut cs);
    } else {
        handle_swap_exists(Some(&mut old_curbuf));
    }
}

/// Handle the situation of `swap_exists_action` being set.
pub fn handle_swap_exists(mut old_curbuf: Option<&mut Bufref>) {
    let mut cs: ExCleanup = ExCleanup::new();
    let old_tw = curbuf_mut().b_p_tw;

    if g_st().swap_exists_action == SEA_QUIT {
        enter_cleanup(&mut cs);

        g_st().swap_exists_action = SEA_NONE;
        g_st().swap_exists_did_quit = true;
        close_buffer(
            curwin_ptr(),
            g_st().curbuf,
            DOBUF_UNLOAD,
            false,
            false,
            true,
        );
        let buf = if old_curbuf.is_none()
            || !bufref_valid((old_curbuf.is_some()).then(|| old_curbuf.as_deref().unwrap()))
            || old_curbuf.as_deref_mut().unwrap().br_buf == g_st().curbuf
        {
            block_autocmds();
            let b = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_CURBUF | BLN_LISTED);
            unblock_autocmds();
            b
        } else {
            old_curbuf.unwrap().br_buf
        };
        if !buf.is_null() {
            enter_buffer(buf);
            if old_tw != curbuf_mut().b_p_tw {
                check_colorcolumn(None::<&CStr>, Some(crate::globals::curwin_mut()));
            }
        }

        leave_cleanup(&mut cs);
    } else if g_st().swap_exists_action == SEA_RECOVER {
        enter_cleanup(&mut cs);

        g_st().msg_scroll = 1;
        ml_recover(false);
        msg_puts(c"\n");
        g_st().cmdline_row = g_st().msg_row;
        do_modelines(0);

        leave_cleanup(&mut cs);
    }
    g_st().swap_exists_action = SEA_NONE;
}

/// Delete or unload buffer(s).
pub fn do_bufdel(
    command: i32,
    arg: impl crate::strings::IntoCCharPtr,
    addr_count: i32,
    start_bnr: i32,
    end_bnr: i32,
    forceit: i32,
) -> RawCCharPtrMut {
    let mut arg = arg.into_c_char_ptr().cast_mut();
    let mut do_current = 0;
    let mut deleted: i32 = 0;
    let mut errormsg: RawCCharPtrMut = ptr::null_mut();
    let mut bnr: i32;

    if addr_count == 0 {
        do_buffer(command, DOBUF_CURRENT, FORWARD, 0, forceit);
    } else {
        if addr_count == 2 {
            if !crate::memory::cstr_is_empty(arg) {
                return ex_errmsg(E_TRAILING_ARG, arg);
            }
            bnr = start_bnr;
        } else {
            bnr = end_bnr;
        }

        while !g_st().got_int {
            if bnr == curbuf_mut().b_fnum {
                do_current = bnr;
            } else if do_buffer(command, DOBUF_FIRST, FORWARD, bnr, forceit) {
                deleted += 1;
            }

            if addr_count == 2 {
                bnr += 1;
                if bnr > end_bnr {
                    break;
                }
            } else {
                arg = skipwhite(arg);
                if crate::memory::cstr_is_empty(arg) {
                    break;
                }
                if !ascii_isdigit(crate::memory::cstr_first_char(arg)) {
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
            && do_buffer(command, DOBUF_FIRST, FORWARD, do_current, forceit)
        {
            deleted += 1;
        }

        if deleted == 0 {
            let iobuff_ptr = g_st().iobuff.as_mut_ptr();
            if command == DOBUF_UNLOAD {
                xstrlcpy(
                    iobuff_ptr,
                    gettext(c"E515: No buffers were unloaded"),
                    IOSIZE,
                );
            } else if command == DOBUF_DEL {
                xstrlcpy(
                    iobuff_ptr,
                    gettext(c"E516: No buffers were deleted"),
                    IOSIZE,
                );
            } else {
                xstrlcpy(
                    iobuff_ptr,
                    gettext(c"E517: No buffers were wiped out"),
                    IOSIZE,
                );
            }
            errormsg = iobuff_ptr;
        } else if i64::from(deleted) >= opt_vars().p_report {
            if command == DOBUF_UNLOAD {
                smsg!(
                    0,
                    ngettext(
                        c"%d buffer unloaded",
                        c"%d buffers unloaded",
                        deleted as usize,
                    ),
                    deleted,
                );
            } else if command == DOBUF_DEL {
                smsg!(
                    0,
                    ngettext(
                        c"%d buffer deleted",
                        c"%d buffers deleted",
                        deleted as usize,
                    ),
                    deleted,
                );
            } else {
                smsg!(
                    0,
                    ngettext(
                        c"%d buffer wiped out",
                        c"%d buffers wiped out",
                        deleted as usize,
                    ),
                    deleted,
                );
            }
        }
    }

    errormsg
}

fn empty_curbuf(close_others: bool, forceit: i32, action: i32) -> bool {
    let buf = g_st().curbuf;

    if action == DOBUF_UNLOAD {
        emsg(gettext(c"E90: Cannot unload last buffer"));
        return false;
    }

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, buf);

    if close_others {
        let mut can_close_all_others = true;
        if curwin_mut().w_floating {
            can_close_all_others = false;
            let mut wp = g_st().firstwin;
            while !unsafe { (*wp).w_floating } {
                if unsafe { (*wp).w_buffer } != g_st().curbuf {
                    can_close_all_others = true;
                    break;
                }
                wp = unsafe { (*wp).w_next };
            }
        }
        close_windows(buf, can_close_all_others);
    }

    setpcmark();
    let retval = do_ecmd(
        0,
        ptr::null_mut(),
        ptr::null_mut(),
        None,
        ECMD_ONE,
        if forceit != 0 { ECMD_FORCEIT } else { 0 },
        curwin_ptr(),
    );

    if buf != g_st().curbuf && bufref_valid(Some(&bufref)) && unsafe { (*buf).b_nwindows } == 0 {
        close_buffer(ptr::null_mut(), buf, action, false, false, false);
    }

    if !close_others {
        g_st().need_fileinfo = false;
    } else if retval && !shortmess(K_SHM_FILEINFO) {
        g_st().need_fileinfo = true;
    }

    retval
}

fn do_buffer_ext(action: i32, start: i32, dir: i32, mut count: i32, flags: i32) -> bool {
    unsafe {
        let mut buf: *mut FileBuf;
        let mut bp: *mut FileBuf;
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
                    if buf == g_st().curbuf || buf_is_changed(&mut *buf) {
                        break;
                    }
                }
            }
            if !buf_is_changed(&mut *buf) {
                emsg(gettext(c"E84: No modified buffer found"));
                return false;
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
                    emsg(gettext(c"E85: There is no listed buffer"));
                    return false;
                }
            }
        }

        if buf.is_null() {
            if start == DOBUF_FIRST {
                if !unload {
                    semsg!(gettext(E_NOBUFNR), i64::from(count));
                }
            } else if dir == FORWARD {
                emsg(gettext(c"E87: Cannot go beyond last buffer"));
            } else {
                emsg(gettext(c"E88: Cannot go before first buffer"));
            }
            return false;
        }

        if action == DOBUF_GOTO
            && buf != g_st().curbuf
            && !check_can_set_curbuf_forceit(flags & DOBUF_FORCEIT)
        {
            return false;
        }

        if (action == DOBUF_GOTO || action == DOBUF_SPLIT) && ((*buf).b_flags & BF_DUMMY) != 0 {
            semsg!(gettext(E_NOBUFNR), count);
            return false;
        }

        if unload {
            let mut bufref: Bufref = Bufref::new();
            if !can_unload_buffer(&mut *buf) {
                return false;
            }
            set_bufref(&mut bufref, buf);

            if action != DOBUF_WIPE && (*buf).b_ml.ml_mfp.is_none() && (*buf).b_p_bl == 0 {
                return false;
            }

            if (flags & DOBUF_FORCEIT) == 0 && buf_is_changed(&mut *buf) {
                if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                    && opt_vars().p_write != 0
                {
                    dialog_changed(&mut *buf, false);
                    if !bufref_valid(Some(&bufref)) {
                        return false;
                    }
                    if buf_is_changed(&mut *buf) {
                        return false;
                    }
                } else {
                    semsg!(
                        gettext(E_NO_WRITE_SINCE_LAST_CHANGE_FOR_BUFFER_NR_ADD_BANG_TO_OVERRIDE,),
                        (*buf).b_fnum,
                    );
                    return false;
                }
            }

            if (flags & DOBUF_FORCEIT) == 0
                && !(*buf).terminal.is_null()
                && terminal_running(&*(*buf).terminal)
            {
                if opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                    if !dialog_close_terminal(&mut *buf) {
                        return false;
                    }
                } else {
                    semsg!(
                        gettext(c"E89: %s will be killed (add ! to override)"),
                        (*buf).b_fname,
                    );
                    return false;
                }
            }

            let buf_fnum = (*buf).b_fnum;

            if buf == g_st().curbuf && g_st().visual_global.active {
                end_visual_mode();
            }

            bp = ptr::null_mut();
            for bp2 in for_all_buffers_ptr() {
                if bp2.b_p_bl != 0 && !std::ptr::eq(bp2, buf) {
                    bp = bp2;
                    break;
                }
            }
            if bp.is_null() && buf == g_st().curbuf {
                return empty_curbuf(true, flags & DOBUF_FORCEIT, action);
            }

            while buf == g_st().curbuf
                && !(win_locked(curwin_mut()) != 0 || curwin_mut().buf_mut().b_locked > 0)
                && (is_ctx_win(g_st().lastwin) || !last_window(curwin_mut()))
            {
                if !win_close(curwin_ptr(), false, false) {
                    break;
                }
            }

            if buf != g_st().curbuf {
                if (opt_vars().jop_flags & K_OPT_JOP_FLAG_CLEAN) != 0 {
                    mark_jumplist_forget_file(curwin_mut(), buf_fnum);
                }

                close_windows(buf, false);
                if buf != g_st().curbuf
                    && bufref_valid(Some(&bufref))
                    && firstwin_mut().w_buffer == buf
                    && one_window(firstwin_mut(), ptr::null_mut())
                {
                    debug_assert!(curwin_mut().w_floating);
                    let mut switchwin: CtxSwitch = CtxSwitch::new();
                    let rv = ctx_switch(
                        &mut switchwin,
                        g_st().firstwin,
                        g_st().curtab,
                        ptr::null_mut(),
                        K_CTX_NO_DISPLAY,
                    );
                    debug_assert!(rv);
                    do_buffer_ext(action, start, dir, count, flags);
                    ctx_restore(&mut switchwin);
                }

                if buf != g_st().curbuf && bufref_valid(Some(&bufref)) && (*buf).b_nwindows <= 0 {
                    close_buffer(ptr::null_mut(), buf, action, false, false, false);
                }
                return true;
            }

            buf = ptr::null_mut();
            bp = ptr::null_mut();
            if !g_st().au_new_curbuf.br_buf.is_null()
                && bufref_valid(Some(&g_st().au_new_curbuf))
                && (*g_st().au_new_curbuf.br_buf).b_locked_split == 0
            {
                buf = g_st().au_new_curbuf.br_buf;
            } else if curwin_mut().w_jumplistlen > 0 {
                if (opt_vars().jop_flags & K_OPT_JOP_FLAG_CLEAN) != 0 {
                    mark_jumplist_forget_file(curwin_mut(), buf_fnum);
                }

                if curwin_mut().w_jumplistlen > 0 {
                    let mut jumpidx = curwin_mut().w_jumplistidx;

                    if (opt_vars().jop_flags & K_OPT_JOP_FLAG_CLEAN) != 0 {
                        if jumpidx == curwin_mut().w_jumplistlen {
                            jumpidx = curwin_mut().w_jumplistlen - 1;
                            curwin_mut().w_jumplistidx = jumpidx;
                        }
                    } else {
                        jumpidx -= 1;
                        if jumpidx < 0 {
                            jumpidx = curwin_mut().w_jumplistlen - 1;
                        }
                    }

                    let forward = jumpidx;
                    while (opt_vars().jop_flags & K_OPT_JOP_FLAG_CLEAN) != 0
                        || jumpidx != curwin_mut().w_jumplistidx
                    {
                        buf = buflist_findnr(curwin_mut().w_jumplist[jumpidx as usize].fmark.fnum);

                        if !buf.is_null() {
                            if buf == g_st().curbuf
                                || (*buf).b_p_bl == 0
                                || bt_quickfix(buf.as_ref())
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
                            if (opt_vars().jop_flags & K_OPT_JOP_FLAG_CLEAN) != 0 {
                                curwin_mut().w_jumplistidx = jumpidx;
                                update_jumplist = false;
                            }
                            break;
                        }
                        if jumpidx == 0 && curwin_mut().w_jumplistidx == curwin_mut().w_jumplistlen
                        {
                            break;
                        }
                        jumpidx -= 1;
                        if jumpidx < 0 {
                            jumpidx = curwin_mut().w_jumplistlen - 1;
                        }
                        if jumpidx == forward {
                            break;
                        }
                    }
                }
            }

            if buf.is_null() {
                let mut forward = true;
                buf = curbuf_mut().b_next;
                loop {
                    if buf.is_null() {
                        if !forward {
                            break;
                        }
                        buf = curbuf_mut().b_prev;
                        forward = false;
                        continue;
                    }
                    if (*buf).b_help == curbuf_mut().b_help
                        && (*buf).b_p_bl != 0
                        && !bt_quickfix(buf.as_ref())
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
                for buf2 in for_all_buffers_ptr() {
                    if buf2.b_p_bl != 0
                        && !std::ptr::eq(buf2, g_st().curbuf)
                        && !bt_quickfix(Some(&*buf2))
                        && buf2.b_locked_split == 0
                    {
                        buf = buf2;
                        break;
                    }
                }
            }
            if buf.is_null() {
                buf = if !curbuf_mut().b_next.is_null() {
                    curbuf_mut().b_next
                } else {
                    curbuf_mut().b_prev
                };
                if bt_quickfix(buf.as_ref())
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
            return true;
        }
        if buf != g_st().curbuf && (*buf).b_locked_split != 0 {
            emsg(gettext(E_CANNOT_SWITCH_TO_A_CLOSING_BUFFER));
            return false;
        }
        if action == DOBUF_SPLIT && !win_split(0, 0) {
            return false;
        }

        if buf == g_st().curbuf {
            return true;
        }

        if action == DOBUF_GOTO && !can_abandon(curbuf_mut(), (flags & DOBUF_FORCEIT) != 0) {
            if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                && opt_vars().p_write != 0
            {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, buf);
                dialog_changed(curbuf_mut(), false);
                if !bufref_valid(Some(&bufref)) {
                    return false;
                }
            }
            if buf_is_changed(curbuf_mut()) {
                no_write_message();
                return false;
            }
        }

        set_curbuf(buf, action, update_jumplist);

        if action == DOBUF_SPLIT {
            reset_binding(curwin_mut());
        }

        if aborting() {
            return false;
        }

        true
    }
}

pub fn do_buffer(action: i32, start: i32, dir: i32, count: i32, forceit: i32) -> bool {
    do_buffer_ext(
        action,
        start,
        dir,
        count,
        if forceit != 0 { DOBUF_FORCEIT } else { 0 },
    )
}

/// Set current buffer to `buf`. Executes autocommands and closes current buffer.
pub fn set_curbuf(buf: *mut FileBuf, action: i32, update_jumplist: bool) {
    unsafe {
        let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;
        let old_tw = curbuf_mut().b_p_tw;

        if update_jumplist {
            setpcmark();
        }

        if (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
            curwin_mut().w_alt_fnum = curbuf_mut().b_fnum;
        }
        buflist_altfpos(curwin_mut());

        g_st().visual_global.reselect = 0;

        let prevbuf = g_st().curbuf;
        let mut newbufref: Bufref = Bufref::new();
        let mut prevbufref: Bufref = Bufref::new();
        set_bufref(&mut prevbufref, prevbuf);
        set_bufref(&mut newbufref, buf);

        if !apply_autocmds(
            EVENT_BUFLEAVE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        ) || (bufref_valid(Some(&prevbufref)) && bufref_valid(Some(&newbufref)) && !aborting())
        {
            if prevbuf == curwin_mut().w_buffer {
                reset_synblock(curwin_mut());
            }
            if unload {
                close_windows(prevbuf, false);
            }
            if bufref_valid(Some(&prevbufref)) && !aborting() {
                if prevbuf == g_st().curbuf
                    && ((g_st().state_global & MODE_INSERT) == 0 || curbuf_mut().b_nwindows <= 1)
                {
                    u_sync(false);
                }
                close_buffer(
                    curwin_ptr(),
                    prevbuf,
                    if unload {
                        action
                    } else if action == DOBUF_GOTO
                        && !buf_hide(&*prevbuf)
                        && !buf_is_changed(&mut *prevbuf)
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
        if (valid && buf != g_st().curbuf && !aborting()) || curwin_mut().w_buffer.is_null() {
            enter_buffer(if valid { buf } else { g_st().lastbuf });
            if old_tw != curbuf_mut().b_p_tw {
                check_colorcolumn(None::<&CStr>, Some(crate::globals::curwin_mut()));
            }
        }

        if bufref_valid(Some(&prevbufref)) && !(*prevbuf).terminal.is_null() {
            terminal_check_size(&mut *(*prevbuf).terminal);
        }

        update_cwd(K_CD_CAUSE_BUFFER);
    }
}

/// Enter a new current buffer.
fn enter_buffer(buf: *mut FileBuf) {
    if g_st().visual_global.active && !mem_st().entered_free_all_mem {
        end_visual_mode();
    }

    if !curwin_mut().w_buffer.is_null() {
        curwin_mut().buf_mut().b_nwindows -= 1;
    }

    curwin_mut().w_buffer = buf;
    g_st().curbuf = buf;
    curbuf_mut().b_nwindows += 1;

    buf_copy_options(unsafe { &mut *buf }, BCO_ENTER | BCO_NOHELP);
    if !unsafe { (*buf).b_help } {
        get_winopts(unsafe { &mut *buf });
    } else {
        clear_folding(curwin_mut());
    }
    fold_update_all(curwin_mut());

    if curwin_mut().w_onebuf_opt.wo_diff != 0 {
        diff_buf_add(g_st().curbuf);
    }

    curwin_mut().w_s_owned = None;

    curwin_mut().w_cursor.lnum = 1;
    curwin_mut().w_cursor.col = 0;
    curwin_mut().w_cursor.coladd = 0;
    curwin_mut().w_set_curswant = true;
    curwin_mut().w_topline_was_set = false;

    curwin_mut().w_valid = 0;

    if curbuf_mut().b_ml.ml_mfp.is_none() {
        if crate::memory::cstr_is_empty(curbuf_mut().b_p_ft) {
            curbuf_mut().b_did_filetype = false;
        }
        open_buffer(false, None, 0);
    } else {
        if g_st().msg_silent == 0 && !shortmess(K_SHM_FILEINFO) {
            g_st().need_fileinfo = true;
        }
        buf_check_timestamp(curbuf_mut());

        curwin_mut().w_topline = 1;
        curwin_mut().w_topfill = 0;
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

    if curwin_mut().w_cursor.lnum == 1 && inindent(0) {
        buflist_getfpos();
    }

    check_arg_idx(curwin_mut());
    maketitle();
    win_float_update_preview(curwin_mut());
    if curwin_mut().w_topline == 1 && !curwin_mut().w_topline_was_set {
        scroll_cursor_halfway(curwin_mut(), false, false);
    }

    do_autochdir();

    if (curbuf_mut().b_kmap_state & KEYMAP_INIT) != 0 {
        keymap_init();
    }
    if !curbuf_mut().b_help
        && curwin_mut().w_onebuf_opt.wo_spell != 0
        && unsafe { *curwin_mut().w_s_ptr().b_p_spl } != NUL
    {
        parse_spelllang(curwin_mut());
    }
    curbuf_mut().b_last_used = os_time() as i64;

    if !curbuf_mut().terminal.is_null() {
        terminal_check_size(curbuf_mut().terminal_mut());
    }

    redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
}

/// Change to the directory of the current buffer.
pub fn do_autochdir() {
    if opt_vars().p_acd != 0
        && starting_val() == 0
        && !curbuf_mut().b_ffname.is_null()
        && vim_chdirfile(curbuf_mut().b_ffname, K_CD_CAUSE_AUTO)
    {
        g_st().last_chdir_reason = Some(c"autochdir");
        shorten_fnames(1);
    }
}

pub fn no_write_message_buf(buf: &mut FileBuf) {
    if !buf.terminal.is_null() && channel_job_running(buf.b_p_channel as u64) {
        emsg(gettext(E_JOB_STILL_RUNNING_ADD_BANG_TO_END_THE_JOB));
    } else {
        semsg!(
            gettext(E_NO_WRITE_SINCE_LAST_CHANGE_FOR_BUFFER_NR_ADD_BANG_TO_OVERRIDE),
            buf.b_fnum,
        );
    }
}

pub fn no_write_message() {
    if !curbuf_mut().terminal.is_null() && channel_job_running(curbuf_mut().b_p_channel as u64) {
        emsg(gettext(E_JOB_STILL_RUNNING_ADD_BANG_TO_END_THE_JOB));
    } else {
        emsg(gettext(E_NO_WRITE_SINCE_LAST_CHANGE_ADD_BANG_TO_OVERRIDE));
    }
}

pub fn no_write_message_nobang(buf: &FileBuf) {
    if !buf.terminal.is_null() && channel_job_running(buf.b_p_channel as u64) {
        emsg(gettext(E_JOB_STILL_RUNNING));
    } else {
        emsg(gettext(E_NO_WRITE_SINCE_LAST_CHANGE));
    }
}

#[inline]
fn buf_init_changedtick(buf: &mut FileBuf) {
    buf.changedtick_di.di_tv = TypVal {
        v_type: VarType::VarNumber,
        v_lock: VAR_FIXED,
        vval: TypvalVval::Number(buf_get_changedtick(buf)),
    };
    buf.changedtick_di.di_flags = DI_FLAGS_RO | DI_FLAGS_FIX;
    tv_dict_add(&mut *buf.b_vars(), (&raw mut buf.changedtick_di).cast());
}

/// Add a file name to the buffer list.
pub fn buflist_new(
    ffname_arg: impl crate::strings::IntoCCharPtr,
    sfname_arg: impl crate::strings::IntoCCharPtr,
    lnum: Linenr,
    flags: i32,
) -> *mut FileBuf {
    let ffname_arg = ffname_arg.into_c_char_ptr().cast_mut();
    let sfname_arg = sfname_arg.into_c_char_ptr().cast_mut();
    unsafe {
        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let mut buf: *mut FileBuf;

        let ffname_cstr = fname_expand(g_st().curbuf, &raw mut ffname, &raw mut sfname);

        let mut file_id: FileID = FileID::new();
        let file_id_valid = !sfname.is_null() && os_fileid(sfname, &mut *(&raw mut file_id).cast());
        if !ffname.is_null() && (flags & (BLN_DUMMY | BLN_NEW)) == 0 {
            buf = buflist_findname_file_id(ffname, &mut file_id, file_id_valid);
            if !buf.is_null() {
                let buf = &mut *buf;
                drop(ffname_cstr);
                if lnum != 0 {
                    buflist_setfpos(
                        &mut *buf,
                        if (flags & BLN_NOCURWIN) != 0 {
                            ptr::null_mut()
                        } else {
                            curwin_ptr()
                        },
                        lnum,
                        0,
                        false,
                    );
                }
                if (flags & BLN_NOOPT) == 0 {
                    buf_copy_options(&mut *buf, 0);
                }
                if (flags & BLN_LISTED) != 0 && buf.b_p_bl == 0 {
                    buf.b_p_bl = 1;
                    let mut bufref: Bufref = Bufref::new();
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
            let mut bufref: Bufref = Bufref::new();

            debug_assert!(!g_st().curbuf.is_null());
            buf = g_st().curbuf;
            set_bufref(&mut bufref, buf);
            trigger_undo_ftplugin(&mut *buf, curwin_mut());
            buf_freeall(&mut *buf, BFA_WIPE | BFA_DEL);
            if aborting() {
                drop(ffname_cstr);
                return ptr::null_mut();
            }
            if !bufref_valid(Some(&bufref)) {
                buf = ptr::null_mut();
            }
        }
        if buf != g_st().curbuf || g_st().curbuf.is_null() {
            buf = FileBuf::alloc();
            let buf = &mut *buf;
            init_var_dict(
                &mut *tv_dict_alloc(),
                &mut *(&raw mut buf.b_bufvar).cast(),
                VAR_SCOPE,
            );
            buf.changedtick_di.di_key = b"changedtick\0".to_vec();
            buf_init_changedtick(&mut *buf);
        }

        if let Some(ff_cstr) = ffname_cstr {
            debug_assert!(!sfname.is_null());
            let mut sf_vec = cstr_to_cstring(sfname).into_bytes_with_nul();
            to_slash(sf_vec.as_mut_ptr());
            (*buf).set_ffname(Some(ff_cstr));
            (*buf).set_sfname(Some(std::ffi::CString::from_vec_with_nul(sf_vec).unwrap()));
        }

        clear_wininfo(&mut *buf);
        (*buf).b_wininfo.push(WinInfo::new());

        if buf == g_st().curbuf {
            free_buffer_stuff(&mut *buf, K_BFF_INIT_CHANGEDTICK);
            (*buf).b_p_initialized = false;
            buf_copy_options(&mut *buf, BCO_ENTER);
            curbuf_mut().b_kmap_state |= KEYMAP_INIT;
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
            g_st().buffer_handles.insert((*buf).b_fnum, buf);
            if buf_st().top_file_num < 0 {
                emsg(gettext(c"W14: Warning: List of file names overflow"));
                if g_st().emsg_silent == 0 && !g_st().in_assert_fails {
                    msg_delay(3001, true);
                }
                buf_st().top_file_num = 1;
            }

            buf_copy_options(&mut *buf, BCO_ALWAYS);
        }

        let curwin_info = &mut (&mut (*buf).b_wininfo)[0];
        curwin_info.wi_mark = INIT_FMARK;
        curwin_info.wi_mark.mark.lnum = lnum;
        curwin_info.wi_win = curwin_ptr();

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
        (*buf).b_p_bl = i32::from((flags & BLN_LISTED) != 0);
        (*buf).update_channels.clear();
        (*buf).update_callbacks.clear();
        if (flags & BLN_DUMMY) == 0 {
            let mut bufref: Bufref = Bufref::new();
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
    !g_st().curbuf.is_null()
        && curbuf_mut().b_ffname.is_null()
        && curbuf_mut().b_nwindows <= 1
        && curbuf_mut().terminal.is_null()
        && (curbuf_mut().b_ml.ml_mfp.is_none() || buf_is_empty(curbuf_mut()))
        && !bt_quickfix(Some(crate::globals::curbuf_mut()))
        && !curbuf_is_changed()
}

/// Free the memory for the options of a buffer.
pub fn free_buf_options(buf: &mut FileBuf, free_p_ff: bool) {
    if free_p_ff {
        clear_string_option(&raw mut buf.b_p_fenc);
        clear_string_option(&raw mut buf.b_p_ff);
        clear_string_option(&raw mut buf.b_p_bh);
        clear_string_option(&raw mut buf.b_p_bt);
    }
    clear_string_option(&raw mut buf.b_p_def);
    clear_string_option(&raw mut buf.b_p_inc);
    callback_free(&mut buf.b_p_inex);
    callback_free(&mut buf.b_p_inde);
    clear_string_option(&raw mut buf.b_p_indk);
    clear_string_option(&raw mut buf.b_p_fp);
    callback_free(&mut buf.b_p_fex);
    clear_string_option(&raw mut buf.b_p_kp);
    clear_string_option(&raw mut buf.b_p_mps);
    clear_string_option(&raw mut buf.b_p_fo);
    clear_string_option(&raw mut buf.b_p_flp);
    clear_string_option(&raw mut buf.b_p_isk);
    clear_string_option(&raw mut buf.b_p_vsts);
    buf.b_p_vsts_nopaste = None;
    buf.b_p_vsts_array = Vec::new();
    clear_string_option(&raw mut buf.b_p_vts);
    buf.b_p_vts_array = Vec::new();
    clear_string_option(&raw mut buf.b_p_keymap);
    buf.b_kmap_ga.clear();
    clear_string_option(&raw mut buf.b_p_com);
    clear_string_option(&raw mut buf.b_p_cms);
    clear_string_option(&raw mut buf.b_p_nf);
    clear_string_option(&raw mut buf.b_p_syn);
    buf.b_s.b_syn_isk = None;
    clear_string_option(&raw mut buf.b_s.b_p_spc);
    clear_string_option(&raw mut buf.b_s.b_p_spf);
    buf.b_s.b_cap_prog = None;
    clear_string_option(&raw mut buf.b_s.b_p_spl);
    clear_string_option(&raw mut buf.b_s.b_p_spo);
    clear_string_option(&raw mut buf.b_p_sua);
    clear_string_option(&raw mut buf.b_p_ft);
    clear_string_option(&raw mut buf.b_p_cink);
    clear_string_option(&raw mut buf.b_p_cino);
    clear_string_option(&raw mut buf.b_p_lop);
    clear_string_option(&raw mut buf.b_p_cinsd);
    clear_string_option(&raw mut buf.b_p_cinw);
    clear_string_option(&raw mut buf.b_p_cot);
    clear_string_option(&raw mut buf.b_p_cpt);
    #[cfg(windows)]
    clear_string_option(&raw mut buf.b_p_csl);
    callback_free(&mut buf.b_p_cfu);
    callback_free(&mut buf.b_p_ofu);
    callback_free(&mut buf.b_p_tsrfu);
    clear_cpt_callbacks(&mut buf.b_p_cpt_cb);
    clear_string_option(&raw mut buf.b_p_gefm);
    clear_string_option(&raw mut buf.b_p_gp);
    clear_string_option(&raw mut buf.b_p_mp);
    clear_string_option(&raw mut buf.b_p_efm);
    clear_string_option(&raw mut buf.b_p_ep);
    clear_string_option(&raw mut buf.b_p_path);
    clear_string_option(&raw mut buf.b_p_tags);
    clear_string_option(&raw mut buf.b_p_tc);
    callback_free(&mut buf.b_p_tfu);
    callback_free(&mut buf.b_p_ffu);
    clear_string_option(&raw mut buf.b_p_dict);
    clear_string_option(&raw mut buf.b_p_dia);
    clear_string_option(&raw mut buf.b_p_tsr);
    clear_string_option(&raw mut buf.b_p_qe);
    buf.b_p_ac = -1;
    buf.b_p_ar = -1;
    buf.b_p_fs = -1;
    buf.b_p_ul = NO_LOCAL_UNDOLEVEL;
    clear_string_option(&raw mut buf.b_p_lw);
    clear_string_option(&raw mut buf.b_p_bkc);
    clear_string_option(&raw mut buf.b_p_menc);
}

/// Get alternate file `n`.
pub fn buflist_getfile(n: i32, mut lnum: Linenr, options: i32, forceit: i32) -> bool {
    let mut fm: *mut Fmark = ptr::null_mut();

    let buf = buflist_findnr(n);
    if buf.is_null() {
        if (options & GETF_ALT) != 0 && n == 0 {
            emsg(gettext(E_NOALT));
        } else {
            semsg!(gettext(E_BUFFER_NR_NOT_FOUND), n);
        }
        return false;
    }

    if buf == g_st().curbuf {
        return true;
    }

    if text_or_buf_locked() {
        return false;
    }

    let col: Colnr;
    let mut restore_view = false;
    if lnum == 0 {
        fm = buflist_findfmark(unsafe { &mut *buf });
        lnum = unsafe { (*fm).mark.lnum };
        col = unsafe { (*fm).mark.col };
        restore_view = true;
    } else {
        col = 0;
    }

    if (options & GETF_SWITCH) != 0 {
        let wp = swbuf_goto_win_with_buf(buf);

        if wp.is_null()
            && (opt_vars().swb_flags
                & (K_OPT_SWB_FLAG_VSPLIT | K_OPT_SWB_FLAG_SPLIT | K_OPT_SWB_FLAG_NEWTAB))
                != 0
            && !buf_is_empty(curbuf_mut())
        {
            if (opt_vars().swb_flags & K_OPT_SWB_FLAG_NEWTAB) != 0 {
                tabpage_new();
            } else if !win_split(
                0,
                if (opt_vars().swb_flags & K_OPT_SWB_FLAG_VSPLIT) != 0 {
                    WSP_VERT
                } else {
                    0
                },
            ) {
                return false;
            }
            reset_binding(curwin_mut());
        }
    }

    g_st().redrawing_disabled += 1;
    if getfile(
        unsafe { (*buf).b_fnum },
        ptr::null_mut(),
        ptr::null_mut(),
        (options & GETF_SETMARK) != 0,
        lnum,
        forceit != 0,
    ) <= 0
    {
        g_st().redrawing_disabled -= 1;

        if opt_vars().p_sol == 0 && col != 0 {
            curwin_mut().w_cursor.col = col;
            check_cursor_col(curwin_mut());
            curwin_mut().w_cursor.coladd = 0;
            curwin_mut().w_set_curswant = true;
        }
        if (opt_vars().jop_flags & K_OPT_JOP_FLAG_VIEW) != 0 && restore_view {
            mark_view_restore(unsafe { fm.as_ref() });
        }
        return true;
    }
    g_st().redrawing_disabled -= 1;
    false
}

fn buflist_getfpos() {
    let fm = buflist_findfmark(curbuf_mut());
    let fpos = &fm.mark;
    curwin_mut().w_cursor.lnum = fpos.lnum;
    check_cursor_lnum(curwin_mut());

    if opt_vars().p_sol != 0 {
        curwin_mut().w_cursor.col = 0;
    } else {
        curwin_mut().w_cursor.col = fpos.col;
        check_cursor_col(curwin_mut());
        curwin_mut().w_cursor.coladd = 0;
        curwin_mut().w_set_curswant = true;
    }

    if (opt_vars().jop_flags & K_OPT_JOP_FLAG_VIEW) != 0 {
        mark_view_restore(Some(&*fm));
    }
}

/// Find file in buffer list by name (it has to be for the current window).
pub fn buflist_findname_exp(fname: impl crate::strings::IntoCCharPtr) -> *mut FileBuf {
    let fname = fname.into_c_char_ptr();
    let mut buf: *mut FileBuf = ptr::null_mut();
    if let Some(ffname) = FullName_save(fname, cfg!(unix)) {
        buf = buflist_findname(ffname.as_ptr().cast_mut());
    }
    buf
}

/// Find file in buffer list by name.
pub fn buflist_findname(ffname: impl crate::strings::IntoCCharPtr) -> *mut FileBuf {
    let ffname = ffname.into_c_char_ptr();
    {
        let mut file_id: FileID = FileID::new();
        let file_id_valid = os_fileid(ffname, unsafe { &mut *(&raw mut file_id).cast() });
        buflist_findname_file_id(ffname, &mut file_id, file_id_valid)
    }
}

fn buflist_findname_file_id(
    ffname: impl crate::strings::IntoCCharPtr,
    file_id: &mut FileID,
    file_id_valid: bool,
) -> *mut FileBuf {
    let ffname = ffname.into_c_char_ptr();
    {
        for buf in for_all_buffers_rev_ptr() {
            if (buf.b_flags & BF_DUMMY) == 0
                && !otherfile_buf(&mut *buf, ffname, file_id, file_id_valid)
            {
                return buf;
            }
        }
        ptr::null_mut()
    }
}

/// Find file in buffer list by a regexp pattern.
pub fn buflist_findpat(
    pattern: impl crate::strings::IntoCCharPtr,
    pattern_end: impl crate::strings::IntoCCharPtr,
    unlisted: bool,
    diffmode: bool,
    curtab_only: bool,
) -> i32 {
    let pattern = pattern.into_c_char_ptr();
    let pattern_end = pattern_end.into_c_char_ptr();
    {
        let mut match_fnum = -1;

        if pattern_end == pattern.wrapping_add(1)
            && (crate::memory::cstr_first_byte(pattern) == b'%'
                || crate::memory::cstr_first_byte(pattern) == b'#')
        {
            match_fnum = if crate::memory::cstr_first_byte(pattern) == b'%' {
                curbuf_mut().b_fnum
            } else {
                curwin_mut().w_alt_fnum
            };
            let found_buf = buflist_findnr(match_fnum);
            if diffmode && (found_buf.is_null() || !diff_mode_buf(found_buf)) {
                match_fnum = -1;
            }
        } else {
            let Some(pat_cstr) = file_pat_to_reg_pat(pattern, pattern_end, None, 0) else {
                return -1;
            };
            let mut pat_vec: Vec<u8> = pat_cstr.into_bytes_with_nul();
            let pat = pat_vec.as_mut_ptr();
            let patend = pat.wrapping_add(crate::memory::cstr_len(&pat_vec).saturating_sub(1));
            let toggledollar = patend > pat && unsafe { *patend } == b'$';

            let mut find_listed = 1;
            loop {
                for attempt in 0..=3 {
                    if toggledollar {
                        unsafe {
                            *patend = if attempt < 2 { 0 } else { b'$' };
                        }
                    }
                    let mut p = pat;
                    if unsafe { *p } == b'^' && (attempt & 1) == 0 {
                        p = p.wrapping_add(1);
                    }

                    let mut regmatch: Regmatch = Regmatch::new();
                    regmatch.regprog =
                        vim_regcomp(p.cast(), if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });

                    for buf in for_all_buffers_rev_ptr() {
                        if regmatch.regprog.is_null() {
                            return -1;
                        }
                        if buf.b_p_bl == find_listed
                            && (!diffmode || diff_mode_buf(buf))
                            && !buflist_match(&mut regmatch, &mut *buf, false).is_null()
                        {
                            if curtab_only {
                                let mut found_window = false;
                                for wp in for_all_windows_mut() {
                                    if wp.w_buffer == buf {
                                        found_window = true;
                                        break;
                                    }
                                }
                                if !found_window {
                                    continue;
                                }
                            }
                            if match_fnum >= 0 {
                                match_fnum = -2;
                                break;
                            }
                            match_fnum = buf.b_fnum;
                        }
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
            semsg!(gettext(c"E93: More than one match for %s"), pattern,);
        } else if match_fnum < 0 {
            semsg!(gettext(c"E94: No matching buffer for %s"), pattern);
        }
        match_fnum
    }
}

fn buf_time_compare(buf1: &FileBuf, buf2: &FileBuf) -> std::cmp::Ordering {
    buf2.b_last_used.cmp(&buf1.b_last_used)
}

/// Find all buffer names that match.
pub fn expand_bufnames(
    pat: impl crate::strings::IntoCCharPtr,
    file: &mut Vec<std::ffi::CString>,
    options: i32,
) -> bool {
    let pat = pat.into_c_char_ptr();
    {
        let mut matches: Vec<Bufmatch> = Vec::new();

        file.clear();

        if (options & BUF_DIFF_FILTER) != 0 && curwin_mut().w_onebuf_opt.wo_diff == 0 {
            return false;
        }

        let fuzzy = cmdline_fuzzy_complete(pat);
        let use_matches = !fuzzy && (options & WILD_BUFLASTUSED) != 0;

        let mut patc_owned: Option<std::ffi::CString> = None;
        let mut fuzmatch: Vec<FuzmatchStr> = Vec::new();
        let mut regmatch: Regmatch = Regmatch::new();

        if !fuzzy {
            let patc: RawCCharPtr = if crate::memory::cstr_first_byte(pat) == b'^'
                && crate::memory::cstr_byte_at(pat, 1) != 0
            {
                let pc = cstr_to_cstring(pat.wrapping_add(1));
                let ptr = pc.as_ptr();
                patc_owned = Some(pc);
                ptr
            } else if crate::memory::cstr_first_byte(pat) == b'^' {
                c"".as_ptr()
            } else {
                pat
            };
            regmatch.regprog = vim_regcomp(patc, RE_MAGIC);
            if regmatch.regprog.is_null() {
                return false;
            }
        }

        let mut score = 0;
        for buf in for_all_buffers_ptr() {
            if buf.b_p_bl == 0 {
                continue;
            }
            if (options & BUF_DIFF_FILTER) != 0
                && (std::ptr::eq(buf, g_st().curbuf) || !diff_mode_buf(buf))
            {
                continue;
            }

            let mut p: RawCCharPtrMut = ptr::null_mut();
            if !fuzzy {
                p = buflist_match(&mut regmatch, &mut *buf, opt_vars().p_wic != 0);
            } else {
                score = fuzzy_match_str(buf.b_sfname, pat);
                if score != FUZZY_SCORE_NONE {
                    p = buf.b_sfname;
                }
                if p.is_null() {
                    score = fuzzy_match_str(buf.b_ffname, pat);
                    if score != FUZZY_SCORE_NONE {
                        p = buf.b_ffname;
                    }
                }
            }

            if p.is_null() {
                continue;
            }

            let p_cstr = if (options & WILD_HOME_REPLACE) != 0 {
                home_replace_save(Some(&*buf), p)
            } else {
                cstr_to_cstring(p)
            };

            if !fuzzy {
                if use_matches {
                    matches.push(Bufmatch {
                        buf,
                        match_: p_cstr,
                    });
                } else {
                    file.push(p_cstr);
                }
            } else {
                let idx = fuzmatch.len() as i32;
                fuzmatch.push(FuzmatchStr {
                    idx,
                    str_cstr: p_cstr,
                    score,
                });
            }
        }

        if !fuzzy {
            vim_regfree(regmatch.regprog);
            drop(patc_owned);
        }

        if !fuzzy {
            if use_matches && !matches.is_empty() {
                if matches.len() > 1 {
                    matches.sort_unstable_by(|a, b| {
                        buf_time_compare(unsafe { &*a.buf }, unsafe { &*b.buf })
                    });
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

        !file.is_empty()
    }
}

fn buflist_match(rmp: &mut Regmatch, buf: &mut FileBuf, ignore_case: bool) -> RawCCharPtrMut {
    let mut match_ = fname_match(rmp, buf.b_sfname, ignore_case);
    if match_.is_null() && !rmp.regprog.is_null() {
        match_ = fname_match(rmp, buf.b_ffname, ignore_case);
    }
    match_
}

fn fname_match(
    rmp: &mut Regmatch,
    name: impl crate::strings::IntoCCharPtr,
    ignore_case: bool,
) -> RawCCharPtrMut {
    let name = name.into_c_char_ptr().cast_mut();
    let mut match_: RawCCharPtrMut = ptr::null_mut();

    if name.is_null() || rmp.regprog.is_null() {
        return ptr::null_mut();
    }

    rmp.rm_ic = opt_vars().p_fic != 0 || ignore_case;
    if vim_regexec(rmp, name, 0) {
        match_ = name;
    } else if !rmp.regprog.is_null() {
        let p = home_replace_save(None, name);
        if vim_regexec(rmp, p.as_ptr().cast_mut(), 0) {
            match_ = name;
        }
    }

    match_
}

/// Find a file in the buffer list by buffer number.
pub fn buflist_findnr(mut nr: i32) -> *mut FileBuf {
    if nr == 0 {
        nr = curwin_mut().w_alt_fnum;
    }
    g_st()
        .buffer_handles
        .get(&nr)
        .copied()
        .unwrap_or(ptr::null_mut())
}

pub fn buflist_nr2name(n: i32, fullname: i32, helptail: i32) -> Option<std::ffi::CString> {
    let buf = buflist_findnr(n);
    if buf.is_null() {
        return None;
    }
    let buf = unsafe { &mut *buf };
    Some(home_replace_save(
        (helptail != 0).then_some(&*buf),
        if fullname != 0 {
            buf.b_ffname
        } else {
            buf.b_fname
        },
    ))
}

/// Set the line and column numbers for the given buffer and window.
pub fn buflist_setfpos(
    buf: &mut FileBuf,
    win: *mut WinInfoInfo,
    mut lnum: Linenr,
    col: Colnr,
    copy_options: bool,
) {
    let mut wip = if let Some(i) = buf.b_wininfo.iter().position(|w| w.wi_win == win) {
        let mut wip = buf.b_wininfo.remove(i);
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
            wip.wi_mark.view = mark_view_make(unsafe { &*win }, wip.wi_mark.mark);
        }
    }
    if !win.is_null() {
        wip.wi_changelistidx = unsafe { (*win).w_changelistidx };
    }
    if copy_options && !win.is_null() {
        copy_winopt(unsafe { &mut (*win).w_onebuf_opt }, &mut wip.wi_opt);
        wip.wi_fold_manual = unsafe { (*win).w_fold_manual };
        wip.wi_folds.clone_from(unsafe { &(*win).w_folds });
        wip.wi_optset = true;
    }

    buf.b_wininfo.insert(0, wip);
}

fn wininfo_other_tab_diff(wip: &WinInfo) -> bool {
    if wip.wi_opt.wo_diff == 0 {
        return false;
    }

    for wp in for_all_windows_ptr() {
        if wip.wi_win == wp {
            return false;
        }
    }
    true
}

fn find_wininfo(buf: &mut FileBuf, need_options: bool, skip_diff_buffer: bool) -> *mut WinInfo {
    for wip in &mut buf.b_wininfo {
        if wip.wi_win == curwin_ptr()
            && (!skip_diff_buffer || !wininfo_other_tab_diff(wip))
            && (!need_options || wip.wi_optset)
        {
            return wip;
        }
    }

    if skip_diff_buffer {
        let buf_ptr: *mut FileBuf = &raw mut *buf;
        for wip in &mut buf.b_wininfo {
            if !wininfo_other_tab_diff(wip)
                && (!need_options
                    || wip.wi_optset
                    || (!wip.wi_win.is_null() && unsafe { (*wip.wi_win).w_buffer } == buf_ptr))
            {
                return wip;
            }
        }
    } else if let Some(wip) = buf.b_wininfo.first_mut() {
        return wip;
    }
    ptr::null_mut()
}

/// Reset the local window options to the values last used in this window.
pub fn get_winopts(buf: &mut FileBuf) {
    unsafe {
        clear_winopt(&mut curwin_mut().w_onebuf_opt);
        clear_folding(curwin_mut());

        let wip = find_wininfo(buf, true, true);
        if !wip.is_null()
            && (*wip).wi_win != curwin_ptr()
            && !(*wip).wi_win.is_null()
            && (*(*wip).wi_win).w_buffer == (&raw mut *buf)
            && (*(*wip).wi_win).w_config.style != K_WIN_STYLE_MINIMAL
        {
            let wp = (*wip).wi_win;
            let wp = &mut *wp;
            copy_winopt(&mut wp.w_onebuf_opt, &mut curwin_mut().w_onebuf_opt);
            curwin_mut().w_fold_manual = wp.w_fold_manual;
            curwin_mut().w_foldinvalid = true;
            curwin_mut().w_folds.clone_from(&wp.w_folds);
        } else if !wip.is_null()
            && (*wip).wi_optset
            && ((*wip).wi_win.is_null()
                || (*wip).wi_win == curwin_ptr()
                || (*(*wip).wi_win).w_config.style != K_WIN_STYLE_MINIMAL)
        {
            copy_winopt(&mut (*wip).wi_opt, &mut curwin_mut().w_onebuf_opt);
            curwin_mut().w_fold_manual = (*wip).wi_fold_manual;
            curwin_mut().w_foldinvalid = true;
            curwin_mut().w_folds.clone_from(&(*wip).wi_folds);
        } else {
            copy_winopt(
                &mut curwin_mut().w_allbuf_opt,
                &mut curwin_mut().w_onebuf_opt,
            );
        }
        if !wip.is_null() {
            curwin_mut().w_changelistidx = (*wip).wi_changelistidx;
        }

        if curwin_mut().w_config.style == K_WIN_STYLE_MINIMAL {
            didset_window_options(curwin_mut(), false);
            win_set_minimal_style(curwin_mut());
        }

        if opt_vars().p_fdls >= 0 {
            curwin_mut().w_onebuf_opt.wo_fdl = opt_vars().p_fdls;
        }
        didset_window_options(curwin_mut(), false);
    }
}

/// Find the mark for the buffer `buf` for the current window.
pub fn buflist_findfmark(buf: &mut FileBuf) -> &'static mut Fmark {
    let wip = find_wininfo(buf, false, false);
    if wip.is_null() {
        &mut buf_st().no_position
    } else {
        unsafe { &mut (*wip).wi_mark }
    }
}

/// Find the lnum for the buffer `buf` for the current window.
pub fn buflist_findlnum(buf: &mut FileBuf) -> Linenr {
    buflist_findfmark(buf).mark.lnum
}

/// List all known file names (for `:files` and `:buffers` command).
pub fn buflist_list(eap: &mut Exarg) {
    let mut buf = g_st().firstbuf;
    let mut buflist: Vec<*mut FileBuf> = Vec::new();
    let mut use_buflist = false;
    let mut buflist_idx = 0usize;

    msg_ext_set_kind(c"list_cmd");
    if crate::strings::vim_haschr(eap.arg, b't' as i32) {
        use_buflist = true;
        while !buf.is_null() {
            buflist.push(buf);
            buf = unsafe { (*buf).b_next };
        }

        if buflist.len() > 1 {
            buflist.sort_unstable_by(|&a, &b| buf_time_compare(unsafe { &*a }, unsafe { &*b }));
        }

        buf = buflist.first().copied().unwrap_or(ptr::null_mut());
    }

    let mut buf_ptr = buf;
    while !buf_ptr.is_null() && !g_st().got_int {
        let buf = unsafe { &mut *buf_ptr };
        let is_terminal = !buf.terminal.is_null();
        let job_running = is_terminal && terminal_running(unsafe { &*buf.terminal });

        let mut next_buf = |b: *mut FileBuf| {
            if use_buflist {
                buflist_idx += 1;
                buflist.get(buflist_idx).copied().unwrap_or(ptr::null_mut())
            } else {
                unsafe { (*b).b_next }
            }
        };

        if (buf.b_p_bl == 0
            && eap.forceit == 0
            && !crate::strings::vim_haschr(eap.arg, b'u' as i32))
            || (crate::strings::vim_haschr(eap.arg, b'u' as i32) && buf.b_p_bl != 0)
            || (crate::strings::vim_haschr(eap.arg, b'+' as i32)
                && ((buf.b_flags & BF_READERR) != 0 || !buf_is_changed(&mut *buf)))
            || (crate::strings::vim_haschr(eap.arg, b'a' as i32)
                && (buf.b_ml.ml_mfp.is_none() || buf.b_nwindows == 0))
            || (crate::strings::vim_haschr(eap.arg, b'h' as i32)
                && (buf.b_ml.ml_mfp.is_none() || buf.b_nwindows != 0))
            || (crate::strings::vim_haschr(eap.arg, b'R' as i32) && (!is_terminal || !job_running))
            || (crate::strings::vim_haschr(eap.arg, b'F' as i32) && (!is_terminal || job_running))
            || (crate::strings::vim_haschr(eap.arg, b'-' as i32) && buf.b_p_ma != 0)
            || (crate::strings::vim_haschr(eap.arg, b'=' as i32) && buf.b_p_ro == 0)
            || (crate::strings::vim_haschr(eap.arg, b'x' as i32) && (buf.b_flags & BF_READERR) == 0)
            || (crate::strings::vim_haschr(eap.arg, b'%' as i32)
                && !std::ptr::eq(buf, g_st().curbuf))
            || (crate::strings::vim_haschr(eap.arg, b'#' as i32)
                && (std::ptr::eq(buf, g_st().curbuf) || curwin_mut().w_alt_fnum != buf.b_fnum))
        {
            buf_ptr = next_buf(buf_ptr);
            continue;
        }

        let name = buf_spname(&*(buf));
        let namebuff_ptr = g_st().name_buff.as_mut_ptr();
        if !name.is_null() {
            xstrlcpy(namebuff_ptr, name, MAXPATHL);
        } else {
            home_replace(Some(&*buf), buf.b_fname, namebuff_ptr, MAXPATHL, true);
        }

        if message_filtered(namebuff_ptr) {
            buf_ptr = next_buf(buf_ptr);
            continue;
        }

        let changed_char = if (buf.b_flags & BF_READERR) != 0 {
            b'x' as i32
        } else if buf_is_changed(&mut *buf) {
            b'+' as i32
        } else {
            b' ' as i32
        };
        let mut ro_char = if buf.b_p_ma == 0 {
            b'-' as i32
        } else if buf.b_p_ro != 0 {
            b'=' as i32
        } else {
            b' ' as i32
        };
        if is_terminal {
            ro_char = if terminal_running(unsafe { &*buf.terminal }) {
                b'R' as i32
            } else {
                b'F' as i32
            };
        }

        if !ui_has(K_UIMESSAGES) || g_st().msg_col > 0 {
            msg_putchar(b'\n' as i32);
        }
        let iobuff_ptr = g_st().iobuff.as_mut_ptr();
        let mut len = vim_snprintf_safelen!(
            iobuff_ptr,
            IOSIZE - 20,
            c"%3d%c%c%c%c%c \"%s\"",
            buf.b_fnum,
            if buf.b_p_bl != 0 {
                b' ' as i32
            } else {
                b'u' as i32
            },
            if std::ptr::eq(buf, g_st().curbuf) {
                b'%' as i32
            } else if curwin_mut().w_alt_fnum == buf.b_fnum {
                b'#' as i32
            } else {
                b' ' as i32
            },
            if buf.b_ml.ml_mfp.is_none() {
                b' ' as i32
            } else if buf.b_nwindows == 0 {
                b'h' as i32
            } else {
                b'a' as i32
            },
            ro_char,
            changed_char,
            namebuff_ptr,
        )
        .min(IOSIZE - 20);

        let mut i = 40 - vim_strsize(iobuff_ptr);
        loop {
            unsafe {
                *iobuff_ptr.wrapping_add(len).cast::<u8>() = b' ';
            }
            len += 1;
            i -= 1;
            if i <= 0 || len >= IOSIZE - 18 {
                break;
            }
        }
        if crate::strings::vim_haschr(eap.arg, b't' as i32) && buf.b_last_used != 0 {
            let slice = unsafe {
                std::slice::from_raw_parts_mut(
                    iobuff_ptr.wrapping_add(len).cast::<u8>(),
                    IOSIZE - len,
                )
            };
            undo_fmt_time(slice, buf.b_last_used);
        } else {
            vim_snprintf!(
                iobuff_ptr.wrapping_add(len),
                IOSIZE - len,
                gettext(c"line %lld"),
                if std::ptr::eq(buf, g_st().curbuf) {
                    i64::from(curwin_mut().w_cursor.lnum)
                } else {
                    i64::from(buflist_findlnum(&mut *buf))
                },
            );
        }

        msg_outtrans(iobuff_ptr, 0, false);
        line_breakcheck();
        buf_ptr = next_buf(buf_ptr);
    }
}

/// Get file name and line number for file `fnum`.
pub fn buflist_name_nr<T>(fnum: i32, fname: *mut *mut T, lnum: &mut Linenr) -> bool {
    unsafe {
        let buf = buflist_findnr(fnum);
        if buf.is_null() || (*buf).b_fname.is_null() {
            return false;
        }

        *fname = (*buf).b_fname.cast();
        *lnum = buflist_findlnum(&mut *buf);

        true
    }
}

/// Set the file name for `buf` to `ffname_arg`, short file name to `sfname_arg`.
pub fn setfname(
    buf: &mut FileBuf,
    ffname_arg: impl crate::strings::IntoCCharPtr,
    sfname_arg: impl crate::strings::IntoCCharPtr,
    message: bool,
) -> bool {
    let ffname_arg = ffname_arg.into_c_char_ptr().cast_mut();
    let sfname_arg = sfname_arg.into_c_char_ptr().cast_mut();
    unsafe {
        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let mut obuf: *mut FileBuf = ptr::null_mut();
        let mut file_id: FileID = FileID::new();
        let mut file_id_valid = false;

        if crate::memory::cstr_is_empty(ffname) {
            buf.clear_fnames();
        } else {
            let Some(ffname_cstr) = fname_expand(&raw mut *buf, &raw mut ffname, &raw mut sfname)
            else {
                return false;
            };

            file_id_valid = os_fileid(ffname, &mut *(&raw mut file_id).cast());
            if (buf.b_flags & BF_DUMMY) == 0 {
                obuf = buflist_findname_file_id(ffname, &mut file_id, file_id_valid);
            }
            if !obuf.is_null() && obuf != (&raw mut *buf) {
                let mut in_use = false;

                for tab in for_all_tabpages_ptr() {
                    for win in for_all_windows_in_tab_mut(&*tab) {
                        if win.w_buffer == obuf {
                            in_use = true;
                        }
                    }
                }

                if (*obuf).b_ml.ml_mfp.is_some() || in_use {
                    if message {
                        emsg(gettext(c"E95: Buffer with this name already exists"));
                    }
                    return false;
                }
                close_buffer(ptr::null_mut(), obuf, DOBUF_WIPE, false, false, false);
            }
            let mut sf_vec = cstr_to_cstring(sfname).into_bytes_with_nul();
            if CASE_INSENSITIVE_FILENAME {
                to_slash(sf_vec.as_mut_ptr());
                path_fix_case(sf_vec.as_mut_ptr());
            }
            let sf_len = crate::memory::cstr_len(&sf_vec);
            let sfname_cstr = cstr_from_bytes(&sf_vec[..sf_len]);
            buf.set_ffname(Some(ffname_cstr));
            buf.set_sfname(Some(sfname_cstr));
        }
        buf.b_fname = buf.b_sfname;
        if !file_id_valid {
            buf.file_id_valid = false;
        } else {
            buf.file_id = file_id;
            buf.file_id_valid = true;
        }

        buf_name_changed(buf);
        true
    }
}

/// Crude way of changing the name of a buffer.
pub fn buf_set_name(fnum: i32, name: &std::ffi::CStr) {
    unsafe {
        let buf = buflist_findnr(fnum);
        if buf.is_null() {
            return;
        }

        (*buf).clear_fnames();
        let name_cstr = name.to_owned();
        let mut ffname = name_cstr.as_ptr().cast_mut();
        let mut sfname: RawCCharPtrMut = ptr::null_mut();
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
pub fn buf_name_changed(buf: &mut FileBuf) {
    if buf.b_ml.ml_mfp.is_some() {
        ml_setname(buf);
    }

    if curwin_mut().w_buffer == (&raw mut *buf) {
        check_arg_idx(curwin_mut());
    }
    maketitle();
    status_redraw_all();
    fmarks_check_names(buf);
    ml_timestamp(buf);
}

/// Set alternate file name for current window.
pub fn setaltfname(
    ffname: impl crate::strings::IntoCCharPtr,
    sfname: impl crate::strings::IntoCCharPtr,
    lnum: Linenr,
) -> *mut FileBuf {
    let buf = buflist_new(ffname, sfname, lnum, 0);
    if !buf.is_null() && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
        curwin_mut().w_alt_fnum = unsafe { (*buf).b_fnum };
    }
    buf
}

/// Get alternate file name for current window.
pub fn getaltfname(errmsg: bool) -> RawCCharPtrMut {
    let mut fname: RawCCharPtrMut = ptr::null_mut();
    let mut dummy: Linenr = 0;

    if !buflist_name_nr(0, &raw mut fname, &mut dummy) {
        if errmsg {
            emsg(gettext(E_NOALT));
        }
        return ptr::null_mut();
    }
    fname
}

/// Add a file name to the buflist and return its number.
pub fn buflist_add(fname: impl crate::strings::IntoCCharPtr, flags: i32) -> i32 {
    unsafe {
        let buf = buflist_new(fname, ptr::null_mut::<u8>().cast(), 0, flags);
        if !buf.is_null() { (*buf).b_fnum } else { 0 }
    }
}

/// Set alternate cursor position for the current buffer and window `win`.
pub fn buflist_altfpos(win: &mut WinInfoInfo) {
    buflist_setfpos(curbuf_mut(), win, win.w_cursor.lnum, win.w_cursor.col, true);
}

/// Check that `ffname` is not the same file as current file.
pub fn otherfile(ffname: impl crate::strings::IntoCCharPtr) -> bool {
    let ffname = ffname.into_c_char_ptr();
    otherfile_buf(curbuf_mut(), ffname, ptr::null_mut(), false)
}

fn otherfile_buf(
    buf: &mut FileBuf,
    ffname: impl crate::strings::IntoCCharPtr,
    mut file_id_p: *mut FileID,
    mut file_id_valid: bool,
) -> bool {
    let ffname = ffname.into_c_char_ptr();
    {
        if crate::memory::cstr_is_empty(ffname) || buf.b_ffname.is_null() {
            return true;
        }
        if path_equal(ffname, buf.b_ffname, K_PATH_CMP_LITERAL) {
            return false;
        }
        {
            let mut file_id: FileID = FileID::new();
            if file_id_p.is_null() {
                file_id_p = &raw mut file_id;
                file_id_valid = os_fileid(ffname, unsafe { &mut *file_id_p.cast() });
            }
            if !file_id_valid {
                return true;
            }
            if buf_same_file_id(buf, file_id_p) {
                buf_set_file_id(&mut *buf);
                if buf_same_file_id(buf, file_id_p) {
                    return false;
                }
            }
        }
        true
    }
}

/// Set `file_id` for a buffer.
pub fn buf_set_file_id(buf: &mut FileBuf) {
    let mut file_id: FileID = FileID::new();
    if !buf.b_fname.is_null() && os_fileid(buf.b_fname, unsafe { &mut *(&raw mut file_id).cast() })
    {
        buf.file_id_valid = true;
        buf.file_id = file_id;
    } else {
        buf.file_id_valid = false;
    }
}

#[inline]
fn buf_same_file_id(buf: &mut FileBuf, file_id: *mut FileID) -> bool {
    unsafe {
        buf.file_id_valid && os_fileid_equal(&*(&raw const buf.file_id).cast(), &*file_id.cast())
    }
}

/// Print info about the current buffer.
pub fn fileinfo(fullname: i32, shorthelp: i32, dont_truncate: bool) {
    let mut buffer_vec = vec![0u8; IOSIZE];
    let buffer: RawCCharPtrMut = buffer_vec.as_mut_ptr().cast();
    let mut bufferlen = 0usize;

    if fullname > 1 {
        bufferlen = vim_snprintf_safelen!(buffer, IOSIZE, c"buf %d: ", curbuf_mut().b_fnum);
    }

    unsafe {
        *buffer.wrapping_add(bufferlen).cast::<u8>() = b'"';
    }
    bufferlen += 1;

    let mut name = buf_spname(unsafe { &*(g_st().curbuf) });
    if !name.is_null() {
        bufferlen += vim_snprintf_safelen!(
            buffer.wrapping_add(bufferlen),
            IOSIZE - bufferlen,
            c"%s",
            name,
        );
    } else {
        name = if fullname == 0 && !curbuf_mut().b_fname.is_null() {
            curbuf_mut().b_fname
        } else {
            curbuf_mut().b_ffname
        };
        bufferlen += home_replace(
            unsafe {
                (if shorthelp != 0 {
                    g_st().curbuf
                } else {
                    ptr::null_mut()
                })
                .as_ref()
            },
            name,
            buffer.wrapping_add(bufferlen),
            IOSIZE - bufferlen,
            true,
        );
    }

    let dontwrite = bt_dontwrite(Some(crate::globals::curbuf_mut()));
    let is_changed = curbuf_is_changed();
    let mod_str = if is_changed {
        if shortmess(K_SHM_MOD) {
            c" [+]".as_ptr()
        } else {
            gettext(c" [Modified]")
        }
    } else {
        c" ".as_ptr()
    };
    let notedited_str = if (curbuf_mut().b_flags & BF_NOTEDITED) != 0 && !dontwrite {
        gettext(c"[Not edited]")
    } else {
        c"".as_ptr()
    };
    let new_str = if (curbuf_mut().b_flags & BF_NEW) != 0 && !dontwrite {
        gettext(c"[New]")
    } else {
        c"".as_ptr()
    };
    let readerr_str = if (curbuf_mut().b_flags & BF_READERR) != 0 {
        gettext(c"[Read errors]")
    } else {
        c"".as_ptr()
    };
    let ro_str = if curbuf_mut().b_p_ro != 0 {
        if shortmess(K_SHM_RO) {
            gettext(c"[RO]")
        } else {
            gettext(c"[readonly]")
        }
    } else {
        c"".as_ptr()
    };
    let sep_str =
        if is_changed || (curbuf_mut().b_flags & BF_WRITE_MASK) != 0 || curbuf_mut().b_p_ro != 0 {
            c" ".as_ptr()
        } else {
            c"".as_ptr()
        };

    bufferlen += vim_snprintf_safelen!(
        buffer.wrapping_add(bufferlen),
        IOSIZE - bufferlen,
        c"\"%s%s%s%s%s%s",
        mod_str,
        notedited_str,
        new_str,
        readerr_str,
        ro_str,
        sep_str,
    );

    let line_count = i64::from(curbuf_mut().b_ml.ml_line_count);
    let cur_lnum = i64::from(curwin_mut().w_cursor.lnum);
    if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
        bufferlen += vim_snprintf_safelen!(
            buffer.wrapping_add(bufferlen),
            IOSIZE - bufferlen,
            c"%s",
            gettext(NO_LINES_MSG),
        );
    } else if opt_vars().p_ru != 0 {
        bufferlen += vim_snprintf_safelen!(
            buffer.wrapping_add(bufferlen),
            IOSIZE - bufferlen,
            ngettext(
                c"%lld line --%d%%--",
                c"%lld lines --%d%%--",
                line_count as usize,
            ),
            line_count,
            calc_percentage(cur_lnum, line_count),
        );
    } else {
        bufferlen += vim_snprintf_safelen!(
            buffer.wrapping_add(bufferlen),
            IOSIZE - bufferlen,
            gettext(c"line %lld of %lld --%d%%-- col "),
            cur_lnum,
            line_count,
            calc_percentage(cur_lnum, line_count),
        );
        validate_virtcol(curwin_mut());
        bufferlen += col_print(
            buffer.wrapping_add(bufferlen).cast(),
            IOSIZE - bufferlen,
            curwin_mut().w_cursor.col + 1,
            curwin_mut().w_virtcol + 1,
        ) as usize;
    }

    append_arg_number(
        curwin_mut(),
        buffer.wrapping_add(bufferlen).cast(),
        IOSIZE - bufferlen,
    );

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

pub fn col_print(buf: *mut u8, buflen: usize, col: i32, vcol: i32) -> i32 {
    if col == vcol {
        vim_snprintf_safelen!(buf.cast(), buflen, c"%d", col) as i32
    } else {
        vim_snprintf_safelen!(buf.cast(), buflen, c"%d-%d", col, vcol) as i32
    }
}

/// Put the title name in the title bar and icon of the window.
pub fn maketitle() {
    let mut title_str: RawCCharPtrMut = ptr::null_mut();
    let mut icon_str: RawCCharPtrMut = ptr::null_mut();
    let mut buf = [0u8; IOSIZE];
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
            maxlen =
                ((opt_vars().p_titlelen * i64::from(g_st().columns_global) / 100) as i32).max(10);
        }

        if !crate::memory::cstr_is_empty(opt_vars().p_titlestring) {
            if (g_st().stl_syntax & STL_IN_TITLE) != 0 {
                build_stl_str_hl(
                    curwin_ptr(),
                    outbuf,
                    opt_vars().p_titlestring.cast(),
                    K_OPT_TITLESTRING,
                    0,
                    0,
                    maxlen,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                title_str = buf.as_mut_ptr().cast();
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
                .cast_mut()
                .cast();
            build_stl_str_hl(
                curwin_ptr(),
                outbuf,
                default_titlestring,
                K_OPT_TITLESTRING,
                0,
                0,
                maxlen,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            title_str = buf.as_mut_ptr().cast();
            #[cfg(windows)]
            {
                opt_vars().p_ssl = p_ssl_save;
            }
        }
    }
    let mut mustset = value_change(title_str, &mut buf_st().lasttitle);

    if opt_vars().p_icon != 0 {
        icon_str = buf.as_mut_ptr().cast();
        if !crate::memory::cstr_is_empty(opt_vars().p_iconstring) {
            if (g_st().stl_syntax & STL_IN_ICON) != 0 {
                build_stl_str_hl(
                    curwin_ptr(),
                    outbuf,
                    opt_vars().p_iconstring.cast(),
                    K_OPT_ICONSTRING,
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
            let mut name = buf_spname(unsafe { &*(g_st().curbuf) });
            if name.is_null() {
                name = path_tail(curbuf_mut().b_ffname);
            }
            let mut namelen = crate::memory::cstr_len(name) as i32;
            if namelen > 100 {
                namelen -= 100;
                namelen +=
                    i32::from(utf_cp_bounds(name, name.wrapping_add(namelen as usize)).end_off);
                name = name.wrapping_add(namelen as usize);
            }
            crate::memory::cstr_cpy(buf.as_mut_ptr().cast(), name);
            trans_characters(buf.as_mut_ptr().cast(), IOSIZE as i32);
        }
    }

    mustset |= value_change(icon_str, &mut buf_st().lasticon);

    if mustset {
        resettitle();
    }
}

fn value_change(
    str_: impl crate::strings::IntoCCharPtr,
    last: &mut Option<std::ffi::CString>,
) -> bool {
    let str_ = str_.into_c_char_ptr();
    {
        let last_ptr = last.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        if str_.is_null() != last_ptr.is_null()
            || (!str_.is_null()
                && !last_ptr.is_null()
                && crate::memory::cstr_as_bytes(str_) != crate::memory::cstr_as_bytes(last_ptr))
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
    let icon_ptr = unsafe { (&raw const buf_st().lasticon).as_ref() }
        .unwrap()
        .as_ref()
        .map_or(ptr::null(), |s| s.as_ptr());
    let title_ptr = unsafe { (&raw const buf_st().lasttitle).as_ref() }
        .unwrap()
        .as_ref()
        .map_or(ptr::null(), |s| s.as_ptr());
    ui_call_set_icon(cstr_as_string(icon_ptr));
    ui_call_set_title(cstr_as_string(title_ptr));
}

pub fn free_titles() {
    buf_st().lasttitle = None;
    buf_st().lasticon = None;
}

/// Get relative cursor position in window into `buf[]`.
pub fn get_rel_pos(wp: &mut WinInfoInfo, buf: *mut u8, buflen: i32) -> i32 {
    if buflen < 3 {
        return 0;
    }

    let mut above = wp.w_topline - 1;
    above += win_get_fill(wp, wp.w_topline) - wp.w_topfill;
    if wp.w_topline == 1 && wp.w_topfill >= 1 {
        above = 0;
    }
    let below = wp.buf_mut().b_ml.ml_line_count - wp.w_botline + 1;
    if below <= 0 {
        return vim_snprintf_safelen!(
            buf.cast(),
            buflen as usize,
            c"%s",
            if above == 0 {
                gettext(c"All")
            } else {
                gettext(c"Bot")
            },
        ) as i32;
    }

    if above <= 0 {
        return vim_snprintf_safelen!(buf.cast(), buflen as usize, c"%s", gettext(c"Top"),) as i32;
    }

    let perc = calc_percentage(i64::from(above), i64::from(above + below));
    let mut tmp = [0u8; 8];
    vim_snprintf!(tmp.as_mut_ptr().cast(), tmp.len(), gettext(c"%d%%"), perc);
    vim_snprintf_safelen!(
        buf.cast(),
        buflen as usize,
        gettext(c"%3s"),
        tmp.as_ptr().cast::<()>(),
    ) as i32
}

/// Append `(2 of 8)` to `buf[]`, if editing more than one file.
pub fn append_arg_number(wp: &mut WinInfoInfo, buf: *mut u8, buflen: usize) -> i32 {
    let argcount = crate::arglist::argcount();
    if argcount <= 1 {
        return 0;
    }

    let msg = if wp.w_arg_idx_invalid {
        gettext(c" ((%d) of %d)")
    } else {
        gettext(c" (%d of %d)")
    };

    vim_snprintf_safelen!(buf.cast(), buflen, msg, wp.w_arg_idx + 1, argcount) as i32
}

/// Make `*ffname` a full file name, set `*sfname` to `*ffname` if not NULL.
pub fn fname_expand<T, U>(
    buf: *mut FileBuf,
    ffname: *mut *mut T,
    sfname: *mut *mut U,
) -> Option<std::ffi::CString> {
    unsafe {
        #[cfg(not(windows))]
        let _ = buf;
        if (*ffname).is_null() {
            return None;
        }
        if (*sfname).is_null() {
            *sfname = (*ffname).cast();
        }
        let ffname_ptr: RawCCharPtr = (*ffname).cast();
        let Some(mut full_cstr) = fix_fname(ffname_ptr) else {
            *ffname = ptr::null_mut();
            *sfname = ptr::null_mut();
            return None;
        };
        if os_isdir(full_cstr.as_ptr()) {
            let sfname_ptr: RawCCharPtr = (*sfname).cast();
            let mut name_vec: Vec<u8> = cstr_to_cstring(sfname_ptr).into_bytes_with_nul();
            let name: RawCCharPtrMut = name_vec.as_mut_ptr().cast();
            let name_len = name_vec.len() - 1;
            if name_len > 0
                && after_pathsep(name, name.wrapping_add(name_len)) != 0
                && name.wrapping_add(name_len) > get_past_head(name)
            {
                *path_tail_with_sep(name) = 0;
            }
            let full = fix_fname(&name_vec);
            drop(name_vec);
            if let Some(dir_full) = full {
                full_cstr = dir_full;
            }
            full_cstr = concat_fnames_cstr(full_cstr.as_ptr(), c"", true);
        }

        #[cfg(windows)]
        if !buf.is_null()
            && (*buf).b_p_bin == 0
            && let Some(rfname_cstr) = os_resolve_shortcut(full_cstr.as_ptr())
        {
            full_cstr = rfname_cstr;
            *sfname = full_cstr.as_ptr().cast_mut().cast();
        }

        *ffname = full_cstr.as_ptr().cast_mut().cast();
        Some(full_cstr)
    }
}

/// Return true if `buf` is a prompt buffer.
pub fn bt_prompt(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'p'
}

/// Return true if `buf` is the `|cmdwin|` scratch buffer.
pub fn bt_cmdwin(buf: *const FileBuf) -> bool {
    !buf.is_null() && buf == g_st().cmdwin_buf.cast_const()
}

/// Open a window for a number of buffers.
pub fn ex_buffer_all(eap: &mut Exarg) {
    unsafe {
        let mut wpnext: *mut WinInfoInfo;
        let mut split_ret = OK;
        let mut open_wins = 0;
        let had_tab = g_st().cmdmod.cmod_tab;

        let count = if eap.addr_count == 0 { 9999 } else { eap.line2 };

        let all = eap.cmdidx != Cmdidx::CmdUnhide as i32 && eap.cmdidx != Cmdidx::CmdSunhide as i32;

        reset_visual_and_resel();
        setpcmark();

        if had_tab > 0 {
            goto_tabpage_tp(g_st().first_tabpage, true, true);
        }
        loop {
            let mut tpnext = curtab_mut().tp_next;
            let mut wp = if lastwin_mut().w_floating {
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
                        < i64::from(g_st().rows_global)
                            - opt_vars().p_ch
                            - i64::from(tabline_height())
                            - i64::from(global_stl_height())
                } else {
                    (*wp).w_width != g_st().columns_global
                };
                if ((*wp).buf_mut().b_nwindows > 1
                    || (*wp).w_floating
                    || size_mismatch
                    || (had_tab > 0 && wp != g_st().firstwin))
                    && g_st().firstwin != g_st().lastwin
                    && !(win_locked(&mut *wp) != 0 || (*wp).buf_mut().b_locked > 0)
                    && !is_ctx_win(wp)
                {
                    if !win_close(wp, false, false) {
                        break;
                    }
                    wpnext = if lastwin_mut().w_floating {
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

            let mut wp: *mut WinInfoInfo;
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
                    win_move_after(&mut *wp, curwin_mut());
                }
            }

            if wp.is_null() && split_ret == OK {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, buf);
                let p_ea_save = opt_vars().p_ea;
                opt_vars().p_ea = 1;
                let ok = win_split(0, WSP_ROOM | WSP_BELOW);
                split_ret = if ok { OK } else { FAIL };
                open_wins += 1;
                opt_vars().p_ea = p_ea_save;
                if !ok {
                    buf = (*buf).b_next;
                    continue;
                }

                g_st().swap_exists_action = SEA_DIALOG;
                set_curbuf(
                    buf,
                    DOBUF_GOTO,
                    (opt_vars().jop_flags & K_OPT_JOP_FLAG_CLEAN) == 0,
                );
                if !bufref_valid(Some(&bufref)) {
                    g_st().swap_exists_action = SEA_NONE;
                    break;
                }
                if g_st().swap_exists_action == SEA_QUIT {
                    let mut cs: ExCleanup = ExCleanup::new();
                    enter_cleanup(&mut cs);
                    win_close(curwin_ptr(), true, false);
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
            let r = (buf_hide((*wp).buf_mut())
                || !buf_is_changed((*wp).buf_mut())
                || autowrite((*wp).buf_mut(), false))
                && !is_ctx_win(wp);
            if !win_valid(wp) {
                wp = g_st().lastwin;
            } else if r {
                win_close(wp, !buf_hide((*wp).buf_mut()), false);
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
pub fn do_modelines(flags: i32) {
    let mut nmlines = opt_vars().p_mls as i32;
    if curbuf_mut().b_p_ml == 0 || nmlines == 0 {
        return;
    }

    if buf_st().modelines_entered != 0 {
        return;
    }

    buf_st().modelines_entered += 1;
    let mut lnum = 1;
    while curbuf_mut().b_p_ml != 0 && lnum <= curbuf_mut().b_ml.ml_line_count && lnum <= nmlines {
        if !chk_modeline(lnum, flags) {
            nmlines = 0;
        }
        lnum += 1;
    }

    lnum = curbuf_mut().b_ml.ml_line_count;
    while curbuf_mut().b_p_ml != 0
        && lnum > 0
        && lnum > nmlines
        && lnum > curbuf_mut().b_ml.ml_line_count - nmlines
    {
        if !chk_modeline(lnum, flags) {
            nmlines = 0;
        }
        lnum -= 1;
    }
    buf_st().modelines_entered -= 1;
}

fn chk_modeline(lnum: Linenr, flags: i32) -> bool {
    let mut e: RawCCharPtrMut;
    let mut retval = true;

    let mut prev: i32 = -1;
    let mut s = ml_get(lnum);
    let mut line_end = s.wrapping_add(ml_get_len(lnum) as usize);
    while !crate::memory::cstr_is_empty(s) {
        if prev == -1 || ascii_isspace(prev) {
            if (prev != -1 && (crate::memory::cstr_prefix_bytes(s, 3) == &b"ex:"[..3]))
                || (crate::memory::cstr_prefix_bytes(s, 3) == &b"vi:"[..3])
            {
                break;
            }
            if (crate::memory::cstr_first_byte(s) == b'v'
                || crate::memory::cstr_first_byte(s) == b'V')
                && crate::memory::cstr_byte_at(s, 1) == b'i'
                && crate::memory::cstr_byte_at(s, 2) == b'm'
            {
                let s3 = crate::memory::cstr_byte_at(s, 3);
                if s3 == b'<' || s3 == b'=' || s3 == b'>' {
                    e = s.wrapping_add(4);
                } else {
                    e = s.wrapping_add(3);
                }
                let mut vers: i64 = 0;
                if !try_getdigits(&raw mut e, &mut vers) {
                    prev = crate::memory::cstr_first_byte(s) as i32;
                    s = s.wrapping_add(1);
                    continue;
                }

                let vim_version = i64::from(min_vim_version());
                if crate::memory::cstr_first_byte(e) == b':'
                    && (crate::memory::cstr_first_byte(s) != b'V'
                        || (crate::memory::cstr_prefix_bytes(skipwhite(e.wrapping_add(1)), 3)
                            == &b"set"[..3]))
                    && (s3 == b':'
                        || (vim_version >= vers && s3.is_ascii_digit())
                        || (vim_version < vers && s3 == b'<')
                        || (vim_version > vers && s3 == b'>')
                        || (vim_version == vers && s3 == b'='))
                {
                    break;
                }
            }
        }
        prev = crate::memory::cstr_first_byte(s) as i32;
        s = s.wrapping_add(1);
    }

    if crate::memory::cstr_is_empty(s) {
        return retval;
    }

    loop {
        s = s.wrapping_add(1);
        if crate::memory::cstr_first_byte(s.wrapping_sub(1)) == b':' {
            break;
        }
    }

    let len = crate::memory::ptr_offset_from(line_end, s) as usize;
    let linecopy_cstr = cstr_from_bytes(crate::memory::cstr_slice(s, len));
    let mut linecopy_vec: Vec<u8> = linecopy_cstr.into_bytes_with_nul();
    s = linecopy_vec.as_mut_ptr().cast();
    line_end = s.wrapping_add(linecopy_vec.len() - 1);

    estack_push(ETYPE_MODELINE, c"modelines", lnum);

    let mut end = false;
    while !end {
        s = skipwhite(s);
        if crate::memory::cstr_is_empty(s) {
            break;
        }

        e = s;
        while crate::memory::cstr_first_byte(e) != b':' && !crate::memory::cstr_is_empty(e) {
            if crate::memory::cstr_first_byte(e) == b'\\'
                && crate::memory::cstr_byte_at(e, 1) == b':'
            {
                unsafe {
                    ptr::copy(
                        e.wrapping_add(1),
                        e,
                        crate::memory::ptr_offset_from(line_end, e.wrapping_add(1)) as usize + 1,
                    )
                };
                line_end = line_end.wrapping_sub(1);
            }
            e = e.wrapping_add(1);
        }
        if crate::memory::cstr_is_empty(e) {
            end = true;
        }

        if (crate::memory::cstr_prefix_bytes(s, 4) == &b"set "[..4])
            || (crate::memory::cstr_prefix_bytes(s, 3) == &b"se "[..3])
        {
            if crate::memory::cstr_first_byte(e) != b':' {
                break;
            }
            end = true;
            s = s.wrapping_add(if crate::memory::cstr_byte_at(s, 2) == b' ' {
                3
            } else {
                4
            });
        }
        unsafe {
            *e = NUL;
        }

        if !crate::memory::cstr_is_empty(s) {
            let secure_save = g_st().secure;
            let save_current_sctx = g_st().current_sctx;
            g_st().current_sctx.sc_sid = SID_MODELINE;
            g_st().current_sctx.sc_seq = 0;
            g_st().current_sctx.sc_lnum = lnum;
            g_st().secure = 1;

            retval = do_set(s, OPT_MODELINE | OPT_LOCAL | flags);

            g_st().secure = secure_save;
            g_st().current_sctx = save_current_sctx;
            if !retval {
                break;
            }
        }
        s = if e == line_end { e } else { e.wrapping_add(1) };
    }

    estack_pop();
    drop(linecopy_vec);

    retval
}

/// Return true if `buf` is a help buffer.
pub fn bt_help(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && buf.unwrap().b_help
}

/// Return true if `buf` is a normal buffer, `'buftype'` is empty.
pub fn bt_normal(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && crate::memory::cstr_is_empty(buf.unwrap().b_p_bt)
}

/// Return true if `buf` is the quickfix buffer.
pub fn bt_quickfix(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'q'
}

/// Return true if `buf` is a terminal buffer.
pub fn bt_terminal(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b't'
}

/// Return true if `buf` is a `"nofile"`, `"acwrite"`, `"terminal"` or `"prompt"` buffer.
pub fn bt_nofilename(buf: Option<&FileBuf>) -> bool {
    buf.is_some()
        && ((crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'n'
            && crate::memory::cstr_byte_at(buf.unwrap().b_p_bt, 2) == b'f')
            || crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'a'
            || !buf.unwrap().terminal.is_null()
            || crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'p')
}

fn bt_nofileread(buf: Option<&FileBuf>) -> bool {
    buf.is_some()
        && ((crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'n'
            && crate::memory::cstr_byte_at(buf.unwrap().b_p_bt, 2) == b'f')
            || crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b't'
            || crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'q'
            || crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'p')
}

/// Return true if `buf` has `'buftype'` set to `"nofile"`.
pub fn bt_nofile(buf: Option<&FileBuf>) -> bool {
    buf.is_some()
        && crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'n'
        && crate::memory::cstr_byte_at(buf.unwrap().b_p_bt, 2) == b'f'
}

/// Return true if `buf` is a `"nowrite"`, `"nofile"`, `"terminal"` or `"prompt"` buffer.
pub fn bt_dontwrite(buf: Option<&FileBuf>) -> bool {
    buf.is_some()
        && (crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'n'
            || !buf.unwrap().terminal.is_null()
            || crate::memory::cstr_first_byte(buf.unwrap().b_p_bt) == b'p')
}

pub fn bt_dontwrite_msg(buf: Option<&FileBuf>) -> bool {
    if bt_dontwrite(buf) {
        emsg(gettext(c"E382: Cannot write, 'buftype' option is set"));
        return true;
    }
    false
}

/// Return true if the buffer should be hidden, according to `'hidden'`, `":hide"` and `'bufhidden'`.
pub fn buf_hide(buf: &FileBuf) -> bool {
    match crate::memory::cstr_first_byte(buf.b_p_bh) {
        b'u' | b'w' | b'd' => false,
        b'h' => true,
        _ => opt_vars().p_hid != 0 || (g_st().cmdmod.cmod_flags & CMOD_HIDE) != 0,
    }
}

/// Return special buffer name or NULL when the buffer has a normal file name.
pub fn buf_spname(buf: &FileBuf) -> RawCCharPtrMut {
    if bt_quickfix(Some(buf)) {
        if buf.b_fnum == qf_stack_get_bufnr() {
            return gettext(MSG_QFLIST);
        }
        return gettext(MSG_LOCLIST);
    }
    if bt_nofilename(Some(buf)) {
        if !buf.b_fname.is_null() {
            return buf.b_fname;
        }
        if bt_cmdwin(buf) {
            return gettext(c"[Command Line]");
        }
        if bt_prompt(Some(buf)) {
            return gettext(c"[Prompt]");
        }
        return gettext(c"[Scratch]");
    }
    if buf.b_fname.is_null() {
        return buf_get_fname(buf);
    }
    ptr::null_mut()
}

/// Get `buf->b_fname`, use `"[No Name]"` if it is NULL.
pub fn buf_get_fname(buf: &FileBuf) -> RawCCharPtrMut {
    if buf.b_fname.is_null() {
        gettext(c"[No Name]")
    } else {
        buf.b_fname
    }
}

/// Set `'buflisted'` for `curbuf` to `on` and trigger autocommands if it changed.
pub fn set_buflisted(on: i32) {
    if on == curbuf_mut().b_p_bl {
        return;
    }

    curbuf_mut().b_p_bl = on;
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

/// Read the file for `buf` again and check if the contents changed.
pub fn buf_contents_changed(buf: &mut FileBuf) -> bool {
    let mut differ = true;

    let newbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
    if newbuf.is_null() {
        return true;
    }

    let mut ea: Exarg = Exarg::new();
    let _ea_cmd = prep_exarg(&mut ea, &*buf);

    let mut aco: CtxSwitch = CtxSwitch::new();
    ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), newbuf, 0);

    block_autocmds();

    if ml_open(curbuf_mut())
        && readfile(
            buf.b_ffname,
            buf.b_fname,
            0,
            0,
            MAXLNUM,
            Some(&ea),
            READ_NEW | READ_DUMMY,
            false,
        ) == OK
        && buf.b_ml.ml_line_count == curbuf_mut().b_ml.ml_line_count
    {
        differ = false;
        for lnum in 1..=curbuf_mut().b_ml.ml_line_count {
            if crate::memory::cstr_as_bytes(ml_get_buf(buf, lnum))
                != crate::memory::cstr_as_bytes(ml_get(lnum))
            {
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

/// Wipe out a (typically temporary) buffer.
pub fn wipe_buffer(buf: *mut FileBuf, aucmd: bool) {
    if !aucmd {
        block_autocmds();
    }
    close_buffer(ptr::null_mut(), buf, DOBUF_WIPE, false, true, false);
    if !aucmd {
        unblock_autocmds();
    }
}

pub fn buf_is_empty(buf: &mut FileBuf) -> bool {
    unsafe { buf.b_ml.ml_line_count == 1 && *ml_get_buf(buf, 1) == NUL }
}

/// Increment `b:changedtick` value.
pub fn buf_inc_changedtick(buf: &mut FileBuf) {
    buf_set_changedtick(buf, buf_get_changedtick(buf) + 1);
}

/// Set `b:changedtick`, also checking `b:` for consistency in debug build.
pub fn buf_set_changedtick(buf: &mut FileBuf, changedtick: Varnumber) {
    let old_val = buf.changedtick_di.di_tv;

    let changedtick_di = tv_dict_find(Some(&*buf.b_vars()), c"changedtick", 11);
    debug_assert!(!changedtick_di.is_null());
    debug_assert!(unsafe { (*changedtick_di).di_tv.v_type } == VarType::VarNumber);
    debug_assert!(unsafe { (*changedtick_di).di_tv.v_lock } == VAR_FIXED);
    debug_assert!(unsafe { (*changedtick_di).di_flags } == (DI_FLAGS_RO | DI_FLAGS_FIX));
    debug_assert!(changedtick_di == (&raw mut buf.changedtick_di).cast());
    let _ = changedtick_di;

    buf.changedtick_di.di_tv.vval = crate::eval::typval::TypvalVval::Number(changedtick);

    if tv_dict_is_watched(Some(&*buf.b_vars())) {
        buf.b_locked += 1;
        tv_dict_watcher_notify(
            &mut *buf.b_vars(),
            &buf.changedtick_di.di_key,
            Some(&buf.changedtick_di.di_tv),
            Some(&old_val),
        );
        buf.b_locked -= 1;
    }
}

/// Read the given buffer contents into a string.
pub fn read_buffer_into(
    mut buf: Option<&mut FileBuf>,
    start: Linenr,
    end: Linenr,
    sb: &mut Vec<u8>,
) {
    debug_assert!(buf.is_some());

    if (buf.as_deref_mut().unwrap().b_ml.ml_flags & ML_EMPTY) != 0 {
        return;
    }

    let mut written = 0usize;
    let mut len: usize;
    let mut lnum = start;
    let mut lp = ml_get_buf(buf.as_deref_mut().unwrap(), lnum);
    let mut lplen = ml_get_buf_len(buf.as_deref_mut().unwrap(), lnum) as usize;

    loop {
        if lplen == 0 {
            len = 0;
        } else if unsafe { *lp.wrapping_add(written) } == NL {
            len = 1;
            sb.push(0);
        } else {
            let s = vim_strchr(lp.wrapping_add(written), NL as i32);
            len = if s.is_null() {
                lplen - written
            } else {
                crate::memory::ptr_offset_from(s, lp.wrapping_add(written)) as usize
            };
            if len > 0 {
                sb.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(lp.wrapping_add(written).cast::<u8>(), len)
                });
            }
        }

        if len == lplen - written {
            if lnum != end
                || (buf.as_deref_mut().unwrap().b_p_bin == 0
                    && buf.as_deref_mut().unwrap().b_p_fixeol != 0)
                || (lnum != buf.as_deref_mut().unwrap().b_no_eol_lnum
                    && (lnum != buf.as_deref_mut().unwrap().b_ml.ml_line_count
                        || buf.as_deref_mut().unwrap().b_p_eol != 0))
            {
                sb.push(b'\n');
            }
            lnum += 1;
            if lnum > end {
                break;
            }
            lp = ml_get_buf(buf.as_deref_mut().unwrap(), lnum);
            lplen = ml_get_buf_len(buf.as_deref_mut().unwrap(), lnum) as usize;
            written = 0;
        } else if len > 0 {
            written += len;
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
        assert!(!bt_help(None));
        assert!(!bt_normal(None));
        assert!(!bt_quickfix(None));
        assert!(!bt_terminal(None));
        assert!(!bt_nofile(None));
        assert!(!bt_dontwrite(None));
    }

    #[test]
    fn buf_valid_and_buflist_findpat_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
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

            let findpat = |pat: &CStr, allow_unlisted: bool| -> i32 {
                buflist_findpat(pat.as_ptr(), ptr::null(), allow_unlisted, false, false)
            };

            let buf1 = buflist_new(
                p1.as_mut_ptr().cast(),
                p1.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            let buf1 = unsafe { &mut *buf1 };
            let buf2 = buflist_new(
                p2.as_mut_ptr().cast(),
                p2.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            let buf2 = unsafe { &mut *buf2 };
            let buf3 = buflist_new(
                p3.as_mut_ptr().cast(),
                p3.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            let buf3 = unsafe { &mut *buf3 };

            assert_eq!(findpat(c"test_file_path", false), buf1.b_fnum);
            assert_eq!(findpat(c"test", false), buf1.b_fnum);
            assert_eq!(findpat(c"file", false), buf2.b_fnum);
            assert_eq!(findpat(c"path", false), buf3.b_fnum);
            assert_eq!(findpat(c"_test_", false), buf3.b_fnum);

            // Include / ignore unlisted buffers based on the flag
            close_buffer(ptr::null_mut(), buf3, DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"_test_", false), -1);
            assert_eq!(findpat(c"_test_", true), buf3.b_fnum);
            close_buffer(ptr::null_mut(), buf3, DOBUF_WIPE, false, false, false);
            assert_eq!(findpat(c"_test_", false), -1);
            assert_eq!(findpat(c"_test_", true), -1);

            // Unlisted vs listed preference
            close_buffer(ptr::null_mut(), buf1, DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"test", false), buf2.b_fnum);
            assert_eq!(findpat(c"test", true), buf2.b_fnum);
            close_buffer(ptr::null_mut(), buf2, DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"test", true), buf1.b_fnum);
            assert_eq!(findpat(c"test", false), -1);

            close_buffer(ptr::null_mut(), buf1, DOBUF_WIPE, false, false, false);
            close_buffer(ptr::null_mut(), buf2, DOBUF_WIPE, false, false, false);

            g_st().emsg_skip -= 1;
        }
    }
}
