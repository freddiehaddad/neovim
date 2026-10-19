//! Deprecated Vimscript evaluation functions (`src/nvim/eval/deprecated.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

use crate::globals::*;
use std::ffi::{c_char, c_int, c_uint, c_void};

use crate::api::private::converter::sctx_T;
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::eval::decode::dict_T;
use crate::garray::garray_T;
use crate::hashtab::hashtab_T;
use crate::kvec::KVec;
use crate::lua::converter::LuaRef;
use crate::map::Map;
use crate::marktree::MarkTree;
use crate::memory::{xmalloc, xstrdup};
use crate::msgpack_rpc::packer::handle_T;

pub type EvalFuncData = *mut c_void;
pub type Channel = c_void;

const K_BOOL_VAR_TRUE: c_int = 1;
const K_CHANNEL_PART_RPC: c_int = 3;
const K_CHANNEL_STDIN_PIPE: c_int = 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Callback {
    pub data: *mut c_void,
    pub cb_type: c_int,
}

#[repr(C)]
pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: *mut dict_T,
    pub buffer: garray_T,
    pub eof: bool,
    pub buffered: bool,
    pub fwd_err: bool,
    pub reader_type: *const c_char,
}

#[repr(C)]
pub struct memline_T {
    pub ml_line_count: i32,
    pub ml_mfp: *mut c_void,
    pub ml_stack: *mut c_void,
    pub ml_stack_top: c_int,
    pub ml_stack_size: c_int,
    pub ml_flags: c_int,
    pub ml_line_textlen: c_int,
    pub ml_line_lnum: i32,
    pub ml_line_ptr: *mut c_char,
    pub ml_line_offset: usize,
    pub ml_line_offset_ff: c_int,
    pub ml_locked: *mut c_void,
    pub ml_locked_low: i32,
    pub ml_locked_high: i32,
    pub ml_locked_lineadd: c_int,
    pub ml_chunksize: *mut c_void,
    pub ml_numchunks: c_int,
    pub ml_usedchunks: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pos_T {
    pub lnum: i32,
    pub col: i32,
    pub coladd: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileID {
    pub inode: u64,
    pub device_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ChangedtickDictItem {
    pub di_tv: typval_T,
    pub di_flags: u8,
    pub di_key: [c_char; 12],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScopeDictDictItem {
    pub di_tv: typval_T,
    pub di_flags: u8,
    pub di_key: [c_char; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fmarkv_T {
    pub topline_offset: i32,
    pub skipcol: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fmark_T {
    pub mark: pos_T,
    pub fnum: c_int,
    pub timestamp: u64,
    pub view: fmarkv_T,
    pub additional_data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct visualinfo_T {
    pub vi_start: pos_T,
    pub vi_end: pos_T,
    pub vi_mode: c_int,
    pub vi_curswant: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct syn_time_T {
    pub total: u64,
    pub slowest: u64,
    pub count: c_int,
    pub match_: c_int,
}

#[repr(C)]
pub struct synblock_T {
    pub b_keywtab: hashtab_T,
    pub b_keywtab_ic: hashtab_T,
    pub b_syn_error: bool,
    pub b_syn_slow: bool,
    pub b_syn_ic: c_int,
    pub b_syn_foldlevel: c_int,
    pub b_syn_spell: c_int,
    pub b_syn_patterns: garray_T,
    pub b_syn_clusters: garray_T,
    pub b_spell_cluster_id: c_int,
    pub b_nospell_cluster_id: c_int,
    pub b_syn_containedin: c_int,
    pub b_syn_sync_flags: c_int,
    pub b_syn_sync_id: i16,
    pub b_syn_sync_minlines: i32,
    pub b_syn_sync_maxlines: i32,
    pub b_syn_sync_linebreaks: i32,
    pub b_syn_linecont_pat: *mut c_char,
    pub b_syn_linecont_prog: *mut c_void,
    pub b_syn_linecont_time: syn_time_T,
    pub b_syn_linecont_ic: c_int,
    pub b_syn_topgrp: c_int,
    pub b_syn_conceal: c_int,
    pub b_syn_folditems: c_int,
    pub b_sst_array: *mut c_void,
    pub b_sst_len: c_int,
    pub b_sst_first: *mut c_void,
    pub b_sst_search: *mut c_void,
    pub b_sst_firstfree: *mut c_void,
    pub b_sst_freecount: c_int,
    pub b_sst_check_lnum: i32,
    pub b_sst_lasttick: u64,
    pub b_idlist_cache: *mut c_void,
    pub b_langp: garray_T,
    pub b_spell_ismw: [bool; 256],
    pub b_spell_ismw_mb: *mut c_char,
    pub b_p_spc: *mut c_char,
    pub b_cap_prog: *mut c_void,
    pub b_p_spf: *mut c_char,
    pub b_p_spl: *mut c_char,
    pub b_p_spo: *mut c_char,
    pub b_p_spo_flags: c_uint,
    pub b_cjk: c_int,
    pub b_syn_chartab: [u64; 4],
    pub b_syn_isk: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufSigncols {
    pub max: c_int,
    pub last_max: c_int,
    pub count: [c_int; 9],
    pub autom: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufUpdateCallbacks {
    pub on_lines: LuaRef,
    pub on_bytes: LuaRef,
    pub on_changedtick: LuaRef,
    pub on_detach: LuaRef,
    pub on_reload: LuaRef,
    pub utf_sizes: bool,
    pub preview: bool,
}

#[repr(C)]
pub struct buf_T {
    pub b_fnum: handle_T,
    pub b_ml: memline_T,
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
    pub b_wininfo: KVec<*mut c_void>,
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
    pub b_visual: visualinfo_T,
    pub b_visual_mode_eval: c_int,
    pub b_last_cursor: fmark_T,
    pub b_last_insert: fmark_T,
    pub b_last_change: fmark_T,
    pub b_changelist: [fmark_T; 100],
    pub b_changelistlen: c_int,
    pub b_new_change: bool,
    pub b_chartab: [u64; 4],
    pub b_maphash: [*mut c_void; 256],
    pub b_first_abbr: *mut c_void,
    pub b_ucmds: garray_T,
    pub b_op_start: pos_T,
    pub b_op_start_orig: pos_T,
    pub b_op_end: pos_T,
    pub b_marks_read: bool,
    pub b_modified_was_set: bool,
    pub b_did_filetype: bool,
    pub b_keep_filetype: bool,
    pub b_au_did_filetype: bool,
    pub b_u_oldhead: *mut c_void,
    pub b_u_newhead: *mut c_void,
    pub b_u_curhead: *mut c_void,
    pub b_u_numhead: c_int,
    pub b_u_synced: bool,
    pub b_u_seq_last: c_int,
    pub b_u_save_nr_last: c_int,
    pub b_u_seq_cur: c_int,
    pub b_u_time_cur: i64,
    pub b_u_save_nr_cur: c_int,
    pub b_u_line_ptr: *mut c_char,
    pub b_u_line_lnum: i32,
    pub b_u_line_colnr: i32,
    pub b_scanned: bool,
    pub b_p_iminsert: i64,
    pub b_p_imsearch: i64,
    pub b_kmap_state: i16,
    pub b_kmap_ga: garray_T,
    pub b_p_initialized: bool,
    pub b_p_script_ctx: [sctx_T; 93],
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
    pub b_p_cpt_cb: *mut Callback,
    pub b_p_cpt_count: c_int,
    pub b_p_cfu: Callback,
    pub b_p_ofu: Callback,
    pub b_p_tfu: Callback,
    pub b_p_ffu: Callback,
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
    pub b_p_inex: Callback,
    pub b_p_inex_flags: u32,
    pub b_p_inde: Callback,
    pub b_p_inde_flags: u32,
    pub b_p_indk: *mut c_char,
    pub b_p_fp: *mut c_char,
    pub b_p_fex: Callback,
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
    pub b_p_vsts_array: *mut i32,
    pub b_p_vsts_nopaste: *mut c_char,
    pub b_p_vts: *mut c_char,
    pub b_p_vts_array: *mut i32,
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
    pub b_p_tsrfu: Callback,
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
    pub b_start_fenc: *mut c_char,
    pub b_bad_char: c_int,
    pub b_start_bomb: c_int,
    pub b_bufvar: ScopeDictDictItem,
    pub b_vars: *mut dict_T,
    pub b_may_swap: bool,
    pub b_did_warn: bool,
    pub b_help: bool,
    pub b_spell: bool,
    pub b_prompt_text: *mut c_char,
    pub b_prompt_callback: Callback,
    pub b_prompt_interrupt: Callback,
    pub b_prompt_append_new_line: bool,
    pub b_prompt_insert: c_int,
    pub b_prompt_start: fmark_T,
    pub b_s: synblock_T,
    pub b_signcols: BufSigncols,
    pub terminal: *mut c_void,
    pub additional_data: *mut c_void,
    pub b_mapped_ctrl_c: c_int,
    pub b_marktree: [MarkTree; 1],
    pub b_extmark_ns: [Map<u32, u32>; 1],
    pub b_prev_line_count: c_int,
    pub update_channels: KVec<u64>,
    pub update_callbacks: KVec<BufUpdateCallbacks>,
    pub update_need_codepoints: bool,
    pub deleted_bytes: usize,
    pub deleted_bytes2: usize,
    pub deleted_codepoints: usize,
    pub deleted_codeunits: usize,
    pub flush_count: c_int,
    pub b_localdir: *mut c_char,
    pub b_prevdir: *mut c_char,
}

#[cfg(not(test))]
unsafe extern "C" {

    fn check_secure() -> bool;
    fn tv_get_string(tv: *const typval_T) -> *const c_char;
    fn tv_dict_alloc() -> *mut dict_T;
    fn tv_dict_add_bool(d: *mut dict_T, key: *const c_char, key_len: usize, val: c_int) -> c_int;
    fn tv_dict_free(d: *mut dict_T);

    fn channel_job_start(
        argv: *mut *mut c_char,
        exepath: *const c_char,
        on_stdout: CallbackReader,
        on_stderr: CallbackReader,
        on_exit: Callback,
        pty: bool,
        rpc: bool,
        overlapped: bool,
        detach: bool,
        stdin_mode: c_int,
        cwd: *const c_char,
        pty_width: u16,
        pty_height: u16,
        env: *mut dict_T,
        status_out: *mut i64,
    ) -> *mut Channel;
    fn channel_create_event(chan: *mut Channel, ext_source: *const c_char);
    fn channel_close(id: u64, part: c_int, error: *mut *const c_char) -> bool;
    fn find_job(id: u64, show_error: bool) -> *mut Channel;
    fn f_jobstart(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData);
    fn f_jobstop(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData);

    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
}

#[inline]
const fn callback_none() -> Callback {
    Callback {
        data: std::ptr::null_mut(),
        cb_type: 0,
    }
}

#[inline]
const fn callback_reader_init() -> CallbackReader {
    CallbackReader {
        cb: callback_none(),
        self_dict: std::ptr::null_mut(),
        buffer: garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: std::ptr::null_mut(),
        },
        eof: false,
        buffered: false,
        fwd_err: false,
        reader_type: std::ptr::null(),
    }
}

/// `"rpcstart()"` function (`f_rpcstart`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_rpcstart(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        let arg0 = &*argvars;
        let arg1 = &*argvars.add(1);
        if arg0.v_type != VarType::VAR_STRING
            || (arg1.v_type != VarType::VAR_LIST && arg1.v_type != VarType::VAR_UNKNOWN)
        {
            emsg(c"E474: Invalid argument".as_ptr());
            return;
        }

        let mut args: *mut list_T = std::ptr::null_mut();
        let mut argsl: c_int = 0;
        if arg1.v_type == VarType::VAR_LIST {
            args = arg1.vval.v_list;
            argsl = if args.is_null() { 0 } else { (*args).lv_len };
            let mut i = 0 as c_int;
            let mut li: *const listitem_T = if args.is_null() {
                std::ptr::null()
            } else {
                (*args).lv_first
            };
            while !li.is_null() {
                if (*li).li_tv.v_type != VarType::VAR_STRING {
                    semsg(
                        c"E5010: List item %d of the second argument is not a string".as_ptr(),
                        i,
                    );
                    return;
                }
                i += 1;
                li = (*li).li_next;
            }
        }

        let prog = arg0.vval.v_string;
        if prog.is_null() || *prog == 0 {
            emsg(c"E903: Could not spawn API job".as_ptr());
            return;
        }

        let argvl = (argsl as usize) + 2;
        let argv = xmalloc(std::mem::size_of::<*mut c_char>() * argvl).cast::<*mut c_char>();
        *argv = xstrdup(prog);

        let mut idx = 1usize;
        if argsl > 0 && !args.is_null() {
            let mut li: *const listitem_T = (*args).lv_first;
            while !li.is_null() {
                *argv.add(idx) = xstrdup(tv_get_string(&(*li).li_tv));
                idx += 1;
                li = (*li).li_next;
            }
        }
        *argv.add(idx) = std::ptr::null_mut();

        let chan = channel_job_start(
            argv,
            std::ptr::null(),
            callback_reader_init(),
            callback_reader_init(),
            callback_none(),
            false,
            true,
            false,
            false,
            K_CHANNEL_STDIN_PIPE,
            std::ptr::null(),
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!((*rettv).vval.v_number),
        );
        if !chan.is_null() {
            channel_create_event(chan, std::ptr::null());
        }
    }
}

/// `"rpcstop()"` function (`f_rpcstop`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_rpcstop(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv, fptr);

    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        let arg0 = &*argvars;
        if arg0.v_type != VarType::VAR_NUMBER {
            emsg(c"E474: Invalid argument".as_ptr());
            return;
        }

        let id = arg0.vval.v_number as u64;
        if !find_job(id, false).is_null() {
            f_jobstop(argvars, rettv, fptr);
        } else {
            let mut error: *const c_char = std::ptr::null();
            let closed = channel_close(id, K_CHANNEL_PART_RPC, &mut error);
            (*rettv).vval.v_number = i64::from(closed);
            if !closed {
                emsg(error);
            }
        }
    }
}

/// `"last_buffer_nr()"` function (`f_last_buffer_nr`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_last_buffer_nr(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = rettv;

    #[cfg(not(test))]
    unsafe {
        let mut n: c_int = 0;
        let mut buf = firstbuf;
        while !buf.is_null() {
            if n < (*buf).b_fnum {
                n = (*buf).b_fnum;
            }
            buf = (*buf).b_next;
        }
        (*rettv).vval.v_number = n as i64;
    }
}

/// `"termopen(cmd[, cwd])"` function (`f_termopen`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_termopen(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv, fptr);

    #[cfg(not(test))]
    unsafe {
        if check_secure() {
            return;
        }

        let mut must_free = false;
        let arg1 = &mut *argvars.add(1);
        if arg1.v_type == VarType::VAR_UNKNOWN {
            must_free = true;
            arg1.v_type = VarType::VAR_DICT;
            arg1.vval.v_ptr = tv_dict_alloc().cast::<c_void>();
        }

        if arg1.v_type != VarType::VAR_DICT {
            semsg(
                c"E475: Invalid argument: %s".as_ptr(),
                c"expected dictionary".as_ptr(),
            );
            return;
        }

        let dict = arg1.vval.v_ptr.cast::<dict_T>();
        tv_dict_add_bool(dict, c"term".as_ptr(), 4, K_BOOL_VAR_TRUE);
        f_jobstart(argvars, rettv, fptr);
        if must_free {
            tv_dict_free(dict);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_reader_init_defaults_to_null_and_false() {
        let r = callback_reader_init();
        assert!(r.cb.data.is_null());
        assert_eq!(r.cb.cb_type, 0);
        assert!(r.self_dict.is_null());
        assert!(!r.eof);
        assert!(!r.buffered);
        assert!(!r.fwd_err);
        assert!(r.reader_type.is_null());
    }
}
