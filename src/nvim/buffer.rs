//! Functions for dealing with the buffer structure (`src/nvim/buffer.c`).
#![forbid(unsafe_code)]

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
    ascii_isdigit, ascii_isspace, buf_init_isk_chartab, getdigits_int, trans_characters,
    try_getdigits, vim_strsize,
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
use crate::hashtab::{hash_find_idx, hash_remove};
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
use crate::mbyte::utf_cp_bounds;
use crate::mcursor::{mc_buf_clear, mc_buf_free};
use crate::memline::{
    ml_close, ml_delete, ml_get_buf_slice, ml_open, ml_recover, ml_setname, ml_timestamp,
};
use crate::memory::entered_free_all_mem;
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
use crate::optionstr::OptIndex;
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
use crate::regexp::{RE_MAGIC, Regmatch, vim_regcomp, vim_regexec};
use crate::runtime::{ETYPE_MODELINE, estack_pop, estack_push};
use crate::spell::parse_spelllang;
use crate::state::{MAP_ALL_MODES, MODE_INSERT};
use crate::statusline::build_stl_str_hl_slice;
use crate::strings::{format_to_buf, format_to_buf_safelen, gettext, ngettext};
use crate::syntax::{Synblock, reset_synblock, syntax_clear};
use crate::terminal::{TermId, terminal_check_size, terminal_close, terminal_running};
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

pub(crate) const BCO_ENTER: i32 = 1;
pub(crate) const BCO_NOHELP: i32 = 4;
pub(crate) const BF_DUMMY: i32 = 0x80;

#[repr(C)]
#[derive(Clone)]
pub struct Bufref {
    pub br_buf: crate::globals::BufPtr,
    pub br_fnum: i32,
    pub br_buf_free_count: i32,
}

impl Bufref {
    #[inline]
    pub const fn new() -> Self {
        Self {
            br_buf: crate::globals::BufPtr::NULL,
            br_fnum: 0,
            br_buf_free_count: 0,
        }
    }

    #[inline]
    pub fn buf_opt(&self) -> Option<&FileBuf> {
        self.br_buf.opt_mut()
    }
}

impl Default for Bufref {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct BufSigncols {
    pub max: std::cell::Cell<i32>,
    pub last_max: std::cell::Cell<i32>,
    pub count: [std::cell::Cell<i32>; 9],
    pub autom: std::cell::Cell<bool>,
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

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BufFname {
    #[default]
    None,
    Short,
    Full,
}

#[repr(C)]
pub struct FileBuf {
    pub self_weak: std::rc::Weak<Self>,
    pub b_fnum: std::cell::Cell<crate::msgpack_rpc::packer::ObjHandle>,
    pub b_ml: crate::memline::Memline,
    pub b_next: crate::globals::CloneCell<crate::globals::BufPtr>,
    pub b_prev: crate::globals::CloneCell<crate::globals::BufPtr>,
    pub b_nwindows: std::cell::Cell<i32>,
    pub b_flags: std::cell::Cell<i32>,
    pub b_locked: std::cell::Cell<i32>,
    pub b_locked_split: std::cell::Cell<i32>,
    pub b_ro_locked: std::cell::Cell<i32>,
    pub b_ffname: crate::globals::CloneCell<Option<crate::option_vars::RcCStr>>,
    pub b_sfname: crate::globals::CloneCell<Option<crate::option_vars::RcCStr>>,
    pub b_fname: std::cell::Cell<BufFname>,
    pub file_id_valid: std::cell::Cell<bool>,
    pub file_id: std::cell::Cell<FileID>,
    pub b_changed: std::cell::Cell<i32>,
    pub changedtick_di: crate::eval::typval::DictitemId,
    pub b_last_changedtick: std::cell::Cell<i64>,
    pub b_last_changedtick_i: std::cell::Cell<i64>,
    pub b_last_changedtick_pum: std::cell::Cell<i64>,
    pub b_saving: std::cell::Cell<bool>,
    pub b_mod_set: std::cell::Cell<bool>,
    pub b_mod_top: std::cell::Cell<i32>,
    pub b_mod_bot: std::cell::Cell<i32>,
    pub b_mod_xlines: std::cell::Cell<i32>,
    pub b_wininfo: std::cell::RefCell<Vec<WinInfo>>,
    pub b_mod_tick_syn: std::cell::Cell<u64>,
    pub b_mod_tick_decor: std::cell::Cell<u64>,
    pub b_mtime: std::cell::Cell<i64>,
    pub b_mtime_ns: std::cell::Cell<i64>,
    pub b_mtime_read: std::cell::Cell<i64>,
    pub b_mtime_read_ns: std::cell::Cell<i64>,
    pub b_orig_size: std::cell::Cell<u64>,
    pub b_orig_mode: std::cell::Cell<i32>,
    pub b_last_used: std::cell::Cell<i64>,
    pub b_namedm: std::cell::RefCell<[Fmark; 26]>,
    pub b_visual: std::cell::RefCell<crate::normal::Visualinfo>,
    pub b_visual_mode_eval: std::cell::Cell<i32>,
    pub b_last_cursor: std::cell::RefCell<Fmark>,
    pub b_last_insert: std::cell::RefCell<Fmark>,
    pub b_last_change: std::cell::RefCell<Fmark>,
    pub b_changelist: std::cell::RefCell<[Fmark; 100]>,
    pub b_changelistlen: std::cell::Cell<i32>,
    pub b_new_change: std::cell::Cell<bool>,
    pub b_chartab: std::cell::Cell<[u64; 4]>,
    pub b_maphash: std::cell::RefCell<[Vec<crate::mapping::Mapblock>; 256]>,
    pub b_first_abbr: std::cell::RefCell<Vec<crate::mapping::Mapblock>>,
    pub b_ucmds: std::cell::RefCell<Vec<crate::usercmd::Ucmd>>,
    pub b_op_start: std::cell::Cell<CursorPos>,
    pub b_op_start_orig: std::cell::Cell<CursorPos>,
    pub b_op_end: std::cell::Cell<CursorPos>,
    pub b_marks_read: std::cell::Cell<bool>,
    pub b_modified_was_set: std::cell::Cell<bool>,
    pub b_did_filetype: std::cell::Cell<bool>,
    pub b_keep_filetype: std::cell::Cell<bool>,
    pub b_au_did_filetype: std::cell::Cell<bool>,
    pub b_u_headers: std::cell::RefCell<Vec<crate::undo::UHeader>>,
    pub b_u_free_headers: std::cell::RefCell<Vec<usize>>,
    pub b_u_oldhead: std::cell::Cell<Option<usize>>,
    pub b_u_newhead: std::cell::Cell<Option<usize>>,
    pub b_u_curhead: std::cell::Cell<Option<usize>>,
    pub b_u_numhead: std::cell::Cell<i32>,
    pub b_u_synced: std::cell::Cell<bool>,
    pub b_u_seq_last: std::cell::Cell<i32>,
    pub b_u_save_nr_last: std::cell::Cell<i32>,
    pub b_u_seq_cur: std::cell::Cell<i32>,
    pub b_u_time_cur: std::cell::Cell<i64>,
    pub b_u_save_nr_cur: std::cell::Cell<i32>,
    pub b_u_line_ptr: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub b_u_line_lnum: std::cell::Cell<i32>,
    pub b_u_line_colnr: std::cell::Cell<i32>,
    pub b_scanned: std::cell::Cell<bool>,
    pub b_p_iminsert: std::cell::Cell<i64>,
    pub b_p_imsearch: std::cell::Cell<i64>,
    pub b_kmap_state: std::cell::Cell<i16>,
    pub b_kmap_ga: std::cell::RefCell<Vec<crate::digraph::Kmap>>,
    pub b_p_initialized: std::cell::Cell<bool>,
    pub b_p_script_ctx: [std::cell::Cell<crate::runtime::Sctx>; 93],
    pub b_p_ac: std::cell::Cell<Option<bool>>,
    pub b_p_ai: std::cell::Cell<bool>,
    pub b_p_ai_nopaste: std::cell::Cell<bool>,
    pub b_p_bkc: crate::option_vars::OptString,
    pub b_bkc_flags: std::cell::Cell<u32>,
    pub b_p_ci: std::cell::Cell<bool>,
    pub b_p_bin: std::cell::Cell<bool>,
    pub b_p_bomb: std::cell::Cell<bool>,
    pub b_p_bh: crate::option_vars::OptString,
    pub b_p_bt: crate::option_vars::OptString,
    pub b_p_busy: std::cell::Cell<i64>,
    pub b_has_qf_entry: std::cell::Cell<i32>,
    pub b_p_bl: std::cell::Cell<bool>,
    pub b_p_channel: std::cell::Cell<i64>,
    pub b_p_cin: std::cell::Cell<bool>,
    pub b_p_cino: crate::option_vars::OptString,
    pub b_p_cink: crate::option_vars::OptString,
    pub b_p_cinw: crate::option_vars::OptString,
    pub b_p_cinsd: crate::option_vars::OptString,
    pub b_p_com: crate::option_vars::OptString,
    pub b_p_cms: crate::option_vars::OptString,
    pub b_p_cot: crate::option_vars::OptString,
    pub b_cot_flags: std::cell::Cell<u32>,
    pub b_p_cpt: crate::option_vars::OptString,
    #[cfg(windows)]
    pub b_p_csl: crate::option_vars::OptString,
    pub b_p_cpt_flags: std::cell::Cell<u32>,
    pub b_p_cpt_cb: std::cell::RefCell<Vec<crate::eval::typval::Callback>>,
    pub b_p_cfu: crate::eval::typval::Callback,
    pub b_p_ofu: crate::eval::typval::Callback,
    pub b_p_tfu: crate::eval::typval::Callback,
    pub b_p_ffu: crate::eval::typval::Callback,
    pub b_p_eof: std::cell::Cell<bool>,
    pub b_p_eol: std::cell::Cell<bool>,
    pub b_p_fixeol: std::cell::Cell<bool>,
    pub b_p_et: std::cell::Cell<bool>,
    pub b_p_et_nobin: std::cell::Cell<bool>,
    pub b_p_et_nopaste: std::cell::Cell<bool>,
    pub b_p_fenc: crate::option_vars::OptString,
    pub b_p_ff: crate::option_vars::OptString,
    pub b_p_ft: crate::option_vars::OptString,
    pub b_p_follow: std::cell::Cell<bool>,
    pub b_p_fo: crate::option_vars::OptString,
    pub b_p_flp: crate::option_vars::OptString,
    pub b_p_inf: std::cell::Cell<bool>,
    pub b_p_isk: crate::option_vars::OptString,
    pub b_p_def: crate::option_vars::OptString,
    pub b_p_inc: crate::option_vars::OptString,
    pub b_p_inex: crate::eval::typval::Callback,
    pub b_p_inex_flags: std::cell::Cell<u32>,
    pub b_p_inde: crate::eval::typval::Callback,
    pub b_p_inde_flags: std::cell::Cell<u32>,
    pub b_p_indk: crate::option_vars::OptString,
    pub b_p_fp: crate::option_vars::OptString,
    pub b_p_fex: crate::eval::typval::Callback,
    pub b_p_fex_flags: std::cell::Cell<u32>,
    pub b_p_fs: std::cell::Cell<Option<bool>>,
    pub b_p_kp: crate::option_vars::OptString,
    pub b_p_lisp: std::cell::Cell<bool>,
    pub b_p_lop: crate::option_vars::OptString,
    pub b_p_menc: crate::option_vars::OptString,
    pub b_p_mps: crate::option_vars::OptString,
    pub b_p_ml: std::cell::Cell<bool>,
    pub b_p_ml_nobin: std::cell::Cell<bool>,
    pub b_p_ma: std::cell::Cell<bool>,
    pub b_p_nf: crate::option_vars::OptString,
    pub b_p_pi: std::cell::Cell<bool>,
    pub b_p_qe: crate::option_vars::OptString,
    pub b_p_ro: std::cell::Cell<bool>,
    pub b_p_sw: std::cell::Cell<i64>,
    pub b_p_scbk: std::cell::Cell<i64>,
    pub b_p_si: std::cell::Cell<bool>,
    pub b_p_sts: std::cell::Cell<i64>,
    pub b_p_sts_nopaste: std::cell::Cell<i64>,
    pub b_p_sua: crate::option_vars::OptString,
    pub b_p_swf: std::cell::Cell<bool>,
    pub b_p_smc: std::cell::Cell<i64>,
    pub b_p_syn: crate::option_vars::OptString,
    pub b_p_ts: std::cell::Cell<i64>,
    pub b_p_tw: std::cell::Cell<i64>,
    pub b_p_tw_nobin: std::cell::Cell<i64>,
    pub b_p_tw_nopaste: std::cell::Cell<i64>,
    pub b_p_wm: std::cell::Cell<i64>,
    pub b_p_wm_nobin: std::cell::Cell<i64>,
    pub b_p_wm_nopaste: std::cell::Cell<i64>,
    pub b_p_vsts: crate::option_vars::OptString,
    pub b_p_vsts_array: std::cell::RefCell<Vec<i32>>,
    pub b_p_vsts_nopaste: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub b_p_vts: crate::option_vars::OptString,
    pub b_p_vts_array: std::cell::RefCell<Vec<i32>>,
    pub b_p_keymap: crate::option_vars::OptString,
    pub b_p_gefm: crate::option_vars::OptString,
    pub b_p_gp: crate::option_vars::OptString,
    pub b_p_mp: crate::option_vars::OptString,
    pub b_p_efm: crate::option_vars::OptString,
    pub b_p_ep: crate::option_vars::OptString,
    pub b_p_path: crate::option_vars::OptString,
    pub b_p_ar: std::cell::Cell<Option<bool>>,
    pub b_p_tags: crate::option_vars::OptString,
    pub b_p_tc: crate::option_vars::OptString,
    pub b_tc_flags: std::cell::Cell<u32>,
    pub b_p_dict: crate::option_vars::OptString,
    pub b_p_dia: crate::option_vars::OptString,
    pub b_p_tsr: crate::option_vars::OptString,
    pub b_p_tsrfu: crate::eval::typval::Callback,
    pub b_p_ul: std::cell::Cell<i64>,
    pub b_p_udf: std::cell::Cell<bool>,
    pub b_p_lw: crate::option_vars::OptString,
    pub b_ind_level: std::cell::Cell<i32>,
    pub b_ind_open_imag: std::cell::Cell<i32>,
    pub b_ind_no_brace: std::cell::Cell<i32>,
    pub b_ind_first_open: std::cell::Cell<i32>,
    pub b_ind_open_extra: std::cell::Cell<i32>,
    pub b_ind_close_extra: std::cell::Cell<i32>,
    pub b_ind_open_left_imag: std::cell::Cell<i32>,
    pub b_ind_jump_label: std::cell::Cell<i32>,
    pub b_ind_case: std::cell::Cell<i32>,
    pub b_ind_case_code: std::cell::Cell<i32>,
    pub b_ind_case_break: std::cell::Cell<i32>,
    pub b_ind_param: std::cell::Cell<i32>,
    pub b_ind_func_type: std::cell::Cell<i32>,
    pub b_ind_comment: std::cell::Cell<i32>,
    pub b_ind_in_comment: std::cell::Cell<i32>,
    pub b_ind_in_comment2: std::cell::Cell<i32>,
    pub b_ind_cpp_baseclass: std::cell::Cell<i32>,
    pub b_ind_continuation: std::cell::Cell<i32>,
    pub b_ind_unclosed: std::cell::Cell<i32>,
    pub b_ind_unclosed2: std::cell::Cell<i32>,
    pub b_ind_unclosed_noignore: std::cell::Cell<i32>,
    pub b_ind_unclosed_wrapped: std::cell::Cell<i32>,
    pub b_ind_unclosed_whiteok: std::cell::Cell<i32>,
    pub b_ind_matching_paren: std::cell::Cell<i32>,
    pub b_ind_paren_prev: std::cell::Cell<i32>,
    pub b_ind_maxparen: std::cell::Cell<i32>,
    pub b_ind_maxcomment: std::cell::Cell<i32>,
    pub b_ind_scopedecl: std::cell::Cell<i32>,
    pub b_ind_scopedecl_code: std::cell::Cell<i32>,
    pub b_ind_java: std::cell::Cell<i32>,
    pub b_ind_js: std::cell::Cell<i32>,
    pub b_ind_keep_case_label: std::cell::Cell<i32>,
    pub b_ind_hash_comment: std::cell::Cell<i32>,
    pub b_ind_cpp_namespace: std::cell::Cell<i32>,
    pub b_ind_if_for_while: std::cell::Cell<i32>,
    pub b_ind_cpp_extern_c: std::cell::Cell<i32>,
    pub b_ind_pragma: std::cell::Cell<i32>,
    pub b_no_eol_lnum: std::cell::Cell<i32>,
    pub b_start_eof: std::cell::Cell<bool>,
    pub b_start_eol: std::cell::Cell<bool>,
    pub b_start_ffc: std::cell::Cell<i32>,
    pub b_start_fenc: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub b_bad_char: std::cell::Cell<i32>,
    pub b_start_bomb: std::cell::Cell<bool>,
    pub b_bufvar: crate::eval::typval::DictitemId,
    pub b_may_swap: std::cell::Cell<bool>,
    pub b_did_warn: std::cell::Cell<bool>,
    pub b_help: std::cell::Cell<bool>,
    pub b_spell: std::cell::Cell<bool>,
    pub b_prompt_text: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub b_prompt_callback: crate::eval::typval::Callback,
    pub b_prompt_interrupt: crate::eval::typval::Callback,
    pub b_prompt_append_new_line: std::cell::Cell<bool>,
    pub b_prompt_insert: std::cell::Cell<i32>,
    pub b_prompt_start: std::cell::RefCell<Fmark>,
    pub b_s: std::rc::Rc<Synblock>,
    pub b_signcols: BufSigncols,
    pub terminal: std::cell::Cell<TermId>,
    pub additional_data: std::cell::RefCell<Option<crate::search::AdditionalData>>,
    pub b_mapped_ctrl_c: std::cell::Cell<i32>,
    pub b_marktree: std::cell::RefCell<crate::marktree::MarkTree>,
    pub b_extmark_ns: std::cell::RefCell<std::collections::BTreeMap<u32, u32>>,
    pub b_conceal_providers: std::cell::Cell<i32>,
    pub b_prev_line_count: std::cell::Cell<i32>,
    pub update_channels: std::cell::RefCell<Vec<u64>>,
    pub update_callbacks: std::cell::RefCell<Vec<BufUpdateCallbacks>>,
    pub update_need_codepoints: std::cell::Cell<bool>,
    pub deleted_bytes: std::cell::Cell<usize>,
    pub deleted_bytes2: std::cell::Cell<usize>,
    pub deleted_codepoints: std::cell::Cell<usize>,
    pub deleted_codeunits: std::cell::Cell<usize>,
    pub flush_count: std::cell::Cell<i32>,
    pub b_localdir: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub b_prevdir: crate::globals::CloneCell<Option<std::ffi::CString>>,
}

impl FileBuf {
    #[inline]
    pub fn terminal_mut(&self) -> TermId {
        self.terminal.get()
    }

    #[inline]
    pub fn terminal_opt(&self) -> Option<TermId> {
        self.terminal.get().opt()
    }

    pub fn new() -> Self {
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
            self_weak: std::rc::Weak::new(),
            b_fnum: std::cell::Cell::new(0),
            b_ml: crate::memline::Memline::new(),
            b_next: crate::globals::CloneCell::new(crate::globals::BufPtr::NULL),
            b_prev: crate::globals::CloneCell::new(crate::globals::BufPtr::NULL),
            b_nwindows: std::cell::Cell::new(0),
            b_flags: std::cell::Cell::new(0),
            b_locked: std::cell::Cell::new(0),
            b_locked_split: std::cell::Cell::new(0),
            b_ro_locked: std::cell::Cell::new(0),
            b_ffname: crate::globals::CloneCell::new(None),
            b_sfname: crate::globals::CloneCell::new(None),
            b_fname: std::cell::Cell::new(BufFname::None),
            file_id_valid: std::cell::Cell::new(false),
            file_id: std::cell::Cell::new(FileID {
                inode: 0,
                device_id: 0,
            }),
            b_changed: std::cell::Cell::new(0),
            changedtick_di: crate::eval::typval::DictitemId::NULL,
            b_last_changedtick: std::cell::Cell::new(0),
            b_last_changedtick_i: std::cell::Cell::new(0),
            b_last_changedtick_pum: std::cell::Cell::new(0),
            b_saving: std::cell::Cell::new(false),
            b_mod_set: std::cell::Cell::new(false),
            b_mod_top: std::cell::Cell::new(0),
            b_mod_bot: std::cell::Cell::new(0),
            b_mod_xlines: std::cell::Cell::new(0),
            b_wininfo: std::cell::RefCell::new(Vec::new()),
            b_mod_tick_syn: std::cell::Cell::new(0),
            b_mod_tick_decor: std::cell::Cell::new(0),
            b_mtime: std::cell::Cell::new(0),
            b_mtime_ns: std::cell::Cell::new(0),
            b_mtime_read: std::cell::Cell::new(0),
            b_mtime_read_ns: std::cell::Cell::new(0),
            b_orig_size: std::cell::Cell::new(0),
            b_orig_mode: std::cell::Cell::new(0),
            b_last_used: std::cell::Cell::new(0),
            b_namedm: std::cell::RefCell::new([const { Fmark::ZERO }; 26]),
            b_visual: std::cell::RefCell::new(crate::normal::Visualinfo {
                vi_start: ZERO_POS,
                vi_end: ZERO_POS,
                vi_mode: 0,
                vi_curswant: 0,
            }),
            b_visual_mode_eval: std::cell::Cell::new(0),
            b_last_cursor: std::cell::RefCell::new(Fmark::ZERO),
            b_last_insert: std::cell::RefCell::new(Fmark::ZERO),
            b_last_change: std::cell::RefCell::new(Fmark::ZERO),
            b_changelist: std::cell::RefCell::new([const { Fmark::ZERO }; 100]),
            b_changelistlen: std::cell::Cell::new(0),
            b_new_change: std::cell::Cell::new(false),
            b_chartab: std::cell::Cell::new([0; 4]),
            b_maphash: std::cell::RefCell::new([const { Vec::new() }; 256]),
            b_first_abbr: std::cell::RefCell::new(Vec::new()),
            b_ucmds: std::cell::RefCell::new(Vec::new()),
            b_op_start: std::cell::Cell::new(ZERO_POS),
            b_op_start_orig: std::cell::Cell::new(ZERO_POS),
            b_op_end: std::cell::Cell::new(ZERO_POS),
            b_marks_read: std::cell::Cell::new(false),
            b_modified_was_set: std::cell::Cell::new(false),
            b_did_filetype: std::cell::Cell::new(false),
            b_keep_filetype: std::cell::Cell::new(false),
            b_au_did_filetype: std::cell::Cell::new(false),
            b_u_headers: std::cell::RefCell::new(Vec::new()),
            b_u_free_headers: std::cell::RefCell::new(Vec::new()),
            b_u_oldhead: std::cell::Cell::new(None),
            b_u_newhead: std::cell::Cell::new(None),
            b_u_curhead: std::cell::Cell::new(None),
            b_u_numhead: std::cell::Cell::new(0),
            b_u_synced: std::cell::Cell::new(false),
            b_u_seq_last: std::cell::Cell::new(0),
            b_u_save_nr_last: std::cell::Cell::new(0),
            b_u_seq_cur: std::cell::Cell::new(0),
            b_u_time_cur: std::cell::Cell::new(0),
            b_u_save_nr_cur: std::cell::Cell::new(0),
            b_u_line_ptr: crate::globals::CloneCell::new(None),
            b_u_line_lnum: std::cell::Cell::new(0),
            b_u_line_colnr: std::cell::Cell::new(0),
            b_scanned: std::cell::Cell::new(false),
            b_p_iminsert: std::cell::Cell::new(0),
            b_p_imsearch: std::cell::Cell::new(0),
            b_kmap_state: std::cell::Cell::new(0),
            b_kmap_ga: std::cell::RefCell::new(Vec::new()),
            b_p_initialized: std::cell::Cell::new(false),
            b_p_script_ctx: [const { std::cell::Cell::new(ZERO_SCTX) }; 93],
            b_p_ac: std::cell::Cell::new(Some(false)),
            b_p_ai: std::cell::Cell::new(false),
            b_p_ai_nopaste: std::cell::Cell::new(false),
            b_p_bkc: crate::option_vars::OptString::new(),
            b_bkc_flags: std::cell::Cell::new(0),
            b_p_ci: std::cell::Cell::new(false),
            b_p_bin: std::cell::Cell::new(false),
            b_p_bomb: std::cell::Cell::new(false),
            b_p_bh: crate::option_vars::OptString::new(),
            b_p_bt: crate::option_vars::OptString::new(),
            b_p_busy: std::cell::Cell::new(0),
            b_has_qf_entry: std::cell::Cell::new(0),
            b_p_bl: std::cell::Cell::new(false),
            b_p_channel: std::cell::Cell::new(0),
            b_p_cin: std::cell::Cell::new(false),
            b_p_cino: crate::option_vars::OptString::new(),
            b_p_cink: crate::option_vars::OptString::new(),
            b_p_cinw: crate::option_vars::OptString::new(),
            b_p_cinsd: crate::option_vars::OptString::new(),
            b_p_com: crate::option_vars::OptString::new(),
            b_p_cms: crate::option_vars::OptString::new(),
            b_p_cot: crate::option_vars::OptString::new(),
            b_cot_flags: std::cell::Cell::new(0),
            b_p_cpt: crate::option_vars::OptString::new(),
            #[cfg(windows)]
            b_p_csl: crate::option_vars::OptString::new(),
            b_p_cpt_flags: std::cell::Cell::new(0),
            b_p_cpt_cb: std::cell::RefCell::new(Vec::new()),
            b_p_cfu: crate::eval::typval::Callback::NONE,
            b_p_ofu: crate::eval::typval::Callback::NONE,
            b_p_tfu: crate::eval::typval::Callback::NONE,
            b_p_ffu: crate::eval::typval::Callback::NONE,
            b_p_eof: std::cell::Cell::new(false),
            b_p_eol: std::cell::Cell::new(false),
            b_p_fixeol: std::cell::Cell::new(false),
            b_p_et: std::cell::Cell::new(false),
            b_p_et_nobin: std::cell::Cell::new(false),
            b_p_et_nopaste: std::cell::Cell::new(false),
            b_p_fenc: crate::option_vars::OptString::new(),
            b_p_ff: crate::option_vars::OptString::new(),
            b_p_ft: crate::option_vars::OptString::new(),
            b_p_follow: std::cell::Cell::new(false),
            b_p_fo: crate::option_vars::OptString::new(),
            b_p_flp: crate::option_vars::OptString::new(),
            b_p_inf: std::cell::Cell::new(false),
            b_p_isk: crate::option_vars::OptString::new(),
            b_p_def: crate::option_vars::OptString::new(),
            b_p_inc: crate::option_vars::OptString::new(),
            b_p_inex: crate::eval::typval::Callback::NONE,
            b_p_inex_flags: std::cell::Cell::new(0),
            b_p_inde: crate::eval::typval::Callback::NONE,
            b_p_inde_flags: std::cell::Cell::new(0),
            b_p_indk: crate::option_vars::OptString::new(),
            b_p_fp: crate::option_vars::OptString::new(),
            b_p_fex: crate::eval::typval::Callback::NONE,
            b_p_fex_flags: std::cell::Cell::new(0),
            b_p_fs: std::cell::Cell::new(Some(false)),
            b_p_kp: crate::option_vars::OptString::new(),
            b_p_lisp: std::cell::Cell::new(false),
            b_p_lop: crate::option_vars::OptString::new(),
            b_p_menc: crate::option_vars::OptString::new(),
            b_p_mps: crate::option_vars::OptString::new(),
            b_p_ml: std::cell::Cell::new(false),
            b_p_ml_nobin: std::cell::Cell::new(false),
            b_p_ma: std::cell::Cell::new(false),
            b_p_nf: crate::option_vars::OptString::new(),
            b_p_pi: std::cell::Cell::new(false),
            b_p_qe: crate::option_vars::OptString::new(),
            b_p_ro: std::cell::Cell::new(false),
            b_p_sw: std::cell::Cell::new(0),
            b_p_scbk: std::cell::Cell::new(0),
            b_p_si: std::cell::Cell::new(false),
            b_p_sts: std::cell::Cell::new(0),
            b_p_sts_nopaste: std::cell::Cell::new(0),
            b_p_sua: crate::option_vars::OptString::new(),
            b_p_swf: std::cell::Cell::new(false),
            b_p_smc: std::cell::Cell::new(0),
            b_p_syn: crate::option_vars::OptString::new(),
            b_p_ts: std::cell::Cell::new(0),
            b_p_tw: std::cell::Cell::new(0),
            b_p_tw_nobin: std::cell::Cell::new(0),
            b_p_tw_nopaste: std::cell::Cell::new(0),
            b_p_wm: std::cell::Cell::new(0),
            b_p_wm_nobin: std::cell::Cell::new(0),
            b_p_wm_nopaste: std::cell::Cell::new(0),
            b_p_vsts: crate::option_vars::OptString::new(),
            b_p_vsts_array: std::cell::RefCell::new(Vec::new()),
            b_p_vsts_nopaste: crate::globals::CloneCell::new(None),
            b_p_vts: crate::option_vars::OptString::new(),
            b_p_vts_array: std::cell::RefCell::new(Vec::new()),
            b_p_keymap: crate::option_vars::OptString::new(),
            b_p_gefm: crate::option_vars::OptString::new(),
            b_p_gp: crate::option_vars::OptString::new(),
            b_p_mp: crate::option_vars::OptString::new(),
            b_p_efm: crate::option_vars::OptString::new(),
            b_p_ep: crate::option_vars::OptString::new(),
            b_p_path: crate::option_vars::OptString::new(),
            b_p_ar: std::cell::Cell::new(Some(false)),
            b_p_tags: crate::option_vars::OptString::new(),
            b_p_tc: crate::option_vars::OptString::new(),
            b_tc_flags: std::cell::Cell::new(0),
            b_p_dict: crate::option_vars::OptString::new(),
            b_p_dia: crate::option_vars::OptString::new(),
            b_p_tsr: crate::option_vars::OptString::new(),
            b_p_tsrfu: crate::eval::typval::Callback::NONE,
            b_p_ul: std::cell::Cell::new(0),
            b_p_udf: std::cell::Cell::new(false),
            b_p_lw: crate::option_vars::OptString::new(),
            b_ind_level: std::cell::Cell::new(0),
            b_ind_open_imag: std::cell::Cell::new(0),
            b_ind_no_brace: std::cell::Cell::new(0),
            b_ind_first_open: std::cell::Cell::new(0),
            b_ind_open_extra: std::cell::Cell::new(0),
            b_ind_close_extra: std::cell::Cell::new(0),
            b_ind_open_left_imag: std::cell::Cell::new(0),
            b_ind_jump_label: std::cell::Cell::new(0),
            b_ind_case: std::cell::Cell::new(0),
            b_ind_case_code: std::cell::Cell::new(0),
            b_ind_case_break: std::cell::Cell::new(0),
            b_ind_param: std::cell::Cell::new(0),
            b_ind_func_type: std::cell::Cell::new(0),
            b_ind_comment: std::cell::Cell::new(0),
            b_ind_in_comment: std::cell::Cell::new(0),
            b_ind_in_comment2: std::cell::Cell::new(0),
            b_ind_cpp_baseclass: std::cell::Cell::new(0),
            b_ind_continuation: std::cell::Cell::new(0),
            b_ind_unclosed: std::cell::Cell::new(0),
            b_ind_unclosed2: std::cell::Cell::new(0),
            b_ind_unclosed_noignore: std::cell::Cell::new(0),
            b_ind_unclosed_wrapped: std::cell::Cell::new(0),
            b_ind_unclosed_whiteok: std::cell::Cell::new(0),
            b_ind_matching_paren: std::cell::Cell::new(0),
            b_ind_paren_prev: std::cell::Cell::new(0),
            b_ind_maxparen: std::cell::Cell::new(0),
            b_ind_maxcomment: std::cell::Cell::new(0),
            b_ind_scopedecl: std::cell::Cell::new(0),
            b_ind_scopedecl_code: std::cell::Cell::new(0),
            b_ind_java: std::cell::Cell::new(0),
            b_ind_js: std::cell::Cell::new(0),
            b_ind_keep_case_label: std::cell::Cell::new(0),
            b_ind_hash_comment: std::cell::Cell::new(0),
            b_ind_cpp_namespace: std::cell::Cell::new(0),
            b_ind_if_for_while: std::cell::Cell::new(0),
            b_ind_cpp_extern_c: std::cell::Cell::new(0),
            b_ind_pragma: std::cell::Cell::new(0),
            b_no_eol_lnum: std::cell::Cell::new(0),
            b_start_eof: std::cell::Cell::new(false),
            b_start_eol: std::cell::Cell::new(false),
            b_start_ffc: std::cell::Cell::new(0),
            b_start_fenc: crate::globals::CloneCell::new(None),
            b_bad_char: std::cell::Cell::new(0),
            b_start_bomb: std::cell::Cell::new(false),
            b_bufvar: crate::eval::typval::DictitemId::NULL,
            b_may_swap: std::cell::Cell::new(false),
            b_did_warn: std::cell::Cell::new(false),
            b_help: std::cell::Cell::new(false),
            b_spell: std::cell::Cell::new(false),
            b_prompt_text: crate::globals::CloneCell::new(None),
            b_prompt_callback: crate::eval::typval::Callback::NONE,
            b_prompt_interrupt: crate::eval::typval::Callback::NONE,
            b_prompt_append_new_line: std::cell::Cell::new(false),
            b_prompt_insert: std::cell::Cell::new(0),
            b_prompt_start: std::cell::RefCell::new(Fmark::ZERO),
            b_s: std::rc::Rc::new(Synblock::new()),
            b_signcols: BufSigncols {
                max: std::cell::Cell::new(0),
                last_max: std::cell::Cell::new(0),
                count: [const { std::cell::Cell::new(0) }; 9],
                autom: std::cell::Cell::new(false),
            },
            terminal: std::cell::Cell::new(TermId::NULL),
            additional_data: std::cell::RefCell::new(None),
            b_mapped_ctrl_c: std::cell::Cell::new(0),
            b_marktree: std::cell::RefCell::new(crate::marktree::MarkTree::new()),
            b_extmark_ns: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            b_conceal_providers: std::cell::Cell::new(0),
            b_prev_line_count: std::cell::Cell::new(0),
            update_channels: std::cell::RefCell::new(Vec::new()),
            update_callbacks: std::cell::RefCell::new(Vec::new()),
            update_need_codepoints: std::cell::Cell::new(false),
            deleted_bytes: std::cell::Cell::new(0),
            deleted_bytes2: std::cell::Cell::new(0),
            deleted_codepoints: std::cell::Cell::new(0),
            deleted_codeunits: std::cell::Cell::new(0),
            flush_count: std::cell::Cell::new(0),
            b_localdir: crate::globals::CloneCell::new(None),
            b_prevdir: crate::globals::CloneCell::new(None),
        }
    }

    pub fn new_rc() -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| {
            let mut b = Self::new();
            b.self_weak = weak.clone();
            b
        })
    }

    pub fn alloc() -> crate::globals::BufPtr {
        let rc = std::rc::Rc::new_cyclic(|weak| {
            let mut b = Self::new();
            b.self_weak = weak.clone();
            b.changedtick_di =
                crate::eval::typval::tv_dict_item_alloc_static_key(b"changedtick\0".to_vec());
            b.b_bufvar = crate::eval::typval::tv_dict_item_alloc_static();
            b
        });
        crate::globals::BufPtr::from(rc)
    }

    #[inline]
    pub fn b_vars(&self) -> crate::eval::typval::DictId {
        self.b_bufvar.tv().vval.v_dict()
    }
}

pub const GETF_SETMARK: i32 = 0x01;
pub const GETF_ALT: i32 = 0x02;
pub const GETF_SWITCH: i32 = 0x04;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GetFileResult {
    Error,
    NotWritten,
    SameFile,
    OpenOther,
    Unused,
}

impl GetFileResult {
    pub const fn is_success(self) -> bool {
        matches!(self, Self::SameFile | Self::OpenOther)
    }
}

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

use crate::charset::NL;
use crate::search::FORWARD;

#[repr(C)]
struct Bufmatch {
    buf: crate::globals::BufPtr,
    match_: std::ffi::CString,
}

const NO_POSITION: Fmark = Fmark {
    mark: CursorPos {
        lnum: 1,
        col: 0,
        coladd: 0,
    },
    fnum: 0,
    timestamp: 0,
    view: INIT_FMARKV,
    additional_data: None,
};

struct BufferModuleState {
    buf_free_count: std::cell::Cell<i32>,
    top_file_num: std::cell::Cell<i32>,
    modelines_entered: std::cell::Cell<i32>,
}

impl BufferModuleState {
    const fn new() -> Self {
        Self {
            buf_free_count: std::cell::Cell::new(0),
            top_file_num: std::cell::Cell::new(1),
            modelines_entered: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static BUF_STATE: BufferModuleState = const { BufferModuleState::new() };
    static LASTTITLE: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
    static LASTICON: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
}

const E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR: &CStr =
    c"E937: Attempt to delete a buffer that is in use: %s";

#[inline]
fn buf_is_changed(buf: &FileBuf) -> bool {
    !bt_dontwrite(Some(buf)) && (buf.b_changed.get() != 0 || file_ff_differs(buf, true))
}

#[inline]
fn curbuf_is_changed() -> bool {
    buf_is_changed(&curbuf_mut())
}

#[inline]
pub fn buf_get_changedtick(buf: &FileBuf) -> Varnumber {
    buf.changedtick_di.tv().vval.v_number()
}

#[inline]
pub fn buf_meta_total(b: &FileBuf, m: usize) -> u32 {
    b.b_marktree.borrow().meta_root[m]
}

#[inline]
pub fn first_win_in_tab(tp: &TabPageInfo) -> crate::globals::WinPtr {
    if g_st().curtab.get() == tp {
        g_st().firstwin.get()
    } else {
        tp.tp_firstwin.get()
    }
}

#[inline]
pub fn modifiable(buf: &FileBuf) -> bool {
    buf.b_p_ma.get() && buf.b_ro_locked.get() == 0
}

fn trigger_undo_ftplugin(buf: &FileBuf, win: &WinInfoInfo) {
    window_layout_lock();
    buf.b_locked.set(buf.b_locked.get() + (1));
    win.w_locked.set(win.w_locked.get() + (1));
    do_cmdline_cmd(c"if exists('b:undo_ftplugin') | exe b:undo_ftplugin | endif");
    buf.b_locked.set(buf.b_locked.get() - (1));
    win.w_locked.set(win.w_locked.get() - (1));
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
    BUF_STATE.with(|s| s.top_file_num.get()) - 1
}

/// Read data from buffer for retrying.
fn read_buffer(read_stdin: bool, eap: Option<&Exarg>, flags: i32) -> bool {
    let silent = shortmess(K_SHM_FILEINFO);

    let mut line_count = curbuf_mut().b_ml.ml_line_count.get();
    let ffname_opt = (!read_stdin).then(|| curbuf_mut().ffname_opt()).flatten();
    let fname_opt = (!read_stdin).then(|| curbuf_mut().fname_opt()).flatten();
    let mut retval = readfile(
        ffname_opt.as_deref(),
        fname_opt.as_deref(),
        line_count,
        0,
        MAXLNUM,
        eap,
        flags | READ_BUFFER,
        silent,
    ) == Some(true);
    if retval {
        while line_count > 0 {
            line_count -= 1;
            ml_delete(1);
        }
    } else {
        while curbuf_mut().b_ml.ml_line_count.get() > line_count {
            ml_delete(line_count);
        }
    }
    curwin_mut().w_cursor.set_lnum(1);
    curwin_mut().w_cursor.set_col(0);

    if read_stdin {
        if !g_st().readonlymode.get() && !buf_is_empty(&curbuf_mut()) {
            changed(&crate::globals::curbuf_mut());
        } else if retval {
            unchanged(&curbuf_mut(), false, true);
        }

        apply_autocmds_retval(
            EVENT_STDINREADPOST,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
            &mut retval,
        );
    }
    retval
}

/// Ensure buffer `buf` is loaded.
pub fn buf_ensure_loaded(buf: &FileBuf) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_some() {
        return true;
    }

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, Some(buf));
    let mut aco: CtxSwitch = CtxSwitch::new();
    ctx_switch(
        &mut aco,
        None,
        None,
        Some(crate::globals::BufPtr::from(buf).borrow_mut()),
        0,
    );
    let status = open_buffer(false, None, 0);
    ctx_restore(&mut aco);
    status && bufref_valid(Some(&bufref)) && buf.b_ml.ml_mfp.borrow().is_some()
}

/// Open current buffer, that is: open the memfile and read the file into
/// memory.
pub fn open_buffer(read_stdin: bool, eap: Option<&Exarg>, flags_arg: i32) -> bool {
    let mut flags = flags_arg;
    let mut retval = Some(true);
    let mut old_curbuf: Bufref = Bufref::new();
    let old_tw = curbuf_mut().b_p_tw.get();
    #[cfg(unix)]
    let mut read_fifo = false;
    #[cfg(not(unix))]
    let read_fifo = false;
    let silent = shortmess(K_SHM_FILEINFO);

    if g_st().readonlymode.get()
        && curbuf_mut().b_ffname.get().is_some()
        && (curbuf_mut().b_flags.get() & BF_NEVERLOADED) != 0
    {
        curbuf_mut().b_p_ro.set(true);
    }

    if !ml_open(&curbuf_mut()) {
        close_buffer(
            curwin_ptr().opt_mut(),
            g_st().curbuf.get().opt_mut(),
            0,
            false,
            false,
            false,
        );

        g_st().curbuf.set(crate::globals::BufPtr::NULL);
        for buf in for_all_buffers_ptr() {
            if buf.b_ml.ml_mfp.borrow().is_some() {
                g_st().curbuf.set(buf);
                break;
            }
        }

        if g_st().curbuf.get().is_null() {
            emsg(gettext(c"E82: Cannot allocate any buffer, exiting..."));
            g_st().v_dying.set(2);
            getout(2);
        }

        emsg(gettext(c"E83: Cannot allocate buffer, using other one..."));
        enter_buffer(g_st().curbuf.get().borrow_mut());
        if old_tw != curbuf_mut().b_p_tw.get() {
            check_colorcolumn(None::<&CStr>, Some(&crate::globals::curwin_mut()));
        }
        return false;
    }

    if let Some(mfp) = curbuf_mut().b_ml.ml_mfp.borrow_mut().as_deref_mut() {
        mfp.mf_dirty = MF_DIRTY_YES_NOSYNC;
    }

    set_bufref(&mut old_curbuf, g_st().curbuf.get().opt_mut());
    curbuf_mut().b_modified_was_set.set(false);

    curwin_mut().w_valid.set(0);

    if bt_nofileread(Some(&crate::globals::curbuf_mut())) {
        flags |= READ_NOFILE;
    }

    if curbuf_mut().b_ffname.get().is_some() {
        #[cfg(unix)]
        let save_bin = curbuf_mut().b_p_bin.get();
        #[cfg(unix)]
        {
            const S_IFMT: i32 = 0o170000;
            const S_IFIFO: i32 = 0o010000;
            const S_IFSOCK: i32 = 0o140000;
            let perm = os_getperm(curbuf_mut().ffname_cstr());
            if perm >= 0 {
                let fmt = perm & S_IFMT;
                #[cfg(any(
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                    target_os = "dragonfly",
                    target_os = "macos"
                ))]
                let is_chr_fd = fmt == 0o020000 && is_dev_fd_file(curbuf_mut().ffname_bytes());
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
                curbuf_mut().b_p_bin.set(true);
            }
        }

        let ffname_opt = curbuf_mut().ffname_opt();
        let fname_opt = curbuf_mut().fname_opt();
        retval = readfile(
            ffname_opt.as_deref(),
            fname_opt.as_deref(),
            0,
            0,
            MAXLNUM,
            eap,
            flags | READ_NEW | if read_fifo { READ_FIFO } else { 0 },
            silent,
        );
        #[cfg(unix)]
        if read_fifo {
            curbuf_mut().b_p_bin.set(save_bin);
            if retval == Some(true) {
                retval = Some(read_buffer(false, eap, flags));
            }
        }

        if bt_help(Some(&crate::globals::curbuf_mut())) {
            get_local_additions();
        }
    } else if read_stdin {
        let save_bin = curbuf_mut().b_p_bin.get();

        curbuf_mut().b_p_bin.set(true);
        retval = readfile(
            None,
            None,
            0,
            0,
            MAXLNUM,
            None,
            flags | (READ_NEW + READ_STDIN),
            silent,
        );
        curbuf_mut().b_p_bin.set(save_bin);
        if retval == Some(true) {
            retval = Some(read_buffer(
                true,
                (eap.is_some()).then(|| eap.unwrap()),
                flags,
            ));
        }
    }

    if let Some(mfp) = curbuf_mut().b_ml.ml_mfp.borrow_mut().as_deref_mut()
        && mfp.mf_dirty == MF_DIRTY_YES_NOSYNC
    {
        mfp.mf_dirty = MF_DIRTY_YES;
    }

    if (curbuf_mut().b_flags.get() & BF_NEVERLOADED) != 0 {
        buf_init_isk_chartab(&curbuf_mut());
        parse_cino(&curbuf_mut());
    }

    if (g_st().got_int.get()
        && crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_INTMOD))
        || curbuf_mut().b_modified_was_set.get()
        || (aborting() && crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_INTMOD))
    {
        changed(&crate::globals::curbuf_mut());
    } else if retval != Some(false) && !read_stdin && !read_fifo {
        unchanged(&curbuf_mut(), false, true);
    }
    save_file_ff(&curbuf_mut());

    let tick = buf_get_changedtick(&curbuf_mut());
    curbuf_mut().b_last_changedtick.set(tick);
    curbuf_mut().b_last_changedtick_i.set(tick);
    curbuf_mut().b_last_changedtick_pum.set(tick);

    if aborting() {
        curbuf_mut()
            .b_flags
            .set(curbuf_mut().b_flags.get() | (BF_READERR));
    }

    if retval.is_none()
        && curbuf_mut().b_p_ft.is_empty()
        && curbuf_mut().b_ffname.get().is_some()
        && after_pathsep(&curbuf_mut().ffname_bytes()) != 0
        && augroup_exists(Some(c"filetypedetect"))
    {
        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, g_st().curbuf.get().opt_mut());
        do_doautocmd(c"filetypedetect BufRead", false, None);
        if !bufref_valid(Some(&bufref)) || old_curbuf.br_buf != g_st().curbuf.get() || aborting() {
            return false;
        }
    }

    fold_update_all(&curwin_mut());

    if (curwin_mut().w_valid.get() & VALID_TOPLINE) == 0 {
        curwin_mut().w_topline.set(1);
        curwin_mut().w_topfill.set(0);
    }
    let mut ok_retval = retval != Some(false);
    apply_autocmds_retval(
        EVENT_BUFENTER,
        None,
        None,
        false,
        curbuf_opt_mut().as_deref(),
        &mut ok_retval,
    );

    if !ok_retval {
        return false;
    }

    if bufref_valid(Some(&old_curbuf))
        && old_curbuf
            .buf_opt()
            .is_some_and(|b| b.b_ml.ml_mfp.borrow().is_some())
    {
        let mut aco: CtxSwitch = CtxSwitch::new();

        ctx_switch(&mut aco, None, None, old_curbuf.buf_opt(), 0);
        do_modelines(0);
        curbuf_mut()
            .b_flags
            .set(curbuf_mut().b_flags.get() & (!(BF_CHECK_RO | BF_NEVERLOADED)));

        if (flags & READ_NOWINENTER) == 0 {
            apply_autocmds_retval(
                EVENT_BUFWINENTER,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
                &mut ok_retval,
            );
        }

        ctx_restore(&mut aco);
    }

    ok_retval
}

/// Store `buf` in `bufref` and set the free count.
pub fn set_bufref(bufref: &mut Bufref, buf: Option<&FileBuf>) {
    bufref.br_buf = buf.map_or(crate::globals::BufPtr::NULL, crate::globals::BufPtr::from);
    bufref.br_fnum = bufref.buf_opt().map_or(0, |b| b.b_fnum.get());
    bufref.br_buf_free_count = BUF_STATE.with(|s| s.buf_free_count.get());
}

/// Return true if `bufref->br_buf` points to the same buffer as when
/// `set_bufref()` was called and it is a valid buffer.
pub fn bufref_valid(bufref: Option<&Bufref>) -> bool {
    let bufref = bufref.unwrap();
    if bufref.br_buf_free_count == BUF_STATE.with(|s| s.buf_free_count.get()) {
        true
    } else {
        buf_valid_handle(bufref.br_buf.clone())
            && bufref
                .buf_opt()
                .is_some_and(|b| bufref.br_fnum == b.b_fnum.get())
    }
}

fn buf_valid_handle(buf: crate::globals::BufPtr) -> bool {
    if buf.is_null() {
        return false;
    }
    let mut bp = g_st().lastbuf.get();
    while let Some(b) = bp.opt() {
        if bp == buf {
            return true;
        }
        bp = b.b_prev.get();
    }
    false
}

/// Check that `buf` points to a valid buffer in the buffer list.
pub fn buf_valid(buf: Option<&FileBuf>) -> bool {
    buf_valid_handle(crate::globals::BufPtr::from(buf))
}

fn can_unload_buffer(buf: &FileBuf) -> bool {
    let mut can_unload = buf.b_locked.get() == 0;

    if can_unload && g_st().updating_screen.get() {
        for wp in for_all_windows_mut() {
            if wp.w_buffer.get() == buf {
                can_unload = false;
                break;
            }
        }
    }
    if can_unload && buf.b_saving.get() {
        can_unload = false;
    }

    if !can_unload {
        let fname = buf.fname_opt().or_else(|| buf.ffname_opt());
        semsg!(
            gettext(E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR),
            fname.as_deref().unwrap_or(c"[No Name]"),
        );
    }
    can_unload
}

pub fn buf_close_terminal(buf: &FileBuf) {
    debug_assert!(!buf.terminal.get().is_null());
    buf.b_locked.set(buf.b_locked.get() + (1));
    terminal_close(buf.terminal_mut(), -1);
    buf.b_locked.set(buf.b_locked.get() - (1));
}

/// Close the link to a buffer.
pub fn close_buffer(
    win: Option<&WinInfoInfo>,
    buf: Option<&FileBuf>,
    action: i32,
    abort_if_last: bool,
    ignore_abort: bool,
    set_context: bool,
) -> bool {
    let win = crate::globals::WinPtr::from(win);
    let buf = crate::globals::BufPtr::from(buf);
    let mut unload_buf = action != 0;
    let mut del_buf = action == DOBUF_DEL || action == DOBUF_WIPE;
    let mut wipe_buf = action == DOBUF_WIPE;

    let is_curwin = !(curwin_ptr()).is_null()
        && win == crate::globals::curwin_handle()
        && curwin_mut().w_buffer.get() == buf.as_ptr();
    let the_curtab = crate::globals::curtab_handle();

    if buf.borrow().terminal.get().is_null() {
        let bh0 = buf.borrow().b_p_bh.first_byte();
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

    if !buf.borrow().terminal.get().is_null() && (unload_buf || del_buf || wipe_buf) {
        unload_buf = true;
        del_buf = true;
        wipe_buf = true;
    }

    if (del_buf || wipe_buf) && !can_unload_buffer(buf.borrow_mut()) {
        return false;
    }

    let mut win_valid = win_valid_any_tab(win.opt());
    if set_context && win_valid && win.borrow().w_buffer.get() == buf.as_ptr() {
        if buf.borrow().b_nwindows.get() == 1 {
            set_last_cursor(win.borrow_mut());
        }
        let cursor = win.borrow().w_cursor.get();
        buflist_setfpos(
            buf.borrow_mut(),
            win.opt_mut(),
            if cursor.lnum == 1 { 0 } else { cursor.lnum },
            cursor.col,
            true,
        );
    }

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, buf.opt_mut());

    if win_valid
        && win.borrow().w_buffer.get() == buf.as_ptr()
        && buf.borrow().b_nwindows.get() == 1
    {
        buf.borrow_mut()
            .b_locked
            .set(buf.borrow_mut().b_locked.get() + (1));
        buf.borrow_mut()
            .b_locked_split
            .set(buf.borrow_mut().b_locked_split.get() + (1));
        let b_fname = buf.borrow().fname_opt();
        if apply_autocmds_win(
            EVENT_BUFWINLEAVE,
            b_fname.as_deref(),
            b_fname.as_deref(),
            false,
            buf.opt_mut(),
            win.opt(),
        ) && !bufref_valid(Some(&bufref))
        {
            emsg(gettext(E_AUABORT));
            return false;
        }
        buf.borrow_mut()
            .b_locked
            .set(buf.borrow_mut().b_locked.get() - (1));
        buf.borrow_mut()
            .b_locked_split
            .set(buf.borrow_mut().b_locked_split.get() - (1));
        if abort_if_last && one_window(win.borrow_mut(), None) {
            emsg(gettext(E_AUABORT));
            return false;
        }

        if !unload_buf {
            buf.borrow_mut()
                .b_locked
                .set(buf.borrow_mut().b_locked.get() + (1));
            buf.borrow_mut()
                .b_locked_split
                .set(buf.borrow_mut().b_locked_split.get() + (1));
            let b_fname = buf.borrow().fname_opt();
            if apply_autocmds(
                EVENT_BUFHIDDEN,
                b_fname.as_deref(),
                b_fname.as_deref(),
                false,
                buf.opt_mut(),
            ) && !bufref_valid(Some(&bufref))
            {
                emsg(gettext(E_AUABORT));
                return false;
            }
            buf.borrow_mut()
                .b_locked
                .set(buf.borrow_mut().b_locked.get() - (1));
            buf.borrow_mut()
                .b_locked_split
                .set(buf.borrow_mut().b_locked_split.get() - (1));
            if abort_if_last && one_window(win.borrow_mut(), None) {
                emsg(gettext(E_AUABORT));
                return false;
            }
        }
        if !ignore_abort && aborting() {
            return false;
        }
        win_valid = win_valid && win_valid_any_tab(win.opt());
    }

    if is_curwin && win != curwin_handle() && win_valid {
        block_autocmds();
        goto_tabpage_win(the_curtab.borrow(), win.borrow_mut());
        unblock_autocmds();
    }

    if bt_quickfix(Some(buf.borrow())) && win_valid && win.borrow().w_buffer.get() == buf.as_ptr() {
        win.borrow_mut().w_onebuf_opt.wo_wfh.set(false);
    }

    let hiding_buf = buf.borrow().b_nwindows.get() <= 0
        || (win_valid
            && win.borrow().w_buffer.get() == buf.as_ptr()
            && buf.borrow().b_nwindows.get() == 1);

    if diffopt_hiddenoff() && !unload_buf && hiding_buf {
        diff_buf_delete(buf.borrow_mut());
    }

    if !hiding_buf || !unload_buf {
        return false;
    }

    if buf.borrow().b_ffname.get().is_none() {
        del_buf = true;
    }

    if !buf_freeall(
        buf.borrow_mut(),
        (if del_buf { BFA_DEL } else { 0 })
            + (if wipe_buf { BFA_WIPE } else { 0 })
            + (if ignore_abort { BFA_IGNORE_ABORT } else { 0 }),
    ) {
        return false;
    }

    let mut clear_w_buf = false;
    win_valid = win_valid && win_valid_any_tab(win.opt());
    if win_valid && win.borrow().w_buffer.get() == buf.as_ptr() {
        buf.borrow_mut()
            .b_nwindows
            .set(buf.borrow_mut().b_nwindows.get() - (1));
        clear_w_buf = true;
    }

    if wipe_buf
        && buf.borrow().b_nwindows.get() <= 0
        && (!buf.borrow().b_prev.get().is_null()
            || !buf.borrow().b_next.get().is_null()
            || entered_free_all_mem())
    {
        if clear_w_buf {
            win.borrow_mut().w_buffer.set(crate::globals::BufPtr::NULL);
        }
        let b_fnum = buf.borrow().b_fnum.get();
        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_mut(&tp) {
                mark_forget_file(&wp, b_fnum);
            }
        }
        buf.borrow_mut().clear_fnames();
        let b_prev = buf.borrow().b_prev.get();
        let b_next = buf.borrow().b_next.get();
        if let Some(prev) = b_prev.opt_mut() {
            prev.b_next.set(b_next.clone());
        } else {
            g_st().firstbuf.set(b_next.clone());
        }
        if let Some(next) = b_next.opt_mut() {
            next.b_prev.set(b_prev);
        } else {
            g_st().lastbuf.set(b_prev);
        }
        free_buffer(buf.borrow_mut());
    } else {
        if del_buf {
            free_buffer_stuff(
                buf.borrow_mut(),
                K_BFF_CLEAR_WIN_INFO | K_BFF_INIT_CHANGEDTICK,
            );
            buf.borrow_mut().b_flags.set(BF_CHECK_RO | BF_NEVERLOADED);
            buf.borrow_mut().b_p_initialized.set(false);
        }
        g_st().textlock.set(g_st().textlock.get() + (1));
        buf_clear_file(buf.borrow_mut());
        g_st().textlock.set(g_st().textlock.get() - (1));
        if clear_w_buf {
            win.borrow_mut().w_buffer.set(crate::globals::BufPtr::NULL);
        }
        if del_buf {
            buf.borrow_mut().b_p_bl.set(false);
        }
    }

    debug_assert!(!win_valid || !is_curwin || win == curwin_handle());
    true
}

/// Make buffer not contain a file.
pub fn buf_clear_file(buf: &FileBuf) {
    buf.b_ml.ml_line_count.set(1);
    unchanged(buf, true, true);
    buf.b_p_eof.set(false);
    buf.b_start_eof.set(false);
    buf.b_p_eol.set(true);
    buf.b_start_eol.set(true);
    buf.b_p_bomb.set(false);
    buf.b_start_bomb.set(false);
    *buf.b_ml.ml_mfp.borrow_mut() = None;
    buf.b_ml.ml_flags.set(ML_EMPTY);
}

/// Clears the current buffer contents.
pub fn buf_clear() {
    let line_count = curbuf_mut().b_ml.ml_line_count.get();
    extmark_free_all(&curbuf_mut());
    mc_buf_free(&curbuf_mut());
    while (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) == 0 {
        ml_delete(1);
    }
    deleted_lines_mark(1, line_count);
}

/// Free all things allocated for a buffer that are related to the file.
pub fn buf_freeall(buf: &FileBuf, flags: i32) -> bool {
    let is_curbuf = g_st().curbuf.get() == buf;
    let is_curwin = !(curwin_ptr()).is_null()
        && curwin_mut().w_buffer.get() == (std::ptr::from_ref(buf).cast_mut());
    let the_curwin = curwin_ptr();
    let the_curtab = g_st().curtab.get().clone();

    buf.b_locked.set(buf.b_locked.get() + (1));
    buf.b_locked_split.set(buf.b_locked_split.get() + (1));

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, Some(buf));

    if !buf.terminal.get().is_null() {
        buf_close_terminal(buf);
    }

    buf_updates_unload(buf, false);

    let buf_ptr = crate::globals::BufPtr::from(buf);
    if buf_ptr.b_ml.ml_mfp.borrow().is_some() {
        let b_fname = buf_ptr.borrow().fname_opt();
        if apply_autocmds(
            EVENT_BUFUNLOAD,
            b_fname.as_deref(),
            b_fname.as_deref(),
            false,
            buf_ptr.opt_mut(),
        ) && !bufref_valid(Some(&bufref))
        {
            return false;
        }
    }
    if (flags & BFA_DEL) != 0 && buf_ptr.b_p_bl.get() {
        let b_fname = buf_ptr.borrow().fname_opt();
        if apply_autocmds(
            EVENT_BUFDELETE,
            b_fname.as_deref(),
            b_fname.as_deref(),
            false,
            buf_ptr.opt_mut(),
        ) && !bufref_valid(Some(&bufref))
        {
            return false;
        }
    }
    if (flags & BFA_WIPE) != 0 {
        let b_fname = buf_ptr.borrow().fname_opt();
        if apply_autocmds(
            EVENT_BUFWIPEOUT,
            b_fname.as_deref(),
            b_fname.as_deref(),
            false,
            buf_ptr.opt_mut(),
        ) && !bufref_valid(Some(&bufref))
        {
            return false;
        }
    }
    buf_ptr.b_locked.set(buf_ptr.b_locked.get() - (1));
    buf_ptr
        .b_locked_split
        .set(buf_ptr.b_locked_split.get() - (1));

    if is_curwin && curwin_ptr() != the_curwin && win_valid_any_tab(the_curwin.opt()) {
        block_autocmds();
        goto_tabpage_win(the_curtab.borrow(), the_curwin.borrow_mut());
        unblock_autocmds();
    }
    if (flags & BFA_IGNORE_ABORT) == 0 && aborting() {
        return false;
    }

    let buf = buf_ptr.borrow_mut();

    if (std::ptr::from_ref(buf).cast_mut()) == g_st().curbuf.get().as_ptr() && !is_curbuf {
        return false;
    }

    if (std::ptr::from_ref(buf).cast_mut()) == g_st().curbuf.get().as_ptr()
        && g_st().visual_global.borrow().active
        && !entered_free_all_mem()
    {
        end_visual_mode();
    }

    diff_buf_delete(buf);
    if !(curwin_ptr()).is_null()
        && curwin_mut().w_buffer.get() == (std::ptr::from_ref(buf).cast_mut())
    {
        reset_synblock(&curwin_mut());
    }

    for tp in for_all_tabpages_ptr() {
        for win in for_all_windows_in_tab_mut(&tp) {
            if win.w_buffer.get() == (std::ptr::from_ref(buf).cast_mut()) {
                clear_folding(&win);
            }
        }
    }

    if !buf.terminal.get().is_null() {
        block_autocmds();
        buf_close_terminal(buf);
        unblock_autocmds();
    }

    let count = buf.b_ml.ml_line_count.get();
    ml_close(buf, 1);
    buf.b_ml.ml_line_count.set(0);
    mc_buf_clear(buf);

    if bt_nofilename(Some(buf)) && !g_st().exiting.get() {
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
    syntax_clear(&buf.b_s);
    buf.b_flags.set(buf.b_flags.get() & (!BF_READERR));
    true
}

impl FileBuf {
    #[inline]
    pub fn ffname_opt(&self) -> Option<crate::option_vars::RcCStr> {
        self.b_ffname.get()
    }

    #[inline]
    pub fn ffname_cstr(&self) -> crate::option_vars::RcCStr {
        self.ffname_opt()
            .unwrap_or(crate::option_vars::RcCStr::EMPTY)
    }

    #[inline]
    pub fn ffname_bytes(&self) -> crate::option_vars::RcSlice {
        self.ffname_cstr().to_bytes_slice()
    }

    #[inline]
    pub fn sfname_opt(&self) -> Option<crate::option_vars::RcCStr> {
        self.b_sfname.get()
    }

    #[inline]
    pub fn sfname_cstr(&self) -> crate::option_vars::RcCStr {
        self.sfname_opt()
            .unwrap_or(crate::option_vars::RcCStr::EMPTY)
    }

    #[inline]
    pub fn sfname_bytes(&self) -> crate::option_vars::RcSlice {
        self.sfname_cstr().to_bytes_slice()
    }

    #[inline]
    pub fn fname_opt(&self) -> Option<crate::option_vars::RcCStr> {
        match self.b_fname.get() {
            BufFname::None => None,
            BufFname::Short => self.sfname_opt(),
            BufFname::Full => self.ffname_opt(),
        }
    }

    #[inline]
    pub fn fname_cstr(&self) -> crate::option_vars::RcCStr {
        self.fname_opt()
            .unwrap_or(crate::option_vars::RcCStr::EMPTY)
    }

    #[inline]
    pub fn fname_bytes(&self) -> crate::option_vars::RcSlice {
        self.fname_cstr().to_bytes_slice()
    }

    #[inline]
    pub fn set_ffname(&self, ffname: Option<std::ffi::CString>) {
        self.b_ffname
            .set(ffname.map(crate::option_vars::RcCStr::from_cstring));
    }

    #[inline]
    pub fn set_sfname(&self, sfname: Option<std::ffi::CString>) {
        self.b_sfname
            .set(sfname.map(crate::option_vars::RcCStr::from_cstring));
    }

    #[inline]
    pub fn clear_fnames(&self) {
        self.b_ffname.set(None);
        self.b_sfname.set(None);
        self.b_fname.set(BufFname::None);
    }

    #[inline]
    pub fn take_fnames(
        &self,
    ) -> (
        Option<std::ffi::CString>,
        Option<std::ffi::CString>,
        BufFname,
    ) {
        let kind = self.b_fname.replace(BufFname::None);
        (
            self.b_ffname.take().map(|s| s.as_cstr().to_owned()),
            self.b_sfname.take().map(|s| s.as_cstr().to_owned()),
            kind,
        )
    }

    #[inline]
    pub fn restore_fnames(
        &self,
        ffname: Option<std::ffi::CString>,
        sfname: Option<std::ffi::CString>,
        kind: BufFname,
    ) {
        self.set_ffname(ffname);
        self.set_sfname(sfname);
        self.b_fname.set(kind);
    }

    #[inline]
    pub fn swap_fnames(&self, other: &Self) {
        self.b_fname.swap(&other.b_fname);
        self.b_ffname.swap(&other.b_ffname);
        self.b_sfname.swap(&other.b_sfname);
    }
}

impl Drop for FileBuf {
    fn drop(&mut self) {
        self.clear_fnames();
        free_buf_options(&*self, true);
        self.b_start_fenc.set(None);
        self.b_localdir.set(None);
        self.b_prevdir.set(None);
        *self.additional_data.borrow_mut() = None;
        self.b_prompt_text.set(None);
        callback_free(&self.b_prompt_callback);
        callback_free(&self.b_prompt_interrupt);
        clear_fmark(&mut self.b_last_cursor.borrow_mut(), 0);
        clear_fmark(&mut self.b_last_insert.borrow_mut(), 0);
        clear_fmark(&mut self.b_last_change.borrow_mut(), 0);
        clear_fmark(&mut self.b_prompt_start.borrow_mut(), 0);
        self.b_changelistlen.set(0);
        if let Some(cdi) = self.changedtick_di.opt() {
            crate::eval::typval::tv_dict_item_free_slot(cdi);
        }
        if let Some(bv) = self.b_bufvar.opt() {
            crate::eval::typval::tv_dict_item_free_slot(bv);
        }
    }
}
fn free_buffer(buf: &FileBuf) {
    let buf_ptr = crate::globals::BufPtr::from(buf);
    let buf_ref = buf_ptr.borrow_mut();
    g_st()
        .buffer_handles
        .borrow_mut()
        .remove(&buf_ref.b_fnum.get());

    BUF_STATE.with(|s| s.buf_free_count.set(s.buf_free_count.get() + (1)));

    free_buffer_stuff(buf_ref, K_BFF_CLEAR_WIN_INFO);

    if buf_ref.b_vars().dv_refcount.get() > DO_NOT_FREE_CNT {
        let changedtick_copy = tv_dict_item_copy(&buf_ref.changedtick_di);
        tv_dict_add(&buf_ref.b_vars(), &changedtick_copy);
    }

    unref_var_dict(&buf_ref.b_vars());

    aubuflocal_remove(buf_ref);

    if g_st().autocmd_busy.get() {
        *buf_ref.additional_data.borrow_mut() = None;
        buf_ref.b_prompt_text.set(None);
        callback_free(&buf_ref.b_prompt_callback);
        callback_free(&buf_ref.b_prompt_interrupt);
        clear_fmark(&mut buf_ref.b_last_cursor.borrow_mut(), 0);
        clear_fmark(&mut buf_ref.b_last_insert.borrow_mut(), 0);
        clear_fmark(&mut buf_ref.b_last_change.borrow_mut(), 0);
        clear_fmark(&mut buf_ref.b_prompt_start.borrow_mut(), 0);
        buf_ref.b_changelistlen.set(0);
        buf_ref.b_namedm.borrow_mut().fill(Fmark::new());
        buf_ref.b_changelist.borrow_mut().fill(Fmark::new());

        g_st().au_pending_free_buf.borrow_mut().push(buf_ptr);
    } else {
        if g_st().curbuf.get() == buf_ptr {
            g_st().curbuf.set(crate::globals::BufPtr::NULL);
        }
        buf_ref.b_next.set(crate::globals::BufPtr::NULL);
        buf_ref.b_prev.set(crate::globals::BufPtr::NULL);
    }
}

fn clear_wininfo(buf: &FileBuf) {
    buf.b_wininfo.borrow_mut().clear();
}

fn free_buffer_stuff(buf: &FileBuf, free_flags: i32) {
    if (free_flags & K_BFF_CLEAR_WIN_INFO) != 0 {
        clear_wininfo(buf);
        free_buf_options(buf, true);
        buf.b_s.b_langp.borrow_mut().clear();
    }
    {
        let changedtick_idx = hash_find_idx(&buf.b_vars().dv_hashtab, c"changedtick");
        hash_remove(&buf.b_vars().dv_hashtab, changedtick_idx);
    }
    vars_clear(&buf.b_vars().dv_hashtab);
    if (free_flags & K_BFF_INIT_CHANGEDTICK) != 0 {
        buf_init_changedtick(buf);
    }
    buf.b_ucmds.borrow_mut().clear();
    extmark_free_all(buf);
    mc_buf_free(buf);
    buf.b_conceal_providers.set(0);
    map_clear_mode(buf, MAP_ALL_MODES, true, false);
    map_clear_mode(buf, MAP_ALL_MODES, true, true);
    buf.b_start_fenc.set(None);
    buf.b_localdir.set(None);
    buf.b_prevdir.set(None);

    buf_free_callbacks(buf);
}

/// Go to another buffer. Handles the result of the ATTENTION dialog.
pub fn goto_buffer(eap: &mut Exarg, start: i32, dir: i32, count: i32) {
    let save_sea = g_st().swap_exists_action.get();
    let skip_help_buf = matches!(
        eap.cmdidx,
        Cmdidx::CmdBnextVariant
            | Cmdidx::CmdSbnextVariant
            | Cmdidx::CmdBnext
            | Cmdidx::CmdBprevious
            | Cmdidx::CmdSbnext
            | Cmdidx::CmdSbprevious
    );

    let mut old_curbuf: Bufref = Bufref::new();
    set_bufref(&mut old_curbuf, g_st().curbuf.get().opt_mut());

    if g_st().swap_exists_action.get() == SEA_NONE {
        g_st().swap_exists_action.set(SEA_DIALOG);
    }
    let _ = do_buffer_ext(
        if eap.cmd.first_byte() == b's' {
            DOBUF_SPLIT
        } else {
            DOBUF_GOTO
        },
        start,
        dir,
        count,
        (if eap.forceit { DOBUF_FORCEIT } else { 0 })
            | (if skip_help_buf { DOBUF_SKIPHELP } else { 0 }),
    );

    if g_st().swap_exists_action.get() == SEA_QUIT && eap.cmd.first_byte() == b's' {
        let mut cs: ExCleanup = ExCleanup::new();
        enter_cleanup(&mut cs);
        win_close(&curwin_mut(), true, false);
        g_st().swap_exists_action.set(save_sea);
        g_st().swap_exists_did_quit.set(true);
        leave_cleanup(&mut cs);
    } else {
        handle_swap_exists(Some(&mut old_curbuf));
    }
}

/// Handle the situation of `swap_exists_action` being set.
pub fn handle_swap_exists(mut old_curbuf: Option<&mut Bufref>) {
    let mut cs: ExCleanup = ExCleanup::new();
    let old_tw = curbuf_mut().b_p_tw.get();

    if g_st().swap_exists_action.get() == SEA_QUIT {
        enter_cleanup(&mut cs);

        g_st().swap_exists_action.set(SEA_NONE);
        g_st().swap_exists_did_quit.set(true);
        close_buffer(
            curwin_ptr().opt_mut(),
            g_st().curbuf.get().opt_mut(),
            DOBUF_UNLOAD,
            false,
            false,
            true,
        );
        let buf = if old_curbuf.is_none()
            || !bufref_valid((old_curbuf.is_some()).then(|| old_curbuf.as_deref().unwrap()))
            || old_curbuf.as_deref_mut().unwrap().br_buf == g_st().curbuf.get()
        {
            block_autocmds();
            let b = buflist_new(None, None, 1, BLN_CURBUF | BLN_LISTED);
            unblock_autocmds();
            b
        } else {
            let b = old_curbuf.unwrap().br_buf.clone();
            (!b.is_null()).then_some(b)
        };
        if let Some(buf_ref) = buf {
            enter_buffer(&buf_ref);
            if old_tw != curbuf_mut().b_p_tw.get() {
                check_colorcolumn(None::<&CStr>, Some(&crate::globals::curwin_mut()));
            }
        }

        leave_cleanup(&mut cs);
    } else if g_st().swap_exists_action.get() == SEA_RECOVER {
        enter_cleanup(&mut cs);

        g_st().msg_scroll.set(1);
        ml_recover(false);
        msg_puts(c"\n");
        g_st().cmdline_row.set(g_st().msg_row.get());
        do_modelines(0);

        leave_cleanup(&mut cs);
    }
    g_st().swap_exists_action.set(SEA_NONE);
}

/// Delete or unload buffer(s).
pub fn do_bufdel(
    command: i32,
    arg_in: &[u8],
    addr_count: i32,
    start_bnr: i32,
    end_bnr: i32,
    forceit: bool,
) -> Option<std::borrow::Cow<'static, std::ffi::CStr>> {
    let arg_cstr = crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(arg_in));
    let mut arg = arg_cstr.as_c_str();
    let mut do_current = 0;
    let mut deleted: i32 = 0;
    let mut errormsg: Option<std::borrow::Cow<'static, std::ffi::CStr>> = None;
    let mut bnr: i32;

    if addr_count == 0 {
        do_buffer(command, DOBUF_CURRENT, FORWARD, 0, i32::from(forceit));
    } else {
        if addr_count == 2 {
            if !arg.is_empty() {
                return Some(ex_errmsg(E_TRAILING_ARG, arg));
            }
            bnr = start_bnr;
        } else {
            bnr = end_bnr;
        }

        while !g_st().got_int.get() {
            if bnr == curbuf_mut().b_fnum.get() {
                do_current = bnr;
            } else if do_buffer(command, DOBUF_FIRST, FORWARD, bnr, i32::from(forceit)) {
                deleted += 1;
            }

            if addr_count == 2 {
                bnr += 1;
                if bnr > end_bnr {
                    break;
                }
            } else {
                arg = crate::charset::skipwhite(arg);
                let arg_bytes = arg.to_bytes();
                if arg_bytes.is_empty() {
                    break;
                }
                if !ascii_isdigit(arg_bytes[0]) {
                    let pat_len = crate::charset::skiptowhite_esc_len(arg_bytes);
                    bnr =
                        buflist_findpat(&arg_bytes[..pat_len], command == DOBUF_WIPE, false, false);
                    if bnr < 0 {
                        break;
                    }
                    arg = &arg[pat_len..];
                } else {
                    let mut rem = arg_bytes;
                    bnr = getdigits_int(&mut rem, false, 0);
                    arg = &arg[arg_bytes.len() - rem.len()..];
                }
            }
            os_breakcheck();
        }
        if !g_st().got_int.get()
            && do_current != 0
            && do_buffer(
                command,
                DOBUF_FIRST,
                FORWARD,
                do_current,
                i32::from(forceit),
            )
        {
            deleted += 1;
        }

        if deleted == 0 {
            let msg_cstr = if command == DOBUF_UNLOAD {
                gettext(c"E515: No buffers were unloaded")
            } else if command == DOBUF_DEL {
                gettext(c"E516: No buffers were deleted")
            } else {
                gettext(c"E517: No buffers were wiped out")
            };
            errormsg = Some(std::borrow::Cow::Borrowed(msg_cstr));
        } else if i64::from(deleted) >= opt_vars().p_report.get() {
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
    let buf = g_st().curbuf.get();

    if action == DOBUF_UNLOAD {
        emsg(gettext(c"E90: Cannot unload last buffer"));
        return false;
    }

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, buf.opt_mut());

    if close_others {
        let mut can_close_all_others = true;
        if curwin_mut().w_floating.get() {
            can_close_all_others = false;
            for wp in for_all_windows_ptr() {
                if wp.w_floating.get() {
                    break;
                }
                if wp.w_buffer.get() != g_st().curbuf.get() {
                    can_close_all_others = true;
                    break;
                }
            }
        }
        close_windows(buf.opt_mut(), can_close_all_others);
    }

    setpcmark();
    let retval = do_ecmd(
        0,
        None,
        None,
        None,
        ECMD_ONE,
        if forceit != 0 { ECMD_FORCEIT } else { 0 },
        Some(&curwin_mut()),
    );

    if g_st().curbuf.get() != buf
        && bufref_valid(Some(&bufref))
        && bufref.buf_opt().is_some_and(|b| b.b_nwindows.get() == 0)
    {
        close_buffer(None, buf.opt_mut(), action, false, false, false);
    }

    if !close_others {
        g_st().need_fileinfo.set(false);
    } else if retval && !shortmess(K_SHM_FILEINFO) {
        g_st().need_fileinfo.set(true);
    }

    retval
}

fn do_buffer_ext(action: i32, start: i32, dir: i32, mut count: i32, flags: i32) -> bool {
    let mut buf: crate::globals::BufPtr;
    let mut bp: crate::globals::BufPtr;
    let mut update_jumplist = true;
    let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;

    buf = match start {
        DOBUF_FIRST => g_st().firstbuf.get(),
        DOBUF_LAST => g_st().lastbuf.get(),
        _ => g_st().curbuf.get(),
    };
    if start == DOBUF_MOD {
        while count > 0 {
            count -= 1;
            loop {
                buf = buf.b_next.get();

                if buf.is_null() {
                    buf = g_st().firstbuf.get();
                }
                if buf == g_st().curbuf.get() || buf_is_changed(buf.borrow_mut()) {
                    break;
                }
            }
        }
        if !buf_is_changed(buf.borrow_mut()) {
            emsg(gettext(c"E84: No modified buffer found"));
            return false;
        }
    } else if start == DOBUF_FIRST && count != 0 {
        while !buf.is_null() && buf.b_fnum.get() != count {
            buf = buf.b_next.get();
        }
    } else {
        let help_only = (flags & DOBUF_SKIPHELP) != 0 && buf.b_help.get();

        bp = crate::globals::BufPtr::NULL;
        while count > 0
            || (bp != buf
                && !unload
                && !(if help_only {
                    buf.b_help.get()
                } else {
                    buf.b_p_bl.get()
                }))
        {
            if bp.is_null() {
                bp = buf.clone();
            }
            buf = if dir == FORWARD {
                if !buf.b_next.get().is_null() {
                    buf.b_next.get()
                } else {
                    g_st().firstbuf.get()
                }
            } else if !buf.b_prev.get().is_null() {
                buf.b_prev.get()
            } else {
                g_st().lastbuf.get()
            };
            if unload
                || (if help_only {
                    buf.b_help.get()
                } else {
                    let b = buf.borrow();
                    b.b_p_bl.get() && ((flags & DOBUF_SKIPHELP) == 0 || !b.b_help.get())
                })
            {
                count -= 1;
                bp = crate::globals::BufPtr::NULL;
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
        && buf != g_st().curbuf.get()
        && !check_can_set_curbuf_forceit((flags & DOBUF_FORCEIT) != 0)
    {
        return false;
    }

    if (action == DOBUF_GOTO || action == DOBUF_SPLIT) && (buf.b_flags.get() & BF_DUMMY) != 0 {
        semsg!(gettext(E_NOBUFNR), count);
        return false;
    }

    if unload {
        let mut bufref: Bufref = Bufref::new();
        if !can_unload_buffer(buf.borrow_mut()) {
            return false;
        }
        set_bufref(&mut bufref, buf.opt_mut());

        if action != DOBUF_WIPE && buf.b_ml.ml_mfp.borrow().is_none() && !buf.b_p_bl.get() {
            return false;
        }

        if (flags & DOBUF_FORCEIT) == 0 && buf_is_changed(buf.borrow_mut()) {
            if (opt_vars().p_confirm.get()
                || (g_st().cmdmod.borrow().cmod_flags & CMOD_CONFIRM) != 0)
                && opt_vars().p_write.get()
            {
                dialog_changed(buf.borrow_mut(), false);

                if !bufref_valid(Some(&bufref)) {
                    return false;
                }
                if buf_is_changed(buf.borrow_mut()) {
                    return false;
                }
            } else {
                semsg!(
                    gettext(E_NO_WRITE_SINCE_LAST_CHANGE_FOR_BUFFER_NR_ADD_BANG_TO_OVERRIDE,),
                    buf.b_fnum.get(),
                );

                return false;
            }
        }

        if (flags & DOBUF_FORCEIT) == 0
            && !buf.terminal.get().is_null()
            && terminal_running(buf.terminal.get().borrow())
        {
            if opt_vars().p_confirm.get() || (g_st().cmdmod.borrow().cmod_flags & CMOD_CONFIRM) != 0
            {
                if !dialog_close_terminal(buf.borrow_mut()) {
                    return false;
                }
            } else {
                semsg!(
                    gettext(c"E89: %s will be killed (add ! to override)"),
                    buf.borrow_mut().fname_cstr(),
                );

                return false;
            }
        }

        let buf_fnum = buf.b_fnum.get();

        if buf == g_st().curbuf.get() && g_st().visual_global.borrow().active {
            end_visual_mode();
        }

        bp = crate::globals::BufPtr::NULL;
        for bp2 in for_all_buffers_ptr() {
            if bp2.b_p_bl.get() && buf != bp2 {
                bp = bp2;
                break;
            }
        }
        if bp.is_null() && buf == g_st().curbuf.get() {
            return empty_curbuf(true, flags & DOBUF_FORCEIT, action);
        }

        while buf == g_st().curbuf.get()
            && !(win_locked(&curwin_mut()) != 0 || curwin_mut().buf_mut().b_locked.get() > 0)
            && (is_ctx_win(&lastwin_mut()) || !last_window(&curwin_mut()))
        {
            if !win_close(&curwin_mut(), false, false) {
                break;
            }
        }

        if g_st().curbuf.get() != buf {
            if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_CLEAN) != 0 {
                mark_jumplist_forget_file(&curwin_mut(), buf_fnum);
            }

            close_windows(buf.opt_mut(), false);
            if g_st().curbuf.get() != buf
                && bufref_valid(Some(&bufref))
                && firstwin_mut().w_buffer.get() == buf
                && one_window(&firstwin_mut(), None)
            {
                debug_assert!(curwin_mut().w_floating.get());
                let mut switchwin: CtxSwitch = CtxSwitch::new();
                let rv = ctx_switch(
                    &mut switchwin,
                    Some(&firstwin_mut()),
                    Some(&curtab_mut()),
                    None,
                    K_CTX_NO_DISPLAY,
                );
                debug_assert!(rv);
                do_buffer_ext(action, start, dir, count, flags);
                ctx_restore(&mut switchwin);
            }

            if g_st().curbuf.get() != buf
                && bufref_valid(Some(&bufref))
                && buf.b_nwindows.get() <= 0
            {
                close_buffer(None, buf.opt_mut(), action, false, false, false);
            }
            return true;
        }

        buf = crate::globals::BufPtr::NULL;
        bp = crate::globals::BufPtr::NULL;
        if !g_st().au_new_curbuf.get().br_buf.is_null()
            && bufref_valid(Some(&g_st().au_new_curbuf.get()))
            && g_st().au_new_curbuf.get().br_buf.b_locked_split.get() == 0
        {
            buf = g_st().au_new_curbuf.get().br_buf;
        } else if curwin_mut().w_jumplistlen.get() > 0 {
            if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_CLEAN) != 0 {
                mark_jumplist_forget_file(&curwin_mut(), buf_fnum);
            }

            if curwin_mut().w_jumplistlen.get() > 0 {
                let mut jumpidx = curwin_mut().w_jumplistidx.get();

                if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_CLEAN) != 0 {
                    if jumpidx == curwin_mut().w_jumplistlen.get() {
                        jumpidx = curwin_mut().w_jumplistlen.get() - 1;
                        curwin_mut().w_jumplistidx.set(jumpidx);
                    }
                } else {
                    jumpidx -= 1;
                    if jumpidx < 0 {
                        jumpidx = curwin_mut().w_jumplistlen.get() - 1;
                    }
                }

                let forward = jumpidx;
                while (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_CLEAN) != 0
                    || jumpidx != curwin_mut().w_jumplistidx.get()
                {
                    buf = crate::globals::BufPtr::from(buflist_findnr(
                        curwin_mut().w_jumplist.borrow()[jumpidx as usize]
                            .fmark
                            .fnum,
                    ));

                    if !buf.is_null() {
                        if buf == g_st().curbuf.get()
                            || !buf.b_p_bl.get()
                            || bt_quickfix(buf.opt())
                            || buf.b_locked_split.get() != 0
                        {
                            buf = crate::globals::BufPtr::NULL;
                        } else if buf.b_ml.ml_mfp.borrow().is_none() {
                            if bp.is_null() {
                                bp = buf;
                            }
                            buf = crate::globals::BufPtr::NULL;
                        }
                    }
                    if !buf.is_null() {
                        if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_CLEAN) != 0 {
                            curwin_mut().w_jumplistidx.set(jumpidx);
                            update_jumplist = false;
                        }
                        break;
                    }
                    if jumpidx == 0
                        && curwin_mut().w_jumplistidx.get() == curwin_mut().w_jumplistlen.get()
                    {
                        break;
                    }
                    jumpidx -= 1;
                    if jumpidx < 0 {
                        jumpidx = curwin_mut().w_jumplistlen.get() - 1;
                    }
                    if jumpidx == forward {
                        break;
                    }
                }
            }
        }

        if buf.is_null() {
            let mut forward = true;
            buf = curbuf_mut().b_next.get();
            loop {
                if buf.is_null() {
                    if !forward {
                        break;
                    }
                    buf = curbuf_mut().b_prev.get();
                    forward = false;
                    continue;
                }
                if buf.b_help.get() == curbuf_mut().b_help.get()
                    && buf.b_p_bl.get()
                    && !bt_quickfix(buf.opt())
                    && buf.b_locked_split.get() == 0
                {
                    if buf.b_ml.ml_mfp.borrow().is_some() {
                        break;
                    }
                    if bp.is_null() {
                        bp = buf.clone();
                    }
                }
                buf = if forward {
                    buf.b_next.get()
                } else {
                    buf.b_prev.get()
                };
            }
        }
        if buf.is_null() {
            buf = bp;
        }
        if buf.is_null() {
            for buf2 in for_all_buffers_ptr() {
                if buf2.b_p_bl.get()
                    && g_st().curbuf.get() != buf2
                    && !bt_quickfix(Some(&buf2))
                    && buf2.b_locked_split.get() == 0
                {
                    buf = buf2;
                    break;
                }
            }
        }
        if buf.is_null() {
            buf = if !curbuf_mut().b_next.get().is_null() {
                curbuf_mut().b_next.get()
            } else {
                curbuf_mut().b_prev.get()
            };
            if bt_quickfix(buf.opt())
                || (!buf.is_null() && buf != g_st().curbuf.get() && buf.b_locked_split.get() != 0)
            {
                buf = crate::globals::BufPtr::NULL;
            }
        }
    }

    if buf.is_null() {
        return empty_curbuf(false, flags & DOBUF_FORCEIT, action);
    }

    if action == DOBUF_SPLIT && swbuf_goto_win_with_buf(buf.opt_mut()).is_some() {
        return true;
    }
    if g_st().curbuf.get() != buf && buf.b_locked_split.get() != 0 {
        emsg(gettext(E_CANNOT_SWITCH_TO_A_CLOSING_BUFFER));
        return false;
    }
    if action == DOBUF_SPLIT && !win_split(0, 0) {
        return false;
    }

    if buf == g_st().curbuf.get() {
        return true;
    }

    if action == DOBUF_GOTO && !can_abandon(&curbuf_mut(), (flags & DOBUF_FORCEIT) != 0) {
        if (opt_vars().p_confirm.get() || (g_st().cmdmod.borrow().cmod_flags & CMOD_CONFIRM) != 0)
            && opt_vars().p_write.get()
        {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, buf.opt_mut());
            dialog_changed(&curbuf_mut(), false);
            if !bufref_valid(Some(&bufref)) {
                return false;
            }
        }
        if buf_is_changed(&curbuf_mut()) {
            no_write_message();
            return false;
        }
    }

    set_curbuf(buf.borrow_mut(), action, update_jumplist);

    if action == DOBUF_SPLIT {
        reset_binding(&curwin_mut());
    }

    if aborting() {
        return false;
    }

    true
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
pub fn set_curbuf(buf_in: &FileBuf, action: i32, update_jumplist: bool) {
    let buf = crate::globals::BufPtr::from(buf_in);
    let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;
    let old_tw = curbuf_mut().b_p_tw.get();

    if update_jumplist {
        setpcmark();
    }

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_KEEPALT) == 0 {
        curwin_mut().w_alt_fnum.set(curbuf_mut().b_fnum.get());
    }
    buflist_altfpos(&curwin_mut());

    g_st().visual_global.borrow_mut().reselect = 0;

    let prevbuf = g_st().curbuf.get();
    let mut newbufref: Bufref = Bufref::new();
    let mut prevbufref: Bufref = Bufref::new();
    set_bufref(&mut prevbufref, prevbuf.opt_mut());
    set_bufref(&mut newbufref, buf.opt_mut());

    if !apply_autocmds(
        EVENT_BUFLEAVE,
        None,
        None,
        false,
        curbuf_opt_mut().as_deref(),
    ) || (bufref_valid(Some(&prevbufref)) && bufref_valid(Some(&newbufref)) && !aborting())
    {
        if curwin_mut().w_buffer.get() == prevbuf {
            reset_synblock(&curwin_mut());
        }
        if unload {
            close_windows(prevbuf.opt_mut(), false);
        }
        if bufref_valid(Some(&prevbufref)) && !aborting() {
            if g_st().curbuf.get() == prevbuf
                && ((g_st().state_global.get() & MODE_INSERT) == 0
                    || curbuf_mut().b_nwindows.get() <= 1)
            {
                u_sync(false);
            }
            close_buffer(
                curwin_ptr().opt_mut(),
                prevbuf.opt_mut(),
                if unload {
                    action
                } else if action == DOBUF_GOTO
                    && prevbufref
                        .buf_opt()
                        .is_some_and(|b| !buf_hide(b) && !buf_is_changed(b))
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

    let valid = buf_valid_handle(buf.clone());
    if (valid && g_st().curbuf.get() != buf && !aborting()) || curwin_mut().w_buffer.get().is_null()
    {
        let target = if valid { buf } else { g_st().lastbuf.get() };
        enter_buffer(target.borrow_mut());
        if old_tw != curbuf_mut().b_p_tw.get() {
            check_colorcolumn(None::<&CStr>, Some(&crate::globals::curwin_mut()));
        }
    }

    if bufref_valid(Some(&prevbufref))
        && let Some(term) = prevbufref.buf_opt().and_then(|b| b.terminal_opt())
    {
        terminal_check_size(term);
    }

    update_cwd(K_CD_CAUSE_BUFFER);
}

/// Enter a new current buffer.
fn enter_buffer(buf_in: &FileBuf) {
    let buf = crate::globals::BufPtr::from(buf_in);
    if g_st().visual_global.borrow().active && !entered_free_all_mem() {
        end_visual_mode();
    }

    if !curwin_mut().w_buffer.get().is_null() {
        curwin_mut()
            .buf_mut()
            .b_nwindows
            .set(curwin_mut().buf_mut().b_nwindows.get() - (1));
    }

    curwin_mut().w_buffer.set(buf.clone());
    g_st().curbuf.set(buf);
    curbuf_mut()
        .b_nwindows
        .set(curbuf_mut().b_nwindows.get() + (1));

    buf_copy_options(&curbuf_mut(), BCO_ENTER | BCO_NOHELP);
    if !curbuf_mut().b_help.get() {
        get_winopts(&curbuf_mut());
    } else {
        clear_folding(&curwin_mut());
    }
    fold_update_all(&curwin_mut());

    if curwin_mut().w_onebuf_opt.wo_diff.get() {
        diff_buf_add(&curbuf_mut());
    }

    curwin_mut().w_s_owned.set(None);

    curwin_mut().w_cursor.set_lnum(1);
    curwin_mut().w_cursor.set_col(0);
    curwin_mut().w_cursor.set_coladd(0);
    curwin_mut().w_set_curswant.set(true);
    curwin_mut().w_topline_was_set.set(false);

    curwin_mut().w_valid.set(0);

    if curbuf_mut().b_ml.ml_mfp.borrow().is_none() {
        if curbuf_mut().b_p_ft.is_empty() {
            curbuf_mut().b_did_filetype.set(false);
        }
        open_buffer(false, None, 0);
    } else {
        if g_st().msg_silent.get() == 0 && !shortmess(K_SHM_FILEINFO) {
            g_st().need_fileinfo.set(true);
        }
        buf_check_timestamp(&curbuf_mut());

        curwin_mut().w_topline.set(1);
        curwin_mut().w_topfill.set(0);
        apply_autocmds(
            EVENT_BUFENTER,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        apply_autocmds(
            EVENT_BUFWINENTER,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
    }

    if curwin_mut().w_cursor.get().lnum == 1 && inindent(0) {
        buflist_getfpos();
    }

    check_arg_idx(&curwin_mut());
    maketitle();
    win_float_update_preview(&curwin_mut());
    if curwin_mut().w_topline.get() == 1 && !curwin_mut().w_topline_was_set.get() {
        scroll_cursor_halfway(&curwin_mut(), false, false);
    }

    do_autochdir();

    if (curbuf_mut().b_kmap_state.get() & KEYMAP_INIT) != 0 {
        keymap_init();
    }
    if !curbuf_mut().b_help.get()
        && curwin_mut().w_onebuf_opt.wo_spell.get()
        && !curwin_mut().w_s_ptr().b_p_spl.is_empty()
    {
        parse_spelllang(&curwin_mut());
    }
    curbuf_mut().b_last_used.set(os_time() as i64);

    if let Some(term) = curbuf_mut().terminal_opt() {
        terminal_check_size(term);
    }

    redraw_later(Some(&crate::globals::curwin_mut()), UPD_NOT_VALID);
}

/// Change to the directory of the current buffer.
pub fn do_autochdir() {
    if opt_vars().p_acd.get()
        && starting_val() == 0
        && curbuf_mut().b_ffname.get().is_some()
        && vim_chdirfile(&curbuf_mut().ffname_cstr(), K_CD_CAUSE_AUTO)
    {
        g_st().last_chdir_reason.set(Some(c"autochdir"));
        shorten_fnames(true);
    }
}

pub fn no_write_message_buf(buf: &FileBuf) {
    if !buf.terminal.get().is_null() && channel_job_running(buf.b_p_channel.get() as u64) {
        emsg(gettext(E_JOB_STILL_RUNNING_ADD_BANG_TO_END_THE_JOB));
    } else {
        semsg!(
            gettext(E_NO_WRITE_SINCE_LAST_CHANGE_FOR_BUFFER_NR_ADD_BANG_TO_OVERRIDE),
            buf.b_fnum.get(),
        );
    }
}

pub fn no_write_message() {
    if !curbuf_mut().terminal.get().is_null()
        && channel_job_running(curbuf_mut().b_p_channel.get() as u64)
    {
        emsg(gettext(E_JOB_STILL_RUNNING_ADD_BANG_TO_END_THE_JOB));
    } else {
        emsg(gettext(E_NO_WRITE_SINCE_LAST_CHANGE_ADD_BANG_TO_OVERRIDE));
    }
}

pub fn no_write_message_nobang(buf: &FileBuf) {
    if !buf.terminal.get().is_null() && channel_job_running(buf.b_p_channel.get() as u64) {
        emsg(gettext(E_JOB_STILL_RUNNING));
    } else {
        emsg(gettext(E_NO_WRITE_SINCE_LAST_CHANGE));
    }
}

#[inline]
fn buf_init_changedtick(buf: &FileBuf) {
    let tick = buf_get_changedtick(buf);
    buf.changedtick_di.set_tv(TypVal {
        v_type: VarType::VarNumber,
        v_lock: VAR_FIXED,
        vval: TypvalVval::Number(tick),
    });
    buf.changedtick_di.di_flags.set(DI_FLAGS_RO | DI_FLAGS_FIX);
    tv_dict_add(&buf.b_vars(), &buf.changedtick_di);
}

/// Add a file name to the buffer list.
pub fn buflist_new(
    ffname_arg: Option<&CStr>,
    sfname_arg: Option<&CStr>,
    lnum: Linenr,
    flags: i32,
) -> Option<crate::globals::BufPtr> {
    let mut buf: crate::globals::BufPtr;

    let expanded = fname_expand(g_st().curbuf.get().opt(), ffname_arg, sfname_arg);

    let mut file_id: FileID = FileID::new();
    let file_id_valid = expanded
        .as_ref()
        .is_some_and(|(_, sf)| os_fileid(sf, &mut file_id));
    if let Some((ref ff_cstr, _)) = expanded
        && (flags & (BLN_DUMMY | BLN_NEW)) == 0
        && let Some(buf_ptr) = buflist_findname_file_id(Some(ff_cstr), &mut file_id, file_id_valid)
    {
        drop(expanded);
        if lnum != 0 {
            let cw = curwin_ptr();
            buflist_setfpos(
                buf_ptr.borrow_mut(),
                if (flags & BLN_NOCURWIN) != 0 {
                    None
                } else {
                    cw.opt_mut()
                },
                lnum,
                0,
                false,
            );
        }
        if (flags & BLN_NOOPT) == 0 {
            buf_copy_options(buf_ptr.borrow_mut(), 0);
        }
        if (flags & BLN_LISTED) != 0 && !buf_ptr.b_p_bl.get() {
            buf_ptr.b_p_bl.set(true);
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, buf_ptr.opt_mut());
            if (flags & BLN_DUMMY) == 0
                && apply_autocmds(EVENT_BUFADD, None, None, false, buf_ptr.opt_mut())
                && !bufref_valid(Some(&bufref))
            {
                return None;
            }
        }
        return Some(buf_ptr);
    }

    buf = crate::globals::BufPtr::NULL;
    if (flags & BLN_CURBUF) != 0 && curbuf_reusable() {
        let mut bufref: Bufref = Bufref::new();

        debug_assert!(!g_st().curbuf.get().is_null());
        buf = g_st().curbuf.get();
        set_bufref(&mut bufref, buf.opt_mut());
        trigger_undo_ftplugin(&curbuf_mut(), &curwin_mut());
        buf_freeall(&curbuf_mut(), BFA_WIPE | BFA_DEL);
        if aborting() {
            drop(expanded);
            return None;
        }
        if !bufref_valid(Some(&bufref)) {
            buf = crate::globals::BufPtr::NULL;
        }
    }
    if g_st().curbuf.get() != buf || g_st().curbuf.get().is_null() {
        buf = FileBuf::alloc();
        init_var_dict(&tv_dict_alloc(), &buf.b_bufvar, VAR_SCOPE);
        buf_init_changedtick(&buf);
    }

    let buf_ref = buf.borrow_mut();
    if let Some((ff_cstr, sf_cstr)) = expanded {
        let mut sf_vec = sf_cstr.into_bytes_with_nul();
        to_slash(&mut sf_vec);
        buf_ref.set_ffname(Some(ff_cstr));
        buf_ref.set_sfname(Some(std::ffi::CString::from_vec_with_nul(sf_vec).unwrap()));
    }

    clear_wininfo(buf_ref);
    buf_ref.b_wininfo.borrow_mut().push(WinInfo::new());

    if buf == g_st().curbuf.get() {
        free_buffer_stuff(buf_ref, K_BFF_INIT_CHANGEDTICK);
        buf_ref.b_p_initialized.set(false);
        buf_copy_options(buf_ref, BCO_ENTER);
        curbuf_mut()
            .b_kmap_state
            .set(curbuf_mut().b_kmap_state.get() | (KEYMAP_INIT));
    } else {
        buf_ref.b_next.set(crate::globals::BufPtr::NULL);
        if g_st().firstbuf.get().is_null() {
            buf_ref.b_prev.set(crate::globals::BufPtr::NULL);
            g_st().firstbuf.set(buf.clone());
        } else {
            g_st().lastbuf.get().b_next.set(buf.clone());
            buf_ref.b_prev.set(g_st().lastbuf.get());
        }
        g_st().lastbuf.set(buf.clone());

        buf_ref.b_fnum.set(BUF_STATE.with(|s| s.top_file_num.get()));
        BUF_STATE.with(|s| s.top_file_num.set(s.top_file_num.get() + (1)));
        g_st()
            .buffer_handles
            .borrow_mut()
            .insert(buf_ref.b_fnum.get(), buf.clone());
        if BUF_STATE.with(|s| s.top_file_num.get()) < 0 {
            emsg(gettext(c"W14: Warning: List of file names overflow"));
            if g_st().emsg_silent.get() == 0 && !g_st().in_assert_fails.get() {
                msg_delay(3001, true);
            }
            BUF_STATE.with(|s| s.top_file_num.set(1));
        }

        buf_copy_options(buf_ref, BCO_ALWAYS);
    }

    {
        let mut wininfo = buf_ref.b_wininfo.borrow_mut();
        let curwin_info = &mut wininfo[0];
        curwin_info.wi_mark = INIT_FMARK;
        curwin_info.wi_mark.mark.lnum = lnum;
        curwin_info.wi_win = crate::globals::curwin_ptr();
    }

    buf_ref.b_fname.set(BufFname::Short);
    if !file_id_valid {
        buf_ref.file_id_valid.set(false);
    } else {
        buf_ref.file_id_valid.set(true);
        buf_ref.file_id.set(file_id);
    }
    buf_ref.b_u_synced.set(true);
    buf_ref.b_flags.set(BF_CHECK_RO | BF_NEVERLOADED);
    if (flags & BLN_DUMMY) != 0 {
        buf_ref.b_flags.set(buf_ref.b_flags.get() | (BF_DUMMY));
    }
    buf_clear_file(buf_ref);
    clrallmarks(buf_ref, 0);
    fmarks_check_names(buf_ref);
    buf_ref.b_p_bl.set((flags & BLN_LISTED) != 0);
    buf_ref.update_channels.borrow_mut().clear();
    buf_ref.update_callbacks.borrow_mut().clear();
    if (flags & BLN_DUMMY) == 0 {
        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, buf.opt_mut());
        if apply_autocmds(EVENT_BUFNEW, None, None, false, buf.opt_mut())
            && !bufref_valid(Some(&bufref))
        {
            return None;
        }
        if (flags & BLN_LISTED) != 0
            && apply_autocmds(EVENT_BUFADD, None, None, false, buf.opt_mut())
            && !bufref_valid(Some(&bufref))
        {
            return None;
        }
        if aborting() {
            return None;
        }
    }

    let buf_ref = buf.borrow_mut();
    buf_ref
        .b_prompt_callback
        .data
        .set(crate::eval::typval::CallbackData::None);
    buf_ref
        .b_prompt_interrupt
        .data
        .set(crate::eval::typval::CallbackData::None);
    buf_ref.b_prompt_text.set(None);
    *buf_ref.b_prompt_start.borrow_mut() = INIT_FMARK;
    buf_ref.b_prompt_start.borrow_mut().mark.col = 2;
    buf_ref.b_prompt_append_new_line.set(true);

    Some(buf)
}

/// Return true if the current buffer is empty, unnamed, unmodified and used in
/// only one window.
pub fn curbuf_reusable() -> bool {
    !g_st().curbuf.get().is_null()
        && curbuf_mut().b_ffname.get().is_none()
        && curbuf_mut().b_nwindows.get() <= 1
        && curbuf_mut().terminal.get().is_null()
        && (curbuf_mut().b_ml.ml_mfp.borrow().is_none() || buf_is_empty(&curbuf_mut()))
        && !bt_quickfix(Some(&crate::globals::curbuf_mut()))
        && !curbuf_is_changed()
}

/// Free the memory for the options of a buffer.
pub fn free_buf_options(buf: &FileBuf, free_p_ff: bool) {
    if free_p_ff {
        buf.b_p_fenc.clear_option_str();
        buf.b_p_ff.clear_option_str();
        buf.b_p_bh.clear_option_str();
        buf.b_p_bt.clear_option_str();
    }
    buf.b_p_def.clear_option_str();
    buf.b_p_inc.clear_option_str();
    callback_free(&buf.b_p_inex);
    callback_free(&buf.b_p_inde);
    buf.b_p_indk.clear_option_str();
    buf.b_p_fp.clear_option_str();
    callback_free(&buf.b_p_fex);
    buf.b_p_kp.clear_option_str();
    buf.b_p_mps.clear_option_str();
    buf.b_p_fo.clear_option_str();
    buf.b_p_flp.clear_option_str();
    buf.b_p_isk.clear_option_str();
    buf.b_p_vsts.clear_option_str();
    buf.b_p_vsts_nopaste.set(None);
    *buf.b_p_vsts_array.borrow_mut() = Vec::new();
    buf.b_p_vts.clear_option_str();
    *buf.b_p_vts_array.borrow_mut() = Vec::new();
    buf.b_p_keymap.clear_option_str();
    buf.b_kmap_ga.borrow_mut().clear();
    buf.b_p_com.clear_option_str();
    buf.b_p_cms.clear_option_str();
    buf.b_p_nf.clear_option_str();
    buf.b_p_syn.clear_option_str();
    buf.b_s.b_syn_isk.set(None);
    buf.b_s.b_p_spc.clear_option_str();
    buf.b_s.b_p_spf.clear_option_str();
    *buf.b_s.b_cap_prog.borrow_mut() = None;
    buf.b_s.b_p_spl.clear_option_str();
    buf.b_s.b_p_spo.clear_option_str();
    buf.b_p_sua.clear_option_str();
    buf.b_p_ft.clear_option_str();
    buf.b_p_cink.clear_option_str();
    buf.b_p_cino.clear_option_str();
    buf.b_p_lop.clear_option_str();
    buf.b_p_cinsd.clear_option_str();
    buf.b_p_cinw.clear_option_str();
    buf.b_p_cot.clear_option_str();
    buf.b_p_cpt.clear_option_str();
    #[cfg(windows)]
    buf.b_p_csl.clear_option_str();
    callback_free(&buf.b_p_cfu);
    callback_free(&buf.b_p_ofu);
    callback_free(&buf.b_p_tsrfu);
    clear_cpt_callbacks(&mut buf.b_p_cpt_cb.borrow_mut());
    buf.b_p_gefm.clear_option_str();
    buf.b_p_gp.clear_option_str();
    buf.b_p_mp.clear_option_str();
    buf.b_p_efm.clear_option_str();
    buf.b_p_ep.clear_option_str();
    buf.b_p_path.clear_option_str();
    buf.b_p_tags.clear_option_str();
    buf.b_p_tc.clear_option_str();
    callback_free(&buf.b_p_tfu);
    callback_free(&buf.b_p_ffu);
    buf.b_p_dict.clear_option_str();
    buf.b_p_dia.clear_option_str();
    buf.b_p_tsr.clear_option_str();
    buf.b_p_qe.clear_option_str();
    buf.b_p_ac.set(None);
    buf.b_p_ar.set(None);
    buf.b_p_fs.set(None);
    buf.b_p_ul.set(NO_LOCAL_UNDOLEVEL);
    buf.b_p_lw.clear_option_str();
    buf.b_p_bkc.clear_option_str();
    buf.b_p_menc.clear_option_str();
}

/// Get alternate file `n`.
pub fn buflist_getfile(n: i32, mut lnum: Linenr, options: i32, forceit: i32) -> bool {
    let Some(buf_ref) = buflist_findnr_opt(n) else {
        if (options & GETF_ALT) != 0 && n == 0 {
            emsg(gettext(E_NOALT));
        } else {
            semsg!(gettext(E_BUFFER_NR_NOT_FOUND), n);
        }
        return false;
    };
    let buf = buf_ref.clone();

    if buf == g_st().curbuf.get() {
        return true;
    }

    if text_or_buf_locked() {
        return false;
    }

    let mut saved_fm: Option<Fmark> = None;
    let col: Colnr;
    let mut restore_view = false;
    if lnum == 0 {
        let fm_ref = buflist_findfmark(&buf_ref);
        lnum = fm_ref.mark.lnum;
        col = fm_ref.mark.col;
        saved_fm = Some(Fmark {
            mark: fm_ref.mark,
            fnum: fm_ref.fnum,
            timestamp: fm_ref.timestamp,
            view: fm_ref.view,
            additional_data: None,
        });
        restore_view = true;
    } else {
        col = 0;
    }
    let buf_fnum = buf_ref.b_fnum.get();

    if (options & GETF_SWITCH) != 0 {
        let wp = swbuf_goto_win_with_buf(buf.opt_mut());

        if wp.is_none()
            && (opt_vars().swb_flags.get()
                & (K_OPT_SWB_FLAG_VSPLIT | K_OPT_SWB_FLAG_SPLIT | K_OPT_SWB_FLAG_NEWTAB))
                != 0
            && !buf_is_empty(&curbuf_mut())
        {
            if (opt_vars().swb_flags.get() & K_OPT_SWB_FLAG_NEWTAB) != 0 {
                tabpage_new();
            } else if !win_split(
                0,
                if (opt_vars().swb_flags.get() & K_OPT_SWB_FLAG_VSPLIT) != 0 {
                    WSP_VERT
                } else {
                    0
                },
            ) {
                return false;
            }
            reset_binding(&curwin_mut());
        }
    }

    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() + (1));
    if getfile(
        buf_fnum,
        None,
        None,
        (options & GETF_SETMARK) != 0,
        lnum,
        forceit != 0,
    )
    .is_success()
    {
        g_st()
            .redrawing_disabled
            .set(g_st().redrawing_disabled.get() - (1));

        if !opt_vars().p_sol.get() && col != 0 {
            curwin_mut().w_cursor.set_col(col);
            check_cursor_col(&curwin_mut());
            curwin_mut().w_cursor.set_coladd(0);
            curwin_mut().w_set_curswant.set(true);
        }
        if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_VIEW) != 0 && restore_view {
            mark_view_restore(saved_fm.as_ref());
        }
        return true;
    }
    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() - (1));
    false
}

fn buflist_getfpos() {
    let fm = buflist_findfmark(&curbuf_mut());
    let fpos = fm.mark;
    let saved_fm = Fmark {
        mark: fm.mark,
        fnum: fm.fnum,
        timestamp: fm.timestamp,
        view: fm.view,
        additional_data: None,
    };
    curwin_mut().w_cursor.set_lnum(fpos.lnum);
    check_cursor_lnum(&curwin_mut());

    if opt_vars().p_sol.get() {
        curwin_mut().w_cursor.set_col(0);
    } else {
        curwin_mut().w_cursor.set_col(fpos.col);
        check_cursor_col(&curwin_mut());
        curwin_mut().w_cursor.set_coladd(0);
        curwin_mut().w_set_curswant.set(true);
    }

    if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_VIEW) != 0 {
        mark_view_restore(Some(&saved_fm));
    }
}

/// Find file in buffer list by name (it has to be for the current window).
pub fn buflist_findname_exp(fname: Option<&CStr>) -> Option<crate::globals::BufPtr> {
    let ffname = FullName_save(fname, cfg!(unix))?;
    buflist_findname(Some(ffname.as_c_str()))
}

/// Find file in buffer list by name.
pub fn buflist_findname(ffname: Option<&CStr>) -> Option<crate::globals::BufPtr> {
    let mut file_id: FileID = FileID::new();
    let file_id_valid = ffname.is_some_and(|s| os_fileid(s, &mut file_id));
    buflist_findname_file_id(ffname, &mut file_id, file_id_valid)
}

#[expect(clippy::manual_find)]
fn buflist_findname_file_id(
    ffname: Option<&CStr>,
    file_id: &mut FileID,
    file_id_valid: bool,
) -> Option<crate::globals::BufPtr> {
    for buf in for_all_buffers_rev_ptr() {
        if (buf.b_flags.get() & BF_DUMMY) == 0
            && !otherfile_buf(&buf, ffname, Some(file_id), file_id_valid)
        {
            return Some(buf);
        }
    }
    None
}

/// Find file in buffer list by a regexp pattern.
pub fn buflist_findpat(pat_slice: &[u8], unlisted: bool, diffmode: bool, curtab_only: bool) -> i32 {
    let mut match_fnum = -1;

    if pat_slice == b"%" || pat_slice == b"#" {
        match_fnum = if pat_slice[0] == b'%' {
            curbuf_mut().b_fnum.get()
        } else {
            curwin_mut().w_alt_fnum.get()
        };
        let found_buf = buflist_findnr(match_fnum);
        if diffmode && found_buf.is_none_or(|b| !diff_mode_buf(&b)) {
            match_fnum = -1;
        }
    } else {
        let Some(pat_cstr) = file_pat_to_reg_pat(pat_slice, None, 0) else {
            return -1;
        };
        let mut pat_vec: Vec<u8> = pat_cstr.into_bytes_with_nul();
        let pat_end_idx = crate::memory::bytes_as_cstr(&pat_vec)
            .to_bytes()
            .len()
            .saturating_sub(1);
        let toggledollar = pat_end_idx > 0 && pat_vec[pat_end_idx] == b'$';

        let mut find_listed = 1;
        loop {
            for attempt in 0..=3 {
                if toggledollar {
                    pat_vec[pat_end_idx] = if attempt < 2 { 0 } else { b'$' };
                }
                let pat_start = usize::from(pat_vec[0] == b'^' && (attempt & 1) == 0);
                let p = crate::memory::bytes_as_cstr(&pat_vec[pat_start..]);

                let mut regmatch: Regmatch = Regmatch::new();
                regmatch.set_prog(vim_regcomp(
                    p,
                    if opt_vars().p_magic.get() {
                        RE_MAGIC
                    } else {
                        0
                    },
                ));

                for buf in for_all_buffers_rev_ptr() {
                    if regmatch.regprog.is_null() {
                        return -1;
                    }
                    if i32::from(buf.b_p_bl.get()) == find_listed
                        && (!diffmode || diff_mode_buf(&buf))
                        && buflist_match(&mut regmatch, &buf, false).is_some()
                    {
                        if curtab_only {
                            let mut found_window = false;
                            for wp in for_all_windows_mut() {
                                if wp.w_buffer.get() == buf {
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
                        match_fnum = buf.b_fnum.get();
                    }
                }

                regmatch.free_prog();
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

    if match_fnum < 0 {
        let pat_cstr = crate::memory::bytes_to_cstring(pat_slice);
        if match_fnum == -2 {
            semsg!(gettext(c"E93: More than one match for %s"), &pat_cstr);
        } else {
            semsg!(gettext(c"E94: No matching buffer for %s"), &pat_cstr);
        }
    }
    match_fnum
}

fn buf_time_compare(buf1: &FileBuf, buf2: &FileBuf) -> std::cmp::Ordering {
    buf2.b_last_used.get().cmp(&buf1.b_last_used.get())
}

/// Find all buffer names that match.
pub fn expand_bufnames(pat_in: &[u8], file: &mut Vec<std::ffi::CString>, options: i32) -> bool {
    let pat_bytes = crate::memory::bytes_until_nul(pat_in);
    let pat_cstr = crate::memory::bytes_to_cstring(pat_bytes);
    {
        let mut matches: Vec<Bufmatch> = Vec::new();

        file.clear();

        if (options & BUF_DIFF_FILTER) != 0 && !curwin_mut().w_onebuf_opt.wo_diff.get() {
            return false;
        }

        let fuzzy = cmdline_fuzzy_complete(&pat_cstr);
        let use_matches = !fuzzy && (options & WILD_BUFLASTUSED) != 0;

        let mut patc_owned: Option<std::ffi::CString> = None;
        let mut fuzmatch: Vec<FuzmatchStr> = Vec::new();
        let mut regmatch: Regmatch = Regmatch::new();

        if !fuzzy {
            let patc: &CStr = if pat_bytes.first() == Some(&b'^') && pat_bytes.len() > 1 {
                let pc = crate::memory::bytes_to_cstring(&pat_bytes[1..]);
                patc_owned = Some(pc);
                patc_owned.as_deref().unwrap()
            } else if pat_bytes.first() == Some(&b'^') {
                c""
            } else {
                &pat_cstr
            };
            regmatch.set_prog(vim_regcomp(patc, RE_MAGIC));
            if regmatch.regprog.is_null() {
                return false;
            }
        }

        let mut score = 0;
        for buf in for_all_buffers_ptr() {
            if !buf.b_p_bl.get() {
                continue;
            }
            if (options & BUF_DIFF_FILTER) != 0
                && (g_st().curbuf.get() == buf || !diff_mode_buf(&buf))
            {
                continue;
            }

            let mut p = None;
            if !fuzzy {
                p = buflist_match(&mut regmatch, &buf, opt_vars().p_wic.get());
            } else {
                let pat_opt = Some(pat_bytes);
                score = fuzzy_match_str(buf.sfname_opt().as_deref().map(CStr::to_bytes), pat_opt);
                if score != FUZZY_SCORE_NONE && buf.b_sfname.get().is_some() {
                    p = buf.sfname_opt();
                }
                if p.is_none() {
                    score =
                        fuzzy_match_str(buf.ffname_opt().as_deref().map(CStr::to_bytes), pat_opt);
                    if score != FUZZY_SCORE_NONE && buf.b_ffname.get().is_some() {
                        p = buf.ffname_opt();
                    }
                }
            }

            let Some(p) = p else {
                continue;
            };

            let p_cstr = if (options & WILD_HOME_REPLACE) != 0 {
                home_replace_save(Some(&buf), Some(&p))
            } else {
                p.as_cstr().to_owned()
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
            regmatch.free_prog();
            drop(patc_owned);
        }

        if !fuzzy {
            if use_matches && !matches.is_empty() {
                if matches.len() > 1 {
                    matches
                        .sort_unstable_by(|a, b| buf_time_compare(a.buf.borrow(), b.buf.borrow()));
                }

                file.reserve_exact(matches.len());
                if matches[0].buf == g_st().curbuf.get() {
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

fn buflist_match(
    rmp: &mut Regmatch,
    buf: &FileBuf,
    ignore_case: bool,
) -> Option<crate::option_vars::RcCStr> {
    let mut match_ = fname_match(rmp, buf.sfname_opt(), ignore_case);
    if match_.is_none() && !rmp.regprog.is_null() {
        match_ = fname_match(rmp, buf.ffname_opt(), ignore_case);
    }
    match_
}

fn fname_match(
    rmp: &mut Regmatch,
    name: Option<crate::option_vars::RcCStr>,
    ignore_case: bool,
) -> Option<crate::option_vars::RcCStr> {
    let name_cstr = name?;
    let mut match_ = None;

    if rmp.regprog.is_null() {
        return None;
    }

    rmp.rm_ic = opt_vars().p_fic.get() || ignore_case;
    if vim_regexec(rmp, &name_cstr, 0) {
        match_ = Some(name_cstr);
    } else if !rmp.regprog.is_null() {
        let p = home_replace_save(None, Some(&name_cstr));
        if vim_regexec(rmp, &p, 0) {
            match_ = Some(name_cstr);
        }
    }

    match_
}

/// Find a file in the buffer list by buffer number.
pub fn buflist_findnr(mut nr: i32) -> Option<crate::globals::BufPtr> {
    if nr == 0 {
        nr = curwin_mut().w_alt_fnum.get();
    }
    g_st()
        .buffer_handles
        .borrow()
        .get(&nr)
        .cloned()
        .filter(|p| !p.is_null())
}

#[inline]
pub fn buflist_findnr_opt(nr: i32) -> Option<crate::globals::BufPtr> {
    buflist_findnr(nr)
}

pub fn buflist_nr2name(n: i32, fullname: i32, helptail: i32) -> Option<std::ffi::CString> {
    let buf = buflist_findnr_opt(n)?;
    let name_opt = if fullname != 0 {
        buf.ffname_opt()
    } else {
        buf.fname_opt()
    };
    Some(home_replace_save(
        (helptail != 0).then_some(&buf),
        name_opt.as_deref(),
    ))
}

/// Set the line and column numbers for the given buffer and window.
pub fn buflist_setfpos(
    buf: &FileBuf,
    win_opt: Option<&WinInfoInfo>,
    mut lnum: Linenr,
    col: Colnr,
    copy_options: bool,
) {
    let win = crate::globals::WinPtr::from(win_opt);
    let existing_pos = buf.b_wininfo.borrow().iter().position(|w| w.wi_win == win);
    let mut wip = if let Some(i) = existing_pos {
        let mut wip = buf.b_wininfo.borrow_mut().remove(i);
        if copy_options && wip.wi_optset {
            clear_winopt(&wip.wi_opt);
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
        if let Some(w) = win_opt {
            wip.wi_mark.view = mark_view_make(w, wip.wi_mark.mark);
        }
    }
    if let Some(w) = win_opt {
        wip.wi_changelistidx = w.w_changelistidx.get();
        if copy_options {
            copy_winopt(&w.w_onebuf_opt, &wip.wi_opt);
            wip.wi_fold_manual = w.w_fold_manual.get();
            wip.wi_folds.clone_from(&w.w_folds.borrow());
            wip.wi_optset = true;
        }
    }

    buf.b_wininfo.borrow_mut().insert(0, wip);
}

fn wininfo_other_tab_diff(wip: &WinInfo) -> bool {
    if !wip.wi_opt.wo_diff.get() {
        return false;
    }

    for wp in for_all_windows_ptr() {
        if wip.wi_win == wp {
            return false;
        }
    }
    true
}

fn find_wininfo(
    buf: &FileBuf,
    need_options: bool,
    skip_diff_buffer: bool,
) -> Option<std::cell::RefMut<'_, WinInfo>> {
    let buf_ptr = crate::globals::BufPtr::from(buf);
    let pos = buf
        .b_wininfo
        .borrow()
        .iter()
        .position(|wip| {
            wip.wi_win == curwin_ptr()
                && (!skip_diff_buffer || !wininfo_other_tab_diff(wip))
                && (!need_options || wip.wi_optset)
        })
        .or_else(|| {
            if skip_diff_buffer {
                buf.b_wininfo.borrow().iter().position(|wip| {
                    !wininfo_other_tab_diff(wip)
                        && (!need_options
                            || wip.wi_optset
                            || wip.win_opt().is_some_and(|w| w.w_buffer.get() == buf_ptr))
                })
            } else {
                (!buf.b_wininfo.borrow().is_empty()).then_some(0)
            }
        })?;
    Some(std::cell::RefMut::map(buf.b_wininfo.borrow_mut(), |v| {
        &mut v[pos]
    }))
}

/// Reset the local window options to the values last used in this window.
pub fn get_winopts(buf: &FileBuf) {
    let buf_ptr = crate::globals::BufPtr::from(buf);
    clear_winopt(&curwin_mut().w_onebuf_opt);
    clear_folding(&curwin_mut());

    let mut wip = find_wininfo(buf, true, true);
    if let Some(wip) = wip.as_deref()
        && wip.wi_win != curwin_ptr()
        && let Some(wp) = wip.win_opt()
        && wp.w_buffer.get() == buf_ptr
        && wp.w_config.borrow().style != K_WIN_STYLE_MINIMAL
    {
        copy_winopt(&wp.w_onebuf_opt, &curwin_mut().w_onebuf_opt);
        curwin_mut().w_fold_manual.set(wp.w_fold_manual.get());
        curwin_mut().w_foldinvalid.set(true);
        let wp_folds = wp.w_folds.borrow().clone();
        *curwin_mut().w_folds.borrow_mut() = wp_folds;
    } else if let Some(wip) = wip.as_deref_mut()
        && wip.wi_optset
        && (wip.wi_win.is_null()
            || wip.wi_win == curwin_ptr()
            || wip
                .win_opt()
                .is_some_and(|w| w.w_config.borrow().style != K_WIN_STYLE_MINIMAL))
    {
        copy_winopt(&wip.wi_opt, &curwin_mut().w_onebuf_opt);
        curwin_mut().w_fold_manual.set(wip.wi_fold_manual);
        curwin_mut().w_foldinvalid.set(true);
        curwin_mut().w_folds.borrow_mut().clone_from(&wip.wi_folds);
    } else {
        copy_winopt(&curwin_mut().w_allbuf_opt, &curwin_mut().w_onebuf_opt);
    }
    if let Some(wip) = wip {
        curwin_mut().w_changelistidx.set(wip.wi_changelistidx);
    }

    if curwin_mut().w_config.borrow().style == K_WIN_STYLE_MINIMAL {
        didset_window_options(&curwin_mut(), false);
        win_set_minimal_style(&curwin_mut());
    }

    if opt_vars().p_fdls.get() >= 0 {
        curwin_mut()
            .w_onebuf_opt
            .wo_fdl
            .set(opt_vars().p_fdls.get());
    }
    didset_window_options(&curwin_mut(), false);
}

/// Find the mark for the buffer `buf` for the current window.
pub fn buflist_findfmark(buf: &FileBuf) -> Fmark {
    if let Some(wip) = find_wininfo(buf, false, false) {
        wip.wi_mark.clone()
    } else {
        NO_POSITION
    }
}

/// Find the lnum for the buffer `buf` for the current window.
pub fn buflist_findlnum(buf: &FileBuf) -> Linenr {
    buflist_findfmark(buf).mark.lnum
}

/// List all known file names (for `:files` and `:buffers` command).
pub fn buflist_list(eap: &mut Exarg) {
    let mut buflist: Vec<crate::globals::BufPtr> = for_all_buffers_ptr().collect();

    msg_ext_set_kind(c"list_cmd");
    if crate::strings::vim_haschr(eap.arg.as_cstr(), b't' as i32) && buflist.len() > 1 {
        buflist.sort_unstable_by(|a, b| buf_time_compare(a.borrow(), b.borrow()));
    }

    for buf_ptr in buflist {
        if g_st().got_int.get() {
            break;
        }
        let buf = buf_ptr.borrow_mut();
        let is_terminal = !buf.terminal.get().is_null();
        let job_running = buf.terminal_opt().is_some_and(terminal_running);

        if (!buf.b_p_bl.get()
            && !eap.forceit
            && !crate::strings::vim_haschr(eap.arg.as_cstr(), b'u' as i32))
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'u' as i32) && buf.b_p_bl.get())
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'+' as i32)
                && ((buf.b_flags.get() & BF_READERR) != 0 || !buf_is_changed(buf)))
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'a' as i32)
                && (buf.b_ml.ml_mfp.borrow().is_none() || buf.b_nwindows.get() == 0))
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'h' as i32)
                && (buf.b_ml.ml_mfp.borrow().is_none() || buf.b_nwindows.get() != 0))
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'R' as i32)
                && (!is_terminal || !job_running))
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'F' as i32)
                && (!is_terminal || job_running))
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'-' as i32) && buf.b_p_ma.get())
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'=' as i32) && !buf.b_p_ro.get())
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'x' as i32)
                && (buf.b_flags.get() & BF_READERR) == 0)
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'%' as i32)
                && g_st().curbuf.get() != buf)
            || (crate::strings::vim_haschr(eap.arg.as_cstr(), b'#' as i32)
                && (g_st().curbuf.get() == buf
                    || curwin_mut().w_alt_fnum.get() != buf.b_fnum.get()))
        {
            continue;
        }

        if let Some(name) = buf_spname(buf) {
            crate::memory::copy_bytes_to_c_buf(&mut *g_st().name_buff_bytes_mut(), name.to_bytes());
        } else {
            home_replace(
                Some(buf),
                buf.fname_opt().as_deref(),
                &mut *g_st().name_buff_bytes_mut(),
                true,
            );
        }

        if message_filtered(&g_st().name_buff_cstr()) {
            continue;
        }

        let changed_char = if (buf.b_flags.get() & BF_READERR) != 0 {
            b'x' as i32
        } else if buf_is_changed(buf) {
            b'+' as i32
        } else {
            b' ' as i32
        };
        let mut ro_char = if !buf.b_p_ma.get() {
            b'-' as i32
        } else if buf.b_p_ro.get() {
            b'=' as i32
        } else {
            b' ' as i32
        };
        if let Some(term) = buf.terminal_opt() {
            ro_char = if terminal_running(term) {
                b'R' as i32
            } else {
                b'F' as i32
            };
        }

        if !ui_has(K_UIMESSAGES) || g_st().msg_col.get() > 0 {
            msg_putchar(b'\n' as i32);
        }
        let mut len = format_to_buf_safelen!(
            &mut g_st().iobuff.borrow_mut()[..IOSIZE - 20],
            c"%3d%c%c%c%c%c \"%s\"",
            buf.b_fnum.get(),
            if buf.b_p_bl.get() {
                b' ' as i32
            } else {
                b'u' as i32
            },
            if g_st().curbuf.get() == buf {
                b'%' as i32
            } else if curwin_mut().w_alt_fnum.get() == buf.b_fnum.get() {
                b'#' as i32
            } else {
                b' ' as i32
            },
            if buf.b_ml.ml_mfp.borrow().is_none() {
                b' ' as i32
            } else if buf.b_nwindows.get() == 0 {
                b'h' as i32
            } else {
                b'a' as i32
            },
            ro_char,
            changed_char,
            g_st().name_buff_cstr(),
        )
        .min(IOSIZE - 20);

        let mut i = 40 - vim_strsize(&g_st().iobuff_cstr());
        loop {
            g_st().iobuff_bytes_mut()[len] = b' ';
            len += 1;
            i -= 1;
            if i <= 0 || len >= IOSIZE - 18 {
                break;
            }
        }
        if crate::strings::vim_haschr(eap.arg.as_cstr(), b't' as i32) && buf.b_last_used.get() != 0
        {
            undo_fmt_time(
                &mut g_st().iobuff_bytes_mut()[len..IOSIZE],
                buf.b_last_used.get(),
            );
        } else {
            format_to_buf!(
                &mut g_st().iobuff.borrow_mut()[len..IOSIZE],
                gettext(c"line %lld"),
                if g_st().curbuf.get() == buf {
                    i64::from(curwin_mut().w_cursor.get().lnum)
                } else {
                    i64::from(buflist_findlnum(buf))
                },
            );
        }

        msg_outtrans(&g_st().iobuff_cstr(), 0, false);
        line_breakcheck();
    }
}

/// Get file name and line number for file `fnum`.
pub fn buflist_name_nr(
    fnum: i32,
    fname: &mut Option<std::ffi::CString>,
    lnum: &mut Linenr,
) -> bool {
    let Some(buf) = buflist_findnr_opt(fnum) else {
        return false;
    };
    let Some(f) = buf.fname_opt().as_deref().map(CStr::to_owned) else {
        return false;
    };

    *fname = Some(f);
    *lnum = buflist_findlnum(&buf);

    true
}

/// Set the file name for `buf` to `ffname_arg`, short file name to `sfname_arg`.
pub fn setfname(
    buf: &FileBuf,
    ffname_arg: Option<&CStr>,
    sfname_arg: Option<&CStr>,
    message: bool,
) -> bool {
    let mut obuf = crate::globals::BufPtr::NULL;
    let mut file_id: FileID = FileID::new();
    let mut file_id_valid = false;

    if ffname_arg.is_none_or(|s| s.to_bytes().is_empty()) {
        buf.clear_fnames();
    } else {
        let Some((ffname_cstr, sfname_raw)) = fname_expand(Some(buf), ffname_arg, sfname_arg)
        else {
            return false;
        };

        file_id_valid = os_fileid(&ffname_cstr, &mut file_id);
        if (buf.b_flags.get() & BF_DUMMY) == 0 {
            obuf = crate::globals::BufPtr::from(buflist_findname_file_id(
                Some(&ffname_cstr),
                &mut file_id,
                file_id_valid,
            ));
        }
        if !obuf.is_null() && obuf != buf {
            let mut in_use = false;

            for tab in for_all_tabpages_ptr() {
                for win in for_all_windows_in_tab_mut(&tab) {
                    if win.w_buffer.get() == obuf {
                        in_use = true;
                    }
                }
            }

            if obuf.b_ml.ml_mfp.borrow().is_some() || in_use {
                if message {
                    emsg(gettext(c"E95: Buffer with this name already exists"));
                }
                return false;
            }
            buf.b_locked.set(buf.b_locked.get() + (1));
            close_buffer(None, obuf.opt_mut(), DOBUF_WIPE, false, false, false);
            buf.b_locked.set(buf.b_locked.get() - (1));
        }
        let mut sf_vec = sfname_raw.into_bytes_with_nul();
        if CASE_INSENSITIVE_FILENAME {
            to_slash(&mut sf_vec);
            path_fix_case(&mut sf_vec);
        }
        let sfname_cstr = crate::memory::cstring_until_nul(&sf_vec);
        buf.set_ffname(Some(ffname_cstr));
        buf.set_sfname(Some(sfname_cstr));
    }
    buf.b_fname.set(BufFname::Short);
    if !file_id_valid {
        buf.file_id_valid.set(false);
    } else {
        buf.file_id.set(file_id);
        buf.file_id_valid.set(true);
    }

    buf_name_changed(buf);
    true
}

/// Crude way of changing the name of a buffer.
pub fn buf_set_name(fnum: i32, name: &std::ffi::CStr) {
    let Some(buf) = buflist_findnr_opt(fnum) else {
        return;
    };

    buf.clear_fnames();
    if let Some((ffname_cstr, sfname_cstr)) = fname_expand(Some(&buf), Some(name), None) {
        buf.set_ffname(Some(ffname_cstr));
        buf.set_sfname(Some(sfname_cstr));
    }
    buf.b_fname.set(BufFname::Short);
}

/// Take care of what needs to be done when the name of buffer `buf` has changed.
pub fn buf_name_changed(buf: &FileBuf) {
    if buf.b_ml.ml_mfp.borrow().is_some() {
        ml_setname(buf);
    }

    if curwin_mut().w_buffer.get() == (std::ptr::from_ref(buf).cast_mut()) {
        check_arg_idx(&curwin_mut());
    }
    maketitle();
    status_redraw_all();
    fmarks_check_names(buf);
    ml_timestamp(buf);
}

/// Set alternate file name for current window.
pub fn setaltfname(
    ffname: Option<&CStr>,
    sfname: Option<&CStr>,
    lnum: Linenr,
) -> Option<crate::globals::BufPtr> {
    let buf = buflist_new(ffname, sfname, lnum, 0);
    if let Some(ref b) = buf
        && (g_st().cmdmod.borrow().cmod_flags & CMOD_KEEPALT) == 0
    {
        curwin_mut().w_alt_fnum.set(b.b_fnum.get());
    }
    buf
}

/// Get alternate file name for current window.
pub fn getaltfname(errmsg: bool) -> Option<std::ffi::CString> {
    let mut fname = None;
    let mut dummy: Linenr = 0;

    if !buflist_name_nr(0, &mut fname, &mut dummy) {
        if errmsg {
            emsg(gettext(E_NOALT));
        }
        return None;
    }
    fname
}

/// Add a file name to the buflist and return its number.
pub fn buflist_add(fname: Option<&CStr>, flags: i32) -> i32 {
    let buf = buflist_new(fname, None, 0, flags);
    buf.map_or(0, |b| b.b_fnum.get())
}

/// Set alternate cursor position for the current buffer and window `win`.
pub fn buflist_altfpos(win: &WinInfoInfo) {
    let lnum = win.w_cursor.get().lnum;
    let col = win.w_cursor.get().col;
    buflist_setfpos(&curbuf_mut(), Some(win), lnum, col, true);
}

/// Check that `ffname` is not the same file as current file.
pub fn otherfile(ffname: Option<&CStr>) -> bool {
    otherfile_buf(&curbuf_mut(), ffname, None, false)
}

fn otherfile_buf(
    buf: &FileBuf,
    ffname: Option<&CStr>,
    file_id_p: Option<&mut FileID>,
    mut file_id_valid: bool,
) -> bool {
    let Some(ffname_cstr) = ffname else {
        return true;
    };
    let Some(buf_ffname) = buf.ffname_opt() else {
        return true;
    };
    if ffname_cstr.to_bytes().is_empty() {
        return true;
    }
    if path_equal(ffname_cstr, &buf_ffname, K_PATH_CMP_LITERAL) {
        return false;
    }
    let mut local_file_id: FileID = FileID::new();
    let file_id_ref: &FileID = if let Some(fid) = file_id_p {
        fid
    } else {
        file_id_valid = os_fileid(ffname_cstr, &mut local_file_id);
        &local_file_id
    };
    if !file_id_valid {
        return true;
    }
    if buf_same_file_id(buf, file_id_ref) {
        buf_set_file_id(buf);
        if buf_same_file_id(buf, file_id_ref) {
            return false;
        }
    }
    true
}

/// Set `file_id` for a buffer.
pub fn buf_set_file_id(buf: &FileBuf) {
    let mut file_id: FileID = FileID::new();
    if let Some(fname) = buf.fname_opt()
        && os_fileid(&fname, &mut file_id)
    {
        buf.file_id_valid.set(true);
        buf.file_id.set(file_id);
    } else {
        buf.file_id_valid.set(false);
    }
}

#[inline]
fn buf_same_file_id(buf: &FileBuf, file_id: &FileID) -> bool {
    buf.file_id_valid.get() && os_fileid_equal(&buf.file_id.get(), file_id)
}

/// Print info about the current buffer.
pub fn fileinfo(fullname: i32, shorthelp: i32, dont_truncate: bool) {
    let mut buffer_vec = vec![0u8; IOSIZE];
    let mut bufferlen = 0usize;

    if fullname > 1 {
        bufferlen = format_to_buf_safelen!(&mut buffer_vec, c"buf %d: ", curbuf_mut().b_fnum.get());
    }

    buffer_vec[bufferlen] = b'"';
    bufferlen += 1;

    if let Some(name) = buf_spname(&curbuf_mut()) {
        bufferlen += format_to_buf_safelen!(&mut buffer_vec[bufferlen..], c"%s", name);
    } else {
        let name = if fullname == 0 && curbuf_mut().fname_opt().is_some() {
            curbuf_mut().fname_opt()
        } else {
            curbuf_mut().ffname_opt()
        };
        bufferlen += home_replace(
            (shorthelp != 0).then(curbuf_mut).as_deref(),
            name.as_deref(),
            &mut buffer_vec[bufferlen..],
            true,
        );
    }

    let dontwrite = bt_dontwrite(Some(&crate::globals::curbuf_mut()));
    let is_changed = curbuf_is_changed();
    let mod_str = if is_changed {
        if shortmess(K_SHM_MOD) {
            c" [+]"
        } else {
            gettext(c" [Modified]")
        }
    } else {
        c" "
    };
    let notedited_str = if (curbuf_mut().b_flags.get() & BF_NOTEDITED) != 0 && !dontwrite {
        gettext(c"[Not edited]")
    } else {
        c""
    };
    let new_str = if (curbuf_mut().b_flags.get() & BF_NEW) != 0 && !dontwrite {
        gettext(c"[New]")
    } else {
        c""
    };
    let readerr_str = if (curbuf_mut().b_flags.get() & BF_READERR) != 0 {
        gettext(c"[Read errors]")
    } else {
        c""
    };
    let ro_str = if curbuf_mut().b_p_ro.get() {
        if shortmess(K_SHM_RO) {
            gettext(c"[RO]")
        } else {
            gettext(c"[readonly]")
        }
    } else {
        c""
    };
    let sep_str = if is_changed
        || (curbuf_mut().b_flags.get() & BF_WRITE_MASK) != 0
        || curbuf_mut().b_p_ro.get()
    {
        c" "
    } else {
        c""
    };

    bufferlen += format_to_buf_safelen!(
        &mut buffer_vec[bufferlen..],
        c"\"%s%s%s%s%s%s",
        mod_str,
        notedited_str,
        new_str,
        readerr_str,
        ro_str,
        sep_str,
    );

    let line_count = i64::from(curbuf_mut().b_ml.ml_line_count.get());
    let cur_lnum = i64::from(curwin_mut().w_cursor.get().lnum);
    if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 {
        bufferlen +=
            format_to_buf_safelen!(&mut buffer_vec[bufferlen..], c"%s", gettext(NO_LINES_MSG));
    } else if opt_vars().p_ru.get() {
        bufferlen += format_to_buf_safelen!(
            &mut buffer_vec[bufferlen..],
            ngettext(
                c"%lld line --%d%%--",
                c"%lld lines --%d%%--",
                line_count as usize,
            ),
            line_count,
            calc_percentage(cur_lnum, line_count),
        );
    } else {
        bufferlen += format_to_buf_safelen!(
            &mut buffer_vec[bufferlen..],
            gettext(c"line %lld of %lld --%d%%-- col "),
            cur_lnum,
            line_count,
            calc_percentage(cur_lnum, line_count),
        );
        validate_virtcol(&curwin_mut());
        bufferlen += col_print(
            &mut buffer_vec[bufferlen..],
            curwin_mut().w_cursor.get().col + 1,
            curwin_mut().w_virtcol.get() + 1,
        ) as usize;
    }

    append_arg_number(&curwin_mut(), &mut buffer_vec[bufferlen..]);

    if dont_truncate {
        msg_start();
        let n = g_st().msg_scroll.get();
        g_st().msg_scroll.set(1);
        msg(crate::memory::bytes_as_cstr(&buffer_vec), 0);
        g_st().msg_scroll.set(n);
    } else {
        let p = msg_trunc(&mut buffer_vec, false, 0);
        if g_st().restart_edit.get() != 0
            || (g_st().msg_scrolled.get() != 0 && !g_st().need_wait_return.get())
        {
            set_keep_msg(p, 0);
        }
    }
}

pub fn col_print(buf: &mut [u8], col: i32, vcol: i32) -> i32 {
    if col == vcol {
        format_to_buf_safelen!(buf, c"%d", col) as i32
    } else {
        format_to_buf_safelen!(buf, c"%d-%d", col, vcol) as i32
    }
}

/// Put the title name in the title bar and icon of the window.
pub fn maketitle() {
    let mut title_cstr: Option<std::ffi::CString> = None;
    let mut icon_cstr: Option<std::ffi::CString> = None;
    let mut buf = [0u8; IOSIZE];

    if !redrawing() {
        g_st().need_maketitle.set(true);
        return;
    }

    g_st().need_maketitle.set(false);
    if !opt_vars().p_title.get()
        && !opt_vars().p_icon.get()
        && LASTTITLE.with(|c| c.borrow().is_none())
        && LASTICON.with(|c| c.borrow().is_none())
    {
        return;
    }

    if opt_vars().p_title.get() {
        let mut maxlen = 0;

        if opt_vars().p_titlelen.get() > 0 {
            maxlen = ((opt_vars().p_titlelen.get() * i64::from(g_st().columns_global.get()) / 100)
                as i32)
                .max(10);
        }

        if !opt_vars().p_titlestring.is_empty() {
            if (g_st().stl_syntax.get() & STL_IN_TITLE) != 0 {
                let titlestr = opt_vars().p_titlestring.as_cstr().to_owned();
                build_stl_str_hl_slice(
                    curwin_ptr(),
                    &mut buf,
                    Some(&titlestr),
                    K_OPT_TITLESTRING,
                    0,
                    0,
                    maxlen,
                    false,
                    None,
                    false,
                    None,
                );
                title_cstr = Some(crate::memory::bytes_as_cstr(&buf).to_owned());
            } else {
                title_cstr = Some(
                    opt_vars()
                        .p_titlestring
                        .as_cstr()
                        .to_owned()
                        .as_cstr()
                        .to_owned(),
                );
            }
        } else {
            #[cfg(windows)]
            let p_ssl_save = opt_vars().p_ssl.get();
            #[cfg(windows)]
            {
                opt_vars().p_ssl.set(true);
            }
            let default_titlestring = c"%t%( %M%)%( (%{expand('%:p:~:h')})%)%a - Nvim";
            build_stl_str_hl_slice(
                curwin_ptr(),
                &mut buf,
                Some(default_titlestring),
                K_OPT_TITLESTRING,
                0,
                0,
                maxlen,
                false,
                None,
                false,
                None,
            );
            title_cstr = Some(crate::memory::bytes_as_cstr(&buf).to_owned());
            #[cfg(windows)]
            {
                opt_vars().p_ssl.set(p_ssl_save);
            }
        }
    }
    let mut mustset = LASTTITLE.with(|c| value_change(title_cstr.as_deref(), &mut c.borrow_mut()));

    if opt_vars().p_icon.get() {
        if !opt_vars().p_iconstring.is_empty() {
            if (g_st().stl_syntax.get() & STL_IN_ICON) != 0 {
                let iconstr = opt_vars().p_iconstring.as_cstr().to_owned();
                build_stl_str_hl_slice(
                    curwin_ptr(),
                    &mut buf,
                    Some(&iconstr),
                    K_OPT_ICONSTRING,
                    0,
                    0,
                    0,
                    false,
                    None,
                    false,
                    None,
                );
                icon_cstr = Some(crate::memory::bytes_as_cstr(&buf).to_owned());
            } else {
                icon_cstr = Some(
                    opt_vars()
                        .p_iconstring
                        .as_cstr()
                        .to_owned()
                        .as_cstr()
                        .to_owned(),
                );
            }
        } else {
            let cb = curbuf_mut();
            let sp = buf_spname(&cb);
            let ffname = cb.ffname_cstr();
            let name_bytes = if let Some(ref sp) = sp {
                sp.to_bytes()
            } else {
                path_tail(&ffname).to_bytes()
            };
            let mut off = 0usize;
            if name_bytes.len() > 100 {
                off = name_bytes.len() - 100;
                off = (off as isize + isize::from(utf_cp_bounds(name_bytes, off).end_off)) as usize;
            }
            let tail = &name_bytes[off..];
            let copy_len = tail.len().min(buf.len() - 1);
            buf[..copy_len].copy_from_slice(&tail[..copy_len]);
            buf[copy_len] = 0;
            trans_characters(&mut buf);
            icon_cstr = Some(crate::memory::bytes_as_cstr(&buf).to_owned());
        }
    }

    mustset |= LASTICON.with(|c| value_change(icon_cstr.as_deref(), &mut c.borrow_mut()));

    if mustset {
        resettitle();
    }
}

fn value_change(str_: Option<&CStr>, last: &mut Option<std::ffi::CString>) -> bool {
    let last_cstr = last.as_deref();
    if str_.is_none() != last_cstr.is_none()
        || (str_.is_some()
            && last_cstr.is_some()
            && str_.unwrap().to_bytes() != last_cstr.unwrap().to_bytes())
    {
        if let Some(s) = str_ {
            *last = Some(s.to_owned());
            return true;
        }
        *last = None;
        resettitle();
    }
    false
}

/// Set current window title.
pub fn resettitle() {
    let lasticon = LASTICON.with(|c| c.borrow().clone());
    let lasttitle = LASTTITLE.with(|c| c.borrow().clone());
    ui_call_set_icon(cstr_as_string(lasticon.as_deref()));
    ui_call_set_title(cstr_as_string(lasttitle.as_deref()));
}

pub fn free_titles() {
    LASTTITLE.with(|c| *c.borrow_mut() = None);
    LASTICON.with(|c| *c.borrow_mut() = None);
}

/// Get relative cursor position in window into `buf[]`.
pub fn get_rel_pos(wp: &WinInfoInfo, buf: &mut [u8]) -> i32 {
    if buf.len() < 3 {
        return 0;
    }

    let mut above = wp.w_topline.get() - 1;
    above += win_get_fill(wp, wp.w_topline.get()) - wp.w_topfill.get();
    if wp.w_topline.get() == 1 && wp.w_topfill.get() >= 1 {
        above = 0;
    }
    let below = wp.buf_mut().b_ml.ml_line_count.get() - wp.w_botline.get() + 1;
    if below <= 0 {
        return format_to_buf_safelen!(
            buf,
            c"%s",
            if above == 0 {
                gettext(c"All")
            } else {
                gettext(c"Bot")
            },
        ) as i32;
    }

    if above <= 0 {
        return format_to_buf_safelen!(buf, c"%s", gettext(c"Top")) as i32;
    }

    let perc = calc_percentage(i64::from(above), i64::from(above + below));
    let mut tmp = [0u8; 8];
    format_to_buf!(&mut tmp, gettext(c"%d%%"), perc);
    format_to_buf_safelen!(buf, gettext(c"%3s"), crate::memory::bytes_as_cstr(&tmp)) as i32
}

/// Append `(2 of 8)` to `buf[]`, if editing more than one file.
pub fn append_arg_number(wp: &WinInfoInfo, buf: &mut [u8]) -> i32 {
    let argcount = crate::arglist::argcount();
    if argcount <= 1 {
        return 0;
    }

    let msg = if wp.w_arg_idx_invalid.get() {
        gettext(c" ((%d) of %d)")
    } else {
        gettext(c" (%d of %d)")
    };

    format_to_buf_safelen!(buf, msg, wp.w_arg_idx.get() + 1, argcount) as i32
}

/// Make `ffname` a full file name, defaulting `sfname` to `ffname` if `None`.
pub fn fname_expand(
    buf: Option<&FileBuf>,
    ffname: Option<&CStr>,
    sfname: Option<&CStr>,
) -> Option<(std::ffi::CString, std::ffi::CString)> {
    #[cfg(not(windows))]
    let _ = buf;
    let ffname_in = ffname?;
    let sfname_in = sfname.unwrap_or(ffname_in);
    let mut full_cstr = fix_fname(Some(ffname_in))?;
    #[cfg_attr(not(windows), expect(unused_mut))]
    let mut sf_cstr = sfname_in.to_owned();
    if os_isdir(&full_cstr) {
        let mut name_vec: Vec<u8> = sf_cstr.as_bytes_with_nul().to_vec();
        let name_len = name_vec.len() - 1;
        let name_cstr = crate::memory::bytes_as_cstr(&name_vec);
        let past_head_off =
            get_past_head(name_cstr).as_ptr() as usize - name_cstr.as_ptr() as usize;
        if name_len > 0 && after_pathsep(&name_vec[..name_len]) != 0 && name_len > past_head_off {
            let tail_off =
                path_tail_with_sep(name_cstr).as_ptr() as usize - name_cstr.as_ptr() as usize;
            name_vec[tail_off] = 0;
        }
        let full = fix_fname(Some(crate::memory::bytes_as_cstr(&name_vec)));
        drop(name_vec);
        if let Some(dir_full) = full {
            full_cstr = dir_full;
        }
        full_cstr = concat_fnames_cstr(&full_cstr, c"", true);
    }

    #[cfg(windows)]
    if buf.is_some_and(|b| !b.b_p_bin.get())
        && let Some(rfname_cstr) = os_resolve_shortcut(Some(&full_cstr))
    {
        sf_cstr.clone_from(&rfname_cstr);
        full_cstr = rfname_cstr;
    }

    Some((full_cstr, sf_cstr))
}

/// Return true if `buf` is a prompt buffer.
pub fn bt_prompt(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && buf.unwrap().b_p_bt.first_byte() == b'p'
}

/// Return true if `buf` is the `|cmdwin|` scratch buffer.
pub fn bt_cmdwin(buf: Option<&FileBuf>) -> bool {
    buf.is_some_and(|b| g_st().cmdwin_buf.get() == b)
}

/// Open a window for a number of buffers.
pub fn ex_buffer_all(eap: &mut Exarg) {
    let mut wpnext: crate::globals::WinPtr;
    let mut split_ret = true;
    let mut open_wins = 0;
    let had_tab = g_st().cmdmod.borrow().cmod_tab;

    let count = if eap.addr_count == 0 { 9999 } else { eap.line2 };

    let all = eap.cmdidx != Cmdidx::CmdUnhide && eap.cmdidx != Cmdidx::CmdSunhide;

    reset_visual_and_resel();
    setpcmark();

    if had_tab > 0 {
        let first_tp = g_st().first_tabpage.get().clone();
        goto_tabpage_tp(first_tp.opt(), true, true);
    }
    loop {
        let mut tpnext = curtab_mut().tp_next.get();
        let mut wp = if lastwin_mut().w_floating.get() {
            g_st().lastwin.get()
        } else {
            g_st().firstwin.get()
        };
        while !wp.is_null() {
            wpnext = if wp.borrow().w_floating.get() {
                if wp.borrow().prev_mut().w_floating.get() {
                    wp.borrow().w_prev.get()
                } else {
                    g_st().firstwin.get()
                }
            } else if wp
                .borrow()
                .next_opt()
                .is_none_or(|next| next.w_floating.get())
            {
                crate::globals::WinPtr::NULL
            } else {
                wp.borrow().w_next.get()
            };
            let size_mismatch = if (g_st().cmdmod.borrow().cmod_split & WSP_VERT) != 0 {
                i64::from(
                    wp.borrow().w_height.get()
                        + wp.borrow().w_hsep_height.get()
                        + wp.borrow().w_status_height.get(),
                ) < i64::from(g_st().rows_global.get())
                    - opt_vars().p_ch.get()
                    - i64::from(tabline_height())
                    - i64::from(global_stl_height())
            } else {
                wp.borrow().w_width.get() != g_st().columns_global.get()
            };
            if (wp.borrow().buf_mut().b_nwindows.get() > 1
                || wp.borrow().w_floating.get()
                || size_mismatch
                || (had_tab > 0 && wp != g_st().firstwin.get()))
                && g_st().firstwin.get() != g_st().lastwin.get()
                && !(win_locked(wp.borrow_mut()) != 0 || wp.borrow().buf_mut().b_locked.get() > 0)
                && !is_ctx_win(&wp)
            {
                if !win_close(wp.borrow_mut(), false, false) {
                    break;
                }
                wpnext = if lastwin_mut().w_floating.get() {
                    g_st().lastwin.get()
                } else {
                    g_st().firstwin.get()
                };
                tpnext = g_st().first_tabpage.get().clone();
                open_wins = 0;
            } else {
                open_wins += 1;
            }
            wp = wpnext;
        }

        if had_tab == 0 || tpnext.is_null() {
            break;
        }
        goto_tabpage_tp(tpnext.opt(), true, true);
    }

    g_st()
        .autocmd_no_enter
        .set(g_st().autocmd_no_enter.get() + (1));
    win_enter(&lastwin_nofloating(None), false);
    g_st()
        .autocmd_no_leave
        .set(g_st().autocmd_no_leave.get() + (1));
    let mut buf = g_st().firstbuf.get();
    while !buf.is_null() && open_wins < count {
        if (!all && buf.borrow().b_ml.ml_mfp.borrow().is_none()) || !buf.borrow().b_p_bl.get() {
            buf = buf.borrow().b_next.get();
            continue;
        }

        let mut wp: crate::globals::WinPtr;
        if had_tab != 0 {
            wp = if buf.borrow().b_nwindows.get() > 0 {
                g_st().lastwin.get()
            } else {
                crate::globals::WinPtr::NULL
            };
        } else {
            wp = g_st().firstwin.get();
            while !wp.is_null() {
                if !wp.borrow().w_floating.get() && wp.borrow().w_buffer.get() == buf {
                    break;
                }
                wp = wp.borrow().w_next.get();
            }
            if !wp.is_null() {
                win_move_after(wp.borrow_mut(), &curwin_mut());
            }
        }

        if wp.is_null() && split_ret {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, buf.opt_mut());
            let p_ea_save = opt_vars().p_ea.get();
            opt_vars().p_ea.set(true);
            let ok = win_split(0, WSP_ROOM | WSP_BELOW);
            split_ret = ok;
            open_wins += 1;
            opt_vars().p_ea.set(p_ea_save);
            if !ok {
                buf = buf.borrow().b_next.get();
                continue;
            }

            g_st().swap_exists_action.set(SEA_DIALOG);
            set_curbuf(
                buf.borrow_mut(),
                DOBUF_GOTO,
                (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_CLEAN) == 0,
            );
            if !bufref_valid(Some(&bufref)) {
                g_st().swap_exists_action.set(SEA_NONE);
                break;
            }
            if g_st().swap_exists_action.get() == SEA_QUIT {
                let mut cs: ExCleanup = ExCleanup::new();
                enter_cleanup(&mut cs);
                win_close(&curwin_mut(), true, false);
                open_wins -= 1;
                g_st().swap_exists_action.set(SEA_NONE);
                g_st().swap_exists_did_quit.set(true);
                leave_cleanup(&mut cs);
            } else {
                handle_swap_exists(None);
            }
        }

        os_breakcheck();
        if g_st().got_int.get() {
            let _ = vgetc();
            break;
        }
        if aborting() {
            break;
        }
        if had_tab > 0 && i64::from(tabpage_index(None)) <= opt_vars().p_tpm.get() {
            g_st().cmdmod.borrow_mut().cmod_tab = 9999;
        }
        buf = buf.borrow().b_next.get();
    }
    g_st()
        .autocmd_no_enter
        .set(g_st().autocmd_no_enter.get() - (1));
    win_enter(&firstwin_mut(), false);
    g_st()
        .autocmd_no_leave
        .set(g_st().autocmd_no_leave.get() - (1));

    let mut wp = g_st().lastwin.get();
    while open_wins > count {
        let r = (buf_hide(&wp.borrow().buf_mut())
            || !buf_is_changed(&wp.borrow().buf_mut())
            || autowrite(&wp.borrow().buf_mut(), false))
            && !is_ctx_win(&wp);
        if !win_valid(wp.opt()) {
            wp = g_st().lastwin.get();
        } else if r {
            let close_buf = !buf_hide(&wp.borrow().buf_mut());
            win_close(wp.borrow_mut(), close_buf, false);
            open_wins -= 1;
            wp = g_st().lastwin.get();
        } else {
            wp = wp.borrow().w_prev.get();
            if wp.is_null() {
                break;
            }
        }
    }
}

/// Process mode lines for the current file.
pub fn do_modelines(flags: i32) {
    let mut nmlines = opt_vars().p_mls.get() as i32;
    if !curbuf_mut().b_p_ml.get() || nmlines == 0 {
        return;
    }

    if BUF_STATE.with(|s| s.modelines_entered.get()) != 0 {
        return;
    }

    BUF_STATE.with(|s| s.modelines_entered.set(s.modelines_entered.get() + (1)));
    let mut lnum = 1;
    while curbuf_mut().b_p_ml.get()
        && lnum <= curbuf_mut().b_ml.ml_line_count.get()
        && lnum <= nmlines
    {
        if !chk_modeline(lnum, flags) {
            nmlines = 0;
        }
        lnum += 1;
    }

    lnum = curbuf_mut().b_ml.ml_line_count.get();
    while curbuf_mut().b_p_ml.get()
        && lnum > 0
        && lnum > nmlines
        && lnum > curbuf_mut().b_ml.ml_line_count.get() - nmlines
    {
        if !chk_modeline(lnum, flags) {
            nmlines = 0;
        }
        lnum -= 1;
    }
    BUF_STATE.with(|s| s.modelines_entered.set(s.modelines_entered.get() - (1)));
}

fn chk_modeline(lnum: Linenr, flags: i32) -> bool {
    let mut retval = true;

    let mut prev: i32 = -1;
    let line_bytes = crate::memline::ml_get_slice(lnum);
    let mut idx = 0usize;
    while idx < line_bytes.len() {
        let rem = &line_bytes[idx..];
        if prev == -1 || ascii_isspace(prev) {
            if (prev != -1 && rem.starts_with(b"ex:")) || rem.starts_with(b"vi:") {
                break;
            }
            if matches!(rem.first(), Some(b'v' | b'V')) && rem.get(1..3) == Some(b"im") {
                let s3 = rem.get(3).copied().unwrap_or(0);
                let e_start = if matches!(s3, b'<' | b'=' | b'>') {
                    4
                } else {
                    3
                };
                let mut vers: i64 = 0;
                let mut e_rem = rem.get(e_start..).unwrap_or(&[]);
                if !try_getdigits(&mut e_rem, &mut vers) {
                    prev = rem[0] as i32;
                    idx += 1;
                    continue;
                }

                let vim_version = i64::from(min_vim_version());
                if e_rem.first() == Some(&b':')
                    && (rem[0] != b'V'
                        || crate::charset::skipwhite_slice(&e_rem[1..]).starts_with(b"set"))
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
        prev = rem[0] as i32;
        idx += 1;
    }

    if idx >= line_bytes.len() {
        return retval;
    }

    while idx < line_bytes.len() {
        let ch = line_bytes[idx];
        idx += 1;
        if ch == b':' {
            break;
        }
    }

    let len = line_bytes.len() - idx;
    let mut linecopy_vec: Vec<u8> = line_bytes[idx..].to_vec();
    linecopy_vec.push(0);
    let mut line_len = len;
    let mut s_idx = 0usize;

    estack_push(ETYPE_MODELINE, Some(c"modelines"), lnum);

    let mut end = false;
    while !end {
        while s_idx < line_len && crate::charset::ascii_iswhite(linecopy_vec[s_idx] as i32) {
            s_idx += 1;
        }
        if s_idx >= line_len || linecopy_vec[s_idx] == 0 {
            break;
        }

        let mut e_idx = s_idx;
        while e_idx < line_len && linecopy_vec[e_idx] != b':' && linecopy_vec[e_idx] != 0 {
            if linecopy_vec[e_idx] == b'\\' && linecopy_vec.get(e_idx + 1) == Some(&b':') {
                linecopy_vec.copy_within((e_idx + 1)..=line_len, e_idx);
                line_len -= 1;
            }
            e_idx += 1;
        }
        if e_idx >= line_len || linecopy_vec[e_idx] == 0 {
            end = true;
        }

        if linecopy_vec[s_idx..].starts_with(b"set ") || linecopy_vec[s_idx..].starts_with(b"se ") {
            if linecopy_vec[e_idx] != b':' {
                break;
            }
            end = true;
            s_idx += if linecopy_vec[s_idx + 2] == b' ' {
                3
            } else {
                4
            };
        }
        linecopy_vec[e_idx] = 0;

        if linecopy_vec[s_idx] != 0 {
            let secure_save = g_st().secure.get();
            let save_current_sctx = g_st().current_sctx.get();
            {
                let mut v = g_st().current_sctx.get();
                v.sc_sid = SID_MODELINE;
                g_st().current_sctx.set(v);
            }
            {
                let mut v = g_st().current_sctx.get();
                v.sc_seq = 0;
                g_st().current_sctx.set(v);
            }
            {
                let mut v = g_st().current_sctx.get();
                v.sc_lnum = lnum;
                g_st().current_sctx.set(v);
            }
            g_st().secure.set(1);

            retval = do_set(
                crate::memory::bytes_as_cstr(&linecopy_vec[s_idx..]),
                OPT_MODELINE | OPT_LOCAL | flags,
            );

            g_st().secure.set(secure_save);
            g_st().current_sctx.set(save_current_sctx);
            if !retval {
                break;
            }
        }
        s_idx = if e_idx == line_len { e_idx } else { e_idx + 1 };
    }

    estack_pop();
    drop(linecopy_vec);

    retval
}

/// Return true if `buf` is a help buffer.
pub fn bt_help(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && buf.unwrap().b_help.get()
}

/// Return true if `buf` is a normal buffer, `'buftype'` is empty.
pub fn bt_normal(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && buf.unwrap().b_p_bt.is_empty()
}

/// Return true if `buf` is the quickfix buffer.
pub fn bt_quickfix(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && buf.unwrap().b_p_bt.first_byte() == b'q'
}

/// Return true if `buf` is a terminal buffer.
pub fn bt_terminal(buf: Option<&FileBuf>) -> bool {
    buf.is_some() && buf.unwrap().b_p_bt.first_byte() == b't'
}

/// Return true if `buf` is a `"nofile"`, `"acwrite"`, `"terminal"` or `"prompt"` buffer.
pub fn bt_nofilename(buf: Option<&FileBuf>) -> bool {
    let Some(buf) = buf else {
        return false;
    };
    let bt = buf.b_p_bt.as_bytes();
    (bt.first() == Some(&b'n') && bt.get(2) == Some(&b'f'))
        || matches!(bt.first(), Some(b'a' | b'p'))
        || !buf.terminal.get().is_null()
}

fn bt_nofileread(buf: Option<&FileBuf>) -> bool {
    let Some(buf) = buf else {
        return false;
    };
    let bt = buf.b_p_bt.as_bytes();
    (bt.first() == Some(&b'n') && bt.get(2) == Some(&b'f'))
        || matches!(bt.first(), Some(b't' | b'q' | b'p'))
}

/// Return true if `buf` has `'buftype'` set to `"nofile"`.
pub fn bt_nofile(buf: Option<&FileBuf>) -> bool {
    let Some(buf) = buf else {
        return false;
    };
    let bt = buf.b_p_bt.as_bytes();
    bt.first() == Some(&b'n') && bt.get(2) == Some(&b'f')
}

/// Return true if `buf` is a `"nowrite"`, `"nofile"`, `"terminal"` or `"prompt"` buffer.
pub fn bt_dontwrite(buf: Option<&FileBuf>) -> bool {
    let Some(buf) = buf else {
        return false;
    };
    let bt = buf.b_p_bt.as_bytes();
    matches!(bt.first(), Some(b'n' | b'p')) || !buf.terminal.get().is_null()
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
    match buf.b_p_bh.first_byte() {
        b'u' | b'w' | b'd' => false,
        b'h' => true,
        _ => opt_vars().p_hid.get() || (g_st().cmdmod.borrow().cmod_flags & CMOD_HIDE) != 0,
    }
}

/// Return special buffer name or NULL when the buffer has a normal file name.
pub fn buf_spname(buf: &FileBuf) -> Option<crate::option_vars::RcCStr> {
    if bt_quickfix(Some(buf)) {
        if buf.b_fnum.get() == qf_stack_get_bufnr() {
            return Some(crate::option_vars::RcCStr::from_static(gettext(MSG_QFLIST)));
        }
        return Some(crate::option_vars::RcCStr::from_static(gettext(
            MSG_LOCLIST,
        )));
    }
    if bt_nofilename(Some(buf)) {
        if let Some(f) = buf.fname_opt() {
            return Some(f);
        }
        if bt_cmdwin(Some(buf)) {
            return Some(crate::option_vars::RcCStr::from_static(gettext(
                c"[Command Line]",
            )));
        }
        if bt_prompt(Some(buf)) {
            return Some(crate::option_vars::RcCStr::from_static(gettext(
                c"[Prompt]",
            )));
        }
        return Some(crate::option_vars::RcCStr::from_static(gettext(
            c"[Scratch]",
        )));
    }
    if buf.fname_opt().is_none() {
        return Some(buf_get_fname(buf));
    }
    None
}

/// Get `buf->b_fname`, use `"[No Name]"` if it is NULL.
pub fn buf_get_fname(buf: &FileBuf) -> crate::option_vars::RcCStr {
    buf.fname_opt()
        .unwrap_or_else(|| crate::option_vars::RcCStr::from_static(gettext(c"[No Name]")))
}

/// Set `'buflisted'` for `curbuf` to `on` and trigger autocommands if it changed.
pub fn set_buflisted(on: i32) {
    let on = on != 0;
    if on == curbuf_mut().b_p_bl.get() {
        return;
    }

    curbuf_mut().b_p_bl.set(on);
    if on {
        apply_autocmds(EVENT_BUFADD, None, None, false, curbuf_opt_mut().as_deref());
    } else {
        apply_autocmds(
            EVENT_BUFDELETE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
    }
}

/// Read the file for `buf` again and check if the contents changed.
pub fn buf_contents_changed(buf: &FileBuf) -> bool {
    let mut differ = true;

    let Some(newbuf_ref) = buflist_new(None, None, 1, BLN_DUMMY) else {
        return true;
    };
    let newbuf = newbuf_ref;

    let mut ea: Exarg = Exarg::new();
    let _ea_cmd = prep_exarg(&mut ea, buf);

    let mut aco: CtxSwitch = CtxSwitch::new();
    ctx_switch(&mut aco, None, None, newbuf.opt_mut(), 0);

    block_autocmds();

    if ml_open(&curbuf_mut())
        && readfile(
            buf.ffname_opt().as_deref(),
            buf.fname_opt().as_deref(),
            0,
            0,
            MAXLNUM,
            Some(&ea),
            READ_NEW | READ_DUMMY,
            false,
        ) == Some(true)
        && buf.b_ml.ml_line_count.get() == curbuf_mut().b_ml.ml_line_count.get()
    {
        differ = false;
        for lnum in 1..=curbuf_mut().b_ml.ml_line_count.get() {
            if ml_get_buf_slice(buf, lnum) != crate::memline::ml_get_slice(lnum) {
                differ = true;
                break;
            }
        }
    }
    drop(_ea_cmd);

    ctx_restore(&mut aco);

    if g_st().curbuf.get() != newbuf {
        wipe_buffer(newbuf.borrow_mut(), false);
    }

    unblock_autocmds();

    differ
}

/// Wipe out a (typically temporary) buffer.
pub fn wipe_buffer(buf: &FileBuf, aucmd: bool) {
    let buf_ptr = crate::globals::BufPtr::from(buf);
    if !aucmd {
        block_autocmds();
    }
    close_buffer(None, buf_ptr.opt_mut(), DOBUF_WIPE, false, true, false);
    if !aucmd {
        unblock_autocmds();
    }
}

pub fn buf_is_empty(buf: &FileBuf) -> bool {
    buf.b_ml.ml_line_count.get() == 1 && ml_get_buf_slice(buf, 1).is_empty()
}

/// Increment `b:changedtick` value.
pub fn buf_inc_changedtick(buf: &FileBuf) {
    buf_set_changedtick(buf, buf_get_changedtick(buf) + 1);
}

/// Set `b:changedtick`, also checking `b:` for consistency in debug build.
pub fn buf_set_changedtick(buf: &FileBuf, changedtick: Varnumber) {
    let old_val = buf.changedtick_di.tv().clone();

    let changedtick_di = tv_dict_find(Some(&*buf.b_vars()), c"changedtick", 11);
    debug_assert!(changedtick_di.is_some());
    debug_assert_eq!(changedtick_di.unwrap(), buf.changedtick_di);
    debug_assert!(buf.changedtick_di.tv().v_type == VarType::VarNumber);
    debug_assert!(buf.changedtick_di.tv().v_lock == VAR_FIXED);
    debug_assert!(buf.changedtick_di.di_flags.get() == (DI_FLAGS_RO | DI_FLAGS_FIX));

    buf.changedtick_di.tv_mut().vval = crate::eval::typval::TypvalVval::Number(changedtick);

    if tv_dict_is_watched(Some(&*buf.b_vars())) {
        let new_val = buf.changedtick_di.tv().clone();
        buf.b_locked.set(buf.b_locked.get() + (1));
        tv_dict_watcher_notify(
            &buf.b_vars(),
            buf.changedtick_di.key_cstr(),
            Some(&new_val),
            Some(&old_val),
        );
        buf.b_locked.set(buf.b_locked.get() - (1));
    }
}

/// Read the given buffer contents into a string.
pub fn read_buffer_into(buf: Option<&FileBuf>, start: Linenr, end: Linenr, sb: &mut Vec<u8>) {
    debug_assert!(buf.is_some());
    let b = buf.unwrap();

    if (b.b_ml.ml_flags.get() & ML_EMPTY) != 0 {
        return;
    }

    let mut written = 0usize;
    let mut len: usize;
    let mut lnum = start;
    let mut line = ml_get_buf_slice(b, lnum);

    loop {
        let rem = &line[written..];
        if rem.is_empty() {
            len = 0;
        } else if rem[0] == NL as u8 {
            len = 1;
            sb.push(0);
        } else {
            len = rem.iter().position(|&b| b == NL as u8).unwrap_or(rem.len());
            if len > 0 {
                sb.extend_from_slice(&rem[..len]);
            }
        }

        if len == rem.len() {
            if lnum != end
                || (!b.b_p_bin.get() && b.b_p_fixeol.get())
                || (lnum != b.b_no_eol_lnum.get()
                    && (lnum != b.b_ml.ml_line_count.get() || b.b_p_eol.get()))
            {
                sb.push(b'\n');
            }
            lnum += 1;
            if lnum > end {
                break;
            }
            line = ml_get_buf_slice(b, lnum);
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
            g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));

            assert!(!buf_valid(None));

            let p1 = c"test_file_path";
            let p2 = c"file_path_test";
            let p3 = c"path_test_file";

            let buf = buflist_new(Some(p1), Some(p1), 1, BLN_LISTED).unwrap();
            assert!(buf_valid(buf.opt()));
            close_buffer(None, buf.opt_mut(), 0, false, false, false);
            assert!(buf_valid(buf.opt()));
            close_buffer(None, buf.opt_mut(), DOBUF_UNLOAD, false, false, false);
            assert!(buf_valid(buf.opt()));
            close_buffer(None, buf.opt_mut(), DOBUF_WIPE, false, false, false);
            assert!(!buf_valid(buf.opt()));

            let findpat = |pat: &CStr, allow_unlisted: bool| -> i32 {
                buflist_findpat(pat.to_bytes(), allow_unlisted, false, false)
            };

            let buf1 = buflist_new(Some(p1), Some(p1), 1, BLN_LISTED).unwrap();
            let buf2 = buflist_new(Some(p2), Some(p2), 1, BLN_LISTED).unwrap();
            let buf3 = buflist_new(Some(p3), Some(p3), 1, BLN_LISTED).unwrap();
            let buf1_fnum = buf1.b_fnum.get();
            let buf2_fnum = buf2.b_fnum.get();
            let buf3_fnum = buf3.b_fnum.get();

            assert_eq!(findpat(c"test_file_path", false), buf1_fnum);
            assert_eq!(findpat(c"test", false), buf1_fnum);
            assert_eq!(findpat(c"file", false), buf2_fnum);
            assert_eq!(findpat(c"path", false), buf3_fnum);
            assert_eq!(findpat(c"_test_", false), buf3_fnum);

            // Include / ignore unlisted buffers based on the flag
            close_buffer(None, buf3.opt_mut(), DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"_test_", false), -1);
            assert_eq!(findpat(c"_test_", true), buf3_fnum);
            close_buffer(None, buf3.opt_mut(), DOBUF_WIPE, false, false, false);
            assert_eq!(findpat(c"_test_", false), -1);
            assert_eq!(findpat(c"_test_", true), -1);

            // Unlisted vs listed preference
            close_buffer(None, buf1.opt_mut(), DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"test", false), buf2_fnum);
            assert_eq!(findpat(c"test", true), buf2_fnum);
            close_buffer(None, buf2.opt_mut(), DOBUF_DEL, false, false, false);
            assert_eq!(findpat(c"test", true), buf1_fnum);
            assert_eq!(findpat(c"test", false), -1);

            close_buffer(None, buf1.opt_mut(), DOBUF_WIPE, false, false, false);
            close_buffer(None, buf2.opt_mut(), DOBUF_WIPE, false, false, false);

            g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
        }
    }
}
